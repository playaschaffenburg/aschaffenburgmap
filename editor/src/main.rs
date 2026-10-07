//! OMSI-Editor: moderner Karteneditor fuer OMSI 2 auf Basis von openOMSI (Darstellung wie im Spiel).
//!
//! Meilenstein 1 "Betrachter": beliebige OMSI-Karte oeffnen, Kamera wie in Transport Fever,
//! Kacheln werden im Hintergrund um den Blickpunkt gestreamt.
//! Meilenstein 2 "Bearbeiten": Objekte auswaehlen, ziehen, drehen, loeschen, kopieren, Rueckgaengig,
//! Speichern als neue Karte (bearbeiten.rs, speichern.rs).
//!
//!   omsi-editor [--root <OMSI-2-Ordner>] [Kartenordner]
//!
//! Bedienung: rechte Maustaste drehen/neigen, mittlere verschieben, Mausrad zoomen,
//! W A S D / Pfeile verschieben, Q / E drehen, R / F neigen.

mod aendern;
mod anschluss;
mod bearbeiten;
mod kamera;
mod karten;
mod katalog;
mod kreuzung;
mod netz;
mod protokoll;
mod speichern;
mod strasse;
mod vorschau;

use anyhow::{Context, Result};
use glam::DVec3;
use bearbeiten::{Bearbeiten, Wahl, Werkzeug};
use kamera::{treffer, Kamera};
use openomsi_game::viewer::Action;
use openomsi_game::viewer::{self, SurfaceState, Viewer};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

const STANDARD_OMSI: &str = r"C:\Program Files (x86)\Steam\steamapps\common\OMSI 2";
const KACHEL_RADIUS: i32 = 3;          // nur fuer --bild (alles auf einmal laden)
const STREAM_BUDGET_MS: u64 = 6;       // je Bild fuer das Hochladen gestreamter Kacheln

/// Grafik nur fuer die Oberflaeche, solange keine Karte offen ist (Startbildschirm mit der Kartenauswahl)
struct Startbild {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
}

impl Startbild {
    fn neu(window: Arc<Window>) -> Result<Startbild> {
        let instance = viewer::instance();
        let surface = instance.create_surface(window.clone()).context("Flaeche anlegen")?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface), ..Default::default()
        })).context("keine Grafikkarte fuer die Oberflaeche")?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).context("Grafikgeraet")?;
        let caps = surface.get_capabilities(&adapter);
        // egui zeichnet in eine Flaeche ohne sRGB (wie im Kartenfenster)
        let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT, format, width: size.width.max(1), height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo, desired_maximum_frame_latency: 2, alpha_mode: caps.alpha_modes[0], view_formats: vec![],
        };
        surface.configure(&device, &config);
        Ok(Startbild { surface, device, queue, config })
    }

    fn groesse(&mut self, w: u32, h: u32) {
        self.config.width = w.max(1);
        self.config.height = h.max(1);
        self.surface.configure(&self.device, &self.config);
    }
}

/// egui-Ausgabe in eine Flaeche zeichnen (`leeren`: vorher mit dem Hintergrund fuellen)
fn egui_malen(gui: &mut Gui, device: &wgpu::Device, queue: &wgpu::Queue, view: &wgpu::TextureView, w: u32, h: u32, out: egui::FullOutput, leeren: bool) {
    let jobs = gui.ctx.tessellate(out.shapes, out.pixels_per_point);
    let sd = egui_wgpu::ScreenDescriptor { size_in_pixels: [w, h], pixels_per_point: out.pixels_per_point };
    for (id, delta) in &out.textures_delta.set {
        gui.renderer.update_texture(device, queue, *id, delta);
    }
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("egui") });
    let extra = gui.renderer.update_buffers(device, queue, &mut enc, &jobs, &sd);
    {
        let load = if leeren { wgpu::LoadOp::Clear(wgpu::Color { r: 0.07, g: 0.07, b: 0.08, a: 1.0 }) } else { wgpu::LoadOp::Load };
        let pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("egui"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        gui.renderer.render(&mut pass.forget_lifetime(), &jobs, &sd);
    }
    queue.submit(extra.into_iter().chain(std::iter::once(enc.finish())));
    for id in &out.textures_delta.free {
        gui.renderer.free_texture(id);
    }
}

/// Dialoge der Kartenverwaltung
enum KartenDialog {
    /// Umbenennen: Karte, neuer Anzeigename, neuer Ordnername
    Umbenennen(String, String, String),
    /// Loeschen, erste Rueckfrage
    Loeschen1(String),
    /// Loeschen, zweite Rueckfrage: Name zur Bestaetigung eintippen
    Loeschen2(String, String),
}

struct Gui {
    ctx: egui::Context,
    state: egui_winit::State,
    renderer: egui_wgpu::Renderer,
}

struct App {
    root: PathBuf,
    karten: Vec<karten::Karte>,
    /// Startbildschirm (ohne Karte) und die Kartenauswahl
    start: Option<Startbild>,
    kartenwahl_offen: bool,
    kartenwahl: Option<String>,
    karten_dialog: Option<KartenDialog>,
    vorschaubilder: std::collections::HashMap<PathBuf, Option<egui::TextureHandle>>,
    start_karte: Option<String>,
    window: Option<Arc<Window>>,
    surface: Option<SurfaceState<'static>>,
    viewer: Option<Viewer>,
    karte: Option<String>,
    gui: Option<Gui>,
    kam: Kamera,
    tasten: HashSet<KeyCode>,
    maus: Option<(f32, f32)>,
    ziehen: Option<(MouseButton, (f32, f32))>,
    boden_unter_maus: Option<DVec3>,
    /// Format, in das egui zeichnet (ohne sRGB, sonst werden die Farben blass)
    ui_format: wgpu::TextureFormat,
    zuletzt: Instant,
    fps: f32,
    meldung: String,
    zu_laden: Option<usize>,
    filter: String,
    /// --absturztest / --haengertest: Bericht ausprobieren, sobald die Karte laeuft
    pruefung: Option<String>,
    /// Werkzeug "Kreuzungen": gewaehlte Kreuzung (Knoten des Netzes), unter der Maus
    kreuzung_wahl: Option<u32>,
    kreuzung_maus: Option<KreuzungsZiel>,
    /// eigene Strassen im Werkzeug "Aendern": gewaehlt, unter der Maus
    eigene_auswahl: Vec<u32>,
    eigene_maus: Option<u32>,
    /// gemeinsamer Verlauf fuer Rueckgaengig/Wiederholen ueber alle Werkzeuge, und die Laengen der Rueckgaengig-
    /// Stapel der Werkzeuge (Objekte, Strassen, Aendern) beim letzten Blick
    verlauf: Vec<Quelle>,
    verlauf_redo: Vec<Quelle>,
    stapel: (usize, usize, usize),
    /// Bericht der vorigen Sitzung, einmal in der Statuszeile melden
    bericht_melden: Option<Option<PathBuf>>,
    /// Testlauf: Strasse, an der spaeter eine Kreuzung gebaut wird
    testlauf_abzweig: Option<i64>,
    /// --testlauf: nach so vielen Sekunden beenden und Bildrate ausgeben
    beenden_nach: Option<f32>,
    gestartet: Instant,
    bilder: u32,
    /// laengstes Bild (s) nach dem ersten Ladebereich, fuer den Testlauf
    laengstes: f32,
    /// --wechsel: im Testlauf nach 60 % auf diese Karte umschalten
    wechsel: Option<String>,
    bearb: Bearbeiten,
    strg: bool,
    umschalt: bool,
    /// Speichern-Dialog offen: Name der neuen Karte
    speichern_name: Option<String>,
    /// Speichern im Hintergrund -> (Ergebnis-Ordner, true wenn die Karte selbst ueberschrieben wurde)
    speichern_job: Option<std::thread::JoinHandle<Result<(PathBuf, bool)>>>,
    /// Rueckfrage: fremde Karte ueberschreiben?
    ueberschreiben_frage: bool,
    /// Kamera beim Neuladen nach dem Speichern behalten
    kamera_merken: Option<Kamera>,
    /// Meldung nach dem Speichern (ueberlebt das Neuladen der Karte)
    speichern_meldung: Option<String>,
    /// Kartenwechsel mit ungespeicherten Aenderungen: erst fragen
    verwerfen_frage: Option<usize>,
    /// Objektkatalog (wird im Hintergrund eingelesen)
    katalog: Option<katalog::Katalog>,
    katalog_job: Option<std::thread::JoinHandle<katalog::Katalog>>,
    katalog_suche: String,
    katalog_ordner: Option<String>,
    katalog_gruppe: Option<String>,
    katalog_herkunft: Option<String>,
    vorschau: vorschau::Vorschau,
    /// gewaehltes Objekt zum Platzieren (relativer .sco-Pfad)
    platzier: Option<String>,
    /// Strassenbau (Meilenstein 3)
    strasse: strasse::Strassenbau,
    querschnitte: Option<Vec<strasse::Querschnitt>>,
    qs_job: Option<std::thread::JoinHandle<Vec<strasse::Querschnitt>>>,
    qs_suche: String,
    qs_herkunft: Option<String>,
    qs_ordner: Option<String>,
    qs_spuren: Option<&'static str>,
    qs_gehweg: Option<bool>,
    /// Rechtsklick ohne Ziehen erkennen (Zug beenden)
    rechts_start: Option<(f32, f32)>,
    /// Enden vorhandener Strassen (frei/angeschlossen) in den geladenen Kacheln
    anschluesse: anschluss::Anschluesse,
    /// Aendern vorhandener Strassen (Sitzungsordner je Karte)
    aendern: Option<aendern::Aendern>,
    aendern_warnung: Option<String>,
}

/// was im Werkzeug "Kreuzungen" unter der Maus liegt
#[derive(Clone, Debug)]
enum KreuzungsZiel {
    /// Kreuzung des eigenen Netzes
    Netz(u32),
    /// vorhandenes Kreuzungsobjekt der Karte (Klick uebernimmt es)
    Vorhanden(kreuzung::Vorhanden),
}

/// welches Werkzeug einen Schritt im gemeinsamen Verlauf gemacht hat
#[derive(Clone, Copy, Debug, PartialEq)]
enum Quelle {
    Objekte,
    Strasse,
    Aendern,
}

/// Was die Oberflaeche ausloesen will (nach dem Zeichnen ausgefuehrt)
enum UiAktion {
    Karte(usize),
    Werkzeug(Werkzeug),
    Objekt(Action),
    Rueckgaengig,
    Wiederholen,
    Abwaehlen,
    SpeichernDialog,
    /// Karte umbenennen: (Ordner, neuer Anzeigename, neuer Ordner)
    KarteUmbenennen(String, String, String),
    /// Karte in den Papierkorb
    KarteLoeschen(String),
    /// "Speichern": die geoeffnete Karte ueberschreiben (eigene sofort, fremde nach Rueckfrage)
    SpeichernHier,
    Ueberschreiben,
    Speichern(String),
    Verwerfen(usize),
    FrageZu,
    Platzier(Option<String>),
    Querschnitt(String),
    SplineQuerschnitt(String),
    SplineSpiegeln,
    SplineLoeschen,
    StrassenModus(strasse::Modus),
    EigeneAendern(strasse::KantenAenderung),
    /// Vorfahrt/Ampel einer Kreuzung setzen (None: wieder vermuten)
    KreuzungRegel(u32, Option<netz::Regel>),
    KreiselQuerschnitt(String),
    StrassenHoehe(f64),
    ZugBeenden,
}

impl App {
    fn new(root: PathBuf, start_karte: Option<String>) -> Self {
        let karten = karten::finden(&root);
        App {
            root,
            karten,
            start_karte,
            window: None,
            surface: None,
            viewer: None,
            karte: None,
            gui: None,
            kam: Kamera::default(),
            tasten: HashSet::new(),
            maus: None,
            ziehen: None,
            boden_unter_maus: None,
            ui_format: wgpu::TextureFormat::Bgra8Unorm,
            zuletzt: Instant::now(),
            fps: 0.0,
            meldung: String::new(),
            zu_laden: None,
            filter: String::new(),
            beenden_nach: None,
            testlauf_abzweig: None,
            bericht_melden: Some(protokoll::neuer_bericht()),
            eigene_auswahl: vec![],
            eigene_maus: None,
            kreuzung_wahl: None,
            kreuzung_maus: None,
            verlauf: vec![],
            verlauf_redo: vec![],
            stapel: (0, 0, 0),
            pruefung: None,
            gestartet: Instant::now(),
            bilder: 0,
            laengstes: 0.0,
            wechsel: None,
            bearb: Bearbeiten::default(),
            strg: false,
            umschalt: false,
            speichern_name: None,
            speichern_job: None,
            start: None,
            kartenwahl_offen: false,
            kartenwahl: None,
            karten_dialog: None,
            vorschaubilder: Default::default(),
            ueberschreiben_frage: false,
            kamera_merken: None,
            speichern_meldung: None,
            verwerfen_frage: None,
            katalog: None,
            katalog_job: None,
            katalog_suche: String::new(),
            katalog_ordner: None,
            katalog_gruppe: None,
            katalog_herkunft: None,
            vorschau: vorschau::Vorschau::default(),
            platzier: None,
            strasse: strasse::Strassenbau::default(),
            querschnitte: None,
            qs_job: None,
            qs_suche: String::new(),
            qs_herkunft: None,
            qs_ordner: None,
            qs_spuren: None,
            qs_gehweg: None,
            rechts_start: None,
            anschluesse: anschluss::Anschluesse::default(),
            aendern: None,
            aendern_warnung: None,
        }
    }

    /// Karte oeffnen: neuer Renderer und neue Szene (openOMSI-Welt)
    fn karte_oeffnen(&mut self, i: usize) -> Result<()> {
        let window = self.window.clone().context("kein Fenster")?;
        let t0 = Instant::now();
        // schon eine Karte offen: Renderer behalten (Pipelines nur einmal kompilieren), Welt tauschen
        self.bearb = Bearbeiten::neu(self.bearb.werkzeug);
        self.strasse = strasse::Strassenbau::neu(self.strasse.sli.clone(), self.strasse.modus);
        self.anschluesse = anschluss::Anschluesse::default();
        self.sitzung_schliessen();
        if let Some(v) = self.viewer.as_mut() {
            let cam = v.open_map(&self.karten[i].global)?;
            self.aendern = Some(aendern::Aendern::neu(v));
            self.kamera_von(&cam);
            let (ziel, weite) = (self.kam.ziel, self.sichtweite());
            if let Some(v) = self.viewer.as_mut() {
                v.stream(ziel, weite, std::time::Duration::from_millis(STREAM_BUDGET_MS));
            }
            self.karte = Some(self.karten[i].ordner.clone());
            self.meldung = format!("{} geoeffnet in {:.1} s", self.karten[i].ordner, t0.elapsed().as_secs_f32());
            window.set_title(&format!("{} - OMSI-Editor", self.karten[i].ordner));
            return Ok(());
        }
        // erste Karte: Grafik starten (Pipelines kompilieren), Flaeche anlegen; die Grafik des Startbildschirms geht
        self.surface = None;
        self.viewer = None;
        self.gui = None;
        self.start = None;
        let instance = viewer::instance();
        let tmp = instance.create_surface(window.clone()).context("Flaeche anlegen")?;
        let (mut v, cam) = Viewer::open(&instance, Some(&tmp), &self.root, &self.karten[i].global)?;
        drop(tmp);
        let size = window.inner_size();
        let mut surface = SurfaceState::new(&instance, window.clone(), &v.renderer, size.width.max(1), size.height.max(1))?;
        // egui ueber eine Sicht ohne sRGB auf dieselbe Flaeche zeichnen lassen
        self.ui_format = surface.config.format.remove_srgb_suffix();
        if self.ui_format != surface.config.format {
            surface.config.view_formats = vec![self.ui_format];
            surface.surface.configure(&v.renderer.device, &surface.config);
        }
        self.kamera_von(&cam);
        // die Kacheln kommen ab dem ersten Bild im Hintergrund (Viewer::stream)
        v.stream(self.kam.ziel, self.sichtweite(), std::time::Duration::from_millis(STREAM_BUDGET_MS));
        self.gui = Some(Self::gui_neu(&window, &v.renderer.device, self.ui_format));
        self.aendern = Some(aendern::Aendern::neu(&v));
        self.surface = Some(surface);
        self.viewer = Some(v);
        self.karte = Some(self.karten[i].ordner.clone());
        self.meldung = format!("{} geladen in {:.1} s", self.karten[i].ordner, t0.elapsed().as_secs_f32());
        window.set_title(&format!("{} - OMSI-Editor", self.karten[i].ordner));
        Ok(())
    }

