"""Boden einer Karte (Qualitaetsplan S4): Malmasken je Kachel mit echten Belaegen - Gras, Erde, Schotter, Pflaster,
Asphalt, Wasser - statt ueberall Gras.

OMSI-Format (gemessen an Grundorf, Berlin-Spandau, Ahlheim, Gladbeck, Koeln ...; openOMSI omsi-map/global.rs und
omsi-app/scene.rs load_ground_paint):
  global.cfg  je Belag ein [groundtex]: Textur, Detailtextur, Maskengroesse als Zweierpotenz (9 = 512 x 512 ueber die
              Kachel), Wiederholungen der Textur und der Detailtextur je Kachel. Pfade gelten ab dem OMSI-Ordner. Der
              erste Eintrag ist der Grund, mit dem jede Kachel beginnt (ohne Maske).
  Masken      maps/<Karte>/texture/map/tile_<x>_<y>.map.<n>.dds fuer den n-ten Eintrag (ab 1): unkomprimiertes A8
              (nur Alphakanal, 8 Bit, ohne Mip-Stufen), Zeile 0 = Norden, Spalte 0 = Westen. Fehlt die Datei, liegt der
              Belag in der Kachel nicht. Die Belaege werden nacheinander ueber den Grund geblendet.

Klassifikation je 0,5-m-Zelle (das Raster der Belegungskarte F4), von unten nach oben:
  1. Luftbild DOP20 (Bayerische Vermessungsverwaltung, CC BY 4.0): Vegetation, offener Boden, dunkel/hell versiegelt,
     roetlich (Pflaster), Schatten = unbekannt
  2. OSM-Flaechen als Zusammenhang (Park, Feld, Bahn, Baustelle, Wald): dieselbe Luftbildklasse heisst im Park etwas
     anderes (heller Weg = Schotter) als in der Stadt (heller Belag = Pflaster)
  3. OSM-Flaechen mit festem Belag (Parkplatz, Platz, Fussgaengerzone, Sportplatz; surface=* gewinnt)
  4. OSM-Linien, die nicht als Strasse gebaut sind (Fuss-/Radwege, Feldwege, Zufahrten, Gleise, Baeche) mit Breite
  5. Gewaesserflaechen
  6. Belegungskarte (aus netz.json, also aus den Querschnitten der gebauten Strassen): Fahrbahn und Kreuzung = Asphalt,
     Gehweg = Pflaster
Unter Gebaeuden (OSM, Belegung) ist der Boden unbekannt und wird wie Schatten aus der Nachbarschaft gefuellt.

Weiche Kanten: die Klassen werden als Anteile auf 512 x 512 gemittelt und weichgezeichnet (etwa 1 m), dann so in
Deckkraefte umgerechnet, dass das Uebereinanderblenden genau die Anteile ergibt (alpha_n = w_n / (w_0 + ... + w_n)).

Ablage: Masken in texture/map, maps/<Karte>/boden.json (Belaege, Anteile je Kachel, Quellen) und ein Eintrag "boden"
je Kachel in schichten.json (F2: gegen welchen Stand von netz gebaut). Das Luftbild ist nie die sichtbare Textur.

  python -m omsigen.boden <Karte> [--kacheln 3_7 4_7] [--omsi <OMSI>] [--ohne-luftbild] [--ohne-osm] [--bild <png>]
"""
import argparse
import datetime
import glob
import hashlib
import json
import math
import os
import re
import shutil
import struct
import sys

import numpy as np

from . import schichten

KACHEL = 300.0
ZELLE = 0.5
N = int(KACHEL / ZELLE)               # 600 Zellen je Kachel (wie die Belegung)
EXP = 9
MASKE = 2 ** EXP                      # 512 Texel je Kachel (0,59 m)
WEICH = 1.0                           # Weichzeichnen der Anteile in Texeln (Sigma)
DATEI = 'boden.json'
FORMAT, VERSION = 'omsigen-boden', 1
GEODATEN = os.path.join(os.environ.get('LOCALAPPDATA', '.'), 'omsi-editor', 'geodaten')
QUELLE_DOP = 'Bodenbelaege nach Luftbild DOP20: Bayerische Vermessungsverwaltung - www.geodaten.bayern.de (CC BY 4.0)'
QUELLE_OSM = 'Flaechennutzung (c) OpenStreetMap-Mitwirkende, ODbL 1.0 (https://www.openstreetmap.org/copyright)'

# Klassen = Index des [groundtex]-Eintrags (0 = Grund ohne Maske)
GRUND, GRAS, ERDE, SCHOTTER, PFLASTER, ASPHALT, WASSER = range(7)
UNBEKANNT = 255
KLASSEN = ['grund', 'gras', 'erde', 'schotter', 'pflaster', 'asphalt', 'wasser']
FARBEN = [(120, 140, 80), (60, 170, 60), (150, 105, 60), (175, 170, 150), (205, 150, 120), (70, 70, 75), (40, 100, 210)]

# eigene, erzeugte Texturen (texturen_installieren): Gewaesser - OMSI-Karten von omsigen haben (noch) keine
# Wasserflaeche, der Belag soll aus der Naehe wie ein dunkler Fluss aussehen, nicht wie Schlamm
EIGEN_WASSER = 'Texture\\Aschaffenburg\\boden_wasser.dds'

# Texturen je Belag: der erste vorhandene Kandidat gilt (Standard-OMSI als Rueckfall). (Textur, Detail, Wiederholungen
# je Kachel, Detail-Wiederholungen); Wiederholungen so, dass die Muster echte Groessen haben (Pflaster ~4 m je Bild).
BELAEGE = [
    ('grund', [('Texture\\gras.bmp', 'Texture\\gras_det.bmp', 1, 60)]),
    ('gras', [('Texture\\ADDON_Rheinhausen\\gras_wild.dds', 'Texture\\gras_det.bmp', 25, 60),     # heller als gras.dds
              ('Texture\\ADDON_Rheinhausen\\gras.dds', 'Texture\\gras_det.bmp', 20, 60),
              ('Texture\\gras.bmp', 'Texture\\gras_det.bmp', 1, 60)]),
    ('erde', [('Texture\\ADDON_Rheinhausen\\erde_feld01.dds', 'Texture\\noise_low.bmp', 30, 1),
              ('Texture\\Sand.bmp', 'Texture\\noise_low.bmp', 40, 1)]),
    ('schotter', [('Texture\\ADDON_Rheinhausen\\schotter_neu.dds', 'Texture\\noise_low.bmp', 60, 1),
                  ('Sceneryobjects\\Railroad_MC\\texture\\Schotter.bmp', 'Texture\\noise_low.bmp', 50, 10)]),
    ('pflaster', [('Texture\\ADDON_Rheinhausen\\str_k_kopfstein_1.dds', 'Texture\\noise_low.bmp', 75, 1),
                  ('Texture\\str_k_kopfstein.bmp', 'Texture\\noise_low.bmp', 50, 1)]),
    ('asphalt', [('Texture\\PAD-Labs\\Asphalt2.bmp', 'Texture\\noise_low.bmp', 50, 1),
                 ('Texture\\str_asphdrk.bmp', 'Texture\\noise_low.bmp', 40, 1)]),
    ('wasser', [(EIGEN_WASSER, 'Texture\\noise_low.bmp', 12, 1),
                ('Texture\\PAD-Labs\\Schlamm.bmp', 'Texture\\noise_low.bmp', 40, 1),
                ('Texture\\dreck.bmp', 'Texture\\noise_low.bmp', 40, 1)]),
]
# Add-ons, aus denen Texturen kommen koennen (Ordner -> Name fuer die Kartenbeschreibung)
ADDONS = {'texture\\addon_rheinhausen\\': 'Add-on Rheinhausen (Texture\\ADDON_Rheinhausen)',
          'texture\\pad-labs\\': 'Koeln/PAD-Labs-Texturen (Texture\\PAD-Labs)'}


