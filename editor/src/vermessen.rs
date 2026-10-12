//! Werkzeug „Vermessen“: ein Bauwerk entlang einer Achse aus den Laserdaten und dem Luftbild (Bayern, CC BY 4.0)
//! vermessen, die Messung pruefen und korrigieren, daraus OMSI-Splines bauen und in die Sitzung setzen.
//!
//! 1. Achse auf der Karte klicken (Mitte des Bauwerks, in Fahrtrichtung), Breite links/rechts einstellen.
//! 2. „Vermessen“: omsigen.aufmass rechnet Kanten (Waende), Streifen (Fahrbahn/Boden), Markierungen, Sperrflaechen;
//!    fehlen Laserdaten, fragt das Werkzeug vor dem Laden (Groesse).
//! 3. Pruefen: alles erscheint in der 3D-Ansicht; Klick waehlt ein Teil, im Fenster stehen seine Annahmen (Wandart,
//!    Gelaender, Fahrbahn oder Boden, Richtung, Fahrstreifen, Material, bauen ja/nein) - der Nutzer entscheidet.
//! 4. „Bauen und einsetzen“: Splines und Objekte entstehen, die Sitzung bekommt sie (Rueckgaengig wie gewohnt).

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

use glam::{DVec2, DVec3};

pub enum Aktion {
    Gebaut { json: PathBuf, objekte: Vec<String> },
    Kamera(DVec3),
    Schliessen,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Teil {
    Kante,
    Streifen,
    Markierung,
    Sperr,
}

impl Teil {
    fn feld(self) -> &'static str {
        match self {
            Teil::Kante => "kanten",
            Teil::Streifen => "streifen",
            Teil::Markierung => "markierungen",
            Teil::Sperr => "sperrflaechen",
        }
    }
}

type Job = Receiver<Result<serde_json::Value, String>>;

pub struct Vermessen {
    root: PathBuf,
    karte: String,
    /// geklickte Achse (Welt)
    pub achse: Vec<DVec3>,
    pub links: f64,
    pub rechts: f64,
    pub name: String,
    messung: Option<serde_json::Value>,
    pfad: Option<PathBuf>,
    geaendert: bool,
    pub wahl: Option<(Teil, usize)>,
    job: Option<(&'static str, Job)>,
    /// fehlende Laserdaten-Kacheln (Name, MB) - Rueckfrage vor dem Laden
    download: Option<Vec<(String, Option<f64>)>>,
    meldung: String,
    pruefung: String,
    vorhandene: Vec<String>,
    /// zuletzt gezeichnete Linien im Bild (fuer die Auswahl per Klick)
    bild: Vec<(Teil, usize, Vec<egui::Pos2>)>,
}

fn aufmass_ordner(root: &Path, karte: &str) -> PathBuf {
    root.join("Sceneryobjects").join("Aschaffenburg").join(karte).join("Aufmass")
}

/// omsigen.aufmass im Hintergrund: letzte stdout-Zeile ist JSON
fn starten(args: Vec<String>) -> Job {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let python = crate::kreuzung::python();
        let mut cmd = std::process::Command::new(&python);
        cmd.args(["-m", "omsigen.aufmass"]).args(&args).current_dir(crate::kreuzung::omsigen_ordner());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        let erg = match cmd.output() {
            Ok(o) => {
                let t = String::from_utf8_lossy(&o.stdout);
                match t.lines().rev().find(|l| l.trim_start().starts_with('{')).map(serde_json::from_str::<serde_json::Value>) {
                    Some(Ok(v)) => match v["fehler"].as_str() {
                        Some(f) => Err(f.to_string()),
                        None => Ok(v),
                    },
                    _ => Err(String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("keine Antwort von omsigen.aufmass").to_string()),
                }
            }
            Err(e) => Err(format!("{python} starten: {e}")),
        };
        let _ = tx.send(erg);
    });
    rx
}

impl Vermessen {
    pub fn neu(root: &Path, karte: &str) -> Vermessen {
        let mut v = Vermessen {
            root: root.to_path_buf(), karte: karte.to_string(), achse: vec![], links: 15.0, rechts: 15.0, name: String::new(),
            messung: None, pfad: None, geaendert: false, wahl: None, job: None, download: None,
            meldung: "Achse klicken: Mitte des Bauwerks in Fahrtrichtung (Ruecktaste nimmt einen Punkt zurueck)".into(),
            pruefung: String::new(), vorhandene: vec![], bild: vec![],
        };
        v.vorhandene = v.liste();
        let mut n = 1;
        while v.vorhandene.contains(&format!("Aufmass_{n}")) {
            n += 1;
        }
        v.name = format!("Aufmass_{n}");
        v
    }

