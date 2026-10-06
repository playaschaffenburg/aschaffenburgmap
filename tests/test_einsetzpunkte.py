import math
from omsigen import pipeline
from omsigen.splinedb import read_text


def test_einsetzpunkte_auf_fahrspur(tmp_path):
    """Start, Ziel und Haltestelle werden Einsetzpunkte: Objekt entrypoint_bus.sco + [entrypoints] in global.cfg"""
    P = pipeline.neues_projekt(49.98, 9.14, 'E')
    P['strassen'] = [dict(id=1, tags={'highway': 'secondary', 'name': 'A'}, punkte=[[0, 0], [200, 0]])]
    P['ziele'] = [dict(name='Hbf', p=[10, 8]), dict(name='Klinikum', p=[190, -8])]
    P['haltestellen'] = [dict(name='Mitte', p=[100, -9])]
    r = pipeline.erzeuge(P, 'E', ausgabe=str(tmp_path), log=lambda *a: None)
    g = read_text(str(tmp_path / 'maps' / 'E' / 'global.cfg')).replace('\r', '').split('\n')
    i = g.index('[entrypoints]')
    n = int(g[i + 1])
    recs = [g[i + 2 + 12 * k:i + 14 + 12 * k] for k in range(n)]
    namen = [x[11] for x in recs]
    assert namen[:2] == ['Start: Hbf', 'Ziel: Klinikum'] and 'Mitte' in namen
    # Start liegt links der Strasse (Norden) -> Spur nach Westen: Drehung 270 Grad
    q = recs[0]
    h = math.degrees(2 * math.atan2(float(q[7]), float(q[9]))) % 360
    assert abs(h - 270) < 1 and float(q[5]) > 0          # nordliche Fahrspur
    tiles = ''.join(p.read_bytes().decode('utf-16') for p in (tmp_path / 'maps' / 'E').glob('tile_*.map'))
    assert tiles.count('entrypoint_bus.sco') == n
