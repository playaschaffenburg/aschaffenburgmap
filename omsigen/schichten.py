"""Schichten einer Karte (Qualitaetsplan F2, docs/qualitaet-anforderungen.md): jede Schicht wird getrennt erzeugt und je
Kachel eingesetzt; erneutes Einsetzen ersetzt nur ihre eigenen Eintraege. ("Schicht" statt "Ebene": Ebene heisst in
omsigen schon Boden/Bruecke/Tunnel, siehe ebenen.py.)

Reihenfolge (eine Schicht liest alle davor, aendert sie nie):
  netz      Strassen-Splines und Kreuzungsobjekte (K_*)         - die Karte selbst, kein Paket
  boden     Gelaende (.terrain) und Malmasken (texture/map)      - die Karte selbst, kein Paket
  gebaeude  LoD2-Gebaeude          Ordner LoD2\\<kx>_<ky>\\
  objekte   Laternen, Schilder ... Ordner Objekte\\<kx>_<ky>\\
  gruen     Baeume, Hecken, Parks  Ordner Gruen\\<kx>_<ky>\\
  backdrop  Kartenrand             Ordner Backdrop\\<kx>_<ky>\\

Paket (je Schicht und Kachel eine JSON-Datei neben den Objekten, Sonderbau-Format wie lod2.json):
  {"format": "omsigen-schicht", "version": 1, "schicht": "gebaeude", "karte": ..., "kachel": [kx, ky],
   "marker": "\\LoD2\\<kx>_<ky>\\", "objekte": [{"name", "rel", "x", "y", "z", "rot" (Grad, optional),
   "texte" (Beschriftungen, optional)}], "strassen": [], "kreuzungen": [], "gelaende": [], "gelaende_zurueck": [],
   "quellen": [...], "manifest": {...}}
  Einsetzen (Rust: sonderbau::einsetzen_nur, Python: einsetzen) entfernt alle [object] der Kachelumgebung, deren Pfad den
  Marker enthaelt (dazu ihre [attachObj]), und traegt die neuen ein.

Verzeichnis der Karte maps/<Karte>/schichten.json: je Kachel und Schicht, wann und gegen welchen Stand der Schichten
davor (Fingerabdruecke) ein Paket gebaut wurde. status() vergleicht das mit den Dateien der Karte: aktuell, veraltet
(eine fruehere Schicht der Kachel hat sich seitdem geaendert), gebaut (Paket da, aber nicht in der gespeicherten Karte)
oder fehlt. Von Hand gesetzte Objekte haben keinen Marker und werden nie ersetzt.

  python -m omsigen.schichten <Karte> [--omsi <OMSI-Ordner>] [--json]
"""
import argparse
import datetime
import glob
import hashlib
import json
import os
import re
import sys

BS = chr(92)
STANDARD_OMSI = r'C:\Program Files (x86)\Steam\steamapps\common\OMSI 2'
FORMAT, VERSION = 'omsigen-schicht', 1
# Name, Objektordner (None: Schicht ist die Karte selbst)
SCHICHTEN = [('netz', None), ('boden', None), ('gebaeude', 'LoD2'), ('objekte', 'Objekte'), ('gruen', 'Gruen'),
             ('backdrop', 'Backdrop')]
NAMEN = [s for s, _ in SCHICHTEN]
ORDNER = dict(SCHICHTEN)
INDEX = 'schichten.json'


def vorher(schicht):
    """die Schichten, von denen `schicht` abhaengt (alle davor)"""
    return NAMEN[:NAMEN.index(schicht)]


def marker(schicht, kachel):
    """Kennung im Objektpfad: \\LoD2\\3_7\\ (dieselbe wie bisher bei LoD2)"""
    return BS + ORDNER[schicht] + BS + f'{kachel[0]}_{kachel[1]}' + BS


def schicht_von(rel):
    """Schicht und Kachel eines Objektpfads oder None (Netz-Objekte und Handarbeit haben keinen Marker)"""
    for s, o in SCHICHTEN:
        if o:
            m = re.search(re.escape(BS + o + BS) + r'(-?\d+)_(-?\d+)' + re.escape(BS), rel)
            if m:
                return s, (int(m.group(1)), int(m.group(2)))
    return None


# ---------------------------------------------------------------- Kacheldateien lesen/schreiben

def kachel_datei(kartenordner, k):
    return os.path.join(kartenordner, f'tile_{k[0]}_{k[1]}.map')


