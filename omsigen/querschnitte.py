"""Querschnitte aus der Wirklichkeit (Qualitaetsplan S1): je Strasse ein eigener Querschnitt statt der festen Marcel-Liste.

Ableitung je OSM-Weg (in Splinerichtung, links nach rechts):
  Gehweg | Radweg (Hochbord) | Parkstreifen | Radfahrstreifen | Fahrspuren zurueck | Fahrspuren vor | ... gespiegelt
Quellen je Wert, in dieser Reihenfolge: OSM-Tag (`width`, `lanes`, `lanes:forward/backward`, `turn:lanes*`,
`sidewalk*`, `cycleway*`, `parking:*`/`parking:lane:*`, `surface`, `busway*`, `bus:lanes*`, `maxspeed`), Messung aus
LoD2 (Gebaeudeflucht, omsigen.querschnitt_messung, Zwischenspeicher in den Kartendaten) und Regeln nach Strassenklasse
(REGELN, aus den width-Angaben in OSM).
Woher jeder Wert kommt, steht in `herkunft` (tag / messung / regel) und landet je Strassenzug in
maps/<Karte>/querschnitte.json und in netz_attribute.json (Attribut "querschnitt").

Gebaut werden die .sli-Dateien vom Querschnitt-Baukasten des Editors (editor/src/querschnitt.rs) ueber
`omsi-editor --querschnitte <auftrag.json>` (ohne Fenster) - ein Spline-Bauer fuer Editor und Generator. Gleiche
Querschnitte teilen sich eine Datei: Splines\\Aschaffenburg\\Q\\Q_<Spuren>_<Fahrbahn cm>_<Pruefsumme>.sli, daneben die
Bauanleitung .qs.json (im Editor: Querschnitt waehlen -> "bearbeiten"). Fehlt der Editor, nimmt der Generator die alten
Standard-Splines (config.choose_spline) und sagt das.

Spurlagen (fuer Netz, Kreuzungen, netz.json, Belegung) rechnet `info` genau wie der Baukasten: die Achse liegt in der
Mitte der Fahr-/Busspuren (querschnitt.rs `links`)."""
import hashlib
import json
import math
import os
import re
import subprocess
import tempfile

import numpy as np

from .config import KI, KARTENDATEN, choose_spline, is_oneway

ORDNER = 'Q'
PFAD = KI + ORDNER + '\\'
FAHRBAHN_H, HOCHBORD_H = 0.10, 0.25
RASTER = 0.05                 # m: Breiten werden darauf gerundet (gleiche Querschnitte teilen sich eine Datei)
MESSUNGEN = os.path.join(KARTENDATEN, 'querschnitte', 'messungen.json')

# je Strassenklasse: Spurbreite (zweistreifig), Breite einer einstreifigen Einbahnstrasse, Gehwegbreite, Gehweg ohne
# Angabe ('both'/'no'). Fahrbahnbreiten = Median der `width`-Angaben in OSM rund um die Linie 16 (Stand 15.10.2026:
# Wohnstrasse 6,0 m (23 Wege), Sammelstrasse tertiary 7,0 m (16), unclassified 5,0 m (6), verkehrsberuhigt 4,9 m (17),
# einstreifige Einbahn-Wohnstrasse 4,5-5,4 m); ohne Daten nach RASt 06
REGELN = {
    'motorway': (3.75, 4.0, 0.0, 'no'), 'motorway_link': (3.5, 4.0, 0.0, 'no'),
    'trunk': (3.5, 4.0, 2.5, 'no'), 'trunk_link': (3.5, 4.0, 2.5, 'no'),
    'primary': (3.5, 4.0, 2.5, 'both'), 'primary_link': (3.25, 4.0, 2.5, 'no'),
    'secondary': (3.5, 4.0, 2.5, 'both'), 'secondary_link': (3.25, 4.0, 2.5, 'no'),
    'tertiary': (3.5, 4.0, 2.25, 'both'), 'tertiary_link': (3.25, 4.0, 2.25, 'no'),
    'unclassified': (2.5, 4.0, 2.0, 'both'), 'residential': (3.0, 5.0, 2.0, 'both'),
    'living_street': (2.45, 3.5, 0.0, 'no'), 'service': (2.5, 3.25, 0.0, 'no'),
    'bus': (3.25, 3.5, 0.0, 'no'), 'busway': (3.25, 3.5, 0.0, 'no'),
}
REGEL_STANDARD = (2.75, 3.5, 2.0, 'no')
OHNE_MARKIERUNG = {'residential', 'living_street', 'service', 'unclassified', 'bus', 'busway'}
RADFAHRSTREIFEN, SCHUTZSTREIFEN, RADWEG = 1.85, 1.5, 1.6
PARKEN = {'parallel': 2.0, 'diagonal': 4.5, 'perpendicular': 5.0}
SPUR_MIN, SPUR_MAX = 2.25, 4.25
# enge Gassen (Messung: Gebaeude zu nah) duerfen schmaler werden: zwei Richtungen auf 3,5 m wie in Altstadtgassen
SPUR_MIN_GASSE = {'living_street': 1.75, 'service': 1.75, 'residential': 2.0, 'unclassified': 2.0}
BELAG_FAHRBAHN = {'asphalt': 'asphalt', 'paving_stones': 'verbund', 'sett': 'kopfstein', 'cobblestone': 'kopfstein',
                  'unhewn_cobblestone': 'kopfstein', 'concrete': 'beton', 'concrete:plates': 'beton',
                  'concrete:lanes': 'beton'}
BELAG_GEHWEG = {'paving_stones': 'platten', 'asphalt': 'asphalt', 'concrete': 'beton', 'concrete:plates': 'beton',
                'sett': 'kopfstein', 'cobblestone': 'kopfstein', 'paved': 'platten'}
UNTEN = {'fahrspur', 'busspur', 'radfahrstreifen', 'parkstreifen'}
SPUR = {'fahrspur', 'busspur'}


# ---------------------------------------------------------------- Tags lesen

def zahl(v):
    """'5,5 m' / '6.1' / '13 m' -> Meter oder None"""
    if v is None:
        return None
    m = re.match(r'\s*(\d+(?:[.,]\d+)?)\s*(m|cm)?\s*$', str(v))
    if not m:
        return None
    w = float(m.group(1).replace(',', '.'))
    return w / 100.0 if m.group(2) == 'cm' else w


def _int(v):
    try:
        return int(str(v).split(';')[0].strip())
    except (TypeError, ValueError):
        return None


