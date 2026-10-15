"""G3/G4 Strassenausstattung und Haltestellen: Platzierung ueber Kante/Station/Seite auf dem Seitenraum, Abstaende,
Vorfahrtzeichen passend zur Vorfahrt der Karte, Zusammensetzung der Haltestellen, exakte Schrift, Schicht veraltet."""
import json
import math
import os

import pytest

from omsigen import (ausstattung as A, ausstattung_eigen as E, ausstattung_modelle as M, belegung as B, config,
                     haltestellen as H, kreuzung, netz_datei as N, schichten as S, writer)
from omsigen.sonderbau_ring import Mesh

BS = chr(92)
SLI = config.MARCEL + 'str_2spur_8m_altonaer1.sli'   # Fahrbahn +-4 m, aussen +-7 m (3 m Gehweg), Spuren +-2 m


# ---------------------------------------------------------------- Hilfen: eine kleine OMSI-Installation

def box_objekt(omsi, rel, box, anhaenge=()):
    """.sco mit einem Quader-Modell (box = xmin, ymin, zmin, xmax, ymax, zmax)"""
    pfad = os.path.join(omsi, *rel.split(BS))
    os.makedirs(os.path.join(os.path.dirname(pfad), 'model'), exist_ok=True)
    x0, y0, z0, x1, y1, z1 = box
    m = Mesh()
    m.quad('m', [(x0, z0, y1), (x1, z0, y1), (x1, z1, y1), (x0, z1, y1)], [(0, 0), (1, 0), (1, 1), (0, 1)], (0, 0, 1))
    m.quad('m', [(x0, z0, y0), (x1, z0, y0), (x1, z0, y1), (x0, z0, y1)], [(0, 0), (1, 0), (1, 1), (0, 1)], (0, -1, 0))
    name = os.path.splitext(os.path.basename(pfad))[0]
    open(os.path.join(os.path.dirname(pfad), 'model', name + '.x'), 'w').write(kreuzung.x_file(m.V, m.F, [('m', 't.bmp')]))
    t = '[mesh]\r\n' + name + '.x\r\n'
    for h in anhaenge:
        t += f'\r\n[new_attachment]\r\nattach_trans\r\n0\r\n0\r\n{h}\r\n'
    open(pfad, 'w').write(t)


KATALOG = {
    'typen': {
        'laterne_wohn': {'kandidaten': [{'rel': 'Sceneryobjects\\T\\lampe.sco', 'vorne': 0, 'radius': 0.15}]},
        'laterne_haupt': {'kandidaten': [{'rel': 'Sceneryobjects\\T\\peitsche.sco', 'vorne': 270, 'radius': 0.15}]},
        'schild_mast': {'kandidaten': [{'rel': 'Sceneryobjects\\T\\mast.sco', 'vorne': 0}]},
        'vz_205': {'kandidaten': [{'rel': 'Sceneryobjects\\T\\vz205.sco', 'vorne': 0, 'mast': True}]},
        'vz_206': {'kandidaten': [{'rel': 'Sceneryobjects\\T\\vz206.sco', 'vorne': 0, 'mast': True}]},
        'vz_274': {'kandidaten': [{'rel': 'Sceneryobjects\\T\\vz274.sco', 'vorne': 0, 'mast': True, 'texte': ['{wert}']}]},
        'wartehalle': {'kandidaten': [{'rel': 'Sceneryobjects\\T\\halle.sco', 'vorne': 180}]},
        'bank': {'kandidaten': [{'rel': 'Sceneryobjects\\T\\bank.sco', 'vorne': 180, 'radius': 0.6}]},
        'muelleimer': {'kandidaten': [{'rel': 'Sceneryobjects\\T\\eimer.sco', 'vorne': 0, 'radius': 0.3}]},
        'haltestelle_mast': {'kandidaten': [{'eigen': 'haltestellenmast', 'vorne': 0}]},
        'hochbord': {'kandidaten': [{'eigen': 'hochbord', 'vorne': 0}]},
        'stoerer': {'kandidaten': [{'rel': 'Sceneryobjects\\T\\stoerer.sco'}]},
    },
    'regeln': json.load(open(A.KATALOG, encoding='utf-8'))['regeln'],
}


