"""Startdialog (OSM-Import / leer / oeffnen) und Export-Dialog. Lange Arbeiten laufen in einem Hintergrund-Thread,
ihre Ausgaben erscheinen im Protokollfeld."""
import os, re, traceback
from PySide6.QtCore import Qt, QThread, Signal, QUrl
from PySide6.QtGui import QPixmap, QDesktopServices, QFont
from PySide6.QtWidgets import (QDialog, QVBoxLayout, QHBoxLayout, QFormLayout, QTabWidget, QWidget, QLineEdit,
                               QPushButton, QLabel, QPlainTextEdit, QDoubleSpinBox, QFileDialog, QCheckBox,
                               QMessageBox, QDialogButtonBox, QScrollArea)
from omsigen import osm, pipeline
from omsigen.config import DEFAULT_OMSI
from .modell import Modell


class Arbeit(QThread):
    """fuehrt fn(log=...) im Hintergrund aus; Ausgaben ueber zeile, Ergebnis ueber fertig(ergebnis, fehler)"""
    zeile = Signal(str)
    fertig = Signal(object, str)

    def __init__(self, fn, parent=None):
        super().__init__(parent)
        self.fn = fn

    def run(self):
        try:
            self.fertig.emit(self.fn(self.zeile.emit), '')
        except Exception as ex:                       # dem Nutzer zeigen statt abstuerzen
            self.zeile.emit(traceback.format_exc())
            self.fertig.emit(None, f'{type(ex).__name__}: {ex}')


def _protokoll():
    t = QPlainTextEdit()
    t.setReadOnly(True)
    t.setFont(QFont('Consolas', 9))
    t.setMinimumHeight(160)
    return t


