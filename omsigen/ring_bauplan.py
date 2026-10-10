"""Bauplan der Kreuzung Ludwigsallee (Sonderbau Ring): alle Bauteile des Kreuzungsobjekts als bearbeitbare Liste.

Der Objekt-Editor (omsi-editor, objekteditor.rs) bearbeitet genau diese Datei (sonderbau/ring/bauplan.json); das Objekt
entsteht immer daraus neu (`bauen`). Fehlt die Datei, legt `vorgabe` sie aus dem Luftbild-Stand (ring_kreuzung) an.

Format (Kartenkoordinaten in m, Richtungen in Grad im Uhrzeigersinn ab Nord):
    flaechen:   [{id, art, punkte: [[x, y]...], loecher: [[[x, y]...]], textur: {dx, dy, skala, dreh}}]
                art: fahrbahn | gehweg | insel | gruen (erhoeht) | sperr | radfurt | busbeton (auf der Fahrbahn)
                Erhoehte Flaechen schneiden die Fahrbahn aus (Bordstein entsteht an der Grenze).
    linien:     [{id, punkte, breite, an, aus}]  weiss; aus = 0: durchgezogen, sonst Strich `an` m, Luecke `aus` m
    haltlinien: [{id, arm, punkte: [a, b], breite}]  die Fahrzeuge der Zufahrt `arm` halten dort
    symbole:    [{id, art: pfeil | bus, pos, richtung, pfeile: [links, gerade, rechts], skala}]
                pos = Fuss des Pfeils / Mitte unten der Schrift, richtung = Fahrtrichtung
    ampeln:     [{id, art: kfz | fg, pos, richtung, mast: klein | ausleger, gruppe}]
                richtung = wohin das Signal schaut (zum wartenden Verkehr); der Mast steht dahinter
    gelaender:  [{id, punkte, unten: gelaende | trog}]  Betonkante mit Stabgelaender, aussen = weg von Gehweg/Insel
    furten:     [{id, a, b, breite, gruppe}]  Fussgaengerfurt (Randlinien, Fussgaengerpfad an der Ampelgruppe)
    gehpfade:   [{id, punkte}]  Fussgaengerpfade auf Gehwegen
    tunnel:     {x0, x1, sohle}  Bahntunnel unter der Kreuzung
    verbindungen, phasen: Abbiegespuren [Arm rein, Spur, Arm raus, Spur] und Ampelphase je Arm
    lsw:        {w: [[s0, s1]...], o: [...]}  Laermschutzwaende (Achs-Stationen) des Bauwerks"""
import json, math, os
import numpy as np
from shapely.geometry import Polygon, Point, LineString, box
from shapely.ops import unary_union
from . import ampel, vorfahrt
from .geom import connect, end_of, dvec, rvec
from .kreuzung import ASPH_H, WALK_H, BLINKER
from . import ring_kreuzung as rk

# Flaechenart -> (Material, Hoehe ueber der Kreuzungsflaeche, Meter je Textur)
ARTEN = {'fahrbahn': ('asphalt', ASPH_H, 4.0), 'gehweg': ('gehweg_asphalt', WALK_H, 4.0), 'insel': ('insel_beton', WALK_H, 3.0),
         'gruen': ('pflanzen', WALK_H + 0.03, 2.0), 'busbeton': ('busbeton', ASPH_H + 0.01, 3.9),
         'sperr': ('sperr', ASPH_H + 0.015, 2.0), 'radfurt': ('radfurt', ASPH_H + 0.02, 4.0)}
ERHOEHT = ('gehweg', 'insel', 'gruen')
AUF_FAHRBAHN = ('busbeton', 'sperr', 'radfurt')
WEISS_H = ASPH_H + 0.025


def _r(v):
    return [round(float(v[0]), 3), round(float(v[1]), 3)]


def _ring(c):
    c = [_r(p) for p in c]
    return c[:-1] if len(c) > 1 and c[0] == c[-1] else c


class Ids:
    def __init__(self):
        self.n = 0

    def __call__(self, art):
        self.n += 1
        return f'{art}{self.n}'


def flaechen_aus(geom, art, neu, toleranz=0.03):
    out = []
    for p in rk._polys(geom.simplify(toleranz)):
        out.append(dict(id=neu(art), art=art, punkte=_ring(p.exterior.coords), loecher=[_ring(r.coords) for r in p.interiors],
                        textur=dict(dx=0.0, dy=0.0, skala=1.0, dreh=0.0)))
    return out


