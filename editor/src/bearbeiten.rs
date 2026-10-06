//! Objekte auswaehlen und bearbeiten (Meilenstein 2) - auf openOMSIs Objekt-Editor aufgebaut, der die
//! Aenderungen live auf der Grafikkarte zeigt und die Kacheldateien beim Speichern umschreibt.
//!
//! Klick waehlt das Objekt unter der Maus, Ziehen schiebt es ueber das Gelaende, Strg+Rad oder , / . dreht es,
//! Bild auf/ab hebt/senkt es, Entf loescht, Strg+Z / Strg+Y nimmt zurueck / wiederholt.

use crate::kamera::Kamera;
use glam::{DVec3, Vec4};
use openomsi_game::viewer::{Action, Editor, ObjectEdit, Viewer};
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Werkzeug {
    Ansehen,
    Objekte,
}

/// Ein Schritt fuer Rueckgaengig: ein Objekt vorher/nachher
#[derive(Clone, Copy, Debug)]
struct Schritt {
    id: i64,
    vorher: ObjectEdit,
    nachher: ObjectEdit,
}

/// Objekt im Bild: id, Standort, Richtung (Grad), Datei
#[derive(Clone, Debug)]
pub struct Objekt {
    pub id: i64,
    pub pos: DVec3,
    pub richtung: f64,
    pub sco: PathBuf,
}

pub struct Bearbeiten {
    pub ed: Editor,
    pub werkzeug: Werkzeug,
    /// Objekt unter der Maus (Vorschau der Auswahl)
    pub unter_maus: Option<i64>,
    /// Ziehen: (id, Versatz Objekt - Bodenpunkt beim Greifen, Zustand vorher)
    ziehen: Option<(i64, DVec3, ObjectEdit)>,
    undo: Vec<Schritt>,
    redo: Vec<Schritt>,
    /// Anzahl Aenderungen seit dem Oeffnen (fuers Speichern)
    pub aenderungen: usize,
}

impl Default for Bearbeiten {
    fn default() -> Self {
        Bearbeiten { ed: Editor::default(), werkzeug: Werkzeug::Ansehen, unter_maus: None, ziehen: None, undo: vec![],
                     redo: vec![], aenderungen: 0 }
    }
}

/// Weltpunkt -> Bildschirmpunkt (Pixel) und Tiefe, oder None hinter der Kamera
pub fn projizieren(kam: &Kamera, p: DVec3, breite: f32, hoehe: f32) -> Option<(f32, f32, f32)> {
    let cam = kam.camera();
    let vp = cam.view_proj(breite / hoehe.max(1.0), cam.position);
    let r = (p - cam.position).as_vec3();
    let c = vp * Vec4::new(r.x, r.y, r.z, 1.0);
    if c.w <= 0.01 {
        return None;
    }
    Some(((c.x / c.w + 1.0) * 0.5 * breite, (1.0 - c.y / c.w) * 0.5 * hoehe, c.w))
}

impl Bearbeiten {
    /// frisch (nach Kartenwechsel), Werkzeug bleibt
    pub fn neu(werkzeug: Werkzeug) -> Self {
        Bearbeiten { werkzeug, ..Default::default() }
    }

    pub fn ausgewaehlt(&self) -> Option<i64> {
        if self.ed.editing_added.is_some() {
            return None;
        }
        self.ed.selected
    }

    pub fn objekte(v: &Viewer) -> Vec<Objekt> {
        v.objects().into_iter().map(|(id, pos, richtung, sco)| Objekt { id, pos, richtung, sco }).collect()
    }

    /// Objekt unter dem Mauszeiger: naechstes am Bildschirm (Fuss oder Mitte, 28 px), bei Gleichstand das naehere
    pub fn suchen(v: &Viewer, kam: &Kamera, maus: (f32, f32), breite: f32, hoehe: f32) -> Option<i64> {
        let mut best: Option<(f32, i64)> = None;
        for (id, pos, _, _) in v.objects() {
            if (pos - kam.ziel).length() > kam.abstand * 4.0 + 300.0 {
                continue;
            }
            for dz in [0.3, 2.0] {
                let Some((x, y, tiefe)) = projizieren(kam, pos + DVec3::Z * dz, breite, hoehe) else { continue };
                let d = ((x - maus.0).powi(2) + (y - maus.1).powi(2)).sqrt();
                if d < 28.0 {
                    let wert = d + tiefe * 0.02;
                    if best.map(|b| wert < b.0).unwrap_or(true) {
                        best = Some((wert, id));
                    }
                }
            }
        }
        best.map(|b| b.1)
    }

