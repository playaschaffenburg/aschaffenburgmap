"""Eigene Objekte der Ausstattung (G4), von omsigen erzeugt: Haltestellenmast mit Zeichen 224 und Namensschild, erhoehter
Bord an Haltestellen. Schrift exakt aus der Schriftart (Pillow, Bahnschrift = DIN 1451 bzw. Arial), nie aus Bildern.

Ein Objekt = .sco + model/<name>.x + texture/<name>.dds im Kachelordner der Schicht objekte
(Sceneryobjects/Aschaffenburg/<Karte>/Objekte/<kx>_<ky>/). Modellkoordinaten wie Direct3D: x rechts, y oben, z vorwaerts;
Dreiecke im Uhrzeigersinn von der Sichtseite (docs/omsi-format.md).

Die Texturwerkstatt kann diese Objekte je Typ ersetzen (ausstattung_katalog.json: eigenes .sco vor "eigen" eintragen).
"""
import io
import json
import math
import os
import re
import unicodedata

SCHRIFTEN = [r'C:\Windows\Fonts\bahnschrift.ttf', r'C:\Windows\Fonts\arialbd.ttf', r'C:\Windows\Fonts\arial.ttf',
             '/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf']
GRUEN = (0, 112, 60)
GELB = (255, 210, 0)
WEISS = (250, 250, 250)
GRAU = (150, 152, 155)
ATLAS = (1024, 256)                 # Zeichen 224 links (256 x 256), Namensschild oben rechts (768 x 128), Mast unten
SCHILD_H = 0.20                     # m Hoehe des Namensschilds
PX_JE_M = 128 / SCHILD_H


def kurzname(name):
    """Dateiname aus dem Haltestellennamen (ASCII, Umlaute ausgeschrieben)"""
    t = name.replace('ä', 'ae').replace('ö', 'oe').replace('ü', 'ue').replace('Ä', 'Ae').replace('Ö', 'Oe')
    t = t.replace('Ü', 'Ue').replace('ß', 'ss')
    t = unicodedata.normalize('NFKD', t).encode('ascii', 'ignore').decode()
    return re.sub(r'[^A-Za-z0-9]+', '_', t).strip('_')[:40] or 'Haltestelle'


def schrift(groesse):
    from PIL import ImageFont
    for s in SCHRIFTEN:
        if os.path.isfile(s):
            f = ImageFont.truetype(s, groesse)
            if s.endswith('bahnschrift.ttf'):
                try:
                    f.set_variation_by_name('Bold')
                except Exception:
                    pass
            return f
    return ImageFont.load_default()


# ---------------------------------------------------------------- Texturen

def namensschild_breite(name):
    """(Schriftgroesse px, Textbreite px, Schildbreite m) - Schrift so gross wie moeglich, hoechstens 728 px breit"""
    from PIL import Image, ImageDraw
    d = ImageDraw.Draw(Image.new('RGB', (8, 8)))
    for g in range(84, 31, -2):
        b = d.textbbox((0, 0), name, font=schrift(g))
        if b[2] - b[0] <= 728:
            break
    w = b[2] - b[0]
    return g, w, round((w + 40) / PX_JE_M, 3)


def atlas(name):
    """Textur des Haltestellenmasts (PIL-Bild 1024 x 256): Zeichen 224, Namensschild mit `name` (genau dieser Text),
    Mastfarbe. -> (Bild, Schildbreite m, Breite des Schilds im Atlas px)"""
    from PIL import Image, ImageDraw
    img = Image.new('RGB', ATLAS, GRAU)
    d = ImageDraw.Draw(img)
    # Zeichen 224: gelbe Scheibe, gruener Rand, gruenes H (Rechtecke - exakt symmetrisch)
    d.ellipse((4, 4, 252, 252), fill=GRUEN)
    d.ellipse((22, 22, 234, 234), fill=GELB)
    d.rectangle((78, 62, 104, 194), fill=GRUEN)
    d.rectangle((152, 62, 178, 194), fill=GRUEN)
    d.rectangle((104, 116, 152, 140), fill=GRUEN)
    # Namensschild: weiss mit gruenem Rand, gruene Schrift, mittig
    g, w, breite = namensschild_breite(name)
    px = w + 40
    x0 = 256
    d.rectangle((x0, 0, x0 + px - 1, 127), fill=WEISS)
    d.rectangle((x0, 0, x0 + px - 1, 127), outline=GRUEN, width=5)
    f = schrift(g)
    b = d.textbbox((0, 0), name, font=f)
    d.text((x0 + 20 - b[0], 64 - (b[1] + b[3]) / 2), name, font=f, fill=GRUEN)
    # Rueckseiten und Kanten: Weiss
    d.rectangle((512, 128, 1023, 255), fill=WEISS)
    return img, breite, px


