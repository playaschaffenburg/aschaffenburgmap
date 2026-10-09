"""Entwurf Ring/Ludwigsallee: Linienfuehrung und Hoehenverlauf aus DGM1, OSM und Luftbild (sonderbau_entwurf).

1. Messen: senkrecht zur (vorlaeufigen) Ring-Achse alle 1 m ein Querprofil (0,25 m, DGM bilinear). Kanten liegen
   dort, wo die Hoehe die Mitte zwischen den beiden Ebenen kreuzt (bei einer senkrechten Wand genau ihre Lage, auch
   wenn das 1-m-DGM sie verschmiert).
2. Achse: Geraden + Boegen durch die Mitte der Ring-Fahrbahn.
3. Kanten: Abstaende zur Achse, abschnittsweise fest, S-Uebergaenge.
4. Gradienten: Ring, Rampe West, Rampe Ost.
5. Pruefbilder: Grundriss ueber dem Luftbild, Laengsschnitte mit Messung.

Aufruf: python -m omsigen.ring_entwurf --karte <OMSI>/maps/Aburg_Newgen --osm sonderbau/ring/osm_map.xml --aus sonderbau/ring/entwurf
Daten: OpenStreetMap (ODbL), DGM1/DOP40 Bayerische Vermessungsverwaltung (CC BY 4.0)."""
import argparse, io, json, math, os, sys
import numpy as np
from .geodaten import Dgm1, luftbild
from .sonderbau_entwurf import Gelaende, achse_anpassen, gradiente_anpassen, versatz_anpassen, ohne_ausreisser
from . import sonderbau_ring as sr

AUSSCHNITT = (1000, 1170, -460, -100)        # x0, x1, y0, y1 (Kartenkoordinaten)
DECKE = (-282.0, -251.0)                     # y-Bereich, in dem das DGM die Decke/Kreuzung zeigt (nicht messen)
SEITEN = {'w': 1, 'o': -1}                   # Achse Nord -> Sued: rechts (t > 0) = West


def _kreuzung(ts, h, c, start, richtung):
    """erste Stelle ab Index `start` in `richtung`, an der h die Hoehe c kreuzt (linear interpoliert)"""
    i = start
    while 0 <= i + richtung < len(ts):
        a, b = h[i] - c, h[i + richtung] - c
        if a == 0:
            return ts[i]
        if a * b < 0:
            f = a / (a - b)
            return ts[i] + f * (ts[i + richtung] - ts[i])
        i += richtung
    return None


def querprofil(ts, h, vz):
    """Kanten auf einer Seite (vz = +1 rechts, -1 links) -> dict(ring, zf, rampe_innen, zr, rampe_aussen, zg) (None,
    wo nicht messbar)"""
    i0 = int(np.argmin(np.abs(ts)))
    zf = float(np.median(h[np.abs(ts) <= 1.5]))
    out = dict(zf=zf, ring=None, rampe_innen=None, zr=None, rampe_aussen=None, zg=None)
    # erste Stelle deutlich ueber der Sohle
    i = i0
    while 0 <= i < len(ts) and h[i] < zf + 0.5:
        i += vz
    if not 0 <= i < len(ts) or abs(ts[i]) > 12:
        return out
    t1 = ts[i]
    weiter = (vz * (ts - t1) >= 2.0) & (vz * (ts - t1) <= 5.0)
    if not weiter.any():
        return out
    zt = float(np.median(h[weiter]))
    if zt - zf < 0.8:
        return out                                     # keine Wand: Rampe muendet auf Ring-Hoehe
    out['ring'] = _kreuzung(ts, h, (zf + zt) / 2, i0, vz)
    out['zr'] = zt
    # Rampe: Ebene zt; nach aussen, bis die Hoehe um mehr als 0,6 m abweicht (Aussenwand oder Boeschung)
    j = int(np.argmin(np.abs(ts - (t1 + vz * 2.0))))
    while 0 <= j < len(ts) and abs(h[j] - zt) < 0.6:
        j += vz
    if not 0 <= j < len(ts):
        return out
    to = ts[j]
    jenseits = (vz * (ts - to) >= 1.5) & (vz * (ts - to) <= 4.0)
    if not jenseits.any():
        return out
    zg = float(np.median(h[jenseits]))
    out['rampe_aussen'] = _kreuzung(ts, h, (zt + zg) / 2, int(np.argmin(np.abs(ts - (t1 + vz * 2.0)))), vz)
    out['zg'] = zg
    return out


