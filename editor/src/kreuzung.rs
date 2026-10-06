//! Kreuzungen an vorhandenen Strassen (wie in Transport Fever 2: eine neue Strasse zweigt mitten aus einer
//! vorhandenen ab). Die Strasse wird an der Stelle aufgeschnitten (die Spline-Kette auch ueber mehrere Splines
//! gekuerzt bzw. geteilt), dazwischen kommt ein Kreuzungsobjekt wie in den Standardkarten - Platte, Abbiegespuren,
//! Vorfahrt - erzeugt von omsigen (python -m omsigen.editorkreuzung), und die neue Strasse beginnt am dritten Arm.
//!
//! Alles geht ueber das Werkzeug "Aendern" (Sitzungskopien der Kacheln, ein Rueckgaengig-Schritt); die Objekte
//! liegen im Sitzungsordner unter Sceneryobjects/Aschaffenburg_KI/<tag>/ und kommen beim Speichern in den Ordner
//! der neuen Karte.

use crate::aendern::{eintrag_ende, eintrag_finden, Aendern, KartenSpline};
use crate::netz::{self, norm180};
use anyhow::{bail, Context, Result};
use glam::{DVec2, DVec3};
use openomsi_game::viewer::Viewer;
use std::collections::BTreeMap;
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

/// Vorfahrt eines Arms (wie omsigen vorfahrt.HAUPT/NEBEN/GLEICH)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rolle {
    Haupt,
    Neben,
    Gleich,
}

impl Rolle {
    fn text(self) -> &'static str {
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

