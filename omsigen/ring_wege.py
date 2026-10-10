"""Wege abseits der Kreuzungsplatte (Sonderbau Ring): Fusswege aus dem Bauplan (`wege`: [{id, punkte, breite, belag}],
Mittellinie nach Luftbild DOP20 / Mapillary-Fotos) als Splines mit Gehweg-Querschnitt (Baukasten), wie Strassen.

Hoehe: die Oberflaeche liegt WEG_UEBER m ueber dem, was dort sonst sichtbar ist - Gelaendedeckel des Sonderbaus bzw.
Kartengelaende (5-m-Raster). Der Deckel wird unter dem Weg ausgespart (etwas schmaler als der Weg, dessen Rand nach
unten reicht), so liegen keine Flaechen uebereinander und es entsteht keine Fuge."""
import math

import numpy as np

WEG_UEBER = 0.06
HOCHBORD = 0.25          # ein Gehweg-Querschnitt allein liegt so hoch ueber der Spline (querschnitt.rs)
KNOTEN = 2.5
UNTER_WEG = 0.15         # Deckel reicht so weit unter den Wegrand


def dicht(punkte, schritt=0.5):
    """Linienzug -> Punkte alle `schritt` m (mit Anfang und Ende)"""
    from shapely.geometry import LineString
    ls = LineString(punkte)
    n = max(1, math.ceil(ls.length / schritt))
    return np.array([ls.interpolate(i * ls.length / n).coords[0] for i in range(n + 1)])