def installation(omsi):
    box_objekt(omsi, 'Sceneryobjects\\T\\lampe.sco', (-0.3, 0, -0.3, 0.3, 4.4, 0.3))
    box_objekt(omsi, 'Sceneryobjects\\T\\peitsche.sco', (-3.5, 0, -0.15, 0.1, 9.7, 0.15))
    box_objekt(omsi, 'Sceneryobjects\\T\\mast.sco', (-0.03, 0, -0.03, 0.03, 3.0, 0.03), anhaenge=(2.55, 1.92))
    for n in ('vz205', 'vz206', 'vz274'):
        box_objekt(omsi, f'Sceneryobjects\\T\\{n}.sco', (-0.3, -0.3, 0.03, 0.3, 0.3, 0.04))
    box_objekt(omsi, 'Sceneryobjects\\T\\halle.sco', (-2.2, 0, -1.6, 2.1, 2.4, 0.2))
    box_objekt(omsi, 'Sceneryobjects\\T\\bank.sco', (-1.0, 0, -0.2, 1.0, 0.9, 0.6))
    box_objekt(omsi, 'Sceneryobjects\\T\\eimer.sco', (-0.25, 0, -0.1, 0.25, 0.8, 0.17))
    # ein Objekt mit [busstop] darf nie gewaehlt werden
    box_objekt(omsi, 'Sceneryobjects\\T\\stoerer.sco', (-0.5, 0, -0.5, 0.5, 1, 0.5))
    with open(os.path.join(omsi, 'Sceneryobjects', 'T', 'stoerer.sco'), 'a') as f:
        f.write('\r\n[busstop]\r\n')


def gebaeude(omsi, karte, k, name, x0, z0, breite):
    rel = BS.join(['Sceneryobjects', 'Aschaffenburg', karte, 'LoD2', f'{k[0]}_{k[1]}', name + '.sco'])
    ordner = os.path.join(omsi, *rel.split(BS)[:-1])
    os.makedirs(os.path.join(ordner, 'model'), exist_ok=True)
    m = Mesh()
    h = breite / 2
    m.quad('dach', [(-h, -h, 8), (h, -h, 8), (h, h, 8), (-h, h, 8)], [(0, 0), (1, 0), (1, 1), (0, 1)], (0, 0, 1))
    open(os.path.join(ordner, 'model', name + '.x'), 'w').write(kreuzung.x_file(m.V, m.F, [('dach', 'd.bmp')]))
    open(os.path.join(ordner, name + '.sco'), 'w').write('[mesh]\r\n' + name + '.x\r\n')
    return dict(name=name, rel=rel, x=x0, y=z0, z=0.0)


