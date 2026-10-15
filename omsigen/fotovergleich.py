"""Fotovergleich ohne Fenster: Mapillary-Fotos rund um einen Sonderbau auswaehlen, die Karte aus genau ihrer Pose rendern
(omsi-editor --posen) und je Foto einen Vergleichsbogen schreiben (Foto | Render, Ueberblendung | Render-Kanten auf dem
Foto). Die Boegen sind zum Ansehen (Nutzer oder Claude) - daraus entstehen die Korrekturen am Bauplan.

Aufruf: python -m omsigen.fotovergleich --omsi <OMSI> --karte <Karte> [--sonderbau sonderbau_ring.json] [--max 80]
Quelle der Fotos: Mapillary-Mitwirkende, CC BY-SA 4.0.
"""
import argparse
import json
import math
import os
import subprocess
import sys

import numpy as np

from . import mapillary as M

EDITOR = os.environ.get('OMSIGEN_EDITOR') or os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), 'editor', 'target', 'release', 'omsi-editor.exe')
BREITE = 1024


class Boden:
    """Bodenhoehe in der Karte (NN - nn0): Kreuzungsflaeche aus dem Hoehenraster des Sonderbaus (ausser Trogoeffnungen),
    sonst DGM1 (echter Boden; das Modell hat keine Bruecken)"""

    def __init__(self, sonderbau, ost0, nord0, nn0, cache):
        from matplotlib.path import Path as MPath
        from .geodaten import Dgm1
        self.dgm = Dgm1(cache)
        self.o = (ost0, nord0, nn0)
        k = sonderbau.get('kontext', {})
        self.hr = k.get('hoehen')
        self.hw = np.array(self.hr['werte']).reshape(self.hr['ny'], self.hr['nx']) if self.hr and self.hr.get('werte') else None
        self.offen = [MPath(np.array(o)) for o in k.get('offen', [])]

    def __call__(self, x, y):
        x, y = np.atleast_1d(np.asarray(x, float)), np.atleast_1d(np.asarray(y, float))
        z = self.dgm.hoehen(x + self.o[0], y + self.o[1]) - self.o[2]
        if self.hw is None:
            return z
        hr = self.hr
        fx, fy = (x - hr['x0']) / hr['schritt'], (y - hr['y0']) / hr['schritt']
        inn = (fx >= 0) & (fy >= 0) & (fx < hr['nx'] - 1) & (fy < hr['ny'] - 1)
        if inn.any():
            pts = np.c_[x[inn], y[inn]]
            frei = np.zeros(len(pts), bool)
            for o in self.offen:
                frei |= o.contains_points(pts)
            i, j = np.floor(fx[inn]).astype(int), np.floor(fy[inn]).astype(int)
            u, v = fx[inn] - i, fy[inn] - j
            w = self.hw
            zp = w[j, i] * (1 - u) * (1 - v) + w[j, i + 1] * u * (1 - v) + w[j + 1, i] * (1 - u) * v + w[j + 1, i + 1] * u * v
            zi = z[inn]
            zi[~frei] = zp[~frei]
            z[inn] = zi
        return z


def bezug_lesen(karte_dir):
    """omsi-editor-geo.cfg der Karte -> dict(ost0, nord0, zone, nn0)"""
    w = {}
    for z in open(os.path.join(karte_dir, 'omsi-editor-geo.cfg'), encoding='utf-8'):
        if '=' in z and not z.lstrip().startswith('#'):
            k, v = z.split('=', 1)
            w[k.strip()] = v.strip()
    return dict(ost0=float(w['ost0']), nord0=float(w['nord0']), zone=int(float(w['utm_zone'])), nn0=float(w.get('nn0', 0)))