def kacheln(kartenordner):
    out = []
    for f in glob.glob(os.path.join(kartenordner, 'tile_*.map')):
        m = re.match(r'tile_(-?\d+)_(-?\d+)\.map$', os.path.basename(f))
        if m:
            out.append((int(m.group(1)), int(m.group(2))))
    return sorted(out)


def bloecke(text):
    """Kachel -> (Kopf, [Block]): ein Block beginnt mit einer [..]-Zeile; 'Object Nr. n'-Zeilen gehoeren zum Block danach"""
    zeilen = text.replace('\r\n', '\n').split('\n')
    kopf, out, akt = [], [], None
    for z in zeilen:
        if z.startswith('[') and z.rstrip().endswith(']'):
            if z.strip() in ('[version]', '[terrain]', '[variable_terrainlightmap]', '[variable_terrain]') and not out:
                kopf.append(z)
                continue
            akt = [z]
            out.append(akt)
        elif akt is None:
            kopf.append(z)
        else:
            akt.append(z)
    return kopf, out


def objekt_pfad(block):
    return block[2].strip() if block and block[0].strip() in ('[object]', '[splineAttachement]') and len(block) > 2 else ''


def lesen(kartenordner, k):
    with open(kachel_datei(kartenordner, k), encoding='utf-16') as f:
        return f.read()


def schreiben(kartenordner, k, text):
    with open(kachel_datei(kartenordner, k), 'w', encoding='utf-16', newline='') as f:
        f.write(text.replace('\r\n', '\n').replace('\n', '\r\n'))


# ---------------------------------------------------------------- Fingerabdruecke

def _hash(teile):
    h = hashlib.sha1()
    for t in teile:
        h.update(t if isinstance(t, bytes) else str(t).encode('utf-8'))
        h.update(b'\0')
    return h.hexdigest()[:16]


def _ohne_id(block):
    """Block ohne Objekt-ID (Zeile 4) und ohne 'Object Nr.'-Zaehlzeilen: Einsetzen anderer Objekte (neue IDs, neue
    Nummern) aendert den Inhalt nicht"""
    return [z.strip() for i, z in enumerate(block) if not (block[0].strip() == '[object]' and i == 3)
            and not z.startswith('Object Nr.') and z.strip()]


def fingerabdruck(kartenordner, k, schicht):
    """Inhalt einer Schicht in einer Kachel der gespeicherten Karte"""
    if schicht == 'boden':
        teile = []
        for f in [kachel_datei(kartenordner, k) + '.terrain'] + sorted(glob.glob(os.path.join(
                kartenordner, 'texture', 'map', f'tile_{k[0]}_{k[1]}.map.*'))):
            if os.path.isfile(f):
                teile += [os.path.basename(f), open(f, 'rb').read()]
        return _hash(teile)
    if not os.path.isfile(kachel_datei(kartenordner, k)):
        return None
    _, bl = bloecke(lesen(kartenordner, k))
    if schicht == 'netz':
        # Splines, Kreuzungs- und Uebergangsobjekte (K_*.sco, UE_*.sco) - nicht Haltestellen, Einsetzpunkte, Handarbeit
        teil = [b for b in bl if b[0].strip() == '[spline]' or b[0].strip().startswith('[spline_h')
                or (b[0].strip() == '[object]' and re.search(r'(^|\\)(K|UE)_[^\\]*\.sco$', objekt_pfad(b), re.I))]
    else:
        mk = marker(schicht, k)
        teil = [b for b in bl if mk in objekt_pfad(b)]
        if not teil:
            return None
    return _hash(['\n'.join(_ohne_id(b)).strip() for b in teil])


# ---------------------------------------------------------------- Verzeichnis

def index_lesen(kartenordner):
    p = os.path.join(kartenordner, INDEX)
    if not os.path.isfile(p):
        return dict(format=FORMAT, version=VERSION, kacheln={})
    with open(p, encoding='utf-8') as f:
        return json.load(f)


def index_schreiben(kartenordner, ix):
    p = os.path.join(kartenordner, INDEX)
    with open(p + '.tmp', 'w', encoding='utf-8') as f:
        json.dump(ix, f, ensure_ascii=False, indent=1)
    os.replace(p + '.tmp', p)


