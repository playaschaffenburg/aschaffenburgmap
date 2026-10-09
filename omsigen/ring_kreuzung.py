"""Sonderbau Ring/Ludwigsallee, Etappe 3: die Kreuzung Ludwigsallee als eigenes Objekt (Einzelstueck).

Gebaut nach dem Luftbild (DOP40, vermessen 10/2026, Lesart in sonderbau/ring/PLAN.md) statt mit dem allgemeinen
Kreuzungsgenerator:
- Flaechen: Fahrbahn (Arme, Rampenmuendungen, Mitte; ausgerundete Ecken), Gehwege an den Ecken, Inseln an den
  Trogenden (sie umfassen die halbrunden Trogoeffnungen) und die Verkehrsinsel Ost; Bordsteine.
- Bruestungen mit Gelaender: an den Ecken ueber dem Bahneinschnitt (die gerundeten Ueberhaenge) und um die
  Trogoeffnungen.
- Markierungen im Objekt selbst (als eigenes Objekt verdeckte sie die Platte): Haltlinien, Spurlinien und Sperrflaechen
  der Zufahrten, Pfeile nach OSM turn:lanes, BUS, Radaufstellflaeche, rote Radfurten mit gestrichelten Raendern,
  Fussgaengerfurten, Leitlinien der Linksabbieger, Sperrflaeche vor der Verkehrsinsel.
- Hoehe: keine Ebene, sondern eine glatte Flaeche, die an jedem Arm genau Hoehe und Steigung der Strasse trifft
  (gewichtete Mischung der verlaengerten Arme); die Pfade folgen ihr (Hoehenaenderung je Pfadstueck).
- Pfade: je ankommender Spur ein Stueck bis zur Haltlinie, ab dort die Abbiegespuren (an die Ampel gebunden),
  Vorfahrt-Regeln wie omsigen vorfahrt; Ampeln mit 3 Phasen; Masten auf Gehwegen und Inseln an den Haltlinien.

Das Objekt hat [absheight] und liegt bei Hoehe 0 (alle Hoehen absolut), ohne Drehung."""
import math
import numpy as np
import shapely
from shapely.geometry import Polygon, Point, LineString, box
from shapely.ops import unary_union, substring
from . import ampel, vorfahrt
from .geom import connect, end_of, dvec, rvec, norm180
from .kreuzung import ASPH_H, WALK_H, BLINKER

# ------------------------------------------------------------------ Lage nach Luftbild (Kartenkoordinaten)
ECKRADIUS = 6.0
TROG_SEITE = (-248.9, -284.0)                 # wo die Trogwaende an die Decke stossen (Nord, Sued)
INSEL_N = [(1079.4, -248.4), (1090.0, -248.4), (1087.3, -257.9), (1083.2, -257.9)]
INSEL_S = [(1083.2, -274.55), (1087.25, -274.55), (1090.4, -284.6), (1079.9, -284.6)]
INSEL_O = ((1094.0, -268.3), (1105.3, -266.0))     # Verkehrsinsel Ost (Rechteck, Westende rund)
INSEL_W = ((1059.5, -266.3), (1064.4, -265.5))     # Fahrbahnteiler West (Ampelmast) am Ende des Sperrstreifens
SPERR_O = [(1088.6, -267.15), (1094.0, -266.0), (1094.0, -268.3)]
ROT = [(-259.7, -257.45), (-274.1, -272.1)]        # Radfurten (y) Nord, Sued
ROT_X = (1065.5, 1099.4)
FURTEN = [((1071.6, -254.55), (1096.6, -254.55)), ((1070.4, -278.2), (1097.8, -278.2)),
          ((1099.4, -255.8), (1099.4, -273.6)), ((1104.8, -255.8), (1104.8, -273.6))]
RADBOX = ((1106.2, -258.8), (1109.8, -256.0))      # Aufstellflaeche fuer Radfahrer auf der Busspur Ost
HALT = {0: ('x', 1064.5), 1: ('y', -253.85), 3: ('x', 1110.2), 4: ('y', -280.4)}   # Haltlinien der Zufahrten
HALT_RAD = (1105.5, -258.8, -266.0)                # Haltlinie fuer Radfahrer vor der Aufstellflaeche Ost (x, y von/bis)
RAND = {2: ('y', -254.55), 5: ('y', -278.2)}       # Ende der Markierungen der Ausfahrten (Furt)
# Ampelmasten: Arm -> (rechts der Zufahrt, links der Zufahrt)
MASTEN = {0: ((1063.6, -274.6), (1063.6, -265.9)), 1: ((1070.9, -253.4), (1084.8, -256.9)),
          3: ((1110.6, -254.7), (1104.6, -267.15)), 4: ((1098.0, -281.0), (1085.2, -275.3))}
