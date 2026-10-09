import math
import numpy as np
from omsigen.sonderbau_entwurf import Achse, Gradiente, achse_anpassen, gradiente_anpassen, versatz_anpassen


def test_achse_bogen_und_projektion():
    a = Achse(0, 0, 0, [(50, 0), (math.pi / 2 * 100, 1 / 100)])     # Gerade nach Osten, Linksbogen R 100 m
    x, y, th = a.punkt(50 + math.pi / 2 * 100)
    assert abs(x - 150) < 1e-6 and abs(y - 100) < 1e-6 and abs(th - math.pi / 2) < 1e-9
    s, t = a.projektion([[25, -3]])[0]
    assert abs(s - 25) < 1e-6 and abs(t - 3) < 1e-6                # rechts der Fahrtrichtung = positiv


def test_achse_anpassen_findet_gerade_und_bogen():
    wahr = Achse(10, 20, 0.3, [(60, 0), (120, -1 / 400)])
    rng = np.random.default_rng(1)
    P = wahr.punkte(np.arange(0, 180, 1.0))[:, :2] + rng.normal(0, 0.1, (180, 2))
    a, rms = achse_anpassen(P, toleranz=0.15)
    assert rms < 0.15
    boegen = [k for _, k in a.elemente if k != 0]
    assert boegen and abs(1 / boegen[0] + 400) < 60


def test_gradiente_mit_ausrundung_glatt():
    s = np.arange(0, 200, 1.0)
    z = np.where(s < 100, 0.05 * s, 5 - 0.03 * (s - 100))
    g = gradiente_anpassen(s, z)
    assert np.max(np.abs(g.z(s) - z)) < 0.6
    dz = np.diff(g.z(s))
    assert np.max(np.abs(np.diff(dz))) < 0.01                      # keine Knicke: Neigung aendert sich stetig


def test_versatz_abschnitte_und_uebergang():
    s = np.arange(0, 200, 1.0)
    t = np.where(s < 80, 4.5, np.where(s > 120, 6.0, 4.5 + 1.5 * (s - 80) / 40)) + np.random.default_rng(2).normal(0, 0.05, 200)
    v = versatz_anpassen(s, t)
    assert len(v.a) == 2 and abs(v.a[0][2] - 4.5) < 0.06 and abs(v.a[1][2] - 6.0) < 0.06
    assert abs(v.t([100])[0] - 5.25) < 0.3


def _omsi_ende(e):
    """Endpunkt eines Elements nach OMSI (docs/omsi-format.md: Mittelpunkt = Start + R (cos h, -sin h))"""
    h = math.radians(e['richtung'])
    if e['radius'] == 0:
        return e['x'] + e['laenge'] * math.sin(h), e['y'] + e['laenge'] * math.cos(h), e['richtung']
    R = e['radius']
    cx, cy = e['x'] + R * math.cos(h), e['y'] - R * math.sin(h)
    h2 = h + e['laenge'] / R
    return cx - R * math.cos(h2), cy + R * math.sin(h2), math.degrees(h2) % 360


def test_strassen_elemente_lueckenlos():
    """Parallele zur Achse (Gerade + Rechtsbogen) als OMSI-Elemente: jedes Ende trifft den naechsten Anfang"""
    from omsigen.ring_strassen import elemente
    a = Achse(0, 0, -math.pi / 2, [(30, 0), (150, -1 / 500)])          # nach Sueden, dann Rechtsbogen
    for t in (0.0, 9.4, -9.7):
        el = elemente(a, t, 0, a.laenge, lambda s: 5 + 0.03 * s)
        for p, q in zip(el[:-1], el[1:]):
            x, y, h = _omsi_ende(p)
            assert math.hypot(x - q['x'], y - q['y']) < 1e-3 and abs((h - q['richtung'] + 180) % 360 - 180) < 1e-6
            assert abs(p['z'] + p['dh'] - q['z']) < 1e-9
        # Lage stimmt mit der Achse: Abstand t
        x, y, _ = _omsi_ende(el[-1])
        s, tt = a.projektion([[x, y]])[0]
        assert abs(tt - t) < 1e-3 and abs(s - a.laenge) < 1e-2
