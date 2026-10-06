//! Werkzeug "Strasse bauen" (wie in Transport Fever 2) auf dem Netz-Kern (netz.rs).
//!
//! Klick setzt den Start (rastet an Knoten ein; an einem freien Strassenende geht es tangential weiter), die Maus
//! zieht die Vorschau als echte OMSI-Strasse, Klick setzt den naechsten Knoten, Rechtsklick/Esc beendet den Zug.
//! Gerade: geradeaus (am Strassenende in dessen Richtung). Kurve: Bogen, der tangential anschliesst; trifft der
//! Bogen ein freies Strassenende, wird er tangential eingefaedelt (Bogenpaar). Bild auf/ab: Hoehe des naechsten
//! Punkts ueber dem Gelaende.

use crate::netz::{self, bogen_durch, norm180, richtung, verbinden, Netz};
use glam::{DVec2, DVec3};
use openomsi_game::viewer::{TileGpu, Viewer};
use std::collections::HashMap;
use std::path::Path;

pub const MIN_RADIUS: f64 = 10.0; // engere Boegen: Warnung
pub const MAX_STEIGUNG: f64 = 12.0; // Prozent

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
}

/// alle .sli unter Splines mit mindestens einer Fahrspur (Hintergrund)
pub fn querschnitte(root: &Path) -> Vec<Querschnitt> {
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
            out.push(Querschnitt { rel, name, ordner, vor, zurueck, gehwege, breite: hi - lo });
        }
    }
    out.sort_by_key(|q| (q.ordner.to_lowercase(), q.name.to_lowercase()));
    out
}

/// Beginn des Zugs
#[derive(Clone, Copy, Debug)]
struct Start {
    knoten: Option<u32>,
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
    pub laenge: f64,
    pub min_radius: f64,
    pub steigung: f64,
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
    undo: Vec<Netz>,
    redo: Vec<Netz>,
    pub aenderungen: usize,
}

