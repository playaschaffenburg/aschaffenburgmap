"""Sonderbau Dr.-Willi-Reiland-Ring / Ludwigsallee (Aschaffenburg), Etappe 1: das Bauwerk.

Ein Einzelstueck fuer genau diese Stelle (siehe sonderbau/ring/PLAN.md): der Ring im Trog, die Rampen links und rechts
auf Stuetzmauern, die Unterfuehrung unter der Ludwigsallee und die Bruecke der Ludwigsallee ueber die Bahn.

Vorgehen: entlang der Ring-Achse (OSM) alle 2 m ein Querschnitt durch das DGM1 (1 m, Bayern). Darin liegen die drei
Fahrbahnen als ebene Stufen: der Ring unten, die Rampen daneben hoeher (oder gleich hoch, wo sie einmuenden). Aus den
Stufenkanten und -hoehen entstehen die Waende:
- Trennwaende Ring/Rampe: Stuetzwand von der Ring-Sohle bis zur Rampe, zur Ring-Seite Klinkerfelder zwischen
  Betonpfeilern, oben Betonkappe, Bruestung mit Gelaender zur Rampe hin.
- Aussenwand West: Gabionen, darauf eine gruene Laermschutzwand.
- Aussenwand Ost: hohe Klinker-Stuetzmauer, ab 3,5 m zweistufig mit Pflanzstreifen, oben eine Glas-Laermschutzwand.
- Unterfuehrung: Betondecke ueber dem Ring mit halbrund vorspringenden Stirnseiten (wie im Luftbild).
- Bahnbruecke: Platte, Bruestungen mit Gelaender, Widerlager.

Ausgabe: Objekte (.sco + model/*.x + texture/) im Ordner der Karte und `sonderbau_ring.json` mit ihrer Lage
(Kartenkoordinaten, Hoehe absolut in der Karte = NN - nn0). Aufruf:
    python -m omsigen.sonderbau_ring --omsi <OMSI-Ordner> --karte Aburg_Newgen --osm sonderbau/ring/osm_map.xml
Daten: OpenStreetMap (ODbL), DGM1/DOP40 Bayerische Vermessungsverwaltung (CC BY 4.0)."""
import argparse, json, math, os, shutil, sys
import xml.etree.ElementTree as ET
import numpy as np
from .utm import to_utm
from .geodaten import Dgm1
from .kreuzung import x_file

# OSM-Wege der drei Fahrbahnen (Nord -> Sued bzw. in Fahrtrichtung)
RING = ['159575718', '227934178', '7975056', '227934179']
WEST = ['227934180', '183815409', '23781579']          # Einbahn nach Sueden
OST = ['233369539', '23781578', '7925387']             # (umgedreht: Nord -> Sued) Einbahn nach Norden
Y_NORD, Y_SUED = -142.0, -414.0                        # Bereich des Trogs (Kartenkoordinaten)
DECKE = (-253.5, -279.0)                               # Unterfuehrung: Nord- und Suedkante (y)
SCHRITT = 2.0
STUECK = 40.0                                          # m je Wand-Objekt

TEX = {'beton': 'betonwand1.bmp', 'klinker': 'AB_klinker.bmp', 'gabionen': 'AB_gabionen.bmp',
       'lsw_gruen': 'AB_lsw_gruen.bmp', 'glas': 'AB_glas.tga', 'gelaender': 'AB_gelaender.bmp',
       'decke': 'AB_beton_dunkel.bmp', 'pflanzen': 'AB_bodendecker.bmp', 'asphalt': 'str_asphdrk.bmp',
       'boden': 'AB_boden.bmp'}


# ------------------------------------------------------------------ Geometrie-Helfer
class Mesh:
    """Dreiecke in Objektkoordinaten (x rechts/Ost, y vor/Nord, z hoch); x_file erwartet (x, hoch, vor)"""

    def __init__(self):
        self.V, self.F, self.mats = [], [], []

    def mat(self, key):
        if key not in self.mats:
            self.mats.append(key)
        return self.mats.index(key)

    def quad(self, key, p, uv, n=None, beidseitig=False):
        """Viereck p0..p3 (je (x, y, z)), sichtbar zur Seite n (sonst aus der Reihenfolge, gegen den Uhrzeigersinn)"""
        p = [np.asarray(q, float) for q in p]
        if n is None:
            n = np.cross(p[1] - p[0], p[2] - p[0])
        n = np.asarray(n, float)
        ln = np.linalg.norm(n)
        if ln < 1e-9:
            return
        n = n / ln
        for s in ((1, -1) if beidseitig else (1,)):
            self._quad(key, p, uv, n * s)

    def _quad(self, key, p, uv, n):
        m = self.mat(key)
        X = [(q[0], q[2], q[1]) for q in p]            # in .x-Koordinaten
        N = (n[0], n[2], n[1])
        a, b, c = (np.array(X[i]) for i in range(3))
        vorne = np.dot(np.cross(b - a, c - a), N) >= 0
        i0 = len(self.V)
        for (x, y, z), (u, v) in zip(X, uv):
            self.V.append((x, y, z) + tuple(N) + (u, v))
        if vorne:
            self.F += [(i0, i0 + 1, i0 + 2, m), (i0, i0 + 2, i0 + 3, m)]
        else:
            self.F += [(i0, i0 + 2, i0 + 1, m), (i0, i0 + 3, i0 + 2, m)]

    def leer(self):
        return not self.F


def band(mesh, key, unten, oben, normale, s0, tex=(2.0, 1.0)):
    """senkrechte Flaeche zwischen zwei Linien (je Liste (x, y, z)), Normale je Abschnitt; uv: Laenge/Hoehe"""
    for i in range(len(unten) - 1):
        a, b, c, d = unten[i], unten[i + 1], oben[i + 1], oben[i]
        sa, sb = s0[i], s0[i + 1]
        mesh.quad(key, [a, b, c, d], [(sa / tex[0], a[2] / tex[1]), (sb / tex[0], b[2] / tex[1]),
                                      (sb / tex[0], c[2] / tex[1]), (sa / tex[0], d[2] / tex[1])], normale[i])


