"""Bauprüfer: prüft ein fertig gebautes Bauwerk (Objekte .x + Splines .sli + Ampeln) auf typische Baufehler, bevor es
in die Karte kommt. Arbeitet nur auf den geschriebenen Dateien, damit er für jedes Bauwerk gilt (heute der Sonderbau
Ring, später jedes neue Objekt).

Prüfungen (je Fund: Art, Text, Weltpunkt, Teil):
- textur_verzerrt: Texturkoordinaten eines Dreiecks stark ungleich gestreckt (Verhältnis der Hauptstreckungen > 4)
  oder zusammengefallen (Textur verschmiert)
- dreieck_entartet: Dreieck ohne Fläche
- doppelte_flaeche: zwei Flächen verschiedener Teile/Materialien liegen deckungsgleich übereinander (Flimmern)
- spalt: freie Kante eines Objekts endet knapp (3 bis 60 cm) neben einem anderen Teil statt daran
- offenes_ende: Ende einer Wand-/Spline-Kette steht frei über dem Gelände (man sieht ins Profil)
- ampel_schwebt / ampel_versinkt / ampel_ohne_mast: Ampelteile passen nicht zum Boden darunter
- grube: abgesenkter Gelaendepunkt, ueber dem kein Teil liegt (man sieht ins Loch)

Aufruf: python -m omsigen.baupruefer <sonderbau.json> [--omsi …] -> baupruefung.json daneben + Zusammenfassung.
"""
import argparse
import json
import math
import os
import re
from collections import defaultdict

import numpy as np

STANDARD_OMSI = r'C:\Program Files (x86)\Steam\steamapps\common\OMSI 2'
ANISO = 4.0               # Textur-Streckung: Verhältnis grösste/kleinste Streckung
SPALT = (0.03, 0.60)      # m: freie Kante so nah an einem anderen Teil, ohne es zu berühren
ENDE_FREI = 0.10          # m: Kettenende weiter als das von jedem anderen Teil
BODEN_TOL = 0.08          # m: Ampelfuss gegen Boden
GRUPPE = 3.0              # m: gleiche Funde in diesem Umkreis zusammenfassen
SPLINE_SCHRITT = 1.0      # m: Abtastung der Splines


# ------------------------------------------------------------------ Teile lesen
class Teil:
    """Dreiecke eines Bauteils in Weltkoordinaten (x Ost, y Nord, z hoch) mit Texturkoordinaten und Material"""

    def __init__(self, name, art):
        self.name, self.art = name, art
        self.P = np.zeros((0, 3, 3))       # Dreiecke x Ecken x xyz
        self.UV = np.zeros((0, 3, 2))
        self.mat = []
        self.enden = []                    # Spline-Ketten: [(Punkte des Endprofils (n x 3), Text)]
        self.koerper = None                # Waende: (Abtastpunkte (x, y, Richtung, z), x-Bereich, Tiefe) - Innenraum

    def innen(self, p):
        """liegt p im Wandkoerper (unter der Krone, zwischen den Sichtflaechen)?"""
        if self.koerper is None:
            return False
        A, (x0, x1), tiefe = self.koerper
        d = np.hypot(A[:, 0] - p[0], A[:, 1] - p[1])
        i = int(np.argmin(d))
        if d[i] > max(abs(x0), abs(x1)) + 1.5:
            return False
        h = math.radians(A[i, 2])
        q = np.asarray(p[:2], float) - A[i, :2]
        quer = q[0] * math.cos(h) - q[1] * math.sin(h)
        laengs = q[0] * math.sin(h) + q[1] * math.cos(h)
        return abs(laengs) <= 1.0 and x0 - 0.01 <= quer <= x1 + 0.01 and A[i, 3] - tiefe - 0.01 <= p[2] <= A[i, 3] + 0.01

    def setzen(self, P, UV, mat):
        self.P, self.UV, self.mat = np.asarray(P, float).reshape(-1, 3, 3), np.asarray(UV, float).reshape(-1, 3, 2), list(mat)