def manifest(kartenordner, schicht, kachel, werkzeug, parameter=None, quellen=()):
    """Manifest eines neuen Pakets: Werkzeug, Parameter, Zeit und die Fingerabdruecke der Schichten davor"""
    return dict(schicht=schicht, kachel=list(kachel), werkzeug=werkzeug, parameter=parameter or {},
                quellen=list(quellen), erzeugt=datetime.datetime.now().isoformat(timespec='seconds'),
                eingaben={s: fingerabdruck(kartenordner, kachel, s) for s in vorher(schicht)})


def vermerken(kartenordner, paket_pfad, paket):
    """ein gebautes Paket ins Verzeichnis der Karte eintragen"""
    ix = index_lesen(kartenordner)
    k = f'{paket["kachel"][0]}_{paket["kachel"][1]}'
    m = dict(paket['manifest'])
    m['paket'] = paket_pfad
    m['inhalt'] = _hash([json.dumps(o, sort_keys=True) for o in paket['objekte']])
    m['objekte'] = len(paket['objekte'])
    ix['kacheln'].setdefault(k, {})[paket['schicht']] = m
    index_schreiben(kartenordner, ix)


def paket(karte, schicht, kachel, objekte, man, quellen=(), **rest):
    """Paket im Sonderbau-Format (lesbar von editor/src/sonderbau.rs)"""
    d = dict(format=FORMAT, version=VERSION, schicht=schicht, karte=karte, kachel=list(kachel),
             marker=marker(schicht, kachel), objekte=objekte, strassen=[], kreuzungen=[], gelaende=[],
             gelaende_zurueck=[], quellen=list(quellen), manifest=man)
    d.update(rest)
    return d


def _anzahl(e):
    """Objekte eines vermerkten Pakets (aeltere Verzeichnisse: aus der Paketdatei)"""
    if 'objekte' in e:
        return e['objekte']
    try:
        with open(e.get('paket', ''), encoding='utf-8') as f:
            return len(json.load(f).get('objekte', []))
    except (OSError, ValueError):
        return 1


def status(kartenordner):
    """je Kachel und Schicht mit Paket: aktuell | veraltet (Grund) | gebaut (nicht in der gespeicherten Karte) | fehlt
    -> {"kx_ky": {"gebaeude": {"zustand": ..., "grund": ...}, ...}}"""
    ix = index_lesen(kartenordner)
    out = {}
    for k in kacheln(kartenordner):
        key = f'{k[0]}_{k[1]}'
        eintraege = ix['kacheln'].get(key, {})
        jetzt = {}
        zeile = {}
        for s, o in SCHICHTEN:
            if not o:
                continue
            e = eintraege.get(s)
            in_karte = fingerabdruck(kartenordner, k, s) is not None
            if e and not in_karte and _anzahl(e) == 0:
                in_karte = True                       # leeres Paket: nichts einzusetzen, nur der Stand zaehlt
            if not e:
                zeile[s] = dict(zustand='eingesetzt' if in_karte else 'fehlt', grund='' if not in_karte else 'ohne Verzeichnis')
                continue
            if not in_karte:
                zeile[s] = dict(zustand='gebaut', grund='Paket da, in der gespeicherten Karte nicht eingesetzt')
                continue
            alt = []
            for v, h in e.get('eingaben', {}).items():
                if v not in jetzt:
                    jetzt[v] = fingerabdruck(kartenordner, k, v)
                if h != jetzt[v]:
                    alt.append(v)
            zeile[s] = dict(zustand='veraltet' if alt else 'aktuell',
                            grund=('geaendert: ' + ', '.join(alt)) if alt else '', erzeugt=e.get('erzeugt', ''))
        if any(z['zustand'] != 'fehlt' for z in zeile.values()):
            out[key] = zeile
    return out


# ---------------------------------------------------------------- Einsetzen (Python, fuer neu erzeugte Karten)

def _next_id(kartenordner):
    p = os.path.join(kartenordner, 'global.cfg')
    t = open(p, encoding='utf-16').read()
    m = re.search(r'\[NextIDCode\]\s*\n\s*(\d+)', t.replace('\r\n', '\n'))
    return int(m.group(1)) if m else 1, t


def _next_id_setzen(kartenordner, t, wert):
    t2 = re.sub(r'(\[NextIDCode\]\s*\r?\n\s*)\d+', lambda m: m.group(1) + str(wert), t, count=1)
    with open(os.path.join(kartenordner, 'global.cfg'), 'w', encoding='utf-16', newline='') as f:
        f.write(t2.replace('\r\n', '\n').replace('\n', '\r\n'))


