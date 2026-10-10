"""Luftbildvergleich: die Karte senkrecht von oben rendern (omsi-editor --posen) und neben/ueber das amtliche Luftbild
DOP20 (Bayerische Vermessungsverwaltung, CC BY 4.0) desselben Ausschnitts legen. Fuer alles am Boden (Markierungen,
Radstreifen, Inseln, Borde) genauer als der Fotovergleich, weil keine Fotopose im Spiel ist.

Aufruf: python -m omsigen.luftbildvergleich --karte <Karte> --bereich x0 y0 x1 y1 [--z 10] [--aus build/luftbildvergleich]
"""
import argparse
import io
import json
import math
import os
import subprocess

DOP20 = 'https://geoservices.bayern.de/od/wms/dop/v1/dop20'
M_PRO_PX = float(os.environ.get("M_PRO_PX", 0.1))


def dop20(e0, n0, e1, n1, breite, hoehe, cache):
    datei = os.path.join(cache, f'dop20_{e0:.1f}_{n0:.1f}_{e1:.1f}_{n1:.1f}_{breite}x{hoehe}.jpg')
    if not os.path.isfile(datei):
        import requests
        r = requests.get(DOP20, params=dict(SERVICE='WMS', REQUEST='GetMap', VERSION='1.3.0', LAYERS='by_dop20c', STYLES='',
                                            CRS='EPSG:25832', BBOX=f'{e0},{n0},{e1},{n1}', WIDTH=breite, HEIGHT=hoehe,
                                            FORMAT='image/jpeg'), timeout=90)
        r.raise_for_status()
        os.makedirs(cache, exist_ok=True)
        open(datei, 'wb').write(r.content)
    return datei


def main():
    from PIL import Image, ImageDraw
    from .fotovergleich import bezug_lesen, rendern
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--omsi', default=r'C:\Program Files (x86)\Steam\steamapps\common\OMSI 2')
    ap.add_argument('--karte', required=True)
    ap.add_argument('--bereich', type=float, nargs=4, required=True)
    ap.add_argument('--z', type=float, default=10.0, help='Hoehe der Flaeche in der Karte (Kamera steht 800 m darueber)')
    ap.add_argument('--aus', default=os.path.join('build', 'luftbildvergleich'))
    ap.add_argument('--name', default='ausschnitt')
    ap.add_argument('--cache', default=os.path.join(os.environ.get('LOCALAPPDATA', '.'), 'omsi-editor', 'geodaten', 'dop20'))
    a = ap.parse_args()
    b = bezug_lesen(os.path.join(a.omsi, 'maps', a.karte))
    x0, y0, x1, y1 = a.bereich
    w, h = int(round((x1 - x0) / M_PRO_PX)), int(round((y1 - y0) / M_PRO_PX))
    os.makedirs(a.aus, exist_ok=True)
    aus = os.path.abspath(a.aus)
    abstand = 800.0
    render = os.path.join(aus, f'{a.name}_render.png')
    posen = [dict(datei=render, x=(x0 + x1) / 2, y=(y0 + y1) / 2, z=a.z + abstand, gier=0.0, neigung=-89.99, roll=0.0,
                  vfov=2 * math.degrees(math.atan((y1 - y0) / 2 / abstand)), breite=w, hoehe=h)]
    pl = os.path.join(aus, f'{a.name}_posen.json')
    json.dump(posen, open(pl, 'w', encoding='utf-8'))
    sb = os.path.join(a.omsi, 'Sceneryobjects', 'Aschaffenburg', a.karte, 'Sonderbau_Ring', 'sonderbau_ring.json')
    rendern(a.omsi, a.karte, pl, sb if os.path.isfile(sb) else None)
    luft = dop20(b['ost0'] + x0, b['nord0'] + y0, b['ost0'] + x1, b['nord0'] + y1, w, h, a.cache)
    ri = Image.open(render).convert('RGB')
    li = Image.open(luft).convert('RGB').resize(ri.size)
    # Raster alle 10 m mit Beschriftung (Kartenkoordinaten), damit Abweichungen sich verorten lassen
    for im in (ri, li):
        d = ImageDraw.Draw(im)
        for gx in range(int(math.ceil(x0 / 10)) * 10, int(x1) + 1, 10):
            px = (gx - x0) / M_PRO_PX
            d.line([(px, 0), (px, h)], fill=(255, 255, 0), width=1)
            d.text((px + 3, 3), f'{gx}', fill=(255, 255, 0))
        for gy in range(int(math.ceil(y0 / 10)) * 10, int(y1) + 1, 10):
            py = (y1 - gy) / M_PRO_PX
            d.line([(0, py), (w, py)], fill=(255, 255, 0), width=1)
            d.text((3, py + 3), f'{gy}', fill=(255, 255, 0))
    out = Image.new('RGB', (w * 2, h + 24), (20, 20, 20))
    out.paste(li, (0, 24))
    out.paste(ri, (w, 24))
    ImageDraw.Draw(out).text((6, 5), 'links: Luftbild DOP20 (c) Bayerische Vermessungsverwaltung, CC BY 4.0   rechts: Karte', fill=(255, 255, 255))
    # Linien des Sonderbaus (Strassen, Wandlinien) ins Luftbild: welches Teil liegt daneben?
    sbp = os.path.join(a.omsi, 'Sceneryobjects', 'Aschaffenburg', a.karte, 'Sonderbau_Ring', 'sonderbau_ring.json')
    if os.path.isfile(sbp):
        k = json.load(open(sbp, encoding='utf-8')).get('kontext', {})
        lk = li.copy()
        d = ImageDraw.Draw(lk)
        bild = lambda q: ((q[0] - x0) / M_PRO_PX, (y1 - q[1]) / M_PRO_PX)
        for st in k.get('strassen', []):
            pts = [bild(q) for q in st['punkte']]
            d.line(pts, fill=(80, 170, 255), width=2)
            inn = [q for q in pts if 0 <= q[0] < w and 0 <= q[1] < h]
            if inn:
                d.text(inn[len(inn) // 2], st['name'], fill=(120, 200, 255))
        stn = k.get('stationen', [])
        farben = {'trenn_w': (255, 120, 0), 'trenn_o': (255, 120, 0), 'aussen_w': (255, 0, 160), 'aussen_o': (255, 0, 160)}
        for art, f_ in farben.items():
            pts = [bild((s_['p'][0] + s_['q'][0] * s_['t'][art], s_['p'][1] + s_['q'][1] * s_['t'][art])) for s_ in stn if art in s_.get('t', {})]
            if len(pts) > 1:
                d.line(pts, fill=f_, width=2)
        d.line([bild(s_['p']) for s_ in stn], fill=(255, 255, 255), width=1)
        lk.save(os.path.join(aus, f'{a.name}_linien.jpg'), quality=90)
    ziel = os.path.join(aus, f'{a.name}.jpg')
    out.save(ziel, quality=90)
    Image.blend(li, ri, 0.5).save(os.path.join(aus, f'{a.name}_ueberblendet.jpg'), quality=90)
    print(ziel)


if __name__ == '__main__':
    main()
