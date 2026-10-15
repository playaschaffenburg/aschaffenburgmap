"""Strassenausstattung (Qualitaetsplan G3) und Haltestellen (G4) als Schicht "objekte" (F2).

Was gebaut wird (Quelle je Objekt im Paket: "osm" oder "regel"):
  - Laternen: OSM highway=street_lamp, sonst Abstand und Seite nach Strassenklasse (Katalog, regeln.laternen);
    auf dem Seitenraum (Belegung gehweg, dahinter frei/gruen), nicht an Kreuzungen, Zufahrten, Querungen
  - Verkehrszeichen: OSM traffic_sign und highway=stop/give_way; Vorfahrtzeichen passend zur Vorfahrt der Karte
    (Regeln [rule] priority an den Kreuzungsobjekten: 64 = wartepflichtiger Arm, 192 = Hauptstrasse) - ein OSM-Schild,
    das der Vorfahrt widerspricht, wird nicht gesetzt, sondern gemeldet; Tempowechsel aus maxspeed der Kanten
  - Poller, Zaeune, Baenke, Muelleimer aus OSM (barrier=bollard|fence, amenity=bench|waste_basket)
  - Haltestellen (omsigen.haltestellen): Mast mit Namensschild, Wartehalle, Bank, Muelleimer, erhoehter Bord
Platziert wird nur ueber Kante + Station + Seite (netz.json, F3) auf erlaubten Klassen der Belegung (F4), mit
Abstaenden zu Bord, Gebaeuden, Kreuzungen und anderen Objekten (Katalog regeln.abstaende).

Objekte stehen im Kachelordner der Schicht: Sceneryobjects/Aschaffenburg/<Karte>/Objekte/<kx>_<ky>/ - eigene Objekte
(Haltestellenmast) liegen dort, installierte werden ueber einen Pfad mit Kennung angesprochen:
Sceneryobjects\\Aschaffenburg\\<Karte>\\Objekte\\<kx>_<ky>\\..\\..\\..\\..\\Streetobjects_MC\\x.sco (Windows loest ".." auf;
die Kennung \\Objekte\\<kx>_<ky>\\ macht das Objekt zur Schicht, erneutes Einsetzen ersetzt nur sie). Je Kachel ein Paket
objekte.json (omsigen.schichten), Verzeichnis schichten.json; veraltet, sobald Netz, Gelaende oder Gebaeude der Kachel
sich aendern. Uebersicht maps/<Karte>/ausstattung.json (Zahlen, Haltestellen, Konflikte).

  python -m omsigen.ausstattung <Karte> [--kacheln kx_ky ...] [--omsi <OMSI>] [--nur-pakete] [--ohne-osm]
"""
import argparse
import collections
import json
import math
import os
import struct
import sys

from . import schichten

BS = chr(92)
KACHEL = 300.0
KATALOG = os.path.join(os.path.dirname(os.path.abspath(__file__)), 'ausstattung_katalog.json')
DATEI = 'ausstattung.json'
PAKET = 'objekte.json'
FORMAT, VERSION = 'omsigen-ausstattung', 1
SEITENRAUM = ('gehweg', 'frei', 'gruen')         # erlaubte Belegung fuer Objekte am Strassenrand
VERBOTEN = ('fahrbahn', 'kreuzung', 'gebaeude', 'wasser', 'bauwerk')
RANG = {'trunk': 7, 'primary': 6, 'secondary': 5, 'trunk_link': 4, 'primary_link': 4, 'secondary_link': 4,
        'tertiary': 4, 'unclassified': 3, 'residential': 2, 'living_street': 1, 'service': 1, 'bus': 1}
VORFAHRT_CODES = {'205': 'vz_205', '206': 'vz_206', '301': 'vz_301', '306': 'vz_306'}


# ---------------------------------------------------------------- Katalog

def katalog_laden(pfad=None):
    with open(pfad or KATALOG, encoding='utf-8') as f:
        return json.load(f)


class Auswahl:
    """je Typ der erste Kandidat des Katalogs, den es in dieser Installation gibt (eigene Objekte immer)"""

    def __init__(self, katalog, omsi):
        from .ausstattung_modelle import masse
        self.katalog, self.omsi = katalog, omsi
        self.wahl, self.masse = {}, {}
        for typ, d in katalog['typen'].items():
            for k in d['kandidaten']:
                if 'eigen' in k:
                    self.wahl[typ] = dict(k)
                    break
                if omsi and os.path.isfile(os.path.join(omsi, *k['rel'].split(BS))):
                    m = masse(omsi, k['rel'])
                    if m.get('busstop'):                 # nie: macht eine zusaetzliche Haltestelle
                        continue
                    self.wahl[typ] = dict(k)
                    self.masse[typ] = m
                    break

    def __contains__(self, typ):
        return typ in self.wahl

    def __getitem__(self, typ):
        return self.wahl[typ]

    def radius(self, typ):
        k = self.wahl.get(typ, {})
        if 'radius' in k:
            return float(k['radius'])
        m = self.masse.get(typ)
        if m and m.get('box'):
            b = m['box']
            return round(max(abs(b[0]), abs(b[3]), abs(b[2]), abs(b[5])), 2)
        return 0.2

    def anhaenge(self, typ):
        """Hoehen der Anhaengepunkte (Mast), oben zuerst"""
        k = self.wahl.get(typ, {})
        m = self.masse.get(typ) or {}
        return sorted(k.get('anhaenge') or m.get('anhaenge_hoehen') or [2.55, 1.92], reverse=True)

    def addons(self, typen):
        out = []
        for t in typen:
            rel = self.wahl.get(t, {}).get('rel', '')
            for pfx, name in self.katalog.get('addons', {}).items():
                if rel.lower().startswith(pfx.lower()) and name not in out:
                    out.append(name)
        return out


def rel_fuer(karte, k, ziel):
    """Objektpfad mit Schicht-Kennung: eigene Objekte im Kachelordner direkt, installierte ueber ..\\ (siehe oben)"""
    basis = BS.join(['Sceneryobjects', 'Aschaffenburg', karte, 'Objekte', f'{k[0]}_{k[1]}']) + BS
    if ziel.lower().startswith(basis.lower()):
        return ziel
    teile = ziel.split(BS)
    if teile[0].lower() != 'sceneryobjects':
        raise ValueError(f'Objekt ausserhalb von Sceneryobjects: {ziel}')
    return basis + '..' + BS + '..' + BS + '..' + BS + '..' + BS + BS.join(teile[1:])


def ziel_von(rel):
    """Objektpfad (auch mit ..\\) -> normaler Pfad ab OMSI-Ordner"""
    out = []
    for t in rel.split(BS):
        if t == '..':
            if out:
                out.pop()
        elif t and t != '.':
            out.append(t)
    return BS.join(out)


# ---------------------------------------------------------------- Gelaende der Karte