def fmt(v):
    return f'{v:.4f}'.rstrip('0').rstrip('.') if isinstance(v, float) else str(v)


def objekt_block(o, oid, kachel, kachelgroesse=300.0):
    """[object]-Block: Pfad, ID, Lage in der Kachel, Hoehe, Drehung, Beschriftungen"""
    texte = list(o.get('texte', []))
    b = ['[object]', '0', o['rel'], str(oid), fmt(o['x'] - kachel[0] * kachelgroesse),
         fmt(o['y'] - kachel[1] * kachelgroesse), fmt(float(o.get('z', 0.0))), fmt(float(o.get('rot', 0.0))), '0', '0',
         str(len(texte))] + texte + ['']
    return b


def einsetzen(kartenordner, paket_pfad):
    """Paket in eine (neu erzeugte) Karte setzen: Objekte mit seinem Marker in der Kachel und den Nachbarn entfernen
    (samt [attachObj] daran), neue mit frischen IDs eintragen, NextIDCode fortschreiben. Nur fuer Karten, die gerade
    entstehen - bestehende Karten aendert der Editor in seiner Sitzung. -> Zahl der eingesetzten Objekte"""
    with open(paket_pfad, encoding='utf-8') as f:
        p = json.load(f)
    mk = p['marker']
    kx, ky = p['kachel']
    nid, gcfg = _next_id(kartenordner)
    neu = {}
    for o in p['objekte']:
        k = (int(o['x'] // 300.0), int(o['y'] // 300.0))
        neu.setdefault(k, []).append(o)
    vorhanden = set(kacheln(kartenordner))
    betroffen = {(kx + dx, ky + dy) for dx in (-1, 0, 1) for dy in (-1, 0, 1)} | set(neu)
    n = 0
    for k in sorted(betroffen & vorhanden):
        kopf, bl = bloecke(lesen(kartenordner, k))
        weg_ids, rest = set(), []
        for b in bl:
            if b[0].strip() == '[object]' and mk in objekt_pfad(b):
                if len(b) > 3:
                    weg_ids.add(b[3].strip())
                continue
            if b[0].strip() == '[attachObj]' and len(b) > 4 and b[4].strip() in weg_ids:
                continue
            rest.append(b)
        for o in neu.get(k, []):
            rest.append(objekt_block(o, nid, k))
            nid += 1
            n += 1
        # "Object Nr." vor jedem [object] neu zaehlen (wie omsigen.writer)
        zeilen, nr = list(kopf), 0
        for b in rest:
            b = [z for z in b if not z.startswith('Object Nr.')]
            if b[0].strip() == '[object]':
                zeilen.append(f'Object Nr. {nr}')
                nr += 1
            zeilen += b
        schreiben(kartenordner, k, '\n'.join(zeilen).rstrip('\n') + '\n')
    fehlend = sum(len(v) for k, v in neu.items() if k not in vorhanden)
    if fehlend:
        print(f'    {fehlend} Objekte liegen in Kacheln, die es in der Karte nicht gibt - weggelassen', file=sys.stderr)
    _next_id_setzen(kartenordner, gcfg, nid + 10)
    return n


# ---------------------------------------------------------------- Kommandozeile

def main():
    ap = argparse.ArgumentParser(description='Schichten einer Karte: Zustand je Kachel')
    ap.add_argument('karte')
    ap.add_argument('--omsi', default=STANDARD_OMSI)
    ap.add_argument('--json', action='store_true')
    a = ap.parse_args()
    d = a.karte if os.path.isdir(a.karte) else os.path.join(a.omsi, 'maps', a.karte)
    st = status(d)
    if a.json:
        print(json.dumps(dict(kacheln=st), ensure_ascii=False))
        return
    zaehler = {}
    for key, zeile in sorted(st.items()):
        teile = []
        for s, z in zeile.items():
            if z['zustand'] != 'fehlt':
                zaehler[(s, z['zustand'])] = zaehler.get((s, z['zustand']), 0) + 1
                teile.append(f'{s}: {z["zustand"]}' + (f' ({z["grund"]})' if z['grund'] else ''))
        print(f'{key:>8}  ' + '; '.join(teile))
    print('Summe: ' + ', '.join(f'{s} {z} {n}' for (s, z), n in sorted(zaehler.items())) if zaehler else 'keine Schicht-Pakete')


if __name__ == '__main__':
    main()
