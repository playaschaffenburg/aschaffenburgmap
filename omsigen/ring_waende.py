"""Sonderbau Ring: die Waende als Splines (wie Staedtedreieck, Ahlheim, Gladbeck - docs/omsi-format.md, „Wie
Fremdkarten Troege, Mauern und grosse Kreuzungen bauen“).

Je Wand-Bauteil des Bauplans (`waende`: Art trenn/aussen, Seite, Bereich, Material, Laermschutz, Versatz, Hoehe,
zweistufig) entsteht eine Kette aus [spline_h] entlang der Wandlinie. Die Spline-Hoehe ist die Mauerkrone; das Profil
reicht TIEFE m nach unten und verschwindet unter der Fahrbahn bzw. dem Gelaende - so wie `Mauer_Unterfuehrung.sli`
in Staedtedreieck. Weil die Kette der Wandlinie und der Kronenhoehe folgt, gibt es keine Fugen und Knicke zwischen
Stuecken mehr (frueher: Wand-Objekte zu je 40 m).

Profilkoordinaten: x rechts der Laufrichtung (Laufrichtung = aufsteigende Achs-Station, Nord -> Sued), y hoch ab der
Krone. Die Fahrbahnseite liegt bei x * r > 0 mit r = -1 (West, Strasse oestlich) bzw. +1 (Ost). Sichtbar ist eine
Profilstrecke auf ihrer linken Seite (Normale (-dy, dx)), wie in bauwerke.bruecke_sli.
"""
import math
import os
import shutil
import numpy as np

from .ring_strassen import omsi_richtung, _ende

KNOTEN = 8.0              # m hoechstens zwischen zwei Kettenpunkten
TIEFE_RUND = 1.0          # Profiltiefe auf ganze Meter (wenige .sli-Dateien)
DICKE_TRENN = 0.5
KAPPE = 0.3               # Betonband unter der Krone
BRUESTUNG = 0.25          # niedrige Betonkante auf der Trennwand
GELAENDER = 1.0
STUFE = 2.0               # zweistufige Klinkerwand: Pflanzstreifen so weit unter der Krone
STUFE_BREIT = 1.5
LSW_H = 2.5
MIN_STUECK = 20.0         # m: kuerzere Wechsel einfach/zweistufig werden dem Nachbarn zugeschlagen

# Texturen im Spline-Ordner (Splines\Aschaffenburg\texture), Quelle: Objektordner des Sonderbaus bzw. Marcel
TEX = {'klinker': 'AB_SB_klinker.bmp', 'beton': 'AB_SB_beton.bmp', 'gabionen': 'AB_SB_gabionen.bmp',
       'pflanzen': 'AB_SB_bodendecker.bmp', 'stab': 'AB_SB_stabgelaender.tga', 'gruen': 'AB_SB_lsw_gruen.bmp',
       'glas': 'AB_SB_glas.tga'}
QUELLE = {'klinker': 'AB_klinker.bmp', 'beton': 'betonwand1.bmp', 'gabionen': 'AB_gabionen.bmp',
          'pflanzen': 'AB_bodendecker.bmp', 'stab': 'AB_stabgelaender.tga', 'gruen': 'AB_lsw_gruen.bmp', 'glas': 'AB_glas.tga'}
ALPHA = {'stab', 'glas'}
GROESSE = {'klinker': 2.0, 'beton': 4.0, 'gabionen': 2.0, 'pflanzen': 2.0, 'stab': 1.0, 'gruen': 2.5, 'glas': 2.5}


