"""G1 Platzierung: Lage (Grundrissmitte, Hauptachse), Gruendung auf dem Kartengelaende, Fahrbahn an Gebaeuden einpassen,
Konflikte einordnen, Hoehenpruefung."""
import json
import math
import os
import struct

import numpy as np
import pytest

from omsigen import belegung as B, config, kreuzung, lod2 as L, netz_datei as N, platzierung as P, schichten as S, writer
from omsigen import querschnitte as Q

BS = chr(92)
SLI = config.MARCEL + 'str_2spur_8m_altonaer1.sli'   # Fahrbahn +-4 m, aussen +-7 m


def kiste(x0, y0, laenge, breite, winkel, boden, hoehe):
    """LoD2-Gebaeude als Quader: Grundriss laenge x breite um (x0, y0), um `winkel` Grad (gegen den Uhrzeigersinn ab
    Ost) gedreht; Boden-, Wand- und Dachflaechen wie omsigen.lod2.gebaeude"""
    a = math.radians(winkel)
    u, v = np.array([math.cos(a), math.sin(a)]), np.array([-math.sin(a), math.cos(a)])
    ecken = [np.array([x0, y0]) + u * sx * laenge / 2 + v * sy * breite / 2 for sx, sy in ((-1, -1), (1, -1), (1, 1), (-1, 1))]
    unten = [np.r_[e, boden] for e in ecken]
    oben = [np.r_[e, boden + hoehe] for e in ecken]
    flaechen = [('GroundSurface', [np.array(unten[::-1])]), ('RoofSurface', [np.array(oben)])]
    for i in range(4):
        j = (i + 1) % 4
        flaechen.append(('WallSurface', [np.array([unten[i], unten[j], oben[j], oben[i]])]))
    Pk = np.vstack([r for _, rs in flaechen for r in rs])
    return dict(id='DEBY_LOD2_T', funktion='31001_1000', dach='1000', hoehe=hoehe, flaechen=flaechen,
                mitte=Pk[:, :2].mean(0), boden=float(Pk[:, 2].min()))


# ---------------------------------------------------------------- Lage

def test_lage_grundrissmitte_und_hauptachse():
    g = kiste(100.0, 50.0, 20.0, 8.0, 30.0, 12.0, 9.0)
    lg = P.lage(g)
    assert abs(lg['x'] - 100.0) < 1e-3 and abs(lg['y'] - 50.0) < 1e-3 and lg['z'] == 12.0
    # lokale x-Achse laengs der langen Seite: 30 Grad gegen den Uhrzeigersinn ab Ost = Drehung -30 Grad (Uhrzeigersinn)
    assert abs(lg['rot'] + 30.0) < 1e-6
    # L-Form: Ursprung im Flaechenschwerpunkt, nicht im Mittel der Punkte
    from shapely.geometry import Polygon
    poly = Polygon([(0, 0), (20, 0), (20, 4), (4, 4), (4, 20), (0, 20)])
    g2 = dict(flaechen=[('GroundSurface', [np.c_[np.asarray(poly.exterior.coords)[:-1], np.zeros(6)]])], boden=0.0,
              mitte=np.array([8.0, 8.0]))
    lg2 = P.lage(g2)
    assert abs(lg2['x'] - poly.centroid.x) < 1e-3 and abs(lg2['y'] - poly.centroid.y) < 1e-3
    # Hauptachse liegt immer in (-90, 90]
    for w in (0.0, 89.0, 91.0, 179.0, -45.0):
        r = P.hauptachse(Polygon(np.asarray(kiste(0, 0, 10, 4, w, 0, 3)['flaechen'][0][1][0])[:, :2]))
        assert -90.0 < r <= 90.0
        assert abs(((-r - w) + 90.0) % 180.0 - 90.0) < 1e-6


