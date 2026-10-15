"""Vorbildstudie: Element-Enden, Kennzahlen eines Arms an der Kreuzung."""
import math

from omsigen import vorbildstudie as V


def test_ende_gerade_und_bogen():
    x, z, h, y = V.ende(dict(x=0.0, z=0.0, y=10.0, h=90.0, L=20.0, R=0.0, g=(5.0, 5.0), dh=None))
    assert abs(x - 20) < 1e-9 and abs(z) < 1e-9 and h == 90 and abs(y - 11.0) < 1e-9
    # Viertelkreis rechts herum, R = 10: von Nord nach Ost
    x, z, h, y = V.ende(dict(x=0.0, z=0.0, y=0.0, h=0.0, L=10 * math.pi / 2, R=10.0, g=(0.0, 4.0), dh=None))
    assert abs(x - 10) < 1e-6 and abs(z - 10) < 1e-6 and abs(h - 90) < 1e-6
    assert abs(y - 10 * math.pi / 2 * 0.02) < 1e-9          # Steigung linear 0 -> 4 %: Mittel 2 %


def test_arm_kennzahlen_ausrundung():
    # Vorbild: 0 % an der Platte, auf 30 m bis 10 %, dann gleichbleibend
    prof = [(0.0, 0.0, 10.0, 30.0), (30.0, 10.0, 10.0, 30.0)]
    an, gmax, aus, flach = V._arm_kennzahlen(prof)
    assert an == 0 and gmax == 10 and flach == 0 and abs(aus - 27.0) < 1e-9
    # v9-artig: volle Steigung schon an der Platte -> Knick
    an, gmax, aus, flach = V._arm_kennzahlen([(0.0, -7.0, -7.0, 19.6)])
    assert an == 7 and aus == 0
