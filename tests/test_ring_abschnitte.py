import numpy as np

from omsigen import ring_abschnitte as R
from omsigen.kreuzung import x_file


def test_x_teilen_rundweg(tmp_path):
    # zwei Dreiecke nebeneinander, Ecken (x, hoch, vor, Normale, UV)
    V = [(0, 0, 0, 0, 1, 0, 0, 0), (1, 0, 0, 0, 1, 0, 1, 0), (0, 0, 1, 0, 1, 0, 0, 1),
         (5, 0, 0, 0, 1, 0, 0, 0), (6, 0, 0, 0, 1, 0, 1, 0), (5, 0, 1, 0, 1, 0, 0, 1)]
    F = [(0, 1, 2, 0), (3, 4, 5, 1)]
    p = tmp_path / 'a.x'
    p.write_bytes(x_file(V, F, [('asphalt', 'str_asphdrk.bmp'), ('beton', 'betonwand1.bmp')]).encode('cp1252'))
    Vx, N, UV, Fx, M, mats = R.x_lesen(str(p))
    assert len(Vx) == 6 and len(Fx) == 2 and list(M) == [0, 1] and mats[1] == ('beton', 'betonwand1.bmp')
    teil = R.x_teilen(Vx, N, UV, Fx, M, np.array([False, True]))
    assert len(teil[0]) == 3 and teil[3].tolist() == [[0, 1, 2]] and teil[4].tolist() == [1]
    q = tmp_path / 'b.x'
    R.x_schreiben(str(q), *teil, mats)
    V2, _, _, F2, M2, _ = R.x_lesen(str(q))
    assert np.allclose(V2[:, 0], [5, 6, 5]) and M2.tolist() == [1]


def test_zuordnung_nach_namen_und_lage():
    # Trennwaende bei x = -5 (West) und +5 (Ost), Portale bei y -248.9 / -284
    alle = [dict(art='[spline_h]', pfad='Splines\\Aschaffenburg\\AB_SB_Wand_trenn_klinker_bruestung_keine_w_7m.sli', x=-5.0, y=y) for y in (-100, -400)]
    alle += [dict(art='[spline_h]', pfad='Splines\\Aschaffenburg\\AB_SB_Wand_trenn_klinker_bruestung_keine_o_8m.sli', x=5.0, y=y) for y in (-100, -400)]
    lage = R.Lage(alle)
    e = lambda pfad, x, y, art='[spline_h]': dict(art=art, pfad=pfad, x=x, y=y)
    assert R.zuordnen(e('Splines\\Aschaffenburg\\AB_SB_Ring_Trog.sli', 0, -150), lage) == 'NT'
    assert R.zuordnen(e('Splines\\Aschaffenburg\\AB_SB_Rampe_West_Sued.sli', -8, -350), lage) == 'SW'
    assert R.zuordnen(e('Splines\\Aschaffenburg\\AB_SB_Wand_portal_beton_nord_3m.sli', 0, -250), lage) == 'PL'
    assert R.zuordnen(e('X\\Sonderbau_Ring\\Kreuzung_Ludwigsallee.sco', 0, -267, '[object]'), lage) == 'PL'
    # Trennwand gehoert zur Rampe (Bruestung steht auf der Rampe)
    assert R.zuordnen(e('Splines\\Aschaffenburg\\AB_SB_Wand_trenn_klinker_betonbr_keine_w_7m.sli', -5, -150), lage) == 'NW'
    assert R.zuordnen(e('Splines\\Aschaffenburg\\AB_SB_Wand_trenn_klinker_betonbr_keine_o_8m.sli', 5, -350), lage) == 'SO'
    assert R.zuordnen(e('X\\Sonderbau_Ring\\Sohle_02.sco', 0, -200, '[object]'), lage) == 'teilen'
    assert lage.abschnitt(8, -350) == 'SO' and lage.abschnitt(-8, -150) == 'NW'