def kasten(mesh, key, x0, x1, y0, y1, z0, z1, uvs=4.0):
    """Quader (achsparallel im Objekt), alle Seiten aussen"""
    P = lambda x, y, z: (x, y, z)
    seiten = [([P(x0, y0, z0), P(x1, y0, z0), P(x1, y0, z1), P(x0, y0, z1)], (0, -1, 0)),
              ([P(x1, y1, z0), P(x0, y1, z0), P(x0, y1, z1), P(x1, y1, z1)], (0, 1, 0)),
              ([P(x0, y1, z0), P(x0, y0, z0), P(x0, y0, z1), P(x0, y1, z1)], (-1, 0, 0)),
              ([P(x1, y0, z0), P(x1, y1, z0), P(x1, y1, z1), P(x1, y0, z1)], (1, 0, 0)),
              ([P(x0, y0, z1), P(x1, y0, z1), P(x1, y1, z1), P(x0, y1, z1)], (0, 0, 1)),
              ([P(x0, y1, z0), P(x1, y1, z0), P(x1, y0, z0), P(x0, y0, z0)], (0, 0, -1))]
    for q, n in seiten:
        ax = [i for i in range(3) if n[i] == 0]
        mesh.quad(key, q, [(p[ax[0]] / uvs, p[ax[1]] / uvs) for p in q], n)


# ------------------------------------------------------------------ Daten
def lade_osm(pfad, E0, N0):
    root = ET.parse(pfad).getroot()
    nodes = {}
    for nd in root.findall('node'):
        e, n = to_utm(float(nd.get('lat')), float(nd.get('lon')), 32)
        nodes[nd.get('id')] = (e - E0, n - N0)
    ways = {w.get('id'): [nodes[r.get('ref')] for r in w.findall('nd') if r.get('ref') in nodes] for w in root.findall('way')}
    return ways


def linie(ways, ids, umdrehen=False):
    pts = []
    for i in ids:
        p = list(ways[i])
        if umdrehen:
            p = p[::-1]
        if pts and math.dist(pts[-1], p[0]) < 0.5:
            p = p[1:]
        pts += p
    return np.array(pts, float)


def schnitt(poly, a, d):
    """Schnittpunkt der Geraden a + t*d (d Einheitsvektor) mit dem Polygonzug -> t (naechster an 0) oder None"""
    best = None
    for p, q in zip(poly[:-1], poly[1:]):
        e = q - p
        den = d[0] * (-e[1]) - d[1] * (-e[0])
        if abs(den) < 1e-9:
            continue
        r = p - a
        t = (r[0] * (-e[1]) - r[1] * (-e[0])) / den
        u = (d[0] * r[1] - d[1] * r[0]) / den
        if 0 <= u <= 1 and (best is None or abs(t) < abs(best)):
            best = t
    return best


def stationen(ring, schritt):
    """Punkte, Querrichtung (nach Osten) und Weg entlang der Ring-Achse im Bereich Y_SUED..Y_NORD"""
    seg = np.hypot(*np.diff(ring, axis=0).T)
    s = np.concatenate([[0], np.cumsum(seg)])
    out = []
    for t in np.arange(0, s[-1], schritt):
        k = min(np.searchsorted(s, t, side='right') - 1, len(seg) - 1)
        f = (t - s[k]) / seg[k]
        p = ring[k] + (ring[k + 1] - ring[k]) * f
        if not (Y_SUED <= p[1] <= Y_NORD):
            continue
        d = (ring[k + 1] - ring[k]) / seg[k]               # Nord -> Sued
        quer = np.array([-d[1], d[0]])                     # rechts von Sued = ...
        if quer[0] < 0:
            quer = -quer                                   # immer nach Osten
        out.append((p, quer, d))
    return out


def stufe(h, ts, t0, z, tol=0.45):
    """zusammenhaengende Stufe auf Hoehe z um t0: (links, rechts) im Querschnitt"""
    i0 = int(np.argmin(np.abs(ts - t0)))
    lo = hi = i0
    while lo > 0 and abs(h[lo - 1] - z) < tol:
        lo -= 1
    while hi < len(ts) - 1 and abs(h[hi + 1] - z) < tol:
        hi += 1
    return ts[lo], ts[hi]


def glatt(werte, fenster=5):
    a = np.array(werte, float)
    out = a.copy()
    k = fenster // 2
    for i in range(len(a)):
        w = a[max(0, i - k):i + k + 1]
        w = w[~np.isnan(w)]
        out[i] = np.median(w) if len(w) else np.nan
    return out


def vermessen(ways, dgm, E0, N0, nn0):
    ring = linie(ways, RING)
    west = linie(ways, WEST)
    ost = linie(ways, OST, umdrehen=True)
    st = stationen(ring, SCHRITT)
    ts = np.arange(-28.0, 28.01, 0.5)
    roh = []
    for p, quer, d in st:
        xs, ys = p[0] + quer[0] * ts, p[1] + quer[1] * ts
        h = dgm.hoehen(E0 + xs, N0 + ys) - nn0
        zr = float(np.median(h[np.abs(ts) <= 1.5]))
        unter_decke = DECKE[1] <= p[1] <= DECKE[0]
        if unter_decke:
            # dort zeigt das DGM die Decke: Ring-Sohle aus den Nachbarn (spaeter interpoliert)
            zr = np.nan
            ra, rb = -4.2, 4.2
        else:
            ra, rb = stufe(h, ts, 0.0, zr)
        tw, to = schnitt(west, p, -quer), schnitt(ost, p, quer)
        e = dict(p=p, quer=quer, d=d, zr=zr, ra=ra, rb=rb, unter_decke=unter_decke)
        for name, t, vz in (('w', tw, -1), ('o', to, 1)):
            if t is None:
                e[name] = None
                continue
            t = t * vz
            z = float(np.median(h[np.abs(ts - t) <= 1.0]))
            a, b = stufe(h, ts, t, z)
            # Gelaende aussen: 3 m hinter der Fahrbahnkante
            g = float(np.interp(a - 3.0 if vz < 0 else b + 3.0, ts, h))
            e[name] = dict(t=t, z=z, a=a, b=b, g=g)
        roh.append(e)
    # glaetten (Kanten springen im 1-m-DGM um eine Zelle)
    for key in ('zr', 'ra', 'rb'):
        werte = glatt([e[key] for e in roh])
        for e, w in zip(roh, werte):
            e[key] = w
    # Ring-Sohle unter der Decke: linear zwischen den Raendern
    zr = np.array([e['zr'] for e in roh])
    ok = ~np.isnan(zr)
    idx = np.arange(len(zr))
    zr[~ok] = np.interp(idx[~ok], idx[ok], zr[ok])
    for e, z in zip(roh, zr):
        e['zr'] = float(z)
    for name in ('w', 'o'):
        for key in ('z', 'a', 'b', 'g', 't'):
            werte = glatt([e[name][key] if e[name] else np.nan for e in roh])
            for e, w in zip(roh, werte):
                if e[name] is not None and not np.isnan(w):
                    e[name][key] = float(w)
    return roh


