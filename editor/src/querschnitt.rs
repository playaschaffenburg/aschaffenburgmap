//! Querschnitt-Baukasten: eigene Strassen-Splines (.sli) aus Bausteinen - Fahrspuren (mit Richtung), Busspur,
//! Radfahrstreifen, Parkstreifen, Gehweg, Radweg, Gruenstreifen, Mittelinsel -, jede mit eigener Breite und eigenem
//! Belag, von links nach rechts. Daraus entstehen:
//! - die sichtbaren Profile: Belaege gekachelt (eine Textur deckt nur einige Meter, breitere Teile werden in Streifen
//!   zerlegt), Bordsteine wo die Hoehe wechselt (Fahrbahn 0,10 m, Hochbord 0,25 m), Markierungen als eigene Profile
//!   1 cm ueber der Fahrbahn mit eigenen Texturen (Leitlinie 3 m / 6 m, durchgezogen, doppelt, Breitstrich);
//! - `[heightprofile]` je Teil (darauf stehen Raeder und Fussgaenger);
//! - `[path]` je Fahrspur (Fahrzeug, Richtung mit/gegen den Spline) und je Gehweg (Fussgaenger, beide Richtungen) -
//!   das ist, was die KI sieht.
//! Die Spline-Achse (x = 0) liegt in der Mitte der Fahrspuren. Positive x = rechts in Splinerichtung; Spuren
//! "vor" fahren mit dem Spline (Rechtsverkehr: rechts der Achse).
//!
//! Gespeichert wird nach `Splines\Aschaffenburg\<Datei>.sli` (die Texturen nach `Splines\Aschaffenburg\texture\`:
//! Belaege aus `Splines\Marcel\texture` kopiert, Markierungen und roter Radweg erzeugt) und die Bauanleitung daneben
//! als `<Datei>.qs.json` - so laesst sich jeder eigene Querschnitt wieder oeffnen und aendern.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

/// Hoehe der Fahrbahn und des Hochbords (wie die Standardsplines)
pub const FAHRBAHN: f64 = 0.10;
pub const HOCHBORD: f64 = 0.25;
/// Breite der Bordsteinoberkante
const BORD: f64 = 0.15;
/// Markierungen so weit ueber der Fahrbahn
const MARKE_HOEHE: f64 = 0.01;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Art {
    Fahrspur,
    Busspur,
    Radfahrstreifen,
    Parkstreifen,
    Gehweg,
    Radweg,
    Gruen,
    Mittelinsel,
}

impl Art {
    pub const ALLE: [Art; 8] = [Art::Fahrspur, Art::Busspur, Art::Radfahrstreifen, Art::Parkstreifen, Art::Gehweg, Art::Radweg, Art::Gruen, Art::Mittelinsel];

    pub fn name(self) -> &'static str {
        match self {
            Art::Fahrspur => "Fahrspur",
            Art::Busspur => "Busspur",
            Art::Radfahrstreifen => "Radfahrstreifen",
            Art::Parkstreifen => "Parkstreifen",
            Art::Gehweg => "Gehweg",
            Art::Radweg => "Radweg (Hochbord)",
            Art::Gruen => "Gruenstreifen",
            Art::Mittelinsel => "Mittelinsel",
        }
    }

    fn schluessel(self) -> &'static str {
        match self {
            Art::Fahrspur => "fahrspur",
            Art::Busspur => "busspur",
            Art::Radfahrstreifen => "radfahrstreifen",
            Art::Parkstreifen => "parkstreifen",
            Art::Gehweg => "gehweg",
            Art::Radweg => "radweg",
            Art::Gruen => "gruen",
            Art::Mittelinsel => "mittelinsel",
        }
    }

    fn aus(s: &str) -> Option<Art> {
        Art::ALLE.into_iter().find(|a| a.schluessel() == s)
    }

    /// faehrt hier die KI (eine Spur mit `[path]`)?
    pub fn spur(self) -> bool {
        matches!(self, Art::Fahrspur | Art::Busspur)
    }

    /// liegt auf Fahrbahnhoehe (sonst Hochbord)
    pub fn unten(self) -> bool {
        matches!(self, Art::Fahrspur | Art::Busspur | Art::Radfahrstreifen | Art::Parkstreifen)
    }

    pub fn standard_breite(self) -> f64 {
        match self {
            Art::Fahrspur => 3.25,
            Art::Busspur => 3.5,
            Art::Radfahrstreifen => 1.85,
            Art::Parkstreifen => 2.2,
            Art::Gehweg => 2.5,
            Art::Radweg => 1.6,
            Art::Gruen => 2.0,
            Art::Mittelinsel => 2.0,
        }
    }

    pub fn standard_belag(self) -> Belag {
        match self {
            Art::Fahrspur | Art::Busspur => Belag::Asphalt,
            Art::Radfahrstreifen | Art::Radweg => Belag::Rot,
            Art::Parkstreifen => Belag::Verbund,
            Art::Gehweg => Belag::Platten,
            Art::Gruen | Art::Mittelinsel => Belag::Gras,
        }
    }

    /// Farbe in der Skizze
    pub fn farbe(self) -> [u8; 3] {
        match self {
            Art::Fahrspur => [70, 72, 78],
            Art::Busspur => [90, 80, 70],
            Art::Radfahrstreifen | Art::Radweg => [170, 70, 60],
            Art::Parkstreifen => [120, 120, 125],
            Art::Gehweg => [175, 172, 165],
            Art::Gruen | Art::Mittelinsel => [80, 140, 70],
        }
    }
}

