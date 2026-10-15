"""Belegungskarte einer Karte (Qualitaetsplan F4): je Kachel ein Raster (0,5 m), was wo liegt - Fahrbahn, Gehweg,
Kreuzungsplatte, Gebaeude, Gruen, Wasser, Bauwerk, frei. Spaetere Schichten platzieren nur auf erlaubten Klassen
(Baum nicht auf Fahrbahn, Laterne auf Gehweg), Pruefer melden Konflikte (Gebaeude ueber Fahrbahn).

Quellen: netz.json (F3: Fahrbahn zwischen den Fahrbahnkanten, Seitenraum bis zu den Aussenkanten als Gehweg,
Bauwerks-Splines), die Kreuzungsobjekte K_*.x (Material Asphalt = Kreuzung, Gehweg = Gehweg) und die Objekte der Schicht
gebaeude (Grundriss = Modell von oben). Gruen und Wasser kommen mit Phase 3 (OSM landuse, Laser).

Ablage: maps/<Karte>/belegung/tile_<kx>_<ky>.png (Palettenbild: Index = Klasse, Zeile 0 = Norden, 600 x 600) und
maps/<Karte>/belegung.json (Klassen, Farben, Fingerabdruecke von Netz und Gebaeuden je Kachel, Konflikte).

  python -m omsigen.belegung <Karte> [--omsi <OMSI-Ordner>]
"""
import argparse
import json
import math
import os
import re

import numpy as np

from . import netz_datei, schichten

FORMAT, VERSION = 'omsigen-belegung', 1
ZELLE = 0.5
KACHEL = 300.0
N = int(KACHEL / ZELLE)
ORDNER, DATEI = 'belegung', 'belegung.json'
FREI, FAHRBAHN, GEHWEG, KREUZUNG, GEBAEUDE, GRUEN, WASSER, BAUWERK = range(8)
KLASSEN = ['frei', 'fahrbahn', 'gehweg', 'kreuzung', 'gebaeude', 'gruen', 'wasser', 'bauwerk']
FARBEN = [(112, 112, 100), (45, 45, 70), (195, 195, 190), (110, 70, 160), (205, 60, 50), (60, 165, 60), (40, 110, 220),
          (230, 150, 40)]
# was gewinnt, wenn sich Flaechen ueberlagern (hinten = oben)
VORRANG = [FREI, WASSER, GRUEN, GEHWEG, FAHRBAHN, KREUZUNG, BAUWERK, GEBAEUDE]
KONFLIKT_M2 = 2.0        # Gebaeude ueber Fahrbahn/Kreuzung ab dieser Flaeche melden


# ---------------------------------------------------------------- .x-Modelle (wie omsigen sie schreibt)

def x_lesen(pfad):
    """.x-Datei -> (Punkte n x 3 in (x, hoch, vor), Dreiecke m x 3, Material je Dreieck, Texturnamen)"""
    text = open(pfad, encoding='cp1252', errors='replace').read()
    zahlen = lambda s: [float(v) for v in re.findall(r'-?\d+(?:\.\d+)?(?:[eE]-?\d+)?', s)]
    i = text.index('Mesh ')
    rumpf = text[text.index('{', i) + 1:]
    nv = int(re.match(r'\s*(\d+);', rumpf).group(1))
    rest = rumpf[re.match(r'\s*\d+;', rumpf).end():]
    ende = rest.index('MeshNormals') if 'MeshNormals' in rest else len(rest)
    w = zahlen(rest[:ende])
    P = np.array(w[:nv * 3], float).reshape(nv, 3)
    nf = int(w[nv * 3])
    F = np.array(w[nv * 3 + 1:nv * 3 + 1 + nf * 4], float).reshape(nf, 4)[:, 1:].astype(int)
    mat, tex = np.zeros(nf, int), []
    if 'MeshMaterialList' in rest:
        ml = rest[rest.index('MeshMaterialList'):]
        kopf = zahlen(ml[ml.index('{') + 1:ml.index('Material ') if 'Material ' in ml else len(ml)])
        if len(kopf) >= 2 + nf:
            mat = np.array(kopf[2:2 + nf], int)
        tex = re.findall(r'TextureFilename\s*\{\s*"([^"]+)"', ml)
    return P, F, mat, tex


def _drehen(x, z, rot):
    """Objektdrehung (Grad im Uhrzeigersinn ab Nord) auf Modellpunkte (x Ost, z Nord)"""
    r = math.radians(rot)
    c, s = math.cos(r), math.sin(r)
    return x * c + z * s, -x * s + z * c


# ---------------------------------------------------------------- Raster zeichnen

