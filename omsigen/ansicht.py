"""Karten-Betrachter: liest einen beliebigen OMSI-2-Kartenordner (eigene Karten und Standardkarten wie Grundorf),
baut alle Pfade (Fahrspuren, Gehwege, Schienen) aus Splines und Objekten nach, prueft die Anschluesse und zeichnet
die Karte als PNG und als interaktive HTML-Seite (Ersatz fuer "Show paths" im nEditor).

Formatdetails: docs/omsi-format.md. Nur lesend - es wird nie in einen Kartenordner geschrieben.

Beispiel:
  python -m omsigen.ansicht Grundorf --html build/grundorf.html --png build/grundorf.png
  python -m omsigen.ansicht build/maps/Test_Offline --bereich 0,0,400,300
"""
import argparse, collections, json, math, os, re, sys
from .config import FALLBACK_SPLINES, DEFAULT_OMSI
from .geom import end_of, rvec, dvec, norm180
from .splinedb import read_text

KIND = {0: 'Strasse', 1: 'Gehweg', 2: 'Schiene', 3: 'Flug'}
STRICT = (0.05, 1.0)       # unsere Pruefung (m, Grad)
LOOSE = (1.5, 40.0)        # so verbindet openOMSI (docs/omsi-format.md, "Verbindungen")
NEAR = 5.0                 # Spurende mit fremdem Spurende in diesem Abstand = Beinahe-Anschluss


def _is_kw(l):
    return len(l) > 2 and l[0] == '[' and l[-1] == ']' and ' ' not in l


def cfg_lines(txt):
    """Zeilen ohne CR, abgeschaltete Bloecke (-<DISABLED>- ... -<ENABLED>-) entfernt"""
    out, off = [], False
    for l in txt.replace('\r', '').split('\n'):
        s = l.strip()
        if s == '-<DISABLED>-':
            off = True
        elif s == '-<ENABLED>-':
            off = False
        elif not off:
            out.append(l)
    return out


def _f(s, default=0.0):
    try:
        return float(s.strip())
    except (ValueError, AttributeError):
        return default


class Reader:
    def __init__(self, lines):
        self.L, self.i = lines, 0

    def next_kw(self):
        while self.i < len(self.L):
            l = self.L[self.i]; self.i += 1
            if _is_kw(l):
                return l[1:-1].lower()
        return None

    def line(self):
        l = self.L[self.i] if self.i < len(self.L) else ''
        self.i += 1
        return l

    def num(self):
        return _f(self.line())

    def peek(self):
        return self.L[self.i] if self.i < len(self.L) else ''


# ============================================================ Inhalte (.sli / .sco)
class Inhalte:
    """Sucht Spline- und Objektdateien in mehreren Wurzelordnern (z. B. build/ und OMSI 2/)"""

    def __init__(self, roots):
        self.roots = [r for r in roots if r and os.path.isdir(r)]
        self._sli, self._sco = {}, {}
        self.fehlend = set()

    def find(self, rel):
        parts = [p for p in re.split(r'[\\/]', rel.strip()) if p]
        for r in self.roots:
            p = os.path.join(r, *parts)
            if os.path.exists(p):
                return p
        return None

    def sli(self, rel):
        if rel in self._sli:
            return self._sli[rel]
        p = self.find(rel)
        info = dict(paths=[], half=0.0, gefunden=bool(p))
        if p:
            L = cfg_lines(read_text(p))
            xs = []
            for i, l in enumerate(L):
                if l == '[path]':
                    v = [_f(x) for x in L[i + 1:i + 6]]
                    info['paths'].append(dict(kind=int(v[0]), x=v[1], h=v[2], w=v[3], d=int(v[4])))
                elif l == '[heightprofile]':
                    xs += [_f(L[i + 1]), _f(L[i + 2])]
            info['half'] = max((abs(x) for x in xs), default=0.0)
        else:
            fb = FALLBACK_SPLINES.get(rel)
            if fb:      # bekannte Standardspline ohne OMSI-Ordner: nur Fahrspuren
                info['paths'] = [dict(kind=0, x=x, h=0.1, w=3.0, d=d) for x, d in fb['lanes']]
                info['half'] = fb['half']
            else:
                self.fehlend.add(rel)
        self._sli[rel] = info
        return info

    def sco(self, rel):
        if rel in self._sco:
            return self._sco[rel]
        p = self.find(rel)
        info = dict(paths=[], helpers=0, ampeln=[], gefunden=bool(p))
        if p:
            r = Reader(cfg_lines(read_text(p)))
            while True:
                k = r.next_kw()
                if k is None:
                    break
                if k in ('path', 'path_2'):
                    v = [r.num() for _ in range(12 if k == 'path' else 14)]
                    info['paths'].append(dict(x=v[0], y=v[1], z=v[2], hd=v[3], R=v[4], L=v[5], kind=int(v[8]),
                                              w=v[9], d=int(v[10]), blinker=int(v[11]), ampel=-1))
                elif k == 'use_traffic_light' and info['paths']:
                    info['paths'][-1]['ampel'] = int(r.num())
                elif k == 'traffic_light':
                    info['ampeln'].append(dict(name=r.line().strip(), phasen=[]))
                elif k == 'phase' and info['ampeln']:
                    info['ampeln'][-1]['phasen'].append((int(r.num()), r.num()))
                elif k == 'traffic_lights_group':
                    info['umlauf'] = r.num()
                elif k == 'splinehelper':
                    info['helpers'] += 1
        else:
            self.fehlend.add(rel)
        self._sco[rel] = info
        return info