/// Oberflaeche: Textur, nutzbarer u-Bereich, Meter je u (quer), v je Meter (laengs)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Belag {
    Asphalt,
    Rot,
    Platten,
    Verbund,
    Kopfstein,
    Beton,
    Gras,
}

pub struct Textur {
    pub datei: &'static str,
    /// aus Splines\Marcel\texture (sonst erzeugt)
    pub marcel: bool,
    pub u: (f64, f64),
    pub m_je_u: f64,
    pub v_je_m: f64,
}

impl Belag {
    pub const ALLE: [Belag; 7] = [Belag::Asphalt, Belag::Rot, Belag::Platten, Belag::Verbund, Belag::Kopfstein, Belag::Beton, Belag::Gras];

    pub fn name(self) -> &'static str {
        match self {
            Belag::Asphalt => "Asphalt",
            Belag::Rot => "Asphalt rot",
            Belag::Platten => "Gehwegplatten",
            Belag::Verbund => "Verbundpflaster",
            Belag::Kopfstein => "Kopfsteinpflaster",
            Belag::Beton => "Betonplatten",
            Belag::Gras => "Gras",
        }
    }

    fn schluessel(self) -> &'static str {
        match self {
            Belag::Asphalt => "asphalt",
            Belag::Rot => "rot",
            Belag::Platten => "platten",
            Belag::Verbund => "verbund",
            Belag::Kopfstein => "kopfstein",
            Belag::Beton => "beton",
            Belag::Gras => "gras",
        }
    }

    fn aus(s: &str) -> Option<Belag> {
        Belag::ALLE.into_iter().find(|b| b.schluessel() == s)
    }

    pub fn textur(self) -> Textur {
        match self {
            Belag::Asphalt => Textur { datei: "str_asphdrk.bmp", marcel: true, u: (0.02, 0.98), m_je_u: 4.0, v_je_m: 0.167 },
            Belag::Rot => Textur { datei: "AB_asphalt_rot.bmp", marcel: false, u: (0.02, 0.98), m_je_u: 4.0, v_je_m: 0.167 },
            Belag::Platten => Textur { datei: "str_gehweg02.bmp", marcel: true, u: (0.0, 0.9), m_je_u: 3.9, v_je_m: 0.2 },
            Belag::Verbund => Textur { datei: "str_verbund1.bmp", marcel: true, u: (0.0, 0.94), m_je_u: 3.9, v_je_m: 0.2 },
            Belag::Kopfstein => Textur { datei: "str_kopfstein.bmp", marcel: true, u: (0.0, 0.94), m_je_u: 3.9, v_je_m: 0.2 },
            Belag::Beton => Textur { datei: "str_betonplatten.bmp", marcel: true, u: (0.0, 0.98), m_je_u: 3.9, v_je_m: 0.2 },
            Belag::Gras => Textur { datei: "gras1.bmp", marcel: true, u: (0.0, 1.0), m_je_u: 4.0, v_je_m: 0.25 },
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Richtung {
    /// mit dem Spline (rechts der Achse ueblich)
    Vor,
    /// gegen den Spline
    Zurueck,
}

/// Markierung an der Grenze zwischen zwei Teilen
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Marke {
    /// nach den Regeln (siehe `auto_marke`)
    Auto,
    Keine,
    Strich,
    Voll,
    Doppelt,
    Breitstrich,
}

impl Marke {
    pub const ALLE: [Marke; 6] = [Marke::Auto, Marke::Keine, Marke::Strich, Marke::Voll, Marke::Doppelt, Marke::Breitstrich];

    pub fn name(self) -> &'static str {
        match self {
            Marke::Auto => "automatisch",
            Marke::Keine => "keine",
            Marke::Strich => "Leitlinie (gestrichelt)",
            Marke::Voll => "durchgezogen",
            Marke::Doppelt => "doppelt durchgezogen",
            Marke::Breitstrich => "Breitstrich",
        }
    }

    fn schluessel(self) -> &'static str {
        match self {
            Marke::Auto => "auto",
            Marke::Keine => "keine",
            Marke::Strich => "strich",
            Marke::Voll => "voll",
            Marke::Doppelt => "doppelt",
            Marke::Breitstrich => "breitstrich",
        }
    }

    fn aus(s: &str) -> Option<Marke> {
        Marke::ALLE.into_iter().find(|m| m.schluessel() == s)
    }

    /// (Textur, Breite in m, v je Meter)
    pub fn textur(self) -> Option<(&'static str, f64, f64)> {
        match self {
            Marke::Strich => Some(("AB_marke_strich.tga", 0.12, 1.0 / 9.0)),
            Marke::Voll => Some(("AB_marke_voll.tga", 0.12, 0.2)),
            Marke::Doppelt => Some(("AB_marke_doppelt.tga", 0.36, 0.2)),
            Marke::Breitstrich => Some(("AB_marke_voll.tga", 0.25, 0.2)),
            Marke::Auto | Marke::Keine => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Teil {
    pub art: Art,
    pub breite: f64,
    pub richtung: Richtung,
    pub belag: Belag,
}

impl Teil {
    pub fn neu(art: Art) -> Teil {
        Teil { art, breite: art.standard_breite(), richtung: Richtung::Vor, belag: art.standard_belag() }
    }

    pub fn hoehe(&self) -> f64 {
        if self.art.unten() { FAHRBAHN } else { HOCHBORD }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Querschnitt {
    pub name: String,
    /// von links nach rechts (in Splinerichtung gesehen)
    pub teile: Vec<Teil>,
    /// Markierung je Grenze (teile.len() - 1)
    pub marken: Vec<Marke>,
}

impl Default for Querschnitt {
    /// Stadtstrasse: Gehweg, Fahrspur zurueck, Fahrspur vor, Gehweg
    fn default() -> Self {
        let mut z = Teil::neu(Art::Fahrspur);
        z.richtung = Richtung::Zurueck;
        let mut q = Querschnitt { name: "Neue Strasse".into(), teile: vec![Teil::neu(Art::Gehweg), z, Teil::neu(Art::Fahrspur), Teil::neu(Art::Gehweg)], marken: vec![] };
        q.marken_anpassen();
        q
    }
}

impl Querschnitt {
    /// Marken-Liste auf die Zahl der Grenzen bringen (neue: automatisch)
    pub fn marken_anpassen(&mut self) {
        self.marken.resize(self.teile.len().saturating_sub(1), Marke::Auto);
    }

    pub fn gesamtbreite(&self) -> f64 {
        self.teile.iter().map(|t| t.breite).sum()
    }

    /// x des linken Rands (Achse = Mitte der Fahrspuren; ohne Fahrspur: Mitte des Ganzen)
    pub fn links(&self) -> f64 {
        let mut x = 0.0;
        let (mut a, mut b) = (None, None);
        for t in &self.teile {
            if t.art.spur() {
                a.get_or_insert(x);
                b = Some(x + t.breite);
            }
            x += t.breite;
        }
        match (a, b) {
            (Some(a), Some(b)) => -(a + b) / 2.0,
            _ => -x / 2.0,
        }
    }

    /// (Spuren vor, Spuren zurueck, Gehwege)
    pub fn spuren(&self) -> (usize, usize, usize) {
        let vor = self.teile.iter().filter(|t| t.art.spur() && t.richtung == Richtung::Vor).count();
        let zur = self.teile.iter().filter(|t| t.art.spur() && t.richtung == Richtung::Zurueck).count();
        let geh = self.teile.iter().filter(|t| t.art == Art::Gehweg).count();
        (vor, zur, geh)
    }

    /// Markierung an der Grenze i (zwischen Teil i und i+1), "automatisch" aufgeloest
    pub fn marke(&self, i: usize) -> Marke {
        match self.marken.get(i).copied().unwrap_or(Marke::Auto) {
            Marke::Auto => auto_marke(&self.teile[i], &self.teile[i + 1]),
            m => m,
        }
    }

    pub fn pruefen(&self) -> Result<()> {
        if self.teile.is_empty() {
            bail!("keine Teile");
        }
        if !self.teile.iter().any(|t| t.art.spur()) {
            bail!("mindestens eine Fahrspur (oder Busspur) - sonst faehrt dort niemand");
        }
        if let Some(t) = self.teile.iter().find(|t| !(0.3..=20.0).contains(&t.breite)) {
            bail!("{}: Breite {:.2} m (0,3 bis 20 m)", t.art.name(), t.breite);
        }
        Ok(())
    }

    /// Dateiname (ohne Endung): AB_ + Name (Buchstaben, Ziffern, _)
    pub fn datei(&self) -> String {
        let mut s = String::from("AB_");
        for c in self.name.trim().chars() {
            match c {
                'ä' => s.push_str("ae"), 'ö' => s.push_str("oe"), 'ü' => s.push_str("ue"), 'Ä' => s.push_str("Ae"), 'Ö' => s.push_str("Oe"),
                'Ü' => s.push_str("Ue"), 'ß' => s.push_str("ss"),
                c if c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == ',' => s.push(c),
                _ => {
                    if !s.ends_with('_') {
                        s.push('_');
                    }
                }
            }
        }
        s.trim_end_matches('_').replace(',', "_").to_string()
    }

    // ------------------------------------------------------------ .sli

    /// die Spline-Definition (Text, CRLF) und die gebrauchten Texturen
    pub fn sli(&self) -> (String, Vec<&'static str>) {
        let mut texturen: Vec<&'static str> = Vec::new();
        let mut tex = |d: &'static str| -> usize {
            match texturen.iter().position(|t| *t == d) {
                Some(i) => i,
                None => {
                    texturen.push(d);
                    texturen.len() - 1
                }
            }
        };
        let mut hoehen = Vec::new();
        let mut profile: Vec<(String, usize, Vec<[f64; 4]>)> = Vec::new();
        let mut pfade = Vec::new();
        let x0 = self.links();
        let mut x = x0;
        let n = self.teile.len();
        for (i, t) in self.teile.iter().enumerate() {
            let (a, b) = (x, x + t.breite);
            x = b;
            let h = t.hoehe();
            hoehen.push([a, b, h, h]);
            // Bordsteine an Hoehenwechseln: das hoehere Teil bekommt Oberkante und Kante
            let links_tiefer = i > 0 && self.teile[i - 1].hoehe() < h - 1e-6;
            let rechts_tiefer = i + 1 < n && self.teile[i + 1].hoehe() < h - 1e-6;
            let bord = tex("str_side1.bmp");
            let (mut fa, mut fb) = (a, b);
            if links_tiefer {
                fa = (a + BORD).min(b);
                profile.push(("Bordstein".into(), bord, vec![[a, FAHRBAHN, 0.995, 0.2], [a, h, 0.953, 0.2]]));
                profile.push(("Bordstein".into(), bord, vec![[a, h, 0.953, 0.2], [fa, h, 0.92, 0.2]]));
            }
            if rechts_tiefer {
                fb = (b - BORD).max(fa);
                profile.push(("Bordstein".into(), bord, vec![[fb, h, 0.92, 0.2], [b, h, 0.953, 0.2]]));
                profile.push(("Bordstein".into(), bord, vec![[b, h, 0.953, 0.2], [b, FAHRBAHN, 0.995, 0.2]]));
            }
            // Aussenkanten erhoehter Teile am Rand des Querschnitts: Kante nach unten (sonst sieht man darunter durch)
            if i == 0 && h > FAHRBAHN {
                profile.push(("Rand".into(), bord, vec![[a, 0.0, 0.92, 0.2], [a, h, 0.953, 0.2]]));
            }
            if i + 1 == n && h > FAHRBAHN {
                profile.push(("Rand".into(), bord, vec![[b, h, 0.953, 0.2], [b, 0.0, 0.92, 0.2]]));
            }
            // Belag gekachelt
            let tx = t.belag.textur();
            let ti = tex(tx.datei);
            let spanne = (tx.u.1 - tx.u.0) * tx.m_je_u;
            let stuecke = ((fb - fa) / spanne).ceil().max(1.0) as usize;
            for k in 0..stuecke {
                let sa = fa + (fb - fa) * k as f64 / stuecke as f64;
                let sb = fa + (fb - fa) * (k + 1) as f64 / stuecke as f64;
                let ub = tx.u.0 + (sb - sa) / tx.m_je_u;
                profile.push((t.art.name().into(), ti, vec![[sa, h, tx.u.0, tx.v_je_m], [sb, h, ub.min(tx.u.1), tx.v_je_m]]));
            }
            // Pfade
            let mitte = (a + b) / 2.0;
            match t.art {
                Art::Fahrspur | Art::Busspur => pfade.push([0.0, mitte, h, t.breite, if t.richtung == Richtung::Vor { 0.0 } else { 1.0 }]),
                Art::Gehweg => pfade.push([1.0, mitte, h, t.breite * 0.6, 2.0]),
                _ => {}
            }
        }
        // Markierungen auf den Grenzen (nur zwischen Teilen auf Fahrbahnhoehe)
        let mut x = x0;
        for i in 0..n.saturating_sub(1) {
            x += self.teile[i].breite;
            if !(self.teile[i].art.unten() && self.teile[i + 1].art.unten()) {
                continue;
            }
            if let Some((datei, w, v)) = self.marke(i).textur() {
                let ti = tex(datei);
                let y = FAHRBAHN + MARKE_HOEHE;
                profile.push(("Markierung".into(), ti, vec![[x - w / 2.0, y, 0.0, v], [x + w / 2.0, y, 1.0, v]]));
            }
        }
        let mut out = vec!["File created with omsi-editor (Querschnitt-Baukasten)".to_string(), String::new(),
                           format!("Querschnitt: {} ({:.2} m)", self.name.replace(['\r', '\n'], " "), self.gesamtbreite()), String::new()];
        for [a, b, h1, h2] in hoehen {
            out.extend(["[heightprofile]".into(), z(a), z(b), z(h1), z(h2), String::new()]);
        }
        for t in &texturen {
            out.extend(["[texture]".into(), t.to_string(), String::new()]);
            // Markierungen: Alpha-Test (die Luecken der Leitlinie sind durchsichtig)
            if t.ends_with(".tga") {
                out.extend(["[matl_alpha]".into(), "1".into(), String::new()]);
            }
        }
        for (name, ti, punkte) in profile {
            out.push(format!("{name}:"));
            out.extend(["[profile]".into(), ti.to_string(), String::new()]);
            for [px, py, u, v] in punkte {
                out.extend(["[profilepnt]".into(), z(px), z(py), z(u), z(v), String::new()]);
            }
        }
        for [art, px, h, w, r] in pfade {
            out.extend(["[path]".into(), (art as i32).to_string(), z(px), z(h), z(w), (r as i32).to_string(), String::new()]);
        }
        (out.join("\r\n") + "\r\n", texturen)
    }

    // ------------------------------------------------------------ Bauanleitung (.qs.json)

    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "name": self.name,
            "teile": self.teile.iter().map(|t| serde_json::json!({
                "art": t.art.schluessel(), "breite": t.breite, "belag": t.belag.schluessel(),
                "richtung": if t.richtung == Richtung::Vor { "vor" } else { "zurueck" },
            })).collect::<Vec<_>>(),
            "marken": self.marken.iter().map(|m| m.schluessel()).collect::<Vec<_>>(),
        })
    }

    pub fn aus_json(v: &serde_json::Value) -> Option<Querschnitt> {
        let teile = v["teile"].as_array()?.iter().map(|t| Some(Teil {
            art: Art::aus(t["art"].as_str()?)?,
            breite: t["breite"].as_f64()?,
            belag: t["belag"].as_str().and_then(Belag::aus).unwrap_or(Belag::Asphalt),
            richtung: if t["richtung"].as_str() == Some("zurueck") { Richtung::Zurueck } else { Richtung::Vor },
        })).collect::<Option<Vec<_>>>()?;
        let marken = v["marken"].as_array().map(|l| l.iter().map(|m| m.as_str().and_then(Marke::aus).unwrap_or(Marke::Auto)).collect()).unwrap_or_default();
        let mut q = Querschnitt { name: v["name"].as_str().unwrap_or("").to_string(), teile, marken };
        q.marken_anpassen();
        Some(q)
    }
}

/// Markierung nach den Regeln (innerorts, StVO/RMS vereinfacht)
pub fn auto_marke(a: &Teil, b: &Teil) -> Marke {
    use Art::*;
    match (a.art, b.art) {
        (Fahrspur, Fahrspur) => Marke::Strich,
        (Busspur, Fahrspur) | (Fahrspur, Busspur) => Marke::Breitstrich,
        (Radfahrstreifen, Fahrspur | Busspur) | (Fahrspur | Busspur, Radfahrstreifen) => Marke::Breitstrich,
        (Parkstreifen, Fahrspur | Busspur) | (Fahrspur | Busspur, Parkstreifen) => Marke::Voll,
        (Radfahrstreifen, Parkstreifen) | (Parkstreifen, Radfahrstreifen) => Marke::Strich,
        _ => Marke::Keine,
    }
}

fn z(x: f64) -> String {
    format!("{x:.3}")
}

// ------------------------------------------------------------ Dateien

/// Ordner der eigenen Splines
pub fn ordner(root: &Path) -> PathBuf {
    root.join("Splines").join(crate::speichern::EIGEN)
}

/// Querschnitt speichern: .sli, Bauanleitung, Texturen -> Pfad der .sli relativ zum OMSI-Ordner
pub fn speichern(root: &Path, q: &Querschnitt) -> Result<String> {
    q.pruefen()?;
    let datei = q.datei();
    if datei.len() <= 3 {
        bail!("Name fehlt");
    }
    schreiben(root, q, &datei)
}

/// .sli unter `datei` (ohne Endung) in den eigenen Spline-Ordner schreiben (auch fuer die Vorschau)
pub fn schreiben(root: &Path, q: &Querschnitt, datei: &str) -> Result<String> {
    let d = ordner(root);
    std::fs::create_dir_all(d.join("texture"))?;
    let (text, texturen) = q.sli();
    texturen_bereitstellen(root, &texturen)?;
    // .sli in der ANSI-Codepage (wie OMSI sie liest): Umlaute im Kommentar fallen auf '?'
    let ansi: Vec<u8> = text.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).collect();
    std::fs::write(d.join(format!("{datei}.sli")), ansi).with_context(|| format!("{datei}.sli schreiben"))?;
    if !datei.starts_with('_') {
        std::fs::write(d.join(format!("{datei}.qs.json")), serde_json::to_string_pretty(&q.json())?)?;
    }
    Ok(format!("Splines\\{}\\{datei}.sli", crate::speichern::EIGEN))
}

