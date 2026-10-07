//! Speichern als neue Karte: der Kartenordner wird kopiert, die geaenderten Kacheln (von openOMSIs
//! Editor umgeschrieben, UTF-16 bleibt) kommen darueber, global.cfg bekommt den neuen Namen. Die Originalkarte
//! wird nie veraendert.

use crate::bearbeiten::Bearbeiten;
use anyhow::{bail, Context, Result};
use openomsi_game::viewer::{Editor, Viewer};
use std::path::{Path, PathBuf};

/// Reste frueherer Sitzungen im Temp-Ordner (aelter als einen Tag) loeschen
pub fn aufraeumen() {
    let d = std::env::temp_dir().join("omsi-editor");
    let Ok(rd) = std::fs::read_dir(&d) else { return };
    for e in rd.flatten() {
        let alt = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).map(|a| a.as_secs() > 86_400).unwrap_or(false);
        let n = e.file_name().to_string_lossy().to_string();
        if alt && (n.starts_with("sitzung-") || n.starts_with("speichern-")) {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

/// Markierungsdatei in Karten, die der Editor angelegt hat
pub const MARKE: &str = "omsi-editor.txt";

/// eigene Karte (vom Editor angelegt oder von omsigen erzeugt): darf ohne Rueckfrage ueberschrieben werden
pub fn eigene_karte(root: &Path, karte: &str) -> bool {
    let d = root.join("maps").join(karte);
    d.join(MARKE).exists() || d.join("LIESMICH_omsigen.txt").exists()
}

/// Ordner der Sicherungen beim Ueberschreiben
pub fn sicherungen() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("omsi-editor").join("sicherungen")
}

/// Teil 2 fuer "Speichern": die Karte selbst ueberschreiben. Jede ersetzte Datei (und global.cfg) kommt vorher nach
/// sicherungen()/<Karte>/<Zeit>/ (die letzten 10 bleiben); Kreuzungsobjekte in den Objektordner der Karte (nie
/// ueberschreibend). -> Ordner der Sicherung
pub fn karte_ueberschreiben(root: &Path, karte: &str, paket: &Paket) -> Result<PathBuf> {
    let ziel = root.join("maps").join(karte);
    if !ziel.join("global.cfg").exists() {
        bail!("{} ist keine Karte", ziel.display());
    }
    let objekte = root.join("Sceneryobjects").join("Aschaffenburg_KI").join(karte);
    // erst pruefen, dann schreiben: kein Kreuzungsobjekt darf ein vorhandenes ersetzen
    if let Some((ordner, _)) = &paket.kreuzungen {
        for e in std::fs::read_dir(ordner)?.flatten() {
            if e.path().is_file() && objekte.join(e.file_name()).exists() {
                bail!("{} gibt es schon - nichts geschrieben", objekte.join(e.file_name()).display());
            }
        }
    }
    let stempel = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let basis = sicherungen().join(karte);
    let sicherung = basis.join(format!("{stempel}"));
    std::fs::create_dir_all(&sicherung).with_context(|| format!("{} anlegen", sicherung.display()))?;
    let mut namen: Vec<std::ffi::OsString> = paket.dateien.iter().filter_map(|d| d.file_name().map(|n| n.to_owned())).collect();
    namen.push("global.cfg".into());
    for n in &namen {
        let alt = ziel.join(n);
        if alt.exists() {
            std::fs::copy(&alt, sicherung.join(n)).with_context(|| format!("{} sichern", alt.display()))?;
        }
    }
    for d in &paket.dateien {
        let name = d.file_name().context("Datei ohne Namen")?;
        std::fs::copy(d, ziel.join(name)).with_context(|| format!("{} schreiben", ziel.join(name).display()))?;
    }
    if let Some((ordner, tag)) = &paket.kreuzungen {
        ordner_zusammenfuehren(ordner, &objekte).with_context(|| format!("Kreuzungsobjekte nach {} kopieren", objekte.display()))?;
        let alt_rel = format!("Aschaffenburg_KI\\{tag}\\");
        let neu_rel = format!("Aschaffenburg_KI\\{karte}\\");
        for d in &paket.dateien {
            let datei = ziel.join(d.file_name().context("Datei ohne Namen")?);
            let (text, utf16) = dekodieren(&std::fs::read(&datei)?);
            if text.contains(&alt_rel) {
                std::fs::write(&datei, kodieren(&text.replace(&alt_rel, &neu_rel), utf16))?;
            }
        }
    }
    if let Some(id) = paket.naechste_id {
        global_setzen(&ziel.join("global.cfg"), &[("[NextIDCode]", id.to_string())])?;
    }
    let _ = std::fs::remove_dir_all(&paket.staging);
    // die letzten 10 Sicherungen behalten (eigene Ordner, nach Zeit benannt)
    let mut alle: Vec<PathBuf> = std::fs::read_dir(&basis).map(|r| r.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect()).unwrap_or_default();
    alle.sort();
    while alle.len() > 10 {
        let _ = std::fs::remove_dir_all(alle.remove(0));
    }
    log::info!("Karte {karte} gespeichert ({} Dateien), Sicherung in {}", paket.dateien.len(), sicherung.display());
    Ok(sicherung)
}

/// Dateien von `von` nach `nach` kopieren, vorhandene nicht ersetzen (Texturen sind gleich)
fn ordner_zusammenfuehren(von: &Path, nach: &Path) -> Result<()> {
    std::fs::create_dir_all(nach)?;
    for e in std::fs::read_dir(von)? {
        let e = e?;
        let p = e.path();
        if e.file_type()?.is_dir() {
            ordner_zusammenfuehren(&p, &nach.join(e.file_name()))?;
        } else if !nach.join(e.file_name()).exists() {
            std::fs::copy(&p, nach.join(e.file_name()))?;
        }
    }
    Ok(())
}

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
    // je Speichervorgang ein eigener Ordner (auch bei mehreren gleichzeitig, z. B. in Tests)
    static NR: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let nr = NR.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let staging = std::env::temp_dir().join("omsi-editor").join(format!("speichern-{}-{nr}", std::process::id()));
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
    pub neue_splines: usize,
    /// Zwischenordner (wird nach dem Anlegen der Karte geloescht)
    pub staging: PathBuf,
    /// Kreuzungsobjekte der Sitzung: (Ordner, Name unter Sceneryobjects/Aschaffenburg_KI) - kommen in den Ordner
    /// der neuen Karte
    pub kreuzungen: Option<(PathBuf, String)>,
}