def x_lesen(pfad):
    """DirectX-.x (Text, wie kreuzung.x_file): Ecken (x, hoch, vor), Dreiecke, Texturkoordinaten, Materialnamen je
    Dreieck -> (V n x 3, F m x 3, UV n x 2, Material je Dreieck)"""
    t = open(pfad, encoding='cp1252', errors='replace').read()
    m = re.search(r'Mesh\s*\w*\s*\{\s*(\d+);', t)
    pos = m.end()
    nv = int(m.group(1))
    zahlen = re.compile(r'-?\d+(?:\.\d+)?(?:[eE][-+]?\d+)?')

    def lies(n, k, pos):
        werte = []
        it = zahlen.finditer(t, pos)
        for z in it:
            werte.append(float(z.group()))
            if len(werte) == n * k:
                return np.array(werte).reshape(n, k), z.end()
        raise ValueError('x-Datei zu kurz')
    V, pos = lies(nv, 3, pos)
    m = zahlen.search(t, pos)
    nf = int(m.group())
    pos = m.end()
    F = []
    it = zahlen.finditer(t, pos)
    while len(F) < nf:
        k = int(next(it).group())
        idx = [int(next(it).group()) for _ in range(k)]
        for i in range(1, k - 1):             # Vielecke in Dreiecke
            F.append((idx[0], idx[i], idx[i + 1]))
    F = np.array(F, int)
    UV = np.zeros((nv, 2))
    m = re.search(r'MeshTextureCoords\s*\{\s*(\d+);', t)
    if m:
        UV, _ = lies(int(m.group(1)), 2, m.end())
    mat = ['?'] * len(F)
    m = re.search(r'MeshMaterialList\s*\{\s*(\d+);\s*(\d+);', t)
    if m:
        nm, nfm = int(m.group(1)), int(m.group(2))
        idx, end = lies(nfm, 1, m.end())
        namen = re.findall(r'Material\s+(\w*)\s*\{', t[end:])
        namen += [f'm{i}' for i in range(len(namen), nm)]
        mat = [namen[int(i)] if int(i) < len(namen) else '?' for i in idx[:, 0]][:len(F)]
        mat += ['?'] * (len(F) - len(mat))
    return V, F, UV, mat


def objekt_teil(omsi, rel, x, y, z, rot=0.0):
    """Objekt (.sco mit [mesh]) an Weltpunkt x, y, Hoehe z, Drehung rot (Grad im Uhrzeigersinn)"""
    sco = os.path.join(omsi, rel)
    ordner = os.path.dirname(sco)
    text = open(sco, encoding='cp1252', errors='replace').read().replace('\r', '')
    meshes = re.findall(r'^\[mesh\]\n([^\n]+)', text, re.M)
    teil = Teil(os.path.splitext(os.path.basename(rel))[0], 'objekt')
    P, UV, MAT = [], [], []
    c, s = math.cos(math.radians(rot)), math.sin(math.radians(rot))
    for mesh in meshes:
        pfad = os.path.join(ordner, 'model', mesh.strip())
        if not os.path.isfile(pfad) or not pfad.lower().endswith('.x'):
            continue
        V, F, uv, mat = x_lesen(pfad)
        # Objektkoordinaten (x rechts, hoch, vor) -> Welt
        wx = x + V[:, 0] * c + V[:, 2] * s
        wy = y - V[:, 0] * s + V[:, 2] * c
        W = np.c_[wx, wy, z + V[:, 1]]
        P.append(W[F])
        UV.append(uv[F])
        MAT += [f'{mesh}:{m}' for m in mat]
    if P:
        teil.setzen(np.concatenate(P), np.concatenate(UV), MAT)
    return teil


def sli_lesen(pfad):
    """Profile einer .sli -> Liste (Textur, [(x, y, u, vrate)])"""
    zeilen = [z.strip() for z in open(pfad, encoding='cp1252', errors='replace').read().replace('\r', '').split('\n')]
    tex, prof, akt = [], [], None
    i = 0
    while i < len(zeilen):
        z = zeilen[i]
        if z == '[texture]':
            tex.append(zeilen[i + 1])
            i += 2
            continue
        if z == '[profile]':
            k = int(float(zeilen[i + 1]))
            akt = (tex[k] if k < len(tex) else '?', [])
            prof.append(akt)
            i += 2
            continue
        if z == '[profilepnt]' and akt is not None:
            akt[1].append(tuple(float(zeilen[i + j]) for j in range(1, 5)))
            i += 5
            continue
        i += 1
    return [p for p in prof if len(p[1]) >= 2]