class StartDialog(QDialog):
    """-> self.modell nach Annahme"""

    def __init__(self, parent=None):
        super().__init__(parent)
        self.setWindowTitle('OMSI-Karteneditor - neues Projekt')
        self.setMinimumWidth(560)
        self.modell = None
        L = QVBoxLayout(self)
        titel = QLabel('<h2>OMSI-Karteneditor</h2>Wie moechtest du anfangen?')
        L.addWidget(titel)
        tabs = QTabWidget()
        L.addWidget(tabs)

        # --- OSM-Import
        w = QWidget(); F = QFormLayout(w)
        self.stadt = QLineEdit('Aschaffenburg'); F.addRow('Stadt', self.stadt)
        self.von = QLineEdit('Hauptbahnhof'); F.addRow('Von', self.von)
        self.nach = QLineEdit('City Galerie'); F.addRow('Nach', self.nach)
        self.ueber = QLineEdit(); self.ueber.setPlaceholderText('optional, mehrere mit ; trennen')
        F.addRow('Ueber', self.ueber)
        self.breite = QDoubleSpinBox(); self.breite.setRange(30, 1000); self.breite.setValue(150); self.breite.setSuffix(' m')
        F.addRow('Korridor links/rechts', self.breite)
        F.addRow(QLabel('<small>Ortsnamen oder Koordinaten "49.98,9.14". Strassen, Haltestellen, Schilder und '
                        'Ampeln kommen aus OpenStreetMap und lassen sich danach bearbeiten.</small>'))
        b = QPushButton('Importieren'); b.clicked.connect(self._import); F.addRow(b)
        tabs.addTab(w, 'Aus OpenStreetMap importieren')

        # --- leer
        w = QWidget(); F = QFormLayout(w)
        self.ort = QLineEdit('Aschaffenburg Hauptbahnhof')
        self.ort.setPlaceholderText('Ortsname oder "49.98,9.14"')
        F.addRow('Ort (Mitte der Karte)', self.ort)
        self.name = QLineEdit('Neues Projekt'); F.addRow('Projektname', self.name)
        F.addRow(QLabel('<small>Leeres Projekt: Du zeichnest alle Strassen selbst auf dem Luftbild.</small>'))
        b = QPushButton('Leer anfangen'); b.clicked.connect(self._leer); F.addRow(b)
        tabs.addTab(w, 'Leer anfangen')

        # --- oeffnen
        w = QWidget(); V = QVBoxLayout(w)
        V.addWidget(QLabel('Ein gespeichertes Projekt (.omsiprojekt) oeffnen.'))
        b = QPushButton('Projekt oeffnen ...'); b.clicked.connect(self._oeffnen); V.addWidget(b); V.addStretch()
        tabs.addTab(w, 'Projekt oeffnen')

        self.log = _protokoll()
        L.addWidget(self.log)
        self._arbeit = None

    def _laeuft(self, an):
        for w in self.findChildren(QPushButton):
            w.setEnabled(not an)

    def _import(self):
        args = dict(von=self.von.text().strip(), nach=self.nach.text().strip(), stadt=self.stadt.text().strip() or None,
                    ueber=[u.strip() for u in self.ueber.text().split(';') if u.strip()], breite=self.breite.value())
        if not args['von'] or not args['nach']:
            QMessageBox.warning(self, 'Import', 'Bitte Start und Ziel angeben.')
            return
        self.log.clear()
        self._laeuft(True)
        self._arbeit = Arbeit(lambda log: pipeline.importiere(log=log, **args), self)
        self._arbeit.zeile.connect(self.log.appendPlainText)
        self._arbeit.fertig.connect(self._import_fertig)
        self._arbeit.start()

    def _import_fertig(self, projekt, fehler):
        self._laeuft(False)
        if fehler:
            QMessageBox.critical(self, 'Import fehlgeschlagen', fehler)
            return
        self.modell = Modell(projekt)
        self.accept()

    def _leer(self):
        text = self.ort.text().strip()
        try:
            p = osm.find_place(text, None, '.cache')
        except Exception as ex:
            QMessageBox.critical(self, 'Ort nicht gefunden', str(ex))
            return
        self.modell = Modell.neu(p['lat'], p['lon'], self.name.text().strip() or 'Neues Projekt')
        self.accept()

    def _oeffnen(self):
        f, _ = QFileDialog.getOpenFileName(self, 'Projekt oeffnen', '', 'OMSI-Projekt (*.omsiprojekt);;JSON (*.json)')
        if f:
            try:
                self.modell = Modell.oeffnen(f)
            except Exception as ex:
                QMessageBox.critical(self, 'Oeffnen fehlgeschlagen', str(ex))
                return
            self.accept()


