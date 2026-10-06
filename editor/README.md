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
target\release\omsi-editor.exe                                   # Kartenliste, erste Karte
target\release\omsi-editor.exe Grundorf                          # Karte direkt
target\release\omsi-editor.exe Grundorf --bild b.png --cam=x,y,gier,neigung,abstand   # Bild ohne Fenster
target\release\omsi-editor.exe Grundorf --testlauf 20           # Fenster, nach 20 s beenden, Bildrate ausgeben
```

Kamera: rechte Maustaste drehen/neigen, mittlere verschieben, Mausrad zoomen (zum Mauszeiger), W A S D / Pfeile,
Q / E drehen, R / F neigen. Koordinaten wie in openOMSI: x Ost, y Nord, z hoch.

**Objekte bearbeiten (Werkzeug „Objekte“, Taste O):** Klick wählt das Objekt unter der Maus (weißer Ring = unter der
Maus, magenta = gewählt), Ziehen schiebt es über das Gelände, Strg+Mausrad oder `,` `.` dreht (Umschalt: fein),
Bild ↑/↓ hebt/senkt, Entf löscht, „Kopieren“ legt eine Kopie daneben; Position und Richtung auch als Zahlen rechts.
Strg+Z / Strg+Y. **Strg+S: als neue Karte speichern** – der Kartenordner wird kopiert, die geänderten Kacheln
(UTF-16 bleibt) kommen darüber, `global.cfg` bekommt den neuen Namen; die Originalkarte bleibt unverändert.

Tests: `cargo test --release` (ohne OMSI), `cargo test --release -- --include-ignored` (mit OMSI und Grafikkarte;
`dxcompiler.dll`/`dxil.dll` auch nach `targetelease\deps`).

## Stand und Plan

1. **Betrachter** (fertig): beliebige Karte, Darstellung wie im Spiel, Kacheln werden im Hintergrund um den
   Blickpunkt gestreamt (Worker-Thread, 6 ms Hochladen je Bild), Kartenwechsel ohne neuen Renderer
2. **Bearbeiten** (Objekte fertig): auswählen, ziehen, drehen, heben, löschen, kopieren, Rückgängig, Speichern als
   neue Karte. Splines verschieben kommt mit dem Netz-Kern (Schritt 3) – das kann auch openOMSI selbst noch nicht
3. Straßenwerkzeug wie in Transport Fever 2 mit Netz-Kern (Knoten, Kanten mit Kurven, Querschnitte)
4. omsigen-Funktionen: OSM-Import, Kreuzungsgenerator, DGM-Gelände, Luftbild
