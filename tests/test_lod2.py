import numpy as np

from omsigen import lod2 as L

GML = """<?xml version="1.0" encoding="UTF-8"?>
<CityModel xmlns:gml="http://www.opengis.net/gml" xmlns:bldg="http://www.opengis.net/citygml/building/1.0" xmlns="http://www.opengis.net/citygml/1.0">
<cityObjectMember><bldg:Building gml:id="DEBY_LOD2_1">
<bldg:function>31001_1000</bldg:function><bldg:roofType>1000</bldg:roofType><bldg:measuredHeight>6.0</bldg:measuredHeight>
<bldg:boundedBy><bldg:WallSurface><bldg:lod2MultiSurface><gml:MultiSurface><gml:surfaceMember><gml:Polygon><gml:exterior><gml:LinearRing>
<gml:posList>1000 2000 136 1010 2000 136 1010 2000 142 1000 2000 142 1000 2000 136</gml:posList>
</gml:LinearRing></gml:exterior></gml:Polygon></gml:surfaceMember></gml:MultiSurface></bldg:lod2MultiSurface></bldg:WallSurface></bldg:boundedBy>
<bldg:boundedBy><bldg:RoofSurface><bldg:lod2MultiSurface><gml:MultiSurface><gml:surfaceMember><gml:Polygon><gml:exterior><gml:LinearRing>
<gml:posList>1000 2000 142 1010 2000 142 1010 2010 142 1000 2010 142 1000 2000 142</gml:posList>
</gml:LinearRing></gml:exterior></gml:Polygon></gml:surfaceMember></gml:MultiSurface></bldg:lod2MultiSurface></bldg:RoofSurface></bldg:boundedBy>
</bldg:Building></cityObjectMember>
<cityObjectMember><bldg:Building gml:id="DEBY_LOD2_2"><bldg:function>53001_1800</bldg:function>
<bldg:boundedBy><bldg:RoofSurface><bldg:lod2MultiSurface><gml:MultiSurface><gml:surfaceMember><gml:Polygon><gml:exterior><gml:LinearRing>
<gml:posList>0 0 1 1 0 1 1 1 1 0 0 1</gml:posList></gml:LinearRing></gml:exterior></gml:Polygon></gml:surfaceMember></gml:MultiSurface></bldg:lod2MultiSurface></bldg:RoofSurface></bldg:boundedBy>
</bldg:Building></cityObjectMember>
</CityModel>"""


def test_lesen_und_bauen(tmp_path):
    p = tmp_path / 'k.gml'
    p.write_text(GML, encoding='utf-8')
    gs = list(L.gebaeude(str(p), dict(ost0=1000, nord0=2000, nn0=130)))
    assert [g['id'] for g in gs] == ['DEBY_LOD2_1']           # Bruecke (53001) bleibt weg
    g = gs[0]
    assert abs(g['boden'] - 6.0) < 1e-9 and np.allclose(g['mitte'], [5.0, 2.5])
    m, o = L.bauen_gebaeude(g)
    assert set(m.mats) == {'wohnen', 'dach_flach'} and len(m.F) == 4
    # Wand reicht 1 m unter den Boden
    assert min(v[1] for v in m.V) == -L.UNTER_BODEN + 6.0


def test_dreiecke_mit_loch():
    aussen = np.array([[0, 0, 5], [10, 0, 5], [10, 10, 5], [0, 10, 5]], float)
    loch = np.array([[4, 4, 5], [4, 6, 5], [6, 6, 5], [6, 4, 5]], float)
    tris, n = L.dreiecke([aussen, loch])
    flaeche = sum(0.5 * abs(np.cross(b - a, c - a)[2]) for a, b, c in (map(np.asarray, t) for t in tris))
    assert abs(flaeche - 96.0) < 1e-6 and n[2] > 0.99


def test_material():
    assert L.wand_material('31001_1000') == 'wohnen' and L.wand_material('31001_3041') == 'sandstein'
    assert L.wand_material('31001_2463') == 'putz' and L.wand_material('31001_2000') == 'gewerbe'


