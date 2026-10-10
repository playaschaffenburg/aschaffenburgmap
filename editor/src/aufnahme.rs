//! Bauaufnahme: was am echten Bauwerk wirklich ist - nachgezeichnet ueber dem Luftbild (Bordkanten, Gelaender, Mauern,
//! Markierungen, Furten, Sperrflaechen, Treppen) und Notizen mit Links zu freien Strassenbildern (Mapillary, Panoramax).
//!
//! Die Aufnahme ist die Vorlage, nicht der Bauplan: der Generator erfindet nichts, was hier fehlt, und der Abgleich
//! zeigt, wo das Gebaute (Bauplan und Waende des letzten Baus) von der Aufnahme abweicht oder etwas fehlt. Gespeichert
//! neben dem Bauplan (`sonderbau/ring/aufnahme.json`).

use glam::DVec2;
use std::path::Path;

/// Arten der Aufnahme: (Schluessel, Name, geschlossen, Toleranz m im Abgleich, nur auf der Platte pruefen)
pub const ARTEN: [(&str, &str, bool, f64, bool); 11] = [
    ("bord", "Bordkante (Gehweg/Insel)", false, 0.35, true),
    ("gelaender", "Gelaender", false, 0.5, false),
    ("mauer", "Mauer / Wandkrone", false, 0.6, false),
    ("randlinie", "Markierung durchgezogen", false, 0.3, true),
    ("leitlinie", "Markierung gestrichelt", false, 0.3, true),
    ("haltlinie", "Haltlinie", false, 0.5, true),
    ("furt", "Fussgaengerfurt (Mittellinie)", false, 1.0, true),
    ("sperr", "Sperrflaeche (Umriss)", true, 0.4, true),
    ("radfurt", "Radfurt (Umriss)", true, 0.4, true),
    ("treppe", "Treppe / Fussweg-Rampe", false, 0.0, false),
    ("sonstiges", "Sonstiges", false, 0.0, false),
];

pub fn art_index(art: &str) -> usize {
    ARTEN.iter().position(|a| a.0 == art).unwrap_or(ARTEN.len() - 1)
}

pub fn art_name(art: &str) -> &'static str {
    ARTEN[art_index(art)].1
}

pub fn geschlossen(art: &str) -> bool {
    ARTEN[art_index(art)].2
}

/// baut der Generator diese Art (sonst nur Hinweis im Abgleich)?
pub fn baubar(art: &str) -> bool {
    ARTEN[art_index(art)].3 > 0.0
}