def glatt(Z, fenster=5):
    """gleitender Mittelwert, nie tiefer als das Rohsignal (der Weg soll nirgends einsinken)"""
    if len(Z) < fenster:
        return Z
    k = np.ones(fenster) / fenster
    g = np.convolve(np.pad(Z, fenster // 2, mode='edge'), k, mode='valid')[:len(Z)]
    return np.maximum(g, Z)


def querschnitt(w):
    return dict(name=f"SB_Weg_{w['id']}", teile=[dict(art='gehweg', breite=round(float(w.get('breite', 2.5)), 3), richtung='vor',
                                                       belag=w.get('belag', 'asphalt'))], marken=[])


def splines(wege, hoehe):
    """Wege -> Strassen (dict name, art='weg', querschnitt, elemente); `hoehe(x, y)`: sichtbare Oberflaeche dort"""
    from .ring_waende import kette
    out = []
    for w in wege:
        if len(w.get('punkte', [])) < 2:
            continue
        P = dicht(w['punkte'])
        Z = glatt(np.array([hoehe(x, y) for x, y in P]) + WEG_UEBER) - HOCHBORD
        out.append(dict(name=f"Weg_{w['id']}", art='weg', querschnitt=querschnitt(w), elemente=kette(P, Z, knoten=KNOTEN)))
    return out


def mittellinie(els, schritt=0.5):
    """Punkte entlang der Spline-Elemente (OMSI-Bogenregel wie ring_strassen._ende)"""
    pts = []
    for e in els:
        n = max(1, math.ceil(e['laenge'] / schritt))
        for k in range(n):
            pts.append(_punkt(e, e['laenge'] * k / n))
    if els:
        pts.append(_punkt(els[-1], els[-1]['laenge']))
    return np.array(pts)


def _punkt(e, s_):
    h = math.radians(e['richtung'])
    if e['radius'] == 0:
        return (e['x'] + s_ * math.sin(h), e['y'] + s_ * math.cos(h))
    R = e['radius']
    cx, cy = e['x'] + R * math.cos(h), e['y'] - R * math.sin(h)
    h2 = h + s_ / R
    return (cx - R * math.cos(h2), cy + R * math.sin(h2))


def grundriss(wege, einzug=0.0):
    """Flaeche der Wege (je Breite, um `einzug` m schmaler) entlang der tatsaechlichen Spline-Mittellinie, shapely.
    `wege`: Strassen aus splines()"""
    from shapely.geometry import LineString, Polygon
    from shapely.ops import unary_union
    teile = []
    for w in wege:
        P = mittellinie(w['elemente'])
        if len(P) >= 2:
            b = float(w['querschnitt']['teile'][0]['breite'])
            teile.append(LineString(P).buffer(max(0.1, b / 2 - einzug), cap_style='flat', join_style='round'))
    return unary_union(teile) if teile else Polygon()


def hoehe_aus(teile, karte):
    """sichtbare Oberflaeche: hoechstes Dreieck der Deckel-Meshes (je (Mesh, Ursprung)), sonst Kartengelaende"""
    from shapely.geometry import Polygon, Point
    from shapely.strtree import STRtree
    tris = []
    for m, o in teile:
        V = m.V
        for a, b, c, _ in m.F:
            P = np.array([[V[i][0] + o[0], V[i][2] + o[1], V[i][1] + o[2]] for i in (a, b, c)])     # .x: x, hoch, vor
            if abs((P[1][0] - P[0][0]) * (P[2][1] - P[0][1]) - (P[2][0] - P[0][0]) * (P[1][1] - P[0][1])) > 1e-6:
                tris.append(P)
    polys = [Polygon(P[:, :2]) for P in tris]
    baum = STRtree(polys) if polys else None

    def h(x, y):
        best = None
        if baum is not None:
            pt = Point(x, y)
            for i in baum.query(pt.buffer(1e-4)):
                if polys[int(i)].buffer(1e-4).contains(pt):
                    (x0, y0, z0), (x1, y1, z1), (x2, y2, z2) = tris[int(i)]
                    det = (x1 - x0) * (y2 - y0) - (x2 - x0) * (y1 - y0)
                    u = ((x - x0) * (y2 - y0) - (x2 - x0) * (y - y0)) / det
                    v = ((x1 - x0) * (y - y0) - (x - x0) * (y1 - y0)) / det
                    z = z0 + u * (z1 - z0) + v * (z2 - z0)
                    best = z if best is None else max(best, z)
        if best is None:
            best = karte.hoehe(x, y)
        return 0.0 if best is None else float(best)
    return h


def anschmiegen(wege, band=2.5, unter=0.03):
    """Gelaendedeckel an die Wege anpassen -> f(x, y, z) -> z: am Wegrand knapp unter der Wegoberflaeche, ueber `band` m
    zurueck auf die urspruengliche Hoehe (wie ein Weg im Hang, ohne Fuge zwischen Weg und Boden)"""
    from shapely.geometry import LineString, Point
    linien = []
    for w in wege:
        P = mittellinie(w['elemente'], 0.5)
        if len(P) < 2:
            continue
        # Oberflaeche je Mittellinienpunkt: Spline-Hoehe (linear im Element) + Hochbord
        Z = []
        for e in w['elemente']:
            n = max(1, math.ceil(e['laenge'] / 0.5))
            Z += [e['z'] + e['dh'] * k / n + HOCHBORD for k in range(n)]
        e = w['elemente'][-1]
        Z.append(e['z'] + e['dh'] + HOCHBORD)
        linien.append((LineString(P), np.array(P), np.array(Z), float(w['querschnitt']['teile'][0]['breite']) / 2))

    def f(x, y, z):
        pt = Point(x, y)
        for ls, P, Z, hb in linien:
            d = ls.distance(pt)
            if d > hb + band:
                continue
            i = int(np.argmin(np.hypot(P[:, 0] - x, P[:, 1] - y)))
            ziel = Z[i] - unter
            w = 1.0 if d <= hb else 1.0 - (d - hb) / band
            return w * ziel + (1 - w) * z
        return z
    return f


def raender(wege_plan, raender_plan, wege, hoehe):
    """Wegraender (Bauplan wege[].rand_links/rand_rechts = {art, von, bis} in m entlang des Wegs) und freie Raender
    (Bauplan raender: [{id, punkte, art, seite: 'links'|'rechts'}]) als Spline-Ketten -> (Ketten, Dateien {name: sli})"""
    from shapely.geometry import LineString
    from shapely.ops import substring
    from . import ring_waende as rw
    linien = []
    nach_name = {w['name']: w for w in wege}
    for wp in wege_plan:
        w = nach_name.get(f"Weg_{wp['id']}")
        if w is None:
            continue
        mitte = LineString(mittellinie(w['elemente']))
        hb = float(w['querschnitt']['teile'][0]['breite']) / 2
        for seite, vz in (('links', 1), ('rechts', -1)):
            r = wp.get(f'rand_{seite}')
            if not r:
                continue
            teil = substring(mitte, float(r.get('von', 0.0)), float(r.get('bis', mitte.length)))
            linien.append((f"{wp['id']}_{seite}", teil.offset_curve(vz * (hb + 0.05)), r.get('art', 'mauer_hecke'), -vz))
    for r in raender_plan:
        if len(r.get('punkte', [])) >= 2:
            linien.append((r['id'], LineString(r['punkte']), r.get('art', 'mauer_hecke'), 1 if r.get('seite', 'rechts') == 'rechts' else -1))
    ketten, dateien = [], {}
    for name, ls, art, s in linien:
        if art not in rw.RAND_ARTEN or ls.is_empty or ls.length < 1.0:
            continue
        P = dicht(list(ls.coords), 0.5)
        Z = np.array([hoehe(x, y) for x, y in P])
        datei = f"AB_SB_Rand_{art}_{'r' if s > 0 else 'l'}"
        dateien[datei] = rw.profil_rand(art, s).text(f'File created with omsigen (Sonderbau Ring). Wegrand {art}.')
        ketten.append(dict(name=f'Rand_{name}', art='wand', bauteil=None, sli=f'Splines\\Aschaffenburg\\{datei}.sli',
                           elemente=rw.kette(P, Z, knoten=KNOTEN), teile_umriss=umriss_rand(art, s)))
    return ketten, dateien


def umriss_rand(art, s):
    """Querschnitt eines Rands in Teilen mit Textur (fuer die Stirnflaechen): [(Textur, [(x, y)])]"""
    from . import ring_waende as rw
    if art == 'mauer_hecke':
        return [('sandstein', [(0.0, -0.3), (0.0, 0.7), (s * 0.4, 0.7), (s * 0.4, -0.3)]),
                ('hecke', [(s * 0.4, -0.3), (s * 0.4, rw.HECKE_H), (s * 1.4, rw.HECKE_H), (s * 1.4, -0.3)])]
    return []


def rand_enden(ketten, o):
    """Stirnflaechen an Anfang und Ende jeder Rand-Kette (Splines haben keine) -> Mesh (Ursprung o)"""
    import shapely
    from shapely.geometry import Polygon
    from .ring_strassen import _ende
    from .sonderbau_ring import Mesh
    m = Mesh()
    for k in ketten:
        els = k['elemente']
        if not els:
            continue
        a = els[0]
        xe, ye, he = _ende(els[-1])
        for (px_, py_), z, h, vz in (((a['x'], a['y']), a['z'], a['richtung'], -1), ((xe, ye), els[-1]['z'] + els[-1]['dh'], he, 1)):
            hr = math.radians(h)
            rechts = np.array([math.cos(hr), -math.sin(hr)])
            vor = np.array([math.sin(hr), math.cos(hr)]) * vz
            for tex, um in k.get('teile_umriss', []):
                for tri in shapely.constrained_delaunay_triangles(Polygon(um)).geoms:
                    pts = list(tri.exterior.coords)[:3]
                    w = [(px_ + rechts[0] * qx - o[0], py_ + rechts[1] * qx - o[1], z + qy) for qx, qy in pts]
                    uv = [(qx / 1.5, qy / 1.5) for qx, qy in pts]
                    m.dreieck(tex, w, uv, (vor[0], vor[1], 0))
                    m.dreieck(tex, w, uv, (-vor[0], -vor[1], 0))
    return m

