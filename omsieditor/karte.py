"""Kartenansicht: Luftbild, Strassen, Kreuzungspunkte; Werkzeuge Auswaehlen und Strasse zeichnen.

Szenenkoordinaten = Projektmeter mit umgedrehter Hochachse (Szene y = -z), damit Norden oben ist."""
import math, os
from PySide6.QtCore import Qt, Signal, QPointF, QRectF
from PySide6.QtGui import (QPainter, QPen, QColor, QBrush, QPainterPath, QPixmap, QImage, QPainterPathStroker,
                           QWheelEvent)
from PySide6.QtWidgets import (QGraphicsView, QGraphicsScene, QGraphicsPathItem, QGraphicsEllipseItem,
                               QGraphicsRectItem, QGraphicsPixmapItem, QGraphicsSimpleTextItem)
from omsigen.config import choose_spline
from omsigen.splinedb import SplineDB
from . import luftbild as lb
from .modell import klasse_von

FARBE = {'primary': '#e8590c', 'secondary': '#f08c00', 'tertiary': '#fcc419', 'unclassified': '#ced4da',
         'residential': '#e9ecef', 'living_street': '#b2f2bb', 'service': '#adb5bd', 'bus': '#4dabf7'}
FANG_PX = 12          # Einrastradius in Bildschirmpunkten


def _p(x, z):
    return QPointF(x, -z)