# ------------------------------------------------------------------ Profile
class Profil:
    def __init__(self):
        self.teile = []           # (Textur, [(x, y, u)])

    def strecke(self, tex, a, b):
        L = math.hypot(b[0] - a[0], b[1] - a[1])
        self.teile.append((tex, [(a[0], a[1], 0.0), (b[0], b[1], L / GROESSE[tex])]))

    def flaeche(self, tex, x, y0, y1, sicht):
        """senkrechte Flaeche bei x von y0 bis y1, sichtbar zur Seite sicht (-1 links, +1 rechts)"""
        if sicht < 0:
            self.strecke(tex, (x, y0), (x, y1))
        else:
            self.strecke(tex, (x, y1), (x, y0))

    def oben(self, tex, x0, x1, y):
        self.strecke(tex, (min(x0, x1), y), (max(x0, x1), y))

    def beidseitig(self, tex, x, y0, y1):
        self.flaeche(tex, x, y0, y1, -1)
        self.flaeche(tex, x, y0, y1, +1)

    def text(self, kopf):
        texturen = sorted({t for t, _ in self.teile}, key=list(TEX).index)
        L = [kopf, '']
        for t in texturen:
            L += ['[texture]', TEX[t], '']
            if t in ALPHA:
                L += ['[matl_alpha]', '1', '']
        for t, pts in self.teile:
            L += ['[profile]', str(texturen.index(t)), '']
            for x, y, u in pts:
                L += ['[profilepnt]', f'{x:.3f}', f'{y:.3f}', f'{u:.3f}', f'{1.0 / GROESSE[t]:.3f}', '']
        return '\r\n'.join(L) + '\r\n'


def profil(art, material, variante, laerm, r, tiefe):
    """Profil einer Wand. r: Seite der Fahrbahn in Profil-x (-1/+1); Krone bei y = 0, unten bis -tiefe.
    variante: trenn 'bruestung'/'flach', aussen 'einfach'/'stufig'"""
    p = Profil()
    m = material if material in ('klinker', 'beton', 'gabionen') else 'klinker'
    u = -tiefe
    if art == 'portal':
        # Stirn des Trogendes ueber der Unterfuehrung: Beton bis unter die Decke, Bruestung und Gelaender wie die
        # Trennwand (laufen ohne Absatz aus ihr heraus)
        h = DICKE_TRENN / 2
        xr, xa = r * h, -r * h
        p.flaeche('beton', xr, u, BRUESTUNG, r)
        p.oben('beton', xr, xa, BRUESTUNG)
        p.flaeche('beton', xa, -0.3, BRUESTUNG, -r)
        p.beidseitig('stab', 0.0, BRUESTUNG, BRUESTUNG + GELAENDER)
        return p
    if art == 'trenn':
        h = DICKE_TRENN / 2
        xr, xa = r * h, -r * h                         # Ring-Seite, Rampen-Seite
        p.flaeche(m, xr, u, -KAPPE - 0.1, r)
        p.flaeche('beton', xr, -KAPPE - 0.1, 0.0, r)
        if variante == 'bruestung':
            p.flaeche('beton', xr, 0.0, BRUESTUNG, r)
            p.oben('beton', xr, xa, BRUESTUNG)
            p.flaeche('beton', xa, -0.3, BRUESTUNG, -r)
            p.beidseitig('stab', 0.0, BRUESTUNG, BRUESTUNG + GELAENDER)
        else:
            p.oben('beton', xr, xa, 0.0)
            p.flaeche('beton', xa, -0.3, 0.0, -r)
        lsw_x = None
    else:
        if m == 'gabionen':
            d = 1.0
            p.flaeche('gabionen', 0.0, u, 0.0, r)
            p.oben('gabionen', 0.0, -r * d, 0.0)
            p.flaeche('gabionen', -r * d, u, 0.0, -r)
            lsw_x = -r * 0.5
        elif variante == 'stufig':
            d = 0.35
            p.flaeche(m, 0.0, u, -STUFE - KAPPE, r)
            p.flaeche('beton', 0.0, -STUFE - KAPPE, -STUFE, r)
            p.oben('pflanzen', 0.0, -r * STUFE_BREIT, -STUFE)
            x1 = -r * STUFE_BREIT
            p.flaeche(m, x1, -STUFE, -KAPPE, r)
            p.flaeche('beton', x1, -KAPPE, 0.0, r)
            p.oben('beton', x1, x1 - r * d, 0.0)
            p.flaeche('beton', x1 - r * d, u, 0.0, -r)
            lsw_x = x1 - r * 0.2
        else:
            d = 0.35
            p.flaeche(m, 0.0, u, -KAPPE, r)
            p.flaeche('beton', 0.0, -KAPPE, 0.0, r)
            p.oben('beton', 0.0, -r * d, 0.0)
            p.flaeche('beton', -r * d, u, 0.0, -r)
            lsw_x = -r * 0.2
    if lsw_x is not None and laerm in ('gruen', 'glas'):
        p.beidseitig(laerm, lsw_x, 0.0, LSW_H)
    return p


