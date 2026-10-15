# Musterbuch Straßen und Kreuzungen

Maßstab (Nutzer, 15.10.2026): **Ahlheim 5** und **Städtedreieck V3**. Dieses Dokument hält fest, wie diese Karten
Straßen und Kreuzungen bauen (gemessen, nicht geschätzt), und leitet daraus Regeln für das Straßenraummodell und
OMSIgen ab. Messwerkzeug: `python -m omsigen.vorbildstudie "Ahlheim 5" "Städtedreieck_V3" <eigene Karte> --md <Datei>`
(Splines mit Fahrzeugpfaden; Kreuzungen = Objekte mit `[splinehelper]`, bei omsigen-Karten die Kreuzungsknoten aus
`netz.json`). Ältere Studien: `docs/omsi-format.md` (Tröge, Mauern, Bordstein-Splines, Kreuzungsobjekte).

## 1. Messung (15.10.2026)

| | Ahlheim 5 | Städtedreieck V3 | Aschaffenburg_Linie16_v9 |
|---|---|---|---|
| Straßensplines | 9928 | 8907 | 3141 |
| Anteil Geraden | 0,58 | 0,48 | 0,67 |
| Bogenradius 25/50/75 % [m] | 30 / 80 / 200 | 20 / 50 / 169 | **6 / 15 / 15** |
| Elementlänge 25/50/75 % [m] | 10 / 30 / 50 | 10 / 20 / 47 | **2 / 9 / 16** |
| Krümmungssprung 25/50/75 % [1/km] | 0 / 5 / 25 | 0 / 10 / 39 | **0 / 31 / 67** |
| Elemente mit Ausrundung (Steigung Anfang ≠ Ende) | 22 % | 18 % | **0 %** |
| Kreuzungen | 474 | 346 | 226 |
| Arme je Kreuzung | 2: 129, 3: 261, 4: 81, 5: 2, 6: 1 | 2: 141, 3: 166, 4: 39 | 2: 45, 3: 124, 4: 30, **5–12: 27** |
| Kreuzungsplatte, Kante 25/50/75 % [m] | 20 / 22 / 31 | 20 / 24 / 31 | 15 / 18 / 19 |
| geneigte Kreuzungsobjekte (Neigung/Rollen) | 2 | 8 | 0 |
| **Arme mit Knick an der Platte ≥ 2 / 4 / 8 %** | 2 / 1 / 0 von 1375 | 1 / 0 / 0 von 916 | **354 / 223 / 89 von 750** |
| Hang-Arme (≥ 4 % innerhalb 60 m) | 133 | 92 | 240 |
| Ausrundung 0 % → Hangsteigung 25/50/75 % [m] | 27 / 45 / 56 | 18 / 27 / 45 | – (keine Ausrundung) |
| Spline-Knoten mit ≥ 3 Straßenenden | 211 (fast nur `invis_street`) | 447 (385 `invis_street`) | 76 |

Bilder (openOMSI, gleiche Kamera 32 m / 7 m hoch): `musterbuch/ahlheim_hang_einmuendung.jpg` – Einmündung am 8-%-Hang,
die Straße läuft ohne Kante durch; `musterbuch/v9_hang_kreuzung.jpg` – Zufahrt steigt mit 7 % an und stößt an eine
flache Platte.

## 2. Kreuzungen am Gefälle

So lösen es die Vorbilder (alle 2291 Arme an Kreuzungsobjekten ausgewertet):

1. **Die Platte ist eben, die Arme setzen mit 0 % an.** Der anschließende Spline beginnt an der Plattenkante mit
   Steigung 0 und ändert sie *innerhalb des Elements* stetig (Steigung Anfang ≠ Ende, in OMSI linear über die
   Elementlänge = Parabel-Ausrundung) bis zur Hangsteigung – über 18–56 m (Median 27 m Städtedreieck, 45 m Ahlheim).
   Es gibt weder eine flache Strecke vor der Platte (Median 0 m) noch Steigungssprünge zwischen den Elementen.
   Beispiel Städtedreieck `Einm_Altonaer` [8,0]: Arm 1 `0 → −13,8 %` auf 27 m, dann `−13,8 → −0,6 %` auf 24 m.
2. **Bei durchgehendem Gefälle wird die Platte gekippt** (`[object]` Neigung/Rollen in Grad). Die durchgehenden Arme
   schließen mit genau der Plattensteigung an, nur der einmündende Arm setzt flach an. Beispiel Ahlheim
   `Einm_Altonaer`: Rollen −5° (≈ 8,7 %), Arme +8 % / −8 % / 0 %. Städtedreieck: Rollen 1°–4°, Neigung −2,5°.
   Das bleibt die Ausnahme (10 von 820 Kreuzungen), weil die Kartenbauer die Kreuzungen meist an flachere Stellen
   legen bzw. das Gelände anpassen.
3. **Höhenversatz zwischen den Anschlüssen** (`[splinehelper]` mit verschiedener Höhe) gibt es nur für Gehweg-Splines
   (0,15 m), nie für Fahrbahnen.