def _punkt(e, u):
    """Punkt (x, y), Richtung (OMSI-Grad) bei Laenge u im Element (OMSI-Bogenregel) und Hoehe (kubisch mit Steigungen)"""
    h = math.radians(e['richtung'])
    if not e.get('radius'):
        x, y, r = e['x'] + u * math.sin(h), e['y'] + u * math.cos(h), e['richtung']
    else:
        R = e['radius']
        cx, cy = e['x'] + R * math.cos(h), e['y'] - R * math.sin(h)
        h2 = h + u / R
        x, y, r = cx - R * math.cos(h2), cy + R * math.sin(h2), math.degrees(h2)
    L = max(e['laenge'], 1e-9)
    f = u / L
    za, zb = e['z'], e['z'] + e.get('dh', 0.0)
    ga, gb = e.get('stg_a', 0.0) / 100, e.get('stg_e', 0.0) / 100
    z = (2 * f ** 3 - 3 * f ** 2 + 1) * za + (f ** 3 - 2 * f ** 2 + f) * L * ga + (-2 * f ** 3 + 3 * f ** 2) * zb + (f ** 3 - f ** 2) * L * gb
    return x, y, r, z


def spline_teil(omsi, name, sli_rel, elemente):
    """Spline-Kette als Dreiecke (Abtastung SPLINE_SCHRITT) und ihre beiden Endprofile"""
    teil = Teil(name, 'spline')
    profile = sli_lesen(os.path.join(omsi, sli_rel))
    # Abtastpunkte entlang der Kette: (x, y, Richtung, z, Laenge ab Anfang)
    pts, kum = [], 0.0
    for e in elemente:
        n = max(1, math.ceil(e['laenge'] / SPLINE_SCHRITT))
        for i in range(n + (1 if e is elemente[-1] else 0)):
            u = e['laenge'] * i / n
            x, y, r, z = _punkt(e, u)
            pts.append((x, y, r, z, kum + u))
        kum += e['laenge']
    if len(pts) < 2 or not profile:
        return teil

    def welt(p, px, py):
        x, y, r, z, _ = p
        h = math.radians(r)
        return (x + px * math.cos(h), y - px * math.sin(h), z + py)
    P, UV, MAT = [], [], []
    for tex, pp in profile:
        for (x0, y0, u0, v0), (x1, y1, u1, v1) in zip(pp, pp[1:]):
            for a, b in zip(pts, pts[1:]):
                A0, A1, B0, B1 = welt(a, x0, y0), welt(a, x1, y1), welt(b, x0, y0), welt(b, x1, y1)
                ua0, ua1 = (u0, a[4] * v0), (u1, a[4] * v1)
                ub0, ub1 = (u0, b[4] * v0), (u1, b[4] * v1)
                P += [(A0, A1, B1), (A0, B1, B0)]
                UV += [(ua0, ua1, ub1), (ua0, ub1, ub0)]
                MAT += [tex, tex]
    teil.setzen(P, UV, MAT)
    unten = [(x, y) for _, pp in profile for x, y, _, _ in pp if y <= 0.0]
    if unten:
        xs = [x for x, _ in unten]
        teil.koerper = (np.array([(p_[0], p_[1], p_[2], p_[3]) for p_ in pts]), (min(xs), max(xs)), -min(y for _, y in unten))
    for p, wo in ((pts[0], 'Anfang'), (pts[-1], 'Ende')):
        # nur massive Profilteile: Gelaender und Laermschutz-Paneele duerfen frei enden
        q = np.array([welt(p, x, y) for tex, pp in profile if not _duenn(tex) for x, y, _, _ in pp] or [welt(p, 0.0, 0.0)])
        teil.enden.append((q, wo))
    return teil


