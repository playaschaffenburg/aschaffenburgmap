//! World Editor, "Kacheln bearbeiten": Kacheln an den Rand der Karte anfuegen oder wegnehmen.
//!
//! Eine neue Kachel bekommt eine leere Kacheldatei (wie omsigen sie schreibt), ein Gelaende, das an den Raendern genau
//! an die Nachbarkacheln anschliesst (dazwischen nach Abstand gemittelt), und die Lichtkarte der OMSI-Vorlage - alles
//! im Sitzungsordner, die Karte bleibt bis zum Speichern unveraendert. Neue Kacheln kommen ans Ende der `[map]`-Liste
//! (die Nummern der vorhandenen bleiben, Einsetzpunkte und Fahrplaene zeigen ueber sie auf Kacheln). Beim Wegnehmen
//! verschieben sich die Nummern der Kacheln dahinter; das Speichern nummeriert Einsetzpunkte und Busstops.cfg um.

use anyhow::{bail, Context, Result};
use openomsi_game::viewer::Viewer;
use std::path::Path;

pub type Liste = Vec<(i32, i32, String)>;

/// Gelaende einer Kachel: 61 x 61 Hoehen im 5-m-Raster (Index iz * 61 + ix, iz von Sueden)
const N: usize = 61;

#[derive(Clone)]
struct Schritt {
    vorher: Liste,
    nachher: Liste,
}

#[derive(Default)]
pub struct Welt {
    undo: Vec<Schritt>,
    redo: Vec<Schritt>,
    pub aenderungen: usize,
    /// Kachelliste beim Oeffnen der Karte (vor der ersten Aenderung) - fuers Speichern
    anfang: Option<Liste>,
}

impl Welt {
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    /// Kachelliste beim Oeffnen der Karte (auf sie zeigen die Kachelnummern in global.cfg und TTData)
    pub fn anfangsliste(&self, v: &Viewer) -> Liste {
        self.anfang.clone().unwrap_or_else(|| v.map_tile_refs())
    }

    /// (Liste beim Oeffnen, jetzige Liste), wenn sich etwas geaendert hat
    pub fn speicherliste(&self, v: &Viewer) -> Option<(Liste, Liste)> {
        let alt = self.anfang.clone()?;
        let neu = v.map_tile_refs();
        (alt != neu).then_some((alt, neu))
    }

    fn setzen(&mut self, v: &mut Viewer, neu: Liste) -> Result<()> {
        let vorher = v.map_tile_refs();
        if self.anfang.is_none() {
            self.anfang = Some(vorher.clone());
        }
        v.set_map_tiles(&neu)?;
        self.undo.push(Schritt { vorher, nachher: neu });
        self.redo.clear();
        self.aenderungen += 1;
        Ok(())
    }

    /// Kachel `k` anfuegen (muss an eine vorhandene grenzen); Dateien in `ordner` (Sitzungsordner der Karte)
    pub fn hinzufuegen(&mut self, v: &mut Viewer, root: &Path, ordner: &Path, k: (i32, i32)) -> Result<String> {
        let liste = v.map_tile_refs();
        if liste.iter().any(|t| (t.0, t.1) == k) {
            bail!("Kachel {} {} gibt es schon", k.0, k.1);
        }
        if !liste.iter().any(|t| (t.0 - k.0).abs() + (t.1 - k.1).abs() == 1) {
            bail!("eine neue Kachel muss an eine vorhandene grenzen");
        }
        let datei = format!("tile_{}_{}.map", k.0, k.1);
        std::fs::create_dir_all(ordner)?;
        let pfad = ordner.join(&datei);
        let stempel = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let text = format!("File created with omsi-editor ({stempel})\r\n\r\n[version]\r\n14\r\n\r\n[terrain]\r\n\r\n\r\n[variable_terrainlightmap]\r\n\r\n[variable_terrain]\r\n\r\n");
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend(text.encode_utf16().flat_map(|u| u.to_le_bytes()));
        std::fs::write(&pfad, bytes).with_context(|| format!("{} schreiben", pfad.display()))?;
        let h = gelaende(v, k);
        let mut t = 60i32.to_le_bytes().to_vec();
        for x in &h {
            t.extend(x.to_le_bytes());
        }
        std::fs::write(ordner.join(format!("{datei}.terrain")), t)?;
        let lm = root.join("template").join("NewMap").join("tile_0_0.map.LM.bmp");
        if lm.is_file() {
            let _ = std::fs::copy(&lm, ordner.join(format!("{datei}.LM.bmp")));
        }
        let mut neu = liste;
        neu.push((k.0, k.1, datei));
        self.setzen(v, neu)?;
        let (lo, hi) = h.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), x| (a.min(*x), b.max(*x)));
        log::info!("Kachel {} {} angelegt (Gelaende {lo:.1} bis {hi:.1} m)", k.0, k.1);
        Ok(format!("Kachel {} {} angelegt - Gelaende schliesst an die Nachbarn an ({lo:.1} bis {hi:.1} m)", k.0, k.1))
    }

    /// Kachel `k` wegnehmen
    pub fn loeschen(&mut self, v: &mut Viewer, k: (i32, i32)) -> Result<String> {
        let liste = v.map_tile_refs();
        if liste.len() <= 1 {
            bail!("die letzte Kachel einer Karte bleibt");
        }
        let neu: Liste = liste.iter().filter(|t| (t.0, t.1) != k).cloned().collect();
        if neu.len() == liste.len() {
            bail!("Kachel {} {} gibt es nicht", k.0, k.1);
        }
        self.setzen(v, neu)?;
        log::info!("Kachel {} {} weggenommen", k.0, k.1);
        Ok(format!("Kachel {} {} weggenommen", k.0, k.1))
    }

    pub fn rueckgaengig(&mut self, v: &mut Viewer) -> Result<bool> {
        let Some(s) = self.undo.pop() else { return Ok(false) };
        v.set_map_tiles(&s.vorher)?;
        self.redo.push(s);
        self.aenderungen += 1;
        Ok(true)
    }

    pub fn wiederholen(&mut self, v: &mut Viewer) -> Result<bool> {
        let Some(s) = self.redo.pop() else { return Ok(false) };
        v.set_map_tiles(&s.nachher)?;
        self.undo.push(s);
        self.aenderungen += 1;
        Ok(true)
    }
}