def test_lage_aendert_die_geometrie_nicht():
    """mit Grundrissmitte, Boden und Drehung liegt jeder Punkt in Kartenkoordinaten, wo er vorher lag"""
    g = kiste(1234.5678, 987.6543, 17.0, 9.0, 23.0, 41.2, 11.0)
    m_alt, lg_alt = L.bauen_gebaeude(g)                       # vorher: Mittel aller Punkte, Drehung 0, absolute Hoehe
    lg = P.lage(g)
    m_neu, lg_neu = L.bauen_gebaeude(g, lg)
    assert lg_neu == lg and lg_alt['rot'] == 0.0 and lg_alt['z'] == 0.0
    welt = lambda m, lg: P.welt([(v[0], v[2], v[1]) for v in m.V], lg)    # Mesh.V = (x, hoch, vor)
    A, Bw = welt(m_alt, lg_alt), welt(m_neu, lg_neu)
    assert A.shape == Bw.shape and np.abs(A - Bw).max() < 1e-6
    # auch nach dem Runden in der .x-Datei (4 Nachkommastellen): unter 0,1 mm
    rund = [(round(v[0], 4), round(v[2], 4), round(v[1], 4)) for v in m_neu.V]
    assert np.abs(P.welt(rund, lg) - A).max() < 1e-4
    # Normalen mitgedreht: in der Welt gleich
    na = np.array([(v[3], v[5], v[4]) for v in m_alt.V])
    nb = np.array([(v[3], v[5], v[4]) for v in m_neu.V])
    nb_welt = P.welt(nb, dict(x=0.0, y=0.0, z=0.0, rot=lg['rot']))
    assert np.abs(na - nb_welt).max() < 1e-9


def test_lage_fassade_weltgleich():
    g = kiste(5.0, 2.5, 10.0, 5.0, 10.0, 6.0, 6.0)
    f = dict(ursprung=[1005.0, 2000.0], punkte=[[-5, 0, 136], [5, 0, 136], [5, 0, 142], [-5, 0, 135]],
             uv=[[0, 1], [1, 1], [1, 0], [0, 1]], dreiecke=[[0, 1, 2], [3, 1, 0]], textur='fassade.jpg')
    bez = dict(ost0=1000, nord0=2000, nn0=130)
    m_alt, lg_alt = L.bauen_fassade(g, f, bez)
    lg = P.lage(g)
    m_neu, _ = L.bauen_fassade(g, f, bez, lage=lg)
    welt = lambda m, l: np.sort(P.welt([(v[0], v[2], v[1]) for v in m.V], l), axis=0)
    assert np.abs(welt(m_alt, lg_alt) - welt(m_neu, lg)).max() < 1e-6
    # Schuerze (tiefster Punkt der Fassade, 5 m ueber nn0) bis unten = 2 m
    m_tief, _ = L.bauen_fassade(g, f, bez, lage=lg, unten=2.0)
    z = sorted(round(v[1] + lg['z'], 3) for v in m_tief.V)
    assert z[0] == 2.0 and 6.0 in z and 12.0 in z


# ---------------------------------------------------------------- Gruendung

def test_sockel_bis_unter_das_gelaende():
    assert P.unten(10.0, None) == 10.0 - P.UNTER_BODEN                  # kein Gelaende: 1 m wie bisher
    assert P.unten(10.0, 10.2) == 10.0 - P.UNTER_BODEN                  # Gelaende hoeher: 1 m reicht
    assert abs(P.unten(10.0, 8.0) - (8.0 - P.EINBINDEN)) < 1e-9         # Hang: bis unter den tiefsten Punkt
    assert P.unten(10.0, -20.0) == 10.0 - P.SOCKEL_MAX                  # Gelaende viel zu tief: Grenze
    g = kiste(50.0, 50.0, 20.0, 10.0, 0.0, 10.0, 6.0)
    poly = P.grundriss(g)
    hang = lambda x, y: 10.0 - (x - 40.0) * 0.2                         # faellt nach Osten: bei x = 60 auf 6 m
    t = P.gelaende_min(hang, poly)
    assert abs(t - 6.0) < 1e-6
    lg = P.lage(g, poly)
    m, _ = L.bauen_gebaeude(g, lg, P.unten(g['boden'], t))
    assert abs(min(v[1] for v in m.V) + lg['z'] - (6.0 - P.EINBINDEN)) < 1e-6
    # Dach unveraendert
    assert abs(max(v[1] for v in m.V) + lg['z'] - 16.0) < 1e-9


