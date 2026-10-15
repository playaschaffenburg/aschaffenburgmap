"""Kreuzungen am Gefaelle (docs/musterbuch.md, Abschnitt 2): Platte als Ebene, Arme ohne Knick, Ausrundung mit
[spline_h], geneigte Kreuzungsmodelle und -pfade."""
import math
import os

from omsigen import kreuzung, netz_datei, writer
from omsigen.hoehen import Hoehen, hoehe_im_element
from omsigen.network import build
from omsigen.splinedb import SplineDB

from .test_belegung import SLI


class Hang:
    def __init__(self, f):
        self.f = f

    def hoehe(self, x, z):
        return self.f(x, z)


def _kreuz(gel, nord_sued=True):
    """Hauptstrasse West-Ost (x -200..200, z 0), Nebenstrasse von der Mitte nach Norden (und Sueden)"""
    ways = [dict(tags=dict(highway='secondary', name='A'), P=[(-200.0, 0.0), (0.0, 0.0), (200.0, 0.0)]),
            dict(tags=dict(highway='residential', name='B'), P=[(0.0, 0.0), (0.0, 150.0)])]
    if nord_sued:
        ways.append(dict(tags=dict(highway='residential', name='C'), P=[(0.0, -150.0), (0.0, 0.0)]))
    sdb = SplineDB()
    net = build(ways, sdb)
    junctions = kreuzung.build_objects(net, sdb, 'T')
    hoe = Hoehen(Hang(gel), log=lambda *a: None).berechnen(net, sdb, junctions)
    return net, hoe, junctions


def _an_kreuzung(net, hoe):
    """je Arm: (Steigung an der Platte in Kettenrichtung, Plattenneigung in Kettenrichtung, Kette, Ende 0/-1)"""
    out = []
    for ch in net['road_chains']:
        for kn, i in ((ch.get('node_s'), 0), (ch.get('node_e'), -1)):
            if kn not in hoe.ebenen:
                continue
            _, _, _, gx, gz = hoe.ebenen[kn]
            e = ch['els'][i]
            h = math.radians(e[2] if i == 0 else (e[2] + (math.degrees(e[3] / e[4]) if e[4] else 0.0)))
            soll = (gx * math.sin(h) + gz * math.cos(h)) * 100
            ist = ch['g01'][i][0 if i == 0 else 1]
            out.append((ist, soll, ch, i))
    return out


def test_durchgehendes_gefaelle_kippt_die_platte():
    net, hoe, _ = _kreuz(lambda x, z: 100 + 0.08 * x)             # 8 % nach Osten
    (_, _, h0, gx, gz), = hoe.ebenen.values()
    assert gx > 0.05 and abs(gz) < 0.01, (gx, gz)                  # Platte folgt der Hauptstrasse
    for ist, soll, ch, i in _an_kreuzung(net, hoe):
        assert abs(ist - soll) < 1e-6                              # kein Knick an der Plattenkante
    for ch in net['road_chains']:                                  # Ausrundung: Steigung stetig, Hoehen exakt
        for i in range(len(ch['els']) - 1):
            assert abs(ch['g01'][i][1] - ch['g01'][i + 1][0]) < 1e-9
            y1 = hoehe_im_element(ch['y'][i], *ch['g01'][i], ch['dh'][i], ch['els'][i][3], ch['els'][i][3])
            assert abs(y1 - ch['y'][i + 1]) < 1e-9


def test_nebenstrasse_am_hang_rundet_aus():
    # Hauptstrasse eben, nur nach Norden steigt das Gelaende mit 8 %: Platte bleibt fast eben, der Nordarm setzt
    # flach an und erreicht die Hangsteigung erst in einigem Abstand (wie Ahlheim/Staedtedreieck)
    net, hoe, _ = _kreuz(lambda x, z: 100 + 0.08 * max(0.0, z), nord_sued=False)
    (_, _, _, gx, gz), = hoe.ebenen.values()
    assert math.hypot(gx, gz) < 0.03
    nord = [(ist, ch, i) for ist, soll, ch, i in _an_kreuzung(net, hoe) if ch['els'][0][2] % 360 < 1 or
            abs(ch['els'][-1][2] % 360 - 180) < 1]
    assert nord
    ist, ch, i = nord[0]
    g = [max(abs(a), abs(b)) for a, b in (ch['g01'] if i == 0 else ch['g01'][::-1])]
    assert abs(ist) < 3.0 and max(g) > 5.0, (ist, g[:6])


def test_geneigte_platte_modell_und_pfade():
    net, hoe, junctions = _kreuz(lambda x, z: 100 + 0.08 * x)
    j, = junctions
    gx, gz = j['neigung']
    assert kreuzung.ebene_anwenden(j)
    V, F = j['mesh']
    for v in V:
        if v[4] > 0.5:                                             # Oberflaechen liegen in der Ebene
            assert abs(v[1] - (kreuzung.ASPH_H + gx * v[0] + gz * v[2])) < 1e-6 or \
                abs(v[1] - (kreuzung.WALK_H + gx * v[0] + gz * v[2])) < 1e-6
    z = j['sco'].replace('\r', '').split('\n')
    dh = [float(z[i + 8]) for i, l in enumerate(z) if l == '[path]']
    assert any(abs(d) > 0.05 for d in dh)                          # Pfade steigen mit der Platte
    # eben: nichts zu tun
    j2 = dict(j, neigung=(0.0, 0.0))
    assert not kreuzung.ebene_anwenden(j2)


def test_spline_h_schreiben_und_lesen(tmp_path):
    els = [(100.0, 100.0, 90.0, 20.0, 0.0, SLI), (120.0, 100.0, 90.0, 20.0, 0.0, SLI)]
    ch = dict(els=els, y=[0.0, 0.4], g=[2.0, 4.0], g01=[(0.0, 4.0), (4.0, 4.0)], dh=[0.4, 0.8])
    info = writer.write_map(str(tmp_path), 'H', [ch], [])
    sp = netz_datei.splines_lesen(info['dir'])
    a, b = sorted(sp.values(), key=lambda s: s['x'])
    assert (a['g0'], a['g1'], a['dh']) == (0.0, 4.0, 0.4) and b['dh'] == 0.8
    assert abs(netz_datei.hoehe_bei(a, 20.0) - b['y']) < 1e-9          # Ende = Anfang des naechsten
    assert abs(netz_datei.hoehe_bei(a, 10.0) - hoehe_im_element(0.0, 0.0, 4.0, 0.4, 20.0, 10.0)) < 1e-12
    assert os.path.isfile(os.path.join(info['dir'], 'global.cfg'))