/// ein Eintrag fuer eine Kachel: neues Objekt oder neuer Spline (in Zeilen, ohne Zeilenende)
enum Eintrag {
    Objekt(Vec<String>),
    Spline(Vec<String>),
}

/// Teil 1 komplett: Aenderungen der Kartenobjekte (openOMSI) und neue Objekte in die Kacheln schreiben
#[allow(clippy::too_many_arguments)]
pub fn vorbereiten(v: &Viewer, b: &Bearbeiten, netz: &crate::netz::Netz, netz_kreuzungen: &[crate::kreuzung::Gesetzt],
                   kopien: &[PathBuf], kreuzungen: Option<(PathBuf, String)>, alt: &str) -> Result<Paket> {
    let (staging, mut dateien) = kacheln_schreiben(v, &b.ed, alt)?;
    let ordner = staging.join("maps").join(alt);
    std::fs::create_dir_all(&ordner)?;
    // vom Aendern-Werkzeug umgeschriebene Kacheln (Sitzungskopien), soweit nicht schon oben geschrieben
    for k in kopien {
        let Some(name) = k.file_name() else { continue };
        let ziel = ordner.join(name);
        if !ziel.exists() {
            std::fs::copy(k, &ziel)?;
            dateien.push(ziel);
        }
    }
    let groesse = omsi_map::tile_size();
    let mut id = v.next_object_id();
    let start = id;
    // alles nach Kacheln sammeln, damit jede Datei einmal gelesen/geschrieben wird
    let kachel = |x: f64, y: f64| ((x / groesse).floor() as i32, (y / groesse).floor() as i32);
    let mut je_kachel: std::collections::BTreeMap<(i32, i32), Vec<Eintrag>> = Default::default();
    let mut neue_objekte = 0;
    for n in b.neue.iter().filter(|n| !n.z.geloescht) {
        let (tx, ty) = kachel(n.z.pos.x, n.z.pos.y);
        let abs = std::fs::read(v.root.join(&n.rel)).map(|b| dekodieren(&b).0.to_ascii_lowercase().contains("[absheight]")).unwrap_or(false);
        let hoehe = if abs { n.z.pos.z } else { n.z.ueber_boden };
        let felder = vec![
            "[object]".to_string(), "0".into(), n.rel.clone(), id.to_string(),
            zahl(n.z.pos.x - tx as f64 * groesse), zahl(n.z.pos.y - ty as f64 * groesse), zahl(hoehe),
            zahl(n.z.richtung.rem_euclid(360.0)), "0".into(), "0".into(), "0".into(),
        ];
        je_kachel.entry((tx, ty)).or_default().push(Eintrag::Objekt(felder));
        id += 1;
        neue_objekte += 1;
    }
    // Kreuzungsobjekte des eigenen Netzes ([absheight]: Hoehe absolut) mit den Vorfahrtregeln ihrer Pfade
    for g in netz_kreuzungen {
        let pos = g.pos;
        let (tx, ty) = kachel(pos.x, pos.y);
        let mut felder = vec![
            "[object]".to_string(), "0".into(), g.rel.clone(), id.to_string(),
            zahl(pos.x - tx as f64 * groesse), zahl(pos.y - ty as f64 * groesse), zahl(pos.z), "0".into(), "0".into(), "0".into(), "0".into(),
        ];
        for (pfad, wert) in &g.rules {
            felder.extend(["".to_string(), "[rule]".into(), pfad.to_string(), "priority".into(), wert.to_string(), "0".into()]);
        }
        je_kachel.entry((tx, ty)).or_default().push(Eintrag::Objekt(felder));
        let kreuzung_id = id;
        id += 1;
        neue_objekte += 1;
        // Ampeln: Signal (Text = Signalgruppe, [varparent] = Kreuzung), Mast, Signal am Ausleger ([attachObj] am Mast) -
        // in der Kachel der Kreuzung, in dieser Reihenfolge (wie omsigen writer.py und die Standardkarten)
        let mut ids: Vec<i64> = Vec::new();
        for sg in &g.signale {
            let mut f = if sg.art == "oben" {
                let eltern = sg.eltern.and_then(|i| ids.get(i).copied()).unwrap_or(kreuzung_id);
                vec!["[attachObj]".to_string(), "0".into(), sg.datei.clone(), id.to_string(), eltern.to_string(), "0".into(),
                     sg.anhang.to_string(), zahl(sg.rot), "0".into(), "0".into()]
            } else {
                let p = sg.pos.unwrap_or(pos.truncate());
                vec!["[object]".to_string(), "0".into(), sg.datei.clone(), id.to_string(), zahl(p.x - tx as f64 * groesse),
                     zahl(p.y - ty as f64 * groesse), zahl(sg.hoehe), zahl(sg.rot), "0".into(), "0".into()]
            };
            match sg.gruppe {
                Some(gr) => f.extend(["1".to_string(), gr.to_string(), "".into(), "[varparent]".into(), kreuzung_id.to_string()]),
                None => f.push("0".into()),
            }
            je_kachel.entry((tx, ty)).or_default().push(Eintrag::Objekt(f));
            ids.push(id);
            id += 1;
            neue_objekte += 1;
        }
    }
    // Strassen: jede Kante als Kette von [spline_h] (Hoehenunterschied nach den Steigungen, wie Omsi.exe liest)
    let mut neue_splines = 0;
    for e in &netz.kanten {
        let el = netz.elemente(e);
        let ids: Vec<i64> = (0..el.len() as i64).map(|i| id + i).collect();
        id += el.len() as i64;
        let mut cum = 0.0;
        for (i, x) in el.iter().enumerate() {
            let (tx, ty) = kachel(x.stueck.start.x, x.stueck.start.y);
            let felder = vec![
                "[spline_h]".to_string(), "0".into(), e.sli.clone(), ids[i].to_string(),
                if i > 0 { ids[i - 1].to_string() } else { "0".into() },
                if i + 1 < ids.len() { ids[i + 1].to_string() } else { "0".into() },
                zahl(x.stueck.start.x - tx as f64 * groesse), zahl(x.z), zahl(x.stueck.start.y - ty as f64 * groesse),
                zahl(x.stueck.richtung.rem_euclid(360.0)), zahl(x.stueck.laenge), zahl(x.stueck.radius),
                zahl(x.stg_a), zahl(x.stg_e), zahl(x.dh), "0".into(), "0".into(), "0".into(), "0".into(), zahl(cum),
            ];
            cum += x.stueck.laenge;
            je_kachel.entry((tx, ty)).or_default().push(Eintrag::Spline(felder));
            neue_splines += 1;
        }
    }
    for ((tx, ty), liste) in je_kachel {
        let quelle = v.tile_file(tx, ty).with_context(|| format!("Kachel {tx} {ty}: die Karte hat dort keine Kachel (Strasse/Objekt ausserhalb)"))?;
        let name = quelle.file_name().context("Kachel ohne Namen")?.to_owned();
        let ziel = ordner.join(&name);
        let bytes = if ziel.exists() { std::fs::read(&ziel)? } else { std::fs::read(&quelle)? };
        let (mut text, utf16) = dekodieren(&bytes);
        let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
        if !text.ends_with('\n') {
            text.push_str(eol);
        }
        // Splines stehen in OMSI-Kacheln vor den Objekten; ans Ende geschrieben liest Omsi.exe sie genauso
        for eintrag in liste {
            let (Eintrag::Objekt(f) | Eintrag::Spline(f)) = eintrag;
            text.push_str(eol);
            for z in f {
                text.push_str(&z);
                text.push_str(eol);
            }
        }
        std::fs::write(&ziel, kodieren(&text, utf16))?;
        if !dateien.contains(&ziel) {
            dateien.push(ziel);
        }
    }
    // nur, wenn es Kreuzungen gibt
    let kreuzungen = kreuzungen.filter(|(d, _)| std::fs::read_dir(d).map(|r| r.flatten().any(|e| e.path().extension().is_some_and(|x| x == "sco"))).unwrap_or(false));
    Ok(Paket { dateien, naechste_id: (id > start).then_some(id), neue_objekte, neue_splines, staging, kreuzungen })
}

