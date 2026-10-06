import math
from omsigen.geom import end_of
from omsigen.network import build, wendeschleife
from omsigen.splinedb import SplineDB


def test_schleife_endet_auf_gegenspur():
    for d in (3.3, 7.0, 14.0):
        p, h = (10.0, 5.0), 90.0
        for e in wendeschleife(p, h, d):
            assert e[4] == 0 or abs(e[4]) >= 6.0 - 1e-9           # keine Haarnadel enger als 6 m
            p, h = end_of(e)
        assert math.dist(p, (10.0, 5.0 + d)) < 1e-6 and abs(h - 270) < 1e-6


def test_sackgasse_bekommt_schleife():
    """zweispurige Strasse, die im Nichts endet: eine Schleife, die Spurenden sind dann verbunden"""
    net = build([dict(tags=dict(highway='residential', name='A'), P=[(0.0, 0.0), (100.0, 0.0)])], SplineDB())
    assert net['stats']['wendeschleifen'] == 2                    # beide Enden
    ch = net['road_chains'][0]
    start, ende = (ch['els'][0][0], ch['els'][0][1]), end_of(ch['els'][-1])[0]
    for w in net['wenden']:
        a = (w['els'][0][0], w['els'][0][1])
        assert min(math.dist(a, start), math.dist(a, ende)) < 3     # beginnt an einem Strassenende
