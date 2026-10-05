import math, random
from omsigen.geom import connect, end_of, norm180, ok


def test_connect_random():
    """Jede Verbindung muss stetig sein und exakt (Lage + Richtung) am Ziel ankommen."""
    rnd = random.Random(1)
    for _ in range(5000):
        a = (rnd.uniform(-20, 20), rnd.uniform(-20, 20)); b = (rnd.uniform(-20, 20), rnd.uniform(-20, 20))
        ha, hb = rnd.uniform(0, 360), rnd.uniform(0, 360)
        els = connect(a, ha, b, hb, 'x')
        assert els, (a, ha, b, hb)
        p, h = a, ha
        for e in els:
            assert math.dist(p, (e[0], e[1])) < 1e-6
            assert abs(norm180(h - e[2])) < 0.05
            p, h = end_of(e)
        assert ok(els, b, hb)


def test_arc_convention():
    """OMSI: Richtung im Uhrzeigersinn ab Nord, positiver Radius = Rechtskurve (aus Grundorf abgeleitet)."""
    (x, z), h = end_of([154.429, 129.72, 271.573, 91.482, 37.468])
    assert abs(x - 132.116) < 0.01 and abs(z - 196.483) < 0.01 and abs(h - 51.466) < 0.01
