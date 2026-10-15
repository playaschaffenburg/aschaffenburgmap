"""F4 Belegungskarte: Fahrbahn/Gehweg aus netz.json, Gebaeude aus der Schicht, Konflikte, Aktualitaet."""
import json
import os

from omsigen import belegung as B, config, kreuzung, netz_datei as N, schichten as S, writer
from omsigen.sonderbau_ring import Mesh

BS = chr(92)
SLI = config.MARCEL + 'str_2spur_8m_altonaer1.sli'   # Fahrbahn +-4 m, aussen +-7 m


def gebaeude(omsi, karte, k, name, x0, z0, breite):
    """quadratisches Gebaeude (Dach) als Objekt der Schicht gebaeude"""
    rel = BS.join(['Sceneryobjects', 'Aschaffenburg', karte, 'LoD2', f'{k[0]}_{k[1]}', name + '.sco'])
    ordner = os.path.join(omsi, *rel.split(BS)[:-1])
    os.makedirs(os.path.join(ordner, 'model'), exist_ok=True)
    m = Mesh()
    h = breite / 2
    m.quad('dach', [(-h, -h, 8), (h, -h, 8), (h, h, 8), (-h, h, 8)], [(0, 0), (1, 0), (1, 1), (0, 1)], (0, 0, 1))
    open(os.path.join(ordner, 'model', name + '.x'), 'w').write(kreuzung.x_file(m.V, m.F, [('dach', 'd.bmp')]))
    open(os.path.join(ordner, name + '.sco'), 'w').write('[mesh]\r\n' + name + '.x\r\n')
    return dict(name=name, rel=rel, x=x0, y=z0, z=0.0)


def test_belegung_und_konflikt(tmp_path):
    omsi = str(tmp_path)
    els = [(100.0, 100.0, 90.0, 200.0, 0.0, SLI)]               # Strasse nach Osten bei z = 100
    info = writer.write_map(os.path.join(omsi, 'maps'), 'K', [dict(els=els)], [])
    d = info['dir']
    ox, oz = info['offset']
    N.schreiben(d, omsi)
    # Mitte der Strasse in Kartenmetern
    sx, sz = 200.0 - ox, 100.0 - oz
    k = (int(sx // 300), int(sz // 300))
    objekte = [gebaeude(omsi, 'K', k, 'B1', sx, sz + 30.0, 10.0),      # frei neben der Strasse
               gebaeude(omsi, 'K', k, 'B2', sx + 50.0, sz + 2.0, 6.0)]  # steht halb auf der Fahrbahn
    p = S.paket('K', 'gebaeude', k, objekte, S.manifest(d, 'gebaeude', k, 'test'))
    pf = os.path.join(omsi, 'paket.json')
    json.dump(p, open(pf, 'w', encoding='utf-8'))
    S.einsetzen(d, pf)
    erg = B.berechnen(d, omsi)
    bel = B.Belegung.laden(d)
    assert bel.klasse(sx, sz) == 'fahrbahn'
    assert bel.klasse(sx, sz + 5.5) == 'gehweg' and bel.klasse(sx, sz - 5.5) == 'gehweg'
    assert bel.klasse(sx, sz + 30.0) == 'gebaeude' and bel.klasse(sx, sz + 20.0) == 'frei'
    assert len(erg['konflikte']) == 1 and erg['konflikte'][0]['objekt'].endswith('B2.sco')
    assert 25.0 < erg['konflikte'][0]['flaeche_m2'] < 40.0          # 6 m x 5 m auf der Fahrbahn, Rasterrand grosszuegig
    assert os.path.isfile(os.path.join(d, 'belegung', f'tile_{k[0]}_{k[1]}.png'))
    # Strasse geaendert -> Belegung veraltet
    t = S.lesen(d, k).replace('altonaer1', 'staakener1')
    S.schreiben(d, k, t)
    assert bel.veraltet() == [f'{k[0]}_{k[1]}']
