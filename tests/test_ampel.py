from omsigen import ampel
from omsigen.kreuzung import build_objects
from omsigen.network import build
from omsigen.splinedb import SplineDB
from omsigen.writer import write_map


def arm(name, h):
    return dict(name=name, h=h)


def test_gruppen_nur_gegenueber_gemeinsam():
    x = [arm('A', 0), arm('B', 90), arm('A', 180), arm('B', 270)]
    assert ampel.gruppen(x, ['neben', 'haupt', 'neben', 'haupt']) == [1, 0, 1, 0]   # Hauptstrasse zuerst
    # fuenf Arme, vier davon Hauptstrasse: nie kreuzende Richtungen in einer Phase
    x = [arm('L', 0), arm('F', 70), arm('L', 180), arm('L', 215), arm('F', 250)]
    ph = ampel.gruppen(x, ['haupt', 'haupt', 'haupt', 'neben', 'haupt'])
    for i in range(5):
        for j in range(i + 1, 5):
            if ph[i] == ph[j]:
                assert abs((x[i]['h'] - x[j]['h'] + 180) % 360 - 180) > 140


def test_programm_zeiten():
    umlauf, progs = ampel.programm(2)
    assert umlauf == 70 and len(progs) == 2
    for p in progs:
        assert p[-1] == (0, 0.0) and [z for z, _ in p if z] == [3, 6, 9]      # rot-gelb, gruen, gelb
    g1 = dict((z, s) for z, s in progs[0])[6]
    g2 = dict((z, s) for z, s in progs[1])[6]
    assert g1 > g2                                       # Hauptrichtung laenger gruen
    assert progs[1][0] == (0, 2 + g1 + 3 + 3)            # zweite Phase beginnt nach Gelb und Raeumzeit
    assert ampel.programm(3)[0] == 85


def test_ampel_in_karte(tmp_path):
    """Kreuzung mit OSM-Ampel: Programm und gebundene Zufahrten im Objekt, Signale mit [varparent] in der Karte"""
    ways = [dict(tags=dict(highway='secondary', name='Haupt'), P=[(-80.0, 0.0), (0.0, 0.0), (80.0, 0.0)]),
            dict(tags=dict(highway='residential', name='Neben'), P=[(0.0, -80.0), (0.0, 0.0), (0.0, 80.0)])]
    sdb = SplineDB()
    net = build(ways, sdb, [dict(kind='signals', p=(0.5, 0.5), direction=None)])
    objs = build_objects(net, sdb, 'T')
    (j,) = [o for o in objs if o['vorfahrt']]
    assert j['vorfahrt']['ampel'] and j['vorfahrt']['umlauf'] == 70
    assert j['sco'].count('[traffic_light]') == 2 and j['sco'].count('[use_traffic_light]') >= 4
    arten = [s['art'] for s in j['signale']]
    assert arten == ['signal', 'mast', 'oben'] * 4                         # je Zufahrt Signal, Mast, Signal oben
    info = write_map(str(tmp_path), 'T', net['road_chains'], [], junctions=objs)
    txt = ''.join(p.read_bytes().decode('utf-16') for p in (tmp_path / 'T').glob('tile_*.map'))
    assert txt.count('Ampel_Kfz_1.sco') == 4 and txt.count('[varparent]') == 8
    assert txt.count('[attachObj]') == 4 and txt.index('whip_beam.sco') < txt.index('[attachObj]')
    ai = (tmp_path / 'T' / 'ailists.cfg').read_text(encoding='cp1252')
    zeilen = ai.splitlines()
    assert zeilen[zeilen.index('[aigroup_2]') + 1] == 'NormalCars'
    g = (tmp_path / 'T' / 'global.cfg').read_bytes().decode('utf-16')
    assert g.count('[trafficdensity_road]') == 11
