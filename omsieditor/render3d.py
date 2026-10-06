"""OpenGL-Teil der 3D-Ansicht (OpenGL 3.3 Core ueber PyOpenGL): Texturen, Gelaendekacheln mit Luftbild, Bauten aus
geometrie3d, Hilfslinien (Strassenachsen, Auswahl, Zeichnen). Braucht einen aktuellen GL-Kontext (QOpenGLWidget
oder QOffscreenSurface fuer Bilder ohne Fenster).

Kamera wie in Transport Fever: Blickpunkt am Boden, Drehung (gier, im Uhrzeigersinn ab Nord), Neigung, Abstand.
GL-Koordinaten = (x, y, -z) der Welt."""
import ctypes, math
import numpy as np
from OpenGL import GL

VS = """#version 330 core
layout(location=0) in vec3 p; layout(location=1) in vec3 n; layout(location=2) in vec2 uv;
uniform mat4 vp; uniform vec3 auge;
out vec3 vn; out vec2 vuv; out float dist;
void main(){ vec3 g = vec3(p.x, p.y, -p.z); gl_Position = vp * vec4(g, 1.0);
  vn = vec3(n.x, n.y, -n.z); vuv = uv; dist = length(g - auge); }"""
FS = """#version 330 core
in vec3 vn; in vec2 vuv; in float dist; uniform sampler2D tex; uniform vec3 licht; uniform vec3 nebel;
uniform float hell; out vec4 farbe;
void main(){ vec4 c = texture(tex, vuv); if (c.a < 0.4) discard;
  float l = 0.5 + 0.55 * abs(dot(normalize(vn), licht));
  float f = clamp((dist - 1200.0) / 4000.0, 0.0, 0.85);
  farbe = vec4(mix(c.rgb * l * hell, nebel, f), 1.0); }"""
VS2 = """#version 330 core
layout(location=0) in vec3 p; layout(location=1) in vec4 c; uniform mat4 vp; out vec4 vc;
void main(){ gl_Position = vp * vec4(p.x, p.y, -p.z, 1.0); vc = c; }"""
FS2 = """#version 330 core
in vec4 vc; out vec4 farbe; void main(){ farbe = vc; }"""
NEBEL = (0.74, 0.80, 0.88)


# ------------------------------------------------------------------ Matrizen
def perspektive(fov, aspekt, nah, fern):
    f = 1 / math.tan(math.radians(fov) / 2)
    M = np.zeros((4, 4), dtype=np.float64)
    M[0, 0], M[1, 1] = f / aspekt, f
    M[2, 2], M[2, 3] = (fern + nah) / (nah - fern), 2 * fern * nah / (nah - fern)
    M[3, 2] = -1
    return M


def blick(auge, ziel, oben=(0, 1, 0)):
    auge, ziel, oben = (np.asarray(v, dtype=np.float64) for v in (auge, ziel, oben))
    f = ziel - auge; f /= np.linalg.norm(f)
    s = np.cross(f, oben); s /= np.linalg.norm(s)
    u = np.cross(s, f)
    M = np.eye(4)
    M[0, :3], M[1, :3], M[2, :3] = s, u, -f
    M[:3, 3] = -M[:3, :3] @ auge
    return M


class Kamera:
    def __init__(self):
        self.ziel = [0.0, 0.0, 0.0]          # Welt x, y, z
        self.gier, self.neigung, self.abstand = 0.0, -35.0, 300.0
        self.fov = 50.0

    def richtung(self):
        g, n = math.radians(self.gier), math.radians(self.neigung)
        return (math.sin(g) * math.cos(n), math.sin(n), math.cos(g) * math.cos(n))     # Welt

    def auge(self):
        d = self.richtung()
        return tuple(self.ziel[i] - d[i] * self.abstand for i in range(3))

    def matrizen(self, breite, hoehe):
        e, t = self.auge(), self.ziel
        V = blick((e[0], e[1], -e[2]), (t[0], t[1], -t[2]))
        nah = max(0.3, self.abstand * 0.003)
        P = perspektive(self.fov, breite / max(hoehe, 1), nah, 9000.0)
        return P @ V, (e[0], e[1], -e[2])

    def strahl(self, mx, my, breite, hoehe):
        """Mausposition -> (Ursprung, Richtung) in Weltkoordinaten"""
        vp, _ = self.matrizen(breite, hoehe)
        inv = np.linalg.inv(vp)
        x, y = 2 * mx / breite - 1, 1 - 2 * my / hoehe
        a = inv @ np.array([x, y, -1, 1]); a = a[:3] / a[3]
        b = inv @ np.array([x, y, 1, 1]); b = b[:3] / b[3]
        d = b - a; d /= np.linalg.norm(d)
        return (a[0], a[1], -a[2]), (d[0], d[1], -d[2])

    def verschieben(self, rechts, vor):
        g = math.radians(self.gier)
        self.ziel[0] += math.cos(g) * rechts + math.sin(g) * vor
        self.ziel[2] += -math.sin(g) * rechts + math.cos(g) * vor


