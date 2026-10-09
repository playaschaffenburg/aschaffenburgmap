"""Eigene Spline-Definitionen (.sli) fuer Splines\\Aschaffenburg: Einbahnstrassen und Kreuzungsspuren.
Texturen werden zur Laufzeit aus Splines\\Marcel\\texture der OMSI-Installation kopiert."""
import os

SIDE, ASPH, LINE = 0, 1, 2  # Texturindizes
TEX = ['str_side1.bmp', 'str_asphdrk.bmp', 'str_asphdrk_1line.bmp']


def sli(name, lanes, lane_w=3.5, walk_l=3.5, walk_r=3.5, single_w=5.0):
    """lanes: Anzahl Fahrstreifen (alle in Splinerichtung).
    walk_l/walk_r: Gehwegbreite links/rechts (0 -> 1 m Bordstein/Mittelstreifen)."""
    cw = single_w if lanes == 1 else lanes * lane_w
    xl, xr = -cw / 2, cw / 2
    wl = walk_l if walk_l > 0 else 1.0
    wr = walk_r if walk_r > 0 else 1.0
    out = ['File created with Aschaffenburg-KI-Generator (Claude)', '',
           '---------------------------', '      Height Profiles', '---------------------------', '']
    for a, b, h in ((xl - wl, xl, 0.25), (xl, xr, 0.1), (xr, xr + wr, 0.25)):
        out += ['[heightprofile]', f'{a:.3f}', f'{b:.3f}', f'{h:.3f}', f'{h:.3f}', '']
    out += ['---------------------------', '         Textures', '---------------------------', '']
    for t in TEX:
        out += ['[texture]', t, '']
    out += ['---------------------------', '     Graphical Lanes', '---------------------------', '']

    def prof(tex, pts):
        r = ['[profile]', str(tex), '']
        for x, y, u, v in pts:
            r += ['[profilepnt]', f'{x:.3f}', f'{y:.3f}', f'{u:.3f}', f'{v:.3f}', '']
        return r
    # linker Gehweg (Oberseite + Bordsteinkante)
    out += ['Left sidewalk:', '']
    out += prof(SIDE, [(xl - wl, 0.25, 0.187 if walk_l else 0.85, 0.2), (xl, 0.25, 0.953, 0.2)])
    out += prof(SIDE, [(xl, 0.25, 0.953, 0.2), (xl, 0.1, 0.995, 0.2)])
    # Fahrbahn
    out += ['Lanes:', '']
    if lanes == 1:
        out += prof(ASPH, [(xl, 0.1, 0.005, 0.167), (xr, 0.1, 0.995, 0.167)])
    else:
        edges = [xl + i * lane_w for i in range(lanes + 1)]
        for i in range(lanes):
            a, b = edges[i], edges[i + 1]
            if i == 0:                       # Linie am rechten Rand dieses Streifens
                out += prof(LINE, [(a, 0.1, 0.995, 0.167), (b, 0.1, 0.005, 0.167)])
            elif i == lanes - 1:             # Linie am linken Rand
                out += prof(LINE, [(a, 0.1, 0.005, 0.167), (b, 0.1, 0.995, 0.167)])
            else:
                out += prof(ASPH, [(a, 0.1, 0.005, 0.167), (b, 0.1, 0.995, 0.167)])
    # rechter Gehweg
    out += ['Right sidewalk:', '']
    out += prof(SIDE, [(xr, 0.1, 0.995, 0.2), (xr, 0.25, 0.953, 0.2)])
    out += prof(SIDE, [(xr, 0.25, 0.953, 0.2), (xr + wr, 0.25, 0.187 if walk_r else 0.85, 0.2)])
    out += ['---------------------------', '          Paths', '---------------------------', '']
    if walk_l:
        out += ['[path]', '1', f'{xl - wl / 2:.3f}', '0.250', f'{wl * 0.6:.3f}', '2', '']
    if lanes == 1:
        out += ['[path]', '0', '0.000', '0.100', '3.300', '0', '']
    else:
        for i in range(lanes):
            out += ['[path]', '0', f'{xl + (i + 0.5) * lane_w:.3f}', '0.100', '3.000', '0', '']
    if walk_r:
        out += ['[path]', '1', f'{xr + wr / 2:.3f}', '0.250', f'{wr * 0.6:.3f}', '2', '']
    half = cw / 2 + max(wl, wr)
    return '\r\n'.join(out) + '\r\n', half


SPLINES = {}
for lanes in (1, 2, 3):        # Gehweg beidseitig / nur rechts / nur links / keiner (nur Bordstein)
    for side, wl, wr in (('', 3.5, 3.5), ('_rechts', 0, 3.5), ('_links', 3.5, 0), ('_ohne', 0, 0)):
        n = f'AB_einbahn_{lanes}spur{side}.sli'
        SPLINES[n] = sli(n, lanes, walk_l=wl, walk_r=wr)

def kreuz(gap=None):
    """Kreuzungsspur: 3,5 m Asphalt mit einem Fahrpfad. gap -> zusaetzlich Asphalt bis
    zur Bordsteinkante (gap m rechts der Spurmitte) und 3,5 m Gehweg (Ecke beim Rechtsabbiegen)."""
    xl, xr = -1.75, (gap if gap else 1.75)
    out = ['File created with Aschaffenburg-KI-Generator (Claude)', '', '[heightprofile]',
           f'{xl:.3f}', f'{xr:.3f}', '0.100', '0.100', '']
    if gap:
        out += ['[heightprofile]', f'{xr:.3f}', f'{xr + 3.5:.3f}', '0.250', '0.250', '']
    for t in TEX:
        out += ['[texture]', t, '']

    def prof(tex, pts):
        r = ['[profile]', str(tex), '']
        for x, y, u, v in pts:
            r += ['[profilepnt]', f'{x:.3f}', f'{y:.3f}', f'{u:.3f}', f'{v:.3f}', '']
        return r
    out += prof(ASPH, [(xl, 0.1, 0.005, 0.167), (xr, 0.1, 0.995, 0.167)])
    if gap:
        out += prof(SIDE, [(xr, 0.1, 0.995, 0.2), (xr, 0.25, 0.953, 0.2)])
        out += prof(SIDE, [(xr, 0.25, 0.953, 0.2), (xr + 3.5, 0.25, 0.187, 0.2)])
    out += ['[path]', '0', '0.000', '0.100', '3.000', '0', '']
    return '\r\n'.join(out) + '\r\n', xr + (3.5 if gap else 0)


SPLINES['AB_kreuzung_spur.sli'] = kreuz()
KREUZ_GAPS = (1.75, 2.5, 3.5, 4.5)
for g in KREUZ_GAPS:
    SPLINES[f'AB_kreuzung_ecke_{str(g).replace(".", "_")}.sli'] = kreuz(g)


TEXTURE_FILES = ['str_side1.bmp', 'str_side1.bmp.cfg', 'str_asphdrk.bmp', 'str_asphdrk.bmp.cfg',
                 'str_asphdrk_1line.bmp', 'str_asphdrk_1line.bmp.cfg', 'betonwand1.bmp']
