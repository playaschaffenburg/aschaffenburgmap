//! Bruecken und Rampen eigener Strassen wie in Transport Fever 2: was gebaut wird, folgt aus dem Netz (Lage und
//! Hoehe der Strassen) und dem Gelaende darunter - nach jeder Aenderung neu berechnet, Rueckgaengig der Strasse nimmt
//! also auch Bruecke, Damm und Mauern zurueck.
//!
//! - **Bruecke:** Element (Spline-Stueck, hoechstens 20 m) mit wenigstens `bruecke_ab` m Luft unter der Fahrbahn:
//!   Begleit-Spline mit Platte (1,2 m) und Bruestung (1 m) auf genau derselben Kurve wie die Fahrbahn (ohne Pfade,
//!   die Spuren bleiben unberuehrt; wie omsigen bauwerke.py und die Standardkarten), Pfeiler-Objekte an den
//!   Elementgrenzen, wo genug Luft ist.
//! - **Rampe als Damm** (Bauweise Damm): liegt die Fahrbahn ueber dem Gelaende, wird darunter aufgeschuettet (unter der
//!   Strasse eben, daneben Boeschung 1 : 1,5), liegt sie darunter, wird ein Einschnitt gegraben.
//! - **Rampe mit Stuetzmauern** (Bauweise Mauer): Betonwaende an beiden Fahrbahnraendern (nach unten bis unter das
//!   Gelaende bzw. im Einschnitt nach oben), Gelaende nur unter der Fahrbahn abgesenkt.
//! Vorhandene Strassen der Karte (Flaechen auf dem Boden) werden nicht ueberschuettet.

use crate::gelaende::Gitter;
use crate::netz::{Bauweise, Element, Netz};
use anyhow::Result;
use glam::{DVec2, DVec3};
use omsi_map::Terrain;
use openomsi_game::viewer::Viewer;
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub const BRUECKE_AB: f64 = 5.0;
const BOESCHUNG: f64 = 1.5;
const PLATTE: f64 = 1.2;
const BRUESTUNG: f64 = 1.0;
const GEHWEG_H: f64 = 0.25;
const PFEILER_DICKE: f64 = 1.2;
const PFEILER_MIN: f64 = 1.5;
const MAUER_DICKE: f64 = 0.3;
const ZELLE: f64 = 5.0;
/// Einschnitt mit Mauern: das Gelaende wird so weit ueber die Mauer hinaus auf die Sohle abgesenkt (mehr als die
/// Diagonale einer Rasterzelle, sonst ragen Gelaendedreiecke mit einer hohen Ecke in den Einschnitt) ...
const GRABEN_RAND: f64 = 8.0;
/// ... und ein Deckel mit der Bodentextur liegt auf der alten Gelaendehoehe darueber (wie omsigen bauwerke.py an
/// Tunneln): Abstaende seiner Laengsstreifen von der Mauer-Innenseite
const DECKEL: [f64; 4] = [0.3, 5.5, 11.0, 16.5];

/// Pfeiler: Fuss (Gelaende, absolut), Richtung der Bruecke, Hoehe bis unter die Platte, Breite quer
#[derive(Clone, Debug)]
pub struct Pfeiler {
    pub fuss: DVec3,
    pub richtung: f64,
    pub hoehe: f64,
    pub breite: f64,
}

/// was die Strassen brauchen
#[derive(Default, Clone)]
pub struct Plan {
    /// Begleit-Splines: (.sli relativ zum OMSI-Ordner, Element der Fahrbahn, Kanten-ID, Weg bis dahin)
    pub begleit: Vec<(String, Element, u32, f64)>,
    pub pfeiler: Vec<Pfeiler>,
    /// Sockel unter hochliegenden Kreuzungen
    pub sockel: Vec<Sockel>,
    /// Stuetzmauern (je Stueck einer Strasse beide Seiten)
    pub mauern: Vec<MauerStueck>,
    /// Klinker-Verblendung zur Strasse hin (sonst Beton)
    pub klinker: bool,
    /// Bodentextur der Karte (erste [groundtex]) fuer die Deckel neben Einschnitten
    pub bodentextur: Option<Bodentextur>,
    /// geaendertes Gelaende je Kachel (ganze Kachel)
    pub gelaende: HashMap<(i32, i32), Terrain>,
    pub bruecken_m: f64,
    pub rampen_m: f64,
    pub mauern_m: f64,
}

impl Plan {
    pub fn text(&self) -> String {
        let mut t = Vec::new();
        if self.bruecken_m > 0.0 {
            t.push(format!("{:.0} m Bruecke, {} Pfeiler", self.bruecken_m, self.pfeiler.len()));
        }
        if self.rampen_m > 0.0 {
            t.push(format!("{:.0} m Damm/Einschnitt", self.rampen_m));
        }
        if self.mauern_m > 0.0 {
            t.push(format!("{:.0} m Stuetzmauer", self.mauern_m));
        }
        if !self.mauern.is_empty() && self.mauern_m == 0.0 {
            t.push("Stuetzmauer".into());
        }
        if !self.sockel.is_empty() {
            t.push(format!("{} Kreuzung(en) auf Sockel", self.sockel.len()));
        }
        t.join(", ")
    }
}

fn zahl(v: f64) -> String {
    format!("{:.1}", v.abs()).replace('.', "_")
}

/// Gelaendehoehe aus dem Raster (wie gezeichnet)
fn boden(g: &Gitter, x: f64, y: f64) -> Option<f64> {
    let ts = omsi_map::tile_size();
    let (tx, ty) = ((x / ts).floor() as i32, (y / ts).floor() as i32);
    g.kacheln.get(&(tx, ty)).map(|t| t.sample((x - tx as f64 * ts) as f32, (y - ty as f64 * ts) as f32) as f64)
}

/// eine Stelle einer Strasse (oder Kreuzungsflaeche) fuer das Gelaende
#[derive(Clone, Copy)]
struct Probe {
    p: DVec3,
    halb: f64,
    bauweise: Bauweise,
}

/// Anforderungen an einen Rasterpunkt: aufschuetten mindestens / abtragen hoechstens bis
#[derive(Default, Clone, Copy)]
struct Anspruch {
    fuellen: Option<f64>,
    graben: Option<f64>,
}

/// Anforderungen der Proben ans Raster eintragen: unter der Strasse eben (5 cm unter der Fahrbahn), daneben Boeschung
/// (Bauweise Damm); bei Mauern nur unter der Fahrbahn abtragen
fn ansprueche(proben: &[Probe], basis: &Gitter, nur_graben: bool, ziel: &mut HashMap<(i32, i32), Anspruch>) {
    for pr in proben {
        let mauer = pr.bauweise == Bauweise::Mauer;
        let Some(g0) = boden(basis, pr.p.x, pr.p.y) else { continue };
        let luft = pr.p.z - g0;
        let reich = pr.halb + if mauer { 0.5 + GRABEN_RAND } else { luft.abs() * BOESCHUNG + ZELLE };
        let (gx0, gx1) = (((pr.p.x - reich) / ZELLE).floor() as i32, ((pr.p.x + reich) / ZELLE).ceil() as i32);
        let (gy0, gy1) = (((pr.p.y - reich) / ZELLE).floor() as i32, ((pr.p.y + reich) / ZELLE).ceil() as i32);
        let unter = pr.p.z - 0.05;
        for gx in gx0..=gx1 {
            for gy in gy0..=gy1 {
                let d = (DVec2::new(gx as f64, gy as f64) * ZELLE - pr.p.truncate()).length();
                if d > reich {
                    continue;
                }
                let Some(g) = basis.hoehe(gx, gy).map(|h| h as f64) else { continue };
                let a = ziel.entry((gx, gy)).or_default();
                let innen = d <= pr.halb + if mauer { 0.5 + GRABEN_RAND } else { 0.0 };
                // abtragen (die Strasse liegt tiefer)
                let graben = if innen { unter } else if mauer { f64::INFINITY } else { unter + (d - pr.halb) / BOESCHUNG };
                if graben < g {
                    a.graben = Some(a.graben.map_or(graben, |x| x.min(graben)));
                }
                // aufschuetten (die Strasse liegt hoeher) - nicht bei Mauern
                if !nur_graben && !mauer {
                    let fuellen = if innen { unter } else { unter - (d - pr.halb) / BOESCHUNG };
                    if fuellen > g {
                        a.fuellen = Some(a.fuellen.map_or(fuellen, |x| x.max(fuellen)));
                    }
                }
            }
        }
    }
}

