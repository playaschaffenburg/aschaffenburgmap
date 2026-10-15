"""F3 netz.json: Kanten aus verketteten Splines, Knoten, Abfragen fuer spaetere Schichten."""
import math

from omsigen import config, netz_datei as N, writer

BS = chr(92)
SLI = config.MARCEL + 'str_2spur_8m_altonaer1.sli'       # bekannte Spurdaten ohne OMSI (FALLBACK_SPLINES)


def kette(x, z, h, stuecke, sli=SLI):
    """Elemente (x, z, h, L, R, sli) aus (L, R)-Stuecken, jedes beginnt genau am Ende des vorigen"""
    els = []
    for L, R in stuecke:
        els.append((x, z, h, L, R, sli))
        x, z, h = N.element_bei(x, z, h, L, R, L)
    return dict(els=els), (x, z, h)


def test_element_bei_rechtskurve():
    # Viertelkreis Radius 10 nach Norden gestartet, Rechtskurve: endet 10 m oestlich, 10 m noerdlich, Richtung Ost
    x, z, h = N.element_bei(0.0, 0.0, 0.0, math.pi * 5, 10.0, math.pi * 5)
    assert abs(x - 10) < 1e-9 and abs(z - 10) < 1e-9 and abs(h - 90) < 1e-9


def test_kanten_knoten_und_abfragen(tmp_path):
    a, ende_a = kette(100.0, 100.0, 90.0, [(40.0, 0.0), (30.0, 50.0), (20.0, 0.0)])
    b, _ = kette(400.0, 100.0, 0.0, [(60.0, 0.0)])
    k = dict(origin=(ende_a[0] + 5.0, ende_a[1]), rel=BS.join(['Sceneryobjects', 'Aschaffenburg', 'K', 'K_001.sco']))
    info = writer.write_map(str(tmp_path), 'K', [a, b], [], junctions=[k])
    d = str(tmp_path / 'K')
    ox, oz = info['offset']
    N.attribute_schreiben(d, {sid: dict(art='strasse', name='Teststrasse', highway='residential') for sid in info['ketten_ids'][0]})
    n = N.Netz(N.schreiben(d))
    assert len(n.kanten) == 2
    ka = n.kanten[f'E{info["ketten_ids"][0][0]}']
    assert ka['splines'] == info['ketten_ids'][0] and abs(ka['laenge'] - 90.0) < 1e-6
    assert ka['attribute']['name'] == 'Teststrasse' and ka['art'] == 'strasse'
    assert ka['querschnitt']['fahrbahn'] == [-4.0, 4.0]
    # Ende der Kante = Ende der Kette (Bogen richtig gerechnet), Seitenversatz rechts der Fahrtrichtung
    x, y, z, h = n.punkt(ka['id'], ka['laenge'])
    assert abs(x - (ende_a[0] - ox)) < 0.05 and abs(z - (ende_a[1] - oz)) < 0.05
    x0, _, z0, h0 = n.punkt(ka['id'], 10.0)
    x1, _, z1, _ = n.punkt(ka['id'], 10.0, seite=3.0)
    assert abs(h0 - 90.0) < 1e-6 and abs(x1 - x0) < 1e-9 and abs(z1 - (z0 - 3.0)) < 1e-9   # Richtung Ost: rechts = Sued
    # naechste Kante, Querlage
    eid, st, seite, dist = n.naechste(x1, z1)
    assert eid == ka['id'] and abs(st - 10.0) < 1e-6 and abs(seite - 3.0) < 1e-6 and abs(dist - 3.0) < 1e-6
    # Umriss der Fahrbahn: Flaeche ~ Laenge x Breite
    poly = n.streifen(ka['id'], *ka['querschnitt']['fahrbahn'])
    fl = abs(sum(p[0] * q[1] - q[0] * p[1] for p, q in zip(poly, poly[1:] + poly[:1]))) / 2
    assert abs(fl - 90.0 * 8.0) < 90.0 * 8.0 * 0.03
    # Knoten: Ende von a an der Kreuzung K_001, sonst Enden
    assert n.knoten[ka['nach']]['art'] == 'kreuzung' and n.knoten[ka['von']]['art'] == 'ende'
    assert n.veraltet(d) == []


def test_netz_veraltet_nach_aenderung(tmp_path):
    from omsigen import schichten as S
    a, _ = kette(100.0, 100.0, 90.0, [(40.0, 0.0)])
    writer.write_map(str(tmp_path), 'K', [a], [])
    d = str(tmp_path / 'K')
    N.schreiben(d)
    k = next(k for k in S.kacheln(d) if 'altonaer1' in S.lesen(d, k))
    S.schreiben(d, k, S.lesen(d, k).replace('altonaer1', 'staakener1'))
    n = N.Netz.laden(d, pruefen=False)
    assert n.veraltet(d) == [f'{k[0]}_{k[1]}']