class Gelaende:
    """Hoehe des Kartengelaendes (tile_x_z.map.terrain, 61 x 61 im 5-m-Raster, Zeilen von Sueden), bilinear"""

    def __init__(self, kartenordner):
        self.ordner = kartenordner
        self.cache = {}

    def _raster(self, k):
        if k not in self.cache:
            p = os.path.join(self.ordner, f'tile_{k[0]}_{k[1]}.map.terrain')
            r = None
            if os.path.isfile(p):
                b = open(p, 'rb').read()
                n = struct.unpack_from('<i', b, 0)[0]
                if len(b) >= 4 + (n + 1) ** 2 * 4:
                    r = (n, struct.unpack_from(f'<{(n + 1) ** 2}f', b, 4))
            self.cache[k] = r
        return self.cache[k]

    def hoehe(self, x, z):
        k = (math.floor(x / KACHEL), math.floor(z / KACHEL))
        r = self._raster(k)
        if r is None:
            return None
        n, h = r
        s = KACHEL / n
        fx, fz = (x - k[0] * KACHEL) / s, (z - k[1] * KACHEL) / s
        ix, iz = min(n - 1, max(0, int(fx))), min(n - 1, max(0, int(fz)))
        tx, tz = fx - ix, fz - iz
        g = lambda a, b: h[b * (n + 1) + a]
        return ((g(ix, iz) * (1 - tx) + g(ix + 1, iz) * tx) * (1 - tz) +
                (g(ix, iz + 1) * (1 - tx) + g(ix + 1, iz + 1) * tx) * tz)


# ---------------------------------------------------------------- Netz-Hilfen

def seitenraum(e, s):
    """(Bordkante, Aussenkante) als Abstand von der Achse auf Seite s (+1 rechts, -1 links)"""
    q = e.get('querschnitt') or {}
    (cl, cr), (ol, orr) = q.get('fahrbahn', [-3, 3]), q.get('aussen', [-3, 3])
    return (cr, orr) if s > 0 else (-cl, -ol)


def hat_fahrtrichtung(e, mit):
    """gibt es eine Spur mit (mit=True) bzw. gegen die Kantenrichtung?"""
    sp = (e.get('querschnitt') or {}).get('spuren') or []
    return any(r in ((0, 2) if mit else (1, 2)) for _, r in sp) if sp else True


def tags(e):
    return (e.get('attribute') or {}).get('tags') or {}


def klasse_von(e):
    return (e.get('attribute') or {}).get('highway') or tags(e).get('highway') or ''


def winkel(a, b):
    return abs((a - b + 180.0) % 360.0 - 180.0)


def tempo(e):
    v = str(tags(e).get('maxspeed', '')).strip()
    return int(v) if v.isdigit() else None


def zone30(e):
    t = tags(e)
    return 'DE:30' in str(t.get('zone:maxspeed', '')) or 'zone30' in str(t.get('zone:traffic', '')).lower()


# ---------------------------------------------------------------- Kreuzungen der Karte (Vorfahrt aus den Regeln)

def _sco_pfade(pfad):
    """[path]-Starts eines Kreuzungsobjekts -> [(x, y_vor, richtung)] in Objektkoordinaten; Ampel ja/nein"""
    try:
        z = [l.strip() for l in open(pfad, encoding='cp1252', errors='replace')]
    except OSError:
        return [], False
    out = []
    for i, l in enumerate(z):
        if l == '[path]' and i + 4 < len(z):
            try:
                out.append((float(z[i + 1]), float(z[i + 2]), float(z[i + 4])))
            except ValueError:
                out.append(None)
    return out, any(l in ('[traffic_light]', '[use_traffic_light]') for l in z)


def kreuzungsrollen(kartenordner, omsi, netz):
    """Vorfahrt je Arm aus der Karte selbst -> {knoten_id: dict(arme={kante: haupt|neben|gleich}, ampel, x, z)}.
    Ein Arm ist neben, wenn ein Pfad von ihm priority 64 hat, haupt bei 192; ohne Regeln: gleich (rechts vor links)."""
    objekte = {}
    for k in schichten.kacheln(kartenordner):
        _, bl = schichten.bloecke(schichten.lesen(kartenordner, k))
        letzt = None
        for b in bl:
            kopf = b[0].strip()
            if kopf == '[object]':
                rel = schichten.objekt_pfad(b)
                letzt = None
                if os.path.basename(rel.replace(BS, '/')).upper().startswith('K_'):
                    try:
                        letzt = dict(id=int(float(b[3])), rel=rel, x=float(b[4]) + k[0] * KACHEL,
                                     z=float(b[5]) + k[1] * KACHEL, rot=float(b[7]), regeln={})
                        objekte[letzt['id']] = letzt
                    except (ValueError, IndexError):
                        letzt = None
            elif kopf == '[rule]' and letzt is not None and len(b) > 3 and b[2].strip() == 'priority':
                try:
                    letzt['regeln'][int(b[1])] = int(float(b[3]))
                except ValueError:
                    pass
            elif kopf != '[rule]':
                letzt = None
    out = {}
    for kn in netz.knoten.values():
        if kn.get('art') != 'kreuzung' or kn.get('objekt') not in objekte:
            continue
        o = objekte[kn['objekt']]
        pfade, ampel = _sco_pfade(os.path.join(omsi, *o['rel'].split(BS)))
        r = math.radians(o['rot'])
        welt = []
        for p in pfade:
            if p is None:
                welt.append(None)
                continue
            x, y, h = p
            welt.append((o['x'] + x * math.cos(r) + y * math.sin(r), o['z'] - x * math.sin(r) + y * math.cos(r),
                         (h + o['rot']) % 360.0))
        arme = {}
        for eid in kn['kanten']:
            e = netz.kanten.get(eid)
            if not e or e['art'] != 'strasse' or (e['von'] == kn['id'] and e['nach'] == kn['id']):
                continue
            am_ende = e['nach'] == kn['id']
            px, _, pz, h = netz.punkt(eid, e['laenge'] if am_ende else 0.0)
            hinein = h if am_ende else (h + 180.0) % 360.0
            q = e.get('querschnitt') or {}
            breite = max(abs(v) for v in (q.get('fahrbahn') or [-4, 4])) + 1.5
            prios = []
            for i, w in enumerate(welt):
                if w and math.hypot(w[0] - px, w[1] - pz) <= breite and winkel(w[2], hinein) <= 50:
                    prios.append(o['regeln'].get(i))
            if any(p == 64 for p in prios):
                arme[eid] = 'neben'
            elif any(p == 192 for p in prios):
                arme[eid] = 'haupt'
            else:
                arme[eid] = 'gleich'
        out[kn['id']] = dict(arme=arme, ampel=ampel, x=o['x'], z=o['z'], objekt=o['id'])
    return out


# ---------------------------------------------------------------- Platzierung

