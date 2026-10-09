"""Geodaten fuer Karten des Rust-Editors (omsi-editor): Ort suchen, Gelaende und Luftbild je OMSI-Kachel.

Aufruf: python -m omsigen.geodaten < auftrag.json > ergebnis.json

Bezug einer Karte (vom Editor in maps/<Karte>/omsi-editor-geo.cfg geschrieben): UTM-Zone, Ost0/Nord0 = UTM-Koordinaten
des Kartenursprungs (Suedwestecke der Kachel 0 0) und nn0 = Hoehe ueber NN, die in OMSI 0 ist. Ein Kartenpunkt (x, y)
liegt bei Ost = Ost0 + x, Nord = Nord0 + y; Kachel (tx, ty) deckt x 300*tx .. 300*tx + 300 (y ebenso). Die Karte ist
damit nach dem UTM-Gitter ausgerichtet (Gitternord weicht in Bayern hoechstens ~1,5 Grad von Nord ab, der Massstab
um 0,04 %) - jede spaeter angefuegte Kachel bekommt so genau das Gelaende und Luftbild ihrer Stelle.

Quellen:
- Gelaende: DGM1 Bayern (1 m, CC BY 4.0, siehe gelaende.py), je Rasterpunkt das Mittel ueber 5 x 5 m; wo es fehlt
  (ausserhalb Bayerns), die weltweiten "Terrain Tiles" (Terrarium-Kodierung, AWS Open Data, aus SRTM/EU-DEM u. a.,
  etwa 30 m Aufloesung) - grober, aber ueberall.
- Luftbild: DOP40 Bayern (40 cm, CC BY 4.0) ueber WMS in EPSG:25832 (nur in UTM-Zone 32), genau auf die Kachel
  zugeschnitten; ausserhalb Bayerns liefert der Dienst ein leeres Bild -> kein Luftbild.
- Ortssuche: Nominatim (OpenStreetMap), oder Koordinaten direkt ("49.973, 9.147").

Auftraege:
  {"auftrag": "suchen", "text": "..."}
      -> {"orte": [{"name", "lat", "lon"}, ...]}
  {"auftrag": "bezug", "lat", "lon", "cache"}
      -> {"zone", "ost", "nord", "nn", "quelle"}    (UTM des Orts, Gelaendehoehe dort; nn null ohne Daten)
  {"auftrag": "kacheln", "zone", "ost0", "nord0", "nn0", "kacheln": [[tx, ty], ...], "gelaende": bool,
   "luftbild": bool, "pixel": 1024, "ordner": Ausgabeordner, "cache": Zwischenspeicher}
      -> {"kacheln": [{"kachel": [tx, ty], "gelaende": Datei (.terrain: int32 60, 61 x 61 float32 von Sueden) oder
          null, "gelaende_quelle": "DGM1 Bayern"/"Terrain Tiles"/null, "luftbild": Datei (.jpg, erste Zeile Norden)
          oder null}], "quellen": [Quellenangaben]}
oder {"fehler": Text}."""
import io, json, math, os, re, sys
import numpy as np
from .utm import to_utm, from_utm

UA = 'omsigen/0.1 (OMSI-2-Kartengenerator; https://github.com/playaschaffenburg/aschaffenburgmap)'
NOMINATIM = 'https://nominatim.openstreetmap.org/search'
DOP_WMS = 'https://geoservices.bayern.de/od/wms/dop/v1/dop40'
DOP_EBENE = 'by_dop40c'
TERRARIUM = 'https://s3.amazonaws.com/elevation-tiles-prod/terrarium/{z}/{x}/{y}.png'
TERRARIUM_Z = 14
QUELLE_DGM = 'Gelaende: DGM1 Bayerische Vermessungsverwaltung - www.geodaten.bayern.de (CC BY 4.0)'
QUELLE_DOP = 'Luftbild: Bayerische Vermessungsverwaltung - www.geodaten.bayern.de (CC BY 4.0)'
QUELLE_TT = 'Gelaende: Terrain Tiles (AWS Open Data; SRTM, EU-DEM u. a.)'
KACHEL = 300.0
N = 60                      # Zellen je Kachel (61 Punkte, 5 m)