# ------------------------------------------------------------------ Geometrie
def _streckung(P, UV):
    """je Dreieck die beiden Hauptstreckungen der Texturabbildung (Texturwiederholungen je Meter)"""
    e1 = P[:, 1] - P[:, 0]
    e2 = P[:, 2] - P[:, 0]
    n = np.cross(e1, e2)
    fl = np.linalg.norm(n, axis=1)
    ax = e1 / np.maximum(np.linalg.norm(e1, axis=1), 1e-12)[:, None]
    ay = np.cross(n / np.maximum(fl, 1e-12)[:, None], ax)
    # Dreieck in seiner Ebene: (x1, 0), (x2, y2)
    x1 = np.linalg.norm(e1, axis=1)
    x2 = np.einsum('ij,ij->i', e2, ax)
    y2 = np.einsum('ij,ij->i', e2, ay)
    d1 = UV[:, 1] - UV[:, 0]
    d2 = UV[:, 2] - UV[:, 0]
    # J * [x1 x2; 0 y2] = [d1 d2]  ->  J = [d1 d2] * inv
    with np.errstate(divide='ignore', invalid='ignore'):
        inv = np.zeros((len(P), 2, 2))
        inv[:, 0, 0] = 1 / x1
        inv[:, 0, 1] = -x2 / (x1 * y2)
        inv[:, 1, 1] = 1 / y2
    D = np.stack([d1, d2], axis=2)                      # (m, 2 uv, 2 Kanten)
    J = np.einsum('mij,mjk->mik', D, inv)
    s = np.linalg.svd(np.nan_to_num(J), compute_uv=False)
    return s[:, 0], s[:, 1], fl / 2


def _punkt_dreieck(p, A, B, C):
    """Abstand Punkt p zu vielen Dreiecken (numpy, Ericson)"""
    ab, ac, ap = B - A, C - A, p - A
    d1, d2 = np.einsum('ij,ij->i', ab, ap), np.einsum('ij,ij->i', ac, ap)
    bp = p - B
    d3, d4 = np.einsum('ij,ij->i', ab, bp), np.einsum('ij,ij->i', ac, bp)
    cp = p - C
    d5, d6 = np.einsum('ij,ij->i', ab, cp), np.einsum('ij,ij->i', ac, cp)
    va, vb, vc = d3 * d6 - d5 * d4, d5 * d2 - d1 * d6, d1 * d4 - d3 * d2
    with np.errstate(divide='ignore', invalid='ignore'):
        den = va + vb + vc
        v = np.where(den != 0, vb / den, 0)
        w = np.where(den != 0, vc / den, 0)
    Q = A + ab * v[:, None] + ac * w[:, None]
    # Randfälle
    def kante(X, Y):
        xy = Y - X
        t = np.clip(np.einsum('ij,ij->i', p - X, xy) / np.maximum(np.einsum('ij,ij->i', xy, xy), 1e-12), 0, 1)
        return X + xy * t[:, None]
    innen = (va >= 0) & (vb >= 0) & (vc >= 0)
    kandidaten = [kante(A, B), kante(B, C), kante(C, A)]
    dk = np.min([np.linalg.norm(K - p, axis=1) for K in kandidaten], axis=0)
    di = np.linalg.norm(Q - p, axis=1)
    return np.where(innen, di, dk)


