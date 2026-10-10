import numpy as np

from omsigen import vermessung as V


def test_wand_im_querschnitt_und_linie():
    # Trog: Sohle 4,5 m zwischen x 10 und 20, links und rechts Rampen auf 9,5 m; senkrechte Waende ohne Punkte
    rng = np.random.default_rng(0)
    pts = []
    for y in np.arange(0, 10, 1.0):
        for x in rng.uniform(0, 30, 600):
            if 10.3 < x < 19.7:
                z = 4.5
            elif x <= 10.0 or x >= 20.0:
                z = 9.5
            else:
                continue
            pts.append((x, y + rng.uniform(-0.2, 0.2), z + rng.normal(0, 0.02), 2))
    P = np.array(pts)
    schnitte = [(y, V.kanten(*V.querschnitt(P, y))) for y in np.arange(0, 10, 1.0)]
    k = schnitte[3][1]
    assert len(k) == 2
    assert abs(k[0][0] - 10.1) < 0.4 and k[0][3] == -1 and abs(k[0][2] - k[0][1] - 5.0) < 0.1
    L = V.linien(schnitte)
    assert len(L) == 2 and all(len(l) == 10 for l in L)
