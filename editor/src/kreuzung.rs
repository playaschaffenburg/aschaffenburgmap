//! Kreuzungen an vorhandenen Strassen (wie in Transport Fever 2: eine neue Strasse zweigt mitten aus einer
//! vorhandenen ab). Die Strasse wird an der Stelle aufgeschnitten (die Spline-Kette auch ueber mehrere Splines
//! gekuerzt bzw. geteilt), dazwischen kommt ein Kreuzungsobjekt wie in den Standardkarten - Platte, Abbiegespuren,
//! Vorfahrt - erzeugt von omsigen (python -m omsigen.editorkreuzung), und die neue Strasse beginnt am dritten Arm.
//!
//! Alles geht ueber das Werkzeug "Aendern" (Sitzungskopien der Kacheln, ein Rueckgaengig-Schritt); die Objekte
//! liegen im Sitzungsordner unter Sceneryobjects/Aschaffenburg_KI/<tag>/ und kommen beim Speichern in den Ordner
//! der neuen Karte.

use crate::aendern::{eintrag_ende, eintrag_finden, Aendern, KartenSpline};
use crate::netz::norm180;
use anyhow::{bail, Context, Result};
use glam::{DVec2, DVec3};
use openomsi_game::viewer::Viewer;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// Platz fuer die Bordsteinecken zusaetzlich zu den Strassenbreiten (wie omsigen network.CORNER_ROOM)
pub const ECKENRAUM: f64 = 6.0;
/// kleinster Winkel zwischen Abzweig und Strasse
pub const MIN_WINKEL: f64 = 35.0;
/// kuerzere Reste einer gekuerzten Strasse fallen weg (die Kreuzung wird so viel groesser)
const MIN_REST: f64 = 1.0;

/// Stelle mitten auf einer vorhandenen Strasse, an der eine neue abzweigt
#[derive(Clone, Debug)]
pub struct Abzweig {
    pub spline_id: i64,
    /// Lage auf dem Spline (Meter ab Anfang)
    pub s: f64,
    /// Punkt auf der Mittellinie
    pub pos: DVec3,
    /// Richtung der Strasse dort
    pub richtung: f64,
    pub sli: String,
    /// halbe Breite der Strasse (aussen, groessere Seite)
    pub halb: f64,
}

/// vorhandenes Kreuzungsobjekt der Karte (z. B. eine Standardkreuzung): wird beim Anschliessen einer neuen Strasse
/// durch eine eigene Kreuzung ersetzt, deren Arme die vorhandenen Strassenenden sind
#[derive(Clone, Debug)]
pub struct Vorhanden {
    pub objekt: i64,
    pub kachel: (i32, i32),
    /// Mitte (zwischen den Armen)
    pub pos: DVec3,
    pub arme: Vec<crate::netz::Kartenarm>,
}

/// Vorfahrt eines Arms (wie omsigen vorfahrt.HAUPT/NEBEN/GLEICH)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Rolle {
    Haupt,
    Neben,
    Gleich,
}

impl Rolle {
    pub fn text(self) -> &'static str {
        match self {
            Rolle::Haupt => "haupt",
            Rolle::Neben => "neben",
            Rolle::Gleich => "gleich",
        }
    }
}

/// ein Arm der Kreuzung: Ende einer Strasse an der Kreuzung
#[derive(Clone, Debug)]
pub struct Arm {
    pub pos: DVec3,
    /// Richtung von der Kreuzung weg
    pub richtung: f64,
    pub sli: String,
    /// zeigt die Splinerichtung (bei gespiegelten Splines die umgekehrte) von der Kreuzung weg?
    pub weg: bool,
    pub rolle: Rolle,
}

/// was omsigen gebaut hat
#[derive(Clone, Debug)]
pub struct Objekt {
    pub rel: String,
    pub ursprung: DVec2,
    pub rules: Vec<(usize, i32)>,
    pub spuren: usize,
    pub fehlgeschlagen: usize,
    /// Ampel: Signale und Masten (omsigen ampel.signale), Phase je Arm, Umlauf in s
    pub signale: Vec<Signal>,
    pub phasen: Vec<usize>,
    pub umlauf: Option<f64>,
}

/// Ampelsignal oder Mast einer Kreuzung (wie omsigen ampel.signale)
#[derive(Clone, Debug)]
pub struct Signal {
    /// "signal", "mast" oder "oben" (haengt am Ausleger des Masts `eltern`)
    pub art: String,
    pub datei: String,
    /// Lage (Welt) und Hoehe ueber dem Gelaende - nicht bei "oben"
    pub pos: Option<DVec2>,
    pub hoehe: f64,
    pub rot: f64,
    /// Signalgruppe (Phase), None beim Mast
    pub gruppe: Option<i64>,
    pub eltern: Option<usize>,
    pub anhang: i64,
}

/// eine fertige Kreuzung zum Speichern: Objekt (.sco), Lage, Vorfahrtregeln, Ampeln
#[derive(Clone, Debug)]
pub struct Gesetzt {
    pub rel: String,
    pub pos: DVec3,
    pub rules: Vec<(usize, i32)>,
    pub signale: Vec<Signal>,
}

/// Richtung des Abzweigs: Wunschrichtung, aber mindestens MIN_WINKEL zur Strasse
pub fn arm_richtung(strasse: f64, wunsch: f64) -> f64 {
    let a = norm180(wunsch - strasse);
    let s = if a < 0.0 { -1.0 } else { 1.0 };
    let b = a.abs().clamp(MIN_WINKEL, 180.0 - MIN_WINKEL);
    (strasse + s * b).rem_euclid(360.0)
}

/// Groesse der Kreuzung: (Schnitt je Seite entlang der Strasse, Abstand des neuen Arms von der Mittellinie)
pub fn masse(strasse: f64, arm: f64, halb_strasse: f64, halb_arm: f64) -> (f64, f64) {
    let a = norm180(arm - strasse).abs();
    let t = a.min(180.0 - a).max(MIN_WINKEL).to_radians();
    let (sin, cot) = (t.sin(), t.cos() / t.sin());
    (halb_arm / sin + halb_strasse * cot + ECKENRAUM, halb_strasse / sin + halb_arm * cot + ECKENRAUM)
}

/// ein Spline der Kette mit seiner Lage in der Kette
#[derive(Clone)]
struct Glied {
    s: KartenSpline,
    von: f64,
}

impl Glied {
    fn bis(&self) -> f64 {
        self.von + self.s.kurve.length
    }
}

/// was mit einem Spline der Kette geschieht
enum Schnitt {
    /// bleibt [a, b] (Meter auf dem Spline), mit neuen prev/next
    Kuerzen { a: f64, b: f64, prev: i64, next: i64 },
    /// zerfaellt in [0, a] (alte ID) und [b, L] (neue ID)
    Teilen { a: f64, b: f64, neu: i64 },
    Weg,
}

impl Aendern {
    /// Ordner der Kreuzungsobjekte dieser Sitzung und sein Name unter Sceneryobjects/Aschaffenburg_KI
    pub fn kreuzungs_ordner(&self) -> (PathBuf, String) {
        (self.sitzung.join("Sceneryobjects").join("Aschaffenburg_KI").join(&self.tag), self.tag.clone())
    }

    /// vorhandenes Kreuzungsobjekt unter dem Bodenpunkt: ein Objekt mit Fahrpfaden, an dem Strassen haengen; seine
    /// Arme sind die Enden der Splines, deren Spuren in seine Pfade fuehren (laut openOMSIs Spurnetz)
    pub fn kreuzungsobjekt_bei(&mut self, v: &Viewer, p: DVec2) -> Option<Vorhanden> {
        let net = &v.lanes;
        let ist_objekt = |i: usize| {
            let l = &net.lanes[i];
            l.kind == omsi_sim_lanekind_strasse() && !l.name.to_ascii_lowercase().ends_with(".sli") && l.key.is_some()
        };
        // Objekte mit Fahrpfaden nahe p: (Abstand zur Mitte, Objekt, Kachel, Mitte)
        let mut objekte: HashMap<i64, ((i32, i32), Vec<DVec3>, Vec<usize>)> = HashMap::new();
        for i in 0..net.lanes.len() {
            if !ist_objekt(i) {
                continue;
            }
            let l = &net.lanes[i];
            if !l.points.iter().any(|q| (q.truncate() - p).length() < 40.0) {
                continue;
            }
            let k = l.key.unwrap();
            let e = objekte.entry(k.id).or_insert((k.tile, vec![], vec![]));
            e.1.extend(l.points.iter().copied());
            e.2.push(i);
        }
        let mut best: Option<(f64, i64)> = None;
        for (id, (_, punkte, _)) in &objekte {
            let mitte = punkte.iter().map(|q| q.truncate()).sum::<DVec2>() / punkte.len() as f64;
            let radius = punkte.iter().map(|q| (q.truncate() - mitte).length()).fold(0.0, f64::max);
            let d = (p - mitte).length();
            if d <= radius + 2.0 && best.map(|b| d < b.0).unwrap_or(true) {
                best = Some((d, *id));
            }
        }
        let (_, id) = best?;
        let (kachel, _, spuren) = objekte.remove(&id)?;
        // Spline-Enden, die in die Pfade fuehren bzw. aus ihnen kommen: (Spline, am Ende)
        let mut enden: Vec<(i64, bool)> = Vec::new();
        for &i in &spuren {
            for &n in &net.lanes[i].next {
                let m = &net.lanes[n];
                if let (Some(k), true) = (m.key, m.name.to_ascii_lowercase().ends_with(".sli")) {
                    enden.push((k.id, m.reversed));
                }
            }
            for &q in net.prev.get(i).map(|x| x.as_slice()).unwrap_or(&[]) {
                let m = &net.lanes[q];
                if let (Some(k), true) = (m.key, m.name.to_ascii_lowercase().ends_with(".sli")) {
                    enden.push((k.id, !m.reversed));
                }
            }
        }
        enden.sort();
        enden.dedup();
        let mut arme = Vec::new();
        for (sid, am_ende) in enden {
            let Some(sp) = self.spline(sid).cloned() else { continue };
            let k = &sp.kurve;
            // Richtung von der Kreuzung weg: am Spline-Anfang seine Richtung, am Ende die Gegenrichtung
            let (pos, h) = if am_ende { (k.end_point(), k.heading_at(k.length) + 180.0) } else { (k.start, k.heading_deg) };
            let (l, r) = self.breite(v, &sp.sli);
            arme.push(crate::netz::Kartenarm { pos, richtung: h.rem_euclid(360.0), sli: sp.sli.clone(), weg: (!am_ende) != sp.gespiegelt, halb: l.max(r) as f64 });
        }
        // doppelte Enden derselben Zufahrt (zwei Splines dicht hintereinander, beide mit den Pfaden verbunden): nur das
        // innere zaehlt
        if arme.len() > 1 {
            let n0 = arme.len() as f64;
            let m = DVec2::new(arme.iter().map(|a| a.pos.x).sum::<f64>() / n0, arme.iter().map(|a| a.pos.y).sum::<f64>() / n0);
            arme.sort_by(|a, b| (a.pos.truncate() - m).length().total_cmp(&(b.pos.truncate() - m).length()));
            let mut einzeln: Vec<crate::netz::Kartenarm> = Vec::new();
            for a in arme {
                if !einzeln.iter().any(|b| (b.pos.truncate() - a.pos.truncate()).length() < 3.0 && norm180(b.richtung - a.richtung).abs() < 20.0) {
                    einzeln.push(a);
                }
            }
            arme = einzeln;
        }
        if arme.len() < 2 {
            return None;
        }
        let n = arme.len() as f64;
        let pos = DVec3::new(arme.iter().map(|a| a.pos.x).sum::<f64>() / n, arme.iter().map(|a| a.pos.y).sum::<f64>() / n, arme.iter().map(|a| a.pos.z).sum::<f64>() / n);
        // Mitte: Schnittpunkt der Armrichtungen waere genauer; der Mittelwert der Armenden reicht fuer den Knoten
        Some(Vorhanden { objekt: id, kachel, pos, arme })
    }

    /// vorhandenes Kreuzungsobjekt aus seiner Kachel nehmen, mit allem, was an ihm haengt ([varparent]: Ampeln, ihre
    /// Masten, daran haengende Objekte). Ein Rueckgaengig-Schritt.
    pub fn objekt_entfernen(&mut self, v: &mut Viewer, kachel: (i32, i32), id: i64) -> Result<usize> {
        log::info!("vorhandenes Kreuzungsobjekt {id} (Kachel {} {}) wird durch eine eigene Kreuzung ersetzt", kachel.0, kachel.1);
        self.kacheln_aendern(v, &[kachel], |_, zeilen| objekte_aus_zeilen(zeilen, &[id]))
    }

    /// Abzweig-Stelle unter dem Bodenpunkt (mitten auf einer Strasse)
    pub fn abzweig_bei(&mut self, v: &Viewer, p: DVec2) -> Option<Abzweig> {
        let id = self.suchen(v, p)?;
        let s = self.spline(id)?.clone();
        let k = &s.kurve;
        let n = (k.length / 0.5).ceil().max(1.0) as usize;
        let lage = (0..=n).map(|i| k.length * i as f64 / n as f64)
            .min_by(|a, b| (k.point_at(*a).truncate() - p).length().total_cmp(&(k.point_at(*b).truncate() - p).length()))?;
        let (l, r) = self.breite(v, &s.sli);
        Some(Abzweig { spline_id: id, s: lage, pos: k.point_at(lage), richtung: k.heading_at(lage).rem_euclid(360.0),
                       sli: s.sli.clone(), halb: l.max(r) as f64 })
    }

    /// Schnitt vor und hinter der Kreuzungsmitte (Meter entlang der Strasse) fuer zusaetzliche Arme (Richtung von der
    /// Kreuzung weg, halbe Breite): wie die Kuerzung der Arme im Netz
    pub fn schnitte(ab: &Abzweig, weitere: &[(f64, f64)]) -> (f64, f64) {
        let mut roh = vec![((ab.richtung + 180.0).rem_euclid(360.0), ab.halb), (ab.richtung, ab.halb)];
        roh.extend_from_slice(weitere);
        let d = crate::netz::kuerzungen(&roh);
        (d[0], d[1])
    }

    /// Abzweig so verschieben, dass die Kreuzung (Schnitt `vor` davor, `nach` dahinter) auf die Strasse passt: nahe
    /// einem Ende rueckt sie in die Strasse hinein. Fehler nur, wenn die Strasse (die lueckenlose Kette) zu kurz ist.
    pub fn abzweig_einpassen(&self, ab: &Abzweig, vor: f64, nach: f64) -> Result<Abzweig> {
        let kette = self.kette_um(ab.spline_id, vor + nach + 20.0);
        let g = kette.iter().find(|g| g.s.id == ab.spline_id).context("Spline nicht geladen")?;
        let anfang = kette.first().map(|g| g.von).unwrap_or(0.0) + vor + MIN_REST + 0.5;
        let ende = kette.last().map(|g| g.bis()).unwrap_or(0.0) - nach - MIN_REST - 0.5;
        if ende < anfang {
            bail!("die Strasse ist hier zu kurz fuer eine Kreuzung (braucht {:.0} m bis zum naechsten Ende/zur naechsten Kreuzung)", vor + nach + 2.0 * MIN_REST);
        }
        let mitte = (g.von + ab.s).clamp(anfang, ende);
        Ok(self.abzweig_in_kette(&kette, mitte, ab.halb).unwrap_or_else(|| ab.clone()))
    }

    fn abzweig_in_kette(&self, kette: &[Glied], mitte: f64, halb: f64) -> Option<Abzweig> {
        let g = kette.iter().find(|g| mitte >= g.von - 1e-9 && mitte <= g.bis() + 1e-9)?;
        let s = (mitte - g.von).clamp(0.0, g.s.kurve.length);
        Some(Abzweig { spline_id: g.s.id, s, pos: g.s.kurve.point_at(s), richtung: g.s.kurve.heading_at(s).rem_euclid(360.0),
                       sli: g.s.sli.clone(), halb })
    }