def test_gelaende_nicht_ueber_gebaeude():
    from shapely.geometry import box
    gr = P.Grundrisse([box(10, 10, 20, 20)], [5.0], puffer=2.0)
    xs = np.arange(0, 31, 5.0)
    X, Z = np.meshgrid(xs, xs)
    H0 = np.full(X.shape, 4.0)                    # DGM 4 m, Boden 5 m
    H = np.full(X.shape, 8.0)                     # Strassenboeschung hat ueberall auf 8 m angehoben
    frei = np.ones(X.shape, bool)
    frei[0, 0] = False
    Hk = gr.klemmen(X, Z, H, H0, frei)
    innen = (X >= 10) & (X <= 20) & (Z >= 10) & (Z <= 20)
    assert (Hk[innen] == 5.0).all()                                  # im Grundriss auf den Boden
    assert Hk[0, 0] == 8.0 and Hk[6, 6] == 8.0                       # unter der Strasse / weit weg: unveraendert
    frei[3, 3] = False                                               # Strasse ragt ins Haus: dort trotzdem der Boden
    assert gr.klemmen(X, Z, H, H0, frei)[3, 3] == 5.0
    # Puffer: hoechstens max(DGM, Boden); tieferes Gelaende bleibt
    H2 = np.full(X.shape, 3.0)
    assert (gr.klemmen(X, Z, H2, H0, frei) == 3.0).all()


# ---------------------------------------------------------------- Fahrbahn an Gebaeuden einpassen

def _q(spur=3.0, gehweg=2.0):
    return Q.querschnitt([Q.teil('gehweg', gehweg), Q.teil('fahrspur', spur, 'zurueck'), Q.teil('fahrspur', spur, 'vor'),
                          Q.teil('gehweg', gehweg)])


def test_fahrbahn_einpassen_versatz():
    q = _q()                                       # Fahrbahn -3 .. 3
    li = [6.0, 6.0, 2.0, 2.0, 6.0]                 # links ragt ein Haus bis 2 m an die Achse
    re = [8.0] * 5                                  # rechts viel Platz
    q2, grund = P.fahrbahn_einpassen(q, li, re, Q.SPUR_MIN)
    assert grund == 'versatz' and q2 is not None
    i = Q.info(q2)
    assert -i['cl'] <= 2.0 - P.RAND + 1e-9 and i['cr'] <= 8.0 - P.RAND
    assert [t['breite'] for t in q2['teile']] == [t['breite'] for t in q['teile']]     # Breiten bleiben
    # eine Station mit der Achse im Haus (Ueberbau, OSM-Linie im Haus) verhindert das Einpassen nicht
    q4, grund = P.fahrbahn_einpassen(q, li + [0.0, 0.0], re + [0.0, 0.0], Q.SPUR_MIN)
    assert grund == 'versatz' and q4['versatz'] == q2['versatz']
    # passt schon: nichts aendern
    assert P.fahrbahn_einpassen(q, [6.0] * 3, [6.0] * 3, Q.SPUR_MIN) == (None, '')