def _ist_omsi(omsi):
    return bool(omsi) and os.path.isdir(os.path.join(omsi, 'Texture'))


def wasser_textur(groesse=512, saat=16):
    """kachelbare Gewaessertextur (PIL-Bild): dunkles Gruenblau wie der Main im DOP20, weiche Stroemungsstreifen aus
    periodischem Rauschen (ganzzahlige Frequenzen -> nahtlos)"""
    from PIL import Image
    rng = np.random.default_rng(saat)
    f = np.fft.fftfreq(groesse) * groesse
    fx, fy = np.meshgrid(f, f)
    r = np.hypot(fx * 0.35, fy)                     # in einer Richtung gestreckt: Stroemungsstreifen
    spek = (rng.normal(size=r.shape) + 1j * rng.normal(size=r.shape)) / np.maximum(r, 1.0) ** 1.6
    spek[0, 0] = 0
    n = np.real(np.fft.ifft2(spek))
    n = (n - n.mean()) / (n.std() + 1e-9)
    basis = np.array([38.0, 66.0, 62.0])
    hell = np.array([70.0, 98.0, 96.0])
    t = np.clip(0.5 + 0.18 * n, 0, 1)[..., None]
    rgb = basis * (1 - t) + hell * t
    return Image.fromarray(np.clip(rgb, 0, 255).astype(np.uint8), 'RGB')


def texturen_installieren(omsi):
    """eigene Bodentexturen in die OMSI-Installation schreiben, falls sie fehlen -> Zahl der neuen Dateien"""
    if not _ist_omsi(omsi):
        return 0
    import io
    from .lod2 import dds_bc1
    ziel = os.path.join(omsi, *EIGEN_WASSER.split(chr(92)))
    if os.path.isfile(ziel):
        return 0
    os.makedirs(os.path.dirname(ziel), exist_ok=True)
    b = io.BytesIO()
    wasser_textur().save(b, format='PNG')
    b.seek(0)
    dds_bc1(b, ziel + '.tmp')
    os.replace(ziel + '.tmp', ziel)
    return 1


def belaege(omsi=None):
    """[groundtex]-Eintraege dieser Installation -> [dict(name, textur, detail, exp, wdh, detail_wdh)]"""
    out = []
    if not _ist_omsi(omsi):
        omsi = None
    for name, kand in BELAEGE:
        wahl = kand[-1]
        if omsi:
            wahl = next((k for k in kand if os.path.isfile(os.path.join(omsi, *k[0].split('\\')))), kand[-1])
        else:
            wahl = kand[0]
        out.append(dict(name=name, textur=wahl[0], detail=wahl[1], exp=0 if name == 'grund' else EXP, wdh=wahl[2],
                        detail_wdh=wahl[3]))
    return out


def groundtex_zeilen(omsi=None):
    """[groundtex]-Bloecke fuer global.cfg (Zeilen); installiert dazu die eigenen Texturen in `omsi`"""
    texturen_installieren(omsi)
    L = []
    for b in belaege(omsi):
        L += ['[groundtex]', b['textur'], b['detail'], str(b['exp']), str(b['wdh']), str(b['detail_wdh']), '']
    return L


def addons(omsi=None):
    """Namen der Add-ons, deren Texturen die Belaege nutzen"""
    out = []
    for b in belaege(omsi):
        for pfx, name in ADDONS.items():
            if b['textur'].lower().startswith(pfx) and name not in out:
                out.append(name)
    return out


# ---------------------------------------------------------------- DDS (A8 wie die Masken der Standardkarten)

def dds_a8(a):
    """uint8-Array (Zeile 0 = Norden) -> DDS-Bytes: A8 unkomprimiert, ohne Mip-Stufen, Kopf wie die Standardmasken"""
    a = np.ascontiguousarray(a, dtype=np.uint8)
    h, w = a.shape
    kopf = struct.pack('<4s31I', b'DDS ', 124, 0x1007, h, w, 0, 0, 0, *([0] * 11), 32, 0x2, 0, 8, 0, 0, 0, 0xFF,
                       0x1002, 0, 0, 0, 0)
    return kopf + a.tobytes()


def dds_lesen(daten_oder_pfad):
    """Maske (A8, auch L8) lesen -> uint8-Array (Zeile 0 = erste Zeile der Datei)"""
    d = daten_oder_pfad
    if isinstance(d, str):
        with open(d, 'rb') as f:
            d = f.read()
    if d[:4] != b'DDS ':
        raise ValueError('keine DDS-Datei')
    h, w = struct.unpack('<2I', d[12:20])
    pf_flags, fourcc, bits = struct.unpack('<I4sI', d[80:92])
    if fourcc != b'\0\0\0\0' or bits != 8:
        raise ValueError(f'Maske nicht 8 Bit unkomprimiert ({fourcc!r}, {bits} Bit)')
    return np.frombuffer(d[128:128 + w * h], np.uint8).reshape(h, w)


# ---------------------------------------------------------------- Luftbild

D_UNBEKANNT, D_GRUEN, D_OFFEN, D_DUNKEL, D_HELL, D_ROT = range(6)


def mehrheit(k, werte, groesse):
    """Mehrheitsfilter: je Zelle der haeufigste Wert im Fenster groesse x groesse (nur Werte aus `werte`)"""
    from scipy import ndimage
    best = np.full(k.shape, -1.0, np.float32)
    wahl = k.copy()
    for c in werte:
        a = ndimage.uniform_filter((k == c).astype(np.float32), groesse, mode='nearest')
        m = a > best
        best[m] = a[m]
        wahl[m] = c
    return wahl


