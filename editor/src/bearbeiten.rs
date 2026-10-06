//! Objekte auswaehlen, bearbeiten und neu platzieren.
//!
//! - Objekte der Karte: ueber openOMSIs Objekt-Editor (Aenderungen live auf der Grafikkarte, beim Speichern schreibt
//!   er die Kachel um).
//! - Neue Objekte (Kopien, Objektbrowser): eigene Liste; gezeichnet als freie Objekte, beim Speichern als neue
//!   `[object]`-Eintraege in ihre Kachel geschrieben (speichern.rs).
//!
//! Klick waehlt, Ziehen schiebt ueber das Gelaende, Strg+Rad oder , / . dreht, Bild auf/ab hebt/senkt, Entf loescht,
//! Strg+Z / Strg+Y nimmt zurueck / wiederholt.

use crate::kamera::Kamera;
use glam::{DVec3, Vec4};
use openomsi_game::viewer::{Action, Editor, ObjectEdit, TileGpu, Viewer};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Werkzeug {
    Ansehen,
    Objekte,
    Platzieren,
}

/// Was gewaehlt ist: ein Objekt der Karte (id) oder ein neues (Index in `neue`)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wahl {
    Karte(i64),
    Neu(usize),
}

/// Zustand eines neuen Objekts
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct NeuZustand {
    /// Fuss des Objekts (Welt)
    pub pos: DVec3,
    /// Grad im Uhrzeigersinn ab Nord
    pub richtung: f64,
    /// Hoehe ueber dem Gelaende (wie das Feld in der Kacheldatei)
    pub ueber_boden: f64,
    pub geloescht: bool,
}

pub struct Neu {
    /// .sco relativ zum OMSI-Ordner (mit \)
    pub rel: String,
    pub z: NeuZustand,
    gpu: Option<TileGpu>,
}

#[derive(Clone, Copy, Debug)]
enum Schritt {
    Karte { id: i64, vorher: ObjectEdit, nachher: ObjectEdit },
    Neu { i: usize, vorher: NeuZustand, nachher: NeuZustand },
}

/// Objekt im Bild
#[derive(Clone, Debug)]
pub struct Objekt {
    pub wahl: Wahl,
    pub pos: DVec3,
    pub richtung: f64,
    pub sco: PathBuf,
}

/// Vorschau beim Platzieren: folgt der Maus
struct Geist {
    rel: String,
    pos: DVec3,
    richtung: f64,
    gpu: Option<TileGpu>,
}

pub struct Bearbeiten {
    pub ed: Editor,
    pub werkzeug: Werkzeug,
    pub wahl: Option<Wahl>,
    pub unter_maus: Option<Wahl>,
    pub neue: Vec<Neu>,
    ziehen: Option<(Wahl, DVec3, Schritt)>,
    undo: Vec<Schritt>,
    redo: Vec<Schritt>,
    geist: Option<Geist>,
    /// Richtung fuer das naechste platzierte Objekt
    pub platzier_richtung: f64,
    /// Anzahl Aenderungen seit dem Oeffnen (fuers Speichern)
    pub aenderungen: usize,
}

impl Default for Bearbeiten {
    fn default() -> Self {
        Bearbeiten { ed: Editor::default(), werkzeug: Werkzeug::Ansehen, wahl: None, unter_maus: None, neue: vec![],
                     ziehen: None, undo: vec![], redo: vec![], geist: None, platzier_richtung: 0.0, aenderungen: 0 }
    }
}

/// Weltpunkt -> Bildschirmpunkt (logische Punkte) und Tiefe, oder None hinter der Kamera
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

/// .sco-Pfad relativ zum OMSI-Ordner, mit \ (wie in Kacheldateien)
pub fn relativ(root: &Path, sco: &Path) -> String {
    sco.strip_prefix(root).unwrap_or(sco).to_string_lossy().replace('/', "\\")
}

impl Bearbeiten {
    /// frisch (nach Kartenwechsel), Werkzeug bleibt
    pub fn neu(werkzeug: Werkzeug) -> Self {
        Bearbeiten { werkzeug, ..Default::default() }
    }

