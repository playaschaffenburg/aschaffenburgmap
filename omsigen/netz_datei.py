"""Strassennetz einer Karte als Datei (Qualitaetsplan F3): maps/<Karte>/netz.json.

Abgeleitet aus der Karte selbst (den [spline]-Eintraegen aller Kacheln) - so stimmt es fuer Generator-Karten, im Editor
gebaute und von Hand geaenderte Karten gleichermassen. Splines, deren Enden in Lage und Richtung zusammenfallen
(gleiche .sli, gleich gespiegelt), bilden eine Kante; Kantenenden werden zu Knoten (Kreuzung, Querschnittswechsel,
Ende). Spaetere Schichten beziehen sich auf (Kante, Station, Seite): Laterne an Kante E1234, 35 m, 6 m rechts.

Kennungen: Kante "E<ID des ersten Splines>", Knoten "N<x>_<z>" (Lage auf 1 dm) - beide bleiben gleich, solange die
Karte an der Stelle gleich bleibt. Aendert sich das Netz einer Kachel, aendert sich ihr Fingerabdruck (omsigen.schichten)
und netz.json gilt als veraltet (Netz.laden prueft das).

  {"format": "omsigen-netz", "version": 1, "karte": ..., "fingerabdruck": {"kx_ky": ...},
   "knoten": [{"id", "pos": [x, y, z], "art": kreuzung|wechsel|ende, "kanten": [...], "objekt": ID des K_*-Objekts
               (Kreuzung "J<ID>") bzw. des UE_*-Objekts (Uebergang zwischen Querschnitten "U<ID>", S1)}],
   "kanten": [{"id", "splines": [IDs], "sli", "spiegel", "art": strasse|unsichtbar|bauwerk, "von", "nach", "laenge",
               "punkte": [[x, y, z, station], ...] (alle ~2 m), "querschnitt": {"fahrbahn": [links, rechts],
               "aussen": [links, rechts], "spuren": [[querlage, richtung], ...], "gehwege": [querlage, ...]},
               "attribute": {name, highway, ...}}]}
Koordinaten: Kartenmeter (x Ost, z Nord, y Hoehe), Querlage positiv = rechts der Fahrtrichtung der Kante.
Attribute (OSM-Name, Klasse, Tags) schreibt der Generator je Spline-ID nach maps/<Karte>/netz_attribute.json.

  python -m omsigen.netz_datei <Karte> [--omsi <OMSI-Ordner>]
"""
import argparse
import json
import math
import os
import sys

from . import schichten

FORMAT, VERSION = 'omsigen-netz', 1
DATEI, ATTRIBUTE = 'netz.json', 'netz_attribute.json'
KACHEL = 300.0
SCHRITT = 2.0            # m zwischen den Punkten einer Kante
EPS_LAGE, EPS_RICHTUNG = 0.05, 2.0
KREUZUNG_RADIUS = 30.0   # m um ein K_*-Objekt: Kantenende gehoert zu dieser Kreuzung


# ---------------------------------------------------------------- Geometrie eines OMSI-Elements

def rechts(h):
    r = math.radians(h)
    return (math.cos(r), -math.sin(r))


def richtung_v(h):
    r = math.radians(h)
    return (math.sin(r), math.cos(r))


def element_bei(x, z, h, L, R, t):
    """Punkt und Richtung nach t Metern (Radius > 0 = Rechtskurve; wie editor/src/netz.rs Stueck::bei)"""
    if R == 0:
        d = richtung_v(h)
        return x + d[0] * t, z + d[1] * t, h
    n = rechts(h)
    cx, cz = x + n[0] * R, z + n[1] * R
    h2 = h + math.degrees(t / R)
    n2 = rechts(h2)
    return cx - n2[0] * R, cz - n2[1] * R, h2 % 360.0


def _winkel(a, b):
    return abs((a - b + 180.0) % 360.0 - 180.0)


# ---------------------------------------------------------------- Karte lesen