def ketten(segmente, tol=0.02):
    """Strecken [(a, b, art)] zu Linienzuegen gleicher Art verbinden"""
    out = []
    for a, b, art in segmente:
        if out and out[-1][1] == art and math.dist(out[-1][0][-1], a) < tol:
            out[-1][0].append(b)
        else:
            out.append(([a, b], art))
    return out


# ------------------------------------------------------------------ Vorgabe aus dem Luftbild-Stand
def vorgabe(achse, K, strassen, arme_roh, gel, verbindungen, phasen, pfeile, rampen_namen):
    from . import ring_strassen as rs
    neu = Ids()
    arme = [rk.Arm(i, a) for i, a in enumerate(arme_roh)]
    T = rk.trog(achse, K)
    rampen = _rampen(achse, K, strassen, arme, rampen_namen)
    F = rk.flaechen(achse, arme, T, rampen)
    plan = dict(version=1, name='Kreuzung_Ludwigsallee', flaechen=[], linien=[], haltlinien=[], symbole=[], ampeln=[],
                gelaender=[], furten=[], gehpfade=[], verbindungen=[list(v) for v in verbindungen], phasen=list(phasen))
    fl = plan['flaechen']
    fl += flaechen_aus(F['fahrbahn_roh'], 'fahrbahn', neu)
    fl += flaechen_aus(F['gehweg'], 'gehweg', neu)
    gruen = F['inseln'].intersection(box(rk.INSEL_O[0][0] - 1.5, rk.INSEL_O[0][1], rk.INSEL_O[1][0], rk.INSEL_O[1][1]))
    fl += flaechen_aus(F['inseln'].difference(gruen), 'insel', neu)
    fl += flaechen_aus(gruen, 'gruen', neu)
    lin = plan['linien']

    def linie(punkte, breite=0.12, an=0.0, aus=0.0):
        lin.append(dict(id=neu('linie'), punkte=[_r(p) for p in punkte], breite=breite, an=an, aus=aus))
    # Zufahrten und Ausfahrten: Sperrflaechen, Busspur-Beton, Linien, Haltlinien, Pfeile, BUS
    for a in arme:
        rein, raus = a.spuren()
        if a.i in rk.HALT:
            ende = a.bis(rk.HALT[a.i])
        elif a.i in rk.RAND:
            ende = a.bis(rk.RAND[a.i])
        else:
            continue
        if a.i in rk.HALT and rein:
            lo = min(q - t['breite'] / 2 for q, t in rein)
            hi = max(q + t['breite'] / 2 for q, t in rein)
            plan['haltlinien'].append(dict(id=neu('halt'), arm=a.i, punkte=[_r(a.punkt(ende - 0.25, lo)), _r(a.punkt(ende - 0.25, hi))], breite=0.5))
        teile = rk.raender(a.q, nur_fahrbahn=True)
        for k, (t, lo, hi) in enumerate(teile):
            if t.get('belag') in ('sperr', 'beton'):
                art = 'sperr' if t['belag'] == 'sperr' else 'busbeton'
                fl += flaechen_aus(Polygon([a.punkt(0, lo), a.punkt(0, hi), a.punkt(ende, hi), a.punkt(ende, lo)]), art, neu)
            if k == 0:
                continue
            vorne = teile[k - 1][0]
            gleich = vorne['art'] == 'fahrspur' and t['art'] == 'fahrspur' and vorne['richtung'] == t['richtung']
            if gleich:
                cut = max(0.0, ende - 20.0)
                if cut > 0.5:
                    linie([a.punkt(0, lo), a.punkt(cut, lo)], 0.12, 3.0, 6.0)
                linie([a.punkt(cut, lo), a.punkt(ende, lo)])
            else:
                linie([a.punkt(0, lo), a.punkt(ende, lo)])
        richt_rein = (a.h + 180) % 360
        for k, (qq, t) in enumerate(rein):
            arten = pfeile.get((a.i, k))
            if t['art'] == 'busspur':
                if ende > 6:
                    plan['symbole'].append(dict(id=neu('bus'), art='bus', pos=_r(a.punkt(ende - 5.5, qq)), richtung=round(richt_rein, 2), pfeile=[], skala=1.0))
                continue
            if arten:
                for d in (4.5, 14.5):
                    if ende - d - 5 > 0.5:
                        plan['symbole'].append(dict(id=neu('pfeil'), art='pfeil', pos=_r(a.punkt(ende - d - 5, qq)), richtung=round(richt_rein, 2),
                                                    pfeile=list(arten), skala=1.0))
        for qq, t in raus:
            if t['art'] == 'busspur' and ende > 9:
                plan['symbole'].append(dict(id=neu('bus'), art='bus', pos=_r(a.punkt(4.0, qq)), richtung=round(a.h, 2), pfeile=[], skala=1.0))
    # Radfurten, Radaufstellflaeche, Sperrflaeche vor der Insel
    for y0, y1 in rk.ROT:
        fl += flaechen_aus(box(rk.ROT_X[0], y0, rk.ROT_X[1], y1), 'radfurt', neu)
        for y in (y0, y1):
            linie([(rk.ROT_X[0], y), (rk.ROT_X[1], y)], 0.15, 0.5, 0.5)
    fl += flaechen_aus(box(rk.RADBOX[0][0], rk.RADBOX[0][1], rk.RADBOX[1][0], rk.RADBOX[1][1]), 'radfurt', neu)
    linie([(rk.HALT_RAD[0] + 0.15, rk.HALT_RAD[2]), (rk.HALT_RAD[0] + 0.15, -255.9)], 0.3)
    fl += flaechen_aus(Polygon(rk.SPERR_O), 'sperr', neu)
    linie(list(rk.SPERR_O) + [rk.SPERR_O[0]], 0.14)
    # Furten, Gehpfade
    for f in rk.FURTEN:
        plan['furten'].append(dict(id=neu('furt'), a=_r(f['a']), b=_r(f['b']), breite=f['breite'], gruppe=f['gruppe']))
    for g in rk.GEHPFADE:
        plan['gehpfade'].append(dict(id=neu('gehpfad'), punkte=[_r(p) for p in g]))
    # Ampeln: Kfz an den Haltlinien, Fussgaenger an den Furten
    for ia, (rechts, links) in rk.MASTEN.items():
        a = arme[ia]
        for wo, p in (('rechts', rechts), ('links', links)):
            plan['ampeln'].append(dict(id=neu('ampel'), art='kfz', pos=_r(p), richtung=round(a.h % 360, 2),
                                       mast='ausleger' if (wo == 'rechts' and ia in rk.AUSLEGER) else 'klein', gruppe=phasen[ia]))
    for f in rk.FURTEN:
        a, b = np.array(f['a']), np.array(f['b'])
        d = (b - a) / np.linalg.norm(b - a)
        for p, gegen, q in ((a, b, a - d * 0.8), (b, a, np.array(f['sig_b']) if 'sig_b' in f else b + d * 0.8)):
            blick = math.degrees(math.atan2(gegen[0] - p[0], gegen[1] - p[1])) % 360
            plan['ampeln'].append(dict(id=neu('ampel'), art='fg', pos=_r(q), richtung=round((blick + 180) % 360, 2), mast='klein', gruppe=f['gruppe']))
    # Gelaender: wie bisher an den Eck-Gehwegen ueber dem Bahneinschnitt und um die Trogenden
    segmente = []
    for g in F['bruestung']:
        for p in rk._polys(g):
            c = list(p.exterior.coords)
            for a_, b_ in zip(c[:-1], c[1:]):
                mitte = Point((a_[0] + b_[0]) / 2, (a_[1] + b_[1]) / 2)
                if F['fahrbahn'].buffer(0.3).contains(mitte):
                    continue
                if any(z.exterior.distance(mitte) < 0.05 for z in rk.GEHWEG_ZONEN) and not any(
                        abs(a_[1] - y_) < 0.05 and abs(b_[1] - y_) < 0.05 for y_ in rk.ECK_ENDE.values()):
                    continue
                segmente.append((a_, b_, 'gelaende'))
    # (die Trogenden bekommen Bruestung und Gelaender vom Portal-Spline, ring_waende.portal_splines)
    for punkte, art in ketten(segmente):
        plan['gelaender'].append(dict(id=neu('gelaender'), punkte=[_r(p) for p in LineString(punkte).simplify(0.02).coords], unten=art))
    plan['tunnel'] = dict(x0=rk.BAHN_X[0], x1=rk.BAHN_X[1], sohle=rk.BAHN_SOHLE)
    # Leitlinien der Linksabbieger aus den Pfaden
    zf = rk.hoehenflaeche(arme)
    _, _, kt = pfade(plan, arme, zf)
    for k in kt:
        if k['mv'] != 'links':
            continue
        lp = [tuple(np.array(e[:2]) - np.array(rvec(e[2])) * 1.6) for e in k['teile']]
        if len(lp) > 2:
            linie(lp[1:-1], 0.12, 1.5, 1.5)
    plan['_id'] = neu.n
    return plan


