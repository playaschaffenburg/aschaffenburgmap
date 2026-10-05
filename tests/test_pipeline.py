import os
from omsigen.cli import main
from omsigen.check import validate
from omsigen.splinedb import SplineDB

SAMPLE = os.path.join(os.path.dirname(__file__), '..', 'samples', 'aschaffenburg_hbf_citygalerie.json')


def test_hbf_city_galerie(tmp_path):
    """Hbf (ROB) -> City Galerie aus den Beispieldaten: Karte entsteht, alle Fahrspuren sind verbunden."""
    rc = main(['--von', '49.98053,9.14023', '--nach', '49.97790,9.14998', '--name', 'Test', '--breite', '120',
               '--osm-datei', SAMPLE, '--ausgabe', str(tmp_path)])
    assert rc == 0
    d = tmp_path / 'maps' / 'Test'
    assert (d / 'global.cfg').exists()
    assert (tmp_path / 'Splines' / 'Aschaffenburg_KI' / 'AB_kreuzung_spur.sli').exists()
    res = validate(str(d), SplineDB(), edge_points=[(0, 0)])
    assert res['splines'] > 300
    raw = (d / 'tile_0_0.map').read_bytes()
    assert raw[:2] == b'\xff\xfe'   # OMSI-Karten sind UTF-16
