//! Objekt-Editor fuer Sonderbauten (zuerst der Sonderbau Ring / Kreuzung Ludwigsallee): jedes Bauteil einzeln
//! bearbeiten - Flaechen (Fahrbahn, Gehweg, Insel, Gruen, Sperrflaeche, Radfurt, Busspur-Beton) mit Eckpunkten und
//! Textur-Lage, Linien, Haltlinien, Pfeile, BUS-Schrift, Ampeln, Gelaender, Fussgaengerfurten, Gehpfade, die Waende
//! des Bauwerks (Trenn- und Aussenwaende: Bereich, Material, Laermschutz, Versatz, Hoehe) und die Strassen
//! (Querschnitt im Baukasten, Versatz).
//!
//! Bedient wird direkt in der 3D-Ansicht (die Leiste rechts zeigt Werkzeuge und Eigenschaften); auf Wunsch zusaetzlich in
//! einem 2D-Plan ueber dem Luftbild mit den echten Texturen. Bearbeitet wird nur der Bauplan
//! (`sonderbau/ring/bauplan.json`, Format in omsigen/ring_bauplan.py); "Anwenden" (oder automatisch) baut daraus mit
//! omsigen alle Objekte neu und setzt den Sonderbau in die Karte (Sitzung, Rueckgaengig, Speichern wie gewohnt).
//!
//! Dazu die Bauaufnahme (aufnahme.rs): das echte Bauwerk, nachgezeichnet ueber dem Luftbild, und der Abgleich mit dem
//! Gebauten (was fehlt, was daneben liegt, was zuviel ist).

use crate::aufnahme::{self, ALinie, Abweichung, Aufnahme, Gebaut, Notiz};
use glam::{DVec2, DVec3};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

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

