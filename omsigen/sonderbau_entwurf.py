"""Entwurf fuer Sonderbauten: saubere Linienfuehrung und Hoehenverlauf statt roher Messwerte.

Wie im Strassenbau: die Achse ist eine Folge von Geraden und Kreisboegen (tangential aneinander - genau die Elemente,
die OMSI-Splines haben), alle Kanten liegen in festem Abstand zur Achse (Aenderungen nur in Uebergaengen), die Hoehen
folgen Geraden mit gleichmaessiger Steigung und ausgerundeten Knicken (Kuppen/Wannen). Gemessen (DGM1, Luftbild)
wird nur, um diesen Entwurf daran anzupassen (kleinste Quadrate) und ihn zu pruefen.

Bausteine:
- Gelaende: DGM-Ausschnitt mit bilinearer Abfrage (1-m-Raster, Pixelmitten)
- stueckweise_linear: Ausgleichsgerade(n) mit freien Knickstellen (der Kern fuer Richtungs- und Hoehenverlauf)
- Achse: Geraden/Boegen aus dem Richtungsverlauf (Richtung linear in s = Bogen, konstant = Gerade)
- Gradiente: Steigungsgeraden mit Ausrundung (Parabel) an den Knicken
- Versatz: Abstand einer Kante zur Achse, stueckweise konstant mit S-foermigen Uebergaengen"""
import math
import numpy as np
from scipy.ndimage import map_coordinates
from scipy.optimize import least_squares


# ------------------------------------------------------------------ Gelaende
class Gelaende:
    """DGM-Ausschnitt: H[zeile, spalte], Zeile 0 = Sueden, Pixelmitten bei (x0 + i + 0.5, y0 + j + 0.5)"""

    def __init__(self, H, x0, y0):
        self.H, self.x0, self.y0 = np.asarray(H, float), x0, y0

    @classmethod
    def laden(cls, dgm, E0, N0, nn0, x0, x1, y0, y1):
        xs, ys = np.arange(x0, x1) + 0.5, np.arange(y0, y1) + 0.5
        XX, YY = np.meshgrid(xs, ys)
        return cls(dgm.hoehen(E0 + XX, N0 + YY) - nn0, x0, y0)

    def h(self, x, y):
        x, y = np.asarray(x, float), np.asarray(y, float)
        return map_coordinates(self.H, [y - self.y0 - 0.5, x - self.x0 - 0.5], order=1, mode='nearest')


# ------------------------------------------------------------------ Ausgleich mit freien Knicken
def _pl(s, knoten, werte):
    return np.interp(s, knoten, werte)


def stueckweise_linear(s, y, n_stuecke, w=None, fest=None, min_laenge=8.0):
    """stetige, stueckweise lineare Funktion mit `n_stuecke` Stuecken und frei liegenden Knicken, Ausgleich nach
    kleinsten Quadraten (robust: soft_l1) -> (knoten, werte). `fest`: Liste (Stueck-Index, Steigung) - etwa Geraden
    der Achse (Steigung der Richtung 0)."""
    s, y = np.asarray(s, float), np.asarray(y, float)
    w = np.ones_like(s) if w is None else np.asarray(w, float)
    a, b = s.min(), s.max()
    n = n_stuecke

    def entpacken(p):
        # innere Knoten als positive Laengen (Reihenfolge bleibt), Werte an allen Knoten
        l = np.abs(p[:n]) + min_laenge
        k = a + np.concatenate([[0], np.cumsum(l)]) * (b - a) / np.sum(l)
        return k, p[n:]

    def res(p):
        k, v = entpacken(p)
        r = (_pl(s, k, v) - y) * w
        if fest:
            r = np.concatenate([r, [100.0 * ((v[i + 1] - v[i]) / max(k[i + 1] - k[i], 1e-6) - m) for i, m in fest]])
        return r

    k0 = np.linspace(a, b, n + 1)
    v0 = np.interp(k0, s, y)
    p0 = np.concatenate([np.full(n, (b - a) / n - min_laenge * 0 + 1.0), v0])
    p0[:n] = np.maximum((b - a) / n - min_laenge, 1.0)
    erg = least_squares(res, p0, loss='soft_l1', f_scale=0.3)
    return entpacken(erg.x)


