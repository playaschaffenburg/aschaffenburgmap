"""Kreuzung Ring/Ludwigsallee in Abschnitte teilen: jeder Abschnitt bekommt eigene Dateien (Praefix `RA_<Kurz>__`),
damit er spaeter einzeln neu gebaut werden kann, ohne dass sich die anderen veraendern.

Grundlage ist der gespeicherte Stand auf der Karte (nicht der Bauplan): alle Eintraege des Sonderbaus (Objekte aus
`Sonderbau_Ring\\`, Splines `AB_SB_*`) werden einem Abschnitt zugeordnet; Objekte ueber mehrere Abschnitte (Sohle,
Gelaendedeckel, Wand-/Randenden) werden an den Grenzen geteilt (Dreiecke nach ihrem Schwerpunkt). Die Kreuzung
selbst (Objekt mit Pfaden und Ampeln) bleibt ganz im Abschnitt Platte.

Ausgabe: kopierte/geteilte Dateien neben den alten und `abschnitte_plan.json` (je Eintrag der Karte: Kachel, ID, alter
Pfad, neue Pfade). Der Editor wendet den Plan in der Sitzung an (Objekt-Editor -> Abschnitte -> Aufteilen); erst
Speichern schreibt die Karte (mit Sicherung). Die Karte selbst wird hier nicht veraendert."""
import argparse
import glob
import json
import os
import re
import shutil
import sys

import numpy as np

STANDARD_OMSI = r'C:\Program Files (x86)\Steam\steamapps\common\OMSI 2'
DECKE = (-248.9, -284.0)              # y der Portale (wie sonderbau_ring.DECKE)

ABSCHNITTE = [
    ('NW', 'Nord-West', 'Westrampe Nord mit Aussen- und Trennwand'),
    ('NT', 'Nord-Trog', 'Ring im Trog nord, Sohle'),
    ('NO', 'Nord-Ost', 'Ostrampe Nord mit Aussen- und Trennwand, Weg'),
    ('PL', 'Platte', 'Kreuzung auf der Platte mit Ampeln, Unterfuehrung, Portale'),
    ('SW', 'Sued-West', 'Westrampe Sued mit Aussen- und Trennwand'),
    ('ST', 'Sued-Trog', 'Ring im Trog sued, Sohle'),
    ('SO', 'Sued-Ost', 'Ostrampe Sued mit Aussen- und Trennwand, Mauer/Hecke, Weg'),
    ('BW', 'Bruecken-West', 'Bahnbruecke und Schiesshausbruecke'),
    ('LO', 'Ludwigsallee-Ost', 'Ludwigsallee Ost und Weg Ost'),
]
KURZ = [a[0] for a in ABSCHNITTE]
PRAEFIX = 'RA_{}__'


def gehoert_dazu(pfad):
    return ('Sonderbau_Ring\\' in pfad or '\\AB_SB_' in pfad) and '\\RA_' not in pfad and '__' not in os.path.basename(pfad)


# ------------------------------------------------------------------ Karte lesen
def eintraege(karte_dir):
    """alle Sonderbau-Eintraege der Karte: dict(kachel, id, art, pfad, x, y)"""
    out = []
    for f in glob.glob(os.path.join(karte_dir, 'tile_*.map')):
        kx, ky = map(int, re.findall(r'tile_(-?\d+)_(-?\d+)\.map$', f)[0])
        z = open(f, 'rb').read().decode('utf-16').splitlines()
        for i, l in enumerate(z):
            art = l.strip().lower()
            if art not in ('[object]', '[spline]', '[spline_h]') or i + 8 >= len(z):
                continue
            pfad = z[i + 2].strip()
            if not gehoert_dazu(pfad):
                continue
            try:
                if art == '[object]':
                    x, y = float(z[i + 4]), float(z[i + 5])
                else:
                    x, y = float(z[i + 6]), float(z[i + 8])
            except ValueError:
                continue
            out.append(dict(kachel=[kx, ky], id=int(float(z[i + 3])), art=art, pfad=pfad, x=kx * 300 + x, y=ky * 300 + y, ox=x, oy=y))
    return out