def dop_klassen(rgb, glaetten=7):
    """Luftbild (H x W x 3, 0..255) -> Klasse je Pixel: unbekannt (Schatten), gruen (Vegetation), offen (Erde, trockene
    Wiese), dunkel bzw. hell versiegelt (Asphalt/Pflaster, Beton), rot (rotes Pflaster, Ziegel). Gemessen am DOP20 in
    Aschaffenburg (Sommer): Asphalt ist fast farblos (v 80..120), Schatten dunkler und blaeulich, Vegetation gruen
    ueberwiegend, Erde/trockene Wiese braun (r > g > b). Danach Mehrheit im Fenster `glaetten` (Zellen), damit keine
    Pixelflecken entstehen."""
    rgb = np.asarray(rgb, float)
    r, g, b = rgb[..., 0], rgb[..., 1], rgb[..., 2]
    v = rgb.mean(axis=-1)
    sat = rgb.max(axis=-1) - rgb.min(axis=-1)
    exg = 2 * g - r - b
    k = np.full(v.shape, D_HELL, np.uint8)
    k[v < 118] = D_DUNKEL
    braun = (r > g) & (g > b) & (sat > 28)
    k[braun] = D_OFFEN
    k[braun & (r - g > 32)] = D_ROT
    k[(exg > 10) & (g >= r)] = D_GRUEN
    k[(exg > 4) & (g > r + 6) & (sat > 14)] = D_GRUEN
    schatten = (v < 70) | ((v < 100) & (b > r + 6) & (k != D_GRUEN))
    k[schatten] = D_UNBEKANNT
    if glaetten and glaetten > 1:
        k = mehrheit(k, range(6), glaetten)
    return k


# Zusammenhang aus OSM-Flaechen -> was eine Luftbildklasse dort ist
K_KEIN, K_GRUEN, K_FELD, K_BAHN, K_OFFEN, K_WALD = range(6)
KONTEXT = {
    #        unbekannt  gruen  offen     dunkel    hell      rot
    K_KEIN: (UNBEKANNT, GRAS, GRAS, ASPHALT, PFLASTER, PFLASTER),
    K_GRUEN: (GRAS, GRAS, GRAS, ASPHALT, SCHOTTER, PFLASTER),
    K_FELD: (ERDE, GRAS, ERDE, ERDE, ERDE, ERDE),
    K_BAHN: (SCHOTTER, GRAS, SCHOTTER, SCHOTTER, SCHOTTER, SCHOTTER),
    K_OFFEN: (ERDE, GRAS, ERDE, ASPHALT, SCHOTTER, ERDE),
    K_WALD: (GRAS, GRAS, ERDE, GRAS, GRAS, ERDE),
}


# ---------------------------------------------------------------- OSM

SURFACE = {
    'asphalt': ASPHALT, 'concrete': ASPHALT, 'concrete:lanes': ASPHALT, 'chipseal': ASPHALT, 'metal': ASPHALT,
    'paved': PFLASTER, 'paving_stones': PFLASTER, 'paving_stones:lanes': PFLASTER, 'sett': PFLASTER,
    'cobblestone': PFLASTER, 'unhewn_cobblestone': PFLASTER, 'concrete:plates': PFLASTER, 'bricks': PFLASTER,
    'stone': PFLASTER, 'wood': PFLASTER,
    'gravel': SCHOTTER, 'fine_gravel': SCHOTTER, 'compacted': SCHOTTER, 'pebblestone': SCHOTTER, 'unpaved': SCHOTTER,
    'rock': SCHOTTER, 'woodchips': ERDE,
    'dirt': ERDE, 'earth': ERDE, 'ground': ERDE, 'mud': ERDE, 'sand': ERDE, 'clay': ERDE, 'tartan': ERDE,
    'grass': GRAS, 'grass_paver': GRAS, 'artificial_turf': GRAS,
}
# Linien: Breite (m) und Belag ohne surface-Tag
LINIEN = {
    'motorway': (11.0, ASPHALT), 'trunk': (9.0, ASPHALT), 'primary': (8.0, ASPHALT), 'secondary': (7.0, ASPHALT),
    'tertiary': (6.5, ASPHALT), 'unclassified': (5.5, ASPHALT), 'residential': (5.5, ASPHALT),
    'living_street': (5.0, PFLASTER), 'service': (3.5, ASPHALT), 'busway': (6.0, ASPHALT), 'pedestrian': (5.0, PFLASTER),
    'footway': (2.0, PFLASTER), 'cycleway': (2.0, ASPHALT), 'path': (1.5, SCHOTTER), 'bridleway': (2.0, SCHOTTER),
    'track': (3.0, SCHOTTER), 'steps': (2.0, PFLASTER), 'platform': (2.5, PFLASTER),
    'motorway_link': (6.0, ASPHALT), 'trunk_link': (6.0, ASPHALT), 'primary_link': (6.0, ASPHALT),
    'secondary_link': (6.0, ASPHALT), 'tertiary_link': (5.5, ASPHALT),
}
R_KONTEXT, R_FEST, R_GEBAEUDE, R_LINIE, R_WASSER = 10, 20, 25, 30, 40


def _breite(t, standard):
    try:
        return max(0.5, min(40.0, float(str(t.get('width', '')).replace(',', '.').split()[0])))
    except (ValueError, IndexError):
        return standard


