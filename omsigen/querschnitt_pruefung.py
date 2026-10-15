"""Querschnitte gegen das Luftbild pruefen (Qualitaetsplan S1, Abnahme): an festen Pruefstellen die Fahrbahn- und
Gehwegbreite der Karte (netz.json) mit der im Luftbild DOP20 (Bayerische Vermessungsverwaltung, CC BY 4.0) abgelesenen
vergleichen; dazu: Abbiegestreifen vorhanden, wo OSM `turn:lanes` hat.

Pruefstellen: Kartendaten `querschnitte/pruefstellen.json` (unabhaengig von der Karte, UTM):
  [{"name", "ost", "nord", "richtung": Strassenrichtung (Grad ab Nord im Uhrzeigersinn),
    "bord": [s_links, s_rechts],          Bordkanten (Fahrbahnrand), Querweg ab (ost, nord), rechts positiv
    "aussen": [s_links, s_rechts],        Hinterkante der Gehwege (null = kein Gehweg / nicht ablesbar)
    "quelle": "DOP20 abgelesen ..."}]
Die Werte liest ein Mensch (oder Claude) im Ablesebild ab (`--ablesen`): Luftbild quer zur Strasse mit Meterteilung und
den Kanten, die das Farbprofil vorschlaegt (Mittel ueber 6 m laengs, Autos und Markierungen fallen heraus). Die
Messungen des Generators (querschnitt_messung) fliessen nicht ein - die Pruefstellen bleiben ein unabhaengiger Massstab.

  python -m omsigen.querschnitt_pruefung <Karte> [--bilder]       Abweichungen je Pruefstelle (+ Bilder in pruefung/)
  python -m omsigen.querschnitt_pruefung --ablesen <Name|alle> --png-ordner d   Ablesebilder der Pruefstellen
  python -m omsigen.querschnitt_pruefung <Karte> --ausschnitt x0 z0 x1 z1 --png a.png   Netz ueber dem Luftbild
"""
import argparse
import json
import math
import os

import numpy as np

from .config import KARTENDATEN

PRUEFSTELLEN = os.path.join(KARTENDATEN, 'querschnitte', 'pruefstellen.json')
DOP_CACHE = os.path.join(os.environ.get('LOCALAPPDATA', '.'), 'omsi-editor', 'geodaten', 'dop20')
GRENZE = 0.5             # m: Abnahme S1
STANDARD_OMSI = r'C:\Program Files (x86)\Steam\steamapps\common\OMSI 2'


def dop(e0, n0, e1, n1, m_px=0.1):
    """Luftbild-Ausschnitt (UTM) -> PIL-Bild (Zeile 0 = Norden)"""
    from PIL import Image
    from .luftbildvergleich import dop20
    import time
    w, h = int(round((e1 - e0) / m_px)), int(round((n1 - n0) / m_px))
    for versuch in range(4):                       # der WMS-Dienst bricht bei vielen Anfragen gelegentlich ab
        try:
            return Image.open(dop20(round(e0, 1), round(n0, 1), round(e1, 1), round(n1, 1), w, h, DOP_CACHE)).convert('RGB')
        except Exception:
            if versuch == 3:
                raise
            time.sleep(3 * (versuch + 1))


def bezug(kartenordner):
    from .fotovergleich import bezug_lesen
    return bezug_lesen(kartenordner)


# ---------------------------------------------------------------- Netz ueber dem Luftbild

