# Kartenstudie: wie OMSI-Karten gebaut sind

Automatisch erzeugt mit `python -m omsigen.studie` am 05.10.2026 aus 21 Karten. Kreuzung = Objekt mit mindestens 3 Armen (Einfahrten). Vorfahrt: 192 = Vorfahrt, 64 = wartepflichtig, 128 = keine Regel (rechts vor links). Nur lesend ausgewertet.

## Ueberblick

| Karte | Kacheln | Splines | Objekte | Kreuzungsobjekte | Arme 3/4/5+ | mit Vorfahrt | mit Ampel | Gehwegpfade | Verzweigungen aus Splines | Anschluss 50/90/99 % |
|---|---:|---:|---:|---:|---|---:|---:|---:|---:|---|
| Kaiserstadt Aachen | 112 | 2263 | 28032 | 60 | 22/13/25 | 43 | 30 | 60 | 6 | 0.0/0.001/0.063 m |
| Aschaffenbzrg | 1 | 0 | 0 | 0 | 0/0/0 | 0 | 0 | 0 | 0 |  m |
| Berlin-Spandau | 329 | 2486 | 29450 | 181 | 116/54/11 | 86 | 84 | 181 | 2 | 0.0/0.035/0.204 m |
| Express 91.06 | 298 | 27671 | 139061 | 8 | 6/1/1 | 1 | 8 | 0 | 223 | 0.0/0.001/0.058 m |
| Gladbeck | 1744 | 87377 | 242118 | 578 | 434/144/0 | 393 | 302 | 560 | 834 | 0.0/0.0/0.049 m |
| Grundorf (Tutorial) | 19 | 60 | 1095 | 10 | 8/2/0 | 8 | 10 | 10 | 0 | 0.0/0.0/0.005 m |
| Bremen-Nord Fiktiv So | 140 | 15560 | 44943 | 4 | 1/1/2 | 2 | 2 | 0 | 2 | 0.0/0.001/0.082 m |
| Bremen-Nord Modern 2026 Mo-Fr | 178 | 16957 | 55729 | 62 | 32/18/12 | 60 | 29 | 37 | 117 | 0.0/0.001/0.011 m |
| Bremen-Nord | 146 | 16725 | 49553 | 54 | 28/16/10 | 48 | 21 | 24 | 115 | 0.0/0.001/0.011 m |
| HafenCity - Hamburg Modern | 347 | 863 | 39637 | 142 | 49/61/32 | 124 | 116 | 137 | 4 | 0.0/0.001/0.083 m |
| Hamburg Tag & Nacht | 149 | 354 | 21576 | 73 | 21/35/17 | 65 | 60 | 72 | 4 | 0.0/0.002/0.112 m |
| Hamburg - Innovationslinie 109 | 149 | 324 | 21315 | 73 | 22/38/13 | 62 | 61 | 71 | 4 | 0.0/0.002/0.107 m |
| Hamburg Linie 20 | 403 | 964 | 52154 | 174 | 59/70/45 | 151 | 144 | 166 | 4 | 0.0/0.001/0.075 m |
| Köln | 119 | 4192 | 15435 | 36 | 12/9/15 | 35 | 16 | 26 | 2 | 0.0/0.001/0.003 m |
| Neuendorf | 57 | 769 | 12557 | 106 | 81/22/3 | 91 | 52 | 106 | 67 | 0.0/0.001/0.007 m |
| Rheinhausen | 233 | 4966 | 49587 | 234 | 154/71/9 | 175 | 86 | 211 | 8 | 0.0/0.001/0.032 m |
| Ruhrgebiet | 2156 | 128198 | 322241 | 836 | 650/186/0 | 551 | 438 | 818 | 1032 | 0.0/0.0/0.047 m |
| X10 Berlin | 356 | 3174 | 80801 | 430 | 290/108/32 | 248 | 128 | 430 | 5 | 0.0/0.002/0.018 m |
| Ahlheim 4 | 730 | 60679 | 278123 | 665 | 443/167/55 | 556 | 264 | 405 | 214 | 0.0/0.002/0.052 m |
| Ahlheim 5 | 776 | 70228 | 356176 | 708 | 470/180/58 | 588 | 269 | 407 | 220 | 0.0/0.002/0.051 m |
| Mainzer Linien 68 und 69 | 188 | 9713 | 28284 | 22 | 14/8/0 | 6 | 8 | 17 | 73 | 0.0/0.0/0.157 m |

