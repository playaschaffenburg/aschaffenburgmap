//! Flaechen-Markierung (Taste L): auf der Karte eine Flaeche markieren - Rechteck aufziehen oder Vieleck klicken - und
//! alles darin auf einmal loeschen (Objekte, Baeume, Strassen-Splines; was mitgeht, ist waehlbar). Geloescht wird in
//! der Sitzung mit Rueckgaengig wie gewohnt (Objekte ein Schritt, Strassen ein Schritt).

use glam::{DVec2, DVec3};

#[derive(Default)]
pub struct Markierer {
    pub an: bool,
    /// Eckpunkte des Vielecks, das gerade geklickt wird (Welt)
    pub punkte: Vec<DVec3>,
    /// Rechteck: Bildschirmpunkt beim Druecken und aktueller
    druck: Option<(f32, f32)>,
    jetzt: Option<(f32, f32)>,
    /// fertige Flaeche (Welt, x/y) und was darin liegt
    pub flaeche: Vec<DVec2>,
    pub objekte: Vec<crate::bearbeiten::Wahl>,
    pub splines: Vec<i64>,
    /// Auswahl der Arten
    pub mit_objekten: bool,
    pub mit_strassen: bool,
    /// Strassen nur, wenn sie ganz drin liegen (sonst: wenn ihre Mitte drin liegt)
    pub ganz: bool,
}

pub fn punkt_in(p: DVec2, poly: &[DVec2]) -> bool {
    let mut innen = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            innen = !innen;
        }
    }
    innen
}

impl Markierer {
    pub fn neu() -> Markierer {
        Markierer { mit_objekten: true, mit_strassen: true, ..Default::default() }
    }

    pub fn umschalten(&mut self) {
        self.an = !self.an;
        self.abbrechen();
    }

    pub fn abbrechen(&mut self) {
        self.punkte.clear();
        self.druck = None;
        self.jetzt = None;
    }

    pub fn leeren(&mut self) {
        self.abbrechen();
        self.flaeche.clear();
        self.objekte.clear();
        self.splines.clear();
    }

    pub fn hat_auswahl(&self) -> bool {
        !self.flaeche.is_empty()
    }

    pub fn zeichnet(&self) -> bool {
        !self.punkte.is_empty() || self.druck.is_some()
    }

    pub fn druecken(&mut self, maus: (f32, f32)) {
        self.druck = Some(maus);
        self.jetzt = Some(maus);
    }

    pub fn bewegen(&mut self, maus: (f32, f32)) {
        if self.druck.is_some() {
            self.jetzt = Some(maus);
        }
    }

    fn rechteck_gezogen(&self) -> Option<((f32, f32), (f32, f32))> {
        let (a, b) = (self.druck?, self.jetzt?);
        ((a.0 - b.0).abs() + (a.1 - b.1).abs() > 8.0).then_some((a, b))
    }

    /// Loslassen: Rechteck fertig oder Klick (Vieleck-Punkt; Klick auf den ersten Punkt schliesst) -> true, wenn eine
    /// Flaeche fertig ist. `strahl`: Bildschirmpunkt -> Boden; `bild`: Welt -> Bildschirm
    pub fn loslassen(&mut self, maus: (f32, f32), strahl: impl Fn((f32, f32)) -> Option<DVec3>, bild: impl Fn(DVec3) -> Option<(f32, f32)>) -> bool {
        let rechteck = self.rechteck_gezogen();
        self.druck = None;
        self.jetzt = None;
        if let Some((a, b)) = rechteck {
            let ecken = [(a.0, a.1), (b.0, a.1), (b.0, b.1), (a.0, b.1)];
            let pts: Vec<DVec2> = ecken.iter().filter_map(|e| strahl(*e)).map(|p| p.truncate()).collect();
            if pts.len() == 4 {
                self.flaeche = pts;
                self.punkte.clear();
                return true;
            }
            return false;
        }
        // Klick: erster Punkt nahe am Mauszeiger -> schliessen
        if self.punkte.len() >= 3 {
            if let Some(q) = bild(self.punkte[0]) {
                if (q.0 - maus.0).abs() + (q.1 - maus.1).abs() < 14.0 {
                    return self.schliessen();
                }
            }
        }
        if let Some(g) = strahl(maus) {
            self.punkte.push(g);
        }
        false
    }

    /// Vieleck schliessen (Enter) -> true, wenn eine Flaeche fertig ist
    pub fn schliessen(&mut self) -> bool {
        if self.punkte.len() < 3 {
            return false;
        }
        self.flaeche = self.punkte.iter().map(|p| p.truncate()).collect();
        self.punkte.clear();
        true
    }

