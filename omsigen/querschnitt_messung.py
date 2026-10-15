"""Messungen fuer die Querschnitte (Qualitaetsplan S1): was OSM nicht sagt, aus amtlichen Messdaten.

Gebaeudeflucht (LoD2, Bayerische Vermessungsverwaltung, CC BY 4.0): je OSM-Weg alle 4 m quer zur Weglinie der Abstand
bis zum naechsten Gebaeude (Grundriss von oben, mit Dachueberstand) links und rechts; je Seite das 25-%-Quantil der
Treffer. Strassenraum (Fahrbahn + Seitenraum) kann nicht breiter sein als der Raum zwischen den Haeusern - das nutzt
querschnitte.ableiten (Querschnitt einpassen, Achsversatz, wenn die OSM-Linie nicht mittig liegt).

Eine automatische Messung der Bordkanten im Luftbild (DOP20) wurde erprobt und verworfen: Schatten, Baumkronen und
parkende Autos machen sie unzuverlaessig (an den 20 Pruefstellen nur 3 auf 0,5 m genau). Das Luftbild dient deshalb nur
der Abnahme (omsigen.querschnitt_pruefung, abgelesene Pruefstellen).

Zwischenspeicher je OSM-Weg in den Kartendaten (querschnitte/messungen.json, mit Pruefsumme der Weggeometrie), damit
ein neuer Lauf nicht wieder alle Gebaeude lesen muss:
  {"format": "omsigen-querschnitt-messungen", "version": 1,
   "wege": {"<osm_id>": {"geo": Pruefsumme, "flucht_links": m, "flucht_rechts": m, "stationen": n, "quelle": "LoD2"}}}
Seiten in OSM-Richtung des Wegs."""
import hashlib
import json
import math
import os

import numpy as np

from .config import KARTENDATEN

DATEI = os.path.join(KARTENDATEN, 'querschnitte', 'messungen.json')
FORMAT, VERSION = 'omsigen-querschnitt-messungen', 2
SCHRITT, ENDE_FREI, REICHWEITE = 4.0, 8.0, 30.0
QUANTIL, MIN_ANTEIL = 25, 0.25
KLASSEN = {'primary', 'secondary', 'tertiary', 'unclassified', 'residential', 'living_street', 'service', 'trunk',
           'primary_link', 'secondary_link', 'tertiary_link', 'trunk_link', 'bus', 'busway'}


def laden(pfad=DATEI):
    try:
        with open(pfad, encoding='utf-8') as f:
            d = json.load(f)
        return d if d.get('version') == VERSION else dict(format=FORMAT, version=VERSION, wege={})
    except (OSError, ValueError):
        return dict(format=FORMAT, version=VERSION, wege={})


def speichern(d, pfad=DATEI):
    os.makedirs(os.path.dirname(pfad), exist_ok=True)
    with open(pfad + '.tmp', 'w', encoding='utf-8') as f:
        json.dump(d, f, ensure_ascii=False, indent=0)
    os.replace(pfad + '.tmp', pfad)


def pruefsumme(P, e0, n0):
    """Weggeometrie in UTM (0,5 m genau) -> kurze Pruefsumme"""
    s = ';'.join(f'{round((x + e0) * 2)},{round((z + n0) * 2)}' for x, z in P)
    return hashlib.sha1(s.encode()).hexdigest()[:12]


# ---------------------------------------------------------------- Gebaeude

def grundrisse(bez, bereich, log=print):
    """LoD2-Gebaeude im Ausschnitt (Kartenmeter x0, y0, x1, y1) -> Liste shapely-Polygone (Ansicht von oben)"""
    from shapely.geometry import Polygon
    from shapely.ops import unary_union
    from . import lod2
    out = []
    for e, n in lod2.kacheln(bez, bereich):
        pfad = lod2.datei(e, n)
        if not os.path.isfile(pfad):
            log(f'    Querschnitte: LoD2-Kachel {e}_{n} laden ...')
            try:
                lod2.laden(e, n)
            except Exception as ex:
                log(f'    Querschnitte: LoD2-Kachel {e}_{n} nicht ladbar ({type(ex).__name__})')
                continue
        x0, y0, x1, y1 = bereich
        for g in lod2.gebaeude(pfad, bez, (x0 - 40, y0 - 40, x1 + 40, y1 + 40)):
            teile = []
            for art, ringe in g['flaechen']:
                if art not in ('RoofSurface', 'GroundSurface'):
                    continue
                for r in ringe[:1]:
                    if len(r) >= 3:
                        p = Polygon(r[:, :2])
                        if p.is_valid and p.area > 0.05:
                            teile.append(p)
                        elif p.area > 0.05:
                            teile.append(p.buffer(0))
            if teile:
                u = unary_union(teile)
                if not u.is_empty:
                    out.append(u)
    return out