def bord_textur(taktil):
    """erhoehter Bord: Beton hell, bei Leitstreifen (tactile_paving=yes) Rillen rechts (256 x 64)"""
    from PIL import Image, ImageDraw
    img = Image.new('RGB', (256, 64), (178, 176, 170))
    d = ImageDraw.Draw(img)
    for x in range(0, 128, 16):                     # Fugen
        d.line((x, 0, x, 63), fill=(150, 148, 142))
    if taktil:
        d.rectangle((128, 0, 255, 63), fill=(222, 222, 216))
        for y in range(4, 64, 10):
            d.rectangle((128, y, 255, y + 4), fill=(196, 196, 190))
    return img


def dds_schreiben(img, ziel):
    from .lod2 import dds_bc1
    b = io.BytesIO()
    img.save(b, format='PNG')
    b.seek(0)
    os.makedirs(os.path.dirname(ziel), exist_ok=True)
    dds_bc1(b, ziel)


# ---------------------------------------------------------------- Netz (Dreiecke)

class Modell:
    """Punkte (x, y, z, nx, ny, nz, u, v_bild) und Dreiecke (a, b, c, Material)"""

    def __init__(self):
        self.V, self.F = [], []

    def dreieck(self, a, b, c, n, mat=0):
        """a, b, c: ((x, y, z), (u, v)); n = Sichtseite. Reihenfolge so, dass die Flaeche von n aus im Uhrzeigersinn
        erscheint (Direct3D: Sichtseite = Richtung von (b - a) x (c - a))"""
        pa, pb, pc = a[0], b[0], c[0]
        u = [pb[i] - pa[i] for i in range(3)]
        v = [pc[i] - pa[i] for i in range(3)]
        kr = (u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0])
        if kr[0] * n[0] + kr[1] * n[1] + kr[2] * n[2] < 0:
            b, c = c, b
        i = len(self.V)
        for p, uv in (a, b, c):
            self.V.append((p[0], p[1], p[2], n[0], n[1], n[2], uv[0], uv[1]))
        self.F.append((i, i + 1, i + 2, mat))

    def viereck(self, p, uv, n, mat=0):
        """p, uv: je 4 Ecken im Umlauf"""
        self.dreieck((p[0], uv[0]), (p[1], uv[1]), (p[2], uv[2]), n, mat)
        self.dreieck((p[0], uv[0]), (p[2], uv[2]), (p[3], uv[3]), n, mat)

    def x_text(self, textur):
        from .kreuzung import x_file
        # x_file schreibt v negiert (Bodentexturen); Bildkoordinate v (0 = oben) -> -v uebergeben
        V = [(x, y, z, nx, ny, nz, u, -v) for x, y, z, nx, ny, nz, u, v in self.V]
        return x_file(V, self.F, [('m0', textur)])