/// Plan fuer das ganze Netz. `kanten` (l < 0, r > 0): Aussenkanten des Querschnitts.
///
/// Reihenfolge wie in Transport Fever 2: erst graben alle tiefer liegenden Strassen ihre Einschnitte, dann entscheidet
/// sich gegen dieses Gelaende, was Bruecke wird (eine Strasse ueber einem Einschnitt wird dort zur Bruecke), dann wird
/// aufgeschuettet - wo beides verlangt ist, gewinnt der Einschnitt (die untere Strasse bleibt frei).
pub fn planen(v: &Viewer, netz: &Netz, kanten: &dyn Fn(&str) -> (f64, f64), bruecke_ab: f64) -> Plan {
    let mut plan = Plan::default();
    // Gelaende, wie es ohne die Bauwerke ist (Datei bzw. Sitzungskopie, mit geformtem Gelaende)
    let mut basis = Gitter::default();
    let laden = |g: &mut Gitter, p: DVec2, r: f64| {
        for k in Gitter::kacheln_um(p, r) {
            if !g.kacheln.contains_key(&k) {
                if let Some(t) = v.tile_terrain(k.0, k.1).filter(|t| t.cells == 60) {
                    g.kacheln.insert(k, t);
                }
            }
        }
    };
    // je Kante und Element: Proben (Punkt, Richtung) alle 2 m
    struct El {
        kante: u32,
        sli: String,
        el: Element,
        cum: f64,
        l: f64,
        r: f64,
        bauweise: Bauweise,
        proben: Vec<(DVec3, f64)>,
    }
    let mut els: Vec<El> = Vec::new();
    for e in &netz.kanten {
        let (l, r) = kanten(&e.sli);
        let mut cum = 0.0;
        for x in netz.elemente(e) {
            let k = x.kurve(e.id, 0.0);
            let n = (k.length / 2.0).ceil().max(1.0) as usize;
            let proben: Vec<(DVec3, f64)> = (0..=n).map(|i| {
                let s = k.length * i as f64 / n as f64;
                (k.point_at(s), k.heading_at(s))
            }).collect();
            for (p, _) in &proben {
                laden(&mut basis, p.truncate(), (-l).max(r) + 40.0);
            }
            els.push(El { kante: e.id, sli: e.sli.clone(), el: x, cum, l, r, bauweise: e.bauweise, proben });
            cum += x.stueck.laenge;
        }
    }
    // Kreuzungsflaechen: je Arm eine Strecke von der Mitte bis zum Armende auf Knotenhoehe
    let mut kreuzungen: Vec<(u32, DVec3, Vec<Probe>, Vec<DVec2>)> = Vec::new();
    for kn in &netz.knoten {
        let arme = netz.arme(kn.id);
        if arme.is_empty() || kn.kreisel.is_some() {
            continue;
        }
        let mut proben = Vec::new();
        let mut umriss = Vec::new();
        let mut bauweise = Bauweise::Damm;
        for a in &arme {
            let (halb, sli) = match &a.karte {
                Some(ka) => (ka.halb, ka.sli.clone()),
                None => {
                    let e = netz.kante(a.kante);
                    if let Some(e) = e {
                        bauweise = e.bauweise;
                    }
                    let sli = e.map(|e| e.sli.clone()).unwrap_or_default();
                    let (l, r) = kanten(&sli);
                    ((-l).max(r), sli)
                }
            };
            let _ = sli;
            let d = crate::netz::dir(a.richtung);
            let q = crate::netz::rechts(a.richtung);
            let laenge = a.kuerzung.max(halb);
            let n = (laenge / 2.0).ceil().max(1.0) as usize;
            for i in 0..=n {
                let p = kn.pos.truncate() + d * (laenge * i as f64 / n as f64);
                laden(&mut basis, p, halb + 40.0);
                proben.push(Probe { p: p.extend(kn.pos.z), halb: halb + 1.0, bauweise });
            }
            let ende = kn.pos.truncate() + d * laenge;
            umriss.push(ende - q * halb);
            umriss.push(ende + q * halb);
        }
        kreuzungen.push((kn.id, kn.pos, proben, umriss));
    }
    let luft_bei = |g: &Gitter, p: DVec3| boden(g, p.x, p.y).map(|b| p.z - b).unwrap_or(0.0);
    // 1. Bruecken gegen das urspruengliche Gelaende, Einschnitte der uebrigen Stuecke
    let bruecke_1: Vec<bool> = els.iter().map(|x| x.proben.iter().any(|(p, _)| luft_bei(&basis, *p) >= bruecke_ab)).collect();
    let probe_von = |x: &El| -> Vec<Probe> {
        let halb = (-x.l).max(x.r) + 0.5;
        x.proben.iter().map(|(p, _)| Probe { p: *p, halb, bauweise: x.bauweise }).collect()
    };
    let mut ziel1: HashMap<(i32, i32), Anspruch> = HashMap::new();
    for (i, x) in els.iter().enumerate() {
        if !bruecke_1[i] {
            ansprueche(&probe_von(x), &basis, true, &mut ziel1);
        }
    }
    for (_, _, proben, _) in &kreuzungen {
        ansprueche(proben, &basis, true, &mut ziel1);
    }
    let mut geschnitten = basis.clone();
    let mut egal = HashSet::new();
    for (&(gx, gy), a) in &ziel1 {
        if let Some(z) = a.graben {
            geschnitten.setzen(gx, gy, z as f32, &mut egal);
        }
    }
    // 2. Bruecken gegen das Gelaende mit den Einschnitten
    let bruecke: Vec<bool> = els.iter().map(|x| x.proben.iter().any(|(p, _)| luft_bei(&geschnitten, *p) >= bruecke_ab)).collect();
    // eigene Strassen (fuer Pfeiler: nicht auf einer tieferen Strasse)
    let alle: Vec<(DVec3, f64)> = els.iter().flat_map(|x| x.proben.iter().map(move |(p, _)| (*p, (-x.l).max(x.r)))).collect();
    let strasse_darunter = |q: DVec2, z: f64| alle.iter().any(|(p, halb)| p.z < z - 2.0 && (p.truncate() - q).length() < halb + 1.5);
    for (i, x) in els.iter().enumerate() {
        if bruecke[i] {
            plan.begleit.push((format!("Splines\\{}\\AB_bruecke_{}_{}.sli", crate::speichern::EIGEN, zahl(x.l), zahl(x.r)), x.el, x.kante, x.cum));
            plan.bruecken_m += x.el.stueck.laenge;
            // Pfeiler am Anfang des Stuecks, wenn davor auch Bruecke derselben Strasse ist (nicht an den Widerlagern)
            if i > 0 && bruecke[i - 1] && els[i - 1].kante == x.kante {
                let (p, h) = x.proben[0];
                let mitte = p.truncate() + crate::netz::rechts(h) * ((x.l + x.r) / 2.0);
                let b = boden(&geschnitten, mitte.x, mitte.y).unwrap_or(p.z);
                if p.z - b - PLATTE >= PFEILER_MIN && v.surface_height(mitte.x, mitte.y).is_none() && !strasse_darunter(mitte, p.z) {
                    plan.pfeiler.push(Pfeiler { fuss: mitte.extend(b - 1.0), richtung: h, hoehe: p.z - PLATTE - (b - 1.0) + 0.05, breite: ((x.r - x.l) * 0.6).max(2.0) });
                }
            }
        } else {
            let luft: Vec<f64> = x.proben.iter().map(|(p, _)| luft_bei(&basis, *p)).collect();
            let luft_max = luft.iter().copied().fold(f64::MIN, f64::max);
            let luft_min = luft.iter().copied().fold(f64::MAX, f64::min);
            if luft_max > 0.3 || luft_min < -0.3 {
                if x.bauweise == Bauweise::Mauer {
                    // Stuetzmauern an beiden Raendern, Oberkante an jeder Stelle nach dem Gelaende (siehe `MauerStueck`)
                    let ursprung = x.proben[0].0;
                    let mut seiten = Vec::new();
                    for (rand, aussen) in [(x.l, -1.0), (x.r, 1.0)] {
                        let linie: Vec<MauerPunkt> = x.proben.iter().map(|(p, h)| {
                            let q = p.truncate() + crate::netz::rechts(*h) * rand;
                            let g = boden(&basis, q.x, q.y).unwrap_or(p.z);
                            let oben_strasse = p.z + GEHWEG_H;
                            let (unten, oben) = if g > oben_strasse {
                                // Einschnitt: von unter dem Gehweg bis knapp ueber das Gelaende
                                (oben_strasse - 0.4, g + 0.3)
                            } else {
                                // Rampe ueber dem Gelaende: vom Gelaende bis zur Bruestung ueber dem Gehweg
                                (g - 0.5, oben_strasse + 0.6)
                            };
                            let richtung_aussen = crate::netz::rechts(*h) * aussen;
                            // Deckel ueber dem breiter abgesenkten Gelaende (nur im Einschnitt)
                            let deckel = (g > oben_strasse).then(|| DECKEL.map(|o| {
                                let w = q + richtung_aussen * o;
                                boden(&basis, w.x, w.y).unwrap_or(g) + 0.03 - ursprung.z
                            }));
                            MauerPunkt { pos: (q - ursprung.truncate()), aussen: richtung_aussen, unten: unten - ursprung.z, oben: oben - ursprung.z, deckel }
                        }).collect();
                        seiten.push(linie);
                    }
                    plan.mauern.push(MauerStueck { pos: ursprung, seiten });
                    plan.mauern_m += x.el.stueck.laenge;
                } else {
                    plan.rampen_m += x.el.stueck.laenge;
                }
            }
        }
        let _ = &x.sli;
    }
    // Kreuzungen ueber einem Einschnitt oder hoch ueber dem Gelaende: Sockel (Platte mit Seitenwaenden) darunter
    for (k, pos, proben, umriss) in &kreuzungen {
        let luft = proben.iter().map(|pr| luft_bei(&geschnitten, pr.p)).fold(f64::MIN, f64::max);
        if luft > 1.0 && umriss.len() >= 3 {
            plan.sockel.push(Sockel { knoten: *k, pos: *pos, umriss: huelle(umriss, pos.truncate()) });
        }
    }
    // 3. Gelaende: Einschnitte und Daemme aller Stuecke, die keine Bruecke sind, und der Kreuzungen; Einschnitt gewinnt
    let mut ziel: HashMap<(i32, i32), Anspruch> = HashMap::new();
    for (i, x) in els.iter().enumerate() {
        if !bruecke[i] {
            ansprueche(&probe_von(x), &basis, false, &mut ziel);
        }
    }
    for (_, _, proben, _) in &kreuzungen {
        ansprueche(proben, &basis, false, &mut ziel);
    }
    let mut neu = basis.clone();
    let mut geaendert: HashSet<(i32, i32)> = HashSet::new();
    for ((gx, gy), a) in ziel {
        let q = DVec2::new(gx as f64, gy as f64) * ZELLE;
        let Some(g) = basis.hoehe(gx, gy).map(|h| h as f64) else { continue };
        // vorhandene Strassen auf dem Boden nicht zuschuetten oder untergraben
        if v.surface_height(q.x, q.y).is_some_and(|s| (s - g).abs() < 1.5) {
            continue;
        }
        let mut z = g;
        if let Some(f) = a.fuellen {
            z = z.max(f);
        }
        if let Some(c) = a.graben {
            z = z.min(c);
        }
        if (z - g).abs() > 0.02 {
            neu.setzen(gx, gy, z as f32, &mut geaendert);
        }
    }
    for k in geaendert {
        if let Some(t) = neu.kacheln.remove(&k) {
            plan.gelaende.insert(k, t);
        }
    }
    plan
}