class Lage:
    """Abschnitt eines Punkts: Hoehe (y) gegen die Portale, quer gegen die Trennwaende (aus der Karte)"""

    def __init__(self, alle):
        self.w = self._linie([e for e in alle if '_trenn_' in e['pfad'] and re.search(r'_w(_\d+m)?\.sli$', e['pfad'])])
        self.o = self._linie([e for e in alle if '_trenn_' in e['pfad'] and re.search(r'_o(_\d+m)?\.sli$', e['pfad'])])

    @staticmethod
    def _linie(es):
        P = sorted((e['y'], e['x']) for e in es)
        return np.array(P) if P else np.zeros((0, 2))

    def quer(self, x, y):
        """-1 West(rampe), 0 Trog, +1 Ost(rampe)"""
        if len(self.w) == 0 or len(self.o) == 0:
            return 0
        xw = np.interp(y, self.w[:, 0], self.w[:, 1])
        xo = np.interp(y, self.o[:, 0], self.o[:, 1])
        if x < xw - 0.4:
            return -1
        if x > xo + 0.4:
            return 1
        return 0

    def abschnitt(self, x, y):
        if x < 1055 and -295 < y < -240:
            return 'BW'
        if x > 1104 and -300 < y < -225:
            return 'LO'
        if DECKE[1] <= y <= DECKE[0]:
            return 'PL'
        n = 'N' if y > DECKE[0] else 'S'
        return n + {-1: 'W', 0: 'T', 1: 'O'}[self.quer(x, y)]


def zuordnen(e, lage):
    """Eintrag -> Abschnitt (Namensregeln vor Lage) oder 'teilen' (Objekt ueber mehrere Abschnitte)"""
    p = e['pfad']
    name = os.path.basename(p)
    n = 'N' if e['y'] > DECKE[0] else ('S' if e['y'] < DECKE[1] else None)
    if e['art'] == '[object]':
        if name.startswith('Kreuzung_') or name.startswith('Unterfuehrung'):
            return 'PL'
        if name.startswith('Bahnbruecke'):
            return 'BW'
        return 'teilen'
    if 'Schiesshausbruecke' in name:
        return 'BW'
    if 'Ludwigsallee_Ost' in name:
        return 'LO'
    if '_portal_' in name or '_trenn_' in name and '_flach_' in name:
        return 'PL'
    if n is None:
        return 'PL' if lage.abschnitt(e['x'], e['y']) == 'PL' else lage.abschnitt(e['x'], e['y'])
    # Trennwand gehoert zur Rampe: ihre Bruestung mit Handlauf steht auf der Rampe (Fotos)
    if '_trenn_' in name:
        return n + ('W' if re.search(r'_w(_\d+m)?\.sli$', name) else 'O')
    if 'Ring_Trog' in name:
        return n + 'T'
    if 'Rampe_West' in name or re.search(r'_aussen_.*_w(_\d+m)?\.sli$', name):
        return n + 'W'
    if 'Rampe_Ost' in name or re.search(r'_aussen_.*_o(_\d+m)?\.sli$', name):
        return n + 'O'
    return lage.abschnitt(e['x'], e['y'])


# ------------------------------------------------------------------ .x lesen/schreiben (Format von kreuzung.x_file)
def x_lesen(pfad):
    z = open(pfad, encoding='cp1252').read().splitlines()
    i = next(k for k, l in enumerate(z) if l.strip().startswith('Mesh '))
    zahl = lambda s: [float(v) for v in re.findall(r'-?\d+(?:\.\d+)?(?:[eE][-+]?\d+)?', s)]
    nv = int(zahl(z[i + 1])[0])
    V = np.array([zahl(l)[:3] for l in z[i + 2:i + 2 + nv]])
    j = i + 2 + nv
    nf = int(zahl(z[j])[0])
    F = np.array([zahl(l)[1:4] for l in z[j + 1:j + 1 + nf]], int)
    k = next(q for q in range(j, len(z)) if 'MeshNormals' in z[q])
    N = np.array([zahl(l)[:3] for l in z[k + 2:k + 2 + nv]])
    k = next(q for q in range(k, len(z)) if 'MeshTextureCoords' in z[q])
    UV = np.array([zahl(l)[:2] for l in z[k + 2:k + 2 + nv]])
    k = next(q for q in range(k, len(z)) if 'MeshMaterialList' in z[q])
    nm, nfm = int(zahl(z[k + 1])[0]), int(zahl(z[k + 2])[0])
    M = np.array([int(zahl(l)[0]) for l in z[k + 3:k + 3 + nfm]], int)
    mats = re.findall(r'Material\s+(\w+)\s*\{[^}]*?TextureFilename\s*\{\s*"([^"]*)"', '\n'.join(z[k + 3 + nfm:]), re.S)
    return V, N, UV, F, M, mats


