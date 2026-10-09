# Sonderbau: Dr.-Willi-Reiland-Ring / Ludwigsallee (Einzelstück)

Kartenkoordinaten (Aburg_Newgen, UTM 32, ost0 510466, nord0 5536235). Daten: OSM (`osm_map.xml`, ODbL),
DOP40 und DGM1 Bayern (CC BY 4.0).

## Lesart (siehe `lesart.png`)

- **Ring** (trunk, 2 Spuren, Gegenverkehr) im Trog auf ~134,8 m NN, Fahrbahn ~8 m (x ≈ 1078–1089), unter der
  Ludwigsallee durch eine **Unterführung** (y ≈ −253,5 … −279, Decke mit gerundeten Stirnseiten).
- **Rampe West** (Einbahn nach Süden, 1 Spur + Sperrfläche): von der Ring-Nordseite (y ≈ −144, 134,5 m) hinauf zur
  Kreuzung (140,7 m), auf der Südseite wieder hinab bis y ≈ −400 (Auf-/Abfahrt Richtung Frankfurt).
  Rampe Ost entsprechend (Einbahn nach Norden: von Süden hinauf, nach Norden hinab, Richtung Würzburg).
- Getrennt von der Ring-Fahrbahn durch niedrige Trennwände (~1–1,5 m breit), außen Trogwände:
  West Gabionen + grüne Lärmschutzwand, Ost zweistufige Klinkermauer mit Pflanzstreifen + Glas-Lärmschutzwand;
  in der Unterführung Klinkerfelder mit Betonpfeilern.
- **Kreuzung Ludwigsallee** (ebenerdig, ~140,8 m, y ≈ −257 … −276): eine Ampelkreuzung mit zwei Knoten (West bei
  x 1075,6: Rampe West rein/raus; Ost bei x 1094,4: Rampe Ost rein/raus), Spuren laut OSM:
  West-Zufahrt (Schießhausbrücke) links | geradeaus+rechts, Busspur Gegenrichtung; Mitte 2+2 mit Linksabbiegern;
  Ost-Zufahrt 2 Spuren links | rechts; rote Radfahrstreifen, Verkehrsinsel mit Sperrfläche Ost.
- **Bahnbrücke** der Ludwigsallee über die Gleise westlich (x ≈ 1050–1062).

## Entwurf (statt roher Messwerte)

Etappe 1 aus Rohmessungen war krumm (OSM-Achse mit Knicken, Kanten springen im 1-m-DGM). Deshalb erst ein Entwurf wie
im Straßenbau (`omsigen/sonderbau_entwurf.py`, `python -m omsigen.ring_entwurf … --aus sonderbau/ring/entwurf`):
- Achse aus Geraden und Bögen durch die gemessene Mitte der Ring-Fahrbahn (bis kurz vor das nächste Bauwerk bei y ≈ −430): Bogen R 513 m (rechts) 229 m, Gerade 66 m;
  mittlere Abweichung 0,15 m.
- Kanten in festem Abstand zur Achse (Ringwände ±4,55–4,70 m, Rampen außen 12,45–13,40 m), Übergänge S-förmig.
- Gradienten mit Ausrundung: Ring +1,3 % / −0,7 % / +0,8 %; Rampen +7,6 % / +4,3 % → Kuppe → −3,5 % / −7,0 %.
- Prüfbilder: `entwurf/grundriss*.png` (über dem Luftbild), `entwurf/laengsschnitt.png`; Werte in `entwurf/entwurf.json`.

## Etappen

1. **Bauwerk-Objekt:** Trog (Wände, Trennwände, Lärmschutz), Unterführung (Decke mit Rundung, Klinker innen) –
   aus den gemessenen Linien und DGM1-Höhen, als Objekte; Prüfbild aus openOMSI gegen die Fotos.
   **Stand:** `python -m omsigen.sonderbau_ring --omsi … --karte Aburg_Newgen --osm sonderbau/ring/osm_map.xml`
   erzeugt aus dem Entwurf (`entwurf/entwurf.json`) 38 Objekte in `Sceneryobjects/Aschaffenburg/Aburg_Newgen/Sonderbau_Ring/` (Trennwände, Außenwände,
   Sohle/Rampenplatten, Deckel, Unterführung, Bahnbrücke) und `sonderbau_ring.json` (Lage, abzusenkende
   Geländepunkte, Messung). Prüfbilder: `cargo test --release ring_bauwerk -- --ignored` mit `OMSI_BILD`
   (Sitzungsordner, die Karte bleibt unverändert) → `etappe1/*.png`.
2. **Straßen:** Ring, Rampen, Ludwigsallee als Splines mit passenden Querschnitten (Baukasten) und Höhen aus dem
   DGM1, genau entlang OSM.
   **Stand:** `omsigen/ring_strassen.py` (läuft in `sonderbau_ring` mit): Ring auf der Achse (16 Elemente, 2 Spuren
   Gegenverkehr), Rampen als konzentrische Parallelen (West +9,38 m, Ost −9,73 m, je 1 Spur, im Kreuzungsbereich
   s 117–145 unterbrochen), Schießhausbrücke und Ludwigsallee-Ost als Geraden entlang OSM bis an den
   Kreuzungsbereich. Querschnitte `AB_SB_*` (Baukasten), Höhen aus den Gradienten bzw. DGM1. Elemente lückenlos nach
   OMSI-Bogenregel (Test). Offen für Etappe 3: Kreuzung und die Ein-/Ausfädelungen an beiden Enden.
3. **Kreuzung:** ein Kreuzungsobjekt mit allen Spuren laut OSM (Abbiegespuren, Busspur), Ampel mit Phasen, rote
   Radstreifen, Sperrflächen.
   **Stand:** ein Objekt `Kreuzung_Ludwigsallee` (omsigen editorkreuzung) mit 6 Armen (Schießhausbrücke,
   Ludwigsallee Ost, 4 Rampenenden), 9 Abbiegespuren nach OSM `turn:lanes` (`ring_strassen.VERBINDUNGEN`), Ampel mit
   3 Phasen (Ludwigsallee / Rampe West / Rampe Ost, 85 s), Haltlinien, rote Radfurten, Sperrfläche der Insel Ost im
   Objekt selbst. Die Platte liegt in der Ausgleichsebene durch die Armenden (Ost +1,44 %, Nord −0,77 %) – als
   Neigung/Rollen des Objekts, die Straßenenden sind auf die Ebene angeglichen (±1 cm). Bahnbrücke genau unter der
   Schießhausbrücke. Ein-/Ausfädeln: laut OSM erst außerhalb des Sonderbaus (s ≈ 0 und ≈ 299) → beim Einbau.
4. **Einbau:** als Einzelstück in die Karte (ein Rückgängig-Schritt, Speichern wie gewohnt).
