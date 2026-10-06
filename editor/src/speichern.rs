//! Speichern als neue Karte: der Kartenordner wird kopiert, die geaenderten Kacheln (von openOMSIs
//! Editor umgeschrieben, UTF-16 bleibt) kommen darueber, global.cfg bekommt den neuen Namen. Die Originalkarte
//! wird nie veraendert.

use anyhow::{bail, Context, Result};
use openomsi_game::viewer::{Editor, Viewer};
use std::path::{Path, PathBuf};

/// Kartenname fuer einen neuen Ordner: Buchstaben, Ziffern, _ - und Leerzeichen
pub fn name_ok(n: &str) -> bool {
    !n.trim().is_empty() && n.chars().all(|c| c.is_alphanumeric() || "_- ".contains(c)) && !n.starts_with(' ')
}

/// Vorschlag: <alt>_editor, sonst _editor2, ...
pub fn vorschlag(root: &Path, alt: &str) -> String {
    let basis = format!("{alt}_editor");
    if !root.join("maps").join(&basis).exists() {
        return basis;
    }
    (2..).map(|i| format!("{basis}{i}")).find(|n| !root.join("maps").join(n).exists()).unwrap()
}

/// Teil 1 (braucht die Welt): geaenderte Kacheln in einen Zwischenordner schreiben -> Dateien
pub fn kacheln_schreiben(v: &Viewer, ed: &Editor, alt: &str) -> Result<(PathBuf, Vec<PathBuf>)> {
    let staging = std::env::temp_dir().join("omsi-editor").join(format!("speichern-{}", std::process::id()));
    if staging.exists() {
        std::fs::remove_dir_all(&staging).ok();
    }
    std::fs::create_dir_all(&staging)?;
    let dateien = v.save_edits(ed, &format!("maps/{alt}/global.cfg"), &staging).map_err(|e| anyhow::anyhow!(e))?;
    Ok((staging, dateien))
}

/// Teil 2 (darf im Hintergrund laufen): Ordner kopieren, Kacheln darueber, Namen setzen -> neuer Ordner
pub fn karte_anlegen(root: &Path, alt: &str, neu: &str, dateien: &[PathBuf]) -> Result<PathBuf> {
    if !name_ok(neu) {
        bail!("Kartenname \"{neu}\": nur Buchstaben, Ziffern, _ - und Leerzeichen");
    }
    let quelle = root.join("maps").join(alt);
    let ziel = root.join("maps").join(neu);
    if ziel.exists() {
        bail!("{} gibt es schon - anderen Namen waehlen (vorhandene Karten werden nie ueberschrieben)", ziel.display());
    }
    ordner_kopieren(&quelle, &ziel).with_context(|| format!("{} nach {} kopieren", quelle.display(), ziel.display()))?;
    for d in dateien {
        let name = d.file_name().context("Datei ohne Namen")?;
        std::fs::copy(d, ziel.join(name)).with_context(|| format!("{} schreiben", ziel.join(name).display()))?;
    }
    name_setzen(&ziel.join("global.cfg"), neu)?;
    Ok(ziel)
}

fn ordner_kopieren(von: &Path, nach: &Path) -> Result<()> {
    std::fs::create_dir_all(nach)?;
    for e in std::fs::read_dir(von)? {
        let e = e?;
        let p = e.path();
        if e.file_type()?.is_dir() {
            ordner_kopieren(&p, &nach.join(e.file_name()))?;
        } else {
            std::fs::copy(&p, nach.join(e.file_name()))?;
        }
    }
    Ok(())
}

/// global.cfg: Zeile nach [name] auf den neuen Namen setzen; Kodierung (UTF-16 LE mit BOM oder 8 Bit) bleibt
pub fn name_setzen(global: &Path, neu: &str) -> Result<()> {
    let bytes = std::fs::read(global)?;
    let (text, utf16) = if bytes.starts_with(&[0xFF, 0xFE]) {
        let u: Vec<u16> = bytes[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        (String::from_utf16_lossy(&u), true)
    } else {
        (bytes.iter().map(|&b| b as char).collect::<String>(), false)
    };
    let mut out = String::with_capacity(text.len() + 32);
    let mut naechste = false;
    let mut gesetzt = false;
    for zeile in text.split_inclusive('\n') {
        let inhalt = zeile.trim_end_matches(['\r', '\n']);
        let ende = &zeile[inhalt.len()..];
        if naechste {
            out.push_str(neu);
            out.push_str(ende);
            naechste = false;
            gesetzt = true;
            continue;
        }
        if inhalt.trim().eq_ignore_ascii_case("[name]") && !gesetzt {
            naechste = true;
        }
        out.push_str(zeile);
    }
    let daten: Vec<u8> = if utf16 {
        [0xFF, 0xFE].into_iter().chain(out.encode_utf16().flat_map(|u| u.to_le_bytes())).collect()
    } else {
        out.chars().map(|c| c as u32 as u8).collect()
    };
    std::fs::write(global, daten)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_in_utf16_global_cfg() {
        let d = std::env::temp_dir().join(format!("omsi-editor-test-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let g = d.join("global.cfg");
        let text = "[friendlyname]\r\nAlt\r\n\r\n[name]\r\nAlt\r\n\r\n[version]\r\n14\r\n";
        let b: Vec<u8> = [0xFF, 0xFE].into_iter().chain(text.encode_utf16().flat_map(|u| u.to_le_bytes())).collect();
        std::fs::write(&g, b).unwrap();
        name_setzen(&g, "Neu_1").unwrap();
        let r = std::fs::read(&g).unwrap();
        assert_eq!(&r[..2], &[0xFF, 0xFE]);
        let u: Vec<u16> = r[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let t = String::from_utf16(&u).unwrap();
        assert!(t.contains("[name]\r\nNeu_1\r\n") && t.contains("[friendlyname]\r\nAlt\r\n"), "{t}");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn namen() {
        assert!(name_ok("Grundorf_editor2") && name_ok("Aschaffenburg-Hbf v2"));
        assert!(!name_ok("") && !name_ok("a/b") && !name_ok("..\\x"));
    }
}
