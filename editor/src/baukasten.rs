//! Fenster "Querschnitt-Baukasten": Querschnitt aus Teilen zusammenstellen (Skizze, Teileliste mit Breiten, Belag,
//! Richtung, Markierungen), 3D-Vorschau aus openOMSI, speichern als eigene Spline (querschnitt.rs).

use crate::querschnitt::{self, Art, Belag, Marke, Querschnitt, Richtung, Teil};
use openomsi_game::viewer::Viewer;
use std::path::Path;
use std::time::{Duration, Instant};

/// was das Fenster ausloest
pub enum Ergebnis {
    /// gespeichert: (.sli relativ zum OMSI-Ordner, Querschnitt), und gleich damit bauen?
    Gespeichert(String, Querschnitt, bool),
}

pub struct Baukasten {
    pub q: Querschnitt,
    pub wahl: Option<usize>,
    eigene: Vec<(String, Querschnitt)>,
    /// Datei, aus der geoeffnet wurde (Ueberschreiben ohne Rueckfrage)
    geoeffnet: Option<String>,
    vorschau: Option<egui::TextureHandle>,
    /// Vorschau neu rendern ab diesem Zeitpunkt (nach Aenderungen kurz warten)
    faellig: Option<Instant>,
    pub meldung: String,
}

impl Baukasten {
    pub fn neu(root: &Path, start: Option<Querschnitt>) -> Baukasten {
        let geoeffnet = start.as_ref().map(|q| q.datei());
        Baukasten { q: start.unwrap_or_default(), wahl: None, eigene: querschnitt::eigene(root), geoeffnet, vorschau: None,
                    faellig: Some(Instant::now()), meldung: String::new() }
    }

    pub fn hat_vorschau(&self) -> bool {
        self.vorschau.is_some()
    }

    fn geaendert(&mut self) {
        self.q.marken_anpassen();
        self.faellig = Some(Instant::now() + Duration::from_millis(350));
    }

    /// 3D-Vorschau rendern, wenn faellig (ausserhalb des UI-Durchlaufs aufrufen)
    pub fn vorschau_rendern(&mut self, v: &mut Viewer, ctx: &egui::Context, root: &Path) {
        if !self.faellig.is_some_and(|t| Instant::now() >= t) {
            return;
        }
        self.faellig = None;
        if self.q.pruefen().is_err() {
            self.vorschau = None;
            return;
        }
        const DATEI: &str = "_vorschau_baukasten";
        match querschnitt::schreiben(root, &self.q, DATEI) {
            Ok(rel) => {
                v.forget_spline_type(&rel);
                if let Some(px) = v.preview_spline_image(&rel, 360) {
                    let img = egui::ColorImage::from_rgba_unmultiplied([360, 360], &px);
                    self.vorschau = Some(ctx.load_texture("qs_vorschau", img, egui::TextureOptions::LINEAR));
                }
                v.forget_spline_type(&rel);
                let _ = std::fs::remove_file(querschnitt::ordner(root).join(format!("{DATEI}.sli")));
            }
            Err(e) => self.meldung = format!("Vorschau: {e:#}"),
        }
    }