def mast_modell(schild_breite, atlas_px):
    """Haltestellenmast 3,0 m mit Zeichen 224 (Durchmesser 0,42 m, Mitte 2,74 m) und Namensschild darunter, beide
    beidseitig (Rueckseite gespiegelt, Schrift lesbar) in der Ebene x-y, Vorderseite +z"""
    m = Modell()
    W, H = ATLAS
    uv = lambda px, py: (px / W, py / H)
    r, h0 = 0.035, 2.95
    mast_u = (300, 140, 500, 250)
    for i in range(8):                               # achteckiger Mast
        a0, a1 = 2 * math.pi * i / 8, 2 * math.pi * (i + 1) / 8
        p0 = (r * math.sin(a0), 0.0, r * math.cos(a0))
        p1 = (r * math.sin(a1), 0.0, r * math.cos(a1))
        am = (a0 + a1) / 2
        n = (math.sin(am), 0.0, math.cos(am))
        m.viereck([p0, p1, (p1[0], h0, p1[2]), (p0[0], h0, p0[2])],
                  [uv(mast_u[0], mast_u[3]), uv(mast_u[2], mast_u[3]), uv(mast_u[2], mast_u[1]), uv(mast_u[0], mast_u[1])], n)
    m.viereck([(-r, h0, -r), (r, h0, -r), (r, h0, r), (-r, h0, r)], [uv(400, 200)] * 4, (0, 1, 0))
    dz = 0.045                                       # Schildflaechen vor und hinter dem Mast
    # Zeichen 224
    cy, rs, n = 2.74, 0.21, 24
    for vorne in (True, False):
        z = dz if vorne else -dz
        for i in range(n):
            a0, a1 = 2 * math.pi * i / n, 2 * math.pi * (i + 1) / n
            pts = [(0.0, cy, z), (rs * math.cos(a0), cy + rs * math.sin(a0), z), (rs * math.cos(a1), cy + rs * math.sin(a1), z)]
            tex = []
            for (x, y, _) in pts:
                ux = (x / rs) * (1 if vorne else -1)          # Rueckseite von hinten gesehen gespiegelt
                tex.append(uv(128 - ux * 124, 128 - (y - cy) / rs * 124))
            m.dreieck((pts[0], tex[0]), (pts[1], tex[1]), (pts[2], tex[2]), (0, 0, 1 if vorne else -1))
    # Namensschild
    b2, y0, y1 = schild_breite / 2, 2.27, 2.27 + SCHILD_H
    u0, u1 = 256, 256 + atlas_px
    for vorne in (True, False):
        z = dz if vorne else -dz
        # von vorn (+z) gesehen liegt +x links (Direct3D, Blick in -z): linke Kante x = +b2
        xl, xr = (b2, -b2) if vorne else (-b2, b2)
        m.viereck([(xl, y1, z), (xr, y1, z), (xr, y0, z), (xl, y0, z)],
                  [uv(u0, 0), uv(u1, 0), uv(u1, 127), uv(u0, 127)], (0, 0, 1 if vorne else -1))
    # Kanten des Schilds (weiss)
    w_u = [uv(700, 200)] * 4
    for sx in (-1, 1):
        x = sx * b2
        m.viereck([(x, y0, -dz), (x, y0, dz), (x, y1, dz), (x, y1, -dz)], w_u, (sx, 0, 0))
    for sy, y in ((1, y1), (-1, y0)):
        m.viereck([(-b2, y, -dz), (b2, y, -dz), (b2, y, dz), (-b2, y, dz)], w_u, (0, sy, 0))
    return m


def mast_vorschau(name):
    """Bild des Schilds, wie es im Spiel von vorn erscheint (fuer Pruefbilder und Tests)"""
    img, breite, px = atlas(name)
    return img.crop((256, 0, 256 + px, 128))


def sco_text(titel, modell_datei, quelle, absolut=False):
    """absolut: Objekthoehe absolut wie die Splines ([absheight]) statt ueber dem Gelaende"""
    L = [f'Erzeugt mit OMSIgen (omsigen.ausstattung_eigen). {quelle}', '', '[friendlyname]', titel, '',
         '[groups]', '1', 'Aschaffenburg', '', '[fixed]', ''] + (['[absheight]', ''] if absolut else []) +         ['[mesh]', modell_datei, '']
    return '\r\n'.join(L) + '\r\n'


def mast_schreiben(ordner, name):
    """Haltestellenmast fuer `name` in `ordner` (Kachelordner) -> Dateiname der .sco (ohne Ordner) und Schildbreite"""
    kurz = 'H_' + kurzname(name)
    img, breite, px = atlas(name)
    dds_schreiben(img, os.path.join(ordner, 'texture', kurz + '.dds'))
    os.makedirs(os.path.join(ordner, 'model'), exist_ok=True)
    with open(os.path.join(ordner, 'model', kurz + '.x'), 'w', encoding='cp1252', newline='') as f:
        f.write(mast_modell(breite, px).x_text(kurz + '.dds'))
    with open(os.path.join(ordner, kurz + '.sco'), 'w', encoding='cp1252', errors='replace', newline='') as f:
        f.write(sco_text(f'Haltestelle {name}', kurz + '.x', 'Name: OpenStreetMap-Mitwirkende, ODbL.'))
    with open(os.path.join(ordner, kurz + '.json'), 'w', encoding='utf-8') as f:
        json.dump(dict(art='haltestellenmast', name=name, schild_breite_m=breite), f, ensure_ascii=False)
    return kurz + '.sco', breite


