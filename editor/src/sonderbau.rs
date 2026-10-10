//! Sonderbauten: Einzelstuecke fuer eine bestimmte Stelle, von omsigen erzeugt (zuerst `omsigen.sonderbau_ring`:
//! Dr.-Willi-Reiland-Ring / Ludwigsallee). Die Objekte liegen mit ihrer Lage in `sonderbau_<name>.json` im Ordner der
//! Objekte; hier werden sie gelesen und in die Karte gesetzt.
#![allow(dead_code)]

use anyhow::{Context, Result};
use glam::DVec2;
use std::path::Path;

/// ein Objekt des Sonderbaus: .sco relativ zum OMSI-Ordner, Ursprung in Kartenkoordinaten, Hoehe absolut ([absheight])
#[derive(Clone, Debug)]
pub struct Teil {
    pub name: String,
    pub rel: String,
    pub pos: DVec2,
    pub hoehe: f64,
}

pub fn lesen(datei: &Path) -> Result<Vec<Teil>> {
    let j: serde_json::Value = serde_json::from_slice(&std::fs::read(datei).with_context(|| datei.display().to_string())?)?;
    let objekte = j["objekte"].as_array().context("objekte fehlen")?;
    Ok(objekte
        .iter()
        .filter_map(|o| {
            Some(Teil {
                name: o["name"].as_str()?.to_string(),
                rel: o["rel"].as_str()?.to_string(),
                pos: DVec2::new(o["x"].as_f64()?, o["y"].as_f64()?),
                hoehe: o["z"].as_f64()?,
            })
        })
        .collect())
}

/// ein [spline_h]-Element (Kartenkoordinaten, Hoehe absolut)
#[derive(Clone, Debug)]
pub struct Element {
    pub pos: DVec2,
    pub z: f64,
    pub richtung: f64,
    pub laenge: f64,
    pub radius: f64,
    pub stg_a: f64,
    pub stg_e: f64,
    pub dh: f64,
    pub kum: f64,
}

/// Strasse des Sonderbaus: Querschnitt (Baukasten) oder fertige .sli (Waende, von omsigen geschrieben) und Elemente
/// in Laufrichtung des Splines
#[derive(Clone, Debug)]
pub struct Strasse {
    pub name: String,
    pub querschnitt: Option<crate::querschnitt::Querschnitt>,
    /// relativer Pfad einer fertigen .sli (statt Querschnitt)
    pub sli: Option<String>,
    pub elemente: Vec<Element>,
}

impl Strasse {
    /// .sli dieser Strasse (aus dem Querschnitt erst schreiben)
    fn sli_datei(&self, root: &Path) -> Result<String> {
        match (&self.sli, &self.querschnitt) {
            (Some(s), _) => Ok(s.clone()),
            (None, Some(q)) => crate::querschnitt::schreiben(root, q, &q.datei()),
            (None, None) => anyhow::bail!("{}: weder Querschnitt noch .sli", self.name),
        }
    }
}

pub fn strassen_lesen(datei: &Path) -> Result<Vec<Strasse>> {
    let j: serde_json::Value = serde_json::from_slice(&std::fs::read(datei)?)?;
    let mut out = Vec::new();
    for s in j["strassen"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
        let sli = s["sli"].as_str().map(|x| x.to_string());
        let q = if sli.is_some() {
            None
        } else {
            Some(crate::querschnitt::Querschnitt::aus_json(&s["querschnitt"]).with_context(|| format!("Querschnitt von {}", s["name"]))?)
        };
        let w = |e: &serde_json::Value, k: &str| e[k].as_f64().unwrap_or(0.0);
        let elemente = s["elemente"].as_array().map(|a| a.as_slice()).unwrap_or(&[]).iter().map(|e| Element {
            pos: DVec2::new(w(e, "x"), w(e, "y")), z: w(e, "z"), richtung: w(e, "richtung"), laenge: w(e, "laenge"),
            radius: w(e, "radius"), stg_a: w(e, "stg_a"), stg_e: w(e, "stg_e"), dh: w(e, "dh"), kum: w(e, "kum"),
        }).collect();
        out.push(Strasse { name: s["name"].as_str().unwrap_or("").to_string(), querschnitt: q, sli, elemente });
    }
    Ok(out)
}

