"""3D-Hauptansicht des Editors (wie Transport Fever 2): Gelaende mit Luftbild, die Karte so, wie omsigen sie baut
(Live-Vorschau im Hintergrund nach jeder Aenderung), Werkzeuge Auswaehlen und Strasse zeichnen in 3D.

Bedienung
  Rechte Maustaste ziehen: drehen/neigen      Mittlere Maustaste ziehen: verschieben     Mausrad: zoomen
  W A S D / Pfeile: verschieben    Q / E: drehen    R / F: neigen
  Zeichnen: Klick setzt Punkte (rastet ein), Bild auf/ab hebt/senkt den naechsten Punkt (Umschalt: 5 m),
            Enter/Doppelklick/Rechtsklick beendet, Esc bricht ab. Hoch ueber dem Gelaende wird die Strasse zur
            Bruecke (gelb), tief darunter zum Tunnel (blau).
  Auswaehlen: Klick auf Strasse, Punkte ziehen, Bild auf/ab aendert die Hoehe des gewaehlten Punkts,
              Pos1 = Punkt folgt wieder dem Gelaende, Entf loescht die Strasse.

Gleiche Schnittstelle wie karte.Karte (Signale, werkzeug_setzen, auswaehlen, neu_zeichnen, ...), damit das Fenster
zwischen 2D und 3D umschalten kann."""
import copy, math, os
import numpy as np
from PySide6.QtCore import Qt, Signal, QTimer
from PySide6.QtOpenGLWidgets import QOpenGLWidget
from omsigen import pipeline, ebenen
from omsigen.config import DEFAULT_OMSI
from omsigen.gelaende import Gelaende
from . import geometrie3d as g3
from . import luftbild as lb
from .render3d import Renderer, Kamera, treffer, band, wuerfel
from .modell import klasse_von
from .dialoge import Arbeit

FANG_PX = 14
FARBE_ART = {None: (1.0, 0.35, 0.3, 0.95), 'bruecke': (1.0, 0.85, 0.2, 0.95), 'tunnel': (0.35, 0.65, 1.0, 0.95)}
FARBE_KLASSE = {'primary': (0.91, 0.35, 0.05), 'secondary': (0.94, 0.55, 0.0), 'tertiary': (0.99, 0.77, 0.1),
                'bus': (0.3, 0.67, 0.97)}


