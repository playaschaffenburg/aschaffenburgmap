"""Kreuzungsobjekt fuer den Karteneditor (omsi-editor, Rust): eine Kreuzung aus gegebenen Armen bauen - dieselbe
Platte, dieselben Abbiegespuren und dieselbe Vorfahrt wie bei den von omsigen erzeugten Karten (kreuzung.py,
network.kreuzungsspuren, vorfahrt.py).

Aufruf: python -m omsigen.editorkreuzung < auftrag.json > ergebnis.json

Auftrag (Weltkoordinaten in Metern: x Ost, y Nord; Richtungen im Uhrzeigersinn ab Nord):
    {"omsi": OMSI-Ordner, "ordner": Zielordner (absolut), "rel_ordner": derselbe Ordner relativ zu OMSI,
     "name": Dateiname ohne Endung, "titel": friendlyname,
     "arme": [{"pos": [x, y], "h": Richtung von der Kreuzung weg, "sli": Spline des Arms,
               "away": true wenn die Splinerichtung von der Kreuzung weg zeigt, "rolle": "haupt"/"neben"/"gleich"}],
     "ampel": true fuer eine Ampelkreuzung (Phasenplan wie omsigen ampel.py),
     ein Arm mit "gesperrt": true ist fuer die KI gesperrt (die Pfade in ihn hinein bekommen no_cars),
     "verbindungen": [[Arm rein, Spur rein, Arm raus, Spur raus], ...] baut genau diese Abbiegespuren (Spurpfeile/
                Verbinder des Editors; Spuren je Arm von rechts gezaehlt, 0 = rechte), sonst wie omsigen,
     "kreisel": {"mitte": [x, y], "r": Radius der Ringspur, "breite": Breite der Ringfahrbahn} fuer einen
                Kreisverkehr als ein Objekt (kreisel.py; die Arme sind seine Zufahrten, ab einer)}
Ergebnis:
    {"rel": .sco relativ zu OMSI, "ursprung": [x, y], "rules": [[Pfad, Prioritaet], ...], "pfade": n,
     "spuren": Abbiegespuren, "fehlgeschlagen": n, "dreiecke": n,
     "signale": [{"art": "signal"/"mast"/"oben", "datei", "x", "y", "hoehe", "rot", "gruppe", "eltern", "anhang"}],
     "phasen": [Phase je Arm], "umlauf": s, "no_cars": [Pfad, ...],
     "verbindungen": [{"von": [Arm, Spur], "nach": [Arm, Spur], "mv": "gerade"/"rechts"/"links"/"wenden"/"nachlauf",
                       "pfade": [erster, Anzahl], "punkte": [[x, y], ...]}],
     "zufahrten": [{"rein": [[x, y], ...], "raus": [[x, y], ...]} je Arm, Spuren von rechts]}
oder {"fehler": Text}."""
import collections, json, math, os, shutil, sys
from . import ampel, kreisel, kreuzung, vorfahrt
from .config import KI
from .custom_splines import KREUZ_GAPS
from .geom import connect, end_of, norm180, sample
from .network import spurenden, kreuzungsspuren
from .splinedb import SplineDB


def _rang(spuren):
    """je Arm: (ankommende, abgehende Spuren), jeweils von rechts nach links (wie kreuzungsspuren)"""
    return [(sorted([l for l in A if l['kind'] == 'in'], key=lambda l: -l['right']),
             sorted([l for l in A if l['kind'] == 'out'], key=lambda l: -l['right'])) for A in spuren]


def _art(ia, ib, a, b):
    dlt = norm180(b['h'] - a['h'])
    if ia == ib or abs(dlt) > 150:
        return 'wenden'
    if abs(dlt) < 35:
        return 'gerade'
    return 'rechts' if dlt > 0 else 'links'


