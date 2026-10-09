//! Werkzeug "Knoten" (N): den Verlauf vorhandener und eigener Strassen an ihren Knoten ziehen, wie in Transport
//! Fever 2. Knoten sind die Verbindungen zweier Splines der Karte, ihre freien Enden, jede Stelle mitten auf einem
//! Spline (Ziehen teilt ihn dort), die Knoten des eigenen Netzes und vorhandene Kreuzungsobjekte.
//!
//! Die Splines am gezogenen Knoten werden durch Boegen (netz::verbinden) ersetzt, die an ihren festen Enden in Lage,
//! Richtung, Steigung und Querneigung genau wie vorher anschliessen - die Spuren zu den Nachbarn bleiben verbunden.
//! Am Knoten selbst bleibt der Verlauf glatt (beide Seiten bekommen dieselbe Richtung, die sich mit der Sehne dreht;
//! das Mausrad dreht sie beim Ziehen von Hand). Ein Zug ist ein Rueckgaengig-Schritt.

use crate::aendern::{eintrag_ende, eintrag_finden, Aendern, KartenSpline};
use crate::kreuzung::{ist_eintrag, zahl, Vorhanden};
use crate::netz::{self, norm180, richtung, Element, Netz};
use crate::strasse::Strassenbau;
use anyhow::{bail, Context, Result};
use glam::{DVec2, DVec3};
use openomsi_game::viewer::Viewer;
use std::collections::{BTreeMap, HashMap, HashSet};

/// so nah muessen Spline-Enden beieinander liegen, um als ein Knoten zu gelten
const FANG_ENDEN: f64 = 0.3;
/// engere Kurven werden angemerkt
const ENGER_RADIUS: f64 = 12.0;
/// so lang wird ein Stueck mindestens neu gelegt (verknuepfte Nachbar-Splines mit gleichem Querschnitt kommen dazu):
/// kurze Splines an Kreuzungen ergaeben sonst enge S-Kurven
const UMLEGE_MIN: f64 = 30.0;

/// Lage an einem Stuetzpunkt: Punkt mit Hoehe, Richtung (Grad), Steigung (Verhaeltnis), Querneigung (wie in der Kachel)
#[derive(Clone, Copy, Debug)]
pub struct Lage {
    pub pos: DVec3,
    pub richtung: f64,
    pub steigung: f64,
    pub quer: f64,
}

impl Lage {
    pub fn anfang(s: &KartenSpline) -> Lage {
        let k = &s.kurve;
        Lage { pos: k.start, richtung: k.heading_deg.rem_euclid(360.0), steigung: k.slope_at(0.0), quer: k.cant_start }
    }

    pub fn ende(s: &KartenSpline) -> Lage {
        let k = &s.kurve;
        Lage { pos: k.end_point(), richtung: k.heading_at(k.length).rem_euclid(360.0), steigung: k.slope_at(k.length), quer: k.cant_end }
    }

    pub fn bei(s: &KartenSpline, x: f64) -> Lage {
        let k = &s.kurve;
        Lage { pos: k.point_at(x), richtung: k.heading_at(x).rem_euclid(360.0), steigung: k.slope_at(x), quer: k.cant_at(x) }
    }
}

/// eine Kette von Splines der Karte (`alt`, dann `weitere` in Splinerichtung, gleicher Querschnitt), ersetzt durch
/// Boegen ueber die Stuetzpunkte (der erste und letzte sind die Enden der Kette, die bewegten liegen woanders);
/// Querschnitt, Spiegelung, Anschluesse bleiben
#[derive(Clone, Debug)]
pub struct Umlegung {
    pub alt: KartenSpline,
    pub weitere: Vec<KartenSpline>,
    pub punkte: Vec<Lage>,
}

impl Umlegung {
    /// alle Splines der Kette in Splinerichtung
    pub fn kette(&self) -> Vec<&KartenSpline> {
        std::iter::once(&self.alt).chain(self.weitere.iter()).collect()
    }

    /// Elemente mit Querneigung am Anfang und Ende; Fehler, wenn sich ein Abschnitt nicht legen laesst
    pub fn elemente(&self) -> Result<Vec<(Element, f64, f64)>> {
        let mut out = Vec::new();
        for w in self.punkte.windows(2) {
            let (a, b) = (w[0], w[1]);
            let sehne = (b.pos - a.pos).truncate().length();
            if sehne < 0.5 {
                bail!("Knoten zu nah am festen Ende");
            }
            let st = netz::verbinden(a.pos.truncate(), a.richtung, b.pos.truncate(), b.richtung);
            let l: f64 = st.iter().map(|s| s.laenge).sum();
            if st.is_empty() || !l.is_finite() {
                bail!("der Verlauf laesst sich so nicht legen");
            }
            // der Bogen liefe in einer Schleife zurueck (Knoten hinter das feste Ende gezogen)
            if l > 2.5 * sehne + 40.0 {
                bail!("der Verlauf liefe in einer Schleife - Knoten weniger weit ziehen oder mit dem Mausrad drehen");
            }
            let el = netz::mit_hoehe(&st, a.pos.z, a.steigung, b.pos.z, b.steigung);
            if el.is_empty() {
                bail!("der Verlauf laesst sich so nicht legen");
            }
            let mut s0 = 0.0;
            for e in el {
                let qa = a.quer + (b.quer - a.quer) * s0 / l;
                s0 += e.stueck.laenge;
                let qb = a.quer + (b.quer - a.quer) * s0 / l;
                out.push((e, qa, qb));
            }
        }
        Ok(out)
    }
}

/// ein Spline-Ende an einem Knoten
#[derive(Clone, Debug)]
pub struct Ende {
    pub spline: KartenSpline,
    pub am_ende: bool,
    /// verknuepfte Splines dahinter (vom Knoten weg), die mit neu gelegt werden
    pub weiter: Vec<KartenSpline>,
}

impl Ende {
    fn neu(spline: KartenSpline, am_ende: bool) -> Ende {
        Ende { spline, am_ende, weiter: vec![] }
    }

    fn lage(&self) -> Lage {
        if self.am_ende { Lage::ende(&self.spline) } else { Lage::anfang(&self.spline) }
    }

    /// das feste Ende: das andere Ende des letzten Splines vom Knoten weg
    fn fern(&self) -> Lage {
        let s = self.weiter.last().unwrap_or(&self.spline);
        if self.am_ende { Lage::anfang(s) } else { Lage::ende(s) }
    }

    /// Splines vom Knoten weg anhaengen (gleicher Querschnitt, lueckenlos verknuepft, keine Abzweigung), bis die
    /// Kette UMLEGE_MIN lang ist; `ausser`: schon anderweitig vergebene Splines
    fn verlaengern(&mut self, ae: &Aendern, ausser: &HashSet<i64>) {
        self.weiter.clear();
        let mut laenge = self.spline.kurve.length;
        let mut cur = self.spline.clone();
        let mut benutzt: HashSet<i64> = ausser.clone();
        benutzt.insert(self.spline.id);
        while laenge < UMLEGE_MIN && self.weiter.len() < 12 {
            // das ferne Ende von cur und die Richtung dort (in Splinerichtung)
            let (q, h) = if self.am_ende { (cur.kurve.start, cur.kurve.heading_deg) } else { (cur.kurve.end_point(), cur.kurve.heading_at(cur.kurve.length)) };
            let mut dort: Vec<(&KartenSpline, bool)> = Vec::new();
            for sp in ae.kacheln.values().flatten() {
                if sp.id == cur.id {
                    continue;
                }
                for am_ende in [false, true] {
                    let p = if am_ende { sp.kurve.end_point() } else { sp.kurve.start };
                    if (p - q).truncate().length() < FANG_ENDEN {
                        dort.push((sp, am_ende));
                    }
                }
            }
            // genau ein Nachbar, in gleicher Richtung fortgesetzt
            let [(n, n_am_ende)] = dort[..] else { break };
            let passt = n_am_ende == self.am_ende
                && norm180(if n_am_ende { n.kurve.heading_at(n.kurve.length) } else { n.kurve.heading_deg } - h).abs() < 2.0
                && n.sli == cur.sli && n.gespiegelt == cur.gespiegelt && !benutzt.contains(&n.id);
            if !passt {
                break;
            }
            benutzt.insert(n.id);
            laenge += n.kurve.length;
            cur = n.clone();
            self.weiter.push(cur.clone());
        }
    }

