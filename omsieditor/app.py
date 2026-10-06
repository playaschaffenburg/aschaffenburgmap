"""Programmstart: python -m omsieditor [projekt.omsiprojekt]"""
import os, sys
from PySide6.QtWidgets import QApplication, QDialog
from .dialoge import StartDialog
from .fenster import Fenster
from .modell import Modell


def main(argv=None):
    argv = sys.argv if argv is None else argv
    from PySide6.QtGui import QSurfaceFormat
    fmt = QSurfaceFormat()                       # 3D-Ansicht: OpenGL 3.3 Core, Tiefenpuffer, Kantenglaettung
    fmt.setVersion(3, 3); fmt.setProfile(QSurfaceFormat.CoreProfile); fmt.setDepthBufferSize(24); fmt.setSamples(4)
    QSurfaceFormat.setDefaultFormat(fmt)
    app = QApplication.instance() or QApplication(argv)
    app.setApplicationName('OMSI-Karteneditor')
    app.setStyle('Fusion')
    if len(argv) > 1 and os.path.exists(argv[1]):
        modell = Modell.oeffnen(argv[1])
    else:
        d = StartDialog()
        if d.exec() != QDialog.Accepted or not d.modell:
            return 0
        modell = d.modell
    w = Fenster(modell)
    w.show()
    if modell.p['strassen']:
        w.karte.alles_zeigen()
    else:
        w.karte.zentrieren(0, 0, 0.5)
    return app.exec()
