//! Pruefbericht der Karte (omsigen.pruefbericht, Qualitaetsplan F8): je Kachel offene Spurenden, veraltete Schichten,
//! Gebaeude ueber Fahrbahn, Boden (fehlende Malmasken, Fahrbahn auf Gras), Budgets. Der World Editor zeigt die Funde der gewaehlten Kachel und erstellt den Bericht im
//! Hintergrund (ohne Bilder; Spielbilder und Luftbildvergleich: python -m omsigen.pruefbericht <Karte> --bilder --luftbild).
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

pub struct Pruefung {
    job: Option<Receiver<Result<(), String>>>,
    bericht: Option<serde_json::Value>,
    geladen_fuer: Option<PathBuf>,
    pub meldung: String,
}

/// lesbare Namen der Fundarten (wie in bericht.html)
pub fn name(art: &str) -> &str {
    match art {
        "spurende_offen" => "Spurende ohne Anschluss",
        "beinahe_anschluss" => "Beinahe-Anschluss",
        "schicht_veraltet" => "Schicht veraltet",
        "schicht_gebaut" => "Schicht nicht eingesetzt",
        "netz_veraltet" => "netz.json veraltet",
        "belegung_veraltet" => "Belegung veraltet",
        "gebaeude_auf_fahrbahn" => "Gebaeude ueber Fahrbahn",
        "boden_fehlt" => "Boden fehlt",
        "boden_fahrbahn_gras" => "Fahrbahn auf Gras (Boden)",
        "budget_objekte" => "Budget Objekte",
        "budget_dreiecke" => "Budget Dreiecke",
        "budget_textur_mb" => "Budget Texturspeicher",
        a => a,
    }
}

impl Pruefung {
    pub fn neu() -> Pruefung {
        Pruefung { job: None, bericht: None, geladen_fuer: None, meldung: String::new() }
    }

    pub fn laeuft(&self) -> bool {
        self.job.is_some()
    }

    fn datei(karte_ordner: &Path) -> PathBuf {
        karte_ordner.join("pruefung").join("bericht.json")
    }

    /// bericht.json der Karte lesen (einmal je Karte bzw. nach dem Erstellen)
    pub fn laden(&mut self, karte_ordner: &Path) {
        if self.geladen_fuer.as_deref() == Some(karte_ordner) {
            return;
        }
        self.geladen_fuer = Some(karte_ordner.to_path_buf());
        self.bericht = std::fs::read(Self::datei(karte_ordner)).ok().and_then(|b| serde_json::from_slice(&b).ok());
        if let Some(b) = &self.bericht {
            self.meldung = format!("Bericht vom {}: {} Funde", b["erstellt"].as_str().unwrap_or("?").replace('T', " "), b["funde_gesamt"]);
        }
    }

    /// Bericht im Hintergrund neu erstellen
    pub fn erstellen(&mut self, root: &Path, karte_ordner: &Path) {
        if self.job.is_some() {
            return;
        }
        let (tx, rx) = channel();
        let args: Vec<String> = vec!["-m".into(), "omsigen.pruefbericht".into(), karte_ordner.display().to_string(), "--omsi".into(), root.display().to_string()];
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
                Ok(o) if o.status.success() => Ok(()),
                Ok(o) => Err(String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("omsigen.pruefbericht fehlgeschlagen").to_string()),
                Err(e) => Err(format!("{python} starten: {e}")),
            };
            let _ = tx.send(r);
        });
        self.job = Some(rx);
        self.meldung = "erstelle Pruefbericht ...".into();
    }

    /// Ergebnis abholen; true, wenn neu
    pub fn abholen(&mut self) -> bool {
        let Some(rx) = &self.job else { return false };
        match rx.try_recv() {
            Ok(r) => {
                self.job = None;
                match r {
                    Ok(()) => {
                        self.geladen_fuer = None;          // neu laden
                        self.meldung = "Pruefbericht erstellt".into();
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

    /// Funde einer Kachel: (Art, Anzahl)
    pub fn kachel(&self, k: (i32, i32)) -> Vec<(String, u64)> {
        let Some(b) = &self.bericht else { return Vec::new() };
        let Some(m) = b["kacheln"][format!("{}_{}", k.0, k.1)].as_object() else { return Vec::new() };
        let mut v: Vec<(String, u64)> = m.iter().map(|(a, n)| (a.clone(), n.as_u64().unwrap_or(0))).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        v
    }

    /// Budget der Kachel als Text
    pub fn budget(&self, k: (i32, i32)) -> Option<String> {
        let b = &self.bericht.as_ref()?["budgets"][format!("{}_{}", k.0, k.1)];
        Some(format!("{} Objekte, {} Dreiecke, {} MB Texturen", b["objekte"], b["dreiecke"], b["textur_mb"]))
    }

    pub fn hat_bericht(&self) -> bool {
        self.bericht.is_some()
    }

    /// bericht.html im Browser oeffnen
    pub fn oeffnen(karte_ordner: &Path) {
        let p = karte_ordner.join("pruefung").join("bericht.html");
        if p.is_file() {
            let _ = std::process::Command::new("cmd").args(["/C", "start", ""]).arg(&p).spawn();
        }
    }
}
