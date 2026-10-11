//! Kreuzung Ring/Ludwigsallee in Abschnitte: den Plan von omsigen.ring_abschnitte in der Sitzung anwenden. Jeder
//! Eintrag der Kreuzung bekommt den Pfad seiner Abschnitts-Datei (`RA_<Kurz>__...`); ein Objekt ueber mehrere
//! Abschnitte wird zu mehreren Eintraegen (gleiche Lage, je Abschnitt ein Teil). Danach laesst sich jeder Abschnitt
//! einzeln neu bauen, ohne dass sich die anderen veraendern. Ein Rueckgaengig-Schritt; Speichern schreibt die Karte.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};

pub struct Abschnitt {
    pub kurz: String,
    pub name: String,
    pub inhalt: String,
    pub anzahl: usize,
    /// Mitte der Eintraege (Hinsehen)
    pub mitte: glam::DVec2,
}

/// Abschnitte und Anzahl der Eintraege aus dem Plan
pub fn lesen(plan: &Path) -> Result<Vec<Abschnitt>> {
    let j: serde_json::Value = serde_json::from_slice(&std::fs::read(plan)?)?;
    let mut out: Vec<Abschnitt> = j["abschnitte"].as_array().context("Plan ohne Abschnitte")?.iter().map(|a| Abschnitt {
        kurz: a["kurz"].as_str().unwrap_or("").into(), name: a["name"].as_str().unwrap_or("").into(),
        inhalt: a["inhalt"].as_str().unwrap_or("").into(), anzahl: 0, mitte: glam::DVec2::ZERO,
    }).collect();
    for e in j["eintraege"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
        let p = glam::DVec2::new(e["x"].as_f64().unwrap_or(0.0), e["y"].as_f64().unwrap_or(0.0));
        for k in e["abschnitte"].as_array().into_iter().flatten().filter_map(|k| k.as_str()) {
            if let Some(a) = out.iter_mut().find(|a| a.kurz == k) {
                a.mitte = (a.mitte * a.anzahl as f64 + p) / (a.anzahl + 1) as f64;
                a.anzahl += 1;
            }
        }
    }
    Ok(out)
}

/// Plan anwenden -> Meldung
pub fn anwenden(ae: &mut crate::aendern::Aendern, v: &mut openomsi_game::viewer::Viewer, plan: &Path) -> Result<String> {
    let (m, n) = anwenden_zaehlen(ae, v, plan)?;
    anyhow::ensure!(n > 0, "{m}");
    Ok(m)
}