def _rampen(achse, K, strassen, arme, rampen_namen):
    rampen = {}
    for a in arme:
        if a.name not in rampen_namen:
            continue
        s_ende, t_mitte = achse.projektion([a.p])[0]
        st = next(s for s in strassen if s['name'] == a.name)
        teile = rk.raender(st['querschnitt'])
        t0, t1 = st['versatz'] + teile[0][1], st['versatz'] + teile[-1][2]
        if t_mitte > 0:
            t0 = min(t0, float(K['ring_w'].t([s_ende])[0]))
        else:
            t1 = max(t1, float(K['ring_o'].t([s_ende])[0]))
        rampen[a.name] = (s_ende, 131.0, t0, t1)
    return rampen


# ------------------------------------------------------------------ Geometrie der Bauteile
ANSCHLUSS_L = 1.5          # m: so weit uebernimmt die Platte an jedem Arm den Querschnitt der Strasse


def _art_teil(t):
    """Bauplan-Flaechenart fuer ein Baukasten-Teil der anschliessenden Strasse"""
    art, belag = t.get('art'), str(t.get('belag', ''))
    if art in ('gehweg', 'radweg'):
        return 'gehweg'
    if art == 'gruen':
        return 'gruen'
    if art == 'mittelinsel':
        return 'insel'
    if belag == 'sperr':
        return 'sperr'
    if belag.startswith('beton'):
        return 'busbeton'
    return 'fahrbahn'