def auswahl(fotos, ab_jahr=2023, zelle=10.0, sektoren=8, maximal=80):
    """je Zelle (zelle m) und Blickrichtung (sektoren) das neueste Foto; die meistbesuchten Stellen zuerst"""
    beste = {}
    for f in fotos:
        jahr = 1970 + f['zeit'] / 1000 / 86400 / 365.2425
        if jahr < ab_jahr:
            continue
        k = (int(f['x'] // zelle), int(f['y'] // zelle), int(((f['gier'] + 180 / sektoren) % 360) // (360 / sektoren)))
        if k not in beste or f['zeit'] > beste[k]['zeit']:
            beste[k] = f
    out = sorted(beste.values(), key=lambda f: -f['zeit'])
    return out[:maximal]


def rendern(omsi, karte, liste, sonderbau=None):
    """omsi-editor --posen (mit --sonderbau: aktueller Stand des Sonderbaus nur in der Sitzung eingesetzt)"""
    cmd = [EDITOR, '--root', omsi, karte, '--posen', liste]
    if sonderbau:
        cmd += ['--sonderbau', sonderbau]
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        raise SystemExit(f'Rendern fehlgeschlagen: {r.stderr[-800:]}')


# ------------------------------------------------------------------ Feinausrichtung
KLEIN = 320


def kanten(grau, schwelle=None, prozent=85):
    """Kantenbild (bool) aus Grauwerten: Sobel-Betrag ueber Schwelle bzw. Perzentil; unteres Siebtel (Motorhaube) aus"""
    from scipy import ndimage
    g = grau.astype(np.float64)
    m = np.hypot(ndimage.sobel(g, 0), ndimage.sobel(g, 1))
    t = schwelle if schwelle is not None else np.percentile(m, prozent)
    k = m > t
    k[int(k.shape[0] * 0.86):] = False
    return k


def masken(rgb):
    """Farbflaechen am Boden (untere Bildhaelfte ohne Motorhaube): rot (Radstreifen/-furt), weiss (Markierungen)"""
    a = rgb.astype(np.int32)
    R, G, B = a[..., 0], a[..., 1], a[..., 2]
    h = a.shape[0]
    unten = np.zeros(R.shape, bool)
    unten[int(h * 0.45):int(h * 0.86)] = True
    rot = (R > G + 35) & (R > B + 30) & (R > 90) & unten
    hell = (R + G + B) / 3
    sat = a.max(axis=2) - a.min(axis=2)
    ref = np.percentile(hell[unten], 50) if unten.any() else 128
    weiss = (hell > ref + 55) & (sat < 45) & unten
    return rot, weiss


def foto_merkmale(datei, breite, hoehe):
    from PIL import Image
    from scipy import ndimage
    im = Image.open(datei).convert('RGB').resize((breite, hoehe))
    k = kanten(np.asarray(im.convert('L')))
    rot, weiss = masken(np.asarray(im))
    # Flaechen leicht verbreitern: kleine Restfehler (Verzeichnung, Unschaerfe) sollen nicht zaehlen
    return dict(abstand=np.minimum(ndimage.distance_transform_edt(~k), 8.0), rot=ndimage.binary_dilation(rot, iterations=1),
                weiss=ndimage.binary_dilation(weiss, iterations=1))


def _iou(a, b):
    u = (a | b).sum()
    return 1.0 if u < 40 else float((a & b).sum() / u)


def guete(m, render_datei):
    """kleiner = besser: fehlende Deckung der roten und weissen Flaechen plus ein Zehntel Kantenabstand (Pixel)"""
    from PIL import Image
    im = np.asarray(Image.open(render_datei).convert('RGB'))
    k = kanten(im.mean(axis=2), schwelle=60.0)
    if int(k.sum()) < 150:
        return 99.0
    rot, weiss = masken(im)
    return 2.0 * (1 - _iou(m['rot'], rot)) + 1.5 * (1 - _iou(m['weiss'], weiss)) + 0.1 * float(m['abstand'][k].mean())


def varianten(f, stufe, beste):
    """Posenvarianten um die beste bisherige: (rechts, vor, d_gier, d_neigung, d_roll, hoehe)"""
    r0, v0, g0, n0, ro0, h0 = beste
    if stufe == 0:
        return [(r, v0, g, n0, ro0, h0) for r in (-5, -4, -3, -2, -1, 0, 1, 2, 3, 4, 5) for g in (-4, -2, 0, 2, 4)]
    if stufe == 1:
        return [(r0, v, g0, n, ro0, h) for v in (-2, 0, 2) for n in (-1.5, 0, 1.5) for h in (1.2, 1.5, 1.9)]
    return [(r0 + r, v0, g0 + g, n0, ro0 + ro, h0) for r in (-0.5, 0, 0.5) for g in (-0.6, 0, 0.6) for ro in (-1, 0, 1)]


def pose_aus(f, v, boden=None):
    """Pose der Variante; Hoehe als 'ueber' (m ueber der gerenderten Oberflaeche, der Renderer setzt sie)"""
    r, vor, dg, dn, dro, h = v
    g = math.radians(f['gier'])
    x = f['x'] + math.cos(g) * r + math.sin(g) * vor
    y = f['y'] - math.sin(g) * r + math.cos(g) * vor
    return dict(x=x, y=y, ueber=h, gier=f['gier'] + dg, neigung=f['neigung'] + dn, roll=f['roll'] + dro, vfov=f['vfov'])


def ausrichten(wahl, boden, omsi, karte, aus, mcache, sonderbau=None):
    """je Foto die Pose suchen, deren Render-Kanten am besten auf den Fotokanten liegen (drei Stufen, je Stufe ein
    Renderlauf fuer alle Fotos) -> wahl mit 'ausgerichtet' (Pose) und 'guete' (vorher, nachher)"""
    tmp = os.path.abspath(os.path.join(aus, 'ausrichten'))
    os.makedirs(tmp, exist_ok=True)
    abst, beste, start = {}, {}, {}
    for f in wahl:
        hk = int(round(KLEIN * f['hoehe'] / f['breite']))
        f['_hk'] = hk
        abst[f['id']] = foto_merkmale(M.foto_datei(f['id'], mcache), KLEIN, hk)
        beste[f['id']] = ((0, 0, 0, 0, 0, 1.5), None)
    for stufe in range(3):
        posen, wer = [], []
        for f in wahl:
            for k, v in enumerate(varianten(f, stufe, beste[f['id']][0])):
                p = pose_aus(f, v)
                p.update(datei=os.path.join(tmp, f"{f['id']}_{stufe}_{k}.png"), breite=KLEIN, hoehe=f['_hk'])
                posen.append(p)
                wer.append((f['id'], v, p['datei']))
        liste = os.path.join(tmp, f'posen_{stufe}.json')
        json.dump(posen, open(liste, 'w', encoding='utf-8'))
        rendern(omsi, karte, liste, sonderbau)
        for fid, v, datei in wer:
            q = guete(abst[fid], datei)
            if v == (0, 0, 0, 0, 0, 1.5):
                start[fid] = q
            if beste[fid][1] is None or q < beste[fid][1]:
                beste[fid] = (v, q)
            os.remove(datei)
        print(f'Ausrichten Stufe {stufe + 1}: {len(posen)} Bilder')
    for f in wahl:
        v, q = beste[f['id']]
        f['ausgerichtet'] = pose_aus(f, v)
        f['versatz'] = dict(rechts=v[0], vor=v[1], gier=v[2], neigung=v[3], roll=v[4], hoehe=v[5])
        f['guete'] = (round(start.get(f['id'], 99.0), 2), round(q, 2))
        print(f"  {f['id']}: rechts {v[0]:+.1f} vor {v[1]:+.1f} Gier {v[2]:+.1f} Neigung {v[3]:+.1f} Hoehe {v[5]:.1f}  Guete {f['guete'][0]} -> {f['guete'][1]}")
    return wahl


def bogen(foto_datei, render_datei, ziel, titel):
    """Vergleichsbogen 2 x 2: Foto | Render / Ueberblendung | Render-Kanten (rot) auf dem Foto"""
    from PIL import Image, ImageDraw, ImageFilter, ImageChops
    f = Image.open(foto_datei).convert('RGB')
    r = Image.open(render_datei).convert('RGB')
    f = f.resize(r.size)
    w, h = r.size
    blend = Image.blend(f, r, 0.5)
    kanten = r.convert('L').filter(ImageFilter.FIND_EDGES).point(lambda v: 255 if v > 40 else 0)
    rot = Image.new('RGB', r.size, (255, 40, 40))
    kant = f.copy()
    kant.paste(rot, (0, 0), kanten)
    out = Image.new('RGB', (w * 2, h * 2 + 28), (20, 20, 20))
    for i, im in enumerate((f, r, blend, kant)):
        out.paste(im, ((i % 2) * w, 28 + (i // 2) * h))
    d = ImageDraw.Draw(out)
    d.text((8, 7), titel + '   (Foto: Mapillary-Mitwirkende, CC BY-SA 4.0)', fill=(255, 255, 255))
    out.save(ziel, quality=88)


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--omsi', default=r'C:\Program Files (x86)\Steam\steamapps\common\OMSI 2')
    ap.add_argument('--karte', required=True)
    ap.add_argument('--sonderbau', default=None, help='sonderbau_ring.json (Vorgabe: Objektordner der Karte)')
    ap.add_argument('--bereich', type=float, nargs=4, default=None, help='x0 y0 x1 y1 (Vorgabe: Sonderbau + 15 m)')
    ap.add_argument('--max', type=int, default=80)
    ap.add_argument('--ab', type=int, default=2023, help='Fotos ab Jahr')
    ap.add_argument('--hoehe', type=float, default=1.5, help='Kamera ueber dem Boden, m')
    ap.add_argument('--ids', default=None, help='nur diese Foto-IDs (Komma)')
    ap.add_argument('--ausrichten', action='store_true', help='Fotopose automatisch feinausrichten (Kanten Foto/Render)')
    ap.add_argument('--wie-karte', action='store_true', help='die Karte rendern, wie sie gespeichert ist (Sonderbau nicht neu einsetzen)')
    ap.add_argument('--aus', default=os.path.join('build', 'fotovergleich'))
    ap.add_argument('--cache', default=os.path.join(os.environ.get('LOCALAPPDATA', '.'), 'omsi-editor'))
    a = ap.parse_args()

    karte_dir = os.path.join(a.omsi, 'maps', a.karte)
    b = bezug_lesen(karte_dir)
    sb_pfad = a.sonderbau or os.path.join(a.omsi, 'Sceneryobjects', 'Aschaffenburg', a.karte, 'Sonderbau_Ring', 'sonderbau_ring.json')
    sb = json.load(open(sb_pfad, encoding='utf-8')) if os.path.isfile(sb_pfad) else {}
    # den aktuellen Bau zeigen, auch wenn er noch nicht per "Anwenden" in die Karte gesetzt ist
    sb_einsetzen = sb_pfad if (os.path.isfile(sb_pfad) and not a.wie_karte) else None
    bereich = a.bereich
    if bereich is None:
        k_ = sb.get('kontext', {})
        P = np.array([q for st in k_.get('strassen', []) for q in st['punkte']] + [e['p'] for e in k_.get('stationen', [])] or [[1084, -266]])
        bereich = [P[:, 0].min() - 10, P[:, 1].min() - 10, P[:, 0].max() + 10, P[:, 1].max() + 10]
    mcache = os.path.join(a.cache, 'mapillary')
    alle = M.fotos(b['ost0'], b['nord0'], b['zone'], bereich, mcache)
    if a.ids:
        ids = set(a.ids.split(','))
        wahl = [f for f in alle if f['id'] in ids]
    else:
        wahl = auswahl(alle, a.ab, maximal=a.max)
    print(f'{len(alle)} Fotos im Bereich, {len(wahl)} gewaehlt')
    os.makedirs(a.aus, exist_ok=True)
    boden = None
    if a.ausrichten:
        wahl = ausrichten(wahl, boden, a.omsi, a.karte, a.aus, mcache, sb_einsetzen)
    posen, ok = [], []
    for f in wahl:
        if 'ausgerichtet' in f:
            p = dict(f['ausgerichtet'])
        else:
            p = dict(x=f['x'], y=f['y'], ueber=a.hoehe, gier=f['gier'], neigung=f['neigung'], roll=f['roll'], vfov=f['vfov'])
        hoehe = int(round(BREITE * f['hoehe'] / f['breite']))
        p.update(datei=os.path.abspath(os.path.join(a.aus, f"{f['id']}_render.png")), breite=BREITE, hoehe=hoehe)
        posen.append(p)
        ok.append(f)
    wahl = ok
    liste = os.path.abspath(os.path.join(a.aus, 'posen.json'))
    json.dump(posen, open(liste, 'w', encoding='utf-8'))
    rendern(a.omsi, a.karte, liste, sb_einsetzen)
    index = []
    for f, p in zip(wahl, posen):
        try:
            fd = M.foto_datei(f['id'], mcache)
        except Exception as ex:
            print(f"Foto {f['id']}: {ex}")
            continue
        jahr = 1970 + f['zeit'] / 1000 / 86400 / 365.2425
        titel = f"Foto {f['id']}  x {p['x']:.1f} y {p['y']:.1f}  Blick {p['gier']:.0f} Grad  {int(jahr)}"
        if 'guete' in f:
            v = f['versatz']
            titel += f"   ausgerichtet: rechts {v['rechts']:+.1f} vor {v['vor']:+.1f} m, Gier {v['gier']:+.1f}, Hoehe {v['hoehe']:.1f} m, Abweichung {f['guete'][0]} -> {f['guete'][1]}"
        ziel = os.path.join(a.aus, f"{f['id']}.jpg")
        bogen(fd, p['datei'], ziel, titel)
        index.append(dict({k: v for k, v in f.items() if not k.startswith('_')}, bogen=ziel))
    json.dump(index, open(os.path.join(a.aus, 'index.json'), 'w', encoding='utf-8'), indent=1)
    print(f'{len(index)} Boegen in {a.aus}')


if __name__ == '__main__':
    main()