/// Inhalt einer Kachel: (Objekte, Splines)
pub fn inhalt(v: &Viewer, k: (i32, i32)) -> (usize, usize) {
    v.tile_file(k.0, k.1).and_then(|p| omsi_map::Tile::load(&p).ok()).map(|t| (t.objects.len(), t.splines.len())).unwrap_or((0, 0))
}

/// Gelaende einer Kachel (61 x 61), wie es jetzt gilt (auch mit "Gelaende formen" geaendert), falls vorhanden
fn gelaende_lesen(v: &Viewer, k: (i32, i32)) -> Option<Vec<f32>> {
    v.tile_terrain(k.0, k.1).filter(|t| t.samples() == N).map(|t| t.heights)
}

/// Gelaende fuer eine neue Kachel: an jedem Rand mit Nachbar genau dessen Randhoehen, innen nach Abstand zu den
/// Raendern gemittelt (ohne Nachbarn: 0 m)
pub fn gelaende(v: &Viewer, k: (i32, i32)) -> Vec<f32> {
    let w = gelaende_lesen(v, (k.0 - 1, k.1));
    let o = gelaende_lesen(v, (k.0 + 1, k.1));
    let s = gelaende_lesen(v, (k.0, k.1 - 1));
    let n = gelaende_lesen(v, (k.0, k.1 + 1));
    mischen(w.as_deref(), o.as_deref(), s.as_deref(), n.as_deref())
}