def ausschnitt_bild(kartenordner, x0, z0, x1, z1, png, m_px=0.1, omsi=None, gebaeude=True, linien=()):
    """Netz der Karte (Fahrbahnkanten rot, Aussenkanten blau, Kreuzungsplatten lila, Gebaeude gelb) ueber dem Luftbild"""
    from PIL import ImageDraw
    from .netz_datei import Netz
    b = bezug(kartenordner)
    img = dop(b['ost0'] + x0, b['nord0'] + z0, b['ost0'] + x1, b['nord0'] + z1, m_px)
    dr = ImageDraw.Draw(img, 'RGBA')

    def px(x, z):
        return ((x - x0) / m_px, (z1 - z) / m_px)
    n = Netz.laden(kartenordner, pruefen=False)
    for e in n.kanten.values():
        q = e.get('querschnitt')
        if not q or e['art'] == 'unsichtbar':
            continue
        P = e['punkte']
        if not any(x0 - 30 < p[0] < x1 + 30 and z0 - 30 < p[2] < z1 + 30 for p in P):
            continue
        fb = [px(*p) for p in n.streifen(e['id'], q['fahrbahn'][0], q['fahrbahn'][1])]
        dr.polygon(fb, fill=(255, 40, 40, 45))
        for s, farbe in ((q['fahrbahn'][0], (255, 40, 40, 255)), (q['fahrbahn'][1], (255, 40, 40, 255)),
                         (q['aussen'][0], (60, 120, 255, 255)), (q['aussen'][1], (60, 120, 255, 255))):
            if s in q['fahrbahn'] and farbe[2] == 255:
                continue                          # Aussenkante = Fahrbahnkante (ohne Gehweg)
            L = [px(*n.punkt(e['id'], p[3], s)[::2][:2]) for p in P]
            dr.line(L, fill=farbe, width=2)
    if True:
        from . import belegung as B, schichten
        omsi = omsi or STANDARD_OMSI
        for rel, ox, oz, rot, _ in B._objekte(kartenordner, lambda r: os.path.basename(r.replace(chr(92), '/')).upper().startswith('K_')):
            if not (x0 - 60 < ox < x1 + 60 and z0 - 60 < oz < z1 + 60):
                continue
            pfad = B._modellpfad(omsi, rel)
            if not pfad or not os.path.isfile(pfad):
                continue
            P, F, mat, tex = B.x_lesen(pfad)
            x, z = B._drehen(P[:, 0], P[:, 2], rot)
            for f, m in zip(F, mat):
                if m < len(tex) and 'side' not in tex[m].lower():
                    dr.polygon([px(x[i] + ox, z[i] + oz) for i in f], fill=(150, 60, 200, 50))
        if gebaeude:
            for rel, ox, oz, rot, _ in B._objekte(kartenordner, lambda r: schichten.schicht_von(r) is not None and schichten.schicht_von(r)[0] == 'gebaeude'):
                if not (x0 - 80 < ox < x1 + 80 and z0 - 80 < oz < z1 + 80):
                    continue
                pfad = B._modellpfad(omsi, rel)
                if not pfad or not os.path.isfile(pfad):
                    continue
                from shapely.geometry import Polygon
                from shapely.ops import unary_union
                P, F, _, _ = B.x_lesen(pfad)
                x, z = B._drehen(P[:, 0], P[:, 2], rot)
                tris = [Polygon([(x[i] + ox, z[i] + oz) for i in f]) for f in F]
                g = unary_union([t.buffer(0.01) for t in tris if t.area > 1e-3])
                for poly in getattr(g, 'geoms', [g]):
                    if poly.geom_type == 'Polygon':
                        dr.line([px(*c) for c in poly.exterior.coords], fill=(255, 220, 0, 230), width=2)
    for (xa, za), (xb, zb), farbe in linien:
        dr.line([px(xa, za), px(xb, zb)], fill=farbe, width=2)
    img.save(png)
    return png


# ---------------------------------------------------------------- Pruefstellen: Profil quer zur Strasse

def pruefstellen_laden(pfad=PRUEFSTELLEN):
    try:
        with open(pfad, encoding='utf-8') as f:
            return json.load(f)
    except (OSError, ValueError):
        return []


def _vektoren(richtung):
    r = math.radians(richtung)
    return (math.sin(r), math.cos(r)), (math.cos(r), -math.sin(r))      # vor, rechts (Ost, Nord)


