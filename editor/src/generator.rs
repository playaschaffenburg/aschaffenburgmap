//! Arbeiten fuer den Generator (omsigen, Python) ohne Fenster: `omsi-editor --querschnitte <auftrag.json>`.
//!
//! Der Generator leitet je Strasse einen Querschnitt aus OSM, Messung und Regeln ab (omsigen/querschnitte.py) und laesst
//! die .sli-Dateien hier mit dem Querschnitt-Baukasten (querschnitt.rs) bauen - so gibt es nur einen Spline-Bauer.
//! Ebenso die Uebergaenge zwischen zwei Querschnitten (uebergang.rs).
//!
//! Auftrag (JSON):
//!   {"root": OMSI-Ordner, "ordner": Unterordner von Splines\Aschaffenburg (z. B. "Q"),
//!    "querschnitte": [{"datei": Dateiname ohne Endung, "querschnitt": Bauanleitung wie .qs.json}],
//!    "uebergaenge": [{"ordner": Objektordner (absolut), "name": "UE_001", "titel": ...,
//!                     "a": {"querschnitt": {...}} oder {"sli": "Splines\\..."}, "a_gedreht": bool,
//!                     "b": ..., "b_gedreht": bool, "laenge": m, "steigung": Verhaeltnis}],
//!    "antwort": Datei fuer die Antwort (optional, sonst nur stdout)}
//! Antwort (eine JSON-Zeile): {"splines": [Pfad relativ zum OMSI-Ordner je Querschnitt oder null], "uebergaenge":
//!   [{"sco": Pfad, "laenge": m, "pfade": n} oder null], "fehler": [Text]}

use crate::querschnitt::{self, Querschnitt};
use crate::uebergang::{self, Seite};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

fn seite(root: &Path, v: &serde_json::Value, gedreht: bool) -> Result<Seite> {
    let s = if let Some(q) = v.get("querschnitt").and_then(Querschnitt::aus_json) {
        Seite::aus_querschnitt(&q)
    } else if let Some(sli) = v["sli"].as_str() {
        uebergang::seite(root, sli)?
    } else {
        anyhow::bail!("Seite ohne querschnitt/sli");
    };
    Ok(if gedreht { s.gedreht() } else { s })
}

/// den Auftrag abarbeiten -> Antwort
pub fn ausfuehren(auftrag: &serde_json::Value, root_vorgabe: &Path) -> serde_json::Value {
    let root = auftrag["root"].as_str().map(PathBuf::from).unwrap_or_else(|| root_vorgabe.to_path_buf());
    let unter = auftrag["ordner"].as_str().unwrap_or("Q");
    // Belagstexturen (Splines\Marcel\texture) aus dieser Installation, wenn `root` nur ein Ausgabeordner ist
    let quelle = auftrag["texturen_aus"].as_str().map(PathBuf::from).filter(|p| p.join("Splines").join("Marcel").is_dir()).unwrap_or_else(|| root.clone());
    let mut fehler = Vec::new();
    let mut warnungen = Vec::new();
    let mut splines = Vec::new();
    for (i, e) in auftrag["querschnitte"].as_array().map(|a| a.as_slice()).unwrap_or(&[]).iter().enumerate() {
        let r = (|| -> Result<String> {
            let q = Querschnitt::aus_json(&e["querschnitt"]).context("Bauanleitung unlesbar")?;
            q.pruefen()?;
            let datei = e["datei"].as_str().context("datei fehlt")?;
            anyhow::ensure!(!datei.is_empty() && !datei.contains(['\\', '/', '.']), "Dateiname {datei:?} ungueltig");
            let (rel, texturordner, texturen) = querschnitt::sli_schreiben_unter(&root, &q, unter, datei)?;
            // fehlende Texturen machen die Spline nicht unbrauchbar (Spuren und Breiten stimmen): nur melden
            if let Err(err) = querschnitt::texturen_bereitstellen_nach(&quelle, &texturordner, &texturen) {
                warnungen.push(format!("{datei}: {err:#}"));
            }
            Ok(rel)
        })();
        match r {
            Ok(rel) => splines.push(serde_json::Value::String(rel)),
            Err(err) => {
                fehler.push(format!("Querschnitt {i} ({}): {err:#}", e["datei"].as_str().unwrap_or("?")));
                splines.push(serde_json::Value::Null);
            }
        }
    }
    let mut uebergaenge = Vec::new();
    for (i, e) in auftrag["uebergaenge"].as_array().map(|a| a.as_slice()).unwrap_or(&[]).iter().enumerate() {
        let r = (|| -> Result<serde_json::Value> {
            let a = seite(&root, &e["a"], e["a_gedreht"].as_bool().unwrap_or(false))?;
            let b = seite(&root, &e["b"], e["b_gedreht"].as_bool().unwrap_or(false))?;
            let laenge = e["laenge"].as_f64().unwrap_or_else(|| uebergang::laenge_vorschlag(&a, &b));
            let u = uebergang::bauen(&a, &b, laenge, e["steigung"].as_f64().unwrap_or(0.0))?;
            let ordner = PathBuf::from(e["ordner"].as_str().context("ordner fehlt")?);
            let name = e["name"].as_str().context("name fehlt")?;
            if let Err(err) = u.texturen_kopieren(&quelle, &ordner) {
                warnungen.push(format!("{name}: {err:#}"));
            }
            let sco = u.objekt_schreiben(&ordner, name, e["titel"].as_str().unwrap_or(name))?;
            Ok(serde_json::json!({"sco": sco.to_string_lossy(), "laenge": laenge, "pfade": u.pfade.len(),
                                  "dreiecke": u.modell.dreiecke.len()}))
        })();
        match r {
            Ok(v) => uebergaenge.push(v),
            Err(err) => {
                fehler.push(format!("Uebergang {i} ({}): {err:#}", e["name"].as_str().unwrap_or("?")));
                uebergaenge.push(serde_json::Value::Null);
            }
        }
    }
    warnungen.sort();
    warnungen.dedup();
    serde_json::json!({"splines": splines, "uebergaenge": uebergaenge, "fehler": fehler, "warnungen": warnungen})
}

