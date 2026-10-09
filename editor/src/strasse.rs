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
    /// Kreisverkehr: Klick setzt die Mitte, die Maus die Groesse
    Kreisel,
}

/// kleinster und groesster Halbmesser eines Kreisverkehrs (Mittellinie des Rings)
pub const KREISEL_MIN: f64 = 12.0;
pub const KREISEL_MAX: f64 = 60.0;
/// Breite der Ringfahrbahn (einspurig mit Rand)
pub const KREISEL_BREITE: f64 = 7.0;
/// kleinster Winkel zwischen zwei Zufahrten eines Kreisverkehrs
pub const KREISEL_ARM_WINKEL: f64 = 30.0;

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

/// Querschnitt fuer Kreisverkehre: Einbahn mit einer Spur und Gehweg, am liebsten aus den Standardsplines
pub fn kreisel_vorschlag(q: &[Querschnitt]) -> Option<String> {
    let einbahn = |x: &&Querschnitt| x.vor == 1 && x.zurueck == 0;
    q.iter().filter(einbahn).filter(|x| x.gehwege > 0).max_by_key(|x| (x.ordner == "Marcel", (x.breite * 10.0) as i32))
        .or_else(|| q.iter().find(einbahn)).map(|x| x.rel.clone())
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

/// Anfang oder Ende einer geplanten Strasse (wohin ein Klick faellt)
#[derive(Clone, Debug)]
enum Ende {
    /// eigener Knoten (mit 2 und mehr Kanten wird er eine Kreuzung)
    Knoten(u32),
    /// freies Ende einer vorhandenen Strasse
    Anschluss(Anschluss),
    /// mitten auf einer vorhandenen Strasse: sie wird aufgeschnitten, dort entsteht eine Kreuzung
    Abzweig(Abzweig),
    /// mitten auf einer eigenen Strasse: sie wird geteilt, der Knoten wird eine Kreuzung
    Kante(Kantenpunkt),
    /// vorhandenes Kreuzungsobjekt: wird durch eine eigene Kreuzung mit dem neuen Arm ersetzt
    Vorhanden(kreuzung::Vorhanden),
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

/// ein Ende fuer die Planung aufgeloest: wo die Kante beginnt/endet (Knotenmitte) und in welcher Richtung
#[derive(Clone, Debug)]
struct Aufgeloest {
    /// das Ende, Kreuzungen eingepasst (Abzweig/Kantenpunkt an der endgueltigen Stelle)
    ende: Ende,
    pos: DVec3,
    /// Richtung beim Verlassen (Start) bzw. beim Ankommen (Ziel)
    richtung: Option<f64>,
    /// Steigung (Verhaeltnis) in Fahrtrichtung der neuen Kante, None: nach der Sehne
    steigung: Option<f64>,
    /// so weit vom Knoten beginnt die Strasse (Kreuzung): fuer die Vorschau
    kuerzung: f64,
    /// Abzweig: Schnitt davor/dahinter
    schnitt: (f64, f64),
    vorschau: Option<KreuzungsVorschau>,
}

/// unterwegs gekreuzte Strasse
#[derive(Clone, Debug)]
enum Querung {
    /// eigene Strasse: wird dort geteilt
    Kante { pos: DVec3 },
    /// vorhandene Strasse: wird aufgeschnitten
    Karte { ab: Abzweig, vor: f64, nach: f64 },
}

/// Kreuzungsobjekt an einem Knoten des eigenen Netzes
struct NetzObjekt {
    signatur: String,
    objekt: kreuzung::Objekt,
    hoehe: f64,
    gpu: Option<TileGpu>,
    /// Ampeln und Masten (gezeichnet)
    signal_gpu: Vec<TileGpu>,
    /// je Arm: Richtung, Vorfahrt, Ende (fuer Anzeige und Werkzeug)
    arme: Vec<(f64, kreuzung::Rolle, DVec3)>,
    vermutet: bool,
}

/// eine Kreuzung des Netzes fuer das Werkzeug "Kreuzungen"
#[derive(Clone, Debug)]
pub struct KreuzungsInfo {
    pub knoten: u32,
    pub pos: DVec3,
    /// je Arm: Richtung von der Kreuzung weg, Vorfahrt, Ende des Arms
    pub arme: Vec<(f64, kreuzung::Rolle, DVec3)>,
    pub ampel: bool,
    /// Vorfahrt nur vermutet (keine Regel gesetzt)
    pub vermutet: bool,
    pub umlauf: Option<f64>,
    pub phasen: Vec<usize>,
}

/// Beginn des Zugs
#[derive(Clone, Debug)]
struct Start {
    ende: Ende,
    /// Richtung, in der es weitergehen muss (letzte Kante des Zugs)
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
    start: Aufgeloest,
    ziel: Aufgeloest,
    /// gekreuzte Strassen: (Meter auf der neuen Kante, was dort geschieht)
    querungen: Vec<(f64, Querung)>,
    stuecke: Vec<netz::Stueck>,
    /// Kreisverkehr: Mitte und Halbmesser
    pub kreisel: Option<(DVec3, f64)>,
    /// endet an etwas Vorhandenem (Knoten, Strassenende, Strasse)
    pub angeschlossen: bool,
    /// geplante Kreuzungen (Start, Ziel, unterwegs) fuer die Markierung
    pub kreuzungen: Vec<KreuzungsVorschau>,
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
    /// Netz vorher und wie viele Schritte des Aendern-Werkzeugs (aufgeschnittene Strassen) dazugehoeren
    undo: Vec<(Netz, usize)>,
    redo: Vec<(Netz, usize)>,
    pub aenderungen: usize,
    /// beim Anschluss an eine vorhandene Strasse deren Querschnitt uebernehmen, wenn der gewaehlte nicht passt
    pub uebernehmen: bool,
    spuren_cache: HashMap<String, Vec<(f32, u8)>>,
    /// Stelle mitten auf einer Strasse unter der Maus (vor dem Start: Markierung "hier zweigt es ab")
    pub zeiger: Option<KreuzungsVorschau>,
    /// Kreuzungsobjekte an den Knoten des Netzes (je Knoten), und schon erzeugte (je Signatur der Arme)
    objekte: HashMap<u32, NetzObjekt>,
    objekt_cache: HashMap<String, kreuzung::Objekt>,
    /// Ordner der Kreuzungsobjekte (vom Aendern-Werkzeug: Sitzungsordner) und sein Name
    kreuzungs_ordner: Option<(PathBuf, String)>,
    /// letzter Fehler beim Erzeugen eines Kreuzungsobjekts
    pub kreuzung_fehler: Option<String>,
    /// Querschnitt fuer Kreisverkehre (Einbahn)
    pub kreisel_sli: Option<String>,
}

impl Default for Strassenbau {
    fn default() -> Self {
        Strassenbau { netz: Netz::default(), sli: None, modus: Modus::Kurve, hoehe: 0.0, start: None, plan: None,
                      vorschau: vec![], gezeichnet: HashMap::new(), undo: vec![], redo: vec![], aenderungen: 0,
                      uebernehmen: true, spuren_cache: HashMap::new(), zeiger: None, objekte: HashMap::new(),
                      objekt_cache: HashMap::new(), kreuzungs_ordner: None, kreuzung_fehler: None, kreisel_sli: None }
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
    /// auf einer eigenen, dann einer vorhandenen Strasse (Kreuzung), sonst der Bodenpunkt
    fn punkt(&self, v: &Viewer, boden: DVec3, fang: f64, ans: &Anschluesse, ae: Option<&mut Aendern>) -> Ende {
        // auf einem Kreisverkehr: an ihn anschliessen (neue Zufahrt)
        if let Some(k) = self.netz.knoten.iter().find(|k| k.kreisel.is_some_and(|kr| (k.pos.truncate() - boden.truncate()).length() < kr.arm_abstand())) {
            return Ende::Knoten(k.id);
        }
        if let Some(k) = self.netz.knoten_bei(boden.truncate(), fang) {
            return Ende::Knoten(k);
        }
        if let Some(i) = ans.bei(boden.truncate(), fang) {
            return Ende::Anschluss(ans.liste[i].clone());
        }
        if let Some((kante, s, pos, richtung)) = self.netz.kante_bei(boden.truncate()) {
            // nahe einem Ende dieser Strasse (freies Ende oder schon eine Kreuzung): dort einrasten
            if let Some(e) = self.netz.kante(kante) {
                for k in [e.a, e.b] {
                    let Some(kn) = self.netz.knoten(k) else { continue };
                    let n = self.netz.an(k).len() + kn.kartenarme.len();
                    let nah = (kn.pos.truncate() - boden.truncate()).length() < if n == 1 { ENDE_FANG } else { self.netz.kuerzung(e.id, k) + 2.0 };
                    if nah {
                        return Ende::Knoten(k);
                    }
                }
            }
            return Ende::Kante(Kantenpunkt { kante, s, pos, richtung });
        }
        if let Some(a) = ae {
            if let Some(k) = a.kreuzungsobjekt_bei(v, boden.truncate()) {
                return Ende::Vorhanden(k);
            }
            if let Some(ab) = a.abzweig_bei(v, boden.truncate()) {
                // nahe einem freien Ende derselben Strasse: dort anschliessen statt eine Kreuzung zu bauen
                let kette = a.kette_ids(ab.spline_id, 40.0);
                let ende = ans.liste.iter()
                    .filter(|x| x.frei && kette.contains(&x.spline_id) && (x.pos.truncate() - boden.truncate()).length() < ENDE_FANG)
                    .min_by(|x, y| (x.pos - boden).length().total_cmp(&(y.pos - boden).length()));
                if let Some(x) = ende {
                    return Ende::Anschluss(x.clone());
                }
                return Ende::Abzweig(ab);
            }
        }
        Ende::Frei(boden + DVec3::Z * self.hoehe)
    }

    /// halbe Breite eines Querschnitts (aussen, groessere Seite)
    fn halb(&self, v: &Viewer, sli: &str) -> f64 {
        v.spline_lanes(sli).map(|(_, (l, r))| l.max(r) as f64).unwrap_or(5.0)
    }

    fn ende_pos(&self, e: &Ende) -> DVec3 {
        match e {
            Ende::Knoten(k) => self.netz.knoten(*k).map(|x| x.pos).unwrap_or_default(),
            Ende::Anschluss(a) => a.pos,
            Ende::Abzweig(ab) => ab.pos,
            Ende::Kante(kp) => kp.pos,
            Ende::Vorhanden(k) => k.pos,
            Ende::Frei(p) => *p,
        }
    }

    /// ein Ende fuer die Planung aufloesen. `gegen`: das andere Ende (dorthin zeigt ein neuer Kreuzungsarm),
    /// `start`: Anfang der Kante (sonst Ziel), `fest`: Richtung, in der es weitergehen muss
    fn aufloesen(&self, v: &Viewer, ae: Option<&Aendern>, ende: &Ende, gegen: DVec3, start: bool, fest: Option<f64>, sli: &str) -> Result<Aufgeloest, String> {
        let um = |h: f64| if start { h } else { (h + 180.0).rem_euclid(360.0) };
        let halb_neu = self.halb(v, sli);
        let mut a = Aufgeloest { ende: ende.clone(), pos: self.ende_pos(ende), richtung: None, steigung: None, kuerzung: 0.0, schnitt: (0.0, 0.0), vorschau: None };
        match ende {
            Ende::Knoten(k) if self.netz.knoten(*k).is_some_and(|x| x.kreisel.is_some()) => {
                // Zufahrt eines Kreisverkehrs: radial, endet ausserhalb des Rings
                let kn = self.netz.knoten(*k).unwrap();
                let h = richtung(kn.pos.truncate(), gegen.truncate());
                if let Some(x) = self.netz.arme(*k).iter().find(|x| norm180(x.richtung - h).abs() < KREISEL_ARM_WINKEL) {
                    return Err(format!("zu nah an einer anderen Zufahrt ({:.0} Grad, mindestens {KREISEL_ARM_WINKEL:.0})", norm180(x.richtung - h).abs()));
                }
                a.richtung = Some(um(h));
                a.steigung = Some(0.0);
                a.kuerzung = kn.kreisel.unwrap().arm_abstand();
            }
            Ende::Knoten(k) => {
                if self.netz.knoten(*k).is_some_and(|x| x.anschluss.is_some()) && !self.netz.an(*k).is_empty() {
                    return Err("hier schliesst eine Strasse an eine vorhandene an - eine Kreuzung an dieser Stelle geht nicht".into());
                }
                a.richtung = fest.or_else(|| self.netz.weiter_richtung(*k).map(um));
            }
            Ende::Anschluss(x) => {
                a.richtung = Some(um(x.richtung));
                a.steigung = Some(if start { x.steigung } else { -x.steigung });
            }
            Ende::Abzweig(ab) => {
                let ae = ae.ok_or("Kreuzung: Aendern-Werkzeug nicht bereit")?;
                let wunsch = richtung(ab.pos.truncate(), gegen.truncate());
                let h0 = kreuzung::arm_richtung(ab.richtung, wunsch);
                let (vor, nach) = Aendern::schnitte(ab, &[(h0, halb_neu)]);
                let ab2 = ae.abzweig_einpassen(ab, vor, nach).map_err(|e| e.to_string())?;
                let h = kreuzung::arm_richtung(ab2.richtung, wunsch);
                let roh = [((ab2.richtung + 180.0).rem_euclid(360.0), ab2.halb), (ab2.richtung, ab2.halb), (h, halb_neu)];
                let d = netz::kuerzungen(&roh);
                a.pos = ab2.pos;
                a.richtung = Some(um(h));
                a.steigung = Some(0.0);
                a.kuerzung = d[2];
                a.schnitt = (d[0], d[1]);
                a.vorschau = Some(KreuzungsVorschau { mitte: ab2.pos, strasse: ab2.richtung, schnitt: d[0].max(d[1]),
                                                      arm: (ab2.pos.truncate() + netz::dir(h) * d[2]).extend(ab2.pos.z) });
                a.ende = Ende::Abzweig(ab2);
            }
            Ende::Kante(kp) => {
                let e = self.netz.kante(kp.kante).ok_or("Strasse nicht mehr da")?;
                let st = self.netz.lage(e);
                let laenge: f64 = st.iter().map(|s| s.laenge).sum();
                let halb_e = self.halb(v, &e.sli);
                let wunsch = richtung(kp.pos.truncate(), gegen.truncate());
                let masse = |hq: f64| {
                    let h = kreuzung::arm_richtung(hq, wunsch);
                    (h, netz::kuerzungen(&[((hq + 180.0).rem_euclid(360.0), halb_e), (hq, halb_e), (h, halb_neu)]))
                };
                let (_, d) = masse(kp.richtung);
                let lo = self.netz.kuerzung(e.id, e.a) + d[0] + 1.0;
                let hi = laenge - self.netz.kuerzung(e.id, e.b) - d[1] - 1.0;
                if lo > hi {
                    return Err(format!("das Strassenstueck ist zu kurz fuer eine Kreuzung (braucht {:.0} m)", laenge - hi + lo));
                }
                let s = kp.s.clamp(lo, hi);
                let (q, hq) = netz::punkt_auf(&st, s).ok_or("Strasse ohne Lage")?;
                let (h, d) = masse(hq);
                let mitte = q.extend(self.netz.hoehe_bei(e, s));
                a.pos = mitte;
                a.richtung = Some(um(h));
                a.steigung = Some(0.0);
                a.kuerzung = d[2];
                a.vorschau = Some(KreuzungsVorschau { mitte, strasse: hq, schnitt: d[0].max(d[1]), arm: (q + netz::dir(h) * d[2]).extend(mitte.z) });
                a.ende = Ende::Kante(Kantenpunkt { kante: kp.kante, s, pos: mitte, richtung: hq });
            }
            Ende::Vorhanden(k) => {
                let h = richtung(k.pos.truncate(), gegen.truncate());
                if let Some(x) = k.arme.iter().find(|x| norm180(x.richtung - h).abs() < kreuzung::MIN_WINKEL) {
                    return Err(format!("die neue Strasse liegt zu nah an einem vorhandenen Arm ({:.0} Grad, mindestens {:.0})",
                                       norm180(x.richtung - h).abs(), kreuzung::MIN_WINKEL));
                }
                let mut roh: Vec<(f64, f64)> = k.arme.iter().map(|x| (x.richtung, x.halb)).collect();
                roh.push((h, halb_neu));
                let d = netz::kuerzungen(&roh);
                // die vorhandenen Strassen enden, wo sie enden: reicht der Platz fuer den neuen Arm?
                for (x, dx) in k.arme.iter().zip(&d) {
                    let ist = (x.pos.truncate() - k.pos.truncate()).length();
                    // die Bordsteinecken duerfen enger werden (omsigen verkleinert die Radien): bis 2 m statt ECKENRAUM
                    if *dx - kreuzung::ECKENRAUM + 2.0 > ist + 0.5 {
                        return Err(format!("zu eng: eine vorhandene Strasse endet {ist:.0} m vor der Mitte, mit dem neuen Arm braeuchte sie {dx:.0} m"));
                    }
                }
                let dn = *d.last().unwrap();
                a.richtung = Some(um(h));
                a.steigung = Some(0.0);
                a.kuerzung = dn;
                a.vorschau = Some(KreuzungsVorschau { mitte: k.pos, strasse: k.arme[0].richtung, schnitt: 0.0,
                                                      arm: (k.pos.truncate() + netz::dir(h) * dn).extend(k.pos.z) });
            }
            Ende::Frei(_) => a.richtung = fest,
        }
        Ok(a)
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
        if a.objekt.is_some() {
            return spuren_passen(&neu, &a.spuren, true);
        }
        let alt = self.spuren(v, &a.sli);
        spuren_passen(&neu, &alt, a.am_ende != a.gespiegelt)
    }

    /// Strassen, die die geplante Kurve kreuzt (eigene und vorhandene, auf gleicher Hoehe), mit ihren Kreuzungen.
    /// `ohne`: Kanten, die an Start/Ziel haengen (dort entsteht schon eine Kreuzung); `frei_a`/`frei_b`: so weit von
    /// Anfang/Ende zaehlt nichts (die Kreuzungen dort). -> (Querungen, Markierungen) oder warum es nicht geht
    #[allow(clippy::too_many_arguments)]
    fn querungen_suchen(&self, v: &Viewer, ae: Option<&mut Aendern>, el: &[netz::Element], halb_neu: f64, ohne: &[u32],
                        frei_a: f64, frei_b: f64, ring: bool) -> Result<(Vec<(f64, Querung)>, Vec<KreuzungsVorschau>, Vec<(f64, f64)>), String> {
        let punkte = netz::abtasten(el, 1.0);
        let laenge = punkte.last().map(|p| p.1).unwrap_or(0.0);
        let mut gefunden: Vec<(f64, Querung, f64, f64)> = Vec::new(); // (s, Querung, Richtung der anderen, ihre halbe Breite)
        for e in &self.netz.kanten {
            if ohne.contains(&e.id) {
                continue;
            }
            let q = self.netz.punkte(e, 1.0);
            for w in punkte.windows(2) {
                for u in q.windows(2) {
                    let Some((t, r)) = kreuzung::schnitt_strecken(w[0].0.truncate(), w[1].0.truncate(), u[0].0.truncate(), u[1].0.truncate()) else { continue };
                    let z1 = w[0].0.z + (w[1].0.z - w[0].0.z) * t;
                    let z2 = u[0].0.z + (u[1].0.z - u[0].0.z) * r;
                    if (z1 - z2).abs() > 3.0 {
                        continue; // Bruecke/Tunnel
                    }
                    let s = w[0].1 + (w[1].1 - w[0].1) * t;
                    let s2 = u[0].1 + (u[1].1 - u[0].1) * r;
                    let (p2, h2) = netz::punkt_auf(&self.netz.lage(e), s2).unwrap();
                    let laenge2 = q.last().map(|x| x.1).unwrap_or(0.0);
                    if s2 < self.netz.kuerzung(e.id, e.a) + 3.0 || s2 > laenge2 - self.netz.kuerzung(e.id, e.b) - 3.0 {
                        return Err("die Strasse kreuzt zu nah an einem Knoten - dort direkt anschliessen".into());
                    }
                    gefunden.push((s, Querung::Kante { pos: p2.extend(z2) }, h2, self.halb(v, &e.sli)));
                }
            }
        }
        if let Some(ae) = ae {
            for (s, ab) in ae.kreuzungen_mit(v, &punkte) {
                if s < frei_a.max(2.0) || s > laenge - frei_b.max(2.0) {
                    continue;
                }
                let h = netz::punkt_auf(&el.iter().map(|x| x.stueck).collect::<Vec<_>>(), s).map(|x| x.1).unwrap_or(0.0);
                let (vor, nach) = Aendern::schnitte(&ab, &[(h, halb_neu), ((h + 180.0).rem_euclid(360.0), halb_neu)]);
                // die Stelle steht fest: passt die Kreuzung dort nicht (Ende/Kreuzung der vorhandenen Strasse zu nah), geht es
                // nicht - am Kreisverkehr faellt das Innere weg, dort zaehlt nur der Platz aussen (beim Bauen geprueft)
                if !ring {
                    match ae.abzweig_einpassen(&ab, vor, nach) {
                        Ok(x) if (x.pos - ab.pos).length() < 0.5 => {}
                        _ => return Err("die Strasse kreuzt eine vorhandene zu nah an deren Ende oder an einer Kreuzung".into()),
                    }
                }
                let (hq, halb) = (ab.richtung, ab.halb);
                gefunden.push((s, Querung::Karte { ab, vor, nach }, hq, halb));
            }
        }
        gefunden.sort_by(|a, b| a.0.total_cmp(&b.0));
        gefunden.dedup_by(|a, b| (a.0 - b.0).abs() < 1.0);
        let stuecke: Vec<netz::Stueck> = el.iter().map(|x| x.stueck).collect();
        let mut querungen = Vec::new();
        let mut marken = Vec::new();
        let mut abschnitte = Vec::new(); // je Querung: Kuerzung der neuen Strasse davor/dahinter
        for (s, q, h2, halb2) in gefunden {
            let (p, h) = netz::punkt_auf(&stuecke, s).unwrap();
            let winkel = norm180(h2 - h).abs();
            if winkel.min(180.0 - winkel) < 30.0 {
                return Err("die Strassen kreuzen sich zu flach (unter 30 Grad)".into());
            }
            let d = netz::kuerzungen(&[(h, halb_neu), ((h + 180.0).rem_euclid(360.0), halb_neu), (h2, halb2), ((h2 + 180.0).rem_euclid(360.0), halb2)]);
            let z = match &q {
                Querung::Kante { pos } => pos.z,
                Querung::Karte { ab, .. } => ab.pos.z,
            };
            marken.push(KreuzungsVorschau { mitte: p.extend(z), strasse: h2, schnitt: d[2].max(d[3]), arm: (p + netz::dir(h) * d[0]).extend(z) });
            abschnitte.push((d[1], d[0]));
            querungen.push((s, q));
        }
        Ok((querungen, marken, abschnitte))
    }

    /// Klick: Start setzen bzw. geplante Kante bauen
    pub fn klick(&mut self, v: &mut Viewer, boden: DVec3, fang: f64, ans: &Anschluesse, mut ae: Option<&mut Aendern>) -> Option<String> {
        // ein Kreisverkehr braucht keinen Strassen-Querschnitt (er ist ein Objekt)
        if self.modus != Modus::Kreisel {
            self.sli.as_ref()?;
        }
        if let Some(a) = ae.as_deref() {
            self.kreuzungs_ordner = Some(a.kreuzungs_ordner());
        }
        if self.start.is_none() {
            if self.modus == Modus::Kreisel {
                self.start = Some(Start { ende: Ende::Frei(boden + DVec3::Z * self.hoehe), richtung: None });
                return Some("Kreisverkehr: Mitte gesetzt - die Maus bestimmt die Groesse, Klick baut ihn".into());
            }
            let ende = self.punkt(v, boden, fang, ans, ae.as_deref_mut());
            let mut meldung = "Start gesetzt - Klick setzt den naechsten Punkt, Rechtsklick/Esc beendet".to_string();
            match &ende {
                Ende::Knoten(k) => {
                    if self.netz.knoten(*k).is_some_and(|x| x.anschluss.is_some()) && !self.netz.an(*k).is_empty() {
                        return Some("hier schliesst die Strasse an eine vorhandene an - eine Kreuzung an dieser Stelle geht nicht; etwas weiter auf der Strasse abzweigen".into());
                    }
                    if self.netz.an(*k).len() >= 2 {
                        meldung = "Abzweig vom Knoten: hier entsteht eine Kreuzung".into();
                    }
                }
                Ende::Anschluss(a) => {
                    // Querschnitt der vorhandenen Strasse uebernehmen, wenn der gewaehlte nicht passt
                    let sli = self.sli.clone().unwrap();
                    if a.objekt.is_some() && !self.passt(v, &sli, a) {
                        meldung = format!("am offenen Arm einer Kreuzung angeschlossen - Spuren passen nicht zum Querschnitt (Arm: {})",
                                          a.spuren.iter().map(|x| format!("{:+.2} m", x.0)).collect::<Vec<_>>().join(", "));
                    } else if a.objekt.is_some() {
                        meldung = "am offenen Arm einer Kreuzung angeschlossen".into();
                    } else if self.uebernehmen && !self.passt(v, &sli, a) {
                        self.sli = Some(a.sli.clone());
                        meldung = format!("an vorhandene Strasse angeschlossen - Querschnitt uebernommen: {}", a.sli.rsplit('\\').next().unwrap_or(""));
                    } else {
                        meldung = "an vorhandene Strasse angeschlossen".into();
                    }
                }
                Ende::Abzweig(_) | Ende::Kante(_) => meldung = "Abzweig: hier entsteht eine Kreuzung - Klick setzt das Ende des ersten Stuecks".into(),
                Ende::Vorhanden(k) => meldung = format!("an die vorhandene Kreuzung ({} Arme): sie wird durch eine eigene mit dem neuen Arm ersetzt", k.arme.len()),
                Ende::Frei(_) => {}
            }
            self.start = Some(Start { ende, richtung: None });
            self.zeiger = None;
            return Some(meldung);
        }
        let sli = if self.modus == Modus::Kreisel { self.kreisel_sli.clone().unwrap_or_default() } else { self.sli.clone()? };
        let p = self.plan.clone()?;
        if let Some(b) = p.blockiert.as_ref() {
            return Some(format!("geht nicht: {b}"));
        }
        if p.laenge < 1.0 {
            return None;
        }
        self.undo.push((self.netz.clone(), 0));
        self.redo.clear();
        self.aenderungen += 1;
        if !sli.is_empty() {
            self.netz.breiten.insert(sli.clone(), self.halb(v, &sli));
        }
        let erg = match p.kreisel {
            Some((mitte, r)) => self.kreisel_bauen(v, ae.as_deref_mut(), mitte, r, &sli, &p),
            None => self.plan_bauen(v, ae.as_deref_mut(), &p, &sli),
        };
        let (b, schritte) = match erg {
            Ok(x) => x,
            Err((e, schritte)) => {
                self.zuruecknehmen(v, ae.as_deref_mut(), schritte);
                return Some(format!("geht nicht: {e}"));
            }
        };
        if let Some(u) = self.undo.last_mut() {
            u.1 = schritte;
        }
        // an Kreuzungen gekuerzt: kein Stueck darf ganz verschwinden
        if self.netz.kanten.iter().any(|e| self.netz.elemente(e).is_empty()) {
            self.zuruecknehmen(v, ae.as_deref_mut(), schritte);
            return Some("zu kurz: zwischen den Kreuzungen bliebe kein Strassenstueck - die Punkte weiter auseinander setzen".into());
        }
        let vorher = self.objekte.len();
        self.zeichnen_alle(v);
        let neue_kreuzungen = self.objekte.len().saturating_sub(vorher);
        self.vorschau_weg(v);
        if p.kreisel.is_some() {
            self.start = None;
            let zufahrten = self.netz.knoten.last().map(|k| k.kartenarme.len()).unwrap_or(0);
            let _ = neue_kreuzungen;
            // weiter geht es mit Zufahrten: Strassen auf den Kreisverkehr ziehen
            if self.sli.is_some() {
                self.modus = Modus::Kurve;
            }
            return Some(format!("Kreisverkehr gebaut: Durchmesser {:.0} m{}{} - Zufahrten: Strasse auf den Kreisverkehr ziehen{}", 2.0 * p.kreisel.unwrap().1,
                                if zufahrten > 0 { format!(", {zufahrten} vorhandene Strasse(n) als Zufahrt") } else { String::new() },
                                self.kreuzung_fehler.as_ref().map(|f| format!(" - Objekt: {f}")).unwrap_or_default(),
                                if self.sli.is_none() { " (erst einen Querschnitt waehlen)" } else { "" }));
        }
        // weiter vom neuen Ende; endete der Zug an etwas Vorhandenem, ist er fertig
        let b_pos = self.netz.knoten(b).map(|k| k.pos).unwrap_or(p.b);
        self.start = if p.angeschlossen { None } else { Some(Start { ende: Ende::Knoten(b), richtung: Some(p.hb) }) };
        let _ = b_pos;
        Some(format!("Strasse gebaut: {:.1} m{}{}{}", p.laenge, if p.angeschlossen { ", angeschlossen" } else { "" },
                     if neue_kreuzungen > 0 { format!(", {neue_kreuzungen} Kreuzung(en)") } else { String::new() },
                     p.warnung.as_ref().map(|w| format!(" - {w}")).unwrap_or_default()))
    }

    /// Bau scheiterte: Netz und aufgeschnittene Strassen zurueck
    fn zuruecknehmen(&mut self, v: &mut Viewer, mut ae: Option<&mut Aendern>, schritte: usize) {
        if let Some((n, _)) = self.undo.pop() {
            self.netz = n;
        }
        for _ in 0..schritte {
            if let Some(a) = ae.as_deref_mut() {
                let _ = a.rueckgaengig(v);
            }
        }
        self.zeichnen_alle(v);
    }

    /// Knoten fuer ein Ende anlegen (Strassen teilen/aufschneiden) -> (Knoten, Schritte des Aendern-Werkzeugs)
    fn knoten_fuer(&mut self, v: &mut Viewer, ae: Option<&mut Aendern>, a: &Aufgeloest) -> Result<(u32, usize), String> {
        Ok(match &a.ende {
            Ende::Knoten(k) => (*k, 0),
            Ende::Anschluss(x) => (self.netz.anschluss_neu(x.pos, x.richtung, x.steigung), 0),
            Ende::Abzweig(ab) => {
                let ae = ae.ok_or("Aendern-Werkzeug nicht bereit")?;
                let arme = ae.aufschneiden(v, ab, a.schnitt.0, a.schnitt.1).map_err(|e| format!("{e:#}"))?;
                (self.netz.knoten_mit_kartenarmen(ab.pos, arme), 1)
            }
            Ende::Kante(kp) => {
                // die Kante kann inzwischen geteilt sein (zwei Enden auf derselben Strasse): an der Stelle neu suchen
                let (id, s, _, _) = self.netz.kante_bei(kp.pos.truncate()).ok_or("Strasse nicht gefunden")?;
                (self.netz.kante_teilen(id, s).ok_or("Strasse laesst sich nicht teilen")?, 0)
            }
            Ende::Vorhanden(k) => {
                let ae = ae.ok_or("Aendern-Werkzeug nicht bereit")?;
                ae.objekt_entfernen(v, k.kachel, k.objekt).map_err(|e| format!("{e:#}"))?;
                (self.netz.knoten_mit_kartenarmen(k.pos, k.arme.clone()), 1)
            }
            Ende::Frei(p) => (self.netz.knoten_neu(*p), 0),
        })
    }

    /// Knoten fuer eine Querung unterwegs -> (Knoten, Schritte des Aendern-Werkzeugs)
    fn knoten_querung(&mut self, v: &mut Viewer, ae: Option<&mut Aendern>, q: &Querung) -> Result<(u32, usize), String> {
        Ok(match q {
            Querung::Kante { pos } => {
                let (id, s, _, _) = self.netz.kante_bei(pos.truncate()).ok_or("gekreuzte Strasse nicht gefunden")?;
                (self.netz.kante_teilen(id, s).ok_or("gekreuzte Strasse laesst sich nicht teilen")?, 0)
            }
            Querung::Karte { ab, vor, nach } => {
                let ae = ae.ok_or("Aendern-Werkzeug nicht bereit")?;
                let arme = ae.aufschneiden(v, ab, *vor, *nach).map_err(|e| format!("{e:#}"))?;
                (self.netz.knoten_mit_kartenarmen(ab.pos, arme), 1)
            }
        })
    }

    /// geplante Kante bauen: Enden, Querungen (Knoten mit Kreuzungen), Kanten dazwischen -> (Endknoten, Schritte)
    fn plan_bauen(&mut self, v: &mut Viewer, mut ae: Option<&mut Aendern>, p: &Plan, sli: &str) -> Result<(u32, usize), (String, usize)> {
        let mut schritte = 0;
        let (a, n) = self.knoten_fuer(v, ae.as_deref_mut(), &p.start).map_err(|e| (e, schritte))?;
        schritte += n;
        let mut knoten = vec![(a, p.ha)];
        for (s, q) in &p.querungen {
            let (k, n) = self.knoten_querung(v, ae.as_deref_mut(), q).map_err(|e| (e, schritte))?;
            schritte += n;
            let h = netz::punkt_auf(&p.stuecke, *s).map(|x| x.1).unwrap_or(p.ha);
            knoten.push((k, h));
        }
        let (b, n) = self.knoten_fuer(v, ae.as_deref_mut(), &p.ziel).map_err(|e| (e, schritte))?;
        schritte += n;
        knoten.push((b, p.hb));
        for w in knoten.windows(2) {
            self.netz.kante_neu(w[0].0, w[1].0, sli, w[0].1, w[1].1);
        }
        Ok((b, schritte))
    }

    /// Kreisverkehr bauen: vier Knoten auf dem Ring, Boegen gegen den Uhrzeigersinn (Rechtsverkehr), Querungen mit
    /// Strassen, die den Ring kreuzen
    fn kreisel_bauen(&mut self, v: &mut Viewer, mut ae: Option<&mut Aendern>, mitte: DVec3, r: f64, _sli: &str, p: &Plan) -> Result<(u32, usize), (String, usize)> {
        let kr = netz::Kreisel { r, breite: KREISEL_BREITE };
        let mut schritte = 0;
        if p.querungen.iter().any(|(_, q)| matches!(q, Querung::Kante { .. })) {
            return Err(("eigene Strassen im Kreisverkehr - erst loeschen und danach an den Kreisverkehr anschliessen".into(), schritte));
        }
        // vorhandene Strassen: das Innere faellt weg, jede endet an der Armlinie als Zufahrt
        let karte: Vec<(kreuzung::Abzweig, f64, f64)> = p.querungen.iter().filter_map(|(_, q)| match q {
            Querung::Karte { ab, .. } => Some((ab.clone(), 0.0, 0.0)),
            Querung::Kante { .. } => None,
        }).collect();
        let mut arme = Vec::new();
        if !karte.is_empty() {
            let a = ae.as_deref_mut().ok_or(("Aendern-Werkzeug nicht bereit".to_string(), schritte))?;
            arme = a.ring_ausschneiden(v, mitte, kr.arm_abstand(), &karte).map_err(|e| (format!("{e:#}"), schritte))?;
            schritte += 1;
        }
        for a in &arme {
            self.netz.breiten.insert(a.sli.clone(), a.halb);
        }
        let k = self.netz.knoten_mit_kartenarmen(mitte, arme);
        if let Some(x) = self.netz.knoten.iter_mut().find(|x| x.id == k) {
            x.kreisel = Some(kr);
        }
        Ok((k, schritte))
    }

    /// Zug beenden (Rechtsklick/Esc)
    pub fn beenden(&mut self, v: &mut Viewer) {
        self.start = None;
        self.plan = None;
        self.vorschau_weg(v);
    }

    /// Vorschau zur Maus berechnen und zeichnen
    pub fn maus(&mut self, v: &mut Viewer, boden: DVec3, fang: f64, ans: &Anschluesse, mut ae: Option<&mut Aendern>) {
        if let Some(a) = ae.as_deref() {
            self.kreuzungs_ordner = Some(a.kreuzungs_ordner());
        }
        if self.modus == Modus::Kreisel {
            self.zeiger = None;
            self.kreisel_vorschau(v, boden, ae);
            return;
        }
        let Some(sli) = self.sli.clone() else {
            self.vorschau_weg(v);
            return;
        };
        let ziel_ende = self.punkt(v, boden, fang, ans, ae.as_deref_mut());
        let Some(start) = self.start.clone() else {
            // noch kein Start: zeigen, wo eine Kreuzung entstuende
            self.zeiger = match &ziel_ende {
                Ende::Abzweig(_) | Ende::Kante(_) | Ende::Vorhanden(_) => {
                    let p = self.ende_pos(&ziel_ende);
                    let r = match &ziel_ende {
                        Ende::Abzweig(ab) => ab.richtung,
                        Ende::Kante(kp) => kp.richtung,
                        Ende::Vorhanden(k) => k.arme[0].richtung,
                        _ => 0.0,
                    };
                    let gegen = p + (netz::rechts(r) * 50.0).extend(0.0);
                    self.aufloesen(v, ae.as_deref(), &ziel_ende, gegen, true, None, &sli).ok().and_then(|a| a.vorschau)
                }
                _ => None,
            };
            self.vorschau_weg(v);
            return;
        };
        // dasselbe Ziel wie der Start: nichts
        let gleich = match (&ziel_ende, &start.ende) {
            (Ende::Knoten(x), Ende::Knoten(y)) => x == y,
            (Ende::Anschluss(x), Ende::Anschluss(y)) => x.spline_id == y.spline_id && x.am_ende == y.am_ende,
            (Ende::Abzweig(x), Ende::Abzweig(y)) => x.spline_id == y.spline_id && (x.s - y.s).abs() < 40.0,
            (Ende::Kante(x), Ende::Kante(y)) => x.kante == y.kante && (x.s - y.s).abs() < 40.0,
            (Ende::Vorhanden(x), Ende::Vorhanden(y)) => x.objekt == y.objekt,
            (Ende::Kante(x), Ende::Knoten(k)) => self.netz.kante(x.kante).is_some_and(|e| e.a == *k || e.b == *k) && (x.pos - self.ende_pos(&start.ende)).length() < 30.0,
            _ => false,
        };
        if gleich {
            self.vorschau_weg(v);
            self.plan = None;
            return;
        }
        let mut fehler: Option<String> = None;
        // Start und Ziel aufloesen (Kreuzungsarme zeigen zum anderen Ende)
        let roh_ziel = self.ende_pos(&ziel_ende);
        let sa = match self.aufloesen(v, ae.as_deref(), &start.ende, roh_ziel, true, start.richtung, &sli) {
            Ok(x) => x,
            Err(e) => {
                fehler = Some(e);
                Aufgeloest { ende: Ende::Frei(self.ende_pos(&start.ende)), pos: self.ende_pos(&start.ende), richtung: start.richtung, steigung: None, kuerzung: 0.0, schnitt: (0.0, 0.0), vorschau: None }
            }
        };
        let mut zb = match self.aufloesen(v, ae.as_deref(), &ziel_ende, sa.pos, false, None, &sli) {
            Ok(x) => x,
            Err(e) => {
                fehler = Some(e);
                Aufgeloest { ende: Ende::Frei(roh_ziel), pos: roh_ziel, richtung: None, steigung: None, kuerzung: 0.0, schnitt: (0.0, 0.0), vorschau: None }
            }
        };
        let a = sa.pos;
        let mut b = zb.pos;
        let (ha, hb, stuecke) = match (self.modus, sa.richtung, zb.richtung) {
            (_, Some(t), Some(hz)) => (t, hz, verbinden(a.truncate(), t, b.truncate(), hz)),
            (Modus::Gerade, Some(t), None) => {
                // geradeaus weiter: Ziel auf die Verlaengerung projizieren
                let d = netz::dir(t);
                let l = (b.truncate() - a.truncate()).dot(d).max(1.0);
                let q = a.truncate() + d * l;
                b = DVec3::new(q.x, q.y, v.terrain_height(q.x, q.y).map(|z| z + self.hoehe).unwrap_or(b.z));
                if let Ende::Frei(_) = zb.ende {
                    zb.ende = Ende::Frei(b);
                    zb.pos = b;
                }
                (t, t, verbinden(a.truncate(), t, q, t))
            }
            (_, Some(t), None) => {
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
        let mut warnung = match &zb.ende {
            Ende::Anschluss(x) if !self.passt(v, &sli, x) => Some(if x.objekt.is_some() {
                format!("Spuren passen nicht zum Arm der Kreuzung (dort {})", x.spuren.iter().map(|s| format!("{:+.2} m", s.0)).collect::<Vec<_>>().join(", "))
            } else {
                "Spuren passen nicht zur vorhandenen Strasse".to_string()
            }),
            _ => None,
        };
        // Steigung an den Enden: vorgegeben (vorhandene Strasse, Kreuzung eben), an eigenen Knoten die der Kante dort
        let sehne = (b.z - a.z) / laenge.max(1.0);
        let ga = sa.steigung.unwrap_or_else(|| match &sa.ende {
            Ende::Knoten(k) => self.steigung_aus(*k, ha),
            _ => sehne,
        });
        let gb = zb.steigung.unwrap_or(sehne);
        let el = netz::mit_hoehe(&stuecke, a.z, ga, b.z, gb);
        // Kanten, die an Start/Ziel haengen, kreuzen nicht (dort ist schon eine Kreuzung)
        let mut ohne: Vec<u32> = Vec::new();
        for e in [&sa.ende, &zb.ende] {
            match e {
                Ende::Knoten(k) => ohne.extend(self.netz.an(*k).iter().map(|x| x.id)),
                Ende::Kante(kp) => ohne.push(kp.kante),
                _ => {}
            }
        }
        let halb_neu = self.halb(v, &sli);
        let (querungen, marken, abschnitte) = match self.querungen_suchen(v, ae.as_deref_mut(), &el, halb_neu, &ohne, sa.kuerzung + 1.0, zb.kuerzung + 1.0, false) {
            Ok(x) => x,
            Err(e) => {
                fehler.get_or_insert(e);
                (vec![], vec![], vec![])
            }
        };
        // quer durch einen Kreisverkehr (der nicht Start oder Ziel ist) geht es nicht
        let enden: Vec<u32> = [&sa.ende, &zb.ende].into_iter().filter_map(|e| if let Ende::Knoten(k) = e { Some(*k) } else { None }).collect();
        for kn in self.netz.knoten.iter().filter(|k| k.kreisel.is_some() && !enden.contains(&k.id)) {
            let ra = kn.kreisel.unwrap().aussen() + 2.0;
            if netz::abtasten(&el, 2.0).iter().any(|(q, _)| (q.truncate() - kn.pos.truncate()).length() < ra) && fehler.is_none() {
                fehler = Some("die Strasse fuehrt durch einen Kreisverkehr - auf ihm enden lassen (sie wird eine Zufahrt)".into());
            }
        }
        // Kreuzungen hintereinander brauchen Platz: zwischen zwei Kreuzungen muss ein Stueck Strasse bleiben
        let mut stellen = vec![(0.0, 0.0, sa.kuerzung)];
        for ((s, _), (vor, nach)) in querungen.iter().zip(&abschnitte) {
            stellen.push((*s, *vor, *nach));
        }
        stellen.push((laenge, zb.kuerzung, 0.0));
        if stellen.windows(2).any(|w| w[1].0 - w[0].0 < w[0].2 + w[1].1 + 1.0) && fehler.is_none() {
            fehler = Some("Kreuzungen zu dicht hintereinander - zwischen ihnen bliebe kein Strassenstueck".into());
        }
        if let Some(f) = &fehler {
            warnung = Some(format!("geht nicht: {f}"));
        }
        let mut kreuzungen: Vec<KreuzungsVorschau> = [sa.vorschau.clone(), zb.vorschau.clone()].into_iter().flatten().collect();
        kreuzungen.extend(marken);
        let angeschlossen = !matches!(zb.ende, Ende::Frei(_));
        let (kuerz_a, kuerz_b) = (sa.kuerzung, zb.kuerzung);
        self.plan = Some(Plan { a, b, ha, hb, start: sa, ziel: zb, querungen, stuecke: stuecke.clone(), kreisel: None, angeschlossen,
                                kreuzungen, blockiert: fehler, warnung, laenge, min_radius, steigung });
        // die alte Vorschau erst nach dem Zeichnen der neuen wegnehmen: so bleibt der Spline-Typ (seine Texturen) auf
        // der Grafikkarte, statt bei jeder Mausbewegung neu geladen zu werden
        let alt = std::mem::take(&mut self.vorschau);
        // an Kreuzungen beginnt/endet die Vorschau am Arm
        let st = if kuerz_a > 0.0 || kuerz_b > 0.0 { netz::schneiden(&stuecke, kuerz_a, laenge - kuerz_b) } else { stuecke };
        let el = netz::mit_hoehe(&st, a.z, ga, b.z, gb);
        let mut cum = 0.0;
        for e in &el {
            if let Some(g) = v.add_spline(&sli, &e.kurve(1, cum)) {
                self.vorschau.push(g);
            }
            cum += e.stueck.laenge;
        }
        for g in alt {
            v.remove_object(g);
        }
    }

    /// Vorschau des Kreisverkehrs: Mitte (erster Klick), Halbmesser bis zur Maus
    fn kreisel_vorschau(&mut self, v: &mut Viewer, boden: DVec3, ae: Option<&mut Aendern>) {
        let alt = std::mem::take(&mut self.vorschau);
        for g in alt {
            v.remove_object(g);
        }
        let Some(Start { ende: Ende::Frei(mitte), .. }) = self.start.clone() else {
            self.plan = None;
            return;
        };
        let sli = self.kreisel_sli.clone();
        let r = (boden.truncate() - mitte.truncate()).length().clamp(KREISEL_MIN, KREISEL_MAX);
        // vier Viertelboegen gegen den Uhrzeigersinn ab Norden
        let mut stuecke = Vec::new();
        for i in 0..4 {
            let phi = -90.0 * i as f64;
            let a = mitte.truncate() + netz::dir(phi) * r;
            let b = mitte.truncate() + netz::dir(phi - 90.0) * r;
            stuecke.extend(verbinden(a, (phi - 90.0).rem_euclid(360.0), b, (phi - 180.0).rem_euclid(360.0)));
        }
        let laenge: f64 = stuecke.iter().map(|s| s.laenge).sum();
        let el = netz::mit_hoehe(&stuecke, mitte.z, 0.0, mitte.z, 0.0);
        let halb = sli.as_deref().map(|x| self.halb(v, x)).unwrap_or(4.0);
        // vorhandene Strassen werden an der Armlinie (ausserhalb des Rings) abgeschnitten: dort die Querungen suchen
        let ra = netz::Kreisel { r, breite: KREISEL_BREITE }.arm_abstand();
        let mut kreis = Vec::new();
        for i in 0..4 {
            let phi = -90.0 * i as f64;
            let a = mitte.truncate() + netz::dir(phi) * ra;
            let b = mitte.truncate() + netz::dir(phi - 90.0) * ra;
            kreis.extend(verbinden(a, (phi - 90.0).rem_euclid(360.0), b, (phi - 180.0).rem_euclid(360.0)));
        }
        let el_arme = netz::mit_hoehe(&kreis, mitte.z, 0.0, mitte.z, 0.0);
        let (querungen, marken, blockiert) = match self.querungen_suchen(v, ae, &el_arme, halb, &[], 0.0, 0.0, true) {
            Ok((q, m, _)) => (q, m, None),
            Err(e) => (vec![], vec![], Some(e)),
        };
        let leer = Aufgeloest { ende: Ende::Frei(mitte), pos: mitte, richtung: None, steigung: None, kuerzung: 0.0, schnitt: (0.0, 0.0), vorschau: None };
        self.plan = Some(Plan { a: mitte, b: mitte, ha: 0.0, hb: 0.0, start: leer.clone(), ziel: leer, querungen, stuecke, kreisel: Some((mitte, r)),
                                angeschlossen: false, kreuzungen: marken, warnung: blockiert.as_ref().map(|b| format!("geht nicht: {b}")), blockiert,
                                laenge, min_radius: r, steigung: 0.0 });
        let mut cum = 0.0;
        for e in el.iter().filter(|_| sli.is_some()) {
            if let Some(g) = v.add_spline(sli.as_deref().unwrap(), &e.kurve(1, cum)) {
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
        // die alten Stuecke erst nach den neuen wegnehmen (Spline-Typen bleiben geladen)
        let alt: Vec<TileGpu> = self.gezeichnet.drain().flat_map(|(_, gs)| gs).collect();
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
        for g in alt {
            v.remove_object(g);
        }
        self.kreuzungen_aktualisieren(v);
    }

    /// Kreuzungsobjekte des Netzes: fuer jeden Knoten mit 3 und mehr Armen eines (von omsigen, je Anordnung der Arme
    /// einmal erzeugt), ueberzaehlige weg
    fn kreuzungen_aktualisieren(&mut self, v: &mut Viewer) {
        self.kreuzung_fehler = None;
        let knoten: Vec<u32> = self.netz.knoten.iter().map(|k| k.id).filter(|k| self.netz.ist_kreuzung(*k)).collect();
        let weg: Vec<u32> = self.objekte.keys().copied().filter(|k| !knoten.contains(k)).collect();
        for k in weg {
            if let Some(o) = self.objekte.remove(&k) {
                if let Some(g) = o.gpu {
                    v.remove_object(g);
                }
                for g in o.signal_gpu {
                    v.remove_object(g);
                }
            }
        }
        for k in knoten {
            let Some(hoehe) = self.netz.knoten(k).map(|x| x.pos.z) else { continue };
            let mut arme = Vec::new();
            let mut info = Vec::new(); // (halbe Breite, vorhandene Strasse, Ring)
            for arm in self.netz.arme(k) {
                if let Some(ka) = &arm.karte {
                    arme.push(kreuzung::Arm { pos: ka.pos, richtung: ka.richtung, sli: ka.sli.clone(), weg: ka.weg, rolle: kreuzung::Rolle::Gleich, gesperrt: false });
                    info.push((ka.halb, true, false));
                    continue;
                }
                let Some(e) = self.netz.kante(arm.kante) else { continue };
                // Lage und Richtung am Ende der gekuerzten Kante: Platte und Abbiegespuren schliessen genau dort an
                let Some((pos, richtung)) = self.netz.arm_ende(&arm) else { continue };
                arme.push(kreuzung::Arm { pos, richtung, sli: e.sli.clone(), weg: arm.weg, rolle: kreuzung::Rolle::Gleich, gesperrt: e.einbahn == netz::Einbahn::Gesperrt });
                info.push((self.netz.breiten.get(&e.sli).copied().unwrap_or(5.0), false, e.ring));
            }
            let kreisel = self.netz.knoten(k).and_then(|x| x.kreisel).map(|kr| (self.netz.knoten(k).unwrap().pos.truncate(), kr));
            if arme.len() < 3 && kreisel.is_none() {
                continue;
            }
            let regel = self.netz.knoten(k).and_then(|x| x.regel.clone());
            let rollen = match &regel {
                Some(r) => rollen_aus_regel(&arme, r),
                None => rollen_vermuten(&arme, &info),
            };
            for (a, r) in arme.iter_mut().zip(rollen) {
                a.rolle = r;
            }
            let ampel = regel.as_ref().is_some_and(|r| r.ampel) && kreisel.is_none();
            // Spurpfeile/Verbinder: die gewuenschten Abbiegespuren ueber die Armrichtungen den Armen zuordnen
            let verbindungen: Option<Vec<[usize; 4]>> = self.netz.knoten(k).and_then(|x| x.spurwahl.clone()).filter(|_| kreisel.is_none()).map(|w| {
                let arm = |h: f64| arme.iter().enumerate().map(|(i, a)| (norm180(a.richtung - h).abs(), i)).filter(|x| x.0 < 20.0)
                    .min_by(|a, b| a.0.total_cmp(&b.0)).map(|x| x.1);
                w.iter().filter_map(|x| Some([arm(x.von)?, x.von_spur, arm(x.nach)?, x.nach_spur])).collect()
            });
            let mut signatur: String = arme.iter().map(|a| format!("{:.2},{:.2},{:.2},{:.3},{},{},{:?};", a.pos.x, a.pos.y, a.pos.z, a.richtung, a.sli, a.weg, a.rolle)).chain(arme.iter().map(|a| if a.gesperrt { "x".to_string() } else { String::new() })).collect();
            if ampel {
                signatur.push_str("Ampel");
            }
            if let Some(vb) = &verbindungen {
                signatur.push_str(&format!("Spuren {vb:?}"));
            }
            if let Some((m, kr)) = kreisel {
                signatur.push_str(&format!("Kreisel {:.2},{:.2},{:.2},{:.2}", m.x, m.y, kr.r, kr.breite));
            }
            if self.objekte.get(&k).is_some_and(|o| o.signatur == signatur) {
                continue;
            }
            if let Some(o) = self.objekte.remove(&k) {
                if let Some(g) = o.gpu {
                    v.remove_object(g);
                }
                for g in o.signal_gpu {
                    v.remove_object(g);
                }
            }
            let objekt = match self.objekt_cache.get(&signatur) {
                Some(o) => o.clone(),
                None => {
                    let Some((ordner, tag)) = self.kreuzungs_ordner.clone() else {
                        self.kreuzung_fehler = Some("Kreuzungsobjekt: kein Sitzungsordner".into());
                        continue;
                    };
                    let rel_ordner = format!("Sceneryobjects\\Aschaffenburg_KI\\{tag}");
                    crate::protokoll::aktion(&format!("Kreuzung (Knoten {k}): omsigen erzeugt das Objekt"));
                    let erg = kreuzung::erzeugen_mit(&v.root, &ordner, &rel_ordner, &kreuzung::freier_name(&ordner), &arme, ampel, kreisel, verbindungen.as_deref());
                    crate::protokoll::aktion("");
                    match erg {
                        Ok(o) => {
                            log::info!("Kreuzung (Knoten {k}, {} Arme, Vorfahrt {:?}): {} mit {} Abbiegespuren", arme.len(),
                                       arme.iter().map(|a| a.rolle).collect::<Vec<_>>(), o.rel, o.spuren);
                            omsi_cfg::content_changed();
                            self.objekt_cache.insert(signatur.clone(), o.clone());
                            o
                        }
                        Err(e) => {
                            log::warn!("Kreuzung (Knoten {k}): {e:#}");
                            self.kreuzung_fehler = Some(format!("{e:#}"));
                            continue;
                        }
                    }
                }
            };
            let gpu = v.add_object(&objekt.rel, objekt.ursprung.extend(hoehe), 0.0);
            // Ampeln und Masten zeigen (die Signale am Ausleger haengen im Spiel am Mast; hier nur Signal und Mast)
            let mut signal_gpu = Vec::new();
            for g in &objekt.signale {
                let Some(p) = g.pos else { continue };
                let z = v.terrain_height(p.x, p.y).unwrap_or(hoehe) + g.hoehe;
                if let Some(t) = v.add_object(&g.datei, p.extend(z), g.rot) {
                    signal_gpu.push(t);
                }
            }
            let arme_info = arme.iter().map(|a| (a.richtung, a.rolle, a.pos)).collect();
            self.objekte.insert(k, NetzObjekt { signatur, objekt, hoehe, gpu, signal_gpu, arme: arme_info, vermutet: regel.is_none() });
        }
    }

    /// Kreuzungsobjekte des Netzes fuers Speichern: (.sco, Lage, Vorfahrtregeln, Ampeln)
    pub fn gesetzte_kreuzungen(&self) -> Vec<kreuzung::Gesetzt> {
        self.objekte.values().map(|o| kreuzung::Gesetzt { rel: o.objekt.rel.clone(), pos: o.objekt.ursprung.extend(o.hoehe),
                                                         rules: o.objekt.rules.clone(), no_cars: o.objekt.no_cars.clone(), signale: o.objekt.signale.clone() }).collect()
    }

    // ------------------------------------------------------------ Werkzeug "Kreuzungen": Vorfahrt und Ampel

    /// alle Kreuzungen des Netzes (mit ihren Armen, wie zuletzt erzeugt)
    pub fn kreuzungen(&self) -> Vec<KreuzungsInfo> {
        let mut out: Vec<KreuzungsInfo> = self.objekte.iter().filter_map(|(k, o)| {
            let kn = self.netz.knoten(*k).filter(|x| x.kreisel.is_none())?;
            Some(KreuzungsInfo { knoten: *k, pos: kn.pos, arme: o.arme.clone(), ampel: !o.objekt.signale.is_empty(), vermutet: o.vermutet,
                                 umlauf: o.objekt.umlauf, phasen: o.objekt.phasen.clone() })
        }).collect();
        out.sort_by_key(|x| x.knoten);
        out
    }

    /// Werkzeug "Kreuzungen", Spuren: Armrichtungen und das gebaute Objekt einer Kreuzung des Netzes
    pub fn spur_objekt(&self, k: u32) -> Option<(Vec<f64>, kreuzung::Objekt, f64)> {
        let o = self.objekte.get(&k).filter(|_| self.netz.knoten(k).is_some_and(|x| x.kreisel.is_none()))?;
        Some((o.arme.iter().map(|a| a.0).collect(), o.objekt.clone(), o.hoehe))
    }

    /// Abbiegespuren einer Kreuzung des Netzes vorgeben ([Arm rein, Spur, Arm raus, Spur]; Arme in der Reihenfolge
    /// von spur_objekt) - ein Rueckgaengig-Schritt
    pub fn spuren_setzen(&mut self, v: &mut Viewer, k: u32, liste: &[[usize; 4]]) -> String {
        let Some((richtungen, ..)) = self.spur_objekt(k) else { return "keine Kreuzung".into() };
        let wahl: Vec<netz::SpurVerbindung> = liste.iter().filter(|x| x[0] < richtungen.len() && x[2] < richtungen.len())
            .map(|x| netz::SpurVerbindung { von: richtungen[x[0]], von_spur: x[1], nach: richtungen[x[2]], nach_spur: x[3] }).collect();
        self.merken();
        if let Some(kn) = self.netz.knoten.iter_mut().find(|x| x.id == k) {
            kn.spurwahl = Some(wahl);
        }
        self.zeichnen_alle(v);
        match &self.kreuzung_fehler {
            Some(f) => format!("Abbiegespuren nicht gebaut: {f}"),
            None => format!("Kreuzung mit {} Abbiegespur(en) neu gebaut", self.spur_objekt(k).map(|x| x.1.verbindungen.len()).unwrap_or(0)),
        }
    }

    /// Spurpfeile/Verbinder zuruecksetzen (wieder wie omsigen sie bildet)
    pub fn spuren_zuruecksetzen(&mut self, v: &mut Viewer, k: u32) -> String {
        self.merken();
        if let Some(kn) = self.netz.knoten.iter_mut().find(|x| x.id == k) {
            kn.spurwahl = None;
        }
        self.zeichnen_alle(v);
        "Abbiegespuren wieder wie vorgeschlagen".into()
    }

    /// Kreuzung des Netzes unter dem Bodenpunkt (innerhalb ihrer Arme)
    pub fn kreuzung_bei(&self, p: DVec2) -> Option<u32> {
        self.kreuzungen().into_iter().filter_map(|k| {
            let r = k.arme.iter().map(|a| (a.2.truncate() - k.pos.truncate()).length()).fold(0.0, f64::max);
            let d = (k.pos.truncate() - p).length();
            (d <= r.max(6.0)).then_some((d, k.knoten))
        }).min_by(|a, b| a.0.total_cmp(&b.0)).map(|x| x.1)
    }

    /// Vorfahrt/Ampel einer Kreuzung setzen (None: wieder vermuten) - ein Rueckgaengig-Schritt
    pub fn regel_setzen(&mut self, v: &mut Viewer, k: u32, regel: Option<netz::Regel>) -> String {
        if self.netz.knoten(k).is_none() {
            return String::new();
        }
        self.merken();
        if let Some(kn) = self.netz.knoten.iter_mut().find(|x| x.id == k) {
            kn.regel = regel;
        }
        self.zeichnen_alle(v);
        match self.objekte.get(&k) {
            Some(o) => format!("Kreuzung: {}{}", o.arme.iter().map(|a| match a.1 {
                kreuzung::Rolle::Haupt => "H", kreuzung::Rolle::Neben => "N", kreuzung::Rolle::Gleich => "R" }).collect::<Vec<_>>().join("-"),
                if o.objekt.signale.is_empty() { String::new() } else { format!(", Ampel (Umlauf {:.0} s)", o.objekt.umlauf.unwrap_or(0.0)) }),
            None => self.kreuzung_fehler.clone().map(|e| format!("Kreuzung nicht erzeugt: {e}")).unwrap_or_default(),
        }
    }

    /// vorhandene Kreuzung der Karte uebernehmen (wird durch eine eigene mit denselben Armen ersetzt), damit sich
    /// Vorfahrt und Ampel setzen lassen -> Knoten
    pub fn vorhandene_uebernehmen(&mut self, v: &mut Viewer, ae: &mut Aendern, k: &kreuzung::Vorhanden) -> Result<u32, String> {
        if k.arme.len() < 3 {
            return Err("die vorhandene Kreuzung hat weniger als 3 angeschlossene Strassen".into());
        }
        if let Some(a) = Some(&*ae) {
            self.kreuzungs_ordner = Some(a.kreuzungs_ordner());
        }
        ae.objekt_entfernen(v, k.kachel, k.objekt).map_err(|e| format!("{e:#}"))?;
        self.undo.push((self.netz.clone(), 1));
        self.redo.clear();
        self.aenderungen += 1;
        for a in &k.arme {
            self.netz.breiten.insert(a.sli.clone(), a.halb);
        }
        let id = self.netz.knoten_mit_kartenarmen(k.pos, k.arme.clone());
        self.zeichnen_alle(v);
        Ok(id)
    }

    /// Netz ersetzen (Werkzeug "Knoten": Knoten gezogen), ein Rueckgaengig-Schritt mit `schritte` Schritten des
    /// Aendern-Werkzeugs (mitgezogene Strassen der Karte)
    pub fn netz_setzen(&mut self, v: &mut Viewer, ae: &Aendern, netz: Netz, schritte: usize) {
        if self.kreuzungs_ordner.is_none() {
            self.kreuzungs_ordner = Some(ae.kreuzungs_ordner());
        }
        self.undo.push((std::mem::replace(&mut self.netz, netz), schritte));
        self.redo.clear();
        self.aenderungen += 1;
        self.zeichnen_alle(v);
    }

    fn merken(&mut self) {
        self.undo.push((self.netz.clone(), 0));
        self.redo.clear();
        self.aenderungen += 1;
    }

    /// letzten Schritt zuruecknehmen (mit den Strassen, die er aufgeschnitten hat - im Aendern-Werkzeug)
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
        // auf dem Ring: der ganze Kreisverkehr mit seinen Zufahrten
        if let Some(k) = self.netz.knoten.iter().find(|k| k.kreisel.is_some_and(|kr| (k.pos.truncate() - p).length() < kr.aussen())).map(|k| k.id) {
            self.merken();
            let an: Vec<u32> = self.netz.an(k).iter().map(|e| e.id).collect();
            for id in &an {
                self.netz.kante_loeschen(*id);
            }
            self.netz.knoten.retain(|x| x.id != k);
            self.zeichnen_alle(v);
            return Some(format!("Kreisverkehr geloescht{}", if an.is_empty() { String::new() } else { format!(" mit {} Zufahrt(en)", an.len()) }));
        }
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

    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    // ------------------------------------------------------------ eigene Strassen aendern (Werkzeug "Aendern")

    /// eigene Strasse (Kante, wie gezeichnet) unter dem Bodenpunkt
    pub fn kante_unter(&self, p: DVec2) -> Option<u32> {
        let mut best: Option<(f64, u32)> = None;
        for e in &self.netz.kanten {
            let w = self.netz.breiten.get(&e.sli).copied().unwrap_or(5.0);
            for el in self.netz.elemente(e) {
                let n = (el.stueck.laenge / 1.0).ceil().max(1.0) as usize;
                for i in 0..=n {
                    let (q, _) = el.stueck.bei(el.stueck.laenge * i as f64 / n as f64);
                    let d = (q - p).length();
                    if d <= w && best.map(|b| d < b.0).unwrap_or(true) {
                        best = Some((d, e.id));
                    }
                }
            }
        }
        best.map(|b| b.1)
    }

    /// ganze Kette eigener Strassen ueber Knoten mit genau zwei Kanten
    pub fn kette_eigen(&self, id: u32) -> Vec<u32> {
        let mut out = vec![id];
        let mut offen = vec![id];
        while let Some(k) = offen.pop() {
            let Some(e) = self.netz.kante(k) else { continue };
            for n in [e.a, e.b] {
                let an = self.netz.an(n);
                if an.len() == 2 && self.netz.knoten(n).is_some_and(|x| x.kartenarme.is_empty()) {
                    for x in an {
                        if !out.contains(&x.id) {
                            out.push(x.id);
                            offen.push(x.id);
                        }
                    }
                }
            }
        }
        out
    }

    /// Mittellinie (Punkte mit Richtung) und halbe Breite einer eigenen Strasse, wie gezeichnet
    pub fn kante_umriss(&self, id: u32) -> Option<(Vec<(DVec3, f64)>, f64)> {
        let e = self.netz.kante(id)?;
        let w = self.netz.breiten.get(&e.sli).copied().unwrap_or(5.0);
        let mut punkte = Vec::new();
        for el in self.netz.elemente(e) {
            let n = (el.stueck.laenge / 2.0).ceil().max(1.0) as usize;
            for i in 0..=n {
                let s = el.stueck.laenge * i as f64 / n as f64;
                let (q, h) = el.stueck.bei(s);
                punkte.push((q.extend(netz::element_z(&el, s)), h));
            }
        }
        Some((punkte, w))
    }

    /// eigene Strassen aendern (ein Rueckgaengig-Schritt) -> Meldung
    pub fn kanten_aendern(&mut self, v: &mut Viewer, ids: &[u32], art: &KantenAenderung) -> String {
        let ids: Vec<u32> = ids.iter().copied().filter(|id| self.netz.kante(*id).is_some()).collect();
        if ids.is_empty() {
            return String::new();
        }
        self.merken();
        let text = match art {
            KantenAenderung::Querschnitt(sli) => {
                self.netz.breiten.insert(sli.clone(), self.halb(v, sli));
                for e in self.netz.kanten.iter_mut().filter(|e| ids.contains(&e.id)) {
                    e.sli = sli.clone();
                }
                format!("{} Strassenstueck(e) auf {} umgestellt", ids.len(), sli.rsplit('\\').next().unwrap_or(""))
            }
            KantenAenderung::Einbahn(art) => {
                for e in self.netz.kanten.iter_mut().filter(|e| ids.contains(&e.id)) {
                    e.einbahn = *art;
                }
                format!("{} Strassenstueck(e): {}", ids.len(), art.text())
            }
            KantenAenderung::Umkehren => {
                for e in self.netz.kanten.iter_mut().filter(|e| ids.contains(&e.id)) {
                    let (ha, hb) = (e.ha, e.hb);
                    std::mem::swap(&mut e.a, &mut e.b);
                    e.ha = (hb + 180.0).rem_euclid(360.0);
                    e.hb = (ha + 180.0).rem_euclid(360.0);
                }
                format!("{} Strassenstueck(e) umgekehrt", ids.len())
            }
            KantenAenderung::Loeschen => {
                for id in &ids {
                    self.netz.kante_loeschen(*id);
                }
                let geheilt = self.heilen();
                format!("{} Strassenstueck(e) geloescht{}", ids.len(),
                        if geheilt > 0 { format!(", {geheilt} aufgeschnittene Strasse(n) wieder geschlossen") } else { String::new() })
            }
        };
        self.zeichnen_alle(v);
        text
    }

    /// Knoten, an denen nur noch zwei Enden einer aufgeschnittenen vorhandenen Strasse liegen (die Kreuzung ist weg):
    /// die Luecke mit einem eigenen Stueck in deren Querschnitt schliessen -> Anzahl
    fn heilen(&mut self) -> usize {
        let knoten: Vec<u32> = self.netz.knoten.iter().filter(|k| k.kartenarme.len() == 2 && self.netz.an(k.id).is_empty()).map(|k| k.id).collect();
        for k in &knoten {
            let arme = self.netz.knoten(*k).unwrap().kartenarme.clone();
            self.netz.knoten.retain(|x| x.id != *k);
            // in Richtung des vorhandenen Splines: von dem Ende, an dem er in die Kreuzung lief, zum anderen
            let (a, b) = if arme[0].weg { (&arme[1], &arme[0]) } else { (&arme[0], &arme[1]) };
            let ha = (a.richtung + 180.0).rem_euclid(360.0);
            let na = self.netz.anschluss_neu(a.pos, ha, 0.0);
            let nb = self.netz.anschluss_neu(b.pos, (b.richtung + 180.0).rem_euclid(360.0), 0.0);
            self.netz.breiten.insert(a.sli.clone(), a.halb);
            self.netz.kante_neu(na, nb, &a.sli, ha, b.richtung.rem_euclid(360.0));
        }
        knoten.len()
    }

    /// Hilfslinien: Knoten des Netzes (fuer die Markierung im Bild)
    pub fn kreisel(&self) -> Vec<(DVec3, netz::Kreisel)> {
        self.netz.knoten.iter().filter_map(|k| k.kreisel.map(|kr| (k.pos, kr))).collect()
    }

    pub fn knoten_punkte(&self) -> Vec<(u32, DVec3, bool)> {
        self.netz.knoten.iter().map(|k| (k.id, k.pos, self.netz.weiter_richtung(k.id).is_some())).collect()
    }
}

/// Aenderung an eigenen Strassen (Werkzeug "Aendern")
#[derive(Clone, Debug)]
pub enum KantenAenderung {
    Querschnitt(String),
    Umkehren,
    Einbahn(netz::Einbahn),
    Loeschen,
}

/// Vorfahrt aus der Regel des Nutzers: jeder Arm bekommt die Rolle des gespeicherten Arms mit der naechsten Richtung
/// (bis 30 Grad); neue Arme warten, wenn es eine Hauptstrasse gibt, sonst rechts vor links
pub fn rollen_aus_regel(arme: &[kreuzung::Arm], regel: &netz::Regel) -> Vec<kreuzung::Rolle> {
    use kreuzung::Rolle;
    let haupt = regel.rollen.iter().any(|r| r.1 == Rolle::Haupt);
    arme.iter().map(|a| {
        regel.rollen.iter().map(|r| (norm180(r.0 - a.richtung).abs(), r.1)).filter(|x| x.0 < 30.0)
            .min_by(|x, y| x.0.total_cmp(&y.0)).map(|x| x.1)
            .unwrap_or(if haupt { Rolle::Neben } else { Rolle::Gleich })
    }).collect()
}

/// Vorfahrt an einer Kreuzung vermuten (spaeter per Klick aenderbar). `info` je Arm: (halbe Breite, vorhandene
/// Strasse, Ring eines Kreisverkehrs).
/// - Kreisverkehr: der Ring hat Vorfahrt, die Zufahrten warten
/// - sonst die durchgehende Strasse (zwei fast gegenueberliegende Arme): bevorzugt eine vorhandene Strasse, dann
///   gleicher Querschnitt, dann die breitere; sind zwei Paare gleichwertig: rechts vor links (alle gleich)
pub fn rollen_vermuten(arme: &[kreuzung::Arm], info: &[(f64, bool, bool)]) -> Vec<kreuzung::Rolle> {
    use kreuzung::Rolle;
    let n = arme.len();
    if info.iter().any(|i| i.2) && info.iter().any(|i| !i.2) {
        return info.iter().map(|i| if i.2 { Rolle::Haupt } else { Rolle::Neben }).collect();
    }
    let mut paare: Vec<(f64, usize, usize)> = Vec::new();
    for i in 0..n {
        for j in i + 1..n {
            if norm180(arme[i].richtung - arme[j].richtung).abs() > 150.0 {
                let wert = if info[i].1 && info[j].1 { 100.0 } else { 0.0 }
                    + if arme[i].sli == arme[j].sli { 10.0 } else { 0.0 }
                    + info[i].0 + info[j].0;
                paare.push((wert, i, j));
            }
        }
    }
    paare.sort_by(|a, b| b.0.total_cmp(&a.0));
    match paare.as_slice() {
        [] => vec![Rolle::Gleich; n],
        [x, y, ..] if (x.0 - y.0).abs() < 0.01 => vec![Rolle::Gleich; n],
        [x, ..] => (0..n).map(|k| if k == x.1 || k == x.2 { Rolle::Haupt } else { Rolle::Neben }).collect(),
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
        // die erste Strasse als Einbahn (Fahrtrichtung a -> b): die Gegenspur bekommt [rule] no_cars
        let erste = s.netz.kanten[0].id;
        s.kanten_aendern(&mut v, &[erste], &KantenAenderung::Einbahn(netz::Einbahn::Vor));
        let einbahn_splines = s.netz.elemente(s.netz.kante(erste).unwrap()).len();
        let ziel = crate::speichern::alles_speichern(&v, &Bearbeiten::neu(Werkzeug::Strasse), &s.netz, "Grundorf", "Grundorf_strasse", &test_root).unwrap();
        // mit openOMSIs Kachel-Parser lesen
        let mut neue: Vec<(omsi_geometry::SplineCurve, i64, i64, i64)> = Vec::new();
        let mut gesperrt = 0;
        for e in std::fs::read_dir(&ziel).unwrap().flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            let Some(rest) = n.strip_prefix("tile_").and_then(|r| r.strip_suffix(".map")) else { continue };
            let mut it = rest.split('_').map(|x| x.parse::<i32>().unwrap());
            let (tx, ty) = (it.next().unwrap(), it.next().unwrap());
            let t = omsi_map::Tile::load(&e.path()).unwrap();
            for sp in t.splines.iter().filter(|sp| sp.id >= start) {
                let o = glam::DVec2::new(tx as f64 * 300.0, ty as f64 * 300.0);
                if sp.rules.iter().any(|r| r.kind.eq_ignore_ascii_case("no_cars")) {
                    gesperrt += 1;
                }
                assert!(sp.delta_h.is_some(), "spline_h erwartet");
                neue.push((omsi_geometry::SplineCurve::from_map(sp, o), sp.id, sp.prev_id, sp.next_id));
            }
        }
        assert_eq!(neue.len(), erwartet, "Anzahl Splines");
        assert_eq!(gesperrt, einbahn_splines, "Einbahn: jeder Spline der ersten Strasse mit no_cars");
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