def querprofil(ost, nord, richtung, breite=36.0, laengs=6.0, m_px=0.1):
    """Luftbild in Strassenkoordinaten: Zeilen = laengs (-laengs/2 .. laengs/2, oben = vorne), Spalten = Querweg
    (-breite/2 .. breite/2, rechts positiv) -> (Bild n_l x n_s x 3 float, s-Werte)"""
    vor, re = _vektoren(richtung)
    r = math.hypot(breite / 2, laengs / 2) + 2
    img = np.asarray(dop(ost - r, nord - r, ost + r, nord + r, m_px)).astype(float)
    H, W = img.shape[:2]
    s = np.arange(-breite / 2, breite / 2 + 1e-9, m_px)
    l = np.arange(laengs / 2, -laengs / 2 - 1e-9, -m_px)
    S, L = np.meshgrid(s, l)
    e = ost + S * re[0] + L * vor[0]
    n = nord + S * re[1] + L * vor[1]
    px = (e - (ost - r)) / m_px - 0.5
    py = ((nord + r) - n) / m_px - 0.5
    x0, y0 = np.clip(np.floor(px).astype(int), 0, W - 2), np.clip(np.floor(py).astype(int), 0, H - 2)
    fx, fy = np.clip(px - x0, 0, 1)[..., None], np.clip(py - y0, 0, 1)[..., None]
    A = (img[y0, x0] * (1 - fx) * (1 - fy) + img[y0, x0 + 1] * fx * (1 - fy) + img[y0 + 1, x0] * (1 - fx) * fy
         + img[y0 + 1, x0 + 1] * fx * fy)
    return A, s


def kanten_vorschlagen(A, s, glaettung=0.3, schwelle=None):
    """Kanten im Querprofil: Median ueber die Zeilen (laengs), Merkmale Helligkeit, Rotanteil, Gruen; Spruenge im
    geglaetteten Verlauf -> [(s, Staerke)] nach Staerke"""
    med = np.median(A, axis=0)
    hell = med.mean(axis=1)
    rot = med[:, 0] - med[:, 2]
    gruen = 2 * med[:, 1] - med[:, 0] - med[:, 2]
    m_px = s[1] - s[0]
    k = max(1, int(round(glaettung / m_px)))
    kern = np.convolve(np.ones(k + 1), np.ones(k + 1))          # Dreieck: ein Gipfel genau auf der Kante
    kern /= kern.sum()
    r = len(kern) // 2
    g = 0
    for f in (hell, 1.5 * rot, gruen):
        fg = np.convolve(np.pad(f, r, mode='edge'), kern, mode='valid')
        g = g + np.abs(np.gradient(fg) / m_px)
    schwelle = schwelle if schwelle is not None else max(15.0, float(np.percentile(g, 80)))
    out = []
    for i in range(1, len(g) - 1):
        if g[i] >= schwelle and g[i] >= g[i - 1] and g[i] >= g[i + 1]:
            if out and abs(s[i] - out[-1][0]) < 0.6:
                if g[i] > out[-1][1]:
                    out[-1] = (float(s[i]), float(g[i]))
                continue
            out.append((float(s[i]), float(g[i])))
    return sorted(out, key=lambda x: -x[1])


