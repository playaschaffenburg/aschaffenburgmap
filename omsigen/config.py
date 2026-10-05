"""Zentrale Einstellungen: welche Strassen-Splines fuer welche OSM-Strassen benutzt werden."""

MARCEL = 'Splines\\Marcel\\'          # Standard-Splines aus OMSI 2 (Spandau/Grundorf)
KI = 'Splines\\Aschaffenburg_KI\\'    # eigene, vom Tool erzeugte Splines

# Ersatzwerte fuer die Fahrspuren der benutzten Standard-Splines (aus den .sli-Dateien ausgelesen).
# Werden benutzt, wenn keine OMSI-Installation angegeben ist (z. B. in Tests).
# lanes: (Querlage in m, Richtung 0 = mit Splinerichtung / 1 = dagegen); cw = halbe Fahrbahnbreite; half = halbe Gesamtbreite;
# walks: Querlage der Gehwegpfade. Alle Standardsplines sind symmetrisch (cl/cr/ol/or_ werden unten ergaenzt).
FALLBACK_SPLINES = {
    MARCEL + 'str_2spur_11m_SeeburgerStr1.sli': dict(lanes=[(-1.639, 1), (1.639, 0)], cw=5.5, half=10.5, walks=[-7.806, 7.806]),
    MARCEL + 'str_2spur_8m_altonaer1.sli': dict(lanes=[(-2.0, 1), (2.0, 0)], cw=4.0, half=7.0, walks=[-5.384, 5.384]),
    MARCEL + 'str_2spur_6m_staakener1.sli': dict(lanes=[(-1.5, 1), (1.5, 0)], cw=3.0, half=7.0, walks=[-4.845, 4.845]),
    MARCEL + 'str_3spur_12m_Charlottenstr1.sli': dict(lanes=[(-4.485, 1), (-1.485, 1), (1.485, 0)], cw=6.0, half=10.0, walks=[-7.845, 7.845]),
    MARCEL + 'str_4spur_12,5m_Heerstr1.sli': dict(lanes=[(-4.75, 1), (-1.609, 1), (1.609, 0), (4.75, 0)], cw=6.25, half=8.0, walks=[]),
    MARCEL + 'str_2spur_13m_Omnibushof.sli': dict(lanes=[(-1.609, 1), (1.609, 0)], cw=6.5, half=10.5, walks=[-8.345, 8.345]),
}
for _v in FALLBACK_SPLINES.values():
    _v.update(cl=-_v['cw'], cr=_v['cw'], ol=-_v['half'], or_=_v['half'])

# Rang fuer die Wahl der Hauptrichtung und Kurvenradien je Strassenklasse
RANK = {'trunk': 7, 'primary': 6, 'secondary': 5, 'secondary_link': 4, 'primary_link': 4, 'tertiary': 4,
        'tertiary_link': 3, 'bus': 3, 'unclassified': 2, 'residential': 2, 'living_street': 1, 'service': 0}
RADIUS = {'trunk': 60, 'primary': 40, 'secondary': 35, 'secondary_link': 25, 'primary_link': 25, 'tertiary': 30,
          'tertiary_link': 20, 'bus': 12, 'unclassified': 15, 'residential': 15, 'living_street': 10, 'service': 8}

# Strassenklassen, die uebernommen werden (Fussgaengerzonen, Feldwege usw. nicht)
DRIVABLE = {'motorway', 'motorway_link', 'trunk', 'trunk_link', 'primary', 'primary_link', 'secondary',
            'secondary_link', 'tertiary', 'tertiary_link', 'unclassified', 'residential', 'living_street',
            'service', 'busway'}


WALK_SUFFIX = {(True, True): '', (False, True): '_rechts', (True, False): '_links', (False, False): '_ohne'}


def is_oneway(t):
    return t.get('oneway') in ('yes', '1', '-1', 'true') or t.get('junction') in ('roundabout', 'circular')


def choose_spline(t, dual, walk=(True, True)):
    """t: OSM-Tags einer Strasse (nach osm.normalize), dual: Richtungsfahrbahn einer getrennten Strasse,
    walk: (links, rechts) Gehweg moeglich (nur fuer Einbahnstrassen, in Splinerichtung).
    -> (Splinepfad, Richtungszwang) mit +1 = Spline in OSM-Richtung, -1 = umgekehrt, 0 = egal"""
    hw = t.get('highway')
    lanes = _int(t.get('lanes'))
    if is_oneway(t):
        n = max(1, min(3, lanes or 1))
        wl, wr = walk
        if dual or t.get('junction') in ('roundabout', 'circular'):
            wl = False                      # Mittelstreifen bzw. Kreisinsel
        return KI + f'AB_einbahn_{n}spur{WALK_SUFFIX[(wl, wr)]}.sli', (-1 if t.get('oneway') == '-1' else +1)
    lf, lb = _int(t.get('lanes:forward')), _int(t.get('lanes:backward'))
    if lanes == 3 and (lf, lb) in ((1, 2), (2, 1)):
        # Charlottenstr1: 2 Streifen gegen, 1 Streifen mit der Splinerichtung
        return MARCEL + 'str_3spur_12m_Charlottenstr1.sli', (+1 if lb == 2 else -1)
    if lanes >= 4:
        return MARCEL + 'str_4spur_12,5m_Heerstr1.sli', 0
    if hw == 'bus':
        return MARCEL + 'str_2spur_13m_Omnibushof.sli', 0
    if hw in ('trunk', 'primary', 'secondary', 'tertiary', 'trunk_link', 'primary_link', 'secondary_link',
              'tertiary_link'):
        return MARCEL + 'str_2spur_11m_SeeburgerStr1.sli', 0
    if hw in ('residential', 'unclassified'):
        return MARCEL + 'str_2spur_8m_altonaer1.sli', 0
    return MARCEL + 'str_2spur_6m_staakener1.sli', 0


def _int(v):
    try:
        return int(str(v).split(';')[0])
    except (TypeError, ValueError):
        return 0

# Standard-Installationsort beim Nutzer (Steam); wird nur benutzt, wenn der Ordner existiert
_OMSI_STEAM = r'C:\Program Files (x86)\Steam\steamapps\common\OMSI 2'
DEFAULT_OMSI = _OMSI_STEAM if __import__('os').path.isdir(_OMSI_STEAM) else None