def splines_lesen(kartenordner):
    """alle [spline]/[spline_h] der Karte -> {id: dict(sli, spiegel, x, y, z, h, L, R, g0, g1, dh, kachel, prev, next)}"""
    out = {}
    for k in schichten.kacheln(kartenordner):
        _, bl = schichten.bloecke(schichten.lesen(kartenordner, k))
        for b in bl:
            kopf = b[0].strip()
            # [spline_h]: die Hoehenaenderung steht direkt nach den beiden Steigungen (wie openOMSI tile.rs, Editor)
            if kopf not in ('[spline]', '[spline_h]') or len(b) < (20 if kopf == '[spline_h]' else 19):
                continue
            try:
                f = [float(v) for v in b[3:14]]
                dh = float(b[14]) if kopf == '[spline_h]' else None
            except ValueError:
                continue
            sid = int(f[0])
            out[sid] = dict(id=sid, sli=b[2].strip(), spiegel=any(z.strip() == '[mirror]' for z in b),
                            prev=int(f[1]), next=int(f[2]), x=f[3] + k[0] * KACHEL, y=f[4], z=f[5] + k[1] * KACHEL,
                            h=f[6], L=f[7], R=f[8], g0=f[9], g1=f[10], dh=dh, kachel=list(k))
    return out


def _ue_laenge(kartenordner, rel):
    """Laenge eines Uebergangsobjekts (UE_*.sco, uebergang.rs): groesstes y seiner Pfade"""
    sco = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(kartenordner))), *rel.split(chr(92)))
    try:
        z = [l.strip() for l in open(sco, encoding='cp1252', errors='replace')]
    except OSError:
        return 0.0
    ys = []
    for i, l in enumerate(z):
        if l == '[path]' and i + 6 < len(z):
            try:
                y0, h, L = float(z[i + 2]), math.radians(float(z[i + 4])), float(z[i + 6])
                ys += [y0, y0 + L * math.cos(h)]
            except ValueError:
                pass
    return max(ys, default=0.0)


def kreuzungen_lesen(kartenordner):
    """Kreuzungsobjekte (K_*.sco) und Uebergaenge zwischen Querschnitten (UE_*.sco, S1) -> [dict(id, rel, x, z, art,
    rot, laenge)]"""
    out = []
    for k in schichten.kacheln(kartenordner):
        _, bl = schichten.bloecke(schichten.lesen(kartenordner, k))
        for b in bl:
            rel = schichten.objekt_pfad(b)
            name = os.path.basename(rel.replace(chr(92), '/')).upper()
            if b[0].strip() != '[object]' or not name.startswith(('K_', 'UE_')):
                continue
            try:
                o = dict(id=int(float(b[3])), rel=rel, x=float(b[4]) + k[0] * KACHEL, z=float(b[5]) + k[1] * KACHEL,
                         art='kreuzung' if name.startswith('K_') else 'wechsel', rot=float(b[7]), laenge=0.0)
            except (ValueError, IndexError):
                continue
            if o['art'] == 'wechsel':
                o['laenge'] = _ue_laenge(kartenordner, rel)
            out.append(o)
    return out


def _abstand_objekt(o, x, z):
    """Abstand eines Punkts zum Objekt (Uebergang: zur Strecke von seinem Ursprung bis zum anderen Ende)"""
    if not o.get('laenge'):
        return math.hypot(o['x'] - x, o['z'] - z)
    d = richtung_v(o['rot'])
    t = max(0.0, min(o['laenge'], (x - o['x']) * d[0] + (z - o['z']) * d[1]))
    return math.hypot(o['x'] + d[0] * t - x, o['z'] + d[1] * t - z)


def ende(s):
    x, z, h = element_bei(s['x'], s['z'], s['h'], s['L'], s['R'], s['L'])
    return x, z, h, hoehe_bei(s, s['L'])


def hoehe_bei(s, t):
    """Hoehe nach t Metern: [spline] Steigung linear von g0 nach g1 (%), [spline_h] kubisch mit Hoehenaenderung dh
    (wie OMSI, hoehen.hoehe_im_element)"""
    from .hoehen import hoehe_im_element
    return hoehe_im_element(s['y'], s['g0'], s['g1'], s.get('dh'), s['L'] or 1.0, t)