def ablesebild(stelle, png, karte_kanten=None, m_px=0.05, breite=36.0, laengs=30.0):
    """Ablesebild einer Pruefstelle: Luftbild quer zur Strasse (Strasse senkrecht, Blick in `richtung` nach oben),
    Meterteilung (Strich je 0,5 m, Zahl je 1 m), Kantenvorschlaege (gelb, nummeriert), abgelesene Werte (gruen) und
    die Kanten einer Karte (rot Fahrbahn, blau aussen)"""
    from PIL import Image, ImageDraw
    A, s = querprofil(stelle['ost'], stelle['nord'], stelle['richtung'], breite, laengs, m_px)
    Ap, sp = querprofil(stelle['ost'], stelle['nord'], stelle['richtung'], breite, 6.0, 0.1)
    vorschlaege = kanten_vorschlagen(Ap, sp)[:12]
    h, w = A.shape[:2]
    rand = 70
    bild = Image.new('RGB', (w, h + rand), (255, 255, 255))
    bild.paste(Image.fromarray(np.clip(A, 0, 255).astype(np.uint8)), (0, 0))
    dr = ImageDraw.Draw(bild)

    def x(sv):
        return (sv + breite / 2) / m_px
    for i in range(int(-breite / 2 * 2), int(breite / 2 * 2) + 1):
        sv = i / 2
        lang = i % 2 == 0
        dr.line([(x(sv), h), (x(sv), h + (14 if lang else 7))], fill=(0, 0, 0))
        if lang:
            dr.text((x(sv) - 6, h + 16), f'{sv:g}', fill=(0, 0, 0))
    dr.line([(x(0), 0), (x(0), h)], fill=(255, 255, 255), width=1)
    for xa in (0, w - 12):                               # Ablesezeile (Mitte)
        dr.line([(xa, h / 2), (xa + 12, h / 2)], fill=(255, 0, 255), width=3)
    for n, (sv, _) in enumerate(vorschlaege):
        dr.line([(x(sv), 0), (x(sv), h * 0.25)], fill=(255, 230, 0), width=1)
        dr.text((x(sv) + 2, 2 + 12 * (n % 3)), str(n + 1), fill=(255, 230, 0))
    for schl, farbe in (('bord', (0, 220, 0)), ('aussen', (0, 160, 0))):
        for sv in (stelle.get(schl) or []):
            if sv is not None:
                dr.line([(x(sv), h * 0.75), (x(sv), h)], fill=farbe, width=2)
    if karte_kanten:
        for sv in karte_kanten.get('bord', []):
            dr.line([(x(sv), h * 0.3), (x(sv), h * 0.7)], fill=(255, 40, 40), width=2)
        for sv in karte_kanten.get('aussen', []):
            dr.line([(x(sv), h * 0.3), (x(sv), h * 0.7)], fill=(60, 120, 255), width=2)
    dr.text((4, h + 40), f"{stelle.get('name', '')}: Vorschlaege " +
            ', '.join(f'{n + 1}={sv:+.1f}' for n, (sv, _) in enumerate(sorted(vorschlaege[:12], key=lambda v: v[0]))),
            fill=(0, 0, 0))
    bild.save(png)
    return vorschlaege


# ---------------------------------------------------------------- Vergleich mit einer Karte

def karte_an_stelle(netz, b, stelle):
    """Kanten der Karte an einer Pruefstelle im Querweg der Stelle -> dict(bord, aussen, kante, abstand) oder None"""
    x, z = stelle['ost'] - b['ost0'], stelle['nord'] - b['nord0']
    best = netz.naechste(x, z)
    if not best or best[3] > 8.0:
        return None
    kid, station, seite, abstand = best
    e = netz.kanten[kid]
    q = e.get('querschnitt')
    if not q:
        return None
    _, _, _, h = netz.punkt(kid, station)
    gleich = abs((h - stelle['richtung'] + 180.0) % 360.0 - 180.0) < 90.0
    sg = 1.0 if gleich else -1.0
    um = lambda u: sg * (u - seite)
    bord = sorted(um(u) for u in q['fahrbahn'])
    aussen = sorted(um(u) for u in q['aussen'])
    return dict(bord=bord, aussen=aussen, kante=kid, abstand=round(abstand, 2), sli=e.get('sli'),
                herkunft=(e.get('attribute', {}).get('querschnitt') or {}).get('herkunft'))


