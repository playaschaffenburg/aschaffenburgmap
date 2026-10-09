//! Georeferenz einer Karte und Geodaten aus omsigen (`python -m omsigen.geodaten`): Ort suchen, Gelaende und
//! Luftbild je Kachel.
//!
//! Eine Karte mit Ort hat die Datei `omsi-editor-geo.cfg` in ihrem Ordner (OMSI liest sie nicht): Ort, UTM-Zone, die
//! UTM-Koordinaten des Kartenursprungs (Suedwestecke der Kachel 0 0) und die Hoehe ueber NN, die in OMSI 0 ist. Die
//! Karte liegt damit im UTM-Gitter: Kartenpunkt (x, y) = (Ost - Ost0, Nord - Nord0). So bekommt jede spaeter angefuegte
//! Kachel (World Editor) genau das Gelaende und Luftbild ihrer Stelle, und die Karte laesst sich beliebig erweitern.
//! Die Datei wandert beim Speichern mit (Kopie der Karte: der ganze Ordner).

use anyhow::{bail, Context, Result};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const DATEI: &str = "omsi-editor-geo.cfg";
const KACHEL: f64 = 300.0;

/// gefundener Ort
#[derive(Clone, Debug, PartialEq)]
pub struct Ort {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

/// Lage der Karte in der Welt
#[derive(Clone, Debug, PartialEq)]
pub struct Bezug {
    pub ort: String,
    pub lat: f64,
    pub lon: f64,
    pub zone: u32,
    pub ost0: f64,
    pub nord0: f64,
    /// Hoehe ueber NN, die in der Karte 0 ist
    pub nn0: f64,
}

impl Bezug {
    pub fn lesen(karte: &Path) -> Option<Bezug> {
        let text = std::fs::read_to_string(karte.join(DATEI)).ok()?;
        let wert = |k: &str| text.lines().find_map(|l| l.split_once('=').filter(|(a, _)| a.trim() == k).map(|(_, b)| b.trim().to_string()));
        let zahl = |k: &str| wert(k).and_then(|v| v.parse::<f64>().ok());
        Some(Bezug {
            ort: wert("ort").unwrap_or_default(),
            lat: zahl("lat")?,
            lon: zahl("lon")?,
            zone: zahl("utm_zone")? as u32,
            ost0: zahl("ost0")?,
            nord0: zahl("nord0")?,
            nn0: zahl("nn0").unwrap_or(0.0),
        })
    }

    pub fn schreiben(&self, karte: &Path) -> Result<()> {
        let text = format!(
            "# omsi-editor: Lage der Karte in der Welt (OMSI liest diese Datei nicht)\r\n\
             # Kartenpunkt (x, y) = (UTM-Ost - ost0, UTM-Nord - nord0); Hoehe in der Karte = Hoehe ueber NN - nn0\r\n\
             ort = {}\r\nlat = {:.7}\r\nlon = {:.7}\r\nutm_zone = {}\r\nost0 = {:.3}\r\nnord0 = {:.3}\r\nnn0 = {:.2}\r\n",
            self.ort.replace(['\r', '\n'], " "), self.lat, self.lon, self.zone, self.ost0, self.nord0, self.nn0);
        std::fs::write(karte.join(DATEI), text).with_context(|| format!("{} schreiben", karte.join(DATEI).display()))
    }

