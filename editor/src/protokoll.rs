//! Protokoll, Absturz- und Hänger-Berichte.
//!
//! - Logdatei: %LOCALAPPDATA%\omsi-editor\logs\omsi-editor-<Zeit>.log (die letzten 10 bleiben), jede Zeile sofort
//!   geschrieben – auch bei einem Absturz ist alles drin. Stufe: RUST_LOG (Standard: info für den Editor, warn sonst).
//! - Absturz (panic): absturz-<Zeit>.txt mit Meldung, Stelle, Backtrace und den letzten Protokollzeilen.
//! - Hänger: ein Wächter-Thread merkt, wenn die Hauptschleife länger als HAENGER_S nicht weiterkommt, und schreibt
//!   haenger-<Zeit>.txt mit der laufenden Aktion (`aktion(...)`) und den letzten Protokollzeilen.
//! Beim nächsten Start meldet die Statuszeile einen neuen Bericht.

use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const HAENGER_S: u64 = 8;
const LETZTE: usize = 300;

struct Zustand {
    datei: Option<std::fs::File>,
    letzte: VecDeque<String>,
    aktion: String,
    /// die Aktion darf lange dauern (Karte öffnen: Shader übersetzen ...)
    lang: bool,
}

static ZUSTAND: Mutex<Option<Zustand>> = Mutex::new(None);
static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
/// Millisekunden seit Start, zu denen die Hauptschleife zuletzt lief
static PULS: AtomicU64 = AtomicU64::new(0);

/// Ordner der Protokolle und Berichte
pub fn ordner() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("omsi-editor").join("logs")
}

fn ms() -> u64 {
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// Datum/Uhrzeit (UTC) für Dateinamen: 20261006-184512
fn zeitstempel() -> String {
    let s = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (tage, rest) = ((s / 86400) as i64, s % 86400);
    // Tage seit 1970 -> Datum (Howard Hinnant, civil_from_days)
    let z = tage + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let tag = doy - (153 * mp + 2) / 5 + 1;
    let monat = if mp < 10 { mp + 3 } else { mp - 9 };
    let jahr = yoe + era * 400 + if monat <= 2 { 1 } else { 0 };
    format!("{jahr:04}{monat:02}{tag:02}-{:02}{:02}{:02}", rest / 3600, rest % 3600 / 60, rest % 60)
}

fn zeile(text: String) {
    eprintln!("{text}");
    if let Ok(mut g) = ZUSTAND.lock() {
        if let Some(z) = g.as_mut() {
            if let Some(f) = z.datei.as_mut() {
                let _ = writeln!(f, "{text}");
            }
            if z.letzte.len() >= LETZTE {
                z.letzte.pop_front();
            }
            z.letzte.push_back(text);
        }
    }
}

struct Logger {
    filter: env_filter::Filter,
}

impl log::Log for Logger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        self.filter.enabled(m)
    }
    fn log(&self, r: &log::Record) {
        if self.filter.matches(r) {
            let t = ms();
            zeile(format!("[{:>7}.{:03}] {:<5} {}: {}", t / 1000, t % 1000, r.level(), r.target(), r.args()));
        }
    }
    fn flush(&self) {}
}

/// Protokoll, Absturzbericht und Hänger-Wächter einrichten -> Pfad der Logdatei
pub fn starten() -> Option<PathBuf> {
    START.get_or_init(Instant::now);
    let d = ordner();
    let _ = std::fs::create_dir_all(&d);
    aufraeumen(&d);
    let pfad = d.join(format!("omsi-editor-{}.log", zeitstempel()));
    let datei = std::fs::File::create(&pfad).ok();
    *ZUSTAND.lock().unwrap() = Some(Zustand { datei, letzte: VecDeque::new(), aktion: String::new(), lang: false });
    let filter = env_filter::Builder::new()
        .parse(&std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,omsi_editor=info".into()))
        .build();
    let stufe = filter.filter();
    if log::set_boxed_logger(Box::new(Logger { filter })).is_ok() {
        log::set_max_level(stufe);
    }
    std::panic::set_hook(Box::new(|info| {
        let bt = std::backtrace::Backtrace::force_capture();
        let ort = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let text = info.payload().downcast_ref::<&str>().map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned()).unwrap_or_default();
        let thread = std::thread::current().name().unwrap_or("?").to_string();
        zeile(format!("ABSTURZ im Thread {thread} bei {ort}: {text}"));
        bericht("absturz", &format!("Absturz im Thread {thread}\nStelle: {ort}\nMeldung: {text}\n\nBacktrace:\n{bt}"));
    }));
    std::thread::Builder::new().name("waechter".into()).spawn(waechter).ok();
    log::info!("omsi-editor {} gestartet, Protokoll {}", env!("CARGO_PKG_VERSION"), pfad.display());
    Some(pfad)
}