pub fn farbe(art: &str) -> egui::Color32 {
    match art {
        "bord" => egui::Color32::from_rgb(255, 200, 60),
        "gelaender" => egui::Color32::from_rgb(255, 90, 200),
        "mauer" => egui::Color32::from_rgb(255, 120, 40),
        "randlinie" | "leitlinie" => egui::Color32::from_rgb(90, 230, 255),
        "haltlinie" => egui::Color32::from_rgb(140, 160, 255),
        "furt" => egui::Color32::from_rgb(120, 255, 140),
        "sperr" => egui::Color32::from_rgb(200, 255, 90),
        "radfurt" => egui::Color32::from_rgb(255, 110, 90),
        "treppe" => egui::Color32::from_rgb(200, 160, 255),
        _ => egui::Color32::from_rgb(230, 230, 230),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ALinie {
    pub id: String,
    pub art: String,
    pub punkte: Vec<DVec2>,
    pub notiz: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Notiz {
    pub id: String,
    pub pos: DVec2,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Aufnahme {
    pub linien: Vec<ALinie>,
    pub notizen: Vec<Notiz>,
    /// Arten, die vollstaendig aufgenommen sind: dort meldet der Abgleich auch Gebautes, das in der Aufnahme fehlt
    pub fertig: Vec<String>,
    pub zaehler: u64,
}

fn p2(v: &serde_json::Value) -> DVec2 {
    DVec2::new(v[0].as_f64().unwrap_or(0.0), v[1].as_f64().unwrap_or(0.0))
}

fn j2(p: DVec2) -> serde_json::Value {
    serde_json::json!([(p.x * 1000.0).round() / 1000.0, (p.y * 1000.0).round() / 1000.0])
}

fn s(v: &serde_json::Value, k: &str) -> String {
    v[k].as_str().unwrap_or("").to_string()
}

impl Aufnahme {
    pub fn aus_json(j: &serde_json::Value) -> Aufnahme {
        let liste = |k: &str| j[k].as_array().cloned().unwrap_or_default();
        Aufnahme {
            linien: liste("linien").iter().map(|v| ALinie {
                id: s(v, "id"), art: s(v, "art"), notiz: s(v, "notiz"),
                punkte: v["punkte"].as_array().map(|a| a.iter().map(p2).collect()).unwrap_or_default(),
            }).collect(),
            notizen: liste("notizen").iter().map(|v| Notiz { id: s(v, "id"), pos: p2(&v["pos"]), text: s(v, "text") }).collect(),
            fertig: liste("fertig").iter().filter_map(|x| x.as_str().map(|x| x.to_string())).collect(),
            zaehler: j["zaehler"].as_u64().unwrap_or(0),
        }
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "version": 1,
            "linien": self.linien.iter().map(|l| serde_json::json!({
                "id": l.id, "art": l.art, "punkte": l.punkte.iter().map(|p| j2(*p)).collect::<Vec<_>>(), "notiz": l.notiz,
            })).collect::<Vec<_>>(),
            "notizen": self.notizen.iter().map(|n| serde_json::json!({"id": n.id, "pos": j2(n.pos), "text": n.text})).collect::<Vec<_>>(),
            "fertig": self.fertig,
            "zaehler": self.zaehler,
        })
    }

    pub fn lesen(datei: &Path) -> Aufnahme {
        std::fs::read(datei).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()).map(|j| Aufnahme::aus_json(&j)).unwrap_or_default()
    }

    pub fn schreiben(&self, datei: &Path) -> Result<(), String> {
        let text = serde_json::to_string_pretty(&self.json()).map_err(|e| e.to_string())?;
        std::fs::write(datei, text).map_err(|e| format!("{}: {e}", datei.display()))
    }

    pub fn neue_id(&mut self, art: &str) -> String {
        self.zaehler += 1;
        format!("a_{art}{}", self.zaehler)
    }

    pub fn ist_fertig(&self, art: &str) -> bool {
        self.fertig.iter().any(|x| x == art)
    }
}

// ------------------------------------------------------------------ Abgleich
/// gebautes Teil fuer den Abgleich: Art wie in der Aufnahme, Linienzug (geschlossene Umrisse mit erstem Punkt am Ende)
#[derive(Clone, Debug)]
pub struct Gebaut {
    pub art: &'static str,
    pub punkte: Vec<DVec2>,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Abweichung {
    /// "fehlt" (Aufnahme ohne Gebautes), "zuviel" (Gebautes ohne Aufnahme), "nicht_baubar"
    pub was: &'static str,
    pub art: String,
    pub text: String,
    pub pos: DVec2,
    pub laenge: f64,
    /// Aufnahme-Linie (fehlt, nicht_baubar) bzw. gebautes Teil (zuviel)
    pub bezug: String,
}

const SCHRITT: f64 = 0.5;
const MIN_LAENGE: f64 = 1.0;

/// Punkte alle `schritt` m entlang des Linienzugs (mit Anfang und Ende)
pub fn proben(pts: &[DVec2], schritt: f64) -> Vec<DVec2> {
    let mut out = Vec::new();
    for w in pts.windows(2) {
        let l = (w[1] - w[0]).length();
        let n = (l / schritt).ceil().max(1.0) as usize;
        for k in 0..n {
            out.push(w[0] + (w[1] - w[0]) * (k as f64 / n as f64));
        }
    }
    if let Some(p) = pts.last() {
        out.push(*p);
    }
    out
}

fn abstand_strecke(p: DVec2, a: DVec2, b: DVec2) -> f64 {
    let ab = b - a;
    let t = if ab.length_squared() > 1e-12 { ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
    (a + ab * t - p).length()
}

fn abstand_zug(p: DVec2, pts: &[DVec2]) -> f64 {
    match pts.len() {
        0 => f64::MAX,
        1 => (pts[0] - p).length(),
        _ => pts.windows(2).map(|w| abstand_strecke(p, w[0], w[1])).fold(f64::MAX, f64::min),
    }
}

fn abstand_art<'a>(p: DVec2, teile: impl Iterator<Item = &'a [DVec2]>) -> f64 {
    teile.map(|t| abstand_zug(p, t)).fold(f64::MAX, f64::min)
}

fn zu(pts: &[DVec2], geschl: bool) -> Vec<DVec2> {
    let mut v = pts.to_vec();
    if geschl && v.len() > 2 && v.first() != v.last() {
        v.push(v[0]);
    }
    v
}

/// zusammenhaengende Stuecke der Proben, fuer die `schlecht` gilt -> (erste, letzte Probe, groesster Wert)
fn stuecke(n: usize, schlecht: impl Fn(usize) -> Option<f64>) -> Vec<(usize, usize, f64)> {
    let mut out = Vec::new();
    let mut lauf: Option<(usize, f64)> = None;
    for i in 0..=n {
        let s = if i < n { schlecht(i) } else { None };
        match (s, lauf) {
            (Some(d), None) => lauf = Some((i, d)),
            (Some(d), Some((a, m))) => lauf = Some((a, m.max(d))),
            (None, Some((a, m))) => {
                out.push((a, i - 1, m));
                lauf = None;
            }
            (None, None) => {}
        }
    }
    out
}

/// Stuecke mit hoechstens `luecke` passenden Proben dazwischen zusammenfassen (sonst zerfaellt eine schraeg liegende
/// Linie in viele kleine Meldungen)
fn zusammen(v: Vec<(usize, usize, f64)>, luecke: usize) -> Vec<(usize, usize, f64)> {
    let mut out: Vec<(usize, usize, f64)> = Vec::new();
    for x in v {
        match out.last_mut() {
            Some(l) if x.0 <= l.1 + luecke + 1 => {
                l.1 = x.1;
                l.2 = l.2.max(x.2);
            }
            _ => out.push(x),
        }
    }
    out
}

const LUECKE: usize = 4;

fn teile<'a>(gebaut: &'a [Gebaut], art: &'a str) -> impl Iterator<Item = &'a [DVec2]> + 'a {
    gebaut.iter().filter(move |g| g.art == art).map(|g| g.punkte.as_slice())
}

fn verwandt(art: &str) -> Option<&'static str> {
    match art {
        "randlinie" => Some("leitlinie"),
        "leitlinie" => Some("randlinie"),
        _ => None,
    }
}

