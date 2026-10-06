"""Kreuzungsobjekte: jede Kreuzung wird ein eigenes OMSI-Objekt (.sco) mit einer durchgehenden Platte (.x-Modell)
aus Asphalt, Bordsteinen und Gehwegecken sowie den Fahr- und Gehwegpfaden - so wie die Kreuzungen der
Standardkarten (z. B. Grundorf, Kreuz_MC). Die Strassen-Splines enden an den Armen und schliessen exakt an.

Geometrie (lokal, Objekt am Kreuzungsmittelpunkt mit Drehung 0, x = Ost, z = Nord):
- Arme im Uhrzeigersinn sortiert; zwischen dem rechten Fahrbahnrand eines Arms und dem linken des naechsten
  liegt eine gerundete Bordsteinkante (geom.connect).
- Asphalt = Flaeche innerhalb der Fahrbahnraender und Bordsteinkanten.
- Gehwegecken = Streifen links der Bordsteinkante, Breite laeuft vom einen Arm zum anderen.
- Nichts ragt in die Flaeche eines Arms (dort liegt der Spline).
Formatdetails: docs/omsi-format.md (Abschnitt Kreuzungsobjekte)."""
import math, os, shutil
import numpy as np
import shapely
from shapely.geometry import Polygon, LineString, Point
from shapely.ops import unary_union
from .geom import end_of, rvec, dvec, heading, connect, sample, norm180, fillet
from . import vorfahrt, ampel

ASPH_H, WALK_H = 0.10, 0.25          # Hoehe Fahrbahn / Gehweg wie in den Splines
U_KERB, U_OUT = 0.953, 0.187         # str_side1.bmp: u an der Bordsteinkante / 3,5 m weiter aussen
TEXTURES = ['str_asphdrk.bmp', 'str_asphdrk.bmp.cfg', 'str_side1.bmp', 'str_side1.bmp.cfg', 'betonwand1.bmp']
FOOT_LEN = 60.0                      # so weit reicht die freigehaltene Flaeche eines Arms nach aussen
R_KERB = 8.0                         # Bordsteinradius an Ecken (wird kleiner, wenn der Platz nicht reicht)


def _arm(a, sdb):
    """Querschnitt eines Arms in seiner Aussenrichtung (rechts positiv)"""
    p = sdb[a['spl']]
    if a['away']:
        cl, cr, ol, or_, walks = -p['cl'], p['cr'], -p['ol'], p['or_'], list(p['walks'])
    else:
        cl, cr, ol, or_, walks = p['cr'], -p['cl'], p['or_'], -p['ol'], [-w for w in p['walks']]
    r, P = rvec(a['h']), a['pos']

    def pt(d):
        return (P[0] + d * r[0], P[1] + d * r[1])
    return dict(h=a['h'], pos=P, CL=pt(-cl), CR=pt(cr), OL=pt(-ol), OR=pt(or_), wl=ol - cl, wr=or_ - cr,
                walk_l=max((w for w in walks if w < 0), default=None),
                walk_r=min((w for w in walks if w > 0), default=None), pt=pt, ol=ol, or_=or_)


def _corner(p, hp, q, hq, r):
    """Ecke von p (Richtung hp) nach q (Richtung hq): Ausrundung, sonst freie Kurve, sonst Gerade -> Elemente"""
    els = fillet(p, hp, q, hq, r)
    if els is not None:
        return els
    d = math.dist(p, q)
    # nur bei fast gleicher Richtung (durchgehende Strasse mit Versatz) eine S-Kurve; sonst treffen sich die
    # Raender schon hinter den Armenden und werden gerade verbunden
    if d >= 0.3 and abs(norm180(hq - hp)) < 30:
        els = connect(p, hp, q, hq, None)
        if els and sum(e[3] for e in els) < 3 * d + 25:     # keine Schleifen
            return els
    return [[p[0], p[1], heading(p, q), d, 0.0, None]] if d > 0.01 else []


def _curve(p, hp, q, hq, step=0.5):
    """Bordsteinlinie als Liste (punkt, richtung)"""
    els = _corner(p, hp, q, hq, R_KERB)
    out = [(p, hp)]
    for e in els:
        out += sample(e, step)[1:]
    if math.dist(out[-1][0], q) > 0.01:
        out.append((q, hq))
    return out


