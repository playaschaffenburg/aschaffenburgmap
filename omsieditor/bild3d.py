"""3D-Ansicht des Editors ohne Fenster als Bild (Pruefen, Tests):

  python -m omsieditor.bild3d projekt.omsiprojekt --png bild.png [--cam x,z,gier,neigung,abstand] [--ohne-luftbild]

Rechnet die Karte wie der Editor (pipeline.berechne), baut die Szene (geometrie3d) und rendert sie mit OpenGL in
einen Bildpuffer."""
import argparse, sys, os


def kontext():
    """GL-3.3-Kontext auf einer unsichtbaren Flaeche -> (app, ctx, surface) oder None"""
    from PySide6.QtGui import QGuiApplication, QOpenGLContext, QOffscreenSurface, QSurfaceFormat
    app = QGuiApplication.instance() or QGuiApplication(sys.argv[:1])
    fmt = QSurfaceFormat(); fmt.setVersion(3, 3); fmt.setProfile(QSurfaceFormat.CoreProfile); fmt.setDepthBufferSize(24)
    ctx = QOpenGLContext(); ctx.setFormat(fmt)
    if not ctx.create():
        return None
    s = QOffscreenSurface(); s.setFormat(ctx.format()); s.create()
    if not ctx.makeCurrent(s):
        return None
    return app, ctx, s


def rendern(szene, raster, kamera, png, breite=1600, hoehe=900, luftbilder=None, gras=None):
    """szene: geometrie3d.Szene (fertig()), raster: geometrie3d.Raster -> PNG"""
    from PySide6.QtOpenGL import QOpenGLFramebufferObject, QOpenGLFramebufferObjectFormat
    from .render3d import Renderer
    k = kontext()
    if not k:
        raise RuntimeError('kein OpenGL-3.3-Kontext')
    ff = QOpenGLFramebufferObjectFormat(); ff.setAttachment(QOpenGLFramebufferObject.Depth); ff.setSamples(4)
    fbo = QOpenGLFramebufferObject(breite, hoehe, ff)
    fbo.bind()
    r = Renderer(); r.init()
    r.bauten_setzen(szene)
    for t in raster.kacheln:
        r.gelaende_setzen(t, raster.dreiecke(t))
    for t, daten in (luftbilder or {}).items():
        r.luftbild_setzen(t, daten)
    r.zeichnen(kamera, breite, hoehe, gras=gras)
    img = fbo.toImage()
    fbo.release()
    img.save(png)
    return png


def main(argv=None):
    ap = argparse.ArgumentParser(prog='omsieditor.bild3d')
    ap.add_argument('projekt')
    ap.add_argument('--png', required=True)
    ap.add_argument('--cam', help='x,z,gier,neigung,abstand (Projektmeter/Grad); Standard: Mitte von oben schraeg')
    ap.add_argument('--omsi', default=None)
    ap.add_argument('--ohne-luftbild', action='store_true')
    a = ap.parse_args(argv)
    from omsigen import pipeline
    from omsigen.config import DEFAULT_OMSI
    from omsigen.route import Projection
    from . import geometrie3d as g3
    from .render3d import Kamera
    from . import luftbild as lb
    omsi = a.omsi or DEFAULT_OMSI
    P = pipeline.laden(a.projekt)
    B = pipeline.berechne(P, omsi=omsi, log=lambda *x: None)
    szene, raster, basis = g3.szene_aus_berechnung(B, omsi)
    k = Kamera()
    pts = [q for s in P['strassen'] for q in s['punkte']]
    if a.cam:
        x, z, k.gier, k.neigung, k.abstand = (float(v) for v in a.cam.split(','))
    else:
        x, z = sum(q[0] for q in pts) / len(pts), sum(q[1] for q in pts) / len(pts)
    k.ziel = [x, raster.hoehe(x, z) or 0.0, z]
    bilder = {}
    if not a.ohne_luftbild:
        proj = Projection(*P['ursprung'])
        for t in raster.kacheln:
            x0, z0 = raster.kachel_ecke(t)
            d = lb.rechteck_laden(proj, x0, z0, x0 + g3.TILE, z0 + g3.TILE, pixel=1024)
            if d:
                bilder[t] = d
    gras = g3.textur_finden('gras.bmp', g3.textur_ordner(omsi))
    print(rendern(szene.fertig(), raster, k, a.png, luftbilder=bilder, gras=gras))


if __name__ == '__main__':
    sys.exit(main())
