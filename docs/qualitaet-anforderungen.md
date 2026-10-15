# Qualitätsoffensive: Anforderungen und Ablauf

Stand 15.10.2026. Grundlage: Bestandsaufnahme des Codes (OMSIgen `404a196`, earth-reverse-engineering `7923538`)
und die Karte `Aschaffenburg_Linie16_v2`. Ziel: Karten, die nah an der Wirklichkeit sind und sich später Stück für
Stück erweitern lassen, ohne dass ein Bereich den anderen kaputt macht.

Drei Arbeitsbereiche, davor ein gemeinsames Fundament:

| | Bereich | Kern |
|---|---|---|
| **F** | Fundament | ein Geobezug, Ebenenmodell, gespeichertes Netz, Belegungskarte, Prüfkette, Budgets |
| **S** | Straßen und Boden | echte Querschnitte, Markierungen, Bodentexturen, Verkehrslogik |
| **G** | Gebäude und Objekte | Platzierung, Fassadenqualität, Straßenmöbel, Haltestellen |
| **V** | Vegetation und Backdrops | Bäume, Parks, Hecken, Kartenrand, logische Straßenenden |

Grundsätze (aus CLAUDE.md, gelten für alles): Editor zuerst (jede Funktion im Rust-Editor bedienbar und sichtbar),
nichts erfinden (Quelle für alles, was gebaut wird), nie bestehende Karten überschreiben, jedes Ergebnis durch einen
Prüfer, Bilder ansehen. Aus Google-Fotos abgeleitete Daten sind nur privat nutzbar (siehe F7).

---

## 1. Bestand (kurz)

**Straßen/Boden.** Der Generator kennt 6 Marcel-Splines + 12 eigene Einbahn-Splines (`config.choose_spline`); Breite,
`turn:lanes`, Gehweg, Radweg, Parken und Belag aus OSM werden ignoriert, Autobahnen bekommen die 6-m-Straße.
Kreuzungen (`kreuzung.py`) haben 2 Materialien, keine Markierungen, festen Bordradius 8 m. Der Boden ist überall
`gras.bmp` (keine Malmasken `texture/map/*.dds`), das Luftbild DOP40 gibt es nur im Editor. Das Netz des Editors
(`netz.rs`) wird nicht gespeichert; Generator (`network.py`) und Editor bauen Straßen mit getrenntem Code.
**Zwei Koordinatensysteme:** Generator = ebene Projektion um lat0/lon0 (`route.Projection`), Editor = UTM-Raster
mit `omsi-editor-geo.cfg`. Generator-Karten können Luftbild, Vermessen, LoD2 je Kachel und Abgleich nicht nutzen.

**Gebäude/Objekte.** LoD2 je Gebäude ein Objekt mit absoluter Höhe und 1 m Schürze; einfache Texturen (256² BMP je
Kachel kopiert) oder Fassaden aus `korridor.py` (DDS BC1). Keine Rücksicht auf Straße/Gehweg/Kreuzungsplatte, keine
Geländeanpassung über die Schürze hinaus, kein LOD, nur Rechteckfenster, große Gebäude mit grobem Atlas.
Sonst nur Haltestellenschild, Einsetzpunkt, Ampeln, Brücken/Tunnel. Keine Laternen, Schilder, Bänke, Zäune.

**Vegetation/Backdrops.** Nichts wird erzeugt. OSM-Abfrage holt nur `highway`, Haltestellen, Schilder. Vorhandene
Bäume lassen sich im Editor verschieben/löschen. Straßenenden bekommen unsichtbare Wendeschleifen
(`network.py`, funktional richtig, optisch endet die Straße im Nichts). Am Kartenrand nacktes Gelände.
Vorhanden und nutzbar: `Trees_MC` (43 Baum-Billboards mit Jahreszeiten), Laserdaten-Anbindung (`aufmass.py`), DOP20.

---

## 2. Fundament (F) – zuerst, Voraussetzung für S, G, V