class Raum:
    """alle Dreiecke aller Teile mit einem groben Gitter für Nachbarschaftsanfragen"""

    def __init__(self, teile, zelle=2.0):
        self.zelle = zelle
        self.teile = teile
        self.P = np.concatenate([t.P for t in teile if len(t.P)]) if any(len(t.P) for t in teile) else np.zeros((0, 3, 3))
        self.teil = np.concatenate([np.full(len(t.P), i) for i, t in enumerate(teile) if len(t.P)]) if len(self.P) else np.zeros(0, int)
        lo = self.P.min(axis=1)[:, :2]
        hi = self.P.max(axis=1)[:, :2]
        self.gitter = defaultdict(list)
        for k, (a, b) in enumerate(zip(np.floor(lo / zelle).astype(int), np.floor(hi / zelle).astype(int))):
            if (b - a).max() > 40:
                continue
            for i in range(a[0], b[0] + 1):
                for j in range(a[1], b[1] + 1):
                    self.gitter[(i, j)].append(k)

    def nahe(self, p, r):
        i0, j0 = int(math.floor((p[0] - r) / self.zelle)), int(math.floor((p[1] - r) / self.zelle))
        i1, j1 = int(math.floor((p[0] + r) / self.zelle)), int(math.floor((p[1] + r) / self.zelle))
        s = set()
        for i in range(i0, i1 + 1):
            for j in range(j0, j1 + 1):
                s.update(self.gitter.get((i, j), ()))
        return np.fromiter(s, int) if s else np.zeros(0, int)

    def abstand(self, p, r, ausser=None):
        """kleinster Abstand von p zu Dreiecken anderer Teile (oder inf)"""
        k = self.nahe(p, r)
        if ausser is not None:
            k = k[self.teil[k] != ausser]
        if not len(k):
            return math.inf
        T = self.P[k]
        return float(_punkt_dreieck(np.asarray(p, float), T[:, 0], T[:, 1], T[:, 2]).min())

    def abstand_eigen(self, p, a, b, teil, r=0.1):
        """kleinster Abstand von p zu Dreiecken des eigenen Teils ohne die, die eine Ecke mit der Kante a-b teilen"""
        k = self.nahe(p, r)
        k = k[self.teil[k] == teil]
        if not len(k):
            return math.inf
        T = self.P[k]
        teilt = np.zeros(len(k), bool)
        for q in (a, b):
            teilt |= (np.linalg.norm(T - q, axis=2) < 0.002).any(axis=1)
        T = T[~teilt]
        if not len(T):
            return math.inf
        return float(_punkt_dreieck(np.asarray(p, float), T[:, 0], T[:, 1], T[:, 2]).min())

    def boden(self, x, y, z_max):
        """höchste nach oben zeigende Fläche unter (x, y) bis z_max (oder None)"""
        k = self.nahe((x, y), 0.01)
        best = None
        for T in self.P[k]:
            a, b, c = T
            n = np.cross(b - a, c - a)
            if abs(n[2]) < 0.3 * np.linalg.norm(n):
                continue
            d = (b - a)[:2], (c - a)[:2]
            det = d[0][0] * d[1][1] - d[0][1] * d[1][0]
            if abs(det) < 1e-12:
                continue
            q = np.array([x, y]) - a[:2]
            u = (q[0] * d[1][1] - q[1] * d[1][0]) / det
            v = (d[0][0] * q[1] - d[0][1] * q[0]) / det
            if u < -1e-6 or v < -1e-6 or u + v > 1 + 1e-6:
                continue
            z = a[2] + u * (b - a)[2] + v * (c - a)[2]
            if z <= z_max + 1e-6 and (best is None or z > best):
                best = z
        return best


def verdeckt(raum, gelaende, p, ausser=None, hoehe=0.4, koerper=True):
    """liegt p unter dem Gelaende, in einem Wandkoerper oder knapp (2 cm .. hoehe) unter einer Flaeche eines anderen
    Teils?"""
    for i, t in enumerate(raum.teile):
        if koerper and i != ausser and t.koerper is not None and t.innen(p):
            return True
    if gelaende is not None:
        g = gelaende(p[0], p[1])
        if g is not None and p[2] < g - 0.05:
            return True
    k = raum.nahe(p, 0.01)
    if ausser is not None:
        k = k[raum.teil[k] != ausser]
    for T in raum.P[k]:
        a, b, c = T
        n = np.cross(b - a, c - a)
        if abs(n[2]) < 0.3 * np.linalg.norm(n):
            continue
        d = (b - a)[:2], (c - a)[:2]
        det = d[0][0] * d[1][1] - d[0][1] * d[1][0]
        if abs(det) < 1e-12:
            continue
        q = np.asarray(p[:2], float) - a[:2]
        u = (q[0] * d[1][1] - q[1] * d[1][0]) / det
        v = (d[0][0] * q[1] - d[0][1] * q[0]) / det
        if u < -1e-6 or v < -1e-6 or u + v > 1 + 1e-6:
            continue
        z = a[2] + u * (b - a)[2] + v * (c - a)[2]
        if p[2] + 0.02 < z <= p[2] + hoehe:
            return True
    return False


def _duenn(mat):
    """duenne Flaechen ohne Koerper (Gelaender, Glas, Laermschutz)"""
    return any(w in str(mat).lower() for w in ('stab', 'glas', 'lsw', 'gelaender'))


def _strich(mat):
    """Texturen, die absichtlich in einer Richtung gestreckt sind (Markierungen, Gelaender, Glas: Alpha-TGA)"""
    return str(mat).lower().endswith('.tga') or any(w in str(mat).lower() for w in ('marke', 'stab', 'glas', 'sperr', 'side'))


def _kanten_frei(P):
    """freie Kanten (nur an einem Dreieck, Ecken auf 1 mm verschweisst) -> Liste (a, b)"""
    Q = np.round(P / 0.001).astype(np.int64)
    zahl = defaultdict(int)
    wo = {}
    for t in range(len(P)):
        for i, j in ((0, 1), (1, 2), (2, 0)):
            a, b = tuple(Q[t, i]), tuple(Q[t, j])
            k = (a, b) if a < b else (b, a)
            zahl[k] += 1
            wo[k] = (P[t, i], P[t, j])
    return [wo[k] for k, n in zahl.items() if n == 1]


