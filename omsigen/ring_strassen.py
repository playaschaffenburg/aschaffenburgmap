"""Sonderbau Ring/Ludwigsallee, Etappe 2: die Strassen als OMSI-Splines, genau auf dem Entwurf (ring_entwurf).

- Ring: auf der Achse (Geraden + Boegen), zwei Spuren Gegenverkehr zwischen den Trogwaenden, Hoehe = Ring-Gradiente.
- Rampen: parallel zur Achse (konzentrische Boegen: k' = k / (1 + k t)), je eine Spur, Hoehe = Rampen-Gradiente,
  die ohne Knick in die Ring-Hoehe einlaeuft. Im Kreuzungsbereich unterbrochen (dort kommt in Etappe 3 das
  Kreuzungsobjekt hin).
- Ludwigsallee: West- (Schiesshausbruecke, ueber die Bahn) und Ostarm als Geraden entlang OSM bis an den
  Kreuzungsbereich, Hoehe aus dem DGM1 (ohne den Bahneinschnitt unter der Bruecke) als Gradiente.

Jede Strasse: Querschnitt (Baukasten-Format, der Editor erzeugt daraus die .sli) und Elemente fuer [spline_h]
(Start, Richtung im Uhrzeigersinn ab Nord, Laenge, Radius > 0 = Rechtskurve, Steigung Anfang/Ende in %, Hoehenaenderung),
hoechstens 20 m lang. Spurrichtungen nach OSM."""
import math
import numpy as np
from .sonderbau_entwurf import (achse_aus_json, gradiente_aus_json, versatz_aus_json, weich_max, gradiente_anpassen,
                                ohne_ausreisser)

MAX_ELEMENT = 20.0
KNOTEN_S = (117.0, 145.0)           # Kreuzungsbereich auf der Achse (Ludwigsallee mit Haltlinien und Furten)
KNOTEN_T = (-18.5, 17.0)            # quer: Ostrand ... Westrand (t > 0 = West)
LUDWIG_T = (-46.0, 49.0)            # Enden der Ludwigsallee-Arme (Ost, West)
BRUECKE_X = (1045.0, 1068.5)        # Bahnbruecke: DGM zeigt dort den Einschnitt


def teil(art, breite, richtung='vor', belag=None):
    t = dict(art=art, breite=breite, richtung=richtung)
    t['belag'] = belag or {'fahrspur': 'asphalt', 'busspur': 'asphalt', 'radfahrstreifen': 'rot', 'parkstreifen': 'asphalt',
                           'gehweg': 'platten'}[art]
    return t


def querschnitte(ring_breite):
    """Querschnitte (Baukasten-JSON). Achse = Mitte der Fahrspuren. Spline-Richtung Ring/Rampen: Nord -> Sued,
    Ludwigsallee: West -> Ost; 'vor' = mit dem Spline (rechts)"""
    schulter = max(0.3, (ring_breite - 7.0) / 2)
    return {
        'ring': dict(name='SB_Ring_Trog', teile=[teil('parkstreifen', schulter), teil('fahrspur', 3.5, 'zurueck'),
                                                  teil('fahrspur', 3.5), teil('parkstreifen', schulter)],
                     marken=['voll', 'voll', 'voll']),
        # Rampe West: Einbahn nach Sueden (mit dem Spline), Sperrflaeche zum Ring hin
        'rampe_w': dict(name='SB_Rampe_West', teile=[teil('parkstreifen', 1.5), teil('fahrspur', 3.5), teil('parkstreifen', 1.5)],
                        marken=['voll', 'voll']),
        # Rampe Ost: Einbahn nach Norden (gegen den Spline)
        'rampe_o': dict(name='SB_Rampe_Ost', teile=[teil('parkstreifen', 1.5), teil('fahrspur', 3.5, 'zurueck'), teil('parkstreifen', 1.5)],
                        marken=['voll', 'voll']),
        # Schiesshausbruecke (Westarm): Busspur nach Westen, zwei Spuren nach Osten, Radfahrstreifen
        'ludwig_w': dict(name='SB_Schiesshausbruecke', teile=[teil('gehweg', 2.5), teil('busspur', 3.5, 'zurueck'),
                                                              teil('fahrspur', 3.25), teil('fahrspur', 3.25),
                                                              teil('radfahrstreifen', 1.85), teil('gehweg', 2.5)],
                         marken=['auto', 'voll', 'strich', 'voll', 'auto']),
        # Ostarm: zwei Spuren nach Westen (links | rechts zur Kreuzung), eine nach Osten, Schutzstreifen
        'ludwig_o': dict(name='SB_Ludwigsallee_Ost', teile=[teil('gehweg', 2.5), teil('radfahrstreifen', 1.5),
                                                            teil('fahrspur', 3.25, 'zurueck'), teil('fahrspur', 3.25, 'zurueck'),
                                                            teil('fahrspur', 3.25), teil('radfahrstreifen', 1.5), teil('gehweg', 2.5)],
                         marken=['auto', 'strich', 'strich', 'voll', 'strich', 'auto']),
    }