    /// alle Objekte im Bild: die der geladenen Kacheln und die neuen (geloeschte nicht)
    pub fn objekte(&self, v: &Viewer) -> Vec<Objekt> {
        let mut out: Vec<Objekt> = v
            .objects()
            .into_iter()
            .map(|(id, pos, richtung, sco)| Objekt { wahl: Wahl::Karte(id), pos, richtung, sco })
            .collect();
        for (i, n) in self.neue.iter().enumerate().filter(|(_, n)| !n.z.geloescht) {
            out.push(Objekt { wahl: Wahl::Neu(i), pos: n.z.pos, richtung: n.z.richtung, sco: v.root.join(&n.rel) });
        }
        out
    }

    /// Objekt unter dem Mauszeiger: naechstes am Bildschirm (Fuss oder 2 m darueber, 28 px), sonst das naehere
    pub fn suchen(&self, v: &Viewer, kam: &Kamera, maus: (f32, f32), breite: f32, hoehe: f32) -> Option<Wahl> {
        let mut best: Option<(f32, Wahl)> = None;
        for o in self.objekte(v) {
            if (o.pos - kam.ziel).length() > kam.abstand * 4.0 + 300.0 {
                continue;
            }
            for dz in [0.3, 2.0] {
                let Some((x, y, tiefe)) = projizieren(kam, o.pos + DVec3::Z * dz, breite, hoehe) else { continue };
                let d = ((x - maus.0).powi(2) + (y - maus.1).powi(2)).sqrt();
                if d < 28.0 {
                    let wert = d + tiefe * 0.02;
                    if best.map(|b| wert < b.0).unwrap_or(true) {
                        best = Some((wert, o.wahl));
                    }
                }
            }
        }
        best.map(|b| b.1)
    }

    pub fn waehlen(&mut self, w: Option<Wahl>) {
        self.ed.editing_added = None;
        self.wahl = w;
        self.ed.selected = match w {
            Some(Wahl::Karte(id)) => Some(id),
            _ => None,
        };
    }

    fn schritt(&mut self, s: Schritt) {
        self.undo.push(s);
        self.redo.clear();
        self.aenderungen += 1;
    }

    /// neues Objekt neu zeichnen (oder wegnehmen, wenn geloescht)
    fn zeichnen(&mut self, v: &mut Viewer, i: usize) {
        let n = &mut self.neue[i];
        if let Some(g) = n.gpu.take() {
            v.remove_object(g);
        }
        if !n.z.geloescht {
            n.gpu = v.add_object(&n.rel, n.z.pos, n.z.richtung);
        }
    }

    /// Aktion auf die Auswahl, mit Rueckgaengig-Schritt
    pub fn aktion(&mut self, v: &mut Viewer, a: Action) -> Option<String> {
        if let Action::Copy = a {
            return self.kopieren(v);
        }
        match self.wahl? {
            Wahl::Karte(id) => {
                self.ed.editing_added = None;
                self.ed.selected = Some(id);
                let vorher = v.object_edit(id);
                let r = v.edit(&mut self.ed, &a);
                let nachher = v.object_edit(id);
                if nachher != vorher {
                    self.schritt(Schritt::Karte { id, vorher, nachher });
                }
                r
            }
            Wahl::Neu(i) => {
                let vorher = self.neue.get(i)?.z;
                let mut z = vorher;
                match a {
                    Action::Move(d) => {
                        z.pos += d;
                        z.ueber_boden += d.z;
                    }
                    Action::Turn(t) => z.richtung += t,
                    Action::Delete => z.geloescht = !z.geloescht,
                    _ => return None,
                }
                self.neue[i].z = z;
                self.zeichnen(v, i);
                self.schritt(Schritt::Neu { i, vorher, nachher: z });
                Some(self.beschreiben(i))
            }
        }
    }

    fn beschreiben(&self, i: usize) -> String {
        let n = &self.neue[i];
        let name = n.rel.rsplit('\\').next().unwrap_or("");
        if n.z.geloescht {
            format!("neues Objekt {name}: geloescht (Entf holt es zurueck)")
        } else {
            format!("neues Objekt {name} bei {:.1} / {:.1}, {:.1} Grad", n.z.pos.x, n.z.pos.y, n.z.richtung)
        }
    }

