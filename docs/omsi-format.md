# OMSI-2-Kartenformat – was wir wissen

Abgeleitet aus den Standardkarten (vor allem Grundorf) und geprüft, indem die Dateien wieder eingelesen und
die Anschlüsse nachgerechnet wurden. Wenn etwas hier nicht stimmt: bitte korrigieren und vermerken, woher die
neue Erkenntnis stammt.

## Kartenordner `maps/<Name>/`

| Datei | Inhalt |
|---|---|
| `global.cfg` | Name, Beschreibung, `[NextIDCode]`, Kamera, Jahreszeiten, Liste der Kacheln (`[map]` x, z, Datei) |
| `tile_X_Z.map` | eine Kachel, 300 × 300 m; X nach Osten, Z nach Norden, auch negativ möglich |
| `tile_X_Z.map.terrain` | Gelände: int32 `60`, dann 61 × 61 float32 Höhen (alle 0 = flach) |
| `tile_X_Z.map.LM.bmp` | Lightmap 256 × 256 (kann aus `template/NewMap` kopiert werden) |
| `ailists.cfg`, `humans.txt`, `Holidays*.txt`, … | Vorlagen aus `OMSI 2/template/NewMap` |

Textdateien sind bei neueren Karten **UTF-16 LE mit BOM** und CRLF; ältere ANSI (cp1252) werden auch gelesen.

## Spline in einer `.map`-Datei

```
[spline]            ([spline_h] gibt es auch)
0
Splines\Marcel\str_2spur_11m_SeeburgerStr1.sli
<ID>                eindeutig in der ganzen Karte, < NextIDCode
<ID Vorgänger>      0 = keiner (nur Editor-Hinweis; Verbindung entsteht über passende Endpunkte)
<ID Nachfolger>
<x>                 lokal in der Kachel (0…300, darf über den Rand hinausragen)
<y>                 Höhe
<z>                 lokal, Nord
<Richtung>          Grad, im Uhrzeigersinn ab Nord (0 = +z, 90 = +x)
<Länge>             m (bei Bögen: Bogenlänge)
<Radius>            0 = Gerade, > 0 Rechtskurve, < 0 Linkskurve
<Steigung Anfang>, <Steigung Ende>, <?>, <Überhöhung Anfang>, <Überhöhung Ende>, <?>   (bei uns alle 0)
<Längen-Offset>     aufsummierte Länge in der Kette (für Texturverlauf)
```

Endpunkt eines Bogens: Mittelpunkt = Start + R·(cos h, −sin h); Endrichtung = h + L/R (rad → Grad).
Geprüft an Grundorf: Fehler < 0,1 mm (Test `test_arc_convention`).

## Objekt

```
[object]
0
Sceneryobjects\Generic\bus_stop.sco
<ID>
<x> <z> <Höhe>      ACHTUNG: andere Reihenfolge als bei Splines (dort x, Höhe, z)
<Drehung>
0
0
<Anzahl Texte> und die Texte (Haltestellenschild: Name, "20", "5", "0", "", "", "")
```

## Spline-Definition `.sli`

- `[heightprofile]` x1 x2 h1 h2: befahrbare Fläche (Fahrbahn 0,10 m, Gehweg 0,25 m).
- `[texture]` Dateien im Unterordner `texture` des Spline-Ordners.
- `[profile]` Texturindex, dann `[profilepnt]` x, y, u, v-Wiederholung.
- `[path]` Typ (0 = Fahrzeug, 1 = Fußgänger), Querlage x, Höhe, Breite, Richtung (0 = mit Spline, 1 = dagegen, 2 = beide).
  Positive x = rechts der Splinerichtung (Rechtsverkehr: Spur mit Richtung 0 liegt bei +x).

## Verbindungen

OMSI verbindet Pfade, wenn Endpunkt und Richtung zweier Pfade zusammenfallen. Deshalb baut omsigen an jeder
Kreuzung eigene Kreuzungs-Splines (eine Fahrspur je Abbiegebeziehung), die exakt an den Spurenden der Arme
beginnen und enden. `omsigen.check.validate` rechnet das für jede Karte nach (Toleranz 5 cm / 1°).

## nEditor

Der nEditor (Unity, eigener Spielstand in `Documents/nEditor/Library`) liest die OMSI-Dateien beim Laden ein.
Neue Spline-Dateien erkennt er erst nach einem Neustart. Mit „Show paths“ werden Fahrspuren (gelb) und
Fußwege (grün) angezeigt – ideal zum Kontrollieren.