/// eigene Querschnitte (Bauanleitungen) -> (Datei ohne Endung, Querschnitt)
pub fn eigene(root: &Path) -> Vec<(String, Querschnitt)> {
    let mut out: Vec<(String, Querschnitt)> = std::fs::read_dir(ordner(root)).map(|rd| rd.flatten().filter_map(|e| {
        let n = e.file_name().to_string_lossy().to_string();
        let stamm = n.strip_suffix(".qs.json")?.to_string();
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok()?;
        Some((stamm, Querschnitt::aus_json(&v)?))
    }).collect()).unwrap_or_default();
    out.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    out
}

/// Texturen in Splines\Aschaffenburg\texture: Belaege aus Splines\Marcel\texture kopieren (mit .cfg/.surf),
/// Markierungen und roten Asphalt erzeugen (nur wenn sie fehlen)
pub fn texturen_bereitstellen(root: &Path, texturen: &[&str]) -> Result<()> {
    let ziel = ordner(root).join("texture");
    let marcel = root.join("Splines").join("Marcel").join("texture");
    for t in texturen {
        let z = ziel.join(t);
        if z.is_file() {
            continue;
        }
        match *t {
            "AB_marke_strich.tga" => tga(&z, 16, 288, |_, y| y < 96)?,
            "AB_marke_voll.tga" => tga(&z, 16, 64, |_, _| true)?,
            "AB_marke_doppelt.tga" => tga(&z, 48, 64, |x, _| !(16..32).contains(&x))?,
            "AB_klinker.bmp" => {
                // Klinker im Laeuferverband: 256 px = 2 m breit, 1 m hoch (Stein 24 x 7 cm, Fuge 1 cm)
                let img = image::RgbImage::from_fn(256, 256, |x, y| {
                    let reihe = y / 19;
                    let versatz = if reihe % 2 == 0 { 0 } else { 16 };
                    let fuge = y % 19 >= 17 || (x + versatz) % 32 >= 30;
                    let stein = (x + versatz) / 32 + reihe * 7;
                    let n = ((x.wrapping_mul(73) ^ y.wrapping_mul(151)) % 23) as i32 - 11;
                    if fuge {
                        image::Rgb([(150 + n) as u8, (146 + n) as u8, (138 + n) as u8])
                    } else {
                        let ton = (stein.wrapping_mul(2654435761) >> 7) % 30;
                        image::Rgb([(128 + ton as i32 + n).clamp(0, 255) as u8, (52 + ton as i32 / 2 + n / 2).clamp(0, 255) as u8, (42 + n / 2).clamp(0, 255) as u8])
                    }
                });
                img.save(&z)?;
            }
            "AB_gelaender.bmp" => {
                // Geländer: Stahl, lackiert (altrosa wie am Dr.-Willi-Reiland-Ring)
                let img = image::RgbImage::from_fn(8, 8, |x, y| image::Rgb([150 + (x % 3) as u8 * 4, 80 + (y % 2) as u8 * 3, 110]));
                img.save(&z)?;
            }
            "AB_asphalt_rot.bmp" => {
                let img = image::open(marcel.join("str_asphdrk.bmp")).context("Splines\\Marcel\\texture\\str_asphdrk.bmp fehlt")?.to_rgb8();
                let rot = image::ImageBuffer::from_fn(img.width(), img.height(), |x, y| {
                    let p = img.get_pixel(x, y);
                    let l = (p[0] as f32 * 0.3 + p[1] as f32 * 0.59 + p[2] as f32 * 0.11) / 255.0;
                    image::Rgb([(70.0 + 150.0 * l) as u8, (25.0 + 60.0 * l) as u8, (22.0 + 55.0 * l) as u8])
                });
                rot.save(&z)?;
                if marcel.join("str_asphdrk.bmp.cfg").is_file() {
                    std::fs::copy(marcel.join("str_asphdrk.bmp.cfg"), ziel.join("AB_asphalt_rot.bmp.cfg"))?;
                }
            }
            _ => {
                let q = marcel.join(t);
                if !q.is_file() {
                    bail!("Textur {} fehlt", q.display());
                }
                std::fs::copy(&q, &z)?;
                for ext in [".cfg", ".surf"] {
                    let neben = marcel.join(format!("{t}{ext}"));
                    if neben.is_file() {
                        std::fs::copy(&neben, ziel.join(format!("{t}{ext}")))?;
                    }
                }
            }
        }
    }
    Ok(())
}

