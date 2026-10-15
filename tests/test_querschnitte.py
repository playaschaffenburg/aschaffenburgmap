"""S1 Querschnitte: Ableitung aus OSM-Tags, Einpassen in die Gebaeudeflucht, gemeinsame Dateien, .sli aus dem
Editor-Baukasten, Spurlagen an Kreuzungen"""
import json
import math
import os
import shutil

import pytest

from omsigen import querschnitte as Q
from omsigen.network import build, spurenden, key
from omsigen.splinedb import SplineDB, parse_sli, read_text


def arten(q):
    return [(t['art'], t['breite'], t['richtung']) for t in q['teile']]


def test_wohnstrasse_nach_regel():
    a = Q.ableiten({'highway': 'residential'})
    assert arten(a['q']) == [('gehweg', 2.0, 'vor'), ('fahrspur', 2.75, 'zurueck'), ('fahrspur', 2.75, 'vor'),
                             ('gehweg', 2.0, 'vor')]
    assert a['force'] == 0 and a['q']['marken'][1] == 'keine'          # Wohnstrasse ohne Mittellinie
    assert a['herkunft']['fahrbahn'] == 'regel' and a['herkunft']['gehweg'] == 'regel'
    i = Q.info(a['q'])
    assert i['lanes'] == [(-1.375, 1), (1.375, 0)] and i['walks'] == [-3.75, 3.75]
    assert (i['cl'], i['cr'], i['ol'], i['or_']) == (-2.75, 2.75, -4.75, 4.75)


def test_breite_spuren_gehweg_rad_parken_aus_tags():
    t = {'highway': 'secondary', 'width': '11,5 m', 'lanes': '3', 'lanes:forward': '2', 'lanes:backward': '1',
         'sidewalk': 'right', 'sidewalk:right:width': '3', 'cycleway:left': 'lane', 'parking:right': 'lane',
         'parking:right:orientation': 'parallel', 'surface': 'asphalt', 'maxspeed': '50'}
    a = Q.ableiten(t)
    q = a['q'] if a['force'] == 1 else Q.spiegeln(a['q'])           # in OSM-Richtung ansehen
    namen = [x[0] for x in arten(q)]
    assert namen == ['radfahrstreifen', 'fahrspur', 'fahrspur', 'fahrspur', 'parkstreifen', 'gehweg']
    i = Q.info(q)
    assert abs((i['cr'] - i['cl']) - 11.5) < 0.06, i               # Fahrbahn = width
    assert [d for _, d in i['lanes']] == [1, 0, 0]
    assert q['teile'][-1]['breite'] == 3.0
    assert a['herkunft']['fahrbahn'] == 'tag' and a['herkunft']['gehweg_rechts'] == 'tag'
    assert a['werte']['maxspeed'] == '50' and a['force'] != 0


def test_einbahn_gegen_osm_und_abbieger():
    a = Q.ableiten({'highway': 'tertiary', 'oneway': '-1', 'sidewalk': 'left', 'turn:lanes': 'left|through;right'})
    assert a['force'] == -1
    q = a['q']
    # Spline gegen OSM: der OSM-linke Gehweg liegt in Splinerichtung rechts
    assert q['teile'][-1]['art'] == 'gehweg' and q['teile'][0]['art'] != 'gehweg'
    assert sum(1 for t in q['teile'] if t['art'] == 'fahrspur') == 2
    assert a['werte']['abbieger'] == {'vor': ['left', 'through;right']}


def test_gleiche_querschnitte_teilen_eine_datei():
    sdb = SplineDB()
    qs = Q.Querschnitte(sdb, 'osm', messungen={}, exe='x')
    r1, f1 = qs.waehle({'highway': 'residential', 'osm_id': '1'}, False)
    r2, f2 = qs.waehle({'highway': 'residential', 'osm_id': '2', 'name': 'andere'}, False)
    assert r1 == r2 and r1.startswith(Q.PFAD) and len(qs.typen) == 1
    # unsymmetrisch, einmal in die andere Richtung gezeichnet: dieselbe Datei, Spline jeweils gleich herum
    a, fa = qs.waehle({'highway': 'residential', 'sidewalk': 'right', 'osm_id': '3'}, False)
    b, fb = qs.waehle({'highway': 'residential', 'sidewalk': 'left', 'osm_id': '4'}, False)
    assert a == b and fa == -fb and fa != 0
    assert len(qs.typen) == 2 and sdb[a] == Q.info(qs.typen[a]['q'])
    assert qs.herkunft({'osm_id': '3'}, a)['herkunft']['gehweg'] == 'tag'
    # Name gleichbleibend
    assert Q.datei(Q.ableiten({'highway': 'residential'})['q']) == os.path.basename(r1)[:-4]


def test_einpassen_in_die_gebaeudeflucht():
    # links stehen die Haeuser 4 m neben der OSM-Linie, rechts 9 m: Achsversatz nach rechts, nichts faellt weg
    a = Q.ableiten({'highway': 'residential', 'oneway': 'yes'}, messung={'flucht_links': 4.0, 'flucht_rechts': 9.0})
    i = Q.info(a['q'])
    assert a['q']['versatz'] > 0 and -i['ol'] <= 4.0 - Q.RAND_GEBAEUDE + 0.01 and i['or_'] <= 9.0
    assert a['herkunft']['versatz'] == 'messung'
    # Gasse, 3 m zu beiden Seiten: Gehwege fallen weg, Spuren schmaler (Gasse bis 1,75 m)
    a = Q.ableiten({'highway': 'living_street'}, messung={'flucht_links': 2.2, 'flucht_rechts': 2.2})
    assert [t['art'] for t in a['q']['teile']] == ['fahrspur', 'fahrspur']
    i = Q.info(a['q'])
    assert i['cr'] <= 1.9 + 1e-9 and a['herkunft']['fahrbahn'] == 'messung'
    # ohne Gebaeude in der Naehe bleibt alles
    assert Q.ableiten({'highway': 'residential'}, messung={'flucht_links': 12.0, 'flucht_rechts': None})['q'] == \
        Q.ableiten({'highway': 'residential'})['q']