    fn liste(&self) -> Vec<String> {
        let mut out: Vec<String> = std::fs::read_dir(aufmass_ordner(&self.root, &self.karte)).map(|d| {
            d.flatten().filter(|e| e.path().join("messung.json").is_file()).map(|e| e.file_name().to_string_lossy().to_string()).collect()
        }).unwrap_or_default();
        out.sort();
        out
    }

    /// Klick auf den Boden: Achspunkt (solange noch nicht vermessen) oder Auswahl eines Teils
    pub fn klick(&mut self, boden: Option<DVec3>, maus: egui::Pos2) {
        if self.messung.is_none() {
            if let Some(p) = boden {
                self.achse.push(p);
            }
            return;
        }
        self.wahl = self.treffer(maus);
    }

    pub fn zurueck(&mut self) {
        if self.messung.is_none() {
            self.achse.pop();
        }
    }

    fn treffer(&self, maus: egui::Pos2) -> Option<(Teil, usize)> {
        let mut best: Option<(f32, Teil, usize)> = None;
        for (teil, i, pts) in &self.bild {
            for w in pts.windows(2) {
                let (a, b) = (w[0], w[1]);
                let ab = b - a;
                let l2 = ab.length_sq().max(1e-6);
                let u = ((maus - a).dot(ab) / l2).clamp(0.0, 1.0);
                let d = (a + ab * u - maus).length();
                // Streifen nur, wenn nichts anderes nahe ist (ihre Mittellinie liegt zwischen den Kanten)
                let d = if *teil == Teil::Streifen { d + 4.0 } else { d };
                if d < 12.0 && best.map(|b| d < b.0).unwrap_or(true) {
                    best = Some((d, *teil, *i));
                }
            }
        }
        best.map(|b| (b.1, b.2))
    }

    /// Mitte der Achse (Hinsehen)
    pub fn mitte_achse(&self) -> Option<DVec3> {
        let m = self.messung.as_ref()?;
        let a = m["achse"].as_array()?;
        let q = a.get(a.len() / 2)?;
        let k = a.len() / 2;
        let z = m["streifen"].as_array()?.iter().filter_map(|s| s["a"][k].as_f64()).fold(f64::MAX, f64::min);
        Some(DVec3::new(q[0].as_f64()?, q[1].as_f64()?, if z < f64::MAX { z } else { 0.0 }))
    }

    /// fruehere Messung oeffnen
    pub fn oeffnen(&mut self, name: &str) {
        self.laden(name);
        if let Ok(w) = std::env::var("OMSI_VERMESSEN_WAHL") {
            // z. B. "kante:2" (Pruefen ohne Klicks)
            let (t, i) = w.split_once(':').unwrap_or(("", ""));
            let teil = match t { "kante" => Some(Teil::Kante), "streifen" => Some(Teil::Streifen), "markierung" => Some(Teil::Markierung), "sperr" => Some(Teil::Sperr), _ => None };
            self.wahl = teil.zip(i.parse().ok());
        }
    }

