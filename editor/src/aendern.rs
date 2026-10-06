//! Werkzeug "Aendern" (wie das Upgrade-Werkzeug in Transport Fever 2) fuer die vorhandenen Strassen einer Karte:
//! Spline unter der Maus waehlen (Umschalt: ganze Kette), Querschnitt aendern, Richtung umkehren (mirror), loeschen.
//!
//! Die geaenderten Kacheln schreibt der Editor in einen Sitzungsordner, den openOMSI vor der Installation liest
//! (Viewer::session_overlay); die Kachel wird neu geladen und zeigt die Aenderung sofort. Beim Speichern als neue
//! Karte kommen diese Kacheln mit - die Originalkarte bleibt unveraendert.

use crate::anschluss::spuren_passen;
use anyhow::{Context, Result};
use glam::{DVec2, DVec3};
use openomsi_game::viewer::Viewer;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// ein vorhandener Spline (aus der Kacheldatei)
#[derive(Clone, Debug)]
pub struct KartenSpline {
    pub id: i64,
    pub kachel: (i32, i32),
    pub sli: String,
    pub kurve: omsi_geometry::SplineCurve,
    pub gespiegelt: bool,
    pub prev: i64,
    pub next: i64,
}

impl KartenSpline {
    /// Punkte der Mittellinie (alle ~2 m) mit Richtung
    pub fn punkte(&self) -> Vec<(DVec3, f64)> {
        let n = (self.kurve.length / 2.0).ceil().max(1.0) as usize;
        (0..=n).map(|i| {
            let s = self.kurve.length * i as f64 / n as f64;
            (self.kurve.point_at(s), self.kurve.heading_at(s))
        }).collect()
    }
}

/// Aenderung an einer Kachel fuer Rueckgaengig: Inhalt der Sitzungskopie vorher (None: es gab keine)
#[derive(Clone)]
struct Schritt {
    kachel: (i32, i32),
    datei: PathBuf,
    vorher: Option<Vec<u8>>,
    nachher: Option<Vec<u8>>,
}

pub struct Aendern {
    /// Sitzungsordner (vor der Installation gelesen)
    pub sitzung: PathBuf,
    kacheln: HashMap<(i32, i32), Vec<KartenSpline>>,
    stand: Vec<(i32, i32)>,
    pub unter_maus: Option<i64>,
    pub auswahl: Vec<i64>,
    undo: Vec<Vec<Schritt>>,
    redo: Vec<Vec<Schritt>>,
    pub aenderungen: usize,
    breiten: HashMap<String, (f32, f32)>,
}

impl Aendern {
    pub fn neu(v: &Viewer) -> Self {
        static NR: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let nr = NR.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let sitzung = std::env::temp_dir().join("omsi-editor").join(format!("sitzung-{}-{nr}", std::process::id()));
        let _ = std::fs::remove_dir_all(&sitzung);
        let _ = std::fs::create_dir_all(&sitzung);
        v.session_overlay(&sitzung);
        Aendern { sitzung, kacheln: HashMap::new(), stand: vec![], unter_maus: None, auswahl: vec![], undo: vec![], redo: vec![],
                  aenderungen: 0, breiten: HashMap::new() }
    }

    /// Splines der geladenen Kacheln (neu lesen, wenn sich die geladenen Kacheln geaendert haben)
    pub fn aktualisieren(&mut self, v: &Viewer) {
        let mut k = v.loaded_tile_keys();
        k.sort();
        if k == self.stand {
            return;
        }
        self.kacheln.retain(|x, _| k.contains(x));
        for t in &k {
            if !self.kacheln.contains_key(t) {
                self.kacheln.insert(*t, lesen(v, *t));
            }
        }
        self.stand = k;
    }

    pub fn spline(&self, id: i64) -> Option<&KartenSpline> {
        self.kacheln.values().flatten().find(|s| s.id == id)
    }

    /// halbe Breiten (links, rechts) einer .sli
    pub fn breite(&mut self, v: &Viewer, sli: &str) -> (f32, f32) {
        *self.breiten.entry(sli.to_string()).or_insert_with(|| v.spline_lanes(sli).map(|x| x.1).unwrap_or((3.0, 3.0)))
    }

