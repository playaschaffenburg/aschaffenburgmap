//! Uebergaenge zwischen Querschnitten: ein Objekt (.sco + .x-Modell + Fahrpfade) wie die Kreuzungsobjekte, das auf
//! einer Laenge L den einen Querschnitt in den anderen ueberfuehrt - eine OMSI-Spline hat ja ueber ihre ganze Laenge
//! dasselbe Profil.
//!
//! - Die Teile beider Seiten werden einander zugeordnet (gleiche Art bevorzugt, gleiche Hoehe Pflicht; was keine
//!   Entsprechung hat, laeuft auf Breite 0 aus - aussen eher als innen, wie eine endende rechte Spur).
//! - Breiten und Lage gehen weich ineinander ueber (S-foermig, smoothstep ueber L).
//! - Modell: Belaege gekachelt, Bordsteinkanten an jedem Hoehenwechsel, Markierungen (Alpha-Test) wie im Baukasten.
//! - Pfade: jede Fahrspur einer Richtung wird ihrer Partnerin auf der anderen Seite zugeordnet (von der Gegenfahrbahn
//!   aus gezaehlt); zusaetzliche Spuren faedeln in die naechste ein bzw. zweigen von ihr ab (S-Kurve aus zwei
//!   Boegen). Gehwege laufen durch. Die Pfadenden liegen genau auf den Spuren der Splines (die KI verbindet sie).
//! Objektkoordinaten: x rechts, y vorwaerts (Seite A bei y = 0, Seite B bei y = L), Hoehe absolut ([absheight]).

use crate::querschnitt::{auto_marke, Art, Belag, Marke, Querschnitt, Richtung, Teil, FAHRBAHN};
use anyhow::{bail, Context, Result};
use std::path::Path;

/// Fahr- oder Gehwegpfad eines Querschnitts: Art (0 Fahrzeug, 1 Fussgaenger), Querlage, Hoehe, Breite, Richtung (0 mit,
/// 1 gegen den Spline, 2 beide)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pfad {
    pub art: u8,
    pub x: f64,
    pub h: f64,
    pub breite: f64,
    pub richtung: u8,
}

/// eine Seite des Uebergangs: Teile von links nach rechts ab `links`, und die Pfade der Spline dort
#[derive(Clone, Debug)]
pub struct Seite {
    pub teile: Vec<Teil>,
    pub links: f64,
    pub pfade: Vec<Pfad>,
}

impl Seite {
    pub fn aus_querschnitt(q: &Querschnitt) -> Seite {
        let links = q.links();
        let mut x = links;
        let mut pfade = Vec::new();
        for t in &q.teile {
            let m = x + t.breite / 2.0;
            match t.art {
                Art::Fahrspur | Art::Busspur => pfade.push(Pfad { art: 0, x: m, h: t.hoehe(), breite: t.breite, richtung: if t.richtung == Richtung::Vor { 0 } else { 1 } }),
                Art::Gehweg => pfade.push(Pfad { art: 1, x: m, h: t.hoehe(), breite: t.breite * 0.6, richtung: 2 }),
                _ => {}
            }
            x += t.breite;
        }
        Seite { teile: q.teile.clone(), links, pfade }
    }

    /// aus einer beliebigen Spline (Standard- oder fremde Strassen): Flaechen aus den [heightprofile]s (Fahrbahn bis
    /// 0,17 m, darueber Hochbord), Fahrbahn an den Fahrspuren geteilt, Gehweg wo ein Fussgaengerpfad liegt; Pfade wie
    /// in der Spline
    pub fn aus_sli(def: &omsi_scenery::Spline) -> Option<Seite> {
        let pfade: Vec<Pfad> = def.paths.iter().filter(|p| (0..=1).contains(&p.kind)).map(|p| Pfad {
            art: p.kind as u8, x: p.start[0] as f64, h: p.start[2] as f64, breite: p.width as f64, richtung: p.direction.clamp(0, 2) as u8,
        }).collect();
        let mut flaechen: Vec<(f64, f64, f64)> = def.height_profiles.iter().map(|h| (h.x0.min(h.x1) as f64, h.x0.max(h.x1) as f64, h.z0.max(h.z1) as f64)).collect();
        if flaechen.is_empty() {
            // ohne Hoehenprofil: Fahrbahn ueber die ganze gezeichnete Breite
            let xs = def.profiles.iter().flat_map(|p| p.points.iter().map(|q| q.x as f64));
            let (lo, hi) = xs.fold((f64::MAX, f64::MIN), |(a, b), x| (a.min(x), b.max(x)));
            if lo >= hi {
                return None;
            }
            flaechen.push((lo, hi, FAHRBAHN));
        }
        flaechen.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut teile = Vec::new();
        let links = flaechen[0].0;
        let mut x = links;
        for (a, b, h) in flaechen {
            if b - a < 0.05 || b <= x + 0.01 {
                continue;
            }
            if a > x + 0.05 {
                // Luecke zwischen den Flaechen: Gruen
                teile.push(Teil { art: Art::Gruen, breite: a - x, richtung: Richtung::Vor, belag: Belag::Gras });
            }
            let a = a.max(x);
            if h <= 0.17 {
                let mut spuren: Vec<&Pfad> = pfade.iter().filter(|p| p.art == 0 && p.x > a && p.x < b).collect();
                spuren.sort_by(|p, q| p.x.total_cmp(&q.x));
                if spuren.is_empty() {
                    teile.push(Teil { art: Art::Parkstreifen, breite: b - a, richtung: Richtung::Vor, belag: Belag::Asphalt });
                } else {
                    let mut lx = a;
                    for (i, p) in spuren.iter().enumerate() {
                        let rx = if i + 1 < spuren.len() { (p.x + spuren[i + 1].x) / 2.0 } else { b };
                        teile.push(Teil { art: Art::Fahrspur, breite: rx - lx, richtung: if p.richtung == 1 { Richtung::Zurueck } else { Richtung::Vor }, belag: Belag::Asphalt });
                        lx = rx;
                    }
                }
            } else {
                let geh = pfade.iter().any(|p| p.art == 1 && p.x > a && p.x < b);
                let (art, belag) = if geh || b - a < 1.0 { (Art::Gehweg, Belag::Platten) } else { (Art::Gruen, Belag::Gras) };
                teile.push(Teil { art, breite: b - a, richtung: Richtung::Vor, belag });
            }
            x = b;
        }
        if !teile.iter().any(|t| t.art.spur()) {
            return None;
        }
        Some(Seite { teile, links, pfade })
    }