def _seiten(t, schluessel):
    """'sidewalk' / 'cycleway' / 'parking' / 'busway': Wert je Seite (links, rechts) in OSM-Richtung aus <k>,
    <k>:both, <k>:left, <k>:right -> ({'left': wert, 'right': wert}, Tag vorhanden)"""
    out = {'left': None, 'right': None}
    v = t.get(schluessel)
    da = False
    if v is not None:
        da = True
        if v in ('both', 'left', 'right', 'no', 'none', 'separate'):     # sidewalk=both/left/right/no
            out['left'] = 'yes' if v in ('both', 'left') else ('separate' if v == 'separate' else 'no')
            out['right'] = 'yes' if v in ('both', 'right') else ('separate' if v == 'separate' else 'no')
        else:                                                             # cycleway=lane, parking=lane ...
            out['left'] = out['right'] = v
    for s in ('both', 'left', 'right'):
        w = t.get(f'{schluessel}:{s}')
        if w is None:
            continue
        da = True
        for seite in (('left', 'right') if s == 'both' else (s,)):
            out[seite] = w
    return out, da


def _turn(v):
    return [x.strip() for x in str(v).split('|')] if v else None


# ---------------------------------------------------------------- Teile

def teil(art, breite, richtung='vor', belag=None):
    return dict(art=art, breite=round(round(breite / RASTER) * RASTER, 3), richtung=richtung, belag=belag or {
        'fahrspur': 'asphalt', 'busspur': 'asphalt', 'radfahrstreifen': 'rot', 'parkstreifen': 'verbund',
        'gehweg': 'platten', 'radweg': 'rot', 'gruen': 'gras', 'mittelinsel': 'gras'}[art])


def querschnitt(teile, marken=None, versatz=0.0):
    """Querschnitt = dict(teile (links nach rechts), marken (je Grenze), versatz (Mitte der Spuren rechts der Achse))"""
    return dict(teile=teile, marken=list(marken) if marken is not None else ['auto'] * max(0, len(teile) - 1),
                versatz=round(round(versatz / RASTER) * RASTER, 3) if versatz else 0.0)


def spiegeln(q):
    """von der anderen Seite gesehen (Spline umgekehrt)"""
    t = [dict(x, richtung='zurueck' if x['richtung'] == 'vor' else 'vor') if x['art'] in SPUR else dict(x)
         for x in reversed(q['teile'])]
    return querschnitt(t, list(reversed(q['marken'])), -q['versatz'])


def links(q):
    """x des linken Rands wie querschnitt.rs: Achse = Mitte der Fahr-/Busspuren (ohne Spuren: Mitte des Ganzen) plus
    Achsversatz"""
    x, a, b = 0.0, None, None
    for t in q['teile']:
        if t['art'] in SPUR:
            a = x if a is None else a
            b = x + t['breite']
        x += t['breite']
    return (-(a + b) / 2.0 if a is not None else -x / 2.0) + q.get('versatz', 0.0)


def info(q):
    """Spurinfo wie splinedb.parse_sli sie aus der gebauten .sli liest: lanes [(Querlage, Richtung 0 mit/1 gegen)],
    walks, cw, half, cl/cr (Fahrbahnkanten), ol/or_ (Aussenkanten)"""
    x = links(q)
    lanes, walks, unten, alle = [], [], [], []
    for t in q['teile']:
        a, b = x, x + t['breite']
        m = (a + b) / 2.0
        alle += [a, b]
        if t['art'] in UNTEN:
            unten += [a, b]
        if t['art'] in SPUR:
            lanes.append((round(m, 3), 0 if t['richtung'] == 'vor' else 1))
        elif t['art'] == 'gehweg':
            walks.append(round(m, 3))
        x = b
    unten = unten or [-3.0, 3.0]
    # wie parse_sli: Werte aus dem Text (3 Nachkommastellen)
    r = lambda v: round(v, 3)
    return dict(lanes=lanes, walks=walks, cw=r(max(abs(min(unten)), abs(max(unten)))),
                half=r(max(abs(min(alle)), abs(max(alle)))), cl=r(min(unten)), cr=r(max(unten)), ol=r(min(alle)),
                or_=r(max(alle)))


def bauanleitung(q, name):
    """.qs.json wie querschnitt.rs Querschnitt::json (versatz nur, wenn nicht 0)"""
    d = dict(name=name, teile=[dict(art=t['art'], breite=t['breite'], belag=t['belag'], richtung=t['richtung'])
                               for t in q['teile']], marken=list(q['marken']))
    if q.get('versatz'):
        d['versatz'] = q['versatz']
    return d


def schluessel(q):
    return json.dumps([[t['art'], round(t['breite'], 2), t['richtung'], t['belag']] for t in q['teile']]
                      + [list(q['marken']), round(q.get('versatz', 0.0), 2)], separators=(',', ':'))


def datei(q):
    """eindeutiger, gleichbleibender Dateiname (ohne Endung): Q_<zurueck>z<vor>v_<Fahrbahn in cm>_<Pruefsumme>"""
    vor = sum(1 for t in q['teile'] if t['art'] in SPUR and t['richtung'] == 'vor')
    zur = sum(1 for t in q['teile'] if t['art'] in SPUR and t['richtung'] == 'zurueck')
    fb = sum(t['breite'] for t in q['teile'] if t['art'] in UNTEN)
    h = hashlib.sha1(schluessel(q).encode()).hexdigest()[:8]
    return f'Q_{zur}z{vor}v_{int(round(fb * 100))}_{h}'


def gehwege(q):
    """Gehwegbreite links und rechts der Fahrbahn"""
    sp = [i for i, t in enumerate(q['teile']) if t['art'] in SPUR]
    li = sum(t['breite'] for t in q['teile'][:sp[0]] if t['art'] == 'gehweg') if sp else 0.0
    re = sum(t['breite'] for t in q['teile'][sp[-1] + 1:] if t['art'] == 'gehweg') if sp else 0.0
    return round(li, 2), round(re, 2)


GEHWEG_MIN = 1.5          # m: so schmal wird ein Gehweg beim Einpassen hoechstens, danach faellt er weg
SPUR_KOMFORT = 2.75       # m: so schmal werden Spuren, bevor ein Gehweg ganz wegfaellt
RAND_GEBAEUDE = 0.3       # m Abstand zur Gebaeudeflucht


