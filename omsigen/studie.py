"""Kartenstudie: wertet vorhandene OMSI-2-Karten aus, um zu lernen, wie erfahrene Kartenbauer Kreuzungen,
Vorfahrt, Ampeln und Strassen aufbauen. Ergebnis: Bericht (Markdown), Daten (JSON) und Beispielbilder.
Nur lesend.

Beispiel:
  python -m omsigen.studie --alle --bericht docs/kartenstudie.md --json docs/kartenstudie.json --bilder build/studie
  python -m omsigen.studie Grundorf Mainz
"""
import argparse, collections, glob, json, math, os, statistics, sys, time
from .ansicht import analyse, write_png, KIND
from .config import DEFAULT_OMSI
from .geom import norm180

OPENOMSI_MAPS = os.path.join(os.path.expanduser('~'), 'Documents', 'OpenOmsi')


def all_maps(omsi=DEFAULT_OMSI, extra=OPENOMSI_MAPS, skip=('Aschaffenburg',)):
    dirs = glob.glob(os.path.join(omsi, 'maps', '*', 'global.cfg')) if omsi else []
    dirs += glob.glob(os.path.join(extra, '*', 'maps', '*', 'global.cfg')) if extra else []
    out = []
    for g in sorted(dirs):
        d = os.path.dirname(g)
        if any(os.path.basename(d).startswith(s) for s in skip) or not glob.glob(os.path.join(d, 'tile_*.map')):
            continue
        out.append(d)
    return out


def _turn(dh):
    if abs(dh) < 30:
        return 'gerade'
    if abs(dh) > 150:
        return 'wenden'
    return 'rechts' if dh > 0 else 'links'


def movements(lanes):
    """Strassenpfade je Objekt zu Bewegungen verketten (Pfad-Ende -> Pfad-Anfang im selben Objekt)
    -> {(kachel, id): [dict(paths=[lane...], dh, prio, ampel, blinker, start, h0)]}"""
    by_obj = collections.defaultdict(list)
    for l in lanes:
        if l['src']['art'] == 'Objekt' and l['kind'] == 0:
            by_obj[(l['src']['kachel'], l['src']['id'])].append(l)
    out = {}
    for k, L in by_obj.items():
        segs = []
        for l in L:            # in Fahrtrichtung
            if l['d'] in (0, 2):
                segs.append(dict(a=l['pts'][0], b=l['pts'][-1], h0=l['h0'], h1=l['h1'], l=l))
            if l['d'] in (1, 2):
                segs.append(dict(a=l['pts'][-1], b=l['pts'][0], h0=(l['h1'] + 180) % 360, h1=(l['h0'] + 180) % 360, l=l))
        nxt = collections.defaultdict(list)
        fed = set()
        for i, s in enumerate(segs):
            for j, t in enumerate(segs):
                if i != j and t['l'] is not s['l'] and math.dist(s['b'], t['a']) < 0.3 and abs(norm180(s['h1'] - t['h0'])) < 20:
                    nxt[i].append(j); fed.add(j)
        mv = []

        def walk(i, chain, depth=0):
            chain = chain + [i]
            if not nxt[i] or depth > 12:
                mv.append(chain)
                return
            for j in nxt[i]:
                if j not in chain:
                    walk(j, chain, depth + 1)
        for i in range(len(segs)):
            if i not in fed:
                walk(i, [])
        res = []
        for ch in mv[:400]:
            P = [segs[i]['l'] for i in ch]
            dh = norm180(segs[ch[-1]]['h1'] - segs[ch[0]]['h0'])
            prios = [p['prio'] for p in P if p['prio'] is not None]
            res.append(dict(paths=P, dh=dh, typ=_turn(dh), prio=prios[0] if prios else None,
                            ampel=max(p['ampel'] for p in P), blinker=max(p['blinker'] for p in P),
                            start=segs[ch[0]]['a'], h0=segs[ch[0]]['h0'],
                            ende=segs[ch[-1]]['b'], h1=segs[ch[-1]]['h1']))
        out[k] = res
    return out


