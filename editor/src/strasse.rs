//! Werkzeug "Strasse bauen" (wie in Transport Fever 2) auf dem Netz-Kern (netz.rs).
//!
//! Klick setzt den Start (rastet an Knoten ein; an einem freien Strassenende geht es tangential weiter), die Maus
//! zieht die Vorschau als echte OMSI-Strasse, Klick setzt den naechsten Knoten, Rechtsklick/Esc beendet den Zug.
//! Gerade: geradeaus (am Strassenende in dessen Richtung). Kurve: Bogen, der tangential anschliesst; trifft der
//! Bogen ein freies Strassenende, wird er tangential eingefaedelt (Bogenpaar). Bild auf/ab: Hoehe des naechsten
//! Punkts ueber dem Gelaende. Start oder Ziel mitten auf einer vorhandenen Strasse: dort entsteht beim Bauen eine
//! Kreuzung (kreuzung.rs), die neue Strasse beginnt bzw. endet an ihrem dritten Arm.

use crate::aendern::Aendern;
use crate::anschluss::{spuren_passen, Anschluesse, Anschluss};
use crate::kreuzung::{self, Abzweig};
use crate::netz::{self, bogen_durch, norm180, richtung, verbinden, Netz};
use glam::{DVec2, DVec3};
use openomsi_game::viewer::{TileGpu, Viewer};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const MIN_RADIUS: f64 = 10.0; // engere Boegen: Warnung
pub const MAX_STEIGUNG: f64 = 12.0; // Prozent
/// so nah an einem freien Strassenende rastet ein Klick auf die Strasse am Ende ein (statt Kreuzung)
pub const ENDE_FANG: f64 = 25.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Modus {
    Gerade,
    Kurve,
}

/// Querschnitt: eine .sli mit Fahrspuren
#[derive(Clone, Debug)]
pub struct Querschnitt {
    pub rel: String,
    pub name: String,
    pub ordner: String,
    /// Fahrspuren (je Richtung) und Gehwege
    pub vor: usize,
    pub zurueck: usize,
    pub gehwege: usize,
    pub breite: f32,
    /// wie im Objektkatalog: "OMSI (Standard)", Stadt/Karte, ...
    pub herkunft: String,
}

impl Querschnitt {
    /// Spurfilter: "1" Einbahn 1 Spur, "1+1", "2+2", "Einbahn 2+", "andere"
    pub fn spurklasse(&self) -> &'static str {
        match (self.vor, self.zurueck) {
            (1, 0) | (0, 1) => "Einbahn 1 Spur",
            (_, 0) | (0, _) => "Einbahn 2+ Spuren",
            (1, 1) => "1+1 Spuren",
            (2, 2) => "2+2 Spuren",
            _ => "andere",
        }
    }
}

pub const SPURKLASSEN: [&str; 5] = ["1+1 Spuren", "2+2 Spuren", "Einbahn 1 Spur", "Einbahn 2+ Spuren", "andere"];

/// alle .sli unter Splines mit mindestens einer Fahrspur (Hintergrund)
pub fn querschnitte(root: &Path) -> Vec<Querschnitt> {
    let genutzt = crate::katalog::nutzung_splines(root);
    let basis = root.join("Splines");
    let mut out = Vec::new();
    let mut stapel = vec![basis.clone()];
    while let Some(d) = stapel.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                stapel.push(p);
                continue;
            }
            if !p.extension().is_some_and(|x| x.eq_ignore_ascii_case("sli")) {
                continue;
            }
            let Ok(b) = std::fs::read(&p) else { continue };
            let text: String = if b.starts_with(&[0xFF, 0xFE]) {
                String::from_utf16_lossy(&b[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>())
            } else {
                b.iter().map(|&c| c as char).collect()
            };
            let z: Vec<&str> = text.lines().map(|l| l.trim()).collect();
            let (mut vor, mut zurueck, mut gehwege) = (0, 0, 0);
            let (mut lo, mut hi) = (0.0f32, 0.0f32);
            for (i, l) in z.iter().enumerate() {
                if l.eq_ignore_ascii_case("[path]") && i + 5 < z.len() {
                    let art = z[i + 1].parse::<i32>().unwrap_or(-1);
                    let richtung = z[i + 5].parse::<i32>().unwrap_or(0);
                    match (art, richtung) {
                        (0, 0) => vor += 1,
                        (0, 1) => zurueck += 1,
                        (0, _) => {
                            vor += 1;
                            zurueck += 1;
                        }
                        (1, _) => gehwege += 1,
                        _ => {}
                    }
                }
                if l.eq_ignore_ascii_case("[profilepnt]") && i + 1 < z.len() {
                    if let Ok(x) = z[i + 1].replace(',', ".").parse::<f32>() {
                        lo = lo.min(x);
                        hi = hi.max(x);
                    }
                }
            }
            if vor + zurueck == 0 {
                continue;
            }
            let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().replace('/', "\\");
            let ordner = p.strip_prefix(&basis).ok().and_then(|r| r.components().next()).map(|c| c.as_os_str().to_string_lossy().to_string()).unwrap_or_default();
            let name = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let herkunft = crate::katalog::herkunft(&ordner, &genutzt);
            out.push(Querschnitt { rel, name, ordner, vor, zurueck, gehwege, breite: hi - lo, herkunft });
        }
    }
    out.sort_by_key(|q| (q.ordner.to_lowercase(), q.name.to_lowercase()));
    out
}

/// wo ein Klick landet
enum Ziel {
    Knoten(u32, DVec3),
    Anschluss(Anschluss),
    /// mitten auf einer vorhandenen Strasse: dort entsteht eine Kreuzung
    Abzweig(Abzweig),
    /// mitten auf einer eigenen Strasse: sie wird dort geteilt, der Knoten wird eine Kreuzung
    Kante(Kantenpunkt),
    Frei(DVec3),
}

/// Stelle mitten auf einer eigenen Strasse (Kante des Netzes)
#[derive(Clone, Debug)]
struct Kantenpunkt {
    kante: u32,
    /// Meter ab dem Anfangsknoten (ohne Kuerzung)
    s: f64,
    pos: DVec3,
    richtung: f64,
}

/// Kreuzungsobjekt an einem Knoten des eigenen Netzes
struct NetzObjekt {
    signatur: String,
    objekt: kreuzung::Objekt,
    hoehe: f64,
    gpu: Option<TileGpu>,
}

/// Beginn des Zugs
#[derive(Clone, Debug)]
struct Start {
    knoten: Option<u32>,
    /// freies Ende einer vorhandenen Strasse (der Knoten entsteht erst mit der ersten Kante)
    anschluss: Option<Anschluss>,
    /// Stelle mitten auf einer vorhandenen Strasse (die Kreuzung entsteht erst mit der ersten Kante)
    abzweig: Option<Abzweig>,
    /// Stelle mitten auf einer eigenen Strasse (geteilt wird erst mit der ersten Kante)
    kante: Option<Kantenpunkt>,
    pos: DVec3,
    /// Richtung, in der es weitergehen muss (freies Strassenende, letzte Kante des Zugs)
    richtung: Option<f64>,
}