# ---------------------------------------------------------------- ableiten

def _querschnitt(sdb, sli, spiegel):
    try:
        q = sdb[sli]
    except KeyError:
        return None
    m = -1.0 if spiegel else 1.0
    lo = lambda a, b: sorted([a * m, b * m])
    return dict(fahrbahn=lo(q['cl'], q['cr']), aussen=lo(q['ol'], q['or_']),
                spuren=[[q0 * m, (r if not spiegel else 1 - r) if r in (0, 1) else r] for q0, r in q['lanes']],
                gehwege=[w * m for w in q['walks']])


def _art(sli, qs):
    if 'invis' in sli.lower():
        return 'unsichtbar'
    if qs and (qs['spuren'] or qs['gehwege']):
        return 'strasse'
    return 'bauwerk'


def ableiten(kartenordner, omsi=None):
    """Netz aus der Karte -> dict (Format siehe oben)"""
    from .splinedb import SplineDB
    # ohne OMSI-Ordner: die Wurzel der Karte (maps/<Karte>/../..) - dort liegen die eigenen Splines (z. B. Querschnitte)
    sdb = SplineDB(omsi or os.path.dirname(os.path.dirname(os.path.abspath(kartenordner))))
    sp = splines_lesen(kartenordner)
    attr_pfad = os.path.join(kartenordner, ATTRIBUTE)
    attr = json.load(open(attr_pfad, encoding='utf-8')) if os.path.isfile(attr_pfad) else {}
    # Nachfolger ueber zusammenfallende Enden (gleiche .sli und Spiegelung); eindeutig, sonst Kantenende
    anfang = {}
    for s in sp.values():
        anfang.setdefault((round(s['x'], 1), round(s['z'], 1)), []).append(s)
    nach = {}
    for s in sp.values():
        ex, ez, eh, _ = ende(s)
        kand = [t for dx in (-0.1, 0.0, 0.1) for dz in (-0.1, 0.0, 0.1)
                for t in anfang.get((round(ex + dx, 1), round(ez + dz, 1)), [])]
        kand = [t for t in {id(t): t for t in kand}.values() if t['id'] != s['id'] and t['sli'] == s['sli']
                and t['spiegel'] == s['spiegel'] and math.hypot(t['x'] - ex, t['z'] - ez) < EPS_LAGE
                and _winkel(t['h'], eh) < EPS_RICHTUNG]
        if len(kand) == 1:
            nach[s['id']] = kand[0]['id']
    vor = {}
    for a, b in nach.items():
        vor.setdefault(b, []).append(a)
    vor = {b: v[0] for b, v in vor.items() if len(v) == 1}
    nach = {a: b for a, b in nach.items() if vor.get(b) == a}
    # Ketten
    kanten, gesehen = [], set()
    starts = [i for i in sp if i not in vor] + [i for i in sp if i in vor]   # Ringe zuletzt
    for i in starts:
        if i in gesehen:
            continue
        kette = [i]
        gesehen.add(i)
        while kette[-1] in nach and nach[kette[-1]] not in gesehen:
            kette.append(nach[kette[-1]])
            gesehen.add(kette[-1])
        kanten.append(kette)
    # Kanten mit Punkten
    out_kanten = []
    for kette in kanten:
        s0 = sp[kette[0]]
        qs = _querschnitt(sdb, s0['sli'], s0['spiegel'])
        punkte, station = [], 0.0
        for sid in kette:
            s = sp[sid]
            n = max(1, int(math.ceil(s['L'] / SCHRITT)))
            for j in range(n + (1 if sid == kette[-1] else 0)):
                t = s['L'] * j / n
                x, z, _ = element_bei(s['x'], s['z'], s['h'], s['L'], s['R'], t)
                punkte.append([round(x, 3), round(hoehe_bei(s, t), 3), round(z, 3), round(station + t, 3)])
            station += s['L']
        a = next((attr[str(sid)] for sid in kette if str(sid) in attr), {})
        out_kanten.append(dict(id=f'E{kette[0]}', splines=kette, sli=s0['sli'], spiegel=s0['spiegel'],
                               art=a.get('art') or _art(s0['sli'], qs), laenge=round(station, 3), punkte=punkte,
                               querschnitt=qs, attribute=a))
    # Knoten: Kantenenden auf 1 dm zusammenfassen
    kreuz = kreuzungen_lesen(kartenordner)
    knoten = {}
    for e in out_kanten:
        for wo, p in (('von', e['punkte'][0]), ('nach', e['punkte'][-1])):
            nid = f'N{round(p[0], 1):g}_{round(p[2], 1):g}'
            kn = knoten.setdefault(nid, dict(id=nid, pos=[p[0], p[1], p[2]], kanten=[]))
            kn['kanten'].append(e['id'])
            e[wo] = nid
    # Kantenenden an einem Kreuzungsobjekt: ein Knoten "J<Objekt-ID>" fuer die ganze Kreuzung (die Arme enden am
    # Plattenrand an verschiedenen Stellen); sonst Querschnittswechsel (zwei Enden treffen sich) oder Ende
    kante = {e['id']: e for e in out_kanten}
    fertig = {}
    for kn in knoten.values():
        x, z = kn['pos'][0], kn['pos'][2]
        nah = min(kreuz, key=lambda o: _abstand_objekt(o, x, z), default=None)
        grenze = 0.5 if nah and nah['art'] == 'wechsel' else KREUZUNG_RADIUS
        if nah and len(kn['kanten']) < 2 and _abstand_objekt(nah, x, z) <= grenze:
            jid = f'{"J" if nah["art"] == "kreuzung" else "U"}{nah["id"]}'
            j = fertig.setdefault(jid, dict(id=jid, pos=[round(nah['x'], 3), kn['pos'][1], round(nah['z'], 3)],
                                            art=nah['art'], objekt=nah['id'], kanten=[]))
            for eid in kn['kanten']:
                e = kante[eid]
                for wo in ('von', 'nach'):
                    if e.get(wo) == kn['id']:
                        e[wo] = jid
                j['kanten'].append(eid)
            continue
        kn['art'] = 'wechsel' if len(kn['kanten']) >= 2 else 'ende'
        fertig[kn['id']] = kn
    fp = {f'{k[0]}_{k[1]}': schichten.fingerabdruck(kartenordner, k, 'netz') for k in schichten.kacheln(kartenordner)}
    return dict(format=FORMAT, version=VERSION, karte=os.path.basename(os.path.normpath(kartenordner)),
                fingerabdruck=fp, knoten=sorted(fertig.values(), key=lambda k: k['id']), kanten=out_kanten)