# ------------------------------------------------------------------ Prüfen
def gruben(raum, senken, hoehe=30.0):
    """abgesenkte Gelaendepunkte (kx, ky, spalte, zeile, z), ueber denen kein Teil liegt: dort sieht man ein Loch"""
    funde = []
    for pkt in senken:
        kx, ky, sp, ze, z = pkt[:5]
        if len(pkt) > 5 and pkt[5] - z < 0.3:
            continue                                    # kaum abgesenkt: kein sichtbares Loch
        x, y = kx * 300 + sp * 5.0, ky * 300 + ze * 5.0
        b_ = raum.boden(x, y, z + hoehe)
        if b_ is None or b_ < z - 0.15:
            funde.append(dict(art='grube', text='abgesenktes Gelaende ohne Abdeckung', pos=np.array([x, y, z]), teil='Gelaende'))
    return funde


def pruefen(teile, gelaende=None, signale=(), senken=()):
    """-> Liste Funde dict(art, text, pos, teil)"""
    funde = []
    raum = Raum(teile)
    for ti, t in enumerate(teile):
        if not len(t.P):
            continue
        s1, s2, fl = _streckung(t.P, t.UV)
        mitte = t.P.mean(axis=1)
        entartet = fl < 1e-5
        for k in np.where(entartet)[0]:
            funde.append(dict(art='dreieck_entartet', text='Dreieck ohne Flaeche', pos=mitte[k], teil=t.name))
        # schmale Streifen (Bordstein, Kappe, Markierung): Textur quer absichtlich gestaucht
        kante = np.max([np.linalg.norm(t.P[:, i] - t.P[:, (i + 1) % 3], axis=1) for i in range(3)], axis=0)
        schmal = 2 * fl / np.maximum(kante, 1e-9) < 0.4
        strich = np.array([_strich(m) for m in t.mat])
        ok = ~entartet & (fl > 0.01) & ~schmal & ~strich
        verschmiert = ok & (s2 < 1e-4 * np.maximum(s1, 1e-9)) & (s1 > 1e-6)
        gestreckt = ok & ~verschmiert & (s1 > ANISO * np.maximum(s2, 1e-12))
        for k in np.where(verschmiert | gestreckt)[0]:
            if verdeckt(raum, gelaende, mitte[k], ausser=ti):
                verschmiert[k] = gestreckt[k] = False
        for k in np.where(verschmiert)[0]:
            funde.append(dict(art='textur_verzerrt', text=f'Textur verschmiert ({t.mat[k]})', pos=mitte[k], teil=t.name))
        for k in np.where(gestreckt)[0]:
            funde.append(dict(art='textur_verzerrt', text=f'Textur {s1[k] / s2[k]:.0f}-fach gestreckt ({t.mat[k]})', pos=mitte[k], teil=t.name))
        # Spalte: freie Objektkanten knapp neben anderen Teilen
        if t.art == 'objekt':
            for a, b in _kanten_frei(t.P):
                if np.linalg.norm(b - a) < 0.05:
                    continue
                m = (a + b) / 2
                d = raum.abstand(m, SPALT[1], ausser=ti)
                # liegt die Kante auf einer eigenen Flaeche auf (z. B. Bordstein-Unterkante auf der Fahrbahn), ist sie
                # angeschlossen: eigene Dreiecke, die keine Ecke mit der Kante teilen
                if SPALT[0] < d < SPALT[1]:
                    d = min(d, raum.abstand_eigen(m, a, b, ti))
                if SPALT[0] < d < SPALT[1] and not verdeckt(raum, gelaende, m, ausser=ti):
                    funde.append(dict(art='spalt', text=f'Spalt {d * 100:.0f} cm zum Nachbarteil', pos=m, teil=t.name))
        # offene Kettenenden
        for q, wo in (t.enden if t.art == 'wand' else ()):
            sicht = [p for p in q if gelaende is None or gelaende(p[0], p[1]) is None or p[2] > gelaende(p[0], p[1]) + 0.05]
            if not sicht:
                continue
            d = max(raum.abstand(p, 1.0, ausser=ti) for p in sicht)
            if d > ENDE_FREI:
                hoch = max(p[2] - (gelaende(p[0], p[1]) or p[2]) for p in sicht) if gelaende else 0.0
                funde.append(dict(art='offenes_ende', text=f'{wo} frei ({hoch:.1f} m ueber Gelaende, {min(d, 9.9):.1f} m zum naechsten Teil)',
                                  pos=np.mean(sicht, axis=0), teil=t.name))
    # doppelte Flächen: nach oben bzw. seitlich zeigende Dreiecke verschiedener Teile/Materialien in derselben Ebene
    funde += [f for f in _doppelt(teile, raum) if not verdeckt(raum, gelaende, f['pos'], koerper=False)]
    funde += gruben(raum, senken)
    # Ampeln
    for i, s in enumerate(signale):
        if 'x' not in s or s.get('z_boden') is None:
            continue
        # Signalkopf am Mast: es zaehlt der Mastfuss (der Kopf darf ueber die Bordkante haengen)
        if s.get('art') == 'signal' and any(m.get('art') == 'mast' and 'x' in m and math.hypot(m['x'] - s['x'], m['y'] - s['y']) < 1.0 for m in signale):
            mast_da = True
        else:
            mast_da = False
        boden = raum.boden(s['x'], s['y'], s['z_boden'] + 0.5)
        if boden is None and gelaende is not None:
            boden = gelaende(s['x'], s['y'])
        if boden is None:
            continue
        dz = s['z_boden'] - boden
        name = os.path.basename(s.get('datei', '?'))
        if dz > BODEN_TOL and not mast_da:
            funde.append(dict(art='ampel_schwebt', text=f'{name} schwebt {dz:.2f} m', pos=np.array([s['x'], s['y'], boden]), teil='Kreuzung'))
        elif dz < -BODEN_TOL and not mast_da:
            funde.append(dict(art='ampel_versinkt', text=f'{name} steckt {-dz:.2f} m im Boden', pos=np.array([s['x'], s['y'], boden]), teil='Kreuzung'))
        if s.get('art') == 'signal' and s.get('hoehe', 0) > 0.6:
            mast = any(m.get('art') == 'mast' and math.hypot(m['x'] - s['x'], m['y'] - s['y']) < 1.0 for m in signale if 'x' in m)
            if not mast:
                funde.append(dict(art='ampel_ohne_mast', text=f'{name} {s["hoehe"]:.1f} m hoch ohne Mast', pos=np.array([s['x'], s['y'], s['z_boden']]), teil='Kreuzung'))
    return funde


