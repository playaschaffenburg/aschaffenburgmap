//! Strassennetz des Editors (Meilenstein 3): Knoten und Kanten, aus denen OMSI-Splines werden.
//!
//! Konvention wie in openOMSI/OMSI: x Ost, y Nord, z Hoehe; Richtung (Grad) im Uhrzeigersinn ab Nord;
//! Radius > 0 = Rechtskurve. Eine Kante besteht aus OMSI-Elementen (Gerade oder Kreisbogen) und endet genau auf
//! ihren Knoten in der dort geltenden Richtung - nur dann verbindet OMSI die Spuren (Lage UND Richtung muessen
//! passen). Hoehe: Knoten haben feste Hoehen, dazwischen ein glatter Verlauf (kubisch mit den Steigungen an den
//! Knoten). Je Element als `[spline_h]`: Starthoehe, Steigung Anfang/Ende in Prozent und Hoehenunterschied - OMSI
//! rechnet dann selbst kubisch zwischen den Steigungen (beim einfachen `[spline]` waere es eine Parabel, die den
//! Verlauf nicht genau trifft: Stufen zwischen den Elementen).

use glam::{DVec2, DVec3};

pub const MAX_ELEMENT: f64 = 50.0; // laengere Elemente werden geteilt (Hoehenverlauf)

/// Richtungsvektor (waagerecht) fuer eine Richtung in Grad
pub fn dir(h: f64) -> DVec2 {
    let r = h.to_radians();
    DVec2::new(r.sin(), r.cos())
}

/// rechte Seite
pub fn rechts(h: f64) -> DVec2 {
    let r = h.to_radians();
    DVec2::new(r.cos(), -r.sin())
}

/// Richtung von a nach b
pub fn richtung(a: DVec2, b: DVec2) -> f64 {
    (b.x - a.x).atan2(b.y - a.y).to_degrees().rem_euclid(360.0)
}

/// Winkel in (-180, 180]
pub fn norm180(a: f64) -> f64 {
    let x = (a + 180.0).rem_euclid(360.0) - 180.0;
    if x == -180.0 { 180.0 } else { x }
}

/// Ein OMSI-Element in der Ebene: Start, Richtung, Laenge, Radius (0 = Gerade)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stueck {
    pub start: DVec2,
    pub richtung: f64,
    pub laenge: f64,
    pub radius: f64,
}

impl Stueck {
    /// Punkt und Richtung nach t Metern
    pub fn bei(&self, t: f64) -> (DVec2, f64) {
        if self.radius == 0.0 {
            return (self.start + dir(self.richtung) * t, self.richtung);
        }
        let c = self.start + rechts(self.richtung) * self.radius;
        let h2 = self.richtung + (t / self.radius).to_degrees();
        (c - rechts(h2) * self.radius, h2.rem_euclid(360.0))
    }

    pub fn ende(&self) -> (DVec2, f64) {
        self.bei(self.laenge)
    }
}

/// Bogen ab p in Richtung h, der durch q geht (Tangente in p) -> Stueck (Gerade, wenn q geradeaus liegt)
pub fn bogen_durch(p: DVec2, h: f64, q: DVec2) -> Stueck {
    let d = q - p;
    let l = d.length();
    let alpha = norm180(richtung(p, q) - h).to_radians(); // > 0: q liegt rechts
    if l < 1e-9 || alpha.abs() < 1e-6 {
        return Stueck { start: p, richtung: h, laenge: l, radius: 0.0 };
    }
    let r = l / (2.0 * alpha.sin());
    Stueck { start: p, richtung: h, laenge: (r * 2.0 * alpha).abs(), radius: r }
}