## Vorfahrt: welche Bewegungen bekommen welche Prioritaet (alle Karten)

| Bewegung | 192 | 128 (keine) | 64 |
|---|---:|---:|---:|
| gerade | 8094 (50 %) | 5928 (37 %) | 2028 (13 %) |
| rechts | 4635 (33 %) | 5509 (39 %) | 3887 (28 %) |
| links | 3362 (25 %) | 4990 (37 %) | 5018 (38 %) |
| wenden | 523 (41 %) | 260 (20 %) | 495 (39 %) |

## Abbiegeradien in Kreuzungsobjekten (Median je Karte)

- rechts: 55.0 m, 21.0 m, 24.4 m, 5.0 m, 9.0 m, 14.5 m, 12.0 m, 13.9 m, 130.0 m, 170.0 m, 160.0 m, 132.1 m, 100.0 m, 12.6 m, 15.0 m, 5.9 m, 11.5 m, 11.4 m, 12.0 m, 20.0 m -> gewichteter Mittelwert 57.2 m
- links: 67.2 m, 30.0 m, 28.1 m, 9.0 m, 10.0 m, 11.9 m, 17.0 m, 18.0 m, 143.0 m, 137.7 m, 140.0 m, 140.0 m, 100.0 m, 10.2 m, 21.0 m, 9.1 m, 12.9 m, 14.2 m, 14.2 m, 20.0 m -> gewichteter Mittelwert 54.7 m

## Ampeln

- Kaiserstadt Aachen: 159 Signalgruppen, Umlauf Median 69 s, haeufig 60 s (34x), 90 s (18x), 80 s (11x), 70 s (11x), 65 s (7x)
- Berlin-Spandau: 276 Signalgruppen, Umlauf Median 51.0 s, haeufig 35 s (21x), 47 s (15x), 55 s (14x), 36 s (12x), 28 s (12x)
- Express 91.06: 28 Signalgruppen, Umlauf Median 64.0 s, haeufig 64 s (6x), 37 s (5x), 36 s (5x), 80 s (2x), 90 s (1x)
- Gladbeck: 94 Signalgruppen, Umlauf Median 78.0 s, haeufig 78 s (21x), 82 s (16x), 79 s (7x), 110 s (6x), 36 s (5x)
- Grundorf (Tutorial): 20 Signalgruppen, Umlauf Median 46.5 s, haeufig 69 s (3x), 61 s (3x), 46 s (2x), 38 s (2x), 51 s (2x)
- Bremen-Nord Fiktiv So: 13 Signalgruppen, Umlauf Median 49 s, haeufig 49 s (4x), 50 s (2x), 46 s (2x), 36 s (2x), 27 s (1x)
- Bremen-Nord Modern 2026 Mo-Fr: 219 Signalgruppen, Umlauf Median 52 s, haeufig 82 s (10x), 50 s (8x), 1 s (8x), 70 s (8x), 49 s (8x)
- Bremen-Nord: 129 Signalgruppen, Umlauf Median 50 s, haeufig 50 s (7x), 58 s (6x), 49 s (5x), 27 s (5x), 70 s (5x)
- HafenCity - Hamburg Modern: 1088 Signalgruppen, Umlauf Median 73.0 s, haeufig 100 s (41x), 90 s (37x), 74 s (32x), 80 s (28x), 76 s (28x)
- Hamburg Tag & Nacht: 541 Signalgruppen, Umlauf Median 75 s, haeufig 100 s (37x), 80 s (22x), 90 s (20x), 97 s (15x), 91 s (15x)
- Hamburg - Innovationslinie 109: 589 Signalgruppen, Umlauf Median 76 s, haeufig 100 s (42x), 90 s (19x), 80 s (19x), 71 s (17x), 91 s (16x)
- Hamburg Linie 20: 1427 Signalgruppen, Umlauf Median 72 s, haeufig 74 s (42x), 90 s (41x), 1 s (38x), 100 s (38x), 80 s (36x)
- Köln: 90 Signalgruppen, Umlauf Median 90.0 s, haeufig 90 s (45x), 94 s (5x), 71 s (5x), 89 s (5x), 88 s (4x)
- Neuendorf: 70 Signalgruppen, Umlauf Median 55.5 s, haeufig 35 s (4x), 41 s (4x), 61 s (4x), 83 s (3x), 72 s (3x)
- Rheinhausen: 313 Signalgruppen, Umlauf Median 60 s, haeufig 110 s (24x), 35 s (16x), 55 s (12x), 66 s (11x), 41 s (11x)
- Ruhrgebiet: 140 Signalgruppen, Umlauf Median 78.0 s, haeufig 78 s (24x), 82 s (24x), 103 s (10x), 36 s (8x), 28 s (8x)
- X10 Berlin: 899 Signalgruppen, Umlauf Median 60 s, haeufig 1 s (71x), 203 s (22x), 58 s (20x), 284 s (20x), 57 s (18x)
- Ahlheim 4: 1835 Signalgruppen, Umlauf Median 48 s, haeufig 1 s (66x), 29 s (53x), 65 s (48x), 30 s (48x), 48 s (47x)
- Ahlheim 5: 2071 Signalgruppen, Umlauf Median 48 s, haeufig 1 s (80x), 29 s (57x), 65 s (56x), 30 s (56x), 41 s (51x)
- Mainzer Linien 68 und 69: 29 Signalgruppen, Umlauf Median 65 s, haeufig 70 s (6x), 60 s (6x), 78 s (3x), 45 s (2x), 65 s (2x)