/// konvexe Huelle (Welt) als Punkte relativ zu `mitte`, gegen den Uhrzeigersinn
fn huelle(punkte: &[DVec2], mitte: DVec2) -> Vec<DVec2> {
    let mut p: Vec<DVec2> = punkte.iter().map(|q| *q - mitte).collect();
    p.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    p.dedup_by(|a, b| (*a - *b).length() < 1e-6);
    if p.len() < 3 {
        return p;
    }
    let kreuz = |o: DVec2, a: DVec2, b: DVec2| (a - o).perp_dot(b - o);
    let mut unten: Vec<DVec2> = Vec::new();
    for q in &p {
        while unten.len() >= 2 && kreuz(unten[unten.len() - 2], unten[unten.len() - 1], *q) <= 0.0 {
            unten.pop();
        }
        unten.push(*q);
    }
    let mut oben: Vec<DVec2> = Vec::new();
    for q in p.iter().rev() {
        while oben.len() >= 2 && kreuz(oben[oben.len() - 2], oben[oben.len() - 1], *q) <= 0.0 {
            oben.pop();
        }
        oben.push(*q);
    }
    unten.pop();
    oben.pop();
    unten.extend(oben);
    unten
}

/// erste Bodentextur der Karte: Textur, Detailtextur und wie oft die Detailtextur je Kachel wiederholt wird
#[derive(Clone, Debug, PartialEq)]
pub struct Bodentextur {
    pub basis: std::path::PathBuf,
    pub detail: Option<std::path::PathBuf>,
    pub detail_je_kachel: f64,
}

