"""Lernstueck Suedportal, Fassung 2: aus den Messdaten saubere Formen bauen (statt die Punkte direkt zu vernetzen).

1. Achse: Mitte zwischen den beiden Trogwaenden (aus den Laserdaten), geglaettet; Stationen s alle 1 m, Querweg t
   (positiv nach rechts = Osten).
2. Kanten: jede gemessene Wand als glatte Kurve t(s) (gleitend gemittelt ueber 9 m); wo eine Wand endet, laeuft sie mit
   Hoehe 0 weiter (keine Topologiewechsel, keine Loecher).
3. Flaechen: zwischen zwei Kanten ein Streifen; je Station Hoehe und Quergefaelle aus den Bodenpunkten des Streifens
   (robuste Ausgleichsgerade), ueber s geglaettet. Waende sitzen genau zwischen den Raendern der Nachbarstreifen.
4. Material je Streifen und Station (Mehrheit im Materialraster aus DOP20 + Baumkronen aus den Laserdaten), Waende zum
   Trog rote Paneele (Fotos), sonst Beton, Geländer auf Waenden ueber 2 m.
5. Markierungen: helle Laengslinien im Luftbild je Station gesucht, zu Linien t(s) verbunden, geglaettet und als saubere
   Streifen (0,12 m) gebaut; Strich/Luecke aus der Helligkeit entlang der Linie.

Quellen: Laserdaten, DOP20 Bayerische Vermessungsverwaltung (CC BY 4.0)."""
import argparse
import json
import math
import os

import numpy as np

from . import vermessung as V
from .portal_modell import TEX, lese_punkte, wandlinien, dop_bild, materialraster, gelaende_absenken

GLATT = 9
# Querweg t: positiv = rechts der Achse (Achse laeuft Nord -> Sued, rechts = Westen)
T_OST = 22.0
T_WEST = 15.0           # im Westen nur bis zur Bahnboeschung, der Bahneinschnitt bleibt Karte


