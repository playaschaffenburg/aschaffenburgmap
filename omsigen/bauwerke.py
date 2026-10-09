"""Bauwerke an Bruecken und Tunneln (nach dem Hoehenausgleich, hoehen.py), gebaut wie in den Standardkarten
(Studie in docs/omsi-format.md, „Wie die Standardkarten Bruecken und Tunnel bauen“):

- Brueckenkoerper und Tunnelroehren sind Begleit-Splines ohne Pfade, die genau auf den Fahrbahn-Splines liegen
  (gleiche Lage, Hoehe, Steigung). So bleiben die Spuranschluesse der Fahrbahn unberuehrt.
  Bruecke: Platte 1,2 m dick, Bruestung 1 m hoch; Pfeiler (etwa alle 30 m) als Objekte auf dem Gelaende.
- Tunnel: Gelaende wird nie ausgeschnitten (Loecher zeigen im Spiel verzerrte Bodentextur), sondern zwischen den
  Portalen als Graben bis auf die Sohle abgesenkt (hoe.graeben -> Hoehen.raster), wie in Gladbeck. Darueber liegt
  ein Deckel-Objekt mit Bodentextur auf der Gelaendehoehe, an jedem Portal eine Portalwand so breit wie der Deckel.
- Alle Objekte mit [absheight] in Sceneryobjects\\Aschaffenburg\\<Karte>\\.
"""
import math
import numpy as np
from .geom import end_of, rvec
from .config import KI
from .kreuzung import x_file
from .network import ebene

TEX = 'betonwand1.bmp'           # Beton aus Splines\Marcel\texture
GRAS = 'gras.bmp'                # Bodentextur der Karte (OMSI\Texture, global.cfg [groundtex], 1x je Kachel)
PLATTE = 1.2                     # m Brueckenplatte unter der Fahrbahn
BRUESTUNG = 1.0                  # m ueber dem Gehweg
GEHWEG_H = 0.25
PFEILER_ABSTAND = 30.0
PFEILER_DICKE = 1.2
PFEILER_MIN = 1.5                # m Luft unter der Platte, darunter keine Pfeiler
TUNNEL_H = 5.0                   # Lichte Hoehe (Decke) ueber der Spline-Hoehe
WAND = 0.5                       # Wand-/Deckendicke
SOHLE = 0.15                     # Grabensohle so weit unter der Fahrbahn
GRABEN_RAND = 7.5                # Graben reicht so weit ueber die Wand hinaus (> Rasterdiagonale 7,07 m, sonst
                                 # ragen Gelaendedreiecke mit einer hohen Ecke in die Roehre)
DECKEL_RAND = 7.5                # Deckel reicht so weit ueber den Graben (> Rasterdiagonale 7,07 m)
DECKEL_SCHRITT = 4.0             # m Laengsraster des Deckels
DECKEL_STUECK = 60.0             # m je Deckel-Objekt


def _r(v):
    """auf 0,5 m aufrunden (Betrag), damit wenige Spline-Dateien entstehen"""
    return math.ceil(abs(v) * 2 - 1e-6) / 2


def _n(v):
    return f'{v:.1f}'.replace('.', '_')


def _prof(pts, vrate=0.1):
    out = ['[profile]', '0', '']
    for x, y, u in pts:
        out += ['[profilepnt]', f'{x:.3f}', f'{y:.3f}', f'{u:.3f}', f'{vrate:.3f}', '']
    return out


def _kopf():
    return ['File created with omsigen (Aschaffenburg-KI). Begleit-Spline ohne Pfade.', '', '[texture]', TEX, '']