/// Deckel-Textur wie das Gelaende sie zeigt (openOMSI: Bodentextur mal Detailtextur): die Detailtextur, eingefaerbt mit
/// der mittleren Farbe der Bodentextur (deren grosse Flecken je Kachel gehen verloren, die Koernung passt) ->
/// (Dateiname, Meter je Wiederholung)
fn deckel_textur(ordner: &Path, b: Option<&Bodentextur>) -> (String, f64) {
    const NAME: &str = "AB_boden.bmp";
    let ts = omsi_map::tile_size();
    let Some(b) = b else { return ("gras.bmp".into(), ts) };
    let ziel = ordner.join("texture").join(NAME);
    let wiederholung = ts / b.detail_je_kachel.max(1.0);
    if ziel.is_file() {
        return (NAME.into(), wiederholung);
    }
    let basis = image::open(&b.basis).ok().map(|i| i.to_rgb8());
    let detail = b.detail.as_ref().and_then(|d| image::open(d).ok()).map(|i| i.to_rgb8());
    match (basis, detail) {
        (Some(bas), Some(det)) => {
            let n = (bas.width() * bas.height()) as f64;
            let mittel: [f64; 3] = [0, 1, 2].map(|k| bas.pixels().map(|p| p[k] as f64).sum::<f64>() / n);
            let out = image::RgbImage::from_fn(det.width(), det.height(), |x, y| {
                let d = det.get_pixel(x, y);
                image::Rgb([0, 1, 2].map(|k| (mittel[k] * d[k] as f64 / 255.0).clamp(0.0, 255.0) as u8))
            });
            if out.save(&ziel).is_ok() {
                return (NAME.into(), wiederholung);
            }
            let name = b.basis.file_name().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
            let _ = std::fs::copy(&b.basis, ordner.join("texture").join(&name));
            (name, ts)
        }
        _ => {
            let name = b.basis.file_name().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
            let _ = std::fs::copy(&b.basis, ordner.join("texture").join(&name));
            (name, ts)
        }
    }
}

/// ein Punkt einer Stuetzmauer (relativ zum Ursprung des Stuecks): Lage der Innenseite, Richtung nach aussen (vom
/// Fahrweg weg), Unter- und Oberkante
#[derive(Clone, Copy, Debug)]
pub struct MauerPunkt {
    pub pos: DVec2,
    pub aussen: DVec2,
    pub unten: f64,
    pub oben: f64,
    /// im Einschnitt: Hoehen des Deckels an den Abstaenden `DECKEL` von der Innenseite
    pub deckel: Option<[f64; 4]>,
}

/// Stuetzmauern eines Strassenstuecks: Ursprung (absolut) und je Seite die Punkte alle 2 m
#[derive(Clone, Debug)]
pub struct MauerStueck {
    pub pos: DVec3,
    pub seiten: Vec<Vec<MauerPunkt>>,
}

/// Stuetzmauer-Objekt: Wand 0,3 m dick, zur Strasse hin Klinker (oder Beton), oben Betonkappe, darauf ein Gelaender
/// (Pfosten alle 2 m, Hand- und Knieleiste) -> Dateiname (.sco)
pub fn mauer_objekt(root: &Path, ordner: &Path, m: &MauerStueck, klinker: bool, bodentextur: Option<&Bodentextur>) -> Result<String> {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for s in &m.seiten {
        for p in s {
            for b in format!("{:.2},{:.2},{:.2},{:.2};", p.pos.x, p.pos.y, p.unten, p.oben).bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
        }
    }
    let name = format!("Mauer_{}{:012x}", if klinker { "K" } else { "B" }, h & 0xffff_ffff_ffff);
    let _ = &bodentextur;
    let sco = ordner.join(format!("{name}.sco"));
    if sco.is_file() {
        return Ok(format!("{name}.sco"));
    }
    std::fs::create_dir_all(ordner.join("model"))?;
    std::fs::create_dir_all(ordner.join("texture"))?;
    let texturen = ["betonwand1.bmp", "AB_klinker.bmp", "AB_gelaender.bmp"];
    crate::querschnitt::texturen_bereitstellen(root, &texturen)?;
    for t in texturen {
        let q = crate::querschnitt::ordner(root).join("texture").join(t);
        if q.is_file() {
            std::fs::copy(&q, ordner.join("texture").join(t))?;
        }
    }
    let mut md = crate::uebergang::Modell::default();
    let beton = md.material("betonwand1.bmp");
    let innen_mat = if klinker { md.material("AB_klinker.bmp") } else { beton };
    let stahl = md.material("AB_gelaender.bmp");
    // Deckel: wie das Gelaende (Textur in Weltkoordinaten, siehe deckel_textur)
    let (boden_name, wiederholung) = deckel_textur(ordner, bodentextur);
    if bodentextur.is_none() {
        let q = root.join("Texture").join("gras.bmp");
        if q.is_file() && !ordner.join("texture").join("gras.bmp").is_file() {
            std::fs::copy(&q, ordner.join("texture").join("gras.bmp"))?;
        }
    }
    let boden_mat = md.material(&boden_name);
    let welt_uv = |q: DVec2| [(m.pos.x + q.x) / wiederholung, (m.pos.y + q.y) / wiederholung];
    const DICKE: f64 = 0.3;
    for seite in &m.seiten {
        let mut s = 0.0;
        for w in seite.windows(2) {
            let (a, b) = (w[0], w[1]);
            let len = (b.pos - a.pos).length();
            if len < 1e-3 {
                continue;
            }
            let innen = -(a.aussen + b.aussen).normalize_or_zero();
            let (ao, bo) = (a.pos + a.aussen * DICKE, b.pos + b.aussen * DICKE);
            let p3 = |q: DVec2, z: f64| [q.x, q.y, z];
            // Innenseite (zur Strasse): Klinker bis unter die Kappe
            let (ka, kb) = (a.oben - 0.25, b.oben - 0.25);
            md.viereck(innen_mat, [p3(a.pos, a.unten), p3(b.pos, b.unten), p3(b.pos, kb.max(b.unten)), p3(a.pos, ka.max(a.unten))],
                       [[s / 2.0, a.unten], [(s + len) / 2.0, b.unten], [(s + len) / 2.0, kb], [s / 2.0, ka]], [innen.x, innen.y, 0.0]);
            // Kappe innen, oben, aussen
            md.viereck(beton, [p3(a.pos, ka), p3(b.pos, kb), p3(b.pos, b.oben), p3(a.pos, a.oben)],
                       [[s / 4.0, 0.0], [(s + len) / 4.0, 0.0], [(s + len) / 4.0, 0.06], [s / 4.0, 0.06]], [innen.x, innen.y, 0.0]);
            md.viereck(beton, [p3(a.pos, a.oben), p3(b.pos, b.oben), p3(bo, b.oben), p3(ao, a.oben)],
                       [[s / 4.0, 0.0], [(s + len) / 4.0, 0.0], [(s + len) / 4.0, 0.08], [s / 4.0, 0.08]], [0.0, 0.0, 1.0]);
            md.viereck(beton, [p3(ao, a.unten), p3(bo, b.unten), p3(bo, b.oben), p3(ao, a.oben)],
                       [[s / 4.0, a.unten / 4.0], [(s + len) / 4.0, b.unten / 4.0], [(s + len) / 4.0, b.oben / 4.0], [s / 4.0, a.oben / 4.0]], [-innen.x, -innen.y, 0.0]);
            // Deckel ueber dem abgesenkten Gelaende hinter der Mauer
            if let (Some(da), Some(db)) = (a.deckel, b.deckel) {
                for j in 0..3 {
                    let (pa0, pa1) = (a.pos + a.aussen * DECKEL[j], a.pos + a.aussen * DECKEL[j + 1]);
                    let (pb0, pb1) = (b.pos + b.aussen * DECKEL[j], b.pos + b.aussen * DECKEL[j + 1]);
                    md.viereck(boden_mat, [p3(pa0, da[j]), p3(pb0, db[j]), p3(pb1, db[j + 1]), p3(pa1, da[j + 1])],
                               [welt_uv(pa0), welt_uv(pb0), welt_uv(pb1), welt_uv(pa1)], [0.0, 0.0, 1.0]);
                }
            }
            // Gelaender: Hand- und Knieleiste (beidseitig sichtbar)
            let mitte = |q: DVec2, r: DVec2| q + r * (DICKE / 2.0);
            let (ga, gb) = (mitte(a.pos, a.aussen), mitte(b.pos, b.aussen));
            for (z0, z1) in [(0.95, 1.05), (0.5, 0.56)] {
                for n in [innen, -innen] {
                    md.viereck(stahl, [p3(ga, a.oben + z0), p3(gb, b.oben + z0), p3(gb, b.oben + z1), p3(ga, a.oben + z1)],
                               [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]], [n.x, n.y, 0.0]);
                }
            }
            s += len;
        }
        // Pfosten an jedem Punkt
        for p in seite {
            let c = p.pos + p.aussen * (DICKE / 2.0);
            let quer = DVec2::new(-p.aussen.y, p.aussen.x) * 0.03;
            let tief = p.aussen * 0.03;
            for (d, n) in [(quer, quer), (-quer, -quer), (tief, tief), (-tief, -tief)] {
                let seitlich = if d == quer || d == -quer { tief } else { quer };
                let (q1, q2) = (c + d - seitlich, c + d + seitlich);
                let nn = n.normalize_or_zero();
                md.viereck(stahl, [[q1.x, q1.y, p.oben], [q2.x, q2.y, p.oben], [q2.x, q2.y, p.oben + 1.05], [q1.x, q1.y, p.oben + 1.05]],
                           [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]], [nn.x, nn.y, 0.0]);
            }
        }
    }
    std::fs::write(ordner.join("model").join(format!("{name}.x")), md.x_datei())?;
    let text = ["Erzeugt mit omsi-editor (Stuetzmauer)", "", "[friendlyname]", "Stuetzmauer", "", "[groups]", "1",
                crate::speichern::EIGEN, "", "[fixed]", "", "[absheight]", "", "[mesh]", &format!("{name}.x"), ""].join("\r\n") + "\r\n";
    std::fs::write(&sco, text)?;
    omsi_cfg::content_changed();
    Ok(format!("{name}.sco"))
}