    pub fn waehlen(&mut self, id: Option<i64>) {
        self.ed.editing_added = None;
        self.ed.selected = id;
    }

    /// Aktion auf das gewaehlte Objekt, mit Rueckgaengig-Schritt
    pub fn aktion(&mut self, v: &mut Viewer, a: Action) -> Option<String> {
        if self.ed.editing_added.is_some() {
            let r = v.edit(&mut self.ed, &a);
            self.aenderungen += 1;
            return r;
        }
        let id = self.ed.selected?;
        let vorher = v.object_edit(id);
        let r = v.edit(&mut self.ed, &a);
        let nachher = v.object_edit(id);
        if nachher != vorher {
            self.undo.push(Schritt { id, vorher, nachher });
            self.redo.clear();
            self.aenderungen += 1;
        }
        r
    }

    /// Objekt in einen bestimmten Zustand bringen (fuer Rueckgaengig/Wiederholen)
    fn setzen(&mut self, v: &mut Viewer, id: i64, z: ObjectEdit) {
        self.ed.editing_added = None;
        self.ed.selected = Some(id);
        v.edit(&mut self.ed, &Action::Undo);
        if z.moved != DVec3::ZERO {
            v.edit(&mut self.ed, &Action::Move(z.moved));
        }
        if z.turned != 0.0 {
            v.edit(&mut self.ed, &Action::Turn(z.turned));
        }
        if z.deleted {
            v.edit(&mut self.ed, &Action::Delete);
        }
    }

    pub fn rueckgaengig(&mut self, v: &mut Viewer) -> Option<String> {
        let s = self.undo.pop()?;
        self.setzen(v, s.id, s.vorher);
        self.redo.push(s);
        self.aenderungen += 1;
        Some(format!("rueckgaengig: Objekt {}", s.id))
    }

    pub fn wiederholen(&mut self, v: &mut Viewer) -> Option<String> {
        let s = self.redo.pop()?;
        self.setzen(v, s.id, s.nachher);
        self.undo.push(s);
        self.aenderungen += 1;
        Some(format!("wiederholt: Objekt {}", s.id))
    }

