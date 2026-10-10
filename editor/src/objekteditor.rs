//! Objekt-Editor fuer Sonderbauten (zuerst die Kreuzung Ludwigsallee am Ring): jedes Bauteil des Objekts einzeln
//! bearbeiten - Flaechen (Fahrbahn, Gehweg, Insel, Gruen, Sperrflaeche, Radfurt, Busspur-Beton) mit Eckpunkten und
//! Textur-Lage, Linien, Haltlinien, Pfeile, BUS-Schrift, Ampeln, Gelaender, Fussgaengerfurten, Gehpfade, dazu
//! Bahntunnel und Laermschutzwaende des Bauwerks.
//!
//! Bearbeitet wird nur der Bauplan (`sonderbau/ring/bauplan.json`, Format in omsigen/ring_bauplan.py); "Anwenden" baut
//! daraus mit omsigen alle Objekte neu und setzt den Sonderbau in die Karte (Sitzung, Rueckgaengig, Speichern wie
//! gewohnt). Die Draufsicht zeigt das Luftbild und die Flaechen mit ihren echten Texturen in derselben Lage wie im
//! Objekt.

use glam::{DVec2, DVec3};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

// ------------------------------------------------------------------ Bauplan
#[derive(Clone, Debug, PartialEq)]
pub struct Textur {
    pub dx: f64,
    pub dy: f64,
    pub skala: f64,
    pub dreh: f64,
}

#[derive(Clone, Debug)]
pub struct Flaeche {
    pub id: String,
    pub art: String,
    pub punkte: Vec<DVec2>,
    pub loecher: Vec<Vec<DVec2>>,
    pub textur: Textur,
}

#[derive(Clone, Debug)]
pub struct Linie {
    pub id: String,
    pub punkte: Vec<DVec2>,
    pub breite: f64,
    pub an: f64,
    pub aus: f64,
}

#[derive(Clone, Debug)]
pub struct Haltlinie {
    pub id: String,
    pub arm: usize,
    pub punkte: Vec<DVec2>,
    pub breite: f64,
}

#[derive(Clone, Debug)]
pub struct Symbol {
    pub id: String,
    pub art: String,
    pub pos: DVec2,
    pub richtung: f64,
    pub pfeile: Vec<String>,
    pub skala: f64,
}

#[derive(Clone, Debug)]
pub struct Ampel {
    pub id: String,
    pub art: String,
    pub pos: DVec2,
    pub richtung: f64,
    pub mast: String,
    pub gruppe: i64,
}

#[derive(Clone, Debug)]
pub struct Gelaender {
    pub id: String,
    pub punkte: Vec<DVec2>,
    pub unten: String,
    pub flip: bool,
}

#[derive(Clone, Debug)]
pub struct Furt {
    pub id: String,
    pub a: DVec2,
    pub b: DVec2,
    pub breite: f64,
    pub gruppe: i64,
}

#[derive(Clone, Debug)]
pub struct Gehpfad {
    pub id: String,
    pub punkte: Vec<DVec2>,
}

#[derive(Clone, Debug, Default)]
pub struct Bauplan {
    /// alle weiteren Felder (Name, Abbiegespuren, Phasen, Tunnel, Laermschutz, Zaehler) unveraendert durchgereicht
    pub rest: serde_json::Map<String, serde_json::Value>,
    pub flaechen: Vec<Flaeche>,
    pub linien: Vec<Linie>,
    pub haltlinien: Vec<Haltlinie>,
    pub symbole: Vec<Symbol>,
    pub ampeln: Vec<Ampel>,
    pub gelaender: Vec<Gelaender>,
    pub furten: Vec<Furt>,
    pub gehpfade: Vec<Gehpfad>,
}

fn p2(v: &serde_json::Value) -> DVec2 {
    DVec2::new(v[0].as_f64().unwrap_or(0.0), v[1].as_f64().unwrap_or(0.0))
}

fn pl(v: &serde_json::Value) -> Vec<DVec2> {
    v.as_array().map(|a| a.iter().map(p2).collect()).unwrap_or_default()
}

fn j2(p: DVec2) -> serde_json::Value {
    serde_json::json!([(p.x * 1000.0).round() / 1000.0, (p.y * 1000.0).round() / 1000.0])
}

fn jl(p: &[DVec2]) -> serde_json::Value {
    serde_json::Value::Array(p.iter().map(|q| j2(*q)).collect())
}

fn f(v: &serde_json::Value, k: &str, std: f64) -> f64 {
    v[k].as_f64().unwrap_or(std)
}

fn s(v: &serde_json::Value, k: &str) -> String {
    v[k].as_str().unwrap_or("").to_string()
}

impl Bauplan {
    pub fn aus_json(j: &serde_json::Value) -> Bauplan {
        let liste = |k: &str| j[k].as_array().cloned().unwrap_or_default();
        let mut rest = j.as_object().cloned().unwrap_or_default();
        for k in ["flaechen", "linien", "haltlinien", "symbole", "ampeln", "gelaender", "furten", "gehpfade"] {
            rest.remove(k);
        }
        Bauplan {
            rest,
            flaechen: liste("flaechen").iter().map(|v| Flaeche {
                id: s(v, "id"), art: s(v, "art"), punkte: pl(&v["punkte"]),
                loecher: v["loecher"].as_array().map(|a| a.iter().map(pl).collect()).unwrap_or_default(),
                textur: Textur { dx: f(&v["textur"], "dx", 0.0), dy: f(&v["textur"], "dy", 0.0), skala: f(&v["textur"], "skala", 1.0), dreh: f(&v["textur"], "dreh", 0.0) },
            }).collect(),
            linien: liste("linien").iter().map(|v| Linie { id: s(v, "id"), punkte: pl(&v["punkte"]), breite: f(v, "breite", 0.12), an: f(v, "an", 0.0), aus: f(v, "aus", 0.0) }).collect(),
            haltlinien: liste("haltlinien").iter().map(|v| Haltlinie { id: s(v, "id"), arm: v["arm"].as_u64().unwrap_or(0) as usize, punkte: pl(&v["punkte"]), breite: f(v, "breite", 0.5) }).collect(),
            symbole: liste("symbole").iter().map(|v| Symbol {
                id: s(v, "id"), art: s(v, "art"), pos: p2(&v["pos"]), richtung: f(v, "richtung", 0.0),
                pfeile: v["pfeile"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(|x| x.to_string())).collect()).unwrap_or_default(),
                skala: f(v, "skala", 1.0),
            }).collect(),
            ampeln: liste("ampeln").iter().map(|v| Ampel { id: s(v, "id"), art: s(v, "art"), pos: p2(&v["pos"]), richtung: f(v, "richtung", 0.0), mast: s(v, "mast"), gruppe: v["gruppe"].as_i64().unwrap_or(0) }).collect(),
            gelaender: liste("gelaender").iter().map(|v| Gelaender { id: s(v, "id"), punkte: pl(&v["punkte"]), unten: s(v, "unten"), flip: v["flip"].as_bool().unwrap_or(false) }).collect(),
            furten: liste("furten").iter().map(|v| Furt { id: s(v, "id"), a: p2(&v["a"]), b: p2(&v["b"]), breite: f(v, "breite", 3.0), gruppe: v["gruppe"].as_i64().unwrap_or(0) }).collect(),
            gehpfade: liste("gehpfade").iter().map(|v| Gehpfad { id: s(v, "id"), punkte: pl(&v["punkte"]) }).collect(),
        }
    }

    pub fn json(&self) -> serde_json::Value {
        let mut o = self.rest.clone();
        let r = |x: f64| (x * 1000.0).round() / 1000.0;
        o.insert("flaechen".into(), self.flaechen.iter().map(|x| serde_json::json!({
            "id": x.id, "art": x.art, "punkte": jl(&x.punkte), "loecher": x.loecher.iter().map(|l| jl(l)).collect::<Vec<_>>(),
            "textur": {"dx": r(x.textur.dx), "dy": r(x.textur.dy), "skala": r(x.textur.skala), "dreh": r(x.textur.dreh)},
        })).collect());
        o.insert("linien".into(), self.linien.iter().map(|x| serde_json::json!({"id": x.id, "punkte": jl(&x.punkte), "breite": r(x.breite), "an": r(x.an), "aus": r(x.aus)})).collect());
        o.insert("haltlinien".into(), self.haltlinien.iter().map(|x| serde_json::json!({"id": x.id, "arm": x.arm, "punkte": jl(&x.punkte), "breite": r(x.breite)})).collect());
        o.insert("symbole".into(), self.symbole.iter().map(|x| serde_json::json!({"id": x.id, "art": x.art, "pos": j2(x.pos), "richtung": r(x.richtung), "pfeile": x.pfeile, "skala": r(x.skala)})).collect());
        o.insert("ampeln".into(), self.ampeln.iter().map(|x| serde_json::json!({"id": x.id, "art": x.art, "pos": j2(x.pos), "richtung": r(x.richtung), "mast": x.mast, "gruppe": x.gruppe})).collect());
        o.insert("gelaender".into(), self.gelaender.iter().map(|x| serde_json::json!({"id": x.id, "punkte": jl(&x.punkte), "unten": x.unten, "flip": x.flip})).collect());
        o.insert("furten".into(), self.furten.iter().map(|x| serde_json::json!({"id": x.id, "a": j2(x.a), "b": j2(x.b), "breite": r(x.breite), "gruppe": x.gruppe})).collect());
        o.insert("gehpfade".into(), self.gehpfade.iter().map(|x| serde_json::json!({"id": x.id, "punkte": jl(&x.punkte)})).collect());
        serde_json::Value::Object(o)
    }

    fn neue_id(&mut self, art: &str) -> String {
        let n = self.rest.get("_id").and_then(|v| v.as_u64()).unwrap_or(1000) + 1;
        self.rest.insert("_id".into(), n.into());
        format!("{art}{n}")
    }
}

// ------------------------------------------------------------------ Auswahl
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wahl {
    Flaeche(usize),
    Linie(usize),
    Halt(usize),
    Symbol(usize),
    Ampel(usize),
    Gelaender(usize),
    Furt(usize),
    Gehpfad(usize),
}

pub const FLAECHEN_ARTEN: [(&str, &str); 7] = [("fahrbahn", "Fahrbahn"), ("gehweg", "Gehweg (erhoeht)"), ("insel", "Insel Beton (erhoeht)"),
    ("gruen", "Insel bepflanzt (erhoeht)"), ("sperr", "Sperrflaeche (schraffiert)"), ("radfurt", "Radfurt (rot)"), ("busbeton", "Busspur Beton")];

/// Material je Flaechenart (Texturdatei im Objektordner, Meter je Textur wie ring_bauplan.ARTEN)
fn material(art: &str) -> (&'static str, f64) {
    match art {
        "fahrbahn" => ("str_asphdrk.bmp", 4.0),
        "gehweg" => ("AB_gehweg_asphalt.bmp", 4.0),
        "insel" => ("AB_insel_beton.bmp", 3.0),
        "gruen" => ("AB_bodendecker.bmp", 2.0),
        "busbeton" => ("str_betonplatten.bmp", 3.9),
        "sperr" => ("AB_sperrflaeche.tga", 2.0),
        _ => ("AB_radfurt.bmp", 4.0),
    }
}

fn reihenfolge(art: &str) -> u8 {
    match art {
        "fahrbahn" => 0,
        "gehweg" | "insel" | "gruen" => 1,
        "busbeton" => 2,
        "sperr" => 3,
        _ => 4,
    }
}

