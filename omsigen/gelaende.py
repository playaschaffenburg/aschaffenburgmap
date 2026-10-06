"""Gelaendehoehen aus dem Digitalen Gelaendemodell DGM1 der Bayerischen Vermessungsverwaltung
(1 m Raster, 1x1-km-Kacheln GeoTIFF in UTM 32 / EPSG:25832, CC BY 4.0, kostenfrei:
https://geodaten.bayern.de/opengeodata/OpenDataDetail.html?pn=dgm1).

Das DGM zeigt den Boden ohne Bauwerke: unter Bruecken liegt der Grund darunter (Fluss, Gleise), in Unterfuehrungen
die Strassensohle. Bruecken- und Tunnelprofile berechnet hoehen.py deshalb zwischen ihren Enden.
Ausserhalb Bayerns oder ohne Netz gibt es keine Daten -> hoehe() liefert None, die Karte bleibt flach."""
import math, os
import numpy as np
from .utm import to_utm

URL = 'https://download1.bayernwolke.de/a/dgm/dgm1/{e}_{n}.tif'
QUELLE = 'Gelaende: DGM1 Bayerische Vermessungsverwaltung - www.geodaten.bayern.de (CC BY 4.0)'
UA = 'omsigen/0.1 (OMSI-2-Kartengenerator; https://github.com/playaschaffenburg/aschaffenburgmap)'


class Gelaende:
    def __init__(self, proj, cache='.cache/dgm1', log=print, laden=True):
        self.proj, self.cache, self.log, self.laden = proj, cache, log, laden
        self._kacheln = {}          # (e_km, n_km) -> np.array (1000x1000, Zeile 0 = Norden) oder None
        self.fehlend = set()

    def _kachel(self, ek, nk):
        key = (ek, nk)
        if key in self._kacheln:
            return self._kacheln[key]
        datei = os.path.join(self.cache, f'{ek}_{nk}.tif')
        a = None
        if not os.path.exists(datei) and self.laden:
            try:
                import requests
                r = requests.get(URL.format(e=ek, n=nk), headers={'User-Agent': UA}, timeout=120)
                if r.status_code == 200 and r.content[:2] in (b'II', b'MM'):
                    os.makedirs(self.cache, exist_ok=True)
                    with open(datei + '.tmp', 'wb') as f:
                        f.write(r.content)
                    os.replace(datei + '.tmp', datei)
                    self.log(f'    DGM1-Kachel {ek}_{nk} geladen ({len(r.content) // 1024} KB)')
            except Exception as ex:                  # Netzfehler: Kachel fehlt, Karte wird dort flach
                self.log(f'    DGM1-Kachel {ek}_{nk} nicht ladbar: {type(ex).__name__}')
        if os.path.exists(datei):
            import tifffile
            a = tifffile.imread(datei).astype(np.float32)
            a[a < -1000] = np.nan                    # NODATA -9999
        else:
            self.fehlend.add(key)
        self._kacheln[key] = a
        return a

    def hoehe_utm(self, e, n):
        ek, nk = int(e // 1000), int(n // 1000)
        a = self._kachel(ek, nk)
        if a is None:
            return None
        # Pixelmitten: Spalte c bei e = ek*1000 + c + 0.5, Zeile r bei n = (nk+1)*1000 - r - 0.5
        fx = e - ek * 1000 - 0.5
        fy = (nk + 1) * 1000 - n - 0.5
        c0, r0 = int(math.floor(fx)), int(math.floor(fy))
        if c0 < 0 or r0 < 0 or c0 >= 999 or r0 >= 999:     # Rand der Kachel: naechster Wert, Nachbarkachel spart man
            c0, r0 = min(max(int(round(fx)), 0), 999), min(max(int(round(fy)), 0), 999)
            v = a[r0, c0]
            return None if np.isnan(v) else float(v)
        tx, ty = fx - c0, fy - r0
        q = a[r0:r0 + 2, c0:c0 + 2]
        v = (q[0, 0] * (1 - tx) + q[0, 1] * tx) * (1 - ty) + (q[1, 0] * (1 - tx) + q[1, 1] * tx) * ty
        return None if np.isnan(v) else float(v)

    def hoehe(self, x, z):
        """Hoehe ueber NN am Projektpunkt (x Ost, z Nord in Metern) oder None"""
        lat, lon = self.proj.to_ll(x, z)
        return self.hoehe_utm(*to_utm(lat, lon))

    def verfuegbar(self, x, z):
        return self.hoehe(x, z) is not None
