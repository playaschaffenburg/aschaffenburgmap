# aschaffenburgmap – reale Strecken als OMSI-2-Karte

`omsigen` baut aus OpenStreetMap-Daten eine befahrbare OMSI-2-Karte: Man gibt eine Stadt, Start und Ziel ein,
das Tool sucht die Strecke über das echte Straßennetz und erzeugt alle Straßen in einem Korridor darum –
mit Kreuzungen, deren Fahrspuren exakt verbunden sind.

![Beispiel: Aschaffenburg Hbf – City Galerie](docs/beispiel_hbf_citygalerie.png)

## Schnellstart (Windows)

```powershell
pip install -r requirements.txt
python -m omsigen --omsi "C:\Program Files (x86)\Steam\steamapps\common\OMSI 2" `
    --stadt Aschaffenburg --von "Hauptbahnhof" --nach "City Galerie" `
    --name Aschaffenburg_Hbf_CityGalerie --vorschau vorschau.png
```

Die Karte erscheint danach in OMSI bzw. im nEditor unter dem angegebenen Namen. Vorhandene Karten werden nie
überschrieben (außer mit `--ueberschreiben` für denselben Namen).

| Option | Bedeutung |
|---|---|
| `--von`, `--nach` | Ortsname (mit `--stadt`) oder Koordinaten `"49.9805,9.1402"` |
| `--ueber` | Zwischenziel, mehrfach möglich – damit lässt sich eine Buslinie nachfahren |
| `--breite` | Korridor links/rechts der Strecke in Metern (Standard 150) |
| `--omsi` | OMSI-Ordner: Karte und eigene Splines werden direkt installiert |
| `--ausgabe` | ohne `--omsi`: Ausgabe in diesen Ordner (Standard `build/`) |
| `--osm-datei` | Daten aus Datei statt Download, z. B. `samples/aschaffenburg_hbf_citygalerie.json` |

Ohne Internet/OMSI ausprobieren:

```bash
python -m omsigen --von "49.98053,9.14023" --nach "49.97790,9.14998" --name Test \
    --osm-datei samples/aschaffenburg_hbf_citygalerie.json --vorschau vorschau.png
python -m pytest
```

## Was das Tool kann

- Strecke über das echte Straßennetz (Einbahnstraßen beachtet), Korridor drumherum
- Straßen aus Geraden und Kreisbögen, Spurenzahl und Einbahnstraßen aus OSM, getrennte Richtungsfahrbahnen
- **Kreuzungsbauer**: alle Arme werden gekürzt, die Kreuzung mit Abbiege- und Geradeausspuren gefüllt,
  die die Fahrspuren exakt verbinden; Rechtsabbieger mit Gehwegecke; Kreisverkehre
- Haltestellenschilder an den OSM-Haltestellen
- automatische Prüfung jedes Spurendes + Draufsicht als PNG

## Noch offen (Roadmap)

1. Vorfahrt / Ampeln an Kreuzungen
2. Fußwege an Kreuzungen verbinden, Zebrastreifen
3. Höhen (SRTM bzw. amtliches DGM)
4. Haltestellen mit Haltepunkten, Linien und Fahrplänen (aus GTFS-Daten)
5. Gebäude (OSM-Umrisse, ggf. LoD2-Modelle), Bäume, Straßenmöbel
6. Tunnel und Brücken (werden derzeit weggelassen)

## Projektaufbau

```
omsigen/
  cli.py            Kommandozeile, Ablauf in 6 Schritten
  osm.py            Ortssuche (Nominatim), Download (Overpass), Zwischenspeicher
  route.py          Projektion, Routing (Dijkstra), Korridor
  network.py        Kanten, Knoten, Kreisverkehre, Kürzen, Ketten, Kreuzungsbauer
  geom.py           Geraden/Bögen/Doppelbögen in OMSI-Konvention
  config.py         welche Spline für welche Straße
  custom_splines.py eigene .sli (Einbahnstraßen, Kreuzungsspuren)
  splinedb.py       Fahrspuren aus .sli lesen
  writer.py         Kartenordner schreiben
  check.py          Spurprüfung und Vorschau
docs/omsi-format.md alles, was wir über das Dateiformat wissen
samples/            Testdaten (OSM-Auszug Aschaffenburg)
```

Benötigt die Standardinhalte von OMSI 2 (Splines `Marcel`, `template/NewMap`); diese werden nicht mitgeliefert.

## Daten und Lizenz

Straßendaten © OpenStreetMap-Mitwirkende, verfügbar unter der
[Open Database License](https://www.openstreetmap.org/copyright). Erzeugte Karten enthalten einen
entsprechenden Hinweis; beim Weitergeben bitte beibehalten.
