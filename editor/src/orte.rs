//! World Editor, "Einsetzpunkte" und "Haltestellen".
//!
//! - Einsetzpunkt: unsichtbares Objekt `Sceneryobjects\Generic\entrypoint_bus.sco` in der Kachel und ein Eintrag in
//!   global.cfg `[entrypoints]` (12 Zeilen: Index des Objekts in der Kachel - alle Objekt-Eintraege gezaehlt wie die
//!   "Object Nr."-Kommentare -, Objekt-ID, 0, x, Hoehe, z lokal, Quaternion der Drehung, Kachelnummer, Name).
//! - Haltestelle: Objekt mit `[busstop]` (Standard `Generic\bus_stop.sco`), erster Text = Name; TTData/Busstops.cfg
//!   fuehrt sie fuer die Fahrplaene (`[busstop]` Name, Kachelnummer, Objekt-ID, Versatz, 0, 0).
//!
//! Die Objekte schreibt das Aendern-Werkzeug sofort in die Sitzungskopien der Kacheln; die Listen haelt der Editor und
//! schreibt sie beim Speichern - dann aus den fertigen Kacheln neu berechnet (Index, Lage, Kachelnummer).

use crate::aendern::Aendern;
use crate::welt::Liste;
use anyhow::{bail, Result};
use glam::{DVec2, DVec3};
use openomsi_game::viewer::{LaneKind, Viewer};
use std::collections::HashMap;
use std::path::Path;

pub const EINSETZPUNKT: &str = "Sceneryobjects\\Generic\\entrypoint_bus.sco";
pub const HALTESTELLE: &str = "Sceneryobjects\\Generic\\bus_stop.sco";