impl Default for Strassenbau {
    fn default() -> Self {
        Strassenbau { netz: Netz::default(), sli: None, modus: Modus::Kurve, hoehe: 0.0, start: None, plan: None,
                      vorschau: vec![], gezeichnet: HashMap::new(), undo: vec![], redo: vec![], aenderungen: 0 }
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

    /// Punkt fuer Start/Ziel: eigener Knoten in `fang` Metern oder der Bodenpunkt (+ Hoehe)
    fn punkt(&self, boden: DVec3, fang: f64) -> (Option<u32>, DVec3) {
        match self.netz.knoten_bei(boden.truncate(), fang) {
            Some(k) => (Some(k), self.netz.knoten(k).unwrap().pos),
            None => (None, boden + DVec3::Z * self.hoehe),
        }
    }

    /// Klick: Start setzen bzw. geplante Kante bauen
    pub fn klick(&mut self, v: &mut Viewer, boden: DVec3, fang: f64) -> Option<String> {
        let sli = self.sli.clone()?;
        match self.start {
            None => {
                let (k, pos) = self.punkt(boden, fang);
                let richtung = k.and_then(|k| self.netz.weiter_richtung(k));
                self.start = Some(Start { knoten: k, pos, richtung });
                Some("Start gesetzt - Klick setzt den naechsten Punkt, Rechtsklick/Esc beendet".into())
            }
            Some(s) => {
                let p = self.plan.clone()?;
                if p.laenge < 1.0 {
                    return None;
                }
                self.merken();
                let a = s.knoten.unwrap_or_else(|| self.netz.knoten_neu(s.pos));
                let b = p.ziel.unwrap_or_else(|| self.netz.knoten_neu(p.b));
                self.netz.kante_neu(a, b, &sli, p.ha, p.hb);
                self.zeichnen_alle(v);
                // weiter vom neuen Ende; endete der Zug auf einem vorhandenen Knoten, ist er fertig
                self.start = if p.ziel.is_some() { None } else { Some(Start { knoten: Some(b), pos: p.b, richtung: Some(p.hb) }) };
                self.vorschau_weg(v);
                Some(format!("Strasse gebaut: {:.1} m{}", p.laenge, if p.ziel.is_some() { ", angeschlossen" } else { "" }))
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
    pub fn maus(&mut self, v: &mut Viewer, boden: DVec3, fang: f64) {
        let (Some(s), Some(sli)) = (self.start, self.sli.clone()) else {
            self.vorschau_weg(v);
            return;
        };
        let (ziel, mut b) = self.punkt(boden, fang);
        if ziel == s.knoten && ziel.is_some() {
            self.vorschau_weg(v);
            self.plan = None;
            return;
        }
        let a = s.pos;
        // Ankunftsrichtung, wenn das Ziel ein freies Strassenende ist (dort tangential einfaedeln)
        let ankunft = ziel.and_then(|k| self.netz.weiter_richtung(k)).map(|h| (h + 180.0).rem_euclid(360.0));
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
        let min_radius = stuecke.iter().filter(|s| s.radius != 0.0).map(|s| s.radius.abs()).fold(f64::INFINITY, f64::min);
        let steigung = if laenge > 0.0 { (b.z - a.z) / laenge * 100.0 } else { 0.0 };
        self.plan = Some(Plan { a, b, ha, hb, ziel, laenge, min_radius, steigung });
        // zeichnen: Hoehe glatt, Anschluss an die Steigung am Start
        self.vorschau_weg(v);
        let ga = s.knoten.map(|k| self.steigung_aus(k, ha)).unwrap_or((b.z - a.z) / laenge.max(1.0));
        let el = netz::mit_hoehe(&stuecke, a.z, ga, b.z, (b.z - a.z) / laenge.max(1.0));
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
    }

    fn merken(&mut self) {
        self.undo.push(self.netz.clone());
        self.redo.clear();
        self.aenderungen += 1;
    }

    pub fn rueckgaengig(&mut self, v: &mut Viewer) -> bool {
        let Some(n) = self.undo.pop() else { return false };
        self.redo.push(std::mem::replace(&mut self.netz, n));
        self.beenden(v);
        self.zeichnen_alle(v);
        self.aenderungen += 1;
        true
    }

    pub fn wiederholen(&mut self, v: &mut Viewer) -> bool {
        let Some(n) = self.redo.pop() else { return false };
        self.undo.push(std::mem::replace(&mut self.netz, n));
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
        use crate::bearbeiten::{tests::grundorf, Bearbeiten, Werkzeug};
        let root = Path::new(crate::bearbeiten::tests::OMSI);
        let mut v = grundorf();
        let mut s = Strassenbau::neu(Some("Splines\\Marcel\\str_2spur_10m_Grunewaldstr.sli".into()), Modus::Gerade);
        let boden = |v: &Viewer, x: f64, y: f64| DVec3::new(x, y, v.terrain_height(x, y).unwrap_or(0.0));
        // Gerade nach Norden, dann tangential eine Rechtskurve, 3 m hoeher
        let a = boden(&v, 120.0, 100.0);
        s.klick(&mut v, a, 3.0).unwrap();
        let b = boden(&v, 120.0, 180.0);
        s.maus(&mut v, b, 3.0);
        s.klick(&mut v, b, 3.0).unwrap();
        s.modus = Modus::Kurve;
        s.hoehe = 3.0;
        let c = boden(&v, 190.0, 240.0);
        s.maus(&mut v, c, 3.0);
        let plan = s.plan.clone().unwrap();
        assert!(plan.min_radius > MIN_RADIUS, "Radius {}", plan.min_radius);
        s.klick(&mut v, c, 3.0).unwrap();
        s.beenden(&mut v);
        assert_eq!((s.netz.knoten.len(), s.netz.kanten.len()), (3, 2));
        // Rueckgaengig/Wiederholen
        assert!(s.rueckgaengig(&mut v));
        assert_eq!(s.netz.kanten.len(), 1);
        assert!(s.wiederholen(&mut v));
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
}
