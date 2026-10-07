//! Kartenverwaltung: die Karten der Installation (Ordner, Anzeigename, Beschreibung, Vorschaubild), Anzeigename und
//! Ordnername aendern, Karte in den Papierkorb verschieben.
//!
//! Eine OMSI-Karte hat zwei Namen: den Ordner unter maps\ (steht auch in global.cfg unter [name]; Spielstaende und
//! die Objektordner unter Aschaffenburg_KI\ verweisen darauf) und den Anzeigenamen [friendlyname] in global.cfg und
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

/// Ordner einer Karte umbenennen: maps\alt -> maps\neu, [name] in global.cfg, die eigenen Ordner unter
/// Sceneryobjects\Aschaffenburg_KI\ und Splines\Aschaffenburg_KI\ und die Verweise der Kacheln darauf
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
        .map(|b| (root.join(b).join("Aschaffenburg_KI").join(alt), root.join(b).join("Aschaffenburg_KI").join(neu)))
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
        let (alt_rel, neu_rel) = (format!("Aschaffenburg_KI\\{alt}\\"), format!("Aschaffenburg_KI\\{neu}\\"));
        for e in std::fs::read_dir(&nach)?.flatten() {
            let n = e.file_name().to_string_lossy().to_lowercase();
            if !(n.starts_with("tile_") && n.ends_with(".map")) {
                continue;
            }
            let (text, utf16) = dekodieren(&std::fs::read(e.path())?);
            if text.contains(&alt_rel) {
                std::fs::write(e.path(), kodieren(&text.replace(&alt_rel, &neu_rel), utf16))?;
            }
        }
    }
    log::info!("Karte {alt} umbenannt in {neu} ({} eigene Ordner mit)", eigene.len());
    Ok(())
}

/// Karte in den Windows-Papierkorb verschieben (mit ihren eigenen Ordnern unter Aschaffenburg_KI\) -> was verschoben
/// wurde. Geloescht wird nichts endgueltig.
pub fn in_papierkorb(root: &Path, ordner: &str) -> Result<Vec<PathBuf>> {
    let d = root.join("maps").join(ordner);
    if !d.join("global.cfg").is_file() || ordner.trim().is_empty() || ordner.contains(['\\', '/']) || ordner.contains("..") {
        bail!("{} ist keine Karte", d.display());
    }
    let mut wege = vec![d];
    for b in ["Sceneryobjects", "Splines"] {
        let e = root.join(b).join("Aschaffenburg_KI").join(ordner);
        if e.is_dir() {
            wege.push(e);
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
        std::fs::write(root.join("Sceneryobjects/Aschaffenburg_KI/B/K_1.sco"), "[mesh]\r\nK_1.x\r\n(B)").unwrap();
        let utf16 = |t: &str| -> Vec<u8> { [0xFF, 0xFE].into_iter().chain(t.encode_utf16().flat_map(|u| u.to_le_bytes())).collect() };
        std::fs::write(root.join("maps/B/tile_0_1.map"), utf16("[object]\r\n0\r\nSceneryobjects\\Aschaffenburg_KI\\A\\K_1.sco\r\n9\r\n")).unwrap();
        assert_eq!(crate::speichern::fremde_objekte(&root, "B"), vec![("A".to_string(), "K_1.sco".to_string())]);
        // A loeschen geht nicht ohne Papierkorb im Test; daher: A umbenennen - B holt sich vorher die Objekte
        umbenennen(&root, "A", "A2").unwrap();
        assert!(crate::speichern::fremde_objekte(&root, "B").is_empty(), "B haengt noch von A ab");
        let kachel = dekodieren(&std::fs::read(root.join("maps/B/tile_0_1.map")).unwrap()).0;
        assert!(kachel.contains("Aschaffenburg_KI\\B\\K_1_A.sco"), "{kachel}");
        // die geholte .sco zeigt auf ihr eigenes (umbenanntes) Modell, das vorhandene K_1.sco von B ist unveraendert
        let sco = std::fs::read_to_string(root.join("Sceneryobjects/Aschaffenburg_KI/B/K_1_A.sco")).unwrap();
        assert!(sco.contains("K_1_A.x"), "{sco}");
        assert_eq!(std::fs::read_to_string(root.join("Sceneryobjects/Aschaffenburg_KI/B/model/K_1_A.x")).unwrap(), "Modell A");
        assert!(std::fs::read_to_string(root.join("Sceneryobjects/Aschaffenburg_KI/B/K_1.sco")).unwrap().contains("(B)"));
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
