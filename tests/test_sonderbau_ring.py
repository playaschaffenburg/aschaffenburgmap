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


def test_kreuzung_hoehenflaeche_trifft_die_arme():
    """die Kreuzungsflaeche liegt an jedem Armende genau auf Hoehe der Strasse und laeuft mit deren Steigung weiter"""
    from omsigen.ring_kreuzung import Arm, hoehenflaeche
    q = dict(teile=[dict(art='fahrspur', breite=3.5, richtung='zurueck'), dict(art='fahrspur', breite=3.5, richtung='vor')])
    roh = [dict(strasse='a', rolle='haupt', pos=[-20.0, 0.0], h=270.0, away=True, z=10.0, g=0.02, querschnitt=q),
           dict(strasse='b', rolle='haupt', pos=[20.0, 0.0], h=90.0, away=True, z=11.0, g=0.03, querschnitt=q),
           dict(strasse='c', rolle='neben', pos=[0.0, 20.0], h=0.0, away=False, z=10.2, g=-0.04, querschnitt=q)]
    arme = [Arm(i, a) for i, a in enumerate(roh)]
    z = hoehenflaeche(arme)
    for a in roh:
        assert abs(z(a['pos'][0], a['pos'][1] + 1.0 * (a['h'] in (90.0, 270.0))) - a['z']) < 0.01
    m = z(0.0, 0.0)
    assert 10.0 < m < 11.0


def test_kreuzung_spuren_von_rechts():
    """Spuren je Arm: ankommend/abgehend, von rechts in Fahrtrichtung gezaehlt"""
    from omsigen.ring_kreuzung import Arm
    q = dict(teile=[dict(art='gehweg', breite=2.5, richtung='vor'), dict(art='busspur', breite=3.0, richtung='zurueck'),
                    dict(art='fahrspur', breite=3.5, richtung='zurueck'), dict(art='fahrspur', breite=3.5, richtung='zurueck'),
                    dict(art='fahrspur', breite=3.5, richtung='vor'), dict(art='gehweg', breite=2.5, richtung='vor')])
    # Ostarm: Spline West -> Ost beginnt an der Kreuzung; ankommend = 'zurueck' (nach Westen), rechts = Norden
    a = Arm(0, dict(strasse='o', rolle='haupt', pos=[0.0, 0.0], h=90.0, away=True, z=0, g=0, querschnitt=q))
    rein, raus = a.spuren()
    assert [t['art'] for _, t in rein] == ['busspur', 'fahrspur', 'fahrspur']
    assert rein[0][0] < rein[1][0] < rein[2][0] and len(raus) == 1