def test_fahrbahn_einpassen_wellig_und_engstelle():
    q = _q()
    # mal links, mal rechts im Haus, an keiner Stelle zu eng: OSM-Linie liegt daneben -> nichts aendern, melden
    q2, grund = P.fahrbahn_einpassen(q, [2.0, 6.0, 6.0], [6.0, 6.0, 2.0], Q.SPUR_MIN)
    assert q2 is None and grund == 'wellig'
    # beide Seiten an derselben Stelle zu nah: Engstelle -> Spuren schmaler
    q3, grund = P.fahrbahn_einpassen(q, [6.0, 2.9, 2.9, 6.0], [6.0, 2.9, 2.9, 6.0], Q.SPUR_MIN)
    assert grund == 'engstelle'
    i = Q.info(q3)
    assert i['cr'] - i['cl'] <= 2 * 2.9 - 2 * P.RAND + 1e-9 and min(t['breite'] for t in q3['teile'] if t['art'] in Q.SPUR) >= Q.SPUR_MIN


def test_gehwege_einpassen():
    q = _q(gehweg=2.5)                              # Fahrbahn -3 .. 3, aussen -5,5 .. 5,5
    q2, seiten = P.gehwege_einpassen(q, [5.0, 6.0, 9.0], [None, 9.0, 9.0])
    assert seiten == ['links'] and q2['teile'][0]['breite'] == 1.7 and q2['teile'][-1]['breite'] == 2.5
    assert -Q.info(q2)['ol'] <= 5.0 - P.RAND + 1e-9
    # nie schmaler als GEHWEG_MIN: der Rest bleibt ein Fund
    q3, _ = P.gehwege_einpassen(q, [3.5] * 3, [9.0] * 3)
    assert q3['teile'][0]['breite'] == Q.GEHWEG_MIN
    assert P.gehwege_einpassen(q, [9.0] * 3, [9.0] * 3) == (None, [])


# ---------------------------------------------------------------- Konflikte einordnen

def _gebaeude(omsi, karte, k, name, x0, z0, breite, tiefe=None, funktion='31001_1000'):
    """quaderfoermiges Gebaeude (Waende + Dach) als Objekt der Schicht gebaeude"""
    from omsigen.sonderbau_ring import Mesh
    rel = BS.join(['Sceneryobjects', 'Aschaffenburg', karte, 'LoD2', f'{k[0]}_{k[1]}', name + '.sco'])
    ordner = os.path.join(omsi, *rel.split(BS)[:-1])
    os.makedirs(os.path.join(ordner, 'model'), exist_ok=True)
    m = Mesh()
    hx, hz = breite / 2, (tiefe or breite) / 2
    m.quad('dach', [(-hx, -hz, 8), (hx, -hz, 8), (hx, hz, 8), (-hx, hz, 8)], [(0, 0), (1, 0), (1, 1), (0, 1)], (0, 0, 1))
    for (ax, az), (bx, bz) in (((-hx, -hz), (hx, -hz)), ((hx, -hz), (hx, hz)), ((hx, hz), (-hx, hz)), ((-hx, hz), (-hx, -hz))):
        m.quad('wand', [(ax, az, -1), (bx, bz, -1), (bx, bz, 8), (ax, az, 8)], [(0, 0), (1, 0), (1, 1), (0, 1)])
    open(os.path.join(ordner, 'model', name + '.x'), 'w').write(kreuzung.x_file(m.V, m.F, [('dach', 'd.bmp'), ('wand', 'w.bmp')]))
    open(os.path.join(ordner, name + '.sco'), 'w').write('[absheight]\r\n[mesh]\r\n' + name + '.x\r\n')
    return dict(name=name, rel=rel, x=x0, y=z0, z=0.0, funktion=funktion)


