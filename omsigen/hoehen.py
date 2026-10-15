"""Hoehen fuer die Karte: Strassenprofile mit Steigungen, Kreuzungsplatten als Ebene, Bruecken und Tunnel, Gelaenderaster.

Alle Hoehen in der Karte sind relativ zu einer Basishoehe (Meter ueber NN, Standard: tiefster Strassenpunkt - 2 m).
- Kreuzung (docs/musterbuch.md, Abschnitt 2): Platte = Ebene h0 + gx (x - cx) + gz (z - cz) um den Mittelpunkt der
  Arme. Eben, solange keine durchgehende Strasse eine Neigung braucht (Neigung kostet NEIGUNG_GEWICHT, hoechstens
  NEIGUNG_MAX - wie die gekippten Platten in Ahlheim/Staedtedreieck). Jeder Arm setzt mit der Plattenneigung in seiner
  Richtung an (ANSATZ_GEWICHT) - kein Knick an der Plattenkante.
- Strasse: Gelaende entlang der Strecke, geglaettet (zweite Ableitung klein, Massstab GLAETTEN m).
- Bruecke / Tunnel (OSM bridge / tunnel): linear zwischen den Hoehen an ihren Enden (das DGM zeigt dort den
  Boden darunter bzw. darueber, nicht die Fahrbahn).
- Spline: [spline_h] mit Steigung am Anfang und Ende und Hoehenaenderung (kubisch wie OMSI, siehe
  hoehe_im_element): die Steigung laeuft stetig ueber die Elementgrenzen (Ausrundung), an Kreuzungen genau die
  Plattenneigung. Elemente sind hoechstens TEILEN m lang.
- Gelaende: DGM im 5-m-Raster der Kacheln; unter Strassen und Kreuzungen auf Fahrbahnhoehe angeglichen (nicht unter
  Bruecken und ueber Tunneln), daneben eine Boeschung 1 : BOESCHUNG zum Gelaende (mindestens UEBERGANG breit) statt
  einer Stufe - je groesser der Hoehenunterschied, desto breiter der Uebergang (S4). Im Grundriss eines Gebaeudes
  (self.gebaeude, platzierung.Grundrisse) hebt die Boeschung das Gelaende nicht ueber DGM/Gebaeudeboden an (G1)."""
import math
import numpy as np
from .geom import end_of, sample
from .config import RANK

TEILEN = 20.0          # m: laengere Elemente werden geteilt (Hoehenverlauf folgt dem Gelaende)
GLAETTEN = 30.0        # m: Fenster des gleitenden Mittels
UNTER_FAHRBAHN = 0.05  # m: Gelaende unter der Fahrbahn so weit unter der Spline-Hoehe
UEBERGANG = 4.0        # m: Breite des Uebergangs von der Strasse zum Gelaende (mindestens)
BOESCHUNG = 1.5        # Boeschungsneigung 1 : 1,5 (wie Damm/Einschnitt im Editor): Breite = 1,5 x Hoehenunterschied
BOESCHUNG_MAX = 30.0   # m: breiter wird keine Boeschung (grosse Unterschiede: Bauwerk, kein Gelaende)
STEIGUNG_GEWICHT = 4.0  # Gewicht der Steigungsstrafe in der Ausgleichsrechnung
LICHTE_HOEHE = 5.5      # m: Brueckenfahrbahn mindestens so weit ueber dem Boden darunter
UEBERDECKUNG = 6.0      # m: Tunnelfahrbahn mindestens so weit unter der Oberflaeche
RAMPE = 0.12            # der geforderte Abstand waechst vom Bauwerksende an mit hoechstens 12 %
KREUZUNG_GEWICHT = 5.0  # Kreuzungen halten sich stark ans Gelaende
ZIEL_GEWICHT = 20.0     # Hoehenvorgaben aus dem Editor (Projekt: strasse['hoehen']) gehen vor dem Gelaende
ZIEL_RADIUS = 3.0       # m: Vorgabe gilt fuer Netzpunkte in diesem Umkreis
ANSATZ_GEWICHT = 100.0  # erstes Element eines Arms: Steigung = Plattenneigung in Armrichtung (je Einheit Steigung)
NEIGUNG_GEWICHT = 15.0  # Neigung der Kreuzungsplatte kostet (eben bevorzugt; durchgehendes Gefaelle kippt sie)
NEIGUNG_MAX = 0.10      # steiler wird keine Platte (Ahlheim: bis 8,7 %)
QUER_AUSLAUF = 15.0        # m: so lang laeuft die Querneigung der Platte im Arm auf 0 aus
AUSRUNDUNG_STRECKE = 40.0  # m: so weit von der Platte wird die Steigung staerker geglaettet ...
AUSRUNDUNG_FAKTOR = 5.0    # ... an der Platte um diesen Faktor
QUER_GEWICHT = 100.0     # Querneigung der Hauptrichtung (Arme der hoechsten Strassenklasse) kostet viel: die Hauptstrasse
                        # behaelt ihr Profil, die Nebenstrasse passt sich an (Ahlheim: Nebenarm mit 8 % Quergefaelle)