    fn laden(&mut self, name: &str) {
        let pfad = aufmass_ordner(&self.root, &self.karte).join(name).join("messung.json");
        match std::fs::read(&pfad).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) {
            Some(m) => {
                self.achse = m["achse_klick"].as_array().map(|a| {
                    a.chunks(2).filter(|c| c.len() == 2).map(|c| DVec3::new(c[0].as_f64().unwrap_or(0.0), c[1].as_f64().unwrap_or(0.0), 0.0)).collect()
                }).unwrap_or_default();
                self.links = -m["t0"].as_f64().unwrap_or(-15.0);
                self.rechts = m["t1"].as_f64().unwrap_or(15.0);
                self.name = name.to_string();
                self.meldung = format!("Messung {name} geladen");
                self.messung = Some(m);
                self.pfad = Some(pfad);
                self.wahl = None;
                self.geaendert = false;
            }
            None => self.meldung = format!("{name}: messung.json nicht lesbar"),
        }
    }

    fn speichern(&mut self) -> Result<(), String> {
        let (Some(m), Some(p)) = (&self.messung, &self.pfad) else { return Ok(()) };
        if self.geaendert {
            let t = serde_json::to_string_pretty(m).map_err(|e| e.to_string())?;
            std::fs::write(p, t).map_err(|e| format!("{}: {e}", p.display()))?;
            self.geaendert = false;
        }
        Ok(())
    }

    fn messen(&mut self, laden: bool) {
        let name: String = self.name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
        if name.is_empty() {
            self.meldung = "Name fehlt".into();
            return;
        }
        self.name = name.clone();
        let mut args: Vec<String> = vec!["messen".into(), "--omsi".into(), self.root.display().to_string(), "--karte".into(), self.karte.clone(),
                                         "--name".into(), name, "--links".into(), self.links.to_string(), "--rechts".into(), self.rechts.to_string(),
                                         "--achse".into()];
        for p in &self.achse {
            args.push(format!("{:.2}", p.x));
            args.push(format!("{:.2}", p.y));
        }
        if laden {
            args.push("--laden".into());
        }
        self.download = None;
        self.meldung = if laden { "lade Laserdaten und vermesse ...".into() } else { "vermesse ...".into() };
        self.job = Some(("messen", starten(args)));
    }

    fn bauen(&mut self) {
        if let Err(e) = self.speichern() {
            self.meldung = format!("Messung nicht gespeichert: {e}");
            return;
        }
        let Some(p) = &self.pfad else { return };
        let args = vec!["bauen".into(), "--omsi".into(), self.root.display().to_string(), "--messung".into(), p.display().to_string()];
        self.meldung = "baue Splines und Objekte ...".into();
        self.job = Some(("bauen", starten(args)));
    }

    /// Job abholen -> Aktion
    fn abholen(&mut self) -> Option<Aktion> {
        let (art, rx) = self.job.as_ref()?;
        let r = rx.try_recv().ok()?;
        let art = *art;
        self.job = None;
        match (art, r) {
            ("messen", Ok(v)) if v.get("download").is_some() => {
                let k: Vec<(String, Option<f64>)> = v["download"].as_array().map(|a| a.iter().map(|x| (x["kachel"].as_str().unwrap_or("").to_string(), x["mb"].as_f64())).collect()).unwrap_or_default();
                self.meldung = "Laserdaten fehlen".into();
                self.download = Some(k);
                None
            }
            ("messen", Ok(v)) => {
                let pfad = PathBuf::from(v["datei"].as_str().unwrap_or(""));
                let name = self.name.clone();
                self.laden(&name);
                self.pfad = Some(pfad);
                self.vorhandene = self.liste();
                self.meldung = format!("vermessen: {} - bitte pruefen (Teile anklicken)", v["zusammenfassung"].as_str().unwrap_or(""));
                None
            }
            ("bauen", Ok(v)) => {
                self.pruefung = v["pruefung"].as_str().unwrap_or("").to_string();
                self.meldung = format!("gebaut: {} Splines - eingesetzt in die Sitzung", v["splines"]);
                let objekte = v["objekte"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
                Some(Aktion::Gebaut { json: PathBuf::from(v["json"].as_str().unwrap_or("")), objekte })
            }
            (_, Err(e)) => {
                self.meldung = format!("Fehler: {e}");
                None
            }
            _ => None,
        }
    }

    // ---------------------------------------------------------------- Fenster
    pub fn ui(&mut self, ctx: &egui::Context) -> Option<Aktion> {
        let mut aktion = self.abholen();
        if self.job.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        egui::Window::new("Vermessen (Laserdaten)").anchor(egui::Align2::RIGHT_TOP, egui::vec2(-12.0, 44.0)).resizable(false).collapsible(true)
            .default_width(330.0).show(ctx, |ui| {
            if self.messung.is_none() {
                self.ui_achse(ui);
            } else {
                if let Some(a) = self.ui_pruefen(ui) {
                    aktion = Some(a);
                }
            }
            ui.separator();
            if self.job.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(&self.meldung);
                });
            } else {
                ui.label(egui::RichText::new(&self.meldung).small());
            }
            ui.label(egui::RichText::new("Laserdaten, DOP20: Bayerische Vermessungsverwaltung (CC BY 4.0)").small().weak());
            if ui.small_button("Werkzeug schliessen").clicked() {
                let _ = self.speichern();
                aktion = Some(Aktion::Schliessen);
            }
        });
        aktion
    }

    fn ui_achse(&mut self, ui: &mut egui::Ui) {
        ui.label("1. Achse klicken: Mitte des Bauwerks, in Fahrtrichtung.\n   Ruecktaste nimmt den letzten Punkt zurueck.");
        ui.label(format!("   {} Punkte, {:.0} m", self.achse.len(), self.achse.windows(2).map(|w| (w[1] - w[0]).truncate().length()).sum::<f64>()));
        ui.add(egui::Slider::new(&mut self.links, 3.0..=40.0).text("m links der Achse"));
        ui.add(egui::Slider::new(&mut self.rechts, 3.0..=40.0).text("m rechts der Achse"));
        ui.horizontal(|ui| {
            ui.label("Name");
            ui.text_edit_singleline(&mut self.name);
        });
        let bereit = self.achse.len() >= 2 && self.job.is_none();
        if ui.add_enabled(bereit, egui::Button::new("2. Vermessen")).on_hover_text("Laserdaten und Luftbild auswerten (laedt fehlende Daten erst nach Rueckfrage)").clicked() {
            self.messen(false);
        }
        if let Some(k) = self.download.clone() {
            let mb: f64 = k.iter().filter_map(|x| x.1).sum();
            ui.colored_label(egui::Color32::from_rgb(230, 160, 40), format!("Laserdaten fehlen: {} Kachel(n) {}, zusammen etwa {mb:.0} MB.",
                k.len(), k.iter().map(|x| x.0.as_str()).collect::<Vec<_>>().join(", ")));
            ui.horizontal(|ui| {
                if ui.button("Herunterladen und vermessen").clicked() {
                    self.messen(true);
                }
                if ui.button("Abbrechen").clicked() {
                    self.download = None;
                }
            });
        }
        if !self.vorhandene.is_empty() {
            ui.separator();
            ui.label("oder eine fruehere Messung oeffnen:");
            let mut oeffnen = None;
            ui.horizontal_wrapped(|ui| {
                for n in &self.vorhandene {
                    if ui.small_button(n).clicked() {
                        oeffnen = Some(n.clone());
                    }
                }
            });
            if let Some(n) = oeffnen {
                self.laden(&n);
            }
        }
    }

    fn anzahl(&self, t: Teil) -> usize {
        self.messung.as_ref().and_then(|m| m[t.feld()].as_array().map(|a| a.len())).unwrap_or(0)
    }

    fn ui_pruefen(&mut self, ui: &mut egui::Ui) -> Option<Aktion> {
        let mut aktion = None;
        ui.label(egui::RichText::new(format!("Messung {}", self.name)).strong());
        ui.label("3. Pruefen: Teile in der 3D-Ansicht anklicken oder hier waehlen.");
        egui::ScrollArea::vertical().max_height(150.0).show(ui, |ui| {
            for (t, titel) in [(Teil::Kante, "Kanten / Waende"), (Teil::Streifen, "Streifen"), (Teil::Markierung, "Markierungen"), (Teil::Sperr, "Sperrflaechen")] {
                let n = self.anzahl(t);
                if n == 0 {
                    continue;
                }
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("{titel}:"));
                    for i in 0..n {
                        let txt = self.kurz(t, i);
                        if ui.selectable_label(self.wahl == Some((t, i)), txt).clicked() {
                            self.wahl = Some((t, i));
                        }
                    }
                });
            }
        });
        ui.separator();
        if let Some((t, i)) = self.wahl {
            let alt = self.messung.as_ref().map(|m| m[t.feld()][i].clone());
            if let Some(m) = self.messung.as_mut() {
                let e = &mut m[t.feld()][i];
                eigenschaften(ui, t, e);
                if let Some(p) = mitte(e) {
                    if ui.small_button("Hinsehen").clicked() {
                        aktion = Some(Aktion::Kamera(p));
                    }
                }
            }
            if self.messung.as_ref().map(|m| m[t.feld()][i].clone()) != alt {
                self.geaendert = true;
            }
        } else {
            ui.label(egui::RichText::new("nichts gewaehlt").weak());
        }
        ui.separator();
        ui.horizontal(|ui| {
            if ui.add_enabled(self.job.is_none(), egui::Button::new("4. Bauen und einsetzen")).on_hover_text("Splines und Objekte bauen und in die Sitzung setzen (Rueckgaengig moeglich, Speichern schreibt die Karte)").clicked() {
                self.bauen();
            }
            if ui.button("Neue Achse").clicked() {
                let _ = self.speichern();
                self.messung = None;
                self.pfad = None;
                self.wahl = None;
                self.achse.clear();
                self.vorhandene = self.liste();
                let mut n = 1;
                while self.vorhandene.contains(&format!("Aufmass_{n}")) {
                    n += 1;
                }
                self.name = format!("Aufmass_{n}");
                self.meldung = "Achse klicken".into();
            }
        });
        if ui.add_enabled(self.job.is_none(), egui::Button::new("neu vermessen (gleiche Achse)")).on_hover_text("Aenderungen an der Messung gehen verloren").clicked() {
            self.messung = None;
            self.messen(false);
        }
        if !self.pruefung.is_empty() {
            ui.label(egui::RichText::new(format!("Baupruefung: {}", self.pruefung)).small());
        }
        aktion
    }

    fn kurz(&self, t: Teil, i: usize) -> String {
        let Some(m) = &self.messung else { return String::new() };
        let e = &m[t.feld()][i];
        let aus = if e["bauen"].as_bool() == Some(false) { " (aus)" } else { "" };
        match t {
            Teil::Kante => format!("{} {:.1} m{aus}", if e["art"] == "trog" { "Trog" } else { "Wand" }, e["hoehe"].as_f64().unwrap_or(0.0)),
            Teil::Streifen => format!("{} {:.1} m", if e["art"] == "fahrbahn" { "Fahrbahn" } else { "Boden" }, e["breite"].as_f64().unwrap_or(0.0)),
            Teil::Markierung => format!("{}{aus}", if e["durch"].as_bool() == Some(true) { "Linie" } else { "Leitlinie" }),
            Teil::Sperr => format!("Sperrflaeche{aus}"),
        }
    }

    // ---------------------------------------------------------------- Zeichnen
    pub fn zeichnen(&mut self, p: &egui::Painter, bild: impl Fn(DVec3) -> Option<egui::Pos2>, hoehe: impl Fn(DVec2) -> f64) {
        self.bild.clear();
        let gelb = egui::Color32::from_rgb(255, 210, 40);
        if self.messung.is_none() {
            // Achse und Korridor
            let a: Vec<DVec2> = self.achse.iter().map(|q| q.truncate()).collect();
            let auf = |q: DVec2| bild(q.extend(hoehe(q) + 0.3));
            let linie = |pts: &[DVec2]| -> Vec<egui::Pos2> {
                let mut out = Vec::new();
                for w in pts.windows(2) {
                    let n = ((w[1] - w[0]).length() / 3.0).ceil().max(1.0) as usize;
                    for k in 0..=n {
                        if let Some(s) = auf(w[0] + (w[1] - w[0]) * (k as f64 / n as f64)) {
                            out.push(s);
                        }
                    }
                }
                out
            };
            p.add(egui::Shape::line(linie(&a), egui::Stroke::new(2.5, gelb)));
            for q in &self.achse {
                if let Some(s) = bild(*q + DVec3::Z * 0.3) {
                    p.circle_filled(s, 4.0, gelb);
                }
            }
            if a.len() >= 2 {
                for (seite, w) in [(-1.0, self.links), (1.0, self.rechts)] {
                    let versetzt: Vec<DVec2> = (0..a.len()).map(|i| {
                        let d = if i + 1 < a.len() { a[i + 1] - a[i] } else { a[i] - a[i - 1] };
                        let d = d.normalize_or_zero();
                        a[i] + DVec2::new(d.y, -d.x) * (seite * w)              // rechts der Laufrichtung = +
                    }).collect();
                    for s in linie(&versetzt).chunks(2) {
                        if s.len() == 2 {
                            p.line_segment([s[0], s[1]], egui::Stroke::new(1.5, gelb.gamma_multiply(0.7)));
                        }
                    }
                }
                // Laufrichtung
                if let (Some(s0), Some(s1)) = (auf(a[a.len() - 2]), auf(a[a.len() - 1])) {
                    p.arrow(s0 + (s1 - s0) * 0.6, (s1 - s0) * 0.4, egui::Stroke::new(2.5, gelb));
                }
            }
            return;
        }
        let Some(m) = &self.messung else { return };
        let punkte = |e: &serde_json::Value, feld: &str| -> Vec<DVec3> {
            e[feld].as_array().map(|a| a.iter().map(|q| DVec3::new(q[0].as_f64().unwrap_or(0.0), q[1].as_f64().unwrap_or(0.0), q[2].as_f64().unwrap_or(0.0))).collect()).unwrap_or_default()
        };
        let mut bild_linien: Vec<(Teil, usize, Vec<egui::Pos2>)> = Vec::new();
        let gewaehlt = self.wahl;
        let mut zeichne = |teil: Teil, i: usize, pts: &[DVec3], farbe: egui::Color32, breite: f32, strich: bool| {
            let l: Vec<egui::Pos2> = pts.iter().filter_map(|q| bild(*q + DVec3::Z * 0.15)).collect();
            let (farbe, breite) = if gewaehlt == Some((teil, i)) { (egui::Color32::from_rgb(80, 255, 255), breite + 2.5) } else { (farbe, breite) };
            if strich {
                for (k, w) in l.windows(2).enumerate() {
                    if k % 3 == 0 {
                        p.line_segment([w[0], w[1]], egui::Stroke::new(breite, farbe));
                    }
                }
            } else {
                p.add(egui::Shape::line(l.clone(), egui::Stroke::new(breite, farbe)));
            }
            bild_linien.push((teil, i, l));
        };
        for (i, s) in m["streifen"].as_array().into_iter().flatten().enumerate() {
            let pts = punkte(s, "mitte");
            let fahrbahn = s["art"] == "fahrbahn";
            let farbe = if fahrbahn { egui::Color32::from_rgb(90, 150, 255) } else { egui::Color32::from_rgb(110, 200, 90) };
            zeichne(Teil::Streifen, i, &pts, farbe, 1.5, true);
            // Richtung als Pfeile
            if fahrbahn && pts.len() > 12 {
                let r = s["richtung"].as_str().unwrap_or("auto");
                for k in (6..pts.len() - 5).step_by(25) {
                    let (Some(a), Some(b)) = (bild(pts[k] + DVec3::Z * 0.15), bild(pts[k + 4] + DVec3::Z * 0.15)) else { continue };
                    match r {
                        "vor" => p.arrow(a, b - a, egui::Stroke::new(2.0, farbe)),
                        "zurueck" => p.arrow(b, a - b, egui::Stroke::new(2.0, farbe)),
                        "beide" => {
                            p.arrow(a, b - a, egui::Stroke::new(2.0, farbe));
                            p.arrow(b, a - b, egui::Stroke::new(2.0, farbe));
                        }
                        _ => {}
                    }
                }
            }
        }
        for (i, k) in m["kanten"].as_array().into_iter().flatten().enumerate() {
            let farbe = if k["bauen"].as_bool() == Some(false) {
                egui::Color32::from_gray(140)
            } else if k["art"] == "trog" {
                egui::Color32::from_rgb(230, 70, 60)
            } else {
                egui::Color32::from_rgb(240, 150, 50)
            };
            zeichne(Teil::Kante, i, &punkte(k, "punkte"), farbe, 3.0, false);
        }
        for (i, k) in m["markierungen"].as_array().into_iter().flatten().enumerate() {
            let farbe = if k["bauen"].as_bool() == Some(false) { egui::Color32::from_gray(120) } else { egui::Color32::WHITE };
            zeichne(Teil::Markierung, i, &punkte(k, "punkte"), farbe, 2.0, k["durch"].as_bool() != Some(true));
        }
        for (i, k) in m["sperrflaechen"].as_array().into_iter().flatten().enumerate() {
            let mut pts = punkte(k, "umriss");
            if let Some(a) = pts.first().copied() {
                pts.push(a);
            }
            let farbe = if k["bauen"].as_bool() == Some(false) { egui::Color32::from_gray(120) } else { gelb };
            zeichne(Teil::Sperr, i, &pts, farbe, 2.0, false);
        }
        self.bild = bild_linien;
    }
}