    /// Strasse (Spline mit Fahrspuren) unter dem Bodenpunkt
    pub fn suchen(&mut self, v: &Viewer, p: DVec2) -> Option<i64> {
        let mut best: Option<(f64, i64)> = None;
        let kandidaten: Vec<KartenSpline> = self.kacheln.values().flatten()
            .filter(|s| (s.kurve.start.truncate() - p).length() < s.kurve.length + 30.0).cloned().collect();
        for s in kandidaten {
            let Some((spuren, _)) = v.spline_lanes(&s.sli) else { continue };
            if !spuren.iter().any(|x| x.0 == 0) {
                continue;
            }
            let (l, r) = self.breite(v, &s.sli);
            for (q, h) in s.punkte() {
                let d = p - q.truncate();
                let quer = d.dot(crate::netz::rechts(h)) as f32;
                let laengs = d.dot(crate::netz::dir(h)).abs();
                let quer = if s.gespiegelt { -quer } else { quer };
                if quer >= -l && quer <= r && laengs < 1.5 {
                    let wert = quer.abs() as f64;
                    if best.map(|b| wert < b.0).unwrap_or(true) {
                        best = Some((wert, s.id));
                    }
                }
            }
        }
        best.map(|b| b.1)
    }

    /// ganze Kette ueber prev/next (soweit geladen)
    pub fn kette(&self, id: i64) -> Vec<i64> {
        let mut out = vec![id];
        let mut seen: HashSet<i64> = [id].into();
        for vorwaerts in [true, false] {
            let mut cur = id;
            loop {
                let Some(s) = self.spline(cur) else { break };
                let n = if vorwaerts { s.next } else { s.prev };
                if n == 0 || !seen.insert(n) || self.spline(n).is_none() {
                    break;
                }
                out.push(n);
                cur = n;
            }
        }
        out
    }

    /// Wohin die Sitzungskopie einer Kacheldatei kommt (gleicher Pfad unter dem Sitzungsordner)
    fn kopie(&self, v: &Viewer, datei: &Path) -> PathBuf {
        if datei.starts_with(&self.sitzung) {
            return datei.to_path_buf();
        }
        let rel = datei.strip_prefix(&v.root).unwrap_or(datei);
        self.sitzung.join(rel)
    }

    /// Kachel(n) mit einer Aenderung je Spline-Eintrag umschreiben, als Sitzungskopie speichern, neu laden
    fn umschreiben(&mut self, v: &mut Viewer, ids: &[i64], f: impl Fn(&mut Vec<String>, usize) -> bool) -> Result<usize> {
        let mut je_kachel: HashMap<(i32, i32), Vec<i64>> = HashMap::new();
        for id in ids {
            if let Some(s) = self.spline(*id) {
                je_kachel.entry(s.kachel).or_default().push(*id);
            }
        }
        let mut schritte = Vec::new();
        let mut n = 0;
        for (k, ids) in je_kachel {
            let quelle = v.tile_file(k.0, k.1).context("Kachel nicht in der Karte")?;
            let ziel = self.kopie(v, &quelle);
            let bytes = std::fs::read(&quelle)?;
            let vorher = ziel.exists().then(|| bytes.clone());
            let (text, utf16) = dekodieren(&bytes);
            let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
            let mut zeilen: Vec<String> = text.split(eol).map(|s| s.to_string()).collect();
            // von hinten, damit Indizes gueltig bleiben
            let mut stellen: Vec<usize> = (0..zeilen.len())
                .filter(|&i| {
                    let w = zeilen[i].trim().to_ascii_lowercase();
                    (w == "[spline]" || w == "[spline_h]") && zeilen.get(i + 3).and_then(|z| z.trim().parse::<i64>().ok()).is_some_and(|x| ids.contains(&x))
                })
                .collect();
            stellen.reverse();
            for i in stellen {
                if f(&mut zeilen, i) {
                    n += 1;
                }
            }
            let neu = kodieren(&zeilen.join(eol), utf16);
            std::fs::create_dir_all(ziel.parent().unwrap())?;
            std::fs::write(&ziel, &neu)?;
            schritte.push(Schritt { kachel: k, datei: ziel, vorher, nachher: Some(neu) });
            self.kacheln.remove(&k);
            self.stand.clear();
        }
        let keys: Vec<(i32, i32)> = schritte.iter().map(|s| s.kachel).collect();
        v.reload_tiles(&keys)?;
        if !schritte.is_empty() {
            self.undo.push(schritte);
            self.redo.clear();
            self.aenderungen += 1;
        }
        Ok(n)
    }

