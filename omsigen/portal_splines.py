"""Lernstueck Suedportal, Fassung 3: Messdaten + OMSI-Bausteine. Statt eines grossen Modells (Fassung 2) entsteht das
Bauwerk wie in den Fremdkarten (Gladbeck, Staedtedreieck) aus Splines - deren Lage und Hoehe aber gemessen sind:

1. Messung wie Fassung 2: Achse aus den Trogwaenden, Kanten (Waende) als glatte Kurven t(s), Streifen dazwischen mit
   Hoehe/Quergefaelle aus den Bodenpunkten, Fahrbahn-Streifen aus dem Materialraster (DOP20).
2. Fahrbahnen: je Fahrbahn-Streifen ein Strassen-Spline auf der Streifenmitte, eigene .sli mit der gemessenen Breite;
   Fahrstreifen = Abschnitte zwischen durchgehenden Markierungen (>= 2,5 m), Richtung: Rechtsverkehr bzw. Einbahn
   (Bauaufnahme: Ostrampe nach Norden, Westrampe nach Sueden).
3. Markierungen: helle Laengslinien im DOP20 je Station, zu Linien verbunden -> je Linie ein Spline (0,12 m), durchgehend
   oder Strich (3 m / 6 m) je nach Anteil der Stationen mit Fund.
4. Waende: je Kante ein Wand-Spline entlang der gemessenen Linie, Krone = Hoehe der hohen Seite; Trogwaende rote Paneele
   mit Betonbruestung 0,8 m und Gelaender (Fotos), sonst Beton, Gelaender ab 2 m Hoehe.
5. Uebrige Streifen (oben neben dem Trog): Boden-Splines (Gras/Gehweg nach DOP20) auf der gemessenen Hoehe.
6. Gelaende: Kartengelaende (5-m-Raster) unter alle Flaechen abgesenkt (Minimum im Umkreis einer Rasterzelle).

Quellen: Laserdaten, DOP20 Bayerische Vermessungsverwaltung (CC BY 4.0)."""
import argparse
import json
import math
import os
import shutil

import numpy as np

from . import portal_modell2 as M
from .portal_modell import lese_punkte, wandlinien, dop_bild, materialraster
from .ring_waende import kette

ORDNER = 'Lernstueck'                     # Splines\Aschaffenburg\Lernstueck
# Texturen: Datei, Meter je Wiederholung (u quer bzw. Hoehe, v laengs)
TEX = {'asphalt': ('str_asphdrk.bmp', 4.0), 'beton': ('betonwand1.bmp', 4.0), 'paneel': ('AB_paneel_rot.bmp', 2.5),
       'stab': ('AB_stabgelaender.tga', 1.0), 'weiss': ('AB_weiss.bmp', 1.0), 'strich': ('AB_LS_strich.tga', 9.0),
       'gras': ('AB_boden.bmp', 4.0), 'gehweg': ('str_gehweg02.bmp', 2.0)}
ALPHA = {'stab', 'strich'}
RICHTUNG = {'ost': -1, 'west': +1}        # Einbahn-Rampen: +1 = in Achsrichtung (Nord -> Sued)
BRUESTUNG, GELAENDER = 0.8, 1.0
LINIE = 0.12


class Sli:
    """eine .sli-Datei: Profilstrecken (x rechts der Laufrichtung, y hoch), Hoehenprofile, Pfade"""

    def __init__(self, kopf):
        self.kopf, self.teile, self.hoehen, self.pfade = kopf, [], [], []

    def strecke(self, tex, a, b, u0=0.0):
        """sichtbar auf der linken Seite von a -> b (Normale (-dy, dx))"""
        g = TEX[tex][1]
        L = math.hypot(b[0] - a[0], b[1] - a[1])
        self.teile.append((tex, [(a[0], a[1], u0 / g), (b[0], b[1], (u0 + L) / g)]))

    def senkrecht(self, tex, x, y0, y1, sicht):
        """senkrechte Flaeche bei x, sichtbar zur Seite sicht (-1 links, +1 rechts)"""
        self.strecke(tex, (x, y0), (x, y1)) if sicht < 0 else self.strecke(tex, (x, y1), (x, y0))

    def flach(self, tex, x0, x1, y, u_abs=False):
        a, b = min(x0, x1), max(x0, x1)
        self.strecke(tex, (a, y), (b, y), u0=a if u_abs else 0.0)

    def text(self):
        tx = sorted({t for t, _ in self.teile}, key=list(TEX).index)
        L = [self.kopf, '']
        for x0, x1, y in self.hoehen:
            L += ['[heightprofile]', f'{x0:.3f}', f'{x1:.3f}', f'{y:.3f}', f'{y:.3f}', '']
        for t in tx:
            L += ['[texture]', TEX[t][0], '']
            if t in ALPHA:
                L += ['[matl_alpha]', '1', '']
        for t, pts in self.teile:
            L += ['[profile]', str(tx.index(t)), '']
            for x, y, u in pts:
                L += ['[profilepnt]', f'{x:.3f}', f'{y:.3f}', f'{u:.3f}', f'{1.0 / TEX[t][1]:.4f}', '']
        for art, x, y, b, r in self.pfade:
            L += ['[path]', str(art), f'{x:.3f}', f'{y:.3f}', f'{b:.3f}', str(r), '']
        return '\r\n'.join(L) + '\r\n'