/// Mitte eines Teils (zum Hinsehen)
fn mitte(e: &serde_json::Value) -> Option<DVec3> {
    let a = ["punkte", "mitte", "umriss"].iter().find_map(|f| e[*f].as_array())?;
    let q = a.get(a.len() / 2)?;
    Some(DVec3::new(q[0].as_f64()?, q[1].as_f64()?, q[2].as_f64()?))
}

fn auswahl(ui: &mut egui::Ui, e: &mut serde_json::Value, feld: &str, titel: &str, werte: &[(&str, &str)]) {
    let jetzt = e[feld].as_str().unwrap_or("").to_string();
    let anzeige = werte.iter().find(|w| w.0 == jetzt).map(|w| w.1).unwrap_or(jetzt.as_str()).to_string();
    egui::ComboBox::from_label(titel).selected_text(anzeige).show_ui(ui, |ui| {
        for (w, t) in werte {
            if ui.selectable_label(jetzt == *w, *t).clicked() {
                e[feld] = serde_json::Value::from(*w);
            }
        }
    });
}

fn schalter(ui: &mut egui::Ui, e: &mut serde_json::Value, feld: &str, titel: &str) {
    let mut b = e[feld].as_bool().unwrap_or(true);
    if ui.checkbox(&mut b, titel).changed() {
        e[feld] = serde_json::Value::from(b);
    }
}

