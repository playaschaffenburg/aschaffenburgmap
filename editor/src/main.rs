//! OMSI-Editor: moderner Karteneditor fuer OMSI 2 auf Basis von openOMSI (Darstellung wie im Spiel).
//!
//! Meilenstein 1 "Betrachter": beliebige OMSI-Karte oeffnen, Kamera wie in Transport Fever,
//! Kacheln werden um den Blickpunkt nachgeladen.
//!
//!   omsi-editor [--root <OMSI-2-Ordner>] [Kartenordner]
//!
//! Bedienung: rechte Maustaste drehen/neigen, mittlere verschieben, Mausrad zoomen,
//! W A S D / Pfeile verschieben, Q / E drehen, R / F neigen.

mod kamera;

use anyhow::{Context, Result};
use glam::DVec3;
use kamera::{treffer, Kamera};
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

struct Karte {
    ordner: String,
    global: PathBuf,
}

fn karten_finden(root: &Path) -> Vec<Karte> {
    let mut v = Vec::new();
    if let Ok(rd) = std::fs::read_dir(root.join("maps")) {
        for e in rd.flatten() {
            let g = e.path().join("global.cfg");
            if g.is_file() {
                v.push(Karte { ordner: e.file_name().to_string_lossy().into_owned(), global: g });
            }
        }
    }
    v.sort_by_key(|k| k.ordner.to_lowercase());
    v
}

struct Gui {
    ctx: egui::Context,
    state: egui_winit::State,
    renderer: egui_wgpu::Renderer,
}

struct App {
    root: PathBuf,
    karten: Vec<Karte>,
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
    /// --testlauf: nach so vielen Sekunden beenden und Bildrate ausgeben
    beenden_nach: Option<f32>,
    gestartet: Instant,
    bilder: u32,
    /// laengstes Bild (s) nach dem ersten Ladebereich, fuer den Testlauf
    laengstes: f32,
    /// --wechsel: im Testlauf nach 60 % auf diese Karte umschalten
    wechsel: Option<String>,
}

