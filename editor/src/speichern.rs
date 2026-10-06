//! Speichern als neue Karte: der Kartenordner wird kopiert, die geaenderten Kacheln (von openOMSIs
//! Editor umgeschrieben, UTF-16 bleibt) kommen darueber, global.cfg bekommt den neuen Namen. Die Originalkarte
//! wird nie veraendert.

use crate::bearbeiten::Bearbeiten;
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

/// Was nach dem Vorbereiten (braucht die Welt) im Hintergrund geschrieben wird
pub struct Paket {
    pub dateien: Vec<PathBuf>,
    /// neuer Wert fuer [NextIDCode], wenn neue Objekte dazukamen
    pub naechste_id: Option<i64>,
    pub neue_objekte: usize,
}

/// Teil 1 komplett: Aenderungen der Kartenobjekte (openOMSI) und neue Objekte in die Kacheln schreiben
pub fn vorbereiten(v: &Viewer, b: &Bearbeiten, alt: &str) -> Result<Paket> {
    let (staging, mut dateien) = kacheln_schreiben(v, &b.ed, alt)?;
    let ordner = staging.join("maps").join(alt);
    std::fs::create_dir_all(&ordner)?;
    let groesse = omsi_map::tile_size();
    let mut id = v.next_object_id();
    let start = id;
    // nach Kacheln sortiert, damit jede Datei einmal gelesen/geschrieben wird
    let mut je_kachel: std::collections::BTreeMap<(i32, i32), Vec<&crate::bearbeiten::Neu>> = Default::default();
    for n in b.neue.iter().filter(|n| !n.z.geloescht) {
        let k = ((n.z.pos.x / groesse).floor() as i32, (n.z.pos.y / groesse).floor() as i32);
        je_kachel.entry(k).or_default().push(n);
    }
    for ((tx, ty), liste) in je_kachel {
        let quelle = v.tile_file(tx, ty).with_context(|| format!("Objekt bei Kachel {tx} {ty}: die Karte hat dort keine Kachel"))?;
        let name = quelle.file_name().context("Kachel ohne Namen")?.to_owned();
        let ziel = ordner.join(&name);
        let bytes = if ziel.exists() { std::fs::read(&ziel)? } else { std::fs::read(&quelle)? };
        let (mut text, utf16) = dekodieren(&bytes);
        let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
        if !text.ends_with('\n') {
            text.push_str(eol);
        }
        for n in liste {
            let abs = std::fs::read(v.root.join(&n.rel)).map(|b| dekodieren(&b).0.to_ascii_lowercase().contains("[absheight]")).unwrap_or(false);
            let hoehe = if abs { n.z.pos.z } else { n.z.ueber_boden };
            let felder = [
                "[object]".to_string(),
                "0".into(),
                n.rel.clone(),
                id.to_string(),
                zahl(n.z.pos.x - tx as f64 * groesse),
                zahl(n.z.pos.y - ty as f64 * groesse),
                zahl(hoehe),
                zahl(n.z.richtung.rem_euclid(360.0)),
                "0".into(),
                "0".into(),
                "0".into(),
            ];
            text.push_str(eol);
            for f in felder {
                text.push_str(&f);
                text.push_str(eol);
            }
            id += 1;
        }
        std::fs::write(&ziel, kodieren(&text, utf16))?;
        if !dateien.contains(&ziel) {
            dateien.push(ziel);
        }
    }
    Ok(Paket { dateien, naechste_id: (id > start).then_some(id), neue_objekte: (id - start) as usize })
}

/// alles in einem Schritt (Tests)
#[cfg(test)]
pub fn alles_speichern(v: &Viewer, b: &Bearbeiten, alt: &str, neu: &str, root: &Path) -> Result<PathBuf> {
    let p = vorbereiten(v, b, alt)?;
    karte_anlegen(root, alt, neu, &p)
}

fn zahl(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

fn dekodieren(bytes: &[u8]) -> (String, bool) {
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let u: Vec<u16> = bytes[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        (String::from_utf16_lossy(&u), true)
    } else {
        (bytes.iter().map(|&b| b as char).collect::<String>(), false)
    }
}

fn kodieren(text: &str, utf16: bool) -> Vec<u8> {
    if utf16 {
        [0xFF, 0xFE].into_iter().chain(text.encode_utf16().flat_map(|u| u.to_le_bytes())).collect()
    } else {
        text.chars().map(|c| c as u32 as u8).collect()
    }
}

/// Teil 2 (darf im Hintergrund laufen): Ordner kopieren, Kacheln darueber, Namen setzen -> neuer Ordner
pub fn karte_anlegen(root: &Path, alt: &str, neu: &str, paket: &Paket) -> Result<PathBuf> {
    let dateien = &paket.dateien;
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
    let mut werte = vec![("[name]", neu.to_string())];
    if let Some(id) = paket.naechste_id {
        werte.push(("[NextIDCode]", id.to_string()));
    }
    global_setzen(&ziel.join("global.cfg"), &werte)?;
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

/// global.cfg: Zeile nach [name] auf den neuen Namen setzen; Kodierung bleibt
#[cfg(test)]
pub fn name_setzen(global: &Path, neu: &str) -> Result<()> {
    global_setzen(global, &[("[name]", neu.to_string())])
}

/// global.cfg: die Zeile nach jedem der Schluessel ersetzen (erstes Vorkommen); Kodierung (UTF-16 LE mit BOM oder
/// 8 Bit) und Zeilenenden bleiben
pub fn global_setzen(global: &Path, werte: &[(&str, String)]) -> Result<()> {
    let (text, utf16) = dekodieren(&std::fs::read(global)?);
    let mut out = String::with_capacity(text.len() + 32);
    let mut naechste: Option<usize> = None;
    let mut erledigt = vec![false; werte.len()];
    for zeile in text.split_inclusive('\n') {
        let inhalt = zeile.trim_end_matches(['\r', '\n']);
        let ende = &zeile[inhalt.len()..];
        if let Some(i) = naechste.take() {
            out.push_str(&werte[i].1);
            out.push_str(ende);
            erledigt[i] = true;
            continue;
        }
        if let Some(i) = werte.iter().position(|(k, _)| inhalt.trim().eq_ignore_ascii_case(k)) {
            if !erledigt[i] {
                naechste = Some(i);
            }
        }
        out.push_str(zeile);
    }
    std::fs::write(global, kodieren(&out, utf16))?;
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
