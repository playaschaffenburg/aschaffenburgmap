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

/// Strasse des Sonderbaus: Querschnitt (Baukasten) und Elemente in Fahrtrichtung des Splines
#[derive(Clone, Debug)]
pub struct Strasse {
    pub name: String,
    pub querschnitt: crate::querschnitt::Querschnitt,
    pub elemente: Vec<Element>,
}

pub fn strassen_lesen(datei: &Path) -> Result<Vec<Strasse>> {
    let j: serde_json::Value = serde_json::from_slice(&std::fs::read(datei)?)?;
    let mut out = Vec::new();
    for s in j["strassen"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
        let q = crate::querschnitt::Querschnitt::aus_json(&s["querschnitt"]).with_context(|| format!("Querschnitt von {}", s["name"]))?;
        let w = |e: &serde_json::Value, k: &str| e[k].as_f64().unwrap_or(0.0);
        let elemente = s["elemente"].as_array().map(|a| a.as_slice()).unwrap_or(&[]).iter().map(|e| Element {
            pos: DVec2::new(w(e, "x"), w(e, "y")), z: w(e, "z"), richtung: w(e, "richtung"), laenge: w(e, "laenge"),
            radius: w(e, "radius"), stg_a: w(e, "stg_a"), stg_e: w(e, "stg_e"), dh: w(e, "dh"), kum: w(e, "kum"),
        }).collect();
        out.push(Strasse { name: s["name"].as_str().unwrap_or("").to_string(), querschnitt: q, elemente });
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
        let sli = crate::querschnitt::schreiben(root, &st.querschnitt, &st.querschnitt.datei())?;
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

/// abzusenkende Gelaendepunkte: (Kachel, Spalte, Zeile ab Sueden, Hoehe)
pub fn gelaende_lesen(datei: &Path) -> Result<Vec<((i32, i32), usize, usize, f32)>> {
    let j: serde_json::Value = serde_json::from_slice(&std::fs::read(datei)?)?;
    Ok(j["gelaende"]
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
        // bisherige Strassen an der Stelle weg (nur in der Sitzung), damit das Bild das Bauwerk zeigt
        ae.aktualisieren(&v);
        let im_bereich = |p: DVec2| (1030.0..1140.0).contains(&p.x) && (-428.0..-130.0).contains(&p.y);
        ae.auswahl = ae.kacheln.values().flatten().filter(|s| {
            s.punkte().iter().any(|(q, _)| im_bereich(q.truncate()))
        }).map(|s| s.id).collect();
        println!("{} Splines an der Stelle entfernt", ae.loeschen(&mut v).unwrap());
        let ts = omsi_map::tile_size();
        let kacheln_hier: Vec<(i32, i32)> = v.loaded_tile_keys().into_iter().collect();
        let weg = ae.kacheln_aendern(&mut v, &kacheln_hier, |k, z| {
            let mut n = 0;
            let mut i = 0;
            while i < z.len() {
                let lage = || Some(DVec2::new(z.get(i + 4)?.trim().parse::<f64>().ok()? + k.0 as f64 * ts, z.get(i + 5)?.trim().parse::<f64>().ok()? + k.1 as f64 * ts));
                if z[i].trim() == "[object]" && lage().map(im_bereich).unwrap_or(false) {
                    let mut j = i + 1;
                    while j < z.len() && !z[j].trim_start().starts_with('[') {
                        j += 1;
                    }
                    z.drain(i..j);
                    n += 1;
                } else {
                    i += 1;
                }
            }
            Ok(n)
        }).unwrap();
        println!("{weg} Objekte an der Stelle entfernt");
        for t in &teile {
            ae.objekt_anlegen_hoehe(&mut v, &t.rel, t.pos, t.hoehe, 0.0, &[]).unwrap();
        }
        // Etappe 2: Strassen
        let strassen = strassen_lesen(&json).unwrap();
        assert!(strassen.len() >= 5, "{} Strassen", strassen.len());
        println!("{} Splines angelegt", strassen_anlegen(&mut ae, &mut v, root, &strassen).unwrap());
        v.tiles_around(mitte, 1).unwrap();
        let senken = gelaende_lesen(&json).unwrap();
        let mut kacheln: std::collections::HashMap<(i32, i32), omsi_map::Terrain> = Default::default();
        for (k, spalte, zeile, z) in &senken {
            let t = kacheln.entry(*k).or_insert_with(|| v.tile_terrain(k.0, k.1).unwrap());
            let i = zeile * (t.cells + 1) + spalte;
            t.heights[i] = t.heights[i].min(*z);
        }
        for (k, t) in &kacheln {
            assert!(v.preview_terrain(k.0, k.1, t), "Kachel {k:?} nicht geladen");
        }
        println!("{} Gelaendepunkte in {} Kacheln abgesenkt", senken.len(), kacheln.len());
        let z =v.terrain_height(1084.0, -265.0).unwrap_or(10.0);
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
                (DVec3::new(1070.0, -385.0, 6.5), 10.0, -4.0, 30.0, "sued_einfahrt"),
                (DVec3::new(1056.0, -266.0, 6.0), 0.0, -12.0, 35.0, "bahnbruecke"),
            ];
            for (ziel, gier, neigung, abstand, name) in ansichten {
                let kam = crate::kamera::Kamera { ziel, gier, neigung, abstand, fov: 60.0 };
                let px = v.render_image(1280, 800, &kam.camera()).unwrap();
                image::save_buffer(bild.with_extension(format!("{name}.png")), &px, 1280, 800, image::ColorType::Rgba8).unwrap();
            }
        }
        drop(ae);
    }
}
