# OMSI-Editor (Rust)

Moderner Karteneditor für OMSI 2. Die Darstellung kommt aus [openOMSI](https://github.com/openOMSI-Project/openOMSI)
(MIT-Lizenz, © 2026 usonskyyyy) – die Karte sieht also aus wie im Spiel. omsigen (Python, eine Ebene höher) bleibt
der Generator für OSM-Import, Kreuzungen, Gelände, Brücken und Tunnel.

## Bauen

1. openOMSI klonen nach `..\..\OpenOmsi\source` (Pfad in `Cargo.toml` anpassbar) und den Zweig `editor-api` nehmen
   bzw. den Patch anwenden:
   ```powershell
   cd ..\..\OpenOmsi\source
   git checkout -b editor-api
   git am ..\..\aschaffenburgmap\editor\patches\openomsi-editor-api.patch   # mehrere Commits
   ```
   Der Patch fügt nur `crates/omsi-app/src/viewer.rs` (öffentliche Schnittstelle: Karte öffnen/wechseln, Kacheln
   streamen, Bild zeichnen, Bodenhöhe) und eine Zeile in `lib.rs` hinzu und lädt OpenXR zur Laufzeit (kein CMake nötig).
2. `cargo build --release`
3. `dxcompiler.dll` und `dxil.dll` aus einem openOMSI-Release neben `target\release\omsi-editor.exe` legen
   (ohne sie nimmt wgpu den alten Shader-Compiler FXC, der mit einem Stapelüberlauf abbricht).

## Starten

```powershell
target\release\omsi-editor.exe                                   # Startbildschirm mit der Kartenauswahl
target\release\omsi-editor.exe Grundorf                          # Karte direkt
target\release\omsi-editor.exe Grundorf --bild b.png --cam=x,y,gier,neigung,abstand   # Bild ohne Fenster
target\release\omsi-editor.exe Grundorf --testlauf 20           # Fenster, nach 20 s beenden, Bildrate ausgeben
```

**Start und Karten:** ohne Kartenangabe zeigt der Editor die Kartenauswahl (Liste mit Suche, rechts Vorschaubild
`picture.jpg`, Anzeigename, Beschreibung, Kachelzahl; „eigene“ = vom Editor angelegt oder von omsigen erzeugt).
Doppelklick oder „Öffnen“ lädt die Karte; im Editor öffnet „Karten …“ (oben) dieselbe Auswahl. **Umbenennen …**:
Anzeigename (`[friendlyname]` in global.cfg und allen `global_<Sprache>.dsc` – so heißt die Karte in OMSI) und/oder
Ordner (`maps\<Ordner>`, `[name]`; die eigenen Ordner unter `Aschaffenburg_KI\` und die Verweise der Kacheln darauf
werden mitgenommen; Spielstände der Karte passen danach nicht mehr). **Löschen …**: zwei Rückfragen (die zweite
verlangt den Ordnernamen), dann kommen die Karte und ihre eigenen Ordner unter `Aschaffenburg_KI\` in den
Papierkorb. Die geöffnete Karte lässt sich weder umbenennen noch löschen.
Jede Karte ist eigenständig: beim Speichern (auch „Als neue Karte“) kommen alle Kreuzungsobjekte, auf die sie in
Ordnern anderer Karten verweist, in ihren eigenen Ordner (bei Namensgleichheit unter neuem Namen); vor dem Löschen
oder Umbenennen einer Karte holen sich alle anderen Karten, die ihre Objekte nutzen, diese zuerst. Öffnet man eine
Karte, die noch auf fremde Ordner verweist (ältere Speicherungen), bietet der Editor „In die Karte holen“ an –
fehlende Objekte einer gelöschten Karte holt er aus dem Papierkorb (nur lesend), die Kacheln werden vorher gesichert.

Kamera: rechte Maustaste drehen/neigen, mittlere verschieben, Mausrad zoomen (zum Mauszeiger), W A S D / Pfeile,
Q / E drehen, R / F neigen. Koordinaten wie in openOMSI: x Ost, y Nord, z hoch.

**Objekte bearbeiten (Werkzeug „Objekte“, Taste O):** alle `[object]`-Einträge der Karte, auch die mit absoluter Höhe
(Kreuzungen, Straßenteile, `[absheight]` – sie behalten beim Ziehen ihre Höhe) und die Bäume; angehängte Objekte (`[attachObj]`,
z. B. Signale am Mast) und Spline-Anhänge (Laternenreihen) noch nicht. Klick wählt das Objekt unter der Maus (weißer Ring = unter der
Maus, magenta = gewählt), Ziehen schiebt es über das Gelände, Strg+Mausrad oder `,` `.` dreht (Umschalt: fein),
Bild ↑/↓ hebt/senkt, Entf löscht, „Kopieren“ legt eine Kopie 3 m daneben (danach gewählt); Position und Richtung
auch als Zahlen rechts. Strg+Z / Strg+Y.

**Objekte platzieren (Werkzeug „Platzieren“, Taste P)** wie im Asset-Menü von Transport Fever 2: rechts der Katalog
aller `.sco` unter `Sceneryobjects` als Kachelraster mit Vorschaubildern. Filter: **Herkunft** („OMSI (Standard)“ =
von Grundorf/Spandau genutzt, sonst die Stadt der Karte, die den Objektordner nutzt – Hamburg, Aachen, Bremen, …),
Ordner, `[groups]`, dazu Suche über Name, Datei, Gruppe und Herkunft. Objekt anklicken – es hängt als Vorschau an
der Maus –, Klick in die Welt setzt es (beliebig oft), `,` `.` dreht (15°, Umschalt 1°), Esc beendet. Neue Objekte
lassen sich danach wie alle anderen bearbeiten.

Vorschaubilder: jedes Objekt wird einmal allein mit openOMSI gerendert und als PNG in
`%LOCALAPPDATA%\omsi-editor\vorschau` gespeichert; erzeugt werden nur die gerade sichtbaren, höchstens ~8 ms je Bild.

**Straße bauen (Werkzeug „Straße bauen“, Taste B)** wie in Transport Fever 2: rechts den Querschnitt wählen – Kachelraster
aller `.sli` mit Fahrspuren mit Vorschaubild (gerades Straßenstück), Filter Herkunft, Spuren (1+1, 2+2, Einbahn …),
Gehweg, Ordner und Suche, Modus **Gerade (G)** oder **Kurve (K)**. Klick setzt den
Start, die Maus zieht die Vorschau als echte OMSI-Straße, Klick setzt den nächsten Punkt, Esc/Rechtsklick beendet den
Zug. Freie Straßenenden (grüner Kreis) setzen tangential fort; trifft eine Kurve ein freies Ende, wird sie tangential
eingefädelt (Bogenpaar). Am Mauszeiger: Länge, kleinster Radius, Steigung (rot bei R < 10 m oder > 12 %). Bild ↑/↓:
Höhe des nächsten Punkts über dem Gelände. Entf: Straße unter der Maus löschen. Strg+Z / Strg+Y.

**An vorhandene Straßen anschließen:** blaue Kreise mit Strich zeigen freie Enden vorhandener Straßen (Richtung, in
der es weitergeht). „Frei“ entscheidet openOMSIs Spurnetz – es verknüpft Spuren wie das Spiel, auch mit den Pfaden
der Kreuzungsobjekte; Enden am Rand des geladenen Bereichs gelten nicht als frei. Start oder Ziel rasten dort ein;
die neue Straße übernimmt Richtung, Höhe und Steigung. Passen die Fahrspuren des gewählten Querschnitts nicht, wird
beim Start der vorhandene übernommen (abschaltbar), am Ziel erscheint eine Warnung.
Gespeichert wird jede Straße als Kette von `[spline_h]`-Einträgen (glatter Höhenverlauf, Enden exakt auf den Knoten
in gleicher Richtung – so verbindet OMSI die Spuren).

**Kreuzungen – eine Logik für alle Fälle** (Werkzeug „Straße bauen“): jede Kreuzung ist ein Knoten des Netzes mit
Armen; ein Arm ist eine eigene Straße oder ein Ende einer vorhandenen Straße, die dort aufgeschnitten wurde
(„Kartenarm“). Ab drei Armen entsteht ein Kreuzungsobjekt wie in den Standardkarten (Platte mit Bordsteinecken und
Gehwegen, Abbiegespuren für alle Richtungen, Vorfahrt), erzeugt von omsigen (`python -m omsigen.editorkreuzung`).
Die Arme enden so weit vor der Mitte, dass sie an den Nachbararmen vorbeikommen (Breiten und Winkel, ab 35°), und
laufen eben auf Kreuzungshöhe ein. Orange Markierungen zeigen in der Vorschau jede geplante Kreuzung.
- **T-Kreuzung / Abzweig:** Start oder Ziel mitten auf einer Straße (eigener oder vorhandener) – sie wird geteilt
  bzw. aufgeschnitten. Nahe einem Ende rückt die Kreuzung in die Straße hinein, nahe einem freien Ende einer
  vorhandenen Straße wird dort angeschlossen; ein Knick-Knoten wird mit einer dritten Straße zur Kreuzung.
- **Kreuzen (4 und mehr Arme):** läuft die neue Straße über eine andere (eigene oder vorhandene) auf gleicher Höhe,
  entsteht dort eine Kreuzung; bei mehr als 3 m Höhenunterschied nicht (Brücke/Tunnel). Zu flach (unter 30°), zu
  nah an einem Knoten oder Kreuzungen zu dicht hintereinander: steht in der Vorschau, es wird nicht gebaut.
- **An vorhandene Kreuzungen anschließen:** Start oder Ziel auf einem vorhandenen Kreuzungsobjekt (z. B. einer
  Standardkreuzung, auch mit einem Arm ohne Pfade): es wird in der Sitzungskopie entfernt – mit seinen Ampeln und
  deren Masten – und durch eine eigene Kreuzung ersetzt, deren Arme die vorhandenen Straßenenden und die neue Straße
  sind. Offene Arme von Kreuzungsobjekten (Pfade, die nirgends hinführen) sind außerdem normale Anschlusspunkte.
- **Kreisverkehr (Modus V):** Klick setzt die Mitte, die Maus die Größe (Durchmesser 24–120 m), Klick baut einen Ring
  aus Einbahn-Straßen gegen den Uhrzeigersinn (Querschnitt rechts wählbar, Vorschlag: Einbahn mit Gehweg). Zufahrten
  auf den Ring ziehen: dort entstehen T-Kreuzungen. Straßen, die der Ring kreuzt, bekommen Kreuzungen.
- **Vorfahrt (vermutet, später per Klick änderbar):** im Kreisverkehr der Ring; sonst die durchgehende Straße – eine
  vorhandene vor einer neuen, dann gleicher Querschnitt, dann die breitere; zwei gleichwertige: rechts vor links.
- Ein Strg+Z nimmt Straße, Kreuzungen und aufgeschnittene vorhandene Straßen zurück. Beim Speichern kommen die
  Kreuzungsobjekte nach `Sceneryobjects\Aschaffenburg_KI\<neue Karte>\`. Python: `python` aus dem Pfad (oder
  `OMSIGEN_PYTHON`), omsigen aus diesem Repository (oder `OMSIGEN_DIR`).

**Vorfahrt und Ampel (Werkzeug „Kreuzungen“, Taste X):** im Bild zeigen farbige Linien die Vorfahrt jeder Kreuzung
(grün Hauptstraße, rot wartet, gelb rechts vor links). Klick wählt eine Kreuzung; rechts je Arm „Hauptstraße“,
„wartet“ oder „rechts vor links“, dazu „alle rechts vor links“, „Vorfahrt vermuten“ (zurück zur Regel des Editors)
und der Schalter **Ampel**: omsigen baut dann die Signalanlage ins Kreuzungsobjekt (`[traffic_lights_group]`,
gegenüberliegende Arme gemeinsam grün, die Hauptstraße zuerst und länger, Umlauf 70–100 s, wie ampel.py) und stellt
an jede Zufahrt Signal und Mast (Ampel_Kfz_1 auf whip_beam, Signal am Ausleger), beim Speichern mit `[varparent]`
auf die Kreuzung wie in den Standardkarten. Vorhandene Kreuzungen der Karte (orange beim Darüberfahren) werden beim
Klick durch eine eigene mit denselben Armen ersetzt und lassen sich dann genauso einstellen. Die Regel hängt an den
Armrichtungen und bleibt erhalten, wenn später Straßen dazukommen (neue Arme warten bzw. rechts vor links).
Strg+Z nimmt jede Änderung zurück. Klick auf eine schon gespeicherte eigene Kreuzung baut sie aus den vorhandenen
Straßenenden neu (so lassen sich ältere Kreuzungen mit schief angesetzter Platte reparieren).

**Eigene Straßen ändern:** im Werkzeug „Ändern“ lassen sich auch die eigenen Straßen wählen (Umschalt+Klick: Kette)
und umkehren, löschen oder auf einen anderen Querschnitt setzen. Wird eine Straße gelöscht, an der eine vorhandene
aufgeschnitten war, schließt ein Stück in deren Querschnitt die Lücke wieder.

**Rückgängig/Wiederholen** (Strg+Z / Strg+Y) gilt über alle Werkzeuge: der jeweils letzte Schritt, egal in welchem
Werkzeug man gerade ist (eine gebaute Straße samt Kreuzungen und aufgeschnittenen Straßen ist ein Schritt).

**Vorhandene Straßen ändern (Werkzeug „Ändern“, Taste U)** wie das Upgrade-Werkzeug in Transport Fever 2: die Straße
unter der Maus wird umrissen, Klick wählt den Spline (Umschalt+Klick: ganze verknüpfte Kette, Strg+Klick: dazu/weg).
Rechts: Daten des Splines, **Richtung umkehren** (`mirror`), **Löschen** (Entf) und **Upgrade** auf einen anderen
Querschnitt (Raster mit Vorschaubildern; Warnung, wenn die Fahrspuren nicht mehr zu den Nachbar-Splines passen).
Technik: die geänderte Kachel wird in einen Sitzungsordner geschrieben, den openOMSI vor der
Installation liest, und neu geladen – sofort sichtbar, die Originalkarte bleibt unverändert; beim Speichern als
neue Karte kommen diese Kacheln mit.

**Speichern (Strg+S)** überschreibt die geöffnete Karte: jede Datei, die dabei ersetzt wird (Kacheln, `global.cfg`),
kommt vorher nach `%LOCALAPPDATA%\omsi-editor\sicherungen\<Karte>\<Zeit>\` (die letzten 10 je Karte bleiben). Eigene
Karten (vom Editor angelegt – Markierung `omsi-editor.txt` – oder von omsigen erzeugt) werden ohne Rückfrage
gespeichert, bei Standard- und Fremdkarten fragt der Editor nach und bietet „Als neue Karte“ an. Danach wird die Karte
frisch geladen (Kamera bleibt). Kreuzungsobjekte kommen in `Sceneryobjects\Aschaffenburg_KI\<Karte>\` (eindeutige
Namen je Sitzung, nichts Vorhandenes wird ersetzt).
**Als neue Karte (Strg+Umschalt+S)**: der Kartenordner wird kopiert, geänderte Kacheln (UTF-16 bleibt) kommen
darüber, neue Objekte stehen als `[object]`-Einträge mit eindeutigen IDs in ihrer Kachel, `global.cfg` bekommt den
neuen Namen und `[NextIDCode]`; die geöffnete Karte bleibt unverändert.

**Protokoll und Berichte** (Knopf „Protokolle“ oben öffnet den Ordner `%LOCALAPPDATA%\omsi-editor\logs`): jede
Sitzung schreibt eine Logdatei (die letzten 10 bleiben; Stufe über `RUST_LOG`, Standard `warn,omsi_editor=info`),
beim Kreuzungsbau jeden Schritt mit Zeit. Absturz: `absturz-<Zeit>.txt` mit Stelle, Backtrace und den letzten
Protokollzeilen. Hänger: ein Wächter merkt, wenn das Fenster 8 s kein Bild zeichnet (beim Kartenöffnen 60 s), und
schreibt `haenger-<Zeit>.txt` mit der laufenden Aktion. Beim nächsten Start meldet die Statuszeile den Bericht.
Ausprobieren: `omsi-editor.exe Grundorf --absturztest` bzw. `--haengertest`.

Tests: `cargo test --release` (ohne OMSI), `cargo test --release -- --include-ignored` (mit OMSI und Grafikkarte;
`dxcompiler.dll`/`dxil.dll` auch nach `target\release\deps`).

## Stand und Plan

1. **Betrachter** (fertig): beliebige Karte, Darstellung wie im Spiel, Kacheln werden im Hintergrund um den
   Blickpunkt gestreamt (Worker-Thread, 6 ms Hochladen je Bild), Kartenwechsel ohne neuen Renderer
2. **Bearbeiten** (Objekte fertig): auswählen, ziehen, drehen, heben, löschen, kopieren, aus dem Katalog
   platzieren, Rückgängig, Speichern als neue Karte. Splines verschieben kommt mit dem Netz-Kern (Schritt 3) – das kann auch openOMSI selbst noch nicht
3. **Straßen** (in Arbeit): Netz-Kern (Knoten, Kanten aus Geraden/Bögen, glatte Höhe), Werkzeug Gerade/Kurve mit
   Live-Vorschau und Einrasten, Speichern als Splines, Anschluss an freie Enden vorhandener Straßen, Ändern/Upgrade,
   allgemeine Kreuzungslogik: T, Kreuzen (4+ Arme), Kreisverkehr, über eigene und vorhandene Straßen (fertig);
   Vorfahrt/Ampel per Klick (fertig); Kreisverkehr-Optik (Platten am gebogenen Ring), Knoten ziehen, Ampelzeiten
   einstellen, Fußgängerampeln (offen)
4. omsigen-Funktionen: OSM-Import, Kreuzungsgenerator, DGM-Gelände, Luftbild