class _Leinwand:
    """je Kachel und Klasse eine Maske; Zeichnen in Kartenmetern"""

    def __init__(self, kacheln):
        from PIL import Image
        self.kacheln = set(kacheln)
        self.bilder = {}
        self._Image = Image

    def _bild(self, k, klasse):
        key = (k, klasse)
        if key not in self.bilder:
            self.bilder[key] = self._Image.new('L', (N, N), 0)
        return self.bilder[key]

    def polygon(self, punkte, klasse):
        from PIL import ImageDraw
        if len(punkte) < 3:
            return
        xs = [p[0] for p in punkte]
        zs = [p[1] for p in punkte]
        for kx in range(math.floor(min(xs) / KACHEL), math.floor(max(xs) / KACHEL) + 1):
            for ky in range(math.floor(min(zs) / KACHEL), math.floor(max(zs) / KACHEL) + 1):
                if (kx, ky) not in self.kacheln:
                    continue
                pix = [((x - kx * KACHEL) / ZELLE, (ky * KACHEL + KACHEL - z) / ZELLE) for x, z in punkte]
                ImageDraw.Draw(self._bild((kx, ky), klasse)).polygon(pix, fill=255)

    def maske(self, k, klasse):
        b = self.bilder.get((k, klasse))
        return np.zeros((N, N), bool) if b is None else np.asarray(b) > 0


def _objekte(kartenordner, pruefe):
    """[object]-Eintraege der Karte, deren Pfad `pruefe(rel)` erfuellt -> (rel, x, z, rot, kachel)"""
    out = []
    for k in schichten.kacheln(kartenordner):
        _, bl = schichten.bloecke(schichten.lesen(kartenordner, k))
        for b in bl:
            rel = schichten.objekt_pfad(b)
            if b[0].strip() != '[object]' or not rel or not pruefe(rel):
                continue
            try:
                out.append((rel, float(b[4]) + k[0] * KACHEL, float(b[5]) + k[1] * KACHEL, float(b[7]), k))
            except (ValueError, IndexError):
                pass
    return out


def _modellpfad(omsi, rel):
    """.sco -> Pfad seines [mesh] (das erste)"""
    sco = os.path.join(omsi, *rel.split(chr(92)))
    try:
        zeilen = [z.strip() for z in open(sco, encoding='cp1252', errors='replace')]
    except OSError:
        return None
    for i, z in enumerate(zeilen):
        if z == '[mesh]':
            datei = next((w for w in zeilen[i + 1:] if w), None)      # naechste nicht leere Zeile
            return os.path.join(os.path.dirname(sco), 'model', datei) if datei else None
    return None


# ---------------------------------------------------------------- berechnen

def strassenraum(kartenordner, omsi=None, kacheln=None):
    """Fahrbahn, Gehweg, Kreuzung, Bauwerk aller Kacheln (ohne Gebaeude) -> (_Leinwand, Zahl der Kreuzungsobjekte)"""
    omsi = omsi or schichten.STANDARD_OMSI
    kacheln = kacheln if kacheln is not None else schichten.kacheln(kartenordner)
    lw = _Leinwand(kacheln)
    netz = netz_datei.Netz.laden(kartenordner)
    # 1. Strassen aus netz.json
    for e in netz.kanten.values():
        q = e.get('querschnitt')
        if not q or e['art'] == 'unsichtbar':
            continue
        (cl, cr), (ol, orr) = q['fahrbahn'], q['aussen']
        if e['art'] == 'bauwerk':
            lw.polygon(netz.streifen(e['id'], ol, orr), BAUWERK)
            continue
        lw.polygon(netz.streifen(e['id'], cl, cr), FAHRBAHN)
        if ol < cl - 0.05:
            lw.polygon(netz.streifen(e['id'], ol, cl), GEHWEG)
        if orr > cr + 0.05:
            lw.polygon(netz.streifen(e['id'], cr, orr), GEHWEG)
    # 2. Kreuzungsplatten aus ihren Modellen
    n_kreuz = 0
    for rel, ox, oz, rot, _ in _objekte(kartenordner, lambda r: os.path.basename(r.replace(chr(92), '/')).upper().startswith('K_')):
        pfad = _modellpfad(omsi, rel)
        if not pfad or not os.path.isfile(pfad):
            continue
        P, F, mat, tex = x_lesen(pfad)
        x, z = _drehen(P[:, 0], P[:, 2], rot)
        x, z = x + ox, z + oz
        for f, m in zip(F, mat):
            t = tex[m].lower() if m < len(tex) else ''
            lw.polygon([(x[i], z[i]) for i in f], GEHWEG if 'side' in t else KREUZUNG)
        n_kreuz += 1
    # 2b. Uebergaenge zwischen Querschnitten (UE_*, S1): Hochbord (ab 0,17 m) = Gehweg, sonst Fahrbahn
    for rel, ox, oz, rot, _ in _objekte(kartenordner, lambda r: os.path.basename(r.replace(chr(92), '/')).upper().startswith('UE_')):
        pfad = _modellpfad(omsi, rel)
        if not pfad or not os.path.isfile(pfad):
            continue
        P, F, mat, tex = x_lesen(pfad)
        x, z = _drehen(P[:, 0], P[:, 2], rot)
        x, z = x + ox, z + oz
        for f in F:
            hoch = float(P[f, 1].mean()) > 0.17
            lw.polygon([(x[i], z[i]) for i in f], GEHWEG if hoch else FAHRBAHN)
            lw.polygon([(x[i], z[i]) for i in f], 'uebergang')      # nur fuer die Einordnung von Konflikten (G1)
    return lw, n_kreuz


