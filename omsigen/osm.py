"""OpenStreetMap-Daten: Orte suchen (Nominatim) und Strassen laden (Overpass), mit Zwischenspeicher.
Kartendaten (c) OpenStreetMap-Mitwirkende, ODbL."""
import json, os, re, time, hashlib
from .config import DRIVABLE

UA = 'omsigen/0.1 (OMSI-2-Kartengenerator; https://github.com/playaschaffenburg/aschaffenburgmap)'
NOMINATIM = 'https://nominatim.openstreetmap.org/search'
OVERPASS = 'https://overpass-api.de/api/interpreter'


def _requests():
    import requests
    return requests


# Abkuerzungen, unter denen Orte in OSM oft heissen (Bahnhof "Aschaffenburg Hbf" statt "Hauptbahnhof")
ABKUERZUNGEN = [('hauptbahnhof', 'hbf'), ('bahnhof', 'bf'), ('strasse', 'str'), ('straße', 'str')]

# Rang der OSM-Kategorie: Haltestellen und Bahnhoefe zuerst (es geht um Busstrecken), Parkhaeuser u. a. zuletzt
RANG_KATEGORIE = {
    ('railway', 'station'): 0, ('amenity', 'bus_station'): 0, ('public_transport', 'station'): 0,
    ('highway', 'bus_stop'): 1, ('public_transport', 'platform'): 1, ('public_transport', 'stop_position'): 1,
    ('railway', 'halt'): 1, ('railway', 'stop'): 1,
    ('amenity', 'parking'): 3, ('amenity', 'parking_entrance'): 3, ('railway', 'signal_box'): 3,
    ('railway', 'junction'): 3, ('railway', 'switch'): 3, ('railway', 'signal'): 3,
}


def _norm(s):
    return re.sub(r'[^a-z0-9äöüß]+', ' ', (s or '').lower()).strip()


def _varianten(query):
    """'Hauptbahnhof' -> ['hauptbahnhof', 'hbf'] (normalisiert, Original zuerst)"""
    out = [_norm(query)]
    for lang, kurz in ABKUERZUNGEN:
        for v in list(out):
            if re.search(rf'\b{lang}\b', v):
                out.append(re.sub(rf'\b{lang}\b', kurz, v))
            elif re.search(rf'\b{kurz}\b', v):
                out.append(re.sub(rf'\b{kurz}\b', lang, v))
    return list(dict.fromkeys(out))


def rank_candidates(cands, query, city=None):
    """Nominatim-Treffer sortieren: 1. Name passt genau (Stadtname im Treffer darf dabeistehen),
    2. Name enthaelt den Suchbegriff, 3. Kategorie (Haltestelle/Bahnhof vor Parkhaus), 4. Reihenfolge der Suche."""
    vs = _varianten(query)
    c = _norm(city)

    def key(item):
        i, r = item
        name = _norm(r.get('name') or r.get('display_name', '').split(',')[0])
        ohne_stadt = re.sub(rf'\b{re.escape(c)}\b', ' ', name).strip() if c else name
        ohne_stadt = re.sub(r'\s+', ' ', ohne_stadt)
        if name in vs or ohne_stadt in vs:
            m = 0
        elif any(re.search(rf'\b{re.escape(v)}\b', name) for v in vs):
            m = 1
        else:
            m = 2
        return (m, RANG_KATEGORIE.get((r.get('class'), r.get('type')), 2), i)
    return [r for i, r in sorted(enumerate(cands), key=key)]


def find_place(query, city=None, cache_dir=None):
    """'49.98,9.14' oder Ortsname -> dict(lat, lon, name, art). Mit Stadt als Zusatz, z. B. ('Hauptbahnhof', 'Aschaffenburg').
    Fragt Nominatim auch mit Abkuerzungen (Hauptbahnhof/Hbf) und waehlt den am besten passenden Treffer."""
    m = re.fullmatch(r'\s*(-?\d+(?:\.\d+)?)\s*,\s*(-?\d+(?:\.\d+)?)\s*', query)
    if m:
        return dict(lat=float(m[1]), lon=float(m[2]), name=None, art='Koordinaten')
    q = f'{query}, {city}' if city else query
    cached = _cache_get(cache_dir, 'geo2', q)
    if cached:
        return cached
    cands, seen = [], set()
    for v in _varianten(query):
        r = _requests().get(NOMINATIM, params=dict(q=f'{v}, {city}' if city else v, format='json', limit=10,
                                                   countrycodes='de'),
                            headers={'User-Agent': UA}, timeout=30)
        r.raise_for_status()
        for x in r.json():
            if (x.get('osm_type'), x.get('osm_id')) not in seen:
                seen.add((x.get('osm_type'), x.get('osm_id'))); cands.append(x)
        time.sleep(1)  # Nominatim-Nutzungsregeln: max. 1 Anfrage/s
    if not cands:
        raise ValueError(f'Ort nicht gefunden: {q}')
    best = rank_candidates(cands, query, city)[0]
    res = dict(lat=float(best['lat']), lon=float(best['lon']),
               name=best.get('name') or best.get('display_name', '').split(',')[0],
               art=f"{best.get('class')}/{best.get('type')}")
    _cache_put(cache_dir, 'geo2', q, res)
    return res