### F1 Ein Geobezug für alle Karten
**Erledigt 15.10.2026:** `route.Projection` rechnet im UTM-Gitter (Bezugspunkt auf ganze Meter), `pipeline.erzeuge`
schreibt `omsi-editor-geo.cfg` (ost0/nord0 = Bezugspunkt + Kartenversatz, nn0 = Höhenbasis); Projektdateien ohne
`"projektion": "utm"` behalten die alte ebene Näherung (`route.EbeneProjektion`). Geprüft an `Aschaffenburg_Linie16_v3`:
Luftbildvergleich Herstallturm und Haibach (5 km vom Bezugspunkt) deckungsgleich; LoD2 über den Editor-Weg (Geobezug) und
über den Generator liegen auf 1 mm gleich. Ein gemeinsamer Geländeabtaster steht noch aus (Generator nutzt bei UTM die
Projektmeter direkt, Editor das 5×5-Mittel).

- Der Generator schreibt Karten im UTM-Raster mit `omsi-editor-geo.cfg` (ost0/nord0/nn0) statt `route.Projection`.
  Eine Karte = ein Bezug; Kachelraster bleibt beim Erweitern gleich.
- Ein Geländeabtaster für Generator und Editor (heute: Punkt-bilinear vs. 5×5-Mittel).
- Bestehende Generator-Karten: Werkzeug, das einen Bezug nachträgt (Ursprung/Offset aus `omsigen.json`) – nur als neue
  Kartenversion.
- **Abnahme:** Linie 16 neu erzeugt → Luftbild, Vermessen, LoD2 je Kachel funktionieren im Editor; Straßen liegen im
  Luftbildvergleich < 0,5 m daneben.

### F2 Ebenenmodell mit festen Kennungen
**Erledigt 15.10.2026 (Grundgerüst):** `omsigen/schichten.py` – Schichten netz, boden, gebaeude (Ordner `LoD2`),
objekte, gruen, backdrop; Schicht-Paket = Sonderbau-Format mit `format`, `schicht`, `marker`, `manifest` (Werkzeug,
Parameter, Quellen, Fingerabdrücke der Schichten davor), Objekte mit optionaler Drehung und Beschriftungen (Rust
`sonderbau.rs` setzt beides ein); Verzeichnis `maps/<Karte>/schichten.json`; `python -m omsigen.schichten <Karte>` zeigt je
Kachel aktuell/veraltet/gebaut; Python-`einsetzen` für neu erzeugte Karten. `omsigen.lod2` schreibt Pakete mit Manifest,
der Generator baut Gebäude bei UTM-Karten über denselben Weg wie der Editor. World Editor → Kacheln bearbeiten →
„Schichten: Zustand prüfen“, bei veralteten Gebäuden „neu bauen“. Geprüft an `Aschaffenburg_Linie16_v4`: 116 Kacheln
aktuell; eine Straßenänderung in Kachel 1/7 macht genau deren Gebäude veraltet. Noch offen: Fingerabdruck der Sitzung
(heute zählt die gespeicherte Karte), weitere Schichten nutzen das Gerüst ab Phase 2/3.
(Begriff im Code: „Schicht“, weil „Ebene“ in omsigen schon Boden/Brücke/Tunnel heißt.)