def treffer(ursprung, richtung, hoehe_fn, weit=6000.0):
    """Schnitt des Strahls mit dem Gelaende (hoehe_fn(x, z) -> y oder None) -> (x, y, z) oder None"""
    o, d = np.array(ursprung), np.array(richtung)
    t, schritt, vorher = 0.0, 1.0, None
    while t < weit:
        p = o + d * t
        h = hoehe_fn(p[0], p[2])
        if h is None:
            h = 0.0
        if p[1] <= h:
            if vorher is None:
                return (p[0], h, p[2])
            lo, hi = vorher, t                  # verfeinern
            for _ in range(20):
                m = (lo + hi) / 2
                q = o + d * m
                hq = hoehe_fn(q[0], q[2]) or 0.0
                lo, hi = (lo, m) if q[1] <= hq else (m, hi)
            q = o + d * hi
            return (q[0], hoehe_fn(q[0], q[2]) or 0.0, q[2])
        vorher = t
        schritt = max(0.5, t * 0.004)
        t += schritt
    return None


# ------------------------------------------------------------------ Renderer
def _programm(vs, fs):
    def sh(src, art):
        s = GL.glCreateShader(art)
        GL.glShaderSource(s, src)
        GL.glCompileShader(s)
        if not GL.glGetShaderiv(s, GL.GL_COMPILE_STATUS):
            raise RuntimeError(GL.glGetShaderInfoLog(s).decode())
        return s
    p = GL.glCreateProgram()
    for s in (sh(vs, GL.GL_VERTEX_SHADER), sh(fs, GL.GL_FRAGMENT_SHADER)):
        GL.glAttachShader(p, s)
    GL.glLinkProgram(p)
    if not GL.glGetProgramiv(p, GL.GL_LINK_STATUS):
        raise RuntimeError(GL.glGetProgramInfoLog(p).decode())
    return p


class _Netz:
    """VAO + VBO; Eckpunkte (n, k) float32 mit Attributgroessen groessen"""

    def __init__(self, arr, groessen=(3, 3, 2)):
        arr = np.ascontiguousarray(arr, dtype=np.float32)
        self.n = len(arr)
        self.vao = GL.glGenVertexArrays(1)
        self.vbo = GL.glGenBuffers(1)
        GL.glBindVertexArray(self.vao)
        GL.glBindBuffer(GL.GL_ARRAY_BUFFER, self.vbo)
        GL.glBufferData(GL.GL_ARRAY_BUFFER, arr.nbytes, arr, GL.GL_STATIC_DRAW)
        st, off = 4 * sum(groessen), 0
        for i, g in enumerate(groessen):
            GL.glEnableVertexAttribArray(i)
            GL.glVertexAttribPointer(i, g, GL.GL_FLOAT, GL.GL_FALSE, st, ctypes.c_void_p(off))
            off += 4 * g
        GL.glBindVertexArray(0)

    def zeichnen(self, art=GL.GL_TRIANGLES):
        if self.n:
            GL.glBindVertexArray(self.vao)
            GL.glDrawArrays(art, 0, self.n)

    def weg(self):
        GL.glDeleteBuffers(1, [self.vbo])
        GL.glDeleteVertexArrays(1, [self.vao])