    /// Splines (IDs) der lueckenlosen Kette um `id` bis `weit` Meter zu jeder Seite
    pub fn kette_ids(&self, id: i64, weit: f64) -> Vec<i64> {
        self.kette_um(id, weit).into_iter().map(|g| g.s.id).collect()
    }

    /// Kette um Spline `id`: Nachbarn ueber prev/next, soweit geladen und lueckenlos (Ende trifft Anfang in Lage
    /// und Richtung), hoechstens `weit` Meter zu jeder Seite
    fn kette_um(&self, id: i64, weit: f64) -> Vec<Glied> {
        let Some(mitte) = self.spline(id).cloned() else { return vec![] };
        let passt = |a: &KartenSpline, b: &KartenSpline| {
            (a.kurve.end_point() - b.kurve.start).length() < 0.3
                && norm180(a.kurve.heading_at(a.kurve.length) - b.kurve.heading_deg).abs() < 2.0
        };
        let mut vor: Vec<KartenSpline> = Vec::new();
        let (mut cur, mut l) = (mitte.clone(), 0.0);
        while l < weit {
            let Some(p) = self.spline(cur.prev).filter(|p| cur.prev != 0 && p.next == cur.id && passt(p, &cur)).cloned() else { break };
            if p.id == id || vor.iter().any(|x| x.id == p.id) {
                break;
            }
            l += p.kurve.length;
            vor.push(p.clone());
            cur = p;
        }
        let mut nach: Vec<KartenSpline> = Vec::new();
        let (mut cur, mut l) = (mitte.clone(), 0.0);
        while l < weit {
            let Some(n) = self.spline(cur.next).filter(|n| cur.next != 0 && n.prev == cur.id && passt(&cur, n)).cloned() else { break };
            if n.id == id || vor.iter().chain(nach.iter()).any(|x| x.id == n.id) {
                break;
            }
            l += n.kurve.length;
            nach.push(n.clone());
            cur = n;
        }
        let mut out = Vec::new();
        let mut von = -vor.iter().map(|s| s.kurve.length).sum::<f64>();
        for s in vor.into_iter().rev().chain(std::iter::once(mitte)).chain(nach) {
            let l = s.kurve.length;
            out.push(Glied { s, von });
            von += l;
        }
        out
    }

    /// Vorhandene Strasse an der Abzweig-Stelle fuer eine Kreuzung aufschneiden: `vor` Meter davor bis `nach` Meter
    /// dahinter faellt weg (auch ueber mehrere verknuepfte Splines; das Stueck dahinter bekommt eine neue ID, die
    /// Nachbarn werden umgehaengt), die Enden laufen eben auf Kreuzungshoehe ein. Ein Rueckgaengig-Schritt.
    /// -> die zwei Enden als Arme der Kreuzung (das Kreuzungsobjekt setzt das Netz, strasse.rs)
    pub fn aufschneiden(&mut self, v: &mut Viewer, ab: &Abzweig, vor: f64, nach: f64) -> Result<Vec<crate::netz::Kartenarm>> {
        let t0 = std::time::Instant::now();
        log::info!("Strasse aufschneiden: Spline {} bei {:.1} m ({}), {:.1} m davor, {:.1} m dahinter", ab.spline_id, ab.s, ab.sli, vor, nach);
        crate::protokoll::aktion(&format!("Strasse aufschneiden: Spline {}", ab.spline_id));
        let kette = self.kette_um(ab.spline_id, vor.max(nach) + 5.0);
        log::info!("  Kette: {} Splines {:?}", kette.len(), kette.iter().map(|g| g.s.id).collect::<Vec<_>>());
        let g = kette.iter().find(|g| g.s.id == ab.spline_id).context("Spline nicht geladen")?;
        let mitte = g.von + ab.s;
        let (mut lo, mut hi) = (mitte - vor, mitte + nach);
        let anfang = kette.first().map(|g| g.von).unwrap_or(0.0);
        let ende = kette.last().map(|g| g.bis()).unwrap_or(0.0);
        if lo < anfang + MIN_REST || hi > ende - MIN_REST {
            bail!("zu nah am Ende der Strasse oder an einer Kreuzung");
        }
        // Reste unter MIN_REST fallen weg
        for g in &kette {
            if lo > g.von && lo - g.von < MIN_REST {
                lo = g.von;
            }
            if hi < g.bis() && g.bis() - hi < MIN_REST {
                hi = g.bis();
            }
        }
        // Hoehe der Kreuzung: die der Mittellinie an der Abzweig-Stelle; die gekuerzten Enden laufen eben ein
        let hoehe = ab.pos.z;
        let punkt = |x: f64| -> (DVec3, f64, &Glied) {
            let g = kette.iter().find(|g| x >= g.von - 1e-6 && x <= g.bis() + 1e-6).unwrap();
            let s = (x - g.von).clamp(0.0, g.s.kurve.length);
            (g.s.kurve.point_at(s), g.s.kurve.heading_at(s), g)
        };
        let (p_lo, h_lo, g_lo) = punkt(lo);
        let (p_hi, h_hi, g_hi) = punkt(hi);
        let mut ids = v.next_object_id();
        let mut neue_id = || {
            ids += 1;
            ids - 1
        };
        // was mit jedem Spline geschieht
        let mut schnitte: Vec<(KartenSpline, Schnitt)> = Vec::new();
        for g in &kette {
            let (a, b) = (g.von, g.bis());
            let l = g.s.kurve.length;
            let s = if b <= lo + 1e-6 || a >= hi - 1e-6 {
                continue;
            } else if a >= lo - 1e-6 && b <= hi + 1e-6 {
                Schnitt::Weg
            } else if a < lo && b > hi {
                Schnitt::Teilen { a: lo - a, b: hi - a, neu: neue_id() }
            } else if a < lo {
                Schnitt::Kuerzen { a: 0.0, b: lo - a, prev: g.s.prev, next: 0 }
            } else {
                Schnitt::Kuerzen { a: hi - a, b: l, prev: 0, next: g.s.next }
            };
            schnitte.push((g.s.clone(), s));
        }
        let halb = ab.halb;
        let arme = vec![
            crate::netz::Kartenarm { pos: p_lo.truncate().extend(hoehe), richtung: (h_lo + 180.0).rem_euclid(360.0), sli: g_lo.s.sli.clone(), weg: g_lo.s.gespiegelt, halb },
            crate::netz::Kartenarm { pos: p_hi.truncate().extend(hoehe), richtung: h_hi.rem_euclid(360.0), sli: g_hi.s.sli.clone(), weg: !g_hi.s.gespiegelt, halb },
        ];
        // Kacheln umschreiben
        let ts = omsi_map::tile_size();
        let kachel_von = |p: DVec2| ((p.x / ts).floor() as i32, (p.y / ts).floor() as i32);
        let mut anhaengen: BTreeMap<(i32, i32), Vec<Vec<String>>> = BTreeMap::new();
        let mut aendern: BTreeMap<(i32, i32), Vec<(i64, Option<Vec<String>>)>> = BTreeMap::new();
        let mut verweise: Vec<(i64, i64)> = Vec::new(); // (Spline, neuer prev)
        for (s, sch) in &schnitte {
            match sch {
                Schnitt::Weg => aendern.entry(s.kachel).or_default().push((s.id, None)),
                Schnitt::Kuerzen { a, b, prev, next } => {
                    let ziel = if *a > 0.0 { Some(hoehe) } else { None };
                    let ziel_ende = if *b < s.kurve.length { Some(hoehe) } else { None };
                    let k = kachel_von(s.kurve.point_at(*a).truncate());
                    let z = datensatz(s, *a, *b, s.id, *prev, *next, k, ziel, ziel_ende);
                    if k == s.kachel {
                        aendern.entry(s.kachel).or_default().push((s.id, Some(z)));
                    } else {
                        aendern.entry(s.kachel).or_default().push((s.id, None));
                        anhaengen.entry(k).or_default().push(z);
                    }
                }
                Schnitt::Teilen { a, b, neu } => {
                    aendern.entry(s.kachel).or_default().push((s.id, Some(datensatz(s, 0.0, *a, s.id, s.prev, 0, s.kachel, None, Some(hoehe)))));
                    let k = kachel_von(s.kurve.point_at(*b).truncate());
                    anhaengen.entry(k).or_default().push(datensatz(s, *b, s.kurve.length, *neu, 0, s.next, k, Some(hoehe), None));
                    if s.next != 0 {
                        verweise.push((s.next, *neu));
                    }
                }
            }
        }
        // Nachbarn, deren prev auf den geteilten Spline zeigte
        let mut prev_neu: BTreeMap<(i32, i32), Vec<(i64, i64)>> = BTreeMap::new();
        for (id, neu) in verweise {
            let s = self.spline(id).context("Nachbar-Spline nicht geladen")?;
            prev_neu.entry(s.kachel).or_default().push((id, neu));
        }
        let mut kacheln: Vec<(i32, i32)> = aendern.keys().chain(anhaengen.keys()).chain(prev_neu.keys()).copied().collect();
        kacheln.sort();
        kacheln.dedup();
        crate::protokoll::aktion(&format!("Strasse aufschneiden: Kacheln {kacheln:?} umschreiben und neu laden"));
        self.kacheln_aendern(v, &kacheln, |k, zeilen| {
            if !zeilen.iter().take(20).position(|l| l.trim().eq_ignore_ascii_case("[version]"))
                .and_then(|i| zeilen.get(i + 1)).and_then(|l| l.trim().parse::<i32>().ok()).is_some_and(|x| x >= 11) {
                bail!("Kachel {} {}: altes Kachelformat (vor Version 11) wird nicht umgeschrieben", k.0, k.1);
            }
            for (id, neu) in prev_neu.get(&k).into_iter().flatten() {
                let i = eintrag_finden(zeilen, *id).context("Nachbar-Spline nicht in seiner Kachel")?;
                zeilen[i + 4] = neu.to_string();
            }
            for (id, ersatz) in aendern.get(&k).into_iter().flatten() {
                let i = eintrag_finden(zeilen, *id).with_context(|| format!("Spline {id} nicht in seiner Kachel"))?;
                let j = eintrag_ende(zeilen, i);
                match ersatz {
                    Some(z) => {
                        // die Erkennungszeile (Detailstufe) bleibt wie sie war
                        let mut z = z.clone();
                        z[1] = zeilen[i + 1].clone();
                        zeilen.splice(i..j, z);
                    }
                    None => {
                        // ganzer Eintrag mit seinen Zusaetzen ([rule], [spline_terrain_align] ...) bis zum naechsten
                        let mut j = j;
                        while j < zeilen.len() && !ist_eintrag(&zeilen[j]) {
                            j += 1;
                        }
                        zeilen.drain(i..j);
                    }
                }
            }
            while zeilen.last().is_some_and(|l| l.trim().is_empty()) {
                zeilen.pop();
            }
            for z in anhaengen.get(&k).into_iter().flatten() {
                zeilen.push(String::new());
                zeilen.extend(z.iter().cloned());
            }
            zeilen.push(String::new());
            Ok(1)
        })?;
        log::info!("  Kacheln {kacheln:?} umgeschrieben ({} ms)", t0.elapsed().as_millis());
        crate::protokoll::aktion("");
        Ok(arme)
    }

