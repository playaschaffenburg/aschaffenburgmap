"""S4 Boden: Malmasken je Kachel (Klassifikation, Georeferenz, weiche Kanten, DDS-Format), Schicht boden, Kommandozeile,
Pruefbericht."""
import json
import os
import struct
import subprocess
import sys

import numpy as np

from omsigen import belegung as B, boden as BO, config, netz_datei as N, pruefbericht as PB, schichten as S, writer

SLI = config.MARCEL + 'str_2spur_8m_altonaer1.sli'   # Fahrbahn +-4 m, aussen +-7 m
WURZEL = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


# ---------------------------------------------------------------- Format

def test_dds_a8_wie_standardmasken():
    a = (np.arange(64 * 32) % 256).astype(np.uint8).reshape(32, 64)
    d = BO.dds_a8(a)
    assert len(d) == 128 + 64 * 32
    f = struct.unpack('<4s31I', d[:128])
    # Kopf wie die Masken in Grundorf/Spandau: Flags 0x1007, Pixelformat ALPHA (2), 8 Bit, Alphamaske 0xFF, Caps 0x1002
    assert f[0] == b'DDS ' and f[1] == 124 and f[2] == 0x1007 and (f[3], f[4]) == (32, 64)
    assert f[19:23] == (32, 2, 0, 8) and f[26] == 0xFF and f[27] == 0x1002 and f[7] == 0     # keine Mip-Stufen
    assert np.array_equal(BO.dds_lesen(d), a)


def test_groundtex_eintraege():
    z = BO.groundtex_zeilen()
    bl = [z[i:i + 6] for i in range(0, len(z), 7)]
    assert [b[0] for b in bl] == ['[groundtex]'] * len(BO.KLASSEN)
    assert bl[0][3] == '0' and all(b[3] == str(BO.EXP) for b in bl[1:])       # Grund ohne Maske, Rest 512 x 512
    # ohne Installation: Rueckfall auf Standard-OMSI-Texturen, wenn keine Kandidaten da sind
    assert BO.belaege(os.path.join(WURZEL, 'gibt_es_nicht'))[1]['textur'] == BO.BELAEGE[1][1][0][0]


def test_wasser_textur_kachelbar():
    t = np.asarray(BO.wasser_textur(64), float)
    # nahtlos: Spruenge ueber den Rand nicht groesser als innen
    innen = np.abs(np.diff(t, axis=1)).mean()
    rand = np.abs(t[:, 0] - t[:, -1]).mean()
    assert rand < 3 * innen + 1 and t[..., 2].mean() > t[..., 0].mean()       # gruenblau


# ---------------------------------------------------------------- Klassifikation

def test_dop_klassen():
    farben = {'gruen': (70, 110, 50), 'asphalt': (95, 97, 99), 'beton': (180, 178, 175), 'erde': (150, 120, 85),
              'ziegel': (175, 95, 70), 'schatten': (30, 32, 40)}
    rgb = np.zeros((6, 20, 20, 3))
    for i, f in enumerate(farben.values()):
        rgb[i] = f
    k = BO.dop_klassen(rgb.reshape(120, 20, 3), glaetten=0).reshape(6, 20, 20)[:, 10, 10]
    assert list(k) == [BO.D_GRUEN, BO.D_DUNKEL, BO.D_HELL, BO.D_OFFEN, BO.D_ROT, BO.D_UNBEKANNT]


