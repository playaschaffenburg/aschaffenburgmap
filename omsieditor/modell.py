"""Datenmodell des Editors: ein omsigen-Projekt (siehe omsigen/pipeline.py) mit Bearbeitungsfunktionen und
Rueckgaengig. Unabhaengig von der Oberflaeche (testbar ohne Qt).

Kreuzungen entstehen automatisch dort, wo Strassen einen gemeinsamen Punkt haben - deshalb rasten neue Punkte an
vorhandenen Punkten ein oder teilen eine vorhandene Strasse an der Klickstelle."""
import copy, math
from omsigen.pipeline import neues_projekt, laden, speichern
from omsigen.network import proj_point

KLASSEN = ['primary', 'secondary', 'tertiary', 'unclassified', 'residential', 'living_street', 'service', 'bus']
KLASSEN_TEXT = {'primary': 'Hauptstrasse (primary)', 'secondary': 'Hauptstrasse (secondary)',
                'tertiary': 'Verbindungsstrasse (tertiary)', 'unclassified': 'Nebenstrasse (unclassified)',
                'residential': 'Wohnstrasse (residential)', 'living_street': 'Verkehrsberuhigt (living_street)',
                'service': 'Zufahrt (service)', 'bus': 'Busspur/Busbahnhof (bus)'}


class Modell:
    def __init__(self, projekt, pfad=None):
        self.p = projekt
        self.pfad = pfad
        self._undo, self._redo = [], []
        self.geaendert = False
        self._next_id = max((s['id'] for s in self.p['strassen']), default=0) + 1

    # ---------------------------------------------------------------- Dateien
    @classmethod
    def neu(cls, lat, lon, name='Neues Projekt'):
        return cls(neues_projekt(lat, lon, name))

    @classmethod
    def oeffnen(cls, pfad):
        return cls(laden(pfad), pfad)

    def speichern(self, pfad=None):
        self.pfad = pfad or self.pfad
        speichern(self.p, self.pfad)
        self.geaendert = False

    # ---------------------------------------------------------------- Rueckgaengig
    def _merken(self):
        self._undo.append(copy.deepcopy(self.p['strassen']))
        del self._undo[:-200]
        self._redo.clear()
        self.geaendert = True

    def rueckgaengig(self):
        if not self._undo:
            return False
        self._redo.append(copy.deepcopy(self.p['strassen']))
        self.p['strassen'] = self._undo.pop()
        self.geaendert = True
        return True

    def wiederholen(self):
        if not self._redo:
            return False
        self._undo.append(copy.deepcopy(self.p['strassen']))
        self.p['strassen'] = self._redo.pop()
        self.geaendert = True
        return True

    # ---------------------------------------------------------------- Abfragen
    def strasse(self, sid):
        return next((s for s in self.p['strassen'] if s['id'] == sid), None)

    def naechster_punkt(self, p, radius, ausser=None):
        """naechster vorhandener Strassenpunkt in radius -> (x, z) oder None"""
        best = None
        for s in self.p['strassen']:
            if s['id'] == ausser:
                continue
            for q in s['punkte']:
                d = math.dist(p, q)
                if d <= radius and (best is None or d < best[0]):
                    best = (d, tuple(q))
        return best[1] if best else None

    def naechste_strasse(self, p, radius):
        """Strasse unter p -> (id, Segmentindex, Punkt auf dem Segment, Abstand) oder None"""
        best = None
        for s in self.p['strassen']:
            P = s['punkte']
            for i in range(len(P) - 1):
                t, d = proj_point(p, P[i], P[i + 1])
                if d <= radius and (best is None or d < best[3]):
                    q = (P[i][0] + (P[i + 1][0] - P[i][0]) * t, P[i][1] + (P[i + 1][1] - P[i][1]) * t)
                    best = (s['id'], i, q, d)
        return best

    def kreuzungspunkte(self):
        """Punkte, an denen sich mindestens 3 Strassenaeste treffen (fuer die Anzeige)"""
        n = {}
        for s in self.p['strassen']:
            P = s['punkte']
            for i, q in enumerate(P):
                k = (round(q[0], 2), round(q[1], 2))
                n[k] = n.get(k, 0) + (1 if i in (0, len(P) - 1) else 2)
        return [k for k, v in n.items() if v >= 3]

    # ---------------------------------------------------------------- Bearbeiten
    def einrasten(self, p, radius, aendern=True):
        """Punkt fuer eine neue Strasse: vorhandener Punkt in radius, sonst Punkt auf einer Strasse (diese bekommt
        dort einen Stuetzpunkt, damit eine Kreuzung entsteht), sonst p selbst -> (x, z)"""
        q = self.naechster_punkt(p, radius)
        if q:
            return q
        hit = self.naechste_strasse(p, radius * 0.8)
        if hit:
            sid, i, q, _ = hit
            if aendern:
                self.strasse(sid)['punkte'].insert(i + 1, [q[0], q[1]])
            return q
        return tuple(p)

    def strasse_hinzufuegen(self, punkte, tags):
        """punkte: bereits eingerastete Punkte -> neue Strassen-ID"""
        P = [list(punkte[0])] + [list(q) for a, q in zip(punkte, punkte[1:]) if math.dist(a, q) > 0.05]
        if len(P) < 2:
            return None
        self._merken()
        sid = self._next_id
        self._next_id += 1
        self.p['strassen'].append(dict(id=sid, tags=dict(tags), punkte=P))
        return sid

    def strasse_loeschen(self, sid):
        s = self.strasse(sid)
        if s:
            self._merken()
            self.p['strassen'].remove(s)

    def tags_setzen(self, sid, tags):
        s = self.strasse(sid)
        if s and s['tags'] != tags:
            self._merken()
            s['tags'] = {k: v for k, v in tags.items() if v not in (None, '')}

    def punkt_verschieben(self, alt, neu, merken=True):
        """alle Strassenpunkte an der Stelle alt (gemeinsamer Kreuzungspunkt) nach neu verschieben"""
        if merken:
            self._merken()
        for s in self.p['strassen']:
            for q in s['punkte']:
                if math.dist(q, alt) < 0.01:
                    q[0], q[1] = neu[0], neu[1]

    def punkt_loeschen(self, sid, index):
        s = self.strasse(sid)
        if s and len(s['punkte']) > 2:
            self._merken()
            del s['punkte'][index]
            return True
        return False

    def umkehren(self, sid):
        s = self.strasse(sid)
        if s:
            self._merken()
            s['punkte'].reverse()


def standard_tags(klasse='residential', name='', einbahn='nein', spuren=0, tempo=''):
    t = dict(highway='service' if klasse == 'bus' else klasse)
    if klasse == 'bus':
        t.update(psv='designated', access='no')
    if name:
        t['name'] = name
    if einbahn == 'ja':
        t['oneway'] = 'yes'
    elif einbahn == 'entgegen':
        t['oneway'] = '-1'
    if spuren:
        t['lanes'] = str(spuren)
    if tempo:
        t['maxspeed'] = str(tempo)
    return t


def klasse_von(tags):
    if tags.get('highway') in ('service', 'busway') and (tags.get('psv') or tags.get('bus')):
        return 'bus'
    return tags.get('highway', 'residential')