    /// Kreisverkehr (Mitte mit Ringhoehe, Radius) ueber vorhandenen Strassen: was von ihnen innerhalb des Rings liegt,
    /// faellt weg - Strassenstuecke und Kreuzungsobjekte mit ihren Ampeln -, an jeder Querung (Abzweig, Platz davor,
    /// dahinter) bleibt das aeussere Ende so weit vor dem Ring, dass die Einmuendung Platz hat, eben auf Ringhoehe.
    /// Alles in einem Schreibvorgang (ein Rueckgaengig-Schritt). -> je Querung der Arm fuer die Kreuzung am Ring
    pub fn ring_ausschneiden(&mut self, v: &mut Viewer, mitte: DVec3, r: f64, querungen: &[(Abzweig, f64, f64)]) -> Result<Vec<crate::netz::Kartenarm>> {
        let m2 = mitte.truncate();
        let hoehe = mitte.z;
        log::info!("Kreisverkehr bei {:.1} {:.1} (R {r:.1}) ueber {} vorhandene Strasse(n)", m2.x, m2.y, querungen.len());
        crate::protokoll::aktion("Kreisverkehr ueber vorhandenen Strassen");
        // je Querung: Schnittstelle (Spline, Meter) und ob es auf der Kette mit steigenden Metern nach innen geht
        let mut punkte: Vec<(i64, f64, bool)> = Vec::new();
        let mut arme = Vec::new();
        for (ab, vor, nach) in querungen {
            let sp = self.spline(ab.spline_id).context("Spline nicht geladen")?.clone();
            let k = &sp.kurve;
            let abstand = |x: f64| (k.point_at(x.clamp(0.0, k.length)).truncate() - m2).length();
            let plus = abstand(ab.s + 1.0) < abstand(ab.s - 1.0);
            let d = if plus { *vor } else { *nach };
            let kette = self.kette_um(ab.spline_id, d + 10.0);
            let g0 = kette.iter().find(|g| g.s.id == ab.spline_id).context("Spline nicht in seiner Kette")?;
            let x = g0.von + ab.s + if plus { -d } else { d };
            let (anfang, ende) = (kette.first().unwrap().von, kette.last().unwrap().bis());
            if (plus && x < anfang + MIN_REST) || (!plus && x > ende - MIN_REST) {
                bail!("die Strasse bei {:.0} {:.0} ist ausserhalb des Rings zu kurz fuer die Einmuendung", ab.pos.x, ab.pos.y);
            }
            // das bleibende Stueck liegt aussen: bei "plus" davor (endet an der Schnittstelle), sonst dahinter
            let aussen = if plus { x - 1e-6 } else { x + 1e-6 };
            let g = kette.iter().find(|g| aussen >= g.von - 1e-6 && aussen <= g.bis() + 1e-6).context("Schnittstelle nicht auf der Strasse")?;
            let si = (x - g.von).clamp(0.0, g.s.kurve.length);
            let h = g.s.kurve.heading_at(si);
            let pos = g.s.kurve.point_at(si).truncate().extend(hoehe);
            let (l, rr) = self.breite(v, &g.s.sli);
            arme.push(crate::netz::Kartenarm {
                pos, richtung: if plus { (h + 180.0).rem_euclid(360.0) } else { h.rem_euclid(360.0) },
                sli: g.s.sli.clone(), weg: if plus { g.s.gespiegelt } else { !g.s.gespiegelt }, halb: l.max(rr) as f64,
            });
            punkte.push((g.s.id, si, plus));
        }
        // wegfallende Bereiche je Spline (Meter von, bis): von jeder Schnittstelle nach innen bis zur Schnittstelle der
        // Gegenseite (die Strasse fuehrt durch den Ring) oder zum Ende der Kette (Kreuzung oder Ende im Ring)
        let mut weg: HashMap<i64, Vec<(f64, f64)>> = HashMap::new();
        for (i, &(id, si, plus)) in punkte.iter().enumerate() {
            let kette = self.kette_um(id, 4.0 * r + 200.0);
            let x0 = kette.iter().find(|g| g.s.id == id).map(|g| g.von + si).context("Spline nicht in seiner Kette")?;
            let mut x1 = if plus { kette.last().unwrap().bis() } else { kette.first().unwrap().von };
            for (j, &(id2, s2, plus2)) in punkte.iter().enumerate() {
                let Some(g) = kette.iter().find(|g| g.s.id == id2) else { continue };
                let x2 = g.von + s2;
                if j != i && plus && !plus2 && x2 > x0 {
                    x1 = x1.min(x2);
                }
                if j != i && !plus && plus2 && x2 < x0 {
                    x1 = x1.max(x2);
                }
            }
            let (lo, hi) = if plus { (x0, x1) } else { (x1, x0) };
            for g in &kette {
                let (a, b) = (lo.max(g.von), hi.min(g.bis()));
                if b - a > 1e-6 {
                    weg.entry(g.s.id).or_default().push((a - g.von, b - g.von));
                }
            }
        }
        // Reste ganz im Ring (Stummel an einer Kreuzung im Ring) fallen auch weg
        let kandidaten: Vec<KartenSpline> = self.kacheln.values().flatten().filter(|x| (x.kurve.start.truncate() - m2).length() < r + x.kurve.length).cloned().collect();
        for sp in &kandidaten {
            if weg.contains_key(&sp.id) || !v.spline_lanes(&sp.sli).is_some_and(|(l, _)| l.iter().any(|x| x.0 == 0)) {
                continue;
            }
            if sp.punkte().iter().all(|(q, _)| (q.truncate() - m2).length() < r) {
                weg.insert(sp.id, vec![(0.0, sp.kurve.length)]);
            }
        }
        // Kreuzungsobjekte im Ring (Mitte ihrer Fahrpfade innerhalb)
        let mut objekte: BTreeMap<(i32, i32), Vec<i64>> = BTreeMap::new();
        {
            let mut je: HashMap<i64, ((i32, i32), DVec2, usize)> = HashMap::new();
            for l in &v.lanes.lanes {
                if l.kind != omsi_sim_lanekind_strasse() || l.name.to_ascii_lowercase().ends_with(".sli") {
                    continue;
                }
                let Some(k) = l.key else { continue };
                let e = je.entry(k.id).or_insert((k.tile, DVec2::ZERO, 0));
                for q in &l.points {
                    e.1 += q.truncate();
                    e.2 += 1;
                }
            }
            for (id, (kachel, summe, n)) in je {
                if n > 0 && (summe / n as f64 - m2).length() < r - 1.0 {
                    objekte.entry(kachel).or_default().push(id);
                }
            }
        }
        // Splines umschreiben: bleibende Stuecke
        let ts = omsi_map::tile_size();
        let kachel_von = |p: DVec2| ((p.x / ts).floor() as i32, (p.y / ts).floor() as i32);
        let mut ids = v.next_object_id();
        let mut aendern: BTreeMap<(i32, i32), Vec<(i64, Option<Vec<String>>)>> = BTreeMap::new();
        let mut anhaengen: BTreeMap<(i32, i32), Vec<Vec<String>>> = BTreeMap::new();
        let mut prev_neu: BTreeMap<(i32, i32), Vec<(i64, i64)>> = BTreeMap::new();
        for (id, mut bereiche) in weg {
            let Some(sp) = self.spline(id).cloned() else { continue };
            let l = sp.kurve.length;
            bereiche.sort_by(|a, b| a.0.total_cmp(&b.0));
            // bleibende Stuecke: das Komplement (Reste unter 0.3 m fallen weg)
            let mut bleibt: Vec<(f64, f64)> = Vec::new();
            let mut x = 0.0;
            for (a, b) in &bereiche {
                if *a - x > 0.3 {
                    bleibt.push((x, *a));
                }
                x = x.max(*b);
            }
            if l - x > 0.3 {
                bleibt.push((x, l));
            }
            let mut an_ort = false;
            let n = bleibt.len();
            for (i, (a, b)) in bleibt.iter().enumerate() {
                let nid = if i == 0 { sp.id } else { ids += 1; ids - 1 };
                let prev = if *a <= 1e-6 { sp.prev } else { 0 };
                let next = if *b >= l - 1e-6 { sp.next } else { 0 };
                let k = kachel_von(sp.kurve.point_at(*a).truncate());
                let za = (*a > 1e-6).then_some(hoehe);
                let zb = (*b < l - 1e-6).then_some(hoehe);
                let z = datensatz(&sp, *a, *b, nid, prev, next, k, za, zb);
                if nid == sp.id && k == sp.kachel {
                    aendern.entry(sp.kachel).or_default().push((sp.id, Some(z)));
                    an_ort = true;
                } else {
                    anhaengen.entry(k).or_default().push(z);
                }
                // der Nachfolger zeigte mit prev auf diesen Spline: jetzt auf das Stueck am Ende
                if i + 1 == n && *b >= l - 1e-6 && nid != sp.id && sp.next != 0 {
                    if let Some(nb) = self.spline(sp.next).filter(|nb| nb.prev == sp.id) {
                        prev_neu.entry(nb.kachel).or_default().push((nb.id, nid));
                    }
                }
            }
            if !an_ort {
                aendern.entry(sp.kachel).or_default().push((sp.id, None));
            }
        }
        let mut kacheln: Vec<(i32, i32)> = aendern.keys().chain(anhaengen.keys()).chain(prev_neu.keys()).chain(objekte.keys()).copied().collect();
        kacheln.sort();
        kacheln.dedup();
        log::info!("  {} Spline(s) gekuerzt/entfernt, {} Kreuzungsobjekt(e) im Ring entfernt, Kacheln {kacheln:?}",
                   aendern.values().map(|x| x.len()).sum::<usize>(), objekte.values().map(|x| x.len()).sum::<usize>());
        self.kacheln_aendern(v, &kacheln, |k, zeilen| {
            if !zeilen.iter().take(20).position(|l| l.trim().eq_ignore_ascii_case("[version]"))
                .and_then(|i| zeilen.get(i + 1)).and_then(|l| l.trim().parse::<i32>().ok()).is_some_and(|x| x >= 11) {
                bail!("Kachel {} {}: altes Kachelformat (vor Version 11) wird nicht umgeschrieben", k.0, k.1);
            }
            if let Some(ids) = objekte.get(&k) {
                objekte_aus_zeilen(zeilen, ids)?;
            }
            for (id, neu) in prev_neu.get(&k).into_iter().flatten() {
                let i = eintrag_finden(zeilen, *id).context("Nachbar-Spline nicht in seiner Kachel")?;
                zeilen[i + 4] = neu.to_string();
            }
            for (id, ersatz) in aendern.get(&k).into_iter().flatten() {
                let i = eintrag_finden(zeilen, *id).with_context(|| format!("Spline {id} nicht in seiner Kachel"))?;
                let j = eintrag_ende(zeilen, i);
                match ersatz {
                    Some(z) => {
                        let mut z = z.clone();
                        z[1] = zeilen[i + 1].clone();
                        zeilen.splice(i..j, z);
                    }
                    None => {
                        let mut j = j;
                        while j < zeilen.len() && !ist_eintrag(&zeilen[j]) {
                            j += 1;
                        }
                        zeilen.drain(i..j);
                    }
                }
            }
            while zeilen.last().is_some_and(|l| l.trim().is_empty()) {
                zeilen.pop();
            }
            for z in anhaengen.get(&k).into_iter().flatten() {
                zeilen.push(String::new());
                zeilen.extend(z.iter().cloned());
            }
            zeilen.push(String::new());
            Ok(1)
        })?;
        crate::protokoll::aktion("");
        Ok(arme)
    }

    /// Kreuzungsstellen der Kurve `punkte` (alle ~1 m: Lage mit Hoehe, Meter ab Anfang) mit vorhandenen Strassen
    /// (Splines mit Fahrspuren) auf gleicher Hoehe (bis 3 m; darueber: Bruecke/Tunnel, keine Kreuzung)
    /// -> (Meter auf der Kurve, Abzweig auf der vorhandenen Strasse), nach Lage sortiert
    pub fn kreuzungen_mit(&mut self, v: &Viewer, punkte: &[(DVec3, f64)]) -> Vec<(f64, Abzweig)> {
        if punkte.len() < 2 {
            return vec![];
        }
        let (lo, hi) = punkte.iter().fold((DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY)), |(a, b), (p, _)| (a.min(p.truncate()), b.max(p.truncate())));
        let kandidaten: Vec<KartenSpline> = self.kacheln.values().flatten()
            .filter(|s| {
                let r = s.kurve.length + 20.0;
                let p = s.kurve.start.truncate();
                p.x > lo.x - r && p.x < hi.x + r && p.y > lo.y - r && p.y < hi.y + r
            })
            .cloned().collect();
        let mut out: Vec<(f64, Abzweig)> = Vec::new();
        for sp in kandidaten {
            let strasse = match self.mit_spuren.get(&sp.sli) {
                Some(x) => *x,
                None => {
                    let x = v.spline_lanes(&sp.sli).is_some_and(|(l, _)| l.iter().any(|x| x.0 == 0));
                    self.mit_spuren.insert(sp.sli.clone(), x);
                    x
                }
            };
            if !strasse {
                continue;
            }
            let k = &sp.kurve;
            let q = self.abgetastet.entry(sp.id).or_insert_with(|| {
                let n = (k.length / 1.0).ceil().max(1.0) as usize;
                let q: Vec<(DVec3, f64)> = (0..=n).map(|i| {
                    let s = k.length * i as f64 / n as f64;
                    (k.point_at(s), s)
                }).collect();
                let (a, b) = q.iter().fold((DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY)), |(a, b), (p, _)| (a.min(p.truncate()), b.max(p.truncate())));
                std::rc::Rc::new((q, a, b))
            }).clone();
            let (q, qlo, qhi) = (&q.0, q.1, q.2);
            // Umriss-Rechtecke ueberschneiden sich nicht: keine Querung
            if qlo.x > hi.x || qhi.x < lo.x || qlo.y > hi.y || qhi.y < lo.y {
                continue;
            }
            for w in punkte.windows(2) {
                let (wlo, whi) = (w[0].0.truncate().min(w[1].0.truncate()), w[0].0.truncate().max(w[1].0.truncate()));
                if wlo.x > qhi.x || whi.x < qlo.x || wlo.y > qhi.y || whi.y < qlo.y {
                    continue;
                }
                for u in q.windows(2) {
                    let Some((t, r)) = schnitt_strecken(w[0].0.truncate(), w[1].0.truncate(), u[0].0.truncate(), u[1].0.truncate()) else { continue };
                    let z1 = w[0].0.z + (w[1].0.z - w[0].0.z) * t;
                    let z2 = u[0].0.z + (u[1].0.z - u[0].0.z) * r;
                    if (z1 - z2).abs() > 3.0 {
                        continue;
                    }
                    let s_neu = w[0].1 + (w[1].1 - w[0].1) * t;
                    let s_alt = u[0].1 + (u[1].1 - u[0].1) * r;
                    if out.iter().any(|(x, _)| (x - s_neu).abs() < 1.0) {
                        continue;
                    }
                    let (l, rr) = self.breite(v, &sp.sli);
                    out.push((s_neu, Abzweig { spline_id: sp.id, s: s_alt, pos: k.point_at(s_alt), richtung: k.heading_at(s_alt).rem_euclid(360.0),
                                               sli: sp.sli.clone(), halb: l.max(rr) as f64 }));
                }
            }
        }
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        out
    }
}

/// Schnittpunkt der Strecken a-b und c-d -> (Anteil auf a-b, Anteil auf c-d)
pub fn schnitt_strecken(a: DVec2, b: DVec2, c: DVec2, d: DVec2) -> Option<(f64, f64)> {
    let r = b - a;
    let s = d - c;
    let n = r.perp_dot(s);
    if n.abs() < 1e-12 {
        return None;
    }
    let t = (c - a).perp_dot(s) / n;
    let u = (c - a).perp_dot(r) / n;
    ((0.0..1.0).contains(&t) && (0.0..1.0).contains(&u)).then_some((t, u))
}

fn omsi_sim_lanekind_strasse() -> openomsi_game::viewer::LaneKind {
    openomsi_game::viewer::LaneKind::Street
}

/// beginnt mit dieser Zeile ein neuer Eintrag der Kachel (Spline, Objekt, ...)?
/// Objekte `ids` aus den Zeilen einer Kachel nehmen, mit allem, was an ihnen haengt ([varparent]: Ampeln, ihre
/// Masten, daran haengende Objekte) -> Anzahl entfernter Eintraege
fn objekte_aus_zeilen(zeilen: &mut Vec<String>, ids: &[i64]) -> Result<usize> {
        // Eintraege: (Anfang inkl. "Object Nr."-Zeile, Ende, ID, Eltern ueber varparent/attachObj)
        let mut eintraege: Vec<(usize, usize, i64, Vec<i64>)> = Vec::new();
        let mut lagen: Vec<Option<(f64, f64)>> = Vec::new();
        let mut i = 0;
        while i < zeilen.len() {
            let w = zeilen[i].trim().to_ascii_lowercase();
            if w == "[object]" || w == "[attachobj]" {
                let anfang = if i > 0 && zeilen[i - 1].trim_start().starts_with("Object Nr.") { i - 1 } else { i };
                let mut j = i + 1;
                while j < zeilen.len() && !ist_eintrag(&zeilen[j]) {
                    j += 1;
                }
                let oid = zeilen.get(i + 3).and_then(|z| z.trim().parse::<i64>().ok()).unwrap_or(-1);
                let mut eltern = Vec::new();
                if w == "[attachobj]" {
                    if let Some(e) = zeilen.get(i + 4).and_then(|z| z.trim().parse::<i64>().ok()) {
                        eltern.push(e);
                    }
                }
                for k in i..j {
                    if zeilen[k].trim().eq_ignore_ascii_case("[varparent]") {
                        if let Some(e) = zeilen.get(k + 1).and_then(|z| z.trim().parse::<i64>().ok()) {
                            eltern.push(e);
                        }
                    }
                }
                let x = zeilen.get(i + 4).and_then(|z| z.trim().parse::<f64>().ok());
                let y = zeilen.get(i + 5).and_then(|z| z.trim().parse::<f64>().ok());
                lagen.push(x.zip(y).filter(|_| w == "[object]"));
                eintraege.push((anfang, j, oid, eltern));
                i = j;
            } else {
                i += 1;
            }
        }
        let mut weg: std::collections::HashSet<i64> = ids.iter().copied().collect();
        loop {
            let n = weg.len();
            for e in &eintraege {
                if e.3.iter().any(|x| weg.contains(x)) {
                    weg.insert(e.2);
                }
            }
            if weg.len() == n {
                break;
            }
        }
        // Masten der entfernten Ampeln: eigene Objekte genau an deren Stelle (ohne Verknuepfung)
        let ampeln: Vec<(f64, f64)> = eintraege.iter().zip(&lagen).filter(|(e, _)| !ids.contains(&e.2) && weg.contains(&e.2)).filter_map(|(_, l)| *l).collect();
        for (e, l) in eintraege.iter().zip(&lagen) {
            if let Some((x, y)) = l {
                if ampeln.iter().any(|(ax, ay)| (ax - x).hypot(ay - y) < 0.6) {
                    weg.insert(e.2);
                }
            }
        }
        if !ids.iter().all(|id| eintraege.iter().any(|e| e.2 == *id)) {
            bail!("Kreuzungsobjekt {ids:?} nicht in seiner Kachel");
        }
        let mut bereiche: Vec<(usize, usize)> = eintraege.iter().filter(|e| weg.contains(&e.2)).map(|e| (e.0, e.1)).collect();
        bereiche.sort();
        bereiche.reverse();
        for (a, b) in &bereiche {
            zeilen.drain(*a..*b);
        }
        log::info!("  {} Eintraege entfernt (Objekt und Ampeln/Masten)", bereiche.len());
        Ok(bereiche.len())
}

pub(crate) fn ist_eintrag(z: &str) -> bool {
    let w = z.trim().to_ascii_lowercase();
    w.starts_with("object nr.") || ["[spline]", "[spline_h]", "[object]", "[splineattachement]", "[splineattachement_repeater]", "[attachobj]"].contains(&w.as_str())
}

