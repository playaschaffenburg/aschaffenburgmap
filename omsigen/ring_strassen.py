"""Sonderbau Ring/Ludwigsallee, Etappe 2: die Strassen als OMSI-Splines, genau auf dem Entwurf (ring_entwurf).

- Ring: auf der Achse (Geraden + Boegen), zwei Spuren Gegenverkehr zwischen den Trogwaenden, Hoehe = Ring-Gradiente.
- Rampen: parallel zur Achse (konzentrische Boegen: k' = k / (1 + k t)), je eine Spur, Hoehe = Rampen-Gradiente,
  die ohne Knick in die Ring-Hoehe einlaeuft. Sie enden am Kreuzungsobjekt (ring_kreuzung), das die letzten Meter mit
  Pfeilen und Haltlinien selbst traegt.
- Ludwigsallee: West- (Schiesshausbruecke) und Ostarm als Geraden in OSM-Richtung durch die im Luftbild gemessene
  Fahrbahnmitte, Hoehe aus dem DGM1 (ohne den Bahneinschnitt unter der Bruecke) als Gradiente.

Querschnitte und Lagen nach dem Luftbild (DOP40, sonderbau/ring/PLAN.md). Jede Strasse: Querschnitt (Baukasten-Format,
der Editor erzeugt daraus die .sli) und Elemente fuer [spline_h] (Start, Richtung im Uhrzeigersinn ab Nord, Laenge,
Radius > 0 = Rechtskurve, Steigung Anfang/Ende in %, Hoehenaenderung), hoechstens 20 m lang."""
import math
import numpy as np
from .sonderbau_entwurf import (achse_aus_json, gradiente_aus_json, versatz_aus_json, weich_max, gradiente_anpassen,
                                ohne_ausreisser)

MAX_ELEMENT = 20.0
BRUECKE_X = (1045.0, 1068.5)        # Bahnbruecke: DGM zeigt dort den Einschnitt

# Rampen nach Luftbild: Abstand der Fahrbahnkante zur Ringwand (Wand + Bruestung), Spurbreite, Rest bis zur
# Aussenwand als Sperrflaeche (schraffiert) oder Randstreifen; Achs-Station, an der das Kreuzungsobjekt beginnt
RAMPEN = {
    'Rampe_West_Nord': dict(seite='w', s=(0.0, 108.0), trenn=0.45, spur=4.95, rest='sperr', richtung='vor'),
    'Rampe_West_Sued': dict(seite='w', s=(152.5, None), trenn=0.45, spur=5.0, rest='parkstreifen', richtung='vor'),
    'Rampe_Ost_Nord': dict(seite='o', s=(0.0, 114.0), trenn=0.8, spur=5.0, rest='parkstreifen', richtung='zurueck'),
    'Rampe_Ost_Sued': dict(seite='o', s=(160.0, None), trenn=2.3, spur=3.6, rest='sperr', richtung='zurueck'),
}
# Ludwigsallee nach Luftbild: Punkt auf der Fahrbahnmitte (Mitte der Fahrspuren) und Ende am Kreuzungsobjekt (x)
LUDWIG = {
    'Schiesshausbruecke': dict(ways=['10058252', '339300221', '575971195'], mitte=(1055.0, -267.0), x=(1035.0, 1050.0)),
    'Ludwigsallee_Ost': dict(ways=['228086398', '49120474'], mitte=(1115.0, -263.85), x=(1120.0, 1132.0)),
}


def teil(art, breite, richtung='vor', belag=None):
    t = dict(art=art, breite=round(breite, 3), richtung=richtung)
    t['belag'] = belag or {'fahrspur': 'asphalt', 'busspur': 'asphalt', 'radfahrstreifen': 'rot', 'parkstreifen': 'asphalt',
                           'gehweg': 'asphalt'}[art]          # Gehwege am Ring/an der Ludwigsallee: Asphalt (OSM, Fotos)
    return t