    /// die Kette mit diesem Ende an `pos`, ihre Richtung dort um `dreh` gedreht
    fn umlegung(&self, pos: DVec3, dreh: f64) -> Umlegung {
        let mut l = self.lage();
        l.pos = pos;
        l.richtung = (l.richtung + dreh).rem_euclid(360.0);
        let f = self.fern();
        // in Splinerichtung: am Ende des Splines liegt die Kette davor
        let mut kette: Vec<KartenSpline> = if self.am_ende {
            self.weiter.iter().rev().cloned().chain(std::iter::once(self.spline.clone())).collect()
        } else {
            std::iter::once(self.spline.clone()).chain(self.weiter.iter().cloned()).collect()
        };
        let alt = kette.remove(0);
        Umlegung { alt, weitere: kette, punkte: if self.am_ende { vec![f, l] } else { vec![l, f] } }
    }

    /// Richtung der Sehne zwischen festem Ende und dem Knoten bei p (in Splinerichtung)
    fn sehne(&self, p: DVec2) -> f64 {
        let f = self.fern().pos.truncate();
        if self.am_ende { richtung(f, p) } else { richtung(p, f) }
    }
}

/// was sich ziehen laesst
#[derive(Clone, Debug)]
pub enum Griff {
    /// Verbindung von Splines der Karte oder ein freies Ende
    Karte { enden: Vec<Ende>, pos: DVec3 },
    /// Stelle mitten auf einem Spline der Karte (s Meter ab Anfang): Ziehen teilt ihn dort
    Mitte { spline: KartenSpline, s: f64, pos: DVec3 },
    /// Knoten des eigenen Netzes
    Netz { knoten: u32, pos: DVec3 },
    /// vorhandenes Kreuzungsobjekt der Karte: wird verschoben, die angeschlossenen Splines folgen
    Kreuzung { k: Vorhanden, enden: Vec<Ende> },
}

impl Griff {
    pub fn pos(&self) -> DVec3 {
        match self {
            Griff::Karte { pos, .. } | Griff::Mitte { pos, .. } | Griff::Netz { pos, .. } => *pos,
            Griff::Kreuzung { k, .. } => k.pos,
        }
    }

    pub fn text(&self) -> &'static str {
        match self {
            Griff::Karte { enden, .. } if enden.len() == 1 => "Strassenende",
            Griff::Karte { .. } => "Knoten",
            Griff::Mitte { .. } => "Strasse (Ziehen setzt hier einen Knoten)",
            Griff::Netz { .. } => "Knoten (eigene Strasse)",
            Griff::Kreuzung { .. } => "Kreuzung",
        }
    }
}

/// Ergebnis eines Zugs, bevor er angewendet wird
#[derive(Clone, Debug, Default)]
pub struct Plan {
    pub umlegungen: Vec<Umlegung>,
    /// Objekte der Karte verschieben: (Kachel, ID, Versatz)
    pub objekte: Vec<((i32, i32), i64, DVec2)>,
    /// eigenes Netz danach (Knoten des Netzes gezogen)
    pub netz: Option<Netz>,
    /// Vorschau: Mittellinien (Punkte mit Richtung) und halbe Breite
    pub linien: Vec<(Vec<(DVec3, f64)>, f64)>,
    pub ziel: DVec3,
    pub fehler: Option<String>,
    pub warnung: Option<String>,
}

struct Zug {
    griff: Griff,
    /// Bodenpunkt beim Greifen
    greif: DVec2,
    letzter: DVec3,
}

#[derive(Default)]
pub struct Knotenwerkzeug {
    /// Griffe in der Naehe der Maus (zum Anzeigen) und der unter der Maus
    pub griffe: Vec<Griff>,
    pub unter_maus: Option<usize>,
    /// Stelle auf der Strasse unter der Maus (neuer Knoten nur mit Umschalt)
    pub mitte: Option<usize>,
    zug: Option<Zug>,
    pub plan: Option<Plan>,
    /// beim Ziehen: Richtung am Knoten von Hand gedreht (Mausrad), Hoehe verschoben (Bild auf/ab)
    pub dreh: f64,
    pub hoehe: f64,
    frei: HashMap<(i64, bool), bool>,
    stand: usize,
}

impl Knotenwerkzeug {
    pub fn zieht(&self) -> bool {
        self.zug.is_some()
    }

    pub fn abbrechen(&mut self) {
        self.zug = None;
        self.plan = None;
        self.dreh = 0.0;
        self.hoehe = 0.0;
    }

    /// Griffe im Umkreis der Maus suchen (ohne Zug)
    pub fn suchen(&mut self, v: &Viewer, ae: &mut Aendern, s: &Strassenbau, boden: DVec3, fang: f64, umkreis: f64) {
        ae.aktualisieren(v);
        if ae.aenderungen != self.stand {
            self.frei.clear();
            self.stand = ae.aenderungen;
        }
        let p = boden.truncate();
        let mut griffe = Vec::new();
        // eigene Knoten, und wo eigene Strassen an die Karte anschliessen (dort keine Griffe der Karte)
        let mut eigene: Vec<DVec2> = Vec::new();
        for k in &s.netz.knoten {
            eigene.push(k.pos.truncate());
            eigene.extend(k.kartenarme.iter().map(|a| a.pos.truncate()));
            if (k.pos.truncate() - p).length() < umkreis {
                griffe.push(Griff::Netz { knoten: k.id, pos: k.pos });
            }
        }
        // Enden der Strassen-Splines im Umkreis, zu Knoten zusammengefasst
        let splines: Vec<KartenSpline> = ae.kacheln.values().flatten()
            .filter(|x| (x.kurve.start.truncate() - p).length() < umkreis || (x.kurve.end_point().truncate() - p).length() < umkreis)
            .cloned().collect();
        let mut enden: Vec<Ende> = Vec::new();
        for sp in splines {
            if !ist_strasse(v, ae, &sp.sli) {
                continue;
            }
            enden.push(Ende::neu(sp.clone(), false));
            enden.push(Ende::neu(sp, true));
        }
        let mut genommen = vec![false; enden.len()];
        for i in 0..enden.len() {
            if genommen[i] {
                continue;
            }
            genommen[i] = true;
            let li = enden[i].lage();
            let mut gruppe = vec![enden[i].clone()];
            for j in i + 1..enden.len() {
                if genommen[j] || enden[j].spline.id == enden[i].spline.id {
                    continue;
                }
                let lj = enden[j].lage();
                // Richtung durchgehend: Ende an Anfang gleich, Ende an Ende (oder Anfang an Anfang) entgegen
                let soll = if enden[i].am_ende == enden[j].am_ende { 180.0 } else { 0.0 };
                if (lj.pos - li.pos).truncate().length() < FANG_ENDEN && norm180(lj.richtung - li.richtung - soll).abs() < 3.0 {
                    genommen[j] = true;
                    gruppe.push(enden[j].clone());
                }
            }
            if (li.pos.truncate() - p).length() > umkreis || eigene.iter().any(|q| (*q - li.pos.truncate()).length() < 1.5) {
                continue;
            }
            // ein einzelnes Ende, dessen Spuren trotzdem weiterfuehren, haengt an einem Objekt (Kreuzung): nicht ziehen
            if gruppe.len() == 1 {
                let e = &gruppe[0];
                let frei = *self.frei.entry((e.spline.id, e.am_ende)).or_insert_with(|| v.spline_end_free(e.spline.id, e.am_ende).unwrap_or(true));
                if !frei {
                    continue;
                }
            }
            griffe.push(Griff::Karte { enden: gruppe, pos: li.pos });
        }
        // Kreuzungsobjekt unter der Maus
        if let Some(k) = ae.kreuzungsobjekt_bei(v, p).filter(|k| k.arme.len() >= 3) {
            if !eigene.iter().any(|q| (*q - k.pos.truncate()).length() < 3.0) {
                let enden = k.arme.iter().filter_map(|a| ende_bei(ae, a.pos, Some(a.richtung))).collect();
                griffe.push(Griff::Kreuzung { k, enden });
            }
        }
        // Stelle auf dem Spline unter der Maus
        if let Some(id) = ae.suchen(v, p) {
            if let Some(sp) = ae.spline(id).cloned() {
                let x = naechste_stelle(&sp, p);
                if x > 2.0 && x < sp.kurve.length - 2.0 {
                    griffe.push(Griff::Mitte { pos: sp.kurve.point_at(x), spline: sp, s: x });
                }
            }
        }
        // unter der Maus: Knoten und Enden im Fang zuerst, dann die Kreuzung (bis 8 m um ihre Mitte), dann die Strasse
        let rang = |g: &Griff| -> Option<f64> {
            let d = (g.pos().truncate() - p).length();
            match g {
                Griff::Netz { .. } | Griff::Karte { .. } => (d <= fang).then_some(d),
                Griff::Kreuzung { .. } => (d <= fang.max(8.0)).then_some(100.0 + d),
                Griff::Mitte { .. } => None,
            }
        };
        self.unter_maus = griffe.iter().enumerate().filter_map(|(i, g)| rang(g).map(|r| (r, i))).min_by(|a, b| a.0.total_cmp(&b.0)).map(|x| x.1);
        self.mitte = griffe.iter().position(|g| matches!(g, Griff::Mitte { .. }));
        self.griffe = griffe;
    }

