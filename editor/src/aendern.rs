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
    /// Pfade mit [rule] no_cars (fuer die KI gesperrt)
    pub gesperrt: Vec<usize>,
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
    pub(crate) kacheln: HashMap<(i32, i32), Vec<KartenSpline>>,
    stand: Vec<(i32, i32)>,
    pub unter_maus: Option<i64>,
    pub auswahl: Vec<i64>,
    undo: Vec<Vec<Schritt>>,
    redo: Vec<Vec<Schritt>>,
    pub aenderungen: usize,
    breiten: HashMap<String, (f32, f32)>,
    /// abgetastete Mittellinien der Splines (alle ~1 m) mit Umriss-Rechteck, und ob ein Querschnitt Fahrspuren hat -
    /// fuer die Suche nach Querungen bei jeder Mausbewegung (geleert, wenn Kacheln neu gelesen werden)
    pub(crate) abgetastet: HashMap<i64, std::rc::Rc<(Vec<(DVec3, f64)>, DVec2, DVec2)>>,
    pub(crate) mit_spuren: HashMap<String, bool>,
    /// Ordnername der Kreuzungsobjekte dieser Sitzung unter Sceneryobjects/Aschaffenburg_KI (beim Speichern wird
    /// daraus der Name der neuen Karte)
    pub tag: String,
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
                  aenderungen: 0, breiten: HashMap::new(), abgetastet: HashMap::new(), mit_spuren: HashMap::new(), tag: format!("Editor_{:x}{nr}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)) }
    }

    /// Splines der geladenen Kacheln (neu lesen, wenn sich die geladenen Kacheln geaendert haben)
    pub fn aktualisieren(&mut self, v: &Viewer) {
        let mut k = v.loaded_tile_keys();
        k.sort();
        if k == self.stand {
            return;
        }
        self.kacheln.retain(|x, _| k.contains(x));
        self.abgetastet.clear();
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
    pub(crate) fn kopie(&self, v: &Viewer, datei: &Path) -> PathBuf {
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
        let kacheln: Vec<(i32, i32)> = je_kachel.keys().copied().collect();
        self.kacheln_aendern(v, &kacheln, |k, zeilen| {
            let ids = &je_kachel[&k];
            // von hinten, damit Indizes gueltig bleiben
            let mut stellen: Vec<usize> = ids.iter().filter_map(|id| eintrag_finden(zeilen, *id)).collect();
            stellen.sort_unstable();
            stellen.reverse();
            Ok(stellen.into_iter().filter(|&i| f(zeilen, i)).count())
        })
    }

    /// Kacheln `kacheln` als Zeilen bearbeiten (f: Kachel, Zeilen -> Anzahl Aenderungen), als Sitzungskopien
    /// speichern, neu laden; ein Schritt fuer Rueckgaengig
    pub(crate) fn kacheln_aendern(&mut self, v: &mut Viewer, kacheln: &[(i32, i32)],
                                  mut f: impl FnMut((i32, i32), &mut Vec<String>) -> Result<usize>) -> Result<usize> {
        // erst alle Kacheln bearbeiten, dann schreiben: ein Fehler laesst nichts halb geaendert zurueck
        let mut fertig = Vec::new();
        let mut n = 0;
        for &k in kacheln {
            let quelle = v.tile_file(k.0, k.1).with_context(|| format!("Kachel {} {} gibt es in der Karte nicht", k.0, k.1))?;
            let ziel = self.kopie(v, &quelle);
            let bytes = std::fs::read(&quelle)?;
            let vorher = ziel.exists().then(|| bytes.clone());
            let (text, utf16) = dekodieren(&bytes);
            let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
            let mut zeilen: Vec<String> = text.split(eol).map(|s| s.to_string()).collect();
            n += f(k, &mut zeilen)?;
            fertig.push((k, ziel, vorher, kodieren(&zeilen.join(eol), utf16)));
        }
        let mut schritte = Vec::new();
        for (k, ziel, vorher, neu) in fertig {
            std::fs::create_dir_all(ziel.parent().unwrap())?;
            std::fs::write(&ziel, &neu)?;
            schritte.push(Schritt { kachel: k, datei: ziel, vorher, nachher: Some(neu) });
            self.kacheln.remove(&k);
            self.stand.clear();
            self.abgetastet.clear();
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

    /// Fahrtrichtungen der gewaehlten Splines fuer die KI setzen: die Fahrzeugpfade der gesperrten Richtung bekommen
    /// [rule] no_cars, die der anderen verlieren ihre no_cars-Regeln (Optik und Querschnitt bleiben) -> Anzahl Splines
    pub fn einbahn(&mut self, v: &mut Viewer, art: crate::netz::Einbahn) -> Result<usize> {
        let ids = self.auswahl.clone();
        // je Spline: Fahrzeugpfade und welche davon gesperrt werden
        let mut plan: HashMap<i64, (Vec<usize>, Vec<usize>)> = HashMap::new();
        for id in &ids {
            let Some(s) = self.spline(*id) else { continue };
            let Some((pfade, _)) = v.spline_lanes(&s.sli) else { continue };
            let pfade: Vec<(u8, u8)> = pfade.iter().map(|x| (x.0, x.4)).collect();
            let fahrzeug: Vec<usize> = pfade.iter().enumerate().filter(|(_, x)| x.0 == 0).map(|(i, _)| i).collect();
            plan.insert(*id, (fahrzeug, art.gesperrt(&pfade, s.gespiegelt)));
        }
        let ids: Vec<i64> = plan.keys().copied().collect();
        self.umschreiben(v, &ids, move |z, i| {
            let Some(id) = z.get(i + 3).and_then(|x| x.trim().parse::<i64>().ok()) else { return false };
            let Some((fahrzeug, sperren)) = plan.get(&id) else { return false };
            // Ende des Eintrags mit seinen Zusaetzen (Regeln ...) bis zum naechsten Eintrag
            let ende = eintrag_ende(z, i);
            let mut j = ende;
            while j < z.len() && !crate::kreuzung::ist_eintrag(&z[j]) {
                j += 1;
            }
            // vorhandene no_cars-Regeln der Fahrzeugpfade weg
            let mut k = ende;
            while k < j {
                if z[k].trim().eq_ignore_ascii_case("[rule]") && k + 2 < j && z[k + 2].trim().eq_ignore_ascii_case("no_cars")
                    && z[k + 1].trim().parse::<usize>().is_ok_and(|p| fahrzeug.contains(&p)) {
                    let mut e = k + 5;
                    if e < z.len() && z[e].trim().is_empty() {
                        e += 1;
                    }
                    let e = e.min(j);
                    z.drain(k..e);
                    j -= e - k;
                } else {
                    k += 1;
                }
            }
            let mut neu = Vec::new();
            for p in sperren {
                neu.extend(["[rule]".to_string(), p.to_string(), "no_cars".into(), "0".into(), "0".into(), String::new()]);
            }
            z.splice(ende..ende, neu);
            true
        })
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
            self.abgetastet.clear();
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

    pub fn undo_len(&self) -> usize {
        self.undo.len()
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
        gesperrt: s.rules.iter().filter(|r| !r.kill && r.kind.eq_ignore_ascii_case("no_cars") && r.path_index >= 0).map(|r| r.path_index as usize).collect(),
    }).collect()
}

impl Drop for Aendern {
    /// Sitzung zu Ende (Kartenwechsel, Programmende): Ordner abmelden und loeschen
    fn drop(&mut self) {
        omsi_cfg::remove_content_root(&self.sitzung);
        let _ = std::fs::remove_dir_all(&self.sitzung);
        omsi_cfg::content_changed();
    }
}

/// Zeile des Schluesselworts ([spline]/[spline_h]) des Spline-Eintrags `id`
pub(crate) fn eintrag_finden(zeilen: &[String], id: i64) -> Option<usize> {
    (0..zeilen.len()).find(|&i| {
        let w = zeilen[i].trim().to_ascii_lowercase();
        (w == "[spline]" || w == "[spline_h]") && zeilen.get(i + 3).and_then(|z| z.trim().parse::<i64>().ok()) == Some(id)
    })
}

/// erste Zeile nach dem Spline-Eintrag ab `i` (mit einer `mirror`-Zeile)
pub(crate) fn eintrag_ende(zeilen: &[String], i: usize) -> usize {
    let mut j = i + 1;
    while j < zeilen.len() {
        let w = zeilen[j].trim();
        if w.is_empty() || w.starts_with('[') || w.starts_with("Object Nr.") {
            break;
        }
        j += 1;
        if w.eq_ignore_ascii_case("mirror") {
            break;
        }
    }
    j
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

    /// streamen, bis mindestens `kacheln` Kacheln um die Mitte geladen sind und nichts mehr kommt -> geladene Kacheln
    pub fn streamen(v: &mut Viewer, kacheln: usize) -> usize {
        let mitte = DVec3::new(150.0, 150.0, 0.0);
        let t0 = std::time::Instant::now();
        let mut ruhig = 0;
        while t0.elapsed().as_secs() < 60 && ruhig < 60 {
            if v.stream(mitte, 400.0, std::time::Duration::from_millis(50)) || v.first_area_progress().is_some() || v.loaded_tiles() < kacheln || !v.streaming_idle() {
                ruhig = 0;
            } else {
                ruhig += 1;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        v.loaded_tiles()
    }

    /// wie im Fenster (Hintergrund-Streaming): die geaenderte Kachel muss aus der Sitzungskopie kommen
    #[test]
    #[ignore]
    fn aendern_mit_streaming() {
        let _sperre = crate::bearbeiten::tests::sperre();
        let root = std::path::Path::new(crate::bearbeiten::tests::OMSI);
        let (mut v, _) = Viewer::open(&openomsi_game::viewer::instance(), None, root, &root.join("maps/Grundorf/global.cfg")).unwrap();
        let n = streamen(&mut v, 1);
        let mut a = Aendern::neu(&v);
        a.aktualisieren(&v);
        // OMSI_SPLINE=id: einen bestimmten Spline pruefen (4173 liegt in Kachel 1,-1 mit langsamem Neuladen)
        let id = match std::env::var("OMSI_SPLINE").ok().and_then(|x| x.parse().ok()) {
            Some(x) => x,
            None => a.kacheln.values().flatten().find(|s| v.spline_end_free(s.id, true).is_some()).map(|s| s.id).expect("keine Strasse"),
        };
        a.auswahl = vec![id];
        a.loeschen(&mut v).unwrap();
        streamen(&mut v, n);
        assert!(v.spline_end_free(id, true).is_none(), "geloeschter Spline {id} ist nach dem Neuladen noch da");
        a.rueckgaengig(&mut v).unwrap();
        streamen(&mut v, n);
        assert!(v.spline_end_free(id, true).is_some(), "Spline {id} nach Rueckgaengig nicht wieder da");
        drop(a);
    }

    #[test]
    #[ignore]
    fn vorhandene_strasse_aendern() {
        let _sperre = crate::bearbeiten::tests::sperre();
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
        drop(a);
    }
}

#[cfg(test)]
mod einbahn_tests {
    use super::*;
    use crate::netz::Einbahn;

    #[test]
    fn gesperrte_pfade() {
        // Gehweg, Spur mit, Spur gegen die Splinerichtung, Gleis in beide
        let pfade = [(1, 2), (0, 0), (0, 1), (2, 2)];
        assert_eq!(Einbahn::Vor.gesperrt(&pfade, false), vec![2]);
        assert_eq!(Einbahn::Zurueck.gesperrt(&pfade, false), vec![1]);
        assert_eq!(Einbahn::Vor.gesperrt(&pfade, true), vec![1], "gespiegelt laufen die Pfade andersherum");
        assert!(Einbahn::Beide.gesperrt(&pfade, false).is_empty());
    }

    /// Grundorf, Spline 4188 (hat schon [rule] 1 trafficdensity 0.5): Einbahn setzen - laut Spurnetz haben genau die
    /// Spuren gegen die gewaehlte Richtung no_cars; die vorhandene Regel bleibt; Rueckgaengig stellt alles her
    #[test]
    #[ignore]
    fn einbahn_vorhandener_strasse() {
        let _sperre = crate::bearbeiten::tests::sperre();
        let mut v = crate::bearbeiten::tests::grundorf();
        v.tiles_around(DVec3::new(150.0, 150.0, 0.0), 8).unwrap();
        let mut a = Aendern::neu(&v);
        a.aktualisieren(&v);
        let s = a.spline(4188).expect("Spline 4188").clone();
        let richtung = s.kurve.heading_at(s.kurve.length / 2.0);
        // (Spuren des Splines: in Splinerichtung?, no_cars)
        let spuren = |v: &Viewer| -> Vec<(bool, bool)> {
            v.lanes.lanes.iter().filter(|l| l.key.is_some_and(|k| k.id == 4188) && l.kind == openomsi_game::viewer::LaneKind::Street && l.points.len() > 1)
                .map(|l| {
                    let d = (l.points[l.points.len() / 2 + 1] - l.points[l.points.len() / 2]).truncate();
                    (d.dot(crate::netz::dir(richtung)) > 0.0, l.no_cars)
                }).collect()
        };
        let vorher = spuren(&v);
        println!("vorher {vorher:?}, gespiegelt {}", s.gespiegelt);
        assert!(vorher.iter().any(|x| x.0) && vorher.iter().any(|x| !x.0) && vorher.iter().all(|x| !x.1));
        a.auswahl = vec![4188];
        for (art, gesperrt_mit) in [(Einbahn::Vor, false), (Einbahn::Zurueck, true)] {
            a.einbahn(&mut v, art).unwrap();
            a.aktualisieren(&v);
            let jetzt = spuren(&v);
            println!("{art:?}: {jetzt:?}");
            assert!(jetzt.iter().all(|(mit, nc)| *nc == (*mit == gesperrt_mit)), "{art:?}: {jetzt:?}");
        }
        a.einbahn(&mut v, Einbahn::Beide).unwrap();
        a.aktualisieren(&v);
        assert!(spuren(&v).iter().all(|x| !x.1));
        // die Regel der Karte ist noch da
        let kopie = a.sitzung.join("maps/Grundorf").join(format!("tile_{}_{}.map", s.kachel.0, s.kachel.1));
        let t = omsi_map::Tile::load(&kopie).unwrap();
        let sp = t.splines.iter().find(|x| x.id == 4188).unwrap();
        assert!(sp.rules.iter().any(|r| r.kind.eq_ignore_ascii_case("trafficdensity")), "{:?}", sp.rules.iter().map(|r| &r.kind).collect::<Vec<_>>());
        for _ in 0..3 {
            a.rueckgaengig(&mut v).unwrap();
        }
        assert_eq!(a.kopien("Grundorf").len(), 0);
    }
}
