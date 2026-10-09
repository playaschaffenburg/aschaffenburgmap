//! Luftbild ueber dem Gelaende (Karten mit Ort, siehe geo.rs): je geladener Kachel ein Bild, im Hintergrund von
//! omsigen geholt (DOP40 Bayern, genau auf die Kachel zugeschnitten), in openOMSI als Ebene auf das Gelaendenetz gelegt
//! (`Viewer::set_ground_image`) - es folgt dem Gelaende, Strassen und Objekte stehen darueber. Die Deckkraft stellt
//! der Regler oben ein (0: aus).

use crate::geo::{self, Bezug};
use openomsi_game::viewer::Viewer;
use std::collections::HashSet;
use std::thread::JoinHandle;

/// so viele Kacheln je Auftrag an omsigen
const JE_AUFTRAG: usize = 6;

type Ergebnis = anyhow::Result<(Vec<((i32, i32), Option<omsi_texture::TextureData>)>, Vec<String>)>;

pub struct Luftbild {
    pub deckkraft: f32,
    gesetzt: f32,
    pub bezug: Option<Bezug>,
    /// Kacheln mit Bild (oder ohne Daten: nicht noch einmal fragen)
    erledigt: HashSet<(i32, i32)>,
    job: Option<(Vec<(i32, i32)>, JoinHandle<Ergebnis>)>,
    pub quellen: Vec<String>,
    pub fehler: Option<String>,
}

impl Default for Luftbild {
    fn default() -> Self {
        Luftbild { deckkraft: 0.6, gesetzt: -1.0, bezug: None, erledigt: HashSet::new(), job: None, quellen: Vec::new(), fehler: None }
    }
}

impl Luftbild {
    /// neue Karte geoeffnet (Bezug aus ihrem Ordner)
    pub fn karte(&mut self, bezug: Option<Bezug>) {
        *self = Luftbild { deckkraft: self.deckkraft, bezug, ..Default::default() };
    }

    pub fn laedt(&self) -> bool {
        self.job.is_some()
    }

    /// jedes Bild: fertige Bilder setzen, fehlende fuer die geladenen Kacheln bestellen, Deckkraft
    pub fn aktualisieren(&mut self, v: &mut Viewer) {
        let Some(b) = self.bezug.clone() else { return };
        if (self.deckkraft - self.gesetzt).abs() > 1e-3 {
            v.set_ground_image_alpha(self.deckkraft);
            self.gesetzt = self.deckkraft;
        }
        if self.job.as_ref().is_some_and(|j| j.1.is_finished()) {
            let (kacheln, job) = self.job.take().unwrap();
            match job.join() {
                Ok(Ok((bilder, quellen))) => {
                    for (k, d) in bilder {
                        if let Some(d) = d {
                            v.set_ground_image(k.0, k.1, Some(d));
                        }
                    }
                    for q in quellen {
                        if !self.quellen.contains(&q) {
                            self.quellen.push(q);
                        }
                    }
                    self.erledigt.extend(kacheln);
                }
                Ok(Err(e)) => {
                    log::warn!("Luftbild: {e:#}");
                    self.fehler = Some(format!("{e:#}"));
                    // nicht dauernd neu versuchen
                    self.erledigt.extend(kacheln);
                }
                Err(_) => self.erledigt.extend(kacheln),
            }
        }
        if self.job.is_some() || self.deckkraft <= 0.0 {
            return;
        }
        let mut fehlen: Vec<(i32, i32)> = v.loaded_tile_keys().into_iter().filter(|k| !self.erledigt.contains(k)).collect();
        if fehlen.is_empty() {
            return;
        }
        fehlen.sort();
        fehlen.truncate(JE_AUFTRAG);
        let auftrag = fehlen.clone();
        self.job = Some((fehlen, std::thread::spawn(move || -> Ergebnis {
            let (daten, quellen) = geo::kacheln(&b, &auftrag, false, true)?;
            let mut aus = Vec::new();
            for d in daten {
                let bild = d.luftbild.as_ref().and_then(|p| {
                    let img = image::open(p).ok()?.to_rgba8();
                    let _ = std::fs::remove_file(p);
                    let (w, h) = img.dimensions();
                    Some(Viewer::ground_image_data(omsi_texture::Image { width: w, height: h, rgba: img.into_raw(), has_alpha: false }))
                });
                aus.push((d.kachel, bild));
            }
            Ok((aus, quellen))
        })));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    /// nur in der Sitzung: Karte OMSI_KARTE (mit Ort) bei OMSI_X/OMSI_Y mit Luftbild von oben (OMSI_BILD)
    #[test]
    #[ignore]
    fn nutzerkarte_luftbild() {
        let Ok(karte) = std::env::var("OMSI_KARTE") else { return };
        let zahl = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(150.0);
        let (x, y) = (zahl("OMSI_X"), zahl("OMSI_Y"));
        let root = std::path::Path::new(crate::bearbeiten::tests::OMSI);
        let ordner = root.join("maps").join(&karte);
        let b = Bezug::lesen(&ordner).expect("Karte ohne Ort");
        let (mut v, _) = Viewer::open(&openomsi_game::viewer::instance(), None, root, &ordner.join("global.cfg")).unwrap();
        v.tiles_around(DVec3::new(x, y, 0.0), 1).unwrap();
        let kacheln = v.loaded_tile_keys();
        let (daten, _) = geo::kacheln(&b, &kacheln, false, true).unwrap();
        for d in daten {
            if let Some(p) = d.luftbild {
                let img = image::open(&p).unwrap().to_rgba8();
                let (w, h) = img.dimensions();
                v.set_ground_image(d.kachel.0, d.kachel.1, Some(Viewer::ground_image_data(omsi_texture::Image { width: w, height: h, rgba: img.into_raw(), has_alpha: false })));
            }
        }
        v.set_ground_image_alpha(1.0);
        let z = v.terrain_height(x, y).unwrap_or(0.0);
        for (name, abstand) in [("oben", 330.0), ("nah", 60.0)] {
            let kam = crate::kamera::Kamera { ziel: DVec3::new(x, y, z), gier: 0.0, neigung: -89.9, abstand, fov: 50.0 };
            let px = v.render_image(1280, 800, &kam.camera()).unwrap();
            if let Some(bild) = std::env::var_os("OMSI_BILD") {
                image::save_buffer(std::path::PathBuf::from(&bild).with_extension(format!("{name}.png")), &px, 1280, 800, image::ColorType::Rgba8).unwrap();
            }
        }
    }
}
