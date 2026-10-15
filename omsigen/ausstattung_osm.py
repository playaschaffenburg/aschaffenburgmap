"""OSM-Daten fuer die Strassenausstattung (G3) und die Haltestellen (G4): Laternen, Verkehrszeichen, Poller, Zaeune,
Baenke, Muelleimer, Haltestellen (public_transport) und Zufahrten - in Kartenmetern der Karte (omsi-editor-geo.cfg).

Abfrage ueber Overpass mit Zwischenspeicher (`.cache/ausstattung_<hash>.json` im omsigen-Ordner) und Ausweichservern
wie omsigen.osm; ohne Netz und ohne Zwischenspeicher gibt es einen Fehler (die Ausstattung baut dann nur Regel-Objekte).
Kartendaten (c) OpenStreetMap-Mitwirkende, ODbL.
"""
import hashlib
import json
import math
import os
import time

QUELLE = 'Ausstattung: (c) OpenStreetMap-Mitwirkende, ODbL (Laternen, Schilder, Poller, Zaeune, Baenke, Haltestellen)'
CACHE = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), '.cache')

ABFRAGE = """[out:json][timeout:180];
(
  node["highway"~"^(street_lamp|stop|give_way|traffic_signals|crossing|bus_stop)$"]({b});
  node["traffic_sign"]({b});
  node["barrier"~"^(bollard|kerb)$"]({b});
  way["barrier"~"^(fence|bollard)$"]({b});
  node["amenity"~"^(bench|waste_basket|shelter)$"]({b});
  way["amenity"="shelter"]({b});
  node["public_transport"~"^(platform|stop_position)$"]({b});
  way["public_transport"="platform"]({b});
  way["highway"="platform"]({b});
  way["highway"="service"]({b});
  way["highway"~"^(footway|path|cycleway|steps|pedestrian)$"]({b});
)->.alle;
.alle out body geom;
node.alle["traffic_sign"]->.s;
node.alle["highway"~"^(stop|give_way)$"]->.v;
(way(bn.s)["highway"]; way(bn.v)["highway"];);
out body geom;"""


def laden(sued, west, nord, ost, cache=CACHE, log=print):
    """Overpass-JSON der Umgebung (Zwischenspeicher je Ausschnitt und Abfrage)"""
    from . import osm
    b = f'{sued:.5f},{west:.5f},{nord:.5f},{ost:.5f}'
    h = hashlib.sha1((b + ABFRAGE).encode()).hexdigest()[:16]
    datei = os.path.join(cache, f'ausstattung_{h}.json')
    if os.path.isfile(datei):
        with open(datei, encoding='utf-8') as f:
            return json.load(f)
    req = osm._requests()
    servers = [osm.OVERPASS] + osm.OVERPASS_ALT
    letzte = None
    for versuch in range(3 * len(servers)):
        url = servers[versuch % len(servers)]
        try:
            r = req.post(url, data={'data': ABFRAGE.format(b=b)}, headers={'User-Agent': osm.UA}, timeout=240)
        except req.exceptions.RequestException as ex:
            letzte = type(ex).__name__
            continue
        if r.status_code != 200:
            letzte = f'HTTP {r.status_code}'
            time.sleep(5 * (versuch // len(servers) + 1))
            continue
        js = r.json()
        if 'runtime error' in js.get('remark', ''):
            letzte = js['remark'][:80]
            continue
        os.makedirs(cache, exist_ok=True)
        with open(datei + '.tmp', 'w', encoding='utf-8') as f:
            json.dump(js, f)
        os.replace(datei + '.tmp', datei)
        log(f'    OSM-Ausstattung geladen ({len(js.get("elements", []))} Elemente)')
        return js
    raise RuntimeError(f'Overpass nicht erreichbar ({letzte})')


def bereich(kartenordner, kacheln, bz, rand=60.0):
    """(sued, west, nord, ost) der Kacheln mit Rand (m)"""
    from .utm import from_utm
    xs = [k[0] for k in kacheln]
    ys = [k[1] for k in kacheln]
    ecken = [from_utm(bz['ost0'] + x, bz['nord0'] + y, bz['zone'])
             for x in (min(xs) * 300.0 - rand, (max(xs) + 1) * 300.0 + rand)
             for y in (min(ys) * 300.0 - rand, (max(ys) + 1) * 300.0 + rand)]
    return (min(e[0] for e in ecken), min(e[1] for e in ecken), max(e[0] for e in ecken), max(e[1] for e in ecken))


class Umrechnung:
    """lat/lon <-> Kartenmeter (x Ost, z Nord) ueber den Geobezug der Karte"""

    def __init__(self, bz):
        self.bz = bz

    def __call__(self, lat, lon):
        from .utm import to_utm
        e, n = to_utm(lat, lon, self.bz['zone'])
        return e - self.bz['ost0'], n - self.bz['nord0']

    def zurueck(self, x, z):
        from .utm import from_utm
        return from_utm(x + self.bz['ost0'], z + self.bz['nord0'], self.bz['zone'])


def merkmale(js, um):
    """Overpass-JSON -> dict mit Listen in Kartenmetern:
    knoten: [dict(id, x, z, tags)], wege: [dict(id, punkte=[(x, z)], tags, knoten=[ids])]"""
    knoten, wege, gesehen = [], [], set()
    for el in js.get('elements', []):
        key = (el['type'], el['id'])
        if key in gesehen:
            continue
        gesehen.add(key)
        t = el.get('tags', {})
        if el['type'] == 'node' and 'lat' in el:
            x, z = um(el['lat'], el['lon'])
            knoten.append(dict(id=el['id'], x=x, z=z, tags=t))
        elif el['type'] == 'way' and el.get('geometry'):
            pk = [um(p['lat'], p['lon']) for p in el['geometry'] if p]
            wege.append(dict(id=el['id'], punkte=pk, tags=t, knoten=el.get('nodes', [])))
    return dict(knoten=knoten, wege=wege)


def richtung_grad(wert):
    """OSM direction (Grad oder N/NE/...) -> Grad oder None"""
    if wert is None:
        return None
    namen = {'N': 0, 'NNE': 22.5, 'NE': 45, 'ENE': 67.5, 'E': 90, 'ESE': 112.5, 'SE': 135, 'SSE': 157.5, 'S': 180,
             'SSW': 202.5, 'SW': 225, 'WSW': 247.5, 'W': 270, 'WNW': 292.5, 'NW': 315, 'NNW': 337.5}
    w = str(wert).strip().upper()
    if w in namen:
        return float(namen[w])
    try:
        return float(w) % 360.0
    except ValueError:
        return None


def schild_codes(wert):
    """traffic_sign=DE:205;DE:1020-30 -> [('205', None), ('1020-30', None)], DE:274[50] -> [('274', '50')]"""
    out = []
    land = 'DE'
    for teil in str(wert or '').replace(',', ';').split(';'):
        teil = teil.strip()
        if not teil:
            continue
        if ':' in teil:
            land, teil = teil.split(':', 1)
            land = land.strip().upper()
        if land not in ('DE', ''):
            continue
        wert_ = None
        if '[' in teil and teil.endswith(']'):
            teil, wert_ = teil[:-1].split('[', 1)
        out.append((teil.strip(), wert_))
    return out


def abstand_strecke(p, a, b):
    dx, dz = b[0] - a[0], b[1] - a[1]
    ll = dx * dx + dz * dz
    t = 0.0 if ll == 0 else max(0.0, min(1.0, ((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / ll))
    return math.hypot(p[0] - a[0] - dx * t, p[1] - a[1] - dz * t), t