def glatt(a, n=GLATT):
    a = np.asarray(a, float)
    if len(a) < 3:
        return a
    n = min(n, len(a) - (1 - len(a) % 2))
    k = np.ones(n) / n
    return np.convolve(np.pad(a, n // 2, mode='edge'), k, mode='valid')[:len(a)]


class Achse:
    def __init__(self, P):
        P = np.asarray(P, float)
        d = np.r_[0, np.cumsum(np.hypot(*np.diff(P, axis=0).T))]
        self.s = np.arange(0, d[-1], 1.0)
        self.p = np.c_[np.interp(self.s, d, P[:, 0]), np.interp(self.s, d, P[:, 1])]
        self.p = np.c_[glatt(self.p[:, 0], 15), glatt(self.p[:, 1], 15)]
        tg = np.gradient(self.p, axis=0)
        tg /= np.linalg.norm(tg, axis=1)[:, None]
        self.tg = tg
        self.n = np.c_[tg[:, 1], -tg[:, 0]]           # rechts der Laufrichtung

    def welt(self, i, t):
        return self.p[i] + self.n[i] * t

    def st(self, x, y):
        """Punkte -> (Stationsindex, t) ueber die naechste Station"""
        Q = np.c_[x, y]
        i = np.argmin(((Q[:, None, :] - self.p[None, :, :]) ** 2).sum(axis=2), axis=1)
        t = ((Q - self.p[i]) * self.n[i]).sum(axis=1)
        return i, t


def achse_aus(L):
    """Achse: Mitte der beiden tiefsten Wandlinien (Trog), Laufrichtung Nord -> Sued"""
    hoch = [i for i in range(len(L)) if np.median([b[3] - b[2] for b in L[i]]) > 3.0 and len(L[i]) > 40]
    unten = sorted(hoch, key=lambda i: np.median([b[2] for b in L[i]]))[:2]
    A, B = (np.array([[b[0], b[1]] for b in L[i]]) for i in unten)
    ys = np.arange(max(A[:, 0].min(), B[:, 0].min()), min(A[:, 0].max(), B[:, 0].max()), 1.0)
    xa = np.interp(ys, np.sort(A[:, 0]), A[np.argsort(A[:, 0]), 1])
    xb = np.interp(ys, np.sort(B[:, 0]), B[np.argsort(B[:, 0]), 1])
    P = np.c_[(xa + xb) / 2, ys][::-1]                  # Norden zuerst
    return Achse(P)


def kanten_st(L, ax):
    """Wandlinien -> je Linie Arrays ueber alle Stationen: t, z_unten, z_oben, aktiv (gemessen), Seite"""
    out = []
    ns = len(ax.s)
    for l in L:
        A = np.array([b[:4] for b in l])
        i, t = ax.st(A[:, 1], A[:, 0])
        T = np.full(ns, np.nan)
        ZU, ZO = np.full(ns, np.nan), np.full(ns, np.nan)
        for k in range(ns):
            m = i == k
            if m.any():
                T[k], ZU[k], ZO[k] = t[m].mean(), A[m, 2].mean(), A[m, 3].mean()
        aktiv = ~np.isnan(T)
        if aktiv.sum() < 6:
            continue
        idx = np.arange(ns)
        for arr in (T, ZU, ZO):
            arr[~aktiv] = np.interp(idx[~aktiv], idx[aktiv], arr[aktiv])
        out.append(dict(t=glatt(T), zu=glatt(ZU), zo=glatt(ZO), aktiv=aktiv, seite=l[0][4],
                        hoehe=float(np.median(ZO[aktiv] - ZU[aktiv])), unten=float(np.median(ZU[aktiv]))))
    out = [k for k in out if -T_OST + 0.5 < np.median(k['t']) < T_WEST - 0.5]
    out.sort(key=lambda k: np.median(k['t']))
    # Wand mit Bruestung = zwei Linien dicht beieinander: zu einer Wand (die hoehere, Krone = hoechste)
    zus = []
    for k in out:
        if zus and np.median(np.abs(k['t'] - zus[-1]['t'])) < 1.0:
            a = zus[-1]
            h = k if k['hoehe'] > a['hoehe'] else a
            h = dict(h, zo=np.maximum(a['zo'], k['zo']), zu=np.minimum(a['zu'], k['zu']), aktiv=a['aktiv'] | k['aktiv'])
            h['hoehe'] = float(np.median((h['zo'] - h['zu'])[h['aktiv']]))
            zus[-1] = h
        else:
            zus.append(k)
    out = zus
    # Reihenfolge erzwingen (Kanten kreuzen sich nicht)
    for a, b in zip(out[:-1], out[1:]):
        b['t'] = np.maximum(b['t'], a['t'] + 0.3)
    return out


def streifen_hoehen(B, ax, grenzen, fahrbahn=None):
    """je Station und Streifen (zwischen grenzen[j] und grenzen[j+1], t) robuste Gerade z = a + b t -> (a, b) Arrays"""
    ns, nb = len(ax.s), len(grenzen) - 1
    i, t = ax.st(B[:, 0], B[:, 1])
    a, b = np.full((ns, nb), np.nan), np.zeros((ns, nb))
    for k in range(ns):
        m = np.abs(i - k) <= 1
        tk, zk = t[m], B[m, 2]
        for j in range(nb):
            lo, hi = grenzen[j][k] + 0.25, grenzen[j + 1][k] - 0.25
            s = (tk > lo) & (tk < hi)
            if s.sum() < 4:
                continue
            tt, zz = tk[s], zk[s]
            if hi - lo > 1.5:
                bb, aa = np.polyfit(tt, zz, 1)
                r = np.abs(zz - (aa + bb * tt))
                g = r < max(0.08, np.percentile(r, 70))
                if g.sum() >= 3:
                    bb, aa = np.polyfit(tt[g], zz[g], 1)
            else:
                aa, bb = float(np.median(zz)), 0.0
            a[k, j], b[k, j] = aa, bb
    # Streifen ganz ohne Punkte: Mittel der Nachbarn
    for j in range(nb):
        if np.isnan(a[:, j]).all():
            nb_ = [a[:, q] for q in (j - 1, j + 1) if 0 <= q < nb and not np.isnan(a[:, q]).all()]
            a[:, j] = np.nanmean(nb_, axis=0) if nb_ else 0.0
    for j in range(nb):
        ok = ~np.isnan(a[:, j])
        if not ok.any():
            a[:, j] = np.nanmean(a)
            continue
        idx = np.arange(ns)
        a[~ok, j] = np.interp(idx[~ok], idx[ok], a[ok, j])
        b[~ok, j] = np.interp(idx[~ok], idx[ok], b[ok, j])
        # Fahrbahnstreifen: Quergefaelle hoechstens 8 % (Bruestungspunkte am Rand duerfen die Flaeche nicht kippen)
        g = 0.08 if (fahrbahn is not None and fahrbahn[j]) else 0.6
        a[:, j], b[:, j] = glatt(a[:, j], 7), glatt(np.clip(b[:, j], -g, g), 7)
    return a, b


def bauen(P, bereich, dop):
    from .sonderbau_ring import Mesh
    B = P[P[:, 3] == 2]
    L = wandlinien(B, bereich)
    ax = achse_aus(L)
    K = kanten_st(L, ax)
    ns = len(ax.s)
    grenzen = [np.full(ns, -T_OST)] + [k['t'] for k in K] + [np.full(ns, T_WEST)]
    mr = materialraster(P, bereich, dop, L)
    x0, y0, x1, y1 = bereich
    H, W = mr.shape
    # Fahrbahnstreifen: in der Mitte ueberwiegend Asphalt
    fahrbahn = []
    for j in range(len(grenzen) - 1):
        n_a = n = 0
        for k in range(0, ns, 3):
            p_ = ax.welt(k, (grenzen[j][k] + grenzen[j + 1][k]) / 2)
            px, py = int((p_[0] - x0) * 5), int((y1 - p_[1]) * 5)
            if 0 <= px < W and 0 <= py < H:
                n += 1
                n_a += mr[py, px] == 1
        fahrbahn.append(n > 0 and n_a / n > 0.6 and np.median(grenzen[j + 1] - grenzen[j]) > 2.5)
    a, b = streifen_hoehen(B, ax, grenzen, fahrbahn)
    z_von = lambda k, j, t: a[k, j] + b[k, j] * t
    o = np.array([round((x0 + x1) / 2, 1), round((y0 + y1) / 2, 1), 0.0])
    m = Mesh()
    ringsohle = min(k['unten'] for k in K)

    def mat_bei(p):
        px, py = int((p[0] - x0) * 5), int((y1 - p[1]) * 5)
        if 0 <= px < W and 0 <= py < H:
            return 'asphalt' if mr[py, px] == 1 else 'boden'
        return 'boden'

    def W3(k, t, z):
        p = ax.welt(k, t)
        return (p[0] - o[0], p[1] - o[1], z)

    nb = len(grenzen) - 1
    for k in range(ns - 1):
        for j in range(nb):
            ta0, tb0 = grenzen[j][k], grenzen[j + 1][k]
            ta1, tb1 = grenzen[j][k + 1], grenzen[j + 1][k + 1]
            # Rand-Streifen in 2-m-Schritten (Gelaende), Fahrbahnstreifen ein Viereck quer
            n = max(1, int(math.ceil(max(tb0 - ta0, tb1 - ta1) / 1.0)))
            for q in range(n):
                f0, f1 = q / n, (q + 1) / n
                t00, t01 = ta0 + (tb0 - ta0) * f0, ta0 + (tb0 - ta0) * f1
                t10, t11 = ta1 + (tb1 - ta1) * f0, ta1 + (tb1 - ta1) * f1
                pts = [W3(k, t00, z_von(k, j, t00)), W3(k, t01, z_von(k, j, t01)), W3(k + 1, t11, z_von(k + 1, j, t11)), W3(k + 1, t10, z_von(k + 1, j, t10))]
                c = np.mean([ax.welt(k, (t00 + t01) / 2), ax.welt(k + 1, (t10 + t11) / 2)], axis=0)
                mat = mat_bei(c)
                uv = [(p_[0] / 4, p_[1] / 4) for p_ in pts]          # lokale Werte (grosse UV -> Ungenauigkeit)
                m.dreieck(mat, [pts[0], pts[1], pts[2]], [uv[0], uv[1], uv[2]], (0, 0, 1))
                m.dreieck(mat, [pts[0], pts[2], pts[3]], [uv[0], uv[2], uv[3]], (0, 0, 1))
    # Waende zwischen den Streifen j (links) und j+1 (rechts) an Kante K[j]
    for ki, kk in enumerate(K):
        j = ki
        trog = kk['unten'] < ringsohle + 1.0 and kk['hoehe'] > 3.0
        for k in range(ns - 1):
            t0, t1 = kk['t'][k], kk['t'][k + 1]
            zl0, zl1 = z_von(k, j, t0), z_von(k + 1, j, t1)
            zr0, zr1 = z_von(k, j + 1, t0), z_von(k + 1, j + 1, t1)
            if max(abs(zl0 - zr0), abs(zl1 - zr1)) < 0.03:
                continue
            p0, p1 = ax.welt(k, t0), ax.welt(k + 1, t1)
            d = math.hypot(*(p1 - p0))
            q = [(p0[0] - o[0], p0[1] - o[1], zl0), (p1[0] - o[0], p1[1] - o[1], zl1), (p1[0] - o[0], p1[1] - o[1], zr1), (p0[0] - o[0], p0[1] - o[1], zr0)]
            uv = [((ax.s[k] % 50) / 2.5, zl0 / 2.5), ((ax.s[k] % 50 + d) / 2.5, zl1 / 2.5), ((ax.s[k] % 50 + d) / 2.5, zr1 / 2.5), ((ax.s[k] % 50) / 2.5, zr0 / 2.5)]
            nr = ax.n[k]
            tief_links = (zl0 + zl1) < (zr0 + zr1)
            n_tief = (-nr[0], -nr[1], 0) if tief_links else (nr[0], nr[1], 0)
            m.quad('paneel' if trog else 'beton', q, uv, n_tief)
            m.quad('beton', q, uv, (-n_tief[0], -n_tief[1], 0))
            zt0, zt1 = max(zl0, zr0), max(zl1, zr1)
            # Trogwand: Betonbruestung 0,3 m breit, 0,8 m hoch auf der hohen Seite (Fotos), Gelaender darauf
            if trog and kk['aktiv'][k]:
                hoch = -1 if not tief_links else 1          # Querrichtung zur hohen Seite (in t)
                ta, tb = (t0, t1)
                q0a, q1a = ax.welt(k, ta + hoch * 0.3), ax.welt(k + 1, tb + hoch * 0.3)
                def pz(p, z):
                    return (p[0] - o[0], p[1] - o[1], z)
                oben0, oben1 = zt0 + 0.8, zt1 + 0.8
                # Innenseite (Trog) bis zur Krone hoch, Aussenseite zur Rampe, Deckel
                m.quad('beton', [pz(p0, zt0), pz(p1, zt1), pz(p1, oben1), pz(p0, oben0)], [(0, 0), (d / 2.5, 0), (d / 2.5, 0.32), (0, 0.32)], n_tief)
                m.quad('beton', [pz(q0a, zt0), pz(q1a, zt1), pz(q1a, oben1), pz(q0a, oben0)], [(0, 0), (d / 2.5, 0), (d / 2.5, 0.32), (0, 0.32)], (-n_tief[0], -n_tief[1], 0))
                m.quad('beton', [pz(p0, oben0), pz(p1, oben1), pz(q1a, oben1), pz(q0a, oben0)], [(0, 0), (d / 2.5, 0), (d / 2.5, 0.12), (0, 0.12)], (0, 0, 1))
                zt0, zt1 = oben0, oben1
            # Gelaender oben auf hohen Waenden
            if kk['hoehe'] > 2.0 and kk['aktiv'][k]:
                g = [(p0[0] - o[0], p0[1] - o[1], zt0), (p1[0] - o[0], p1[1] - o[1], zt1), (p1[0] - o[0], p1[1] - o[1], zt1 + 1.0), (p0[0] - o[0], p0[1] - o[1], zt0 + 1.0)]
                m.quad('stab', g, [(0, 0), (d, 0), (d, 1), (0, 1)], None, beidseitig=True)
    linien_markierung(m, ax, grenzen, a, b, dop, bereich, o, mr)
    return m, o, L, ax, K, grenzen, a, b


def linien_markierung(m, ax, grenzen, a, b, dop, bereich, o, mr):
    """helle Laengslinien auf Asphalt je Station suchen, verbinden, glaetten, als 0,12-m-Streifen bauen"""
    x0, y0, x1, y1 = bereich
    H, W = dop.shape[:2]
    v = dop.mean(axis=2)
    sat = dop.max(axis=2) - dop.min(axis=2)
    ns = len(ax.s)
    funde = []                                   # je Station Liste t
    for k in range(ns):
        ts = np.arange(-T_OST + 1, T_WEST - 1, 0.1)
        P = ax.p[k][None, :] + ts[:, None] * ax.n[k][None, :]
        px = np.clip(((P[:, 0] - x0) * 5).astype(int), 0, W - 1)
        py = np.clip(((y1 - P[:, 1]) * 5).astype(int), 0, H - 1)
        vv, ss, mm = v[py, px], sat[py, px], mr[py, px]
        ref = np.median(vv[mm == 1]) if (mm == 1).any() else 120
        hell = (vv > ref + 45) & (ss < 40) & (mm == 1)
        # Laeufe heller Werte, schmal (bis 0,5 m) = Linie
        t_l, lauf = [], []
        for t_, h_ in zip(ts, hell):
            if h_:
                lauf.append(t_)
            elif lauf:
                if len(lauf) <= 5:
                    t_l.append(float(np.mean(lauf)))
                lauf = []
        funde.append(t_l)
    # verbinden
    linien, offen = [], []
    for k, ts in enumerate(funde):
        frei = list(ts)
        neu = []
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
        for t in frei:
            neu.append([(k, t)])
        offen = neu
    linien += offen
    for l in linien:
        if len(l) < 4 or l[-1][0] - l[0][0] < 8:
            continue
        K = np.array([p[0] for p in l])
        T = glatt(np.interp(np.arange(K[0], K[-1] + 1), K, [p[1] for p in l]), 7)
        ks = np.arange(K[0], K[-1] + 1)
        gemessen = set(K.tolist())
        for n_, k in enumerate(ks[:-1]):
            # Luecke in der Linie (mehr als 2 Stationen ohne Fund) = Strichluecke
            if not any(abs(k - g) <= 1 for g in gemessen):
                continue
            t0, t1 = T[n_], T[n_ + 1]
            j0 = int(np.searchsorted([g[k] for g in grenzen], t0)) - 1
            j1 = int(np.searchsorted([g[k + 1] for g in grenzen], t1)) - 1
            if j0 < 0 or j1 < 0 or j0 >= a.shape[1] or j1 >= a.shape[1]:
                continue
            z0, z1 = a[k, j0] + b[k, j0] * t0 + 0.015, a[k + 1, j1] + b[k + 1, j1] * t1 + 0.015
            pa, pb = ax.welt(k, t0 - 0.06), ax.welt(k, t0 + 0.06)
            pc, pd = ax.welt(k + 1, t1 + 0.06), ax.welt(k + 1, t1 - 0.06)
            q = [(pa[0] - o[0], pa[1] - o[1], z0), (pb[0] - o[0], pb[1] - o[1], z0), (pc[0] - o[0], pc[1] - o[1], z1), (pd[0] - o[0], pd[1] - o[1], z1)]
            m.quad('weiss', q, [(0, 0), (1, 0), (1, 1), (0, 1)], (0, 0, 1))


def main():
    from .sonderbau_ring import texturen
    from .kreuzung import x_file
    from .portal_modell import ringspline
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
    m, o, L, ax, K, grenzen, A, Bb = bauen(P, bereich, dop)
    name = 'Suedportal'
    ordner = os.path.join(a.omsi, 'Sceneryobjects', 'Aschaffenburg', a.karte, 'Lernstueck')
    rel = f'Sceneryobjects\\Aschaffenburg\\{a.karte}\\Lernstueck'
    os.makedirs(os.path.join(ordner, 'model'), exist_ok=True)
    texturen(a.omsi, os.path.join(ordner, 'texture'))
    with open(os.path.join(ordner, 'model', name + '.x'), 'w', newline='') as f:
        f.write(x_file(m.V, m.F, [(k, TEX[k]) for k in m.mats]))
    sco = ['Erzeugt mit omsigen (portal_modell2) aus Laserdaten und DOP20, Bayerische Vermessungsverwaltung (CC BY 4.0).', '',
           '[friendlyname]', 'Lernstueck Suedportal', '', '[groups]', '1', 'Aschaffenburg', '', '[fixed]', '', '[absheight]', '',
           '[mesh]', name + '.x', '']
    for k in m.mats:
        if TEX[k].endswith('.tga'):
            sco += ['[matl]', TEX[k], '0', '[matl_alpha]', '2', '']
    open(os.path.join(ordner, name + '.sco'), 'w', encoding='cp1252', newline='').write('\r\n'.join(sco) + '\r\n')
    Q = np.array([(p[0] + o[0], p[1] + o[1], p[2]) for p in [(v_[0], v_[2], v_[1]) for v_ in m.V]])
    out = dict(karte=a.karte, objekte=[dict(name=name, rel=rel + '\\' + name + '.sco', x=float(o[0]), y=float(o[1]), z=0.0)],
               strassen=ringspline(L, bereich), kreuzungen=[], gelaende=gelaende_absenken(Q, bereich), gelaende_zurueck=[],
               quellen=['Laserdaten, DOP20 Bayern (CC BY 4.0)'])
    json.dump(out, open(os.path.join(ordner, 'lernstueck.json'), 'w', encoding='utf-8'), indent=1)
    print(f'{name}: {len(m.F)} Dreiecke, {len(K)} Kanten, Achse {ax.s[-1]:.0f} m, Materialien {m.mats}')


if __name__ == '__main__':
    main()