def x_schreiben(pfad, V, N, UV, F, M, mats):
    f = lambda v: f'{v:.4f}'
    L = ['xof 0302txt 0032', '', 'Mesh Kreuzung {', f' {len(V)};']
    L += [f' {f(v[0])};{f(v[1])};{f(v[2])};' + (',' if i < len(V) - 1 else ';') for i, v in enumerate(V)]
    L += [f' {len(F)};'] + [f' 3;{a},{b},{c};' + (',' if i < len(F) - 1 else ';') for i, (a, b, c) in enumerate(F)]
    L += [' MeshNormals {', f'  {len(V)};'] + [f'  {f(v[0])};{f(v[1])};{f(v[2])};' + (',' if i < len(N) - 1 else ';') for i, v in enumerate(N)]
    L += [f'  {len(F)};'] + [f'  3;{a},{b},{c};' + (',' if i < len(F) - 1 else ';') for i, (a, b, c) in enumerate(F)]
    L += [' }', ' MeshTextureCoords {', f'  {len(V)};'] + [f'  {f(v[0])};{f(v[1])};' + (',' if i < len(UV) - 1 else ';') for i, v in enumerate(UV)]
    L += [' }', ' MeshMaterialList {', f'  {len(mats)};', f'  {len(F)};'] + [f'  {m}' + (',' if i < len(M) - 1 else ';') for i, m in enumerate(M)]
    for name, tex in mats:
        L += [f'  Material {name} {{', '   1.000000;1.000000;1.000000;1.000000;;', '   0.000000;', '   0.000000;0.000000;0.000000;;',
              '   0.000000;0.000000;0.000000;;', f'   TextureFilename {{ "{tex}"; }}', '  }']
    L += [' }', '}', '']
    open(pfad, 'w', encoding='cp1252', newline='').write('\r\n'.join(L))


def x_teilen(V, N, UV, F, M, maske):
    """nur die Dreiecke mit maske -> Netz mit neu nummerierten Ecken"""
    Fm = F[maske]
    benutzt = np.unique(Fm)
    neu = -np.ones(len(V), int)
    neu[benutzt] = np.arange(len(benutzt))
    return V[benutzt], N[benutzt], UV[benutzt], neu[Fm], M[maske]


# ------------------------------------------------------------------ Dateien je Abschnitt
def sco_kopieren(quelle, ziel, modelle):
    """.sco kopieren, [mesh]-Dateien umbenennen: modelle {alt: neu}"""
    z = open(quelle, encoding='cp1252').read().replace('\r\n', '\n').split('\n')
    for i in range(len(z) - 1):
        if z[i].strip().lower() == '[mesh]' and z[i + 1].strip() in modelle:
            z[i + 1] = modelle[z[i + 1].strip()]
    open(ziel, 'w', encoding='cp1252', newline='').write('\r\n'.join(z))


def sco_modelle(pfad):
    z = open(pfad, encoding='cp1252').read().replace('\r\n', '\n').split('\n')
    return [z[i + 1].strip() for i in range(len(z) - 1) if z[i].strip().lower() == '[mesh]']