    /// Griff unter der Maus nehmen (mit `neu`: sonst die Stelle auf der Strasse, dort entsteht ein Knoten) -> ob ein
    /// Zug beginnt
    pub fn greifen(&mut self, boden: DVec3, neu: bool) -> bool {
        let i = self.unter_maus.or(if neu { self.mitte } else { None });
        let Some(g) = i.and_then(|i| self.griffe.get(i)).cloned() else { return false };
        self.dreh = 0.0;
        self.hoehe = 0.0;
        self.plan = None;
        self.zug = Some(Zug { griff: g, greif: boden.truncate(), letzter: boden });
        true
    }

    /// Maus beim Ziehen (None: mit dem letzten Bodenpunkt neu rechnen, nach Drehen/Hoehe)
    pub fn ziehen(&mut self, v: &Viewer, ae: &mut Aendern, s: &Strassenbau, boden: Option<DVec3>) {
        let Some(zug) = self.zug.as_mut() else { return };
        if let Some(b) = boden {
            zug.letzter = b;
        }
        let alt = zug.griff.pos();
        let xy = alt.truncate() + (zug.letzter.truncate() - zug.greif);
        // die Strasse folgt dem Gelaende (Hoehenunterschied zwischen alter und neuer Stelle), Kreuzungsobjekte nicht
        let gelaende = match (&zug.griff, v.terrain_height(xy.x, xy.y), v.terrain_height(alt.x, alt.y)) {
            (Griff::Kreuzung { .. }, _, _) => 0.0,
            (_, Some(a), Some(b)) => a - b,
            _ => 0.0,
        };
        let ziel = xy.extend(alt.z + gelaende + self.hoehe);
        let griff = zug.griff.clone();
        self.plan = Some(planen(v, ae, s, &griff, ziel, self.dreh));
    }

    /// Maus losgelassen: Zug anwenden -> Meldung (None: nichts geschehen)
    pub fn loslassen(&mut self, v: &mut Viewer, ae: &mut Aendern, s: &mut Strassenbau) -> Option<String> {
        let zug = self.zug.take()?;
        let plan = self.plan.take();
        let (dreh, hoehe) = (self.dreh, self.hoehe);
        self.dreh = 0.0;
        self.hoehe = 0.0;
        let plan = plan?;
        if (plan.ziel - zug.griff.pos()).length() < 0.05 && dreh == 0.0 && hoehe == 0.0 {
            return None;
        }
        if let Some(f) = &plan.fehler {
            return Some(format!("nicht verschoben: {f}"));
        }
        crate::protokoll::aktion(&format!("Knoten ziehen: {} nach {:.1} {:.1}", zug.griff.text(), plan.ziel.x, plan.ziel.y));
        let erg = (|| -> Result<String> {
            let n: usize = plan.umlegungen.iter().map(|u| 1 + u.weitere.len()).sum();
            if !plan.umlegungen.is_empty() || !plan.objekte.is_empty() {
                ae.umlegen(v, &plan.umlegungen, &plan.objekte)?;
            }
            let d = (plan.ziel - zug.griff.pos()).truncate().length();
            Ok(match plan.netz {
                Some(netz) => {
                    s.netz_setzen(v, ae, netz, usize::from(!plan.umlegungen.is_empty() || !plan.objekte.is_empty()));
                    format!("Knoten um {d:.1} m verschoben{}", if n > 0 { format!(", {n} angeschlossene Strasse(n) der Karte angepasst") } else { String::new() })
                }
                None => format!("{} um {d:.1} m verschoben, {n} Spline(s) neu gelegt{}", zug.griff.text().split(' ').next().unwrap_or(""),
                                plan.warnung.as_ref().map(|w| format!(" - {w}")).unwrap_or_default()),
            })
        })();
        crate::protokoll::aktion("");
        self.griffe.clear();
        self.unter_maus = None;
        Some(match erg {
            Ok(m) => {
                log::info!("{m}");
                m
            }
            Err(e) => {
                log::warn!("Knoten ziehen fehlgeschlagen: {e:#}");
                format!("Knoten ziehen fehlgeschlagen: {e:#}")
            }
        })
    }
}