def schreiben(kartenordner, omsi=None):
    netz = ableiten(kartenordner, omsi)
    p = os.path.join(kartenordner, DATEI)
    with open(p + '.tmp', 'w', encoding='utf-8') as f:
        json.dump(netz, f, ensure_ascii=False, separators=(',', ':'))
    os.replace(p + '.tmp', p)
    return netz


def attribute_schreiben(kartenordner, attribute):
    """Generator: Attribute je Spline-ID (OSM-Name, Klasse, Tags, Art)"""
    with open(os.path.join(kartenordner, ATTRIBUTE), 'w', encoding='utf-8') as f:
        json.dump({str(k): v for k, v in attribute.items()}, f, ensure_ascii=False)


# ---------------------------------------------------------------- lesen und abfragen

class Netz:
    """netz.json einer Karte mit Abfragen fuer spaetere Schichten"""

    def __init__(self, daten):
        self.daten = daten
        self.kanten = {e['id']: e for e in daten['kanten']}
        self.knoten = {k['id']: k for k in daten['knoten']}

    @classmethod
    def laden(cls, kartenordner, pruefen=True):
        with open(os.path.join(kartenordner, DATEI), encoding='utf-8') as f:
            n = cls(json.load(f))
        if pruefen:
            alt = n.veraltet(kartenordner)
            if alt:
                raise RuntimeError(f'netz.json ist veraltet ({len(alt)} Kacheln geaendert, z. B. {alt[0]}) - '
                                   f'python -m omsigen.netz_datei neu ausfuehren')
        return n

    def veraltet(self, kartenordner):
        """Kacheln, deren Netz sich seit dem Schreiben von netz.json geaendert hat"""
        fp = self.daten.get('fingerabdruck', {})
        out = []
        for k in schichten.kacheln(kartenordner):
            key = f'{k[0]}_{k[1]}'
            if fp.get(key) != schichten.fingerabdruck(kartenordner, k, 'netz'):
                out.append(key)
        return out

    def punkt(self, kante, station, seite=0.0):
        """(x, y, z, Richtung) auf einer Kante: station m ab Anfang, seite m rechts (negativ: links)"""
        P = self.kanten[kante]['punkte']
        station = min(max(station, 0.0), P[-1][3])
        lo, hi = 0, len(P) - 1
        while hi - lo > 1:
            mid = (lo + hi) // 2
            if P[mid][3] <= station:
                lo = mid
            else:
                hi = mid
        a, b = P[lo], P[hi]
        t = 0.0 if b[3] == a[3] else (station - a[3]) / (b[3] - a[3])
        x, y, z = (a[i] + (b[i] - a[i]) * t for i in range(3))
        h = math.degrees(math.atan2(b[0] - a[0], b[2] - a[2])) % 360.0
        n = rechts(h)
        return x + n[0] * seite, y, z + n[1] * seite, h

    def naechste(self, x, z, arten=('strasse',), min_laenge=0.0):
        """naechste Kante (mindestens min_laenge lang): (Kante, Station, Querlage rechts positiv, Abstand)"""
        best = None
        for e in self.kanten.values():
            if (arten and e['art'] not in arten) or e['laenge'] < min_laenge:
                continue
            P = e['punkte']
            for a, b in zip(P, P[1:]):
                dx, dz = b[0] - a[0], b[2] - a[2]
                ll = dx * dx + dz * dz
                t = 0.0 if ll == 0 else max(0.0, min(1.0, ((x - a[0]) * dx + (z - a[2]) * dz) / ll))
                px, pz = a[0] + dx * t, a[2] + dz * t
                d = math.hypot(x - px, z - pz)
                if best is None or d < best[3]:
                    seite = ((x - a[0]) * dz - (z - a[2]) * dx) / math.sqrt(ll) if ll else 0.0
                    best = (e['id'], a[3] + (b[3] - a[3]) * t, seite, d)
        return best

    def streifen(self, kante, links, rechts_, schritt=None):
        """Umriss zwischen Querlage links und rechts (z. B. querschnitt['aussen']) als Polygon [(x, z), ...]"""
        e = self.kanten[kante]
        L = e['laenge']
        n = max(1, int(math.ceil(L / (schritt or SCHRITT))))
        st = [L * i / n for i in range(n + 1)]
        li = [self.punkt(kante, s, links)[::2][:2] for s in st]
        re = [self.punkt(kante, s, rechts_)[::2][:2] for s in st]
        return [(p[0], p[1]) for p in li] + [(p[0], p[1]) for p in reversed(re)]


def main():
    ap = argparse.ArgumentParser(description='Strassennetz einer Karte nach netz.json')
    ap.add_argument('karte')
    ap.add_argument('--omsi', default=schichten.STANDARD_OMSI)
    a = ap.parse_args()
    d = a.karte if os.path.isdir(a.karte) else os.path.join(a.omsi, 'maps', a.karte)
    n = schreiben(d, a.omsi if os.path.isdir(a.omsi) else None)
    arten = {}
    for e in n['kanten']:
        arten[e['art']] = arten.get(e['art'], 0) + 1
    kn = {}
    for k in n['knoten']:
        kn[k['art']] = kn.get(k['art'], 0) + 1
    print(json.dumps(dict(datei=os.path.join(d, DATEI), kanten=len(n['kanten']), knoten=len(n['knoten']),
                          kanten_arten=arten, knoten_arten=kn,
                          laenge_km=round(sum(e['laenge'] for e in n['kanten'] if e['art'] == 'strasse') / 1000, 2)),
                     ensure_ascii=False))


if __name__ == '__main__':
    main()
