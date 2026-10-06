"""Hoehen fuer die Karte: Strassenprofile mit Steigungen, flache Kreuzungen, Bruecken und Tunnel, Gelaenderaster.

Alle Hoehen in der Karte sind relativ zu einer Basishoehe (Meter ueber NN, Standard: tiefster Strassenpunkt - 2 m).
- Kreuzung: flach auf Gelaendehoehe ihres Mittelpunkts; die Strassen enden genau auf dieser Hoehe.
- Strasse: Gelaende entlang der Strecke, geglaettet (gleitendes Mittel ueber GLAETTEN m).
- Bruecke / Tunnel (OSM bridge / tunnel): linear zwischen den Hoehen an ihren Enden (das DGM zeigt dort den
  Boden darunter bzw. darueber, nicht die Fahrbahn).
- Spline: Starthoehe + gleichbleibende Steigung (Prozent) je Element; Elemente sind hoechstens TEILEN m lang.
- Gelaende: DGM im 5-m-Raster der Kacheln; unter Strassen und Kreuzungen auf Fahrbahnhoehe abgesenkt (nicht unter
  Bruecken und ueber Tunneln), mit Uebergang zum Gelaende."""
import math
import numpy as np
from .geom import end_of, sample

TEILEN = 20.0          # m: laengere Elemente werden geteilt (Hoehenverlauf folgt dem Gelaende)
GLAETTEN = 30.0        # m: Fenster des gleitenden Mittels
UNTER_FAHRBAHN = 0.05  # m: Gelaende unter der Fahrbahn so weit unter der Spline-Hoehe
UEBERGANG = 4.0        # m: Breite des Uebergangs von der Strasse zum Gelaende
STEIGUNG_GEWICHT = 4.0  # Gewicht der Steigungsstrafe in der Ausgleichsrechnung
LICHTE_HOEHE = 5.5      # m: Brueckenfahrbahn mindestens so weit ueber dem Boden darunter
UEBERDECKUNG = 6.0      # m: Tunnelfahrbahn mindestens so weit unter der Oberflaeche
RAMPE = 0.12            # der geforderte Abstand waechst vom Bauwerksende an mit hoechstens 12 %
KREUZUNG_GEWICHT = 5.0  # Kreuzungen halten sich stark ans Gelaende (flache Platte)
ZIEL_GEWICHT = 20.0     # Hoehenvorgaben aus dem Editor (Projekt: strasse['hoehen']) gehen vor dem Gelaende
ZIEL_RADIUS = 3.0       # m: Vorgabe gilt fuer Netzpunkte in diesem Umkreis


def zerlegen(els, lvls, maxlen=TEILEN):
    """lange Elemente in gleich lange Stuecke teilen (Bogen behalten ihren Radius)"""
    out, ol = [], []
    for el, lv in zip(els, lvls):
        n = max(1, math.ceil(el[3] / maxlen))
        x, z, h = el[0], el[1], el[2]
        for _ in range(n):
            e = [x, z, h, el[3] / n, el[4], el[5]]
            out.append(e); ol.append(lv)
            (x, z), h = end_of(e)
    return out, ol


def _glaetten(s, h, fest, fenster=GLAETTEN):
    """gleitendes Mittel ueber die Strecke; feste Punkte bleiben"""
    out = list(h)
    for i in range(len(h)):
        if fest[i]:
            continue
        w = [j for j in range(len(h)) if abs(s[j] - s[i]) <= fenster / 2]
        out[i] = sum(h[j] for j in w) / len(w)
    return out