    /// das Fenster; `offen` false = schliessen; `messung`: letzte Messung (m) fuer "Breite uebernehmen"
    pub fn ui(&mut self, ctx: &egui::Context, root: &Path, offen: &mut bool, messung: Option<f64>) -> Option<Ergebnis> {
        let mut erg = None;
        egui::Window::new("Querschnitt-Baukasten").open(offen).default_size([820.0, 640.0]).resizable(true).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Name");
                if ui.add(egui::TextEdit::singleline(&mut self.q.name).desired_width(240.0)).changed() {
                    self.geoeffnet = None;
                }
                ui.label(egui::RichText::new(format!("-> Splines\\{}\\{}.sli", crate::speichern::EIGEN, self.q.datei())).weak().small());
                egui::ComboBox::from_id_salt("qs_oeffnen").selected_text("Oeffnen ...").show_ui(ui, |ui| {
                    if ui.selectable_label(false, "Neu (Stadtstrasse)").clicked() {
                        self.q = Querschnitt::default();
                        self.geoeffnet = None;
                        self.wahl = None;
                        self.geaendert();
                    }
                    for (datei, q) in self.eigene.clone() {
                        if ui.selectable_label(false, &q.name).on_hover_text(&datei).clicked() {
                            self.q = q;
                            self.geoeffnet = Some(datei);
                            self.wahl = None;
                            self.geaendert();
                        }
                    }
                });
            });
            ui.separator();
            if self.skizze(ui) {
                self.geaendert();
            }
            let (vor, zur, geh) = self.q.spuren();
            ui.label(format!("Gesamtbreite {:.2} m  |  Fahrspuren {} vor, {} zurueck  |  Gehwege {}", self.q.gesamtbreite(), vor, zur, geh));
            ui.separator();
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_min_width(440.0);
                    if self.teileliste(ui, messung) {
                        self.geaendert();
                    }
                });
                ui.vertical(|ui| {
                    match &self.vorschau {
                        Some(t) => {
                            ui.image((t.id(), egui::vec2(300.0, 300.0)));
                        }
                        None => {
                            ui.allocate_space(egui::vec2(300.0, 300.0));
                        }
                    }
                    ui.label(egui::RichText::new("3D-Vorschau (so zeichnet openOMSI die Spline)").small().weak());
                });
            });
            ui.separator();
            if let Err(e) = self.q.pruefen() {
                ui.colored_label(egui::Color32::from_rgb(255, 120, 90), format!("{e:#}"));
            }
            let vorhanden = querschnitt::ordner(root).join(format!("{}.sli", self.q.datei())).is_file();
            let fremd = vorhanden && !querschnitt::ordner(root).join(format!("{}.qs.json", self.q.datei())).is_file();
            if vorhanden && self.geoeffnet.as_deref() != Some(self.q.datei().as_str()) {
                ui.colored_label(egui::Color32::from_rgb(255, 200, 80), if fremd { "Diese Datei gibt es schon (nicht aus dem Baukasten) - anderen Namen waehlen." } else { "Ein eigener Querschnitt mit diesem Namen wird ueberschrieben." });
            }
            ui.horizontal(|ui| {
                let ok = self.q.pruefen().is_ok() && !fremd;
                for (text, bauen) in [("Speichern", false), ("Speichern und damit bauen", true)] {
                    if ui.add_enabled(ok, egui::Button::new(text)).clicked() {
                        match querschnitt::speichern(root, &self.q) {
                            Ok(rel) => {
                                self.meldung = format!("gespeichert: {rel}");
                                self.geoeffnet = Some(self.q.datei());
                                self.eigene = querschnitt::eigene(root);
                                erg = Some(Ergebnis::Gespeichert(rel, self.q.clone(), bauen));
                            }
                            Err(e) => self.meldung = format!("nicht gespeichert: {e:#}"),
                        }
                    }
                }
                ui.label(egui::RichText::new(&self.meldung).small());
            });
            ui.label(egui::RichText::new("Teile von links nach rechts in Fahrtrichtung des Splines. \"vor\" faehrt mit dem Spline (Rechtsverkehr: rechts der Achse). Die rote Linie in der Skizze ist die Spline-Achse (Mitte der Fahrspuren). Schon gebaute Strassen mit einem geaenderten Querschnitt zeigen die Aenderung nach dem Neuladen der Karte.").small().weak());
        });
        erg
    }

    /// Querschnitt als Skizze: Teile massstaeblich, Hochbord hoeher, Richtungspfeile, Markierungen, Achse;
    /// Klick waehlt ein Teil. true wenn sich etwas geaendert hat (hier nie, nur Auswahl)
    fn skizze(&mut self, ui: &mut egui::Ui) -> bool {
        let breite = ui.available_width().max(200.0);
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(breite, 96.0), egui::Sense::click());
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 4.0, egui::Color32::from_gray(28));
        let gesamt = self.q.gesamtbreite().max(1.0);
        let m = (rect.width() - 20.0) as f64 / gesamt;
        let x0 = rect.left() + 10.0;
        let boden = rect.bottom() - 18.0;
        let mut x = 0.0;
        let mut klick = None;
        for (i, t) in self.q.teile.iter().enumerate() {
            let (a, b) = (x0 + (x * m) as f32, x0 + ((x + t.breite) * m) as f32);
            let h = if t.art.unten() { 18.0 } else { 30.0 };
            let r = egui::Rect::from_min_max(egui::pos2(a, boden - h), egui::pos2(b, boden));
            let [cr, cg, cb] = t.art.farbe();
            p.rect_filled(r, 0.0, egui::Color32::from_rgb(cr, cg, cb));
            if self.wahl == Some(i) {
                p.rect_stroke(r, 0.0, egui::Stroke::new(2.5, egui::Color32::from_rgb(255, 60, 220)), egui::StrokeKind::Inside);
            }
            if t.art.spur() {
                let pfeil = if t.richtung == Richtung::Vor { "\u{25B2}" } else { "\u{25BC}" };
                p.text(r.center() - egui::vec2(0.0, 22.0), egui::Align2::CENTER_CENTER, pfeil, egui::FontId::proportional(16.0), egui::Color32::WHITE);
            }
            p.text(egui::pos2((a + b) / 2.0, boden + 9.0), egui::Align2::CENTER_CENTER, format!("{:.2}", t.breite), egui::FontId::proportional(11.0), egui::Color32::GRAY);
            if let Some(pos) = resp.interact_pointer_pos() {
                if resp.clicked() && pos.x >= a && pos.x < b {
                    klick = Some(i);
                }
            }
            x += t.breite;
        }
        // Markierungen
        let mut x = 0.0;
        for i in 0..self.q.teile.len().saturating_sub(1) {
            x += self.q.teile[i].breite;
            if !(self.q.teile[i].art.unten() && self.q.teile[i + 1].art.unten()) {
                continue;
            }
            let px = x0 + (x * m) as f32;
            let (y0, y1) = (boden - 18.0, boden);
            let weiss = egui::Stroke::new(2.0, egui::Color32::WHITE);
            match self.q.marke(i) {
                Marke::Strich => {
                    for k in 0..3 {
                        let ya = y0 + k as f32 * 6.0;
                        p.line_segment([egui::pos2(px, ya), egui::pos2(px, ya + 3.0)], weiss);
                    }
                }
                Marke::Voll => {
                    p.line_segment([egui::pos2(px, y0), egui::pos2(px, y1)], weiss);
                }
                Marke::Breitstrich => {
                    p.line_segment([egui::pos2(px, y0), egui::pos2(px, y1)], egui::Stroke::new(4.0, egui::Color32::WHITE));
                }
                Marke::Doppelt => {
                    p.line_segment([egui::pos2(px - 2.5, y0), egui::pos2(px - 2.5, y1)], weiss);
                    p.line_segment([egui::pos2(px + 2.5, y0), egui::pos2(px + 2.5, y1)], weiss);
                }
                _ => {}
            }
        }
        // Achse
        let ax = x0 + ((-self.q.links()) * m) as f32;
        p.line_segment([egui::pos2(ax, rect.top() + 4.0), egui::pos2(ax, boden)], egui::Stroke::new(1.5, egui::Color32::from_rgb(255, 70, 70)));
        if let Some(i) = klick {
            self.wahl = Some(i);
        }
        false
    }

    /// Liste der Teile mit allen Einstellungen; true wenn geaendert
    fn teileliste(&mut self, ui: &mut egui::Ui, messung: Option<f64>) -> bool {
        let mut geaendert = false;
        let mut tausch: Option<(usize, usize)> = None;
        let mut weg: Option<usize> = None;
        let n = self.q.teile.len();
        egui::ScrollArea::vertical().id_salt("qs_teile").max_height(330.0).show(ui, |ui| {
            for i in 0..n {
                let gewaehlt = self.wahl == Some(i);
                let rahmen = egui::Frame::new().inner_margin(3.0).fill(if gewaehlt { egui::Color32::from_rgb(60, 40, 60) } else { egui::Color32::TRANSPARENT });
                rahmen.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui.selectable_label(gewaehlt, format!("{}", i + 1)).clicked() {
                            self.wahl = Some(i);
                        }
                        let t = &mut self.q.teile[i];
                        egui::ComboBox::from_id_salt(("qs_art", i)).width(130.0).selected_text(t.art.name()).show_ui(ui, |ui| {
                            for a in Art::ALLE {
                                if ui.selectable_label(t.art == a, a.name()).clicked() && t.art != a {
                                    let r = t.richtung;
                                    *t = Teil::neu(a);
                                    t.richtung = r;
                                    geaendert = true;
                                }
                            }
                        });
                        if ui.add(egui::DragValue::new(&mut t.breite).speed(0.05).range(0.3..=20.0).fixed_decimals(2).suffix(" m")).changed() {
                            geaendert = true;
                        }
                        if t.art.spur() {
                            let text = if t.richtung == Richtung::Vor { "\u{25B2} vor" } else { "\u{25BC} zurueck" };
                            if ui.button(text).on_hover_text("Fahrtrichtung: mit oder gegen den Spline").clicked() {
                                t.richtung = if t.richtung == Richtung::Vor { Richtung::Zurueck } else { Richtung::Vor };
                                geaendert = true;
                            }
                        }
                        egui::ComboBox::from_id_salt(("qs_belag", i)).width(110.0).selected_text(t.belag.name()).show_ui(ui, |ui| {
                            for b in Belag::ALLE {
                                if ui.selectable_label(t.belag == b, b.name()).clicked() {
                                    t.belag = b;
                                    geaendert = true;
                                }
                            }
                        });
                        if i > 0 && ui.small_button("\u{2190}").on_hover_text("nach links").clicked() {
                            tausch = Some((i, i - 1));
                        }
                        if i + 1 < n && ui.small_button("\u{2192}").on_hover_text("nach rechts").clicked() {
                            tausch = Some((i, i + 1));
                        }
                        if ui.small_button("\u{2715}").on_hover_text("entfernen").clicked() {
                            weg = Some(i);
                        }
                    });
                });
                // Markierung zur naechsten Grenze
                if i + 1 < n && self.q.teile[i].art.unten() && self.q.teile[i + 1].art.unten() {
                    ui.horizontal(|ui| {
                        ui.add_space(24.0);
                        ui.label(egui::RichText::new("Markierung:").small().weak());
                        let m = &mut self.q.marken[i];
                        let jetzt = if *m == Marke::Auto { format!("automatisch ({})", querschnitt::auto_marke(&self.q.teile[i], &self.q.teile[i + 1]).name()) } else { m.name().to_string() };
                        egui::ComboBox::from_id_salt(("qs_marke", i)).width(200.0).selected_text(jetzt).show_ui(ui, |ui| {
                            for k in Marke::ALLE {
                                if ui.selectable_label(*m == k, k.name()).clicked() {
                                    *m = k;
                                    geaendert = true;
                                }
                            }
                        });
                    });
                }
            }
        });
        if let Some((a, b)) = tausch {
            self.q.teile.swap(a, b);
            if self.wahl == Some(a) {
                self.wahl = Some(b);
            }
            geaendert = true;
        }
        if let Some(i) = weg {
            self.q.teile.remove(i);
            if i < self.q.marken.len() {
                self.q.marken.remove(i);
            }
            self.wahl = None;
            geaendert = true;
        }
        ui.horizontal_wrapped(|ui| {
            ui.label("Teil hinzufuegen:");
            for a in Art::ALLE {
                if ui.small_button(a.name()).on_hover_text("rechts neben dem gewaehlten Teil (sonst ganz rechts)").clicked() {
                    let i = self.wahl.map(|w| w + 1).unwrap_or(self.q.teile.len()).min(self.q.teile.len());
                    let mut t = Teil::neu(a);
                    // links der Achse gegen den Spline
                    if a.spur() && (i as f64) < self.q.teile.len() as f64 / 2.0 {
                        t.richtung = Richtung::Zurueck;
                    }
                    self.q.teile.insert(i, t);
                    self.q.marken.insert(i.min(self.q.marken.len()), Marke::Auto);
                    self.wahl = Some(i);
                    geaendert = true;
                }
            }
        });
        if let Some(mm) = messung {
            ui.horizontal(|ui| {
                ui.label(format!("Messung: {mm:.2} m"));
                if let Some(w) = self.wahl {
                    if ui.button(format!("als Breite von Teil {}", w + 1)).clicked() {
                        self.q.teile[w].breite = (mm * 100.0).round() / 100.0;
                        geaendert = true;
                    }
                }
                if ui.button("Gesamtbreite darauf strecken").on_hover_text("alle Teile im gleichen Verhaeltnis").clicked() && self.q.gesamtbreite() > 0.0 {
                    let f = mm / self.q.gesamtbreite();
                    for t in &mut self.q.teile {
                        t.breite = ((t.breite * f) * 100.0).round() / 100.0;
                    }
                    geaendert = true;
                }
            });
        } else {
            ui.label(egui::RichText::new("Tipp: Messen (M) im Luftbild - die Messung laesst sich hier als Breite uebernehmen.").small().weak());
        }
        geaendert
    }
}