## Haeufigste Strassen-Splines je Karte

- **Kaiserstadt Aachen**: `6_Meter_2R.sli` 116x (2 Spuren, 2 Gehwege, halbe Breite 5.5 m); `Kornelimuensterweg.sli` 33x (2 Spuren, 2 Gehwege, halbe Breite 7.0 m); `65_Meter_2R_NL.sli` 27x (2 Spuren, 2 Gehwege, halbe Breite 5.75 m); `Eilfschornsteinstr.sli` 25x (2 Spuren, 2 Gehwege, halbe Breite 5.5 m)
- **Berlin-Spandau**: `str_2spur_6m_erzgebirgs.sli` 134x (2 Spuren, 2 Gehwege, halbe Breite 6.0 m); `str_6spur_falkenseer1.sli` 35x (6 Spuren, 2 Gehwege, halbe Breite 20.0 m); `str_2spur_8m_sedan1.sli` 74x (2 Spuren, 2 Gehwege, halbe Breite 7.0 m); `Hstr_6spur_Ruhlebener1.sli` 32x (3 Spuren, 1 Gehwege, halbe Breite 15.1 m)
- **Express 91.06**: `invis_street.sli` 3014x (1 Spuren, 0 Gehwege, halbe Breite 0.0 m); `str_2spur_8m_chevreuse.sli` 511x (2 Spuren, 0 Gehwege, halbe Breite 4.0 m); `str_2spur_8m_TCSPMassy1.sli` 104x (2 Spuren, 0 Gehwege, halbe Breite 4.0 m); `BS_Sonstiges_InvisStreet_ER_2Spur.sli` 93x (2 Spuren, 0 Gehwege, halbe Breite 0.0 m)
- **Gladbeck**: `invis_street.sli` 11052x (1 Spuren, 0 Gehwege, halbe Breite 0.0 m); `str_2spur_8m_altonaer1.sli` 1012x (2 Spuren, 2 Gehwege, halbe Breite 7.0 m); `RQ_36.sli` 329x (6 Spuren, 0 Gehwege, halbe Breite 16.25 m); `RQ_10,5_2spur_7,5m_Oneway.sli` 591x (2 Spuren, 0 Gehwege, halbe Breite 4.15 m)
- **Grundorf (Tutorial)**: `str_2spur_11m_SeeburgerStr1.sli` 20x (2 Spuren, 2 Gehwege, halbe Breite 10.5 m); `str_2spur_8m_borkumer1.sli` 15x (2 Spuren, 2 Gehwege, halbe Breite 7.0 m); `str_2spur_8m_sedan1.sli` 5x (2 Spuren, 2 Gehwege, halbe Breite 7.0 m); `invis_street.sli` 4x (1 Spuren, 0 Gehwege, halbe Breite 0.0 m)
- **Bremen-Nord Fiktiv So**: `invis_street.sli` 98x (1 Spuren, 0 Gehwege, halbe Breite 0.0 m)
- **Bremen-Nord Modern 2026 Mo-Fr**: `invis_street.sli` 1389x (1 Spuren, 0 Gehwege, halbe Breite 0.0 m); `Inv.streets_1Spur_2Richtungen.sli` 86x (2 Spuren, 0 Gehwege, halbe Breite 0.0 m); `invis_street.sli` 93x (1 Spuren, 0 Gehwege, halbe Breite 0.0 m); `KA_Autobahn.sli` 17x (4 Spuren, 0 Gehwege, halbe Breite 8.5 m)
- **Bremen-Nord**: `invis_street.sli` 1234x (1 Spuren, 0 Gehwege, halbe Breite 0.0 m); `Inv.streets_1Spur_2Richtungen.sli` 112x (2 Spuren, 0 Gehwege, halbe Breite 0.0 m); `KA_Autobahn.sli` 19x (4 Spuren, 0 Gehwege, halbe Breite 8.5 m); `invis_street.sli` 75x (1 Spuren, 0 Gehwege, halbe Breite 0.0 m)
- **HafenCity - Hamburg Modern**: `invis_street.sli` 343x (1 Spuren, 0 Gehwege, halbe Breite 0.0 m); `2_SGW_moenckeberg1.sli` 6x (4 Spuren, 2 Gehwege, halbe Breite 10.66 m); `3_invis_bike.sli` 17x (1 Spuren, 0 Gehwege, halbe Breite 0.0 m); `SGW_ottowels.sli` 8x (2 Spuren, 2 Gehwege, halbe Breite 7.15 m)
- **Hamburg Tag & Nacht**: `invis_street.sli` 35x (1 Spuren, 0 Gehwege, halbe Breite 0.0 m); `SGW_moenckeberg1.sli` 6x (2 Spuren, 2 Gehwege, halbe Breite 10.66 m); `SGW_grossetheaterstr.sli` 5x (2 Spuren, 2 Gehwege, halbe Breite 7.15 m); `SGW_holzdamm_1.sli` 4x (2 Spuren, 2 Gehwege, halbe Breite 7.15 m)
- **Hamburg - Innovationslinie 109**: `invis_street.sli` 26x (1 Spuren, 0 Gehwege, halbe Breite 0.0 m); `2_SGW_moenckeberg1.sli` 6x (2 Spuren, 2 Gehwege, halbe Breite 10.66 m); `SGW_grossetheaterstr.sli` 5x (2 Spuren, 2 Gehwege, halbe Breite 7.15 m); `2_SGW_holzdamm_1.sli` 4x (2 Spuren, 2 Gehwege, halbe Breite 7.15 m)
- **Hamburg Linie 20**: `invis_street.sli` 417x (1 Spuren, 0 Gehwege, halbe Breite 0.0 m); `2_SGW_moenckeberg1.sli` 6x (4 Spuren, 2 Gehwege, halbe Breite 10.66 m); `3_invis_bike.sli` 23x (1 Spuren, 0 Gehwege, halbe Breite 0.0 m); `SGW_ottowels.sli` 8x (2 Spuren, 2 Gehwege, halbe Breite 7.15 m)
- **Köln**: `Colide_K.sli` 206x (1 Spuren, 0 Gehwege, halbe Breite 1.25 m); `Engeldorfer_str.sli` 52x (2 Spuren, 2 Gehwege, halbe Breite 6.16 m); `Boedigerstrasse.sli` 35x (2 Spuren, 0 Gehwege, halbe Breite 4.0 m); `2x250cm_Linienlos.sli` 34x (2 Spuren, 0 Gehwege, halbe Breite 2.75 m)
- **Neuendorf**: `str_2spur_8m_altonaer1.sli` 194x (2 Spuren, 2 Gehwege, halbe Breite 7.0 m); `str_2spur_11m_SeeburgerStr1.sli` 110x (2 Spuren, 2 Gehwege, halbe Breite 10.5 m); `Hstr_6spur_Wilhelm3.sli` 70x (3 Spuren, 1 Gehwege, halbe Breite 13.9 m); `str_2spur_8m_sedan1.sli` 33x (2 Spuren, 2 Gehwege, halbe Breite 7.0 m)
- **Rheinhausen**: `str_6spur_falkenseer1.sli` 71x (6 Spuren, 2 Gehwege, halbe Breite 20.0 m); `str_2spur_8m_altonaer1.sli` 173x (2 Spuren, 2 Gehwege, halbe Breite 7.0 m); `Hstr_6spur_Wilhelm3.sli` 96x (3 Spuren, 1 Gehwege, halbe Breite 13.9 m); `str_2spur_11m_SeeburgerStr1.sli` 104x (2 Spuren, 2 Gehwege, halbe Breite 10.5 m)
- **Ruhrgebiet**: `invis_street.sli` 13850x (1 Spuren, 0 Gehwege, halbe Breite 0.0 m); `RQ_36.sli` 358x (6 Spuren, 0 Gehwege, halbe Breite 16.25 m); `str_2spur_8m_altonaer1.sli` 915x (2 Spuren, 2 Gehwege, halbe Breite 7.0 m); `RQ_10,5_2spur_7,5m_Oneway.sli` 851x (2 Spuren, 0 Gehwege, halbe Breite 4.15 m)
- **X10 Berlin**: `str_2spur_8m_altonaer1.sli` 334x (2 Spuren, 2 Gehwege, halbe Breite 7.0 m); `X5.5_2spurC_Beethoven.sli` 221x (2 Spuren, 2 Gehwege, halbe Breite 4.75 m); `str_2spur_6m_Leubnitzer.sli` 122x (2 Spuren, 2 Gehwege, halbe Breite 6.0 m); `str_2spur_11m_SeeburgerStr1.sli` 94x (2 Spuren, 2 Gehwege, halbe Breite 10.5 m)
- **Ahlheim 4**: `invis_street.sli` 3398x (1 Spuren, 0 Gehwege, halbe Breite 0.0 m); `str_2spur_8m_altonaer1.sli` 945x (2 Spuren, 2 Gehwege, halbe Breite 7.0 m); `str_2spur_6m_Leubnitzer.sli` 369x (2 Spuren, 2 Gehwege, halbe Breite 6.0 m); `RQ_X_2spur_Altonaer_8m.sli` 276x (2 Spuren, 2 Gehwege, halbe Breite 7.0 m)
- **Ahlheim 5**: `invis_street.sli` 3495x (1 Spuren, 0 Gehwege, halbe Breite 0.0 m); `str_2spur_8m_altonaer1_AL.sli` 645x (2 Spuren, 2 Gehwege, halbe Breite 7.0 m); `str_2spur_6m_Leubnitzer_AL.sli` 257x (2 Spuren, 2 Gehwege, halbe Breite 6.0 m); `RQ_X_2spur_Altonaer_8m_AL.sli` 232x (2 Spuren, 2 Gehwege, halbe Breite 7.0 m)
- **Mainzer Linien 68 und 69**: `invis_street.sli` 1214x (1 Spuren, 0 Gehwege, halbe Breite 0.0 m); `RQ_10,5_2spur_7,5m_Oneway.sli` 153x (2 Spuren, 0 Gehwege, halbe Breite 4.15 m); `RQ_X_2spur_Altonaer_8m.sli` 145x (2 Spuren, 2 Gehwege, halbe Breite 7.0 m); `str_2spur_8m_altonaer1.sli` 125x (2 Spuren, 2 Gehwege, halbe Breite 7.0 m)