GEHWEG_ZONEN = [box(1049.9, -261.6, 1072.6, -244.0), box(1096.0, -256.6, 1120.1, -244.0),
                box(1049.9, -290.0, 1071.6, -272.9), box(1096.2, -294.0, 1120.1, -273.3)]
BRUESTUNG_ZONEN = [0, 2]                           # Ecken ueber dem Bahneinschnitt (NW, SW)
GEHWEG_BREITE = 2.6


def raender(q, nur_fahrbahn=False):
    """Querschnitt -> Liste (Teil, von, bis) quer zur Achse (rechts der Splinerichtung positiv; Achse = Mitte der
    Fahrspuren, wie querschnitt.rs links())"""
    x, a, b = 0.0, None, None
    for t in q['teile']:
        if t['art'] in ('fahrspur', 'busspur'):
            a = x if a is None else a
            b = x + t['breite']
        x += t['breite']
    m = (a + b) / 2
    out, x = [], 0.0
    for t in q['teile']:
        if not (nur_fahrbahn and t['art'] == 'gehweg'):
            out.append((t, x - m, x - m + t['breite']))
        x += t['breite']
    return out


# ------------------------------------------------------------------ Arme
class Arm:
    def __init__(self, i, a):
        self.i, self.name, self.rolle = i, a['strasse'], a['rolle']
        self.p = np.array(a['pos'], float)
        self.h = a['h'] % 360                   # von der Kreuzung weg
        self.away, self.z, self.g, self.q = a['away'], a['z'], a['g'], a['querschnitt']
        self.weg = np.array(dvec(self.h))
        self.spline_h = self.h if self.away else (self.h + 180) % 360
        self.rechts = np.array(rvec(self.spline_h))        # rechts der Splinerichtung
        teile = raender(self.q, nur_fahrbahn=True)
        self.links_rand, self.rechts_rand = teile[0][1], teile[-1][2]

    def punkt(self, d, quer):
        """d Meter von der Armende-Linie in die Kreuzung hinein, quer rechts der Splinerichtung"""
        return self.p - self.weg * d + self.rechts * quer

    def bis(self, ref):
        """Abstand vom Armende bis zur Linie x = .. bzw. y = .. (in die Kreuzung hinein)"""
        achse, wert = ref
        k = 0 if achse == 'x' else 1
        return (self.p[k] - wert) / self.weg[k]

    def spuren(self):
        """(rein, raus): je Liste (quer, Fahrtrichtung, Teil), von rechts in Fahrtrichtung gezaehlt"""
        rein, raus = [], []
        for t, a, b in raender(self.q):
            if t['art'] not in ('fahrspur', 'busspur'):
                continue
            mit = t['richtung'] == 'vor'
            kommt = mit != self.away            # faehrt zur Kreuzung
            (rein if kommt else raus).append(((a + b) / 2, t))
        # rechts in Fahrtrichtung: zur Kreuzung = gegen 'weg'; Querachse 'rechts' gehoert zur Splinerichtung
        def rang(liste, zur_kreuzung):
            fahrt = (self.h + 180) % 360 if zur_kreuzung else self.h
            r = np.array(rvec(fahrt))
            return sorted(liste, key=lambda x: -float(np.dot(self.rechts * x[0], r)))
        return rang(rein, True), rang(raus, False)


def hoehenflaeche(arme):
    """z(x, y): glatte Mischung der in die Kreuzung verlaengerten Arme (Hoehe und Steigung am Armende), am Armende
    genau dessen Hoehe"""
    daten = []
    for a in arme:
        p0, p1 = a.p + a.rechts * a.links_rand, a.p + a.rechts * a.rechts_rand
        daten.append((a.p, a.weg, a.z, a.g, p0, p1))

    def z(x, y):
        P = np.stack([np.atleast_1d(np.asarray(x, float)), np.atleast_1d(np.asarray(y, float))], axis=1)
        num = np.zeros(len(P))
        den = np.zeros(len(P))
        for p, weg, z0, g, p0, p1 in daten:
            d = p1 - p0
            t = np.clip(((P - p0) @ d) / (d @ d), 0, 1)
            dist2 = ((P - (p0 + t[:, None] * d)) ** 2).sum(axis=1)
            w = 1.0 / (dist2 + 1e-6)
            num += w * (z0 + g * ((P - p) @ weg))
            den += w
        v = num / den
        return v[0] if np.ndim(x) == 0 else v
    return z


# ------------------------------------------------------------------ Flaechen
def streifen_achse(achse, s0, s1, t0, t1):
    """Flaeche zwischen den Parallelen t0, t1 zur Ring-Achse von s0 bis s1"""
    S = np.linspace(s0, s1, max(2, int(abs(s1 - s0)) + 1))
    a = achse.versetzt(S, np.full(len(S), t0))
    b = achse.versetzt(S, np.full(len(S), t1))
    return Polygon(np.vstack([a, b[::-1]]))