    /// was in der Flaeche liegt bestimmen
    pub fn berechnen(&mut self, objekte: &[crate::bearbeiten::Objekt], splines: &[(i64, Vec<DVec2>)]) {
        self.objekte.clear();
        self.splines.clear();
        if self.flaeche.len() < 3 {
            return;
        }
        if self.mit_objekten {
            self.objekte = objekte.iter().filter(|o| punkt_in(o.pos.truncate(), &self.flaeche)).map(|o| o.wahl).collect();
        }
        if self.mit_strassen {
            for (id, pts) in splines {
                if pts.is_empty() {
                    continue;
                }
                let drin = if self.ganz { pts.iter().all(|p| punkt_in(*p, &self.flaeche)) } else { punkt_in(pts[pts.len() / 2], &self.flaeche) };
                if drin {
                    self.splines.push(*id);
                }
            }
        }
    }

    /// Flaeche, laufendes Vieleck/Rechteck und Treffer zeichnen
    pub fn zeichnen(&self, p: &egui::Painter, bild: impl Fn(DVec3) -> Option<(f32, f32)>, hoehe: impl Fn(DVec2) -> f64,
                    objekte: &[crate::bearbeiten::Objekt], splines: &[(i64, Vec<DVec2>)]) {
        let gelb = egui::Color32::from_rgb(255, 210, 40);
        let pos = |q: (f32, f32)| egui::pos2(q.0, q.1);
        // fertige Flaeche (Kanten fein unterteilt, damit sie dem Gelaende folgen)
        if self.flaeche.len() >= 3 {
            let n = self.flaeche.len();
            let mut linie = Vec::new();
            for i in 0..=n {
                let (a, b) = (self.flaeche[i % n], self.flaeche[(i + 1) % n]);
                let teile = ((b - a).length() / 4.0).ceil().max(1.0) as usize;
                for k in 0..teile {
                    let q = a + (b - a) * (k as f64 / teile as f64);
                    if let Some(s) = bild(q.extend(hoehe(q) + 0.3)) {
                        linie.push(pos(s));
                    }
                }
                if i == n {
                    break;
                }
            }
            p.add(egui::Shape::line(linie, egui::Stroke::new(2.5, gelb)));
            for o in objekte.iter().filter(|o| self.objekte.contains(&o.wahl)) {
                if let Some(s) = bild(o.pos + DVec3::Z * 0.5) {
                    p.circle_filled(pos(s), 4.0, egui::Color32::from_rgb(255, 80, 60));
                }
            }
            for (id, pts) in splines.iter().filter(|s| self.splines.contains(&s.0)) {
                let _ = id;
                let l: Vec<egui::Pos2> = pts.iter().filter_map(|q| bild(q.extend(hoehe(*q) + 0.3))).map(pos).collect();
                p.add(egui::Shape::line(l, egui::Stroke::new(3.0, egui::Color32::from_rgb(255, 80, 60))));
            }
        }
        // laufendes Vieleck
        if !self.punkte.is_empty() {
            let l: Vec<egui::Pos2> = self.punkte.iter().filter_map(|q| bild(*q + DVec3::Z * 0.3)).map(pos).collect();
            p.add(egui::Shape::line(l.clone(), egui::Stroke::new(2.0, gelb)));
            for (i, q) in l.iter().enumerate() {
                p.circle_filled(*q, if i == 0 { 6.0 } else { 4.0 }, gelb);
            }
        }
        // Rechteck
        if let Some((a, b)) = self.rechteck_gezogen() {
            let r = egui::Rect::from_two_pos(pos(a), pos(b));
            p.rect_filled(r, 0.0, egui::Color32::from_rgba_unmultiplied(255, 210, 40, 30));
            p.rect_stroke(r, 0.0, egui::Stroke::new(1.5, gelb), egui::StrokeKind::Middle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auswahl_in_der_flaeche() {
        let mut m = Markierer::neu();
        m.punkte = vec![DVec3::new(0.0, 0.0, 0.0), DVec3::new(10.0, 0.0, 0.0), DVec3::new(10.0, 10.0, 0.0), DVec3::new(0.0, 10.0, 0.0)];
        assert!(m.schliessen());
        let o = |id: i64, x: f64, y: f64| crate::bearbeiten::Objekt { wahl: crate::bearbeiten::Wahl::Karte(id), pos: DVec3::new(x, y, 0.0), richtung: 0.0, sco: Default::default() };
        let objekte = vec![o(1, 5.0, 5.0), o(2, 15.0, 5.0)];
        let splines = vec![(7, vec![DVec2::new(-5.0, 5.0), DVec2::new(5.0, 5.0), DVec2::new(8.0, 5.0)]), (8, vec![DVec2::new(20.0, 0.0), DVec2::new(30.0, 0.0)])];
        m.berechnen(&objekte, &splines);
        assert_eq!(m.objekte, vec![crate::bearbeiten::Wahl::Karte(1)]);
        assert_eq!(m.splines, vec![7]);
        m.ganz = true;
        m.berechnen(&objekte, &splines);
        assert!(m.splines.is_empty());
        m.mit_objekten = false;
        m.berechnen(&objekte, &splines);
        assert!(m.objekte.is_empty());
    }
}