/// Aufnahme gegen das Gebaute: je Aufnahme-Linie die Stuecke, die weiter als die Toleranz vom Gebauten gleicher Art
/// liegen (fehlt/weicht ab); fuer vollstaendig aufgenommene Arten auch Gebautes ohne Aufnahme (zuviel), nur in der Naehe
/// der Aufnahme. `pruefbar(p)`: liegt p auf der Kreuzungsplatte (Markierungen, Borde usw. werden nur dort verglichen,
/// die anschliessenden Strassen kommen aus dem Baukasten)
pub fn abgleich(a: &Aufnahme, gebaut: &[Gebaut], pruefbar: &dyn Fn(DVec2) -> bool) -> Vec<Abweichung> {
    let mut out = Vec::new();
    for l in &a.linien {
        let (_, name, geschl, tol, nur_platte) = ARTEN[art_index(&l.art)];
        if l.punkte.len() < 2 {
            continue;
        }
        if tol <= 0.0 {
            out.push(Abweichung { was: "nicht_baubar", art: l.art.clone(), text: format!("{name}: baut der Generator noch nicht"),
                                  pos: l.punkte[l.punkte.len() / 2], laenge: 0.0, bezug: l.id.clone() });
            continue;
        }
        let pr = proben(&zu(&l.punkte, geschl), SCHRITT);
        let d: Vec<Option<f64>> = pr.iter().map(|p| {
            if nur_platte && !pruefbar(*p) { None } else { Some(abstand_art(*p, teile(gebaut, &l.art))) }
        }).collect();
        for (i0, i1, m) in zusammen(stuecke(pr.len(), |i| d[i].filter(|x| *x > tol)), LUECKE) {
            let laenge = (i1 - i0) as f64 * SCHRITT + SCHRITT;
            if laenge < MIN_LAENGE {
                continue;
            }
            let anders = verwandt(&l.art).filter(|v| (i0..=i1).filter(|&i| abstand_art(pr[i], teile(gebaut, v)) <= tol).count() * 2 > i1 - i0 + 1);
            let text = match anders {
                Some(v) => format!("{name}: {laenge:.1} m gebaut als {}", art_name(v).to_lowercase()),
                None if m > 3.0 => format!("{name}: {laenge:.1} m fehlen"),
                None => format!("{name}: {laenge:.1} m liegen bis {m:.1} m daneben"),
            };
            out.push(Abweichung { was: "fehlt", art: l.art.clone(), text, pos: pr[(i0 + i1) / 2], laenge, bezug: l.id.clone() });
        }
    }
    // Gebautes ohne Aufnahme (nur vollstaendig aufgenommene Arten, nur bis 20 m an der Aufnahme)
    let alle: Vec<Vec<DVec2>> = a.linien.iter().map(|l| zu(&l.punkte, geschlossen(&l.art))).collect();
    for g in gebaut {
        if !a.ist_fertig(g.art) {
            continue;
        }
        let (_, name, _, tol, nur_platte) = ARTEN[art_index(g.art)];
        let auf: Vec<&[DVec2]> = a.linien.iter().zip(&alle).filter(|(l, _)| l.art == g.art).map(|(_, z)| z.as_slice()).collect();
        let pr = proben(&g.punkte, SCHRITT);
        let schlecht = |i: usize| {
            let p = pr[i];
            if nur_platte && !pruefbar(p) {
                return None;
            }
            let dn = abstand_art(p, alle.iter().map(|z| z.as_slice()));
            let d = abstand_art(p, auf.iter().copied());
            (dn < 20.0 && d > tol).then_some(d)
        };
        for (i0, i1, _) in zusammen(stuecke(pr.len(), schlecht), LUECKE) {
            let laenge = (i1 - i0) as f64 * SCHRITT + SCHRITT;
            if laenge < MIN_LAENGE {
                continue;
            }
            out.push(Abweichung { was: "zuviel", art: g.art.to_string(), text: format!("{name}: {laenge:.1} m gebaut, aber nicht in der Aufnahme ({})", g.name),
                                  pos: pr[(i0 + i1) / 2], laenge, bezug: g.name.clone() });
        }
    }
    out
}