/// alles in einem Schritt (Tests)
#[cfg(test)]
pub fn alles_speichern(v: &Viewer, b: &Bearbeiten, netz: &crate::netz::Netz, alt: &str, neu: &str, root: &Path) -> Result<PathBuf> {
    let p = vorbereiten(v, b, netz, &[], &[], None, alt)?;
    karte_anlegen(root, alt, neu, &p)
}

fn zahl(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

pub(crate) fn dekodieren(bytes: &[u8]) -> (String, bool) {
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let u: Vec<u16> = bytes[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        (String::from_utf16_lossy(&u), true)
    } else {
        (bytes.iter().map(|&b| b as char).collect::<String>(), false)
    }
}

pub(crate) fn kodieren(text: &str, utf16: bool) -> Vec<u8> {
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
    let objekte = root.join("Sceneryobjects").join("Aschaffenburg_KI").join(neu);
    if paket.kreuzungen.is_some() && objekte.exists() {
        bail!("{} gibt es schon - anderen Namen waehlen", objekte.display());
    }
    ordner_kopieren(&quelle, &ziel).with_context(|| format!("{} nach {} kopieren", quelle.display(), ziel.display()))?;
    for d in dateien {
        let name = d.file_name().context("Datei ohne Namen")?;
        std::fs::copy(d, ziel.join(name)).with_context(|| format!("{} schreiben", ziel.join(name).display()))?;
    }
    // Kreuzungsobjekte in den Ordner der neuen Karte, die Kacheln verweisen dann dorthin
    if let Some((ordner, tag)) = &paket.kreuzungen {
        ordner_kopieren(ordner, &objekte).with_context(|| format!("Kreuzungsobjekte nach {} kopieren", objekte.display()))?;
        let alt_rel = format!("Aschaffenburg_KI\\{tag}\\");
        let neu_rel = format!("Aschaffenburg_KI\\{neu}\\");
        for d in dateien {
            let datei = ziel.join(d.file_name().context("Datei ohne Namen")?);
            let (text, utf16) = dekodieren(&std::fs::read(&datei)?);
            if text.contains(&alt_rel) {
                std::fs::write(&datei, kodieren(&text.replace(&alt_rel, &neu_rel), utf16))?;
            }
        }
    }
    let mut werte = vec![("[name]", neu.to_string())];
    if let Some(id) = paket.naechste_id {
        werte.push(("[NextIDCode]", id.to_string()));
    }
    global_setzen(&ziel.join("global.cfg"), &werte)?;
    let _ = std::fs::write(ziel.join(MARKE), format!("Mit dem OMSI-Editor (aschaffenburgmap) angelegt aus der Karte \"{alt}\".\r\nDiese Karte darf der Editor beim Speichern ueberschreiben (mit Sicherung).\r\n"));
    let _ = std::fs::remove_dir_all(&paket.staging);
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