/// Kommandozeile: Auftrag lesen, ausfuehren, Antwort auf stdout (und in `antwort`)
pub fn kommandozeile(pfad: &Path, root: &Path) -> Result<()> {
    let auftrag: serde_json::Value = serde_json::from_slice(&std::fs::read(pfad).with_context(|| format!("{} lesen", pfad.display()))?)
        .with_context(|| format!("{} ist kein JSON", pfad.display()))?;
    let antwort = ausfuehren(&auftrag, root);
    let text = serde_json::to_string(&antwort)?;
    if let Some(a) = auftrag["antwort"].as_str() {
        std::fs::write(a, &text).with_context(|| format!("Antwort nach {a}"))?;
    }
    println!("{text}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Auftrag mit zwei Querschnitten und einem Uebergang in einen Test-Ordner (Texturen nur, soweit ohne Installation
    /// moeglich: die Markierungen werden erzeugt, Belaege fehlen -> Fehler wird gemeldet, nicht abgebrochen)
    #[test]
    fn auftrag_ohne_installation() {
        let root = std::env::temp_dir().join(format!("omsi-editor-gen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let q = Querschnitt::default();
        let auftrag = serde_json::json!({"root": root.to_string_lossy(), "ordner": "Q",
            "querschnitte": [{"datei": "Q_test", "querschnitt": q.json()}, {"datei": "../boese", "querschnitt": q.json()}]});
        let a = ausfuehren(&auftrag, &root);
        // ohne Splines\Marcel\texture fehlen die Belaege: die Spline entsteht trotzdem (Warnung), der boese Name nicht
        assert_eq!(a["splines"].as_array().unwrap().len(), 2);
        assert_eq!(a["splines"][0], "Splines\\Aschaffenburg\\Q\\Q_test.sli", "{a}");
        assert!(a["splines"][1].is_null());
        assert_eq!(a["fehler"].as_array().unwrap().len(), 1, "{a}");
        assert!(!a["warnungen"].as_array().unwrap().is_empty(), "{a}");
        // mit (leeren) Belagstexturen ohne Warnung
        let marcel = root.join("Splines/Marcel/texture");
        std::fs::create_dir_all(&marcel).unwrap();
        for t in ["str_side1.bmp", "str_asphdrk.bmp", "str_gehweg02.bmp"] {
            std::fs::write(marcel.join(t), b"BM").unwrap();
        }
        let a = ausfuehren(&auftrag, &root);
        assert!(a["warnungen"].as_array().unwrap().is_empty(), "{a}");
        // Uebergang 1+1 -> 2+1 in einen Objektordner
        let mut b = Querschnitt::default();
        b.teile.insert(2, crate::querschnitt::Teil::neu(crate::querschnitt::Art::Fahrspur));
        b.marken_anpassen();
        let ue = serde_json::json!({"root": root.to_string_lossy(), "uebergaenge": [{"ordner": root.join("obj").to_string_lossy(),
            "name": "UE_001", "a": {"querschnitt": q.json()}, "b": {"querschnitt": b.json()}, "laenge": 20.0}]});
        let a = ausfuehren(&ue, &root);
        assert!(a["fehler"].as_array().unwrap().is_empty(), "{a}");
        assert!(root.join("obj/UE_001.sco").is_file() && root.join("obj/model/UE_001.x").is_file());
        assert_eq!(a["uebergaenge"][0]["laenge"], 20.0);
        let d = root.join("Splines/Aschaffenburg/Q");
        assert!(d.join("Q_test.sli").is_file() && d.join("Q_test.qs.json").is_file());
        assert!(d.join("texture/AB_marke_strich.tga").is_file() && d.join("texture/str_asphdrk.bmp").is_file());
        let (text, _) = q.sli();
        let geschrieben: String = std::fs::read(d.join("Q_test.sli")).unwrap().iter().map(|&c| c as char).collect();
        assert_eq!(geschrieben, text);
        std::fs::remove_dir_all(&root).ok();
    }
}