    /// von der anderen Seite gesehen (Spline in Gegenrichtung befahren / gespiegelt)
    pub fn gedreht(&self) -> Seite {
        let breite: f64 = self.teile.iter().map(|t| t.breite).sum();
        let teile = self.teile.iter().rev().map(|t| Teil { richtung: if t.richtung == Richtung::Vor { Richtung::Zurueck } else { Richtung::Vor }, ..t.clone() }).collect();
        let pfade = self.pfade.iter().map(|p| Pfad { x: -p.x, richtung: match p.richtung { 0 => 1, 1 => 0, r => r }, ..*p }).collect();
        Seite { teile, links: -(self.links + breite), pfade }
    }

    pub fn breite(&self) -> f64 {
        self.teile.iter().map(|t| t.breite).sum()
    }
}

/// ein Streifen des Uebergangs: Teil auf Seite A und/oder B
#[derive(Clone, Debug)]
struct Streifen {
    a: Option<Teil>,
    b: Option<Teil>,
}

impl Streifen {
    fn teil(&self, t: f64) -> &Teil {
        match (&self.a, &self.b) {
            (Some(a), Some(b)) => if t < 0.5 { a } else { b },
            (Some(a), None) => a,
            (None, Some(b)) => b,
            _ => unreachable!(),
        }
    }

    fn hoehe(&self) -> f64 {
        self.a.as_ref().or(self.b.as_ref()).map(|t| t.hoehe()).unwrap_or(FAHRBAHN)
    }

    fn breite(&self, s: f64) -> f64 {
        let wa = self.a.as_ref().map(|t| t.breite).unwrap_or(0.0);
        let wb = self.b.as_ref().map(|t| t.breite).unwrap_or(0.0);
        wa + (wb - wa) * s
    }
}

/// Teile beider Seiten zuordnen (Ausrichtung wie bei einem Textvergleich): gleiche Art kostet nichts, andere Art
/// gleicher Hoehe etwas, verschiedene Hoehen gehen nicht; ein Teil ohne Partner kostet aussen weniger als innen
fn zuordnen(a: &[Teil], b: &[Teil]) -> Vec<Streifen> {
    let (n, m) = (a.len(), b.len());
    let aussen = |i: usize, len: usize| -> f64 {
        let mitte = (len as f64 - 1.0) / 2.0;
        (i as f64 - mitte).abs() / len.max(1) as f64
    };
    let lueck_a = |i: usize| 1.0 - 0.3 * aussen(i, n);
    let lueck_b = |j: usize| 1.0 - 0.3 * aussen(j, m);
    let passt = |i: usize, j: usize| -> Option<f64> {
        let (x, y) = (&a[i], &b[j]);
        if (x.hoehe() - y.hoehe()).abs() > 0.01 {
            return None;
        }
        // Fahrspuren nur gleicher Richtung
        if x.art.spur() && y.art.spur() && x.richtung != y.richtung {
            return None;
        }
        Some(if x.art == y.art { 0.0 } else if x.art.spur() == y.art.spur() { 0.6 } else { 1.5 })
    };
    let mut k = vec![vec![f64::INFINITY; m + 1]; n + 1];
    k[0][0] = 0.0;
    for i in 0..=n {
        for j in 0..=m {
            let c = k[i][j];
            if !c.is_finite() {
                continue;
            }
            if i < n {
                k[i + 1][j] = k[i + 1][j].min(c + lueck_a(i));
            }
            if j < m {
                k[i][j + 1] = k[i][j + 1].min(c + lueck_b(j));
            }
            if i < n && j < m {
                if let Some(p) = passt(i, j) {
                    k[i + 1][j + 1] = k[i + 1][j + 1].min(c + p);
                }
            }
        }
    }
    // zurueckverfolgen
    let (mut i, mut j) = (n, m);
    let mut out = Vec::new();
    while i > 0 || j > 0 {
        let c = k[i][j];
        if i > 0 && j > 0 && passt(i - 1, j - 1).is_some_and(|p| (k[i - 1][j - 1] + p - c).abs() < 1e-9) {
            out.push(Streifen { a: Some(a[i - 1].clone()), b: Some(b[j - 1].clone()) });
            i -= 1;
            j -= 1;
        } else if i > 0 && (k[i - 1][j] + lueck_a(i - 1) - c).abs() < 1e-9 {
            out.push(Streifen { a: Some(a[i - 1].clone()), b: None });
            i -= 1;
        } else {
            out.push(Streifen { a: None, b: Some(b[j - 1].clone()) });
            j -= 1;
        }
    }
    out.reverse();
    out
}