/// Wand des Bauwerks: Bereich entlang der Ring-Achse (Stationen von/bis) auf einer Seite
#[derive(Clone, Debug)]
pub struct Wand {
    pub id: String,
    /// "w" oder "o"
    pub seite: String,
    /// "trenn" (zwischen Ring und Rampe) oder "aussen"
    pub art: String,
    pub von: f64,
    pub bis: f64,
    /// klinker | beton | gabionen
    pub material: String,
    /// keine | gruen | glas
    pub laerm: String,
    /// m nach aussen (weg vom Ring)
    pub versatz: f64,
    /// m hoeher als die Fahrbahn bzw. das Gelaende dahinter
    pub hoehe: f64,
    /// ab 3,5 m zweistufig mit Pflanzstreifen (Aussenwand Klinker)
    pub stufig: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Bauplan {
    /// weitere Felder (Name, Abbiegespuren, Phasen, Tunnel, Strassen, Zaehler) - Strassen werden hier bearbeitet
    pub rest: serde_json::Map<String, serde_json::Value>,
    pub flaechen: Vec<Flaeche>,
    pub linien: Vec<Linie>,
    pub haltlinien: Vec<Haltlinie>,
    pub symbole: Vec<Symbol>,
    pub ampeln: Vec<Ampel>,
    pub gelaender: Vec<Gelaender>,
    pub furten: Vec<Furt>,
    pub gehpfade: Vec<Gehpfad>,
    pub waende: Vec<Wand>,
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
        for k in ["flaechen", "linien", "haltlinien", "symbole", "ampeln", "gelaender", "furten", "gehpfade", "waende"] {
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
            waende: liste("waende").iter().map(|v| Wand {
                id: s(v, "id"), seite: s(v, "seite"), art: s(v, "art"), von: f(v, "von", 0.0), bis: f(v, "bis", 0.0),
                material: s(v, "material"), laerm: if s(v, "laerm").is_empty() { "keine".into() } else { s(v, "laerm") },
                versatz: f(v, "versatz", 0.0), hoehe: f(v, "hoehe", 0.0), stufig: v["stufig"].as_bool().unwrap_or(true),
            }).collect(),
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
        o.insert("waende".into(), self.waende.iter().map(|x| serde_json::json!({
            "id": x.id, "seite": x.seite, "art": x.art, "von": r(x.von), "bis": r(x.bis), "material": x.material, "laerm": x.laerm,
            "versatz": r(x.versatz), "hoehe": r(x.hoehe), "stufig": x.stufig,
        })).collect());
        serde_json::Value::Object(o)
    }

    fn neue_id(&mut self, art: &str) -> String {
        let n = self.rest.get("_id").and_then(|v| v.as_u64()).unwrap_or(1000) + 1;
        self.rest.insert("_id".into(), n.into());
        format!("{art}{n}")
    }

    /// Strassen des Sonderbaus (Name, Querschnitt, Versatz) in Bauplan-Reihenfolge
    pub fn strassen(&self) -> Vec<(String, serde_json::Value, Option<f64>)> {
        self.rest.get("strassen").and_then(|v| v.as_object()).map(|o| o.iter().map(|(k, v)| (k.clone(), v["querschnitt"].clone(), v["versatz"].as_f64())).collect()).unwrap_or_default()
    }

    fn strasse_mut(&mut self, name: &str) -> Option<&mut serde_json::Value> {
        self.rest.get_mut("strassen").and_then(|v| v.as_object_mut()).and_then(|o| o.get_mut(name))
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
    Wand(usize),
    Strasse(usize),
    /// Linie der Bauaufnahme
    Aufnahme(usize),
    /// Notiz der Bauaufnahme
    Notiz(usize),
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
        "sperr" => ("AB_sperrflaeche.bmp", 4.0),
        _ => ("AB_radfurt.bmp", 4.0),
    }
}

fn farbe_art(art: &str) -> egui::Color32 {
    match art {
        "fahrbahn" => egui::Color32::from_rgb(70, 72, 76),
        "gehweg" => egui::Color32::from_rgb(150, 150, 150),
        "insel" => egui::Color32::from_rgb(190, 185, 170),
        "gruen" => egui::Color32::from_rgb(80, 130, 60),
        "sperr" => egui::Color32::from_rgb(230, 230, 230),
        "radfurt" => egui::Color32::from_rgb(176, 62, 52),
        _ => egui::Color32::from_rgb(170, 170, 165),
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
    pub links: DVec2,
    pub rechts: DVec2,
}

/// Station entlang der Ring-Achse: Punkt, Querrichtung (nach Osten), Lage der Wandlinien ohne Versatz
#[derive(Clone, Debug, Default)]
pub struct Station {
    pub s: f64,
    pub p: DVec2,
    pub q: DVec2,
    pub t: std::collections::HashMap<String, f64>,
}

#[derive(Clone, Debug, Default)]
pub struct Raster {
    x0: f64,
    y0: f64,
    schritt: f64,
    nx: usize,
    ny: usize,
    werte: Vec<f64>,
}

impl Raster {
    fn h(&self, p: DVec2) -> Option<f64> {
        if self.werte.is_empty() || self.schritt <= 0.0 {
            return None;
        }
        let fx = (p.x - self.x0) / self.schritt;
        let fy = (p.y - self.y0) / self.schritt;
        if fx < 0.0 || fy < 0.0 || fx >= (self.nx - 1) as f64 || fy >= (self.ny - 1) as f64 {
            return None;
        }
        let (i, j) = (fx.floor() as usize, fy.floor() as usize);
        let (u, v) = (fx - i as f64, fy - j as f64);
        let w = |a: usize, b: usize| self.werte[b * self.nx + a];
        Some(w(i, j) * (1.0 - u) * (1.0 - v) + w(i + 1, j) * u * (1.0 - v) + w(i, j + 1) * (1.0 - u) * v + w(i + 1, j + 1) * u * v)
    }
}

/// Fund des Bauprüfers (omsigen/baupruefer.py, baupruefung.json neben sonderbau_ring.json)
#[derive(Clone, Debug, Default)]
pub struct Fund {
    pub art: String,
    pub text: String,
    pub teil: String,
    pub pos: DVec3,
    pub anzahl: usize,
    /// wand_luecke: Wand-Bauteil (id), Ende ("Anfang"/"Ende") und Abstand zum Anschlussteil
    pub bauteil: String,
    pub ende: String,
    pub abstand: f64,
}

pub fn fund_art(art: &str) -> &str {
    match art {
        "textur_verzerrt" => "Textur verzerrt",
        "dreieck_entartet" => "Dreieck ohne Flaeche",
        "doppelte_flaeche" => "Flaechen uebereinander (Flimmern)",
        "spalt" => "Spalt zwischen Teilen",
        "offenes_ende" => "offenes Wandende",
        "ampel_schwebt" => "Ampel schwebt",
        "ampel_versinkt" => "Ampel im Boden",
        "ampel_ohne_mast" => "Ampel ohne Mast",
        "grube" => "Loch im Gelaende",
        "wand_luecke" => "Luecke am Wandende",
        a => a,
    }
}

#[derive(Clone, Debug, Default)]
pub struct Kontext {
    pub funde: Vec<Fund>,
    pub offen: Vec<Vec<DVec2>>,
    pub arme: Vec<Arm>,
    pub stationen: Vec<Station>,
    pub strassen: Vec<(String, Vec<DVec2>)>,
    pub hoehen: Raster,
    pub lose: usize,
}

impl Kontext {
    fn lesen(datei: &Path) -> Kontext {
        let Ok(b) = std::fs::read(datei) else { return Kontext::default() };
        let Ok(j) = serde_json::from_slice::<serde_json::Value>(&b) else { return Kontext::default() };
        let k = &j["kontext"];
        let h = &k["hoehen"];
        let funde = std::fs::read(datei.with_file_name("baupruefung.json")).ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|b| b["funde"].as_array().map(|a| a.iter().map(|f| Fund {
                art: s(f, "art"), text: s(f, "text"), teil: s(f, "teil"),
                pos: DVec3::new(f["pos"][0].as_f64().unwrap_or(0.0), f["pos"][1].as_f64().unwrap_or(0.0), f["pos"][2].as_f64().unwrap_or(0.0)),
                anzahl: f["anzahl"].as_u64().unwrap_or(1) as usize,
                bauteil: s(f, "bauteil"), ende: s(f, "ende"), abstand: f["abstand"].as_f64().unwrap_or(0.0),
            }).collect()))
            .unwrap_or_default();
        Kontext {
            funde,
            offen: k["offen"].as_array().map(|a| a.iter().map(pl).collect()).unwrap_or_default(),
            arme: k["arme"].as_array().map(|a| a.iter().map(|v| Arm { name: s(v, "name"), pos: p2(&v["pos"]), links: p2(&v["links"]), rechts: p2(&v["rechts"]) }).collect()).unwrap_or_default(),
            stationen: k["stationen"].as_array().map(|a| a.iter().map(|v| Station {
                s: f(v, "s", 0.0), p: p2(&v["p"]), q: p2(&v["q"]),
                t: v["t"].as_object().map(|o| o.iter().filter_map(|(k, x)| x.as_f64().map(|x| (k.clone(), x))).collect()).unwrap_or_default(),
            }).collect()).unwrap_or_default(),
            strassen: k["strassen"].as_array().map(|a| a.iter().map(|v| (s(v, "name"), pl(&v["punkte"]))).collect()).unwrap_or_default(),
            hoehen: Raster {
                x0: f(h, "x0", 0.0), y0: f(h, "y0", 0.0), schritt: f(h, "schritt", 0.0), nx: h["nx"].as_u64().unwrap_or(0) as usize,
                ny: h["ny"].as_u64().unwrap_or(0) as usize, werte: h["werte"].as_array().map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect()).unwrap_or_default(),
            },
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
    /// Kamera nah an diesen Punkt (Fund des Bauprüfers)
    KameraNah(DVec3),
    /// Querschnitt dieser Strasse im Baukasten bearbeiten (Baukasten-JSON)
    Querschnitt(String, serde_json::Value),
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Zug {
    Nichts,
    Griff(usize),
    Verschieben,
    Textur,
    Schwenken,
}

/// Werkzeug in der Leiste
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Werkzeug {
    Auswahl,
    /// Textur der gewaehlten Flaeche mit der Maus verschieben
    Textur,
    /// neue Flaeche Ecke fuer Ecke klicken
    Flaeche,
    Linie,
    Gelaender,
    Gehpfad,
    /// Linie der Bauaufnahme (Art in der Leiste)
    Aufnahme,
    /// Notiz der Bauaufnahme per Klick
    Notiz,
}

impl Werkzeug {
    fn zeichnet(self) -> bool {
        matches!(self, Werkzeug::Flaeche | Werkzeug::Linie | Werkzeug::Gelaender | Werkzeug::Gehpfad | Werkzeug::Aufnahme | Werkzeug::Notiz)
    }
}

pub struct ObjektEditor {
    pub plan_pfad: PathBuf,
    json: PathBuf,
    root: PathBuf,
    karte: String,
    pub plan: Bauplan,
    kontext: Kontext,
    pub wahl: Option<Wahl>,
    unter: Option<Wahl>,
    pub werkzeug: Werkzeug,
    /// Eckpunkte des Bauteils, das gerade gezeichnet wird
    entwurf: Vec<DVec2>,
    zug: Zug,
    letzte: Option<DVec2>,
    undo: Vec<String>,
    redo: Vec<String>,
    offen_aenderung: Option<String>,
    pub geaendert: bool,
    zuletzt_geaendert: Instant,
    job: Option<Receiver<Result<String, String>>>,
    pub meldung: String,
    luftbild: Option<(egui::TextureHandle, [f64; 4])>,
    luft_job: Option<Receiver<Option<(egui::ColorImage, [f64; 4])>>>,
    luft_alpha: f32,
    texturen: std::collections::HashMap<String, Option<egui::TextureHandle>>,
    einsetzen: bool,
    pub automatisch: bool,
    einrasten: bool,
    neue_art: usize,
    zeige: [bool; 11],
    /// in der 3D-Ansicht bearbeiten (sonst nur im 2D-Plan)
    pub drei_d: bool,
    /// Flaechen in der 3D-Ansicht farbig fuellen
    pub fuellen_3d: bool,
    /// 2D-Plan (Fenster) zeigen
    pub plan2d: bool,
    mitte: DVec2,
    m_px: f32,
    /// Funde des Bauprüfers in 3D markieren
    pub funde_zeigen: bool,
    fund_wahl: Option<usize>,
    /// Bauaufnahme (echtes Bauwerk) neben dem Bauplan
    pub aufnahme: Aufnahme,
    aufnahme_pfad: PathBuf,
    /// Art fuer das Werkzeug Aufnahme (Index in aufnahme::ARTEN)
    aufnahme_art: usize,
    abweichungen: Vec<Abweichung>,
    /// Stand fuer den Abgleich: zaehlt bei jeder Aenderung hoch
    rev: u64,
    abw_rev: u64,
    pub abgleich_zeigen: bool,
    abw_wahl: Option<usize>,
    bezug: Option<crate::geo::Bezug>,
    /// Klicks treffen zuerst die Bauaufnahme (sonst erst den Bauplan)
    pub aufnahme_vorne: bool,
}

const EBENEN: [&str; 11] = ["Flaechen", "Linien", "Haltlinien", "Symbole", "Ampeln", "Gelaender", "Furten", "Gehpfade", "Waende", "Strassen", "Bauaufnahme"];
/// Ebene der Bauaufnahme in `zeige`
const AUFNAHME: usize = 10;

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
        let mut ed = ObjektEditor::leer();
        ed.plan_pfad = plan_pfad;
        ed.json = json;
        ed.root = root.to_path_buf();
        ed.karte = karte.to_string();
        ed.plan = plan;
        ed.kontext = kontext;
        ed.aufnahme_pfad = ed.plan_pfad.with_file_name("aufnahme.json");
        ed.aufnahme = Aufnahme::lesen(&ed.aufnahme_pfad);
        ed.bezug = crate::geo::Bezug::lesen(&root.join("maps").join(karte));
        if ed.plan.flaechen.is_empty() || ed.kontext.arme.is_empty() || ed.kontext.stationen.is_empty() {
            ed.bauen(false);
            ed.meldung = "Erzeugen des Sonderbaus laeuft (Bauplan und Kontext) ...".into();
        }
        ed.luftbild_laden(root);
        Ok(ed)
    }

    fn leer() -> ObjektEditor {
        ObjektEditor {
            plan_pfad: PathBuf::new(), json: PathBuf::new(), root: PathBuf::new(), karte: String::new(), plan: Bauplan::default(),
            kontext: Kontext::default(), wahl: None, unter: None, werkzeug: Werkzeug::Auswahl, entwurf: vec![], zug: Zug::Nichts,
            letzte: None, undo: vec![], redo: vec![], offen_aenderung: None, geaendert: false, zuletzt_geaendert: Instant::now(), job: None,
            meldung: String::new(), luftbild: None, luft_job: None, luft_alpha: 0.55, texturen: Default::default(), einsetzen: true,
            automatisch: false, einrasten: true, neue_art: 0, zeige: [true; 11], drei_d: true, fuellen_3d: false, plan2d: false,
            mitte: DVec2::new(1084.0, -265.0), m_px: 6.0, funde_zeigen: true, fund_wahl: None,
            aufnahme: Aufnahme::default(), aufnahme_pfad: PathBuf::new(), aufnahme_art: 1, abweichungen: vec![], rev: 1, abw_rev: 0,
            abgleich_zeigen: true, abw_wahl: None, bezug: None, aufnahme_vorne: true,
        }
    }

    fn luftbild_laden(&mut self, root: &Path) {
        let Some(b) = crate::geo::Bezug::lesen(&root.join("maps").join(&self.karte)) else { return };
        let (tx, rx) = channel();
        let bereich = [1030.0, -440.0, 1140.0, -120.0];
        std::thread::spawn(move || {
            let erg = crate::geo::ausschnitt(&b, bereich, 1600).ok().flatten().and_then(|p| image::open(p).ok()).map(|i| {
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

    /// Hoehe der Kreuzungsflaeche (Raster aus dem letzten Bauen), sonst None
    pub fn hoehe(&self, p: DVec2) -> Option<f64> {
        self.kontext.hoehen.h(p)
    }

    // ---------------------------------------------------------- Aenderungen und Rueckgaengig
    fn stand(&self) -> String {
        serde_json::json!({"plan": self.plan.json(), "aufnahme": self.aufnahme.json()}).to_string()
    }

    fn stand_setzen(&mut self, s: &str) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(s) {
            self.plan = Bauplan::aus_json(&v["plan"]);
            self.aufnahme = Aufnahme::aus_json(&v["aufnahme"]);
        }
        self.rev += 1;
    }

    /// vor einer Aenderung aufrufen: merkt den Stand fuer Rueckgaengig (einmal je Bedienvorgang)
    fn aendern(&mut self) {
        if self.offen_aenderung.is_none() {
            self.offen_aenderung = Some(self.stand());
        }
        self.geaendert = true;
        self.zuletzt_geaendert = Instant::now();
        self.rev += 1;
    }

    /// Bedienvorgang abschliessen (Maus losgelassen): Stand fuer Rueckgaengig ablegen
    pub fn abschliessen(&mut self) {
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

    pub fn rueckgaengig(&mut self) {
        self.abschliessen();
        if let Some(s) = self.undo.pop() {
            self.redo.push(self.stand());
            self.stand_setzen(&s);
            self.wahl = None;
            self.geaendert = true;
            self.zuletzt_geaendert = Instant::now();
        }
    }

    pub fn wiederholen(&mut self) {
        if let Some(s) = self.redo.pop() {
            self.undo.push(self.stand());
            self.stand_setzen(&s);
            self.wahl = None;
            self.geaendert = true;
            self.zuletzt_geaendert = Instant::now();
        }
    }

    pub fn speichern(&mut self) -> Result<(), String> {
        let text = serde_json::to_string_pretty(&self.plan.json()).map_err(|e| e.to_string())?;
        if let Some(d) = self.plan_pfad.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        std::fs::write(&self.plan_pfad, text).map_err(|e| format!("{}: {e}", self.plan_pfad.display()))?;
        if !self.aufnahme_pfad.as_os_str().is_empty() && (self.aufnahme != Aufnahme::default() || self.aufnahme_pfad.is_file()) {
            self.aufnahme.schreiben(&self.aufnahme_pfad)?;
        }
        Ok(())
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
                    Ok(t.lines().filter(|l| l.starts_with("Kreuzung:") || l.contains("Objekte,") || l.starts_with("Baupruefung:")).collect::<Vec<_>>().join(" | "))
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

    /// Querschnitt einer Strasse aus dem Baukasten uebernehmen
    pub fn querschnitt_setzen(&mut self, name: &str, q: serde_json::Value) {
        self.aendern();
        if let Some(st) = self.plan.strasse_mut(name) {
            st["querschnitt"] = q;
        }
        self.abschliessen();
        self.meldung = format!("Querschnitt von {name} uebernommen - Anwenden baut neu");
    }

    // ---------------------------------------------------------- Bauaufnahme
    /// das Gebaute fuer den Abgleich: Borde (Umrisse erhoehter Flaechen), Markierungen, Haltlinien, Furten, Sperrflaechen,
    /// Radfurten, Gelaender (auch auf den Trennwaenden) und Mauern (Wandlinien des letzten Baus)
    fn gebaut(&self) -> Vec<Gebaut> {
        let zu = |pts: &[DVec2]| {
            let mut v = pts.to_vec();
            if v.len() > 2 {
                v.push(v[0]);
            }
            v
        };
        let mut g = Vec::new();
        for f_ in &self.plan.flaechen {
            let art = match f_.art.as_str() {
                "gehweg" | "insel" | "gruen" => "bord",
                "sperr" => "sperr",
                "radfurt" => "radfurt",
                _ => continue,
            };
            g.push(Gebaut { art, punkte: zu(&f_.punkte), name: f_.id.clone() });
            for l in &f_.loecher {
                g.push(Gebaut { art, punkte: zu(l), name: f_.id.clone() });
            }
        }
        for l in &self.plan.linien {
            let art = if l.breite >= 0.4 { "haltlinie" } else if l.an > 0.0 && l.aus > 0.0 { "leitlinie" } else { "randlinie" };
            g.push(Gebaut { art, punkte: l.punkte.clone(), name: l.id.clone() });
        }
        for h in &self.plan.haltlinien {
            g.push(Gebaut { art: "haltlinie", punkte: h.punkte.clone(), name: h.id.clone() });
        }
        for f_ in &self.plan.furten {
            g.push(Gebaut { art: "furt", punkte: vec![f_.a, f_.b], name: f_.id.clone() });
        }
        for x in &self.plan.gelaender {
            g.push(Gebaut { art: "gelaender", punkte: x.punkte.clone(), name: x.id.clone() });
        }
        for w in &self.plan.waende {
            let l = self.wand_linie(w);
            if l.len() < 2 {
                continue;
            }
            if w.art == "trenn" {
                g.push(Gebaut { art: "gelaender", punkte: l.clone(), name: w.id.clone() });
            }
            g.push(Gebaut { art: "mauer", punkte: l, name: w.id.clone() });
        }
        g
    }

    /// Abgleich neu, wenn sich Bauplan, Aufnahme oder Kontext geaendert haben
    fn abgleich_aktuell(&mut self) {
        if self.abw_rev == self.rev {
            return;
        }
        self.abw_rev = self.rev;
        let g = self.gebaut();
        let platte: Vec<&Vec<DVec2>> = self.plan.flaechen.iter().map(|f_| &f_.punkte).collect();
        let auf_platte = |q: DVec2| platte.iter().any(|f_| punkt_in(q, f_));
        self.abweichungen = aufnahme::abgleich(&self.aufnahme, &g, &auf_platte);
        if self.abw_wahl.is_some_and(|i| i >= self.abweichungen.len()) {
            self.abw_wahl = None;
        }
    }

    /// Kartenpunkt -> (Breite, Laenge), nur fuer Karten mit Ort
    fn wgs(&self, q: DVec2) -> Option<(f64, f64)> {
        self.bezug.as_ref().map(|b| aufnahme::utm_zu_wgs(b.ost0 + q.x, b.nord0 + q.y, b.zone))
    }

    /// Knoepfe "Mapillary" / "Panoramax": freie Strassenbilder an q im Browser
    fn bilder_knoepfe(&self, ui: &mut egui::Ui, q: DVec2) {
        match self.wgs(q) {
            Some((lat, lon)) => {
                for (name, url) in aufnahme::bilder_links(lat, lon) {
                    if ui.small_button(format!("\u{1F4F7} {name}")).on_hover_text(format!("Strassenbilder hier im Browser oeffnen ({lat:.5}, {lon:.5})")).clicked() {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                    }
                }
            }
            None => {
                ui.label(egui::RichText::new("Strassenbilder nur fuer Karten mit Ort").small().weak());
            }
        }
    }

    /// Aufnahme-Linie i als Bauteil in den Bauplan (Gelaender, Markierung, Haltlinie, Furt, Sperrflaeche, Radfurt)
    pub fn uebernehmen(&mut self, i: usize) -> String {
        let Some(l) = self.aufnahme.linien.get(i).cloned() else { return "keine Aufnahme-Linie".into() };
        let pts = l.punkte.clone();
        if pts.len() < 2 {
            return "zu wenige Punkte".into();
        }
        let neu = match l.art.as_str() {
            "gelaender" => {
                self.aendern();
                let id = self.plan.neue_id("gelaender");
                self.plan.gelaender.push(Gelaender { id, punkte: pts, unten: "gelaende".into(), flip: false });
                Wahl::Gelaender(self.plan.gelaender.len() - 1)
            }
            "randlinie" | "leitlinie" => {
                self.aendern();
                let id = self.plan.neue_id("linie");
                let (an, aus) = if l.art == "leitlinie" { (3.0, 6.0) } else { (0.0, 0.0) };
                self.plan.linien.push(Linie { id, punkte: pts, breite: 0.12, an, aus });
                Wahl::Linie(self.plan.linien.len() - 1)
            }
            "haltlinie" => {
                self.aendern();
                let c = (pts[0] + pts[pts.len() - 1]) / 2.0;
                let arm = self.kontext.arme.iter().enumerate().min_by(|a, b| (a.1.pos - c).length().total_cmp(&(b.1.pos - c).length())).map(|x| x.0).unwrap_or(0);
                let id = self.plan.neue_id("halt");
                self.plan.haltlinien.push(Haltlinie { id, arm, punkte: pts, breite: 0.5 });
                Wahl::Halt(self.plan.haltlinien.len() - 1)
            }
            "furt" => {
                self.aendern();
                let id = self.plan.neue_id("furt");
                self.plan.furten.push(Furt { id, a: pts[0], b: pts[pts.len() - 1], breite: 3.0, gruppe: 0 });
                Wahl::Furt(self.plan.furten.len() - 1)
            }
            "sperr" | "radfurt" if pts.len() >= 3 => {
                self.aendern();
                let id = self.plan.neue_id("flaeche");
                self.plan.flaechen.push(Flaeche { id, art: l.art.clone(), punkte: pts, loecher: vec![], textur: Textur { dx: 0.0, dy: 0.0, skala: 1.0, dreh: 0.0 } });
                Wahl::Flaeche(self.plan.flaechen.len() - 1)
            }
            "bord" => return "Bordkanten: die Flaeche (Gehweg/Insel) im Bauplan an die Linie ziehen".into(),
            "mauer" => return "Mauern: Wand im Bauplan waehlen und Bereich/Versatz anpassen".into(),
            _ => return format!("{} baut der Generator noch nicht", aufnahme::art_name(&l.art)),
        };
        self.wahl = Some(neu);
        self.abschliessen();
        format!("{} in den Bauplan uebernommen - Anwenden baut neu", aufnahme::art_name(&l.art))
    }

    fn aufnahme_ui(&mut self, ui: &mut egui::Ui) -> Option<Aktion> {
        let mut erg = None;
        let n_abw = self.abweichungen.iter().filter(|a| a.was != "nicht_baubar").count();
        let titel = if self.aufnahme.linien.is_empty() { egui::RichText::new("Bauaufnahme: noch leer") }
                    else if n_abw == 0 { egui::RichText::new(format!("Bauaufnahme: {} Linien, keine Abweichung", self.aufnahme.linien.len())).color(egui::Color32::from_rgb(120, 220, 120)) }
                    else { egui::RichText::new(format!("Bauaufnahme: {} Linien, {n_abw} Abweichungen", self.aufnahme.linien.len())).color(egui::Color32::from_rgb(255, 120, 190)) };
        egui::CollapsingHeader::new(titel).id_salt("oe_aufnahme").default_open(true).show(ui, |ui| {
            ui.label(egui::RichText::new("Das echte Bauwerk ueber dem Luftbild nachzeichnen (Strassenbilder zum Nachsehen). Der Abgleich zeigt, was im Gebauten fehlt oder daneben liegt.").small().weak());
            ui.horizontal_wrapped(|ui| {
                if ui.selectable_label(self.werkzeug == Werkzeug::Aufnahme, "\u{270F} Aufnahme zeichnen").on_hover_text("Punkte klicken; Doppelklick oder Enter beendet (Umrisse: Klick auf den ersten Punkt). Das Werkzeug bleibt fuer die naechste Linie.").clicked() {
                    self.werkzeug = Werkzeug::Aufnahme;
                    self.entwurf.clear();
                    self.zeige[AUFNAHME] = true;
                }
                egui::ComboBox::from_id_salt("oe_aufnahme_art").selected_text(aufnahme::ARTEN[self.aufnahme_art].1).show_ui(ui, |ui| {
                    for (i, a) in aufnahme::ARTEN.iter().enumerate() {
                        ui.selectable_value(&mut self.aufnahme_art, i, egui::RichText::new(a.1).color(aufnahme::farbe(a.0)));
                    }
                });
                if ui.selectable_label(self.werkzeug == Werkzeug::Notiz, "\u{1F4CC} Notiz").on_hover_text("Klick setzt eine Notiz (z. B. was auf einem Foto zu sehen ist)").clicked() {
                    self.werkzeug = Werkzeug::Notiz;
                    self.entwurf.clear();
                    self.zeige[AUFNAHME] = true;
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new("Strassenbilder an der Kamera-Mitte:").small());
                self.bilder_knoepfe(ui, self.mitte);
            });
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut self.zeige[AUFNAHME], "zeigen");
                ui.checkbox(&mut self.aufnahme_vorne, "zuerst anklicken").on_hover_text("Klicks waehlen zuerst Aufnahme-Linien (sonst zuerst Bauteile des Bauplans)");
                ui.checkbox(&mut self.abgleich_zeigen, "Abweichungen markieren");
            });
            egui::CollapsingHeader::new("vollstaendig aufgenommen").id_salt("oe_aufnahme_fertig").show(ui, |ui| {
                ui.label(egui::RichText::new("Angehakte Arten sind ganz nachgezeichnet: dann meldet der Abgleich auch Gebautes, das es in echt nicht gibt.").small().weak());
                ui.horizontal_wrapped(|ui| {
                    for a in aufnahme::ARTEN.iter().filter(|a| a.3 > 0.0 && a.0 != "bord") {
                        let mut an = self.aufnahme.ist_fertig(a.0);
                        if ui.checkbox(&mut an, a.1).changed() {
                            self.aendern();
                            self.aufnahme.fertig.retain(|x| x != a.0);
                            if an {
                                self.aufnahme.fertig.push(a.0.to_string());
                            }
                            self.abschliessen();
                        }
                    }
                });
            });
            for (was, titel) in [("fehlt", "fehlt / weicht ab"), ("zuviel", "gebaut, aber nicht in der Aufnahme"), ("nicht_baubar", "baut der Generator noch nicht")] {
                let idx: Vec<usize> = (0..self.abweichungen.len()).filter(|&i| self.abweichungen[i].was == was).collect();
                if idx.is_empty() {
                    continue;
                }
                egui::CollapsingHeader::new(format!("{titel} ({})", idx.len())).id_salt(("oe_abw", was)).default_open(was == "fehlt").show(ui, |ui| {
                    for i in idx.into_iter().take(200) {
                        let a = &self.abweichungen[i];
                        if ui.selectable_label(self.abw_wahl == Some(i), egui::RichText::new(&a.text).small().color(aufnahme::farbe(&a.art))).clicked() {
                            self.abw_wahl = Some(i);
                            let z = self.hoehe(a.pos).unwrap_or(f64::NAN);
                            erg = Some(Aktion::KameraNah(a.pos.extend(z)));
                            if let Some(k) = self.aufnahme.linien.iter().position(|l| l.id == a.bezug) {
                                self.wahl = Some(Wahl::Aufnahme(k));
                            }
                        }
                    }
                });
            }
        });
        erg
    }

    // ---------------------------------------------------------- Waende: Linien entlang der Stationen
    fn wand_punkt(&self, st: &Station, w: &Wand) -> Option<DVec2> {
        let t = *st.t.get(&format!("{}_{}", w.art, w.seite))?;
        let vz = if w.seite == "w" { -1.0 } else { 1.0 };
        Some(st.p + st.q * (t + vz * w.versatz))
    }

    fn wand_linie(&self, w: &Wand) -> Vec<DVec2> {
        self.kontext.stationen.iter().filter(|st| st.s >= w.von - 1e-6 && st.s <= w.bis + 1e-6).filter_map(|st| self.wand_punkt(st, w)).collect()
    }

    /// Station der Wandlinie (Art, Seite) am naechsten zu p
    fn station_bei(&self, w: &Wand, p: DVec2) -> Option<f64> {
        self.kontext.stationen.iter().filter_map(|st| self.wand_punkt(st, w).map(|q| ((q - p).length(), st.s)))
            .min_by(|a, b| a.0.total_cmp(&b.0)).map(|x| x.1)
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
            Wahl::Wand(i) => self.plan.waende.get(i).map(|w| {
                let l = self.wand_linie(w);
                match (l.first(), l.last()) {
                    (Some(a), Some(b)) => vec![*a, *b],
                    _ => vec![],
                }
            }).unwrap_or_default(),
            Wahl::Strasse(_) => vec![],
            Wahl::Aufnahme(i) => self.aufnahme.linien.get(i).map(|x| x.punkte.clone()).unwrap_or_default(),
            Wahl::Notiz(i) => self.aufnahme.notizen.get(i).map(|x| vec![x.pos]).unwrap_or_default(),
        }
    }

    /// Linie des Bauteils zum Zeichnen der Auswahl
    fn umriss(&self, w: Wahl) -> (Vec<DVec2>, bool) {
        match w {
            Wahl::Flaeche(i) => (self.plan.flaechen.get(i).map(|x| x.punkte.clone()).unwrap_or_default(), true),
            Wahl::Wand(i) => (self.plan.waende.get(i).map(|x| self.wand_linie(x)).unwrap_or_default(), false),
            Wahl::Strasse(i) => (self.kontext.strassen.get(i).map(|x| x.1.clone()).unwrap_or_default(), false),
            Wahl::Furt(i) => (self.plan.furten.get(i).map(|x| vec![x.a, x.b]).unwrap_or_default(), false),
            Wahl::Symbol(i) => (self.plan.symbole.get(i).map(|x| vec![x.pos, x.pos + richtung_vec(x.richtung) * 5.0 * x.skala]).unwrap_or_default(), false),
            Wahl::Ampel(i) => (self.plan.ampeln.get(i).map(|x| vec![x.pos - richtung_vec(x.richtung) * 0.46, x.pos + richtung_vec(x.richtung) * 1.0]).unwrap_or_default(), false),
            Wahl::Aufnahme(i) => self.aufnahme.linien.get(i).map(|x| (x.punkte.clone(), aufnahme::geschlossen(&x.art))).unwrap_or_default(),
            _ => (self.griffe(w), false),
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
            Wahl::Aufnahme(i) => self.aufnahme.linien.get_mut(i).map(|x| x.punkte.iter_mut().collect()).unwrap_or_default(),
            Wahl::Notiz(i) => self.aufnahme.notizen.get_mut(i).map(|x| vec![&mut x.pos]).unwrap_or_default(),
            Wahl::Wand(_) | Wahl::Strasse(_) => vec![],
        }
    }

    /// Linienzug des Bauteils (fuer Punkte einfuegen/loeschen): (Liste, geschlossen, Mindestzahl)
    fn zug_mut(&mut self, w: Wahl) -> Option<(&mut Vec<DVec2>, bool, usize)> {
        match w {
            Wahl::Flaeche(i) => self.plan.flaechen.get_mut(i).map(|x| (&mut x.punkte, true, 3)),
            Wahl::Linie(i) => self.plan.linien.get_mut(i).map(|x| (&mut x.punkte, false, 2)),
            Wahl::Gelaender(i) => self.plan.gelaender.get_mut(i).map(|x| (&mut x.punkte, false, 2)),
            Wahl::Gehpfad(i) => self.plan.gehpfade.get_mut(i).map(|x| (&mut x.punkte, false, 2)),
            Wahl::Aufnahme(i) => self.aufnahme.linien.get_mut(i).map(|x| {
                let zu = aufnahme::geschlossen(&x.art);
                (&mut x.punkte, zu, if zu { 3 } else { 2 })
            }),
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

    fn verschieben(&mut self, w: Wahl, von: DVec2, nach: DVec2) {
        if let Wahl::Wand(i) = w {
            let Some(wd) = self.plan.waende.get(i).cloned() else { return };
            if let (Some(a), Some(b)) = (self.station_bei(&wd, von), self.station_bei(&wd, nach)) {
                let d = b - a;
                if let Some(x) = self.plan.waende.get_mut(i) {
                    x.von += d;
                    x.bis += d;
                }
            }
            return;
        }
        let d = nach - von;
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
        if g.is_empty() || matches!(w, Wahl::Wand(_) | Wahl::Strasse(_)) {
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
            Wahl::Wand(i) => drop(self.plan.waende.remove(i)),
            Wahl::Strasse(_) => self.meldung = "Strassen lassen sich nicht loeschen (nur Querschnitt und Lage aendern)".into(),
            Wahl::Aufnahme(i) => drop(self.aufnahme.linien.remove(i)),
            Wahl::Notiz(i) => drop(self.aufnahme.notizen.remove(i)),
        }
    }

    fn duplizieren(&mut self, w: Wahl) -> Wahl {
        macro_rules! dup {
            ($liste:ident, $art:expr, $v:ident) => {{
                let mut x = self.plan.$liste[$v].clone();
                x.id = self.plan.neue_id($art);
                self.plan.$liste.push(x);
                self.plan.$liste.len() - 1
            }};
        }
        let neu = match w {
            Wahl::Flaeche(i) => Wahl::Flaeche(dup!(flaechen, "flaeche", i)),
            Wahl::Linie(i) => Wahl::Linie(dup!(linien, "linie", i)),
            Wahl::Halt(i) => Wahl::Halt(dup!(haltlinien, "halt", i)),
            Wahl::Symbol(i) => Wahl::Symbol(dup!(symbole, "symbol", i)),
            Wahl::Ampel(i) => Wahl::Ampel(dup!(ampeln, "ampel", i)),
            Wahl::Gelaender(i) => Wahl::Gelaender(dup!(gelaender, "gelaender", i)),
            Wahl::Furt(i) => Wahl::Furt(dup!(furten, "furt", i)),
            Wahl::Gehpfad(i) => Wahl::Gehpfad(dup!(gehpfade, "gehpfad", i)),
            Wahl::Wand(i) => {
                let k = dup!(waende, "wand", i);
                return Wahl::Wand(k);
            }
            Wahl::Strasse(_) => return w,
            Wahl::Aufnahme(i) => {
                let mut x = self.aufnahme.linien[i].clone();
                x.id = self.aufnahme.neue_id(&x.art);
                self.aufnahme.linien.push(x);
                Wahl::Aufnahme(self.aufnahme.linien.len() - 1)
            }
            Wahl::Notiz(i) => {
                let mut x = self.aufnahme.notizen[i].clone();
                x.id = self.aufnahme.neue_id("notiz");
                self.aufnahme.notizen.push(x);
                Wahl::Notiz(self.aufnahme.notizen.len() - 1)
            }
        };
        let d = DVec2::new(1.0, -1.0);
        for p in self.punkte_mut(neu) {
            *p += d;
        }
        neu
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
            Wahl::Wand(i) => self.plan.waende.get(i).map(|w| abstand_zug(p, &self.wand_linie(w), false).0 < tol.max(0.6)).unwrap_or(false),
            Wahl::Strasse(i) => self.kontext.strassen.get(i).map(|s| abstand_zug(p, &s.1, false).0 < tol.max(0.8)).unwrap_or(false),
            Wahl::Aufnahme(i) => self.aufnahme.linien.get(i).map(|l| abstand_zug(p, &l.punkte, aufnahme::geschlossen(&l.art)).0 < tol.max(0.3)).unwrap_or(false),
            Wahl::Notiz(i) => self.aufnahme.notizen.get(i).map(|n| (n.pos - p).length() < tol.max(0.8)).unwrap_or(false),
        }
    }

    /// oberstes Bauteil unter p (Toleranz in m)
    fn treffer(&self, p: DVec2, tol: f64) -> Option<Wahl> {
        let z = &self.zeige;
        let reihen: [(usize, Vec<Wahl>); 11] = [
            (AUFNAHME, (0..self.aufnahme.notizen.len()).map(Wahl::Notiz).collect()),
            (AUFNAHME, (0..self.aufnahme.linien.len()).map(Wahl::Aufnahme).collect()),
            (4, (0..self.plan.ampeln.len()).map(Wahl::Ampel).collect()),
            (3, (0..self.plan.symbole.len()).map(Wahl::Symbol).collect()),
            (2, (0..self.plan.haltlinien.len()).map(Wahl::Halt).collect()),
            (6, (0..self.plan.furten.len()).map(Wahl::Furt).collect()),
            (1, (0..self.plan.linien.len()).map(Wahl::Linie).collect()),
            (5, (0..self.plan.gelaender.len()).map(Wahl::Gelaender).collect()),
            (8, (0..self.plan.waende.len()).map(Wahl::Wand).collect()),
            (7, (0..self.plan.gehpfade.len()).map(Wahl::Gehpfad).collect()),
            (9, (0..self.kontext.strassen.len()).map(Wahl::Strasse).collect()),
        ];
        let reihen: Vec<&(usize, Vec<Wahl>)> = if self.aufnahme_vorne { reihen.iter().collect() } else { reihen[2..].iter().chain(reihen[..2].iter()).collect() };
        for (ebene, liste) in reihen {
            if !z[*ebene] {
                continue;
            }
            if let Some(w) = liste.iter().rev().find(|w| self.liegt_unter(**w, p, tol)) {
                return Some(*w);
            }
        }
        if z[0] {
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

    /// naechster Eckpunkt eines anderen Bauteils (Einrasten)
    fn einrasten_an(&self, p: DVec2, tol: f64, ausser: Option<Wahl>) -> Option<DVec2> {
        let mut best: Option<(f64, DVec2)> = None;
        let mut pruefe = |q: DVec2| {
            let d = (q - p).length();
            if d < tol && best.map(|b| d < b.0).unwrap_or(true) {
                best = Some((d, q));
            }
        };
        for (i, f) in self.plan.flaechen.iter().enumerate() {
            if ausser != Some(Wahl::Flaeche(i)) {
                f.punkte.iter().for_each(|q| pruefe(*q));
            }
        }
        for (i, l) in self.plan.linien.iter().enumerate() {
            if ausser != Some(Wahl::Linie(i)) {
                l.punkte.iter().for_each(|q| pruefe(*q));
            }
        }
        for (i, g) in self.plan.gelaender.iter().enumerate() {
            if ausser != Some(Wahl::Gelaender(i)) {
                g.punkte.iter().for_each(|q| pruefe(*q));
            }
        }
        for (i, l) in self.aufnahme.linien.iter().enumerate() {
            if ausser != Some(Wahl::Aufnahme(i)) {
                l.punkte.iter().for_each(|q| pruefe(*q));
            }
        }
        for a in &self.kontext.arme {
            pruefe(a.links);
            pruefe(a.rechts);
        }
        self.entwurf.iter().for_each(|q| pruefe(*q));
        best.map(|b| b.1)
    }

    fn neues(&mut self, art: &str, c: DVec2) {
        self.aendern();
        let q = |dx: f64, dy: f64| c + DVec2::new(dx, dy);
        let w = match art {
            "halt" => {
                let id = self.plan.neue_id("halt");
                let arm = self.kontext.arme.iter().enumerate().min_by(|a, b| (a.1.pos - c).length().total_cmp(&(b.1.pos - c).length())).map(|x| x.0).unwrap_or(0);
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
            "furt" => {
                let id = self.plan.neue_id("furt");
                self.plan.furten.push(Furt { id, a: q(-4.0, 0.0), b: q(4.0, 0.0), breite: 3.0, gruppe: 0 });
                Wahl::Furt(self.plan.furten.len() - 1)
            }
            _ => {
                // Wand: 20 m an der naechsten Station, Seite nach Lage des Punkts
                let st = self.kontext.stationen.iter().min_by(|a, b| (a.p - c).length().total_cmp(&(b.p - c).length())).cloned().unwrap_or_default();
                let seite = if (c - st.p).dot(st.q) < 0.0 { "w" } else { "o" };
                let id = self.plan.neue_id("wand");
                self.plan.waende.push(Wand { id, seite: seite.into(), art: "aussen".into(), von: st.s - 10.0, bis: st.s + 10.0,
                                             material: if seite == "w" { "gabionen".into() } else { "klinker".into() }, laerm: "keine".into(),
                                             versatz: 0.0, hoehe: 0.0, stufig: true });
                Wahl::Wand(self.plan.waende.len() - 1)
            }
        };
        self.wahl = Some(w);
        self.abschliessen();
    }

    /// gezeichnetes Bauteil anlegen
    fn entwurf_fertig(&mut self) {
        let pts = std::mem::take(&mut self.entwurf);
        let art_a = aufnahme::ARTEN[self.aufnahme_art.min(aufnahme::ARTEN.len() - 1)].0;
        let min = if self.werkzeug == Werkzeug::Flaeche || (self.werkzeug == Werkzeug::Aufnahme && aufnahme::geschlossen(art_a)) { 3 } else { 2 };
        if pts.len() < min {
            self.meldung = format!("zu wenige Punkte ({} noetig)", min);
            return;
        }
        self.aendern();
        let w = match self.werkzeug {
            Werkzeug::Flaeche => {
                let id = self.plan.neue_id("flaeche");
                self.plan.flaechen.push(Flaeche { id, art: FLAECHEN_ARTEN[self.neue_art].0.into(), punkte: pts, loecher: vec![],
                                                  textur: Textur { dx: 0.0, dy: 0.0, skala: 1.0, dreh: 0.0 } });
                Wahl::Flaeche(self.plan.flaechen.len() - 1)
            }
            Werkzeug::Linie => {
                let id = self.plan.neue_id("linie");
                self.plan.linien.push(Linie { id, punkte: pts, breite: 0.12, an: 0.0, aus: 0.0 });
                Wahl::Linie(self.plan.linien.len() - 1)
            }
            Werkzeug::Gelaender => {
                let id = self.plan.neue_id("gelaender");
                self.plan.gelaender.push(Gelaender { id, punkte: pts, unten: "gelaende".into(), flip: false });
                Wahl::Gelaender(self.plan.gelaender.len() - 1)
            }
            Werkzeug::Aufnahme => {
                // Werkzeug bleibt: gleich die naechste Linie nachzeichnen
                let id = self.aufnahme.neue_id(art_a);
                self.aufnahme.linien.push(ALinie { id, art: art_a.into(), punkte: pts, notiz: String::new() });
                self.wahl = Some(Wahl::Aufnahme(self.aufnahme.linien.len() - 1));
                self.abschliessen();
                self.meldung = format!("{} aufgenommen - naechste Linie klicken oder Esc", aufnahme::art_name(art_a));
                return;
            }
            _ => {
                let id = self.plan.neue_id("gehpfad");
                self.plan.gehpfade.push(Gehpfad { id, punkte: pts });
                Wahl::Gehpfad(self.plan.gehpfade.len() - 1)
            }
        };
        self.abschliessen();
        self.wahl = Some(w);
        self.werkzeug = Werkzeug::Auswahl;
        self.meldung = "Bauteil angelegt".into();
    }

    // ---------------------------------------------------------- Bedienung (2D und 3D, Weltpunkte)
    /// Maus ueber q (Hervorhebung)
    pub fn hover(&mut self, q: Option<DVec2>, tol: f64) {
        self.unter = if self.werkzeug.zeichnet() { None } else { q.and_then(|q| self.treffer(q, tol)) };
    }

    pub fn zieht(&self) -> bool {
        self.zug != Zug::Nichts
    }

    /// haelt die Maus ein Bauteil, einen Griff oder eine Textur (sonst darf die Kamera schwenken)?
    pub fn greift(&self) -> bool {
        matches!(self.zug, Zug::Griff(_) | Zug::Verschieben | Zug::Textur)
    }

    /// zeichnet gerade ein neues Bauteil (Klicks gehoeren dem Editor)
    pub fn zeichnet(&self) -> bool {
        self.werkzeug.zeichnet()
    }

    /// linke Maustaste gedrueckt bei q
    pub fn druecken(&mut self, q: DVec2, tol: f64, alt: bool) {
        self.letzte = Some(q);
        if self.werkzeug.zeichnet() {
            self.zug = Zug::Nichts;
            return;
        }
        let textur = alt || self.werkzeug == Werkzeug::Textur;
        let griff = self.wahl.and_then(|w| self.griffe(w).iter().position(|g| (*g - q).length() < tol * 1.4));
        self.zug = match (griff, self.wahl) {
            (Some(k), Some(_)) if !textur => Zug::Griff(k),
            (_, Some(Wahl::Flaeche(i))) if textur && self.liegt_unter(Wahl::Flaeche(i), q, tol) => Zug::Textur,
            _ => {
                let t = if self.wahl.map(|w| self.liegt_unter(w, q, tol)).unwrap_or(false) { self.wahl } else { self.treffer(q, tol) };
                self.wahl = t;
                match t {
                    Some(Wahl::Flaeche(_)) if textur => Zug::Textur,
                    Some(Wahl::Strasse(_)) => Zug::Nichts,
                    Some(_) => Zug::Verschieben,
                    None => Zug::Schwenken,
                }
            }
        };
    }

    /// Maus mit gedrueckter linker Taste bei q
    pub fn ziehen(&mut self, q: DVec2, tol: f64, strg: bool) {
        let Some(l) = self.letzte else { return };
        let d = q - l;
        match (self.zug, self.wahl) {
            (Zug::Griff(k), Some(Wahl::Wand(i))) => {
                self.aendern();
                let Some(wd) = self.plan.waende.get(i).cloned() else { return };
                if let Some(s) = self.station_bei(&wd, q) {
                    if let Some(x) = self.plan.waende.get_mut(i) {
                        if k == 0 { x.von = s.min(x.bis - 2.0) } else { x.bis = s.max(x.von + 2.0) }
                    }
                }
            }
            (Zug::Griff(k), Some(w)) => {
                self.aendern();
                let mut ziel = q;
                if self.einrasten && !strg {
                    if let Some(s) = self.einrasten_an(q, tol * 1.5, Some(w)) {
                        ziel = s;
                    }
                }
                if let Some(pk) = self.punkte_mut(w).into_iter().nth(k) {
                    *pk = ziel;
                }
            }
            (Zug::Verschieben, Some(w)) => {
                self.aendern();
                self.verschieben(w, l, q);
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

    pub fn loslassen(&mut self) {
        self.zug = Zug::Nichts;
        self.letzte = None;
        self.abschliessen();
    }

    /// Klick (ohne Ziehen) bei q
    pub fn klicken(&mut self, q: DVec2, tol: f64, strg: bool) {
        if self.werkzeug == Werkzeug::Notiz {
            self.aendern();
            let id = self.aufnahme.neue_id("notiz");
            self.aufnahme.notizen.push(Notiz { id, pos: q, text: String::new() });
            self.wahl = Some(Wahl::Notiz(self.aufnahme.notizen.len() - 1));
            self.abschliessen();
            self.werkzeug = Werkzeug::Auswahl;
            self.meldung = "Notiz gesetzt - Text rechts eintragen".into();
            return;
        }
        if self.werkzeug.zeichnet() {
            let mut p = q;
            if self.einrasten && !strg {
                if let Some(s) = self.einrasten_an(q, tol * 1.5, None) {
                    p = s;
                }
            }
            // Klick auf den ersten Punkt schliesst die Flaeche
            let zu = self.werkzeug == Werkzeug::Flaeche || (self.werkzeug == Werkzeug::Aufnahme && aufnahme::geschlossen(aufnahme::ARTEN[self.aufnahme_art.min(aufnahme::ARTEN.len() - 1)].0));
            if zu && self.entwurf.len() >= 3 && (self.entwurf[0] - p).length() < tol * 1.5 {
                self.entwurf_fertig();
                return;
            }
            self.entwurf.push(p);
            return;
        }
        if self.werkzeug == Werkzeug::Textur {
            return;
        }
        self.wahl = self.treffer(q, tol);
    }

    /// Doppelklick: Zeichnen beenden, sonst Punkt auf der Kante einfuegen
    pub fn doppelklick(&mut self, q: DVec2, tol: f64) {
        if self.werkzeug == Werkzeug::Notiz {
            return;
        }
        if self.werkzeug.zeichnet() {
            self.entwurf.pop();
            self.entwurf_fertig();
            return;
        }
        if let Some(w) = self.wahl {
            if let Some((zug, zu, _)) = self.zug_mut(w) {
                let (d, k) = abstand_zug(q, zug, zu);
                if d < tol * 2.0 {
                    self.aendern();
                    if let Some((zug, _, _)) = self.zug_mut(w) {
                        zug.insert(k + 1, q);
                    }
                    self.abschliessen();
                }
            }
        }
    }

    /// Rechtsklick ohne Ziehen: Eckpunkt loeschen bzw. letzten gezeichneten Punkt zuruecknehmen
    pub fn rechtsklick(&mut self, q: DVec2, tol: f64) -> bool {
        if self.werkzeug.zeichnet() && !self.entwurf.is_empty() {
            self.entwurf.pop();
            return true;
        }
        if let Some(w) = self.wahl {
            if let Some(k) = self.griffe(w).iter().position(|g| (*g - q).length() < tol * 1.4) {
                let geht = self.zug_mut(w).map(|(z, _, min)| z.len() > min).unwrap_or(false);
                if geht {
                    self.aendern();
                    if let Some((zug, _, _)) = self.zug_mut(w) {
                        zug.remove(k);
                    }
                    self.abschliessen();
                    return true;
                }
            }
        }
        false
    }

    /// Mausrad mit Alt/Strg: Textur skalieren/drehen, Pfeil/Ampel drehen -> true, wenn verbraucht
    pub fn rad(&mut self, rad: f32, alt: bool, strg: bool) -> bool {
        if rad == 0.0 || !(alt || strg) {
            return false;
        }
        match (alt, strg, self.wahl) {
            (true, _, Some(Wahl::Flaeche(i))) => {
                self.aendern();
                if let Some(fl) = self.plan.flaechen.get_mut(i) {
                    fl.textur.skala = (fl.textur.skala * 1.05f64.powf(rad as f64)).clamp(0.05, 20.0);
                }
            }
            (false, true, Some(Wahl::Flaeche(i))) => {
                self.aendern();
                if let Some(fl) = self.plan.flaechen.get_mut(i) {
                    fl.textur.dreh += 5.0 * rad.signum() as f64;
                }
            }
            (false, true, Some(w)) => {
                self.aendern();
                self.drehen(w, 5.0 * rad.signum() as f64);
            }
            _ => return false,
        }
        self.abschliessen();
        true
    }

    /// Taste -> true, wenn der Editor sie verbraucht hat
    pub fn taste(&mut self, k: egui::Key, umschalt: bool, strg: bool) -> bool {
        match k {
            egui::Key::Escape if self.werkzeug.zeichnet() && !self.entwurf.is_empty() => self.entwurf.clear(),
            egui::Key::Escape if self.werkzeug != Werkzeug::Auswahl => self.werkzeug = Werkzeug::Auswahl,
            egui::Key::Escape if self.wahl.is_some() => self.wahl = None,
            egui::Key::Enter if self.werkzeug.zeichnet() && self.werkzeug != Werkzeug::Notiz => self.entwurf_fertig(),
            egui::Key::Backspace if self.werkzeug.zeichnet() => drop(self.entwurf.pop()),
            egui::Key::Delete if self.wahl.is_some() => {
                let w = self.wahl.unwrap();
                self.aendern();
                self.loeschen(w);
                self.wahl = None;
                self.abschliessen();
            }
            egui::Key::D if strg && self.wahl.is_some() => {
                let w = self.wahl.unwrap();
                self.aendern();
                self.wahl = Some(self.duplizieren(w));
                self.abschliessen();
            }
            egui::Key::Z if strg && !umschalt && !self.undo.is_empty() => self.rueckgaengig(),
            egui::Key::Y if strg && !self.redo.is_empty() => self.wiederholen(),
            egui::Key::Z if strg && umschalt && !self.redo.is_empty() => self.wiederholen(),
            _ => return false,
        }
        true
    }

    // ---------------------------------------------------------- Zeichnen (2D und 3D)
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

    /// alle Bauteile zeichnen. `bild`: Weltpunkt (mit Hoehe) -> Bildschirm, `boden`: Hoehe ohne Raster,
    /// `m`: Pixel je Meter (fuer Linienbreiten), `fuellen`: Flaechen fuellen, `texturiert`: mit den echten Texturen
    pub fn zeichnen(&mut self, ctx: &egui::Context, p: &egui::Painter, bild: &dyn Fn(DVec3) -> Option<egui::Pos2>, boden: &dyn Fn(DVec2) -> f64,
                    m: f32, fuellen: bool, texturiert: bool, deck: u8) {
        let raster = std::mem::take(&mut self.kontext.hoehen);
        let hoehe = |q: DVec2| raster.h(q).map(|z| z + 0.1).unwrap_or_else(|| boden(q)) + 0.06;
        let b = |q: DVec2| bild(q.extend(hoehe(q)));
        let zug = |pts: &[DVec2]| -> Vec<egui::Pos2> { pts.iter().filter_map(|q| b(*q)).collect() };
        // Trogoeffnungen (fest)
        for o in &self.kontext.offen {
            p.add(egui::Shape::closed_line(zug(o), egui::Stroke::new(1.5, egui::Color32::from_rgb(120, 120, 140))));
        }
        if self.zeige[0] {
            let mut ord: Vec<usize> = (0..self.plan.flaechen.len()).collect();
            ord.sort_by_key(|&i| reihenfolge(&self.plan.flaechen[i].art));
            for i in ord {
                let fl = self.plan.flaechen[i].clone();
                if fuellen {
                    let ring = mit_loechern(&fl.punkte, &fl.loecher);
                    let tris = dreiecke(&ring);
                    let (datei, m_tex) = material(&fl.art);
                    let tex = if texturiert { self.textur(ctx, datei) } else { None };
                    let farbe = farbe_art(&fl.art);
                    let mut mesh = match tex {
                        Some(id) => egui::epaint::Mesh::with_texture(id),
                        None => egui::epaint::Mesh::default(),
                    };
                    let sk = m_tex * fl.textur.skala.max(0.05);
                    let (sn, cs) = fl.textur.dreh.to_radians().sin_cos();
                    let a = if fl.art == "sperr" && tex.is_none() { deck / 2 } else { deck };
                    let mut ok = true;
                    for q in &ring {
                        let Some(pos) = b(*q) else { ok = false; break };
                        let uv = egui::pos2(((cs * q.x + sn * q.y) / sk + fl.textur.dx) as f32, -((-sn * q.x + cs * q.y) / sk + fl.textur.dy) as f32);
                        let col = if tex.is_some() { egui::Color32::from_white_alpha(a) } else { egui::Color32::from_rgba_unmultiplied(farbe.r(), farbe.g(), farbe.b(), a) };
                        mesh.vertices.push(egui::epaint::Vertex { pos, uv: if tex.is_some() { uv } else { egui::epaint::WHITE_UV }, color: col });
                    }
                    if ok {
                        for t in tris {
                            mesh.indices.extend([t[0] as u32, t[1] as u32, t[2] as u32]);
                        }
                        p.add(egui::Shape::mesh(mesh));
                    }
                }
                let rand = match fl.art.as_str() {
                    "gehweg" | "insel" | "gruen" => egui::Stroke::new(1.5, egui::Color32::from_rgb(210, 210, 200)),
                    "sperr" => egui::Stroke::new(1.2, egui::Color32::from_rgb(235, 235, 235)),
                    "radfurt" => egui::Stroke::new(1.2, egui::Color32::from_rgb(220, 90, 80)),
                    _ => egui::Stroke::new(0.8, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 70)),
                };
                p.add(egui::Shape::closed_line(zug(&fl.punkte), rand));
            }
        }
        let weiss = egui::Color32::from_rgb(245, 245, 240);
        let breite_px = |b_: f64| ((b_ as f32) * m).max(1.2);
        if self.zeige[9] {
            for (name, pts) in &self.kontext.strassen {
                let l = zug(pts);
                p.add(egui::Shape::line(l.clone(), egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(110, 170, 255, 150))));
                if let Some(q) = l.get(l.len() / 2) {
                    p.text(*q, egui::Align2::CENTER_CENTER, name.replace('_', " "), egui::FontId::proportional(10.0), egui::Color32::from_rgb(150, 200, 255));
                }
            }
        }
        if self.zeige[8] {
            for w in &self.plan.waende {
                let l = zug(&self.wand_linie(w));
                let farbe = match w.material.as_str() {
                    "gabionen" => egui::Color32::from_rgb(170, 160, 140),
                    "beton" => egui::Color32::from_rgb(200, 200, 200),
                    _ => egui::Color32::from_rgb(190, 80, 60),
                };
                p.add(egui::Shape::line(l.clone(), egui::Stroke::new(if w.art == "trenn" { 3.0 } else { 4.0 }, farbe)));
                if w.laerm != "keine" {
                    let c = if w.laerm == "glas" { egui::Color32::from_rgb(140, 220, 255) } else { egui::Color32::from_rgb(60, 170, 110) };
                    p.extend(egui::Shape::dashed_line(&l, egui::Stroke::new(2.0, c), 6.0, 4.0));
                }
            }
        }
        if self.zeige[1] {
            for l in &self.plan.linien {
                let pts = zug(&l.punkte);
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
                p.add(egui::Shape::line(zug(&h.punkte), egui::Stroke::new(breite_px(h.breite), weiss)));
            }
        }
        if self.zeige[6] {
            for f_ in &self.plan.furten {
                let d = (f_.b - f_.a).normalize_or_zero();
                let nq = DVec2::new(-d.y, d.x) * f_.breite / 2.0;
                for sgn in [1.0, -1.0] {
                    p.extend(egui::Shape::dashed_line(&zug(&[f_.a + nq * sgn, f_.b + nq * sgn]), egui::Stroke::new(breite_px(0.12), weiss), 0.5 * m, 0.2 * m));
                }
                p.add(egui::Shape::line(zug(&[f_.a, f_.b]), egui::Stroke::new(1.0, egui::Color32::from_rgb(120, 220, 255))));
            }
        }
        if self.zeige[3] {
            for sy in &self.plan.symbole {
                let v = richtung_vec(sy.richtung) * sy.skala;
                let r = rechts_vec(sy.richtung) * sy.skala;
                let w = |a: f64, bb: f64| b(sy.pos + v * bb + r * a);
                let st = egui::Stroke::new(breite_px(0.18).max(1.5), weiss);
                let seg = |x: Option<egui::Pos2>, y: Option<egui::Pos2>| if let (Some(x), Some(y)) = (x, y) { p.line_segment([x, y], st); };
                if sy.art == "bus" {
                    if let Some(c) = w(0.0, 1.2) {
                        p.text(c, egui::Align2::CENTER_CENTER, "BUS", egui::FontId::proportional((1.6 * m).clamp(9.0, 40.0)), weiss);
                    }
                    continue;
                }
                seg(w(0.0, 0.0), w(0.0, 2.6));
                if sy.pfeile.iter().any(|x| x == "gerade") || sy.pfeile.is_empty() {
                    seg(w(0.0, 2.5), w(0.0, 3.9));
                    if let (Some(a), Some(bb), Some(c)) = (w(-0.35, 3.8), w(0.0, 5.0), w(0.35, 3.8)) {
                        p.add(egui::Shape::convex_polygon(vec![a, bb, c], weiss, egui::Stroke::NONE));
                    }
                }
                for (k, sgn) in [("links", -1.0), ("rechts", 1.0)] {
                    if sy.pfeile.iter().any(|x| x == k) {
                        seg(w(0.0, 2.4), w(sgn * 0.7, 3.3));
                        if let (Some(a), Some(bb), Some(c)) = (w(sgn * 0.45, 3.35), w(sgn * 1.15, 3.85), w(sgn * 0.95, 2.95)) {
                            p.add(egui::Shape::convex_polygon(vec![a, bb, c], weiss, egui::Stroke::NONE));
                        }
                    }
                }
            }
        }
        if self.zeige[5] {
            for g in &self.plan.gelaender {
                p.add(egui::Shape::line(zug(&g.punkte), egui::Stroke::new(3.0, egui::Color32::from_rgb(220, 110, 160))));
            }
        }
        if self.zeige[7] {
            for g in &self.plan.gehpfade {
                p.extend(egui::Shape::dashed_line(&zug(&g.punkte), egui::Stroke::new(1.2, egui::Color32::from_rgb(120, 220, 255)), 4.0, 3.0));
            }
        }
        if self.zeige[4] {
            for a in &self.plan.ampeln {
                let v = richtung_vec(a.richtung);
                let mast = a.pos - v * 0.46;
                let farbe = if a.art == "fg" { egui::Color32::from_rgb(90, 220, 120) } else { egui::Color32::from_rgb(255, 80, 70) };
                if let (Some(x), Some(y)) = (b(mast), b(a.pos + v * 1.6)) {
                    p.line_segment([x, y], egui::Stroke::new(2.0, farbe));
                }
                if a.mast == "ausleger" {
                    if let (Some(x), Some(y)) = (b(mast), b(mast - rechts_vec(a.richtung) * 4.0)) {
                        p.line_segment([x, y], egui::Stroke::new(2.0, egui::Color32::from_rgb(160, 170, 170)));
                    }
                }
                if let Some(c) = b(a.pos) {
                    p.circle(c, 5.0, farbe, egui::Stroke::new(1.0, egui::Color32::BLACK));
                    p.text(c + egui::vec2(6.0, -6.0), egui::Align2::LEFT_BOTTOM, format!("{}", a.gruppe + 1), egui::FontId::proportional(11.0), egui::Color32::WHITE);
                }
            }
        }
        for a in &self.kontext.arme {
            if let (Some(x), Some(y)) = (b(a.links), b(a.rechts)) {
                p.line_segment([x, y], egui::Stroke::new(2.5, egui::Color32::from_rgb(255, 160, 40)));
            }
        }
        if self.zeige[AUFNAHME] {
            // Bauaufnahme: kraeftige Farbe je Art, Punkte als Rauten, Notizen als Stecknadeln
            for l in &self.aufnahme.linien {
                let c = aufnahme::farbe(&l.art);
                let mut pts = zug(&l.punkte);
                if aufnahme::geschlossen(&l.art) && pts.len() > 2 {
                    pts.push(pts[0]);
                }
                p.add(egui::Shape::line(pts.clone(), egui::Stroke::new(4.0, egui::Color32::from_black_alpha(140))));
                p.extend(egui::Shape::dashed_line(&pts, egui::Stroke::new(2.2, c), 9.0, 3.0));
                for q in &pts {
                    p.add(egui::Shape::convex_polygon(vec![*q + egui::vec2(0.0, -3.5), *q + egui::vec2(3.5, 0.0), *q + egui::vec2(0.0, 3.5), *q + egui::vec2(-3.5, 0.0)], c, egui::Stroke::NONE));
                }
            }
            for n in &self.aufnahme.notizen {
                if let Some(c) = b(n.pos) {
                    let gelb = egui::Color32::from_rgb(255, 225, 60);
                    p.line_segment([c, c + egui::vec2(0.0, -16.0)], egui::Stroke::new(2.0, gelb));
                    p.circle(c + egui::vec2(0.0, -18.0), 5.0, gelb, egui::Stroke::new(1.0, egui::Color32::BLACK));
                    if !n.text.is_empty() {
                        let t: String = n.text.lines().next().unwrap_or("").chars().take(40).collect();
                        p.text(c + egui::vec2(8.0, -18.0), egui::Align2::LEFT_CENTER, t, egui::FontId::proportional(12.0), egui::Color32::WHITE);
                    }
                }
            }
        }
        if self.abgleich_zeigen {
            for (i, a) in self.abweichungen.iter().enumerate() {
                if a.was == "nicht_baubar" {
                    continue;
                }
                if let Some(c) = b(a.pos) {
                    let gewaehlt = self.abw_wahl == Some(i);
                    let farbe = if a.was == "zuviel" { egui::Color32::from_rgb(120, 170, 255) } else { egui::Color32::from_rgb(255, 60, 160) };
                    let r = if gewaehlt { 13.0 } else { 7.0 };
                    p.rect_stroke(egui::Rect::from_center_size(c, egui::vec2(r * 2.0, r * 2.0)), 2.0, egui::Stroke::new(if gewaehlt { 3.0 } else { 2.0 }, farbe), egui::StrokeKind::Middle);
                    if gewaehlt {
                        p.text(c + egui::vec2(16.0, 16.0), egui::Align2::LEFT_TOP, &a.text, egui::FontId::proportional(13.0), egui::Color32::WHITE);
                    }
                }
            }
        }
        // Hervorhebung unter der Maus, Auswahl mit Griffen
        if let Some(w) = self.unter.filter(|u| Some(*u) != self.wahl) {
            let (l, zu) = self.umriss(w);
            let st = egui::Stroke::new(2.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 200));
            if zu { p.add(egui::Shape::closed_line(zug(&l), st)); } else { p.add(egui::Shape::line(zug(&l), st)); }
        }
        if let Some(w) = self.wahl {
            let gelb = egui::Color32::from_rgb(255, 230, 40);
            let (l, zu) = self.umriss(w);
            let st = egui::Stroke::new(2.5, gelb);
            if zu { p.add(egui::Shape::closed_line(zug(&l), st)); } else { p.add(egui::Shape::line(zug(&l), st)); }
            for q in self.griffe(w) {
                if let Some(c) = b(q) {
                    p.rect_filled(egui::Rect::from_center_size(c, egui::vec2(9.0, 9.0)), 1.5, gelb);
                    p.rect_stroke(egui::Rect::from_center_size(c, egui::vec2(9.0, 9.0)), 1.5, egui::Stroke::new(1.0, egui::Color32::BLACK), egui::StrokeKind::Outside);
                }
            }
        }
        // laufender Entwurf
        if !self.entwurf.is_empty() {
            let c = egui::Color32::from_rgb(80, 255, 160);
            let l = zug(&self.entwurf);
            p.add(egui::Shape::line(l.clone(), egui::Stroke::new(2.0, c)));
            for (i, q) in l.iter().enumerate() {
                p.circle_filled(*q, if i == 0 { 6.0 } else { 4.0 }, c);
            }
        }
        if self.funde_zeigen {
            for (i, f) in self.kontext.funde.iter().enumerate() {
                if let Some(c) = bild(f.pos) {
                    let gewaehlt = self.fund_wahl == Some(i);
                    let farbe = if f.art.starts_with("ampel") { egui::Color32::from_rgb(255, 80, 200) } else { egui::Color32::from_rgb(255, 90, 40) };
                    p.circle_stroke(c, if gewaehlt { 14.0 } else { 7.0 }, egui::Stroke::new(if gewaehlt { 3.0 } else { 2.0 }, farbe));
                    if gewaehlt {
                        p.text(c + egui::vec2(16.0, -16.0), egui::Align2::LEFT_BOTTOM, &f.text, egui::FontId::proportional(13.0), egui::Color32::WHITE);
                    }
                }
            }
        }
        self.kontext.hoehen = raster;
    }

    // ---------------------------------------------------------- Oberflaeche
    /// Leiste rechts (Panel) und auf Wunsch das 2D-Fenster
    pub fn ui(&mut self, haupt: &mut egui::Ui, offen: &mut bool) -> Option<Aktion> {
        let ctx = &haupt.ctx().clone();
        let mut erg = None;
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
                        if let Ok(b) = std::fs::read(&self.plan_pfad) {
                            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&b) {
                                // der Generator ergaenzt beim ersten Mal Strassen und Waende: uebernehmen, ohne Eigenes zu verlieren
                                let neu = Bauplan::aus_json(&v);
                                if self.plan.flaechen.is_empty() {
                                    self.plan = neu;
                                } else {
                                    if self.plan.waende.is_empty() {
                                        self.plan.waende = neu.waende;
                                    }
                                    if !self.plan.rest.contains_key("strassen") {
                                        if let Some(st) = neu.rest.get("strassen") {
                                            self.plan.rest.insert("strassen".into(), st.clone());
                                        }
                                    }
                                }
                            }
                        }
                        self.texturen.clear();
                        self.rev += 1;
                        if self.offen_aenderung.is_none() && Instant::now().duration_since(self.zuletzt_geaendert) > Duration::from_millis(50) {
                            self.geaendert = self.plan.json().to_string() != std::fs::read_to_string(&self.plan_pfad).map(|t| serde_json::from_str::<serde_json::Value>(&t).map(|v| Bauplan::aus_json(&v).json().to_string()).unwrap_or_default()).unwrap_or_default();
                        }
                        self.meldung = format!("Neu gebaut: {t}");
                        erg = Some(Aktion::Gebaut { json: self.json.clone(), einsetzen: self.einsetzen });
                    }
                    Err(e) => self.meldung = format!("Bauen fehlgeschlagen: {e}"),
                }
            }
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        // automatisch anwenden: 1,5 s nach der letzten Aenderung (nicht waehrend gezogen wird)
        if self.automatisch && self.geaendert && self.job.is_none() && self.offen_aenderung.is_none()
            && Instant::now().duration_since(self.zuletzt_geaendert) > Duration::from_millis(1500) && !ctx.input(|i| i.pointer.any_down()) {
            self.bauen(true);
        } else if self.automatisch && self.geaendert {
            ctx.request_repaint_after(Duration::from_millis(300));
        }
        let mut schliessen = false;
        egui::Panel::right("objekteditor").default_size(340.0).resizable(true).show(haupt, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Objekt-Editor");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("x").on_hover_text("Objekt-Editor schliessen").clicked() {
                        schliessen = true;
                    }
                });
            });
            ui.label(egui::RichText::new("Sonderbau Ring / Kreuzung Ludwigsallee").weak());
            egui::ScrollArea::vertical().id_salt("oe_leiste").show(ui, |ui| {
                if let Some(a) = self.seitenleiste(ui) {
                    erg = Some(a);
                }
            });
        });
        if schliessen {
            *offen = false;
        }
        if self.plan2d {
            let mut auf = true;
            egui::Window::new("2D-Plan (Luftbild)").open(&mut auf).default_size([760.0, 620.0]).resizable(true).show(ctx, |ui| {
                self.zeichenflaeche(ui);
            });
            self.plan2d = auf;
        }
        if !ctx.input(|i| i.pointer.any_down()) && self.zug == Zug::Nichts {
            self.abschliessen();
        }
        self.abgleich_aktuell();
        erg
    }

    fn seitenleiste(&mut self, ui: &mut egui::Ui) -> Option<Aktion> {
        let mut erg = None;
        // --- Anwenden
        ui.horizontal_wrapped(|ui| {
            let laeuft = self.job.is_some();
            let text = if self.geaendert { egui::RichText::new("\u{25B6} Anwenden").strong().color(egui::Color32::from_rgb(255, 210, 80)) } else { egui::RichText::new("\u{25B6} Anwenden").strong() };
            if ui.add_enabled(!laeuft, egui::Button::new(text)).on_hover_text("Bauplan speichern, alle Objekte neu bauen und in die Karte setzen").clicked() {
                self.bauen(true);
            }
            if laeuft {
                ui.spinner();
            }
            ui.checkbox(&mut self.automatisch, "automatisch").on_hover_text("nach jeder Aenderung von selbst neu bauen (1,5 s spaeter)");
        });
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(!self.undo.is_empty(), egui::Button::new("Rueckgaengig")).on_hover_text("Strg+Z").clicked() {
                self.rueckgaengig();
            }
            if ui.add_enabled(!self.redo.is_empty(), egui::Button::new("Wiederholen")).on_hover_text("Strg+Y").clicked() {
                self.wiederholen();
            }
            if ui.button("Kamera hin").on_hover_text("die 3D-Ansicht auf die Kreuzung richten").clicked() {
                erg = Some(Aktion::Kamera(DVec3::new(1084.0, -266.0, 10.0)));
            }
            if ui.button("Speichern").on_hover_text("nur den Bauplan speichern (ohne neu zu bauen)").clicked() {
                self.meldung = match self.speichern() {
                    Ok(()) => format!("Bauplan gespeichert: {}", self.plan_pfad.display()),
                    Err(e) => e,
                };
            }
        });
        if !self.meldung.is_empty() {
            ui.label(egui::RichText::new(&self.meldung).small());
        }
        if let Some(a) = self.pruefung_ui(ui) {
            erg = Some(a);
        }
        if let Some(a) = self.aufnahme_ui(ui) {
            erg = Some(a);
        }
        if self.kontext.lose > 0 {
            ui.colored_label(egui::Color32::from_rgb(255, 120, 90), format!("{} Ampelteile stehen nicht auf Gehweg/Insel", self.kontext.lose));
        }
        ui.separator();
        // --- Werkzeuge
        ui.label(egui::RichText::new("Werkzeug").strong());
        ui.horizontal_wrapped(|ui| {
            for (w, t, tip) in [
                (Werkzeug::Auswahl, "\u{1F446} Auswaehlen", "Bauteil anklicken, ziehen verschiebt, gelbe Punkte ziehen; Doppelklick auf eine Kante fuegt einen Punkt ein, Rechtsklick loescht ihn"),
                (Werkzeug::Textur, "\u{1F5BC} Textur schieben", "gewaehlte Flaeche: Ziehen verschiebt die Textur (auch Alt+Ziehen im Auswaehlen), Alt+Rad skaliert, Strg+Rad dreht"),
                (Werkzeug::Flaeche, "\u{2B1F} Flaeche zeichnen", "Ecken klicken; Klick auf den ersten Punkt, Doppelklick oder Enter schliesst"),
                (Werkzeug::Linie, "\u{2571} Linie zeichnen", "Punkte klicken; Doppelklick oder Enter beendet"),
                (Werkzeug::Gelaender, "Gelaender zeichnen", "Punkte klicken; Doppelklick oder Enter beendet"),
                (Werkzeug::Gehpfad, "Gehpfad zeichnen", "Punkte klicken; Doppelklick oder Enter beendet"),
            ] {
                if ui.selectable_label(self.werkzeug == w, t).on_hover_text(tip).clicked() {
                    self.werkzeug = w;
                    self.entwurf.clear();
                }
            }
        });
        if self.werkzeug == Werkzeug::Flaeche {
            egui::ComboBox::from_id_salt("oe_neue_art").selected_text(FLAECHEN_ARTEN[self.neue_art].1).show_ui(ui, |ui| {
                for (i, (_, n)) in FLAECHEN_ARTEN.iter().enumerate() {
                    ui.selectable_value(&mut self.neue_art, i, *n);
                }
            });
        }
        let hinweis = match self.werkzeug {
            Werkzeug::Auswahl => "Klick waehlt (weiss = unter der Maus, gelb = gewaehlt). Ziehen verschiebt, gelbe Punkte ziehen. Entf loescht, Strg+D dupliziert, Esc hebt auf.",
            Werkzeug::Textur => "Flaeche waehlen und ziehen: die Textur folgt der Maus. Alt+Rad: Groesse, Strg+Rad: Drehung.",
            Werkzeug::Notiz => "Klick setzt die Notiz, den Text rechts eintragen.",
            _ => "Punkte klicken (rastet an Eckpunkten ein, Strg ohne). Doppelklick/Enter beendet, Rechtsklick/Ruecktaste nimmt den letzten zurueck, Esc bricht ab.",
        };
        ui.label(egui::RichText::new(hinweis).small().weak());
        ui.label(egui::RichText::new("Neu an der Kamera-Mitte:").small());
        let c = self.mitte;
        ui.horizontal_wrapped(|ui| {
            for (k, t) in [("halt", "Haltlinie"), ("pfeil", "Pfeil"), ("bus", "BUS"), ("kfz", "Kfz-Ampel"), ("fg", "Fussg.-Ampel"), ("furt", "Furt"), ("wand", "Wand")] {
                if ui.small_button(format!("+ {t}")).clicked() {
                    self.neues(k, c);
                }
            }
        });
        ui.separator();
        // --- Auswahl
        if let Some(w) = self.wahl {
            if let Some(a) = self.eigenschaften(ui, w) {
                erg = Some(a);
            }
        } else {
            ui.label(egui::RichText::new("Kein Bauteil gewaehlt - in der 3D-Ansicht oder im 2D-Plan anklicken, oder unten in der Liste.").weak());
        }
        ui.separator();
        // --- Ansicht
        egui::CollapsingHeader::new("Ansicht").default_open(false).show(ui, |ui| {
            ui.checkbox(&mut self.drei_d, "in der 3D-Ansicht bearbeiten");
            ui.checkbox(&mut self.fuellen_3d, "Flaechen in 3D farbig fuellen");
            ui.checkbox(&mut self.plan2d, "2D-Plan mit Luftbild und Texturen");
            ui.checkbox(&mut self.einrasten, "an Eckpunkten einrasten");
            ui.checkbox(&mut self.einsetzen, "beim Anwenden in die Karte setzen");
            ui.horizontal_wrapped(|ui| {
                for (i, n) in EBENEN.iter().enumerate() {
                    ui.checkbox(&mut self.zeige[i], *n);
                }
            });
        });
        self.bauwerk_ui(ui);
        self.liste(ui);
        erg
    }

    /// Bauprüfung: Zusammenfassung, Funde je Art, Klick fährt die Kamera hin
    fn pruefung_ui(&mut self, ui: &mut egui::Ui) -> Option<Aktion> {
        let mut erg = None;
        let n = self.kontext.funde.len();
        let titel = if n == 0 { egui::RichText::new("Baupruefung: keine Funde").color(egui::Color32::from_rgb(120, 220, 120)) }
                    else { egui::RichText::new(format!("Baupruefung: {n} Funde")).color(egui::Color32::from_rgb(255, 150, 90)) };
        egui::CollapsingHeader::new(titel).id_salt("oe_pruefung").default_open(n > 0).show(ui, |ui| {
            ui.checkbox(&mut self.funde_zeigen, "Funde in 3D markieren");
            let mut arten: Vec<String> = self.kontext.funde.iter().map(|f| f.art.clone()).collect();
            arten.sort();
            arten.dedup();
            let mut verlaengern: Option<usize> = None;
            for art in arten {
                let idx: Vec<usize> = (0..n).filter(|&i| self.kontext.funde[i].art == art).collect();
                egui::CollapsingHeader::new(format!("{} ({})", fund_art(&art), idx.len())).id_salt(("oe_fund", art.clone())).show(ui, |ui| {
                    for i in idx.into_iter().take(200) {
                        let f = &self.kontext.funde[i];
                        let t = format!("{} - {}{}", f.teil, f.text, if f.anzahl > 1 { format!(" (x{})", f.anzahl) } else { String::new() });
                        let mut kamera = false;
                        ui.horizontal_wrapped(|ui| {
                            if f.art == "wand_luecke" && !f.bauteil.is_empty()
                                && ui.button(egui::RichText::new("Wand verlaengern + anwenden").strong()).on_hover_text("den Bereich der Wand im Bauplan bis zum Nachbarteil verlaengern (nicht ueber die naechste Wand hinaus); Anwenden baut neu, die Wand endet genau am Anschlussteil").clicked() {
                                verlaengern = Some(i);
                            }
                            kamera = ui.selectable_label(self.fund_wahl == Some(i), egui::RichText::new(t).small()).clicked();
                        });
                        if kamera {
                            self.fund_wahl = Some(i);
                            erg = Some(Aktion::KameraNah(f.pos));
                        }
                    }
                });
            }
            if let Some(i) = verlaengern {
                let f = self.kontext.funde[i].clone();
                self.meldung = self.wand_verlaengern(&f.bauteil, &f.ende, f.abstand);
                // gleich neu bauen und einsetzen: das Ergebnis ist sofort in 3D zu sehen
                self.bauen(true);
            }
        });
        erg
    }

    /// Wand-Bauteil `id` am Ende `ende` ("Anfang" = von, sonst bis) um `abstand` * 2 + 5 m verlaengern, hoechstens bis zur
    /// naechsten Wand gleicher Art und Seite (der Generator laesst sie genau am Anschlussteil enden) -> Meldung
    pub fn wand_verlaengern(&mut self, id: &str, ende: &str, abstand: f64) -> String {
        let Some(i) = self.plan.waende.iter().position(|w| w.id == id) else { return format!("Wand {id} nicht im Bauplan") };
        let w = self.plan.waende[i].clone();
        let d = abstand * 2.0 + 5.0;                  // grosszuegig: der Generator laesst die Wand genau an der Platte enden
        let andere: Vec<(f64, f64)> = self.plan.waende.iter().enumerate().filter(|(k, x)| *k != i && x.art == w.art && x.seite == w.seite).map(|(_, x)| (x.von, x.bis)).collect();
        self.aendern();
        let text = if ende == "Anfang" {
            let grenze = andere.iter().filter(|x| x.1 <= w.von + 1e-6).map(|x| x.1).fold(f64::MIN, f64::max);
            let neu = (w.von - d).max(grenze);
            self.plan.waende[i].von = neu;
            format!("Wand {id}: Anfang {:.1} -> {:.1} m", w.von, neu)
        } else {
            let grenze = andere.iter().filter(|x| x.0 >= w.bis - 1e-6).map(|x| x.0).fold(f64::MAX, f64::min);
            let neu = (w.bis + d).min(grenze);
            self.plan.waende[i].bis = neu;
            format!("Wand {id}: Ende {:.1} -> {:.1} m", w.bis, neu)
        };
        self.wahl = Some(Wahl::Wand(i));
        self.abschliessen();
        format!("{text} - Anwenden baut neu")
    }

    fn eigenschaften(&mut self, ui: &mut egui::Ui, w: Wahl) -> Option<Aktion> {
        let mut erg = None;
        let mut weg = false;
        let mut dup = false;
        let titel = match w {
            Wahl::Flaeche(_) => "Flaeche", Wahl::Linie(_) => "Linie", Wahl::Halt(_) => "Haltlinie", Wahl::Symbol(_) => "Symbol",
            Wahl::Ampel(_) => "Ampel", Wahl::Gelaender(_) => "Gelaender", Wahl::Furt(_) => "Fussgaengerfurt", Wahl::Gehpfad(_) => "Gehpfad",
            Wahl::Wand(_) => "Wand", Wahl::Strasse(_) => "Strasse", Wahl::Aufnahme(_) => "Bauaufnahme", Wahl::Notiz(_) => "Notiz",
        };
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(titel).strong().size(16.0));
            if !matches!(w, Wahl::Strasse(_)) {
                if ui.small_button("duplizieren").on_hover_text("Strg+D").clicked() {
                    dup = true;
                }
                if ui.small_button("loeschen").on_hover_text("Entf").clicked() {
                    weg = true;
                }
            }
        });
        if !matches!(w, Wahl::Wand(_) | Wahl::Strasse(_)) {
            ui.horizontal(|ui| {
                ui.label("drehen");
                for (t, g) in [("-15\u{B0}", -15.0), ("-1\u{B0}", -1.0), ("+1\u{B0}", 1.0), ("+15\u{B0}", 15.0)] {
                    if ui.small_button(t).clicked() {
                        self.aendern();
                        self.drehen(w, g);
                    }
                }
            });
        }
        if dup {
            self.aendern();
            self.wahl = Some(self.duplizieren(w));
            return None;
        }
        if weg {
            self.aendern();
            self.loeschen(w);
            self.wahl = None;
            return None;
        }
        let mut ae = false;
        let mut uebernehmen = None;
        match w {
            Wahl::Aufnahme(i) => {
                let mitte = self.aufnahme.linien.get(i).map(|l| l.punkte[l.punkte.len() / 2]).unwrap_or(self.mitte);
                let abw: Vec<String> = self.aufnahme.linien.get(i).map(|l| self.abweichungen.iter().filter(|a| a.bezug == l.id).map(|a| a.text.clone()).collect()).unwrap_or_default();
                let Some(l) = self.aufnahme.linien.get_mut(i) else { return None };
                egui::ComboBox::from_id_salt("oe_a_art").selected_text(aufnahme::art_name(&l.art)).show_ui(ui, |ui| {
                    for a in aufnahme::ARTEN {
                        if ui.selectable_label(l.art == a.0, egui::RichText::new(a.1).color(aufnahme::farbe(a.0))).clicked() {
                            l.art = a.0.to_string();
                            ae = true;
                        }
                    }
                });
                let laenge: f64 = l.punkte.windows(2).map(|x| (x[1] - x[0]).length()).sum();
                ui.label(format!("{} Punkte, {laenge:.1} m", l.punkte.len()));
                ui.label("Notiz");
                ae |= ui.add(egui::TextEdit::multiline(&mut l.notiz).desired_rows(2).hint_text("z. B. Stahlgelaender gruen, 1,1 m")).changed();
                let art = l.art.clone();
                ui.horizontal_wrapped(|ui| self.bilder_knoepfe(ui, mitte));
                if aufnahme::baubar(&art) && !matches!(art.as_str(), "bord" | "mauer") {
                    if ui.button("In den Bauplan uebernehmen").on_hover_text("legt das passende Bauteil im Bauplan genau auf diese Linie").clicked() {
                        uebernehmen = Some(i);
                    }
                }
                if abw.is_empty() {
                    ui.label(egui::RichText::new("passt zum Gebauten").small().color(egui::Color32::from_rgb(120, 220, 120)));
                } else {
                    for t in abw {
                        ui.label(egui::RichText::new(t).small().color(egui::Color32::from_rgb(255, 120, 190)));
                    }
                }
            }
            Wahl::Notiz(i) => {
                let pos = self.aufnahme.notizen.get(i).map(|n| n.pos).unwrap_or(self.mitte);
                let Some(n) = self.aufnahme.notizen.get_mut(i) else { return None };
                ae |= ui.add(egui::TextEdit::multiline(&mut n.text).desired_rows(4).hint_text("was hier wirklich ist (Foto, Mapillary ...)")).changed();
                ui.horizontal_wrapped(|ui| self.bilder_knoepfe(ui, pos));
            }
            Wahl::Flaeche(i) => {
                let Some(fl) = self.plan.flaechen.get_mut(i) else { return None };
                egui::ComboBox::from_id_salt("oe_f_art").selected_text(FLAECHEN_ARTEN.iter().find(|a| a.0 == fl.art).map(|a| a.1).unwrap_or(&fl.art)).show_ui(ui, |ui| {
                    for (k, n) in FLAECHEN_ARTEN {
                        if ui.selectable_label(fl.art == k, n).clicked() {
                            fl.art = k.to_string();
                            ae = true;
                        }
                    }
                });
                ui.label(format!("{} Eckpunkte, {:.1} m\u{B2}", fl.punkte.len(), flaeche2(&fl.punkte).abs()));
                ui.label(egui::RichText::new("Textur (Werkzeug \"Textur schieben\" oder Zahlen)").strong());
                ui.horizontal(|ui| {
                    ae |= ui.add(egui::DragValue::new(&mut fl.textur.dx).speed(0.01).prefix("u ")).changed();
                    ae |= ui.add(egui::DragValue::new(&mut fl.textur.dy).speed(0.01).prefix("v ")).changed();
                });
                ui.horizontal(|ui| {
                    ae |= ui.add(egui::DragValue::new(&mut fl.textur.skala).speed(0.01).range(0.05..=20.0).prefix("Groesse ")).changed();
                    ae |= ui.add(egui::DragValue::new(&mut fl.textur.dreh).speed(0.5).suffix("\u{B0}").prefix("Drehung ")).changed();
                });
                if ui.small_button("Textur zuruecksetzen").clicked() {
                    fl.textur = Textur { dx: 0.0, dy: 0.0, skala: 1.0, dreh: 0.0 };
                    ae = true;
                }
            }
            Wahl::Linie(i) => {
                let Some(l) = self.plan.linien.get_mut(i) else { return None };
                ae |= ui.add(egui::DragValue::new(&mut l.breite).speed(0.005).range(0.05..=2.0).prefix("Breite ").suffix(" m")).changed();
                ui.horizontal(|ui| {
                    ae |= ui.add(egui::DragValue::new(&mut l.an).speed(0.05).range(0.0..=20.0).prefix("Strich ").suffix(" m")).changed();
                    ae |= ui.add(egui::DragValue::new(&mut l.aus).speed(0.05).range(0.0..=20.0).prefix("Luecke ").suffix(" m")).changed();
                });
                ui.horizontal_wrapped(|ui| {
                    for (t, b, a, u) in [("durchgezogen", 0.12, 0.0, 0.0), ("Leitlinie 3/6", 0.12, 3.0, 6.0), ("Leitlinie 6/3", 0.12, 6.0, 3.0),
                                         ("Radfurt-Rand", 0.15, 0.5, 0.5), ("Breitstrich", 0.25, 0.0, 0.0), ("Abbieger", 0.12, 1.5, 1.5), ("Haltlinie", 0.5, 0.0, 0.0)] {
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
                let Some(h) = self.plan.haltlinien.get_mut(i) else { return None };
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
                let Some(sy) = self.plan.symbole.get_mut(i) else { return None };
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
                let Some(a) = self.plan.ampeln.get_mut(i) else { return None };
                ui.horizontal(|ui| {
                    ae |= ui.selectable_value(&mut a.art, "kfz".to_string(), "Kfz").changed();
                    ae |= ui.selectable_value(&mut a.art, "fg".to_string(), "Fussgaenger").changed();
                });
                ui.horizontal(|ui| {
                    ae |= ui.selectable_value(&mut a.mast, "klein".to_string(), "kleiner Mast").changed();
                    ae |= ui.selectable_value(&mut a.mast, "ausleger".to_string(), "Ausleger").changed();
                });
                ui.horizontal(|ui| {
                    ae |= ui.add(egui::DragValue::new(&mut a.richtung).speed(0.5).suffix("\u{B0}").prefix("blickt nach ")).changed();
                    ae |= ui.add(egui::DragValue::new(&mut a.gruppe).range(0..=9).prefix("Phase ")).changed();
                });
            }
            Wahl::Gelaender(i) => {
                let Some(g) = self.plan.gelaender.get_mut(i) else { return None };
                ui.horizontal(|ui| {
                    ae |= ui.selectable_value(&mut g.unten, "gelaende".to_string(), "Wand bis zum Gelaende").changed();
                    ae |= ui.selectable_value(&mut g.unten, "trog".to_string(), "Trogrand").changed();
                });
                ae |= ui.checkbox(&mut g.flip, "Seite tauschen").changed();
            }
            Wahl::Furt(i) => {
                let Some(fu) = self.plan.furten.get_mut(i) else { return None };
                ui.horizontal(|ui| {
                    ae |= ui.add(egui::DragValue::new(&mut fu.breite).speed(0.05).range(1.0..=10.0).prefix("Breite ").suffix(" m")).changed();
                    ae |= ui.add(egui::DragValue::new(&mut fu.gruppe).range(0..=9).prefix("Phase ")).changed();
                });
            }
            Wahl::Gehpfad(_) => {
                ui.label(egui::RichText::new("Fussgaengerpfad").small().weak());
            }
            Wahl::Wand(i) => {
                let Some(wd) = self.plan.waende.get_mut(i) else { return None };
                ui.horizontal(|ui| {
                    ae |= ui.selectable_value(&mut wd.art, "trenn".to_string(), "Trennwand (Ring/Rampe)").changed();
                    ae |= ui.selectable_value(&mut wd.art, "aussen".to_string(), "Aussenwand").changed();
                });
                ui.horizontal(|ui| {
                    ae |= ui.selectable_value(&mut wd.seite, "w".to_string(), "West").changed();
                    ae |= ui.selectable_value(&mut wd.seite, "o".to_string(), "Ost").changed();
                });
                ui.horizontal(|ui| {
                    ui.label("Material");
                    for (k, n) in [("klinker", "Klinker"), ("beton", "Beton"), ("gabionen", "Gabionen")] {
                        ae |= ui.selectable_value(&mut wd.material, k.to_string(), n).changed();
                    }
                });
                if wd.art == "aussen" {
                    ui.horizontal(|ui| {
                        ui.label("Laermschutz");
                        for (k, n) in [("keine", "keiner"), ("gruen", "gruen"), ("glas", "Glas")] {
                            ae |= ui.selectable_value(&mut wd.laerm, k.to_string(), n).changed();
                        }
                    });
                    ae |= ui.checkbox(&mut wd.stufig, "zweistufig mit Pflanzstreifen (ab 3,5 m, Klinker)").changed();
                }
                ui.horizontal(|ui| {
                    ae |= ui.add(egui::DragValue::new(&mut wd.von).speed(0.5).prefix("von ").suffix(" m")).changed();
                    ae |= ui.add(egui::DragValue::new(&mut wd.bis).speed(0.5).prefix("bis ").suffix(" m")).changed();
                });
                ui.horizontal(|ui| {
                    ae |= ui.add(egui::DragValue::new(&mut wd.versatz).speed(0.02).range(-5.0..=5.0).prefix("Versatz ").suffix(" m")).on_hover_text("nach aussen (weg vom Ring) positiv").changed();
                    ae |= ui.add(egui::DragValue::new(&mut wd.hoehe).speed(0.02).range(-5.0..=10.0).prefix("Hoehe +").suffix(" m")).changed();
                });
                ui.label(egui::RichText::new("Stationen: m entlang der Ring-Achse (Nord = 0). Enden mit den gelben Punkten ziehen.").small().weak());
                if ui.small_button("in der Mitte teilen").clicked() {
                    let mut b = wd.clone();
                    let m = (wd.von + wd.bis) / 2.0;
                    wd.bis = m;
                    b.von = m;
                    b.id = format!("{}b", b.id);
                    self.aendern();
                    self.plan.waende.push(b);
                }
            }
            Wahl::Strasse(i) => {
                let Some((name, _)) = self.kontext.strassen.get(i).cloned() else { return None };
                ui.label(egui::RichText::new(name.replace('_', " ")).strong());
                let st = self.plan.strassen().into_iter().find(|s| s.0 == name);
                match st {
                    Some((_, q, versatz)) => {
                        if let Some(teile) = q["teile"].as_array() {
                            let zeile: Vec<String> = teile.iter().map(|t| format!("{} {:.2}", s(t, "art"), f(t, "breite", 0.0))).collect();
                            ui.label(egui::RichText::new(zeile.join(" | ")).small());
                        }
                        if ui.button("Querschnitt im Baukasten bearbeiten ...").clicked() {
                            erg = Some(Aktion::Querschnitt(name.clone(), q.clone()));
                        }
                        if let Some(mut v) = versatz {
                            if ui.add(egui::DragValue::new(&mut v).speed(0.02).prefix("Lage zur Ring-Achse ").suffix(" m")).on_hover_text("Abstand der Spurmitte von der Ring-Achse (West positiv)").changed() {
                                self.aendern();
                                if let Some(x) = self.plan.strasse_mut(&name) {
                                    x["versatz"] = v.into();
                                }
                            }
                        }
                        ui.label(egui::RichText::new("Die Linie zeigt den letzten Bau; Anwenden verlegt die Strasse.").small().weak());
                    }
                    None => {
                        ui.label("noch kein Querschnitt im Bauplan - einmal Anwenden");
                    }
                }
            }
        }
        // Punkte als Zahlen
        if !matches!(w, Wahl::Wand(_) | Wahl::Strasse(_)) {
            egui::CollapsingHeader::new("Punkte als Zahlen").id_salt(("oe_punkte", format!("{w:?}"))).show(ui, |ui| {
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
        }
        if ae {
            self.aendern();
        }
        if let Some(i) = uebernehmen {
            self.meldung = self.uebernehmen(i);
        }
        erg
    }

    fn bauwerk_ui(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Bahntunnel und Ampelphasen").show(ui, |ui| {
            let mut ae = false;
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
            ui.label("Ampelphase je Zufahrt");
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
                self.plan.rest.insert("tunnel".into(), tun);
                self.plan.rest.insert("phasen".into(), serde_json::json!(phasen));
            }
        });
    }

    fn liste(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Alle Bauteile").show(ui, |ui| {
            let mut neu = None;
            let mut gruppe = |ui: &mut egui::Ui, titel: &str, eintraege: Vec<(Wahl, String)>, wahl: Option<Wahl>| {
                egui::CollapsingHeader::new(format!("{titel} ({})", eintraege.len())).id_salt(("oe_l", titel.to_string())).show(ui, |ui| {
                    for (w, t) in eintraege {
                        if ui.selectable_label(wahl == Some(w), t).clicked() {
                            neu = Some(w);
                        }
                    }
                });
            };
            let p = &self.plan;
            gruppe(ui, "Waende", p.waende.iter().enumerate().map(|(i, x)| (Wahl::Wand(i), format!("{} {} {:.0}-{:.0} m, {}{}", if x.art == "trenn" { "Trennwand" } else { "Aussenwand" },
                if x.seite == "w" { "West" } else { "Ost" }, x.von, x.bis, x.material, if x.laerm != "keine" { format!(", LSW {}", x.laerm) } else { String::new() }))).collect(), self.wahl);
            gruppe(ui, "Strassen", self.kontext.strassen.iter().enumerate().map(|(i, x)| (Wahl::Strasse(i), x.0.replace('_', " "))).collect(), self.wahl);
            gruppe(ui, "Flaechen", p.flaechen.iter().enumerate().map(|(i, x)| (Wahl::Flaeche(i), format!("{} ({})", x.id, x.art))).collect(), self.wahl);
            gruppe(ui, "Linien", p.linien.iter().enumerate().map(|(i, x)| (Wahl::Linie(i), format!("{} ({:.2} m{})", x.id, x.breite, if x.aus > 0.0 { ", gestrichelt" } else { "" }))).collect(), self.wahl);
            gruppe(ui, "Haltlinien", p.haltlinien.iter().enumerate().map(|(i, x)| (Wahl::Halt(i), format!("{} (Arm {})", x.id, x.arm))).collect(), self.wahl);
            gruppe(ui, "Pfeile und BUS", p.symbole.iter().enumerate().map(|(i, x)| (Wahl::Symbol(i), format!("{} {}", x.id, x.pfeile.join("/")))).collect(), self.wahl);
            gruppe(ui, "Ampeln", p.ampeln.iter().enumerate().map(|(i, x)| (Wahl::Ampel(i), format!("{} ({}, {}, Phase {})", x.id, x.art, x.mast, x.gruppe))).collect(), self.wahl);
            gruppe(ui, "Gelaender", p.gelaender.iter().enumerate().map(|(i, x)| (Wahl::Gelaender(i), format!("{} ({})", x.id, x.unten))).collect(), self.wahl);
            gruppe(ui, "Furten", p.furten.iter().enumerate().map(|(i, x)| (Wahl::Furt(i), x.id.clone())).collect(), self.wahl);
            gruppe(ui, "Gehpfade", p.gehpfade.iter().enumerate().map(|(i, x)| (Wahl::Gehpfad(i), x.id.clone())).collect(), self.wahl);
            let a = &self.aufnahme;
            gruppe(ui, "Bauaufnahme", a.linien.iter().enumerate().map(|(i, x)| (Wahl::Aufnahme(i), format!("{} ({})", aufnahme::art_name(&x.art), x.id))).collect(), self.wahl);
            gruppe(ui, "Notizen", a.notizen.iter().enumerate().map(|(i, x)| (Wahl::Notiz(i), if x.text.is_empty() { x.id.clone() } else { x.text.lines().next().unwrap_or("").chars().take(40).collect() })).collect(), self.wahl);
            if let Some(w) = neu {
                self.wahl = Some(w);
                let (l, _) = self.umriss(w);
                if !l.is_empty() {
                    self.mitte = l.iter().fold(DVec2::ZERO, |a, b| a + *b) / l.len() as f64;
                }
            }
        });
    }

    /// Kamera-Mitte der 3D-Ansicht (fuer "Neu an der Kamera-Mitte")
    pub fn mitte_setzen(&mut self, p: DVec2) {
        self.mitte = p;
    }

    // ---------------------------------------------------------- 2D-Plan
    fn zeichenflaeche(&mut self, ui: &mut egui::Ui) {
        let groesse = ui.available_size();
        let (resp, p) = ui.allocate_painter(groesse, egui::Sense::click_and_drag());
        let rect = resp.rect;
        let c = rect.center();
        let (mitte, m) = (self.mitte, self.m_px);
        let bild = move |w: DVec3| Some(egui::pos2(c.x + ((w.x - mitte.x) as f32) * m, c.y - ((w.y - mitte.y) as f32) * m));
        let welt = move |s_: egui::Pos2| DVec2::new(mitte.x + ((s_.x - c.x) / m) as f64, mitte.y - ((s_.y - c.y) / m) as f64);
        p.rect_filled(rect, 0.0, egui::Color32::from_gray(24));
        if let Some((t, b)) = &self.luftbild {
            if self.luft_alpha > 0.01 {
                let r = egui::Rect::from_two_pos(bild(DVec3::new(b[0], b[3], 0.0)).unwrap(), bild(DVec3::new(b[2], b[1], 0.0)).unwrap());
                p.image(t.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::from_white_alpha((self.luft_alpha * 255.0) as u8));
            }
        }
        let deck = if self.luftbild.is_some() { (255.0 * (1.0 - self.luft_alpha * 0.55)) as u8 } else { 255 };
        self.zeichnen(&ui.ctx().clone(), &p, &bild, &|_| 0.0, m, true, true, deck);
        // Bedienung ueber dieselben Funktionen wie in 3D
        let maus = resp.hover_pos().map(welt);
        let tol = 7.0 / m as f64;
        let (alt, strg, umschalt) = ui.input(|i| (i.modifiers.alt, i.modifiers.ctrl || i.modifiers.command, i.modifiers.shift));
        let rechts = ui.input(|i| i.pointer.secondary_down() || i.pointer.middle_down());
        self.hover(maus, tol);
        if resp.drag_started() {
            let start = ui.input(|i| i.pointer.press_origin()).map(welt).or(maus);
            if let Some(q) = start {
                if rechts {
                    self.zug = Zug::Schwenken;
                } else {
                    self.druecken(q, tol, alt);
                }
            }
        }
        if resp.dragged() {
            if self.zug == Zug::Schwenken {
                let dd = resp.drag_delta();
                self.mitte -= DVec2::new((dd.x / m) as f64, -(dd.y / m) as f64);
            } else if let Some(q) = maus {
                self.ziehen(q, tol, strg);
            }
        }
        if resp.drag_stopped() {
            self.loslassen();
        }
        if resp.clicked() {
            if let Some(q) = maus {
                self.klicken(q, tol, strg);
            }
        }
        if resp.double_clicked() {
            if let Some(q) = maus {
                self.doppelklick(q, tol);
            }
        }
        if resp.secondary_clicked() {
            if let Some(q) = maus {
                self.rechtsklick(q, tol);
            }
        }
        if resp.hovered() {
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
            if rad != 0.0 && !self.rad(rad, alt, strg) {
                let vor = maus;
                self.m_px = (self.m_px * 1.15f32.powf(rad)).clamp(1.0, 200.0);
                if let (Some(v), Some(hp)) = (vor, resp.hover_pos()) {
                    let nach = DVec2::new(self.mitte.x + ((hp.x - c.x) / self.m_px) as f64, self.mitte.y - ((hp.y - c.y) / self.m_px) as f64);
                    self.mitte += v - nach;
                }
            }
            let tasten: Vec<egui::Key> = ui.input(|i| i.events.iter().filter_map(|e| match e {
                egui::Event::Key { key, pressed: true, .. } => Some(*key),
                _ => None,
            }).collect());
            for k in tasten {
                if self.taste(k, umschalt, strg) {
                    continue;
                }
                if let Some(w) = self.wahl {
                    let schritt = if umschalt { 0.5 } else { 0.05 };
                    let d = match k {
                        egui::Key::ArrowLeft => DVec2::new(-schritt, 0.0),
                        egui::Key::ArrowRight => DVec2::new(schritt, 0.0),
                        egui::Key::ArrowUp => DVec2::new(0.0, schritt),
                        egui::Key::ArrowDown => DVec2::new(0.0, -schritt),
                        _ => continue,
                    };
                    self.aendern();
                    let g = self.griffe(w).first().copied().unwrap_or_default();
                    self.verschieben(w, g, g + d);
                    self.abschliessen();
                }
            }
        }
        p.text(rect.right_bottom() + egui::vec2(-8.0, -8.0), egui::Align2::RIGHT_BOTTOM, "Luftbild: DOP40 (c) Bayerische Vermessungsverwaltung, CC BY 4.0",
               egui::FontId::proportional(10.0), egui::Color32::from_gray(180));
        ui.ctx().request_repaint_after(Duration::from_millis(100));
        let _ = umschalt;
        ui.horizontal(|ui| {
            ui.label("Luftbild");
            ui.add(egui::Slider::new(&mut self.luft_alpha, 0.0..=1.0).show_value(false));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bauplan_hin_und_zurueck() {
        let j = serde_json::json!({
            "version": 1, "name": "K", "phasen": [0, 1], "_id": 5,
            "strassen": {"Ring": {"querschnitt": {"name": "R", "teile": []}, "versatz": null}, "Rampe": {"querschnitt": {}, "versatz": 9.4}},
            "flaechen": [{"id": "f1", "art": "sperr", "punkte": [[0, 0], [4, 0], [4, 3]], "loecher": [], "textur": {"dx": 0.5, "dy": 0, "skala": 2, "dreh": 10}}],
            "linien": [{"id": "l1", "punkte": [[0, 0], [1, 1]], "breite": 0.12, "an": 3, "aus": 6}],
            "haltlinien": [], "symbole": [{"id": "p1", "art": "pfeil", "pos": [1, 2], "richtung": 90, "pfeile": ["links"], "skala": 1}],
            "ampeln": [{"id": "a1", "art": "kfz", "pos": [5, 5], "richtung": 180, "mast": "ausleger", "gruppe": 2}],
            "gelaender": [], "furten": [], "gehpfade": [],
            "waende": [{"id": "w1", "seite": "w", "art": "aussen", "von": 10, "bis": 50, "material": "gabionen", "laerm": "gruen", "versatz": 0.2, "hoehe": 0, "stufig": true}]
        });
        let mut b = Bauplan::aus_json(&j);
        assert_eq!(b.flaechen[0].textur, Textur { dx: 0.5, dy: 0.0, skala: 2.0, dreh: 10.0 });
        assert_eq!(b.neue_id("x"), "x6");
        assert_eq!(b.waende[0].laerm, "gruen");
        assert_eq!(b.strassen().len(), 2);
        assert_eq!(b.strassen().iter().find(|s| s.0 == "Rampe").unwrap().2, Some(9.4));
        let k = b.json();
        assert_eq!(k["phasen"], serde_json::json!([0, 1]));
        assert_eq!(k["waende"][0]["versatz"], 0.2);
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
    }

    fn editor() -> ObjektEditor {
        let mut ed = ObjektEditor::leer();
        ed.kontext.stationen = (0..50).map(|k| {
            let s = k as f64 * 2.0;
            Station { s, p: DVec2::new(0.0, -s), q: DVec2::new(1.0, 0.0), t: [("aussen_w".to_string(), -10.0), ("aussen_o".to_string(), 10.0)].into_iter().collect() }
        }).collect();
        ed
    }

    #[test]
    fn bedienen_zeichnen_und_verschieben() {
        let mut ed = editor();
        // Flaeche zeichnen: drei Klicks, Klick auf den ersten Punkt schliesst
        ed.werkzeug = Werkzeug::Flaeche;
        for p in [(10.0, 10.0), (14.0, 10.0), (14.0, 14.0)] {
            ed.klicken(DVec2::new(p.0, p.1), 0.3, true);
        }
        ed.klicken(DVec2::new(10.05, 10.0), 0.3, true);
        assert_eq!(ed.plan.flaechen.len(), 1);
        assert_eq!(ed.werkzeug, Werkzeug::Auswahl);
        // ziehen verschiebt, Rueckgaengig nimmt es zurueck
        ed.druecken(DVec2::new(13.0, 11.0), 0.3, false);
        ed.ziehen(DVec2::new(14.0, 11.0), 0.3, false);
        ed.loslassen();
        assert!((ed.plan.flaechen[0].punkte[0].x - 11.0).abs() < 1e-9);
        ed.rueckgaengig();
        assert!((ed.plan.flaechen[0].punkte[0].x - 10.0).abs() < 1e-9);
        // Textur schieben: die Lage der Flaeche bleibt, die Textur wandert
        ed.wahl = Some(Wahl::Flaeche(0));
        ed.werkzeug = Werkzeug::Textur;
        ed.druecken(DVec2::new(13.0, 11.0), 0.3, false);
        ed.ziehen(DVec2::new(15.0, 11.0), 0.3, false);
        ed.loslassen();
        assert!((ed.plan.flaechen[0].punkte[0].x - 10.0).abs() < 1e-9);
        assert!((ed.plan.flaechen[0].textur.dx + 0.5).abs() < 1e-9, "{}", ed.plan.flaechen[0].textur.dx);
    }

    #[test]
    fn bauaufnahme_zeichnen_abgleichen_uebernehmen() {
        let mut ed = editor();
        let p = |x: f64, y: f64| DVec2::new(x, y);
        // Gelaender nachzeichnen: Werkzeug bleibt fuer die naechste Linie
        ed.werkzeug = Werkzeug::Aufnahme;
        ed.aufnahme_art = aufnahme::art_index("gelaender");
        for q in [p(20.0, 0.0), p(20.0, -30.0)] {
            ed.klicken(q, 0.3, true);
        }
        ed.taste(egui::Key::Enter, false, false);
        assert_eq!(ed.aufnahme.linien.len(), 1);
        assert_eq!(ed.werkzeug, Werkzeug::Aufnahme);
        // Notiz per Klick
        ed.werkzeug = Werkzeug::Notiz;
        ed.klicken(p(25.0, -5.0), 0.3, true);
        assert_eq!(ed.aufnahme.notizen.len(), 1);
        assert_eq!(ed.wahl, Some(Wahl::Notiz(0)));
        // Abgleich: im Bauplan gibt es das Gelaender nicht
        ed.abgleich_aktuell();
        assert_eq!(ed.abweichungen.iter().filter(|a| a.was == "fehlt").count(), 1, "{:?}", ed.abweichungen);
        // uebernehmen -> Gelaender im Bauplan genau auf der Linie, Abgleich sauber
        let msg = ed.uebernehmen(0);
        assert!(msg.contains("uebernommen"), "{msg}");
        assert_eq!(ed.plan.gelaender.len(), 1);
        ed.abgleich_aktuell();
        assert!(ed.abweichungen.is_empty(), "{:?}", ed.abweichungen);
        // Rueckgaengig nimmt Bauplan und Aufnahme gemeinsam zurueck
        ed.rueckgaengig();
        assert_eq!(ed.plan.gelaender.len(), 0);
        assert_eq!(ed.aufnahme.notizen.len(), 1);
        ed.rueckgaengig();
        assert_eq!(ed.aufnahme.notizen.len(), 0);
        assert_eq!(ed.aufnahme.linien.len(), 1);
    }

    #[test]
    fn wand_verlaengern_bis_zur_naechsten_wand() {
        let mut ed = editor();
        let w = |id: &str, von: f64, bis: f64| Wand { id: id.into(), seite: "o".into(), art: "aussen".into(), von, bis, material: "klinker".into(),
                                                     laerm: "keine".into(), versatz: 0.0, hoehe: 0.0, stufig: true };
        ed.plan.waende = vec![w("a", 10.0, 40.0), w("b", 44.0, 80.0)];
        ed.wand_verlaengern("a", "Ende", 9.0);
        assert_eq!(ed.plan.waende[0].bis, 44.0);
        ed.wand_verlaengern("b", "Anfang", 1.5);
        assert_eq!(ed.plan.waende[1].von, 44.0);
        ed.wand_verlaengern("a", "Anfang", 3.0);
        assert_eq!(ed.plan.waende[0].von, -1.0);
        ed.rueckgaengig();
        assert_eq!(ed.plan.waende[0].von, 10.0);
    }

    #[test]
    fn wand_enden_ziehen() {
        let mut ed = editor();
        ed.neues("wand", DVec2::new(-9.0, -40.0));
        assert_eq!(ed.plan.waende.len(), 1);
        assert_eq!(ed.plan.waende[0].seite, "w");
        assert!((ed.plan.waende[0].von - 30.0).abs() < 1e-9 && (ed.plan.waende[0].bis - 50.0).abs() < 1e-9);
        // Ende bei Station 50 (Punkt -10/-50) nach Station 70 ziehen
        ed.druecken(DVec2::new(-10.0, -50.0), 0.5, false);
        ed.ziehen(DVec2::new(-10.2, -70.0), 0.5, false);
        ed.loslassen();
        assert!((ed.plan.waende[0].bis - 70.0).abs() < 1e-9, "{}", ed.plan.waende[0].bis);
        assert!(ed.treffer(DVec2::new(-10.0, -60.0), 0.5) == Some(Wahl::Wand(0)));
    }
}
