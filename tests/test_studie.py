import os
import pytest
from omsigen.config import DEFAULT_OMSI
from omsigen.studie import study_map, bericht

GRUNDORF = os.path.join(DEFAULT_OMSI or '', 'maps', 'Grundorf')


@pytest.mark.skipif(not os.path.isdir(GRUNDORF), reason='OMSI 2 mit Grundorf nicht installiert')
def test_grundorf():
    """Grundorf: 10 Kreuzungsobjekte (8 Einmuendungen, 2 Kreuzungen); Hauptstrasse geradeaus hat Vorfahrt."""
    x = study_map(GRUNDORF)
    assert x['kreuzungsobjekte'] == 10 and x['arme'] == {3: 8, 4: 2}
    assert x['vorfahrt_muster'].get('192/gerade/4', 0) > 0 and x['vorfahrt_muster'].get('64/gerade/4', 0) > 0
    assert x['anschluss_abstand'][90] < 0.01
    assert '| Grundorf (Tutorial) |' in bericht([x])