def hoehe_im_element(y, g0, g1, dh, L, t):
    """Hoehe nach t Metern in einem Element: Steigung g0 -> g1 (%) und Hoehenaenderung dh - kubisch wie OMSI
    [spline_h] (openOMSI omsi-geometry height_at); dh None: [spline] (Steigung linear)"""
    L = max(L, 1e-6)
    u = min(max(t / L, 0.0), 1.0)
    if dh is None:
        return y + t * (g0 + (g1 - g0) * u * 0.5) / 100.0
    m0, m1 = g0 / 100.0 * L, g1 / 100.0 * L
    return y + (u ** 3 - 2 * u * u + u) * m0 + (3 * u * u - 2 * u ** 3) * dh + (u ** 3 - u * u) * m1


def scheitel_steigungen(s, h, ansatz):
    """Steigung (%) an jedem Punkt einer Kette (C1: Ableitung der Parabel durch die Nachbarpunkte); ansatz[i] (oder
    None) gibt sie vor (Kreuzung: Plattenneigung in Kettenrichtung)"""
    n = len(h)
    m = [(h[i + 1] - h[i]) / (s[i + 1] - s[i]) * 100 if s[i + 1] > s[i] else 0.0 for i in range(n - 1)]
    G = []
    for i in range(n):
        if ansatz[i] is not None:
            G.append(ansatz[i])
        elif i == 0:
            G.append(m[0] if m else 0.0)
        elif i == n - 1:
            G.append(m[-1])
        else:
            d1, d2 = s[i] - s[i - 1], s[i + 1] - s[i]
            G.append((m[i - 1] * d2 + m[i] * d1) / (d1 + d2) if d1 + d2 > 0 else 0.0)
    return G


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


