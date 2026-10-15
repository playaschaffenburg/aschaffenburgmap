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
  Fassaden: liegt fuer ein Gebaeude eine Fassade aus earth-reverse-engineering (tools/lod2/korridor.py) in
          Kartendaten/fassaden/gebaeude/<gml-id>/, wird deren Netz mit Texturatlas benutzt (--fassaden, --ohne-fassaden)

Bruecken (53001), Mauern (51007/51009_1700), Staumauern/Wehre gehoeren nicht dazu: die baut OMSIgen selbst."""
import argparse
import json
import math
import os
import shutil
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


# Fassaden aus earth-reverse-engineering/tools/lod2/korridor.py (je Gebaeude ein Netz in UTM 32 + NHN mit Texturatlas,
# aus Google-Earth-Fotos abgeleitet - nur private Nutzung): gibt es eine, ersetzt sie die erzeugten Texturen
def fassaden_ordner():
    from .config import KARTENDATEN
    return os.path.join(KARTENDATEN, 'fassaden', 'gebaeude')


QUELLE_FASSADE = 'Fassaden aus Google-Earth-Fotos abgeleitet (nur private Nutzung)'


def dds_bc1(quelle, ziel):
    """Bild -> DDS mit BC1 (DXT1) und allen Mip-Stufen: 1/8 des Grafikspeichers von RGBA, und openOMSI/OMSI laden
    es ohne eigenes Komprimieren (1400 Fassaden als JPG: ~5 GB unkomprimiert, der Editor hing beim ersten Bild)"""
    import io
    import struct
    from PIL import Image
    img = Image.open(quelle).convert('RGB')
    w, h = img.size
    stufen = []
    while True:
        lw, lh = stufen and (max(1, stufen[-1][0] // 2), max(1, stufen[-1][1] // 2)) or (w, h)
        stufen.append((lw, lh))
        if lw == 1 and lh == 1:
            break
    daten = b''
    for lw, lh in stufen:
        stufe = img.resize((lw, lh), Image.LANCZOS) if (lw, lh) != (w, h) else img
        if lw < 4 or lh < 4:                         # ein Block: auf 4x4 gestreckt kodieren
            stufe = stufe.resize((4, 4), Image.NEAREST)
        b = io.BytesIO()
        stufe.save(b, format='DDS', pixel_format='DXT1')
        daten += b.getvalue()[128:]                  # nur die Bloecke (Kopf: 128 Byte)
    # Kopf 128 Byte: Kennung, 7 Felder, 11 reserviert, Pixelformat (Groesse, FOURCC, 'DXT1', 5 Masken), Caps 1-4, reserviert
    kopf = struct.pack('<4s7I44x2I4s5I5I', b'DDS ', 124, 0x1 | 0x2 | 0x4 | 0x1000 | 0x20000 | 0x80000,
                       h, w, max(1, (w + 3) // 4) * max(1, (h + 3) // 4) * 8, 0, len(stufen),
                       32, 0x4, b'DXT1', 0, 0, 0, 0, 0, 0x1000 | 0x400000 | 0x8, 0, 0, 0, 0)
    assert len(kopf) == 128
    with open(ziel, 'wb') as f:
        f.write(kopf + daten)


def fassade_laden(gid, ordner):
    pfad = os.path.join(ordner, gid, 'gebaeude.json') if ordner else ''
    if not pfad or not os.path.isfile(pfad):
        return None
    f = json.load(open(pfad, encoding='utf-8'))
    f['bild'] = os.path.join(ordner, gid, f['textur'])
    return f if os.path.isfile(f['bild']) else None


def bauen_fassade(g, f, bez, abbild=None):
    """Gebaeude mit Fassade -> Mesh (Objektursprung wie bauen_gebaeude: Mitte des LoD2-Grundrisses, Hoehe absolut).
    abbild(ost, nord) -> (x, y) Kartenkoordinaten (Arrays); ohne: UTM-Karte mit bez ost0/nord0"""
    from .sonderbau_ring import Mesh
    m = Mesh()
    o = np.array([g['mitte'][0], g['mitte'][1], 0.0])
    P = np.asarray(f['punkte'], float)
    E, N = P[:, 0] + f['ursprung'][0], P[:, 1] + f['ursprung'][1]
    X, Y = abbild(E, N) if abbild else (E - bez['ost0'], N - bez['nord0'])
    P = np.c_[X - o[0], Y - o[1], P[:, 2] - bez['nn0']]
    UV = np.asarray(f['uv'], float)
    for a, b, c in f['dreiecke']:
        n = np.cross(P[b] - P[a], P[c] - P[a])
        if np.linalg.norm(n) < 1e-9:
            continue
        # glTF-uv (0,0 oben links) -> Mesh-uv (x_file schreibt -v)
        m.dreieck('fassade', [P[a], P[b], P[c]], [(UV[i, 0], -UV[i, 1]) for i in (a, b, c)], n)
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


def sco_text(name, mats, fassade=False):
    quelle = QUELLE + ('; ' + QUELLE_FASSADE if fassade else '')
    L = [f'Erzeugt mit OMSIgen (omsigen.lod2). Quelle: {quelle}.', '', '[friendlyname]', f'LoD2 {name}', '',
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


def bauen(omsi, karte, omsi_kacheln, laden_erlaubt=False, pruefen=True, fassaden='standard', statistik=None, auswahl=None):
    """LoD2-Gebaeude je OMSI-Kachel (Gebaeudemitte in der Kachel): Objekte in LoD2/<kx>_<ky>/ und je Kachel ein
    lod2.json mit Kennung LoD2/<kx>_<ky>/ im Pfad - erneutes Einsetzen ersetzt nur die Gebaeude dieser Kachel.
    fassaden: Ordner der Fassaden je Gebaeude ('standard' = Kartendaten/fassaden/gebaeude, None = keine);
    statistik (dict) bekommt 'fassaden' = Zahl der Gebaeude mit Fassade; auswahl(mitte) -> bool: nur diese Gebaeude
    (Kartenkoordinaten, z. B. Korridor des Generators). Jedes lod2.json ist ein Schicht-Paket "gebaeude"
    (omsigen.schichten) und steht im Verzeichnis der Karte (schichten.json).
    -> (Liste json-Pfade, Gebaeude, Dreiecke)"""
    from . import schichten
    if fassaden == 'standard':
        fassaden = fassaden_ordner()
    n_fassaden = 0
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
        for g in gebaeude(datei(e, n), bez, lambda p: x0 <= p[0] < x1 and y0 <= p[1] < y1 and in_kachel(p)
                                                     and (auswahl is None or auswahl(p))):
            k = (math.floor(g['mitte'][0] / 300), math.floor(g['mitte'][1] / 300))
            name = g['id'].replace('DEBY_LOD2_', 'B')
            fa = fassade_laden(g['id'], fassaden)
            m, o = bauen_fassade(g, fa, bez) if fa else bauen_gebaeude(g)
            if m.leer():
                continue
            ordner = os.path.join(basis, f'{k[0]}_{k[1]}')
            os.makedirs(os.path.join(ordner, 'model'), exist_ok=True)
            if k not in fertig_tex:                    # Texturen je Kachelordner (OMSI sucht sie neben dem Objekt)
                texturen(os.path.join(ordner, 'texture'))
                fertig_tex.add(k)
            if fa:
                dds_bc1(fa['bild'], os.path.join(ordner, 'texture', name + '.dds'))
                mats = [('fassade', name + '.dds')]
                n_fassaden += 1
            else:
                mats = [(kk, TEX[kk]) for kk in m.mats]
            with open(os.path.join(ordner, 'model', name + '.x'), 'w', newline='') as f:
                f.write(x_file(m.V, m.F, mats))
            open(os.path.join(ordner, name + '.sco'), 'w', encoding='cp1252', newline='').write(sco_text(name, m.mats, bool(fa)))
            je[k].append(dict(name=name, rel=bs.join([rel0, f'{k[0]}_{k[1]}', name + '.sco']), x=round(float(o[0]), 3), y=round(float(o[1]), 3),
                              z=0.0, funktion=g['funktion'], dach=g['dach'], hoehe=g['hoehe'], fassade=bool(fa)))
            n_dr += len(m.F)
    pfade = []
    for (kx, ky), objekte in sorted(je.items()):
        ordner = os.path.join(basis, f'{kx}_{ky}')
        os.makedirs(ordner, exist_ok=True)
        quellen = [QUELLE] + ([QUELLE_FASSADE] if any(o.get('fassade') for o in objekte) else [])
        man = schichten.manifest(kdir, 'gebaeude', (kx, ky), 'omsigen.lod2',
                                 dict(fassaden=bool(fassaden), auswahl=auswahl is not None), quellen)
        out = schichten.paket(karte, 'gebaeude', (kx, ky), objekte, man, quellen)
        pfad = os.path.join(ordner, 'lod2.json')
        json.dump(out, open(pfad, 'w', encoding='utf-8'), indent=1, ensure_ascii=False)
        if os.path.isdir(kdir):
            schichten.vermerken(kdir, pfad, out)
        pfade.append(pfad)
    if statistik is not None:
        statistik['fassaden'] = n_fassaden
    return pfade, sum(len(v) for v in je.values()), n_dr


def projekt_abbild(proj):
    """UTM 32 -> Projektmeter des Generators (route.Projection: ebene Naeherung um lat0/lon0)"""
    from .utm import from_utm

    def abbild(e, n):
        e, n = np.atleast_1d(e).astype(float), np.atleast_1d(n).astype(float)
        if getattr(proj, 'art', '') == 'utm' and proj.zone == 32:
            return e - proj.e0, n - proj.n0          # Projektmeter sind UTM 32: nur verschieben
        ll = [from_utm(a, b) for a, b in zip(e, n)]
        xy = np.array([proj.to_m(lat, lon) for lat, lon in ll])
        return xy[:, 0], xy[:, 1]
    return abbild


def bauen_fuer_projekt(root, name, proj, basis, linie, breite=150.0, fassaden='standard', laden_erlaubt=True,
                       ueberschreiben=False, log=print):
    """LoD2-Gebaeude fuer eine Generator-Karte (omsigen.pipeline): alle Gebaeude, deren Mitte hoechstens `breite` m
    von der Linie (Projektmeter) entfernt ist, als Objekte in Sceneryobjects/Aschaffenburg/<name>/LoD2/ (nach den
    Kreuzungsobjekten aufrufen: install_objects ersetzt den Kartenordner; Fassaden wie in bauen()). Hoehe = NN - basis.
    -> Liste dict(origin=(x, z) Projektmeter, rel=Pfad der .sco, fassade=bool)"""
    from .kreuzung import x_file
    from .utm import to_utm
    if fassaden == 'standard':
        fassaden = fassaden_ordner()
    bs = '\\'
    abbild = projekt_abbild(proj)
    L = np.array([to_utm(*proj.to_ll(x, z)) for x, z in linie])
    bez = dict(ost0=0.0, nord0=0.0, nn0=basis)
    lo, hi = L.min(0) - breite, L.max(0) + breite
    noetig = sorted({(e, n) for e in range(int(lo[0] // 2000 * 2), int(hi[0] // 2000 * 2) + 1, 2)
                     for n in range(int(lo[1] // 2000 * 2), int(hi[1] // 2000 * 2) + 1, 2)})
    a, b = L[:-1], L[1:]
    ab = b - a

    def nah(p):
        t = np.clip(((p - a) * ab).sum(1) / np.maximum((ab * ab).sum(1), 1e-9), 0, 1)
        return np.linalg.norm(a + t[:, None] * ab - p, axis=1).min() <= breite
    ordner = os.path.join(root, 'Sceneryobjects', 'Aschaffenburg', name, 'LoD2')   # im eigenen Objektordner der Karte
    rel0 = bs.join(['Sceneryobjects', 'Aschaffenburg', name, 'LoD2'])            # (sonst fragt der Editor nach "fremden" Objekten)
    if os.path.exists(ordner):
        if not ueberschreiben:
            raise FileExistsError(f'Objektordner {ordner} existiert schon - anderen Kartennamen waehlen')
        shutil.rmtree(ordner)
    os.makedirs(os.path.join(ordner, 'model'))
    texturen(os.path.join(ordner, 'texture'))
    out, n_dr = [], 0
    for e, n in noetig:
        if not os.path.isfile(datei(e, n)):
            if not laden_erlaubt:
                continue
            try:
                laden(e, n)
            except Exception as err:                   # ausserhalb Bayerns gibt es keine Kachel
                log(f'    LoD2-Kachel {e}_{n} nicht verfuegbar: {err}')
                continue
        for g in gebaeude(datei(e, n), bez, lambda p: lo[0] <= p[0] <= hi[0] and lo[1] <= p[1] <= hi[1] and nah(p)):
            gname = g['id'].replace('DEBY_LOD2_', 'B')
            fa = fassade_laden(g['id'], fassaden)
            # LoD2 in Projektmeter: Ringe und Mitte abbilden (die Hoehen bleiben)
            utm_mitte = g['mitte'].copy()
            g['flaechen'] = [(art, [np.c_[np.stack(abbild(r[:, 0], r[:, 1]), 1), r[:, 2]] for r in ringe])
                             for art, ringe in g['flaechen']]
            g['mitte'] = np.array(abbild(utm_mitte[0], utm_mitte[1])).ravel()
            m, o = bauen_fassade(g, fa, bez, abbild) if fa else bauen_gebaeude(g)
            if m.leer():
                continue
            if fa:
                dds_bc1(fa['bild'], os.path.join(ordner, 'texture', gname + '.dds'))
                mats = [('fassade', gname + '.dds')]
            else:
                mats = [(kk, TEX[kk]) for kk in m.mats]
            with open(os.path.join(ordner, 'model', gname + '.x'), 'w', newline='') as f:
                f.write(x_file(m.V, m.F, mats))
            open(os.path.join(ordner, gname + '.sco'), 'w', encoding='cp1252', newline='').write(sco_text(gname, m.mats, bool(fa)))
            out.append(dict(origin=(float(o[0]), float(o[1])), rel=bs.join([rel0, gname + '.sco']), fassade=bool(fa)))
            n_dr += len(m.F)
    log(f'    LoD2: {len(out)} Gebaeude ({sum(o["fassade"] for o in out)} mit Fassade), {n_dr} Dreiecke')
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--omsi', default=STANDARD_OMSI)
    ap.add_argument('--karte', required=True)
    ap.add_argument('--kacheln', type=int, nargs='+', help='OMSI-Kacheln kx ky kx ky ...')
    ap.add_argument('--mitte', type=float, nargs=2, help='statt --kacheln: alle Kacheln, die den Kreis beruehren')
    ap.add_argument('--radius', type=float, default=300.0)
    ap.add_argument('--laden', action='store_true')
    ap.add_argument('--ohne-pruefung', action='store_true', help='Kacheln nicht gegen die gespeicherte Karte pruefen (Editor-Sitzung)')
    ap.add_argument('--fassaden', help='Ordner der Fassaden je Gebaeude (Standard: Kartendaten/fassaden/gebaeude)')
    ap.add_argument('--ohne-fassaden', action='store_true', help='nur erzeugte Texturen, auch wo es Fassaden gibt')
    a = ap.parse_args()
    try:
        if a.kacheln:
            ks = list(zip(a.kacheln[0::2], a.kacheln[1::2]))
        elif a.mitte:
            ks = kacheln_im_kreis(a.mitte, a.radius)
        else:
            raise ValueError('--kacheln oder --mitte angeben')
        st = {}
        pfade, n, d = bauen(a.omsi, a.karte, ks, a.laden, not a.ohne_pruefung,
                            None if a.ohne_fassaden else (a.fassaden or 'standard'), st)
        erg = dict(jsons=pfade, gebaeude=n, dreiecke=d, kacheln=[list(k) for k in ks], fassaden=st.get('fassaden', 0))
    except Exception as e:
        erg = dict(fehler=f'{type(e).__name__}: {e}')
    print(json.dumps(erg, ensure_ascii=False))


if __name__ == '__main__':
    main()