// ------------------------------------------------------------------ Kontext (aus sonderbau_ring.json)
#[derive(Clone, Debug, Default)]
pub struct Arm {
    pub name: String,
    pub pos: DVec2,
    pub h: f64,
    pub links: DVec2,
    pub rechts: DVec2,
}

#[derive(Clone, Debug, Default)]
pub struct Kontext {
    pub offen: Vec<Vec<DVec2>>,
    pub arme: Vec<Arm>,
    pub achse: Vec<DVec2>,
    pub lose: usize,
}

impl Kontext {
    fn lesen(datei: &Path) -> Kontext {
        let Ok(b) = std::fs::read(datei) else { return Kontext::default() };
        let Ok(j) = serde_json::from_slice::<serde_json::Value>(&b) else { return Kontext::default() };
        let k = &j["kontext"];
        Kontext {
            offen: k["offen"].as_array().map(|a| a.iter().map(pl).collect()).unwrap_or_default(),
            arme: k["arme"].as_array().map(|a| a.iter().map(|v| Arm { name: s(v, "name"), pos: p2(&v["pos"]), h: f(v, "h", 0.0), links: p2(&v["links"]), rechts: p2(&v["rechts"]) }).collect()).unwrap_or_default(),
            achse: pl(&k["achse"]),
            lose: k["lose_signale"].as_array().map(|a| a.len()).unwrap_or(0),
        }
    }
}

