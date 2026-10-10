"""Lernstueck Suedportal: ein reales Bauwerk aus Messdaten modellieren, ohne Schema und ohne die fruehere Ring-Version.

Vorgehen (vermessen -> frei modellieren -> fuer OMSI zerlegen -> pruefen):
1. Laserpunkte (Klasse 2, Boden) des Ausschnitts; daraus mit omsigen.vermessung die Waende/Kanten als Linien
   (Querschnitte je 1 m, Hoehenspruenge ohne Punkte dazwischen = senkrechte Flaeche).
2. Oberflaeche: Rasterpunkte (1 m) mit dem Median der Bodenpunkte in ihrer Naehe, nicht naeher als 0,5 m an einer Wand;
   jede Wand als zwei Punktreihen dicht nebeneinander (unten/oben). Eine Delaunay-Triangulierung aller Punkte ergibt die
   Flaechen und zwischen den Reihen die (fast) senkrechten Wandflaechen - Sonderformen entstehen so von selbst.
3. Material: Waende des Trogs (unten auf Sohlenhoehe) rote Laermschutz-Paneele (Fotos), andere Waende Beton; Flaechen
   nach dem Luftbild DOP20 (gruen = Gras, sonst Asphalt); weisse Markierungen aus dem Luftbild als eigene Schicht;
   Gelaender auf Waenden ueber 2 m.
4. Fuer OMSI: ein Objekt (.x/.sco), Gelaendepunkte unter dem Modell abgesenkt, ein unsichtbarer Spline auf der Ringsohle
   (Einsetzen braucht eine Strasse; Fahrspuren folgen spaeter).

Quellen: Laserdaten, DOP20 Bayerische Vermessungsverwaltung (CC BY 4.0)."""
import argparse
import json
import math
import os

import numpy as np

from . import vermessung as V

TEX = {'asphalt': 'str_asphdrk.bmp', 'boden': 'AB_boden.bmp', 'beton': 'betonwand1.bmp', 'paneel': 'AB_paneel_rot.bmp',
       'weiss': 'AB_weiss.bmp', 'stab': 'AB_stabgelaender.tga'}
RASTER = 1.0
WAND_ABSTAND = 0.5
REIHE = 0.15            # halber Abstand der beiden Wandreihen
GELAENDER = 1.0


def lese_punkte(laz, bereich, cache=None):
    if cache and os.path.isfile(cache):
        P = np.load(cache)
        x0, y0, x1, y1 = bereich
        m = (P[:, 0] >= x0) & (P[:, 0] <= x1) & (P[:, 1] >= y0) & (P[:, 1] <= y1)
        if m.sum() > 1000:
            return P[m]
    P = V.laden(laz, bereich)
    if cache:
        np.save(cache, P)
    return P


def wandlinien(P, bereich, schritt=1.0, min_hoehe=0.5, min_laenge=6):
    x0, y0, x1, y1 = bereich
    schnitte = [(ys, V.kanten(*V.querschnitt(P, ys))) for ys in np.arange(y0 + 0.5, y1, schritt)]
    L = V.linien(schnitte, min_laenge=min_laenge)
    return [glaetten(l) for l in L if np.median([b[3] - b[2] for b in l]) >= min_hoehe]


