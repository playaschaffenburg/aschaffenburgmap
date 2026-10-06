"""Hauptfenster: Werkzeugleiste, 3D-Ansicht (Hauptansicht, wie Transport Fever) bzw. 2D-Luftbildkarte (F2),
Eigenschaften der gewaehlten Strasse und des gewaehlten Punkts, Statuszeile."""
import os
from PySide6.QtCore import Qt
from PySide6.QtGui import QAction, QKeySequence, QActionGroup
from PySide6.QtWidgets import (QMainWindow, QDockWidget, QWidget, QFormLayout, QLineEdit, QComboBox, QSpinBox,
                               QCheckBox, QLabel, QPushButton, QFileDialog, QMessageBox, QVBoxLayout, QStackedWidget,
                               QDoubleSpinBox, QGroupBox)
from omsigen.route import Projection
from omsigen.gelaende import Gelaende
from . import luftbild as lb
from .karte import Karte
from .modell import KLASSEN, KLASSEN_TEXT, standard_tags, klasse_von
from .dialoge import ExportDialog

EINBAHN = [('nein', 'nein'), ('ja', 'ja (in Zeichenrichtung)'), ('entgegen', 'entgegen der Zeichenrichtung')]
TEMPO = ['', '30', '50', '70']
EBENE = [('auto', 'automatisch (aus den Hoehen)'), ('boden', 'Boden (folgt dem Gelaende)'), ('bruecke', 'Bruecke'),
         ('tunnel', 'Tunnel')]
EBENE_TAGS = ('bridge', 'tunnel', 'layer', 'omsigen:ebene')


def ebene_von(tags):
    if tags.get('bridge') not in (None, 'no'):
        return 'bruecke'
    if tags.get('tunnel') not in (None, 'no'):
        return 'tunnel'
    if tags.get('omsigen:ebene') == 'boden':
        return 'boden'
    return 'auto'


