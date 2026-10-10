//! Fotovergleich: freie Strassenfotos (Mapillary, CC BY-SA 4.0) genau dort, wo sie aufgenommen wurden. Die 3D-Ansicht
//! springt an Standort, Blickrichtung und Bildwinkel des Fotos, das Foto liegt halbtransparent darueber - so fallen
//! falsche Gelaender, fehlende Uebergaenge oder Markierungen sofort auf.
//!
//! omsigen (`python -m omsigen.mapillary`) holt die Fotos, rechnet die Pose ins Kartengitter um und entzerrt die Bilder
//! (Lochkamera), damit Foto und 3D-Bild deckungsgleich sind. Feinjustierung (Hoehe, Richtung, Neigung, Rollen,
//! Bildwinkel) gleicht Ungenauigkeiten der Fotopose aus.

use anyhow::{bail, Context, Result};
use glam::{DVec2, DVec3};
use openomsi_game::viewer::Camera;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};

#[derive(Clone, Debug, PartialEq)]
pub struct FotoPose {
    pub id: String,
    /// Kartenpunkt
    pub pos: DVec2,
    /// Grad, im Uhrzeigersinn ab Nord (Kartengitter)
    pub gier: f64,
    /// Grad, oben positiv
    pub neigung: f64,
    /// Grad, oben nach rechts positiv (wie openOMSI)
    pub roll: f64,
    pub vfov: f64,
    pub hfov: f64,
    /// ms seit 1970
    pub zeit: i64,
    pub sequenz: String,
}

impl FotoPose {
    pub fn aus_json(v: &serde_json::Value) -> Option<FotoPose> {
        Some(FotoPose {
            id: v["id"].as_str()?.to_string(),
            pos: DVec2::new(v["x"].as_f64()?, v["y"].as_f64()?),
            gier: v["gier"].as_f64()?,
            neigung: v["neigung"].as_f64()?,
            roll: v["roll"].as_f64().unwrap_or(0.0),
            vfov: v["vfov"].as_f64()?,
            hfov: v["hfov"].as_f64()?,
            zeit: v["zeit"].as_i64().unwrap_or(0),
            sequenz: v["sequenz"].as_str().unwrap_or("").to_string(),
        })
    }

    /// Jahr der Aufnahme
    pub fn jahr(&self) -> i32 {
        // 365,2425 Tage je Jahr reichen fuer die Anzeige
        1970 + (self.zeit as f64 / 1000.0 / 86400.0 / 365.2425).floor() as i32
    }

    /// Datum TT.MM.JJJJ
    pub fn datum(&self) -> String {
        let tage = self.zeit.div_euclid(86_400_000);
        // Tage seit 1970 -> Datum (Howard Hinnant, civil_from_days)
        let z = tage + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
        format!("{d:02}.{m:02}.{y}")
    }

    pub fn url(&self) -> String {
        format!("https://www.mapillary.com/app/?pKey={}&focus=photo", self.id)
    }
}

fn omsigen(args: &[String]) -> Result<serde_json::Value> {
    let python = std::env::var("OMSIGEN_PYTHON").unwrap_or_else(|_| "python".into());
    let mut cmd = std::process::Command::new(&python);
    cmd.args(["-m", "omsigen.mapillary"]).args(args).current_dir(crate::kreuzung::omsigen_ordner());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let aus = cmd.output().with_context(|| format!("{python} starten"))?;
    let erg: serde_json::Value = serde_json::from_slice(&aus.stdout)
        .with_context(|| format!("omsigen.mapillary: keine Antwort ({})", String::from_utf8_lossy(&aus.stderr).lines().last().unwrap_or("")))?;
    if let Some(f) = erg["fehler"].as_str() {
        bail!("{f}");
    }
    Ok(erg)
}