def einpassen(q, flucht_l, flucht_r, herkunft, versatz_max=2.5, spur_min=SPUR_MIN):
    """Querschnitt in den Raum zwischen den Gebaeuden einpassen (Abstaende links/rechts der Achse, None = frei):
    1. Achsversatz, wenn eine Seite zu eng ist und die andere Platz hat (OSM-Linie nicht mittig),
    2. aussen beginnend Seitenraum schmaler/weg (Gehweg bis GEHWEG_MIN, Radweg, Parken), Spuren bis SPUR_KOMFORT,
       dann Gehweg und Radfahrstreifen weg,
    3. zuletzt die Spuren (bis spur_min). -> neuer Querschnitt (herkunft wird ergaenzt)"""
    import copy
    q = copy.deepcopy(q)
    lim = [flucht_l - RAND_GEBAEUDE if flucht_l is not None else None,
           flucht_r - RAND_GEBAEUDE if flucht_r is not None else None]

    def ueber():
        i = info(q)
        return [(-i['ol'] - lim[0]) if lim[0] is not None else -1e9, (i['or_'] - lim[1]) if lim[1] is not None else -1e9]
    u = ueber()
    if max(u) <= 0.01:
        return q
    herkunft['einpassung'] = 'messung'
    # 1. Achsversatz
    if lim[0] is not None and lim[1] is not None:
        if u[0] > 0 > u[1]:
            d = min(u[0], -u[1], versatz_max - q['versatz'])
            q['versatz'] = round(round((q['versatz'] + max(0.0, d)) / RASTER) * RASTER, 3)
        elif u[1] > 0 > u[0]:
            d = min(u[1], -u[0], versatz_max + q['versatz'])
            q['versatz'] = round(round((q['versatz'] - max(0.0, d)) / RASTER) * RASTER, 3)
        if q['versatz']:
            herkunft['versatz'] = 'messung'
        u = ueber()
    # 2. Seitenraum von aussen: erst schmaler und Nebensachen weg, dann die Spuren bis SPUR_KOMFORT, erst danach
    #    faellt der Gehweg (in der Wirklichkeit gibt es ihn fast immer, nur schmal), zuletzt Spuren bis spur_min
    def seitenraum(schritte):
        nonlocal u
        for seite in (0, 1):
            geaendert = False
            for art, wie in schritte:
                if u[seite] <= 0.01:
                    break
                spuren = [i for i, t in enumerate(q['teile']) if t['art'] in SPUR]
                idx = range(0, spuren[0]) if seite == 0 else range(len(q['teile']) - 1, spuren[-1], -1)
                for i in list(idx):
                    t = q['teile'][i]
                    if t['art'] != art or u[seite] <= 0.01:
                        continue
                    geaendert = True
                    if wie == 'schmal':
                        t['breite'] = round(math.floor(max(GEHWEG_MIN, t['breite'] - u[seite]) / RASTER + 1e-6) * RASTER, 3)
                    else:
                        t['breite'] = 0.0
                    u = ueber()
                k = [i for i, t in enumerate(q['teile']) if t['breite'] <= 0.0]
                for i in reversed(k):             # entfernte Teile samt einer Grenzmarkierung
                    del q['teile'][i]
                    if q['marken']:
                        del q['marken'][min(i, len(q['marken']) - 1)]
                u = ueber()
            if geaendert:
                herkunft['gehweg_' + ('links' if seite == 0 else 'rechts')] = 'messung'

    def spuren_schmaler(grenze):
        nonlocal u
        if max(u) <= 0.01:
            return
        spuren = [t for t in q['teile'] if t['art'] in SPUR]
        w = spuren[0]['breite']
        neu = max(min(grenze, w), w - 2.0 * max(u) / len(spuren))
        if neu < w - 1e-9:
            for t in spuren:
                t['breite'] = round(math.floor(neu / RASTER + 1e-6) * RASTER, 3)
            herkunft['fahrbahn'] = 'messung'
            u = ueber()
    seitenraum([('gehweg', 'schmal'), ('radweg', 'weg'), ('gruen', 'weg'), ('parkstreifen', 'weg')])
    spuren_schmaler(max(spur_min, SPUR_KOMFORT))
    seitenraum([('gehweg', 'weg'), ('radfahrstreifen', 'weg'), ('mittelinsel', 'weg')])
    # 3. Spuren
    spuren_schmaler(spur_min)
    return q


# ---------------------------------------------------------------- Ableitung

