"""omsigen.geodaten ohne Netz: Koordinaten statt Ortssuche, UTM-Zone, Terrarium-Dekodierung"""
import numpy as np
from omsigen import geodaten


def test_koordinaten_direkt():
    assert geodaten.suchen('49.9755, 9.1420') == [{'name': '49.975500, 9.142000', 'lat': 49.9755, 'lon': 9.142}]
    assert geodaten.suchen('   ') == []


def test_zone():
    assert geodaten.zone_fuer(49.97, 9.14) == 32      # Aschaffenburg
    assert geodaten.zone_fuer(48.57, 13.46) == 32     # Passau: Bayern bleibt in 32 (wie DGM1/DOP)
    assert geodaten.zone_fuer(52.52, 13.40) == 33     # Berlin


def test_terrarium_bilinear(tmp_path):
    t = geodaten.Terrarium(str(tmp_path))
    # alle Kacheln konstant 100 m (R*256 + G + B/256 - 32768 = 100)
    t._kachel = lambda x, y: np.full((256, 256), 100.0)
    assert abs(t.hoehe(49.97, 9.14) - 100.0) < 1e-9


def test_kacheln_ohne_daten(tmp_path, monkeypatch):
    # keine Hoehen, kein Luftbild: Eintraege ohne Dateien
    monkeypatch.setattr(geodaten.Hoehen, 'raster', lambda self, e, n: (np.full(e.shape, np.nan), None))
    erg = geodaten.kacheln({'zone': 33, 'ost0': 0, 'nord0': 0, 'kacheln': [[0, 0]], 'ordner': str(tmp_path / 'a'),
                            'cache': str(tmp_path)})
    assert erg['kacheln'] == [{'kachel': [0, 0], 'gelaende': None, 'gelaende_quelle': None, 'luftbild': None}]
