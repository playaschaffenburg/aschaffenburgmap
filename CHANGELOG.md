# OMSIgen - Versionen

Release-Staende (PROD). Entwickelt wird in TEST (Zweig main).

## Unveroeffentlicht (TEST, seit v0.1.0)

Qualitaetsoffensive fuer die Pilotkarte Linie 16 (Plan: docs/qualitaet-anforderungen.md, Massstab: docs/musterbuch.md).

- **Fundament (Phase 0):** Generator rechnet im UTM-Gitter (omsi-editor-geo.cfg); Schichten je Kachel mit Manifest und
  Erkennung veralteter Schichten; netz.json je Karte; Belegungskarte (Fahrbahn, Gehweg, Gebaeude ...); Pruefbericht je
  Kachel mit Budgets, Spiel- und Luftbildern, Anzeige im Editor.
- **LoD2-Gebaeude** mit Fassadenfotos im Korridor (`--gebaeude`), DDS-Texturen.
- **Strassen und Boden (Phase 1):** echte Querschnitte aus OSM-Tags und Gebaeudeflucht, Abbiegestreifen nach
  `turn:lanes`, Querschnittsuebergaenge; .sli vom Editor-Baukasten (`omsi-editor --querschnitte`). Malmasken je Kachel
  (Gras, Erde, Schotter, Pflaster, Asphalt, Wasser) aus Luftbild, OSM und Belegung; Boeschungen statt Stufen.
- **Gebaeude und Objekte (Phase 2):** Gebaeude auf dem Kartengelaende gegruendet und nach Hauptachse gedreht,
  Konflikte mit Hinweis im Pruefbericht; Strassenhoehen exakt geloest. Strassenausstattung (Laternen, Verkehrszeichen
  passend zur Vorfahrt, Poller, Baenke, Muelleimer, Zaeune) und Haltestellen (Mast mit Namensschild, Wartehalle,
  Hochbord) als Schicht, im Editor je Kachel neu baubar.
- **Strassenlogik nach Vorbild:** Vorbildstudie Ahlheim 5 / Staedtedreieck V3 (`omsigen.vorbildstudie`, Musterbuch).
  Kreuzungen am Gefaelle: Platten als geneigte Ebene, Arme ohne Knick, stetige Steigung mit `[spline_h]`
  (Knick an Platten 338 -> 1 von 638 Armen).
- **Gelaende unter der Strasse:** Rasterpunkte neben der Strasse liegen hoechstens auf Strassenhoehe (OMSI-Dreiecke hoben die Bodentextur ueber Gehweg und Fahrbahn; 438 -> 28 Elemente); neue Pruefung `omsigen.gelaende_pruefung`.
- **Trassierung:** OSM-Linien entzittert, Kurven mit dem echten Radius statt 15-m-Ecken (Median 15 -> 68 m).
- Fehlerbehebungen: netz.json las `[spline_h]` falsch; Pruefbericht stuerzte bei Add-on-Modellen ab.

## v0.1.0 (2026-10-12)

Erstes Release: Karteneditor mit Strassen, Kreuzungen, Sonderbau, Vermessen, LoD2-Import; Editor findet sein Python selbst (Luftbild ging mit leerem Python 3.15 im PATH nicht)
