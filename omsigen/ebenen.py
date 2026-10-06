"""Hoehenvorgaben aus dem Editor und automatische Bruecken/Tunnel (wie in Transport Fever 2).

Projekt: strasse['hoehen'] = Liste parallel zu strasse['punkte'], je Punkt die Hoehe ueber dem Gelaende in Metern
oder None (Punkt folgt dem Gelaende, wie OSM-Strassen ohne Vorgabe). Zwischen zwei Punkten verlaeuft die Strasse
gerade (lineare absolute Hoehe). Liegt sie dabei mindestens BRUECKE_AB ueber dem Gelaende, wird der Abschnitt
zur Bruecke, mindestens TUNNEL_AB darunter zum Tunnel; dazwischen entstehen Damm bzw. Einschnitt.

Erzwungene Ebene: Tags bridge/tunnel (wie OSM) oder 'omsigen:ebene' = 'boden' schalten die Automatik ab."""
import math

BRUECKE_AB = 4.0        # m Fahrbahn ueber Gelaende -> Bruecke
TUNNEL_AB = 5.0         # m Fahrbahn unter Gelaende -> Tunnel
SCHRITT = 4.0           # m Abtastung entlang der Strasse
MIN_LAUF = 8.0          # m: kuerzere Bruecken/Tunnel werden Damm/Einschnitt


def _boden(gel, x, z):
    h = gel.hoehe(x, z) if gel else None
    return 0.0 if h is None else h


def hat_vorgaben(strasse):
    H = strasse.get('hoehen')
    return bool(H) and any(h is not None for h in H)


def automatik(tags):
    """True, wenn die Ebene aus den Hoehen folgen soll (keine erzwungene Ebene)"""
    return not (tags.get('bridge') not in (None, 'no') or tags.get('tunnel') not in (None, 'no')
                or tags.get('omsigen:ebene') == 'boden')


def knotenhoehen(punkte, hoehen, gel):
    """absolute Hoehe (ue. NN bzw. ueber 0 ohne Gelaende) je Punkt: Gelaende + Vorgabe; Punkte ohne Vorgabe
    folgen dem Gelaende"""
    return [_boden(gel, *p) + (h or 0.0) for p, h in zip(punkte, hoehen)]


def abtasten(punkte, hoehen, gel, schritt=SCHRITT):
    """-> [(x, z, absolute Hoehe, Gelaende, Segmentindex, t)] entlang der Strasse, Hoehe linear je Segment"""
    K = knotenhoehen(punkte, hoehen, gel)
    out = []
    for i in range(len(punkte) - 1):
        (xa, za), (xb, zb) = punkte[i], punkte[i + 1]
        n = max(1, int(math.ceil(math.dist(punkte[i], punkte[i + 1]) / schritt)))
        for k in range(n + (1 if i == len(punkte) - 2 else 0)):
            t = k / n
            x, z = xa + (xb - xa) * t, za + (zb - za) * t
            out.append((x, z, K[i] + (K[i + 1] - K[i]) * t, _boden(gel, x, z), i, t))
    return out


def ziele(punkte, hoehen, gel, schritt=SCHRITT):
    """Hoehenvorgaben fuer hoehen.Hoehen.berechnen: [(x, z, absolute Hoehe)] auf Segmenten mit mindestens einem
    gesetzten Endpunkt"""
    return [(x, z, h) for x, z, h, _, i, t in abtasten(punkte, hoehen, gel, schritt)
            if hoehen[i] is not None or hoehen[i + 1] is not None]


def art_bei(h, boden):
    if h - boden >= BRUECKE_AB:
        return 'bruecke'
    if boden - h >= TUNNEL_AB:
        return 'tunnel'
    return None


def einteilen(punkte, hoehen, gel, schritt=SCHRITT):
    """Strasse in Abschnitte Boden/Bruecke/Tunnel teilen -> [(art, punkte, hoehen)]; an Uebergaengen entstehen
    neue Punkte (gemeinsamer Endpunkt, damit omsigen sie wieder zu einem Strassenzug verbindet)"""
    A = abtasten(punkte, hoehen, gel, schritt)
    if len(A) < 2:
        return [(None, list(punkte), list(hoehen))]
    arten = [art_bei(h, b) for _, _, h, b, _, _ in A]
    # kurze Bauwerke glaetten: Laeufe unter MIN_LAUF werden Boden
    i = 0
    while i < len(A):
        j = i
        while j + 1 < len(A) and arten[j + 1] == arten[i]:
            j += 1
        if arten[i] and math.dist(A[i][:2], A[j][:2]) < MIN_LAUF:
            for k in range(i, j + 1):
                arten[k] = None
        i = j + 1
    # Abschnitte: Wechsel zwischen zwei Abtastpunkten -> Uebergang in der Mitte
    teile, aktuell = [], dict(art=arten[0], P=[tuple(punkte[0])], H=[hoehen[0]])
    for k in range(1, len(A)):
        x, z, h, b, seg, t = A[k]
        if arten[k] != aktuell['art']:
            xm, zm = (A[k - 1][0] + x) / 2, (A[k - 1][1] + z) / 2
            hm = (A[k - 1][2] + h) / 2
            vm = round(hm - _boden(gel, xm, zm), 2)
            aktuell['P'].append((xm, zm)); aktuell['H'].append(vm)
            teile.append(aktuell)
            aktuell = dict(art=arten[k], P=[(xm, zm)], H=[vm])
        if t == 0 and seg > 0 and math.dist(punkte[seg], aktuell['P'][-1]) > 1e-6:
            aktuell['P'].append(tuple(punkte[seg])); aktuell['H'].append(hoehen[seg])
    if math.dist(punkte[-1], aktuell['P'][-1]) > 1e-6:
        aktuell['P'].append(tuple(punkte[-1])); aktuell['H'].append(hoehen[-1])
    teile.append(aktuell)
    return [(t['art'], t['P'], t['H']) for t in teile if len(t['P']) >= 2]


def ebene_tags(tags, art):
    """Tags eines automatisch entstandenen Abschnitts"""
    t = {k: v for k, v in tags.items() if k not in ('bridge', 'tunnel', 'layer', 'omsigen:ebene')}
    if art == 'bruecke':
        t.update(bridge='yes', layer='1')
    elif art == 'tunnel':
        t.update(tunnel='yes', layer='-1')
    return t
