"""Ampeln an Kreuzungen: Phasenplan, Bindung der Zufahrten und Signalmasten - nach dem Vorbild der Standardkarten.

Aufbau wie Grundorf (Kreuz_See_Elsflether): im Kreuzungsobjekt [traffic_lights_group] <Umlauf s>, je Signalgruppe
[traffic_light] <Name> mit [phase] <Zustand> <Sekunden> (nacheinander ab Umlaufbeginn; die letzte Phase "0 0" =
rot bis zum Ende des Umlaufs). Zustaende: 0 rot, 3 rot-gelb, 6 gruen, 9 gelb (so lesen es die Lampenskripte;
openOMSI traffic.rs aspect()). Gebunden wird das erste Stueck jeder Zufahrt ([use_traffic_light] nach dem [path]).
Signale: Verkehrszeichen_MC\\Ampel_Kfz_1.sco (2,8 m hoch) auf Streetobjects_RUE\\whip_beam.sco, Text = Index der
Signalgruppe, [varparent] = ID des Kreuzungsobjekts.

Zeiten nach der Kartenstudie (Umlauf meist 60-90 s): je Phase 2 s rot-gelb, gruen, 3 s gelb, 3 s alles rot;
die Hauptrichtung bekommt das 1,5-fache der Gruenzeit."""
import math
from .geom import rvec, dvec

SIGNAL = 'Sceneryobjects\\Verkehrszeichen_MC\\Ampel_Kfz_1.sco'
MAST = 'Sceneryobjects\\Streetobjects_RUE\\whip_beam.sco'
OBEN = 'Sceneryobjects\\Verkehrszeichen_MC\\Ampel_Kfz_Oh_1.sco'   # haengt am Ausleger des Masts (Anhaengepunkt 0)
OBEN_DREHUNG = 2.98373405277684                                     # wie in Grundorf
SIGNAL_HOEHE, MAST_HOEHE, MAST_VERSATZ = 2.8, 0.25, 0.46     # gemessen an Grundorf
ROT_GELB, GELB, RAEUMEN = 2.0, 3.0, 3.0
UMLAUF_2, UMLAUF_PRO_PHASE = 70.0, 15.0                     # Umlauf: 70 s bei 2 Phasen, +15 s je weitere


def _gegenueber(arms, i, kandidaten):
    """gegenueberliegender Arm (Winkel > 140 Grad) zu i oder None"""
    best = None
    for j in kandidaten:
        ang = abs((arms[i]['h'] - arms[j]['h'] + 180) % 360 - 180)
        if j != i and ang > 140 and (best is None or ang > best[0]):
            best = (ang, j)
    return best[1] if best else None


def gruppen(arms, rollen):
    """Arme zu Phasen zusammenfassen -> Liste Phase je Arm (0 = zuerst). Gemeinsam Gruen haben nur
    gegenueberliegende Arme (Winkel > 140 Grad); das durchlaufende Paar der Hauptstrasse kommt zuerst."""
    n = len(arms)
    phase = [None] * n
    reihenfolge = sorted(range(n), key=lambda i: (not (rollen and rollen[i] == 'haupt'), i))
    k = 0
    for i in reihenfolge:
        if phase[i] is not None:
            continue
        phase[i] = k
        frei = [j for j in reihenfolge if phase[j] is None]
        # Partner: bevorzugt ein Arm gleicher Rolle (durchgehende Hauptstrasse), sonst irgendein gegenueberliegender
        gleich = [j for j in frei if not rollen or rollen[j] == rollen[i]]
        j = _gegenueber(arms, i, gleich)
        if j is None:
            j = _gegenueber(arms, i, frei)
        if j is not None:
            phase[j] = k
        k += 1
    return phase


def programm(n_phasen, haupt_zuerst=True):
    """-> (Umlauf s, [Liste (Zustand, Sekunden) je Phase])"""
    umlauf = min(100.0, UMLAUF_2 + UMLAUF_PRO_PHASE * max(0, n_phasen - 2))
    zwischen = ROT_GELB + GELB + RAEUMEN
    gewichte = [1.5 if (i == 0 and haupt_zuerst and n_phasen > 1) else 1.0 for i in range(n_phasen)]
    gruen_gesamt = umlauf - zwischen * n_phasen
    out, t = [], 0.0
    for i, w in enumerate(gewichte):
        g = round(gruen_gesamt * w / sum(gewichte))
        if i == len(gewichte) - 1:                       # Rest, damit der Umlauf genau stimmt
            g = round(umlauf - t - zwischen)
        ph = ([(0, t)] if t > 0 else []) + [(3, ROT_GELB), (6, float(g)), (9, GELB), (0, 0.0)]
        out.append(ph)
        t += ROT_GELB + g + GELB + RAEUMEN
    return round(t), out


def plane(arms, rollen):
    """-> dict(phase je Arm, umlauf, signalgruppen=[dict(name, phasen)])"""
    ph = gruppen(arms, rollen)
    n = max(ph) + 1
    umlauf, progs = programm(n, haupt_zuerst=bool(rollen) and 'haupt' in rollen and 'neben' in rollen)
    namen = []
    for k in range(n):
        str_ = sorted({arms[i]['name'] for i in range(len(arms)) if ph[i] == k and arms[i]['name']})
        namen.append(f'Phase{k + 1}_' + ('_'.join(s.replace(' ', '') for s in str_)[:40] or 'Zufahrt'))
    return dict(phase=ph, umlauf=umlauf, signalgruppen=[dict(name=nm, phasen=p) for nm, p in zip(namen, progs)])


def sco_block(plan):
    """Zeilen fuer die .sco (vor den Pfaden)"""
    L = ['[traffic_lights_group]', f"{plan['umlauf']:.1f}", '']
    for g in plan['signalgruppen']:
        L += ['[traffic_light]', g['name'], '']
        for z, s in g['phasen']:
            L += ['[phase]', str(z), f'{s:.1f}', '']
    return L


def signale(arms, plan, sdb):
    """Signalmasten je Zufahrt mit ankommenden Fahrspuren: rechts der Zufahrt am Bordstein, Blick nach aussen.
    -> Liste dict(art, datei, ...): 'signal' und 'mast' mit x, z, hoehe, rot (Welt); 'oben' = Signal am Ausleger
    (eltern = Index des Masts in der Liste, anhang = Anhaengepunkt)"""
    out = []
    for i, a in enumerate(arms):
        p = sdb[a['spl']]
        incoming = [x for x, d in p['lanes'] if (d == 1) == a['away']]
        if not incoming:
            continue
        # ankommender Verkehr faehrt Richtung h+180; sein rechter Bordstein liegt im Aussen-Rahmen links
        rand = (-p['cl'] if a['away'] else p['cr'])
        off = -(rand + 0.8)
        rv, dv = rvec(a['h']), dvec(a['h'])
        sx, sz = a['pos'][0] + off * rv[0], a['pos'][1] + off * rv[1]
        rot = a['h'] % 360
        g = plan['phase'][i]
        out.append(dict(art='signal', datei=SIGNAL, x=sx, z=sz, hoehe=SIGNAL_HOEHE, rot=rot, gruppe=g))
        out.append(dict(art='mast', datei=MAST, x=sx - MAST_VERSATZ * dv[0], z=sz - MAST_VERSATZ * dv[1],
                        hoehe=MAST_HOEHE, rot=(rot + 180) % 360, gruppe=None))
        out.append(dict(art='oben', datei=OBEN, eltern=len(out) - 1, anhang=0, rot=OBEN_DREHUNG, gruppe=g))
    return out