def osm_regel(t, flaeche):
    """OSM-Tags eines Wegs/einer Flaeche -> (Rang, Art, Wert, Breite) oder None. Art 'kontext' (Wert K_*), 'fest'
    (Wert Klasse), 'gebaeude' (Boden unbekannt). Breite nur bei Linien."""
    if t.get('tunnel') not in (None, 'no') or t.get('covered') == 'yes' or t.get('location') == 'underground':
        return None
    try:
        if int(str(t.get('layer', '0')).split(';')[0]) < 0:
            return None
    except ValueError:
        pass
    s = SURFACE.get(str(t.get('surface', '')).split(';')[0])
    if flaeche:
        if t.get('building') not in (None, 'no') or t.get('building:part') not in (None, 'no'):
            return (R_GEBAEUDE, 'gebaeude', None, None)
        if t.get('natural') == 'water' or t.get('waterway') in ('riverbank', 'dock') or \
                t.get('landuse') in ('reservoir', 'basin') or t.get('leisure') == 'swimming_pool':
            return (R_WASSER, 'fest', WASSER, None)
        if t.get('parking') in ('multi-storey', 'underground', 'rooftop'):
            return (R_GEBAEUDE, 'gebaeude', None, None) if t.get('parking') == 'multi-storey' else None
        if t.get('amenity') in ('parking', 'bicycle_parking', 'motorcycle_parking') or t.get('parking') or \
                t.get('landuse') == 'garages':
            return (R_FEST, 'fest', ASPHALT if s is None else s, None)
        if t.get('area:highway') or (t.get('highway') and t.get('area') == 'yes') or t.get('place') == 'square' or \
                t.get('amenity') in ('marketplace',) or t.get('highway') in ('pedestrian',):
            hw = t.get('area:highway') or t.get('highway')
            standard = ASPHALT if hw in LINIEN and LINIEN[hw][1] == ASPHALT and hw not in ('pedestrian',) else PFLASTER
            return (R_FEST, 'fest', standard if s is None else s, None)
        if t.get('leisure') in ('pitch', 'track', 'playground') and s is not None:
            return (R_FEST, 'fest', s, None)
        if t.get('railway') == 'platform' or t.get('public_transport') == 'platform':
            return (R_FEST, 'fest', PFLASTER if s is None else s, None)
        lu, le, na = t.get('landuse'), t.get('leisure'), t.get('natural')
        if lu in ('farmland', 'farmyard') or na in ('bare_rock',):
            return (R_KONTEXT, 'kontext', K_FELD, None)
        if lu in ('railway',):
            return (R_KONTEXT, 'kontext', K_BAHN, None)
        if lu in ('construction', 'brownfield', 'landfill', 'quarry') or na in ('sand', 'beach') or le == 'playground':
            return (R_KONTEXT, 'kontext', K_OFFEN, None)
        if lu in ('forest',) or na in ('wood', 'scrub'):
            return (R_KONTEXT, 'kontext', K_WALD, None)
        if lu in ('grass', 'meadow', 'village_green', 'recreation_ground', 'cemetery', 'allotments', 'orchard',
                  'vineyard', 'greenfield', 'plant_nursery', 'flowerbed') or \
                le in ('park', 'garden', 'golf_course', 'pitch', 'dog_park', 'nature_reserve') or \
                na in ('grassland', 'heath', 'wetland') or t.get('amenity') == 'grave_yard':
            return (R_KONTEXT, 'kontext', K_GRUEN, None)
        return None
    # Linien
    if t.get('bridge') not in (None, 'no') or t.get('man_made') == 'bridge':
        return None
    if t.get('area') == 'yes':
        return None
    hw = t.get('highway')
    if hw in LINIEN:
        b, k = LINIEN[hw]
        if hw == 'track' and s is None and t.get('tracktype') in ('grade1',):
            s = ASPHALT
        if hw == 'track' and s is None and t.get('tracktype') in ('grade4', 'grade5'):
            s = ERDE
        return (R_LINIE, 'fest', k if s is None else s, _breite(t, b))
    rw = t.get('railway')
    if rw in ('rail', 'light_rail', 'narrow_gauge', 'disused', 'preserved') and t.get('embedded') not in ('yes',):
        return (R_LINIE, 'fest', SCHOTTER, _breite(t, 4.0))
    ww = t.get('waterway')
    if ww in ('river', 'stream', 'canal', 'ditch', 'drain'):
        return (R_LINIE, 'fest', WASSER, _breite(t, {'river': 12.0, 'canal': 8.0, 'stream': 2.5}.get(ww, 1.2)))
    return None


ABFRAGE = """[out:json][timeout:180];
(
  way["landuse"]({b}); relation["landuse"]["type"="multipolygon"]({b});
  way["leisure"]({b}); relation["leisure"]["type"="multipolygon"]({b});
  way["natural"]({b}); relation["natural"]["type"="multipolygon"]({b});
  way["amenity"~"^(parking|bicycle_parking|motorcycle_parking|marketplace|grave_yard)$"]({b});
  relation["amenity"~"^(parking|marketplace)$"]["type"="multipolygon"]({b});
  way["place"="square"]({b}); relation["place"="square"]({b});
  way["area:highway"]({b});
  way["highway"]({b}); relation["highway"]["type"="multipolygon"]({b});
  way["railway"~"^(rail|light_rail|narrow_gauge|platform|disused|preserved)$"]({b});
  way["public_transport"="platform"]({b});
  way["waterway"]({b}); relation["waterway"]["type"="multipolygon"]({b});
  way["building"]({b}); relation["building"]["type"="multipolygon"]({b});
  way["parking"]({b});
);
out body geom;"""