    /// neues Objekt anlegen -> Index (None, wenn das Objekt sich nicht laden laesst)
    pub fn anlegen(&mut self, v: &mut Viewer, rel: &str, pos: DVec3, richtung: f64, ueber_boden: f64) -> Option<usize> {
        let z = NeuZustand { pos, richtung, ueber_boden, geloescht: false };
        let gpu = v.add_object(rel, pos, richtung);
        gpu.as_ref()?;
        self.neue.push(Neu { rel: rel.to_string(), z, gpu });
        let i = self.neue.len() - 1;
        self.schritt(Schritt::Neu { i, vorher: NeuZustand { geloescht: true, ..z }, nachher: z });
        Some(i)
    }

    /// Kopie der Auswahl 3 m rechts daneben, danach gewaehlt
    fn kopieren(&mut self, v: &mut Viewer) -> Option<String> {
        let o = self.objekte(v).into_iter().find(|o| Some(o.wahl) == self.wahl)?;
        let rel = match o.wahl {
            Wahl::Neu(i) => self.neue[i].rel.clone(),
            Wahl::Karte(_) => relativ(&v.root, &o.sco),
        };
        let ueber = match o.wahl {
            Wahl::Neu(i) => self.neue[i].z.ueber_boden,
            Wahl::Karte(_) => o.pos.z - v.terrain_height(o.pos.x, o.pos.y).unwrap_or(o.pos.z),
        };
        let (s, c) = o.richtung.to_radians().sin_cos();
        let mut pos = o.pos + DVec3::new(c, -s, 0.0) * 3.0;
        pos.z = v.terrain_height(pos.x, pos.y).unwrap_or(o.pos.z) + ueber;
        match self.anlegen(v, &rel, pos, o.richtung, ueber) {
            Some(i) => {
                self.waehlen(Some(Wahl::Neu(i)));
                Some(format!("Kopie angelegt: {}", self.beschreiben(i)))
            }
            None => Some(format!("{rel} laesst sich nicht laden")),
        }
    }

    // ------------------------------------------------------------------ Platzieren
    /// Vorschau des zu platzierenden Objekts an den Bodenpunkt setzen
    pub fn geist(&mut self, v: &mut Viewer, rel: &str, boden: DVec3) {
        let richtung = self.platzier_richtung;
        if let Some(g) = self.geist.as_ref() {
            if g.rel == rel && (g.pos - boden).length() < 0.05 && (g.richtung - richtung).abs() < 1e-6 {
                return;
            }
        }
        self.geist_weg(v);
        let gpu = v.add_object(rel, boden, richtung);
        self.geist = Some(Geist { rel: rel.to_string(), pos: boden, richtung, gpu });
    }

    pub fn geist_weg(&mut self, v: &mut Viewer) {
        if let Some(g) = self.geist.take() {
            if let Some(t) = g.gpu {
                v.remove_object(t);
            }
        }
    }

    pub fn geist_ok(&self) -> bool {
        self.geist.as_ref().map(|g| g.gpu.is_some()).unwrap_or(false)
    }

    /// Objekt am Bodenpunkt setzen (Fuss auf dem Gelaende)
    pub fn platzieren(&mut self, v: &mut Viewer, rel: &str, boden: DVec3) -> Option<String> {
        let i = self.anlegen(v, rel, boden, self.platzier_richtung, 0.0)?;
        Some(format!("platziert: {}", self.beschreiben(i)))
    }

    // ------------------------------------------------------------------ Rueckgaengig
    fn anwenden(&mut self, v: &mut Viewer, s: Schritt, nachher: bool) {
        match s {
            Schritt::Karte { id, vorher, nachher: n } => {
                let z = if nachher { n } else { vorher };
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
                self.wahl = Some(Wahl::Karte(id));
            }
            Schritt::Neu { i, vorher, nachher: n } => {
                self.neue[i].z = if nachher { n } else { vorher };
                self.zeichnen(v, i);
                self.wahl = (!self.neue[i].z.geloescht).then_some(Wahl::Neu(i));
            }
        }
    }

    pub fn rueckgaengig(&mut self, v: &mut Viewer) -> Option<String> {
        let s = self.undo.pop()?;
        self.anwenden(v, s, false);
        self.redo.push(s);
        self.aenderungen += 1;
        Some("rueckgaengig".into())
    }

    pub fn wiederholen(&mut self, v: &mut Viewer) -> Option<String> {
        let s = self.redo.pop()?;
        self.anwenden(v, s, true);
        self.undo.push(s);
        self.aenderungen += 1;
        Some("wiederholt".into())
    }