def vergleichen(kartenordner, stellen=None, bilder_ordner=None, log=print):
    """Abweichungen Karte <-> abgelesene Pruefstellen -> dict(stellen=[...], max_fahrbahn, max_gehweg, ueber_grenze)"""
    from .netz_datei import Netz
    stellen = stellen if stellen is not None else pruefstellen_laden()
    b = bezug(kartenordner)
    netz = Netz.laden(kartenordner, pruefen=False)
    out = []
    for st in stellen:
        if not st.get('bord'):
            continue
        k = karte_an_stelle(netz, b, st)
        r = dict(name=st['name'], ost=st['ost'], nord=st['nord'])
        if k is None:
            r['fehler'] = 'keine Strasse der Karte an der Stelle'
            out.append(r)
            continue
        fb_ref = st['bord'][1] - st['bord'][0]
        fb_gen = k['bord'][1] - k['bord'][0]
        r.update(fahrbahn_ref=round(fb_ref, 2), fahrbahn_karte=round(fb_gen, 2), fahrbahn_abw=round(fb_gen - fb_ref, 2),
                 bord_lage_abw=[round(k['bord'][i] - st['bord'][i], 2) for i in (0, 1)], kante=k['kante'],
                 sli=k['sli'], herkunft=k['herkunft'])
        aussen = st.get('aussen') or [None, None]
        geh = []
        for i, (ref_a, ref_b, gen_a, gen_b) in enumerate(((aussen[0], st['bord'][0], k['aussen'][0], k['bord'][0]),
                                                          (st['bord'][1], aussen[1], k['bord'][1], k['aussen'][1]))):
            if ref_a is None or ref_b is None:
                geh.append(None)
                continue
            geh.append(dict(ref=round(ref_b - ref_a, 2), karte=round(gen_b - gen_a, 2),
                            abw=round((gen_b - gen_a) - (ref_b - ref_a), 2)))
        r['gehwege'] = geh
        out.append(r)
        if bilder_ordner:
            os.makedirs(bilder_ordner, exist_ok=True)
            ablesebild(st, os.path.join(bilder_ordner, f"qs_{st['name']}.png"), karte_kanten=k)
    fb = [abs(r['fahrbahn_abw']) for r in out if 'fahrbahn_abw' in r]
    gw = [abs(g['abw']) for r in out for g in (r.get('gehwege') or []) if g]
    ueber = [r['name'] for r in out if 'fahrbahn_abw' in r and (abs(r['fahrbahn_abw']) >= GRENZE or
                                                                 any(g and abs(g['abw']) >= GRENZE for g in r['gehwege']))]
    return dict(stellen=out, n=len(fb), max_fahrbahn=max(fb, default=None), mittel_fahrbahn=round(float(np.mean(fb)), 2) if fb else None,
                max_gehweg=max(gw, default=None), mittel_gehweg=round(float(np.mean(gw)), 2) if gw else None,
                ueber_grenze=ueber, ohne_strasse=[r['name'] for r in out if 'fehler' in r])


# ---------------------------------------------------------------- Abbiegestreifen

