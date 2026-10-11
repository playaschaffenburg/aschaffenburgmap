"""Aufmass: ein Bauwerk entlang einer Achse aus Laserdaten und Luftbild vermessen und als OMSI-Splines bauen - die
Schritte hinter dem Werkzeug „Vermessen“ im Rust-Editor (Bedienung dort, hier nur Rechnen).

  messen: Achse (vom Nutzer geklickt) + Breite links/rechts -> Laserdaten (Bayern, km-Kacheln, im Zwischenspeicher;
          fehlt eine, meldet `messen` sie mit Groesse und laedt erst mit --laden) und DOP20 -> Querschnitte je Meter,
          Kanten (Waende), Streifen dazwischen (Fahrbahn/Boden, Hoehe), Markierungen, Sperrflaechen -> messung.json.
          Jede Annahme steht als Eigenschaft darin (Wandart, Richtung, Fahrstreifen, Material, bauen ja/nein) - der
          Nutzer prueft und aendert sie im Editor.
  bauen:  messung.json -> .sli-Dateien (Splines\\Aschaffenburg\\Aufmass\\<Karte>_<Name>), Sperrflaechen-Objekt und
          aufmass.json (Sonderbau-Format: der Editor setzt es in die Sitzung) + Baupruefung.

Querweg t: positiv rechts der Achsrichtung. Ausgabe auf stdout: eine JSON-Zeile (Editor liest sie).
Quellen: Laserdaten, DOP20 Bayerische Vermessungsverwaltung (CC BY 4.0)."""
import argparse
import json
import math
import os
import sys

import numpy as np

from . import portal_modell2 as M
from . import portal_splines as S
from . import vermessung as V

LASER_URL = 'https://geodaten.bayern.de/odd_data/laser/{e}_{n}.laz'
GEODATEN = os.path.join(os.environ.get('LOCALAPPDATA', '.'), 'omsi-editor', 'geodaten')
STANDARD_OMSI = r'C:\Program Files (x86)\Steam\steamapps\common\OMSI 2'
VERSION = 1


class Achse(M.Achse):
    """Achse mit schneller Zuordnung (k-d-Baum) - auch fuer grosse Ausschnitte"""

    def __init__(self, P=None, p=None, n=None):
        if P is not None:
            super().__init__(P)
        else:
            self.p, self.n = np.asarray(p, float), np.asarray(n, float)
            self.s = np.arange(len(self.p), dtype=float)
            self.tg = np.c_[-self.n[:, 1], self.n[:, 0]]
        from scipy.spatial import cKDTree
        self._baum = cKDTree(self.p)

    def stl(self, x, y):
        """Punkte -> (Station, Querweg t, Laengsabstand zur Station)"""
        Q = np.c_[x, y]
        _, i = self._baum.query(Q)
        d = Q - self.p[i]
        return i, (d * self.n[i]).sum(axis=1), (d * self.tg[i]).sum(axis=1)

    def st(self, x, y):
        i, t, _ = self.stl(x, y)
        return i, t


def ordner(omsi, karte, name):
    return os.path.join(omsi, 'Sceneryobjects', 'Aschaffenburg', karte, 'Aufmass', name)


def bezug_lesen(omsi, karte):
    from .fotovergleich import bezug_lesen as bl
    return bl(os.path.join(omsi, 'maps', karte))


# ------------------------------------------------------------------ Daten
def laser_kacheln(bez, bereich):
    x0, y0, x1, y1 = bereich
    es = range(int(math.floor((bez['ost0'] + x0) / 1000)), int(math.floor((bez['ost0'] + x1) / 1000)) + 1)
    ns = range(int(math.floor((bez['nord0'] + y0) / 1000)), int(math.floor((bez['nord0'] + y1) / 1000)) + 1)
    return [(e, n) for e in es for n in ns]


def laser_datei(e, n):
    for d in (os.path.join(GEODATEN, 'laser'), os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), 'build', 'vermessung')):
        f = os.path.join(d, f'{e}_{n}.laz')
        if os.path.isfile(f):
            return f
    return None