# ------------------------------------------------------------------ Messung
def messung(P, bereich, dop):
    B = P[P[:, 3] == 2]
    L = wandlinien(B, bereich)
    ax = M.achse_aus(L)
    K = M.kanten_st(L, ax)
    ns = len(ax.s)
    grenzen = [np.full(ns, -M.T_OST)] + [k['t'] for k in K] + [np.full(ns, M.T_WEST)]
    mr = materialraster(P, bereich, dop, L)
    x0, y0, x1, y1 = bereich
    H, W = mr.shape

    def material_quer(j, k_alle, schritt=0.25):
        """Materialprofil quer ueber den Streifen: je 0,25 m (ab linkem Rand) Anteil Asphalt ueber die Stationen"""
        breite = float(np.median(grenzen[j + 1] - grenzen[j]))
        dt = np.arange(schritt / 2, breite, schritt)
        n, n_a = np.zeros(len(dt)), np.zeros(len(dt))
        for k in k_alle:
            for i, d in enumerate(dt):
                p_ = ax.welt(k, grenzen[j][k] + d * (grenzen[j + 1][k] - grenzen[j][k]) / breite)
                px, py = int((p_[0] - x0) * 5), int((y1 - p_[1]) * 5)
                if 0 <= px < W and 0 <= py < H:
                    n[i] += 1
                    n_a[i] += mr[py, px] == 1
        return dt, np.where(n > 0, n_a / np.maximum(n, 1), 0.0)
    nb = len(grenzen) - 1
    quer = [material_quer(j, range(0, ns, 2)) for j in range(nb)]
    # Fahrbahn: mindestens 5 m Asphalt quer (ein Weg mit Gruenstreifen daneben ist keine Fahrbahn)
    fahrbahn = [float((q[1] > 0.5).sum()) * 0.25 >= 5.0 for q in quer]
    a, b = M.streifen_hoehen(B, ax, grenzen, fahrbahn)
    # Splines haben keine Querneigung: jede Flaeche eben quer, Hoehe der Streifenmitte - so liegen Markierungen,
    # Schraffur und Wandkronen genau auf dem, was gebaut wird
    for j in range(nb):
        tc = M.glatt((grenzen[j] + grenzen[j + 1]) / 2, 9)
        a[:, j] = a[:, j] + b[:, j] * tc
        b[:, j] = 0.0
    return dict(L=L, ax=ax, K=K, grenzen=grenzen, mr=mr, fahrbahn=fahrbahn, quer=quer, a=a, b=b)


def z_bei(m, k, t):
    """Oberflaeche an Station k, Querweg t (Streifen, in dem t liegt)"""
    g = m['grenzen']
    j = sum(1 for q in g[1:-1] if q[k] < t)
    if j in m.get('gebaut', {}):
        return float(m['gebaut'][j][k])
    return m['a'][k, j] + m['b'][k, j] * t


def kette_hoehe(els, s):
    """Hoehe einer Spline-Kette an der Bogenlaenge s (kubisch mit den Steigungen am Anfang/Ende jedes Elements)"""
    for e in els:
        if s <= e['kum'] + e['laenge'] or e is els[-1]:
            L = e['laenge']
            u = min(max((s - e['kum']) / L, 0.0), 1.0)
            m0, m1 = e['stg_a'] / 100 * L, e['stg_e'] / 100 * L
            h00, h10, h01, h11 = 2 * u**3 - 3 * u**2 + 1, u**3 - 2 * u**2 + u, -2 * u**3 + 3 * u**2, u**3 - u**2
            return e['z'] * h00 + m0 * h10 + (e['z'] + e['dh']) * h01 + m1 * h11
    return 0.0