def ableiten(t, dual=False, walk=(True, True), messung=None):
    """OSM-Tags eines Wegs (+ Messung) -> dict(q (Querschnitt, siehe querschnitt()), force, herkunft, werte).
    force wie config.choose_spline: +1 Spline in OSM-Richtung, -1 umgekehrt, 0 egal (symmetrisch).
    walk: (links, rechts) Gehweg moeglich in Splinerichtung (network: Nachbarfahrbahn zu nah); dual: Richtungsfahrbahn.
    messung: dict(fahrbahn, gehweg_links, gehweg_rechts, flucht_links, flucht_rechts) in OSM-Richtung
    (omsigen.querschnitt_messung; flucht = Abstand der Gebaeude von der Weglinie)"""
    hw = t.get('highway') or ''
    spur_w, einbahn_w, geh_w, geh_standard = REGELN.get(hw, REGEL_STANDARD)
    herkunft, werte = {}, {}
    messung = messung or {}
    einbahn = is_oneway(t)
    umgekehrt = t.get('oneway') == '-1'          # Spline gegen die OSM-Richtung (Einbahn entgegen)

    def seite(s):                                # Seite in Splinerichtung -> Seite in OSM-Richtung
        return {'left': 'right', 'right': 'left'}[s] if umgekehrt else s

    # --- Fahrspuren je Richtung (in Fahrtrichtung von links nach rechts: Liste von Arten)
    lanes = _int(t.get('lanes'))
    lf, lb = _int(t.get('lanes:forward')), _int(t.get('lanes:backward'))
    tl, tf, tb = _turn(t.get('turn:lanes')), _turn(t.get('turn:lanes:forward')), _turn(t.get('turn:lanes:backward'))
    herkunft['spuren'] = 'tag' if (lanes or lf or lb or tl or tf or tb) else 'regel'
    # turn:lanes zaehlt jede Spur einzeln - widerspricht lanes, gilt turn:lanes (Abbiegestreifen sind sonst weg)
    if einbahn:
        n_vor = max(lanes or lf or 1, len(tl or tf or []))
        n_zur = 0                                 # (Gegenbus oneway:bus=no: noch keine eigene Spur)
    else:
        if tf and (lf is None or len(tf) > lf):
            lf = len(tf)
        if tb and (lb is None or len(tb) > lb):
            lb = len(tb)
        if lf and lb:
            n_vor, n_zur = lf, lb
        elif lanes and lf:
            n_vor, n_zur = lf, max(1, lanes - lf)
        elif lanes and lb:
            n_vor, n_zur = max(1, lanes - lb), lb
        elif lanes and lanes >= 2:
            n_vor = n_zur = max(1, lanes // 2)
        else:
            n_vor = n_zur = 1
    n_vor = max(1, min(n_vor, 6))
    n_zur = 0 if einbahn else max(1, min(n_zur, 6))
    # Busspuren (bus:lanes / psv:lanes "...|designated") je Richtung, aussen
    def busse(k):
        v = t.get(f'bus:lanes{k}') or t.get(f'psv:lanes{k}')
        return sum(1 for x in str(v).split('|') if x.strip() == 'designated') if v else 0
    bus_vor = busse(':forward') if not einbahn else busse('')
    bus_zur = busse(':backward') if not einbahn else 0
    bw, _ = _seiten(t, 'busway')
    if bw[seite('right')] == 'lane' and not bus_vor:
        bus_vor = 1; n_vor += 1
    if not einbahn and bw[seite('left')] == 'lane' and not bus_zur:
        bus_zur = 1; n_zur += 1
    if bus_vor or bus_zur:
        herkunft['busspur'] = 'tag'
    werte.update(spuren_vor=n_vor, spuren_zurueck=n_zur)
    werte['abbieger'] = {k: v for k, v in (('vor', tl if einbahn else tf), ('zurueck', None if einbahn else tb)) if v}
    if t.get('maxspeed'):
        werte['maxspeed'] = t['maxspeed']

    # --- Seitenraum je Seite (in Splinerichtung)
    geh, geh_da = _seiten(t, 'sidewalk')
    rad, rad_da = _seiten(t, 'cycleway')
    if einbahn and t.get('cycleway') in ('lane', 'track') and not t.get('cycleway:left') and not t.get('cycleway:both'):
        rad[{'left': 'right', 'right': 'left'}[seite('right')]] = None     # Einbahn: cycleway=lane nur rechts
    park_neu, park_da = _seiten(t, 'parking')
    park_alt, park_alt_da = _seiten(t, 'parking:lane')
    herkunft['gehweg'] = 'tag' if geh_da else 'regel'
    if rad_da:
        herkunft['radweg'] = 'tag'
    if park_da or park_alt_da:
        herkunft['parken'] = 'tag'
    aussen = {}
    for s in ('left', 'right'):
        o = seite(s)
        st = []                                   # von innen (Fahrbahn) nach aussen
        # Radfahrstreifen / Radweg
        rv = rad[o]
        if einbahn and s == 'left' and rv == 'opposite_lane':
            rv = 'lane'
        if rv in ('lane', 'opposite_lane'):
            art_l = t.get(f'cycleway:{o}:lane') or t.get('cycleway:both:lane') or t.get('cycleway:lane')
            st.append(('radfahrstreifen', SCHUTZSTREIFEN if art_l == 'advisory' else RADFAHRSTREIFEN))
        # Parken auf der Fahrbahn
        pv, ort = park_neu[o], None
        if pv in ('lane', 'street_side', 'half_on_kerb', 'on_kerb', 'shoulder'):
            ort = pv
            ausr = t.get(f'parking:{o}:orientation') or t.get('parking:both:orientation') or 'parallel'
        elif park_alt[o] in PARKEN or park_alt[o] == 'marked':
            ausr = park_alt[o] if park_alt[o] in PARKEN else 'parallel'
            ort = {'on_street': 'lane', 'half_on_kerb': 'half_on_kerb', 'on_kerb': 'on_kerb',
                   'street_side': 'street_side', 'painted_area_only': 'lane'}.get(
                t.get(f'parking:lane:{o}:{park_alt[o]}') or t.get(f'parking:lane:both:{park_alt[o]}') or 'on_street', 'lane')
        if ort in ('lane', 'street_side', 'shoulder'):
            st.append(('parkstreifen', PARKEN.get(ausr, 2.0)))
        elif ort == 'half_on_kerb':
            st.append(('parkstreifen', PARKEN.get(ausr, 2.0) / 2))
        # Radweg auf Hochbord
        if rv == 'track':
            st.append(('radweg', RADWEG))
        # Gehweg
        g = geh[o]
        if g is None:
            g = 'yes' if (geh_standard == 'both' or (geh_standard == 'right' and s == 'right')) else 'no'
        if dual and s == 'left' and not (geh_da and geh[o] == 'yes'):
            g = 'no'                              # Mittelstreifen einer Richtungsfahrbahn
        if not walk[0 if s == 'left' else 1]:
            g = 'nah'                             # Nachbarfahrbahn: kein Platz
            st = [x for x in st if x[0] not in ('radweg',)]
        breite_g = None
        if g in ('yes', 'separate'):
            breite_g = zahl(t.get(f'sidewalk:{o}:width')) or zahl(t.get('sidewalk:both:width')) or zahl(t.get('sidewalk:width'))
            q = 'tag' if breite_g else None
            if not breite_g and messung.get(f'gehweg_{"links" if o == "left" else "rechts"}'):
                breite_g, q = messung[f'gehweg_{"links" if o == "left" else "rechts"}'], 'messung'
            if not breite_g:
                breite_g, q = (geh_w or 2.0), 'regel'
            herkunft[f'gehweg_{"links" if s == "left" else "rechts"}'] = q
            sf = t.get(f'sidewalk:{o}:surface') or t.get('sidewalk:both:surface') or t.get('sidewalk:surface')
            st.append(('gehweg', breite_g, BELAG_GEHWEG.get(sf, 'platten')))
        elif g == 'nah' or (dual and s == 'left'):
            st.append(('mittelinsel', 0.5, 'beton'))   # Bordsteinstreifen statt Gehweg (wie AB_einbahn_*_ohne)
        werte[f'gehweg_{"links" if s == "left" else "rechts"}'] = round(breite_g, 2) if breite_g else 0.0
        aussen[s] = st

    # --- Fahrbahnbreite
    fest = sum(x[1] for s in aussen.values() for x in s if x[0] in ('radfahrstreifen', 'parkstreifen'))
    n = n_vor + n_zur
    w_tag = zahl(t.get('width:carriageway')) or zahl(t.get('width')) or zahl(t.get('est_width'))
    if w_tag and w_tag > 30:
        w_tag = None
    w_mess = messung.get('fahrbahn')
    W, q = (w_tag, 'tag') if w_tag else ((w_mess, 'messung') if w_mess else (None, 'regel'))
    herkunft['fahrbahn'] = q
    seitenstreifen = 0.0
    if W:
        spur = (W - fest) / n
        if spur < SPUR_MIN and fest:
            # Breite ohne die Parkstreifen angegeben
            fest_p = sum(x[1] for s in aussen.values() for x in s if x[0] == 'parkstreifen')
            spur = (W - fest + fest_p) / n
        if spur > SPUR_MAX:
            # breite Fahrbahn ohne Angaben zum Parken: Spuren nach Regel, Rest als ungeteilter Seitenstreifen
            regel = max(spur_w, einbahn_w if (einbahn and n == 1) else spur_w)
            seitenstreifen = W - fest - regel * n
            spur = regel
            if seitenstreifen < 0.65:
                spur, seitenstreifen = (W - fest) / n, 0.0
        spur = max(SPUR_MIN, spur)
    else:
        spur = einbahn_w if (einbahn and n == 1) else spur_w
    werte['fahrbahn'] = round(spur * n + fest + seitenstreifen, 2)

    # --- Teile zusammensetzen
    belag = BELAG_FAHRBAHN.get(t.get('surface'), 'asphalt')
    if t.get('surface') in BELAG_FAHRBAHN:
        herkunft['belag'] = 'tag'
    teile, marken = [], []

    def seitenteile(st):
        out = []
        for x in st:
            art, b = x[0], x[1]
            bl = x[2] if len(x) > 2 else None
            out.append(teil(art, b, belag=bl))
        if seitenstreifen > 0.6:
            out.insert(0, teil('parkstreifen', seitenstreifen / 2, belag=belag))
        return out
    lt = seitenteile(aussen['left'])[::-1]
    rt = seitenteile(aussen['right'])
    spuren_zur = [teil('busspur' if i < bus_zur else 'fahrspur', spur, 'zurueck', belag) for i in range(n_zur)]
    spuren_vor = [teil('busspur' if i >= n_vor - bus_vor else 'fahrspur', spur, 'vor', belag) for i in range(n_vor)]
    teile = lt + spuren_zur + spuren_vor + rt
    # Markierungen: Wohn-/Erschliessungsstrassen und schmale Fahrbahnen ohne Mittellinie, sonst nach Regeln
    keine_mitte = hw in OHNE_MARKIERUNG or spur < 2.75 or t.get('lane_markings') == 'no'
    for i in range(len(teile) - 1):
        a, b = teile[i], teile[i + 1]
        m = 'auto'
        if a['art'] in SPUR and b['art'] in SPUR and a['richtung'] != b['richtung'] and keine_mitte:
            m = 'keine'
        if 'parkstreifen' in (a['art'], b['art']) and a['belag'] == b['belag']:
            m = 'keine'                           # ungeteilter Seitenstreifen
        marken.append(m)
    q = querschnitt(teile, marken)
    # Gebaeudeflucht (Messung LoD2): der Strassenraum passt zwischen die Haeuser
    fl, fr = messung.get('flucht_links'), messung.get('flucht_rechts')
    if umgekehrt:
        fl, fr = fr, fl
    if fl is not None or fr is not None:
        q = einpassen(q, fl, fr, herkunft, spur_min=SPUR_MIN_GASSE.get(hw, SPUR_MIN))
        i = info(q)
        werte['fahrbahn'] = round(i['cr'] - i['cl'], 2)
        werte['gehweg_links'], werte['gehweg_rechts'] = gehwege(q)
    if q['versatz']:
        werte['versatz'] = q['versatz']
    # Richtung: Einbahn in Fahrtrichtung; unsymmetrische Querschnitte in einer festen Lage (die kleinere Pruefsumme),
    # damit gleich getaggte, verschieden gezeichnete Wege dieselbe Datei in derselben Richtung bekommen
    if einbahn:
        force = -1 if umgekehrt else +1
    else:
        sp = spiegeln(q)
        if schluessel(sp) == schluessel(q):
            force = 0
        elif schluessel(sp) < schluessel(q):
            q, force = sp, -1
        else:
            force = +1
    return dict(q=q, force=force, herkunft=herkunft, werte=werte)


# ---------------------------------------------------------------- Angleichen und Uebergaenge

AEHNLICH = 0.5            # m: so weit duerfen Breiten benachbarter Querschnitte abweichen, um einer zu werden
UE_LAENGE = (15.0, 30.0)  # m: Laenge eines Uebergangs (uebergang.rs laenge_vorschlag, hier begrenzt)


KNICK_GRAD = 6.0        # Grad: groesserer Knick am Knoten -> dort kein (gerades) Uebergangsobjekt moeglich
KNICK_GRENZE = 99.0     # m: im Knick gleichen auch deutlich verschiedene Breiten an (gleicher Aufbau vorausgesetzt)


def _winkel_am_knoten(e1, x1, e2, x2, weit=10.0):
    """Winkel (Grad) zwischen den beiden Kanten am gemeinsamen Knoten: 180 = gerade durch"""
    def weg(e, x):
        P = e['P'] if x == 0 else e['P'][::-1]
        s, q = 0.0, P[1]
        for a, b in zip(P, P[1:]):
            s += math.dist(a, b)
            q = b
            if s >= weit:
                break
        return math.atan2(q[0] - P[0][0], q[1] - P[0][1])
    d = abs(math.degrees(weg(e1, x1) - weg(e2, x2))) % 360
    return 360 - d if d > 180 else d


def aehnlich(a, b, grenze=AEHNLICH):
    """gleicher Aufbau (Arten, Richtungen), jede Breite und der Achsversatz hoechstens `grenze` verschieden"""
    if len(a['teile']) != len(b['teile']) or abs(a.get('versatz', 0.0) - b.get('versatz', 0.0)) > grenze:
        return False
    return all(x['art'] == y['art'] and x['richtung'] == y['richtung'] and abs(x['breite'] - y['breite']) <= grenze
               for x, y in zip(a['teile'], b['teile']))


def kanonisch(q):
    """True, wenn q die feste Lage seines Spiegelbilds ist (siehe ableiten)"""
    return schluessel(q) <= schluessel(spiegeln(q))


def symmetrisch(q):
    return schluessel(q) == schluessel(spiegeln(q))


def _pfade(q):
    """(Art 0 Fahrzeug / 1 Fussgaenger, Querlage, Richtung 0/1/2) wie uebergang.rs Seite::aus_querschnitt"""
    i = info(q)
    return [(0, x, d) for x, d in i['lanes']] + [(1, x, 2) for x in i['walks']]


def pfade_paaren(a, b):
    """uebergang.rs pfade_paaren: (xa, xb) je gepaartem Pfad"""
    out = []
    for art, richtung in ((0, 0), (0, 1), (1, 2)):
        pa = sorted(x for k, x, r in a if k == art and (art == 1 or r == richtung))
        pb = sorted(x for k, x, r in b if k == art and (art == 1 or r == richtung))
        if not pa or not pb:
            continue
        if art == 1:
            for seite in (-1, 1):
                ga = [x for x in pa if (x > 0 or (x == 0 and seite > 0)) == (seite > 0)]
                gb = [x for x in pb if (x > 0 or (x == 0 and seite > 0)) == (seite > 0)]
                out += list(zip(ga, gb))
            continue
        if richtung == 1:
            pa, pb = pa[::-1], pb[::-1]
        n = min(len(pa), len(pb))
        out += list(zip(pa[:n], pb[:n]))
        out += [(x, pb[n - 1]) for x in pa[n:]] + [(pa[n - 1], x) for x in pb[n:]]
    return out


def uebergang_laenge(qa, qb):
    """Laenge des Uebergangs von qa nach qb (beide in Fahrtrichtung A -> B): je Meter Spurversatz 8 m, mindestens 15 m,
    auf 5 m gerundet (uebergang.rs laenge_vorschlag), hoechstens UE_LAENGE[1]"""
    versatz = max((abs(xb - xa) for xa, xb in pfade_paaren(_pfade(qa), _pfade(qb))), default=0.0)
    ia, ib = info(qa), info(qb)
    breiten = max(abs(ia['ol'] - ib['ol']), abs(ia['or_'] - ib['or_']))
    L = math.ceil(max(max(versatz, breiten * 0.7) * 8.0, UE_LAENGE[0]) / 5.0) * 5.0
    return min(L, UE_LAENGE[1])


# ---------------------------------------------------------------- Editor (Baukasten ohne Fenster)

def editor_exe():
    """omsi-editor.exe: OMSIGEN_EDITOR, sonst der gebaute Editor dieses Repos (release vor debug)"""
    e = os.environ.get('OMSIGEN_EDITOR')
    if e:
        return e if os.path.isfile(e) else None
    basis = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), 'editor', 'target')
    for art in ('release', 'debug'):
        p = os.path.join(basis, art, 'omsi-editor.exe')
        if os.path.isfile(p):
            return p
    return None


