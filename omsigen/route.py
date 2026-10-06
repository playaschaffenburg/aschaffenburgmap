"""Projektion, Routing auf dem OSM-Strassennetz und Auswahl eines Korridors um die Strecke."""
import collections, heapq, math
import numpy as np


class Projection:
    """Ebene Naeherung um einen Bezugspunkt: x = Ost (m), z = Nord (m)"""

    def __init__(self, lat0, lon0):
        self.lat0, self.lon0 = lat0, lon0
        self.my = 111132.92 - 559.82 * math.cos(2 * math.radians(lat0))
        self.mx = 111412.84 * math.cos(math.radians(lat0))

    def to_m(self, lat, lon):
        return ((lon - self.lon0) * self.mx, (lat - self.lat0) * self.my)

    def to_ll(self, x, z):
        return (self.lat0 + z / self.my, self.lon0 + x / self.mx)


def _nk(c):
    return (round(c[0], 7), round(c[1], 7))


def build_graph(ways, proj, skip_layers=False):
    """gerichteter Graph: Knoten = Koordinate, Kante = Wegstueck mit Laenge"""
    G = {}
    pos = {}
    for w in ways:
        t = w['tags']
        if skip_layers and (t.get('layer', '0') != '0' or t.get('tunnel') or t.get('bridge')):
            continue
        ow = t.get('oneway')
        fwd = ow != '-1'
        bwd = not (ow in ('yes', '1', 'true', '-1') or t.get('junction') in ('roundabout', 'circular')) or ow == '-1'
        C = w['coords']
        for a, b in zip(C, C[1:]):
            ka, kb = _nk(a), _nk(b)
            pa, pb = proj.to_m(*a), proj.to_m(*b)
            pos[ka], pos[kb] = pa, pb
            d = math.dist(pa, pb)
            if fwd:
                G.setdefault(ka, []).append((kb, d))
            if bwd:
                G.setdefault(kb, []).append((ka, d))
            G.setdefault(ka, []); G.setdefault(kb, [])
    return G, pos


def nearest_node(pos, p, G=None, need_out=True):
    best, bd = None, 1e18
    for k, q in pos.items():
        if need_out and G is not None and not G.get(k):
            continue
        d = (q[0] - p[0]) ** 2 + (q[1] - p[1]) ** 2
        if d < bd:
            best, bd = k, d
    return best, math.sqrt(bd)


def dijkstra(G, s, t):
    dist, prev = {s: 0.0}, {}
    pq = [(0.0, s)]
    while pq:
        d, u = heapq.heappop(pq)
        if u == t:
            break
        if d > dist.get(u, 1e18):
            continue
        for v, w in G.get(u, ()):
            nd = d + w
            if nd < dist.get(v, 1e18):
                dist[v], prev[v] = nd, u
                heapq.heappush(pq, (nd, v))
    if t not in dist:
        return None, None
    path = [t]
    while path[-1] != s:
        path.append(prev[path[-1]])
    return path[::-1], dist[t]


def components(G):
    """Zusammenhaengende Teilnetze (ohne Beachtung der Einbahnrichtung): Knoten -> Nummer"""
    adj = {k: set() for k in G}
    for k, v in G.items():
        for n, _ in v:
            adj[k].add(n); adj.setdefault(n, set()).add(k)
    comp = {}
    for k in adj:
        if k in comp:
            continue
        comp[k] = k
        stack = [k]
        while stack:
            for w in adj[stack.pop()]:
                if w not in comp:
                    comp[w] = k; stack.append(w)
    return comp


def main_component(G, pos, pts_m, max_d=400):
    """Teilnetz, an das alle Punkte andocken: kleinste Summe der Abstaende (z. B. liegt der Hbf naeher am Nordring,
    der aber ohne Bruecken/Unterfuehrungen vom Stadtnetz abgeschnitten ist). -> Menge der Knoten oder None"""
    comp = components(G)
    best = collections.defaultdict(lambda: [1e18] * len(pts_m))
    for k, q in pos.items():
        b = best[comp[k]]
        for i, p in enumerate(pts_m):
            b[i] = min(b[i], math.dist(p, q))
    ok = {c: sum(b) for c, b in best.items() if max(b) <= max_d}
    if not ok:
        return None
    c = min(ok, key=ok.get)
    return {k for k in pos if comp[k] == c}