def sli_name(art, material, variante, laerm, seite, tiefe):
    return f"AB_SB_Wand_{art}_{material}_{variante}_{laerm}_{seite}_{int(tiefe)}m"


# ------------------------------------------------------------------ Kette durch Punkte
def kette(P, Z, knoten=KNOTEN):
    """[spline_h]-Elemente (Bogenkette, tangentenstetig) durch die Punkte P (n x 2, dicht, in Laufrichtung) mit
    Kronenhoehe Z -> Liste dict wie ring_strassen.elemente. Knoten hoechstens `knoten` m auseinander; jeder Bogen
    beginnt mit der Endrichtung des vorigen und trifft den naechsten Knoten genau."""
    P = np.asarray(P, float)
    Z = np.asarray(Z, float)
    if len(P) < 2:
        return []
    seg = np.hypot(*np.diff(P, axis=0).T)
    s = np.concatenate([[0.0], np.cumsum(seg)])
    n = max(1, math.ceil(s[-1] / knoten - 1e-9))
    ziel = np.linspace(0.0, s[-1], n + 1)
    idx = sorted({int(np.argmin(np.abs(s - z))) for z in ziel} | {0, len(P) - 1})
    K = P[idx]
    ZK = Z[idx]
    SK = s[idx]
    # Steigung je Knoten aus den Nachbarn (Kronenlinie)
    G = np.gradient(ZK, SK) if len(SK) > 1 else np.zeros(1)
    # Anfangsrichtung: Tangente der Punkte am Anfang
    j = min(len(P) - 1, max(1, int(np.searchsorted(s, min(2.0, s[-1])))))
    th = math.atan2(P[j, 1] - P[0, 1], P[j, 0] - P[0, 0])
    out = []
    kum = 0.0
    for i in range(len(K) - 1):
        a, b = K[i], K[i + 1]
        c = b - a
        sehne = float(np.hypot(*c))
        if sehne < 1e-6:
            continue
        delta = math.atan2(c[1], c[0]) - th
        delta = (delta + math.pi) % (2 * math.pi) - math.pi
        if abs(delta) < 1e-6:
            L, kk = sehne, 0.0
        else:
            L = sehne * delta / math.sin(delta)
            kk = 2 * delta / L
        out.append(dict(x=float(a[0]), y=float(a[1]), z=float(ZK[i]), richtung=omsi_richtung(th), laenge=float(L),
                        radius=0.0 if kk == 0.0 else -1.0 / kk, stg_a=float(G[i]) * 100 * sehne / L,
                        stg_e=float(G[i + 1]) * 100 * sehne / L, dh=float(ZK[i + 1] - ZK[i]), kum=kum))
        kum += L
        th += 2 * delta
    return out


# ------------------------------------------------------------------ Waende aus den Stationen
def laeufe(roh, art, seite, linie, krone, unten, variante):
    """zusammenhaengende Stuecke einer Wandart: je Station Lage t (linie), Krone, Unterkante, Variante; ein neues Stueck
    beginnt bei anderem Bauteil oder anderer Variante (am selben Punkt, ohne Luecke) -> Liste (Bauteil, Variante,
    [(e, t, krone, unten)])"""
    from .sonderbau_ring import wand
    out, akt = [], None
    for e in roh:
        bt = wand(e, art, seite)
        t = linie(e, seite)
        if bt is None or t is None:
            akt = None
            continue
        k = krone(e, seite, bt)
        pkt = (e, t, k, unten(e, seite))
        v = variante(e, seite, bt)
        if akt is not None and (akt[0] is bt and akt[1] == v):
            akt[2].append(pkt)
            continue
        if akt is not None and akt[1] == 'flach':
            # unter der Decke -> Bruestung: der Wechselpunkt (Portal-Station) gehoert zu beiden, die Bruestung beginnt dort
            akt[2].append(pkt)
            neu = (bt, v, [pkt])
        else:
            neu = (bt, v, [akt[2][-1], pkt] if akt is not None else [pkt])
        out.append(neu)
        akt = neu
    return [l for l in out if len(l[2]) >= 2]