def arms_of(mv):
    """Arme einer Kreuzung: Einfahrten mit aehnlicher Richtung und Lage zusammenfassen"""
    arms = []
    for m in mv:
        for a in arms:
            if abs(norm180(a['h'] - m['h0'])) < 35 and math.dist(a['p'], m['start']) < 20:
                a['n'] += 1
                break
        else:
            arms.append(dict(h=m['h0'], p=m['start'], n=1))
    return arms


def study_map(d, omsi=DEFAULT_OMSI, bilder=None):
    t0 = time.time()
    r = analyse(d, omsi)
    s = r['summary']
    lanes = r['lanes']
    M = movements(lanes)
    sco = r['inhalte']._sco
    obj_file = {}
    for l in lanes:
        if l['src']['art'] == 'Objekt':
            obj_file[(l['src']['kachel'], l['src']['id'])] = l['src']['datei']

    kreuz = []                       # Objekte mit >= 3 Armen
    prio_typ = collections.Counter()  # (prio, typ, armzahl) -> n
    radien = collections.defaultdict(list)
    for k, mv in M.items():
        if len(mv) < 3:
            continue
        arms = arms_of(mv)
        if len(arms) < 3:
            continue
        info = sco.get(obj_file[k], {})
        has_prio = any(m['prio'] is not None for m in mv)
        kreuz.append(dict(key=k, datei=obj_file[k], arme=len(arms), bewegungen=len(mv), prio=has_prio,
                          ampeln=sum(1 for m in mv if m['ampel'] >= 0), helpers=info.get('helpers', 0),
                          gehwege=sum(1 for p in info.get('paths', []) if p['kind'] == 1),
                          pos=mv[0]['start']))
        for m in mv:
            prio_typ[(m['prio'] if m['prio'] is not None else 128, m['typ'], min(len(arms), 5))] += 1
            if m['typ'] in ('rechts', 'links'):
                for p in m['paths']:
                    src = sco.get(p['src']['datei'], {}).get('paths', [])
                    j = p['src']['pfad']
                    if j < len(src) and src[j]['R'] != 0 and abs(src[j]['L']) > 2:
                        radien[m['typ']].append(abs(src[j]['R']))

    # Kreuzungen aus Splines: Spurende, an dem >= 2 andere Spline-Spuren beginnen (Verzweigung ohne Objekt)
    starts = collections.defaultdict(list)
    for i, l in enumerate(lanes):
        if l['kind'] == 0 and l['src']['art'] == 'Spline' and l['d'] == 0:
            starts[(round(l['pts'][0][0], 1), round(l['pts'][0][1], 1))].append(i)
    spline_verzw = sum(1 for v in starts.values() if len(v) >= 2)

    # Ampelprogramme
    umlaeufe, phasen = [], collections.Counter()
    for k in {c['datei'] for c in kreuz}:
        info = sco.get(k, {})
        for a in info.get('ampeln', []):
            dur = sum(p[1] for p in a['phasen'])
            if dur > 0:
                umlaeufe.append(round(dur))
            for st, du in a['phasen']:
                if du > 0:
                    phasen[st] += 1

    # Genauigkeit der Anschluesse (Abstand der verbundenen Spurenden)
    gaps = sorted(e['naechster'][0] for e in r['ends'] if e['loose'] and lanes[e['lane']]['kind'] == 0)
    gap_q = {q: round(gaps[int(len(gaps) * q / 100) - 1], 3) for q in (50, 90, 99)} if gaps else {}

    # Strassen-Splines
    spl = collections.Counter(l['src']['datei'] for l in lanes if l['src']['art'] == 'Spline' and l['kind'] == 0)
    spl_n = collections.Counter()
    for t in r['m']['tiles']:
        if t[3]:
            for x in t[3]['splines']:
                spl_n[x['file']] += 1
    road_spl = [(f, spl_n[f]) for f, _ in spl.most_common()][:12]
    profiles = {}
    for f, _ in road_spl:
        info = r['inhalte'].sli(f)
        st = [p for p in info['paths'] if p['kind'] == 0]
        profiles[f] = dict(spuren=len(st), gegen=sum(1 for p in st if p['d'] == 1),
                           gehwege=sum(1 for p in info['paths'] if p['kind'] == 1), halbe_breite=round(info['half'], 2))

    arm_n = collections.Counter(c['arme'] for c in kreuz)
    res = dict(karte=s['karte'], ordner=d, kacheln=s['kacheln'], kachelgroesse=s['kachelgroesse'],
               splines=s['splines'], objekte=s['objekte'], pfade=s['pfade'],
               kreuzungsobjekte=len(kreuz), kreuzungen_aus_splines=spline_verzw,
               arme=dict(sorted(arm_n.items())), mit_vorfahrt=sum(c['prio'] for c in kreuz),
               mit_ampel=sum(1 for c in kreuz if c['ampeln']), mit_gehwegpfaden=sum(1 for c in kreuz if c['gehwege']),
               mit_splinehelper=sum(1 for c in kreuz if c['helpers']),
               vorfahrt_muster={f'{p}/{t}/{a}': n for (p, t, a), n in sorted(prio_typ.items())},
               radien={k: dict(n=len(v), median=round(statistics.median(v), 1),
                               p10=round(sorted(v)[len(v) // 10], 1), p90=round(sorted(v)[len(v) * 9 // 10], 1))
                       for k, v in radien.items() if v},
               ampel_umlauf=dict(n=len(umlaeufe), median=statistics.median(umlaeufe) if umlaeufe else None,
                                 haeufig=collections.Counter(umlaeufe).most_common(5)),
               ampel_phasen=dict(phasen), anschluss_abstand=gap_q,
               strassen_splines=[dict(datei=f, anzahl=n, **profiles[f]) for f, n in road_spl],
               fehlend=len(s['fehlende_dateien']), dauer_s=round(time.time() - t0, 1))
    if bilder and kreuz:
        os.makedirs(bilder, exist_ok=True)
        # Beispiel: Kreuzung mit 4 Armen, Vorfahrt und moeglichst Ampel; sonst die mit den meisten Bewegungen
        best = max(kreuz, key=lambda c: (c['arme'] == 4, c['prio'], c['ampeln'] > 0, c['bewegungen']))
        P = [p for m in M[best['key']] for l in m['paths'] for p in l['pts']]
        x0, x1 = min(p[0] for p in P), max(p[0] for p in P)
        z0, z1 = min(p[1] for p in P), max(p[1] for p in P)
        x, z = (x0 + x1) / 2, (z0 + z1) / 2
        hw = max(x1 - x0, z1 - z0) / 2 + 20
        name = os.path.basename(d).replace(' ', '_')
        res['beispiel'] = dict(datei=best['datei'], bild=os.path.join(bilder, f'{name}.png'))
        write_png(res['beispiel']['bild'], lanes, r['roads'], r['objs'], r['ends'], (x - hw, z - hw, x + hw, z + hw),
                  f"{s['karte']}: {best['datei'].split(chr(92))[-1]}")
    return res


def bericht(R):
    L = ['# Kartenstudie: wie OMSI-Karten gebaut sind', '',
         f'Automatisch erzeugt mit `python -m omsigen.studie` am {time.strftime("%d.%m.%Y")} aus {len(R)} Karten. '
         'Kreuzung = Objekt mit mindestens 3 Armen (Einfahrten). Vorfahrt: 192 = Vorfahrt, 64 = wartepflichtig, '
         '128 = keine Regel (rechts vor links). Nur lesend ausgewertet.', '',
         '## Ueberblick', '',
         '| Karte | Kacheln | Splines | Objekte | Kreuzungsobjekte | Arme 3/4/5+ | mit Vorfahrt | mit Ampel | '
         'Gehwegpfade | Verzweigungen aus Splines | Anschluss 50/90/99 % |',
         '|---|---:|---:|---:|---:|---|---:|---:|---:|---:|---|']
    for x in R:
        a = {int(k): v for k, v in x['arme'].items()}
        L.append(f"| {x['karte']} | {x['kacheln']} | {x['splines']} | {x['objekte']} | {x['kreuzungsobjekte']} | "
                 f"{a.get(3, 0)}/{a.get(4, 0)}/{sum(v for k, v in a.items() if k >= 5)} | {x['mit_vorfahrt']} | "
                 f"{x['mit_ampel']} | {x['mit_gehwegpfaden']} | {x['kreuzungen_aus_splines']} | "
                 f"{'/'.join(str(v) for v in x['anschluss_abstand'].values())} m |")
    # Vorfahrtsmuster ueber alle Karten
    tot = collections.Counter()
    for x in R:
        for k, n in x['vorfahrt_muster'].items():
            p, t, a = k.split('/')
            tot[(int(float(p)), t)] += n
    L += ['', '## Vorfahrt: welche Bewegungen bekommen welche Prioritaet (alle Karten)', '',
          '| Bewegung | 192 | 128 (keine) | 64 |', '|---|---:|---:|---:|']
    for t in ('gerade', 'rechts', 'links', 'wenden'):
        row = [tot[(p, t)] for p in (192, 128, 64)]
        if sum(row):
            L.append(f'| {t} | ' + ' | '.join(f'{v} ({100 * v / sum(row):.0f} %)' for v in row) + ' |')
    rad = collections.defaultdict(list)
    for x in R:
        for k, v in x['radien'].items():
            rad[k].append((v['median'], v['n']))
    L += ['', '## Abbiegeradien in Kreuzungsobjekten (Median je Karte)', '']
    for k in ('rechts', 'links'):
        if rad[k]:
            L.append(f"- {k}: " + ', '.join(f'{m} m' for m, _ in rad[k]) +
                     f" -> gewichteter Mittelwert {sum(m * n for m, n in rad[k]) / sum(n for _, n in rad[k]):.1f} m")
    L += ['', '## Ampeln', '']
    for x in R:
        u = x['ampel_umlauf']
        if u['n']:
            L.append(f"- {x['karte']}: {u['n']} Signalgruppen, Umlauf Median {u['median']} s, haeufig "
                     + ', '.join(f'{d} s ({n}x)' for d, n in u['haeufig']))
    L += ['', '## Haeufigste Strassen-Splines je Karte', '']
    for x in R:
        if x['strassen_splines']:
            L.append(f"- **{x['karte']}**: " + '; '.join(
                f"`{s['datei'].split(chr(92))[-1]}` {s['anzahl']}x ({s['spuren']} Spuren, {s['gehwege']} Gehwege, "
                f"halbe Breite {s['halbe_breite']} m)" for s in x['strassen_splines'][:4]))
    L += ['', '## Beispielkreuzungen', '']
    for x in R:
        if x.get('beispiel'):
            L.append(f"- {x['karte']}: `{x['beispiel']['datei']}` -> `{x['beispiel']['bild']}`")
    return '\n'.join(L) + '\n'


def main(argv=None):
    ap = argparse.ArgumentParser(prog='omsigen.studie', description='Vorhandene OMSI-Karten auswerten')
    ap.add_argument('karten', nargs='*', help='Kartenordner oder Namen unter <OMSI>/maps')
    ap.add_argument('--alle', action='store_true', help='alle installierten Karten (OMSI und openOMSI)')
    ap.add_argument('--omsi', default=DEFAULT_OMSI)
    ap.add_argument('--bericht', help='Markdown-Bericht')
    ap.add_argument('--json', help='Daten als JSON')
    ap.add_argument('--bilder', help='Ordner fuer Beispielbilder')
    a = ap.parse_args(argv)
    dirs = all_maps(a.omsi) if a.alle else [k if os.path.isdir(k) else os.path.join(a.omsi, 'maps', k) for k in a.karten]
    R = []
    for d in dirs:
        try:
            x = study_map(d, a.omsi, a.bilder)
        except Exception as ex:      # eine kaputte Karte soll die Studie nicht abbrechen
            print(f'{os.path.basename(d)}: Fehler {type(ex).__name__}: {ex}')
            continue
        R.append(x)
        print(f"{x['karte']}: {x['kreuzungsobjekte']} Kreuzungsobjekte {x['arme']}, Vorfahrt {x['mit_vorfahrt']}, "
              f"Ampel {x['mit_ampel']}, Spline-Verzweigungen {x['kreuzungen_aus_splines']} ({x['dauer_s']} s)")
    if a.json:
        with open(a.json, 'w', encoding='utf-8') as f:
            json.dump(R, f, ensure_ascii=False, indent=1)
    if a.bericht:
        with open(a.bericht, 'w', encoding='utf-8') as f:
            f.write(bericht(R))
        print('Bericht:', a.bericht)
    return 0


if __name__ == '__main__':
    sys.exit(main())