class Renderer:
    def __init__(self):
        self.bereit = False
        self.texturen = {}                 # Schluessel -> GL-Textur
        self.bauten = {}                   # Texturschluessel -> _Netz
        self.gelaende = {}                 # Kachel -> (_Netz, Texturschluessel)
        self.luftbilder = {}               # Kachel -> GL-Textur
        self.hilfe = None                  # _Netz der Hilfslinien (pos + rgba)
        self.hilfe_oben = None             # immer sichtbar (Auswahl, Zeichnen)

    def init(self):
        self.prog = _programm(VS, FS)
        self.prog2 = _programm(VS2, FS2)
        GL.glEnable(GL.GL_DEPTH_TEST)
        GL.glDisable(GL.GL_CULL_FACE)
        self.bereit = True

    # ---------------------------------------------------------- Texturen
    def _textur_aus_rgba(self, w, h, daten, wiederholen=True):
        t = GL.glGenTextures(1)
        GL.glBindTexture(GL.GL_TEXTURE_2D, t)
        GL.glPixelStorei(GL.GL_UNPACK_ALIGNMENT, 1)
        GL.glTexImage2D(GL.GL_TEXTURE_2D, 0, GL.GL_RGBA8, w, h, 0, GL.GL_RGBA, GL.GL_UNSIGNED_BYTE, daten)
        GL.glGenerateMipmap(GL.GL_TEXTURE_2D)
        art = GL.GL_REPEAT if wiederholen else GL.GL_CLAMP_TO_EDGE
        GL.glTexParameteri(GL.GL_TEXTURE_2D, GL.GL_TEXTURE_WRAP_S, art)
        GL.glTexParameteri(GL.GL_TEXTURE_2D, GL.GL_TEXTURE_WRAP_T, art)
        GL.glTexParameteri(GL.GL_TEXTURE_2D, GL.GL_TEXTURE_MIN_FILTER, GL.GL_LINEAR_MIPMAP_LINEAR)
        GL.glTexParameteri(GL.GL_TEXTURE_2D, GL.GL_TEXTURE_MAG_FILTER, GL.GL_LINEAR)
        try:
            GL.glTexParameterf(GL.GL_TEXTURE_2D, 0x84FE, 8.0)       # anisotrop, falls vorhanden
        except Exception:
            pass
        return t

    def _bild_textur(self, img, wiederholen=True):
        from PySide6.QtGui import QImage
        img = img.convertToFormat(QImage.Format_RGBA8888)
        if img.width() > 2048 or img.height() > 2048:
            img = img.scaled(2048, 2048)
        return self._textur_aus_rgba(img.width(), img.height(), bytes(img.constBits())[:img.sizeInBytes()],
                                     wiederholen)

    def textur(self, key):
        if key in self.texturen:
            return self.texturen[key]
        t = None
        if key and not key.startswith('farbe:'):
            from PySide6.QtGui import QImage
            img = QImage(key)
            if not img.isNull():
                t = self._bild_textur(img)
        if t is None:
            c = key[6:] if key and key.startswith('farbe:') else '#8a8a8a'
            rgb = bytes(int(c[i:i + 2], 16) for i in (1, 3, 5)) + b'\xff'
            t = self._textur_aus_rgba(1, 1, rgb)
        self.texturen[key] = t
        return t

    # ---------------------------------------------------------- Inhalte
    def bauten_setzen(self, szene):
        for n in self.bauten.values():
            n.weg()
        self.bauten = {k: _Netz(a) for k, a in szene.items() if len(a)}

    def gelaende_setzen(self, kachel, arr):
        alt = self.gelaende.pop(kachel, None)
        if alt:
            alt.weg()
        self.gelaende[kachel] = _Netz(arr)

    def gelaende_leeren(self):
        for n in self.gelaende.values():
            n.weg()
        self.gelaende = {}

    def luftbilder_leeren(self):
        for t in self.luftbilder.values():
            GL.glDeleteTextures(1, [t])
        self.luftbilder = {}

    def luftbild_setzen(self, kachel, jpeg):
        from PySide6.QtGui import QImage
        img = QImage.fromData(jpeg)
        if img.isNull():
            return
        alt = self.luftbilder.pop(kachel, None)
        if alt:
            GL.glDeleteTextures(1, [alt])
        self.luftbilder[kachel] = self._bild_textur(img, wiederholen=False)

    def hilfe_setzen(self, arr, oben=None):
        for attr, a in (('hilfe', arr), ('hilfe_oben', oben)):
            alt = getattr(self, attr)
            if alt:
                alt.weg()
            setattr(self, attr, _Netz(a, (3, 4)) if a is not None and len(a) else None)

    # ---------------------------------------------------------- Zeichnen
    def zeichnen(self, kamera, breite, hoehe, gras=None, luftbild_an=True):
        GL.glViewport(0, 0, breite, hoehe)
        GL.glClearColor(*NEBEL, 1.0)
        GL.glClear(GL.GL_COLOR_BUFFER_BIT | GL.GL_DEPTH_BUFFER_BIT)
        GL.glEnable(GL.GL_DEPTH_TEST)
        vp, auge = kamera.matrizen(breite, hoehe)
        vpf = np.ascontiguousarray(vp.T, dtype=np.float32)       # GL erwartet spaltenweise
        GL.glUseProgram(self.prog)
        GL.glUniformMatrix4fv(GL.glGetUniformLocation(self.prog, 'vp'), 1, GL.GL_FALSE, vpf)
        GL.glUniform3f(GL.glGetUniformLocation(self.prog, 'auge'), *auge)
        l = np.array([0.35, 0.85, 0.4]); l /= np.linalg.norm(l)
        GL.glUniform3f(GL.glGetUniformLocation(self.prog, 'licht'), *l)
        GL.glUniform3f(GL.glGetUniformLocation(self.prog, 'nebel'), *NEBEL)
        hell = GL.glGetUniformLocation(self.prog, 'hell')
        GL.glActiveTexture(GL.GL_TEXTURE0)
        GL.glUniform1i(GL.glGetUniformLocation(self.prog, 'tex'), 0)
        # Gelaende etwas nach hinten schieben, damit Strassen darauf nicht flimmern
        GL.glEnable(GL.GL_POLYGON_OFFSET_FILL)
        GL.glPolygonOffset(1.0, 2.0)
        GL.glUniform1f(hell, 1.0)
        for k, n in self.gelaende.items():
            t = self.luftbilder.get(k) if luftbild_an else None
            GL.glBindTexture(GL.GL_TEXTURE_2D, t if t else self.textur(gras or 'farbe:#6f8a4e'))
            n.zeichnen()
        GL.glDisable(GL.GL_POLYGON_OFFSET_FILL)
        for k, n in self.bauten.items():
            GL.glBindTexture(GL.GL_TEXTURE_2D, self.textur(k))
            n.zeichnen()
        GL.glUseProgram(self.prog2)
        GL.glUniformMatrix4fv(GL.glGetUniformLocation(self.prog2, 'vp'), 1, GL.GL_FALSE, vpf)
        GL.glEnable(GL.GL_BLEND)
        GL.glBlendFunc(GL.GL_SRC_ALPHA, GL.GL_ONE_MINUS_SRC_ALPHA)
        if self.hilfe:
            self.hilfe.zeichnen()
        if self.hilfe_oben:
            GL.glDisable(GL.GL_DEPTH_TEST)
            self.hilfe_oben.zeichnen()
            GL.glEnable(GL.GL_DEPTH_TEST)
        GL.glDisable(GL.GL_BLEND)
        GL.glBindVertexArray(0)


