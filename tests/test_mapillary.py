import math

from omsigen import mapillary as M


def test_strahl_trifft_kompass():
    # Bild 949752907603283 (Ring Richtung Norden, April 2026): Bildmitte zeigt in die berechnete Kompassrichtung
    b = dict(width=2560, height=1440, camera_parameters=[0.4246476258798, -0.14647433247554, 0.015180775590469],
             computed_rotation=[1.5674466451668, 0.25061553053503, -0.16494363465753])
    v = M.strahl(b, 0.5, 0.5)
    assert abs(math.degrees(math.atan2(v[0], v[1])) % 360 - 344.86623267914) < 1e-6
    # weiter unten im Bild zeigt der Strahl steiler nach unten
    assert M.strahl(b, 0.5, 0.75)[2] < v[2]


def test_kachel_polygon():
    # Vektor-Kachel von Hand: Ebene mit einem Feature, Quadrat (0,0)-(2048,2048) bei extent 4096
    def vi(n):
        out = b''
        while True:
            c = n & 0x7F
            n >>= 7
            if n:
                out += bytes([c | 0x80])
            else:
                return out + bytes([c])

    def feld(nr, daten):
        return vi(nr << 3 | 2) + vi(len(daten)) + daten

    zz = lambda n: (n << 1) ^ (n >> 31)
    cmd = [1 << 3 | 1, zz(0), zz(0), 3 << 3 | 2, zz(2048), zz(0), zz(0), zz(2048), zz(-2048), zz(0), 1 << 3 | 7]
    geo = b''.join(vi(c) for c in cmd)
    feat = feld(4, geo)
    layer = feld(1, b'mpy-or') + feld(2, feat) + vi(5 << 3) + vi(4096)
    import base64
    ringe = M.kachel_polygone(base64.b64encode(feld(3, layer)).decode())
    assert ringe == [[(0.0, 0.0), (0.5, 0.0), (0.5, 0.5), (0.0, 0.5)]]


def test_pose_im_kartengitter():
    b = dict(id=1, width=2560, height=1440, camera_parameters=[0.4246476258798, -0.14647433247554, 0.015180775590469],
             computed_rotation=[1.5674466451668, 0.25061553053503, -0.16494363465753], captured_at=1776543608100,
             computed_geometry={'type': 'Point', 'coordinates': [9.161253200614, 49.976625664883]}, sequence='s')
    p = M.pose(b, 510466, 5536235)
    # Gitter-Nord gegen geografisch Nord um die Meridiankonvergenz (~0,12 Grad) gedreht
    assert 0.08 < 344.86623267914 - p['gier'] < 0.16
    assert abs(p['hfov'] - 2 * math.degrees(math.atan(0.5 / 0.4246476258798))) < 1e-3
    assert -10 < p['roll'] < 10 and -10 < p['neigung'] < 10


def test_entzerren_laesst_mitte():
    import numpy as np
    a = np.zeros((90, 160, 3), np.uint8)
    a[45, 80] = 255
    e = M.entzerren(a, [0.42, -0.15, 0.02])
    assert e[45, 80].max() == 255
