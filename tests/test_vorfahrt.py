import json
from omsigen import vorfahrt as V
from omsigen.network import build
from omsigen.splinedb import SplineDB


def arm(name, hw, h, sign=None, prio=False, ring=False, **tags):
    return dict(name=name, tags=dict(highway=hw, name=name, **tags), sign=sign, prio_road=prio, ring=ring, h=h)


T = lambda side_sign=None: [arm('Haupt', 'secondary', 90), arm('Haupt', 'secondary', 270),
                            arm('Neben', 'residential', 180, sign=side_sign)]


def test_schild_vor_klasse():
    d = V.decide([arm('A', 'residential', 0, sign='give_way'), arm('B', 'residential', 90),
                  arm('C', 'residential', 180, sign='give_way'), arm('D', 'residential', 270)])
    assert d['quelle'] == 'OSM Schild' and d['rollen'] == ['neben', 'haupt', 'neben', 'haupt']


def test_vorfahrtstrasse_kreisel_tempo30():
    d = V.decide([arm('A', 'tertiary', 0, prio=True), arm('B', 'secondary', 90), arm('A', 'tertiary', 180, prio=True)])
    assert d['quelle'] == 'OSM Vorfahrtstrasse' and d['rollen'] == ['haupt', 'neben', 'haupt']
    d = V.decide([arm('', 'secondary', 0, ring=True), arm('', 'secondary', 120, ring=True), arm('X', 'primary', 240)])
    assert d['quelle'] == 'StVO Kreisverkehr' and d['rollen'] == ['haupt', 'haupt', 'neben']
    d = V.decide([arm(n, 'residential', h, maxspeed='30') for n, h in (('A', 0), ('B', 90), ('C', 180))])
    assert d['quelle'] == 'StVO Tempo 30' and set(d['rollen']) == {'gleich'}


def test_vermutungen():
    assert V.decide(T())['rollen'] == ['haupt', 'haupt', 'neben']
    assert V.decide([arm('A', 'residential', h) for h in (0, 90, 180)])['rollen'] == ['gleich'] * 3
    d = V.decide([arm('Nordring', 'primary', 0), arm('Nordring', 'primary', 175), arm('Nordring', 'primary', 260)])
    assert d['rollen'] == ['haupt', 'haupt', 'neben']          # durchgehendes Paar statt rechts vor links
    d = V.decide([arm('A', 'secondary', 0), arm('Hof', 'service', 90), arm('A', 'secondary', 180)])
    assert d['quelle'] == 'StVO Paragraf 10' and d['rollen'] == ['haupt', 'neben', 'haupt']


def test_korrektur(tmp_path):
    f = tmp_path / 'k.json'
    f.write_text(json.dumps({'vorfahrt': [{'lat': 49.98, 'lon': 9.14, 'haupt': ['Neben']},
                                          {'lat': 49.99, 'lon': 9.14, 'regel': 'rechts_vor_links'}]}), encoding='utf-8')
    K = V.load_corrections(str(f))
    k = V.match_correction(K, 49.98010, 9.14010)                # 13 m daneben
    assert k['haupt'] == ['Neben'] and V.match_correction(K, 49.981, 9.14) is None
    d = V.decide(T(), korrektur=k)
    assert d['quelle'] == 'Korrektur' and d['rollen'] == ['neben', 'neben', 'haupt']
    assert V.decide(T(), korrektur=K[1])['rollen'] == ['gleich'] * 3


def test_prioritaeten_der_bewegungen():
    assert V.priority('haupt', 'haupt', 'gerade', 2) == 192
    assert V.priority('haupt', 'neben', 'rechts', 2) == 192
    assert V.priority('haupt', 'neben', 'links', 2) is None      # wartet auf Gegenverkehr
    assert V.priority('haupt', 'haupt', 'links', 2) == 192       # abknickende Vorfahrt
    assert V.priority('neben', 'haupt', 'rechts', 2) == 64
    assert V.priority('gleich', 'gleich', 'gerade', 0) is None


def test_schild_am_richtigen_arm():
    """Einmuendung: 'Vorfahrt gewaehren' 12 m vor der Kreuzung auf der Nebenstrasse"""
    ways = [dict(tags=dict(highway='secondary', name='Haupt'), P=[(-80.0, 0.0), (0.0, 0.0), (80.0, 0.0)]),
            dict(tags=dict(highway='residential', name='Neben'), P=[(0.0, -80.0), (0.0, 0.0)])]
    net = build(ways, SplineDB(), [dict(kind='give_way', p=(0.4, -12.0), direction=None),
                                   dict(kind='signals', p=(1.0, 1.0), direction=None)])
    (k, arms), = [(k, a) for k, a in net['arms'].items() if len(a) == 3]
    assert {a['name']: a['sign'] for a in arms} == {'Haupt': None, 'Neben': 'give_way'}
    assert 'signals' in net['node_flags'][k]
    d = V.decide(arms, net['node_flags'][k])
    assert d['quelle'] == 'OSM Schild' and d['ampel']


def test_bestaetigt():
    d = V.decide(T(), korrektur={'bestaetigt': True})
    assert d['quelle'] == 'bestaetigt' and d['rollen'] == ['haupt', 'haupt', 'neben'] and 'vermutet' not in d['text']
