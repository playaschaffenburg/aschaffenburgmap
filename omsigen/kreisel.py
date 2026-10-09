"""Kreisverkehr als ein Objekt - wie in Rheinhausen (Zane_Crossings/Kreisverkehr2.sco: ein rundes Modell mit allen
Fahrpfaden) statt aus Ring-Splines und Kreuzungsplatten, die nie rund aussehen.

Lokale Geometrie (Objekt in der Mitte des Rings, Drehung 0; x = Ost, z = Nord, Richtungen im Uhrzeigersinn ab Nord):
- Ringfahrbahn zwischen Innenradius ri = r - breite/2 und Aussenradius ro = r + breite/2, Mittelinsel (Gras, Bordstein),
  an jedem Arm ein Trichter von der Fahrbahn des Arms zum Ring; die Ecken zwischen Trichter und Ring sind
  ausgerundet (morphologisches Schliessen), aussen herum Gehweg.
- Fahrpfade: eine Ringspur (Radius r, gegen den Uhrzeigersinn) aus Kreisboegen zwischen den Aus- und Einfahrpunkten;
  jede ankommende Spur eines Arms faedelt tangential in den Ring ein, jede abgehende faedelt vorher aus.
- Vorfahrt: Ring 192, Einfahrten 64 (wie Haupt-/Nebenstrasse in Grundorf, docs/omsi-format.md).
- Gehwegpfade: aussen um den Ring von Arm zu Arm.
Die Arme muessen ausserhalb von ro + R_ECKE enden (der Editor bemisst sie so)."""
import math
import shapely
from shapely.geometry import Point, Polygon
from shapely.ops import unary_union
from .geom import heading, dvec, rvec, connect, norm180
from . import kreuzung
from .kreuzung import ASPH_H, WALK_H, U_KERB, U_OUT, _polys, _tris, _arm
from .network import spurenden

R_ECKE = 6.0                 # Ausrundung zwischen Trichter und Ring
SEGMENTE = 48                # Viertelkreis-Segmente der Kreise (192 je Kreis)
GEHWEG = 3.0                 # Gehwegbreite um den Ring, wenn die Arme keine haben
INSELRAND = 1.5              # gepflasterter Rand der Mittelinsel
GRAS_M = 20.0                # Meter je Wiederholung der Grastextur (wie das Gelaende)
P_RING, P_EIN = 192, 64
MATS = kreuzung.MATS + [('Gras', 'gras.bmp')]


def _kreis(r):
    return Point(0, 0).buffer(r, quad_segs=SEGMENTE)


def _auf_ring(r, phi):
    d = dvec(phi)
    return (d[0] * r, d[1] * r)


def _bogen(r, phi_a, phi_b, richtung):
    """Kreisbogen um die Mitte von Winkel phi_a nach phi_b; richtung -1: gegen den Uhrzeigersinn (Ring), +1: mit"""
    d = ((phi_a - phi_b) if richtung < 0 else (phi_b - phi_a)) % 360
    if d < 1e-6:
        return []
    p = _auf_ring(r, phi_a)
    h = (phi_a - 90) % 360 if richtung < 0 else (phi_a + 90) % 360
    return [[p[0], p[1], h, r * math.radians(d), -r if richtung < 0 else r, None]]


def _ok(els, a, b):
    return bool(els) and sum(e[3] for e in els) < 3 * math.dist(a, b) + 20