def zone_fuer(lat, lon):
    """UTM-Zone: in Bayern (und angrenzend) 32 wie das DGM1/DOP (EPSG:25832), sonst die Standardzone"""
    if 47.0 <= lat <= 51.0 and 8.5 <= lon <= 14.0:
        return 32
    return int((lon + 180) // 6) + 1


def _get(url, **kw):
    import requests
    return requests.get(url, headers={'User-Agent': UA}, timeout=kw.pop('timeout', 60), **kw)


# ---------------------------------------------------------------- Ortssuche

def suchen(text):
    m = re.match(r'^\s*(-?\d+(?:\.\d+)?)\s*[,; ]\s*(-?\d+(?:\.\d+)?)\s*$', text or '')
    if m:
        lat, lon = float(m.group(1)), float(m.group(2))
        if -90 <= lat <= 90 and -180 <= lon <= 180:
            return [{'name': f'{lat:.6f}, {lon:.6f}', 'lat': lat, 'lon': lon}]
    if not (text or '').strip():
        return []
    r = _get(NOMINATIM, params={'q': text, 'format': 'jsonv2', 'limit': 8, 'accept-language': 'de'}, timeout=20)
    r.raise_for_status()
    return [{'name': e.get('display_name', ''), 'lat': float(e['lat']), 'lon': float(e['lon'])} for e in r.json()]


# ---------------------------------------------------------------- Gelaende

class Dgm1:
    """DGM1-Kacheln (1 km, Zeile 0 = Norden) aus gelaende.Gelaende, vektorisiert abgefragt"""

    def __init__(self, cache):
        from .gelaende import Gelaende
        self.g = Gelaende(None, cache=os.path.join(cache, 'dgm1'), log=lambda *a: print(*a, file=sys.stderr))

    def hoehen(self, e, n):
        """Hoehen (NaN ohne Daten) an UTM-32-Punkten (Arrays), naechstes 1-m-Pixel"""
        out = np.full(e.shape, np.nan, dtype=np.float64)
        ek, nk = np.floor(e / 1000).astype(int), np.floor(n / 1000).astype(int)
        for a, b in set(zip(ek.ravel().tolist(), nk.ravel().tolist())):
            arr = self.g._kachel(a, b)
            if arr is None:
                continue
            sel = (ek == a) & (nk == b)
            c = np.clip(np.floor(e[sel] - a * 1000).astype(int), 0, 999)
            r = np.clip(np.floor((b + 1) * 1000 - n[sel]).astype(int), 0, 999)
            out[sel] = arr[r, c]
        return out


class Terrarium:
    """weltweite Hoehen aus den Terrain Tiles (Web-Mercator, Zoom 14, bilinear)"""

    def __init__(self, cache):
        self.cache = os.path.join(cache, 'terrarium')
        self.kacheln = {}

    def _kachel(self, x, y):
        key = (x, y)
        if key not in self.kacheln:
            datei = os.path.join(self.cache, str(TERRARIUM_Z), f'{x}_{y}.png')
            a = None
            try:
                if not os.path.exists(datei):
                    r = _get(TERRARIUM.format(z=TERRARIUM_Z, x=x, y=y))
                    if r.status_code == 200:
                        os.makedirs(os.path.dirname(datei), exist_ok=True)
                        with open(datei + '.tmp', 'wb') as f:
                            f.write(r.content)
                        os.replace(datei + '.tmp', datei)
                if os.path.exists(datei):
                    from PIL import Image
                    p = np.asarray(Image.open(datei).convert('RGB'), dtype=np.float64)
                    a = p[:, :, 0] * 256 + p[:, :, 1] + p[:, :, 2] / 256 - 32768
            except Exception as ex:
                print(f'Terrain Tile {x}/{y} nicht ladbar: {type(ex).__name__}', file=sys.stderr)
            self.kacheln[key] = a
        return self.kacheln[key]

    def hoehe(self, lat, lon):
        n = 2 ** TERRARIUM_Z
        fx = (lon + 180) / 360 * n * 256
        s = math.sin(math.radians(lat))
        fy = (0.5 - math.log((1 + s) / (1 - s)) / (4 * math.pi)) * n * 256
        fx, fy = fx - 0.5, fy - 0.5
        x0, y0 = math.floor(fx), math.floor(fy)
        tx, ty = fx - x0, fy - y0

        def px(x, y):
            a = self._kachel(x // 256, y // 256)
            return None if a is None else a[y % 256, x % 256]
        q = [px(x0, y0), px(x0 + 1, y0), px(x0, y0 + 1), px(x0 + 1, y0 + 1)]
        if any(v is None for v in q):
            return None
        return float((q[0] * (1 - tx) + q[1] * tx) * (1 - ty) + (q[2] * (1 - tx) + q[3] * tx) * ty)


class Hoehen:
    def __init__(self, cache, zone):
        self.zone = zone
        self.dgm = Dgm1(cache) if zone == 32 else None
        self.tt = Terrarium(cache)

    def raster(self, e, n):
        """Hoehen ueber NN an UTM-Punkten (Arrays) -> (Hoehen, Quelle); DGM1 als 5x5-m-Mittel, Rest Terrain Tiles"""
        h = np.full(e.shape, np.nan)
        quelle = None
        if self.dgm is not None:
            summe, anzahl = np.zeros(e.shape), np.zeros(e.shape)
            for dx in (-2, -1, 0, 1, 2):
                for dy in (-2, -1, 0, 1, 2):
                    v = self.dgm.hoehen(e + dx + 0.5, n + dy + 0.5)
                    ok = ~np.isnan(v)
                    summe[ok] += v[ok]
                    anzahl[ok] += 1
            ok = anzahl > 0
            h[ok] = summe[ok] / anzahl[ok]
            if ok.any():
                quelle = 'DGM1 Bayern'
        fehlt = np.isnan(h)
        if fehlt.any():
            for i in zip(*np.nonzero(fehlt)):
                lat, lon = from_utm(float(e[i]), float(n[i]), self.zone)
                v = self.tt.hoehe(lat, lon)
                if v is not None:
                    h[i] = v
            if (~np.isnan(h[fehlt])).any():
                quelle = 'Terrain Tiles' if quelle is None else quelle + ' + Terrain Tiles'
        return h, quelle


# ---------------------------------------------------------------- Luftbild

def luftbild(zone, e0, n0, e1, n1, pixel, cache):
    """DOP40-Ausschnitt (EPSG:25832) als JPEG-Bytes oder None (Zone nicht 32, leer, Netzfehler)"""
    if zone != 32:
        return None
    datei = os.path.join(cache, 'luftbild', f'{DOP_EBENE}_{e0:.2f}_{n0:.2f}_{e1:.2f}_{n1:.2f}_{pixel}.jpg')
    if os.path.exists(datei):
        with open(datei, 'rb') as f:
            daten = f.read()
        return daten or None
    try:
        r = _get(DOP_WMS, params=dict(SERVICE='WMS', REQUEST='GetMap', VERSION='1.3.0', LAYERS=DOP_EBENE, STYLES='',
                                      CRS='EPSG:25832', BBOX=f'{e0},{n0},{e1},{n1}', WIDTH=pixel, HEIGHT=pixel,
                                      FORMAT='image/jpeg'), timeout=60)
    except Exception as ex:
        print(f'Luftbild nicht ladbar: {type(ex).__name__}', file=sys.stderr)
        return None
    if r.status_code != 200 or not r.headers.get('content-type', '').startswith('image'):
        return None
    daten = r.content
    # ausserhalb der Abdeckung: einfarbiges Bild (weiss/schwarz)
    from PIL import Image
    a = np.asarray(Image.open(io.BytesIO(daten)).convert('L').resize((64, 64)), dtype=np.float64)
    leer = a.std() < 1.5
    os.makedirs(os.path.dirname(datei), exist_ok=True)
    with open(datei, 'wb') as f:
        f.write(b'' if leer else daten)
    return None if leer else daten


# ---------------------------------------------------------------- Auftraege

def bezug(lat, lon, cache):
    zone = zone_fuer(lat, lon)
    e, n = to_utm(lat, lon, zone)
    h, quelle = Hoehen(cache, zone).raster(np.array([e]), np.array([n]))
    nn = None if np.isnan(h[0]) else float(h[0])
    return {'zone': zone, 'ost': e, 'nord': n, 'nn': nn, 'quelle': quelle}


def kacheln(a):
    zone, e0, n0, nn0 = int(a['zone']), float(a['ost0']), float(a['nord0']), float(a.get('nn0', 0.0))
    ordner, cache = a['ordner'], a.get('cache', '.cache')
    pixel = int(a.get('pixel', 1024))
    os.makedirs(ordner, exist_ok=True)
    hoehen = Hoehen(cache, zone)
    quellen, aus = set(), []
    for tx, ty in a['kacheln']:
        eintrag = {'kachel': [tx, ty], 'gelaende': None, 'gelaende_quelle': None, 'luftbild': None}
        ke, kn = e0 + tx * KACHEL, n0 + ty * KACHEL
        if a.get('gelaende', True):
            ix, iz = np.meshgrid(np.arange(N + 1), np.arange(N + 1))      # Zeile iz von Sueden
            h, q = hoehen.raster(ke + ix * 5.0, kn + iz * 5.0)
            if q is not None:
                h = np.where(np.isnan(h), np.nanmean(h), h) - nn0
                datei = os.path.join(ordner, f'gelaende_{tx}_{ty}.terrain')
                with open(datei, 'wb') as f:
                    f.write(np.int32(N).tobytes() + h.astype('<f4').tobytes())
                eintrag['gelaende'], eintrag['gelaende_quelle'] = datei, q
                if 'DGM1' in q:
                    quellen.add(QUELLE_DGM)
                if 'Terrain Tiles' in q:
                    quellen.add(QUELLE_TT)
        if a.get('luftbild', True):
            daten = luftbild(zone, ke, kn, ke + KACHEL, kn + KACHEL, pixel, cache)
            if daten:
                datei = os.path.join(ordner, f'luftbild_{tx}_{ty}.jpg')
                with open(datei, 'wb') as f:
                    f.write(daten)
                eintrag['luftbild'] = datei
                quellen.add(QUELLE_DOP)
        aus.append(eintrag)
    return {'kacheln': aus, 'quellen': sorted(quellen)}


def main():
    try:
        a = json.load(sys.stdin)
        art = a.get('auftrag')
        if art == 'suchen':
            erg = {'orte': suchen(a.get('text', ''))}
        elif art == 'bezug':
            erg = bezug(float(a['lat']), float(a['lon']), a.get('cache', '.cache'))
        elif art == 'kacheln':
            erg = kacheln(a)
        else:
            erg = {'fehler': f'unbekannter Auftrag {art!r}'}
    except Exception as ex:
        erg = {'fehler': f'{type(ex).__name__}: {ex}'}
    json.dump(erg, sys.stdout)


if __name__ == '__main__':
    main()