/// Bericht mit Kopf, Text, laufender Aktion und den letzten Protokollzeilen -> Pfad
fn bericht(art: &str, text: &str) -> Option<PathBuf> {
    let (aktion, letzte) = ZUSTAND.lock().ok()
        .and_then(|g| g.as_ref().map(|z| (z.aktion.clone(), z.letzte.iter().cloned().collect::<Vec<_>>())))
        .unwrap_or_default();
    let pfad = ordner().join(format!("{art}-{}.txt", zeitstempel()));
    let inhalt = format!(
        "omsi-editor {} - {art}\nSystem: {} {}\nlaufende Aktion: {}\n\n{text}\n\nLetzte Protokollzeilen:\n{}\n",
        env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH,
        if aktion.is_empty() { "-" } else { &aktion }, letzte.join("\n"));
    std::fs::write(&pfad, inhalt).ok()?;
    eprintln!("Bericht: {}", pfad.display());
    Some(pfad)
}

/// Die Hauptschleife lebt (einmal je Bild aufrufen)
pub fn puls() {
    PULS.store(ms(), Ordering::Relaxed);
}

/// Was gerade läuft (für Hänger-Berichte); leer: nichts Besonderes
pub fn aktion(text: &str) {
    setzen(text, false);
}

/// wie `aktion`, darf aber bis 60 s dauern, ohne als Hänger zu gelten
pub fn aktion_lang(text: &str) {
    setzen(text, true);
}

fn setzen(text: &str, lang: bool) {
    if let Ok(mut g) = ZUSTAND.lock() {
        if let Some(z) = g.as_mut() {
            z.aktion = text.to_string();
            z.lang = lang;
        }
    }
    if !text.is_empty() {
        log::info!("Aktion: {text}");
    }
}

fn waechter() {
    let mut gemeldet = false;
    loop {
        std::thread::sleep(Duration::from_secs(1));
        let puls = PULS.load(Ordering::Relaxed);
        if puls == 0 {
            continue; // Fenster noch nicht offen
        }
        let still = ms().saturating_sub(puls) / 1000;
        let lang = ZUSTAND.lock().ok().and_then(|g| g.as_ref().map(|z| z.lang)).unwrap_or(false);
        if still >= if lang { 60 } else { HAENGER_S } && !gemeldet {
            zeile(format!("HAENGER: Hauptschleife seit {still} s ohne Bild"));
            bericht("haenger", &format!("Die Hauptschleife lief seit {still} s nicht mehr (Fenster reagiert nicht)."));
            gemeldet = true;
        } else if still < 2 && gemeldet {
            log::warn!("Hauptschleife läuft wieder");
            gemeldet = false;
        }
    }
}

/// Bericht (Absturz/Hänger), der neuer ist als die vorige Logdatei: beim Start in der Statuszeile melden
pub fn neuer_bericht() -> Option<PathBuf> {
    let d = ordner();
    let mut logs: Vec<(SystemTime, PathBuf)> = Vec::new();
    let mut berichte: Vec<(SystemTime, PathBuf)> = Vec::new();
    for e in std::fs::read_dir(&d).ok()?.flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        let t = e.metadata().and_then(|m| m.modified()).unwrap_or(UNIX_EPOCH);
        if n.starts_with("omsi-editor-") {
            // Beginn der Sitzung
            logs.push((e.metadata().and_then(|m| m.created()).unwrap_or(t), e.path()));
        } else if n.starts_with("absturz-") || n.starts_with("haenger-") {
            berichte.push((t, e.path()));
        }
    }
    logs.sort();
    // vorletzte Logdatei = vorige Sitzung (die letzte ist die eben angelegte)
    let vorige = logs.iter().rev().nth(1).map(|x| x.0)?;
    berichte.into_iter().filter(|(t, _)| *t >= vorige).max().map(|x| x.1)
}

/// nur die letzten 10 Protokolle und 20 Berichte behalten
fn aufraeumen(d: &std::path::Path) {
    for (praefix, n) in [("omsi-editor-", 10usize), ("absturz-", 20), ("haenger-", 20)] {
        let mut v: Vec<PathBuf> = std::fs::read_dir(d).map(|r| r.flatten().map(|e| e.path())
            .filter(|p| p.file_name().is_some_and(|f| f.to_string_lossy().starts_with(praefix))).collect()).unwrap_or_default();
        v.sort();
        while v.len() > n {
            let _ = std::fs::remove_file(v.remove(0));
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn zeitstempel_form() {
        let z = super::zeitstempel();
        assert_eq!(z.len(), 15, "{z}");
        assert!(z.starts_with("20") && z.as_bytes()[8] == b'-');
    }
}
