"""Mapillary (CC BY-SA 4.0, Quelle: Mapillary-Mitwirkende): Bilder, Kameraposen, Erkennungen und Kartenobjekte fuer die
Bauaufnahme. Zugang ueber den Client-Token in `mapillary_token.txt` (nicht eingecheckt) oder MAPILLARY_TOKEN.

- bilder(bbox): Bilder mit berechneter Lage, Blickrichtung, Kamera
- erkennungen(bild): Segmente je Klasse als Polygone in Bildkoordinaten (0..1)
- kartenobjekte(bbox): von Mapillary triangulierte Punktobjekte (Ampeln, Schilder, Haltlinien, Furten ...)
- strahl(bild, u, v): Blickstrahl eines Bildpunkts in Kartenrichtung (Ost, Nord, oben)
"""
import base64
import json
import math
import os
import urllib.request

import numpy as np

API = 'https://graph.mapillary.com'
FELDER_BILD = ('id,captured_at,camera_type,camera_parameters,computed_rotation,computed_altitude,width,height,'
               'computed_geometry,computed_compass_angle,sequence,is_pano')


def token():
    t = os.environ.get('MAPILLARY_TOKEN')
    if t:
        return t.strip()
    for d in (os.getcwd(), os.path.dirname(os.path.dirname(os.path.abspath(__file__)))):
        p = os.path.join(d, 'mapillary_token.txt')
        if os.path.isfile(p):
            return open(p, encoding='utf-8').read().strip()
    raise RuntimeError('kein Mapillary-Token (mapillary_token.txt oder MAPILLARY_TOKEN)')


def holen(pfad, cache=None):
    """GET auf die Graph-API (Token im Kopf, nicht in der URL); mit Zwischenspeicher"""
    if cache:
        os.makedirs(cache, exist_ok=True)
        datei = os.path.join(cache, ''.join(c if c.isalnum() else '_' for c in pfad)[:180] + '.json')
        if os.path.isfile(datei):
            return json.load(open(datei, encoding='utf-8'))
    r = urllib.request.Request(API + pfad, headers={'Authorization': 'OAuth ' + token()})
    j = json.load(urllib.request.urlopen(r, timeout=90))
    if cache:
        json.dump(j, open(datei, 'w', encoding='utf-8'))
    return j


def bilder(bbox, cache=None):
    """bbox (lon0, lat0, lon1, lat1) -> Liste Bilder (nur mit berechneter Lage)"""
    b = ','.join(f'{x:.6f}' for x in bbox)
    d = holen(f'/images?fields={FELDER_BILD}&bbox={b}&limit=2000', cache)['data']
    return [x for x in d if x.get('computed_geometry') and x.get('computed_rotation')]


def kartenobjekte(bbox, cache=None):
    b = ','.join(f'{x:.6f}' for x in bbox)
    return holen(f'/map_features?fields=id,object_value,geometry,first_seen_at,last_seen_at&bbox={b}&limit=2000', cache)['data']


# ------------------------------------------------------------------ Erkennungen (Vektor-Kachel, base64)
def _varint(b, i):
    v = s = 0
    while True:
        c = b[i]
        i += 1
        v |= (c & 0x7F) << s
        s += 7
        if c < 0x80:
            return v, i


def _felder(b):
    """Protobuf -> [(Nummer, Wert)] (Wert: int oder bytes)"""
    i, out = 0, []
    while i < len(b):
        k, i = _varint(b, i)
        nr, typ = k >> 3, k & 7
        if typ == 0:
            v, i = _varint(b, i)
        elif typ == 2:
            n, i = _varint(b, i)
            v, i = b[i:i + n], i + n
        elif typ == 5:
            v, i = b[i:i + 4], i + 4
        elif typ == 1:
            v, i = b[i:i + 8], i + 8
        else:
            raise ValueError(f'Protobuf-Typ {typ}')
        out.append((nr, v))
    return out


def _zz(n):
    return (n >> 1) ^ -(n & 1)


def kachel_polygone(geom_b64):
    """Mapillary-Erkennung (Vektor-Kachel) -> Liste Ringe [(u, v)] in 0..1 (u nach rechts, v nach unten)"""
    ringe = []
    for nr, layer in _felder(base64.b64decode(geom_b64)):
        if nr != 3:
            continue
        lf = _felder(layer)
        extent = next((v for n, v in lf if n == 5), 4096)
        for n, feat in lf:
            if n != 2:
                continue
            geo = next((v for k, v in _felder(feat) if k == 4), b'')
            ints, j = [], 0
            while j < len(geo):
                v, j = _varint(geo, j)
                ints.append(v)
            x = y = 0
            k, ring = 0, []
            while k < len(ints):
                cmd, cnt = ints[k] & 7, ints[k] >> 3
                k += 1
                if cmd == 7:
                    if ring:
                        ringe.append(ring)
                    ring = []
                    continue
                for _ in range(cnt):
                    x += _zz(ints[k])
                    y += _zz(ints[k + 1])
                    k += 2
                    if cmd == 1 and ring:
                        ringe.append(ring)
                        ring = []
                    ring.append((x / extent, y / extent))
            if ring:
                ringe.append(ring)
    return ringe


def erkennungen(bild_id, cache=None):
    """-> [(Klasse, [Ringe])]"""
    d = holen(f'/{bild_id}/detections?fields=value,geometry', cache)['data']
    return [(x['value'], kachel_polygone(x['geometry'])) for x in d if x.get('geometry')]


# ------------------------------------------------------------------ Kamera (OpenSfM)
def drehung(aa):
    """Achse-Winkel -> Drehmatrix (Welt ENU -> Kamera, x rechts, y unten, z vorn)"""
    a = np.asarray(aa, float)
    t = np.linalg.norm(a)
    if t < 1e-12:
        return np.eye(3)
    k = a / t
    K = np.array([[0, -k[2], k[1]], [k[2], 0, -k[0]], [-k[1], k[0], 0]])
    return np.eye(3) + math.sin(t) * K + (1 - math.cos(t)) * K @ K


def strahl(bild, u, v):
    """Bildpunkt (u, v in 0..1) -> Einheitsvektor (Ost, Nord, oben); perspektivisch mit k1, k2 (OpenSfM)"""
    w, h = bild['width'], bild['height']
    s = max(w, h)
    f, k1, k2 = (bild['camera_parameters'] + [0, 0, 0])[:3]
    xd, yd = (u * w - w / 2) / s, (v * h - h / 2) / s
    # Verzeichnung umkehren (iterativ)
    x, y = xd / f, yd / f
    for _ in range(10):
        r2 = x * x + y * y
        d = 1 + k1 * r2 + k2 * r2 * r2
        x, y = xd / f / d, yd / f / d
    r = drehung(bild['computed_rotation']).T @ np.array([x, y, 1.0])
    return r / np.linalg.norm(r)