def route(ways, proj, points, info=None):
    """points: Liste (lat, lon) Start, Zwischenziele, Ziel -> (Polylinie in m, Laenge).
    info (dict, optional) bekommt 'andocken': Abstand jedes Punkts zur benutzten Strasse in m."""
    G, pos = build_graph(ways, proj)
    nodes = main_component(G, pos, [proj.to_m(*p) for p in points])
    if nodes is None:
        raise ValueError('Start/Ziel liegt mehr als 400 m von der naechsten Strasse entfernt '
                         '(oder Punkte liegen in nicht verbundenen Teilnetzen)')
    pos_c = {k: q for k, q in pos.items() if k in nodes}
    line, total, snap = [], 0.0, []
    for a, b in zip(points, points[1:]):
        sa, da = nearest_node(pos_c, proj.to_m(*a), G)
        if sa is None:   # nur Knoten ohne Ausfahrt in der Naehe (Einbahnende)
            sa, da = nearest_node(pos_c, proj.to_m(*a), G, need_out=False)
        sb, db = nearest_node(pos_c, proj.to_m(*b), G, need_out=False)
        snap += [da, db] if not snap else [db]
        if da > 400 or db > 400:
            raise ValueError('Start/Ziel liegt mehr als 400 m von der naechsten Strasse entfernt')
        path, L = dijkstra(G, sa, sb)
        if path is None:   # notfalls ohne Einbahnregeln
            Gu = {k: list(v) for k, v in G.items()}
            for k, v in G.items():
                for n, d in v:
                    Gu.setdefault(n, []).append((k, d))
            path, L = dijkstra(Gu, sa, sb)
        if path is None:
            raise ValueError('Keine Strassenverbindung gefunden')
        pts = [pos[k] for k in path]
        line += pts if not line else pts[1:]
        total += L
    if info is not None:
        info['andocken'] = snap
    return line, total


def _seg_dist(P, A, B):
    """Abstand Punkte P (n,2) zu Strecken A->B (m,2) -> (n,) Minimum"""
    AB = B - A
    L2 = np.maximum((AB ** 2).sum(1), 1e-9)
    AP = P[:, None, :] - A[None, :, :]
    s = np.clip((AP * AB[None]).sum(2) / L2[None], 0, 1)
    Q = A[None] + s[..., None] * AB[None]
    return np.sqrt(((P[:, None, :] - Q) ** 2).sum(2)).min(1)


def corridor(ways, proj, line, width):
    """Wege auf den Bereich <= width (m) um die Strecke zuschneiden
    -> (Liste dict(tags, P in m), Schnittpunkte am Korridorrand)"""
    A = np.array(line[:-1]); B = np.array(line[1:])
    out, cuts = [], []
    for w in ways:
        P = [proj.to_m(*c) for c in w['coords']]
        dense, orig = [], []
        for a, b in zip(P, P[1:]):
            n = max(1, int(math.dist(a, b) / 2.0))
            for k in range(n):
                dense.append((a[0] + (b[0] - a[0]) * k / n, a[1] + (b[1] - a[1]) * k / n)); orig.append(k == 0)
        dense.append(P[-1]); orig.append(True)
        D = np.array(dense)
        if _seg_dist(D[::5], A, B).min() > width + 50:
            continue
        inside = _seg_dist(D, A, B) <= width
        run, last, was_out = [], None, False
        for q, o, ins in zip(dense, orig, inside):
            if ins:
                if not run or o:            # Eintrittspunkt bzw. Original-Stuetzpunkt
                    run.append(q)
                last = q
            elif run:
                if math.dist(run[-1], last) > 0.01:
                    run.append(last)        # Austrittspunkt
                cuts.append(last)
                if len(run) >= 2:
                    out.append(dict(tags=w['tags'], P=run))
                run = []
            if not ins:
                was_out = True
            elif was_out and len(run) == 1:
                cuts.append(q)              # Eintritt nach Abschnitt ausserhalb
            if ins:
                was_out = False
        if run:
            if math.dist(run[-1], last) > 0.01:
                run.append(last)
            if len(run) >= 2:
                out.append(dict(tags=w['tags'], P=run))
    return out, cuts