    /// Blickpunkt dorthin, wohin die Startkamera der Karte ([mapcam]) schaut
    fn kamera_von(&mut self, cam: &openomsi_game::viewer::Camera) {
        let f = cam.forward().as_dvec3();
        let mut ziel = cam.position + f * if f.z < -0.05 { (cam.position.z / -f.z).min(400.0) } else { 150.0 };
        ziel.z = cam.position.z.min(ziel.z.max(0.0));
        self.kam = Kamera { ziel, gier: cam.yaw, neigung: cam.pitch.min(-15.0), abstand: 250.0, fov: 50.0 };
    }

    fn gui_neu(window: &Window, device: &wgpu::Device, format: wgpu::TextureFormat) -> Gui {
        let ctx = egui::Context::default();
        ctx.set_visuals(egui::Visuals::dark());
        let state = egui_winit::State::new(ctx.clone(), egui::ViewportId::ROOT, window, Some(window.scale_factor() as f32), None, None);
        let renderer = egui_wgpu::Renderer::new(device, format, egui_wgpu::RendererOptions::default());
        Gui { ctx, state, renderer }
    }

    fn boden(&self, x: f64, y: f64) -> Option<f64> {
        self.viewer.as_ref().and_then(|v| v.ground_height(x, y))
    }

    /// Kamera aus Tasten, Kacheln nachladen
    fn bewegen(&mut self, dt: f32) {
        let s = self.kam.abstand * 0.9 * dt as f64;
        let strg = self.strg;
        let t = |k: KeyCode| !strg && self.tasten.contains(&k);
        let (mut r, mut v) = (0.0, 0.0);
        if t(KeyCode::KeyW) || t(KeyCode::ArrowUp) { v += s; }
        if t(KeyCode::KeyS) || t(KeyCode::ArrowDown) { v -= s; }
        if t(KeyCode::KeyD) || t(KeyCode::ArrowRight) { r += s; }
        if t(KeyCode::KeyA) || t(KeyCode::ArrowLeft) { r -= s; }
        let mut dg = 0.0;
        let mut dn = 0.0;
        if t(KeyCode::KeyQ) { dg -= 70.0 * dt; }
        if t(KeyCode::KeyE) { dg += 70.0 * dt; }
        if t(KeyCode::KeyR) { dn += 40.0 * dt; }
        if t(KeyCode::KeyF) { dn -= 40.0 * dt; }
        if r != 0.0 || v != 0.0 {
            self.kam.verschieben(r, v);
        }
        if dg != 0.0 || dn != 0.0 {
            self.kam.drehen(dg, dn);
        }
        if let Some(z) = self.boden(self.kam.ziel.x, self.kam.ziel.y) {
            self.kam.ziel.z += (z - self.kam.ziel.z) * (1.0 - (-dt as f64 * 8.0).exp());
        }
        // Kacheln im Hintergrund: Lesen und Zerlegen im Worker, hier nur ein paar ms Hochladen
        let (ziel, weite) = (self.kam.ziel, self.sichtweite());
        if let Some(v) = self.viewer.as_mut() {
            v.stream(ziel, weite, std::time::Duration::from_millis(STREAM_BUDGET_MS));
            if self.bearb.werkzeug == Werkzeug::Strasse {
                self.anschluesse.aktualisieren(v);
            }
            // die Splines der Karte braucht auch das Strassenwerkzeug (Abzweige mitten aus vorhandenen Strassen)
            if matches!(self.bearb.werkzeug, Werkzeug::Aendern | Werkzeug::Strasse | Werkzeug::Kreuzung) {
                if let Some(a) = self.aendern.as_mut() {
                    a.aktualisieren(v);
                }
            }
        }
    }

