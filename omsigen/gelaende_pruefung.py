"""Gelaende ueber der Strasse finden: Kartengelaende (tile_*.map.terrain, wie OMSI in Dreiecken) gegen die Oberflaeche
der Strassensplines (quer ueber den ganzen Querschnitt) und der Kreuzungsplatten (Modellpunkte). Wo das Gelaende
hoeher liegt, verdeckt es im Spiel Fahrbahn oder Gehweg (Bodentextur auf der Strasse).

Beispiel:
  python -m omsigen.gelaende_pruefung Aschaffenburg_Linie16_v11 [--json build/gelaende.json]"""
import argparse, collections, json, math, os, sys

from .config import DEFAULT_OMSI
from .splinedb import SplineDB
from . import netz_datei as N
from .ausstattung import Gelaende
from . import ausstattung_modelle as AM

BS = chr(92)
SCHWELLE = 0.02          # m: so weit darf das Gelaende ueber der Oberflaeche liegen, bevor es zaehlt
FAHRBAHN, GEHWEG = 0.10, 0.25


def _oberflaeche(q, d):
    """Hoehe der Oberflaeche ueber der Spline-Hoehe bei Querlage d (rechts positiv); None ausserhalb"""
    if d < q['ol'] - 1e-6 or d > q['or_'] + 1e-6:
        return None
    return FAHRBAHN if q['cl'] - 1e-6 <= d <= q['cr'] + 1e-6 else GEHWEG


def splines_pruefen(kartenordner, omsi, gel, schritt=1.0, quer=0.5):
    sdb = SplineDB(omsi)
    sp = N.splines_lesen(kartenordner)
    funde = []
    for s in sp.values():
        try:
            q = dict(sdb[s['sli']])
        except KeyError:
            continue
        if not q.get('lanes') and not q.get('walks'):
            continue                                   # Begleit-Splines (Bruecke, Tunnel) ohne Pfade
        if 'invis' in s['sli'].lower():
            continue                                   # unsichtbare KI-Wendeschleifen
        if s['spiegel']:
            q = dict(q, cl=-q['cr'], cr=-q['cl'], ol=-q['or_'], or_=-q['ol'])
        n = max(2, int(s['L'] / schritt) + 1)
        for i in range(n):
            t = s['L'] * i / (n - 1)
            x, z, h = N.element_bei(s['x'], s['z'], s['h'], s['L'], s['R'], t)
            y = N.hoehe_bei(s, t)
            r = (math.cos(math.radians(h)), -math.sin(math.radians(h)))
            d = q['ol']
            while d <= q['or_'] + 1e-6:
                o = _oberflaeche(q, d)
                g = gel.hoehe(x + d * r[0], z + d * r[1])
                if g is not None and o is not None and g - (y + o) > SCHWELLE:
                    funde.append(dict(art='spline', id=s['id'], x=round(x + d * r[0], 1), z=round(z + d * r[1], 1),
                                      ueber=round(g - (y + o), 3), teil='fahrbahn' if o == FAHRBAHN else 'gehweg'))
                d += quer
    return funde


def platten_pruefen(kartenordner, omsi, gel):
    funde = []
    for o in N.kreuzungen_lesen(kartenordner):
        if o['art'] != 'kreuzung':
            continue
        sco = os.path.join(omsi, *o['rel'].split(BS))
        try:
            info = AM.sco_lesen(sco)
            pfad = AM.mesh_pfad(sco, info['meshes'][0])
            punkte, _, _ = AM.x_lesen(open(pfad, encoding='cp1252', errors='replace').read())
        except (OSError, IndexError, TypeError, ValueError):
            continue
        y0 = o.get('y', 0.0)
        for p in punkte:                               # x rechts (Ost), y hoch, z vor (Nord); Drehung 0
            g = gel.hoehe(o['x'] + p[0], o['z'] + p[2])
            if g is not None and g - (y0 + p[1]) > SCHWELLE:
                funde.append(dict(art='platte', id=o['id'], x=round(o['x'] + p[0], 1), z=round(o['z'] + p[2], 1),
                                  ueber=round(g - (y0 + p[1]), 3), teil='platte'))
    return funde


def pruefen(kartenordner, omsi=DEFAULT_OMSI):
    gel = Gelaende(kartenordner)
    f = splines_pruefen(kartenordner, omsi, gel) + platten_pruefen(kartenordner, omsi, gel)
    return f


def funde(kartenordner, omsi=DEFAULT_OMSI, ab=0.10):
    """fuer den Pruefbericht: je Spline/Platte ein Fund (hoechster Punkt), wenn das Gelaende >= ab m darueber liegt"""
    best = {}
    for x in pruefen(kartenordner, omsi):
        k = (x['art'], x['id'])
        if x['ueber'] >= ab and (k not in best or x['ueber'] > best[k]['ueber']):
            best[k] = x
    return [dict(art='gelaende_ueber_strasse', x=b['x'], z=b['z'],
                 text=f"{'Kreuzungsplatte' if b['art'] == 'platte' else 'Spline'} {b['id']} ({b['teil']}): "
                      f"Gelaende {b['ueber']:.2f} m darueber") for b in best.values()]


def zusammenfassung(f):
    st = collections.Counter((x['art'], x['teil']) for x in f)
    stellen = collections.Counter((x['art'], x['id']) for x in f)
    return dict(punkte=len(f), je_teil={f'{a}/{t}': n for (a, t), n in st.items()},
                elemente=len(stellen), max_ueber=max((x['ueber'] for x in f), default=0.0),
                ueber_10cm=sum(1 for x in f if x['ueber'] > 0.10), ueber_50cm=sum(1 for x in f if x['ueber'] > 0.50))


def main(argv=None):
    ap = argparse.ArgumentParser(description='Kartengelaende ueber Strassen und Kreuzungsplatten finden')
    ap.add_argument('karte')
    ap.add_argument('--omsi', default=DEFAULT_OMSI)
    ap.add_argument('--json')
    a = ap.parse_args(argv)
    d = a.karte if os.path.isdir(a.karte) else os.path.join(a.omsi, 'maps', a.karte)
    f = pruefen(d, a.omsi)
    print(json.dumps(zusammenfassung(f), ensure_ascii=False))
    if a.json:
        json.dump(f, open(a.json, 'w', encoding='utf-8'), ensure_ascii=False)


if __name__ == '__main__':
    sys.exit(main())