class ExportDialog(QDialog):
    def __init__(self, modell, parent=None):
        super().__init__(parent)
        self.m = modell
        self.setWindowTitle('Karte erzeugen')
        self.setMinimumSize(720, 640)
        L = QVBoxLayout(self)
        F = QFormLayout()
        vorschlag = re.sub(r'[^A-Za-z0-9_]+', '_', modell.p.get('name', 'Karte')).strip('_') or 'Karte'
        self.name = QLineEdit(vorschlag); F.addRow('Kartenname (neuer Ordner)', self.name)
        self.omsi_an = QCheckBox('direkt in OMSI installieren'); self.omsi_an.setChecked(bool(DEFAULT_OMSI))
        F.addRow(self.omsi_an)
        self.omsi = QLineEdit(DEFAULT_OMSI or ''); F.addRow('OMSI-2-Ordner', self.omsi)
        self.kor = QLineEdit(); self.kor.setPlaceholderText('optional: korrekturen/....json'); F.addRow('Korrekturdatei', self.kor)
        self.ueber = QCheckBox('vorhandene Karte gleichen Namens ersetzen'); F.addRow(self.ueber)
        L.addLayout(F)
        H = QHBoxLayout()
        self.los = QPushButton('Karte erzeugen'); self.los.clicked.connect(self._los); H.addWidget(self.los)
        self.bild_knopf = QPushButton('Spielbild (openOMSI)'); self.bild_knopf.setEnabled(False)
        self.bild_knopf.clicked.connect(self._spielbild); H.addWidget(self.bild_knopf)
        self.html_knopf = QPushButton('Pfad-Ansicht oeffnen'); self.html_knopf.setEnabled(False)
        self.html_knopf.clicked.connect(lambda: QDesktopServices.openUrl(QUrl.fromLocalFile(os.path.abspath(self.html))))
        H.addWidget(self.html_knopf)
        L.addLayout(H)
        self.log = _protokoll(); L.addWidget(self.log, 1)
        self.bild = QLabel(); self.bild.setAlignment(Qt.AlignCenter)
        sc = QScrollArea(); sc.setWidget(self.bild); sc.setWidgetResizable(True); sc.setMinimumHeight(220)
        L.addWidget(sc, 2)
        self.ergebnis = None
        self.html = None

    def _los(self):
        name = self.name.text().strip()
        if not re.fullmatch(r'[A-Za-z0-9_\-]+', name):
            QMessageBox.warning(self, 'Kartenname', 'Bitte nur Buchstaben, Ziffern, _ und - verwenden.')
            return
        omsi = self.omsi.text().strip() if self.omsi_an.isChecked() else None
        os.makedirs('build', exist_ok=True)
        self.html = os.path.join('build', f'ansicht_{name}.html')
        kw = dict(name=name, omsi=omsi, ausgabe='build', korrekturen=self.kor.text().strip() or None,
                  ueberschreiben=self.ueber.isChecked(), ansicht_html=self.html)
        self.log.clear(); self.los.setEnabled(False)
        projekt = self.m.p
        self._arbeit = Arbeit(lambda log: pipeline.erzeuge(projekt, log=log, **kw), self)
        self._arbeit.zeile.connect(self.log.appendPlainText)
        self._arbeit.fertig.connect(self._fertig)
        self._arbeit.start()

    def _fertig(self, r, fehler):
        self.los.setEnabled(True)
        if fehler:
            QMessageBox.critical(self, 'Erzeugen fehlgeschlagen', fehler)
            return
        self.ergebnis = r
        self.html_knopf.setEnabled(True)
        self.bild_knopf.setEnabled(self.omsi_an.isChecked())
        msg = ('Karte fertig, alle Fahrspuren verbunden.' if r['rc'] == 0 else
               f'Karte fertig, aber {r["offen"]} Spurenden ohne Anschluss - siehe Protokoll.')
        self.log.appendPlainText('\n' + msg)

    def _spielbild(self):
        from omsigen import openomsi
        r = self.ergebnis
        ox, oz = r['offset']
        pts = [q for s in self.m.p['strassen'] for q in s['punkte']]
        cx = sum(q[0] for q in pts) / len(pts) - ox
        cz = sum(q[1] for q in pts) / len(pts) - oz
        os.makedirs('build', exist_ok=True)
        png = os.path.abspath(os.path.join('build', f'spielbild_{self.name.text().strip()}.png'))
        name, omsi = self.name.text().strip(), self.omsi.text().strip()
        self.bild_knopf.setEnabled(False)
        self.log.appendPlainText('openOMSI rendert ...')

        def job(log):
            res = openomsi.render(name, png, omsi=omsi, cam=f'{cx:.0f},{cz - 80:.0f},70,0,-40', verkehr=30, sekunden=30)
            for k, v in res['werte'].items():
                log(f'  {k}: {v}')
            return res
        self._arbeit2 = Arbeit(job, self)
        self._arbeit2.zeile.connect(self.log.appendPlainText)
        self._arbeit2.fertig.connect(lambda res, f: self._bild_da(png, res, f))
        self._arbeit2.start()

    def _bild_da(self, png, res, fehler):
        self.bild_knopf.setEnabled(True)
        if fehler or not res or not res['ok']:
            QMessageBox.warning(self, 'openOMSI', fehler or 'openOMSI konnte kein Bild erzeugen (ist es installiert?)')
            return
        self.bild.setPixmap(QPixmap(png).scaledToWidth(680, Qt.SmoothTransformation))
