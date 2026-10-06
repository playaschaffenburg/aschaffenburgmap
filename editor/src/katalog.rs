//! Objektkatalog fuer das Platzieren (wie das Asset-Menue in Transport Fever 2): alle .sco unter Sceneryobjects,
//! mit Anzeigename ([friendlyname]), Ordner und Gruppen ([groups]) zum Filtern. Wird im Hintergrund eingelesen.

use std::path::Path;

#[derive(Clone, Debug)]
pub struct Eintrag {
    /// relativ zum OMSI-Ordner, mit \ (wie in Kacheldateien)
    pub rel: String,
    pub name: String,
    /// erster Ordner unter Sceneryobjects
    pub ordner: String,
    pub gruppen: Vec<String>,
    /// woher das Objekt stammt: "OMSI (Standard)", eine Stadt/Karte, ...
    pub herkunft: String,
    /// alles klein, fuer die Suche
    such: String,
}

impl Eintrag {
    pub fn passt(&self, suche: &str, ordner: Option<&str>, gruppe: Option<&str>, herkunft: Option<&str>) -> bool {
        herkunft.map(|h| self.herkunft == h).unwrap_or(true)
            && ordner.map(|o| self.ordner == o).unwrap_or(true)
            && gruppe.map(|g| self.gruppen.iter().any(|x| x == g)).unwrap_or(true)
            && (suche.is_empty() || suche.split_whitespace().all(|w| self.such.contains(w)))
    }
}

pub struct Katalog {
    pub eintraege: Vec<Eintrag>,
    pub ordner: Vec<String>,
    pub gruppen: Vec<String>,
    /// Herkuenfte mit Anzahl Objekte, "OMSI (Standard)" zuerst
    pub herkuenfte: Vec<(String, usize)>,
}

pub const STANDARD: &str = "OMSI (Standard)";
/// Karten, die mit OMSI 2 kommen
const STANDARD_KARTEN: [&str; 2] = ["grundorf", "berlin-spandau"];
/// Stadt/Paket aus Karten- oder Ordnernamen (klein geschrieben gesucht)
const STAEDTE: [(&str, &str); 16] = [
    ("aschaffenburg", "Aschaffenburg (omsigen)"), ("hamburg", "Hamburg"), ("aachen", "Aachen"), ("bremen", "Bremen"),
    ("hb_", "Bremen"), ("gladbeck", "Gladbeck"), ("ruhr", "Ruhrgebiet"), ("koeln", "Koeln"), ("köln", "Koeln"),
    ("rheinhausen", "Rheinhausen"), ("neuendorf", "Neuendorf"), ("x10", "Berlin X10"), ("express", "Express 91.06"),
    ("szczecin", "Express 91.06"), ("spandau", STANDARD), ("grundorf", STANDARD),
];

pub type Nutzung = std::collections::HashMap<String, std::collections::HashMap<String, usize>>;

fn stadt(name: &str) -> Option<&'static str> {
    let n = name.to_lowercase();
    STAEDTE.iter().find(|(k, _)| n.contains(k)).map(|(_, s)| *s)
}

/// Welche Karte nutzt welche Objekte: erster Ordner unter Sceneryobjects (klein) -> Karte -> Anzahl
pub fn nutzung(root: &Path) -> Nutzung {
    nutzung_von(root, "sceneryobjects", &["[object]", "[attachobj]", "[splineattachement]"])
}

/// dasselbe fuer Splines: erster Ordner unter Splines -> Karte -> Anzahl
pub fn nutzung_splines(root: &Path) -> Nutzung {
    nutzung_von(root, "splines", &["[spline]", "[spline_h]"])
}