// ------------------------------------------------------------------ Strassenbilder
/// UTM (Nordhalbkugel) -> (Breite, Laenge), Krueger-Reihen wie omsigen/utm.py
pub fn utm_zu_wgs(ost: f64, nord: f64, zone: u32) -> (f64, f64) {
    let a: f64 = 6378137.0;
    let f: f64 = 1.0 / 298.257223563;
    let k0 = 0.9996;
    let n = f / (2.0 - f);
    let aa = a / (1.0 + n) * (1.0 + n * n / 4.0 + n.powi(4) / 64.0);
    let beta = [n / 2.0 - 2.0 * n * n / 3.0 + 37.0 * n.powi(3) / 96.0, n * n / 48.0 + n.powi(3) / 15.0, 17.0 * n.powi(3) / 480.0];
    let delta = [2.0 * n - 2.0 * n * n / 3.0 - 2.0 * n.powi(3), 7.0 * n * n / 3.0 - 8.0 * n.powi(3) / 5.0, 56.0 * n.powi(3) / 15.0];
    let xi = nord / (k0 * aa);
    let eta = (ost - 500000.0) / (k0 * aa);
    let (mut xi_, mut eta_) = (xi, eta);
    for (j, b) in beta.iter().enumerate() {
        let j = (j + 1) as f64;
        xi_ -= b * (2.0 * j * xi).sin() * (2.0 * j * eta).cosh();
        eta_ -= b * (2.0 * j * xi).cos() * (2.0 * j * eta).sinh();
    }
    let chi = (xi_.sin() / eta_.cosh()).asin();
    let phi = chi + delta.iter().enumerate().map(|(j, d)| d * (2.0 * (j + 1) as f64 * chi).sin()).sum::<f64>();
    let lam = eta_.sinh().atan2(xi_.cos());
    (phi.to_degrees(), zone as f64 * 6.0 - 183.0 + lam.to_degrees())
}