/// berechnete Vorschau einer Kante
#[derive(Clone, Debug)]
pub struct Plan {
    #[allow(dead_code)]
    pub a: DVec3,
    pub b: DVec3,
    pub ha: f64,
    pub hb: f64,
    /// Zielknoten (eingerastet) oder neu
    pub ziel: Option<u32>,
    /// Ziel ist das freie Ende einer vorhandenen Strasse
    pub ziel_anschluss: Option<Anschluss>,
    /// Ziel ist eine Stelle mitten auf einer vorhandenen Strasse (Kreuzung)
    pub ziel_abzweig: Option<Abzweig>,
    /// geplante Kreuzungen (Start, Ziel): Mitte, Richtung der Strasse, Schnitt je Seite, Ende des neuen Arms
    pub kreuzungen: Vec<KreuzungsVorschau>,
    /// eigene Strasse, die am Start geteilt wird: (Kante, Meter ab ihrem Anfang)
    kante_a: Option<(u32, f64)>,
    /// eigene Strasse, die am Ziel geteilt wird: Punkt darauf
    kante_b: Option<DVec3>,
    /// eine geplante Kreuzung geht nicht (dann wird nicht gebaut)
    pub blockiert: Option<String>,
    /// z. B. Spuren passen nicht zur vorhandenen Strasse
    pub warnung: Option<String>,
    pub laenge: f64,
    pub min_radius: f64,
    pub steigung: f64,
}

/// eine geplante Kreuzung fuer die Markierung im Bild
#[derive(Clone, Debug)]
pub struct KreuzungsVorschau {
    pub mitte: DVec3,
    pub strasse: f64,
    pub schnitt: f64,
    pub arm: DVec3,
}

pub struct Strassenbau {
    pub netz: Netz,
    pub sli: Option<String>,
    pub modus: Modus,
    pub hoehe: f64,
    start: Option<Start>,
    pub plan: Option<Plan>,
    vorschau: Vec<TileGpu>,
    gezeichnet: HashMap<u32, Vec<TileGpu>>,
    /// Netz vorher und wie viele Schritte des Aendern-Werkzeugs (Kreuzungen) dazugehoeren
    undo: Vec<(Netz, usize)>,
    redo: Vec<(Netz, usize)>,
    pub aenderungen: usize,
    /// Stelle mitten auf einer Strasse unter der Maus (vor dem Start: Markierung "hier zweigt es ab")
    pub zeiger: Option<KreuzungsVorschau>,
    /// Kreuzungsobjekte an den Knoten des eigenen Netzes (je Knoten), und schon erzeugte (je Signatur der Arme)
    objekte: HashMap<u32, NetzObjekt>,
    objekt_cache: HashMap<String, kreuzung::Objekt>,
    /// Ordner der Kreuzungsobjekte (vom Aendern-Werkzeug: Sitzungsordner) und sein Name
    kreuzungs_ordner: Option<(PathBuf, String)>,
    /// letzter Fehler beim Erzeugen eines Kreuzungsobjekts
    pub kreuzung_fehler: Option<String>,
    /// beim Anschluss an eine vorhandene Strasse deren Querschnitt uebernehmen, wenn der gewaehlte nicht passt
    pub uebernehmen: bool,
    spuren_cache: HashMap<String, Vec<(f32, u8)>>,
}

impl Default for Strassenbau {
    fn default() -> Self {
        Strassenbau { netz: Netz::default(), sli: None, modus: Modus::Kurve, hoehe: 0.0, start: None, plan: None,
                      vorschau: vec![], gezeichnet: HashMap::new(), undo: vec![], redo: vec![], aenderungen: 0,
                      uebernehmen: true, spuren_cache: HashMap::new(), zeiger: None, objekte: HashMap::new(),
                      objekt_cache: HashMap::new(), kreuzungs_ordner: None, kreuzung_fehler: None }
    }
}

impl Strassenbau {
    /// frisch (nach Kartenwechsel), Querschnitt und Modus bleiben
    pub fn neu(sli: Option<String>, modus: Modus) -> Self {
        Strassenbau { sli, modus, ..Default::default() }
    }

    pub fn baut(&self) -> bool {
        self.start.is_some()
    }

    /// Punkt fuer Start/Ziel: eigener Knoten, sonst freies Ende einer vorhandenen Strasse, sonst eine Stelle mitten
    /// auf einer vorhandenen Strasse (Kreuzung), sonst der Bodenpunkt
    fn punkt(&self, v: &Viewer, boden: DVec3, fang: f64, ans: &Anschluesse, ae: Option<&mut Aendern>) -> Ziel {
        if let Some(k) = self.netz.knoten_bei(boden.truncate(), fang) {
            return Ziel::Knoten(k, self.netz.knoten(k).unwrap().pos);
        }
        if let Some(i) = ans.bei(boden.truncate(), fang) {
            return Ziel::Anschluss(ans.liste[i].clone());
        }
        if let Some((kante, s, pos, richtung)) = self.netz.kante_bei(boden.truncate()) {
            return Ziel::Kante(Kantenpunkt { kante, s, pos, richtung });
        }
        if let Some(a) = ae {
            if let Some(ab) = a.abzweig_bei(v, boden.truncate()) {
                // nahe einem freien Ende derselben Strasse: dort anschliessen statt eine Kreuzung zu bauen
                let kette = a.kette_ids(ab.spline_id, 40.0);
                let ende = ans.liste.iter()
                    .filter(|x| x.frei && kette.contains(&x.spline_id) && (x.pos.truncate() - boden.truncate()).length() < ENDE_FANG)
                    .min_by(|x, y| (x.pos - boden).length().total_cmp(&(y.pos - boden).length()));
                if let Some(x) = ende {
                    return Ziel::Anschluss(x.clone());
                }
                return Ziel::Abzweig(ab);
            }
        }
        Ziel::Frei(boden + DVec3::Z * self.hoehe)
    }

    /// halbe Breite eines Querschnitts (aussen, groessere Seite)
    fn halb(&self, v: &Viewer, sli: &str) -> f64 {
        v.spline_lanes(sli).map(|(_, (l, r))| l.max(r) as f64).unwrap_or(5.0)
    }

