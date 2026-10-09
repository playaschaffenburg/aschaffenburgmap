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
Ordner (`maps\<Ordner>`, `[name]`; die eigenen Ordner unter `Aschaffenburg\` und die Verweise der Kacheln darauf
werden mitgenommen; Spielstände der Karte passen danach nicht mehr). **Löschen …**: zwei Rückfragen (die zweite
verlangt den Ordnernamen), dann kommen die Karte und ihre eigenen Ordner unter `Aschaffenburg\` in den
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

**Pipette (Knopf oben, Taste I):** der nächste Klick nimmt, was unter der Maus liegt, und wechselt ins passende
Werkzeug – ein Objekt oder Baum ins Platzieren (samt Drehung, der Katalog springt auf den Eintrag), eine Straße
(vorhandene oder eigene) in „Straße bauen“ mit ihrem Querschnitt, ein Kreisverkehr in den Modus Kreisverkehr. Gelb
markiert, was genommen würde; Esc bricht ab. Im Platzieren geht außerdem Strg+Klick, im Werkzeug „Objekte“ übernimmt
„Pipette: zum Platzieren übernehmen“ das gewählte Objekt.

Vorschaubilder: jedes Objekt wird einmal allein mit openOMSI gerendert und als PNG in
`%LOCALAPPDATA%\omsi-editor\vorschau` gespeichert; erzeugt werden nur die gerade sichtbaren, höchstens ~8 ms je Bild.

**Straße bauen (Werkzeug „Straße bauen“, Taste B)** wie in Transport Fever 2: rechts den Querschnitt wählen – Kachelraster
aller `.sli` mit Fahrspuren mit Vorschaubild (gerades Straßenstück), Filter Herkunft, Spuren (1+1, 2+2, Einbahn …),
Gehweg, Ordner und Suche, Modus **Gerade (G)** oder **Kurve (K)**. Klick setzt den
Start, die Maus zieht die Vorschau als echte OMSI-Straße, Klick setzt den nächsten Punkt, Esc/Rechtsklick beendet den
Zug. Freie Straßenenden (grüner Kreis) setzen tangential fort; trifft eine Kurve ein freies Ende, wird sie tangential
eingefädelt (Bogenpaar). Am Mauszeiger: Länge, kleinster Radius, Steigung (rot bei R < 10 m oder > 12 %). Bild ↑/↓:
Höhe des nächsten Punkts über dem Gelände. Entf: Straße unter der Maus löschen. Strg+Z / Strg+Y.

**Querschnitt-Baukasten** („Straße bauen“ → „Querschnitt-Baukasten …“; eigener Querschnitt gewählt: „bearbeiten“):
eigene Straßen-Splines aus Teilen von links nach rechts – **Fahrspur** (Richtung ▲ vor = mit dem Spline / ▼ zurück),
**Busspur**, **Radfahrstreifen**, **Parkstreifen** (Fahrbahnhöhe 0,10 m), **Gehweg**, **Radweg**, **Grünstreifen**,
**Mittelinsel** (Hochbord 0,25 m), je mit Breite (0,3–20 m) und Belag (Asphalt, roter Asphalt, Gehwegplatten,
Verbund-, Kopfsteinpflaster, Betonplatten, Gras). Markierungen je Grenze automatisch (zwei Spuren: Leitlinie 3 m / 6 m;
Bus-/Radfahrstreifen: Breitstrich; Parkstreifen: durchgezogen) oder von Hand (keine, Leitlinie, durchgezogen, doppelt,
Breitstrich). Skizze maßstäblich mit Richtungspfeilen, Markierungen und Spline-Achse (Mitte der Fahrspuren; Klick wählt
ein Teil), 3D-Vorschau aus openOMSI. Die Spline bekommt Bordsteine an jedem Höhenwechsel, gekachelte Beläge (breite
Teile in Streifen, damit die Textur nicht verzerrt), Markierungen als eigene Profile 1 cm über der Fahrbahn
(`[matl_alpha] 1`), `[heightprofile]` je Teil und `[path]` je Fahrspur (Richtung) und Gehweg – das sieht die KI.
Gespeichert nach `Splines\Aschaffenburg\AB_<Name>.sli` mit Bauanleitung `AB_<Name>.qs.json` (wieder öffnen und ändern);
Texturen in `Splines\Aschaffenburg\texture` (Beläge aus `Splines\Marcel\texture` kopiert, Markierungen und roter
Asphalt erzeugt). Herkunft im Katalog: „Eigene (Baukasten)“. „Speichern und damit bauen“ wechselt gleich zu Straße bauen.

**Brücken und Rampen** (wie Transport Fever 2; Höhe der Punkte beim Bauen mit Bild ↑/↓): was gebaut wird, folgt aus
Lage und Höhe der eigenen Straßen und dem Gelände darunter und wird nach jeder Änderung neu berechnet – Strg+Z auf die
Straße nimmt Brücke, Damm und Mauern mit zurück. Je Spline-Stück (höchstens 20 m): mit wenigstens „Brücke ab“ m Luft
(Standard 5 m) wird es eine **Brücke** – Begleit-Spline `Splines\Aschaffenburg\AB_bruecke_<l>_<r>.sli` mit Platte
(1,2 m) und Brüstung (1 m) auf genau derselben Kurve wie die Fahrbahn (ohne Pfade, die Spuren bleiben unberührt; wie
in den Standardkarten), Pfeiler als Objekte an den Stückgrenzen, wo genug Luft ist (nicht auf vorhandenen Straßen).
Darunter **Rampen** nach der gewählten Bauweise: **Damm** (Gelände unter der Straße eben aufgeschüttet, daneben
Böschung 1 : 1,5; liegt die Straße tiefer, ein Einschnitt) oder **Stützmauer** (für die Stadt, wie am Dr.-Willi-Reiland-Ring): Mauer-Objekte an beiden Rändern, deren
Oberkante an jeder Stelle dem Gelände folgt – im Einschnitt knapp über Geländehöhe, an der Rampe 0,6 m über dem Gehweg –,
zur Straße hin Klinker (abschaltbar: Beton), Betonkappe, Geländer (Pfosten alle 2 m, Hand- und Knieleiste). Im Einschnitt
wird das Gelände 8 m über die Mauer hinaus auf die Sohle abgesenkt (das 5-m-Raster kann keine senkrechte Kante) und ein
Deckel in der Bodentextur der Karte (Detailtextur in der mittleren Farbe der Grastextur, Weltkoordinaten) liegt auf der
alten Höhe darüber – so baut omsigen auch an Tunneln. Vorhandene Straßen der Karte werden
nicht zugeschüttet. Reihenfolge wie in TPF 2: erst graben tiefer liegende Straßen ihre Einschnitte, dann entscheidet sich gegen dieses
Gelände, was Brücke wird (eine Straße über einem Einschnitt wird dort zur Brücke), dann wird aufgeschüttet – wo beides
verlangt ist, gewinnt der Einschnitt. Kreuzungsflächen werden wie Straßen aufgeschüttet bzw. eingeschnitten; liegt eine
Kreuzung hoch (über einem Einschnitt), bekommt sie einen **Sockel** (Platte mit Seitenwänden, `Sockel_*.sco`).
Pfeiler nie auf einer tieferen Straße. Vorschau sofort (Brückenkörper, Pfeiler, Sockel, Gelände); beim Speichern kommen Begleit-Splines, Pfeiler
(`Sceneryobjects\Aschaffenburg\<Karte>\Pfeiler_*.sco`, `[absheight]`) und die `.terrain` der Kacheln in die Karte.

**Übergänge zwischen Querschnitten** („Straße bauen“ → „Übergang setzen“): Klick auf ein freies Straßenende (blau) setzt
dort einen Übergang vom Querschnitt dieser Straße auf den gewählten – ein Objekt (`.sco` + `.x` + Pfade) wie die
Kreuzungen, denn eine OMSI-Spline hat über ihre ganze Länge dasselbe Profil. Teile beider Seiten werden einander
zugeordnet (gleiche Art bevorzugt, gleiche Höhe Pflicht, Fahrspuren nur gleicher Richtung; was keine Entsprechung hat,
läuft auf Breite 0 aus – außen eher als innen), Breiten und Lage gehen S-förmig ineinander über; Modell mit gekachelten
Belägen, Bordsteinkanten und Markierungen wie im Baukasten. Pfade: Spuren je Richtung von der Gegenfahrbahn aus gepaart,
zusätzliche fädeln in die nächste ein bzw. zweigen ab (S-Kurve aus zwei Bögen), Gehwege laufen durch; die Pfadenden
liegen genau auf den Spuren der Splines. Beide Seiten können Baukasten-Querschnitte sein (aus der Bauanleitung) oder
beliebige `.sli` (Flächen aus `[heightprofile]`, Spuren aus `[path]`). Länge automatisch (8 m je Meter Versatz, mindestens
15 m) oder fest. Vorschau: Umriss und „A (Spuren, Breite) → B …“ am Mauszeiger. Danach am freien Ende des Übergangs mit
dem neuen Querschnitt weiterbauen. Das Objekt liegt bis zum Speichern im Sitzungsordner und kommt dann nach
`Sceneryobjects\Aschaffenburg\<Karte>\` (Strg+Z nimmt ihn zurück). **Steigung:** der Übergang übernimmt die Steigung
des Straßenendes (Modell und Pfade steigen gleichmäßig – Objektpfade haben dafür eine Höhenänderung, siehe
docs/omsi-format.md), der offene Arm am anderen Ende meldet sie weiter, die nächste Straße setzt sie fort. **Eigene
Straßen:** auch an freien Enden eigener, noch nicht gespeicherter Straßen; beim Speichern endet deren Spline genau am
Übergang (Lage, Höhe, Richtung, Steigung).

**Messen (M, Knopf oben):** Klick setzt den Anfang, zweiter Klick das Ende; angezeigt werden die waagerechte Länge und
der Höhenunterschied (mit Steigung). Im Baukasten lässt sich die letzte Messung als Breite des gewählten Teils
übernehmen oder der ganze Querschnitt auf sie strecken – so passt der Querschnitt zum Luftbild.

**An vorhandene Straßen anschließen:** blaue Kreise mit Strich zeigen freie Enden vorhandener Straßen (Richtung, in
der es weitergeht). „Frei“ entscheidet openOMSIs Spurnetz – es verknüpft Spuren wie das Spiel, auch mit den Pfaden
der Kreuzungsobjekte; Enden am Rand des geladenen Bereichs gelten nicht als frei. Start oder Ziel rasten dort ein;
die neue Straße übernimmt Richtung, Höhe und Steigung. Passen die Fahrspuren des gewählten Querschnitts nicht, wird
beim Start der vorhandene übernommen (abschaltbar), am Ziel erscheint eine Warnung.
Gespeichert wird jede Straße als Kette von `[spline_h]`-Einträgen (glatter Höhenverlauf, Enden exakt auf den Knoten
in gleicher Richtung – so verbindet OMSI die Spuren).

**Kreuzung erstellen** (Werkzeug „Kreuzungen“ → „Kreuzung erstellen“): Klick auf freie Straßenenden (blau, Enden
vorhandener Straßen der Karte und eigener Straßen) wählt sie aus bzw. ab (nummeriert, orange); „Kreuzung bauen“ macht
aus allen gewählten **ein** Kreuzungsobjekt – ein Knoten nur mit Armen an genau diesen Enden, die Straßen bleiben, wie
sie sind; omsigen baut Platte, Bordsteinecken, Abbiegespuren und Vorfahrt und schließt die Lücke. Gedacht für eng
aufeinanderfolgende Kreuzungen (alle Enden beider zusammen wählen) und verunglückte Stellen; eine vorhandene Kreuzung
im Weg vorher mit Ändern löschen. Danach wie jede eigene Kreuzung: Vorfahrt/Ampel, Spurpfeile, beim Speichern in die
Karte; Strg+Z nimmt sie zurück. Mindestens 3 Enden. Die Kreuzung ist eben: auf der Höhe der Kartenstraßen; eigene Straßen werden an ihrem Ende auf
diese Höhe gebracht (ihr Höhenverlauf passt sich an); enden Kartenstraßen verschieden hoch, wird gewarnt. Auch Enden am
Rand des geladenen Bereichs sind wählbar. **Beim Bauen:** bleibt zwischen zwei Kreuzungen kein Straßenstück, werden sie
automatisch zu einer Kreuzung vereint (statt „zu kurz“).

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
- **Kreisverkehr (Modus V):** Klick setzt die Mitte, die Maus die Größe (Durchmesser 24–120 m), Klick baut. Er ist
  **ein Objekt** wie in Rheinhausen (`omsigen/kreisel.py`): runde Ringfahrbahn (7 m, gegen den Uhrzeigersinn), Insel
  mit Pflasterrand, Gehweg außen herum, Zufahrten mit ausgerundeten Ecken; der Ring hat Vorfahrt. Zufahrten: eine
  Straße auf den Kreisverkehr ziehen (sie endet außerhalb des Rings). Liegt er über vorhandenen Straßen (auch über
  einer vorhandenen Kreuzung), fällt alles innerhalb weg – Straßenstücke, Kreuzungsobjekte mit Ampeln und Masten –
  und jede Straße wird eine Zufahrt. Mit dem Knoten-Werkzeug lässt er sich samt Zufahrten verschieben.
  Kein Querschnitt nötig; nach dem Bau wechselt der Modus auf Kurve für die Zufahrten. Im Bild zeigen zwei blaue
  Kreise Ringrand und Armlinie (dort enden die Zufahrten). Zufahrten brauchen mindestens 30° Abstand; eine Straße
  quer durch den Kreisverkehr wird abgelehnt; Entf auf dem Ring löscht ihn mit seinen Zufahrten.
- **Vorfahrt (vermutet, später per Klick änderbar):** im Kreisverkehr der Ring; sonst die durchgehende Straße – eine
  vorhandene vor einer neuen, dann gleicher Querschnitt, dann die breitere; zwei gleichwertige: rechts vor links.
- Ein Strg+Z nimmt Straße, Kreuzungen und aufgeschnittene vorhandene Straßen zurück. Beim Speichern kommen die
  Kreuzungsobjekte nach `Sceneryobjects\Aschaffenburg\<neue Karte>\`. Python: `python` aus dem Pfad (oder
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

**Spurpfeile und Verbinder (Werkzeug „Kreuzungen“, Ansicht „Spuren“)** wie Traffic Manager: President Edition:
Klick auf eine Kreuzung wählt sie (eigene oder vorhandene der Karte – die wird dabei nicht ersetzt). Im Bild: weiße
Punkte = Zufahrtsspuren, Linien = Abbiegespuren (grün erlaubt, rot für die KI gesperrt). Klick auf einen Punkt wählt
die Spur, rechts ihre Pfeile (← links, ↑ geradeaus, → rechts, ↶ wenden) zum An-/Abschalten; Klick auf eine Linie
schaltet diese Abbiegespur. Eigene Kreuzungen werden mit genau den gewählten Abbiegespuren neu gebaut (omsigen,
`verbindungen`; auch neue wie Wenden oder Linksabbiegen von einer anderen Spur), „Vorschlag wiederherstellen“ geht
zurück. Kreuzungen der Karte: vorhandene Abbiegespuren werden per `[rule] no_cars` auf ihren Pfaden gesperrt (Pfade,
die eine noch erlaubte Verbindung braucht, bleiben frei); die Verbindungen liest der Editor aus openOMSIs Spurnetz.

**Eigene Straßen ändern:** im Werkzeug „Ändern“ lassen sich auch die eigenen Straßen wählen (Umschalt+Klick: Kette)
und umkehren, löschen oder auf einen anderen Querschnitt setzen. Wird eine Straße gelöscht, an der eine vorhandene
aufgeschnitten war, schließt ein Stück in deren Querschnitt die Lücke wieder.

**Verlauf ändern (Werkzeug „Knoten“, Taste N)** wie in Transport Fever 2: Knoten greifen und ziehen, die Straße legt
sich in glatten Bögen neu. Griffe (im Umkreis der Maus): blau die Verbindung zweier Splines einer Karte oder ein
freies Straßenende, grün die Knoten eigener Straßen, orange Quadrat eine vorhandene Kreuzung; **Umschalt** + irgendwo
auf einer Straße ziehen setzt dort einen neuen Knoten (der Spline wird geteilt). Kurze Splines (an Kreuzungen oft
unter 10 m) werden mit ihren verknüpften Nachbarn gleichen Querschnitts neu gelegt, bis mindestens 30 m Strecke da
sind – sonst entstünden enge S-Kurven; Kurven enger als der Querschnitt breit ist, werden abgelehnt. Ersetzt werden nur die Splines am gezogenen
Knoten: durch Bögen (`netz::verbinden`, Bogen oder Bogenpaar), die an ihren festen Enden in Lage, Richtung, Steigung
und Querneigung genau wie vorher anschließen – die Spuren zu den Nachbarn bleiben verbunden, Querschnitt und
`mirror` bleiben. Am Knoten bleibt der Verlauf knickfrei (die Richtung dreht sich mit den Sehnen zu den festen
Enden), die Höhe folgt dem Gelände. Beim Ziehen: **Mausrad** dreht die Richtung am Knoten (Umschalt: fein), **Bild
↑/↓** hebt/senkt ihn, Pos1 setzt zurück, Esc bricht ab. Vorschau blau, orange bei engen Kurven (R < 12 m), rot wenn
es so nicht geht (Schleife, Knoten hinter dem festen Ende). Eine **Kreuzung** wird mit ihren Ampeln und Masten
verschoben (nicht gedreht), die angeschlossenen Splines folgen. Eigene Knoten: an Kreuzungen mit aufgeschnittenen
vorhandenen Straßen und an Anschlüssen werden deren Enden mitgezogen, das Kreuzungsobjekt wird neu erzeugt. Jeder Zug
ist ein Rückgängig-Schritt. Technik: das erste neue Stück behält die Spline-ID (Vorgänger zeigen weiter darauf),
weitere bekommen neue IDs, Nachfolger werden umgehängt (`Aendern::umlegen` in `knoten.rs`).

**Hilfsansicht (Taste H, Schalter „Pfade“ oben)** wie „Show paths“ im nEditor: die Pfade der geladenen Kacheln als
Linien mit Pfeil in Fahrtrichtung – blau Fahrspuren der Straßen, gelb Pfade der Kreuzungsobjekte, grün Gehwege, orange
Gleise, lila unsichtbare Straßen (`[onlyeditor]`-Splines) – und die Objekte, die das Spiel nicht zeichnet
(`[onlyeditor]`: Haltestellen-Marken, Einstiegspunkte, Schallquellen …) mit ihrem Editor-Modell und einer Raute
(Name beim Darüberfahren). Solange die Hilfsansicht an ist, lassen sich diese Objekte im Werkzeug „Objekte“
wählen, verschieben, drehen, kopieren und löschen; im Platzieren-Katalog zeigt der Filter **Art → Editor-Objekte**
nur sie (z. B. `Generic\entrypoint_bus.sco`, Haltestellen-Marken, Schallquellen).

**World Editor (Knopf oben) → Kacheln bearbeiten:** im Bild sind die Kacheln der Karte weiß umrandet, freie Plätze
am Rand mit „+“ grün. Klick auf ein „+“ legt dort eine 300-m-Kachel an: leere Kacheldatei, Gelände (`.terrain`), das
an den Rändern genau an die Nachbarkacheln anschließt und dazwischen nach Abstand gemittelt ist, Lichtkarte aus der
OMSI-Vorlage – im Sitzungsordner. Klick auf eine Kachel wählt sie, „Kachel löschen“ bzw. Entf nimmt sie nach Rückfrage
(Inhalt, Warnung bei Fahrwegen `TTData/*.ttr`) aus der Karte. Neue Kacheln kommen ans Ende der `[map]`-Liste; beim
Speichern wird die Liste neu geschrieben und die Kachelnummern der Einsetzpunkte (global.cfg) und von
`TTData/Busstops.cfg` umgerechnet (Einträge auf gelöschten Kacheln fallen weg). Jeder Schritt ist im gemeinsamen
Rückgängig. Technik: openOMSI `Viewer::set_map_tiles` (Kachelliste zur Laufzeit).

**World Editor → Gelände formen** (wie Transport Fever 2): runder Pinsel, wirkt solange die linke Maustaste gedrückt
ist. Modi **Anheben**, **Absenken** (Strg kehrt jeweils um), **Glätten** (gleicht Stufen, Kanten und Huckel aus; das
Mittelungsfenster wächst mit dem Pinsel) und **Ebnen** (auf die Höhe beim Ansetzen oder eine feste Zielhöhe – Strg+Klick
greift sie ab –, wahlweise **eingerastet** auf 0,5/1/2,5/5 m; fester Kern mit weichem Rand). Radius 5–200 m
(Strg+Mausrad), Stärke (Umschalt+Mausrad). „Straßen schützen“ (Standard): wo eine Straße, ein Gehweg oder eine
Kreuzung auf dem Boden liegt (bis 1,5 m darüber), bleibt das Gelände – OMSI schneidet es unter Straßen nicht aus;
unter Brücken wird geformt. Im Bild: Pinselkreis auf dem Gelände in der Farbe des Modus, Höhe unter der Maus (beim
Ebnen „jetzt -> Ziel“). Der Pinsel arbeitet auf dem Weltraster (5 m), Kachelränder bleiben dicht. Während des Strichs
zeigt openOMSI das Gelände sofort (`Viewer::preview_terrain`: nur die Höhen des Geländenetzes); beim Loslassen kommen
die `.map.terrain` als Kopien in den Sitzungsordner und die Kacheln werden neu gelesen (Objekte und Bäume stehen
wieder auf dem Boden). Jeder Strich ist ein Schritt im gemeinsamen Rückgängig; Speichern übernimmt die Dateien.

**World Editor → Einsetzpunkte / Haltestellen:** Listen mit „hin“ (Kamera), „umbenennen“, „löschen“; Name eingeben
und „+ setzen“, dann Klick ins Bild: der Punkt kommt auf die nächste Fahrspur (bis 8 m), in deren Richtung.
Einsetzpunkte sind `Generic\entrypoint_bus.sco` + Eintrag in global.cfg `[entrypoints]`, Haltestellen
`Generic\bus_stop.sco` (Name = erster Text) + Eintrag in `TTData/Busstops.cfg`; Haltestellen, die in Busstops.cfg
fehlen, lassen sich aufnehmen. Die Objekte stehen sofort in den Sitzungskopien der Kacheln (verschieben/drehen mit dem
Objekte-Werkzeug bei Pfad-Ansicht H); beim Speichern werden alle Einsetzpunkte aus den fertigen Kacheln neu berechnet
(Index in der Kachel – alle Objekt-Einträge gezählt wie die „Object Nr.“-Kommentare –, Lage, Drehung, Kachelnummer)
und Busstops.cfg neu geschrieben. Ein Strg+Z nimmt Objekt und Listeneintrag zusammen zurück.

**Neue Karte (Kartenauswahl → „+ Neue Karte erstellen …“):** Anzeigename, Ordner (aus dem Namen vorgeschlagen,
Umlaute umschrieben) und Beschreibung eingeben, „Erstellen und öffnen“. Die Karte entsteht aus OMSIs Vorlage
`template\NewMap`: Kachel 0 0 (300 × 300 m, flach) mit einem 120 m langen geraden Straßenstück (Nord–Süd durch die
Mitte, Querschnitt aus `Splines\Marcel`) und darauf einem Einsetzpunkt „Start“ in der rechten Fahrspur; global.cfg
bekommt Namen, Beschreibung, `[NextIDCode]`, Kamera und `[entrypoints]`. Sie trägt die Marke des Editors (Speichern
ohne Rückfrage).

**Neue Karte an einem echten Ort:** im Dialog „Ort“ suchen (Name/Adresse über Nominatim/OpenStreetMap, oder direkt
Koordinaten „49.97, 9.14“), einen Treffer wählen, Anfangsgebiet 1, 3 × 3 oder 5 × 5 Kacheln. Der Ort liegt in der Mitte
der Kachel 0 0; die Karte liegt im **UTM-Gitter** (Datei `omsi-editor-geo.cfg` im Kartenordner: Ort, UTM-Zone,
UTM-Koordinaten des Ursprungs, Höhe über NN, die in OMSI 0 ist – OMSI liest sie nicht, sie wandert beim Speichern mit).
Gelände je Kachel aus dem **DGM1 Bayern** (1 m, je Rasterpunkt das Mittel über 5 × 5 m), außerhalb Bayerns aus den
weltweiten **Terrain Tiles** (AWS Open Data, ~30 m); die Startstraße bekommt Höhe und Steigung des Geländes, darunter
wird es auf Fahrbahnhöhe gebracht. Angelegt wird im Hintergrund (Fortschritt in der Kartenauswahl). **Erweitern:** im
World Editor angefügte Kacheln einer Karte mit Ort bekommen das echte Gelände ihrer Stelle (am Rand genau an die
Nachbarn angeglichen, auch wenn dort geformt wurde; Auslauf 40 m) und ihr Luftbild. Ohne gewählten Treffer nimmt „Erstellen“ den
ersten Treffer des eingetippten Orts. **Ort nachträglich festlegen** (Karten ohne Ort, z. B. ältere): World Editor →
Kacheln bearbeiten → „Ort der Karte“ (oder Knopf „Luftbild …“ oben): suchen, Treffer wählen, „Ort festlegen“ – wahlweise
mit echtem Gelände für die vorhandenen Kacheln (ein Rückgängig-Schritt; Straßen behalten ihre Höhe). Die Ortsdatei
kommt beim Speichern in die Karte.

**Luftbild** (Karten mit Ort, Leiste oben: „Luftbild“ + Deckkraft-Regler): je geladener Kachel ein Ausschnitt aus dem
**DOP40 Bayern** (40 cm, WMS in EPSG:25832, genau auf die Kachel), im Hintergrund geholt und als Ebene auf das
Geländenetz gelegt (openOMSI `Viewer::set_ground_image` / `set_ground_image_alpha`) – es folgt dem Gelände, auch beim
Formen; Straßen und Objekte stehen darüber. Außerhalb Bayerns gibt es (noch) kein Luftbild. Quellenangabe rechts in
der Statuszeile. Daten: `python -m omsigen.geodaten` (Zwischenspeicher `%LOCALAPPDATA%\omsi-editor\geodaten`).

**Rückgängig/Wiederholen** (Strg+Z / Strg+Y) gilt über alle Werkzeuge: der jeweils letzte Schritt, egal in welchem
Werkzeug man gerade ist (eine gebaute Straße samt Kreuzungen und aufgeschnittenen Straßen ist ein Schritt).

**Vorhandene Straßen ändern (Werkzeug „Ändern“, Taste U)** wie das Upgrade-Werkzeug in Transport Fever 2: die Straße
unter der Maus wird umrissen, Klick wählt den Spline (Umschalt+Klick: ganze verknüpfte Kette, Strg+Klick: dazu/weg).
Rechts: Daten des Splines, **Richtung umkehren** (`mirror`), **Löschen** (Entf) und **Upgrade** auf einen anderen
Querschnitt (Raster mit Vorschaubildern; Warnung, wenn die Fahrspuren nicht mehr zu den Nachbar-Splines passen).
**Fahrtrichtung der KI (Einbahn):** „beide“, „Einbahn → Pfeil“, „Einbahn ← gegen“ – Optik und Querschnitt bleiben, die
Fahrzeugpfade der Gegenrichtung bekommen `[rule] <Pfad> no_cars 0 0` (wie die Standardkarten; openOMSI/OMSI lassen die
KI dort weder einsetzen noch abbiegen, der Spieler darf). Pfeile am gewählten Spline zeigen die Splinerichtung; mit H
erscheinen gesperrte Spuren rot. Andere Regeln der Karte (Verkehrsdichte usw.) bleiben. Gilt auch für eigene Straßen
(beim Speichern geschrieben). **„⛔ für KI-Verkehr sperren“** sperrt alle Fahrspuren der gewählten Straße und
die Pfade der Kreuzungen, die (laut Spurnetz) in sie hineinführen – an vorhandenen Kreuzungsobjekten als `[rule]`
hinter dem `[object]`, an eigenen Kreuzungen und Kreisverkehren erzeugt omsigen die Regeln mit. „beide“ gibt alles
wieder frei.
Technik: die geänderte Kachel wird in einen Sitzungsordner geschrieben, den openOMSI vor der
Installation liest, und neu geladen – sofort sichtbar, die Originalkarte bleibt unverändert; beim Speichern als
neue Karte kommen diese Kacheln mit.

**Speichern (Strg+S)** überschreibt die geöffnete Karte: jede Datei, die dabei ersetzt wird (Kacheln, `global.cfg`),
kommt vorher nach `%LOCALAPPDATA%\omsi-editor\sicherungen\<Karte>\<Zeit>\` (die letzten 10 je Karte bleiben). Eigene
Karten (vom Editor angelegt – Markierung `omsi-editor.txt` – oder von omsigen erzeugt) werden ohne Rückfrage
gespeichert, bei Standard- und Fremdkarten fragt der Editor nach und bietet „Als neue Karte“ an. Danach wird die Karte
frisch geladen (Kamera bleibt). Kreuzungsobjekte kommen in `Sceneryobjects\Aschaffenburg\<Karte>\` (eindeutige
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
   platzieren, Rückgängig, Speichern als neue Karte. Splines verschieben: Werkzeug „Knoten“ (Schritt 3)
3. **Straßen** (in Arbeit): Netz-Kern (Knoten, Kanten aus Geraden/Bögen, glatte Höhe), Werkzeug Gerade/Kurve mit
   Live-Vorschau und Einrasten, Speichern als Splines, Anschluss an freie Enden vorhandener Straßen, Ändern/Upgrade,
   allgemeine Kreuzungslogik: T, Kreuzen (4+ Arme), Kreisverkehr, über eigene und vorhandene Straßen (fertig);
   Vorfahrt/Ampel per Klick (fertig); Knoten ziehen für vorhandene und eigene Straßen und Kreuzungen (fertig);
   Kreisverkehr-Optik (Platten am gebogenen Ring), Kreuzungen drehen, Ampelzeiten einstellen, Fußgängerampeln (offen)
4. omsigen-Funktionen: OSM-Import, Kreuzungsgenerator, DGM-Gelände, Luftbild
