//! World Editor, "Gelaende formen" wie in Transport Fever 2: ein runder Pinsel mit Radius und Staerke wirkt, solange
//! die linke Maustaste gedrueckt ist - Anheben, Absenken, Glaetten und Ebnen (auf die Hoehe beim Ansetzen, eine
//! abgegriffene oder eingegebene Hoehe, wahlweise auf ein Raster eingerastet).
//!
//! Das Gelaende einer Kachel sind 61 x 61 Hoehen im 5-m-Raster (`.map.terrain`); benachbarte Kacheln teilen sich die
//! Randpunkte, der Pinsel arbeitet deshalb auf dem Weltraster und schreibt jeden Punkt in alle Kacheln, die ihn haben.
//! Waehrend des Strichs zeigt openOMSI das neue Gelaende sofort (`Viewer::preview_terrain`, nur die Hoehen des
//! Gelaendenetzes); beim Loslassen kommen die Kacheln als `.terrain`-Kopien in den Sitzungsordner und werden neu
//! gelesen (Objekte stehen wieder auf dem Boden, der Schnitt unter den Strassen passt). Die Karte selbst bleibt bis
//! zum Speichern unveraendert. Ein Strich ist ein Rueckgaengig-Schritt.
//!
//! "Strassen schuetzen": Punkte, an denen eine Strasse, ein Gehweg oder eine Kreuzung auf dem Boden liegt (hoechstens
//! 1,5 m darueber), bleiben, wie sie sind - in OMSI wird das Gelaende unter Strassen nicht ausgeschnitten, angehobener
//! Boden laege sonst auf der Fahrbahn. Unter Bruecken darf geformt werden.

use anyhow::{Context, Result};
use glam::DVec2;
use omsi_map::Terrain;
use openomsi_game::viewer::Viewer;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Zellen je Kachel (61 Punkte) und Abstand der Punkte
const N: i32 = 60;
const ZELLE: f64 = 5.0;

pub type Kachel = (i32, i32);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Modus {
    Heben,
    Senken,
    Glaetten,
    Ebnen,
}

impl Modus {
    pub const ALLE: [Modus; 4] = [Modus::Heben, Modus::Senken, Modus::Glaetten, Modus::Ebnen];