def querschnitte(ring_breite, rest):
    """Querschnitte (Baukasten-JSON). Achse = Mitte der Fahrspuren. Spline-Richtung Ring/Rampen: Nord -> Sued (links =
    Osten), Ludwigsallee: West -> Ost (links = Norden); 'vor' = mit dem Spline. `rest`: Breite neben der Rampenspur"""
    schulter = max(0.3, (ring_breite - 7.0) / 2)

    def rand(name):
        r = RAMPEN[name]
        return teil('parkstreifen', rest[name], belag='sperr' if r['rest'] == 'sperr' else 'asphalt')
    return {
        'Ring': dict(name='SB_Ring_Trog', teile=[teil('parkstreifen', schulter), teil('fahrspur', 3.5, 'zurueck'),
                                                  teil('fahrspur', 3.5), teil('parkstreifen', schulter)],
                     marken=['voll', 'voll', 'voll']),
        # West: Spur am Ring, Sperrflaeche/Randstreifen aussen (rechts); Ost: aussen links, Spur gegen den Spline
        'Rampe_West_Nord': dict(name='SB_Rampe_West_Nord', teile=[teil('fahrspur', RAMPEN['Rampe_West_Nord']['spur']), rand('Rampe_West_Nord')], marken=['voll']),
        'Rampe_West_Sued': dict(name='SB_Rampe_West_Sued', teile=[teil('fahrspur', RAMPEN['Rampe_West_Sued']['spur']), rand('Rampe_West_Sued')], marken=['voll']),
        'Rampe_Ost_Nord': dict(name='SB_Rampe_Ost_Nord', teile=[rand('Rampe_Ost_Nord'), teil('fahrspur', RAMPEN['Rampe_Ost_Nord']['spur'], 'zurueck')], marken=['voll']),
        'Rampe_Ost_Sued': dict(name='SB_Rampe_Ost_Sued', teile=[rand('Rampe_Ost_Sued'), teil('fahrspur', RAMPEN['Rampe_Ost_Sued']['spur'], 'zurueck')], marken=['voll']),
        # Schiesshausbruecke: Busspur (Beton, mit Radverkehr) nach Westen, Sperrstreifen, zwei Spuren nach Osten
        'Schiesshausbruecke': dict(name='SB_Schiesshausbruecke', teile=[
            teil('gehweg', 2.6), teil('busspur', 4.5, 'zurueck', belag='beton'), teil('parkstreifen', 0.8, belag='sperr'),
            teil('fahrspur', 3.35), teil('fahrspur', 3.35), teil('gehweg', 2.7)],
            marken=['auto', 'voll', 'voll', 'strich', 'auto']),
        # Ostarm: Busspur und zwei Spuren nach Westen, Mittelstreifen (Sperrflaeche), eine Spur nach Osten, Radstreifen
        'Ludwigsallee_Ost': dict(name='SB_Ludwigsallee_Ost', teile=[
            teil('gehweg', 2.5), teil('busspur', 2.9, 'zurueck'), teil('fahrspur', 3.6, 'zurueck'), teil('fahrspur', 3.6, 'zurueck'),
            teil('parkstreifen', 2.3, belag='sperr'), teil('fahrspur', 3.5), teil('radfahrstreifen', 1.6), teil('gehweg', 2.5)],
            marken=['auto', 'voll', 'strich', 'voll', 'voll', 'voll', 'auto']),
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
    """-> (Strassen: Liste dict(name, art, querschnitt, elemente[, versatz]), Arme der Kreuzung)"""
    achse = achse_aus_json(entwurf['achse'])
    K = {k: versatz_aus_json(v) for k, v in entwurf['kanten'].items()}
    G = {k: gradiente_aus_json(v) for k, v in entwurf['hoehen'].items()}
    L = achse.laenge
    S = np.arange(0, L, 1.0)
    breite = float(np.nanmedian(K['ring_w'].t(S) - K['ring_o'].t(S)))
    zr = lambda s: G['ring'].z(s)[0] if np.ndim(s) == 0 else G['ring'].z(s)

    def rampe_z(seite):
        g = G[f'rampe_{seite}']
        def z(s):
            s_ = np.atleast_1d(s)
            roh = np.where((s_ >= g.k[0] - 30) & (s_ <= g.k[-1] + 30), g.z(s_), -99.0)
            v = weich_max(roh, G['ring'].z(s_))
            return v[0] if np.ndim(s) == 0 else v
        return z

    # Rampen: Spurmitte und Restbreite aus Ringwand, Trennstreifen und Aussenwand (Median ueber die Strecke)
    lage, rest = {}, {}
    for name, r in RAMPEN.items():
        vz = 1 if r['seite'] == 'w' else -1
        a, b = r['s'][0], r['s'][1] if r['s'][1] is not None else L
        Ss = S[(S >= a) & (S <= b)]
        wand = float(np.nanmedian(K[f'ring_{r["seite"]}'].t(Ss)))
        aussen = float(np.nanmedian(K[f'rampe_aussen_{r["seite"]}'].t(Ss)))
        innen = wand + vz * r['trenn']
        lage[name] = innen + vz * r['spur'] / 2
        rest[name] = max(0.5, abs(aussen - innen) - r['spur'])
    Q = querschnitte(breite, rest)
    out = [dict(name='Ring', art='ring', querschnitt=Q['Ring'], elemente=elemente(achse, 0.0, 0.0, L, zr))]
    for name, r in RAMPEN.items():
        a, b = r['s'][0], r['s'][1] if r['s'][1] is not None else L
        out.append(dict(name=name, art='rampe', versatz=lage[name], querschnitt=Q[name],
                        elemente=elemente(achse, lage[name], a, b, rampe_z(r['seite']))))
    for name, l in LUDWIG.items():
        P = np.concatenate([np.array(ways[i]) for i in l['ways']])
        # Richtung aus OSM (Ausgleichsgerade), Lage durch die gemessene Mitte
        d = np.polyfit(P[:, 0], P[:, 1], 1)[0]
        u = np.array([1.0, d]) / math.hypot(1.0, d)
        m = np.array(l['mitte'])
        pa = m + u * (l['x'][0] - m[0]) / u[0]
        pb = m + u * (l['x'][1] - m[0]) / u[0]
        # Hoehe: DGM ueber die ganze Ludwigsallee (auch durch die Kreuzung), ohne Bahneinschnitt, als Gradiente
        q0 = m + u * (1030.0 - m[0]) / u[0]
        q1 = m + u * (1135.0 - m[0]) / u[0]
        Lq = float(np.hypot(*(q1 - q0)))
        uu = np.arange(0, Lq, 1.0)
        xy = q0[None, :] + np.outer(uu / Lq, q1 - q0)
        h = gel.h(xy[:, 0], xy[:, 1])
        h = np.where((xy[:, 0] > BRUECKE_X[0] - 1) & (xy[:, 0] < BRUECKE_X[1] + 1), np.nan, h)
        h = np.where((xy[:, 0] > 1068) & (xy[:, 0] < 1104), np.nan, h)      # Kreuzung: Decke/Rampen im DGM
        g, _ = ohne_ausreisser(uu, h, lambda a, b: gradiente_anpassen(a, b, toleranz=0.08, max_stuecke=3), lambda gg, x: gg.z(x), 0.4)
        off = float(np.hypot(*(pa - q0)))
        zu = lambda v, g=g, off=off: g.z(np.asarray(v) + off)[0] if np.ndim(v) == 0 else g.z(np.asarray(v) + off)
        out.append(dict(name=name, art='ludwigsallee', querschnitt=Q[name], elemente=gerade_elemente(pa, pb, zu)))
    return out, kreuzungsarme(out)


# Arme der Kreuzung (Reihenfolge = Index in VERBINDUNGEN/PHASEN) und Vorfahrt-Rolle
ARM_REIHENFOLGE = [('Schiesshausbruecke', 'haupt'), ('Rampe_West_Nord', 'neben'), ('Rampe_Ost_Nord', 'neben'),
                   ('Ludwigsallee_Ost', 'haupt'), ('Rampe_Ost_Sued', 'neben'), ('Rampe_West_Sued', 'neben')]
# Abbiegespuren nach OSM turn:lanes und Luftbild; Spuren je Arm von rechts (in Fahrtrichtung) gezaehlt:
# 0 Schiesshausbruecke (rein: 0 geradeaus/rechts, 1 links; raus: 0 Busspur), 1 Rampe West Nord (rein: 0),
# 2 Rampe Ost Nord (raus: 0, Wuerzburg), 3 Ludwigsallee Ost (rein: 0 Busspur, 1 rechts, 2 links; raus: 0),
# 4 Rampe Ost Sued (rein: 0), 5 Rampe West Sued (raus: 0, Frankfurt)
VERBINDUNGEN = [
    [0, 1, 2, 0],     # von Westen links (am Ostknoten) -> Wuerzburg
    [0, 0, 3, 0],     # von Westen geradeaus
    [0, 0, 5, 0],     # von Westen rechts -> Frankfurt
    [3, 0, 0, 0],     # Busspur von Osten geradeaus auf die Busspur der Schiesshausbruecke
    [3, 1, 2, 0],     # von Osten rechts -> Wuerzburg
    [3, 2, 5, 0],     # von Osten links (am Westknoten) -> Frankfurt
    [1, 0, 3, 0],     # Rampe West (vom Ring aus Norden) links nach Osten
    [1, 0, 2, 0],     # ... und am Ostknoten links zurueck nach Wuerzburg
    [4, 0, 3, 0],     # Rampe Ost (vom Ring aus Sueden) rechts nach Osten
]
# Pfeile je ankommender Spur (Arm, Spur von rechts): Liste von 'links', 'gerade', 'rechts'
PFEILE = {(0, 0): ['gerade', 'rechts'], (0, 1): ['links'], (1, 0): ['links'], (3, 1): ['rechts'], (3, 2): ['links'],
          (4, 0): ['rechts']}
# Ampelphasen je Arm: 1 Ludwigsallee (beide Richtungen), 2 Rampe West, 3 Rampe Ost - die beiden Rampen fahren in
# denselben Ostarm und bekommen getrennt Gruen; Arme ohne Zufahrt: 0
PHASEN = [0, 1, 0, 0, 2, 0]


def _ende(e):
    """Endpunkt (x, y, Richtung OMSI) eines Elements nach der OMSI-Bogenregel"""
    h = math.radians(e['richtung'])
    if e['radius'] == 0:
        return e['x'] + e['laenge'] * math.sin(h), e['y'] + e['laenge'] * math.cos(h), e['richtung']
    R = e['radius']
    cx, cy = e['x'] + R * math.cos(h), e['y'] - R * math.sin(h)
    h2 = h + e['laenge'] / R
    return cx - R * math.cos(h2), cy + R * math.sin(h2), math.degrees(h2) % 360


def kreuzungsarme(strassen):
    """Arme der Kreuzung: Strassenende am Kreuzungsobjekt -> dict(strasse, pos, z, h (von der Kreuzung weg), away
    (Splinerichtung zeigt von der Kreuzung weg), rolle, g (Steigung von der Kreuzung weg), querschnitt)"""
    nach_name = {st['name']: st for st in strassen}
    out = []
    for name, rolle in ARM_REIHENFOLGE:
        st = nach_name[name]
        el = st['elemente']
        endet = name in ('Rampe_West_Nord', 'Rampe_Ost_Nord', 'Schiesshausbruecke')
        if endet:
            x, y, h = _ende(el[-1])
            out.append(dict(strasse=name, pos=[x, y], z=el[-1]['z'] + el[-1]['dh'], h=(h + 180) % 360, away=False, rolle=rolle,
                            g=-el[-1]['stg_e'] / 100, querschnitt=st['querschnitt']))
        else:
            e = el[0]
            out.append(dict(strasse=name, pos=[e['x'], e['y']], z=e['z'], h=e['richtung'] % 360, away=True, rolle=rolle,
                            g=e['stg_a'] / 100, querschnitt=st['querschnitt']))
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
