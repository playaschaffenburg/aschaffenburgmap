"""Hauptfenster: Werkzeugleiste, Kartenansicht, Eigenschaften der gewaehlten Strasse, Statuszeile."""
from PySide6.QtCore import Qt
from PySide6.QtGui import QAction, QKeySequence, QActionGroup
from PySide6.QtWidgets import (QMainWindow, QDockWidget, QWidget, QFormLayout, QLineEdit, QComboBox, QSpinBox,
                               QCheckBox, QLabel, QPushButton, QFileDialog, QMessageBox, QVBoxLayout)
from omsigen.route import Projection
from . import luftbild as lb
from .karte import Karte
from .modell import KLASSEN, KLASSEN_TEXT, standard_tags, klasse_von
from .dialoge import ExportDialog

EINBAHN = [('nein', 'nein'), ('ja', 'ja (in Zeichenrichtung)'), ('entgegen', 'entgegen der Zeichenrichtung')]
TEMPO = ['', '30', '50', '70']


class Eigenschaften(QWidget):
    """Formular fuer die gewaehlte Strasse bzw. fuer neue Strassen"""

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
        self.vorfahrt = QCheckBox('Vorfahrtstrasse (Zeichen 306)'); F.addRow(self.vorfahrt)
        self.umkehren = QPushButton('Richtung umkehren'); self.umkehren.clicked.connect(self._umkehren)
        F.addRow(self.umkehren)
        self.info = QLabel(); self.info.setWordWrap(True)
        V.addWidget(self.info)
        V.addStretch()
        for w in (self.klasse, self.einbahn, self.tempo):
            w.currentIndexChanged.connect(self._uebernehmen)
        self.spuren.valueChanged.connect(self._uebernehmen)
        self.vorfahrt.toggled.connect(self._uebernehmen)
        self.name.editingFinished.connect(self._uebernehmen)
        self._laden = False
        self.zeigen(None)

    def _tags(self):
        t = standard_tags(self.klasse.currentData(), self.name.text().strip(), self.einbahn.currentData(),
                          self.spuren.value(), self.tempo.currentData())
        if self.vorfahrt.isChecked():
            t['priority_road'] = 'designated'
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
        self.vorfahrt.setChecked(t.get('priority_road') in ('designated', 'yes', 'yes_unposted'))
        self.umkehren.setEnabled(bool(s))
        andere = {k: v for k, v in t.items() if k not in ('highway', 'name', 'lanes', 'oneway', 'maxspeed',
                                                           'priority_road', 'psv', 'access')}
        self.info.setText(('<small>weitere OSM-Angaben: ' + ', '.join(f'{k}={v}' for k, v in andere.items()) +
                           '</small>') if andere else '')
        self._laden = False

    def _uebernehmen(self, *_):
        if self._laden:
            return
        sid = self.f.karte.sel
        t = self._tags()
        if sid:
            alt = self.f.m.strasse(sid)['tags']
            for k in ('junction', 'layer', 'bridge', 'tunnel', 'zone:maxspeed', 'zone:traffic', 'covered'):
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
        self.resize(1400, 900)
        self.karte = Karte(modell, self.proj, self)
        self.setCentralWidget(self.karte)
        self.eig = Eigenschaften(self)
        d = QDockWidget('Eigenschaften', self); d.setWidget(self.eig); d.setMinimumWidth(300)
        self.addDockWidget(Qt.RightDockWidgetArea, d)
        self.karte.auswahl.connect(self.eig.zeigen)
        self.karte.geaendert.connect(self.titel_setzen)
        self.karte.status.connect(lambda t: self.statusBar().showMessage(t))
        self.quelle = QLabel(lb.QUELLE + (' | Strassen: (c) OpenStreetMap-Mitwirkende' if
                                          'OpenStreetMap' in modell.p.get('quelle', '') else ''))
        self.statusBar().addPermanentWidget(self.quelle)
        self._aktionen()
        self.titel_setzen()
        self.karte.werkzeug_setzen('auswahl')

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
        self.a_zeichnen = self._aktion('Strasse zeichnen (S)', 'S', lambda: self.karte.werkzeug_setzen('zeichnen'),
                                       mbe, tb, True)
        g.addAction(self.a_wahl); g.addAction(self.a_zeichnen); self.a_wahl.setChecked(True)
        self.karte.werkzeug_geaendert.connect(lambda w: (self.a_zeichnen if w == 'zeichnen' else self.a_wahl).setChecked(True))
        mbe.addSeparator()
        self._aktion('Rueckgaengig', 'Ctrl+Z', self.rueckgaengig, mbe, tb)
        self._aktion('Wiederholen', 'Ctrl+Y', self.wiederholen, mbe)
        tb.addSeparator()
        a = self._aktion('Luftbild (L)', 'L', lambda an: self.karte.luftbild_schalten(an), mans, tb, True)
        a.setChecked(True)
        self._aktion('Alles zeigen (A)', 'A', self.karte.alles_zeigen, mans, tb)

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