def bruecke_sli(l, r):
    """Brueckenkoerper zur Fahrbahn mit Aussenkanten l (< 0) und r (> 0). Profilpunkte von links nach rechts
    gelesen: die Flaeche ist auf der linken Seite der Laufrichtung sichtbar (wie in rail_boxcut_concrete)."""
    a, b = l - 0.3, r + 0.3
    oben, unten = GEHWEG_H + BRUESTUNG, -PLATTE
    L = _kopf()
    L += _prof([(a, unten, 0.0), (a, oben, (oben - unten) / 4)])           # linke Aussenseite
    L += _prof([(a, oben, 0.0), (l, oben, 0.08)])                          # Bruestung oben
    L += _prof([(l, oben, 0.0), (l, GEHWEG_H, BRUESTUNG / 4)])             # Bruestung innen
    L += _prof([(r, GEHWEG_H, 0.0), (r, oben, BRUESTUNG / 4)])
    L += _prof([(r, oben, 0.0), (b, oben, 0.08)])
    L += _prof([(b, oben, 0.0), (b, unten, (oben - unten) / 4)])           # rechte Aussenseite
    L += _prof([(b, unten, 0.0), (a, unten, (b - a) / 4)])                 # Unterseite
    return '\r\n'.join(L) + '\r\n'


def tunnel_sli(l, r):
    """Tunnelroehre von innen: Waende bei l/r (bis unter die Grabensohle), Decke TUNNEL_H. Von aussen ist sie
    nie zu sehen (Deckel und Portale schliessen den Graben)."""
    L = _kopf()
    L += _prof([(l, TUNNEL_H, 0.0), (l, -SOHLE - 0.3, TUNNEL_H / 4)])      # linke Wand innen
    L += _prof([(r, TUNNEL_H, 0.0), (l, TUNNEL_H, (r - l) / 4)])           # Decke von unten
    L += _prof([(r, -SOHLE - 0.3, 0.0), (r, TUNNEL_H, TUNNEL_H / 4)])      # rechte Wand innen
    return '\r\n'.join(L) + '\r\n'


# ------------------------------------------------------------------ Objekte
def _quader(V, F, x0, x1, y0, y1, z0, z1):
    """Quader (Objektkoordinaten x rechts, y hoch, z vorwaerts); jede Seite beidseitig, Beton in 4-m-Kacheln"""
    P = [(x, y, z) for x in (x0, x1) for y in (y0, y1) for z in (z0, z1)]
    seiten = [((0, 1, 3, 2), (-1, 0, 0)), ((4, 6, 7, 5), (1, 0, 0)), ((0, 4, 5, 1), (0, -1, 0)),
              ((2, 3, 7, 6), (0, 1, 0)), ((0, 2, 6, 4), (0, 0, -1)), ((1, 5, 7, 3), (0, 0, 1))]
    for idx, n in seiten:
        q = [P[i] for i in idx]
        ax = [i for i in range(3) if n[i] == 0]
        _viereck(V, F, q, n, [(p[ax[0]] / 4, p[ax[1]] / 4) for p in q], 0)


def _viereck(V, F, q, n, uv, mat):
    """Viereck q (4 Punkte) beidseitig"""
    for sign, ordn in ((1, (0, 1, 2, 0, 2, 3)), (-1, (0, 2, 1, 0, 3, 2))):
        i0 = len(V)
        for p, t in zip(q, uv):
            V.append(tuple(p) + tuple(sign * c for c in n) + tuple(t))
        for k in range(0, 6, 3):
            F.append((i0 + ordn[k], i0 + ordn[k + 1], i0 + ordn[k + 2], mat))


def _objekt(name, map_name, V, F, x, z, hoehe, rot, art, mats=((('Beton', TEX)),)):
    sco = '\r\n'.join(['Erzeugt mit omsigen (Aschaffenburg-KI).', '', '[friendlyname]', f'{art} {name}', '',
                       '[groups]', '1', 'Aschaffenburg', '', '[fixed]', '', '[absheight]', '',
                       '[mesh]', name + '.x', '']) + '\r\n'
    return dict(name=name, origin=(x, z), hoehe=hoehe, rot=rot % 360, sco=sco, art=art, mats=list(mats),
                x=x_file(V, F, list(mats)), faces=len(F), paths=0, mesh=(V, F),
                rel=f'Sceneryobjects\\Aschaffenburg\\{map_name}\\{name}.sco')


# ------------------------------------------------------------------ Laeufe und Aufbau
def _punkt(el, t):
    """Punkt, Richtung bei Laenge t im Element"""
    return end_of([el[0], el[1], el[2], t, el[4]])