def aufteilen(omsi, karte):
    kdir = os.path.join(omsi, 'maps', karte)
    alle = eintraege(kdir)
    if not alle:
        raise ValueError('kein Sonderbau Ring auf der Karte (oder schon aufgeteilt)')
    lage = Lage(alle)
    plan, zaehl, fehlt = [], {k: 0 for k in KURZ}, []
    erledigt = {}                                   # alter Pfad (+Abschnitt) -> neuer Pfad bzw. Teile
    for e in alle:
        ab = zuordnen(e, lage)
        alt = e['pfad']
        quelle = os.path.join(omsi, alt)
        ordner, name = os.path.split(alt)
        if ab != 'teilen':
            neu_rel = os.path.join(ordner, PRAEFIX.format(ab) + name)
            if (alt, ab) not in erledigt:
                ziel = os.path.join(omsi, neu_rel)
                if e['art'] == '[object]':
                    mod = {m: PRAEFIX.format(ab) + m for m in sco_modelle(quelle)}
                    for a, n in mod.items():
                        shutil.copyfile(os.path.join(os.path.dirname(quelle), 'model', a), os.path.join(os.path.dirname(quelle), 'model', n))
                    sco_kopieren(quelle, ziel, mod)
                elif os.path.isfile(quelle):
                    shutil.copyfile(quelle, ziel)
                else:
                    fehlt.append(alt)
                erledigt[(alt, ab)] = neu_rel
            plan.append(dict(e, neu=[neu_rel], abschnitte=[ab]))
            zaehl[ab] += 1
            continue
        # Objekt teilen: Dreiecke nach Schwerpunkt (Objektkoordinaten x, hoch, vor; Objekt ohne Drehung am Ursprung x/y)
        if alt not in erledigt:
            teile = {}
            modelle = sco_modelle(quelle)
            for mname in modelle:
                V, N, UV, F, M, mats = x_lesen(os.path.join(os.path.dirname(quelle), 'model', mname))
                S = V[F].mean(axis=1)
                ab_d = np.array([lage.abschnitt(e['x'] + s[0], e['y'] + s[2]) for s in S])
                for ab in np.unique(ab_d):
                    netz = x_teilen(V, N, UV, F, M, ab_d == ab)
                    neu_m = PRAEFIX.format(ab) + mname
                    x_schreiben(os.path.join(os.path.dirname(quelle), 'model', neu_m), *netz, mats)
                    teile.setdefault(str(ab), {})[mname] = neu_m
            neue = []
            for ab, mod in sorted(teile.items(), key=lambda t: KURZ.index(t[0])):
                neu_rel = os.path.join(ordner, PRAEFIX.format(ab) + name)
                # Modelle dieses Abschnitts; fehlt eines (keine Dreiecke darin), faellt die [mesh]-Zeile weg
                z = open(quelle, encoding='cp1252').read().replace('\r\n', '\n').split('\n')
                out, i = [], 0
                while i < len(z):
                    if z[i].strip().lower() == '[mesh]' and i + 1 < len(z):
                        m = z[i + 1].strip()
                        if m in mod:
                            out += [z[i], mod[m]]
                        i += 2
                        continue
                    out.append(z[i])
                    i += 1
                open(os.path.join(omsi, neu_rel), 'w', encoding='cp1252', newline='').write('\r\n'.join(out))
                neue.append((ab, neu_rel))
            erledigt[alt] = neue
        neue = erledigt[alt]
        plan.append(dict(e, neu=[n for _, n in neue], abschnitte=[a for a, _ in neue]))
        for a, _ in neue:
            zaehl[a] += 1
    aus = os.path.join(omsi, 'Sceneryobjects', 'Aschaffenburg', karte, 'Sonderbau_Ring', 'abschnitte_plan.json')
    json.dump(dict(karte=karte, abschnitte=[dict(kurz=k, name=n, inhalt=b) for k, n, b in ABSCHNITTE], praefix=PRAEFIX,
                   eintraege=plan, fehlt=fehlt), open(aus, 'w', encoding='utf-8'), indent=1, ensure_ascii=False)
    return aus, zaehl, len(plan), fehlt


# ------------------------------------------------------------------ einen Abschnitt aus einem neuen Bau
def _qs_name(st):
    return st.get('querschnitt', {}).get('name', st.get('name', ''))