So macht es v9: Die Arme kommen mit voller Hangsteigung an der Platte an, die Platte ist flach. Daraus ergibt sich
an 354 von 750 Armen ein Knick von mindestens 2 %, an 89 Armen von mindestens 8 %.

**Regeln**
- **K-H1 (Pflicht):** Steigung des Arms an der Plattenkante = Steigung der Platte in Armrichtung (Toleranz 0,5 %).
- **K-H2:** Übergang mit Ausrundung, also Steigung linear im Element. Mindestlänge `L ≥ |Δs| · 4 m/%`, mindestens 20 m;
  aus den Vorbildern ergibt sich etwa 3–6 m je Prozentpunkt. Kein Sprung > 1 %-Punkt zwischen Elementen.
- **K-H3:** Liegt die Hauptrichtung im Gefälle > 3 %, wird die Platte in diese Steigung gekippt (Neigung/Rollen). Die
  Hauptarme laufen dann ohne Ausrundung durch, die Nebenarme runden auf die Querneigung der Platte aus.
  Bei < 3 % bleibt die Platte eben (Regel K-H1/K-H2).
- **K-H4 (über die Vorbilder hinaus, zu prüfen):** Weil OMSIgen die Plattenmodelle selbst erzeugt, kann die Fläche
  auch verwunden sein (Höhe je Eckpunkt aus der Gradiente der Hauptstraße und der Querneigung). Damit bleibt die
  Wirklichkeit erhalten: Die Hauptstraße behält ihre Längsneigung, die Nebenstraße passt sich an. Voraussetzung ist,
  dass die Pfade in der `.sco` ihre Höhenänderung (`[path]` Feld 8) mitbekommen. Das muss in OMSI 2 selbst geprüft
  werden, bevor es Standard wird.

**Umsetzung im Generator (15.10.2026)**, schon vor dem Straßenraummodell:
- `hoehen.py`: Jede Kreuzungsplatte ist eine Ebene mit Höhe und Neigung nach Ost und Nord. Neigung kostet in der
  Ausgleichsrechnung etwas, die Querneigung der Hauptarme (höchste Straßenklasse) viel. Höchstneigung 10 %.
  Das erste Element jedes Arms muss die Plattenneigung in Armrichtung haben (K-H1).
- Die Straßen werden als `[spline_h]` geschrieben, wie OMSI und openOMSI es rechnen: kubisch, mit Steigung am
  Anfang und Ende und Höhenänderung. Die Steigung ist an den Elementgrenzen stetig (K-H2/T-4), an der Kreuzung
  ist sie genau die Plattenneigung.
- `kreuzung.ebene_anwenden`: Modell und Pfade der Platte liegen in dieser Ebene, die Pfade bekommen dazu ihre
  Höhenänderung (Feld 8). Querschnittsübergänge (S1) bekommen die Steigung längs ihrer Achse.
- Ausrundung: In den ersten 40 m eines Arms wird die Steigung stärker geglättet (an der Platte das Fünffache).
  Im T-Test mit einer 8-%-Nebenstraße erreicht die Steigung 90 % erst nach 38 m (Vorbilder: 27–45 m). Die Platte
  bleibt dabei mit 1,7 % Querneigung fast eben (Querstrafe 100). Nebenstraßen liegen dadurch vor der Einmündung bis
  etwa 1,5 m neben dem Rohgelände; die Böschung gleicht das an.
- Befund am T-Test: Ohne die Strafe auf die Querneigung kippte die Platte um 7,5 % quer zur ebenen Hauptstraße,
  damit die steile Nebenstraße flacher ansetzt. In der Wirklichkeit behält die Hauptstraße ihr Profil, deshalb die
  starke Strafe.
- `netz_datei.splines_lesen` las die Höhenänderung von `[spline_h]` an der falschen Stelle und rechnete die Höhe
  linear statt kubisch. Beides ist korrigiert.

## 3. Linienführung

- **T-1:** Bögen mit großen Radien. Median 50–80 m, nur ein Viertel unter 20–30 m (Abbiegebögen, Kreuzungsnähe).
  v9 hat Median 15 m und ein Drittel bei 6 m, weil es jede OSM-Ecke mit einem kleinen Festradius ausrundet. Dadurch
  wirken die Straßen eckig.
- **T-2:** Lange Elemente, 10–50 m, Median 20–30 m. v9 hat 2–16 m, weil die Höhe in höchstens 20 m langen
  Elementen mit fester Steigung geführt wird.
- **T-3:** Krümmung ändert sich in kleinen Schritten (Median 5–10 /km). v9 hat 31 /km.
- **T-4:** Die Höhe folgt einer Gradiente mit Ausrundungen (Steigung Anfang ≠ Ende), nicht einem Treppenzug aus
  Elementen mit fester Steigung.

**Umsetzung im Generator (15.10.2026), `network.fillet_chain`:**
- Die OSM-Linie wird zuerst vereinfacht (Douglas-Peucker 0,5 m), sodass zittrige Geraden wirklich gerade werden.
  Wechsel von Querschnitt und Ebene bleiben dabei als Punkte erhalten.