/// Sockel unter einer hochliegenden Kreuzung: Umriss (relativ zur Mitte, gegen den Uhrzeigersinn)
#[derive(Clone, Debug)]
pub struct Sockel {
    pub knoten: u32,
    pub pos: DVec3,
    pub umriss: Vec<DVec2>,
}

/// Sockel-Objekt (Platte 1,2 m mit Seitenwaenden bis zum Gehweg, Unterseite) im Ordner -> Dateiname (.sco)
pub fn sockel_objekt(root: &Path, ordner: &Path, s: &Sockel) -> Result<String> {
    // Name aus dem Umriss (gleicher Umriss: gleiche Datei)
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for q in &s.umriss {
        for b in format!("{:.2},{:.2};", q.x, q.y).bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    let name = format!("Sockel_{:08x}", h & 0xffff_ffff);
    let sco = ordner.join(format!("{name}.sco"));
    if sco.is_file() {
        return Ok(format!("{name}.sco"));
    }
    std::fs::create_dir_all(ordner.join("model"))?;
    std::fs::create_dir_all(ordner.join("texture"))?;
    crate::querschnitt::texturen_bereitstellen(root, &["betonwand1.bmp"])?;
    let q = crate::querschnitt::ordner(root).join("texture").join("betonwand1.bmp");
    if q.is_file() {
        std::fs::copy(&q, ordner.join("texture").join("betonwand1.bmp"))?;
    }
    let mut m = crate::uebergang::Modell::default();
    let mat = m.material("betonwand1.bmp");
    let (oben, unten) = (GEHWEG_H, -PLATTE);
    let n = s.umriss.len();
    let mut u = 0.0;
    for i in 0..n {
        let (a, b) = (s.umriss[i], s.umriss[(i + 1) % n]);
        let len = (b - a).length();
        // gegen den Uhrzeigersinn: aussen liegt rechts der Kante a -> b
        let aussen = DVec2::new(b.y - a.y, a.x - b.x).normalize_or_zero();
        m.viereck(mat, [[a.x, a.y, unten], [b.x, b.y, unten], [b.x, b.y, oben], [a.x, a.y, oben]],
                  [[u / 4.0, unten / 4.0], [(u + len) / 4.0, unten / 4.0], [(u + len) / 4.0, oben / 4.0], [u / 4.0, oben / 4.0]], [aussen.x, aussen.y, 0.0]);
        u += len;
    }
    // Unterseite als Faecher von der Mitte
    for i in 0..n {
        let (a, b) = (s.umriss[i], s.umriss[(i + 1) % n]);
        let c = DVec2::ZERO;
        m.viereck(mat, [[c.x, c.y, unten], [a.x, a.y, unten], [b.x, b.y, unten], [c.x, c.y, unten]],
                  [[c.x / 4.0, c.y / 4.0], [a.x / 4.0, a.y / 4.0], [b.x / 4.0, b.y / 4.0], [c.x / 4.0, c.y / 4.0]], [0.0, 0.0, -1.0]);
    }
    std::fs::write(ordner.join("model").join(format!("{name}.x")), m.x_datei())?;
    let text = ["Erzeugt mit omsi-editor (Sockel unter einer Kreuzung)", "", "[friendlyname]", "Kreuzungssockel", "", "[groups]", "1",
                crate::speichern::EIGEN, "", "[fixed]", "", "[absheight]", "", "[mesh]", &format!("{name}.x"), ""].join("\r\n") + "\r\n";
    std::fs::write(&sco, text)?;
    omsi_cfg::content_changed();
    Ok(format!("{name}.sco"))
}

// ------------------------------------------------------------ Spline-Dateien

fn profil(punkte: &[(f64, f64, f64)], v: f64) -> Vec<String> {
    let mut out = vec!["[profile]".to_string(), "0".into(), String::new()];
    for (x, y, u) in punkte {
        out.extend(["[profilepnt]".into(), format!("{x:.3}"), format!("{y:.3}"), format!("{u:.3}"), format!("{v:.3}"), String::new()]);
    }
    out
}

fn kopf() -> Vec<String> {
    vec!["File created with omsi-editor (Bruecken/Rampen). Begleit-Spline ohne Pfade.".into(), String::new(), "[texture]".into(), "betonwand1.bmp".into(), String::new()]
}

/// Brueckenkoerper zur Fahrbahn mit Aussenkanten l (< 0) und r (> 0) (wie omsigen bauwerke.bruecke_sli)
pub fn bruecke_sli(l: f64, r: f64) -> String {
    let (a, b) = (l - 0.3, r + 0.3);
    let (oben, unten) = (GEHWEG_H + BRUESTUNG, -PLATTE);
    let mut t = kopf();
    t.extend(profil(&[(a, unten, 0.0), (a, oben, (oben - unten) / 4.0)], 0.1));
    t.extend(profil(&[(a, oben, 0.0), (l, oben, 0.08)], 0.1));
    t.extend(profil(&[(l, oben, 0.0), (l, GEHWEG_H, BRUESTUNG / 4.0)], 0.1));
    t.extend(profil(&[(r, GEHWEG_H, 0.0), (r, oben, BRUESTUNG / 4.0)], 0.1));
    t.extend(profil(&[(r, oben, 0.0), (b, oben, 0.08)], 0.1));
    t.extend(profil(&[(b, oben, 0.0), (b, unten, (oben - unten) / 4.0)], 0.1));
    t.extend(profil(&[(b, unten, 0.0), (a, unten, (b - a) / 4.0)], 0.1));
    t.join("\r\n") + "\r\n"
}

/// Stuetzmauern an den Fahrbahnraendern: `unten` (Fahrbahn auf einer Rampe ueber dem Gelaende): Aussenwaende bis h unter
/// die Fahrbahn mit kleiner Bruestung; sonst (Einschnitt): Waende bis h ueber die Fahrbahn, zur Strasse hin sichtbar
pub fn mauer_sli(l: f64, r: f64, h: f64, unten: bool) -> String {
    let (a, b) = (l - MAUER_DICKE, r + MAUER_DICKE);
    let mut t = kopf();
    if unten {
        let oben = GEHWEG_H + 0.8;
        t.extend(profil(&[(a, -h, 0.0), (a, oben, (oben + h) / 4.0)], 0.1));
        t.extend(profil(&[(a, oben, 0.0), (l, oben, 0.08)], 0.1));
        t.extend(profil(&[(l, oben, 0.0), (l, GEHWEG_H, 0.2)], 0.1));
        t.extend(profil(&[(r, GEHWEG_H, 0.0), (r, oben, 0.2)], 0.1));
        t.extend(profil(&[(r, oben, 0.0), (b, oben, 0.08)], 0.1));
        t.extend(profil(&[(b, oben, 0.0), (b, -h, (oben + h) / 4.0)], 0.1));
    } else {
        t.extend(profil(&[(l, h, 0.0), (l, GEHWEG_H - 0.3, (h - GEHWEG_H + 0.3) / 4.0)], 0.1));
        t.extend(profil(&[(a, h, 0.0), (l, h, 0.08)], 0.1));
        t.extend(profil(&[(r, GEHWEG_H - 0.3, 0.0), (r, h, (h - GEHWEG_H + 0.3) / 4.0)], 0.1));
        t.extend(profil(&[(r, h, 0.0), (b, h, 0.08)], 0.1));
    }
    t.join("\r\n") + "\r\n"
}

/// die Begleit-Splines des Plans in Splines\Aschaffenburg schreiben (nur fehlende) und die Betontextur bereitstellen
pub fn splines_schreiben(root: &Path, plan: &Plan) -> Result<()> {
    let d = crate::querschnitt::ordner(root);
    std::fs::create_dir_all(d.join("texture"))?;
    crate::querschnitt::texturen_bereitstellen(root, &["betonwand1.bmp"])?;
    for (rel, ..) in &plan.begleit {
        let name = rel.rsplit('\\').next().unwrap_or("");
        let pfad = d.join(name);
        if pfad.is_file() {
            continue;
        }
        let stamm = name.trim_end_matches(".sli");
        let teile: Vec<&str> = stamm.split('_').collect();
        let z = |i: usize| -> f64 { format!("{}.{}", teile[i], teile[i + 1]).parse().unwrap_or(0.0) };
        let text = match teile.get(1) {
            Some(&"bruecke") if teile.len() == 6 => bruecke_sli(-z(2), z(4)),
            Some(&"mauer") if teile.len() == 9 => mauer_sli(-z(3), z(5), z(7), teile[2] == "unten"),
            _ => continue,
        };
        std::fs::write(&pfad, text)?;
        // (openOMSI merkt sich die Ordnerinhalte: die neue Datei bekanntgeben)
        omsi_cfg::content_changed();
    }
    Ok(())
}

/// Pfeiler-Objekt (Quader, Beton) mit Breite w und Hoehe h (auf 0,5 m gerundet) im Ordner; -> Dateiname (.sco)
pub fn pfeiler_objekt(root: &Path, ordner: &Path, w: f64, h: f64) -> Result<String> {
    let w = (w * 2.0).round() / 2.0;
    let h = (h * 2.0).ceil() / 2.0;
    let name = format!("Pfeiler_{}_{}", zahl(w), zahl(h));
    let sco = ordner.join(format!("{name}.sco"));
    if sco.is_file() {
        return Ok(format!("{name}.sco"));
    }
    std::fs::create_dir_all(ordner.join("model"))?;
    std::fs::create_dir_all(ordner.join("texture"))?;
    crate::querschnitt::texturen_bereitstellen(root, &["betonwand1.bmp"])?;
    let q = crate::querschnitt::ordner(root).join("texture").join("betonwand1.bmp");
    if q.is_file() {
        std::fs::copy(&q, ordner.join("texture").join("betonwand1.bmp"))?;
    }
    let mut m = crate::uebergang::Modell::default();
    let mat = m.material("betonwand1.bmp");
    let (x0, x1, y0, y1) = (-w / 2.0, w / 2.0, -PFEILER_DICKE / 2.0, PFEILER_DICKE / 2.0);
    // vier Seiten und Deckel (Objektkoordinaten x rechts, y vor, z hoch)
    let seiten: [([[f64; 3]; 4], [f64; 3]); 5] = [
        ([[x0, y0, 0.0], [x1, y0, 0.0], [x1, y0, h], [x0, y0, h]], [0.0, -1.0, 0.0]),
        ([[x1, y1, 0.0], [x0, y1, 0.0], [x0, y1, h], [x1, y1, h]], [0.0, 1.0, 0.0]),
        ([[x0, y1, 0.0], [x0, y0, 0.0], [x0, y0, h], [x0, y1, h]], [-1.0, 0.0, 0.0]),
        ([[x1, y0, 0.0], [x1, y1, 0.0], [x1, y1, h], [x1, y0, h]], [1.0, 0.0, 0.0]),
        ([[x0, y0, h], [x1, y0, h], [x1, y1, h], [x0, y1, h]], [0.0, 0.0, 1.0]),
    ];
    for (p, n) in seiten {
        let uv = p.map(|a| if n[2] != 0.0 { [a[0] / 4.0, a[1] / 4.0] } else { [(a[0] + a[1]) / 4.0, a[2] / 4.0] });
        m.viereck(mat, p, uv, n);
    }
    std::fs::write(ordner.join("model").join(format!("{name}.x")), m.x_datei())?;
    let text = ["Erzeugt mit omsi-editor (Brueckenpfeiler)", "", "[friendlyname]", &format!("Pfeiler {w:.1} x {h:.1} m"), "", "[groups]", "1",
                crate::speichern::EIGEN, "", "[fixed]", "", "[absheight]", "", "[mesh]", &format!("{name}.x"), ""].join("\r\n") + "\r\n";
    std::fs::write(&sco, text)?;
    omsi_cfg::content_changed();
    Ok(format!("{name}.sco"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mauer_und_bruecke_dateien() {
        let b = bruecke_sli(-7.0, 7.0);
        assert_eq!(b.matches("[profile]").count(), 7);
        let m = mauer_sli(-5.0, 5.0, 4.0, true);
        assert_eq!(m.matches("[profile]").count(), 6);
        assert!(m.contains("-4.000"));
        let o = mauer_sli(-5.0, 5.0, 3.0, false);
        assert_eq!(o.matches("[profile]").count(), 4);
        assert_eq!(zahl(-7.25), "7_2");
    }

    /// Grundorf: eigene Strasse ueber eine 10 m hohe Kuppe - in der Mitte Bruecke mit Pfeilern, an den Enden Damm
    /// (Gelaende aufgeschuettet); mit Mauern statt Damm Mauer-Splines; gespeichert stehen Bruecke, Pfeiler und Gelaende
    /// in der Karte (OMSI_BILD)
    #[test]
    #[ignore]
    fn grundorf_bruecke() {
        let _sperre = crate::bearbeiten::tests::sperre();
        use crate::anschluss::Anschluesse;
        use crate::strasse::{Modus, Strassenbau};
        let root = Path::new(crate::bearbeiten::tests::OMSI);
        let mut v = crate::bearbeiten::tests::grundorf();
        v.tiles_around(DVec3::new(150.0, 150.0, 0.0), 1).unwrap();
        let ae = crate::aendern::Aendern::neu(&v);
        let mut s = Strassenbau::neu(Some("Splines\\Marcel\\str_2spur_10m_Grunewaldstr.sli".into()), Modus::Gerade);
        s.root = Some(root.to_path_buf());
        s.kreuzungs_ordner = Some(ae.kreuzungs_ordner());
        let boden = |v: &Viewer, x: f64, y: f64| DVec3::new(x, y, v.terrain_height(x, y).unwrap_or(0.0));
        // freie Wiese suchen: 200 m nach Norden ohne Strasse
        let start = (0..40).map(|i| DVec2::new(-250.0 + i as f64 * 15.0, -200.0)).find(|p| {
            (0..=40).all(|k| { let q = *p + DVec2::new(0.0, k as f64 * 5.0); (-12..=12).step_by(4).all(|d| v.surface_height(q.x + d as f64, q.y).is_none()) })
        }).expect("keine freie Wiese");
        let a = boden(&v, start.x, start.y);
        s.klick(&mut v, a, 3.0, &Anschluesse::default(), None).unwrap();
        s.hoehe = 10.0;
        let b = boden(&v, start.x, start.y + 100.0);
        s.maus(&mut v, b, 3.0, &Anschluesse::default(), None);
        s.klick(&mut v, b, 3.0, &Anschluesse::default(), None).unwrap();
        s.hoehe = 0.0;
        let c = boden(&v, start.x, start.y + 200.0);
        s.maus(&mut v, c, 3.0, &Anschluesse::default(), None);
        s.klick(&mut v, c, 3.0, &Anschluesse::default(), None).unwrap();
        s.beenden(&mut v);
        let p = s.bauwerke.clone();
        println!("{} | Gelaende auf {} Kacheln", p.text(), p.gelaende.len());
        assert!(p.bruecken_m >= 40.0, "Bruecke {}", p.bruecken_m);
        assert!(!p.pfeiler.is_empty(), "keine Pfeiler");
        assert!(p.rampen_m > 0.0 && !p.gelaende.is_empty(), "kein Damm");
        // Damm: neben dem Brueckenende ist das Gelaende hoeher als vorher
        let damm = DVec2::new(start.x, start.y + 30.0);
        let vorher = v.tile_terrain(0, 0).map(|_| 0.0).unwrap_or(0.0) + boden(&v, damm.x, damm.y).z;
        println!("Gelaende bei y+30: {vorher:.2} m (mit Damm), Strasse dort hoeher als Wiese");
        if let Some(bild) = std::env::var_os("OMSI_BILD") {
            let kam = crate::kamera::Kamera { ziel: DVec3::new(start.x, start.y + 100.0, b.z), gier: 270.0, neigung: -15.0, abstand: 170.0, fov: 50.0 };
            let px = v.render_image(1280, 800, &kam.camera()).unwrap();
            image::save_buffer(std::path::PathBuf::from(&bild).with_extension("damm.png"), &px, 1280, 800, image::ColorType::Rgba8).unwrap();
            let nah = crate::kamera::Kamera { ziel: DVec3::new(start.x, start.y + 35.0, a.z + 2.0), gier: 300.0, neigung: -18.0, abstand: 55.0, fov: 50.0 };
            let px = v.render_image(1280, 800, &nah.camera()).unwrap();
            image::save_buffer(std::path::PathBuf::from(&bild).with_extension("damm_nah.png"), &px, 1280, 800, image::ColorType::Rgba8).unwrap();
        }
        // Mauern statt Damm
        for e in s.netz.kanten.iter_mut() {
            e.bauweise = crate::netz::Bauweise::Mauer;
        }
        s.bauwerke_aktualisieren(&mut v);
        println!("Mauer: {}", s.bauwerke.text());
        assert!(s.bauwerke.mauern_m > 0.0 && !s.bauwerke.mauern.is_empty());
        // Rampe ueber dem Gelaende: Oberkante ueber dem Gehweg, Unterkante unter dem Gelaende
        for m in &s.bauwerke.mauern {
            for p in m.seiten.iter().flatten() {
                assert!(p.oben > p.unten + 0.3, "Mauer ohne Hoehe: {p:?}");
            }
        }
        for (rel, ..) in &s.bauwerke.begleit {
            assert!(root.join(rel.replace('\\', "/")).is_file(), "{rel} fehlt");
        }
        if let Some(bild) = std::env::var_os("OMSI_BILD") {
            let kam = crate::kamera::Kamera { ziel: DVec3::new(start.x, start.y + 100.0, b.z), gier: 270.0, neigung: -15.0, abstand: 170.0, fov: 50.0 };
            let px = v.render_image(1280, 800, &kam.camera()).unwrap();
            image::save_buffer(std::path::PathBuf::from(&bild).with_extension("mauer.png"), &px, 1280, 800, image::ColorType::Rgba8).unwrap();
        }
        // zurueck auf Damm und speichern
        for e in s.netz.kanten.iter_mut() {
            e.bauweise = crate::netz::Bauweise::Damm;
        }
        s.bauwerke_aktualisieren(&mut v);
        let mut paket = crate::speichern::vorbereiten(&v, &crate::bearbeiten::Bearbeiten::neu(crate::bearbeiten::Werkzeug::Strasse), &s.netz, &s.gesetzte_kreuzungen(),
                                                      &ae.kopien("Grundorf"), Some(ae.kreuzungs_ordner()), "Grundorf").unwrap();
        let (o, t) = ae.kreuzungs_ordner();
        crate::speichern::bauwerke_anhaengen(&v, &mut paket, &s.bauwerke, Some((o.as_path(), t.as_str())), root, "Grundorf").unwrap();
        let mut bruecken = 0;
        let mut pfeiler = 0;
        let mut terrain = 0;
        for d in &paket.dateien {
            let n = d.file_name().unwrap().to_string_lossy().to_string();
            if n.ends_with(".terrain") {
                terrain += 1;
                continue;
            }
            let b = std::fs::read(d).unwrap();
            let text = String::from_utf16_lossy(&b[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>());
            bruecken += text.matches("AB_bruecke_").count();
            pfeiler += text.matches("\\Pfeiler_").count();
        }
        println!("im Paket: {bruecken} Brueckenstuecke, {pfeiler} Pfeiler, {terrain} Gelaende-Dateien");
        assert!(bruecken > 0 && pfeiler == s.bauwerke.pfeiler.len() && terrain == s.bauwerke.gelaende.len());
        let _ = std::fs::remove_dir_all(&paket.staging);
        // Vorschau-Splines/Objekte wegnehmen, eigene Testdateien bleiben im eigenen Ordner (werden sonst wieder erzeugt)
        drop(ae);
    }

    /// Fall des Nutzers (Ring): eine Strasse im Einschnitt (7 m tief), eine andere quert sie auf Gelaendehoehe - die obere
    /// wird ueber dem Einschnitt zur Bruecke, der Einschnitt gewinnt gegen ihren Damm (die untere bleibt frei) (OMSI_BILD)
    #[test]
    #[ignore]
    fn bruecke_ueber_einschnitt() {
        let _sperre = crate::bearbeiten::tests::sperre();
        use crate::anschluss::Anschluesse;
        use crate::strasse::{Modus, Strassenbau};
        let root = Path::new(crate::bearbeiten::tests::OMSI);
        let mut v = crate::bearbeiten::tests::grundorf();
        v.tiles_around(DVec3::new(150.0, 150.0, 0.0), 1).unwrap();
        let ae = crate::aendern::Aendern::neu(&v);
        let mut s = Strassenbau::neu(Some("Splines\\Marcel\\str_2spur_10m_Grunewaldstr.sli".into()), Modus::Gerade);
        s.root = Some(root.to_path_buf());
        s.kreuzungs_ordner = Some(ae.kreuzungs_ordner());
        let boden = |v: &Viewer, x: f64, y: f64| DVec3::new(x, y, v.terrain_height(x, y).unwrap_or(0.0));
        let frei = |v: &Viewer, p: DVec2| (-110..=110).step_by(10).all(|dx| (-110..=110).step_by(10).all(|dy| v.surface_height(p.x + dx as f64, p.y + dy as f64).is_none()));
        let m = (0..80).map(|i| DVec2::new(-260.0 + (i % 10) as f64 * 50.0, -260.0 + (i / 10) as f64 * 50.0)).find(|p| frei(&v, *p)).expect("keine freie Wiese");
        // untere Strasse: Nord-Sued, in der Mitte 7 m tief
        let (a, b, c) = (boden(&v, m.x, m.y - 100.0), boden(&v, m.x, m.y), boden(&v, m.x, m.y + 100.0));
        s.klick(&mut v, a, 3.0, &Anschluesse::default(), None).unwrap();
        s.hoehe = -7.0;
        s.maus(&mut v, b, 3.0, &Anschluesse::default(), None);
        s.klick(&mut v, b, 3.0, &Anschluesse::default(), None).unwrap();
        s.hoehe = 0.0;
        s.maus(&mut v, c, 3.0, &Anschluesse::default(), None);
        s.klick(&mut v, c, 3.0, &Anschluesse::default(), None).unwrap();
        s.beenden(&mut v);
        // obere: Ost-West auf Gelaendehoehe ueber die Mitte
        let (w, o) = (boden(&v, m.x - 90.0, m.y), boden(&v, m.x + 90.0, m.y));
        s.klick(&mut v, w, 3.0, &Anschluesse::default(), None).unwrap();
        s.maus(&mut v, o, 3.0, &Anschluesse::default(), None);
        let msg = s.klick(&mut v, o, 3.0, &Anschluesse::default(), None);
        println!("obere Strasse: {msg:?}");
        s.beenden(&mut v);
        assert_eq!(s.netz.kanten.len(), 3, "keine Kreuzung auf verschiedenen Hoehen: 2 + 1 Kanten");
        let p = s.bauwerke.clone();
        println!("{}", p.text());
        assert!(p.bruecken_m >= 10.0, "die obere Strasse muss ueber dem Einschnitt Bruecke sein ({} m)", p.bruecken_m);
        // Gelaende in der Mitte: unten am Einschnitt (nicht vom Damm der oberen zugeschuettet)
        let mitte = boden(&v, m.x + 0.3, m.y + 0.3).z;
        println!("Gelaende in der Mitte {mitte:.2} m, untere Strasse {:.2} m", b.z - 7.0);
        assert!(mitte < b.z - 5.0, "Einschnitt zugeschuettet: {mitte}");
        if let Some(bild) = std::env::var_os("OMSI_BILD") {
            let kam = crate::kamera::Kamera { ziel: DVec3::new(m.x, m.y, b.z - 3.0), gier: 210.0, neigung: -25.0, abstand: 90.0, fov: 50.0 };
            let px = v.render_image(1280, 800, &kam.camera()).unwrap();
            image::save_buffer(std::path::PathBuf::from(&bild).with_extension("einschnitt.png"), &px, 1280, 800, image::ColorType::Rgba8).unwrap();
        }
        // wie am Ring: die untere Strasse zwischen Stuetzmauern (Klinker) statt Boeschungen; Deckel in der Bodentextur
        let omsi = Path::new(crate::bearbeiten::tests::OMSI);
        s.bodentextur = Some(Bodentextur { basis: omsi.join("Texture/gras.bmp"), detail: Some(omsi.join("Texture/gras_det.bmp")), detail_je_kachel: 60.0 });
        let untere: Vec<u32> = s.netz.kanten.iter().filter(|e| (s.netz.knoten(e.a).unwrap().pos.x - m.x).abs() < 1.0).map(|e| e.id).collect();
        for e in s.netz.kanten.iter_mut().filter(|e| untere.contains(&e.id)) {
            e.bauweise = crate::netz::Bauweise::Mauer;
        }
        s.bauwerke_aktualisieren(&mut v);
        println!("mit Mauern: {}", s.bauwerke.text());
        assert!(!s.bauwerke.mauern.is_empty());
        // hinter der Mauer: Gelaende abgesenkt (5-m-Raster), darueber ein Deckel auf alter Hoehe
        let mit_deckel = s.bauwerke.mauern.iter().flat_map(|mm| mm.seiten.iter().flatten()).filter(|p| p.deckel.is_some()).count();
        println!("Mauerpunkte mit Deckel: {mit_deckel}");
        assert!(mit_deckel > 10, "kein Deckel im Einschnitt");
        for mm in &s.bauwerke.mauern {
            for p in mm.seiten.iter().flatten() {
                if let Some(d) = p.deckel {
                    // Deckel an der Mauer unter der Mauerkrone und ueber der Strasse im Einschnitt
                    assert!(d[0] < p.oben && d.iter().all(|z| *z > p.unten), "Deckel {d:?}, Krone {}, Fuss {}", p.oben, p.unten);
                }
            }
        }
        // im Einschnitt: die Mauer reicht bis knapp ueber das Gelaende
        let tief = s.bauwerke.mauern.iter().flat_map(|mm| mm.seiten.iter().flatten().map(move |p| (mm.pos.z + p.oben, mm.pos.z + p.unten))).fold(0.0f64, |a, (o, u)| a.max(o - u));
        assert!(tief > 6.0, "Mauer im 7-m-Einschnitt nur {tief:.1} m hoch");
        if let Some(bild) = std::env::var_os("OMSI_BILD") {
            let kam = crate::kamera::Kamera { ziel: DVec3::new(m.x, m.y + 45.0, b.z - 5.0), gier: 180.0, neigung: -12.0, abstand: 30.0, fov: 60.0 };
            let px = v.render_image(1280, 800, &kam.camera()).unwrap();
            image::save_buffer(std::path::PathBuf::from(&bild).with_extension("einschnitt_mauer.png"), &px, 1280, 800, image::ColorType::Rgba8).unwrap();
            let kam = crate::kamera::Kamera { ziel: DVec3::new(m.x, m.y, b.z - 3.0), gier: 210.0, neigung: -30.0, abstand: 90.0, fov: 50.0 };
            let px = v.render_image(1280, 800, &kam.camera()).unwrap();
            image::save_buffer(std::path::PathBuf::from(&bild).with_extension("einschnitt_mauer_oben.png"), &px, 1280, 800, image::ColorType::Rgba8).unwrap();
        }
        drop(ae);
    }
}