def eigene_spuren(spuren, verbindungen):
    """genau die gewuenschten Abbiegespuren [Arm rein, Spur rein, Arm raus, Spur raus] -> wie kreuzungsspuren"""
    R = _rang(spuren)
    ketten, fehl, zaehler = [], 0, collections.Counter()
    for ia, i, ib, j in verbindungen:
        if not (0 <= ia < len(R) and 0 <= ib < len(R)) or i >= len(R[ia][0]) or j >= len(R[ib][1]):
            continue
        a, b = R[ia][0][i], R[ib][1][j]
        mv = _art(ia, ib, a, b)
        spl = KI + 'AB_kreuzung_spur.sli'
        if mv == 'rechts' and len(R) >= 3:
            g = min(KREUZ_GAPS, key=lambda x: abs(x - (a['gap'] + b['gap']) / 2))
            spl = KI + f'AB_kreuzung_ecke_{str(g).replace(".", "_")}.sli'
        els = connect(a['p'], a['h'], b['p'], b['h'], spl)
        if not els or sum(e[3] for e in els) > 3 * math.dist(a['p'], b['p']) + 30:
            fehl += 1
            continue
        zaehler[mv] += 1
        ketten.append(dict(els=els, name='Kreuzung', hw='junction', mv=mv, arms=(ia, ib)))
    return ketten, fehl, zaehler


def _spur_von(R, p, art):
    """(Arm, Rang) der Spur, deren Ende bei p liegt"""
    for ia, (rein, raus) in enumerate(R):
        for i, l in enumerate(rein if art == 'in' else raus):
            if math.dist(l['p'], p) < 0.05:
                return [ia, i]
    return None


def _schreiben(auftrag, name, sco, x):
    d = auftrag['ordner']
    os.makedirs(os.path.join(d, 'model'), exist_ok=True)
    os.makedirs(os.path.join(d, 'texture'), exist_ok=True)
    with open(os.path.join(d, name + '.sco'), 'w', encoding='cp1252', newline='') as f:
        f.write(sco)
    with open(os.path.join(d, 'model', name + '.x'), 'w', encoding='ascii', newline='') as f:
        f.write(x)
    om = auftrag['omsi']
    for src in (os.path.join(om, 'Splines', 'Marcel', 'texture'), os.path.join(om, 'Texture')):
        for t in kreuzung.TEXTURES:
            if os.path.exists(os.path.join(src, t)) and not os.path.exists(os.path.join(d, 'texture', t)):
                shutil.copy(os.path.join(src, t), os.path.join(d, 'texture', t))


def kreisel_bauen(auftrag, sdb):
    k = auftrag['kreisel']
    arms = [dict(pos=tuple(a['pos']), h=float(a['h']) % 360, spl=a['sli'], away=bool(a['away']), gesperrt=bool(a.get('gesperrt')))
            for a in auftrag['arme']]
    K = kreisel.bauen(arms, k['mitte'], float(k['r']), float(k['breite']), sdb)
    name = auftrag['name']
    x, dreiecke = kreisel.x_file(K)
    _schreiben(auftrag, name, kreisel.sco_text(auftrag.get('titel') or name, name + '.x', K), x)
    pfade = sum(len(m[0]) for m in K['moves'])
    return dict(rel=auftrag['rel_ordner'].rstrip('\\') + '\\' + name + '.sco', ursprung=list(K['origin']),
                rules=K['rules'], pfade=pfade + sum(len(w) for w in K['walks']), spuren=len(K['moves']),
                fehlgeschlagen=0, bewegungen={}, dreiecke=dreiecke, signale=[], phasen=[], umlauf=None,
                no_cars=K['no_cars'])


