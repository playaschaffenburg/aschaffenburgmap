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
    /// naechste freie Objekt-ID fuer Objekte, die direkt in die Sitzungskopie geschrieben werden (World Editor)
    naechste_objekt_id: i64,
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
                  aenderungen: 0, breiten: HashMap::new(), abgetastet: HashMap::new(), mit_spuren: HashMap::new(), naechste_objekt_id: 0, tag: format!("Editor_{:x}{nr}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)) }
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
    /// [rule] no_cars, die der anderen verlieren ihre no_cars-Regeln (Optik und Querschnitt bleiben). Die Pfade der
    /// Kreuzungsobjekte, die (laut Spurnetz) in diese Splines fuehren, folgen: gesperrt, wenn alle Spuren, in die sie
    /// fuehren, gesperrt sind. Ein Rueckgaengig-Schritt -> Anzahl Splines
    pub fn einbahn(&mut self, v: &mut Viewer, art: crate::netz::Einbahn) -> Result<usize> {
        let ids = self.auswahl.clone();
        // je Spline (Kachel, Fahrzeugpfade, gesperrte)
        let mut splines: HashMap<i64, ((i32, i32), Vec<usize>, Vec<usize>)> = HashMap::new();
        for id in &ids {
            let Some(s) = self.spline(*id) else { continue };
            let Some((pfade, _)) = v.spline_lanes(&s.sli) else { continue };
            let pfade: Vec<(u8, u8)> = pfade.iter().map(|x| (x.0, x.4)).collect();
            let fahrzeug: Vec<usize> = pfade.iter().enumerate().filter(|(_, x)| x.0 == 0).map(|(i, _)| i).collect();
            splines.insert(*id, (s.kachel, fahrzeug, art.gesperrt(&pfade, s.gespiegelt)));
        }
        // Kreuzungspfade hinein: (Kachel, Objekt) -> (Pfad, gesperrt)
        let net = &v.lanes;
        let ist_sli = |i: usize| net.lanes[i].name.to_ascii_lowercase().ends_with(".sli");
        let mut objekte: HashMap<((i32, i32), i64), Vec<(usize, bool)>> = HashMap::new();
        for (j, l) in net.lanes.iter().enumerate() {
            let Some(k) = l.key else { continue };
            if !ist_sli(j) || !splines.contains_key(&k.id) {
                continue;
            }
            for &i in net.prev.get(j).map(|x| x.as_slice()).unwrap_or(&[]) {
                let Some(ko) = net.lanes[i].key else { continue };
                if ist_sli(i) || net.lanes[i].kind != l.kind {
                    continue;
                }
                // gesperrt, wenn alle Folgespuren gesperrte Pfade der gewaehlten Splines sind
                let alle = net.lanes[i].next.iter().all(|&n| net.lanes[n].key.is_some_and(|kn| {
                    ist_sli(n) && splines.get(&kn.id).is_some_and(|(_, _, g)| g.contains(&(kn.path as usize)))
                }));
                let e = objekte.entry((ko.tile, ko.id)).or_default();
                if !e.iter().any(|x| x.0 == ko.path as usize) {
                    e.push((ko.path as usize, alle));
                }
            }
        }
        let mut kacheln: Vec<(i32, i32)> = splines.values().map(|x| x.0).chain(objekte.keys().map(|x| x.0)).collect();
        kacheln.sort();
        kacheln.dedup();
        log::info!("KI-Fahrtrichtung ({}): {} Spline(s), Kreuzungspfade hinein {:?}", art.text(), splines.len(), objekte);
        let n = splines.len();
        self.kacheln_aendern(v, &kacheln, |k, z| {
            for (id, (kachel, fahrzeug, sperren)) in &splines {
                if *kachel != k {
                    continue;
                }
                let i = eintrag_finden(z, *id).with_context(|| format!("Spline {id} nicht in seiner Kachel"))?;
                regeln_no_cars(z, eintrag_ende(z, i), fahrzeug, sperren);
            }
            for ((kachel, oid), pfade) in &objekte {
                if *kachel != k {
                    continue;
                }
                let Some(i) = (0..z.len()).find(|&i| z[i].trim().eq_ignore_ascii_case("[object]") && z.get(i + 3).and_then(|x| x.trim().parse::<i64>().ok()) == Some(*oid)) else { continue };
                let betroffen: Vec<usize> = pfade.iter().map(|x| x.0).collect();
                let sperren: Vec<usize> = pfade.iter().filter(|x| x.1).map(|x| x.0).collect();
                regeln_no_cars(z, i + 1, &betroffen, &sperren);
            }
            Ok(1)
        })?;
        Ok(n)
    }

    /// neues Objekt direkt in die Sitzungskopie seiner Kachel schreiben (auf dem Gelaende), ein Rueckgaengig-Schritt
    /// -> (ID, Kachel)
    pub fn objekt_anlegen(&mut self, v: &mut Viewer, rel: &str, pos: DVec2, rot: f64, texte: &[String]) -> Result<(i64, (i32, i32))> {
        let ts = omsi_map::tile_size();
        let k = ((pos.x / ts).floor() as i32, (pos.y / ts).floor() as i32);
        if v.tile_file(k.0, k.1).is_none() {
            anyhow::bail!("an dieser Stelle hat die Karte keine Kachel");
        }
        let id = v.next_object_id().max(self.naechste_objekt_id);
        self.naechste_objekt_id = id + 1;
        let z = crate::kreuzung::zahl;
        let mut eintrag = vec!["[object]".to_string(), "0".into(), rel.to_string(), id.to_string(), z(pos.x - k.0 as f64 * ts),
                               z(pos.y - k.1 as f64 * ts), "0".into(), z(rot.rem_euclid(360.0)), "0".into(), "0".into(), texte.len().to_string()];
        eintrag.extend(texte.iter().cloned());
        self.kacheln_aendern(v, &[k], |_, zeilen| {
            while zeilen.last().is_some_and(|l| l.trim().is_empty()) {
                zeilen.pop();
            }
            zeilen.push(String::new());
            zeilen.extend(eintrag.iter().cloned());
            zeilen.push(String::new());
            Ok(1)
        })?;
        Ok((id, k))
    }

    /// ersten Text eines Objekts setzen (Name einer Haltestelle), ein Rueckgaengig-Schritt
    pub fn objekt_text(&mut self, v: &mut Viewer, kachel: (i32, i32), id: i64, text: &str) -> Result<()> {
        self.kacheln_aendern(v, &[kachel], |_, z| {
            let i = (0..z.len()).find(|&i| z[i].trim().eq_ignore_ascii_case("[object]") && z.get(i + 3).and_then(|x| x.trim().parse::<i64>().ok()) == Some(id))
                .with_context(|| format!("Objekt {id} nicht in seiner Kachel"))?;
            let n: usize = z.get(i + 10).and_then(|x| x.trim().parse().ok()).unwrap_or(0);
            if n == 0 || i + 11 >= z.len() {
                anyhow::bail!("Objekt {id} hat keinen Namen");
            }
            z[i + 11] = text.to_string();
            Ok(1)
        })?;
        Ok(())
    }

    /// Pfade eines Kreuzungsobjekts fuer die KI sperren / freigeben ([rule] no_cars; Werkzeug "Kreuzungen", Spuren) -
    /// ein Rueckgaengig-Schritt
    pub fn objekt_pfade(&mut self, v: &mut Viewer, kachel: (i32, i32), objekt: i64, sperren: &[usize], frei: &[usize]) -> Result<()> {
        let mut betroffen: Vec<usize> = sperren.iter().chain(frei).copied().collect();
        betroffen.sort_unstable();
        betroffen.dedup();
        log::info!("Kreuzungsobjekt {objekt}: Pfade {sperren:?} gesperrt, {frei:?} frei");
        self.kacheln_aendern(v, &[kachel], |_, z| {
            let i = (0..z.len()).find(|&i| z[i].trim().eq_ignore_ascii_case("[object]") && z.get(i + 3).and_then(|x| x.trim().parse::<i64>().ok()) == Some(objekt))
                .with_context(|| format!("Objekt {objekt} nicht in seiner Kachel"))?;
            regeln_no_cars(z, i + 1, &betroffen, sperren);
            Ok(1)
        })?;
        Ok(())
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

/// no_cars-Regeln eines Eintrags (Spline oder Objekt) ab Zeile `ab` bis zum naechsten Eintrag: die der Pfade `betroffen`
/// weg, fuer `sperren` neue (am Ende des Eintrags, vor Leerzeilen - nie zwischen seine Felder)
fn regeln_no_cars(z: &mut Vec<String>, ab: usize, betroffen: &[usize], sperren: &[usize]) {
    let mut j = ab;
    while j < z.len() && !crate::kreuzung::ist_eintrag(&z[j]) {
        j += 1;
    }
    let mut k = ab;
    while k < j {
        if z[k].trim().eq_ignore_ascii_case("[rule]") && k + 4 < z.len() && z[k + 2].trim().eq_ignore_ascii_case("no_cars")
            && z[k + 1].trim().parse::<usize>().is_ok_and(|p| betroffen.contains(&p)) {
            let mut e = k + 5;
            if e < j && z[e].trim().is_empty() {
                e += 1;
            }
            let e = e.min(j);
            z.drain(k..e);
            j -= e - k;
        } else {
            k += 1;
        }
    }
    let mut ende = j;
    while ende > ab && z[ende - 1].trim().is_empty() {
        ende -= 1;
    }
    let mut neu = Vec::new();
    for p in sperren {
        neu.extend([String::new(), "[rule]".to_string(), p.to_string(), "no_cars".into(), "0".into(), "0".into()]);
    }
    z.splice(ende..ende, neu);
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

#[cfg(test)]
mod sperren_tests {
    use super::*;
    use crate::netz::Einbahn;
    use openomsi_game::viewer::LaneKind;

    /// Grundorf: eine Strasse an einer Kreuzung fuer die KI sperren - ihre Fahrspuren und die Pfade der Kreuzung hinein
    /// haben danach no_cars, die anderen Pfade der Kreuzung nicht; "beide" gibt alles wieder frei
    #[test]
    #[ignore]
    fn strasse_fuer_ki_sperren() {
        let _sperre = crate::bearbeiten::tests::sperre();
        let mut v = crate::bearbeiten::tests::grundorf();
        v.tiles_around(DVec3::new(150.0, 150.0, 0.0), 8).unwrap();
        let mut a = Aendern::neu(&v);
        a.aktualisieren(&v);
        let sli = |v: &Viewer, i: usize| v.lanes.lanes[i].name.to_ascii_lowercase().ends_with(".sli");
        // Spline, in dessen Spuren Pfade eines Objekts fuehren
        let (id, objekt) = (0..v.lanes.lanes.len()).find_map(|j| {
            let l = &v.lanes.lanes[j];
            let k = l.key?;
            if !sli(&v, j) || l.kind != LaneKind::Street || a.spline(k.id).is_none() {
                return None;
            }
            v.lanes.prev.get(j)?.iter().find(|&&i| !sli(&v, i) && v.lanes.lanes[i].key.is_some()).map(|&i| (k.id, v.lanes.lanes[i].key.unwrap().id))
        }).expect("Strasse an einer Kreuzung");
        println!("Spline {id}, Kreuzung {objekt}");
        // (Spuren des Splines gesperrt, Pfade des Objekts hinein gesperrt, andere Pfade des Objekts gesperrt)
        let stand = |v: &Viewer| {
            let mut spuren = vec![];
            let mut hinein = vec![];
            let mut andere = vec![];
            for (j, l) in v.lanes.lanes.iter().enumerate() {
                let Some(k) = l.key else { continue };
                if l.kind != LaneKind::Street {
                    continue;
                }
                if k.id == id && sli(v, j) {
                    spuren.push(l.no_cars);
                } else if k.id == objekt && !sli(v, j) {
                    let rein = l.next.iter().any(|&n| v.lanes.lanes[n].key.is_some_and(|x| x.id == id));
                    if rein { hinein.push(l.no_cars) } else { andere.push(l.no_cars) }
                }
            }
            (spuren, hinein, andere)
        };
        let (s0, h0, o0) = stand(&v);
        assert!(!s0.is_empty() && !h0.is_empty() && s0.iter().chain(&h0).all(|x| !x), "{s0:?} {h0:?}");
        a.auswahl = vec![id];
        a.einbahn(&mut v, Einbahn::Gesperrt).unwrap();
        a.aktualisieren(&v);
        let (s1, h1, o1) = stand(&v);
        println!("gesperrt: Spuren {s1:?}, hinein {h1:?}, andere {o1:?}");
        assert!(s1.iter().all(|x| *x) && h1.iter().all(|x| *x), "nicht alles gesperrt");
        assert_eq!(o1, o0, "andere Pfade der Kreuzung veraendert");
        a.einbahn(&mut v, Einbahn::Beide).unwrap();
        a.aktualisieren(&v);
        let (s2, h2, _) = stand(&v);
        assert!(s2.iter().chain(&h2).all(|x| !x), "nicht wieder frei: {s2:?} {h2:?}");
        a.rueckgaengig(&mut v).unwrap();
        a.rueckgaengig(&mut v).unwrap();
        assert_eq!(a.kopien("Grundorf").len(), 0);
    }
}

#[cfg(test)]
mod sperren_nutzer {
    use super::*;
    use crate::netz::Einbahn;

    /// nur in der Sitzung: Spline OMSI_SPLINE der Karte OMSI_KARTE sperren und freigeben - die Kreuzungen an seinen Enden
    /// bleiben, ihre Pfade hinein werden gesperrt und wieder frei
    #[test]
    #[ignore]
    fn spline_der_nutzerkarte_sperren() {
        let _sperre = crate::bearbeiten::tests::sperre();
        let (Ok(karte), Ok(id)) = (std::env::var("OMSI_KARTE"), std::env::var("OMSI_SPLINE")) else { return };
        let id: i64 = id.parse().unwrap();
        let root = Path::new(crate::bearbeiten::tests::OMSI);
        let (mut v, _) = Viewer::open(&openomsi_game::viewer::instance(), None, root, &root.join("maps").join(&karte).join("global.cfg")).unwrap();
        v.tiles_around(DVec3::new(450.0, 300.0, 0.0), 3).unwrap();
        let mut a = Aendern::neu(&v);
        a.aktualisieren(&v);
        let s = a.spline(id).expect("Spline").clone();
        let objekte = |v: &Viewer| -> usize { v.objects().len() };
        let zahl = (objekte(&v), a.kreuzungsobjekt_bei(&v, s.kurve.start.truncate()).map(|k| k.arme.len()), a.kreuzungsobjekt_bei(&v, s.kurve.end_point().truncate()).map(|k| k.arme.len()));
        println!("vorher: {zahl:?}");
        let gesperrt = |v: &Viewer| v.lanes.lanes.iter().filter(|l| l.no_cars).count();
        let g0 = gesperrt(&v);
        a.auswahl = vec![id];
        a.einbahn(&mut v, Einbahn::Gesperrt).unwrap();
        a.aktualisieren(&v);
        let nach = (objekte(&v), a.kreuzungsobjekt_bei(&v, s.kurve.start.truncate()).map(|k| k.arme.len()), a.kreuzungsobjekt_bei(&v, s.kurve.end_point().truncate()).map(|k| k.arme.len()));
        println!("gesperrt: {nach:?}, no_cars {} -> {}", g0, gesperrt(&v));
        assert_eq!(nach, zahl, "Kreuzungen/Objekte veraendert");
        assert!(gesperrt(&v) > g0);
        a.einbahn(&mut v, Einbahn::Beide).unwrap();
        a.aktualisieren(&v);
        println!("frei: no_cars {}", gesperrt(&v));
        assert_eq!(gesperrt(&v), g0, "nicht wieder frei");
        drop(a);
    }
}