/// Fotos mit Pose im Kartenausschnitt (x0, y0, x1, y1), blockiert
pub fn fotos(b: &crate::geo::Bezug, bereich: [f64; 4]) -> Result<Vec<FotoPose>> {
    let mut args: Vec<String> = vec!["fotos".into(), "--ost0".into(), b.ost0.to_string(), "--nord0".into(), b.nord0.to_string(),
                                     "--zone".into(), b.zone.to_string(), "--bereich".into()];
    args.extend(bereich.iter().map(|x| x.to_string()));
    let erg = omsigen(&args)?;
    Ok(erg["fotos"].as_array().map(|a| a.iter().filter_map(FotoPose::aus_json).collect()).unwrap_or_default())
}

/// entzerrtes Foto (JPEG im Zwischenspeicher), blockiert
pub fn bild_datei(id: &str) -> Result<PathBuf> {
    let erg = omsigen(&["bild".into(), "--id".into(), id.into()])?;
    Ok(PathBuf::from(erg["datei"].as_str().context("keine Datei")?))
}

pub enum FotoAktion {
    /// voriges/naechstes Foto derselben Fahrt
    Blaettern(i32),
    Ende,
}

pub struct Fotovergleich {
    pub foto: FotoPose,
    /// Bodenhoehe am Standort (Kreuzungsflaeche oder Gelaende)
    pub boden: f64,
    pub deckkraft: f32,
    /// Kamera ueber dem Boden, m
    pub hoehe: f64,
    pub d_gier: f64,
    pub d_neigung: f64,
    pub d_roll: f64,
    /// Bildwinkel: Faktor auf den des Fotos
    pub fov_faktor: f64,
    /// Ansicht beim Start (Blickpunkt, Gier, Neigung, Abstand): aendert der Nutzer sie, endet der Vergleich
    pub merke: (DVec2, f32, f32, f64),
    /// Markierungen des Objekt-Editors im Vergleich ausblenden (nur Foto und 3D-Bild)
    pub ohne_markierungen: bool,
    textur: Option<egui::TextureHandle>,
    job: Option<Receiver<std::result::Result<egui::ColorImage, String>>>,
    pub fehler: Option<String>,
}

impl Fotovergleich {
    pub fn neu(foto: FotoPose, boden: f64, vorher: Option<&Fotovergleich>) -> Fotovergleich {
        let (tx, rx) = channel();
        let id = foto.id.clone();
        std::thread::spawn(move || {
            let erg = bild_datei(&id).map_err(|e| format!("{e:#}")).and_then(|p| image::open(&p).map_err(|e| e.to_string())).map(|i| {
                let i = i.to_rgba8();
                egui::ColorImage::from_rgba_unmultiplied([i.width() as usize, i.height() as usize], i.as_raw())
            });
            let _ = tx.send(erg);
        });
        // beim Blaettern bleiben Deckkraft und Hoehe (dieselbe Kamera), die Feinjustierung gilt je Foto
        let (deckkraft, hoehe, ohne) = vorher.map(|v| (v.deckkraft, v.hoehe, v.ohne_markierungen)).unwrap_or((0.5, 1.5, true));
        Fotovergleich { foto, boden, deckkraft, hoehe, d_gier: 0.0, d_neigung: 0.0, d_roll: 0.0, fov_faktor: 1.0, merke: (DVec2::ZERO, 0.0, 0.0, 0.0), ohne_markierungen: ohne,
                        textur: None, job: Some(rx), fehler: None }
    }

    /// Bildwinkel des Fotos (vertikal, horizontal) in Grad, mit Feinjustierung
    fn fov(&self) -> (f64, f64) {
        let t = |g: f64| ((g / 2.0).to_radians().tan() * self.fov_faktor).atan().to_degrees() * 2.0;
        (t(self.foto.vfov), t(self.foto.hfov))
    }

    /// vertikaler Bildwinkel der 3D-Ansicht: das ganze Foto passt hinein
    fn fov_ansicht(&self, aspekt: f64) -> f64 {
        let (v, h) = self.fov();
        let aus_breite = ((h / 2.0).to_radians().tan() / aspekt.max(0.1)).atan().to_degrees() * 2.0;
        v.max(aus_breite)
    }

