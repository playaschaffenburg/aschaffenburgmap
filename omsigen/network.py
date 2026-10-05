"""Strassennetz bauen: Wege -> Kanten -> Knoten/Kreuzungen -> Ketten aus Geraden und Kreisboegen
-> Kreuzungsspuren, die die Fahrspuren aller Arme exakt verbinden."""
import math, collections
import numpy as np
from .config import RADIUS, KI, choose_spline, is_oneway
from .custom_splines import KREUZ_GAPS
from .geom import heading, norm180, rvec, end_of, straight, connect


CORNER_ROOM = 6.0      # zusaetzliche Kuerzung der Arme an Kreuzungen (Platz fuer Bordsteinradien)


def key(p):
    return (round(p[0], 2), round(p[1], 2))


def plen(P):
    return sum(math.dist(a, b) for a, b in zip(P, P[1:]))


def proj_point(p, a, b):
    dx, dz = b[0] - a[0], b[1] - a[1]
    L2 = dx * dx + dz * dz
    if L2 == 0:
        return 0, math.dist(p, a)
    s = max(0, min(1, ((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / L2))
    return s, math.dist(p, (a[0] + s * dx, a[1] + s * dz))


def _at(P, s):
    """Punkt und Richtung nach s Metern entlang P"""
    for a, b in zip(P, P[1:]):
        d = math.dist(a, b)
        if s <= d or b is P[-1]:
            f = min(1.0, s / d) if d else 0
            return (a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f), heading(a, b)
        s -= d
    return P[-1], heading(P[-2], P[-1])


def _closest(p, P):
    best = (1e18, P[0])
    for a, b in zip(P, P[1:]):
        t, d = proj_point(p, a, b)
        if d < best[0]:
            best = (d, (a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t))
    return best


def build(ways_in, sdb):
    """ways_in: Liste dict(tags, P=[(x,z),...]) in Metern. -> dict(road_chains, conn_chains, junctions, stats)
    Kette = dict(els=[[x, z, h, L, R, spline], ...], name, hw)"""
    ways = [dict(t=w['tags'], P=[tuple(p) for p in w['P']]) for w in ways_in
            if not (w['tags'].get('layer', '0') != '0' or w['tags'].get('tunnel') or w['tags'].get('bridge'))]
    def proj(p, a, b):
        dx, dz = b[0] - a[0], b[1] - a[1]
        L2 = dx * dx + dz * dz
        if L2 == 0:
            return 0, math.dist(p, a)
        s = max(0, min(1, ((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / L2))
        return s, math.dist(p, (a[0] + s * dx, a[1] + s * dz))


    for w in ways:   # Wegende mitten auf anderem Weg -> Stuetzpunkt einfuegen
        for end in (w['P'][0], w['P'][-1]):
            for o in ways:
                if o is w or any(math.dist(end, p) < 0.3 for p in o['P']):
                    continue
                for i in range(len(o['P']) - 1):
                    s, d = proj(end, o['P'][i], o['P'][i + 1])
                    if d < 1.5 and 0 < s < 1:
                        o['P'].insert(i + 1, end)
                        break

    cnt = collections.Counter()
    ends = set()
    for w in ways:
        for i, p in enumerate(w['P']):
            cnt[key(p)] += 1 if i in (0, len(w['P']) - 1) else 2
        ends.add(key(w['P'][0])); ends.add(key(w['P'][-1]))

    edges = []
    for w in ways:
        P, cur = w['P'], [w['P'][0]]
        for i in range(1, len(P)):
            cur.append(P[i])
            k = key(P[i])
            if i == len(P) - 1 or cnt[k] >= 3 or (k in ends and cnt[k] >= 2):
                dd = [cur[0]] + [q for a, q in zip(cur, cur[1:]) if math.dist(a, q) > 0.05]
                if len(dd) >= 2:
                    edges.append(dict(t=w['t'], P=dd))
                cur = [P[i]]


    def mid_dir(P):
        L, s = plen(P), 0
        for a, b in zip(P, P[1:]):
            d = math.dist(a, b)
            if s + d >= L / 2:
                f = (L / 2 - s) / d if d else 0
                return (a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f), heading(a, b)
            s += d
        return P[0], heading(P[0], P[-1])


    for e in edges:
        e['dual'] = False
        if e['t'].get('oneway') not in ('yes', '1', '-1') or 'name' not in e['t']:
            continue
        m, h = mid_dir(e['P'])
        for o in edges:
            if o is e or o['t'].get('oneway') not in ('yes', '1', '-1') or o['t'].get('name') != e['t']['name']:
                continue
            best = min(proj(m, a, b)[1] for a, b in zip(o['P'], o['P'][1:]))
            if 3 < best < 25 and abs(norm180(h - mid_dir(o['P'])[1])) > 140:
                e['dual'] = True

    for e in edges:
        e['spl'], force = choose_spline(e['t'], e['dual'])
        if force == -1:
            e['P'] = e['P'][::-1]
        e['force'] = force != 0
        e['ring'] = e['t'].get('junction') in ('roundabout', 'circular')

    # Einbahnstrassen: Gehweg nur auf Seiten, neben denen keine andere Strasse liegt (Bypaesse, Richtungsfahrbahnen
    # ohne gleichen Namen) - sonst ueberlappen die Gehwege mit der Nachbarfahrbahn
    for e in edges:
        if not is_oneway(e['t']):
            continue
        walk = [True, True]
        me = sdb[e['spl']]
        L = plen(e['P'])
        for s_ in [L * f for f in (0.25, 0.5, 0.75)] if L > 16 else [L / 2]:
            m, h = _at(e['P'], s_)
            for o in edges:
                if o is e:
                    continue
                d, q = _closest(m, o['P'])
                need = me['cw'] + 3.5 + sdb[o['spl']]['half']
                if d < need:
                    side = (q[0] - m[0]) * math.cos(math.radians(h)) - (q[1] - m[1]) * math.sin(math.radians(h))
                    walk[1 if side > 0 else 0] = False
        if walk != [True, True]:
            e['spl'], _ = choose_spline(e['t'], e['dual'], tuple(walk))

    # ============================================================ Kreisverkehre auf Kreis legen
    groups = []
    for e in [e for e in edges if e['ring']]:
        ks = {key(p) for p in e['P']}
        g = dict(k=ks, e=[e])
        for h in [g2 for g2 in groups if g2['k'] & ks]:
            g['k'] |= h['k']; g['e'] += h['e']; groups.remove(h)
        groups.append(g)
    moved = {}
    for g in groups:
        pts = [p for e in g['e'] for p in e['P']]
        A = np.array([[2 * x, 2 * z, 1] for x, z in pts]); bb = np.array([x * x + z * z for x, z in pts])
        cx, cz, c = np.linalg.lstsq(A, bb, rcond=None)[0]
        r = math.sqrt(c + cx * cx + cz * cz)
        for e in g['e']:
            e['circle'] = (cx, cz, r)
            for i, p in enumerate(e['P']):
                a = math.atan2(p[0] - cx, p[1] - cz)
                q = (cx + r * math.sin(a), cz + r * math.cos(a))
                moved[key(p)] = q
                e['P'][i] = q
    for e in edges:
        for i in (0, -1):
            if key(e['P'][i]) in moved and 'circle' not in e:
                e['P'][i] = moved[key(e['P'][i])]

    # ============================================================ Knoten bestimmen
    for e in edges:
        e['n0'], e['n1'] = key(e['P'][0]), key(e['P'][-1])

    # sehr kurze Kanten (< 7 m, z. B. zwischen zwei dicht liegenden Einmuendungen) zu einem Knoten zusammenziehen
    while True:
        short = [e for e in edges if 'circle' not in e and plen(e['P']) < 7 and e['n0'] != e['n1']]
        if not short:
            break
        e = min(short, key=lambda e: plen(e['P']))
        keep, drop = e['n0'], e['n1']
        kp = e['P'][0]
        edges.remove(e)
        for o in edges:
            if o['n0'] == drop:
                o['n0'] = keep; o['P'][0] = kp
            if o['n1'] == drop:
                o['n1'] = keep; o['P'][-1] = kp
    edges = [e for e in edges if e['n0'] != e['n1'] or plen(e['P']) > 20]
    node_edges = collections.defaultdict(list)
    for e in edges:
        node_edges[e['n0']].append((e, 0))
        node_edges[e['n1']].append((e, -1))

    JN = set()
    for k, lst in node_edges.items():
        if len(lst) >= 3:
            JN.add(k)
        elif len(lst) == 2:
            (e1, x1), (e2, x2) = lst
            if e1['ring'] and e2['ring']:
                continue                       # Kreisbogen geht nahtlos weiter
            if e1['spl'] != e2['spl'] or e1['ring'] != e2['ring'] or ((e1['force'] or e2['force']) and x1 == x2):
                JN.add(k)


    def out_dir(e, end, dist=10):
        P = e['P'] if end == 0 else e['P'][::-1]
        s, q = 0, P[1]
        for a, b in zip(P, P[1:]):
            s += math.dist(a, b); q = b
            if s > dist:
                break
        return heading(P[0], q)


    # Kuerzungslaengen je Kantenende
    trim_d = {}
    for k in JN:
        lst = node_edges[k]
        dirs = [out_dir(e, end) for e, end in lst]
        for i, (e, end) in enumerate(lst):
            d = 0
            for j, (o, oend) in enumerate(lst):
                if i == j:
                    continue
                ang = abs(norm180(dirs[i] - dirs[j]))
                if 20 <= ang <= 160:
                    # Platz fuer die Fahrbahn des anderen Arms und fuer die Bordsteinausrundung
                    d = max(d, sdb[o['spl']]['cw'] / math.sin(math.radians(ang)) + CORNER_ROOM)
            if len(lst) == 2:
                d = 6.0
            trim_d[(id(e), end)] = min(max(d, 4.0), 30.0)
    for e in edges:
        a, b = trim_d.get((id(e), 0), 0), trim_d.get((id(e), -1), 0)
        L = plen(e['P'])
        if a + b > L - 2:
            f = max(0.0, (L - 2) / (a + b)) if a + b else 0
            a, b = a * f, b * f
        e['trim'] = (a, b)


    def cut(P, d):
        P = list(P)
        rest = d
        while len(P) >= 2 and rest > 0:
            seg = math.dist(P[0], P[1])
            if seg > rest:
                f = rest / seg
                P[0] = (P[0][0] + (P[1][0] - P[0][0]) * f, P[0][1] + (P[1][1] - P[0][1]) * f)
                break
            rest -= seg
            P.pop(0)
        return P


    for e in edges:
        a, b = e['trim']
        P = cut(e['P'], a)
        e['P'] = cut(P[::-1], b)[::-1]

    # ============================================================ Ketten bilden
    node_edges = collections.defaultdict(list)
    for e in edges:
        node_edges[e['n0']].append((e, 0))
        node_edges[e['n1']].append((e, -1))
    used, chains = set(), []


    def joinable(k):
        return k not in JN and len(node_edges[k]) == 2


    for e in edges:
        if id(e) in used:
            continue
        chain = [(e, False)]; used.add(id(e))
        if 'circle' not in e:
            while True:          # vorwaerts
                last, rev = chain[-1]
                k = last['n0'] if rev else last['n1']
                if not joinable(k):
                    break
                nx, nend = [x for x in node_edges[k] if x[0] is not last][0]
                if id(nx) in used or 'circle' in nx:
                    break
                chain.append((nx, nend == -1)); used.add(id(nx))
            while True:          # rueckwaerts
                first, rev = chain[0]
                k = first['n1'] if rev else first['n0']
                if not joinable(k):
                    break
                pv, pend = [x for x in node_edges[k] if x[0] is not first][0]
                if id(pv) in used or 'circle' in pv:
                    break
                chain.insert(0, (pv, pend == 0)); used.add(id(pv))
            if any(r for x, r in chain if x['force']):
                chain = [(x, not r) for x, r in chain[::-1]]
        chains.append(chain)

    # ============================================================ Geometrie der Strassen


    def fillet_chain(chain):
        pts, seg_spl, seg_rad = [], [], []
        for e, rev in chain:
            P = e['P'][::-1] if rev else e['P']
            if pts and math.dist(pts[-1], P[0]) < 0.05:
                P = P[1:]
            for p in P:
                if pts:
                    seg_spl.append(e['spl']); seg_rad.append(RADIUS.get(e['t'].get('highway'), 12))
                pts.append(p)
        clean, cs, cr = [pts[0]], [], []
        for i in range(1, len(pts)):
            if math.dist(clean[-1], pts[i]) < 0.8 and i < len(pts) - 1:
                continue
            if math.dist(clean[-1], pts[i]) < 0.05:
                continue
            clean.append(pts[i]); cs.append(seg_spl[i - 1]); cr.append(seg_rad[i - 1])
        pts, seg_spl, seg_rad = clean, cs, cr
        n = len(pts)
        if n < 2:
            return []
        L = [math.dist(pts[i], pts[i + 1]) for i in range(n - 1)]
        hd = [heading(pts[i], pts[i + 1]) for i in range(n - 1)]
        tl, R, dth = [0.0] * n, [0.0] * n, [0.0] * n
        for i in range(1, n - 1):
            th = norm180(hd[i] - hd[i - 1]); dth[i] = th
            if abs(th) < 0.3:
                continue
            r = max(seg_rad[i - 1], seg_rad[i])
            t = r * math.tan(math.radians(abs(th)) / 2)
            t = min(t, L[i - 1] * (0.5 if i - 1 > 0 else 0.95), L[i] * (0.5 if i < n - 2 else 0.95))
            tl[i] = t; R[i] = t / math.tan(math.radians(abs(th)) / 2) * (1 if th > 0 else -1)
        els, cur = [], pts[0]
        for i in range(1, n):
            a = pts[i - 1]
            if i == n - 1:
                els += straight(cur, pts[i], seg_spl[i - 1]); break
            u = ((pts[i][0] - a[0]) / L[i - 1], (pts[i][1] - a[1]) / L[i - 1])
            t1 = (pts[i][0] - u[0] * tl[i], pts[i][1] - u[1] * tl[i])
            els += straight(cur, t1, seg_spl[i - 1])
            if tl[i] > 0:
                els.append([t1[0], t1[1], hd[i - 1], abs(R[i]) * math.radians(abs(dth[i])), R[i], seg_spl[i - 1]])
                v = ((pts[i + 1][0] - pts[i][0]) / L[i], (pts[i + 1][1] - pts[i][1]) / L[i])
                cur = (pts[i][0] + v[0] * tl[i], pts[i][1] + v[1] * tl[i])
            else:
                cur = pts[i]
        # wegen Mini-Knicken (< 0,3 Grad) Elemente exakt aneinanderhaengen
        for j in range(1, len(els)):
            p, h = end_of(els[j - 1])
            els[j][0], els[j][1] = p
            if els[j][4] != 0 or abs(norm180(h - els[j][2])) < 0.5:
                els[j][2] = h
        return els


    def circle_chain(e):
        cx, cz, r = e['circle']
        P = e['P']
        a0 = math.atan2(P[0][0] - cx, P[0][1] - cz)
        am = math.atan2(P[len(P) // 2][0] - cx, P[len(P) // 2][1] - cz) if len(P) > 2 else None
        a1 = math.atan2(P[-1][0] - cx, P[-1][1] - cz)
        cw = norm180(math.degrees((am if am is not None else a1) - a0)) > 0
        span = (math.degrees(a1 - a0)) % 360 if cw else (math.degrees(a0 - a1)) % 360
        if span < 0.5:
            span = 360 if len(P) > 3 else span
        if span < 0.5:
            return []
        h0, R = (math.degrees(a0) + 90, r) if cw else (math.degrees(a0) - 90, -r)
        x, z = cx + r * math.sin(a0), cz + r * math.cos(a0)
        els, k = [], max(1, math.ceil(span / 90))
        for j in range(k):
            el = [x, z, h0 % 360, abs(R) * math.radians(span / k), R, e['spl']]
            els.append(el)
            (x, z), h0 = end_of(el)
        return els


    road_chains = []
    for ch in chains:
        if len(ch) == 1 and 'circle' in ch[0][0]:
            els = circle_chain(ch[0][0])
        else:
            els = fillet_chain(ch)
        if not els:
            continue
        e0, r0 = ch[0]; e1, r1 = ch[-1]
        road_chains.append(dict(els=els, name=e0['t'].get('name', ''), hw=e0['t'].get('highway'),
                                node_s=(e0['n1'] if r0 else e0['n0']), node_e=(e1['n0'] if r1 else e1['n1'])))

    # ============================================================ Kreuzungsbauer
    def lane_ends(ch, which):
        """Spurenden eines Strassenendes: Liste (pos, richtung, 'in'/'out', querlage, abstand_bordstein)"""
        if which == 's':
            el = ch['els'][0]; pos, h, spl = (el[0], el[1]), el[2], el[5]
        else:
            el = ch['els'][-1]; (pos, h), spl = end_of(el), el[5]
        db = sdb[spl]
        out = []
        for x, d in db['lanes']:
            rv = rvec(h)
            p = (pos[0] + x * rv[0], pos[1] + x * rv[1])
            if which == 'e':
                kind, hl = ('in', h) if d == 0 else ('out', (h + 180) % 360)
            else:
                kind, hl = ('out', h) if d == 0 else ('in', (h + 180) % 360)
            out.append(dict(p=p, h=hl, kind=kind, gap=max(0.5, db['cw'] - abs(x))))
        for l in out:     # Rechts-Rang relativ zur Fahrtrichtung
            rv = rvec(l['h'])
            l['right'] = (l['p'][0] - pos[0]) * rv[0] + (l['p'][1] - pos[1]) * rv[1]
        return out


    def arm_info(ch, which):
        """Strassenende an einer Kreuzung: Mittelpunkt, Richtung von der Kreuzung weg, Spline,
        und ob die Splinerichtung von der Kreuzung weg zeigt"""
        if which == 's':
            el = ch['els'][0]
            return dict(pos=(el[0], el[1]), h=el[2] % 360, spl=el[5], away=True)
        el = ch['els'][-1]
        pos, h = end_of(el)
        return dict(pos=pos, h=(h + 180) % 360, spl=el[5], away=False)

    arms_at = collections.defaultdict(list)
    arm_meta = collections.defaultdict(list)
    for ch in road_chains:
        if ch['node_s'] in JN:
            arms_at[ch['node_s']].append(lane_ends(ch, 's')); arm_meta[ch['node_s']].append(arm_info(ch, 's'))
        if ch['node_e'] in JN:
            arms_at[ch['node_e']].append(lane_ends(ch, 'e')); arm_meta[ch['node_e']].append(arm_info(ch, 'e'))

    conn_chains, conn_fail, n_moves = [], 0, collections.Counter()
    SPUR = KI + 'AB_kreuzung_spur.sli'
    for k, arms in arms_at.items():
        done = set()
        for ia, A in enumerate(arms):
            lin = sorted([l for l in A if l['kind'] == 'in'], key=lambda l: -l['right'])
            if not lin:
                continue
            for ib, B in enumerate(arms):
                if ia == ib:
                    continue
                lout = sorted([l for l in B if l['kind'] == 'out'], key=lambda l: -l['right'])
                if not lout:
                    continue
                dlt = norm180(lout[0]['h'] - lin[0]['h'])
                if abs(dlt) > 150:
                    continue
                pairs = []
                if abs(dlt) < 35 or len(arms) == 2:
                    mv = 'gerade'
                    pairs = {(i, min(i, len(lout) - 1)) for i in range(len(lin))} | \
                            {(min(j, len(lin) - 1), j) for j in range(len(lout))}
                elif dlt > 0:
                    mv = 'rechts'; pairs = {(0, 0)}
                else:
                    mv = 'links'; pairs = {(len(lin) - 1, len(lout) - 1)}
                for i, j in sorted(pairs):
                    a, b = lin[i], lout[j]
                    kk = (key(a['p']), key(b['p']))
                    if kk in done or math.dist(a['p'], b['p']) < 0.05:
                        continue
                    done.add(kk)
                    spl = SPUR
                    if mv == 'rechts' and len(arms) >= 3:
                        g = (a['gap'] + b['gap']) / 2
                        g = min(KREUZ_GAPS, key=lambda x: abs(x - g))
                        spl = KI + f'AB_kreuzung_ecke_{str(g).replace(".", "_")}.sli'
                    els = connect(a['p'], a['h'], b['p'], b['h'], spl)
                    if not els:
                        conn_fail += 1
                        continue
                    n_moves[mv] += 1
                    conn_chains.append(dict(els=els, name='Kreuzung', hw='junction', node=k, mv=mv,
                                            arms=(ia, ib)))
        # Nachlauf: jede ankommende Spur braucht ein Ziel, jede abgehende Spur einen Zulauf
        allin = [l for A in arms for l in A if l['kind'] == 'in']
        allout = [l for A in arms for l in A if l['kind'] == 'out']
        used_in = {a for a, b in done}; used_out = {b for a, b in done}

        def best_pair(cands):
            cands = [(abs(norm180(b['h'] - a['h'])), math.dist(a['p'], b['p']), id(a), a, b) for a, b in cands
                     if abs(norm180(b['h'] - a['h'])) <= 150 and math.dist(a['p'], b['p']) > 1.0]
            return min(cands)[3:] if cands else None
        for a in allin:
            if key(a['p']) not in used_in:
                pr = best_pair([(a, b) for b in allout if not any(b in A and a in A for A in arms)])
                if pr:
                    els = connect(pr[0]['p'], pr[0]['h'], pr[1]['p'], pr[1]['h'], SPUR)
                    if els:
                        conn_chains.append(dict(els=els, name='Kreuzung', hw='junction', node=k, mv='nachlauf'))
                        n_moves['nachlauf'] += 1
                        used_out.add(key(pr[1]['p']))
        for b in allout:
            if key(b['p']) not in used_out:
                pr = best_pair([(a, b) for a in allin if not any(b in A and a in A for A in arms)])
                if pr:
                    els = connect(pr[0]['p'], pr[0]['h'], pr[1]['p'], pr[1]['h'], SPUR)
                    if els:
                        conn_chains.append(dict(els=els, name='Kreuzung', hw='junction', node=k, mv='nachlauf'))
                        n_moves['nachlauf'] += 1


    return dict(road_chains=road_chains, conn_chains=conn_chains, junctions=[list(k) for k in JN],
                arms=dict(arm_meta), node_pos={k: tuple(k) for k in JN},
                stats=dict(kreuzungen=len(JN), verbindungen=len(conn_chains), fehlgeschlagen=conn_fail,
                           bewegungen=dict(n_moves)))