/// [spline_h]-Eintrag fuer das Stueck [a, b] von `s` in Kachel `k`. `za`/`zb`: Hoehe am Anfang/Ende vorgeben
/// (dort eben, an der Kreuzung), sonst die des Splines mit seiner Steigung
#[allow(clippy::too_many_arguments)]
fn datensatz(s: &KartenSpline, a: f64, b: f64, id: i64, prev: i64, next: i64, k: (i32, i32), za: Option<f64>, zb: Option<f64>) -> Vec<String> {
    let kv = &s.kurve;
    let ts = omsi_map::tile_size();
    let pa = kv.point_at(a);
    let (ha, ga) = match za {
        Some(z) => (z, 0.0),
        None => (pa.z, kv.slope_at(a) * 100.0),
    };
    let (hb, gb) = match zb {
        Some(z) => (z, 0.0),
        None => (kv.point_at(b).z, kv.slope_at(b) * 100.0),
    };
    let l = kv.length.max(1e-9);
    let quer = |x: f64| kv.cant_start + (kv.cant_end - kv.cant_start) * x / l;
    let mut z = vec![
        "[spline_h]".to_string(), "0".into(), s.sli.clone(), id.to_string(), prev.to_string(), next.to_string(),
        zahl(pa.x - k.0 as f64 * ts), zahl(ha), zahl(pa.y - k.1 as f64 * ts),
        zahl(kv.heading_at(a).rem_euclid(360.0)), zahl(b - a), zahl(kv.radius), zahl(ga), zahl(gb), zahl(hb - ha),
        zahl(quer(a)), zahl(quer(b)),
        zahl(if a <= 0.0 { kv.skew_start } else { 0.0 }), zahl(if b >= kv.length { kv.skew_end } else { 0.0 }),
        zahl(kv.tex_offset + a),
    ];
    if s.gespiegelt {
        z.push("mirror".into());
    }
    z
}

pub(crate) fn zahl(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

/// naechster freier Name im Ordner: K_<Sitzung>_0001, ... - die Sitzung (Ordnername ohne "Editor_") macht die Namen
/// eindeutig, so dass beim Speichern in den Objektordner einer Karte nichts Aelteres ueberschrieben wird
pub fn freier_name(ordner: &Path) -> String {
    let sitzung = ordner.file_name().map(|n| n.to_string_lossy().trim_start_matches("Editor_").to_string()).unwrap_or_default();
    let sitzung: String = sitzung.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    (1..).map(|i| format!("K_{sitzung}_{i:04}")).find(|n| !ordner.join(format!("{n}.sco")).exists()).unwrap()
}

/// Ordner von omsigen (das Repository): OMSIGEN_DIR, sonst neben dem Editor-Quelltext
pub fn omsigen_ordner() -> PathBuf {
    std::env::var_os("OMSIGEN_DIR").map(PathBuf::from).unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join(".."))
}