def test_spurenden_an_kreuzungen_alle_verbunden():
    """T-Kreuzung und Querschnittswechsel mit unsymmetrischen Querschnitten: jede ankommende Spur hat eine
    Kreuzungsspur, jede abgehende einen Zulauf (keine offenen Enden)"""
    sdb = SplineDB()
    qs = Q.Querschnitte(sdb, 'osm', messungen={}, exe='x')
    ways = [dict(tags={'highway': 'secondary', 'lanes:forward': '2', 'lanes:backward': '1', 'sidewalk': 'right',
                       'osm_id': '10'}, P=[(-150, 0), (0, 0)]),
            dict(tags={'highway': 'secondary', 'osm_id': '11', 'cycleway': 'lane'}, P=[(0, 0), (150, 0)]),
            dict(tags={'highway': 'residential', 'oneway': 'yes', 'osm_id': '12'}, P=[(0, 120), (0, 0)]),
            dict(tags={'highway': 'residential', 'osm_id': '13', 'width': '6.5'}, P=[(150, 0), (300, 0)])]
    net = build(ways, sdb, waehle=qs.waehle)
    assert len(qs.typen) == 4
    enden = net['conn_chains']
    starts = {key((c['els'][0][0], c['els'][0][1])) for c in enden}
    from omsigen.geom import end_of
    ziele = {key(end_of(c['els'][-1])[0]) for c in enden}
    geprueft = 0
    for ch in net['road_chains']:
        for which, node in (('s', ch['node_s']), ('e', ch['node_e'])):
            if node not in net['junctions'] and list(node) not in net['junctions']:
                continue
            if which == 's':
                el = ch['els'][0]; pos, h = (el[0], el[1]), el[2]
            else:
                pos, h = end_of(ch['els'][-1])
            for l in spurenden(pos, h, (ch['els'][0] if which == 's' else ch['els'][-1])[5], which, sdb):
                assert key(l['p']) in (starts if l['kind'] == 'in' else ziele), (ch['name'], which, l)
                geprueft += 1
    assert len(net['junctions']) == 2 and geprueft == (3 + 2 + 1) + (2 + 2), geprueft


EXE = Q.editor_exe()


@pytest.mark.skipif(not EXE, reason='omsi-editor.exe nicht gebaut')
def test_sli_aus_dem_baukasten_passt_zu_info(tmp_path):
    """der Editor baut die .sli; was splinedb daraus liest, ist genau, was querschnitte.info vorhersagt (Netz,
    Kreuzungen und netz.json rechnen damit); die Bauanleitung liegt daneben"""
    sdb = SplineDB()
    qs = Q.Querschnitte(sdb, 'osm', messungen={}, exe=EXE)
    tags = [{'highway': 'residential'}, {'highway': 'secondary', 'lanes:forward': '2', 'lanes:backward': '1',
                                         'cycleway:right': 'track', 'parking:left': 'lane'},
            {'highway': 'tertiary', 'oneway': 'yes', 'sidewalk': 'left', 'bus:lanes': 'none|designated', 'lanes': '2'},
            {'highway': 'living_street'}]
    rels = [qs.waehle(t, False)[0] for t in tags]
    rels.append(qs.registrieren(Q.ableiten({'highway': 'residential', 'oneway': 'yes'},
                                           messung={'flucht_links': 3.0, 'flucht_rechts': 9.0})['q']))
    assert qs.bauen(str(tmp_path), log=lambda *a: None) == len(set(rels))
    for rel in set(rels):
        p = tmp_path / rel.replace('\\', '/')
        assert p.is_file()
        gelesen = parse_sli(read_text(str(p)))
        erwartet = Q.info(qs.typen[rel]['q'])
        assert gelesen['lanes'] == erwartet['lanes'] and gelesen['walks'] == erwartet['walks'], rel
        for k in ('cw', 'half', 'cl', 'cr', 'ol', 'or_'):
            assert abs(gelesen[k] - erwartet[k]) < 1e-6, (rel, k)
        bau = json.loads(p.with_suffix('').with_suffix('.qs.json').read_text(encoding='utf-8'))
        assert bau == Q.bauanleitung(qs.typen[rel]['q'], qs.typen[rel]['datei'])


def test_gebaeudeflucht_messen():
    """Haeuserzeile links 5 m neben der Weglinie, rechts 8 m, eine Luecke rechts: Quantil je Seite; Zwischenspeicher
    mit Pruefsumme"""
    from shapely import STRtree
    from shapely.geometry import box
    from omsigen import querschnitt_messung as M
    polys = [box(x, 5.0, x + 9.0, 15.0) for x in range(0, 100, 10)] + [box(x, -18.0, x + 9.0, -8.0) for x in range(0, 50, 10)]
    li, re = M.flucht([(0.0, 0.0), (100.0, 0.0)], STRtree(polys), polys)
    assert len(li) == len(re) == 22
    assert M.seite_auswerten(li) == 5.0
    assert M.seite_auswerten(re) == 8.0                    # die halbe Zeile reicht (Anteil >= 40 %)
    assert M.seite_auswerten([None] * 8 + [3.0] * 2) is None
    assert M.pruefsumme([(0, 0), (1, 1)], 500000, 5500000) != M.pruefsumme([(0, 0), (1, 2)], 500000, 5500000)