def laser_groesse(e, n):
    """Dateigroesse auf dem Server (MB) - fuer die Rueckfrage vor dem Laden"""
    import requests
    try:
        r = requests.head(LASER_URL.format(e=e, n=n), timeout=30, allow_redirects=True)
        return round(int(r.headers.get('Content-Length', 0)) / 1e6)
    except Exception:
        return None


def laser_laden(e, n):
    import requests
    ziel = os.path.join(GEODATEN, 'laser')
    os.makedirs(ziel, exist_ok=True)
    f = os.path.join(ziel, f'{e}_{n}.laz')
    with requests.get(LASER_URL.format(e=e, n=n), stream=True, timeout=120) as r:
        r.raise_for_status()
        with open(f + '.teil', 'wb') as aus:
            for block in r.iter_content(1 << 20):
                aus.write(block)
    os.replace(f + '.teil', f)
    return f


class DownloadNoetig(Exception):
    def __init__(self, kacheln):
        super().__init__('Laserdaten fehlen')
        self.kacheln = kacheln


def punkte(bez, bereich, laden=False, cache=None):
    if cache and os.path.isfile(cache):
        return np.load(cache)
    fehlen = [(e, n) for e, n in laser_kacheln(bez, bereich) if laser_datei(e, n) is None]
    if fehlen and not laden:
        raise DownloadNoetig(fehlen)
    for e, n in fehlen:
        laser_laden(e, n)
    teile = [V.laden(laser_datei(e, n), bereich, bez['ost0'], bez['nord0'], bez['nn0']) for e, n in laser_kacheln(bez, bereich)]
    P = np.vstack([t for t in teile if len(t)]) if any(len(t) for t in teile) else np.zeros((0, 4))
    if cache:
        np.save(cache, P)
    return P


def luftbild(bez, bereich):
    from PIL import Image
    from .luftbildvergleich import dop20
    x0, y0, x1, y1 = bereich
    w, h = int(round((x1 - x0) * 5)), int(round((y1 - y0) * 5))
    f = dop20(bez['ost0'] + x0, bez['nord0'] + y0, bez['ost0'] + x1, bez['nord0'] + y1, w, h, os.path.join(GEODATEN, 'dop20'))
    return np.asarray(Image.open(f).convert('RGB')).astype(float)


# ------------------------------------------------------------------ Messen
def kantenlinien(G, ax, t0, t1, zelle=0.2):
    """Querschnitt je Station (Bodenpunkte bis 0,3 m laengs) -> Hoehenspruenge -> zu Linien verbunden
    [(k, t, z_unten, z_oben, seite)]"""
    i, t, l = ax.stl(G[:, 0], G[:, 1])
    schnitte = []
    for k in range(len(ax.p)):
        m = (i == k) & (np.abs(l) < 0.3) & (t > t0 - 1) & (t < t1 + 1)
        if m.sum() < 10:
            schnitte.append((k, []))
            continue
        tt, zz = t[m], G[m, 2]
        a = math.floor(tt.min() / zelle) * zelle
        j = ((tt - a) / zelle).astype(int)
        zs = np.full(j.max() + 1, np.nan)
        for q in np.unique(j):
            zs[q] = np.median(zz[j == q])
        schnitte.append((k, V.kanten(a + (np.arange(len(zs)) + 0.5) * zelle, zs, lage='tief', luecke_hoch=3.5)))
    L = V.linien(schnitte, min_laenge=6)
    return [l for l in L if np.median([b[3] - b[2] for b in l]) >= 0.5]