    /// Querschnitt der gewaehlten Splines aendern -> (Anzahl, Warnung wenn Spuren zu Nachbarn nicht passen)
    pub fn querschnitt(&mut self, v: &mut Viewer, sli: &str) -> Result<(usize, Option<String>)> {
        let ids = self.auswahl.clone();
        let warnung = self.spuren_warnung(v, &ids, sli);
        let neu = sli.to_string();
        let n = self.umschreiben(v, &ids, move |z, i| {
            z[i + 2] = neu.clone();
            true
        })?;
        Ok((n, warnung))
    }

    /// Richtung umkehren: `mirror` am Ende des Eintrags setzen bzw. entfernen
    pub fn spiegeln(&mut self, v: &mut Viewer) -> Result<usize> {
        let ids = self.auswahl.clone();
        self.umschreiben(v, &ids, |z, i| {
            let mut j = i + 1;
            while j < z.len() {
                let w = z[j].trim();
                if w.is_empty() || w.starts_with('[') || w.starts_with("Object Nr.") || w.eq_ignore_ascii_case("mirror") {
                    break;
                }
                j += 1;
            }
            if j < z.len() && z[j].trim().eq_ignore_ascii_case("mirror") {
                z.remove(j);
            } else {
                z.insert(j, "mirror".into());
            }
            true
        })
    }

    /// gewaehlte Splines loeschen
    pub fn loeschen(&mut self, v: &mut Viewer) -> Result<usize> {
        let ids = self.auswahl.clone();
        let n = self.umschreiben(v, &ids, |z, i| {
            let mut j = i + 1;
            while j < z.len() && !z[j].trim_start().starts_with('[') && !z[j].trim_start().starts_with("Object Nr.") {
                j += 1;
            }
            z.drain(i..j);
            true
        })?;
        self.auswahl.clear();
        Ok(n)
    }

    fn spuren_warnung(&mut self, v: &Viewer, ids: &[i64], sli: &str) -> Option<String> {
        let neu: Vec<(f32, u8)> = v.spline_lanes(sli)?.0.into_iter().filter(|x| x.0 == 0).map(|x| (x.1, x.4)).collect();
        for id in ids {
            let s = self.spline(*id)?.clone();
            for nb in [s.prev, s.next] {
                if nb == 0 || ids.contains(&nb) {
                    continue;
                }
                let Some(n) = self.spline(nb) else { continue };
                let alt: Vec<(f32, u8)> = v.spline_lanes(&n.sli)?.0.into_iter().filter(|x| x.0 == 0).map(|x| (x.1, x.4)).collect();
                if !spuren_passen(&neu, &alt, s.gespiegelt == n.gespiegelt) {
                    return Some(format!("Fahrspuren passen nicht zum Nachbar-Spline {nb} ({})", n.sli.rsplit('\\').next().unwrap_or("")));
                }
            }
        }
        None
    }

    fn anwenden(&mut self, v: &mut Viewer, schritte: &[Schritt], nachher: bool) -> Result<()> {
        for s in schritte {
            match if nachher { &s.nachher } else { &s.vorher } {
                Some(b) => std::fs::write(&s.datei, b)?,
                None => {
                    let _ = std::fs::remove_file(&s.datei);
                }
            }
            self.kacheln.remove(&s.kachel);
        }
        self.stand.clear();
        let keys: Vec<(i32, i32)> = schritte.iter().map(|s| s.kachel).collect();
        v.reload_tiles(&keys)?;
        self.aenderungen += 1;
        Ok(())
    }

    pub fn rueckgaengig(&mut self, v: &mut Viewer) -> Result<bool> {
        let Some(s) = self.undo.pop() else { return Ok(false) };
        self.anwenden(v, &s, false)?;
        self.redo.push(s);
        Ok(true)
    }