def trog(achse, K):
    """Trogoeffnungen (Nord, Sued) mit halbrundem Ende an der Decke und die Decke dazwischen -> dict"""
    s_n = float(achse.projektion([[1084.0, TROG_SEITE[0]]])[0, 0])
    s_s = float(achse.projektion([[1083.0, TROG_SEITE[1]]])[0, 0])
    out = dict(s=(s_n, s_s))
    for name, s, sgn, s_weit in (('nord', s_n, 1, s_n - 60), ('sued', s_s, -1, s_s + 60)):
        tw, to = float(K['ring_w'].t([s])[0]), float(K['ring_o'].t([s])[0])
        r, tc = (tw - to) / 2, (tw + to) / 2
        mitte = achse.versetzt([s], [tc])[0]
        scheibe = Point(mitte).buffer(r, 64)
        x, y, th = achse.punkt(s)
        d = np.array([math.cos(th), math.sin(th)]) * sgn          # in die Decke hinein
        halb = Polygon([mitte + achse_quer(th) * (r + 1), mitte + achse_quer(th) * (r + 1) + d * (r + 1),
                        mitte - achse_quer(th) * (r + 1) + d * (r + 1), mitte - achse_quer(th) * (r + 1)])
        # der Streifen reicht etwas in den Halbkreis hinein: keine Spalte auf dem Durchmesser
        offen = unary_union([streifen_achse(achse, s_weit, s + sgn * 0.3, to, tw), scheibe.intersection(halb)]).buffer(0.01).buffer(-0.01)
        out[name] = dict(mitte=mitte, r=r, offen=offen, richtung=d)
    out['decke'] = streifen_achse(achse, s_n, s_s, float(K['ring_o'].t([s_n])[0]), float(K['ring_w'].t([s_n])[0])).difference(
        unary_union([out['nord']['offen'], out['sued']['offen']]))
    return out


def achse_quer(th):
    return np.array([math.sin(th), -math.cos(th)])


def flaechen(achse, arme, T, rampen):
    """-> dict(fahrbahn, gehweg, inseln, kern) als shapely-Geometrien"""
    teile = []
    for a in arme:
        if a.name in rampen:
            continue
        # Ludwigsallee-Arme: Fahrbahn vom Armende bis an die Rampen (dort uebernimmt die Mitte)
        tiefe = 22.0
        teile.append(Polygon([a.punkt(0, a.links_rand), a.punkt(0, a.rechts_rand), a.punkt(tiefe, a.rechts_rand), a.punkt(tiefe, a.links_rand)]))
    for name, (s0, s1, t0, t1) in rampen.items():
        teile.append(streifen_achse(achse, s0, s1, t0, t1))
    kern = Polygon([(1071.0, -273.6), (1096.6, -273.6), (1096.3, -253.6), (1072.1, -253.6)])
    teile.append(kern)
    roh = unary_union(teile)
    fahrbahn = roh.buffer(ECKRADIUS, 32).buffer(-ECKRADIUS, 32)          # einspringende Ecken ausrunden
    fahrbahn = fahrbahn.intersection(roh.envelope.buffer(0))             # nicht ueber die Armenden hinaus
    insel_o = unary_union([box(INSEL_O[0][0], INSEL_O[0][1], INSEL_O[1][0], INSEL_O[1][1]),
                           Point(INSEL_O[0][0], (INSEL_O[0][1] + INSEL_O[1][1]) / 2).buffer((INSEL_O[1][1] - INSEL_O[0][1]) / 2, 24)])
    offen = unary_union([T['nord']['offen'], T['sued']['offen']])
    insel_w = box(INSEL_W[0][0], INSEL_W[0][1], INSEL_W[1][0], INSEL_W[1][1])
    inseln = unary_union([Polygon(INSEL_N), Polygon(INSEL_S), insel_o, insel_w]).difference(offen)
    fahrbahn = fahrbahn.difference(offen).difference(inseln)
    gehweg = fahrbahn.buffer(GEHWEG_BREITE, 32).difference(fahrbahn).difference(offen).difference(inseln)
    gehweg = unary_union([gehweg.intersection(z) for z in GEHWEG_ZONEN])
    return dict(fahrbahn=fahrbahn, gehweg=gehweg, inseln=inseln, offen=offen,
                bruestung=[gehweg.intersection(GEHWEG_ZONEN[i]) for i in BRUESTUNG_ZONEN])


def _polys(g):
    if g.is_empty:
        return []
    if g.geom_type == 'Polygon':
        return [g]
    return [p for p in getattr(g, 'geoms', []) if p.geom_type == 'Polygon' and p.area > 1e-4]


