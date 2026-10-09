"""Die zwei Schritte von omsigen, gemeinsam fuer Kommandozeile und Editor:

  importiere()  OSM-Ort/Strecke -> Projekt (Strassen in Metern mit OSM-Tags, Haltestellen, Schilder, Rand)
  erzeuge()     Projekt -> fertige OMSI-Karte (Kreuzungsobjekte, Vorfahrt, Ampeln, Wendeschleifen) + Pruefung

Projekt (JSON, Endung .omsiprojekt): siehe neues_projekt(). Koordinaten in Metern um den Ursprung (x Ost, z Nord,
Projektion route.Projection)."""
import collections, json, math, os
from . import osm, ansicht, kreuzung, vorfahrt, bauwerke, ebenen, gelaende as gel_mod, hoehen as hoe_mod
from .route import Projection, route, corridor
from .network import build, proj_point
from .splinedb import SplineDB
from .writer import write_map, place_stops, install_splines, spur_punkt

VERSION = 1


def neues_projekt(lat0, lon0, name='Neues Projekt'):
    return dict(version=VERSION, name=name, ursprung=[lat0, lon0], strassen=[], haltestellen=[], schilder=[],
                rand=[], linie=[], quelle='', beschreibung='')


def laden(path):
    with open(path, encoding='utf-8') as f:
        p = json.load(f)
    if p.get('version', 0) > VERSION:
        raise ValueError(f'Projekt {path} ist von einer neueren Version ({p["version"]})')
    return p


def speichern(projekt, path):
    tmp = path + '.tmp'
    with open(tmp, 'w', encoding='utf-8') as f:
        json.dump(projekt, f, ensure_ascii=False, indent=1)
    os.replace(tmp, path)


