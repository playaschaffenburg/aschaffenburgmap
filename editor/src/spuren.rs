//! Werkzeug "Kreuzungen", Ansicht "Spuren" - wie Traffic Manager: President Edition in Cities: Skylines:
//! Spurpfeile (welche Richtungen eine Zufahrtsspur fahren darf) und Fahrstreifen-Verbinder (welche Abbiegespur von
//! welcher Spur zu welcher fuehrt).
//!
//! - Eigene Kreuzungen (Netz): omsigen meldet die gebauten Abbiegespuren; der Editor gibt eine Liste vor
//!   (Knoten::spurwahl) und omsigen baut genau diese - Spuren lassen sich wegnehmen und dazunehmen (auch Wenden).
//! - Kreuzungsobjekte der Karte: ihre Verbindungen kommen aus openOMSIs Spurnetz (von der Zufahrtsspur durch die
//!   Objektpfade bis zur Ausfahrt); abschalten heisst [rule] no_cars auf diesen Pfaden (ausser denen, die eine noch
//!   erlaubte Verbindung braucht), einschalten nimmt die Regeln weg. Neue Verbindungen gehen dort nicht.

use crate::netz::norm180;
use glam::{DVec2, DVec3};
use openomsi_game::viewer::{LaneKind, Viewer};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ziel {
    Netz(u32),
    Karte { kachel: (i32, i32), objekt: i64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Art {
    Links,
    Gerade,
    Rechts,
    Wenden,
}

impl Art {
    pub const ALLE: [Art; 4] = [Art::Links, Art::Gerade, Art::Rechts, Art::Wenden];

    /// aus der Aenderung der Fahrtrichtung (Grad, rechts positiv)
    pub fn aus(dlt: f64) -> Art {
        let d = norm180(dlt);
        if d.abs() > 150.0 {
            Art::Wenden
        } else if d.abs() < 35.0 {
            Art::Gerade
        } else if d > 0.0 {
            Art::Rechts
        } else {
            Art::Links
        }
    }

    pub fn zeichen(self) -> &'static str {
        match self {
            Art::Links => "\u{2190} links",
            Art::Gerade => "\u{2191} geradeaus",
            Art::Rechts => "\u{2192} rechts",
            Art::Wenden => "\u{21B6} wenden",
        }
    }
}

/// eine Zufahrtsspur: wo sie in die Kreuzung kommt, in welche Richtung
#[derive(Clone, Debug)]
pub struct Zufahrt {
    pub pos: DVec3,
    pub richtung: f64,
    arm: usize,
    spur: usize,
}

#[derive(Clone, Debug, PartialEq)]
enum Schluessel {
    /// [Arm rein, Spur, Arm raus, Spur]
    Netz([usize; 4]),
    /// Pfade des Objekts
    Karte(Vec<usize>),
}

#[derive(Clone, Debug)]
pub struct Verbindung {
    pub zufahrt: usize,
    pub art: Art,
    pub punkte: Vec<DVec3>,
    pub erlaubt: bool,
    schluessel: Schluessel,
}

/// was am Netz bzw. an der Karte zu aendern ist
#[derive(Clone, Debug, PartialEq)]
pub enum Aenderung {
    /// Kreuzung des Netzes mit genau diesen Abbiegespuren neu bauen
    Netz(u32, Vec<[usize; 4]>),
    /// Pfade eines Kreuzungsobjekts der Karte sperren / freigeben
    Karte { kachel: (i32, i32), objekt: i64, sperren: Vec<usize>, frei: Vec<usize> },
}

pub struct Ansicht {
    pub ziel: Ziel,
    pub zufahrten: Vec<Zufahrt>,
    pub verbindungen: Vec<Verbindung>,
    /// Netz: Armrichtungen (von der Kreuzung weg) und Anzahl abgehender Spuren je Arm
    richtungen: Vec<f64>,
    raus: Vec<usize>,
}

impl Ansicht {
    /// eine Kreuzung des eigenen Netzes
    pub fn netz(s: &crate::strasse::Strassenbau, k: u32) -> Option<Ansicht> {
        let (richtungen, o, hoehe) = s.spur_objekt(k)?;
        let mut zufahrten = Vec::new();
        for (arm, (rein, _)) in o.zufahrten.iter().enumerate() {
            for (spur, p) in rein.iter().enumerate() {
                let h = richtungen.get(arm).map(|h| (h + 180.0).rem_euclid(360.0)).unwrap_or(0.0);
                zufahrten.push(Zufahrt { pos: p.extend(hoehe), richtung: h, arm, spur });
            }
        }
        let verbindungen = o.verbindungen.iter().filter_map(|vb| {
            let z = zufahrten.iter().position(|z| (z.arm, z.spur) == vb.von)?;
            let art = if vb.von.0 == vb.nach.0 { Art::Wenden } else {
                Art::aus(richtungen.get(vb.nach.0)? - (richtungen.get(vb.von.0)? + 180.0))
            };
            Some(Verbindung { zufahrt: z, art, punkte: vb.punkte.iter().map(|p| p.extend(hoehe)).collect(), erlaubt: true,
                              schluessel: Schluessel::Netz([vb.von.0, vb.von.1, vb.nach.0, vb.nach.1]) })
        }).collect();
        Some(Ansicht { ziel: Ziel::Netz(k), zufahrten, verbindungen, raus: o.zufahrten.iter().map(|z| z.1.len()).collect(), richtungen })
    }

    /// ein Kreuzungsobjekt der Karte: Verbindungen von jeder Zufahrtsspur durch die Objektpfade bis zur Ausfahrt
    pub fn karte(v: &Viewer, kachel: (i32, i32), objekt: i64) -> Option<Ansicht> {
        let net = &v.lanes;
        let im_objekt = |i: usize| {
            let l = &net.lanes[i];
            l.kind == LaneKind::Street && l.key.is_some_and(|k| k.id == objekt && k.tile == kachel) && !l.name.to_ascii_lowercase().ends_with(".sli")
        };
        let eigene: Vec<usize> = (0..net.lanes.len()).filter(|&i| im_objekt(i)).collect();
        if eigene.is_empty() {
            return None;
        }
        // Anfaenge: Pfade, in die keine Pfade desselben Objekts fuehren
        let anfaenge: Vec<usize> = eigene.iter().copied().filter(|&i| !net.prev.get(i).is_some_and(|p| p.iter().any(|&q| im_objekt(q)))).collect();
        let mut ketten: Vec<Vec<usize>> = Vec::new();
        for a in anfaenge {
            // Tiefensuche bis zum Ende im Objekt (hoechstens 12 Glieder, keine Schleifen)
            let mut stapel = vec![vec![a]];
            while let Some(k) = stapel.pop() {
                let letzte = *k.last().unwrap();
                let weiter: Vec<usize> = net.lanes[letzte].next.iter().copied().filter(|&n| im_objekt(n) && !k.contains(&n)).collect();
                if weiter.is_empty() || k.len() >= 12 {
                    ketten.push(k);
                } else {
                    for n in weiter {
                        let mut k2 = k.clone();
                        k2.push(n);
                        stapel.push(k2);
                    }
                }
            }
        }
        let mut zufahrten: Vec<Zufahrt> = Vec::new();
        let mut verbindungen = Vec::new();
        for k in ketten {
            let punkte: Vec<DVec3> = k.iter().flat_map(|&i| net.lanes[i].points.iter().copied()).collect();
            if punkte.len() < 2 {
                continue;
            }
            let h0 = net.lanes[k[0]].headings.first().copied().unwrap_or(0.0) as f64;
            let h1 = net.lanes[*k.last().unwrap()].headings.last().copied().unwrap_or(0.0) as f64;
            let p0 = punkte[0];
            let z = match zufahrten.iter().position(|z| (z.pos - p0).truncate().length() < 0.5) {
                Some(z) => z,
                None => {
                    zufahrten.push(Zufahrt { pos: p0, richtung: h0, arm: 0, spur: 0 });
                    zufahrten.len() - 1
                }
            };
            let mut pfade: Vec<usize> = k.iter().filter_map(|&i| net.lanes[i].key.map(|x| x.path as usize)).collect();
            pfade.dedup();
            verbindungen.push(Verbindung { zufahrt: z, art: Art::aus(h1 - h0), punkte, erlaubt: !k.iter().any(|&i| net.lanes[i].no_cars),
                                           schluessel: Schluessel::Karte(pfade) });
        }
        Some(Ansicht { ziel: Ziel::Karte { kachel, objekt }, zufahrten, verbindungen, richtungen: vec![], raus: vec![] })
    }

    /// Richtungen, die eine Zufahrtsspur fahren darf
    pub fn pfeile(&self, z: usize) -> HashSet<Art> {
        self.verbindungen.iter().filter(|x| x.zufahrt == z && x.erlaubt).map(|x| x.art).collect()
    }

    /// Richtungen, die es an einer Zufahrtsspur ueberhaupt gibt (Karte: gebaute Pfade; Netz: alle Arme)
    pub fn moeglich(&self, z: usize) -> HashSet<Art> {
        match self.ziel {
            Ziel::Karte { .. } => self.verbindungen.iter().filter(|x| x.zufahrt == z).map(|x| x.art).collect(),
            Ziel::Netz(_) => {
                let a = self.zufahrten[z].arm;
                (0..self.richtungen.len()).filter(|&b| self.raus.get(b).is_some_and(|n| *n > 0)).map(|b| self.art_zu(a, b)).collect()
            }
        }
    }

    fn art_zu(&self, a: usize, b: usize) -> Art {
        if a == b { Art::Wenden } else { Art::aus(self.richtungen[b] - (self.richtungen[a] + 180.0)) }
    }

    fn netz_liste(&self) -> Vec<[usize; 4]> {
        self.verbindungen.iter().filter_map(|x| match x.schluessel { Schluessel::Netz(n) => Some(n), _ => None }).collect()
    }

    /// eine Verbindung an- bzw. abschalten
    pub fn schalten(&self, i: usize) -> Option<Aenderung> {
        let vb = self.verbindungen.get(i)?;
        match (&self.ziel, &vb.schluessel) {
            (Ziel::Netz(k), Schluessel::Netz(n)) => Some(Aenderung::Netz(*k, self.netz_liste().into_iter().filter(|x| x != n).collect())),
            (Ziel::Karte { .. }, _) => Some(self.karte_setzen(&[i], !vb.erlaubt)),
            _ => None,
        }
    }

    /// Spurpfeil setzen: die Zufahrtsspur z darf in Richtung `art` fahren (oder nicht mehr)
    pub fn pfeil(&self, z: usize, art: Art, an: bool) -> Option<Aenderung> {
        match self.ziel {
            Ziel::Karte { .. } => {
                let betroffen: Vec<usize> = (0..self.verbindungen.len()).filter(|&i| self.verbindungen[i].zufahrt == z && self.verbindungen[i].art == art).collect();
                (!betroffen.is_empty()).then(|| self.karte_setzen(&betroffen, an))
            }
            Ziel::Netz(k) => {
                let zf = &self.zufahrten[z];
                let mut liste: Vec<[usize; 4]> = self.netz_liste().into_iter()
                    .filter(|x| !((x[0], x[1]) == (zf.arm, zf.spur) && self.art_zu(x[0], x[2]) == art)).collect();
                if an {
                    for b in 0..self.richtungen.len() {
                        let n = self.raus.get(b).copied().unwrap_or(0);
                        if n == 0 || self.art_zu(zf.arm, b) != art {
                            continue;
                        }
                        // rechts: in die rechte Spur, links und wenden: in die linke, geradeaus: in die gleiche
                        let j = match art { Art::Rechts => 0, Art::Links | Art::Wenden => n - 1, Art::Gerade => zf.spur.min(n - 1) };
                        liste.push([zf.arm, zf.spur, b, j]);
                    }
                }
                Some(Aenderung::Netz(k, liste))
            }
        }
    }

    /// Karte: Verbindungen `idx` erlauben bzw. sperren (Pfade, die eine andere erlaubte Verbindung braucht, bleiben frei)
    fn karte_setzen(&self, idx: &[usize], erlauben: bool) -> Aenderung {
        let Ziel::Karte { kachel, objekt } = self.ziel else { unreachable!() };
        let pfade = |i: usize| match &self.verbindungen[i].schluessel { Schluessel::Karte(p) => p.clone(), _ => vec![] };
        let mut betroffen: Vec<usize> = idx.iter().flat_map(|&i| pfade(i)).collect();
        betroffen.sort_unstable();
        betroffen.dedup();
        if erlauben {
            return Aenderung::Karte { kachel, objekt, sperren: vec![], frei: betroffen };
        }
        let gebraucht: HashSet<usize> = (0..self.verbindungen.len()).filter(|i| !idx.contains(i) && self.verbindungen[*i].erlaubt).flat_map(pfade).collect();
        Aenderung::Karte { kachel, objekt, sperren: betroffen.into_iter().filter(|p| !gebraucht.contains(p)).collect(), frei: vec![] }
    }

    /// Verbindung nahe dem Bildpunkt (Abstand zur Linie in Bildpunkten) - nur die der Zufahrt `nur`, wenn gesetzt
    pub fn verbindung_bei(&self, bild: impl Fn(DVec3) -> Option<DVec2>, maus: DVec2, nur: Option<usize>) -> Option<usize> {
        let mut best: Option<(f64, usize)> = None;
        for (i, vb) in self.verbindungen.iter().enumerate() {
            if nur.is_some_and(|z| z != vb.zufahrt) {
                continue;
            }
            let pts: Vec<DVec2> = vb.punkte.iter().filter_map(|p| bild(*p)).collect();
            for w in pts.windows(2) {
                let d = abstand_strecke(maus, w[0], w[1]);
                if d < 8.0 && best.map(|b| d < b.0).unwrap_or(true) {
                    best = Some((d, i));
                }
            }
        }
        best.map(|b| b.1)
    }

    /// Zufahrtsspur nahe dem Bildpunkt
    pub fn zufahrt_bei(&self, bild: impl Fn(DVec3) -> Option<DVec2>, maus: DVec2) -> Option<usize> {
        self.zufahrten.iter().enumerate().filter_map(|(i, z)| bild(z.pos).map(|p| ((p - maus).length(), i)))
            .filter(|x| x.0 < 12.0).min_by(|a, b| a.0.total_cmp(&b.0)).map(|x| x.1)
    }
}

fn abstand_strecke(p: DVec2, a: DVec2, b: DVec2) -> f64 {
    let ab = b - a;
    let t = if ab.length_squared() < 1e-9 { 0.0 } else { ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) };
    (a + ab * t - p).length()
}
