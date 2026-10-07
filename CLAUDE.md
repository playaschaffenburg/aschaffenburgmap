# Hinweise für Claude Code

Projekt: `omsigen` erzeugt OMSI-2-Karten aus OpenStreetMap (siehe README.md). Sprache im Projekt: Deutsch
(Ausgaben, Doku, Kommentare). Nutzer: spielt OMSI 2, baut Aschaffenburg nach, testet im nEditor und im Spiel.

## Arbeitsweise

- **Nie bestehende Karten oder OMSI-Dateien verändern.** Neue Karten immer in einen neuen Ordner
  (`--name …_v2` usw.). Ausnahme auf Wunsch des Nutzers: „Speichern“ im Rust-Editor überschreibt die geöffnete
  Karte (eigene ohne Rückfrage, fremde nach Rückfrage), immer mit Sicherung der ersetzten Dateien. Eigene Splines nur in `Splines/Aschaffenburg_KI/`, eigene Objekte
  (Kreuzungen) nur in `Sceneryobjects/Aschaffenburg_KI/<Kartenname>/`.
- Nach jeder Änderung: `python -m pytest` und einen Beispiellauf mit `--osm-datei samples/...` und
  `--vorschau`. Ziel: „ohne Anschluss: 0“. Das PNG ansehen.
- Zusätzlich `python -m omsigen.ansicht <Karte> --png …` (Ziel: Beinahe-Anschlüsse 0) und für installierte
  Karten `python -m omsigen.openomsi <Karte> --png … --cam …` (Spielbild aus openOMSI). Bilder ansehen.
- Standardkarten (Grundorf, Berlin-Spandau) sind die Referenz: mit `omsigen.ansicht` nachsehen, wie OMSI etwas
  baut, bevor wir es nachbauen. Format-Wissen aus openOMSI (`Documents\OpenOmsi\source`) in
  docs/omsi-format.md immer mit Quelle eintragen.
- Kontrolle im nEditor (`C:\Users\ewanh\Documents\nEditor\nEditor.exe`): Karte wählen → Start → „Show paths“.
  Neue `.sli`-Dateien erkennt der nEditor erst nach Neustart. Start nur über `nEditor-downloader` → „nEditor
  starten“ (direkt gestartet: Lizenzfehler 409). Der nEditor hat einen eigenen Spielstand – neue
  Karten-Versionen deshalb unter neuem Namen laden.
- Erst im Spiel testen lassen, wenn Prüfung und nEditor sauber sind.

## Wichtige Fakten zum Format (Details: docs/omsi-format.md)

- Kacheln 300 m, Richtung im Uhrzeigersinn ab Nord, Radius > 0 = Rechtskurve, positive Querlage = rechts.
- Spline-Zeilen: x, **Höhe**, z; Objekt-Zeilen: x, z, **Höhe**.
- `.map`/`global.cfg` als UTF-16 LE mit BOM schreiben.
- Verbindungen entstehen nur, wenn Spurenden in Lage und Richtung zusammenfallen.
- Kreuzungen sind eigene Objekte (`omsigen/kreuzung.py`: .sco + .x-Modell + Pfade), wie in den Standardkarten.
  Nur so erkennt die KI Vorfahrtskonflikte, und nur dort gibt es Ampeln.

## Karteneditor (`omsieditor/`, PySide6)

Bearbeitet nur das Projekt (`.omsiprojekt`, Format in `omsigen/pipeline.py`), nie OMSI-Dateien direkt; die Karte
entsteht immer über `pipeline.erzeuge()`. Luftbild: Bayern DOP40-WMS (CC BY 4.0, Quellenangabe in der Statuszeile
behalten). Tests laufen ohne Bildschirm (`QT_QPA_PLATFORM=offscreen`, `OMSIEDITOR_OHNE_LUFTBILD=1`).
3D-Hauptansicht (OpenGL 3.3 über PyOpenGL) zeigt das Ergebnis von `pipeline.berechne()` – dieselbe Geometrie,
die OMSI bekommt; zum Prüfen ohne Fenster `python -m omsieditor.bild3d <projekt> --png …` und Bild ansehen.
**Jede neue Generator-Funktion muss auch im Editor bedienbar und sichtbar sein** (Hauptprodukt ist der Editor).
Höhen je Punkt (`strasse['hoehen']`) → Brücken/Tunnel automatisch (`omsigen/ebenen.py`, wie Transport Fever 2).
Etappen: 1 Strassen (fertig) → 1b 3D + Brücken/Tunnel (fertig) → 2 Kreuzungswerkzeug (Vorfahrt/Ampel per Klick;
im Rust-Editor fertig) → 3 Haltestellen/Linien.

## Vorfahrt

Regelwerk in `omsigen/vorfahrt.py` (Reihenfolge: Korrektur, Kreisel, Schild, Vorfahrtstraße, StVO, Vermutung).
Vermutete Kreuzungen in der HTML-Ansicht orange; Nutzer korrigiert über `korrekturen/*.json`. Muster der
Prioritäten aus der Kartenstudie (docs/kartenstudie.md), nicht ändern ohne neue Auswertung.

## Pfad-Konventionen beim Nutzer

- OMSI: `C:\Program Files (x86)\Steam\steamapps\common\OMSI 2`
- Vorlage für neue Karten: `OMSI 2\template\NewMap`
- Standardsplines: `OMSI 2\Splines\Marcel` (Spurdaten als Ersatz in `config.FALLBACK_SPLINES`)

## Nächste Schritte (Roadmap im README)

Fußgängerampeln/-querungen → Haltestellen/Linien/Fahrpläne aus GTFS → Gebäude.
