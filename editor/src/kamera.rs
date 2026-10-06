//! Kamera wie in Transport Fever: ein Blickpunkt am Boden, Drehung, Neigung, Abstand.
//! openOMSI-Welt: x Ost, y Nord, z hoch; Richtung (yaw) im Uhrzeigersinn ab Nord.

use glam::{DVec3, Vec3};
use openomsi_game::viewer::Camera;

#[derive(Clone, Debug)]
pub struct Kamera {
    /// Blickpunkt (am Boden)
    pub ziel: DVec3,
    /// Grad, 0 = nach Norden, im Uhrzeigersinn
    pub gier: f32,
    /// Grad, negativ = nach unten
    pub neigung: f32,
    pub abstand: f64,
    pub fov: f32,
}

impl Default for Kamera {
    fn default() -> Self {
        Kamera { ziel: DVec3::new(150.0, 150.0, 0.0), gier: 0.0, neigung: -40.0, abstand: 250.0, fov: 50.0 }
    }
}

impl Kamera {
    pub fn vorwaerts(&self) -> Vec3 {
        let (sy, cy) = self.gier.to_radians().sin_cos();
        let (sp, cp) = self.neigung.to_radians().sin_cos();
        Vec3::new(sy * cp, cy * cp, sp)
    }

    /// Kamera fuer den openOMSI-Renderer
    pub fn camera(&self) -> Camera {
        let pos = self.ziel - self.vorwaerts().as_dvec3() * self.abstand;
        Camera {
            position: pos,
            yaw: self.gier,
            pitch: self.neigung,
            roll: 0.0,
            fov_deg: self.fov,
            near: (self.abstand as f32 * 0.002).clamp(0.2, 5.0),
            far: 9000.0,
        }
    }

    /// Verschieben in Blickrichtung (vor) und quer dazu (rechts), waagerecht
    pub fn verschieben(&mut self, rechts: f64, vor: f64) {
        let (s, c) = (self.gier as f64).to_radians().sin_cos();
        self.ziel.x += c * rechts + s * vor;
        self.ziel.y += -s * rechts + c * vor;
    }

    pub fn drehen(&mut self, d_gier: f32, d_neigung: f32) {
        self.gier = (self.gier + d_gier).rem_euclid(360.0);
        self.neigung = (self.neigung + d_neigung).clamp(-89.0, -3.0);
    }

    /// Meter je Bildschirmpunkt am Blickpunkt
    pub fn m_pro_px(&self, hoehe_px: f32) -> f64 {
        self.abstand * 2.0 * (self.fov.to_radians() as f64 / 2.0).tan() / hoehe_px.max(1.0) as f64
    }

    /// Strahl durch einen Bildschirmpunkt -> (Ursprung, Richtung) in Weltkoordinaten
    pub fn strahl(&self, px: f32, py: f32, breite: f32, hoehe: f32) -> (DVec3, DVec3) {
        let cam = self.camera();
        let (o, d) = cam.ray(2.0 * px / breite - 1.0, 1.0 - 2.0 * py / hoehe, breite / hoehe.max(1.0), cam.position);
        (cam.position + o.as_dvec3(), d.as_dvec3())
    }
}

/// Schnitt eines Strahls mit dem Boden (hoehe(x, y) -> z) -> Punkt
pub fn treffer(o: DVec3, d: DVec3, hoehe: impl Fn(f64, f64) -> Option<f64>, weit: f64) -> Option<DVec3> {
    let mut t = 0.0;
    let mut vorher: Option<f64> = None;
    while t < weit {
        let p = o + d * t;
        let h = hoehe(p.x, p.y).unwrap_or(0.0);
        if p.z <= h {
            let (mut lo, mut hi) = match vorher {
                Some(v) => (v, t),
                None => return Some(DVec3::new(p.x, p.y, h)),
            };
            for _ in 0..24 {
                let m = 0.5 * (lo + hi);
                let q = o + d * m;
                if q.z <= hoehe(q.x, q.y).unwrap_or(0.0) {
                    hi = m;
                } else {
                    lo = m;
                }
            }
            let q = o + d * hi;
            return Some(DVec3::new(q.x, q.y, hoehe(q.x, q.y).unwrap_or(0.0)));
        }
        vorher = Some(t);
        t += (t * 0.004).max(0.5);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mitte_trifft_blickpunkt() {
        let k = Kamera { ziel: DVec3::new(100.0, 200.0, 10.0), gier: 30.0, neigung: -45.0, abstand: 150.0, fov: 50.0 };
        let (o, d) = k.strahl(400.0, 300.0, 800.0, 600.0);
        let p = treffer(o, d, |_, _| Some(10.0), 5000.0).unwrap();
        assert!((p.x - 100.0).abs() < 0.3 && (p.y - 200.0).abs() < 0.3, "{p:?}");
    }

    #[test]
    fn verschieben_in_blickrichtung() {
        let mut k = Kamera { gier: 90.0, ..Default::default() };
        let x0 = k.ziel.x;
        k.verschieben(0.0, 10.0);
        assert!((k.ziel.x - x0 - 10.0).abs() < 1e-9);
    }
}