def test_osm_regeln():
    r = BO.osm_regel
    assert r({'leisure': 'park'}, True)[1:3] == ('kontext', BO.K_GRUEN)
    assert r({'landuse': 'farmland'}, True)[1:3] == ('kontext', BO.K_FELD)
    assert r({'amenity': 'parking'}, True)[1:3] == ('fest', BO.ASPHALT)
    assert r({'amenity': 'parking', 'surface': 'paving_stones'}, True)[2] == BO.PFLASTER
    assert r({'amenity': 'parking', 'parking': 'multi-storey'}, True)[1] == 'gebaeude'
    assert r({'amenity': 'parking', 'parking': 'underground'}, True) is None
    assert r({'place': 'square'}, True)[1:3] == ('fest', BO.PFLASTER)
    assert r({'highway': 'pedestrian', 'area': 'yes'}, True)[2] == BO.PFLASTER
    assert r({'building': 'yes'}, True)[1] == 'gebaeude'
    assert r({'natural': 'water'}, True)[2] == BO.WASSER
    assert r({'highway': 'footway'}, False) == (BO.R_LINIE, 'fest', BO.PFLASTER, 2.0)
    assert r({'highway': 'path', 'surface': 'gravel', 'width': '3'}, False)[2:] == (BO.SCHOTTER, 3.0)
    assert r({'highway': 'track', 'tracktype': 'grade5'}, False)[2] == BO.ERDE
    assert r({'railway': 'rail'}, False)[2] == BO.SCHOTTER
    assert r({'highway': 'footway', 'tunnel': 'yes'}, False) is None
    assert r({'highway': 'footway', 'bridge': 'yes'}, False) is None
    assert r({'landuse': 'residential'}, True) is None
    # Rangfolge: Kontext < fest < Gebaeude < Linie < Wasser
    assert BO.R_KONTEXT < BO.R_FEST < BO.R_GEBAEUDE < BO.R_LINIE < BO.R_WASSER


def test_osm_merkmale_ringe_und_reihenfolge():
    def geom(pts):
        return [dict(lat=z, lon=x) for x, z in pts]
    quadrat = [(0, 0), (100, 0), (100, 100), (0, 100), (0, 0)]
    js = dict(elements=[
        dict(type='way', id=1, tags={'amenity': 'parking'}, geometry=geom([(10, 10), (20, 10), (20, 20), (10, 20), (10, 10)])),
        dict(type='relation', id=2, tags={'type': 'multipolygon', 'leisure': 'park'},
             members=[dict(type='way', role='outer', geometry=geom(quadrat[:3])),
                      dict(type='way', role='outer', geometry=geom(quadrat[2:])),
                      dict(type='way', role='inner', geometry=geom([(40, 40), (60, 40), (60, 60), (40, 60), (40, 40)]))]),
        dict(type='way', id=3, tags={'highway': 'footway'}, geometry=geom([(0, 50), (100, 50)])),
    ])
    m = BO.osm_merkmale(js, lambda la, lo: (lo, la))
    assert [x['id'] for x in m] == [2, 1, 3]                   # Kontext, fest, Linie
    park = m[0]
    assert len(park['aussen']) == 1 and len(park['aussen'][0]) == 5 and len(park['innen']) == 1
    assert abs(park['flaeche_m2'] - 10000) < 1


def test_georeferenz_zeile_null_ist_norden():
    """Flaeche 10..20 m oestlich und 280..290 m noerdlich der Kachelecke -> Zeilen 20..40 (von Norden), Spalten 20..40"""
    k = (2, 3)
    x0, z0 = k[0] * 300.0, k[1] * 300.0
    m = [dict(rang=BO.R_FEST, art='fest', wert=BO.PFLASTER, innen=[], flaeche_m2=100.0,
              aussen=[[(x0 + 10, z0 + 280), (x0 + 20, z0 + 280), (x0 + 20, z0 + 290), (x0 + 10, z0 + 290), (x0 + 10, z0 + 280)]])]
    kontext, klasse, gesetzt = BO.merkmale_zeichnen(m, k)
    jj, ii = np.nonzero(gesetzt)
    assert 19 <= jj.min() <= 21 and 39 <= jj.max() <= 41 and 19 <= ii.min() <= 21 and 39 <= ii.max() <= 41
    assert (klasse[gesetzt] == BO.PFLASTER).all() and (kontext == BO.K_KEIN).all()
    # im Raster der Masken (512): dieselbe Ecke oben links
    kl = BO.klassifizieren(None, kontext, klasse, gesetzt)
    w = BO.anteile(kl)
    jj, ii = np.nonzero(w[BO.PFLASTER] > 0.5)
    assert abs((ii.mean() + 0.5) * 300 / 512 - 15) < 1 and abs(300 - (jj.mean() + 0.5) * 300 / 512 - 285) < 1


