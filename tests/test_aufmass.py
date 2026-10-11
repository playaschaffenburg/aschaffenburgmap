import numpy as np

from omsigen import aufmass as A


def trog_punkte():
    """kuenstlicher Trog: Sohle 0 m zwischen x = -4 und 4, oben 4 m ab |x| > 4,6 (Wand ohne Bodenpunkte dazwischen)"""
    rng = np.random.default_rng(1)
    x = rng.uniform(-12, 12, 60000)
    y = rng.uniform(-2, 42, 60000)
    z = np.where(np.abs(x) < 4.0, 0.0, 4.0) + rng.normal(0, 0.02, len(x))
    frei = (np.abs(x) >= 4.0) & (np.abs(x) <= 4.6)
    return np.c_[x, y, z, np.full(len(x), 2.0)][~frei]


def test_kanten_am_wandfuss():
    G = trog_punkte()
    s = np.arange(0, 40.01, 0.5)
    ax = A.Achse(np.c_[np.zeros(len(s)), s])               # Achse nach Norden: rechts = Osten (+x)
    L = A.kantenlinien(G, ax, -10, 10)
    K = A.kanten_aus(L, len(ax.p), -10, 10)
    assert len(K) == 2
    t = sorted(float(np.median(k['t'])) for k in K)
    # Wandflaeche = Ende der Sohle (Fuss), nicht die obere Kante
    assert abs(t[0] + 3.9) < 0.3 and abs(t[1] - 3.9) < 0.3
    assert all(abs(k['hoehe'] - 4.0) < 0.2 for k in K)


def test_messung_json_rundweg():
    G = trog_punkte()
    s = np.arange(0, 40.01, 0.5)
    ax = A.Achse(np.c_[np.zeros(len(s)), s])
    ns = len(ax.p)
    K = A.kanten_aus(A.kantenlinien(G, ax, -10, 10), ns, -10, 10)
    g = [np.full(ns, -10.0)] + [k['t'] for k in K] + [np.full(ns, 10.0)]
    a = np.c_[np.full(ns, 4.0), np.zeros(ns), np.full(ns, 4.0)]
    quer = [(np.arange(0.125, 6, 0.25), np.zeros(24))] * 3
    m = dict(ax=ax, K=K, grenzen=g, a=a, b=np.zeros_like(a), fahrbahn=[False, True, False], quer=quer, t0=-10.0, t1=10.0)
    mj = A.messung_json(m, [], [], 'Testkarte', 'Test', [-12, -2, 12, 42], dict(ost0=0, nord0=0, zone=32, nn0=0), [0, 0, 0, 40])
    assert [k['art'] for k in mj['kanten']] == ['trog', 'trog']
    assert mj['streifen'][1]['art'] == 'fahrbahn'
    mj['streifen'][1]['richtung'] = 'vor'
    mj['kanten'][0]['bauen'] = False
    m2, linien, sperr = A.m_aus_json(mj)
    assert m2['streifen_info'][1]['richtung'] == 'vor'
    assert m2['K'][0]['bauen'] is False
    assert np.allclose(m2['ax'].p, ax.p, atol=1e-3)