def linie_probe(projekt):
    """einige Punkte des Projekts (fuer die Frage, ob es Gelaendedaten gibt)"""
    pts = [q for s in projekt['strassen'] for q in s['punkte']]
    return [tuple(pts[i]) for i in range(0, len(pts), max(1, len(pts) // 8))][:10]


def importiere(von, nach, stadt=None, ueber=(), breite=150.0, cache='.cache', osm_datei=None, log=print):
    """OSM-Strecke importieren -> Projekt"""
    log('1/6 Orte suchen ...')
    qs = [von] + list(ueber) + [nach]
    places = [osm.find_place(q, stadt, cache) for q in qs]
    pts = [(p['lat'], p['lon']) for p in places]
    for q, p in zip(qs, places):
        info = f'  ({p["name"]}, {p["art"]})' if p['name'] else ''
        log(f'    {q}: {p["lat"]:.6f}, {p["lon"]:.6f}{info}')

    log('2/6 OSM-Daten laden ...')
    lat = [p[0] for p in pts]; lon = [p[1] for p in pts]
    m = (breite + 400) / 111000
    bbox = (min(lat) - m, min(lon) - m / math.cos(math.radians(lat[0])), max(lat) + m,
            max(lon) + m / math.cos(math.radians(lat[0])))
    data = osm.load(osm_datei) if osm_datei else osm.fetch(bbox, cache)
    log(f'    {len(data["ways"])} Strassen, {len(data["stops"])} Haltestellen')

    proj = Projection(sum(lat) / len(lat), sum(lon) / len(lon))
    log('3/6 Strecke berechnen ...')
    rinfo = {}
    line, length = route(data['ways'], proj, pts, rinfo)
    log(f'    Streckenlaenge {length:.0f} m (Abstand zur Strasse: '
        + ', '.join(f'{d:.0f} m' for d in rinfo['andocken']) + ')')
    # Korridor bis zu den echten Start- und Zielpunkten, auch wenn die Strecke an der naechsten Strasse beginnt
    ziele = [proj.to_m(*p) for p in pts]
    ways, cuts = corridor(data['ways'], proj, [ziele[0]] + list(line) + [ziele[-1]], breite)
    stops = []
    for s in data['stops']:
        p = proj.to_m(s['lat'], s['lon'])
        if min(proj_point(p, a_, b_)[1] for a_, b_ in zip(line, line[1:])) <= breite:
            stops.append(dict(name=s['name'], p=list(p)))
    P = neues_projekt(proj.lat0, proj.lon0, f'{von} - {nach}')
    P.update(strassen=[dict(id=i + 1, tags=w['tags'], punkte=[list(q) for q in w['P']]) for i, w in enumerate(ways)],
             haltestellen=stops,
             schilder=[dict(kind=g['kind'], p=list(proj.to_m(g['lat'], g['lon'])), direction=g.get('direction'))
                       for g in data.get('signs', [])],
             rand=[list(c) for c in cuts], linie=[list(q) for q in line], laenge=length,
             ziele=[dict(name=q, p=list(z)) for q, z in zip(qs, ziele)],
             quelle='OpenStreetMap (ODbL)',
             beschreibung=f'{von} -> {nach}' + (f' ({stadt})' if stadt else '') + f', Korridor {breite:.0f} m')
    return P


def wege(projekt, gel=None):
    """Projektstrassen -> (Wege fuer network.build, Hoehenvorgaben). Strassen mit Hoehen aus dem Editor
    (strasse['hoehen'], siehe ebenen.py) werden automatisch in Boden/Bruecke/Tunnel geteilt."""
    ways, ziele = [], []
    for s in projekt['strassen']:
        if len(s['punkte']) < 2:
            continue
        P = [tuple(q) for q in s['punkte']]
        if ebenen.hat_vorgaben(s) and len(s['hoehen']) == len(P):
            ziele += ebenen.ziele(P, s['hoehen'], gel)
            if ebenen.automatik(s['tags']):
                for art, Q, _ in ebenen.einteilen(P, s['hoehen'], gel):
                    ways.append(dict(tags=ebenen.ebene_tags(s['tags'], art), P=[tuple(q) for q in Q], sid=s['id']))
                continue
        ways.append(dict(tags=s['tags'], P=P, sid=s['id']))
    return ways, ziele


def berechne(projekt, name='Vorschau', omsi=None, korrekturen=None, kreuzungen='objekt', gelaende=True, log=print,
             gel=None):
    """Projekt -> alles, was die Karte ausmacht, ohne Dateien zu schreiben (auch fuer die 3D-Vorschau im Editor):
    dict(proj, sdb, net, signs, stops, junctions, vorfahrt, hoe, hat_gelaende, bw, chains, objekte).
    gel: vorhandenes Gelaende-Objekt (Editor), sonst wird es angelegt."""
    proj = Projection(*projekt['ursprung'])
    if gelaende and gel is None:
        gel = gel_mod.Gelaende(proj, log=log)
    hat_gelaende = bool(gelaende and gel and any(gel.hoehe(*q) is not None for q in linie_probe(projekt)))
    if not hat_gelaende:
        gel = None
    ways, ziele = wege(projekt, gel)
    if not ways:
        raise ValueError('Das Projekt enthaelt keine Strassen')
    log('4/6 Strassennetz und Kreuzungen bauen ...')
    sdb = SplineDB(omsi)
    signs = [dict(kind=g['kind'], p=tuple(g['p']), direction=g.get('direction')) for g in projekt.get('schilder', [])]
    net = build(ways, sdb, signs)
    st = net['stats']
    log(f'    {len(net["road_chains"])} Strassenzuege, {st["kreuzungen"]} Kreuzungen, '
        f'{st["verbindungen"]} Kreuzungsspuren {st["bewegungen"]}')
    log(f'    {st["wendeschleifen"]} unsichtbare Wendeschleifen an Strassenenden'
        + (f', {st["enden_ohne_wende"]} Einbahn-Enden ohne Gegenspur' if st['enden_ohne_wende'] else ''))
    stops = place_stops([(s['name'], tuple(s['p'])) for s in projekt.get('haltestellen', [])], net['road_chains'], sdb)
    chains, junctions, V = net['road_chains'] + net['wenden'], [], []
    if kreuzungen == 'objekt':
        junctions = kreuzung.build_objects(net, sdb, name, vorfahrt.load_corrections(korrekturen) if
                                           isinstance(korrekturen, str) else (korrekturen or []), proj.to_ll)
        V = [j['vorfahrt'] for j in junctions if j['vorfahrt']]
        q = collections.Counter(v['quelle'] for v in V)
        log(f'    Vorfahrt: {len(signs)} Schilder/Ampeln aus OSM; Quelle je Kreuzung: ' +
            ', '.join(f'{k} {n}' for k, n in q.most_common()) + f" (davon mit Ampel {sum(v['ampel'] for v in V)})")
    else:
        chains = chains + net['conn_chains']
    # Hoehen: Gelaende (DGM1 Bayern), Strassenprofile, Bruecken/Tunnel, flache Kreuzungen, Vorgaben aus dem Editor
    hoe = hoe_mod.Hoehen(gel, log=log)
    hoe.berechnen(net, sdb, junctions, ziele)
    if hat_gelaende:
        hs = [h for ch in net['road_chains'] for h in ch['y']]
        log(f'    Gelaende: Basis {hoe.base:.0f} m ue. NN, Strassen {min(hs):.1f} bis {max(hs):.1f} m darueber, '
            f'{sum(1 for ch in net["road_chains"] for v in ch["ebene"] if v)} Elemente auf Bruecken/in Tunneln')
    else:
        log('    Gelaende: keine DGM-Daten (ausserhalb Bayerns, ohne Netz oder abgeschaltet) - Karte bleibt flach')
    # Bauwerke: Brueckenkoerper, Tunnelroehren (Begleit-Splines), Pfeiler, Portale, Deckel ueber Tunnelgraeben
    bw = bauwerke.bauen(net, sdb, hoe, name)
    b = bw['stats']
    if b['bruecken'] or b['tunnel']:
        log(f"    Bauwerke: {b['bruecken']} Brueckenabschnitte mit {b['pfeiler']} Pfeilern, {b['tunnel']} "
            f"Tunnelabschnitte mit {b['portale']} Portalen und {b['deckel']} Deckeln ueber dem Tunnelgraben")
    return dict(proj=proj, sdb=sdb, net=net, signs=signs, stops=stops, junctions=junctions, vorfahrt=V, hoe=hoe,
                hat_gelaende=hat_gelaende, bw=bw, chains=chains + bw['ketten'], objekte=junctions + bw['objekte'])


def erzeuge(projekt, name, omsi=None, ausgabe='build', korrekturen=None, ueberschreiben=False, kreuzungen='objekt',
            titel=None, vorschau=None, ansicht_html=None, log=print, gelaende=True):
    """Projekt -> OMSI-Karte. -> dict(rc, dir, offen, enden, stats, kreuzungen, vorfahrt)"""
    B = berechne(projekt, name, omsi, korrekturen, kreuzungen, gelaende, log)
    proj, sdb, net, stops, junctions, V, hoe = (B['proj'], B['sdb'], B['net'], B['stops'], B['junctions'],
                                                B['vorfahrt'], B['hoe'])
    hat_gelaende, bw, chains, objekte, st = B['hat_gelaende'], B['bw'], B['chains'], B['objekte'], net['stats']
    log('5/6 Karte schreiben ...')
    root = omsi if omsi else ausgabe
    os.makedirs(root, exist_ok=True)
    install_splines(root, bw['splines'])
    desc = (f'Erzeugt mit omsigen: {projekt.get("beschreibung") or projekt.get("name", "")}.\n'
            + ('Strassendaten (c) OpenStreetMap-Mitwirkende, ODbL.' if 'OpenStreetMap' in projekt.get('quelle', '')
               else '') + (('\n' + gel_mod.QUELLE) if hat_gelaende else ''))
    if objekte:
        kdir = os.path.join(root, 'Sceneryobjects', 'Aschaffenburg', name)
        if os.path.exists(kdir) and not ueberschreiben:
            raise FileExistsError(f'Objektordner {kdir} existiert schon - anderen Kartennamen waehlen')
    linie = projekt.get('linie') or [projekt['strassen'][0]['punkte'][0]]
    # Einsetzpunkte: Start, Ziel (bzw. Anfang der ersten Strasse) und alle Haltestellen
    orte = [(('Start: ' if i == 0 else 'Ziel: ') + z['name'], z['p']) for i, z in
            enumerate(projekt.get('ziele', [])[::max(1, len(projekt.get('ziele', [])) - 1)])]
    if not orte:
        orte = [('Start', linie[0])]
    orte += [(s['name'], (s['x'], s['z'])) for s in stops]
    entrypoints, namen = [], set()
    for n, p in orte:
        e = spur_punkt(tuple(p), net['road_chains'], sdb)
        if e and hat_gelaende:
            e['y'] = hoe.gelaende(e['x'], e['z']) or 0.0
        if e and n not in namen:
            namen.add(n)
            entrypoints.append(dict(e, name=n[:60]))
    info = write_map(os.path.join(root, 'maps'), name, chains, stops, omsi_dir=omsi, friendly=titel or name,
                     description=desc, cam_xz=(entrypoints[0]['x'], entrypoints[0]['z']) if entrypoints else
                     tuple(linie[0]), overwrite=ueberschreiben, junctions=objekte, entrypoints=entrypoints,
                     raster=hoe.raster if hat_gelaende else None)
    kdir = None
    if objekte:
        kdir, _ = kreuzung.install_objects(root, name, objekte, omsi_dir=omsi, overwrite=ueberschreiben)
    with open(os.path.join(info['dir'], 'omsigen.json'), 'w', encoding='utf-8') as f:
        json.dump(dict(projekt=projekt.get('name'), origin=[proj.lat0, proj.lon0], offset=info['offset'],
                       hoehe_basis=hoe.base if hat_gelaende else None,
                       length_m=projekt.get('laenge'), stats=st,
                       kreuzungen=[dict(j['vorfahrt'], objekt=j['name'], x=j['origin'][0] - info['offset'][0],
                                        z=j['origin'][1] - info['offset'][1]) for j in junctions if j['vorfahrt']]),
                  f, ensure_ascii=False, indent=1)
    log(f'    {info["dir"]}: {info["tiles"]} Kacheln, {info["splines"]} Splines, '
        f'{len(junctions)} Kreuzungsobjekte, {len(stops)} Haltestellen, {len(entrypoints)} Einsetzpunkte')
    if junctions:
        log(f'    Kreuzungsobjekte: {kdir} ({sum(j["faces"] for j in junctions)} Dreiecke, '
            f'{sum(j["paths"] for j in junctions)} Pfade)')

    log('6/6 Pruefen ...')
    ox, oz = info['offset']
    r = ansicht.analyse(info['dir'], omsi)
    offen, enden = ansicht.classify(r['lanes'], r['ends'],
                                    edge_points=[(x - ox, z - oz) for x, z in projekt.get('rand', [])])
    se = r['summary']['Strasse_enden']
    log(f'    {se["gesamt"]} Spurenden, ohne Anschluss: {len(offen)} (+ {len(enden)} Strassenenden am '
        f'Rand/Sackgassen)')
    for e in offen[:10]:
        src = r['lanes'][e['lane']]['src']
        log(f'      offen: {src["art"]} {src["id"]} Pfad {src["pfad"]} bei x={e["p"][0]:.1f} z={e["p"][1]:.1f}')
    if r['summary']['fehlende_dateien']:
        log('    Nicht gefunden: ' + ', '.join(r['summary']['fehlende_dateien'][:5]))
    if vorschau:
        ansicht.write_png(vorschau, r['lanes'], r['roads'], r['objs'], r['ends'])
        log(f'    Vorschau: {vorschau}')
    if ansicht_html:
        ansicht.write_html(ansicht_html, r['m'], r['lanes'], r['roads'], r['objs'], r['ends'], r['summary'],
                           r['kreuzungen'])
        log(f'    Ansicht: {ansicht_html}')
    return dict(rc=0 if not offen else 2, dir=info['dir'], offen=len(offen), enden=len(enden), stats=st,
                kreuzungen=len(junctions), vorfahrt=V, kdir=kdir, offset=info['offset'])