def anschluesse(plan, arme):
    """Anschluss an die Strassen: je Arm die ersten ANSCHLUSS_L m genau im Querschnitt der Strasse (Teil fuer Teil);
    die Flaechen des Bauplans treten dort zurueck -> neue Flaechenliste (Kopie)"""
    stuecke = []
    for a in arme:
        for t, qa, qb in rk.raender(a.q):
            r_ = Polygon([a.punkt(0, qa), a.punkt(0, qb), a.punkt(ANSCHLUSS_L, qb), a.punkt(ANSCHLUSS_L, qa)])
            art = _art_teil(t)
            stuecke.append((art, r_))
            if art in AUF_FAHRBAHN:                      # Belag auf der Fahrbahn: darunter Fahrbahn
                stuecke.append(('fahrbahn', r_))
    if not stuecke:
        return plan['flaechen']
    bereich = unary_union([g for _, g in stuecke])
    out = [dict(id=f'anschluss{i}', art=art, punkte=[list(c) for c in g.exterior.coords[:-1]], loecher=[],
                textur=dict(dx=0, dy=0, skala=1, dreh=0)) for i, (art, g) in enumerate(stuecke)]
    for f in plan['flaechen']:
        rest = poly(f).difference(bereich)
        for g in rk._polys(rest):
            if g.area > 1e-3:
                out.append(dict(f, punkte=[list(c) for c in g.exterior.coords[:-1]], loecher=[[list(c) for c in i.coords[:-1]] for i in g.interiors]))
    return out


def poly(f):
    p = Polygon(f['punkte'], f.get('loecher') or [])
    return p if p.is_valid else p.buffer(0)


def linie_geom(l):
    L = LineString(l['punkte'])
    if L.length < 1e-3:
        return Polygon()
    if l.get('aus', 0) > 0 and l.get('an', 0) > 0:
        return rk.gestrichelt(L, l['breite'], l['an'], l['aus'])
    return L.buffer(l['breite'] / 2, cap_style='flat', join_style='mitre')


def symbol_geom(s):
    p = np.array(s['pos'], float)
    k = s.get('skala', 1.0)
    vor, rechts = np.array(dvec(s['richtung'])) * k, np.array(rvec(s['richtung'])) * k
    if s['art'] == 'bus':
        return rk.bus_schrift(p, vor, rechts)
    return rk.pfeil(s.get('pfeile') or ['gerade'], p, vor, rechts)


def furt_linien(f):
    a, b = np.array(f['a'], float), np.array(f['b'], float)
    if np.linalg.norm(b - a) < 0.1:
        return []
    d = (b - a) / np.linalg.norm(b - a)
    nq = np.array([-d[1], d[0]]) * f['breite'] / 2
    return [rk.gestrichelt(LineString([a + nq * s, b + nq * s]), 0.12, 0.5, 0.2) for s in (1, -1)]