def test_konflikte_einordnen(tmp_path):
    omsi = str(tmp_path)
    els = [(100.0, 100.0, 90.0, 200.0, 0.0, SLI)]               # Strasse nach Osten bei z = 100, Fahrbahn +-4 m
    info = writer.write_map(os.path.join(omsi, 'maps'), 'K', [dict(els=els)], [])
    d = info['dir']
    ox, oz = info['offset']
    N.schreiben(d, omsi)
    sx, sz = 200.0 - ox, 100.0 - oz
    k = (int(sx // 300), int(sz // 300))
    objekte = [_gebaeude(omsi, 'K', k, 'UEBER', sx - 60.0, sz, 6.0),            # ueberspannt die Strasse
               _gebaeude(omsi, 'K', k, 'VERSETZT', sx - 20.0, sz + 6.0, 6.0),   # 1 m in der Fahrbahn, gegenueber frei
               _gebaeude(omsi, 'K', k, 'ENG', sx + 20.0, sz + 6.0, 6.0),        # 1 m in der Fahrbahn ...
               _gebaeude(omsi, 'K', k, 'GEGENUEBER', sx + 20.0, sz - 6.5, 6.0),  # ... und gegenueber auch zu nah
               _gebaeude(omsi, 'K', k, 'DACH', sx + 60.0, sz + 6.5, 6.0, funktion='51009_1610'),
               _gebaeude(omsi, 'K', k, 'GEHWEG', sx + 5.0, sz - 9.5, 6.0)]      # 1 m ueber dem Gehweg (aussen 7 m)
    p = S.paket('K', 'gebaeude', k, objekte, S.manifest(d, 'gebaeude', k, 'test'))
    pf = os.path.join(omsi, 'Sceneryobjects', 'Aschaffenburg', 'K', 'LoD2', f'{k[0]}_{k[1]}', 'lod2.json')
    json.dump(p, open(pf, 'w', encoding='utf-8'))
    S.einsetzen(d, pf)
    erg = B.berechnen(d, omsi)
    je = {os.path.basename(c['objekt']).split('.')[0]: c for c in erg['konflikte']}
    assert je['UEBER']['klasse'] == 'ueberbauung'
    assert je['VERSETZT']['klasse'] == 'achse_versetzt' and 0.5 < je['VERSETZT']['tiefe'] < 1.5
    assert je['ENG']['klasse'] == 'engstelle' and je['GEGENUEBER']['klasse'] == 'engstelle'
    assert je['DACH']['klasse'] == 'ueberbauung'
    assert je['GEHWEG']['art'] == 'gebaeude_auf_gehweg' and 'klasse' not in je['GEHWEG']
    assert all(c['art'] == 'gebaeude_auf_fahrbahn' and c['hinweis'] for n, c in je.items() if n != 'GEHWEG')


# ---------------------------------------------------------------- Hoehen pruefen

def test_hoehen_funde(tmp_path):
    omsi = str(tmp_path)
    els = [(100.0, 100.0, 90.0, 200.0, 0.0, SLI)]
    info = writer.write_map(os.path.join(omsi, 'maps'), 'K', [dict(els=els)], [])
    d = info['dir']
    ox, oz = info['offset']
    sx, sz = 200.0 - ox, 100.0 - oz
    k = (int(sx // 300), int(sz // 300))
    objekte = [_gebaeude(omsi, 'K', k, 'GUT', sx, sz + 30.0, 6.0),
               _gebaeude(omsi, 'K', k, 'HOCH', sx + 30.0, sz + 30.0, 6.0)]
    objekte[0]['z'] = 0.5                             # Wandfuss bei -0,5 m: steckt im flachen Gelaende (0 m)
    objekte[1]['z'] = 3.0                             # Wandfuss bei 2 m: 2 m Spalt
    p = S.paket('K', 'gebaeude', k, objekte, S.manifest(d, 'gebaeude', k, 'test'))
    pf = os.path.join(omsi, 'paket.json')
    json.dump(p, open(pf, 'w', encoding='utf-8'))
    S.einsetzen(d, pf)
    funde = P.hoehen_funde(d, omsi)
    assert [f['art'] for f in funde] == ['gebaeude_schwebt'] and 'HOCH' in funde[0]['text']
    h = {os.path.basename(g['rel']).split('.')[0]: g for g in P.gebaeude_hoehen(d, omsi)}
    assert abs(h['HOCH']['spalt'] - 2.0) < 1e-6 and abs(h['GUT']['spalt'] + 0.5) < 1e-6
    assert h['GUT']['boden'] == 0.5