    /// Startbildschirm: Kartenauswahl ohne 3D (eigene kleine Grafik nur fuer die Oberflaeche)
    fn startbild_zeichnen(&mut self, window: &Arc<Window>) {
        if self.start.is_none() {
            match Startbild::neu(window.clone()) {
                Ok(st) => {
                    self.gui = Some(Self::gui_neu(window, &st.device, st.config.format));
                    self.start = Some(st);
                }
                Err(e) => {
                    log::error!("Startbildschirm: {e:#}");
                    // ohne Oberflaeche: wie frueher die erste Karte oeffnen
                    self.zu_laden = (!self.karten.is_empty()).then_some(0);
                    return;
                }
            }
        }
        let st = self.start.as_mut().unwrap();
        let frame = match st.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            _ => {
                let size = window.inner_size();
                st.groesse(size.width, size.height);
                return;
            }
        };
        let (w, h) = (st.config.width, st.config.height);
        let view = frame.texture.create_view(&Default::default());
        let Some(mut gui) = self.gui.take() else { return };
        let input = gui.state.take_egui_input(window);
        let mut aktionen: Vec<UiAktion> = Vec::new();
        let out = gui.ctx.run_ui(input, |ctx| {
            egui::Panel::bottom("status_start").show(ctx, |ui| {
                ui.label(&self.meldung);
            });
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.heading("OMSI-Editor");
                ui.label(egui::RichText::new(format!("{} Karten in {}", self.karten.len(), self.root.join("maps").display())).weak());
                ui.separator();
                self.kartenwahl_ui(ui, &mut aktionen);
            });
            self.karten_dialoge(ctx, &mut aktionen);
        });
        gui.state.handle_platform_output(window, out.platform_output.clone());
        let st = self.start.as_ref().unwrap();
        egui_malen(&mut gui, &st.device, &st.queue, &view, w, h, out, true);
        frame.present();
        self.gui = Some(gui);
        for a in aktionen {
            self.ausfuehren(a);
        }
        // Testlauf ohne Startkarte: nach 2 s die Karte aus --wechsel ueber die Auswahl oeffnen
        if self.beenden_nach.is_some() && self.gestartet.elapsed().as_secs_f32() > 2.0 {
            if let Some(z) = self.wechsel.take() {
                if let Some(i) = self.karten.iter().position(|k| k.ordner == z) {
                    println!("Testlauf Startbildschirm: oeffne {z}");
                    self.ausfuehren(UiAktion::Karte(i));
                }
            }
        }
        window.request_redraw();
    }

    /// Vorschaubild einer Karte (picture.jpg) als Textur, einmal geladen
    fn vorschaubild(&mut self, ctx: &egui::Context, k: &karten::Karte) -> Option<egui::TextureHandle> {
        let p = k.bild.clone()?;
        if let Some(t) = self.vorschaubilder.get(&p) {
            return t.clone();
        }
        let t = image::open(&p).ok().map(|i| {
            let i = i.thumbnail(640, 400).to_rgba8();
            let (w, h) = (i.width() as usize, i.height() as usize);
            ctx.load_texture(p.to_string_lossy(), egui::ColorImage::from_rgba_unmultiplied([w, h], i.as_raw()), Default::default())
        });
        self.vorschaubilder.insert(p, t.clone());
        t
    }

    /// Kartenauswahl: Liste mit Suche, rechts Vorschaubild, Beschreibung, Oeffnen/Umbenennen/Loeschen
    fn kartenwahl_ui(&mut self, ui: &mut egui::Ui, aktionen: &mut Vec<UiAktion>) {
        let ctx = &ui.ctx().clone();
        let f = self.filter.to_lowercase();
        let gewaehlt = self.kartenwahl.clone().or_else(|| self.karte.clone());
        let mut liste_breite = 320.0f32.min(ui.available_width() * 0.45);
        if liste_breite < 200.0 {
            liste_breite = ui.available_width();
        }
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(liste_breite);
                ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("suchen (Ordner oder Name)"));
                egui::ScrollArea::vertical().id_salt("kartenliste").max_height(ui.available_height() - 10.0).show(ui, |ui| {
                    for (i, k) in self.karten.iter().enumerate() {
                        if !f.is_empty() && !k.ordner.to_lowercase().contains(&f) && !k.anzeige.to_lowercase().contains(&f) {
                            continue;
                        }
                        let offen = self.karte.as_deref() == Some(k.ordner.as_str());
                        let text = format!("{}{}{}", k.ordner, if k.eigene { "  (eigene)" } else if k.standard { "  (Standard)" } else { "" }, if offen { "  - offen" } else { "" });
                        let r = ui.selectable_label(gewaehlt.as_deref() == Some(k.ordner.as_str()), text);
                        if r.clicked() {
                            self.kartenwahl = Some(k.ordner.clone());
                        }
                        if r.double_clicked() && !offen {
                            aktionen.push(UiAktion::Karte(i));
                        }
                    }
                });
            });
            ui.separator();
            ui.vertical(|ui| {
                let Some(i) = gewaehlt.as_ref().and_then(|g| self.karten.iter().position(|k| &k.ordner == g)) else {
                    ui.label("Karte links waehlen (Doppelklick oeffnet sie).");
                    return;
                };
                let k = self.karten[i].clone();
                ui.heading(&k.anzeige);
                ui.label(egui::RichText::new(format!("Ordner: maps\\{}   {} Kacheln{}", k.ordner, k.kacheln,
                    if k.eigene { "   eigene Karte (Speichern ohne Rueckfrage)" } else if k.standard { "   Standardkarte von OMSI 2" } else { "" })).weak());
                if let Some(t) = self.vorschaubild(ctx, &k) {
                    let s = t.size_vec2();
                    let b = ui.available_width().min(560.0);
                    ui.image((t.id(), egui::vec2(b, b * s.y / s.x.max(1.0))));
                }
                if !k.beschreibung.is_empty() {
                    egui::ScrollArea::vertical().id_salt("beschreibung").max_height(140.0).show(ui, |ui| {
                        ui.label(&k.beschreibung);
                    });
                }
                ui.add_space(8.0);
                let offen = self.karte.as_deref() == Some(k.ordner.as_str());
                ui.horizontal(|ui| {
                    if ui.add_enabled(!offen, egui::Button::new(egui::RichText::new("Oeffnen").strong())).clicked() {
                        aktionen.push(UiAktion::Karte(i));
                    }
                    if ui.add_enabled(!offen, egui::Button::new("Umbenennen ...")).clicked() {
                        self.karten_dialog = Some(KartenDialog::Umbenennen(k.ordner.clone(), k.anzeige.clone(), k.ordner.clone()));
                    }
                    if ui.add_enabled(!offen, egui::Button::new("Loeschen ...")).clicked() {
                        self.karten_dialog = Some(KartenDialog::Loeschen1(k.ordner.clone()));
                    }
                });
                if offen {
                    ui.label(egui::RichText::new("Die Karte ist gerade offen: zum Umbenennen oder Loeschen zuerst eine andere oeffnen.").small().weak());
                }
            });
        });
    }

    /// Dialoge Umbenennen / Loeschen (zwei Rueckfragen)
    fn karten_dialoge(&mut self, ctx: &egui::Context, aktionen: &mut Vec<UiAktion>) {
        let Some(d) = self.karten_dialog.as_mut() else { return };
        let mut zu = false;
        match d {
            KartenDialog::Umbenennen(karte, anzeige, ordner) => {
                egui::Window::new("Karte umbenennen").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                    egui::Grid::new("umbenennen").num_columns(2).show(ui, |ui| {
                        ui.label("Anzeigename");
                        ui.text_edit_singleline(anzeige);
                        ui.end_row();
                        ui.label("Ordner (maps\\...)");
                        ui.text_edit_singleline(ordner);
                        ui.end_row();
                    });
                    ui.label(egui::RichText::new("Der Anzeigename steht in OMSIs Kartenauswahl. Der Ordner ist der interne Name: Spielstaende dieser Karte passen danach nicht mehr.").small().weak());
                    let ordner_ok = ordner == karte || (speichern::name_ok(ordner) && !self.root.join("maps").join(ordner.as_str()).exists());
                    if !ordner_ok {
                        ui.colored_label(egui::Color32::from_rgb(255, 120, 90), "Ordnername ungueltig oder schon vorhanden");
                    }
                    ui.horizontal(|ui| {
                        if ui.add_enabled(ordner_ok && !anzeige.trim().is_empty(), egui::Button::new("Umbenennen")).clicked() {
                            aktionen.push(UiAktion::KarteUmbenennen(karte.clone(), anzeige.trim().to_string(), ordner.trim().to_string()));
                            zu = true;
                        }
                        if ui.button("Abbrechen").clicked() {
                            zu = true;
                        }
                    });
                });
            }
            KartenDialog::Loeschen1(karte) => {
                let karte = karte.clone();
                let standard = self.karten.iter().any(|k| k.ordner == karte && k.standard);
                egui::Window::new("Karte loeschen?").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                    ui.label(format!("Soll die Karte \"{karte}\" wirklich geloescht werden?"));
                    ui.label("Sie kommt mit ihren eigenen Objekten (Aschaffenburg_KI) in den Papierkorb.");
                    if standard {
                        ui.colored_label(egui::Color32::from_rgb(255, 120, 90), "Das ist eine Standardkarte von OMSI 2 (Steam stellt sie nur ueber \"Dateien pruefen\" wieder her).");
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Ja, weiter ...").clicked() {
                            self.karten_dialog = Some(KartenDialog::Loeschen2(karte.clone(), String::new()));
                        }
                        if ui.button("Abbrechen").clicked() {
                            zu = true;
                        }
                    });
                });
            }
            KartenDialog::Loeschen2(karte, eingabe) => {
                egui::Window::new("Wirklich loeschen? (zweite Rueckfrage)").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                    ui.label(format!("Zur Bestaetigung den Ordnernamen eintippen: {karte}"));
                    ui.text_edit_singleline(eingabe);
                    ui.horizontal(|ui| {
                        if ui.add_enabled(eingabe.trim() == karte.as_str(), egui::Button::new("Endgueltig in den Papierkorb")).clicked() {
                            aktionen.push(UiAktion::KarteLoeschen(karte.clone()));
                            zu = true;
                        }
                        if ui.button("Abbrechen").clicked() {
                            zu = true;
                        }
                    });
                });
            }
        }
        if zu {
            self.karten_dialog = None;
        }
    }

    /// Ladeweite in Metern: weiter, je hoeher die Kamera steht
    fn sichtweite(&self) -> f64 {
        (self.kam.abstand * 2.5).clamp(900.0, 3000.0)
    }

    fn zeichnen(&mut self) {
        let Some(window) = self.window.clone() else { return };
        let jetzt = Instant::now();
        let dt = (jetzt - self.zuletzt).as_secs_f32().min(0.1);
        self.zuletzt = jetzt;
        self.fps = self.fps * 0.9 + 0.1 / dt.max(1e-4);
        if self.viewer.as_ref().map(|v| v.first_area_progress().is_none() && v.loaded_tiles() > 0).unwrap_or(false) && self.bilder > 5 {
            self.laengstes = self.laengstes.max(dt);
        }
        self.bewegen(dt);
        if self.katalog_job.as_ref().map(|j| j.is_finished()).unwrap_or(false) {
            if let Ok(k) = self.katalog_job.take().unwrap().join() {
                self.meldung = format!("Objektkatalog: {} Objekte in {} Ordnern ({:.1} s)", k.eintraege.len(), k.ordner.len(), self.gestartet.elapsed().as_secs_f32());
                if self.beenden_nach.is_some() {
                    println!("Testlauf: {}", self.meldung);
                }
                self.katalog = Some(k);
            }
        }
        if self.qs_job.as_ref().map(|j| j.is_finished()).unwrap_or(false) {
            if let Ok(q) = self.qs_job.take().unwrap().join() {
                if self.strasse.sli.is_none() {
                    self.strasse.sli = q.iter().find(|q| q.name == "str_2spur_10m_Grunewaldstr").or(q.first()).map(|q| q.rel.clone());
                }
                if self.strasse.kreisel_sli.is_none() {
                    self.strasse.kreisel_sli = strasse::kreisel_vorschlag(&q);
                }
                self.querschnitte = Some(q);
            }
        }
        if self.speichern_job.as_ref().map(|j| j.is_finished()).unwrap_or(false) {
            match self.speichern_job.take().unwrap().join() {
                Ok(Ok((ziel, ueberschrieben))) => {
                    self.bearb.aenderungen = 0;
                    self.strasse.aenderungen = 0;
                    if let Some(a) = self.aendern.as_mut() {
                        a.aenderungen = 0;
                    }
                    self.karten = karten::finden(&self.root);
                    if ueberschrieben {
                        // die Aenderungen stehen jetzt in der Karte: frisch laden (Sitzung, Netz, Verlauf von vorn), Kamera bleibt
                        self.kamera_merken = Some(self.kam.clone());
                        self.zu_laden = self.karte.as_ref().and_then(|k| self.karten.iter().position(|x| &x.ordner == k));
                        self.meldung = format!("gespeichert - Sicherung der ersetzten Dateien: {}", ziel.display());
                        self.speichern_meldung = Some(self.meldung.clone());
                    } else {
                        self.meldung = format!("gespeichert als neue Karte: {}", ziel.display());
                    }
                }
                Ok(Err(e)) => self.meldung = format!("Speichern fehlgeschlagen: {e:#}"),
                Err(_) => self.meldung = "Speichern fehlgeschlagen (Absturz im Hintergrund)".into(),
            }
        }
        if let Some(i) = self.zu_laden.take() {
            protokoll::aktion_lang(&format!("Karte oeffnen: {}", self.karten[i].ordner));
            match self.karte_oeffnen(i) {
                Ok(()) => {
                    log::info!("{}", self.meldung);
                    if let Some(k) = self.kamera_merken.take() {
                        self.kam = k;
                    }
                    if let Some(m) = self.speichern_meldung.take() {
                        self.meldung = m;
                    }
                }
                Err(e) => {
                    self.meldung = format!("Karte nicht geladen: {e:#}");
                    log::error!("{}", self.meldung);
                }
            }
            protokoll::aktion("");
            // Bericht der vorigen Sitzung (Absturz/Haenger) melden
            if let Some(b) = self.bericht_melden.take().flatten() {
                self.meldung = format!("Die letzte Sitzung ist abgestuerzt oder hing - Bericht: {}", b.display());
                log::warn!("{}", self.meldung);
            }
        }
        if self.surface.is_none() {
            // noch keine Karte: eine auf der Kommandozeile genannte oeffnen, sonst der Startbildschirm mit der Auswahl
            if let Some(s) = self.start_karte.take() {
                self.zu_laden = self.karten.iter().position(|k| k.ordner == s);
                if self.zu_laden.is_none() {
                    self.meldung = format!("Karte {s} nicht gefunden");
                }
                window.request_redraw();
                return;
            }
            if self.zu_laden.is_none() {
                self.startbild_zeichnen(&window);
            }
            return;
        }
        let (w, h) = {
            let s = self.surface.as_ref().unwrap();
            (s.config.width, s.config.height)
        };
        let frame = match self.surface.as_ref().unwrap().surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            _ => {
                let size = window.inner_size();
                if let (Some(s), Some(v)) = (self.surface.as_mut(), self.viewer.as_ref()) {
                    s.resize(&v.renderer, size.width, size.height);
                }
                return;
            }
        };
        let view = frame.texture.create_view(&Default::default());
        let cam = self.kam.camera();
        if let Some(v) = self.viewer.as_mut() {
            v.render(&view, w, h, &cam);
        }
        let view_ui = frame.texture.create_view(&wgpu::TextureViewDescriptor { format: Some(self.ui_format), ..Default::default() });
        self.oberflaeche(&window, &view_ui, w, h);
        window.pre_present_notify();
        frame.present();
    }

    fn oberflaeche(&mut self, window: &Window, view: &wgpu::TextureView, w: u32, h: u32) {
        let Some(mut gui) = self.gui.take() else { return };
        let input = gui.state.take_egui_input(window);
        let mut aktionen: Vec<UiAktion> = Vec::new();
        let (bw, bh) = (w as f32 / window.scale_factor() as f32, h as f32 / window.scale_factor() as f32);
        // Daten fuer die Anzeige vorab (die Oberflaeche liest den Viewer nur)
        let objekte = if self.bearb.werkzeug == Werkzeug::Objekte {
            self.viewer.as_ref().map(|v| self.bearb.objekte(v)).unwrap_or_default()
        } else {
            Vec::new()
        };
        let gewaehlt = self.bearb.wahl.and_then(|w| objekte.iter().find(|o| o.wahl == w).cloned());
        let edit = match gewaehlt.as_ref().map(|o| o.wahl) {
            Some(Wahl::Karte(id)) => self.viewer.as_ref().map(|v| v.object_edit(id)),
            _ => None,
        };
        let out = gui.ctx.run_ui(input, |ctx| {
            egui::Panel::top("werkzeuge").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.strong("OMSI-Editor");
                    if ui.button("Karten ...").on_hover_text("Karte waehlen, oeffnen, umbenennen, loeschen").clicked() {
                        self.kartenwahl_offen = !self.kartenwahl_offen;
                        self.kartenwahl = self.karte.clone();
                    }
                    ui.separator();
                    for (wz, t) in [(Werkzeug::Ansehen, "Ansehen"), (Werkzeug::Objekte, "Objekte (O)"), (Werkzeug::Platzieren, "Platzieren (P)"), (Werkzeug::Strasse, "Strasse bauen (B)"), (Werkzeug::Aendern, "Aendern (U)"), (Werkzeug::Kreuzung, "Kreuzungen (X)")] {
                        if ui.selectable_label(self.bearb.werkzeug == wz, t).clicked() {
                            aktionen.push(UiAktion::Werkzeug(wz));
                        }
                    }
                    ui.add_enabled(false, egui::Button::new("Gelaende"));
                    ui.separator();
                    // ein Verlauf fuer alle Werkzeuge (neue Schritte kommen beim naechsten Bild dazu)
                    let neu = self.stapel_jetzt() != self.stapel;
                    let (kr, kw) = (!self.verlauf.is_empty() || neu, !self.verlauf_redo.is_empty() && !neu);
                    if ui.add_enabled(kr, egui::Button::new("Rueckgaengig")).on_hover_text("Strg+Z").clicked() {
                        aktionen.push(UiAktion::Rueckgaengig);
                    }
                    if ui.add_enabled(kw, egui::Button::new("Wiederholen")).on_hover_text("Strg+Y").clicked() {
                        aktionen.push(UiAktion::Wiederholen);
                    }
                    ui.separator();
                    let geaendert = self.bearb.aenderungen + self.strasse.aenderungen + self.aendern.as_ref().map(|a| a.aenderungen).unwrap_or(0) > 0;
                    let frei = self.viewer.is_some() && self.speichern_job.is_none();
                    if ui.add_enabled(frei && geaendert, egui::Button::new(if geaendert { "Speichern *" } else { "Speichern" }))
                        .on_hover_text("Strg+S: die geoeffnete Karte ueberschreiben (die ersetzten Dateien werden vorher gesichert)").clicked() {
                        aktionen.push(UiAktion::SpeichernHier);
                    }
                    if ui.add_enabled(frei, egui::Button::new("Als neue Karte ...")).on_hover_text("Strg+Umschalt+S: Kopie unter neuem Namen, die geoeffnete Karte bleibt unveraendert").clicked() {
                        aktionen.push(UiAktion::SpeichernDialog);
                    }
                    if ui.button("Protokolle").on_hover_text("Ordner mit Logdateien, Absturz- und Haenger-Berichten oeffnen").clicked() {
                        let _ = std::process::Command::new("explorer").arg(protokoll::ordner()).spawn();
                    }
                });
            });
            if self.kartenwahl_offen {
                let mut offen = true;
                egui::Window::new("Karten").open(&mut offen).default_size([900.0, 560.0]).show(ctx, |ui| {
                    self.kartenwahl_ui(ui, &mut aktionen);
                });
                self.kartenwahl_offen = offen;
            }
            self.karten_dialoge(ctx, &mut aktionen);
            if self.bearb.werkzeug == Werkzeug::Objekte {
                egui::Panel::right("eigenschaften").default_size(270.0).show(ctx, |ui| {
                    ui.heading("Objekt");
                    if let Some(o) = gewaehlt.as_ref() {
                        let e = edit.unwrap_or_default();
                        let name = o.sco.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                        ui.label(egui::RichText::new(name).strong());
                        ui.label(egui::RichText::new(o.sco.parent().map(|p| p.display().to_string()).unwrap_or_default()).small().weak());
                        match o.wahl {
                            Wahl::Karte(id) => ui.label(format!("ID {}   Kachel {} {}", id, (o.pos.x / 300.0).floor(), (o.pos.y / 300.0).floor())),
                            Wahl::Neu(_) => ui.label(format!("neu   Kachel {} {}  (ID beim Speichern)", (o.pos.x / 300.0).floor(), (o.pos.y / 300.0).floor())),
                        };
                        ui.separator();
                        let mut p = o.pos;
                        let alt_h = o.richtung.rem_euclid(360.0);
                        let mut h = alt_h;
                        egui::Grid::new("pos").num_columns(2).show(ui, |ui| {
                            for (t, v) in [("x (Ost)", &mut p.x), ("y (Nord)", &mut p.y), ("Hoehe", &mut p.z)] {
                                ui.label(t);
                                ui.add(egui::DragValue::new(v).speed(0.05).suffix(" m").fixed_decimals(2));
                                ui.end_row();
                            }
                            ui.label("Richtung");
                            ui.add(egui::DragValue::new(&mut h).speed(0.5).suffix(" Grad").fixed_decimals(1));
                            ui.end_row();
                        });
                        let d = p - o.pos;
                        if d.length() > 1e-6 {
                            aktionen.push(UiAktion::Objekt(Action::Move(d)));
                        }
                        if (h - alt_h).abs() > 1e-6 {
                            aktionen.push(UiAktion::Objekt(Action::Turn(h - alt_h)));
                        }
                        if e != Default::default() {
                            ui.label(egui::RichText::new(format!("geaendert: {:+.2} / {:+.2} / {:+.2} m, {:+.1} Grad", e.moved.x, e.moved.y, e.moved.z, e.turned)).small());
                        }
                        ui.separator();
                        ui.horizontal(|ui| {
                            if ui.button("Loeschen").on_hover_text("Entf").clicked() {
                                aktionen.push(UiAktion::Objekt(Action::Delete));
                            }
                            if ui.button("Kopieren").on_hover_text("legt eine Kopie daneben").clicked() {
                                aktionen.push(UiAktion::Objekt(Action::Copy));
                            }
                            if matches!(o.wahl, Wahl::Karte(_)) && ui.button("Zuruecksetzen").clicked() {
                                aktionen.push(UiAktion::Objekt(Action::Undo));
                            }
                        });
                        if ui.button("Auswahl aufheben (Esc)").clicked() {
                            aktionen.push(UiAktion::Abwaehlen);
                        }
                    } else {
                        ui.label("Klick auf ein Objekt waehlt es aus.");
                    }
                    ui.separator();
                    ui.label(egui::RichText::new("Ziehen: verschieben | Strg+Rad oder , . : drehen (Umschalt fein) | Bild auf/ab: heben/senken | Entf: loeschen | Strg+Z/Y").small().weak());
                });
            }
            if self.bearb.werkzeug == Werkzeug::Platzieren {
                egui::Panel::right("katalog").default_size(360.0).show(ctx, |ui| {
                    ui.heading("Objekte platzieren");
                    let Some(kat) = self.katalog.as_ref() else {
                        ui.label("Objektkatalog wird eingelesen ...");
                        return;
                    };
                    ui.add(egui::TextEdit::singleline(&mut self.katalog_suche).hint_text("suchen (Name, Datei, Gruppe, Herkunft)").desired_width(f32::INFINITY));
                    egui::Grid::new("filter").num_columns(2).show(ui, |ui| {
                        ui.label("Herkunft");
                        let txt = self.katalog_herkunft.clone().unwrap_or("alle".into());
                        egui::ComboBox::from_id_salt("herkunft").width(220.0).selected_text(txt).show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.katalog_herkunft, None, format!("alle ({})", kat.eintraege.len()));
                            for (h, n) in &kat.herkuenfte {
                                ui.selectable_value(&mut self.katalog_herkunft, Some(h.clone()), format!("{h} ({n})"));
                            }
                        });
                        ui.end_row();
                        ui.label("Ordner");
                        egui::ComboBox::from_id_salt("ordner").width(220.0).selected_text(self.katalog_ordner.clone().unwrap_or("alle".into())).show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.katalog_ordner, None, "alle");
                            for o in &kat.ordner {
                                ui.selectable_value(&mut self.katalog_ordner, Some(o.clone()), o);
                            }
                        });
                        ui.end_row();
                        ui.label("Gruppe");
                        egui::ComboBox::from_id_salt("gruppe").width(220.0).selected_text(self.katalog_gruppe.clone().unwrap_or("alle".into())).show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.katalog_gruppe, None, "alle");
                            for g in &kat.gruppen {
                                ui.selectable_value(&mut self.katalog_gruppe, Some(g.clone()), g);
                            }
                        });
                        ui.end_row();
                    });
                    let suche = self.katalog_suche.to_lowercase();
                    let treffer: Vec<&katalog::Eintrag> = kat.eintraege.iter()
                        .filter(|e| e.passt(&suche, self.katalog_ordner.as_deref(), self.katalog_gruppe.as_deref(), self.katalog_herkunft.as_deref()))
                        .collect();
                    ui.label(egui::RichText::new(format!("{} von {} Objekten", treffer.len(), kat.eintraege.len())).small().weak());
                    ui.separator();
                    // Kachelraster mit Vorschaubildern (nur die sichtbaren Zeilen werden gebaut und gerendert)
                    let bild = vorschau::GROESSE as f32;
                    let zelle = bild + 14.0;
                    let spalten = ((ui.available_width() / zelle).floor() as usize).max(1);
                    let zeilen = treffer.len().div_ceil(spalten);
                    let hoehe_zeile = bild + 34.0;
                    let ctx2 = ui.ctx().clone();
                    egui::ScrollArea::vertical().auto_shrink(false).max_height(ui.available_height() - 60.0).show_rows(ui, hoehe_zeile, zeilen, |ui, bereich| {
                        for z in bereich {
                            ui.horizontal(|ui| {
                                for e in treffer.iter().skip(z * spalten).take(spalten) {
                                    let aktiv = self.platzier.as_deref() == Some(e.rel.as_str());
                                    let textur = self.vorschau.textur(&ctx2, &e.rel);
                                    let fehlt = self.vorschau.fehlt(&e.rel);
                                    let antwort = ui.vertical(|ui| {
                                        ui.set_width(zelle - 6.0);
                                        let (rect, r) = ui.allocate_exact_size(egui::vec2(bild, bild), egui::Sense::click());
                                        let rand = if aktiv { egui::Color32::from_rgb(255, 60, 220) } else if r.hovered() { egui::Color32::WHITE } else { egui::Color32::from_gray(70) };
                                        ui.painter().rect_filled(rect, 4.0, egui::Color32::from_gray(35));
                                        match textur {
                                            Some(t) => {
                                                egui::Image::new(&t).corner_radius(4.0).paint_at(ui, rect);
                                            }
                                            None => {
                                                let t = if fehlt { "kein Bild" } else { "..." };
                                                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, t, egui::FontId::proportional(11.0), egui::Color32::GRAY);
                                            }
                                        }
                                        ui.painter().rect_stroke(rect, 4.0, egui::Stroke::new(if aktiv { 3.0 } else { 1.0 }, rand), egui::StrokeKind::Inside);
                                        let mut name = e.name.clone();
                                        if name.chars().count() > 16 {
                                            name = name.chars().take(15).collect::<String>() + "…";
                                        }
                                        ui.label(egui::RichText::new(name).small());
                                        r
                                    }).inner;
                                    let antwort = antwort.on_hover_text(format!("{}\n{}\nHerkunft: {}\nGruppen: {}", e.name, e.rel, e.herkunft, e.gruppen.join(", ")));
                                    if antwort.clicked() {
                                        aktionen.push(UiAktion::Platzier(if aktiv { None } else { Some(e.rel.clone()) }));
                                    }
                                }
                            });
                        }
                    });
                    ui.separator();
                    match self.platzier.as_deref() {
                        Some(rel) => {
                            ui.label(egui::RichText::new(rel).small());
                            ui.label(egui::RichText::new(format!("Klick in die Welt setzt (mehrfach), , . drehen ({:.0} Grad), Esc beendet", self.bearb.platzier_richtung.rem_euclid(360.0))).small().weak());
                        }
                        None => {
                            ui.label(egui::RichText::new("Objekt waehlen, dann in die Welt klicken.").small().weak());
                        }
                    }
                });
            }
            if self.bearb.werkzeug == Werkzeug::Strasse {
                egui::Panel::right("strasse").default_size(360.0).show(ctx, |ui| {
                    ui.heading("Strasse bauen");
                    ui.horizontal(|ui| {
                        for (m, t) in [(strasse::Modus::Gerade, "Gerade (G)"), (strasse::Modus::Kurve, "Kurve (K)"), (strasse::Modus::Kreisel, "Kreisverkehr (V)")] {
                            if ui.selectable_label(self.strasse.modus == m, t).clicked() {
                                aktionen.push(UiAktion::StrassenModus(m));
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label(format!("Hoehe naechster Punkt: {:+.2} m", self.strasse.hoehe));
                        if ui.small_button("-").clicked() {
                            aktionen.push(UiAktion::StrassenHoehe(-1.0));
                        }
                        if ui.small_button("+").clicked() {
                            aktionen.push(UiAktion::StrassenHoehe(1.0));
                        }
                        if ui.small_button("0").clicked() {
                            aktionen.push(UiAktion::StrassenHoehe(-self.strasse.hoehe));
                        }
                    });
                    if self.strasse.baut() && ui.button("Zug beenden (Esc / Rechtsklick)").clicked() {
                        aktionen.push(UiAktion::ZugBeenden);
                    }
                    ui.checkbox(&mut self.strasse.uebernehmen, "an vorhandenen Strassen deren Querschnitt uebernehmen");
                    let frei = self.anschluesse.liste.iter().filter(|a| a.frei).count();
                    ui.label(egui::RichText::new(format!("{} Knoten, {} Strassenstuecke | {} freie Enden vorhandener Strassen (blau)", self.strasse.netz.knoten.len(), self.strasse.netz.kanten.len(), frei)).small().weak());
                    ui.separator();
                    if self.strasse.modus == strasse::Modus::Kreisel {
                        ui.label("Querschnitt des Rings (Einbahn, gegen den Uhrzeigersinn befahren)");
                        let einbahn = self.querschnitte.as_ref().and_then(|q| q.iter().find(|q| Some(&q.rel) == self.strasse.kreisel_sli.as_ref())).map(|q| q.zurueck == 0 && q.vor >= 1);
                        if einbahn == Some(false) {
                            ui.colored_label(egui::Color32::from_rgb(255, 140, 90), "kein Einbahn-Querschnitt - im Kreisverkehr fuehren Spuren falsch herum");
                        }
                        ui.label(egui::RichText::new("Klick setzt die Mitte, die Maus die Groesse (Durchmesser 24 bis 120 m), Klick baut. Danach Zufahrten auf den Ring ziehen: dort entstehen T-Kreuzungen, der Ring hat Vorfahrt.").small().weak());
                        if let Some(rel) = self.qs_raster(ui, self.strasse.kreisel_sli.clone()) {
                            aktionen.push(UiAktion::KreiselQuerschnitt(rel));
                        }
                    } else {
                        ui.label("Querschnitt");
                        if let Some(rel) = self.qs_raster(ui, self.strasse.sli.clone()) {
                            aktionen.push(UiAktion::Querschnitt(rel));
                        }
                    }
                    ui.separator();
                    ui.label(egui::RichText::new("Klick: Start / naechster Punkt (rastet an Knoten ein, gruen = freies Ende: tangential weiter). Bild auf/ab: Hoehe. Entf: Strasse unter der Maus loeschen.").small().weak());
                });
            }
            if self.bearb.werkzeug == Werkzeug::Kreuzung {
                egui::Panel::right("kreuzungen").default_size(360.0).show(ctx, |ui| {
                    ui.heading("Kreuzungen: Vorfahrt und Ampel");
                    let info = self.kreuzung_wahl.and_then(|k| self.strasse.kreuzungen().into_iter().find(|x| x.knoten == k));
                    let Some(info) = info else {
                        ui.label("Klick auf eine Kreuzung waehlt sie. Vorhandene Kreuzungen der Karte (orange) werden beim Klick durch eine eigene mit denselben Armen ersetzt.");
                        ui.label(egui::RichText::new(format!("{} eigene Kreuzungen", self.strasse.kreuzungen().len())).small().weak());
                        return;
                    };
                    use kreuzung::Rolle;
                    let regel_mit = |rollen: Vec<Rolle>, ampel: bool| netz::Regel { rollen: info.arme.iter().zip(rollen).map(|(a, r)| (a.0, r)).collect(), ampel };
                    let jetzt: Vec<Rolle> = info.arme.iter().map(|a| a.1).collect();
                    ui.label(if info.vermutet { "Vorfahrt vermutet (noch nicht festgelegt)" } else { "Vorfahrt festgelegt" });
                    ui.separator();
                    let himmel = |h: f64| ["Nord", "Nordost", "Ost", "Suedost", "Sued", "Suedwest", "West", "Nordwest"][(((h + 22.5).rem_euclid(360.0)) / 45.0) as usize % 8];
                    for (i, a) in info.arme.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(format!("Arm {} ({}, {:.0} Grad)", i + 1, himmel(a.0), a.0));
                        });
                        ui.horizontal(|ui| {
                            for (r, t) in [(Rolle::Haupt, "Hauptstrasse"), (Rolle::Neben, "wartet"), (Rolle::Gleich, "rechts vor links")] {
                                if ui.selectable_label(a.1 == r, t).clicked() && a.1 != r {
                                    let mut neu = jetzt.clone();
                                    neu[i] = r;
                                    aktionen.push(UiAktion::KreuzungRegel(info.knoten, Some(regel_mit(neu, info.ampel))));
                                }
                            }
                        });
                    }
                    if jetzt.iter().filter(|r| **r == Rolle::Haupt).count() == 1 {
                        ui.colored_label(egui::Color32::from_rgb(255, 140, 90), "nur ein Arm ist Hauptstrasse - meist sind es zwei (die durchgehende Strasse)");
                    }
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button("alle rechts vor links").clicked() {
                            aktionen.push(UiAktion::KreuzungRegel(info.knoten, Some(regel_mit(vec![Rolle::Gleich; jetzt.len()], info.ampel))));
                        }
                        if ui.button("Vorfahrt vermuten").on_hover_text("die Regel des Editors: Ring, durchgehende Strasse, sonst rechts vor links").clicked() {
                            aktionen.push(UiAktion::KreuzungRegel(info.knoten, None));
                        }
                    });
                    ui.separator();
                    let mut ampel = info.ampel;
                    if ui.checkbox(&mut ampel, "Ampel").changed() {
                        aktionen.push(UiAktion::KreuzungRegel(info.knoten, Some(regel_mit(jetzt.clone(), ampel))));
                    }
                    if info.ampel {
                        let n = info.phasen.iter().max().map(|x| x + 1).unwrap_or(0);
                        ui.label(format!("Umlauf {:.0} s, {} Phasen (gegenueberliegende Arme gemeinsam gruen, die Hauptstrasse zuerst und laenger)",
                                         info.umlauf.unwrap_or(0.0), n));
                        for (i, ph) in info.phasen.iter().enumerate() {
                            ui.label(egui::RichText::new(format!("  Arm {}: Phase {}", i + 1, ph + 1)).small());
                        }
                    }
                    ui.separator();
                    ui.label(egui::RichText::new("Farben im Bild: gruen = Hauptstrasse, rot = wartet, gelb = rechts vor links. Strg+Z nimmt jede Aenderung zurueck.").small().weak());
                });
            }
            if self.bearb.werkzeug == Werkzeug::Aendern {
                egui::Panel::right("aendern").default_size(360.0).show(ctx, |ui| {
                    ui.heading("Strassen aendern");
                    if !self.eigene_auswahl.is_empty() {
                        let n = self.eigene_auswahl.len();
                        let e = self.strasse.netz.kante(self.eigene_auswahl[0]).cloned();
                        ui.label(egui::RichText::new(if n > 1 { format!("{n} eigene Strassenstuecke gewaehlt") } else { "eigene Strasse".to_string() }).strong());
                        if let Some(e) = &e {
                            let laenge: f64 = self.eigene_auswahl.iter().filter_map(|id| self.strasse.netz.kante(*id))
                                .map(|k| self.strasse.netz.elemente(k).iter().map(|x| x.stueck.laenge).sum::<f64>()).sum();
                            ui.label(egui::RichText::new(e.sli.rsplit('\\').next().unwrap_or("").to_string()).strong());
                            ui.label(egui::RichText::new(&e.sli).small().weak());
                            ui.label(format!("Laenge {laenge:.1} m{}", if e.ring { "   Kreisverkehr" } else { "" }));
                        }
                        ui.horizontal(|ui| {
                            if ui.button("Richtung umkehren").clicked() {
                                aktionen.push(UiAktion::EigeneAendern(strasse::KantenAenderung::Umkehren));
                            }
                            if ui.button("Loeschen").on_hover_text("Entf").clicked() {
                                aktionen.push(UiAktion::EigeneAendern(strasse::KantenAenderung::Loeschen));
                            }
                        });
                        ui.separator();
                        ui.label("Upgrade: neuer Querschnitt");
                        if let Some(rel) = self.qs_raster(ui, e.map(|e| e.sli)) {
                            aktionen.push(UiAktion::EigeneAendern(strasse::KantenAenderung::Querschnitt(rel)));
                        }
                        return;
                    }
                    let info: Option<(usize, aendern::KartenSpline)> = self.aendern.as_ref().and_then(|a| {
                        a.auswahl.first().and_then(|id| a.spline(*id)).map(|s| (a.auswahl.len(), s.clone()))
                    });
                    match info {
                        None => {
                            ui.label("Klick auf eine Strasse waehlt sie (Umschalt+Klick: ganze Kette, Strg+Klick: dazu/weg).");
                        }
                        Some((n, s)) => {
                            if n > 1 {
                                ui.label(egui::RichText::new(format!("{n} Splines gewaehlt")).strong());
                            }
                            ui.label(egui::RichText::new(s.sli.rsplit('\\').next().unwrap_or("").to_string()).strong());
                            ui.label(egui::RichText::new(&s.sli).small().weak());
                            ui.label(format!("ID {}   Kachel {} {}", s.id, s.kachel.0, s.kachel.1));
                            let k = &s.kurve;
                            ui.label(format!("Laenge {:.1} m   {}   Steigung {:+.1} / {:+.1} %{}", k.length,
                                             if k.radius == 0.0 { "gerade".to_string() } else { format!("Radius {:.0} m", k.radius) },
                                             k.grad_start, k.grad_end, if s.gespiegelt { "   gespiegelt" } else { "" }));
                            ui.horizontal(|ui| {
                                if ui.button("Richtung umkehren").on_hover_text("spiegelt den Spline (mirror)").clicked() {
                                    aktionen.push(UiAktion::SplineSpiegeln);
                                }
                                if ui.button("Loeschen").on_hover_text("Entf").clicked() {
                                    aktionen.push(UiAktion::SplineLoeschen);
                                }
                            });
                            if let Some(w) = self.aendern_warnung.as_ref() {
                                ui.colored_label(egui::Color32::from_rgb(255, 140, 90), w);
                            }
                            ui.separator();
                            ui.label("Upgrade: neuer Querschnitt");
                            if let Some(rel) = self.qs_raster(ui, Some(s.sli.clone())) {
                                aktionen.push(UiAktion::SplineQuerschnitt(rel));
                            }
                        }
                    }
                });
            }
            egui::Panel::bottom("status").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(format!("{:.0} fps", self.fps));
                    ui.separator();
                    if let Some(v) = self.viewer.as_ref() {
                        match v.first_area_progress() {
                            Some((a, b)) => ui.label(format!("lade Kacheln {a}/{b}")),
                            None => ui.label(format!("{} Kacheln geladen", v.loaded_tiles())),
                        };
                        ui.separator();
                    }
                    if let Some(p) = self.boden_unter_maus {
                        ui.label(format!("x {:.1}  y {:.1}  Hoehe {:.2} m  (Kachel {} {})", p.x, p.y, p.z,
                                         (p.x / 300.0).floor(), (p.y / 300.0).floor()));
                        ui.separator();
                    }
                    ui.label(&self.meldung);
                });
            });
            // Markierungen ueber dem 3D-Bild (unter den Panels)
            let maler = ui_maler(ctx);
            if let Some(w) = self.bearb.unter_maus.filter(|w| Some(*w) != self.bearb.wahl) {
                if let Some(o) = objekte.iter().find(|o| o.wahl == w) {
                    bearbeiten::markieren(&maler, &self.kam, o, bw, bh, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 170), 1.5);
                }
            }
            if let Some(o) = gewaehlt.as_ref() {
                bearbeiten::markieren(&maler, &self.kam, o, bw, bh, egui::Color32::from_rgb(255, 60, 220), 3.0);
            }
            if self.bearb.werkzeug == Werkzeug::Kreuzung {
                let maus_netz = match &self.kreuzung_maus { Some(KreuzungsZiel::Netz(k)) => Some(*k), _ => None };
                for k in self.strasse.kreuzungen() {
                    let Some(m) = bearbeiten::projizieren(&self.kam, k.pos + DVec3::Z * 0.3, bw, bh).map(|(x, y, _)| egui::pos2(x, y)) else { continue };
                    for a in &k.arme {
                        let farbe = match a.1 {
                            kreuzung::Rolle::Haupt => egui::Color32::from_rgb(70, 220, 90),
                            kreuzung::Rolle::Neben => egui::Color32::from_rgb(240, 70, 60),
                            kreuzung::Rolle::Gleich => egui::Color32::from_rgb(250, 210, 60),
                        };
                        if let Some((x, y, _)) = bearbeiten::projizieren(&self.kam, a.2 + DVec3::Z * 0.3, bw, bh) {
                            maler.line_segment([m, egui::pos2(x, y)], egui::Stroke::new(4.0, farbe));
                        }
                    }
                    let (f, r) = if Some(k.knoten) == self.kreuzung_wahl { (egui::Color32::from_rgb(255, 60, 220), 9.0) }
                                 else if Some(k.knoten) == maus_netz { (egui::Color32::WHITE, 8.0) } else { (egui::Color32::from_gray(200), 6.0) };
                    maler.circle_stroke(m, r, egui::Stroke::new(2.5, f));
                    if k.ampel {
                        maler.text(m + egui::vec2(10.0, -10.0), egui::Align2::LEFT_BOTTOM, "Ampel", egui::FontId::proportional(13.0), egui::Color32::WHITE);
                    }
                }
                if let Some(KreuzungsZiel::Vorhanden(k)) = &self.kreuzung_maus {
                    let c = egui::Color32::from_rgb(255, 170, 40);
                    if let Some((x, y, _)) = bearbeiten::projizieren(&self.kam, k.pos + DVec3::Z * 0.3, bw, bh) {
                        maler.circle_stroke(egui::pos2(x, y), 10.0, egui::Stroke::new(2.5, c));
                        for a in &k.arme {
                            if let Some((x2, y2, _)) = bearbeiten::projizieren(&self.kam, a.pos + DVec3::Z * 0.3, bw, bh) {
                                maler.line_segment([egui::pos2(x, y), egui::pos2(x2, y2)], egui::Stroke::new(2.0, c));
                            }
                        }
                        maler.text(egui::pos2(x + 14.0, y + 12.0), egui::Align2::LEFT_TOP, "vorhandene Kreuzung - Klick uebernimmt sie", egui::FontId::proportional(14.0), c);
                    }
                }
            }
            if self.bearb.werkzeug == Werkzeug::Aendern {
                let mut eigene: Vec<(u32, egui::Color32, f32)> = self.eigene_auswahl.iter().map(|id| (*id, egui::Color32::from_rgb(255, 60, 220), 3.0)).collect();
                if let Some(id) = self.eigene_maus.filter(|id| !self.eigene_auswahl.contains(id)) {
                    eigene.push((id, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 200), 2.0));
                }
                for (id, farbe, dick) in eigene {
                    let Some((punkte, w)) = self.strasse.kante_umriss(id) else { continue };
                    for seite in [-w, w] {
                        let linie: Vec<egui::Pos2> = punkte.iter().filter_map(|(q, h)| {
                            let p = *q + (crate::netz::rechts(*h) * seite).extend(0.2);
                            bearbeiten::projizieren(&self.kam, p, bw, bh).map(|(x, y, _)| egui::pos2(x, y))
                        }).collect();
                        if linie.len() > 1 {
                            maler.add(egui::Shape::line(linie, egui::Stroke::new(dick, farbe)));
                        }
                    }
                }
                if let (Some(a), Some(v)) = (self.aendern.as_mut(), self.viewer.as_ref()) {
                    let mut zeigen: Vec<(i64, egui::Color32, f32)> = a.auswahl.iter().map(|id| (*id, egui::Color32::from_rgb(255, 60, 220), 3.0)).collect();
                    if let Some(id) = a.unter_maus.filter(|id| !a.auswahl.contains(id)) {
                        zeigen.push((id, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 200), 2.0));
                    }
                    for (id, farbe, dick) in zeigen {
                        let Some(s) = a.spline(id).cloned() else { continue };
                        let (l, r) = a.breite(v, &s.sli);
                        let (l, r) = if s.gespiegelt { (r, l) } else { (l, r) };
                        for seite in [-(l as f64), r as f64] {
                            let linie: Vec<egui::Pos2> = s.punkte().into_iter().filter_map(|(q, h)| {
                                let p = q + (crate::netz::rechts(h) * seite).extend(0.2);
                                bearbeiten::projizieren(&self.kam, p, bw, bh).map(|(x, y, _)| egui::pos2(x, y))
                            }).collect();
                            if linie.len() > 1 {
                                maler.add(egui::Shape::line(linie, egui::Stroke::new(dick, farbe)));
                            }
                        }
                    }
                }
            }
            if self.bearb.werkzeug == Werkzeug::Strasse {
                for a in self.anschluesse.liste.iter().filter(|a| a.frei) {
                    if (a.pos - self.kam.ziel).length() > self.kam.abstand * 3.0 + 200.0 {
                        continue;
                    }
                    if let Some((x, y, _)) = bearbeiten::projizieren(&self.kam, a.pos, bw, bh) {
                        let c = egui::Color32::from_rgb(80, 200, 255);
                        maler.circle_stroke(egui::pos2(x, y), 7.0, egui::Stroke::new(2.5, c));
                        // Richtung, in der es weitergeht
                        let s = a.pos + glam::DVec3::new(a.richtung.to_radians().sin(), a.richtung.to_radians().cos(), 0.0) * 6.0;
                        if let Some((x2, y2, _)) = bearbeiten::projizieren(&self.kam, s, bw, bh) {
                            maler.line_segment([egui::pos2(x, y), egui::pos2(x2, y2)], egui::Stroke::new(2.0, c));
                        }
                    }
                }
                for (_, p, frei) in self.strasse.knoten_punkte() {
                    if let Some((x, y, _)) = bearbeiten::projizieren(&self.kam, p, bw, bh) {
                        let f = if frei { egui::Color32::from_rgb(90, 230, 120) } else { egui::Color32::WHITE };
                        maler.circle_stroke(egui::pos2(x, y), 6.0, egui::Stroke::new(2.0, f));
                    }
                }
                // geplante Kreuzungen: Mitte, aufgeschnittenes Stueck der Strasse, neuer Arm
                let mut kreuzungen: Vec<strasse::KreuzungsVorschau> = self.strasse.zeiger.iter().cloned().collect();
                if self.strasse.baut() {
                    kreuzungen.extend(self.strasse.plan.iter().flat_map(|p| p.kreuzungen.iter().cloned()));
                }
                for k in kreuzungen {
                    let c = egui::Color32::from_rgb(255, 170, 40);
                    let d = crate::netz::dir(k.strasse) * k.schnitt;
                    let p = |q: glam::DVec2| bearbeiten::projizieren(&self.kam, q.extend(k.mitte.z + 0.2), bw, bh).map(|(x, y, _)| egui::pos2(x, y));
                    if let (Some(a), Some(b), Some(m), Some(e)) = (p(k.mitte.truncate() - d), p(k.mitte.truncate() + d), p(k.mitte.truncate()), p(k.arm.truncate())) {
                        maler.line_segment([a, b], egui::Stroke::new(4.0, c));
                        maler.line_segment([m, e], egui::Stroke::new(2.0, c));
                        maler.circle_stroke(m, 6.0, egui::Stroke::new(2.5, c));
                        maler.circle_filled(e, 4.0, c);
                    }
                }
                if let (Some(plan), Some(m)) = (self.strasse.plan.as_ref(), self.maus) {
                    let warn = plan.min_radius < strasse::MIN_RADIUS || plan.steigung.abs() > strasse::MAX_STEIGUNG;
                    let mut t = format!("{:.1} m", plan.laenge);
                    if plan.min_radius.is_finite() {
                        t += &format!("   R {:.0} m", plan.min_radius);
                    }
                    t += &format!("   {:+.1} %", plan.steigung);
                    if plan.angeschlossen {
                        t += "   Anschluss";
                    }
                    if !plan.kreuzungen.is_empty() {
                        t += "   Kreuzung";
                    }
                    if let Some(w) = plan.warnung.as_ref() {
                        t += &format!("\n{w}");
                    }
                    let warn = warn || plan.warnung.is_some();
                    let farbe = if warn { egui::Color32::from_rgb(255, 120, 90) } else { egui::Color32::WHITE };
                    maler.text(egui::pos2(m.0 + 18.0, m.1 + 14.0), egui::Align2::LEFT_TOP, t, egui::FontId::proportional(14.0), farbe);
                }
            }
            // Dialoge
            if let Some(name) = self.speichern_name.as_mut() {
                let mut offen = true;
                egui::Window::new("Als neue Karte speichern").collapsible(false).resizable(false).open(&mut offen).show(ctx, |ui| {
                    ui.label(format!("Kopie von {} mit allen Aenderungen ({}). Die Originalkarte bleibt unveraendert.",
                                     self.karte.as_deref().unwrap_or("-"), self.bearb.aenderungen));
                    ui.horizontal(|ui| {
                        ui.label("Name");
                        ui.text_edit_singleline(name);
                    });
                    let ok = speichern::name_ok(name) && !self.root.join("maps").join(name.as_str()).exists();
                    if !ok {
                        ui.colored_label(egui::Color32::from_rgb(255, 120, 90), "Name ungueltig oder schon vorhanden");
                    }
                    if ui.add_enabled(ok, egui::Button::new("Speichern")).clicked() {
                        aktionen.push(UiAktion::Speichern(name.clone()));
                    }
                });
                if !offen {
                    self.speichern_name = None;
                }
            }
            if self.ueberschreiben_frage {
                let karte = self.karte.clone().unwrap_or_default();
                egui::Window::new("Karte ueberschreiben?").collapsible(false).resizable(false).show(ctx, |ui| {
                    ui.label(format!("\"{karte}\" wurde nicht mit diesem Editor angelegt (Standard- oder Fremdkarte)."));
                    ui.label("Beim Ueberschreiben werden die ersetzten Dateien vorher gesichert:");
                    ui.label(egui::RichText::new(speichern::sicherungen().join(&karte).display().to_string()).small().weak());
                    ui.label("Fuer Standardkarten ist \"Als neue Karte\" meist besser (Steam kann die Originale bei einer Pruefung zuruecksetzen).");
                    ui.horizontal(|ui| {
                        if ui.button("Ueberschreiben").clicked() {
                            aktionen.push(UiAktion::Ueberschreiben);
                        }
                        if ui.button("Als neue Karte ...").clicked() {
                            self.ueberschreiben_frage = false;
                            aktionen.push(UiAktion::SpeichernDialog);
                        }
                        if ui.button("Abbrechen").clicked() {
                            self.ueberschreiben_frage = false;
                        }
                    });
                });
            }
            if let Some(i) = self.verwerfen_frage {
                egui::Window::new("Ungespeicherte Aenderungen").collapsible(false).resizable(false).show(ctx, |ui| {
                    ui.label(format!("{} Aenderungen an {} gehen verloren.", self.bearb.aenderungen, self.karte.as_deref().unwrap_or("-")));
                    ui.horizontal(|ui| {
                        if ui.button("Verwerfen und wechseln").clicked() {
                            aktionen.push(UiAktion::Verwerfen(i));
                        }
                        if ui.button("Abbrechen").clicked() {
                            aktionen.push(UiAktion::FrageZu);
                        }
                    });
                });
            }
        });
        gui.state.handle_platform_output(window, out.platform_output);
        if matches!(self.bearb.werkzeug, Werkzeug::Platzieren | Werkzeug::Strasse) {
            if let Some(v) = self.viewer.as_mut() {
                self.vorschau.erzeugen(v, &gui.ctx, 8);
            }
        }
        let Some(v) = self.viewer.as_ref() else {
            self.gui = Some(gui);
            return;
        };
        let (device, queue) = (&v.renderer.device, &v.renderer.queue);
        let jobs = gui.ctx.tessellate(out.shapes, out.pixels_per_point);
        let sd = egui_wgpu::ScreenDescriptor { size_in_pixels: [w, h], pixels_per_point: out.pixels_per_point };
        for (id, delta) in &out.textures_delta.set {
            gui.renderer.update_texture(device, queue, *id, delta);
        }
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("egui") });
        let extra = gui.renderer.update_buffers(device, queue, &mut enc, &jobs, &sd);
        {
            let pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            gui.renderer.render(&mut pass.forget_lifetime(), &jobs, &sd);
        }
        queue.submit(extra.into_iter().chain(std::iter::once(enc.finish())));
        for id in &out.textures_delta.free {
            gui.renderer.free_texture(id);
        }
        self.gui = Some(gui);
        for a in aktionen {
            self.ausfuehren(a);
        }
    }
}