/// Plan fuer den Griff an der Stelle `ziel`
pub fn planen(v: &Viewer, ae: &mut Aendern, s: &Strassenbau, griff: &Griff, ziel: DVec3, dreh: f64) -> Plan {
    let alt = griff.pos();
    let mut plan = Plan { ziel, ..Default::default() };
    let mittel = |w: &[f64]| if w.is_empty() { 0.0 } else { w.iter().sum::<f64>() / w.len() as f64 };
    match griff {
        Griff::Karte { enden, .. } => {
            let enden = verlaengert(ae, enden);
            let enden = &enden;
            // die Richtung am Knoten dreht sich mit den Sehnen zu den festen Enden (im Mittel)
            let d: Vec<f64> = enden.iter().map(|e| norm180(e.sehne(ziel.truncate()) - e.sehne(alt.truncate()))).collect();
            let delta = mittel(&d) + dreh;
            plan.umlegungen = enden.iter().map(|e| e.umlegung(ziel, delta)).collect();
        }
        Griff::Mitte { spline, s: x, .. } => {
            let (a, b) = (Lage::anfang(spline), Lage::ende(spline));
            let d1 = norm180(richtung(a.pos.truncate(), ziel.truncate()) - richtung(a.pos.truncate(), alt.truncate()));
            let d2 = norm180(richtung(ziel.truncate(), b.pos.truncate()) - richtung(alt.truncate(), b.pos.truncate()));
            let mut m = Lage::bei(spline, *x);
            m.pos = ziel;
            m.richtung = (m.richtung + (d1 + d2) / 2.0 + dreh).rem_euclid(360.0);
            plan.umlegungen = vec![Umlegung { alt: spline.clone(), weitere: vec![], punkte: vec![a, m, b] }];
        }
        Griff::Kreuzung { k, enden } => {
            let enden = verlaengert(ae, enden);
            // das Objekt wird verschoben (nicht gedreht): die Enden der Strassen gehen mit, in derselben Richtung
            let off = ziel - alt;
            plan.objekte = vec![(k.kachel, k.objekt, off.truncate())];
            plan.umlegungen = enden.iter().map(|e| e.umlegung(e.lage().pos + off, 0.0)).collect();
        }
        Griff::Netz { knoten, .. } => {
            let mut netz = s.netz.clone();
            let Some(kn) = netz.knoten(*knoten).cloned() else {
                plan.fehler = Some("Knoten gibt es nicht mehr".into());
                return plan;
            };
            let off = ziel - kn.pos;
            let an: Vec<netz::Kante> = netz.an(*knoten).into_iter().cloned().collect();
            let anderer = |e: &netz::Kante| netz.knoten(if e.a == *knoten { e.b } else { e.a }).map(|x| x.pos.truncate()).unwrap_or(alt.truncate());
            let d: Vec<f64> = an.iter().map(|e| {
                let o = anderer(e);
                norm180(richtung(ziel.truncate(), o) - richtung(kn.pos.truncate(), o))
            }).collect();
            // am Anschluss an eine Strasse der Karte bleibt die Richtung (die vorhandene Strasse wird mitgezogen);
            // an einem Verbindungsknoten drehen beide Seiten gleich (glatt), sonst jede Kante mit ihrer Sehne
            let drehung: Vec<f64> = if kn.anschluss.is_some() {
                vec![0.0; an.len()]
            } else if an.len() == 2 && kn.kartenarme.is_empty() {
                vec![mittel(&d) + dreh; 2]
            } else if an.len() == 1 {
                vec![d[0] + dreh]
            } else {
                d.clone()
            };
            for (e, dd) in an.iter().zip(&drehung) {
                if let Some(x) = netz.kanten.iter_mut().find(|x| x.id == e.id) {
                    if x.a == *knoten {
                        x.ha = (x.ha + dd).rem_euclid(360.0);
                    }
                    if x.b == *knoten {
                        x.hb = (x.hb + dd).rem_euclid(360.0);
                    }
                }
            }
            // Strassen der Karte an diesem Knoten (aufgeschnittene an einer Kreuzung, oder der Anschluss) folgen
            // (Lage, Richtung der Strasse vom Knoten weg; der Anschluss zeigt von der Strasse weg)
            let mut karten_enden: Vec<(DVec3, f64)> = kn.kartenarme.iter().map(|a| (a.pos, a.richtung)).collect();
            if let Some((h, _)) = kn.anschluss {
                karten_enden.push((kn.pos, h + 180.0));
            }
            let mut gefunden = Vec::new();
            for (q, h) in karten_enden {
                match ende_bei(ae, q, Some(h)) {
                    Some(e) => gefunden.push(e),
                    None if kn.kartenarme.is_empty() => {
                        // Anschluss an einen offenen Arm eines Kreuzungsobjekts (dort endet kein Spline)
                        plan.fehler = Some("der Knoten haengt an einer Kreuzung der Karte - die Kreuzung verschieben".into());
                        return plan;
                    }
                    None => {
                        plan.fehler = Some("angeschlossene Strasse der Karte nicht gefunden (Kachel wird noch geladen?)".into());
                        return plan;
                    }
                }
            }
            for e in verlaengert(ae, &gefunden) {
                plan.umlegungen.push(e.umlegung(e.lage().pos + off, 0.0));
            }
            if let Some(x) = netz.knoten.iter_mut().find(|x| x.id == *knoten) {
                x.pos = ziel;
                for a in &mut x.kartenarme {
                    a.pos += off;
                }
            }
            // Vorschau: die Kanten am Knoten
            for e in netz.an(*knoten) {
                let el = netz.elemente(e);
                if el.is_empty() || el.iter().any(|x| !x.stueck.laenge.is_finite()) {
                    plan.fehler = Some("die Strasse laesst sich so nicht legen (zu kurz oder verschlungen)".into());
                    continue;
                }
                let l: f64 = el.iter().map(|x| x.stueck.laenge).sum();
                let sehne = (netz.knoten(e.b).map(|x| x.pos).unwrap_or_default() - netz.knoten(e.a).map(|x| x.pos).unwrap_or_default()).truncate().length();
                if l > 2.5 * sehne + 40.0 {
                    plan.fehler = Some("der Verlauf liefe in einer Schleife".into());
                }
                if let Some(r) = el.iter().filter(|x| x.stueck.radius != 0.0).map(|x| x.stueck.radius.abs()).min_by(|a, b| a.total_cmp(b)) {
                    if r < ENGER_RADIUS {
                        plan.warnung = Some(format!("enge Kurve (Radius {r:.0} m)"));
                    }
                }
                let w = netz.breiten.get(&e.sli).copied().unwrap_or(5.0);
                plan.linien.push((netz::abtasten(&el, 2.0).into_iter().map(|(p, _)| p).zip(richtungen(&el)).collect(), w));
            }
            plan.netz = Some(netz);
        }
    }
    for u in &plan.umlegungen {
        match u.elemente() {
            Ok(el) => {
                let el: Vec<Element> = el.into_iter().map(|x| x.0).collect();
                if let Some(r) = el.iter().filter(|x| x.stueck.radius != 0.0).map(|x| x.stueck.radius.abs()).min_by(|a, b| a.total_cmp(b)) {
                    if r < ENGER_RADIUS {
                        plan.warnung = Some(format!("enge Kurve (Radius {r:.0} m)"));
                    }
                }
                let (l, r) = ae.breite(v, &u.alt.sli);
                let halb = l.max(r) as f64;
                if let Some(rmin) = el.iter().filter(|x| x.stueck.radius != 0.0).map(|x| x.stueck.radius.abs()).min_by(|a, b| a.total_cmp(b)) {
                    if rmin < halb + 1.0 {
                        plan.fehler = Some(format!("Kurve zu eng fuer den Querschnitt (Radius {rmin:.1} m) - weiter vom Ende weg ziehen"));
                    }
                }
                plan.linien.push((netz::abtasten(&el, 2.0).into_iter().map(|(p, _)| p).zip(richtungen(&el)).collect(), l.max(r) as f64));
            }
            Err(e) => {
                plan.fehler = Some(format!("{e:#}"));
            }
        }
    }
    plan
}

/// Enden mit ihren Verlaengerungen (jede Kette nur einmal)
fn verlaengert(ae: &Aendern, enden: &[Ende]) -> Vec<Ende> {
    let mut vergeben: HashSet<i64> = enden.iter().map(|e| e.spline.id).collect();
    enden.iter().map(|e| {
        let mut e = e.clone();
        e.verlaengern(ae, &vergeben);
        vergeben.extend(e.weiter.iter().map(|x| x.id));
        e
    }).collect()
}

/// Richtungen passend zu netz::abtasten(el, 2.0)
fn richtungen(el: &[Element]) -> Vec<f64> {
    let mut out = Vec::new();
    for x in el {
        let n = (x.stueck.laenge / 2.0).ceil().max(1.0) as usize;
        for i in 0..n {
            out.push(x.stueck.bei(x.stueck.laenge * i as f64 / n as f64).1);
        }
    }
    if let Some(x) = el.last() {
        out.push(x.stueck.ende().1);
    }
    out
}

/// Spline-Ende der Karte genau bei q (bis FANG_ENDEN); mit `weg`: der Spline fuehrt von q in diese Richtung (dort
/// beginnt oft auch der naechste Spline der Kette - der zaehlt dann nicht)
fn ende_bei(ae: &Aendern, q: DVec3, weg: Option<f64>) -> Option<Ende> {
    let mut best: Option<(f64, Ende)> = None;
    for sp in ae.kacheln.values().flatten() {
        for am_ende in [false, true] {
            let k = &sp.kurve;
            let (p, h) = if am_ende { (k.end_point(), k.heading_at(k.length) + 180.0) } else { (k.start, k.heading_deg) };
            if weg.is_some_and(|w| norm180(h - w).abs() > 20.0) {
                continue;
            }
            let d = (p - q).truncate().length();
            if d < FANG_ENDEN && best.as_ref().map(|b| d < b.0).unwrap_or(true) {
                best = Some((d, Ende::neu(sp.clone(), am_ende)));
            }
        }
    }
    best.map(|b| b.1)
}

/// Stelle (Meter ab Anfang) auf dem Spline, die p am naechsten liegt
fn naechste_stelle(sp: &KartenSpline, p: DVec2) -> f64 {
    let l = sp.kurve.length;
    let n = (l / 0.5).ceil().max(1.0) as usize;
    (0..=n).map(|i| l * i as f64 / n as f64)
        .min_by(|a, b| (sp.kurve.point_at(*a).truncate() - p).length().total_cmp(&(sp.kurve.point_at(*b).truncate() - p).length()))
        .unwrap_or(0.0)
}

/// hat der Querschnitt Fahrspuren (eine Strasse)?
fn ist_strasse(v: &Viewer, ae: &mut Aendern, sli: &str) -> bool {
    if let Some(x) = ae.mit_spuren.get(sli) {
        return *x;
    }
    let x = v.spline_lanes(sli).is_some_and(|(l, _)| l.iter().any(|x| x.0 == 0));
    ae.mit_spuren.insert(sli.to_string(), x);
    x
}