def editor_auftrag(auftrag, exe=None, timeout=600):
    """Auftrag an `omsi-editor --querschnitte` -> Antwort (dict)"""
    exe = exe or editor_exe()
    if not exe:
        raise FileNotFoundError('omsi-editor.exe nicht gefunden (editor: cargo build --release, oder OMSIGEN_EDITOR)')
    with tempfile.TemporaryDirectory(prefix='omsigen-qs-') as d:
        pfad = os.path.join(d, 'auftrag.json')
        antwort = os.path.join(d, 'antwort.json')
        with open(pfad, 'w', encoding='utf-8') as f:
            json.dump(dict(auftrag, antwort=antwort), f, ensure_ascii=False)
        p = subprocess.run([exe, '--querschnitte', pfad], capture_output=True, text=True, timeout=timeout)
        if os.path.isfile(antwort):
            with open(antwort, encoding='utf-8') as f:
                return json.load(f)
        raise RuntimeError(f'omsi-editor --querschnitte: Rueckgabe {p.returncode}: {(p.stderr or p.stdout)[-500:]}')


def messungen_laden(pfad=MESSUNGEN):
    try:
        with open(pfad, encoding='utf-8') as f:
            return json.load(f).get('wege', {})
    except (OSError, ValueError):
        return {}


# ---------------------------------------------------------------- je Karte

