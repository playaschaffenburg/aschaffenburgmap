"""Luftbild als Hintergrund: amtliche Orthophotos der Bayerischen Vermessungsverwaltung (DOP40, 40 cm, CC BY 4.0,
kostenfrei, ohne Anmeldung) ueber WMS. Kacheln je Zoomstufe, Laden im Hintergrund, Zwischenspeicher auf der Platte.
Ausserhalb Bayerns liefert der Dienst leere Bilder.

Kachelraster in Projektmetern (route.Projection ist eine ebene Naeherung um den Ursprung, lat/lon haengen linear
an x/z - deshalb passt ein WMS-Bild in EPSG:4326 genau auf eine rechteckige Kachel)."""
import hashlib, os
import requests
from PySide6.QtCore import QObject, QRunnable, QThreadPool, Signal

WMS = 'https://geoservices.bayern.de/od/wms/dop/v1/dop40'
EBENE = 'by_dop40c'
QUELLE = 'Luftbild: Bayerische Vermessungsverwaltung - www.geodaten.bayern.de (CC BY 4.0)'
UA = 'omsigen-editor/0.1 (https://github.com/playaschaffenburg/aschaffenburgmap)'
PIXEL = 512                                  # Kachelgroesse in Bildpunkten
STUFEN = [0.4 * 2 ** k for k in range(8)]    # Meter je Bildpunkt (0,4 m = volle Aufloesung)


def stufe_fuer(m_pro_px_bildschirm):
    """passende Stufe: die feinste, die nicht mehr als 1,5 Kachelpixel je Bildschirmpixel braucht"""
    for k, s in enumerate(STUFEN):
        if s * 1.5 >= m_pro_px_bildschirm:
            return k
    return len(STUFEN) - 1


def kacheln_im_bereich(x0, z0, x1, z1, k):
    g = STUFEN[k] * PIXEL
    import math
    for ix in range(math.floor(x0 / g), math.floor(x1 / g) + 1):
        for iz in range(math.floor(z0 / g), math.floor(z1 / g) + 1):
            yield (k, ix, iz)


def kachel_rechteck(kachel):
    k, ix, iz = kachel
    g = STUFEN[k] * PIXEL
    return ix * g, iz * g, (ix + 1) * g, (iz + 1) * g       # x0, z0, x1, z1


class _Signale(QObject):
    fertig = Signal(tuple, bytes)


class _Laden(QRunnable):
    def __init__(self, kachel, proj, cache, signale):
        super().__init__()
        self.kachel, self.proj, self.cache, self.s = kachel, proj, cache, signale

    def run(self):
        x0, z0, x1, z1 = kachel_rechteck(self.kachel)
        la0, lo0 = self.proj.to_ll(x0, z0)
        la1, lo1 = self.proj.to_ll(x1, z1)
        key = f'{EBENE}_{la0:.7f}_{lo0:.7f}_{la1:.7f}_{lo1:.7f}_{PIXEL}'
        datei = os.path.join(self.cache, hashlib.sha1(key.encode()).hexdigest()[:20] + '.jpg')
        try:
            if os.path.exists(datei):
                with open(datei, 'rb') as f:
                    daten = f.read()
            else:
                r = requests.get(WMS, params=dict(SERVICE='WMS', REQUEST='GetMap', VERSION='1.3.0', LAYERS=EBENE,
                                                  STYLES='', CRS='EPSG:4326', BBOX=f'{la0},{lo0},{la1},{lo1}',
                                                  WIDTH=PIXEL, HEIGHT=PIXEL, FORMAT='image/jpeg'),
                                 headers={'User-Agent': UA}, timeout=30)
                if r.status_code != 200 or not r.headers.get('content-type', '').startswith('image'):
                    return
                daten = r.content
                os.makedirs(self.cache, exist_ok=True)
                with open(datei, 'wb') as f:
                    f.write(daten)
            self.s.fertig.emit(self.kachel, daten)
        except (requests.RequestException, OSError):
            pass


class Luftbild(QObject):
    """bestellt Kacheln; meldet fertige Kacheln ueber das Signal kachel(kachel, jpeg-bytes)"""
    kachel = Signal(tuple, bytes)

    def __init__(self, proj, cache='.cache/luftbild', parallel=4):
        super().__init__()
        self.proj, self.cache = proj, cache
        self.pool = QThreadPool()
        self.pool.setMaxThreadCount(parallel)
        self.s = _Signale()
        self.s.fertig.connect(self._fertig)
        self.bestellt = set()

    def bestellen(self, kachel):
        if kachel in self.bestellt:
            return
        self.bestellt.add(kachel)
        self.pool.start(_Laden(kachel, self.proj, self.cache, self.s))

    def _fertig(self, kachel, daten):
        self.kachel.emit(kachel, daten)
