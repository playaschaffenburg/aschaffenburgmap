"""Haltestellen (Qualitaetsplan G4, ohne Fahrplan): echte Haltestellen aus OSM public_transport als Teil der Schicht
objekte (omsigen.ausstattung).

Je Steig (public_transport=platform bzw. highway=bus_stop mit Namen): Kante und Seite aus netz.json (die Seite, auf der
der Steig liegt; der Bus faehrt rechts), Haltepunkt = der OMSI-Haltestellenwuerfel gleichen Namens
(Sceneryobjects\\Generic\\bus_stop.sco, den der Generator setzt und den KI und Spielerbus anfahren - er bleibt unveraendert,
damit Linien, Einsetzpunkte und spaetere Fahrplaene ihre Objekt-IDs behalten), sonst die Lage des Steigs. Daraus:
  - Mast mit Zeichen 224 und Namensschild (eigenes Objekt, Schrift exakt aus der Schriftart) am Haltepunkt, 0,5 m vom Bord
  - Wartehalle hinter dem Mast (shelter=yes), offen zur Fahrbahn, 1,2 m vom Bord - sonst schmale Halle, sonst Fund
  - Bank (bench=yes, ohne Halle) und Muelleimer (bin=yes)
  - erhoehter Bord (kerb=raised bzw. kerb:height ueber 0,15 m) auf dem Gehweg, 16 m vor dem Mast, mit Leitstreifen bei
    tactile_paving=yes
Bucht/Kap (bus_bay) wird vermerkt; die Fahrbahn selbst baut die Schicht netz (Qualitaetsplan S3).
"""
import collections
import json
import math
import os

from . import schichten
from .ausstattung import SEITENRAUM, KACHEL, objekt, seitenraum, hat_fahrtrichtung

WUERFEL = 'bus_stop.sco'


def wuerfel(kartenordner):
    """OMSI-Haltestellenwuerfel der Karte -> [dict(name, x, z, id)]"""
    out = []
    for k in schichten.kacheln(kartenordner):
        _, bl = schichten.bloecke(schichten.lesen(kartenordner, k))
        for b in bl:
            if b[0].strip() == '[object]' and schichten.objekt_pfad(b).lower().endswith(WUERFEL) and len(b) > 11:
                try:
                    out.append(dict(name=b[11].strip(), x=float(b[4]) + k[0] * KACHEL, z=float(b[5]) + k[1] * KACHEL,
                                    id=int(float(b[3]))))
                except ValueError:
                    pass
    return out


def steige(osm):
    """Bussteige aus OSM -> [dict(name, x, z, tags, osm)]: Knoten platform/bus_stop und Steigflaechen (Mitte)"""
    out = []
    for k in (osm or {}).get('knoten', []):
        t = k['tags']
        if not t.get('name') or t.get('public_transport') == 'stop_position':
            continue
        if t.get('highway') == 'bus_stop' or (t.get('public_transport') == 'platform' and t.get('bus') != 'no'
                                              and t.get('railway') != 'platform' and t.get('train') != 'yes'):
            out.append(dict(name=t['name'], x=k['x'], z=k['z'], tags=t, osm=f'n{k["id"]}'))
    for w in (osm or {}).get('wege', []):
        t = w['tags']
        if not t.get('name') or not (t.get('public_transport') == 'platform' or t.get('highway') == 'platform'):
            continue
        if t.get('railway') == 'platform' or t.get('train') == 'yes' or t.get('tram') == 'yes' and t.get('bus') != 'yes':
            continue
        P = w['punkte']
        if not P:
            continue
        x, z = sum(p[0] for p in P) / len(P), sum(p[1] for p in P) / len(P)
        if any(s['name'] == t['name'] and math.hypot(s['x'] - x, s['z'] - z) < 25 for s in out):
            continue                                  # dieselbe Haltestelle als Knoten schon da
        out.append(dict(name=t['name'], x=x, z=z, tags=t, osm=f'w{w["id"]}'))
    return out


def bordhoehe(t, regeln):
    """Bordhoehe aus kerb:height bzw. kerb (raised = Sonderbord) -> m oder None (Standardbord)"""
    try:
        h = float(str(t.get('kerb:height', '')).replace('m', '').replace(',', '.'))
        return h
    except ValueError:
        pass
    if t.get('kerb') == 'raised':
        return regeln.get('bord_raised_m', 0.18)
    return None