impl App {
    fn new(root: PathBuf, start_karte: Option<String>) -> Self {
        let karten = karten_finden(&root);
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
            gestartet: Instant::now(),
            bilder: 0,
            laengstes: 0.0,
            wechsel: None,
        }
    }

    /// Karte oeffnen: neuer Renderer und neue Szene (openOMSI-Welt)
    fn karte_oeffnen(&mut self, i: usize) -> Result<()> {
        let window = self.window.clone().context("kein Fenster")?;
        let t0 = Instant::now();
        // schon eine Karte offen: Renderer behalten (Pipelines nur einmal kompilieren), Welt tauschen
        if let Some(v) = self.viewer.as_mut() {
            let cam = v.open_map(&self.karten[i].global)?;
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
        // erste Karte: Grafik starten (Pipelines kompilieren), Flaeche anlegen
        self.surface = None;
        self.viewer = None;
        self.gui = None;
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
        let t = |k: KeyCode| self.tasten.contains(&k);
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
        if let Some(i) = self.zu_laden.take() {
            if let Err(e) = self.karte_oeffnen(i) {
                self.meldung = format!("Karte nicht geladen: {e:#}");
            }
        }
        if self.surface.is_none() {
            // noch keine Karte: nur die Oberflaeche ohne 3D (Flaeche ueber einen leeren Viewer
            // gibt es erst mit der Karte) - die erste Karte waehlen
            if self.zu_laden.is_none() && !self.karten.is_empty() {
                let i = self.start_karte.as_ref().and_then(|s| self.karten.iter().position(|k| &k.ordner == s)).unwrap_or(0);
                self.start_karte = None;
                self.zu_laden = Some(i);
                window.request_redraw();
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
        let mut waehle: Option<usize> = None;
        let out = gui.ctx.run_ui(input, |ctx| {
            egui::Panel::top("werkzeuge").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.strong("OMSI-Editor");
                    ui.separator();
                    let _ = ui.selectable_label(true, "Ansehen");
                    ui.add_enabled(false, egui::Button::new("Strasse bauen"));
                    ui.add_enabled(false, egui::Button::new("Objekte"));
                    ui.add_enabled(false, egui::Button::new("Gelaende"));
                    ui.separator();
                    ui.label(egui::RichText::new("Rechte Maus drehen, mittlere verschieben, Rad zoomen, WASD").weak());
                });
            });
            egui::Panel::left("karten").default_size(230.0).show(ctx, |ui| {
                ui.heading("Karten");
                ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("suchen"));
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let f = self.filter.to_lowercase();
                    for (i, k) in self.karten.iter().enumerate() {
                        if !f.is_empty() && !k.ordner.to_lowercase().contains(&f) {
                            continue;
                        }
                        let aktiv = self.karte.as_deref() == Some(k.ordner.as_str());
                        if ui.selectable_label(aktiv, &k.ordner).clicked() && !aktiv {
                            waehle = Some(i);
                        }
                    }
                });
            });
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
        });
        gui.state.handle_platform_output(window, out.platform_output);
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
        if let Some(i) = waehle {
            self.zu_laden = Some(i);
            self.meldung = format!("lade {} ...", self.karten[i].ordner);
        }
        self.gui = Some(gui);
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
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(size) => {
                if let (Some(s), Some(v)) = (self.surface.as_mut(), self.viewer.as_ref()) {
                    s.resize(&v.renderer, size.width, size.height);
                }
            }
            WindowEvent::RedrawRequested => {
                self.zeichnen();
                if self.viewer.is_some() {
                    self.bilder += 1;
                }
                if let Some(t) = self.beenden_nach {
                    let s = self.gestartet.elapsed().as_secs_f32();
                    if s > t {
                        println!("Testlauf: {} Bilder in {:.1} s, zuletzt {:.0} fps, laengstes Bild {:.0} ms, {} Kacheln, {} | {}", self.bilder, s, self.fps, self.laengstes * 1000.0, self.viewer.as_ref().map(|v| v.loaded_tiles()).unwrap_or(0),
                                 self.karte.as_deref().unwrap_or("-"), self.meldung);
                        el.exit();
                    } else if s > t * 0.6 && self.wechsel.is_some() {
                        let ziel = self.wechsel.take().unwrap();
                        if let Some(i) = self.karten.iter().position(|k| k.ordner == ziel) {
                            self.zu_laden = Some(i);
                            self.laengstes = 0.0;
                        }
                    } else if s > t * 0.4 {
                        // Kamera bewegen wie ein Nutzer: drehen, fahren, zoomen
                        self.kam.drehen(0.4, 0.0);
                        self.kam.verschieben(0.0, 2.5);
                    }
                }
                window.request_redraw();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let tastatur_bei_egui = self.gui.as_ref().map(|g| g.ctx.egui_wants_keyboard_input()).unwrap_or(false);
                if let PhysicalKey::Code(k) = event.physical_key {
                    if event.state == ElementState::Pressed && !tastatur_bei_egui {
                        self.tasten.insert(k);
                    } else if event.state == ElementState::Released {
                        self.tasten.remove(&k);
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let p = (position.x as f32, position.y as f32);
                if let (Some((knopf, alt)), Some(s)) = (self.ziehen, self.surface.as_ref()) {
                    let (dx, dy) = (p.0 - alt.0, p.1 - alt.1);
                    match knopf {
                        MouseButton::Right => self.kam.drehen(dx * 0.25, -dy * 0.25),
                        _ => {
                            let f = self.kam.m_pro_px(s.config.height as f32);
                            let neig = (-self.kam.neigung as f64).to_radians().sin().max(0.25);
                            self.kam.verschieben(-dx as f64 * f, dy as f64 * f / neig);
                        }
                    }
                    self.ziehen = Some((knopf, p));
                }
                self.maus = Some(p);
                if let Some(s) = self.surface.as_ref() {
                    let (o, d) = self.kam.strahl(p.0, p.1, s.config.width as f32, s.config.height as f32);
                    self.boden_unter_maus = treffer(o, d, |x, y| self.boden(x, y), 6000.0);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if state == ElementState::Pressed && !egui_will {
                    if matches!(button, MouseButton::Right | MouseButton::Middle) {
                        if let Some(p) = self.maus {
                            self.ziehen = Some((button, p));
                        }
                    }
                } else if state == ElementState::Released {
                    if self.ziehen.map(|z| z.0) == Some(button) {
                        self.ziehen = None;
                    }
                }
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
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
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
    let mut i = 0;
    while i < args.len() {
        let wert = args.get(i + 1).cloned();
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
    let el = EventLoop::new()?;
    let mut app = App::new(root, karte);
    app.beenden_nach = testlauf;
    app.wechsel = wechsel;
    el.run_app(&mut app)?;
    Ok(())
}