def _laeufe(ebene_):
    """-> [(art, i0, i1)] zusammenhaengende Bruecken-/Tunnelabschnitte (Elementindizes, i1 exklusiv)"""
    out, i = [], 0
    while i < len(ebene_):
        if ebene_[i]:
            j = i
            while j < len(ebene_) and ebene_[j] == ebene_[i]:
                j += 1
            out.append((ebene_[i], i, j))
            i = j
        else:
            i += 1
    return out


def _kanten(els, sdb):
    l, r = -7.0, 7.0
    try:
        l = min(sdb[e[5]]['ol'] for e in els)
        r = max(sdb[e[5]]['or_'] for e in els)
    except KeyError:
        pass
    return -_r(l), _r(r)


def _deckel(an, laenge, l, r, hoe, versatz):
    """Deckel ueber dem Tunnelgraben in Stuecken: -> [(Ursprung, V, F)]. Hoehe = Gelaende ohne Graben
    (hoe.oberflaeche), ueber der Roehre mindestens Decke + 0,3 m; Rand liegt auf unveraendertem Gelaende."""
    innen = max(-l, r) + WAND
    M = innen + GRABEN_RAND + DECKEL_RAND
    quer = np.linspace(-M, M, max(5, int(math.ceil(2 * M / 3.0)) + 1))
    n = max(1, int(math.ceil(laenge / DECKEL_SCHRITT)))
    reihen = []
    for k in range(n + 1):
        s = laenge * k / n
        p, h, y = an(s)
        rv = rvec(h)
        reihe = []
        for o in quer:
            q = (p[0] + o * rv[0], p[1] + o * rv[1])
            hh = hoe.oberflaeche(*q)
            if abs(o) <= innen + 0.5:
                hh = max(hh, y + TUNNEL_H + WAND + 0.3)
            reihe.append((q[0], hh + (0.1 if abs(o) >= M - 1e-6 else 0.05) + versatz, q[1]))
        reihen.append((s, reihe))
    stuecke, k0 = [], 0
    while k0 < n:
        k1 = k0
        while k1 < n and reihen[k1 + 1][0] - reihen[k0][0] <= DECKEL_STUECK + 1e-6:
            k1 += 1
        k1 = max(k1, k0 + 1)
        ox, _, oz = reihen[k0][1][len(quer) // 2]
        V, F = [], []
        for a, b in zip(reihen[k0:k1], reihen[k0 + 1:k1 + 1]):
            for j in range(len(quer) - 1):
                q = [a[1][j], a[1][j + 1], b[1][j + 1], b[1][j]]
                loc = [(x - ox, y, z - oz) for x, y, z in q]
                _viereck(V, F, loc, (0, 1, 0), [(x / 300, z / 300) for x, _, z in q], 0)
        stuecke.append(((ox, oz), V, F))
        k0 = k1
    return stuecke, M


def bauen(net, sdb, hoe, map_name):
    """-> dict(splines={datei: text}, ketten=[Begleitketten mit els/y/g], objekte=[...], stats);
    setzt hoe.graeben (Tunnelgraeben als Strecken fuer Hoehen.raster)"""
    splines, ketten, objekte = {}, [], []
    st = dict(bruecken=0, tunnel=0, pfeiler=0, portale=0, deckel=0)
    mit_gelaende = bool(hoe and hoe.gel)
    gel = hoe.gelaende if mit_gelaende else (lambda x, z: None)
    hoe.graeben = []
    for ch in net['road_chains']:
        els, eb = ch['els'], ch.get('ebene') or [None] * len(ch['els'])
        ys, gs = ch.get('y') or [0.0] * len(els), ch.get('g') or [0.0] * len(els)
        for art, i0, i1 in _laeufe(eb):
            run = els[i0:i1]
            l, r = _kanten(run, sdb)
            if art == 'bruecke':
                datei = f'AB_bruecke_{_n(-l)}_{_n(r)}.sli'
                splines.setdefault(datei, bruecke_sli(l, r))
                st['bruecken'] += 1
            else:
                datei = f'AB_tunnel_{_n(-l)}_{_n(r)}.sli'
                splines.setdefault(datei, tunnel_sli(l, r))
                st['tunnel'] += 1
            k = dict(els=[list(e[:5]) + [KI + datei] for e in run], y=ys[i0:i1], g=gs[i0:i1], begleit=art)
            s0 = [0.0]
            for e in run:
                s0.append(s0[-1] + e[3])
            laenge = s0[-1]

            def an(s):
                """Punkt, Richtung, Fahrbahnhoehe bei Weg s im Lauf"""
                i = next((j for j in range(len(run)) if s0[j + 1] >= s), len(run) - 1)
                t = min(max(s - s0[i], 0.0), run[i][3])
                p, h = _punkt(run[i], t)
                return p, h, k['y'][i] + k['g'][i] / 100 * t
            if art == 'bruecke':
                n = max(0, round(laenge / PFEILER_ABSTAND) - 1)
                for m in range(n):
                    p, h, y = an(laenge * (m + 1) / (n + 1))
                    c, w = (l + r) / 2, max(2.0, (r - l) * 0.6)
                    q = (p[0] + c * rvec(h)[0], p[1] + c * rvec(h)[1])
                    boden = gel(*q)
                    if boden is None or y - PLATTE - boden < PFEILER_MIN:
                        continue
                    V, F = [], []
                    _quader(V, F, -w / 2, w / 2, 0.0, y - PLATTE - (boden - 1.0) + 0.05,
                            -PFEILER_DICKE / 2, PFEILER_DICKE / 2)
                    objekte.append(_objekt(f'pfeiler_{len(objekte)}', map_name, V, F, q[0], q[1], boden - 1.0, h,
                                           'Pfeiler'))
                    st['pfeiler'] += 1
            elif mit_gelaende:
                innen = max(-l, r) + WAND
                # Graben: Strecken entlang der Roehre (nur zwischen den Portalebenen, siehe Hoehen.raster)
                n = max(1, int(math.ceil(laenge / 2.0)))
                pts = [an(laenge * i / n) for i in range(n + 1)]
                for (p1, _, y1), (p2, _, y2) in zip(pts, pts[1:]):
                    hoe.graeben.append((p1[0], p1[1], p2[0], p2[1], min(y1, y2) - SOHLE, innen + GRABEN_RAND))
                stuecke, M = _deckel(an, laenge, l, r, hoe, 0.02 * (st['tunnel'] % 3))
                for (ox, oz), V, F in stuecke:
                    objekte.append(_objekt(f'deckel_{len(objekte)}', map_name, V, F, ox, oz, 0.0, 0.0, 'Deckel',
                                           (('Boden', GRAS),)))
                    st['deckel'] += 1
                for ende, nb in (('anfang', i0 - 1), ('ende', i1)):
                    offen = 0 <= nb < len(els) and not eb[nb]
                    if not offen:           # Kettenende: Portal, wenn dort eine Strasse ausserhalb des Tunnels ankommt
                        knoten = ch.get('node_s') if ende == 'anfang' else ch.get('node_e')
                        offen = any(ebene(arm.get('tags', {})) != 'tunnel' for arm in net['arms'].get(knoten, ()))
                    if not offen:
                        continue            # Tunnel geht unter der Erde in eine Kreuzung o. ae. ueber
                    s = 0.0 if ende == 'anfang' else laenge
                    p, h, y = an(s)
                    rv = rvec(h)
                    oben = max(hoe.oberflaeche(p[0] + o * rv[0], p[1] + o * rv[1]) for o in np.linspace(-M, M, 9))
                    oben = max(oben - y + 0.4, TUNNEL_H + WAND + 0.5)
                    a, b = l, r
                    V, F = [], []
                    unten = -SOHLE - 1.0
                    _quader(V, F, -M, a, unten, oben, -0.5, 0.5)            # Portalwand links und rechts der
                    _quader(V, F, b, M, unten, oben, -0.5, 0.5)             # Oeffnung, bis an den Deckelrand
                    _quader(V, F, a, b, TUNNEL_H, oben, -0.5, 0.5)          # Sturz ueber der Oeffnung
                    objekte.append(_objekt(f'portal_{len(objekte)}', map_name, V, F, p[0], p[1], y, h, 'Portal'))
                    st['portale'] += 1
            ketten.append(k)
    return dict(splines=splines, ketten=ketten, objekte=objekte, stats=st)