def planen(platz, auswahl, regeln_alle, osm, kartenordner):
    """Haltestellen setzen -> (Liste je Steig fuer ausstattung.json, Zahlen)"""
    R = regeln_alle.get('haltestelle', {})
    netz = platz.netz
    n = collections.Counter()
    alle_wuerfel = wuerfel(kartenordner)
    liste = []
    gesehen = []
    for sg in sorted(steige(osm), key=lambda s: (s['name'], s['x'], s['z'])):
        t = sg['tags']
        nah = netz.naechste(sg['x'], sg['z'], min_laenge=8.0)
        if not nah or nah[3] > R.get('osm_umkreis_m', 30.0):
            n['ohne_strasse'] += 1
            continue
        eid, st, q, _ = nah
        e = netz.kanten[eid]
        bord, aussen = seitenraum(e, 1 if q >= 0 else -1)
        # Wuerfel gleichen Namens in der Naehe: er bestimmt den Haltepunkt (und die Seite, wenn der Steig auf der
        # Fahrbahn eingetragen ist)
        wf = min((w for w in alle_wuerfel if w['name'] == sg['name']),
                 key=lambda w: math.hypot(w['x'] - sg['x'], w['z'] - sg['z']), default=None)
        if wf and math.hypot(wf['x'] - sg['x'], wf['z'] - sg['z']) > 40:
            wf = None
        if abs(q) > bord - 0.3:
            s = 1 if q >= 0 else -1
        elif wf:
            nw = netz.naechste(wf['x'], wf['z'])
            s = 1 if (nw and nw[0] == eid and nw[2] >= 0) else -1
        else:
            s = 1 if hat_fahrtrichtung(e, True) else -1
        st_halt = st
        if wf:
            nw = netz.naechste(wf['x'], wf['z'])
            if nw and nw[0] == eid and (nw[2] >= 0) == (s > 0):
                st_halt = nw[1]
        if any(g[0] == sg['name'] and g[1] == eid and g[2] == s and abs(g[3] - st_halt) < 20 for g in gesehen):
            n['doppelt'] += 1
            continue
        gesehen.append((sg['name'], eid, s, st_halt))
        L = e['laenge']
        tr = s                                          # Fahrtrichtung entlang der Kante (+1 mit, -1 gegen)
        bord, aussen = seitenraum(e, s)
        info = dict(name=sg['name'], osm=sg['osm'], kante=eid, seite=s, station=round(st_halt, 1),
                    osm_wartehalle=t.get('shelter') == 'yes', osm_bank=t.get('bench') == 'yes',
                    osm_muelleimer=t.get('bin') == 'yes', taktil=t.get('tactile_paving') == 'yes',
                    bucht=t.get('bus_bay'), bord_m=bordhoehe(t, R), wuerfel_m=None, mast=False, wartehalle=None,
                    bank=False, muelleimer=False, hochbord=False, gehweg_m=round(aussen - bord, 2))
        if wf:
            info['wuerfel_m'] = round(math.hypot(wf['x'] - sg['x'], wf['z'] - sg['z']), 1)
        grp = f'H{len(liste)}'
        # Mast
        mast = None
        if 'haltestelle_mast' in auswahl:
            for d in (0, 1, -1, 2, -2, 3, -3):
                stt = min(max(st_halt + d, 0.5), L - 0.5)
                p = platz.am_rand(eid, stt, s, R.get('mast_bord_m', 0.5), 0.05)
                if p and platz.frei(p[0], p[1], 0.1):
                    mast = (stt, p)
                    break
        if mast:
            stt, (x, z, y, h, qq) = mast
            fahrt = h if s > 0 else (h + 180.0) % 360.0
            platz.setzen(objekt('haltestelle_mast', x, z, platz.hoehe_gehweg(x, z, y), fahrt, 'osm', name=sg['name'],
                                eigen_schluessel=sg['name'], kante=eid, station=round(stt, 1), seite=s,
                                osm=sg['osm'], gruppe=grp, radius=0.1))
            info.update(mast=True, x=round(x, 1), z=round(z, 1))
            st_halt = stt
        else:
            x, _, z, _ = netz.punkt(eid, st_halt, s * (bord + 0.5))
            info.update(x=round(x, 1), z=round(z, 1), grund='kein Platz fuer den Mast')
            n['ohne_mast'] += 1
        # Wartehalle
        if info['osm_wartehalle']:
            for typ in ('wartehalle', 'wartehalle_schmal'):
                if typ not in auswahl or info['wartehalle']:
                    continue
                m = auswahl.masse.get(typ) or {}
                box = m.get('box') or [-2.2, 0, -1.6, 2.2, 2.4, 0.2]
                vorne = auswahl[typ].get('vorne', 180)
                # Tiefe zur Fahrbahn hin (Vorderseite) und nach hinten
                vor = -box[2] if vorne == 180 else box[5] if vorne == 0 else (-box[0] if vorne == 270 else box[3])
                for halle_bord in (R.get('halle_bord_m', 1.2), 0.8):
                    if info['wartehalle']:
                        break
                    for d in (0, -2, 2, -4, 4, -6, 6):
                        stt = min(max(st_halt + tr * R.get('halle_versatz_m', -6.0) + d, 2.5), L - 2.5)
                        qo = bord + halle_bord + vor
                        x, y, z, h = netz.punkt(eid, stt, s * qo)
                        zur = (h - 90.0) if s > 0 else (h + 90.0)
                        rot = zur - vorne
                        if not platz.rechteck_ok(x, z, rot, box):
                            continue
                        platz.setzen(objekt(typ, x, z, platz.hoehe_gehweg(x, z, y), rot, 'osm', name=sg['name'],
                                            kante=eid, station=round(stt, 1), seite=s, osm=sg['osm'], gruppe=grp,
                                            radius=max(box[3] - box[0], box[5] - box[2]) / 2))
                        info['wartehalle'] = typ
                        break
            if not info['wartehalle']:
                info['grund'] = f'Seitenraum {info["gehweg_m"]} m, Belegung/Gebaeude lassen keine Halle zu'
                n['wartehalle_kein_platz'] += 1
        # Bank (ohne Halle) und Muelleimer
        for typ, will, versatz, abst in (('bank', info['osm_bank'] and not info['wartehalle'], -4.0, 1.2),
                                         ('muelleimer', info['osm_muelleimer'], -1.5, 0.6)):
            if not will or typ not in auswahl:
                continue
            r = auswahl.radius(typ)
            for d in (0, -1, 1, -2, 2):
                stt = min(max(st_halt + tr * versatz + d, 0.5), L - 0.5)
                p = platz.am_rand(eid, stt, s, abst, r)
                if p and platz.frei(p[0], p[1], r):
                    x, z, y, h, qq = p
                    zur = (h - 90.0) if s > 0 else (h + 90.0)
                    platz.setzen(objekt(typ, x, z, platz.hoehe_gehweg(x, z, y), zur - auswahl[typ].get('vorne', 0),
                                        'osm', name=sg['name'], kante=eid, station=round(stt, 1), seite=s,
                                        osm=sg['osm'], gruppe=grp, radius=r))
                    info[typ] = True
                    break
        # erhoehter Bord
        bh = info['bord_m']
        if bh and bh > R.get('bord_standard_m', 0.15) + 0.02 and 'hochbord' in auswahl and aussen - bord >= 0.8:
            a, b = sorted((st_halt - tr * 16.0, st_halt + tr * 1.0))
            a, b = max(a, 0.5), min(b, L - 0.5)
            if b - a >= 6.0:
                mitte = (a + b) / 2
                breite = min(1.5, aussen - bord)
                ox, oy, oz, _ = netz.punkt(eid, mitte, s * (bord + breite / 2))     # Ursprung auf dem Gehweg
                pk = []
                anz = max(2, int((b - a) / 2.0) + 1)
                for i in range(anz):
                    sx = a + (b - a) * i / (anz - 1)
                    px, py, pz, ph = netz.punkt(eid, sx, s * bord)
                    rr = math.radians(ph)
                    rx, rz = math.cos(rr) * s, -math.sin(rr) * s      # nach aussen
                    pk.append([round(px - ox, 3), round(py - oy, 3), round(pz - oz, 3), [round(rx, 4), round(rz, 4)]])
                # [absheight]: Hoehe absolut = Gehwegoberkante (Spline + 0,25 m), unabhaengig vom Gelaende darunter
                platz.setzen(objekt('hochbord', ox, oz, round(oy + 0.25, 3), 0.0, 'osm', name=sg['name'],
                                    eigen_schluessel=f'{sg["name"]}|{eid}|{s}|{round(mitte)}', kante=eid,
                                    station=round(mitte, 1), seite=s, osm=sg['osm'], gruppe=grp, radius=0.0,
                                    bord=dict(punkte=pk, breite=breite, hoehe=round(bh - 0.15, 3),
                                              taktil=info['taktil'])))
                info['hochbord'] = True
        n['steige'] += 1
        n['mit_mast'] += info['mast']
        n['osm_wartehalle'] += info['osm_wartehalle']
        n['mit_wartehalle'] += bool(info['wartehalle'])
        n['bank'] += info['bank']
        n['muelleimer'] += info['muelleimer']
        n['hochbord'] += info['hochbord']
        n['mit_wuerfel'] += info['wuerfel_m'] is not None
        n['bucht_vermerkt'] += bool(info['bucht'])
        liste.append(info)
    return liste, dict(n)