def linie_xy(ax, ks, T):
    return np.array([ax.welt(k, t) for k, t in zip(ks, T)])


# ------------------------------------------------------------------ Markierungen
def markierungen(m, dop, bereich):
    """helle Laengslinien auf Asphalt -> [(Stationen, t(s) geglaettet, durchgehend?)]"""
    ax, mr = m['ax'], m['mr']
    x0, y0, x1, y1 = bereich
    H, W = dop.shape[:2]
    v = dop.mean(axis=2)
    sat = dop.max(axis=2) - dop.min(axis=2)
    ns = len(ax.s)
    funde = []
    for k in range(ns):
        ts = np.arange(m.get('t0', -M.T_OST) + 1, m.get('t1', M.T_WEST) - 1, 0.1)
        Q = ax.p[k][None, :] + ts[:, None] * ax.n[k][None, :]
        px = np.clip(((Q[:, 0] - x0) * 5).astype(int), 0, W - 1)
        py = np.clip(((y1 - Q[:, 1]) * 5).astype(int), 0, H - 1)
        vv, ss, mm = v[py, px], sat[py, px], mr[py, px]
        ref = np.median(vv[mm == 1]) if (mm == 1).any() else 120
        hell = (vv > ref + 45) & (ss < 40) & (mm == 1)
        t_l, lauf = [], []
        for t_, h_ in zip(ts, hell):
            if h_:
                lauf.append(t_)
            elif lauf:
                if len(lauf) <= 5:
                    t_l.append(float(np.mean(lauf)))
                lauf = []
        funde.append(t_l)
    linien, offen = [], []
    for k, ts in enumerate(funde):
        frei, neu = list(ts), []
        for l in offen:
            kl, tl = l[-1]
            best = min(frei, key=lambda t: abs(t - tl), default=None)
            if best is not None and abs(best - tl) < 0.5 and k - kl <= 8:
                l.append((k, best))
                frei.remove(best)
                neu.append(l)
            elif k - kl <= 8:
                neu.append(l)
            else:
                linien.append(l)
        neu += [[(k, t)] for t in frei]
        offen = neu
    linien += offen
    out = []
    for l in linien:
        K_ = np.array([p[0] for p in l])
        if len(l) < 6 or K_[-1] - K_[0] < 8:
            continue
        ks = np.arange(K_[0], K_[-1] + 1)
        T = M.glatt(np.interp(ks, K_, [p[1] for p in l]), 21)                 # Linien laufen parallel: stark glaetten
        dicht = len(set(K_.tolist())) / len(ks)
        out.append((ks, T, dicht > 0.75))
    return out


def hell_maske(dop):
    v = dop.mean(axis=2)
    sat = dop.max(axis=2) - dop.min(axis=2)
    return v, sat


def sperrflaechen(m, dop, bereich, min_breite=1.0):
    """schraffierte Flaechen am Fahrbahnrand: Anteil heller Pixel in 1-m-Zellen ueber 5 Stationen zwischen 20 und 75 %
    (Schraffur; Linie oder Asphalt liegen darunter, Beton darueber), vom Rand nach innen zusammenhaengend
    -> [(Streifen j, Rand -1 links/+1 rechts, Stationen, t der Grenzlinie)]"""
    ax, g, mr = m['ax'], m['grenzen'], m['mr']
    x0, y0, x1, y1 = bereich
    H, W = dop.shape[:2]
    v, sat = hell_maske(dop)
    ns = len(ax.s)
    out = []
    for j, fb in enumerate(m['fahrbahn']):
        if not fb:
            continue
        breite = float(np.median(g[j + 1] - g[j]))
        dt = np.arange(0.05, breite, 0.1)
        hell = np.zeros((ns, len(dt)))
        for k in range(ns):
            Q = np.array([ax.welt(k, g[j][k] + d * (g[j + 1][k] - g[j][k]) / breite) for d in dt])
            px = np.clip(((Q[:, 0] - x0) * 5).astype(int), 0, W - 1)
            py = np.clip(((y1 - Q[:, 1]) * 5).astype(int), 0, H - 1)
            vv, ss, mm = v[py, px], sat[py, px], mr[py, px]
            ref = np.median(vv[mm == 1]) if (mm == 1).any() else 120
            hell[k] = (vv > ref + 30) & (ss < 40)
        # Anteil je Station und 1-m-Fenster
        # Schraffur wechselt auch laengs (schraege Streifen): an festem t hell/dunkel ueber die Stationen; eine helle
        # Rinne oder Bordkante ist laengs gleichmaessig
        zelle, wechsel = np.zeros_like(hell), np.zeros_like(hell)
        for k in range(ns):
            blk = hell[max(0, k - 2):k + 3]
            zelle[k] = np.convolve(blk.mean(axis=0), np.ones(10) / 10, mode='same')
            wechsel[k] = np.convolve(blk.std(axis=0), np.ones(10) / 10, mode='same')
        schraff = (zelle > 0.2) & (zelle < 0.75) & (wechsel > 0.3)
        for rand in (-1, 1):
            w = np.zeros(ns)
            for k in range(ns):
                reihe = schraff[k] if rand < 0 else schraff[k][::-1]
                # vom Rand (erste 2,5 m duerfen fehlen: Rinne, Bord, Schatten der Wand) nach innen
                i = 0
                while i < 25 and not reihe[i]:
                    i += 1
                n = i
                while n < len(reihe) and reihe[n]:
                    n += 1
                w[k] = n * 0.1 if n > i else 0.0
            ja = w >= min_breite
            # Laeufe von mindestens 8 Stationen
            k = 0
            while k < ns:
                if not ja[k]:
                    k += 1
                    continue
                e = k
                while e < ns and ja[e]:
                    e += 1
                if e - k >= 8:
                    ks = np.arange(k, e)
                    ww = M.glatt(w[ks], 15)
                    T = np.array([g[j][q] + ww[n] if rand < 0 else g[j + 1][q] - ww[n] for n, q in enumerate(ks)])
                    out.append((j, rand, ks, T))
                k = e
    return out


