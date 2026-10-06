from omsigen import ebenen, pipeline
from omsigen.network import build
from omsigen.splinedb import SplineDB
from omsigen.hoehen import Hoehen


class Gel:
    """eben auf 100 m, bei x 150..250 ein 12 m tiefes Tal, bei x 450..550 ein 14 m hoher Huegel"""
    def hoehe(self, x, z):
        if 150 <= x <= 250:
            return 88.0
        if 450 <= x <= 550:
            return 114.0
        return 100.0


def test_einteilen_wie_tf2():
    P, H = [(0, 0), (700, 0)], [0.0, 0.0]          # nur die Enden gesetzt: gerade Linie auf 100 m
    teile = ebenen.einteilen(P, H, Gel())
    arten = [a for a, _, _ in teile]
    assert arten == [None, 'bruecke', None, 'tunnel', None]
    br = teile[1][1]
    assert 145 <= br[0][0] <= 155 and 245 <= br[-1][0] <= 255
    # Abschnitte haengen lueckenlos aneinander
    for (_, a, _), (_, b, _) in zip(teile, teile[1:]):
        assert a[-1] == b[0]
    # Vorgaben auf der ganzen Strecke, Hoehe der geraden Linie
    Z = ebenen.ziele(P, H, Gel())
    assert all(abs(h - 100) < 1e-9 for _, _, h in Z) and len(Z) > 100
    # kurze Senke: kein Bauwerk
    assert [a for a, _, _ in ebenen.einteilen([(140, 0), (160, 0)], [0, 0], Gel())] == [None]
    # erzwungene Ebene schaltet die Automatik ab
    assert not ebenen.automatik({'bridge': 'yes'}) and not ebenen.automatik({'omsigen:ebene': 'boden'})


def test_projekt_mit_hoehen():
    """Editor-Strasse mit Hoehen -> Wege mit bridge/tunnel und Strassenhoehe auf der Linie"""
    projekt = pipeline.neues_projekt(49.98, 9.14)
    projekt['strassen'] = [dict(id=1, tags={'highway': 'secondary'}, punkte=[[0, 0], [700, 0]], hoehen=[0.0, 0.0])]
    ways, ziele = pipeline.wege(projekt, Gel())
    assert [w['tags'].get('bridge') or w['tags'].get('tunnel') for w in ways] == [None, 'yes', None, 'yes', None]
    net = build(ways, SplineDB())
    hoe = Hoehen(Gel(), log=lambda *a: None).berechnen(net, SplineDB(), (), ziele)
    for ch in net['road_chains']:
        for e, y in zip(ch['els'], ch['y']):
            assert abs(y + hoe.base - 100) < 0.6, (e[0], y + hoe.base)      # gerade, nicht dem Tal folgend