def messen(gel, achse, s_werte, breite=30.0):
    """Querprofile senkrecht zur Achse -> Liste dict(s, x, y, w=..., o=...)"""
    ts = np.arange(-breite, breite + 0.001, 0.25)
    out = []
    for s in s_werte:
        x, y, th = achse.punkt(s)
        if DECKE[0] <= y <= DECKE[1]:
            continue
        P = achse.versetzt(np.full(ts.shape, s), ts)
        h = gel.h(P[:, 0], P[:, 1])
        e = dict(s=float(s), x=x, y=y)
        for seite, vz in SEITEN.items():
            e[seite] = querprofil(ts, h, vz)
        out.append(e)
    return out


def vorlaeufige_achse(osm, E0, N0):
    from .sonderbau_entwurf import Achse
    ways = sr.lade_osm(osm, E0, N0)
    ring = sr.linie(ways, sr.RING)
    m = (ring[:, 1] < AUSSCHNITT[3] - 10) & (ring[:, 1] > AUSSCHNITT[2] + 10)
    # Polygonzug als Achse aus kurzen Geraden (nur fuer die erste Messung)
    P = ring
    d = np.diff(P, axis=0)
    l = np.hypot(d[:, 0], d[:, 1])
    th = np.arctan2(d[:, 1], d[:, 0])
    # Knicke als sehr kurze Boegen: Polygon fein abtasten, dann glatt anpassen
    fein = []
    for p, q in zip(P[:-1], P[1:]):
        n = max(2, int(math.dist(p, q) / 2))
        for f in np.linspace(0, 1, n, endpoint=False):
            fein.append(p + (q - p) * f)
    fein.append(P[-1])
    fein = np.array(fein)
    fein = fein[(fein[:, 1] < AUSSCHNITT[3] - 5) & (fein[:, 1] > AUSSCHNITT[2] + 5)]
    a, rms = achse_anpassen(fein, toleranz=1.0, max_elemente=5)
    return a, ring


def entwerfen(gel, osm, E0, N0, log=print):
    a0, ring_osm = vorlaeufige_achse(osm, E0, N0)
    log(f'vorlaeufige Achse (OSM): {a0.laenge:.0f} m')
    # 1. Messung an der vorlaeufigen Achse: Mitte der Ring-Fahrbahn
    m0 = messen(gel, a0, np.arange(0, a0.laenge, 1.0))
    mitte = []
    for e in m0:
        r, l = e['w']['ring'], e['o']['ring']
        if r is not None and l is not None and 6.5 < r - l < 11.5:
            mitte.append(a0.versetzt([e['s']], [(r + l) / 2])[0])
    mitte = np.array(mitte)
    log(f'{len(mitte)} Querprofile mit beiden Ringkanten')
    # an den Enden (ohne Waende) OSM, um den mittleren Versatz OSM -> gemessene Mitte an diesem Ende verschoben
    a1, _ = achse_anpassen(mitte, toleranz=0.12, max_elemente=6)
    st_m = a1.projektion(mitte)
    st_o = a1.projektion(ring_osm)
    enden = []
    for ende, auswahl in (('Nord', st_o[:, 0] < st_m[:, 0].min()), ('Sued', st_o[:, 0] > st_m[:, 0].max())):
        nah = (np.abs(st_o[:, 0] - (st_m[:, 0].min() if ende == 'Nord' else st_m[:, 0].max())) < 40) & ~auswahl
        versatz = float(np.median(st_o[nah, 1])) if nah.any() else 0.0
        # OSM-Polygon fein abtasten (alle 2 m), nur bis 40 m ueber den Trog hinaus
        for p, q in zip(ring_osm[:-1], ring_osm[1:]):
            for f in np.linspace(0, 1, max(2, int(math.dist(p, q) / 2)), endpoint=False):
                pt = p + (q - p) * f
                sp, tp = a1.projektion([pt])[0]
                if (ende == 'Nord' and st_m[:, 0].min() - 45 < sp < st_m[:, 0].min() - 3) or (ende == 'Sued' and st_m[:, 0].max() + 3 < sp < st_m[:, 0].max() + 45):
                    d = q - p
                    n = np.array([d[1], -d[0]]) / np.hypot(*d)          # rechts der OSM-Richtung (Nord -> Sued)
                    enden.append(pt - n * versatz)
        log(f'Ende {ende}: OSM um {versatz:+.2f} m auf die gemessene Mitte geschoben')
    alle = np.array(sorted(list(mitte) + enden, key=lambda q: a1.projektion([q])[0, 0]))
    achse, rms = achse_anpassen(alle, toleranz=0.15, max_elemente=6)
    log(f'Achse: {len(achse.elemente)} Elemente, mittlere Abweichung {rms:.2f} m\n' + achse.text())
    # 2. Messung an der neuen Achse
    st = np.arange(0, achse.laenge, 1.0)
    m = messen(gel, achse, st)
    S = np.array([e['s'] for e in m])
    feld = lambda seite, k: np.array([np.nan if e[seite][k] is None else e[seite][k] for e in m])
    kanten = {}
    for seite in ('w', 'o'):
        for k in ('ring', 'rampe_aussen'):
            t = feld(seite, k)
            if np.sum(~np.isnan(t)) > 20:
                kanten[f'{k}_{seite}'], _ = ohne_ausreisser(S, t, lambda a, b: versatz_anpassen(a, b, toleranz=0.15), lambda v, x: v.t(x), 0.5)
    hoehen = {}
    zf = feld('w', 'zf')
    hoehen['ring'], _ = ohne_ausreisser(S, zf, gradiente_anpassen, lambda g, x: g.z(x), 0.4)
    for seite in ('w', 'o'):
        z = feld(seite, 'zr')
        # nur, wo die Ringkante auf der Wandlinie liegt (sonst wurde Gelaende statt Rampe gemessen)
        v = kanten.get(f'ring_{seite}')
        ok = ~np.isnan(z) & (np.abs(feld(seite, 'ring') - v.t(S)) < 0.6 if v is not None else ~np.isnan(z))
        if ok.sum() > 20:
            hoehen[f'rampe_{seite}'], _ = ohne_ausreisser(S[ok], z[ok], gradiente_anpassen, lambda g, x: g.z(x), 0.4)
    for k, v in kanten.items():
        log(f'{k}: ' + ', '.join(f'{a:.0f}-{b:.0f} m: {t:+.2f}' for a, b, t in v.a))
    for k, g in hoehen.items():
        log(f'Gradiente {k}:\n{g.text()}')
    return dict(achse=achse, rms=rms, messung=m, kanten=kanten, hoehen=hoehen, osm=ring_osm, mitte=mitte, achse0=a0)