class Karte(QGraphicsView):
    auswahl = Signal(object)        # Strassen-ID oder None
    geaendert = Signal()
    status = Signal(str)
    zeichnen_fertig = Signal()
    werkzeug_geaendert = Signal(str)

    def __init__(self, modell, proj, parent=None):
        super().__init__(parent)
        self.m, self.proj = modell, proj
        self.sdb = SplineDB()
        self.scene_ = QGraphicsScene(self)
        self.setScene(self.scene_)
        self.setRenderHints(QPainter.Antialiasing | QPainter.SmoothPixmapTransform)
        self.setViewportUpdateMode(QGraphicsView.SmartViewportUpdate)
        self.setTransformationAnchor(QGraphicsView.AnchorUnderMouse)
        self.setResizeAnchor(QGraphicsView.AnchorViewCenter)
        self.setHorizontalScrollBarPolicy(Qt.ScrollBarAlwaysOff)
        self.setVerticalScrollBarPolicy(Qt.ScrollBarAlwaysOff)
        self.setDragMode(QGraphicsView.NoDrag)
        self.setMouseTracking(True)
        self.setBackgroundBrush(QColor('#2b2d31'))
        self.scene_.setSceneRect(-200000, -200000, 400000, 400000)

        self.werkzeug = 'auswahl'
        self.sel = None
        self.zeichnung = []                # Punkte der Strasse im Bau
        self.neue_tags = {'highway': 'residential'}
        self._strassen, self._griffe, self._kreuz, self._halte = {}, [], [], []
        self._vorschau = self.scene_.addPath(QPainterPath(), QPen(QColor('#ff6b6b'), 0))
        self._vorschau.setZValue(20)
        self._fang = self.scene_.addEllipse(QRectF(-1, -1, 2, 2), QPen(QColor('#ffffff'), 0), QBrush(QColor(255, 80, 80, 160)))
        self._fang.setZValue(21); self._fang.hide()
        self._pan = None
        self._ziehen = None                # (alter Punkt, Griff)

        self.luftbild_an = os.environ.get('OMSIEDITOR_OHNE_LUFTBILD') != '1'    # Tests ohne Netz
        self.lb = lb.Luftbild(proj)
        self.lb.kachel.connect(self._kachel_da)
        self._kacheln = {}
        self.neu_zeichnen()

    # ---------------------------------------------------------------- Umrechnung
    def m_pro_px(self):
        return 1.0 / max(self.transform().m11(), 1e-6)

    def welt(self, pos):
        s = self.mapToScene(pos)
        return (s.x(), -s.y())

    def fang_radius(self):
        return FANG_PX * self.m_pro_px()

    def zentrieren(self, x, z, m_pro_px=None):
        if m_pro_px:
            f = 1.0 / m_pro_px
            self.resetTransform()
            self.scale(f, f)
        self.centerOn(_p(x, z))
        self._luftbild_aktualisieren()

    def alles_zeigen(self):
        pts = [q for s in self.m.p['strassen'] for q in s['punkte']]
        if not pts:
            self.zentrieren(0, 0, 1.0)
            return
        x0, x1 = min(q[0] for q in pts), max(q[0] for q in pts)
        z0, z1 = min(q[1] for q in pts), max(q[1] for q in pts)
        self.fitInView(QRectF(x0 - 30, -z1 - 30, x1 - x0 + 60, z1 - z0 + 60), Qt.KeepAspectRatio)
        self._luftbild_aktualisieren()

    # ---------------------------------------------------------------- Zeichnen der Inhalte
    def _breite(self, tags):
        try:
            spl, _ = choose_spline(tags, False)
            return 2 * self.sdb[spl]['cw'], 2 * self.sdb[spl]['half']
        except KeyError:
            return 6.0, 12.0

    def neu_zeichnen(self):
        for it in list(self._strassen.values()) + self._kreuz + self._halte:
            self.scene_.removeItem(it)
        self._strassen, self._kreuz, self._halte = {}, [], []
        for s in self.m.p['strassen']:
            P = s['punkte']
            if len(P) < 2:
                continue
            path = QPainterPath(_p(*P[0]))
            for q in P[1:]:
                path.lineTo(_p(*q))
            fb, ges = self._breite(s['tags'])
            farbe = QColor(FARBE.get(klasse_von(s['tags']), '#ced4da'))
            it = QGraphicsPathItem(path)
            gesamt = QColor(farbe); gesamt.setAlpha(70)
            pen = QPen(gesamt, ges); pen.setCapStyle(Qt.FlatCap); pen.setJoinStyle(Qt.RoundJoin)
            it.setPen(pen)
            innen = QGraphicsPathItem(path, it)
            fahr = QColor(farbe); fahr.setAlpha(190)
            p2 = QPen(fahr, fb); p2.setCapStyle(Qt.FlatCap); p2.setJoinStyle(Qt.RoundJoin)
            innen.setPen(p2)
            mitte = QGraphicsPathItem(path, it)
            pm = QPen(QColor(40, 40, 40, 200), 0); pm.setCosmetic(True)
            if s['tags'].get('oneway') in ('yes', '-1'):
                pm.setStyle(Qt.DashLine)
            mitte.setPen(pm)
            it.setData(0, s['id'])
            it.setZValue(1 + (0.5 if s['id'] == self.sel else 0))
            self.scene_.addItem(it)
            self._strassen[s['id']] = it
        for k in self.m.kreuzungspunkte():
            r = 3.0
            e = self.scene_.addEllipse(QRectF(k[0] - r, -k[1] - r, 2 * r, 2 * r), QPen(QColor('#ffffff'), 0),
                                       QBrush(QColor(30, 30, 30, 200)))
            e.setZValue(5)
            self._kreuz.append(e)
        for h in self.m.p.get('haltestellen', []):
            r = 2.5
            e = self.scene_.addRect(QRectF(h['p'][0] - r, -h['p'][1] - r, 2 * r, 2 * r), QPen(QColor('#ffffff'), 0),
                                    QBrush(QColor('#1c7ed6')))
            e.setZValue(6); e.setToolTip(h['name'])
            self._halte.append(e)
        self._auswahl_zeigen()

    def _auswahl_zeigen(self):
        for g in self._griffe:
            self.scene_.removeItem(g)
        self._griffe = []
        for sid, it in self._strassen.items():
            hl = sid == self.sel
            it.setZValue(1.5 if hl else 1)
            it.setGraphicsEffect(None)
            p = it.pen()
            c = QColor('#ff3b30') if hl else QColor(FARBE.get(klasse_von(self.m.strasse(sid)['tags']), '#ced4da'))
            c.setAlpha(150 if hl else 70)
            p.setColor(c)
            it.setPen(p)
        s = self.m.strasse(self.sel) if self.sel else None
        if s:
            r = 6 * self.m_pro_px()
            for i, q in enumerate(s['punkte']):
                g = self.scene_.addRect(QRectF(q[0] - r / 2, -q[1] - r / 2, r, r), QPen(QColor('#000000'), 0),
                                        QBrush(QColor('#ffffff')))
                g.setZValue(10); g.setData(0, i)
                self._griffe.append(g)

    # ---------------------------------------------------------------- Luftbild
    def luftbild_schalten(self, an):
        self.luftbild_an = an
        for it in self._kacheln.values():
            it.setVisible(an)
        self._luftbild_aktualisieren()

    def _luftbild_aktualisieren(self):
        if not self.luftbild_an:
            return
        r = self.mapToScene(self.viewport().rect()).boundingRect()
        k = lb.stufe_fuer(self.m_pro_px())
        x0, x1, z0, z1 = r.left(), r.right(), -r.bottom(), -r.top()
        n = 0
        for kachel in lb.kacheln_im_bereich(x0, z0, x1, z1, k):
            n += 1
            if n > 80:
                break
            self.lb.bestellen(kachel)

    def _kachel_da(self, kachel, daten):
        img = QImage.fromData(daten)
        if img.isNull():
            return
        x0, z0, x1, z1 = lb.kachel_rechteck(kachel)
        it = QGraphicsPixmapItem(QPixmap.fromImage(img))
        it.setTransformationMode(Qt.SmoothTransformation)
        it.setScale((x1 - x0) / img.width())
        it.setPos(x0, -z1)
        it.setZValue(-10 - kachel[0])
        it.setVisible(self.luftbild_an)
        self.scene_.addItem(it)
        self._kacheln[kachel] = it

    # ---------------------------------------------------------------- Werkzeuge
    def werkzeug_setzen(self, w):
        self.werkzeug = w
        self.werkzeug_geaendert.emit(w)
        self.zeichnung = []
        self._vorschau.setPath(QPainterPath())
        self.setCursor(Qt.CrossCursor if w == 'zeichnen' else Qt.ArrowCursor)
        self.status.emit('Strasse zeichnen: Klick setzt Punkte (rastet an Strassen ein), Doppelklick/Enter/Rechtsklick '
                         'beendet, Esc bricht ab' if w == 'zeichnen' else
                         'Auswaehlen: Klick auf Strasse, Punkte ziehen, Alt+Klick loescht Punkt, Entf loescht Strasse')

    def auswaehlen(self, sid):
        self.sel = sid
        self._auswahl_zeigen()
        self.auswahl.emit(sid)

    def _strasse_unter(self, pos):
        hit = self.m.naechste_strasse(self.welt(pos), 8 * self.m_pro_px())
        return hit[0] if hit else None

    def _griff_unter(self, pos):
        if not self.sel:
            return None
        p = self.welt(pos)
        s = self.m.strasse(self.sel)
        for i, q in enumerate(s['punkte']):
            if math.dist(p, q) <= 8 * self.m_pro_px():
                return i
        return None

    def _vorschau_setzen(self, maus=None):
        P = list(self.zeichnung) + ([maus] if maus else [])
        path = QPainterPath()
        if P:
            path.moveTo(_p(*P[0]))
            for q in P[1:]:
                path.lineTo(_p(*q))
        pen = QPen(QColor('#ff6b6b'), 3); pen.setCosmetic(True)
        self._vorschau.setPen(pen)
        self._vorschau.setPath(path)

    def zeichnen_beenden(self):
        if len(self.zeichnung) >= 2:
            sid = self.m.strasse_hinzufuegen(self.zeichnung, self.neue_tags)
            self.zeichnung = []
            self._vorschau_setzen()
            self.neu_zeichnen()
            self.geaendert.emit()
            if sid:
                self.auswaehlen(sid)
        else:
            self.zeichnung = []
            self._vorschau_setzen()
        self.zeichnen_fertig.emit()

    # ---------------------------------------------------------------- Maus und Tastatur
    def wheelEvent(self, ev: QWheelEvent):
        f = math.pow(1.0015, ev.angleDelta().y())
        neu = self.transform().m11() * f
        if 0.01 < neu < 60:
            self.scale(f, f)
        self._auswahl_zeigen()
        self._luftbild_aktualisieren()

    def mousePressEvent(self, ev):
        if ev.button() in (Qt.MiddleButton, Qt.RightButton):
            self._pan = (ev.position(), False, ev.button())
            return
        if ev.button() != Qt.LeftButton:
            return
        p = self.welt(ev.position().toPoint())
        if self.werkzeug == 'zeichnen':
            q = self.m.einrasten(p, self.fang_radius(), aendern=True)
            if not self.zeichnung or math.dist(q, self.zeichnung[-1]) > 0.05:
                self.zeichnung.append(q)
            self._vorschau_setzen()
            self.neu_zeichnen()
            return
        i = self._griff_unter(ev.position().toPoint())
        if i is not None:
            if ev.modifiers() & Qt.AltModifier:
                if self.m.punkt_loeschen(self.sel, i):
                    self.neu_zeichnen(); self.geaendert.emit()
                return
            alt = tuple(self.m.strasse(self.sel)['punkte'][i])
            self.m._merken()
            self._ziehen = [alt, alt]
            return
        self.auswaehlen(self._strasse_unter(ev.position().toPoint()))

    def mouseMoveEvent(self, ev):
        pos = ev.position()
        p = self.welt(pos.toPoint())
        la, lo = self.proj.to_ll(*p)
        self.status.emit(f'x {p[0]:.1f} m  z {p[1]:.1f} m   |   {la:.6f}, {lo:.6f}')
        if self._pan:
            start, bewegt, knopf = self._pan
            d = pos - start
            if abs(d.x()) + abs(d.y()) > 3 or bewegt:
                self.horizontalScrollBar().setValue(self.horizontalScrollBar().value() - d.x())
                self.verticalScrollBar().setValue(self.verticalScrollBar().value() - d.y())
                self._pan = (pos, True, knopf)
                self._luftbild_aktualisieren()
            return
        if self._ziehen:
            ziel = self.m.naechster_punkt(p, self.fang_radius(), ausser=None)
            if ziel and math.dist(ziel, self._ziehen[0]) < 0.01:
                ziel = None
            neu = ziel or p
            self.m.punkt_verschieben(self._ziehen[1], neu, merken=False)
            self._ziehen[1] = tuple(neu)
            self.neu_zeichnen()
            return
        if self.werkzeug == 'zeichnen':
            q = self.m.naechster_punkt(p, self.fang_radius())
            hit = None if q else self.m.naechste_strasse(p, self.fang_radius() * 0.8)
            ziel = q or (hit[2] if hit else None)
            if ziel:
                r = 2.5 * self.m_pro_px() * 2
                self._fang.setRect(QRectF(ziel[0] - r, -ziel[1] - r, 2 * r, 2 * r)); self._fang.show()
            else:
                self._fang.hide()
            self._vorschau_setzen(ziel or p if self.zeichnung else None)

    def mouseReleaseEvent(self, ev):
        if self._pan and ev.button() == self._pan[2]:
            bewegt = self._pan[1]
            self._pan = None
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

    def keyPressEvent(self, ev):
        if ev.key() in (Qt.Key_Return, Qt.Key_Enter) and self.werkzeug == 'zeichnen':
            self.zeichnen_beenden()
        elif ev.key() == Qt.Key_Escape:
            if self.zeichnung:
                self.zeichnung = []
                self._vorschau_setzen()
            else:
                self.auswaehlen(None)
        elif ev.key() == Qt.Key_Delete and self.sel:
            self.m.strasse_loeschen(self.sel)
            self.sel = None
            self.neu_zeichnen()
            self.auswahl.emit(None)
            self.geaendert.emit()
        else:
            super().keyPressEvent(ev)

    def resizeEvent(self, ev):
        super().resizeEvent(ev)
        self._luftbild_aktualisieren()
