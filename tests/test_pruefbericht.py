"""F8 Pruefbericht: Funde je Kachel, Budgets, Prueforte."""
import json
import os

from omsigen import belegung as B, netz_datei as N, pruefbericht as P, schichten as S, writer

from .test_belegung import SLI, gebaeude

BS = chr(92)


def test_auswahl_gleichmaessig():
    orte = [dict(name=str(i)) for i in range(10)]
    assert [o['name'] for o in P.auswahl(orte, 3)] == ['0', '4', '9'] or [o['name'] for o in P.auswahl(orte, 3)] == ['0', '5', '9']
    assert P.auswahl(orte[:2], 5) == orte[:2]


def test_bericht(tmp_path, monkeypatch):
    omsi = str(tmp_path)
    halt = [dict(name='Am Wingert', x=150.0, z=110.0, rot=90.0)]
    info = writer.write_map(os.path.join(omsi, 'maps'), 'K', [dict(els=[(100.0, 100.0, 90.0, 200.0, 0.0, SLI)])], halt)
    d = info['dir']
    ox, oz = info['offset']
    N.schreiben(d, omsi)
    sx, sz = 200.0 - ox, 100.0 - oz
    k = (int(sx // 300), int(sz // 300))
    pk = S.paket('K', 'gebaeude', k, [gebaeude(omsi, 'K', k, 'B2', sx + 50.0, sz + 2.0, 6.0)], S.manifest(d, 'gebaeude', k, 't'))
    pf = os.path.join(omsi, 'p.json')
    json.dump(pk, open(pf, 'w', encoding='utf-8'))
    S.einsetzen(d, pf)
    B.berechnen(d, omsi)
    # Strassen-Analyse braucht OMSI-Splines: hier ein offenes Spurende vorgeben
    monkeypatch.setattr(P, 'strassen_funde', lambda karte, omsi: (
        [dict(art='spurende_offen', x=sx, z=sz, text='Spline 1 Pfad 0')],
        dict(splines=1, objekte=2, Strasse_enden={})))
    erg = P.erstellen(d, omsi)
    assert erg['arten'] == {'spurende_offen': 1, 'gebaeude_auf_fahrbahn': 1}
    key = f'{k[0]}_{k[1]}'
    assert erg['kacheln'][key] == {'spurende_offen': 1, 'gebaeude_auf_fahrbahn': 1}
    assert erg['budgets'][key]['objekte'] >= 2 and erg['budgets'][key]['dreiecke'] >= 2
    assert [o['name'] for o in erg['prueforte']] == ['Am Wingert']
    assert os.path.isfile(os.path.join(d, 'pruefung', 'bericht.html'))
    html = open(os.path.join(d, 'pruefung', 'bericht.html'), encoding='utf-8').read()
    assert 'Gebaeude ueber Fahrbahn' in html and 'Am Wingert' in html
    # Strasse geaendert, ohne netz.json/Belegung neu zu rechnen -> beides als veraltet gemeldet
    S.schreiben(d, k, S.lesen(d, k).replace('altonaer1', 'staakener1'))
    arten = P.erstellen(d, omsi)['arten']
    assert arten.get('netz_veraltet') == 1 and arten.get('belegung_veraltet') == 1