def bauen(arms, mitte, r, breite, sdb):
    """arms: Liste dict(pos, h, spl, away) in Weltkoordinaten (h: Richtung vom Ring weg) -> dict(origin, asphalt,
    side, insel, kerbs, moves [(Elemente, Blinker)], rules [(Pfad, Prioritaet)], walks)"""
    O = (float(mitte[0]), float(mitte[1]))
    ri, ro = r - breite / 2, r + breite / 2
    if ri < 3:
        raise ValueError(f'Kreisverkehr zu klein (Innenradius {ri:.1f} m)')
    loc = [dict(a, pos=(a['pos'][0] - O[0], a['pos'][1] - O[1])) for a in arms]
    A = [_arm(a, sdb) for a in loc]
    for a in A:
        if math.hypot(*a['pos']) < ro + 2:
            raise ValueError('eine Zufahrt endet im Ring - Arme muessen ausserhalb enden')
    # Flaechen: Ring + Trichter, Ecken ausgerundet; Insel; Gehweg aussen herum
    trichter = []
    for a in A:
        d_in = dvec((a['h'] + 180) % 360)
        L = math.hypot(*a['pos']) - r + 1.0
        cl, cr = a['CL'], a['CR']
        trichter.append(Polygon([cl, cr, (cr[0] + L * d_in[0], cr[1] + L * d_in[1]), (cl[0] + L * d_in[0], cl[1] + L * d_in[1])]))
    aussen = unary_union([_kreis(ro)] + trichter).buffer(R_ECKE, quad_segs=16).buffer(-R_ECKE, quad_segs=16)
    insel = _kreis(ri)

    def fuss(a, extra):
        dv, rv = dvec(a['h']), rvec(a['h'])
        lo, hi = -a['ol'] - extra, a['or_'] + extra
        P = a['pos']
        pt = lambda q, s: (P[0] + q * rv[0] + s * dv[0], P[1] + q * rv[1] + s * dv[1])
        return Polygon([pt(lo, 0), pt(hi, 0), pt(hi, kreuzung.FOOT_LEN), pt(lo, kreuzung.FOOT_LEN)])
    fuesse = unary_union([fuss(a, 0.0) for a in A])
    breiten = [w for a in A for w in (a['wl'], a['wr']) if w > 0.5]
    w_geh = sum(breiten) / len(breiten) if breiten else GEHWEG
    asphalt = aussen.difference(insel).difference(fuesse)
    side = aussen.buffer(w_geh, quad_segs=16).difference(aussen).difference(unary_union([fuss(a, w_geh + 2) for a in A]))
    side = unary_union(_polys(side))
    kerbs = [shapely.geometry.LineString(list(aussen.exterior.coords))]

    # Fahrpfade: je Arm ankommende/abgehende Spuren, Ein- und Ausfahrpunkt auf dem Ring (Winkel vor bzw. hinter dem
    # Arm); der Winkel aus der Lage der Spur (die Spur trifft den Ring etwa bei asin(Versatz/r)) plus Rundung, hoechstens
    # so gross, dass die Ausfahrt eines Arms und die Einfahrt des naechsten sich nicht kreuzen
    phis = [heading((0, 0), a['pos']) for a in A]
    spuren_je = []
    for a, a0 in zip(A, loc):
        if a0['away']:
            spuren = spurenden(a0['pos'], a0['h'], a0['spl'], 's', sdb)
        else:
            spuren = spurenden(a0['pos'], (a0['h'] + 180) % 360, a0['spl'], 'e', sdb)
        rv = rvec(a['h'])
        q = lambda l: abs((l['p'][0] - a['pos'][0]) * rv[0] + (l['p'][1] - a['pos'][1]) * rv[1])
        rein = [l for l in spuren if l['kind'] == 'in']
        raus = [l for l in spuren if l['kind'] == 'out']

        def winkel(ls):
            if not ls:
                return 0.0
            return min(max(math.degrees(math.asin(min(0.9, (max(q(l) for l in ls) + 1.5) / r))) + 10.0, 12.0), 45.0)
        spuren_je.append((rein, raus, winkel(rein), winkel(raus)))
    th_ein = [x[2] for x in spuren_je]
    th_aus = [x[3] for x in spuren_je]
    if len(A) > 1:
        for k in range(len(A)):
            # naechster Arm in Fahrtrichtung (gegen den Uhrzeigersinn: kleinerer Winkel)
            j = min((i for i in range(len(A)) if i != k), key=lambda i: (phis[k] - phis[i]) % 360)
            luecke = (phis[k] - phis[j]) % 360
            platz = luecke - 8.0
            if th_ein[k] + th_aus[j] > platz:
                f = max(platz, 1.0) / (th_ein[k] + th_aus[j])
                th_ein[k] *= f
                th_aus[j] *= f
    ereignisse = []          # (Winkel, Art, Arm)
    ein, aus = {}, {}
    for k, (rein, raus, _, _) in enumerate(spuren_je):
        if rein:
            ein[k] = ((phis[k] - th_ein[k]) % 360, rein)
            ereignisse.append((ein[k][0], 'ein', k))
        if raus:
            aus[k] = ((phis[k] + th_aus[k]) % 360, raus)
            ereignisse.append((aus[k][0], 'aus', k))
    if not ereignisse:
        # (noch) ohne Zufahrten: nur der Ring, zwei Halbkreise
        ereignisse = [(0.0, 'ring', None), (180.0, 'ring', None)]
    # in Fahrtrichtung (gegen den Uhrzeigersinn = fallender Winkel)
    ereignisse.sort(key=lambda e: -e[0])
    for (w1, _, _), (w2, _, _) in zip(ereignisse, ereignisse[1:] + ereignisse[:1]):
        if len(ereignisse) > 1 and (w1 - w2) % 360 < 2:
            raise ValueError('Zufahrten zu dicht beieinander - Kreisverkehr groesser waehlen')
    moves, rules, idx = [], [], 0
    no_cars = []

    def neu(els, blinker, prio, gesperrt=False):
        nonlocal idx
        moves.append((els, blinker))
        if prio is not None:
            rules.extend((idx + j, prio) for j in range(len(els)))
        if gesperrt:
            no_cars.extend(idx + j for j in range(len(els)))
        idx += len(els)
    n = len(ereignisse)
    for i in range(n):
        w1, w2 = ereignisse[i][0], ereignisse[(i + 1) % n][0]
        if n == 1:
            w2 = w1 - 359.999
        neu(_bogen(r, w1, w2, -1), 0, P_RING)
    for k, (w, spuren) in ein.items():
        E = _auf_ring(r, w)
        for l in spuren:
            els = connect(l['p'], l['h'], E, (w - 90) % 360, None)
            if not _ok(els, l['p'], E):
                raise ValueError('Einfahrt laesst sich nicht legen')
            neu(els, 0, P_EIN)
    for k, (w, spuren) in aus.items():
        X = _auf_ring(r, w)
        for l in spuren:
            els = connect(X, (w - 90) % 360, l['p'], l['h'], None)
            if not _ok(els, X, l['p']):
                raise ValueError('Ausfahrt laesst sich nicht legen')
            neu(els, kreuzung.BLINKER['rechts'], None, bool(arms[k].get('gesperrt')))

    # Gehwegpfade aussen herum, von Arm zu Arm (im Uhrzeigersinn)
    walks = []
    rw = ro + w_geh / 2
    folge = sorted(range(len(A)), key=lambda i: heading((0, 0), A[i]['pos']))
    for m, i in enumerate(folge):
        j = folge[(m + 1) % len(folge)]
        a, b = A[i], A[j]
        if a['walk_r'] is None or b['walk_l'] is None or (i == j and len(A) > 1):
            continue
        p, q = a['pt'](a['walk_r']), b['pt'](b['walk_l'])
        phi_a = heading((0, 0), a['pos']) + math.degrees((a['walk_r'] + R_ECKE) / rw)
        phi_b = heading((0, 0), b['pos']) - math.degrees((abs(b['walk_l']) + R_ECKE) / rw)
        if len(A) == 1:
            phi_b += 360
        if (phi_b - phi_a) % 360 > 300 and len(A) > 1:
            continue
        P1, P2 = _auf_ring(rw, phi_a), _auf_ring(rw, phi_b)
        e1 = connect(p, (a['h'] + 180) % 360, P1, (phi_a + 90) % 360, None)
        e3 = connect(P2, (phi_b + 90) % 360, q, b['h'], None)
        if _ok(e1, p, P1) and _ok(e3, P2, q):
            walks.append(e1 + _bogen(rw, phi_a, phi_b, 1) + e3)
    return dict(origin=O, asphalt=asphalt, side=side, insel=insel, kerbs=kerbs, moves=moves, rules=rules,
                walks=walks, ri=ri, ro=ro, no_cars=no_cars)


