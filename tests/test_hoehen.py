import math
from omsigen.utm import to_utm, from_utm
from omsigen.network import build
from omsigen.splinedb import SplineDB
from omsigen.hoehen import Hoehen, LICHTE_HOEHE, UEBERDECKUNG


def test_utm():
    e, n = to_utm(49.9807, 9.1439)                       # Aschaffenburg Hbf, Zone 32
    assert abs(e - 510317.0) < 0.5 and abs(n - 5536494.8) < 0.5
    la, lo = from_utm(e, n)
    assert abs(la - 49.9807) < 1e-7 and abs(lo - 9.1439) < 1e-7      # < 1 cm


class Kunstgelaende:
    """Hang mit 4 % nach Osten; bei x 200..300 ein 10 m tiefes Tal, bei x 500..600 ein 15 m hoher Huegel"""
    def hoehe(self, x, z):
        h = 100 + 0.04 * x
        if 200 <= x <= 300:
            h -= 10
        if 500 <= x <= 600:
            h += 15
        return h


def _netz(*tags_abschnitte):
    """Strasse von x=0 bis 800 entlang z=0, Abschnitte mit eigenen Tags (Bruecke, Tunnel)"""
    ways, x = [], 0.0
    for laenge, extra in tags_abschnitte:
        ways.append(dict(tags=dict(highway='secondary', name='A', **extra), P=[(x, 0.0), (x + laenge, 0.0)]))
        x += laenge
    return build(ways, SplineDB())


def _hoehe_bei(net, x):
    for ch in net['road_chains']:
        for i, e in enumerate(ch['els']):
            if e[0] <= x < e[0] + e[3] + 1e-6 and abs(e[1]) < 1:
                return ch['y'][i] + ch['g'][i] / 100 * (x - e[0])
    raise AssertionError(f'keine Strasse bei x={x}')


def test_hang_bruecke_tunnel():
    net = _netz((190, {}), (120, {'bridge': 'yes', 'layer': '1'}), (180, {}), (130, {'tunnel': 'yes', 'layer': '-1'}),
                (180, {}))
    g = Kunstgelaende()
    hoe = Hoehen(g, log=lambda *a: None).berechnen(net, SplineDB())
    b = hoe.base
    # auf Grund folgt die Strasse dem Hang (bis auf die Glaettung)
    assert abs(_hoehe_bei(net, 100) + b - g.hoehe(100, 0)) < 0.5
    assert abs(_hoehe_bei(net, 420) + b - g.hoehe(420, 0)) < 0.5
    # Bruecke ueber das Tal: mindestens lichte Hoehe ueber dem Talboden, ohne Sprung
    assert _hoehe_bei(net, 250) + b >= g.hoehe(250, 0) + LICHTE_HOEHE - 0.1
    # Tunnel durch den Huegel: unter der Oberflaeche
    assert _hoehe_bei(net, 550) + b <= g.hoehe(550, 0) - UEBERDECKUNG + 0.1
    # Steigungen bleiben fahrbar
    assert max(abs(v) for ch in net['road_chains'] for v in ch['g']) < 12


def test_bauwerke():
    from omsigen import bauwerke
    from omsigen.splinedb import parse_sli
    net = _netz((190, {}), (120, {'bridge': 'yes', 'layer': '1'}), (180, {}), (130, {'tunnel': 'yes', 'layer': '-1'}),
                (180, {}))
    hoe = Hoehen(Kunstgelaende(), log=lambda *a: None).berechnen(net, SplineDB())
    bw = bauwerke.bauen(net, SplineDB(), hoe, 'Test')
    st = bw['stats']
    assert st['bruecken'] == 1 and st['tunnel'] == 1
    assert st['pfeiler'] >= 2 and st['portale'] == 2
    arten = {k['begleit']: k for k in bw['ketten']}
    # Begleit-Splines liegen genau auf der Fahrbahn und haben keine Pfade
    assert abs(sum(e[3] for e in arten['bruecke']['els']) - 120) < 1
    for datei, text in bw['splines'].items():
        assert parse_sli(text)['lanes'] == [] and parse_sli(text)['walks'] == []
        assert ('[terrainholeprofile]' in text) == datei.startswith('AB_tunnel')
    # Gelaendeloch nur am Portal, nicht mitten im Tunnel
    al = arten['tunnel']['align']
    assert al[0] and al[-1] and not all(al)
    # Pfeiler stehen auf dem Talboden und reichen bis unter die Platte
    for o in bw['objekte']:
        if o['name'].startswith('pfeiler'):
            x, z = o['origin']
            assert abs(o['hoehe'] + 1.0 - hoe.gelaende(x, z)) < 0.01 and 200 <= x <= 300
    # ueber der Tunnelroehre bleibt Gelaende
    H = hoe.raster(500.0, -150.0)                       # Kachel ueber dem Tunnel (x 500..800)
    assert hoe.decken and H[30, 10] >= _hoehe_bei(net, 550) + bauwerke.TUNNEL_H
