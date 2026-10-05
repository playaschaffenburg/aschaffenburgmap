"""Liest Fahrspuren und Breiten aus .sli-Dateien (OMSI-Spline-Definitionen)."""
import os
from .config import FALLBACK_SPLINES, KI
from .custom_splines import SPLINES as CUSTOM


def read_text(path):
    b = open(path, 'rb').read()
    return b.decode('utf-16') if b[:2] == b'\xff\xfe' else b.decode('cp1252')


def parse_sli(txt):
    """-> dict(lanes=[(querlage, richtung)], cw=halbe Fahrbahnbreite, half=halbe Gesamtbreite)"""
    L = [l.strip() for l in txt.replace('\r', '').split('\n')]
    lanes, asph, total = [], [], []
    for i, l in enumerate(L):
        if l == '[path]' and L[i + 1] == '0':                  # Typ 0 = Fahrzeugpfad
            lanes.append((float(L[i + 2]), int(L[i + 5])))
        if l == '[heightprofile]':
            a, b, h1 = float(L[i + 1]), float(L[i + 2]), float(L[i + 3])
            total += [a, b]
            if h1 < 0.2:                                      # Fahrbahnhoehe (Gehweg liegt hoeher)
                asph += [a, b]
    return dict(lanes=lanes,
                cw=max(abs(min(asph)), abs(max(asph))) if asph else 3.0,
                half=max(abs(min(total)), abs(max(total))) if total else 7.0)


class SplineDB(dict):
    """Splinepfad (wie in .map-Dateien, z. B. 'Splines\\Marcel\\x.sli') -> Spurinfo"""

    def __init__(self, omsi_dir=None):
        super().__init__()
        self.update(FALLBACK_SPLINES)
        for n, (t, _) in CUSTOM.items():
            self[KI + n] = parse_sli(t)
        self.omsi_dir = omsi_dir

    def __missing__(self, rel):
        if self.omsi_dir:
            p = os.path.join(self.omsi_dir, *rel.split('\\'))
            if os.path.exists(p):
                self[rel] = parse_sli(read_text(p))
                return self[rel]
        raise KeyError(f'Spline {rel} unbekannt (OMSI-Ordner mit --omsi angeben)')
