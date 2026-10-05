import math
from shapely.geometry import Polygon
from omsigen.geom import fillet, end_of
from omsigen.kreuzung import build_junction, mesh, x_file, sco_text
from omsigen.splinedb import SplineDB

SPL = 'Splines\\Marcel\\str_2spur_8m_altonaer1.sli'   # Fahrbahn +-4 m, Gehweg bis +-7 m


def _x_kreuzung(d=15):
    """vier Arme, die d m vom Mittelpunkt (0, 0) entfernt enden, Spline zeigt jeweils von der Kreuzung weg"""
    return [dict(pos=(d * math.sin(math.radians(h)), d * math.cos(math.radians(h))), h=h, spl=SPL, away=True)
            for h in (0, 90, 180, 270)]


def test_fillet_ecke():
    els = fillet((0, -10), 0, (10, 0), 90, 6)
    p, h = (0, -10), 0
    for e in els:
        p, h = end_of(e)
    assert math.dist(p, (10, 0)) < 1e-9 and abs(h - 90) < 1e-9
    assert [round(e[4], 6) for e in els] == [0, 6, 0]          # Gerade, Rechtsbogen R 6, Gerade
    assert fillet((0, 0), 0, (5, -5), 90, 6) is None             # Schnittpunkt hinter dem Start


def test_x_kreuzung_flaechen():
    J = build_junction(_x_kreuzung(), SplineDB())
    assert J['origin'] == (0.0, 0.0) or math.dist(J['origin'], (0, 0)) < 1e-9
    a, s = J['asphalt'], J['side']
    # Asphalt: Quadrat 30 x 30 minus vier Ecken mit Radius-Ausrundung, aber mindestens die Fahrbahnkreuzung
    assert 8 * 30 * 2 - 64 < a.area < 30 * 30
    assert s.area > 4 * 3 * 3 and a.intersection(s).area < 1e-3
    # nichts ragt in die Flaeche der Arme (|x| > 15 oder |z| > 15 innerhalb der Strassenbreite)
    for poly in (Polygon([(15.01, -7), (60, -7), (60, 7), (15.01, 7)]),
                 Polygon([(-7, 15.01), (7, 15.01), (7, 60), (-7, 60)])):
        assert a.intersection(poly).area < 1e-6 and s.intersection(poly).area < 1e-6
    assert len(J['kerbs']) == 4 and len(J['walks']) == 4           # vier Ecken mit Bordstein und Gehwegpfad


def test_modell_und_objekt():
    J = build_junction(_x_kreuzung(), SplineDB())
    V, F = mesh(J)
    assert V and F and max(max(f[:3]) for f in F) < len(V)
    assert {f[3] for f in F} == {0, 1}                             # Asphalt und Gehweg
    assert any(v[4] == 0 for v in V)                               # senkrechte Bordsteinflaechen
    x = x_file(V, F)
    assert x.startswith('xof 0302txt') and 'TextureFilename { "str_side1.bmp"; }' in x
    t = sco_text('T', 'K_001.x', [([[0.0, -15.0, 0.0, 30.0, 0.0]], 0)], J['walks'])
    assert t.count('[path]') == 1 + sum(len(w) for w in J['walks']) and '[mesh]\r\nK_001.x' in t