def kanten_aus(L, ns, t0, t1):
    """Linien -> je Kante Arrays ueber alle Stationen (wie portal_modell2.kanten_st, Achse beliebig)"""
    out = []
    idx = np.arange(ns)
    for l in L:
        A = np.array([b[:4] for b in l], float)
        T, ZU, ZO = np.full(ns, np.nan), np.full(ns, np.nan), np.full(ns, np.nan)
        for k in np.unique(A[:, 0]).astype(int):
            m = A[:, 0] == k
            T[k], ZU[k], ZO[k] = A[m, 1].mean(), A[m, 2].mean(), A[m, 3].mean()
        aktiv = ~np.isnan(T)
        if aktiv.sum() < 6:
            continue
        for arr in (T, ZU, ZO):
            arr[~aktiv] = np.interp(idx[~aktiv], idx[aktiv], arr[aktiv])
        out.append(dict(t=M.glatt(T), zu=M.glatt(ZU), zo=M.glatt(ZO), aktiv=aktiv, seite=int(l[0][4]),
                        hoehe=float(np.median(ZO[aktiv] - ZU[aktiv])), unten=float(np.median(ZU[aktiv]))))
    out = [k for k in out if t0 + 0.5 < np.median(k['t']) < t1 - 0.5]
    out.sort(key=lambda k: np.median(k['t']))
    zus = []
    for k in out:                                    # Wand + Bruestung = zwei Linien dicht beieinander
        if zus and np.median(np.abs(k['t'] - zus[-1]['t'])) < 1.0:
            a = zus[-1]
            h = k if k['hoehe'] > a['hoehe'] else a
            h = dict(h, zo=np.maximum(a['zo'], k['zo']), zu=np.minimum(a['zu'], k['zu']), aktiv=a['aktiv'] | k['aktiv'])
            h['hoehe'] = float(np.median((h['zo'] - h['zu'])[h['aktiv']]))
            zus[-1] = h
        else:
            zus.append(k)
    for a, b in zip(zus[:-1], zus[1:]):
        b['t'] = np.maximum(b['t'], a['t'] + 0.3)
    return zus


def material_quer(ax, grenzen, j, mr, bereich, schritt=0.25):
    x0, y0, x1, y1 = bereich
    H, W = mr.shape
    breite = float(np.median(grenzen[j + 1] - grenzen[j]))
    dt = np.arange(schritt / 2, breite, schritt)
    n, n_a = np.zeros(len(dt)), np.zeros(len(dt))
    for k in range(0, len(ax.p), 2):
        for i, d in enumerate(dt):
            p_ = ax.welt(k, grenzen[j][k] + d * (grenzen[j + 1][k] - grenzen[j][k]) / max(breite, 1e-6))
            px, py = int((p_[0] - x0) * 5), int((y1 - p_[1]) * 5)
            if 0 <= px < W and 0 <= py < H:
                n[i] += 1
                n_a[i] += mr[py, px] == 1
    return dt, np.where(n > 0, n_a / np.maximum(n, 1), 0.0)


def messen(omsi, karte, achse, links, rechts, name, laden=False):
    from .portal_modell import materialraster
    bez = bezug_lesen(omsi, karte)
    A = np.asarray(achse, float).reshape(-1, 2)
    if len(A) < 2 or np.hypot(*(A[-1] - A[0])) < 10:
        raise ValueError('Achse zu kurz (mindestens 10 m)')
    # dicht abtasten, dann Achse (1-m-Stationen, geglaettet)
    d = np.r_[0, np.cumsum(np.hypot(*np.diff(A, axis=0).T))]
    s = np.arange(0, d[-1] + 1e-9, 0.5)
    ax = Achse(np.c_[np.interp(s, d, A[:, 0]), np.interp(s, d, A[:, 1])])
    t0, t1 = -float(links), float(rechts)
    rand = max(links, rechts) + 3
    bereich = [math.floor(ax.p[:, 0].min() - rand), math.floor(ax.p[:, 1].min() - rand),
               math.ceil(ax.p[:, 0].max() + rand), math.ceil(ax.p[:, 1].max() + rand)]
    aus = ordner(omsi, karte, name)
    os.makedirs(aus, exist_ok=True)
    cache = os.path.join(aus, 'punkte_' + '_'.join(str(int(v)) for v in bereich) + '.npy')
    P = punkte(bez, bereich, laden, cache)
    if len(P) < 1000:
        raise ValueError('kaum Laserpunkte im Bereich (ausserhalb Bayerns?)')
    dop = luftbild(bez, bereich)
    G = P[P[:, 3] == 2]
    ns = len(ax.p)
    K = kanten_aus(kantenlinien(G, ax, t0, t1), ns, t0, t1)
    grenzen = [np.full(ns, t0)] + [k['t'] for k in K] + [np.full(ns, t1)]
    welt_linien = [[(ax.welt(k, kk['t'][k])[1], ax.welt(k, kk['t'][k])[0]) for k in range(ns) if kk['aktiv'][k]] for kk in K]
    mr = materialraster(P, bereich, dop, welt_linien)
    nb = len(grenzen) - 1
    quer = [material_quer(ax, grenzen, j, mr, bereich) for j in range(nb)]
    fahrbahn = [float((q[1] > 0.5).sum()) * 0.25 >= 5.0 for q in quer]
    a, b = M.streifen_hoehen(G, ax, grenzen, fahrbahn)
    for j in range(nb):                                  # eben quer (Splines haben keine Querneigung)
        tc = M.glatt((grenzen[j] + grenzen[j + 1]) / 2, 9)
        a[:, j] = a[:, j] + b[:, j] * tc
        b[:, j] = 0.0
    m = dict(ax=ax, K=K, grenzen=grenzen, mr=mr, fahrbahn=fahrbahn, quer=quer, a=a, b=b, t0=t0, t1=t1)
    sperr = S.sperrflaechen(m, dop, bereich)
    linien = S.linien_bereinigen(m, S.markierungen(m, dop, bereich), sperr)
    mj = messung_json(m, linien, sperr, karte, name, bereich, bez, achse)
    pfad = os.path.join(aus, 'messung.json')
    json.dump(mj, open(pfad, 'w', encoding='utf-8'), indent=1)
    return pfad, mj


