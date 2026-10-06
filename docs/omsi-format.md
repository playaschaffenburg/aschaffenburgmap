# OMSI-2-Kartenformat – was wir wissen

Abgeleitet aus den Standardkarten (vor allem Grundorf) und geprüft, indem die Dateien wieder eingelesen und
die Anschlüsse nachgerechnet wurden. Wenn etwas hier nicht stimmt: bitte korrigieren und vermerken, woher die
neue Erkenntnis stammt.

**Quellen** (bei jeder Angabe vermerkt, wo es nicht offensichtlich ist):
- *[Grundorf]*, *[Spandau]*: an den Standardkarten nachgezählt/nachgerechnet.
- *[openOMSI]*: aus dem Nachbau [openOMSI](https://github.com/openOMSI-Project/openOMSI) (MIT-Lizenz),
  `docs/FORMATS.md` und Quelltext (`crates/omsi-map/src/tile.rs`, `crates/omsi-sim/src/traffic.rs`), Stand
  Commit `d322526` (Okt. 2026). Lokale Kopie: `Documents\OpenOmsi\source`. openOMSI ist ein Nachbau im frühen
  Stadium – bei Widerspruch gilt, was OMSI selbst tut.

## Kartenordner `maps/<Name>/`

| Datei | Inhalt |
|---|---|
| `global.cfg` | Name, Beschreibung, `[NextIDCode]`, Kamera, Jahreszeiten, Liste der Kacheln (`[map]` x, z, Datei) |
| `tile_X_Z.map` | eine Kachel, 300 × 300 m; X nach Osten, Z nach Norden, auch negativ möglich |
| `tile_X_Z.map.terrain` | Gelände: int32 `60`, dann 61 × 61 float32 Höhen im 5-m-Raster, **Zeile für Zeile von Süden (lokal z = 0) nach Norden, je Zeile von Westen nach Osten** (Index `iz * 61 + ix`) *[openOMSI `terrain.rs`]* |
| `tile_X_Z.map.LM.bmp` | Nacht-Lichtkarte 256 × 256 für die Kachel **und ihre 8 Nachbarn** (Kachel = mittleres Drittel) *[openOMSI]*; kann aus `template/NewMap` kopiert werden |
| `ailists.cfg`, `humans.txt`, `Holidays*.txt`, … | Vorlagen aus `OMSI 2/template/NewMap` |
| `TTData/` | Fahrplan: `Busstops.cfg`, `StnLinks.cfg`, `*.ttp` (Fahrten), `*.ttr` (Spurfolgen), `*.ttl` (Linien) – siehe unten |

`.map`, `global.cfg` und Situationen (`.osn`) sind **UTF-16 LE mit BOM** und CRLF; alle anderen Textdateien
(`.sli`, `.sco`, `.cfg` …) liest OMSI in der ANSI-Codepage (cp1252) *[openOMSI]*.

Mit `[worldcoordinates]` in `global.cfg` (nur Berlin-Spandau) ist das Kachelraster 1/300 Grad und eine Kachel
371,9 m groß *[openOMSI]*. omsigen benutzt das nicht (300-m-Kacheln).

Allgemeines Blockformat aller Textdateien: Ein `[schlüsselwort]` muss **allein und ohne Leerzeichen** auf der
Zeile stehen, danach folgen die Parameter je Zeile; alles andere ist Kommentar *[openOMSI]*.

## Spline in einer `.map`-Datei

```
[spline]            ([spline_h]: nach den Steigungen eine zusätzliche Zeile Höhenänderung)
0                   Detailstufe
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
<Steigung Anfang>   %
<Steigung Ende>     %
<Querneigung Anfang>
<Querneigung Ende>
<Skew Anfang>
<Skew Ende>
<Längen-Offset>     aufsummierte Länge in der Kette (für Texturverlauf)
[mirror]            optional: Querschnitt gespiegelt (alle Pfade bei −x und in Gegenrichtung)
```

Feldfolge nach dem Radius *[openOMSI]*, bestätigt an Spandau: die Felder 14/15 sind nur paarweise belegt
(Querneigung, 18 Splines), 16/17 bei Kabeln und Zäunen (Skew). **Früher stand hier fälschlich „?, Überhöhung,
Überhöhung, ?“** – omsigen schreibt alle sechs Werte als 0, die Karten waren davon nicht betroffen. Für Höhen und
Querneigung (Roadmap) gilt die Folge oben.

Höhe und Steigung: `<y>` ist die Höhe am Anfang, `<Steigung Anfang/Ende>` in Prozent. omsigen schreibt je Element
eine gleichbleibende Steigung (Anfang = Ende), die Elemente sind höchstens 20 m lang; so endet jedes Element genau auf
der Starthöhe des nächsten. Kreuzungsobjekte bekommen `[absheight]` und stehen auf der Höhe ihrer Platte (wie die
Standardkreuzungen mit `[splinehelper]`).

Endpunkt eines Bogens: Mittelpunkt = Start + R·(cos h, −sin h); Endrichtung = h + L/R (rad → Grad).
Geprüft an Grundorf: Fehler < 0,1 mm (Test `test_arc_convention`).

## Objekt

```
[object]
0                   Detailstufe
Sceneryobjects\Generic\bus_stop.sco
<ID>
<x> <z> <Höhe>      ACHTUNG: andere Reihenfolge als bei Splines (dort x, Höhe, z)
<Drehung>           Grad, im Uhrzeigersinn ab Nord
<Neigung>           (pitch)
<Rollen>            (bank)
<Anzahl Texte> und genau so viele Zeilen (auch leere) – Haltestellenschild: Name, "20", "5", "0", "", "", ""
```

Die Höhe ist relativ zum Gelände – außer bei Objekten mit `[absheight]` und bei Objekten mit
`[splinehelper]` (Kreuzungen, Weichen): deren Höhe ist absolut wie die der Splines *[openOMSI]*.

Straßennamen stehen nirgends in der Karte außer auf Straßenschildern (`StreetSign_*`, erster Text = Name,
Drehung = Straßenrichtung + 90°) – das Navi liest sie *[openOMSI]*.

## Regeln `[rule]` (Vorfahrt, Tempo, Verkehrsdichte)

Ein `[rule]` gehört zum `[spline]` oder `[object]` **direkt davor** und gilt für einen seiner Pfade *[openOMSI]*:

```
[rule]
<Pfad-Index>        0-basiert: Reihenfolge der [path]-Einträge in .sli bzw. .sco
<Art>               priority | speedlimit | trafficdensity | overtaking_prohib | bus | trucks | no_cars
<Wert>
<Zusatz>            meist 0
```

**Vorfahrt** *[openOMSI, Grundorf]*: `priority` 192 auf den Geradeaus-Pfaden der Hauptstraße, 64 auf den Pfaden,
die aus der Nebenstraße kommen; ohne Regel gilt 128. Bei gleicher Priorität: rechts vor links, Linksabbieger
warten auf den Gegenverkehr. In Grundorf hängen alle 53 `priority`-Regeln an Kreuzungsobjekten
(`Kreuz_See_Elsflether.sco`, `Einm_See*.sco`), nicht an Splines. Weitere Zählung Grundorf: 217
`trafficdensity`, 110 `trucks`, 28 `no_cars`, 8 `speedlimit` (80), 2 `bus`.

## Einsetzpunkte `[entrypoints]` *[Grundorf, openOMSI]*

Wo man bei „Freie Fahrt“ startet. In der Kachel steht ein unsichtbares Objekt `Sceneryobjects\Generic\entrypoint_bus.sco`
(`[entrypoint]`, `[onlyeditor]`); in `global.cfg` je Einsetzpunkt 12 Zeilen: Index des Objekts in der Kachel,
Objekt-ID, `0`, x, Höhe, z (lokal), Quaternion x y z w (Drehung h: `0, sin(h/2), 0, cos(h/2)`), Index der Kachel in
der `[map]`-Liste, Name. Ohne Einsetzpunkt setzt OMSI den Bus irgendwo hin (bei uns: an den Kartenrand).
omsigen legt Einsetzpunkte an Start, Ziel und jeder Haltestelle an (rechte Fahrspur, in Fahrtrichtung);
geprüft mit openOMSI `--entry 0`: „spawned at entry point … "Start: Hauptbahnhof"“.

## Spline-Definition `.sli`

- `[heightprofile]` x1 x2 h1 h2: befahrbare Fläche (Fahrbahn 0,10 m, Gehweg 0,25 m).
- `[texture]` Dateien im Unterordner `texture` des Spline-Ordners.
- `[profile]` Texturindex, dann `[profilepnt]` x, y, u, v-Wiederholung.
- `[path]` Typ (0 = Fahrzeug, 1 = Fußgänger, 2 = Schiene), Querlage x, Höhe, Breite, Richtung
  (0 = mit Spline, 1 = dagegen, 2 = beide). Positive x = rechts der Splinerichtung (Rechtsverkehr: Spur mit
  Richtung 0 liegt bei +x).
- `[terrainholeprofile]`, danach je `[terrainholeprofilepnt]` x, Höhe, Überstand am Ende: Profil des
  Geländelochs, von links nach rechts *[openOMSI docs/FORMATS.md, `omsi-geometry` `terrain_hole_profiles`]*.
  Geschnitten wird nur bei Splines mit `[spline_terrain_align]` in der Kachel; der Umriss von oben ist die
  Profilbreite entlang des Splines. Ohne eigenes Profil macht OMSI eines aus den `[profile]`s (Mulde von der
  linken bis zur rechten Kante, 3 cm eingerückt). Das Original schneidet nur die Geländedreiecke weg; openOMSI
  zeichnet zusätzlich Lochwände vom Lochrand bis zum Gelände (gestreift) – die gibt es im Spiel nicht.
- Sichtbare Seite eines `[profile]`: links der Laufrichtung von Punkt zu Punkt (waagerecht von links nach rechts
  = Oberseite, senkrecht nach oben = Fläche nach links) *[Splines\Marcel\rail_boxcut_concrete_1tr6m.sli]*.

## Gelände ausschneiden in der Kachel

Hinter einem `[spline]`-Block (nach den zwei Leerzeilen) steht `[spline_terrain_align]` oder
`[spline_terrain_align_2]` mit einer Zahl (1–4; 2/4 halten das ferne, 3/4 das nahe Lochende am Splineende)
*[Berlin-Spandau tile_2382_11285.map; openOMSI docs/FORMATS.md]*. In Spandau tragen 236 von 2486 Splines das
Kennzeichen.

## Brücken und Tunnel (omsigen/bauwerke.py)

Die Standardkarten benutzen eigene Brücken-Splines (z. B. `Splines\Ruede\str_2spur_9.5m_DDR_Potsdamer_Str_BUE.sli`).
omsigen legt stattdessen einen **Begleit-Spline ohne Pfade** genau auf die Fahrbahn (gleiche Lage, Höhe,
Steigung), damit die Spuranschlüsse unverändert bleiben:

- `AB_bruecke_<links>_<rechts>.sli`: Platte 1,2 m, Brüstung 1 m über dem Gehweg, Beton `betonwand1.bmp`
  aus `Splines\Marcel\texture`; Pfeiler (Objekte `pfeiler_n`, `[absheight]`) etwa alle 30 m, 1 m eingegraben.
- `AB_tunnel_<links>_<rechts>.sli`: Wände, Decke 5 m, Außenhaut und `[terrainholeprofile]`. Das Gelände wird
  auf den ersten 15 m hinter jedem Portal ausgeschnitten (Röhre und Fahrbahn davor), weiter innen auf
  mindestens Deckenhöhe + 0,3 m angehoben. Portale sind Objekte (`portal_n`): Rahmen um die Öffnung bis über das
  Gelände. Ein Portal entsteht, wo der Tunnel in offene Strecke übergeht oder an einer Kreuzung mit Straßen
  außerhalb des Tunnels endet.
- Fahrbahn mehr als 1,5 m unter dem Gelände (Einschnitt): `[spline_terrain_align]` am Straßen-Spline.

## Kreuzungsobjekte `.sco` (so bauen die Standardkarten Kreuzungen)

Grundorf und Spandau verwenden fertige Kreuzungsobjekte: eine Platte (`[mesh]`) mit eigenen Pfaden, an die die
Straßen-Splines heranführen. omsigen macht es seit Okt. 2026 genauso (`omsigen/kreuzung.py`):

- `Sceneryobjects\Aschaffenburg_KI\<Karte>\K_nnn.sco`, Modell in `model\K_nnn.x`, Texturen in `texture\`
  (kopiert aus `Splines\Marcel\texture`). Kopf wie `Kreuz_MC\Einm_See.sco`: `[rendertype] surface`,
  `[LightMapMapping]`, `[fixed]`, `[surface]`.
- Objekt am Kreuzungsmittelpunkt, Drehung 0; Modell-Koordinaten x = Ost, y = Höhe, z = Nord (Direct3D: x rechts,
  y oben, z vorwärts). Fahrbahn 0,10 m, Gehweg 0,25 m hoch, senkrechte Bordsteinflächen dazwischen.
- Dreiecke im Uhrzeigersinn von der Sichtseite aus (Direct3D-Vorderseite); geprüft im openOMSI-Spielbild.
- `.x`-Modelle (Textformat `xof 0302txt`) liest OMSI wie `.o3d` (Standardobjekte, z. B. `Aachen_Gruenzeug\Baum1.sco`).
- Nur Pfade **eines** Kreuzungsobjekts werden auf Konflikte (Vorfahrt) geprüft *[openOMSI: `compute_conflicts`]*:
  mit Kreuzungsobjekten meldet openOMSI für Aschaffenburg_Live_v2 101 Konfliktpaare, mit Spline-Kreuzungen 0.

`[path]` in einer `.sco`, 12 Zeilen *[openOMSI]*, geprüft an `Einm_See.sco`:

```
<x> <y> <z>         Start im Objekt (x rechts, y vorwärts, z hoch)
<Richtung>          Grad, relativ zum Objekt
<Radius>            0 = gerade, > 0 rechts
<Länge>
0
<Höhenänderung>
<Art>               0 Straße, 1 Gehweg, 2 Schiene
<Breite>
<Richtung>          0/1/2 wie bei .sli
<Blinker>           0 keiner, 2 links, 3 rechts (für die KI)
```

**Ampeln** *[openOMSI, Grundorf]*: `[use_traffic_light] n` nach einem Pfad bindet ihn an Signalgruppe n (0-basiert);
`[traffic_lights_group] <Umlauf s>`, je Gruppe `[traffic_light] <Name>` und `[phase] <Zustand> <Sekunden>` –
die Phasen laufen nacheinander ab Umlaufbeginn, die letzte `0 0` = rot bis zum Ende des Umlaufs.
Zustände: 0–2 rot, 3–5 rot-gelb, 6/7 grün, 8 grün-gelb, **9–11 gelb**, ab 12 dunkel (openOMSI `traffic.rs`
`aspect()`; die Angabe „8 gelb, 9 alles rot“ in openOMSIs FORMATS.md ist veraltet). Grundorf `Kreuz_See_Elsflether`:
Umlauf 72 s, Hauptrichtung `3 2 / 6 31 / 9 3 / 0 0`, Nebenrichtung ab 39 s.
- Gebunden ist in Grundorf nur das **erste, 2–3 m kurze Stück jeder Zufahrt**; dort halten die Autos bei Rot.
- Signale sind eigene Kartenobjekte: `Verkehrszeichen_MC\Ampel_Kfz_1.sco` (Kfz) bzw. `Ampel_Mensch_1.sco`
  (Fußgänger), Höhe 2,8 m, ein Text = Index der Signalgruppe, danach `[varparent] <ID des Kreuzungsobjekts>`.
  Darunter steht ein Mast (`Streetobjects_RUE\whip_beam.sco` bzw. `single_pole.sco`, Höhe 0,25 m),
  0,46 m hinter dem Signal und um 180° gedreht. Das Signal blickt den ankommenden Fahrern entgegen
  (Drehung ≈ Richtung des Arms von der Kreuzung weg).
- Das Signal **oben am Ausleger** ist ein `[attachObj]` *[Grundorf]*: `0`, `Verkehrszeichen_MC\Ampel_Kfz_Oh_1.sco`
  (bzw. `…OhBlind_1`), ID, **ID des Masts**, `0` (Instanz), `0` (Anhängepunkt des Masts = Ende des Auslegers,
  `whip_beam.sco` hat zwei `[new_attachment]`), Drehung `2.98…`, `0`, `0`, Texte (`1`, Signalgruppe), danach
  `[varparent] <Kreuzung>`. Muss in der Kachel **nach** dem Mast stehen.
- omsigen (`omsigen/ampel.py`) baut es genauso: Phasen aus gegenüberliegenden Armen (Hauptstraße zuerst),
  2 s rot-gelb, 3 s gelb, 3 s Räumzeit, Umlauf 70 s bei 2 Phasen (+15 s je weitere), Hauptrichtung 1,5-fache
  Grünzeit; Signal + Mast je Zufahrt rechts am Bordstein.

## Verbindungen

OMSI verbindet Pfade, wenn Endpunkt und Richtung zweier Pfade zusammenfallen. Deshalb baut omsigen an jeder
Kreuzung eigene Kreuzungs-Splines (eine Fahrspur je Abbiegebeziehung), die exakt an den Spurenden der Arme
beginnen und enden. `omsigen.check.validate` rechnet das für jede Karte nach (Toleranz 5 cm / 1°).

openOMSI verbindet großzügiger: Abstand ≤ 1,5 m, Richtung < 40°, Höhe < 3 m *[openOMSI]*. Wie tolerant das
Original ist, ist nicht geprüft – deshalb bleibt unsere Prüfung streng.

## KI-Verkehr *[Grundorf, openOMSI]*

- `ailists.cfg`: je Gruppe `[aigroup_2]`, Name, Hof-Datei (leer bei Autos), dann `Fahrzeugdatei<TAB>Anzahl`
  je Zeile, `[end]`. Die Vorlage `template\NewMap\ailists.cfg` hat ein älteres Format, mit dem openOMSI keine
  Fahrzeuge findet – omsigen schreibt deshalb eine eigene (Gruppen `NormalCars`, `Trucks`, wie sie
  `unsched_vehgroups.txt` der Vorlage erwartet).
- `global.cfg` `[trafficdensity_road] <Stunde> <Faktor>`: Tagesganglinie des Straßenverkehrs (Grundorf: 11 Punkte,
  Spitzen 7 und 17 Uhr mit 1,5).
- Straßenenden ohne Fortsetzung (Kartenrand, Sackgasse) fangen KI-Fahrzeuge: sie bleiben stehen und stauen sich.
  Die Standardkarten legen dafür unsichtbare Hilfsstraßen `Splines\Marcel\invis_street.sli` an (`[onlyeditor]`,
  ein Fahrpfad; in Gladbeck über 11.000-mal). omsigen baut an jedem solchen Ende Wendeschleifen daraus
  (Schlüsselloch: Gerade, Rechtsbogen, Linksbogen ≥ 6 m Radius, Rechtsbogen, Gerade); Einbahn-Enden wenden auf
  die nächste Gegenrichtung eines anderen Straßenendes. openOMSI-Zählung „lanes lead into a dead end“ für die
  Teststrecke: 196 → 63.

## Fahrplan `TTData/` *[openOMSI]*

- `Busstops.cfg`: `[busstop]` Name, Kachel-Index (Position in `[map]` von global.cfg), Objekt-ID, Offset, 0, 0.
- `*.ttp` Fahrt: `[trip]` + 3 Zeilen (Strecke, Ziel, Linie), `[station_typ2] <Objekt-ID>` je Halt,
  `[profile] name minuten`.
- `*.ttr` Spurfolge: `[track_entry] <ID> <Pfad-Index> <Kachel-Index> <interne Nr> <Länge> 0`.
- `*.ttl` Linie: `[newtour] nummer KI-Gruppe extra`, `[addtrip] fahrt profil abfahrt-minuten`.
- `StnLinks.cfg`: Wege zwischen Haltestellen aus `[StnLink_entry]`.

## nEditor

Der nEditor (Unity, eigener Spielstand in `Documents/nEditor/Library`) liest die OMSI-Dateien beim Laden ein.
Neue Spline-Dateien erkennt er erst nach einem Neustart. Mit „Show paths“ werden Fahrspuren (gelb) und
Fußwege (grün) angezeigt – ideal zum Kontrollieren. Start nur über `nEditor-downloader` („nEditor starten“),
direkter Start der exe meldet „Licencja w użyciu (409)“.
