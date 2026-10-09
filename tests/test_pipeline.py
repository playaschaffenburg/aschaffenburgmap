import os
from omsigen.cli import main
from omsigen.ansicht import analyse
from omsigen.check import validate
from omsigen.splinedb import SplineDB

SAMPLE = os.path.join(os.path.dirname(__file__), '..', 'samples', 'aschaffenburg_hbf_citygalerie.json')
ARGS = ['--von', '49.98053,9.14023', '--nach', '49.97790,9.14998', '--name', 'Test', '--breite', '120',
        '--osm-datei', SAMPLE]


def test_hbf_city_galerie(tmp_path):
    """Hbf (ROB) -> City Galerie aus den Beispieldaten: Karte mit Kreuzungsobjekten, alle Fahrspuren verbunden."""
    rc = main(ARGS + ['--ausgabe', str(tmp_path)])
    assert rc == 0
    d = tmp_path / 'maps' / 'Test'
    assert (d / 'global.cfg').exists()
    k = tmp_path / 'Sceneryobjects' / 'Aschaffenburg' / 'Test'
    scos = sorted(k.glob('K_*.sco'))
    assert len(scos) > 20 and all((k / 'model' / (s.stem + '.x')).exists() for s in scos)
    assert (k / 'model' / 'K_001.x').read_text(encoding='ascii').startswith('xof 0302txt')
    r = analyse(str(d))
    assert r['summary']['objekte_mit_pfaden'] == len(scos)
    assert r['summary']['Strasse_enden']['beinahe'] == 0
    raw = (d / 'tile_0_0.map').read_bytes()
    assert raw[:2] == b'\xff\xfe'   # OMSI-Karten sind UTF-16
    tiles = ''.join(p.read_bytes().decode('utf-16') for p in d.glob('tile_*.map'))
    assert tiles.count('priority') > 20                       # Vorfahrt an den Kreuzungsobjekten
    assert any(l['prio'] for l in r['lanes'])


def test_alte_spline_kreuzungen(tmp_path):
    """--kreuzungen spline: der alte Kreuzungsbau aus Spur-Splines bleibt verfuegbar."""
    assert main(ARGS + ['--ausgabe', str(tmp_path), '--kreuzungen', 'spline']) == 0
    assert (tmp_path / 'Splines' / 'Aschaffenburg' / 'AB_kreuzung_spur.sli').exists()
    assert not (tmp_path / 'Sceneryobjects').exists()
    res = validate(str(tmp_path / 'maps' / 'Test'), SplineDB(), edge_points=[(0, 0)])
    assert res['splines'] > 300
