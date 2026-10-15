"""F2 Schichten: Pakete je Kachel, Einsetzen ersetzt nur die eigene Schicht, Verzeichnis erkennt veraltete Schichten."""
import json
import os

from omsigen import schichten as S
from omsigen import writer

BS = chr(92)


def karte(tmp_path):
    kette = dict(els=[(10.0, 10.0, 90.0, 50.0, 0.0, 'Splines' + BS + 'x.sli')])
    hand = [dict(origin=(120.0, 40.0), rel='Sceneryobjects' + BS + 'Hand' + BS + 'bank.sco')]
    writer.write_map(str(tmp_path), 'K', [kette], [], junctions=hand)
    return str(tmp_path / 'K')


def strassenkachel(d):
    return next(k for k in S.kacheln(d) if 'x.sli' in S.lesen(d, k))


def paket_schreiben(d, kachel, objekte, ordner):
    p = S.paket('K', 'gebaeude', kachel, objekte, S.manifest(d, 'gebaeude', kachel, 'test'))
    pfad = os.path.join(ordner, f'paket_{kachel[0]}_{kachel[1]}.json')
    json.dump(p, open(pfad, 'w', encoding='utf-8'))
    return pfad, p


def objekte_in(d, k):
    _, bl = S.bloecke(S.lesen(d, k))
    return [S.objekt_pfad(b) for b in bl if b[0].strip() == '[object]']


def test_marker_und_schicht():
    assert S.marker('gebaeude', (3, -7)) == BS + 'LoD2' + BS + '3_-7' + BS
    rel = BS.join(['Sceneryobjects', 'Aschaffenburg', 'X', 'LoD2', '3_-7', 'B1.sco'])
    assert S.schicht_von(rel) == ('gebaeude', (3, -7))
    assert S.schicht_von(BS.join(['Sceneryobjects', 'Aschaffenburg', 'X', 'K_001.sco'])) is None
    assert S.vorher('gruen') == ['netz', 'boden', 'gebaeude', 'objekte']


def test_einsetzen_ersetzt_nur_eigene_schicht(tmp_path):
    d = karte(tmp_path)
    k = strassenkachel(d)
    rel = lambda n: BS.join(['Sceneryobjects', 'Aschaffenburg', 'K', 'LoD2', f'{k[0]}_{k[1]}', n])
    x0, y0 = k[0] * 300.0, k[1] * 300.0
    pfad, p = paket_schreiben(d, k, [dict(name='B1', rel=rel('B1.sco'), x=x0 + 30, y=y0 + 30, z=5.0),
                                     dict(name='B2', rel=rel('B2.sco'), x=x0 + 60, y=y0 + 60, z=6.0, rot=45.0,
                                          texte=['Baum.tga', '12.5', '0.9', ''])], str(tmp_path))
    assert S.status(d) == {}                                 # noch nichts gebaut
    S.vermerken(d, pfad, p)
    assert S.status(d)[f'{k[0]}_{k[1]}']['gebaeude']['zustand'] == 'gebaut'
    id0 = S._next_id(d)[0]
    assert S.einsetzen(d, pfad) == 2
    pfade = objekte_in(d, k)
    assert rel('B1.sco') in pfade and rel('B2.sco') in pfade and any('bank.sco' in q for q in pfade)
    assert S._next_id(d)[0] >= id0 + 2
    # Drehung und Beschriftungen stehen im Block
    _, bl = S.bloecke(S.lesen(d, k))
    b2 = next(b for b in bl if S.objekt_pfad(b) == rel('B2.sco'))
    assert b2[7] == '45' and b2[10] == '4' and b2[11] == 'Baum.tga'
    assert S.status(d)[f'{k[0]}_{k[1]}']['gebaeude']['zustand'] == 'aktuell'
    # neu einsetzen: B1 faellt weg, B3 kommt dazu, die Bank bleibt
    pfad, p = paket_schreiben(d, k, [dict(name='B3', rel=rel('B3.sco'), x=x0 + 90, y=y0 + 90, z=5.0)], str(tmp_path))
    S.einsetzen(d, pfad)
    pfade = objekte_in(d, k)
    assert rel('B3.sco') in pfade and rel('B1.sco') not in pfade and any('bank.sco' in q for q in pfade)


def test_netz_aenderung_macht_schicht_veraltet(tmp_path):
    d = karte(tmp_path)
    k = strassenkachel(d)
    rel = BS.join(['Sceneryobjects', 'Aschaffenburg', 'K', 'LoD2', f'{k[0]}_{k[1]}', 'B1.sco'])
    pfad, p = paket_schreiben(d, k, [dict(name='B1', rel=rel, x=k[0] * 300.0 + 30, y=k[1] * 300.0 + 30, z=5.0)], str(tmp_path))
    S.vermerken(d, pfad, p)
    netz_vorher = S.fingerabdruck(d, k, 'netz')
    S.einsetzen(d, pfad)
    assert S.fingerabdruck(d, k, 'netz') == netz_vorher       # Einsetzen aendert das Netz nicht
    assert S.status(d)[f'{k[0]}_{k[1]}']['gebaeude']['zustand'] == 'aktuell'
    # eine Strasse der Kachel aendert sich (Editor speichert) -> Gebaeude veraltet
    t = S.lesen(d, k).replace('[spline]', '[spline]', 1)
    i = t.index('Splines' + BS + 'x.sli')
    S.schreiben(d, k, t[:i] + 'Splines' + BS + 'y.sli' + t[i + len('Splines' + BS + 'x.sli'):])
    st = S.status(d)[f'{k[0]}_{k[1]}']['gebaeude']
    assert st['zustand'] == 'veraltet' and 'netz' in st['grund']


def test_leeres_paket_ist_aktuell(tmp_path):
    d = karte(tmp_path)
    k = strassenkachel(d)
    pfad, p = paket_schreiben(d, k, [], str(tmp_path))
    S.vermerken(d, pfad, p)
    assert S.status(d)[f'{k[0]}_{k[1]}']['gebaeude']['zustand'] == 'aktuell'
