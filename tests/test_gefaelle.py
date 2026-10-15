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


def test_gelaende_im_einschnitt_bleibt_unter_der_strasse():
    # Strasse quer zum 5-m-Raster versetzt (z = 1,3) in einem Einschnitt: daneben steigt das Gelaende mit 30 % an.
    # OMSI legt das Gelaende als Dreiecke zwischen den Rasterpunkten - kein Punkt der Strasse darf darunter liegen
    from omsigen.splinedb import SplineDB
    ways = [dict(tags=dict(highway='residential', name='A'), P=[(-400.0, 1.3), (400.0, 1.3)])]
    sdb = SplineDB()
    net = build(ways, sdb)
    hoe = Hoehen(Hang(lambda x, z: 100 + 0.3 * abs(z - 1.3)), log=lambda *a: None).berechnen(net, sdb)
    x0, z0, n, schritt = -150.0, -150.0, 61, 5.0
    H = hoe.raster(x0, z0, n, schritt)

    def gel(x, z):                              # wie OMSI / ausstattung.Gelaende.hoehe
        fx, fz = (x - x0) / schritt, (z - z0) / schritt
        ix, iz = int(fx), int(fz)
        tx, tz = fx - ix, fz - iz
        h00, h10, h01, h11 = H[iz, ix], H[iz, ix + 1], H[iz + 1, ix], H[iz + 1, ix + 1]
        if tx >= tz:
            return h00 + (h10 - h00) * tx + (h11 - h10) * tz
        return h00 + (h11 - h01) * tx + (h01 - h00) * tz
    q = sdb[net['road_chains'][0]['els'][0][5]]
    ch = net['road_chains'][0]
    ueber = 0.0
    for e, y in zip(ch['els'], ch['y']):
        if not -140 < e[0] < 140:
            continue
        for d in range(int(q['ol'] * 4), int(q['or_'] * 4) + 1):
            z = e[1] - d / 4.0                  # Strasse nach Osten: rechts = Sueden
            ueber = max(ueber, gel(e[0], z) - (y + 0.10))
    assert ueber < 0.02, ueber


def test_trassierung_kurve_mit_echtem_radius():
    # OSM zeichnet eine Kurve (R = 80 m) als Polygonzug mit 8-m-Stuecken und etwas Zittern: frueher wurde jede Ecke
    # mit 15 m ausgerundet (eckig), jetzt entsteht ein Bogen mit dem echten Radius, der nah an der Linie bleibt
    import random
    from omsigen.splinedb import SplineDB
    from omsigen.geom import sample
    random.seed(3)
    R0, P = 80.0, []
    for k in range(0, 21):
        a = math.radians(k * 4.5)                       # 90 Grad in 21 Punkten, Abstand ~6,3 m
        r = R0 + random.uniform(-0.3, 0.3)
        P.append((R0 - r * math.cos(a), r * math.sin(a)))
    # Gerade davor (von Sueden nach Norden) und danach (nach Osten)
    P = [(P[0][0], P[0][1] - 60.0)] + P + [(P[-1][0] + 60.0, P[-1][1])]
    net = build([dict(tags=dict(highway='residential', name='K'), P=P)], SplineDB())
    els = net['road_chains'][0]['els']
    boegen = [abs(e[4]) for e in els if e[4] != 0]
    assert boegen and sorted(boegen)[len(boegen) // 2] > 50, boegen
    # Abstand der neuen Achse zur OSM-Kurve
    from shapely.geometry import LineString, Point
    osm = LineString(P)
    abw = max(osm.distance(Point(q)) for e in els for q, _ in sample(e, 1.0))
    assert abw < 1.6, abw
