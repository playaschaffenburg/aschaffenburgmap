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
- Das Straßennetz (Knoten, Kanten, Querschnitte, Regeln) wird als eigene Datei je Karte gespeichert (`netz.json`) und
  beim Öffnen gelesen – nicht nur als Splines.
- Generator und Editor teilen ein Datenmodell (Rust `netz.rs` als Referenz, Python liest/schreibt dasselbe Format).
- Spätere Ebenen beziehen sich auf Kanten-IDs (z. B. „Laterne an Kante 412, Station 35 m, rechts“), nicht auf Koordinaten.

### F4 Belegungskarte
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
Je Ebene ein Prüfer (Bauprüfer erweitert), dazu Abgleich mit der Wirklichkeit (Luftbild, Fotos) und Spielbilder aus
openOMSI an festen Prüfpunkten. Ein Lauf erzeugt einen Bericht je Kachel (Funde, Budgets, Bilder). Ziel 0 Funde.

---

## 3. Straßen und Boden (S)

### S1 Querschnitte aus der Wirklichkeit
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
