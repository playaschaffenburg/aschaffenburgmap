"""Pruefbericht einer Karte (Qualitaetsplan F8): je Kachel alle Funde an einer Stelle, dazu Bilder an festen Prueforten.

Gesammelt wird, was die Werkzeuge der Fundament-Phase liefern:
  - Strassen: Spurenden ohne Anschluss und Beinahe-Anschluesse (omsigen.ansicht; gewollte Enden am Kartenrand und in
    Sackgassen zaehlen nicht)
  - Schichten: veraltete oder gebaute, aber nicht eingesetzte Pakete (omsigen.schichten)
  - Ableitungen: netz.json und Belegung passen nicht mehr zur Karte (omsigen.netz_datei, omsigen.belegung)
  - Platzierung: Gebaeude ueber Fahrbahn/Kreuzung mit Klasse und Hinweis, ueber Gehweg (Belegung); Gebaeude, die
    ueber dem Gelaende schweben oder im Gelaende versinken (G1, omsigen.platzierung)
  - Boden (S4, omsigen.boden): Kacheln ohne Malmasken, Fahrbahn auf Gras/Erde (Masken passen nicht zur Belegung)
  - Budgets je Kachel (F6, vorlaeufig): Objekte, Dreiecke, Texturspeicher
  - Querschnitte (S1, omsigen.querschnitt_pruefung): Pruefstellen der Kartendaten, an denen Fahrbahn- oder Gehwegbreite
    0,5 m oder mehr vom Luftbild abweichen; Strassenzuege mit weniger Spuren als OSM turn:lanes angibt
Prueforte (maps/<Karte>/pruefpunkte.json, sonst die Haltestellen der Karte) bekommen auf Wunsch ein Spielbild aus
openOMSI (--bilder) und einen Luftbildvergleich von oben (--luftbild).

Ausgabe: maps/<Karte>/pruefung/bericht.json und bericht.html (Bilder daneben). Ziel: 0 Funde.

  python -m omsigen.pruefbericht <Karte> [--omsi <OMSI>] [--bilder] [--luftbild] [--max 6]
"""
import argparse
import datetime
import html
import json
import math
import os

from . import schichten

KACHEL = 300.0
BUDGET = dict(objekte=600, dreiecke=300_000, textur_mb=64.0)   # je Kachel, vorlaeufig (F6)
RAND = 20.0              # m: Spurende so nah an einem Kartenrand ohne Nachbarkachel = gewolltes Ende
ORDNER = 'pruefung'


# ---------------------------------------------------------------- Budgets

class _Modelle:
    """.sco -> (Dreiecke, {Texturdatei: Bytes im Grafikspeicher}) mit Zwischenspeicher"""

    def __init__(self, omsi):
        self.omsi = omsi
        self.cache = {}

    def __call__(self, rel):
        if rel in self.cache:
            return self.cache[rel]
        from .belegung import x_lesen
        sco = os.path.join(self.omsi, *rel.split(chr(92)))
        dreiecke, tex = 0, {}
        try:
            zeilen = [z.strip() for z in open(sco, encoding='cp1252', errors='replace')]
        except OSError:
            self.cache[rel] = (0, {})
            return self.cache[rel]
        for i, z in enumerate(zeilen):
            if z != '[mesh]':
                continue
            datei = next((w for w in zeilen[i + 1:] if w), None)
            if not datei:
                continue
            x = os.path.join(os.path.dirname(sco), 'model', datei)
            if not os.path.isfile(x) or not x.lower().endswith('.x'):
                continue
            try:
                _, F, _, texnamen = x_lesen(x)
            except (ValueError, IndexError):
                continue
            dreiecke += len(F)
            for t in texnamen:
                p = os.path.join(os.path.dirname(sco), 'texture', t)
                if p not in tex:
                    tex[p] = _gpu_bytes(p)
        self.cache[rel] = (dreiecke, tex)
        return self.cache[rel]