def _footprint(A):
    a, b = A['OL'], A['OR']
    dv = dvec(A['h'])
    return Polygon([a, b, (b[0] + FOOT_LEN * dv[0], b[1] + FOOT_LEN * dv[1]),
                    (a[0] + FOOT_LEN * dv[0], a[1] + FOOT_LEN * dv[1])])


def _polys(g):
    if g.is_empty:
        return []
    if g.geom_type == 'Polygon':
        return [g]
    return [p for p in getattr(g, 'geoms', []) if p.geom_type == 'Polygon' and p.area > 1e-4]


def build_junction(arms, sdb):
    """arms: Liste dict(pos, h, spl, away) (Weltkoordinaten) -> dict(origin, asphalt, side, kerbs, walks) in
    lokalen Koordinaten oder None, wenn keine Flaeche entsteht"""
    A = [_arm(a, sdb) for a in arms]
    O = (sum(a['pos'][0] for a in A) / len(A), sum(a['pos'][1] for a in A) / len(A))
    order = sorted(range(len(A)), key=lambda i: heading(O, A[i]['pos']))
    ring, corners = [], []
    for n, i in enumerate(order):
        j = order[(n + 1) % len(order)]
        a, b = A[i], A[j]
        ring += [a['CL'], a['CR']]
        kp = _curve(a['CR'], (a['h'] + 180) % 360, b['CL'], b['h'])
        ring += [p for p, _ in kp[1:-1]]
        corners.append((kp, a, b))
    foot = unary_union([_footprint(a) for a in A])
    asphalt = Polygon(ring).buffer(0)
    asphalt = unary_union(_polys(asphalt)).difference(foot)

    # Gehwegecken: Vierecke links der Bordsteinkante, Breite von Arm a (rechts) zu Arm b (links)
    quads, kerbs = [], []
    for kp, a, b in corners:
        if len(kp) < 2 or LineString([p for p, _ in kp]).length < 0.05:
            continue
        L = [0.0]
        for (p, _), (q, _) in zip(kp, kp[1:]):
            L.append(L[-1] + math.dist(p, q))
        outer = []
        for (p, h), s in zip(kp, L):
            w = a['wr'] + (b['wl'] - a['wr']) * (s / L[-1] if L[-1] else 0)
            nv = rvec(h)
            outer.append((p[0] - w * nv[0], p[1] - w * nv[1]))
        for k in range(len(kp) - 1):
            q = Polygon([kp[k][0], kp[k + 1][0], outer[k + 1], outer[k]])
            if q.is_valid and q.area > 1e-6:
                quads.append(q)
            elif q.area > 1e-6:
                quads.append(q.buffer(0))
        kerbs.append(LineString([p for p, _ in kp]))
    side = unary_union(quads).buffer(0.005).buffer(-0.005) if quads else Polygon()
    side = side.difference(asphalt).difference(foot)

    # Gehwegpfade um die Ecken (Gehweg rechts von Arm a -> Gehweg links von Arm b)
    walks = []
    for kp, a, b in corners:
        if a['walk_r'] is None or b['walk_l'] is None:
            continue
        p, q = a['pt'](a['walk_r']), b['pt'](b['walk_l'])
        els = _corner(p, (a['h'] + 180) % 360, q, b['h'], R_KERB + (a['walk_r'] - a['or_'] + a['wr']))
        if els:
            walks.append(els)

    def loc(g):
        return shapely.transform(g, lambda c: c - [O[0], O[1]])
    return dict(origin=O, asphalt=loc(asphalt), side=loc(side), kerbs=[loc(k) for k in kerbs],
                walks=[[[e[0] - O[0], e[1] - O[1]] + list(e[2:5]) for e in els] for els in walks])


# ============================================================ Modell (.x)
def _tris(geom):
    """Dreiecke (im Uhrzeigersinn von oben gesehen = Vorderseite in Direct3D)"""
    out = []
    for poly in _polys(geom):
        for t in shapely.constrained_delaunay_triangles(poly).geoms:
            a, b, c = list(t.exterior.coords)[:3]
            area = (b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])
            if abs(area) < 1e-8:
                continue
            out.append((a, c, b) if area > 0 else (a, b, c))
    return out