# ------------------------------------------------------------------ Bauteile
def welt(e, t, z):
    p = e['p'] + e['quer'] * t
    return np.array([p[0], p[1], z])


def fuge(e, seite):
    """Lage der Trennwand zwischen Ring und Rampe (Querweg t) oder None, wo die Rampe auf Ring-Hoehe liegt"""
    r = e[seite]
    vz = -1 if seite == 'w' else 1
    if r is None or r['z'] - e['zr'] < 0.4 and not e['unter_decke']:
        return None
    kante_ring = e['ra'] if vz < 0 else e['rb']
    kante_rampe = r['b'] if vz < 0 else r['a']
    return (kante_ring + kante_rampe) / 2.0 if abs(kante_ring - kante_rampe) < 3.0 else kante_ring + vz * 0.4


def im_knoten(e):
    """Station im Bereich der Ludwigsallee (dort keine Aussenwaende, die Kreuzung liegt darueber)"""
    return DECKE[1] - 1.0 <= e['p'][1] <= DECKE[0] + 1.0


def aussen(e, seite):
    """Lage der Aussenwand (Querweg t) oder None"""
    r = e[seite]
    if r is None or im_knoten(e) or r['z'] - e['zr'] < 0.4 and abs(r['g'] - r['z']) < 0.6:
        return None
    return r['a'] - 0.2 if seite == 'w' else r['b'] + 0.2


def rand(e, seite):
    """aeusserster Querweg des Bauwerks auf dieser Seite: Aussenwand, sonst Fahrbahnkante der Rampe bzw. des Rings"""
    t = aussen(e, seite)
    if t is not None:
        return t
    r = e[seite]
    if r is not None:
        return r['a'] if seite == 'w' else r['b']
    return e['ra'] - 1.0 if seite == 'w' else e['rb'] + 1.0