fn glatt(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// S-Kurve als Pfadelemente (x, y, Richtung in Grad, Radius (> 0 rechts), Laenge) von (xa, 0) nach (xb, L)
fn s_kurve(xa: f64, xb: f64, laenge: f64) -> Vec<[f64; 5]> {
    let d = xb - xa;
    if d.abs() < 0.02 {
        return vec![[xa, 0.0, 0.0, 0.0, laenge]];
    }
    let theta = 2.0 * (d.abs() / laenge).atan();
    let r = (laenge / 2.0) / theta.sin();
    let bogen = r * theta;
    let rechts = d > 0.0;
    let grad = theta.to_degrees();
    vec![
        [xa, 0.0, 0.0, if rechts { r } else { -r }, bogen],
        [(xa + xb) / 2.0, laenge / 2.0, if rechts { grad } else { -grad }, if rechts { -r } else { r }, bogen],
    ]
}

/// Pfade beider Seiten paaren: je Gruppe (Fahrspuren je Richtung, Gehwege) von der Mitte bzw. Gegenfahrbahn aus; was
/// uebrig bleibt, schliesst an die naechste gepaarte an. -> (xa, xb, Pfad der Zielseite)
fn pfade_paaren(a: &[Pfad], b: &[Pfad]) -> Vec<(f64, f64, Pfad)> {
    let mut out = Vec::new();
    for (art, richtung) in [(0u8, 0u8), (0, 1), (0, 2), (1, 2)] {
        let mut pa: Vec<&Pfad> = a.iter().filter(|p| p.art == art && (art == 1 || p.richtung == richtung)).collect();
        let mut pb: Vec<&Pfad> = b.iter().filter(|p| p.art == art && (art == 1 || p.richtung == richtung)).collect();
        if pa.is_empty() || pb.is_empty() {
            continue;
        }
        if art == 1 {
            // Gehwege: links mit links, rechts mit rechts
            for seite in [-1.0f64, 1.0] {
                let ga: Vec<&&Pfad> = pa.iter().filter(|p| p.x.signum() == seite || (p.x == 0.0 && seite > 0.0)).collect();
                let gb: Vec<&&Pfad> = pb.iter().filter(|p| p.x.signum() == seite || (p.x == 0.0 && seite > 0.0)).collect();
                for (x, y) in ga.iter().zip(gb.iter()) {
                    out.push((x.x, y.x, ***y));
                }
            }
            continue;
        }
        // von der Gegenfahrbahn aus: "vor" (rechts) von links nach rechts, "zurueck" von rechts nach links
        pa.sort_by(|p, q| p.x.total_cmp(&q.x));
        pb.sort_by(|p, q| p.x.total_cmp(&q.x));
        if richtung == 1 {
            pa.reverse();
            pb.reverse();
        }
        let n = pa.len().min(pb.len());
        for k in 0..n {
            out.push((pa[k].x, pb[k].x, *pb[k]));
        }
        // zusaetzliche auf A faedeln in die letzte gepaarte von B ein, zusaetzliche auf B zweigen von der letzten von A ab
        for p in pa.iter().skip(n) {
            out.push((p.x, pb[n - 1].x, Pfad { breite: p.breite, ..*pb[n - 1] }));
        }
        for p in pb.iter().skip(n) {
            out.push((pa[n - 1].x, p.x, **p));
        }
    }
    out
}

/// Modell: Eckpunkte (x, y, z, nx, ny, nz, u, v) in .x-Koordinaten (x rechts, y hoch, z vorwaerts) und Dreiecke
/// (a, b, c, Material)
#[derive(Default)]
pub struct Modell {
    pub ecken: Vec<[f64; 8]>,
    pub dreiecke: Vec<[usize; 4]>,
    pub materialien: Vec<String>,
}

impl Modell {
    fn material(&mut self, datei: &str) -> usize {
        match self.materialien.iter().position(|m| m == datei) {
            Some(i) => i,
            None => {
                self.materialien.push(datei.to_string());
                self.materialien.len() - 1
            }
        }
    }

    /// Viereck p0 p1 p2 p3 (umlaufend) mit uv, sichtbar zur Seite `n` (Objektkoordinaten x rechts, y vor, z hoch)
    fn viereck(&mut self, mat: usize, p: [[f64; 3]; 4], uv: [[f64; 2]; 4], n: [f64; 3]) {
        // in .x-Koordinaten (x, z hoch -> y, y vor -> z)
        let q: Vec<[f64; 3]> = p.iter().map(|a| [a[0], a[2], a[1]]).collect();
        let nn = [n[0], n[2], n[1]];
        let i0 = self.ecken.len();
        for k in 0..4 {
            self.ecken.push([q[k][0], q[k][1], q[k][2], nn[0], nn[1], nn[2], uv[k][0], uv[k][1]]);
        }
        // Vorderseite: (b - a) x (c - a) zeigt zur Normalen
        let kreuz = |a: [f64; 3], b: [f64; 3], c: [f64; 3]| {
            let (u, v) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
            [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]]
        };
        let c = kreuz(q[0], q[1], q[2]);
        let vorne = c[0] * nn[0] + c[1] * nn[1] + c[2] * nn[2] >= 0.0;
        let (a, b, cc, d) = (i0, i0 + 1, i0 + 2, i0 + 3);
        if vorne {
            self.dreiecke.push([a, b, cc, mat]);
            self.dreiecke.push([a, cc, d, mat]);
        } else {
            self.dreiecke.push([a, cc, b, mat]);
            self.dreiecke.push([a, d, cc, mat]);
        }
    }

    pub fn x_datei(&self) -> String {
        let f = |v: f64| format!("{v:.4}");
        let (nv, nf) = (self.ecken.len(), self.dreiecke.len());
        let ende = |i: usize, n: usize| if i + 1 < n { "," } else { ";" };
        let mut l = vec!["xof 0302txt 0032".to_string(), String::new(), "Mesh Uebergang {".into(), format!(" {nv};")];
        l.extend(self.ecken.iter().enumerate().map(|(i, e)| format!(" {};{};{};{}", f(e[0]), f(e[1]), f(e[2]), ende(i, nv))));
        l.push(format!(" {nf};"));
        l.extend(self.dreiecke.iter().enumerate().map(|(i, d)| format!(" 3;{},{},{};{}", d[0], d[1], d[2], ende(i, nf))));
        l.extend([" MeshNormals {".into(), format!("  {nv};")]);
        l.extend(self.ecken.iter().enumerate().map(|(i, e)| format!("  {};{};{};{}", f(e[3]), f(e[4]), f(e[5]), ende(i, nv))));
        l.push(format!("  {nf};"));
        l.extend(self.dreiecke.iter().enumerate().map(|(i, d)| format!("  3;{},{},{};{}", d[0], d[1], d[2], ende(i, nf))));
        l.extend([" }".into(), " MeshTextureCoords {".into(), format!("  {nv};")]);
        l.extend(self.ecken.iter().enumerate().map(|(i, e)| format!("  {};{};{}", f(e[6]), f(-e[7]), ende(i, nv))));
        l.extend([" }".into(), " MeshMaterialList {".into(), format!("  {};", self.materialien.len()), format!("  {nf};")]);
        l.extend(self.dreiecke.iter().enumerate().map(|(i, d)| format!("  {}{}", d[3], ende(i, nf))));
        for (i, m) in self.materialien.iter().enumerate() {
            l.extend([format!("  Material M{i} {{"), "   1.000000;1.000000;1.000000;1.000000;;".into(), "   0.000000;".into(),
                      "   0.000000;0.000000;0.000000;;".into(), "   0.000000;0.000000;0.000000;;".into(),
                      format!("   TextureFilename {{ \"{m}\"; }}"), "  }".into()]);
        }
        l.extend([" }".into(), "}".into(), String::new()]);
        l.join("\r\n")
    }
}