    pub fn kann_rueckgaengig(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn kann_wiederholen(&self) -> bool {
        !self.redo.is_empty()
    }

    // ------------------------------------------------------------------ Ziehen
    pub fn greifen(&mut self, v: &Viewer, w: Wahl, boden: DVec3) {
        let pos = self.objekte(v).into_iter().find(|o| o.wahl == w).map(|o| o.pos).unwrap_or(boden);
        let vorher = match w {
            Wahl::Karte(id) => {
                let e = v.object_edit(id);
                Schritt::Karte { id, vorher: e, nachher: e }
            }
            Wahl::Neu(i) => Schritt::Neu { i, vorher: self.neue[i].z, nachher: self.neue[i].z },
        };
        self.ziehen = Some((w, DVec3::new(pos.x - boden.x, pos.y - boden.y, 0.0), vorher));
    }

    pub fn zieht(&self) -> bool {
        self.ziehen.is_some()
    }

    /// Fuss des Objekts auf den Boden unter der Maus (plus Versatz vom Greifen)
    pub fn ziehen_nach(&mut self, v: &mut Viewer, boden: DVec3) -> Option<String> {
        let (w, versatz, _) = self.ziehen?;
        let (x, y) = (boden.x + versatz.x, boden.y + versatz.y);
        let gelaende = v.terrain_height(x, y).unwrap_or(boden.z);
        match w {
            Wahl::Karte(id) => {
                self.ed.editing_added = None;
                self.ed.selected = Some(id);
                v.drag(&mut self.ed, DVec3::new(x, y, gelaende))
            }
            Wahl::Neu(i) => {
                let z = &mut self.neue[i].z;
                z.pos = DVec3::new(x, y, gelaende + z.ueber_boden);
                self.zeichnen(v, i);
                Some(self.beschreiben(i))
            }
        }
    }

    pub fn loslassen(&mut self, v: &Viewer) {
        let Some((_, _, s)) = self.ziehen.take() else { return };
        match s {
            Schritt::Karte { id, vorher, .. } => {
                let nachher = v.object_edit(id);
                if nachher != vorher {
                    self.schritt(Schritt::Karte { id, vorher, nachher });
                }
            }
            Schritt::Neu { i, vorher, .. } => {
                let nachher = self.neue[i].z;
                if nachher != vorher {
                    self.schritt(Schritt::Neu { i, vorher, nachher });
                }
            }
        }
    }
}

/// Auswahl im Bild markieren: Ring am Boden und Richtungspfeil (egui, ohne Tiefentest)
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
    //! Mit echter Karte und Grafikkarte (ohne Fenster): cargo test --release -- --include-ignored
    use super::*;

    pub const OMSI: &str = r"C:\Program Files (x86)\Steam\steamapps\common\OMSI 2";

    pub fn grundorf() -> Viewer {
        let root = Path::new(OMSI);
        let instance = openomsi_game::viewer::instance();
        let (mut v, _) = Viewer::open(&instance, None, root, &root.join("maps/Grundorf/global.cfg")).unwrap();
        v.tiles_around(DVec3::new(150.0, 150.0, 0.0), 1).unwrap();
        v
    }

    fn kopieren(a: &Path, b: &Path) {
        std::fs::create_dir_all(b).unwrap();
        for e in std::fs::read_dir(a).unwrap().flatten() {
            if e.file_type().unwrap().is_dir() {
                kopieren(&e.path(), &b.join(e.file_name()))
            } else {
                std::fs::copy(e.path(), b.join(e.file_name())).unwrap();
            }
        }
    }