impl App {
    fn ausfuehren(&mut self, a: UiAktion) {
        match a {
            UiAktion::Karte(i) => {
                if self.bearb.aenderungen + self.strasse.aenderungen + self.aendern.as_ref().map(|a| a.aenderungen).unwrap_or(0) > 0 {
                    self.verwerfen_frage = Some(i);
                } else {
                    self.zu_laden = Some(i);
                    self.meldung = format!("lade {} ...", self.karten[i].ordner);
                }
            }
            UiAktion::Verwerfen(i) => {
                self.verwerfen_frage = None;
                self.bearb.aenderungen = 0;
                self.strasse.aenderungen = 0;
                if let Some(a) = self.aendern.as_mut() {
                    a.aenderungen = 0;
                }
                self.zu_laden = Some(i);
            }
            UiAktion::FrageZu => self.verwerfen_frage = None,
            UiAktion::SplineQuerschnitt(rel) => {
                if let (Some(v), Some(a)) = (self.viewer.as_mut(), self.aendern.as_mut()) {
                    match a.querschnitt(v, &rel) {
                        Ok((n, w)) => {
                            self.meldung = format!("{n} Spline(s) auf {} umgestellt", rel.rsplit('\\').next().unwrap_or(""));
                            self.aendern_warnung = w;
                        }
                        Err(e) => self.meldung = format!("Aendern fehlgeschlagen: {e:#}"),
                    }
                    self.anschluesse.vergessen();
                }
            }
            UiAktion::SplineSpiegeln => {
                if let (Some(v), Some(a)) = (self.viewer.as_mut(), self.aendern.as_mut()) {
                    match a.spiegeln(v) {
                        Ok(n) => self.meldung = format!("{n} Spline(s) umgekehrt"),
                        Err(e) => self.meldung = format!("Aendern fehlgeschlagen: {e:#}"),
                    }
                    self.anschluesse.vergessen();
                }
            }
            UiAktion::SplineLoeschen => {
                if let (Some(v), Some(a)) = (self.viewer.as_mut(), self.aendern.as_mut()) {
                    if !a.auswahl.is_empty() {
                        match a.loeschen(v) {
                            Ok(n) => self.meldung = format!("{n} Spline(s) geloescht"),
                            Err(e) => self.meldung = format!("Loeschen fehlgeschlagen: {e:#}"),
                        }
                        self.anschluesse.vergessen();
                    }
                }
            }
            UiAktion::Querschnitt(rel) => self.strasse.sli = Some(rel),
            UiAktion::StrassenModus(m) => {
                if m != self.strasse.modus {
                    if let Some(v) = self.viewer.as_mut() {
                        self.strasse.beenden(v);
                    }
                }
                self.strasse.modus = m;
            }
            UiAktion::KreiselQuerschnitt(rel) => self.strasse.kreisel_sli = Some(rel),
            UiAktion::KreuzungRegel(k, regel) => {
                if let Some(v) = self.viewer.as_mut() {
                    self.meldung = self.strasse.regel_setzen(v, k, regel);
                }
            }
            UiAktion::EigeneAendern(art) => {
                if let Some(v) = self.viewer.as_mut() {
                    let ids = self.eigene_auswahl.clone();
                    self.meldung = self.strasse.kanten_aendern(v, &ids, &art);
                    if matches!(art, strasse::KantenAenderung::Loeschen) {
                        self.eigene_auswahl.clear();
                    }
                    self.anschluesse.vergessen();
                }
            }
            UiAktion::StrassenHoehe(d) => self.strasse.hoehe = (self.strasse.hoehe + d).clamp(-30.0, 40.0),
            UiAktion::ZugBeenden => {
                if let Some(v) = self.viewer.as_mut() {
                    self.strasse.beenden(v);
                }
            }
            UiAktion::Werkzeug(w) => {
                if w != Werkzeug::Strasse {
                    if let Some(v) = self.viewer.as_mut() {
                        self.strasse.beenden(v);
                    }
                }
                self.bearb.werkzeug = w;
                if w != Werkzeug::Objekte {
                    self.bearb.waehlen(None);
                    self.bearb.unter_maus = None;
                }
                if w != Werkzeug::Platzieren {
                    self.platzier = None;
                    if let Some(v) = self.viewer.as_mut() {
                        self.bearb.geist_weg(v);
                    }
                }
            }
            UiAktion::Platzier(rel) => {
                self.platzier = rel;
                if let Some(v) = self.viewer.as_mut() {
                    self.bearb.geist_weg(v);
                    if let (Some(r), Some(g)) = (self.platzier.clone(), self.boden_unter_maus) {
                        self.bearb.geist(v, &r, g);
                        if !self.bearb.geist_ok() {
                            self.meldung = format!("{r} laesst sich nicht laden");
                        }
                    }
                }
            }
            UiAktion::Objekt(act) => {
                if let Some(v) = self.viewer.as_mut() {
                    if let Some(m) = self.bearb.aktion(v, act) {
                        self.meldung = m;
                    }
                }
            }
            UiAktion::Rueckgaengig => self.verlauf_schritt(true),
            UiAktion::Wiederholen => self.verlauf_schritt(false),
            UiAktion::Abwaehlen => self.bearb.waehlen(None),
            UiAktion::SpeichernDialog => {
                if let Some(k) = self.karte.as_deref() {
                    self.speichern_name = Some(speichern::vorschlag(&self.root, k));
                }
            }
            UiAktion::KarteUmbenennen(karte, anzeige, ordner) => {
                if self.karte.as_deref() == Some(karte.as_str()) {
                    self.meldung = "die offene Karte laesst sich nicht umbenennen".into();
                    return;
                }
                let alt_anzeige = self.karten.iter().find(|k| k.ordner == karte).map(|k| k.anzeige.clone()).unwrap_or_default();
                let mut erg = Ok(());
                if anzeige != alt_anzeige {
                    erg = karten::anzeigename_setzen(&self.root, &karte, &anzeige);
                }
                if erg.is_ok() && ordner != karte {
                    erg = karten::umbenennen(&self.root, &karte, &ordner);
                }
                self.meldung = match erg {
                    Ok(()) => format!("Karte umbenannt: {ordner} (\"{anzeige}\")"),
                    Err(e) => format!("Umbenennen fehlgeschlagen: {e:#}"),
                };
                self.karten = karten::finden(&self.root);
                self.kartenwahl = Some(ordner);
            }
            UiAktion::KarteLoeschen(karte) => {
                if self.karte.as_deref() == Some(karte.as_str()) {
                    self.meldung = "die offene Karte laesst sich nicht loeschen".into();
                    return;
                }
                self.meldung = match karten::in_papierkorb(&self.root, &karte) {
                    Ok(w) => format!("Karte {karte} in den Papierkorb verschoben ({} Ordner)", w.len()),
                    Err(e) => format!("Loeschen fehlgeschlagen: {e:#}"),
                };
                self.karten = karten::finden(&self.root);
                self.kartenwahl = None;
            }
            UiAktion::SpeichernHier => {
                let Some(k) = self.karte.clone() else { return };
                if self.speichern_job.is_some() {
                    return;
                }
                if self.bearb.aenderungen + self.strasse.aenderungen + self.aendern.as_ref().map(|a| a.aenderungen).unwrap_or(0) == 0 {
                    self.meldung = "nichts zu speichern".into();
                } else if speichern::eigene_karte(&self.root, &k) {
                    self.ausfuehren(UiAktion::Ueberschreiben);
                } else {
                    self.ueberschreiben_frage = true;
                }
            }
            UiAktion::Ueberschreiben => {
                self.ueberschreiben_frage = false;
                let (Some(v), Some(karte)) = (self.viewer.as_ref(), self.karte.clone()) else { return };
                let kopien = self.aendern.as_ref().map(|a| a.kopien(&karte)).unwrap_or_default();
                let kreuzungen = self.aendern.as_ref().map(|a| a.kreuzungs_ordner());
                match speichern::vorbereiten(v, &self.bearb, &self.strasse.netz, &self.strasse.gesetzte_kreuzungen(), &kopien, kreuzungen, &karte) {
                    Ok(paket) => {
                        let root = self.root.clone();
                        self.meldung = format!("speichere {karte} ({} geaenderte Dateien) ...", paket.dateien.len());
                        protokoll::aktion(&format!("Speichern: {karte} ueberschreiben"));
                        self.speichern_job = Some(std::thread::spawn(move || speichern::karte_ueberschreiben(&root, &karte, &paket).map(|p| (p, true))));
                    }
                    Err(e) => self.meldung = format!("Speichern fehlgeschlagen: {e:#}"),
                }
            }
            UiAktion::Speichern(neu) => {
                self.speichern_name = None;
                let (Some(v), Some(alt)) = (self.viewer.as_ref(), self.karte.clone()) else { return };
                let kopien = self.aendern.as_ref().map(|a| a.kopien(&alt)).unwrap_or_default();
                let kreuzungen = self.aendern.as_ref().map(|a| a.kreuzungs_ordner());
                match speichern::vorbereiten(v, &self.bearb, &self.strasse.netz, &self.strasse.gesetzte_kreuzungen(), &kopien, kreuzungen, &alt) {
                    Ok(paket) => {
                        let root = self.root.clone();
                        self.meldung = format!("speichere {neu} ({} geaenderte Dateien, {} neue Objekte, {} neue Splines) ...", paket.dateien.len(), paket.neue_objekte, paket.neue_splines);
                        self.speichern_job = Some(std::thread::spawn(move || speichern::karte_anlegen(&root, &alt, &neu, &paket).map(|p| (p, false))));
                    }
                    Err(e) => self.meldung = format!("Speichern fehlgeschlagen: {e:#}"),
                }
            }
        }
    }