/// Annahmen eines Teils: der Nutzer entscheidet
fn eigenschaften(ui: &mut egui::Ui, t: Teil, e: &mut serde_json::Value) {
    let f = |e: &serde_json::Value, k: &str| e[k].as_f64().unwrap_or(0.0);
    match t {
        Teil::Kante => {
            ui.label(egui::RichText::new(format!("Kante {}: Hoehe {:.2} m (gemessen), Fuss bei {:.2} m", e["id"].as_str().unwrap_or(""), f(e, "hoehe"), f(e, "unten"))).strong());
            schalter(ui, e, "bauen", "Wand bauen");
            auswahl(ui, e, "art", "Art", &[("trog", "Trogwand (Bruestung 0,8 m)"), ("wand", "Stuetzwand / Mauer")]);
            auswahl(ui, e, "material", "Wandflaeche", &[("paneel", "rote Paneele"), ("beton", "Beton")]);
            schalter(ui, e, "gelaender", "Gelaender");
        }
        Teil::Streifen => {
            ui.label(egui::RichText::new(format!("Streifen {}: {:.2} m breit (gemessen)", e["id"].as_str().unwrap_or(""), f(e, "breite"))).strong());
            auswahl(ui, e, "art", "Art", &[("fahrbahn", "Fahrbahn"), ("boden", "Boden (Gras, Gehweg)")]);
            if e["art"] == "fahrbahn" {
                auswahl(ui, e, "richtung", "Richtung", &[("auto", "automatisch"), ("beide", "Gegenverkehr"), ("vor", "Einbahn in Achsrichtung"), ("zurueck", "Einbahn gegen die Achse")]);
                let mut n = e["spuren"].as_i64().unwrap_or(0);
                egui::ComboBox::from_label("Fahrstreifen").selected_text(if n == 0 { "aus Markierungen".to_string() } else { n.to_string() }).show_ui(ui, |ui| {
                    for w in 0..=4 {
                        ui.selectable_value(&mut n, w, if w == 0 { "aus Markierungen".to_string() } else { w.to_string() });
                    }
                });
                if n != e["spuren"].as_i64().unwrap_or(0) {
                    e["spuren"] = serde_json::Value::from(n);
                }
            } else {
                auswahl(ui, e, "material", "Material", &[("auto", "aus dem Luftbild"), ("gras", "Gras"), ("gehweg", "Gehweg")]);
            }
        }
        Teil::Markierung => {
            ui.label(egui::RichText::new(format!("Markierung {} ({} m)", e["id"].as_str().unwrap_or(""), e["t"].as_array().map(|a| a.len()).unwrap_or(0))).strong());
            schalter(ui, e, "bauen", "bauen");
            let mut d = e["durch"].as_bool().unwrap_or(true);
            ui.horizontal(|ui| {
                ui.radio_value(&mut d, true, "durchgehend");
                ui.radio_value(&mut d, false, "Leitlinie (3 m / 6 m)");
            });
            if Some(d) != e["durch"].as_bool() {
                e["durch"] = serde_json::Value::from(d);
            }
        }
        Teil::Sperr => {
            ui.label(egui::RichText::new(format!("Sperrflaeche {} ({} m lang)", e["id"].as_str().unwrap_or(""), e["t"].as_array().map(|a| a.len()).unwrap_or(0))).strong());
            schalter(ui, e, "bauen", "bauen");
        }
    }
}