def test_klassifizieren_vorrang_und_fuellen():
    n = BO.N
    dop = np.full((n, n), BO.D_GRUEN, np.uint8)
    dop[:, :100] = BO.D_UNBEKANNT                       # Schatten links -> aus der Nachbarschaft
    kontext = np.zeros((n, n), np.uint8)
    kontext[300:, :] = BO.K_FELD                        # Feld unten: unbekannt = Erde
    osm_k = np.zeros((n, n), np.uint8)
    osm_g = np.zeros((n, n), bool)
    osm_k[:, 200:220] = BO.SCHOTTER
    osm_g[:, 200:220] = True                            # Weg
    bel = np.zeros((n, n), np.uint8)
    bel[100:110, :] = B.FAHRBAHN                        # Strasse quer: gewinnt ueber alles
    bel[400:420, 400:420] = B.GEBAEUDE                  # Gebaeude: Boden unbekannt -> gefuellt
    kl = BO.klassifizieren(dop, kontext, osm_k, osm_g, bel)
    assert (kl[100:110] == BO.ASPHALT).all()
    assert (kl[150, 200:220] == BO.SCHOTTER).all() and kl[150, 300] == BO.GRAS
    assert kl[200, 80] == BO.GRAS and kl[500, 50] == BO.ERDE      # Schatten: Nachbarschaft bzw. Feld
    assert kl[410, 410] == BO.GRAS and (kl != BO.UNBEKANNT).all()
    # ohne Luftbild: nur, was sicher ist; sonst der Grund
    kl2 = BO.klassifizieren(None, None, osm_k, osm_g, bel)
    assert kl2[50, 50] == BO.GRUND and (kl2[100:110] == BO.ASPHALT).all()


def test_weiche_kanten_und_deckkraft():
    n = BO.N
    kl = np.full((n, n), BO.GRAS, np.uint8)
    kl[:, 300:] = BO.ASPHALT
    kl[:100, :100] = BO.PFLASTER
    w = BO.anteile(kl)
    assert np.allclose(w.sum(axis=0), 1, atol=1e-4)
    zeile = w[BO.ASPHALT, 300]
    rand = (zeile > 0.05) & (zeile < 0.95)
    assert 2 <= rand.sum() <= 8                        # weicher Uebergang von etwa 1-3 m, nicht hart, nicht verwaschen
    alphas = BO.deckkraft(w)
    assert len(alphas) == len(BO.KLASSEN) - 1 and all(a.dtype == np.uint8 for a in alphas)
    sicht = BO.ueberblenden(alphas)
    assert np.abs(sicht - w).max() < 0.02             # Uebereinanderblenden ergibt genau die Anteile
    assert alphas[BO.WASSER - 1].max() == 0


# ---------------------------------------------------------------- Karte, Schicht, Kommandozeile, Pruefbericht

def _karte(tmp_path):
    omsi = str(tmp_path)
    els = [(100.0, 100.0, 90.0, 200.0, 0.0, SLI)]               # Strasse nach Osten bei z = 100
    info = writer.write_map(os.path.join(omsi, 'maps'), 'K', [dict(els=els)], [])
    d = info['dir']
    N.schreiben(d, omsi)
    B.berechnen(d, omsi)
    ox, oz = info['offset']
    return omsi, d, (200.0 - ox, 100.0 - oz)