# ============================================================ Karte einlesen
def _has(v, n):
    return v == 0 or v >= n


def _labels(r):
    n = r.line().strip()
    try:
        n = int(n)
    except ValueError:
        return []
    return [r.line() for _ in range(max(0, n))]


def read_tile(path):
    """-> dict(version, splines=[...], objects=[...]); Regeln haengen am Element davor"""
    r = Reader(read_text(path).replace('\r', '').split('\n'))
    t = dict(version=0, splines=[], objects=[])
    last = None
    while True:
        k = r.next_kw()
        if k is None:
            break
        v = t['version']
        if k == 'version':
            t['version'] = int(r.num())
        elif k in ('spline', 'spline_h'):
            if _has(v, 9):
                r.line()
            s = dict(file=r.line().strip())
            s['id'] = int(r.num()) if _has(v, 6) else -1
            if _has(v, 11):
                s['prev'], s['next'] = int(r.num()), int(r.num())
            else:
                r.line()
            s['x'], s['y'], s['z'] = r.num(), r.num(), r.num()
            s['h'], s['L'], s['R'] = r.num(), r.num(), r.num()
            s['g'] = (r.num(), r.num())
            if k == 'spline_h':
                r.num()
            s['cant'] = (r.num(), r.num()) if _has(v, 5) else (0.0, 0.0)
            s['mirror'] = False
            while r.i < len(r.L):
                w = r.peek().strip()
                if w == '' or _is_kw(w):
                    break
                r.line()
                if w.lower() == 'mirror':
                    s['mirror'] = True
                    break
            s['rules'] = []
            t['splines'].append(s); last = s
        elif k in ('object', 'attachobj'):
            att = k == 'attachobj'
            if _has(v, 9):
                r.line()
            o = dict(file=r.line().strip(), attach=att)
            o['id'] = int(r.num()) if _has(v, 6) else -1
            if att:
                r.line()                        # Eltern-ID bzw. -Index
                r.line(); r.line()              # Instanz, Anhaengepunkt
                o['x'] = o['z'] = o['y'] = 0.0
                o['h'] = r.num() if _has(v, 8) else 0.0
            else:
                o['x'], o['z'], o['y'] = r.num(), r.num(), r.num()
                o['h'] = r.num()
            if _has(v, 12):
                r.line(); r.line()
            o['labels'] = _labels(r) if _has(v, 4) else []
            o['rules'] = []
            if att:
                last = None                     # ohne eigene Lage: nicht gezeichnet
            else:
                t['objects'].append(o); last = o
        elif k in ('rule', 'kill_rule'):
            ru = dict(path=int(r.num()), kind=r.line().strip().lower(), value=r.num(), kill=k == 'kill_rule')
            w = r.peek().strip()
            if w and not _is_kw(w):
                r.line()
            if last is not None:
                last['rules'].append(ru)
        elif k in ('splineattachement', 'splineattachement_repeater'):
            r.line()
            if k.endswith('repeater'):
                r.line(); r.line()
            for _ in range(12):                 # Datei, ID, Spline, Versatz, Hoehe, Start, 3 Winkel, Abstand, Bereich, Kippen
                r.line()
            _labels(r)
            last = None
    return t


def read_map(map_dir):
    """-> dict(name, tile, tiles=[(tx, tz, datei, tile|None)])"""
    g = Reader(cfg_lines(read_text(os.path.join(map_dir, 'global.cfg'))))
    name, tiles, world = os.path.basename(os.path.normpath(map_dir)), [], False
    while True:
        k = g.next_kw()
        if k is None:
            break
        if k == 'friendlyname':
            name = g.line().strip() or name
        elif k == 'worldcoordinates':
            world = True
        elif k == 'map':
            tx, tz, f = int(g.num()), int(g.num()), g.line().strip()
            p = os.path.join(map_dir, f)
            tiles.append((tx, tz, f, read_tile(p) if os.path.exists(p) else None))
    # Spandau: Kachelraster 1/300 Grad = 371,9 m (laut openOMSI)
    return dict(name=name, tile=371.9 if world else 300.0, tiles=tiles)


# ============================================================ Pfade bauen
def _samples(x, z, h, L, R, off, step=2.0):
    """Punkte entlang eines Elements mit Querversatz off (rechts positiv)"""
    n = 2 if R == 0 else max(3, int(abs(L) / step) + 2)
    pts, hs = [], []
    for k in range(n):
        (px, pz), hh = end_of([x, z, h, L * k / (n - 1), R])
        rv = rvec(hh)
        pts.append((px + off * rv[0], pz + off * rv[1])); hs.append(hh)
    return pts, hs


