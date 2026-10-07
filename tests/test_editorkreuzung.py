"""Kreuzungsobjekt fuer den Rust-Editor (omsigen.editorkreuzung)"""
import json, os, subprocess, sys
from omsigen import editorkreuzung
from omsigen.config import MARCEL
from omsigen.splinedb import SplineDB

SPL = MARCEL + 'str_2spur_8m_altonaer1.sli'


def auftrag(tmp_path, rolle_neben=None):
    return dict(omsi=str(tmp_path / 'omsi'), ordner=str(tmp_path / 'K'), rel_ordner=r'Sceneryobjects\Aschaffenburg_KI\T',
                name='K_E0001', titel='Test',
                arme=[dict(pos=[0, -12], h=180, sli=SPL, away=False, rolle='haupt'),
                      dict(pos=[0, 12], h=0, sli=SPL, away=True, rolle='haupt'),
                      dict(pos=[12, 0], h=90, sli=SPL, away=True, rolle=rolle_neben or 'neben')])


def test_t_kreuzung(tmp_path):
    erg = editorkreuzung.bauen(auftrag(tmp_path), SplineDB())
    assert erg['rel'] == r'Sceneryobjects\Aschaffenburg_KI\T\K_E0001.sco'
    assert erg['fehlgeschlagen'] == 0
    # T-Kreuzung mit je einer Spur pro Richtung: 2 geradeaus, 2 rechts, 2 links
    assert erg['bewegungen'] == {'gerade': 2, 'rechts': 2, 'links': 2}
    assert abs(erg['ursprung'][0] - 4) < 1e-6 and abs(erg['ursprung'][1]) < 1e-6
    assert erg['rules'] and all(v in (64, 192) for _, v in erg['rules'])
    sco = open(tmp_path / 'K' / 'K_E0001.sco', encoding='cp1252').read()
    assert sco.count('[path]') == erg['pfade'] and '[mesh]' in sco
    assert os.path.exists(tmp_path / 'K' / 'model' / 'K_E0001.x')


def test_kommandozeile_meldet_fehler(tmp_path):
    a = auftrag(tmp_path)
    a['arme'] = a['arme'][:2]
    r = subprocess.run([sys.executable, '-m', 'omsigen.editorkreuzung'], input=json.dumps(a), capture_output=True,
                       text=True, cwd=os.path.dirname(os.path.dirname(__file__)))
    assert 'mindestens 3 Arme' in json.loads(r.stdout)['fehler']


def test_ampelkreuzung(tmp_path):
    a = auftrag(tmp_path)
    a['ampel'] = True
    erg = editorkreuzung.bauen(a, SplineDB())
    # T-Kreuzung: Hauptstrasse (2 Arme) eine Phase, Nebenstrasse die zweite
    assert erg['phasen'] == [0, 0, 1] and 60 <= erg['umlauf'] <= 100
    sco = open(tmp_path / 'K' / 'K_E0001.sco', encoding='cp1252').read()
    assert '[traffic_lights_group]' in sco and sco.count('[traffic_light]') == 2 and '[use_traffic_light]' in sco
    # je Zufahrt mit ankommenden Spuren: Signal, Mast, Signal oben am Mast
    arten = [g['art'] for g in erg['signale']]
    assert arten.count('signal') == 3 and arten.count('mast') == 3 and arten.count('oben') == 3
    for g in erg['signale']:
        if g['art'] == 'oben':
            assert erg['signale'][g['eltern']]['art'] == 'mast'
        else:
            assert g['x'] is not None and g['y'] is not None


def kreisel_auftrag(tmp_path, winkel=(0, 90, 180, 270), r=12.0, breite=7.0, d=27.0):
    import math
    arme = []
    for w in winkel:
        p = [d * math.sin(math.radians(w)), d * math.cos(math.radians(w))]
        arme.append(dict(pos=p, h=w, sli=SPL, away=True))
    return dict(omsi=str(tmp_path / 'omsi'), ordner=str(tmp_path / 'K'), rel_ordner=r'Sceneryobjects\Aschaffenburg_KI\T',
                name='KV_0001', titel='Kreisel', arme=arme, kreisel=dict(mitte=[0, 0], r=r, breite=breite))


def test_kreisverkehr_ein_objekt(tmp_path):
    import math
    from omsigen import kreisel
    from omsigen.geom import end_of
    a = kreisel_auftrag(tmp_path, winkel=(10, 95, 200, 290))
    erg = editorkreuzung.bauen(a, SplineDB())
    assert erg['rel'].endswith(r'\KV_0001.sco') and erg['ursprung'] == [0.0, 0.0]
    sco = open(tmp_path / 'K' / 'KV_0001.sco', encoding='cp1252').read()
    assert sco.count('[path]') == erg['pfade']
    arms = [dict(pos=tuple(x['pos']), h=x['h'], spl=x['sli'], away=x['away']) for x in a['arme']]
    K = kreisel.bauen(arms, (0, 0), 12.0, 7.0, SplineDB())
    ring = [m[0] for m in K['moves'] if m[0][0][4] == -12.0 and len(m[0]) == 1]
    # Ringspur geschlossen: jedes Bogenende ist der Anfang des naechsten
    assert len(ring) == 8
    for b1, b2 in zip(ring, ring[1:] + ring[:1]):
        p, h = end_of(b1[0])
        assert math.dist(p, b2[0][:2]) < 1e-6 and abs((h - b2[0][2] + 180) % 360 - 180) < 1e-6
    starts = [tuple(b[0][:2]) for b in ring]
    sonst = [m for m in K['moves'] if m[0] not in ring]
    assert len(sonst) == 8                          # je Arm eine Ein- und eine Ausfahrt
    for els, blinker in sonst:
        anfang, (ende, _) = tuple(els[0][:2]), end_of(els[-1])
        assert any(math.dist(anfang, q) < 1e-6 for q in starts) or any(math.dist(ende, q) < 1e-6 for q in starts)
        for e1, e2 in zip(els, els[1:]):
            assert math.dist(end_of(e1)[0], e2[:2]) < 1e-6
    # Ein- und Ausfahrten kreuzen sich nicht (nur gemeinsame Ringpunkte)
    from shapely.geometry import LineString
    from omsigen.geom import sample
    linien = [LineString([q for e in els for q, _ in sample(e, 0.5)]) for els, _ in sonst]
    for i in range(len(linien)):
        for j in range(i + 1, len(linien)):
            x = linien[i].intersection(linien[j])
            assert x.is_empty or x.length < 1e-6 and all(any(math.dist(pt.coords[0], q) < 1e-3 for q in starts) for pt in getattr(x, 'geoms', [x])), (i, j, x)
    # Vorfahrt: Ring 192, Einfahrten 64
    assert sorted(set(v for _, v in K['rules'])) == [64, 192]
    assert not K['asphalt'].is_empty and K['insel'].area > 100 and not K['side'].is_empty
    # rund: der Aussenrand der Fahrbahn weicht zwischen den Zufahrten kaum vom Kreis ab
    assert abs(K['asphalt'].exterior.distance(__import__('shapely').geometry.Point(0, 0)) - 15.5) < 0.05


def test_kreisverkehr_zu_klein(tmp_path):
    import pytest
    a = kreisel_auftrag(tmp_path, r=5.0, breite=7.0)
    with pytest.raises(ValueError):
        editorkreuzung.bauen(a, SplineDB())
