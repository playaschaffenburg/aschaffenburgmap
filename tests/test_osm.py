from omsigen.osm import normalize, geocode


def test_geocode_coordinates():
    assert geocode('49.98053, 9.14023') == (49.98053, 9.14023)


def test_normalize_filters():
    js = {'elements': [
        {'type': 'way', 'id': 1, 'tags': {'highway': 'secondary', 'name': 'A'}, 'geometry': [{'lat': 0, 'lon': 0}, {'lat': 0, 'lon': .001}]},
        {'type': 'way', 'id': 2, 'tags': {'highway': 'footway'}, 'geometry': [{'lat': 0, 'lon': 0}, {'lat': 0, 'lon': .001}]},
        {'type': 'way', 'id': 3, 'tags': {'highway': 'service', 'access': 'no', 'psv': 'yes'}, 'geometry': [{'lat': 0, 'lon': 0}, {'lat': 0, 'lon': .001}]},
        {'type': 'way', 'id': 4, 'tags': {'highway': 'service', 'service': 'parking_aisle'}, 'geometry': [{'lat': 0, 'lon': 0}, {'lat': 0, 'lon': .001}]},
        {'type': 'node', 'id': 5, 'lat': 0, 'lon': 0, 'tags': {'highway': 'bus_stop', 'name': 'Hbf'}},
    ]}
    d = normalize(js)
    assert [w['id'] for w in d['ways']] == [1, 3]
    assert d['ways'][1]['tags']['highway'] == 'bus'
    assert d['stops'][0]['name'] == 'Hbf'


def _r(lat, lon, cls, typ, name):
    return dict(lat=str(lat), lon=str(lon), osm_type='node', osm_id=hash((lat, lon, name)), name=name,
                display_name=f'{name}, Aschaffenburg', **{'class': cls, 'type': typ})


# echte Nominatim-Treffer (Oktober 2026) fuer "Hauptbahnhof, Aschaffenburg" und "hbf, Aschaffenburg"
HBF = [_r(49.979708, 9.148063, 'amenity', 'parking', 'Parkhaus Am Hauptbahnhof'),
       _r(49.980665, 9.143881, 'railway', 'station', 'Aschaffenburg Hbf'),
       _r(49.980320, 9.143962, 'railway', 'platform', 'Aschaffenburg Hbf'),
       _r(49.980291, 9.149645, 'railway', 'signal_box', 'Aschaffenburg Hbf'),
       _r(49.980326, 9.140783, 'amenity', 'bus_station', 'Hbf/ROB'),
       _r(49.980204, 9.144452, 'amenity', 'parking', 'Parkhaus Aschaffenburg Hbf')]
CITY_GALERIE = [_r(49.977974, 9.151321, 'shop', 'mall', 'City Galerie'),
                _r(49.978684, 9.152344, 'amenity', 'parking', 'City Galerie'),
                _r(49.977556, 9.149336, 'highway', 'bus_stop', 'City Galerie (Fernbushaltestelle)'),
                _r(49.977878, 9.150050, 'highway', 'bus_stop', 'City Galerie')]


def test_varianten_hauptbahnhof():
    from omsigen.osm import _varianten
    assert _varianten('Hauptbahnhof') == ['hauptbahnhof', 'hbf']
    assert _varianten('Hbf') == ['hbf', 'hauptbahnhof']
    assert _varianten('Goldbacher Straße') == ['goldbacher straße', 'goldbacher str']


def test_rank_hauptbahnhof_nicht_parkhaus():
    """Frueher wurde 'Hauptbahnhof' zum Parkhaus Am Hauptbahnhof (330 m daneben)."""
    from omsigen.osm import rank_candidates
    best = rank_candidates(HBF, 'Hauptbahnhof', 'Aschaffenburg')[0]
    assert (best['class'], best['type'], best['name']) == ('railway', 'station', 'Aschaffenburg Hbf')


def test_rank_city_galerie_haltestelle():
    from omsigen.osm import rank_candidates
    best = rank_candidates(CITY_GALERIE, 'City Galerie', 'Aschaffenburg')[0]
    assert (best['type'], best['name'], best['lon']) == ('bus_stop', 'City Galerie', '9.15005')


def test_find_place_fragt_varianten(monkeypatch):
    import omsigen.osm as osm
    gefragt = []

    class Resp:
        def __init__(self, js): self.js = js
        def raise_for_status(self): pass
        def json(self): return self.js

    class Req:
        @staticmethod
        def get(url, params, headers, timeout):
            gefragt.append(params['q'])
            return Resp(HBF[:1] if params['q'].startswith('hauptbahnhof') else HBF[1:])
    monkeypatch.setattr(osm, '_requests', lambda: Req)
    monkeypatch.setattr(osm.time, 'sleep', lambda s: None)
    p = osm.find_place('Hauptbahnhof', 'Aschaffenburg')
    assert gefragt == ['hauptbahnhof, Aschaffenburg', 'hbf, Aschaffenburg']
    assert abs(p['lat'] - 49.9807) < 0.0005 and abs(p['lon'] - 9.1439) < 0.0005