def _doppelt(teile, raum, ebene=0.006, mind=0.05):
    from shapely.geometry import Polygon
    funde = []
    P = raum.P
    if not len(P):
        return funde
    mat = np.concatenate([np.array(t.mat, object) for t in teile if len(t.P)])
    n = np.cross(P[:, 1] - P[:, 0], P[:, 2] - P[:, 0])
    fl = np.linalg.norm(n, axis=1)
    gut = fl > 0.02
    n = n / np.maximum(fl, 1e-12)[:, None]
    d = np.einsum('ij,ij->i', n, P[:, 0])
    gesehen = set()
    for k in np.where(gut)[0]:
        c = P[k].mean(axis=0)
        kand = raum.nahe(c, 0.5)
        kand = kand[(kand > k) & gut[kand]]
        if not len(kand):
            continue
        gleich = (np.einsum('ij,j->i', n[kand], n[k]) > 0.999) & (np.abs(d[kand] - d[k]) < ebene)
        gleich &= (raum.teil[kand] != raum.teil[k]) | (mat[kand] != mat[k])
        for j in kand[gleich]:
            # Überlappung in der gemeinsamen Ebene
            ax = np.argmax(np.abs(n[k]))
            sel = [i for i in range(3) if i != ax]
            a, b = Polygon(P[k][:, sel]), Polygon(P[j][:, sel])
            if not (a.is_valid and b.is_valid):
                continue
            ue = a.intersection(b).area
            if ue > mind:
                schl = (int(c[0] // GRUPPE), int(c[1] // GRUPPE), int(c[2] // GRUPPE))
                if schl in gesehen:
                    continue
                gesehen.add(schl)
                ta, tb = teile_name(teile, raum.teil[k]), teile_name(teile, raum.teil[j])
                funde.append(dict(art='doppelte_flaeche', text=f'deckungsgleiche Flaechen {ta} ({mat[k]}) / {tb} ({mat[j]})', pos=c, teil=ta))
    return funde


def teile_name(teile, i):
    return teile[int(i)].name


def zusammenfassen(funde):
    """gleiche Art und gleiches Teil im Umkreis GRUPPE zu einem Fund (mit Anzahl)"""
    out = []
    for f in funde:
        p = np.asarray(f['pos'], float)
        for g in out:
            if g['art'] == f['art'] and g['teil'] == f['teil'] and np.linalg.norm(np.asarray(g['pos']) - p) < GRUPPE:
                g['anzahl'] += 1
                break
        else:
            out.append(dict(f, pos=p, anzahl=1))
    for g in out:
        g['pos'] = [round(float(v), 2) for v in g['pos']]
    return out


# ------------------------------------------------------------------ Sonderbau
def gelaende_funktion(omsi, karte, senken=(), zurueck=()):
    from .sonderbau_ring import Kartengelaende
    kg = Kartengelaende(os.path.join(omsi, 'maps', karte))
    for kx, ky, sp, ze, z in zurueck:
        a = kg.kachel(kx, ky)
        if a is not None:
            a[ze, sp] = max(a[ze, sp], z)
    for kx, ky, sp, ze, z, *_ in senken:
        a = kg.kachel(kx, ky)
        if a is not None:
            a[ze, sp] = min(a[ze, sp], z)
    return kg.hoehe


def sonderbau_pruefen(json_pfad, omsi=STANDARD_OMSI):
    j = json.load(open(json_pfad, encoding='utf-8'))
    teile = []
    for o in j.get('objekte', []):
        teile.append(objekt_teil(omsi, o['rel'], o['x'], o['y'], o.get('z', 0.0), o.get('rot', 0.0)))
    for k in j.get('kreuzungen', []):
        teile.append(objekt_teil(omsi, k['rel'], k['ursprung'][0], k['ursprung'][1], 0.0))
    sli_ordner = os.path.join(omsi, 'Splines', 'Aschaffenburg')
    for st in j.get('strassen', []):
        if not st.get('elemente'):
            continue
        rel = st.get('sli')
        if not rel:
            name = st.get('querschnitt', {}).get('name', st['name'])
            datei = 'AB_' + re.sub(r'[^A-Za-z0-9_]+', '_', name) + '.sli'
            if not os.path.isfile(os.path.join(sli_ordner, datei)):
                continue
            rel = os.path.join('Splines', 'Aschaffenburg', datei)
        t = spline_teil(omsi, st['name'], rel, st['elemente'])
        if st.get('sli'):
            t.art = 'wand'
        teile.append(t)
    signale = [s for k in j.get('kreuzungen', []) for s in k.get('signale', [])]
    gel = gelaende_funktion(omsi, j['karte'], j.get('gelaende', []), j.get('gelaende_zurueck', []))
    funde = zusammenfassen(pruefen(teile, gel, signale, j.get('gelaende', [])))
    zaehl = defaultdict(int)
    for f in funde:
        zaehl[f['art']] += 1
    bericht = dict(teile=len(teile), dreiecke=int(sum(len(t.P) for t in teile)), arten=dict(zaehl), funde=funde)
    with open(os.path.join(os.path.dirname(json_pfad), 'baupruefung.json'), 'w', encoding='utf-8') as f:
        json.dump(bericht, f, indent=1, ensure_ascii=False)
    return bericht


def text(bericht):
    z = [f"Baupruefung: {bericht['teile']} Teile, {bericht['dreiecke']} Dreiecke, {len(bericht['funde'])} Funde"]
    for art, n in sorted(bericht['arten'].items(), key=lambda x: -x[1]):
        z.append(f'  {art:18} {n}')
    return '\n'.join(z)


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('json', help='sonderbau_ring.json (oder gleich aufgebaute Datei)')
    ap.add_argument('--omsi', default=STANDARD_OMSI)
    ap.add_argument('--liste', action='store_true', help='alle Funde ausgeben')
    a = ap.parse_args()
    b = sonderbau_pruefen(a.json, a.omsi)
    print(text(b))
    if a.liste:
        for f in b['funde']:
            print(f"  {f['art']:18} {f['teil']:28} {f['pos']}  {f['text']}" + (f" (x{f['anzahl']})" if f['anzahl'] > 1 else ''))


if __name__ == '__main__':
    main()
