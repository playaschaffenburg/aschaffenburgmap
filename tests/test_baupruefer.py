import numpy as np

from omsigen import baupruefer as bp


def _platte(name, x0, x1, y0, y1, z, uv=lambda x, y: (x / 4, y / 4), mat='beton'):
    """waagerechtes Rechteck aus zwei Dreiecken"""
    t = bp.Teil(name, 'objekt')
    a, b, c, d = (x0, y0, z), (x1, y0, z), (x1, y1, z), (x0, y1, z)
    P = [(a, b, c), (a, c, d)]
    UV = [[uv(*p[:2]) for p in tri] for tri in P]
    t.setzen(P, UV, [mat, mat])
    return t


def _arten(funde):
    return sorted({f['art'] for f in funde})


def test_saubere_teile_ohne_funde():
    a = _platte('A', 0, 4, 0, 4, 0.0)
    b = _platte('B', 4, 8, 0, 4, 0.0)          # stoesst genau an
    assert bp.pruefen([a, b]) == []


def test_findet_spalt_verzerrung_doppelt_und_ampel():
    a = _platte('A', 0, 4, 0, 4, 0.0)
    b = _platte('B', 4.2, 8, 0, 4, 0.0)        # 20 cm Spalt
    c = _platte('C', 10, 14, 0, 4, 0.0, uv=lambda x, y: (x / 4, y / 40))       # 10-fach gestreckt
    d = _platte('D', 20, 24, 0, 4, 1.0)
    e = _platte('E', 21, 23, 1, 3, 1.0, mat='asphalt')                        # deckungsgleich auf D
    signale = [dict(art='signal', datei='Ampel_Kfz_1.sco', x=22.0, y=2.0, hoehe=2.8, z_boden=1.5)]   # 0,5 m ueber D, ohne Mast
    funde = bp.pruefen([a, b, c, d, e], None, signale)
    arten = _arten(funde)
    assert 'spalt' in arten
    assert any(f['art'] == 'spalt' and '20 cm' in f['text'] for f in funde)
    assert any(f['art'] == 'textur_verzerrt' and f['teil'] == 'C' for f in funde)
    assert 'doppelte_flaeche' in arten
    assert 'ampel_schwebt' in arten and 'ampel_ohne_mast' in arten


def test_verdeckte_spalte_zaehlen_nicht():
    # Platte A liegt 10 cm unter einer grossen Fahrbahn F: ihr Spalt zu B ist nicht zu sehen
    a = _platte('A', 0, 4, 0, 4, 0.0)
    b = _platte('B', 4.2, 8, 0, 4, 0.0)
    f = _platte('F', -1, 9, -1, 5, 0.1, mat='asphalt')
    funde = [x for x in bp.pruefen([a, b, f]) if x['teil'] in ('A', 'B')]
    assert not any(x['art'] == 'spalt' for x in funde)


def test_x_datei_lesen(tmp_path):
    from omsigen.kreuzung import x_file
    V = [(0, 0, 0, 0, 1, 0, 0, 0), (1, 0, 0, 0, 1, 0, 1, 0), (0, 0, 1, 0, 1, 0, 0, 1)]
    F = [(0, 1, 2, 0)]
    p = tmp_path / 'a.x'
    p.write_text(x_file(V, F, [('beton', 'b.bmp')]), encoding='cp1252')
    V2, F2, UV, mat = bp.x_lesen(str(p))
    assert V2.shape == (3, 3) and F2.tolist() == [[0, 1, 2]] and mat == ['beton']
    assert np.allclose(UV[:, 1], [0, 0, -1])