/// Strassen in die Karte (Sitzung) schreiben: .sli aus dem Querschnitt (eigener Spline-Ordner), je Element ein
/// [spline_h] (verkettet ueber Vorgaenger/Nachfolger) -> Anzahl Splines; ein Rueckgaengig-Schritt
pub fn strassen_anlegen(ae: &mut crate::aendern::Aendern, v: &mut openomsi_game::viewer::Viewer, root: &Path, strassen: &[Strasse]) -> Result<usize> {
    let ts = omsi_map::tile_size();
    let z = crate::kreuzung::zahl;
    let mut je_kachel: std::collections::BTreeMap<(i32, i32), Vec<Vec<String>>> = Default::default();
    let mut n = 0;
    for st in strassen {
        if st.elemente.is_empty() {
            continue;
        }
        let sli = st.sli_datei(root)?;
        let id0 = ae.neue_ids(v, st.elemente.len() as i64);
        for (i, e) in st.elemente.iter().enumerate() {
            let k = ((e.pos.x / ts).floor() as i32, (e.pos.y / ts).floor() as i32);
            let id = id0 + i as i64;
            je_kachel.entry(k).or_default().push(vec![
                "[spline_h]".to_string(), "0".into(), sli.clone(), id.to_string(),
                if i > 0 { (id - 1).to_string() } else { "0".into() },
                if i + 1 < st.elemente.len() { (id + 1).to_string() } else { "0".into() },
                z(e.pos.x - k.0 as f64 * ts), z(e.z), z(e.pos.y - k.1 as f64 * ts), z(e.richtung.rem_euclid(360.0)), z(e.laenge),
                z(e.radius), z(e.stg_a), z(e.stg_e), z(e.dh), "0".into(), "0".into(), "0".into(), "0".into(), z(e.kum),
            ]);
            n += 1;
        }
    }
    let kacheln: Vec<(i32, i32)> = je_kachel.keys().copied().collect();
    ae.kacheln_aendern(v, &kacheln, |k, zeilen| {
        let liste = je_kachel.get(&k).cloned().unwrap_or_default();
        for f in &liste {
            zeilen.push(String::new());
            zeilen.extend(f.iter().cloned());
        }
        Ok(liste.len())
    })?;
    Ok(n)
}

/// Kreuzung des Sonderbaus: fertiges Objekt von omsigen (ring_kreuzung) mit Vorfahrt-Regeln und Ampeln; die Signale
/// tragen die Hoehe ihres Standorts (Gehweg/Insel, absolut), ihre Hoehe ueber dem Gelaende ergibt sich beim Setzen
#[derive(Clone, Debug)]
pub struct KreuzungPlan {
    pub name: String,
    pub gesetzt: crate::kreuzung::Gesetzt,
    /// je Signal: Hoehe des Bodens, auf dem es steht (absolut), None bei Signalen am Ausleger
    pub boden: Vec<Option<f64>>,
    pub phasen: Vec<usize>,
    pub umlauf: Option<f64>,
}

pub fn kreuzungen_lesen(datei: &Path) -> Result<Vec<KreuzungPlan>> {
    let j: serde_json::Value = serde_json::from_slice(&std::fs::read(datei)?)?;
    let mut out = Vec::new();
    for k in j["kreuzungen"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
        let f = |v: &serde_json::Value| v.as_f64().unwrap_or(0.0);
        let liste = k["signale"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);
        let signale = liste.iter().map(|g| crate::kreuzung::Signal {
            art: g["art"].as_str().unwrap_or("").to_string(),
            datei: g["datei"].as_str().unwrap_or("").to_string(),
            pos: g["x"].as_f64().zip(g["y"].as_f64()).map(|(x, y)| DVec2::new(x, y)),
            hoehe: f(&g["hoehe"]),
            rot: f(&g["rot"]),
            gruppe: g["gruppe"].as_i64(),
            eltern: g["eltern"].as_u64().map(|x| x as usize),
            anhang: g["anhang"].as_i64().unwrap_or(0),
        }).collect();
        let gesetzt = crate::kreuzung::Gesetzt {
            rel: k["rel"].as_str().context("Kreuzung ohne Datei")?.to_string(),
            pos: glam::DVec3::new(f(&k["ursprung"][0]), f(&k["ursprung"][1]), 0.0),
            rules: k["rules"].as_array().map(|l| l.iter().filter_map(|r| Some((r[0].as_u64()? as usize, r[1].as_i64()? as i32))).collect()).unwrap_or_default(),
            no_cars: k["no_cars"].as_array().map(|l| l.iter().filter_map(|x| x.as_u64().map(|x| x as usize)).collect()).unwrap_or_default(),
            signale,
        };
        out.push(KreuzungPlan {
            name: k["name"].as_str().unwrap_or("Kreuzung").to_string(),
            gesetzt,
            boden: liste.iter().map(|g| g["z_boden"].as_f64()).collect(),
            phasen: k["phasen"].as_array().map(|l| l.iter().filter_map(|x| x.as_u64().map(|x| x as usize)).collect()).unwrap_or_default(),
            umlauf: k["umlauf"].as_f64(),
        });
    }
    Ok(out)
}