    /// Sitzungsordner der vorigen Karte abmelden und loeschen (temporaer, enthaelt nur Kopien)
    /// neue Schritte der Werkzeuge in den gemeinsamen Verlauf (die Laengen ihrer Rueckgaengig-Stapel sind gewachsen;
    /// baut das Strassenwerkzeug, gehoeren die gleichzeitig entstandenen Schritte des Aendern-Werkzeugs - aufgeschnittene
    /// Strassen - zu seinem Schritt)
    fn verlauf_pruefen(&mut self) {
        let jetzt = self.stapel_jetzt();
        let (b0, s0, a0) = self.stapel;
        let mut neu = false;
        if jetzt.1 > s0 {
            for _ in s0..jetzt.1 {
                self.verlauf.push(Quelle::Strasse);
            }
            neu = true;
        } else if jetzt.2 > a0 {
            for _ in a0..jetzt.2 {
                self.verlauf.push(Quelle::Aendern);
            }
            neu = true;
        }
        if jetzt.0 > b0 {
            for _ in b0..jetzt.0 {
                self.verlauf.push(Quelle::Objekte);
            }
            neu = true;
        }
        if neu {
            self.verlauf_redo.clear();
        }
        self.stapel = jetzt;
    }

    fn stapel_jetzt(&self) -> (usize, usize, usize) {
        (self.bearb.undo_len(), self.strasse.undo_len(), self.aendern.as_ref().map(|a| a.undo_len()).unwrap_or(0))
    }