    /// Abzweig so verschieben, dass die Kreuzung (Schnitt je Seite fuer einen Arm Richtung `richtung` mit halber
    /// Breite `halb_arm`) auf die Strasse passt: nahe einem Ende rueckt sie in die Strasse hinein. Fehler nur, wenn
    /// die Strasse (die lueckenlose Kette) zu kurz ist.
    pub fn abzweig_einpassen(&self, ab: &Abzweig, richtung: f64, halb_arm: f64) -> Result<Abzweig> {
        let h = arm_richtung(ab.richtung, richtung);
        let (d, _) = masse(ab.richtung, h, ab.halb, halb_arm);
        let kette = self.kette_um(ab.spline_id, 2.0 * d + 20.0);
        let g = kette.iter().find(|g| g.s.id == ab.spline_id).context("Spline nicht geladen")?;
        let anfang = kette.first().map(|g| g.von).unwrap_or(0.0) + d + MIN_REST + 0.5;
        let ende = kette.last().map(|g| g.bis()).unwrap_or(0.0) - d - MIN_REST - 0.5;
        if ende < anfang {
            bail!("die Strasse ist hier zu kurz fuer eine Kreuzung (braucht {:.0} m bis zum naechsten Ende/zur naechsten Kreuzung)", 2.0 * (d + MIN_REST));
        }
        let mitte = (g.von + ab.s).clamp(anfang, ende);
        let g = kette.iter().find(|g| mitte >= g.von && mitte <= g.bis()).unwrap_or(g);
        let s = (mitte - g.von).clamp(0.0, g.s.kurve.length);
        Ok(Abzweig { spline_id: g.s.id, s, pos: g.s.kurve.point_at(s), richtung: g.s.kurve.heading_at(s).rem_euclid(360.0),
                     sli: g.s.sli.clone(), halb: ab.halb })
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

    /// Kreuzung an der Abzweig-Stelle bauen: Strasse aufschneiden, Kreuzungsobjekt setzen. Der neue Arm zeigt in
    /// `richtung` (wird auf MIN_WINKEL begrenzt) und bekommt Querschnitt `sli`; `weg`: die neue Strasse beginnt
    /// an der Kreuzung (sonst endet sie dort). -> der neue Arm (Lage, Richtung von der Kreuzung weg; eben)
    pub fn kreuzung_bauen(&mut self, v: &mut Viewer, ab: &Abzweig, richtung: f64, sli: &str, weg: bool) -> Result<(Arm, Objekt)> {
        let (l, r) = v.spline_lanes(sli).map(|x| x.1).context("Querschnitt unbekannt")?;
        let halb_neu = l.max(r) as f64;
        let ab = &self.abzweig_einpassen(ab, richtung, halb_neu)?;
        let richtung = arm_richtung(ab.richtung, richtung);
        let (d, d_arm) = masse(ab.richtung, richtung, ab.halb, halb_neu);
        let t0 = std::time::Instant::now();
        log::info!("Kreuzung bauen: Spline {} bei {:.1} m ({}), Arm {:.1} Grad, Schnitt {:.1} m je Seite, Arm {:.1} m", ab.spline_id, ab.s, ab.sli, richtung, d, d_arm);
        crate::protokoll::aktion(&format!("Kreuzung bauen: Kette um Spline {}", ab.spline_id));
        let kette = self.kette_um(ab.spline_id, d + 5.0);
        log::info!("  Kette: {} Splines {:?} ({} ms)", kette.len(), kette.iter().map(|g| g.s.id).collect::<Vec<_>>(), t0.elapsed().as_millis());
        let g = kette.iter().find(|g| g.s.id == ab.spline_id).context("Spline nicht geladen")?;
        let mitte = g.von + ab.s;
        let (mut lo, mut hi) = (mitte - d, mitte + d);
        let anfang = kette.first().map(|g| g.von).unwrap_or(0.0);
        let ende = kette.last().map(|g| g.bis()).unwrap_or(0.0);
        if lo < anfang + MIN_REST || hi > ende - MIN_REST {
            bail!("zu nah am Ende der Strasse oder an einer Kreuzung - etwas weiter in der Mitte abzweigen");
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
        crate::protokoll::aktion("Kreuzung bauen: naechste freie ID (Kartenindex)");
        let mut ids = v.next_object_id();
        log::info!("  naechste ID {ids} ({} ms)", t0.elapsed().as_millis());
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
        // Arme: die zwei Enden der Strasse und der neue
        let c = ab.pos.truncate();
        let arm_pos = c + netz::dir(richtung) * d_arm;
        let arme = vec![
            Arm { pos: p_lo, richtung: (h_lo + 180.0).rem_euclid(360.0), sli: g_lo.s.sli.clone(), weg: g_lo.s.gespiegelt, rolle: Rolle::Haupt },
            Arm { pos: p_hi, richtung: h_hi.rem_euclid(360.0), sli: g_hi.s.sli.clone(), weg: !g_hi.s.gespiegelt, rolle: Rolle::Haupt },
            Arm { pos: arm_pos.extend(hoehe), richtung, sli: sli.to_string(), weg, rolle: Rolle::Neben },
        ];
        let (ordner, _) = self.kreuzungs_ordner();
        let rel_ordner = format!("Sceneryobjects\\Aschaffenburg_KI\\{}", self.tag);
        let name = freier_name(&ordner);
        crate::protokoll::aktion("Kreuzung bauen: omsigen erzeugt das Kreuzungsobjekt");
        let obj = erzeugen(&v.root, &ordner, &rel_ordner, &name, &arme)?;
        log::info!("  Objekt {} bei {:?}: {} Abbiegespuren, {} Regeln ({} ms)", obj.rel, obj.ursprung, obj.spuren, obj.rules.len(), t0.elapsed().as_millis());
        let obj_id = neue_id();
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
        // Kreuzungsobjekt ([absheight]: Hoehe absolut) mit den Vorfahrtregeln seiner Pfade
        let ko = kachel_von(obj.ursprung);
        let mut z = vec!["[object]".to_string(), "0".into(), obj.rel.clone(), obj_id.to_string(),
                         zahl(obj.ursprung.x - ko.0 as f64 * ts), zahl(obj.ursprung.y - ko.1 as f64 * ts), zahl(hoehe),
                         "0".into(), "0".into(), "0".into(), "0".into()];
        for (pfad, wert) in &obj.rules {
            z.extend(["".into(), "[rule]".into(), pfad.to_string(), "priority".into(), wert.to_string(), "0".into()]);
        }
        anhaengen.entry(ko).or_default().push(z);
        // Nachbarn, deren prev auf den geteilten Spline zeigte
        let mut prev_neu: BTreeMap<(i32, i32), Vec<(i64, i64)>> = BTreeMap::new();
        for (id, neu) in verweise {
            let s = self.spline(id).context("Nachbar-Spline nicht geladen")?;
            prev_neu.entry(s.kachel).or_default().push((id, neu));
        }
        let mut kacheln: Vec<(i32, i32)> = aendern.keys().chain(anhaengen.keys()).chain(prev_neu.keys()).copied().collect();
        kacheln.sort();
        kacheln.dedup();
        crate::protokoll::aktion(&format!("Kreuzung bauen: Kacheln {kacheln:?} umschreiben und neu laden"));
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
        let arm = arme[2].clone();
        Ok((Arm { pos: arm_pos.extend(hoehe), ..arm }, obj))
    }
}

/// beginnt mit dieser Zeile ein neuer Eintrag der Kachel (Spline, Objekt, ...)?
fn ist_eintrag(z: &str) -> bool {
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

fn zahl(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

/// naechster freier Name K_E0001, K_E0002, ... im Ordner
pub fn freier_name(ordner: &Path) -> String {
    (1..).map(|i| format!("K_E{i:04}")).find(|n| !ordner.join(format!("{n}.sco")).exists()).unwrap()
}

/// Ordner von omsigen (das Repository): OMSIGEN_DIR, sonst neben dem Editor-Quelltext
pub fn omsigen_ordner() -> PathBuf {
    std::env::var_os("OMSIGEN_DIR").map(PathBuf::from).unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join(".."))
}

/// Kreuzungsobjekt von omsigen bauen lassen (Python, OMSIGEN_PYTHON oder python)
pub fn erzeugen(root: &Path, ordner: &Path, rel_ordner: &str, name: &str, arme: &[Arm]) -> Result<Objekt> {
    use std::io::Write;
    let auftrag = serde_json::json!({
        "omsi": root, "ordner": ordner, "rel_ordner": rel_ordner, "name": name, "titel": format!("Editor-Kreuzung {name}"),
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let o = erzeugen(root, &d, "Sceneryobjects\\Aschaffenburg_KI\\Test", "K_E0001", &arme).unwrap();
        assert_eq!(o.rel, "Sceneryobjects\\Aschaffenburg_KI\\Test\\K_E0001.sco");
        assert_eq!((o.spuren, o.fehlgeschlagen), (6, 0));
        assert!(!o.rules.is_empty() && d.join("K_E0001.sco").exists() && d.join("model").join("K_E0001.x").exists());
        assert_eq!(freier_name(&d), "K_E0002");
        let _ = std::fs::remove_dir_all(&d);
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
        let e = &s.netz.kanten[0];
        let start = s.netz.knoten(e.a).unwrap().pos;
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
        assert!(test_root.join("Sceneryobjects/Aschaffenburg_KI/Grundorf_kreuzung/K_E0001.sco").exists());
        // neue Strasse: der Spline, der am Arm beginnt
        let mut neu_id = None;
        let mut verweis = false;
        for e in std::fs::read_dir(&karte).unwrap().flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            let Some(rest) = n.strip_prefix("tile_").and_then(|r| r.strip_suffix(".map")) else { continue };
            let mut it = rest.split('_').map(|x| x.parse::<i32>().unwrap());
            let (tx, ty) = (it.next().unwrap(), it.next().unwrap());
            let t = omsi_map::Tile::load(&e.path()).unwrap();
            verweis |= t.objects.iter().any(|o| o.file.contains("Aschaffenburg_KI\\Grundorf_kreuzung\\K_E0001.sco"));
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
                    if ms > 100 {
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
                let ok = m.as_deref().is_some_and(|m| m.contains("Kreuzung ("));
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
        let boden = |v: &Viewer, x: f64, y: f64| DVec3::new(x, y, v.terrain_height(x, y).unwrap());
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
            let k = crate::kamera::Kamera { ziel: DVec3::new(120.0, 170.0, boden(&v, 120.0, 170.0).z), gier: 200.0, neigung: -55.0, abstand: 150.0, fov: 50.0 };
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
                let rand = [(120.0, 100.0), (120.0, 230.0), (190.0, 140.0), (50.0, 200.0)].iter().any(|q| (p.truncate() - DVec2::new(q.0, q.1)).length() < 0.5);
                if !innen && !rand {
                    an_kreuzung += 1;
                    assert_eq!(v2.spline_end_free(*id, am_ende), Some(false), "Spline {id} {} haengt nicht an der Kreuzung", if am_ende { "Ende" } else { "Anfang" });
                }
            }
        }
        assert_eq!(an_kreuzung, 6, "2 Kreuzungen mit je 3 Armen");
        std::fs::remove_dir_all(&test_root).ok();
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
        let sli = "Splines\\Marcel\\str_2spur_8m_altonaer1.sli";
        let (arm, obj) = a.kreuzung_bauen(&mut v, &ab, ab.richtung + 90.0, sli, true).unwrap();
        println!("Kreuzung {} bei {:?}: {} Abbiegespuren, {} Regeln; Arm bei {:?} Richtung {:.1}", obj.rel, obj.ursprung, obj.spuren, obj.rules.len(), arm.pos, arm.richtung);
        assert_eq!(obj.fehlgeschlagen, 0);
        assert!(norm180(arm.richtung - ab.richtung - 90.0).abs() < 1e-6);
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
        // das Kreuzungsobjekt steht in der Karte, und laut Spurnetz fuehren beide Strassenenden in seine Pfade
        // (v.objects() kennt nur am Boden stehende Objekte; die Kreuzung hat [absheight])
        assert_eq!(v.spline_end_free(vorn.id, true), Some(false), "vorderes Stueck endet frei");
        assert_eq!(v.spline_end_free(hinten.id, false), Some(false), "hinteres Stueck beginnt frei");
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
