"""Gebaeude aus den 3D-Gebaeudemodellen LoD2 der Bayerischen Vermessungsverwaltung (Open Data, CC BY 4.0,
"Bayerische Vermessungsverwaltung - www.geodaten.bayern.de") als OMSI-Objekte.

LoD2: Grundrisse aus dem Kataster, Daecher (13 Standardformen) aus den Laserdaten, Hoehen typisch auf 20-30 cm, ohne
Texturen. Je Gebaeude ein Objekt (.sco + .x) mit absoluter Hoehe; Waende reichen 1 m unter den Boden (kein Spalt am
Gelaende). Fassaden und Daecher bekommen erzeugte Texturen nach Gebaeudefunktion und Dachneigung.

  laden:  km-Kacheln (2 km) im Zwischenspeicher %LOCALAPPDATA%\\omsi-editor\\geodaten\\lod2; fehlende nur mit --laden
  bauen:  python -m omsigen.lod2 --karte Aburg_Newgen_testring --kacheln 3 -2 3 -1   (OMSI-Kacheln kx ky ...)
          oder --mitte x y --radius r (alle Kacheln, die den Kreis beruehren)
          -> je OMSI-Kachel Sceneryobjects/Aschaffenburg/<Karte>/LoD2/<kx>_<ky>/ mit Objekten, Texturen und lod2.json
          (Sonderbau-Format, Kennung LoD2/<kx>_<ky>/: erneutes Einsetzen ersetzt nur die LoD2-Gebaeude dieser Kachel)

Bruecken (53001), Mauern (51007/51009_1700), Staumauern/Wehre gehoeren nicht dazu: die baut OMSIgen selbst."""
import argparse
import json
import math
import os
import sys
import xml.etree.ElementTree as ET

import numpy as np

GEODATEN = os.path.join(os.environ.get('LOCALAPPDATA', '.'), 'omsi-editor', 'geodaten', 'lod2')
URL = 'https://download1.bayernwolke.de/a/lod2/citygml/{e}_{n}.gml'
STANDARD_OMSI = r'C:\Program Files (x86)\Steam\steamapps\common\OMSI 2'
QUELLE = 'Bayerische Vermessungsverwaltung - www.geodaten.bayern.de (CC BY 4.0), LoD2-Gebaeudemodelle, trianguliert und texturiert'

NS = {'gml': 'http://www.opengis.net/gml', 'bldg': 'http://www.opengis.net/citygml/building/1.0',
      'gen': 'http://www.opengis.net/citygml/generics/1.0', 'core': 'http://www.opengis.net/citygml/1.0'}
WEG = ('53001', '51007', '51009_1700', '53009')          # Bruecken, Mauern, Staumauern/Wehre
UNTER_BODEN = 1.0
STOCKWERK = 3.0


