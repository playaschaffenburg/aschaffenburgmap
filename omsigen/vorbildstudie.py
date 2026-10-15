"""Vorbildstudie: misst, wie eine OMSI-2-Karte Strassen und Kreuzungen baut, und vergleicht Karten miteinander -
Massstab sind Ahlheim 5 und Staedtedreieck V3 (Nutzer, 15.10.2026). Nur lesend.

Gemessen je Karte (nur Splines mit Fahrzeugpfaden, Kreuzungen = Objekte mit [splinehelper] bzw. bei omsigen-Karten
die Kreuzungsknoten aus netz.json):
- Trasse: Anteil Geraden, Bogenradien, Elementlaengen, Kruemmungssprung zwischen Elementen (1/R in 1/km),
  Richtungsknicke an Anschluessen.
- Gradiente: Steigungen, Elemente mit Steigungswechsel (Ausrundung), Steigungssprung zwischen Elementen.
- Kreuzungen: Anzahl, Arme, Plattengroesse, geneigte Objekte (Neigung/Rollen), Steigung der Arme an der Platte
  (= Knick, wenn die Platte flach ist) und - an Armen mit >= 4 % Hangsteigung in 60 m - Laenge der Ausrundung.
- Spline-Knoten: >= 3 Strassenenden auf einem Punkt (Kreuzungen/Aufweitungen ohne Objekt).

Beispiel:
  python -m omsigen.vorbildstudie "Ahlheim 5" "Städtedreieck_V3" Aschaffenburg_Linie16_v9 --md build/vorbilder.md
Ergebnis und Folgerungen: docs/musterbuch.md."""
import argparse, collections, json, math, os, statistics as st, sys

from .ansicht import Reader, read_text, cfg_lines, _has, _labels
from .config import DEFAULT_OMSI
from . import ausstattung_modelle as AM

BS = chr(92)


# ------------------------------------------------------------------ Lesen

def kachel_lesen(path):
    """.map -> dict(version, splines, objects); Objekte mit Neigung/Rollen (ab Version 12)"""
    r = Reader(read_text(path).replace('\r', '').split('\n'))
    t = dict(version=0, splines=[], objects=[])
    while True:
        k = r.next_kw()
        if k is None:
            break
        v = t['version']
        if k == 'version':
            t['version'] = int(r.num())
        elif k in ('spline', 'spline_h'):
            if _has(v, 9):
                r.line()
            s = dict(file=r.line().strip())
            s['id'] = int(r.num()) if _has(v, 6) else -1
            if _has(v, 11):
                r.num(); r.num()
            else:
                r.line()
            s['x'], s['y'], s['z'] = r.num(), r.num(), r.num()
            s['h'], s['L'], s['R'] = r.num(), r.num(), r.num()
            s['g'] = (r.num(), r.num())
            s['dh'] = r.num() if k == 'spline_h' else None
            t['splines'].append(s)
        elif k in ('object', 'attachobj'):
            att = k == 'attachobj'
            if _has(v, 9):
                r.line()
            o = dict(file=r.line().strip())
            o['id'] = int(r.num()) if _has(v, 6) else -1
            if att:
                r.line(); r.line(); r.line()
                if _has(v, 8):
                    r.num()
            else:
                o['x'], o['z'], o['y'] = r.num(), r.num(), r.num()
                o['h'] = r.num()
                o['pitch'], o['bank'] = (r.num(), r.num()) if _has(v, 12) else (0.0, 0.0)
            _labels(r) if _has(v, 4) else None
            if not att:
                t['objects'].append(o)
    return t


