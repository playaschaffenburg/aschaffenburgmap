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

## Karten prüfen und ansehen

```powershell
# beliebige Karte lesen (eigene oder Standardkarten), Anschlüsse prüfen, Draufsicht + interaktive Ansicht
python -m omsigen.ansicht Aschaffenburg_Live_v1 --png build/ansicht.png --html build/ansicht.html
python -m omsigen.ansicht Grundorf --png build/kreuzung.png --bereich=340,-230,420,-150

# Karte im Nachbau openOMSI laden und ein Spielbild rendern (Kamera: x,y,z,gier,neigung in Kartenmetern)
python -m omsigen.openomsi Aschaffenburg_Live_v1 --png build/spiel.png --cam 390,320,60,0,-40
```

`omsigen.ansicht` ersetzt „Show paths“ im nEditor: Fahrspuren, Gehwege, Vorfahrt-Regeln, Ampelpfade und
Kreuzungsobjekte; Klick auf eine Spur zeigt Spline/Objekt, Datei, Kachel und Pfad-Index. Gemeldet werden
„Beinahe-Anschlüsse“ (Spurenden mit Partner in ≤ 5 m, aber nicht exakt verbunden). Beim Generator erzeugt
`--ansicht build/x.html` die Ansicht gleich mit. `omsigen.openomsi` braucht
[openOMSI](https://github.com/openOMSI-Project/openOMSI) unter `Documents\OpenOmsi`.

## Was das Tool kann

- Strecke über das echte Straßennetz (Einbahnstraßen beachtet), Korridor drumherum
- Straßen aus Geraden und Kreisbögen, Spurenzahl und Einbahnstraßen aus OSM, getrennte Richtungsfahrbahnen
- **Kreuzungsobjekte**: jede Kreuzung wird ein eigenes OMSI-Objekt wie in den Standardkarten – eine
  durchgehende Platte (.x-Modell) aus Asphalt, Bordsteinen und gerundeten Gehwegecken, dazu die Abbiege-,
  Geradeaus- und Gehwegpfade, die exakt an die Straßen anschließen; Mittelinseln bei getrennten Fahrbahnen,
  Kreisverkehre (`--kreuzungen spline` baut noch wie früher nur aus Spur-Splines)
- Gehwege an Einbahnstraßen nur dort, wo daneben Platz ist (Bypässe, Richtungsfahrbahnen)
- Haltestellenschilder an den OSM-Haltestellen
- automatische Prüfung jedes Spurendes + Draufsicht als PNG

## Noch offen (Roadmap)

1. Vorfahrt / Ampeln an Kreuzungen
2. Fußwege über die Fahrbahn (Zebrastreifen/Querungen); um die Ecken sind sie verbunden
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
  network.py        Kanten, Knoten, Kreisverkehre, Kürzen, Ketten, Kreuzungsspuren
  kreuzung.py       Kreuzungsobjekte: Fläche, Bordsteine, Pfade -> .sco + .x
  geom.py           Geraden/Bögen/Doppelbögen in OMSI-Konvention
  config.py         welche Spline für welche Straße
  custom_splines.py eigene .sli (Einbahnstraßen, Kreuzungsspuren)
  splinedb.py       Fahrspuren aus .sli lesen
  writer.py         Kartenordner schreiben
  check.py          Spurprüfung und Vorschau
  ansicht.py        Karten-Betrachter für beliebige OMSI-Karten (PNG, HTML, Bericht)
  openomsi.py       Karte in openOMSI rendern, dessen Pfadnetz-Auswertung lesen
docs/omsi-format.md alles, was wir über das Dateiformat wissen
samples/            Testdaten (OSM-Auszug Aschaffenburg)
```

Benötigt die Standardinhalte von OMSI 2 (Splines `Marcel`, `template/NewMap`); diese werden nicht mitgeliefert.

## Daten und Lizenz

Straßendaten © OpenStreetMap-Mitwirkende, verfügbar unter der
[Open Database License](https://www.openstreetmap.org/copyright). Erzeugte Karten enthalten einen
entsprechenden Hinweis; beim Weitergeben bitte beibehalten.
