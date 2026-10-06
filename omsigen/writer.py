"""Schreibt einen OMSI-2-Kartenordner: Kacheln (.map, UTF-16), global.cfg, Gelaende, Haltestellenschilder,
und installiert die eigenen Splines. Bestehende Karten werden nie ueberschrieben."""
import os, math, shutil, datetime, collections, json
from .geom import end_of, heading, rvec
from .network import proj_point
from .custom_splines import SPLINES as CUSTOM, TEXTURE_FILES

TILE = 300.0
TEMPLATE_FILES = ['drivers.txt', 'gras1.bmp', 'Holidays.txt', 'Holidays_DEU.txt', 'Holidays_ENG.txt',
                  'Holidays_FRA.txt', 'Holidays_POL.txt', 'humans.txt', 'parklist_p.txt', 'registrations.txt',
                  'signalroutes.cfg', 'unsched_trafficdens.txt', 'unsched_vehgroups.txt',
                  'texture/water.tga', 'texture/water_bump.bmp', 'texture/water_envmap.bmp']


def fmt(v):
    return repr(float(v)) if v != 0 else '0'


def place_stops(stops, road_chains, sdb, maxdist=25):
    """Haltestellen (name, (x,z)) an den naechsten Fahrbahnrand setzen -> Liste dict(name, x, z, rot)"""
    segs = [((el[0], el[1]), end_of(el)[0], el[5]) for c in road_chains for el in c['els']]
    out = []
    for name, p in stops:
        if not segs:
            break
        a, b, spl = min(segs, key=lambda s: proj_point(p, s[0], s[1])[1])
        s, d = proj_point(p, a, b)
        if d > maxdist:
            continue
        q = (a[0] + (b[0] - a[0]) * s, a[1] + (b[1] - a[1]) * s)
        h = heading(a, b)
        side = 1 if ((b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])) < 0 else -1
        off = sdb[spl]['cw'] + 1.5
        rv = rvec(h)
        out.append(dict(name=name, x=q[0] + side * off * rv[0], z=q[1] + side * off * rv[1],
                        rot=h if side == 1 else (h + 180) % 360))
    return out


# Tagesganglinie des Strassenverkehrs (Stunde, Faktor) wie in Grundorf
DENS_ROAD = ((0, .1), (4, 0), (6, 1), (7, 1.5), (8, 1), (10, .5), (15, .6), (16, 1), (17, 1.5), (20, .4), (24, .1))
# KI-Fahrzeuge (Pfad unter Vehicles, Gewicht) - die Gruppen erwartet unsched_vehgroups.txt der Vorlage
AI_GROUPS = {
    'NormalCars': [('VW_Golf_2\\AI_VW_Golf_2.bus', 70), ('MB_W123_230E\\AI_mb_w123_230e.bus', 50),
                   ('MB_W123_230E\\AI_mb_w123_230e_cab.bus', 10), ('VW_T3\\VW_T3_Van.ovh', 10),
                   ('VW_T3\\VW_T3_Transporter.ovh', 10), ('Opel_Manta_B\\ai_opel_manta_b.ovh', 10),
                   ('Citr_BX\\BX.ovh', 40), ('MB_T1\\ai_mb_t1_kasten.ovh', 10)],
    'Trucks': [('MAN_F90\\AI_MAN_F90_Wechselbruecke.bus', 30)],
}


def ailists_cfg(omsi_dir=None):
    """ailists.cfg im Format der Standardkarten ([aigroup_2] Name, Hof-Datei (leer), Fahrzeug<TAB>Anzahl, [end]);
    nur Fahrzeuge, die in der OMSI-Installation vorhanden sind"""
    L = ['Erzeugt mit omsigen', '']
    for name, veh in AI_GROUPS.items():
        ok = [(v, n) for v, n in veh
              if not omsi_dir or os.path.exists(os.path.join(omsi_dir, 'Vehicles', *v.split('\\')))]
        L += ['[aigroup_2]', name, ''] + [f'vehicles\\{v}\t{n}' for v, n in ok] + ['[end]', '', '']
    return '\r\n'.join(L)