def bord_modell(punkte, bord, breite, hoehe, taktil=False):
    """erhoehter Bord entlang einer Kante: punkte = [(x, y, z) relativ zum Objekt, Richtung rechts (dx, dz)] je Station
    (y = Gehwegoberkante relativ), Streifen von `bord` (Querlage der Bordkante) `breite` m nach aussen, `hoehe` m ueber dem
    Gehweg; vorn die Bordflaeche bis zur Fahrbahn (0,15 m tiefer), an den Enden Rampen"""
    m = Modell()
    u0 = 0.5 if taktil else 0.0                      # Textur: links Beton, rechts Rillenplatten
    n = len(punkte)
    for i in range(n - 1):
        (xa, ya, za, ra), (xb, yb, zb, rb) = punkte[i], punkte[i + 1]
        ha = 0.0 if i == 0 else hoehe                # Rampen an beiden Enden
        hb = hoehe if i + 1 < n - 1 else 0.0
        innen_a = (xa + ra[0] * bord, za + ra[1] * bord)
        innen_b = (xb + rb[0] * bord, zb + rb[1] * bord)
        aussen_a = (xa + ra[0] * (bord + breite), za + ra[1] * (bord + breite))
        aussen_b = (xb + rb[0] * (bord + breite), zb + rb[1] * (bord + breite))
        m.viereck([(innen_a[0], ya + ha, innen_a[1]), (aussen_a[0], ya + ha, aussen_a[1]),
                   (aussen_b[0], yb + hb, aussen_b[1]), (innen_b[0], yb + hb, innen_b[1])],
                  [(u0, 0.0), (u0 + 0.5, 0.0), (u0 + 0.5, 1.0), (u0, 1.0)], (0, 1, 0))
        m.viereck([(innen_a[0], ya - 0.15, innen_a[1]), (innen_b[0], yb - 0.15, innen_b[1]),
                   (innen_b[0], yb + hb, innen_b[1]), (innen_a[0], ya + ha, innen_a[1])],
                  [(0.0, 0.0), (0.25, 0.0), (0.25, 0.3), (0.0, 0.3)], (-ra[0], 0, -ra[1]))
    return m


# ---------------------------------------------------------------- Verkehrszeichen (eigene, exakt aus Vektoren)

ROT, BLAU, SCHWARZ, VZ_GELB = (200, 16, 30), (0, 82, 160), (20, 20, 20), (255, 200, 0)
VZ_ORDNER = ('Sceneryobjects', 'Aschaffenburg', 'Ausstattung', 'VZ')
# Code -> (Form, Breite m, Hoehe m): Form kreis | dreieck_unten | dreieck | achteck | raute | rechteck
VZ_FORM = {'205': ('dreieck_unten', 0.90, 0.78), '206': ('achteck', 0.60, 0.60), '301': ('dreieck', 0.90, 0.78),
           '306': ('raute', 0.60, 0.60), '307': ('raute', 0.60, 0.60), '274': ('kreis', 0.60, 0.60),
           '278': ('kreis', 0.60, 0.60), '274.1': ('rechteck', 0.60, 0.84), '274.2': ('rechteck', 0.60, 0.84),
           '215': ('kreis', 0.60, 0.60), '267': ('kreis', 0.60, 0.60), '250': ('kreis', 0.60, 0.60),
           '283': ('kreis', 0.60, 0.60), '286': ('kreis', 0.60, 0.60), '350': ('rechteck', 0.60, 0.60),
           '357': ('rechteck', 0.60, 0.60)}