class Eigenschaften(QWidget):
    """Formular fuer die gewaehlte Strasse bzw. fuer neue Strassen, dazu die Hoehe des gewaehlten Punkts"""

    def __init__(self, fenster):
        super().__init__()
        self.f = fenster
        V = QVBoxLayout(self)
        self.titel = QLabel('<b>Neue Strassen</b>')
        V.addWidget(self.titel)
        F = QFormLayout()
        V.addLayout(F)
        self.klasse = QComboBox()
        for k in KLASSEN:
            self.klasse.addItem(KLASSEN_TEXT[k], k)
        F.addRow('Klasse', self.klasse)
        self.name = QLineEdit(); F.addRow('Name', self.name)
        self.spuren = QSpinBox(); self.spuren.setRange(0, 6); self.spuren.setSpecialValueText('automatisch')
        F.addRow('Spuren', self.spuren)
        self.einbahn = QComboBox()
        for k, t in EINBAHN:
            self.einbahn.addItem(t, k)
        F.addRow('Einbahn', self.einbahn)
        self.tempo = QComboBox()
        for t in TEMPO:
            self.tempo.addItem(f'{t} km/h' if t else '(Standard)', t)
        F.addRow('Tempo', self.tempo)
        self.ebene = QComboBox()
        for k, t in EBENE:
            self.ebene.addItem(t, k)
        F.addRow('Ebene', self.ebene)
        self.vorfahrt = QCheckBox('Vorfahrtstrasse (Zeichen 306)'); F.addRow(self.vorfahrt)
        self.umkehren = QPushButton('Richtung umkehren'); self.umkehren.clicked.connect(self._umkehren)
        F.addRow(self.umkehren)
        # gewaehlter Punkt
        self.punkt_box = QGroupBox('Gewaehlter Punkt')
        P = QFormLayout(self.punkt_box)
        self.folgt = QCheckBox('folgt dem Gelaende')
        P.addRow(self.folgt)
        self.hoehe = QDoubleSpinBox(); self.hoehe.setRange(-60, 80); self.hoehe.setSingleStep(0.5)
        self.hoehe.setSuffix(' m'); self.hoehe.setDecimals(2)
        P.addRow('Hoehe ueber Gelaende', self.hoehe)
        P.addRow(QLabel('<small>In der 3D-Ansicht: Bild auf/ab, Pos1 = folgt Gelaende. Zwischen Punkten mit Hoehe '
                        'verlaeuft die Strasse gerade; hoch ueber dem Gelaende wird sie zur Bruecke, tief darunter '
                        'zum Tunnel.</small>'))
        V.addWidget(self.punkt_box)
        self.info = QLabel(); self.info.setWordWrap(True)
        V.addWidget(self.info)
        V.addStretch()
        for w in (self.klasse, self.einbahn, self.tempo, self.ebene):
            w.currentIndexChanged.connect(self._uebernehmen)
        self.spuren.valueChanged.connect(self._uebernehmen)
        self.vorfahrt.toggled.connect(self._uebernehmen)
        self.name.editingFinished.connect(self._uebernehmen)
        self.folgt.toggled.connect(self._punkt_uebernehmen)
        self.hoehe.editingFinished.connect(self._punkt_uebernehmen)
        self._laden = False
        self._punkt = None
        self.zeigen(None)
        self.punkt_zeigen(None)

    def _tags(self):
        t = standard_tags(self.klasse.currentData(), self.name.text().strip(), self.einbahn.currentData(),
                          self.spuren.value(), self.tempo.currentData())
        if self.vorfahrt.isChecked():
            t['priority_road'] = 'designated'
        e = self.ebene.currentData()
        if e == 'bruecke':
            t.update(bridge='yes', layer='1')
        elif e == 'tunnel':
            t.update(tunnel='yes', layer='-1')
        elif e == 'boden':
            t['omsigen:ebene'] = 'boden'
        return t

    def zeigen(self, sid):
        self._laden = True
        s = self.f.m.strasse(sid) if sid else None
        t = s['tags'] if s else self.f.karte.neue_tags
        self.titel.setText(f'<b>Strasse {sid}</b>' if s else '<b>Neue Strassen</b> (Vorgaben fuer das Zeichnen)')
        k = klasse_von(t)
        self.klasse.setCurrentIndex(max(0, KLASSEN.index(k)) if k in KLASSEN else 4)
        self.name.setText(t.get('name', ''))
        try:
            self.spuren.setValue(int(str(t.get('lanes', 0)).split(';')[0]))
        except ValueError:
            self.spuren.setValue(0)
        ow = {'yes': 'ja', '1': 'ja', 'true': 'ja', '-1': 'entgegen'}.get(t.get('oneway'), 'nein')
        self.einbahn.setCurrentIndex([k for k, _ in EINBAHN].index(ow))
        self.tempo.setCurrentIndex(TEMPO.index(str(t.get('maxspeed', ''))) if str(t.get('maxspeed', '')) in TEMPO else 0)
        self.ebene.setCurrentIndex([k for k, _ in EBENE].index(ebene_von(t)))
        self.vorfahrt.setChecked(t.get('priority_road') in ('designated', 'yes', 'yes_unposted'))
        self.umkehren.setEnabled(bool(s))
        andere = {k: v for k, v in t.items() if k not in ('highway', 'name', 'lanes', 'oneway', 'maxspeed',
                                                           'priority_road', 'psv', 'access') + EBENE_TAGS}
        self.info.setText(('<small>weitere OSM-Angaben: ' + ', '.join(f'{k}={v}' for k, v in andere.items()) +
                           '</small>') if andere else '')
        self._laden = False

    def punkt_zeigen(self, q):
        self._punkt = q
        self._laden = True
        self.punkt_box.setEnabled(q is not None)
        h = self.f.m.punkt_hoehe(q) if q is not None else None
        self.folgt.setChecked(h is None)
        self.hoehe.setValue(h or 0.0)
        self.hoehe.setEnabled(h is not None)
        self._laden = False

    def _punkt_uebernehmen(self, *_):
        if self._laden or self._punkt is None:
            return
        h = None if self.folgt.isChecked() else self.hoehe.value()
        self.hoehe.setEnabled(h is not None)
        if h != self.f.m.punkt_hoehe(self._punkt):
            self.f.m.punkt_hoehe_setzen(self._punkt, h)
            self.f.karte.neu_zeichnen()
            self.f.titel_setzen()

    def _uebernehmen(self, *_):
        if self._laden:
            return
        sid = self.f.karte.sel
        t = self._tags()
        if sid:
            alt = self.f.m.strasse(sid)['tags']
            for k in ('junction', 'zone:maxspeed', 'zone:traffic', 'covered'):
                if k in alt:
                    t[k] = alt[k]
            self.f.m.tags_setzen(sid, t)
            self.f.karte.neu_zeichnen()
            self.f.titel_setzen()
        else:
            self.f.karte.neue_tags = t

    def _umkehren(self):
        if self.f.karte.sel:
            self.f.m.umkehren(self.f.karte.sel)
            self.f.karte.neu_zeichnen()