def anzahl_waehlen(s, y, toleranz, max_stuecke=8, **kw):
    """wenigste Stuecke, deren mittlere Abweichung unter `toleranz` liegt"""
    letzte = None
    for n in range(1, max_stuecke + 1):
        k, v = stueckweise_linear(s, y, n, **kw)
        rms = float(np.sqrt(np.mean((_pl(s, k, v) - y) ** 2)))
        letzte = (k, v, rms)
        if rms < toleranz:
            break
    return letzte


# ------------------------------------------------------------------ Achse aus Geraden und Boegen
class Achse:
    """Start (x, y), Richtung (Bogenmass, mathematisch: 0 = Ost) und Elemente [(Laenge, Kruemmung)] (Kruemmung > 0 =
    Linksbogen, 0 = Gerade); tangential stetig"""

    def __init__(self, x, y, richtung, elemente):
        self.x, self.y, self.richtung, self.elemente = float(x), float(y), float(richtung), [(float(l), float(k)) for l, k in elemente]

    @property
    def laenge(self):
        return sum(l for l, _ in self.elemente)

    def punkt(self, s):
        """(x, y, Richtung) bei Bogenlaenge s (ausserhalb: Verlaengerung als Gerade)"""
        x, y, th = self.x, self.y, self.richtung
        rest = float(s)
        if rest < 0:
            return x + rest * math.cos(th), y + rest * math.sin(th), th
        for l, k in self.elemente:
            d = min(rest, l)
            if abs(k) < 1e-9:
                x, y = x + d * math.cos(th), y + d * math.sin(th)
            else:
                x += (math.sin(th + k * d) - math.sin(th)) / k
                y -= (math.cos(th + k * d) - math.cos(th)) / k
                th += k * d
            rest -= d
            if rest <= 1e-12:
                return x, y, th
        return x + rest * math.cos(th), y + rest * math.sin(th), th

    def punkte(self, s):
        return np.array([self.punkt(t) for t in np.atleast_1d(s)])

    def projektion(self, P, schritt=0.5):
        """(s, t) je Punkt: Bogenlaenge und Abstand (t > 0 = rechts der Fahrtrichtung)"""
        ss = np.arange(-20, self.laenge + 20, schritt)
        A = self.punkte(ss)
        P = np.atleast_2d(np.asarray(P, float))
        out = []
        for p in P:
            i = int(np.argmin(np.hypot(A[:, 0] - p[0], A[:, 1] - p[1])))
            s = ss[i]
            for _ in range(3):                       # Newton auf der Tangente
                x, y, th = self.punkt(s)
                s += (p[0] - x) * math.cos(th) + (p[1] - y) * math.sin(th)
            x, y, th = self.punkt(s)
            t = (p[0] - x) * math.sin(th) - (p[1] - y) * math.cos(th)
            out.append((s, t))
        return np.array(out)

    def versetzt(self, s, t):
        """Punkte im Abstand t (rechts positiv) bei s"""
        s, t = np.broadcast_arrays(np.atleast_1d(s).astype(float), np.atleast_1d(t).astype(float))
        A = self.punkte(s)
        return np.stack([A[:, 0] + t * np.sin(A[:, 2]), A[:, 1] - t * np.cos(A[:, 2])], axis=1)

    def text(self):
        z = []
        for l, k in self.elemente:
            z.append(f'Gerade {l:6.1f} m' if abs(k) < 1e-6 else f'Bogen  {l:6.1f} m  R = {1 / k:+8.1f} m ({"links" if k > 0 else "rechts"})')
        return '\n'.join(z)

    def json(self):
        return dict(x=self.x, y=self.y, richtung_grad=math.degrees(self.richtung), elemente=[dict(laenge=l, radius=None if abs(k) < 1e-9 else 1 / k) for l, k in self.elemente])