def karte_lesen(d):
    """-> (Splines, Objekte, Kachelgroesse); Koordinaten global (gx, gz), Spline-Ende in s['e'] = (x, z, Richtung, Hoehe)"""
    g = Reader(cfg_lines(read_text(os.path.join(d, 'global.cfg'))))
    kacheln, welt = [], False
    while True:
        k = g.next_kw()
        if k is None:
            break
        if k == 'worldcoordinates':
            welt = True
        elif k == 'map':
            tx, tz, f = int(g.num()), int(g.num()), g.line().strip()
            p = os.path.join(d, f)
            if os.path.exists(p):
                kacheln.append((tx, tz, kachel_lesen(p)))
    T = 371.9 if welt else 300.0
    S, O = [], []
    for tx, tz, t in kacheln:
        for s in t['splines']:
            s['gx'], s['gz'] = tx * T + s['x'], tz * T + s['z']
            ex, ez, eh, ey = ende(s)
            s['e'] = (tx * T + ex, tz * T + ez, eh, ey)
            S.append(s)
        for o in t['objects']:
            o['gx'], o['gz'] = tx * T + o['x'], tz * T + o['z']
            O.append(o)
    return S, O, T, len(kacheln)


def ende(s):
    """Endpunkt (x, z), Endrichtung (Grad), Endhoehe eines Elements (Bogen: R > 0 rechts)"""
    h, L, R = math.radians(s['h']), s['L'], s['R']
    if R == 0:
        x, z, he = s['x'] + L * math.sin(h), s['z'] + L * math.cos(h), s['h']
    else:
        cx, cz = s['x'] + R * math.cos(h), s['z'] - R * math.sin(h)
        he = s['h'] + math.degrees(L / R)
        hr = math.radians(he)
        x, z = cx - R * math.cos(hr), cz + R * math.sin(hr)
    y = s['y'] + (s['dh'] if s['dh'] is not None else L * (s['g'][0] + s['g'][1]) / 200.0)
    return x, z, he, y


class Bibliothek:
    """.sli/.sco-Eigenschaften aus der OMSI-Installation (zwischengespeichert)"""

    def __init__(self, omsi):
        self.omsi, self._sli, self._sco = omsi, {}, {}

    def strasse(self, rel):
        if rel not in self._sli:
            p = os.path.join(self.omsi, *rel.split(BS))
            ja = False
            if os.path.isfile(p):
                z = [l.strip() for l in open(p, encoding='cp1252', errors='replace')]
                for i, l in enumerate(z):
                    if l.lower() == '[path]':
                        try:
                            ja = ja or int(float(z[i + 1])) == 0
                        except (ValueError, IndexError):
                            pass
            self._sli[rel] = ja
        return self._sli[rel]

    def sco(self, rel):
        if rel not in self._sco:
            p = os.path.join(self.omsi, *rel.split(BS))
            info = dict(helper=[], box=None)
            if os.path.isfile(p):
                z = [l.rstrip('\r\n') for l in open(p, encoding='cp1252', errors='replace')]
                for i, l in enumerate(z):
                    if l.strip().lower() == '[splinehelper]':
                        w = [x.strip() for x in z[i + 1:i + 8]]
                        try:
                            info['helper'].append(dict(x=float(w[1]), y=float(w[2]), z=float(w[3]), h=float(w[4])))
                        except (ValueError, IndexError):
                            pass
                if info['helper']:
                    info['box'] = AM.masse(self.omsi, rel).get('box')
            self._sco[rel] = info
        return self._sco[rel]


# ------------------------------------------------------------------ Messen

def _q(v, n=4):
    v = [x for x in v if x is not None]
    if len(v) < 2:
        return [round(x, 1) for x in v]
    return [round(x, 1) for x in st.quantiles(v, n=n, method='inclusive')]