def geocode(query, city=None, cache_dir=None):
    """'49.98,9.14' oder Ortsname -> (lat, lon)"""
    p = find_place(query, city, cache_dir)
    return p['lat'], p['lon']


def fetch(bbox, cache_dir=None):
    """bbox = (sued, west, nord, ost) -> normalisierte Daten (siehe normalize)"""
    s, w, n, e = bbox
    b = f'{s:.6f},{w:.6f},{n:.6f},{e:.6f}'
    cached = _cache_get(cache_dir, 'osm', b)
    if cached:
        return cached
    q = f"""[out:json][timeout:120];
(
  way["highway"]({b});
  node["highway"="bus_stop"]({b});
  node["public_transport"="platform"]["bus"="yes"]({b});
);
out tags geom;"""
    for attempt in range(4):
        r = _requests().post(OVERPASS, data={'data': q}, headers={'User-Agent': UA}, timeout=180)
        if r.status_code in (429, 504):
            time.sleep(10 * (attempt + 1))
            continue
        r.raise_for_status()
        break
    else:
        raise RuntimeError(f'Overpass-Server ueberlastet (HTTP {r.status_code}), bitte spaeter erneut versuchen')
    js = r.json()
    if 'runtime error' in js.get('remark', ''):   # Abbruch auf dem Server -> Daten unvollstaendig
        raise RuntimeError(f'Overpass-Abfrage abgebrochen: {js["remark"]}')
    data = normalize(js, bbox)
    _cache_put(cache_dir, 'osm', b, data)
    return data


def normalize(overpass_json, bbox=None):
    ways, stops = [], []
    for el in overpass_json.get('elements', []):
        t = el.get('tags', {})
        if el['type'] == 'node':
            if t.get('name'):
                stops.append(dict(name=t['name'], lat=el['lat'], lon=el['lon']))
        elif el['type'] == 'way' and 'geometry' in el:
            tt = classify(t)
            if tt:
                ways.append(dict(id=el['id'], tags=tt, coords=[[p['lat'], p['lon']] for p in el['geometry']]))
    return dict(source='OpenStreetMap (ODbL)', bbox=list(bbox) if bbox else None, ways=ways, stops=stops)


def classify(t, include_service=False):
    """OSM-Tags -> uebernommene Tags oder None (Weg wird ignoriert)"""
    hw = t.get('highway')
    if hw not in DRIVABLE or t.get('area') == 'yes':
        return None
    bus = t.get('psv') in ('yes', 'designated') or t.get('bus') in ('yes', 'designated') or hw == 'busway'
    tt = {k: v for k, v in t.items() if k in (
        'highway', 'name', 'lanes', 'lanes:forward', 'lanes:backward', 'oneway', 'junction', 'layer',
        'tunnel', 'bridge', 'service', 'access', 'psv', 'bus', 'covered')}
    if hw == 'service' or hw == 'busway':
        if bus and t.get('access') in ('no', 'private', None) and (t.get('psv') or t.get('bus') or hw == 'busway'):
            tt['highway'] = 'bus'
        elif not include_service or t.get('service') in ('parking_aisle', 'driveway', 'drive-through'):
            return None
    elif t.get('access') in ('no', 'private') and not bus:
        return None
    return tt


def load(path):
    with open(path, encoding='utf-8') as f:
        return json.load(f)


def save(data, path):
    with open(path, 'w', encoding='utf-8') as f:
        json.dump(data, f, ensure_ascii=False)


def _cache_file(cache_dir, kind, key):
    h = hashlib.sha1(key.encode()).hexdigest()[:16]
    return os.path.join(cache_dir, f'{kind}_{h}.json')


def _cache_get(cache_dir, kind, key):
    if not cache_dir:
        return None
    p = _cache_file(cache_dir, kind, key)
    return load(p) if os.path.exists(p) else None


def _cache_put(cache_dir, kind, key, data):
    if cache_dir:
        os.makedirs(cache_dir, exist_ok=True)
        save(data, _cache_file(cache_dir, kind, key))