/// der fertige Uebergang
pub struct Uebergang {
    pub modell: Modell,
    /// Pfadelemente: (x, y, Hoehe, Richtung, Radius, Laenge, Art, Breite, Fahrtrichtung)
    pub pfade: Vec<([f64; 6], u8, f64, u8)>,
    pub laenge: f64,
    pub breite: (f64, f64),
}

/// Laenge, die der Uebergang mindestens braucht: je Meter seitlichem Versatz einer Spur 8 m, wenigstens 15 m
pub fn laenge_vorschlag(a: &Seite, b: &Seite) -> f64 {
    let versatz = pfade_paaren(&a.pfade, &b.pfade).iter().map(|(xa, xb, _)| (xb - xa).abs()).fold(0.0, f64::max);
    let breiten = (a.links - b.links).abs().max((a.links + a.breite() - b.links - b.breite()).abs());
    ((versatz.max(breiten * 0.7) * 8.0).max(15.0) / 5.0).ceil() * 5.0
}

pub fn bauen(a: &Seite, b: &Seite, laenge: f64) -> Result<Uebergang> {
    if laenge < 5.0 {
        bail!("Uebergang zu kurz ({laenge:.1} m)");
    }
    let streifen = zuordnen(&a.teile, &b.teile);
    let mut m = Modell::default();
    let schritte = ((laenge / 2.0).ceil() as usize).max(4);
    let lage = |t: f64| -> Vec<(f64, f64)> {
        let s = glatt(t);
        let mut x = a.links + (b.links - a.links) * s;
        streifen.iter().map(|st| {
            let w = st.breite(s);
            let r = (x, x + w);
            x += w;
            r
        }).collect()
    };
    let bord = m.material("str_side1.bmp");
    for k in 0..schritte {
        let (t0, t1) = (k as f64 / schritte as f64, (k + 1) as f64 / schritte as f64);
        let (y0, y1) = (laenge * t0, laenge * t1);
        let (l0, l1) = (lage(t0), lage(t1));
        let tm = (t0 + t1) / 2.0;
        for (i, st) in streifen.iter().enumerate() {
            let h = st.hoehe();
            let teil = st.teil(tm);
            let (a0, b0) = l0[i];
            let (a1, b1) = l1[i];
            if (b0 - a0) < 1e-4 && (b1 - a1) < 1e-4 {
                continue;
            }
            // Bordsteine zu tieferen Nachbarn
            let tiefer = |j: Option<usize>| j.and_then(|j| streifen.get(j)).map(|n| n.hoehe() < h - 1e-6).unwrap_or(false);
            let links_tiefer = tiefer(i.checked_sub(1)) || (i == 0 && h > FAHRBAHN);
            let rechts_tiefer = tiefer(Some(i + 1)) || (i + 1 == streifen.len() && h > FAHRBAHN);
            let unten_l = if i == 0 { 0.0 } else { FAHRBAHN };
            let unten_r = if i + 1 == streifen.len() { 0.0 } else { FAHRBAHN };
            let (mut fa0, mut fa1, mut fb0, mut fb1) = (a0, a1, b0, b1);
            if links_tiefer && h > FAHRBAHN {
                m.viereck(bord, [[a0, y0, unten_l], [a0, y0, h], [a1, y1, h], [a1, y1, unten_l]],
                          [[0.995, y0 * 0.2], [0.953, y0 * 0.2], [0.953, y1 * 0.2], [0.995, y1 * 0.2]], [-1.0, 0.0, 0.0]);
                fa0 = (a0 + 0.15).min((a0 + b0) / 2.0);
                fa1 = (a1 + 0.15).min((a1 + b1) / 2.0);
                m.viereck(bord, [[a0, y0, h], [fa0, y0, h], [fa1, y1, h], [a1, y1, h]],
                          [[0.953, y0 * 0.2], [0.92, y0 * 0.2], [0.92, y1 * 0.2], [0.953, y1 * 0.2]], [0.0, 0.0, 1.0]);
            }
            if rechts_tiefer && h > FAHRBAHN {
                m.viereck(bord, [[b0, y0, h], [b0, y0, unten_r], [b1, y1, unten_r], [b1, y1, h]],
                          [[0.953, y0 * 0.2], [0.995, y0 * 0.2], [0.995, y1 * 0.2], [0.953, y1 * 0.2]], [1.0, 0.0, 0.0]);
                fb0 = (b0 - 0.15).max(fa0);
                fb1 = (b1 - 0.15).max(fa1);
                m.viereck(bord, [[fb0, y0, h], [b0, y0, h], [b1, y1, h], [fb1, y1, h]],
                          [[0.92, y0 * 0.2], [0.953, y0 * 0.2], [0.953, y1 * 0.2], [0.92, y1 * 0.2]], [0.0, 0.0, 1.0]);
            }
            // Belag in Spalten (wie in der Spline gekachelt)
            let tx = teil.belag.textur();
            let mat = m.material(tx.datei);
            let spanne = (tx.u.1 - tx.u.0) * tx.m_je_u;
            let spalten = (((fb0 - fa0).max(fb1 - fa1)) / spanne).ceil().max(1.0) as usize;
            for c in 0..spalten {
                let (c0, c1) = (c as f64 / spalten as f64, (c + 1) as f64 / spalten as f64);
                let xa0 = fa0 + (fb0 - fa0) * c0;
                let xb0 = fa0 + (fb0 - fa0) * c1;
                let xa1 = fa1 + (fb1 - fa1) * c0;
                let xb1 = fa1 + (fb1 - fa1) * c1;
                let u0 = tx.u.0;
                let ub0 = (tx.u.0 + (xb0 - xa0) / tx.m_je_u).min(tx.u.1);
                let ub1 = (tx.u.0 + (xb1 - xa1) / tx.m_je_u).min(tx.u.1);
                m.viereck(mat, [[xa0, y0, h], [xb0, y0, h], [xb1, y1, h], [xa1, y1, h]],
                          [[u0, y0 * tx.v_je_m], [ub0, y0 * tx.v_je_m], [ub1, y1 * tx.v_je_m], [u0, y1 * tx.v_je_m]], [0.0, 0.0, 1.0]);
            }
        }
        // Markierungen zwischen Teilen auf Fahrbahnhoehe, solange beide breit genug sind
        for i in 0..streifen.len().saturating_sub(1) {
            let (s1, s2) = (&streifen[i], &streifen[i + 1]);
            if s1.hoehe() > FAHRBAHN + 0.01 || s2.hoehe() > FAHRBAHN + 0.01 {
                continue;
            }
            let breit = |l: &Vec<(f64, f64)>, j: usize| l[j].1 - l[j].0 > 0.8;
            if !(breit(&l0, i) && breit(&l0, i + 1) && breit(&l1, i) && breit(&l1, i + 1)) {
                continue;
            }
            let marke = auto_marke(s1.teil(tm), s2.teil(tm));
            let Some((datei, w, v)) = (match marke { Marke::Keine | Marke::Auto => None, mm => mm.textur() }) else { continue };
            let mat = m.material(datei);
            let (x0, x1) = (l0[i].1, l1[i].1);
            let z = FAHRBAHN + 0.01;
            m.viereck(mat, [[x0 - w / 2.0, y0, z], [x0 + w / 2.0, y0, z], [x1 + w / 2.0, y1, z], [x1 - w / 2.0, y1, z]],
                      [[0.0, y0 * v], [1.0, y0 * v], [1.0, y1 * v], [0.0, y1 * v]], [0.0, 0.0, 1.0]);
        }
    }
    // Pfade
    let mut pfade = Vec::new();
    for (xa, xb, p) in pfade_paaren(&a.pfade, &b.pfade) {
        for e in s_kurve(xa, xb, laenge) {
            pfade.push(([e[0], e[1], p.h, e[2], e[3], e[4]], p.art, p.breite.min(3.5), p.richtung));
        }
    }
    if !pfade.iter().any(|p| p.1 == 0) {
        bail!("keine Fahrspuren, die sich verbinden lassen (Richtungen passen nicht?)");
    }
    Ok(Uebergang { modell: m, pfade, laenge, breite: (a.breite(), b.breite()) })
}