/// Eintraege mit einem der Schluessel, Datei zwei Zeilen darunter unter `basis\<Ordner>\...`
fn nutzung_von(root: &Path, basis: &str, schluessel: &[&str]) -> Nutzung {
    let mut out: Nutzung = Default::default();
    let Ok(karten) = std::fs::read_dir(root.join("maps")) else { return out };
    let trenner = |c: char| c == '\\' || c == '/';
    for k in karten.flatten() {
        let karte = k.file_name().to_string_lossy().to_string();
        let Ok(rd) = std::fs::read_dir(k.path()) else { continue };
        for f in rd.flatten() {
            let n = f.file_name().to_string_lossy().to_lowercase();
            if !(n.starts_with("tile_") && n.ends_with(".map")) {
                continue;
            }
            let Ok(b) = std::fs::read(f.path()) else { continue };
            let text = if b.starts_with(&[0xFF, 0xFE]) {
                String::from_utf16_lossy(&b[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>())
            } else {
                b.iter().map(|&c| c as char).collect()
            };
            let zeilen: Vec<&str> = text.lines().collect();
            for (i, z) in zeilen.iter().enumerate() {
                let z = z.trim();
                if !schluessel.iter().any(|k| z.eq_ignore_ascii_case(k)) {
                    continue;
                }
                let Some(pfad) = zeilen.get(i + 2) else { continue };
                let teile: Vec<String> = pfad.trim().split(trenner).map(|t| t.to_lowercase()).collect();
                if teile.len() >= 3 && teile[0] == basis {
                    *out.entry(teile[1].clone()).or_default().entry(karte.clone()).or_default() += 1;
                }
            }
        }
    }
    out
}

/// Herkunft eines Objektordners: Standard, wenn eine Standardkarte ihn nutzt; sonst die Stadt der Karte, die ihn am
/// meisten nutzt; sonst nach Namen; sonst "ohne Karte"
pub fn herkunft(ordner: &str, nutzung: &Nutzung) -> String {
    if let Some(k) = nutzung.get(&ordner.to_lowercase()) {
        if k.keys().any(|karte| STANDARD_KARTEN.contains(&karte.to_lowercase().as_str())) {
            return STANDARD.into();
        }
        if let Some((karte, _)) = k.iter().max_by_key(|(_, n)| **n) {
            return stadt(karte).or_else(|| stadt(ordner)).map(|s| s.to_string()).unwrap_or_else(|| karte.clone());
        }
    }
    stadt(ordner).map(|s| s.to_string()).unwrap_or_else(|| "ohne Karte".into())
}


/// [friendlyname] und [groups] aus einer .sco (8-Bit-Text)
fn kopf(text: &str) -> (Option<String>, Vec<String>) {
    let zeilen: Vec<&str> = text.lines().map(|l| l.trim()).collect();
    let mut name = None;
    let mut gruppen = Vec::new();
    let mut i = 0;
    while i < zeilen.len() {
        let z = zeilen[i].to_ascii_lowercase();
        if z == "[friendlyname]" && i + 1 < zeilen.len() && name.is_none() {
            name = Some(zeilen[i + 1].to_string()).filter(|n| !n.is_empty());
        } else if z == "[groups]" && i + 1 < zeilen.len() {
            let n: usize = zeilen[i + 1].parse().unwrap_or(0);
            for k in 0..n.min(20) {
                if let Some(g) = zeilen.get(i + 2 + k) {
                    if !g.is_empty() && !g.starts_with('[') {
                        gruppen.push(g.to_string());
                    }
                }
            }
        } else if z == "[mesh]" {
            break; // der Kopf steht vor den Modellen
        }
        i += 1;
    }
    (name, gruppen)
}

pub fn einlesen(root: &Path) -> Katalog {
    let genutzt = nutzung(root);
    let basis = root.join("Sceneryobjects");
    let mut eintraege = Vec::new();
    let mut stapel = vec![basis.clone()];
    while let Some(d) = stapel.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                stapel.push(p);
                continue;
            }
            if !p.extension().is_some_and(|x| x.eq_ignore_ascii_case("sco")) {
                continue;
            }
            let text: String = std::fs::read(&p).map(|b| b.iter().map(|&c| c as char).collect()).unwrap_or_default();
            let (fname, gruppen) = kopf(&text);
            let datei = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().replace('/', "\\");
            let ordner = p.strip_prefix(&basis).ok().and_then(|r| r.components().next()).map(|c| c.as_os_str().to_string_lossy().to_string()).unwrap_or_default();
            let name = fname.unwrap_or(datei.clone());
            let herkunft = herkunft(&ordner, &genutzt);
            let such = format!("{} {} {} {} {}", name, datei, rel, gruppen.join(" "), herkunft).to_lowercase();
            eintraege.push(Eintrag { rel, name, ordner, gruppen, herkunft, such });
        }
    }
    eintraege.sort_by(|a, b| (a.ordner.to_lowercase(), a.name.to_lowercase()).cmp(&(b.ordner.to_lowercase(), b.name.to_lowercase())));
    let mut ordner: Vec<String> = eintraege.iter().map(|e| e.ordner.clone()).collect();
    ordner.sort_by_key(|o| o.to_lowercase());
    ordner.dedup();
    let mut gruppen: Vec<String> = eintraege.iter().flat_map(|e| e.gruppen.clone()).collect();
    gruppen.sort_by_key(|g| g.to_lowercase());
    gruppen.dedup();
    let mut zahl: std::collections::HashMap<String, usize> = Default::default();
    for e in &eintraege {
        *zahl.entry(e.herkunft.clone()).or_default() += 1;
    }
    let mut herkuenfte: Vec<(String, usize)> = zahl.into_iter().collect();
    herkuenfte.sort_by_key(|(h, _)| (h != STANDARD, h == "ohne Karte", h.to_lowercase()));
    Katalog { eintraege, ordner, gruppen, herkuenfte }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kopf_lesen() {
        let t = "[friendlyname]\r\nBushaltestelle\r\n\r\n[groups]\r\n2\r\nHaltestellen\r\nStrassenmoebel\r\n\r\n[mesh]\r\nx.o3d\r\n[groups]\r\n1\r\nnicht\r\n";
        let (n, g) = kopf(t);
        assert_eq!(n.as_deref(), Some("Bushaltestelle"));
        assert_eq!(g, vec!["Haltestellen", "Strassenmoebel"]);
    }

    #[test]
    fn filtern() {
        let e = Eintrag { rel: "Sceneryobjects\\A\\bank.sco".into(), name: "Parkbank".into(), ordner: "A".into(),
                          gruppen: vec!["Moebel".into()], herkunft: "Hamburg".into(),
                          such: "parkbank bank sceneryobjects\\a\\bank.sco moebel".into() };
        assert!(e.passt("park", None, None, None) && e.passt("bank moeb", Some("A"), Some("Moebel"), Some("Hamburg")));
        assert!(!e.passt("haus", None, None, None) && !e.passt("", Some("B"), None, None) && !e.passt("", None, None, Some(STANDARD)));
    }

    #[test]
    fn herkunft_bestimmen() {
        let mut n: Nutzung = Default::default();
        n.entry("streetobjects_mc".into()).or_default().insert("Grundorf".into(), 5);
        n.entry("streetobjects_mc".into()).or_default().insert("HamburgLi20".into(), 50);
        n.entry("hafencityhamburgobjects".into()).or_default().insert("HafenCityHamburg".into(), 9);
        n.entry("adda".into()).or_default().insert("Koeln".into(), 3);
        assert_eq!(herkunft("Streetobjects_MC", &n), STANDARD);
        assert_eq!(herkunft("HafenCityHamburgObjects", &n), "Hamburg");
        assert_eq!(herkunft("ADDA", &n), "Koeln");
        assert_eq!(herkunft("Aachen_Gruenzeug", &n), "Aachen");
        assert_eq!(herkunft("Irgendwas", &n), "ohne Karte");
    }
}