# ------------------------------------------------------------------ Elemente
def omsi_richtung(th):
    """mathematische Richtung (0 = Ost, gegen den Uhrzeigersinn) -> OMSI (Grad, im Uhrzeigersinn ab Nord)"""
    return (90.0 - math.degrees(th)) % 360.0


def versetzt_elemente(achse, t, s0, s1):
    """Elemente der zur Achse parallelen Linie im Abstand t (rechts positiv) zwischen Achs-Stationen s0..s1:
    Liste (Achs-s Anfang, Achs-s Ende, Kruemmung der Parallele, Laenge der Parallele)"""
    out = []
    s = 0.0
    for l, k in achse.elemente:
        a, b = max(s, s0), min(s + l, s1)
        if b > a + 1e-6:
            f = 1.0 + k * t                     # Laengenfaktor der Parallele (Linksbogen, rechts versetzt: laenger)
            out.append((a, b, k / f, (b - a) * f))
        s += l
    if s1 > s + 1e-6:                            # Verlaengerung ueber das Achsende als Gerade
        out.append((max(s, s0), s1, 0.0, s1 - max(s, s0)))
    return out


def elemente(achse, t, s0, s1, z_von_s):
    """[spline_h]-Elemente (<= 20 m) der Parallele im Abstand t, Hoehe z(Achs-s) -> Liste dict"""
    out = []
    kum = 0.0
    for a, b, kk, laenge in versetzt_elemente(achse, t, s0, s1):
        n = max(1, math.ceil(laenge / MAX_ELEMENT - 1e-9))
        for i in range(n):
            sa, sb = a + (b - a) * i / n, a + (b - a) * (i + 1) / n
            x, y, th = achse.punkt(sa)
            px, py = x + t * math.sin(th), y - t * math.cos(th)
            l = laenge / n
            f = l / (sb - sa)                    # Meter Parallele je Meter Achse
            h = 0.05
            za, zb = float(z_von_s(sa)), float(z_von_s(sb))
            ga = (float(z_von_s(sa + h)) - float(z_von_s(sa - h))) / (2 * h) / f
            gb = (float(z_von_s(sb + h)) - float(z_von_s(sb - h))) / (2 * h) / f
            out.append(dict(x=px, y=py, z=za, richtung=omsi_richtung(th), laenge=l,
                            radius=0.0 if abs(kk) < 1e-9 else -1.0 / kk, stg_a=ga * 100, stg_e=gb * 100, dh=zb - za, kum=kum))
            kum += l
    return out


def gerade_elemente(p0, p1, z_von_u):
    """Gerade von p0 nach p1 (Hoehe z(u), u in Metern ab p0) in Elementen <= 20 m"""
    d = np.asarray(p1, float) - np.asarray(p0, float)
    L = float(np.hypot(*d))
    th = math.atan2(d[1], d[0])
    n = max(1, math.ceil(L / MAX_ELEMENT - 1e-9))
    out = []
    for i in range(n):
        ua, ub = L * i / n, L * (i + 1) / n
        h = 0.05
        za, zb = float(z_von_u(ua)), float(z_von_u(ub))
        ga = (float(z_von_u(ua + h)) - float(z_von_u(ua - h))) / (2 * h)
        gb = (float(z_von_u(ub + h)) - float(z_von_u(ub - h))) / (2 * h)
        out.append(dict(x=float(p0[0] + d[0] * ua / L), y=float(p0[1] + d[1] * ua / L), z=za, richtung=omsi_richtung(th),
                        laenge=L / n, radius=0.0, stg_a=ga * 100, stg_e=gb * 100, dh=zb - za, kum=ua))
    return out