def boeschung_breite(dh):
    """Breite des Uebergangs Strasse -> Gelaende bei Hoehenunterschied dh (m): Boeschung 1 : BOESCHUNG, mindestens
    UEBERGANG, hoechstens BOESCHUNG_MAX"""
    return np.clip(np.abs(dh) * BOESCHUNG, UEBERGANG, BOESCHUNG_MAX)


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
        self.kreuz = {}            # Knoten -> Hoehe (relativ) im Mittelpunkt der Arme
        self.ebenen = {}           # Knoten -> (cx, cz, h0, gx, gz): Platte als Ebene
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
        from scipy.sparse import identity
        from scipy.sparse.linalg import spsolve
        chains = net['road_chains']
        for ch in chains:
            ch['els'], ch['ebene'] = zerlegen(ch['els'], ch.get('ebene') or [None] * len(ch['els']))
        werte = [self.gel.hoehe(e[0], e[1]) for ch in chains for e in ch['els']] if self.gel else []
        werte = [h for h in werte if h is not None] + [h for _, _, h in ziele]
        self.base = math.floor(min(werte)) - 2.0 if werte and self.gel else 0.0

        # Unbekannte: je Kreuzung Hoehe h0 im Mittelpunkt der Arme und Neigung gx, gz (Platte = Ebene), je
        # Kettenpunkt (ausser an Kreuzungen) eine Hoehe. Ein Punkt ist eine Summe [(Spalte, Faktor)].
        nvar = 0
        jvar, jgx, jgz, jmitte = {}, {}, {}, {}
        for k, arms in net['arms'].items():
            jvar[k], jgx[k], jgz[k] = nvar, nvar + 1, nvar + 2
            nvar += 3
            jmitte[k] = (sum(a['pos'][0] for a in arms) / len(arms), sum(a['pos'][1] for a in arms) / len(arms))
        rows, cols, vals, rhs = [], [], [], []
        nrow = 0

        def zeile(eintraege, ziel):
            nonlocal nrow
            for c, v in eintraege:
                rows.append(nrow); cols.append(c); vals.append(v)
            rhs.append(ziel); nrow += 1

        def mal(T, f):
            return [(c, v * f) for c, v in T]

        def an_platte(k, p):
            cx, cz = jmitte[k]
            return [(jvar[k], 1.0), (jgx[k], p[0] - cx), (jgz[k], p[1] - cz)]

        def richtung(p, q):
            d = math.dist(p, q)
            return ((q[0] - p[0]) / d, (q[1] - p[1]) / d) if d > 1e-6 else (0.0, 0.0)
        bauwerk_arme = {}                        # Kreuzung -> Anzahl Arme auf Bruecke/im Tunnel
        bauwerk = []                             # (Punkt, Boden, 'bruecke'/'tunnel', Weg) fuer die Ungleichungen
        info = []
        for ch in chains:
            els, lv = ch['els'], ch['ebene']
            pts = [(e[0], e[1]) for e in els] + [end_of(els[-1])[0]]
            s = [0.0]
            for e in els:
                s.append(s[-1] + e[3])
            var, knoten_an = [], []
            for i in range(len(pts)):
                knoten = ch.get('node_s') if i == 0 else ch.get('node_e') if i == len(pts) - 1 else None
                if knoten in jvar:
                    var.append(an_platte(knoten, pts[i]))
                    knoten_an.append(knoten)
                    if lv[0 if i == 0 else -1]:
                        bauwerk_arme[knoten] = bauwerk_arme.get(knoten, 0) + 1
                else:
                    var.append([(nvar, 1.0)]); nvar += 1
                    knoten_an.append(None)
            info.append((ch, pts, s, var, knoten_an))
            # Gelaende: Punkte, deren beide Nachbarelemente auf Grund liegen
            for i, p in enumerate(pts):
                if knoten_an[i] is not None:
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
                    zeile(mal(var[i], w), w * h)
            # Glaette: zweite Ableitung * GLAETTEN^2
            for i in range(1, len(pts) - 1):
                d1, d2 = s[i] - s[i - 1], s[i + 1] - s[i]
                if d1 < 1e-3 or d2 < 1e-3:
                    continue
                f = GLAETTEN ** 2 * 2 / (d1 + d2) * math.sqrt((d1 + d2) / (2 * TEILEN))
                # nahe einer Kreuzung staerker: lange Ausrundung von der Platte in die Hangsteigung (Vorbilder 20-55 m)
                ab = min(s[i] if knoten_an[0] is not None else 1e9, s[-1] - s[i] if knoten_an[-1] is not None else 1e9)
                f *= 1.0 + (AUSRUNDUNG_FAKTOR - 1.0) * max(0.0, 1.0 - ab / AUSRUNDUNG_STRECKE)
                zeile(mal(var[i - 1], f / d1) + mal(var[i], -f / d1 - f / d2) + mal(var[i + 1], f / d2), 0.0)
            # Steigung leicht bestrafen: auf langen Strecken bestimmt das Gelaende den Verlauf, kurze Stuecke
            # zwischen zwei Kreuzungen werden fast eben (sonst Spruenge zwischen benachbarten Kreuzungshoehen)
            for i in range(len(pts) - 1):
                d = max(s[i + 1] - s[i], 0.5)
                f = STEIGUNG_GEWICHT * math.sqrt(d / TEILEN) / d
                zeile(mal(var[i], -f) + mal(var[i + 1], f), 0.0)
            # Ansatz an der Platte: Steigung des ersten Elements = Neigung der Platte in Armrichtung (kein Knick)
            for i_e, i_n in ((0, 1), (len(pts) - 1, len(pts) - 2)):
                k = knoten_an[i_e]
                if k is None or len(pts) < 2:
                    continue
                d = s[i_n] - s[i_e] if i_n > i_e else s[i_e] - s[i_n]
                if d < 1e-3:
                    continue
                u = richtung(pts[i_e], pts[i_n])
                f = ANSATZ_GEWICHT / d
                zeile(mal(var[i_n], f) + mal(var[i_e], -f) + [(jgx[k], -ANSATZ_GEWICHT * u[0]),
                                                                 (jgz[k], -ANSATZ_GEWICHT * u[1])], 0.0)
            # sehr kurze Verbindung zwischen zwei Kreuzungen: praktisch gleiche Hoehe (beide Platten gehoeren zusammen)
            if s[-1] < 10 and knoten_an[0] is not None and knoten_an[-1] is not None:
                zeile(mal(var[0], -20.0) + mal(var[-1], 20.0), 0.0)
        if ziele:                                 # Hoehenvorgaben: naechste Vorgabe im Umkreis ZIEL_RADIUS
            from scipy.spatial import cKDTree
            baum = cKDTree(np.array([(x, z) for x, z, _ in ziele]))
            gesetzt = set()
            for ch, pts, s, var, kn in info:
                d, idx = baum.query(np.array(pts), distance_upper_bound=ZIEL_RADIUS)
                for T, di, k in zip(var, d, idx):
                    if np.isfinite(di) and T[0][0] not in gesetzt:
                        gesetzt.add(T[0][0])
                        zeile(mal(T, ZIEL_GEWICHT), ZIEL_GEWICHT * (ziele[k][2] - self.base))
        for k, arms in net['arms'].items():
            zeile([(jgx[k], NEIGUNG_GEWICHT)], 0.0)            # eben bevorzugt
            zeile([(jgz[k], NEIGUNG_GEWICHT)], 0.0)
            rang = [RANK.get((a.get('tags') or {}).get('highway'), 0) for a in arms]
            ue = k in net.get('uebergang_knoten', {})               # Uebergangsobjekt: nur laengs neigbar
            for a, r in zip(arms, rang):                         # Querneigung der Hauptarme klein halten
                if (r == max(rang) or ue) and 'h' in a:
                    hr = math.radians(a['h'])
                    f = 20 * QUER_GEWICHT if ue else QUER_GEWICHT
                    zeile([(jgx[k], -f * math.cos(hr)), (jgz[k], f * math.sin(hr))], 0.0)
            if bauwerk_arme.get(k, 0) * 2 > len(arms):    # Kreuzung liegt selbst auf der Bruecke/im Tunnel
                continue
            h = self.gelaende(*jmitte[k])
            if h is not None:
                zeile([(jvar[k], KREUZUNG_GEWICHT)], KREUZUNG_GEWICHT * h)
        if nvar == 0:
            return self

        def loesen():
            # exakt ueber die Normalgleichungen (duenn, symmetrisch positiv definit; winzige Regularisierung fuer
            # Teilnetze ohne Gelaendewert). lsqr brach bei der ganzen Linie 16 (2400 Unbekannte) nach 20000 Schritten
            # ungeloest ab: Sackgassen am Korridorrand lagen bis 17 m unter dem Gelaende, Haeuser daneben schwebten (G1)
            if not nrow:
                return np.zeros(nvar)
            A = coo_matrix((vals, (rows, cols)), shape=(nrow, nvar)).tocsr()
            N = (A.T @ A).tocsc() + identity(nvar, format='csc') * 1e-9
            return spsolve(N, A.T @ np.array(rhs))

        def wert(T):
            return float(sum(loes[c] * v for c, v in T))
        loes = loesen()
        # Ungleichungen: Bruecke >= Boden + LICHTE_HOEHE, Tunnel <= Oberflaeche - UEBERDECKUNG, Plattenneigung
        # <= NEIGUNG_MAX; verletzte Bedingungen als Zusatzzeilen, dann neu rechnen (aktive Menge)
        aktiv, gekappt = set(), set()
        for _ in range(6):
            neu = 0
            for n_, (T, g, art, weg) in enumerate(bauwerk):
                abstand = min(LICHTE_HOEHE if art == 'bruecke' else UEBERDECKUNG, RAMPE * weg)
                ziel = g + abstand if art == 'bruecke' else g - abstand
                v = wert(T)
                verletzt = v < ziel - 0.05 if art == 'bruecke' else v > ziel + 0.05
                if verletzt and n_ not in aktiv:
                    aktiv.add(n_)
                    zeile(mal(T, 3.0), 3.0 * ziel)
                    neu += 1
            for k in jvar:
                gx, gz = loes[jgx[k]], loes[jgz[k]]
                n_ = math.hypot(gx, gz)
                if n_ > NEIGUNG_MAX + 1e-3 and k not in gekappt:
                    gekappt.add(k)
                    f = NEIGUNG_MAX / n_
                    zeile([(jgx[k], 200.0)], 200.0 * gx * f)
                    zeile([(jgz[k], 200.0)], 200.0 * gz * f)
                    neu += 1
            if not neu:
                break
            loes = loesen()
        if aktiv:
            self.log(f'    Gelaende: {len(aktiv)} Bruecken-/Tunnelpunkte auf Mindestabstand zum Boden gesetzt')
        for k, v in jvar.items():
            cx, cz = jmitte[k]
            self.kreuz[k] = float(loes[v])
            self.ebenen[k] = (cx, cz, float(loes[v]), float(loes[jgx[k]]), float(loes[jgz[k]]))
        geneigt = sum(1 for e in self.ebenen.values() if math.hypot(e[3], e[4]) >= 0.01)
        if geneigt:
            self.log(f'    Kreuzungen: {geneigt} von {len(self.ebenen)} Platten folgen dem Gefaelle (Neigung >= 1 %, '
                     f'hoechstens {max(math.hypot(e[3], e[4]) for e in self.ebenen.values()) * 100:.1f} %)')
        enden = {}
        for ch, pts, s, var, knoten_an in info:
            h = [wert(T) for T in var]
            els, lv = ch['els'], ch['ebene']
            ch['y'] = h[:-1]
            ch['g'] = [(h[i + 1] - h[i]) / els[i][3] * 100 if els[i][3] > 0 else 0.0 for i in range(len(els))]
            # Steigung an jedem Punkt: an Kreuzungen die Plattenneigung in Kettenrichtung, sonst stetig (C1)
            ansatz = [None] * len(pts)
            for i_e, i_n in ((0, 1), (len(pts) - 1, len(pts) - 2)):
                k = knoten_an[i_e]
                if k is None:
                    continue
                u = richtung(pts[i_n], pts[i_e]) if i_e else richtung(pts[i_e], pts[i_n])     # in Kettenrichtung
                _, _, _, gx, gz = self.ebenen[k]
                ansatz[i_e] = (gx * u[0] + gz * u[1]) * 100
            G = scheitel_steigungen(s, h, ansatz)
            ch['g01'] = [(G[i], G[i + 1]) for i in range(len(els))]
            ch['dh'] = [h[i + 1] - h[i] for i in range(len(els))]
            # Querneigung: am Arm genau die der Platte quer zur Fahrtrichtung (OMSI: Prozent, dz = -x * cant / 100,
            # x rechts), ueber QUER_AUSLAUF m auf 0 - sonst klafft an der Plattenkante ein Keil (helle Linie)
            q = [0.0] * len(pts)
            for i_e in (0, len(pts) - 1):
                k = knoten_an[i_e]
                if k is None or k in net.get('uebergang_knoten', {}):
                    continue
                hd = els[0][2] if i_e == 0 else end_of(els[-1])[1]
                _, _, _, gx, gz = self.ebenen[k]
                rr = math.radians(hd)
                c = -100.0 * (gx * math.cos(rr) - gz * math.sin(rr))
                lauf = min(QUER_AUSLAUF, s[-1]) or 1.0           # kurze Stuecke: am anderen Ende schon 0
                for i in range(len(pts)):
                    ab = s[i] if i_e == 0 else s[-1] - s[i]
                    q[i] += c * max(0.0, 1.0 - ab / lauf)
            ch['quer'] = [(q[i], q[i + 1]) for i in range(len(els))]
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
                    self.proben.append((q[0], q[1], hoehe_im_element(h[i], G[i], G[i + 1], h[i + 1] - h[i], e[3],
                                                                     t * e[3]), innen))
        for ch in net.get('wenden', []) + net.get('conn_chains', []):
            self._verbinden(ch, enden)
        for j in kreuzungen:
            k = j.get('knoten')
            if k in self.ebenen:                 # Platte als Ebene: Hoehe am Objektursprung, Neigung je m nach Ost/Nord
                cx, cz, h0, gx, gz = self.ebenen[k]
                ox, oz = j['origin']
                j['hoehe'] = h0 + gx * (ox - cx) + gz * (oz - cz)
                j['neigung'] = (gx, gz)
                for x, z in j.get('flaeche', ()):
                    self.flaechen.append((x, z, h0 + gx * (x - cx) + gz * (z - cz)))
            else:
                j['hoehe'] = self.gelaende(*j['origin']) or 0.0
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
        ziel = P[:, 2] - UNTER_FAHRBAHN
        r = np.maximum(d, 0.0) / boeschung_breite(ziel - g)
        k = int(np.argmin(r))
        if r[k] >= 1.0:
            return g
        return ziel[k] * (1 - r[k]) + g * r[k]

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
        rand = BOESCHUNG_MAX + 10                               # Strassen so weit ausserhalb wirken noch
        aussen = (n - 1) * schritt + rand
        H0 = H.copy()
        best = np.full((n, n), np.inf, dtype=np.float32)      # Lage in der Boeschung der massgebenden Strasse
        deckel = np.full((n, n), np.inf, dtype=np.float32)    # hoechstens so hoch: Punkte, deren Dreiecke die Strasse
                                                              # beruehren koennen (innerhalb einer Zellendiagonale)
        diag = schritt * math.sqrt(2.0)
        d_deckel = np.full((n, n), np.inf, dtype=np.float32)  # Abstand zum Strassenpunkt, der die Obergrenze gibt
        ziel = np.zeros((n, n), dtype=np.float32)              # (0 = auf der Strasse, 1 = Gelaende)
        for (x, z, h, innen) in self.proben + [(x, z, h, 2.0) for x, z, h in self.flaechen]:
            if x < x0 - rand or z < z0 - rand or x > x0 + aussen or z > z0 + aussen:
                continue
            d = np.hypot(X - x, Z - z) - innen
            nah = (d < diag) & (d < d_deckel)                   # naechster Strassenpunkt gibt die Hoehe vor
            deckel = np.where(nah, h - UNTER_FAHRBAHN, deckel)
            d_deckel = np.where(nah, d, d_deckel)
            r = np.maximum(d, 0.0) / boeschung_breite(h - UNTER_FAHRBAHN - H0)
            m = r < np.minimum(best, 1.0)
            best = np.where(m, r, best)
            ziel = np.where(m, h - UNTER_FAHRBAHN, ziel)
        w = np.clip(best, 0, 1)
        m = np.isfinite(best)
        H = np.where(m, ziel * (1 - w) + H * w, H)
        # OMSI legt das Gelaende als Dreiecke zwischen den 5-m-Punkten: ein hoeherer Punkt neben dem Bordstein hebt das
        # Dreieck quer ueber Gehweg und Fahrbahn (Bodentextur auf der Strasse). Deshalb liegt jeder Punkt in Reichweite
        # einer Zellendiagonale hoechstens auf Strassenhoehe; die Boeschung beginnt erst dahinter.
        H = np.minimum(H, deckel)
        if getattr(self, 'gebaeude', None) is not None:          # G1: an Gebaeuden nicht ueber DGM/Boden anheben
            H = self.gebaeude.klemmen(X, Z, H, H0, ~(best <= 1e-9))
        for (xa, za, xb, zb, h, innen) in getattr(self, 'graeben', ()):  # Tunnelgraben bis auf die Sohle
            if max(xa, xb) < x0 - 30 or max(za, zb) < z0 - 30 or min(xa, xb) > x0 + aussen or min(za, zb) > z0 + aussen:
                continue
            dx, dz = xb - xa, zb - za
            ll = dx * dx + dz * dz
            if ll < 1e-9:
                continue
            t = ((X - xa) * dx + (Z - za) * dz) / ll        # nur zwischen den Enden: am Portal endet der Graben
            quer = np.abs((X - xa) * dz - (Z - za) * dx) / math.sqrt(ll)
            H = np.where((t >= 0) & (t <= 1) & (quer < innen), np.minimum(H, h), H)
        return H