## Beispielkreuzungen

- Kaiserstadt Aachen: `Sceneryobjects\Aachen_X\X_Audimax.sco` -> `build/studie\Aachen_Linie_33.png`
- Berlin-Spandau: `Sceneryobjects\Kreuz_MC\Kreuz_Altst_Klost_Seeg_Stabholz_2.sco` -> `build/studie\Berlin-Spandau.png`
- Express 91.06: `Sceneryobjects\Express 91.06\Cubes AI\CEA-Algo TCSP.sco` -> `build/studie\Express_91.06.png`
- Gladbeck: `Sceneryobjects\ADDON_Gladbeck_Kreuz\RQ_X8m_2spur_8,0m_Sidewalk_cross.sco` -> `build/studie\Gladbeck.png`
- Grundorf (Tutorial): `Sceneryobjects\Kreuz_MC\Kreuz_See_Elsflether.sco` -> `build/studie\Grundorf.png`
- Bremen-Nord Fiktiv So: `Sceneryobjects\HB_76_Objekte\Kreuzungen\Kreuzung_Splinesystem\Ampelobjekt_54.sco` -> `build/studie\HB_2017_Bremen-Nord_FIKTIV.png`
- Bremen-Nord Modern 2026 Mo-Fr: `Sceneryobjects\HB_76_crossings_splines\Kreuz_Landratstr_Luessumer2018.sco` -> `build/studie\HB_2026_Bremen-Nord.png`
- Bremen-Nord: `Sceneryobjects\HB_76_crossings_splines\Kreuz_Landratstr_Luessumer2009.sco` -> `build/studie\HB_76_Bremen-Nord.png`
- HafenCity - Hamburg Modern: `Sceneryobjects\HafenCityHamburg\3_K_borgweg.sco` -> `build/studie\HafenCityHamburg.png`
- Hamburg Tag & Nacht: `Sceneryobjects\HamburgLinie109\K_ualsterdorf.sco` -> `build/studie\Hamburg109.png`
- Hamburg - Innovationslinie 109: `Sceneryobjects\HamburgLinie109\2_K_altona.sco` -> `build/studie\Hamburg109_2.png`
- Hamburg Linie 20: `Sceneryobjects\HamburgLi20\3_K_borgweg.sco` -> `build/studie\HamburgLi20.png`
- Köln: `Sceneryobjects\PAD-Labs\Streets\Kr_M_B51_In der Hell.sco` -> `build/studie\Koeln.png`
- Neuendorf: `Sceneryobjects\Kreuz_MC\Kreuz_Gatower_Omni_Graetschel.sco` -> `build/studie\Neuendorf.png`
- Rheinhausen: `Sceneryobjects\ADDON_Rheinhausen\Zane_Crossings\Kreuz_Billinghauser_Rheinstrasse.sco` -> `build/studie\Rheinhausen.png`
- Ruhrgebiet: `Sceneryobjects\ADDON_Ruhr_Kreuz2\RQ_X8m_2spur_8,0m_Sidewalk_cross.sco` -> `build/studie\Ruhrgebiet.png`
- X10 Berlin: `Sceneryobjects\X10 Kreuzungen\KuJoach2.sco` -> `build/studie\X10_Berlin.png`
- Ahlheim 4: `Sceneryobjects\Ahlheim4_Objekte2\EVAG4101\Kreuzungen\Listau_S.sco` -> `build/studie\Ahlheim_4.png`
- Ahlheim 5: `Sceneryobjects\Ahlheim4_Objekte\Kreuzungen\Luehrer_Kreuzung_V5.sco` -> `build/studie\Ahlheim_5.png`
- Mainzer Linien 68 und 69: `Sceneryobjects\Mainz\XBleiche.sco` -> `build/studie\Mainz.png`
