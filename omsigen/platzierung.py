"""Platzierung der Gebaeude (Qualitaetsplan G1, docs/qualitaet-anforderungen.md).

1. Lage: Objektursprung = Schwerpunkt des Grundrisses (auf 1 mm), Hoehe = Boden des Gebaeudes (LoD2), Drehung nach der
   Hauptachse (laengere Seite des kleinsten umschliessenden Rechtecks; Grad im Uhrzeigersinn ab Nord wie OMSI und
   belegung._drehen). Das Modell wird in diese Lage umgerechnet - in Kartenkoordinaten bleibt jeder Punkt, wo er war
   (0,1 mm, Genauigkeit der .x-Datei); vorher: Mittel aller Punkte, Drehung 0, Hoehe absolut im Modell.
2. Gruendung: die Waende reichen bis EINBINDEN unter den tiefsten Punkt des Kartengelaendes im Grundriss (mindestens
   UNTER_BODEN wie bisher, hoechstens SOCKEL_MAX unter den Boden - tiefer ist es ein Fehler im Gelaende, den die Pruefung
   meldet). Das Gelaende der Karte liegt im Grundriss hoechstens auf dem Boden des Gebaeudes und wird in PUFFER darum
   nicht ueber DGM/Boden angehoben (Strassenboeschungen; hoehen.Hoehen.raster) - die Schicht boden liest dafuer die
   LoD2-Grundrisse als Quelle wie das DGM, nicht die Schicht gebaeude.
3. Strassenraum (mit S1, querschnitte.Querschnitte.angleichen): je Kante des Netzes die Gebaeudeflucht alle SCHRITT m
   (LoD2 von oben, wie die Belegung). Ragt ein Gebaeude in die Fahrbahn, wird die Fahrbahn der Kante mit Achsversatz
   eingepasst, in echten Engstellen (beide Seiten an derselben Stelle zu nah) auch schmaler. Liegt die OSM-Linie mal
   links, mal rechts daneben ("wellig"), bleibt der Querschnitt und der Konflikt wird gemeldet. Gebaeude werden nie
   verschoben.
4. Pruefen: Konflikte Gebaeude/Fahrbahn mit Klasse (kreuzung, ueberbauung, engstelle, achse_versetzt) und Hinweis zum
   Beheben (belegung.berechnen), Gebaeude ueber Gehweg, schwebende und versunkene Gebaeude (omsigen.pruefbericht).

  python -m omsigen.platzierung <Karte> [--omsi <OMSI>]     Hoehen der Gebaeude gegen Gelaende/DGM messen
"""
import argparse
import json
import math
import os
import pickle

import numpy as np

UNTER_BODEN = 1.0        # m: so weit reichen die Waende mindestens unter den Boden (wie bisher lod2.UNTER_BODEN)
EINBINDEN = 0.3          # m: Wandfuss unter dem tiefsten Gelaendepunkt im Grundriss
SOCKEL_MAX = 6.0         # m: tiefer nicht (tiefer = Gelaende/Strassenhoehe falsch, die Pruefung meldet es)
PUFFER = 2.0             # m um den Grundriss: Gelaende dort nicht ueber DGM/Boden anheben
RAND = 0.3               # m Abstand der Fahrbahn zur Gebaeudeflucht (wie querschnitte.RAND_GEBAEUDE)
TOLERANZ = 0.1           # m: kleinere Ueberstaende in die Fahrbahn aendern den Querschnitt nicht
SCHRITT, ENDE_FREI = 2.0, 4.0   # Stationen der Kantenprofile; die ersten Meter liegen unter der Kreuzungsplatte
VERSATZ_MAX = 2.5        # m: hoechster Achsversatz (wie querschnitte.einpassen)
SCHWEBT_M = 0.1          # m Spalt unter einer Wand: Fund
VERSUNKEN_M = 0.5        # m Gelaende ueber Boden und DGM an einer Wand: Fund (Ursache Strassenboeschung)
VERSUNKEN_RASTER_M = 1.0  # m dasselbe, wenn nur das 5-m-Raster eine Stufe im DGM glaettet (an sich unvermeidbar)
DURCHFAHRT_MIN = 2.5     # m: weniger Platz zwischen den Haeusern = keine Strasse (Ueberbau, OSM-Linie im Haus)
ENG_RAND = 0.1           # m: Engstelle, wenn zwischen den Haeusern weniger als Fahrbahn + ENG_RAND Platz ist
GEHWEG_M2 = 5.0          # m2 Grundriss ueber Gehweg: Fund (Rasterzellen 0,5 m: entlang einer Flucht ~0,25 m Saum)
FORMAT_KANTEN, VERSION_KANTEN = 'omsigen-kantenprofile', 1


# ---------------------------------------------------------------- Grundriss, Lage

def _poly(ringe):
    """Ringe [(n x 2|3)] -> shapely-Flaeche (Vereinigung) oder None"""
    from shapely.geometry import Polygon
    from shapely.ops import unary_union
    teile = []
    for r in ringe:
        r = np.asarray(r, float)
        if len(r) < 3:
            continue
        p = Polygon(r[:, :2])
        if not p.is_valid:
            p = p.buffer(0)
        if p.area > 0.01:
            teile.append(p)
    if not teile:
        return None
    u = unary_union(teile)
    return None if u.is_empty else u


def grundriss(g):
    """Grundriss eines LoD2-Gebaeudes (omsigen.lod2.gebaeude): Bodenflaechen, ohne sie die Daecher von oben"""
    boden = [rs[0] for art, rs in g['flaechen'] if art == 'GroundSurface' and rs]
    p = _poly(boden)
    if p is None:
        p = _poly([rs[0] for art, rs in g['flaechen'] if art == 'RoofSurface' and rs])
    return p


