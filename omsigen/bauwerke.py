"""Bauwerke an Bruecken und Tunneln (nach dem Hoehenausgleich, hoehen.py):

- Brueckenkoerper und Tunnelroehren sind Begleit-Splines ohne Pfade, die genau auf den Fahrbahn-Splines liegen
  (gleiche Lage, Hoehe, Steigung). So bleiben die Spuranschluesse der Fahrbahn unberuehrt, wie bei den
  Bruecken-Splines der Standardkarten (z. B. Ruede ..._BUE.sli in Berlin-Spandau), nur ohne eigene Spurdaten.
  Bruecke: Platte 1,2 m dick, Bruestung 1 m hoch. Tunnel: Waende, Decke, Aussenhaut und [terrainholeprofile].
- Pfeiler (etwa alle 30 m) und Tunnelportale sind Objekte mit [absheight] in
  Sceneryobjects\\Aschaffenburg_KI\\<Karte>\\.
- Gelaendeloch ([spline_terrain_align] hinter dem Spline in der Kachel, Format wie Berlin-Spandau): Tunnelroehre auf
  den ersten Metern hinter jedem Portal und Fahrbahn in tiefen Einschnitten.
"""
import math
from .geom import end_of, rvec
from .config import KI
from .kreuzung import x_file
from .network import ebene

TEX = 'betonwand1.bmp'           # Beton aus Splines\Marcel\texture
PLATTE = 1.2                     # m Brueckenplatte unter der Fahrbahn
BRUESTUNG = 1.0                  # m ueber dem Gehweg
GEHWEG_H = 0.25
PFEILER_ABSTAND = 30.0
PFEILER_DICKE = 1.2
PFEILER_MIN = 1.5                # m Luft unter der Platte, darunter keine Pfeiler
TUNNEL_H = 5.0                   # Lichte Hoehe (Decke) ueber der Spline-Hoehe
WAND = 0.5                       # Wand-/Deckendicke
LOCH_LAENGE = 15.0               # m Geländeloch hinter dem Portal
EINSCHNITT = 1.5                 # m: Fahrbahn so weit unter dem Gelaende -> Gelaendeloch
DECKE_LUFT = 0.3                 # Gelaende ueber der Tunneldecke mindestens so hoch
DECKE_AB = 8.0                   # m hinter dem Portal beginnt das (angehobene) Gelaende ueber der Roehre


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
    """Tunnelroehre: Waende bei l/r, Decke TUNNEL_H, Aussenhaut WAND dicker (sichtbar, wo das Gelaende offen ist),
    dazu das Profil des Gelaendelochs (x, Hoehe, Ueberstand am Ende)."""
    a, b, d = l - WAND, r + WAND, TUNNEL_H + WAND
    L = _kopf()
    L += _prof([(l, TUNNEL_H, 0.0), (l, -0.2, TUNNEL_H / 4)])              # linke Wand innen
    L += _prof([(r, TUNNEL_H, 0.0), (l, TUNNEL_H, (r - l) / 4)])           # Decke von unten
    L += _prof([(r, -0.2, 0.0), (r, TUNNEL_H, TUNNEL_H / 4)])              # rechte Wand innen
    L += _prof([(a, -0.5, 0.0), (a, d, d / 4)])                            # Aussenhaut
    L += _prof([(a, d, 0.0), (b, d, (b - a) / 4)])
    L += _prof([(b, d, 0.0), (b, -0.5, d / 4)])
    L += ['[terrainholeprofile]', '']
    for x, y in ((a - 0.05, d), (a - 0.05, -0.6), (b + 0.05, -0.6), (b + 0.05, d)):
        L += ['[terrainholeprofilepnt]', f'{x:.3f}', f'{y:.3f}', '0', '']
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

        def uv(p):
            return (p[ax[0]] / 4, p[ax[1]] / 4)
        for sign, ordn in ((1, (0, 1, 2, 0, 2, 3)), (-1, (0, 2, 1, 0, 3, 2))):
            i0 = len(V)
            for p in q:
                V.append(p + tuple(sign * c for c in n) + uv(p))
            for k in range(0, 6, 3):
                F.append((i0 + ordn[k], i0 + ordn[k + 1], i0 + ordn[k + 2], 0))


def _objekt(name, map_name, V, F, x, z, hoehe, rot, art):
    sco = '\r\n'.join(['Erzeugt mit omsigen (Aschaffenburg-KI).', '', '[friendlyname]', f'{art} {name}', '',
                       '[groups]', '1', 'Aschaffenburg_KI', '', '[fixed]', '', '[absheight]', '',
                       '[mesh]', name + '.x', '']) + '\r\n'
    return dict(name=name, origin=(x, z), hoehe=hoehe, rot=rot % 360, sco=sco,
                x=x_file(V, F, [('Beton', TEX)]), faces=len(F), paths=0,
                rel=f'Sceneryobjects\\Aschaffenburg_KI\\{map_name}\\{name}.sco')


# ------------------------------------------------------------------ Laeufe und Aufbau
def _punkt(el, t):
    """Punkt, Richtung bei Laenge t im Element"""
    return end_of([el[0], el[1], el[2], t, el[4]])


