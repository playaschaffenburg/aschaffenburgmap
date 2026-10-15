"""Masse und Eigenschaften installierter Objekte (.sco mit .x/.o3d-Modellen) fuer die Strassenausstattung (G3/G4).

Liest ein Objekt so weit, wie die Platzierung es braucht: Umriss (Bounding Box in Objektkoordinaten: x rechts, y oben,
z vorwaerts - wie Direct3D, siehe docs/omsi-format.md), Dreiecke, Texturen, ob das Objekt eine Haltestelle ist
(`[busstop]` - solche Objekte duerfen nicht als Moebel gesetzt werden, sonst entstehen zusaetzliche Haltestellen),
Beschriftungen (`[texttexture]`) und Anhaengepunkte (`[new_attachment]`).

.x: Textformat mit Frames und FrameTransformMatrix (Zeilenvektoren, Verschiebung in der letzten Zeile).
.o3d: Binaerformat wie openOMSI `omsi-o3d` (Kennung 0x84 0x19, Abschnitte 0x17 Punkte, 0x49 Dreiecke, 0x26 Materialien,
0x79 Matrix; ab Version 4 verwuerfelte Punkte).

  python -m omsigen.ausstattung_modelle <rel.sco> [...] [--omsi <OMSI>]
"""
import argparse
import json
import math
import os
import re
import struct

BS = chr(92)


# ---------------------------------------------------------------- .sco

def sco_lesen(pfad):
    """.sco -> dict(meshes=[Datei], busstop, texttexture (Zahl), anhaenge (Zahl), matl=[Textur], licht, gruppen)"""
    z = [l.rstrip('\r\n') for l in open(pfad, encoding='cp1252', errors='replace')]
    z = [l.lstrip('﻿\xef\xbb\xbf') for l in z]
    out = dict(meshes=[], busstop=False, texttexture=0, anhaenge=0, anhaenge_hoehen=[], matl=[], licht=False, model=None)
    for i, l in enumerate(z):
        k = l.strip().lower()
        nxt = next((w.strip() for w in z[i + 1:] if w.strip()), '')
        if k == '[new_attachment]':
            werte = [w.strip() for w in z[i + 1:] if w.strip()][:4]       # attach_trans, x, y (vor), z (oben)
            try:
                out['anhaenge_hoehen'].append(float(werte[3]))
            except (IndexError, ValueError):
                pass
        if k == '[mesh]':
            out['meshes'].append(nxt)
        elif k == '[model]':
            out['model'] = nxt
        elif k == '[busstop]':
            out['busstop'] = True
        elif k == '[texttexture]' or k == '[texttexture_enh]':
            out['texttexture'] += 1
        elif k == '[new_attachment]':
            out['anhaenge'] += 1
        elif k in ('[matl]', '[matl_change]'):
            out['matl'].append(nxt)
        elif k in ('[spotlight]', '[lamp]', '[light_enh]', '[light_enh_2]', '[interiorlight]'):
            out['licht'] = True
    return out


# ---------------------------------------------------------------- .x (Text)

_TOK = re.compile(r'"[^"]*"|\{|\}|[A-Za-z_][\w\-\.]*|-?\d+\.?\d*(?:[eE][-+]?\d+)?|-?\.\d+(?:[eE][-+]?\d+)?')


def _mal(a, b):
    """4x4 (Liste 16, Zeilen) a * b"""
    return [sum(a[r * 4 + k] * b[k * 4 + c] for k in range(4)) for r in range(4) for c in range(4)]


EINHEIT = [1.0, 0, 0, 0, 0, 1.0, 0, 0, 0, 0, 1.0, 0, 0, 0, 0, 1.0]


