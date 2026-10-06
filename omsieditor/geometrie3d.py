"""Geometrie fuer die 3D-Ansicht, ohne OpenGL (testbar): aus dem Ergebnis von pipeline.berechne() entstehen
Dreieckslisten je Textur - Strassen aus den echten Spline-Profilen (.sli), Kreuzungsplatten und Bauwerke aus
ihren Meshes, Gelaende aus dem Kachelraster (Hoehen.raster, also mit Angleichung unter Strassen und Tunnelgraeben).
So zeigt der Editor dieselbe Geometrie, die OMSI bekommt.

Koordinaten: Welt (x Ost, y Hoehe relativ zur Basis, z Nord). Ein Eckpunkt = 8 float32: x, y, z, nx, ny, nz, u, v;
u, v wie in OMSI/DirectX (v = 0 oben im Bild).
Texturschluessel: Dateipfad der Textur oder 'farbe:#rrggbb'."""
import math, os
import numpy as np
from omsigen.geom import end_of, rvec
from omsigen.config import KI
from omsigen.custom_splines import SPLINES as CUSTOM
from omsigen.splinedb import read_text

TILE = 300.0
N = 61                      # Rasterpunkte je Kachelkante (5 m)
FARBE_FAHRBAHN = 'farbe:#55585c'
FARBE_GEHWEG = 'farbe:#a9a9a4'
FARBE_BETON = 'farbe:#9a9890'


class Szene(dict):
    """Texturschluessel -> Liste von Arrays (n, 8); fertig() fasst zusammen"""

    def add(self, key, arr):
        if len(arr):
            self.setdefault(key, []).append(np.asarray(arr, dtype=np.float32).reshape(-1, 8))

    def fertig(self):
        return {k: np.concatenate(v) for k, v in self.items() if v}

    def dreiecke(self):
        return sum(len(a) for v in self.values() for a in v) // 3


# ------------------------------------------------------------------ Texturen
def textur_ordner(omsi):
    if not omsi:
        return []
    return [os.path.join(omsi, 'Splines', 'Aschaffenburg_KI', 'texture'), os.path.join(omsi, 'Splines', 'Marcel', 'texture'),
            os.path.join(omsi, 'Texture')]


def textur_finden(name, ordner):
    for d in ordner:
        p = os.path.join(d, name)
        if os.path.exists(p):
            return p
    return None


# ------------------------------------------------------------------ Splines
def sli_lesen(text):
    """-> dict(texturen=[Dateiname], profile=[(Texturindex, [(x, y, u, v)])])"""
    L = [l.strip() for l in text.replace('\r', '').split('\n')]
    tex, prof = [], []
    i = 0
    while i < len(L):
        if L[i] == '[texture]' and i + 1 < len(L):
            tex.append(L[i + 1]); i += 2; continue
        if L[i] == '[profile]' and i + 1 < len(L):
            try:
                prof.append((int(L[i + 1]), []))
            except ValueError:
                pass
            i += 2; continue
        if L[i] == '[profilepnt]' and prof and i + 4 < len(L):
            try:
                prof[-1][1].append(tuple(float(L[i + k].replace(',', '.')) for k in range(1, 5)))
            except ValueError:
                pass
            i += 5; continue
        i += 1
    return dict(texturen=tex, profile=[p for p in prof if len(p[1]) >= 2])


class Splines:
    """Spline-Pfad (wie in der Karte) -> gelesene Profile und aufgeloeste Texturpfade (zwischengespeichert)"""

    def __init__(self, omsi=None, extra=None):
        self.omsi, self.extra = omsi, dict(extra or {})
        self._cache = {}

    def __getitem__(self, rel):
        if rel in self._cache:
            return self._cache[rel]
        text, ordner = None, []
        name = rel.split('\\')[-1]
        if rel.startswith(KI) and (name in self.extra or name in CUSTOM):
            text = self.extra[name] if name in self.extra else CUSTOM[name][0]
            ordner = textur_ordner(self.omsi)
        elif self.omsi:
            p = os.path.join(self.omsi, *rel.split('\\'))
            if os.path.exists(p):
                text = read_text(p)
                ordner = [os.path.join(os.path.dirname(p), 'texture')] + textur_ordner(self.omsi)
        d = sli_lesen(text) if text else dict(texturen=[], profile=[])
        d['pfade'] = [textur_finden(t, ordner) for t in d['texturen']]
        self._cache[rel] = d
        return d


def _farbe_fuer(profil_pts):
    """Ersatzfarbe ohne Textur: hohe Profile = Gehweg, sonst Fahrbahn"""
    return FARBE_GEHWEG if min(p[1] for p in profil_pts) > 0.18 else FARBE_FAHRBAHN


