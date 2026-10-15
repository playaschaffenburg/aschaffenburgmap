"""Querschnitte gegen das Luftbild pruefen (Qualitaetsplan S1, Abnahme): an festen Pruefstellen die Fahrbahn- und
Gehwegbreite der Karte (netz.json) mit der im Luftbild DOP20 (Bayerische Vermessungsverwaltung, CC BY 4.0) abgelesenen
vergleichen.

Pruefstellen: Kartendaten `querschnitte/pruefstellen.json` (unabhaengig von der Karte, in UTM):
  [{"name", "a": [Ost, Nord], "b": [Ost, Nord],          Querlinie ueber die Strasse (von a nach b)
    "bord": [s_links, s_rechts],                          Bordkanten (Fahrbahnrand) als Abstand von a in m
    "aussen": [s_links, s_rechts] (optional),             Hinterkante der Gehwege
    "quelle": "DOP20 abgelesen ..."}]
Die Werte liest ein Mensch (oder Claude) im Bild `--ablesen` ab: Luftbild entlang der Querlinie mit Meterteilung und
den Kanten, die das Helligkeitsprofil vorschlaegt. Die Messung des Generators (querschnitt_messung) fliesst hier nicht
ein - die Pruefstellen bleiben ein unabhaengiger Massstab.

  python -m omsigen.querschnitt_pruefung <Karte> [--bilder]          Abweichungen je Pruefstelle (+ Bilder)
  python -m omsigen.querschnitt_pruefung <Karte> --ausschnitt x0 z0 x1 z1 --png a.png   Netz ueber dem Luftbild
  python -m omsigen.querschnitt_pruefung --ablesen <Ost> <Nord> <Richtung> [--laenge 40] --png a.png
"""
import argparse
import json
import math
import os

import numpy as np

from .config import KARTENDATEN

PRUEFSTELLEN = os.path.join(KARTENDATEN, 'querschnitte', 'pruefstellen.json')
DOP_CACHE = os.path.join(os.environ.get('LOCALAPPDATA', '.'), 'omsi-editor', 'geodaten', 'dop20')
GRENZE = 0.5             # m: Abnahme S1
STANDARD_OMSI = r'C:\Program Files (x86)\Steam\steamapps\common\OMSI 2'


def dop(e0, n0, e1, n1, m_px=0.1):
    """Luftbild-Ausschnitt (UTM) -> PIL-Bild (Zeile 0 = Norden)"""
    from PIL import Image
    from .luftbildvergleich import dop20
    w, h = int(round((e1 - e0) / m_px)), int(round((n1 - n0) / m_px))
    return Image.open(dop20(round(e0, 1), round(n0, 1), round(e1, 1), round(n1, 1), w, h, DOP_CACHE)).convert('RGB')


def bezug(kartenordner):
    from .fotovergleich import bezug_lesen
    return bezug_lesen(kartenordner)


# ---------------------------------------------------------------- Netz ueber dem Luftbild

def ausschnitt_bild(kartenordner, x0, z0, x1, z1, png, m_px=0.1, omsi=None, gebaeude=True, linien=()):
    """Netz der Karte (Fahrbahnkanten rot, Aussenkanten blau, Kreuzungsplatten lila, Gebaeude gelb) ueber dem Luftbild"""
    from PIL import ImageDraw
    from .netz_datei import Netz
    b = bezug(kartenordner)
    img = dop(b['ost0'] + x0, b['nord0'] + z0, b['ost0'] + x1, b['nord0'] + z1, m_px)
    dr = ImageDraw.Draw(img, 'RGBA')

    def px(x, z):
        return ((x - x0) / m_px, (z1 - z) / m_px)
    n = Netz.laden(kartenordner, pruefen=False)
    for e in n.kanten.values():
        q = e.get('querschnitt')
        if not q or e['art'] == 'unsichtbar':
            continue
        P = e['punkte']
        if not any(x0 - 30 < p[0] < x1 + 30 and z0 - 30 < p[2] < z1 + 30 for p in P):
            continue
        fb = [px(*p) for p in n.streifen(e['id'], q['fahrbahn'][0], q['fahrbahn'][1])]
        dr.polygon(fb, fill=(255, 40, 40, 45))
        for s, farbe in ((q['fahrbahn'][0], (255, 40, 40, 255)), (q['fahrbahn'][1], (255, 40, 40, 255)),
                         (q['aussen'][0], (60, 120, 255, 255)), (q['aussen'][1], (60, 120, 255, 255))):
            if s in q['fahrbahn'] and farbe[2] == 255:
                continue                          # Aussenkante = Fahrbahnkante (ohne Gehweg)
            L = [px(*n.punkt(e['id'], p[3], s)[::2][:2]) for p in P]
            dr.line(L, fill=farbe, width=2)
    if gebaeude or True:
        from . import belegung as B, schichten
        omsi = omsi or STANDARD_OMSI
        for rel, ox, oz, rot, _ in B._objekte(kartenordner, lambda r: os.path.basename(r.replace(chr(92), '/')).upper().startswith('K_')):
            if not (x0 - 60 < ox < x1 + 60 and z0 - 60 < oz < z1 + 60):
                continue
            pfad = B._modellpfad(omsi, rel)
            if not pfad or not os.path.isfile(pfad):
                continue
            P, F, mat, tex = B.x_lesen(pfad)
            x, z = B._drehen(P[:, 0], P[:, 2], rot)
            for f, m in zip(F, mat):
                if m < len(tex) and 'side' not in tex[m].lower():
                    dr.polygon([px(x[i] + ox, z[i] + oz) for i in f], fill=(150, 60, 200, 50))
        if gebaeude:
            for rel, ox, oz, rot, _ in B._objekte(kartenordner, lambda r: schichten.schicht_von(r) is not None and schichten.schicht_von(r)[0] == 'gebaeude'):
                if not (x0 - 80 < ox < x1 + 80 and z0 - 80 < oz < z1 + 80):
                    continue
                pfad = B._modellpfad(omsi, rel)
                if not pfad or not os.path.isfile(pfad):
                    continue
                from shapely.geometry import Polygon
                from shapely.ops import unary_union
                P, F, _, _ = B.x_lesen(pfad)
                x, z = B._drehen(P[:, 0], P[:, 2], rot)
                tris = [Polygon([(x[i] + ox, z[i] + oz) for i in f]) for f in F]
                g = unary_union([t.buffer(0.01) for t in tris if t.area > 1e-3])
                for poly in getattr(g, 'geoms', [g]):
                    if poly.geom_type == 'Polygon':
                        dr.line([px(*c) for c in poly.exterior.coords], fill=(255, 220, 0, 230), width=2)
    for (xa, za), (xb, zb), farbe in linien:
        dr.line([px(xa, za), px(xb, zb)], fill=farbe, width=2)
    img.save(png)
    return png