    /// Bezug, der den Ort in die Mitte der Kachel 0 0 legt (UTM und Gelaendehoehe dort aus omsigen)
    pub fn fuer(ort: &Ort) -> Result<Bezug> {
        let erg = python(&serde_json::json!({"auftrag": "bezug", "lat": ort.lat, "lon": ort.lon, "cache": cache()}))?;
        let zahl = |k: &str| erg.get(k).and_then(|x| x.as_f64());
        let (ost, nord) = (zahl("ost").context("omsigen: kein UTM")?, zahl("nord").context("omsigen: kein UTM")?);
        Ok(Bezug {
            ort: ort.name.clone(),
            lat: ort.lat,
            lon: ort.lon,
            zone: zahl("zone").unwrap_or(32.0) as u32,
            ost0: (ost - KACHEL / 2.0).round(),
            nord0: (nord - KACHEL / 2.0).round(),
            nn0: zahl("nn").map(|h| h.round()).unwrap_or(0.0),
        })
    }
}

/// Orte zu einem Suchtext (Nominatim/OpenStreetMap) oder Koordinaten "lat, lon"
pub fn suchen(text: &str) -> Result<Vec<Ort>> {
    let erg = python(&serde_json::json!({"auftrag": "suchen", "text": text}))?;
    Ok(erg["orte"].as_array().map(|l| l.iter().filter_map(|o| Some(Ort {
        name: o["name"].as_str()?.to_string(), lat: o["lat"].as_f64()?, lon: o["lon"].as_f64()?,
    })).collect()).unwrap_or_default())
}

/// Geodaten einer Kachel
#[derive(Debug, Default)]
pub struct KachelDaten {
    pub kachel: (i32, i32),
    /// 61 x 61 Hoehen (relativ zu nn0), Zeile von Sueden
    pub gelaende: Option<Vec<f32>>,
    pub gelaende_quelle: Option<String>,
    /// JPEG, erste Zeile Norden
    pub luftbild: Option<PathBuf>,
}

/// Gelaende und/oder Luftbild fuer Kacheln holen (blockiert; Zwischenspeicher unter %LOCALAPPDATA%\omsi-editor\geodaten).
/// Liefert die Daten und die Quellenangaben.
pub fn kacheln(b: &Bezug, kacheln: &[(i32, i32)], gelaende: bool, luftbild: bool) -> Result<(Vec<KachelDaten>, Vec<String>)> {
    let ordner = PathBuf::from(cache()).join("auftrag").join(format!("{}-{}", std::process::id(), zaehler()));
    let erg = python(&serde_json::json!({
        "auftrag": "kacheln", "zone": b.zone, "ost0": b.ost0, "nord0": b.nord0, "nn0": b.nn0,
        "kacheln": kacheln.iter().map(|k| [k.0, k.1]).collect::<Vec<_>>(), "gelaende": gelaende, "luftbild": luftbild,
        "pixel": 1024, "ordner": ordner.to_string_lossy(), "cache": cache(),
    }))?;
    let mut aus = Vec::new();
    for e in erg["kacheln"].as_array().map(|v| v.as_slice()).unwrap_or(&[]) {
        let k = (e["kachel"][0].as_i64().unwrap_or(0) as i32, e["kachel"][1].as_i64().unwrap_or(0) as i32);
        let mut d = KachelDaten { kachel: k, ..Default::default() };
        if let Some(p) = e["gelaende"].as_str() {
            let t = std::fs::read(p).ok().and_then(|b| omsi_map::Terrain::parse(&b).ok()).filter(|t| t.samples() == 61);
            d.gelaende = t.map(|t| t.heights);
            d.gelaende_quelle = e["gelaende_quelle"].as_str().map(|s| s.to_string());
            let _ = std::fs::remove_file(p);
        }
        if let Some(p) = e["luftbild"].as_str() {
            d.luftbild = Some(PathBuf::from(p));
        }
        aus.push(d);
    }
    let quellen = erg["quellen"].as_array().map(|l| l.iter().filter_map(|q| q.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
    Ok((aus, quellen))
}

fn zaehler() -> usize {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Zwischenspeicher der Geodaten (DGM-Kacheln, Luftbilder)
pub fn cache() -> String {
    let basis = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    basis.join("omsi-editor").join("geodaten").to_string_lossy().replace('\\', "/")
}

fn python(auftrag: &serde_json::Value) -> Result<serde_json::Value> {
    let python = std::env::var("OMSIGEN_PYTHON").unwrap_or_else(|_| "python".into());
    let mut cmd = std::process::Command::new(&python);
    cmd.args(["-m", "omsigen.geodaten"])
        .current_dir(crate::kreuzung::omsigen_ordner())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // kein Konsolenfenster
    }
    let mut kind = cmd.spawn().with_context(|| format!("{python} starten (omsigen holt die Geodaten)"))?;
    kind.stdin.take().unwrap().write_all(auftrag.to_string().as_bytes())?;
    let aus = kind.wait_with_output()?;
    if !aus.stderr.is_empty() {
        log::info!("omsigen.geodaten: {}", String::from_utf8_lossy(&aus.stderr).trim());
    }
    let erg: serde_json::Value = serde_json::from_slice(&aus.stdout)
        .with_context(|| format!("omsigen: keine Antwort ({})", String::from_utf8_lossy(&aus.stderr).lines().last().unwrap_or("")))?;
    if let Some(f) = erg.get("fehler").and_then(|f| f.as_str()) {
        bail!("omsigen: {f}");
    }
    Ok(erg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bezug_datei() {
        let d = std::env::temp_dir().join(format!("omsi-editor-geo-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let b = Bezug { ort: "Aschaffenburg".into(), lat: 49.97, lon: 9.14, zone: 32, ost0: 510146.0, nord0: 5536334.0, nn0: 130.0 };
        b.schreiben(&d).unwrap();
        assert_eq!(Bezug::lesen(&d), Some(b));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// (Netz) Ort suchen, Bezug, Gelaende und Luftbild einer Kachel in Aschaffenburg
    #[test]
    #[ignore]
    fn aschaffenburg_holen() {
        let orte = suchen("49.9806, 9.1436").unwrap();
        assert_eq!(orte.len(), 1);
        let b = Bezug::fuer(&orte[0]).unwrap();
        println!("{b:?}");
        assert_eq!(b.zone, 32);
        assert!((b.nn0 - 130.0).abs() < 15.0, "Hoehe {}", b.nn0);
        let (d, q) = kacheln(&b, &[(0, 0)], true, true).unwrap();
        let h = d[0].gelaende.as_ref().expect("Gelaende");
        // Mitte der Kachel = der Ort, dort ungefaehr 0 m
        assert!(h[30 * 61 + 30].abs() < 6.0, "Mitte {}", h[30 * 61 + 30]);
        assert!(d[0].luftbild.as_ref().is_some_and(|p| p.is_file()), "Luftbild");
        assert!(q.iter().any(|q| q.contains("DGM1")));
    }
}
