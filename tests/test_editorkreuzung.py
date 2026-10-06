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