def spline_dreiecke(szene, el, y0, g, sli, laenge0=0.0, schritt=2.0):
    """ein Spline-Element (x, z, h, L, R, datei) mit Starthoehe y0 und Steigung g (%) entlang seiner Profile"""
    x, z, h, L, R = el[:5]
    if not sli['profile'] or L <= 0:
        return
    n = 1 if R == 0 else max(2, int(math.ceil(L / schritt)))
    S = []
    for k in range(n + 1):
        t = L * k / n
        (px, pz), hh = end_of([x, z, h, t, R])
        rx, rz = rvec(hh)
        S.append((px, pz, rx, rz, y0 + g / 100 * t, laenge0 + t))
    S = np.array(S)
    for tex, pts in sli['profile']:
        name = sli['texturen'][tex] if tex < len(sli['texturen']) else ''
        if name.lower().startswith('helper'):              # Hilfstexturen zeigt OMSI nur im Editor
            continue
        key = (sli['pfade'][tex] if tex < len(sli['pfade']) else None) or _farbe_fuer(pts)
        P = np.array(pts)                                   # (m, 4): x, y, u, v
        # Eckpunkte: Probe k, Profilpunkt j
        X = S[:, None, 0] + P[None, :, 0] * S[:, None, 2]
        Z = S[:, None, 1] + P[None, :, 0] * S[:, None, 3]
        Y = S[:, None, 4] + P[None, :, 1]
        U = np.broadcast_to(P[None, :, 2], X.shape)
        V = P[None, :, 3] * S[:, None, 5]
        # Normale je Profilstrecke: (-dy) nach rechts + dx nach oben
        d = np.diff(P[:, :2], axis=0)
        ln = np.maximum(np.hypot(d[:, 0], d[:, 1]), 1e-9)
        nq, nh = -d[:, 1] / ln, d[:, 0] / ln               # quer (rechts), hoch
        tris = []
        for j in range(len(P) - 1):
            nx = nq[j] * S[:, 2]; nz = nq[j] * S[:, 3]; ny = np.full(len(S), nh[j])
            a = np.stack([X[:, j], Y[:, j], Z[:, j], nx, ny, nz, U[:, j], V[:, j]], 1)
            b = np.stack([X[:, j + 1], Y[:, j + 1], Z[:, j + 1], nx, ny, nz, U[:, j + 1], V[:, j + 1]], 1)
            tris.append(np.stack([a[:-1], b[:-1], b[1:], a[:-1], b[1:], a[1:]], 1).reshape(-1, 8))
        szene.add(key, np.concatenate(tris))


def ketten_dreiecke(szene, chains, splines):
    for c in chains:
        cum = 0.0
        for i, el in enumerate(c['els']):
            y = c['y'][i] if 'y' in c else 0.0
            g = c['g'][i] if 'g' in c else 0.0
            spline_dreiecke(szene, el, y, g, splines[el[5]], cum)
            cum += el[3]


# ------------------------------------------------------------------ Objekte
def objekt_dreiecke(szene, obj, ordner):
    """Objekt mit mesh=(V, F) und mats=[(Name, Textur)] an origin, Hoehe hoehe ([absheight]), Drehung rot"""
    if 'mesh' not in obj:
        return
    V, F = obj['mesh']
    if not F:
        return
    A = np.array(V, dtype=np.float64)
    r = math.radians(obj.get('rot', 0.0))
    c, s = math.cos(r), math.sin(r)
    ox, oz = obj['origin']
    W = np.empty_like(A)
    W[:, 0] = ox + A[:, 0] * c + A[:, 2] * s                # lokal x = rechts, z = vorwaerts (Richtung rot)
    W[:, 2] = oz - A[:, 0] * s + A[:, 2] * c
    W[:, 1] = A[:, 1] + obj.get('hoehe', 0.0)
    W[:, 3] = A[:, 3] * c + A[:, 5] * s
    W[:, 5] = -A[:, 3] * s + A[:, 5] * c
    W[:, 4] = A[:, 4]
    W[:, 6] = A[:, 6]
    W[:, 7] = -A[:, 7]                                      # wie x_file: v nach DirectX (0 = oben)
    mats = obj.get('mats') or [('m', None)]
    for m, (_, tex) in enumerate(mats):
        idx = [i for f in F if f[3] == m for i in f[:3]]
        if idx:
            key = (textur_finden(tex, ordner) if tex else None) or (FARBE_BETON if 'beton' in str(tex) else
                                                                     FARBE_GEHWEG if m else FARBE_FAHRBAHN)
            szene.add(key, W[idx])