def hauptachse(poly):
    """Drehung (Grad im Uhrzeigersinn ab Nord, -90 < rot <= 90), die die lokale x-Achse auf die laengere Seite des
    kleinsten umschliessenden Rechtecks legt"""
    r = poly.minimum_rotated_rectangle
    if r.geom_type != 'Polygon':
        return 0.0
    c = np.asarray(r.exterior.coords)[:3]
    a, b = c[1] - c[0], c[2] - c[1]
    lang = a if np.hypot(*a) >= np.hypot(*b) else b
    if np.hypot(*lang) < 1e-6:
        return 0.0
    rot = -math.degrees(math.atan2(lang[1], lang[0]))       # lokale x-Achse nach Drehung rot: Winkel -rot ab Ost
    rot = (rot + 90.0) % 180.0 - 90.0
    return 90.0 if abs(rot + 90.0) < 1e-9 else rot


def lage(g, poly=None):
    """Objektlage eines Gebaeudes: dict(x, y (Schwerpunkt des Grundrisses, 1 mm), z (Boden), rot (Grad, 0,001))"""
    poly = grundriss(g) if poly is None else poly
    z = round(float(g['boden']), 3)
    if poly is None or poly.area < 0.05:
        m = g['mitte']
        return dict(x=round(float(m[0]), 3), y=round(float(m[1]), 3), z=z, rot=0.0)
    c = poly.centroid
    return dict(x=round(float(c.x), 3), y=round(float(c.y), 3), z=z, rot=round(hauptachse(poly), 3))


def lokal(P, lg):
    """Kartenpunkte (n x 3: Ost, Nord, Hoehe) -> Objektkoordinaten in der Lage lg (x rechts, y vor, z hoch)"""
    P = np.asarray(P, float).reshape(-1, 3)
    h = math.radians(lg['rot'])
    c, s = math.cos(h), math.sin(h)
    dx, dy = P[:, 0] - lg['x'], P[:, 1] - lg['y']
    return np.c_[dx * c - dy * s, dx * s + dy * c, P[:, 2] - lg['z']]


def lokal_richtung(n, lg):
    h = math.radians(lg['rot'])
    c, s = math.cos(h), math.sin(h)
    return (n[0] * c - n[1] * s, n[0] * s + n[1] * c, n[2])


def welt(Q, lg):
    """Objektkoordinaten -> Kartenpunkte (Umkehrung von lokal; wie OMSI/openOMSI das Objekt setzen)"""
    Q = np.asarray(Q, float).reshape(-1, 3)
    h = math.radians(lg['rot'])
    c, s = math.cos(h), math.sin(h)
    return np.c_[Q[:, 0] * c + Q[:, 1] * s + lg['x'], -Q[:, 0] * s + Q[:, 1] * c + lg['y'], Q[:, 2] + lg['z']]


# ---------------------------------------------------------------- Gruendung

def gelaende_min(hoehe, poly, schritt=0.5, raster=5.0):
    """tiefster Punkt des Gelaendes hoehe(x, y) im Grundriss: Umriss alle `schritt` m und die Rasterpunkte darin
    (das Gelaende ist je Rasterdreieck eben) -> Hoehe oder None (kein Gelaende)"""
    if poly is None:
        return None
    import shapely
    werte = []
    for p in getattr(poly, 'geoms', [poly]):
        for ring in [p.exterior] + list(p.interiors):
            c = np.asarray(ring.coords)
            for a, b in zip(c[:-1], c[1:]):
                n = max(1, int(math.ceil(math.dist(a, b) / schritt)))
                for f in np.arange(n) / n:
                    werte.append(hoehe(a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f))
    x0, y0, x1, y1 = poly.bounds
    xs = np.arange(math.ceil(x0 / raster), math.floor(x1 / raster) + 1) * raster
    ys = np.arange(math.ceil(y0 / raster), math.floor(y1 / raster) + 1) * raster
    if len(xs) and len(ys):
        X, Y = np.meshgrid(xs, ys)
        m = shapely.contains_xy(poly, X, Y)
        werte += [hoehe(x, y) for x, y in zip(X[m], Y[m])]
    werte = [w for w in werte if w is not None]
    return min(werte) if werte else None


def unten(boden, t_min):
    """Hoehe des Wandfusses: UNTER_BODEN unter dem Boden, tiefer bis EINBINDEN unter das Gelaende (hoechstens
    SOCKEL_MAX unter den Boden)"""
    u = boden - UNTER_BODEN
    if t_min is not None:
        u = min(u, max(t_min - EINBINDEN, boden - SOCKEL_MAX))
    return u


# ---------------------------------------------------------------- LoD2-Grundrisse (UTM) mit Zwischenspeicher

def _kachel_grundrisse(pfad):
    """alle Gebaeude einer LoD2-Kachel: [dict(id, funktion, boden (NN), mitte, grund, oben)] - Ringe in UTM (x, y);
    grund = Bodenflaechen, oben = Daecher und Boden (Ansicht von oben). Zwischenspeicher <Kachel>.grundrisse.pkl"""
    from . import lod2
    cache = pfad + '.grundrisse.pkl'
    st = os.stat(pfad)
    kennung = (st.st_size, int(st.st_mtime), 1)
    try:
        with open(cache, 'rb') as f:
            d = pickle.load(f)
        if d.get('kennung') == kennung:
            return d['gebaeude']
    except (OSError, ValueError, EOFError, pickle.UnpicklingError, KeyError, AttributeError):
        pass
    out = []
    for g in lod2.gebaeude(pfad, dict(ost0=0.0, nord0=0.0, nn0=0.0)):
        grund = [np.asarray(rs[0][:, :2], np.float64) for art, rs in g['flaechen'] if art == 'GroundSurface' and rs]
        oben = [np.asarray(rs[0][:, :2], np.float64) for art, rs in g['flaechen']
                if art in ('RoofSurface', 'GroundSurface') and rs]
        out.append(dict(id=g['id'], funktion=g['funktion'], boden=g['boden'], mitte=np.asarray(g['mitte'], float),
                        grund=grund, oben=oben))
    try:
        with open(cache + '.tmp', 'wb') as f:
            pickle.dump(dict(kennung=kennung, gebaeude=out), f, protocol=pickle.HIGHEST_PROTOCOL)
        os.replace(cache + '.tmp', cache)
    except OSError:
        pass
    return out