/// Kreuzungsobjekt mit Regeln und Ampeln in die Karte (Sitzung) setzen; die Ampeln stehen auf dem Boden ihres
/// Standorts (das Gelaende darunter kann abgesenkt sein) -> Anzahl Signale/Masten
pub fn kreuzung_anlegen(ae: &mut crate::aendern::Aendern, v: &mut openomsi_game::viewer::Viewer, plan: &KreuzungPlan) -> Result<usize> {
    let mut g = plan.gesetzt.clone();
    for (sg, boden) in g.signale.iter_mut().zip(&plan.boden) {
        if let (Some(p), Some(b)) = (sg.pos, boden) {
            let gelaende = v.terrain_height(p.x, p.y).unwrap_or(*b);
            sg.hoehe += b - gelaende;
        }
    }
    let mut id = ae.neue_ids(v, 1 + g.signale.len() as i64);
    let (k, eintraege) = crate::speichern::kreuzung_eintraege(&g, &mut id);
    ae.kacheln_aendern(v, &[k], |_, zeilen| {
        for f in &eintraege {
            zeilen.push(String::new());
            zeilen.extend(f.iter().cloned());
        }
        Ok(eintraege.len())
    })?;
    Ok(g.signale.len())
}

/// Eintraege je Kachel (fuer `einsetzen`)
type JeKachel = std::collections::BTreeMap<(i32, i32), Vec<Vec<String>>>;

fn kachel_von(p: DVec2) -> (i32, i32) {
    let ts = omsi_map::tile_size();
    ((p.x / ts).floor() as i32, (p.y / ts).floor() as i32)
}