def mesh(J):
    """-> (vertices [(x,y,z,nx,ny,nz,u,v)], faces [(a,b,c,material)]); Material 0 Asphalt, 1 Gehweg"""
    V, F = [], []

    def add(pts, n, uvs, m):
        i0 = len(V)
        for (x, y, z), (u, v) in zip(pts, uvs):
            V.append((x, y, z) + n + (u, v))
        F.append((i0, i0 + 1, i0 + 2, m))

    for a, b, c in _tris(J['asphalt']):
        add([(p[0], ASPH_H, p[1]) for p in (a, b, c)], (0, 1, 0), [(p[0] / 3.5, p[1] / 6.0) for p in (a, b, c)], 0)
    kerbs = J['kerbs']

    def side_uv(p):
        P = Point(p)
        if not kerbs:
            return (U_OUT, 0.0)
        k = min(kerbs, key=lambda l: l.distance(P))
        d = k.distance(P)
        return (max(0.0, U_KERB - (U_KERB - U_OUT) * d / 3.5), k.project(P) * 0.2)
    for a, b, c in _tris(J['side']):
        add([(p[0], WALK_H, p[1]) for p in (a, b, c)], (0, 1, 0), [side_uv(p) for p in (a, b, c)], 1)
    # Bordsteinkanten: senkrechte Flaechen, wo Gehweg an Asphalt grenzt
    ab = J['asphalt'].boundary
    for poly in _polys(J['side']):
        for ring in [poly.exterior] + list(poly.interiors):
            cs = list(ring.coords)
            for p, q in zip(cs, cs[1:]):
                if math.dist(p, q) < 1e-3:
                    continue
                m = ((p[0] + q[0]) / 2, (p[1] + q[1]) / 2)
                if ab.distance(Point(m)) > 0.02:
                    continue
                dx, dz = q[0] - p[0], q[1] - p[1]
                L = math.hypot(dx, dz)
                nx, nz = dz / L, -dx / L               # rechts von p->q
                if not J['asphalt'].buffer(1e-3).contains(Point(m[0] + 0.05 * nx, m[1] + 0.05 * nz)):
                    p, q, nx, nz = q, p, -nx, -nz      # Normale muss zur Fahrbahn zeigen
                v0 = side_uv(p)[1]; v1 = v0 + L * 0.2
                top_p, top_q = (p[0], WALK_H, p[1]), (q[0], WALK_H, q[1])
                bot_p, bot_q = (p[0], ASPH_H, p[1]), (q[0], ASPH_H, q[1])
                n = (nx, 0, nz)
                # von der Fahrbahn aus gesehen liegt p links, q rechts; Vorderseite = im Uhrzeigersinn
                add([top_p, top_q, bot_q], n, [(U_KERB, v0), (U_KERB, v1), (0.995, v1)], 1)
                add([top_p, bot_q, bot_p], n, [(U_KERB, v0), (0.995, v1), (0.995, v0)], 1)
    return V, F


def x_file(V, F, mats=None):
    def f(v):
        return f'{v:.4f}'
    mats = mats or [('Asphalt', 'str_asphdrk.bmp'), ('Gehweg', 'str_side1.bmp')]
    L = ['xof 0302txt 0032', '', 'Mesh Kreuzung {', f' {len(V)};']
    L += [f' {f(v[0])};{f(v[1])};{f(v[2])};' + (',' if i < len(V) - 1 else ';') for i, v in enumerate(V)]
    L += [f' {len(F)};']
    L += [f' 3;{a},{b},{c};' + (',' if i < len(F) - 1 else ';') for i, (a, b, c, _) in enumerate(F)]
    L += [' MeshNormals {', f'  {len(V)};']
    L += [f'  {f(v[3])};{f(v[4])};{f(v[5])};' + (',' if i < len(V) - 1 else ';') for i, v in enumerate(V)]
    L += [f'  {len(F)};']
    L += [f'  3;{a},{b},{c};' + (',' if i < len(F) - 1 else ';') for i, (a, b, c, _) in enumerate(F)]
    L += [' }', ' MeshTextureCoords {', f'  {len(V)};']
    L += [f'  {f(v[6])};{f(-v[7])};' + (',' if i < len(V) - 1 else ';') for i, v in enumerate(V)]
    L += [' }', ' MeshMaterialList {', f'  {len(mats)};', f'  {len(F)};']
    L += [f'  {m}' + (',' if i < len(F) - 1 else ';') for i, (_, _, _, m) in enumerate(F)]
    for name, tex in mats:
        L += [f'  Material {name} {{', '   1.000000;1.000000;1.000000;1.000000;;', '   0.000000;',
              '   0.000000;0.000000;0.000000;;', '   0.000000;0.000000;0.000000;;',
              f'   TextureFilename {{ "{tex}"; }}', '  }']
    L += [' }', '}', '']
    return '\r\n'.join(L)