def _gpu_bytes(p):
    """Grafikspeicher einer Textur: DDS wie auf der Platte, sonst RGBA mit Mip-Stufen (obere Schaetzung)"""
    stamm = os.path.splitext(p)[0]
    if os.path.isfile(stamm + '.dds'):           # OMSI nimmt eine gleichnamige DDS zuerst
        return os.path.getsize(stamm + '.dds')
    if not os.path.isfile(p):
        return 0
    if p.lower().endswith('.dds'):
        return os.path.getsize(p)
    try:
        from PIL import Image
        w, h = Image.open(p).size
        return int(w * h * 4 * 4 / 3)
    except Exception:
        return os.path.getsize(p)


def budgets(kartenordner, omsi):
    modelle = _Modelle(omsi)
    out = {}
    for k in schichten.kacheln(kartenordner):
        _, bl = schichten.bloecke(schichten.lesen(kartenordner, k))
        n_obj = n_spl = dr = 0
        tex = {}
        for b in bl:
            kopf = b[0].strip()
            if kopf in ('[spline]', '[spline_h]'):
                n_spl += 1
            elif kopf == '[object]':
                n_obj += 1
                d, t = modelle(schichten.objekt_pfad(b))
                dr += d
                tex.update(t)
        out[f'{k[0]}_{k[1]}'] = dict(objekte=n_obj, splines=n_spl, dreiecke=dr,
                                     textur_mb=round(sum(tex.values()) / 2 ** 20, 1))
    return out


# ---------------------------------------------------------------- Strassen

def strassen_funde(kartenordner, omsi):
    """-> Liste Funde (art, x, z, text) fuer Spurenden ohne Anschluss und Beinahe-Anschluesse"""
    from . import ansicht
    r = ansicht.analyse(kartenordner, omsi)
    kacheln = set(schichten.kacheln(kartenordner))

    def am_rand(p):
        k = (math.floor(p[0] / KACHEL), math.floor(p[1] / KACHEL))
        lx, lz = p[0] - k[0] * KACHEL, p[1] - k[1] * KACHEL
        return ((lx < RAND and (k[0] - 1, k[1]) not in kacheln) or (lx > KACHEL - RAND and (k[0] + 1, k[1]) not in kacheln)
                or (lz < RAND and (k[0], k[1] - 1) not in kacheln) or (lz > KACHEL - RAND and (k[0], k[1] + 1) not in kacheln))
    rand = [e['p'] for e in r['ends'] if not e['strict'] and am_rand(e['p'])]
    meta = os.path.join(kartenordner, 'omsigen.json')        # Generator: wo der Korridor Strassen abschneidet
    if os.path.isfile(meta):
        rand += [tuple(p) for p in json.load(open(meta, encoding='utf-8')).get('rand', [])]
    offen, _ = ansicht.classify(r['lanes'], r['ends'], edge_points=rand)
    funde = []
    for e in offen:
        src = r['lanes'][e['lane']]['src']
        art = 'beinahe_anschluss' if e['naechster'] is not None else 'spurende_offen'
        funde.append(dict(art=art, x=round(e['p'][0], 1), z=round(e['p'][1], 1),
                          text=f'{src["art"]} {src["id"]} Pfad {src["pfad"]}' + (f' ({os.path.basename(src.get("datei", "").replace(chr(92), "/"))})' if src.get('datei') else '')))
    return funde, r['summary']


# ---------------------------------------------------------------- Boden

BODEN_M2 = 5.0           # Fahrbahn auf Gras/Erde ab dieser Flaeche je Kachel melden


