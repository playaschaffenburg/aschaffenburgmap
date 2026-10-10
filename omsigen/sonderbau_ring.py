"""Sonderbau Dr.-Willi-Reiland-Ring / Ludwigsallee (Aschaffenburg), Etappe 1: das Bauwerk.

Ein Einzelstueck fuer genau diese Stelle (siehe sonderbau/ring/PLAN.md): der Ring im Trog, die Rampen links und rechts
auf Stuetzmauern, die Unterfuehrung unter der Ludwigsallee und die Bruecke der Ludwigsallee ueber die Bahn.

Vorgehen: alles folgt dem Entwurf (omsigen.ring_entwurf -> sonderbau/ring/entwurf/entwurf.json): Ring-Achse aus
Geraden und Boegen, Wandlinien in festem Abstand zur Achse, Hoehen als Gradienten mit Ausrundung. Alle 2 m Station
entlang der Achse; die Waende:
- Trennwaende Ring/Rampe: Stuetzwand von der Ring-Sohle bis zur Rampe, zur Ring-Seite Klinkerfelder zwischen
  Betonpfeilern, oben Betonkappe, Bruestung mit Gelaender zur Rampe hin.
- Aussenwand West: Gabionen, darauf eine gruene Laermschutzwand.
- Aussenwand Ost: hohe Klinker-Stuetzmauer, ab 3,5 m zweistufig mit Pflanzstreifen, oben eine Glas-Laermschutzwand.
- Unterfuehrung: Betondecke ueber dem Ring mit halbrund vorspringenden Stirnseiten (wie im Luftbild).
- Bahnbruecke: Platte, Bruestungen mit Gelaender, Widerlager.

Ausgabe: Objekte (.sco + model/*.x + texture/) im Ordner der Karte und `sonderbau_ring.json` mit ihrer Lage
(Kartenkoordinaten, Hoehe absolut in der Karte = NN - nn0). Aufruf:
    python -m omsigen.sonderbau_ring --omsi <OMSI-Ordner> --karte Aburg_Newgen --osm sonderbau/ring/osm_map.xml
        [--entwurf sonderbau/ring/entwurf/entwurf.json]
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
DECKE = (-248.9, -284.0)                               # Unterfuehrung: wo die Trogwaende an die Decke stossen (y)
S_PLATTE = (107.0, 161.0)
LSW_FREI = (96.0, 166.0)                               # ohne Laermschutzwand: die Rampen an der Kreuzung (Fotos)                              # Achs-Stationen unter dem Kreuzungsobjekt (ohne Rampenplatten)
SCHRITT = 2.0
STUECK = 40.0                                          # m je Wand-Objekt

TEX = {'beton': 'betonwand1.bmp', 'klinker': 'AB_klinker.bmp', 'gabionen': 'AB_gabionen.bmp',
       'lsw_gruen': 'AB_lsw_gruen.bmp', 'glas': 'AB_glas.tga', 'gelaender': 'AB_gelaender.bmp', 'stab': 'AB_stabgelaender.tga',
       'decke': 'AB_beton_dunkel.bmp', 'pflanzen': 'AB_bodendecker.bmp', 'asphalt': 'str_asphdrk.bmp',
       'boden': 'AB_boden.bmp', 'weiss': 'AB_weiss.bmp', 'radfurt': 'AB_radfurt.bmp', 'sperr': 'AB_sperrflaeche.tga',
       'gehweg': 'str_gehweg02.bmp', 'bord': 'str_side1.bmp', 'busbeton': 'str_betonplatten.bmp',
       'gehweg_asphalt': 'AB_gehweg_asphalt.bmp', 'insel_beton': 'AB_insel_beton.bmp'}


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

    def dreieck(self, key, p, uv, n):
        """Dreieck p0..p2 (je (x, y, z)), sichtbar zur Seite n"""
        p = [np.asarray(q, float) for q in p]
        n = np.asarray(n, float)
        n = n / np.linalg.norm(n)
        m = self.mat(key)
        X = [(q[0], q[2], q[1]) for q in p]
        N = (n[0], n[2], n[1])
        a, b, c = (np.array(X[i]) for i in range(3))
        if np.linalg.norm(np.cross(b - a, c - a)) < 1e-9:
            return
        vorne = np.dot(np.cross(b - a, c - a), N) >= 0
        i0 = len(self.V)
        for (x, y, z), (u, v) in zip(X, uv):
            self.V.append((x, y, z) + tuple(N) + (u, v))
        self.F.append((i0, i0 + 1, i0 + 2, m) if vorne else (i0, i0 + 2, i0 + 1, m))

    def leer(self):
        return not self.F


def band(mesh, key, unten, oben, normale, s0, tex=(2.0, 1.0)):
    """senkrechte Flaeche zwischen zwei Linien (je Liste (x, y, z)), Normale je Abschnitt; uv: Laenge/Hoehe"""
    for i in range(len(unten) - 1):
        a, b, c, d = unten[i], unten[i + 1], oben[i + 1], oben[i]
        sa, sb = s0[i], s0[i + 1]
        mesh.quad(key, [a, b, c, d], [(sa / tex[0], a[2] / tex[1]), (sb / tex[0], b[2] / tex[1]),
                                      (sb / tex[0], c[2] / tex[1]), (sa / tex[0], d[2] / tex[1])], normale[i])


def stabgelaender(mesh, p0, p1, z0, z1, hoehe=1.0, u0=0.0, pfosten=2.5):
    """Stabgelaender wie am Ring (rosa, senkrechte Staebe, Holm oben): eine durchscheinende Flaeche von p0 nach p1
    (Fuss z0/z1, je (x, y) im Objektrahmen), Pfosten alle `pfosten` m. u0: Laenge davor (Staebe laufen durch)"""
    p0, p1 = np.asarray(p0, float)[:2], np.asarray(p1, float)[:2]
    L = float(np.linalg.norm(p1 - p0))
    if L < 1e-3:
        return
    n = np.array([p1[1] - p0[1], p0[0] - p1[0]]) / L
    q = [(p0[0], p0[1], z0), (p1[0], p1[1], z1), (p1[0], p1[1], z1 + hoehe), (p0[0], p0[1], z0 + hoehe)]
    mesh.quad('stab', q, [(u0, 0), (u0 + L, 0), (u0 + L, 1), (u0, 1)], (n[0], n[1], 0), beidseitig=True)
    k0 = int(np.ceil(u0 / pfosten))
    while k0 * pfosten <= u0 + L:
        f = (k0 * pfosten - u0) / L
        c = p0 + (p1 - p0) * f
        zf = z0 + (z1 - z0) * f
        kasten(mesh, 'gelaender', c[0] - 0.03, c[0] + 0.03, c[1] - 0.03, c[1] + 0.03, zf, zf + hoehe + 0.02)
        k0 += 1


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


def trennstreifen(seite, s):
    """Ringwand (Sichtflaeche) bis Fahrbahnkante der Rampe nach Luftbild (ring_strassen.RAMPEN), Nord/Sued der Kreuzung"""
    from .ring_strassen import RAMPEN
    nord = s < 131.0
    return next(r['trenn'] for r in RAMPEN.values() if r['seite'] == seite and (r['s'][1] is not None) == nord)


WAND = 0.5               # Dicke der Trennwand


def aus_entwurf(entwurf, dgm, E0, N0, nn0, schritt=SCHRITT):
    """Stationen aus dem Entwurf (ring_entwurf: Achse, Kantenabstaende, Gradienten) - alles glatt, parallel zur
    Achse. Je Station dasselbe Format wie frueher die Messung: p, quer (nach Osten), d (Nord -> Sued), s, zr (Ring),
    ra/rb (Ringwaende West/Ost, quer gemessen), w/o: Rampe mit a/b (Kanten West/Ost), z (Hoehe), g (Gelaende hinter der
    Aussenwand, als Gradiente geglaettet)"""
    from .sonderbau_entwurf import (achse_aus_json, gradiente_aus_json, versatz_aus_json, weich_max,
                                    gradiente_anpassen, ohne_ausreisser)
    achse = achse_aus_json(entwurf['achse'])
    K = {k: versatz_aus_json(v) for k, v in entwurf['kanten'].items()}
    G = {k: gradiente_aus_json(v) for k, v in entwurf['hoehen'].items()}
    # Bereich: die ganze Achse - ueber die gemessenen Waende hinaus, bis die Rampen eingemuendet sind und der Ring
    # weiterlaeuft (die Waende enden von selbst, wo Rampe und Gelaende auf Ring-Hoehe liegen)
    S = np.arange(0.0, achse.laenge + 1e-6, schritt)
    A = achse.punkte(S)
    zr = G['ring'].z(S)
    # Kanten: Entwurf t > 0 = rechts von Nord -> Sued = West; hier quer nach Osten -> Vorzeichen drehen
    tq = {}
    for k, v in K.items():
        t = -v.t(S)
        ok = ~np.isnan(t)
        # vor/hinter der Messung: letzter Abstand bleibt (Kante laeuft parallel weiter)
        tq[k] = np.interp(S, S[ok], t[ok]) if ok.any() else t
    zw = {sd: weich_max(np.where((S >= G[f'rampe_{sd}'].k[0] - 30) & (S <= G[f'rampe_{sd}'].k[-1] + 30), G[f'rampe_{sd}'].z(S), -99), zr) for sd in ('w', 'o')}
    roh = []
    for i, s in enumerate(S):
        x, y, th = A[i]
        d = np.array([math.cos(th), math.sin(th)])
        quer = np.array([-d[1], d[0]])
        if quer[0] < 0:
            quer = -quer
        ra = tq['ring_w'][i] if not np.isnan(tq['ring_w'][i]) else -4.6
        rb = tq['ring_o'][i] if not np.isnan(tq['ring_o'][i]) else 4.6
        e = dict(s=float(s), p=np.array([x, y]), quer=quer, d=d, zr=float(zr[i]), ra=float(ra), rb=float(rb),
                 unter_decke=DECKE[1] <= y <= DECKE[0])
        for sd, vz in (('w', -1), ('o', 1)):
            aussen_t = tq[f'rampe_aussen_{sd}'][i]
            if np.isnan(aussen_t):
                aussen_t = (ra if vz < 0 else rb) + vz * (trennstreifen(sd, s) + 6.5)
            innen = (ra if vz < 0 else rb) + vz * trennstreifen(sd, s)
            a, b = (aussen_t, innen) if vz < 0 else (innen, aussen_t)
            e[sd] = dict(a=float(a), b=float(b), z=float(zw[sd][i]), g=np.nan)
        roh.append(e)
    # Gelaende hinter den Aussenwaenden (3 m), je Seite als Gradiente geglaettet - Mauerkronen ohne Wellen
    for sd, vz in (('w', -1), ('o', 1)):
        t = np.array([(e[sd]['a'] if vz < 0 else e[sd]['b']) + vz * 3.0 for e in roh])
        P = np.array([e['p'] + e['quer'] * tt for e, tt in zip(roh, t)])
        g = dgm.hoehen(E0 + P[:, 0], N0 + P[:, 1]) - nn0
        knoten = np.array([im_knoten(e) for e in roh])
        g = np.where(knoten, np.nan, g)
        gr, _ = ohne_ausreisser(S, g, lambda a, b: gradiente_anpassen(a, b, toleranz=0.2, max_stuecke=8, min_laenge=10.0),
                                lambda gg, x: gg.z(x), 0.8)
        for e, wert in zip(roh, gr.z(S)):
            e[sd]['g'] = float(wert)
    return roh, achse


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
    # die Ringwand-Kante ist die Sichtflaeche zum Ring; die Wand steht dahinter
    return (e['ra'] if vz < 0 else e['rb']) + vz * WAND / 2


def im_knoten(e, seite=None):
    """Station im Bereich der Ludwigsallee (dort keine Aussenwaende, die Kreuzung liegt darueber); West: bis an die
    Eckbruestungen des Kreuzungsobjekts (ring_kreuzung.ECK_ENDE)"""
    if seite == 'w':
        return -279.5 <= e['p'][1] <= -254.5
    return -281.0 <= e['p'][1] <= -252.0


def aussen(e, seite):
    """Lage der Aussenwand (Querweg t) oder None"""
    r = e[seite]
    if r is None or im_knoten(e, seite) or r['z'] - e['zr'] < 0.4 and abs(r['g'] - r['z']) < 0.6:
        return None
    return r['a'] if seite == 'w' else r['b']


def rand(e, seite):
    """aeusserster Querweg des Bauwerks auf dieser Seite: Aussenwand, sonst Fahrbahnkante der Rampe bzw. des Rings"""
    t = aussen(e, seite)
    if t is not None:
        return t
    r = e[seite]
    if r is not None:
        return r['a'] if seite == 'w' else r['b']
    return e['ra'] - 1.0 if seite == 'w' else e['rb'] + 1.0


def trennwand(roh, seite, o, platte=None):
    """Stuetzwand zwischen Ring und Rampe `seite` ('w'/'o'): Linie in der Mitte der Fuge; Ring-Seite Klinkerfelder mit
    Pfeilern alle 5 m, Kappe, Bruestung mit Gelaender zur Rampe -> Mesh (Objektursprung o). Unter dem
    Kreuzungsobjekt (`platte`: Hoehe z(x, y)) endet die Wand knapp unter dessen Flaeche, ohne Bruestung."""
    m = Mesh()
    vz = -1 if seite == 'w' else 1
    unter = lambda e: platte is not None and S_PLATTE[0] <= e['s'] <= S_PLATTE[1]
    lauf = []                                # zusammenhaengende Stuecke mit Rampe hoeher als der Ring
    for e in roh:
        t = fuge(e, seite)
        if t is None:
            lauf.append(None)
            continue
        z = e[seite]['z']
        if unter(e):
            q = welt(e, t, 0)
            zp = float(platte(q[0], q[1]))
            # unter der Decke darunter; davor/dahinter steht die Bruestung auf der Kreuzungsflaeche (Trogrand)
            z = min(z, zp - 0.1) if e['unter_decke'] else zp + 0.05
        lauf.append((e, t, z))
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
        s_ges, laenge = ea['s'], eb['s'] - ea['s']        # Teilung nach Station: Pfeiler und Fugen laufen durch
        n_ring = np.append(-vz * ea['quer'], 0)
        p = lambda e, t, z: welt(e, t, z) - o
        # Klinker bis 0,4 m unter die Kappe, darueber Beton
        m.quad('klinker', [p(ea, ri_a, za), p(eb, ri_b, zb), p(eb, ri_b, kappe_b - 0.4), p(ea, ri_a, kappe_a - 0.4)],
               [(s_ges / 2, za), ((s_ges + laenge) / 2, zb), ((s_ges + laenge) / 2, kappe_b - 0.4), (s_ges / 2, kappe_a - 0.4)], n_ring)
        m.quad('beton', [p(ea, ri_a, kappe_a - 0.4), p(eb, ri_b, kappe_b - 0.4), p(eb, ri_b, kappe_b), p(ea, ri_a, kappe_a)],
               [(s_ges / 4, 0), ((s_ges + laenge) / 4, 0), ((s_ges + laenge) / 4, 0.1), (s_ges / 4, 0.1)], n_ring)
        if not (ea['unter_decke'] or eb['unter_decke']):
            # Bruestung (niedrige Betonkante) ueber der Rampe, darauf das Stabgelaender (Fotos)
            br_a, br_b = kappe_a + 0.25, kappe_b + 0.25
            m.quad('beton', [p(ea, ri_a, kappe_a), p(eb, ri_b, kappe_b), p(eb, ri_b, br_b), p(ea, ri_a, br_a)],
                   [(s_ges / 4, 0), ((s_ges + laenge) / 4, 0), ((s_ges + laenge) / 4, 0.25), (s_ges / 4, 0.25)], n_ring)
            m.quad('beton', [p(ea, ra_a, kappe_a - 0.2), p(eb, ra_b, kappe_b - 0.2), p(eb, ra_b, br_b), p(ea, ra_a, br_a)],
                   [(s_ges / 4, 0), ((s_ges + laenge) / 4, 0), ((s_ges + laenge) / 4, 0.3), (s_ges / 4, 0.3)], -n_ring)
            m.quad('beton', [p(ea, ri_a, br_a), p(eb, ri_b, br_b), p(eb, ra_b, br_b), p(ea, ra_a, br_a)],
                   [(0, 0), (laenge / 4, 0), (laenge / 4, 0.1), (0, 0.1)], (0, 0, 1))
            stabgelaender(m, p(ea, ta, 0), p(eb, tb, 0), br_a - o[2], br_b - o[2], 1.0, s_ges)
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
        s_ges, laenge = ea['s'], eb['s'] - ea['s']        # Teilung nach Station: Pfeiler und Fugen laufen durch
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
        nah = LSW_FREI[0] <= ea['s'] <= LSW_FREI[1]
        if not nah and (seite == 'w' or ga > za + 1.0 and gb > zb + 1.0):
            m.quad(lsw, [p(ea, lsw_t[0], hoch_a), p(eb, lsw_t[1], hoch_b), p(eb, lsw_t[1], hoch_b + lsw_h), p(ea, lsw_t[0], hoch_a + lsw_h)],
                   [(s_ges / 2, 0), ((s_ges + laenge) / 2, 0), ((s_ges + laenge) / 2, 1), (s_ges / 2, 1)], n_innen, beidseitig=True)
            if int((s_ges + laenge) // 3) > int(s_ges // 3):     # Pfosten alle 3 m
                q = welt(eb, lsw_t[1], 0) - o
                kasten(m, 'gelaender', q[0] - 0.08, q[0] + 0.08, q[1] - 0.08, q[1] + 0.08, hoch_b, hoch_b + lsw_h + 0.05)
        s_ges += laenge
    return m


def decke(T, z_top, o):
    """Unterfuehrung: Betonplatte zwischen den Trogwaenden unter dem Kreuzungsobjekt; an beiden Enden endet der Trog
    halbrund (Luftbild): die Stirnseite der Decke folgt dem Halbkreis. Unterseite dunkler Beton"""
    from .ring_kreuzung import fein_dreiecke
    m = Mesh()
    dicke = 1.1
    for c in fein_dreiecke(T['decke'], 4.0):
        m.dreieck('decke', [(x - o[0], y - o[1], z_top - dicke - o[2]) for x, y in c], [(x / 4, y / 4) for x, y in c], (0, 0, -1))
        m.dreieck('beton', [(x - o[0], y - o[1], z_top - o[2]) for x, y in c], [(x / 4, y / 4) for x, y in c], (0, 0, 1))
    for name in ('nord', 'sued'):
        info = T[name]
        mitte, rr, d = info['mitte'], info['r'], info['richtung']
        quer = np.array([d[1], -d[0]])
        pts = [mitte + quer * rr * math.cos(w) + d * rr * math.sin(w) for w in np.linspace(0, math.pi, 33)]
        for a, b in zip(pts[:-1], pts[1:]):
            mid = (a + b) / 2
            nrm = (mitte - mid) / np.linalg.norm(mitte - mid)        # zur Trogoeffnung hin
            L = float(np.linalg.norm(b - a))
            m.quad('beton', [(a[0] - o[0], a[1] - o[1], z_top - dicke - 0.3 - o[2]), (b[0] - o[0], b[1] - o[1], z_top - dicke - 0.3 - o[2]),
                             (b[0] - o[0], b[1] - o[1], z_top - o[2]), (a[0] - o[0], a[1] - o[1], z_top - o[2])],
                   [(0, 0), (L / 4, 0), (L / 4, 0.35), (0, 0.35)], (nrm[0], nrm[1], 0))
    return m


def strassen_raender(q):
    """Querschnitt (Baukasten-JSON) -> (linker Rand, rechter Rand) quer zur Achse (Achse = Mitte der Fahrspuren,
    rechts positiv) wie querschnitt.rs links()"""
    x, a, b = 0.0, None, None
    for t in q['teile']:
        if t['art'] in ('fahrspur', 'busspur'):
            a = x if a is None else a
            b = x + t['breite']
        x += t['breite']
    mitte = (a + b) / 2 if a is not None else x / 2
    return -mitte, x - mitte


def bahnbruecke(st, gel, o, z_platte=None):
    """Bruecke der Schiesshausbruecke ueber die Bahn, genau unter der Strasse (Etappe 2): Platte so breit wie der
    Querschnitt, Bruestungen mit Gelaender an den Raendern, Widerlager dort, wo der Bahneinschnitt (DGM) beginnt und
    endet. Jenseits des Splines (auf dem Kreuzungsobjekt) gilt die Hoehe `z_platte(x, y)`; dort gibt es keine
    Bruestungen, das Kreuzungsobjekt fuehrt sie gerundet um die Ecken. -> (Mesh, Hoehe der Fahrbahn in der Mitte)"""
    m = Mesh()
    el = st['elemente']
    p0 = np.array([el[0]['x'], el[0]['y']])
    h = math.radians(el[0]['richtung'])
    u = np.array([math.sin(h), math.cos(h)])                # entlang der Strasse (West -> Ost)
    n = np.array([-u[1], u[0]])                              # links
    L = sum(e['laenge'] for e in el)
    kum = np.array([e['kum'] for e in el] + [L])
    zs = np.array([e['z'] for e in el] + [el[-1]['z'] + el[-1]['dh']])
    def z_bei(a):
        if a > L and z_platte is not None:
            q = p0 + u * a
            return float(z_platte(q[0], q[1]))
        return float(np.interp(a, kum, zs))
    links, rechts = strassen_raender(st['querschnitt'])
    # Einschnitt: wo das Gelaende in der Strassenmitte mehr als 2 m unter der Fahrbahn liegt
    A = np.arange(0, L + 30.0, 0.5)
    P = p0[None, :] + A[:, None] * u[None, :]
    tief = gel.h(P[:, 0], P[:, 1]) < np.array([z_bei(a) for a in A]) - 2.0
    if not tief.any():
        return m, z_bei(L / 2)
    a0, a1 = A[tief].min() - 1.5, A[tief].max() + 1.5
    sohle = float(np.nanmin(gel.h(P[tief, 0], P[tief, 1])))
    # lokaler Rahmen: a entlang, b quer (links positiv), Hoehe absolut
    W = lambda a, b, z: (p0[0] + u[0] * a + n[0] * b - o[0], p0[1] + u[1] * a + n[1] * b - o[1], z - o[2])
    N = lambda du, dn, dz: (u[0] * du + n[0] * dn, u[1] * du + n[1] * dn, dz)
    bl, br = -links + 0.35, -rechts - 0.35                   # Aussenkanten quer (links positiv)
    za, zb = z_bei(a0) - 0.05, z_bei(a1) - 0.05
    a1_voll = a1
    dicke = 1.2
    m.quad('decke', [W(a0, bl, za - dicke), W(a1, bl, zb - dicke), W(a1, br, zb - dicke), W(a0, br, za - dicke)],
           [(0, 0), ((a1 - a0) / 4, 0), ((a1 - a0) / 4, (bl - br) / 4), (0, (bl - br) / 4)], N(0, 0, -1))
    a1, zb_alt = min(a1, L), zb
    zb = z_bei(a1) - 0.05
    for b, sgn in ((bl, 1), (br, -1)) if a1 > a0 + 0.5 else ():
        bi = b - sgn * 0.3
        kante = 0.3
        m.quad('beton', [W(a0, b, za - dicke), W(a1, b, zb - dicke), W(a1, b, zb + kante), W(a0, b, za + kante)],
               [(0, 0), ((a1 - a0) / 4, 0), ((a1 - a0) / 4, 0.4), (0, 0.4)], N(0, sgn, 0))
        m.quad('beton', [W(a0, bi, za), W(a1, bi, zb), W(a1, bi, zb + kante), W(a0, bi, za + kante)],
               [(0, 0), ((a1 - a0) / 4, 0), ((a1 - a0) / 4, 0.1), (0, 0.1)], N(0, -sgn, 0))
        m.quad('beton', [W(a0, b, za + kante), W(a1, b, zb + kante), W(a1, bi, zb + kante), W(a0, bi, za + kante)],
               [(0, 0), ((a1 - a0) / 4, 0), ((a1 - a0) / 4, 0.1), (0, 0.1)], N(0, 0, 1))
        bm = (b + bi) / 2
        stabgelaender(m, W(a0, bm, 0), W(a1, bm, 0), za + kante - o[2], zb + kante - o[2], 1.0, a0)
    # Widerlager: Waende von der Sohle bis unter die Platte, zur Bahn hin Klinker
    a1, zb = a1_voll, zb_alt
    for a, sgn, z in ((a0, 1, za),) if a0 < L else ():
        m.quad('klinker', [W(a, br, sohle - 0.5), W(a, bl, sohle - 0.5), W(a, bl, z - dicke), W(a, br, z - dicke)],
               [(0, sohle), ((bl - br) / 2, sohle), ((bl - br) / 2, z - dicke), (0, z - dicke)], N(sgn, 0, 0), beidseitig=True)
    return m, z_bei((a0 + a1) / 2)


def _spuren_quer(q, arten):
    """(von, bis) quer zur Achse (rechts der Splinerichtung positiv) der Teile mit Art in `arten`, mit Richtung"""
    links, _ = strassen_raender(q)
    x, out = 0.0, []
    for t in q['teile']:
        if t['art'] in arten:
            out.append((links + x, links + x + t['breite'], t['richtung']))
        x += t['breite']
    return out


def tunnel_gelaende(t):
    """Gelaendepunkte im Bahntunnel unter der Kreuzung auf Gleishoehe (das DGM zeigt dort Reste der Rampe)"""
    out = []
    if not t or not t['y']:
        return out
    (x0, x1), (y0, y1) = t['x'], t['y']
    for kx in range(int(math.floor(x0 / 300)), int(math.floor(x1 / 300)) + 1):
        for ky in range(int(math.floor((y0 - 5) / 300)), int(math.floor((y1 + 5) / 300)) + 1):
            for zeile in range(61):
                for spalte in range(61):
                    x, y = kx * 300 + spalte * 5.0, ky * 300 + zeile * 5.0
                    if x0 + 0.5 <= x <= x1 - 0.5 and y0 - 5 <= y <= y1 + 5:
                        out.append([kx, ky, spalte, zeile, t['sohle'] - 0.1])
    return out


def platte_gelaende(platte, zf):
    """Gelaendepunkte unter dem Kreuzungsobjekt (mindestens 1,5 m innerhalb), 0,4 m unter seiner Flaeche"""
    out = []
    innen = platte.buffer(-1.5)
    x0, y0, x1, y1 = platte.bounds
    from shapely.geometry import Point as _P
    for kx in range(int(math.floor(x0 / 300)), int(math.floor(x1 / 300)) + 1):
        for ky in range(int(math.floor(y0 / 300)), int(math.floor(y1 / 300)) + 1):
            for zeile in range(61):
                for spalte in range(61):
                    x, y = kx * 300 + spalte * 5.0, ky * 300 + zeile * 5.0
                    if x0 <= x <= x1 and y0 <= y <= y1 and innen.contains(_P(x, y)):
                        out.append([kx, ky, spalte, zeile, round(float(zf(x, y)) - 0.4, 3)])
    return out


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
        unter_platte = S_PLATTE[0] <= ea['s'] and eb['s'] <= S_PLATTE[1]
        for k_, ((a0, a1, za), (b0, b1, zb)) in enumerate(zip(pa, pb)):
            if a1 - a0 < 0.05 and b1 - b0 < 0.05:
                continue
            if unter_platte and k_ != 1:
                continue                         # Rampen: dort liegt das Kreuzungsobjekt
            q = [p(ea, a0, za), p(eb, b0, zb), p(eb, b1, zb), p(ea, a1, za)]
            m.quad('asphalt', q, [((c[0] + o[0]) / 5, (c[1] + o[1]) / 5) for c in q], (0, 0, 1))
        # Stufen zwischen den Flaechen (wo keine Trennwand steht, sind sie niedrig)
        for k in (() if unter_platte else (0, 1)):
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


HINTER_MAUER = 5.5        # abgesenktes Gelaende hinter Aussenmauern (eine Rasterzelle + Rand)


def deckel(roh, karte, o, breite=12.0):
    """Deckel neben dem Bauwerk: vom Rand (hinter der Aussenwand auf Gelaendehoehe) 12 m nach aussen bis auf das
    unveraenderte Gelaende - verdecken die Schraege des abgesenkten 5-m-Gelaenderasters (wie bauwerke.rs)"""
    m = Mesh()
    teile = 6
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
                    # hinter Aussenmauern eine Rasterzelle mehr: die Schraege zum unveraenderten Gelaende liegt dann
                    # hinter der Mauer unter dem Deckel statt davor auf der Fahrbahn
                    if aussen(e, 'w') is not None:
                        lo -= HINTER_MAUER
                    if aussen(e, 'o') is not None:
                        hi += HINTER_MAUER
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
    # Stabgelaender (1 m x 1 m je Textur): Holm oben, Gurt unten, senkrechte Staebe alle 12,5 cm, sonst durchsichtig
    st = np.zeros((128, 128, 4), np.uint8)
    farbe = (156, 88, 112, 255)
    st[0:7, :] = farbe
    st[118:124, :] = farbe
    st[:, [x for x in range(128) if x % 16 in (7, 8)]] = farbe
    Image.fromarray(st, 'RGBA').save(os.path.join(ziel, TEX['stab']))
    # Bodendecker im Pflanzstreifen
    p = (rng.normal(0, 18, (128, 128, 3)) + (70, 105, 55)).clip(0, 255).astype(np.uint8)
    Image.fromarray(p).save(os.path.join(ziel, TEX['pflanzen']))
    shutil.copy(os.path.join(marcel, 'str_asphdrk.bmp'), os.path.join(ziel, TEX['asphalt']))
    # Inseln an den Trogenden: heller Beton
    ib = np.asarray(Image.open(os.path.join(marcel, 'betonwand1.bmp')).convert('RGB'), float) * 0.55 + 95
    Image.fromarray(ib.clip(0, 255).astype(np.uint8)).save(os.path.join(ziel, TEX['insel_beton']))
    # Gehweg-Asphalt: etwas heller als die Fahrbahn
    g = np.asarray(Image.open(os.path.join(marcel, 'str_asphdrk.bmp')).convert('RGB'), float) * 1.18 + 8
    Image.fromarray(g.clip(0, 255).astype(np.uint8)).save(os.path.join(ziel, TEX['gehweg_asphalt']))
    for t in ('str_gehweg02.bmp', 'str_side1.bmp', 'str_betonplatten.bmp'):
        shutil.copy(os.path.join(marcel, t), os.path.join(ziel, t))
    Image.new('RGB', (8, 8), (235, 235, 230)).save(os.path.join(ziel, TEX['weiss']))
    # Radfurt: dieselbe rote Asphalttextur wie die Radstreifen der Baukasten-Querschnitte (sonst einfarbig)
    rot = os.path.join(omsi, 'Splines', 'Aschaffenburg', 'texture', 'AB_asphalt_rot.bmp')
    if os.path.isfile(rot):
        shutil.copy(rot, os.path.join(ziel, TEX['radfurt']))
    else:
        Image.new('RGB', (8, 8), (176, 62, 52)).save(os.path.join(ziel, TEX['radfurt']))
    # Sperrflaeche: weisse Schraegstreifen (45 Grad), sonst durchsichtig; 2 m je Textur
    sp = np.zeros((128, 128, 4), np.uint8)
    yy, xx = np.mgrid[0:128, 0:128]
    sp[((xx + yy) % 64) < 22] = (235, 235, 230, 255)
    Image.fromarray(sp, 'RGBA').save(os.path.join(ziel, TEX['sperr']))
    # Deckel wie das Gelaende (openOMSI: Bodentextur mal Detailtextur, Detail 60 x je Kachel = alle 5 m):
    # Detailtextur eingefaerbt mit der mittleren Farbe der Bodentextur (wie bauwerke.rs deckel_textur)
    basis = np.asarray(Image.open(os.path.join(omsi, 'Texture', 'gras.bmp')).convert('RGB'), float)
    detail = np.asarray(Image.open(os.path.join(omsi, 'Texture', 'gras_det.bmp')).convert('RGB'), float)
    mittel = basis.reshape(-1, 3).mean(axis=0)
    Image.fromarray((detail * mittel / 255.0).clip(0, 255).astype(np.uint8)).save(os.path.join(ziel, TEX['boden']))


# ------------------------------------------------------------------ Ausgabe
def sco_text(name, modell, mats, rendertype=None):
    """rendertype 'onsurface': Markierung auf einer Kreuzungsplatte (wird nach der Platte gezeichnet, wie in OMSI)"""
    L = ['Erzeugt mit omsigen (sonderbau_ring). OSM (ODbL), DGM1/DOP40 Bayern (CC BY 4.0).', '', '[friendlyname]', name, '',
         '[groups]', '1', 'Aschaffenburg', '', '[fixed]', '', '[absheight]', '']
    if rendertype:
        L += ['[rendertype]', rendertype, '']
    L += ['[mesh]', modell, '']
    for key in mats:
        if TEX[key].endswith('.tga'):
            L += ['[matl]', TEX[key], '0', '[matl_alpha]', '2', '']
    return '\r\n'.join(L) + '\r\n'


def schreiben(ordner, rel_ordner, name, mesh, o, liste, rendertype=None):
    if mesh.leer():
        return
    mats = [(k, TEX[k]) for k in mesh.mats]
    with open(os.path.join(ordner, 'model', name + '.x'), 'w', newline='') as f:
        f.write(x_file(mesh.V, mesh.F, mats))
    with open(os.path.join(ordner, name + '.sco'), 'w', encoding='cp1252', newline='') as f:
        f.write(sco_text(f'Ring/Ludwigsallee {name}', name + '.x', mesh.mats, rendertype))
    liste.append(dict(name=name, rel=rel_ordner + '\\' + name + '.sco', x=float(o[0]), y=float(o[1]), z=float(o[2]), dreiecke=len(mesh.F)))


def bauen(omsi, karte, osm_pfad, entwurf_pfad, nn0, ost0, nord0, cache):
    ordner = os.path.join(omsi, 'Sceneryobjects', 'Aschaffenburg', karte, 'Sonderbau_Ring')
    rel = f'Sceneryobjects\\Aschaffenburg\\{karte}\\Sonderbau_Ring'
    os.makedirs(os.path.join(ordner, 'model'), exist_ok=True)
    texturen(omsi, os.path.join(ordner, 'texture'))
    ways = lade_osm(osm_pfad, ost0, nord0)
    dgm = Dgm1(cache)
    with open(entwurf_pfad, encoding='utf-8') as f:
        entwurf = json.load(f)
    roh, _ = aus_entwurf(entwurf, dgm, ost0, nord0, nn0)
    karte_gel = Kartengelaende(os.path.join(omsi, 'maps', karte))
    liste = []
    # Etappe 2: Strassen auf dem Entwurf
    from .ring_strassen import planen, text
    from .sonderbau_entwurf import Gelaende
    gel = Gelaende.laden(dgm, ost0, nord0, nn0, 1000, 1170, -460, -100)
    strassen, arme = planen(entwurf, gel, ways)
    print(text(strassen))
    # Etappe 3: Kreuzung als eigenes Objekt (ring_kreuzung)
    from . import ring_kreuzung as rk, ring_strassen as rs
    from .sonderbau_entwurf import achse_aus_json, versatz_aus_json
    from shapely.ops import unary_union
    achse = achse_aus_json(entwurf['achse'])
    K = {k_: versatz_aus_json(v) for k_, v in entwurf['kanten'].items()}
    kz = rk.bauen(achse, K, strassen, arme, gel, rs.VERBINDUNGEN, rs.PHASEN, rs.PFEILE,
                  [n_ for n_ in rs.RAMPEN])
    O = kz['O']
    name = 'Kreuzung_Ludwigsallee'
    alpha = [TEX[k_] for k_ in kz['mesh'].mats if TEX[k_].endswith('.tga')]
    with open(os.path.join(ordner, 'model', name + '.x'), 'w', newline='') as f:
        f.write(x_file(kz['mesh'].V, kz['mesh'].F, [(k_, TEX[k_]) for k_ in kz['mesh'].mats]))
    with open(os.path.join(ordner, name + '.sco'), 'w', encoding='cp1252', newline='') as f:
        f.write(rk.sco_text('Ring/Ludwigsallee Kreuzung', name + '.x', kz, alpha))
    F = kz['flaechen']
    print(f"Kreuzung: {len(kz['pfade'])} Pfadstuecke, {len(kz['ketten'])} Abbiegespuren, {len(kz['regeln'])} Regeln, "
          f"{len(kz['mesh'].F)} Dreiecke, Signale nicht auf Gehweg/Insel: {kz['lose_signale']}")
    kreuzung = dict(name=name, rel=rel + '\\' + name + '.sco', ursprung=[float(O[0]), float(O[1])], rules=kz['regeln'], no_cars=[],
                    signale=kz['signale'], phasen=kz['plan']['phase'], umlauf=kz['plan']['umlauf'],
                    abbiegen=[dict(von=k_['von'], nach=k_['nach'], mv=k_['mv']) for k_ in kz['ketten']])
    # Decke unter dem Kreuzungsobjekt, Bahnbruecke bis an seine Eckbruestungen
    T = kz['trog']
    zs = [float(kz['zf'](x, y)) for g_ in rk._polys(T['decke']) for x, y in np.array(g_.exterior.coords)]
    o = np.array([T['nord']['mitte'][0], T['nord']['mitte'][1], 0.0])
    schreiben(ordner, rel, 'Unterfuehrung', decke(T, min(zs) - 0.12, o), o, liste)
    st_w = next(st for st in strassen if st['name'] == 'Schiesshausbruecke')
    o = np.array([st_w['elemente'][0]['x'], st_w['elemente'][0]['y'], 0.0])
    bb, _ = bahnbruecke(st_w, gel, o, kz['zf'])
    schreiben(ordner, rel, 'Bahnbruecke', bb, o, liste)
    # Waende in Stuecken von STUECK m (Ursprung je Stueck: erster Punkt auf der Ring-Achse, Hoehe 0)
    n = int(STUECK / SCHRITT)
    for k in range(0, len(roh) - 1, n):
        teil = roh[k:k + n + 1]
        o = np.array([teil[0]['p'][0], teil[0]['p'][1], 0.0])
        for seite in ('w', 'o'):
            schreiben(ordner, rel, f'Trennwand_{seite}_{k // n:02d}', trennwand(teil, seite, o, kz['zf']), o, liste)
            schreiben(ordner, rel, f'Aussenwand_{seite}_{k // n:02d}', aussenwand(teil, seite, o), o, liste)
        schreiben(ordner, rel, f'Sohle_{k // n:02d}', flaechen(teil, o), o, liste)
        schreiben(ordner, rel, f'Deckel_{k // n:02d}', deckel(teil, karte_gel, o), o, liste)
    messung = [dict(s=e['s'], x=float(e['p'][0]), y=float(e['p'][1]), ring=[float(e['ra']), float(e['rb']), float(e['zr'])],
                    west=None if e['w'] is None else [e['w'][k] for k in ('a', 'b', 'z', 'g')],
                    ost=None if e['o'] is None else [e['o'][k] for k in ('a', 'b', 'z', 'g')], decke=bool(e['unter_decke'])) for e in roh]
    platte = unary_union([F['fahrbahn'], F['gehweg'], F['inseln']])
    out = dict(karte=karte, nn0=nn0, objekte=liste, strassen=strassen, kreuzungen=[kreuzung],
               gelaende=gelaende(roh) + platte_gelaende(platte, kz['zf']) + tunnel_gelaende(kz['tunnel']), messung=messung, quellen=['OpenStreetMap (ODbL)', 'DGM1/DOP40 Bayerische Vermessungsverwaltung (CC BY 4.0)'])
    with open(os.path.join(ordner, 'sonderbau_ring.json'), 'w', encoding='utf-8') as f:
        json.dump(out, f, indent=1)
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--omsi', required=True)
    ap.add_argument('--karte', required=True)
    ap.add_argument('--osm', required=True)
    ap.add_argument('--entwurf', default=os.path.join('sonderbau', 'ring', 'entwurf', 'entwurf.json'))
    ap.add_argument('--cache', default=os.path.join(os.environ.get('LOCALAPPDATA', '.'), 'omsi-editor', 'geodaten'))
    a = ap.parse_args()
    geo = {}
    with open(os.path.join(a.omsi, 'maps', a.karte, 'omsi-editor-geo.cfg'), encoding='utf-8') as f:
        for z in f:
            if '=' in z and not z.startswith('#'):
                k, v = z.split('=', 1)
                geo[k.strip()] = v.strip()
    out = bauen(a.omsi, a.karte, a.osm, a.entwurf, float(geo['nn0']), float(geo['ost0']), float(geo['nord0']), a.cache)
    for o in out['objekte']:
        print(f"{o['name']:24s} {o['dreiecke']:6d} Dreiecke  bei {o['x']:.1f} {o['y']:.1f}")
    print(f"{len(out['objekte'])} Objekte, {len(out['messung'])} Querschnitte")


if __name__ == '__main__':
    sys.exit(main())