class Enden:
    """Rasterindex der Element-Anfaenge/-Enden"""

    def __init__(self, S, zelle=10.0):
        self.z, self.r = zelle, collections.defaultdict(list)
        for s in S:
            self.r[(int(s['gx'] // zelle), int(s['gz'] // zelle))].append((s['gx'], s['gz'], s, 'a'))
            self.r[(int(s['e'][0] // zelle), int(s['e'][1] // zelle))].append((s['e'][0], s['e'][1], s, 'e'))

    def bei(self, x, z, r):
        i0, j0 = int(x // self.z), int(z // self.z)
        out = []
        for i in (i0 - 1, i0, i0 + 1):
            for j in (j0 - 1, j0, j0 + 1):
                out += [(s, w) for (a, b, s, w) in self.r[(i, j)] if math.hypot(a - x, b - z) <= r]
        return out


def _profil(s, wo, enden, laenge=60.0):
    """Steigungen der Kette von einem Anschluss weg: [(Abstand, Steigung Anfang, Steigung Ende, Laenge)] (in %)"""
    out, ab, cur, w = [], 0.0, s, wo
    for _ in range(60):
        g0, g1 = (cur['g'][0], cur['g'][1]) if w == 'a' else (-cur['g'][1], -cur['g'][0])
        out.append((ab, g0, g1, cur['L']))
        ab += cur['L']
        if ab >= laenge:
            break
        x, z = (cur['e'][0], cur['e'][1]) if w == 'a' else (cur['gx'], cur['gz'])
        nx = [(t, tw) for (t, tw) in enden.bei(x, z, 0.05) if t is not cur and t['file'] == cur['file']]
        if not nx:
            break
        cur, w = nx[0]
    return out


def _arm_kennzahlen(prof):
    """-> (Steigung an der Platte, max. Hangsteigung, Ausrundungslaenge bis 90 % davon, flache Strecke)"""
    gmax = max(max(abs(p[1]), abs(p[2])) for p in prof)
    flach = 0.0
    for p in prof:
        if abs(p[1]) < 1 and abs(p[2]) < 1:
            flach = p[0] + p[3]
        else:
            break
    aus = None
    for p in prof:
        # Element, in dem die Steigung 90 % erreicht: anteilig (Steigung linear im Element)
        if max(abs(p[1]), abs(p[2])) >= 0.9 * gmax:
            a, b = abs(p[1]), abs(p[2])
            f = 0.0 if a >= 0.9 * gmax or b == a else (0.9 * gmax - a) / (b - a)
            aus = p[0] + f * p[3] - flach
            break
    return abs(prof[0][1]), gmax, aus, flach


def messen(d, omsi=DEFAULT_OMSI, log=print):
    bib = Bibliothek(omsi)
    S_alle, O, T, n_kacheln = karte_lesen(d)
    S = [s for s in S_alle if bib.strasse(s['file'])]
    enden = Enden(S)
    r = dict(karte=os.path.basename(os.path.normpath(d)), kacheln=n_kacheln, splines=len(S_alle), strassen_splines=len(S),
             objekte=len(O))
    # Trasse
    radien = [abs(s['R']) for s in S if s['R'] != 0]
    knick, ksprung, gsprung = [], [], []
    for s in S:
        for t, w in enden.bei(s['e'][0], s['e'][1], 0.05):
            if t is s or w != 'a' or t['file'] != s['file']:
                continue
            knick.append(abs((t['h'] - s['e'][2] + 180) % 360 - 180))
            k1 = 0 if s['R'] == 0 else 1000 / abs(s['R'])
            k2 = 0 if t['R'] == 0 else 1000 / abs(t['R'])
            ksprung.append(abs(k1 - k2))
            gsprung.append(abs(t['g'][0] - s['g'][1]))
            break
    r['trasse'] = dict(geraden_anteil=round(sum(1 for s in S if s['R'] == 0) / max(1, len(S)), 2),
                       radius_quartile=_q(radien), laenge_quartile=_q([s['L'] for s in S]),
                       kruemmungssprung_quartile=_q(ksprung),
                       kruemmungssprung_ueber_20=round(sum(k > 20 for k in ksprung) / max(1, len(ksprung)), 2),
                       richtungsknick_ueber_2grad=sum(k > 2 for k in knick))
    r['gradiente'] = dict(ueber_6proz=round(sum(1 for s in S if max(abs(s['g'][0]), abs(s['g'][1])) >= 6) / max(1, len(S)), 3),
                          ausrundung_anteil=round(sum(1 for s in S if abs(s['g'][0] - s['g'][1]) > 0.05) / max(1, len(S)), 2),
                          steigungssprung_quartile=_q(gsprung), steigungssprung_ueber_2=sum(g > 2 for g in gsprung))
    # Kreuzungen: Arme mit Anschluss-Spline
    arme, platten, n_kr, geneigt, armzahl = [], [], 0, 0, collections.Counter()
    netz_datei = os.path.join(d, 'netz.json')
    helper_objekte = [(o, bib.sco(o['file'])) for o in O if o['file'].lower().endswith('.sco')]
    helper_objekte = [(o, i) for o, i in helper_objekte if len(i['helper']) >= 2]
    if helper_objekte:
        for o, i in helper_objekte:
            rr = math.radians(o['h'])
            an = []
            for hp in i['helper']:
                wx = o['gx'] + hp['x'] * math.cos(rr) + hp['z'] * math.sin(rr)
                wz = o['gz'] - hp['x'] * math.sin(rr) + hp['z'] * math.cos(rr)
                t = enden.bei(wx, wz, 0.3)
                if t:
                    an.append(t[0])
            if len(an) < 2:
                continue
            n_kr += 1
            armzahl[len(an)] += 1
            gk = abs(o['pitch']) > 0.01 or abs(o['bank']) > 0.01
            geneigt += gk
            if i['box']:
                b = i['box']
                platten.append(max(b[3] - b[0], b[5] - b[2]))
            if not gk:                                  # geneigte Platte: Steigung an der Platte ist kein Knick
                arme += [_arm_kennzahlen(_profil(s, w, enden)) for s, w in an]
    elif os.path.exists(netz_datei):                   # omsigen-Karte: Kreuzungsknoten aus netz.json
        netz = json.load(open(netz_datei, encoding='utf-8'))
        byid = {s['id']: s for s in S}
        kanten = {k['id']: k for k in netz['kanten']}
        for kn in netz['knoten']:
            if kn['art'] != 'kreuzung':
                continue
            x0, _, z0 = kn['pos']
            an, ab = [], []
            for kid in kn['kanten']:
                ids = [i for i in kanten.get(kid, {}).get('splines', []) if i in byid]
                if not ids:
                    continue
                kand = [(byid[ids[0]], 'a'), (byid[ids[0]], 'e'), (byid[ids[-1]], 'a'), (byid[ids[-1]], 'e')]
                s, w = min(kand, key=lambda c: math.hypot((c[0]['gx'] if c[1] == 'a' else c[0]['e'][0]) - x0,
                                                         (c[0]['gz'] if c[1] == 'a' else c[0]['e'][1]) - z0))
                an.append((s, w))
                ab.append(math.hypot((s['gx'] if w == 'a' else s['e'][0]) - x0, (s['gz'] if w == 'a' else s['e'][1]) - z0))
            if len(an) < 2:
                continue
            n_kr += 1
            armzahl[len(an)] += 1
            platten.append(2 * st.median(ab))
            arme += [_arm_kennzahlen(_profil(s, w, enden)) for s, w in an]
    hang = [a for a in arme if a[1] >= 4.0]
    r['kreuzungen'] = dict(anzahl=n_kr, arme=dict(sorted(armzahl.items())), platte_quartile=_q(platten),
                           geneigt=geneigt, arme_gesamt=len(arme),
                           knick_ueber_2proz=sum(1 for a in arme if a[0] >= 2), knick_ueber_4proz=sum(1 for a in arme if a[0] >= 4),
                           knick_ueber_8proz=sum(1 for a in arme if a[0] >= 8),
                           hang_arme=len(hang), hang_ausrundung_quartile=_q([a[2] for a in hang if a[0] < 1]),
                           hang_flach_quartile=_q([a[3] for a in hang]))
    # Spline-Knoten ohne Objekt
    gruppen = collections.Counter()
    for s in S:
        gruppen[(round(s['gx'] * 2) / 2, round(s['gz'] * 2) / 2)] += 1
        gruppen[(round(s['e'][0] * 2) / 2, round(s['e'][1] * 2) / 2)] += 1
    r['spline_knoten_3plus'] = sum(1 for n in gruppen.values() if n >= 3)
    log(f"{r['karte']}: {len(S)} Strassensplines, {n_kr} Kreuzungen")
    return r


ZEILEN = [
    ('Strassensplines', lambda r: r['strassen_splines']),
    ('Anteil Geraden', lambda r: r['trasse']['geraden_anteil']),
    ('Bogenradius 25/50/75 % [m]', lambda r: r['trasse']['radius_quartile']),
    ('Elementlaenge 25/50/75 % [m]', lambda r: r['trasse']['laenge_quartile']),
    ('Kruemmungssprung 25/50/75 % [1/km]', lambda r: r['trasse']['kruemmungssprung_quartile']),
    ('Anschluesse mit Kruemmungssprung > 20/km', lambda r: r['trasse']['kruemmungssprung_ueber_20']),
    ('Elemente mit Ausrundung (Steigung Anfang != Ende)', lambda r: r['gradiente']['ausrundung_anteil']),
    ('Steigungssprung zwischen Elementen > 2 %-Punkte', lambda r: r['gradiente']['steigungssprung_ueber_2']),
    ('Kreuzungen', lambda r: r['kreuzungen']['anzahl']),
    ('Arme je Kreuzung', lambda r: r['kreuzungen']['arme']),
    ('Kreuzungsplatte Kante 25/50/75 % [m]', lambda r: r['kreuzungen']['platte_quartile']),
    ('geneigte Kreuzungsobjekte', lambda r: r['kreuzungen']['geneigt']),
    ('Arme mit Knick an der Platte >= 2 / 4 / 8 %',
     lambda r: f"{r['kreuzungen']['knick_ueber_2proz']} / {r['kreuzungen']['knick_ueber_4proz']} / {r['kreuzungen']['knick_ueber_8proz']} von {r['kreuzungen']['arme_gesamt']}"),
    ('Hang-Arme (>= 4 % in 60 m)', lambda r: r['kreuzungen']['hang_arme']),
    ('  Ausrundung 0 % -> Hang 25/50/75 % [m]', lambda r: r['kreuzungen']['hang_ausrundung_quartile']),
    ('Spline-Knoten mit >= 3 Strassenenden', lambda r: r['spline_knoten_3plus']),
]


def markdown(R):
    kopf = '| | ' + ' | '.join(r['karte'] for r in R) + ' |\n|---|' + '---|' * len(R) + '\n'
    return kopf + ''.join('| ' + n + ' | ' + ' | '.join(str(f(r)) for r in R) + ' |\n' for n, f in ZEILEN)


def main(argv=None):
    ap = argparse.ArgumentParser(description='Strassen- und Kreuzungsbau von Karten messen und vergleichen')
    ap.add_argument('karten', nargs='+', help='Kartenordner oder Name unter <OMSI>/maps')
    ap.add_argument('--omsi', default=DEFAULT_OMSI)
    ap.add_argument('--json')
    ap.add_argument('--md')
    a = ap.parse_args(argv)
    R = []
    for k in a.karten:
        d = k if os.path.isdir(k) else os.path.join(a.omsi, 'maps', k)
        R.append(messen(d, a.omsi))
    t = markdown(R)
    print(t)
    if a.json:
        json.dump(R, open(a.json, 'w', encoding='utf-8'), indent=1, ensure_ascii=False)
    if a.md:
        os.makedirs(os.path.dirname(os.path.abspath(a.md)), exist_ok=True)
        open(a.md, 'w', encoding='utf-8').write(t)


if __name__ == '__main__':
    sys.exit(main())