def glatt(S, an, min_laenge=MIN_STUECK):
    """Ja/Nein je Station ohne Stuecke kuerzer als min_laenge (die kuerzesten zuerst dem Nachbarn zuschlagen)"""
    an = list(an)
    while True:
        stuecke, i = [], 0
        while i < len(an):
            j = i
            while j + 1 < len(an) and an[j + 1] == an[i]:
                j += 1
            stuecke.append((i, j))
            i = j + 1
        kurz = [(S[j] - S[i], i, j) for i, j in stuecke if S[j] - S[i] < min_laenge and len(stuecke) > 1]
        if not kurz:
            return an
        _, i, j = min(kurz)
        for k in range(i, j + 1):
            an[k] = not an[k]


def splines(roh, platte, omsi):
    """alle Waende als Spline-Ketten -> (Liste dict(name, art='wand', sli, elemente), {sli-Datei: Text})"""
    from .sonderbau_ring import fuge, aussen, S_PLATTE, welt, wand
    dateien = {}
    out = []

    def krone_trenn(e, seite, bt):
        z = e[seite]['z'] + float(bt.get('hoehe', 0.0))
        if platte is not None and S_PLATTE[0] <= e['s'] <= S_PLATTE[1]:
            q = welt(e, fuge(e, seite), 0)
            zp = float(platte(q[0], q[1]))
            z = min(z, zp - 0.1) if e['unter_decke'] else zp + 0.05
        return z + 0.05

    def var_trenn(e, seite, bt):
        return 'flach' if e['unter_decke'] else 'bruestung'

    def krone_aussen(e, seite, bt):
        r = e[seite]
        plus = float(bt.get('hoehe', 0.0))
        if bt.get('material') == 'gabionen':
            return max(r['g'] + 0.3, r['z'] + 1.0) + plus
        return max(r['g'], r['z']) + 0.3 + plus

    def roh_stufig(e, seite):
        r = e[seite]
        bt = wand(e, 'aussen', seite) or {}
        return (bt.get('material', 'klinker') == 'klinker' and bt.get('stufig', True) and r is not None
                and r['g'] - r['z'] > 3.5)

    stufig = {sd: glatt([e['s'] for e in roh], [roh_stufig(e, sd) for e in roh]) for sd in ('w', 'o')}
    idx = {id(e): i for i, e in enumerate(roh)}

    def var_aussen(e, seite, bt):
        return 'stufig' if stufig[seite][idx[id(e)]] else 'einfach'

    arten = [('trenn', fuge, krone_trenn, lambda e, sd: e['zr'] - 0.3, var_trenn),
             ('aussen', aussen, krone_aussen, lambda e, sd: min(e[sd]['g'], e[sd]['z']) - 0.5, var_aussen)]
    for seite in ('w', 'o'):
        r = -1 if seite == 'w' else 1
        for art, linie, krone, unten, variante in arten:
            for nr, (bt, v, pkt) in enumerate(laeufe(roh, art, seite, linie, krone, unten, variante)):
                tiefe = math.ceil(max(k - u for _, _, k, u in pkt) / TIEFE_RUND + 0.5) * TIEFE_RUND
                material = bt.get('material') or ('klinker' if art == 'trenn' else ('gabionen' if seite == 'w' else 'klinker'))
                laerm = bt.get('laerm', 'keine') if art == 'aussen' else 'keine'
                name = sli_name(art, material, v, laerm, seite, tiefe)
                if name not in dateien:
                    dateien[name] = profil(art, material, v, laerm, r, tiefe).text(
                        f'File created with omsigen (Sonderbau Ring). Wand {art} {material} {v} Laermschutz {laerm}, Tiefe {tiefe:.0f} m.')
                P = [welt(e, t, 0)[:2] for e, t, _, _ in pkt]
                Z = [k for _, _, k, _ in pkt]
                out.append(dict(name=f"Wand_{art}_{seite}_{bt.get('id', 'x')}_{nr}", art='wand', bauteil=bt.get('id'),
                                sli=f'Splines\\Aschaffenburg\\{name}.sli', elemente=kette(P, Z)))
    return out, dateien