def _r(a, n=3):
    return [round(float(x), n) for x in a]


def messung_json(m, linien, sperr, karte, name, bereich, bez, achse):
    """Messung -> JSON: Rechengroessen je Station (fuer bauen) und Weltpunkte (zum Zeichnen im Editor); alle Annahmen als
    Eigenschaften, die der Nutzer aendert"""
    ax, K, g, a = m['ax'], m['K'], m['grenzen'], m['a']
    ns = len(ax.p)
    ringsohle = min((k['unten'] for k in K), default=0.0)

    def welt(k, t, z):
        p = ax.welt(k, t)
        return [round(float(p[0]), 2), round(float(p[1]), 2), round(float(z), 2)]
    kanten = []
    for i, kk in enumerate(K):
        trog = kk['unten'] < ringsohle + 1.0 and kk['hoehe'] > 3.0
        kanten.append(dict(id=f'K{i}', t=_r(kk['t']), zu=_r(kk['zu']), zo=_r(kk['zo']), aktiv=[bool(x) for x in kk['aktiv']],
                           hoehe=round(kk['hoehe'], 2), unten=round(kk['unten'], 2), seite=kk['seite'],
                           art='trog' if trog else 'wand', material='paneel' if trog else 'beton',
                           gelaender=bool(kk['hoehe'] > 2.0), bauen=True,
                           punkte=[welt(k, kk['t'][k], kk['zo'][k]) for k in range(ns) if kk['aktiv'][k]]))
    streifen = []
    for j in range(len(g) - 1):
        tc = M.glatt((g[j] + g[j + 1]) / 2, 9)
        dt, asph = m['quer'][j]
        streifen.append(dict(id=f'S{j}', a=_r(a[:, j]), breite=round(float(np.median(g[j + 1] - g[j])), 2),
                             art='fahrbahn' if m['fahrbahn'][j] else 'boden', richtung='auto', spuren=0, material='auto',
                             quer=_r(asph, 2), mitte=[welt(k, tc[k], a[k, j]) for k in range(ns)]))
    mk = []
    for i, (ks, T, durch) in enumerate(linien):
        mk.append(dict(id=f'M{i}', k0=int(ks[0]), t=_r(T), durch=bool(durch), bauen=True,
                       punkte=[welt(k, t, S.z_bei(m, k, t) + 0.05) for k, t in zip(ks, T)]))
    sp = []
    for i, (j, rnd, ks, T) in enumerate(sperr):
        rand_t = g[j] if rnd < 0 else g[j + 1]
        umriss = [welt(k, rand_t[k], a[k, j] + 0.05) for k in ks] + [welt(k, t, a[k, j] + 0.05) for k, t in zip(ks[::-1], T[::-1])]
        sp.append(dict(id=f'F{i}', streifen=int(j), rand=int(rnd), k0=int(ks[0]), t=_r(T), bauen=True, umriss=umriss))
    return dict(version=VERSION, karte=karte, name=name, bereich=bereich, bezug=bez, achse_klick=achse,
                t0=m['t0'], t1=m['t1'], achse=[_r(p, 3) for p in ax.p], normale=[_r(n, 5) for n in ax.n],
                kanten=kanten, streifen=streifen, markierungen=mk, sperrflaechen=sp,
                quellen=['Laserdaten, DOP20 Bayerische Vermessungsverwaltung (CC BY 4.0)'])


