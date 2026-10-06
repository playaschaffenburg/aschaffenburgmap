//! Anschluesse an vorhandene Strassen der Karte: die Enden der Splines in den geladenen Kacheln (mit openOMSIs
//! Kachel-Parser gelesen) und ob sie frei sind - laut openOMSIs Spurnetz, das Spuren so verknuepft wie das Spiel
//! (auch mit den Pfaden der Kreuzungsobjekte). Nur an freien Enden laesst sich ohne Kreuzung weiterbauen.

use glam::{DVec2, DVec3};
use openomsi_game::viewer::Viewer;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct Anschluss {
    pub spline_id: i64,
    /// am Ende (true) oder am Anfang des Splines
    pub am_ende: bool,
    pub pos: DVec3,
    /// Richtung, in der eine neue Strasse den Punkt verlassen muss (weg vom vorhandenen Spline)
    pub richtung: f64,
    /// Steigung (Verhaeltnis) in dieser Richtung
    pub steigung: f64,
    /// .sli der vorhandenen Strasse (relativ zum OMSI-Ordner)
    pub sli: String,
    pub gespiegelt: bool,
    pub frei: bool,
}

/// Spline-Enden je Kachel (aus der Datei gelesen, zwischengespeichert)
#[derive(Default)]
pub struct Anschluesse {
    kacheln: HashMap<(i32, i32), Vec<Anschluss>>,
    /// Stand: geladene Kacheln und Spuren, aus denen `liste` berechnet wurde
    stand: (Vec<(i32, i32)>, usize),
    pub liste: Vec<Anschluss>,
}

fn kachel_lesen(v: &Viewer, tx: i32, ty: i32) -> Vec<Anschluss> {
    let Some(datei) = v.tile_file(tx, ty) else { return vec![] };
    let Ok(t) = omsi_map::Tile::load(&datei) else { return vec![] };
    let ts = omsi_map::tile_size();
    let o = DVec2::new(tx as f64 * ts, ty as f64 * ts);
    let mut out = Vec::new();
    for sp in t.splines.iter().filter(|s| !s.deleted && !s.file.trim().is_empty() && s.length > 0.0) {
        let k = omsi_geometry::SplineCurve::from_map(sp, o);
        let l = k.length;
        out.push(Anschluss {
            spline_id: sp.id, am_ende: false, pos: k.start, richtung: (k.heading_deg + 180.0).rem_euclid(360.0),
            steigung: -k.slope_at(0.0), sli: sp.file.trim().to_string(), gespiegelt: sp.mirror, frei: false,
        });
        out.push(Anschluss {
            spline_id: sp.id, am_ende: true, pos: k.end_point(), richtung: k.heading_at(l).rem_euclid(360.0),
            steigung: k.slope_at(l), sli: sp.file.trim().to_string(), gespiegelt: sp.mirror, frei: false,
        });
    }
    out
}

impl Anschluesse {
    /// neu berechnen, wenn sich geladene Kacheln oder Spurnetz geaendert haben -> true bei Aenderung
    pub fn aktualisieren(&mut self, v: &Viewer) -> bool {
        let mut kacheln = v.loaded_tile_keys();
        kacheln.sort();
        let stand = (kacheln.clone(), v.lanes.lanes.len());
        if stand == self.stand {
            return false;
        }
        self.kacheln.retain(|k, _| kacheln.contains(k));
        for k in &kacheln {
            if !self.kacheln.contains_key(k) {
                let e = kachel_lesen(v, k.0, k.1);
                self.kacheln.insert(*k, e);
            }
        }
        self.liste.clear();
        let karte: std::collections::HashSet<(i32, i32)> = v.map_tiles().into_iter().collect();
        let geladen: std::collections::HashSet<(i32, i32)> = kacheln.iter().copied().collect();
        let ts = omsi_map::tile_size();
        for e in self.kacheln.values() {
            for a in e {
                // nur Strassen: Splines ohne Fahrspur (Gehwege, Schienen, Zaeune) zaehlen nicht
                if let Some(frei) = v.spline_end_free(a.spline_id, a.am_ende) {
                    // am Rand des geladenen Bereichs koennte es in der Nachbarkachel weitergehen
                    let (tx, ty) = ((a.pos.x / ts).floor() as i32, (a.pos.y / ts).floor() as i32);
                    let rand = (-1..=1).any(|dx| (-1..=1).any(|dy| karte.contains(&(tx + dx, ty + dy)) && !geladen.contains(&(tx + dx, ty + dy))));
                    self.liste.push(Anschluss { frei: frei && !rand, ..a.clone() });
                }
            }
        }
        self.stand = stand;
        true
    }

    /// Kacheldateien wurden geaendert: alles neu lesen
    pub fn vergessen(&mut self) {
        self.kacheln.clear();
        self.stand = (vec![], 0);
    }

    /// naechster freier Anschluss in r Metern (waagerecht)
    pub fn bei(&self, p: DVec2, r: f64) -> Option<usize> {
        self.liste
            .iter()
            .enumerate()
            .filter(|(_, a)| a.frei)
            .map(|(i, a)| ((a.pos.truncate() - p).length(), i))
            .filter(|(d, _)| *d <= r)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|x| x.1)
    }
}

/// Passen die Fahrspuren zweier Querschnitte an einem Anschluss zusammen? `gleich`: beide in derselben Richtung
/// verlegt (die neue Strasse setzt den vorhandenen Spline fort), sonst gegenlaeufig. Spuren: (Querlage, Richtung
/// 0 vor / 1 zurueck / 2 beide).
pub fn spuren_passen(neu: &[(f32, u8)], alt: &[(f32, u8)], gleich: bool) -> bool {
    let wende = |(x, d): (f32, u8)| (-x, match d { 0 => 1, 1 => 0, d => d });
    let alt: Vec<(f32, u8)> = alt.iter().map(|&s| if gleich { s } else { wende(s) }).collect();
    neu.len() == alt.len() && neu.iter().all(|a| alt.iter().any(|b| (a.0 - b.0).abs() < 0.15 && a.1 == b.1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spuren_vergleichen() {
        let zwei = [(-1.75, 1), (1.75, 0)];
        assert!(spuren_passen(&zwei, &zwei, true));
        assert!(spuren_passen(&zwei, &zwei, false)); // symmetrisch: gegenlaeufig passt auch
        let einbahn = [(0.0, 0)];
        assert!(spuren_passen(&einbahn, &einbahn, true));
        assert!(!spuren_passen(&einbahn, &einbahn, false)); // Einbahn gegen die Richtung
        assert!(!spuren_passen(&zwei, &[(-2.0, 1), (2.0, 0)], true));
    }

    #[test]
    #[ignore]
    fn freie_enden_in_grundorf() {
        let _sperre = crate::bearbeiten::tests::sperre();
        let v = crate::bearbeiten::tests::grundorf();
        let mut a = Anschluesse::default();
        assert!(a.aktualisieren(&v));
        let frei = a.liste.iter().filter(|x| x.frei).count();
        let belegt = a.liste.len() - frei;
        println!("Grundorf: {} Strassen-Enden, {frei} frei, {belegt} angeschlossen", a.liste.len());
        assert!(belegt > frei, "die meisten Enden sind angeschlossen");
        assert!(a.liste.len() > 20);
    }
}