def glaetten(l, fenster=5, dichte=0.5):
    """Wandlinie glaetten (Lage und Hoehen gleitend gemittelt, die Messung zittert um +-0,2 m) und alle `dichte` m
    abtasten, damit die beiden Punktreihen einer Wand sich nie kreuzen"""
    A = np.array([b[:4] for b in l], float)                  # y, x, z_unten, z_oben
    k = np.ones(fenster) / fenster
    S = A.copy()
    for c in (1, 2, 3):
        S[:, c] = np.convolve(np.pad(A[:, c], fenster // 2, mode='edge'), k, mode='valid')[:len(A)]
    ys = np.arange(S[0, 0], S[-1, 0] + (dichte if S[-1, 0] > S[0, 0] else -dichte) * 0.5, dichte if S[-1, 0] > S[0, 0] else -dichte)
    seite = l[0][4]
    return [(float(y), float(np.interp(y, S[:, 0], S[:, 1])), float(np.interp(y, S[:, 0], S[:, 2])), float(np.interp(y, S[:, 0], S[:, 3])), seite)
            for y in ys]


def boden_z(P, x, y, r):
    m = (np.abs(P[:, 0] - x) < r) & (np.abs(P[:, 1] - y) < r)
    return float(np.median(P[m, 2])) if m.sum() >= 3 else None


def dop_bild(bereich, cache):
    from PIL import Image
    from .luftbildvergleich import dop20
    x0, y0, x1, y1 = bereich
    w, h = int(round((x1 - x0) * 5)), int(round((y1 - y0) * 5))
    f = dop20(V.E0 + x0, V.N0 + y0, V.E0 + x1, V.N0 + y1, w, h, cache)
    return np.asarray(Image.open(f).convert('RGB')).astype(float)


def materialraster(P, bereich, dop, linien=()):
    """Material je 0,2-m-Zelle: 1 Asphalt (grau, wenig gesaettigt), 2 Gras (gruen); unter Baumkronen (Laserpunkte mehr als
    1,5 m ueber dem Boden) und in dunklen Schatten unbekannt -> von der naechsten bekannten Zelle uebernommen, dann
    Mehrheit im 1,4-m-Fenster (glatte Grenzen)"""
    from scipy import ndimage
    x0, y0, x1, y1 = bereich
    H, W = dop.shape[:2]
    r_, g_, b_ = dop[..., 0], dop[..., 1], dop[..., 2]
    v = dop.mean(axis=2)
    sat = dop.max(axis=2) - dop.min(axis=2)
    kl = np.zeros((H, W), np.uint8)
    # Asphalt ist fast farblos (Saettigung < 20, leicht gruenlich-grau); Gras/Laub gruen und gesaettigt, trockene
    # Wiese beige (stark gesaettigt) - gemessen am DOP20 des Suedportals
    kl[(sat < 20) & (v > 60)] = 1
    kl[((g_ > r_ + 4) & (g_ > b_ + 4) & (sat > 18)) | (sat > 35)] = 2
    # Baumkronen aus den Laserdaten: hoechster Punkt je Zelle gegen den Boden darunter
    ix = np.clip(((P[:, 0] - x0) * 5).astype(int), 0, W - 1)
    iy = np.clip(((y1 - P[:, 1]) * 5).astype(int), 0, H - 1)
    oben = np.full((H, W), -1e9)
    np.maximum.at(oben, (iy, ix), P[:, 2])
    boden = np.full((H, W), 1e9)
    g = P[:, 3] == 2
    np.minimum.at(boden, (iy[g], ix[g]), P[g, 2])
    boden = ndimage.grey_erosion(np.where(boden > 1e8, 1e9, boden), size=7)
    oben = ndimage.grey_dilation(oben, size=3)
    krone = (oben - np.where(boden > 1e8, oben, boden)) > 1.5
    kl[krone] = 0
    kl[v < 50] = 0
    # unter Kronen auf steilem Boden (Boeschung, Bahneinschnitt): Gras
    bz = np.where(boden > 1e8, np.nan, boden)
    gy, gx = np.gradient(np.nan_to_num(bz, nan=0.0), 0.2)
    steil = np.hypot(gx, gy) > 0.25
    kl[krone & steil] = 2
    # Bereiche zwischen den Waenden: Unbekanntes nur aus dem eigenen Bereich fuellen (Trogsohle nicht aus der Wiese)
    from PIL import Image as _I, ImageDraw as _D
    wm = _I.new('L', (W, H), 0)
    dw = _D.Draw(wm)
    for l in linien:
        dw.line([((b[1] - x0) * 5, (y1 - b[0]) * 5) for b in l], fill=255, width=3)
    wand = np.asarray(wm) > 0
    lab, n = ndimage.label(~wand)
    for k in range(1, n + 1):
        reg = lab == k
        bekannt = reg & (kl > 0)
        if not bekannt.any():
            kl[reg] = 2
            continue
        _, (ii, jj) = ndimage.distance_transform_edt(~bekannt, return_indices=True)
        luecke = reg & (kl == 0)
        kl[luecke] = kl[ii[luecke], jj[luecke]]
    if (kl == 0).any():
        _, (ii, jj) = ndimage.distance_transform_edt(kl == 0, return_indices=True)
        kl = kl[ii, jj]
    # Mehrheit
    a = ndimage.uniform_filter((kl == 1).astype(float), 7)
    return np.where(a >= 0.5, 1, 2).astype(np.uint8)


def bauen(P, bereich, dop):
    """-> (Mesh, Ursprung, Linien, Oberflaechenpunkte (n x 3), Dreiecke)"""
    from scipy.spatial import Delaunay
    from shapely.geometry import LineString, Point
    from shapely.strtree import STRtree
    from .sonderbau_ring import Mesh
    x0, y0, x1, y1 = bereich
    B = P[P[:, 3] == 2]
    L = wandlinien(B, bereich)
    # Wandreihen: je Messpunkt der Linie ein Punkt oben (zur hoeheren Seite) und einer unten
    pts, art = [], []           # art: ('wand', Linienindex, oben?) oder ('flaeche',)
    for li, l in enumerate(L):
        for ys, x, zu, zo, seite in l:
            pts.append((x + seite * REIHE, ys, zo))
            art.append(('wand', li, True))
            pts.append((x - seite * REIHE, ys, zu))
            art.append(('wand', li, False))
    linien_geo = [LineString([(b[1], b[0]) for b in l]) for l in L]
    baum = STRtree(linien_geo) if linien_geo else None
    for x in np.arange(x0, x1 + 1e-6, RASTER):
        for y in np.arange(y0, y1 + 1e-6, RASTER):
            if baum is not None and any(linien_geo[int(i)].distance(Point(x, y)) < WAND_ABSTAND
                                        for i in baum.query(Point(x, y).buffer(WAND_ABSTAND))):
                continue
            z = boden_z(B, x, y, 0.6)
            if z is None:
                z = boden_z(B, x, y, 1.5)
            if z is None:
                continue
            pts.append((x, y, z))
            art.append(('flaeche',))
    Q = np.array(pts)
    tri = Delaunay(Q[:, :2])
    m = Mesh()
    o = np.array([round((x0 + x1) / 2, 1), round((y0 + y1) / 2, 1), 0.0])
    hoehe_linie = [float(np.median([b[3] - b[2] for b in l])) for l in L]
    unten_linie = [float(np.median([b[2] for b in l])) for l in L]
    ringsohle = min(unten_linie) if unten_linie else -99
    H, W = dop.shape[:2]
    mraster = materialraster(P, bereich, dop, L)
    behalten = []
    for t in tri.simplices:
        A = Q[t]
        kanten = [np.hypot(*(A[i, :2] - A[(i + 1) % 3, :2])) for i in range(3)]
        if max(kanten) > 3.2:
            continue                                    # Rand der Triangulierung / Luecken in den Daten
        n = np.cross(A[1] - A[0], A[2] - A[0])
        if np.linalg.norm(n) < 1e-9:
            continue
        n = n / np.linalg.norm(n)
        if n[2] < 0:
            n = -n
        w = [(p[0] - o[0], p[1] - o[1], p[2]) for p in A]
        wand_li = [art[k][1] for k in t if art[k][0] == 'wand']
        if n[2] < 0.55 and wand_li:
            li = max(set(wand_li), key=wand_li.count)
            trog = unten_linie[li] < ringsohle + 1.0 and hoehe_linie[li] > 3.0
            # zwei Seiten: zur tieferen Seite (Trog) Paneele bzw. Beton, Rueckseite Beton
            seite = L[li][0][4]
            nh = np.array([n[0], n[1], 0.0])
            nh = nh / (np.linalg.norm(nh) or 1)
            if nh[0] * -seite < 0:
                nh = -nh                                # nh zeigt jetzt zur tieferen Seite
            uv = [((A[k, 0] + A[k, 1]) / 2.5, A[k, 2] / 2.5) for k in range(3)]
            m.dreieck('paneel' if trog else 'beton', w, uv, nh)
            m.dreieck('beton', w, uv, -nh)
            continue
        if n[2] < 0.45:
            # sehr steil ohne Wandlinie (die Messung hat die Wand dort nicht als Linie erkannt): Beton, beidseitig
            nh = np.array([n[0], n[1], 0.0])
            nh = nh / (np.linalg.norm(nh) or 1)
            uv = [((A[k, 0] + A[k, 1]) / 2.5, A[k, 2] / 2.5) for k in range(3)]
            m.dreieck('beton', w, uv, nh)
            m.dreieck('beton', w, uv, -nh)
            continue
        c = A.mean(axis=0)
        px, py = int((c[0] - x0) * 5), int((y1 - c[1]) * 5)
        mat = 'asphalt' if mraster[min(max(py, 0), H - 1), min(max(px, 0), W - 1)] == 1 else 'boden'
        m.dreieck(mat, w, [(p[0] / 4, p[1] / 4) for p in A], n)
        behalten.append((t, mat, n))
    # Gelaender auf hohen Waenden (oben)
    for li, l in enumerate(L):
        if hoehe_linie[li] < 2.0:
            continue
        for a, b in zip(l[:-1], l[1:]):
            pa = (a[1] + a[4] * REIHE - o[0], a[0] - o[1], a[3])
            pb = (b[1] + b[4] * REIHE - o[0], b[0] - o[1], b[3])
            q = [pa, pb, (pb[0], pb[1], pb[2] + GELAENDER), (pa[0], pa[1], pa[2] + GELAENDER)]
            d = math.hypot(pb[0] - pa[0], pb[1] - pa[1])
            m.quad('stab', q, [(0, 0), (d, 0), (d, 1), (0, 1)], None, beidseitig=True)
    markierungen(m, Q, behalten, dop, bereich, o)
    return m, o, L, Q, behalten


def markierungen(m, Q, behalten, dop, bereich, o):
    """weisse Pixel des Luftbilds auf Asphalt -> Flaechen 2 cm ueber der Oberflaeche"""
    import shapely
    from shapely.geometry import Polygon, box
    from shapely.ops import unary_union
    x0, y0, x1, y1 = bereich
    v = dop.mean(axis=2)
    sat = dop.max(axis=2) - dop.min(axis=2)
    weiss = (v > np.percentile(v, 97)) & (sat < 40)
    ys_, xs_ = np.nonzero(weiss)
    if not len(xs_):
        return
    kaest = [box(x0 + i * 0.2, y1 - (j + 1) * 0.2, x0 + (i + 1) * 0.2, y1 - j * 0.2) for j, i in zip(ys_, xs_)]
    # Pixeltreppen glaetten: schliessen, oeffnen, vereinfachen (DOP20: 0,2 m je Pixel)
    flaeche = unary_union(kaest).buffer(0.12).buffer(-0.16).buffer(0.04).simplify(0.12)
    asphalt = unary_union([Polygon(Q[t][:, :2]) for t, mat, _ in behalten if mat == 'asphalt'])
    flaeche = flaeche.intersection(asphalt.buffer(-0.05))
    ebene = {}
    import shapely.geometry as sg
    from shapely.strtree import STRtree
    polys = [Polygon(Q[t][:, :2]) for t, mat, _ in behalten]
    baum = STRtree(polys)

    def z_bei(x, y):
        pt = sg.Point(x, y)
        for i in baum.query(pt):
            i = int(i)
            if polys[i].buffer(1e-6).contains(pt):
                (ax, ay, az), (bx, by, bz), (cx, cy, cz) = Q[behalten[i][0]]
                det = (bx - ax) * (cy - ay) - (cx - ax) * (by - ay)
                if abs(det) < 1e-12:
                    continue
                u = ((x - ax) * (cy - ay) - (cx - ax) * (y - ay)) / det
                w = ((bx - ax) * (y - ay) - (x - ax) * (by - ay)) / det
                return az + u * (bz - az) + w * (cz - az)
        return None
    for g in getattr(flaeche, 'geoms', [flaeche]):
        if g.is_empty or g.area < 0.08:
            continue
        for tr in shapely.constrained_delaunay_triangles(g).geoms:
            c = list(tr.exterior.coords)[:3]
            zs = [z_bei(x, y) for x, y in c]
            if any(z is None for z in zs):
                continue
            m.dreieck('weiss', [(x - o[0], y - o[1], z + 0.02) for (x, y), z in zip(c, zs)], [(0, 0), (1, 0), (1, 1)], (0, 0, 1))


def gelaende_absenken(Q, bereich, tief=0.6):
    """Kachelgelaende (5-m-Raster) im Modell unter die Modelloberflaeche -> [[kx, ky, spalte, zeile, z]]"""
    x0, y0, x1, y1 = bereich
    out = []
    for kx in range(math.floor(x0 / 300), math.floor(x1 / 300) + 1):
        for ky in range(math.floor(y0 / 300), math.floor(y1 / 300) + 1):
            for zeile in range(61):
                for spalte in range(61):
                    x, y = kx * 300 + spalte * 5.0, ky * 300 + zeile * 5.0
                    if not (x0 + 2 <= x <= x1 - 2 and y0 + 2 <= y <= y1 - 2):
                        continue
                    m = (np.abs(Q[:, 0] - x) < 5.5) & (np.abs(Q[:, 1] - y) < 5.5)
                    if m.any():
                        out.append([kx, ky, spalte, zeile, round(float(Q[m, 2].min()) - tief, 3)])
    return out


def ringspline(L, bereich):
    """unsichtbarer Spline auf der Ringsohle (Mitte zwischen den beiden Trogwaenden)"""
    from .ring_waende import kette
    unten = sorted(range(len(L)), key=lambda i: np.median([b[2] for b in L[i]]))[:2]
    if len(unten) < 2:
        return []
    a, b = (dict((round(p[0], 1), p) for p in L[i]) for i in unten)
    ys = sorted(set(a) & set(b))
    P = [((a[y][1] + b[y][1]) / 2, y) for y in ys]
    Z = [min(a[y][2], b[y][2]) for y in ys]
    P, Z = P[::-1], Z[::-1]                            # Nord -> Sued
    return [dict(name='Ring_Suedportal', art='weg', sli='Splines\\Marcel\\invis_street.sli', elemente=kette(np.array(P), np.array(Z), knoten=5.0))]


def main():
    from .sonderbau_ring import texturen
    from .sonderbau_ring import Mesh  # noqa: F401
    from .kreuzung import x_file
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--omsi', default=r'C:\Program Files (x86)\Steam\steamapps\common\OMSI 2')
    ap.add_argument('--karte', default='Aburg_Lernstueck')
    ap.add_argument('--laz', default=os.path.join('build', 'vermessung', '511_5535.laz'))
    ap.add_argument('--bereich', type=float, nargs=4, default=[1062.0, -378.0, 1105.0, -281.0])
    ap.add_argument('--cache', default=os.path.join(os.environ.get('LOCALAPPDATA', '.'), 'omsi-editor', 'geodaten', 'dop20'))
    a = ap.parse_args()
    bereich = a.bereich
    P = lese_punkte(a.laz, [bereich[0] - 2, bereich[1] - 2, bereich[2] + 2, bereich[3] + 2], os.path.join('build', 'vermessung', 'suedportal.npy'))
    dop = dop_bild(bereich, a.cache)
    m, o, L, Q, _ = bauen(P, bereich, dop)
    name = 'Suedportal'
    ordner = os.path.join(a.omsi, 'Sceneryobjects', 'Aschaffenburg', a.karte, 'Lernstueck')
    rel = f'Sceneryobjects\\Aschaffenburg\\{a.karte}\\Lernstueck'
    os.makedirs(os.path.join(ordner, 'model'), exist_ok=True)
    texturen(a.omsi, os.path.join(ordner, 'texture'))
    with open(os.path.join(ordner, 'model', name + '.x'), 'w', newline='') as f:
        f.write(x_file(m.V, m.F, [(k, TEX[k]) for k in m.mats]))
    sco = ['Erzeugt mit omsigen (portal_modell) aus Laserdaten und DOP20, Bayerische Vermessungsverwaltung (CC BY 4.0).', '',
           '[friendlyname]', 'Lernstueck Suedportal', '', '[groups]', '1', 'Aschaffenburg', '', '[fixed]', '', '[absheight]', '',
           '[mesh]', name + '.x', '']
    for k in m.mats:
        if TEX[k].endswith('.tga'):
            sco += ['[matl]', TEX[k], '0', '[matl_alpha]', '2', '']
    open(os.path.join(ordner, name + '.sco'), 'w', encoding='cp1252', newline='').write('\r\n'.join(sco) + '\r\n')
    out = dict(karte=a.karte, objekte=[dict(name=name, rel=rel + '\\' + name + '.sco', x=float(o[0]), y=float(o[1]), z=0.0)],
               strassen=ringspline(L, bereich), kreuzungen=[], gelaende=gelaende_absenken(Q, bereich), gelaende_zurueck=[],
               linien=[[list(map(float, b[:4])) for b in l] for l in L], quellen=['Laserdaten, DOP20 Bayern (CC BY 4.0)'])
    json.dump(out, open(os.path.join(ordner, 'lernstueck.json'), 'w', encoding='utf-8'), indent=1)
    print(f'{name}: {len(m.F)} Dreiecke, {len(L)} Wandlinien, Materialien {m.mats}, {len(out["gelaende"])} Gelaendepunkte')


if __name__ == '__main__':
    main()