def x_lesen(text):
    """.x-Text -> (Punkte [(x, y, z)] in Modellkoordinaten nach allen Frames, Dreiecke (Zahl), Texturen)"""
    text = text[text.find('txt') + 3:] if text.startswith('xof') else text
    text = re.sub(r'(//|#)[^\n]*', ' ', text)                    # Kommentare
    t = _TOK.findall(text)
    punkte, tex = [], []
    n_dr = 0
    i = 0

    def ueberspringen(i):
        """i zeigt auf '{' -> Index nach der passenden '}'"""
        tiefe = 0
        while i < len(t):
            if t[i] == '{':
                tiefe += 1
            elif t[i] == '}':
                tiefe -= 1
                if tiefe == 0:
                    return i + 1
            i += 1
        return i

    def block(i, m):
        """Inhalt eines Blocks ab i bis zur schliessenden '}' (oder Dateiende) -> Index danach"""
        nonlocal n_dr
        lokal = m
        while i < len(t):
            w = t[i]
            if w == '}':
                return i + 1
            if w == 'template':
                j = i
                while j < len(t) and t[j] != '{':
                    j += 1
                i = ueberspringen(j)
                continue
            if w == 'FrameTransformMatrix':
                j = t.index('{', i)
                zahlen = [float(v) for v in t[j + 1:j + 17]]
                lokal = _mal(zahlen, m)
                i = ueberspringen(j)
                continue
            if w == 'Frame':
                j = t.index('{', i)
                i = block(j + 1, lokal)
                continue
            if w == 'Mesh':
                j = t.index('{', i)
                nv = int(float(t[j + 1]))
                v = [float(a) for a in t[j + 2:j + 2 + 3 * nv]]
                k = j + 2 + 3 * nv
                nf = int(float(t[k]))
                k += 1
                for _ in range(nf):
                    c = int(float(t[k]))
                    n_dr += max(0, c - 2)
                    k += 1 + c
                for a in range(nv):
                    x, y, z = v[3 * a:3 * a + 3]
                    punkte.append((x * lokal[0] + y * lokal[4] + z * lokal[8] + lokal[12],
                                   x * lokal[1] + y * lokal[5] + z * lokal[9] + lokal[13],
                                   x * lokal[2] + y * lokal[6] + z * lokal[10] + lokal[14]))
                ende = ueberspringen(j)
                for a in range(k, ende):
                    if t[a] == 'TextureFilename' and a + 2 < ende:
                        tex.append(t[a + 2].strip('"'))
                i = ende
                continue
            if w == '{':
                i = ueberspringen(i)
                continue
            i += 1
        return i

    block(0, EINHEIT)
    return punkte, n_dr, tex


# ---------------------------------------------------------------- .o3d (binaer)

def _frac_byte(p):
    f = lambda x: x - math.trunc(x)
    return int(math.trunc(abs(f(p[0]) * f(p[1]) * f(p[2])) * 600.0)) % 256


def o3d_lesen(daten):
    """.o3d-Bytes -> (Punkte [(x, y, z)], Dreiecke, Texturen); Matrix 0x79 wird angewandt"""
    if len(daten) < 3 or daten[0] != 0x84 or daten[1] != 0x19:
        raise ValueError('keine o3d-Datei')
    p = 2
    version = daten[p]
    p += 1
    lang = alt = False
    if version >= 3:
        fl = daten[p]
        p += 1
        lang, alt = bool(fl & 1), bool(fl & 2)
    key = 0
    if version >= 4:
        key = struct.unpack_from('<I', daten, p)[0]
        p += 4
    verw = version >= 4 and key != 0xFFFFFFFF
    zustand = ((key + (version - 4) + (0x17D if alt else 0)) % 0xFDE8) if verw else 0
    breit = version >= 3
    punkte, tex, n_dr = [], [], 0
    matrix = None
    while p < len(daten):
        tag = daten[p]
        p += 1
        try:
            if tag == 0x17:
                n = struct.unpack_from('<I' if breit else '<H', daten, p)[0]
                p += 4 if breit else 2
                n16 = n % 0xFDE8
                fb = 0
                for _ in range(n):
                    x, y, z = struct.unpack_from('<3f', daten, p)
                    p += 32
                    if verw:
                        if key == 0:
                            zustand = 0x130 if alt else 0
                        zustand = (zustand * n16 + n16 * fb) % 8000
                        fb = _frac_byte((x, y, z))
                        s = zustand
                        if s < 1000:
                            x, y = y, x
                        elif s < 3000:
                            x, z = z, x
                        elif s > 7000:
                            y, z = z, y
                    punkte.append((x, y, z))
            elif tag == 0x49:
                n = struct.unpack_from('<I' if breit else '<H', daten, p)[0]
                p += (4 if breit else 2) + n * (3 * (4 if lang else 2) + 2)
                n_dr += n
            elif tag == 0x26:
                n = struct.unpack_from('<H', daten, p)[0]
                p += 2
                for _ in range(n):
                    p += 4 * 11
                    ln = daten[p]
                    p += 1
                    tex.append(daten[p:p + ln].decode('cp1252', 'replace'))
                    p += ln
            elif tag == 0x79:
                matrix = list(struct.unpack_from('<16f', daten, p))
                p += 64
            elif tag == 0x54:
                n = struct.unpack_from('<H', daten, p)[0]
                p += 2
                for _ in range(n):
                    ln = daten[p]
                    p += 1 + ln
                    w = struct.unpack_from('<H', daten, p)[0]
                    p += 2 + w * ((4 if lang else 2) + 4)
        except struct.error:
            break
    if matrix and matrix != EINHEIT:
        m = matrix
        punkte = [(x * m[0] + y * m[4] + z * m[8] + m[12], x * m[1] + y * m[5] + z * m[9] + m[13],
                   x * m[2] + y * m[6] + z * m[10] + m[14]) for x, y, z in punkte]
    return punkte, n_dr, tex


