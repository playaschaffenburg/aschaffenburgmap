import os
import numpy as np
import pytest

pytest.importorskip('PySide6')
from omsigen import pipeline
from omsieditor import geometrie3d as g3
from omsieditor.render3d import Kamera, treffer

SAMPLE = os.path.join(os.path.dirname(__file__), '..', 'samples', 'aschaffenburg_hbf_citygalerie.json')

SLI = """[texture]
str_asphdrk.bmp

[profile]
0

[profilepnt]
-3
0.1
0
0.2

[profilepnt]
3
0.1
1
0.2
"""


def test_spline_profil_entlang_eines_bogens():
    sli = g3.sli_lesen(SLI)
    assert sli['texturen'] == ['str_asphdrk.bmp'] and len(sli['profile']) == 1
    sli['pfade'] = [None]
    sz = g3.Szene()
    g3.spline_dreiecke(sz, [0.0, 0.0, 0.0, 20.0, 50.0, 'x.sli'], 10.0, 5.0, sli)     # Rechtsbogen, 5 % Steigung
    A = np.concatenate(sz[g3.FARBE_FAHRBAHN])
    assert len(A) % 3 == 0 and len(A) >= 6 * 10
    assert A[:, 1].min() == pytest.approx(10.1) and A[:, 1].max() == pytest.approx(10.1 + 1.0, abs=1e-4)
    assert A[:, 0].max() > 3.0                                       # Bogen nach rechts (Osten)
    assert A[:, 7].max() == pytest.approx(0.2 * 20, abs=1e-4)        # v waechst mit der Laenge


def test_objekt_drehung_wie_omsi():
    """Objekt mit Drehung 90 Grad: lokale z-Achse (vorwaerts) zeigt nach Osten"""
    V = [(0, 0, 0, 0, 1, 0, 0, 0), (0, 0, 10, 0, 1, 0, 0, 1), (1, 0, 0, 0, 1, 0, 1, 0)]
    sz = g3.Szene()
    g3.objekt_dreiecke(sz, dict(mesh=(V, [(0, 1, 2, 0)]), mats=[('m', None)], origin=(100, 200), hoehe=5, rot=90), [])
    A = np.concatenate(next(iter(sz.values())))
    assert A[1, 0] == pytest.approx(110) and A[1, 2] == pytest.approx(200) and A[1, 1] == pytest.approx(5)
    assert A[2, 2] == pytest.approx(199)                             # lokal rechts = Sueden


def test_raster_und_klick():
    R = g3.Raster(0.0, 0.0)
    R.kacheln[(0, 0)] = np.fromfunction(lambda j, i: i * 0.5, (61, 61), dtype=np.float32)   # 10 % nach Osten
    assert R.hoehe(100, 50) == pytest.approx(10.0)
    k = Kamera(); k.ziel = [150, 15, 150]; k.abstand = 200; k.neigung = -45
    o, d = k.strahl(400, 300, 800, 600)                              # Bildmitte trifft den Blickpunkt
    p = treffer(o, d, R.hoehe)
    assert p[0] == pytest.approx(150, abs=0.5) and p[2] == pytest.approx(150, abs=0.5)
    T = R.dreiecke((0, 0))
    assert T.shape == (60 * 60 * 6, 8)


def test_szene_aus_berechnung():
    P = pipeline.importiere('49.98053,9.14023', '49.97790,9.14998', osm_datei=SAMPLE, log=lambda *a: None)
    B = pipeline.berechne(P, gelaende=False, log=lambda *a: None)
    sz, R, basis = g3.szene_aus_berechnung(B, None)
    assert sz.dreiecke() > 5000 and len(R.kacheln) >= 9 and basis == 0.0
    # ohne OMSI-Ordner: Ersatzfarben fuer Fahrbahn und Gehweg
    assert g3.FARBE_FAHRBAHN in sz and g3.FARBE_GEHWEG in sz


def test_rendern_ohne_fenster(tmp_path):
    from omsieditor import bild3d
    if os.environ.get('QT_QPA_PLATFORM') == 'offscreen' or not bild3d.kontext():
        pytest.skip('kein OpenGL-3.3-Kontext')
    sz = g3.Szene()
    g3.objekt_dreiecke(sz, dict(mesh=([(-20, 0, -20, 0, 1, 0, 0, 0), (20, 0, -20, 0, 1, 0, 1, 0), (0, 10, 20, 0, 1, 0, 0, 1)],
                                      [(0, 1, 2, 0)]), mats=[('m', 'farbe:#ff0000')], origin=(150, 150), hoehe=1),
                       [])
    R = g3.Raster(0.0, 0.0); R.kacheln[(0, 0)] = np.zeros((61, 61), dtype=np.float32)
    k = Kamera(); k.ziel = [150, 0, 150]; k.abstand = 120
    png = bild3d.rendern(sz.fertig(), R, k, str(tmp_path / 'b.png'), 320, 200)
    from PySide6.QtGui import QImage
    img = QImage(png)
    farben = {img.pixel(x, y) for x in range(0, 320, 16) for y in range(0, 200, 16)}
    assert len(farben) >= 3                                          # Himmel, Gelaende, Objekt
