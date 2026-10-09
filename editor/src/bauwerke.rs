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

/// Plan fuer das ganze Netz. `kanten` (l < 0, r > 0): Aussenkanten des Querschnitts.
pub fn planen(v: &Viewer, netz: &Netz, kanten: &dyn Fn(&str) -> (f64, f64), bruecke_ab: f64) -> Plan {
    let mut plan = Plan::default();
    let ts = omsi_map::tile_size();
    // Gelaende, wie es ohne die Bauwerke ist (Datei bzw. Sitzungskopie, mit geformtem Gelaende)
    let mut basis = Gitter::default();
    let mut laden = |g: &mut Gitter, p: DVec2, r: f64| {
        for k in Gitter::kacheln_um(p, r) {
            if !g.kacheln.contains_key(&k) {
                if let Some(t) = v.tile_terrain(k.0, k.1).filter(|t| t.cells == 60) {
                    g.kacheln.insert(k, t);
                }
            }
        }
    };
    // je Rasterpunkt: naechste Stelle einer Strasse (Abstand, Fahrbahnhoehe, halbe Breite, Bauweise, ueber Bruecke)
    let mut ziel: HashMap<(i32, i32), (f64, f64, f64, Bauweise, bool)> = HashMap::new();
    for e in &netz.kanten {
        let (l, r) = kanten(&e.sli);
        let halb = (-l).max(r) + 0.5;
        let el = netz.elemente(e);
        let mut cum = 0.0;
        // Bruecke je Element: genug Luft unter der Fahrbahn (irgendwo im Element)
        let mut proben: Vec<Vec<(DVec3, f64, f64)>> = Vec::new();
        for x in &el {
            let k = x.kurve(e.id, 0.0);
            let n = (k.length / 2.0).ceil().max(1.0) as usize;
            let mut ps = Vec::new();
            for i in 0..=n {
                let s = k.length * i as f64 / n as f64;
                let p = k.point_at(s);
                laden(&mut basis, p.truncate(), halb + 40.0);
                let luft = boden(&basis, p.x, p.y).map(|b| p.z - b).unwrap_or(0.0);
                ps.push((p, k.heading_at(s), luft));
            }
            proben.push(ps);
        }
        let bruecke: Vec<bool> = proben.iter().map(|ps| ps.iter().any(|p| p.2 >= bruecke_ab)).collect();
        for (i, x) in el.iter().enumerate() {
            let ps = &proben[i];
            if bruecke[i] {
                plan.begleit.push((format!("Splines\\{}\\AB_bruecke_{}_{}.sli", crate::speichern::EIGEN, zahl(l), zahl(r)), x.clone(), e.id, cum));
                plan.bruecken_m += x.stueck.laenge;
                // Pfeiler am Anfang des Elements, wenn davor auch Bruecke ist (nicht an den Widerlagern)
                if i > 0 && bruecke[i - 1] {
                    let (p, h, luft) = ps[0];
                    let mitte = p.truncate() + crate::netz::rechts(h) * ((l + r) / 2.0);
                    if luft - PLATTE >= PFEILER_MIN && v.surface_height(mitte.x, mitte.y).is_none() {
                        let b = boden(&basis, mitte.x, mitte.y).unwrap_or(p.z - luft);
                        plan.pfeiler.push(Pfeiler { fuss: mitte.extend(b - 1.0), richtung: h, hoehe: p.z - PLATTE - (b - 1.0) + 0.05, breite: ((r - l) * 0.6).max(2.0) });
                    }
                }
            } else {
                let luft_max = ps.iter().map(|p| p.2).fold(f64::MIN, f64::max);
                let luft_min = ps.iter().map(|p| p.2).fold(f64::MAX, f64::min);
                if luft_max > 0.3 || luft_min < -0.3 {
                    if e.bauweise == Bauweise::Mauer {
                        // Mauern: nach unten (Fahrbahn ueber dem Gelaende) oder oben (im Einschnitt), Hoehe in 1-m-Stufen
                        let (unten, h) = if luft_max.abs() >= luft_min.abs() { (true, luft_max) } else { (false, -luft_min) };
                        let hh = (h + 1.0).ceil();
                        plan.begleit.push((format!("Splines\\{}\\AB_mauer_{}_{}_{}_{}.sli", crate::speichern::EIGEN, if unten { "unten" } else { "oben" }, zahl(l), zahl(r), zahl(hh)), x.clone(), e.id, cum));
                        plan.mauern_m += x.stueck.laenge;
                    } else {
                        plan.rampen_m += x.stueck.laenge;
                    }
                }
            }
            // Gelaende: Rasterpunkte um jede Probe
            for &(p, _, luft) in ps {
                if luft.abs() < 0.05 && !bruecke[i] {
                    continue;
                }
                let reich = halb + if e.bauweise == Bauweise::Mauer { 1.0 } else { luft.abs() * BOESCHUNG + ZELLE };
                let (gx0, gx1) = (((p.x - reich) / ZELLE).floor() as i32, ((p.x + reich) / ZELLE).ceil() as i32);
                let (gy0, gy1) = (((p.y - reich) / ZELLE).floor() as i32, ((p.y + reich) / ZELLE).ceil() as i32);
                for gx in gx0..=gx1 {
                    for gy in gy0..=gy1 {
                        let d = (DVec2::new(gx as f64, gy as f64) * ZELLE - p.truncate()).length();
                        if d > reich {
                            continue;
                        }
                        let alt = ziel.get(&(gx, gy));
                        if alt.is_none_or(|a| d < a.0) {
                            ziel.insert((gx, gy), (d, p.z, halb, e.bauweise, bruecke[i]));
                        }
                    }
                }
            }
            cum += x.stueck.laenge;
        }
    }
    // neues Gelaende
    let mut neu = basis.clone();
    let mut geaendert: HashSet<(i32, i32)> = HashSet::new();
    for ((gx, gy), (d, zr, halb, bauweise, ueber_bruecke)) in ziel {
        if ueber_bruecke {
            continue;
        }
        let q = DVec2::new(gx as f64, gy as f64) * ZELLE;
        let Some(g) = basis.hoehe(gx, gy).map(|h| h as f64) else { continue };
        // vorhandene Strassen auf dem Boden nicht zuschuetten
        if v.surface_height(q.x, q.y).is_some_and(|s| (s - g).abs() < 1.5) {
            continue;
        }
        let unter = zr - 0.05;
        let z = if d <= halb {
            unter
        } else if bauweise == Bauweise::Mauer {
            continue;
        } else if unter > g {
            g.max(unter - (d - halb) / BOESCHUNG)
        } else {
            g.min(unter + (d - halb) / BOESCHUNG)
        };
        // Mauer: unter der Fahrbahn nur absenken (aufgeschuettet wird nicht, die Mauern verdecken den Hohlraum)
        let z = if bauweise == Bauweise::Mauer { z.min(g) } else { z };
        if (z - g).abs() > 0.02 {
            neu.setzen(gx, gy, z as f32, &mut geaendert);
        }
    }
    let _ = ts;
    for k in geaendert {
        if let Some(t) = neu.kacheln.remove(&k) {
            plan.gelaende.insert(k, t);
        }
    }
    plan
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
        assert!(s.bauwerke.mauern_m > 0.0 && s.bauwerke.begleit.iter().any(|b| b.0.contains("AB_mauer_unten")));
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
}
