"""Prueft einen erzeugten Kartenordner: liest alle Kacheln neu ein und sucht Fahrspuren ohne Anschluss.
Optional wird eine Draufsicht als PNG gezeichnet."""
import os, re, glob, math, collections
from .geom import end_of, rvec, norm180, sample
from .splinedb import read_text


def read_splines(map_dir):
    S = []
    for p in glob.glob(os.path.join(map_dir, 'tile_*.map')):
        m = re.search(r'tile_(-?\d+)_(-?\d+)\.map$', p)
        tx, tz = int(m[1]), int(m[2])
        L = read_text(p).replace('\r', '').split('\n')
        for i, l in enumerate(L):
            if l.strip() in ('[spline]', '[spline_h]'):
                f = L[i + 1:i + 19]
                v = [float(x) for x in f[5:18]]
                S.append(dict(spl=f[1], id=int(f[2]), x=v[0] + 300 * tx, z=v[2] + 300 * tz, h=v[3], L=v[4], R=v[5]))
    return S


def lane_points(S, sdb):
    entries, exits = [], []
    for s in S:
        el = [s['x'], s['z'], s['h'], s['L'], s['R']]
        pe, he = end_of(el)
        r0, r1 = rvec(s['h']), rvec(he)
        for x, d in sdb[s['spl']]['lanes']:
            p0 = (s['x'] + x * r0[0], s['z'] + x * r0[1])
            p1 = (pe[0] + x * r1[0], pe[1] + x * r1[1])
            if d == 0:
                entries.append((p0, s['h'] % 360, s['id'])); exits.append((p1, he % 360, s['id']))
            else:
                entries.append((p1, (he + 180) % 360, s['id'])); exits.append((p0, (s['h'] + 180) % 360, s['id']))
    return entries, exits


def validate(map_dir, sdb, tol=0.05, tol_deg=1.0, edge_points=(), edge_radius=12.0):
    """edge_points: Punkte (Kartenkoordinaten), an denen Strassen bewusst enden (Korridorrand)"""
    S = read_splines(map_dir)
    entries, exits = lane_points(S, sdb)

    def grid(pts):
        g = collections.defaultdict(list)
        for e in pts:
            g[(int(e[0][0] // 5), int(e[0][1] // 5))].append(e)
        return g
    ge, gx = grid(entries), grid(exits)

    def matched(p, h, g):
        gx_, gz_ = int(p[0] // 5), int(p[1] // 5)
        for dx in (-1, 0, 1):
            for dz in (-1, 0, 1):
                for q, hq, _ in g[(gx_ + dx, gz_ + dz)]:
                    if math.dist(p, q) < tol and abs(norm180(h - hq)) < tol_deg:
                        return True
        return False

    allpts = entries + exits
    gall = grid(allpts)

    def crowd(p):   # wie viele Spurenden in 15 m Umkreis (wenige = Strassenende/Korridorrand)
        gx_, gz_ = int(p[0] // 5), int(p[1] // 5)
        return sum(1 for dx in range(-3, 4) for dz in range(-3, 4) for q, _, _ in gall[(gx_ + dx, gz_ + dz)]
                   if math.dist(p, q) < 15)

    def is_dead(p):
        return crowd(p) <= 4 or any(math.dist(p, q) < edge_radius for q in edge_points)

    open_ends, dead = [], []
    for p, h, sid in exits:
        if not matched(p, h, ge):
            (dead if is_dead(p) else open_ends).append((p, h, sid))
    for p, h, sid in entries:
        if not matched(p, h, gx):
            (dead if is_dead(p) else open_ends).append((p, h, sid))
    return dict(splines=len(S), lane_ends=len(allpts), open=open_ends, dead_ends=dead, S=S)


def preview(result, sdb, png, lim=None, title=''):
    import matplotlib
    matplotlib.use('Agg')
    import matplotlib.pyplot as plt
    from matplotlib.patches import Polygon
    S = result['S']
    xs = [s['x'] for s in S]; zs = [s['z'] for s in S]
    if lim is None:
        lim = ((min(xs) - 20, max(xs) + 20), (min(zs) - 20, max(zs) + 20))
    w = lim[0][1] - lim[0][0]; h = lim[1][1] - lim[1][0]
    fig, ax = plt.subplots(figsize=(14, 14 * h / max(w, 1)))
    for s in S:
        el = [s['x'], s['z'], s['h'], s['L'], s['R']]
        P = [(p[0], p[1], hh) for p, hh in sample(el, 1.0)]
        db = sdb[s['spl']]
        junction = 'kreuzung' in s['spl']
        for half, col, zo in ((db['half'], '#cfc8b8', 1), (db['cw'], '#666' if junction else '#555', 2)):
            Lp = [(x - half * math.cos(math.radians(a)), z + half * math.sin(math.radians(a))) for x, z, a in P]
            Rp = [(x + half * math.cos(math.radians(a)), z - half * math.sin(math.radians(a))) for x, z, a in P]
            ax.add_patch(Polygon(Lp + Rp[::-1], closed=True, fc=col, ec='none', zorder=zo))
        for x, d in db['lanes']:
            pts = [(px + x * math.cos(math.radians(a)), pz - x * math.sin(math.radians(a))) for px, pz, a in P]
            ax.plot([q[0] for q in pts], [q[1] for q in pts], color='#ff8c00' if junction else '#f5d000',
                    lw=0.6, zorder=3)
    for p, _, _ in result['open']:
        ax.plot(*p, 'x', color='red', ms=8, mew=2, zorder=9)
    ax.set_aspect('equal'); ax.set_xlim(*lim[0]); ax.set_ylim(*lim[1])
    ax.set_title(title or 'Strassen (grau), Fahrspuren (gelb), Kreuzungsspuren (orange), offene Enden (rot)')
    fig.savefig(png, dpi=100, bbox_inches='tight')
    plt.close(fig)
