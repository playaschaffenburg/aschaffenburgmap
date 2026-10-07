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
     "kreisel": {"mitte": [x, y], "r": Radius der Ringspur, "breite": Breite der Ringfahrbahn} fuer einen
                Kreisverkehr als ein Objekt (kreisel.py; die Arme sind seine Zufahrten, ab einer)}
Ergebnis:
    {"rel": .sco relativ zu OMSI, "ursprung": [x, y], "rules": [[Pfad, Prioritaet], ...], "pfade": n,
     "spuren": Abbiegespuren, "fehlgeschlagen": n, "dreiecke": n,
     "signale": [{"art": "signal"/"mast"/"oben", "datei", "x", "y", "hoehe", "rot", "gruppe", "eltern", "anhang"}],
     "phasen": [Phase je Arm], "umlauf": s}
oder {"fehler": Text}."""
import json, os, shutil, sys
from . import ampel, kreisel, kreuzung, vorfahrt
from .network import spurenden, kreuzungsspuren
from .splinedb import SplineDB


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
    arms = [dict(pos=tuple(a['pos']), h=float(a['h']) % 360, spl=a['sli'], away=bool(a['away'])) for a in auftrag['arme']]
    K = kreisel.bauen(arms, k['mitte'], float(k['r']), float(k['breite']), sdb)
    name = auftrag['name']
    x, dreiecke = kreisel.x_file(K)
    _schreiben(auftrag, name, kreisel.sco_text(auftrag.get('titel') or name, name + '.x', K), x)
    pfade = sum(len(m[0]) for m in K['moves'])
    return dict(rel=auftrag['rel_ordner'].rstrip('\\') + '\\' + name + '.sco', ursprung=list(K['origin']),
                rules=K['rules'], pfade=pfade + sum(len(w) for w in K['walks']), spuren=len(K['moves']),
                fehlgeschlagen=0, bewegungen={}, dreiecke=dreiecke, signale=[], phasen=[], umlauf=None)


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
    ketten, fehl, zaehler = kreuzungsspuren(spuren, 0)
    J = kreuzung.build_junction(arms, sdb)
    if J is None or J['asphalt'].is_empty:
        raise ValueError('aus diesen Armen entsteht keine Kreuzungsflaeche')
    O = J['origin']
    rollen = [a.get('rolle') or vorfahrt.GLEICH for a in arme]
    n_haupt = rollen.count(vorfahrt.HAUPT)
    plan = ampel.plane(arms, rollen) if auftrag.get('ampel') else None
    moves, rules, idx = [], [], 0
    for c in ketten:
        els = [[e[0] - O[0], e[1] - O[1]] + list(e[2:5]) for e in c['els']]
        gruppe = plan['phase'][c['arms'][0]] if (plan and c.get('arms')) else None
        moves.append((els, kreuzung.BLINKER.get(c.get('mv'), 0), gruppe))
        if c.get('arms'):
            ia, ib = c['arms']
            pr = vorfahrt.priority(rollen[ia], rollen[ib], c.get('mv'), n_haupt)
            if pr is not None:
                rules += [(idx + j, pr) for j in range(len(els))]
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
                phasen=plan['phase'] if plan else [], umlauf=plan['umlauf'] if plan else None)


def main():
    try:
        erg = bauen(json.load(sys.stdin))
    except Exception as e:          # der Editor zeigt den Text in der Statuszeile
        erg = dict(fehler=f'{type(e).__name__}: {e}')
    json.dump(erg, sys.stdout)


if __name__ == '__main__':
    main()