def zusammenfassung(mj):
    fb = [s for s in mj['streifen'] if s['art'] == 'fahrbahn']
    return (f"{len(mj['achse'])} m Achse, {len(mj['kanten'])} Kanten, {len(fb)} Fahrbahnen, "
            f"{len(mj['streifen']) - len(fb)} Bodenstreifen, {len(mj['markierungen'])} Markierungen, {len(mj['sperrflaechen'])} Sperrflaechen")


# ------------------------------------------------------------------ Bauen
def m_aus_json(mj):
    ax = Achse(p=mj['achse'], n=mj['normale'])
    ns = len(ax.p)
    K = []
    for k in mj['kanten']:
        K.append(dict(t=np.array(k['t']), zu=np.array(k['zu']), zo=np.array(k['zo']), aktiv=np.array(k['aktiv'], bool),
                      hoehe=k['hoehe'], unten=k['unten'], seite=k['seite'], art=k['art'], material=k['material'],
                      gelaender=bool(k['gelaender']), bauen=bool(k['bauen'])))
    g = [np.full(ns, mj['t0'])] + [k['t'] for k in K] + [np.full(ns, mj['t1'])]
    a = np.array([s['a'] for s in mj['streifen']]).T
    info = {}
    for j, s_ in enumerate(mj['streifen']):
        breite = s_['breite']
        dt = np.arange(0.125, breite, 0.25)[:len(s_['quer'])]
        info[j] = dict(name=s_['id'], richtung=None if s_['richtung'] == 'auto' else s_['richtung'],
                       spuren=int(s_.get('spuren') or 0) or None, material=s_.get('material', 'auto'))
    quer = [(np.arange(0.125, s_['breite'], 0.25)[:len(s_['quer'])], np.array(s_['quer'])) for s_ in mj['streifen']]
    m = dict(ax=ax, K=K, grenzen=g, a=a, b=np.zeros_like(a), fahrbahn=[s_['art'] == 'fahrbahn' for s_ in mj['streifen']],
             quer=quer, streifen_info=info, t0=mj['t0'], t1=mj['t1'],
             ordner=f"Aufmass\\{mj['karte']}_{mj['name']}", praefix=f"AB_AM_{mj['name']}")
    linien = [(np.arange(x['k0'], x['k0'] + len(x['t'])), np.array(x['t']), x['durch']) for x in mj['markierungen'] if x['bauen']]
    sperr = [(x['streifen'], x['rand'], np.arange(x['k0'], x['k0'] + len(x['t'])), np.array(x['t'])) for x in mj['sperrflaechen'] if x['bauen']]
    return m, linien, sperr


