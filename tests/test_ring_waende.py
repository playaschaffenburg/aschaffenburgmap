import math
import numpy as np

from omsigen import ring_waende as rw
from omsigen.ring_strassen import _ende


def _abtasten(els, schritt=0.5):
    """Punkte entlang der Kette (OMSI-Bogenregel) und Richtung an den Uebergaengen"""
    pts = []
    for e in els:
        n = max(1, int(e['laenge'] / schritt))
        for i in range(n + 1):
            f = dict(e, laenge=e['laenge'] * i / n)
            x, y, _ = _ende(f)
            pts.append((x, y))
    return np.array(pts)


def test_kette_trifft_punkte_und_ist_glatt():
    # S-Kurve mit wechselndem Abstand (wie eine Wandlinie, deren Rampe auseinanderlaeuft)
    s = np.arange(0, 160.0, 2.0)
    P = np.c_[s, 6.0 * np.tanh((s - 80) / 25.0) + 0.002 * s ** 1.5]
    Z = 5.0 + 0.04 * s
    els = rw.kette(P, Z)
    assert all(e['laenge'] <= rw.KNOTEN * 1.05 for e in els)
    # Ende genau am letzten Punkt
    x, y, _ = _ende(els[-1])
    assert math.hypot(x - P[-1, 0], y - P[-1, 1]) < 1e-6
    # jeder Element-Anfang liegt auf der Kette des Vorgaengers, Richtung stetig
    for a, b in zip(els, els[1:]):
        xa, ya, ha = _ende(a)
        assert math.hypot(xa - b['x'], ya - b['y']) < 1e-6
        assert abs((ha - b['richtung'] + 180) % 360 - 180) < 1e-6
    # die Kette weicht hoechstens wenige cm von der Linie ab
    K = _abtasten(els)
    d = [np.min(np.hypot(*(P - k).T)) for k in K]
    assert max(d) < 1.05          # Punkte liegen 2 m auseinander: Abstand zum naechsten Punkt <= 1 m + Abweichung
    from shapely.geometry import LineString, Point
    linie = LineString(P)
    assert max(linie.distance(Point(k)) for k in K) < 0.05
    # Hoehen: Anfang + dh je Element ergibt die Krone am Ende
    assert abs(els[-1]['z'] + els[-1]['dh'] - Z[-1]) < 1e-9


def test_profil_seiten_und_tiefe():
    # Trennwand an der Rampe: Betonbruestung 1,0 m mit Handlauf (Fotos), Handlauf von beiden Seiten mit u = 0 unten
    p = rw.profil('trenn', 'klinker', 'betonbr', 'keine', -1, 7.0)
    txt = p.text('Test')
    assert '[matl_alpha]' in txt and rw.TEX['handlauf'] in txt
    ys = [y for _, pts in p.teile for _, y, _ in pts]
    assert min(ys) == -7.0 and abs(max(ys) - (rw.BR_HOCH + rw.HANDLAUF)) < 1e-9
    hl = [pts for t, pts in p.teile if t == 'handlauf']
    assert len(hl) == 2 and all(min(pts, key=lambda q: q[1])[2] == 0.0 for pts in hl)
    # Ring-Seite einer Westwand liegt bei x < 0 (Strasse oestlich, Laufrichtung Nord -> Sued): dort ist die
    # Klinkerflaeche, sichtbar nach links (von unten nach oben gelesen)
    klinker = [pts for t, pts in p.teile if t == 'klinker']
    assert len(klinker) == 1 and klinker[0][0][0] < 0 and klinker[0][1][1] > klinker[0][0][1]
    a = rw.profil('aussen', 'klinker', 'stufig', 'glas', 1, 9.0)
    assert any(t == 'pflanzen' for t, _ in a.teile) and sum(t == 'glas' for t, _ in a.teile) == 2
