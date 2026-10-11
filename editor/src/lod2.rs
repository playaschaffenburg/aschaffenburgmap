//! Import „Gebaeude aus LoD2“: die 3D-Gebaeudemodelle der Bayerischen Vermessungsverwaltung (Open Data, CC BY 4.0) um
//! die Blickmitte als OMSI-Objekte bauen (omsigen.lod2) und in die Sitzung setzen. Erneutes Einsetzen ersetzt nur die
//! LoD2-Gebaeude (Kennung `\LoD2\` im Pfad). Fehlende LoD2-Kacheln (2 km, je bis ~120 MB) laedt es erst nach Rueckfrage.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

use glam::DVec2;

pub enum Aktion {
    Gebaut { json: PathBuf, objekte: Vec<String> },
    Schliessen,
}

pub struct Lod2 {
    root: PathBuf,
    karte: String,
    pub radius: f64,
    mitte: DVec2,
    job: Option<Receiver<Result<serde_json::Value, String>>>,
    fehlt: Option<String>,
    meldung: String,
}

impl Lod2 {
    pub fn neu(root: &Path, karte: &str) -> Lod2 {
        Lod2 { root: root.to_path_buf(), karte: karte.to_string(), radius: 300.0, mitte: DVec2::ZERO, job: None, fehlt: None,
               meldung: "Gebaeude im Umkreis der Blickmitte bauen".into() }
    }

    fn starten(&mut self, laden: bool) {
        let (tx, rx) = channel();
        let mut args: Vec<String> = vec!["-m".into(), "omsigen.lod2".into(), "--omsi".into(), self.root.display().to_string(), "--karte".into(),
                                         self.karte.clone(), "--mitte".into(), format!("{:.1}", self.mitte.x), format!("{:.1}", self.mitte.y),
                                         "--radius".into(), format!("{:.0}", self.radius)];
        if laden {
            args.push("--laden".into());
        }
        std::thread::spawn(move || {
            let python = std::env::var("OMSIGEN_PYTHON").unwrap_or_else(|_| "python".into());
            let mut cmd = std::process::Command::new(&python);
            cmd.args(&args).current_dir(crate::kreuzung::omsigen_ordner());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x0800_0000);
            }
            let r = match cmd.output() {
                Ok(o) => {
                    let t = String::from_utf8_lossy(&o.stdout);
                    match t.lines().rev().find(|l| l.trim_start().starts_with('{')).map(serde_json::from_str::<serde_json::Value>) {
                        Some(Ok(v)) => Ok(v),
                        _ => Err(String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("keine Antwort von omsigen.lod2").to_string()),
                    }
                }
                Err(e) => Err(format!("{python} starten: {e}")),
            };
            let _ = tx.send(r);
        });
        self.job = Some(rx);
        self.fehlt = None;
        self.meldung = if laden { "lade LoD2-Kacheln und baue ...".into() } else { "baue Gebaeude ...".into() };
    }

    pub fn ui(&mut self, ctx: &egui::Context, mitte: DVec2) -> Option<Aktion> {
        let mut aktion = None;
        if let Some(rx) = &self.job {
            match rx.try_recv() {
                Ok(Ok(v)) => {
                    self.job = None;
                    if let Some(f) = v["fehler"].as_str() {
                        if f.contains("Kacheln fehlen") {
                            self.fehlt = Some(f.split(": ").nth(1).unwrap_or(f).replace(" (--laden)", ""));
                            self.meldung = "LoD2-Kacheln fehlen".into();
                        } else {
                            self.meldung = format!("Fehler: {f}");
                        }
                    } else {
                        self.meldung = format!("{} Gebaeude ({} Dreiecke) gebaut - eingesetzt", v["gebaeude"], v["dreiecke"]);
                        let json = PathBuf::from(v["json"].as_str().unwrap_or(""));
                        let objekte = std::fs::read(&json).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                            .and_then(|j| j["objekte"].as_array().map(|a| a.iter().filter_map(|o| o["rel"].as_str().map(|s| s.to_string())).collect()))
                            .unwrap_or_default();
                        aktion = Some(Aktion::Gebaut { json, objekte });
                    }
                }
                Ok(Err(e)) => {
                    self.job = None;
                    self.meldung = format!("Fehler: {e}");
                }
                Err(_) => ctx.request_repaint_after(std::time::Duration::from_millis(300)),
            }
        }
        egui::Window::new("Gebaeude aus LoD2").anchor(egui::Align2::RIGHT_TOP, egui::vec2(-12.0, 44.0)).resizable(false).collapsible(true).show(ctx, |ui| {
            ui.label("3D-Gebaeudemodelle (Kataster-Grundrisse, Daecher aus Laserdaten)\nder Bayerischen Vermessungsverwaltung.");
            ui.add(egui::Slider::new(&mut self.radius, 50.0..=1500.0).text("m Umkreis um die Blickmitte"));
            ui.label(format!("Blickmitte: x {:.0}, y {:.0}", mitte.x, mitte.y));
            if ui.add_enabled(self.job.is_none(), egui::Button::new("Bauen und einsetzen")).on_hover_text("ersetzt fruehere LoD2-Gebaeude, sonst nichts in der Karte").clicked() {
                self.mitte = mitte;
                self.starten(false);
            }
            if let Some(k) = self.fehlt.clone() {
                ui.colored_label(egui::Color32::from_rgb(230, 160, 40), format!("Es fehlen LoD2-Kacheln: {k} (je bis etwa 120 MB)."));
                ui.horizontal(|ui| {
                    if ui.button("Herunterladen und bauen").clicked() {
                        self.starten(true);
                    }
                    if ui.button("Abbrechen").clicked() {
                        self.fehlt = None;
                    }
                });
            }
            if self.job.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(&self.meldung);
                });
            } else {
                ui.label(egui::RichText::new(&self.meldung).small());
            }
            ui.label(egui::RichText::new("Bayerische Vermessungsverwaltung - www.geodaten.bayern.de (CC BY 4.0)").small().weak());
            if ui.small_button("Schliessen").clicked() {
                aktion = Some(Aktion::Schliessen);
            }
        });
        aktion
    }
}