/// den ganzen Sonderbau in die Karte (Sitzung) setzen: Gelaende absenken (ein Rueckgaengig-Schritt im Gelaende),
/// alles an der Stelle entfernen (Strassen, Objekte - auch einen frueher eingesetzten Sonderbau) und Bauwerk, Strassen
/// und Kreuzung samt Ampeln eintragen (ein Rueckgaengig-Schritt) -> Meldung. `gel_ordner`: Sitzungsordner der Karte
pub fn einsetzen(ae: &mut crate::aendern::Aendern, gel: &mut crate::gelaende::Gelaende, v: &mut openomsi_game::viewer::Viewer, root: &Path,
                 gel_ordner: &Path, json: &Path) -> Result<String> {
    let teile = lesen(json)?;
    let strassen = strassen_lesen(json)?;
    let kreuzungen = kreuzungen_lesen(json)?;
    let senken = gelaende_lesen(json)?;
    let zurueck = gelaende_lesen_feld(json, "gelaende_zurueck")?;
    // Bereich: alle Strassen des Sonderbaus, 8 m Rand
    let mut lo = DVec2::splat(f64::MAX);
    let mut hi = DVec2::splat(f64::MIN);
    for e in strassen.iter().flat_map(|s| s.elemente.iter()) {
        lo = lo.min(e.pos);
        hi = hi.max(e.pos);
    }
    if lo.x > hi.x {
        anyhow::bail!("der Sonderbau hat keine Strassen");
    }
    let (lo, hi) = (lo - DVec2::splat(8.0), hi + DVec2::splat(8.0));
    let im_bereich = |p: DVec2| p.x >= lo.x && p.x <= hi.x && p.y >= lo.y && p.y <= hi.y;
    v.tiles_around(((lo + hi) * 0.5).extend(0.0), 1)?;
    // 1. Gelaende (vor den Ampeln: ihre Hoehe ueber dem Gelaende haengt davon ab)
    let mut kacheln: std::collections::HashMap<(i32, i32), omsi_map::Terrain> = Default::default();
    // Punkte, die ein frueheres Einsetzen abgesenkt hat und die jetzt nicht mehr gebraucht werden: zurueck aufs DGM
    for (k, spalte, zeile, z) in &zurueck {
        if !kacheln.contains_key(k) {
            let Some(t) = v.tile_terrain(k.0, k.1) else { continue };
            kacheln.insert(*k, t);
        }
        let t = kacheln.get_mut(k).unwrap();
        let i = zeile * (t.cells + 1) + spalte;
        if i < t.heights.len() {
            t.heights[i] = t.heights[i].max(*z);
        }
    }
    for (k, spalte, zeile, z) in &senken {
        if !kacheln.contains_key(k) {
            let Some(t) = v.tile_terrain(k.0, k.1) else { continue };
            kacheln.insert(*k, t);
        }
        let t = kacheln.get_mut(k).unwrap();
        let i = zeile * (t.cells + 1) + spalte;
        if i < t.heights.len() {
            t.heights[i] = t.heights[i].min(*z);
        }
    }
    let neu: Vec<((i32, i32), Vec<f32>)> = kacheln.iter().map(|(k, t)| (*k, t.heights.clone())).collect();
    let n_gel = gel.uebernehmen(v, gel_ordner, neu)?;
    v.tiles_around(((lo + hi) * 0.5).extend(0.0), 1)?;
    // 2. was weg muss: Strassen mit einem Punkt im Bereich
    ae.aktualisieren(v);
    let weg_ids: std::collections::HashSet<i64> = ae.kacheln.values().flatten()
        .filter(|s| s.punkte().iter().any(|(q, _)| im_bereich(q.truncate()))).map(|s| s.id).collect();
    // 3. neue Eintraege
    let z = crate::kreuzung::zahl;
    let ts = omsi_map::tile_size();
    let mut je: JeKachel = Default::default();
    let mut n_obj = 0;
    for t in &teile {
        let k = kachel_von(t.pos);
        let id = ae.neue_ids(v, 1);
        je.entry(k).or_default().push(vec!["[object]".to_string(), "0".into(), t.rel.clone(), id.to_string(), z(t.pos.x - k.0 as f64 * ts),
                                            z(t.pos.y - k.1 as f64 * ts), z(t.hoehe), "0".into(), "0".into(), "0".into(), "0".into()]);
        n_obj += 1;
    }
    let mut n_spl = 0;
    for st in &strassen {
        if st.elemente.is_empty() {
            continue;
        }
        let sli = st.sli_datei(root)?;
        v.forget_spline_type(&sli);
        let id0 = ae.neue_ids(v, st.elemente.len() as i64);
        for (i, e) in st.elemente.iter().enumerate() {
            let k = kachel_von(e.pos);
            let id = id0 + i as i64;
            je.entry(k).or_default().push(vec![
                "[spline_h]".to_string(), "0".into(), sli.clone(), id.to_string(),
                if i > 0 { (id - 1).to_string() } else { "0".into() },
                if i + 1 < st.elemente.len() { (id + 1).to_string() } else { "0".into() },
                z(e.pos.x - k.0 as f64 * ts), z(e.z), z(e.pos.y - k.1 as f64 * ts), z(e.richtung.rem_euclid(360.0)), z(e.laenge),
                z(e.radius), z(e.stg_a), z(e.stg_e), z(e.dh), "0".into(), "0".into(), "0".into(), "0".into(), z(e.kum),
            ]);
            n_spl += 1;
        }
    }
    let mut n_sig = 0;
    for plan in &kreuzungen {
        let mut g = plan.gesetzt.clone();
        for (sg, boden) in g.signale.iter_mut().zip(&plan.boden) {
            if let (Some(p), Some(b)) = (sg.pos, boden) {
                sg.hoehe += b - v.terrain_height(p.x, p.y).unwrap_or(*b);
            }
        }
        let mut id = ae.neue_ids(v, 1 + g.signale.len() as i64);
        let (k, eintraege) = crate::speichern::kreuzung_eintraege(&g, &mut id);
        n_sig += g.signale.len();
        je.entry(k).or_default().extend(eintraege);
    }
    // 4. eine Aenderung ueber alle betroffenen Kacheln
    let mut betroffen: Vec<(i32, i32)> = je.keys().copied().collect();
    for k in v.loaded_tile_keys() {
        let (a, b) = (DVec2::new(k.0 as f64 * ts, k.1 as f64 * ts), DVec2::new((k.0 + 1) as f64 * ts, (k.1 + 1) as f64 * ts));
        if a.x <= hi.x && b.x >= lo.x && a.y <= hi.y && b.y >= lo.y && !betroffen.contains(&k) && v.tile_file(k.0, k.1).is_some() {
            betroffen.push(k);
        }
    }
    let mut entfernt = 0;
    ae.kacheln_aendern(v, &betroffen, |k, zeilen| {
        let o = DVec2::new(k.0 as f64 * ts, k.1 as f64 * ts);
        let f = |i: usize| zeilen.get(i).and_then(|x| x.trim().parse::<f64>().ok());
        // Eintraege als Bereiche [Anfang, Ende)
        let starts: Vec<usize> = (0..zeilen.len()).filter(|&i| crate::kreuzung::ist_eintrag(&zeilen[i])).collect();
        let mut weg: Vec<(usize, usize)> = Vec::new();
        let mut weg_obj: std::collections::HashSet<i64> = Default::default();
        for (n, &i) in starts.iter().enumerate() {
            let ende = starts.get(n + 1).copied().unwrap_or(zeilen.len());
            let w = zeilen[i].trim().to_ascii_lowercase();
            let id = f(i + 3).map(|x| x as i64);
            let raus = match w.as_str() {
                "[spline]" | "[spline_h]" => id.map(|x| weg_ids.contains(&x)).unwrap_or(false)
                    || f(i + 6).zip(f(i + 8)).map(|(x, y)| im_bereich(o + DVec2::new(x, y))).unwrap_or(false),
                "[object]" => f(i + 4).zip(f(i + 5)).map(|(x, y)| im_bereich(o + DVec2::new(x, y))).unwrap_or(false),
                "[attachobj]" => f(i + 4).map(|p| weg_obj.contains(&(p as i64))).unwrap_or(false),
                _ => false,
            };
            if raus {
                if let Some(id) = id {
                    weg_obj.insert(id);
                }
                weg.push((i, ende));
            }
        }
        for (a, b) in weg.iter().rev() {
            zeilen.drain(*a..*b);
        }
        let neu = je.get(&k).cloned().unwrap_or_default();
        for e in &neu {
            zeilen.push(String::new());
            zeilen.extend(e.iter().cloned());
        }
        entfernt += weg.len();
        Ok(weg.len() + neu.len())
    })?;
    Ok(format!("Sonderbau eingesetzt: {n_obj} Objekte, {n_spl} Splines, {} Kreuzung(en) mit {n_sig} Ampelteilen; {entfernt} alte Eintraege an der Stelle entfernt, Gelaende in {n_gel} Kachel(n) abgesenkt",
               kreuzungen.len()))
}