def fein_dreiecke(geom, zelle=2.5):
    """Dreiecke (x, y) der Flaeche, in Zellen geteilt (damit die gewoelbte Hoehe ankommt)"""
    out = []
    x0, y0, x1, y1 = geom.bounds
    for gx in np.arange(math.floor(x0 / zelle) * zelle, x1, zelle):
        for gy in np.arange(math.floor(y0 / zelle) * zelle, y1, zelle):
            stueck = geom.intersection(box(gx, gy, gx + zelle, gy + zelle))
            for p in _polys(stueck):
                for t in shapely.constrained_delaunay_triangles(p).geoms:
                    c = list(t.exterior.coords)[:3]
                    out.append(c)
    return out


# ------------------------------------------------------------------ Markierungen
def strich(p, q, breite=0.12):
    return LineString([p, q]).buffer(breite / 2, cap_style='flat')


def gestrichelt(linie, breite=0.12, an=3.0, aus=6.0, start=0.0):
    out = []
    L = linie.length
    d = start
    while d < L:
        e = min(L, d + an)
        if e - d > 0.05:
            out.append(substring(linie, d, e).buffer(breite / 2, cap_style='flat'))
        d += an + aus
    return unary_union(out) if out else Polygon()


def pfeil(arten, p, vor, rechts):
    """Richtungspfeil (Laenge 5 m) an p, Spitze in Fahrtrichtung `vor`; arten: links/gerade/rechts"""
    def W(a, b):
        return tuple(p + vor * b + rechts * a)
    teile = [LineString([W(0, 0), W(0, 2.6)]).buffer(0.09, cap_style='flat')]
    if 'gerade' in arten:
        teile.append(LineString([W(0, 2.5), W(0, 3.9)]).buffer(0.09, cap_style='flat'))
        teile.append(Polygon([W(-0.35, 3.8), W(0.35, 3.8), W(0, 5.0)]))
    for art, sgn in (('links', -1), ('rechts', 1)):
        if art in arten:
            teile.append(LineString([W(0, 2.4), W(sgn * 0.7, 3.3)]).buffer(0.09, cap_style='flat'))
            teile.append(Polygon([W(sgn * 0.45, 3.35), W(sgn * 0.95, 2.95), W(sgn * 1.15, 3.85)]))
    return unary_union(teile)


def bus_schrift(p, vor, rechts):
    """'BUS' (je Buchstabe 0,8 m breit, 2,4 m lang), zu lesen in Fahrtrichtung `vor`"""
    def W(a, b):
        return tuple(p + vor * b + rechts * a)
    s = []
    def zug(pkte, x0):
        s.append(LineString([W(x0 + a, b) for a, b in pkte]).buffer(0.1, cap_style='flat', join_style='mitre'))
    # Buchstaben nebeneinander quer zur Fahrtrichtung (von links nach rechts), aufrecht = in Fahrtrichtung
    zug([(0, 0), (0, 2.4), (0.55, 2.4), (0.75, 2.1), (0.55, 1.2), (0, 1.2), (0.6, 1.2), (0.8, 0.9), (0.8, 0.3), (0.6, 0), (0, 0)], -1.5)
    zug([(0, 2.4), (0, 0.3), (0.2, 0), (0.6, 0), (0.8, 0.3), (0.8, 2.4)], -0.4)
    zug([(0.8, 2.2), (0.6, 2.4), (0.2, 2.4), (0, 2.1), (0, 1.5), (0.8, 0.9), (0.8, 0.3), (0.6, 0), (0.2, 0), (0, 0.2)], 0.7)
    return unary_union(s)


# ------------------------------------------------------------------ Pfade
def stuecke(el, n_max=5.0):
    """Element (x, y, h, L, R) in Stuecke <= n_max m teilen"""
    x, y, h, L, R = el[:5]
    n = max(1, math.ceil(L / n_max))
    out = []
    for k in range(n):
        if k == 0:
            px, py, ph = x, y, h
        else:
            (px, py), ph = end_of([x, y, h, L * k / n, R])
        out.append([px, py, ph, L / n, R])
    return out


def art(h_rein, h_raus):
    dlt = norm180(h_raus - h_rein)
    if abs(dlt) > 150:
        return 'wenden'
    if abs(dlt) < 35:
        return 'gerade'
    return 'rechts' if dlt > 0 else 'links'