def bauen(auftrag, sdb=None):
    sdb = sdb or SplineDB(auftrag['omsi'])
    if auftrag.get('kreisel'):
        return kreisel_bauen(auftrag, sdb)
    arme = auftrag['arme']
    if len(arme) < 3:
        raise ValueError('eine Kreuzung braucht mindestens 3 Arme')
    spuren, arms = [], []
    for a in arme:
        pos, h = tuple(a['pos']), float(a['h']) % 360
        if a['away']:
            spuren.append(spurenden(pos, h, a['sli'], 's', sdb))
        else:
            spuren.append(spurenden(pos, (h + 180) % 360, a['sli'], 'e', sdb))
        arms.append(dict(pos=pos, h=h, spl=a['sli'], away=bool(a['away']), name=a.get('name', '')))
    if auftrag.get('verbindungen') is not None:
        ketten, fehl, zaehler = eigene_spuren(spuren, auftrag['verbindungen'])
    else:
        ketten, fehl, zaehler = kreuzungsspuren(spuren, 0)
    R = _rang(spuren)
    J = kreuzung.build_junction(arms, sdb)
    if J is None or J['asphalt'].is_empty:
        raise ValueError('aus diesen Armen entsteht keine Kreuzungsflaeche')
    O = J['origin']
    rollen = [a.get('rolle') or vorfahrt.GLEICH for a in arme]
    n_haupt = rollen.count(vorfahrt.HAUPT)
    plan = ampel.plane(arms, rollen) if auftrag.get('ampel') else None
    moves, rules, idx = [], [], 0
    no_cars, verbindungen = [], []
    for c in ketten:
        von, nach = _spur_von(R, c['els'][0][:2], 'in'), _spur_von(R, end_of(c['els'][-1])[0], 'out')
        n = len(c['els'])
        verbindungen.append(dict(von=von, nach=nach, mv=c.get('mv'), pfade=[idx, n],
                                 punkte=[list(q) for e in c['els'] for q, _ in sample(e, 1.0)]))
        els = [[e[0] - O[0], e[1] - O[1]] + list(e[2:5]) for e in c['els']]
        gruppe = plan['phase'][c['arms'][0]] if (plan and c.get('arms')) else None
        moves.append((els, kreuzung.BLINKER.get(c.get('mv'), 0), gruppe))
        if c.get('arms'):
            ia, ib = c['arms']
            pr = vorfahrt.priority(rollen[ia], rollen[ib], 'links' if c.get('mv') == 'wenden' else c.get('mv'), n_haupt)
            if pr is not None:
                rules += [(idx + j, pr) for j in range(len(els))]
            if arme[ib].get('gesperrt'):
                no_cars += [idx + j for j in range(len(els))]
        idx += len(els)
    V, F = kreuzung.mesh(J)
    name = auftrag['name']
    sco = kreuzung.sco_text(auftrag.get('titel') or name, name + '.x', moves, J['walks'], ampel.sco_block(plan) if plan else ())
    _schreiben(auftrag, name, sco, kreuzung.x_file(V, F))
    return dict(rel=auftrag['rel_ordner'].rstrip('\\') + '\\' + name + '.sco', ursprung=[O[0], O[1]], rules=rules,
                pfade=idx + sum(len(w) for w in J['walks']), spuren=len(ketten), fehlgeschlagen=fehl,
                bewegungen=dict(zaehler), dreiecke=len(F),
                signale=[dict(art=g['art'], datei=g['datei'], x=g.get('x'), y=g.get('z'), hoehe=g.get('hoehe', 0.0),
                              rot=g['rot'], gruppe=g['gruppe'], eltern=g.get('eltern'), anhang=g.get('anhang'))
                         for g in (ampel.signale(arms, plan, sdb) if plan else [])],
                phasen=plan['phase'] if plan else [], umlauf=plan['umlauf'] if plan else None, no_cars=no_cars,
                verbindungen=verbindungen,
                zufahrten=[dict(rein=[list(l['p']) for l in rein], raus=[list(l['p']) for l in raus]) for rein, raus in R])


def main():
    try:
        erg = bauen(json.load(sys.stdin))
    except Exception as e:          # der Editor zeigt den Text in der Statuszeile
        erg = dict(fehler=f'{type(e).__name__}: {e}')
    json.dump(erg, sys.stdout)


if __name__ == '__main__':
    main()