def _laeufe(ebene):
    """-> [(art, i0, i1)] zusammenhaengende Bruecken-/Tunnelabschnitte (Elementindizes, i1 exklusiv)"""
    out, i = [], 0
    while i < len(ebene):
        if ebene[i]:
            j = i
            while j < len(ebene) and ebene[j] == ebene[i]:
                j += 1
            out.append((ebene[i], i, j))
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


def bauen(net, sdb, hoe, map_name):
    """-> dict(splines={datei: text}, ketten=[Begleitketten mit els/y/g/align], objekte=[...], stats);
    setzt ch['align'] (Gelaendeloch je Element) an den Strassenketten und hoe.decken (Gelaende ueber Tunneln)"""
    splines, ketten, objekte = {}, [], []
    st = dict(bruecken=0, tunnel=0, pfeiler=0, portale=0, einschnitte=0)
    gel = hoe.gelaende if hoe and hoe.gel else (lambda x, z: None)
    hoe.decken = getattr(hoe, 'decken', [])
    for ch in net['road_chains']:
        els, eb = ch['els'], ch.get('ebene') or [None] * len(ch['els'])
        ys, gs = ch.get('y') or [0.0] * len(els), ch.get('g') or [0.0] * len(els)
        align = [False] * len(els)
        for i, e in enumerate(els):        # tiefe Einschnitte: Fahrbahn deutlich unter dem Gelaende
            if eb[i]:
                continue
            p, _ = _punkt(e, e[3] / 2)
            g = gel(*p)
            if g is not None and g - (ys[i] + gs[i] / 100 * e[3] / 2) > EINSCHNITT:
                align[i] = True
                st['einschnitte'] += 1
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
            k = dict(els=[list(e[:5]) + [KI + datei] for e in run], y=ys[i0:i1], g=gs[i0:i1],
                     align=[False] * (i1 - i0), begleit=art)
            s0 = [0.0]
            for e in run:
                s0.append(s0[-1] + e[3])
            laenge = s0[-1]

            def an(s):
                """Punkt, Richtung, Fahrbahnhoehe bei Weg s im Lauf"""
                i = max(0, min(len(run) - 1, next((j for j in range(len(run)) if s0[j + 1] >= s), len(run) - 1)))
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
            else:
                portale = []
                for ende, idx, nb in (('anfang', i0, i0 - 1), ('ende', i1 - 1, i1)):
                    s = 0.0 if ende == 'anfang' else laenge
                    p, h, y = an(s)
                    g = gel(*p)
                    offen = 0 <= nb < len(els) and not eb[nb]
                    if not offen:           # Kettenende: Portal, wenn dort eine Strasse ausserhalb des Tunnels ankommt
                        knoten = ch.get('node_s') if ende == 'anfang' else ch.get('node_e')
                        offen = any(ebene(arm.get('tags', {})) != 'tunnel' for arm in net['arms'].get(knoten, ()))
                    if not offen:
                        continue            # Tunnel geht unter der Erde in eine Kreuzung o. ae. ueber
                    if 0 <= nb < len(els):  # Fahrbahn vor dem Portal schneidet das Gelaende mit
                        align[nb] = True
                    portale.append(s)
                    innen = min(10.0, laenge / 2)
                    pi, _, yi = an(s + innen if ende == 'anfang' else s - innen)
                    gi = gel(*pi)
                    oben = max(TUNNEL_H + WAND + 1.0, min(15.0, (gi - yi + 1.0) if gi is not None else 0.0))
                    V, F = [], []
                    a, b = l - WAND, r + WAND
                    _quader(V, F, a - 2.5, a, -1.0, oben, -0.5, 0.5)
                    _quader(V, F, b, b + 2.5, -1.0, oben, -0.5, 0.5)
                    _quader(V, F, a, b, TUNNEL_H, oben, -0.5, 0.5)
                    objekte.append(_objekt(f'portal_{len(objekte)}', map_name, V, F, p[0], p[1], y, h, 'Portal'))
                    st['portale'] += 1
                for j, e in enumerate(run):    # Gelaende ueber der Roehre nicht unter die Decke (ab DECKE_AB
                    for t in (0.0, e[3] / 2):  # hinter dem Portal, davor liegt das Gelaendeloch)
                        if any(abs(s0[j] + t - s) < DECKE_AB for s in portale):
                            continue
                        p, _ = _punkt(e, t)
                        hoe.decken.append((p[0], p[1], k['y'][j] + k['g'][j] / 100 * t + TUNNEL_H + WAND
                                           + DECKE_LUFT, max(-l, r) + WAND + 0.5))
                for j in range(i1 - i0):         # Gelaendeloch auf den ersten Metern hinter jedem Portal
                    if any(min(abs(s0[j] - s), abs(s0[j + 1] - s)) < LOCH_LAENGE or s0[j] <= s <= s0[j + 1]
                           for s in portale):
                        k['align'][j] = True
            ketten.append(k)
        ch['align'] = align
    return dict(splines=splines, ketten=ketten, objekte=objekte, stats=st)
