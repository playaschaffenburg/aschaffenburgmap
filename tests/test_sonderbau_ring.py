import numpy as np
from omsigen.sonderbau_ring import Kartengelaende, Mesh, kasten


def test_kartengelaende_dreiecke(tmp_path):
    """Hoehe wie openOMSI zeichnet: Diagonale (i, j)-(i+1, j+1), Zeile 0 = Sueden"""
    h = np.zeros((61, 61), '<f4')
    h[0, 1] = 5.0          # Sueden, ein Punkt nach Osten
    with open(tmp_path / 'tile_0_0.map.terrain', 'wb') as f:
        np.array([60], '<i4').tofile(f)
        h.tofile(f)
    k = Kartengelaende(str(tmp_path))
    assert abs(k.hoehe(5.0, 0.0) - 5.0) < 1e-6
    assert abs(k.hoehe(4.0, 1.0) - 3.0) < 1e-6      # unteres Dreieck (fx >= fy): h00 + fx*(h10-h00) + fy*(h11-h10)
    assert abs(k.hoehe(1.0, 4.0) - 0.0) < 1e-6      # oberes Dreieck ohne den Punkt
    assert k.hoehe(-1.0, 5.0) is None


def test_kasten_aussen():
    """alle Seiten eines Quaders zeigen nach aussen"""
    m = Mesh()
    kasten(m, 'beton', 0, 1, 0, 2, 0, 3)
    assert len(m.F) == 12
    V = np.array([v[:3] for v in m.V])
    mitte = V.mean(axis=0)
    for a, b, c, _ in m.F:
        n = np.array(m.V[a][3:6])
        assert np.dot((V[a] + V[b] + V[c]) / 3 - mitte, n) > 0
