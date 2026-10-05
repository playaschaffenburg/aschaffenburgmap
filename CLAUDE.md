# Hinweise für Claude Code

Projekt: `omsigen` erzeugt OMSI-2-Karten aus OpenStreetMap (siehe README.md). Sprache im Projekt: Deutsch
(Ausgaben, Doku, Kommentare). Nutzer: spielt OMSI 2, baut Aschaffenburg nach, testet im nEditor und im Spiel.

## Arbeitsweise

- **Nie bestehende Karten oder OMSI-Dateien verändern.** Neue Karten immer in einen neuen Ordner
  (`--name …_v2` usw.). Eigene Splines nur in `Splines/Aschaffenburg_KI/`.
- Nach jeder Änderung: `python -m pytest` und einen Beispiellauf mit `--osm-datei samples/...` und
  `--vorschau`. Ziel: „ohne Anschluss: 0“. Das PNG ansehen.
- Kontrolle im nEditor (`C:\Users\ewanh\Documents\nEditor\nEditor.exe`): Karte wählen → Start → „Show paths“.
  Neue `.sli`-Dateien erkennt der nEditor erst nach Neustart. Der nEditor hat einen eigenen Spielstand – neue
  Karten-Versionen deshalb unter neuem Namen laden.
- Erst im Spiel testen lassen, wenn Prüfung und nEditor sauber sind.

## Wichtige Fakten zum Format (Details: docs/omsi-format.md)

- Kacheln 300 m, Richtung im Uhrzeigersinn ab Nord, Radius > 0 = Rechtskurve, positive Querlage = rechts.
- Spline-Zeilen: x, **Höhe**, z; Objekt-Zeilen: x, z, **Höhe**.
- `.map`/`global.cfg` als UTF-16 LE mit BOM schreiben.
- Verbindungen entstehen nur, wenn Spurenden in Lage und Richtung zusammenfallen → Kreuzungsspuren.

## Pfad-Konventionen beim Nutzer

- OMSI: `C:\Program Files (x86)\Steam\steamapps\common\OMSI 2`
- Vorlage für neue Karten: `OMSI 2\template\NewMap`
- Standardsplines: `OMSI 2\Splines\Marcel` (Spurdaten als Ersatz in `config.FALLBACK_SPLINES`)

## Nächste Schritte (Roadmap im README)

Vorfahrt/Ampeln → Fußwege an Kreuzungen → Höhen → Haltestellen/Linien/Fahrpläne aus GTFS → Gebäude.