# ------------------------------------------------------------------ Planung
def planen(entwurf, gel, ways):
    """-> Liste dict(name, querschnitt, elemente, art) (art: ring/rampe/ludwigsallee)"""
    from . import sonderbau_ring as sr
    achse = achse_aus_json(entwurf['achse'])
    K = {k: versatz_aus_json(v) for k, v in entwurf['kanten'].items()}
    G = {k: gradiente_aus_json(v) for k, v in entwurf['hoehen'].items()}
    L = achse.laenge
    S = np.arange(0, L, 1.0)
    # Ringbreite zwischen den Waenden (Median)
    breite = float(np.nanmedian(K['ring_w'].t(S) - K['ring_o'].t(S)))
    Q = querschnitte(breite)
    zr = lambda s: G['ring'].z(s)[0] if np.ndim(s) == 0 else G['ring'].z(s)

    def rampe_z(seite):
        g = G[f'rampe_{seite}']
        def z(s):
            s_ = np.atleast_1d(s)
            roh = np.where((s_ >= g.k[0] - 30) & (s_ <= g.k[-1] + 30), g.z(s_), -99.0)
            v = weich_max(roh, G['ring'].z(s_))
            return v[0] if np.ndim(s) == 0 else v
        return z

    out = []
    out.append(dict(name='Ring', art='ring', querschnitt=Q['ring'], elemente=elemente(achse, 0.0, 0.0, L, zr)))
    # Rampen: Spurmitte = Mitte zwischen Trennstreifen und Aussenkante (Median ueber die Wandstrecke)
    for seite, vz in (('w', 1), ('o', -1)):
        innen = K[f'ring_{seite}'].t(S) + vz * sr.TRENNSTREIFEN
        aussen = K[f'rampe_aussen_{seite}'].t(S)
        mitte = float(np.nanmedian((innen + aussen) / 2))
        z = rampe_z(seite)
        for i, (a, b) in enumerate(((0.0, KNOTEN_S[0]), (KNOTEN_S[1], L))):
            out.append(dict(name=f'Rampe_{"West" if seite == "w" else "Ost"}_{"Nord" if i == 0 else "Sued"}', art='rampe',
                            versatz=mitte, querschnitt=Q[f'rampe_{seite}'], elemente=elemente(achse, mitte, a, b, z)))
    # Ludwigsallee: Gerade durch die OSM-Punkte der Arme
    s_k = (KNOTEN_S[0] + KNOTEN_S[1]) / 2
    for arm, ids, (t_a, t_b) in (('w', ['10058252', '339300221', '575971195'], (LUDWIG_T[1], KNOTEN_T[1])),
                                 ('o', ['228086398', '49120474'], (KNOTEN_T[0], LUDWIG_T[0]))):
        P = np.concatenate([np.array(ways[i]) for i in ids])
        st = achse.projektion(P)
        # Ausgleichsgerade s(t) durch die OSM-Punkte (Achse quer zum Ring)
        m, c = np.polyfit(st[:, 1], st[:, 0], 1)
        pa = achse.versetzt([m * t_a + c], [t_a])[0]
        pb = achse.versetzt([m * t_b + c], [t_b])[0]
        # Hoehe: DGM auf der Geraden, ohne Bahneinschnitt, als Gradiente
        d = pb - pa
        Lg = float(np.hypot(*d))
        u = np.arange(0, Lg, 1.0)
        xy = pa[None, :] + np.outer(u / Lg, d)
        h = gel.h(xy[:, 0], xy[:, 1])
        h = np.where((xy[:, 0] > BRUECKE_X[0] - 1) & (xy[:, 0] < BRUECKE_X[1] + 1), np.nan, h)
        g, _ = ohne_ausreisser(u, h, lambda a, b: gradiente_anpassen(a, b, toleranz=0.08, max_stuecke=3), lambda gg, x: gg.z(x), 0.4)
        zu = lambda uu, g=g: g.z(uu)[0] if np.ndim(uu) == 0 else g.z(uu)
        out.append(dict(name='Schiesshausbruecke' if arm == 'w' else 'Ludwigsallee_Ost', art='ludwigsallee',
                        querschnitt=Q[f'ludwig_{arm}'], elemente=gerade_elemente(pa, pb, zu)))
    return out


def text(strassen):
    z = []
    for st in strassen:
        el = st['elemente']
        if not el:
            continue
        L = sum(e['laenge'] for e in el)
        z.append(f"{st['name']:24s} {len(el):3d} Elemente  {L:6.1f} m  Hoehe {el[0]['z']:.2f} -> {el[-1]['z'] + el[-1]['dh']:.2f} m"
                 + (f"  Versatz {st['versatz']:+.2f} m" if 'versatz' in st else ''))
    return '\n'.join(z)