    pub fn name(self) -> &'static str {
        match self {
            Modus::Heben => "Anheben",
            Modus::Senken => "Absenken",
            Modus::Glaetten => "Glaetten",
            Modus::Ebnen => "Ebnen",
        }
    }

    pub fn hilfe(self) -> &'static str {
        match self {
            Modus::Heben => "Maustaste halten hebt das Gelaende unter dem Pinsel (Strg: absenken).",
            Modus::Senken => "Maustaste halten senkt das Gelaende unter dem Pinsel (Strg: anheben).",
            Modus::Glaetten => "Maustaste halten gleicht Kanten, Stufen und Huckel aus - ueber das Gelaende streichen.",
            Modus::Ebnen => "Ebnet auf die Zielhoehe: die Hoehe beim Ansetzen oder eine feste (Strg+Klick greift sie ab).",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Pinsel {
    pub modus: Modus,
    /// Radius in m
    pub radius: f64,
    /// 0..1
    pub staerke: f64,
    /// Ebnen: feste Zielhoehe (sonst die Hoehe beim Ansetzen)
    pub fest: bool,
    pub ziel: f64,
    /// Ebnen: Zielhoehe auf dieses Raster einrasten (0: aus)
    pub raster: f64,
    pub schuetzen: bool,
}

impl Default for Pinsel {
    fn default() -> Self {
        Pinsel { modus: Modus::Glaetten, radius: 25.0, staerke: 0.5, fest: false, ziel: 0.0, raster: 0.0, schuetzen: true }
    }
}

pub const RADIUS: (f64, f64) = (5.0, 200.0);

impl Pinsel {
    /// Zielhoehe beim Ebnen, wenn bei `h` angesetzt wird (eingerastet)
    pub fn zielhoehe(&self, h: f64) -> f64 {
        let z = if self.fest { self.ziel } else { h };
        if self.raster > 0.0 { (z / self.raster).round() * self.raster } else { z }
    }

    /// Gewicht am Abstand `d` von der Mitte: weich bis zum Rand, beim Ebnen ein fester Kern mit weichem Rand
    fn gewicht(&self, d: f64) -> f64 {
        let r = self.radius;
        if d >= r {
            return 0.0;
        }
        if self.modus == Modus::Ebnen {
            return ((1.0 - d / r) / 0.3).clamp(0.0, 1.0);
        }
        (1.0 - (d / r).powi(2)).powi(2)
    }
}

/// Gelaende der beteiligten Kacheln, als Weltraster gelesen und geschrieben (Punkt gx, gy liegt bei 5 m * gx, gy)
#[derive(Default, Clone)]
pub struct Gitter {
    pub kacheln: HashMap<Kachel, Terrain>,
}

/// Kacheln, in denen der Rasterwert `g` liegt (Index darin): am Rand zwei
fn lagen(g: i32) -> impl Iterator<Item = (i32, usize)> {
    let (t, i) = (g.div_euclid(N), g.rem_euclid(N));
    std::iter::once((t, i as usize)).chain((i == 0).then_some((t - 1, N as usize)))
}

impl Gitter {
    pub fn hoehe(&self, gx: i32, gy: i32) -> Option<f32> {
        for (tx, ix) in lagen(gx) {
            for (ty, iy) in lagen(gy) {
                if let Some(t) = self.kacheln.get(&(tx, ty)) {
                    return Some(t.heights[iy * t.samples() + ix]);
                }
            }
        }
        None
    }

    /// Wert in alle Kacheln schreiben, die den Punkt haben; die geaenderten Kacheln
    fn setzen(&mut self, gx: i32, gy: i32, h: f32, geaendert: &mut HashSet<Kachel>) {
        for (tx, ix) in lagen(gx) {
            for (ty, iy) in lagen(gy) {
                if let Some(t) = self.kacheln.get_mut(&(tx, ty)) {
                    let n = t.samples();
                    let alt = &mut t.heights[iy * n + ix];
                    if (*alt - h).abs() > 1e-4 {
                        *alt = h;
                        geaendert.insert((tx, ty));
                    }
                }
            }
        }
    }

    /// Kacheln, die ein Bereich um `mitte` mit Radius `r` beruehrt
    pub fn kacheln_um(mitte: DVec2, r: f64) -> Vec<Kachel> {
        let s = N as f64 * ZELLE;
        let (x0, x1) = (((mitte.x - r) / s).floor() as i32, ((mitte.x + r) / s).floor() as i32);
        let (y0, y1) = (((mitte.y - r) / s).floor() as i32, ((mitte.y + r) / s).floor() as i32);
        (x0..=x1).flat_map(|x| (y0..=y1).map(move |y| (x, y))).collect()
    }

    /// Einen Pinselschritt (Dauer `dt` s) bei `mitte` anwenden; `frei(gx, gy)`: darf der Punkt sich aendern.
    /// Liefert die geaenderten Kacheln.
    pub fn anwenden(&mut self, p: &Pinsel, modus: Modus, mitte: DVec2, ziel: f64, dt: f64, frei: &mut dyn FnMut(i32, i32) -> bool) -> HashSet<Kachel> {
        let r = p.radius;
        let (gx0, gx1) = (((mitte.x - r) / ZELLE).floor() as i32, ((mitte.x + r) / ZELLE).ceil() as i32);
        let (gy0, gy1) = (((mitte.y - r) / ZELLE).floor() as i32, ((mitte.y + r) / ZELLE).ceil() as i32);
        let s = p.staerke.clamp(0.01, 1.0);
        // Glaetten: Mittel ueber ein Fenster, das mit dem Pinsel waechst (sonst bewegt ein grosser Pinsel kaum etwas)
        let kr = ((r / ZELLE / 4.0).round() as i32).clamp(1, 3);
        let mut neu = Vec::new();
        for gx in gx0..=gx1 {
            for gy in gy0..=gy1 {
                let d = (DVec2::new(gx as f64, gy as f64) * ZELLE - mitte).length();
                let w = p.gewicht(d);
                if w <= 0.0 {
                    continue;
                }
                let Some(h) = self.hoehe(gx, gy) else { continue };
                let h = h as f64;
                let h2 = match modus {
                    Modus::Heben => h + 10.0 * s * w * dt,
                    Modus::Senken => h - 10.0 * s * w * dt,
                    Modus::Ebnen => h + (ziel - h) * (1.0 - (-dt * 25.0 * s * w).exp()),
                    Modus::Glaetten => {
                        let (mut summe, mut n) = (0.0, 0.0);
                        for dx in -kr..=kr {
                            for dy in -kr..=kr {
                                if let Some(q) = self.hoehe(gx + dx, gy + dy) {
                                    summe += q as f64;
                                    n += 1.0;
                                }
                            }
                        }
                        h + (summe / n - h) * (1.0 - (-dt * 12.0 * s * w).exp())
                    }
                };
                if (h2 - h).abs() > 1e-5 && frei(gx, gy) {
                    neu.push((gx, gy, h2 as f32));
                }
            }
        }
        // erst rechnen, dann schreiben (Glaetten liest die Nachbarn vom Stand vor dem Schritt)
        let mut geaendert = HashSet::new();
        for (gx, gy, h) in neu {
            self.setzen(gx, gy, h, &mut geaendert);
        }
        geaendert
    }
}

struct Strich {
    gitter: Gitter,
    vorher: HashMap<Kachel, Terrain>,
    geaendert: HashSet<Kachel>,
    modus: Modus,
    ziel: f64,
    /// Rasterpunkt -> darf sich aendern (Strassen schuetzen), einmal je Strich bestimmt
    frei: HashMap<(i32, i32), bool>,
}

#[derive(Clone)]
struct Schritt {
    kacheln: Vec<(Kachel, Terrain, Terrain)>,
}

#[derive(Default)]
pub struct Gelaende {
    pub pinsel: Pinsel,
    strich: Option<Strich>,
    undo: Vec<Schritt>,
    redo: Vec<Schritt>,
    pub aenderungen: usize,
}

impl Gelaende {
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    /// neue Karte: Verlauf und Strich weg, der Pinsel bleibt eingestellt
    pub fn zuruecksetzen(&mut self) {
        *self = Gelaende { pinsel: self.pinsel.clone(), ..Default::default() };
    }

    pub fn malt(&self) -> bool {
        self.strich.is_some()
    }

    /// Zielhoehe des laufenden Strichs (Ebnen)
    pub fn strich_ziel(&self) -> Option<f64> {
        self.strich.as_ref().filter(|s| s.modus == Modus::Ebnen).map(|s| s.ziel)
    }

    /// Strich ansetzen bei `p` (Gelaendehoehe dort `h`); `umkehren`: Strg (Heben <-> Senken)
    pub fn ansetzen(&mut self, h: f64, umkehren: bool) {
        let modus = match (self.pinsel.modus, umkehren) {
            (Modus::Heben, true) => Modus::Senken,
            (Modus::Senken, true) => Modus::Heben,
            (m, _) => m,
        };
        let ziel = self.pinsel.zielhoehe(h);
        self.strich = Some(Strich { gitter: Gitter::default(), vorher: HashMap::new(), geaendert: HashSet::new(), modus, ziel, frei: HashMap::new() });
    }

    /// Pinselschritt am Punkt `mitte` (Dauer `dt`); `geschuetzt(x, y)` fuer eigene Strassen des Editors.
    /// Zeigt die geaenderten Kacheln sofort.
    pub fn malen(&mut self, v: &mut Viewer, mitte: DVec2, dt: f64, geschuetzt: &dyn Fn(DVec2) -> bool) -> usize {
        let Some(st) = self.strich.as_mut() else { return 0 };
        let p = self.pinsel.clone();
        let rand = if p.modus == Modus::Glaetten { 3.0 * ZELLE } else { 0.0 };
        for k in Gitter::kacheln_um(mitte, p.radius + rand) {
            if st.gitter.kacheln.contains_key(&k) {
                continue;
            }
            if let Some(t) = v.tile_terrain(k.0, k.1).filter(|t| t.cells == N as usize) {
                st.vorher.insert(k, t.clone());
                st.gitter.kacheln.insert(k, t);
            }
        }
        let (modus, ziel) = (st.modus, st.ziel);
        let schuetzen = p.schuetzen;
        let frei_cache = &mut st.frei;
        let gitter = &mut st.gitter;
        let vr: &Viewer = v;
        let mut frei = |gx: i32, gy: i32| -> bool {
            if !schuetzen {
                return true;
            }
            *frei_cache.entry((gx, gy)).or_insert_with(|| {
                let q = DVec2::new(gx as f64, gy as f64) * ZELLE;
                let boden = vr.terrain_height(q.x, q.y).unwrap_or(0.0);
                let strasse = [(0.0, 0.0), (2.5, 0.0), (-2.5, 0.0), (0.0, 2.5), (0.0, -2.5)].iter().any(|(dx, dy)| {
                    vr.surface_height(q.x + dx, q.y + dy).is_some_and(|s| s - boden < 1.5 && s - boden > -4.0)
                });
                !strasse && !geschuetzt(q)
            })
        };
        let neu = gitter.anwenden(&p, modus, mitte, ziel, dt, &mut frei);
        for k in &neu {
            if let Some(t) = st.gitter.kacheln.get(k) {
                v.preview_terrain(k.0, k.1, t);
            }
        }
        st.geaendert.extend(neu.iter().copied());
        neu.len()
    }

    /// Strich beenden: geaenderte Kacheln als `.terrain` in den Sitzungsordner `ordner` und neu lesen
    pub fn loslassen(&mut self, v: &mut Viewer, ordner: &Path) -> Result<Option<String>> {
        let Some(st) = self.strich.take() else { return Ok(None) };
        if st.geaendert.is_empty() {
            return Ok(None);
        }
        let mut kacheln: Vec<(Kachel, Terrain, Terrain)> = st.geaendert.iter().map(|k| (*k, st.vorher[k].clone(), st.gitter.kacheln[k].clone())).collect();
        kacheln.sort_by_key(|k| k.0);
        let s = Schritt { kacheln };
        schreiben(v, ordner, &s, false)?;
        let (lo, hi) = s.kacheln.iter().flat_map(|(_, a, b)| a.heights.iter().zip(&b.heights).map(|(x, y)| y - x)).fold((0f32, 0f32), |(a, b), d| (a.min(d), b.max(d)));
        let n = s.kacheln.len();
        self.undo.push(s);
        self.redo.clear();
        self.aenderungen += 1;
        Ok(Some(format!("Gelaende: {} ({} Kachel{}, {:+.1} bis {:+.1} m)", st.modus.name(), n, if n == 1 { "" } else { "n" }, lo, hi)))
    }

    /// Gelaende ganzer Kacheln ersetzen (Ort festlegen: echtes Gelaende), ein Rueckgaengig-Schritt
    pub fn uebernehmen(&mut self, v: &mut Viewer, ordner: &Path, neu: Vec<(Kachel, Vec<f32>)>) -> Result<usize> {
        let mut kacheln = Vec::new();
        for (k, h) in neu {
            let Some(alt) = v.tile_terrain(k.0, k.1).filter(|t| t.cells == N as usize) else { continue };
            if h.len() != alt.heights.len() {
                continue;
            }
            kacheln.push((k, alt, Terrain { cells: N as usize, heights: h }));
        }
        if kacheln.is_empty() {
            return Ok(0);
        }
        let s = Schritt { kacheln };
        schreiben(v, ordner, &s, false)?;
        let n = s.kacheln.len();
        self.undo.push(s);
        self.redo.clear();
        self.aenderungen += 1;
        Ok(n)
    }

    /// Strich verwerfen (z. B. Werkzeugwechsel ohne Loslassen): Vorschau zuruecknehmen
    pub fn abbrechen(&mut self, v: &mut Viewer) {
        if let Some(st) = self.strich.take() {
            for k in &st.geaendert {
                v.preview_terrain(k.0, k.1, &st.vorher[k]);
            }
        }
    }

    pub fn rueckgaengig(&mut self, v: &mut Viewer, ordner: &Path) -> Result<bool> {
        let Some(s) = self.undo.pop() else { return Ok(false) };
        schreiben(v, ordner, &s, true)?;
        self.redo.push(s);
        self.aenderungen += 1;
        Ok(true)
    }

    pub fn wiederholen(&mut self, v: &mut Viewer, ordner: &Path) -> Result<bool> {
        let Some(s) = self.redo.pop() else { return Ok(false) };
        schreiben(v, ordner, &s, false)?;
        self.undo.push(s);
        self.aenderungen += 1;
        Ok(true)
    }
}

/// Gelaende eines Schritts (vorher oder nachher) in den Sitzungsordner schreiben, zeigen und die Kacheln neu lesen
fn schreiben(v: &mut Viewer, ordner: &Path, s: &Schritt, vorher: bool) -> Result<()> {
    let refs = v.map_tile_refs();
    std::fs::create_dir_all(ordner)?;
    let mut keys = Vec::new();
    for (k, a, b) in &s.kacheln {
        let t = if vorher { a } else { b };
        let Some((_, _, datei)) = refs.iter().find(|r| (r.0, r.1) == *k) else { continue };
        let name = Path::new(datei).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| datei.clone());
        let pfad = ordner.join(format!("{name}.terrain"));
        std::fs::write(&pfad, t.to_bytes()).with_context(|| format!("{} schreiben", pfad.display()))?;
        v.preview_terrain(k.0, k.1, t);
        keys.push(*k);
    }
    v.reload_tiles(&keys)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kachel(h: impl Fn(usize, usize) -> f32) -> Terrain {
        let n = N as usize + 1;
        Terrain { cells: N as usize, heights: (0..n * n).map(|i| h(i % n, i / n)).collect() }
    }

    /// Zwei Kacheln nebeneinander: der Pinsel ueber der Grenze schreibt die gemeinsamen Randpunkte in beide gleich
    #[test]
    fn rand_bleibt_dicht() {
        let mut g = Gitter::default();
        g.kacheln.insert((0, 0), kachel(|_, _| 10.0));
        g.kacheln.insert((1, 0), kachel(|_, _| 10.0));
        let p = Pinsel { modus: Modus::Heben, radius: 30.0, staerke: 1.0, ..Default::default() };
        let neu = g.anwenden(&p, Modus::Heben, DVec2::new(300.0, 150.0), 0.0, 0.5, &mut |_, _| true);
        assert_eq!(neu.len(), 2);
        let (a, b) = (&g.kacheln[&(0, 0)], &g.kacheln[&(1, 0)]);
        for iy in 0..=60 {
            assert_eq!(a.height_at(60, iy), b.height_at(0, iy));
        }
        assert!((a.height_at(60, 30) - 15.0).abs() < 1e-3, "Mitte +5 m: {}", a.height_at(60, 30));
        assert_eq!(a.height_at(50, 30), 10.0, "ausserhalb des Radius unveraendert");
    }

    /// Glaetten baut eine Stufe ab, Ebnen bringt den Kern genau auf die (eingerastete) Zielhoehe
    #[test]
    fn glaetten_und_ebnen() {
        let mut g = Gitter::default();
        g.kacheln.insert((0, 0), kachel(|ix, _| if ix < 30 { 0.0 } else { 10.0 }));
        let mut p = Pinsel { modus: Modus::Glaetten, radius: 40.0, staerke: 1.0, ..Default::default() };
        let stufe = |g: &Gitter| g.hoehe(30, 30).unwrap() - g.hoehe(29, 30).unwrap();
        assert_eq!(stufe(&g), 10.0);
        for _ in 0..60 {
            g.anwenden(&p, Modus::Glaetten, DVec2::new(150.0, 150.0), 0.0, 1.0 / 30.0, &mut |_, _| true);
        }
        assert!(stufe(&g) < 2.0, "Stufe nach Glaetten {}", stufe(&g));
        p.modus = Modus::Ebnen;
        p.raster = 2.5;
        let ziel = p.zielhoehe(6.3);
        assert_eq!(ziel, 7.5);
        for _ in 0..60 {
            g.anwenden(&p, Modus::Ebnen, DVec2::new(150.0, 150.0), ziel, 1.0 / 30.0, &mut |_, _| true);
        }
        assert!((g.hoehe(30, 30).unwrap() - 7.5).abs() < 0.01);
        assert!((g.hoehe(34, 27).unwrap() - 7.5).abs() < 0.01);
    }

    /// Grundorf: auf freiem Feld anheben (sofort sichtbar), loslassen (Sitzungskopie, neu gelesen), rueckgaengig und
    /// wiederholen; an einer Strasse bleibt das Gelaende mit "Strassen schuetzen"
    #[test]
    #[ignore]
    fn grundorf_formen() {
        let _sperre = crate::bearbeiten::tests::sperre();
        let mut v = crate::bearbeiten::tests::grundorf();
        let a = crate::aendern::Aendern::neu(&v);
        let ordner = a.sitzung.join("maps").join("Grundorf");
        // freies Feld und ein Punkt auf einer Strasse in den geladenen Kacheln
        let mut feld = None;
        let mut strasse = None;
        for i in 0..60 {
            for j in 0..60 {
                let q = DVec2::new(-280.0 + i as f64 * 15.0 + 2.5, -280.0 + j as f64 * 15.0 + 2.5);
                let Some(b) = v.terrain_height(q.x, q.y) else { continue };
                let frei = (0..12).all(|k| {
                    let w = k as f64 / 12.0 * std::f64::consts::TAU;
                    v.surface_height(q.x + 30.0 * w.cos(), q.y + 30.0 * w.sin()).is_none()
                }) && v.surface_height(q.x, q.y).is_none();
                if frei && feld.is_none() {
                    feld = Some(q);
                }
                // Strassenpunkt genau auf dem Raster (5 m)
                let r = (q / ZELLE).round() * ZELLE;
                if strasse.is_none() && v.surface_height(r.x, r.y).is_some_and(|s| (s - b).abs() < 0.5) {
                    strasse = Some(r);
                }
            }
        }
        let feld = (feld.expect("freies Feld") / ZELLE).round() * ZELLE;
        let strasse = strasse.expect("Strasse");
        println!("Feld {feld:?}, Strasse {strasse:?}");
        let h0 = v.terrain_height(feld.x, feld.y).unwrap();
        // OMSI_BILD: vorher, Vorschau waehrend des Strichs, nach dem Neulesen
        let kam = crate::kamera::Kamera { ziel: feld.extend(h0), gier: 30.0, neigung: -25.0, abstand: 90.0, fov: 50.0 };
        let bild = |v: &mut Viewer, name: &str| {
            if let Some(b) = std::env::var_os("OMSI_BILD") {
                let px = v.render_image(1280, 800, &kam.camera()).unwrap();
                image::save_buffer(std::path::PathBuf::from(b).with_extension(format!("{name}.png")), &px, 1280, 800, image::ColorType::Rgba8).unwrap();
            }
        };
        bild(&mut v, "vorher");
        let mut g = Gelaende::default();
        g.pinsel = Pinsel { modus: Modus::Heben, radius: 20.0, staerke: 1.0, schuetzen: true, ..Default::default() };
        g.ansetzen(h0, false);
        for _ in 0..5 {
            assert!(g.malen(&mut v, feld, 0.1, &|_| false) > 0);
        }
        let h1 = v.terrain_height(feld.x, feld.y).unwrap();
        assert!((h1 - h0 - 5.0).abs() < 0.01, "Vorschau {h0} -> {h1}");
        bild(&mut v, "vorschau");
        println!("{}", g.loslassen(&mut v, &ordner).unwrap().unwrap());
        assert_eq!(g.undo_len(), 1);
        assert!(a.kopien("Grundorf").iter().any(|p| p.to_string_lossy().ends_with(".map.terrain")), "keine Gelaende-Kopie");
        v.tiles_around(glam::DVec3::new(150.0, 150.0, 0.0), 1).unwrap();
        let h2 = v.terrain_height(feld.x, feld.y).unwrap();
        assert!((h2 - h1).abs() < 0.01, "nach dem Neulesen {h2} statt {h1}");
        bild(&mut v, "gelesen");
        assert!(g.rueckgaengig(&mut v, &ordner).unwrap());
        v.tiles_around(glam::DVec3::new(150.0, 150.0, 0.0), 1).unwrap();
        assert!((v.terrain_height(feld.x, feld.y).unwrap() - h0).abs() < 0.01, "rueckgaengig");
        assert!(g.wiederholen(&mut v, &ordner).unwrap());
        v.tiles_around(glam::DVec3::new(150.0, 150.0, 0.0), 1).unwrap();
        assert!((v.terrain_height(feld.x, feld.y).unwrap() - h1).abs() < 0.01, "wiederholt");
        // Strasse: geschuetzt bleibt der Punkt unter ihr
        let s0 = v.terrain_height(strasse.x, strasse.y).unwrap();
        g.ansetzen(s0, false);
        g.malen(&mut v, strasse, 0.3, &|_| false);
        g.loslassen(&mut v, &ordner).unwrap();
        v.tiles_around(glam::DVec3::new(150.0, 150.0, 0.0), 1).unwrap();
        assert!((v.terrain_height(strasse.x, strasse.y).unwrap() - s0).abs() < 0.01, "Gelaende unter der Strasse veraendert");
        // ganze Kachel ersetzen (Ort festlegen) und zurueck
        let vorher = v.terrain_height(150.0, 150.0).unwrap();
        assert_eq!(g.uebernehmen(&mut v, &ordner, vec![((0, 0), vec![3.0; 61 * 61])]).unwrap(), 1);
        v.tiles_around(glam::DVec3::new(150.0, 150.0, 0.0), 1).unwrap();
        assert!((v.terrain_height(150.0, 150.0).unwrap() - 3.0).abs() < 1e-3);
        assert!(g.rueckgaengig(&mut v, &ordner).unwrap());
        v.tiles_around(glam::DVec3::new(150.0, 150.0, 0.0), 1).unwrap();
        assert!((v.terrain_height(150.0, 150.0).unwrap() - vorher).abs() < 1e-3);
    }

    /// geschuetzte Punkte bleiben
    #[test]
    fn schutz() {
        let mut g = Gitter::default();
        g.kacheln.insert((0, 0), kachel(|_, _| 0.0));
        let p = Pinsel { modus: Modus::Senken, radius: 20.0, staerke: 1.0, ..Default::default() };
        g.anwenden(&p, Modus::Senken, DVec2::new(100.0, 100.0), 0.0, 1.0, &mut |gx, _| gx != 20);
        assert_eq!(g.hoehe(20, 20), Some(0.0));
        assert!(g.hoehe(21, 20).unwrap() < -5.0);
    }
}