#[derive(Clone, Debug, PartialEq)]
pub struct Einsetzpunkt {
    pub name: String,
    pub id: i64,
    /// Lage (Welt) und Drehung beim Lesen bzw. Anlegen, und die Kachel, in deren Datei das Objekt steht
    pub pos: DVec3,
    pub rot: f64,
    pub kachel: (i32, i32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Busstop {
    pub name: String,
    pub id: i64,
    pub versatz: String,
}

#[derive(Clone, Debug)]
pub struct Haltestelle {
    pub name: String,
    pub id: i64,
    pub pos: DVec3,
    pub kachel: (i32, i32),
}

#[derive(Clone, PartialEq)]
struct Stand {
    punkte: Vec<Einsetzpunkt>,
    busstops: Vec<Busstop>,
}

#[derive(Clone)]
struct Schritt {
    vorher: Stand,
    nachher: Stand,
    /// Schritte des Aendern-Werkzeugs (Objekte in den Kacheln), die dazugehoeren
    aendern: usize,
}

/// was beim Speichern geschrieben wird
#[derive(Clone, Debug)]
pub struct Daten {
    /// Einsetzpunkte: Name, Objekt-ID, Lage (Welt, mit Aenderungen aus dem Objekte-Werkzeug), Drehung
    pub punkte: Vec<(String, i64, DVec3, f64)>,
    pub busstops: Vec<Busstop>,
}

#[derive(Default)]
pub struct Orte {
    stand: Option<Stand>,
    undo: Vec<Schritt>,
    redo: Vec<Schritt>,
    pub aenderungen: usize,
    /// Haltestellen der Karte (aus den Kacheln gelesen; nach jeder Aenderung neu)
    halte: Option<Vec<Haltestelle>>,
    sco_halt: HashMap<String, bool>,
}

impl Orte {
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    /// Listen aus der Karte lesen (beim ersten Gebrauch): global.cfg und TTData/Busstops.cfg der Karte `ordner`;
    /// `liste`: Kachelliste beim Oeffnen (auf sie zeigen die Kachelnummern)
    pub fn laden(&mut self, ordner: &Path, liste: &Liste) {
        if self.stand.is_some() {
            return;
        }
        let ts = omsi_map::tile_size();
        let mut punkte = Vec::new();
        let g = std::fs::read(ordner.join("global.cfg")).map(|b| crate::speichern::dekodieren(&b).0).unwrap_or_default();
        let z: Vec<&str> = g.lines().collect();
        if let Some(i) = z.iter().position(|l| l.trim().eq_ignore_ascii_case("[entrypoints]")) {
            let n: usize = z.get(i + 1).and_then(|x| x.trim().parse().ok()).unwrap_or(0);
            for k in 0..n {
                let Some(r) = z.get(i + 2 + k * 12..i + 14 + k * 12) else { break };
                let f = |j: usize| r[j].trim().replace(',', ".").parse::<f64>().unwrap_or(0.0);
                let Some((tx, ty, _)) = liste.get(f(10) as usize) else { continue };
                let rot = (2.0 * f(7).atan2(f(9))).to_degrees().rem_euclid(360.0);
                punkte.push(Einsetzpunkt {
                    name: r[11].trim().to_string(), id: f(1) as i64, rot, kachel: (*tx, *ty),
                    pos: DVec3::new(*tx as f64 * ts + f(3), *ty as f64 * ts + f(5), f(4)),
                });
            }
        }
        let mut busstops = Vec::new();
        let b = std::fs::read(ordner.join("TTData").join("Busstops.cfg")).map(|b| crate::speichern::dekodieren(&b).0).unwrap_or_default();
        let z: Vec<&str> = b.lines().collect();
        for i in 0..z.len() {
            if z[i].trim().eq_ignore_ascii_case("[busstop]") && i + 4 < z.len() {
                busstops.push(Busstop { name: z[i + 1].trim().to_string(), id: z[i + 3].trim().parse().unwrap_or(0), versatz: z[i + 4].trim().to_string() });
            }
        }
        self.stand = Some(Stand { punkte, busstops });
    }

    pub fn punkte(&self) -> &[Einsetzpunkt] {
        self.stand.as_ref().map(|s| s.punkte.as_slice()).unwrap_or(&[])
    }

    pub fn busstops(&self) -> &[Busstop] {
        self.stand.as_ref().map(|s| s.busstops.as_slice()).unwrap_or(&[])
    }

    /// Haltestellen der Karte (Objekte mit [busstop] in allen Kacheln)
    pub fn haltestellen(&mut self, v: &Viewer) -> Vec<Haltestelle> {
        if let Some(h) = &self.halte {
            return h.clone();
        }
        let ts = omsi_map::tile_size();
        let mut out = Vec::new();
        for (tx, ty, _) in v.map_tile_refs() {
            let Some(p) = v.tile_file(tx, ty) else { continue };
            let Ok(t) = omsi_map::Tile::load(&p) else { continue };
            for o in &t.objects {
                let halt = *self.sco_halt.entry(o.file.to_ascii_lowercase()).or_insert_with(|| {
                    std::fs::read(v.root.join(o.file.replace('\\', "/"))).map(|b| String::from_utf8_lossy(&b).to_ascii_lowercase().contains("[busstop]")).unwrap_or(false)
                });
                if halt {
                    out.push(Haltestelle { name: o.extra.first().cloned().unwrap_or_default(), id: o.id, kachel: (tx, ty),
                                           pos: DVec3::new(tx as f64 * ts + o.pos[0], ty as f64 * ts + o.pos[1], o.pos[2]) });
                }
            }
        }
        out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        self.halte = Some(out.clone());
        out
    }

    fn schritt(&mut self, vorher: Stand, aendern: usize) {
        let nachher = self.stand.clone().unwrap();
        self.undo.push(Schritt { vorher, nachher, aendern });
        self.redo.clear();
        self.aenderungen += 1;
        self.halte = None;
    }

    fn stand(&mut self) -> Result<Stand> {
        match &self.stand {
            Some(s) => Ok(s.clone()),
            None => bail!("Listen nicht geladen"),
        }
    }

    /// neuer Einsetzpunkt an `pos` (Richtung `rot`) -> Meldung
    pub fn punkt_neu(&mut self, v: &mut Viewer, ae: &mut Aendern, pos: DVec3, rot: f64, name: &str) -> Result<String> {
        let vorher = self.stand()?;
        let (id, kachel) = ae.objekt_anlegen(v, EINSETZPUNKT, pos.truncate(), rot, &[])?;
        self.stand.as_mut().unwrap().punkte.push(Einsetzpunkt { name: name.to_string(), id, pos, rot, kachel });
        self.schritt(vorher, 1);
        Ok(format!("Einsetzpunkt \"{name}\" angelegt (Objekt {id})"))
    }

    pub fn punkt_umbenennen(&mut self, i: usize, name: &str) -> Result<()> {
        let vorher = self.stand()?;
        let p = self.stand.as_mut().unwrap().punkte.get_mut(i).ok_or_else(|| anyhow::anyhow!("kein Einsetzpunkt {i}"))?;
        p.name = name.to_string();
        self.schritt(vorher, 0);
        Ok(())
    }

    pub fn punkt_loeschen(&mut self, v: &mut Viewer, ae: &mut Aendern, i: usize) -> Result<String> {
        let vorher = self.stand()?;
        let p = vorher.punkte.get(i).cloned().ok_or_else(|| anyhow::anyhow!("kein Einsetzpunkt {i}"))?;
        // dasselbe Objekt kann mehrere Eintraege haben: das Objekt nur weg, wenn kein anderer es nutzt
        let geteilt = vorher.punkte.iter().enumerate().any(|(j, x)| j != i && x.id == p.id);
        let n = if geteilt { 0 } else { ae.objekt_entfernen(v, p.kachel, p.id).map(|_| 1).unwrap_or(0) };
        self.stand.as_mut().unwrap().punkte.remove(i);
        self.schritt(vorher, n);
        Ok(format!("Einsetzpunkt \"{}\" geloescht", p.name))
    }

    /// neue Haltestelle an `pos` -> Meldung
    pub fn halt_neu(&mut self, v: &mut Viewer, ae: &mut Aendern, pos: DVec3, rot: f64, name: &str) -> Result<String> {
        let vorher = self.stand()?;
        let texte: Vec<String> = [name, "0", "0", "10", "", "", ""].iter().map(|x| x.to_string()).collect();
        let (id, _) = ae.objekt_anlegen(v, HALTESTELLE, pos.truncate(), rot, &texte)?;
        self.stand.as_mut().unwrap().busstops.push(Busstop { name: name.to_string(), id, versatz: "0.0000000000".into() });
        self.schritt(vorher, 1);
        Ok(format!("Haltestelle \"{name}\" angelegt (Objekt {id}, in Busstops.cfg)"))
    }

    pub fn halt_umbenennen(&mut self, v: &mut Viewer, ae: &mut Aendern, h: &Haltestelle, name: &str) -> Result<String> {
        let vorher = self.stand()?;
        ae.objekt_text(v, h.kachel, h.id, name)?;
        for b in self.stand.as_mut().unwrap().busstops.iter_mut().filter(|b| b.id == h.id) {
            b.name = name.to_string();
        }
        self.schritt(vorher, 1);
        Ok(format!("Haltestelle umbenannt: \"{name}\""))
    }

    pub fn halt_loeschen(&mut self, v: &mut Viewer, ae: &mut Aendern, h: &Haltestelle) -> Result<String> {
        let vorher = self.stand()?;
        ae.objekt_entfernen(v, h.kachel, h.id)?;
        self.stand.as_mut().unwrap().busstops.retain(|b| b.id != h.id);
        self.schritt(vorher, 1);
        Ok(format!("Haltestelle \"{}\" geloescht", h.name))
    }

    /// Haltestelle in Busstops.cfg aufnehmen (fuer die Fahrplaene)
    pub fn halt_aufnehmen(&mut self, h: &Haltestelle) -> Result<()> {
        let vorher = self.stand()?;
        self.stand.as_mut().unwrap().busstops.push(Busstop { name: h.name.clone(), id: h.id, versatz: "0.0000000000".into() });
        self.schritt(vorher, 0);
        Ok(())
    }

    pub fn rueckgaengig(&mut self, v: &mut Viewer, ae: Option<&mut Aendern>) -> Result<bool> {
        let Some(s) = self.undo.pop() else { return Ok(false) };
        if let Some(ae) = ae {
            for _ in 0..s.aendern {
                ae.rueckgaengig(v)?;
            }
        }
        self.stand = Some(s.vorher.clone());
        self.redo.push(s);
        self.aenderungen += 1;
        self.halte = None;
        Ok(true)
    }

    pub fn wiederholen(&mut self, v: &mut Viewer, ae: Option<&mut Aendern>) -> Result<bool> {
        let Some(s) = self.redo.pop() else { return Ok(false) };
        if let Some(ae) = ae {
            for _ in 0..s.aendern {
                ae.wiederholen(v)?;
            }
        }
        self.stand = Some(s.nachher.clone());
        self.undo.push(s);
        self.aenderungen += 1;
        self.halte = None;
        Ok(true)
    }

    /// fuers Speichern (nur wenn etwas geaendert wurde): Lagen der Einsetzpunkte, wie sie jetzt stehen
    pub fn daten(&self, v: &Viewer) -> Option<Daten> {
        if self.aenderungen == 0 {
            return None;
        }
        let s = self.stand.as_ref()?;
        let jetzt: HashMap<i64, (DVec3, f64)> = v.hidden_objects().into_iter().map(|x| (x.0, (x.1, x.2))).collect();
        Some(Daten {
            punkte: s.punkte.iter().map(|p| {
                let (pos, rot) = jetzt.get(&p.id).copied().unwrap_or((p.pos, p.rot));
                (p.name.clone(), p.id, pos, rot)
            }).collect(),
            busstops: s.busstops.clone(),
        })
    }
}

/// naechste Fahrspur (Strasse) bei p, bis 8 m: Punkt und Richtung - Einsetzpunkte und Haltestellen stehen auf der Spur
pub fn spur_bei(v: &Viewer, p: DVec2) -> Option<(DVec3, f64)> {
    let mut best: Option<(f64, DVec3, f64)> = None;
    for l in &v.lanes.lanes {
        if l.kind != LaneKind::Street || !l.name.to_ascii_lowercase().ends_with(".sli") || l.no_cars {
            continue;
        }
        // naechster Punkt auf den Strecken zwischen den Spurpunkten (gerade Spuren haben nur zwei)
        for i in 0..l.points.len().saturating_sub(1) {
            let (a, b) = (l.points[i], l.points[i + 1]);
            let ab = (b - a).truncate();
            let t = if ab.length_squared() < 1e-9 { 0.0 } else { ((p - a.truncate()).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) };
            let q = a + (b - a) * t;
            let d = (q.truncate() - p).length();
            if d < 8.0 && best.map(|x| d < x.0).unwrap_or(true) {
                let (ha, hb) = (l.headings.get(i).copied().unwrap_or(0.0) as f64, l.headings.get(i + 1).copied().unwrap_or(0.0) as f64);
                best = Some((d, q, ha + crate::netz::norm180(hb - ha) * t));
            }
        }
    }
    best.map(|b| (b.1, b.2.rem_euclid(360.0)))
}

/// Einsetzpunkte und Busstops.cfg der Karte im Ordner `karte` schreiben: Index, Lage und Kachelnummer aus den
/// fertigen Kacheln (Kachelliste `liste`, in [map]-Reihenfolge)
pub fn schreiben(karte: &Path, daten: &Daten, liste: &Liste) -> Result<()> {
    let ts = omsi_map::tile_size();
    // Objekt-ID -> (Kachelnummer, Index in der Kachel)
    let mut wo: HashMap<i64, (usize, usize)> = HashMap::new();
    for (nr, (_, _, datei)) in liste.iter().enumerate() {
        let Ok(b) = std::fs::read(karte.join(datei)) else { continue };
        let (t, _) = crate::speichern::dekodieren(&b);
        let z: Vec<&str> = t.lines().collect();
        let mut index = 0;
        for i in 0..z.len() {
            let w = z[i].trim().to_ascii_lowercase();
            if ["[object]", "[attachobj]", "[splineattachement]", "[splineattachement_repeater]"].contains(&w.as_str()) {
                if let Some(id) = z.get(i + 3).and_then(|x| x.trim().parse::<i64>().ok()) {
                    wo.entry(id).or_insert((nr, index));
                }
                index += 1;
            }
        }
    }
    let f = |x: f64| format!("{x:.3}");
    let mut saetze: Vec<String> = Vec::new();
    let mut n = 0;
    for (name, id, pos, rot) in &daten.punkte {
        let Some((nr, index)) = wo.get(id) else {
            log::warn!("Einsetzpunkt {name}: Objekt {id} in keiner Kachel - faellt weg");
            continue;
        };
        let (tx, ty, _) = &liste[*nr];
        let h = ((rot + 180.0).rem_euclid(360.0) - 180.0).to_radians();
        saetze.extend([index.to_string(), id.to_string(), "0".into(), f(pos.x - *tx as f64 * ts), f(pos.z), f(pos.y - *ty as f64 * ts),
                       "0.000".into(), f((h / 2.0).sin()), "0.000".into(), f((h / 2.0).cos()), nr.to_string(), name.clone()]);
        n += 1;
    }
    // global.cfg: [entrypoints] ersetzen (oder vor die erste [map] setzen)
    let global = karte.join("global.cfg");
    let (text, utf16) = crate::speichern::dekodieren(&std::fs::read(&global)?);
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let z: Vec<&str> = text.split(eol).collect();
    let mut out: Vec<String> = Vec::new();
    let mut gesetzt = false;
    let mut i = 0;
    let block = |out: &mut Vec<String>| {
        out.push("[entrypoints]".into());
        out.push(n.to_string());
        out.extend(saetze.iter().cloned());
        out.push(String::new());
    };
    while i < z.len() {
        let w = z[i].trim().to_ascii_lowercase();
        if w == "[entrypoints]" {
            let alt: usize = z.get(i + 1).and_then(|x| x.trim().parse().ok()).unwrap_or(0);
            i += 2 + alt * 12;
            while i < z.len() && z[i].trim().is_empty() {
                i += 1;
            }
            if !gesetzt {
                block(&mut out);
                gesetzt = true;
            }
            continue;
        }
        if w == "[map]" && !gesetzt {
            block(&mut out);
            gesetzt = true;
        }
        out.push(z[i].to_string());
        i += 1;
    }
    if !gesetzt {
        block(&mut out);
    }
    std::fs::write(&global, crate::speichern::kodieren(&out.join(eol), utf16))?;
    // TTData/Busstops.cfg: Kopf behalten, Eintraege neu
    let tt = karte.join("TTData");
    let bs = tt.join("Busstops.cfg");
    let (kopf, utf16, eol) = match std::fs::read(&bs) {
        Ok(b) => {
            let (t, u) = crate::speichern::dekodieren(&b);
            let eol = if t.contains("\r\n") { "\r\n" } else { "\n" };
            let k: Vec<String> = t.split(eol).take_while(|l| !l.trim().eq_ignore_ascii_case("[busstop]")).map(|l| l.to_string()).collect();
            (k, u, eol)
        }
        Err(_) => (["---------------------------", "Time Table BusStopList File", "---------------------------", "", "Created with omsi-editor", ""]
                       .iter().map(|x| x.to_string()).collect(), false, "\r\n"),
    };
    let mut out = kopf;
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    out.push(String::new());
    for b in &daten.busstops {
        let Some((nr, _)) = wo.get(&b.id) else { continue };
        out.extend(["[busstop]".to_string(), b.name.clone(), nr.to_string(), b.id.to_string(), b.versatz.clone(), "0".into(), "0".into(), String::new(), String::new()]);
    }
    if !daten.busstops.is_empty() || bs.is_file() {
        std::fs::create_dir_all(&tt)?;
        std::fs::write(&bs, crate::speichern::kodieren(&out.join(eol), utf16))?;
    }
    log::info!("{n} Einsetzpunkte, {} Haltestellen in Busstops.cfg geschrieben", daten.busstops.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kopieren(a: &Path, b: &Path) {
        std::fs::create_dir_all(b).unwrap();
        for e in std::fs::read_dir(a).unwrap().flatten() {
            if e.file_type().unwrap().is_dir() { kopieren(&e.path(), &b.join(e.file_name())) } else { std::fs::copy(e.path(), b.join(e.file_name())).unwrap(); }
        }
    }

    /// Grundorf: Einsetzpunkt und Haltestelle auf einer Fahrspur anlegen, Haltestelle umbenennen, einen vorhandenen
    /// Einsetzpunkt loeschen, rueckgaengig/wiederholen; als neue Karte speichern - global.cfg und Busstops.cfg zeigen
    /// auf die richtigen Objekte (Kachelnummer, Index in der Kachel)
    #[test]
    #[ignore]
    fn einsetzpunkte_und_haltestellen() {
        let _sperre = crate::bearbeiten::tests::sperre();
        let root = Path::new(crate::bearbeiten::tests::OMSI);
        let mut v = crate::bearbeiten::tests::grundorf();
        v.tiles_around(DVec3::new(150.0, 150.0, 0.0), 8).unwrap();
        let mut a = Aendern::neu(&v);
        let mut o = Orte::default();
        o.laden(&root.join("maps/Grundorf"), &v.map_tile_refs());
        let halte = o.haltestellen(&v);
        println!("{} Einsetzpunkte, {} Haltestellen, {} in Busstops.cfg", o.punkte().len(), halte.len(), o.busstops().len());
        assert_eq!(o.punkte().len(), 7);
        assert!(halte.len() >= 5 && halte.iter().any(|h| h.name == "Elsterplatz"));
        // Fahrspur nahe der Kartenmitte
        let (pos, rot) = (0..40).find_map(|i| spur_bei(&v, DVec2::new(150.0 + i as f64 * 7.0, 150.0))).expect("Fahrspur");
        println!("{}", o.punkt_neu(&mut v, &mut a, pos, rot, "Test Start").unwrap());
        let pos2 = pos + (crate::netz::dir(rot) * 40.0).extend(0.0);
        println!("{}", o.halt_neu(&mut v, &mut a, pos2, rot, "Teststelle").unwrap());
        let neu_halt = o.haltestellen(&v).into_iter().find(|h| h.name == "Teststelle").expect("neue Haltestelle in der Kachel");
        println!("{}", o.halt_umbenennen(&mut v, &mut a, &neu_halt, "Teststelle Nord").unwrap());
        assert!(o.haltestellen(&v).iter().any(|h| h.name == "Teststelle Nord" && h.id == neu_halt.id));
        println!("{}", o.punkt_loeschen(&mut v, &mut a, 0).unwrap());
        assert_eq!(o.punkte().len(), 7);
        // rueckgaengig nimmt Liste und Kachel zusammen zurueck
        let n_aendern = a.undo_len();
        assert!(o.rueckgaengig(&mut v, Some(&mut a)).unwrap());
        assert_eq!(o.punkte().len(), 8);
        assert!(o.wiederholen(&mut v, Some(&mut a)).unwrap());
        assert_eq!(a.undo_len(), n_aendern);
        // speichern als neue Karte
        let test_root = std::env::temp_dir().join(format!("omsi-editor-orte-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&test_root);
        kopieren(&root.join("maps/Grundorf"), &test_root.join("maps/Grundorf"));
        let mut paket = crate::speichern::vorbereiten(&v, &crate::bearbeiten::Bearbeiten::neu(crate::bearbeiten::Werkzeug::Welt),
                                                      &crate::netz::Netz::default(), &[], &a.kopien("Grundorf"), None, "Grundorf").unwrap();
        paket.orte = o.daten(&v).map(|d| (d, v.map_tile_refs()));
        paket.naechste_id = Some(v.next_object_id());
        let ziel = crate::speichern::karte_anlegen(&test_root, "Grundorf", "Grundorf_orte", &paket).unwrap();
        // global.cfg: jeder Einsetzpunkt zeigt mit Kachelnummer und Index auf sein Objekt (ein Einsetzpunkt-Objekt)
        let g = omsi_map::GlobalCfg::load(&ziel.join("global.cfg")).unwrap();
        let text = crate::speichern::dekodieren(&std::fs::read(ziel.join("global.cfg")).unwrap()).0;
        let z: Vec<&str> = text.lines().collect();
        let i = z.iter().position(|l| l.trim() == "[entrypoints]").unwrap();
        let n: usize = z[i + 1].trim().parse().unwrap();
        assert_eq!(n, 7);
        let mut neu_gefunden = false;
        for k in 0..n {
            let r = &z[i + 2 + k * 12..i + 14 + k * 12];
            let (index, id, nr): (usize, i64, usize) = (r[0].trim().parse().unwrap(), r[1].trim().parse().unwrap(), r[10].trim().parse().unwrap());
            let t = g.tiles.iter().find(|t| t.index == nr).unwrap();
            let kachel = crate::speichern::dekodieren(&std::fs::read(ziel.join(&t.file)).unwrap()).0;
            let kz: Vec<&str> = kachel.lines().collect();
            let eintraege: Vec<usize> = (0..kz.len()).filter(|&j| ["[object]", "[attachobj]", "[splineattachement]", "[splineattachement_repeater]"].contains(&kz[j].trim().to_ascii_lowercase().as_str())).collect();
            let j = eintraege[index];
            assert_eq!(kz[j + 3].trim().parse::<i64>().unwrap(), id, "Einsetzpunkt {}: Index {index} zeigt auf ein anderes Objekt", r[11]);
            assert!(kz[j + 2].to_ascii_lowercase().contains("entrypoint"), "{}", kz[j + 2]);
            neu_gefunden |= r[11].trim() == "Test Start";
        }
        assert!(neu_gefunden, "neuer Einsetzpunkt fehlt");
        // Busstops.cfg: die neue Haltestelle mit ihrem Namen und ihrer Kachel
        let bs = crate::speichern::dekodieren(&std::fs::read(ziel.join("TTData/Busstops.cfg")).unwrap()).0;
        let bz: Vec<&str> = bs.lines().collect();
        let j = (0..bz.len()).find(|&j| bz[j].trim() == "[busstop]" && bz[j + 3].trim() == neu_halt.id.to_string()).expect("neue Haltestelle in Busstops.cfg");
        assert_eq!(bz[j + 1].trim(), "Teststelle Nord");
        let nr: usize = bz[j + 2].trim().parse().unwrap();
        let t = g.tiles.iter().find(|t| t.index == nr).unwrap();
        assert_eq!((t.x, t.y), neu_halt.kachel);
        assert!(bz.iter().filter(|l| l.trim() == "[busstop]").count() >= 14);
        std::fs::remove_dir_all(&test_root).ok();
        drop(a);
    }
}