    /// geplante Kreuzung an `ab` fuer einen Arm Richtung `wunsch` (eingepasst, wenn ein Ende zu nah ist):
    /// (Arm-Ende, Arm-Richtung von der Kreuzung weg, Markierung) oder warum es dort nicht geht
    fn arm_plan(&self, v: &Viewer, ae: Option<&Aendern>, ab: &Abzweig, wunsch: f64, sli: &str) -> Result<(DVec3, f64, KreuzungsVorschau), String> {
        let ab = &match ae {
            Some(a) => a.abzweig_einpassen(ab, wunsch, self.halb(v, sli)).map_err(|e| e.to_string())?,
            None => ab.clone(),
        };
        let h = kreuzung::arm_richtung(ab.richtung, wunsch);
        let (d, d_arm) = kreuzung::masse(ab.richtung, h, ab.halb, self.halb(v, sli));
        let arm = (ab.pos.truncate() + netz::dir(h) * d_arm).extend(ab.pos.z);
        Ok((arm, h, KreuzungsVorschau { mitte: ab.pos, strasse: ab.richtung, schnitt: d, arm }))
    }

    /// geplante Kreuzung mitten auf einer eigenen Strasse fuer einen Arm Richtung `wunsch`: eingepasst zwischen die
    /// Enden der Kante (und deren Kreuzungen) -> (Mitte, Arm-Richtung, Abstand des Arm-Endes, Markierung, Meter ab
    /// dem Anfang der Kante) oder warum es dort nicht geht
    fn kante_plan(&self, v: &Viewer, kp: &Kantenpunkt, wunsch: f64, sli: &str) -> Result<(DVec3, f64, f64, KreuzungsVorschau, f64), String> {
        let e = self.netz.kante(kp.kante).ok_or("Strasse nicht mehr da")?;
        let st = self.netz.lage(e);
        let laenge: f64 = st.iter().map(|s| s.laenge).sum();
        let h0 = kreuzung::arm_richtung(kp.richtung, wunsch);
        let (d, _) = kreuzung::masse(kp.richtung, h0, self.halb(v, &e.sli), self.halb(v, sli));
        let lo = self.netz.kuerzung(e.id, e.a) + d + 1.0;
        let hi = laenge - self.netz.kuerzung(e.id, e.b) - d - 1.0;
        if lo > hi {
            return Err(format!("das Strassenstueck ist zu kurz fuer eine Kreuzung (braucht {:.0} m)", laenge - hi + lo));
        }
        let s = kp.s.clamp(lo, hi);
        let (q, hq) = netz::punkt_auf(&st, s).ok_or("Strasse ohne Lage")?;
        let mitte = q.extend(self.netz.hoehe_bei(e, s));
        let h = kreuzung::arm_richtung(hq, wunsch);
        let (d, d_arm) = kreuzung::masse(hq, h, self.halb(v, &e.sli), self.halb(v, sli));
        let arm = (q + netz::dir(h) * d_arm).extend(mitte.z);
        Ok((mitte, h, d_arm, KreuzungsVorschau { mitte, strasse: hq, schnitt: d, arm }, s))
    }

    /// Fahrspuren (Querlage, Richtung) einer .sli, zwischengespeichert
    fn spuren(&mut self, v: &Viewer, rel: &str) -> Vec<(f32, u8)> {
        if let Some(s) = self.spuren_cache.get(rel) {
            return s.clone();
        }
        let s: Vec<(f32, u8)> = v.spline_lanes(rel).map(|(l, _)| l.into_iter().filter(|x| x.0 == 0).map(|x| (x.1, x.4)).collect()).unwrap_or_default();
        self.spuren_cache.insert(rel.to_string(), s.clone());
        s
    }

    /// passt der gewaehlte Querschnitt an das Ende einer vorhandenen Strasse? (die neue Strasse verlaesst den Punkt
    /// in a.richtung: am Spline-Ende setzt sie ihn gleichlaeufig fort, am Anfang gegenlaeufig)
    fn passt(&mut self, v: &Viewer, sli: &str, a: &Anschluss) -> bool {
        let neu = self.spuren(v, sli);
        let alt = self.spuren(v, &a.sli);
        spuren_passen(&neu, &alt, a.am_ende != a.gespiegelt)
    }