def test_kacheln_im_kreis():
    ks = L.kacheln_im_kreis((150.0, 150.0), 100.0)
    assert ks == [(0, 0)]
    ks = set(L.kacheln_im_kreis((299.0, 150.0), 10.0))
    assert ks == {(0, 0), (1, 0)}


def test_fassade_aus_ablage(tmp_path):
    # Ablage wie earth-reverse-engineering/tools/lod2/korridor.py: Punkte relativ zum Ursprung (UTM), z = NHN, uv glTF
    ordner = tmp_path / 'DEBY_LOD2_1'
    ordner.mkdir()
    (ordner / 'fassade.jpg').write_bytes(b'jpg')
    f = dict(id='DEBY_LOD2_1', ursprung=[1005.0, 2000.0], punkte=[[-5, 0, 136], [5, 0, 136], [5, 0, 142]],
             uv=[[0.0, 1.0], [1.0, 1.0], [1.0, 0.0]], dreiecke=[[0, 1, 2]], textur='fassade.jpg')
    (ordner / 'gebaeude.json').write_text(__import__('json').dumps(f), encoding='utf-8')
    fa = L.fassade_laden('DEBY_LOD2_1', str(tmp_path))
    assert fa is not None and L.fassade_laden('DEBY_LOD2_2', str(tmp_path)) is None
    g = dict(mitte=np.array([5.0, 2.5]))
    m, o = L.bauen_fassade(g, fa, dict(ost0=1000, nord0=2000, nn0=130))
    assert m.mats == ['fassade'] and len(m.F) == 1
    # .x-Koordinaten (x, hoch, vor) relativ zur Grundrissmitte, Hoehe ueber nn0; uv wie glTF (x_file schreibt -v)
    pts = sorted((round(v[0], 3), round(v[1], 3), round(v[2], 3)) for v in m.V)
    assert pts == [(-5.0, 6.0, -2.5), (5.0, 6.0, -2.5), (5.0, 12.0, -2.5)]
    assert sorted((v[6], -v[7]) for v in m.V) == [(0.0, 1.0), (1.0, 0.0), (1.0, 1.0)]


def test_projekt_abbild():
    from omsigen.route import EbeneProjektion, Projection
    from omsigen.utm import to_utm
    e, n = to_utm(49.97, 9.18)
    # UTM-Projekt: Projektmeter = UTM minus Bezugspunkt (ganze Meter)
    proj = Projection(49.97, 9.18)
    x, y = L.projekt_abbild(proj)(np.array([e, e + 100.0]), np.array([n, n]))
    assert abs(x[0] - (e - proj.e0)) < 1e-6 and abs(y[0] - (n - proj.n0)) < 1e-6 and abs(x[1] - x[0] - 100.0) < 1e-6
    # alte ebene Projektion: Umweg ueber lat/lon, 100 m Ost bleiben ~100 m
    alt = EbeneProjektion(49.97, 9.18)
    x, y = L.projekt_abbild(alt)(np.array([e, e + 100.0]), np.array([n, n]))
    assert abs(x[0]) < 0.2 and abs(y[0]) < 0.2 and abs(np.hypot(x[1] - x[0], y[1] - y[0]) - 100.0) < 0.3


def test_gebaeude_nur_in_kartenkacheln(tmp_path):
    from omsigen import writer
    kette = dict(els=[(10.0, 10.0, 90.0, 50.0, 0.0, r'Splines\x.sli')])
    geb = [dict(origin=(40.0, 30.0), rel=r'Sceneryobjects\A\B1.sco'), dict(origin=(5000.0, 5000.0), rel=r'Sceneryobjects\A\B2.sco')]
    info = writer.write_map(str(tmp_path), 'K', [kette], [], gebaeude=geb)
    assert info['gebaeude'] == 1
    text = ''.join(open(f, encoding='utf-16').read() for f in __import__('glob').glob(str(tmp_path / 'K' / 'tile_*.map')))
    assert 'B1.sco' in text and 'B2.sco' not in text