def strassenkarte(tmp_path, stops=(), gebaeude_an=()):
    """Strasse nach Osten (Welt z = 100, x 100..300), Karte K; gebaeude_an: [(dx, dz, breite)] relativ zur Strassenmitte"""
    omsi = str(tmp_path)
    installation(omsi)
    els = [(100.0, 100.0, 90.0, 200.0, 0.0, SLI)]
    info = writer.write_map(os.path.join(omsi, 'maps'), 'K', [dict(els=els)], list(stops))
    d = info['dir']
    ox, oz = info['offset']
    N.attribute_schreiben(d, {1: dict(art='strasse', name='Teststrasse', highway='residential',
                                      tags=dict(highway='residential', maxspeed='30'))})
    N.schreiben(d, omsi)
    sx, sz = 200.0 - ox, 100.0 - oz
    k = (int(sx // 300), int(sz // 300))
    if gebaeude_an:
        objekte = [gebaeude(omsi, 'K', k, f'G{i}', sx + dx, sz + dz, b) for i, (dx, dz, b) in enumerate(gebaeude_an)]
        p = S.paket('K', 'gebaeude', k, objekte, S.manifest(d, 'gebaeude', k, 'test'))
        pf = os.path.join(omsi, 'geb.json')
        json.dump(p, open(pf, 'w', encoding='utf-8'))
        S.einsetzen(d, pf)
    B.berechnen(d, omsi)
    return omsi, d, (sx, sz), (ox, oz)


def platz_fuer(omsi, d):
    netz = N.Netz.laden(d, pruefen=False)
    bel = B.Belegung.laden(d, pruefen=False)
    return A.Platz(netz, bel, A.Gelaende(d), KATALOG['regeln']), A.Auswahl(KATALOG, omsi)


# ---------------------------------------------------------------- Tests

def test_auswahl_und_pfad_mit_kennung(tmp_path):
    installation(str(tmp_path))
    aw = A.Auswahl(KATALOG, str(tmp_path))
    assert 'stoerer' not in aw                       # [busstop]: wuerde eine zusaetzliche Haltestelle machen
    assert aw.anhaenge('schild_mast') == [2.55, 1.92]
    rel = A.rel_fuer('K', (3, 7), 'Sceneryobjects\\Streetobjects_MC\\strlt.sco')
    assert rel == 'Sceneryobjects\\Aschaffenburg\\K\\Objekte\\3_7\\..\\..\\..\\..\\Streetobjects_MC\\strlt.sco'
    assert A.ziel_von(rel) == 'Sceneryobjects\\Streetobjects_MC\\strlt.sco'
    assert S.schicht_von(rel) == ('objekte', (3, 7))
    eigen = 'Sceneryobjects\\Aschaffenburg\\K\\Objekte\\3_7\\H_x.sco'
    assert A.rel_fuer('K', (3, 7), eigen) == eigen


def test_platz_am_rand_auf_gehweg(tmp_path):
    omsi, d, (sx, sz), _ = strassenkarte(tmp_path)
    platz, aw = platz_fuer(omsi, d)
    e = next(iter(platz.netz.kanten))
    x, z, y, h, q = platz.am_rand(e, 100.0, 1, 0.5, 0.15)
    assert abs(q - 4.65) < 1e-6                      # Bord 4 m + 0,5 m + Radius
    assert platz.klasse(x, z) == 'gehweg'
    assert abs(z - (sz - 4.65)) < 0.01 and abs(x - sx) < 0.01   # rechts einer Strasse nach Osten = Sueden
    x2, z2, *_ = platz.am_rand(e, 100.0, -1, 0.5, 0.15)
    assert abs(z2 - (sz + 4.65)) < 0.01


def test_laternen_regel_abstand_und_ausleger(tmp_path):
    omsi, d, (sx, sz), _ = strassenkarte(tmp_path)
    platz, aw = platz_fuer(omsi, d)
    n = A.laternen(platz, aw, KATALOG['regeln'], None)
    lat = [o for o in platz.objekte if o['typ'].startswith('laterne')]
    assert n['regel'] == len(lat) >= 5               # 200 m Wohnstrasse, alle 30 m
    for o in lat:
        assert platz.klasse(o['x'], o['z']) in A.SEITENRAUM
        assert abs(abs(o['z'] - sz) - 4.65) < 0.05    # 0,5 m vom Bord
        assert 6.0 - 0.01 <= o['station'] <= 194.0 + 0.01      # nicht im Kreuzungsbereich der Enden
    xs = sorted(o['x'] for o in lat)
    assert min(b - a for a, b in zip(xs, xs[1:])) >= 8.0


def test_laterne_nicht_vor_gebaeude_und_zufahrt(tmp_path):
    # Gebaeude steht bis an den Bord (Gehweg verbaut) zwischen x = sx-10 .. sx+10
    omsi, d, (sx, sz), _ = strassenkarte(tmp_path, gebaeude_an=[(0.0, -14.0, 20.0)])
    platz, aw = platz_fuer(omsi, d)
    platz.sperren[(int((sx + 40) // 5), int((sz - 5) // 5))].append((sx + 40, sz - 5.5, 2.5))   # Zufahrt
    A.laternen(platz, aw, KATALOG['regeln'], None)
    for o in platz.objekte:
        assert not (abs(o['x'] - sx) < 10.5 and o['z'] < sz), 'Laterne vor dem Gebaeude'
        assert math.hypot(o['x'] - (sx + 40), o['z'] - (sz - 5.5)) >= 2.5, 'Laterne in der Zufahrt'


def kreuzungskarte(tmp_path):
    """T-Kreuzung bei Welt (150, 150): Arme von Westen, Osten, Sueden; Kreuzungsobjekt mit Pfaden und Vorfahrt:
    Westen wartet (64), Osten und Sueden Hauptstrasse (192)"""
    omsi = str(tmp_path)
    installation(omsi)
    rel = 'Sceneryobjects\\Aschaffenburg\\K\\K_001.sco'
    pfad = os.path.join(omsi, *rel.split(BS))
    os.makedirs(os.path.dirname(pfad), exist_ok=True)
    pfade = [(-10.0, -2.0, 90.0), (10.0, 2.0, 270.0), (2.0, -10.0, 0.0)]
    t = '[mesh]\r\nK_001.x\r\n'
    for x, y, h in pfade:
        t += '\r\n'.join(['', '[path]', str(x), str(y), '0', str(h), '0', '5', '0', '0', '0', '3', '0', '0']) + '\r\n'
    open(pfad, 'w', newline='').write(t)
    ketten = [dict(els=[(60.0, 150.0, 90.0, 80.0, 0.0, SLI)]), dict(els=[(160.0, 150.0, 90.0, 80.0, 0.0, SLI)]),
              dict(els=[(150.0, 60.0, 0.0, 80.0, 0.0, SLI)])]
    junc = [dict(origin=(150.0, 150.0), rel=rel, rules=[(0, 64), (1, 192), (2, 192)])]
    info = writer.write_map(os.path.join(omsi, 'maps'), 'K', ketten, [], junctions=junc)
    d = info['dir']
    N.attribute_schreiben(d, {i: dict(art='strasse', name=f'S{i}', highway='residential', tags={'highway': 'residential'})
                              for i in (1, 2, 3)})
    N.schreiben(d, omsi)
    B.berechnen(d, omsi)
    return omsi, d, info['offset']


def test_vorfahrt_aus_der_karte_und_schilder(tmp_path):
    omsi, d, (ox, oz) = kreuzungskarte(tmp_path)
    platz, aw = platz_fuer(omsi, d)
    rollen = A.kreuzungsrollen(d, omsi, platz.netz)
    assert len(rollen) == 1
    (kn, info), = rollen.items()
    namen = {platz.netz.kanten[e]['attribute']['name']: r for e, r in info['arme'].items()}
    assert namen == {'S1': 'neben', 'S2': 'haupt', 'S3': 'haupt'}
    west = next(e for e in info['arme'] if platz.netz.kanten[e]['attribute']['name'] == 'S1')
    ost = next(e for e in info['arme'] if platz.netz.kanten[e]['attribute']['name'] == 'S2')
    # OSM: Vorfahrt gewaehren an der Hauptstrasse (Osten) widerspricht, Stopp am Westarm passt
    osm = dict(knoten=[dict(id=1, x=172.0 - ox, z=152.0 - oz, tags={'highway': 'give_way', 'direction': 'backward'}),
                       dict(id=2, x=135.0 - ox, z=144.0 - oz, tags={'traffic_sign': 'DE:206'})], wege=[])
    n, konflikte = A.verkehrszeichen(platz, aw, KATALOG['regeln'], osm, rollen, d)
    schilder = [o for o in platz.objekte if o['typ'].startswith('vz_')]
    assert [o['typ'] for o in schilder] == ['vz_206']          # statt Regel-205 das OSM-Stoppschild, nichts am Hauptarm
    s = schilder[0]
    assert s['kante'] == west and s['rolle'] == 'neben' and s['quelle'] == 'osm'
    assert s['z'] < 150.0 - oz - 4.0                              # rechts der ankommenden Fahrt (Sueden), am Gehweg
    assert abs((s['rot'] - 270.0 + 180) % 360 - 180) < 1        # Schild schaut dem Verkehr (nach Osten) entgegen
    assert abs(s['hoehe'] - next(o for o in platz.objekte if o['typ'] == 'schild_mast')['hoehe'] - 2.55) < 1e-6
    assert len(konflikte) == 1 and konflikte[0]['kante'] == ost and 'haupt' in konflikte[0]['text']


def test_haltestelle_zusammensetzung(tmp_path):
    # Steig rechts der Strasse (Sueden) mit Halle, Bank, Eimer, erhoehtem Bord; Wuerfel gleichen Namens 3 m daneben
    omsi, d, (sx, sz), (ox, oz) = strassenkarte(tmp_path, stops=[dict(name='Großmannstraße', x=213.0, z=94.5, rot=90.0)])
    platz, aw = platz_fuer(omsi, d)
    osm = dict(knoten=[dict(id=7, x=sx + 10.0, z=sz - 6.0, tags={
        'highway': 'bus_stop', 'public_transport': 'platform', 'name': 'Großmannstraße', 'shelter': 'yes',
        'bench': 'yes', 'bin': 'yes', 'kerb:height': '0.20', 'tactile_paving': 'yes'})], wege=[])
    liste, n = H.planen(platz, aw, KATALOG['regeln'], osm, d)
    assert len(liste) == 1
    h = liste[0]
    assert h['mast'] and h['wartehalle'] == 'wartehalle' and h['muelleimer'] and h['hochbord']
    assert not h['bank']                                         # die Halle hat Sitze
    typen = {o['typ']: o for o in platz.objekte}
    m = typen['haltestelle_mast']
    assert abs(m['x'] - (213.0 - ox)) < 0.6                      # Haltepunkt = Wuerfel
    assert abs((sz - m['z']) - 4.55) < 0.05                      # 0,5 m vom Bord
    w = typen['wartehalle']
    assert w['x'] < m['x'] - 2.0                                 # hinter dem Mast (Bus faehrt nach Osten)
    assert sz - w['z'] > 4.0 + 1.2                               # Abstand zum Bord
    assert abs((w['rot'] - 180.0 + 180) % 360 - 180) < 1         # Vorderseite (-z) zur Fahrbahn (Norden)
    for o in platz.objekte:
        assert platz.klasse(o['x'], o['z']) in A.SEITENRAUM


def test_haltestelle_ohne_platz_fuer_halle_wird_gemeldet(tmp_path):
    omsi, d, (sx, sz), (ox, oz) = strassenkarte(tmp_path, gebaeude_an=[(10.0, -21.0, 30.0)])         # Hauswand 6 m vor der Achse
    platz, aw = platz_fuer(omsi, d)
    osm = dict(knoten=[dict(id=7, x=sx + 10.0, z=sz - 5.0, tags={
        'highway': 'bus_stop', 'name': 'Rathaus', 'shelter': 'yes'})], wege=[])
    liste, n = H.planen(platz, aw, KATALOG['regeln'], osm, d)
    assert liste[0]['osm_wartehalle'] and not liste[0]['wartehalle']
    assert n['wartehalle_kein_platz'] == 1


def test_schildtext_exakt():
    for name in ('Großmannstraße', 'Dörrmorsbach', 'City Galerie (Fernbushaltestelle)'):
        img, breite, px = E.atlas(name)
        g, w, b2 = E.namensschild_breite(name)
        assert b2 == breite and px == w + 40 and px <= 768
        # das Schild ist genau der mit der Schriftart gesetzte Text
        from PIL import Image, ImageDraw
        ref = Image.new('RGB', E.ATLAS, E.GRAU)
        dr = ImageDraw.Draw(ref)
        f = E.schrift(g)
        bb = dr.textbbox((0, 0), name, font=f)
        dr.rectangle((256, 0, 256 + px - 1, 127), fill=E.WEISS)
        dr.rectangle((256, 0, 256 + px - 1, 127), outline=E.GRUEN, width=5)
        dr.text((256 + 20 - bb[0], 64 - (bb[1] + bb[3]) / 2), name, font=f, fill=E.GRUEN)
        a = img.crop((256, 0, 256 + px, 128)).tobytes()
        assert a == ref.crop((256, 0, 256 + px, 128)).tobytes()
    assert E.atlas('Großmannstraße')[0].tobytes() != E.atlas('Grossmannstrasse')[0].tobytes()
    assert E.kurzname('Großmannstraße') == 'Grossmannstrasse'


def test_mast_objekt_dateien(tmp_path):
    datei, breite = E.mast_schreiben(str(tmp_path), 'Am Wingert')
    assert datei == 'H_Am_Wingert.sco'
    meta = json.load(open(tmp_path / 'H_Am_Wingert.json', encoding='utf-8'))
    assert meta['name'] == 'Am Wingert'
    m = M.masse(str(tmp_path), 'H_Am_Wingert.sco')
    assert abs(m['hoehe'] - 2.95) < 0.01 and abs(m['breite'] - max(0.42, breite)) < 0.01
    assert open(tmp_path / 'texture' / 'H_Am_Wingert.dds', 'rb').read(4) == b'DDS '


def test_x_rahmen_und_masse():
    t = ('xof 0303txt 0032\ntemplate Mesh { <3d82ab44-62da-11cf-ab39-0020af71e433> DWORD nVertices; }\n'
         'Frame A { FrameTransformMatrix { 1,0,0,0, 0,1,0,0, 0,0,1,0, 0,2,0,1;; }\n'
         ' Mesh { 3; 0;0;0;, 1;0;0;, 0;1;0;; 1; 3;0,1,2;; // Ende des Mesh A\n'
         ' MeshMaterialList { 1; 1; 0;; Material { 1;1;1;1;; 0; 0;0;0;; 0;0;0;; TextureFilename { "a.bmp"; } } } } }\n')
    P, n, tex = M.x_lesen(t)
    assert n == 1 and tex == ['a.bmp']
    assert sorted(p[1] for p in P) == [2.0, 2.0, 3.0]          # um 2 m angehoben (Zeilenvektor-Matrix)


def test_schicht_objekte_veraltet_nach_netzaenderung(tmp_path):
    omsi, d, (sx, sz), _ = strassenkarte(tmp_path)
    # Geobezug fuer die Karte (OSM wird hier nicht geladen)
    erg = A.bauen(d, omsi, einsetzen=True, osm_daten=False, katalog=KATALOG, log=lambda *a: None)
    assert erg['objekte'] > 0
    st = S.status(d)
    kx, ky = int(sx // 300), int(sz // 300)
    k = f'{kx}_{ky}'
    assert st[k]['objekte']['zustand'] == 'aktuell'
    # Objektpfade mit Kennung, die Ziele gibt es
    _, bl = S.bloecke(S.lesen(d, (kx, ky)))
    rels = [S.objekt_pfad(b) for b in bl if S.marker('objekte', (kx, ky)) in S.objekt_pfad(b)]
    assert rels and all(os.path.isfile(os.path.join(omsi, *A.ziel_von(r).split(BS))) for r in rels)
    assert A.funde(d, omsi, B.Belegung.laden(d, pruefen=False)) == []
    # Strasse geaendert -> Schicht objekte veraltet
    t = S.lesen(d, (kx, ky)).replace('altonaer1', 'staakener1')
    S.schreiben(d, (kx, ky), t)
    assert S.status(d)[k]['objekte']['zustand'] == 'veraltet'
    assert 'netz' in S.status(d)[k]['objekte']['grund']


def test_gelaende_wie_omsi_dreiecke(tmp_path):
    import struct
    d = tmp_path / 'K'
    d.mkdir()
    h = [0.0] * (61 * 61)
    h[0 * 61 + 1] = 1.0                      # Ecke (1, 0) = 5 m Ost
    h[1 * 61 + 1] = 2.0                      # Ecke (1, 1)
    (d / 'tile_0_0.map.terrain').write_bytes(struct.pack('<i', 60) + struct.pack(f'<{61 * 61}f', *h))
    g = A.Gelaende(str(d))
    assert abs(g.hoehe(4.0, 1.0) - (0.8 + (2.0 - 1.0) * 0.2)) < 1e-6     # unteres Dreieck (u >= v)
    assert abs(g.hoehe(1.0, 4.0) - (2.0 * 0.2)) < 1e-6                   # oberes Dreieck