/// Weg von (a, ha) nach (b, hb), tangential an beiden Enden: Gerade, Bogen oder Bogenpaar (Biarc)
pub fn verbinden(a: DVec2, ha: f64, b: DVec2, hb: f64) -> Vec<Stueck> {
    let v = b - a;
    if v.length() < 1e-6 {
        return vec![];
    }
    let gerade = richtung(a, b);
    if norm180(gerade - ha).abs() < 1e-4 && norm180(gerade - hb).abs() < 1e-4 {
        return vec![Stueck { start: a, richtung: ha, laenge: v.length(), radius: 0.0 }];
    }
    // ein Bogen reicht, wenn er in b schon die Richtung hb hat
    let ein = bogen_durch(a, ha, b);
    if norm180(ein.ende().1 - hb).abs() < 1e-3 {
        return vec![ein];
    }
    // Biarc mit gleich langen Tangenten (d): Verbindungspunkt m
    let t1 = dir(ha);
    let t2 = dir(hb);
    let t = t1 + t2;
    let k = 2.0 * (1.0 - t1.dot(t2));
    let d = if k.abs() < 1e-9 {
        v.dot(v) / (4.0 * v.dot(t2))
    } else {
        (-v.dot(t) + (v.dot(t).powi(2) + k * v.dot(v)).sqrt()) / k
    };
    let m = (a + t1 * d + b - t2 * d) * 0.5;
    let s1 = bogen_durch(a, ha, m);
    let (m2, hm) = s1.ende();
    let s2 = bogen_durch(m2, hm, b);
    [s1, s2].into_iter().filter(|s| s.laenge > 1e-6).collect()
}

/// Element mit Hoehe, fertig fuer einen [spline]-Eintrag
#[derive(Clone, Copy, Debug)]
pub struct Element {
    pub stueck: Stueck,
    pub z: f64,
    /// Steigung Anfang/Ende in Prozent
    pub stg_a: f64,
    pub stg_e: f64,
    /// Hoehenunterschied ueber das Element ([spline_h])
    pub dh: f64,
}

impl Element {
    pub fn kurve(&self, seed: u32, tex_offset: f64) -> omsi_geometry::SplineCurve {
        omsi_geometry::SplineCurve {
            start: DVec3::new(self.stueck.start.x, self.stueck.start.y, self.z),
            heading_deg: self.stueck.richtung,
            length: self.stueck.laenge,
            radius: self.stueck.radius,
            grad_start: self.stg_a,
            grad_end: self.stg_e,
            delta_h: Some(self.dh),
            cant_start: 0.0,
            cant_end: 0.0,
            skew_start: 0.0,
            skew_end: 0.0,
            tex_offset,
            seed,
            half_cant_width: 0.0,
        }
    }
}

/// Stuecke in Elemente <= MAX_ELEMENT teilen und den Hoehenverlauf daraufsetzen: kubisch von za (Steigung ga) nach
/// zb (Steigung gb), Steigungen als Verhaeltnis (0.05 = 5 %)
pub fn mit_hoehe(stuecke: &[Stueck], za: f64, ga: f64, zb: f64, gb: f64) -> Vec<Element> {
    let mut teile = Vec::new();
    for s in stuecke {
        let n = (s.laenge / MAX_ELEMENT).ceil().max(1.0) as usize;
        let l = s.laenge / n as f64;
        let mut cur = *s;
        for _ in 0..n {
            let st = Stueck { laenge: l, ..cur };
            let (p, h) = st.ende();
            teile.push(st);
            cur = Stueck { start: p, richtung: h, ..cur };
        }
    }
    let gesamt: f64 = teile.iter().map(|s| s.laenge).sum();
    if gesamt <= 0.0 {
        return vec![];
    }
    // kubische Hermite-Kurve ueber s in [0, L]
    let z = |s: f64| {
        let t = s / gesamt;
        let (h00, h10, h01, h11) = (2.0 * t.powi(3) - 3.0 * t * t + 1.0, t.powi(3) - 2.0 * t * t + t, -2.0 * t.powi(3) + 3.0 * t * t, t.powi(3) - t * t);
        h00 * za + h10 * gesamt * ga + h01 * zb + h11 * gesamt * gb
    };
    let dz = |s: f64| {
        let t = s / gesamt;
        let (d00, d10, d01, d11) = (6.0 * t * t - 6.0 * t, 3.0 * t * t - 4.0 * t + 1.0, -6.0 * t * t + 6.0 * t, 3.0 * t * t - 2.0 * t);
        (d00 * za + d01 * zb) / gesamt + d10 * ga + d11 * gb
    };
    let mut s0 = 0.0;
    teile
        .into_iter()
        .map(|st| {
            let e = Element { stueck: st, z: z(s0), stg_a: dz(s0) * 100.0, stg_e: dz(s0 + st.laenge) * 100.0,
                              dh: z(s0 + st.laenge) - z(s0) };
            s0 += st.laenge;
            e
        })
        .collect()
}

// ---------------------------------------------------------------------- Netz

