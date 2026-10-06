//! Vorschaubilder fuer Objekt- und Strassenkatalog: jedes Objekt (.sco) bzw. ein gerades Strassenstueck (.sli)
//! einmal allein gerendert (openOMSI, vor Himmel), als PNG
//! zwischengespeichert in %LOCALAPPDATA%\omsi-editor\vorschau. Erzeugt werden nur die Bilder, die gerade im
//! Katalog zu sehen sind, wenige je Bild, damit die Bedienung fluessig bleibt.

use openomsi_game::viewer::Viewer;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;

pub const GROESSE: u32 = 96;
const VERSION: &str = "v1";

pub fn ordner() -> PathBuf {
    let basis = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    basis.join("omsi-editor").join("vorschau").join(VERSION)
}

/// Dateiname fuer ein Objekt (relativer .sco-Pfad, Gross/Klein egal)
pub fn datei(rel: &str) -> PathBuf {
    let k = rel.to_lowercase().replace('/', "\\");
    // FNV-1a, 64 Bit: stabil und ohne weitere Abhaengigkeit
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in k.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    ordner().join(format!("{h:016x}.png"))
}

#[derive(Default)]
pub struct Vorschau {
    texturen: HashMap<String, egui::TextureHandle>,
    /// keine Vorschau moeglich (Objekt nicht ladbar)
    fehlt: HashSet<String>,
    /// in diesem Bild angefragt (sichtbar), noch ohne Bild
    wunsch: VecDeque<String>,
    pub erzeugt: usize,
}

impl Vorschau {
    /// Textur fuer die Anzeige, sonst None (dann wird sie angefragt)
    pub fn textur(&mut self, ctx: &egui::Context, rel: &str) -> Option<egui::TextureHandle> {
        if let Some(t) = self.texturen.get(rel) {
            return Some(t.clone());
        }
        if self.fehlt.contains(rel) {
            return None;
        }
        let d = datei(rel);
        if let Ok(img) = image::open(&d) {
            let rgba = img.to_rgba8();
            let t = ctx.load_texture(rel, egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()), egui::TextureOptions::LINEAR);
            self.texturen.insert(rel.to_string(), t.clone());
            return Some(t);
        }
        if !self.wunsch.iter().any(|w| w == rel) {
            self.wunsch.push_back(rel.to_string());
        }
        None
    }

    pub fn fehlt(&self, rel: &str) -> bool {
        self.fehlt.contains(rel)
    }

    /// Ein paar der gewuenschten Bilder rendern (hoechstens `zeit_ms` lang), die Wunschliste danach leeren:
    /// was dann noch zu sehen ist, wird im naechsten Bild wieder angefragt
    pub fn erzeugen(&mut self, v: &mut Viewer, ctx: &egui::Context, zeit_ms: u64) {
        let t0 = std::time::Instant::now();
        while let Some(rel) = self.wunsch.pop_front() {
            if t0.elapsed().as_millis() as u64 >= zeit_ms {
                break;
            }
            let bild = if rel.to_ascii_lowercase().ends_with(".sli") { v.preview_spline_image(&rel, GROESSE) } else { v.preview_image(&rel, GROESSE) };
            match bild {
                Some(px) => {
                    let d = datei(&rel);
                    let _ = std::fs::create_dir_all(d.parent().unwrap());
                    let _ = image::save_buffer(&d, &px, GROESSE, GROESSE, image::ColorType::Rgba8);
                    let t = ctx.load_texture(&rel, egui::ColorImage::from_rgba_unmultiplied([GROESSE as usize, GROESSE as usize], &px), egui::TextureOptions::LINEAR);
                    self.texturen.insert(rel, t);
                    self.erzeugt += 1;
                }
                None => {
                    self.fehlt.insert(rel);
                }
            }
        }
        self.wunsch.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dateiname_stabil() {
        assert_eq!(datei("Sceneryobjects\\A\\b.sco"), datei("sceneryobjects/a/B.SCO"));
        assert_ne!(datei("Sceneryobjects\\A\\b.sco"), datei("Sceneryobjects\\A\\c.sco"));
    }

    #[test]
    #[ignore]
    fn vorschaubilder_rendern() {
        // mit echter Installation: ein paar Bilder nach %TEMP%\omsi-editor-vorschau-test
        let mut v = crate::bearbeiten::tests::grundorf();
        let aus = std::env::temp_dir().join("omsi-editor-vorschau-test");
        std::fs::create_dir_all(&aus).unwrap();
        let mut n = 0;
        let mut arten: Vec<String> = v.objects().into_iter().map(|o| crate::bearbeiten::relativ(&v.root, &o.3)).collect();
        arten.sort();
        arten.dedup();
        let schritt = (arten.len() / 6).max(1);
        for rel in arten.iter().step_by(schritt).take(6) {
            if let Some(px) = v.preview_image(rel, 256) {
                image::save_buffer(aus.join(format!("{n}.png")), &px, 256, 256, image::ColorType::Rgba8).unwrap();
                println!("{n}: {rel}");
                n += 1;
            }
        }
        assert!(n >= 2, "nur {n} Vorschaubilder");
        // Strassen
        for (k, rel) in ["Splines\\Marcel\\str_2spur_10m_Grunewaldstr.sli", "Splines\\Marcel\\str_2spur_11m_SeeburgerStr1.sli"].iter().enumerate() {
            let px = v.preview_spline_image(rel, 256).expect("Strassenvorschau");
            image::save_buffer(aus.join(format!("s{k}.png")), &px, 256, 256, image::ColorType::Rgba8).unwrap();
        }
    }
}