    pub fn wiederholen(&mut self, v: &mut Viewer) -> Result<bool> {
        let Some(s) = self.redo.pop() else { return Ok(false) };
        self.anwenden(v, &s, true)?;
        self.undo.push(s);
        Ok(true)
    }

    pub fn kann_rueckgaengig(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn kann_wiederholen(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Sitzungskopien der Karte `alt` (fuers Speichern als neue Karte)
    pub fn kopien(&self, alt: &str) -> Vec<PathBuf> {
        let d = self.sitzung.join("maps").join(alt);
        std::fs::read_dir(d).map(|r| r.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect()).unwrap_or_default()
    }
}

fn lesen(v: &Viewer, (tx, ty): (i32, i32)) -> Vec<KartenSpline> {
    let Some(d) = v.tile_file(tx, ty) else { return vec![] };
    let Ok(t) = omsi_map::Tile::load(&d) else { return vec![] };
    let ts = omsi_map::tile_size();
    let o = DVec2::new(tx as f64 * ts, ty as f64 * ts);
    t.splines.iter().filter(|s| !s.deleted && s.length > 0.0 && !s.file.trim().is_empty()).map(|s| KartenSpline {
        id: s.id, kachel: (tx, ty), sli: s.file.trim().to_string(), kurve: omsi_geometry::SplineCurve::from_map(s, o),
        gespiegelt: s.mirror, prev: s.prev_id, next: s.next_id,
    }).collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore]
    fn vorhandene_strasse_aendern() {
        let mut v = crate::bearbeiten::tests::grundorf();
        let mut a = Aendern::neu(&v);
        a.aktualisieren(&v);
        // eine Strasse in der Naehe der Kartenmitte
        let id = (0..40).find_map(|k| a.suchen(&v, DVec2::new(150.0 + k as f64 * 5.0, 150.0))).or_else(|| {
            a.kacheln.values().flatten().find(|s| v.spline_lanes(&s.sli).is_some_and(|l| l.0.iter().any(|x| x.0 == 0))).map(|s| s.id)
        }).expect("keine Strasse gefunden");
        let s = a.spline(id).unwrap().clone();
        println!("Spline {id}: {} ({:.1} m)", s.sli, s.kurve.length);
        a.auswahl = vec![id];
        // Querschnitt aendern: Kachel neu geladen, Sitzungskopie hat die neue Datei, Original unveraendert
        let neu = "Splines\\Marcel\\str_2spur_10m_Grunewaldstr.sli";
        let (n, _) = a.querschnitt(&mut v, neu).unwrap();
        assert_eq!(n, 1);
        a.aktualisieren(&v);
        assert_eq!(a.spline(id).unwrap().sli, neu);
        let original = v.root.join("maps/Grundorf").join(format!("tile_{}_{}.map", s.kachel.0, s.kachel.1));
        let orig_text = dekodieren(&std::fs::read(&original).unwrap()).0;
        assert!(!orig_text.contains(&format!("{neu}\r\n{id}\r\n")) || s.sli == neu, "Original veraendert");
        // spiegeln hin und zurueck
        a.spiegeln(&mut v).unwrap();
        a.aktualisieren(&v);
        assert!(a.spline(id).unwrap().gespiegelt != s.gespiegelt);
        a.rueckgaengig(&mut v).unwrap();
        a.aktualisieren(&v);
        assert_eq!(a.spline(id).unwrap().gespiegelt, s.gespiegelt);
        // loeschen und zurueck
        a.loeschen(&mut v).unwrap();
        a.aktualisieren(&v);
        assert!(a.spline(id).is_none());
        a.rueckgaengig(&mut v).unwrap();
        a.aktualisieren(&v);
        assert_eq!(a.spline(id).unwrap().sli, neu);
        // alles zurueck: die Sitzungskopie entspricht wieder dem Original (bzw. ist weg)
        a.rueckgaengig(&mut v).unwrap();
        a.aktualisieren(&v);
        assert_eq!(a.spline(id).unwrap().sli, s.sli);
        assert_eq!(a.kopien("Grundorf").len(), 0, "Sitzungskopie nach vollstaendigem Rueckgaengig");
        let _ = std::fs::remove_dir_all(&a.sitzung);
    }
}