    pub fn position(&self) -> DVec3 {
        self.foto.pos.extend(self.boden + self.hoehe)
    }

    /// Kamera fuer den Renderer (Seitenverhaeltnis der Ansicht)
    pub fn kamera(&self, aspekt: f64) -> Camera {
        Camera {
            position: self.position(),
            yaw: (self.foto.gier + self.d_gier) as f32,
            pitch: (self.foto.neigung + self.d_neigung) as f32,
            roll: (self.foto.roll + self.d_roll) as f32,
            fov_deg: self.fov_ansicht(aspekt) as f32,
            near: 0.2,
            far: 9000.0,
        }
    }

    /// Lage des Fotos auf dem Bildschirm (logische Punkte)
    pub fn rechteck(&self, breite: f32, hoehe: f32) -> egui::Rect {
        let (v, h) = self.fov();
        let va = self.fov_ansicht(breite as f64 / hoehe.max(1.0) as f64);
        let hh = hoehe as f64 / 2.0 * (v / 2.0).to_radians().tan() / (va / 2.0).to_radians().tan();
        let hw = hh * (h / 2.0).to_radians().tan() / (v / 2.0).to_radians().tan();
        egui::Rect::from_center_size(egui::pos2(breite / 2.0, hoehe / 2.0), egui::vec2((hw * 2.0) as f32, (hh * 2.0) as f32))
    }

