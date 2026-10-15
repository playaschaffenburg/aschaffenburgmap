//! Schichten der Karte (omsigen.schichten, Qualitaetsplan F2): Netz, Boden, Gebaeude, Objekte, Gruen, Backdrop werden
//! je Kachel getrennt gebaut und eingesetzt. `maps/<Karte>/schichten.json` haelt fest, gegen welchen Stand der Schichten
//! davor ein Paket gebaut wurde; hier fragt der World Editor den Zustand ab (aktuell, veraltet, gebaut, eingesetzt) und
//! zeigt ihn je Kachel. Verglichen wird die gespeicherte Karte - Aenderungen der Sitzung zaehlen erst nach dem Speichern.
use std::path::Path;
use std::sync::mpsc::{channel, Receiver};

pub struct Schichten {
    job: Option<Receiver<Result<serde_json::Value, String>>>,
    stand: Option<serde_json::Value>,
    pub meldung: String,
}

impl Schichten {
    pub fn neu() -> Schichten {
        Schichten { job: None, stand: None, meldung: String::new() }
    }

    pub fn laeuft(&self) -> bool {
        self.job.is_some()
    }

    /// Zustand im Hintergrund abfragen: python -m omsigen.schichten <Karte> --omsi <root> --json
    pub fn pruefen(&mut self, root: &Path, karte: &str) {
        if self.job.is_some() {
            return;
        }
        let (tx, rx) = channel();
        let args: Vec<String> = vec!["-m".into(), "omsigen.schichten".into(), karte.to_string(), "--omsi".into(), root.display().to_string(), "--json".into()];
        std::thread::spawn(move || {
            let python = crate::kreuzung::python();
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
                        _ => Err(String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("keine Antwort von omsigen.schichten").to_string()),
                    }
                }
                Err(e) => Err(format!("{python} starten: {e}")),
            };
            let _ = tx.send(r);
        });
        self.job = Some(rx);
        self.meldung = "pruefe Schichten ...".into();
    }

    /// Ergebnis abholen (jedes Bild); true, wenn neu
    pub fn abholen(&mut self) -> bool {
        let Some(rx) = &self.job else { return false };
        match rx.try_recv() {
            Ok(r) => {
                self.job = None;
                match r {
                    Ok(v) => {
                        let n = v["kacheln"].as_object().map(|m| m.len()).unwrap_or(0);
                        self.meldung = format!("{n} Kachel(n) mit Schichten");
                        self.stand = Some(v);
                    }
                    Err(e) => self.meldung = format!("Fehler: {e}"),
                }
                true
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => false,
            Err(_) => {
                self.job = None;
                false
            }
        }
    }

    /// (Schicht, Zustand, Grund) einer Kachel in der Reihenfolge der Schichten
    pub fn zeilen(&self, k: (i32, i32)) -> Vec<(String, String, String)> {
        let Some(v) = &self.stand else { return Vec::new() };
        let Some(m) = v["kacheln"][format!("{}_{}", k.0, k.1)].as_object() else { return Vec::new() };
        ["gebaeude", "objekte", "gruen", "backdrop"]
            .iter()
            .filter_map(|s| {
                let z = m.get(*s)?;
                let zustand = z["zustand"].as_str().unwrap_or("").to_string();
                (zustand != "fehlt").then(|| (s.to_string(), zustand, z["grund"].as_str().unwrap_or("").to_string()))
            })
            .collect()
    }

    /// Kacheln mit veralteter Schicht `schicht`
    pub fn veraltet(&self, schicht: &str) -> Vec<(i32, i32)> {
        let Some(m) = self.stand.as_ref().and_then(|v| v["kacheln"].as_object()) else { return Vec::new() };
        m.iter()
            .filter(|(_, z)| z[schicht]["zustand"].as_str() == Some("veraltet"))
            .filter_map(|(k, _)| {
                let (a, b) = k.split_once('_')?;
                Some((a.parse().ok()?, b.parse().ok()?))
            })
            .collect()
    }
}

/// Anzeigename und Farbe eines Zustands
pub fn anzeige(zustand: &str) -> (&'static str, egui::Color32) {
    match zustand {
        "aktuell" => ("aktuell", egui::Color32::from_rgb(110, 200, 110)),
        "veraltet" => ("veraltet", egui::Color32::from_rgb(235, 160, 50)),
        "gebaut" => ("gebaut, nicht eingesetzt", egui::Color32::from_rgb(150, 170, 230)),
        "eingesetzt" => ("eingesetzt (ohne Verzeichnis)", egui::Color32::from_rgb(180, 180, 180)),
        _ => ("-", egui::Color32::GRAY),
    }
}

pub fn name(schicht: &str) -> &'static str {
    match schicht {
        "gebaeude" => "Gebaeude",
        "objekte" => "Objekte",
        "gruen" => "Gruen",
        "backdrop" => "Backdrop",
        _ => "?",
    }
}

/// netz.json (F3) und Belegungskarte (F4) der Karte neu ableiten - nach dem Speichern, im Hintergrund, nacheinander
/// (die Belegung liest netz.json); das Ergebnis steht im Protokoll. So passen Netz und Belegung, auf die sich spaetere
/// Schichten beziehen, immer zur gespeicherten Karte.
pub fn netz_aktualisieren(root: &Path, karte_ordner: &Path) {
    let ordner = karte_ordner.display().to_string();
    let omsi = root.display().to_string();
    std::thread::spawn(move || {
        let python = crate::kreuzung::python();
        for (modul, name) in [("omsigen.netz_datei", "netz.json"), ("omsigen.belegung", "Belegung")] {
            let mut cmd = std::process::Command::new(&python);
            cmd.args(["-m", modul, &ordner, "--omsi", &omsi]).current_dir(crate::kreuzung::omsigen_ordner());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x0800_0000);
            }
            match cmd.output() {
                Ok(o) if o.status.success() => log::info!("{name}: {}", String::from_utf8_lossy(&o.stdout).trim()),
                Ok(o) => {
                    log::warn!("{name} nicht geschrieben: {}", String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("?"));
                    break;
                }
                Err(e) => {
                    log::warn!("{name}: {python} starten: {e}");
                    break;
                }
            }
        }
    });
}
