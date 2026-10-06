import os
import pytest

pytest.importorskip('PySide6')
os.environ.setdefault('QT_QPA_PLATFORM', 'offscreen')
os.environ['OMSIEDITOR_OHNE_LUFTBILD'] = '1'
os.environ['OMSIEDITOR_OHNE_3D'] = '1'          # Testplattform ohne OpenGL; 3D: test_ansicht3d.py

from omsigen import pipeline
from omsieditor.modell import Modell, standard_tags, klasse_von

SAMPLE = os.path.join(os.path.dirname(__file__), '..', 'samples', 'aschaffenburg_hbf_citygalerie.json')


def test_zeichnen_erzeugt_kreuzung():
    m = Modell.neu(49.98, 9.14)
    m.strasse_hinzufuegen([(-50, 0), (50, 0)], standard_tags('secondary', 'Haupt'))
    q = m.einrasten((0.6, -1.0), 3)                       # Klick neben die Strasse: rastet ein, Stuetzpunkt entsteht
    assert q == pytest.approx((0.6, 0.0)) and m.p['strassen'][0]['punkte'][1] == pytest.approx([0.6, 0.0])
    m.strasse_hinzufuegen([q, (0, -60)], standard_tags('residential', 'Neben'))
    assert m.kreuzungspunkte() == [(0.6, 0.0)]
    q = tuple(m.p['strassen'][0]['punkte'][1])
    assert m.einrasten((49, 1), 3) == (50, 0)             # vorhandener Punkt hat Vorrang
    m.punkt_verschieben(q, (2.0, 0.0))                    # gemeinsamer Punkt bewegt beide Strassen
    assert m.p['strassen'][1]['punkte'][0] == [2.0, 0.0]
    m.rueckgaengig(); assert m.p['strassen'][1]['punkte'][0] == pytest.approx([0.6, 0.0])
    m.wiederholen(); assert m.p['strassen'][1]['punkte'][0] == [2.0, 0.0]


def test_tags_und_speichern(tmp_path):
    m = Modell.neu(49.98, 9.14, 'T')
    sid = m.strasse_hinzufuegen([(0, 0), (30, 0)], standard_tags('bus'))
    assert klasse_von(m.strasse(sid)['tags']) == 'bus'
    m.tags_setzen(sid, standard_tags('residential', 'Weg', 'entgegen', 0, '30'))
    assert m.strasse(sid)['tags'] == {'highway': 'residential', 'name': 'Weg', 'oneway': '-1', 'maxspeed': '30'}
    f = str(tmp_path / 'p.omsiprojekt')
    m.speichern(f)
    m2 = Modell.oeffnen(f)
    assert m2.p['strassen'] == m.p['strassen'] and not m2.geaendert


def test_projekt_zu_karte(tmp_path):
    """leeres Projekt, zwei gezeichnete Strassen -> Karte mit einer Kreuzung, alles verbunden"""
    m = Modell.neu(49.98, 9.14, 'Gezeichnet')
    m.strasse_hinzufuegen([(-80, 0), (0, 0), (80, 0)], standard_tags('secondary', 'Haupt'))
    m.strasse_hinzufuegen([(0, 0), (0, -80)], standard_tags('residential', 'Neben'))
    r = pipeline.erzeuge(m.p, 'Gezeichnet', ausgabe=str(tmp_path), log=lambda *a: None)
    assert r['rc'] == 0 and r['kreuzungen'] >= 1 and r['stats']['wendeschleifen'] == 3


def test_fenster_startet():
    from PySide6.QtWidgets import QApplication
    from omsieditor.fenster import Fenster
    app = QApplication.instance() or QApplication([])
    P = pipeline.importiere('49.98053,9.14023', '49.97790,9.14998', osm_datei=SAMPLE, log=lambda *a: None)
    w = Fenster(Modell(P))
    w.show(); w.karte.alles_zeigen()
    assert len(w.karte._strassen) == len(P['strassen']) and w.karte._kreuz
    sid = P['strassen'][0]['id']
    w.karte.auswaehlen(sid)
    assert w.eig.titel.text().startswith(f'<b>Strasse {sid}')
    w.eig.klasse.setCurrentIndex(5)                       # Klasse aendern -> Tags der Strasse aendern sich
    assert w.m.strasse(sid)['tags']['highway'] == 'living_street' and w.m.geaendert
    w.m.geaendert = False
    w.close()


def test_hoehen_im_modell():
    m = Modell.neu(49.98, 9.14)
    sid = m.strasse_hinzufuegen([(0, 0), (100, 0)], standard_tags('secondary'), hoehen=[0.0, 8.0])
    assert m.strasse(sid)['hoehen'] == [0.0, 8.0]
    q = m.einrasten((50, 1), 3)                          # Stuetzpunkt in der Mitte bekommt die Zwischenhoehe
    assert m.strasse(sid)['hoehen'] == [0.0, 4.0, 8.0]
    sid2 = m.strasse_hinzufuegen([q, (50, -60)], standard_tags('residential'))
    assert 'hoehen' not in m.strasse(sid2)
    m.punkt_hoehe_setzen(q, 6.5)                         # gemeinsamer Punkt: beide Strassen
    assert m.punkt_hoehe(q) == 6.5 and m.strasse(sid2)['hoehen'] == [6.5, None]
    m.umkehren(sid)
    assert m.strasse(sid)['hoehen'] == [8.0, 6.5, 0.0]
    m.rueckgaengig(); m.rueckgaengig()
    assert m.strasse(sid)['hoehen'] == [0.0, 4.0, 8.0] and 'hoehen' not in m.strasse(sid2)


def test_keine_numpy_zahlen_im_projekt(tmp_path):
    import json, numpy as np
    m = Modell.neu(49.98, 9.14)
    q = m.einrasten((np.float64(1.5), np.float64(2.0)), 3)
    sid = m.strasse_hinzufuegen([q, (np.float64(40.0), np.float64(2.0))], standard_tags(), hoehen=[np.float64(1.0), 0.0])
    m.punkt_verschieben((40.0, 2.0), (np.float64(41.0), np.float64(3.0)))
    json.dumps(m.p)                                     # wirft bei numpy-Zahlen
    assert type(m.strasse(sid)['punkte'][1][0]) is float and type(m.strasse(sid)['hoehen'][0]) is float


def test_gezeichnete_bruecke_in_der_karte(tmp_path):
    """Strasse mit Hoehen 0 / 8 / 0 ueber flachem Land -> Bruecke mit Pfeilern in der erzeugten Karte"""
    m = Modell.neu(49.98, 9.14, 'Bruecke')
    m.strasse_hinzufuegen([(0, 0), (120, 0), (240, 0)], standard_tags('secondary', 'Hoch'), hoehen=[0.0, 8.0, 0.0])
    r = pipeline.erzeuge(m.p, 'Bruecke', ausgabe=str(tmp_path), gelaende=False, log=lambda *a: None)
    assert r['rc'] == 0
    import glob
    text = ''.join(open(f, encoding='utf-16').read() for f in glob.glob(str(tmp_path / 'maps' / 'Bruecke' / 'tile_*.map')))
    assert 'AB_bruecke_' in text
