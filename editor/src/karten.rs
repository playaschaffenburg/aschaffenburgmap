//! Kartenverwaltung: die Karten der Installation (Ordner, Anzeigename, Beschreibung, Vorschaubild), Anzeigename und
//! Ordnername aendern, Karte in den Papierkorb verschieben.
//!
//! Eine OMSI-Karte hat zwei Namen: den Ordner unter maps\ (steht auch in global.cfg unter [name]; Spielstaende und
//! die Objektordner unter Aschaffenburg\ verweisen darauf) und den Anzeigenamen [friendlyname] in global.cfg und
//! in den Sprachdateien global_<Sprache>.dsc, den OMSI in der Kartenauswahl zeigt.

use crate::speichern::{dekodieren, global_setzen, kodieren, name_ok};
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

/// Karten, die mit OMSI 2 kommen (Loeschen/Umbenennen mit Warnung)
const STANDARD: [&str; 2] = ["grundorf", "berlin-spandau"];

#[derive(Clone, Debug)]
pub struct Karte {
    pub ordner: String,
    pub global: PathBuf,
    /// [friendlyname] (deutsche .dsc, sonst global.cfg), sonst der Ordner
    pub anzeige: String,
    pub beschreibung: String,
    pub kacheln: usize,
    /// vom Editor angelegt oder von omsigen erzeugt
    pub eigene: bool,
    /// kommt mit OMSI 2
    pub standard: bool,
    /// picture.jpg der Karte
    pub bild: Option<PathBuf>,
}

