"""Vorfahrt an Kreuzungen nach realem Vorbild.

Je Kreuzung wird bestimmt, welche Arme Hauptstrasse (Vorfahrt) und welche Nebenstrasse (wartepflichtig) sind.
Die erste zutreffende Regel gilt (Quelle wird vermerkt):
  0. Korrekturdatei des Nutzers
  1. Kreisverkehr: Einfahrten warten (StVO-Zeichen 215 + 205)
  2. Schilder aus OSM (Vorfahrt gewaehren / Stopp): diese Arme warten
  3. Vorfahrtstrasse aus OSM (priority_road, Zeichen 306): diese Arme haben Vorfahrt
  4. StVO: Ausfahrt aus Zufahrt/Parkplatz/verkehrsberuhigtem Bereich wartet (Paragraf 10);
     Tempo-30-Zone: rechts vor links
  5. vermutet: hoehere Strassenklasse hat Vorfahrt, bei gleicher Klasse rechts vor links
Ampel (OSM traffic_signals) wird als Merkmal vermerkt; die Rollen gelten dann fuer ausgeschaltete Ampeln.

Prioritaeten der Bewegungen wie in den Standardkarten (docs/kartenstudie.md): von der Hauptstrasse geradeaus
und rechts 192, links ohne Regel (wartet auf Gegenverkehr), entlang einer abknickenden Vorfahrt 192;
alles aus der Nebenstrasse 64; ohne Hauptstrasse keine Regel (rechts vor links)."""
import json, math
from .config import RANK

HAUPT, NEBEN, GLEICH = 'haupt', 'neben', 'gleich'
P_HAUPT, P_NEBEN = 192, 64
NACHRANG = {'service', 'living_street', 'track'}          # Paragraf 10 StVO


def _tempo30(t):
    return str(t.get('maxspeed', '')).strip() == '30' or 'DE:30' in str(t.get('zone:maxspeed', '')) or \
        'DE:zone30' in str(t.get('zone:traffic', ''))


def _rank(t):
    return RANK.get(t.get('highway'), 2)


def decide(arms, flags=(), korrektur=None):
    """arms: Liste dict(tags, sign, prio_road, ring, name) -> dict(rollen, quelle, text, ampel)"""
    n = len(arms)
    ampel = 'signals' in flags
    res = lambda rollen, quelle, text: dict(rollen=rollen, quelle=quelle, text=text, ampel=ampel)
    if korrektur:
        if korrektur.get('bestaetigt'):                  # berechnete Vorfahrt ist vom Nutzer geprueft
            d = decide(arms, flags)
            return dict(d, quelle='bestaetigt', text=d['text'].replace('vermutet: ', '') + ' (bestaetigt)')
        if korrektur.get('regel') == 'rechts_vor_links':
            return res([GLEICH] * n, 'Korrektur', 'rechts vor links (Korrekturdatei)')
        namen = [x.lower() for x in korrektur.get('haupt', [])]
        rollen = [HAUPT if a['name'].lower() in namen else NEBEN for a in arms]
        if HAUPT in rollen:
            return res(rollen, 'Korrektur', 'Hauptstrasse ' + ', '.join(korrektur['haupt']) + ' (Korrekturdatei)')
    if 'mini_roundabout' in flags:
        return res([GLEICH] * n, 'OSM Mini-Kreisel', 'Mini-Kreisverkehr: alle Einfahrten gleich')
    if any(a['ring'] for a in arms) and not all(a['ring'] for a in arms):
        return res([HAUPT if a['ring'] else NEBEN for a in arms], 'StVO Kreisverkehr',
                   'Kreisverkehr: Einfahrten warten')
    if any(a['sign'] in ('give_way', 'stop') for a in arms):
        rollen = [NEBEN if a['sign'] in ('give_way', 'stop') else HAUPT for a in arms]
        if HAUPT in rollen:
            art = 'Stopp' if any(a['sign'] == 'stop' for a in arms) else 'Vorfahrt gewaehren'
            return res(rollen, 'OSM Schild', f'Schild {art} an {rollen.count(NEBEN)} Arm(en)')
    if any(a['prio_road'] or a['tags'].get('priority_road') in ('designated', 'yes_unposted', 'yes')
           for a in arms):
        rollen = [HAUPT if (a['prio_road'] or a['tags'].get('priority_road') in ('designated', 'yes_unposted', 'yes'))
                  else NEBEN for a in arms]
        if NEBEN in rollen:
            return res(rollen, 'OSM Vorfahrtstrasse', 'Vorfahrtstrasse (priority_road)')
    low = [a['tags'].get('highway') in NACHRANG for a in arms]
    if any(low) and not all(low):
        rest = [a for a, l in zip(arms, low) if not l]
        if len(rest) <= 2:
            return res([NEBEN if l else HAUPT for l in low], 'StVO Paragraf 10',
                       'Ausfahrt aus Zufahrt/verkehrsberuhigtem Bereich wartet')
    if all(_tempo30(a['tags']) for a in arms):
        return res([GLEICH] * n, 'StVO Tempo 30', 'Tempo-30-Bereich: rechts vor links')
    ranks = [_rank(a['tags']) for a in arms]
    top = max(ranks)
    group = [i for i, r in enumerate(ranks) if r == top]
    if len(group) > 2 and top >= RANK['tertiary']:
        pair = _straight_pair(arms, group)            # durchgehende Hauptstrasse unter gleich wichtigen Armen
        if pair:
            return res([HAUPT if i in pair else NEBEN for i in range(n)], 'vermutet',
                       'vermutet: durchgehende Hauptstrasse hat Vorfahrt')
    if len(group) < n:
        return res([HAUPT if r == top else NEBEN for r in ranks], 'vermutet',
                   'vermutet: hoehere Strassenklasse hat Vorfahrt')
    return res([GLEICH] * n, 'vermutet', 'vermutet: gleiche Strassenklasse, rechts vor links')