    fn utf16(b: &[u8]) -> String {
        String::from_utf16_lossy(&b[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>())
    }

    #[test]
    #[ignore]
    fn bearbeiten_kopieren_platzieren_speichern() {
        let root = Path::new(OMSI);
        let mut v = grundorf();
        let mut b = Bearbeiten::neu(Werkzeug::Objekte);
        let objekte = b.objekte(&v);
        assert!(objekte.len() > 20, "{} Objekte", objekte.len());
        let o = objekte
            .iter()
            .filter_map(|o| match o.wahl {
                Wahl::Karte(id) => Some((id, o.clone())),
                _ => None,
            })
            .min_by_key(|x| x.0)
            .unwrap()
            .1;
        let Wahl::Karte(id) = o.wahl else { unreachable!() };
        // Kartenobjekt verschieben/drehen, Rueckgaengig, Wiederholen
        b.waehlen(Some(o.wahl));
        b.aktion(&mut v, Action::Move(DVec3::new(3.0, -2.0, 0.0)));
        b.aktion(&mut v, Action::Turn(15.0));
        let e = v.object_edit(id);
        b.rueckgaengig(&mut v);
        assert_eq!(v.object_edit(id).turned, 0.0);
        b.wiederholen(&mut v);
        assert_eq!(v.object_edit(id), e);
        // Kopieren: neues Objekt 3 m daneben, gewaehlt, im Bild
        b.aktion(&mut v, Action::Copy);
        let Some(Wahl::Neu(k)) = b.wahl else { panic!("Kopie nicht gewaehlt: {:?}", b.wahl) };
        assert!(b.objekte(&v).iter().any(|x| x.wahl == Wahl::Neu(k)));
        b.aktion(&mut v, Action::Turn(90.0));
        // Platzieren: an einer freien Stelle
        let rel = b.neue[k].rel.clone();
        let boden = DVec3::new(160.0, 140.0, v.terrain_height(160.0, 140.0).unwrap_or(0.0));
        b.platzieren(&mut v, &rel, boden).unwrap();
        assert_eq!(b.neue.len(), 2);
        b.rueckgaengig(&mut v);
        assert!(b.neue[1].z.geloescht);
        b.wiederholen(&mut v);
        assert!(!b.neue[1].z.geloescht);
        // Speichern in einen Test-OMSI-Ordner (die echte Installation bleibt unberuehrt)
        let test_root = std::env::temp_dir().join(format!("omsi-editor-root-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&test_root);
        kopieren(&root.join("maps/Grundorf"), &test_root.join("maps/Grundorf"));
        let start = v.next_object_id();
        let ziel = crate::speichern::alles_speichern(&v, &b, "Grundorf", "Grundorf_test", &test_root).unwrap();
        let g = utf16(&std::fs::read(ziel.join("global.cfg")).unwrap());
        assert!(g.contains("[name]\r\nGrundorf_test"), "Name");
        assert!(g.contains(&format!("[NextIDCode]\r\n{}", start + 2)), "NextIDCode");
        // neue Objekte stehen mit eindeutigen IDs in ihren Kacheln
        let mut gefunden = 0;
        for e in std::fs::read_dir(&ziel).unwrap().flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.ends_with(".map") {
                let b = std::fs::read(e.path()).unwrap();
                let t = if b.starts_with(&[0xFF, 0xFE]) { utf16(&b) } else { String::from_utf8_lossy(&b).into() };
                for id in [start, start + 1] {
                    if t.contains(&format!("[object]\r\n0\r\n{rel}\r\n{id}\r\n")) {
                        gefunden += 1;
                    }
                }
            }
        }
        assert_eq!(gefunden, 2);
        // die gespeicherte Karte wieder oeffnen (Inhalte aus der echten Installation): neue Objekte stehen dort
        let erwartet: Vec<DVec3> = b.neue.iter().map(|n| n.z.pos).collect();
        drop(v);
        let instance = openomsi_game::viewer::instance();
        let (mut v2, _) = Viewer::open(&instance, None, root, &ziel.join("global.cfg")).unwrap();
        v2.tiles_around(DVec3::new(150.0, 150.0, 0.0), 1).unwrap();
        let geladen = v2.objects();
        // (die IDs werden kachelweise vergeben: Zuordnung ueber die Stelle)
        for id in [start, start + 1] {
            let o = geladen.iter().find(|o| o.0 == id).unwrap_or_else(|| panic!("Objekt {id} fehlt in der gespeicherten Karte"));
            let d = erwartet.iter().map(|p| (o.1 - *p).truncate().length()).fold(f64::INFINITY, f64::min);
            assert!(d < 0.05, "Objekt {id} steht {d:.2} m neben jeder erwarteten Stelle");
        }
        // das verschobene Kartenobjekt steht an seiner neuen Stelle
        let o2 = geladen.iter().find(|o| o.0 == id).unwrap();
        assert!(((o2.1 - o.pos).truncate() - glam::DVec2::new(3.0, -2.0)).length() < 0.05);
        std::fs::remove_dir_all(&test_root).ok();
    }
}