    /// Klick: Start setzen bzw. geplante Kante bauen
    pub fn klick(&mut self, v: &mut Viewer, boden: DVec3, fang: f64, ans: &Anschluesse, mut ae: Option<&mut Aendern>) -> Option<String> {
        self.sli.as_ref()?;
        match self.start.clone() {
            None => {
                let mut meldung = "Start gesetzt - Klick setzt den naechsten Punkt, Rechtsklick/Esc beendet".to_string();
                self.start = Some(match self.punkt(v, boden, fang, ans, ae) {
                    Ziel::Knoten(k, pos) => {
                        if self.netz.knoten(k).is_some_and(|x| x.anschluss.is_some()) && !self.netz.an(k).is_empty() {
                            return Some("hier schliesst die Strasse an eine vorhandene an - eine Kreuzung an dieser Stelle geht nicht; etwas weiter auf der Strasse abzweigen".into());
                        }
                        let meldung_k = if self.netz.an(k).len() >= 2 { "Abzweig vom Knoten: hier entsteht eine Kreuzung" } else { "" };
                        if !meldung_k.is_empty() {
                            meldung = meldung_k.into();
                        }
                        Start { knoten: Some(k), anschluss: None, abzweig: None, kante: None, pos, richtung: self.netz.weiter_richtung(k) }
                    }
                    Ziel::Anschluss(a) => {
                        // Querschnitt der vorhandenen Strasse uebernehmen, wenn der gewaehlte nicht passt
                        let sli = self.sli.clone().unwrap();
                        if self.uebernehmen && !self.passt(v, &sli, &a) {
                            self.sli = Some(a.sli.clone());
                            meldung = format!("an vorhandene Strasse angeschlossen - Querschnitt uebernommen: {}", a.sli.rsplit('\\').next().unwrap_or(""));
                        } else {
                            meldung = "an vorhandene Strasse angeschlossen".into();
                        }
                        Start { knoten: None, anschluss: Some(a.clone()), abzweig: None, kante: None, pos: a.pos, richtung: Some(a.richtung) }
                    }
                    Ziel::Abzweig(ab) => {
                        meldung = "Abzweig: hier entsteht eine Kreuzung - Klick setzt das Ende des ersten Stuecks".into();
                        Start { knoten: None, anschluss: None, pos: ab.pos, richtung: None, abzweig: Some(ab), kante: None }
                    }
                    Ziel::Kante(kp) => {
                        meldung = "Abzweig von der eigenen Strasse: hier entsteht eine Kreuzung - Klick setzt das Ende des ersten Stuecks".into();
                        Start { knoten: None, anschluss: None, pos: kp.pos, richtung: None, abzweig: None, kante: Some(kp) }
                    }
                    Ziel::Frei(pos) => Start { knoten: None, anschluss: None, abzweig: None, kante: None, pos, richtung: None },
                });
                self.zeiger = None;
                Some(meldung)
            }
            Some(s) => {
                let sli = self.sli.clone()?;
                let p = self.plan.clone()?;
                if p.laenge < 1.0 {
                    return None;
                }
                // Kreuzungen zuerst (sie koennen scheitern, z. B. zu nah am Strassenende): die Strasse beginnt bzw.
                // endet an ihrem neuen Arm, eben auf Kreuzungshoehe
                let mut kreuzungen = 0;
                let mut arm_a = None;
                let mut arm_b = None;
                let mut kreuzung_texte = Vec::new();
                for (ab, h, weg) in [(s.abzweig.as_ref(), p.ha, true), (p.ziel_abzweig.as_ref(), (p.hb + 180.0).rem_euclid(360.0), false)] {
                    let Some(ab) = ab else { continue };
                    let Some(ae) = ae.as_deref_mut() else { return Some("Kreuzung: Aendern-Werkzeug nicht bereit".into()) };
                    match ae.kreuzung_bauen(v, ab, h, &sli, weg) {
                        Ok((arm, obj)) => {
                            kreuzungen += 1;
                            kreuzung_texte.push(format!("{} Abbiegespuren", obj.spuren));
                            if weg { arm_a = Some(arm) } else { arm_b = Some(arm) }
                        }
                        Err(e) => {
                            for _ in 0..kreuzungen {
                                let _ = ae.rueckgaengig(v);
                            }
                            return Some(format!("Kreuzung nicht moeglich: {e:#}"));
                        }
                    }
                }
                if let Some(b) = p.blockiert.as_ref() {
                    return Some(format!("Kreuzung nicht moeglich: {b}"));
                }
                if p.ziel.is_some_and(|k| self.netz.knoten(k).is_some_and(|x| x.anschluss.is_some()) && !self.netz.an(k).is_empty()) {
                    return Some("dort schliesst eine Strasse an eine vorhandene an - eine Kreuzung an dieser Stelle geht nicht".into());
                }
                self.undo.push((self.netz.clone(), kreuzungen));
                self.redo.clear();
                self.aenderungen += 1;
                self.netz.breiten.insert(sli.clone(), self.halb(v, &sli));
                // eigene Strassen an Start/Ziel teilen: der neue Knoten wird mit der neuen Kante eine Kreuzung
                let geteilt_a = p.kante_a.and_then(|(id, s)| self.netz.kante_teilen(id, s));
                let geteilt_b = p.kante_b.and_then(|q| {
                    let (id, s, _, _) = self.netz.kante_bei(q.truncate())?;
                    self.netz.kante_teilen(id, s)
                });
                let a = match (geteilt_a, s.knoten, &s.anschluss, &arm_a) {
                    (Some(k), ..) => k,
                    (_, _, _, Some(x)) => self.netz.anschluss_neu(x.pos, x.richtung, 0.0),
                    (_, Some(k), _, _) => k,
                    (_, None, Some(x), _) => self.netz.anschluss_neu(x.pos, x.richtung, x.steigung),
                    (_, None, None, _) => self.netz.knoten_neu(s.pos),
                };
                let b = match (geteilt_b, &p.ziel, &p.ziel_anschluss, &arm_b) {
                    (Some(k), ..) => k,
                    (_, _, _, Some(x)) => self.netz.anschluss_neu(x.pos, x.richtung, 0.0),
                    (_, Some(k), _, _) => *k,
                    (_, None, Some(x), _) => self.netz.anschluss_neu(x.pos, x.richtung, x.steigung),
                    (_, None, None, _) => self.netz.knoten_neu(p.b),
                };
                // an Kreuzungen genau in Richtung ihres Arms (die Vorschau kann um Bruchteile abweichen)
                let ha = arm_a.as_ref().map(|x| x.richtung).unwrap_or(p.ha);
                let hb = arm_b.as_ref().map(|x| (x.richtung + 180.0).rem_euclid(360.0)).unwrap_or(p.hb);
                self.netz.kante_neu(a, b, &sli, ha, hb);
                // an Kreuzungen gekuerzt: kein Stueck darf ganz verschwinden
                if self.netz.kanten.iter().any(|e| self.netz.elemente(e).is_empty()) {
                    if let Some((n, k)) = self.undo.pop() {
                        self.netz = n;
                        for _ in 0..k {
                            if let Some(a) = ae.as_deref_mut() {
                                let _ = a.rueckgaengig(v);
                            }
                        }
                    }
                    return Some("zu kurz: zwischen den Kreuzungen bliebe kein Strassenstueck - die Punkte weiter auseinander setzen".into());
                }
                self.zeichnen_alle(v);
                let n_netz = self.objekte.len();
                // weiter vom neuen Ende; endete der Zug auf einem vorhandenen Knoten oder einer Strasse, ist er fertig
                let fertig = p.ziel.is_some() || p.ziel_anschluss.is_some() || p.ziel_abzweig.is_some() || geteilt_b.is_some();
                let b_pos = self.netz.knoten(b).map(|k| k.pos).unwrap_or(p.b);
                self.start = if fertig { None } else { Some(Start { knoten: Some(b), anschluss: None, abzweig: None, kante: None, pos: b_pos, richtung: Some(p.hb) }) };
                if geteilt_a.is_some() || geteilt_b.is_some() || self.netz.ist_kreuzung(a) || self.netz.ist_kreuzung(b) {
                    kreuzung_texte.push(format!("im eigenen Netz, dort jetzt {n_netz}"));
                    kreuzungen += 1;
                }
                self.vorschau_weg(v);
                Some(format!("Strasse gebaut: {:.1} m{}{}{}", p.laenge, if fertig { ", angeschlossen" } else { "" },
                             if kreuzungen > 0 { format!(", Kreuzung ({})", kreuzung_texte.join(", ")) } else { String::new() },
                             p.warnung.as_ref().map(|w| format!(" - {w}")).unwrap_or_default()))
            }
        }
    }

    /// Zug beenden (Rechtsklick/Esc)
    pub fn beenden(&mut self, v: &mut Viewer) {
        self.start = None;
        self.plan = None;
        self.vorschau_weg(v);
    }