# ============================================================ Objekt (.sco)
def _path_lines(e, height, kind, width, direction, blinker):
    """eine Geraden/Bogen-Strecke als [path] (12 Zeilen, Objektkoordinaten: x rechts, y vorwaerts, z hoch)"""
    x, z, h, L, R = e[:5]
    vals = [x, z, height, h % 360, R, L, 0, 0, kind, width, direction, blinker]
    return ['[path]'] + [f'{v:.4f}' if isinstance(v, float) else str(v) for v in vals] + ['']


def sco_text(name, mesh_file, moves, walks, ampel_block=()):
    """moves: Liste (Elemente, Blinker[, Signalgruppe oder None]); die Signalgruppe gilt fuer das erste Stueck"""
    L = ['Erzeugt mit omsigen (Aschaffenburg-KI). Strassendaten (c) OpenStreetMap-Mitwirkende, ODbL.', '',
         '[friendlyname]', name, '', '[groups]', '1', 'Aschaffenburg_KI', '',
         '[rendertype]', 'surface', '', '[LightMapMapping]', '', '[fixed]', '', '[surface]', '',
         '[absheight]', '']
    L += list(ampel_block)
    for mv in moves:
        els, blinker = mv[0], mv[1]
        gruppe = mv[2] if len(mv) > 2 else None
        for i, e in enumerate(els):
            L += _path_lines(e, ASPH_H, 0, 3.0, 0, blinker)
            if i == 0 and gruppe is not None:
                L += ['[use_traffic_light]', str(gruppe), '']
    for els in walks:
        for e in els:
            L += _path_lines(e, WALK_H, 1, 2.0, 2, 0)
    L += ['[mesh]', mesh_file, '']
    return '\r\n'.join(L) + '\r\n'


BLINKER = {'links': 2, 'rechts': 3}