- Jede verbleibende Ecke bekommt den größten Bogen, der zwischen die Nachbarstücke passt und höchstens 1,5 m innen an
  der Ecke vorbeigeht (`BOGEN_TOL`), höchstens 1000 m. Der Klassenradius (`config.RADIUS`) gilt nur noch als Untergrenze
  an echten Ecken. Ein Bogen belegt höchstens 80 % des ersten bzw. letzten Stücks, damit die Arme gerade an die
  Kreuzung anschließen.
- Test: Eine OSM-Kurve mit R = 80 m (6-m-Stücke, ±0,3 m Zittern) ergab früher lauter 15-m-Bögen, jetzt Bögen über
  50 m mit höchstens 1,6 m Abstand zur OSM-Linie.

## 3b. Gelände unter der Straße

OMSI spannt das Gelände als Dreiecke zwischen Punkten im 5-m-Raster. Liegt ein Punkt neben dem Bordstein höher als
die Straße (Einschnitt, Böschung), hebt sein Dreieck die Bodentextur quer über Gehweg und Fahrbahn. In v11 war das an
438 Straßen- und Plattenelementen der Fall (≥ 10 cm). Seit dem 15.10.2026 darf jeder Rasterpunkt in Reichweite einer
Zellendiagonale (7,07 m) höchstens auf der Höhe des nächsten Straßenpunkts liegen (`hoehen.raster`); in v12 bleiben
29 Elemente, alle am Ring-Trog (Richtungsfahrbahnen auf verschiedener Höhe, gehört zum Sonderbau Ring).
Prüfung: `python -m omsigen.gelaende_pruefung <Karte>`, im Prüfbericht als „Gelände über der Straße“.

## 3c. Naht Kreuzungsplatte – Straße

Eine geneigte Platte fällt auch quer zu ihren Armen ab, das Spline-Ende war quer eben. An den Fahrbahnrändern entstand
eine Stufe bzw. ein Keil (v14: 140 Armenden ≥ 2 cm, bis 23 cm), im Spiel als helle Linie über die Fahrbahn und als
Versatz im Bordstein (`musterbuch/naht_vorher.jpg`). Seit dem 15.10.2026 übernimmt jeder Arm an der Plattenkante genau
die Querneigung der Platte (OMSI: Prozent, `dz = −x · cant / 100`, x rechts, innerhalb der halben Breite aus den
Höhenprofilen) und läuft über 15 m (bei kurzen Stücken über deren Länge) auf 0 aus (`hoehen.py`, `quer`). An
Querschnittsübergängen bleibt die Platte quer eben. Ergebnis v17: 5 Armenden ≥ 2 cm (`musterbuch/naht_nachher.jpg`).
Prüfung: Fund „Stufe an der Kreuzungskante“ (`gelaende_pruefung.naehte`).

Querschnittswechsel in Kurven: Übergangsobjekte gibt es nur gerade. Gleich aufgebaute Nachbarn (nur andere Breiten)
werden in einer Kurve > 6° deshalb angeglichen (`querschnitte.angleichen`). Die meisten Wechsel in Kurven sind aber
echte Aufbauwechsel (Bus-, Radstreifen, Parken, Mittelinsel beginnt oder endet), die OSM zufällig in die Kurve legt.
Dort bleibt vorerst eine kleine Platte; besser wäre es, den Wechselpunkt auf das nächste gerade Stück zu verschieben.

## 4. Kreuzungsaufbau

- **K-1:** 3- und 4-armig, 5 und mehr Arme fast nie (3 von 820). v9 hat 27 Knoten mit 5–12 Armen (Busbahnhof,
  Parkplatzzufahrten), die zu einer großen Platte verschmelzen. Solche Flächen sind Sonderbauten bzw. Zufahrten,
  keine Kreuzungsarme.
- **K-2:** Objekte mit 2 Anschlüssen gibt es nur für bestimmte Zwecke: Querschnittsübergang (`RQ_9,5_to_X8`), Busbucht
  (`…_busstop`), Brückenanschluss, Kreisverkehrsteil, Ein- und Ausfahrt. Eine Platte an einer bloßen Wegteilung (v9: 45)
  gibt es nicht.
- **K-3:** Plattenkante 20–31 m; die Arme schließen gerade an.
- **K-4:** Bordsteine, Mauern und Trogwände sind eigene Splines parallel zur Fahrbahn (siehe `omsi-format.md`).

## 5. Folgerungen für das Straßenraummodell (Vorstufe vor OMSIgen)

Das Modell muss je Straße eine **trassierte Achse** (Geraden, Bögen mit Radien nach T-1, Element ≥ 10 m) und eine
**Gradiente mit Ausrundungen** (T-4) führen, je Kreuzung **Typ, Plattenebene (eben oder gekippt, K-H3) und
Arme** und **Zweckbauten** (Übergang, Busbucht, Brückenanschluss) getrennt von Kreuzungen. OMSIgen übersetzt dann
nur noch. Die Kennzahlen aus Abschnitt 1 werden Prüfregeln: Knick an der Platte, Radius < 20 m außerhalb von
Kreuzungen, Krümmungssprung, Steigungssprung, Kreuzungen mit ≥ 5 Armen oder 2 Armen ohne Zweck.