    /// Vorschau zur Maus berechnen und zeichnen
    pub fn maus(&mut self, v: &mut Viewer, boden: DVec3, fang: f64, ans: &Anschluesse, mut ae: Option<&mut Aendern>) {
        let Some(sli) = self.sli.clone() else {
            self.vorschau_weg(v);
            return;
        };
        if let Some(a) = ae.as_deref() {
            self.kreuzungs_ordner = Some(a.kreuzungs_ordner());
        }
        let ziel = self.punkt(v, boden, fang, ans, ae.as_deref_mut());
        let ae = ae.as_deref();
        let Some(mut s) = self.start.clone() else {
            // noch kein Start: zeigen, wo eine Kreuzung entstuende
            self.zeiger = match &ziel {
                Ziel::Abzweig(ab) => self.arm_plan(v, ae, ab, ab.richtung + 90.0, &sli).ok().map(|x| x.2),
                Ziel::Kante(kp) => self.kante_plan(v, kp, kp.richtung + 90.0, &sli).ok().map(|x| x.3),
                _ => None,
            };
            self.vorschau_weg(v);
            return;
        };
        let mut ziel_kante = None;
        let (ziel_knoten, ziel_anschluss, ziel_abzweig, mut b) = match &ziel {
            Ziel::Knoten(k, p) => (Some(*k), None, None, *p),
            Ziel::Anschluss(a) => (None, Some(a.clone()), None, a.pos),
            Ziel::Abzweig(ab) => (None, None, Some(ab.clone()), ab.pos),
            Ziel::Kante(kp) => {
                ziel_kante = Some(kp.clone());
                (None, None, None, kp.pos)
            }
            Ziel::Frei(p) => (None, None, None, *p),
        };
        let gleich = match (&ziel_anschluss, &s.anschluss, &ziel_abzweig, &s.abzweig) {
            (Some(x), Some(y), _, _) => x.spline_id == y.spline_id && x.am_ende == y.am_ende,
            (_, _, Some(x), Some(y)) => x.spline_id == y.spline_id && (x.s - y.s).abs() < 40.0,
            _ => match (&ziel_kante, &s.kante) {
                (Some(x), Some(y)) => x.kante == y.kante && (x.s - y.s).abs() < 40.0,
                // Ziel auf einer Kante am Startknoten
                (Some(x), None) => s.knoten.is_some_and(|k| self.netz.kante(x.kante).is_some_and(|e| e.a == k || e.b == k) && (x.pos - s.pos).length() < 30.0),
                _ => ziel_knoten.is_some() && ziel_knoten == s.knoten,
            },
        };
        if gleich {
            self.vorschau_weg(v);
            self.plan = None;
            return;
        }
        // Kreuzungen: der Start liegt am Ende des neuen Arms, der zum Ziel zeigt; ein Ziel mitten auf einer Strasse
        // bekommt einen Arm, der zum Start zeigt
        let mut kreuzungen = Vec::new();
        let mut kreuzung_fehler = None;
        if let Some(ab) = s.abzweig.clone() {
            match self.arm_plan(v, ae, &ab, richtung(ab.pos.truncate(), b.truncate()), &sli) {
                Ok((arm, h, k)) => {
                    s.pos = arm;
                    s.richtung = Some(h);
                    kreuzungen.push(k);
                }
                Err(e) => kreuzung_fehler = Some(e),
            }
        }
        // eigene Strasse am Start/Ziel: Kreuzung mitten darauf (die neue Kante beginnt am Knoten, gezeichnet wird
        // erst ab dem Ende des Arms)
        let (mut kante_a, mut kante_b, mut kuerz_a, mut kuerz_b) = (None, None, 0.0, 0.0);
        if let Some(kp) = s.kante.clone() {
            match self.kante_plan(v, &kp, richtung(kp.pos.truncate(), b.truncate()), &sli) {
                Ok((mitte, h, d_arm, k, sk)) => {
                    s.pos = mitte;
                    s.richtung = Some(h);
                    kuerz_a = d_arm;
                    kante_a = Some((kp.kante, sk));
                    kreuzungen.push(k);
                }
                Err(e) => kreuzung_fehler = Some(e),
            }
        }
        let mut ankunft_abzweig = None;
        if let Some(kp) = ziel_kante.as_ref() {
            match self.kante_plan(v, kp, richtung(kp.pos.truncate(), s.pos.truncate()), &sli) {
                Ok((mitte, h, d_arm, k, _)) => {
                    b = mitte;
                    ankunft_abzweig = Some((h + 180.0).rem_euclid(360.0));
                    kuerz_b = d_arm;
                    kante_b = Some(mitte);
                    kreuzungen.push(k);
                }
                Err(e) => kreuzung_fehler = Some(e),
            }
        }
        if let Some(ab) = ziel_abzweig.as_ref() {
            match self.arm_plan(v, ae, ab, richtung(ab.pos.truncate(), s.pos.truncate()), &sli) {
                Ok((arm, h, k)) => {
                    b = arm;
                    ankunft_abzweig = Some((h + 180.0).rem_euclid(360.0));
                    kreuzungen.push(k);
                }
                Err(e) => kreuzung_fehler = Some(e),
            }
        }
        let a = s.pos;
        // Ankunftsrichtung, wenn das Ziel ein freies Ende ist (eigenes oder einer vorhandenen Strasse)
        let ankunft = match (&ziel_anschluss, ziel_knoten) {
            (Some(x), _) => Some((x.richtung + 180.0).rem_euclid(360.0)),
            (None, Some(k)) => self.netz.weiter_richtung(k).map(|h| (h + 180.0).rem_euclid(360.0)),
            _ => ankunft_abzweig,
        };
        let (ha, hb, stuecke) = match (self.modus, s.richtung, ankunft) {
            (_, Some(t), Some(hz)) => (t, hz, verbinden(a.truncate(), t, b.truncate(), hz)),
            (Modus::Gerade, Some(t), None) => {
                // geradeaus weiter: Ziel auf die Verlaengerung projizieren
                let d = netz::dir(t);
                let l = (b.truncate() - a.truncate()).dot(d).max(1.0);
                let q = a.truncate() + d * l;
                b = DVec3::new(q.x, q.y, v.terrain_height(q.x, q.y).map(|z| z + self.hoehe).unwrap_or(b.z));
                (t, t, verbinden(a.truncate(), t, q, t))
            }
            (Modus::Kurve, Some(t), None) => {
                let st = bogen_durch(a.truncate(), t, b.truncate());
                let he = st.ende().1;
                (t, he, vec![st])
            }
            (_, None, Some(hz)) => {
                // freier Start, Ziel mit Richtung: Bogen rueckwaerts vom Ziel
                let st = bogen_durch(b.truncate(), (hz + 180.0).rem_euclid(360.0), a.truncate());
                let ha = (st.ende().1 + 180.0).rem_euclid(360.0);
                (ha, hz, verbinden(a.truncate(), ha, b.truncate(), hz))
            }
            (_, None, None) => {
                let h = richtung(a.truncate(), b.truncate());
                (h, h, verbinden(a.truncate(), h, b.truncate(), h))
            }
        };
        let laenge: f64 = stuecke.iter().map(|s| s.laenge).sum();
        // Ziel hinter der Fahrtrichtung oder entartet: keine Vorschau (sonst riesige Boegen)
        let luftlinie = (b.truncate() - a.truncate()).length();
        if stuecke.is_empty() || !laenge.is_finite() || laenge > netz::MAX_PLAN || laenge > 4.0 * luftlinie + 300.0 {
            self.vorschau_weg(v);
            self.plan = None;
            return;
        }
        let min_radius = stuecke.iter().filter(|s| s.radius != 0.0).map(|s| s.radius.abs()).fold(f64::INFINITY, f64::min);
        let steigung = if laenge > 0.0 { (b.z - a.z) / laenge * 100.0 } else { 0.0 };
        let kreuzung_fehler_text = kreuzung_fehler.clone();
        let warnung = match &ziel_anschluss {
            Some(x) if !self.passt(v, &sli, x) => Some("Spuren passen nicht zur vorhandenen Strasse".to_string()),
            _ => kreuzung_fehler.map(|e| format!("keine Kreuzung moeglich: {e}")),
        };
        // Steigung an den Enden: an vorhandenen Strassen deren Steigung, an eigenen Knoten die der Kante dort
        let sehne = (b.z - a.z) / laenge.max(1.0);
        let ga = match (&s.anschluss, s.knoten, &s.abzweig) {
            _ if s.kante.is_some() => 0.0,
            (_, _, Some(_)) => 0.0,
            (Some(x), _, _) => x.steigung,
            (None, Some(k), _) => self.steigung_aus(k, ha),
            _ => sehne,
        };
        let gb = match (&ziel_anschluss, &ziel_abzweig) {
            _ if ziel_kante.is_some() => 0.0,
            (_, Some(_)) => 0.0,
            (Some(x), _) => -x.steigung,
            _ => sehne,
        };
        self.plan = Some(Plan { a, b, ha, hb, ziel: ziel_knoten, ziel_anschluss, ziel_abzweig, kreuzungen, kante_a, kante_b,
                                blockiert: kreuzung_fehler_text, laenge, min_radius, steigung, warnung });
        self.vorschau_weg(v);
        // an Kreuzungen auf eigenen Strassen beginnt/endet die Vorschau am Arm
        let stuecke = if kuerz_a > 0.0 || kuerz_b > 0.0 { netz::schneiden(&stuecke, kuerz_a, laenge - kuerz_b) } else { stuecke };
        let el = netz::mit_hoehe(&stuecke, a.z, ga, b.z, gb);
        let mut cum = 0.0;
        for e in &el {
            if let Some(g) = v.add_spline(&sli, &e.kurve(1, cum)) {
                self.vorschau.push(g);
            }
            cum += e.stueck.laenge;
        }
    }