def linien_bereinigen(m, linien, sperr):
    """Funde an den Waenden (Bruestungskanten) und in Sperrflaechen (Schraffur) weg; Grenzlinien der Sperrflaechen dazu"""
    g = m['grenzen']
    raus = []
    for ks, T, durch in linien:
        an_wand = min(np.median(np.abs(T - q[ks])) for q in g[1:-1]) < 0.7
        drin = 0
        for j, rand, sk, ST in sperr:
            for n, k in enumerate(ks):
                i = np.searchsorted(sk, k)
                if i < len(sk) and sk[i] == k and ((rand < 0 and T[n] < ST[i] + 0.4) or (rand > 0 and T[n] > ST[i] - 0.4)) \
                        and g[j][k] < T[n] < g[j + 1][k]:
                    drin += 1
        if not an_wand and drin < 0.3 * len(ks):
            raus.append((ks, T, durch))
    return raus + [(sk, ST, True) for _, _, sk, ST in sperr]


def sperr_objekt(m, sperr):
    """Schraffur als Auflage-Objekt (Breite aendert sich - kein Spline): kleine Dreiecke, UV in Weltkoordinaten"""
    from .sonderbau_ring import Mesh
    ax, g = m['ax'], m['grenzen']
    o = np.array([round(float(ax.p[:, 0].mean()), 1), round(float(ax.p[:, 1].mean()), 1)])
    me = Mesh()
    for j, rand, ks, T in sperr:
        rand_t = g[j] if rand < 0 else g[j + 1]
        for n in range(len(ks) - 1):
            k0, k1 = ks[n], ks[n + 1]
            ta0, tb0 = sorted((rand_t[k0], T[n]))
            ta1, tb1 = sorted((rand_t[k1], T[n + 1]))
            q = max(1, int(math.ceil(max(tb0 - ta0, tb1 - ta1))))
            for i in range(q):
                f0, f1 = i / q, (i + 1) / q
                ts = [(k0, ta0 + (tb0 - ta0) * f0), (k0, ta0 + (tb0 - ta0) * f1), (k1, ta1 + (tb1 - ta1) * f1), (k1, ta1 + (tb1 - ta1) * f0)]
                pts = []
                for k, t in ts:
                    p = ax.welt(k, t)
                    pts.append((p[0] - o[0], p[1] - o[1], float(m['gebaut'][j][k]) + 0.10))   # Objekt ueber Spline: 5 cm reichen dem Renderer nicht
                uv = [(p[0] / 3.0, p[1] / 3.0) for p in pts]
                me.dreieck('sperr', [pts[0], pts[1], pts[2]], [uv[0], uv[1], uv[2]], (0, 0, 1))
                me.dreieck('sperr', [pts[0], pts[2], pts[3]], [uv[0], uv[2], uv[3]], (0, 0, 1))
    return me, o