# ------------------------------------------------------------------ Hilfslinien als Baender
def band(punkte, breite, farbe, hoch=0.4):
    """Polylinie [(x, y, z)] -> Dreiecke (n, 7) als waagerechtes Band (Breite in m)"""
    out = []
    for a, b in zip(punkte, punkte[1:]):
        dx, dz = b[0] - a[0], b[2] - a[2]
        L = math.hypot(dx, dz)
        if L < 1e-6:
            continue
        rx, rz = dz / L * breite / 2, -dx / L * breite / 2
        A1 = (a[0] - rx, a[1] + hoch, a[2] - rz); A2 = (a[0] + rx, a[1] + hoch, a[2] + rz)
        B1 = (b[0] - rx, b[1] + hoch, b[2] - rz); B2 = (b[0] + rx, b[1] + hoch, b[2] + rz)
        for p in (A1, A2, B2, A1, B2, B1):
            out.append(p + tuple(farbe))
    return out


def wuerfel(p, r, farbe):
    """kleiner Wuerfel als Griff an p = (x, y, z)"""
    x, y, z = p
    P = [(x + dx * r, y + dy * r, z + dz * r) for dx in (-1, 1) for dy in (-1, 1) for dz in (-1, 1)]
    seiten = [(0, 1, 3, 2), (4, 6, 7, 5), (0, 4, 5, 1), (2, 3, 7, 6), (0, 2, 6, 4), (1, 5, 7, 3)]
    out = []
    for a, b, c, d in seiten:
        for i in (a, b, c, a, c, d):
            out.append(P[i] + tuple(farbe))
    return out