def _umriss(form, w, h, n=32):
    """Umriss des Schilds in Metern (Mitte 0, y nach oben)"""
    if form == 'kreis':
        return [(w / 2 * math.cos(2 * math.pi * i / n), h / 2 * math.sin(2 * math.pi * i / n)) for i in range(n)]
    if form == 'achteck':
        r = w / 2 / math.cos(math.pi / 8)
        return [(r * math.cos(math.pi / 8 + math.pi * i / 4), r * math.sin(math.pi / 8 + math.pi * i / 4)) for i in range(8)]
    if form == 'raute':
        return [(w / 2, 0), (0, h / 2), (-w / 2, 0), (0, -h / 2)]
    if form == 'dreieck_unten':
        return [(0, -h / 2), (w / 2, h / 2), (-w / 2, h / 2)]
    if form == 'dreieck':
        return [(-w / 2, -h / 2), (w / 2, -h / 2), (0, h / 2)]
    return [(-w / 2, -h / 2), (w / 2, -h / 2), (w / 2, h / 2), (-w / 2, h / 2)]


def vz_bild(code, wert='', groesse=256):
    """Verkehrszeichen als Bild (PIL, quadratisch; das Quadrat umfasst max(Breite, Hoehe) des Schilds um seine Mitte):
    Formen, Farben und Schrift (Bahnschrift) gezeichnet, 4-fach ueberabgetastet"""
    from PIL import Image, ImageDraw
    form, w, h = VZ_FORM[code]
    S = groesse * 4
    img = Image.new('RGB', (S, S), WEISS)
    d = ImageDraw.Draw(img)
    m = max(w, h)
    px = lambda x, y: (S / 2 + x / m * S, S / 2 - y / m * S)         # Meter -> Pixel (y nach unten)
    pts = [px(x, y) for x, y in _umriss(form, w, h, 96)]
    innen = lambda f: [px(x * f, y * f) for x, y in _umriss(form, w, h, 96)]

    def text(t, groesse_px, farbe, mitte=(S / 2, S / 2)):
        f = schrift(groesse_px)
        b = d.textbbox((0, 0), t, font=f)
        d.text((mitte[0] - (b[0] + b[2]) / 2, mitte[1] - (b[1] + b[3]) / 2), t, font=f, fill=farbe)

    def streifen(x0, x1, farbe, breite):
        for i in range(-2, 3):
            o = i * S * 0.06
            d.line((x0 + o, S * 0.8 + o, x1 + o, S * 0.2 + o), fill=farbe, width=breite)

    if code in ('205', '301'):
        d.polygon(pts, fill=ROT)
        d.polygon(innen(0.72), fill=WEISS)
        if code == '301':                                   # schwarzes Kreuz (Vorfahrt an der naechsten Kreuzung)
            cx, cy = S / 2, S * 0.58
            d.rectangle((cx - S * 0.03, cy - S * 0.16, cx + S * 0.03, cy + S * 0.14), fill=SCHWARZ)
            d.rectangle((cx - S * 0.11, cy - S * 0.05, cx + S * 0.11, cy - S * 0.02), fill=SCHWARZ)
    elif code == '206':
        d.polygon(pts, fill=WEISS)
        d.polygon(innen(0.92), fill=ROT)
        text('STOP', int(S * 0.27), WEISS)
    elif code in ('306', '307'):
        d.polygon(pts, fill=WEISS)
        d.polygon(innen(0.94), fill=SCHWARZ)
        d.polygon(innen(0.88), fill=WEISS)
        d.polygon(innen(0.62), fill=VZ_GELB)
        if code == '307':
            streifen(S * 0.25, S * 0.75, SCHWARZ, int(S * 0.02))
    elif code in ('283', '286'):
        d.ellipse((0, 0, S - 1, S - 1), fill=ROT)
        d.ellipse((S * 0.1, S * 0.1, S * 0.9, S * 0.9), fill=BLAU)
        d.line((S * 0.22, S * 0.22, S * 0.78, S * 0.78), fill=ROT, width=int(S * 0.1))
        if code == '283':
            d.line((S * 0.78, S * 0.22, S * 0.22, S * 0.78), fill=ROT, width=int(S * 0.1))
    elif code == '267':
        d.ellipse((0, 0, S - 1, S - 1), fill=ROT)
        d.rectangle((S * 0.18, S * 0.42, S * 0.82, S * 0.58), fill=WEISS)
    elif code == '278':
        d.ellipse((0, 0, S - 1, S - 1), fill=SCHWARZ)
        d.ellipse((S * 0.02, S * 0.02, S * 0.98, S * 0.98), fill=WEISS)
        text(str(wert), int(S * 0.42), (120, 120, 120))
        streifen(S * 0.2, S * 0.8, SCHWARZ, int(S * 0.018))
    elif code in ('274', '250'):
        d.ellipse((0, 0, S - 1, S - 1), fill=ROT)
        d.ellipse((S * 0.13, S * 0.13, S * 0.87, S * 0.87), fill=WEISS)
        if code == '274':
            text(str(wert), int(S * (0.46 if len(str(wert)) < 3 else 0.36)), SCHWARZ)
    elif code == '215':
        d.ellipse((0, 0, S - 1, S - 1), fill=BLAU)
        for i in range(3):                                 # drei Pfeile im Kreis (gegen den Uhrzeigersinn)
            a0 = 90 + i * 120
            d.arc((S * 0.22, S * 0.22, S * 0.78, S * 0.78), -a0 - 70, -a0 + 10, fill=WEISS, width=int(S * 0.07))
            a = math.radians(a0 + 15)
            cx, cy = S / 2 + S * 0.28 * math.cos(a), S / 2 - S * 0.28 * math.sin(a)
            t = math.radians(a0 + 15 + 90)
            spitze = (cx + S * 0.1 * math.cos(t), cy - S * 0.1 * math.sin(t))
            n_ = (math.cos(a), -math.sin(a))
            d.polygon([spitze, (cx + n_[0] * S * 0.08, cy + n_[1] * S * 0.08),
                       (cx - n_[0] * S * 0.08, cy - n_[1] * S * 0.08)], fill=WEISS)
    elif code in ('274.1', '274.2'):
        bx0, bx1 = S / 2 - S * 0.6 / 0.84 / 2, S / 2 + S * 0.6 / 0.84 / 2     # Hochformat 0,60 x 0,84
        r = S * 0.03
        d.rectangle((bx0 + r, r, bx1 - r, S - r), outline=SCHWARZ, width=int(S * 0.015))
        cr = S * 0.2
        cx, cy = S / 2, S * 0.36
        d.ellipse((cx - cr, cy - cr, cx + cr, cy + cr), fill=ROT)
        d.ellipse((cx - cr * 0.78, cy - cr * 0.78, cx + cr * 0.78, cy + cr * 0.78), fill=WEISS)
        text('30', int(S * 0.2), SCHWARZ, (cx, cy))
        text('ZONE', int(S * 0.15), SCHWARZ, (cx, S * 0.72))
        if code == '274.2':
            streifen(bx0 + S * 0.05, bx1 - S * 0.05, (90, 90, 90), int(S * 0.012))
    elif code == '350':
        d.rectangle((0, 0, S - 1, S - 1), fill=BLAU)
        d.polygon([(S * 0.5, S * 0.12), (S * 0.88, S * 0.82), (S * 0.12, S * 0.82)], fill=WEISS)
        for i in range(4):
            x0 = S * (0.3 + i * 0.11)
            d.rectangle((x0, S * 0.7, x0 + S * 0.06, S * 0.78), fill=SCHWARZ)
        d.ellipse((S * 0.46, S * 0.3, S * 0.54, S * 0.38), fill=SCHWARZ)
        d.line((S * 0.5, S * 0.38, S * 0.5, S * 0.56), fill=SCHWARZ, width=int(S * 0.04))
        d.line((S * 0.5, S * 0.56, S * 0.43, S * 0.67), fill=SCHWARZ, width=int(S * 0.035))
        d.line((S * 0.5, S * 0.56, S * 0.58, S * 0.67), fill=SCHWARZ, width=int(S * 0.035))
    elif code == '357':
        d.rectangle((0, 0, S - 1, S - 1), fill=BLAU)
        d.rectangle((S * 0.42, S * 0.3, S * 0.58, S * 0.88), fill=WEISS)
        d.rectangle((S * 0.22, S * 0.14, S * 0.78, S * 0.3), fill=ROT)
    img = img.resize((groesse, groesse), Image.LANCZOS)
    ImageDraw.Draw(img).rectangle((groesse - 4, 0, groesse - 1, 3), fill=(150, 150, 150))   # Rueckseite (Ecke)
    return img