def portal_splines(roh, platte, z_unten):
    """je Trogende ein Halbkreis-Spline von der West- zur Ost-Trennwand (ueber die Unterfuehrung): beginnt und endet
    genau an deren Endpunkten (Anschlusspunkte = Portal-Stationen), Krone wie dort (Kreuzungsflaeche + 5 cm),
    Stirnflaeche bis z_unten (Unterkante der Decke) -> (Ketten, Dateien)"""
    from .sonderbau_ring import fuge, welt
    out, dateien, geometrie = [], {}, {}
    s_portal = [e for e in roh if any(abs(e['s'] - sp) < 1e-6 for sp in _portale(roh))]
    for name, e in zip(('nord', 'sued'), s_portal):
        tw, to = fuge(e, 'w'), fuge(e, 'o')
        if tw is None or to is None:
            continue
        pw, po = welt(e, tw, 0)[:2], welt(e, to, 0)[:2]
        c = (pw + po) / 2
        R = float(np.linalg.norm(pw - c))
        u = (pw - c) / R
        dd = np.asarray(e['d'], float) * (1 if name == 'nord' else -1)      # in die Decke hinein
        w = np.linspace(0.0, math.pi, 37)
        P = np.array([c + u * R * math.cos(a) + dd * R * math.sin(a) for a in w])
        Z = np.array([float(platte(x, y)) + 0.05 for x, y in P])
        # Seite des Trogs (Mittelpunkt) in Profil-x: links der Laufrichtung -> -1
        kreuz = dd[0] * (c - pw)[1] - dd[1] * (c - pw)[0]
        rr = -1 if kreuz > 0 else 1
        tiefe = math.ceil(max(Z) - z_unten + 0.5)
        datei_ = f'AB_SB_Wand_portal_beton_{name}_{int(tiefe)}m'
        dateien[datei_] = profil('portal', 'beton', 'bogen', 'keine', rr, tiefe).text(
            f'File created with omsigen (Sonderbau Ring). Portal {name}: Stirn, Bruestung, Gelaender. Tiefe {tiefe:.0f} m.')
        els = kette(P, Z, knoten=2.5)
        geometrie[name] = (c, R, dd)
        out.append(dict(name=f'Wand_portal_{name}', art='wand', bauteil=None, sli=f'Splines\\Aschaffenburg\\{datei_}.sli', elemente=els))
    return out, dateien, geometrie


def halbscheibe(c, R, dd, n=48):
    """Halbkreisflaeche um c mit Radius R in Richtung dd (shapely)"""
    from shapely.geometry import Polygon
    u = np.array([dd[1], -dd[0]])
    pts = [c + u * R * math.cos(a) + dd * R * math.sin(a) for a in np.linspace(0, math.pi, n)]
    return Polygon(pts)


def _portale(roh):
    # Stationen aus dem Raster selbst: Wechsel von 'unter_decke' (das Portal ist die letzte Station davor/danach)
    out = []
    for a, b in zip(roh, roh[1:]):
        if not a['unter_decke'] and b['unter_decke']:
            out.append(a['s'])
        if a['unter_decke'] and not b['unter_decke']:
            out.append(b['s'])
    return out


def schreiben(omsi, dateien, objekt_texturen):
    """.sli-Dateien und ihre Texturen in Splines\\Aschaffenburg (eigener Ordner)"""
    ziel = os.path.join(omsi, 'Splines', 'Aschaffenburg')
    os.makedirs(os.path.join(ziel, 'texture'), exist_ok=True)
    marcel = os.path.join(omsi, 'Splines', 'Marcel', 'texture')
    for k, datei in TEX.items():
        quelle = os.path.join(objekt_texturen, QUELLE[k])
        if not os.path.isfile(quelle):
            quelle = os.path.join(marcel, QUELLE[k])
        if os.path.isfile(quelle):
            shutil.copyfile(quelle, os.path.join(ziel, 'texture', datei))
    for name, text in dateien.items():
        with open(os.path.join(ziel, name + '.sli'), 'w', encoding='cp1252', newline='') as f:
            f.write(text)


def kette_ende(els):
    """Endpunkt der Kette (Pruefung)"""
    e = els[-1]
    return _ende(e)