# ------------------------------------------------------------------ Bauen
def bauen(achse, K, strassen, arme_roh, gel, verbindungen, phasen, pfeile, rampen_namen):
    """-> dict(mesh, pfade, regeln, ampel, signale, flaechen, z, ursprung, ...)"""
    from .sonderbau_ring import Mesh, kasten
    arme = [Arm(i, a) for i, a in enumerate(arme_roh)]
    zf = hoehenflaeche(arme)
    T = trog(achse, K)
    # Rampenmuendungen: Achs-Station von Armende bis an die Mitte, quer zwischen den Fahrbahnkanten
    rampen = {}
    for a in arme:
        if a.name not in rampen_namen:
            continue
        s_ende, t_mitte = achse.projektion([a.p])[0]
        st = next(s for s in strassen if s['name'] == a.name)
        teile = raender(st['querschnitt'])
        # Splinerichtung Nord -> Sued: rechts der Splinerichtung = West = t positiv
        t0, t1 = st['versatz'] + teile[0][1], st['versatz'] + teile[-1][2]
        # bis an die Trennwand (der Trennstreifen zwischen Wand und Spur gehoert zur Flaeche)
        if t_mitte > 0:
            t0 = min(t0, float(K['ring_w'].t([s_ende])[0]) + 0.25)
        else:
            t1 = max(t1, float(K['ring_o'].t([s_ende])[0]) - 0.25)
        s_innen = 131.0
        rampen[a.name] = (s_ende, s_innen, t0, t1)
    F = flaechen(achse, arme, T, rampen)
    xs = [a.p[0] for a in arme]
    ys = [a.p[1] for a in arme]
    O = np.array([(min(xs) + max(xs)) / 2, (min(ys) + max(ys)) / 2])
    m = Mesh()

    def flaeche(key, geom, dz, uv_m):
        for c in fein_dreiecke(geom):
            P = [(x - O[0], y - O[1], float(zf(x, y)) + dz) for x, y in c]
            m.dreieck(key, P, [(x / uv_m, y / uv_m) for x, y in c], (0, 0, 1))

    flaeche('asphalt', F['fahrbahn'], ASPH_H, 4.0)
    flaeche('gehweg', F['gehweg'], WALK_H, 2.0)
    flaeche('gehweg', F['inseln'], WALK_H, 2.0)
    # Bordsteine: wo Gehweg/Insel an die Fahrbahn stoesst
    fb = F['fahrbahn'].buffer(0.05)
    for erhoeht in _polys(unary_union([F['gehweg'], F['inseln']])):
        for ring in [erhoeht.exterior] + list(erhoeht.interiors):
            c = list(ring.coords)
            for p, q in zip(c[:-1], c[1:]):
                L = math.dist(p, q)
                if L < 1e-3:
                    continue
                mitte = ((p[0] + q[0]) / 2, (p[1] + q[1]) / 2)
                if not fb.contains(Point(mitte)):
                    continue
                n = np.array([q[1] - p[1], p[0] - q[0]]) / L
                if not F['fahrbahn'].buffer(0.02).contains(Point(mitte[0] + n[0] * 0.1, mitte[1] + n[1] * 0.1)):
                    n = -n
                zp, zq = float(zf(*p)), float(zf(*q))
                P = [(p[0] - O[0], p[1] - O[1], zp + ASPH_H), (q[0] - O[0], q[1] - O[1], zq + ASPH_H),
                     (q[0] - O[0], q[1] - O[1], zq + WALK_H), (p[0] - O[0], p[1] - O[1], zp + WALK_H)]
                m.quad('bord', P, [(0, 0), (L / 2, 0), (L / 2, 0.08), (0, 0.08)], (n[0], n[1], 0))
    # Bruestungen: aussen an den Eck-Gehwegen ueber dem Bahneinschnitt und um die Trogoeffnungen
    linien = []
    for g in F['bruestung']:
        for p in _polys(g):
            c = list(p.exterior.coords)
            for a_, b_ in zip(c[:-1], c[1:]):
                mitte = Point((a_[0] + b_[0]) / 2, (a_[1] + b_[1]) / 2)
                if F['fahrbahn'].buffer(0.3).contains(mitte):
                    continue
                if any(z.exterior.distance(mitte) < 0.05 for z in GEHWEG_ZONEN):
                    continue                   # Schnitt am Rand der Zone (Armende, Rampe)
                linien.append((a_, b_, 'aussen'))
    for name in ('nord', 'sued'):
        bogen = T[name]['offen'].exterior
        insel = F['inseln'].buffer(0.05)
        c = list(bogen.coords)
        for a_, b_ in zip(c[:-1], c[1:]):
            mitte = Point((a_[0] + b_[0]) / 2, (a_[1] + b_[1]) / 2)
            # nur der Halbkreis (Trogende), nicht die geraden Trogwaende
            if insel.contains(mitte) and abs(mitte.distance(Point(T[name]['mitte'])) - T[name]['r']) < 0.3:
                linien.append((a_, b_, 'trog'))
    for a_, b_, wo in linien:
        L = math.dist(a_, b_)
        if L < 1e-3:
            continue
        za, zb = float(zf(*a_)) + WALK_H, float(zf(*b_)) + WALK_H
        n = np.array([b_[1] - a_[1], a_[0] - b_[0]]) / L
        # Normale nach aussen (weg vom Gehweg/von der Insel)
        innen = unary_union([F['gehweg'], F['inseln']]).buffer(0.02)
        if innen.contains(Point((a_[0] + b_[0]) / 2 + n[0] * 0.1, (a_[1] + b_[1]) / 2 + n[1] * 0.1)):
            n = -n
        A = lambda p, dn, z: (p[0] + n[0] * dn - O[0], p[1] + n[1] * dn - O[1], z)
        if wo == 'aussen':
            ga, gb = (float(gel.h(np.array([q[0] + n[0]]), np.array([q[1] + n[1]]))[0]) for q in (a_, b_))
            unten_a, unten_b = min(ga, za) - 0.5, min(gb, zb) - 0.5
        else:
            unten_a, unten_b = za - 1.2, zb - 1.2                     # Trogrand: bis unter die Deckenkante
        hoch = 0.9 if wo == 'aussen' else 0.2                          # Trogrand: niedrige Kante, darauf das Gelaender
        m.quad('beton', [A(a_, 0.25, unten_a), A(b_, 0.25, unten_b), A(b_, 0.25, zb + hoch), A(a_, 0.25, za + hoch)],
               [(0, 0), (L / 4, 0), (L / 4, 0.4), (0, 0.4)], (n[0], n[1], 0))
        m.quad('beton', [A(b_, 0.0, zb), A(a_, 0.0, za), A(a_, 0.0, za + hoch), A(b_, 0.0, zb + hoch)],
               [(0, 0), (L / 4, 0), (L / 4, 0.2), (0, 0.2)], (-n[0], -n[1], 0))
        m.quad('beton', [A(a_, 0.0, za + hoch), A(b_, 0.0, zb + hoch), A(b_, 0.25, zb + hoch), A(a_, 0.25, za + hoch)],
               [(0, 0), (L / 4, 0), (L / 4, 0.06), (0, 0.06)], (0, 0, 1))
        # Pfosten alle 2 m, zwei Holme
        for k in range(int(L // 2.0) + 1):
            f = (k * 2.0 + 0.5) / L if L > 0.6 else 0.5
            if f > 1:
                break
            q = (a_[0] + (b_[0] - a_[0]) * f, a_[1] + (b_[1] - a_[1]) * f)
            zq = za + (zb - za) * f + hoch
            c = A(q, 0.12, 0)
            kasten(m, 'gelaender', c[0] - 0.03, c[0] + 0.03, c[1] - 0.03, c[1] + 0.03, zq, zq + (0.5 if wo == 'aussen' else 0.9))
        for z0 in ((0.45, 0.1) if wo == 'aussen' else (0.85, 0.45)):
            m.quad('gelaender', [A(a_, 0.12, za + hoch + z0), A(b_, 0.12, zb + hoch + z0), A(b_, 0.12, zb + hoch + z0 + 0.05), A(a_, 0.12, za + hoch + z0 + 0.05)],
                   [(0, 0), (1, 0), (1, 1), (0, 1)], (n[0], n[1], 0), beidseitig=True)
    # ---------------------------------------------------------- Markierungen (2 cm ueber der Fahrbahn)
    weiss, rot, sperr, beton = [], [], [], []
    for a in arme:
        rein, raus = a.spuren()
        if a.i in HALT:
            ende = a.bis(HALT[a.i])
        elif a.i in RAND:
            ende = a.bis(RAND[a.i])
        else:
            continue
        # Haltlinie ueber die ankommenden Spuren (0,5 m breit)
        if a.i in HALT and rein:
            qs = [qq for qq, _ in rein]
            tb = [t['breite'] for _, t in rein]
            lo = min(q - b / 2 for q, b in zip(qs, tb))
            hi = max(q + b / 2 for q, b in zip(qs, tb))
            weiss.append(Polygon([a.punkt(ende - 0.5, lo), a.punkt(ende - 0.5, hi), a.punkt(ende, hi), a.punkt(ende, lo)]))
        # Linien zwischen den Teilen der Fahrbahn, Sperrflaechen der Zufahrt
        teile = raender(a.q, nur_fahrbahn=True)
        marken = a.q.get('marken', [])
        for k, (t, lo, hi) in enumerate(teile):
            if t.get('belag') in ('sperr', 'beton'):
                (sperr if t['belag'] == 'sperr' else beton).append(Polygon([a.punkt(0, lo), a.punkt(0, hi), a.punkt(ende, hi), a.punkt(ende, lo)]))
            if k == 0:
                continue
            mk = marken[k] if k < len(marken) else 'auto'     # Marke zwischen Teil k-1 und k (mit Gehweg vorne: Index verschoben)
            vorne = teile[k - 1][0]
            gleich = vorne['art'] in ('fahrspur', 'busspur') and t['art'] in ('fahrspur', 'busspur') and vorne['richtung'] == t['richtung']
            linie = LineString([a.punkt(0, lo), a.punkt(ende, lo)])
            if gleich and t['art'] == 'fahrspur' and vorne['art'] == 'fahrspur':
                # Leitlinie, die letzten 20 m vor der Haltlinie durchgezogen
                cut = max(0.0, ende - 20.0)
                if cut > 0.5:
                    weiss.append(gestrichelt(LineString([a.punkt(0, lo), a.punkt(cut, lo)])))
                weiss.append(strich(a.punkt(cut, lo), a.punkt(ende, lo)))
            else:
                weiss.append(linie.buffer(0.06, cap_style='flat'))
        # Pfeile (4 m und 14 m vor der Haltlinie), BUS
        vor = -a.weg
        for k, (qq, t) in enumerate(rein):
            arten = pfeile.get((a.i, k))
            if t['art'] == 'busspur':
                if ende > 6:
                    weiss.append(bus_schrift(a.punkt(ende - 5.5, qq), vor, np.array(rvec((a.h + 180) % 360))))
                continue
            if arten:
                for d in (4.5, 14.5):
                    if ende - d - 5 > 0.5:
                        weiss.append(pfeil(arten, a.punkt(ende - d - 5, qq), vor, np.array(rvec((a.h + 180) % 360))))
        for qq, t in raus:
            if t['art'] == 'busspur' and ende > 9:
                weiss.append(bus_schrift(a.punkt(4.0, qq), a.weg, np.array(rvec(a.h))))
    # Radfurten mit gestrichelten Raendern, Radaufstellflaeche, Fussgaengerfurten, Sperrflaeche vor der Insel
    for y0, y1 in ROT:
        rot.append(box(ROT_X[0], y0, ROT_X[1], y1))
        for y in (y0, y1):
            weiss.append(gestrichelt(LineString([(ROT_X[0], y), (ROT_X[1], y)]), 0.15, 0.5, 0.5))
    rot.append(box(RADBOX[0][0], RADBOX[0][1], RADBOX[1][0], RADBOX[1][1]))
    weiss.append(box(HALT_RAD[0], HALT_RAD[2] + 0.0, HALT_RAD[0] + 0.3, -255.9))
    for p, q in FURTEN:
        weiss.append(gestrichelt(LineString([p, q]), 0.15, 0.5, 0.5))
    sperr.append(Polygon(SPERR_O))
    weiss.append(Polygon(SPERR_O).exterior.buffer(0.07))
    # ---------------------------------------------------------- Pfade
    lanes = []
    for a in arme:
        rein, raus = a.spuren()
        lanes.append((rein, raus))
    pfade, regeln, ketten = [], [], []
    rollen = [a.rolle for a in arme]
    halt_pkt = {}

    def hoehe_el(el):
        x, y, h, L, R = el
        (ex, ey), _ = end_of(el)
        z0 = float(zf(x, y))
        return z0 + ASPH_H, float(zf(ex, ey)) - z0

    # Zufahrt bis zur Haltlinie (ohne Ampel), je ankommender Spur
    for a in arme:
        for k, (qq, t) in enumerate(lanes[a.i][0]):
            start = a.punkt(0, qq)
            ende = a.bis(HALT[a.i]) if a.i in HALT else 0.0
            ziel = a.punkt(ende, qq)
            halt_pkt[(a.i, k)] = (ziel, (a.h + 180) % 360)
            if ende > 0.1:
                el = [start[0], start[1], (a.h + 180) % 360, float(np.hypot(*(ziel - start))), 0.0]
                for e in stuecke(el):
                    pfade.append((e, 0, None, hoehe_el(e), t['breite']))
    gruppe_von = {}
    for ia, i, ib, j in verbindungen:
        (p_in, h_in) = halt_pkt[(ia, i)]
        qq, t = lanes[ib][1][j]
        b = arme[ib]
        p_out = b.punkt(0, qq)
        els = connect(tuple(p_in), h_in, tuple(p_out), b.h, 'x')
        if not els:
            raise ValueError(f'keine Verbindung {ia}/{i} -> {ib}/{j}')
        mv = art(h_in, b.h)
        erster = len(pfade)
        teile = [e2 for e in els for e2 in stuecke(list(e[:5]))]
        for n, e in enumerate(teile):
            pfade.append((e, BLINKER.get(mv, 0), phasen[ia] if n == 0 else None, hoehe_el(e), min(3.0, t['breite'])))
        pr = vorfahrt.priority(rollen[ia], rollen[ib], 'links' if mv == 'wenden' else mv, rollen.count('haupt'))
        if pr is not None:
            regeln += [(erster + k, pr) for k in range(len(teile))]
        ketten.append(dict(von=(ia, i), nach=(ib, j), mv=mv, punkte=[e[:2] for e in teile], erster=erster, n=len(teile)))
        # Leitlinie der Linksabbieger: links der Spurmitte, zwischen Haltlinie und Ausfahrt
        if mv == 'links':
            P = [np.array(e[:2]) for e in teile]
            lp = []
            for e in teile:
                hl = np.array(rvec(e[2]))
                lp.append(tuple(np.array(e[:2]) - hl * 1.6))
            if len(lp) > 2:
                weiss.append(gestrichelt(LineString(lp[1:-1]), 0.12, 1.5, 1.5).intersection(F['fahrbahn']))
    flaeche('weiss', unary_union(weiss).intersection(F['fahrbahn'].buffer(0.01)), ASPH_H + 0.02, 1.0)
    flaeche('radfurt', unary_union(rot).intersection(F['fahrbahn']).difference(unary_union(weiss)), ASPH_H + 0.02, 4.0)
    flaeche('busbeton', unary_union(beton).intersection(F['fahrbahn']).difference(unary_union(weiss)), ASPH_H + 0.01, 3.9)
    flaeche('sperr', unary_union(sperr).intersection(F['fahrbahn']).difference(unary_union(weiss)), ASPH_H + 0.015, 2.0)
    # ---------------------------------------------------------- Ampel
    plan = ampel.plane([dict(name=a.name, h=a.h) for a in arme], rollen, phasen)
    signale = []
    for ia, (rechts, links) in MASTEN.items():
        a = arme[ia]
        g = phasen[ia]
        rot_ = a.h % 360
        dv = np.array(dvec(a.h))
        for wo, p in (('rechts', rechts), ('links', links)):
            p = np.array(p)
            boden = float(zf(*p)) + WALK_H
            signale.append(dict(art='signal', datei=ampel.SIGNAL, x=float(p[0]), y=float(p[1]), hoehe=ampel.SIGNAL_HOEHE, rot=rot_, gruppe=g,
                                z_boden=boden))
            mp = p - ampel.MAST_VERSATZ * dv
            signale.append(dict(art='mast', datei=ampel.MAST, x=float(mp[0]), y=float(mp[1]), hoehe=ampel.MAST_HOEHE, rot=(rot_ + 180) % 360,
                                gruppe=None, z_boden=float(zf(*mp)) + WALK_H))
            if wo == 'rechts':
                signale.append(dict(art='oben', datei=ampel.OBEN, eltern=len(signale) - 1, anhang=0, rot=ampel.OBEN_DREHUNG, gruppe=g))
    erhoeht = unary_union([F['gehweg'], F['inseln']])
    lose = [s for s in signale if s['art'] in ('signal', 'mast') and not erhoeht.buffer(0.3).contains(Point(s['x'], s['y']))]
    return dict(mesh=m, O=O, pfade=pfade, regeln=regeln, ketten=ketten, plan=plan, signale=signale, flaechen=F, zf=zf, trog=T,
                arme=arme, lose_signale=[(round(s_['x'], 1), round(s_['y'], 1), s_['art']) for s_ in lose])


def sco_text(name, modell, k, alpha):
    """[surface]-Objekt mit Ampelgruppe, Pfaden (Hoehe und Hoehenaenderung je Stueck) und Mesh"""
    L = ['Erzeugt mit omsigen (ring_kreuzung, Einzelstueck). OSM (ODbL), DOP40/DGM1 Bayern (CC BY 4.0).', '',
         '[friendlyname]', name, '', '[groups]', '1', 'Aschaffenburg', '', '[rendertype]', 'surface', '', '[LightMapMapping]', '',
         '[fixed]', '', '[surface]', '', '[absheight]', '']
    L += list(ampel.sco_block(k['plan']))
    O = k['O']
    for (x, y, h, Ln, R), blinker, gruppe, (z, dh), breite in k['pfade']:
        vals = [x - O[0], y - O[1], z, h % 360, R, Ln, 0, dh, 0, breite, 0, blinker]
        L += ['[path]'] + [f'{v:.4f}' if isinstance(v, float) else str(v) for v in vals] + ['']
        if gruppe is not None:
            L += ['[use_traffic_light]', str(gruppe), '']
    L += ['[mesh]', modell, '']
    for t in alpha:
        L += ['[matl]', t, '0', '[matl_alpha]', '2', '']
    return '\r\n'.join(L) + '\r\n'
