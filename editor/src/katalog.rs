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
    /// alles klein, fuer die Suche
    such: String,
}

impl Eintrag {
    pub fn passt(&self, suche: &str, ordner: Option<&str>, gruppe: Option<&str>) -> bool {
        ordner.map(|o| self.ordner == o).unwrap_or(true)
            && gruppe.map(|g| self.gruppen.iter().any(|x| x == g)).unwrap_or(true)
            && (suche.is_empty() || suche.split_whitespace().all(|w| self.such.contains(w)))
    }
}

pub struct Katalog {
    pub eintraege: Vec<Eintrag>,
    pub ordner: Vec<String>,
    pub gruppen: Vec<String>,
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
            let such = format!("{} {} {} {}", name, datei, rel, gruppen.join(" ")).to_lowercase();
            eintraege.push(Eintrag { rel, name, ordner, gruppen, such });
        }
    }
    eintraege.sort_by(|a, b| (a.ordner.to_lowercase(), a.name.to_lowercase()).cmp(&(b.ordner.to_lowercase(), b.name.to_lowercase())));
    let mut ordner: Vec<String> = eintraege.iter().map(|e| e.ordner.clone()).collect();
    ordner.sort_by_key(|o| o.to_lowercase());
    ordner.dedup();
    let mut gruppen: Vec<String> = eintraege.iter().flat_map(|e| e.gruppen.clone()).collect();
    gruppen.sort_by_key(|g| g.to_lowercase());
    gruppen.dedup();
    Katalog { eintraege, ordner, gruppen }
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
                          gruppen: vec!["Moebel".into()], such: "parkbank bank sceneryobjects\\a\\bank.sco moebel".into() };
        assert!(e.passt("park", None, None) && e.passt("bank moeb", Some("A"), Some("Moebel")));
        assert!(!e.passt("haus", None, None) && !e.passt("", Some("B"), None));
    }
}