# ------------------------------------------------------------------ Bauteile
def fahrbahnen(m, linien, seiten, sperr=()):
    """je Fahrbahn-Streifen: Spline auf der Mitte, .sli mit gemessener Breite, Fahrstreifen zwischen durchgehenden Linien"""
    ax, g = m['ax'], m['grenzen']
    ns = len(ax.s)
    ks = np.arange(ns)
    out, dateien = [], {}
    for j, fb in enumerate(m['fahrbahn']):
        if not fb:
            continue
        tl, tr = g[j], g[j + 1]
        tc = M.glatt((tl + tr) / 2, 9)
        breite = float(np.median(tr - tl)) + 0.2                     # 10 cm unter die Waende
        # Fahrstreifen: durchgehende Linien in diesem Streifen (Lage relativ zur Mitte, Median)
        teil = sorted(float(np.median(T - tc[k_])) for k_, T, durch in linien
                      if durch and np.median(T - (tl[k_] + tr[k_]) / 2) ** 2 < (breite / 2 - 0.5) ** 2)
        # Sperrflaechen am Rand (Median der Breite ueber ihre Laenge) sind keine Fahrstreifen; ihre Grenzlinie trennt nicht
        lo, hi = -breite / 2, breite / 2
        for j_, rand, sk, ST in sperr:
            if j_ == j:
                rel = float(np.median(ST - tc[sk]))
                lo, hi = (max(lo, rel), hi) if rand < 0 else (lo, min(hi, rel))
        teil = [x for x in teil if lo + 0.5 < x < hi - 0.5]
        kanten_ = [lo] + teil + [hi]
        spuren = [(a_ + b_) / 2 for a_, b_ in zip(kanten_[:-1], kanten_[1:]) if b_ - a_ >= 2.5]
        if not spuren:
            spuren = [(lo + hi) / 2]
        info = m.get('streifen_info', {}).get(j, {})
        seite = info.get('name') or ('trog' if j == seiten['trog'] else ('ost' if j < seiten['trog'] else 'west'))
        if info.get('spuren'):                                       # vom Nutzer: gleich breite Fahrstreifen
            n_ = int(info['spuren'])
            spuren = [lo + (hi - lo) * (i + 0.5) / n_ for i in range(n_)]
        name = f'{m.get("praefix", "AB_LS")}_Fahrbahn_{j}'                  # fester Name: neu bauen ueberschreibt
        s = Sli(f'File created with omsigen (portal_splines). Fahrbahn {seite}, gemessene Breite {breite:.2f} m, {len(spuren)} Fahrstreifen.')
        s.hoehen.append((-breite / 2, breite / 2, 0.0))
        s.flach('asphalt', -breite / 2, breite / 2, 0.0, u_abs=True)
        for x in spuren:
            rv = info.get('richtung')
            if rv in ('vor', 'zurueck'):
                r = 0 if rv == 'vor' else 1
            elif rv == 'beide' or seite == 'trog':
                r = 0 if x > 0 else 1                                # Rechtsverkehr: rechts der Achse in Achsrichtung
            elif seite in RICHTUNG:
                r = 0 if RICHTUNG[seite] > 0 else 1
            else:                                                    # ohne Angabe: ab 2 Fahrstreifen Gegenverkehr
                r = (0 if x > 0 else 1) if len(spuren) >= 2 else 0
            s.pfade.append((0, x, 0.0, 3.0, r))
        dateien[name] = s.text()
        Z = np.array([m['a'][k, j] + m['b'][k, j] * tc[k] for k in ks])
        P = linie_xy(ax, ks, tc)
        els = kette(P, Z, knoten=5.0)
        # Hoehe der gebauten Fahrbahn je Station (Auflagen wie die Schraffur liegen genau darauf)
        sb = np.r_[0, np.cumsum(np.hypot(*np.diff(P, axis=0).T))]
        m.setdefault('gebaut', {})[j] = np.array([kette_hoehe(els, x) for x in sb])
        out.append(dict(name=f'Fahrbahn_{j}_{seite}', art='weg', sli=f'Splines\\Aschaffenburg\\{m.get("ordner", ORDNER)}\\{name}.sli',
                        elemente=els))
    return out, dateien