# ------------------------------------------------------------------ Pfade
def pfade(plan, arme, zf):
    """-> (pfade, regeln, ketten): Zufahrten bis zur Haltlinie, Abbiegespuren, Fussgaenger"""
    out, regeln, kt = [], [], []
    rollen = [a.rolle for a in arme]
    phasen = plan['phasen']

    def hoehe_el(el):
        x, y, h, L, R = el
        (ex, ey), _ = end_of(el)
        z0 = float(zf(x, y))
        return z0 + ASPH_H, float(zf(ex, ey)) - z0
    halt_pkt = {}
    for a in arme:
        rein, _ = a.spuren()
        hl = [h for h in plan['haltlinien'] if h['arm'] == a.i]
        ende = 0.0
        if hl:
            m = (np.array(hl[0]['punkte'][0]) + np.array(hl[0]['punkte'][1])) / 2
            ende = max(0.0, float(np.dot(a.p - m, a.weg)) - hl[0].get('breite', 0.5) / 2)
        for k, (qq, t) in enumerate(rein):
            start, ziel = a.punkt(0, qq), a.punkt(ende, qq)
            halt_pkt[(a.i, k)] = (ziel, (a.h + 180) % 360)
            if ende > 0.1:
                for e in rk.stuecke([start[0], start[1], (a.h + 180) % 360, float(np.hypot(*(ziel - start))), 0.0]):
                    out.append((e, 0, None, hoehe_el(e), t['breite']))
    for ia, i, ib, j in plan['verbindungen']:
        if (ia, i) not in halt_pkt:
            continue
        p_in, h_in = halt_pkt[(ia, i)]
        _, raus = arme[ib].spuren()
        if j >= len(raus):
            continue
        qq, t = raus[j]
        b = arme[ib]
        els = connect(tuple(p_in), h_in, tuple(b.punkt(0, qq)), b.h, 'x')
        if not els:
            continue
        mv = rk.art(h_in, b.h)
        erster = len(out)
        teile = [e2 for e in els for e2 in rk.stuecke(list(e[:5]))]
        for n, e in enumerate(teile):
            out.append((e, BLINKER.get(mv, 0), phasen[ia] if n == 0 else None, hoehe_el(e), min(3.0, t['breite'])))
        pr = vorfahrt.priority(rollen[ia], rollen[ib], 'links' if mv == 'wenden' else mv, rollen.count('haupt'))
        if pr is not None:
            regeln += [(erster + k, pr) for k in range(len(teile))]
        kt.append(dict(von=(ia, i), nach=(ib, j), mv=mv, teile=teile))
    for f in plan['furten']:
        a, b = np.array(f['a'], float), np.array(f['b'], float)
        L = float(np.linalg.norm(b - a))
        if L < 0.2:
            continue
        h = math.degrees(math.atan2(b[0] - a[0], b[1] - a[1])) % 360
        for n_, e in enumerate(rk.stuecke([a[0], a[1], h, L, 0.0], 3.0)):
            z0, dz = hoehe_el(e)
            out.append((e, 0, f['gruppe'] if n_ == 0 else None, (z0 - ASPH_H + WALK_H if n_ == 0 else z0, dz), 2.0, 1))
    for g in plan['gehpfade']:
        for p, q in zip(g['punkte'][:-1], g['punkte'][1:]):
            if math.dist(p, q) < 0.05:
                continue
            h = math.degrees(math.atan2(q[0] - p[0], q[1] - p[1])) % 360
            e = [p[0], p[1], h, math.dist(p, q), 0.0]
            z0, dz = hoehe_el(e)
            out.append((e, 0, None, (z0 - ASPH_H + WALK_H, dz), 2.0, 1))
    return out, regeln, kt