def gebaeude_grundrisse(kartenordner, omsi=None):
    """Gebaeude der Schicht gebaeude: Modell von oben -> [(rel, [Dreieck [(x, z) * 3]])] in Kartenmetern"""
    omsi = omsi or schichten.STANDARD_OMSI
    gebaeude = _objekte(kartenordner, lambda r: schichten.schicht_von(r) is not None and schichten.schicht_von(r)[0] == 'gebaeude')
    grundrisse = []
    for rel, ox, oz, rot, k in gebaeude:
        pfad = _modellpfad(omsi, rel)
        if not pfad or not os.path.isfile(pfad):
            continue
        P, F, _, _ = x_lesen(pfad)
        x, z = _drehen(P[:, 0], P[:, 2], rot)
        x, z = x + ox, z + oz
        tris = []
        for f in F:
            tri = [(x[i], z[i]) for i in f]
            if abs((tri[1][0] - tri[0][0]) * (tri[2][1] - tri[0][1]) - (tri[2][0] - tri[0][0]) * (tri[1][1] - tri[0][1])) > 1e-4:
                tris.append(tri)
        grundrisse.append((rel, tris))
    return grundrisse


def berechnen(kartenordner, omsi=None):
    """Belegung aller Kacheln -> dict (belegung.json); schreibt die PNGs"""
    from PIL import Image
    omsi = omsi or schichten.STANDARD_OMSI
    kacheln = schichten.kacheln(kartenordner)
    lw, n_kreuz = strassenraum(kartenordner, omsi, kacheln)
    # 3. Gebaeude (Schicht gebaeude): Modell von oben = Grundriss
    grundrisse = gebaeude_grundrisse(kartenordner, omsi)
    # Konflikte vor dem Gebaeude-Zeichnen: Gebaeude gegen Fahrbahn/Kreuzung (mit Klasse und Hinweis, G1
    # platzierung.Einordnung), sonst gegen Gehweg
    from .platzierung import GEHWEG_M2, Einordnung
    konflikte, einordnung = [], None
    for i, (rel, tris) in enumerate(grundrisse):
        pruef = _Leinwand(kacheln)
        for tri in tris:
            pruef.polygon(tri, GEBAEUDE)
        flaeche, gehweg, zf, zk = 0.0, 0.0, [], []
        for (k, _), b in pruef.bilder.items():
            m = np.asarray(b) > 0
            fb, kr = m & lw.maske(k, FAHRBAHN), m & lw.maske(k, KREUZUNG) & ~lw.maske(k, FAHRBAHN)
            flaeche += float((fb | kr).sum()) * ZELLE * ZELLE
            gehweg += float((m & lw.maske(k, GEHWEG) & ~fb & ~kr).sum()) * ZELLE * ZELLE
            ue = lw.maske(k, 'uebergang')                     # Fahrbahn eines Uebergangsobjekts zaehlt wie Kreuzung
            for maske, liste in ((fb & ~ue, zf), (kr | (fb & ue), zk)):
                ze, sp = np.nonzero(maske)
                liste += list(zip(k[0] * KACHEL + (sp + 0.5) * ZELLE, k[1] * KACHEL + KACHEL - (ze + 0.5) * ZELLE))
        if flaeche >= KONFLIKT_M2:
            c = dict(art='gebaeude_auf_fahrbahn', objekt=rel, flaeche_m2=round(flaeche, 1))
            try:
                if einordnung is None:
                    einordnung = Einordnung(kartenordner, netz_datei.Netz.laden(kartenordner, pruefen=False),
                                            grundrisse, lw, KACHEL, ZELLE, omsi)
                c.update(einordnung.einordnen(i, zf, zk))
            except Exception as ex:               # die Belegung entsteht auch ohne Einordnung
                c.update(klasse='', hinweis=f'nicht eingeordnet ({type(ex).__name__}: {ex})')
            konflikte.append(c)
        elif gehweg >= GEHWEG_M2:
            konflikte.append(dict(art='gebaeude_auf_gehweg', objekt=rel, flaeche_m2=round(gehweg, 1)))
        for tri in tris:
            lw.polygon(tri, GEBAEUDE)
    # 4. zusammensetzen und schreiben
    ordner = os.path.join(kartenordner, ORDNER)
    os.makedirs(ordner, exist_ok=True)
    palette = [c for f in FARBEN for c in f] + [0] * (768 - 3 * len(FARBEN))
    anteile = np.zeros(len(KLASSEN))
    fp = {}
    for k in kacheln:
        r = np.zeros((N, N), np.uint8)
        for klasse in VORRANG[1:]:
            r[lw.maske(k, klasse)] = klasse
        anteile += np.bincount(r.ravel(), minlength=len(KLASSEN))
        bild = Image.fromarray(r, 'P')
        bild.putpalette(palette)
        bild.save(os.path.join(ordner, f'tile_{k[0]}_{k[1]}.png'), optimize=True)
        fp[f'{k[0]}_{k[1]}'] = {s: schichten.fingerabdruck(kartenordner, k, s) for s in ('netz', 'gebaeude')}
    d = dict(format=FORMAT, version=VERSION, zelle_m=ZELLE, kachel_m=KACHEL, klassen=KLASSEN,
             farben=[list(f) for f in FARBEN], fingerabdruck=fp, konflikte=sorted(konflikte, key=lambda c: -c['flaeche_m2']),
             anteile={KLASSEN[i]: round(float(a / max(1, anteile.sum())), 4) for i, a in enumerate(anteile)},
             kreuzungsobjekte=n_kreuz, gebaeude=len(grundrisse))
    p = os.path.join(kartenordner, DATEI)
    with open(p + '.tmp', 'w', encoding='utf-8') as f:
        json.dump(d, f, ensure_ascii=False, indent=1)
    os.replace(p + '.tmp', p)
    return d