class Platz:
    """bereits gesetzte Objekte (Raster 5 m) und die Pruefungen fuer neue: Belegung, Abstaende"""

    def __init__(self, netz, belegung, gelaende, regeln, sperren=()):
        self.netz, self.bel, self.gel = netz, belegung, gelaende
        self.ab = regeln.get('abstaende', {})
        self.raster = collections.defaultdict(list)
        self.sperren = collections.defaultdict(list)     # Zufahrten, Querungen: (x, z, r)
        for x, z, r in sperren:
            self.sperren[(int(x // 5), int(z // 5))].append((x, z, r))
        self.objekte = []
        self.gruende = collections.Counter()               # warum Objekte nicht gesetzt wurden

    def klasse(self, x, z):
        return self.bel.klasse(x, z) if self.bel is not None else 'gehweg'

    def flaeche_ok(self, x, z, r, erlaubt=SEITENRAUM, gebaeude_m=None):
        """Mitte auf einer erlaubten Klasse; im Umkreis r + Abstand kein Gebaeude, Fahrbahn, Kreuzung, Wasser"""
        if self.bel is None:
            return True
        if self.klasse(x, z) not in erlaubt:
            return False
        # Fahrbahn: nur die Mitte (das 0,5-m-Raster ist an Kanten eine Zelle ungenau; den Bordabstand gibt die Querlage)
        g = self.ab.get('gebaeude_m', 0.3) if gebaeude_m is None else gebaeude_m
        for rr in (r, r + g):
            for i in range(8):
                a = math.pi * i / 4
                kl = self.klasse(x + rr * math.cos(a), z + rr * math.sin(a))
                if kl == 'gebaeude' or (rr == r and kl in ('wasser', 'bauwerk')):
                    return False
        return True

    def gesperrt(self, x, z):
        k = (int(x // 5), int(z // 5))
        for dx in (-1, 0, 1):
            for dz in (-1, 0, 1):
                for sx, sz, r in self.sperren.get((k[0] + dx, k[1] + dz), ()):
                    if math.hypot(sx - x, sz - z) < r:
                        return True
        return False

    def frei(self, x, z, r, typ=None, mindest=None, gruppe=None):
        """kein anderes Objekt naeher als r + dessen Radius + Abstand; mindest: {Typpraefix: Abstand} zusaetzlich"""
        k = (int(x // 5), int(z // 5))
        reich = 1 + int(max([r + 1] + list((mindest or {}).values())) // 5)
        for dx in range(-reich, reich + 1):
            for dz in range(-reich, reich + 1):
                for o in self.raster.get((k[0] + dx, k[1] + dz), ()):
                    if gruppe is not None and o.get('gruppe') == gruppe:
                        continue
                    d = math.hypot(o['x'] - x, o['z'] - z)
                    if d < r + o['radius'] + self.ab.get('objekt_m', 0.5) * (0 if o['radius'] < 0.15 and r < 0.15 else 1):
                        return False
                    for pfx, m in (mindest or {}).items():
                        if o['typ'].startswith(pfx) and d < m:
                            return False
        return True

    def rechteck_ok(self, ox, oz, rot, box, erlaubt=SEITENRAUM, schritt=0.5):
        """Grundriss eines Objekts (box = [xmin, _, zmin, xmax, _, zmax] in Objektkoordinaten, Drehung rot) liegt ganz
        auf erlaubten Klassen, mit Gebaeudeabstand, und kein anderes Objekt steht darin (0,3 m Rand)"""
        r = math.radians(rot)
        c, s_ = math.cos(r), math.sin(r)
        g = self.ab.get('gebaeude_m', 0.3)
        x0, z0, x1, z1 = box[0], box[2], box[3], box[5]
        nx, nz = max(2, int((x1 - x0) / schritt) + 1), max(2, int((z1 - z0) / schritt) + 1)
        for i in range(nx):
            for j in range(nz):
                lx, lz = x0 + (x1 - x0) * i / (nx - 1), z0 + (z1 - z0) * j / (nz - 1)
                x, z = ox + lx * c + lz * s_, oz - lx * s_ + lz * c
                if self.bel is not None and self.klasse(x, z) not in erlaubt:
                    return False
        if self.bel is not None:                         # Gebaeudeabstand am Rand
            for lx, lz in ((x0 - g, z0 - g), (x1 + g, z0 - g), (x1 + g, z1 + g), (x0 - g, z1 + g),
                           ((x0 + x1) / 2, z1 + g), ((x0 + x1) / 2, z0 - g)):
                if self.klasse(ox + lx * c + lz * s_, oz - lx * s_ + lz * c) == 'gebaeude':
                    return False
        k = (int(ox // 5), int(oz // 5))
        for dx in (-1, 0, 1):
            for dz in (-1, 0, 1):
                for o in self.raster.get((k[0] + dx, k[1] + dz), ()):
                    ddx, ddz = o['x'] - ox, o['z'] - oz
                    lx, lz = ddx * c - ddz * s_, ddx * s_ + ddz * c        # in Objektkoordinaten
                    rr = o['radius'] + 0.3
                    if x0 - rr <= lx <= x1 + rr and z0 - rr <= lz <= z1 + rr:
                        return False
        return True

    def setzen(self, o):
        self.objekte.append(o)
        self.raster[(int(o['x'] // 5), int(o['z'] // 5))].append(o)
        return o

    def hoehe_gehweg(self, x, z, y_spline):
        """Objekthoehe relativ zum Gelaende, damit der Fuss auf der Gehwegoberkante (Spline + 0,25 m) steht"""
        g = self.gel.hoehe(x, z) if self.gel else None
        if g is None or y_spline is None:
            return 0.0
        return round(y_spline + 0.25 - g, 3)

    def am_rand(self, kante, station, s, abstand_bord, r, erlaubt=SEITENRAUM):
        """Punkt am Seitenraum: Querlage = Bord + abstand_bord + r (nach aussen), Belegung und Abstaende pruefen
        -> (x, z, y_spline, h, querlage) oder None"""
        e = self.netz.kanten[kante]
        bord, aussen = seitenraum(e, s)
        q = bord + abstand_bord + r
        x, y, z, h = self.netz.punkt(kante, station, s * q)
        if not self.flaeche_ok(x, z, r, erlaubt):
            return None
        return x, z, y, h, s * q


def objekt(typ, x, z, hoehe, rot, quelle, **rest):
    o = dict(typ=typ, x=round(x, 3), z=round(z, 3), hoehe=round(hoehe, 3), rot=round(rot % 360.0, 2), quelle=quelle)
    o.update(rest)
    return o


# ---------------------------------------------------------------- Sperrstellen (Zufahrten, Querungen)

def sperrstellen(netz, osm, ab):
    """Zufahrten (highway=service, Fuss-/Radwege) und Querungen (highway=crossing) am Strassenrand -> [(x, z, r)]"""
    out = []
    if not osm:
        return out
    from .ausstattung_osm import abstand_strecke
    for k in osm['knoten']:
        if k['tags'].get('highway') == 'crossing':
            out.append((k['x'], k['z'], ab.get('querung_m', 3.0)))
    # Zufahrten: Wegpunkte bis 1 m Abstand (alle 1 m), die im Seitenraum einer Strasse liegen
    kanten = [e for e in netz.kanten.values() if e['art'] == 'strasse']
    raster = collections.defaultdict(list)
    for e in kanten:
        for a, b in zip(e['punkte'], e['punkte'][1:]):
            raster[(int(a[0] // 20), int(a[2] // 20))].append((e, a, b))
    for w in osm['wege']:
        hw = w['tags'].get('highway')
        if hw not in ('service', 'footway', 'path', 'cycleway', 'steps', 'pedestrian'):
            continue
        for p0, p1 in zip(w['punkte'], w['punkte'][1:]):
            L = math.hypot(p1[0] - p0[0], p1[1] - p0[1])
            for i in range(int(L) + 1):
                t = i / max(L, 1e-6)
                p = (p0[0] + (p1[0] - p0[0]) * t, p0[1] + (p1[1] - p0[1]) * t)
                for e, a, b in raster.get((int(p[0] // 20), int(p[1] // 20)), ()):
                    d, _ = abstand_strecke(p, (a[0], a[2]), (b[0], b[2]))
                    q = e.get('querschnitt') or {}
                    cw = max(abs(v) for v in q.get('fahrbahn', [-3, 3]))
                    ow = max(abs(v) for v in q.get('aussen', [-3, 3]))
                    if cw - 0.5 <= d <= ow + 1.0:
                        out.append((p[0], p[1], ab.get('zufahrt_m', 2.5)))
                        break
    return out


# ---------------------------------------------------------------- Laternen

def laternen(platz, auswahl, regeln, osm):
    R = regeln.get('laternen', {})
    ab = regeln.get('abstaende', {})
    netz = platz.netz
    n = collections.Counter()
    haupt_ab = RANG.get(R.get('osm_laterne_haupt_ab', 'tertiary'), 4)
    # 1. OSM-Laternen
    osm_je_kante = collections.defaultdict(list)
    for k in (osm or {}).get('knoten', []):
        if k['tags'].get('highway') != 'street_lamp':
            continue
        nah = netz.naechste(k['x'], k['z'])
        if not nah or nah[3] > R.get('osm_umkreis_m', 15.0):
            n['osm_weit'] += 1
            continue
        eid, st, q, _ = nah
        e = netz.kanten[eid]
        typ = 'laterne_haupt' if RANG.get(klasse_von(e), 2) >= haupt_ab else 'laterne_wohn'
        if typ not in auswahl:
            continue
        s = 1 if q >= 0 else -1
        bord, aussen = seitenraum(e, s)
        r = auswahl.radius(typ)
        x, z = k['x'], k['z']
        qq = abs(q)
        if qq < bord + ab.get('bord_m', 0.5) + 0.1 or not platz.flaeche_ok(x, z, r):
            p = None
            for d in (0.0, 1.0, -1.0, 2.0, -2.0):                 # an den Bord ruecken, entlang etwas verschieben
                p = platz.am_rand(eid, min(max(st + d, 0.0), e['laenge']), s, ab.get('bord_m', 0.5), 0.15)
                if p:
                    break
            if not p:
                n['osm_kein_platz'] += 1
                continue
            x, z, y, h, qq = p
        else:
            _, y, _, h = netz.punkt(eid, st)
        if not platz.frei(x, z, 0.15, mindest={'laterne': ab.get('laterne_laterne_m', 8.0) / 2}):
            n['osm_doppelt'] += 1
            continue
        ziel = (h - 90.0) if s > 0 else (h + 90.0)                # Ausleger zur Fahrbahn
        platz.setzen(objekt(typ, x, z, platz.hoehe_gehweg(x, z, y), ziel - auswahl[typ].get('vorne', 0), 'osm',
                            kante=eid, station=round(st, 1), seite=s, osm=k['id'], radius=0.15))
        osm_je_kante[eid].append(st)
        n['osm'] += 1
    # 2. Regel: Abstand und Seite je Strassenklasse
    for e in sorted(netz.kanten.values(), key=lambda e: e['id']):
        if e['art'] != 'strasse' or e['laenge'] < 12:
            continue
        rg = R.get(klasse_von(e))
        if not isinstance(rg, dict) or rg['typ'] not in auswahl:
            continue
        typ, a = rg['typ'], float(rg['abstand'])
        q = e.get('querschnitt') or {}
        fb = q.get('fahrbahn', [-3, 3])
        seiten = [1]
        if rg.get('seiten') == 'beide_versetzt' or (fb[1] - fb[0]) >= R.get('breit_beidseitig_m', 12.0):
            seiten = [1, -1]
        rand = ab.get('kreuzung_m', 6.0)
        L = e['laenge']
        if L < 2 * rand:
            continue
        # gleichmaessig: Phase aus der Kante (bleibt beim Neubau einer Kachel gleich)
        anz = max(1, int(round((L - 2 * rand) / a)))
        schritt = (L - 2 * rand) / anz
        for si, s in enumerate(seiten):
            versatz = schritt / 2 if (len(seiten) == 2 and si == 1) else 0.0
            i = 0
            while True:
                st = rand + versatz + i * schritt
                i += 1
                if st > L - rand + 0.01:
                    break
                if any(abs(st - so) < 0.75 * a for so in osm_je_kante.get(e['id'], ())):
                    n['regel_osm_nah'] += 1
                    continue
                p = None
                for d in (0.0, 1.5, -1.5, 3.0, -3.0):
                    stt = min(max(st + d, rand), L - rand)
                    p = platz.am_rand(e['id'], stt, s, ab.get('bord_m', 0.5), 0.15)
                    if p and not platz.gesperrt(p[0], p[1]) and platz.frei(
                            p[0], p[1], 0.15, mindest={'laterne': ab.get('laterne_laterne_m', 8.0)}):
                        break
                    p = None
                if not p:
                    n['regel_kein_platz'] += 1
                    continue
                x, z, y, h, qq = p
                ziel = (h - 90.0) if s > 0 else (h + 90.0)
                platz.setzen(objekt(typ, x, z, platz.hoehe_gehweg(x, z, y), ziel - auswahl[typ].get('vorne', 0),
                                    'regel', kante=e['id'], station=round(stt, 1), seite=s, radius=0.15))
                n['regel'] += 1
    return dict(n)


# ---------------------------------------------------------------- Verkehrszeichen

def schild_setzen(platz, auswahl, typen, kante, station, s, quelle, texte=None, info=None, schieben=(0, 1, -1, 2, -2, 3)):
    """Mast + Schild(e) am rechten Rand fuer Verkehr, der auf Seite s faehrt (s=+1: in Kantenrichtung).
    typen: Schildtypen von oben nach unten (hoechstens 2). -> Liste Objekte oder None"""
    ab = platz.ab
    typen = [t for t in typen if t in auswahl]
    if not typen:
        return None
    e = platz.netz.kanten[kante]
    L = e['laenge']
    mast_noetig = any(auswahl[t].get('mast') for t in typen)
    if mast_noetig and 'schild_mast' not in auswahl:
        return None
    # dasselbe Zeichen fuer dieselbe Fahrtrichtung steht schon in der Naehe (zwei OSM-Punkte fuer ein Schild)
    x0, _, z0, h0 = platz.netz.punkt(kante, station)
    blick0 = (h0 + (180.0 if s > 0 else 0.0)) % 360.0
    for o in platz.objekte:
        if o['typ'] == typen[-1] and math.hypot(o['x'] - x0, o['z'] - z0) < 15.0 and \
                winkel(o['rot'] + auswahl[o['typ']].get('vorne', 0), blick0) < 45:
            return [o]
    for d in schieben:
        st = min(max(station + d * s * -1, 0.5), L - 0.5)
        p = platz.am_rand(kante, st, s, ab.get('bord_schild_m', 0.4), 0.1)
        if not p or platz.gesperrt(p[0], p[1]) or not platz.frei(p[0], p[1], 0.12):
            continue
        x, z, y, h, q = p
        fahrt = h if s > 0 else (h + 180.0) % 360.0
        blick = (fahrt + 180.0) % 360.0                           # Schildflaeche schaut dem Verkehr entgegen
        hoehe = platz.hoehe_gehweg(x, z, y)
        gruppe = f'S{len(platz.objekte)}'
        out = []
        meta = dict(kante=kante, station=round(st, 1), seite=s, gruppe=gruppe, radius=0.12, **(info or {}))
        if mast_noetig:
            out.append(platz.setzen(objekt('schild_mast', x, z, hoehe, blick - auswahl['schild_mast'].get('vorne', 0),
                                           quelle, **meta)))
            hoehen = auswahl.anhaenge('schild_mast')
        for i, t in enumerate(typen[:2]):
            k = auswahl[t]
            wert = str((texte or {}).get('wert', '') or '')
            tx = [v.format(wert=wert) for v in k.get('texte', [])]
            hz = hoehe + (hoehen[min(i, len(hoehen) - 1)] if k.get('mast') else 0.0)
            extra = dict(wert=wert, eigen_schluessel=wert) if 'eigen' in k else {}
            out.append(platz.setzen(objekt(t, x, z, hz, blick - k.get('vorne', 0), quelle, texte=tx, **meta, **extra)))
        return out
    return None


def _arm_ende(netz, e, knoten):
    """(Station am Knoten, s fuer den Verkehr zum Knoten hin)"""
    if e['nach'] == knoten:
        return e['laenge'], 1
    return 0.0, -1


def verkehrszeichen(platz, auswahl, regeln, osm, rollen, kartenordner):
    netz = platz.netz
    R = regeln.get('vorfahrtschilder', {})
    T = regeln.get('tempo', {})
    n = collections.Counter()
    konflikte = []
    vor_ab = R.get('abstand_kreuzung_m', 3.0)
    # Arm -> Knoten (Kreuzung) fuer beide Enden
    osm_arm = {}                                   # (knoten, kante) -> '205'|'206'|'301'|'306'
    weitere = []
    from .ausstattung_osm import schild_codes
    for k in (osm or {}).get('knoten', []):
        t = k['tags']
        codes = schild_codes(t.get('traffic_sign'))
        hw = t.get('highway')
        if hw == 'stop':
            codes.append(('206', None))
        elif hw == 'give_way':
            codes.append(('205', None))
        if not codes:
            continue
        nah = netz.naechste(k['x'], k['z'])
        if not nah or nah[3] > 15.0:
            n['osm_weit'] += 1
            continue
        eid, st, q, _ = nah
        e = netz.kanten[eid]
        bord, _ = seitenraum(e, 1 if q >= 0 else -1)
        richt = (t.get('direction') or t.get('traffic_sign:direction') or '').lower()
        if abs(q) > bord - 0.3:                            # neben der Fahrbahn: die Seite sagt die Richtung
            s = 1 if q >= 0 else -1
        elif richt in ('forward', 'backward'):
            # forward = in Richtung des OSM-Wegs; Weg und Kante gleich gerichtet? ueber die Wege der Abfrage
            s = _osm_richtung(osm, k, netz, eid, st) * (1 if richt == 'forward' else -1)
        else:
            s = 0                                          # auf der Fahrbahn, ohne Richtung
        for code, wert in codes:
            haupt = code.split('-')[0]
            if haupt in VORFAHRT_CODES:
                # zum naechsten Kreuzungsknoten in Fahrtrichtung (ohne Richtung: zum naeheren Ende)
                kands = []
                for kn, (stk, sk) in ((e['nach'], (e['laenge'], 1)), (e['von'], (0.0, -1))):
                    if kn in rollen and (s == 0 or s == sk):
                        kands.append((abs(stk - st), kn, sk))
                if not kands:
                    n['osm_vorfahrt_ohne_kreuzung'] += 1
                    continue
                _, kn, sk = min(kands)
                osm_arm[(kn, eid)] = (haupt, k['id'], sk)
            else:
                weitere.append((code, wert, k, eid, st, s))
    # Vorfahrtzeichen je Kreuzungsarm
    for kn, info in sorted(rollen.items()):
        if info['ampel']:
            continue
        ring = any(tags(netz.kanten[k_]).get('junction') == 'roundabout' for k_ in info['arme'])
        for eid, rolle in sorted(info['arme'].items()):
            e = netz.kanten[eid]
            st, s = _arm_ende(netz, e, kn)
            if not hat_fahrtrichtung(e, s > 0):
                continue
            o = osm_arm.get((kn, eid))
            typ, quelle = None, 'regel'
            if o:
                code = o[0]
                ok = (code in ('205', '206') and rolle == 'neben') or (code in ('301', '306') and rolle == 'haupt')
                if not ok:
                    x, _, z, _ = netz.punkt(eid, st)
                    konflikte.append(dict(art='schild_widerspricht_vorfahrt', x=round(x, 1), z=round(z, 1), kante=eid,
                                          knoten=kn, osm=o[1], text=f'OSM Zeichen {code}, Vorfahrt der Karte: {rolle} '
                                          f'- nicht gesetzt'))
                    n['osm_widerspruch'] += 1
                else:
                    typ, quelle = VORFAHRT_CODES[code], 'osm'
            if typ is None and rolle == 'neben' and R.get('regel_vermutet', True):
                if tags(e).get('junction') != 'roundabout':
                    typ = 'vz_205'
            if typ is None and rolle == 'haupt' and R.get('regel_vorfahrtstrasse', True) and \
                    tags(e).get('priority_road') in ('designated', 'yes'):
                typ = 'vz_306'
            if typ is None:
                continue
            typen = [typ]
            if ring and typ in ('vz_205', 'vz_206') and tags(e).get('junction') != 'roundabout':
                typen = ['vz_215', typ]
            stv = st - s * vor_ab
            r = schild_setzen(platz, auswahl, typen, eid, stv, s, quelle,
                              info=dict(knoten=kn, rolle=rolle, osm=o[1] if o and quelle == 'osm' else None))
            n[f'{quelle}_{typ}'] += 1 if r else 0
            n['kein_platz'] += 0 if r else 1
    # weitere OSM-Zeichen (Tempo, Einfahrt verboten, Fussgaengerueberweg ...)
    osm_tempo = collections.defaultdict(list)
    for code, wert, k, eid, st, s in weitere:
        haupt = code.split('-')[0]
        if haupt == 'maxspeed':
            haupt, wert = '274', k['tags'].get('maxspeed')
        if haupt == '274' and '-' in code and not wert:
            wert = code.split('-')[1]
        if haupt == '274' and not wert:
            wert = k['tags'].get('maxspeed')
        typ = f'vz_{haupt}'
        if typ not in auswahl:
            n[f'osm_nicht_im_katalog_{haupt}'] += 1
            continue
        e = netz.kanten[eid]
        seiten = [s] if s else [sv for sv in (1, -1) if hat_fahrtrichtung(e, sv > 0)]
        for sv in seiten:
            r = schild_setzen(platz, auswahl, [typ], eid, st, sv, 'osm', texte=dict(wert=wert or ''),
                              info=dict(osm=k['id']))
            n[f'osm_{typ}'] += 1 if r else 0
            n['kein_platz'] += 0 if r else 1
            if r and haupt in ('274', '274.1'):
                osm_tempo[eid].append(st)
    # Tempowechsel aus maxspeed: am Anfang der Kante mit dem neuen Tempo, je Fahrtrichtung
    if T.get('wechsel', True):
        ak = T.get('abstand_knoten_m', 6.0)
        for kn in sorted(netz.knoten.values(), key=lambda k: k['id']):
            ks = [netz.kanten[i] for i in kn['kanten'] if netz.kanten.get(i, {}).get('art') == 'strasse']
            if len(ks) < 2:
                continue
            for e in ks:
                v = tempo(e)
                if v is None or e['laenge'] < 2 * ak + 5:
                    continue
                von_hier = e['von'] == kn['id']
                s = 1 if von_hier else -1
                if not hat_fahrtrichtung(e, s > 0):
                    continue
                h_weg = netz.punkt(e['id'], 0.0 if von_hier else e['laenge'])[3]
                h_weg = h_weg if von_hier else (h_weg + 180.0) % 360.0
                # Vorgaenger: die Kante, die am geradesten hineinfuehrt
                best = None
                for f in ks:
                    if f is e:
                        continue
                    an_ende = f['nach'] == kn['id']
                    hf = netz.punkt(f['id'], f['laenge'] if an_ende else 0.0)[3]
                    hf = hf if an_ende else (hf + 180.0) % 360.0
                    if best is None or winkel(hf, h_weg) < best[0]:
                        best = (winkel(hf, h_weg), f)
                if not best or best[0] > 45 or tempo(best[1]) in (None, v):
                    continue
                st = ak if von_hier else e['laenge'] - ak
                if any(abs(st - so) < 40 for so in osm_tempo.get(e['id'], ())):
                    continue
                if v == 30 and zone30(e) and not zone30(best[1]):
                    typen = ['vz_274.1']
                elif v != 30 and zone30(best[1]) and not zone30(e):
                    typen = ['vz_274.2', 'vz_274'] if v != 50 else ['vz_274.2']
                elif v == 50 and tempo(best[1]) is not None and tempo(best[1]) < 50 and not zone30(best[1]):
                    typen = ['vz_278']
                    v = tempo(best[1])
                else:
                    typen = ['vz_274']
                r = schild_setzen(platz, auswahl, typen, e['id'], st, s, 'regel', texte=dict(wert=str(v)),
                                  info=dict(knoten=kn['id']))
                n['regel_tempo'] += 1 if r else 0
    return dict(n), konflikte


def _osm_richtung(osm, k, netz, eid, st):
    """+1, wenn der OSM-Weg durch den Knoten k in Kantenrichtung laeuft, sonst -1"""
    h_kante = netz.punkt(eid, st)[3]
    for w in osm['wege']:
        if k['id'] in w['knoten'] and w['tags'].get('highway'):
            i = w['knoten'].index(k['id'])
            P = w['punkte']
            if len(P) < 2 or i >= len(P):
                continue
            a, b = (P[i - 1], P[i]) if i > 0 else (P[0], P[1])
            h = math.degrees(math.atan2(b[0] - a[0], b[1] - a[1])) % 360.0
            return 1 if winkel(h, h_kante) < 90 else -1
    return 1


# ---------------------------------------------------------------- Moebel aus OSM

def moebel(platz, auswahl, regeln, osm):
    netz = platz.netz
    umkreis = regeln.get('moebel_umkreis_m', 30.0)
    n = collections.Counter()
    arten = {('barrier', 'bollard'): 'poller', ('amenity', 'bench'): 'bank', ('amenity', 'waste_basket'): 'muelleimer'}
    for k in (osm or {}).get('knoten', []):
        typ = next((v for (a, b), v in arten.items() if k['tags'].get(a) == b), None)
        if not typ or typ not in auswahl:
            continue
        nah = netz.naechste(k['x'], k['z'])
        if not nah or nah[3] > umkreis:
            n[f'{typ}_weit'] += 1
            continue
        eid, st, q, d = nah
        e = netz.kanten[eid]
        s = 1 if q >= 0 else -1
        r = auswahl.radius(typ)
        x, z = k['x'], k['z']
        kl = platz.klasse(x, z)
        y = None
        bord, aussen = seitenraum(e, s)
        if kl in ('fahrbahn', 'kreuzung'):
            if typ == 'poller':                       # Poller auf befahrbarer Strasse: wuerde die Fahrbahn sperren
                n['poller_auf_fahrbahn'] += 1
                continue
            p = platz.am_rand(eid, st, s, platz.ab.get('bord_m', 0.5), r)
            if not p:
                n[f'{typ}_kein_platz'] += 1
                continue
            x, z, y, h, q = p
        elif not platz.flaeche_ok(x, z, r, SEITENRAUM + ('frei',)):
            n[f'{typ}_kein_platz'] += 1
            continue
        if not platz.frei(x, z, r):
            n[f'{typ}_doppelt'] += 1
            continue
        if y is None and abs(q) <= aussen + 0.2:
            y = netz.punkt(eid, st)[1]
        h = netz.punkt(eid, st)[3]
        zur_strasse = (h - 90.0) if s > 0 else (h + 90.0)
        hoehe = platz.hoehe_gehweg(x, z, y) if y is not None else 0.0
        platz.setzen(objekt(typ, x, z, hoehe, zur_strasse - auswahl[typ].get('vorne', 0), 'osm', kante=eid,
                            station=round(st, 1), seite=s, osm=k['id'], radius=r))
        n[typ] += 1
    return dict(n)


def zaeune(platz, auswahl, regeln, osm):
    if 'zaun' not in auswahl:
        return {}
    netz = platz.netz
    k = auswahl['zaun']
    L, mx = float(k.get('laenge', 3.0)), float(k.get('mitte_x', 0.0))
    umkreis = regeln.get('zaun_umkreis_m', 25.0)
    n = collections.Counter()
    for w in (osm or {}).get('wege', []):
        if w['tags'].get('barrier') != 'fence':
            continue
        P = w['punkte']
        for a, b in zip(P, P[1:]):
            dl = math.hypot(b[0] - a[0], b[1] - a[1])
            if dl < 0.5:
                continue
            h = math.degrees(math.atan2(b[0] - a[0], b[1] - a[1])) % 360.0
            anz = max(1, int(round(dl / L)))
            for i in range(anz):
                t = (i + 0.5) / anz
                cx, cz = a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t
                nah = netz.naechste(cx, cz)
                if not nah or nah[3] > umkreis:
                    n['weit'] += 1
                    continue
                ok = all(platz.klasse(cx + (b[0] - a[0]) / dl * f, cz + (b[1] - a[1]) / dl * f) not in VERBOTEN
                         for f in (-L / 2 + 0.2, 0.0, L / 2 - 0.2))
                if not ok:
                    n['kein_platz'] += 1
                    continue
                dx, dz = math.sin(math.radians(h)), math.cos(math.radians(h))
                ox, oz = cx - dx * mx, cz - dz * mx
                platz.setzen(objekt('zaun', ox, oz, 0.0, h - 90.0, 'osm', osm=w['id'], radius=0.0))
                n['zaun'] += 1
    return dict(n)


# ---------------------------------------------------------------- Plan und Pakete

def planen(kartenordner, omsi, katalog=None, osm_daten=True, log=print):
    """alle Objekte der Karte planen -> dict(objekte, zahlen, konflikte, haltestellen, auswahl)"""
    from . import belegung, haltestellen, netz_datei
    katalog = katalog or katalog_laden()
    regeln = katalog.get('regeln', {})
    auswahl = Auswahl(katalog, omsi)
    netz = netz_datei.Netz.laden(kartenordner, pruefen=False)
    try:
        bel = belegung.Belegung.laden(kartenordner, pruefen=False)
    except OSError:
        bel = None
        log('    Belegung fehlt - Objekte ohne Flaechenpruefung')
    osm, quellen = None, []
    if osm_daten:
        try:
            osm = osm_lesen(kartenordner, log)
            from .ausstattung_osm import QUELLE
            quellen.append(QUELLE)
        except Exception as e:
            log(f'    OSM-Ausstattung nicht geladen ({type(e).__name__}: {e}) - nur Regel-Objekte')
    platz = Platz(netz, bel, Gelaende(kartenordner), regeln, sperrstellen(netz, osm, regeln.get('abstaende', {})))
    rollen = kreuzungsrollen(kartenordner, omsi, netz)
    zahlen = {}
    # Haltestellen zuerst (sie brauchen am meisten Platz und sind am wichtigsten)
    hs, zahlen['haltestellen'] = haltestellen.planen(platz, auswahl, regeln, osm, kartenordner)
    zahlen['verkehrszeichen'], konflikte = verkehrszeichen(platz, auswahl, regeln, osm, rollen, kartenordner)
    zahlen['laternen'] = laternen(platz, auswahl, regeln, osm)
    zahlen['moebel'] = moebel(platz, auswahl, regeln, osm)
    zahlen['zaeune'] = zaeune(platz, auswahl, regeln, osm)
    return dict(objekte=platz.objekte, zahlen=zahlen, konflikte=konflikte, haltestellen=hs, auswahl=auswahl,
                quellen=quellen, rollen=rollen)


def osm_lesen(kartenordner, log=print):
    from . import ausstattung_osm as ao
    from .fotovergleich import bezug_lesen
    bz = bezug_lesen(kartenordner)
    js = ao.laden(*ao.bereich(kartenordner, schichten.kacheln(kartenordner), bz), log=log)
    return ao.merkmale(js, ao.Umrechnung(bz))


def bauen(kartenordner, omsi=None, kacheln=None, einsetzen=False, osm_daten=True, katalog=None, log=print):
    """Plan fuer die ganze Karte, Pakete fuer `kacheln` (alle, wenn None) schreiben; einsetzen: in die Karte setzen
    (nur fuer neu erzeugte Karten - im Editor setzt die Sitzung ein) -> dict(kacheln, objekte, pakete, zahlen, ...)"""
    from . import ausstattung_eigen as eigen
    from .boden import quellen_eintragen
    omsi = omsi or schichten.STANDARD_OMSI
    root = os.path.dirname(os.path.dirname(os.path.abspath(kartenordner)))
    karte = os.path.basename(os.path.normpath(kartenordner))
    alle = schichten.kacheln(kartenordner)
    kacheln = sorted(set(alle) if kacheln is None else set(map(tuple, kacheln)) & set(alle))
    plan = planen(kartenordner, omsi, katalog, osm_daten, log)
    auswahl = plan['auswahl']
    je = collections.defaultdict(list)
    for o in plan['objekte']:
        je[(math.floor(o['x'] / KACHEL), math.floor(o['z'] / KACHEL))].append(o)
    basis = os.path.join(root, 'Sceneryobjects', 'Aschaffenburg', karte, 'Objekte')
    pakete, n_obj = [], 0
    typen_benutzt = set()
    for k in kacheln:
        ordner = os.path.join(basis, f'{k[0]}_{k[1]}')
        if os.path.isdir(ordner):                       # eigene Schicht: alte Dateien der Kachel weg
            import shutil
            shutil.rmtree(ordner)
        os.makedirs(ordner, exist_ok=True)
        rel0 = BS.join(['Sceneryobjects', 'Aschaffenburg', karte, 'Objekte', f'{k[0]}_{k[1]}']) + BS
        eigene = {}
        objekte = []
        gemeinsam = {}
        for i, o in enumerate(sorted(je.get(k, []), key=lambda o: (o['typ'], o['x'], o['z']))):
            w = auswahl[o['typ']]
            if w.get('eigen') in ('vz', 'schildmast'):          # gemeinsame eigene Objekte (alle Karten)
                schl = (w['eigen'], w.get('code'), o.get('wert', ''))
                if schl not in gemeinsam:
                    gemeinsam[schl] = (eigen.vz_schreiben(root, w['code'], o.get('wert', '')) if w['eigen'] == 'vz'
                                       else eigen.schildmast_schreiben(root))
                rel = rel_fuer(karte, k, gemeinsam[schl])
            elif 'eigen' in w:
                schl = (w['eigen'], o.get('eigen_schluessel'))
                if schl not in eigene:
                    if w['eigen'] == 'haltestellenmast':
                        datei, _ = eigen.mast_schreiben(ordner, o['name'])
                    else:
                        datei = haltestellen_bord_schreiben(ordner, o)
                    eigene[schl] = rel0 + datei
                rel = eigene[schl]
            else:
                rel = rel_fuer(karte, k, w['rel'])
            typen_benutzt.add(o['typ'])
            meta = {kk: v for kk, v in o.items() if kk in ('typ', 'quelle', 'kante', 'station', 'seite', 'osm', 'knoten',
                                                         'rolle', 'gruppe')}
            if o.get('name'):
                meta['haltestelle'] = o['name']
            objekte.append(dict(name=f'{o["typ"]}_{i}', rel=rel, x=o['x'], y=o['z'], z=o['hoehe'], rot=o['rot'],
                                texte=o.get('texte', []), **meta))
        man = schichten.manifest(kartenordner, 'objekte', k, 'omsigen.ausstattung',
                                 dict(katalog=os.path.basename(KATALOG), osm=bool(plan['quellen'])), plan['quellen'])
        p = schichten.paket(karte, 'objekte', k, objekte, man, plan['quellen'])
        pfad = os.path.join(ordner, PAKET)
        with open(pfad, 'w', encoding='utf-8') as f:
            json.dump(p, f, ensure_ascii=False, indent=1)
        if einsetzen:
            schichten.einsetzen(kartenordner, pfad)
            # Manifest nach dem Einsetzen: eigene Eingaben unveraendert (Einsetzen aendert nur die Schicht objekte)
        schichten.vermerken(kartenordner, pfad, p)
        pakete.append(pfad)
        n_obj += len(objekte)
    zaehl = collections.Counter(o['typ'] for o in plan['objekte'])
    quellen_zaehl = collections.Counter(f'{o["typ"]}:{o["quelle"]}' for o in plan['objekte'])
    ad = auswahl.addons(sorted(typen_benutzt))
    quellen_eintragen(kartenordner, plan['quellen'], ['Strassenausstattung braucht: ' + ', '.join(ad)] if ad else [])
    uebersicht = dict(format=FORMAT, version=VERSION, karte=karte, objekte=dict(zaehl), quellen_je_typ=dict(quellen_zaehl),
                      zahlen=plan['zahlen'], konflikte=plan['konflikte'], haltestellen=plan['haltestellen'],
                      auswahl={t: w.get('rel') or ('eigen:' + w['eigen']) for t, w in auswahl.wahl.items()},
                      addons=ad, je_kachel={f'{k[0]}_{k[1]}': len(v) for k, v in sorted(je.items())})
    p = os.path.join(kartenordner, DATEI)
    with open(p + '.tmp', 'w', encoding='utf-8') as f:
        json.dump(uebersicht, f, ensure_ascii=False, indent=1)
    os.replace(p + '.tmp', p)
    log(f'    Ausstattung: {len(plan["objekte"])} Objekte geplant, {n_obj} in {len(pakete)} Kachel-Paketen; '
        + ', '.join(f'{t} {c}' for t, c in zaehl.most_common()))
    return dict(kacheln=len(pakete), objekte=n_obj, geplant=len(plan['objekte']), pakete=pakete, typen=dict(zaehl),
                haltestellen=len(plan['haltestellen']), konflikte=len(plan['konflikte']), addons=ad)


def haltestellen_bord_schreiben(ordner, o):
    from . import haltestellen
    return haltestellen.bord_schreiben(ordner, o)


def neue_karte(kartenordner, omsi, log=print, osm_daten=True):
    """Schritt in pipeline.erzeuge: Ausstattung der neuen Karte bauen und einsetzen (nach Gebaeuden, Belegung, Boden).
    Fehler beenden die Erzeugung nicht - python -m omsigen.ausstattung holt sie nach"""
    try:
        bauen(kartenordner, omsi, einsetzen=True, osm_daten=osm_daten, log=log)
    except Exception as e:
        log(f'    Ausstattung nicht gebaut: {type(e).__name__}: {e}')


# ---------------------------------------------------------------- Pruefung (fuer omsigen.pruefbericht)

def funde(kartenordner, omsi=None, bel=None):
    """Funde der Schicht objekte: Objekt auf Fahrbahn/Kreuzung, Haltestelle ohne Wartehalle (OSM shelter=yes),
    Vorfahrtzeichen gegen die Vorfahrt der Karte (OSM-Schild nicht gesetzt oder gesetztes Schild passt nicht mehr)"""
    from . import netz_datei
    omsi = omsi or schichten.STANDARD_OMSI
    p = os.path.join(kartenordner, DATEI)
    if not os.path.isfile(p):
        return []
    with open(p, encoding='utf-8') as f:
        ueb = json.load(f)
    out = []
    for h in ueb.get('haltestellen', []):
        if h.get('osm_wartehalle') and not h.get('wartehalle'):
            out.append(dict(art='haltestelle_ohne_wartehalle', x=h.get('x'), z=h.get('z'),
                            text=f'{h["name"]}: OSM shelter=yes, kein Platz fuer die Wartehalle ({h.get("grund", "")})'))
    for c in ueb.get('konflikte', []):
        out.append(dict(art=c['art'], x=c['x'], z=c['z'], text=c['text']))
    # gesetzte Objekte der Karte gegen Belegung und Vorfahrt
    gesetzt = []
    for k in schichten.kacheln(kartenordner):
        pk = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(kartenordner))), 'Sceneryobjects',
                          'Aschaffenburg', os.path.basename(os.path.normpath(kartenordner)), 'Objekte', f'{k[0]}_{k[1]}', PAKET)
        if os.path.isfile(pk):
            with open(pk, encoding='utf-8') as f:
                gesetzt += json.load(f).get('objekte', [])
    if bel is not None:
        for o in gesetzt:
            if o.get('typ') in ('zaun',):
                continue
            kl = bel.klasse(o['x'], o['y'])
            if kl in ('fahrbahn', 'kreuzung'):
                out.append(dict(art='objekt_auf_fahrbahn', x=round(o['x'], 1), z=round(o['y'], 1),
                                text=f'{o.get("typ")} ({o.get("quelle")}) auf {kl}'))
    try:
        netz = netz_datei.Netz.laden(kartenordner, pruefen=False)
        rollen = kreuzungsrollen(kartenordner, omsi, netz)
    except Exception:
        rollen = {}
    for o in gesetzt:
        t, kn, eid = o.get('typ'), o.get('knoten'), o.get('kante')
        if t not in ('vz_205', 'vz_206', 'vz_301', 'vz_306') or kn not in rollen:
            continue
        rolle = rollen[kn]['arme'].get(eid)
        if rolle is None:
            continue
        soll = 'neben' if t in ('vz_205', 'vz_206') else 'haupt'
        if rolle != soll:
            out.append(dict(art='schild_widerspricht_vorfahrt', x=round(o['x'], 1), z=round(o['y'], 1),
                            text=f'{t} an {eid}, Vorfahrt der Karte jetzt: {rolle} (Objekte neu bauen)'))
    return out


# ---------------------------------------------------------------- Kommandozeile

def main():
    ap = argparse.ArgumentParser(description='Strassenausstattung und Haltestellen (Schicht objekte) bauen')
    ap.add_argument('karte')
    ap.add_argument('--omsi', default=schichten.STANDARD_OMSI)
    ap.add_argument('--kacheln', nargs='*', help='nur diese Kacheln (kx_ky), Standard: alle')
    ap.add_argument('--nur-pakete', action='store_true', help='Pakete schreiben, nicht in die Karte setzen (Editor)')
    ap.add_argument('--ohne-osm', action='store_true', help='nur Regel-Objekte')
    a = ap.parse_args()
    d = a.karte if os.path.isdir(a.karte) else os.path.join(a.omsi, 'maps', a.karte)
    ks = None
    if a.kacheln:
        ks = [tuple(int(v) for v in s.replace(',', '_').split('_')) for s in a.kacheln]
    try:
        erg = bauen(d, a.omsi, ks, einsetzen=not a.nur_pakete, osm_daten=not a.ohne_osm,
                    log=lambda *x: print(*x, file=sys.stderr))
    except Exception as e:
        print(f'{type(e).__name__}: {e}', file=sys.stderr)
        sys.exit(1)
    print(json.dumps(erg, ensure_ascii=False))


if __name__ == '__main__':
    main()
