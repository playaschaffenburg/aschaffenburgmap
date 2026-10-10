import numpy as np

from omsigen import portal_splines as S
from omsigen.ring_waende import kette


def test_kette_hoehe_trifft_die_knoten():
    P = np.c_[np.zeros(41), np.arange(41.0)]
    Z = 5.0 + 0.002 * np.arange(41.0) ** 2           # Kuppe/Wanne wie eine Rampe
    els = kette(P, Z, knoten=5.0)
    for e in els:
        assert abs(S.kette_hoehe(els, e['kum']) - e['z']) < 1e-6
    # zwischen den Knoten nahe an der Messung (Auflagen liegen sonst unter der Fahrbahn)
    for s in (2.5, 17.5, 33.0):
        assert abs(S.kette_hoehe(els, s) - (5.0 + 0.002 * s ** 2)) < 0.02


def test_sli_fahrbahn_mit_pfaden():
    s = S.Sli('Test')
    s.hoehen.append((-4.0, 4.0, 0.0))
    s.flach('asphalt', -4.0, 4.0, 0.0, u_abs=True)
    s.pfade.append((0, 2.0, 0.0, 3.0, 0))
    s.pfade.append((0, -2.0, 0.0, 3.0, 1))
    t = s.text()
    assert t.count('[path]') == 2 and '[heightprofile]' in t and 'str_asphdrk.bmp' in t
    # Strecke von links nach rechts: sichtbar nach oben
    assert '-4.000\r\n0.000' in t


def test_wand_sichtseite():
    s = S.Sli('Wand')
    s.senkrecht('beton', 0.0, -3.0, 0.0, -1)          # sichtbar nach links: von unten nach oben
    (tex, pts), = s.teile
    assert pts[0][1] < pts[1][1]