def bauen(omsi, messung_pfad, pruefen=True):
    from .kreuzung import x_file
    from .sonderbau_ring import texturen as objekt_texturen
    mj = json.load(open(messung_pfad, encoding='utf-8'))
    m, linien, sperr = m_aus_json(mj)
    karte, name = mj['karte'], mj['name']
    aus = os.path.dirname(os.path.abspath(messung_pfad))
    rel = f'Sceneryobjects\\Aschaffenburg\\{karte}\\Aufmass\\{name}'
    objekt_texturen(omsi, os.path.join(aus, 'texture'))
    # Fahrbahnen zuerst: ihre gebaute Hoehe tragen Markierungen und Schraffur
    strassen, dateien = [], {}
    for teil, d in (S.fahrbahnen(m, linien, dict(trog=-1), sperr), S.markierung_splines(m, linien), S.waende(m), S.boden(m)):
        strassen += teil
        dateien.update(d)
    ziel = os.path.join(omsi, 'Splines', 'Aschaffenburg', *m['ordner'].split('\\'))
    S.texturen(omsi, os.path.join(ziel, 'texture'), [os.path.join(aus, 'texture')])
    for n_, text in dateien.items():
        with open(os.path.join(ziel, n_ + '.sli'), 'w', encoding='cp1252', newline='') as f:
            f.write(text)
    objekte = []
    if sperr and 'gebaut' in m:
        me, o = S.sperr_objekt(m, sperr)
        n_ = f'{name}_Sperrflaechen'
        os.makedirs(os.path.join(aus, 'model'), exist_ok=True)
        with open(os.path.join(aus, 'model', n_ + '.x'), 'w', newline='') as f:
            f.write(x_file(me.V, me.F, [('sperr', 'AB_sperrflaeche.bmp')]))
        sco = ['Erzeugt mit omsigen (aufmass): Sperrflaechen aus DOP20, Bayerische Vermessungsverwaltung (CC BY 4.0).', '',
               '[friendlyname]', f'Aufmass {name} Sperrflaechen', '', '[groups]', '1', 'Aschaffenburg', '', '[fixed]', '',
               '[absheight]', '', '[mesh]', n_ + '.x', '', '[matl]', 'AB_sperrflaeche.bmp', '0', '[matl_zbias]', '1', '']
        open(os.path.join(aus, n_ + '.sco'), 'w', encoding='cp1252', newline='').write('\r\n'.join(sco) + '\r\n')
        objekte.append(dict(name=n_, rel=rel + '\\' + n_ + '.sco', x=float(o[0]), y=float(o[1]), z=0.0))
    out = dict(karte=karte, objekte=objekte, strassen=strassen, kreuzungen=[], gelaende=S.gelaende(m, mj['bereich']),
               gelaende_zurueck=[], quellen=mj['quellen'])
    pfad = os.path.join(aus, 'aufmass.json')
    json.dump(out, open(pfad, 'w', encoding='utf-8'), indent=1)
    ergebnis = dict(json=pfad, splines=len(strassen), dateien=len(dateien), objekte=[o['rel'] for o in objekte])
    if pruefen:
        from .baupruefer import sonderbau_pruefen
        try:
            b = sonderbau_pruefen(pfad, omsi)
            ergebnis['pruefung'] = f"{len(b['funde'])} Funde" + (' (' + ', '.join(f'{k} {v}' for k, v in sorted(b['arten'].items())) + ')' if b['funde'] else '')
            ergebnis['funde'] = [dict(art=f['art'], teil=f['teil'], pos=f['pos'], text=f.get('text', '')) for f in b['funde']]
        except Exception as e:                                   # Pruefung ist Zugabe, das Bauen gilt
            ergebnis['pruefung'] = f'Baupruefung nicht moeglich: {e}'
    return ergebnis


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = ap.add_subparsers(dest='befehl', required=True)
    me = sub.add_parser('messen')
    me.add_argument('--omsi', default=STANDARD_OMSI)
    me.add_argument('--karte', required=True)
    me.add_argument('--name', required=True)
    me.add_argument('--achse', type=float, nargs='+', required=True, help='x y x y ... (Kartenkoordinaten)')
    me.add_argument('--links', type=float, default=15.0)
    me.add_argument('--rechts', type=float, default=15.0)
    me.add_argument('--laden', action='store_true', help='fehlende Laserdaten-Kacheln herunterladen')
    ba = sub.add_parser('bauen')
    ba.add_argument('--omsi', default=STANDARD_OMSI)
    ba.add_argument('--messung', required=True)
    ba.add_argument('--ohne-pruefung', action='store_true')
    a = ap.parse_args()
    try:
        if a.befehl == 'messen':
            pfad, mj = messen(a.omsi, a.karte, a.achse, a.links, a.rechts, a.name, a.laden)
            erg = dict(datei=pfad, zusammenfassung=zusammenfassung(mj))
        else:
            erg = bauen(a.omsi, a.messung, not a.ohne_pruefung)
    except DownloadNoetig as e:
        erg = dict(download=[dict(kachel=f'{x}_{y}', url=LASER_URL.format(e=x, n=y), mb=laser_groesse(x, y)) for x, y in e.kacheln])
    except Exception as e:
        erg = dict(fehler=f'{type(e).__name__}: {e}')
    sys.stdout.write(json.dumps(erg, ensure_ascii=False) + '\n')


if __name__ == '__main__':
    main()
