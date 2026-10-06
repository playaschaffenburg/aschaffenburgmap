"""Datenmodell des Editors: ein omsigen-Projekt (siehe omsigen/pipeline.py) mit Bearbeitungsfunktionen und
Rueckgaengig. Unabhaengig von der Oberflaeche (testbar ohne Qt).

Kreuzungen entstehen automatisch dort, wo Strassen einen gemeinsamen Punkt haben - deshalb rasten neue Punkte an
vorhandenen Punkten ein oder teilen eine vorhandene Strasse an der Klickstelle.

Hoehen: strasse['hoehen'] (optional, parallel zu 'punkte') = Hoehe ueber dem Gelaende je Punkt oder None (folgt dem
Gelaende). Bruecken und Tunnel entstehen daraus automatisch (omsigen/ebenen.py), wie in Transport Fever 2."""
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
        p = (float(p[0]), float(p[1]))                 # keine numpy-Zahlen ins Projekt (JSON)
        q = self.naechster_punkt(p, radius)
        if q:
            return q
        hit = self.naechste_strasse(p, radius * 0.8)
        if hit:
            sid, i, q, _ = hit
            if aendern:
                s = self.strasse(sid)
                s['punkte'].insert(i + 1, [q[0], q[1]])
                if s.get('hoehen'):
                    a, b = s['hoehen'][i], s['hoehen'][i + 1]
                    t = math.dist(s['punkte'][i], q) / max(math.dist(s['punkte'][i], s['punkte'][i + 2]), 1e-9)
                    s['hoehen'].insert(i + 1, None if a is None or b is None else round(a + (b - a) * t, 2))
            return q
        return tuple(p)

    def strasse_hinzufuegen(self, punkte, tags, hoehen=None):
        """punkte: bereits eingerastete Punkte; hoehen: je Punkt Hoehe ueber Gelaende oder None -> neue Strassen-ID"""
        hoehen = list(hoehen) if hoehen is not None else [None] * len(punkte)
        P, H = [[float(v) for v in punkte[0]]], [None if hoehen[0] is None else float(hoehen[0])]
        for a, q, h in zip(punkte, punkte[1:], hoehen[1:]):
            if math.dist(a, q) > 0.05:
                P.append([float(v) for v in q]); H.append(None if h is None else float(h))
        if len(P) < 2:
            return None
        self._merken()
        sid = self._next_id
        self._next_id += 1
        s = dict(id=sid, tags=dict(tags), punkte=P)
        if any(h is not None for h in H):
            s['hoehen'] = H
        self.p['strassen'].append(s)
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
                    q[0], q[1] = float(neu[0]), float(neu[1])

    def punkt_loeschen(self, sid, index):
        s = self.strasse(sid)
        if s and len(s['punkte']) > 2:
            self._merken()
            del s['punkte'][index]
            if s.get('hoehen'):
                del s['hoehen'][index]
            return True
        return False

    def punkt_hoehe(self, p):
        """Hoehe ueber Gelaende am Punkt p (erste Strasse mit Vorgabe) oder None"""
        for s in self.p['strassen']:
            for i, q in enumerate(s['punkte']):
                if math.dist(q, p) < 0.01 and s.get('hoehen') and s['hoehen'][i] is not None:
                    return s['hoehen'][i]
        return None

    def punkt_hoehe_setzen(self, p, h, merken=True):
        """Hoehe ueber Gelaende an allen Strassenpunkten bei p setzen (None = folgt dem Gelaende)"""
        if merken:
            self._merken()
        for s in self.p['strassen']:
            for i, q in enumerate(s['punkte']):
                if math.dist(q, p) < 0.01:
                    if 'hoehen' not in s or len(s['hoehen']) != len(s['punkte']):
                        s['hoehen'] = [None] * len(s['punkte'])
                    s['hoehen'][i] = None if h is None else round(float(h), 2)
            if s.get('hoehen') is not None and all(v is None for v in s['hoehen']):
                del s['hoehen']

    def umkehren(self, sid):
        s = self.strasse(sid)
        if s:
            self._merken()
            s['punkte'].reverse()
            if s.get('hoehen'):
                s['hoehen'].reverse()


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
