"""Kommandozeile: eine reale Strecke als OMSI-2-Karte erzeugen.

Beispiel:
  python -m omsigen --omsi "C:/Program Files (x86)/Steam/steamapps/common/OMSI 2" \\
      --stadt Aschaffenburg --von "Hauptbahnhof" --nach "City Galerie" --name Aschaffenburg_Test
"""
import argparse, sys
from . import pipeline


def main(argv=None):
    ap = argparse.ArgumentParser(prog='omsigen', description='Reale Strecke -> OMSI-2-Karte (Strassennetz mit Kreuzungen)')
    ap.add_argument('--stadt', help='Stadt, in der gesucht wird (z. B. Aschaffenburg)')
    ap.add_argument('--von', required=True, help='Startpunkt: Ortsname oder "lat,lon"')
    ap.add_argument('--nach', required=True, help='Ziel: Ortsname oder "lat,lon"')
    ap.add_argument('--ueber', action='append', default=[], help='Zwischenziel (mehrfach moeglich)')
    ap.add_argument('--breite', type=float, default=150, help='Korridor links/rechts der Strecke in m (Standard 150)')
    ap.add_argument('--name', required=True, help='Name des neuen Kartenordners (z. B. Aschaffenburg_Linie1)')
    ap.add_argument('--titel', help='Anzeigename der Karte in OMSI')
    ap.add_argument('--omsi', help='OMSI-2-Ordner: Karte wird direkt dort installiert')
    ap.add_argument('--ausgabe', default='build', help='Ausgabeordner, falls --omsi fehlt (Standard: build)')
    ap.add_argument('--osm-datei', help='OSM-Daten aus Datei statt Download (siehe samples/)')
    ap.add_argument('--cache', default='.cache', help='Zwischenspeicher fuer Downloads')
    ap.add_argument('--ueberschreiben', action='store_true', help='vorhandenen Kartenordner mit gleichem Namen ersetzen')
    ap.add_argument('--vorschau', default=None, help='PNG-Draufsicht hierhin schreiben')
    ap.add_argument('--ansicht', default=None, help='interaktive HTML-Ansicht aller Pfade hierhin schreiben')
    ap.add_argument('--korrekturen', help='JSON-Datei mit Korrekturen (Vorfahrt), siehe korrekturen/beispiel.json')
    ap.add_argument('--kreuzungen', choices=['objekt', 'spline'], default='objekt',
                    help='Kreuzungen als eigene Objekte mit Flaeche (Standard) oder nur aus Spur-Splines (alt)')
    a = ap.parse_args(argv)
    projekt = pipeline.importiere(a.von, a.nach, a.stadt, a.ueber, a.breite, a.cache, a.osm_datei)
    r = pipeline.erzeuge(projekt, a.name, omsi=a.omsi, ausgabe=a.ausgabe, korrekturen=a.korrekturen,
                         ueberschreiben=a.ueberschreiben, kreuzungen=a.kreuzungen, titel=a.titel,
                         vorschau=a.vorschau, ansicht_html=a.ansicht)
    return r['rc']


if __name__ == '__main__':
    sys.exit(main())
