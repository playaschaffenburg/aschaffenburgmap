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


def geocode(query, city=None, cache_dir=None):
    """'49.98,9.14' oder Ortsname -> (lat, lon). Mit Stadt als Zusatz, z. B. ('Hauptbahnhof', 'Aschaffenburg')."""
    m = re.fullmatch(r'\s*(-?\d+(?:\.\d+)?)\s*,\s*(-?\d+(?:\.\d+)?)\s*', query)
    if m:
        return float(m[1]), float(m[2])
    q = f'{query}, {city}' if city else query
    cached = _cache_get(cache_dir, 'geo', q)
    if cached:
        return tuple(cached)
    r = _requests().get(NOMINATIM, params=dict(q=q, format='json', limit=1, countrycodes='de'),
                        headers={'User-Agent': UA}, timeout=30)
    r.raise_for_status()
    res = r.json()
    if not res:
        raise ValueError(f'Ort nicht gefunden: {q}')
    ll = (float(res[0]['lat']), float(res[0]['lon']))
    _cache_put(cache_dir, 'geo', q, list(ll))
    time.sleep(1)  # Nominatim-Nutzungsregeln: max. 1 Anfrage/s
    return ll


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
    data = normalize(r.json(), bbox)
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