/// abzusenkende Gelaendepunkte: (Kachel, Spalte, Zeile ab Sueden, Hoehe)
pub fn gelaende_lesen(datei: &Path) -> Result<Vec<((i32, i32), usize, usize, f32)>> {
    gelaende_lesen_feld(datei, "gelaende")
}

/// Gelaendepunkte aus einem Feld der Sonderbau-Datei ([kx, ky, spalte, zeile, z, ...])
pub fn gelaende_lesen_feld(datei: &Path, feld: &str) -> Result<Vec<((i32, i32), usize, usize, f32)>> {
    let j: serde_json::Value = serde_json::from_slice(&std::fs::read(datei)?)?;
    Ok(j[feld]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    let w = |i: usize| p.get(i).and_then(|x| x.as_f64());
                    Some(((w(0)? as i32, w(1)? as i32), w(2)? as usize, w(3)? as usize, w(4)? as f32))
                })
                .collect()
        })
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;
    use openomsi_game::viewer::Viewer;

    /// Etappe 1 am Ring: alle Teile in Aburg_Newgen setzen (Sitzungsordner, die Karte bleibt unveraendert) und Bilder
    /// aus openOMSI (OMSI_BILD) - Vergleich mit den Fotos
    #[test]
    #[ignore]
    fn ring_bauwerk() {
        let _sperre = crate::bearbeiten::tests::sperre();
        let root = Path::new(crate::bearbeiten::tests::OMSI);
        let karte = root.join("maps/Aburg_Newgen");
        let json = root.join("Sceneryobjects/Aschaffenburg/Aburg_Newgen/Sonderbau_Ring/sonderbau_ring.json");
        let teile = lesen(&json).unwrap();
        assert!(teile.len() > 10, "{} Teile", teile.len());
        let instance = openomsi_game::viewer::instance();
        let (mut v, _) = Viewer::open(&instance, None, root, &karte.join("global.cfg")).unwrap();
        let mitte = DVec3::new(1084.0, -265.0, 0.0);
        v.tiles_around(mitte, 1).unwrap();
        let mut ae = crate::aendern::Aendern::neu(&v);
        let mut gel = crate::gelaende::Gelaende::default();
        let gel_ordner = ae.sitzung.join("maps").join("Aburg_Newgen");
        let msg = einsetzen(&mut ae, &mut gel, &mut v, root, &gel_ordner, &json).unwrap();
        println!("{msg}");
        let strassen = strassen_lesen(&json).unwrap();
        assert!(strassen.len() >= 5, "{} Strassen", strassen.len());
        // die Kreuzung trifft die Strassen: Oberflaeche 1 m innerhalb jedes Strassenendes = Spline + 0,10 m
        v.tiles_around(mitte, 1).unwrap();
        for plan in kreuzungen_lesen(&json).unwrap() {
            assert!(plan.gesetzt.signale.len() >= 12);
            let o = plan.gesetzt.pos.truncate();
            for st in &strassen {
                for (p, z, h) in [st.elemente.first().map(|e| (e.pos, e.z, e.richtung + 180.0)), st.elemente.last().map(|e| {
                    let c = crate::netz::Stueck { start: e.pos, richtung: e.richtung, laenge: e.laenge, radius: e.radius };
                    let (q, hq) = c.ende();
                    (q, e.z + e.dh, hq)
                })].into_iter().flatten() {
                    if (p - o).length() > 45.0 || st.name == "Ring" || st.sli.is_some() {
                        continue;
                    }
                    let r = h.to_radians();
                    let q = p + DVec2::new(r.sin(), r.cos()) * 1.0;
                    let ist = v.surface_height(q.x, q.y);
                    println!("  {:20} Strasse {:.2}  Kreuzung {:?}", st.name, z + 0.10, ist.map(|x| (x * 100.0).round() / 100.0));
                    assert!(ist.map(|x| (x - z - 0.10).abs() < 0.08).unwrap_or(false), "{}: Kreuzung trifft die Strasse nicht", st.name);
                }
            }
        }
        // ein zweites Einsetzen (nach "Anwenden" im Objekt-Editor) ersetzt den ersten, statt ihn zu verdoppeln
        let vorher: usize = v.loaded_tile_keys().iter().filter_map(|k| v.tile_file(k.0, k.1)).map(|d| std::fs::read(ae.kopie(&v, &d)).map(|b| b.len()).unwrap_or(0)).sum();
        einsetzen(&mut ae, &mut gel, &mut v, root, &gel_ordner, &json).unwrap();
        let nachher: usize = v.loaded_tile_keys().iter().filter_map(|k| v.tile_file(k.0, k.1)).map(|d| std::fs::read(ae.kopie(&v, &d)).map(|b| b.len()).unwrap_or(0)).sum();
        println!("Kacheln vorher {vorher} B, nach erneutem Einsetzen {nachher} B");
        assert!((nachher as f64 - vorher as f64).abs() < vorher as f64 * 0.02, "erneutes Einsetzen verdoppelt");
        v.tiles_around(mitte, 1).unwrap();
        let z = v.terrain_height(1084.0, -265.0).unwrap_or(10.0);
        println!("Gelaende an der Kreuzung: {z:.2} m");
        if let Some(bild) = std::env::var_os("OMSI_BILD") {
            let bild = std::path::PathBuf::from(bild);
            // (Ziel, Gier, Neigung, Abstand, Name): wie die Fotos - im Trog nach Sueden zur Unterfuehrung, von der
            // Rampe Ost, von oben, die Bahnbruecke
            let ansichten = [
                (DVec3::new(1082.5, -215.0, 6.5), 180.0, -3.0, 22.0, "trog_sued"),
                (DVec3::new(1078.0, -320.0, 6.0), 0.0, -3.0, 22.0, "trog_nord"),
                (DVec3::new(1084.0, -258.0, 7.0), 180.0, -2.0, 25.0, "portal"),
                (DVec3::new(1093.0, -225.0, 11.5), 180.0, -5.0, 18.0, "rampe_ost"),
                (DVec3::new(1072.0, -230.0, 11.5), 180.0, -5.0, 18.0, "rampe_west"),
                (DVec3::new(1084.0, -265.0, 6.0), 200.0, -40.0, 120.0, "oben"),
                (DVec3::new(1068.0, -390.0, 6.0), 200.0, -35.0, 70.0, "sued_ende"),
                (DVec3::new(1074.0, -165.0, 6.0), 165.0, -40.0, 60.0, "nord_ende"),
                (DVec3::new(1074.0, -247.0, 10.0), 160.0, -45.0, 22.0, "rampe_nw"),
                (DVec3::new(1094.0, -249.0, 10.5), 200.0, -45.0, 22.0, "rampe_no"),
                (DVec3::new(1074.0, -282.0, 10.0), 340.0, -45.0, 22.0, "rampe_sw"),
                (DVec3::new(1094.0, -284.0, 10.0), 20.0, -45.0, 22.0, "rampe_so"),
                (DVec3::new(1076.0, -150.0, 5.5), 165.0, -6.0, 18.0, "nord_einfahrt"),
                (DVec3::new(1084.0, -266.0, 11.0), 0.0, -65.0, 75.0, "kreuzung_oben"),
                (DVec3::new(1080.0, -266.0, 12.0), 95.0, -6.0, 45.0, "kreuzung_west"),
                (DVec3::new(1075.0, -262.0, 12.0), 180.0, -8.0, 35.0, "kreuzung_rampe"),
                (DVec3::new(1057.0, -269.5, 10.5), 90.0, -75.0, 16.0, "zufahrt_west"),
                (DVec3::new(1063.0, -269.0, 12.0), 90.0, -2.0, 14.0, "fahrer_west"),
                (DVec3::new(1111.0, -261.0, 13.0), 270.0, -2.0, 14.0, "fahrer_ost"),
                (DVec3::new(1076.0, -252.0, 11.5), 175.0, -2.0, 14.0, "fahrer_rampe_west"),
                (DVec3::new(1093.0, -282.0, 11.5), 355.0, -2.0, 14.0, "fahrer_rampe_ost"),
                (DVec3::new(1085.0, -252.0, 10.5), 180.0, -55.0, 22.0, "insel_nord"),
                (DVec3::new(1110.0, -261.0, 11.5), 270.0, -75.0, 16.0, "zufahrt_ost"),
                (DVec3::new(1070.0, -385.0, 6.5), 10.0, -4.0, 30.0, "sued_einfahrt"),
                (DVec3::new(1056.0, -266.0, 6.0), 0.0, -12.0, 35.0, "bahnbruecke"),
                (DVec3::new(1060.0, -268.0, 5.2), 2.0, -3.0, 32.0, "bahntunnel"),
            ];
            // eigene Ansichten: OMSI_ANSICHTEN="x,y,z,gier,neigung,abstand,name;..." (Nahaufnahmen von Stossstellen)
            let eigene: Vec<(DVec3, f32, f32, f64, String)> = std::env::var("OMSI_ANSICHTEN").unwrap_or_default().split(';').filter_map(|a| {
                let w: Vec<&str> = a.split(',').map(|x| x.trim()).collect();
                if w.len() != 7 {
                    return None;
                }
                let z = |i: usize| w[i].parse::<f64>().ok();
                Some((DVec3::new(z(0)?, z(1)?, z(2)?), z(3)? as f32, z(4)? as f32, z(5)?, w[6].to_string()))
            }).collect();
            let alle: Vec<(DVec3, f32, f32, f64, String)> = if eigene.is_empty() {
                ansichten.iter().map(|(a, b, c, d, e)| (*a, *b as f32, *c as f32, *d, e.to_string())).collect()
            } else {
                eigene
            };
            for (ziel, gier, neigung, abstand, name) in alle {
                let kam = crate::kamera::Kamera { ziel, gier, neigung, abstand, fov: 60.0, fest: None };
                let px = v.render_image(1280, 800, &kam.camera()).unwrap();
                image::save_buffer(bild.with_extension(format!("{name}.png")), &px, 1280, 800, image::ColorType::Rgba8).unwrap();
            }
        }
        drop(ae);
    }
}