impl Uebergang {
    /// .sco-Text (Modell `modell`, Markierungen mit Alpha-Test)
    pub fn sco(&self, name: &str, modell: &str) -> String {
        let mut l = vec!["Erzeugt mit omsi-editor (Uebergang zwischen Querschnitten)".to_string(), String::new(), "[friendlyname]".into(), name.into(), String::new(),
                         "[groups]".into(), "1".into(), crate::speichern::EIGEN.into(), String::new(), "[rendertype]".into(), "surface".into(), String::new(),
                         "[LightMapMapping]".into(), String::new(), "[fixed]".into(), String::new(), "[surface]".into(), String::new(), "[absheight]".into(), String::new()];
        for (e, art, breite, richtung) in &self.pfade {
            l.push("[path]".into());
            for v in [e[0], e[1], e[2], e[3].rem_euclid(360.0), e[4], e[5]] {
                l.push(format!("{v:.4}"));
            }
            l.extend(["0".into(), "0".into(), art.to_string(), format!("{breite:.4}"), richtung.to_string(), "0".into(), String::new()]);
        }
        l.extend(["[mesh]".into(), modell.into(), String::new()]);
        for m in &self.modell.materialien {
            if m.ends_with(".tga") {
                l.extend(["[matl]".into(), m.clone(), "0".into(), "[matl_alpha]".into(), "1".into(), String::new()]);
            }
        }
        l.join("\r\n") + "\r\n"
    }