def achse_anpassen(P, toleranz=0.15, max_elemente=7, gerade_unter=1 / 3000.0):
    """Achse (Geraden + Boegen) durch die Punktfolge P (geordnet, ~gleichmaessig): Richtungsverlauf ueber der
    Bogenlaenge stueckweise linear ausgleichen (linear = Bogen, waagerecht = Gerade), dann Lage, Startrichtung, Laengen
    und Kruemmungen zusammen gegen die Punkte nachfuehren -> (Achse, mittlere Abweichung)"""
    P = np.asarray(P, float)
    d = np.diff(P, axis=0)
    s = np.concatenate([[0], np.cumsum(np.hypot(d[:, 0], d[:, 1]))])
    th = np.unwrap(np.arctan2(d[:, 1], d[:, 0]))
    sm = (s[:-1] + s[1:]) / 2
    beste = None
    for n in range(1, max_elemente + 1):
        k, v = stueckweise_linear(sm, th, n, min_laenge=10.0)
        elemente = []
        for i in range(n):
            l = k[i + 1] - k[i]
            kr = (v[i + 1] - v[i]) / l
            elemente.append((l, 0.0 if abs(kr) < gerade_unter else kr))
        # Elemente reichen von s=0 bis Ende
        elemente[0] = (elemente[0][0] + (k[0] - 0), elemente[0][1])
        elemente[-1] = (elemente[-1][0] + (s[-1] - k[-1]), elemente[-1][1])
        a = Achse(P[0, 0], P[0, 1], v[0] - elemente[0][1] * (k[0] - 0), elemente)
        a = _nachfuehren(a, P)
        # sehr flache Boegen nach dem Nachfuehren werden Geraden (R > 3 km sieht niemand), dann nochmals nachfuehren
        if any(0 < abs(k) < gerade_unter for _, k in a.elemente):
            a = _nachfuehren(Achse(a.x, a.y, a.richtung, [(l, 0.0 if abs(k) < gerade_unter else k) for l, k in a.elemente]), P)
        r = np.abs(a.projektion(P)[:, 1])
        rms = float(np.sqrt(np.mean(r ** 2)))
        if beste is None or rms < beste[1] - 0.02:
            beste = (a, rms)
        if rms < toleranz:
            break
    return beste