    /// letzten Schritt (welches Werkzeug auch immer) zuruecknehmen bzw. wiederholen
    fn verlauf_schritt(&mut self, zurueck: bool) {
        self.verlauf_pruefen();
        let Some(q) = (if zurueck { self.verlauf.pop() } else { self.verlauf_redo.pop() }) else { return };
        let Some(v) = self.viewer.as_mut() else { return };
        let ok = match q {
            Quelle::Objekte => {
                let m = if zurueck { self.bearb.rueckgaengig(v) } else { self.bearb.wiederholen(v) };
                if let Some(m) = m {
                    self.meldung = m;
                }
                true
            }
            Quelle::Strasse => if zurueck { self.strasse.rueckgaengig(v, self.aendern.as_mut()) } else { self.strasse.wiederholen(v, self.aendern.as_mut()) },
            Quelle::Aendern => match self.aendern.as_mut().map(|a| if zurueck { a.rueckgaengig(v) } else { a.wiederholen(v) }) {
                Some(Ok(b)) => b,
                Some(Err(e)) => {
                    self.meldung = format!("{} fehlgeschlagen: {e:#}", if zurueck { "Rueckgaengig" } else { "Wiederholen" });
                    false
                }
                None => false,
            },
        };
        if ok {
            if zurueck { self.verlauf_redo.push(q) } else { self.verlauf.push(q) }
            if q != Quelle::Objekte {
                self.meldung = format!("{} ({})", if zurueck { "rueckgaengig" } else { "wiederholt" },
                                       match q { Quelle::Strasse => "Strasse bauen", Quelle::Aendern => "Aendern", Quelle::Objekte => "Objekte" });
            }
        }
        self.anschluesse.vergessen();
        self.eigene_auswahl.retain(|id| self.strasse.netz.kante(*id).is_some());
        self.stapel = self.stapel_jetzt();
    }

    fn sitzung_schliessen(&mut self) {
        self.kreuzung_wahl = None;
        self.kreuzung_maus = None;
        // Aendern meldet seinen Sitzungsordner beim Verwerfen ab und loescht ihn
        self.aendern = None;
        self.verlauf.clear();
        self.verlauf_redo.clear();
        self.eigene_auswahl.clear();
        self.eigene_maus = None;
        self.aendern_warnung = None;
    }

    /// Tasten des Bearbeitens (nur beim Druecken, nicht wenn egui die Tastatur hat)
    fn taste(&mut self, k: KeyCode) {
        let fein = self.umschalt;
        let akt = match k {
            KeyCode::KeyO if !self.strg => {
                let w = if self.bearb.werkzeug == Werkzeug::Objekte { Werkzeug::Ansehen } else { Werkzeug::Objekte };
                Some(UiAktion::Werkzeug(w))
            }
            KeyCode::KeyP if !self.strg => {
                let w = if self.bearb.werkzeug == Werkzeug::Platzieren { Werkzeug::Ansehen } else { Werkzeug::Platzieren };
                Some(UiAktion::Werkzeug(w))
            }
            KeyCode::Escape if self.platzier.is_some() => Some(UiAktion::Platzier(None)),
            KeyCode::KeyX if !self.strg => {
                let w = if self.bearb.werkzeug == Werkzeug::Kreuzung { Werkzeug::Ansehen } else { Werkzeug::Kreuzung };
                Some(UiAktion::Werkzeug(w))
            }
            KeyCode::Escape if self.bearb.werkzeug == Werkzeug::Kreuzung && self.kreuzung_wahl.is_some() => {
                self.kreuzung_wahl = None;
                None
            }
            KeyCode::KeyU if !self.strg => {
                let w = if self.bearb.werkzeug == Werkzeug::Aendern { Werkzeug::Ansehen } else { Werkzeug::Aendern };
                Some(UiAktion::Werkzeug(w))
            }
            KeyCode::Delete if self.bearb.werkzeug == Werkzeug::Aendern && !self.eigene_auswahl.is_empty() => Some(UiAktion::EigeneAendern(strasse::KantenAenderung::Loeschen)),
            KeyCode::Delete if self.bearb.werkzeug == Werkzeug::Aendern => Some(UiAktion::SplineLoeschen),
            KeyCode::Escape if self.bearb.werkzeug == Werkzeug::Aendern => {
                if let Some(a) = self.aendern.as_mut() {
                    a.auswahl.clear();
                }
                None
            }
            KeyCode::KeyB if !self.strg => {
                let w = if self.bearb.werkzeug == Werkzeug::Strasse { Werkzeug::Ansehen } else { Werkzeug::Strasse };
                Some(UiAktion::Werkzeug(w))
            }
            KeyCode::Escape if self.bearb.werkzeug == Werkzeug::Strasse && self.strasse.baut() => Some(UiAktion::ZugBeenden),
            KeyCode::PageUp if self.bearb.werkzeug == Werkzeug::Strasse => Some(UiAktion::StrassenHoehe(if fein { 0.25 } else { 1.0 })),
            KeyCode::PageDown if self.bearb.werkzeug == Werkzeug::Strasse => Some(UiAktion::StrassenHoehe(if fein { -0.25 } else { -1.0 })),
            KeyCode::Home if self.bearb.werkzeug == Werkzeug::Strasse => Some(UiAktion::StrassenHoehe(-self.strasse.hoehe)),
            KeyCode::KeyG if self.bearb.werkzeug == Werkzeug::Strasse => Some(UiAktion::StrassenModus(strasse::Modus::Gerade)),
            KeyCode::KeyK if self.bearb.werkzeug == Werkzeug::Strasse => Some(UiAktion::StrassenModus(strasse::Modus::Kurve)),
            KeyCode::KeyV if self.bearb.werkzeug == Werkzeug::Strasse => Some(UiAktion::StrassenModus(strasse::Modus::Kreisel)),
            KeyCode::Delete if self.bearb.werkzeug == Werkzeug::Strasse => {
                let (bw, bh) = self.bildgroesse();
                let fang = 12.0 * self.kam.m_pro_px(bh).max(0.02);
                let _ = bw;
                if let (Some(g), Some(v)) = (self.boden_unter_maus, self.viewer.as_mut()) {
                    if let Some(m) = self.strasse.loeschen_bei(v, g.truncate(), fang.max(3.0)) {
                        self.meldung = m;
                    }
                }
                None
            }
            KeyCode::Comma | KeyCode::Period if self.bearb.werkzeug == Werkzeug::Platzieren => {
                let d = if fein { 1.0 } else { 15.0 } * if k == KeyCode::Comma { -1.0 } else { 1.0 };
                self.bearb.platzier_richtung = (self.bearb.platzier_richtung + d).rem_euclid(360.0);
                if let (Some(v), Some(r), Some(g)) = (self.viewer.as_mut(), self.platzier.clone(), self.boden_unter_maus) {
                    self.bearb.geist(v, &r, g);
                }
                None
            }
            KeyCode::KeyZ if self.strg => Some(UiAktion::Rueckgaengig),
            KeyCode::KeyY if self.strg => Some(UiAktion::Wiederholen),
            KeyCode::KeyS if self.strg && self.umschalt => Some(UiAktion::SpeichernDialog),
            KeyCode::KeyS if self.strg => Some(UiAktion::SpeichernHier),
            KeyCode::Escape => Some(UiAktion::Abwaehlen),
            _ if self.bearb.werkzeug != Werkzeug::Objekte => None,
            KeyCode::Delete => Some(UiAktion::Objekt(Action::Delete)),
            KeyCode::Comma => Some(UiAktion::Objekt(Action::Turn(if fein { -0.5 } else { -5.0 }))),
            KeyCode::Period => Some(UiAktion::Objekt(Action::Turn(if fein { 0.5 } else { 5.0 }))),
            KeyCode::PageUp => Some(UiAktion::Objekt(Action::Move(DVec3::Z * if fein { 0.02 } else { 0.1 }))),
            KeyCode::PageDown => Some(UiAktion::Objekt(Action::Move(-DVec3::Z * if fein { 0.02 } else { 0.1 }))),
            _ => None,
        };
        if let Some(a) = akt {
            self.ausfuehren(a);
        }
    }

    /// Bildgroesse in logischen Punkten (wie die Mauskoordinaten)
    fn bildgroesse(&self) -> (f32, f32) {
        match (self.surface.as_ref(), self.window.as_ref()) {
            (Some(s), Some(w)) => (s.config.width as f32 / w.scale_factor() as f32, s.config.height as f32 / w.scale_factor() as f32),
            _ => (1.0, 1.0),
        }
    }
}

/// Maler fuer Markierungen ueber dem 3D-Bild, unter den Panels
fn ui_maler(ui: &egui::Ui) -> egui::Painter {
    ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("auswahl")))
}

