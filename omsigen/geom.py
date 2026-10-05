"""Geometrie in OMSI-Konvention: x = Ost, z = Nord, Richtung h in Grad im Uhrzeigersinn ab Nord.
Element = [x, z, h, L, R, spline]; R > 0 Rechtskurve, R = 0 Gerade."""
import math


def heading(a, b):
    return math.degrees(math.atan2(b[0] - a[0], b[1] - a[1])) % 360


def norm180(a):
    return (a + 180) % 360 - 180


def dvec(h):
    r = math.radians(h)
    return (math.sin(r), math.cos(r))


def rvec(h):            # rechte Seite (positiver Querversatz in .sli)
    r = math.radians(h)
    return (math.cos(r), -math.sin(r))


def end_of(el):
    """Endpunkt und Endrichtung eines Elements"""
    x, z, h, L, R = el[:5]
    hr = math.radians(h)
    if R == 0:
        return (x + L * math.sin(hr), z + L * math.cos(hr)), h % 360
    cx, cz = x + R * math.cos(hr), z - R * math.sin(hr)
    h2 = h + math.degrees(L / R)
    h2r = math.radians(h2)
    return (cx - R * math.cos(h2r), cz + R * math.sin(h2r)), h2 % 360


def sample(el, step=1.0):
    x, z, h, L, R = el[:5]
    n = max(2, int(L / step) + 2)
    out = []
    for k in range(n):
        l = L * k / (n - 1)
        out.append(end_of([x, z, h, l, R]) )
    return out


def straight(a, b, spl, maxlen=100.0):
    d = math.dist(a, b)
    if d < 0.02:
        return []
    h = heading(a, b)
    k = max(1, math.ceil(d / maxlen))
    return [[a[0] + (b[0] - a[0]) * j / k, a[1] + (b[1] - a[1]) * j / k, h, d / k, 0.0, spl] for j in range(k)]


def arc_to(p, h, q, spl):
    """Kreisbogen von p (Richtung h) nach q"""
    c = math.dist(p, q)
    if c < 0.02:
        return []
    phi = norm180(heading(p, q) - h)
    if abs(phi) < 0.05:
        return [[p[0], p[1], h % 360, c, 0.0, spl]]
    R = c / (2 * math.sin(math.radians(abs(phi)))) * (1 if phi > 0 else -1)
    L = abs(R) * math.radians(2 * abs(phi))
    return [[p[0], p[1], h % 360, L, R, spl]]


def biarc(a, ha, b, hb, spl):
    t1, t2 = dvec(ha), dvec(hb)
    v = (b[0] - a[0], b[1] - a[1])
    t = (t1[0] + t2[0], t1[1] + t2[1])
    vt = v[0] * t[0] + v[1] * t[1]
    vv = v[0] ** 2 + v[1] ** 2
    t1t2 = t1[0] * t2[0] + t1[1] * t2[1]
    denom = 2 * (1 - t1t2)
    if abs(denom) < 1e-9:
        vt2 = v[0] * t2[0] + v[1] * t2[1]
        if abs(vt2) < 1e-9:
            return None
        al = vv / (4 * vt2)
    else:
        disc = vt * vt + denom * vv
        al = (-vt + math.sqrt(max(disc, 0))) / denom
    if al <= 0:
        return None
    p1 = (a[0] + al * t1[0], a[1] + al * t1[1])
    p2 = (b[0] - al * t2[0], b[1] - al * t2[1])
    pm = ((p1[0] + p2[0]) / 2, (p1[1] + p2[1]) / 2)
    e1 = arc_to(a, ha, pm, spl)
    if not e1:
        return arc_to(a, ha, b, spl)
    _, hm = end_of(e1[0])
    return e1 + arc_to(pm, hm, b, spl)


def connect(a, ha, b, hb, spl):
    """G1-stetige Verbindung von (a,ha) nach (b,hb): Gerade-Bogen-Gerade, sonst Doppelbogen"""
    da, db = dvec(ha), dvec(hb)
    cross = da[0] * db[1] - da[1] * db[0]
    dlt = norm180(hb - ha)
    if abs(cross) > 1e-4 and abs(dlt) > 2:
        w = (b[0] - a[0], b[1] - a[1])
        s = (w[0] * db[1] - w[1] * db[0]) / cross
        t = (da[0] * w[1] - da[1] * w[0]) / cross
        if s > 0.05 and t > 0.05:
            tm = min(s, t)
            half = math.radians(abs(dlt)) / 2
            R = tm / math.tan(half) * (1 if dlt > 0 else -1)
            I = (a[0] + s * da[0], a[1] + s * da[1])
            T1 = (I[0] - tm * da[0], I[1] - tm * da[1])
            T2 = (I[0] + tm * db[0], I[1] + tm * db[1])
            els = straight(a, T1, spl)
            els.append([T1[0], T1[1], ha % 360, abs(R) * math.radians(abs(dlt)), R, spl])
            els += straight(T2, b, spl)
            return rechain(els, b, a, ha)
    els = biarc(a, ha, b, hb, spl)
    if els:
        rechain(els, b, a, ha)
    if ok(els, b, hb):
        return els
    # Ersatz: kurze Geraden an beiden Enden, dazwischen Doppelbogen
    for d in (1.0, 2.5, 5.0):
        da_, db_ = dvec(ha), dvec(hb)
        a2 = (a[0] + d * da_[0], a[1] + d * da_[1]); b2 = (b[0] - d * db_[0], b[1] - d * db_[1])
        mid = biarc(a2, ha, b2, hb, spl)
        els = straight(a, a2, spl) + (mid or []) + straight(b2, b, spl)
        if mid:
            rechain(els, b, a, ha)
        if mid and ok(els, b, hb):
            return els
    return None


def ok(els, b, hb):
    if not els:
        return False
    p, h = end_of(els[-1])
    return math.dist(p, b) < 0.01 and abs(norm180(h - hb)) < 0.1


def chain_end(els):
    return end_of(els[-1])


def rechain(els, b=None, a=None, ha=None):
    """Elemente lueckenlos aneinanderhaengen (Rundungsfehler beseitigen); letzte Gerade exakt auf b ausrichten"""
    if els and a is not None:
        els[0][0], els[0][1] = a
        if ha is not None:
            els[0][2] = ha % 360
    for j in range(1, len(els)):
        (x, z), h = end_of(els[j - 1])
        els[j][0], els[j][1], els[j][2] = x, z, h
    if b is not None and els and els[-1][4] == 0:
        e = els[-1]
        e[3] = math.dist((e[0], e[1]), b)
        if e[3] > 1e-6:
            e[2] = heading((e[0], e[1]), b)
    return els