def _nachfuehren(a, P):
    """Lage, Richtung, Laengen und Kruemmungen gemeinsam nach kleinsten Quadraten (Geraden bleiben Geraden)"""
    n = len(a.elemente)
    gerade = [abs(k) < 1e-12 for _, k in a.elemente]
    sub = P[:: max(1, len(P) // 120)]

    def bauen(p):
        el = [(max(p[3 + i], 1.0), 0.0 if gerade[i] else p[3 + n + i]) for i in range(n)]
        return Achse(p[0], p[1], p[2], el)

    def res(p):
        b = bauen(p)
        st = b.projektion(sub, schritt=1.0)
        r = list(st[:, 1])
        # Anfang und Ende auf die Endpunkte (s = 0 und Laenge), damit die Achse nicht wandert
        r.append(st[0, 0] * 0.2)
        r.append((st[-1, 0] - b.laenge) * 0.2)
        return np.array(r)

    p0 = np.concatenate([[a.x, a.y, a.richtung], [l for l, _ in a.elemente], [k for _, k in a.elemente]])
    erg = least_squares(res, p0, loss='soft_l1', f_scale=0.3, x_scale='jac')
    return bauen(erg.x)


# ------------------------------------------------------------------ Gradiente
class Gradiente:
    """Hoehe ueber der Bogenlaenge: Tangenten durch die Neigungswechsel (s_i, z_i), an jedem inneren Knick eine
    quadratische Ausrundung der Laenge T_i (Tangentenlaenge je Seite)"""

    def __init__(self, knoten, werte, ausrundung=None):
        self.k, self.v = np.asarray(knoten, float), np.asarray(werte, float)
        n = len(self.k)
        self.T = np.zeros(n) if ausrundung is None else np.asarray(ausrundung, float)

    def z(self, s):
        s = np.atleast_1d(np.asarray(s, float))
        z = np.interp(s, self.k, self.v)
        # Verlaengerung ausserhalb mit der Endneigung
        g0 = (self.v[1] - self.v[0]) / (self.k[1] - self.k[0])
        g1 = (self.v[-1] - self.v[-2]) / (self.k[-1] - self.k[-2])
        z = np.where(s < self.k[0], self.v[0] + (s - self.k[0]) * g0, z)
        z = np.where(s > self.k[-1], self.v[-1] + (s - self.k[-1]) * g1, z)
        for i in range(1, len(self.k) - 1):
            T = self.T[i]
            if T <= 0:
                continue
            ga = (self.v[i] - self.v[i - 1]) / (self.k[i] - self.k[i - 1])
            gb = (self.v[i + 1] - self.v[i]) / (self.k[i + 1] - self.k[i])
            m = np.abs(s - self.k[i]) < T
            u = s[m] - (self.k[i] - T)
            z[m] = self.v[i] - ga * T + ga * u + (gb - ga) / (4 * T) * u ** 2
        return z

    def neigungen(self):
        return np.diff(self.v) / np.diff(self.k)

    def halbmesser(self):
        """Ausrundungshalbmesser je innerem Knick (+ Wanne, - Kuppe)"""
        g = self.neigungen()
        return [2 * self.T[i] / (g[i] - g[i - 1]) if self.T[i] > 0 and abs(g[i] - g[i - 1]) > 1e-9 else None for i in range(1, len(self.k) - 1)]

    def text(self):
        g = self.neigungen()
        z = [f'{self.k[i]:7.1f} m  {self.v[i]:6.2f} m' + (f'  -> {g[i] * 100:+5.2f} %' if i < len(g) else '') for i in range(len(self.k))]
        return '\n'.join(z)

    def json(self):
        return dict(knoten=self.k.tolist(), hoehen=self.v.tolist(), ausrundung=self.T.tolist())


def gradiente_anpassen(s, z, toleranz=0.08, max_stuecke=6, ausrunden=0.35, min_laenge=15.0):
    """Gradiente durch Messhoehen: Tangenten frei ausgleichen, Knicke mit `ausrunden` x kuerzerer Nachbartangente
    ausrunden, dann Hoehen nochmals mit Ausrundung nachfuehren"""
    k, v, _ = anzahl_waehlen(s, z, toleranz, max_stuecke=max_stuecke, min_laenge=min_laenge)
    n = len(k)
    T = np.zeros(n)
    for i in range(1, n - 1):
        T[i] = ausrunden * min(k[i] - k[i - 1], k[i + 1] - k[i])

    def res(p):
        return Gradiente(k, p, T).z(s) - z

    v = least_squares(res, v, loss='soft_l1', f_scale=0.2).x
    return Gradiente(k, v, T)


def ohne_ausreisser(s, y, anpassen, auswerten, grenze, runden=2):
    """anpassen, Punkte mit Abweichung > grenze weglassen, nochmals anpassen (Messfehler am Rand des Trogs)"""
    s, y = np.asarray(s, float), np.asarray(y, float)
    ok = ~np.isnan(y)
    erg = anpassen(s[ok], y[ok])
    for _ in range(runden):
        r = np.abs(auswerten(erg, s) - y)
        neu = ~np.isnan(y) & (r < grenze)
        if neu.sum() < 10 or (neu == ok).all():
            break
        ok = neu
        erg = anpassen(s[ok], y[ok])
    return erg, ok


# ------------------------------------------------------------------ Versatz (Kanten)
def s_kurve(u):
    """0..1 glatt (kubisch, Anfang und Ende tangential)"""
    u = np.clip(u, 0, 1)
    return u * u * (3 - 2 * u)


class Versatz:
    """Abstand einer Kante zur Achse ueber s: Abschnitte [(s_von, s_bis, t)], dazwischen S-Uebergaenge; ausserhalb
    der Abschnitte gibt es die Kante nicht (None)"""

    def __init__(self, abschnitte):
        self.a = sorted([tuple(map(float, x)) for x in abschnitte])

    def t(self, s):
        s = np.atleast_1d(np.asarray(s, float))
        out = np.full(s.shape, np.nan)
        for i, (a, b, t) in enumerate(self.a):
            out[(s >= a) & (s <= b)] = t
            if i + 1 < len(self.a):
                a2, _, t2 = self.a[i + 1]
                m = (s > b) & (s < a2)
                out[m] = t + (t2 - t) * s_kurve((s[m] - b) / (a2 - b))
        return out

    def json(self):
        return [dict(von=a, bis=b, abstand=t) for a, b, t in self.a]


def versatz_anpassen(s, t, toleranz=0.12, max_stuecke=6, min_laenge=12.0, raster=0.05, zusammen=0.15):
    """Kante aus Messpunkten: stueckweise linear ausgleichen, flache Stuecke (|Neigung| < 1 %) werden Abschnitte mit
    festem Abstand (gerundet auf `raster`), schraege Stuecke Uebergaenge"""
    ok = ~np.isnan(t)
    s, t = np.asarray(s)[ok], np.asarray(t)[ok]
    k, v, _ = anzahl_waehlen(s, t, toleranz, max_stuecke=max_stuecke, min_laenge=min_laenge)
    abschnitte = []
    for i in range(len(k) - 1):
        g = (v[i + 1] - v[i]) / (k[i + 1] - k[i])
        if abs(g) < 0.01:
            m = (s >= k[i]) & (s <= k[i + 1])
            wert = float(np.median(t[m])) if m.any() else (v[i] + v[i + 1]) / 2
            abschnitte.append([k[i], k[i + 1], round(wert / raster) * raster])
    if not abschnitte:
        abschnitte = [[s.min(), s.max(), round(float(np.median(t)) / raster) * raster]]
    # benachbarte Abschnitte mit fast gleichem Abstand sind eine Kante (Unterschied = Messrauschen)
    i = 0
    while i + 1 < len(abschnitte):
        (a0, b0, t0), (a1, b1, t1) = abschnitte[i], abschnitte[i + 1]
        if abs(t1 - t0) <= zusammen:
            m = (s >= a0) & (s <= b1)
            abschnitte[i] = [a0, b1, round(float(np.median(t[m])) / raster) * raster if m.any() else t0]
            del abschnitte[i + 1]
        else:
            i += 1
    # Abschnitte bis an die Enden ziehen, wenn dort keine Uebergaenge liegen
    abschnitte[0][0] = min(abschnitte[0][0], s.min())
    abschnitte[-1][1] = max(abschnitte[-1][1], s.max())
    return Versatz(abschnitte)


# ------------------------------------------------------------------ Laden (entwurf.json)
def achse_aus_json(j):
    return Achse(j['x'], j['y'], math.radians(j['richtung_grad']),
                 [(e['laenge'], 0.0 if e['radius'] is None else 1.0 / e['radius']) for e in j['elemente']])


def gradiente_aus_json(j):
    return Gradiente(j['knoten'], j['hoehen'], j['ausrundung'])


def versatz_aus_json(j):
    return Versatz([(a['von'], a['bis'], a['abstand']) for a in j])


def weich_max(a, b, k=0.4):
    """glattes Maximum (Uebergang ueber etwa k): Rampe laeuft ohne Knick in den Ring ein"""
    return (a + b + np.sqrt((a - b) ** 2 + k * k)) / 2
