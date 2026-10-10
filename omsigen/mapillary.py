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


# ------------------------------------------------------------------ Fotovergleich (Editor)
def pose(bild, ost0, nord0, zone=32):
    """Foto -> Pose in der Karte: x, y (Kartenpunkt), gier (Grad, im Uhrzeigersinn ab Gitter-Nord), neigung (Grad, oben
    positiv), roll (Grad, wie openOMSI: oben nach rechts positiv), vfov/hfov (Grad, des entzerrten Bilds)"""
    from .utm import to_utm
    lon, lat = bild['computed_geometry']['coordinates']
    e, n = to_utm(lat, lon, zone)
    R = drehung(bild['computed_rotation']).T
    f_, oben = R @ np.array([0, 0, 1.0]), -(R @ np.array([0, 1.0, 0]))
    # Meridiankonvergenz: Gitter-Nord ist gegen geografisch Nord gedreht
    gamma = math.degrees(math.radians(lon - (zone * 6 - 183)) * math.sin(math.radians(lat)))
    gier = (math.degrees(math.atan2(f_[0], f_[1])) - gamma) % 360
    neigung = math.degrees(math.asin(max(-1.0, min(1.0, f_[2]))))
    r0 = np.array([f_[1], -f_[0], 0.0])
    r0 /= max(np.linalg.norm(r0), 1e-12)
    u0 = np.cross(r0, f_)
    roll = math.degrees(math.atan2(oben @ r0, oben @ u0))
    w, h = bild['width'], bild['height']
    s, f = max(w, h), bild['camera_parameters'][0]
    return dict(id=str(bild['id']), x=round(e - ost0, 3), y=round(n - nord0, 3), gier=round(gier, 4), neigung=round(neigung, 4),
                roll=round(roll, 4), vfov=round(2 * math.degrees(math.atan(h / s / 2 / f)), 4),
                hfov=round(2 * math.degrees(math.atan(w / s / 2 / f)), 4), breite=w, hoehe=h,
                zeit=int(bild['captured_at']), sequenz=bild.get('sequence', ''))


def fotos(ost0, nord0, zone, bereich, cache=None):
    """perspektivische Fotos mit berechneter Pose im Kartenausschnitt (x0, y0, x1, y1) -> Liste Posen"""
    from .utm import from_utm
    x0, y0, x1, y1 = bereich
    la0, lo0 = from_utm(ost0 + x0, nord0 + y0, zone)
    la1, lo1 = from_utm(ost0 + x1, nord0 + y1, zone)
    # in Kacheln von hoechstens ~60 m abfragen: bei grossen Ausschnitten liefert die API nicht alle Bilder
    out, gesehen = [], set()
    nx = max(1, math.ceil((x1 - x0) / 60.0))
    ny = max(1, math.ceil((y1 - y0) / 60.0))
    roh = []
    for i in range(nx):
        for j in range(ny):
            ax, bx = x0 + (x1 - x0) * i / nx, x0 + (x1 - x0) * (i + 1) / nx
            ay, by = y0 + (y1 - y0) * j / ny, y0 + (y1 - y0) * (j + 1) / ny
            la_a, lo_a = from_utm(ost0 + ax, nord0 + ay, zone)
            la_b, lo_b = from_utm(ost0 + bx, nord0 + by, zone)
            for b in bilder((min(lo_a, lo_b), min(la_a, la_b), max(lo_a, lo_b), max(la_a, la_b)), cache):
                if b['id'] not in gesehen:
                    gesehen.add(b['id'])
                    roh.append(b)
    for b in roh:
        if b.get('camera_type') != 'perspective' or not b.get('camera_parameters') or b.get('is_pano'):
            continue
        p = pose(b, ost0, nord0, zone)
        if x0 <= p['x'] <= x1 and y0 <= p['y'] <= y1:
            out.append(p)
    out.sort(key=lambda p: (p['sequenz'], p['zeit']))
    return out


def entzerren(bild_rgb, camera_parameters):
    """Bild (H x W x 3) mit Verzeichnung k1, k2 -> Lochkamerabild mit derselben Brennweite (OpenSfM-Modell)"""
    h, w = bild_rgb.shape[:2]
    s = max(w, h)
    f, k1, k2 = (list(camera_parameters) + [0, 0, 0])[:3]
    yy, xx = np.mgrid[0:h, 0:w].astype(np.float64)
    xn, yn = (xx + 0.5 - w / 2) / s, (yy + 0.5 - h / 2) / s
    r2 = (xn / f) ** 2 + (yn / f) ** 2
    d = 1 + k1 * r2 + k2 * r2 * r2
    sx = np.clip((xn * d * s + w / 2 - 0.5).round().astype(int), 0, w - 1)
    sy = np.clip((yn * d * s + h / 2 - 0.5).round().astype(int), 0, h - 1)
    return bild_rgb[sy, sx]


def foto_datei(bild_id, cache):
    """entzerrtes Foto (JPEG, 2048 px) im Zwischenspeicher -> Pfad"""
    import io
    from PIL import Image
    ordner = os.path.join(cache, 'fotos')
    os.makedirs(ordner, exist_ok=True)
    datei = os.path.join(ordner, f'{bild_id}.jpg')
    if os.path.isfile(datei):
        return datei
    b = holen(f'/{bild_id}?fields=thumb_2048_url,thumb_1024_url,camera_parameters,camera_type', cache)
    url = b.get('thumb_2048_url') or b['thumb_1024_url']
    daten = urllib.request.urlopen(url, timeout=90).read()
    a = np.asarray(Image.open(io.BytesIO(daten)).convert('RGB'))
    if b.get('camera_type') == 'perspective' and b.get('camera_parameters'):
        a = entzerren(a, b['camera_parameters'])
    Image.fromarray(a).save(datei, quality=90)
    return datei


def main():
    import argparse
    import sys
    ap = argparse.ArgumentParser(description='Mapillary fuer den Editor (Fotovergleich), Ausgabe JSON')
    ap.add_argument('auftrag', choices=['fotos', 'bild'])
    ap.add_argument('--id')
    ap.add_argument('--ost0', type=float)
    ap.add_argument('--nord0', type=float)
    ap.add_argument('--zone', type=int, default=32)
    ap.add_argument('--bereich', type=float, nargs=4)
    ap.add_argument('--cache', default=os.path.join(os.environ.get('LOCALAPPDATA', '.'), 'omsi-editor', 'mapillary'))
    a = ap.parse_args()
    try:
        if a.auftrag == 'fotos':
            erg = dict(fotos=fotos(a.ost0, a.nord0, a.zone, a.bereich, a.cache), quelle='Mapillary-Mitwirkende, CC BY-SA 4.0')
        else:
            erg = dict(datei=foto_datei(a.id, a.cache))
    except Exception as ex:
        erg = dict(fehler=f'{type(ex).__name__}: {ex}')
    sys.stdout.write(json.dumps(erg))


if __name__ == '__main__':
    main()
