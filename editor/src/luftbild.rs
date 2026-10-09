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
