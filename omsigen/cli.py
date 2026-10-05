"""Kommandozeile: eine reale Strecke als OMSI-2-Karte erzeugen.

Beispiel:
  python -m omsigen --omsi "C:/Program Files (x86)/Steam/steamapps/common/OMSI 2" \\
      --stadt Aschaffenburg --von "Hauptbahnhof" --nach "City Galerie" --name Aschaffenburg_Test
"""
import argparse, json, os, sys, math
from . import osm
from .route import Projection, route, corridor
from .network import build
from .splinedb import SplineDB
from .writer import write_map, place_stops, install_splines
from . import ansicht, kreuzung


def main(argv=None):
    ap = argparse.ArgumentParser(prog='omsigen', description='Reale Strecke -> OMSI-2-Karte (Strassennetz mit Kreuzungen)')
    ap.add_argument('--stadt', help='Stadt, in der gesucht wird (z. B. Aschaffenburg)')
    ap.add_argument('--von', required=True, help='Startpunkt: Ortsname oder "lat,lon"')
    ap.add_argument('--nach', required=True, help='Ziel: Ortsname oder "lat,lon"')
    ap.add_argument('--ueber', action='append', default=[], help='Zwischenziel (mehrfach moeglich)')
    ap.add_argument('--breite', type=float, default=150, help='Korridor links/rechts der Strecke in m (Standard 150)')
    ap.add_argument('--name', required=True, help='Name des neuen Kartenordners (z. B. Aschaffenburg_Linie1)')
    ap.add_argument('--titel', help='Anzeigename der Karte in OMSI')
    ap.add_argument('--omsi', help='OMSI-2-Ordner: Karte wird direkt dort installiert')
    ap.add_argument('--ausgabe', default='build', help='Ausgabeordner, falls --omsi fehlt (Standard: build)')
    ap.add_argument('--osm-datei', help='OSM-Daten aus Datei statt Download (siehe samples/)')
    ap.add_argument('--cache', default='.cache', help='Zwischenspeicher fuer Downloads')
    ap.add_argument('--ueberschreiben', action='store_true', help='vorhandenen Kartenordner mit gleichem Namen ersetzen')
    ap.add_argument('--vorschau', default=None, help='PNG-Draufsicht hierhin schreiben')
    ap.add_argument('--ansicht', default=None, help='interaktive HTML-Ansicht aller Pfade hierhin schreiben')
    ap.add_argument('--kreuzungen', choices=['objekt', 'spline'], default='objekt',
                    help='Kreuzungen als eigene Objekte mit Flaeche (Standard) oder nur aus Spur-Splines (alt)')
    a = ap.parse_args(argv)

    print('1/6 Orte suchen ...')
    places = [osm.find_place(q, a.stadt, a.cache) for q in [a.von] + a.ueber + [a.nach]]
    pts = [(p['lat'], p['lon']) for p in places]
    for q, p in zip([a.von] + a.ueber + [a.nach], places):
        info = f'  ({p["name"]}, {p["art"]})' if p['name'] else ''
        print(f'    {q}: {p["lat"]:.6f}, {p["lon"]:.6f}{info}')

    print('2/6 OSM-Daten laden ...')
    lat = [p[0] for p in pts]; lon = [p[1] for p in pts]
    m = (a.breite + 400) / 111000
    bbox = (min(lat) - m, min(lon) - m / math.cos(math.radians(lat[0])), max(lat) + m,
            max(lon) + m / math.cos(math.radians(lat[0])))
    data = osm.load(a.osm_datei) if a.osm_datei else osm.fetch(bbox, a.cache)
    print(f'    {len(data["ways"])} Strassen, {len(data["stops"])} Haltestellen')

    proj = Projection(sum(lat) / len(lat), sum(lon) / len(lon))
    print('3/6 Strecke berechnen ...')
    rinfo = {}
    line, length = route(data['ways'], proj, pts, rinfo)
    print(f'    Streckenlaenge {length:.0f} m (Abstand zur Strasse: '
          + ', '.join(f'{d:.0f} m' for d in rinfo['andocken']) + ')')

    print('4/6 Strassennetz und Kreuzungen bauen ...')
    ways, cuts = corridor(data['ways'], proj, line, a.breite)
    sdb = SplineDB(a.omsi)
    net = build(ways, sdb)
    st = net['stats']
    print(f'    {len(net["road_chains"])} Strassenzuege, {st["kreuzungen"]} Kreuzungen, '
          f'{st["verbindungen"]} Kreuzungsspuren {st["bewegungen"]}')

    from .network import proj_point
    stops_m = []
    for s in data['stops']:
        p = proj.to_m(s['lat'], s['lon'])
        if min(proj_point(p, a_, b_)[1] for a_, b_ in zip(line, line[1:])) <= a.breite:
            stops_m.append((s['name'], p))
    stops = place_stops(stops_m, net['road_chains'], sdb)

    print('5/6 Karte schreiben ...')
    root = a.omsi if a.omsi else a.ausgabe
    target = os.path.join(root, 'maps')
    os.makedirs(root, exist_ok=True)
    install_splines(root)
    chains, junctions = net['road_chains'], []
    if a.kreuzungen == 'objekt':
        junctions = kreuzung.build_objects(net, sdb, a.name)
    else:
        chains = chains + net['conn_chains']
    desc = (f'Automatisch erzeugt mit omsigen: {a.von} -> {a.nach}' + (f' ({a.stadt})' if a.stadt else '') +
            f', Korridor {a.breite:.0f} m.\nStrassendaten (c) OpenStreetMap-Mitwirkende, ODbL.')
    if junctions:     # vor der Karte pruefen, ob der Objektordner frei ist
        kdir = os.path.join(root, 'Sceneryobjects', 'Aschaffenburg_KI', a.name)
        if os.path.exists(kdir) and not a.ueberschreiben:
            raise FileExistsError(f'Objektordner {kdir} existiert schon - anderen Kartennamen waehlen')
    info = write_map(target, a.name, chains, stops, omsi_dir=a.omsi, friendly=a.titel or a.name,
                     description=desc, cam_xz=line[0], overwrite=a.ueberschreiben, junctions=junctions)
    if junctions:
        kdir, _ = kreuzung.install_objects(root, a.name, junctions, omsi_dir=a.omsi, overwrite=a.ueberschreiben)
    with open(os.path.join(info['dir'], 'omsigen.json'), 'w', encoding='utf-8') as f:
        json.dump(dict(args=vars(a), origin=[proj.lat0, proj.lon0], offset=info['offset'], length_m=length,
                       stats=st), f, ensure_ascii=False, indent=1)
    print(f'    {info["dir"]}: {info["tiles"]} Kacheln, {info["splines"]} Splines, '
          f'{len(junctions)} Kreuzungsobjekte, {len(stops)} Haltestellen')
    if junctions:
        print(f'    Kreuzungsobjekte: {kdir} ({sum(j["faces"] for j in junctions)} Dreiecke, '
              f'{sum(j["paths"] for j in junctions)} Pfade)')

    print('6/6 Pruefen ...')
    ox, oz = info['offset']
    r = ansicht.analyse(info['dir'], a.omsi)
    offen, enden = ansicht.classify(r['lanes'], r['ends'], edge_points=[(x - ox, z - oz) for x, z in cuts])
    se = r['summary']['Strasse_enden']
    print(f'    {se["gesamt"]} Spurenden, ohne Anschluss: {len(offen)} (+ {len(enden)} Strassenenden am '
          f'Rand/Sackgassen)')
    for e in offen[:10]:
        src = r['lanes'][e['lane']]['src']
        print(f'      offen: {src["art"]} {src["id"]} Pfad {src["pfad"]} bei x={e["p"][0]:.1f} z={e["p"][1]:.1f}')
    if r['summary']['fehlende_dateien']:
        print('    Nicht gefunden: ' + ', '.join(r['summary']['fehlende_dateien'][:5]))
    if a.vorschau:
        ansicht.write_png(a.vorschau, r['lanes'], r['roads'], r['objs'], r['ends'])
        print(f'    Vorschau: {a.vorschau}')
    if a.ansicht:
        ansicht.write_html(a.ansicht, r['m'], r['lanes'], r['roads'], r['objs'], r['ends'], r['summary'])
        print(f'    Ansicht: {a.ansicht}')
    return 0 if not offen else 2


if __name__ == '__main__':
    sys.exit(main())