def test_boden_bauen_schicht_und_pruefung(tmp_path):
    omsi, d, (sx, sz) = _karte(tmp_path)
    k = (int(sx // 300), int(sz // 300))
    erg = BO.bauen(d, omsi, luftbild=False, mit_osm=False, log=lambda *a: None)
    assert erg['kacheln'] == len(S.kacheln(d)) and erg['masken'] >= 1
    # Fahrbahn = Asphalt an der Strasse, daneben Grund; global.cfg hat die Belaege
    kl = BO.kachel_klassen(d, k)
    i = int((sx - k[0] * 300) / 300 * 512)
    j = int((300 - (sz - k[1] * 300)) / 300 * 512)
    assert kl[j, i] == BO.ASPHALT and kl[j - 25, i] in (BO.PFLASTER, BO.GRUND) and kl[j - 60, i] == BO.GRUND
    assert [g[0] for g in BO.groundtex_lesen(d)] == [b['textur'] for b in BO.belaege(omsi)]
    assert os.path.isfile(os.path.join(d, 'texture', 'map', f'tile_{k[0]}_{k[1]}.map.{BO.ASPHALT}.dds'))
    # Schicht boden: aktuell; Masken aendern das Gelaende als Eingabe spaeterer Schichten nicht
    st = S.status(d)
    assert st[f'{k[0]}_{k[1]}']['boden']['zustand'] == 'aktuell'
    assert S.eingabe(d, k, 'boden') == S.fingerabdruck(d, k, 'boden', nur_gelaende=True) != S.fingerabdruck(d, k, 'boden')
    assert PB.boden_funde(d, B.Belegung.laden(d, pruefen=False)) == []
    # Maske von Hand geaendert -> geaendert; Fahrbahn auf Gras -> Fund
    p = os.path.join(d, 'texture', 'map', f'tile_{k[0]}_{k[1]}.map.{BO.ASPHALT}.dds')
    open(p, 'wb').write(BO.dds_a8(np.zeros((512, 512), np.uint8)))
    open(os.path.join(d, 'texture', 'map', f'tile_{k[0]}_{k[1]}.map.{BO.GRAS}.dds'), 'wb').write(BO.dds_a8(np.full((512, 512), 255, np.uint8)))
    assert S.status(d)[f'{k[0]}_{k[1]}']['boden']['zustand'] == 'geaendert'
    funde = PB.boden_funde(d, B.Belegung.laden(d, pruefen=False))
    assert [f['art'] for f in funde] == ['boden_fahrbahn_gras'] and abs(funde[0]['z'] - sz) < 3
    # Strasse geaendert (netz) -> boden veraltet
    t = S.lesen(d, k).replace('[spline]\n0\n', '[spline]\n1\n', 1).replace('[spline]\r\n0\r\n', '[spline]\r\n1\r\n', 1)
    S.schreiben(d, k, t)
    st = S.status(d)[f'{k[0]}_{k[1]}']['boden']
    assert st['zustand'] == 'veraltet' and 'netz' in st['grund']


def test_kommandozeile_einzelne_kachel(tmp_path):
    omsi, d, (sx, sz) = _karte(tmp_path)
    k = (int(sx // 300), int(sz // 300))
    BO.bauen(d, omsi, luftbild=False, mit_osm=False, log=lambda *a: None)
    andere = [x for x in S.kacheln(d) if x != k][0]
    vorher = BO.masken_hash(d, andere)
    for f in os.listdir(os.path.join(d, 'texture', 'map')):
        if f.startswith(f'tile_{k[0]}_{k[1]}.'):
            os.remove(os.path.join(d, 'texture', 'map', f))
    assert BO.kachel_klassen(d, k) is None
    ix = S.index_lesen(d)
    del ix['kacheln'][f'{k[0]}_{k[1]}']['boden']                  # wie eine Kachel, die nach dem Bau dazukam
    S.index_schreiben(d, ix)
    assert [f['art'] for f in PB.boden_funde(d, None)] == ['boden_fehlt']
    p = subprocess.run([sys.executable, '-m', 'omsigen.boden', d, '--omsi', omsi, '--kacheln', f'{k[0]}_{k[1]}',
                        '--ohne-luftbild', '--ohne-osm'], capture_output=True, text=True, cwd=WURZEL, timeout=300)
    assert p.returncode == 0, p.stderr
    erg = json.loads(p.stdout.strip().splitlines()[-1])
    assert erg['kacheln'] == 1 and erg['global_geaendert'] is False
    assert BO.kachel_klassen(d, k) is not None and BO.masken_hash(d, andere) == vorher
    assert S.status(d)[f'{k[0]}_{k[1]}']['boden']['zustand'] == 'aktuell'


def test_groundtex_fremde_masken_bleiben(tmp_path):
    """Karte mit im OMSI-Editor gemalten Masken (ohne boden.json): global.cfg wird nicht ersetzt"""
    omsi, d, _ = _karte(tmp_path)
    os.makedirs(os.path.join(d, 'texture', 'map'), exist_ok=True)
    open(os.path.join(d, 'texture', 'map', 'tile_0_0.map.1.dds'), 'wb').write(BO.dds_a8(np.zeros((8, 8), np.uint8)))
    text = open(os.path.join(d, 'global.cfg'), encoding='utf-16').read()
    text = text.replace('[groundtex]', '[groundtex_alt]', 1)          # eine abweichende Liste
    open(os.path.join(d, 'global.cfg'), 'w', encoding='utf-16').write(text)
    try:
        BO.groundtex_einrichten(d, omsi, log=lambda *a: None)
        assert False, 'haette abbrechen muessen'
    except RuntimeError as e:
        assert 'gemalte Masken' in str(e)