/// Text eines Abschnitts ([schluessel] bis zur Leerzeile bzw. bis [end] bei der Beschreibung)
fn abschnitt(text: &str, schluessel: &str, bis_end: bool) -> Option<String> {
    let zeilen: Vec<&str> = text.lines().map(|l| l.trim_end_matches('\r')).collect();
    let i = zeilen.iter().position(|z| z.trim().eq_ignore_ascii_case(schluessel))?;
    let mut out = Vec::new();
    for z in &zeilen[i + 1..] {
        let t = z.trim();
        if bis_end {
            if t.eq_ignore_ascii_case("[end]") {
                break;
            }
        } else if t.is_empty() || t.starts_with('[') {
            break;
        }
        out.push(t.to_string());
    }
    let s = out.join(if bis_end { "\n" } else { " " }).trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn lesen(p: &Path) -> Option<String> {
    std::fs::read(p).ok().map(|b| dekodieren(&b).0)
}

pub fn finden(root: &Path) -> Vec<Karte> {
    let mut v = Vec::new();
    if let Ok(rd) = std::fs::read_dir(root.join("maps")) {
        for e in rd.flatten() {
            let d = e.path();
            let g = d.join("global.cfg");
            if !g.is_file() {
                continue;
            }
            let ordner = e.file_name().to_string_lossy().into_owned();
            let cfg = lesen(&g).unwrap_or_default();
            let dsc = lesen(&d.join("global_DEU.dsc"));
            let anzeige = dsc.as_deref().and_then(|t| abschnitt(t, "[friendlyname]", false))
                .or_else(|| abschnitt(&cfg, "[friendlyname]", false)).unwrap_or_else(|| ordner.clone());
            let beschreibung = dsc.as_deref().and_then(|t| abschnitt(t, "[description]", true))
                .or_else(|| abschnitt(&cfg, "[description]", true)).unwrap_or_default();
            let kacheln = std::fs::read_dir(&d).map(|r| r.flatten().filter(|f| {
                let n = f.file_name().to_string_lossy().to_lowercase();
                n.starts_with("tile_") && n.ends_with(".map")
            }).count()).unwrap_or(0);
            let bild = ["picture.jpg", "picture.png"].iter().map(|n| d.join(n)).find(|p| p.is_file());
            v.push(Karte {
                eigene: crate::speichern::eigene_karte(root, &ordner),
                standard: STANDARD.contains(&ordner.to_lowercase().as_str()),
                ordner, global: g, anzeige, beschreibung, kacheln, bild,
            });
        }
    }
    v.sort_by_key(|k| k.ordner.to_lowercase());
    v
}

/// Anzeigenamen setzen: [friendlyname] in global.cfg und in allen global_<Sprache>.dsc
pub fn anzeigename_setzen(root: &Path, ordner: &str, neu: &str) -> Result<()> {
    let neu = neu.trim();
    if neu.is_empty() || neu.contains(['\r', '\n']) {
        bail!("Anzeigename leer oder mehrzeilig");
    }
    let d = root.join("maps").join(ordner);
    global_setzen(&d.join("global.cfg"), &[("[friendlyname]", neu.to_string())])?;
    for e in std::fs::read_dir(&d)?.flatten() {
        let n = e.file_name().to_string_lossy().to_lowercase();
        if n.starts_with("global_") && n.ends_with(".dsc") {
            global_setzen(&e.path(), &[("[friendlyname]", neu.to_string())])?;
        }
    }
    log::info!("Karte {ordner}: Anzeigename jetzt \"{neu}\"");
    Ok(())
}

/// Angaben fuer eine neue Karte
#[derive(Clone, Debug, Default)]
pub struct NeueKarte {
    pub ordner: String,
    pub anzeige: String,
    pub beschreibung: String,
    /// Ort der Karte: Gelaende aus den Geodaten, Luftbild im Editor, spaeter erweiterbar (sonst flach, ohne Ort)
    pub bezug: Option<crate::geo::Bezug>,
    /// Kantenlaenge des Anfangsgebiets in Kacheln (1, 3, 5; 0 = 1)
    pub groesse: i32,
}

/// Strassenprofil auf der Grundkachel (lokal x = 150, y 90..210): unter der Fahrbahn wird das Gelaende auf die
/// Strassenhoehe gebracht (5 cm darunter), daneben laeuft es auf 8 m Breite ins Gelaende aus
fn strasse_einebnen(h: &mut [f32], hs: f64, he: f64) {
    for iz in 0..61 {
        for ix in 0..61 {
            let (x, y) = (ix as f64 * 5.0, iz as f64 * 5.0);
            let t = ((y - 90.0) / 120.0).clamp(0.0, 1.0);
            let profil = hs + (he - hs) * t - 0.05;
            let quer = (x - 150.0).abs();
            let laengs = if y < 90.0 { 90.0 - y } else if y > 210.0 { y - 210.0 } else { 0.0 };
            let d = quer.max(laengs);
            let w = if d <= 7.0 { 1.0 } else { (1.0 - (d - 7.0) / 8.0).max(0.0) };
            let i = iz * 61 + ix;
            h[i] = (h[i] as f64 + (profil - h[i] as f64) * w) as f32;
        }
    }
}

/// Querschnitt des Strassenstuecks auf der Grundkachel: der erste, den es gibt
const START_SLI: [&str; 3] = ["Splines\\Marcel\\str_2spur_8m_altonaer1.sli", "Splines\\Marcel\\str_2spur_10m_Grunewaldstr.sli",
                              "Splines\\Marcel\\str_2spur_11m_SeeburgerStr1.sli"];

/// Ordnername aus einem Anzeigenamen (Buchstaben, Ziffern, _ -; Umlaute umschrieben)
pub fn ordner_aus(anzeige: &str) -> String {
    let mut o = String::new();
    for c in anzeige.trim().chars() {
        match c {
            'ä' => o.push_str("ae"), 'ö' => o.push_str("oe"), 'ü' => o.push_str("ue"), 'Ä' => o.push_str("Ae"), 'Ö' => o.push_str("Oe"),
            'Ü' => o.push_str("Ue"), 'ß' => o.push_str("ss"),
            c if c.is_ascii_alphanumeric() || c == '-' || c == '_' => o.push(c),
            ' ' => o.push('_'),
            _ => {}
        }
    }
    o
}

/// Querlage der rechten Fahrspur in Splinerichtung (aus den [path]-Eintraegen der .sli), sonst 1,75 m
fn rechte_spur(sli: &Path) -> f64 {
    let t = std::fs::read(sli).map(|b| b.iter().map(|&c| c as char).collect::<String>()).unwrap_or_default();
    let z: Vec<&str> = t.lines().map(|l| l.trim()).collect();
    let mut beste: Option<f64> = None;
    for i in 0..z.len() {
        if z[i].eq_ignore_ascii_case("[path]") && i + 5 < z.len() {
            let art: i32 = z[i + 1].parse().unwrap_or(-1);
            let x: f64 = z[i + 2].replace(',', ".").parse().unwrap_or(f64::NAN);
            let richtung: i32 = z[i + 5].parse().unwrap_or(-1);
            if art == 0 && richtung == 0 && x > 0.0 && beste.is_none_or(|b| x < b) {
                beste = Some(x);
            }
        }
    }
    beste.unwrap_or(1.75)
}

/// Neue Karte aus der OMSI-Vorlage (template\\NewMap): Name, Anzeigename, Beschreibung; die Grundkachel 0 0 bekommt
/// ein 120 m langes Strassenstueck (Nord-Sued durch die Mitte) und darauf einen Einsetzpunkt in der rechten Spur.
/// Die Karte traegt die Marke des Editors (Speichern ohne Rueckfrage). -> Kartenordner
pub fn neue_karte(root: &Path, n: &NeueKarte) -> Result<PathBuf> {
    let ordner = n.ordner.trim();
    let anzeige = n.anzeige.trim();
    if !name_ok(ordner) {
        bail!("Ordnername \"{ordner}\": nur Buchstaben, Ziffern, _ - und Leerzeichen");
    }
    if anzeige.is_empty() || anzeige.contains(['\r', '\n']) {
        bail!("Anzeigename leer oder mehrzeilig");
    }
    let vorlage = root.join("template").join("NewMap");
    if !vorlage.join("global.cfg").is_file() {
        bail!("OMSI-Vorlage {} fehlt", vorlage.display());
    }
    let ziel = root.join("maps").join(ordner);
    if ziel.exists() {
        bail!("maps\\{ordner} gibt es schon - anderen Namen waehlen");
    }
    let sli = START_SLI.iter().find(|s| root.join(s.replace('\\', "/")).is_file()).context("kein Strassen-Querschnitt fuer die Grundkachel (Splines\\Marcel)")?;
    // Anfangsgebiet: Kachel 0 0 (in ihrer Mitte der Ort) und die Kacheln rundherum
    let r = (n.groesse.max(1) - 1) / 2;
    let mut kacheln = vec![(0, 0)];
    for x in -r..=r {
        for y in -r..=r {
            if (x, y) != (0, 0) {
                kacheln.push((x, y));
            }
        }
    }
    // Gelaende des Orts (vor dem Anlegen holen: fehlt es, bleibt nichts Halbes liegen)
    let mut gelaende: std::collections::HashMap<(i32, i32), Vec<f32>> = Default::default();
    if let Some(b) = &n.bezug {
        let (daten, _) = crate::geo::kacheln(b, &kacheln, true, false).context("Gelaende holen")?;
        for d in daten {
            if let Some(h) = d.gelaende {
                gelaende.insert(d.kachel, h);
            }
        }
        if !gelaende.contains_key(&(0, 0)) {
            bail!("keine Gelaendedaten fuer \"{}\" - ohne Netz oder ausserhalb der Daten?", b.ort);
        }
    }
    let (hs, he) = match gelaende.get(&(0, 0)) {
        Some(h) => {
            let t = omsi_map::Terrain { cells: 60, heights: h.clone() };
            (t.sample(150.0, 90.0) as f64, t.sample(150.0, 210.0) as f64)
        }
        None => (0.0, 0.0),
    };
    if let Some(h) = gelaende.get_mut(&(0, 0)) {
        strasse_einebnen(h, hs, he);
    }
    let steigung = (he - hs) / 120.0 * 100.0;
    kopieren(&vorlage, &ziel).with_context(|| format!("Vorlage nach {} kopieren", ziel.display()))?;
    // Grundkachel: Strasse von (150, 90) nach Norden, 120 m; Einsetzpunkt in der rechten Spur
    let x_spur = 150.0 + rechte_spur(&root.join(sli.replace('\\', "/")));
    let stempel = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let kachel = [
        format!("File created with omsi-editor ({stempel})"), String::new(), "[version]".into(), "14".into(), String::new(),
        "[terrain]".into(), String::new(), String::new(), "[variable_terrainlightmap]".into(), String::new(), "[variable_terrain]".into(), String::new(),
        "[spline_h]".into(), "0".into(), sli.to_string(), "1".into(), "0".into(), "0".into(), "150".into(), format!("{hs:.3}"), "90".into(), "0".into(),
        "120".into(), "0".into(), format!("{steigung:.4}"), format!("{steigung:.4}"), format!("{:.3}", he - hs), "0".into(), "0".into(), "0".into(), "0".into(), "0".into(), String::new(),
        "Object Nr. 0".into(), "[object]".into(), "0".into(), crate::orte::EINSETZPUNKT.into(), "2".into(), format!("{x_spur:.3}"), "100".into(),
        "0".into(), "0".into(), "0".into(), "0".into(), "0".into(), String::new(),
    ].join("\r\n");
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend(kachel.encode_utf16().flat_map(|u| u.to_le_bytes()));
    std::fs::write(ziel.join("tile_0_0.map"), bytes)?;
    // die anderen Kacheln des Anfangsgebiets: leer, Lichtkarte der Vorlage
    let mut liste = vec![(0, 0, "tile_0_0.map".to_string())];
    for &(x, y) in kacheln.iter().skip(1) {
        let datei = format!("tile_{x}_{y}.map");
        let text = format!("File created with omsi-editor ({stempel})\r\n\r\n[version]\r\n14\r\n\r\n[terrain]\r\n\r\n\r\n[variable_terrainlightmap]\r\n\r\n[variable_terrain]\r\n\r\n");
        let mut b = vec![0xFF, 0xFE];
        b.extend(text.encode_utf16().flat_map(|u| u.to_le_bytes()));
        std::fs::write(ziel.join(&datei), b)?;
        let lm = ziel.join("tile_0_0.map.LM.bmp");
        if lm.is_file() {
            std::fs::copy(&lm, ziel.join(format!("{datei}.LM.bmp")))?;
        }
        if !gelaende.contains_key(&(x, y)) {
            // ohne Ort: flach wie die Vorlage
            gelaende.insert((x, y), vec![0.0; 61 * 61]);
        }
        liste.push((x, y, datei));
    }
    for ((x, y), h) in &gelaende {
        let t = omsi_map::Terrain { cells: 60, heights: h.clone() };
        std::fs::write(ziel.join(format!("tile_{x}_{y}.map.terrain")), t.to_bytes())?;
    }
    if let Some(b) = &n.bezug {
        b.schreiben(&ziel)?;
    }
    // global.cfg: Namen, naechste ID, Beschreibung, Kamera an der Strasse, Einsetzpunkt
    let g = ziel.join("global.cfg");
    global_setzen(&g, &[("[name]", ordner.to_string()), ("[friendlyname]", anzeige.to_string()), ("[NextIDCode]", "3".into())])?;
    let (text, utf16) = dekodieren(&std::fs::read(&g)?);
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let z: Vec<&str> = text.split(eol).collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < z.len() {
        let w = z[i].trim().to_ascii_lowercase();
        if w == "[description]" {
            out.push(z[i].to_string());
            let beschreibung = if n.beschreibung.trim().is_empty() { anzeige.to_string() } else { n.beschreibung.trim().replace("\r\n", "\n") };
            out.extend(beschreibung.split('\n').map(|x| x.to_string()));
            while i < z.len() && !z[i].trim().eq_ignore_ascii_case("[end]") {
                i += 1;
            }
            continue;
        }
        if w == "[mapcam]" && i + 8 < z.len() {
            out.extend([z[i].to_string(), "0".into(), "0".into(), "150".into(), z[i + 4].to_string(), "100".into()]);
            out.extend(z[i + 6..i + 9].iter().map(|x| x.to_string()));
            i += 9;
            continue;
        }
        if w == "[map]" && !out.iter().any(|x| x.trim().eq_ignore_ascii_case("[entrypoints]")) {
            out.extend(["[entrypoints]".to_string(), "1".into(), "0".into(), "2".into(), "0".into(), format!("{x_spur:.3}"), format!("{:.3}", hs + (he - hs) * 10.0 / 120.0), "100.000".into(),
                        "0.000".into(), "0.000".into(), "0.000".into(), "1.000".into(), "0".into(), "Start".into(), String::new()]);
        }
        out.push(z[i].to_string());
        i += 1;
    }
    std::fs::write(&g, kodieren(&out.join(eol), utf16))?;
    if liste.len() > 1 {
        crate::speichern::kachelliste_setzen(&ziel, &liste[..1], &liste)?;
    }
    let _ = std::fs::write(ziel.join(crate::speichern::MARKE), format!("Mit dem OMSI-Editor (aschaffenburgmap) neu angelegt: \"{anzeige}\".\r\nDiese Karte darf der Editor beim Speichern ueberschreiben (mit Sicherung).\r\n"));
    log::info!("neue Karte maps\\{ordner} (\"{anzeige}\") mit {} Kachel(n), Strasse {sli} und Einsetzpunkt{}", liste.len(),
               n.bezug.as_ref().map(|b| format!(", Ort {} (UTM {} {:.0} {:.0}, NN {:.0} m)", b.ort, b.zone, b.ost0, b.nord0, b.nn0)).unwrap_or_default());
    Ok(ziel)
}

fn kopieren(von: &Path, nach: &Path) -> Result<()> {
    std::fs::create_dir_all(nach)?;
    for e in std::fs::read_dir(von)? {
        let e = e?;
        if e.file_type()?.is_dir() {
            kopieren(&e.path(), &nach.join(e.file_name()))?;
        } else {
            std::fs::copy(e.path(), nach.join(e.file_name()))?;
        }
    }
    Ok(())
}

/// Ordner einer Karte umbenennen: maps\alt -> maps\neu, [name] in global.cfg, die eigenen Ordner unter
/// Sceneryobjects\Aschaffenburg\ und Splines\Aschaffenburg\ (und die frueheren unter Aschaffenburg_KI\) und die
/// Verweise der Kacheln darauf
pub fn umbenennen(root: &Path, alt: &str, neu: &str) -> Result<()> {
    if !name_ok(neu) {
        bail!("Ordnername \"{neu}\": nur Buchstaben, Ziffern, _ - und Leerzeichen");
    }
    let (von, nach) = (root.join("maps").join(alt), root.join("maps").join(neu));
    if !von.join("global.cfg").is_file() {
        bail!("{} ist keine Karte", von.display());
    }
    if nach.exists() {
        bail!("{} gibt es schon", nach.display());
    }
    let eigene: Vec<(PathBuf, PathBuf)> = ["Sceneryobjects", "Splines"].iter()
        .flat_map(|b| crate::speichern::EIGENE_ORDNER.iter().map(move |o| (root.join(b).join(o).join(alt), root.join(b).join(o).join(neu))))
        .filter(|(a, _)| a.is_dir()).collect();
    for (_, n) in &eigene {
        if n.exists() {
            bail!("{} gibt es schon", n.display());
        }
    }
    // andere Karten, die Objekte dieser Karte nutzen, holen sie vorher zu sich
    let andere = crate::speichern::abhaengige_loesen(root, alt)?;
    if !andere.is_empty() {
        log::info!("vor dem Umbenennen von {alt}: Objekte in {andere:?} uebernommen");
    }
    std::fs::rename(&von, &nach).with_context(|| format!("{} umbenennen (ist die Karte noch in OMSI oder im nEditor offen?)", von.display()))?;
    global_setzen(&nach.join("global.cfg"), &[("[name]", neu.to_string())])?;
    for (a, n) in &eigene {
        std::fs::rename(a, n).with_context(|| format!("{} umbenennen", a.display()))?;
    }
    if !eigene.is_empty() {
        for e in std::fs::read_dir(&nach)?.flatten() {
            let n = e.file_name().to_string_lossy().to_lowercase();
            if !(n.starts_with("tile_") && n.ends_with(".map")) {
                continue;
            }
            let (text, utf16) = dekodieren(&std::fs::read(e.path())?);
            let mut neu_text = text.clone();
            for o in crate::speichern::EIGENE_ORDNER {
                neu_text = neu_text.replace(&format!("{o}\\{alt}\\"), &format!("{o}\\{neu}\\"));
            }
            if neu_text != text {
                std::fs::write(e.path(), kodieren(&neu_text, utf16))?;
            }
        }
    }
    log::info!("Karte {alt} umbenannt in {neu} ({} eigene Ordner mit)", eigene.len());
    Ok(())
}

/// Karte in den Windows-Papierkorb verschieben (mit ihren eigenen Ordnern unter Aschaffenburg\ bzw. Aschaffenburg_KI\) -> was verschoben
/// wurde. Geloescht wird nichts endgueltig.
pub fn in_papierkorb(root: &Path, ordner: &str) -> Result<Vec<PathBuf>> {
    let d = root.join("maps").join(ordner);
    if !d.join("global.cfg").is_file() || ordner.trim().is_empty() || ordner.contains(['\\', '/']) || ordner.contains("..") {
        bail!("{} ist keine Karte", d.display());
    }
    let mut wege = vec![d];
    for b in ["Sceneryobjects", "Splines"] {
        for o in crate::speichern::EIGENE_ORDNER {
            let e = root.join(b).join(o).join(ordner);
            if e.is_dir() {
                wege.push(e);
            }
        }
    }
    // andere Karten, die Objekte dieser Karte nutzen, holen sie vorher zu sich (sonst fehlen ihnen danach Kreuzungen)
    let andere = crate::speichern::abhaengige_loesen(root, ordner)?;
    if !andere.is_empty() {
        log::info!("vor dem Loeschen von {ordner}: Objekte in {andere:?} uebernommen");
    }
    for w in &wege {
        papierkorb(w)?;
    }
    log::info!("Karte {ordner} in den Papierkorb verschoben: {wege:?}");
    Ok(wege)
}

/// einen Ordner in den Papierkorb (Windows: Microsoft.VisualBasic.FileIO, wie der Explorer)
fn papierkorb(p: &Path) -> Result<()> {
    let pfad = p.to_string_lossy().replace('\'', "''");
    let befehl = format!("Add-Type -AssemblyName Microsoft.VisualBasic; [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteDirectory('{pfad}', 'OnlyErrorDialogs', 'SendToRecycleBin')");
    let aus = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &befehl])
        .output()
        .context("powershell starten (Papierkorb)")?;
    if !aus.status.success() || p.exists() {
        bail!("{} nicht in den Papierkorb verschoben: {}", p.display(), String::from_utf8_lossy(&aus.stderr).trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn testkarte(root: &Path, name: &str) {
        let d = root.join("maps").join(name);
        std::fs::create_dir_all(&d).unwrap();
        let utf16 = |t: &str| -> Vec<u8> { [0xFF, 0xFE].into_iter().chain(t.encode_utf16().flat_map(|u| u.to_le_bytes())).collect() };
        std::fs::write(d.join("global.cfg"), utf16(&format!("x\r\n\r\n[name]\r\n{name}\r\n\r\n[friendlyname]\r\nAlter Name\r\n\r\n[description]\r\nZeile 1\r\nZeile 2\r\n[end]\r\n\r\n[NextIDCode]\r\n5\r\n"))).unwrap();
        std::fs::write(d.join("global_DEU.dsc"), "\r\n[friendlyname]\r\nAlter Name DE\r\n\r\n[description]\r\nBeschreibung DE\r\n[end]").unwrap();
        std::fs::write(d.join("tile_0_0.map"), utf16(&format!("[object]\r\n0\r\nSceneryobjects\\Aschaffenburg_KI\\{name}\\K_1.sco\r\n7\r\n"))).unwrap();
        let o = root.join("Sceneryobjects").join("Aschaffenburg_KI").join(name);
        std::fs::create_dir_all(&o).unwrap();
        std::fs::write(o.join("K_1.sco"), "x").unwrap();
    }

    #[test]
    fn finden_umbenennen_anzeigename() {
        let root = std::env::temp_dir().join(format!("omsi-editor-karten-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        testkarte(&root, "Probe");
        let k = finden(&root);
        assert_eq!(k.len(), 1);
        assert_eq!(k[0].anzeige, "Alter Name DE");
        assert_eq!(k[0].beschreibung, "Beschreibung DE");
        assert_eq!(k[0].kacheln, 1);
        anzeigename_setzen(&root, "Probe", "Neuer Name").unwrap();
        let k = finden(&root);
        assert_eq!(k[0].anzeige, "Neuer Name");
        let cfg = dekodieren(&std::fs::read(root.join("maps/Probe/global.cfg")).unwrap()).0;
        assert!(cfg.contains("[friendlyname]\r\nNeuer Name\r\n") && cfg.starts_with('x'), "{cfg}");
        // Ordner umbenennen: Objektordner und Verweise mit
        assert!(umbenennen(&root, "Probe", "a/b").is_err());
        umbenennen(&root, "Probe", "Probe2").unwrap();
        assert!(!root.join("maps/Probe").exists() && root.join("maps/Probe2/global.cfg").exists());
        assert!(root.join("Sceneryobjects/Aschaffenburg_KI/Probe2/K_1.sco").exists());
        let cfg = dekodieren(&std::fs::read(root.join("maps/Probe2/global.cfg")).unwrap()).0;
        assert!(cfg.contains("[name]\r\nProbe2\r\n"));
        let kachel = dekodieren(&std::fs::read(root.join("maps/Probe2/tile_0_0.map")).unwrap()).0;
        assert!(kachel.contains("Aschaffenburg_KI\\Probe2\\K_1.sco"), "{kachel}");
        // zweite Karte mit dem Zielnamen: nicht umbenennen
        testkarte(&root, "Probe3");
        assert!(umbenennen(&root, "Probe3", "Probe2").is_err());
        std::fs::remove_dir_all(&root).ok();
    }

    /// Fall des Nutzers (7.10.): Karte B (als neue Karte aus A gespeichert) verweist auf Kreuzungen im Ordner von A -
    /// nach dem Einsammeln haengt B nicht mehr von A ab; Umbenennen und Loeschen von A lassen B heil
    #[test]
    fn karte_haengt_nicht_von_ihrer_vorlage_ab() {
        let root = std::env::temp_dir().join(format!("omsi-editor-einsammeln-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        testkarte(&root, "A");
        std::fs::create_dir_all(root.join("Sceneryobjects/Aschaffenburg_KI/A/model")).unwrap();
        std::fs::write(root.join("Sceneryobjects/Aschaffenburg_KI/A/K_1.sco"), "[mesh]\r\nK_1.x\r\n").unwrap();
        std::fs::write(root.join("Sceneryobjects/Aschaffenburg_KI/A/model/K_1.x"), "Modell A").unwrap();
        // B: verweist auf A\K_1.sco und hat selbst ein anderes K_1.sco
        testkarte(&root, "B");
        std::fs::create_dir_all(root.join("Sceneryobjects/Aschaffenburg/B")).unwrap();
        std::fs::write(root.join("Sceneryobjects/Aschaffenburg/B/K_1.sco"), "[mesh]\r\nK_1.x\r\n(B)").unwrap();
        let utf16 = |t: &str| -> Vec<u8> { [0xFF, 0xFE].into_iter().chain(t.encode_utf16().flat_map(|u| u.to_le_bytes())).collect() };
        std::fs::write(root.join("maps/B/tile_0_1.map"), utf16("[object]\r\n0\r\nSceneryobjects\\Aschaffenburg_KI\\A\\K_1.sco\r\n9\r\n")).unwrap();
        assert_eq!(crate::speichern::fremde_objekte(&root, "B"), vec![("Aschaffenburg_KI".to_string(), "A".to_string(), "K_1.sco".to_string())]);
        // A loeschen geht nicht ohne Papierkorb im Test; daher: A umbenennen - B holt sich vorher die Objekte
        umbenennen(&root, "A", "A2").unwrap();
        assert!(crate::speichern::fremde_objekte(&root, "B").is_empty(), "B haengt noch von A ab");
        let kachel = dekodieren(&std::fs::read(root.join("maps/B/tile_0_1.map")).unwrap()).0;
        assert!(kachel.contains("Aschaffenburg\\B\\K_1_A.sco"), "{kachel}");
        // die geholte .sco zeigt auf ihr eigenes (umbenanntes) Modell, das vorhandene K_1.sco von B ist unveraendert
        let sco = std::fs::read_to_string(root.join("Sceneryobjects/Aschaffenburg/B/K_1_A.sco")).unwrap();
        assert!(sco.contains("K_1_A.x"), "{sco}");
        assert_eq!(std::fs::read_to_string(root.join("Sceneryobjects/Aschaffenburg/B/model/K_1_A.x")).unwrap(), "Modell A");
        assert!(std::fs::read_to_string(root.join("Sceneryobjects/Aschaffenburg/B/K_1.sco")).unwrap().contains("(B)"));
        // der Ordner von A (jetzt A2) kann weg, B bleibt vollstaendig
        std::fs::remove_dir_all(root.join("Sceneryobjects/Aschaffenburg_KI/A2")).unwrap();
        assert!(crate::speichern::fremde_objekte(&root, "B").is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn karten_der_installation() {
        let root = Path::new(crate::bearbeiten::tests::OMSI);
        if !root.exists() {
            return;
        }
        let k = finden(root);
        let g = k.iter().find(|k| k.ordner == "Grundorf").expect("Grundorf");
        assert!(g.standard && g.kacheln > 5 && !g.beschreibung.is_empty(), "{g:?}");
        // Vorschaubild (JPEG) laesst sich lesen
        let bild = image::open(g.bild.as_ref().expect("picture.jpg")).unwrap();
        assert!(bild.width() > 50);
    }

    /// verschiebt wirklich in den Papierkorb (nur Testdaten im Temp-Ordner)
    #[test]
    #[ignore]
    fn papierkorb_verschiebt() {
        let root = std::env::temp_dir().join(format!("omsi-editor-papierkorb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        testkarte(&root, "Weg");
        let w = in_papierkorb(&root, "Weg").unwrap();
        assert_eq!(w.len(), 2);
        assert!(!root.join("maps/Weg").exists() && !root.join("Sceneryobjects/Aschaffenburg_KI/Weg").exists());
        assert!(in_papierkorb(&root, "..").is_err());
        std::fs::remove_dir_all(&root).ok();
    }
}

#[cfg(test)]
mod neue_karte_tests {
    use super::*;
    use glam::{DVec2, DVec3};

    /// neue Karte in einem Test-Ordner (Vorlage und Querschnitt kopiert), mit openOMSI geoeffnet: die Grundkachel
    /// laedt, die Strasse hat Fahrspuren, der Einsetzpunkt steht in der rechten Spur und steht in global.cfg
    #[test]
    #[ignore]
    fn neue_karte_mit_strasse_und_einsetzpunkt() {
        let _sperre = crate::bearbeiten::tests::sperre();
        let root = Path::new(crate::bearbeiten::tests::OMSI);
        let test = std::env::temp_dir().join(format!("omsi-editor-neu-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&test);
        kopieren(&root.join("template").join("NewMap"), &test.join("template").join("NewMap")).unwrap();
        let sli = START_SLI[0].replace('\\', "/");
        std::fs::create_dir_all(test.join(&sli).parent().unwrap()).unwrap();
        std::fs::copy(root.join(&sli), test.join(&sli)).unwrap();
        let n = NeueKarte { ordner: ordner_aus("Neue Stadt Süd"), anzeige: "Neue Stadt Süd".into(), beschreibung: "Testkarte\nzweite Zeile".into(), ..Default::default() };
        assert_eq!(n.ordner, "Neue_Stadt_Sued");
        let karte = neue_karte(&test, &n).unwrap();
        assert!(neue_karte(&test, &n).is_err(), "zweimal derselbe Ordner");
        let g = omsi_map::GlobalCfg::load(&karte.join("global.cfg")).unwrap();
        assert_eq!((g.name.as_str(), g.friendly_name.as_str()), ("Neue_Stadt_Sued", "Neue Stadt Süd"));
        assert!(g.description.contains("zweite Zeile"), "{}", g.description);
        assert_eq!(g.entry_points.len(), 1);
        let ep = &g.entry_points[0];
        assert_eq!((ep.object_id, ep.index, ep.group, ep.name.as_str()), (2, 0, 0, "Start"));
        assert!(crate::speichern::eigene_karte(&test, &n.ordner));
        // mit openOMSI oeffnen (Inhalte aus der Installation)
        let (mut v, _) = openomsi_game::viewer::Viewer::open(&openomsi_game::viewer::instance(), None, root, &karte.join("global.cfg")).unwrap();
        v.tiles_around(DVec3::new(150.0, 150.0, 0.0), 1).unwrap();
        assert_eq!(v.map_tiles(), vec![(0, 0)]);
        assert!(v.spline_end_free(1, true).is_some(), "Strasse ohne Fahrspuren");
        let (p, h) = crate::orte::spur_bei(&v, DVec2::new(ep.pos[0], ep.pos[1])).expect("Einsetzpunkt nicht auf einer Fahrspur");
        assert!((p.truncate() - DVec2::new(ep.pos[0], ep.pos[1])).length() < 0.5 && crate::netz::norm180(h).abs() < 1.0, "{p:?} {h}");
        assert!(v.hidden_objects().iter().any(|o| o.0 == 2), "Einsetzpunkt-Objekt fehlt");
        if let Some(b) = std::env::var_os("OMSI_BILD") {
            let kam = crate::kamera::Kamera { ziel: DVec3::new(150.0, 120.0, 0.0), gier: 200.0, neigung: -40.0, abstand: 70.0, fov: 50.0 };
            let px = v.render_image(1280, 800, &kam.camera()).unwrap();
            image::save_buffer(b, &px, 1280, 800, image::ColorType::Rgba8).unwrap();
        }
        drop(v);
        std::fs::remove_dir_all(&test).ok();
    }

    /// (Netz) neue Karte in Aschaffenburg, 3 x 3 Kacheln: Georeferenz, echtes Gelaende, Strasse darauf; Luftbild ueber
    /// dem Gelaende (OMSI_BILD); eine Kachel anfuegen: ihr Gelaende passt zu den Nachbarn (gleiche Daten, gleiches Gitter)
    #[test]
    #[ignore]
    fn neue_karte_mit_ort() {
        let _sperre = crate::bearbeiten::tests::sperre();
        let root = Path::new(crate::bearbeiten::tests::OMSI);
        let test = std::env::temp_dir().join(format!("omsi-editor-ort-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&test);
        kopieren(&root.join("template").join("NewMap"), &test.join("template").join("NewMap")).unwrap();
        let sli = START_SLI[0].replace('\\', "/");
        std::fs::create_dir_all(test.join(&sli).parent().unwrap()).unwrap();
        std::fs::copy(root.join(&sli), test.join(&sli)).unwrap();
        let ort = crate::geo::Ort { name: "Aschaffenburg Schloss".into(), lat: 49.9755, lon: 9.1420 };
        let b = crate::geo::Bezug::fuer(&ort).unwrap();
        println!("{b:?}");
        let n = NeueKarte { ordner: "Aschaffenburg_Test".into(), anzeige: "Aschaffenburg Test".into(), bezug: Some(b.clone()), groesse: 3, ..Default::default() };
        let karte = neue_karte(&test, &n).unwrap();
        assert_eq!(crate::geo::Bezug::lesen(&karte), Some(b.clone()));
        let g = omsi_map::GlobalCfg::load(&karte.join("global.cfg")).unwrap();
        assert_eq!(g.tiles.len(), 9);
        assert_eq!((g.tiles[0].x, g.tiles[0].y), (0, 0));
        assert_eq!(g.entry_points.len(), 1);
        let t00 = omsi_map::Terrain::load(&karte.join("tile_0_0.map.terrain")).unwrap();
        let (lo, hi) = t00.heights.iter().fold((f32::MAX, f32::MIN), |(a, b), h| (a.min(*h), b.max(*h)));
        println!("Kachel 0 0: Gelaende {lo:.1} .. {hi:.1} m");
        assert!(hi - lo > 1.0, "Gelaende flach");
        let (mut v, _) = openomsi_game::viewer::Viewer::open(&openomsi_game::viewer::instance(), None, root, &karte.join("global.cfg")).unwrap();
        v.tiles_around(DVec3::new(150.0, 150.0, 0.0), 1).unwrap();
        assert_eq!(v.map_tiles().len(), 9);
        let (s, t) = (v.surface_height(150.0, 150.0).expect("Strasse"), v.terrain_height(150.0, 150.0).unwrap());
        println!("Strasse {s:.2} m, Gelaende darunter {t:.2} m");
        assert!((s - t).abs() < 0.5, "Strasse nicht auf dem Gelaende: {s} / {t}");
        assert!(v.spline_end_free(1, true).is_some(), "Strasse ohne Fahrspuren");
        // Luftbild
        let alle: Vec<(i32, i32)> = g.tiles.iter().map(|t| (t.x, t.y)).collect();
        let (daten, quellen) = crate::geo::kacheln(&b, &alle, false, true).unwrap();
        println!("{quellen:?}");
        let mut bilder = 0;
        for d in daten {
            if let Some(p) = d.luftbild {
                let img = image::open(&p).unwrap().to_rgba8();
                let (w, h) = img.dimensions();
                v.set_ground_image(d.kachel.0, d.kachel.1, Some(openomsi_game::viewer::Viewer::ground_image_data(omsi_texture::Image { width: w, height: h, rgba: img.into_raw(), has_alpha: false })));
                bilder += 1;
            }
        }
        assert_eq!(bilder, 9, "Luftbilder");
        v.set_ground_image_alpha(0.85);
        if let Some(bild) = std::env::var_os("OMSI_BILD") {
            for (name, kam) in [("uebersicht", crate::kamera::Kamera { ziel: DVec3::new(150.0, 150.0, 0.0), gier: 0.0, neigung: -70.0, abstand: 900.0, fov: 50.0 }),
                                ("nah", crate::kamera::Kamera { ziel: DVec3::new(150.0, 150.0, t), gier: 200.0, neigung: -35.0, abstand: 160.0, fov: 50.0 }),
                                ("oben", crate::kamera::Kamera { ziel: DVec3::new(150.0, 150.0, t), gier: 0.0, neigung: -89.9, abstand: 330.0, fov: 50.0 })] {
                let px = v.render_image(1280, 800, &kam.camera()).unwrap();
                image::save_buffer(std::path::PathBuf::from(&bild).with_extension(format!("{name}.png")), &px, 1280, 800, image::ColorType::Rgba8).unwrap();
            }
        }
        // Deckkraft: 100 % weicht am meisten vom Bild ohne Luftbild ab, 50 % etwa halb so viel
        let kam = crate::kamera::Kamera { ziel: DVec3::new(150.0, 150.0, t), gier: 0.0, neigung: -89.9, abstand: 330.0, fov: 50.0 };
        let mut bilder = Vec::new();
        for a in [0.0f32, 0.5, 1.0] {
            v.set_ground_image_alpha(a);
            bilder.push(v.render_image(320, 200, &kam.camera()).unwrap());
        }
        let abw = |x: &[u8], y: &[u8]| x.iter().zip(y).map(|(a, b)| (*a as f64 - *b as f64).abs()).sum::<f64>() / x.len() as f64;
        let (halb, voll) = (abw(&bilder[1], &bilder[0]), abw(&bilder[2], &bilder[0]));
        println!("Deckkraft: 50 % weicht {halb:.1} ab, 100 % {voll:.1}");
        assert!(voll > 5.0 && halb > voll * 0.3 && halb < voll * 0.7, "Deckkraft wirkt nicht: {halb} / {voll}");
        // erweitern: Kachel 2 0 mit dem echten Gelaende; schon die rohen Daten passen an den Rand der Kachel 1 0
        let a = crate::aendern::Aendern::neu(&v);
        let ordner = a.sitzung.join("maps").join("Aschaffenburg_Test");
        let (d, _) = crate::geo::kacheln(&b, &[(2, 0)], true, false).unwrap();
        let roh = d.into_iter().next().unwrap().gelaende.expect("Gelaende 2 0");
        let t10 = omsi_map::Terrain::load(&karte.join("tile_1_0.map.terrain")).unwrap();
        let fehler = (0..61).map(|iz| (roh[iz * 61] - t10.height_at(60, iz)).abs()).fold(0.0f32, f32::max);
        println!("Rand 1 0 / 2 0 (rohe Daten): hoechstens {fehler:.3} m");
        assert!(fehler < 0.05, "Gitter passt nicht: {fehler}");
        let mut w = crate::welt::Welt::default();
        println!("{}", w.hinzufuegen(&mut v, root, &ordner, (2, 0), Some(roh.clone())).unwrap());
        // (die Testkarte liegt nicht unter dem OMSI-Ordner: die Sitzungskopie direkt lesen)
        let neu = omsi_map::Terrain::load(&ordner.join("tile_2_0.map.terrain")).unwrap();
        for iz in 0..61 {
            assert_eq!(neu.height_at(0, iz), t10.height_at(60, iz));
        }
        assert!((neu.height_at(30, 30) - roh[30 * 61 + 30]).abs() < 1e-4);
        drop(a);
        drop(v);
        std::fs::remove_dir_all(&test).ok();
    }
}