def bord_schreiben(ordner, o):
    """erhoehten Bord einer Haltestelle als eigenes Objekt schreiben -> Dateiname der .sco"""
    from . import ausstattung_eigen as eigen
    b = o['bord']
    import hashlib
    nr = hashlib.sha1(o['eigen_schluessel'].encode('utf-8')).hexdigest()[:6]
    kurz = f'B_{eigen.kurzname(o["name"])}_{nr}'
    m = eigen.bord_modell([tuple(p[:3]) + (tuple(p[3]),) for p in b['punkte']], 0.0, b['breite'], b['hoehe'], b['taktil'])
    eigen.dds_schreiben(eigen.bord_textur(b['taktil']), os.path.join(ordner, 'texture', kurz + '.dds'))
    os.makedirs(os.path.join(ordner, 'model'), exist_ok=True)
    with open(os.path.join(ordner, 'model', kurz + '.x'), 'w', encoding='cp1252', newline='') as f:
        f.write(m.x_text(kurz + '.dds'))
    with open(os.path.join(ordner, kurz + '.sco'), 'w', encoding='cp1252', errors='replace', newline='') as f:
        f.write(eigen.sco_text(f'Bord Haltestelle {o["name"]}', kurz + '.x', 'Bordhoehe: OpenStreetMap kerb:height.',
                               absolut=True))
    return kurz + '.sco'