/// Plan anwenden -> (Meldung, Anzahl umgestellter Eintraege)
pub fn anwenden_zaehlen(ae: &mut crate::aendern::Aendern, v: &mut openomsi_game::viewer::Viewer, plan: &Path) -> Result<(String, usize)> {
    let j: serde_json::Value = serde_json::from_slice(&std::fs::read(plan)?)?;
    // je Kachel: (alter Pfad, Lage auf 1 cm) -> neue Pfade; die IDs taugen nicht (Speichern/Neu-Einsetzen nummeriert neu)
    let mut je: HashMap<(i32, i32), HashMap<(String, i64, i64), Vec<String>>> = HashMap::new();
    let cm = |v: f64| (v * 100.0).round() as i64;
    let mut extra = 0i64;
    for e in j["eintraege"].as_array().context("Plan ohne Eintraege")? {
        let k = (e["kachel"][0].as_i64().unwrap_or(0) as i32, e["kachel"][1].as_i64().unwrap_or(0) as i32);
        let neu: Vec<String> = e["neu"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(|s| s.to_string())).collect();
        if neu.is_empty() {
            continue;
        }
        extra += neu.len() as i64 - 1;
        je.entry(k).or_default().insert((e["pfad"].as_str().unwrap_or("").to_string(), cm(e["ox"].as_f64().unwrap_or(f64::NAN)), cm(e["oy"].as_f64().unwrap_or(f64::NAN))), neu);
    }
    let mut naechste = ae.neue_ids(v, extra.max(1));
    let kacheln: Vec<(i32, i32)> = je.keys().copied().collect();
    let (mut n_um, mut n_neu, mut n_fehlt) = (0usize, 0usize, 0usize);
    ae.kacheln_aendern(v, &kacheln, |k, zeilen| {
        let Some(tab) = je.get(&k) else { return Ok(0) };
        let starts: Vec<usize> = (0..zeilen.len()).filter(|&i| crate::kreuzung::ist_eintrag(&zeilen[i])).collect();
        let mut dazu: Vec<Vec<String>> = Vec::new();
        let mut gefunden = 0;
        for (n, &i) in starts.iter().enumerate() {
            let ende = starts.get(n + 1).copied().unwrap_or(zeilen.len());
            if i + 3 >= zeilen.len() {
                continue;
            }
            let w = zeilen[i].trim().to_ascii_lowercase();
            let (ix, iy) = if w == "[object]" { (i + 4, i + 5) } else { (i + 6, i + 8) };
            let lage = |q: usize| zeilen.get(q).and_then(|x| x.trim().parse::<f64>().ok());
            let (Some(x), Some(y)) = (lage(ix), lage(iy)) else { continue };
            let pfad = zeilen[i + 2].trim().to_string();
            let Some(neu) = tab.get(&(pfad, cm(x), cm(y))) else { continue };
            gefunden += 1;
            for p in &neu[1..] {
                let mut b: Vec<String> = zeilen[i..ende].to_vec();
                b[2] = p.clone();
                b[3] = naechste.to_string();
                naechste += 1;
                // Leerzeilen am Ende des Blocks nicht doppeln
                while b.last().is_some_and(|l| l.trim().is_empty()) {
                    b.pop();
                }
                dazu.push(b);
            }
            zeilen[i + 2] = neu[0].clone();
        }
        n_um += gefunden;
        n_fehlt += tab.len() - gefunden;
        n_neu += dazu.len();
        for b in dazu {
            zeilen.push(String::new());
            zeilen.extend(b);
        }
        Ok(gefunden)
    })?;
    let mut m = format!("Kreuzung in Abschnitte geteilt: {n_um} Eintraege umgestellt, {n_neu} Teilobjekte dazu");
    if n_fehlt > 0 {
        m += &format!(", {n_fehlt} Eintraege des Plans nicht in der Karte gefunden (Kreuzung in der Sitzung veraendert? erst speichern, dann aufteilen)");
    }
    Ok((m, n_um))
}

/// gibt es in den Kacheln der Sitzung Eintraege mit dieser Abschnitts-Kennung?
pub fn hat_marker(v: &openomsi_game::viewer::Viewer, marker: &str) -> bool {
    v.map_tiles().into_iter().filter_map(|(x, y)| v.tile_file(x, y)).any(|p| {
        std::fs::read(&p).map(|b| {
            String::from_utf16_lossy(&b.get(2..).unwrap_or(&[]).chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>()).contains(marker)
        }).unwrap_or(false)
    })
}

/// Abschnitts-Kennung eines Sonderbau-JSON (abschnitt_<X>.json von omsigen.ring_abschnitte), sonst None
pub fn marker(json: &Path) -> Option<String> {
    let j: serde_json::Value = serde_json::from_slice(&std::fs::read(json).ok()?).ok()?;
    j["marker"].as_str().map(|s| s.to_string())
}

/// ist die Kreuzung der Karte (Sitzung) schon in Abschnitte geteilt? (Eintraege mit `RA_<Kurz>__` in einer Kachel)
pub fn aufgeteilt(v: &openomsi_game::viewer::Viewer) -> bool {
    v.map_tiles().into_iter().filter_map(|(x, y)| v.tile_file(x, y)).any(|p| {
        std::fs::read(&p).map(|b| {
            let t = String::from_utf16_lossy(&b.get(2..).unwrap_or(&[]).chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>());
            t.contains("\\RA_") && t.contains("__")
        }).unwrap_or(false)
    })
}