impl App {
    /// Kachelraster der Strassen-Querschnitte mit Vorschaubildern und Filtern -> angeklickter Querschnitt
    fn qs_raster(&mut self, ui: &mut egui::Ui, gewaehlt: Option<String>) -> Option<String> {
        let mut geklickt = None;
        let Some(qs) = self.querschnitte.as_ref() else {
            ui.label("Splines werden eingelesen ...");
            return None;
        };
        ui.add(egui::TextEdit::singleline(&mut self.qs_suche).hint_text("suchen (Name, Ordner, Herkunft)").desired_width(f32::INFINITY));
        let mut herkuenfte: Vec<(String, usize)> = Vec::new();
        for q in qs.iter() {
            match herkuenfte.iter_mut().find(|(h, _)| *h == q.herkunft) {
                Some(e) => e.1 += 1,
                None => herkuenfte.push((q.herkunft.clone(), 1)),
            }
        }
        herkuenfte.sort_by_key(|(h, _)| (h != katalog::STANDARD, h == "ohne Karte", h.to_lowercase()));
        let mut ordner: Vec<String> = qs.iter().map(|q| q.ordner.clone()).collect();
        ordner.sort_by_key(|o| o.to_lowercase());
        ordner.dedup();
        egui::Grid::new("qs_filter").num_columns(2).show(ui, |ui| {
            ui.label("Herkunft");
            egui::ComboBox::from_id_salt("qs_herkunft").width(200.0).selected_text(self.qs_herkunft.clone().unwrap_or("alle".into())).show_ui(ui, |ui| {
                ui.selectable_value(&mut self.qs_herkunft, None, format!("alle ({})", qs.len()));
                for (h, n) in &herkuenfte {
                    ui.selectable_value(&mut self.qs_herkunft, Some(h.clone()), format!("{h} ({n})"));
                }
            });
            ui.end_row();
            ui.label("Spuren");
            egui::ComboBox::from_id_salt("qs_spuren").width(200.0).selected_text(self.qs_spuren.map(|s| s.to_string()).unwrap_or("alle".into())).show_ui(ui, |ui| {
                ui.selectable_value(&mut self.qs_spuren, None, "alle");
                for k in strasse::SPURKLASSEN {
                    ui.selectable_value(&mut self.qs_spuren, Some(k), k);
                }
            });
            ui.end_row();
            ui.label("Gehweg");
            egui::ComboBox::from_id_salt("qs_gehweg").width(200.0).selected_text(match self.qs_gehweg { None => "egal", Some(true) => "mit Gehweg", Some(false) => "ohne Gehweg" }).show_ui(ui, |ui| {
                ui.selectable_value(&mut self.qs_gehweg, None, "egal");
                ui.selectable_value(&mut self.qs_gehweg, Some(true), "mit Gehweg");
                ui.selectable_value(&mut self.qs_gehweg, Some(false), "ohne Gehweg");
            });
            ui.end_row();
            ui.label("Ordner");
            egui::ComboBox::from_id_salt("qs_ordner").width(200.0).selected_text(self.qs_ordner.clone().unwrap_or("alle".into())).show_ui(ui, |ui| {
                ui.selectable_value(&mut self.qs_ordner, None, "alle");
                for o in &ordner {
                    ui.selectable_value(&mut self.qs_ordner, Some(o.clone()), o);
                }
            });
            ui.end_row();
        });
        let such = self.qs_suche.to_lowercase();
        let treffer: Vec<&strasse::Querschnitt> = qs.iter()
            .filter(|q| self.qs_herkunft.as_ref().map(|h| &q.herkunft == h).unwrap_or(true))
            .filter(|q| self.qs_ordner.as_ref().map(|o| &q.ordner == o).unwrap_or(true))
            .filter(|q| self.qs_spuren.map(|k| q.spurklasse() == k).unwrap_or(true))
            .filter(|q| self.qs_gehweg.map(|g| (q.gehwege > 0) == g).unwrap_or(true))
            .filter(|q| such.split_whitespace().all(|w| q.name.to_lowercase().contains(w) || q.ordner.to_lowercase().contains(w) || q.herkunft.to_lowercase().contains(w)))
            .collect();
        ui.label(egui::RichText::new(format!("{} von {} Strassen-Splines", treffer.len(), qs.len())).small().weak());
        let bild = vorschau::GROESSE as f32;
        let zelle = bild + 14.0;
        let spalten = ((ui.available_width() / zelle).floor() as usize).max(1);
        let zeilen = treffer.len().div_ceil(spalten);
        let ctx2 = ui.ctx().clone();
        egui::ScrollArea::vertical().auto_shrink(false).max_height(ui.available_height() - 70.0).show_rows(ui, bild + 46.0, zeilen, |ui, bereich| {
            for z in bereich {
                ui.horizontal(|ui| {
                    for q in treffer.iter().skip(z * spalten).take(spalten) {
                        let aktiv = gewaehlt.as_deref() == Some(q.rel.as_str());
                        let textur = self.vorschau.textur(&ctx2, &q.rel);
                        let fehlt = self.vorschau.fehlt(&q.rel);
                        let spuren = if q.zurueck == 0 || q.vor == 0 { format!("{} Einb.", q.vor + q.zurueck) } else { format!("{}+{}", q.vor, q.zurueck) };
                        let antwort = ui.vertical(|ui| {
                            ui.set_width(zelle - 6.0);
                            let (rect, r) = ui.allocate_exact_size(egui::vec2(bild, bild), egui::Sense::click());
                            let rand = if aktiv { egui::Color32::from_rgb(255, 60, 220) } else if r.hovered() { egui::Color32::WHITE } else { egui::Color32::from_gray(70) };
                            ui.painter().rect_filled(rect, 4.0, egui::Color32::from_gray(35));
                            match textur {
                                Some(t) => {
                                    egui::Image::new(&t).corner_radius(4.0).paint_at(ui, rect);
                                }
                                None => {
                                    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, if fehlt { "kein Bild" } else { "..." }, egui::FontId::proportional(11.0), egui::Color32::GRAY);
                                }
                            }
                            ui.painter().rect_stroke(rect, 4.0, egui::Stroke::new(if aktiv { 3.0 } else { 1.0 }, rand), egui::StrokeKind::Inside);
                            let mut name = q.name.clone();
                            if name.chars().count() > 16 {
                                name = name.chars().take(15).collect::<String>() + "…";
                            }
                            ui.label(egui::RichText::new(name).small());
                            ui.label(egui::RichText::new(format!("{spuren} | {:.1} m", q.breite)).small().weak());
                            r
                        }).inner;
                        let antwort = antwort.on_hover_text(format!("{}\n{}\nHerkunft: {}\nFahrspuren {} vor / {} zurueck, {} Gehweg(e), {:.1} m breit",
                                                                   q.name, q.rel, q.herkunft, q.vor, q.zurueck, q.gehwege, q.breite));
                        if antwort.clicked() {
                            geklickt = Some(q.rel.clone());
                        }
                    }
                });
            }
        });
        geklickt
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("OMSI-Editor")
            .with_inner_size(winit::dpi::LogicalSize::new(1500.0, 900.0));
        let window = Arc::new(el.create_window(attrs).expect("Fenster"));
        self.window = Some(window.clone());
        let root = self.root.clone();
        self.katalog_job = Some(std::thread::spawn(move || katalog::einlesen(&root)));
        let root = self.root.clone();
        self.qs_job = Some(std::thread::spawn(move || strasse::querschnitte(&root)));
        window.request_redraw();
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(window) = self.window.clone() else { return };
        let mut egui_will = false;
        if let Some(g) = self.gui.as_mut() {
            let r = g.state.on_window_event(&window, &event);
            egui_will = r.consumed || g.ctx.egui_wants_pointer_input() || g.ctx.is_pointer_over_egui();
        }
        match event {
            WindowEvent::CloseRequested => {
                self.sitzung_schliessen();
                el.exit();
            }
            WindowEvent::Resized(size) => {
                if let (Some(s), Some(v)) = (self.surface.as_mut(), self.viewer.as_ref()) {
                    s.resize(&v.renderer, size.width, size.height);
                }
                if let Some(st) = self.start.as_mut() {
                    st.groesse(size.width, size.height);
                }
            }
            WindowEvent::RedrawRequested => {
                protokoll::puls();
                self.verlauf_pruefen();
                if self.bilder > 120 {
                    match self.pruefung.take().as_deref() {
                        Some("--absturztest") => panic!("Absturztest (--absturztest)"),
                        Some("--haengertest") => {
                            protokoll::aktion("Haengertest (--haengertest): 12 s warten");
                            std::thread::sleep(std::time::Duration::from_secs(12));
                            protokoll::aktion("");
                        }
                        _ => {}
                    }
                }
                self.zeichnen();
                if self.viewer.is_some() {
                    self.bilder += 1;
                }
                if let Some(t) = self.beenden_nach {
                    let s = self.gestartet.elapsed().as_secs_f32();
                    if s > t {
                        if let Some(k) = self.katalog.as_ref() {
                            if let Some(q) = self.querschnitte.as_ref() {
                                let mut h: std::collections::BTreeMap<&str, usize> = Default::default();
                                for x in q {
                                    *h.entry(x.herkunft.as_str()).or_default() += 1;
                                }
                                println!("Testlauf Strassen: {} Querschnitte, Herkunft {:?}", q.len(), h);
                            }
                            println!("Testlauf Katalog: {} Vorschaubilder erzeugt, Herkuenfte: {}", self.vorschau.erzeugt,
                                     k.herkuenfte.iter().map(|(h, n)| format!("{h} {n}")).collect::<Vec<_>>().join(", "));
                        }
                        println!("Testlauf: {} Bilder in {:.1} s, zuletzt {:.0} fps, laengstes Bild {:.0} ms, {} Kacheln, {} | {}", self.bilder, s, self.fps, self.laengstes * 1000.0, self.viewer.as_ref().map(|v| v.loaded_tiles()).unwrap_or(0),
                                 self.karte.as_deref().unwrap_or("-"), self.meldung);
                        self.sitzung_schliessen();
                        el.exit();
                    } else if s > t * 0.6 && self.wechsel.is_some() {
                        let ziel = self.wechsel.take().unwrap();
                        if let Some(i) = self.karten.iter().position(|k| k.ordner == ziel) {
                            self.zu_laden = Some(i);
                            self.laengstes = 0.0;
                        }
                    } else if s > t * 0.3 && self.bearb.werkzeug == Werkzeug::Ansehen && self.wechsel.is_none() && self.bearb.aenderungen == 0
                        && self.viewer.as_ref().map(|v| v.first_area_progress().is_none() && v.loaded_tiles() > 0).unwrap_or(false) {
                        // Bearbeiten ausprobieren: Objekt in der Bildmitte waehlen und drehen
                        self.ausfuehren(UiAktion::Werkzeug(Werkzeug::Objekte));
                        let (bw, bh) = self.bildgroesse();
                        let mut gefunden = None;
                        if let Some(v) = self.viewer.as_ref() {
                            for k in 0..40 {
                                let r = k as f32 * 12.0;
                                let p = (bw / 2.0 + r * (k as f32).cos(), bh / 2.0 + r * (k as f32).sin());
                                if let Some(id) = self.bearb.suchen(v, &self.kam, p, bw, bh) {
                                    gefunden = Some(id);
                                    break;
                                }
                            }
                        }
                        self.bearb.waehlen(gefunden);
                        self.ausfuehren(UiAktion::Objekt(Action::Turn(30.0)));
                        self.ausfuehren(UiAktion::Rueckgaengig);
                        self.ausfuehren(UiAktion::Wiederholen);
                        self.ausfuehren(UiAktion::Objekt(Action::Copy));
                        let kopie = self.bearb.wahl;
                        // Platzieren aus dem Katalog an den Blickpunkt
                        let eintrag = self.katalog.as_ref().and_then(|k| {
                            k.eintraege.iter().find(|e| e.name.to_lowercase().contains("bank")).or(k.eintraege.first()).map(|e| e.rel.clone())
                        });
                        self.ausfuehren(UiAktion::Werkzeug(Werkzeug::Platzieren));
                        self.ausfuehren(UiAktion::Platzier(eintrag.clone()));
                        let ziel = self.kam.ziel;
                        let platziert = match (eintrag.as_ref(), self.viewer.as_mut()) {
                            (Some(rel), Some(v)) => self.bearb.platzieren(v, rel, ziel),
                            _ => None,
                        };
                        // Strasse bauen: Gerade, dann Kurve
                        self.ausfuehren(UiAktion::Werkzeug(Werkzeug::Strasse));
                        let z = self.kam.ziel;
                        let punkte = [z, z + DVec3::new(0.0, 60.0, 0.0), z + DVec3::new(50.0, 110.0, 0.0)];
                        let mut meldungen = Vec::new();
                        if let Some(v) = self.viewer.as_mut() {
                            for (i, p) in punkte.iter().enumerate() {
                                let g = DVec3::new(p.x, p.y, v.terrain_height(p.x, p.y).unwrap_or(p.z));
                                self.strasse.modus = if i < 2 { strasse::Modus::Gerade } else { strasse::Modus::Kurve };
                                self.strasse.maus(v, g, 3.0, &self.anschluesse, self.aendern.as_mut());
                                meldungen.push(self.strasse.klick(v, g, 3.0, &self.anschluesse, self.aendern.as_mut()).unwrap_or_default());
                            }
                            self.strasse.beenden(v);
                        }
                        // Aendern: vorhandene Strasse nahe dem Blickpunkt, Upgrade, umkehren, rueckgaengig
                        self.ausfuehren(UiAktion::Werkzeug(Werkzeug::Aendern));
                        let mut gefunden = None;
                        if let (Some(v), Some(a)) = (self.viewer.as_ref(), self.aendern.as_mut()) {
                            a.aktualisieren(v);
                            for k in 0..200 {
                                let w = k as f64 * 0.7;
                                let p = z.truncate() + glam::DVec2::new(w.cos(), w.sin()) * (k as f64 * 1.5);
                                if let Some(id) = a.suchen(v, p) {
                                    gefunden = Some(id);
                                    a.auswahl = vec![id];
                                    break;
                                }
                            }
                        }
                        if gefunden.is_some() {
                            self.ausfuehren(UiAktion::SplineQuerschnitt("Splines\\Marcel\\str_2spur_10m_Grunewaldstr.sli".into()));
                            let m1 = self.meldung.clone();
                            self.ausfuehren(UiAktion::SplineSpiegeln);
                            self.ausfuehren(UiAktion::Rueckgaengig);
                            println!("Testlauf Aendern: Spline {:?} | {} | {}", gefunden, m1, self.meldung);
                            self.testlauf_abzweig = gefunden;
                        } else {
                            println!("Testlauf Aendern: keine Strasse am Blickpunkt");
                        }
                        println!("Testlauf Strasse: {} Knoten, {} Kanten | {}", self.strasse.netz.knoten.len(), self.strasse.netz.kanten.len(), meldungen.join(" | "));
                        println!("Testlauf Kopie: {:?}, Katalog {} Objekte, platziert: {:?}",
                                 kopie, self.katalog.as_ref().map(|k| k.eintraege.len()).unwrap_or(0), platziert);
                        println!("Testlauf Bearbeiten: {} Objekte, Objekt {:?} gewaehlt, Aenderungen {}, Meldung: {}",
                                 self.viewer.as_ref().map(|v| v.objects().len()).unwrap_or(0), gefunden, self.bearb.aenderungen, self.meldung);
                    } else if s > t * 0.36 && self.testlauf_abzweig.is_some() {
                        let id = self.testlauf_abzweig.take().unwrap();
                        // Abzweig mitten aus dieser Strasse (Kreuzung), dann rueckgaengig
                        self.ausfuehren(UiAktion::Werkzeug(Werkzeug::Strasse));
                        let mut m = Vec::new();
                        if let (Some(v), Some(a)) = (self.viewer.as_mut(), self.aendern.as_mut()) {
                            a.aktualisieren(v);
                            // die laengste Strasse in der Naehe (sonst die vom Aendern-Test)
                            let z = self.kam.ziel.truncate();
                            let lang = a.kacheln.values().flatten()
                                .filter(|s| (s.kurve.start.truncate() - z).length() < 400.0 && v.spline_end_free(s.id, true).is_some())
                                .max_by(|x, y| x.kurve.length.total_cmp(&y.kurve.length)).map(|s| s.id).unwrap_or(id);
                            let mitte = a.spline(lang).map(|s| s.kurve.point_at(s.kurve.length / 2.0).truncate());
                            if let Some(ab) = mitte.and_then(|p| a.abzweig_bei(v, p)) {
                                m.push(self.strasse.klick(v, ab.pos, 2.0, &self.anschluesse, Some(&mut *a)).unwrap_or_default());
                                let q = ab.pos.truncate() + crate::netz::rechts(ab.richtung) * 40.0;
                                let g = q.extend(v.terrain_height(q.x, q.y).unwrap_or(ab.pos.z));
                                self.strasse.maus(v, g, 2.0, &self.anschluesse, Some(&mut *a));
                                m.push(self.strasse.klick(v, g, 2.0, &self.anschluesse, Some(&mut *a)).unwrap_or_default());
                                self.strasse.beenden(v);
                            }
                        }
                        self.anschluesse.vergessen();
                        // gemeinsamer Verlauf: im Werkzeug "Aendern" nimmt Strg+Z die gebaute Strasse samt Kreuzung zurueck
                        self.ausfuehren(UiAktion::Werkzeug(Werkzeug::Aendern));
                        let vorher = (self.strasse.netz.kanten.len(), self.strasse.gesetzte_kreuzungen().len());
                        self.ausfuehren(UiAktion::Rueckgaengig);
                        let nachher = (self.strasse.netz.kanten.len(), self.strasse.gesetzte_kreuzungen().len());
                        println!("Testlauf Kreuzung: {} | im Aendern-Werkzeug rueckgaengig: {} | Kanten/Kreuzungen {:?} -> {:?}", m.join(" | "), self.meldung, vorher, nachher);
                        // die restlichen Bilder mit dem Werkzeug "Kreuzungen" (Panel, Markierungen)
                        self.ausfuehren(UiAktion::Werkzeug(Werkzeug::Kreuzung));
                        self.kreuzung_wahl = self.strasse.kreuzungen().first().map(|k| k.knoten);
                    } else if s > t * 0.4 {
                        // Kamera bewegen wie ein Nutzer: drehen, fahren, zoomen
                        self.kam.drehen(0.4, 0.0);
                        self.kam.verschieben(0.0, 2.5);
                    }
                }
                window.request_redraw();
            }
            WindowEvent::ModifiersChanged(m) => {
                self.strg = m.state().control_key();
                self.umschalt = m.state().shift_key();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let tastatur_bei_egui = self.gui.as_ref().map(|g| g.ctx.egui_wants_keyboard_input()).unwrap_or(false);
                if let PhysicalKey::Code(k) = event.physical_key {
                    if event.state == ElementState::Pressed && !tastatur_bei_egui {
                        if !event.repeat || matches!(k, KeyCode::Comma | KeyCode::Period | KeyCode::PageUp | KeyCode::PageDown) {
                            self.taste(k);
                        }
                        self.tasten.insert(k);
                    } else if event.state == ElementState::Released {
                        self.tasten.remove(&k);
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                // logische Punkte wie egui und die Projektion (winit liefert physische Pixel)
                let sf = window.scale_factor();
                let p = ((position.x / sf) as f32, (position.y / sf) as f32);
                if let (Some((knopf, alt)), Some(s)) = (self.ziehen, self.surface.as_ref()) {
                    let (dx, dy) = (p.0 - alt.0, p.1 - alt.1);
                    match knopf {
                        MouseButton::Right => self.kam.drehen(dx * 0.25, -dy * 0.25),
                        _ => {
                            let _ = s;
                            let f = self.kam.m_pro_px(self.bildgroesse().1);
                            let neig = (-self.kam.neigung as f64).to_radians().sin().max(0.25);
                            self.kam.verschieben(-dx as f64 * f, dy as f64 * f / neig);
                        }
                    }
                    self.ziehen = Some((knopf, p));
                }
                self.maus = Some(p);
                if self.surface.is_some() {
                    let (bw, bh) = self.bildgroesse();
                    let (o, d) = self.kam.strahl(p.0, p.1, bw, bh);
                    self.boden_unter_maus = treffer(o, d, |x, y| self.boden(x, y), 6000.0);
                    if let (Werkzeug::Platzieren, Some(rel), Some(g)) = (self.bearb.werkzeug, self.platzier.clone(), self.boden_unter_maus) {
                        if let Some(v) = self.viewer.as_mut() {
                            if !egui_will {
                                self.bearb.geist(v, &rel, g);
                            } else {
                                self.bearb.geist_weg(v);
                            }
                        }
                    }
                    if self.bearb.werkzeug == Werkzeug::Kreuzung && !egui_will {
                        self.kreuzung_maus = None;
                        if let Some(g) = self.boden_unter_maus {
                            if let Some(k) = self.strasse.kreuzung_bei(g.truncate()) {
                                self.kreuzung_maus = Some(KreuzungsZiel::Netz(k));
                            } else if let (Some(v), Some(a)) = (self.viewer.as_ref(), self.aendern.as_mut()) {
                                self.kreuzung_maus = a.kreuzungsobjekt_bei(v, g.truncate()).filter(|k| k.arme.len() >= 3).map(KreuzungsZiel::Vorhanden);
                            }
                        }
                    }
                    if self.bearb.werkzeug == Werkzeug::Aendern && !egui_will {
                        // eigene Strassen liegen ueber den vorhandenen: zuerst
                        self.eigene_maus = self.boden_unter_maus.and_then(|g| self.strasse.kante_unter(g.truncate()));
                        if let (Some(g), Some(v), Some(a)) = (self.boden_unter_maus, self.viewer.as_ref(), self.aendern.as_mut()) {
                            a.unter_maus = if self.eigene_maus.is_some() { None } else { a.suchen(v, g.truncate()) };
                        }
                    }
                    if self.bearb.werkzeug == Werkzeug::Strasse {
                        let fang = 12.0 * self.kam.m_pro_px(bh);
                        if let (Some(g), Some(v)) = (self.boden_unter_maus, self.viewer.as_mut()) {
                            self.strasse.maus(v, g, fang.max(2.0), &self.anschluesse, self.aendern.as_mut());
                        }
                    }
                    if self.bearb.werkzeug == Werkzeug::Objekte {
                        if self.bearb.zieht() {
                            if let (Some(g), Some(v)) = (self.boden_unter_maus, self.viewer.as_mut()) {
                                if let Some(m) = self.bearb.ziehen_nach(v, g) {
                                    self.meldung = m;
                                }
                            }
                        } else if !egui_will {
                            self.bearb.unter_maus = self.viewer.as_ref().and_then(|v| self.bearb.suchen(v, &self.kam, p, bw, bh));
                        }
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if state == ElementState::Pressed && !egui_will {
                    if matches!(button, MouseButton::Right | MouseButton::Middle) {
                        if let Some(p) = self.maus {
                            self.ziehen = Some((button, p));
                        }
                    }
                    if button == MouseButton::Right {
                        self.rechts_start = self.maus;
                    }
                    if button == MouseButton::Left && self.bearb.werkzeug == Werkzeug::Kreuzung {
                        match self.kreuzung_maus.clone() {
                            Some(KreuzungsZiel::Netz(k)) => self.kreuzung_wahl = Some(k),
                            Some(KreuzungsZiel::Vorhanden(k)) => {
                                if let (Some(v), Some(a)) = (self.viewer.as_mut(), self.aendern.as_mut()) {
                                    match self.strasse.vorhandene_uebernehmen(v, a, &k) {
                                        Ok(id) => {
                                            self.kreuzung_wahl = Some(id);
                                            self.meldung = format!("vorhandene Kreuzung uebernommen ({} Arme) - Vorfahrt und Ampel rechts einstellen", k.arme.len());
                                        }
                                        Err(e) => self.meldung = format!("Kreuzung nicht uebernommen: {e}"),
                                    }
                                }
                                self.anschluesse.vergessen();
                                self.kreuzung_maus = None;
                            }
                            None => self.kreuzung_wahl = None,
                        }
                    }
                    if button == MouseButton::Left && self.bearb.werkzeug == Werkzeug::Aendern && self.eigene_maus.is_some() {
                        let id = self.eigene_maus.unwrap();
                        if self.umschalt {
                            self.eigene_auswahl = self.strasse.kette_eigen(id);
                        } else if self.strg {
                            if let Some(p) = self.eigene_auswahl.iter().position(|x| *x == id) {
                                self.eigene_auswahl.remove(p);
                            } else {
                                self.eigene_auswahl.push(id);
                            }
                        } else {
                            self.eigene_auswahl = vec![id];
                        }
                        if let Some(a) = self.aendern.as_mut() {
                            a.auswahl.clear();
                        }
                    } else if button == MouseButton::Left && self.bearb.werkzeug == Werkzeug::Aendern {
                        self.eigene_auswahl.clear();
                        let umschalt = self.umschalt;
                        if let Some(a) = self.aendern.as_mut() {
                            match a.unter_maus {
                                Some(id) if umschalt => a.auswahl = a.kette(id),
                                Some(id) if self.strg => {
                                    if let Some(p) = a.auswahl.iter().position(|x| *x == id) {
                                        a.auswahl.remove(p);
                                    } else {
                                        a.auswahl.push(id);
                                    }
                                }
                                Some(id) => a.auswahl = vec![id],
                                None => a.auswahl.clear(),
                            }
                            self.aendern_warnung = None;
                        }
                    }
                    if button == MouseButton::Left && self.bearb.werkzeug == Werkzeug::Strasse {
                        let (_, bh) = self.bildgroesse();
                        let fang = (12.0 * self.kam.m_pro_px(bh)).max(2.0);
                        if let (Some(g), Some(v)) = (self.boden_unter_maus, self.viewer.as_mut()) {
                            let vorher = self.aendern.as_ref().map(|a| a.aenderungen);
                            match self.strasse.klick(v, g, fang, &self.anschluesse, self.aendern.as_mut()) {
                                Some(m) => self.meldung = m,
                                None if self.strasse.sli.is_none() => self.meldung = "erst einen Querschnitt waehlen".into(),
                                None => {}
                            }
                            // eine Kreuzung hat Kacheln neu geladen: Strassenenden neu bestimmen
                            if self.aendern.as_ref().map(|a| a.aenderungen) != vorher {
                                self.anschluesse.vergessen();
                            }
                            self.strasse.maus(v, g, fang, &self.anschluesse, self.aendern.as_mut());
                        }
                    }
                    if button == MouseButton::Left && self.bearb.werkzeug == Werkzeug::Platzieren {
                        if let (Some(rel), Some(g), Some(v)) = (self.platzier.clone(), self.boden_unter_maus, self.viewer.as_mut()) {
                            self.meldung = self.bearb.platzieren(v, &rel, g).unwrap_or_else(|| format!("{rel} laesst sich nicht laden"));
                        }
                    }
                    if button == MouseButton::Left && self.bearb.werkzeug == Werkzeug::Objekte {
                        let (bw, bh) = self.bildgroesse();
                        let treffer_id = match (self.viewer.as_ref(), self.maus) {
                            (Some(v), Some(p)) => self.bearb.suchen(v, &self.kam, p, bw, bh),
                            _ => None,
                        };
                        self.bearb.waehlen(treffer_id);
                        if let (Some(id), Some(g), Some(v)) = (treffer_id, self.boden_unter_maus, self.viewer.as_ref()) {
                            self.bearb.greifen(v, id, g);
                        }
                    }
                } else if state == ElementState::Released {
                    if self.ziehen.map(|z| z.0) == Some(button) {
                        self.ziehen = None;
                    }
                    if button == MouseButton::Left {
                        if let Some(v) = self.viewer.as_ref() {
                            self.bearb.loslassen(v);
                        }
                    }
                    if button == MouseButton::Right && self.bearb.werkzeug == Werkzeug::Strasse {
                        let ohne_ziehen = match (self.rechts_start, self.maus) {
                            (Some(a), Some(b)) => (a.0 - b.0).abs() + (a.1 - b.1).abs() < 5.0,
                            _ => false,
                        };
                        if ohne_ziehen && self.strasse.baut() {
                            self.ausfuehren(UiAktion::ZugBeenden);
                        }
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } if !egui_will && self.strg && self.bearb.wahl.is_some() => {
                let y = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 60.0,
                };
                let schritt = if self.umschalt { 0.5 } else { 5.0 };
                self.ausfuehren(UiAktion::Objekt(Action::Turn(-(y as f64) * schritt)));
            }
            WindowEvent::MouseWheel { delta, .. } if !egui_will => {
                let y = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y * 60.0,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32,
                };
                let alt = self.kam.abstand;
                self.kam.abstand = (alt * 0.9985f64.powf(y as f64)).clamp(5.0, 6000.0);
                // zum Mauspunkt hin zoomen
                if let Some(p) = self.boden_unter_maus {
                    let k = 1.0 - self.kam.abstand / alt;
                    self.kam.ziel.x += (p.x - self.kam.ziel.x) * k;
                    self.kam.ziel.y += (p.y - self.kam.ziel.y) * k;
                }
            }
            _ => {}
        }
    }
}

/// Bild ohne Fenster: Karte laden, Kacheln um den Blickpunkt, ein Bild (zum Pruefen und fuer Tests)
fn bild(root: &Path, karte: &str, png: &Path, cam: Option<&str>) -> Result<()> {
    let t0 = Instant::now();
    let instance = viewer::instance();
    let global = root.join("maps").join(karte).join("global.cfg");
    let (mut v, start) = Viewer::open(&instance, None, root, &global)?;
    let mut k = Kamera::default();
    if let Some(c) = cam {
        let w: Vec<f64> = c.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        anyhow::ensure!(w.len() == 5, "--cam x,y,gier,neigung,abstand");
        k = Kamera { ziel: DVec3::new(w[0], w[1], 0.0), gier: w[2] as f32, neigung: w[3] as f32, abstand: w[4], fov: 50.0 };
    } else {
        let f = start.forward().as_dvec3();
        k.ziel = start.position + f * 150.0;
        k.gier = start.yaw;
    }
    let n = v.tiles_around(k.ziel, KACHEL_RADIUS)?;
    k.ziel.z = v.ground_height(k.ziel.x, k.ziel.y).unwrap_or(0.0);
    let t1 = Instant::now();
    let (w, h) = (1600u32, 900u32);
    let px = v.render_image(w, h, &k.camera())?;
    let t2 = Instant::now();
    // zweites Bild: so lange braucht ein Bild im Betrieb
    let _ = v.render_image(w, h, &k.camera())?;
    let t3 = Instant::now();
    write_png(png, w, h, &px)?;
    println!("{karte}: {n} Kacheln geladen in {:.1} s, erstes Bild {:.0} ms, weiteres Bild {:.0} ms -> {}",
             (t1 - t0).as_secs_f32(), (t2 - t1).as_secs_f32() * 1000.0, (t3 - t2).as_secs_f32() * 1000.0, png.display());
    Ok(())
}

/// RGBA -> PNG ohne weitere Abhaengigkeit (unkomprimiert, zlib "stored")
fn write_png(path: &Path, w: u32, h: u32, rgba: &[u8]) -> Result<()> {
    fn crc(data: &[u8]) -> u32 {
        let mut c = 0xffff_ffffu32;
        for &b in data {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
    fn chunk(out: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc(&body).to_be_bytes());
    }
    let mut raw = Vec::with_capacity((w * 4 + 1) as usize * h as usize);
    for y in 0..h as usize {
        raw.push(0);
        raw.extend_from_slice(&rgba[y * w as usize * 4..(y + 1) * w as usize * 4]);
    }
    let mut z = vec![0x78, 0x01];
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    for (i, block) in raw.chunks(65535).enumerate() {
        let last = (i + 1) * 65535 >= raw.len();
        z.push(last as u8);
        z.extend_from_slice(&(block.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        z.extend_from_slice(block);
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    std::fs::write(path, out)?;
    Ok(())
}

fn main() -> Result<()> {
    protokoll::starten();
    // "--name=wert" wie "--name wert" (negative Zahlen in --cam sehen sonst wie Schalter aus)
    let args: Vec<String> = std::env::args()
        .skip(1)
        .flat_map(|a| match a.split_once('=') {
            Some((k, w)) if a.starts_with("--") => vec![k.to_string(), w.to_string()],
            _ => vec![a],
        })
        .collect();
    let mut root = PathBuf::from(STANDARD_OMSI);
    let mut karte = None;
    let (mut png, mut cam, mut testlauf, mut wechsel) = (None, None, None::<f32>, None);
    let mut pruefung = None;
    let mut i = 0;
    while i < args.len() {
        let wert = args.get(i + 1).cloned();
        if args[i] == "--absturztest" || args[i] == "--haengertest" {
            pruefung = Some(args[i].clone());
            i += 1;
            continue;
        }
        match (args[i].as_str(), wert) {
            ("--root", Some(w)) => root = PathBuf::from(w),
            ("--bild", Some(w)) => png = Some(PathBuf::from(w)),
            ("--cam", Some(w)) => cam = Some(w),
            ("--testlauf", Some(w)) => testlauf = w.parse().ok(),
            ("--wechsel", Some(w)) => wechsel = Some(w),
            _ => {
                karte = Some(args[i].clone());
                i += 1;
                continue;
            }
        }
        i += 2;
    }
    if let Some(png) = png {
        return bild(&root, karte.as_deref().context("Kartenordner angeben")?, &png, cam.as_deref());
    }
    speichern::aufraeumen();
    let el = EventLoop::new()?;
    let mut app = App::new(root, karte);
    app.beenden_nach = testlauf;
    app.wechsel = wechsel;
    app.pruefung = pruefung;
    el.run_app(&mut app)?;
    Ok(())
}