# ------------------------------------------------------------------ Gelaende
class Raster:
    """Gelaendehoehen in OMSI-Kacheln (61 x 61, 5 m) ab Welt-Ecke (ox, oz); hoehe() bilinear fuer Klicks"""

    def __init__(self, ox, oz):
        self.ox, self.oz = ox, oz
        self.kacheln = {}              # (tx, tz) -> np.array (61, 61), Zeile 0 = Sueden

    def kachel_ecke(self, t):
        return self.ox + TILE * t[0], self.oz + TILE * t[1]

    def hoehe(self, x, z):
        tx, tz = math.floor((x - self.ox) / TILE), math.floor((z - self.oz) / TILE)
        H = self.kacheln.get((tx, tz))
        if H is None:
            return None
        fx = (x - self.ox - TILE * tx) / 5.0
        fz = (z - self.oz - TILE * tz) / 5.0
        i, j = min(int(fx), N - 2), min(int(fz), N - 2)
        u, v = fx - i, fz - j
        return float((H[j, i] * (1 - u) + H[j, i + 1] * u) * (1 - v) + (H[j + 1, i] * (1 - u) + H[j + 1, i + 1] * u) * v)

    def dreiecke(self, t):
        """eine Kachel -> Array (n, 8); u, v laufen 0..1 ueber die Kachel (Luftbild je Kachel)"""
        H = self.kacheln[t]
        x0, z0 = self.kachel_ecke(t)
        g = np.arange(N) * 5.0
        X, Z = np.meshgrid(x0 + g, z0 + g)
        # Normalen aus den Nachbarn
        gx = np.gradient(H, 5.0, axis=1); gz = np.gradient(H, 5.0, axis=0)
        n = np.stack([-gx, np.ones_like(H), -gz], -1)
        n /= np.linalg.norm(n, axis=-1, keepdims=True)
        U, Vv = np.meshgrid(g / TILE, 1.0 - g / TILE)      # Bild: Zeile 0 = Norden (DirectX: v = 0 oben)
        E = np.concatenate([np.stack([X, H, Z], -1), n, np.stack([U, Vv], -1)], -1)   # (61, 61, 8)
        a, b, c, d = E[:-1, :-1], E[:-1, 1:], E[1:, 1:], E[1:, :-1]
        T = np.stack([a, b, c, a, c, d], 2)                  # (60, 60, 6, 8)
        return T.reshape(-1, 8).astype(np.float32)


def kacheln_fuer(punkte, ox, oz, rand=1):
    """OMSI-Kacheln, die die Punkte enthalten, plus Nachbarn (wie write_map)"""
    used = {(math.floor((x - ox) / TILE), math.floor((z - oz) / TILE)) for x, z in punkte}
    return sorted({(a + dx, b + dz) for a, b in used for dx in range(-rand, rand + 1) for dz in range(-rand, rand + 1)
                   if a + dx >= 0 and b + dz >= 0})


def roh_raster(raster, kacheln, gel, basis):
    """Gelaende direkt aus dem DGM (vor der ersten Vorschau): Hoehe relativ zu basis"""
    for t in kacheln:
        x0, z0 = raster.kachel_ecke(t)
        H = np.zeros((N, N), dtype=np.float32)
        for j in range(N):
            for i in range(N):
                h = gel.hoehe(x0 + 5 * i, z0 + 5 * j) if gel else None
                H[j, i] = 0.0 if h is None else h - basis
        raster.kacheln[t] = H


# ------------------------------------------------------------------ alles zusammen
def szene_aus_berechnung(B, omsi=None):
    """pipeline.berechne(...) -> (Szene der Bauten, Raster des Gelaendes, Basis ue. NN)"""
    from omsigen.writer import karten_ursprung
    szene = Szene()
    splines = Splines(omsi, B['bw']['splines'])
    ketten_dreiecke(szene, B['chains'], splines)
    ordner = textur_ordner(omsi)
    for o in B['objekte']:
        objekt_dreiecke(szene, o, ordner)
    ox, oz = karten_ursprung(B['chains'], B['stops'], B['objekte'])
    raster = Raster(ox, oz)
    pts = [(el[0], el[1]) for c in B['chains'] for el in c['els']]
    for t in kacheln_fuer(pts, ox, oz):
        x0, z0 = raster.kachel_ecke(t)
        if B['hat_gelaende']:
            raster.kacheln[t] = B['hoe'].raster(x0, z0).astype(np.float32)
        else:
            raster.kacheln[t] = np.zeros((N, N), dtype=np.float32)
    return szene, raster, B['hoe'].base