def vz_modell(code):
    """Schild in der Ebene x-y (Mitte 0), Vorderseite +z bei z = 0,045 vor dem Mast, Rueckseite grau (z = 0,04)"""
    form, w, h = VZ_FORM[code]
    m = Modell()
    um = _umriss(form, w, h)
    mm = max(w, h)
    for vorne in (True, False):
        z = 0.045 if vorne else 0.04
        for i in range(len(um)):
            a, b = um[i], um[(i + 1) % len(um)]
            pts = [(0.0, 0.0, z), (a[0], a[1], z), (b[0], b[1], z)]
            if vorne:                                    # von vorn (Blick -z) liegt +x links: u = 0,5 - x/mm
                tex = [(0.5 - p[0] / mm, 0.5 - p[1] / mm) for p in pts]
            else:
                tex = [(1.0 - 2.0 / 256, 2.0 / 256)] * 3
            m.dreieck((pts[0], tex[0]), (pts[1], tex[1]), (pts[2], tex[2]), (0, 0, 1 if vorne else -1))
    return m


def vz_name(code, wert=''):
    return f'VZ_{code.replace(".", "_")}' + (f'_{kurzname(str(wert))}' if wert else '')


def vz_schreiben(omsi, code, wert=''):
    """eigenes Verkehrszeichen im gemeinsamen Ordner Sceneryobjects/Aschaffenburg/Ausstattung/VZ (fuer alle Karten)
    -> rel (.sco ab OMSI-Ordner)"""
    ordner = os.path.join(omsi, *VZ_ORDNER)
    name = vz_name(code, wert)
    dds_schreiben(vz_bild(code, wert), os.path.join(ordner, 'texture', name + '.dds'))
    os.makedirs(os.path.join(ordner, 'model'), exist_ok=True)
    with open(os.path.join(ordner, 'model', name + '.x'), 'w', encoding='cp1252', newline='') as f:
        f.write(vz_modell(code).x_text(name + '.dds'))
    with open(os.path.join(ordner, name + '.sco'), 'w', encoding='cp1252', newline='') as f:
        f.write(sco_text(f'Zeichen {code} {wert}'.strip(), name + '.x', 'Verkehrszeichen nach StVO, aus Vektoren gezeichnet.'))
    return chr(92).join(VZ_ORDNER + (name + '.sco',))