// ------------------------------------------------------------------ Geometrie
fn punkt_in(p: DVec2, poly: &[DVec2]) -> bool {
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

fn abstand_strecke(p: DVec2, a: DVec2, b: DVec2) -> f64 {
    let d = b - a;
    let t = if d.length_squared() < 1e-12 { 0.0 } else { ((p - a).dot(d) / d.length_squared()).clamp(0.0, 1.0) };
    (a + d * t - p).length()
}

fn abstand_zug(p: DVec2, pts: &[DVec2], zu: bool) -> (f64, usize) {
    let n = pts.len();
    let mut best = (f64::MAX, 0);
    let m = if zu { n } else { n.saturating_sub(1) };
    for i in 0..m {
        let d = abstand_strecke(p, pts[i], pts[(i + 1) % n]);
        if d < best.0 {
            best = (d, i);
        }
    }
    best
}

fn richtung_vec(h: f64) -> DVec2 {
    let r = h.to_radians();
    DVec2::new(r.sin(), r.cos())
}

fn rechts_vec(h: f64) -> DVec2 {
    let r = h.to_radians();
    DVec2::new(r.cos(), -r.sin())
}

fn flaeche2(p: &[DVec2]) -> f64 {
    let n = p.len();
    (0..n).map(|i| p[i].x * p[(i + 1) % n].y - p[(i + 1) % n].x * p[i].y).sum::<f64>() * 0.5
}

/// Polygon mit Loechern in ein einfaches Polygon verwandeln (Loecher ueber eine Bruecke anhaengen)
fn mit_loechern(aussen: &[DVec2], loecher: &[Vec<DVec2>]) -> Vec<DVec2> {
    let mut ring: Vec<DVec2> = aussen.to_vec();
    if flaeche2(&ring) < 0.0 {
        ring.reverse();
    }
    let mut loecher: Vec<Vec<DVec2>> = loecher.iter().filter(|l| l.len() >= 3).map(|l| {
        let mut l = l.clone();
        if flaeche2(&l) > 0.0 {
            l.reverse();
        }
        l
    }).collect();
    loecher.sort_by(|a, b| b.iter().map(|p| p.x).fold(f64::MIN, f64::max).partial_cmp(&a.iter().map(|p| p.x).fold(f64::MIN, f64::max)).unwrap());
    for l in loecher {
        let (hi, h) = l.iter().enumerate().max_by(|a, b| a.1.x.partial_cmp(&b.1.x).unwrap()).map(|(i, p)| (i, *p)).unwrap();
        // naechster Punkt des Rings, dessen Verbindung keine Kante schneidet
        let mut beste: Option<(f64, usize)> = None;
        for (i, q) in ring.iter().enumerate() {
            let d = (*q - h).length();
            if beste.map(|b| d >= b.0).unwrap_or(false) {
                continue;
            }
            let schneidet = (0..ring.len()).any(|k| {
                let (a, b) = (ring[k], ring[(k + 1) % ring.len()]);
                k != i && (k + 1) % ring.len() != i && schnitt(h, *q, a, b)
            }) || (0..l.len()).any(|k| {
                let (a, b) = (l[k], l[(k + 1) % l.len()]);
                k != hi && (k + 1) % l.len() != hi && schnitt(h, *q, a, b)
            });
            if !schneidet {
                beste = Some((d, i));
            }
        }
        let Some((_, i)) = beste else { continue };
        let mut neu = ring[..=i].to_vec();
        for k in 0..=l.len() {
            neu.push(l[(hi + k) % l.len()]);
        }
        neu.push(ring[i]);
        neu.extend_from_slice(&ring[i + 1..]);
        ring = neu;
    }
    ring
}

fn schnitt(p1: DVec2, p2: DVec2, p3: DVec2, p4: DVec2) -> bool {
    let d = |a: DVec2, b: DVec2, c: DVec2| (b - a).perp_dot(c - a);
    let (d1, d2, d3, d4) = (d(p3, p4, p1), d(p3, p4, p2), d(p1, p2, p3), d(p1, p2, p4));
    ((d1 > 1e-9 && d2 < -1e-9) || (d1 < -1e-9 && d2 > 1e-9)) && ((d3 > 1e-9 && d4 < -1e-9) || (d3 < -1e-9 && d4 > 1e-9))
}

/// Ohrenschneiden: Dreiecke (Indizes) eines einfachen Polygons
fn dreiecke(p: &[DVec2]) -> Vec<[usize; 3]> {
    let n = p.len();
    if n < 3 {
        return vec![];
    }
    let mut idx: Vec<usize> = (0..n).collect();
    if flaeche2(p) < 0.0 {
        idx.reverse();
    }
    let mut out = Vec::new();
    let mut schutz = 0;
    while idx.len() > 3 && schutz < n * n {
        schutz += 1;
        let m = idx.len();
        let mut gefunden = false;
        for k in 0..m {
            let (ia, ib, ic) = (idx[(k + m - 1) % m], idx[k], idx[(k + 1) % m]);
            let (a, b, c) = (p[ia], p[ib], p[ic]);
            if (b - a).perp_dot(c - b) <= 1e-12 {
                continue;
            }
            let innen = idx.iter().any(|&j| {
                if j == ia || j == ib || j == ic {
                    return false;
                }
                let q = p[j];
                if q == a || q == b || q == c {
                    return false;
                }
                (b - a).perp_dot(q - a) > 0.0 && (c - b).perp_dot(q - b) > 0.0 && (a - c).perp_dot(q - c) > 0.0
            });
            if innen {
                continue;
            }
            out.push([ia, ib, ic]);
            idx.remove(k);
            gefunden = true;
            break;
        }
        if !gefunden {
            break;
        }
    }
    if idx.len() == 3 {
        out.push([idx[0], idx[1], idx[2]]);
    }
    out
}

// ------------------------------------------------------------------ Editor
pub enum Aktion {
    /// Objekte neu gebaut (sonderbau_ring.json), in die Karte setzen?
    Gebaut { json: PathBuf, einsetzen: bool },
    /// Kamera der 3D-Ansicht auf diesen Punkt
    Kamera(DVec3),
}

#[derive(Clone, Copy, PartialEq)]
enum Zug {
    Nichts,
    Griff(usize),
    Verschieben,
    Textur,
    Schwenken,
}

pub struct ObjektEditor {
    pub plan_pfad: PathBuf,
    json: PathBuf,
    root: PathBuf,
    karte: String,
    pub plan: Bauplan,
    kontext: Kontext,
    pub wahl: Option<Wahl>,
    mitte: DVec2,
    m_px: f32,
    zug: Zug,
    letzte: Option<DVec2>,
    undo: Vec<String>,
    redo: Vec<String>,
    offen_aenderung: Option<String>,
    pub geaendert: bool,
    job: Option<Receiver<Result<String, String>>>,
    pub meldung: String,
    luftbild: Option<(egui::TextureHandle, [f64; 4])>,
    luft_job: Option<Receiver<Option<(egui::ColorImage, [f64; 4])>>>,
    luft_alpha: f32,
    texturen: std::collections::HashMap<String, Option<egui::TextureHandle>>,
    texturen_echt: bool,
    einsetzen: bool,
    einrasten: bool,
    neue_art: usize,
    zeige: [bool; 8],
}

const EBENEN: [&str; 8] = ["Flaechen", "Linien", "Haltlinien", "Symbole", "Ampeln", "Gelaender", "Furten", "Gehpfade"];

impl ObjektEditor {
    /// Editor fuer den Sonderbau Ring in Karte `karte` (Objekte in Sceneryobjects\Aschaffenburg\<karte>\Sonderbau_Ring)
    pub fn neu(root: &Path, karte: &str) -> Result<ObjektEditor, String> {
        let plan_pfad = crate::kreuzung::omsigen_ordner().join("sonderbau").join("ring").join("bauplan.json");
        let json = root.join("Sceneryobjects").join(crate::speichern::EIGEN).join(karte).join("Sonderbau_Ring").join("sonderbau_ring.json");
        let plan = match std::fs::read(&plan_pfad) {
            Ok(b) => Bauplan::aus_json(&serde_json::from_slice(&b).map_err(|e| format!("Bauplan unlesbar: {e}"))?),
            Err(_) => Bauplan::default(),
        };
        let kontext = Kontext::lesen(&json);
        let mut ed = ObjektEditor {
            plan_pfad, json, root: root.to_path_buf(), karte: karte.to_string(), plan, kontext, wahl: None,
            mitte: DVec2::new(1084.0, -265.0), m_px: 9.0, zug: Zug::Nichts, letzte: None, undo: vec![], redo: vec![],
            offen_aenderung: None, geaendert: false, job: None, meldung: String::new(), luftbild: None, luft_job: None,
            luft_alpha: 0.55, texturen: Default::default(), texturen_echt: true, einsetzen: true, einrasten: true, neue_art: 0,
            zeige: [true; 8],
        };
        if ed.plan.flaechen.is_empty() || ed.kontext.arme.is_empty() {
            ed.bauen(false);
            ed.meldung = "Erstes Erzeugen des Sonderbaus laeuft (Bauplan aus dem Luftbild-Stand) ...".into();
        }
        ed.luftbild_laden(root);
        Ok(ed)
    }

    fn luftbild_laden(&mut self, root: &Path) {
        let Some(b) = crate::geo::Bezug::lesen(&root.join("maps").join(&self.karte)) else { return };
        let (tx, rx) = channel();
        let bereich = [1030.0, -330.0, 1140.0, -200.0];
        std::thread::spawn(move || {
            let erg = crate::geo::ausschnitt(&b, bereich, 1400).ok().flatten().and_then(|p| image::open(p).ok()).map(|i| {
                let i = i.to_rgba8();
                (egui::ColorImage::from_rgba_unmultiplied([i.width() as usize, i.height() as usize], i.as_raw()), bereich)
            });
            let _ = tx.send(erg);
        });
        self.luft_job = Some(rx);
    }

    fn ordner(&self) -> PathBuf {
        self.json.parent().map(|p| p.to_path_buf()).unwrap_or_default()
    }

    // ---------------------------------------------------------- Aenderungen und Rueckgaengig
    fn stand(&self) -> String {
        self.plan.json().to_string()
    }

    /// vor einer Aenderung aufrufen: merkt den Stand fuer Rueckgaengig (einmal je Bedienvorgang)
    fn aendern(&mut self) {
        if self.offen_aenderung.is_none() {
            self.offen_aenderung = Some(self.stand());
        }
        self.geaendert = true;
    }

    fn abschliessen(&mut self) {
        if let Some(s) = self.offen_aenderung.take() {
            if s != self.stand() {
                self.undo.push(s);
                self.redo.clear();
                if self.undo.len() > 200 {
                    self.undo.remove(0);
                }
            }
        }
    }

    fn rueckgaengig(&mut self) {
        self.abschliessen();
        if let Some(s) = self.undo.pop() {
            self.redo.push(self.stand());
            if let Ok(v) = serde_json::from_str(&s) {
                self.plan = Bauplan::aus_json(&v);
            }
            self.wahl = None;
            self.geaendert = true;
        }
    }

    fn wiederholen(&mut self) {
        if let Some(s) = self.redo.pop() {
            self.undo.push(self.stand());
            if let Ok(v) = serde_json::from_str(&s) {
                self.plan = Bauplan::aus_json(&v);
            }
            self.wahl = None;
            self.geaendert = true;
        }
    }

    pub fn speichern(&mut self) -> Result<(), String> {
        let text = serde_json::to_string_pretty(&self.plan.json()).map_err(|e| e.to_string())?;
        if let Some(d) = self.plan_pfad.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        std::fs::write(&self.plan_pfad, text).map_err(|e| format!("{}: {e}", self.plan_pfad.display()))
    }

    /// Bauplan speichern und alle Objekte mit omsigen neu bauen (im Hintergrund)
    fn bauen(&mut self, speichern: bool) {
        if self.job.is_some() {
            return;
        }
        if speichern {
            if let Err(e) = self.speichern() {
                self.meldung = format!("Bauplan nicht gespeichert: {e}");
                return;
            }
        }
        let (tx, rx) = channel();
        let (root, karte, plan) = (self.root.clone(), self.karte.clone(), self.plan_pfad.clone());
        std::thread::spawn(move || {
            let repo = crate::kreuzung::omsigen_ordner();
            let python = std::env::var("OMSIGEN_PYTHON").unwrap_or_else(|_| "python".into());
            let mut cmd = std::process::Command::new(&python);
            cmd.args(["-m", "omsigen.sonderbau_ring", "--omsi"]).arg(&root).args(["--karte", &karte, "--osm"])
                .arg(repo.join("sonderbau").join("ring").join("osm_map.xml")).arg("--bauplan").arg(&plan).current_dir(&repo);
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x0800_0000);
            }
            let erg = match cmd.output() {
                Ok(o) if o.status.success() => {
                    let t = String::from_utf8_lossy(&o.stdout);
                    Ok(t.lines().filter(|l| l.starts_with("Kreuzung:") || l.contains("Objekte,")).collect::<Vec<_>>().join(" | "))
                }
                Ok(o) => Err(String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("Fehler").to_string()),
                Err(e) => Err(format!("{python} starten: {e}")),
            };
            let _ = tx.send(erg);
        });
        self.job = Some(rx);
        self.meldung = "Baue alle Objekte neu ...".into();
    }

    pub fn arbeitet(&self) -> bool {
        self.job.is_some()
    }

    /// relative .sco aller Objekte des Sonderbaus (zum Neu-Einlesen nach dem Bauen)
    pub fn objekte(&self) -> Vec<String> {
        let Ok(b) = std::fs::read(&self.json) else { return vec![] };
        let Ok(j) = serde_json::from_slice::<serde_json::Value>(&b) else { return vec![] };
        let mut out: Vec<String> = j["objekte"].as_array().map(|a| a.iter().filter_map(|o| o["rel"].as_str().map(|x| x.to_string())).collect()).unwrap_or_default();
        out.extend(j["kreuzungen"].as_array().map(|a| a.iter().filter_map(|o| o["rel"].as_str().map(|x| x.to_string())).collect::<Vec<_>>()).unwrap_or_default());
        out
    }

    // ---------------------------------------------------------- Bauteile allgemein
    fn griffe(&self, w: Wahl) -> Vec<DVec2> {
        match w {
            Wahl::Flaeche(i) => self.plan.flaechen.get(i).map(|x| x.punkte.clone()).unwrap_or_default(),
            Wahl::Linie(i) => self.plan.linien.get(i).map(|x| x.punkte.clone()).unwrap_or_default(),
            Wahl::Halt(i) => self.plan.haltlinien.get(i).map(|x| x.punkte.clone()).unwrap_or_default(),
            Wahl::Symbol(i) => self.plan.symbole.get(i).map(|x| vec![x.pos]).unwrap_or_default(),
            Wahl::Ampel(i) => self.plan.ampeln.get(i).map(|x| vec![x.pos]).unwrap_or_default(),
            Wahl::Gelaender(i) => self.plan.gelaender.get(i).map(|x| x.punkte.clone()).unwrap_or_default(),
            Wahl::Furt(i) => self.plan.furten.get(i).map(|x| vec![x.a, x.b]).unwrap_or_default(),
            Wahl::Gehpfad(i) => self.plan.gehpfade.get(i).map(|x| x.punkte.clone()).unwrap_or_default(),
        }
    }

    fn punkte_mut(&mut self, w: Wahl) -> Vec<&mut DVec2> {
        match w {
            Wahl::Flaeche(i) => self.plan.flaechen.get_mut(i).map(|x| x.punkte.iter_mut().chain(x.loecher.iter_mut().flatten()).collect()).unwrap_or_default(),
            Wahl::Linie(i) => self.plan.linien.get_mut(i).map(|x| x.punkte.iter_mut().collect()).unwrap_or_default(),
            Wahl::Halt(i) => self.plan.haltlinien.get_mut(i).map(|x| x.punkte.iter_mut().collect()).unwrap_or_default(),
            Wahl::Symbol(i) => self.plan.symbole.get_mut(i).map(|x| vec![&mut x.pos]).unwrap_or_default(),
            Wahl::Ampel(i) => self.plan.ampeln.get_mut(i).map(|x| vec![&mut x.pos]).unwrap_or_default(),
            Wahl::Gelaender(i) => self.plan.gelaender.get_mut(i).map(|x| x.punkte.iter_mut().collect()).unwrap_or_default(),
            Wahl::Furt(i) => self.plan.furten.get_mut(i).map(|x| vec![&mut x.a, &mut x.b]).unwrap_or_default(),
            Wahl::Gehpfad(i) => self.plan.gehpfade.get_mut(i).map(|x| x.punkte.iter_mut().collect()).unwrap_or_default(),
        }
    }

    /// Linienzug des Bauteils (fuer Punkte einfuegen/loeschen): (Liste, geschlossen, Mindestzahl)
    fn zug_mut(&mut self, w: Wahl) -> Option<(&mut Vec<DVec2>, bool, usize)> {
        match w {
            Wahl::Flaeche(i) => self.plan.flaechen.get_mut(i).map(|x| (&mut x.punkte, true, 3)),
            Wahl::Linie(i) => self.plan.linien.get_mut(i).map(|x| (&mut x.punkte, false, 2)),
            Wahl::Gelaender(i) => self.plan.gelaender.get_mut(i).map(|x| (&mut x.punkte, false, 2)),
            Wahl::Gehpfad(i) => self.plan.gehpfade.get_mut(i).map(|x| (&mut x.punkte, false, 2)),
            _ => None,
        }
    }

    fn richtung_mut(&mut self, w: Wahl) -> Option<&mut f64> {
        match w {
            Wahl::Symbol(i) => self.plan.symbole.get_mut(i).map(|x| &mut x.richtung),
            Wahl::Ampel(i) => self.plan.ampeln.get_mut(i).map(|x| &mut x.richtung),
            _ => None,
        }
    }

    fn verschieben(&mut self, w: Wahl, d: DVec2) {
        for p in self.punkte_mut(w) {
            *p += d;
        }
    }

    fn drehen(&mut self, w: Wahl, grad: f64) {
        if let Some(r) = self.richtung_mut(w) {
            *r = (*r + grad).rem_euclid(360.0);
            return;
        }
        let g = self.griffe(w);
        if g.is_empty() {
            return;
        }
        let c = g.iter().fold(DVec2::ZERO, |a, b| a + *b) / g.len() as f64;
        let (sn, cs) = (-grad.to_radians()).sin_cos();
        for p in self.punkte_mut(w) {
            let d = *p - c;
            *p = c + DVec2::new(d.x * cs - d.y * sn, d.x * sn + d.y * cs);
        }
    }

    fn loeschen(&mut self, w: Wahl) {
        match w {
            Wahl::Flaeche(i) => drop(self.plan.flaechen.remove(i)),
            Wahl::Linie(i) => drop(self.plan.linien.remove(i)),
            Wahl::Halt(i) => drop(self.plan.haltlinien.remove(i)),
            Wahl::Symbol(i) => drop(self.plan.symbole.remove(i)),
            Wahl::Ampel(i) => drop(self.plan.ampeln.remove(i)),
            Wahl::Gelaender(i) => drop(self.plan.gelaender.remove(i)),
            Wahl::Furt(i) => drop(self.plan.furten.remove(i)),
            Wahl::Gehpfad(i) => drop(self.plan.gehpfade.remove(i)),
        }
    }

    fn duplizieren(&mut self, w: Wahl) -> Wahl {
        let d = DVec2::new(1.0, -1.0);
        let neu = match w {
            Wahl::Flaeche(i) => {
                let mut x = self.plan.flaechen[i].clone();
                x.id = self.plan.neue_id("flaeche");
                self.plan.flaechen.push(x);
                Wahl::Flaeche(self.plan.flaechen.len() - 1)
            }
            Wahl::Linie(i) => {
                let mut x = self.plan.linien[i].clone();
                x.id = self.plan.neue_id("linie");
                self.plan.linien.push(x);
                Wahl::Linie(self.plan.linien.len() - 1)
            }
            Wahl::Halt(i) => {
                let mut x = self.plan.haltlinien[i].clone();
                x.id = self.plan.neue_id("halt");
                self.plan.haltlinien.push(x);
                Wahl::Halt(self.plan.haltlinien.len() - 1)
            }
            Wahl::Symbol(i) => {
                let mut x = self.plan.symbole[i].clone();
                x.id = self.plan.neue_id(&x.art.clone());
                self.plan.symbole.push(x);
                Wahl::Symbol(self.plan.symbole.len() - 1)
            }
            Wahl::Ampel(i) => {
                let mut x = self.plan.ampeln[i].clone();
                x.id = self.plan.neue_id("ampel");
                self.plan.ampeln.push(x);
                Wahl::Ampel(self.plan.ampeln.len() - 1)
            }
            Wahl::Gelaender(i) => {
                let mut x = self.plan.gelaender[i].clone();
                x.id = self.plan.neue_id("gelaender");
                self.plan.gelaender.push(x);
                Wahl::Gelaender(self.plan.gelaender.len() - 1)
            }
            Wahl::Furt(i) => {
                let mut x = self.plan.furten[i].clone();
                x.id = self.plan.neue_id("furt");
                self.plan.furten.push(x);
                Wahl::Furt(self.plan.furten.len() - 1)
            }
            Wahl::Gehpfad(i) => {
                let mut x = self.plan.gehpfade[i].clone();
                x.id = self.plan.neue_id("gehpfad");
                self.plan.gehpfade.push(x);
                Wahl::Gehpfad(self.plan.gehpfade.len() - 1)
            }
        };
        self.verschieben(neu, d);
        neu
    }

    /// oberstes Bauteil unter p (Toleranz in m)
    fn treffer(&self, p: DVec2, tol: f64) -> Option<Wahl> {
        let z = &self.zeige;
        if z[4] {
            if let Some(i) = self.plan.ampeln.iter().rposition(|a| (a.pos - p).length() < tol.max(0.6)) {
                return Some(Wahl::Ampel(i));
            }
        }
        if z[3] {
            if let Some(i) = self.plan.symbole.iter().rposition(|s| {
                let v = richtung_vec(s.richtung);
                let (a, b) = (s.pos, s.pos + v * 5.0 * s.skala);
                abstand_strecke(p, a, b) < tol.max(0.9)
            }) {
                return Some(Wahl::Symbol(i));
            }
        }
        if z[2] {
            if let Some(i) = self.plan.haltlinien.iter().rposition(|h| abstand_zug(p, &h.punkte, false).0 < tol + h.breite / 2.0) {
                return Some(Wahl::Halt(i));
            }
        }
        if z[6] {
            if let Some(i) = self.plan.furten.iter().rposition(|f| abstand_strecke(p, f.a, f.b) < f.breite / 2.0 + tol * 0.5) {
                return Some(Wahl::Furt(i));
            }
        }
        if z[1] {
            if let Some(i) = self.plan.linien.iter().rposition(|l| abstand_zug(p, &l.punkte, false).0 < tol + l.breite / 2.0) {
                return Some(Wahl::Linie(i));
            }
        }
        if z[5] {
            if let Some(i) = self.plan.gelaender.iter().rposition(|g| abstand_zug(p, &g.punkte, false).0 < tol) {
                return Some(Wahl::Gelaender(i));
            }
        }
        if z[7] {
            if let Some(i) = self.plan.gehpfade.iter().rposition(|g| abstand_zug(p, &g.punkte, false).0 < tol) {
                return Some(Wahl::Gehpfad(i));
            }
        }
        if z[0] {
            // oben liegende Flaechen zuerst (Markierungsflaechen, dann erhoehte, dann Fahrbahn)
            let mut kand: Vec<usize> = (0..self.plan.flaechen.len()).filter(|&i| {
                let f = &self.plan.flaechen[i];
                punkt_in(p, &f.punkte) && !f.loecher.iter().any(|l| punkt_in(p, l))
            }).collect();
            kand.sort_by_key(|&i| std::cmp::Reverse(reihenfolge(&self.plan.flaechen[i].art)));
            if let Some(&i) = kand.first() {
                return Some(Wahl::Flaeche(i));
            }
        }
        None
    }

    /// liegt Bauteil w unter p?
    fn liegt_unter(&self, w: Wahl, p: DVec2, tol: f64) -> bool {
        match w {
            Wahl::Flaeche(i) => self.plan.flaechen.get(i).map(|f| punkt_in(p, &f.punkte)).unwrap_or(false),
            Wahl::Linie(i) => self.plan.linien.get(i).map(|l| abstand_zug(p, &l.punkte, false).0 < tol + l.breite / 2.0).unwrap_or(false),
            Wahl::Halt(i) => self.plan.haltlinien.get(i).map(|h| abstand_zug(p, &h.punkte, false).0 < tol + h.breite / 2.0).unwrap_or(false),
            Wahl::Symbol(i) => self.plan.symbole.get(i).map(|s| abstand_strecke(p, s.pos, s.pos + richtung_vec(s.richtung) * 5.0 * s.skala) < tol.max(0.9)).unwrap_or(false),
            Wahl::Ampel(i) => self.plan.ampeln.get(i).map(|a| (a.pos - p).length() < tol.max(0.6)).unwrap_or(false),
            Wahl::Gelaender(i) => self.plan.gelaender.get(i).map(|g| abstand_zug(p, &g.punkte, false).0 < tol).unwrap_or(false),
            Wahl::Furt(i) => self.plan.furten.get(i).map(|f| abstand_strecke(p, f.a, f.b) < f.breite / 2.0 + tol * 0.5).unwrap_or(false),
            Wahl::Gehpfad(i) => self.plan.gehpfade.get(i).map(|g| abstand_zug(p, &g.punkte, false).0 < tol).unwrap_or(false),
        }
    }

    /// naechster Eckpunkt eines anderen Bauteils (Einrasten)
    fn einrasten_an(&self, p: DVec2, tol: f64, ausser: Wahl) -> Option<DVec2> {
        let mut best: Option<(f64, DVec2)> = None;
        let mut pruefe = |q: DVec2| {
            let d = (q - p).length();
            if d < tol && best.map(|b| d < b.0).unwrap_or(true) {
                best = Some((d, q));
            }
        };
        for (i, f) in self.plan.flaechen.iter().enumerate() {
            if ausser != Wahl::Flaeche(i) {
                f.punkte.iter().for_each(|q| pruefe(*q));
            }
        }
        for (i, l) in self.plan.linien.iter().enumerate() {
            if ausser != Wahl::Linie(i) {
                l.punkte.iter().for_each(|q| pruefe(*q));
            }
        }
        for (i, g) in self.plan.gelaender.iter().enumerate() {
            if ausser != Wahl::Gelaender(i) {
                g.punkte.iter().for_each(|q| pruefe(*q));
            }
        }
        for a in &self.kontext.arme {
            pruefe(a.links);
            pruefe(a.rechts);
        }
        best.map(|b| b.1)
    }

    fn neues(&mut self, art: &str, c: DVec2) {
        self.aendern();
        let q = |dx: f64, dy: f64| c + DVec2::new(dx, dy);
        let w = match art {
            "flaeche" => {
                let a = FLAECHEN_ARTEN[self.neue_art].0.to_string();
                let id = self.plan.neue_id("flaeche");
                self.plan.flaechen.push(Flaeche { id, art: a, punkte: vec![q(-2.0, -2.0), q(2.0, -2.0), q(2.0, 2.0), q(-2.0, 2.0)], loecher: vec![],
                                                  textur: Textur { dx: 0.0, dy: 0.0, skala: 1.0, dreh: 0.0 } });
                Wahl::Flaeche(self.plan.flaechen.len() - 1)
            }
            "linie" | "strich" => {
                let id = self.plan.neue_id("linie");
                let (an, aus) = if art == "strich" { (3.0, 6.0) } else { (0.0, 0.0) };
                self.plan.linien.push(Linie { id, punkte: vec![q(-3.0, 0.0), q(3.0, 0.0)], breite: 0.12, an, aus });
                Wahl::Linie(self.plan.linien.len() - 1)
            }
            "halt" => {
                let id = self.plan.neue_id("halt");
                let arm = self.kontext.arme.iter().enumerate().min_by(|a, b| (a.1.pos - c).length().partial_cmp(&(b.1.pos - c).length()).unwrap()).map(|x| x.0).unwrap_or(0);
                self.plan.haltlinien.push(Haltlinie { id, arm, punkte: vec![q(-1.75, 0.0), q(1.75, 0.0)], breite: 0.5 });
                Wahl::Halt(self.plan.haltlinien.len() - 1)
            }
            "pfeil" | "bus" => {
                let id = self.plan.neue_id(art);
                self.plan.symbole.push(Symbol { id, art: art.into(), pos: c, richtung: 0.0, pfeile: if art == "pfeil" { vec!["gerade".into()] } else { vec![] }, skala: 1.0 });
                Wahl::Symbol(self.plan.symbole.len() - 1)
            }
            "kfz" | "fg" => {
                let id = self.plan.neue_id("ampel");
                self.plan.ampeln.push(Ampel { id, art: art.into(), pos: c, richtung: 0.0, mast: "klein".into(), gruppe: 0 });
                Wahl::Ampel(self.plan.ampeln.len() - 1)
            }
            "gelaender" => {
                let id = self.plan.neue_id("gelaender");
                self.plan.gelaender.push(Gelaender { id, punkte: vec![q(-2.0, 0.0), q(2.0, 0.0)], unten: "gelaende".into(), flip: false });
                Wahl::Gelaender(self.plan.gelaender.len() - 1)
            }
            "furt" => {
                let id = self.plan.neue_id("furt");
                self.plan.furten.push(Furt { id, a: q(-4.0, 0.0), b: q(4.0, 0.0), breite: 3.0, gruppe: 0 });
                Wahl::Furt(self.plan.furten.len() - 1)
            }
            _ => {
                let id = self.plan.neue_id("gehpfad");
                self.plan.gehpfade.push(Gehpfad { id, punkte: vec![q(-3.0, 0.0), q(3.0, 0.0)] });
                Wahl::Gehpfad(self.plan.gehpfade.len() - 1)
            }
        };
        self.wahl = Some(w);
        self.abschliessen();
    }

    // ---------------------------------------------------------- Fenster
    pub fn ui(&mut self, ctx: &egui::Context, offen: &mut bool) -> Option<Aktion> {
        let mut erg = None;
        // Hintergrundarbeiten
        if let Some(rx) = &self.luft_job {
            if let Ok(r) = rx.try_recv() {
                self.luft_job = None;
                if let Some((img, b)) = r {
                    self.luftbild = Some((ctx.load_texture("oe_luftbild", img, egui::TextureOptions::LINEAR), b));
                }
            }
        }
        if let Some(rx) = &self.job {
            if let Ok(r) = rx.try_recv() {
                self.job = None;
                match r {
                    Ok(t) => {
                        self.kontext = Kontext::lesen(&self.json);
                        if self.plan.flaechen.is_empty() {
                            if let Ok(b) = std::fs::read(&self.plan_pfad) {
                                if let Ok(v) = serde_json::from_slice(&b) {
                                    self.plan = Bauplan::aus_json(&v);
                                }
                            }
                        }
                        self.texturen.clear();
                        self.geaendert = false;
                        self.meldung = format!("Neu gebaut: {t}");
                        erg = Some(Aktion::Gebaut { json: self.json.clone(), einsetzen: self.einsetzen });
                    }
                    Err(e) => self.meldung = format!("Bauen fehlgeschlagen: {e}"),
                }
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        // feste Groesse nach dem Bildschirm (ein Fenster, dessen Inhalt den Platz fuellt, wuerde sonst immer weiter wachsen)
        let scr = ctx.content_rect();
        let groesse = egui::vec2((scr.width() - 60.0).clamp(500.0, 1500.0), (scr.height() - 120.0).clamp(300.0, 880.0));
        egui::Window::new("Objekt-Editor: Sonderbau Ring / Kreuzung Ludwigsallee").open(offen).fixed_size(groesse).default_pos(scr.min + egui::vec2(20.0, 50.0)).show(ctx, |ui| {
            // links die Zeichenflaeche, rechts die Leiste
            let (w, h) = (groesse.x, groesse.y);
            ui.horizontal_top(|ui| {
                let cw = (w - 330.0 - 3.0 * ui.spacing().item_spacing.x - 12.0).max(200.0);
                ui.allocate_ui_with_layout(egui::vec2(cw, h), egui::Layout::top_down(egui::Align::Min), |ui| self.zeichenflaeche(ui));
                ui.separator();
                ui.allocate_ui_with_layout(egui::vec2(330.0, h), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(330.0);
                    ui.set_max_width(330.0);
                    egui::ScrollArea::vertical().id_salt("oe_leiste").show(ui, |ui| {
                        if let Some(a) = self.seitenleiste(ui) {
                            erg = Some(a);
                        }
                    });
                });
            });
        });
        if !ctx.input(|i| i.pointer.any_down()) {
            self.abschliessen();
        }
        erg
    }

    fn seitenleiste(&mut self, ui: &mut egui::Ui) -> Option<Aktion> {
        let mut erg = None;
        ui.horizontal_wrapped(|ui| {
            let laeuft = self.job.is_some();
            if ui.add_enabled(!laeuft, egui::Button::new(egui::RichText::new("Anwenden").strong())).on_hover_text("Bauplan speichern, alle Objekte neu bauen und (wenn angehakt) in die Karte setzen").clicked() {
                self.bauen(true);
            }
            if laeuft {
                ui.spinner();
            }
            if ui.button("Speichern").on_hover_text("nur den Bauplan speichern").clicked() {
                self.meldung = match self.speichern() {
                    Ok(()) => format!("Bauplan gespeichert: {}", self.plan_pfad.display()),
                    Err(e) => e,
                };
            }
            if ui.add_enabled(!self.undo.is_empty(), egui::Button::new("Rueckgaengig")).on_hover_text("Rueckgaengig (im Editor)").clicked() {
                self.rueckgaengig();
            }
            if ui.add_enabled(!self.redo.is_empty(), egui::Button::new("Wiederholen")).on_hover_text("Wiederholen").clicked() {
                self.wiederholen();
            }
            if ui.button("Kamera").on_hover_text("die 3D-Ansicht auf die Kreuzung richten").clicked() {
                erg = Some(Aktion::Kamera(DVec3::new(self.mitte.x, self.mitte.y, 10.0)));
            }
        });
        ui.checkbox(&mut self.einsetzen, "beim Anwenden in die Karte setzen").on_hover_text("ersetzt Strassen und Objekte an der Stelle (Sitzung, Rueckgaengig, Speichern wie gewohnt)");
        if self.geaendert {
            ui.colored_label(egui::Color32::from_rgb(255, 200, 80), "nicht angewendete Aenderungen");
        }
        if !self.meldung.is_empty() {
            ui.label(egui::RichText::new(&self.meldung).small());
        }
        if self.kontext.lose > 0 {
            ui.colored_label(egui::Color32::from_rgb(255, 120, 90), format!("{} Ampelteile stehen nicht auf Gehweg/Insel", self.kontext.lose));
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Luftbild");
            ui.add(egui::Slider::new(&mut self.luft_alpha, 0.0..=1.0).show_value(false));
            ui.checkbox(&mut self.texturen_echt, "Texturen");
            ui.checkbox(&mut self.einrasten, "Einrasten");
        });
        ui.horizontal_wrapped(|ui| {
            for (i, n) in EBENEN.iter().enumerate() {
                ui.checkbox(&mut self.zeige[i], *n);
            }
        });
        ui.separator();
        ui.label(egui::RichText::new("Neu (in der Mitte der Ansicht)").strong());
        let c = self.mitte;
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("oe_neue_art").selected_text(FLAECHEN_ARTEN[self.neue_art].1).show_ui(ui, |ui| {
                for (i, (_, n)) in FLAECHEN_ARTEN.iter().enumerate() {
                    ui.selectable_value(&mut self.neue_art, i, *n);
                }
            });
            if ui.button("+ Flaeche").clicked() {
                self.neues("flaeche", c);
            }
        });
        ui.horizontal_wrapped(|ui| {
            for (k, t) in [("linie", "Linie"), ("strich", "Leitlinie"), ("halt", "Haltlinie"), ("pfeil", "Pfeil"), ("bus", "BUS"), ("kfz", "Kfz-Ampel"),
                           ("fg", "Fussg.-Ampel"), ("gelaender", "Gelaender"), ("furt", "Furt"), ("gehpfad", "Gehpfad")] {
                if ui.small_button(format!("+ {t}")).clicked() {
                    self.neues(k, c);
                }
            }
        });
        ui.separator();
        if let Some(w) = self.wahl {
            self.eigenschaften(ui, w);
        } else {
            ui.label(egui::RichText::new("Bauteil anklicken (oder unten in der Liste waehlen).").weak());
        }
        ui.separator();
        self.bauwerk_ui(ui);
        ui.separator();
        self.liste(ui);
        ui.separator();
        ui.label(egui::RichText::new("Bedienung: Klick waehlt, Ziehen verschiebt, Eckpunkte ziehen. Doppelklick auf eine Kante fuegt einen Punkt ein, \
            Rechtsklick auf einen Punkt loescht ihn. Alt+Ziehen verschiebt die Textur einer Flaeche, Alt+Rad skaliert sie, Strg+Rad dreht \
            (Textur bzw. Pfeil/Ampel). Rechte Maustaste/Mitte: Ansicht schieben, Rad: Zoom. Pfeiltasten: 5 cm (Umschalt 50 cm), Entf loescht.").small().weak());
        erg
    }

    fn eigenschaften(&mut self, ui: &mut egui::Ui, w: Wahl) {
        let mut weg = false;
        let mut dup = false;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(match w {
                Wahl::Flaeche(_) => "Flaeche", Wahl::Linie(_) => "Linie", Wahl::Halt(_) => "Haltlinie", Wahl::Symbol(_) => "Symbol",
                Wahl::Ampel(_) => "Ampel", Wahl::Gelaender(_) => "Gelaender", Wahl::Furt(_) => "Fussgaengerfurt", Wahl::Gehpfad(_) => "Gehpfad",
            }).strong());
            if ui.small_button("duplizieren").clicked() {
                dup = true;
            }
            if ui.small_button("loeschen").clicked() {
                weg = true;
            }
            for (t, g) in [("-15\u{B0}", -15.0), ("-1\u{B0}", -1.0), ("+1\u{B0}", 1.0), ("+15\u{B0}", 15.0)] {
                if ui.small_button(t).on_hover_text("drehen (um die Mitte bzw. die Richtung)").clicked() {
                    self.aendern();
                    self.drehen(w, g);
                }
            }
        });
        if dup {
            self.aendern();
            self.wahl = Some(self.duplizieren(w));
            return;
        }
        if weg {
            self.aendern();
            self.loeschen(w);
            self.wahl = None;
            return;
        }
        let mut ae = false;
        let dv = |ui: &mut egui::Ui, v: &mut f64, speed: f64, txt: &str| ui.add(egui::DragValue::new(v).speed(speed).prefix(txt)).changed();
        match w {
            Wahl::Flaeche(i) => {
                let Some(fl) = self.plan.flaechen.get_mut(i) else { return };
                egui::ComboBox::from_id_salt("oe_f_art").selected_text(FLAECHEN_ARTEN.iter().find(|a| a.0 == fl.art).map(|a| a.1).unwrap_or(&fl.art)).show_ui(ui, |ui| {
                    for (k, n) in FLAECHEN_ARTEN {
                        if ui.selectable_label(fl.art == k, n).clicked() {
                            fl.art = k.to_string();
                            ae = true;
                        }
                    }
                });
                ui.label(format!("{} Eckpunkte, {} Loecher, {:.1} m\u{B2}", fl.punkte.len(), fl.loecher.len(), flaeche2(&fl.punkte).abs()));
                ui.label(egui::RichText::new("Textur").strong());
                ui.horizontal(|ui| {
                    ae |= dv(ui, &mut fl.textur.dx, 0.01, "u ");
                    ae |= dv(ui, &mut fl.textur.dy, 0.01, "v ");
                });
                ui.horizontal(|ui| {
                    ae |= ui.add(egui::DragValue::new(&mut fl.textur.skala).speed(0.01).range(0.05..=20.0).prefix("Skala ")).changed();
                    ae |= ui.add(egui::DragValue::new(&mut fl.textur.dreh).speed(0.5).suffix("\u{B0}").prefix("Drehung ")).changed();
                    if ui.small_button("zuruecksetzen").clicked() {
                        fl.textur = Textur { dx: 0.0, dy: 0.0, skala: 1.0, dreh: 0.0 };
                        ae = true;
                    }
                });
            }
            Wahl::Linie(i) => {
                let Some(l) = self.plan.linien.get_mut(i) else { return };
                ui.horizontal(|ui| {
                    ae |= ui.add(egui::DragValue::new(&mut l.breite).speed(0.005).range(0.05..=2.0).prefix("Breite ").suffix(" m")).changed();
                });
                ui.horizontal(|ui| {
                    ae |= ui.add(egui::DragValue::new(&mut l.an).speed(0.05).range(0.0..=20.0).prefix("Strich ").suffix(" m")).changed();
                    ae |= ui.add(egui::DragValue::new(&mut l.aus).speed(0.05).range(0.0..=20.0).prefix("Luecke ").suffix(" m")).changed();
                });
                ui.horizontal_wrapped(|ui| {
                    for (t, b, a, u) in [("durchgezogen", 0.12, 0.0, 0.0), ("Leitlinie 3/6", 0.12, 3.0, 6.0), ("Leitlinie 6/3", 0.12, 6.0, 3.0),
                                         ("Radfurt-Rand", 0.15, 0.5, 0.5), ("Breitstrich", 0.25, 0.0, 0.0), ("Abbieger 1,5/1,5", 0.12, 1.5, 1.5)] {
                        if ui.small_button(t).clicked() {
                            l.breite = b;
                            l.an = a;
                            l.aus = u;
                            ae = true;
                        }
                    }
                });
            }
            Wahl::Halt(i) => {
                let arme: Vec<String> = self.kontext.arme.iter().map(|a| a.name.clone()).collect();
                let Some(h) = self.plan.haltlinien.get_mut(i) else { return };
                egui::ComboBox::from_id_salt("oe_h_arm").selected_text(arme.get(h.arm).cloned().unwrap_or_else(|| format!("Arm {}", h.arm))).show_ui(ui, |ui| {
                    for (k, n) in arme.iter().enumerate() {
                        if ui.selectable_label(h.arm == k, n).clicked() {
                            h.arm = k;
                            ae = true;
                        }
                    }
                });
                ae |= ui.add(egui::DragValue::new(&mut h.breite).speed(0.01).range(0.1..=1.0).prefix("Breite ").suffix(" m")).changed();
                ui.label(egui::RichText::new("Die Fahrzeuge dieser Zufahrt halten an der Linie.").small().weak());
            }
            Wahl::Symbol(i) => {
                let Some(sy) = self.plan.symbole.get_mut(i) else { return };
                ui.horizontal(|ui| {
                    ae |= ui.selectable_value(&mut sy.art, "pfeil".to_string(), "Pfeil").changed();
                    ae |= ui.selectable_value(&mut sy.art, "bus".to_string(), "BUS").changed();
                });
                if sy.art == "pfeil" {
                    ui.horizontal(|ui| {
                        for k in ["links", "gerade", "rechts"] {
                            let mut an = sy.pfeile.iter().any(|x| x == k);
                            if ui.checkbox(&mut an, k).changed() {
                                sy.pfeile.retain(|x| x != k);
                                if an {
                                    sy.pfeile.push(k.to_string());
                                }
                                ae = true;
                            }
                        }
                    });
                }
                ui.horizontal(|ui| {
                    ae |= ui.add(egui::DragValue::new(&mut sy.richtung).speed(0.5).suffix("\u{B0}").prefix("Richtung ")).changed();
                    ae |= ui.add(egui::DragValue::new(&mut sy.skala).speed(0.01).range(0.3..=3.0).prefix("Groesse ")).changed();
                });
            }
            Wahl::Ampel(i) => {
                let Some(a) = self.plan.ampeln.get_mut(i) else { return };
                ui.horizontal(|ui| {
                    ae |= ui.selectable_value(&mut a.art, "kfz".to_string(), "Kfz").changed();
                    ae |= ui.selectable_value(&mut a.art, "fg".to_string(), "Fussgaenger").changed();
                    ui.separator();
                    ae |= ui.selectable_value(&mut a.mast, "klein".to_string(), "kleiner Mast").changed();
                    ae |= ui.selectable_value(&mut a.mast, "ausleger".to_string(), "Ausleger").changed();
                });
                ui.horizontal(|ui| {
                    ae |= ui.add(egui::DragValue::new(&mut a.richtung).speed(0.5).suffix("\u{B0}").prefix("blickt nach ")).changed();
                    ae |= ui.add(egui::DragValue::new(&mut a.gruppe).range(0..=9).prefix("Phase ")).on_hover_text("Signalgruppe (0 = erste Phase)").changed();
                });
                ui.label(egui::RichText::new("Der Mast steht 0,46 m hinter dem Signal; Ausleger ragen nach links ueber die Fahrbahn.").small().weak());
            }
            Wahl::Gelaender(i) => {
                let Some(g) = self.plan.gelaender.get_mut(i) else { return };
                ui.horizontal(|ui| {
                    ae |= ui.selectable_value(&mut g.unten, "gelaende".to_string(), "Wand bis zum Gelaende").changed();
                    ae |= ui.selectable_value(&mut g.unten, "trog".to_string(), "Trogrand").changed();
                });
                ae |= ui.checkbox(&mut g.flip, "Seite tauschen").on_hover_text("Wand/Gelaender auf die andere Seite der Linie").changed();
            }
            Wahl::Furt(i) => {
                let Some(fu) = self.plan.furten.get_mut(i) else { return };
                ui.horizontal(|ui| {
                    ae |= ui.add(egui::DragValue::new(&mut fu.breite).speed(0.05).range(1.0..=10.0).prefix("Breite ").suffix(" m")).changed();
                    ae |= ui.add(egui::DragValue::new(&mut fu.gruppe).range(0..=9).prefix("Phase ")).changed();
                });
            }
            Wahl::Gehpfad(_) => {
                ui.label(egui::RichText::new("Fussgaengerpfad (Punkte ziehen, Doppelklick fuegt ein)").small().weak());
            }
        }
        // Punkte als Zahlen
        egui::CollapsingHeader::new("Punkte").id_salt(("oe_punkte", format!("{w:?}"))).show(ui, |ui| {
            let n = self.griffe(w).len();
            for k in 0..n.min(60) {
                ui.horizontal(|ui| {
                    ui.label(format!("{k}"));
                    let mut pts = self.punkte_mut(w);
                    if let Some(p) = pts.get_mut(k) {
                        ae |= ui.add(egui::DragValue::new(&mut p.x).speed(0.02).prefix("x ")).changed();
                        ae |= ui.add(egui::DragValue::new(&mut p.y).speed(0.02).prefix("y ")).changed();
                    }
                });
            }
        });
        if ae {
            self.aendern();
        }
    }

    fn bauwerk_ui(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Bauwerk: Laermschutzwaende, Bahntunnel, Ampelphasen").show(ui, |ui| {
            let mut ae = false;
            let mut lsw = self.plan.rest.get("lsw").cloned().unwrap_or(serde_json::json!({"w": [], "o": []}));
            for (seite, name) in [("w", "West (gruen)"), ("o", "Ost (Glas)")] {
                ui.label(format!("Laermschutzwand {name}, von/bis Station (m entlang der Ring-Achse, Nord = 0)"));
                let mut liste: Vec<[f64; 2]> = lsw[seite].as_array().map(|a| a.iter().map(|x| [x[0].as_f64().unwrap_or(0.0), x[1].as_f64().unwrap_or(0.0)]).collect()).unwrap_or_default();
                let mut weg = None;
                for (k, b) in liste.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ae |= ui.add(egui::DragValue::new(&mut b[0]).speed(0.5).range(0.0..=400.0)).changed();
                        ae |= ui.add(egui::DragValue::new(&mut b[1]).speed(0.5).range(0.0..=400.0)).changed();
                        if ui.small_button("x").clicked() {
                            weg = Some(k);
                        }
                    });
                }
                if let Some(k) = weg {
                    liste.remove(k);
                    ae = true;
                }
                if ui.small_button("+ Bereich").clicked() {
                    liste.push([0.0, 20.0]);
                    ae = true;
                }
                lsw[seite] = serde_json::json!(liste);
            }
            let mut tun = self.plan.rest.get("tunnel").cloned().unwrap_or(serde_json::json!({"x0": 1054.0, "x1": 1066.0, "sohle": 3.3}));
            ui.label("Bahntunnel unter der Kreuzung (Waende bei x, Sohle)");
            ui.horizontal(|ui| {
                for k in ["x0", "x1", "sohle"] {
                    let mut v = tun[k].as_f64().unwrap_or(0.0);
                    if ui.add(egui::DragValue::new(&mut v).speed(0.05).prefix(format!("{k} "))).changed() {
                        tun[k] = v.into();
                        ae = true;
                    }
                }
            });
            ui.label("Ampelphase je Zufahrt (Arm)");
            let mut phasen: Vec<i64> = self.plan.rest.get("phasen").and_then(|v| v.as_array()).map(|a| a.iter().map(|x| x.as_i64().unwrap_or(0)).collect()).unwrap_or_default();
            for (k, p) in phasen.iter_mut().enumerate() {
                let n = self.kontext.arme.get(k).map(|a| a.name.clone()).unwrap_or_else(|| format!("Arm {k}"));
                ui.horizontal(|ui| {
                    ui.label(n);
                    ae |= ui.add(egui::DragValue::new(p).range(0..=9)).changed();
                });
            }
            if ae {
                self.aendern();
                self.plan.rest.insert("lsw".into(), lsw);
                self.plan.rest.insert("tunnel".into(), tun);
                self.plan.rest.insert("phasen".into(), serde_json::json!(phasen));
            }
        });
    }

    fn liste(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Alle Bauteile").show(ui, |ui| {
            let mut neu = None;
            let eintrag = |ui: &mut egui::Ui, aktiv: bool, t: String| ui.selectable_label(aktiv, t).clicked();
            for (i, x) in self.plan.flaechen.iter().enumerate() {
                if eintrag(ui, self.wahl == Some(Wahl::Flaeche(i)), format!("Flaeche {} ({})", x.id, x.art)) {
                    neu = Some(Wahl::Flaeche(i));
                }
            }
            for (i, x) in self.plan.linien.iter().enumerate() {
                if eintrag(ui, self.wahl == Some(Wahl::Linie(i)), format!("Linie {} ({:.2} m{})", x.id, x.breite, if x.aus > 0.0 { ", gestrichelt" } else { "" })) {
                    neu = Some(Wahl::Linie(i));
                }
            }
            for (i, x) in self.plan.haltlinien.iter().enumerate() {
                if eintrag(ui, self.wahl == Some(Wahl::Halt(i)), format!("Haltlinie {} (Arm {})", x.id, x.arm)) {
                    neu = Some(Wahl::Halt(i));
                }
            }
            for (i, x) in self.plan.symbole.iter().enumerate() {
                if eintrag(ui, self.wahl == Some(Wahl::Symbol(i)), format!("{} {} {}", if x.art == "bus" { "BUS" } else { "Pfeil" }, x.id, x.pfeile.join("/"))) {
                    neu = Some(Wahl::Symbol(i));
                }
            }
            for (i, x) in self.plan.ampeln.iter().enumerate() {
                if eintrag(ui, self.wahl == Some(Wahl::Ampel(i)), format!("Ampel {} ({}, {}, Phase {})", x.id, x.art, x.mast, x.gruppe)) {
                    neu = Some(Wahl::Ampel(i));
                }
            }
            for (i, x) in self.plan.gelaender.iter().enumerate() {
                if eintrag(ui, self.wahl == Some(Wahl::Gelaender(i)), format!("Gelaender {} ({})", x.id, x.unten)) {
                    neu = Some(Wahl::Gelaender(i));
                }
            }
            for (i, x) in self.plan.furten.iter().enumerate() {
                if eintrag(ui, self.wahl == Some(Wahl::Furt(i)), format!("Furt {}", x.id)) {
                    neu = Some(Wahl::Furt(i));
                }
            }
            for (i, x) in self.plan.gehpfade.iter().enumerate() {
                if eintrag(ui, self.wahl == Some(Wahl::Gehpfad(i)), format!("Gehpfad {}", x.id)) {
                    neu = Some(Wahl::Gehpfad(i));
                }
            }
            if let Some(w) = neu {
                self.wahl = Some(w);
                let g = self.griffe(w);
                if !g.is_empty() {
                    self.mitte = g.iter().fold(DVec2::ZERO, |a, b| a + *b) / g.len() as f64;
                }
            }
        });
    }

    // ---------------------------------------------------------- Zeichenflaeche
    fn textur(&mut self, ctx: &egui::Context, datei: &str) -> Option<egui::TextureId> {
        if !self.texturen.contains_key(datei) {
            let pfad = self.ordner().join("texture").join(datei);
            let t = image::open(&pfad).ok().map(|i| {
                let i = i.to_rgba8();
                let img = egui::ColorImage::from_rgba_unmultiplied([i.width() as usize, i.height() as usize], i.as_raw());
                ctx.load_texture(format!("oe_{datei}"), img, egui::TextureOptions { wrap_mode: egui::TextureWrapMode::Repeat, ..egui::TextureOptions::LINEAR })
            });
            self.texturen.insert(datei.to_string(), t);
        }
        self.texturen.get(datei).and_then(|t| t.as_ref().map(|t| t.id()))
    }

    fn zeichenflaeche(&mut self, ui: &mut egui::Ui) {
        let groesse = ui.available_size();
        let (resp, p) = ui.allocate_painter(groesse, egui::Sense::click_and_drag());
        let rect = resp.rect;
        let c = rect.center();
        let (mitte, m) = (self.mitte, self.m_px);
        let bild = move |w: DVec2| egui::pos2(c.x + ((w.x - mitte.x) as f32) * m, c.y - ((w.y - mitte.y) as f32) * m);
        let welt = move |s: egui::Pos2| DVec2::new(mitte.x + ((s.x - c.x) / m) as f64, mitte.y - ((s.y - c.y) / m) as f64);
        p.rect_filled(rect, 0.0, egui::Color32::from_gray(24));
        // Luftbild
        if let Some((t, b)) = &self.luftbild {
            if self.luft_alpha > 0.01 {
                let r = egui::Rect::from_two_pos(bild(DVec2::new(b[0], b[3])), bild(DVec2::new(b[2], b[1])));
                p.image(t.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::from_white_alpha((self.luft_alpha * 255.0) as u8));
            }
        }
        let deck = if self.luftbild.is_some() { (255.0 * (1.0 - self.luft_alpha * 0.55)) as u8 } else { 255 };
        // Trogoeffnungen (fest, aus dem Entwurf)
        for o in &self.kontext.offen {
            let pts: Vec<egui::Pos2> = o.iter().map(|q| bild(*q)).collect();
            p.add(egui::Shape::closed_line(pts, egui::Stroke::new(1.5, egui::Color32::from_rgb(120, 120, 140))));
        }
        // Flaechen in Zeichenreihenfolge, mit Texturen in der Lage wie im Objekt
        if self.zeige[0] {
            let mut ord: Vec<usize> = (0..self.plan.flaechen.len()).collect();
            ord.sort_by_key(|&i| reihenfolge(&self.plan.flaechen[i].art));
            for i in ord {
                let fl = self.plan.flaechen[i].clone();
                let ring = mit_loechern(&fl.punkte, &fl.loecher);
                let tris = dreiecke(&ring);
                let (datei, m_tex) = material(&fl.art);
                let tex = if self.texturen_echt { self.textur(ui.ctx(), datei) } else { None };
                let farbe = match fl.art.as_str() {
                    "fahrbahn" => egui::Color32::from_rgb(70, 72, 76),
                    "gehweg" => egui::Color32::from_rgb(150, 150, 150),
                    "insel" => egui::Color32::from_rgb(190, 185, 170),
                    "gruen" => egui::Color32::from_rgb(80, 130, 60),
                    "sperr" => egui::Color32::from_rgb(230, 230, 230),
                    "radfurt" => egui::Color32::from_rgb(176, 62, 52),
                    _ => egui::Color32::from_rgb(170, 170, 165),
                };
                let mut mesh = match tex {
                    Some(id) => egui::epaint::Mesh::with_texture(id),
                    None => egui::epaint::Mesh::default(),
                };
                let sk = m_tex * fl.textur.skala.max(0.05);
                let (sn, cs) = fl.textur.dreh.to_radians().sin_cos();
                let a = if fl.art == "sperr" && tex.is_none() { 120 } else { deck };
                for q in &ring {
                    let uv = egui::pos2(((cs * q.x + sn * q.y) / sk + fl.textur.dx) as f32, -((-sn * q.x + cs * q.y) / sk + fl.textur.dy) as f32);
                    let col = if tex.is_some() { egui::Color32::from_white_alpha(a) } else { egui::Color32::from_rgba_unmultiplied(farbe.r(), farbe.g(), farbe.b(), a) };
                    mesh.vertices.push(egui::epaint::Vertex { pos: bild(*q), uv: if tex.is_some() { uv } else { egui::epaint::WHITE_UV }, color: col });
                }
                for t in tris {
                    mesh.indices.extend([t[0] as u32, t[1] as u32, t[2] as u32]);
                }
                p.add(egui::Shape::mesh(mesh));
                let rand = match fl.art.as_str() {
                    "gehweg" | "insel" | "gruen" => egui::Stroke::new(1.5, egui::Color32::from_rgb(210, 210, 200)),
                    _ => egui::Stroke::new(0.6, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 50)),
                };
                p.add(egui::Shape::closed_line(fl.punkte.iter().map(|q| bild(*q)).collect(), rand));
            }
        }
        let weiss = egui::Color32::from_rgb(245, 245, 240);
        let breite_px = |b: f64| ((b as f32) * m).max(1.0);
        // Linien
        if self.zeige[1] {
            for l in &self.plan.linien {
                let pts: Vec<egui::Pos2> = l.punkte.iter().map(|q| bild(*q)).collect();
                let st = egui::Stroke::new(breite_px(l.breite), weiss);
                if l.aus > 0.0 && l.an > 0.0 {
                    p.extend(egui::Shape::dashed_line(&pts, st, (l.an as f32) * m, (l.aus as f32) * m));
                } else {
                    p.add(egui::Shape::line(pts, st));
                }
            }
        }
        if self.zeige[2] {
            for h in &self.plan.haltlinien {
                p.add(egui::Shape::line(h.punkte.iter().map(|q| bild(*q)).collect(), egui::Stroke::new(breite_px(h.breite), weiss)));
                if let (Some(a), Some(arm)) = (h.punkte.first(), self.kontext.arme.get(h.arm)) {
                    p.text(bild(*a), egui::Align2::RIGHT_BOTTOM, format!("{}", arm.name.replace('_', " ")), egui::FontId::proportional(10.0), egui::Color32::from_rgb(255, 220, 120));
                }
            }
        }
        if self.zeige[6] {
            for f in &self.plan.furten {
                let d = (f.b - f.a).normalize_or_zero();
                let nq = DVec2::new(-d.y, d.x) * f.breite / 2.0;
                for sgn in [1.0, -1.0] {
                    p.extend(egui::Shape::dashed_line(&[bild(f.a + nq * sgn), bild(f.b + nq * sgn)], egui::Stroke::new(breite_px(0.12), weiss), 0.5 * m, 0.2 * m));
                }
                p.add(egui::Shape::line_segment([bild(f.a), bild(f.b)], egui::Stroke::new(1.0, egui::Color32::from_rgb(120, 220, 255))));
            }
        }
        // Symbole: Pfeile und BUS
        if self.zeige[3] {
            for sy in &self.plan.symbole {
                let v = richtung_vec(sy.richtung) * sy.skala;
                let r = rechts_vec(sy.richtung) * sy.skala;
                let w = |a: f64, b: f64| bild(sy.pos + v * b + r * a);
                let st = egui::Stroke::new(breite_px(0.18), weiss);
                if sy.art == "bus" {
                    p.add(egui::Shape::closed_line(vec![w(-1.5, 0.0), w(1.5, 0.0), w(1.5, 2.4), w(-1.5, 2.4)], egui::Stroke::new(1.0, weiss)));
                    p.text(w(0.0, 1.2), egui::Align2::CENTER_CENTER, "BUS", egui::FontId::proportional((1.6 * m).clamp(8.0, 40.0)), weiss);
                    continue;
                }
                p.line_segment([w(0.0, 0.0), w(0.0, 2.6)], st);
                if sy.pfeile.iter().any(|x| x == "gerade") || sy.pfeile.is_empty() {
                    p.line_segment([w(0.0, 2.5), w(0.0, 3.9)], st);
                    p.add(egui::Shape::convex_polygon(vec![w(-0.35, 3.8), w(0.0, 5.0), w(0.35, 3.8)], weiss, egui::Stroke::NONE));
                }
                for (k, sgn) in [("links", -1.0), ("rechts", 1.0)] {
                    if sy.pfeile.iter().any(|x| x == k) {
                        p.line_segment([w(0.0, 2.4), w(sgn * 0.7, 3.3)], st);
                        p.add(egui::Shape::convex_polygon(vec![w(sgn * 0.45, 3.35), w(sgn * 1.15, 3.85), w(sgn * 0.95, 2.95)], weiss, egui::Stroke::NONE));
                    }
                }
            }
        }
        // Gelaender (rosa, Wandseite angedeutet)
        if self.zeige[5] {
            for g in &self.plan.gelaender {
                let pts: Vec<egui::Pos2> = g.punkte.iter().map(|q| bild(*q)).collect();
                p.add(egui::Shape::line(pts, egui::Stroke::new(3.0, egui::Color32::from_rgb(220, 110, 160))));
            }
        }
        if self.zeige[7] {
            for g in &self.plan.gehpfade {
                let pts: Vec<egui::Pos2> = g.punkte.iter().map(|q| bild(*q)).collect();
                p.extend(egui::Shape::dashed_line(&pts, egui::Stroke::new(1.2, egui::Color32::from_rgb(120, 220, 255)), 4.0, 3.0));
            }
        }
        // Ampeln: Signal (Kreis), Blickrichtung, Mast dahinter, Ausleger
        if self.zeige[4] {
            for a in &self.plan.ampeln {
                let v = richtung_vec(a.richtung);
                let mast = a.pos - v * 0.46;
                let farbe = if a.art == "fg" { egui::Color32::from_rgb(90, 220, 120) } else { egui::Color32::from_rgb(255, 80, 70) };
                p.line_segment([bild(mast), bild(a.pos + v * 1.6)], egui::Stroke::new(2.0, farbe));
                if a.mast == "ausleger" {
                    let l = -rechts_vec(a.richtung);
                    p.line_segment([bild(mast), bild(mast + l * 4.0)], egui::Stroke::new(2.0, egui::Color32::from_rgb(160, 170, 170)));
                }
                p.circle_filled(bild(mast), 3.0, egui::Color32::from_gray(200));
                p.circle(bild(a.pos), 5.0, farbe, egui::Stroke::new(1.0, egui::Color32::BLACK));
                p.text(bild(a.pos) + egui::vec2(6.0, -6.0), egui::Align2::LEFT_BOTTOM, format!("{}", a.gruppe + 1), egui::FontId::proportional(11.0), egui::Color32::WHITE);
            }
        }
        // Armenden (Anschluss an die Strassen, fest)
        for a in &self.kontext.arme {
            p.line_segment([bild(a.links), bild(a.rechts)], egui::Stroke::new(2.5, egui::Color32::from_rgb(255, 160, 40)));
        }
        // Auswahl
        if let Some(w) = self.wahl {
            let g = self.griffe(w);
            let gelb = egui::Color32::from_rgb(255, 230, 40);
            match w {
                Wahl::Flaeche(_) => {
                    p.add(egui::Shape::closed_line(g.iter().map(|q| bild(*q)).collect(), egui::Stroke::new(2.0, gelb)));
                }
                Wahl::Symbol(_) | Wahl::Ampel(_) => {
                    p.circle_stroke(bild(g[0]), 10.0, egui::Stroke::new(2.0, gelb));
                }
                _ => {
                    p.add(egui::Shape::line(g.iter().map(|q| bild(*q)).collect(), egui::Stroke::new(1.5, gelb)));
                }
            }
            for q in &g {
                p.rect_filled(egui::Rect::from_center_size(bild(*q), egui::vec2(7.0, 7.0)), 1.0, gelb);
            }
        }
        // ------------------------------------------------------ Bedienung
        let maus = resp.hover_pos().map(welt);
        let tol = 6.0 / m as f64;
        let alt = ui.input(|i| i.modifiers.alt);
        let strg = ui.input(|i| i.modifiers.ctrl || i.modifiers.command);
        let (gezogen_l, gezogen_r) = (ui.input(|i| i.pointer.primary_down()), ui.input(|i| i.pointer.secondary_down() || i.pointer.middle_down()));
        if resp.drag_started() {
            // vom Druckpunkt aus (beim Erkennen des Ziehens ist die Maus schon ein Stueck weiter)
            let start = ui.input(|i| i.pointer.press_origin()).map(welt).or(maus);
            if let Some(q) = start {
                if gezogen_r {
                    self.zug = Zug::Schwenken;
                } else {
                    let griff = self.wahl.and_then(|w| self.griffe(w).iter().position(|g| (*g - q).length() < tol * 1.3));
                    self.zug = match (griff, self.wahl) {
                        (Some(k), Some(_)) if !alt => Zug::Griff(k),
                        (_, Some(Wahl::Flaeche(_))) if alt => Zug::Textur,
                        _ => {
                            // das gewaehlte Bauteil hat Vorrang, wenn es unter der Maus liegt
                            let t = if self.wahl.map(|w| self.liegt_unter(w, q, tol)).unwrap_or(false) { self.wahl } else { self.treffer(q, tol) };
                            if t.is_some() {
                                if t != self.wahl {
                                    self.wahl = t;
                                }
                                if alt && matches!(t, Some(Wahl::Flaeche(_))) { Zug::Textur } else { Zug::Verschieben }
                            } else {
                                Zug::Schwenken
                            }
                        }
                    };
                }
                self.letzte = Some(q);
            }
        }
        if resp.dragged() && (gezogen_l || gezogen_r) {
            if self.zug == Zug::Schwenken {
                let dd = resp.drag_delta();
                self.mitte -= DVec2::new((dd.x / m) as f64, -(dd.y / m) as f64);
            } else if let (Some(q), Some(l)) = (maus, self.letzte) {
                let d = q - l;
                match (self.zug, self.wahl) {
                    (Zug::Griff(k), Some(w)) => {
                        self.aendern();
                        let mut ziel = q;
                        if self.einrasten && !strg {
                            if let Some(s) = self.einrasten_an(q, tol * 1.5, w) {
                                ziel = s;
                            }
                        }
                        if let Some(pk) = self.punkte_mut(w).into_iter().nth(k) {
                            *pk = ziel;
                        }
                    }
                    (Zug::Verschieben, Some(w)) => {
                        self.aendern();
                        self.verschieben(w, d);
                    }
                    (Zug::Textur, Some(Wahl::Flaeche(i))) => {
                        self.aendern();
                        if let Some(fl) = self.plan.flaechen.get_mut(i) {
                            let sk = material(&fl.art).1 * fl.textur.skala.max(0.05);
                            let (sn, cs) = fl.textur.dreh.to_radians().sin_cos();
                            // die Textur folgt der Maus: uv am verschobenen Punkt bleibt gleich
                            fl.textur.dx -= (cs * d.x + sn * d.y) / sk;
                            fl.textur.dy -= (-sn * d.x + cs * d.y) / sk;
                        }
                    }
                    _ => {}
                }
                self.letzte = Some(q);
            }
        }
        if resp.drag_stopped() {
            self.zug = Zug::Nichts;
            self.letzte = None;
        }
        if resp.clicked() {
            if let Some(q) = maus {
                self.wahl = self.treffer(q, tol);
            }
        }
        if resp.double_clicked() {
            if let (Some(q), Some(w)) = (maus, self.wahl) {
                if let Some((zug, zu, _)) = self.zug_mut(w) {
                    let (d, k) = abstand_zug(q, zug, zu);
                    if d < tol * 2.0 {
                        self.aendern();
                        if let Some((zug, _, _)) = self.zug_mut(w) {
                            zug.insert(k + 1, q);
                        }
                    }
                }
            }
        }
        if resp.secondary_clicked() {
            if let (Some(q), Some(w)) = (maus, self.wahl) {
                if let Some(k) = self.griffe(w).iter().position(|g| (*g - q).length() < tol * 1.3) {
                    let geht = self.zug_mut(w).map(|(z, _, min)| z.len() > min).unwrap_or(false);
                    if geht {
                        self.aendern();
                        if let Some((zug, _, _)) = self.zug_mut(w) {
                            zug.remove(k);
                        }
                    }
                }
            }
        }
        if resp.hovered() {
            // Mausrad als einzelne Rasten (rohe Ereignisse: Strg/Alt aendern sie sonst in Zoom/seitliches Scrollen)
            let rad: f32 = ui.input(|i| i.raw.events.iter().map(|e| match e {
                egui::Event::MouseWheel { delta, unit, .. } => {
                    let d = if delta.y.abs() > 0.0 { delta.y } else { delta.x };
                    match unit {
                        egui::MouseWheelUnit::Point => d / 50.0,
                        egui::MouseWheelUnit::Page => d * 10.0,
                        _ => d,
                    }
                }
                _ => 0.0,
            }).sum());
            if rad != 0.0 {
                match (alt, strg, self.wahl) {
                    (true, _, Some(Wahl::Flaeche(i))) => {
                        self.aendern();
                        if let Some(fl) = self.plan.flaechen.get_mut(i) {
                            fl.textur.skala = (fl.textur.skala * if rad > 0.0 { 1.05 } else { 1.0 / 1.05 }).clamp(0.05, 20.0);
                        }
                    }
                    (false, true, Some(Wahl::Flaeche(i))) => {
                        self.aendern();
                        if let Some(fl) = self.plan.flaechen.get_mut(i) {
                            fl.textur.dreh += if rad > 0.0 { 5.0 } else { -5.0 };
                        }
                    }
                    (false, true, Some(w)) => {
                        self.aendern();
                        self.drehen(w, if rad > 0.0 { 5.0 } else { -5.0 });
                    }
                    _ => {
                        // Zoom um die Maus
                        let vor = maus;
                        self.m_px = (self.m_px * 1.15f32.powf(rad)).clamp(1.0, 200.0);
                        if let (Some(v), Some(hp)) = (vor, resp.hover_pos()) {
                            let nach = DVec2::new(self.mitte.x + ((hp.x - c.x) / self.m_px) as f64, self.mitte.y - ((hp.y - c.y) / self.m_px) as f64);
                            self.mitte += v - nach;
                        }
                    }
                }
            }
            // Tasten
            if let Some(w) = self.wahl {
                let schritt = if ui.input(|i| i.modifiers.shift) { 0.5 } else { 0.05 };
                let mut d = DVec2::ZERO;
                ui.input(|i| {
                    if i.key_pressed(egui::Key::ArrowLeft) { d.x -= schritt; }
                    if i.key_pressed(egui::Key::ArrowRight) { d.x += schritt; }
                    if i.key_pressed(egui::Key::ArrowUp) { d.y += schritt; }
                    if i.key_pressed(egui::Key::ArrowDown) { d.y -= schritt; }
                });
                if d != DVec2::ZERO {
                    self.aendern();
                    self.verschieben(w, d);
                    self.abschliessen();
                }
                if ui.input(|i| i.key_pressed(egui::Key::Delete)) {
                    self.aendern();
                    self.loeschen(w);
                    self.wahl = None;
                    self.abschliessen();
                }
            }
        }
        // Massstab und Mausposition
        if let Some(q) = maus {
            p.text(rect.left_bottom() + egui::vec2(8.0, -8.0), egui::Align2::LEFT_BOTTOM, format!("x {:.2}  y {:.2}   {:.0} px/m", q.x, q.y, m),
                   egui::FontId::monospace(11.0), egui::Color32::from_gray(220));
        }
        p.text(rect.right_bottom() + egui::vec2(-8.0, -8.0), egui::Align2::RIGHT_BOTTOM, "Luftbild: DOP40 (c) Bayerische Vermessungsverwaltung, CC BY 4.0",
               egui::FontId::proportional(10.0), egui::Color32::from_gray(180));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bauplan_hin_und_zurueck() {
        let j = serde_json::json!({
            "version": 1, "name": "K", "phasen": [0, 1], "_id": 5,
            "flaechen": [{"id": "f1", "art": "sperr", "punkte": [[0, 0], [4, 0], [4, 3]], "loecher": [], "textur": {"dx": 0.5, "dy": 0, "skala": 2, "dreh": 10}}],
            "linien": [{"id": "l1", "punkte": [[0, 0], [1, 1]], "breite": 0.12, "an": 3, "aus": 6}],
            "haltlinien": [], "symbole": [{"id": "p1", "art": "pfeil", "pos": [1, 2], "richtung": 90, "pfeile": ["links"], "skala": 1}],
            "ampeln": [{"id": "a1", "art": "kfz", "pos": [5, 5], "richtung": 180, "mast": "ausleger", "gruppe": 2}],
            "gelaender": [], "furten": [], "gehpfade": []
        });
        let mut b = Bauplan::aus_json(&j);
        assert_eq!(b.flaechen[0].textur, Textur { dx: 0.5, dy: 0.0, skala: 2.0, dreh: 10.0 });
        let id = b.neue_id("x");
        assert_eq!(id, "x6");
        let k = b.json();
        assert_eq!(k["phasen"], serde_json::json!([0, 1]));
        assert_eq!(k["ampeln"][0]["mast"], "ausleger");
        assert_eq!(Bauplan::aus_json(&k).symbole[0].pfeile, vec!["links".to_string()]);
    }

    #[test]
    fn dreiecke_mit_loch() {
        let aussen = vec![DVec2::new(0.0, 0.0), DVec2::new(10.0, 0.0), DVec2::new(10.0, 10.0), DVec2::new(0.0, 10.0)];
        let loch = vec![DVec2::new(4.0, 4.0), DVec2::new(6.0, 4.0), DVec2::new(6.0, 6.0), DVec2::new(4.0, 6.0)];
        let ring = mit_loechern(&aussen, &[loch]);
        let t = dreiecke(&ring);
        let summe: f64 = t.iter().map(|d| flaeche2(&[ring[d[0]], ring[d[1]], ring[d[2]]]).abs()).sum();
        assert!((summe - 96.0).abs() < 1e-6, "{summe}");
        // nicht konvex (L-Form)
        let l = vec![DVec2::new(0.0, 0.0), DVec2::new(4.0, 0.0), DVec2::new(4.0, 1.0), DVec2::new(1.0, 1.0), DVec2::new(1.0, 4.0), DVec2::new(0.0, 4.0)];
        let t = dreiecke(&l);
        let summe: f64 = t.iter().map(|d| flaeche2(&[l[d[0]], l[d[1]], l[d[2]]]).abs()).sum();
        assert!((summe - 7.0).abs() < 1e-9);
    }

    #[test]
    fn treffer_und_verschieben() {
        let mut ed = ObjektEditor {
            plan_pfad: PathBuf::new(), json: PathBuf::new(), root: PathBuf::new(), karte: String::new(), plan: Bauplan::default(),
            kontext: Kontext::default(), wahl: None, mitte: DVec2::ZERO, m_px: 10.0, zug: Zug::Nichts, letzte: None, undo: vec![], redo: vec![],
            offen_aenderung: None, geaendert: false, job: None, meldung: String::new(), luftbild: None, luft_job: None, luft_alpha: 0.5,
            texturen: Default::default(), texturen_echt: false, einsetzen: false, einrasten: true, neue_art: 0, zeige: [true; 8],
        };
        ed.neues("flaeche", DVec2::new(10.0, 10.0));
        ed.neues("kfz", DVec2::new(11.0, 10.0));
        assert_eq!(ed.treffer(DVec2::new(11.1, 10.0), 0.3), Some(Wahl::Ampel(0)));
        assert_eq!(ed.treffer(DVec2::new(9.0, 9.0), 0.3), Some(Wahl::Flaeche(0)));
        ed.aendern();
        ed.verschieben(Wahl::Flaeche(0), DVec2::new(1.0, 0.0));
        ed.abschliessen();
        assert!((ed.plan.flaechen[0].punkte[0].x - 9.0).abs() < 1e-9);
        ed.rueckgaengig();
        assert!((ed.plan.flaechen[0].punkte[0].x - 8.0).abs() < 1e-9);
        ed.wiederholen();
        assert!((ed.plan.flaechen[0].punkte[0].x - 9.0).abs() < 1e-9);
        ed.aendern();
        ed.drehen(Wahl::Ampel(0), 90.0);
        assert!((ed.plan.ampeln[0].richtung - 90.0).abs() < 1e-9);
    }
}