/// [spline_h]-Eintrag fuer ein Element (Kachel k), mit Querschnitt und Spiegelung von `alt`
#[allow(clippy::too_many_arguments)]
fn datensatz(alt: &KartenSpline, e: &Element, quer: (f64, f64), schraeg: (f64, f64), id: i64, prev: i64, next: i64, k: (i32, i32), tex: f64) -> Vec<String> {
    let ts = omsi_map::tile_size();
    let p = e.stueck.start;
    let mut z = vec![
        "[spline_h]".to_string(), "0".into(), alt.sli.clone(), id.to_string(), prev.to_string(), next.to_string(),
        zahl(p.x - k.0 as f64 * ts), zahl(e.z), zahl(p.y - k.1 as f64 * ts),
        zahl(e.stueck.richtung.rem_euclid(360.0)), zahl(e.stueck.laenge), zahl(e.stueck.radius), zahl(e.stg_a), zahl(e.stg_e), zahl(e.dh),
        zahl(quer.0), zahl(quer.1), zahl(schraeg.0), zahl(schraeg.1), zahl(tex),
    ];
    if alt.gespiegelt {
        z.push("mirror".into());
    }
    z
}

impl Aendern {
    /// Splines der Karte neu legen und Objekte verschieben (ein Rueckgaengig-Schritt). Das erste Stueck eines
    /// Splines behaelt seine ID (Vorgaenger zeigen weiter darauf), weitere bekommen neue; Nachfolger, deren prev auf
    /// ihn zeigte, zeigen danach auf sein letztes Stueck. -> Anzahl neuer Spline-Eintraege
    pub fn umlegen(&mut self, v: &mut Viewer, umlegungen: &[Umlegung], objekte: &[((i32, i32), i64, DVec2)]) -> Result<usize> {
        let ts = omsi_map::tile_size();
        let kachel_von = |p: DVec2| ((p.x / ts).floor() as i32, (p.y / ts).floor() as i32);
        let alte: HashSet<i64> = umlegungen.iter().flat_map(|u| u.kette().into_iter().map(|x| x.id)).collect();
        if alte.len() != umlegungen.iter().map(|u| 1 + u.weitere.len()).sum::<usize>() {
            bail!("ein Spline wuerde zweimal neu gelegt");
        }
        let mut naechste = v.next_object_id();
        // neue Stuecke je Spline: (Kachel, Element, Querneigung, ID)
        let mut stuecke: Vec<Vec<((i32, i32), Element, (f64, f64), i64)>> = Vec::new();
        let mut letzte: HashMap<i64, i64> = HashMap::new();
        for u in umlegungen {
            let el = u.elemente().with_context(|| format!("Spline {}", u.alt.id))?;
            // IDs: das erste Stueck behaelt die des ersten Splines, weitere nehmen die der uebrigen, dann neue
            let mut frei: std::collections::VecDeque<i64> = u.weitere.iter().map(|x| x.id).collect();
            let st: Vec<_> = el.into_iter().enumerate().map(|(i, (e, qa, qb))| {
                let id = if i == 0 {
                    u.alt.id
                } else if let Some(x) = frei.pop_front() {
                    x
                } else {
                    naechste += 1;
                    naechste - 1
                };
                (kachel_von(e.stueck.start), e, (qa, qb), id)
            }).collect();
            let letzter = u.weitere.last().unwrap_or(&u.alt).id;
            letzte.insert(letzter, st.last().map(|x| x.3).unwrap_or(u.alt.id));
            stuecke.push(st);
        }
        // Eintraege: an Ort und Stelle ersetzen (erstes Stueck in der Kachel des alten Splines) oder entfernen und
        // in der Kachel des Stuecks anhaengen
        let mut ersetzen: BTreeMap<(i32, i32), Vec<(i64, Option<Vec<String>>)>> = BTreeMap::new();
        let mut anhaengen: BTreeMap<(i32, i32), Vec<Vec<String>>> = BTreeMap::new();
        let mut neu = 0;
        for (u, st) in umlegungen.iter().zip(&stuecke) {
            let alt = &u.alt;
            let kette = u.kette();
            let letzter = *kette.last().unwrap();
            let mut tex = alt.kurve.tex_offset;
            let n = st.len();
            let mut an_ort: HashSet<i64> = HashSet::new();
            for (i, (k, e, quer, id)) in st.iter().enumerate() {
                let prev = if i == 0 { letzte.get(&alt.prev).copied().unwrap_or(alt.prev) } else { st[i - 1].3 };
                let next = if i + 1 == n { letzter.next } else { st[i + 1].3 };
                let schraeg = (if i == 0 { alt.kurve.skew_start } else { 0.0 }, if i + 1 == n { letzter.kurve.skew_end } else { 0.0 });
                let z = datensatz(alt, e, *quer, schraeg, *id, prev, next, *k, tex);
                tex += e.stueck.laenge;
                // ein Stueck mit der ID eines alten Splines in dessen Kachel ersetzt ihn an Ort und Stelle
                match kette.iter().find(|x| x.id == *id) {
                    Some(x) if x.kachel == *k => {
                        ersetzen.entry(x.kachel).or_default().push((x.id, Some(z)));
                        an_ort.insert(x.id);
                    }
                    _ => anhaengen.entry(*k).or_default().push(z),
                }
            }
            for x in &kette {
                if !an_ort.contains(&x.id) {
                    ersetzen.entry(x.kachel).or_default().push((x.id, None));
                }
            }
            neu += n.saturating_sub(kette.len());
        }
        // Nachfolger ausserhalb, deren prev auf einen neu gelegten Spline zeigte
        let mut prev_neu: BTreeMap<(i32, i32), Vec<(i64, i64)>> = BTreeMap::new();
        for sp in self.kacheln.values().flatten() {
            if alte.contains(&sp.id) {
                continue;
            }
            if let Some(l) = letzte.get(&sp.prev).filter(|l| **l != sp.prev) {
                prev_neu.entry(sp.kachel).or_default().push((sp.id, *l));
            }
        }
        let mut verschieben: BTreeMap<(i32, i32), Vec<(i64, DVec2)>> = BTreeMap::new();
        for (k, id, d) in objekte {
            verschieben.entry(*k).or_default().push((*id, *d));
        }
        let mut kacheln: Vec<(i32, i32)> = ersetzen.keys().chain(anhaengen.keys()).chain(prev_neu.keys()).chain(verschieben.keys()).copied().collect();
        kacheln.sort();
        kacheln.dedup();
        log::info!("Knoten ziehen: {} Spline(s) neu gelegt ({neu} Stuecke mehr), {} Objekt(e) verschoben, Kacheln {kacheln:?}",
                   alte.len(), objekte.len());
        self.kacheln_aendern(v, &kacheln, |k, zeilen| {
            if !zeilen.iter().take(20).position(|l| l.trim().eq_ignore_ascii_case("[version]"))
                .and_then(|i| zeilen.get(i + 1)).and_then(|l| l.trim().parse::<i32>().ok()).is_some_and(|x| x >= 11) {
                bail!("Kachel {} {}: altes Kachelformat (vor Version 11) wird nicht umgeschrieben", k.0, k.1);
            }
            for (id, d) in verschieben.get(&k).into_iter().flatten() {
                objekt_verschieben(zeilen, *id, *d)?;
            }
            for (id, p) in prev_neu.get(&k).into_iter().flatten() {
                let i = eintrag_finden(zeilen, *id).context("Nachbar-Spline nicht in seiner Kachel")?;
                zeilen[i + 4] = p.to_string();
            }
            for (id, ersatz) in ersetzen.get(&k).into_iter().flatten() {
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
        Ok(neu)
    }
}

/// Objekt `id` der Kachel um d verschieben, mit allem, was ueber [varparent] an ihm haengt (Ampeln), und den Masten
/// genau an deren Stelle; [attachObj] liegt relativ zum Elternobjekt und geht von selbst mit
fn objekt_verschieben(zeilen: &mut [String], id: i64, d: DVec2) -> Result<usize> {
    // [object]-Eintraege: (Zeile, ID, Eltern ueber varparent, Lage)
    let mut eintraege: Vec<(usize, i64, Vec<i64>, Option<(f64, f64)>)> = Vec::new();
    let mut i = 0;
    while i < zeilen.len() {
        let w = zeilen[i].trim().to_ascii_lowercase();
        if w == "[object]" || w == "[attachobj]" {
            let mut j = i + 1;
            while j < zeilen.len() && !ist_eintrag(&zeilen[j]) {
                j += 1;
            }
            if w == "[object]" {
                let oid = zeilen.get(i + 3).and_then(|z| z.trim().parse::<i64>().ok()).unwrap_or(-1);
                let eltern = (i..j).filter(|&k| zeilen[k].trim().eq_ignore_ascii_case("[varparent]"))
                    .filter_map(|k| zeilen.get(k + 1).and_then(|z| z.trim().parse::<i64>().ok())).collect();
                let x = zeilen.get(i + 4).and_then(|z| z.trim().parse::<f64>().ok());
                let y = zeilen.get(i + 5).and_then(|z| z.trim().parse::<f64>().ok());
                eintraege.push((i, oid, eltern, x.zip(y)));
            }
            i = j;
        } else {
            i += 1;
        }
    }
    if !eintraege.iter().any(|e| e.1 == id) {
        bail!("Objekt {id} nicht in seiner Kachel");
    }
    let mut mit: HashSet<i64> = [id].into();
    loop {
        let n = mit.len();
        for e in &eintraege {
            if e.2.iter().any(|x| mit.contains(x)) {
                mit.insert(e.1);
            }
        }
        if mit.len() == n {
            break;
        }
    }
    let ampeln: Vec<(f64, f64)> = eintraege.iter().filter(|e| e.1 != id && mit.contains(&e.1)).filter_map(|e| e.3).collect();
    for e in &eintraege {
        if let Some((x, y)) = e.3 {
            if ampeln.iter().any(|(ax, ay)| (ax - x).hypot(ay - y) < 0.6) {
                mit.insert(e.1);
            }
        }
    }
    let mut n = 0;
    for e in &eintraege {
        if let (true, Some((x, y))) = (mit.contains(&e.1), e.3) {
            zeilen[e.0 + 4] = zahl(x + d.x);
            zeilen[e.0 + 5] = zahl(y + d.y);
            n += 1;
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bearbeiten::tests::{grundorf, sperre};

    fn gerade(l: f64) -> KartenSpline {
        KartenSpline {
            id: 1, kachel: (0, 0), sli: "test.sli".into(), gespiegelt: false, prev: 0, next: 0, gesperrt: vec![],
            kurve: omsi_geometry::SplineCurve {
                start: DVec3::new(0.0, 0.0, 10.0), heading_deg: 0.0, length: l, radius: 0.0, grad_start: 2.0, grad_end: 2.0,
                delta_h: Some(l * 0.02), cant_start: 0.0, cant_end: 0.0, skew_start: 0.0, skew_end: 0.0, tex_offset: 0.0, seed: 0,
                half_cant_width: 0.0,
            },
        }
    }

    /// Mitte einer Geraden zur Seite gezogen: die neuen Elemente schliessen an den Enden genau wie vorher an (Lage,
    /// Richtung, Hoehe, Steigung) und laufen lueckenlos und knickfrei durch den neuen Knoten
    #[test]
    fn umlegung_schliesst_an_den_enden_an() {
        let sp = gerade(100.0);
        let mut m = Lage::bei(&sp, 50.0);
        m.pos.x += 8.0;
        let u = Umlegung { alt: sp.clone(), weitere: vec![], punkte: vec![Lage::anfang(&sp), m, Lage::ende(&sp)] };
        let el: Vec<Element> = u.elemente().unwrap().into_iter().map(|x| x.0).collect();
        assert!(el.len() >= 2);
        let k: Vec<omsi_geometry::SplineCurve> = el.iter().map(|e| e.kurve(0, 0.0)).collect();
        let (a, b) = (k.first().unwrap(), k.last().unwrap());
        assert!((a.start - sp.kurve.start).length() < 1e-6 && norm180(a.heading_deg).abs() < 1e-6);
        assert!((a.slope_at(0.0) - 0.02).abs() < 1e-6, "Steigung am Anfang {}", a.slope_at(0.0));
        assert!((b.end_point() - sp.kurve.end_point()).length() < 1e-6, "{:?} {:?}", b.end_point(), sp.kurve.end_point());
        assert!(norm180(b.heading_at(b.length)).abs() < 1e-6 && (b.slope_at(b.length) - 0.02).abs() < 1e-6);
        for w in k.windows(2) {
            assert!((w[0].end_point() - w[1].start).length() < 1e-6);
            assert!(norm180(w[0].heading_at(w[0].length) - w[1].heading_deg).abs() < 1e-6);
            assert!((w[0].slope_at(w[0].length) - w[1].slope_at(0.0)).abs() < 1e-6);
        }
        assert!(k.iter().any(|x| (x.start.truncate() - DVec2::new(8.0, 50.0)).length() < 1e-6), "Knoten bei 8/50");
        // hinter das feste Ende gezogen: abgelehnt statt Schleife
        let mut m = Lage::bei(&sp, 50.0);
        m.pos.y = -30.0;
        assert!(Umlegung { alt: sp.clone(), weitere: vec![], punkte: vec![Lage::anfang(&sp), m, Lage::ende(&sp)] }.elemente().is_err());
    }

    fn boden(v: &Viewer, p: DVec2) -> DVec3 {
        p.extend(v.terrain_height(p.x, p.y).unwrap_or(0.0))
    }

    /// Verbindung zweier Splines und eine Stelle mitten auf einem Spline in Grundorf ziehen: die festen Enden bleiben
    /// laut Spurnetz so verbunden wie vorher, am neuen Knoten geht es weiter; Rueckgaengig stellt alles wieder her
    #[test]
    #[ignore]
    fn knoten_ziehen_in_grundorf() {
        let _sperre = sperre();
        let mut v = grundorf();
        v.tiles_around(DVec3::new(150.0, 150.0, 0.0), 8).unwrap();
        let mut a = Aendern::neu(&v);
        a.aktualisieren(&v);
        let s = Strassenbau::default();
        let mut w = Knotenwerkzeug::default();
        // Verbindung zweier Strassen-Splines (beide ueber 25 m, verknuepft)
        let alle: Vec<KartenSpline> = a.kacheln.values().flatten().cloned().collect();
        let mut paar = None;
        for x in &alle {
            if x.kurve.length <= 25.0 || !ist_strasse(&v, &mut a, &x.sli) || v.spline_end_free(x.id, false) != Some(false) {
                continue;
            }
            let Some(n) = alle.iter().find(|n| n.id == x.next && n.prev == x.id && n.kurve.length > 25.0 && (n.kurve.start - x.kurve.end_point()).length() < 0.05) else { continue };
            if v.spline_end_free(n.id, true) == Some(false) {
                paar = Some((x.clone(), n.clone()));
                break;
            }
        }
        let (sa, sb) = paar.expect("keine verknuepften Strassen-Splines");
        let knoten = sa.kurve.end_point();
        println!("Knoten zwischen {} und {} bei {:.1} {:.1}", sa.id, sb.id, knoten.x, knoten.y);
        w.suchen(&v, &mut a, &s, knoten, 1.0, 60.0);
        // bei jeder Mausbewegung: schnell genug fuer fluessiges Arbeiten (zweiter Aufruf, Caches gefuellt)
        let t0 = std::time::Instant::now();
        w.suchen(&v, &mut a, &s, knoten, 1.0, 400.0);
        let dauer = t0.elapsed().as_secs_f64() * 1000.0;
        println!("Griffe suchen (400 m): {dauer:.1} ms, {} Griffe", w.griffe.len());
        assert!(dauer < 30.0, "Griffe suchen dauert {dauer:.1} ms");
        w.suchen(&v, &mut a, &s, knoten, 1.0, 60.0);
        let g = w.unter_maus.map(|i| w.griffe[i].clone()).expect("kein Griff");
        assert!(matches!(&g, Griff::Karte { enden, .. } if enden.len() == 2), "{}", g.text());
        assert!(w.greifen(knoten, false));
        let quer = netz::rechts(sa.kurve.heading_at(sa.kurve.length));
        let ziel = boden(&v, knoten.truncate() + quer * 4.0);
        w.ziehen(&v, &mut a, &s, Some(ziel));
        let plan = w.plan.clone().unwrap();
        assert!(plan.fehler.is_none(), "{:?}", plan.fehler);
        let mut s2 = Strassenbau::default();
        let m = w.loslassen(&mut v, &mut a, &mut s2).unwrap();
        println!("{m}");
        assert!(m.contains("verschoben"), "{m}");
        a.aktualisieren(&v);
        // feste Enden: Lage gleich, im Spurnetz weiter verbunden
        let anfang = a.spline(sa.id).expect("erstes Stueck behaelt die ID");
        assert!((anfang.kurve.start - sa.kurve.start).length() < 0.01);
        assert_eq!(v.spline_end_free(sa.id, false), Some(false), "Anfang von {} nicht mehr verbunden", sa.id);
        let ende = ende_bei(&a, sb.kurve.end_point(), Some(sb.kurve.heading_at(sb.kurve.length) + 180.0)).expect("Ende der Kette");
        assert!(ende.am_ende);
        assert_eq!(v.spline_end_free(ende.spline.id, true), Some(false), "Ende der Kette nicht mehr verbunden");
        // am neuen Knoten: verbunden
        let neu = ende_bei(&a, plan.ziel, None).expect("Spline am neuen Knoten");
        assert_eq!(v.spline_end_free(neu.spline.id, neu.am_ende), Some(false), "am neuen Knoten nicht verbunden");
        // Mitte eines langen Splines ziehen: er wird geteilt, seine Enden bleiben verbunden
        let mut lange: Vec<KartenSpline> = alle.iter().filter(|x| x.kurve.length > 60.0 && x.id != sa.id && x.id != sb.id).cloned().collect();
        lange.sort_by(|x, y| y.kurve.length.total_cmp(&x.kurve.length));
        let lang = lange.into_iter().find(|x| ist_strasse(&v, &mut a, &x.sli) && v.spline_end_free(x.id, false) == Some(false)
            && v.spline_end_free(x.id, true) == Some(false)).expect("langer Spline");
        let mitte = lang.kurve.point_at(lang.kurve.length / 2.0);
        w.suchen(&v, &mut a, &s, mitte, 1.0, 60.0);
        assert!(w.unter_maus.is_none(), "ohne Umschalt kein neuer Knoten");
        let g = w.mitte.map(|i| w.griffe[i].clone()).expect("kein Griff auf dem Spline");
        assert!(matches!(&g, Griff::Mitte { spline, .. } if spline.id == lang.id), "{}", g.text());
        w.greifen(mitte, true);
        let ziel = boden(&v, mitte.truncate() + netz::rechts(lang.kurve.heading_at(lang.kurve.length / 2.0)) * 5.0);
        w.ziehen(&v, &mut a, &s, Some(ziel));
        let m = w.loslassen(&mut v, &mut a, &mut s2).unwrap();
        println!("{m}");
        a.aktualisieren(&v);
        assert_eq!(v.spline_end_free(lang.id, false), Some(false));
        let e = ende_bei(&a, lang.kurve.end_point(), Some(lang.kurve.heading_at(lang.kurve.length) + 180.0)).unwrap();
        assert!(e.am_ende);
        assert_ne!(e.spline.id, lang.id, "geteilt: das Ende gehoert zu einem neuen Stueck");
        assert_eq!(v.spline_end_free(e.spline.id, true), Some(false));
        // beide Zuege zurueck
        a.rueckgaengig(&mut v).unwrap();
        a.rueckgaengig(&mut v).unwrap();
        a.aktualisieren(&v);
        assert!((a.spline(sa.id).unwrap().kurve.end_point() - knoten).length() < 0.01);
        assert!((a.spline(lang.id).unwrap().kurve.length - lang.kurve.length).abs() < 0.01);
        assert_eq!(a.kopien("Grundorf").len(), 0);
    }

    /// vorhandene Kreuzung (Grundorf 414/215) verschieben: Objekt und Strassenenden gehen mit, die Strassen haengen
    /// danach wieder an ihren Pfaden
    #[test]
    #[ignore]
    fn kreuzung_verschieben() {
        let _sperre = sperre();
        let mut v = grundorf();
        v.tiles_around(DVec3::new(150.0, 150.0, 0.0), 8).unwrap();
        let mut a = Aendern::neu(&v);
        a.aktualisieren(&v);
        let mut s = Strassenbau::default();
        let mut w = Knotenwerkzeug::default();
        let k = a.kreuzungsobjekt_bei(&v, DVec2::new(414.0, 215.0)).expect("Kreuzung");
        w.suchen(&v, &mut a, &s, k.pos, 1.0, 60.0);
        let g = w.unter_maus.map(|i| w.griffe[i].clone()).expect("kein Griff");
        let Griff::Kreuzung { enden, .. } = &g else { panic!("{}", g.text()) };
        assert_eq!(enden.len(), 3, "jeder Arm hat seinen Spline");
        let bild = |v: &mut Viewer, name: &str| {
            if let Some(b) = std::env::var_os("OMSI_BILD") {
                let kam = crate::kamera::Kamera { ziel: k.pos, gier: 200.0, neigung: -60.0, abstand: 70.0, fov: 50.0 };
                let px = v.render_image(1280, 800, &kam.camera()).unwrap();
                let p = std::path::PathBuf::from(b).with_extension(format!("{name}.png"));
                image::save_buffer(p, &px, 1280, 800, image::ColorType::Rgba8).unwrap();
            }
        };
        bild(&mut v, "vorher");
        w.greifen(k.pos, false);
        let ziel = k.pos + DVec3::new(3.0, -2.0, 0.0);
        w.ziehen(&v, &mut a, &s, Some(ziel));
        assert!(w.plan.as_ref().unwrap().fehler.is_none(), "{:?}", w.plan.as_ref().unwrap().fehler);
        let m = w.loslassen(&mut v, &mut a, &mut s).unwrap();
        println!("{m}");
        a.aktualisieren(&v);
        bild(&mut v, "nachher");
        let k2 = a.kreuzungsobjekt_bei(&v, ziel.truncate()).expect("Kreuzung an der neuen Stelle");
        assert_eq!(k2.arme.len(), 3, "alle Strassen haengen wieder an der Kreuzung");
        assert!((k2.pos - k.pos - DVec3::new(3.0, -2.0, 0.0)).truncate().length() < 0.1, "{:?} -> {:?}", k.pos, k2.pos);
        a.rueckgaengig(&mut v).unwrap();
        a.aktualisieren(&v);
        assert_eq!(a.kreuzungsobjekt_bei(&v, k.pos.truncate()).map(|x| x.arme.len()), Some(3));
    }

    /// Knoten eigener Strassen ziehen: ein Verbindungsknoten bleibt glatt; ein Abzweig aus einer vorhandenen Strasse
    /// zieht deren aufgeschnittene Enden mit
    #[test]
    #[ignore]
    fn eigene_knoten_ziehen() {
        let _sperre = sperre();
        use crate::anschluss::Anschluesse;
        use crate::strasse::Modus;
        let mut v = grundorf();
        v.tiles_around(DVec3::new(150.0, 150.0, 0.0), 8).unwrap();
        let mut a = Aendern::neu(&v);
        a.aktualisieren(&v);
        let ans = Anschluesse::default();
        let mut s = Strassenbau::neu(Some("Splines\\Marcel\\str_2spur_10m_Grunewaldstr.sli".into()), Modus::Gerade);
        // Abzweig mitten aus einer vorhandenen Strasse, 60 m geradeaus, dann eine Kurve
        let ab = crate::kreuzung::tests::abzweig_suchen(&v, &mut a);
        let h = (ab.richtung + 90.0).rem_euclid(360.0);
        let p1 = ab.pos.truncate() + netz::dir(h) * 60.0;
        let p2 = p1 + netz::dir(h) * 40.0 + netz::dir(ab.richtung) * 30.0;
        for (i, p) in [ab.pos.truncate(), p1, p2].into_iter().enumerate() {
            s.modus = if i < 2 { Modus::Gerade } else { Modus::Kurve };
            let g = boden(&v, p);
            s.maus(&mut v, g, 2.0, &ans, Some(&mut a));
            let m = s.klick(&mut v, g, 2.0, &ans, Some(&mut a));
            println!("Klick {i}: {m:?}");
        }
        s.beenden(&mut v);
        assert_eq!(s.netz.kanten.len(), 2, "{:?}", s.netz.kanten);
        let mitte = s.netz.knoten_bei(p1, 1.0).expect("Knoten bei p1");
        let mut w = Knotenwerkzeug::default();
        w.suchen(&v, &mut a, &s, boden(&v, p1), 1.0, 80.0);
        assert!(matches!(w.unter_maus.map(|i| &w.griffe[i]), Some(Griff::Netz { knoten, .. }) if *knoten == mitte));
        w.greifen(boden(&v, p1), false);
        w.ziehen(&v, &mut a, &s, Some(boden(&v, p1 + netz::dir(ab.richtung) * 6.0)));
        let m = w.loslassen(&mut v, &mut a, &mut s).unwrap();
        println!("{m}");
        let an: Vec<netz::Kante> = s.netz.an(mitte).into_iter().cloned().collect();
        assert_eq!(an.len(), 2);
        let rein = |e: &netz::Kante| if e.b == mitte { e.hb } else { e.ha + 180.0 };
        let raus = |e: &netz::Kante| if e.a == mitte { e.ha } else { e.hb + 180.0 };
        assert!(norm180(rein(&an[0]) - raus(&an[1])).abs() < 1e-6, "Knick am Verbindungsknoten: {} / {}", rein(&an[0]), raus(&an[1]));
        assert!(s.netz.kanten.iter().all(|e| !s.netz.elemente(e).is_empty()));
        // die Kreuzung am Abzweig 3 m die Strasse entlang ziehen: die aufgeschnittenen Enden gehen mit
        let kr = s.netz.knoten.iter().find(|k| k.kartenarme.len() == 2).expect("Kreuzung mit Kartenarmen").clone();
        w.suchen(&v, &mut a, &s, kr.pos, 1.0, 80.0);
        assert!(matches!(w.unter_maus.map(|i| &w.griffe[i]), Some(Griff::Netz { knoten, .. }) if *knoten == kr.id));
        w.greifen(kr.pos, false);
        let ziel = kr.pos + (netz::dir(ab.richtung) * 3.0).extend(0.0);
        w.ziehen(&v, &mut a, &s, Some(ziel));
        assert!(w.plan.as_ref().unwrap().fehler.is_none(), "{:?}", w.plan.as_ref().unwrap().fehler);
        assert_eq!(w.plan.as_ref().unwrap().umlegungen.len(), 2);
        let m = w.loslassen(&mut v, &mut a, &mut s).unwrap();
        println!("{m}");
        a.aktualisieren(&v);
        let kr2 = s.netz.knoten(kr.id).unwrap().clone();
        for ka in &kr2.kartenarme {
            let e = ende_bei(&a, ka.pos, Some(ka.richtung)).expect("Spline-Ende am verschobenen Kartenarm");
            let weg = if e.am_ende { e.lage().richtung + 180.0 } else { e.lage().richtung };
            assert!(norm180(weg - ka.richtung).abs() < 0.1, "{weg} / {}", ka.richtung);
        }
        assert_eq!(s.gesetzte_kreuzungen().len(), 1, "{:?}", s.kreuzung_fehler);
        // Rueckgaengig nimmt Netz und Kartenstrassen zusammen zurueck
        assert!(s.rueckgaengig(&mut v, Some(&mut a)));
        a.aktualisieren(&v);
        for ka in &kr.kartenarme {
            assert!(ende_bei(&a, ka.pos, Some(ka.richtung)).is_some(), "Kartenarm nach Rueckgaengig nicht zurueck");
        }
    }
}

#[cfg(test)]
mod nutzer_tests {
    use super::*;

    /// nur in der Sitzung: Griffe an einer Stelle der Karte des Nutzers auflisten und jeden 2 m ziehen (OMSI_KARTE,
    /// OMSI_X, OMSI_Y)
    #[test]
    #[ignore]
    fn griffe_der_nutzerkarte() {
        let _sperre = crate::bearbeiten::tests::sperre();
        let Ok(karte) = std::env::var("OMSI_KARTE") else { return };
        let x: f64 = std::env::var("OMSI_X").unwrap().parse().unwrap();
        let y: f64 = std::env::var("OMSI_Y").unwrap().parse().unwrap();
        let root = std::path::Path::new(crate::bearbeiten::tests::OMSI);
        let (mut v, _) = Viewer::open(&openomsi_game::viewer::instance(), None, root, &root.join("maps").join(&karte).join("global.cfg")).unwrap();
        v.tiles_around(DVec3::new(x, y, 0.0), 2).unwrap();
        let mut a = Aendern::neu(&v);
        a.aktualisieren(&v);
        let p = DVec2::new(x, y);
        let mut sp: Vec<KartenSpline> = a.kacheln.values().flatten().filter(|s| (s.kurve.start.truncate() - p).length() < 60.0 || (s.kurve.end_point().truncate() - p).length() < 60.0).cloned().collect();
        sp.sort_by_key(|s| s.id);
        for s in &sp {
            let e = s.kurve.end_point();
            println!("Spline {} {} L {:.2} R {:.1}  {:.2} {:.2} h {:.1} -> {:.2} {:.2} h {:.1}  prev {} next {} frei {:?}/{:?}", s.id, s.sli.rsplit('\\').next().unwrap(), s.kurve.length, s.kurve.radius,
                     s.kurve.start.x, s.kurve.start.y, s.kurve.heading_deg, e.x, e.y, s.kurve.heading_at(s.kurve.length).rem_euclid(360.0), s.prev, s.next,
                     v.spline_end_free(s.id, false), v.spline_end_free(s.id, true));
        }
        let s = Strassenbau::default();
        let mut w = Knotenwerkzeug::default();
        let g = DVec3::new(x, y, v.terrain_height(x, y).unwrap_or(0.0));
        w.suchen(&v, &mut a, &s, g, 1.0, 60.0);
        let griffe = w.griffe.clone();
        for gr in &griffe {
            let pos = gr.pos();
            let extra = match gr {
                Griff::Karte { enden, .. } => format!("{:?}", enden.iter().map(|e| (e.spline.id, e.am_ende)).collect::<Vec<_>>()),
                Griff::Kreuzung { k, enden } => format!("Objekt {} Arme {:?} Enden {:?}", k.objekt, k.arme.iter().map(|a| (a.pos.x.round(), a.pos.y.round(), a.richtung.round())).collect::<Vec<_>>(),
                                                        enden.iter().map(|e| (e.spline.id, e.am_ende)).collect::<Vec<_>>()),
                Griff::Mitte { spline, .. } => format!("Spline {}", spline.id),
                Griff::Netz { knoten, .. } => format!("Knoten {knoten}"),
            };
            let plan = planen(&v, &mut a, &s, gr, pos + DVec3::new(2.0, 0.0, 0.0), 0.0);
            println!("Griff {} bei {:.1} {:.1}: {extra} -> Fehler {:?} Warnung {:?}", gr.text(), pos.x, pos.y, plan.fehler, plan.warnung);
        }
        // die Kreuzung 2 m verschieben und zeigen (OMSI_BILD: vorher/nachher)
        if let Some(k) = griffe.iter().find(|g| matches!(g, Griff::Kreuzung { .. })) {
            let kam = crate::kamera::Kamera { ziel: k.pos(), gier: 200.0, neigung: -55.0, abstand: 55.0, fov: 50.0 };
            let bild = |v: &mut Viewer, name: &str| {
                if let Some(b) = std::env::var_os("OMSI_BILD") {
                    let px = v.render_image(1280, 800, &kam.camera()).unwrap();
                    image::save_buffer(std::path::PathBuf::from(b).with_extension(format!("{name}.png")), &px, 1280, 800, image::ColorType::Rgba8).unwrap();
                }
            };
            bild(&mut v, "vorher");
            let mut s2 = Strassenbau::default();
            w.suchen(&v, &mut a, &s2, k.pos(), 1.0, 60.0);
            assert!(w.greifen(k.pos(), false));
            w.ziehen(&v, &mut a, &s2, Some(k.pos() + DVec3::new(2.0, 0.0, 0.0)));
            println!("{:?}", w.loslassen(&mut v, &mut a, &mut s2));
            bild(&mut v, "nachher");
        }
        drop(a);
    }
}