def abschnitt_aus_bau(omsi, karte, ab):
    """aus sonderbau_ring.json (frisch gebaut) nur den Abschnitt `ab` -> abschnitt_<ab>.json (Sonderbau-Format, eigene
    Dateien mit Praefix); der Editor ersetzt damit nur die Eintraege dieses Abschnitts"""
    import copy
    if ab not in KURZ:
        raise ValueError(f'unbekannter Abschnitt {ab}')
    sb = os.path.join(omsi, 'Sceneryobjects', 'Aschaffenburg', karte, 'Sonderbau_Ring')
    j = json.load(open(os.path.join(sb, 'sonderbau_ring.json'), encoding='utf-8'))
    pfx = PRAEFIX.format(ab)
    # Lage aus den Trennwaenden dieses Baus
    pseudo = []
    for st in j['strassen']:
        p = st.get('sli') or ('Splines\\Aschaffenburg\\AB_' + re.sub(r'[^A-Za-z0-9_]+', '_', _qs_name(st)) + '.sli')
        for e in st.get('elemente', []):
            pseudo.append(dict(art='[spline_h]', pfad=p, x=e['x'], y=e['y']))
    lage = Lage(pseudo)
    strassen = []
    for st in j['strassen']:
        p = st.get('sli') or ('Splines\\Aschaffenburg\\AB_' + re.sub(r'[^A-Za-z0-9_]+', '_', _qs_name(st)) + '.sli')
        lauf, teile = [], []
        for e in st.get('elemente', []):
            if zuordnen(dict(art='[spline_h]', pfad=p, x=e['x'], y=e['y']), lage) == ab:
                lauf.append(e)
            elif lauf:
                teile.append(lauf)
                lauf = []
        if lauf:
            teile.append(lauf)
        for n, els in enumerate(teile):
            neu = copy.deepcopy(st)
            neu['name'] = f"{pfx}{st.get('name', '')}_{n}"
            neu['elemente'] = els
            if st.get('sli'):
                ordner, name = os.path.split(st['sli'])
                ziel = os.path.join(ordner, pfx + name)
                if os.path.isfile(os.path.join(omsi, st['sli'])):
                    shutil.copyfile(os.path.join(omsi, st['sli']), os.path.join(omsi, ziel))
                neu['sli'] = ziel
            else:
                # der Editor schreibt die .sli aus dem Querschnitt: Name mit Praefix -> AB_RA_<ab>__....sli
                neu['querschnitt']['name'] = pfx + _qs_name(st)
            strassen.append(neu)
    objekte = []
    for o in j['objekte']:
        e = dict(art='[object]', pfad=o['rel'], x=o['x'], y=o['y'])
        z = zuordnen(e, lage)
        quelle = os.path.join(omsi, o['rel'])
        ordner, name = os.path.split(o['rel'])
        neu_rel = os.path.join(ordner, pfx + name)
        if z == ab:
            mod = {m: pfx + m for m in sco_modelle(quelle)}
            for a_, n_ in mod.items():
                shutil.copyfile(os.path.join(sb, 'model', a_), os.path.join(sb, 'model', n_))
            sco_kopieren(quelle, os.path.join(omsi, neu_rel), mod)
            objekte.append(dict(o, rel=neu_rel, name=pfx + o['name']))
        elif z == 'teilen':
            mod = {}
            for mname in sco_modelle(quelle):
                V, N, UV, F, M, mats = x_lesen(os.path.join(sb, 'model', mname))
                S = V[F].mean(axis=1)
                maske = np.array([lage.abschnitt(o['x'] + s[0], o['y'] + s[2]) == ab for s in S], bool)
                if maske.any():
                    x_schreiben(os.path.join(sb, 'model', pfx + mname), *x_teilen(V, N, UV, F, M, maske), mats)
                    mod[mname] = pfx + mname
            if not mod:
                continue
            zl = open(quelle, encoding='cp1252').read().replace('\r\n', '\n').split('\n')
            out, i = [], 0
            while i < len(zl):
                if zl[i].strip().lower() == '[mesh]' and i + 1 < len(zl):
                    if zl[i + 1].strip() in mod:
                        out += [zl[i], mod[zl[i + 1].strip()]]
                    i += 2
                    continue
                out.append(zl[i])
                i += 1
            open(os.path.join(omsi, neu_rel), 'w', encoding='cp1252', newline='').write('\r\n'.join(out))
            objekte.append(dict(o, rel=neu_rel, name=pfx + o['name']))
    kreuzungen = []
    if ab == 'PL':
        for k in j['kreuzungen']:
            quelle = os.path.join(omsi, k['rel'])
            ordner, name = os.path.split(k['rel'])
            mod = {m: pfx + m for m in sco_modelle(quelle)}
            for a_, n_ in mod.items():
                shutil.copyfile(os.path.join(sb, 'model', a_), os.path.join(sb, 'model', n_))
            sco_kopieren(quelle, os.path.join(omsi, ordner, pfx + name), mod)
            kreuzungen.append(dict(k, rel=os.path.join(ordner, pfx + name)))
    gelaende = [g for g in j.get('gelaende', []) if lage.abschnitt(g[0] * 300 + g[2] * 5.0, g[1] * 300 + g[3] * 5.0) == ab]
    out = dict(karte=karte, nn0=j.get('nn0'), abschnitt=ab, marker=pfx, objekte=objekte, strassen=strassen, kreuzungen=kreuzungen,
               gelaende=gelaende, gelaende_zurueck=[], quellen=j.get('quellen', []))
    pfad = os.path.join(sb, f'abschnitt_{ab}.json')
    json.dump(out, open(pfad, 'w', encoding='utf-8'), indent=1, ensure_ascii=False)
    return pfad, out


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--omsi', default=STANDARD_OMSI)
    ap.add_argument('--karte', default='Aburg_Newgen_testring')
    ap.add_argument('--abschnitt', default=None, help='nur diesen Abschnitt aus dem letzten Bau (sonderbau_ring.json) herausloesen')
    a = ap.parse_args()
    try:
        if a.abschnitt:
            pfad, out = abschnitt_aus_bau(a.omsi, a.karte, a.abschnitt)
            erg = dict(json=pfad, objekte=[o['rel'] for o in out['objekte']] + [k['rel'] for k in out['kreuzungen']],
                       splines=sum(len(s['elemente']) for s in out['strassen']), marker=out['marker'])
        else:
            aus, zaehl, n, fehlt = aufteilen(a.omsi, a.karte)
            erg = dict(plan=aus, eintraege=n, je_abschnitt=zaehl, fehlt=fehlt)
    except Exception as e:
        erg = dict(fehler=f'{type(e).__name__}: {e}')
    sys.stdout.write(json.dumps(erg, ensure_ascii=False) + '\n')


if __name__ == '__main__':
    main()