Jede Ebene wird getrennt erzeugt und je Kachel eingesetzt; erneutes Einsetzen ersetzt **nur ihre eigenen** Einträge
(verallgemeinert aus dem LoD2-Marker `\LoD2\<kx>_<ky>\`).

| Reihenfolge | Ebene | Kennung (Ordner/Marker) | liest |
|---|---|---|---|
| 1 | Netz (Straßen, Kreuzungen, Bauwerke) | Splines + `K_*` | Geodaten |
| 2 | Boden (Gelände, Malmasken) | `.terrain`, `texture/map/` | Netz |
| 3 | Gebäude | `\LoD2\<kachel>\` | Netz, Boden |
| 4 | Straßenobjekte (Laternen, Schilder, Haltestellen) | `\Objekte\<kachel>\` | Netz, Boden, Gebäude |
| 5 | Vegetation | `\Gruen\<kachel>\` | alle davor |
| 6 | Backdrop | `\Backdrop\<kachel>\` | alle davor |

- Eine Ebene darf frühere lesen, nie ändern. Ändert sich eine frühere Ebene, werden die späteren der betroffenen
  Kacheln als „veraltet“ markiert (Anzeige im Editor, gezielt neu bauen).
- Jede Ebene hat ein Manifest je Kachel (Quelle, Version, Erzeugungszeit, Parameter), damit Neubauten reproduzierbar
  sind.
- Von Hand im Editor gesetzte Objekte haben keinen Ebenen-Marker und werden nie automatisch ersetzt.

### F3 Gespeichertes Netz, ein Netz-Kern
**Erledigt 15.10.2026:** `omsigen/netz_datei.py` leitet `maps/<Karte>/netz.json` aus der Karte selbst ab (alle
`[spline]` der Kacheln): Splines mit zusammenfallenden Enden (gleiche .sli/Spiegelung) bilden Kanten `E<erste
Spline-ID>`, Kantenenden an einem `K_*`-Objekt einen Kreuzungsknoten `J<Objekt-ID>`, sonst Querschnittswechsel bzw.
Ende `N<x>_<z>`; je Kante Punkte alle 2 m mit Station und Höhe, Querschnitt aus der .sli (Fahrbahn, Außenkanten,
Spuren, Gehwege) und Attribute. Der Generator schreibt die Attribute je Spline (`netz_attribute.json`: OSM-Name,
Klasse, Tags, Art) und `netz.json` mit; der Editor schreibt `netz.json` nach jedem Speichern neu (Hintergrund,
Protokoll). Abfragen für spätere Schichten: `Netz.punkt(kante, station, seite)`, `Netz.naechste(x, z)`,
`Netz.streifen(kante, links, rechts)`; `Netz.laden` prüft per Fingerabdruck, ob netz.json zur Karte passt.
Entscheidung: kein eigenes Rust-Netzformat, weil der Editor beim Speichern ohnehin Splines schreibt – die Karte ist die
eine Wahrheit, netz.json ihre Lesart. Geprüft an `Aschaffenburg_Linie16_v5`: 412 Straßenkanten (= Straßenzüge des
Generators), 235 Kreuzungsknoten (= Kreuzungsobjekte), 379 mit Straßennamen.

- Das Straßennetz (Knoten, Kanten, Querschnitte, Regeln) wird als eigene Datei je Karte gespeichert (`netz.json`) und
  beim Öffnen gelesen – nicht nur als Splines.
- Generator und Editor teilen ein Datenmodell (Rust `netz.rs` als Referenz, Python liest/schreibt dasselbe Format).
- Spätere Ebenen beziehen sich auf Kanten-IDs (z. B. „Laterne an Kante 412, Station 35 m, rechts“), nicht auf Koordinaten.

### F4 Belegungskarte
**Erledigt 15.10.2026:** `omsigen/belegung.py` – je Kachel ein 0,5-m-Raster (600 × 600) mit den Klassen frei,
fahrbahn, gehweg, kreuzung, gebaeude, gruen, wasser, bauwerk; Quellen netz.json (Fahrbahn zwischen den Fahrbahnkanten,
Seitenraum bis zu den Außenkanten), die Kreuzungsmodelle `K_*.x` (Asphalt/Gehweg) und die Modelle der Schicht gebaeude
(von oben). Ablage `maps/<Karte>/belegung/tile_<x>_<y>.png` (Palettenbild, Index = Klasse, Zeile 0 = Norden) und
`belegung.json` (Fingerabdrücke Netz/Gebäude je Kachel, Konflikte „Gebäude über Fahrbahn“ ab 2 m²). Abfragen:
`Belegung.klasse(x, z)`, `erlaubt(x, z, klassen)`, `raster(kachel)`. Generator berechnet sie mit, der Editor nach dem
Speichern (nach netz.json); in der Leiste zeigt „Belegung“ (neben dem Luftbild-Regler) sie als Bodenbild. Geprüft an
`Aschaffenburg_Linie16_v5`: deckungsgleich mit dem Luftbild (Herstallturm, Haibach) und im Editor; 183 Gebäude über
Fahrbahn – erwartbar, solange die Straßen die alten Standardbreiten haben (S1). Grün und Wasser füllt Phase 3.

Je Kachel ein Raster (z. B. 0,5 m), abgeleitet aus Netz und Gebäuden: Fahrbahn, Gehweg, Radweg, Kreuzungsplatte,
Gebäude, Grün, Wasser, frei. Jede spätere Ebene platziert nur auf erlaubten Klassen (Baum nicht auf Fahrbahn, Laterne
auf Gehweg, Gebäude nicht über Gehweg). Die Prüfer nutzen dieselbe Karte für Konflikte.

### F5 Datenablage und Quellen
- Alles Kartenbezogene in den Kartendaten (`Documents\Aschaffenburg\…`), Rohdaten im Zwischenspeicher
  (`%LOCALAPPDATA%\omsi-editor\geodaten`), getrennt nach Quelle mit Lizenzvermerk.
- Quellen: OSM (ODbL), DGM1/DOP/LoD2/Laser Bayern (CC BY 4.0), Mapillary/Panoramax (Hinweise), Google (privat).

### F6 Budgets und LOD
- Budgets je Kachel: Objekte, Dreiecke, Texturspeicher (Ziel z. B. ≤ 40 MB DDS je Kachel); Prüfer meldet Überschreitung.
- Texturen nur DDS (BC1/BC3 mit Mips); gemeinsame Texturen einmal pro Karte statt pro Kachel.
- Ferne Gebäude/Bäume einfacher (Sichtweite, ggf. zusammengefasste Blöcke).

### F7 Herkunft je Objekt (nur privat)
- Entschieden 15.10.2026: Die Karten bleiben privat, es gibt **keinen** veröffentlichbaren Modus. Google-abgeleitete
  Fassaden dürfen überall verwendet werden.
- Trotzdem trägt jedes erzeugte Objekt seine Quelle (Manifest, `.sco`-Kommentar), damit das später nachrüstbar bleibt.

### F8 Prüfkette
**Erledigt 15.10.2026:** `omsigen/pruefbericht.py` sammelt je Kachel: Spurenden ohne Anschluss und
Beinahe-Anschlüsse (omsigen.ansicht; Kartenrand und vom Korridor abgeschnittene Straßen aus `omsigen.json` zählen
nicht), veraltete/nicht eingesetzte Schichten, veraltete netz.json/Belegung, Gebäude über Fahrbahn, Budgets
(vorläufig: 600 Objekte, 300 000 Dreiecke, 64 MB Texturen je Kachel). Prüforte: `maps/<Karte>/pruefpunkte.json` (der
Generator schreibt die Haltestellen der Strecke hinein, eigene Kamera je Ort möglich), sonst gleichmäßig verteilte
Haltestellen; `--bilder` Spielbild aus openOMSI (Blick die Straße entlang), `--luftbild` Luftbildvergleich.
Ausgabe `maps/<Karte>/pruefung/bericht.json` + `bericht.html`. Der Generator erstellt den Bericht ohne Bilder am Ende;
World Editor → Kacheln bearbeiten → „Prüfung“ zeigt die Funde der Kachel, „Prüfbericht erstellen“, „Bericht öffnen“.
Stand `Aschaffenburg_Linie16_v7`: 186 Funde (3 Spurenden ohne Anschluss = Generator, 183 Gebäude über Fahrbahn).

Je Ebene ein Prüfer (Bauprüfer erweitert), dazu Abgleich mit der Wirklichkeit (Luftbild, Fotos) und Spielbilder aus
openOMSI an festen Prüfpunkten. Ein Lauf erzeugt einen Bericht je Kachel (Funde, Budgets, Bilder). Ziel 0 Funde.

---

## 3. Straßen und Boden (S)

### S1 Querschnitte aus der Wirklichkeit
**Erledigt 15.10.2026 (Abnahme Breiten nicht erreicht, siehe unten):** `omsigen/querschnitte.py` leitet je OSM-Weg den
Querschnitt ab (Spuren je Richtung, `turn:lanes` zählt vor `lanes`, Bus-/Radfahrstreifen, Radweg, Parken neu/alt,
Gehweg je Seite, Beläge, Markierung) – Fahrbahnbreite aus `width`, sonst Regel je Straßenklasse (Median der
`width`-Angaben in OSM um die Linie 16: Wohnstraße 6,0 m, tertiary 7,0 m …). `omsigen/querschnitt_messung.py` misst je
Weg die Gebäudeflucht aus LoD2 (Zwischenspeicher `Kartendaten\querschnitte\messungen.json`); der Querschnitt wird
eingepasst (Achsversatz, wenn die OSM-Linie nicht mittig liegt, dann Seitenraum, dann Spuren). Woher jeder Wert kommt
(tag/messung/regel) steht in `maps/<Karte>/querschnitte.json` und als Attribut `querschnitt` in netz.json. Die `.sli`
baut der Querschnitt-Baukasten des Editors ohne Fenster (`omsi-editor --querschnitte`), geteilt je Typ in
`Splines\Aschaffenburg\Q\` mit `.qs.json` (Baukasten: neues Feld Achsversatz); ohne Editor die alten Standard-Splines
(`--querschnitte standard`). Ähnliche Nachbarn (≤ 0,5 m) werden angeglichen, echte Wechsel bekommen ein gerades
Übergangsobjekt `UE_*` (uebergang.rs, über den Editor) statt einer Kreuzungsplatte; Kreuzungsspuren folgen `turn:lanes`.
Abnahme `omsigen/querschnitt_pruefung.py`: 20 Prüfstellen entlang der Linie 16 (`Kartendaten\querschnitte\pruefstellen.json`,
im Luftbild abgelesen, nur baumfreie Stellen), Funde `querschnitt_abweichung`/`abbiegestreifen_fehlt` im Prüfbericht.
`Aschaffenburg_Linie16_S1` (gegen v7): 79 Funde – Spurenden 3 (dieselben wie v7), Gebäude über Fahrbahn 57 (v7: 183),
Abbiegestreifen 33 geprüft / 0 fehlen; Fahrbahnbreite an den Prüfstellen im Mittel 1,86 m daneben (v7: 2,99 m), 2 von
20 unter 0,5 m (6 bis 0,5 m), Gehwege im Mittel 1,07 m (v7: 1,98 m). Offen: echte Fahrbahnbreiten mit Parkstreifen
(Würzburger Straße bis 15 m) – eine automatische Bordkanten-Messung im DOP20 wurde erprobt und wegen Schatten/Bäumen/Autos
verworfen (3 von 20 auf 0,5 m); nächster Schritt Bordkanten aus den Laserdaten oder Breiten von Hand im Editor
(Vermessen → `messungen.json`, Feld `fahrbahn`).

- Querschnitt je Kante aus OSM-Tags: `width`, `lanes`, `lanes:forward/backward`, `turn:lanes`, `sidewalk`,
  `cycleway*`, `parking:*`, `surface`, `busway`, `maxspeed`; fehlende Werte aus Luftbild-Messung (`aufmass.py`) oder
  Regeln nach Straßenklasse.
- Erzeugt über den Querschnitt-Baukasten (`querschnitt.rs`) statt fester Marcel-Liste; gleiche Querschnitte werden
  geteilt (eine `.sli` je Typ).
- Querschnittswechsel bekommen Übergänge (`uebergang.rs`).
- **Abnahme:** an 20 Prüfstellen Fahrbahn-/Gehwegbreite im Luftbildvergleich < 0,5 m; Abbiegestreifen vorhanden, wo
  OSM `turn:lanes` hat.

### S2 Kreuzungen
- Bordradien aus Daten (Luftbild/OSM, sonst Kartenstudie 10–20 m statt fest 8 m).
- Markierungen: Haltelinien, Fußgängerfurten, Zebrastreifen (`crossing=*`), Abbiegepfeile, Sperrflächen.
- Mehr Beläge (Pflaster, Asphalt hell/dunkel, Radweg rot).
- Fußgängerampeln (Roadmap), Ampelphasen im Editor (TM:PE-Plan).

### S3 Verkehrslogik
- Tempolimits je Kante (`[rule] speedlimit`), Busspuren, Haltebuchten an Haltestellen, Einbahn-Logik wie heute.
- Vorfahrt bleibt im Regelwerk `vorfahrt.py`; Korrekturen über Kartendaten.

### S4 Boden
**Erledigt 15.10.2026:** `omsigen/boden.py` – Malmasken je Kachel (A8-DDS 512 × 512 wie die Standardkarten, Zeile 0 =
Norden) für Gras, Erde, Schotter, Pflaster, Asphalt, Wasser über dem Grund (`texture\gras.bmp`); Klassifikation je
0,5-m-Zelle: DOP20 (Vegetation, offen, dunkel/hell versiegelt, rötlich, Schatten = aus der Nachbarschaft), OSM-Flächen
als Zusammenhang (Park, Feld, Bahn, Baustelle, Wald) und mit festem Belag (Parkplatz, Platz, Fußgängerzone, `surface`),
OSM-Wege mit Breite (Fuß-/Radwege, Feldwege, Zufahrten, Gleise, Bäche), Gewässer, obenauf die Belegung (Fahrbahn/Kreuzung
= Asphalt, Gehweg = Pflaster – Breiten aus netz.json, also nach S1 automatisch richtig); weiche Kanten (~1 m) mit
exakter Umrechnung in Deckkräfte. Texturen: Rheinhausen `gras_wild`, `erde_feld01`, `schotter_neu`,
`str_k_kopfstein_1`, PAD-Labs `Asphalt2` (alle mit Winterfassung), Rückfall auf Standard-OMSI; Gewässer
erzeugt (`Texture\Aschaffenburg\boden_wasser.dds`). Das Luftbild ist nie sichtbare Textur. Quellen und nötige Add-ons
stehen in der Kartenbeschreibung. Generator baut den Boden nach der Belegung; `python -m omsigen.boden <Karte>
[--kacheln]` baut einzelne Kacheln neu; Schicht `boden` in schichten.json (veraltet nach Straßenänderung, „von Hand
geändert“), im World Editor unter Schichten mit „neu bauen“; Prüfbericht: „Boden fehlt“, „Fahrbahn auf Gras“.
Gelände (`hoehen.py`): Böschung 1 : 1,5 (4–30 m breit) statt fester 4-m-Übergang. Geprüft an
`Aschaffenburg_Linie16_S4` (116 Kacheln, 574 Masken: Gras 70 %, Pflaster 14 %, Asphalt 10 %, Erde 2,5 %, Schotter 2 %,
Wasser 2 %): Schöntal = Gras mit Schotterwegen und Teich, Main = Wasser, Altstadt/Höfe = Pflaster, Bahnanlagen =
Schotter, Felder bei Haibach = Erde; 186 Funde wie v7, keine Bodenfunde. Offen: Wasseroberfläche (`tile.map.water`),
Luftbild als großflächige Farbmodulation, eigene Belagsfarben für Tartan/Rotpflaster.
- Malmasken je Kachel (`texture/map/tile_x_y.map.<n>.dds`) mit Ebenen: Gras, Erde, Pflaster, Asphalt, Schotter, Wasser.
- Quelle: OSM landuse/surface + Klassifikation aus DOP20 (wie in `portal_modell.py`, verallgemeinert) + Belegungskarte.
- Echte Texturen: hochwertige Kacheltexturen je Belag (selbst erzeugt oder freie Quellen); das Luftbild höchstens als
  großflächige Farbmodulation (DOP ist CC BY 4.0, Quellenangabe in der Karte).
- Gelände unter Straßen/Plätzen sauber angeglichen; Böschungen statt Stufen.

---

## 4. Gebäude und Objekte (G)

### G1 Platzierung
- Gebäude auf dem **Kartengelände** gründen: Sockel/Schürze bis zum tiefsten Geländepunkt des Grundrisses (statt
  pauschal 1 m), Boden nicht über Gebäude angehoben.
- Konfliktprüfung mit Belegungskarte: kein Gebäude über Fahrbahn/Gehweg/Kreuzungsplatte; Abweichung LoD2↔OSM-Straße
  wird gemeldet, nicht still verschoben.
- Objektursprung = Grundrissmitte, Drehung nach Hauptachse (heute Mittel aller Punkte, Drehung 0).

### G2 Fassaden- und Dachqualität
- Atlas bei großen Gebäuden aufteilen (mehrere Materialien) statt mehrfach verkleinern.
- Fensterformen: Rundbogen/Segmentbogen (Kirchen, Altbau), Schaufenster, Balkone; Schrägansichten (fehlende Fenster).
- Dächer: Gauben, Kamine, Dachfenster, Solarflächen aus Laser/DOP; Dachfarbe aus DOP statt Google.
- Erdgeschoss: Läden/Eingänge aus OSM (`shop`, `entrance`), Schilder.
- Einfache Gebäude (ohne Fassade): Texturfamilien je Funktion/Baujahr/Stockwerkszahl als DDS, Varianten statt einer
  Textur.

### G3 Straßenobjekte
- Laternen (Abstand/Seite nach Straßenklasse, OSM `highway=street_lamp` wo vorhanden), Verkehrszeichen aus OSM
  (heute nur für Vorfahrt genutzt), Poller, Zäune, Bänke, Mülleimer.
- Platzierung über Kanten-ID + Station + Seite (F3) auf Gehweg-Klasse (F4).

### G4 Haltestellen
- Echte Haltestellen: Haltepunkt, Bordhöhe, Bucht oder Kap, Wartehäuschen, Mast mit Name (OSM `public_transport`).
- Linien und Fahrpläne (TTData) – Roadmap 4, z. B. aus GTFS/VAB.

### G5 Leistung
- LOD je Gebäude (fern: ohne Dachdetails, kleinere Mip-Stufe), kleine Nachbargebäude zusammenfassen, Budgets aus F6.

---

## 5. Vegetation und Backdrops (V)

### V1 Daten
- OSM: `landuse` (grass, forest, meadow, allotments), `leisure=park|garden`, `natural=tree|tree_row|wood|scrub`,
  `barrier=hedge`.
- Laser Bayern (Klasse Vegetation, DOM20 – DGM) für Baumstandorte und Höhen; DOP20 für Kronen/Grünflächen.
- Baumkataster Aschaffenburg prüfen (falls offen verfügbar).

### V2 Bäume
- Einzelbäume aus Laser/OSM mit echter Höhe und Kronenbreite (→ `[tree]`-Labels Höhe/Verhältnis), Art nach
  Umgebung (Allee, Park, Wald) aus `Trees_MC` und weiteren freien Sätzen.
- Nur auf erlaubten Belegungsklassen (Grün, Gehweg mit Baumscheibe), Abstand zu Gebäuden/Fahrbahn.
- Jahreszeiten über die vorhandenen Ordner (`spring`, `fall`, `Winter` …).
- Editor: Bäume setzen mit allen Labels (heute vermutlich fehlend), Pinsel für Gruppen.

### V3 Parks, Hecken, Grünflächen
- Bodenmaske Gras/Wiese (S4), Wege im Park, Hecken entlang `barrier=hedge` und Grundstücksgrenzen, Sträucher.

### V4 Backdrops und Kartenrand
- Hinter dem Korridor: einfache LoD2-Hüllen (ohne Fassade) und Waldflächen als Billboard-Gruppen bis zur Sichtweite;
  Hügel aus DGM bleiben sichtbar.
- Kartenrand: Backdrop-Objekte (Silhouetten) statt nacktem Gelände.

### V5 Logische Straßenenden
- Straßen, die den Korridor verlassen, laufen bis in den Backdrop weiter (Stummel bis Kachelrand + Fortsetzung als
  Backdrop), statt im Feld zu enden; KI-Wendeschleifen bleiben unsichtbar wie heute.
- Echte Sackgassen bekommen Wendehammer/Poller nach Daten.

---

## 6. Ablauf

Jede Etappe: **Bauaufnahme/Daten → Erzeugen → Prüfen → im Editor bedienbar → Spielbild → Abnahme durch den Nutzer**.
Pilotgebiet: Linie 16, zuerst die Kachel am Herstallturm (Altstadtrand + Park), dann Klinikum (Hang), dann Haibach
(Wohngebiet).

| Phase | Inhalt | Ergebnis |
|---|---|---|
| 0 | F1 Geobezug, F2 Ebenen-Marker, F3 Netz speichern, F4 Belegungskarte, F8 Prüfbericht | Linie 16 als UTM-Karte, Ebenen einzeln neu baubar |
| 1 | S1 Querschnitte, S4 Bodenmasken | Straßen in echter Breite, Boden mit Belägen |
| 2 | G1 Platzierung, G3 Laternen/Schilder, G4 Haltestellen | Gebäude sitzen, Straßenraum möbliert |
| 3 | V1/V2 Bäume, V3 Parks | Grün an den echten Stellen |
| 4 | S2 Markierungen/Fußgängerampeln, G2 Fassaden, V4/V5 Backdrop und Enden | Feinschliff |
| 5 | F6 LOD/Budgets, G4 Fahrplan | flüssig spielbar, Linie nach Fahrplan |

Phase 1–3 können nach Phase 0 parallel laufen, weil sie nur über Ebenen und Belegungskarte gekoppelt sind.

### Pilotkarte Linie 16 erzeugen
Jede Etappe endet mit einer neuen Version (`_vN`, die alte kommt in den Papierkorb) und deren Prüfbericht:

```
py -3.12 -m omsigen --omsi "C:\Program Files (x86)\Steam\steamapps\common\OMSI 2" --von 49.98015,9.14104 --nach 49.96144,9.22032 --ueber 49.97672,9.14859 --ueber 49.97700,9.15165 --ueber 49.97616,9.15848 --ueber 49.97585,9.16269 --ueber 49.97506,9.16737 --ueber 49.97446,9.17092 --ueber 49.97409,9.17441 --ueber 49.97491,9.18022 --ueber 49.97656,9.18695 --ueber 49.97005,9.19406 --ueber 49.96826,9.19660 --ueber 49.96719,9.20027 --ueber 49.96647,9.20518 --ueber 49.96471,9.20449 --ueber 49.96086,9.20640 --ueber 49.96041,9.21080 --ueber 49.96114,9.21705 --name Aschaffenburg_Linie16_vN --titel "Aschaffenburg Linie 16" --gebaeude 100
py -3.12 -m omsigen.pruefbericht Aschaffenburg_Linie16_vN --bilder --luftbild --max 6
```
Fassaden kommen aus `Documents\Aschaffenburg\fassaden` (earth-reverse-engineering `tools/lod2/korridor.py`).
Stand 15.10.2026 nach Phase 1: `Aschaffenburg_Linie16_v8`, 79 Funde (3 Spurenden, 57 Gebäude über Fahrbahn,
19 Querschnitt-Prüfstellen ≥ 0,5 m vom Luftbild; Boden 0). Vorher v7: 186 Funde.
Hinweis: Der Generator endet mit Rückgabe 1, solange offene Spurenden gefunden werden – den Prüfbericht dann
getrennt starten. Der OSM-Zwischenspeicher liegt relativ zum Arbeitsordner (`.cache`).

### Agenten
- Nach Phase 0 je Bereich ein Agent in einem eigenen Git-Worktree (S, G, V), alle gegen dieselbe Schnittstelle
  (Ebenen-Marker, Netz-Datei, Belegungskarte); ein Prüf-Agent rendert Prüfpunkte und bewertet gegen Luftbild/Fotos.
- Phase 0 macht **ein** Agent bzw. die Hauptsitzung – das Fundament darf nicht parallel entstehen.
- Zusammenführen nur nach grüner Prüfkette; Releases nur auf Wunsch des Nutzers.

---

## 7. Entscheidungen (15.10.2026)

1. **Geobezug:** Der Generator wird ganz auf UTM umgestellt (`omsi-editor-geo.cfg`); Linie 16 wird danach neu erzeugt.
2. **Veröffentlichung:** Alles bleibt privat, kein veröffentlichbarer Modus (F7).
3. **Assets:** Standard-OMSI, eigene erzeugte Objekte und installierte Add-ons. Karten nennen die benötigten Add-ons
   in ihrer Beschreibung (LIESMICH), damit klar ist, was installiert sein muss.
4. **Generator:** Python bleibt der Generator, der Rust-Editor die Oberfläche (wie heute).
5. **Reihenfolge:** Fundament → Straßen/Boden → Gebäude/Objekte → Vegetation/Backdrops.
6. **Texturwerkstatt (Phase 2):** eigene Texturen fuer eigene Objekte aus vier Quellen - Fotos (entzerrt, kachelbar),
   prozedural in Blender (Blender MCP, als DDS gebacken), CC0-Materialien (Poly Haven, ambientCG) und ein lokaler
   KI-Bildgenerator (ComfyUI auf der RTX 3070, mit MCP) fuer Plakate, Ladenschilder, Graffiti und Gebaeude-Varianten.
   Schilder mit Schrift (Haltestelle, Verkehrszeichen) exakt aus Vektoren, nicht per KI.
7. **Werkzeuge:** Blender MCP (Modelle/Texturen ansehen und bauen), QGIS MCP (Ebenen, Luftbild, Belegung gemeinsam
   pruefen) und ComfyUI MCP sind eingerichtet (15.10.2026). Bildgenerator: vorhandene Installation
   `C:\Users\ewanh\ComfyUI` (starten mit `py -3.12 main.py --listen 127.0.0.1 --port 8188`), Modell Juggernaut-XL v9
   (SDXL): 1024x1024 in ca. 18 s, Spitze 7,6 GB von 8 GB. Der Generator ruft ComfyUI direkt ueber dessen HTTP-API auf
   (reproduzierbar: Ablauf + Seed je Textur speichern), das MCP dient der Arbeit in der Sitzung. Schrift erzeugt SDXL
   fehlerhaft - Texte immer aus Vektoren. Kachelbare Texturen brauchen einen Kachel-Modus (nahtlos) bzw. Nacharbeit.