# ------------------------------------------------------------------ Bauen
def bauen(plan, achse, K, arme_roh, gel, oeffnungen=None):
    """Kreuzungsobjekt aus dem Bauplan -> dict wie ring_kreuzung.bauen (mesh, O, pfade, regeln, ketten, plan, signale,
    flaechen, zf, trog, tunnel, lose_signale). oeffnungen: {nord, sued: Polygon} - Trogoeffnungen aus den Wandlinien
    (ring_waende.oeffnungen; Anschluss: die Platte endet in der Wandmitte)"""
    from .sonderbau_ring import Mesh, stabgelaender
    arme = [rk.Arm(i, a) for i, a in enumerate(arme_roh)]
    zf = rk.hoehenflaeche(arme)
    T = rk.trog(achse, K)
    for name, poly_ in (oeffnungen or {}).items():
        T[name]['offen'] = poly_
    offen = unary_union([T['nord']['offen'], T['sued']['offen']])
    plan = dict(plan, flaechen=anschluesse(plan, arme))
    nach_art = {a: unary_union([poly(f) for f in plan['flaechen'] if f['art'] == a] or [Polygon()]) for a in ARTEN}
    erhoeht = unary_union([nach_art[a] for a in ERHOEHT]).difference(offen)
    fahrbahn = nach_art['fahrbahn'].difference(erhoeht).difference(offen)
    xs = [a.p[0] for a in arme]
    ys = [a.p[1] for a in arme]
    O = np.array([(min(xs) + max(xs)) / 2, (min(ys) + max(ys)) / 2])
    m = Mesh()

    def flaeche(key, geom, dz, uv_m, tx=None):
        tx = tx or {}
        sk = uv_m * max(0.05, tx.get('skala', 1.0))
        c_, s_ = math.cos(math.radians(tx.get('dreh', 0.0))), math.sin(math.radians(tx.get('dreh', 0.0)))
        for c in rk.fein_dreiecke(geom):
            P = [(x - O[0], y - O[1], float(zf(x, y)) + dz) for x, y in c]
            uv = [((c_ * x + s_ * y) / sk + tx.get('dx', 0.0), (-s_ * x + c_ * y) / sk + tx.get('dy', 0.0)) for x, y in c]
            m.dreieck(key, P, uv, (0, 0, 1))
    # Flaechen einzeln (jede mit ihrer Textur-Lage); spaetere Flaechen derselben Art liegen nicht doppelt
    weiss = []
    for l in plan['linien']:
        weiss.append(linie_geom(l))
    for h in plan['haltlinien']:
        weiss.append(LineString(h['punkte']).buffer(h.get('breite', 0.5) / 2, cap_style='flat'))
    for s in plan['symbole']:
        weiss.append(symbol_geom(s))
    for f in plan['furten']:
        weiss += furt_linien(f)
    weiss = unary_union([w for w in weiss if not w.is_empty] or [Polygon()]).intersection(fahrbahn.buffer(0.01))
    belegt = {a: Polygon() for a in ARTEN}
    darueber = {'busbeton': unary_union([nach_art['sperr'], nach_art['radfurt']]), 'sperr': nach_art['radfurt'], 'radfurt': Polygon()}
    for f in plan['flaechen']:
        key, dz, uv_m = ARTEN[f['art']]
        g = poly(f).difference(belegt[f['art']])
        belegt[f['art']] = belegt[f['art']].union(poly(f))
        if f['art'] == 'fahrbahn':
            g = g.intersection(fahrbahn)
        elif f['art'] in ERHOEHT:
            g = g.difference(offen)
            if f['art'] != 'gruen':
                g = g.difference(nach_art['gruen'])
        else:
            g = g.intersection(fahrbahn).difference(weiss).difference(darueber[f['art']])
        if not g.is_empty:
            flaeche(key, g, dz, uv_m, f.get('textur'))
    if not weiss.is_empty:
        flaeche('weiss', weiss, WEISS_H, 1.0)
    # Bordsteine
    fb = fahrbahn.buffer(0.05)
    for e_ in rk._polys(erhoeht):
        for ring in [e_.exterior] + list(e_.interiors):
            c = list(ring.coords)
            for p, q in zip(c[:-1], c[1:]):
                L = math.dist(p, q)
                if L < 1e-3:
                    continue
                mitte = ((p[0] + q[0]) / 2, (p[1] + q[1]) / 2)
                if not fb.contains(Point(mitte)):
                    continue
                n = np.array([q[1] - p[1], p[0] - q[0]]) / L
                if not fahrbahn.buffer(0.02).contains(Point(mitte[0] + n[0] * 0.1, mitte[1] + n[1] * 0.1)):
                    n = -n
                zp, zq = float(zf(*p)), float(zf(*q))
                m.quad('bord', [(p[0] - O[0], p[1] - O[1], zp + ASPH_H), (q[0] - O[0], q[1] - O[1], zq + ASPH_H),
                                (q[0] - O[0], q[1] - O[1], zq + WALK_H), (p[0] - O[0], p[1] - O[1], zp + WALK_H)],
                       [(0, 0), (L / 2, 0), (L / 2, 0.08), (0, 0.08)], (n[0], n[1], 0))
    # Gelaender
    tun = plan.get('tunnel') or {}
    innen = erhoeht.buffer(0.02)
    u_lauf = 0.0
    for g_ in plan['gelaender']:
        P = g_['punkte']
        for a_, b_ in zip(P[:-1], P[1:]):
            L = math.dist(a_, b_)
            if L < 1e-3:
                continue
            u_lauf += L
            za, zb = float(zf(*a_)) + WALK_H, float(zf(*b_)) + WALK_H
            n = np.array([b_[1] - a_[1], a_[0] - b_[0]]) / L
            if innen.contains(Point((a_[0] + b_[0]) / 2 + n[0] * 0.1, (a_[1] + b_[1]) / 2 + n[1] * 0.1)):
                n = -n
            if g_.get('flip'):
                n = -n
            A = lambda p, dn, z: (p[0] + n[0] * dn - O[0], p[1] + n[1] * dn - O[1], z)
            if g_.get('unten') == 'trog':
                continue                         # ersetzt durch den Portal-Spline (ring_waende.portal_splines)
            else:
                ga, gb = (float(gel.h(np.array([q[0] + n[0]]), np.array([q[1] + n[1]]))[0]) for q in (a_, b_))
                unten_a, unten_b = min(ga, za) - 0.5, min(gb, zb) - 0.5
                if tun and tun['x0'] - 0.3 <= (a_[0] + b_[0]) / 2 <= tun['x1'] + 0.3:
                    unten_a, unten_b = za - 1.45, zb - 1.45
            hoch = 0.25
            u0 = u_lauf - L
            m.quad('beton', [A(a_, 0.25, unten_a), A(b_, 0.25, unten_b), A(b_, 0.25, zb + hoch), A(a_, 0.25, za + hoch)],
                   [(u0 / 4, unten_a / 4), (u_lauf / 4, unten_b / 4), (u_lauf / 4, (zb + hoch) / 4), (u0 / 4, (za + hoch) / 4)], (n[0], n[1], 0))
            m.quad('beton', [A(b_, 0.0, zb), A(a_, 0.0, za), A(a_, 0.0, za + hoch), A(b_, 0.0, zb + hoch)],
                   [(0, 0), (L / 4, 0), (L / 4, 0.2), (0, 0.2)], (-n[0], -n[1], 0))
            m.quad('beton', [A(a_, 0.0, za + hoch), A(b_, 0.0, zb + hoch), A(b_, 0.25, zb + hoch), A(a_, 0.25, za + hoch)],
                   [(0, 0), (L / 4, 0), (L / 4, 0.06), (0, 0.06)], (0, 0, 1))
            stabgelaender(m, A(a_, 0.12, 0), A(b_, 0.12, 0), za + hoch, zb + hoch, 1.0, u0)
    # Bahntunnel
    tunnel_y = None
    if tun:
        tunnel = unary_union([fahrbahn, erhoeht]).buffer(0.01).intersection(box(tun['x0'], -320.0, tun['x1'], -200.0))
        tunnel_y = (tunnel.bounds[1], tunnel.bounds[3]) if not tunnel.is_empty else None
        for c in rk.fein_dreiecke(tunnel, 3.0):
            m.dreieck('decke', [(x - O[0], y - O[1], float(zf(x, y)) - 1.2) for x, y in c], [(x / 4, y / 4) for x, y in c], (0, 0, -1))
        if tunnel_y:
            for wx, nx in ((tun['x0'], 1.0), (tun['x1'], -1.0)):
                ys_ = np.arange(tunnel_y[0], tunnel_y[1] + 0.01, 2.0)
                ys_ = np.append(ys_, tunnel_y[1]) if ys_[-1] < tunnel_y[1] - 0.05 else ys_
                for y0, y1 in zip(ys_[:-1], ys_[1:]):
                    z0, z1 = float(zf(wx, y0)) - 1.2, float(zf(wx, y1)) - 1.2
                    m.quad('klinker', [(wx - O[0], y0 - O[1], tun['sohle'] - 0.5), (wx - O[0], y1 - O[1], tun['sohle'] - 0.5),
                                       (wx - O[0], y1 - O[1], z1), (wx - O[0], y0 - O[1], z0)],
                           [(y0 / 2, tun['sohle']), (y1 / 2, tun['sohle']), (y1 / 2, z1), (y0 / 2, z0)], (nx, 0, 0))
            for p in rk._polys(tunnel):
                c = list(p.exterior.coords)
                for a_, b_ in zip(c[:-1], c[1:]):
                    if abs(a_[0] - b_[0]) < 0.05 and min(abs(a_[0] - tun['x0']), abs(a_[0] - tun['x1'])) < 0.05:
                        continue
                    L = math.dist(a_, b_)
                    if L < 0.05:
                        continue
                    n = np.array([b_[1] - a_[1], a_[0] - b_[0]]) / L
                    if tunnel.contains(Point((a_[0] + b_[0]) / 2 + n[0] * 0.2, (a_[1] + b_[1]) / 2 + n[1] * 0.2)):
                        n = -n
                    za, zb = float(zf(*a_)), float(zf(*b_))
                    m.quad('beton', [(a_[0] - O[0], a_[1] - O[1], za - 1.45), (b_[0] - O[0], b_[1] - O[1], zb - 1.45),
                                     (b_[0] - O[0], b_[1] - O[1], zb + WALK_H), (a_[0] - O[0], a_[1] - O[1], za + WALK_H)],
                           [(0, 0), (L / 4, 0), (L / 4, 0.4), (0, 0.4)], (n[0], n[1], 0))
    # Pfade, Ampel
    pf, regeln, kt = pfade(plan, arme, zf)
    rollen = [a.rolle for a in arme]
    ampelplan = ampel.plane([dict(name=a.name, h=a.h) for a in arme], rollen, plan['phasen'])
    signale = []
    from shapely.ops import nearest_points
    standflaeche = erhoeht.buffer(-0.25)             # Mast mindestens 25 cm hinter der Bordkante
    for s in plan['ampeln']:
        p = np.array(s['pos'], float)
        rot_ = s['richtung'] % 360
        # Anschluss: Mastfuss muss auf Gehweg/Insel stehen; knapp daneben (bis 1,5 m) -> hinter die naechste Kante
        mp0 = p - ampel.MAST_VERSATZ * np.array(dvec(rot_))
        if not standflaeche.is_empty and not standflaeche.contains(Point(*mp0)):
            q = nearest_points(standflaeche, Point(*mp0))[0]
            if q.distance(Point(*mp0)) <= 1.5:
                p = p + (np.array([q.x, q.y]) - mp0)
        signal = ampel.SIGNAL if s['art'] == 'kfz' else rk.FG_SIGNAL
        signale.append(dict(art='signal', datei=signal, x=float(p[0]), y=float(p[1]), hoehe=ampel.SIGNAL_HOEHE, rot=rot_, gruppe=s['gruppe'],
                            z_boden=float(zf(*p)) + WALK_H))
        mp = p - ampel.MAST_VERSATZ * np.array(dvec(rot_))
        ausleger = s.get('mast') == 'ausleger'
        signale.append(dict(art='mast', datei=ampel.MAST if ausleger else rk.KLEINMAST, x=float(mp[0]), y=float(mp[1]),
                            hoehe=ampel.MAST_HOEHE if ausleger else 0.2, rot=(rot_ + 180) % 360 if ausleger else rot_,
                            gruppe=None, z_boden=float(zf(*mp)) + WALK_H))
        if ausleger:
            signale.append(dict(art='oben', datei=ampel.OBEN, eltern=len(signale) - 1, anhang=0, rot=ampel.OBEN_DREHUNG, gruppe=s['gruppe']))
    lose = [s for s in signale if s['art'] == 'mast' and not erhoeht.contains(Point(s['x'], s['y']))]
    F = dict(fahrbahn=fahrbahn, gehweg=nach_art['gehweg'], inseln=unary_union([nach_art['insel'], nach_art['gruen']]), offen=offen)
    return dict(mesh=m, O=O, pfade=pf, regeln=regeln, ketten=[dict(von=k['von'], nach=k['nach'], mv=k['mv']) for k in kt], plan=ampelplan,
                signale=signale, flaechen=F, zf=zf, trog=T, arme=arme, tunnel=dict(x=(tun.get('x0'), tun.get('x1')), y=tunnel_y, sohle=tun.get('sohle')),
                lose_signale=[(round(s_['x'], 1), round(s_['y'], 1), s_['art']) for s_ in lose])


def lesen(pfad):
    with open(pfad, encoding='utf-8') as f:
        return json.load(f)


def schreiben(pfad, plan):
    os.makedirs(os.path.dirname(pfad), exist_ok=True)
    with open(pfad, 'w', encoding='utf-8') as f:
        json.dump(plan, f, indent=1, ensure_ascii=False)