    /// Objekt schreiben: `ordner`\<name>.sco, model\<name>.x, texture\ (aus Splines\Aschaffenburg\texture) -> .sco-Datei
    pub fn schreiben(&self, root: &Path, ordner: &Path, name: &str, titel: &str) -> Result<std::path::PathBuf> {
        std::fs::create_dir_all(ordner.join("model"))?;
        std::fs::create_dir_all(ordner.join("texture"))?;
        let texturen: Vec<&str> = self.modell.materialien.iter().map(|s| s.as_str()).collect();
        crate::querschnitt::texturen_bereitstellen(root, &texturen)?;
        let quelle = crate::querschnitt::ordner(root).join("texture");
        for t in &texturen {
            for ext in ["", ".cfg", ".surf"] {
                let q = quelle.join(format!("{t}{ext}"));
                if q.is_file() {
                    std::fs::copy(&q, ordner.join("texture").join(format!("{t}{ext}"))).with_context(|| format!("Textur {t} kopieren"))?;
                }
            }
        }
        std::fs::write(ordner.join("model").join(format!("{name}.x")), self.modell.x_datei())?;
        let sco = ordner.join(format!("{name}.sco"));
        let text = self.sco(titel, &format!("{name}.x"));
        let ansi: Vec<u8> = text.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).collect();
        std::fs::write(&sco, ansi)?;
        Ok(sco)
    }
}

/// Seite eines Querschnitts (.sli relativ zum OMSI-Ordner): aus der Bauanleitung des Baukastens, sonst aus der Spline
pub fn seite(root: &Path, sli: &str) -> Result<Seite> {
    let pfad = omsi_cfg::resolve_path(root, sli);
    if let Some(q) = std::fs::read(pfad.with_extension("qs.json")).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| Querschnitt::aus_json(&v)) {
        return Ok(Seite::aus_querschnitt(&q));
    }
    let def = omsi_scenery::Spline::load(&pfad).map_err(|e| anyhow::anyhow!("{sli}: {e}"))?;
    Seite::aus_sli(&def).with_context(|| format!("{sli}: keine Fahrspuren erkannt"))
}

/// was `setzen` braucht: die Stelle (Spline-Ende) und wie die Strasse dort liegt
pub struct Stelle {
    pub pos: glam::DVec3,
    /// Richtung, in der es weitergeht (vom vorhandenen Spline weg)
    pub richtung: f64,
    pub sli: String,
    /// faehrt man dort in Splinerichtung weiter (Spline-Ende, nicht gespiegelt)
    pub gleichsinnig: bool,
}

/// Uebergang planen: (Seite A, Seite B, Laenge)
pub fn planen(root: &Path, st: &Stelle, ziel_sli: &str, laenge: Option<f64>) -> Result<(Seite, Seite, f64)> {
    let a = seite(root, &st.sli)?;
    let a = if st.gleichsinnig { a } else { a.gedreht() };
    let b = seite(root, ziel_sli)?;
    let l = laenge.unwrap_or_else(|| laenge_vorschlag(&a, &b));
    Ok((a, b, l))
}