#[derive(Clone, Debug, PartialEq)]
pub struct Knoten {
    pub id: u32,
    pub pos: DVec3,
    /// Anschluss an eine vorhandene Strasse der Karte: (Richtung weg von ihr, Steigung in dieser Richtung)
    pub anschluss: Option<(f64, f64)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Kante {
    pub id: u32,
    pub a: u32,
    pub b: u32,
    /// .sli relativ zum OMSI-Ordner
    pub sli: String,
    /// Richtung beim Verlassen von a und beim Ankommen in b
    pub ha: f64,
    pub hb: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Netz {
    pub knoten: Vec<Knoten>,
    pub kanten: Vec<Kante>,
    naechste: u32,
}

impl Netz {
    fn neue_id(&mut self) -> u32 {
        self.naechste += 1;
        self.naechste
    }

    pub fn knoten(&self, id: u32) -> Option<&Knoten> {
        self.knoten.iter().find(|k| k.id == id)
    }

    #[allow(dead_code)]
    pub fn kante(&self, id: u32) -> Option<&Kante> {
        self.kanten.iter().find(|k| k.id == id)
    }

    pub fn knoten_neu(&mut self, pos: DVec3) -> u32 {
        let id = self.neue_id();
        self.knoten.push(Knoten { id, pos, anschluss: None });
        id
    }

    /// Knoten am Ende einer vorhandenen Strasse: neue Kanten muessen ihn in `richtung` verlassen
    pub fn anschluss_neu(&mut self, pos: DVec3, richtung: f64, steigung: f64) -> u32 {
        let id = self.neue_id();
        self.knoten.push(Knoten { id, pos, anschluss: Some((richtung, steigung)) });
        id
    }

    pub fn kante_neu(&mut self, a: u32, b: u32, sli: &str, ha: f64, hb: f64) -> u32 {
        let id = self.neue_id();
        self.kanten.push(Kante { id, a, b, sli: sli.to_string(), ha, hb });
        id
    }

    /// Kanten am Knoten
    pub fn an(&self, k: u32) -> Vec<&Kante> {
        self.kanten.iter().filter(|e| e.a == k || e.b == k).collect()
    }

    /// Richtung, in der eine neue Kante den Knoten verlassen muss, damit sie an die eine vorhandene Kante
    /// anschliesst (None: freies Ende ohne Kante oder Kreuzung)
    pub fn weiter_richtung(&self, k: u32) -> Option<f64> {
        let an = self.an(k);
        let anschluss = self.knoten(k).and_then(|x| x.anschluss);
        if an.len() + anschluss.is_some() as usize != 1 {
            return None;
        }
        if let Some((h, _)) = anschluss {
            return Some(h);
        }
        let e = an[0];
        Some(if e.b == k { e.hb } else { (e.ha + 180.0).rem_euclid(360.0) })
    }

    /// Steigung (Verhaeltnis) in Fahrtrichtung der Kante e am Knoten k: Mittel der Sehnensteigungen der
    /// Kanten am Knoten (glatter Verlauf ueber Verbindungsknoten), an Enden die eigene Sehnensteigung
    fn steigung(&self, e: &Kante, k: u32) -> f64 {
        let sehne = |e: &Kante| -> f64 {
            let (a, b) = (self.knoten(e.a).unwrap().pos, self.knoten(e.b).unwrap().pos);
            let l = (b.truncate() - a.truncate()).length().max(1.0);
            (b.z - a.z) / l
        };
        let an = self.an(k);
        if let (Some((_, g)), 1) = (self.knoten(k).and_then(|x| x.anschluss), an.len()) {
            // an der vorhandenen Strasse: deren Steigung (verlaesst e den Knoten, gilt sie so; kommt e an, umgekehrt)
            return if e.a == k { g } else { -g };
        }
        if an.len() == 2 {
            // Steigung in Richtung von e: die andere Kante zaehlt mit ihrem Vorzeichen in derselben Fahrtrichtung
            let andere = if an[0].id == e.id { an[1] } else { an[0] };
            let s_andere = if (andere.b == k) == (e.a == k) { sehne(andere) } else { -sehne(andere) };
            return 0.5 * (sehne(e) + s_andere);
        }
        sehne(e)
    }

    /// Elemente einer Kante (Lage aus ha/hb, Hoehe glatt)
    pub fn elemente(&self, e: &Kante) -> Vec<Element> {
        let (Some(a), Some(b)) = (self.knoten(e.a), self.knoten(e.b)) else { return vec![] };
        let st = verbinden(a.pos.truncate(), e.ha, b.pos.truncate(), e.hb);
        mit_hoehe(&st, a.pos.z, self.steigung(e, e.a), b.pos.z, self.steigung(e, e.b))
    }

    /// Kante loeschen; Knoten ohne Kanten verschwinden mit
    pub fn kante_loeschen(&mut self, id: u32) {
        self.kanten.retain(|e| e.id != id);
        let benutzt: std::collections::HashSet<u32> = self.kanten.iter().flat_map(|e| [e.a, e.b]).collect();
        self.knoten.retain(|k| benutzt.contains(&k.id));
    }

    /// Knoten verschieben: die Kanten folgen; an Verbindungsknoten bleibt die Richtung durchgehend
    #[allow(dead_code)] // Knoten ziehen: naechster Schritt
    pub fn knoten_setzen(&mut self, id: u32, pos: DVec3) {
        if let Some(k) = self.knoten.iter_mut().find(|k| k.id == id) {
            k.pos = pos;
        }
    }

    /// naechster Knoten in r Metern (waagerecht)
    pub fn knoten_bei(&self, p: DVec2, r: f64) -> Option<u32> {
        self.knoten
            .iter()
            .map(|k| ((k.pos.truncate() - p).length(), k.id))
            .filter(|(d, _)| *d <= r)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|x| x.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nah(a: DVec2, b: DVec2) -> bool {
        (a - b).length() < 1e-6
    }

    #[test]
    fn gerade_und_bogen() {
        let s = Stueck { start: DVec2::ZERO, richtung: 90.0, laenge: 10.0, radius: 0.0 };
        assert!(nah(s.ende().0, DVec2::new(10.0, 0.0)));
        // Viertelkreis rechts herum: nach Norden starten, nach Osten enden
        let r = 20.0;
        let b = Stueck { start: DVec2::ZERO, richtung: 0.0, laenge: r * std::f64::consts::FRAC_PI_2, radius: r };
        let (p, h) = b.ende();
        assert!(nah(p, DVec2::new(20.0, 20.0)), "{p:?}");
        assert!((h - 90.0).abs() < 1e-9);
    }

    #[test]
    fn bogen_trifft_ziel() {
        for q in [DVec2::new(30.0, 40.0), DVec2::new(-25.0, 10.0), DVec2::new(5.0, -3.0)] {
            let s = bogen_durch(DVec2::new(1.0, 2.0), 15.0, q);
            assert!(nah(s.ende().0, q), "{q:?} -> {:?}", s.ende());
        }
    }

    #[test]
    fn biarc_tangential_an_beiden_enden() {
        for (b, hb) in [(DVec2::new(50.0, 80.0), 90.0), (DVec2::new(-40.0, 30.0), 200.0), (DVec2::new(0.0, 100.0), 0.0),
                        (DVec2::new(30.0, 60.0), 10.0)] {
            let st = verbinden(DVec2::ZERO, 0.0, b, hb);
            assert!(!st.is_empty());
            let (p, h) = st.last().unwrap().ende();
            assert!(nah(p, b), "Ende {p:?} statt {b:?}");
            assert!(norm180(h - hb).abs() < 1e-6, "Richtung {h} statt {hb}");
            assert!((st[0].richtung - 0.0).abs() < 1e-9);
            // lueckenlos
            for w in st.windows(2) {
                let (e, he) = w[0].ende();
                assert!(nah(e, w[1].start) && norm180(he - w[1].richtung).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn hoehe_glatt_und_genau() {
        let st = verbinden(DVec2::ZERO, 0.0, DVec2::new(0.0, 120.0), 0.0);
        let el = mit_hoehe(&st, 10.0, 0.0, 16.0, 0.0);
        assert_eq!(el.len(), 3); // 120 m in Stuecken <= 50 m
        assert!((el[0].z - 10.0).abs() < 1e-9 && el[0].stg_a.abs() < 1e-9);
        assert!(el.last().unwrap().stg_e.abs() < 1e-9);
        // Elementgrenzen: Steigung durchgehend
        for w in el.windows(2) {
            assert!((w[0].stg_e - w[1].stg_a).abs() < 1e-9);
        }
        // jedes Element endet genau auf der Starthoehe des naechsten, das letzte auf der Zielhoehe
        for w in el.windows(2) {
            assert!((w[0].z + w[0].dh - w[1].z).abs() < 1e-9);
        }
        let e = el.last().unwrap();
        assert!((e.z + e.dh - 16.0).abs() < 1e-9);
        // openOMSI rechnet [spline_h] genauso: Hoehe am Ende der Kurve
        let k = e.kurve(1, 0.0);
        let p = k.point_at(k.length);
        assert!((p.z - 16.0).abs() < 1e-6, "openOMSI: {}", p.z);
    }

    #[test]
    fn lage_wie_openomsi() {
        // Biarc mit Links- und Rechtsbogen: openOMSI muss an jedem Elementende denselben Punkt berechnen
        let st = verbinden(DVec2::new(10.0, 20.0), 30.0, DVec2::new(-60.0, 140.0), 300.0);
        let el = mit_hoehe(&st, 0.0, 0.0, 5.0, 0.0);
        assert!(el.iter().any(|e| e.stueck.radius > 0.0) || el.iter().any(|e| e.stueck.radius < 0.0));
        for e in &el {
            let k = e.kurve(1, 0.0);
            let p = k.point_at(k.length).truncate();
            let (q, _) = e.stueck.ende();
            assert!((p - q).length() < 1e-6, "openOMSI {p:?}, wir {q:?} (Radius {})", e.stueck.radius);
        }
    }

    #[test]
    fn netz_anschluss_richtung() {
        let mut n = Netz::default();
        let a = n.knoten_neu(DVec3::new(0.0, 0.0, 0.0));
        let b = n.knoten_neu(DVec3::new(0.0, 100.0, 0.0));
        n.kante_neu(a, b, "x.sli", 0.0, 0.0);
        // weiter ab b: nach Norden; ab a (Gegenrichtung): nach Sueden
        assert_eq!(n.weiter_richtung(b), Some(0.0));
        assert_eq!(n.weiter_richtung(a), Some(180.0));
        let c = n.knoten_neu(DVec3::new(60.0, 160.0, 3.0));
        let e2 = n.kante_neu(b, c, "x.sli", 0.0, 90.0);
        assert_eq!(n.weiter_richtung(b), None); // jetzt Verbindungsknoten
        let el = n.elemente(n.kante(e2).unwrap());
        let (p, h) = el.last().unwrap().stueck.ende();
        assert!(nah(p, DVec2::new(60.0, 160.0)) && norm180(h - 90.0).abs() < 1e-6);
        n.kante_loeschen(e2);
        assert!(n.knoten(c).is_none() && n.knoten(b).is_some());
    }
}

#[cfg(test)]
mod bild_tests {
    //! mit echter Installation: cargo test --release -- --include-ignored strasse_im_bild
    use super::*;

    #[test]
    #[ignore]
    fn strasse_im_bild() {
        let mut v = crate::bearbeiten::tests::grundorf();
        let sli = "Splines\\Marcel\\str_2spur_10m_Grunewaldstr.sli";
        let a = DVec2::new(140.0, 120.0);
        let b = DVec2::new(220.0, 200.0);
        let za = v.terrain_height(a.x, a.y).unwrap_or(0.0);
        let zb = v.terrain_height(b.x, b.y).unwrap_or(0.0) + 4.0;
        let st = verbinden(a, 0.0, b, 90.0);
        let el = mit_hoehe(&st, za, 0.0, zb, 0.0);
        let mut n = 0;
        let mut cum = 0.0;
        for e in &el {
            if v.add_spline(sli, &e.kurve(1, cum)).is_some() {
                n += 1;
            }
            cum += e.stueck.laenge;
        }
        assert_eq!(n, el.len(), "nicht alle Elemente gezeichnet");
        let (spuren, breite) = v.spline_lanes(sli).unwrap();
        assert!(spuren.iter().filter(|s| s.0 == 0).count() >= 2, "{spuren:?}");
        assert!(breite.0 > 3.0 && breite.1 > 3.0, "{breite:?}");
        let k = crate::kamera::Kamera { ziel: DVec3::new(180.0, 160.0, za), gier: 20.0, neigung: -35.0, abstand: 140.0, fov: 50.0 };
        let px = v.render_image(1200, 700, &k.camera()).unwrap();
        let aus = std::env::temp_dir().join("omsi-editor-strasse.png");
        image::save_buffer(&aus, &px, 1200, 700, image::ColorType::Rgba8).unwrap();
        println!("Bild: {}", aus.display());
    }
}