def _straight_pair(arms, idx):
    """das am geradesten durchlaufende Armpaar (Winkel > 140 Grad), gleicher Name bevorzugt -> {i, j} oder None"""
    best = None
    for x, i in enumerate(idx):
        for j in idx[x + 1:]:
            ang = abs((arms[i]['h'] - arms[j]['h'] + 180) % 360 - 180)
            if ang < 140:
                continue
            same = bool(arms[i]['name']) and arms[i]['name'] == arms[j]['name']
            key = (same, ang)
            if best is None or key > best[0]:
                best = (key, {i, j})
    return best[1] if best else None


def priority(rolle_von, rolle_nach, mv, n_haupt):
    """Prioritaet einer Bewegung (oder None = keine Regel)"""
    if rolle_von == NEBEN:
        return P_NEBEN
    if rolle_von != HAUPT:
        return None
    if rolle_nach == HAUPT and n_haupt == 2:            # entlang der (ggf. abknickenden) Hauptstrasse
        return P_HAUPT
    if mv in ('gerade', 'rechts'):
        return P_HAUPT
    return None                                          # links: wartet auf den Gegenverkehr


def load_corrections(path):
    """Korrekturdatei (JSON): {"vorfahrt": [{"lat", "lon", "haupt": [Strassennamen]} oder {"regel":
    "rechts_vor_links"} oder {"bestaetigt": true} (berechnete Vorfahrt stimmt), ...]}"""
    if not path:
        return []
    with open(path, encoding='utf-8') as f:
        return json.load(f).get('vorfahrt', [])


def distance_m(lat1, lon1, lat2, lon2):
    return math.dist((lat1 * 111320, lon1 * 111320 * math.cos(math.radians(lat1))),
                     (lat2 * 111320, lon2 * 111320 * math.cos(math.radians(lat1))))


def match_correction(corrections, lat, lon, radius=30.0):
    best = None
    for c in corrections:
        d = distance_m(lat, lon, c['lat'], c['lon'])
        if d <= radius and (best is None or d < best[0]):
            best = (d, c)
    return best[1] if best else None