    /// Steigung (Verhaeltnis) beim Verlassen von Knoten k in Richtung h: die der anschliessenden Kante
    fn steigung_aus(&self, k: u32, h: f64) -> f64 {
        let an = self.netz.an(k);
        if an.len() != 1 {
            return 0.0;
        }
        let e = an[0];
        let el = self.netz.elemente(e);
        match (e.b == k, el.first(), el.last()) {
            (true, _, Some(l)) if norm180(l.stueck.ende().1 - h).abs() < 1.0 => l.stg_e / 100.0,
            (false, Some(f), _) if norm180(f.stueck.richtung + 180.0 - h).abs() < 1.0 => -f.stg_a / 100.0,
            _ => 0.0,
        }
    }

    fn vorschau_weg(&mut self, v: &mut Viewer) {
        for g in self.vorschau.drain(..) {
            v.remove_object(g);
        }
    }

    /// alle Kanten neu zeichnen (nach jeder Aenderung: die Hoehen an Verbindungsknoten haengen von beiden Seiten ab)
    pub fn zeichnen_alle(&mut self, v: &mut Viewer) {
        for (_, gs) in self.gezeichnet.drain() {
            for g in gs {
                v.remove_object(g);
            }
        }
        for e in self.netz.kanten.clone() {
            let mut gs = Vec::new();
            let mut cum = 0.0;
            for el in self.netz.elemente(&e) {
                if let Some(g) = v.add_spline(&e.sli, &el.kurve(e.id, cum)) {
                    gs.push(g);
                }
                cum += el.stueck.laenge;
            }
            self.gezeichnet.insert(e.id, gs);
        }
        self.kreuzungen_aktualisieren(v);
    }

