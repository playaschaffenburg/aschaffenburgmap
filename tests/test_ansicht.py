import os
from omsigen.ansicht import analyse, near_misses
from omsigen.cli import main
from omsigen.check import validate
from omsigen.splinedb import SplineDB
from omsigen.writer import write_utf16

SAMPLE = os.path.join(os.path.dirname(__file__), '..', 'samples', 'aschaffenburg_hbf_citygalerie.json')

SCO = """[friendlyname]
Testkreuzung

[path]
0
0
0
0
0
10
0
0
0
3
0
0
"""

SLI = """[path]
0
0
0
3
0
"""


def _tile(*records):
    return '\r\n'.join(['[version]', '14', ''] + [l for r in records for l in r + ['']]) + '\r\n'


def test_objektpfad_lage_und_regel(tmp_path):
    """Objektpfade: Lage im Objekt (x rechts, y vorwaerts) + Drehung im Uhrzeigersinn; [rule] haengt am Objekt."""
    (tmp_path / 'Sceneryobjects' / 'T').mkdir(parents=True)
    (tmp_path / 'Sceneryobjects' / 'T' / 'k.sco').write_text(SCO, encoding='cp1252')
    (tmp_path / 'Splines' / 'T').mkdir(parents=True)
    (tmp_path / 'Splines' / 'T' / 's.sli').write_text(SLI, encoding='cp1252')
    d = tmp_path / 'maps' / 'X'
    d.mkdir(parents=True)
    write_utf16(str(d / 'global.cfg'), '[friendlyname]\r\nX\r\n\r\n[map]\r\n0\r\n0\r\ntile_0_0.map\r\n')
    obj = ['[object]', '0', 'Sceneryobjects\\T\\k.sco', '1', '100', '50', '0', '90', '0', '0', '0']
    rule = ['[rule]', '0', 'priority', '64', '0']
    # Spline beginnt genau am Ende des Objektpfads (110, 50) in Richtung 90 (Osten)
    spl = ['[spline]', '0', 'Splines\\T\\s.sli', '2', '0', '0', '110', '0', '50', '90', '20', '0',
           '0', '0', '0', '0', '0', '0', '0']
    write_utf16(str(d / 'tile_0_0.map'), _tile(obj, rule, spl))
    r = analyse(str(d))
    obj_lane = [l for l in r['lanes'] if l['src']['art'] == 'Objekt'][0]
    assert abs(obj_lane['pts'][-1][0] - 110) < 1e-6 and abs(obj_lane['pts'][-1][1] - 50) < 1e-6
    assert obj_lane['prio'] == 64
    se = r['summary']['Strasse_enden']
    assert se['gesamt'] == 4 and se['verbunden_streng'] == 2 and se['frei'] == 2


def test_beinahe_anschluss(tmp_path):
    """2 m Luecke zwischen zwei Strassenstuecken wird gemeldet, das Ende am Rand nicht."""
    (tmp_path / 'Splines' / 'T').mkdir(parents=True)
    (tmp_path / 'Splines' / 'T' / 's.sli').write_text(SLI, encoding='cp1252')
    d = tmp_path / 'maps' / 'X'
    d.mkdir(parents=True)
    write_utf16(str(d / 'global.cfg'), '[map]\r\n0\r\n0\r\ntile_0_0.map\r\n')
    a = ['[spline]', '0', 'Splines\\T\\s.sli', '1', '0', '0', '10', '0', '10', '90', '20', '0'] + ['0'] * 7
    b = ['[spline]', '0', 'Splines\\T\\s.sli', '2', '0', '0', '32', '0', '10', '90', '20', '0'] + ['0'] * 7
    write_utf16(str(d / 'tile_0_0.map'), _tile(a, b))
    r = analyse(str(d))
    nm = near_misses(r['lanes'], r['ends'])
    assert len(nm) == 2 and all(abs(e['naechster'][0] - 2) < 1e-6 for e in nm)
    assert r['summary']['Strasse_enden']['frei'] == 2


def test_eigene_karte_wie_validate(tmp_path):
    """Fuer omsigen-Karten: freie Strassenenden = Strassenenden aus check.validate, keine Beinahe-Anschluesse."""
    rc = main(['--von', '49.98053,9.14023', '--nach', '49.97790,9.14998', '--name', 'T', '--breite', '120',
               '--osm-datei', SAMPLE, '--ausgabe', str(tmp_path), '--ansicht', str(tmp_path / 'a.html')])
    assert rc == 0 and (tmp_path / 'a.html').exists()
    d = str(tmp_path / 'maps' / 'T')
    r = analyse(d)
    se = r['summary']['Strasse_enden']
    assert se['beinahe'] == 0
    res = validate(d, SplineDB(), edge_points=[(0, 0)])
    assert se['frei'] == len(res['open']) + len(res['dead_ends'])
