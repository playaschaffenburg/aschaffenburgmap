//! Hilfsansicht (Taste H) wie "Show paths" und unsichtbare Objekte im nEditor: die Pfade der geladenen Kacheln
//! (Fahrspuren, Gehwege, Gleise; unsichtbare Strassen eigens) als Linien mit Fahrtrichtung, und die Objekte, die das
//! Spiel nicht zeichnet - `[onlyeditor]`-Objekte mit Modell werden als Hilfsobjekt gezeigt, solche ohne Modell
//! (Schallquellen, Marker, unsichtbare Kreuzungen) bekommen eine Markierung.

use glam::DVec3;
use openomsi_game::viewer::{LaneKind, TileGpu, Viewer};
use std::collections::HashMap;

#[derive(Default)]
pub struct Hilfsansicht {
    pub an: bool,
    /// gezeigte Modelle unsichtbarer Objekte (je Objekt-ID, mit Lage und Richtung beim Zeichnen)
    gezeigt: HashMap<i64, (Option<TileGpu>, DVec3, f64)>,
    /// unsichtbare Objekte der geladenen Kacheln: Lage, Richtung, Name, mit Modell
    pub objekte: Vec<(DVec3, f64, String, bool)>,
}

/// ein Pfad fuer die Anzeige: Punkte, Farbe (RGB), unsichtbare Strasse
pub struct Pfad<'a> {
    pub punkte: &'a [DVec3],
    pub farbe: [u8; 3],
    pub unsichtbar: bool,
}

impl Hilfsansicht {
    /// unsichtbare Objekte abgleichen (jedes Bild: das Objekte-Werkzeug verschiebt sie; neu gezeichnet wird nur, was
    /// sich bewegt hat)
    pub fn aktualisieren(&mut self, v: &mut Viewer) {
        if !self.an {
            if !self.gezeigt.is_empty() {
                self.leeren(v);
            }
            return;
        }
        let liste = v.hidden_objects();
        let ids: std::collections::HashSet<i64> = liste.iter().map(|x| x.0).collect();
        let weg: Vec<i64> = self.gezeigt.keys().copied().filter(|id| !ids.contains(id)).collect();
        for id in weg {
            if let Some((Some(g), _, _)) = self.gezeigt.remove(&id) {
                v.remove_object(g);
            }
        }
        self.objekte.clear();
        for (id, pos, richtung, sco, modell) in liste {
            let rel = relativ(&v.root, &sco);
            let bewegt = self.gezeigt.get(&id).is_some_and(|(_, p, h)| (*p - pos).length() > 1e-4 || (h - richtung).abs() > 1e-4);
            if bewegt {
                if let Some((Some(g), _, _)) = self.gezeigt.remove(&id) {
                    v.remove_object(g);
                }
            }
            if modell && !self.gezeigt.contains_key(&id) {
                let g = v.add_object(&rel, pos, richtung);
                self.gezeigt.insert(id, (g, pos, richtung));
            }
            let name = rel.rsplit('\\').next().unwrap_or(&rel).to_string();
            self.objekte.push((pos, richtung, name, modell));
        }
    }

    /// alle gezeigten Modelle wegnehmen (Ansicht aus, Kartenwechsel)
    pub fn leeren(&mut self, v: &mut Viewer) {
        for (_, (g, _, _)) in self.gezeigt.drain() {
            if let Some(g) = g {
                v.remove_object(g);
            }
        }
        self.objekte.clear();
    }
}

/// Pfade der geladenen Kacheln im Umkreis r um p
pub fn pfade(v: &Viewer, p: DVec3, r: f64) -> Vec<Pfad<'_>> {
    let r2 = r * r;
    v.lanes.lanes.iter().filter(|l| l.points.len() >= 2).filter(|l| {
        let n = l.points.len();
        [l.points[0], l.points[n / 2], l.points[n - 1]].iter().any(|q| (q.truncate() - p.truncate()).length_squared() < r2)
    }).map(|l| {
        let farbe = if l.kind == LaneKind::Street && (l.no_cars || l.density <= 0.0) { [230, 60, 60] } else { match (l.kind, l.invisible) {
            (LaneKind::Street, true) => [235, 80, 235],
            (LaneKind::Street, false) => if l.name.to_ascii_lowercase().ends_with(".sli") { [70, 170, 255] } else { [255, 210, 60] },
            (LaneKind::Sidewalk, _) => [90, 230, 110],
            (LaneKind::Rail, _) => [255, 140, 40],
            (LaneKind::Air, _) => [200, 200, 200],
        } };
        Pfad { punkte: &l.points, farbe, unsichtbar: l.invisible }
    }).collect()
}

/// .sco-Pfad relativ zum OMSI-Ordner, mit Backslashes (wie in den Kacheln)
fn relativ(root: &std::path::Path, p: &std::path::Path) -> String {
    p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('/', "\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Grundorf: unsichtbare Objekte werden gemeldet und mit Modell gezeigt; Pfade gibt es (OMSI_BILD: Bild beim
    /// ersten unsichtbaren Objekt)
    #[test]
    #[ignore]
    fn unsichtbare_objekte_und_pfade() {
        let _sperre = crate::bearbeiten::tests::sperre();
        let mut v = crate::bearbeiten::tests::grundorf();
        v.tiles_around(DVec3::new(150.0, 150.0, 0.0), 8).unwrap();
        let mut h = Hilfsansicht { an: true, ..Default::default() };
        h.aktualisieren(&mut v);
        println!("{} unsichtbare Objekte, {} mit Modell gezeigt", h.objekte.len(), h.gezeigt.values().filter(|g| g.0.is_some()).count());
        let mut namen: Vec<&String> = h.objekte.iter().map(|o| &o.2).collect();
        namen.sort();
        namen.dedup();
        println!("{namen:?}");
        assert!(!h.objekte.is_empty());
        assert!(pfade(&v, DVec3::new(150.0, 150.0, 0.0), 2000.0).len() > 100);
        if let (Some(b), Some(o)) = (std::env::var_os("OMSI_BILD"), h.objekte.iter().find(|o| o.2 == "bus_stop.sco")) {
            println!("Bild bei {:?}", o.0);
            let kam = crate::kamera::Kamera { ziel: o.0, gier: 200.0, neigung: -35.0, abstand: 14.0, fov: 50.0 };
            let px = v.render_image(1280, 800, &kam.camera()).unwrap();
            image::save_buffer(b, &px, 1280, 800, image::ColorType::Rgba8).unwrap();
        }
        h.leeren(&mut v);
    }
}