# ------------------------------------------------------------------ Lesen
def kacheln(bez, bereich):
    """2-km-Kacheln (gerade km-Werte) fuer den Kartenausschnitt (x0, y0, x1, y1)"""
    x0, y0, x1, y1 = bereich
    e0, e1 = int((bez['ost0'] + x0) // 2000 * 2), int((bez['ost0'] + x1) // 2000 * 2)
    n0, n1 = int((bez['nord0'] + y0) // 2000 * 2), int((bez['nord0'] + y1) // 2000 * 2)
    return [(e, n) for e in range(e0, e1 + 1, 2) for n in range(n0, n1 + 1, 2)]


def datei(e, n):
    return os.path.join(GEODATEN, f'{e}_{n}.gml')


def laden(e, n):
    import requests
    os.makedirs(GEODATEN, exist_ok=True)
    ziel = datei(e, n)
    with requests.get(URL.format(e=e, n=n), stream=True, timeout=120) as r:
        r.raise_for_status()
        with open(ziel + '.teil', 'wb') as f:
            for b in r.iter_content(1 << 20):
                f.write(b)
    os.replace(ziel + '.teil', ziel)
    return ziel


def gebaeude(pfad, bez, bereich=None):
    """CityGML -> Liste dict(id, funktion, dach, hoehe, flaechen=[(art, [Ring n x 3, ...])]) in Kartenkoordinaten"""
    e0, n0, z0 = bez['ost0'], bez['nord0'], bez['nn0']
    tag = lambda p, t: f'{{{NS[p]}}}{t}'
    for _, el in ET.iterparse(pfad, events=('end',)):
        if el.tag != tag('bldg', 'Building'):
            continue
        g = dict(id=el.get(tag('gml', 'id')), funktion=el.findtext('bldg:function', '', NS), dach=el.findtext('bldg:roofType', '', NS),
                 hoehe=float(el.findtext('bldg:measuredHeight', '0', NS) or 0), flaechen=[])
        for art in ('WallSurface', 'RoofSurface', 'GroundSurface'):
            for fl in el.iter(tag('bldg', art)):
                for poly in fl.iter(tag('gml', 'Polygon')):
                    ringe = []
                    for pl in poly.iter(tag('gml', 'posList')):
                        v = np.array(pl.text.split(), float).reshape(-1, 3)
                        v[:, 0] -= e0
                        v[:, 1] -= n0
                        v[:, 2] -= z0
                        ringe.append(v[:-1] if np.allclose(v[0], v[-1]) else v)
                    if ringe:
                        g['flaechen'].append((art, ringe))
        el.clear()
        if not g['flaechen'] or g['funktion'].startswith(WEG):
            continue
        P = np.vstack([r for _, rs in g['flaechen'] for r in rs])
        g['mitte'] = P[:, :2].mean(axis=0)
        g['boden'] = float(P[:, 2].min())
        if bereich is not None:
            if callable(bereich):
                if not bereich(g['mitte']):
                    continue
            else:
                x0, y0, x1, y1 = bereich
                if not (x0 <= g['mitte'][0] <= x1 and y0 <= g['mitte'][1] <= y1):
                    continue
        yield g


# ------------------------------------------------------------------ Dreiecke
def normale(r):
    """Newell: Normale eines (fast) ebenen Rings, Laenge = doppelte Flaeche"""
    n = np.zeros(3)
    for a, b in zip(r, np.roll(r, -1, axis=0)):
        n += [(a[1] - b[1]) * (a[2] + b[2]), (a[2] - b[2]) * (a[0] + b[0]), (a[0] - b[0]) * (a[1] + b[1])]
    return n


def dreiecke(ringe):
    """ebenes Vieleck mit Loechern (3D) -> Dreiecke [(p0, p1, p2)] und Normale (aussen, aus dem Umlaufsinn)"""
    import shapely
    from shapely.geometry import Polygon
    n = normale(ringe[0])
    if np.linalg.norm(n) < 1e-6:
        return [], n
    n = n / np.linalg.norm(n)
    achse = int(np.argmax(np.abs(n)))
    ij = [k for k in range(3) if k != achse]
    if len(ringe[0]) == 3 and len(ringe) == 1:
        return [tuple(ringe[0])], n
    zu3 = {}
    for r in ringe:
        for p in r:
            zu3[(round(p[ij[0]], 4), round(p[ij[1]], 4))] = p
    try:
        poly = Polygon(ringe[0][:, ij], [r[:, ij] for r in ringe[1:]])
        if not poly.is_valid:
            poly = poly.buffer(0)
        tris = shapely.constrained_delaunay_triangles(poly)
    except Exception:
        return [], n
    out = []
    for t in getattr(tris, 'geoms', []):
        q = list(t.exterior.coords)[:3]
        pts = []
        for x, y in q:
            p = zu3.get((round(x, 4), round(y, 4)))
            if p is None:                                  # neuer Punkt (buffer): auf die Ebene legen
                p = np.zeros(3)
                p[ij[0]], p[ij[1]] = x, y
                r0 = ringe[0][0]
                p[achse] = r0[achse] - (n[ij[0]] * (x - r0[ij[0]]) + n[ij[1]] * (y - r0[ij[1]])) / n[achse]
            pts.append(p)
        out.append(tuple(pts))
    return out, n


# ------------------------------------------------------------------ Material
def wand_material(funktion):
    f = funktion.split('_')[-1] if funktion else ''
    if funktion.startswith('51009_1610'):
        return 'beton'                                    # Ueberdachung
    if f in ('2463', '2523'):
        return 'putz'                                     # Garage, Umformer: ohne Fenster
    if f in ('3041', '3042', '3043', '3031', '3038', '3048'):
        return 'sandstein'                                # Kirchen, Schloss, Burg, Kloster: Aschaffenburger Sandstein
    if f.startswith('1') or f in ('2072', '3052', '3242', '3065'):
        return 'wohnen'
    return 'gewerbe'


def bauen_gebaeude(g):
    """Gebaeude -> Mesh (Objektursprung: Mitte des Grundrisses, Hoehe absolut)"""
    from .sonderbau_ring import Mesh
    m = Mesh()
    o = np.array([g['mitte'][0], g['mitte'][1], 0.0])
    boden = g['boden']
    wand = wand_material(g['funktion'])
    for art, ringe in g['flaechen']:
        if art == 'GroundSurface':
            continue
        if art == 'WallSurface':
            # Waende 1 m unter den Boden (Gelaende der Karte weicht ab)
            ringe = [np.where((np.abs(r[:, 2:3] - boden) < 0.05), r - [0, 0, UNTER_BODEN], r) for r in ringe]
        tris, n = dreiecke(ringe)
        if not tris:
            continue
        steil = abs(n[2]) < 0.2
        if art == 'WallSurface' or steil:
            mat = wand
            t = np.array([-n[1], n[0], 0.0])
            t = t / (np.linalg.norm(t) or 1.0)
            uvf = lambda p: (float(np.dot(p[:2], t[:2])) / STOCKWERK, (float(p[2]) - boden) / STOCKWERK)
        else:
            mat = 'dach_flach' if abs(n[2]) > 0.97 else 'dach_ziegel'
            uvf = lambda p: (float(p[0]) / 2.5, float(p[1]) / 2.5)
        for tri in tris:
            pts = [tuple(np.asarray(p) - o) for p in tri]
            m.dreieck(mat, pts, [uvf(np.asarray(p)) for p in tri], tuple(n))
    return m, o


TEX = {'wohnen': 'LOD2_fassade_wohnen.bmp', 'gewerbe': 'LOD2_fassade_gewerbe.bmp', 'putz': 'LOD2_putz.bmp',
       'sandstein': 'LOD2_sandstein.bmp', 'beton': 'LOD2_beton.bmp', 'dach_ziegel': 'LOD2_dach_ziegel.bmp',
       'dach_flach': 'LOD2_dach_flach.bmp'}


def texturen(ziel):
    """Fassaden je 3 m x 3 m (ein Stockwerk, ein Fenster), Dach je 2,5 m"""
    from PIL import Image, ImageDraw
    os.makedirs(ziel, exist_ok=True)
    rng = np.random.default_rng(3)

    def rauschen(farbe, s=256, st=6):
        return Image.fromarray((rng.normal(0, st, (s, s, 3)) + farbe).clip(0, 255).astype(np.uint8))
    for name, farbe, fenster in (('wohnen', (222, 210, 186), (86, 64, 170, 196)), ('gewerbe', (196, 196, 192), (30, 84, 226, 196))):
        im = rauschen(farbe)
        d = ImageDraw.Draw(im)
        x0, y0, x1, y1 = fenster
        d.rectangle([x0 - 6, 256 - y1 - 6, x1 + 6, 256 - y0 + 6], fill=tuple(int(c * 0.82) for c in farbe))   # Laibung
        d.rectangle([x0, 256 - y1, x1, 256 - y0], fill=(52, 62, 72))                                         # Glas
        d.line([(x0 + x1) // 2, 256 - y1, (x0 + x1) // 2, 256 - y0], fill=(235, 235, 230), width=4)          # Kreuz
        d.line([x0, 256 - (y0 + y1) // 2, x1, 256 - (y0 + y1) // 2], fill=(235, 235, 230), width=4)
        im.save(os.path.join(ziel, TEX[name]))
    rauschen((214, 206, 192)).save(os.path.join(ziel, TEX['putz']))
    rauschen((168, 150, 140), st=4).save(os.path.join(ziel, TEX['beton']))
    sa = np.array(rauschen((150, 82, 66), st=10))                                                          # roter Sandstein
    for y in range(0, 256, 43):
        sa[y:y + 3, :] = (200, 180, 160)
    Image.fromarray(sa).save(os.path.join(ziel, TEX['sandstein']))
    zi = np.array(rauschen((150, 62, 44), st=12))                                                          # Ziegel
    for y in range(0, 256, 21):
        zi[y:y + 4, :] = (95, 36, 26)
    Image.fromarray(zi).save(os.path.join(ziel, TEX['dach_ziegel']))
    rauschen((128, 128, 124), st=14).save(os.path.join(ziel, TEX['dach_flach']))


def sco_text(name, mats):
    L = [f'Erzeugt mit OMSIgen (omsigen.lod2). Quelle: {QUELLE}.', '', '[friendlyname]', f'LoD2 {name}', '',
         '[groups]', '1', 'Aschaffenburg', '', '[fixed]', '', '[absheight]', '', '[mesh]', name + '.x', '']
    return '\r\n'.join(L) + '\r\n'


def kacheln_im_kreis(mitte, radius):
    """OMSI-Kacheln (300 m), die den Kreis beruehren"""
    cx, cy = mitte
    out = []
    for kx in range(math.floor((cx - radius) / 300), math.floor((cx + radius) / 300) + 1):
        for ky in range(math.floor((cy - radius) / 300), math.floor((cy + radius) / 300) + 1):
            nx, ny = min(max(cx, kx * 300), kx * 300 + 300), min(max(cy, ky * 300), ky * 300 + 300)
            if math.hypot(nx - cx, ny - cy) <= radius:
                out.append((kx, ky))
    return out


def bauen(omsi, karte, omsi_kacheln, laden_erlaubt=False, pruefen=True):
    """LoD2-Gebaeude je OMSI-Kachel (Gebaeudemitte in der Kachel): Objekte in LoD2/<kx>_<ky>/ und je Kachel ein
    lod2.json mit Kennung LoD2/<kx>_<ky>/ im Pfad - erneutes Einsetzen ersetzt nur die Gebaeude dieser Kachel
    -> (Liste json-Pfade, Gebaeude, Dreiecke)"""
    bs = '\\'
    from .aufmass import bezug_lesen
    from .kreuzung import x_file
    bez = bezug_lesen(omsi, karte)
    kdir = os.path.join(omsi, 'maps', karte)
    # der Editor gibt nur Kacheln seiner Sitzung (auch neue, noch nicht gespeicherte) und prueft selbst
    if pruefen:
        omsi_kacheln = [k for k in omsi_kacheln if os.path.isfile(os.path.join(kdir, f'tile_{k[0]}_{k[1]}.map'))]
    if not omsi_kacheln:
        raise RuntimeError('keine dieser Kacheln gibt es in der Karte')
    noetig = sorted({k for kx, ky in omsi_kacheln for k in kacheln(bez, (kx * 300, ky * 300, kx * 300 + 299.9, ky * 300 + 299.9))})
    fehlen = [(e, n) for e, n in noetig if not os.path.isfile(datei(e, n))]
    if fehlen and not laden_erlaubt:
        raise RuntimeError('LoD2-Kacheln fehlen: ' + ', '.join(f'{e}_{n}' for e, n in fehlen) + ' (--laden)')
    for e, n in fehlen:
        laden(e, n)
    basis = os.path.join(omsi, 'Sceneryobjects', 'Aschaffenburg', karte, 'LoD2')
    rel0 = bs.join(['Sceneryobjects', 'Aschaffenburg', karte, 'LoD2'])
    fertig_tex = set()
    gewollt = set(map(tuple, omsi_kacheln))
    je = {k: [] for k in gewollt}
    x0 = min(k[0] for k in gewollt) * 300
    y0 = min(k[1] for k in gewollt) * 300
    x1 = (max(k[0] for k in gewollt) + 1) * 300
    y1 = (max(k[1] for k in gewollt) + 1) * 300
    in_kachel = lambda p: (math.floor(p[0] / 300), math.floor(p[1] / 300)) in gewollt
    n_dr = 0
    for e, n in noetig:
        for g in gebaeude(datei(e, n), bez, lambda p: x0 <= p[0] < x1 and y0 <= p[1] < y1 and in_kachel(p)):
            k = (math.floor(g['mitte'][0] / 300), math.floor(g['mitte'][1] / 300))
            m, o = bauen_gebaeude(g)
            if m.leer():
                continue
            ordner = os.path.join(basis, f'{k[0]}_{k[1]}')
            os.makedirs(os.path.join(ordner, 'model'), exist_ok=True)
            if k not in fertig_tex:                    # Texturen je Kachelordner (OMSI sucht sie neben dem Objekt)
                texturen(os.path.join(ordner, 'texture'))
                fertig_tex.add(k)
            name = g['id'].replace('DEBY_LOD2_', 'B')
            with open(os.path.join(ordner, 'model', name + '.x'), 'w', newline='') as f:
                f.write(x_file(m.V, m.F, [(kk, TEX[kk]) for kk in m.mats]))
            open(os.path.join(ordner, name + '.sco'), 'w', encoding='cp1252', newline='').write(sco_text(name, m.mats))
            je[k].append(dict(name=name, rel=bs.join([rel0, f'{k[0]}_{k[1]}', name + '.sco']), x=round(float(o[0]), 3), y=round(float(o[1]), 3),
                              z=0.0, funktion=g['funktion'], dach=g['dach'], hoehe=g['hoehe']))
            n_dr += len(m.F)
    pfade = []
    for (kx, ky), objekte in sorted(je.items()):
        ordner = os.path.join(basis, f'{kx}_{ky}')
        os.makedirs(ordner, exist_ok=True)
        out = dict(karte=karte, marker=bs + 'LoD2' + bs + f'{kx}_{ky}' + bs, kachel=[kx, ky], objekte=objekte, strassen=[], kreuzungen=[],
                   gelaende=[], gelaende_zurueck=[], quellen=[QUELLE])
        pfad = os.path.join(ordner, 'lod2.json')
        json.dump(out, open(pfad, 'w', encoding='utf-8'), indent=1, ensure_ascii=False)
        pfade.append(pfad)
    return pfade, sum(len(v) for v in je.values()), n_dr


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--omsi', default=STANDARD_OMSI)
    ap.add_argument('--karte', required=True)
    ap.add_argument('--kacheln', type=int, nargs='+', help='OMSI-Kacheln kx ky kx ky ...')
    ap.add_argument('--mitte', type=float, nargs=2, help='statt --kacheln: alle Kacheln, die den Kreis beruehren')
    ap.add_argument('--radius', type=float, default=300.0)
    ap.add_argument('--laden', action='store_true')
    ap.add_argument('--ohne-pruefung', action='store_true', help='Kacheln nicht gegen die gespeicherte Karte pruefen (Editor-Sitzung)')
    a = ap.parse_args()
    try:
        if a.kacheln:
            ks = list(zip(a.kacheln[0::2], a.kacheln[1::2]))
        elif a.mitte:
            ks = kacheln_im_kreis(a.mitte, a.radius)
        else:
            raise ValueError('--kacheln oder --mitte angeben')
        pfade, n, d = bauen(a.omsi, a.karte, ks, a.laden, not a.ohne_pruefung)
        erg = dict(jsons=pfade, gebaeude=n, dreiecke=d, kacheln=[list(k) for k in ks])
    except Exception as e:
        erg = dict(fehler=f'{type(e).__name__}: {e}')
    print(json.dumps(erg, ensure_ascii=False))


if __name__ == '__main__':
    main()