# ------------------------------------------------------------------ Pruefbilder
def bilder(e, gel, dop, aus):
    import matplotlib
    matplotlib.use('Agg')
    import matplotlib.pyplot as plt
    from PIL import Image
    os.makedirs(aus, exist_ok=True)
    a = e['achse']
    x0, x1, y0, y1 = AUSSCHNITT
    S = np.array([m['s'] for m in e['messung']])
    s_all = np.arange(-30, a.laenge + 30, 0.5)
    farben = {'ring_w': '#ff3b30', 'ring_o': '#ff3b30', 'rampe_aussen_w': '#ffd60a', 'rampe_aussen_o': '#ffd60a'}
    # Grundriss: drei Ausschnitte (Nord, Kreuzung, Sued) und Uebersicht
    for name, (cx, cy, r) in {'grundriss': ((x0 + x1) / 2, (y0 + y1) / 2, (y1 - y0) / 2),
                              'grundriss_nord': (1080, -190, 45), 'grundriss_kreuzung': (1080, -266, 40),
                              'grundriss_sued': (1075, -350, 45)}.items():
        fig, ax = plt.subplots(figsize=(9, 9) if r > 100 else (10, 10), dpi=110)
        if dop is not None:
            ax.imshow(dop, extent=(x0, x1, y0, y1), origin='upper')
        ax.plot(e['osm'][:, 0], e['osm'][:, 1], '--', color='white', lw=0.8, label='OSM Ring')
        A = a.punkte(s_all)
        ax.plot(A[:, 0], A[:, 1], '-', color='#00e5ff', lw=1.2, label='Achse (Entwurf)')
        # Elementgrenzen
        s = 0
        for l, k in a.elemente:
            p = a.punkt(s)
            ax.plot(p[0], p[1], 'o', color='#00e5ff', ms=4)
            s += l
        for k, v in e['kanten'].items():
            t = v.t(s_all)
            P = a.versetzt(s_all, np.nan_to_num(t))
            P[np.isnan(t)] = np.nan
            ax.plot(P[:, 0], P[:, 1], '-', color=farben.get(k, 'w'), lw=1.0)
        # Messpunkte
        for seite in ('w', 'o'):
            for k, c in (('ring', '#ff9f0a'), ('rampe_aussen', '#bf5af2')):
                pts = [(m['s'], m[seite][k]) for m in e['messung'] if m[seite][k] is not None]
                if pts:
                    pts = np.array(pts)
                    P = a.versetzt(pts[:, 0], pts[:, 1])
                    ax.plot(P[:, 0], P[:, 1], '.', color=c, ms=1.5 if r > 100 else 3)
        ax.set_xlim(cx - r * 0.6 if r > 100 else cx - r, cx + r * 0.6 if r > 100 else cx + r)
        ax.set_ylim(cy - r, cy + r)
        ax.set_aspect('equal')
        ax.set_title(f'{name}: Achse (cyan), Ringkanten (rot), Rampen aussen (gelb), Messpunkte (orange/lila)', fontsize=8)
        fig.tight_layout()
        fig.savefig(os.path.join(aus, f'{name}.png'))
        plt.close(fig)
    # Laengsschnitt
    fig, axs = plt.subplots(3, 1, figsize=(14, 10), dpi=100, sharex=True)
    for ax, (key, seite, feld, titel) in zip(axs, [('ring', 'w', 'zf', 'Ring (Sohle)'), ('rampe_w', 'w', 'zr', 'Rampe West'), ('rampe_o', 'o', 'zr', 'Rampe Ost')]):
        z = np.array([np.nan if m[seite][feld] is None else m[seite][feld] for m in e['messung']])
        ax.plot(S, z, '.', ms=2, color='#888', label='Messung (DGM1)')
        g = e['hoehen'].get(key)
        if g is not None:
            sg = s_all[(s_all >= g.k[0]) & (s_all <= g.k[-1])]
            ax.plot(sg, g.z(sg), '-', color='#d00', lw=1.2, label='Gradiente')
            for k_, v_ in zip(g.k, g.v):
                ax.plot(k_, v_, 'o', color='#d00', ms=3)
            gr = g.neigungen()
            for i in range(len(gr)):
                ax.text((g.k[i] + g.k[i + 1]) / 2, (g.v[i] + g.v[i + 1]) / 2 + 0.4, f'{gr[i] * 100:+.1f} %', fontsize=8, ha='center')
        y = np.array([a.punkt(s)[1] for s in S])
        knoten = (y > DECKE[0] - 5) & (y < DECKE[1] + 5)
        if knoten.any():
            ax.axvspan(S[knoten].min(), S[knoten].max(), color='#ccf', alpha=0.4, label='Unterfuehrung/Kreuzung')
        ax.set_ylabel('Hoehe (m, Karte)')
        ax.set_title(titel, fontsize=9)
        ax.grid(alpha=0.3)
        ax.legend(fontsize=7, loc='upper right')
    axs[-1].set_xlabel('Station s (m, Achse Nord -> Sued)')
    fig.tight_layout()
    fig.savefig(os.path.join(aus, 'laengsschnitt.png'))
    plt.close(fig)


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--karte', required=True)
    ap.add_argument('--osm', required=True)
    ap.add_argument('--aus', required=True)
    ap.add_argument('--cache', default=os.path.join(os.environ.get('LOCALAPPDATA', '.'), 'omsi-editor', 'geodaten'))
    a = ap.parse_args()
    geo = {}
    with open(os.path.join(a.karte, 'omsi-editor-geo.cfg'), encoding='utf-8') as f:
        for z in f:
            if '=' in z and not z.startswith('#'):
                k, v = z.split('=', 1)
                geo[k.strip()] = v.strip()
    E0, N0, nn0 = float(geo['ost0']), float(geo['nord0']), float(geo['nn0'])
    x0, x1, y0, y1 = AUSSCHNITT
    gel = Gelaende.laden(Dgm1(a.cache), E0, N0, nn0, x0, x1, y0, y1)
    from PIL import Image
    seite = max(x1 - x0, y1 - y0)
    bild = luftbild(32, E0 + x0, N0 + y0, E0 + x0 + seite, N0 + y0 + seite, int(seite / 0.4), a.cache)
    dop = None
    if bild:
        img = Image.open(io.BytesIO(bild)).convert('RGB')
        px = img.width / seite
        # Bild: oberste Zeile = Norden (y0 + seite); gebraucht wird y0..y1, x0..x1
        dop = np.asarray(img)[int((seite - (y1 - y0)) * px):, :int((x1 - x0) * px)]
    e = entwerfen(gel, a.osm, E0, N0)
    bilder(e, gel, dop, a.aus)
    out = dict(achse=e['achse'].json(), kanten={k: v.json() for k, v in e['kanten'].items()},
               hoehen={k: g.json() for k, g in e['hoehen'].items()}, mittlere_abweichung_achse=e['rms'],
               quellen=['OpenStreetMap (ODbL)', 'DGM1/DOP40 Bayerische Vermessungsverwaltung (CC BY 4.0)'])
    with open(os.path.join(a.aus, 'entwurf.json'), 'w', encoding='utf-8') as f:
        json.dump(out, f, indent=1)


if __name__ == '__main__':
    sys.exit(main())
