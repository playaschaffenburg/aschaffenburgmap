import math

import numpy as np

from omsigen import ring_wege as rwg
from omsigen.ring_strassen import abgewandelt


def _wege(hoehe=lambda x, y: 10.0):
    plan = [dict(id='a', breite=3.0, punkte=[[0.0, 0.0], [0.0, -30.0], [5.0, -60.0]], rand_links=dict(art='mauer_hecke', von=1, bis=20))]
    return plan, rwg.splines(plan, hoehe)


def test_weg_folgt_der_oberflaeche():
    plan, wege = _wege(lambda x, y: 10.0 - 0.05 * (-y))      # faellt nach Sueden
    w = wege[0]
    assert w['querschnitt']['teile'][0]['art'] == 'gehweg'
    e0, e1 = w['elemente'][0], w['elemente'][-1]
    # Oberflaeche = Spline + Hochbord = sichtbare Hoehe + WEG_UEBER
    assert abs(e0['z'] + rwg.HOCHBORD - (10.0 + rwg.WEG_UEBER)) < 0.05
    assert e1['z'] + e1['dh'] < e0['z']


def test_grundriss_folgt_dem_spline():
    plan, wege = _wege()
    g = rwg.grundriss(wege)
    g2 = rwg.grundriss(wege, rwg.UNTER_WEG)
    assert abs(g.area - 3.0 * 60.6) < 6.0
    assert g2.area < g.area and g.buffer(1e-6).contains(g2)


def test_anschmiegen_zieht_den_deckel_an_den_wegrand():
    plan, wege = _wege()
    f = rwg.anschmiegen(wege)
    oben = 10.0 + rwg.WEG_UEBER
    assert abs(f(0.0, -10.0, 13.0) - (oben - 0.03)) < 0.05       # auf dem Weg
    assert f(5.0, -10.0, 13.0) == 13.0                           # weit weg: unveraendert
    z = f(2.5, -10.0, 13.0)                                     # im Uebergang
    assert oben < z < 13.0


def test_rand_links_mit_stirnflaechen():
    plan, wege = _wege()
    ketten, dateien = rwg.raender(plan, [], wege, lambda x, y: 10.0)
    assert len(ketten) == 1 and len(dateien) == 1
    k = ketten[0]
    # links des nach Sueden laufenden Wegs = Osten
    assert k['elemente'][0]['x'] > 1.4
    assert 'AB_SB_sandstein.bmp' in next(iter(dateien.values()))
    m = rwg.rand_enden(ketten, np.zeros(3))
    assert len(m.F) >= 8


def test_abschnitt_ohne_sperrflaeche():
    q = dict(name='SB_Rampe', teile=[dict(art='parkstreifen', breite=2.6, belag='sperr'), dict(art='fahrspur', breite=3.6)])
    a = abgewandelt(q, 'ohne_sperr')
    assert a['teile'][0]['belag'] == 'asphalt' and q['teile'][0]['belag'] == 'sperr'
    assert a['name'] != q['name']