class Hoehen:
    def __init__(self, gel, log=print):
        self.gel, self.log = gel, log
        self.base = 0.0
        self.kreuz = {}            # Knoten -> Hoehe (relativ)
        self.proben = []           # (x, z, h, innen) fuer das Gelaende: Strassen
        self.flaechen = []         # (x, z, h) Punkte auf Kreuzungsflaechen

    def gelaende(self, x, z):
        h = self.gel.hoehe(x, z) if self.gel else None
        return None if h is None else h - self.base

    # ---------------------------------------------------------------- Strassen und Kreuzungen
    def berechnen(self, net, sdb, kreuzungen=(), ziele=()):
        """Hoehen aller Strassen als Ausgleichsrechnung ueber das ganze Netz:
        - jeder Punkt auf Grund (nicht Bruecke/Tunnel) moeglichst nah am Gelaende,
        - Steigung aendert sich entlang der Strasse nur sanft (zweite Ableitung klein, Massstab GLAETTEN),
        - alle Arme einer Kreuzung teilen eine Hoehe; liegt die Mehrheit der Arme auf Bruecke/im Tunnel, hat die
          Kreuzung keinen Gelaendewert (das DGM zeigt dort den Boden darunter/darueber).
        - Hoehenvorgaben ziele [(x, z, Hoehe ue. NN)] (Editor, pipeline.hoehenziele) gehen mit ZIEL_GEWICHT vor.
        Setzt ch['els'] (geteilt), ch['ebene'], ch['y'], ch['g']; Wendeschleifen linear; Kreuzungsobjekte 'hoehe'."""
        from scipy.sparse import coo_matrix
        from scipy.sparse.linalg import lsqr
        chains = net['road_chains']
        for ch in chains:
            ch['els'], ch['ebene'] = zerlegen(ch['els'], ch.get('ebene') or [None] * len(ch['els']))
        werte = [self.gel.hoehe(e[0], e[1]) for ch in chains for e in ch['els']] if self.gel else []
        werte = [h for h in werte if h is not None] + [h for _, _, h in ziele]
        self.base = math.floor(min(werte)) - 2.0 if werte and self.gel else 0.0

        nvar = 0
        jvar = {}
        for k in net['arms']:
            jvar[k] = nvar; nvar += 1
        rows, cols, vals, rhs = [], [], [], []
        nrow = 0

        def zeile(eintraege, ziel):
            nonlocal nrow
            for c, v in eintraege:
                rows.append(nrow); cols.append(c); vals.append(v)
            rhs.append(ziel); nrow += 1
        bauwerk_arme = {}                        # Kreuzung -> Anzahl Arme auf Bruecke/im Tunnel
        bauwerk = []                             # (Variable, Boden, 'bruecke'/'tunnel') fuer die Ungleichungen
        info = []
        for ch in chains:
            els, lv = ch['els'], ch['ebene']
            pts = [(e[0], e[1]) for e in els] + [end_of(els[-1])[0]]
            s = [0.0]
            for e in els:
                s.append(s[-1] + e[3])
            var = []
            for i in range(len(pts)):
                knoten = ch.get('node_s') if i == 0 else ch.get('node_e') if i == len(pts) - 1 else None
                if knoten in jvar:
                    var.append(jvar[knoten])
                    if lv[0 if i == 0 else -1]:
                        bauwerk_arme[knoten] = bauwerk_arme.get(knoten, 0) + 1
                else:
                    var.append(nvar); nvar += 1
            info.append((ch, pts, s, var))
            # Gelaende: Punkte, deren beide Nachbarelemente auf Grund liegen
            for i, p in enumerate(pts):
                if var[i] in jvar.values() and i in (0, len(pts) - 1):
                    continue                     # Kreuzungshoehe hat eine eigene Zeile
                links = lv[i - 1] if i > 0 else lv[0]                 # am Ende zaehlt das eine Nachbarelement
                rechts = lv[i] if i < len(els) else lv[-1]
                if links and rechts:                                   # mitten auf Bruecke/im Tunnel
                    g = self.gelaende(*p)
                    if g is not None:
                        # Abstand zum naechsten Bauwerksende: der geforderte Abstand zum Boden waechst mit hoechstens
                        # RAMPE (am Tunnelmund zeigt das DGM den offenen Einschnitt)
                        a_ = max((j for j in range(i) if not (lv[j - 1] if j > 0 else lv[0]) or not lv[j]),
                                 default=0)
                        b_ = min((j for j in range(i + 1, len(pts)) if not lv[j - 1] or
                                  not (lv[j] if j < len(els) else lv[-1])), default=len(pts) - 1)
                        weg = min(s[i] - s[a_], s[b_] - s[i])
                        bauwerk.append((var[i], g, links, weg))
                    continue
                h = self.gelaende(*p)
                if h is not None:
                    ds = ((s[i] - s[i - 1]) if i > 0 else 0) + ((s[i + 1] - s[i]) if i < len(els) else 0)
                    w = math.sqrt(max(ds, 1.0) / (2 * TEILEN))
                    zeile([(var[i], w)], w * h)
            # Glaette: zweite Ableitung * GLAETTEN^2
            for i in range(1, len(pts) - 1):
                d1, d2 = s[i] - s[i - 1], s[i + 1] - s[i]
                if d1 < 1e-3 or d2 < 1e-3:
                    continue
                f = GLAETTEN ** 2 * 2 / (d1 + d2) * math.sqrt((d1 + d2) / (2 * TEILEN))
                zeile([(var[i - 1], f / d1), (var[i], -f / d1 - f / d2), (var[i + 1], f / d2)], 0.0)
            # Steigung leicht bestrafen: auf langen Strecken bestimmt das Gelaende den Verlauf, kurze Stuecke
            # zwischen zwei Kreuzungen werden fast eben (sonst Spruenge zwischen benachbarten Kreuzungshoehen)
            for i in range(len(pts) - 1):
                d = max(s[i + 1] - s[i], 0.5)
                f = STEIGUNG_GEWICHT * math.sqrt(d / TEILEN) / d
                zeile([(var[i], -f), (var[i + 1], f)], 0.0)
            # sehr kurze Verbindung zwischen zwei Kreuzungen: praktisch gleiche Hoehe (beide Platten gehoeren zusammen)
            if s[-1] < 10 and var[0] in jvar.values() and var[-1] in jvar.values():
                zeile([(var[0], -20.0), (var[-1], 20.0)], 0.0)
        if ziele:                                 # Hoehenvorgaben: naechste Vorgabe im Umkreis ZIEL_RADIUS
            from scipy.spatial import cKDTree
            baum = cKDTree(np.array([(x, z) for x, z, _ in ziele]))
            gesetzt = set()
            for ch, pts, s, var in info:
                d, idx = baum.query(np.array(pts), distance_upper_bound=ZIEL_RADIUS)
                for v, di, k in zip(var, d, idx):
                    if np.isfinite(di) and v not in gesetzt:
                        gesetzt.add(v)
                        zeile([(v, ZIEL_GEWICHT)], ZIEL_GEWICHT * (ziele[k][2] - self.base))
        for k, arms in net['arms'].items():
            if bauwerk_arme.get(k, 0) * 2 > len(arms):    # Kreuzung liegt selbst auf der Bruecke/im Tunnel
                continue
            cx = sum(a['pos'][0] for a in arms) / len(arms)
            cz = sum(a['pos'][1] for a in arms) / len(arms)
            h = self.gelaende(cx, cz)
            if h is not None:
                zeile([(jvar[k], KREUZUNG_GEWICHT)], KREUZUNG_GEWICHT * h)
        if nvar == 0:
            return self
        def loesen():
            A = coo_matrix((vals, (rows, cols)), shape=(nrow, nvar)).tocsr()
            return lsqr(A, np.array(rhs), atol=1e-10, btol=1e-10, iter_lim=20000)[0] if nrow else np.zeros(nvar)
        loes = loesen()
        # Ungleichungen: Bruecke >= Boden + LICHTE_HOEHE, Tunnel <= Oberflaeche - UEBERDECKUNG; verletzte
        # Bedingungen als Zusatzzeilen, dann neu rechnen (aktive Menge)
        aktiv = set()
        for _ in range(6):
            neu = 0
            for v, g, art, weg in bauwerk:
                abstand = min(LICHTE_HOEHE if art == 'bruecke' else UEBERDECKUNG, RAMPE * weg)
                ziel = g + abstand if art == 'bruecke' else g - abstand
                verletzt = loes[v] < ziel - 0.05 if art == 'bruecke' else loes[v] > ziel + 0.05
                if verletzt and v not in aktiv:
                    aktiv.add(v)
                    zeile([(v, 3.0)], 3.0 * ziel)
                    neu += 1
            if not neu:
                break
            loes = loesen()
        if aktiv:
            self.log(f'    Gelaende: {len(aktiv)} Bruecken-/Tunnelpunkte auf Mindestabstand zum Boden gesetzt')
        for k, v in jvar.items():
            self.kreuz[k] = float(loes[v])
        enden = {}
        for ch, pts, s, var in info:
            h = [float(loes[v]) for v in var]
            els, lv = ch['els'], ch['ebene']
            ch['y'] = h[:-1]
            ch['g'] = [(h[i + 1] - h[i]) / els[i][3] * 100 if els[i][3] > 0 else 0.0 for i in range(len(els))]
            enden[pts[0]] = h[0]; enden[pts[-1]] = h[-1]
            for i, e in enumerate(els):          # Proben fuers Gelaende (nicht unter Bruecken, nicht ueber Tunneln)
                if lv[i]:
                    continue
                try:
                    innen = sdb[e[5]]['half']
                except KeyError:
                    innen = 7.0
                P = sample(e, 2.0)
                for (q, _), t in zip(P, np.linspace(0, 1, len(P))):
                    self.proben.append((q[0], q[1], h[i] + (h[i + 1] - h[i]) * t, innen))
        for ch in net.get('wenden', []) + net.get('conn_chains', []):
            self._verbinden(ch, enden)
        for j in kreuzungen:
            k = j.get('knoten')
            j['hoehe'] = self.kreuz.get(k, self.gelaende(*j['origin']) or 0.0)
            for x, z in j.get('flaeche', ()):
                self.flaechen.append((x, z, j['hoehe']))
        return self

    def _verbinden(self, ch, enden):
        """Wendeschleife/Kreuzungsspur: linear von der Hoehe am Anfang zur Hoehe am Ende"""
        els = ch['els']
        a = (els[0][0], els[0][1])
        b = end_of(els[-1])[0]

        def bei(p):
            best = min(enden.items(), key=lambda kv: math.dist(kv[0], p), default=None)
            if best and math.dist(best[0], p) < 12:
                return best[1]
            g = self.gelaende(*p)
            return 0.0 if g is None else g
        ha, hb = bei(a), bei(b)
        L = sum(e[3] for e in els) or 1.0
        y, g, acc = [], [], 0.0
        for e in els:
            y.append(ha + (hb - ha) * acc / L)
            g.append((hb - ha) / L * 100)
            acc += e[3]
        ch['y'], ch['g'] = y, g

    # ---------------------------------------------------------------- Gelaenderaster
    def _proben_feld(self):
        if getattr(self, '_feld_n', -1) != len(self.proben) + len(self.flaechen):
            P = self.proben + [(x, z, h, 2.0) for x, z, h in self.flaechen]
            self._feld = np.array(P, dtype=np.float64).reshape(-1, 4)
            self._feld_n = len(self.proben) + len(self.flaechen)
        return self._feld

    def oberflaeche(self, x, z):
        """Gelaendehoehe (relativ) an einem Punkt wie im Raster, aber ohne Tunnelgraeben: DGM, unter Strassen und
        Kreuzungen angeglichen (fuer Deckel ueber Tunnelgraeben, bauwerke.py)"""
        g = self.gelaende(x, z)
        g = 0.0 if g is None else g
        P = self._proben_feld()
        if not len(P):
            return g
        d = np.hypot(P[:, 0] - x, P[:, 1] - z) - P[:, 3]
        k = int(np.argmin(d))
        if d[k] >= UEBERGANG:
            return g
        w = max(0.0, d[k] / UEBERGANG)
        return (P[k, 2] - UNTER_FAHRBAHN) * (1 - w) + g * w

    def raster(self, x0, z0, n=61, schritt=5.0):
        """Hoehen (relativ) einer Kachel ab Welt-Ecke (x0, z0): Array [iz][ix], Zeile 0 = Sueden.
        Unter Tunneln (self.graeben, bauwerke.py) wird das Gelaende bis auf die Sohle abgesenkt, wie in den
        Standardkarten (Gladbeck); ausgeschnitten wird nichts."""
        xs = x0 + np.arange(n) * schritt
        zs = z0 + np.arange(n) * schritt
        H = np.zeros((n, n), dtype=np.float32)
        for j, z in enumerate(zs):
            for i, x in enumerate(xs):
                v = self.gelaende(x, z)
                H[j, i] = 0.0 if v is None else v
        X, Z = np.meshgrid(xs, zs)
        rand = (n - 1) * schritt + 30
        best = np.full((n, n), np.inf, dtype=np.float32)      # Abstand zur naechsten Strasse ausserhalb
        ziel = np.zeros((n, n), dtype=np.float32)
        for (x, z, h, innen) in self.proben + [(x, z, h, 2.0) for x, z, h in self.flaechen]:
            if x < x0 - 30 or z < z0 - 30 or x > x0 + rand or z > z0 + rand:
                continue
            d = np.hypot(X - x, Z - z) - innen
            m = d < np.minimum(best, UEBERGANG)
            best = np.where(m, d, best)
            ziel = np.where(m, h - UNTER_FAHRBAHN, ziel)
        w = np.clip(best / UEBERGANG, 0, 1)                      # 0 = auf der Strasse, 1 = Gelaende
        m = np.isfinite(best)
        H = np.where(m, ziel * (1 - w) + H * w, H)
        for (xa, za, xb, zb, h, innen) in getattr(self, 'graeben', ()):  # Tunnelgraben bis auf die Sohle
            if max(xa, xb) < x0 - 30 or max(za, zb) < z0 - 30 or min(xa, xb) > x0 + rand or min(za, zb) > z0 + rand:
                continue
            dx, dz = xb - xa, zb - za
            ll = dx * dx + dz * dz
            if ll < 1e-9:
                continue
            t = ((X - xa) * dx + (Z - za) * dz) / ll        # nur zwischen den Enden: am Portal endet der Graben
            quer = np.abs((X - xa) * dz - (Z - za) * dx) / math.sqrt(ll)
            H = np.where((t >= 0) & (t <= 1) & (quer < innen), np.minimum(H, h), H)
        return H