class Querschnitte:
    """Querschnittswahl fuer einen Generatorlauf. modus 'osm' (S1) oder 'standard' (feste Liste, config.choose_spline).
    `waehle` ist die Spline-Wahl fuer network.build; `bauen` laesst die gebrauchten .sli vom Editor schreiben."""

    def __init__(self, sdb, modus='osm', messungen=None, exe=None, log=print):
        self.sdb, self.log = sdb, log
        self.exe = exe if exe is not None else editor_exe()
        if modus == 'osm' and not self.exe:
            log('    Querschnitte: omsi-editor.exe fehlt (editor: cargo build --release) - nehme die Standard-Splines')
            modus = 'standard'
        self.modus = modus
        self.messungen = messungen if messungen is not None else (messungen_laden() if modus == 'osm' else {})
        self.typen = {}          # rel -> dict(datei, q)
        self.wahl = {}           # (Weg-Schluessel, rel) -> dict(herkunft, werte)
        self.wahl_weg = {}       # Weg-Schluessel -> zuletzt gewaehlt (fuer angeglichene Kanten)
        self.benutzt = set()     # Typen, die die Strassenzuege wirklich nutzen (ketten_kennzeichnen)
        self.kantenprofile = None  # G1: Gebaeudeflucht je Kante (platzierung.Kantenprofile), Fahrbahn einpassen

    @staticmethod
    def weg_schluessel(t):
        return str(t.get('osm_id') or hashlib.sha1(json.dumps(t, sort_keys=True).encode()).hexdigest()[:12])

    def waehle(self, t, dual, walk=(True, True)):
        if self.modus != 'osm':
            return choose_spline(t, dual, walk)
        a = ableiten(t, dual, walk, self.messungen.get(str(t.get('osm_id'))))
        rel = self.registrieren(a['q'])
        self.wahl[(self.weg_schluessel(t), rel)] = dict(herkunft=a['herkunft'], werte=a['werte'])
        self.wahl_weg[self.weg_schluessel(t)] = self.wahl[(self.weg_schluessel(t), rel)]
        return rel, a['force']

    def registrieren(self, q):
        name = datei(q)
        rel = PFAD + name + '.sli'
        if rel not in self.typen:
            self.typen[rel] = dict(datei=name, q=q)
            self.sdb[rel] = info(q)
        return rel

    def herkunft(self, tags, rel):
        k = self.weg_schluessel(tags)
        return self.wahl.get((k, rel)) or self.wahl_weg.get(k)

    # ------------------------------------------------------------ Netz: angleichen, Uebergaenge

    def angleichen(self, edges):
        """network.build: benachbarte Kanten (Knoten mit genau 2 Kanten) mit aehnlichem Querschnitt bekommen denselben -
        den der laengsten Kante der Gruppe -, damit nicht an jeder kleinen Tag-Aenderung ein Uebergang entsteht"""
        if self.modus != 'osm':
            return 0
        if self.kantenprofile is not None:            # G1: erst je Kante die Fahrbahn an den Gebaeuden einpassen
            try:
                self.kantenprofile.einpassen(self, edges)
            except Exception as ex:                   # ohne Profile geht es mit den Querschnitten je Weg weiter
                self.log(f'    Querschnitte: Einpassen je Kante nicht moeglich ({type(ex).__name__}: {ex})')
                self.kantenprofile = None
        from .network import key, plen
        at = {}
        for e in edges:
            at.setdefault(key(e['P'][0]), []).append((e, 0))
            at.setdefault(key(e['P'][-1]), []).append((e, -1))
        nachbarn = {}
        for lst in at.values():
            if len(lst) != 2:
                continue
            (e1, x1), (e2, x2) = lst
            if e1 is e2 or e1.get('ring') or e2.get('ring') or e1['spl'] not in self.typen or e2['spl'] not in self.typen:
                continue
            gleich = x1 != x2                          # e1 -> e2 in derselben Richtung
            # Knick am Knoten: Uebergangsobjekte gibt es nur gerade - in der Kurve gleicht jeder gleich aufgebaute
            # Nachbar an (die Vorbilder wechseln den Querschnitt nur auf Geraden, docs/musterbuch.md 4)
            knick = 180.0 - _winkel_am_knoten(e1, x1, e2, x2) > KNICK_GRAD
            nachbarn.setdefault(id(e1), []).append((e2, gleich, knick))
            nachbarn.setdefault(id(e2), []).append((e1, gleich, knick))
        q_von = {id(e): self.typen[e['spl']]['q'] for e in edges if e['spl'] in self.typen}
        fertig, n = set(), 0
        for start in sorted((e for e in edges if id(e) in nachbarn), key=lambda e: -plen(e['P'])):
            if id(start) in fertig:
                continue
            fertig.add(id(start))
            ziel = q_von[id(start)]
            stapel = [(start, ziel)]                   # (Kante, Ziel-Querschnitt in Richtung ihrer Linie P)
            while stapel:
                e, qz = stapel.pop()
                for o, gleich, knick in nachbarn.get(id(e), []):
                    if id(o) in fertig:
                        continue
                    qo = qz if gleich else spiegeln(qz)          # Ziel in Richtung von o's Linie
                    if not aehnlich(qo, q_von[id(o)], grenze=KNICK_GRENZE if knick else AEHNLICH):
                        continue
                    if self.kantenprofile is not None and not self.kantenprofile.frei(o, qo, q_von[id(o)]):
                        continue                               # G1: angeglichen laege die Fahrbahn im Gebaeude
                    einbahn = is_oneway(o['t'])
                    umdrehen = not einbahn and not kanonisch(qo)
                    if einbahn and qo['teile'] and any(t['richtung'] == 'zurueck' for t in qo['teile'] if t['art'] in SPUR):
                        continue
                    fertig.add(id(o))
                    q_neu = spiegeln(qo) if umdrehen else qo
                    rel = self.registrieren(q_neu)
                    if rel != o['spl']:
                        n += 1
                        alt = self.wahl_weg.get(self.weg_schluessel(o['t']))
                        if alt:
                            alt['herkunft']['angeglichen'] = os.path.basename(rel)
                    o['spl'] = rel
                    if umdrehen:
                        o['P'] = o['P'][::-1]
                        o['osm_rev'] = not o.get('osm_rev')
                    o['force'] = einbahn or not symmetrisch(q_neu)
                    q_von[id(o)] = q_neu
                    stapel.append((o, q_neu))
        return n

    def uebergang(self, lst):
        """network.build: Laenge des Uebergangs an einem Knoten mit zwei Kanten [(Kante, Ende), ...] oder None"""
        if self.modus != 'osm':
            return None
        (e1, x1), (e2, x2) = lst
        if e1['spl'] not in self.typen or e2['spl'] not in self.typen:
            return None
        qa = self.typen[e1['spl']]['q']
        qa = qa if x1 == -1 else spiegeln(qa)       # A = e1, Fahrtrichtung zum Knoten hin
        qb = self.typen[e2['spl']]['q']
        qb = qb if x2 == 0 else spiegeln(qb)        # B = e2, vom Knoten weg
        return uebergang_laenge(qa, qb)

    def uebergaenge_planen(self, net, karte):
        """Knoten mit zwei Armen, deren Enden gerade gegenueber liegen (Abstand = Uebergangslaenge, gleiche Richtung):
        dort ein Uebergangsobjekt (uebergang.rs) statt einer Kreuzungsplatte -> Liste Objekte (wie kreuzung.build_objects)
        und net['uebergang_knoten']"""
        out = []
        net['uebergang_knoten'] = {}
        if self.modus != 'osm':
            return out
        from .geom import dvec, heading
        for k, arms in sorted(net.get('arms', {}).items()):
            if len(arms) != 2 or any(a['spl'] not in self.typen for a in arms):
                continue
            a, b = arms
            ha = (a['h'] + 180.0) % 360.0                # Fahrtrichtung A -> B
            d = math.dist(a['pos'], b['pos'])
            if d < 8.0:
                continue
            hab = heading(a['pos'], b['pos'])
            winkel = lambda u, v: abs((u - v + 180.0) % 360.0 - 180.0)
            # die Spurenden passen nur, wenn beide Arme auf einer Geraden liegen (Pruefung: 5 cm, 1 Grad)
            if winkel(hab, ha) > 0.3 or winkel(b['h'], ha) > 0.3:
                continue                                 # sonst Kreuzungsplatte wie bisher
            ha = hab
            dv = dvec(ha)
            qa = self.typen[a['spl']]['q']
            qa = qa if not a['away'] else spiegeln(qa)
            qb = self.typen[b['spl']]['q']
            qb = qb if b['away'] else spiegeln(qb)
            name = f'UE_{len(out) + 1:03d}'
            flaeche = []
            ia, ib = info(qa), info(qb)
            for s in np.arange(0.0, d + 0.01, 2.0):
                for q in np.arange(min(ia['ol'], ib['ol']), max(ia['or_'], ib['or_']) + 0.01, 2.0):
                    flaeche.append((a['pos'][0] + dv[0] * s + dv[1] * q, a['pos'][1] + dv[1] * s - dv[0] * q))
            o = dict(name=name, origin=tuple(a['pos']), rot=ha, knoten=k, laenge=round(d, 3), flaeche=flaeche,
                     rel=f'Sceneryobjects\\Aschaffenburg\\{karte}\\{name}.sco', rules=[], signale=[], vorfahrt=None,
                     faces=0, paths=0, uebergang=dict(a=qa, b=qb, sli_a=a['spl'], sli_b=b['spl']))
            out.append(o)
            net['uebergang_knoten'][k] = name
        return out

    def uebergaenge_bauen(self, root, kdir, objekte, log=None):
        """die geplanten Uebergangsobjekte vom Editor bauen lassen (nach kreuzung.install_objects) -> Anzahl"""
        log = log or self.log
        ue = [o for o in objekte if o.get('uebergang')]
        if not ue:
            return 0
        auftrag = dict(root=os.path.abspath(root), uebergaenge=[
            dict(ordner=os.path.abspath(kdir), name=o['name'], titel=f"Uebergang {os.path.basename(o['uebergang']['sli_a'])[:-4]}"
                 f" -> {os.path.basename(o['uebergang']['sli_b'])[:-4]}",
                 a=dict(querschnitt=bauanleitung(o['uebergang']['a'], 'a')),
                 b=dict(querschnitt=bauanleitung(o["uebergang"]["b"], "b")), laenge=o["laenge"], steigung=o.get("steigung", 0.0))
            for o in ue])
        a = editor_auftrag(auftrag, self.exe)
        ok = 0
        for o, r in zip(ue, a.get('uebergaenge', [])):
            if r:
                ok += 1
                o['faces'], o['paths'] = r.get('dreiecke', 0), r.get('pfade', 0)
        for f in a.get('fehler', [])[:5]:
            log(f'      Fehler: {f}')
        if ok < len(ue):
            raise RuntimeError(f'Uebergaenge: nur {ok} von {len(ue)} gebaut ({(a.get("fehler") or [""])[0]})')
        log(f'    Querschnitte: {ok} Uebergaenge zwischen Querschnitten (uebergang.rs) statt Kreuzungsplatten')
        return ok

    def bauen(self, root, omsi=None, log=None):
        """die gebrauchten Querschnitte nach <root>\\Splines\\Aschaffenburg\\Q schreiben (Editor) -> Anzahl"""
        log = log or self.log
        if self.modus != 'osm' or not self.typen:
            return 0
        reihe = sorted(r for r in self.typen if not self.benutzt or r in self.benutzt)   # angeglichene fallen weg
        auftrag = dict(root=os.path.abspath(root), ordner=ORDNER,
                       querschnitte=[dict(datei=self.typen[r]['datei'],
                                          querschnitt=bauanleitung(self.typen[r]['q'], self.typen[r]['datei']))
                                     for r in reihe])
        if omsi:
            auftrag['texturen_aus'] = os.path.abspath(omsi)
        a = editor_auftrag(auftrag, self.exe)
        ok = sum(1 for s in a.get('splines', []) if s)
        for f in a.get('fehler', [])[:5]:
            log(f'      Fehler: {f}')
        if a.get('warnungen'):
            log(f'      {len(a["warnungen"])} Warnungen, z. B. {a["warnungen"][0]}')
        if ok < len(reihe):
            raise RuntimeError(f'Querschnitte: nur {ok} von {len(reihe)} .sli gebaut ({a.get("fehler", [""])[0]})')
        quellen = {}
        for w in self.wahl.values():
            q = w['herkunft'].get('fahrbahn')
            quellen[q] = quellen.get(q, 0) + 1
        log(f'    Querschnitte: {ok} Typen in {PFAD} (Editor-Baukasten); Fahrbahnbreite je Weg aus '
            + ', '.join(f'{k} {v}' for k, v in sorted(quellen.items())))
        return ok

    def ketten_kennzeichnen(self, ketten, net=None):
        """je Strassenzug c['querschnitt'] = dict(sli, herkunft, werte, osm) (fuer netz_attribute.json)"""
        self.benutzt = {el[5] for c in ketten for el in c.get('els', ()) if el[5] in self.typen}
        for c in ketten:
            if c.get('hw') in ('junction', 'invisible') or not c.get('els') or not c.get('tags'):
                continue
            rel = c['els'][0][5]
            h = self.herkunft(c['tags'], rel) if self.modus == 'osm' else None
            c['querschnitt'] = dict(sli=rel, modus=self.modus, osm=c['tags'].get('osm_id'),
                                    **(h or dict(herkunft={'alles': 'standard'}, werte={})))

    def datei_schreiben(self, kartenordner, ketten, ketten_ids):
        """maps/<Karte>/querschnitte.json: Typen (Bauanleitung) und je Strassenzug Spline-IDs, Datei, Herkunft"""
        kanten = []
        for c, ids in zip(ketten, ketten_ids):
            q = c.get('querschnitt')
            if not q or not ids:
                continue
            kanten.append(dict(splines=[ids[0], ids[-1]], name=c.get('name', ''), highway=c.get('hw'), **q))
        zaehler = {}
        for k in kanten:
            for feld, quelle in k.get('herkunft', {}).items():
                zaehler.setdefault(feld, {}).setdefault(quelle, 0)
                zaehler[feld][quelle] += 1
        d = dict(format='omsigen-querschnitte', version=1, modus=self.modus, ordner=PFAD,
                 typen={r: dict(datei=v['datei'], querschnitt=bauanleitung(v['q'], v['datei']),
                                fahrbahn=round(sum(t['breite'] for t in v['q']['teile'] if t['art'] in UNTEN), 2),
                                breite=round(sum(t['breite'] for t in v['q']['teile']), 2))
                        for r, v in sorted(self.typen.items()) if not self.benutzt or r in self.benutzt},
                 herkunft=zaehler, kanten=kanten)
        with open(os.path.join(kartenordner, 'querschnitte.json'), 'w', encoding='utf-8') as f:
            json.dump(d, f, ensure_ascii=False, indent=1)
        return d


def vorbereiten(projekt, proj, sdb, modus='osm', messen=False, log=print):
    """Querschnittswahl fuer einen Lauf (pipeline.berechne); messen: Gebaeudeflucht (LoD2) der Strassen vorher messen"""
    if modus == 'osm' and messen:
        from . import querschnitt_messung
        try:
            querschnitt_messung.ergaenzen(projekt, proj, log=log)
        except Exception as e:                    # ohne Messung geht es mit Tags und Regeln weiter
            log(f'    Querschnitte: Messung nicht moeglich ({type(e).__name__}: {e})')
    qs = Querschnitte(sdb, modus, log=log)
    if qs.modus == 'osm':                         # G1: Fahrbahn je Kante gegen die Gebaeudeflucht (LoD2)
        from . import platzierung
        qs.kantenprofile = platzierung.Kantenprofile.fuer(projekt, proj, log)
    return qs