def abbiegestreifen(kartenordner):
    """Strassenzuege mit turn:lanes in OSM: hat der Querschnitt so viele Spuren in dieser Richtung? -> (geprueft, Funde)"""
    from .netz_datei import Netz
    netz = Netz.laden(kartenordner, pruefen=False)
    geprueft, funde = 0, []
    for e in netz.kanten.values():
        a = e.get('attribute') or {}
        t, q = a.get('tags') or {}, e.get('querschnitt')
        if not q or not any(k.startswith('turn:lanes') for k in t):
            continue
        qs = (a.get('querschnitt') or {})
        oneway = t.get('oneway') in ('yes', '1', '-1', 'true')
        for schl, richtung in (('turn:lanes', None), ('turn:lanes:forward', 'vor'), ('turn:lanes:backward', 'zurueck')):
            v = t.get(schl)
            if not v or (schl == 'turn:lanes' and not oneway):
                continue
            soll = len(str(v).split('|'))
            werte = qs.get('werte') or {}
            if richtung is None or oneway:
                ist = werte.get('spuren_vor') if werte else len(q['spuren'])
            else:
                ist = werte.get('spuren_vor' if richtung == 'vor' else 'spuren_zurueck')
            if ist is None:
                continue
            geprueft += 1
            if ist < soll:
                p = e['punkte'][len(e['punkte']) // 2]
                funde.append(dict(art='abbiegestreifen_fehlt', x=round(p[0], 1), z=round(p[2], 1),
                                  text=f"{a.get('name', '')} {schl}={v}: {ist} statt {soll} Spuren"))
    return geprueft, funde


def funde(kartenordner, log=print):
    """fuer den Pruefbericht (F8): Pruefstellen mit Abweichung >= GRENZE und fehlende Abbiegestreifen"""
    out = []
    _, fa = abbiegestreifen(kartenordner)
    out += fa
    stellen = pruefstellen_laden()
    if stellen and os.path.isfile(os.path.join(kartenordner, 'omsi-editor-geo.cfg')):
        v = vergleichen(kartenordner, stellen)
        b = bezug(kartenordner)
        for r in v['stellen']:
            if r['name'] in v['ueber_grenze']:
                out.append(dict(art='querschnitt_abweichung', x=round(r['ost'] - b['ost0'], 1), z=round(r['nord'] - b['nord0'], 1),
                                text=f"{r['name']}: Fahrbahn {r['fahrbahn_karte']} statt {r['fahrbahn_ref']} m" +
                                ''.join(f", Gehweg {g['karte']} statt {g['ref']} m" for g in r['gehwege'] if g and abs(g['abw']) >= GRENZE)))
    return out


def main():
    ap = argparse.ArgumentParser(description='Querschnitte gegen das Luftbild pruefen (S1)')
    ap.add_argument('karte', nargs='?')
    ap.add_argument('--omsi', default=STANDARD_OMSI)
    ap.add_argument('--bilder', action='store_true', help='Ablesebilder mit den Kanten der Karte nach maps/<Karte>/pruefung/')
    ap.add_argument('--ablesen', help='Name einer Pruefstelle oder "alle": Ablesebilder ohne Karte')
    ap.add_argument('--png-ordner', default=os.path.join('build', 'querschnitte'))
    ap.add_argument('--ausschnitt', type=float, nargs=4)
    ap.add_argument('--png')
    a = ap.parse_args()
    if a.ablesen:
        os.makedirs(a.png_ordner, exist_ok=True)
        for st in pruefstellen_laden():
            if a.ablesen in ('alle', st['name']):
                v = ablesebild(st, os.path.join(a.png_ordner, f"ablesen_{st['name']}.png"))
                print(st['name'], ' '.join(f'{sv:+.2f}' for sv, _ in sorted(v, key=lambda x: x[0])))
        return
    d = a.karte if os.path.isdir(a.karte) else os.path.join(a.omsi, 'maps', a.karte)
    if a.ausschnitt:
        print(ausschnitt_bild(d, *a.ausschnitt, a.png or 'ausschnitt.png', omsi=a.omsi))
        return
    v = vergleichen(d, bilder_ordner=os.path.join(d, 'pruefung', 'querschnitte') if a.bilder else None)
    g, fa = abbiegestreifen(d)
    v['abbiegestreifen'] = dict(geprueft=g, fehlen=len(fa), funde=fa)
    with open(os.path.join(d, 'pruefung', 'querschnitte.json') if os.path.isdir(os.path.join(d, 'pruefung')) else
              os.path.join(d, 'querschnitt_pruefung.json'), 'w', encoding='utf-8') as f:
        json.dump(v, f, ensure_ascii=False, indent=1)
    for r in v['stellen']:
        if 'fehler' in r:
            print(f"{r['name']:>6}: {r['fehler']}")
            continue
        gw = ' '.join(f"G{i}: {g['karte']:5.2f}/{g['ref']:5.2f} ({g['abw']:+.2f})" if g else f'G{i}: -' for i, g in enumerate(r['gehwege']))
        print(f"{r['name']:>6}: Fahrbahn {r['fahrbahn_karte']:5.2f}/{r['fahrbahn_ref']:5.2f} ({r['fahrbahn_abw']:+.2f}) "
              f"Bordlage {r['bord_lage_abw']}  {gw}  {r.get('herkunft')}")
    print(json.dumps({k: v[k] for k in ('n', 'max_fahrbahn', 'mittel_fahrbahn', 'max_gehweg', 'mittel_gehweg', 'ueber_grenze', 'ohne_strasse')},
                     ensure_ascii=False))
    print(f"Abbiegestreifen: {g} geprueft, {len(fa)} fehlen")


if __name__ == '__main__':
    main()