def mesh(K):
    """-> (vertices, faces): Material 0 Asphalt, 1 Gehweg, 2 Gras; Bordsteinkanten aussen und an der Insel"""
    V, F = [], []

    def add(pts, n, uvs, m):
        i0 = len(V)
        for (x, y, z), (u, v) in zip(pts, uvs):
            V.append((x, y, z) + n + (u, v))
        F.append((i0, i0 + 1, i0 + 2, m))

    for a, b, c in _tris(K['asphalt']):
        add([(p[0], ASPH_H, p[1]) for p in (a, b, c)], (0, 1, 0), [(p[0] / 3.5, p[1] / 6.0) for p in (a, b, c)], 0)
    kerbs = K['kerbs']

    def side_uv(p):
        P = Point(p)
        k = min(kerbs, key=lambda l: l.distance(P))
        d = k.distance(P)
        return (max(0.0, U_KERB - (U_KERB - U_OUT) * d / 3.5), k.project(P) * 0.2)
    for a, b, c in _tris(K['side']):
        add([(p[0], WALK_H, p[1]) for p in (a, b, c)], (0, 1, 0), [side_uv(p) for p in (a, b, c)], 1)
    # Insel: Gras, aussen ein gepflasterter Rand (Textur des Gehwegs, vom Bordstein nach innen)
    ri = K['ri']
    gras = _kreis(max(ri - INSELRAND, 0.5))
    for a, b, c in _tris(gras):
        add([(p[0], WALK_H, p[1]) for p in (a, b, c)], (0, 1, 0), [(p[0] / GRAS_M, p[1] / GRAS_M) for p in (a, b, c)], 2)

    def rand_uv(p):
        d = ri - math.hypot(*p)
        return (max(0.0, U_KERB - (U_KERB - U_OUT) * d / 3.5), math.radians(heading((0, 0), p)) * ri * 0.2)
    for a, b, c in _tris(K['insel'].difference(gras)):
        uv = [rand_uv(p) for p in (a, b, c)]
        # Naht bei 360 Grad: v nicht ueber die ganze Runde springen lassen
        vs = [x[1] for x in uv]
        if max(vs) - min(vs) > math.pi * ri * 0.2:
            uv = [(u, v + 2 * math.pi * ri * 0.2 if v < math.pi * ri * 0.2 else v) for u, v in uv]
        add([(p[0], WALK_H, p[1]) for p in (a, b, c)], (0, 1, 0), uv, 1)

    def kante(p, q, zur_fahrbahn, v0):
        """senkrechte Bordsteinflaeche p-q, Vorderseite zur Fahrbahn"""
        dx, dz = q[0] - p[0], q[1] - p[1]
        L = math.hypot(dx, dz)
        nx, nz = dz / L, -dx / L
        m = ((p[0] + q[0]) / 2, (p[1] + q[1]) / 2)
        if not zur_fahrbahn(m[0] + 0.05 * nx, m[1] + 0.05 * nz):
            p, q, nx, nz = q, p, -nx, -nz
        v1 = v0 + L * 0.2
        n = (nx, 0, nz)
        tp, tq, bp, bq = (p[0], WALK_H, p[1]), (q[0], WALK_H, q[1]), (p[0], ASPH_H, p[1]), (q[0], ASPH_H, q[1])
        add([tp, tq, bq], n, [(U_KERB, v0), (U_KERB, v1), (0.995, v1)], 1)
        add([tp, bq, bp], n, [(U_KERB, v0), (0.995, v1), (0.995, v0)], 1)
        return v1
    fahrbahn = K['asphalt'].buffer(1e-3)
    zur = lambda x, z: fahrbahn.contains(Point(x, z))
    ab = K['asphalt'].boundary
    for poly in _polys(K['side']):
        for ring in [poly.exterior] + list(poly.interiors):
            cs = list(ring.coords)
            for p, q in zip(cs, cs[1:]):
                if math.dist(p, q) < 1e-3 or ab.distance(Point((p[0] + q[0]) / 2, (p[1] + q[1]) / 2)) > 0.02:
                    continue
                kante(p, q, zur, side_uv(p)[1])
    cs = list(K['insel'].exterior.coords)
    v = 0.0
    for p, q in zip(cs, cs[1:]):
        v = kante(p, q, zur, v)
    return V, F


def x_file(K):
    V, F = mesh(K)
    return kreuzung.x_file(V, F, MATS), len(F)


def sco_text(name, mesh_file, K):
    return kreuzung.sco_text(name, mesh_file, K['moves'], K['walks'])