def global_cfg(name, friendly, description, next_id, tiles, cam):
    tx, tz, cx, cz = cam
    L = ['File created with omsigen', '', '[name]', name, '', '[friendlyname]', friendly, '',
         '[description]', description, '[end]', '', '[version]', '14', '', '[NextIDCode]', str(next_id), '',
         '[backgroundimage]', '0', '', '299.999995292025', '299.999995292025', '0', '0', '',
         '[mapcam]', str(tx), str(tz), f'{cx:.3f}', '0.95', f'{cz:.3f}', '0', '-30', '250', '',
         '[moneysystem]', 'Money\\DM\\DM.cfg', '', '[repair_time_min]', '10.000', '',
         '[ticketpack]', 'TicketPacks\\Berlin_1\\Berlin_1.otp', '', '[years]', '1988', '2000', '',
         '[standarddepot]', 'Hof Spandau', '',
         '[groundtex]', 'texture\\gras.bmp', 'texture\\gras_det.bmp', '0', '1', '60', '',
         '[groundtex]', 'Texture\\str_k_kopfstein.bmp', 'Texture\\noise_low.bmp', '9', '20', '1', '']
    for code, a, b in ((4, 0, 38), (3, 38, 79), (1, 79, 172), (2, 264, 355), (3, 355, 370)):
        L += ['[addseason]', str(code), str(a), str(b), '']
    for h, v in ((0, .2), (4, 0), (6, 1), (7, 1.2), (8, 1), (10, .6), (15, .8), (16, 1.2), (17, 1.2), (20, .8), (24, .2)):
        L += ['[trafficdensity_passenger]', f'{h:.3f}', f'{v:.3f}', '']
    for h, v in DENS_ROAD:
        L += ['[trafficdensity_road]', f'{h:.3f}', f'{v:.3f}', '']
    for (x, z) in sorted(tiles):
        L += ['[map]', str(x), str(z), f'tile_{x}_{z}.map', '']
    return '\r\n'.join(L) + '\r\n'


def write_utf16(path, text):
    with open(path, 'wb') as f:
        f.write(b'\xff\xfe' + text.replace('\r\n', '\n').replace('\n', '\r\n').encode('utf-16-le'))


def flat_terrain():
    import struct
    return struct.pack('<i', 60) + b'\x00' * (61 * 61 * 4)