/// weisse Markierung mit Alpha (TGA, 32 bit): `an(x, y)` deckend, sonst durchsichtig; weiche Kanten quer
fn tga(pfad: &Path, w: u32, h: u32, an: impl Fn(u32, u32) -> bool) -> Result<()> {
    let img = image::RgbaImage::from_fn(w, h, |x, y| {
        let a = if an(x, y) { 235 } else { 0 };
        image::Rgba([242, 242, 236, a])
    });
    img.save_with_format(pfad, image::ImageFormat::Tga)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn werte(text: &str, schluessel: &str, n: usize) -> Vec<Vec<f64>> {
        let z: Vec<&str> = text.lines().collect();
        z.iter().enumerate().filter(|(_, l)| l.trim() == schluessel)
            .map(|(i, _)| (1..=n).map(|k| z[i + k].trim().parse::<f64>().unwrap()).collect()).collect()
    }

    /// Standard-Stadtstrasse: Achse in der Mitte der Fahrspuren, Pfade mit Richtungen, Bordsteine, Mittellinie
    #[test]
    fn stadtstrasse() {
        let q = Querschnitt::default();
        assert_eq!(q.spuren(), (1, 1, 2));
        assert!((q.links() + 5.75).abs() < 1e-9, "{}", q.links());
        let (text, tex) = q.sli();
        let pfade = werte(&text, "[path]", 5);
        assert_eq!(pfade.len(), 4);
        // Gehweg links, Spur zurueck links der Achse, Spur vor rechts, Gehweg rechts
        assert_eq!(pfade[0][0], 1.0);
        assert_eq!((pfade[1][0], pfade[1][4]), (0.0, 1.0));
        assert!((pfade[1][1] + 1.625).abs() < 1e-9 && (pfade[2][1] - 1.625).abs() < 1e-9);
        assert_eq!(pfade[2][4], 0.0);
        let hp = werte(&text, "[heightprofile]", 4);
        assert_eq!(hp.len(), 4);
        assert!((hp[0][0] + 5.75).abs() < 1e-9 && (hp[3][1] - 5.75).abs() < 1e-9);
        assert!(tex.contains(&"str_side1.bmp") && tex.contains(&"AB_marke_strich.tga") && tex.contains(&"str_asphdrk.bmp"));
        // genau eine Markierung (Mitte), zwei Bordsteine je Seite (Oberkante + Kante) + Raender
        assert_eq!(text.matches("Markierung:").count(), 1);
        assert_eq!(text.matches("Bordstein:").count(), 4);
    }

    /// breite Teile werden gekachelt (eine Textur deckt nur ~3,5 m), u bleibt im nutzbaren Bereich
    #[test]
    fn breiter_gehweg_gekachelt() {
        let mut q = Querschnitt::default();
        q.teile[3].breite = 9.0;
        let (text, _) = q.sli();
        let stuecke = text.matches("Gehweg:").count();
        assert!(stuecke >= 4, "{stuecke}");
        for p in werte(&text, "[profilepnt]", 4) {
            assert!((0.0..=1.0).contains(&p[2]), "u {}", p[2]);
        }
    }

    /// automatische Markierungen, Busspur, Radfahrstreifen, Einbahn mit Parkstreifen
    #[test]
    fn markierungen_und_einbahn() {
        let mut q = Querschnitt { name: "Einbahn mit Bus".into(), teile: vec![
            Teil::neu(Art::Gehweg), Teil::neu(Art::Parkstreifen), Teil::neu(Art::Fahrspur), Teil::neu(Art::Fahrspur),
            Teil::neu(Art::Busspur), Teil::neu(Art::Radfahrstreifen), Teil::neu(Art::Gehweg)], marken: vec![] };
        q.marken_anpassen();
        assert_eq!((0..6).map(|i| q.marke(i)).collect::<Vec<_>>(),
                   vec![Marke::Keine, Marke::Voll, Marke::Strich, Marke::Breitstrich, Marke::Breitstrich, Marke::Keine]);
        assert_eq!(q.spuren(), (3, 0, 2));
        q.marken[2] = Marke::Voll;
        let (text, tex) = q.sli();
        assert_eq!(text.matches("Markierung:").count(), 4);
        assert!(!tex.contains(&"AB_marke_strich.tga"));
        assert!(tex.contains(&"AB_asphalt_rot.bmp") && tex.contains(&"str_verbund1.bmp"));
        assert_eq!(q.datei(), "AB_Einbahn_mit_Bus");
        // Bauanleitung hin und zurueck
        assert_eq!(Querschnitt::aus_json(&q.json()), Some(q.clone()));
        let leer = Querschnitt { name: "x".into(), teile: vec![Teil::neu(Art::Gehweg)], marken: vec![] };
        assert!(leer.pruefen().is_err());
    }

    /// in einen Test-Ordner speichern: .sli, Bauanleitung, Texturen (aus der Installation kopiert bzw. erzeugt);
    /// openOMSI liest die Spline und findet die Spuren
    #[test]
    #[ignore]
    fn speichern_und_lesen() {
        let omsi = Path::new(crate::bearbeiten::tests::OMSI);
        let root = std::env::temp_dir().join(format!("omsi-editor-qs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Splines/Marcel")).unwrap();
        // nur die Texturen, die gebraucht werden
        std::fs::create_dir_all(root.join("Splines/Marcel/texture")).unwrap();
        for e in std::fs::read_dir(omsi.join("Splines/Marcel/texture")).unwrap().flatten() {
            if e.path().is_file() {
                std::fs::copy(e.path(), root.join("Splines/Marcel/texture").join(e.file_name())).unwrap();
            }
        }
        let mut q = Querschnitt::default();
        q.name = "Weißenburger Straße".into();
        q.teile.insert(2, Teil::neu(Art::Radfahrstreifen));
        q.marken_anpassen();
        let rel = speichern(&root, &q).unwrap();
        assert_eq!(rel, "Splines\\Aschaffenburg\\AB_Weissenburger_Strasse.sli");
        let d = ordner(&root);
        for t in ["str_side1.bmp", "str_asphdrk.bmp", "AB_marke_voll.tga", "AB_asphalt_rot.bmp", "str_gehweg02.bmp"] {
            assert!(d.join("texture").join(t).is_file(), "{t} fehlt");
        }
        assert_eq!(eigene(&root), vec![("AB_Weissenburger_Strasse".to_string(), q.clone())]);
        let def = omsi_scenery::Spline::load(&d.join("AB_Weissenburger_Strasse.sli")).unwrap();
        println!("{} Profile, {} Texturen", def.profiles.len(), def.textures.len());
        assert_eq!(def.textures.len(), 5);
        let qs = crate::strasse::querschnitte(&root);
        let e = qs.iter().find(|x| x.name == "AB_Weissenburger_Strasse").expect("in der Querschnittsliste");
        assert_eq!((e.vor, e.zurueck, e.gehwege), (1, 1, 2));
        assert!((e.breite - q.gesamtbreite() as f32).abs() < 0.01);
        std::fs::remove_dir_all(&root).ok();
    }

    /// 3D-Vorschau (OMSI_BILD): Stadtstrasse und vierspurige Strasse mit Mittelinsel, Bus- und Radstreifen
    #[test]
    #[ignore]
    fn vorschau_bilder() {
        let _sperre = crate::bearbeiten::tests::sperre();
        let omsi = Path::new(crate::bearbeiten::tests::OMSI);
        // (die Vorschau liegt wie im Editor im eigenen Spline-Ordner der Installation und wird danach entfernt)
        let mut v = crate::bearbeiten::tests::grundorf();
        let mut gross = Querschnitt { name: "Ring".into(), teile: vec![], marken: vec![] };
        for (art, r) in [(Art::Gehweg, Richtung::Vor), (Art::Radweg, Richtung::Vor), (Art::Parkstreifen, Richtung::Vor), (Art::Busspur, Richtung::Zurueck),
                         (Art::Fahrspur, Richtung::Zurueck), (Art::Mittelinsel, Richtung::Vor), (Art::Fahrspur, Richtung::Vor), (Art::Fahrspur, Richtung::Vor),
                         (Art::Radfahrstreifen, Richtung::Vor), (Art::Gehweg, Richtung::Vor)] {
            let mut t = Teil::neu(art);
            t.richtung = r;
            gross.teile.push(t);
        }
        gross.marken_anpassen();
        for (name, q) in [("stadt", Querschnitt::default()), ("ring", gross)] {
            let datei = format!("_vorschau_{name}");
            let abs = schreiben(omsi, &q, &datei).unwrap();
            v.forget_spline_type(&abs);
            let lanes = v.spline_lanes(&abs).expect("Spline nicht lesbar");
            println!("{name}: {} Pfade, Breite links/rechts {:?}", lanes.0.len(), lanes.1);
            let px = v.preview_spline_image(&abs, 640).expect("kein Bild");
            if let Some(b) = std::env::var_os("OMSI_BILD") {
                image::save_buffer(std::path::PathBuf::from(&b).with_extension(format!("{name}.png")), &px, 640, 640, image::ColorType::Rgba8).unwrap();
            }
            let _ = std::fs::remove_file(ordner(omsi).join(format!("{datei}.sli")));
        }
    }
}