/// aus den Gelaenden der Nachbarn (West, Ost, Sued, Nord; je 61 x 61) das der Kachel dazwischen
fn mischen(w: Option<&[f32]>, o: Option<&[f32]>, s: Option<&[f32]>, n: Option<&[f32]>) -> Vec<f32> {
    // Raender der Nachbarn, die an diese Kachel grenzen (je 61 Werte)
    let west: Option<Vec<f32>> = w.map(|g| (0..N).map(|iz| g[iz * N + (N - 1)]).collect());
    let ost: Option<Vec<f32>> = o.map(|g| (0..N).map(|iz| g[iz * N]).collect());
    let sued: Option<Vec<f32>> = s.map(|g| (0..N).map(|ix| g[(N - 1) * N + ix]).collect());
    let nord: Option<Vec<f32>> = n.map(|g| (0..N).map(|ix| g[ix]).collect());
    let mut out = vec![0.0f32; N * N];
    for iz in 0..N {
        for ix in 0..N {
            // (Wert, Abstand zum Rand in Rasterschritten)
            let seiten: Vec<(f32, usize)> = [
                west.as_ref().map(|r| (r[iz], ix)),
                ost.as_ref().map(|r| (r[iz], N - 1 - ix)),
                sued.as_ref().map(|r| (r[ix], iz)),
                nord.as_ref().map(|r| (r[ix], N - 1 - iz)),
            ].into_iter().flatten().collect();
            if seiten.is_empty() {
                continue;
            }
            let genau: Vec<f32> = seiten.iter().filter(|x| x.1 == 0).map(|x| x.0).collect();
            out[iz * N + ix] = if !genau.is_empty() {
                genau.iter().sum::<f32>() / genau.len() as f32
            } else {
                let (s, w) = seiten.iter().fold((0.0f32, 0.0f32), |(s, w), (h, d)| {
                    let g = 1.0 / (*d as f32).powi(2);
                    (s + h * g, w + g)
                });
                s / w
            };
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    fn kopieren(a: &Path, b: &Path) {
        std::fs::create_dir_all(b).unwrap();
        for e in std::fs::read_dir(a).unwrap().flatten() {
            if e.file_type().unwrap().is_dir() { kopieren(&e.path(), &b.join(e.file_name())) } else { std::fs::copy(e.path(), b.join(e.file_name())).unwrap(); }
        }
    }

    fn utf16(b: &[u8]) -> String {
        if b.starts_with(&[0xFF, 0xFE]) {
            String::from_utf16_lossy(&b[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>())
        } else {
            b.iter().map(|&c| c as char).collect()
        }
    }

    /// Gelaende zwischen zwei Nachbarn (West 10 m, Ost 0 m): Raender genau, dazwischen stetig fallend
    #[test]
    fn gelaende_zwischen_nachbarn() {
        let mut w = vec![0.0f32; N * N];
        for x in w.iter_mut() {
            *x = 10.0;
        }
        let ost = vec![0.0f32; N * N];
        let h = mischen(Some(&w), Some(&ost), None, None);
        for iz in 0..N {
            assert!((h[iz * N] - 10.0).abs() < 1e-6 && h[iz * N + N - 1].abs() < 1e-6);
            for ix in 1..N {
                assert!(h[iz * N + ix] <= h[iz * N + ix - 1] + 1e-6);
            }
        }
    }

    /// Grundorf: Kachel am Rand anlegen (Gelaende schliesst an), eine wegnehmen, rueckgaengig/wiederholen, als neue Karte
    /// speichern - [map]-Liste, Einsetzpunkte und Busstops.cfg stimmen
    #[test]
    #[ignore]
    fn kacheln_anlegen_loeschen_speichern() {
        let _sperre = crate::bearbeiten::tests::sperre();
        let root = Path::new(crate::bearbeiten::tests::OMSI);
        let mut v = crate::bearbeiten::tests::grundorf();
        let a = crate::aendern::Aendern::neu(&v);
        let ordner = a.sitzung.join("maps").join("Grundorf");
        let mut w = Welt::default();
        let alt = v.map_tile_refs();
        // freier Platz oestlich der Kachel 2 0 (grenzt an sie)
        let k = (3, 0);
        assert!(!alt.iter().any(|t| (t.0, t.1) == k) && alt.iter().any(|t| (t.0, t.1) == (2, 0)));
        println!("{}", w.hinzufuegen(&mut v, root, &ordner, k).unwrap());
        assert!(v.map_tiles().contains(&k));
        // Gelaende: Westrand gleich dem Ostrand der Nachbarkachel
        let neu_h = gelaende_lesen(&v, k).expect("Gelaende der neuen Kachel");
        let nb = gelaende_lesen(&v, (2, 0)).unwrap();
        for iz in 0..N {
            assert!((neu_h[iz * N] - nb[iz * N + N - 1]).abs() < 1e-4, "Rand bei iz {iz}");
        }
        // freie Plaetze, deren Nachbarrand Hoehe hat: dort muss der Rand genau anschliessen
        let mut huegelig = 0;
        for t in v.map_tile_refs() {
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let p = (t.0 + dx, t.1 + dy);
                if v.map_tiles().contains(&p) {
                    continue;
                }
                let g = gelaende_lesen(&v, (t.0, t.1)).unwrap();
                // Rand von t, der an p grenzt
                let rand: Vec<f32> = (0..N).map(|i| match (dx, dy) {
                    (1, 0) => g[i * N + N - 1], (-1, 0) => g[i * N], (0, 1) => g[(N - 1) * N + i], _ => g[i],
                }).collect();
                if rand.iter().all(|x| x.abs() < 0.1) {
                    continue;
                }
                let h = gelaende(&v, p);
                let eigen: Vec<f32> = (0..N).map(|i| match (dx, dy) {
                    (1, 0) => h[i * N], (-1, 0) => h[i * N + N - 1], (0, 1) => h[i], _ => h[(N - 1) * N + i],
                }).collect();
                for i in 0..N {
                    assert!((eigen[i] - rand[i]).abs() < 1e-4, "Platz {p:?}: Rand bei {i}");
                }
                huegelig += 1;
            }
        }
        println!("{huegelig} freie Plaetze an huegeligen Raendern geprueft");
        // laden laesst sie sich
        v.tiles_around(DVec3::new(3.5 * 300.0, 150.0, 0.0), 0).unwrap();
        assert!(v.terrain_height(3.5 * 300.0, 150.0).is_some(), "neue Kachel nicht geladen");
        // Kachel 0 1 (Nummer 1) wegnehmen, zurueck, wieder
        println!("{}", w.loeschen(&mut v, (0, 1)).unwrap());
        assert!(!v.map_tiles().contains(&(0, 1)));
        assert!(w.rueckgaengig(&mut v).unwrap());
        assert!(v.map_tiles().contains(&(0, 1)));
        assert!(w.wiederholen(&mut v).unwrap());
        let neu = v.map_tile_refs();
        assert_eq!(neu.len(), alt.len());
        // speichern als neue Karte
        let test_root = std::env::temp_dir().join(format!("omsi-editor-welt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&test_root);
        kopieren(&root.join("maps/Grundorf"), &test_root.join("maps/Grundorf"));
        let mut paket = crate::speichern::vorbereiten(&v, &crate::bearbeiten::Bearbeiten::neu(crate::bearbeiten::Werkzeug::Welt),
                                                      &crate::netz::Netz::default(), &[], &a.kopien("Grundorf"), None, "Grundorf").unwrap();
        paket.kacheln = w.speicherliste(&v);
        assert!(paket.kacheln.is_some());
        let ziel = crate::speichern::karte_anlegen(&test_root, "Grundorf", "Grundorf_kacheln", &paket).unwrap();
        assert!(ziel.join("tile_3_0.map").is_file() && ziel.join("tile_3_0.map.terrain").is_file());
        let g = omsi_map::GlobalCfg::load(&ziel.join("global.cfg")).unwrap();
        let liste: Vec<(i32, i32)> = g.tiles.iter().map(|t| (t.x, t.y)).collect();
        assert_eq!(liste, neu.iter().map(|t| (t.0, t.1)).collect::<Vec<_>>());
        // Einsetzpunkte: jede Nummer zeigt auf dieselbe Kachel wie vorher
        let g_alt = omsi_map::GlobalCfg::load(&root.join("maps/Grundorf/global.cfg")).unwrap();
        let text_alt = utf16(&std::fs::read(root.join("maps/Grundorf/global.cfg")).unwrap());
        let text_neu = utf16(&std::fs::read(ziel.join("global.cfg")).unwrap());
        let eps = |t: &str| -> Vec<(String, usize)> {
            let z: Vec<&str> = t.lines().collect();
            let i = z.iter().position(|l| l.trim().eq_ignore_ascii_case("[entrypoints]")).unwrap();
            let n: usize = z[i + 1].trim().parse().unwrap();
            (0..n).map(|k| (z[i + 2 + k * 12 + 11].trim().to_string(), z[i + 2 + k * 12 + 10].trim().parse().unwrap())).collect()
        };
        let (ea, en) = (eps(&text_alt), eps(&text_neu));
        println!("Einsetzpunkte vorher {} nachher {}", ea.len(), en.len());
        for (name, nr) in &en {
            let (_, nr_alt) = ea.iter().find(|x| &x.0 == name).unwrap();
            let k_alt = g_alt.tiles.iter().find(|t| t.index == *nr_alt).map(|t| (t.x, t.y));
            assert_eq!(k_alt, Some(liste[*nr]), "Einsetzpunkt {name}");
        }
        assert!(en.len() <= ea.len());
        // Haltestellen ebenso
        let bs = |p: &Path| -> Vec<(i64, usize)> {
            let t = utf16(&std::fs::read(p).unwrap());
            let z: Vec<&str> = t.lines().collect();
            (0..z.len()).filter(|&i| z[i].trim() == "[busstop]").map(|i| (z[i + 3].trim().parse().unwrap(), z[i + 2].trim().parse().unwrap())).collect()
        };
        let (ba, bn) = (bs(&root.join("maps/Grundorf/TTData/Busstops.cfg")), bs(&ziel.join("TTData/Busstops.cfg")));
        println!("Haltestellen vorher {} nachher {}", ba.len(), bn.len());
        for (id, nr) in &bn {
            let nr_alt = ba.iter().find(|x| x.0 == *id).unwrap().1;
            assert_eq!(g_alt.tiles.iter().find(|t| t.index == nr_alt).map(|t| (t.x, t.y)), Some(liste[*nr]), "Haltestelle {id}");
        }
        std::fs::remove_dir_all(&test_root).ok();
        drop(a);
    }
}