/// Kreuzungsobjekt von omsigen bauen lassen (Python, OMSIGEN_PYTHON oder python)
pub fn erzeugen(root: &Path, ordner: &Path, rel_ordner: &str, name: &str, arme: &[Arm], ampel: bool) -> Result<Objekt> {
    use std::io::Write;
    let auftrag = serde_json::json!({
        "omsi": root, "ordner": ordner, "rel_ordner": rel_ordner, "name": name, "titel": format!("Editor-Kreuzung {name}"),
        "ampel": ampel,
        "arme": arme.iter().map(|a| serde_json::json!({
            "pos": [a.pos.x, a.pos.y], "h": a.richtung, "sli": a.sli, "away": a.weg, "rolle": a.rolle.text(),
        })).collect::<Vec<_>>(),
    });
    let python = std::env::var("OMSIGEN_PYTHON").unwrap_or_else(|_| "python".into());
    let mut kind = std::process::Command::new(&python)
        .args(["-m", "omsigen.editorkreuzung"])
        .current_dir(omsigen_ordner())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("{python} starten (omsigen erzeugt die Kreuzung)"))?;
    kind.stdin.take().unwrap().write_all(auftrag.to_string().as_bytes())?;
    let aus = kind.wait_with_output()?;
    if !aus.stderr.is_empty() {
        log::info!("omsigen (stderr): {}", String::from_utf8_lossy(&aus.stderr).trim());
    }
    let erg: serde_json::Value = serde_json::from_slice(&aus.stdout)
        .with_context(|| format!("omsigen: keine Antwort ({})", String::from_utf8_lossy(&aus.stderr).lines().last().unwrap_or("")))?;
    if let Some(f) = erg.get("fehler").and_then(|f| f.as_str()) {
        bail!("omsigen: {f}");
    }
    let zahl = |k: &str| erg.get(k).and_then(|x| x.as_u64()).unwrap_or(0) as usize;
    let u = erg["ursprung"].as_array().context("omsigen: kein Ursprung")?;
    Ok(Objekt {
        rel: erg["rel"].as_str().context("omsigen: keine Datei")?.to_string(),
        ursprung: DVec2::new(u[0].as_f64().unwrap_or(0.0), u[1].as_f64().unwrap_or(0.0)),
        rules: erg["rules"].as_array().map(|r| r.iter().filter_map(|x| Some((x[0].as_u64()? as usize, x[1].as_i64()? as i32))).collect()).unwrap_or_default(),
        spuren: zahl("spuren"),
        fehlgeschlagen: zahl("fehlgeschlagen"),
        signale: erg["signale"].as_array().map(|l| l.iter().map(|g| Signal {
            art: g["art"].as_str().unwrap_or("").to_string(),
            datei: g["datei"].as_str().unwrap_or("").to_string(),
            pos: g["x"].as_f64().zip(g["y"].as_f64()).map(|(x, y)| DVec2::new(x, y)),
            hoehe: g["hoehe"].as_f64().unwrap_or(0.0),
            rot: g["rot"].as_f64().unwrap_or(0.0),
            gruppe: g["gruppe"].as_i64(),
            eltern: g["eltern"].as_u64().map(|x| x as usize),
            anhang: g["anhang"].as_i64().unwrap_or(0),
        }).collect()).unwrap_or_default(),
        phasen: erg["phasen"].as_array().map(|l| l.iter().filter_map(|x| x.as_u64().map(|x| x as usize)).collect()).unwrap_or_default(),
        umlauf: erg["umlauf"].as_f64(),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::netz;

    #[test]
    fn richtung_und_masse() {
        assert!((arm_richtung(0.0, 90.0) - 90.0).abs() < 1e-9);
        assert!((arm_richtung(0.0, 10.0) - MIN_WINKEL).abs() < 1e-9);
        assert!((arm_richtung(0.0, 350.0) - (360.0 - MIN_WINKEL)).abs() < 1e-9);
        assert!((arm_richtung(90.0, 265.0) - (90.0 + 180.0 - MIN_WINKEL)).abs() < 1e-9);
        // rechtwinklig: Schnitt = halbe Breite des Arms + Eckenraum, Arm = halbe Strassenbreite + Eckenraum
        let (d, a) = masse(0.0, 90.0, 7.0, 5.0);
        assert!((d - 11.0).abs() < 1e-9 && (a - 13.0).abs() < 1e-9);
        // schraeg wird beides groesser
        let (d2, a2) = masse(0.0, 45.0, 7.0, 5.0);
        assert!(d2 > d && a2 > a);
    }

    #[test]
    fn omsigen_aufrufen() {
        let root = Path::new(crate::bearbeiten::tests::OMSI);
        if !root.exists() {
            return;
        }
        let d = std::env::temp_dir().join(format!("omsi-editor-kreuzung-{}", std::process::id()));
        let sli = "Splines\\Marcel\\str_2spur_8m_altonaer1.sli".to_string();
        let arm = |x: f64, y: f64, h: f64, weg: bool, rolle| Arm { pos: DVec3::new(x, y, 0.0), richtung: h, sli: sli.clone(), weg, rolle };
        let arme = [arm(0.0, -12.0, 180.0, false, Rolle::Haupt), arm(0.0, 12.0, 0.0, true, Rolle::Haupt), arm(12.0, 0.0, 90.0, true, Rolle::Neben)];
        let o = erzeugen(root, &d, "Sceneryobjects\\Aschaffenburg_KI\\Test", "K_E0001", &arme, false).unwrap();
        assert_eq!(o.rel, "Sceneryobjects\\Aschaffenburg_KI\\Test\\K_E0001.sco");
        assert_eq!((o.spuren, o.fehlgeschlagen), (6, 0));
        assert!(!o.rules.is_empty() && d.join("K_E0001.sco").exists() && d.join("model").join("K_E0001.x").exists());
        assert!(freier_name(&d).ends_with("_0001") && freier_name(&d).starts_with("K_omsieditorkreuzung"), "{}", freier_name(&d));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Ursprung o, so dass die Strecken o+(a..b) (Liste) keine vorhandene Strasse kreuzen (Grundorf, ganze Karte geladen)
    pub fn freie_flaeche(v: &Viewer, a: &mut Aendern, strecken: &[((f64, f64), (f64, f64))]) -> DVec2 {
        a.aktualisieren(v);
        // Kandidaten innerhalb der Kacheln (100 m vom Rand), von der Mitte nach aussen
        let ts = omsi_map::tile_size();
        let k = v.map_tiles();
        let (x0, x1) = (k.iter().map(|t| t.0).min().unwrap() as f64 * ts + 100.0, (k.iter().map(|t| t.0).max().unwrap() + 1) as f64 * ts - 100.0);
        let (y0, y1) = (k.iter().map(|t| t.1).min().unwrap() as f64 * ts + 100.0, (k.iter().map(|t| t.1).max().unwrap() + 1) as f64 * ts - 100.0);
        let mitte = DVec2::new((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let mut kandidaten: Vec<DVec2> = Vec::new();
        let mut x = x0;
        while x <= x1 {
            let mut y = y0;
            while y <= y1 {
                kandidaten.push(DVec2::new(x, y));
                y += 25.0;
            }
            x += 25.0;
        }
        kandidaten.sort_by(|a, b| (*a - mitte).length().total_cmp(&(*b - mitte).length()));
        for o in kandidaten {
            {
                let frei = strecken.iter().all(|&((x0, y0), (x1, y1))| {
                    let (p, q) = (o + DVec2::new(x0, y0), o + DVec2::new(x1, y1));
                    let n = ((q - p).length() / 2.0).ceil() as usize;
                    let punkte: Vec<(DVec3, f64)> = (0..=n).map(|k| {
                        let t = k as f64 / n as f64;
                        let r = p + (q - p) * t;
                        (r.extend(v.terrain_height(r.x, r.y).unwrap_or(f64::NAN)), (q - p).length() * t)
                    }).collect();
                    // Gelaende ueberall da und eben (hoechstens 3 m Unterschied), keine vorhandene Strasse naeher als 25 m
                    let (zmin, zmax) = punkte.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), x| (a.min(x.0.z), b.max(x.0.z)));
                    zmax - zmin < 3.0 && punkte.iter().all(|x| x.0.z.is_finite() && a.abzweig_bei(v, x.0.truncate()).is_none()
                        && [-25.0, 25.0].iter().all(|d| a.abzweig_bei(v, x.0.truncate() + crate::netz::rechts(crate::netz::richtung(p, q)) * *d).is_none()))
                        && a.kreuzungen_mit(v, &punkte).is_empty()
                });
                if frei {
                    return o;
                }
            }
        }
        panic!("keine freie Flaeche gefunden");
    }

    /// eine lange Strasse mitten in Grundorf: (Abzweig in ihrer Mitte)
    pub fn abzweig_suchen(v: &Viewer, a: &mut Aendern) -> Abzweig {
        a.aktualisieren(v);
        let mut kandidaten: Vec<KartenSpline> = a.kacheln.values().flatten()
            .filter(|s| s.kurve.length > 45.0 && v.spline_end_free(s.id, true).is_some())
            .cloned().collect();
        kandidaten.sort_by(|x, y| y.kurve.length.total_cmp(&x.kurve.length));
        for s in kandidaten {
            let m = s.kurve.point_at(s.kurve.length / 2.0).truncate();
            if let Some(ab) = a.abzweig_bei(v, m).filter(|ab| ab.spline_id == s.id) {
                return ab;
            }
        }
        panic!("keine passende Strasse");
    }

    /// mit dem Strassenwerkzeug abzweigen, als neue Karte speichern, laden: die neue Strasse haengt im Spurnetz an
    #[test]
    #[ignore]
    fn abzweig_bauen_speichern_laden() {
        let _sperre = crate::bearbeiten::tests::sperre();
        use crate::anschluss::Anschluesse;
        use crate::bearbeiten::{Bearbeiten, Werkzeug};
        use crate::strasse::{Modus, Strassenbau};
        let root = Path::new(crate::bearbeiten::tests::OMSI);
        let mut v = crate::bearbeiten::tests::grundorf();
        v.tiles_around(DVec3::new(150.0, 150.0, 0.0), 8).unwrap();
        let mut a = Aendern::neu(&v);
        let ab = abzweig_suchen(&v, &mut a);
        let sli = "Splines\\Marcel\\str_2spur_8m_altonaer1.sli";
        let mut s = Strassenbau::neu(Some(sli.into()), Modus::Kurve);
        let ans = Anschluesse::default();
        let m = s.klick(&mut v, ab.pos, 2.0, &ans, Some(&mut a)).unwrap();
        assert!(m.starts_with("Abzweig"), "{m}");
        // 50 m quer zur Strasse
        let z = ab.pos.truncate() + netz::rechts(ab.richtung) * 50.0;
        let ziel = z.extend(v.terrain_height(z.x, z.y).unwrap());
        s.maus(&mut v, ziel, 2.0, &ans, Some(&mut a));
        assert_eq!(s.plan.as_ref().unwrap().kreuzungen.len(), 1);
        let m = s.klick(&mut v, ziel, 2.0, &ans, Some(&mut a)).unwrap();
        assert!(m.contains("Kreuzung"), "{m}");
        s.beenden(&mut v);
        if let Some(bild) = std::env::var_os("OMSI_BILD") {
            let k = crate::kamera::Kamera { ziel: ab.pos + (netz::rechts(ab.richtung) * 15.0).extend(0.0), gier: (ab.richtung + 140.0) as f32, neigung: -42.0, abstand: 85.0, fov: 50.0 };
            let px = v.render_image(1280, 800, &k.camera()).unwrap();
            image::save_buffer(bild, &px, 1280, 800, image::ColorType::Rgba8).unwrap();
        }
        // die neue Strasse beginnt am Arm der Kreuzung (gekuerzt)
        let e = s.netz.kanten.iter().find(|e| s.netz.ist_kreuzung(e.a)).expect("Kante an der Kreuzung");
        let el0 = s.netz.elemente(e)[0];
        let start = el0.stueck.start.extend(el0.z);
        // speichern in einen Test-OMSI-Ordner
        let test_root = std::env::temp_dir().join(format!("omsi-editor-kreuzung-speichern-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&test_root);
        fn kopieren(a: &Path, b: &Path) {
            std::fs::create_dir_all(b).unwrap();
            for e in std::fs::read_dir(a).unwrap().flatten() {
                if e.file_type().unwrap().is_dir() { kopieren(&e.path(), &b.join(e.file_name())) } else { std::fs::copy(e.path(), b.join(e.file_name())).unwrap(); }
            }
        }
        kopieren(&root.join("maps/Grundorf"), &test_root.join("maps/Grundorf"));
        let paket = crate::speichern::vorbereiten(&v, &Bearbeiten::neu(Werkzeug::Strasse), &s.netz, &s.gesetzte_kreuzungen(), &a.kopien("Grundorf"), Some(a.kreuzungs_ordner()), "Grundorf").unwrap();
        let karte = crate::speichern::karte_anlegen(&test_root, "Grundorf", "Grundorf_kreuzung", &paket).unwrap();
        let objekte = test_root.join("Sceneryobjects/Aschaffenburg_KI/Grundorf_kreuzung");
        assert!(std::fs::read_dir(&objekte).unwrap().flatten().any(|e| e.path().extension().is_some_and(|x| x == "sco")));
        // neue Strasse: der Spline, der am Arm beginnt
        let mut neu_id = None;
        let mut verweis = false;
        for e in std::fs::read_dir(&karte).unwrap().flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            let Some(rest) = n.strip_prefix("tile_").and_then(|r| r.strip_suffix(".map")) else { continue };
            let mut it = rest.split('_').map(|x| x.parse::<i32>().unwrap());
            let (tx, ty) = (it.next().unwrap(), it.next().unwrap());
            let t = omsi_map::Tile::load(&e.path()).unwrap();
            verweis |= t.objects.iter().any(|o| o.file.contains("Aschaffenburg_KI\\Grundorf_kreuzung\\K_"));
            for sp in &t.splines {
                let k = omsi_geometry::SplineCurve::from_map(sp, DVec2::new(tx as f64 * 300.0, ty as f64 * 300.0));
                if (k.start - start).length() < 0.01 {
                    neu_id = Some(sp.id);
                }
            }
        }
        assert!(verweis, "keine Kachel verweist auf das Kreuzungsobjekt im Ordner der neuen Karte");
        let neu_id = neu_id.expect("neue Strasse nicht gefunden");
        drop(a);
        drop(v);
        // laden: die Objekte liegen im Test-Ordner (vor der Installation gelesen)
        let (mut v2, _) = Viewer::open(&openomsi_game::viewer::instance(), None, root, &karte.join("global.cfg")).unwrap();
        v2.session_overlay(&test_root);
        v2.tiles_around(DVec3::new(150.0, 150.0, 0.0), 8).unwrap();
        assert_eq!(v2.spline_end_free(ab.spline_id, true), Some(false), "Strasse vor der Kreuzung endet frei");
        assert_eq!(v2.spline_end_free(neu_id, false), Some(false), "neue Strasse beginnt frei statt an der Kreuzung");
        std::fs::remove_dir_all(&test_root).ok();
    }

    /// Start auf einer Strasse, dann die Maus ueber ein Raster von Punkten (auch ueber dieselbe und andere
    /// Strassen): jede Vorschau muss schnell und endlich sein
    #[test]
    #[ignore]
    fn abzweig_vorschau_ueberall() {
        let _sperre = crate::bearbeiten::tests::sperre();
        use crate::anschluss::Anschluesse;
        use crate::strasse::{Modus, Strassenbau};
        let mut v = crate::bearbeiten::tests::grundorf();
        v.tiles_around(DVec3::new(150.0, 150.0, 0.0), 8).unwrap();
        let mut a = Aendern::neu(&v);
        let ab = abzweig_suchen(&v, &mut a);
        let mut ans = Anschluesse::default();
        ans.aktualisieren(&v);
        for modus in [Modus::Kurve, Modus::Gerade] {
            let mut s = Strassenbau::neu(Some("Splines\\Marcel\\str_2spur_8m_altonaer1.sli".into()), modus);
            s.klick(&mut v, ab.pos, 2.0, &ans, Some(&mut a)).unwrap();
            let mut langsam = Vec::new();
            for i in -40..=40 {
                for j in -40..=40 {
                    let p = ab.pos.truncate() + DVec2::new(i as f64 * 4.0, j as f64 * 4.0);
                    let g = p.extend(v.terrain_height(p.x, p.y).unwrap_or(ab.pos.z));
                    let t = std::time::Instant::now();
                    s.maus(&mut v, g, 2.0, &ans, Some(&mut a));
                    let ms = t.elapsed().as_millis();
                    if let Some(pl) = s.plan.as_ref() {
                        assert!(pl.laenge.is_finite() && pl.laenge < 5000.0, "Vorschau zu {p:?}: Laenge {}", pl.laenge);
                    }
                    // das Zeichnen der Vorschau-Splines in openOMSI kostet ~20 ms je Stueck (Geometrie); die Planung selbst
                    // (Ziel, Kreuzungen, Querungen) unter 1 ms
                    if ms > 150 {
                        langsam.push((p, ms));
                    }
                }
            }
            s.beenden(&mut v);
            assert!(langsam.is_empty(), "{modus:?}: langsame Vorschauen {:?}", &langsam[..langsam.len().min(10)]);
        }
        // an einem groeberen Raster wirklich bauen (Kreuzung, auch ein Ziel auf einer Strasse) und zuruecknehmen
        let mut gebaut = 0;
        let mut meldungen = std::collections::BTreeMap::<String, usize>::new();
        for i in -3..=3 {
            for j in -3..=3 {
                let mut s = Strassenbau::neu(Some("Splines\\Marcel\\str_2spur_8m_altonaer1.sli".into()), Modus::Kurve);
                a.aktualisieren(&v);
                ans.vergessen();
                ans.aktualisieren(&v);
                s.klick(&mut v, ab.pos, 2.0, &ans, Some(&mut a)).unwrap();
                let p = ab.pos.truncate() + DVec2::new(i as f64 * 25.0, j as f64 * 25.0);
                let g = p.extend(v.terrain_height(p.x, p.y).unwrap_or(ab.pos.z));
                s.maus(&mut v, g, 2.0, &ans, Some(&mut a));
                if s.plan.is_none() {
                    continue;
                }
                let t = std::time::Instant::now();
                let m = s.klick(&mut v, g, 2.0, &ans, Some(&mut a)).unwrap_or_default();
                assert!(t.elapsed().as_secs() < 10, "Bauen zu {p:?} dauerte {:?}", t.elapsed());
                *meldungen.entry(m.split(':').next().unwrap_or("").to_string()).or_default() += 1;
                s.beenden(&mut v);
                if s.kann_rueckgaengig() {
                    gebaut += 1;
                    s.rueckgaengig(&mut v, Some(&mut a));
                }
            }
        }
        println!("gebaut {gebaut}, Meldungen {meldungen:?}");
        assert!(gebaut > 10);
    }

    /// neue Strasse frei beginnen (auch mit Zwischenpunkt) und mitten auf einer vorhandenen enden
    #[test]
    #[ignore]
    fn strasse_endet_auf_vorhandener() {
        let _sperre = crate::bearbeiten::tests::sperre();
        use crate::anschluss::Anschluesse;
        use crate::strasse::{Modus, Strassenbau};
        let mut v = crate::bearbeiten::tests::grundorf();
        v.tiles_around(DVec3::new(150.0, 150.0, 0.0), 8).unwrap();
        let mut a = Aendern::neu(&v);
        let ab = abzweig_suchen(&v, &mut a);
        let mut ans = Anschluesse::default();
        ans.aktualisieren(&v);
        let boden = |v: &Viewer, p: DVec2| p.extend(v.terrain_height(p.x, p.y).unwrap_or(0.0));
        let r = netz::rechts(ab.richtung);
        let d = netz::dir(ab.richtung);
        for (fall, punkte) in [
            ("direkt", vec![ab.pos.truncate() + r * 60.0]),
            ("schraeg", vec![ab.pos.truncate() + r * 50.0 + d * 30.0]),
            ("mit Zwischenpunkt", vec![ab.pos.truncate() + r * 90.0 + d * 20.0, ab.pos.truncate() + r * 45.0]),
            ("Zwischenpunkt parallel", vec![ab.pos.truncate() + r * 60.0 + d * 60.0, ab.pos.truncate() + r * 60.0 + d * 10.0]),
        ] {
            for modus in [Modus::Kurve, Modus::Gerade] {
                let mut s = Strassenbau::neu(Some("Splines\\Marcel\\str_2spur_8m_altonaer1.sli".into()), modus);
                for p in &punkte {
                    let g = boden(&v, *p);
                    s.maus(&mut v, g, 2.0, &ans, Some(&mut a));
                    s.klick(&mut v, g, 2.0, &ans, Some(&mut a)).unwrap();
                }
                s.maus(&mut v, ab.pos, 2.0, &ans, Some(&mut a));
                let plan = s.plan.as_ref().map(|p| format!("Plan {:.1} m, {} Kreuzung(en)", p.laenge, p.kreuzungen.len()));
                let m = s.klick(&mut v, ab.pos, 2.0, &ans, Some(&mut a));
                println!("{fall} {modus:?}: {plan:?} -> {m:?}");
                let ok = m.as_deref().is_some_and(|m| m.contains("Kreuzung"));
                s.beenden(&mut v);
                while s.kann_rueckgaengig() {
                    s.rueckgaengig(&mut v, Some(&mut a));
                }
                a.aktualisieren(&v);
                assert!(ok, "{fall} {modus:?}: keine Kreuzung am Ziel");
            }
        }
    }

    /// wie beim Nutzer (Protokoll vom 6.10.): Ziel kurz vor dem Ende von Spline 524 - die Kreuzung rueckt in die
    /// Strasse hinein bzw. rastet am freien Ende ein, statt "zu nah am Ende"
    #[test]
    #[ignore]
    fn ziel_nahe_strassenende() {
        let _sperre = crate::bearbeiten::tests::sperre();
        use crate::anschluss::Anschluesse;
        use crate::strasse::{Modus, Strassenbau};
        let mut v = crate::bearbeiten::tests::grundorf();
        v.tiles_around(DVec3::new(150.0, 150.0, 0.0), 8).unwrap();
        let mut a = Aendern::neu(&v);
        a.aktualisieren(&v);
        let mut ans = Anschluesse::default();
        ans.aktualisieren(&v);
        let sp = a.spline(524).expect("Spline 524").clone();
        for (lage, seite) in [(sp.kurve.length - 17.5, 1.0), (sp.kurve.length - 3.0, -1.0), (12.0, 1.0)] {
            let z = sp.kurve.point_at(lage);
            let h = sp.kurve.heading_at(lage);
            let start = z.truncate() + netz::rechts(h) * 60.0 * seite;
            let mut s = Strassenbau::neu(Some("Splines\\Marcel\\str_2spur_8m_altonaer1.sli".into()), Modus::Kurve);
            let g = start.extend(v.terrain_height(start.x, start.y).unwrap());
            s.klick(&mut v, g, 2.0, &ans, Some(&mut a)).unwrap();
            s.maus(&mut v, z, 2.0, &ans, Some(&mut a));
            let warnung = s.plan.as_ref().and_then(|p| p.warnung.clone());
            let m = s.klick(&mut v, z, 2.0, &ans, Some(&mut a)).unwrap_or_default();
            println!("Lage {lage:.1} m: Warnung {warnung:?} -> {m}");
            assert!(m.contains("angeschlossen"), "Lage {lage:.1}: {m}");
            s.beenden(&mut v);
            while s.kann_rueckgaengig() {
                s.rueckgaengig(&mut v, Some(&mut a));
            }
            a.aktualisieren(&v);
            ans.vergessen();
            ans.aktualisieren(&v);
        }
    }

    /// eigene Strasse bauen, davon abzweigen, eine zweite darauf enden lassen; speichern, laden: alle Enden haengen
    /// an den Kreuzungen (Spurnetz). Rueckgaengig nimmt alles zurueck.
    #[test]
    #[ignore]
    fn kreuzungen_im_eigenen_netz() {
        let _sperre = crate::bearbeiten::tests::sperre();
        use crate::anschluss::Anschluesse;
        use crate::bearbeiten::{Bearbeiten, Werkzeug};
        use crate::strasse::{Modus, Strassenbau};
        let root = Path::new(crate::bearbeiten::tests::OMSI);
        let mut v = crate::bearbeiten::tests::grundorf();
        v.tiles_around(DVec3::new(150.0, 150.0, 0.0), 8).unwrap();
        let mut a = Aendern::neu(&v);
        a.aktualisieren(&v);
        let ans = Anschluesse::default();
        let o = freie_flaeche(&v, &mut a, &[((0.0, 0.0), (0.0, 130.0)), ((0.0, 40.0), (70.0, 40.0)), ((-70.0, 100.0), (0.0, 100.0))]);
        println!("freie Flaeche bei {o:?}");
        let boden = move |v: &Viewer, x: f64, y: f64| DVec3::new(o.x + x - 120.0, o.y + y - 100.0, v.terrain_height(o.x + x - 120.0, o.y + y - 100.0).unwrap());
        let mut s = Strassenbau::neu(Some("Splines\\Marcel\\str_2spur_10m_Grunewaldstr.sli".into()), Modus::Gerade);
        let bauen = |s: &mut Strassenbau, v: &mut Viewer, a: &mut Aendern, punkte: &[(f64, f64)]| -> String {
            let mut m = String::new();
            for &(x, y) in punkte {
                let p = boden(v, x, y);
                s.maus(v, p, 2.0, &ans, Some(a));
                m = s.klick(v, p, 2.0, &ans, Some(a)).unwrap_or_default();
            }
            s.beenden(v);
            m
        };
        // Strasse A nach Norden
        bauen(&mut s, &mut v, &mut a, &[(120.0, 100.0), (120.0, 230.0)]);
        assert_eq!(s.netz.kanten.len(), 1);
        // Abzweig mitten aus A nach Osten
        s.sli = Some("Splines\\Marcel\\str_2spur_8m_altonaer1.sli".into());
        let m1 = bauen(&mut s, &mut v, &mut a, &[(120.5, 140.0), (190.0, 140.0)]);
        println!("Abzweig: {m1}");
        assert_eq!(s.netz.kanten.len(), 3, "A geteilt + Abzweig");
        assert_eq!(s.gesetzte_kreuzungen().len(), 1, "{:?}", s.kreuzung_fehler);
        // zweite Strasse von Westen endet mitten auf A
        let m2 = bauen(&mut s, &mut v, &mut a, &[(50.0, 200.0), (119.5, 200.0)]);
        println!("Ende auf A: {m2}");
        assert_eq!(s.netz.kanten.len(), 5);
        assert_eq!(s.gesetzte_kreuzungen().len(), 2, "{:?}", s.kreuzung_fehler);
        assert!(s.netz.kanten.iter().all(|e| !s.netz.elemente(e).is_empty()));
        if let Some(bild) = std::env::var_os("OMSI_BILD") {
            let k = crate::kamera::Kamera { ziel: boden(&v, 120.0, 170.0), gier: 200.0, neigung: -55.0, abstand: 150.0, fov: 50.0 };
            let px = v.render_image(1280, 800, &k.camera()).unwrap();
            image::save_buffer(bild, &px, 1280, 800, image::ColorType::Rgba8).unwrap();
        }
        // speichern und laden
        let test_root = std::env::temp_dir().join(format!("omsi-editor-netzkreuzung-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&test_root);
        fn kopieren(a: &Path, b: &Path) {
            std::fs::create_dir_all(b).unwrap();
            for e in std::fs::read_dir(a).unwrap().flatten() {
                if e.file_type().unwrap().is_dir() { kopieren(&e.path(), &b.join(e.file_name())) } else { std::fs::copy(e.path(), b.join(e.file_name())).unwrap(); }
            }
        }
        kopieren(&root.join("maps/Grundorf"), &test_root.join("maps/Grundorf"));
        let start_id = v.next_object_id();
        let paket = crate::speichern::vorbereiten(&v, &Bearbeiten::neu(Werkzeug::Strasse), &s.netz, &s.gesetzte_kreuzungen(), &a.kopien("Grundorf"), Some(a.kreuzungs_ordner()), "Grundorf").unwrap();
        let karte = crate::speichern::karte_anlegen(&test_root, "Grundorf", "Grundorf_netzkreuzung", &paket).unwrap();
        // Rueckgaengig (2 x Kreuzung, 1 x A): leeres Netz, keine Objekte
        for _ in 0..3 {
            assert!(s.rueckgaengig(&mut v, Some(&mut a)));
        }
        assert!(s.netz.kanten.is_empty() && s.gesetzte_kreuzungen().is_empty());
        drop(a);
        drop(v);
        let (mut v2, _) = Viewer::open(&openomsi_game::viewer::instance(), None, root, &karte.join("global.cfg")).unwrap();
        v2.session_overlay(&test_root);
        v2.tiles_around(DVec3::new(150.0, 150.0, 0.0), 8).unwrap();
        // alle neuen Splines: jedes Ende, das an keinem anderen neuen Spline anschliesst, liegt an einer Kreuzung
        let mut neue: Vec<(i64, omsi_geometry::SplineCurve)> = Vec::new();
        for e in std::fs::read_dir(&karte).unwrap().flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            let Some(rest) = n.strip_prefix("tile_").and_then(|r| r.strip_suffix(".map")) else { continue };
            let mut it = rest.split('_').map(|x| x.parse::<i32>().unwrap());
            let (tx, ty) = (it.next().unwrap(), it.next().unwrap());
            for sp in omsi_map::Tile::load(&e.path()).unwrap().splines.iter().filter(|sp| sp.id >= start_id) {
                neue.push((sp.id, omsi_geometry::SplineCurve::from_map(sp, DVec2::new(tx as f64 * 300.0, ty as f64 * 300.0))));
            }
        }
        let mut an_kreuzung = 0;
        for (id, k) in &neue {
            for am_ende in [false, true] {
                let p = if am_ende { k.end_point() } else { k.start };
                let innen = neue.iter().any(|(i, m)| i != id && ((if am_ende { m.start } else { m.end_point() }) - p).length() < 0.01);
                let rand = [(120.0, 100.0), (120.0, 230.0), (190.0, 140.0), (50.0, 200.0)].iter().any(|q| (p.truncate() - boden(&v2, q.0, q.1).truncate()).length() < 0.5);
                if !innen && !rand {
                    an_kreuzung += 1;
                    assert_eq!(v2.spline_end_free(*id, am_ende), Some(false), "Spline {id} {} haengt nicht an der Kreuzung", if am_ende { "Ende" } else { "Anfang" });
                }
            }
        }
        assert_eq!(an_kreuzung, 6, "2 Kreuzungen mit je 3 Armen");
        std::fs::remove_dir_all(&test_root).ok();
    }

    /// als neue Karte speichern und laden: jedes Ende eines neuen Splines, das an keinem anderen neuen Spline
    /// anschliesst und kein Zugende (`rand`) ist, haengt laut Spurnetz an einer Kreuzung; ebenso die Enden der
    /// aufgeschnittenen vorhandenen Strassen (`karten`). -> Anzahl der Enden an Kreuzungen
    fn speichern_laden_pruefen(v: Viewer, s: &crate::strasse::Strassenbau, a: Aendern, name: &str, rand: &[DVec2], karten: &[DVec2]) -> usize {
        speichern_laden_mit(v, s, a, name, rand, karten, |_| {})
    }

    fn speichern_laden_mit(v: Viewer, s: &crate::strasse::Strassenbau, a: Aendern, name: &str, rand: &[DVec2], karten: &[DVec2],
                           pruefen: impl FnOnce(&Viewer)) -> usize {
        use crate::bearbeiten::{Bearbeiten, Werkzeug};
        let root = Path::new(crate::bearbeiten::tests::OMSI);
        let test_root = std::env::temp_dir().join(format!("omsi-editor-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&test_root);
        fn kopieren(a: &Path, b: &Path) {
            std::fs::create_dir_all(b).unwrap();
            for e in std::fs::read_dir(a).unwrap().flatten() {
                if e.file_type().unwrap().is_dir() { kopieren(&e.path(), &b.join(e.file_name())) } else { std::fs::copy(e.path(), b.join(e.file_name())).unwrap(); }
            }
        }
        kopieren(&root.join("maps/Grundorf"), &test_root.join("maps/Grundorf"));
        let start_id = v.next_object_id();
        let paket = crate::speichern::vorbereiten(&v, &Bearbeiten::neu(Werkzeug::Strasse), &s.netz, &s.gesetzte_kreuzungen(), &a.kopien("Grundorf"), Some(a.kreuzungs_ordner()), "Grundorf").unwrap();
        let karte = crate::speichern::karte_anlegen(&test_root, "Grundorf", name, &paket).unwrap();
        drop(a);
        drop(v);
        let (mut v2, _) = Viewer::open(&openomsi_game::viewer::instance(), None, root, &karte.join("global.cfg")).unwrap();
        v2.session_overlay(&test_root);
        v2.tiles_around(DVec3::new(150.0, 150.0, 0.0), 8).unwrap();
        let mut neue: Vec<(i64, omsi_geometry::SplineCurve)> = Vec::new();
        for e in std::fs::read_dir(&karte).unwrap().flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            let Some(rest) = n.strip_prefix("tile_").and_then(|r| r.strip_suffix(".map")) else { continue };
            let mut it = rest.split('_').map(|x| x.parse::<i32>().unwrap());
            let (tx, ty) = (it.next().unwrap(), it.next().unwrap());
            for sp in omsi_map::Tile::load(&e.path()).unwrap().splines.iter().filter(|sp| sp.id >= start_id) {
                neue.push((sp.id, omsi_geometry::SplineCurve::from_map(sp, DVec2::new(tx as f64 * 300.0, ty as f64 * 300.0))));
            }
        }
        let mut an_kreuzung = 0;
        for (id, k) in &neue {
            for am_ende in [false, true] {
                let p = if am_ende { k.end_point() } else { k.start };
                let innen = neue.iter().any(|(i, m)| i != id && ((if am_ende { m.start } else { m.end_point() }) - p).length() < 0.01);
                let zugende = rand.iter().any(|q| (p.truncate() - *q).length() < 0.5);
                if !innen && !zugende {
                    an_kreuzung += 1;
                    assert_eq!(v2.spline_end_free(*id, am_ende), Some(false), "Spline {id} {} haengt nicht an der Kreuzung", if am_ende { "Ende" } else { "Anfang" });
                }
            }
        }
        let mut ans = crate::anschluss::Anschluesse::default();
        ans.aktualisieren(&v2);
        for q in karten {
            assert!(!ans.liste.iter().any(|x| x.frei && (x.pos.truncate() - *q).length() < 1.0), "aufgeschnittene Strasse bei {q:?} endet frei");
        }
        pruefen(&v2);
        std::fs::remove_dir_all(&test_root).ok();
        an_kreuzung
    }

    #[test]
    fn vorfahrt_vermuten() {
        use crate::strasse::rollen_vermuten;
        let arm = |h: f64, sli: &str| Arm { pos: DVec3::ZERO, richtung: h, sli: sli.into(), weg: true, rolle: Rolle::Gleich };
        // T: durchgehende Strasse hat Vorfahrt
        let t = [arm(0.0, "a"), arm(180.0, "a"), arm(90.0, "b")];
        assert_eq!(rollen_vermuten(&t, &[(5.0, false, false); 3]), vec![Rolle::Haupt, Rolle::Haupt, Rolle::Neben]);
        // Kreuzung zweier gleicher Strassen: rechts vor links
        let x = [arm(0.0, "a"), arm(180.0, "a"), arm(90.0, "a"), arm(270.0, "a")];
        assert_eq!(rollen_vermuten(&x, &[(5.0, false, false); 4]), vec![Rolle::Gleich; 4]);
        // ueber eine vorhandene Strasse: die vorhandene hat Vorfahrt
        let i = [(5.0, true, false), (5.0, true, false), (5.0, false, false), (5.0, false, false)];
        assert_eq!(rollen_vermuten(&x, &i), vec![Rolle::Haupt, Rolle::Haupt, Rolle::Neben, Rolle::Neben]);
        // breitere Strasse hat Vorfahrt
        let i = [(7.0, false, false), (7.0, false, false), (5.0, false, false), (5.0, false, false)];
        assert_eq!(rollen_vermuten(&x, &i), vec![Rolle::Haupt, Rolle::Haupt, Rolle::Neben, Rolle::Neben]);
        // Kreisverkehr: Ring hat Vorfahrt
        let k = [arm(270.0, "r"), arm(90.0, "r"), arm(0.0, "z")];
        assert_eq!(rollen_vermuten(&k, &[(4.0, false, true), (4.0, false, true), (5.0, false, false)]), vec![Rolle::Haupt, Rolle::Haupt, Rolle::Neben]);
        // Stern ohne gegenueberliegende Arme: rechts vor links
        let y = [arm(0.0, "a"), arm(120.0, "a"), arm(240.0, "a")];
        assert_eq!(rollen_vermuten(&y, &[(5.0, false, false); 3]), vec![Rolle::Gleich; 3]);
    }

    /// Kreuzen: eine neue Strasse ueber eine eigene und ueber eine vorhandene Strasse -> zwei Kreuzungen mit 4 Armen
    #[test]
    #[ignore]
    fn kreuzen_eigene_und_vorhandene() {
        let _sperre = crate::bearbeiten::tests::sperre();
        use crate::anschluss::Anschluesse;
        use crate::strasse::{Modus, Strassenbau};
        let mut v = crate::bearbeiten::tests::grundorf();
        v.tiles_around(DVec3::new(150.0, 150.0, 0.0), 8).unwrap();
        let mut a = Aendern::neu(&v);
        let ans = Anschluesse::default();
        // vorhandene lange Strasse: quer dazu eine neue, 70 m zu jeder Seite
        let ab = abzweig_suchen(&v, &mut a);
        let r = crate::netz::rechts(ab.richtung);
        let d = crate::netz::dir(ab.richtung);
        let c = ab.pos.truncate();
        let boden = |v: &Viewer, p: DVec2| p.extend(v.terrain_height(p.x, p.y).unwrap());
        let mut s = Strassenbau::neu(Some("Splines\\Marcel\\str_2spur_8m_altonaer1.sli".into()), Modus::Gerade);
        let bauen = |s: &mut Strassenbau, v: &mut Viewer, a: &mut Aendern, punkte: &[DVec2]| -> String {
            let mut m = String::new();
            for q in punkte {
                let g = boden(v, *q);
                s.maus(v, g, 2.0, &ans, Some(a));
                m = s.klick(v, g, 2.0, &ans, Some(a)).unwrap_or_default();
            }
            s.beenden(v);
            m
        };
        // eigene Strasse E parallel zur vorhandenen, 45 m daneben
        let e0 = c + r * 45.0 - d * 50.0;
        let e1 = c + r * 45.0 + d * 50.0;
        bauen(&mut s, &mut v, &mut a, &[e0, e1]);
        assert_eq!(s.netz.kanten.len(), 1);
        // neue Strasse quer ueber beide: von links der vorhandenen bis rechts von E
        let q0 = c - r * 40.0;
        let q1 = c + r * 90.0;
        let g0 = boden(&v, q0);
        s.klick(&mut v, g0, 2.0, &ans, Some(&mut a));
        let g1 = boden(&v, q1);
        s.maus(&mut v, g1, 2.0, &ans, Some(&mut a));
        let plan = s.plan.clone().unwrap();
        println!("Plan: {} Kreuzungen, blockiert {:?}", plan.kreuzungen.len(), plan.blockiert);
        assert_eq!(plan.kreuzungen.len(), 2, "zwei Querungen erwartet");
        let m = s.klick(&mut v, g1, 2.0, &ans, Some(&mut a)).unwrap();
        s.beenden(&mut v);
        println!("{m}");
        assert!(m.contains("2 Kreuzung"), "{m}");
        assert_eq!(s.gesetzte_kreuzungen().len(), 2, "{:?}", s.kreuzung_fehler);
        // beide Knoten mit 4 Armen; an der vorhandenen Strasse hat diese Vorfahrt (siehe vorfahrt_vermuten)
        let vier: Vec<u32> = s.netz.knoten.iter().map(|k| k.id).filter(|k| s.netz.arme(*k).len() == 4).collect();
        assert_eq!(vier.len(), 2);
        let karte = s.netz.knoten.iter().find(|k| k.kartenarme.len() == 2).expect("Knoten an der vorhandenen Strasse");
        let karten: Vec<DVec2> = karte.kartenarme.iter().map(|x| x.pos.truncate()).collect();
        if let Some(bild) = std::env::var_os("OMSI_BILD") {
            let k = crate::kamera::Kamera { ziel: (c + r * 25.0).extend(ab.pos.z), gier: (ab.richtung + 160.0) as f32, neigung: -52.0, abstand: 130.0, fov: 50.0 };
            let px = v.render_image(1280, 800, &k.camera()).unwrap();
            image::save_buffer(bild, &px, 1280, 800, image::ColorType::Rgba8).unwrap();
        }
        let n = speichern_laden_pruefen(v, &s, a, "Grundorf_kreuzen", &[e0, e1, q0, q1], &karten);
        // 4-Arm-Kreuzung an E: 4 neue Enden; an der vorhandenen: 2 neue Enden
        assert_eq!(n, 6);
    }

    /// Kreisverkehr mit drei Zufahrten: T-Kreuzungen am Ring, Ring hat Vorfahrt
    #[test]
    #[ignore]
    fn kreisverkehr_mit_zufahrten() {
        let _sperre = crate::bearbeiten::tests::sperre();
        use crate::anschluss::Anschluesse;
        use crate::strasse::{Modus, Strassenbau};
        let mut v = crate::bearbeiten::tests::grundorf();
        v.tiles_around(DVec3::new(150.0, 150.0, 0.0), 8).unwrap();
        let mut a = Aendern::neu(&v);
        let ans = Anschluesse::default();
        let o = freie_flaeche(&v, &mut a, &[((-90.0, 0.0), (90.0, 0.0)), ((0.0, -90.0), (0.0, 90.0))]);
        println!("freie Flaeche bei {o:?}");
        let boden = |v: &Viewer, p: DVec2| p.extend(v.terrain_height(p.x, p.y).unwrap());
        let mut s = Strassenbau::neu(Some("Splines\\Marcel\\str_2spur_8m_altonaer1.sli".into()), Modus::Kreisel);
        s.kreisel_sli = crate::strasse::kreisel_vorschlag(&crate::strasse::querschnitte(Path::new(crate::bearbeiten::tests::OMSI)));
        println!("Ring: {:?}", s.kreisel_sli);
        let mitte = boden(&v, o);
        let m = s.klick(&mut v, mitte, 2.0, &ans, Some(&mut a)).unwrap();
        assert!(m.starts_with("Kreisverkehr"), "{m}");
        let rand = boden(&v, o + DVec2::new(22.0, 0.0));
        s.maus(&mut v, rand, 2.0, &ans, Some(&mut a));
        let m = s.klick(&mut v, rand, 2.0, &ans, Some(&mut a)).unwrap();
        println!("{m}");
        assert_eq!(s.netz.kanten.iter().filter(|e| e.ring).count(), 4);
        // Zufahrten von Westen, Sueden, Nordosten auf den Ring
        s.modus = Modus::Gerade;
        let mut zufahrten = Vec::new();
        for (w, n) in [(270.0, "West"), (180.0, "Sued"), (45.0, "Nordost")] {
            let aussen = o + crate::netz::dir(w) * 85.0;
            let am_ring = o + crate::netz::dir(w) * 22.0;
            let ga = boden(&v, aussen);
            s.klick(&mut v, ga, 2.0, &ans, Some(&mut a));
            let gb = boden(&v, am_ring);
            s.maus(&mut v, gb, 2.0, &ans, Some(&mut a));
            let m = s.klick(&mut v, gb, 2.0, &ans, Some(&mut a)).unwrap_or_default();
            s.beenden(&mut v);
            println!("Zufahrt {n}: {m}");
            assert!(m.contains("Kreuzung"), "Zufahrt {n}: {m}");
            zufahrten.push(aussen);
        }
        assert_eq!(s.gesetzte_kreuzungen().len(), 3, "{:?}", s.kreuzung_fehler);
        if let Some(bild) = std::env::var_os("OMSI_BILD") {
            let k = crate::kamera::Kamera { ziel: boden(&v, o + DVec2::new(-12.0, -12.0)), gier: 225.0, neigung: -50.0, abstand: 55.0, fov: 50.0 };
            let px = v.render_image(1280, 800, &k.camera()).unwrap();
            image::save_buffer(bild, &px, 1280, 800, image::ColorType::Rgba8).unwrap();
        }
        let n = speichern_laden_pruefen(v, &s, a, "Grundorf_kreisel", &zufahrten, &[]);
        assert_eq!(n, 9, "3 Kreuzungen mit je 3 Armen");
    }

    /// Kreisverkehr auf die Ampelkreuzung von Grundorf (414/215): die Kreuzung mit Ampeln und die Strassenstuecke im
    /// Ring fallen weg, die drei Strassen muenden aussen in den Ring (3 T-Kreuzungen); gespeichert und geladen haengen
    /// sie an den Kreuzungen, Rueckgaengig stellt alles wieder her
    #[test]
    #[ignore]
    fn kreisverkehr_ueber_vorhandener_kreuzung() {
        let _sperre = crate::bearbeiten::tests::sperre();
        use crate::anschluss::Anschluesse;
        use crate::strasse::{Modus, Strassenbau};
        let mut v = crate::bearbeiten::tests::grundorf();
        v.tiles_around(DVec3::new(150.0, 150.0, 0.0), 8).unwrap();
        let mut a = Aendern::neu(&v);
        a.aktualisieren(&v);
        let k = a.kreuzungsobjekt_bei(&v, DVec2::new(414.0, 215.0)).expect("Kreuzung");
        let ans = Anschluesse::default();
        let mut s = Strassenbau::neu(Some("Splines\\Marcel\\str_2spur_8m_altonaer1.sli".into()), Modus::Kreisel);
        s.kreisel_sli = crate::strasse::kreisel_vorschlag(&crate::strasse::querschnitte(Path::new(crate::bearbeiten::tests::OMSI)));
        let mitte = k.pos.truncate().extend(v.terrain_height(k.pos.x, k.pos.y).unwrap());
        let kam = crate::kamera::Kamera { ziel: mitte, gier: 200.0, neigung: -60.0, abstand: 95.0, fov: 50.0 };
        if let Some(b) = std::env::var_os("OMSI_BILD_VORHER") {
            let px = v.render_image(1280, 800, &kam.camera()).unwrap();
            image::save_buffer(b, &px, 1280, 800, image::ColorType::Rgba8).unwrap();
        }
        s.klick(&mut v, mitte, 2.0, &ans, Some(&mut a));
        let rand = mitte + DVec3::new(24.0, 0.0, 0.0);
        s.maus(&mut v, rand, 2.0, &ans, Some(&mut a));
        println!("Plan: {:?}", s.plan.as_ref().map(|p| (p.kreuzungen.len(), p.warnung.clone())));
        let m = s.klick(&mut v, rand, 2.0, &ans, Some(&mut a)).unwrap();
        println!("{m}");
        assert!(m.contains("Kreisverkehr") || m.contains("gebaut"), "{m}");
        a.aktualisieren(&v);
        // die Kreuzung ist weg, an ihrer Stelle keine vorhandene Strasse mehr
        assert!(a.kreuzungsobjekt_bei(&v, k.pos.truncate()).is_none(), "Kreuzung im Ring noch da");
        assert!(a.abzweig_bei(&v, k.pos.truncate()).is_none(), "Strasse im Ring noch da");
        let einmuendungen: Vec<_> = s.netz.knoten.iter().filter(|x| x.kartenarme.len() == 1).collect();
        assert_eq!(einmuendungen.len(), 3, "drei Einmuendungen am Ring");
        assert_eq!(s.gesetzte_kreuzungen().len(), 3, "{:?}", s.kreuzung_fehler);
        if let Some(b) = std::env::var_os("OMSI_BILD") {
            let px = v.render_image(1280, 800, &kam.camera()).unwrap();
            image::save_buffer(b, &px, 1280, 800, image::ColorType::Rgba8).unwrap();
        }
        let karten: Vec<DVec2> = einmuendungen.iter().map(|x| x.kartenarme[0].pos.truncate()).collect();
        let n = speichern_laden_pruefen(v, &s, a, "Grundorf_kreisel_karte", &[], &karten);
        assert_eq!(n, 6, "4 Ringstuecke an 3 Kreuzungen: 6 Enden");
    }

    /// Fall des Nutzers (Screenshot 6.10.): neue Strasse an die Ampelkreuzung von Grundorf (x 412, y 223, 3 Arme)
    /// anschliessen - sie wird durch eine eigene Kreuzung mit 4 Armen ersetzt
    #[test]
    #[ignore]
    fn an_vorhandene_kreuzung_anschliessen() {
        let _sperre = crate::bearbeiten::tests::sperre();
        use crate::anschluss::Anschluesse;
        use crate::strasse::{Modus, Strassenbau};
        let mut v = crate::bearbeiten::tests::grundorf();
        v.tiles_around(DVec3::new(150.0, 150.0, 0.0), 8).unwrap();
        let mut a = Aendern::neu(&v);
        a.aktualisieren(&v);
        let k = a.kreuzungsobjekt_bei(&v, DVec2::new(414.0, 215.0)).expect("Kreuzungsobjekt bei 414/215");
        println!("Kreuzung {} mit {} Armen bei {:?}: {:?}", k.objekt, k.arme.len(), k.pos, k.arme.iter().map(|x| x.richtung.round()).collect::<Vec<_>>());
        assert_eq!(k.arme.len(), 3);
        // freie Richtung: am weitesten von allen Armen
        let frei = (0..360).map(|g| g as f64).max_by(|x, y| {
            let m = |h: f64| k.arme.iter().map(|a| norm180(a.richtung - h).abs()).fold(f64::INFINITY, f64::min);
            m(*x).total_cmp(&m(*y))
        }).unwrap();
        let aussen = k.pos.truncate() + crate::netz::dir(frei) * 70.0;
        let ans = Anschluesse::default();
        let mut s = Strassenbau::neu(Some("Splines\\Marcel\\str_2spur_11m_SeeburgerStr1.sli".into()), Modus::Gerade);
        let g = aussen.extend(v.terrain_height(aussen.x, aussen.y).unwrap());
        s.klick(&mut v, g, 2.0, &ans, Some(&mut a));
        let z = DVec3::new(414.0, 215.0, 0.0);
        s.maus(&mut v, z, 2.0, &ans, Some(&mut a));
        println!("Plan: {:?}", s.plan.as_ref().map(|p| (p.laenge, p.blockiert.clone(), p.kreuzungen.len())));
        let m = s.klick(&mut v, z, 2.0, &ans, Some(&mut a)).unwrap();
        s.beenden(&mut v);
        println!("{m}");
        assert!(m.contains("Kreuzung") && m.contains("angeschlossen"), "{m}");
        let knoten = s.netz.knoten.iter().find(|x| x.kartenarme.len() == 3).expect("Knoten mit den 3 vorhandenen Armen");
        assert_eq!(s.netz.arme(knoten.id).len(), 4);
        for g in s.gesetzte_kreuzungen() {
            println!("  Kreuzung {} bei {:.1} {:.1}", g.rel, g.pos.x, g.pos.y);
        }
        for kn in &s.netz.knoten {
            println!("  Knoten {} bei {:.1} {:.1}: {} Kanten, {} Kartenarme", kn.id, kn.pos.x, kn.pos.y, s.netz.an(kn.id).len(), kn.kartenarme.len());
        }
        assert_eq!(s.gesetzte_kreuzungen().len(), 1, "{:?}", s.kreuzung_fehler);
        let karten: Vec<DVec2> = knoten.kartenarme.iter().map(|x| x.pos.truncate()).collect();
        if let Some(bild) = std::env::var_os("OMSI_BILD") {
            let kam = crate::kamera::Kamera { ziel: k.pos, gier: (frei + 180.0) as f32, neigung: -50.0, abstand: 90.0, fov: 50.0 };
            let px = v.render_image(1280, 800, &kam.camera()).unwrap();
            image::save_buffer(bild, &px, 1280, 800, image::ColorType::Rgba8).unwrap();
        }
        let n = speichern_laden_pruefen(v, &s, a, "Grundorf_vorhanden", &[aussen], &karten);
        assert_eq!(n, 1, "das Ende der neuen Strasse an der Kreuzung");
    }

    /// eigene Strasse aendern (umkehren, Querschnitt) und loeschen: die aufgeschnittene vorhandene Strasse wird wieder
    /// geschlossen; nach dem Speichern haengt das Flickstueck an beiden Enden
    #[test]
    #[ignore]
    fn eigene_strasse_aendern_und_heilen() {
        let _sperre = crate::bearbeiten::tests::sperre();
        use crate::anschluss::Anschluesse;
        use crate::strasse::{KantenAenderung, Modus, Strassenbau};
        let mut v = crate::bearbeiten::tests::grundorf();
        v.tiles_around(DVec3::new(150.0, 150.0, 0.0), 8).unwrap();
        let mut a = Aendern::neu(&v);
        let ab = abzweig_suchen(&v, &mut a);
        let ans = Anschluesse::default();
        let mut s = Strassenbau::neu(Some("Splines\\Marcel\\str_2spur_8m_altonaer1.sli".into()), Modus::Gerade);
        let aussen = ab.pos.truncate() + crate::netz::rechts(ab.richtung) * 60.0;
        let g = aussen.extend(v.terrain_height(aussen.x, aussen.y).unwrap());
        s.klick(&mut v, g, 2.0, &ans, Some(&mut a));
        s.maus(&mut v, ab.pos, 2.0, &ans, Some(&mut a));
        let m = s.klick(&mut v, ab.pos, 2.0, &ans, Some(&mut a)).unwrap();
        s.beenden(&mut v);
        assert!(m.contains("Kreuzung"), "{m}");
        let neu = s.netz.kanten[0].id;
        // auswaehlen wie im Werkzeug: unter einem Punkt der Strasse
        let mitte = s.kante_umriss(neu).unwrap().0[3].0;
        assert_eq!(s.kante_unter(mitte.truncate()), Some(neu));
        let m = s.kanten_aendern(&mut v, &[neu], &KantenAenderung::Umkehren);
        assert!(m.contains("umgekehrt"), "{m}");
        let e = s.netz.kante(neu).unwrap().clone();
        assert!(s.netz.knoten(e.a).unwrap().kartenarme.len() == 2, "nach dem Umkehren beginnt die Strasse an der Kreuzung");
        let m = s.kanten_aendern(&mut v, &[neu], &KantenAenderung::Querschnitt("Splines\\Marcel\\str_2spur_10m_Grunewaldstr.sli".into()));
        assert!(m.contains("umgestellt") && s.netz.kante(neu).unwrap().sli.contains("Grunewald"), "{m}");
        assert_eq!(s.gesetzte_kreuzungen().len(), 1);
        let m = s.kanten_aendern(&mut v, &[neu], &KantenAenderung::Loeschen);
        println!("{m}");
        assert!(m.contains("wieder geschlossen"), "{m}");
        assert_eq!(s.netz.kanten.len(), 1, "das Flickstueck");
        assert!(s.gesetzte_kreuzungen().is_empty());
        let flick = s.netz.kanten[0].clone();
        assert_eq!(flick.sli, ab.sli);
        let enden: Vec<DVec2> = [flick.a, flick.b].iter().map(|k| s.netz.knoten(*k).unwrap().pos.truncate()).collect();
        // Rueckgaengig bringt die Kreuzung wieder (3 Schritte zurueck: loeschen, Querschnitt, umkehren)
        assert!(s.rueckgaengig(&mut v, Some(&mut a)));
        assert_eq!(s.gesetzte_kreuzungen().len(), 1);
        assert!(s.wiederholen(&mut v, Some(&mut a)));
        assert!(s.gesetzte_kreuzungen().is_empty());
        let n = speichern_laden_pruefen(v, &s, a, "Grundorf_heilen", &[], &enden);
        assert_eq!(n, 2, "beide Enden des Flickstuecks haengen an der vorhandenen Strasse");
    }

    /// Werkzeug "Kreuzungen": Vorfahrt umstellen und Ampel setzen; vorhandene Kreuzung uebernehmen. Gespeichert und
    /// geladen: die Zufahrten der Ampelkreuzung haengen an ihrer Signalanlage (openOMSI liest [traffic_lights_group])
    #[test]
    #[ignore]
    fn vorfahrt_und_ampel_per_klick() {
        let _sperre = crate::bearbeiten::tests::sperre();
        use crate::anschluss::Anschluesse;
        use crate::kreuzung::Rolle;
        use crate::strasse::{Modus, Strassenbau};
        let mut v = crate::bearbeiten::tests::grundorf();
        v.tiles_around(DVec3::new(150.0, 150.0, 0.0), 8).unwrap();
        let mut a = Aendern::neu(&v);
        a.aktualisieren(&v);
        let mut s = Strassenbau::neu(Some("Splines\\Marcel\\str_2spur_8m_altonaer1.sli".into()), Modus::Gerade);
        // 1. die Ampelkreuzung von Grundorf uebernehmen (3 Arme, Vorfahrt vermutet)
        let vorhanden = a.kreuzungsobjekt_bei(&v, DVec2::new(414.0, 215.0)).unwrap();
        let k1 = s.vorhandene_uebernehmen(&mut v, &mut a, &vorhanden).unwrap();
        let info = s.kreuzungen().into_iter().find(|x| x.knoten == k1).unwrap();
        println!("uebernommen: {:?}", info.arme.iter().map(|x| (x.0.round(), x.1)).collect::<Vec<_>>());
        assert_eq!(info.arme.len(), 3);
        assert!(info.vermutet && !info.ampel);
        // 2. ein Abzweig an einer anderen Strasse, dort Vorfahrt und Ampel
        let ab = abzweig_suchen(&v, &mut a);
        let ans = Anschluesse::default();
        let aussen = ab.pos.truncate() + crate::netz::rechts(ab.richtung) * 60.0;
        let g = aussen.extend(v.terrain_height(aussen.x, aussen.y).unwrap());
        s.klick(&mut v, g, 2.0, &ans, Some(&mut a));
        s.maus(&mut v, ab.pos, 2.0, &ans, Some(&mut a));
        s.klick(&mut v, ab.pos, 2.0, &ans, Some(&mut a)).unwrap();
        s.beenden(&mut v);
        let k2 = s.kreuzungen().into_iter().find(|x| x.knoten != k1).expect("Abzweig-Kreuzung").knoten;
        let info = s.kreuzungen().into_iter().find(|x| x.knoten == k2).unwrap();
        assert_eq!(info.arme.iter().filter(|x| x.1 == Rolle::Haupt).count(), 2, "vorhandene Strasse hat Vorfahrt (vermutet)");
        // rechts vor links
        let regel = |r: Vec<Rolle>, ampel: bool| crate::netz::Regel { rollen: info.arme.iter().zip(r).map(|(x, r)| (x.0, r)).collect(), ampel };
        let m = s.regel_setzen(&mut v, k2, Some(regel(vec![Rolle::Gleich; 3], false)));
        println!("{m}");
        let i2 = s.kreuzungen().into_iter().find(|x| x.knoten == k2).unwrap();
        assert!(i2.arme.iter().all(|x| x.1 == Rolle::Gleich) && !i2.vermutet);
        // Ampel (mit Vorfahrt der vorhandenen Strasse)
        let rollen: Vec<Rolle> = info.arme.iter().map(|x| x.1).collect();
        let m = s.regel_setzen(&mut v, k2, Some(regel(rollen, true)));
        println!("{m}");
        let i2 = s.kreuzungen().into_iter().find(|x| x.knoten == k2).unwrap();
        assert!(i2.ampel && i2.umlauf.is_some(), "{:?}", s.kreuzung_fehler);
        assert_eq!(i2.phasen.iter().max(), Some(&1), "zwei Phasen");
        let g2 = s.gesetzte_kreuzungen().into_iter().find(|g| !g.signale.is_empty()).unwrap();
        assert_eq!(g2.signale.iter().filter(|x| x.art == "signal").count(), 3);
        // Rueckgaengig: wieder ohne Ampel, rechts vor links
        assert!(s.rueckgaengig(&mut v, Some(&mut a)));
        assert!(!s.kreuzungen().into_iter().find(|x| x.knoten == k2).unwrap().ampel);
        assert!(s.wiederholen(&mut v, Some(&mut a)));
        if let Some(bild) = std::env::var_os("OMSI_BILD") {
            let kam = crate::kamera::Kamera { ziel: ab.pos, gier: (ab.richtung + 200.0) as f32, neigung: -35.0, abstand: 55.0, fov: 50.0 };
            let px = v.render_image(1280, 800, &kam.camera()).unwrap();
            image::save_buffer(bild, &px, 1280, 800, image::ColorType::Rgba8).unwrap();
        }
        // speichern und laden: die Signale stehen mit [varparent] in der Kachel, die Zufahrten haengen an der Anlage
        let karten: Vec<DVec2> = s.netz.knoten.iter().flat_map(|k| k.kartenarme.iter().map(|x| x.pos.truncate())).collect();
        let neue_ampeln = std::cell::Cell::new(0usize);
        let n = speichern_laden_mit(v, &s, a, "Grundorf_ampel", &[aussen], &karten, |v2| {
            neue_ampeln.set(v2.lanes.lanes.iter().filter(|l| l.traffic_light.is_some() && l.points.first().is_some_and(|q| (q.truncate() - ab.pos.truncate()).length() < 30.0)).count());
        });
        println!("{n} Enden an Kreuzungen, {} Spuren mit Ampel", neue_ampeln.get());
        assert!(neue_ampeln.get() >= 3, "die Zufahrten der Ampelkreuzung");
    }

    /// "Speichern": als neue Karte anlegen (eigene Karte), dort weiterbauen und die Karte selbst ueberschreiben -
    /// Sicherung angelegt, Kreuzungsobjekte beider Sitzungen im Ordner der Karte, alles im Spurnetz verbunden
    #[test]
    #[ignore]
    fn speichern_ueberschreibt_eigene_karte() {
        let _sperre = crate::bearbeiten::tests::sperre();
        use crate::anschluss::Anschluesse;
        use crate::bearbeiten::{Bearbeiten, Werkzeug};
        use crate::strasse::{Modus, Strassenbau};
        let root = Path::new(crate::bearbeiten::tests::OMSI);
        let test_root = std::env::temp_dir().join(format!("omsi-editor-hier-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&test_root);
        fn kopieren(a: &Path, b: &Path) {
            std::fs::create_dir_all(b).unwrap();
            for e in std::fs::read_dir(a).unwrap().flatten() {
                if e.file_type().unwrap().is_dir() { kopieren(&e.path(), &b.join(e.file_name())) } else { std::fs::copy(e.path(), b.join(e.file_name())).unwrap(); }
            }
        }
        kopieren(&root.join("maps/Grundorf"), &test_root.join("maps/Grundorf"));
        assert!(!crate::speichern::eigene_karte(&test_root, "Grundorf"));
        let ans = Anschluesse::default();
        let sli = "Splines\\Marcel\\str_2spur_8m_altonaer1.sli";
        // Sitzung 1: Abzweig, als neue Karte speichern
        let mut v = crate::bearbeiten::tests::grundorf();
        v.tiles_around(DVec3::new(150.0, 150.0, 0.0), 8).unwrap();
        let mut a = Aendern::neu(&v);
        let ab = abzweig_suchen(&v, &mut a);
        let mut s = Strassenbau::neu(Some(sli.into()), Modus::Gerade);
        let aussen = ab.pos.truncate() + crate::netz::rechts(ab.richtung) * 60.0;
        let g = aussen.extend(v.terrain_height(aussen.x, aussen.y).unwrap());
        s.klick(&mut v, g, 2.0, &ans, Some(&mut a));
        s.maus(&mut v, ab.pos, 2.0, &ans, Some(&mut a));
        assert!(s.klick(&mut v, ab.pos, 2.0, &ans, Some(&mut a)).unwrap().contains("Kreuzung"));
        s.beenden(&mut v);
        let paket = crate::speichern::vorbereiten(&v, &Bearbeiten::neu(Werkzeug::Strasse), &s.netz, &s.gesetzte_kreuzungen(), &a.kopien("Grundorf"), Some(a.kreuzungs_ordner()), "Grundorf").unwrap();
        let karte = crate::speichern::karte_anlegen(&test_root, "Grundorf", "Grundorf_hier", &paket).unwrap();
        assert!(crate::speichern::eigene_karte(&test_root, "Grundorf_hier"), "Markierung fehlt");
        drop(a);
        drop(v);
        // Sitzung 2: die eigene Karte oeffnen, eine zweite Strasse mit Kreuzung, Karte selbst speichern
        let (mut v2, _) = Viewer::open(&openomsi_game::viewer::instance(), None, root, &karte.join("global.cfg")).unwrap();
        v2.session_overlay(&test_root);
        v2.tiles_around(DVec3::new(150.0, 150.0, 0.0), 8).unwrap();
        let mut a2 = Aendern::neu(&v2);
        // eine andere lange Strasse als die schon aufgeschnittene
        a2.aktualisieren(&v2);
        let mut kandidaten: Vec<KartenSpline> = a2.kacheln.values().flatten()
            .filter(|x| x.kurve.length > 45.0 && v2.spline_end_free(x.id, true).is_some() && (x.kurve.start.truncate() - ab.pos.truncate()).length() > 150.0)
            .cloned().collect();
        kandidaten.sort_by(|x, y| y.kurve.length.total_cmp(&x.kurve.length));
        let ab2 = kandidaten.iter().find_map(|x| a2.abzweig_bei(&v2, x.kurve.point_at(x.kurve.length / 2.0).truncate()).filter(|q| q.spline_id == x.id)).expect("zweite Strasse");
        let mut s2 = Strassenbau::neu(Some(sli.into()), Modus::Gerade);
        let aussen2 = ab2.pos.truncate() + crate::netz::rechts(ab2.richtung) * 60.0;
        let g2 = aussen2.extend(v2.terrain_height(aussen2.x, aussen2.y).unwrap());
        s2.klick(&mut v2, g2, 2.0, &ans, Some(&mut a2));
        s2.maus(&mut v2, ab2.pos, 2.0, &ans, Some(&mut a2));
        let m = s2.klick(&mut v2, ab2.pos, 2.0, &ans, Some(&mut a2)).unwrap();
        assert!(m.contains("Kreuzung"), "{m}");
        s2.beenden(&mut v2);
        let paket2 = crate::speichern::vorbereiten(&v2, &Bearbeiten::neu(Werkzeug::Strasse), &s2.netz, &s2.gesetzte_kreuzungen(), &a2.kopien("Grundorf_hier"), Some(a2.kreuzungs_ordner()), "Grundorf_hier").unwrap();
        let sicherung = crate::speichern::karte_ueberschreiben(&test_root, "Grundorf_hier", &paket2).unwrap();
        println!("Sicherung: {}", sicherung.display());
        assert!(sicherung.join("global.cfg").exists());
        let gesichert = std::fs::read_dir(&sicherung).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with("tile_")).count();
        assert!(gesichert >= 1, "ersetzte Kacheln gesichert");
        let objekte = test_root.join("Sceneryobjects/Aschaffenburg_KI/Grundorf_hier");
        let sco: Vec<String> = std::fs::read_dir(&objekte).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n.ends_with(".sco")).collect();
        println!("Kreuzungsobjekte: {sco:?}");
        assert_eq!(sco.len(), 2, "beide Sitzungen");
        drop(a2);
        drop(v2);
        // laden: beide Kreuzungen verweisen in den Ordner der Karte, die neue Strasse haengt
        let (mut v3, _) = Viewer::open(&openomsi_game::viewer::instance(), None, root, &karte.join("global.cfg")).unwrap();
        v3.session_overlay(&test_root);
        v3.tiles_around(DVec3::new(150.0, 150.0, 0.0), 8).unwrap();
        let mut verweise = 0;
        let mut ende2 = None;
        let start2 = s2.netz.kanten.iter().find(|e| s2.netz.ist_kreuzung(e.b)).map(|e| { let el = s2.netz.elemente(e); let l = el.last().unwrap(); l.stueck.ende().0 }).unwrap();
        for e in std::fs::read_dir(&karte).unwrap().flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            let Some(rest) = n.strip_prefix("tile_").and_then(|r| r.strip_suffix(".map")) else { continue };
            let mut it = rest.split('_').map(|x| x.parse::<i32>().unwrap());
            let (tx, ty) = (it.next().unwrap(), it.next().unwrap());
            let t = omsi_map::Tile::load(&e.path()).unwrap();
            verweise += t.objects.iter().filter(|o| o.file.contains("Aschaffenburg_KI\\Grundorf_hier\\K_")).count();
            for sp in &t.splines {
                let k = omsi_geometry::SplineCurve::from_map(sp, DVec2::new(tx as f64 * 300.0, ty as f64 * 300.0));
                if (k.end_point().truncate() - start2).length() < 0.01 {
                    ende2 = Some(sp.id);
                }
            }
        }
        assert_eq!(verweise, 2);
        let ende2 = ende2.expect("Strasse der zweiten Sitzung");
        assert_eq!(v3.spline_end_free(ende2, true), Some(false), "zweite Strasse haengt nicht an ihrer Kreuzung");
        std::fs::remove_dir_all(&test_root).ok();
        let _ = std::fs::remove_dir_all(crate::speichern::sicherungen().join("Grundorf_hier"));
    }

    #[test]
    #[ignore]
    fn abzweig_mitten_aus_vorhandener_strasse() {
        let _sperre = crate::bearbeiten::tests::sperre();
        let mut v = crate::bearbeiten::tests::grundorf();
        v.tiles_around(DVec3::new(150.0, 150.0, 0.0), 8).unwrap();
        let mut a = Aendern::neu(&v);
        let ab = abzweig_suchen(&v, &mut a);
        let alt = a.spline(ab.spline_id).unwrap().clone();
        println!("Abzweig von Spline {} ({}, {:.1} m) bei {:.1} m", ab.spline_id, ab.sli, alt.kurve.length, ab.s);
        // Schnitte wie fuer einen Arm quer zur Strasse (halbe Breite 5 m)
        let (vor, nach) = Aendern::schnitte(&ab, &[(ab.richtung + 90.0, 5.0)]);
        let arme = a.aufschneiden(&mut v, &ab, vor, nach).unwrap();
        assert_eq!(arme.len(), 2);
        assert!(((arme[0].pos.truncate() - ab.pos.truncate()).length() - vor).abs() < 0.5 && ((arme[1].pos.truncate() - ab.pos.truncate()).length() - nach).abs() < 0.5);
        assert!(norm180(arme[1].richtung - ab.richtung).abs() < 15.0 && norm180(arme[0].richtung - ab.richtung - 180.0).abs() < 15.0);
        a.aktualisieren(&v);
        // der alte Spline ist gekuerzt (gleiche ID), das Stueck hinter der Kreuzung hat eine neue ID
        let vorn = a.spline(ab.spline_id).expect("vorderes Stueck").clone();
        assert!(vorn.kurve.length < ab.s && vorn.next == 0, "vorn {:.1} m next {}", vorn.kurve.length, vorn.next);
        assert!((vorn.kurve.start - alt.kurve.start).length() < 0.01);
        let hinten = a.kacheln.values().flatten().find(|s| s.prev == 0 && s.next == alt.next && s.id != alt.id && s.sli == alt.sli
            && (s.kurve.end_point() - alt.kurve.end_point()).length() < 0.01).expect("hinteres Stueck").clone();
        // Hoehen: an der Kreuzung eben auf ihrer Hoehe, am anderen Ende wie vorher
        assert!((vorn.kurve.end_point().z - ab.pos.z).abs() < 0.01 && (hinten.kurve.start.z - ab.pos.z).abs() < 0.01);
        assert!((hinten.kurve.end_point().z - alt.kurve.end_point().z).abs() < 0.01);
        // (ob die Enden an der Kreuzung haengen, prueft abzweig_bauen_speichern_laden nach dem Speichern)
        // OMSI_BILD=pfad.png: Bild der Kreuzung (von oben schraeg)
        if let Some(bild) = std::env::var_os("OMSI_BILD") {
            let k = crate::kamera::Kamera { ziel: ab.pos, gier: (ab.richtung + 200.0) as f32, neigung: -50.0, abstand: 70.0, fov: 50.0 };
            let px = v.render_image(1280, 800, &k.camera()).unwrap();
            image::save_buffer(bild, &px, 1280, 800, image::ColorType::Rgba8).unwrap();
        }
        // Rueckgaengig: alles wie vorher
        a.rueckgaengig(&mut v).unwrap();
        a.aktualisieren(&v);
        assert!((a.spline(ab.spline_id).unwrap().kurve.length - alt.kurve.length).abs() < 1e-6);
        assert!(a.spline(hinten.id).is_none());
        drop(a);
    }
}

#[cfg(test)]
mod nutzer_tests {
    use super::*;

    /// nur in der Sitzung (nichts gespeichert): eine gespeicherte Kreuzung der Karte des Nutzers neu bauen und zeigen
    #[test]
    #[ignore]
    fn gespeicherte_kreuzung_neu_bauen() {
        let _sperre = crate::bearbeiten::tests::sperre();
        let Ok(karte) = std::env::var("OMSI_KARTE") else { return };
        let x: f64 = std::env::var("OMSI_X").unwrap().parse().unwrap();
        let y: f64 = std::env::var("OMSI_Y").unwrap().parse().unwrap();
        let root = Path::new(crate::bearbeiten::tests::OMSI);
        let (mut v, _) = Viewer::open(&openomsi_game::viewer::instance(), None, root, &root.join("maps").join(&karte).join("global.cfg")).unwrap();
        v.tiles_around(DVec3::new(x, y, 0.0), 2).unwrap();
        let mut a = Aendern::neu(&v);
        a.aktualisieren(&v);
        let k = (0..400).find_map(|i| {
            let w = i as f64 * 0.7;
            a.kreuzungsobjekt_bei(&v, DVec2::new(x + w.cos() * i as f64 * 0.3, y + w.sin() * i as f64 * 0.3)).filter(|k| k.arme.len() >= 3)
        }).expect("keine Kreuzung");
        println!("bei {:?}", k.pos);
        println!("Kreuzung {} mit {} Armen", k.objekt, k.arme.len());
        for x in &k.arme {
            println!("  Arm bei {:.1} {:.1}, Richtung {:.1}, {}", x.pos.x, x.pos.y, x.richtung, x.sli);
        }
        let mut s = crate::strasse::Strassenbau::neu(None, crate::strasse::Modus::Gerade);
        let kam = crate::kamera::Kamera { ziel: k.pos, gier: 200.0, neigung: -55.0, abstand: 45.0, fov: 50.0 };
        if let Some(b) = std::env::var_os("OMSI_BILD_VORHER") {
            let px = v.render_image(1280, 800, &kam.camera()).unwrap();
            image::save_buffer(b, &px, 1280, 800, image::ColorType::Rgba8).unwrap();
        }
        s.vorhandene_uebernehmen(&mut v, &mut a, &k).unwrap();
        v.tiles_around(DVec3::new(x, y, 0.0), 2).unwrap();
        for _ in 0..3 {
            v.tiles_around(DVec3::new(x, y, 0.0), 2).unwrap();
        }
        if let Some(b) = std::env::var_os("OMSI_BILD") {
            let px = v.render_image(1280, 800, &kam.camera()).unwrap();
            image::save_buffer(b, &px, 1280, 800, image::ColorType::Rgba8).unwrap();
        }
        drop(a);
    }
}