    /// Kreuzungsobjekte des eigenen Netzes: fuer jeden Knoten mit 3 und mehr Kanten eines (von omsigen, je
    /// Anordnung der Arme einmal erzeugt), ueberzaehlige weg
    fn kreuzungen_aktualisieren(&mut self, v: &mut Viewer) {
        let knoten: Vec<u32> = self.netz.knoten.iter().map(|k| k.id).filter(|k| self.netz.ist_kreuzung(*k)).collect();
        let weg: Vec<u32> = self.objekte.keys().copied().filter(|k| !knoten.contains(k)).collect();
        for k in weg {
            if let Some(g) = self.objekte.remove(&k).and_then(|o| o.gpu) {
                v.remove_object(g);
            }
        }
        for k in knoten {
            let Some(hoehe) = self.netz.knoten(k).map(|x| x.pos.z) else { continue };
            let mut arme = Vec::new();
            for arm in self.netz.arme(k) {
                let Some(e) = self.netz.kante(arm.kante) else { continue };
                let el = self.netz.elemente(e);
                let pos = if arm.weg {
                    el.first().map(|x| x.stueck.start.extend(x.z))
                } else {
                    el.last().map(|x| x.stueck.ende().0.extend(x.z + x.dh))
                };
                let Some(pos) = pos else { continue };
                arme.push(kreuzung::Arm { pos, richtung: arm.richtung, sli: e.sli.clone(), weg: arm.weg, rolle: kreuzung::Rolle::Gleich });
            }
            if arme.len() < 3 {
                continue;
            }
            // durchgehende Strasse (zwei fast gegenueberliegende Arme gleichen Querschnitts) hat Vorfahrt
            'paar: for i in 0..arme.len() {
                for j in i + 1..arme.len() {
                    if arme[i].sli == arme[j].sli && norm180(arme[i].richtung - arme[j].richtung).abs() > 150.0 {
                        for (n, a) in arme.iter_mut().enumerate() {
                            a.rolle = if n == i || n == j { kreuzung::Rolle::Haupt } else { kreuzung::Rolle::Neben };
                        }
                        break 'paar;
                    }
                }
            }
            let signatur: String = arme.iter().map(|a| format!("{:.2},{:.2},{:.2},{:.3},{},{},{:?};", a.pos.x, a.pos.y, a.pos.z, a.richtung, a.sli, a.weg, a.rolle)).collect();
            if self.objekte.get(&k).is_some_and(|o| o.signatur == signatur) {
                continue;
            }
            if let Some(g) = self.objekte.remove(&k).and_then(|o| o.gpu) {
                v.remove_object(g);
            }
            let objekt = match self.objekt_cache.get(&signatur) {
                Some(o) => o.clone(),
                None => {
                    let Some((ordner, tag)) = self.kreuzungs_ordner.clone() else {
                        self.kreuzung_fehler = Some("Kreuzungsobjekt: kein Sitzungsordner".into());
                        continue;
                    };
                    let rel_ordner = format!("Sceneryobjects\\Aschaffenburg_KI\\{tag}");
                    match kreuzung::erzeugen(&v.root, &ordner, &rel_ordner, &kreuzung::freier_name(&ordner), &arme) {
                        Ok(o) => {
                            log::info!("Kreuzung im eigenen Netz (Knoten {k}): {} mit {} Abbiegespuren", o.rel, o.spuren);
                            omsi_cfg::content_changed();
                            self.objekt_cache.insert(signatur.clone(), o.clone());
                            o
                        }
                        Err(e) => {
                            log::warn!("Kreuzung im eigenen Netz (Knoten {k}): {e:#}");
                            self.kreuzung_fehler = Some(format!("{e:#}"));
                            continue;
                        }
                    }
                }
            };
            let gpu = v.add_object(&objekt.rel, objekt.ursprung.extend(hoehe), 0.0);
            self.objekte.insert(k, NetzObjekt { signatur, objekt, hoehe, gpu });
        }
    }

    /// Kreuzungsobjekte des eigenen Netzes fuers Speichern: (.sco, Lage, Vorfahrtregeln)
    pub fn gesetzte_kreuzungen(&self) -> Vec<(String, DVec3, Vec<(usize, i32)>)> {
        self.objekte.values().map(|o| (o.objekt.rel.clone(), o.objekt.ursprung.extend(o.hoehe), o.objekt.rules.clone())).collect()
    }

    fn merken(&mut self) {
        self.undo.push((self.netz.clone(), 0));
        self.redo.clear();
        self.aenderungen += 1;
    }

    /// letzten Schritt zuruecknehmen (mit seinen Kreuzungen, die im Aendern-Werkzeug liegen)
    pub fn rueckgaengig(&mut self, v: &mut Viewer, mut ae: Option<&mut Aendern>) -> bool {
        let Some((n, k)) = self.undo.pop() else { return false };
        for _ in 0..k {
            if let Some(a) = ae.as_deref_mut() {
                let _ = a.rueckgaengig(v);
            }
        }
        self.redo.push((std::mem::replace(&mut self.netz, n), k));
        self.beenden(v);
        self.zeichnen_alle(v);
        self.aenderungen += 1;
        true
    }

    pub fn wiederholen(&mut self, v: &mut Viewer, mut ae: Option<&mut Aendern>) -> bool {
        let Some((n, k)) = self.redo.pop() else { return false };
        for _ in 0..k {
            if let Some(a) = ae.as_deref_mut() {
                let _ = a.wiederholen(v);
            }
        }
        self.undo.push((std::mem::replace(&mut self.netz, n), k));
        self.beenden(v);
        self.zeichnen_alle(v);
        self.aenderungen += 1;
        true
    }

    pub fn kann_rueckgaengig(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn kann_wiederholen(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Kante unter dem Bodenpunkt (waagerecht in r Metern) loeschen
    pub fn loeschen_bei(&mut self, v: &mut Viewer, p: DVec2, r: f64) -> Option<String> {
        let mut best: Option<(f64, u32)> = None;
        for e in &self.netz.kanten {
            for el in self.netz.elemente(e) {
                let n = (el.stueck.laenge / 2.0).ceil().max(1.0) as usize;
                for i in 0..=n {
                    let (q, _) = el.stueck.bei(el.stueck.laenge * i as f64 / n as f64);
                    let d = (q - p).length();
                    if d <= r && best.map(|b| d < b.0).unwrap_or(true) {
                        best = Some((d, e.id));
                    }
                }
            }
        }
        let (_, id) = best?;
        self.merken();
        self.netz.kante_loeschen(id);
        self.zeichnen_alle(v);
        Some(format!("Strasse {id} geloescht"))
    }

    /// Hilfslinien: Knoten des Netzes (fuer die Markierung im Bild)
    pub fn knoten_punkte(&self) -> Vec<(u32, DVec3, bool)> {
        self.netz.knoten.iter().map(|k| (k.id, k.pos, self.netz.weiter_richtung(k.id).is_some())).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn querschnitte_der_installation() {
        let root = Path::new(crate::bearbeiten::tests::OMSI);
        if !root.exists() {
            return;
        }
        let q = querschnitte(root);
        assert!(q.len() > 50, "{}", q.len());
        let g = q.iter().find(|q| q.name == "str_2spur_10m_Grunewaldstr").expect("Grunewaldstr");
        assert!(g.vor >= 1 && g.zurueck >= 1 && g.breite > 8.0, "{g:?}");
    }

    #[test]
    #[ignore]
    fn strasse_bauen_speichern_und_laden() {
        let _sperre = crate::bearbeiten::tests::sperre();
        use crate::bearbeiten::{tests::grundorf, Bearbeiten, Werkzeug};
        let root = Path::new(crate::bearbeiten::tests::OMSI);
        let mut v = grundorf();
        let mut s = Strassenbau::neu(Some("Splines\\Marcel\\str_2spur_10m_Grunewaldstr.sli".into()), Modus::Gerade);
        let boden = |v: &Viewer, x: f64, y: f64| DVec3::new(x, y, v.terrain_height(x, y).unwrap_or(0.0));
        // Gerade nach Norden, dann tangential eine Rechtskurve, 3 m hoeher
        let a = boden(&v, 120.0, 100.0);
        s.klick(&mut v, a, 3.0, &Anschluesse::default(), None).unwrap();
        let b = boden(&v, 120.0, 180.0);
        s.maus(&mut v, b, 3.0, &Anschluesse::default(), None);
        s.klick(&mut v, b, 3.0, &Anschluesse::default(), None).unwrap();
        s.modus = Modus::Kurve;
        s.hoehe = 3.0;
        let c = boden(&v, 190.0, 240.0);
        s.maus(&mut v, c, 3.0, &Anschluesse::default(), None);
        let plan = s.plan.clone().unwrap();
        assert!(plan.min_radius > MIN_RADIUS, "Radius {}", plan.min_radius);
        s.klick(&mut v, c, 3.0, &Anschluesse::default(), None).unwrap();
        s.beenden(&mut v);
        assert_eq!((s.netz.knoten.len(), s.netz.kanten.len()), (3, 2));
        // Rueckgaengig/Wiederholen
        assert!(s.rueckgaengig(&mut v, None));
        assert_eq!(s.netz.kanten.len(), 1);
        assert!(s.wiederholen(&mut v, None));
        assert_eq!(s.netz.kanten.len(), 2);
        // speichern in einen Test-OMSI-Ordner
        let test_root = std::env::temp_dir().join(format!("omsi-editor-strasse-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&test_root);
        fn kopieren(a: &Path, b: &Path) {
            std::fs::create_dir_all(b).unwrap();
            for e in std::fs::read_dir(a).unwrap().flatten() {
                if e.file_type().unwrap().is_dir() { kopieren(&e.path(), &b.join(e.file_name())) } else { std::fs::copy(e.path(), b.join(e.file_name())).unwrap(); }
            }
        }
        kopieren(&root.join("maps/Grundorf"), &test_root.join("maps/Grundorf"));
        let start = v.next_object_id();
        let erwartet: usize = s.netz.kanten.iter().map(|e| s.netz.elemente(e).len()).sum();
        let ziel = crate::speichern::alles_speichern(&v, &Bearbeiten::neu(Werkzeug::Strasse), &s.netz, "Grundorf", "Grundorf_strasse", &test_root).unwrap();
        // mit openOMSIs Kachel-Parser lesen
        let mut neue: Vec<(omsi_geometry::SplineCurve, i64, i64, i64)> = Vec::new();
        for e in std::fs::read_dir(&ziel).unwrap().flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            let Some(rest) = n.strip_prefix("tile_").and_then(|r| r.strip_suffix(".map")) else { continue };
            let mut it = rest.split('_').map(|x| x.parse::<i32>().unwrap());
            let (tx, ty) = (it.next().unwrap(), it.next().unwrap());
            let t = omsi_map::Tile::load(&e.path()).unwrap();
            for sp in t.splines.iter().filter(|sp| sp.id >= start) {
                let o = glam::DVec2::new(tx as f64 * 300.0, ty as f64 * 300.0);
                assert!(sp.delta_h.is_some(), "spline_h erwartet");
                neue.push((omsi_geometry::SplineCurve::from_map(sp, o), sp.id, sp.prev_id, sp.next_id));
            }
        }
        assert_eq!(neue.len(), erwartet, "Anzahl Splines");
        // jedes Ende liegt (Lage, Hoehe, Richtung) genau am Anfang eines anderen Splines, ausser den zwei Zugenden
        let mut offen = 0;
        for (k, ..) in &neue {
            let ende = k.end_point();
            let he = k.heading_at(k.length);
            let anschluss = neue.iter().any(|(m, ..)| (m.start - ende).length() < 0.01 && norm180(m.heading_deg - he).abs() < 0.05);
            if !anschluss {
                offen += 1;
            }
        }
        assert_eq!(offen, 1, "nur das Ende des Zugs darf offen sein");
        // Ketten-IDs: prev/next innerhalb einer Kante verweisen aufeinander
        for (_, id, _, next) in &neue {
            if *next != 0 {
                assert!(neue.iter().any(|(_, i, prev, _)| i == next && prev == id));
            }
        }
        // letzter Punkt 3 m ueber dem Gelaende bei c
        let hoechst = neue.iter().map(|(k, ..)| k.end_point()).find(|p| (p.truncate() - c.truncate()).length() < 0.01).unwrap();
        assert!((hoechst.z - (c.z + 3.0)).abs() < 0.01, "Hoehe {}", hoechst.z);
        std::fs::remove_dir_all(&test_root).ok();
    }

    #[test]
    #[ignore]
    fn an_vorhandene_strasse_anschliessen() {
        let _sperre = crate::bearbeiten::tests::sperre();
        use crate::bearbeiten::{tests::grundorf, Bearbeiten, Werkzeug};
        let root = Path::new(crate::bearbeiten::tests::OMSI);
        let mut v = grundorf();
        v.tiles_around(DVec3::new(150.0, 150.0, 0.0), 8).unwrap(); // ganze Karte: keine Raender
        let mut ans = Anschluesse::default();
        ans.aktualisieren(&v);
        // freies Ende mitten im geladenen Bereich (Grundorf: Kachel 0 0 mit Nachbarn)
        let a = ans.liste.iter().find(|a| a.frei).cloned().expect("kein freies Ende");
        println!("freies Ende: Spline {} {} bei {:?}, weiter Richtung {:.1}, Querschnitt {}", a.spline_id,
                 if a.am_ende { "Ende" } else { "Anfang" }, a.pos, a.richtung, a.sli);
        let mut s = Strassenbau::neu(Some("Splines\\Marcel\\str_2spur_10m_Grunewaldstr.sli".into()), Modus::Kurve);
        let m = s.klick(&mut v, a.pos + DVec3::new(0.5, 0.5, 0.0), 3.0, &ans, None).unwrap();
        println!("{m}");
        // 60 m geradeaus weiter, dann etwas zur Seite
        let d = netz::dir(a.richtung);
        let r = netz::rechts(a.richtung);
        let z = a.pos.truncate() + d * 60.0 + r * 15.0;
        let ziel = DVec3::new(z.x, z.y, v.terrain_height(z.x, z.y).unwrap_or(a.pos.z));
        s.maus(&mut v, ziel, 3.0, &ans, None);
        assert!(s.plan.as_ref().unwrap().warnung.is_none());
        s.klick(&mut v, ziel, 3.0, &ans, None).unwrap();
        s.beenden(&mut v);
        // die erste Kante verlaesst das Ende genau in dessen Richtung und auf dessen Hoehe
        let e = &s.netz.kanten[0];
        let el = s.netz.elemente(e);
        assert!((el[0].stueck.start - a.pos.truncate()).length() < 1e-6 && (el[0].z - a.pos.z).abs() < 1e-6);
        assert!(norm180(el[0].stueck.richtung - a.richtung).abs() < 1e-6);
        assert!((el[0].stg_a / 100.0 - a.steigung).abs() < 1e-6, "Steigung {} statt {}", el[0].stg_a / 100.0, a.steigung);
        // speichern, neu laden: das bisher freie Ende ist jetzt im Spurnetz angeschlossen
        let test_root = std::env::temp_dir().join(format!("omsi-editor-anschluss-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&test_root);
        fn kopieren(a: &Path, b: &Path) {
            std::fs::create_dir_all(b).unwrap();
            for e in std::fs::read_dir(a).unwrap().flatten() {
                if e.file_type().unwrap().is_dir() { kopieren(&e.path(), &b.join(e.file_name())) } else { std::fs::copy(e.path(), b.join(e.file_name())).unwrap(); }
            }
        }
        kopieren(&root.join("maps/Grundorf"), &test_root.join("maps/Grundorf"));
        let ziel_dir = crate::speichern::alles_speichern(&v, &Bearbeiten::neu(Werkzeug::Strasse), &s.netz, "Grundorf", "Grundorf_anschluss", &test_root).unwrap();
        drop(v);
        let instance = openomsi_game::viewer::instance();
        let (mut v2, _) = Viewer::open(&instance, None, root, &ziel_dir.join("global.cfg")).unwrap();
        v2.tiles_around(DVec3::new(150.0, 150.0, 0.0), 8).unwrap();
        assert_eq!(v2.spline_end_free(a.spline_id, a.am_ende), Some(false), "das Ende ist im Spiel nicht angeschlossen");
        std::fs::remove_dir_all(&test_root).ok();
    }
}