    /// Foto ueber das 3D-Bild legen
    pub fn zeichnen(&mut self, ctx: &egui::Context, p: &egui::Painter, breite: f32, hoehe: f32) {
        if let Some(rx) = &self.job {
            if let Ok(r) = rx.try_recv() {
                self.job = None;
                match r {
                    Ok(img) => self.textur = Some(ctx.load_texture(format!("foto_{}", self.foto.id), img, egui::TextureOptions::LINEAR)),
                    Err(e) => self.fehler = Some(e),
                }
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(150));
            }
        }
        let r = self.rechteck(breite, hoehe);
        match &self.textur {
            Some(t) if self.deckkraft > 0.0 => {
                p.image(t.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::from_white_alpha((self.deckkraft * 255.0) as u8));
            }
            None if self.fehler.is_none() => {
                p.text(r.center(), egui::Align2::CENTER_CENTER, "Foto wird geladen ...", egui::FontId::proportional(18.0), egui::Color32::WHITE);
            }
            _ => {}
        }
        p.rect_stroke(r, 0.0, egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 120)), egui::StrokeKind::Outside);
        p.text(r.left_bottom() + egui::vec2(6.0, -6.0), egui::Align2::LEFT_BOTTOM, format!("Foto: Mapillary-Mitwirkende, CC BY-SA 4.0 ({})", self.foto.datum()),
               egui::FontId::proportional(11.0), egui::Color32::from_gray(230));
    }

    /// Bedienfenster
    pub fn ui(&mut self, ctx: &egui::Context, nachbarn: (bool, bool)) -> Option<FotoAktion> {
        let mut erg = None;
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            return Some(FotoAktion::Ende);
        }
        egui::Window::new("Fotovergleich").anchor(egui::Align2::LEFT_TOP, [12.0, 48.0]).resizable(false).collapsible(true).show(ctx, |ui| {
            ui.label(egui::RichText::new(format!("Foto vom {}", self.foto.datum())).strong());
            if let Some(f) = &self.fehler {
                ui.colored_label(egui::Color32::from_rgb(255, 120, 90), format!("Foto nicht ladbar: {f}"));
            }
            ui.horizontal(|ui| {
                if ui.add_enabled(nachbarn.0, egui::Button::new("\u{2039} voriges")).on_hover_text("voriges Foto derselben Fahrt (Bild auf)").clicked() {
                    erg = Some(FotoAktion::Blaettern(-1));
                }
                if ui.add_enabled(nachbarn.1, egui::Button::new("naechstes \u{203A}")).on_hover_text("naechstes Foto derselben Fahrt (Bild ab)").clicked() {
                    erg = Some(FotoAktion::Blaettern(1));
                }
                if ui.button("Beenden").on_hover_text("Esc - oder einfach die Kamera bewegen").clicked() {
                    erg = Some(FotoAktion::Ende);
                }
            });
            ui.add(egui::Slider::new(&mut self.deckkraft, 0.0..=1.0).text("Foto-Deckkraft"));
            ui.checkbox(&mut self.ohne_markierungen, "Markierungen des Objekt-Editors ausblenden");
            ui.label(egui::RichText::new("Feinjustierung (bis Foto und 3D-Bild deckungsgleich sind)").small().weak());
            ui.add(egui::Slider::new(&mut self.hoehe, 0.3..=4.0).step_by(0.05).suffix(" m").text("Kamerahoehe"));
            ui.add(egui::Slider::new(&mut self.d_gier, -8.0..=8.0).step_by(0.1).suffix("\u{B0}").text("Richtung"));
            ui.add(egui::Slider::new(&mut self.d_neigung, -8.0..=8.0).step_by(0.1).suffix("\u{B0}").text("Neigung"));
            ui.add(egui::Slider::new(&mut self.d_roll, -8.0..=8.0).step_by(0.1).suffix("\u{B0}").text("Rollen"));
            ui.add(egui::Slider::new(&mut self.fov_faktor, 0.8..=1.25).step_by(0.005).text("Bildwinkel"));
            ui.horizontal(|ui| {
                if ui.small_button("Justierung zuruecksetzen").clicked() {
                    self.d_gier = 0.0;
                    self.d_neigung = 0.0;
                    self.d_roll = 0.0;
                    self.fov_faktor = 1.0;
                }
                ui.hyperlink_to(egui::RichText::new("in Mapillary").small(), self.foto.url());
            });
        });
        if ctx.input(|i| i.key_pressed(egui::Key::PageUp)) && nachbarn.0 {
            erg = Some(FotoAktion::Blaettern(-1));
        }
        if ctx.input(|i| i.key_pressed(egui::Key::PageDown)) && nachbarn.1 {
            erg = Some(FotoAktion::Blaettern(1));
        }
        erg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose() -> FotoPose {
        FotoPose { id: "1".into(), pos: DVec2::new(100.0, 200.0), gier: 344.7, neigung: -0.8, roll: -3.0, vfov: 67.03, hfov: 99.32,
                   zeit: 1_776_543_608_100, sequenz: "s".into() }
    }

    #[test]
    fn datum_und_jahr() {
        assert_eq!(pose().datum(), "18.04.2026");
        assert_eq!(pose().jahr(), 2026);
    }

    #[test]
    fn foto_passt_in_die_ansicht() {
        let f = Fotovergleich::neu(pose(), 10.0, None);
        // breites Fenster: Hoehe bestimmt, Foto fuellt die Hoehe
        let r = f.rechteck(2000.0, 800.0);
        assert!((r.height() - 800.0).abs() < 0.5 && r.width() < 2000.0, "{r:?}");
        // schmales Fenster: Breite bestimmt
        let r = f.rechteck(800.0, 800.0);
        assert!((r.width() - 800.0).abs() < 0.5 && r.height() < 800.0, "{r:?}");
        // die Kamera sieht genau den Bildwinkel des Fotos: Fotorand = Bildschirmrand des Strahls
        let cam = f.kamera(1.0);
        assert!((cam.position.z - 11.5).abs() < 1e-9);
        let (_, d) = cam.ray(1.0 * r.width() / 800.0, 0.0, 1.0, cam.position);
        let rechts = cam.right();
        let winkel = d.dot(rechts).asin().to_degrees();
        assert!((winkel as f64 - 99.32 / 2.0).abs() < 0.3, "{winkel}");
    }
}