def trennwand(roh, seite, o):
    """Stuetzwand zwischen Ring und Rampe `seite` ('w'/'o'): Linie in der Mitte der Fuge; Ring-Seite Klinkerfelder mit
    Pfeilern alle 5 m, Kappe, Bruestung mit Gelaender zur Rampe -> Mesh (Objektursprung o)"""
    m = Mesh()
    vz = -1 if seite == 'w' else 1
    lauf = []                                # zusammenhaengende Stuecke mit Rampe hoeher als der Ring
    for e in roh:
        t = fuge(e, seite)
        lauf.append(None if t is None else (e, t, e[seite]['z']))
    s_ges = 0.0
    letzte = None
    for i in range(len(lauf) - 1):
        a, b = lauf[i], lauf[i + 1]
        if a is None or b is None:
            letzte = None
            continue
        (ea, ta, oa), (eb, tb, ob) = a, b
        dicke = 0.5
        # Ring-Seite (Innenkante zum Ring)
        ri_a, ri_b = ta - vz * dicke / 2, tb - vz * dicke / 2
        ra_a, ra_b = ta + vz * dicke / 2, tb + vz * dicke / 2
        za, zb = ea['zr'] - 0.3, eb['zr'] - 0.3
        kappe_a, kappe_b = oa + 0.05, ob + 0.05
        laenge = float(np.linalg.norm(welt(eb, tb, 0) - welt(ea, ta, 0)))
        n_ring = np.append(-vz * ea['quer'], 0)
        p = lambda e, t, z: welt(e, t, z) - o
        # Klinker bis 0,4 m unter die Kappe, darueber Beton
        m.quad('klinker', [p(ea, ri_a, za), p(eb, ri_b, zb), p(eb, ri_b, kappe_b - 0.4), p(ea, ri_a, kappe_a - 0.4)],
               [(s_ges / 2, za), ((s_ges + laenge) / 2, zb), ((s_ges + laenge) / 2, kappe_b - 0.4), (s_ges / 2, kappe_a - 0.4)], n_ring)
        m.quad('beton', [p(ea, ri_a, kappe_a - 0.4), p(eb, ri_b, kappe_b - 0.4), p(eb, ri_b, kappe_b), p(ea, ri_a, kappe_a)],
               [(s_ges / 4, 0), ((s_ges + laenge) / 4, 0), ((s_ges + laenge) / 4, 0.1), (s_ges / 4, 0.1)], n_ring)
        if not (ea['unter_decke'] and eb['unter_decke']):
            # Bruestung 0,5 m ueber der Rampe, Rueckseite zur Rampe, darauf Gelaender bis 1,1 m
            br_a, br_b = kappe_a + 0.5, kappe_b + 0.5
            m.quad('beton', [p(ea, ri_a, kappe_a), p(eb, ri_b, kappe_b), p(eb, ri_b, br_b), p(ea, ri_a, br_a)],
                   [(s_ges / 4, 0), ((s_ges + laenge) / 4, 0), ((s_ges + laenge) / 4, 0.25), (s_ges / 4, 0.25)], n_ring)
            m.quad('beton', [p(ea, ra_a, kappe_a - 0.2), p(eb, ra_b, kappe_b - 0.2), p(eb, ra_b, br_b), p(ea, ra_a, br_a)],
                   [(s_ges / 4, 0), ((s_ges + laenge) / 4, 0), ((s_ges + laenge) / 4, 0.3), (s_ges / 4, 0.3)], -n_ring)
            m.quad('beton', [p(ea, ri_a, br_a), p(eb, ri_b, br_b), p(eb, ra_b, br_b), p(ea, ra_a, br_a)],
                   [(0, 0), (laenge / 4, 0), (laenge / 4, 0.1), (0, 0.1)], (0, 0, 1))
            for z0, z1 in ((0.55, 0.6), (0.25, 0.29)):
                m.quad('gelaender', [p(ea, ta, br_a + z0), p(eb, tb, br_b + z0), p(eb, tb, br_b + z1), p(ea, ta, br_a + z1)],
                       [(0, 0), (1, 0), (1, 1), (0, 1)], n_ring, beidseitig=True)
            for k in range(int(s_ges // 2.5) + 1, int((s_ges + laenge) // 2.5) + 1):
                f = (k * 2.5 - s_ges) / laenge
                q = welt(ea, ta, 0) * (1 - f) + welt(eb, tb, 0) * f - o
                zb_ = br_a * (1 - f) + br_b * f
                kasten(m, 'gelaender', q[0] - 0.03, q[0] + 0.03, q[1] - 0.03, q[1] + 0.03, zb_ - o[2], zb_ + 0.6 - o[2])
        # Pfeiler alle 5 m (Beton, 0,15 m vor der Wand, 0,6 m breit)
        for k in range(int(s_ges // 5) + 1, int((s_ges + laenge) // 5) + 1):
            f = (k * 5 - s_ges) / laenge
            e_ = ea  # Richtung des Abschnitts
            mitte = welt(ea, ri_a, 0) * (1 - f) + welt(eb, ri_b, 0) * f - o
            zf = za * (1 - f) + zb * f
            zk = kappe_a * (1 - f) + kappe_b * f
            d = np.append(e_['d'], 0)
            nq = np.append(-vz * e_['quer'], 0)
            a0 = mitte - d * 0.3
            a1 = mitte + d * 0.3
            v0, v1 = a0 + nq * 0.15, a1 + nq * 0.15
            for q, nn in (([a0, v0, v0, a0], -d), ([v1, a1, a1, v1], d), ([v0, v1, v1, v0], nq)):
                pts = [q[0] + [0, 0, zf], q[1] + [0, 0, zf], q[2] + [0, 0, zk], q[3] + [0, 0, zk]]
                m.quad('beton', pts, [(0, zf / 4), (0.15, zf / 4), (0.15, zk / 4), (0, zk / 4)], nn)
        s_ges += laenge
    return m


def aussenwand(roh, seite, o):
    """Aussenwand neben der Rampe `seite`: West Gabionen + gruene LSW, Ost Klinker (ab 3,5 m zweistufig mit
    Pflanzstreifen) + Glas-LSW"""
    m = Mesh()
    vz = -1 if seite == 'w' else 1
    s_ges = 0.0
    punkte = []
    for e in roh:
        t = aussen(e, seite)
        punkte.append(None if t is None else (e, t, e[seite]['z'], e[seite]['g']))
    p = lambda e, t, z: welt(e, t, z) - o
    for i in range(len(punkte) - 1):
        a, b = punkte[i], punkte[i + 1]
        if a is None or b is None:
            continue
        (ea, ta, za, ga), (eb, tb, zb, gb) = a, b
        laenge = float(np.linalg.norm(welt(eb, tb, 0) - welt(ea, ta, 0)))
        n_innen = np.append(-vz * ea['quer'], 0)          # zur Rampe hin
        hoch_a, hoch_b = max(ga, za) + 0.3, max(gb, zb) + 0.3
        if seite == 'w':
            hoch_a, hoch_b = max(ga + 0.3, za + 1.0), max(gb + 0.3, zb + 1.0)     # Gabionen als Bruestung
        unten_a, unten_b = min(ga, za) - 0.5, min(gb, zb) - 0.5
        if seite == 'w':
            # Gabionen (beide Seiten: die Rampe liegt teils hoeher als das Gelaende)
            m.quad('gabionen', [p(ea, ta, unten_a), p(eb, tb, unten_b), p(eb, tb, hoch_b), p(ea, ta, hoch_a)],
                   [(s_ges / 2, unten_a / 2), ((s_ges + laenge) / 2, unten_b / 2), ((s_ges + laenge) / 2, hoch_b / 2), (s_ges / 2, hoch_a / 2)], n_innen)
            m.quad('gabionen', [p(ea, ta + vz, unten_a), p(eb, tb + vz, unten_b), p(eb, tb + vz, hoch_b), p(ea, ta + vz, hoch_a)],
                   [(s_ges / 2, unten_a / 2), ((s_ges + laenge) / 2, unten_b / 2), ((s_ges + laenge) / 2, hoch_b / 2), (s_ges / 2, hoch_a / 2)], -n_innen)
            m.quad('gabionen', [p(ea, ta, hoch_a), p(eb, tb, hoch_b), p(eb, tb + vz, hoch_b), p(ea, ta + vz, hoch_a)],
                   [(0, 0), (laenge / 2, 0), (laenge / 2, 0.5), (0, 0.5)], (0, 0, 1))
            lsw, lsw_h = 'lsw_gruen', 2.5
            lsw_t = (ta + vz * 0.5, tb + vz * 0.5)
        else:
            stufig = ga - za > 3.5 or gb - zb > 3.5
            if stufig:
                # untere Stufe bis zur halben Hoehe, Pflanzstreifen 1,5 m, obere Stufe zurueckgesetzt
                ma, mb = za + (hoch_a - za) * 0.5, zb + (hoch_b - zb) * 0.5
                stufen = [(ta, tb, unten_a, unten_b, ma, mb), (ta + vz * 1.5, tb + vz * 1.5, ma - 0.3, mb - 0.3, hoch_a, hoch_b)]
                m.quad('pflanzen', [p(ea, ta, ma), p(eb, tb, mb), p(eb, tb + vz * 1.5, mb), p(ea, ta + vz * 1.5, ma)],
                       [(0, 0), (laenge / 2, 0), (laenge / 2, 0.75), (0, 0.75)], (0, 0, 1))
            else:
                stufen = [(ta, tb, unten_a, unten_b, hoch_a, hoch_b)]
            for (t0, t1, u0, u1, k0, k1) in stufen:
                m.quad('klinker', [p(ea, t0, u0), p(eb, t1, u1), p(eb, t1, k1 - 0.3), p(ea, t0, k0 - 0.3)],
                       [(s_ges / 2, u0), ((s_ges + laenge) / 2, u1), ((s_ges + laenge) / 2, k1 - 0.3), (s_ges / 2, k0 - 0.3)], n_innen)
                m.quad('beton', [p(ea, t0, k0 - 0.3), p(eb, t1, k1 - 0.3), p(eb, t1, k1), p(ea, t0, k0)],
                       [(s_ges / 4, 0), ((s_ges + laenge) / 4, 0), ((s_ges + laenge) / 4, 0.08), (s_ges / 4, 0.08)], n_innen)
                m.quad('beton', [p(ea, t0, k0), p(eb, t1, k1), p(eb, t1 + vz * 0.35, k1), p(ea, t0 + vz * 0.35, k0)],
                       [(0, 0), (laenge / 4, 0), (laenge / 4, 0.1), (0, 0.1)], (0, 0, 1))
            lsw, lsw_h = 'glas', 2.5
            lsw_t = (stufen[-1][0] + vz * 0.2, stufen[-1][1] + vz * 0.2)
        # Laermschutzwand oben (nur wo die Wand das Gelaende haelt: Gelaende ueber der Rampe)
        if seite == 'w' or ga > za + 1.0 and gb > zb + 1.0:
            m.quad(lsw, [p(ea, lsw_t[0], hoch_a), p(eb, lsw_t[1], hoch_b), p(eb, lsw_t[1], hoch_b + lsw_h), p(ea, lsw_t[0], hoch_a + lsw_h)],
                   [(s_ges / 2, 0), ((s_ges + laenge) / 2, 0), ((s_ges + laenge) / 2, 1), (s_ges / 2, 1)], n_innen, beidseitig=True)
            if int((s_ges + laenge) // 3) > int(s_ges // 3):     # Pfosten alle 3 m
                q = welt(eb, lsw_t[1], 0) - o
                kasten(m, 'gelaender', q[0] - 0.08, q[0] + 0.08, q[1] - 0.08, q[1] + 0.08, hoch_b, hoch_b + lsw_h + 0.05)
        s_ges += laenge
    return m


def decke(roh, ludwig_z, o):
    """Unterfuehrung: Betonplatte ueber dem Ring zwischen den Trennwaenden, an beiden Stirnseiten halbrund
    vorspringend (Luftbild), Unterseite dunkler Beton, Stirnseiten Beton"""
    m = Mesh()
    unter = [e for e in roh if e['unter_decke']]
    if not unter:
        return m
    e_n, e_s = unter[0], unter[-1]
    breite_l = min(e['ra'] for e in unter) - 0.5
    breite_r = max(e['rb'] for e in unter) + 0.5
    r = (breite_r - breite_l) / 2.0
    oben = ludwig_z - 0.05
    dicke = 1.1
    # Umriss: gerade Seiten an den Trennwaenden, Halbkreise an Nord- und Suedkante (Mittelpunkte auf der Achse)
    def umriss():
        pts = []
        for e_, sgn in ((e_n, 1), (e_s, -1)):
            mitte = e_['p'] + e_['quer'] * (breite_l + r)
            d = -e_['d'] * sgn                        # nach aussen (Nord bei e_n)
            for k in range(17):
                w = math.pi * k / 16
                q = mitte + e_['quer'] * (math.cos(w) * r * sgn) + d * (math.sin(w) * r * 0.6)
                pts.append(q)
        return pts
    U = [np.array([q[0], q[1]]) - o[:2] for q in umriss()]
    n = len(U)
    c = sum(U) / n
    for i in range(n):
        a, b = U[i], U[(i + 1) % n]
        # Stirn/Seiten
        aussen = np.array([b[1] - a[1], a[0] - b[0]])
        if np.dot(aussen, (a + b) / 2 - c) < 0:
            aussen = -aussen
        L = np.linalg.norm(b - a)
        m.quad('beton', [(a[0], a[1], oben - dicke - o[2]), (b[0], b[1], oben - dicke - o[2]), (b[0], b[1], oben - o[2] + 0.3), (a[0], a[1], oben - o[2] + 0.3)],
               [(0, 0), (L / 4, 0), (L / 4, 0.35), (0, 0.35)], (aussen[0], aussen[1], 0))
        # Unterseite (Faecher)
        m.quad('decke', [(c[0], c[1], oben - dicke - o[2]), (b[0], b[1], oben - dicke - o[2]), (a[0], a[1], oben - dicke - o[2]), (c[0], c[1], oben - dicke - o[2])],
               [(c[0] / 4, c[1] / 4), (b[0] / 4, b[1] / 4), (a[0] / 4, a[1] / 4), (c[0] / 4, c[1] / 4)], (0, 0, -1))
        # Oberseite (bis Etappe 3 die Kreuzung darueber liegt)
        m.quad('beton', [(c[0], c[1], oben - o[2]), (a[0], a[1], oben - o[2]), (b[0], b[1], oben - o[2]), (c[0], c[1], oben - o[2])],
               [(c[0] / 4, c[1] / 4), (a[0] / 4, a[1] / 4), (b[0] / 4, b[1] / 4), (c[0] / 4, c[1] / 4)], (0, 0, 1))
    return m


def bahnbruecke(ways, dgm, E0, N0, nn0, o):
    """Bruecke der Ludwigsallee (Schiesshausbruecke) ueber die Bahn: Platte, Bruestungen mit Gelaender, Widerlager.
    Lage aus dem DGM: der Einschnitt der Bahn unter der Strasse (x ~1046..1068)."""
    m = Mesh()
    x0, x1 = 1045.0, 1068.5          # Widerlager (Westen, Osten)
    y0, y1 = -276.0, -256.5          # Bruestungen (Sued, Nord)
    hz = dgm.hoehen(np.array([E0 + 1040.0, E0 + 1072.0]), np.array([N0 - 266.0, N0 - 266.0])) - nn0
    oben = float(np.mean(hz)) - 0.05
    xs = np.arange(x0, x1, 1.0)
    sohle = float(np.nanmin(dgm.hoehen(E0 + xs, np.full(len(xs), N0 - 266.0)))) - nn0
    P = lambda x, y, z: (x - o[0], y - o[1], z - o[2])
    dicke = 1.2
    # Platte: Unterseite und Seiten
    m.quad('decke', [P(x0, y1, oben - dicke), P(x1, y1, oben - dicke), P(x1, y0, oben - dicke), P(x0, y0, oben - dicke)],
           [(0, 0), ((x1 - x0) / 4, 0), ((x1 - x0) / 4, (y1 - y0) / 4), (0, (y1 - y0) / 4)], (0, 0, -1))
    for y, n in ((y0, (0, -1, 0)), (y1, (0, 1, 0))):
        m.quad('beton', [P(x0, y, oben - dicke), P(x1, y, oben - dicke), P(x1, y, oben + 1.1), P(x0, y, oben + 1.1)],
               [(0, 0), ((x1 - x0) / 4, 0), ((x1 - x0) / 4, 0.6), (0, 0.6)], n)
        # Bruestung (0,3 m dick) mit Gelaender
        yi = y + (0.3 if y == y0 else -0.3)
        m.quad('beton', [P(x0, yi, oben), P(x1, yi, oben), P(x1, yi, oben + 1.1), P(x0, yi, oben + 1.1)],
               [(0, 0), ((x1 - x0) / 4, 0), ((x1 - x0) / 4, 0.3), (0, 0.3)], (0, -n[1], 0))
        m.quad('beton', [P(x0, min(y, yi), oben + 1.1), P(x1, min(y, yi), oben + 1.1), P(x1, max(y, yi), oben + 1.1), P(x0, max(y, yi), oben + 1.1)],
               [(0, 0), ((x1 - x0) / 4, 0), ((x1 - x0) / 4, 0.1), (0, 0.1)], (0, 0, 1))
        ym = (y + yi) / 2
        for z0 in (1.6, 2.1):
            m.quad('gelaender', [P(x0, ym, oben + z0), P(x1, ym, oben + z0), P(x1, ym, oben + z0 + 0.06), P(x0, ym, oben + z0 + 0.06)],
                   [(0, 0), (1, 0), (1, 1), (0, 1)], n, beidseitig=True)
        for x in np.arange(x0 + 1.0, x1, 2.0):
            kasten(m, 'gelaender', x - o[0] - 0.04, x - o[0] + 0.04, ym - o[1] - 0.04, ym - o[1] + 0.04, oben + 1.1 - o[2], oben + 2.16 - o[2])
    # Widerlager: Waende von der Sohle bis unter die Platte, zur Bahn hin Klinker
    for x, n in ((x0, (1, 0, 0)), (x1, (-1, 0, 0))):
        m.quad('klinker', [P(x, y0, sohle - 0.5), P(x, y1, sohle - 0.5), P(x, y1, oben - dicke), P(x, y0, oben - dicke)],
               [(0, sohle), ((y1 - y0) / 2, sohle), ((y1 - y0) / 2, oben - dicke), (0, oben - dicke)], n, beidseitig=True)
    return m, oben


def profil(e):
    """Querschnitt der Fahrflaechen an einer Station: [(t0, t1, z)] West-Rampe, Ring, Ost-Rampe (Stufen an den
    Trennwaenden; wo die Rampe auf Ring-Hoehe einmuendet, ohne Trennwand)"""
    zr = e['zr']
    zw = e['w']['z'] if e['w'] else zr
    zo = e['o']['z'] if e['o'] else zr
    tw, to = rand(e, 'w'), rand(e, 'o')
    fw, fo = fuge(e, 'w'), fuge(e, 'o')
    fw = e['ra'] - 0.3 if fw is None else fw
    fo = e['rb'] + 0.3 if fo is None else fo
    fw, fo = max(fw, tw), min(fo, to)
    return [(tw, fw, zw), (fw, fo, zr), (fo, to, zo)]


def flaechen(roh, o):
    """Sohle des Trogs und Platten der Rampen, 5 cm unter den Fahrbahnen (darauf kommen in Etappe 2 die Strassen):
    decken das abgesenkte Gelaende ab"""
    m = Mesh()
    p = lambda e, t, z: welt(e, t, z - 0.05) - o
    for ea, eb in zip(roh[:-1], roh[1:]):
        pa, pb = profil(ea), profil(eb)
        for (a0, a1, za), (b0, b1, zb) in zip(pa, pb):
            if a1 - a0 < 0.05 and b1 - b0 < 0.05:
                continue
            q = [p(ea, a0, za), p(eb, b0, zb), p(eb, b1, zb), p(ea, a1, za)]
            m.quad('asphalt', q, [((c[0] + o[0]) / 5, (c[1] + o[1]) / 5) for c in q], (0, 0, 1))
        # Stufen zwischen den Flaechen (wo keine Trennwand steht, sind sie niedrig)
        for k in (0, 1):
            ta, tb = pa[k][1], pb[k][1]
            q = [p(ea, ta, pa[k][2]), p(eb, tb, pb[k][2]), p(eb, tb, pb[k + 1][2]), p(ea, ta, pa[k + 1][2])]
            m.quad('beton', q, [(0, 0), (0.5, 0), (0.5, 0.2), (0, 0.2)], None, beidseitig=True)
    return m


def aussen_kante(e, seite):
    """(t, z) der Aussenkante des Bauwerks: hinter der Wand auf dem Gelaende, sonst die Rampe"""
    vz = -1 if seite == 'w' else 1
    t = aussen(e, seite)
    r = e[seite]
    if t is not None:
        dicke = 1.0 if seite == 'w' else 0.35 + (1.5 if r['g'] - r['z'] > 3.5 else 0.0)
        return t + vz * dicke, r['g']
    pr = profil(e)
    return (pr[0][0], pr[0][2]) if seite == 'w' else (pr[2][1], pr[2][2])


class Kartengelaende:
    """Gelaende der Karte (.map.terrain, 61 x 61, Zeile 0 = Sueden) wie gezeichnet: je Zelle zwei Dreiecke mit der
    Diagonale (i, j)-(i+1, j+1) (openOMSI Terrain::sample)"""

    def __init__(self, karten_ordner):
        self.ordner, self.kacheln = karten_ordner, {}

    def kachel(self, kx, ky):
        if (kx, ky) not in self.kacheln:
            pfad = os.path.join(self.ordner, f'tile_{kx}_{ky}.map.terrain')
            a = None
            if os.path.isfile(pfad):
                b = np.fromfile(pfad, dtype='<f4', offset=4)
                n = int(np.fromfile(pfad, dtype='<i4', count=1)[0]) + 1
                a = b[:n * n].reshape(n, n).astype(float)
            self.kacheln[(kx, ky)] = a
        return self.kacheln[(kx, ky)]

    def hoehe(self, x, y):
        kx, ky = math.floor(x / 300), math.floor(y / 300)
        a = self.kachel(kx, ky)
        if a is None:
            return None
        n = a.shape[0] - 1
        gx, gy = (x - kx * 300) / 300 * n, (y - ky * 300) / 300 * n
        i, j = min(int(gx), n - 1), min(int(gy), n - 1)
        fx, fy = gx - i, gy - j
        h00, h10, h01, h11 = a[j, i], a[j, i + 1], a[j + 1, i], a[j + 1, i + 1]
        if fx >= fy:
            return h00 + fx * (h10 - h00) + fy * (h11 - h10)
        return h00 + fy * (h01 - h00) + fx * (h11 - h01)


def deckel(roh, karte, o, breite=7.0):
    """Deckel neben dem Bauwerk: vom Rand (hinter der Aussenwand auf Gelaendehoehe) 7 m nach aussen bis auf das
    unveraenderte Gelaende - verdecken die Schraege des abgesenkten 5-m-Gelaenderasters (wie bauwerke.rs)"""
    m = Mesh()
    teile = 4
    for seite in ('w', 'o'):
        vz = -1 if seite == 'w' else 1
        reihen = []
        for e in roh:
            if im_knoten(e):
                reihen.append(None)
                continue
            t0, z0 = aussen_kante(e, seite)
            ts = t0 + vz * np.linspace(0, breite, teile + 1)
            xy = e['p'][None, :] + ts[:, None] * e['quer'][None, :]
            zs = [karte.hoehe(x, y) for x, y in xy]
            zs = [z0 if z is None or i == 0 else z for i, z in enumerate(zs)]
            reihen.append([np.array([x, y, z + 0.05]) - o for (x, y), z in zip(xy, zs)])
        for ra, rb in zip(reihen[:-1], reihen[1:]):
            if ra is None or rb is None:
                continue
            for i in range(teile):
                q = [ra[i], rb[i], rb[i + 1], ra[i + 1]]
                m.quad('boden', q, [((c[0] + o[0]) / 5, (c[1] + o[1]) / 5) for c in q], (0, 0, 1))
    return m


def gelaende(roh):
    """abgesenkte Gelaendepunkte (5-m-Raster der Kacheln): unter Sohle und Rampen 0,8 m unter der tiefsten Fahrbahn
    -> [[kx, ky, spalte, zeile, z]] (Zeile 0 = Sueden, Hoehe in der Karte)"""
    P = np.array([e['p'] for e in roh])
    out = []
    xs, ys = P[:, 0], P[:, 1]
    for kx in range(int(math.floor((xs.min() - 30) / 300)), int(math.floor((xs.max() + 30) / 300)) + 1):
        for ky in range(int(math.floor((ys.min() - 30) / 300)), int(math.floor((ys.max() + 30) / 300)) + 1):
            for zeile in range(61):
                for spalte in range(61):
                    x, y = kx * 300 + spalte * 5.0, ky * 300 + zeile * 5.0
                    if not (xs.min() - 30 <= x <= xs.max() + 30 and ys.min() - 5 <= y <= ys.max() + 5):
                        continue
                    i = int(np.argmin(np.hypot(xs - x, ys - y)))
                    if i < 3 or i > len(roh) - 4:
                        continue
                    e = roh[i]
                    v = np.array([x, y]) - e['p']
                    if abs(np.dot(v, e['d'])) > SCHRITT:
                        continue
                    t = float(np.dot(v, e['quer']))
                    pr = profil(e)
                    lo, hi = pr[0][0], pr[2][1]
                    if lo - 0.3 <= t <= hi + 0.3:
                        out.append([kx, ky, spalte, zeile, round(min(z for _, _, z in pr) - 0.8, 3)])
    return out


# ------------------------------------------------------------------ Texturen
def texturen(omsi, ziel):
    from PIL import Image, ImageDraw
    os.makedirs(ziel, exist_ok=True)
    marcel = os.path.join(omsi, 'Splines', 'Marcel', 'texture')
    rng = np.random.default_rng(7)
    shutil.copy(os.path.join(marcel, 'betonwand1.bmp'), os.path.join(ziel, TEX['beton']))
    # dunkler Beton (Deckenunterseite)
    b = np.asarray(Image.open(os.path.join(marcel, 'betonwand1.bmp')).convert('RGB'), float) * 0.55
    Image.fromarray(b.clip(0, 255).astype(np.uint8)).save(os.path.join(ziel, TEX['decke']))
    # Klinker im Laeuferverband (256 px = 2 m x 1 m), wie am Ring rotbraun mit hellen Fugen
    w, h = 256, 256
    img = np.zeros((h, w, 3))
    for y in range(h):
        reihe = y // 19
        for x in range(w):
            xv = x + (16 if reihe % 2 else 0)
            fuge = y % 19 >= 17 or xv % 32 >= 30
            if fuge:
                img[y, x] = (150, 146, 138)
            else:
                ton = ((xv // 32) * 2654435761 + reihe * 97) % 30
                img[y, x] = (130 + ton, 56 + ton / 2, 44 + ton / 4)
    img += rng.normal(0, 6, img.shape)
    Image.fromarray(img.clip(0, 255).astype(np.uint8)).save(os.path.join(ziel, TEX['klinker']))
    # Gabionen: Steine (grau-beige) hinter Drahtgitter (256 px = 2 m x 2 m)
    g = Image.new('RGB', (256, 256), (120, 112, 98))
    d = ImageDraw.Draw(g)
    for _ in range(420):
        x, y = rng.integers(0, 256, 2)
        r = rng.integers(5, 14)
        c = tuple(int(v) for v in rng.integers(115, 200, 1)[0] + rng.integers(-12, 12, 3))
        d.ellipse([x - r, y - r * 0.7, x + r, y + r * 0.7], fill=c, outline=(80, 74, 66))
    for k in range(0, 256, 13):
        d.line([(k, 0), (k, 255)], fill=(70, 70, 70), width=1)
        d.line([(0, k), (255, k)], fill=(70, 70, 70), width=1)
    for k in (0, 127):
        d.line([(0, k), (255, k)], fill=(60, 60, 60), width=3)
    g.save(os.path.join(ziel, TEX['gabionen']))
    # gruene Laermschutzwand: Metallpaneele mit senkrechten Rippen
    lg = np.zeros((128, 128, 3))
    for x in range(128):
        lg[:, x] = (62, 112, 92) if x % 8 >= 2 else (44, 86, 70)
    lg[60:64, :] = (40, 70, 58)
    Image.fromarray((lg + rng.normal(0, 3, lg.shape)).clip(0, 255).astype(np.uint8)).save(os.path.join(ziel, TEX['lsw_gruen']))
    # Glas: leicht blaeulich, durchsichtig (Alpha), Rahmen oben/unten
    gl = Image.new('RGBA', (64, 64), (200, 225, 235, 70))
    dg = ImageDraw.Draw(gl)
    dg.rectangle([0, 0, 63, 3], fill=(150, 155, 160, 255))
    dg.rectangle([0, 60, 63, 63], fill=(150, 155, 160, 255))
    gl.save(os.path.join(ziel, TEX['glas']))
    Image.new('RGB', (8, 8), (150, 82, 110)).save(os.path.join(ziel, TEX['gelaender']))
    # Bodendecker im Pflanzstreifen
    p = (rng.normal(0, 18, (128, 128, 3)) + (70, 105, 55)).clip(0, 255).astype(np.uint8)
    Image.fromarray(p).save(os.path.join(ziel, TEX['pflanzen']))
    shutil.copy(os.path.join(marcel, 'str_asphdrk.bmp'), os.path.join(ziel, TEX['asphalt']))
    # Deckel wie das Gelaende (openOMSI: Bodentextur mal Detailtextur, Detail 60 x je Kachel = alle 5 m):
    # Detailtextur eingefaerbt mit der mittleren Farbe der Bodentextur (wie bauwerke.rs deckel_textur)
    basis = np.asarray(Image.open(os.path.join(omsi, 'Texture', 'gras.bmp')).convert('RGB'), float)
    detail = np.asarray(Image.open(os.path.join(omsi, 'Texture', 'gras_det.bmp')).convert('RGB'), float)
    mittel = basis.reshape(-1, 3).mean(axis=0)
    Image.fromarray((detail * mittel / 255.0).clip(0, 255).astype(np.uint8)).save(os.path.join(ziel, TEX['boden']))


# ------------------------------------------------------------------ Ausgabe
def sco_text(name, modell, mats):
    L = ['Erzeugt mit omsigen (sonderbau_ring). OSM (ODbL), DGM1/DOP40 Bayern (CC BY 4.0).', '', '[friendlyname]', name, '',
         '[groups]', '1', 'Aschaffenburg', '', '[fixed]', '', '[absheight]', '', '[mesh]', modell, '']
    for key in mats:
        if TEX[key].endswith('.tga'):
            L += ['[matl]', TEX[key], '0', '[matl_alpha]', '2', '']
    return '\r\n'.join(L) + '\r\n'


def schreiben(ordner, rel_ordner, name, mesh, o, liste):
    if mesh.leer():
        return
    mats = [(k, TEX[k]) for k in mesh.mats]
    with open(os.path.join(ordner, 'model', name + '.x'), 'w', newline='') as f:
        f.write(x_file(mesh.V, mesh.F, mats))
    with open(os.path.join(ordner, name + '.sco'), 'w', encoding='cp1252', newline='') as f:
        f.write(sco_text(f'Ring/Ludwigsallee {name}', name + '.x', mesh.mats))
    liste.append(dict(name=name, rel=rel_ordner + '\\' + name + '.sco', x=float(o[0]), y=float(o[1]), z=float(o[2]), dreiecke=len(mesh.F)))


def bauen(omsi, karte, osm_pfad, nn0, ost0, nord0, cache):
    ordner = os.path.join(omsi, 'Sceneryobjects', 'Aschaffenburg', karte, 'Sonderbau_Ring')
    rel = f'Sceneryobjects\\Aschaffenburg\\{karte}\\Sonderbau_Ring'
    os.makedirs(os.path.join(ordner, 'model'), exist_ok=True)
    texturen(omsi, os.path.join(ordner, 'texture'))
    ways = lade_osm(osm_pfad, ost0, nord0)
    dgm = Dgm1(cache)
    roh = vermessen(ways, dgm, ost0, nord0, nn0)
    karte_gel = Kartengelaende(os.path.join(omsi, 'maps', karte))
    liste = []
    # Waende in Stuecken von STUECK m (Ursprung je Stueck: erster Punkt auf der Ring-Achse, Hoehe 0)
    n = int(STUECK / SCHRITT)
    for k in range(0, len(roh) - 1, n):
        teil = roh[k:k + n + 1]
        o = np.array([teil[0]['p'][0], teil[0]['p'][1], 0.0])
        for seite in ('w', 'o'):
            schreiben(ordner, rel, f'Trennwand_{seite}_{k // n:02d}', trennwand(teil, seite, o), o, liste)
            schreiben(ordner, rel, f'Aussenwand_{seite}_{k // n:02d}', aussenwand(teil, seite, o), o, liste)
        schreiben(ordner, rel, f'Sohle_{k // n:02d}', flaechen(teil, o), o, liste)
        schreiben(ordner, rel, f'Deckel_{k // n:02d}', deckel(teil, karte_gel, o), o, liste)
    ludwig_z = float(np.median(dgm.hoehen(ost0 + np.arange(1078.0, 1092.0), np.full(14, nord0 - 266.0)))) - nn0
    unter = [e for e in roh if e['unter_decke']]
    o = np.array([unter[0]['p'][0], unter[0]['p'][1], 0.0]) if unter else np.zeros(3)
    schreiben(ordner, rel, 'Unterfuehrung', decke(roh, ludwig_z, o), o, liste)
    o = np.array([1056.0, -266.0, 0.0])
    bb, _ = bahnbruecke(ways, dgm, ost0, nord0, nn0, o)
    schreiben(ordner, rel, 'Bahnbruecke', bb, o, liste)
    messung = [dict(x=float(e['p'][0]), y=float(e['p'][1]), ring=[float(e['ra']), float(e['rb']), float(e['zr'])],
                    west=None if e['w'] is None else [e['w'][k] for k in ('a', 'b', 'z', 'g')],
                    ost=None if e['o'] is None else [e['o'][k] for k in ('a', 'b', 'z', 'g')], decke=bool(e['unter_decke'])) for e in roh]
    out = dict(karte=karte, nn0=nn0, objekte=liste, gelaende=gelaende(roh), messung=messung, quellen=['OpenStreetMap (ODbL)', 'DGM1/DOP40 Bayerische Vermessungsverwaltung (CC BY 4.0)'])
    with open(os.path.join(ordner, 'sonderbau_ring.json'), 'w', encoding='utf-8') as f:
        json.dump(out, f, indent=1)
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--omsi', required=True)
    ap.add_argument('--karte', required=True)
    ap.add_argument('--osm', required=True)
    ap.add_argument('--cache', default=os.path.join(os.environ.get('LOCALAPPDATA', '.'), 'omsi-editor', 'geodaten'))
    a = ap.parse_args()
    geo = {}
    with open(os.path.join(a.omsi, 'maps', a.karte, 'omsi-editor-geo.cfg'), encoding='utf-8') as f:
        for z in f:
            if '=' in z and not z.startswith('#'):
                k, v = z.split('=', 1)
                geo[k.strip()] = v.strip()
    out = bauen(a.omsi, a.karte, a.osm, float(geo['nn0']), float(geo['ost0']), float(geo['nord0']), a.cache)
    for o in out['objekte']:
        print(f"{o['name']:24s} {o['dreiecke']:6d} Dreiecke  bei {o['x']:.1f} {o['y']:.1f}")
    print(f"{len(out['objekte'])} Objekte, {len(out['messung'])} Querschnitte")


if __name__ == '__main__':
    sys.exit(main())