def build_lanes(m, inh):
    """-> (lanes, roads, objects). Spur: dict(kind, d, pts, h0, h1, src, prio, ampel, blinker)"""
    T = m['tile']
    lanes, roads, objs = [], [], []
    for ti, (tx, tz, f, t) in enumerate(m['tiles']):
        if not t:
            continue
        ox, oz = tx * T, tz * T
        for s in t['splines']:
            info = inh.sli(s['file'])
            x, z = s['x'] + ox, s['z'] + oz
            if info['half']:
                roads.append(dict(pts=_samples(x, z, s['h'], s['L'], s['R'], 0.0, 4.0)[0], half=info['half']))
            prio = {ru['path']: ru['value'] for ru in s['rules'] if ru['kind'] == 'priority' and not ru['kill']}
            for j, p in enumerate(info['paths']):
                off, d = p['x'], p['d']
                if s['mirror']:
                    off, d = -off, {0: 1, 1: 0}.get(d, d)
                pts, hs = _samples(x, z, s['h'], s['L'], s['R'], off)
                lanes.append(dict(kind=p['kind'], d=d, pts=pts, h0=hs[0], h1=hs[-1], prio=prio.get(j), ampel=-1,
                                  blinker=0, src=dict(art='Spline', id=s['id'], datei=s['file'], pfad=j, kachel=f,
                                                      lokal=(round(s['x'], 2), round(s['z'], 2)))))
        for o in t['objects']:
            info = inh.sco(o['file'])
            x, z, H = o['x'] + ox, o['z'] + oz, o['h']
            objs.append(dict(x=x, z=z, pfade=len(info['paths']), datei=o['file'], id=o['id'],
                             text=o['labels'][0].strip() if o['labels'] else ''))
            if not info['paths']:
                continue
            rv, dv = rvec(H), dvec(H)
            prio = {ru['path']: ru['value'] for ru in o['rules'] if ru['kind'] == 'priority' and not ru['kill']}
            for j, p in enumerate(info['paths']):
                lp, lh = _samples(p['x'], p['y'], p['hd'], p['L'], p['R'], 0.0)
                pts = [(x + a * rv[0] + b * dv[0], z + a * rv[1] + b * dv[1]) for a, b in lp]
                lanes.append(dict(kind=p['kind'], d=p['d'], pts=pts, h0=(lh[0] + H) % 360, h1=(lh[-1] + H) % 360,
                                  prio=prio.get(j), ampel=p['ampel'], blinker=p['blinker'],
                                  src=dict(art='Objekt', id=o['id'], datei=o['file'], pfad=j, kachel=f,
                                           lokal=(round(o['x'], 2), round(o['z'], 2)))))
    return lanes, roads, objs


