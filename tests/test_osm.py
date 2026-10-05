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
