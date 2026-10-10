"""Vermessung realer Bauwerke aus den Laserdaten (Airborne-Laserscan) der Bayerischen Vermessungsverwaltung (CC BY 4.0):
Punkte eines Ausschnitts laden, Querschnitte ziehen und darin Flaechen (ebene Laeufe der Bodenpunkte) und Kanten
(Hoehenspruenge = Wand, Bord, Boeschungsoberkante) finden, die Kanten zu Linien verbinden.

Ziel: ein Bauwerk vermessen statt raten - Lage und Hoehe jeder Wand, Flaeche und Kante aus echten Messpunkten.
Klassen in den Daten: 2 Boden, 6 Gebaeude, 20 Rest (u. a. Bruecken, Vegetation)."""
import math

import numpy as np

E0, N0, NN0 = 510466.0, 5536235.0, 130.0


def laden(laz, bereich, ost0=E0, nord0=N0, nn0=NN0):
    """LAZ -> Punkte (n x 4: x, y, z in Kartenkoordinaten, Klasse) im Ausschnitt (x0, y0, x1, y1)"""
    import laspy
    x0, y0, x1, y1 = bereich
    teile = []
    with laspy.open(laz) as f:
        for ch in f.chunk_iterator(5_000_000):
            X, Y = np.asarray(ch.x) - ost0, np.asarray(ch.y) - nord0
            m = (X >= x0) & (X <= x1) & (Y >= y0) & (Y <= y1)
            if m.any():
                teile.append(np.c_[X[m], Y[m], np.asarray(ch.z)[m] - nn0, np.asarray(ch.classification)[m]])
    return np.vstack(teile) if teile else np.zeros((0, 4))


def querschnitt(P, ys, breite=0.3, zelle=0.2, klassen=(2,)):
    """Bodenpunkte im Streifen |y - ys| < breite -> (x-Mitten, Median-z je Zelle, NaN ohne Punkte)"""
    m = (np.abs(P[:, 1] - ys) < breite) & np.isin(P[:, 3], klassen)
    if not m.any():
        return np.zeros(0), np.zeros(0)
    x, z = P[m, 0], P[m, 2]
    x0 = math.floor(x.min() / zelle) * zelle
    i = ((x - x0) / zelle).astype(int)
    n = i.max() + 1
    zs = np.full(n, np.nan)
    for k in range(n):
        s = z[i == k]
        if len(s):
            zs[k] = np.median(s)
    return x0 + (np.arange(n) + 0.5) * zelle, zs


def kanten(xs, zs, sprung=0.5, luecke=2.0):
    """Hoehenspruenge im Querschnitt: zwischen zwei Zellen mit Punkten (hoechstens `luecke` m Abstand, dazwischen
    nichts = senkrechte Flaeche ohne Laserpunkte) mehr als `sprung` m Unterschied -> [(x, z_unten, z_oben, Seite)]
    Seite: +1 = oben liegt rechts (Osten), -1 = oben liegt links"""
    ok = ~np.isnan(zs)
    X, Z = xs[ok], zs[ok]
    out = []
    for a in range(len(X) - 1):
        dx, dz = X[a + 1] - X[a], Z[a + 1] - Z[a]
        if dx <= luecke and abs(dz) > sprung:
            # Lage der Wand: an der Seite der hoeheren Flaeche (dort steht sie), Mitte der Luecke sonst
            x = X[a + 1] - 0.1 if dz > 0 else X[a] + 0.1
            out.append((round(float(x), 2), round(float(min(Z[a], Z[a + 1])), 2), round(float(max(Z[a], Z[a + 1])), 2), 1 if dz > 0 else -1))
    return out


def linien(schnitte, abstand=1.2, min_laenge=4):
    """Kanten der Querschnitte zu Linien verbinden: gleiche Seite, x hoechstens `abstand` m vom letzten Punkt der Linie
    -> Liste [(y, x, z_unten, z_oben)] je Linie (mindestens `min_laenge` Schnitte)"""
    offen, fertig = [], []
    for ys, ks in schnitte:
        neu = []
        frei = list(ks)
        for L in offen:
            yl, xl, _, _, sl = L[-1]
            best = min((k for k in frei if k[3] == sl), key=lambda k: abs(k[0] - xl), default=None)
            if best is not None and abs(best[0] - xl) <= abstand:
                L.append((ys, best[0], best[1], best[2], best[3]))
                frei.remove(best)
                neu.append(L)
            else:
                fertig.append(L)
        for k in frei:
            neu.append([(ys, k[0], k[1], k[2], k[3])])
        offen = neu
    fertig += offen
    return [L for L in fertig if len(L) >= min_laenge]