def markierung_splines(m, linien):
    out, dateien = [], {}
    for durch in (True, False):
        name = 'AB_LS_Linie' if durch else 'AB_LS_Leitlinie'
        s = Sli(f'File created with omsigen (portal_splines). Markierung {"durchgehend" if durch else "Strich 3 m / Luecke 6 m"}, {LINIE:.2f} m.')
        s.flach('weiss' if durch else 'strich', -LINIE / 2, LINIE / 2, 0.0)
        dateien[name] = s.text()
    for n, (ks, T, durch) in enumerate(linien):
        Z = np.array([z_bei(m, k, t) for k, t in zip(ks, T)]) + 0.02
        out.append(dict(name=f'Markierung_{n}', art='markierung',
                        sli=f'Splines\\Aschaffenburg\\{m.get("ordner", ORDNER)}\\{"AB_LS_Linie" if durch else "AB_LS_Leitlinie"}.sli',
                        elemente=kette(linie_xy(m['ax'], ks, T), Z, knoten=5.0)))
    return out, dateien


def waende(m):
    """je Kante ein Wand-Spline; Profil-x = +t (rechts der Achsrichtung)"""
    ax, K, a, b = m['ax'], m['K'], m['a'], m['b']
    ns = len(ax.s)
    ringsohle = min(k['unten'] for k in K)
    out, dateien = [], {}
    for ki, kk in enumerate(K):
        ak = np.where(kk['aktiv'])[0]
        ks = np.arange(max(0, ak.min() - 1), min(ns - 1, ak.max() + 1) + 1)
        zl = np.array([a[k, ki] + b[k, ki] * kk['t'][k] for k in ks])           # Seite kleineres t
        zr = np.array([a[k, ki + 1] + b[k, ki + 1] * kk['t'][k] for k in ks])
        hoch = 1 if np.median(zr - zl) > 0 else -1                             # hohe Seite in t
        krone = np.maximum(zl, zr)
        diff = np.abs(zr - zl)
        tiefe = math.ceil(float(diff.max()) + 1.0)
        if not kk.get('bauen', True):
            continue
        trog = kk['art'] == 'trog' if 'art' in kk else (kk['unten'] < ringsohle + 1.0 and kk['hoehe'] > 3.0)
        material = kk.get('material') or ('paneel' if trog else 'beton')
        mit_gelaender = kk.get('gelaender', kk['hoehe'] > 2.0)
        tief = -hoch                                                           # Sichtseite der Wand
        s = Sli(f'File created with omsigen (portal_splines). Wand {ki}: {"Trog, Paneele, Bruestung" if trog else "Beton"}, '
                f'gemessen {kk["hoehe"]:.1f} m, Tiefe {tiefe} m.')
        if trog:
            s.senkrecht(material, 0.0, -tiefe, -0.3, tief)
            s.senkrecht('beton', 0.0, -0.3, BRUESTUNG, tief)
            s.flach('beton', 0.0, hoch * 0.3, BRUESTUNG)
            s.senkrecht('beton', hoch * 0.3, -0.3, BRUESTUNG, hoch)
            if mit_gelaender:
                s.senkrecht('stab', hoch * 0.15, BRUESTUNG, BRUESTUNG + GELAENDER, -1)
                s.senkrecht('stab', hoch * 0.15, BRUESTUNG, BRUESTUNG + GELAENDER, 1)
        else:
            # Kappe 15 cm ueber dem Boden der hohen Seite (liegt nicht deckungsgleich auf dem Boden-Spline)
            s.senkrecht(material, 0.0, -tiefe, 0.15, tief)
            s.flach('beton', 0.0, hoch * 0.35, 0.15)
            s.senkrecht('beton', hoch * 0.35, -0.3, 0.15, hoch)
            if mit_gelaender:
                s.senkrecht('stab', hoch * 0.2, 0.15, 0.15 + GELAENDER, -1)
                s.senkrecht('stab', hoch * 0.2, 0.15, 0.15 + GELAENDER, 1)
        name = f'{m.get("praefix", "AB_LS")}_Wand_{ki}'
        dateien[name] = s.text()
        out.append(dict(name=f'Wand_{ki}', art='wand', sli=f'Splines\\Aschaffenburg\\{m.get("ordner", ORDNER)}\\{name}.sli',
                        elemente=kette(linie_xy(ax, ks, kk['t'][ks]), krone, knoten=4.0)))
    return out, dateien