def schildmast_schreiben(omsi):
    """eigener Mast fuer Verkehrszeichen (2,9 m, grau) im gemeinsamen Ordner -> rel"""
    from PIL import Image
    ordner = os.path.join(omsi, *VZ_ORDNER)
    name = 'VZ_Mast'
    m = Modell()
    r, h0 = 0.03, 2.9
    for i in range(8):
        a0, a1 = 2 * math.pi * i / 8, 2 * math.pi * (i + 1) / 8
        p0 = (r * math.sin(a0), 0.0, r * math.cos(a0))
        p1 = (r * math.sin(a1), 0.0, r * math.cos(a1))
        am = (a0 + a1) / 2
        m.viereck([p0, p1, (p1[0], h0, p1[2]), (p0[0], h0, p0[2])], [(0.5, 0.5)] * 4, (math.sin(am), 0.0, math.cos(am)))
    m.viereck([(-r, h0, -r), (r, h0, -r), (r, h0, r), (-r, h0, r)], [(0.5, 0.5)] * 4, (0, 1, 0))
    dds_schreiben(Image.new('RGB', (8, 8), (140, 142, 145)), os.path.join(ordner, 'texture', name + '.dds'))
    os.makedirs(os.path.join(ordner, 'model'), exist_ok=True)
    with open(os.path.join(ordner, 'model', name + '.x'), 'w', encoding='cp1252', newline='') as f:
        f.write(m.x_text(name + '.dds'))
    with open(os.path.join(ordner, name + '.sco'), 'w', encoding='cp1252', newline='') as f:
        f.write(sco_text('Mast fuer Verkehrszeichen', name + '.x', ''))
    return chr(92).join(VZ_ORDNER + (name + '.sco',))
