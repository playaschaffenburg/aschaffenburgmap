from omsigen.route import Projection, route

P = Projection(49.98, 9.14)


def _way(*pts_m, **tags):
    return dict(tags=dict(highway='residential', **tags), coords=[list(P.to_ll(x, z)) for x, z in pts_m])


def test_route_meidet_abgeschnittenes_teilnetz():
    """Wie am Klinikum: Der naechste Knoten (20 m) liegt in einem Teilnetz, das nur ueber eine im Import
    weggelassene Strasse (z. B. private Zufahrt) angebunden waere. Die Strecke muss im Stadtnetz (60 m) beginnen."""
    ways = [_way((-100, 20), (0, 20), (100, 20), name='Insel'),
            _way((-100, -60), (0, -60), (100, -60), (100, -300), name='Ludwigstrasse')]
    info = {}
    line, L = route(ways, P, [P.to_ll(0, 0), P.to_ll(100, -300)], info)
    assert abs(line[0][1] - (-60)) < 0.1 and abs(line[-1][1] - (-300)) < 0.1
    assert abs(L - 340) < 1
    assert [round(d) for d in info['andocken']] == [60, 0]


def test_route_oneway_gegenrichtung():
    """oneway=-1: nur entgegen der Zeichenrichtung befahrbar -> Umweg ueber die Parallelstrasse"""
    ways = [_way((0, 0), (200, 0), oneway='-1'),
            _way((0, 0), (0, 50), (200, 50), (200, 0))]
    line, L = route(ways, P, [P.to_ll(0, 0), P.to_ll(200, 0)])
    assert abs(L - 300) < 1
    line, L = route(ways, P, [P.to_ll(200, 0), P.to_ll(0, 0)])
    assert abs(L - 200) < 1


def test_route_ueber_bruecke():
    """Bruecken gehoeren zum Netz: die Strecke darf darueber fuehren"""
    ways = [_way((0, 0), (0, 100), name='A'), _way((0, 100), (0, 200), name='Bruecke', bridge='yes', layer='1'),
            _way((0, 200), (0, 300), name='B')]
    line, L = route(ways, P, [P.to_ll(0, 0), P.to_ll(0, 300)])
    assert abs(L - 300) < 1