class Fenster(QMainWindow):
    def __init__(self, modell):
        super().__init__()
        self.m = modell
        self.proj = Projection(*modell.p['ursprung'])
        self.resize(1500, 950)
        self.gel = Gelaende(self.proj, log=lambda *a: None)
        self.karte2d = Karte(modell, self.proj, self)
        self.ansicht3d = None
        if os.environ.get('OMSIEDITOR_OHNE_3D') != '1':
            from .ansicht3d import Ansicht3D
            self.ansicht3d = Ansicht3D(modell, self.proj, self, gel=self.gel)
        self.stapel = QStackedWidget()
        self.stapel.addWidget(self.karte2d)
        if self.ansicht3d:
            self.stapel.addWidget(self.ansicht3d)
        self.setCentralWidget(self.stapel)
        self.karte = self.ansicht3d or self.karte2d          # aktive Ansicht
        self.stapel.setCurrentWidget(self.karte)
        self.eig = Eigenschaften(self)
        d = QDockWidget('Eigenschaften', self); d.setWidget(self.eig); d.setMinimumWidth(320)
        self.addDockWidget(Qt.RightDockWidgetArea, d)
        for v in self._ansichten():
            v.auswahl.connect(self.eig.zeigen)
            v.geaendert.connect(self.titel_setzen)
            v.status.connect(lambda t: self.statusBar().showMessage(t))
        if self.ansicht3d:
            self.ansicht3d.punkt_gewaehlt.connect(self.eig.punkt_zeigen)
        self.quelle = QLabel(lb.QUELLE + (' | Strassen: (c) OpenStreetMap-Mitwirkende' if
                                          'OpenStreetMap' in modell.p.get('quelle', '') else '')
                             + ' | Gelaende: DGM1 Bayern (CC BY 4.0)')
        self.statusBar().addPermanentWidget(self.quelle)
        self._aktionen()
        self.titel_setzen()
        self.karte.werkzeug_setzen('auswahl')

    def _ansichten(self):
        return [v for v in (self.ansicht3d, self.karte2d) if v]

    def _aktion(self, text, kuerzel, fn, menue=None, tb=None, checkbar=False):
        a = QAction(text, self)
        if kuerzel:
            a.setShortcut(QKeySequence(kuerzel))
        a.setCheckable(checkbar)
        a.triggered.connect(fn)
        if menue:
            menue.addAction(a)
        if tb:
            tb.addAction(a)
        return a

    def _aktionen(self):
        mb = self.menuBar()
        tb = self.addToolBar('Werkzeuge')
        tb.setMovable(False)
        mdat, mbe, mans = mb.addMenu('&Datei'), mb.addMenu('&Bearbeiten'), mb.addMenu('&Ansicht')
        self._aktion('Speichern', 'Ctrl+S', self.speichern, mdat, tb)
        self._aktion('Speichern unter ...', 'Ctrl+Shift+S', lambda: self.speichern(neu=True), mdat)
        mdat.addSeparator()
        self._aktion('Karte erzeugen ...', 'Ctrl+E', self.exportieren, mdat, tb)
        mdat.addSeparator()
        self._aktion('Beenden', 'Ctrl+Q', self.close, mdat)
        tb.addSeparator()
        g = QActionGroup(self)
        self.a_wahl = self._aktion('Auswaehlen (V)', 'V', lambda: self.karte.werkzeug_setzen('auswahl'), mbe, tb, True)
        self.a_zeichnen = self._aktion('Strasse zeichnen (B)', 'B', lambda: self.karte.werkzeug_setzen('zeichnen'),
                                       mbe, tb, True)
        g.addAction(self.a_wahl); g.addAction(self.a_zeichnen); self.a_wahl.setChecked(True)
        for v in self._ansichten():
            v.werkzeug_geaendert.connect(lambda w: (self.a_zeichnen if w == 'zeichnen' else self.a_wahl).setChecked(True))
        mbe.addSeparator()
        self._aktion('Rueckgaengig', 'Ctrl+Z', self.rueckgaengig, mbe, tb)
        self._aktion('Wiederholen', 'Ctrl+Y', self.wiederholen, mbe)
        tb.addSeparator()
        if self.ansicht3d:
            self.a_3d = self._aktion('3D / 2D (F2)', 'F2', self.umschalten, mans, tb, True)
            self.a_3d.setChecked(True)
        a = self._aktion('Luftbild (L)', 'L', self.luftbild, mans, tb, True)
        a.setChecked(True)
        self._aktion('Alles zeigen (Pos1 im 2D / Ansicht)', 'Ctrl+0', lambda: self.karte.alles_zeigen(), mans, tb)

    def umschalten(self):
        """zwischen 3D und 2D wechseln; Auswahl, Werkzeug und Vorgaben gehen mit"""
        if not self.ansicht3d:
            return
        alt = self.karte
        neu = self.karte2d if alt is self.ansicht3d else self.ansicht3d
        neu.neue_tags = alt.neue_tags
        self.karte = neu
        self.stapel.setCurrentWidget(neu)
        neu.werkzeug_setzen(alt.werkzeug)
        neu.neu_zeichnen()
        neu.auswaehlen(alt.sel)
        if neu is self.karte2d and not self.karte2d._kacheln:
            self.karte2d.alles_zeigen()
        self.a_3d.setChecked(neu is self.ansicht3d)
        neu.setFocus()

    def luftbild(self, an):
        for v in self._ansichten():
            v.luftbild_schalten(an)

    def titel_setzen(self):
        n = self.m.p.get('name', 'Projekt')
        self.setWindowTitle(f'{"*" if self.m.geaendert else ""}{n} - OMSI-Karteneditor'
                            + (f' ({self.m.pfad})' if self.m.pfad else ''))

    def rueckgaengig(self):
        if self.m.rueckgaengig():
            self.karte.auswaehlen(None); self.karte.neu_zeichnen(); self.titel_setzen()

    def wiederholen(self):
        if self.m.wiederholen():
            self.karte.auswaehlen(None); self.karte.neu_zeichnen(); self.titel_setzen()

    def speichern(self, neu=False):
        pfad = self.m.pfad
        if neu or not pfad:
            pfad, _ = QFileDialog.getSaveFileName(self, 'Projekt speichern', (self.m.p.get('name', 'projekt')
                                                  .replace(' ', '_') + '.omsiprojekt'), 'OMSI-Projekt (*.omsiprojekt)')
            if not pfad:
                return False
        self.m.speichern(pfad)
        self.titel_setzen()
        self.statusBar().showMessage(f'Gespeichert: {pfad}', 5000)
        return True

    def exportieren(self):
        ExportDialog(self.m, self).exec()

    def closeEvent(self, ev):
        if self.m.geaendert:
            r = QMessageBox.question(self, 'Nicht gespeichert', 'Das Projekt hat ungespeicherte Aenderungen. Speichern?',
                                     QMessageBox.Save | QMessageBox.Discard | QMessageBox.Cancel)
            if r == QMessageBox.Cancel or (r == QMessageBox.Save and not self.speichern()):
                ev.ignore()
                return
        ev.accept()