class Ansicht3D(QOpenGLWidget):
    auswahl = Signal(object)
    geaendert = Signal()
    status = Signal(str)
    zeichnen_fertig = Signal()
    werkzeug_geaendert = Signal(str)
    punkt_gewaehlt = Signal(object)      # (x, z) des gewaehlten Punkts oder None
    vorschau_fertig = Signal(str)

    def __init__(self, modell, proj, parent=None, omsi=None, gel=None):
        super().__init__(parent)
        self.m, self.proj = modell, proj
        self.omsi = omsi if omsi is not None else DEFAULT_OMSI
        self.gel = gel or Gelaende(proj, log=lambda *a: None)
        self.setFocusPolicy(Qt.StrongFocus)
        self.setMouseTracking(True)
        self.r = Renderer()
        self.gl_ok = False
        self.kam = Kamera()
        pts = [q for s in self.m.p['strassen'] for q in s['punkte']] or [(0.0, 0.0)]
        ox = g3.TILE * math.floor((min(q[0] for q in pts) - 20) / g3.TILE)
        oz = g3.TILE * math.floor((min(q[1] for q in pts) - 20) / g3.TILE)
        self.raster = g3.Raster(ox, oz)
        self.basis = None                     # Hoehe ue. NN, auf die sich alle angezeigten Hoehen beziehen
        self.gras = g3.textur_finden('gras.bmp', g3.textur_ordner(self.omsi))
        self.luftbild_an = os.environ.get('OMSIEDITOR_OHNE_LUFTBILD') != '1'
        self.lb = lb.Luftbild(proj)
        self.lb.kachel.connect(self._luftbild_da)
        # ausstehende GL-Arbeiten (werden in paintGL mit aktuellem Kontext erledigt)
        self._neu_szene, self._neu_kacheln, self._neu_bilder = None, set(), {}
        self._bilder = {}                     # (x0, z0) -> JPEG, damit Kacheln nach Verschiebung sofort Bilder haben
        self._reset = False
        self._hilfe_neu = True
        # Werkzeuge
        self.werkzeug, self.sel, self.griff = 'auswahl', None, None
        self.zeichnung, self.zeich_hoehen, self.zeich_h = [], [], 0.0
        self.neue_tags = {'highway': 'residential'}
        self._maus, self._fang, self._drag, self._ziehen = None, None, None, None
        self.achsen = False                   # Strassenachsen immer zeigen (Taste X)
        # Vorschau
        self.veraltet = True
        self._arbeit, self._erneut, self._roh = None, False, None
        self._timer = QTimer(self); self._timer.setSingleShot(True); self._timer.timeout.connect(self._vorschau_starten)
        self.kamera_auf_alles()
        self._roh_laden()

    # ---------------------------------------------------------------- Gelaende und Vorschau
    def _roh_laden(self):
        """Gelaende direkt aus dem DGM, bevor die erste Vorschau fertig ist"""
        pts = [tuple(q) for s in self.m.p['strassen'] for q in s['punkte']] or [(0.0, 0.0)]
        kacheln = g3.kacheln_fuer(pts, self.raster.ox, self.raster.oz)
        gel = self.gel

        def job(log):
            proben = [gel.hoehe(*p) for p in pts[::max(1, len(pts) // 20)]]
            proben = [h for h in proben if h is not None]
            basis = math.floor(min(proben)) - 2.0 if proben else 0.0
            R = g3.Raster(self.raster.ox, self.raster.oz)
            g3.roh_raster(R, kacheln, gel if proben else None, basis)
            return R, basis
        self._roh = Arbeit(job, self)
        self._roh.fertig.connect(self._roh_fertig)
        self._roh.start()

    def _roh_fertig(self, erg, fehler):
        if fehler or not erg:
            self.status.emit(f'Gelaende nicht geladen: {fehler}')
            erg = (self.raster, 0.0)
        R, basis = erg
        if self.basis is None:
            self.basis = basis
        if not self.raster.kacheln:              # die Vorschau war nicht schneller
            self.raster = R
            self._neu_kacheln |= set(R.kacheln)
            self._luftbilder_bestellen()
            self.kamera_auf_alles()
        self.vorschau_anstossen(0)
        self.update()

    def vorschau_anstossen(self, ms=700):
        self.veraltet = True
        self._timer.start(ms)

    def _vorschau_starten(self):
        if self.basis is None:                  # erst das Gelaende
            return
        if self._arbeit and self._arbeit.isRunning():
            self._erneut = True
            return
        if not any(len(s['punkte']) >= 2 for s in self.m.p['strassen']):
            self.veraltet = False
            self._neu_szene = {}
            self.update()
            return
        projekt, basis, omsi, gel = copy.deepcopy(self.m.p), self.basis, self.omsi, self.gel

        def job(log):
            B = pipeline.berechne(projekt, omsi=omsi, gel=gel, log=lambda *a: None)
            szene, raster, base = g3.szene_aus_berechnung(B, omsi)
            arrays = szene.fertig()
            off = (base - basis) if B['hat_gelaende'] else 0.0
            for a in arrays.values():
                a[:, 1] += off
            for t in raster.kacheln:
                raster.kacheln[t] = raster.kacheln[t] + off
            st, bw = B['net']['stats'], B['bw']['stats']
            info = (f"Vorschau: {st['kreuzungen']} Kreuzungen, {bw['bruecken']} Bruecken, {bw['tunnel']} Tunnel, "
                    f"{sum(len(a) for a in arrays.values()) // 3} Dreiecke")
            return arrays, raster, info
        self.status.emit('Vorschau wird berechnet ...')
        self._arbeit = Arbeit(job, self)
        self._arbeit.fertig.connect(self._vorschau_fertig)
        self._arbeit.start()

    def _vorschau_fertig(self, erg, fehler):
        if fehler:
            self.status.emit(f'Vorschau fehlgeschlagen: {fehler}')
        elif erg:
            arrays, raster, info = erg
            self._neu_szene = arrays
            verschoben = (raster.ox, raster.oz) != (self.raster.ox, self.raster.oz)
            neu = set(raster.kacheln) - set(self.raster.kacheln)
            self.raster = raster
            self._neu_kacheln |= set(raster.kacheln)
            if verschoben:                       # Kachelnummern bedeuten jetzt andere Orte
                self._reset = True
                for t in raster.kacheln:
                    d = self._bilder.get(raster.kachel_ecke(t))
                    if d:
                        self._neu_bilder[t] = d
            if neu or verschoben:
                self._luftbilder_bestellen()
            self.veraltet = False
            self._hilfe_neu = True
            self.status.emit(info)
            self.vorschau_fertig.emit(info)
        if self._erneut:
            self._erneut = False
            self._vorschau_starten()
        self.update()

    def _luftbilder_bestellen(self):
        if not self.luftbild_an:
            return
        for t in self.raster.kacheln:
            x0, z0 = self.raster.kachel_ecke(t)
            self.lb.bestellen(('3d', x0, z0), (x0, z0, x0 + g3.TILE, z0 + g3.TILE), pixel=1024)

    def _luftbild_da(self, kachel, daten):
        if kachel[0] != '3d':
            return
        x0, z0 = kachel[1], kachel[2]
        self._bilder[(x0, z0)] = daten
        t = (round((x0 - self.raster.ox) / g3.TILE), round((z0 - self.raster.oz) / g3.TILE))
        self._neu_bilder[t] = daten
        self.update()

    def luftbild_schalten(self, an):
        self.luftbild_an = an
        if an:
            self._luftbilder_bestellen()
        self.update()

    # ---------------------------------------------------------------- GL
    def initializeGL(self):
        try:
            self.r.init()
            self.gl_ok = True
        except Exception as ex:                  # z. B. ohne Grafikkarte (Tests): Ansicht bleibt leer
            self.gl_ok = False
            self.status.emit(f'3D-Ansicht nicht verfuegbar: {ex}')

    def paintGL(self):
        if not self.gl_ok:
            return
        if self._neu_szene is not None:
            self.r.bauten_setzen(self._neu_szene)
            self._neu_szene = None
        if self._reset:
            self.r.gelaende_leeren()
            self.r.luftbilder_leeren()
            self._reset = False
        if self._neu_kacheln:
            for t in list(self.r.gelaende):
                if t not in self.raster.kacheln:
                    self.r.gelaende.pop(t).weg()
            for t in self._neu_kacheln:
                if t in self.raster.kacheln:
                    self.r.gelaende_setzen(t, self.raster.dreiecke(t))
            self._neu_kacheln = set()
        for t, d in self._neu_bilder.items():
            self.r.luftbild_setzen(t, d)
        self._neu_bilder = {}
        if self._hilfe_neu:
            self._hilfe_bauen()
            self._hilfe_neu = False
        dpr = self.devicePixelRatioF()
        self.r.zeichnen(self.kam, int(self.width() * dpr), int(self.height() * dpr), gras=self.gras,
                        luftbild_an=self.luftbild_an)

    # ---------------------------------------------------------------- Hoehen und Hilfslinien
    def boden(self, x, z):
        h = self.raster.hoehe(x, z)
        return 0.0 if h is None else h

    def _linie(self, P, H=None):
        """Strassenachse in 3D: zwischen Punkten mit Vorgabe gerade, sonst auf dem Gelaende -> [(x, y, z)], Arten"""
        H = H or [None] * len(P)
        out, arten = [], []
        for i in range(len(P) - 1):
            a, b = P[i], P[i + 1]
            n = max(1, int(math.ceil(math.dist(a, b) / 4.0)))
            ya = self.boden(*a) + (H[i] or 0.0)
            yb = self.boden(*b) + (H[i + 1] or 0.0)
            gerade = H[i] is not None or H[i + 1] is not None
            for k in range(n + (1 if i == len(P) - 2 else 0)):
                t = k / n
                x, z = a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t
                g = self.boden(x, z)
                y = ya + (yb - ya) * t if gerade else g
                out.append((x, y, z))
                arten.append(ebenen.art_bei(y, g) if gerade else None)
        return out, arten

    def _bunt(self, pts, arten, breite):
        """Band, abschnittsweise nach Bruecke/Tunnel/Boden gefaerbt"""
        out = []
        for i in range(len(pts) - 1):
            out += band(pts[i:i + 2], breite, FARBE_ART[arten[i + 1] or arten[i]])
        return out

    def _griff_r(self):
        return min(6.0, max(0.5, self.kam.abstand * 0.006))

    def _hilfe_bauen(self):
        unten, oben = [], []
        b = max(0.8, self.kam.abstand * 0.003)
        if self.achsen or self.veraltet:
            for s in self.m.p['strassen']:
                if len(s['punkte']) < 2 or s['id'] == self.sel:
                    continue
                pts, _ = self._linie(s['punkte'], s.get('hoehen'))
                c = FARBE_KLASSE.get(klasse_von(s['tags']), (0.95, 0.95, 0.95))
                unten += band(pts, b, c + (0.85,))
        s = self.m.strasse(self.sel) if self.sel else None
        if s:
            pts, arten = self._linie(s['punkte'], s.get('hoehen'))
            oben += band(pts, b * 1.6, (1.0, 0.23, 0.19, 0.9))
            for i, q in enumerate(s['punkte']):
                y = self.boden(*q) + ((s.get('hoehen') or [None] * len(s['punkte']))[i] or 0.0)
                oben += wuerfel((q[0], y + self._griff_r(), q[1]), self._griff_r(),
                                (1, 0.85, 0.1, 1) if i == self.griff else (1, 1, 1, 1))
        if self.zeichnung:
            P = list(self.zeichnung) + ([self._maus_xz()] if self._maus else [])
            H = list(self.zeich_hoehen) + ([self.zeich_h] if self._maus else [])
            pts, arten = self._linie(P, H)
            oben += self._bunt(pts, arten, b * 2.5)
        if self._fang:
            q = self._fang
            oben += wuerfel((q[0], self.boden(*q) + self._griff_r(), q[1]), self._griff_r() * 1.3, (1, 0.3, 0.3, 0.8))
        elif self.werkzeug == 'zeichnen' and self._maus:
            x, y, z = self._maus
            oben += wuerfel((x, self.boden(x, z) + self.zeich_h + self._griff_r(), z), self._griff_r(),
                            FARBE_ART[ebenen.art_bei(self.boden(x, z) + self.zeich_h, self.boden(x, z))])
        self.r.hilfe_setzen(np.array(unten, dtype=np.float32).reshape(-1, 7) if unten else None,
                            np.array(oben, dtype=np.float32).reshape(-1, 7) if oben else None)

    def neu_zeichnen(self):
        """nach Aenderungen am Modell: Hilfslinien neu, Vorschau anstossen"""
        self._hilfe_neu = True
        self.vorschau_anstossen()
        self.update()

    # ---------------------------------------------------------------- Kamera
    def kamera_auf_alles(self):
        pts = [q for s in self.m.p['strassen'] for q in s['punkte']]
        if not pts:
            self.kam.ziel = [0.0, self.boden(0, 0), 0.0]
            self.kam.abstand = 400.0
            return
        x0, x1 = min(q[0] for q in pts), max(q[0] for q in pts)
        z0, z1 = min(q[1] for q in pts), max(q[1] for q in pts)
        cx, cz = (x0 + x1) / 2, (z0 + z1) / 2
        self.kam.ziel = [cx, self.boden(cx, cz), cz]
        self.kam.abstand = max(150.0, 0.9 * max(x1 - x0, z1 - z0))
        self.kam.neigung = -50.0
        self._hilfe_neu = True
        self.update()

    def alles_zeigen(self):
        self.kamera_auf_alles()

    def zentrieren(self, x, z, m_pro_px=None):
        self.kam.ziel = [x, self.boden(x, z), z]
        if m_pro_px:
            self.kam.abstand = max(50.0, m_pro_px * 800)
        self.update()

    def _ziel_nachfuehren(self):
        self.kam.ziel[1] = self.boden(self.kam.ziel[0], self.kam.ziel[2])

    def _boden_unter(self, pos):
        o, d = self.kam.strahl(pos.x(), pos.y(), max(self.width(), 1), max(self.height(), 1))
        return treffer(o, d, self.raster.hoehe)

    def _bildschirm(self, p):
        """Weltpunkt -> Bildschirmpunkt (x, y) oder None"""
        vp, _ = self.kam.matrizen(max(self.width(), 1), max(self.height(), 1))
        c = vp @ np.array([p[0], p[1], -p[2], 1.0])
        if c[3] <= 0:
            return None
        return ((c[0] / c[3] + 1) / 2 * self.width(), (1 - c[1] / c[3]) / 2 * self.height())

    def _m_pro_px(self):
        return self.kam.abstand * 2 * math.tan(math.radians(self.kam.fov / 2)) / max(self.height(), 1)

    def _maus_xz(self):
        return self._fang or (self._maus[0], self._maus[2])

    # ---------------------------------------------------------------- Werkzeuge
    def werkzeug_setzen(self, w):
        self.werkzeug = w
        self.werkzeug_geaendert.emit(w)
        self.zeichnung, self.zeich_hoehen = [], []
        self.setCursor(Qt.CrossCursor if w == 'zeichnen' else Qt.ArrowCursor)
        self.status.emit('Zeichnen: Klick setzt Punkte, Bild auf/ab = Hoehe des naechsten Punkts (gelb Bruecke, blau '
                         'Tunnel), Enter/Rechtsklick beendet, Esc bricht ab' if w == 'zeichnen' else
                         'Auswaehlen: Klick auf Strasse, Punkte ziehen, Bild auf/ab = Punkthoehe, Pos1 = folgt Gelaende, '
                         'Entf loescht | Kamera: rechte Taste drehen, mittlere verschieben, Rad zoomen, WASD')
        self._hilfe_neu = True
        self.update()

    def auswaehlen(self, sid):
        self.sel = sid
        self.griff = None
        self._hilfe_neu = True
        self.update()
        self.auswahl.emit(sid)
        self.punkt_gewaehlt.emit(None)

    def zeichnen_beenden(self):
        if len(self.zeichnung) >= 2:
            sid = self.m.strasse_hinzufuegen(self.zeichnung, self.neue_tags, self.zeich_hoehen)
            self.zeichnung, self.zeich_hoehen = [], []
            self.neu_zeichnen()
            self.geaendert.emit()
            if sid:
                self.auswaehlen(sid)
        else:
            self.zeichnung, self.zeich_hoehen = [], []
            self._hilfe_neu = True
            self.update()
        self.zeichnen_fertig.emit()

    def _griff_unter(self, pos):
        s = self.m.strasse(self.sel) if self.sel else None
        if not s:
            return None
        H = s.get('hoehen') or [None] * len(s['punkte'])
        best = None
        for i, q in enumerate(s['punkte']):
            sp = self._bildschirm((q[0], self.boden(*q) + (H[i] or 0.0) + self._griff_r(), q[1]))
            if sp:
                d = math.hypot(sp[0] - pos.x(), sp[1] - pos.y())
                if d <= FANG_PX and (best is None or d < best[0]):
                    best = (d, i)
        return best[1] if best else None

    def _hoehe_aendern(self, d):
        if self.werkzeug == 'zeichnen':
            self.zeich_h = round(self.zeich_h + d, 2)
            self.status.emit(f'naechster Punkt {self.zeich_h:+.1f} m ueber Gelaende')
        elif self.sel and self.griff is not None:
            q = tuple(self.m.strasse(self.sel)['punkte'][self.griff])
            alt = self.m.punkt_hoehe(q)
            neu = (alt or 0.0) + d if d is not None else None
            self.m.punkt_hoehe_setzen(q, neu)
            self.status.emit('Punkt folgt dem Gelaende' if neu is None else f'Punkt {neu:+.1f} m ueber Gelaende')
            self.punkt_gewaehlt.emit(q)
            self.neu_zeichnen()
            self.geaendert.emit()
            return
        self._hilfe_neu = True
        self.update()

    # ---------------------------------------------------------------- Maus und Tastatur
    def mousePressEvent(self, ev):
        pos = ev.position()
        if ev.button() in (Qt.RightButton, Qt.MiddleButton):
            self._drag = [ev.button(), pos, False]
            return
        if ev.button() != Qt.LeftButton:
            return
        hit = self._boden_unter(pos)
        if self.werkzeug == 'zeichnen':
            if not hit:
                return
            r = FANG_PX * self._m_pro_px()
            q = self.m.einrasten((hit[0], hit[2]), r, aendern=True)
            if not self.zeichnung or math.dist(q, self.zeichnung[-1]) > 0.05:
                vorh = self.m.punkt_hoehe(q)
                self.zeichnung.append(q)
                self.zeich_hoehen.append(vorh if vorh is not None else self.zeich_h)
            self._hilfe_neu = True
            self.update()
            return
        i = self._griff_unter(pos)
        if i is not None:
            self.griff = i
            q = tuple(self.m.strasse(self.sel)['punkte'][i])
            self.punkt_gewaehlt.emit(q)
            self.m._merken()
            self._ziehen = [q, q]
            self._hilfe_neu = True
            self.update()
            return
        if hit:
            t = self.m.naechste_strasse((hit[0], hit[2]), max(3.0, 10 * self._m_pro_px()))
            self.auswaehlen(t[0] if t else None)

    def mouseMoveEvent(self, ev):
        pos = ev.position()
        if self._drag:
            knopf, start, _ = self._drag
            d = pos - start
            if abs(d.x()) + abs(d.y()) > 2 or self._drag[2]:
                self._drag = [knopf, pos, True]
                if knopf == Qt.RightButton:
                    self.kam.gier = (self.kam.gier + d.x() * 0.3) % 360
                    self.kam.neigung = min(-5.0, max(-89.0, self.kam.neigung - d.y() * 0.3))
                else:
                    f = self._m_pro_px()
                    self.kam.verschieben(-d.x() * f, d.y() * f / max(0.2, math.sin(math.radians(-self.kam.neigung))))
                    self._ziel_nachfuehren()
                self._hilfe_neu = True
                self.update()
            return
        hit = self._boden_unter(pos)
        self._maus = hit
        if hit:
            la, lo = self.proj.to_ll(hit[0], hit[2])
            hoehe = (hit[1] + self.basis) if self.basis is not None else hit[1]
            self.status.emit(f'x {hit[0]:.1f} m  z {hit[2]:.1f} m  Hoehe {hoehe:.1f} m ue. NN   |   {la:.6f}, {lo:.6f}')
        if self._ziehen and hit:
            r = FANG_PX * self._m_pro_px()
            ziel = self.m.naechster_punkt((hit[0], hit[2]), r)
            if ziel and math.dist(ziel, self._ziehen[0]) < 0.01:
                ziel = None
            neu = ziel or (hit[0], hit[2])
            self.m.punkt_verschieben(self._ziehen[1], neu, merken=False)
            self._ziehen[1] = tuple(neu)
            self._hilfe_neu = True
            self.update()
            return
        if self.werkzeug == 'zeichnen':
            self._fang = None
            if hit:
                r = FANG_PX * self._m_pro_px()
                q = self.m.naechster_punkt((hit[0], hit[2]), r)
                t = None if q else self.m.naechste_strasse((hit[0], hit[2]), r * 0.8)
                self._fang = q or (t[2] if t else None)
            self._hilfe_neu = True
            self.update()

    def mouseReleaseEvent(self, ev):
        if self._drag and ev.button() == self._drag[0]:
            bewegt = self._drag[2]
            self._drag = None
            if not bewegt and ev.button() == Qt.RightButton and self.werkzeug == 'zeichnen':
                self.zeichnen_beenden()
            return
        if self._ziehen:
            self._ziehen = None
            self.neu_zeichnen()
            self.geaendert.emit()

    def mouseDoubleClickEvent(self, ev):
        if self.werkzeug == 'zeichnen' and ev.button() == Qt.LeftButton:
            self.zeichnen_beenden()

    def wheelEvent(self, ev):
        f = math.pow(0.9985, ev.angleDelta().y())
        hit = self._boden_unter(ev.position())
        alt = self.kam.abstand
        self.kam.abstand = min(6000.0, max(8.0, alt * f))
        if hit:                                 # zum Mauspunkt hin zoomen
            k = 1 - self.kam.abstand / alt
            self.kam.ziel[0] += (hit[0] - self.kam.ziel[0]) * k
            self.kam.ziel[2] += (hit[2] - self.kam.ziel[2]) * k
            self._ziel_nachfuehren()
        self._hilfe_neu = True
        self.update()

    def keyPressEvent(self, ev):
        k, shift = ev.key(), bool(ev.modifiers() & Qt.ShiftModifier)
        schritt = self.kam.abstand * 0.05
        bewegen = {Qt.Key_W: (0, schritt), Qt.Key_Up: (0, schritt), Qt.Key_S: (0, -schritt),
                   Qt.Key_Down: (0, -schritt), Qt.Key_A: (-schritt, 0), Qt.Key_Left: (-schritt, 0),
                   Qt.Key_D: (schritt, 0), Qt.Key_Right: (schritt, 0)}
        if k in bewegen:
            self.kam.verschieben(*bewegen[k]); self._ziel_nachfuehren()
        elif k in (Qt.Key_Q, Qt.Key_E):
            self.kam.gier = (self.kam.gier + (-10 if k == Qt.Key_Q else 10)) % 360
        elif k in (Qt.Key_R, Qt.Key_F):
            self.kam.neigung = min(-5.0, max(-89.0, self.kam.neigung + (5 if k == Qt.Key_R else -5)))
        elif k in (Qt.Key_PageUp, Qt.Key_PageDown):
            self._hoehe_aendern((5.0 if shift else 1.0) * (1 if k == Qt.Key_PageUp else -1))
            return
        elif k == Qt.Key_Home:
            if self.werkzeug == 'zeichnen':
                self.zeich_h = 0.0
                self._hilfe_neu = True
            else:
                self._hoehe_aendern(None)
            return
        elif k == Qt.Key_X:
            self.achsen = not self.achsen
        elif k in (Qt.Key_Return, Qt.Key_Enter) and self.werkzeug == 'zeichnen':
            self.zeichnen_beenden()
            return
        elif k == Qt.Key_Escape:
            if self.zeichnung:
                self.zeichnung, self.zeich_hoehen = [], []
            else:
                self.auswaehlen(None)
        elif k == Qt.Key_Delete and self.sel:
            self.m.strasse_loeschen(self.sel)
            self.sel = None
            self.auswahl.emit(None)
            self.neu_zeichnen()
            self.geaendert.emit()
            return
        else:
            super().keyPressEvent(ev)
            return
        self._hilfe_neu = True
        self.update()