def osm_laden(sued, west, nord, ost, cache):
    """Flaechen und Linien der Umgebung (Overpass, Zwischenspeicher je Ausschnitt) -> Overpass-JSON"""
    from . import osm
    b = f'{sued:.5f},{west:.5f},{nord:.5f},{ost:.5f}'
    h = hashlib.sha1((b + ABFRAGE).encode()).hexdigest()[:16]          # neue Abfrage = neuer Eintrag
    datei = os.path.join(cache, f'flaechen_{h}.json')
    if os.path.isfile(datei):
        with open(datei, encoding='utf-8') as f:
            return json.load(f)
    import time
    req = osm._requests()
    servers = [osm.OVERPASS] + osm.OVERPASS_ALT
    letzte = None
    for versuch in range(2 * len(servers)):
        try:
            r = req.post(servers[versuch % len(servers)], data={'data': ABFRAGE.format(b=b)},
                         headers={'User-Agent': osm.UA}, timeout=240)
        except req.exceptions.RequestException as ex:
            letzte = type(ex).__name__
            continue
        if r.status_code != 200:
            letzte = f'HTTP {r.status_code}'
            time.sleep(4 * (versuch // len(servers) + 1))
            continue
        js = r.json()
        if 'runtime error' in js.get('remark', ''):
            letzte = js['remark'][:80]
            continue
        os.makedirs(cache, exist_ok=True)
        with open(datei + '.tmp', 'w', encoding='utf-8') as f:
            json.dump(js, f)
        os.replace(datei + '.tmp', datei)
        return js
    raise RuntimeError(f'Overpass nicht erreichbar ({letzte})')


def _ringe(teile):
    """Wegstuecke (Punktlisten) zu geschlossenen Ringen verbinden"""
    offen = [list(t) for t in teile if len(t) >= 2]
    ringe = []
    while offen:
        r = offen.pop(0)
        weiter = True
        while weiter and r[0] != r[-1]:
            weiter = False
            for i, t in enumerate(offen):
                if t[0] == r[-1]:
                    r += t[1:]
                elif t[-1] == r[-1]:
                    r += t[::-1][1:]
                elif t[-1] == r[0]:
                    r = t + r[1:]
                elif t[0] == r[0]:
                    r = t[::-1] + r[1:]
                else:
                    continue
                offen.pop(i)
                weiter = True
                break
        if len(r) >= 4:
            ringe.append(r)
    return ringe


def osm_merkmale(js, nach_karte):
    """Overpass-JSON -> Merkmale [dict(rang, art, wert, breite, aussen: [Ringe], innen: [Ringe] | linie: Punkte,
    flaeche_m2)] in Kartenmetern; nach_karte(lat, lon) -> (x, z)"""
    out = []

    def pkt(geom):
        return [tuple(round(v, 2) for v in nach_karte(p['lat'], p['lon'])) for p in geom if p]

    def flaeche(ring):
        a = np.asarray(ring, float)
        return 0.5 * abs(float(np.dot(a[:-1, 0], a[1:, 1]) - np.dot(a[1:, 0], a[:-1, 1])))
    for el in js.get('elements', []):
        t = el.get('tags', {})
        if el['type'] == 'way' and el.get('geometry'):
            P = pkt(el['geometry'])
            geschlossen = len(P) >= 4 and P[0] == P[-1]
            ist_flaeche = geschlossen and (t.get('area') == 'yes' or not (t.get('highway') or t.get('railway') in
                                           ('rail', 'light_rail', 'narrow_gauge') or t.get('waterway') in
                                           ('river', 'stream', 'canal', 'ditch', 'drain') or t.get('barrier')))
            if t.get('highway') and geschlossen and (t.get('area') == 'yes' or t.get('area:highway')):
                ist_flaeche = True
            if t.get('highway') in ('pedestrian',) and geschlossen and t.get('area') != 'no':
                ist_flaeche = ist_flaeche or t.get('area') == 'yes'
            regel = osm_regel(t, ist_flaeche)
            if regel is None and geschlossen and not ist_flaeche:      # geschlossene Linie ohne Linienregel: Flaeche?
                regel = osm_regel(t, True)
                ist_flaeche = regel is not None
            if regel is None:
                continue
            rang, art, wert, breite = regel
            if ist_flaeche:
                out.append(dict(rang=rang, art=art, wert=wert, aussen=[P], innen=[], flaeche_m2=flaeche(P), id=el['id']))
            else:
                out.append(dict(rang=rang, art=art, wert=wert, breite=breite, linie=P, flaeche_m2=0.0, id=el['id']))
        elif el['type'] == 'relation':
            regel = osm_regel(t, True)
            if regel is None:
                continue
            aus = [pkt(m['geometry']) for m in el.get('members', []) if m.get('type') == 'way' and m.get('geometry')
                   and m.get('role') in ('outer', '')]
            inn = [pkt(m['geometry']) for m in el.get('members', []) if m.get('type') == 'way' and m.get('geometry')
                   and m.get('role') == 'inner']
            aussen, innen = _ringe(aus), _ringe(inn)
            if aussen:
                out.append(dict(rang=regel[0], art=regel[1], wert=regel[2], aussen=aussen, innen=innen,
                                flaeche_m2=sum(flaeche(r) for r in aussen), id=el['id']))
    # Zeichenreihenfolge: Rang, dann grosse Flaechen zuerst (kleinere darin gewinnen)
    out.sort(key=lambda m: (m['rang'], -m['flaeche_m2']))
    return out


# ---------------------------------------------------------------- Raster je Kachel

def _px(punkte, k):
    return [((x - k[0] * KACHEL) / ZELLE, (k[1] * KACHEL + KACHEL - z) / ZELLE) for x, z in punkte]


def _bbox(punkte):
    a = np.asarray(punkte, float)
    return a[:, 0].min(), a[:, 1].min(), a[:, 0].max(), a[:, 1].max()


def _trifft(bb, k, rand=0.0):
    x0, z0, x1, z1 = bb
    return not (x1 < k[0] * KACHEL - rand or x0 > (k[0] + 1) * KACHEL + rand or z1 < k[1] * KACHEL - rand
                or z0 > (k[1] + 1) * KACHEL + rand)


def merkmale_zeichnen(merkmale, k):
    """OSM-Merkmale in die Kachel k -> (Kontext uint8, Klasse uint8 mit UNBEKANNT=255 fuer Gebaeude, gesetzt bool)"""
    from PIL import Image, ImageDraw
    kontext = Image.new('L', (N, N), K_KEIN)
    klasse = Image.new('L', (N, N), 0)
    gesetzt = Image.new('L', (N, N), 0)
    dk, dc, dg = ImageDraw.Draw(kontext), ImageDraw.Draw(klasse), ImageDraw.Draw(gesetzt)
    for m in merkmale:
        if 'linie' in m:
            if len(m['linie']) < 2 or not _trifft(m.setdefault('bb', _bbox(m['linie'])), k, m['breite']):
                continue
            w = max(1, int(round(m['breite'] / ZELLE)))
            pts = _px(m['linie'], k)
            dc.line(pts, fill=m['wert'], width=w, joint='curve')
            dg.line(pts, fill=255, width=w, joint='curve')
            for p in (pts[0], pts[-1]):            # runde Enden
                r = w / 2
                dc.ellipse([p[0] - r, p[1] - r, p[0] + r, p[1] + r], fill=m['wert'])
                dg.ellipse([p[0] - r, p[1] - r, p[0] + r, p[1] + r], fill=255)
            continue
        if not _trifft(m.setdefault('bb', _bbox([p for r in m['aussen'] for p in r])), k):
            continue
        if m['innen']:
            maske = Image.new('L', (N, N), 0)
            dm = ImageDraw.Draw(maske)
            for r in m['aussen']:
                dm.polygon(_px(r, k), fill=255)
            for r in m['innen']:
                dm.polygon(_px(r, k), fill=0)
            if m['art'] == 'kontext':
                kontext.paste(m['wert'], mask=maske)
            else:
                klasse.paste(UNBEKANNT if m['art'] == 'gebaeude' else m['wert'], mask=maske)
                gesetzt.paste(255, mask=maske)
            continue
        for r in m['aussen']:
            pts = _px(r, k)
            if m['art'] == 'kontext':
                dk.polygon(pts, fill=m['wert'])
            else:
                dc.polygon(pts, fill=UNBEKANNT if m['art'] == 'gebaeude' else m['wert'])
                dg.polygon(pts, fill=255)
    return np.asarray(kontext), np.asarray(klasse), np.asarray(gesetzt) > 0


# Belegung (F4): frei, fahrbahn, gehweg, kreuzung, gebaeude, gruen, wasser, bauwerk -> Klasse (None = nicht setzen)
AUS_BELEGUNG = [None, ASPHALT, PFLASTER, ASPHALT, UNBEKANNT, GRAS, WASSER, None]


def klassifizieren(dop=None, kontext=None, osm_klasse=None, osm_gesetzt=None, belegung=None, fuellen_m=60.0):
    """Klasse je 0,5-m-Zelle einer Kachel (N x N, Zeile 0 = Norden) aus Luftbild-Klassen (dop_klassen oder None),
    OSM-Kontext, festen OSM-Klassen (mit Maske) und der Belegung (Indexraster der Belegungskarte)"""
    if dop is None:
        dop = np.full((N, N), D_UNBEKANNT, np.uint8)
        mit_dop = False
    else:
        mit_dop = True
    if kontext is None:
        kontext = np.zeros(dop.shape, np.uint8)
    tab = np.array([KONTEXT[c] for c in range(len(KONTEXT))], np.uint8)
    out = tab[kontext, dop]
    if mit_dop:                         # Flecken aus dem Luftbild glaetten (feste Flaechen kommen danach)
        out = mehrheit(out, [c for c in range(len(KLASSEN))] + [UNBEKANNT], 7)
    if not mit_dop:                     # ohne Luftbild: nichts aus der Nachbarschaft schwemmen, Grund bleibt
        out[(kontext == K_KEIN)] = GRUND
    if osm_klasse is not None and osm_gesetzt is not None:
        out[osm_gesetzt] = osm_klasse[osm_gesetzt]
    if belegung is not None:
        for b, c in enumerate(AUS_BELEGUNG):
            if c is not None:
                out[belegung == b] = c
    unb = out == UNBEKANNT
    if unb.any():
        if unb.all():
            out[:] = GRUND
        else:
            from scipy import ndimage
            d, (ii, jj) = ndimage.distance_transform_edt(unb, return_indices=True)
            voll = out[ii, jj]
            voll[d * ZELLE > fuellen_m] = GRUND
            out = np.where(unb, voll, out)
    return out.astype(np.uint8)


def anteile(klassen, n=MASKE, weich=WEICH):
    """Klassenraster -> Anteile je Klasse (len(KLASSEN) x n x n, Summe 1), gemittelt und weichgezeichnet"""
    from PIL import Image
    from scipy import ndimage
    w = np.zeros((len(KLASSEN), n, n), np.float32)
    for c in range(len(KLASSEN)):
        m = (klassen == c)
        if not m.any():
            continue
        a = np.asarray(Image.fromarray(m.astype(np.float32), 'F').resize((n, n), Image.BOX), np.float32)
        if weich:
            a = ndimage.gaussian_filter(a, weich, mode='nearest')
        w[c] = a
    s = w.sum(axis=0)
    s[s <= 1e-6] = 1.0
    return w / s


def deckkraft(w):
    """Anteile -> Deckkraft je Belag fuer das Uebereinanderblenden (Belag n ueber allen davor):
    alpha_n = w_n / (w_0 + ... + w_n); Belag 0 (Grund) hat keine Maske -> Liste fuer n = 1.."""
    kum = np.cumsum(w, axis=0)
    out = []
    for c in range(1, len(w)):
        a = np.where(w[c] > 1e-4, w[c] / np.maximum(kum[c], 1e-6), 0.0)
        out.append(np.clip(np.round(a * 255), 0, 255).astype(np.uint8))
    return out


def ueberblenden(alphas):
    """Kontrolle: Deckkraefte -> sichtbare Anteile je Belag (wie OMSI blendet)"""
    n = len(alphas) + 1
    sicht = np.zeros((n,) + alphas[0].shape, np.float32)
    sicht[0] = 1.0
    for c, a in enumerate(alphas, 1):
        a = a.astype(np.float32) / 255
        sicht[:c] *= (1 - a)
        sicht[c] = a
    return sicht


def vorschau(klassen):
    """Klassenraster -> RGB-Bild (PIL)"""
    from PIL import Image
    pal = np.array(FARBEN + [(255, 0, 255)] * (256 - len(FARBEN)), np.uint8)
    return Image.fromarray(pal[klassen], 'RGB')


# ---------------------------------------------------------------- Karte

def masken_hash(kartenordner, k):
    """Inhalt der Malmasken einer Kachel (ohne Gelaende)"""
    h = hashlib.sha1()
    for f in sorted(glob.glob(os.path.join(kartenordner, 'texture', 'map', f'tile_{k[0]}_{k[1]}.map.*.dds'))):
        h.update(os.path.basename(f).encode())
        with open(f, 'rb') as d:
            h.update(d.read())
    return h.hexdigest()[:16]


def _gcfg_lesen(kartenordner):
    with open(os.path.join(kartenordner, 'global.cfg'), encoding='utf-16') as f:
        return f.read().replace('\r\n', '\n')


def groundtex_lesen(kartenordner):
    """[groundtex]-Eintraege der Karte -> [(Textur, Detail, p1, p2, p3)]"""
    z = _gcfg_lesen(kartenordner).split('\n')
    out = []
    for i, s in enumerate(z):
        if s.strip() == '[groundtex]' and i + 5 < len(z):
            out.append(tuple(v.strip() for v in z[i + 1:i + 6]))
    return out


def groundtex_einrichten(kartenordner, omsi, log=print):
    """global.cfg der Karte auf die Belaege von omsigen.boden bringen. Hat die Karte schon Masken anderer Belaege
    (z. B. im OMSI-Editor gemalt), bleibt sie unveraendert (Fehler). -> True, wenn geaendert"""
    texturen_installieren(omsi)
    soll = [(b['textur'], b['detail'], str(b['exp']), str(b['wdh']), str(b['detail_wdh'])) for b in belaege(omsi)]
    ist = groundtex_lesen(kartenordner)
    if ist == soll:
        return False
    eigen = os.path.isfile(os.path.join(kartenordner, DATEI))
    fremd = [f for f in glob.glob(os.path.join(kartenordner, 'texture', 'map', 'tile_*.map.*.dds'))]
    if fremd and not eigen:
        raise RuntimeError(f'Karte hat schon {len(fremd)} gemalte Masken anderer Belaege (z. B. '
                           f'{os.path.basename(fremd[0])}) - Boden wird nicht ersetzt')
    text = _gcfg_lesen(kartenordner)
    z = text.split('\n')
    neu, i, eingefuegt = [], 0, False
    while i < len(z):
        if z[i].strip() == '[groundtex]':
            if not eingefuegt:
                neu += groundtex_zeilen(omsi)[:-1]
                eingefuegt = True
            i += 6
            while i < len(z) and not z[i].strip():
                i += 1
            neu.append('')
            continue
        neu.append(z[i])
        i += 1
    if not eingefuegt:              # ohne [groundtex]: vor den [map]-Eintraegen einfuegen
        j = next((n for n, s in enumerate(neu) if s.strip() == '[map]'), len(neu))
        neu[j:j] = groundtex_zeilen(omsi)
    p = os.path.join(kartenordner, 'global.cfg')
    if not os.path.isfile(p + '.vor_boden'):
        shutil.copy(p, p + '.vor_boden')
    with open(p, 'w', encoding='utf-16', newline='') as f:
        f.write('\n'.join(neu).replace('\n', '\r\n'))
    log('    global.cfg: Bodenbelaege eingetragen (alte Fassung: global.cfg.vor_boden)')
    return True


def quellen_eintragen(kartenordner, quellen, hinweise=()):
    """Quellenangaben (DOP, OSM) und noetige Add-ons in die Kartenbeschreibung (global.cfg) und LIESMICH schreiben"""
    text = _gcfg_lesen(kartenordner)
    m = re.search(r'\[description\]\n(.*?)\n\[end\]', text, re.S)
    zeilen = list(quellen) + list(hinweise)
    if m:
        alt = m.group(1)
        dazu = [q for q in zeilen if q not in alt]
        if dazu:
            text = text[:m.end(1)] + ''.join('\n' + q for q in dazu) + text[m.end(1):]
            with open(os.path.join(kartenordner, 'global.cfg'), 'w', encoding='utf-16', newline='') as f:
                f.write(text.replace('\n', '\r\n'))
    lies = os.path.join(kartenordner, 'LIESMICH_omsigen.txt')
    alt = open(lies, encoding='utf-8').read() if os.path.isfile(lies) else ''
    dazu = [q for q in zeilen if q not in alt]
    if dazu:
        with open(lies, 'a', encoding='utf-8') as f:
            f.write(''.join(q + '\n' for q in dazu))


def bezug(kartenordner):
    from .fotovergleich import bezug_lesen
    p = os.path.join(kartenordner, 'omsi-editor-geo.cfg')
    return bezug_lesen(kartenordner) if os.path.isfile(p) else None


def _belegung(kartenordner, omsi, kacheln, log):
    """Belegungskarte (F4), bei Bedarf neu abgeleitet (netz.json zuerst) -> belegung.Belegung oder None"""
    from . import belegung, netz_datei
    try:
        b = belegung.Belegung.laden(kartenordner, pruefen=False)
        alt = set(b.veraltet())
    except (OSError, ValueError):
        b, alt = None, None
    if b is not None and not alt & {f'{k[0]}_{k[1]}' for k in kacheln}:
        return b
    try:
        try:
            if netz_datei.Netz.laden(kartenordner, pruefen=False).veraltet(kartenordner):
                raise OSError
        except (OSError, ValueError):
            log('    netz.json neu ableiten ...')
            netz_datei.schreiben(kartenordner, omsi)
        log('    Belegung neu berechnen ...')
        belegung.berechnen(kartenordner, omsi)
        return belegung.Belegung.laden(kartenordner, pruefen=False)
    except Exception as e:
        log(f'    Belegung nicht verfuegbar ({type(e).__name__}: {e}) - Boden ohne Strassenflaechen')
        return None


def bauen(kartenordner, omsi=None, kacheln=None, luftbild=True, mit_osm=True, cache=None, bild=None, log=print):
    """Malmasken der Kacheln (alle, wenn None) bauen und schreiben -> dict(kacheln, masken, anteile, quellen)"""
    from .utm import from_utm, to_utm
    omsi = omsi or schichten.STANDARD_OMSI
    cache = cache or GEODATEN
    alle = schichten.kacheln(kartenordner)
    kacheln = sorted(set(alle) if kacheln is None else set(kacheln) & set(alle))
    if not kacheln:
        raise ValueError('keine Kacheln')
    global_geaendert = groundtex_einrichten(kartenordner, omsi, log)
    bz = bezug(kartenordner)
    if bz is None:
        log('    Karte ohne Geobezug (omsi-editor-geo.cfg): Boden nur aus der Belegung')
    bel = _belegung(kartenordner, omsi, kacheln, log)
    quellen = set()
    merkmale = []
    if bz and mit_osm:
        ecken = [from_utm(bz['ost0'] + kx * KACHEL, bz['nord0'] + ky * KACHEL, bz['zone'])
                 for kx in (min(k[0] for k in kacheln), max(k[0] for k in kacheln) + 1)
                 for ky in (min(k[1] for k in kacheln), max(k[1] for k in kacheln) + 1)]
        lat = [e[0] for e in ecken]
        lon = [e[1] for e in ecken]
        try:
            js = osm_laden(min(lat) - 3e-4, min(lon) - 4e-4, max(lat) + 3e-4, max(lon) + 4e-4,
                           os.path.join(cache, 'osm_flaechen'))

            def nach_karte(la, lo):
                e, n = to_utm(la, lo, bz['zone'])
                return e - bz['ost0'], n - bz['nord0']
            merkmale = osm_merkmale(js, nach_karte)
            quellen.add(QUELLE_OSM)
            log(f'    OSM: {len(merkmale)} Flaechen und Wege')
        except Exception as e:
            log(f'    OSM-Flaechen nicht geladen ({type(e).__name__}: {e}) - ohne')
    masken_ordner = os.path.join(kartenordner, 'texture', 'map')
    os.makedirs(masken_ordner, exist_ok=True)
    d = _boden_lesen(kartenordner)
    d.update(format=FORMAT, version=VERSION, belaege=belaege(omsi), maske=MASKE, zelle_m=ZELLE, klassen=KLASSEN)
    ix = schichten.index_lesen(kartenordner)
    gesamt = np.zeros(len(KLASSEN))
    n_masken = 0
    bilder = {}
    for k in kacheln:
        dop = None
        if bz and luftbild:
            dop = _dop(bz, k, cache, log)
            if dop is not None:
                quellen.add(QUELLE_DOP)
        kontext = osm_k = osm_g = None
        if merkmale:
            kontext, osm_k, osm_g = merkmale_zeichnen(merkmale, k)
        braster = bel.raster(k) if bel is not None else None
        kl = klassifizieren(dop_klassen(dop) if dop is not None else None, kontext, osm_k, osm_g, braster)
        w = anteile(kl)
        alphas = deckkraft(w)
        for f in glob.glob(os.path.join(masken_ordner, f'tile_{k[0]}_{k[1]}.map.*.dds')):
            os.remove(f)                           # alte Masken der Kachel (eigene Schicht) weg
        for c, a in enumerate(alphas, 1):
            if int(a.max()) < 4:
                continue
            with open(os.path.join(masken_ordner, f'tile_{k[0]}_{k[1]}.map.{c}.dds'), 'wb') as f:
                f.write(dds_a8(a))
            n_masken += 1
        cnt = np.bincount(kl.ravel(), minlength=len(KLASSEN))[:len(KLASSEN)]
        gesamt += cnt
        key = f'{k[0]}_{k[1]}'
        d.setdefault('kacheln', {})[key] = dict(anteile={KLASSEN[i]: round(float(c / kl.size), 4) for i, c in enumerate(cnt) if c},
                                                luftbild=dop is not None, osm=bool(merkmale),
                                                erzeugt=datetime.datetime.now().isoformat(timespec='seconds'))
        m = schichten.manifest(kartenordner, 'boden', k, 'omsigen.boden',
                               dict(maske=MASKE, weich=WEICH, luftbild=dop is not None, osm=bool(merkmale)),
                               sorted(quellen))
        m['inhalt'] = masken_hash(kartenordner, k)
        m['objekte'] = 0
        ix['kacheln'].setdefault(key, {})['boden'] = m
        if bild:
            bilder[k] = vorschau(kl)
    d['quellen'] = sorted(set(d.get('quellen', [])) | quellen)
    _boden_schreiben(kartenordner, d)
    schichten.index_schreiben(kartenordner, ix)
    hinweise = []
    ad = addons(omsi)
    if ad:
        hinweise.append('Bodentexturen brauchen: ' + ', '.join(ad))
    quellen_eintragen(kartenordner, sorted(quellen), hinweise)
    if bild and bilder:
        _mosaik(bilder).save(bild)
    a = {KLASSEN[i]: round(float(v / max(1, gesamt.sum())), 4) for i, v in enumerate(gesamt)}
    log(f'    Boden: {len(kacheln)} Kacheln, {n_masken} Masken; ' + ', '.join(f'{n} {v * 100:.1f} %' for n, v in a.items() if v))
    return dict(kacheln=len(kacheln), masken=n_masken, anteile=a, quellen=sorted(quellen), global_geaendert=global_geaendert)


def neue_karte(kartenordner, omsi, log=print, geodaten=True):
    """Schritt in pipeline.erzeuge: Boden der neuen Karte bauen (nach netz.json und Belegung); geodaten False (Karte
    ohne Gelaende, Tests): ohne Luftbild und OSM-Flaechen, nur die Strassenflaechen. Fehler beenden die Erzeugung
    nicht - der Boden laesst sich mit python -m omsigen.boden nachholen"""
    try:
        bauen(kartenordner, omsi, luftbild=geodaten, mit_osm=geodaten, log=log)
    except Exception as e:
        log(f'    Boden nicht gebaut: {type(e).__name__}: {e}')


def _mosaik(bilder):
    from PIL import Image
    xs = [k[0] for k in bilder]
    ys = [k[1] for k in bilder]
    s = 150
    out = Image.new('RGB', ((max(xs) - min(xs) + 1) * s, (max(ys) - min(ys) + 1) * s), (30, 30, 30))
    for k, b in bilder.items():
        out.paste(b.resize((s, s)), ((k[0] - min(xs)) * s, (max(ys) - k[1]) * s))
    return out


def _dop(bz, k, cache, log):
    """Luftbild DOP20 der Kachel in 0,5 m (N x N x 3, Zeile 0 = Norden) oder None"""
    from PIL import Image
    from .luftbildvergleich import dop20
    e0, n0 = bz['ost0'] + k[0] * KACHEL, bz['nord0'] + k[1] * KACHEL
    try:
        f = dop20(e0, n0, e0 + KACHEL, n0 + KACHEL, N, N, os.path.join(cache, 'dop20'))
        a = np.asarray(Image.open(f).convert('RGB'))
    except Exception as e:
        log(f'    Luftbild {k[0]}_{k[1]} nicht geladen ({type(e).__name__})')
        return None
    if a.shape[:2] != (N, N) or float(a.std()) < 2.0:       # ausserhalb der Abdeckung: einfarbig
        return None
    return a


def _boden_lesen(kartenordner):
    p = os.path.join(kartenordner, DATEI)
    if os.path.isfile(p):
        try:
            with open(p, encoding='utf-8') as f:
                return json.load(f)
        except ValueError:
            pass
    return {}


def _boden_schreiben(kartenordner, d):
    p = os.path.join(kartenordner, DATEI)
    with open(p + '.tmp', 'w', encoding='utf-8') as f:
        json.dump(d, f, ensure_ascii=False, indent=1)
    os.replace(p + '.tmp', p)


def kachel_klassen(kartenordner, k, n_belaege=len(KLASSEN)):
    """sichtbarer Belag je Maskentexel der Kachel (aus den gespeicherten Masken) -> uint8 MASKE x MASKE oder None"""
    alphas = []
    groesse = None
    for c in range(1, n_belaege):
        p = os.path.join(kartenordner, 'texture', 'map', f'tile_{k[0]}_{k[1]}.map.{c}.dds')
        if os.path.isfile(p):
            a = dds_lesen(p)
            groesse = a.shape
            alphas.append(a)
        else:
            alphas.append(None)
    if groesse is None:
        return None
    alphas = [np.zeros(groesse, np.uint8) if a is None else a for a in alphas]
    return np.argmax(ueberblenden(alphas), axis=0).astype(np.uint8)


def main():
    ap = argparse.ArgumentParser(description='Boden einer Karte: Malmasken je Kachel (Gras, Erde, Schotter, Pflaster, '
                                             'Asphalt, Wasser) neu bauen')
    ap.add_argument('karte')
    ap.add_argument('--omsi', default=schichten.STANDARD_OMSI)
    ap.add_argument('--kacheln', nargs='*', help='nur diese Kacheln (kx_ky), Standard: alle')
    ap.add_argument('--ohne-luftbild', action='store_true', help='ohne DOP20-Klassifikation')
    ap.add_argument('--ohne-osm', action='store_true', help='ohne OSM-Flaechen')
    ap.add_argument('--cache', default=GEODATEN)
    ap.add_argument('--bild', help='Uebersicht der Belaege als PNG')
    a = ap.parse_args()
    d = a.karte if os.path.isdir(a.karte) else os.path.join(a.omsi, 'maps', a.karte)
    ks = None
    if a.kacheln:
        ks = [tuple(int(v) for v in s.replace(',', '_').split('_')) for s in a.kacheln]
    try:
        erg = bauen(d, a.omsi, ks, not a.ohne_luftbild, not a.ohne_osm, a.cache, a.bild,
                    log=lambda *x: print(*x, file=sys.stderr))
    except Exception as e:
        print(f'{type(e).__name__}: {e}', file=sys.stderr)
        sys.exit(1)
    print(json.dumps(erg, ensure_ascii=False))


if __name__ == '__main__':
    main()