# ============================================================ Anschluesse pruefen
def check(lanes):
    """Jedes Spurende (Ausfahrt) braucht einen Anfang (Einfahrt) derselben Art an gleicher Stelle und in gleicher
    Richtung. -> Liste dict(lane, p, h, ende='aus'|'ein', strict, loose, naechster=(abstand, winkel))"""
    ein, aus = [], []
    for i, l in enumerate(lanes):
        a, b = l['pts'][0], l['pts'][-1]
        if l['d'] in (0, 2):
            ein.append((a, l['h0'] % 360, i)); aus.append((b, l['h1'] % 360, i))
        if l['d'] in (1, 2):
            ein.append((b, (l['h1'] + 180) % 360, i)); aus.append((a, (l['h0'] + 180) % 360, i))

    def grid(E):
        g = collections.defaultdict(list)
        for e in E:
            g[(int(e[0][0] // NEAR), int(e[0][1] // NEAR))].append(e)
        return g
    gein, gaus = grid(ein), grid(aus)

    def best(p, h, i, g, vorne):
        """bester Partner: (abstand, winkel) mit kleinster Abweichung. Der Partner eines Spurendes muss in
        Fahrtrichtung davor liegen (vorne=+1), der eines Spuranfangs dahinter (-1) - sonst ist es nur das
        Ende des naechsten Stuecks derselben Spur."""
        res = None
        dv = dvec(h)
        cx, cz = int(p[0] // NEAR), int(p[1] // NEAR)
        for dx in (-1, 0, 1):
            for dz in (-1, 0, 1):
                for q, hq, j in g.get((cx + dx, cz + dz), ()):
                    if j == i or lanes[j]['kind'] != lanes[i]['kind']:
                        continue
                    dd, da = math.dist(p, q), abs(norm180(h - hq))
                    ahead = ((q[0] - p[0]) * dv[0] + (q[1] - p[1]) * dv[1]) * vorne
                    # Gegenrichtung (z. B. die andere Fahrbahnseite an einer Wendestelle) ist kein Partner
                    if dd <= NEAR and da < 90 and (dd <= LOOSE[0] or ahead > -0.1) and (res is None or (dd + da / 10) < (res[0] + res[1] / 10)):
                        res = (dd, da)
        return res

    out = []
    for E, g, ende, vorne in ((aus, gein, 'aus', 1), (ein, gaus, 'ein', -1)):
        for p, h, i in E:
            b = best(p, h, i, g, vorne)
            ok_s = b is not None and b[0] <= STRICT[0] and b[1] <= STRICT[1]
            ok_l = b is not None and b[0] <= LOOSE[0] and b[1] < LOOSE[1]
            out.append(dict(lane=i, p=p, h=h, ende=ende, strict=ok_s, loose=ok_l, naechster=b))
    return out


def summary(m, lanes, objs, ends, inh):
    kinds = collections.Counter(KIND.get(l['kind'], str(l['kind'])) for l in lanes)
    s = dict(karte=m['name'], kacheln=sum(1 for t in m['tiles'] if t[3]), kachelgroesse=m['tile'],
             splines=sum(len(t[3]['splines']) for t in m['tiles'] if t[3]), objekte=len(objs),
             objekte_mit_pfaden=sum(1 for o in objs if o['pfade']), pfade=dict(kinds),
             vorfahrt=dict(collections.Counter(int(l['prio']) for l in lanes if l['prio'] is not None)),
             ampelpfade=sum(1 for l in lanes if l['ampel'] >= 0), fehlende_dateien=sorted(inh.fehlend))
    for kind in (0, 1):
        E = [e for e in ends if lanes[e['lane']]['kind'] == kind]
        near = [e for e in E if not e['strict'] and e['naechster'] is not None]
        s[f'{KIND[kind]}_enden'] = dict(
            gesamt=len(E), verbunden_streng=sum(e['strict'] for e in E), verbunden_openomsi=sum(e['loose'] for e in E),
            beinahe=len(near), frei=sum(1 for e in E if e['naechster'] is None))
    return s


def classify(lanes, ends, edge_points=(), edge_radius=12.0, kind=0):
    """Nicht verbundene Spurenden einordnen -> (offen, enden). 'enden' sind gewollte Strassenenden: am
    Korridorrand (edge_points) oder Sackgassen mit wenigen Spurenden in 15 m Umkreis. Ein Ende mit Partner in
    <= 5 m (Beinahe-Anschluss) ist immer offen."""
    allp = [e['p'] for e in ends if lanes[e['lane']]['kind'] == kind]
    g = collections.defaultdict(list)
    for p in allp:
        g[(int(p[0] // 5), int(p[1] // 5))].append(p)

    def crowd(p):
        cx, cz = int(p[0] // 5), int(p[1] // 5)
        return sum(1 for dx in range(-3, 4) for dz in range(-3, 4) for q in g[(cx + dx, cz + dz)]
                   if math.dist(p, q) < 15)
    offen, enden = [], []
    for e in ends:
        if e['strict'] or lanes[e['lane']]['kind'] != kind:
            continue
        if e['naechster'] is None and (crowd(e['p']) <= 4 or
                                       any(math.dist(e['p'], q) < edge_radius for q in edge_points)):
            enden.append(e)
        else:
            offen.append(e)
    return offen, enden


def near_misses(lanes, ends, kind=0):
    """Spurenden, die einen Partner in <= 5 m haben, aber nicht streng verbunden sind (meist Fehler)"""
    out = []
    for e in ends:
        if e['strict'] or e['naechster'] is None or lanes[e['lane']]['kind'] != kind:
            continue
        out.append(e)
    out.sort(key=lambda e: -e['naechster'][0])
    return out


# ============================================================ Ausgabe
def write_png(path, lanes, roads, objs, ends, bereich=None, titel=''):
    import matplotlib
    matplotlib.use('Agg')
    import matplotlib.pyplot as plt
    from matplotlib.collections import LineCollection
    xs = [p[0] for l in lanes for p in (l['pts'][0], l['pts'][-1])] or [0, 1]
    zs = [p[1] for l in lanes for p in (l['pts'][0], l['pts'][-1])] or [0, 1]
    x0, z0, x1, z1 = bereich or (min(xs) - 20, min(zs) - 20, max(xs) + 20, max(zs) + 20)
    w, h = max(x1 - x0, 1), max(z1 - z0, 1)
    fig, ax = plt.subplots(figsize=(14, max(4, min(40, 14 * h / w))))
    big = max(w, h) > 1500
    lw = 0.3 if big else 0.7
    ax.add_collection(LineCollection([r['pts'] for r in roads], colors='#d9d4c7',
                                     linewidths=[max(0.5, 2 * r['half'] * 72 * 14 / w / 1.0) for r in roads],
                                     capstyle='butt', zorder=1))
    col = {0: '#e0b000', 1: '#2ca02c', 2: '#7f7f7f', 3: '#9467bd'}
    for kind in (2, 1, 0):
        L = [l for l in lanes if l['kind'] == kind]
        if L:
            ax.add_collection(LineCollection([l['pts'] for l in L], colors=col[kind], linewidths=lw, zorder=2 + kind))
    pr = [l for l in lanes if l['prio'] is not None]
    if pr:
        ax.add_collection(LineCollection([l['pts'] for l in pr], linewidths=lw * 2.5, zorder=5,
                                         colors=['#1f9e3a' if l['prio'] > 128 else '#d62728' for l in pr]))
    po = [o for o in objs if o['pfade']]
    ax.scatter([o['x'] for o in po], [o['z'] for o in po], s=6, marker='s', c='#1f77b4', zorder=6)
    bad = [e for e in ends if not e['strict'] and e['naechster'] is not None and lanes[e['lane']]['kind'] == 0]
    ax.scatter([e['p'][0] for e in bad], [e['p'][1] for e in bad], s=40, marker='x', c='red', zorder=9,
               linewidths=1.5)
    ax.set_aspect('equal'); ax.set_xlim(x0, x1); ax.set_ylim(z0, z1)
    ax.set_title(titel or 'Fahrspuren (gelb), Gehwege (gruen), Vorfahrt (gruen/rot dick), '
                 'Objekte mit Pfaden (blau), Beinahe-Anschluesse (rote x)', fontsize=9)
    fig.savefig(path, dpi=110, bbox_inches='tight')
    plt.close(fig)


QUELLE_FARBE = {'OSM Schild': '#1f9e3a', 'OSM Vorfahrtstrasse': '#1f6fd6', 'Korrektur': '#8e44ad', 'bestaetigt': '#8e44ad',
                'vermutet': '#ff8c00', 'OSM Mini-Kreisel': '#17a2b8'}     # StVO-Regeln: grau


def write_html(path, m, lanes, roads, objs, ends, s, kreuzungen=()):
    def r1(v):
        return round(v, 2)
    L = []
    for l in lanes:
        L.append([l['kind'], l['d'], [r1(c) for p in l['pts'] for c in p],
                  -1 if l['prio'] is None else int(l['prio']), l['ampel'], l['blinker'],
                  l['src']['art'][0], l['src']['id'], l['src']['datei'], l['src']['pfad'], l['src']['kachel'],
                  l['src']['lokal']])
    data = dict(
        name=m['name'], lanes=L,
        roads=[[r1(r['half']), [r1(c) for p in r['pts'] for c in p]] for r in roads],
        objs=[[r1(o['x']), r1(o['z']), o['pfade'], o['id'], o['datei'], o['text']] for o in objs],
        ends=[[r1(e['p'][0]), r1(e['p'][1]), e['lane'], int(e['strict']), int(e['loose']),
               -1 if e['naechster'] is None else r1(e['naechster'][0]),
               -1 if e['naechster'] is None else r1(e['naechster'][1])] for e in ends
              if not e['strict'] and e['naechster'] is not None],
        kreuz=[[r1(k['x']), r1(k['z']), k['quelle'], k['text'], int(k.get('ampel', 0)),
                [[a['name'], a['strasse'], a['schild'], a['rolle']] for a in k['arme']],
                k.get('lat'), k.get('lon'), QUELLE_FARBE.get(k['quelle'], '#888888')] for k in kreuzungen],
        stats=s)
    html = HTML.replace('__TITLE__', m['name']).replace('__DATA__', json.dumps(data, ensure_ascii=False,
                                                                               separators=(',', ':')))
    with open(path, 'w', encoding='utf-8') as f:
        f.write(html)


HTML = r"""<!doctype html>
<html lang="de"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>__TITLE__ – Pfade</title>
<style>
:root{--bg:#f4f2ee;--panel:#fff;--ink:#222;--muted:#666;--road:#d9d4c7;--lane:#d4a000;--walk:#2ca02c;--rail:#888;
--prioHi:#1f9e3a;--prioLo:#d62728;--obj:#1f77b4;--bad:#e00;--line:#ddd}
@media (prefers-color-scheme:dark){:root:not([data-theme=light]){--bg:#1b1c1e;--panel:#26282b;--ink:#eee;--muted:#aaa;
--road:#3a3936;--lane:#f0c419;--walk:#4cc24c;--rail:#999;--obj:#4ea3ff;--line:#3a3c40}}
*{box-sizing:border-box}body{margin:0;font:13px system-ui,sans-serif;background:var(--bg);color:var(--ink);height:100vh;
display:flex;flex-direction:column}
header{padding:8px 16px;display:flex;flex-wrap:wrap;gap:12px;align-items:center;border-bottom:1px solid var(--line);
background:var(--panel)}h1{font-size:15px;margin:0 8px 0 0}label{white-space:nowrap;cursor:pointer}
#wrap{flex:1;position:relative;overflow:hidden}canvas{position:absolute;inset:0;width:100%;height:100%;cursor:grab}
#info{position:absolute;right:12px;top:12px;max-width:min(360px,calc(100% - 24px));background:var(--panel);
border:1px solid var(--line);border-radius:8px;padding:10px 12px;display:none;white-space:pre-wrap;
font:12px ui-monospace,monospace;box-shadow:0 2px 10px #0003}
#stats{position:absolute;left:12px;bottom:12px;background:var(--panel);border:1px solid var(--line);border-radius:8px;
padding:8px 10px;font-size:12px;color:var(--muted);max-width:calc(100% - 24px)}
input[type=text]{width:110px;padding:3px 6px;border:1px solid var(--line);border-radius:4px;background:var(--bg);
color:var(--ink)}button{padding:3px 8px;border:1px solid var(--line);border-radius:4px;background:var(--bg);
color:var(--ink);cursor:pointer}
</style></head><body>
<header><h1>__TITLE__</h1>
<label><input type="checkbox" id="lRoad" checked> Strassenflaeche</label>
<label><input type="checkbox" id="lLane" checked> Fahrspuren</label>
<label><input type="checkbox" id="lWalk" checked> Gehwege</label>
<label><input type="checkbox" id="lRail"> Schienen</label>
<label><input type="checkbox" id="lPrio" checked> Vorfahrt</label>
<label><input type="checkbox" id="lObj" checked> Objekte</label>
<label><input type="checkbox" id="lBad" checked> Beinahe-Anschluesse</label>
<label><input type="checkbox" id="lKr" checked> Vorfahrt-Quelle</label>
<span><input type="text" id="q" placeholder="ID suchen"> <button id="go">Suchen</button> <button id="home">Ganz</button></span>
</header>
<div id="wrap"><canvas id="c"></canvas><div id="info"></div><div id="stats"></div></div>
<script>
const D=__DATA__;
const cv=document.getElementById('c'),ctx=cv.getContext('2d'),info=document.getElementById('info');
const css=n=>getComputedStyle(document.documentElement).getPropertyValue(n).trim();
const KIND=['Strasse','Gehweg','Schiene','Flug'],DIR=['mit Spline','gegen Spline','beide'];
let bb=[1e9,1e9,-1e9,-1e9];
for(const l of D.lanes){const p=l[2];for(let i=0;i<p.length;i+=2){bb[0]=Math.min(bb[0],p[i]);bb[1]=Math.min(bb[1],p[i+1]);
bb[2]=Math.max(bb[2],p[i]);bb[3]=Math.max(bb[3],p[i+1]);}l.bb=[...bb];}
// eigene Huelle je Spur fuer schnelles Zeichnen
for(const l of D.lanes){const p=l[2];let a=[1e9,1e9,-1e9,-1e9];for(let i=0;i<p.length;i+=2){a[0]=Math.min(a[0],p[i]);
a[1]=Math.min(a[1],p[i+1]);a[2]=Math.max(a[2],p[i]);a[3]=Math.max(a[3],p[i+1]);}l.bb=a;}
let W=0,H=0,s=1,cx=0,cz=0,sel=null;
function fit(){const w=bb[2]-bb[0]+40,h=bb[3]-bb[1]+40;s=Math.min(W/w,H/h);cx=(bb[0]+bb[2])/2;cz=(bb[1]+bb[3])/2;}
function resize(){const r=cv.getBoundingClientRect(),d=devicePixelRatio||1;W=r.width;H=r.height;cv.width=W*d;cv.height=H*d;
ctx.setTransform(d,0,0,d,0,0);}
const X=x=>(x-cx)*s+W/2,Z=z=>H/2-(z-cz)*s;
function vis(a){const x0=cx-W/2/s,x1=cx+W/2/s,z0=cz-H/2/s,z1=cz+H/2/s;return !(a[2]<x0||a[0]>x1||a[3]<z0||a[1]>z1);}
function poly(p){ctx.beginPath();ctx.moveTo(X(p[0]),Z(p[1]));for(let i=2;i<p.length;i+=2)ctx.lineTo(X(p[i]),Z(p[i+1]));}
function on(id){return document.getElementById(id).checked;}
function draw(){ctx.fillStyle=css('--bg');ctx.fillRect(0,0,W,H);ctx.lineCap='butt';ctx.lineJoin='round';
 if(on('lRoad')){ctx.strokeStyle=css('--road');for(const r of D.roads){const p=r[1];ctx.lineWidth=Math.max(1,2*r[0]*s);poly(p);ctx.stroke();}}
 const lw=Math.max(0.6,Math.min(2.5,s*0.4));
 const layers=[[2,'lRail','--rail'],[1,'lWalk','--walk'],[0,'lLane','--lane']];
 for(const [k,id,c] of layers){if(!on(id))continue;ctx.strokeStyle=css(c);ctx.lineWidth=lw;ctx.beginPath();
  for(const l of D.lanes){if(l[0]!==k||!vis(l.bb))continue;const p=l[2];ctx.moveTo(X(p[0]),Z(p[1]));
   for(let i=2;i<p.length;i+=2)ctx.lineTo(X(p[i]),Z(p[i+1]));}ctx.stroke();
  if(s>3&&k===0){ctx.fillStyle=css(c);for(const l of D.lanes){if(l[0]!==0||!vis(l.bb))continue;arrows(l);}}}
 if(on('lPrio')){ctx.lineWidth=lw*3;for(const l of D.lanes){if(l[3]<0||!vis(l.bb))continue;
  ctx.strokeStyle=css(l[3]>128?'--prioHi':'--prioLo');poly(l[2]);ctx.stroke();}}
 if(on('lObj')){ctx.fillStyle=css('--obj');for(const o of D.objs){if(!o[2]&&s<2)continue;const x=X(o[0]),z=Z(o[1]);
  if(x<-5||x>W+5||z<-5||z>H+5)continue;const r=o[2]?4:1.5;ctx.fillRect(x-r,z-r,2*r,2*r);}}
 if(on('lBad')){ctx.strokeStyle=css('--bad');ctx.lineWidth=2;for(const e of D.ends){const x=X(e[0]),z=Z(e[1]);
  ctx.beginPath();ctx.moveTo(x-6,z-6);ctx.lineTo(x+6,z+6);ctx.moveTo(x+6,z-6);ctx.lineTo(x-6,z+6);ctx.stroke();}}
 if(sel){ctx.strokeStyle='#ff00ff';ctx.lineWidth=lw*4;poly(sel[2]);ctx.stroke();}
 if(on('lKr')){for(const k of D.kreuz){const x=X(k[0]),z=Z(k[1]);if(x<-9||x>W+9||z<-9||z>H+9)continue;
  ctx.beginPath();ctx.arc(x,z,7,0,7);ctx.fillStyle=k[8];ctx.fill();ctx.lineWidth=k[4]?3:1;
  ctx.strokeStyle=k[4]?'#d000d0':'#fff';ctx.stroke();}}
}
function arrows(l){const p=l[2],n=p.length/2;if(n<2)return;const m=Math.floor((n-1)/2)*2;const x0=p[m],z0=p[m+1],x1=p[m+2],z1=p[m+3];
 let a=Math.atan2(-(z1-z0),x1-x0);if(l[1]===1)a+=Math.PI;const x=X((x0+x1)/2),z=Z((z0+z1)/2),r=Math.min(8,s*0.8);
 if(l[1]===2)return;ctx.beginPath();ctx.moveTo(x+r*Math.cos(a),z+r*Math.sin(a));
 ctx.lineTo(x+r*Math.cos(a+2.5),z+r*Math.sin(a+2.5));ctx.lineTo(x+r*Math.cos(a-2.5),z+r*Math.sin(a-2.5));ctx.fill();}
function pick(mx,my){let best=null,bd=10;const wx=(mx-W/2)/s+cx,wz=cz-(my-H/2)/s;
 for(const l of D.lanes){if(!vis(l.bb))continue;const p=l[2];for(let i=0;i+3<p.length;i+=2){
  const d=segd(wx,wz,p[i],p[i+1],p[i+2],p[i+3])*s;if(d<bd){bd=d;best=l;}}}return [best,wx,wz];}
function segd(x,z,ax,az,bx,bz){const dx=bx-ax,dz=bz-az,L=dx*dx+dz*dz;let t=L?((x-ax)*dx+(z-az)*dz)/L:0;t=Math.max(0,Math.min(1,t));
 return Math.hypot(x-ax-t*dx,z-az-t*dz);}
function show(l,wx,wz){sel=l;if(!l){info.style.display='none';draw();return;}
 const art=l[6]==='S'?'Spline':'Objekt';
 let t=`${art} ${l[7]}  (Pfad ${l[9]})\n${l[8]}\nKachel ${l[10]}  lokal x=${l[11][0]} z=${l[11][1]}\n`+
  `Art: ${KIND[l[0]]}, Richtung: ${DIR[l[1]]||l[1]}\n`;
 if(l[3]>=0)t+=`Vorfahrt-Prioritaet: ${l[3]} ${l[3]>128?'(Hauptstrasse)':'(wartepflichtig)'}\n`;
 if(l[4]>=0)t+=`Ampel: ${l[4]}\n`;if(l[5])t+=`Blinker: ${l[5]===2?'links':'rechts'}\n`;
 t+=`Klick bei x=${wx.toFixed(1)} z=${wz.toFixed(1)}`;info.textContent=t;info.style.display='block';draw();}
let drag=null;
cv.addEventListener('mousedown',e=>{drag=[e.clientX,e.clientY,cx,cz,false];cv.style.cursor='grabbing';});
addEventListener('mousemove',e=>{if(!drag)return;const dx=e.clientX-drag[0],dy=e.clientY-drag[1];if(Math.abs(dx)+Math.abs(dy)>3)drag[4]=true;
 cx=drag[2]-dx/s;cz=drag[3]+dy/s;draw();});
function pickK(mx,my){if(!on('lKr'))return null;for(const k of D.kreuz){if(Math.hypot(X(k[0])-mx,Z(k[1])-my)<9)return k;}return null;}
function showK(k){sel=null;let t=`Kreuzung - Vorfahrt: ${k[2]}${k[4]?' (Ampel)':''}
${k[3]}
`;
 for(const a of k[5])t+=`  ${a[3].padEnd(6)} ${a[0]||'(ohne Name)'} [${a[1]}]${a[2]?' Schild: '+a[2]:''}
`;
 if(k[6]!==null)t+=`lat ${k[6].toFixed(6)}, lon ${k[7].toFixed(6)}  (fuer die Korrekturdatei)`;
 info.textContent=t;info.style.display='block';draw();}
addEventListener('mouseup',e=>{if(drag&&!drag[4]){const r=cv.getBoundingClientRect();const mx=e.clientX-r.left,my=e.clientY-r.top;
 const k=pickK(mx,my);if(k)showK(k);else{const [l,wx,wz]=pick(mx,my);show(l,wx,wz);}}
 drag=null;cv.style.cursor='grab';});
cv.addEventListener('wheel',e=>{e.preventDefault();const r=cv.getBoundingClientRect(),mx=e.clientX-r.left,my=e.clientY-r.top;
 const wx=(mx-W/2)/s+cx,wz=cz-(my-H/2)/s,f=Math.exp(-e.deltaY*0.0015);s=Math.max(0.01,Math.min(200,s*f));
 cx=wx-(mx-W/2)/s;cz=wz+(my-H/2)/s;draw();},{passive:false});
document.querySelectorAll('header input[type=checkbox]').forEach(c=>c.addEventListener('change',draw));
document.getElementById('home').onclick=()=>{fit();draw();};
document.getElementById('go').onclick=()=>{const q=document.getElementById('q').value.trim();const l=D.lanes.find(l=>String(l[7])===q);
 if(l){const p=l[2];cx=p[0];cz=p[1];s=Math.max(s,3);show(l,p[0],p[1]);}else{info.textContent='ID '+q+' nicht gefunden';info.style.display='block';}};
const st=D.stats,se=st.Strasse_enden;
document.getElementById('stats').textContent=`${st.kacheln} Kacheln · ${st.splines} Splines · ${st.objekte} Objekte (${st.objekte_mit_pfaden} mit Pfaden) · `+
 `Fahrspur-Enden: ${se.gesamt}, streng verbunden ${se.verbunden_streng}, nach openOMSI ${se.verbunden_openomsi}, Beinahe-Anschluesse ${se.beinahe}`;
addEventListener('resize',()=>{resize();draw();});resize();fit();draw();
</script></body></html>
"""


# ============================================================ Kommandozeile
def resolve_map(arg, omsi):
    if os.path.isdir(arg):
        return arg
    if omsi and os.path.isdir(os.path.join(omsi, 'maps', arg)):
        return os.path.join(omsi, 'maps', arg)
    raise SystemExit(f'Kartenordner nicht gefunden: {arg}')


def analyse(map_dir, omsi=None):
    """Alles in einem: -> dict(m, lanes, roads, objs, ends, summary, inhalte)"""
    root = os.path.dirname(os.path.dirname(os.path.abspath(map_dir)))   # .../maps/<Name> -> Wurzel
    inh = Inhalte([root, omsi])
    m = read_map(map_dir)
    lanes, roads, objs = build_lanes(m, inh)
    ends = check(lanes)
    kreuz = []
    meta = os.path.join(map_dir, 'omsigen.json')
    if os.path.exists(meta):                      # von omsigen erzeugte Karte: Vorfahrt-Herkunft je Kreuzung
        with open(meta, encoding='utf-8') as f:
            kreuz = json.load(f).get('kreuzungen', [])
    return dict(m=m, lanes=lanes, roads=roads, objs=objs, ends=ends, inhalte=inh, kreuzungen=kreuz,
                summary=summary(m, lanes, objs, ends, inh))


def main(argv=None):
    ap = argparse.ArgumentParser(prog='omsigen.ansicht', description='OMSI-2-Karte lesen, Pfade pruefen und zeichnen')
    ap.add_argument('karte', help='Kartenordner oder Kartenname unter <OMSI>/maps')
    ap.add_argument('--omsi', default=DEFAULT_OMSI, help='OMSI-2-Ordner (fuer Splines/Objekte der Standardinhalte)')
    ap.add_argument('--html', help='interaktive Ansicht hierhin schreiben')
    ap.add_argument('--png', help='Draufsicht als PNG')
    ap.add_argument('--json', help='Bericht als JSON')
    ap.add_argument('--bereich', help='Ausschnitt fuer das PNG: x0,z0,x1,z1 (Kartenmeter)')
    ap.add_argument('--liste', type=int, default=10, help='so viele Beinahe-Anschluesse auflisten')
    a = ap.parse_args(argv)
    d = resolve_map(a.karte, a.omsi)
    r = analyse(d, a.omsi)
    s = r['summary']
    print(f"Karte {s['karte']}: {s['kacheln']} Kacheln ({s['kachelgroesse']} m), {s['splines']} Splines, "
          f"{s['objekte']} Objekte ({s['objekte_mit_pfaden']} mit Pfaden)")
    print('Pfade: ' + ', '.join(f'{k} {v}' for k, v in s['pfade'].items()) +
          (f" | Vorfahrt-Regeln {s['vorfahrt']}" if s['vorfahrt'] else '') +
          (f" | Ampel-Pfade {s['ampelpfade']}" if s['ampelpfade'] else ''))
    for k in ('Strasse', 'Gehweg'):
        e = s[f'{k}_enden']
        print(f"{k}-Enden: {e['gesamt']}, streng verbunden {e['verbunden_streng']}, nach openOMSI "
              f"{e['verbunden_openomsi']}, Beinahe-Anschluesse (<= {NEAR:.0f} m) {e['beinahe']}, frei {e['frei']}")
    if s['fehlende_dateien']:
        print(f"Nicht gefunden ({len(s['fehlende_dateien'])}): " + ', '.join(s['fehlende_dateien'][:8]))
    nm = near_misses(r['lanes'], r['ends'])
    T = r['m']['tile']
    for e in nm[:a.liste]:
        l = r['lanes'][e['lane']]; src = l['src']
        tx, tz = math.floor(e['p'][0] / T), math.floor(e['p'][1] / T)
        print(f"  {'Ende' if e['ende'] == 'aus' else 'Anfang'} {src['art']} {src['id']} Pfad {src['pfad']} "
              f"({src['datei'].split(chr(92))[-1]}) Kachel {tx},{tz} lokal x={e['p'][0] - tx * T:.1f} "
              f"z={e['p'][1] - tz * T:.1f}: naechster Partner {e['naechster'][0]:.2f} m / {e['naechster'][1]:.1f} Grad")
    bereich = tuple(float(v) for v in a.bereich.split(',')) if a.bereich else None
    if a.png:
        write_png(a.png, r['lanes'], r['roads'], r['objs'], r['ends'], bereich, f"{s['karte']}")
        print(f'PNG: {a.png}')
    if a.html:
        write_html(a.html, r['m'], r['lanes'], r['roads'], r['objs'], r['ends'], s, r['kreuzungen'])
        print(f'HTML: {a.html}')
    if a.json:
        with open(a.json, 'w', encoding='utf-8') as f:
            json.dump(dict(summary=s, beinahe=[dict(p=e['p'], ende=e['ende'], quelle=r['lanes'][e['lane']]['src'],
                                                    abstand=e['naechster'][0], winkel=e['naechster'][1])
                                               for e in nm]), f, ensure_ascii=False, indent=1)
    return 0


if __name__ == '__main__':
    sys.exit(main())