def boden_funde(kartenordner, bel):
    """Funde der Schicht boden (nur Karten mit boden.json): Kacheln ohne Malmasken, Fahrbahn/Kreuzung der Belegung, die in
    den Masken als Gras, Erde oder Grund erscheint (ohne den 1-m-Rand, der weich in die Nachbarschaft uebergeht)"""
    import numpy as np
    from . import belegung, boden
    if not os.path.isfile(os.path.join(kartenordner, boden.DATEI)):
        return []
    from scipy import ndimage
    funde = []
    ix = schichten.index_lesen(kartenordner)['kacheln']
    for k in schichten.kacheln(kartenordner):
        mitte = (k[0] * KACHEL + 150, k[1] * KACHEL + 150)
        if 'boden' not in ix.get(f'{k[0]}_{k[1]}', {}):
            funde.append(dict(art='boden_fehlt', x=mitte[0], z=mitte[1], text='Boden nicht gebaut (python -m omsigen.boden)'))
            continue
        kl = boden.kachel_klassen(kartenordner, k)
        if kl is None:                       # gebaut, aber ohne Masken: ueberall der Grund
            kl = np.zeros((boden.MASKE, boden.MASKE), np.uint8)
        r = bel.raster(k) if bel is not None else None
        if r is None:
            continue
        strasse = np.isin(r, (belegung.FAHRBAHN, belegung.KREUZUNG))
        strasse = ndimage.binary_erosion(strasse, iterations=2)
        n = kl.shape[0]
        idx = (np.arange(n) * belegung.N // n).astype(int)
        s = strasse[np.ix_(idx, idx)]
        falsch = s & np.isin(kl, (boden.GRUND, boden.GRAS, boden.ERDE))
        m2 = float(falsch.sum()) * (KACHEL / n) ** 2
        if m2 >= BODEN_M2:
            jj, ii = np.nonzero(falsch)
            x = k[0] * KACHEL + (ii.mean() + 0.5) * KACHEL / n
            z = k[1] * KACHEL + KACHEL - (jj.mean() + 0.5) * KACHEL / n
            funde.append(dict(art='boden_fahrbahn_gras', x=round(x, 1), z=round(z, 1),
                              text=f'{m2:.0f} m2 Fahrbahn auf Gras/Erde - Boden neu bauen'))
    return funde


# ---------------------------------------------------------------- Prueforte und Bilder

def prueforte(kartenordner):
    """maps/<Karte>/pruefpunkte.json [{"name", "x", "z", "cam" (optional: "x,y,z,gier,neigung")}] oder die Haltestellen"""
    p = os.path.join(kartenordner, 'pruefpunkte.json')
    if os.path.isfile(p):
        return json.load(open(p, encoding='utf-8'))
    return haltestellen(kartenordner)


def auswahl(orte, n):
    """n gleichmaessig verteilte Prueforte (in Reihenfolge der Liste)"""
    if len(orte) <= n:
        return orte
    return [orte[round(i * (len(orte) - 1) / max(1, n - 1))] for i in range(n)]


def haltestellen(kartenordner):
    """Haltestellenschilder der Karte (Generic/bus_stop.sco, Name = erste Beschriftung) -> [{"name", "x", "z"}]"""
    out, namen = [], set()
    for k in schichten.kacheln(kartenordner):
        _, bl = schichten.bloecke(schichten.lesen(kartenordner, k))
        for b in bl:
            if b[0].strip() == '[object]' and schichten.objekt_pfad(b).lower().endswith('bus_stop.sco') and len(b) > 11:
                name = b[11].strip()
                if name and name not in namen:
                    namen.add(name)
                    out.append(dict(name=name, x=round(float(b[4]) + k[0] * KACHEL, 1), z=round(float(b[5]) + k[1] * KACHEL, 1)))
    return out


def kamera(kartenordner, ort):
    """Kamera 35 m vor dem Ort entlang der naechsten laengeren Strasse (sonst dahinter), 14 m hoch, Blick die Strasse
    entlang zum Ort
    -> 'x,y,z,gier,neigung'"""
    from .netz_datei import Netz
    try:
        n = Netz.laden(kartenordner, pruefen=False)
        e, st, _, _ = n.naechste(ort['x'], ort['z'], min_laenge=40.0)
        _, y, _, h = n.punkt(e, st)
        st_k = st - 35.0 if st >= 35.0 else min(n.kanten[e]['laenge'], st + 35.0)
        cx, _, cz, hk = n.punkt(e, st_k)
        # entlang der Strasse blicken (in Kurven zielt die gerade Linie zum Ort sonst durch die Haeuser)
        gier = hk if st_k <= st else (hk + 180.0) % 360
        return f'{cx:.1f},{cz:.1f},{y + 14:.1f},{gier:.1f},-18'
    except Exception:
        return f'{ort["x"] - 30:.1f},{ort["z"] - 30:.1f},40,45,-25'


def bilder(kartenordner, omsi, orte, aus, mit_spiel, mit_luft, log=print):
    from . import openomsi
    karte = os.path.basename(os.path.normpath(kartenordner))
    for i, o in enumerate(orte):
        stamm = f'ort_{i:02d}'
        if mit_spiel:
            cam = o.get('cam') or kamera(kartenordner, o)      # eigene Kamera in pruefpunkte.json: "x,y,z,gier,neigung"
            png = os.path.join(aus, stamm + '_spiel.png')
            try:
                r = openomsi.render(karte, png, omsi=omsi, cam=cam, timeout=300)
                o['spiel'] = os.path.basename(png) if r['ok'] else None
            except Exception as e:
                o['spiel'], o['fehler'] = None, f'{type(e).__name__}: {e}'
            log(f'    Spielbild {o["name"]}: {"ok" if o.get("spiel") else "fehlgeschlagen"}')
        if mit_luft and os.path.isfile(os.path.join(kartenordner, 'omsi-editor-geo.cfg')):
            import subprocess
            import sys
            ziel = os.path.join(aus, stamm)
            p = subprocess.run([sys.executable, '-m', 'omsigen.luftbildvergleich', '--omsi', omsi, '--karte', karte,
                                '--bereich', str(o['x'] - 60), str(o['z'] - 60), str(o['x'] + 60), str(o['z'] + 60),
                                '--aus', aus, '--name', stamm + '_luft'], capture_output=True, text=True, timeout=600,
                               cwd=os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
            ok = os.path.isfile(ziel + '_luft_ueberblendet.jpg')
            o['luft'] = os.path.basename(ziel + '_luft_ueberblendet.jpg') if ok else None
            log(f'    Luftbild {o["name"]}: {"ok" if ok else "fehlgeschlagen"}')


# ---------------------------------------------------------------- Bericht

def erstellen(kartenordner, omsi=None, mit_spiel=False, mit_luft=False, max_orte=6, log=print):
    omsi = omsi or schichten.STANDARD_OMSI
    aus = os.path.join(kartenordner, ORDNER)
    os.makedirs(aus, exist_ok=True)
    funde = []
    # Strassen
    s_funde, summary = strassen_funde(kartenordner, omsi)
    funde += s_funde
    # Schichten
    for key, zeile in schichten.status(kartenordner).items():
        kx, ky = map(int, key.split('_'))
        for s, z in zeile.items():
            if z['zustand'] in ('veraltet', 'gebaut'):
                funde.append(dict(art=f'schicht_{z["zustand"]}', x=kx * KACHEL + 150, z=ky * KACHEL + 150,
                                  text=f'{s}: {z["grund"]}'))
    # Ableitungen
    from . import belegung, netz_datei
    netz_alt = bel_alt = None
    try:
        netz_alt = netz_datei.Netz.laden(kartenordner, pruefen=False).veraltet(kartenordner)
    except OSError:
        funde.append(dict(art='netz_fehlt', x=None, z=None, text='netz.json fehlt (python -m omsigen.netz_datei)'))
    for key in netz_alt or []:
        kx, ky = map(int, key.split('_'))
        funde.append(dict(art='netz_veraltet', x=kx * KACHEL + 150, z=ky * KACHEL + 150, text='netz.json passt nicht zur Kachel'))
    bel = None
    try:
        bel = belegung.Belegung.laden(kartenordner, pruefen=False)
        bel_alt = bel.veraltet()
    except OSError:
        funde.append(dict(art='belegung_fehlt', x=None, z=None, text='Belegung fehlt (python -m omsigen.belegung)'))
    for key in bel_alt or []:
        kx, ky = map(int, key.split('_'))
        funde.append(dict(art='belegung_veraltet', x=kx * KACHEL + 150, z=ky * KACHEL + 150, text='Belegung passt nicht zur Kachel'))
    if bel:
        objpos = {}
        for k in schichten.kacheln(kartenordner):
            _, bl = schichten.bloecke(schichten.lesen(kartenordner, k))
            for b in bl:
                if b[0].strip() == '[object]':
                    try:
                        objpos[schichten.objekt_pfad(b)] = (float(b[4]) + k[0] * KACHEL, float(b[5]) + k[1] * KACHEL)
                    except (ValueError, IndexError):
                        pass
        for c in bel.daten.get('konflikte', []):
            x, z = objpos.get(c['objekt'], (None, None))
            funde.append(dict(art=c['art'], x=x and round(x, 1), z=z and round(z, 1), klasse=c.get('klasse', ''),
                              text=f'{os.path.basename(c["objekt"].replace(chr(92), "/"))}: {c["flaeche_m2"]} m2'
                              + (f' - {c["hinweis"]}' if c.get('hinweis') else '')))
    # Gebaeude gegen Gelaende (G1): schwebt / versunken
    try:
        from . import platzierung
        funde += platzierung.hoehen_funde(kartenordner, omsi, log)
    except Exception as e:
        log(f'    Gebaeudehoehen nicht geprueft: {type(e).__name__}: {e}')
    # Boden (S4)
    funde += boden_funde(kartenordner, bel)
    # Querschnitte (S1): Pruefstellen gegen das Luftbild, Abbiegestreifen nach turn:lanes
    try:
        from . import querschnitt_pruefung
        funde += querschnitt_pruefung.funde(kartenordner)
    except Exception as e:                       # Luftbild nicht erreichbar o. ae.: der Bericht entsteht trotzdem
        log(f'    Querschnitte nicht geprueft: {type(e).__name__}: {e}')
    # Budgets
    bud = budgets(kartenordner, omsi)
    for key, b in bud.items():
        kx, ky = map(int, key.split('_'))
        for g, grenze in BUDGET.items():
            if b[g] > grenze:
                funde.append(dict(art=f'budget_{g}', x=kx * KACHEL + 150, z=ky * KACHEL + 150, text=f'{b[g]} > {grenze}'))
    # je Kachel
    for f in funde:
        f['kachel'] = None if f['x'] is None else f'{math.floor(f["x"] / KACHEL)}_{math.floor(f["z"] / KACHEL)}'
    je = {}
    for f in funde:
        if f['kachel']:
            je.setdefault(f['kachel'], {}).setdefault(f['art'], 0)
            je[f['kachel']][f['art']] += 1
    # Prueforte
    orte = auswahl(prueforte(kartenordner), max_orte) if (mit_spiel or mit_luft) else prueforte(kartenordner)
    if mit_spiel or mit_luft:
        bilder(kartenordner, omsi, orte, aus, mit_spiel, mit_luft, log)
    arten = {}
    for f in funde:
        arten[f['art']] = arten.get(f['art'], 0) + 1
    d = dict(format='omsigen-pruefbericht', version=1, karte=os.path.basename(os.path.normpath(kartenordner)),
             erstellt=datetime.datetime.now().isoformat(timespec='seconds'), funde_gesamt=len(funde), arten=arten,
             kacheln=je, budgets=bud, budget_grenzen=BUDGET, funde=funde, prueforte=orte,
             strassen=dict(splines=summary['splines'], objekte=summary['objekte'], enden=summary['Strasse_enden']))
    with open(os.path.join(aus, 'bericht.json'), 'w', encoding='utf-8') as f:
        json.dump(d, f, ensure_ascii=False, indent=1)
    with open(os.path.join(aus, 'bericht.html'), 'w', encoding='utf-8') as f:
        f.write(_html(d))
    return d


NAMEN = {'spurende_offen': 'Spurende ohne Anschluss', 'beinahe_anschluss': 'Beinahe-Anschluss',
         'schicht_veraltet': 'Schicht veraltet', 'schicht_gebaut': 'Schicht nicht eingesetzt',
         'netz_veraltet': 'netz.json veraltet', 'netz_fehlt': 'netz.json fehlt', 'belegung_veraltet': 'Belegung veraltet',
         'belegung_fehlt': 'Belegung fehlt', 'gebaeude_auf_fahrbahn': 'Gebaeude ueber Fahrbahn',
         'gebaeude_auf_gehweg': 'Gebaeude ueber Gehweg', 'gebaeude_schwebt': 'Gebaeude schwebt',
         'gebaeude_versunken': 'Gebaeude versunken',
         'boden_fehlt': 'Boden fehlt', 'boden_fahrbahn_gras': 'Fahrbahn auf Gras (Boden)',
         'budget_objekte': 'Budget Objekte', 'budget_dreiecke': 'Budget Dreiecke', 'budget_textur_mb': 'Budget Texturspeicher',
         'querschnitt_abweichung': 'Querschnitt weicht vom Luftbild ab', 'abbiegestreifen_fehlt': 'Abbiegestreifen fehlt'}


def _html(d):
    e = html.escape
    zeilen = [f'<!doctype html><html lang="de"><head><meta charset="utf-8"><title>Pruefbericht {e(d["karte"])}</title>',
              '<style>body{font-family:system-ui,sans-serif;margin:16px;max-width:1200px}table{border-collapse:collapse}'
              'td,th{border:1px solid #ccc;padding:3px 8px;text-align:left}th{background:#eee}.ok{color:#2a7}.f{color:#c40}'
              'img{max-width:560px;margin:4px;border:1px solid #ccc}</style></head><body>',
              f'<h1>Pruefbericht {e(d["karte"])}</h1><p>erstellt {e(d["erstellt"])} &middot; '
              f'<b class="{"ok" if not d["funde_gesamt"] else "f"}">{d["funde_gesamt"]} Funde</b></p>',
              '<h2>Funde nach Art</h2><table><tr><th>Art</th><th>Anzahl</th></tr>']
    zeilen += [f'<tr><td>{e(NAMEN.get(a, a))}</td><td>{n}</td></tr>' for a, n in sorted(d['arten'].items(), key=lambda x: -x[1])]
    zeilen += ['</table><h2>Kacheln mit Funden</h2><table><tr><th>Kachel</th><th>Funde</th><th>Objekte</th><th>Dreiecke</th><th>Textur MB</th></tr>']
    for key, arten in sorted(d['kacheln'].items(), key=lambda x: -sum(x[1].values())):
        b = d['budgets'].get(key, {})
        zeilen.append(f'<tr><td>{key}</td><td>{e(", ".join(f"{NAMEN.get(a, a)} {n}" for a, n in arten.items()))}</td>'
                      f'<td>{b.get("objekte", "")}</td><td>{b.get("dreiecke", "")}</td><td>{b.get("textur_mb", "")}</td></tr>')
    zeilen += ['</table><h2>Alle Funde</h2><table><tr><th>Art</th><th>Kachel</th><th>x</th><th>z</th><th>Hinweis</th></tr>']
    for f in d['funde']:
        zeilen.append(f'<tr><td>{e(NAMEN.get(f["art"], f["art"]))}</td><td>{f["kachel"] or ""}</td><td>{f["x"] if f["x"] is not None else ""}</td>'
                      f'<td>{f["z"] if f["z"] is not None else ""}</td><td>{e(f["text"])}</td></tr>')
    zeilen.append('</table><h2>Prueforte</h2>')
    for o in d['prueforte']:
        zeilen.append(f'<h3>{e(o["name"])} <small>({o["x"]}, {o["z"]})</small></h3>')
        for schl in ('spiel', 'luft'):
            if o.get(schl):
                zeilen.append(f'<img src="{e(o[schl])}" alt="{schl}">')
    zeilen.append('</body></html>')
    return '\n'.join(zeilen)


def main():
    ap = argparse.ArgumentParser(description='Pruefbericht einer Karte')
    ap.add_argument('karte')
    ap.add_argument('--omsi', default=schichten.STANDARD_OMSI)
    ap.add_argument('--bilder', action='store_true', help='Spielbilder (openOMSI) an den Prueforten')
    ap.add_argument('--luftbild', action='store_true', help='Luftbildvergleich an den Prueforten')
    ap.add_argument('--max', type=int, default=6, help='hoechstens so viele Prueforte mit Bildern')
    a = ap.parse_args()
    d = a.karte if os.path.isdir(a.karte) else os.path.join(a.omsi, 'maps', a.karte)
    erg = erstellen(d, a.omsi, a.bilder, a.luftbild, a.max)
    print(json.dumps(dict(bericht=os.path.join(d, ORDNER, 'bericht.html'), funde=erg['funde_gesamt'], arten=erg['arten'],
                          kacheln_mit_funden=len(erg['kacheln'])), ensure_ascii=False))


if __name__ == '__main__':
    main()