/// Uebergang an der Stelle setzen: Objekt im Sitzungsordner der Kreuzungsobjekte, Eintrag in der Kachel (ein
/// Rueckgaengig-Schritt des Aendern-Werkzeugs) -> Meldung
pub fn setzen(v: &mut openomsi_game::viewer::Viewer, ae: &mut crate::aendern::Aendern, root: &Path, st: &Stelle, ziel_sli: &str, laenge: Option<f64>) -> Result<String> {
    let (a, b, l) = planen(root, st, ziel_sli, laenge)?;
    let u = bauen(&a, &b, l)?;
    let (ordner, tag) = ae.kreuzungs_ordner();
    let nr = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() % 1_000_000).unwrap_or(0);
    let name = format!("UE_{nr:06}");
    let kurz = |s: &str| Path::new(s).file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    u.schreiben(root, &ordner, &name, &format!("Uebergang {} -> {}", kurz(&st.sli), kurz(ziel_sli)))?;
    let rel = format!("Sceneryobjects\\{}\\{tag}\\{name}.sco", crate::speichern::EIGEN);
    let (id, _) = ae.objekt_anlegen_hoehe(v, &rel, st.pos.truncate(), st.pos.z, st.richtung, &[])?;
    log::info!("Uebergang {rel} (Objekt {id}): {} -> {}, {l:.0} m, {} Pfadelemente", st.sli, ziel_sli, u.pfade.len());
    Ok(format!("Uebergang gesetzt: {} -> {} auf {l:.0} m - am freien Ende mit dem neuen Querschnitt weiterbauen", kurz(&st.sli), kurz(ziel_sli)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(teile: &[(Art, Richtung)]) -> Querschnitt {
        let mut q = Querschnitt { name: "t".into(), teile: teile.iter().map(|(a, r)| Teil { richtung: *r, ..Teil::neu(*a) }).collect(), marken: vec![] };
        q.marken_anpassen();
        q
    }

    use Art::*;
    use Richtung::*;

    /// 1+1 -> 2+1: die zweite Spur "vor" zweigt rechts ab; Pfadenden genau auf den Spuren beider Seiten
    #[test]
    fn spur_kommt_dazu() {
        let a = Seite::aus_querschnitt(&q(&[(Gehweg, Vor), (Fahrspur, Zurueck), (Fahrspur, Vor), (Gehweg, Vor)]));
        let b = Seite::aus_querschnitt(&q(&[(Gehweg, Vor), (Fahrspur, Zurueck), (Fahrspur, Vor), (Fahrspur, Vor), (Gehweg, Vor)]));
        let st = zuordnen(&a.teile, &b.teile);
        assert_eq!(st.len(), 5);
        assert!(st[3].a.is_none() && st[3].b.as_ref().unwrap().art == Fahrspur, "die aeussere Spur ist neu");
        let paare = pfade_paaren(&a.pfade, &b.pfade);
        let vor: Vec<_> = paare.iter().filter(|p| p.2.art == 0 && p.2.richtung == 0).collect();
        assert_eq!(vor.len(), 2);
        let ziele: Vec<f64> = b.pfade.iter().filter(|p| p.art == 0 && p.richtung == 0).map(|p| p.x).collect();
        assert!(vor.iter().all(|p| ziele.iter().any(|z| (z - p.1).abs() < 1e-9)));
        assert!(vor.iter().all(|p| (p.0 - a.pfade.iter().find(|q| q.art == 0 && q.richtung == 0).unwrap().x).abs() < 1e-9), "beide aus der einen Spur von A");
        let u = bauen(&a, &b, laenge_vorschlag(&a, &b)).unwrap();
        println!("Laenge {} m, {} Pfadelemente, {} Dreiecke, Materialien {:?}", u.laenge, u.pfade.len(), u.modell.dreiecke.len(), u.modell.materialien);
        assert!(u.laenge >= 15.0);
        // Gehwege laufen durch, Pfad-Enden: S-Kurve endet genau bei xb, y = L
        for (xa, xb, _) in paare {
            let el = s_kurve(xa, xb, u.laenge);
            let ende = el.iter().fold((xa, 0.0, 0.0f64), |(x, y, h), e| {
                let (x0, y0, hd, r, l) = (e[0], e[1], e[2], e[3], e[4]);
                assert!((x0 - x).abs() < 1e-6 && (y0 - y).abs() < 1e-6 && (hd - h).abs() < 1e-6, "Elemente schliessen nicht an");
                if r == 0.0 {
                    (x0 + hd.to_radians().sin() * l, y0 + hd.to_radians().cos() * l, hd)
                } else {
                    let dw = l / r; // Bogenmass, rechts positiv
                    let h1 = hd.to_radians() + dw;
                    (x0 + r * (hd.to_radians().cos() - h1.cos()), y0 + r * (h1.sin() - hd.to_radians().sin()), h1.to_degrees())
                }
            });
            assert!((ende.0 - xb).abs() < 1e-6 && (ende.1 - u.laenge).abs() < 1e-6 && ende.2.abs() < 1e-6, "{ende:?} statt {xb}");
        }
        let x = u.modell.x_datei();
        assert!(x.starts_with("xof 0302txt 0032") && x.contains("TextureFilename"));
        let sco = u.sco("t", "t.x");
        assert_eq!(sco.matches("[path]").count(), u.pfade.len());
    }

    /// Standard-Spline (Marcel, 2 Spuren mit Gehwegen) lesen und umdrehen
    #[test]
    fn aus_sli_marcel() {
        let p = Path::new(crate::bearbeiten::tests::OMSI).join("Splines/Marcel/str_2spur_8m_altonaer1.sli");
        let Ok(def) = omsi_scenery::Spline::load(&p) else { return };
        let s = Seite::aus_sli(&def).unwrap();
        let arten: Vec<Art> = s.teile.iter().map(|t| t.art).collect();
        assert_eq!(arten, vec![Gehweg, Fahrspur, Fahrspur, Gehweg]);
        assert_eq!((s.teile[1].richtung, s.teile[2].richtung), (Zurueck, Vor));
        assert!((s.links + 7.0).abs() < 1e-6 && (s.breite() - 14.0).abs() < 1e-6);
        let g = s.gedreht();
        assert!((g.links + 7.0).abs() < 1e-6);
        assert_eq!((g.teile[1].richtung, g.teile[2].richtung), (Zurueck, Vor));
        // Marcel 2 Spuren -> Baukasten mit Radfahrstreifen: Uebergang geht
        let b = Seite::aus_querschnitt(&q(&[(Gehweg, Vor), (Fahrspur, Zurueck), (Fahrspur, Vor), (Radfahrstreifen, Vor), (Gehweg, Vor)]));
        let u = bauen(&s, &b, 20.0).unwrap();
        assert_eq!(u.pfade.iter().filter(|p| p.1 == 0).count(), 4, "zwei Spuren, je zwei Boegen");
    }

    /// Grundorf: an einem freien Strassenende einen Uebergang auf einen breiteren Querschnitt setzen - die Spuren der
    /// Strasse gehen ins Objekt, am anderen Ende ist ein offener Arm mit den Spuren des neuen Querschnitts (OMSI_BILD)
    #[test]
    #[ignore]
    fn grundorf_uebergang() {
        let _sperre = crate::bearbeiten::tests::sperre();
        let root = Path::new(crate::bearbeiten::tests::OMSI);
        let mut v = crate::bearbeiten::tests::grundorf();
        v.tiles_around(glam::DVec3::new(150.0, 150.0, 0.0), 2).unwrap();
        let mut an = crate::anschluss::Anschluesse::default();
        an.aktualisieren(&v);
        let mut ae = crate::aendern::Aendern::neu(&v);
        ae.aktualisieren(&v);
        // freies Ende schaffen: einen Spline mitten in einer Strasse loeschen (in der Sitzungskopie)
        if !an.liste.iter().any(|a| a.frei && a.objekt.is_none()) {
            let weg = an.liste.iter().filter(|a| !a.frei && a.objekt.is_none() && a.sli.to_lowercase().contains("2spur") && (a.pos.truncate() - glam::DVec2::new(150.0, 150.0)).length() < 250.0)
                .find(|a| ae.spline(a.spline_id).is_some_and(|s| s.prev != 0 && s.next != 0 && s.kurve.length > 20.0)).map(|a| a.spline_id).expect("kein Spline zum Loeschen");
            ae.auswahl = vec![weg];
            ae.loeschen(&mut v).unwrap();
            v.tiles_around(glam::DVec3::new(150.0, 150.0, 0.0), 2).unwrap();
            an = crate::anschluss::Anschluesse::default();
            an.aktualisieren(&v);
            println!("Spline {weg} geloescht");
        }
        let a = an.liste.iter().find(|a| a.frei && a.objekt.is_none()).cloned().unwrap_or_else(|| {
            panic!("kein freies Strassenende: {:?}", an.liste.iter().filter(|a| (a.pos.truncate() - glam::DVec2::new(150.0, 150.0)).length() < 250.0).map(|a| (a.spline_id, a.am_ende, a.frei)).collect::<Vec<_>>())
        });
        println!("freies Ende: Spline {} ({}) bei {:?}", a.spline_id, a.sli, a.pos);
        let st = Stelle { pos: a.pos, richtung: a.richtung, sli: a.sli.clone(), gleichsinnig: a.am_ende != a.gespiegelt };
        let ziel = "Splines\\Marcel\\str_2spur_8m_altonaer1.sli";
        let (sa, sb, l) = planen(root, &st, ziel, None).unwrap();
        println!("A {:.2} m, B {:.2} m, Laenge {l} m", sa.breite(), sb.breite());
        println!("{}", setzen(&mut v, &mut ae, root, &st, ziel, None).unwrap());
        v.tiles_around(glam::DVec3::new(150.0, 150.0, 0.0), 2).unwrap();
        assert_eq!(v.spline_end_free(a.spline_id, a.am_ende), Some(false), "die Strasse ist nicht mit dem Uebergang verbunden");
        let mut an2 = crate::anschluss::Anschluesse::default();
        an2.aktualisieren(&v);
        let fern = a.pos.truncate() + crate::netz::dir(a.richtung) * l;
        let arm = an2.liste.iter().find(|x| x.objekt.is_some() && (x.pos.truncate() - fern).length() < 1.0).expect("kein offener Arm am Ende des Uebergangs");
        println!("offener Arm: {:?}, Spuren {:?}", arm.pos, arm.spuren);
        assert_eq!(arm.spuren.len(), sb.pfade.iter().filter(|p| p.art == 0).count());
        if let Some(b) = std::env::var_os("OMSI_BILD") {
            let mitte = a.pos + (crate::netz::dir(a.richtung) * (l / 2.0)).extend(0.0);
            let kam = crate::kamera::Kamera { ziel: mitte, gier: a.richtung as f32 + 200.0, neigung: -40.0, abstand: 45.0, fov: 50.0 };
            let px = v.render_image(1280, 800, &kam.camera()).unwrap();
            image::save_buffer(b, &px, 1280, 800, image::ColorType::Rgba8).unwrap();
        }
        // als neue Karte speichern: Objekt in den Ordner der Karte, die Kachel verweist dorthin
        fn kopieren(a: &Path, b: &Path) {
            std::fs::create_dir_all(b).unwrap();
            for e in std::fs::read_dir(a).unwrap().flatten() {
                if e.file_type().unwrap().is_dir() { kopieren(&e.path(), &b.join(e.file_name())) } else { std::fs::copy(e.path(), b.join(e.file_name())).unwrap(); }
            }
        }
        let test_root = std::env::temp_dir().join(format!("omsi-editor-ue-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&test_root);
        kopieren(&root.join("maps/Grundorf"), &test_root.join("maps/Grundorf"));
        let paket = crate::speichern::vorbereiten(&v, &crate::bearbeiten::Bearbeiten::neu(crate::bearbeiten::Werkzeug::Strasse), &crate::netz::Netz::default(), &[],
                                                  &ae.kopien("Grundorf"), Some(ae.kreuzungs_ordner()), "Grundorf").unwrap();
        let ziel = crate::speichern::karte_anlegen(&test_root, "Grundorf", "Grundorf_ue", &paket).unwrap();
        let objekte: Vec<String> = std::fs::read_dir(test_root.join("Sceneryobjects/Aschaffenburg/Grundorf_ue")).unwrap().flatten()
            .map(|e| e.file_name().to_string_lossy().to_string()).collect();
        assert!(objekte.iter().any(|n| n.starts_with("UE_") && n.ends_with(".sco")), "{objekte:?}");
        let verweis = std::fs::read_dir(&ziel).unwrap().flatten().any(|e| {
            let b = std::fs::read(e.path()).unwrap_or_default();
            let t = String::from_utf16_lossy(&b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>());
            t.contains("Aschaffenburg\\Grundorf_ue\\UE_")
        });
        assert!(verweis, "keine Kachel verweist auf den Uebergang im Ordner der neuen Karte");
        std::fs::remove_dir_all(&test_root).ok();
        drop(ae);
    }
}