def build_objects(net, sdb, map_name, korrekturen=(), to_ll=None):
    """-> Liste dict(name, origin (Welt), sco, x, paths, rel, rules, vorfahrt) fuer alle Kreuzungen des Netzes.
    to_ll: (x, z) -> (lat, lon) fuer Korrekturdatei und Bericht"""
    moves_at = {}
    for c in net['conn_chains']:
        moves_at.setdefault(c.get('node'), []).append(c)
    out = []
    # jede Korrektur gilt nur fuer die ihr naechste Kreuzung (mit mindestens 3 Armen, bis 30 m)
    kor_at = {}
    if korrekturen and to_ll:
        centers = {k: to_ll(sum(a['pos'][0] for a in arms) / len(arms), sum(a['pos'][1] for a in arms) / len(arms))
                   for k, arms in net['arms'].items() if len(arms) >= 3}
        for c in korrekturen:
            best = min(((vorfahrt.distance_m(c['lat'], c['lon'], *ll), k) for k, ll in centers.items()), default=None)
            if best and best[0] <= 30:
                kor_at[best[1]] = c
    for n, (k, arms) in enumerate(sorted(net['arms'].items())):
        J = build_junction(arms, sdb)
        if J is None or J['asphalt'].is_empty:
            continue
        O = J['origin']
        ll = to_ll(*O) if to_ll else None
        kor = kor_at.get(k)
        V_ = vorfahrt.decide(arms, net.get('node_flags', {}).get(k, ()), kor) if len(arms) >= 3 else None
        n_haupt = V_['rollen'].count(vorfahrt.HAUPT) if V_ else 0
        mit_ampel = bool(V_) and (kor.get('ampel', V_['ampel']) if kor else V_['ampel'])
        plan = ampel.plane(arms, V_['rollen']) if mit_ampel else None
        moves, rules, idx = [], [], 0
        for c in moves_at.get(k, []):
            els = [[e[0] - O[0], e[1] - O[1]] + list(e[2:5]) for e in c['els']]
            gruppe = plan['phase'][c['arms'][0]] if (plan and c.get('arms')) else None
            moves.append((els, BLINKER.get(c.get('mv'), 0), gruppe))
            if V_ and c.get('arms'):
                ia, ib = c['arms']
                pr = vorfahrt.priority(V_['rollen'][ia], V_['rollen'][ib], c.get('mv'), n_haupt)
                if pr is not None:
                    rules += [(idx + j, pr) for j in range(len(els))]
            idx += len(els)
        V, F = mesh(J)
        name = f'K_{n + 1:03d}'
        info = None
        if V_:
            info = dict(V_, ampel=mit_ampel, lat=ll[0] if ll else None, lon=ll[1] if ll else None, x=O[0], z=O[1],
                        umlauf=plan['umlauf'] if plan else None,
                        arme=[dict(name=a['name'], strasse=a['tags'].get('highway'), schild=a['sign'], rolle=r)
                              for a, r in zip(arms, V_['rollen'])])
        flaeche = []                       # Punkte (Welt) auf der Kreuzungsflaeche, fuer das Gelaende darunter
        ges = J['asphalt'].union(J['side'])
        if not ges.is_empty:
            bx0, bz0, bx1, bz1 = ges.bounds
            for gx in np.arange(bx0, bx1 + 0.1, 2.0):
                for gz in np.arange(bz0, bz1 + 0.1, 2.0):
                    if ges.contains(Point(gx, gz)):
                        flaeche.append((gx + O[0], gz + O[1]))
        out.append(dict(name=name, origin=O, knoten=k, flaeche=flaeche, x=x_file(V, F), faces=len(F), rules=rules,
                        vorfahrt=info,
                        signale=ampel.signale(arms, plan, sdb) if plan else [],
                        sco=sco_text(f'{map_name} Kreuzung {n + 1}', name + '.x', moves, J['walks'],
                                     ampel.sco_block(plan) if plan else ()),
                        paths=idx + sum(len(w) for w in J['walks']),
                        rel=f'Sceneryobjects\\Aschaffenburg_KI\\{map_name}\\{name}.sco', geom=J))
    return out


def install_objects(root, map_name, objects, omsi_dir=None, overwrite=False):
    """Dateien nach <root>/Sceneryobjects/Aschaffenburg_KI/<Karte>/ schreiben (Modelle in model/, Texturen in
    texture/, kopiert aus Splines/Marcel/texture der OMSI-Installation)"""
    d = os.path.join(root, 'Sceneryobjects', 'Aschaffenburg_KI', map_name)
    if os.path.exists(d):
        if not overwrite:
            raise FileExistsError(f'Objektordner {d} existiert schon - anderen Kartennamen waehlen')
        shutil.rmtree(d)
    os.makedirs(os.path.join(d, 'model')); os.makedirs(os.path.join(d, 'texture'))
    for o in objects:
        with open(os.path.join(d, o['name'] + '.sco'), 'w', encoding='cp1252', newline='') as f:
            f.write(o['sco'])
        with open(os.path.join(d, 'model', o['name'] + '.x'), 'w', encoding='ascii', newline='') as f:
            f.write(o['x'])
    copied = 0
    for src_root in (omsi_dir, root):
        src = os.path.join(src_root, 'Splines', 'Marcel', 'texture') if src_root else None
        if src and os.path.isdir(src):
            for t in TEXTURES:
                if os.path.exists(os.path.join(src, t)) and not os.path.exists(os.path.join(d, 'texture', t)):
                    shutil.copy(os.path.join(src, t), os.path.join(d, 'texture', t)); copied += 1
    return d, copied