    pub fn kann_rueckgaengig(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn kann_wiederholen(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Greifen: Maustaste ueber dem gewaehlten Objekt gedrueckt
    pub fn greifen(&mut self, v: &Viewer, id: i64, boden: DVec3) {
        let pos = v.objects().into_iter().find(|o| o.0 == id).map(|o| o.1).unwrap_or(boden);
        self.ziehen = Some((id, DVec3::new(pos.x - boden.x, pos.y - boden.y, 0.0), v.object_edit(id)));
    }

    pub fn zieht(&self) -> bool {
        self.ziehen.is_some()
    }

    /// Ziehen: Fuss des Objekts auf den Boden unter der Maus (plus Versatz vom Greifen)
    pub fn ziehen_nach(&mut self, v: &mut Viewer, boden: DVec3) -> Option<String> {
        let (id, versatz, _) = self.ziehen?;
        self.ed.editing_added = None;
        self.ed.selected = Some(id);
        let (x, y) = (boden.x + versatz.x, boden.y + versatz.y);
        let z = v.terrain_height(x, y).unwrap_or(boden.z);
        v.drag(&mut self.ed, DVec3::new(x, y, z))
    }

    pub fn loslassen(&mut self, v: &Viewer) {
        if let Some((id, _, vorher)) = self.ziehen.take() {
            let nachher = v.object_edit(id);
            if nachher != vorher {
                self.undo.push(Schritt { id, vorher, nachher });
                self.redo.clear();
                self.aenderungen += 1;
            }
        }
    }
}

/// Auswahl im Bild markieren: Ring am Boden und Richtungspfeil (egui-Malfunktionen, ohne Tiefentest)
pub fn markieren(p: &egui::Painter, kam: &Kamera, o: &Objekt, breite: f32, hoehe: f32, farbe: egui::Color32, dick: f32) {
    let r = 2.5;
    let mut ring = Vec::new();
    for k in 0..=32 {
        let a = k as f64 / 32.0 * std::f64::consts::TAU;
        if let Some((x, y, _)) = projizieren(kam, o.pos + DVec3::new(a.cos() * r, a.sin() * r, 0.1), breite, hoehe) {
            ring.push(egui::pos2(x, y));
        }
    }
    if ring.len() > 2 {
        p.add(egui::Shape::line(ring, egui::Stroke::new(dick, farbe)));
    }
    let (s, c) = o.richtung.to_radians().sin_cos();
    let spitze = o.pos + DVec3::new(s * r * 1.8, c * r * 1.8, 0.1);
    if let (Some(a), Some(b)) = (projizieren(kam, o.pos + DVec3::Z * 0.1, breite, hoehe), projizieren(kam, spitze, breite, hoehe)) {
        p.arrow(egui::pos2(a.0, a.1), egui::vec2(b.0 - a.0, b.1 - a.1), egui::Stroke::new(dick, farbe));
    }
}

#[cfg(test)]
mod tests {
    //! Mit echter Karte und Grafikkarte (ohne Fenster): cargo test --release -- --ignored
    use super::*;
    use std::path::Path;

    const OMSI: &str = r"C:\Program Files (x86)\Steam\steamapps\common\OMSI 2";

    #[test]
    #[ignore]
    fn objekt_bearbeiten_und_als_neue_karte_speichern() {
        let root = Path::new(OMSI);
        let instance = openomsi_game::viewer::instance();
        let (mut v, _) = Viewer::open(&instance, None, root, &root.join("maps/Grundorf/global.cfg")).unwrap();
        v.tiles_around(DVec3::new(150.0, 150.0, 0.0), 1).unwrap();
        let objekte = Bearbeiten::objekte(&v);
        assert!(objekte.len() > 20, "{} Objekte", objekte.len());
        let o = objekte.iter().min_by_key(|o| o.id).unwrap().clone();
        let mut b = Bearbeiten::neu(Werkzeug::Objekte);
        b.waehlen(Some(o.id));
        b.aktion(&mut v, Action::Move(DVec3::new(3.0, -2.0, 0.0)));
        b.aktion(&mut v, Action::Turn(15.0));
        let e = v.object_edit(o.id);
        assert!((e.moved.x - 3.0).abs() < 1e-9 && (e.turned - 15.0).abs() < 1e-9);
        b.rueckgaengig(&mut v);
        assert_eq!(v.object_edit(o.id).turned, 0.0);
        b.wiederholen(&mut v);
        assert_eq!(v.object_edit(o.id), e);
        let neu_pos = Bearbeiten::objekte(&v).into_iter().find(|x| x.id == o.id).unwrap().pos;
        assert!((neu_pos.x - o.pos.x - 3.0).abs() < 1e-6);
        // Kachel schreiben: das Objekt hat neue Koordinaten, sonst bleibt alles
        let (_, dateien) = crate::speichern::kacheln_schreiben(&v, &b.ed, "Grundorf").unwrap();
        assert_eq!(dateien.len(), 1, "{dateien:?}");
        // als neue Karte in einem Test-OMSI-Ordner (die echte Installation bleibt unberuehrt)
        let test_root = std::env::temp_dir().join(format!("omsi-editor-root-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&test_root);
        std::fs::create_dir_all(test_root.join("maps")).unwrap();
        let kopie = test_root.join("maps/Grundorf");
        fn kopieren(a: &Path, b: &Path) {
            std::fs::create_dir_all(b).unwrap();
            for e in std::fs::read_dir(a).unwrap().flatten() {
                if e.file_type().unwrap().is_dir() { kopieren(&e.path(), &b.join(e.file_name())) } else { std::fs::copy(e.path(), b.join(e.file_name())).unwrap(); }
            }
        }
        kopieren(&root.join("maps/Grundorf"), &kopie);
        let ziel = crate::speichern::karte_anlegen(&test_root, "Grundorf", "Grundorf_test", &dateien).unwrap();
        let name = dateien[0].file_name().unwrap();
        let alt = std::fs::read(root.join("maps/Grundorf").join(name)).unwrap();
        let neu = std::fs::read(ziel.join(name)).unwrap();
        assert_ne!(alt, neu);
        assert_eq!(&neu[..2], &[0xFF, 0xFE], "UTF-16 bleibt");
        let text = String::from_utf16_lossy(&neu[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>());
        assert!(text.contains(&format!("\r\n{}\r\n", o.id)));
        let g = std::fs::read(ziel.join("global.cfg")).unwrap();
        let g = String::from_utf16_lossy(&g[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>());
        assert!(g.contains("[name]\r\nGrundorf_test"), "{}", &g[..200.min(g.len())]);
        // unveraenderte Kacheln sind Kopien des Originals
        let andere = std::fs::read_dir(&ziel).unwrap().flatten().find(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.ends_with(".map") && e.file_name() != name
        }).unwrap();
        assert_eq!(std::fs::read(andere.path()).unwrap(), std::fs::read(root.join("maps/Grundorf").join(andere.file_name())).unwrap());
        std::fs::remove_dir_all(&test_root).ok();
    }
}
