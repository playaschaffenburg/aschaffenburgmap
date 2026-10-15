"""F1 Geobezug: Generator-Projekte rechnen im UTM-Gitter und schreiben omsi-editor-geo.cfg wie der Rust-Editor."""
from omsigen import pipeline
from omsigen.fotovergleich import bezug_lesen
from omsigen.route import EbeneProjektion, Projection, projektion_fuer
from omsigen.utm import to_utm


def test_projektion_ist_utm():
    p = Projection(49.9797, 9.1481)
    e, n = to_utm(49.9797, 9.1481)
    assert p.zone == 32 and p.e0 == round(e) and p.n0 == round(n)
    x, z = p.to_m(49.9797, 9.1481)
    assert abs(x - (e - p.e0)) < 1e-6 and abs(z - (n - p.n0)) < 1e-6
    lat, lon = p.to_ll(1234.5, -678.9)                         # hin und zurueck genau
    x2, z2 = p.to_m(lat, lon)
    assert abs(x2 - 1234.5) < 1e-3 and abs(z2 + 678.9) < 1e-3
    assert p.utm(10.0, 20.0) == (p.e0 + 10.0, p.n0 + 20.0)


def test_neue_projekte_utm_alte_eben():
    neu = pipeline.neues_projekt(49.98, 9.14)
    assert neu['projektion'] == 'utm' and isinstance(projektion_fuer(neu), Projection)
    alt = dict(ursprung=[49.98, 9.14], strassen=[])            # Datei von vor Okt. 2026
    assert isinstance(projektion_fuer(alt), EbeneProjektion)


def test_geo_datei(tmp_path):
    p = Projection(49.9797, 9.1481)
    pipeline.geo_schreiben(str(tmp_path), p, (-600.0, -300.0), 121.0, 'Linie 16\nTest')
    raw = (tmp_path / 'omsi-editor-geo.cfg').read_bytes()
    assert b'\r\n' in raw and b'ort = Linie 16 Test' in raw
    b = bezug_lesen(str(tmp_path))
    assert b == dict(ost0=p.e0 - 600.0, nord0=p.n0 - 300.0, zone=32, nn0=121.0)
    # ein Kartenpunkt (Projektmeter minus Versatz) ist UTM minus ost0/nord0
    x, z = p.to_m(49.9797, 9.1481)
    e, n = to_utm(49.9797, 9.1481)
    assert abs((x + 600.0) - (e - b['ost0'])) < 1e-6 and abs((z + 300.0) - (n - b['nord0'])) < 1e-6