def flucht(P, baum, polys, schritt=None, ende_frei=None):
    """Weglinie P [(x, z)] -> (Abstaende links, rechts je Station: Liste, None = kein Gebaeude in Reichweite);
    Stationen alle `schritt` m ohne `ende_frei` m an beiden Enden (Standard SCHRITT, ENDE_FREI)"""
    from shapely.geometry import LineString, Point
    schritt = SCHRITT if schritt is None else schritt
    ende_frei = ENDE_FREI if ende_frei is None else ende_frei
    L = [0.0]
    for a, b in zip(P, P[1:]):
        L.append(L[-1] + math.dist(a, b))
    gesamt = L[-1]
    if gesamt < 2.0:
        return [], []
    if gesamt < 2 * ende_frei + schritt:
        stationen = [gesamt / 2]
    else:
        stationen = list(np.arange(ende_frei, gesamt - ende_frei + 1e-6, schritt))
    li, re = [], []
    j = 0
    for s in stationen:
        while j < len(P) - 2 and L[j + 1] < s:
            j += 1
        a, b = P[j], P[j + 1]
        d = max(L[j + 1] - L[j], 1e-9)
        f = (s - L[j]) / d
        p = (a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f)
        ux, uz = (b[0] - a[0]) / d, (b[1] - a[1]) / d
        rechts = (uz, -ux)                         # rechts der Fahrtrichtung (x Ost, z Nord)
        for seite, liste in ((-1, li), (1, re)):
            q = (p[0] + seite * rechts[0] * REICHWEITE, p[1] + seite * rechts[1] * REICHWEITE)
            strahl = LineString([p, q])
            best = None
            for i in baum.query(strahl):
                g = polys[i]
                if g.contains(Point(p)):
                    best = 0.0
                    break
                x = strahl.intersection(g)
                if x.is_empty:
                    continue
                dd = Point(p).distance(x)
                best = dd if best is None else min(best, dd)
            liste.append(best)
    return li, re


def seite_auswerten(werte):
    treffer = [v for v in werte if v is not None]
    if not werte or len(treffer) < MIN_ANTEIL * len(werte) or not treffer:
        return None
    return round(float(np.percentile(treffer, QUANTIL)), 2)


def ergaenzen(projekt, proj, log=print, pfad=DATEI):
    """fehlende Messungen der Projektstrassen ergaenzen (Gebaeudeflucht aus LoD2) -> Anzahl neu gemessener Wege"""
    if getattr(proj, 'art', '') != 'utm' and not hasattr(proj, 'e0'):
        log('    Querschnitte: Messung nur fuer UTM-Projekte')
        return 0
    d = laden(pfad)
    wege = d['wege']
    stuecke = {}                                  # ein OSM-Weg kann im Projekt in mehreren Stuecken liegen
    for s in projekt.get('strassen', []):
        t = s.get('tags', {})
        oid = t.get('osm_id')
        if not oid or t.get('highway') not in KLASSEN or len(s['punkte']) < 2:
            continue
        stuecke.setdefault(str(oid), []).append([tuple(q) for q in s['punkte']])
    offen = []
    for oid, Ps in stuecke.items():
        ps = hashlib.sha1('|'.join(sorted(pruefsumme(P, proj.e0, proj.n0) for P in Ps)).encode()).hexdigest()[:12]
        alt = wege.get(oid)
        if alt and alt.get('geo') == ps:
            continue
        offen.append((oid, Ps, ps))
    if not offen:
        log(f'    Querschnitte: Messungen aus den Kartendaten ({len(wege)} Wege)')
        return 0
    from shapely import STRtree
    xs = [p[0] for _, Ps, _ in offen for P in Ps for p in P]
    zs = [p[1] for _, Ps, _ in offen for P in Ps for p in P]
    bez = dict(ost0=proj.e0, nord0=proj.n0, nn0=0.0)
    bereich = (min(xs) - REICHWEITE, min(zs) - REICHWEITE, max(xs) + REICHWEITE, max(zs) + REICHWEITE)
    log(f'    Querschnitte: Gebaeudeflucht (LoD2) fuer {len(offen)} Wege messen ...')
    polys = grundrisse(bez, bereich, log)
    if not polys:
        log('    Querschnitte: keine LoD2-Gebaeude im Bereich - keine Messung')
        return 0
    baum = STRtree(polys)
    for oid, Ps, ps in offen:
        li, re = [], []
        for P in Ps:
            a, b = flucht(P, baum, polys)
            li += a
            re += b
        wege[oid] = dict(geo=ps, flucht_links=seite_auswerten(li), flucht_rechts=seite_auswerten(re),
                         stationen=len(li), quelle='LoD2')
    speichern(d, pfad)
    log(f'    Querschnitte: {len(offen)} Wege gemessen ({len(polys)} Gebaeude), gespeichert in {pfad}')
    return len(offen)