# ---------------------------------------------------------------- lesen und abfragen

class Belegung:
    """Belegungskarte einer Karte: Klasse an einem Punkt, Masken je Kachel"""

    def __init__(self, kartenordner, daten):
        self.ordner = kartenordner
        self.daten = daten
        self._raster = {}

    @classmethod
    def laden(cls, kartenordner, pruefen=True):
        with open(os.path.join(kartenordner, DATEI), encoding='utf-8') as f:
            b = cls(kartenordner, json.load(f))
        if pruefen:
            alt = b.veraltet()
            if alt:
                raise RuntimeError(f'Belegung veraltet ({len(alt)} Kacheln, z. B. {alt[0]}) - python -m omsigen.belegung neu ausfuehren')
        return b

    def veraltet(self):
        fp = self.daten.get('fingerabdruck', {})
        out = []
        for k in schichten.kacheln(self.ordner):
            alt = fp.get(f'{k[0]}_{k[1]}', {})
            if any(alt.get(s) != schichten.fingerabdruck(self.ordner, k, s) for s in ('netz', 'gebaeude')):
                out.append(f'{k[0]}_{k[1]}')
        return out

    def raster(self, k):
        """Klassen der Kachel k als uint8-Array (Zeile 0 = Norden)"""
        if k not in self._raster:
            from PIL import Image
            p = os.path.join(self.ordner, ORDNER, f'tile_{k[0]}_{k[1]}.png')
            self._raster[k] = np.asarray(Image.open(p)) if os.path.isfile(p) else None
        return self._raster[k]

    def klasse(self, x, z):
        """Klasse (Name) am Kartenpunkt oder None ausserhalb der Karte"""
        k = (math.floor(x / KACHEL), math.floor(z / KACHEL))
        r = self.raster(k)
        if r is None:
            return None
        sp = min(N - 1, int((x - k[0] * KACHEL) / ZELLE))
        ze = min(N - 1, int((k[1] * KACHEL + KACHEL - z) / ZELLE))
        return KLASSEN[r[ze, sp]]

    def erlaubt(self, x, z, klassen):
        return self.klasse(x, z) in klassen


def main():
    ap = argparse.ArgumentParser(description='Belegungskarte einer Karte berechnen')
    ap.add_argument('karte')
    ap.add_argument('--omsi', default=schichten.STANDARD_OMSI)
    a = ap.parse_args()
    d = a.karte if os.path.isdir(a.karte) else os.path.join(a.omsi, 'maps', a.karte)
    erg = berechnen(d, a.omsi)
    print(json.dumps(dict(datei=os.path.join(d, DATEI), anteile=erg['anteile'], konflikte=len(erg['konflikte']),
                          gebaeude=erg['gebaeude'], kreuzungsobjekte=erg['kreuzungsobjekte']), ensure_ascii=False))


if __name__ == '__main__':
    main()