def boden(m):
    """Streifen ohne Fahrbahn: Boden-Spline (Gras, Gehweg wenn ueberwiegend Asphalt im Luftbild) auf gemessener Hoehe"""
    ax, g = m['ax'], m['grenzen']
    ks = np.arange(len(ax.s))
    out, dateien = [], {}
    for j, fb in enumerate(m['fahrbahn']):
        if fb:
            continue
        tl, tr = g[j], g[j + 1]
        breite = float(np.median(tr - tl))
        if breite < 0.5:
            continue
        tc = M.glatt((tl + tr) / 2, 9)
        dt, asph = m['quer'][j]
        geh = M.glatt((asph > 0.5).astype(float), 5) > 0.5              # Laeufe unter 1 m gehen im Nachbarn auf
        wahl = m.get('streifen_info', {}).get(j, {}).get('material', 'auto')
        if wahl in ('gras', 'gehweg'):
            geh = np.full(len(dt), wahl == 'gehweg')
        name = f'{m.get("praefix", "AB_LS")}_Boden_{j}'
        s = Sli(f'File created with omsigen (portal_splines). Boden {breite:.2f} m, Gras/Gehweg quer nach DOP20.')
        s.hoehen.append((-breite / 2 - 0.1, breite / 2 + 0.1, 0.0))
        rand = [0] + [i for i in range(1, len(dt)) if geh[i] != geh[i - 1]] + [len(dt)]
        for i0, i1 in zip(rand[:-1], rand[1:]):
            xa = -breite / 2 - (0.1 if i0 == 0 else 0.0) + i0 * 0.25
            xb = -breite / 2 + (0.1 if i1 == len(dt) else 0.0) + i1 * 0.25
            s.flach('gehweg' if geh[i0] else 'gras', xa, min(xb, breite / 2 + 0.1), 0.0, u_abs=True)
        dateien[name] = s.text()
        Z = np.array([m['a'][k, j] + m['b'][k, j] * tc[k] for k in ks])
        out.append(dict(name=f'Boden_{j}', art='boden', sli=f'Splines\\Aschaffenburg\\{m.get("ordner", ORDNER)}\\{name}.sli',
                        elemente=kette(linie_xy(ax, ks, tc), Z, knoten=5.0)))
    return out, dateien


def gelaende(m, bereich, tief=0.4, umkreis=7.2):
    """Kachelgelaende (5-m-Raster) unter die gemessenen Flaechen: Minimum im Umkreis einer Rasterzelle (Diagonale)"""
    ax, g = m['ax'], m['grenzen']
    Q = []
    for k in range(len(ax.s)):
        for t in np.arange(m.get('t0', -M.T_OST), m.get('t1', M.T_WEST) + 0.01, 1.0):
            p = ax.welt(k, t)
            Q.append((p[0], p[1], z_bei(m, k, t)))
    Q = np.array(Q)
    x0, y0, x1, y1 = bereich
    out = []
    for kx in range(math.floor(x0 / 300), math.floor(x1 / 300) + 1):
        for ky in range(math.floor(y0 / 300), math.floor(y1 / 300) + 1):
            for zeile in range(61):
                for spalte in range(61):
                    x, y = kx * 300 + spalte * 5.0, ky * 300 + zeile * 5.0
                    k_, t_ = ax.st(np.array([x]), np.array([y]))
                    if not (0 < k_[0] < len(ax.s) - 1 and m.get('t0', -M.T_OST) + 0.5 < t_[0] < m.get('t1', M.T_WEST) - 0.5):
                        continue                                       # nur Rasterpunkte unter den Splines
                    d = np.hypot(Q[:, 0] - x, Q[:, 1] - y)
                    out.append([kx, ky, spalte, zeile, round(float(Q[d < umkreis, 2].min()) - tief, 3)])
    return out