# ---------------------------------------------------------------- Masse eines Objekts

def mesh_pfad(sco_pfad, datei):
    ordner = os.path.dirname(sco_pfad)
    for kand in (os.path.join(ordner, 'model', *datei.split(BS)), os.path.join(ordner, *datei.split(BS))):
        if os.path.isfile(kand):
            return kand
    return None


def masse(omsi, rel):
    """rel (.sco ab OMSI-Ordner) -> dict(rel, gefunden, busstop, texttexture, anhaenge, licht, dreiecke, texturen,
    box=[xmin, ymin, zmin, xmax, ymax, zmax] (x rechts, y oben, z vorwaerts), breite, tiefe, hoehe, fehlt=[Meshes])"""
    sco = os.path.join(omsi, *rel.split(BS))
    if not os.path.isfile(sco):
        return dict(rel=rel, gefunden=False)
    s = sco_lesen(sco)
    pk, dr, tex, fehlt = [], 0, [], []
    for m in s['meshes']:
        pfad = mesh_pfad(sco, m)
        if not pfad:
            fehlt.append(m)
            continue
        try:
            if pfad.lower().endswith('.x'):
                p, d, t = x_lesen(open(pfad, encoding='cp1252', errors='replace').read())
            else:
                p, d, t = o3d_lesen(open(pfad, 'rb').read())
        except (ValueError, IndexError):
            fehlt.append(m)
            continue
        pk += p
        dr += d
        tex += t
    out = dict(rel=rel, gefunden=True, busstop=s['busstop'], texttexture=s['texttexture'], anhaenge=s['anhaenge'],
               anhaenge_hoehen=s['anhaenge_hoehen'],
               licht=s['licht'], dreiecke=dr, texturen=sorted(set(tex) | set(s['matl'])), fehlt=fehlt)
    if pk:
        lo = [min(p[i] for p in pk) for i in range(3)]
        hi = [max(p[i] for p in pk) for i in range(3)]
        out.update(box=[round(v, 3) for v in lo + hi], breite=round(hi[0] - lo[0], 3), hoehe=round(hi[1] - lo[1], 3),
                   tiefe=round(hi[2] - lo[2], 3))
    return out


def main():
    from . import schichten
    ap = argparse.ArgumentParser(description='Masse installierter Objekte (.sco)')
    ap.add_argument('rel', nargs='+')
    ap.add_argument('--omsi', default=schichten.STANDARD_OMSI)
    a = ap.parse_args()
    for r in a.rel:
        print(json.dumps(masse(a.omsi, r), ensure_ascii=False))


if __name__ == '__main__':
    main()