/// freie Strassenbilder an dieser Stelle (Breite, Laenge): (Name, URL)
pub fn bilder_links(lat: f64, lon: f64) -> [(&'static str, String); 2] {
    [
        ("Mapillary", format!("https://www.mapillary.com/app/?lat={lat:.6}&lng={lon:.6}&z=19")),
        ("Panoramax", format!("https://api.panoramax.xyz/#focus=map&map=19/{lat:.6}/{lon:.6}")),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linie(art: &str, pts: &[(f64, f64)]) -> ALinie {
        ALinie { id: format!("a_{art}"), art: art.into(), punkte: pts.iter().map(|p| DVec2::new(p.0, p.1)).collect(), notiz: String::new() }
    }

    fn gebaut(art: &'static str, pts: &[(f64, f64)]) -> Gebaut {
        Gebaut { art, punkte: pts.iter().map(|p| DVec2::new(p.0, p.1)).collect(), name: format!("g_{art}") }
    }

    #[test]
    fn hin_und_zurueck() {
        let mut a = Aufnahme::default();
        let id = a.neue_id("gelaender");
        a.linien.push(ALinie { id, art: "gelaender".into(), punkte: vec![DVec2::new(1.0, 2.0), DVec2::new(3.5, -1.25)], notiz: "Stahl, gruen".into() });
        a.notizen.push(Notiz { id: "n1".into(), pos: DVec2::new(5.0, 6.0), text: "Treppe zur Ludwigsallee".into() });
        a.fertig.push("gelaender".into());
        let b = Aufnahme::aus_json(&a.json());
        assert_eq!(a, b);
    }

    #[test]
    fn abgleich_fehlt_daneben_und_zuviel() {
        let mut a = Aufnahme::default();
        a.linien.push(linie("gelaender", &[(0.0, 0.0), (20.0, 0.0)]));
        a.linien.push(linie("leitlinie", &[(0.0, 10.0), (10.0, 10.0)]));
        a.linien.push(linie("treppe", &[(0.0, 30.0), (3.0, 30.0)]));
        let g = vec![
            gebaut("gelaender", &[(0.0, 0.1), (12.0, 0.1)]),        // 12 m passen, 8 m fehlen
            gebaut("randlinie", &[(0.0, 10.05), (10.0, 10.05)]),    // gestrichelt erwartet, durchgezogen gebaut
            gebaut("gelaender", &[(0.0, 5.0), (6.0, 5.0)]),         // nicht in der Aufnahme
        ];
        let alles = |_: DVec2| true;
        let erg = abgleich(&a, &g, &alles);
        let fehlt: Vec<_> = erg.iter().filter(|x| x.was == "fehlt").collect();
        assert_eq!(fehlt.len(), 2, "{erg:#?}");
        assert!(fehlt[0].text.contains("fehlen") && (fehlt[0].laenge - 8.0).abs() < 1.1, "{:?}", fehlt[0]);
        assert!(fehlt[1].text.contains("gebaut als markierung durchgezogen"), "{:?}", fehlt[1]);
        assert_eq!(erg.iter().filter(|x| x.was == "nicht_baubar").count(), 1);
        // erst wenn die Gelaender vollstaendig aufgenommen sind, zaehlt das zusaetzliche als zuviel
        assert_eq!(erg.iter().filter(|x| x.was == "zuviel").count(), 0);
        a.fertig.push("gelaender".into());
        let erg = abgleich(&a, &g, &alles);
        let zuviel: Vec<_> = erg.iter().filter(|x| x.was == "zuviel").collect();
        assert_eq!(zuviel.len(), 1, "{erg:#?}");
        assert!((zuviel[0].pos.y - 5.0).abs() < 1e-9);
        // Markierungen nur auf der Platte: ausserhalb wird nicht verglichen
        let platte = |p: DVec2| p.y < 5.0;
        assert_eq!(abgleich(&a, &g, &platte).iter().filter(|x| x.art == "leitlinie").count(), 0);
    }

    #[test]
    fn utm_wie_omsigen() {
        // python: omsigen.utm.from_utm(511550, 5536035) = (49.97654223076167, 9.161083543857506)
        let (lat, lon) = utm_zu_wgs(511550.0, 5536035.0, 32);
        assert!((lat - 49.97654223076167).abs() < 1e-9 && (lon - 9.161083543857506).abs() < 1e-9, "{lat} {lon}");
    }
}