def lod2_grundrisse(e0, n0, e1, n1, laden_erlaubt=False, log=print):
    """LoD2-Gebaeude mit Mitte im UTM-Ausschnitt (fehlende Kacheln nur mit laden_erlaubt)"""
    from . import lod2
    out = []
    for e in range(int(e0 // 2000 * 2), int(e1 // 2000 * 2) + 1, 2):
        for n in range(int(n0 // 2000 * 2), int(n1 // 2000 * 2) + 1, 2):
            pfad = lod2.datei(e, n)
            if not os.path.isfile(pfad):
                if not laden_erlaubt:
                    continue
                try:
                    lod2.laden(e, n)
                except Exception as ex:
                    log(f'    LoD2-Kachel {e}_{n} nicht ladbar ({type(ex).__name__})')
                    continue
            for b in _kachel_grundrisse(pfad):
                if e0 <= b['mitte'][0] <= e1 and n0 <= b['mitte'][1] <= n1:
                    out.append(b)
    return out


def _verschoben(ringe, dx, dy):
    return [r - (dx, dy) for r in ringe]


# ---------------------------------------------------------------- Gelaende nicht ueber Gebaeude anheben

class Grundrisse:
    """Grundrisse (Projektmeter) mit Bodenhoehe (relativ zur Hoehenbasis) fuer hoehen.Hoehen.raster"""

    def __init__(self, flaechen, boeden, puffer=PUFFER):
        from shapely import STRtree
        self.flaechen = flaechen
        self.boeden = np.asarray(boeden, float)
        self.puffer = [f.buffer(puffer) for f in flaechen]
        self.baum = STRtree(self.puffer) if self.puffer else None

    def __len__(self):
        return len(self.flaechen)

    def klemmen(self, X, Z, H, H0, frei):
        """Rasterhoehen H (an den Punkten X, Z): im Grundriss hoechstens der Boden (dort sieht man das Gelaende nicht,
        aber es zieht die Waende entlang nach oben - auch wo eine Strasse in das Haus ragt), im PUFFER darum hoechstens
        max(DGM H0, Boden); frei: Punkte, die keine Strasse braucht (dort bleibt sonst die Strassenhoehe)"""
        if self.baum is None:
            return H
        import shapely
        from shapely.geometry import box
        for i in self.baum.query(box(float(X.min()), float(Z.min()), float(X.max()), float(Z.max()))):
            m = shapely.contains_xy(self.puffer[i], X, Z)
            if not m.any():
                continue
            innen = m & shapely.contains_xy(self.flaechen[i], X, Z)    # im Haus auch unter einer Strasse
            m = innen | (m & frei)
            grenze = np.where(innen, self.boeden[i], np.maximum(H0, self.boeden[i]))
            H = np.where(m, np.minimum(H, grenze), H)
        return H


def gelaende_vorbereiten(hoe, proj, linie, breite, log=print):
    """Grundrisse der LoD2-Gebaeude im Korridor (Mitte bis `breite` m um die Linie, wie pipeline.gebaeude_schicht) an
    hoe haengen: hoehen.Hoehen.raster hebt das Gelaende dort nicht ueber DGM/Boden an. -> Anzahl"""
    L = np.asarray(linie, float).reshape(-1, 2)
    if not len(L) or not hasattr(proj, 'e0'):
        return 0
    lo, hi = L.min(0) - breite - 10, L.max(0) + breite + 10
    gs = lod2_grundrisse(lo[0] + proj.e0, lo[1] + proj.n0, hi[0] + proj.e0, hi[1] + proj.n0, laden_erlaubt=True, log=log)
    a, b = (L[:-1], L[1:]) if len(L) > 1 else (L, L)
    ab = b - a
    flaechen, boeden = [], []
    for g in gs:
        p = g['mitte'] - (proj.e0, proj.n0)
        t = np.clip(((p - a) * ab).sum(1) / np.maximum((ab * ab).sum(1), 1e-9), 0, 1)
        if np.linalg.norm(a + t[:, None] * ab - p, axis=1).min() > breite + PUFFER + 20:
            continue
        f = _poly(_verschoben(g['grund'], proj.e0, proj.n0))
        if f is not None:
            flaechen.append(f)
            boeden.append(g['boden'] - hoe.base)
    hoe.gebaeude = Grundrisse(flaechen, boeden)
    log(f'    Gelaende: {len(flaechen)} Gebaeudegrundrisse - dort keine Anhebung ueber DGM/Boden (G1)')
    return len(flaechen)


# ---------------------------------------------------------------- Fahrbahn je Kante einpassen (mit S1)

def ueberbaut(li, re):
    """Stationen ohne Platz fuer irgendeine Strasse (Achse im Gebaeude oder weniger als DURCHFAHRT_MIN zwischen den
    Haeusern): Durchfahrt, Ueberbau oder die OSM-Linie liegt dort im Haus - kein Querschnitt hilft, die Pruefung meldet
    es -> (li, re) ohne diese Stationen"""
    paare = [(a, b) for a, b in zip(li, re)
             if not ((a is not None and a < 0.05) or (b is not None and b < 0.05)
                     or (a is not None and b is not None and a + b < DURCHFAHRT_MIN))]
    return [a for a, _ in paare], [b for _, b in paare]


def _stationen(q, li, re, rand):
    """je Station der zulaessige Zusatzversatz der Fahrbahn [lo_i, hi_i], damit sie `rand` m von den Gebaeuden bleibt
    (li/re: Abstand der Gebaeude links/rechts der Achse je Station, None = frei; ueberbaute Stationen zaehlen nicht)"""
    from .querschnitte import info
    li, re = ueberbaut(li, re)
    i = info(q)
    lo = np.array([-i['cl'] - (v - rand) if v is not None else -math.inf for v in li], float)
    hi = np.array([(v - rand) - i['cr'] if v is not None else math.inf for v in re], float)
    return lo, hi


def fahrbahn_grenzen(q, li, re, rand=RAND):
    """zulaessiger Zusatzversatz (lo, hi) fuer alle Stationen zugleich"""
    lo, hi = _stationen(q, li, re, rand)
    return float(lo.max(initial=-math.inf)), float(hi.min(initial=math.inf))


def _ueber(lo, hi, v):
    """je Station: so weit liegt die Fahrbahn mit Zusatzversatz v im Gebaeude"""
    return np.maximum(lo - v, 0.0) + np.maximum(v - hi, 0.0)


def _bester_versatz(q, li, re, versatz_max):
    """Zusatzversatz (Raster) mit der kleinsten Quadratsumme der Ueberstaende (Abstand RAND), bei Gleichstand der
    kleinste -> (v, Ueberstand je Station ohne Abstand)"""
    from .querschnitte import RASTER
    lo, hi = _stationen(q, li, re, RAND)
    v0 = q.get('versatz', 0.0)
    n = int(round(versatz_max / RASTER))
    kandidaten = sorted((k * RASTER - v0 for k in range(-n, n + 1)), key=abs)
    kosten = [float((_ueber(lo, hi, v) ** 2).sum()) for v in kandidaten]
    v = kandidaten[int(np.argmin(kosten))]
    lo0, hi0 = _stationen(q, li, re, 0.0)
    return v, _ueber(lo0, hi0, v)


def _schmaler(q, ziel, spur_min):
    """Fahrbahn (zwischen den Fahrbahnkanten) auf hoechstens `ziel` m: Parkstreifen weg, dann Radfahrstreifen, dann
    Spuren bis spur_min -> True, wenn etwas geaendert wurde"""
    from .querschnitte import RASTER, SPUR, info
    geaendert = False

    def breite():
        i = info(q)
        return i['cr'] - i['cl']
    for art in ('parkstreifen', 'radfahrstreifen'):
        while breite() > ziel + 1e-6:
            k = [i for i, t in enumerate(q['teile']) if t['art'] == art]
            if not k:
                break
            i = max(k, key=lambda j: q['teile'][j]['breite'])
            del q['teile'][i]
            if q['marken']:
                del q['marken'][min(i, len(q['marken']) - 1)]
            geaendert = True
    spuren = [t for t in q['teile'] if t['art'] in SPUR]
    zuviel = breite() - ziel
    if spuren and zuviel > 1e-6:
        w = spuren[0]['breite']
        neu = max(spur_min, w - zuviel / len(spuren))
        neu = math.floor(neu / RASTER + 1e-6) * RASTER
        if neu < w - 1e-9:
            for t in spuren:
                t['breite'] = round(max(neu, spur_min), 3)
            geaendert = True
    return geaendert


def fahrbahn_einpassen(q, li, re, spur_min, versatz_max=VERSATZ_MAX):
    """Querschnitt einer Kante gegen ihr Profil (li/re je Station) -> (neuer Querschnitt oder None, Grund):
    'versatz' (ein Achsversatz genuegt), 'engstelle' (an mindestens zwei Stationen ist zwischen den Haeusern weniger
    Platz als die Fahrbahn breit ist: Parkstreifen/Radfahrstreifen weg, Spuren schmaler), 'wellig' (die Gebaeude stehen
    mal links, mal rechts in der Fahrbahn: die OSM-Linie liegt lokal daneben - nur ein Versatz, der den groessten
    Ueberstand um mindestens 0,3 m verkleinert, der Rest bleibt ein Fund), '' (passt).
    Der Versatz ist der mit der kleinsten Quadratsumme der Ueberstaende (bis versatz_max)."""
    import copy
    from .querschnitte import RASTER, info
    li, re = ueberbaut(li, re)
    lo0, hi0 = _stationen(q, li, re, 0.0)
    vorher = _ueber(lo0, hi0, 0.0)
    if not len(vorher) or vorher.max() <= TOLERANZ:
        return None, ''
    q2 = copy.deepcopy(q)
    grund = 'versatz'
    v, rest = _bester_versatz(q2, li, re, versatz_max)
    if rest.max() > TOLERANZ:
        i = info(q2)
        platz = [a + b for a, b, x in zip(li, re, rest) if x > TOLERANZ and a is not None and b is not None]
        eng = [w for w in platz if w < i['cr'] - i['cl']]
        if len(eng) >= 2 and _schmaler(q2, min(eng) - 2 * RAND, spur_min):
            grund = 'engstelle'
            v, rest = _bester_versatz(q2, li, re, versatz_max)
    if rest.max() > TOLERANZ and grund != 'engstelle':
        grund = 'wellig'
        if rest.max() > vorher.max() - 0.3:
            return None, grund
    q2['versatz'] = round(round((q2.get('versatz', 0.0) + v) / RASTER) * RASTER, 3)
    if q2 == q:
        return None, grund
    return q2, grund


def gehwege_einpassen(q, li, re):
    """Seitenraum einer Kante an die naechsten Gebaeude ihres Profils: je Seite Gruenstreifen weg, dann der Gehweg
    schmaler (hoechstens bis querschnitte.GEHWEG_MIN - nie ganz weg; was dann noch im Haus steht, meldet die Pruefung)
    -> (neuer Querschnitt oder None, geaenderte Seiten ['links'/'rechts'])"""
    import copy
    from .querschnitte import GEHWEG_MIN, RASTER, SPUR, info
    li, re = ueberbaut(li, re)
    q2 = copy.deepcopy(q)
    seiten = []
    for seite, werte in ((0, li), (1, re)):
        w = [v for v in werte if v is not None]
        if not w:
            continue
        i = info(q2)
        zuviel = (-i['ol'] if seite == 0 else i['or_']) - (min(w) - RAND)
        if zuviel <= TOLERANZ:
            continue
        sp = [j for j, t in enumerate(q2['teile']) if t['art'] in SPUR]
        if not sp:
            continue
        aussen = list(range(0, sp[0])) if seite == 0 else list(range(len(q2['teile']) - 1, sp[-1], -1))
        weg = []
        for j in aussen:                                   # von aussen nach innen
            t = q2['teile'][j]
            if zuviel <= TOLERANZ:
                break
            if t['art'] == 'gruen':
                zuviel -= t['breite']
                weg.append(j)
            elif t['art'] == 'gehweg' and t['breite'] > GEHWEG_MIN:
                neu = max(GEHWEG_MIN, math.floor((t['breite'] - zuviel) / RASTER + 1e-6) * RASTER)
                zuviel -= t['breite'] - neu
                t['breite'] = round(neu, 3)
        for j in sorted(weg, reverse=True):
            del q2['teile'][j]
            if q2['marken']:
                del q2['marken'][min(j, len(q2['marken']) - 1)]
        if q2 != q and (not seiten or seiten[-1] != ('links', 'rechts')[seite]):
            seiten.append(('links', 'rechts')[seite])
    return (q2, seiten) if q2 != q else (None, [])


def pruefsumme(P, e0, n0):
    import hashlib
    s = ';'.join(f'{round((x + e0) * 2)},{round((z + n0) * 2)}' for x, z in P)
    return hashlib.sha1(f'{SCHRITT}/{ENDE_FREI}|{s}'.encode()).hexdigest()[:16]


class Kantenprofile:
    """Gebaeudeflucht je Netzkante (Projektmeter) fuer querschnitte.Querschnitte.angleichen: profil(P) -> (li, re);
    Zwischenspeicher je Kantengeometrie in den Kartendaten (querschnitte/kantenprofile.json), die LoD2-Grundrisse
    werden nur gelesen, wenn ein Profil fehlt"""

    def __init__(self, proj, bereich, datei=None, log=print):
        from .config import KARTENDATEN
        self.proj, self.bereich, self.log = proj, bereich, log
        self.datei = datei or os.path.join(KARTENDATEN, 'querschnitte', 'kantenprofile.json')
        self.cache, self.neu = {}, 0
        try:
            with open(self.datei, encoding='utf-8') as f:
                d = json.load(f)
            if d.get('version') == VERSION_KANTEN:
                self.cache = d.get('kanten', {})
        except (OSError, ValueError):
            pass
        self._baum = None
        self.statistik = {}

    @classmethod
    def fuer(cls, projekt, proj, log=print):
        """fuer ein UTM-Projekt (Bereich = alle Strassen + 30 m) oder None"""
        if getattr(proj, 'art', '') != 'utm' and not hasattr(proj, 'e0'):
            return None
        P = [q for s in projekt.get('strassen', []) for q in s.get('punkte', [])]
        if not P:
            return None
        P = np.asarray(P, float)[:, :2]
        lo, hi = P.min(0) - 30, P.max(0) + 30
        return cls(proj, (lo[0], lo[1], hi[0], hi[1]), log=log)

    def _gebaeude(self):
        if self._baum is None:
            from shapely import STRtree
            x0, y0, x1, y1 = self.bereich
            e0, n0 = self.proj.e0, self.proj.n0
            gs = lod2_grundrisse(x0 + e0 - 40, y0 + n0 - 40, x1 + e0 + 40, y1 + n0 + 40, log=self.log)
            self._polys = [p for p in (_poly(_verschoben(g['oben'], e0, n0)) for g in gs) if p is not None]
            self._baum = STRtree(self._polys) if self._polys else False
            self.log(f'    Querschnitte: Gebaeudeflucht je Kante aus {len(self._polys)} LoD2-Gebaeuden (G1)')
        return self._baum

    def profil(self, P):
        from .querschnitt_messung import flucht
        k = pruefsumme(P, self.proj.e0, self.proj.n0)
        if k in self.cache:
            return self.cache[k]
        baum = self._gebaeude()
        if not baum:
            return None
        li, re = flucht(P, baum, self._polys, schritt=SCHRITT, ende_frei=ENDE_FREI)
        r = lambda v: None if v is None else round(float(v), 2)
        self.cache[k] = [[r(v) for v in li], [r(v) for v in re]]
        self.neu += 1
        return self.cache[k]

    def speichern(self):
        if not self.neu:
            return
        os.makedirs(os.path.dirname(self.datei), exist_ok=True)
        with open(self.datei + '.tmp', 'w', encoding='utf-8') as f:
            json.dump(dict(format=FORMAT_KANTEN, version=VERSION_KANTEN, kanten=self.cache), f, separators=(',', ':'))
        os.replace(self.datei + '.tmp', self.datei)
        self.neu = 0

    @staticmethod
    def _seitenrichtig(e, prof):
        """Profil in Richtung der aktuellen Linie der Kante (angleichen dreht Kanten um)"""
        if e.get('_profil_p0') is not None and tuple(e['P'][0]) != e['_profil_p0']:
            return prof[1][::-1], prof[0][::-1]
        return prof[0], prof[1]

    def einpassen(self, qs, edges):
        """Fahrbahn jeder Kante gegen ihre Gebaeudeflucht (vor dem Angleichen; qs = querschnitte.Querschnitte)"""
        from .config import is_oneway
        from .querschnitte import SPUR_MIN, SPUR_MIN_GASSE, kanonisch, spiegeln, symmetrisch
        st = self.statistik
        for e in edges:
            if e.get('ring') or e['spl'] not in qs.typen or len(e['P']) < 2:
                continue
            prof = self.profil([tuple(p) for p in e['P']])
            if not prof:
                continue
            e['_profil'], e['_profil_p0'] = prof, tuple(e['P'][0])
            q = qs.typen[e['spl']]['q']
            hw = e['t'].get('highway', '')
            q2, grund = fahrbahn_einpassen(q, prof[0], prof[1], SPUR_MIN_GASSE.get(hw, SPUR_MIN))
            if grund:
                st[grund] = st.get(grund, 0) + 1
            q3, seiten = gehwege_einpassen(q2 or q, prof[0], prof[1])
            if q3 is not None:
                q2 = q3
                st['gehweg'] = st.get('gehweg', 0) + 1
            if q2 is None:
                continue
            einbahn = is_oneway(e['t'])
            if not einbahn and not kanonisch(q2):
                q2 = spiegeln(q2)
                e['P'] = e['P'][::-1]
                e['osm_rev'] = not e.get('osm_rev')
            rel = qs.registrieren(q2)
            alt = qs.herkunft(e['t'], e['spl'])
            if alt:
                neu = dict(herkunft=dict(alt['herkunft']), werte=dict(alt['werte']))
                if grund:
                    neu['herkunft']['fahrbahn_gebaeude'] = grund
                for seite in seiten:
                    neu['herkunft'][f'gehweg_{seite}'] = 'gebaeude'
                neu['werte']['versatz'] = q2['versatz']
                qs.wahl[(qs.weg_schluessel(e['t']), rel)] = neu
            e['spl'] = rel
            e['force'] = einbahn or not symmetrisch(q2)
        self.speichern()
        if st:
            self.log('    Querschnitte an Gebaeuden (G1): ' + ', '.join(f'{k} {n}' for k, n in sorted(st.items())))

    def frei(self, e, q, q_alt=None):
        """True, wenn Querschnitt q (in Richtung der Linie von e) die Fahrbahn von e nicht tiefer in Gebaeude legt als
        q_alt (bzw. hoechstens TOLERANZ)"""
        prof = e.get('_profil')
        if not prof:
            return True
        li, re = self._seitenrichtig(e, prof)

        def ueber(qq):
            lo, hi = fahrbahn_grenzen(qq, li, re, 0.0)
            return max(lo, -hi, 0.0)
        return ueber(q) <= max(TOLERANZ, ueber(q_alt) if q_alt is not None else 0.0) + 1e-9


# ---------------------------------------------------------------- Konflikte einordnen (belegung.berechnen)

KLASSEN = {'kreuzung': 'Kreuzungsplatte', 'ueberbauung': 'Ueberbauung', 'engstelle': 'Engstelle',
           'achse_versetzt': 'Strassenachse versetzt'}


class Einordnung:
    """Gebaeude ueber Fahrbahn/Kreuzung einordnen: wo die Ursache liegt und wie man sie behebt"""

    def __init__(self, kartenordner, netz, grundrisse, lw, kachel=300.0, zelle=0.5, omsi=None):
        self.ordner, self.netz, self.lw = kartenordner, netz, lw
        self.omsi = omsi or os.path.dirname(os.path.dirname(os.path.abspath(kartenordner)))
        self.kachel, self.zelle = kachel, zelle
        self.grundrisse = grundrisse
        self.kasten = [self._kasten(t) for _, t in grundrisse]
        self._polys = {}
        A, B, kante, sta, stb = [], [], [], [], []
        self.kanten = []
        for e in netz.kanten.values():
            if e['art'] != 'strasse' or not e.get('querschnitt'):
                continue
            ix = len(self.kanten)
            self.kanten.append(e)
            P = e['punkte']
            for a, b in zip(P, P[1:]):
                A.append((a[0], a[2]))
                B.append((b[0], b[2]))
                kante.append(ix)
                sta.append(a[3])
                stb.append(b[3])
        self.A, self.B = np.asarray(A, float).reshape(-1, 2), np.asarray(B, float).reshape(-1, 2)
        self.kante, self.sta, self.stb = np.asarray(kante, int), np.asarray(sta, float), np.asarray(stb, float)
        self.funktion = {}

    @staticmethod
    def _kasten(tris):
        if not tris:
            return (0, 0, 0, 0)
        P = np.asarray([p for t in tris for p in t], float)
        return (*P.min(0), *P.max(0))

    def _poly(self, i):
        if i not in self._polys:
            from shapely.geometry import Polygon
            from shapely.ops import unary_union
            tris = [Polygon(t) for t in self.grundrisse[i][1]]
            self._polys[i] = unary_union([t for t in tris if t.is_valid and t.area > 1e-6]) if tris else None
        return self._polys[i]

    def _funktion(self, rel):
        """LoD2-Funktion aus dem Schicht-Paket des Gebaeudes (lod2.json neben dem Objekt)"""
        from . import schichten
        ordner = os.path.dirname(os.path.join(self.omsi, *rel.split(chr(92))))
        if ordner not in self.funktion:
            try:
                with open(os.path.join(ordner, 'lod2.json'), encoding='utf-8') as f:
                    self.funktion[ordner] = {o['rel']: o.get('funktion', '') for o in json.load(f).get('objekte', [])}
            except (OSError, ValueError):
                self.funktion[ordner] = {}
        return self.funktion[ordner].get(rel, '')

    def naechste(self, X):
        """naechste Strassenkante je Punkt (m x 2) -> (Kantenindex, Station, Querlage rechts positiv)"""
        X = np.asarray(X, float).reshape(-1, 2)
        lo, hi = X.min(0) - 40, X.max(0) + 40
        sel = np.nonzero((np.minimum(self.A, self.B) <= hi).all(1) & (np.maximum(self.A, self.B) >= lo).all(1))[0]
        if not len(sel):
            return None
        A, B = self.A[sel], self.B[sel]
        D = B - A
        ll = np.maximum((D * D).sum(1), 1e-12)
        t = np.clip((((X[:, None, :] - A[None]) * D[None]).sum(2)) / ll[None], 0, 1)
        Q = A[None] + t[..., None] * D[None]
        d = np.hypot(*(X[:, None, :] - Q).transpose(2, 0, 1))
        j = d.argmin(1)
        a, dd = A[j], D[j]
        seite = ((X[:, 0] - a[:, 0]) * dd[:, 1] - (X[:, 1] - a[:, 1]) * dd[:, 0]) / np.sqrt(ll[j])
        st = self.sta[sel][j] + (self.stb[sel][j] - self.sta[sel][j]) * t[np.arange(len(X)), j]
        return self.kante[sel][j], st, seite

    def _strahl(self, x, z, richtung, ohne, laenge=30.0):
        """Abstand bis zum naechsten Gebaeude (ausser Index `ohne`) entlang richtung oder None"""
        from shapely.geometry import LineString, Point
        q = (x + richtung[0] * laenge, z + richtung[1] * laenge)
        s = LineString([(x, z), q])
        x0, z0, x1, z1 = s.bounds
        best = None
        for i, (a, b, c, d) in enumerate(self.kasten):
            if i == ohne or c < x0 or a > x1 or d < z0 or b > z1:
                continue
            p = self._poly(i)
            if p is None:
                continue
            xs = s.intersection(p)
            if not xs.is_empty:
                dd = Point(x, z).distance(xs)
                best = dd if best is None else min(best, dd)
        return best

    def einordnen(self, i, zellen_fahrbahn, zellen_kreuzung):
        """Gebaeude i (Index in grundrisse) mit den Zellmitten ueber Fahrbahn bzw. Kreuzungsplatte -> dict(klasse,
        hinweis, ...)"""
        from . import netz_datei
        rel = self.grundrisse[i][0]
        nf, nk = len(zellen_fahrbahn), len(zellen_kreuzung)
        if nk >= nf:
            return dict(klasse='kreuzung', hinweis='reicht in die Kreuzungsplatte bzw. den Uebergang (Eckausrundung, '
                        'Plattengroesse) - Kreuzung im Editor anpassen')
        if self._funktion(rel).startswith('51009'):
            return dict(klasse='ueberbauung', hinweis='Ueberdachung ueber der Fahrbahn (LoD2 51009) - echt, '
                        'Strasse oder Dach pruefen')
        r = self.naechste(zellen_fahrbahn)
        if r is None:
            return dict(klasse='achse_versetzt', hinweis='keine Strassenkante in der Naehe')
        kix, st, seite = r
        k = int(np.bincount(kix).argmax())
        m = kix == k
        e = self.kanten[k]
        st, seite = st[m], seite[m]
        t = e.get('attribute', {}).get('tags', {})
        if t.get('tunnel') in ('building_passage', 'yes') or t.get('covered') == 'yes' or \
                (seite.min() < -0.5 and seite.max() > 0.5):
            return dict(klasse='ueberbauung', hinweis=f'steht ueber {e["attribute"].get("name") or "der Strasse"} '
                        '(Durchfahrt/Ueberbau) - echt: Strasse als Durchfahrt bauen oder Gebaeude pruefen', kante=e['id'])
        cl, cr = e['querschnitt']['fahrbahn']
        rechts = float(np.median(seite)) > 0
        eigen = float(np.abs(seite).min())               # Zellmitte: PIL fuellt angeschnittene Zellen mit
        s0 = float(np.median(st))
        x, _, z, h = self.netz.punkt(e['id'], s0)
        rx, rz = netz_datei.rechts(h)
        sg = -1.0 if rechts else 1.0
        gegen = self._strahl(x, z, (sg * rx, sg * rz), i)
        W = cr - cl
        tiefe = (cr - eigen) if rechts else (-cl - eigen)
        name = e['attribute'].get('name') or e['id']
        if tiefe <= 0.0:                                 # nicht die Fahrbahn dieser Strasse: Nachbarstrasse/-flaeche
            return dict(klasse='kreuzung', hinweis=f'Fahrbahnflaeche neben {name} (Uebergang/Kreuzung) - im Editor '
                        'anpassen', kante=e['id'])
        if gegen is not None and eigen + gegen < W + ENG_RAND:
            return dict(klasse='engstelle', hinweis=f'Engstelle {name}: zwischen den Haeusern {eigen + gegen:.1f} m, '
                        f'Fahrbahn {W:.1f} m - Querschnitt zu breit (Breite im Editor messen)', kante=e['id'],
                        platz=round(eigen + gegen, 2), fahrbahn=round(W, 2))
        frei = 'frei' if gegen is None else f'{gegen + (cl if rechts else -cr):.1f} m'
        return dict(klasse='achse_versetzt', hinweis=f'{name}: Fahrbahn {tiefe:.1f} m im Gebaeude, gegenueber {frei} '
                    'bis zum naechsten Haus - OSM-Linie liegt hier neben der Strassenmitte (Strasse im Editor '
                    'verschieben)', kante=e['id'], tiefe=round(tiefe, 2))


# ---------------------------------------------------------------- Hoehen pruefen (schwebt / versunken)

def _bezug(kartenordner):
    p = os.path.join(kartenordner, 'omsi-editor-geo.cfg')
    if not os.path.isfile(p):
        return None
    from .fotovergleich import bezug_lesen
    try:
        return bezug_lesen(kartenordner)
    except Exception:
        return None


class _DgmRaster:
    """das DGM, wie es das 5-m-Raster der Karte zeigen wuerde (Rasterpunkte aus dem DGM, je Zelle zwei Dreiecke wie
    sonderbau_ring.Kartengelaende): zeigt, ob eine Abweichung vom Raster kommt oder von den Strassen"""

    def __init__(self, gel, bez, schritt=5.0):
        self.gel, self.bez, self.s, self.p = gel, bez, schritt, {}

    def _punkt(self, i, j):
        if (i, j) not in self.p:
            d = self.gel.hoehe_utm(i * self.s + self.bez['ost0'], j * self.s + self.bez['nord0'])
            self.p[(i, j)] = None if d is None else d - self.bez['nn0']
        return self.p[(i, j)]

    def hoehe(self, x, y):
        i, j = math.floor(x / self.s), math.floor(y / self.s)
        fx, fy = x / self.s - i, y / self.s - j
        h00, h10, h01, h11 = self._punkt(i, j), self._punkt(i + 1, j), self._punkt(i, j + 1), self._punkt(i + 1, j + 1)
        if None in (h00, h10, h01, h11):
            return None
        if fx >= fy:
            return h00 + fx * (h10 - h00) + fy * (h11 - h10)
        return h00 + fy * (h01 - h00) + fx * (h11 - h01)


def gebaeude_hoehen(kartenordner, omsi=None, mit_dgm=True):
    """je Gebaeude der Schicht gebaeude: Wandfuss gegen Kartengelaende (Spalt = schwebt) und Kartengelaende gegen Boden
    und DGM (versunken) -> [dict(rel, x, z, spalt, angehoben, ursache, boden, unten)]"""
    from . import belegung, schichten
    from .sonderbau_ring import Kartengelaende
    omsi = omsi or schichten.STANDARD_OMSI
    kg = Kartengelaende(kartenordner)
    bez = _bezug(kartenordner) if mit_dgm else None
    gel = None
    if bez:
        from .gelaende import Gelaende
        gel = Gelaende(None, cache=os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), '.cache', 'dgm1'),
                       laden=False)
        raster = _DgmRaster(gel, bez)
    out = []
    objekte = belegung._objekte(kartenordner, lambda r: (schichten.schicht_von(r) or ('',))[0] == 'gebaeude')
    hoehen = {}
    for k in schichten.kacheln(kartenordner):              # Objekthoehe (z) je Pfad
        _, bl = schichten.bloecke(schichten.lesen(kartenordner, k))
        for b in bl:
            rel = schichten.objekt_pfad(b)
            if b[0].strip() == '[object]' and rel:
                try:
                    hoehen[rel] = float(b[6])
                except (ValueError, IndexError):
                    pass
    for rel, ox, oz, rot, _ in objekte:
        pfad = belegung._modellpfad(omsi, rel)
        if not pfad or not os.path.isfile(pfad):
            continue
        try:
            P, F, _, _ = belegung.x_lesen(pfad)
        except (ValueError, IndexError):
            continue
        oy = hoehen.get(rel, 0.0)
        x, z = belegung._drehen(P[:, 0], P[:, 2], rot)
        W = np.c_[x + ox, z + oz, P[:, 1] + oy]
        fuss = W[:, 2].min()
        if W[:, 2].max() - fuss < 1.0:                # ohne Waende (nur Dach/Flaeche): nichts zu gruenden
            continue
        unten_v = W[:, 2] < fuss + 0.05
        boden = oy if abs(oy) > 1e-6 else fuss + UNTER_BODEN          # alte Karten: Objekt auf 0, Schuerze 1 m
        kanten = set()
        for f in F:
            for a, b in ((f[0], f[1]), (f[1], f[2]), (f[2], f[0])):
                if unten_v[a] and unten_v[b]:
                    kanten.add((min(a, b), max(a, b)))
        spalt, angehoben, ursache = -math.inf, -math.inf, ''
        for a, b in kanten:
            pa, pb = W[a], W[b]
            n = max(1, int(math.ceil(math.dist(pa[:2], pb[:2]) / 0.5)))
            for t in np.arange(n + 1) / n:
                p = pa + (pb - pa) * t
                h = kg.hoehe(p[0], p[1])
                if h is None:
                    continue
                spalt = max(spalt, p[2] - h)
                if gel is not None:
                    d = gel.hoehe_utm(p[0] + bez['ost0'], p[1] + bez['nord0'])
                    if d is not None:
                        a_ = h - max(boden, d - bez['nn0'])
                        if a_ > angehoben:
                            angehoben = a_
                            dr = raster.hoehe(p[0], p[1])
                            ursache = 'raster' if dr is not None and h - max(boden, dr) < 0.2 else 'strasse'
        if spalt == -math.inf:
            continue
        out.append(dict(rel=rel, x=round(float(ox), 1), z=round(float(oz), 1), spalt=round(float(spalt), 2),
                        angehoben=None if angehoben == -math.inf else round(float(angehoben), 2), ursache=ursache,
                        boden=round(float(boden), 2), unten=round(float(fuss), 2)))
    return out


def hoehen_funde(kartenordner, omsi=None, log=print):
    """Funde fuer den Pruefbericht: gebaeude_schwebt (Spalt unter einer Wand), gebaeude_versunken (Gelaende der Karte
    ueber Boden und DGM)"""
    funde = []
    for g in gebaeude_hoehen(kartenordner, omsi):
        name = os.path.basename(g['rel'].replace(chr(92), '/'))
        if g['spalt'] > SCHWEBT_M:
            funde.append(dict(art='gebaeude_schwebt', x=g['x'], z=g['z'], text=f'{name}: Spalt bis {g["spalt"]:.1f} m '
                              f'unter der Wand (Sockel {g["boden"] - g["unten"]:.1f} m reicht nicht) - Gelaende/Strassenhoehe '
                              'dort pruefen'))
        if g['angehoben'] is not None and g['angehoben'] > (VERSUNKEN_M if g['ursache'] == 'strasse' else VERSUNKEN_RASTER_M):
            grund = ('Strassenboeschung hebt das Gelaende an' if g['ursache'] == 'strasse'
                     else 'Gelaenderaster (5 m) glaettet eine Stufe im DGM')
            funde.append(dict(art='gebaeude_versunken', x=g['x'], z=g['z'], text=f'{name}: Gelaende bis '
                              f'{g["angehoben"]:.1f} m ueber dem Boden - {grund}'))
    return funde


def main():
    ap = argparse.ArgumentParser(description='Gebaeude gegen Gelaende und DGM messen (schwebt/versunken)')
    ap.add_argument('karte')
    ap.add_argument('--omsi', default=r'C:\Program Files (x86)\Steam\steamapps\common\OMSI 2')
    ap.add_argument('--json', help='Ergebnis je Gebaeude in diese Datei')
    a = ap.parse_args()
    d = a.karte if os.path.isdir(a.karte) else os.path.join(a.omsi, 'maps', a.karte)
    r = gebaeude_hoehen(d, a.omsi)
    if a.json:
        with open(a.json, 'w', encoding='utf-8') as f:
            json.dump(r, f, ensure_ascii=False, indent=0)
    s = np.array([g['spalt'] for g in r]) if r else np.zeros(0)
    an = np.array([g['angehoben'] for g in r if g['angehoben'] is not None]) if r else np.zeros(0)
    print(json.dumps(dict(gebaeude=len(r), schwebt=int((s > SCHWEBT_M).sum()), schwebt_1m=int((s > 1.0).sum()),
                          versunken=sum(1 for g in r if g['angehoben'] is not None and g['angehoben'] > (
                              VERSUNKEN_M if g['ursache'] == 'strasse' else VERSUNKEN_RASTER_M)),
                          angehoben_05=int((an > 0.5).sum()), angehoben_03=int((an > 0.3).sum()),
                          angehoben_1m=int((an > 1.0).sum()),
                          versunken_strasse=sum(1 for g in r if g['angehoben'] and g['angehoben'] > VERSUNKEN_M
                                                and g['ursache'] == 'strasse')), ensure_ascii=False))


if __name__ == '__main__':
    main()