def write_map(out_maps_dir, name, chains, stops, omsi_dir=None, friendly=None, description='', cam_xz=None,
              overwrite=False, junctions=()):
    """chains: Liste Ketten (els in Metern, beliebiger Ursprung); junctions: Kreuzungsobjekte (kreuzung.py) mit
    origin und rel (Pfad der .sco). Gibt Infos inkl. Verschiebung zurueck."""
    D = os.path.join(out_maps_dir, name)
    if os.path.exists(D) and not overwrite:
        raise FileExistsError(f'Kartenordner {D} existiert schon - anderen Namen waehlen')
    os.makedirs(os.path.join(D, 'texture'), exist_ok=True)

    xs = [el[0] for c in chains for el in c['els']] + [s['x'] for s in stops] + [j['origin'][0] for j in junctions]
    zs = [el[1] for c in chains for el in c['els']] + [s['z'] for s in stops] + [j['origin'][1] for j in junctions]
    ox, oz = math.floor(min(xs)) - 20.0, math.floor(min(zs)) - 20.0     # Karte beginnt bei Kachel 0_0

    nid, tiles = 1, collections.defaultdict(list)
    for c in chains:
        ids = list(range(nid, nid + len(c['els']))); nid += len(c['els'])
        cum = 0.0
        for i, el in enumerate(c['els']):
            x, z, h, L, R, spl = el
            x, z = x - ox, z - oz
            t = (int(x // TILE), int(z // TILE))
            tiles[t].append(['[spline]', '0', spl, str(ids[i]), str(ids[i - 1] if i else 0),
                             str(ids[i + 1] if i < len(ids) - 1 else 0), fmt(x - TILE * t[0]), '0', fmt(z - TILE * t[1]),
                             fmt(h % 360), fmt(L), fmt(R), '0', '0', '0', '0', '0', '0', fmt(cum), '', ''])
            cum += L
    objs = collections.defaultdict(list)
    for s in stops:
        x, z = s['x'] - ox, s['z'] - oz
        t = (int(x // TILE), int(z // TILE))
        objs[t].append(['[object]', '0', 'Sceneryobjects\\Generic\\bus_stop.sco', str(nid), fmt(x - TILE * t[0]),
                        fmt(z - TILE * t[1]), '0', fmt(s['rot']), '0', '0', '7', s['name'], '20', '5', '0', '', '', '',
                        '', ''])
        nid += 1
    for j in junctions:          # Kreuzungsobjekt: Drehung 0, Hoehe 0, keine Texte
        x, z = j['origin'][0] - ox, j['origin'][1] - oz
        t = (int(x // TILE), int(z // TILE))
        blk = ['[object]', '0', j['rel'], str(nid), fmt(x - TILE * t[0]), fmt(z - TILE * t[1]), '0', '0',
               '0', '0', '0', '']
        for idx, val in j.get('rules', ()):     # Vorfahrt: [rule] gilt fuer den Pfad idx des Objekts davor
            blk += ['[rule]', str(idx), 'priority', str(val), '0', '']
        objs[t].append(blk)
        jid = nid
        nid += 1
        ids = []
        for sg in j.get('signale', ()):         # Ampel: Signal (Text = Signalgruppe, [varparent] = Kreuzung) und Mast
            if sg['art'] == 'oben':             # am Mast eingehaengt; muss nach dem Mast in der Kachel stehen
                blk = ['[attachObj]', '0', sg['datei'], str(nid), str(ids[sg['eltern']]), '0', str(sg['anhang']),
                       fmt(sg['rot']), '0', '0']
            else:
                sx, sz = sg['x'] - ox - TILE * t[0], sg['z'] - oz - TILE * t[1]      # in der Kachel der Kreuzung
                blk = ['[object]', '0', sg['datei'], str(nid), fmt(sx), fmt(sz), fmt(sg['hoehe']), fmt(sg['rot']),
                       '0', '0']
            if sg['gruppe'] is None:
                blk += ['0', '']
            else:
                blk += ['1', str(sg['gruppe']), '', '[varparent]', str(jid), '']
            objs[t].append(blk)
            ids.append(nid)
            nid += 1
    # Nachbarkacheln mit anlegen, damit rundherum Gelaende ist
    used = set(tiles) | set(objs)
    all_tiles = {(x + dx, z + dz) for x, z in used for dx in (-1, 0, 1) for dz in (-1, 0, 1)
                 if x + dx >= 0 and z + dz >= 0}

    stamp = datetime.datetime.now().strftime('%d.%m.%Y %H:%M:%S')
    tpl = os.path.join(omsi_dir, 'template', 'NewMap') if omsi_dir else None
    for t in sorted(all_tiles):
        L = [f'File created with omsigen on {stamp}', '', '[version]', '14', '', '[terrain]', '', '',
             '[variable_terrainlightmap]', '', '[variable_terrain]', '']
        for blk in tiles.get(t, []):
            L += blk
        for i, blk in enumerate(objs.get(t, [])):
            L += [f'Object Nr. {i}'] + blk
        base = os.path.join(D, f'tile_{t[0]}_{t[1]}.map')
        write_utf16(base, '\n'.join(L) + '\n')
        with open(base + '.terrain', 'wb') as f:
            f.write(flat_terrain())
        if tpl and os.path.exists(os.path.join(tpl, 'tile_0_0.map.LM.bmp')):
            shutil.copy(os.path.join(tpl, 'tile_0_0.map.LM.bmp'), base + '.LM.bmp')

    if tpl and os.path.isdir(tpl):
        for f in TEMPLATE_FILES:
            src = os.path.join(tpl, *f.split('/'))
            if os.path.exists(src):
                shutil.copy(src, os.path.join(D, *f.split('/')))
    with open(os.path.join(D, 'ailists.cfg'), 'w', encoding='cp1252', newline='') as f:
        f.write(ailists_cfg(omsi_dir))
    cx, cz = (cam_xz[0] - ox, cam_xz[1] - oz) if cam_xz else (150.0, 150.0)
    cam = (int(cx // TILE), int(cz // TILE), cx % TILE, cz % TILE)
    write_utf16(os.path.join(D, 'global.cfg'),
                global_cfg(name, friendly or name, description, nid + 10, all_tiles, cam))
    with open(os.path.join(D, 'LIESMICH_omsigen.txt'), 'w', encoding='utf-8') as f:
        f.write(f'Erzeugt mit omsigen am {stamp}.\nStrassendaten (c) OpenStreetMap-Mitwirkende, ODbL 1.0 '
                f'(https://www.openstreetmap.org/copyright).\n{description}\n')
    return dict(dir=D, offset=(ox, oz), tiles=len(all_tiles), splines=sum(len(v) for v in tiles.values()),
                objects=sum(len(v) for v in objs.values()))


def install_splines(omsi_dir_or_out):
    """Eigene .sli-Dateien nach Splines\\Aschaffenburg_KI schreiben; Texturen aus Splines\\Marcel\\texture kopieren"""
    d = os.path.join(omsi_dir_or_out, 'Splines', 'Aschaffenburg_KI')
    os.makedirs(os.path.join(d, 'texture'), exist_ok=True)
    for n, (t, _) in CUSTOM.items():
        with open(os.path.join(d, n), 'w', encoding='cp1252', newline='') as f:
            f.write(t)
    src = os.path.join(omsi_dir_or_out, 'Splines', 'Marcel', 'texture')
    copied = 0
    for f in TEXTURE_FILES:
        if os.path.exists(os.path.join(src, f)) and not os.path.exists(os.path.join(d, 'texture', f)):
            shutil.copy(os.path.join(src, f), os.path.join(d, 'texture', f)); copied += 1
    return d, copied