def texturen(omsi, ziel, quellen=None):
    from PIL import Image
    os.makedirs(ziel, exist_ok=True)
    quellen = (quellen or [os.path.join(omsi, 'Sceneryobjects', 'Aschaffenburg', 'Aburg_Lernstueck', 'Lernstueck', 'texture')]) +         [os.path.join(omsi, 'Splines', 'Marcel', 'texture')]
    for k, (datei, _) in TEX.items():
        if k == 'strich':
            continue
        for q in quellen:
            if os.path.isfile(os.path.join(q, datei)):
                shutil.copyfile(os.path.join(q, datei), os.path.join(ziel, datei))
                break
        else:
            raise FileNotFoundError(datei)
    # Leitlinie: 3 m Strich, 6 m Luecke (v laengs: obere Drittel weiss, Rest durchsichtig)
    im = Image.new('RGBA', (8, 96), (0, 0, 0, 0))
    for y in range(32):
        for x in range(8):
            im.putpixel((x, y), (235, 235, 230, 255))
    im.save(os.path.join(ziel, TEX['strich'][0]))


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--omsi', default=r'C:\Program Files (x86)\Steam\steamapps\common\OMSI 2')
    ap.add_argument('--karte', default='Aburg_Lernstueck')
    ap.add_argument('--laz', default=os.path.join('build', 'vermessung', '511_5535.laz'))
    ap.add_argument('--bereich', type=float, nargs=4, default=[1050.0, -378.0, 1115.0, -281.0])
    ap.add_argument('--cache', default=os.path.join(os.environ.get('LOCALAPPDATA', '.'), 'omsi-editor', 'geodaten', 'dop20'))
    a = ap.parse_args()
    bereich = a.bereich
    P = lese_punkte(a.laz, [bereich[0] - 2, bereich[1] - 2, bereich[2] + 2, bereich[3] + 2], os.path.join('build', 'vermessung', 'suedportal2.npy'))
    dop = dop_bild(bereich, a.cache)
    m = messung(P, bereich, dop)
    # Trog = tiefster Fahrbahn-Streifen
    fb = [j for j, f in enumerate(m['fahrbahn']) if f]
    trog = min(fb, key=lambda j: float(np.median(m['a'][:, j])))
    sperr = sperrflaechen(m, dop, bereich)
    linien = linien_bereinigen(m, markierungen(m, dop, bereich), sperr)
    strassen, dateien = [], {}
    for teil, d in (fahrbahnen(m, linien, dict(trog=trog), sperr), markierung_splines(m, linien), waende(m), boden(m)):
        strassen += teil
        dateien.update(d)
    ziel = os.path.join(a.omsi, 'Splines', 'Aschaffenburg', ORDNER)
    texturen(a.omsi, os.path.join(ziel, 'texture'))
    for name, text in dateien.items():
        with open(os.path.join(ziel, name + '.sli'), 'w', encoding='cp1252', newline='') as f:
            f.write(text)
    objekte = []
    if sperr:
        from .kreuzung import x_file
        me, o = sperr_objekt(m, sperr)
        ordner = os.path.join(a.omsi, 'Sceneryobjects', 'Aschaffenburg', a.karte, 'Lernstueck')
        rel = f'Sceneryobjects\\Aschaffenburg\\{a.karte}\\Lernstueck'
        name = 'Suedportal_Sperrflaechen'
        os.makedirs(os.path.join(ordner, 'model'), exist_ok=True)
        with open(os.path.join(ordner, 'model', name + '.x'), 'w', newline='') as f:
            f.write(x_file(me.V, me.F, [('sperr', 'AB_sperrflaeche.bmp')]))
        sco = ['Erzeugt mit omsigen (portal_splines): Sperrflaechen aus DOP20, Bayerische Vermessungsverwaltung (CC BY 4.0).', '',
               '[friendlyname]', 'Lernstueck Suedportal Sperrflaechen', '', '[groups]', '1', 'Aschaffenburg', '', '[fixed]', '',
               '[absheight]', '', '[mesh]', name + '.x', '',
               # liegt auf der Fahrbahn: Tiefenversatz wie bei OMSI-Markierungsobjekten (openOMSI: omsi-model matl_zbias)
               '[matl]', 'AB_sperrflaeche.bmp', '0', '[matl_zbias]', '1', '']
        open(os.path.join(ordner, name + '.sco'), 'w', encoding='cp1252', newline='').write('\r\n'.join(sco) + '\r\n')
        objekte.append(dict(name=name, rel=rel + '\\' + name + '.sco', x=float(o[0]), y=float(o[1]), z=0.0))
    print('Sperrflaechen:', [(j, r, int(ks[0]), int(ks[-1])) for j, r, ks, _ in sperr])
    out = dict(karte=a.karte, objekte=objekte, strassen=strassen, kreuzungen=[], gelaende=gelaende(m, bereich), gelaende_zurueck=[],
               quellen=['Laserdaten, DOP20 Bayern (CC BY 4.0)'])
    pfad = os.path.join(a.omsi, 'Sceneryobjects', 'Aschaffenburg', a.karte, 'Lernstueck', 'lernstueck3.json')
    json.dump(out, open(pfad, 'w', encoding='utf-8'), indent=1)
    print('Streifen:', [f"{float(np.median(m['grenzen'][j + 1] - m['grenzen'][j])):.1f} m {'Fahrbahn' if f else 'Boden'}"
                        for j, f in enumerate(m['fahrbahn'])])
    print('Markierungen:', [(int(ks[0]), int(ks[-1]), round(float(np.median(T)), 1), 'durch' if d else 'Strich') for ks, T, d in linien])
    print(f'{len(strassen)} Splines, {len(dateien)} .sli-Dateien, {len(out["gelaende"])} Gelaendepunkte -> {pfad}')


if __name__ == '__main__':
    main()
