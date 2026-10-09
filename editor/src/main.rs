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
mod baukasten;
mod bearbeiten;
mod gelaende;
mod geo;
mod hilfsansicht;
mod kamera;
mod karten;
mod katalog;
mod knoten;
mod kreuzung;
mod luftbild;
mod netz;
mod orte;
mod protokoll;
mod querschnitt;
mod speichern;
mod spuren;
mod strasse;
mod vorschau;
mod welt;

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
    /// die geoeffnete Karte nutzt Objekte aus Ordnern anderer Karten: (Anzahl, davon fehlend)
    FremdeObjekte(usize, usize),
    /// neue Karte: Angaben, Ordner von Hand geaendert (sonst aus dem Anzeigenamen)
    Neu(karten::NeueKarte, bool),
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
    stapel: (usize, usize, usize, usize, usize, usize),
    /// Bericht der vorigen Sitzung, einmal in der Statuszeile melden
    bericht_melden: Option<Option<PathBuf>>,
    /// Testlauf: Strasse, an der spaeter eine Kreuzung gebaut wird
    testlauf_abzweig: Option<i64>,
    /// Testlauf: Strasse, an der spaeter ein Knoten gezogen wird
    testlauf_knoten: Option<i64>,
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
    /// nur sichtbare (false) bzw. nur Editor-Objekte (true)
    katalog_editor: Option<bool>,
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
    /// Werkzeug "Knoten": Verlauf an Knoten ziehen
    knoten: knoten::Knotenwerkzeug,
    /// Hilfsansicht (H): Pfade und unsichtbare Objekte
    hilfe: hilfsansicht::Hilfsansicht,
    /// Werkzeug "Kreuzungen", Ansicht Spuren (Spurpfeile/Verbinder): an, gewaehlte Kreuzung, gewaehlte Zufahrtsspur
    spur_modus: bool,
    spur_ziel: Option<spuren::Ziel>,
    spur_zufahrt: Option<usize>,
    /// Pipette (Taste I, Knopf oben): der naechste Klick nimmt, was unter der Maus liegt - Objekt/Baum (Platzieren),
    /// Strasse (Strasse bauen mit ihrem Querschnitt), Kreisverkehr
    pipette: bool,
    pip_ziel: Option<PipZiel>,
    /// World Editor: Kacheln (Rueckgaengig), gewaehlte und unter der Maus, Rueckfrage zum Loeschen
    welt: welt::Welt,
    welt_wahl: Option<(i32, i32)>,
    welt_unter: Option<(i32, i32)>,
    welt_frage: Option<(i32, i32)>,
    /// World Editor: Unterfunktion, Einsetzpunkte/Haltestellen (Listen, Rueckgaengig), Name fuer neue, Setzen per
    /// Klick an, Umbenennen (Eintrag, Text)
    welt_modus: WeltModus,
    orte: orte::Orte,
    orte_name: String,
    orte_setzen: bool,
    orte_umbenennen: Option<(i64, String)>,
    /// World Editor, Gelaende formen: Pinsel, Rueckgaengig, Punkt unter der Maus (auf dem Gelaende)
    gelaende: gelaende::Gelaende,
    gelaende_unter: Option<DVec3>,
    /// Luftbild ueber dem Gelaende (Karten mit Ort)
    luftbild: luftbild::Luftbild,
    /// neue Karte: Ortssuche (Text, Treffer, gewaehlter Ort, laufende Suche) und das Anlegen im Hintergrund (Ordner)
    ort_text: String,
    ort_treffer: Vec<geo::Ort>,
    ort_wahl: Option<geo::Ort>,
    ort_job: Option<std::thread::JoinHandle<Result<Vec<geo::Ort>>>>,
    neue_karte_job: Option<(String, std::thread::JoinHandle<Result<()>>)>,
    /// Ort einer geoeffneten Karte nachtraeglich festlegen (Bezug, Gelaende der Kacheln), mit Gelaende ersetzen
    ort_setzen_job: Option<std::thread::JoinHandle<Result<(geo::Bezug, Vec<geo::KachelDaten>)>>>,
    ort_gelaende: bool,
    /// Querschnitt-Baukasten (Fenster offen)
    baukasten: Option<baukasten::Baukasten>,
    /// Messwerkzeug (M): an, Anfangs- und Endpunkt der letzten Messung
    messen: bool,
    mess_a: Option<DVec3>,
    mess_b: Option<DVec3>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum WeltModus {
    Kacheln,
    Einsetzpunkte,
    Haltestellen,
    Gelaende,
}

impl WeltModus {
    /// Einsetzpunkte oder Haltestellen
    fn orte(self) -> bool {
        matches!(self, WeltModus::Einsetzpunkte | WeltModus::Haltestellen)
    }
}

/// was die Pipette unter der Maus hat
#[derive(Clone)]
enum PipZiel {
    Objekt(bearbeiten::Objekt),
    Strasse { sli: String, punkte: Vec<(DVec3, f64)>, halb: f64 },
    Kreisel(DVec3, netz::Kreisel),
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
    Welt,
    /// Einsetzpunkte/Haltestellen (mit ihren Objekten in den Kacheln)
    Orte,
    Gelaende,
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
    /// neue Karte anlegen und oeffnen
    /// neue Karte: Angaben, gewaehlter Ort, sonst Suchtext (erster Treffer)
    KarteNeu(karten::NeueKarte, Option<geo::Ort>, String),
    /// Ort der geoeffneten Karte festlegen (mit echtem Gelaende fuer die vorhandenen Kacheln)
    OrtFestlegen(geo::Ort, bool),
    /// Ort fuer eine neue Karte suchen
    OrtSuchen(String),
    /// Querschnitt-Baukasten oeffnen (mit dem Querschnitt aus dieser Bauanleitung)
    Baukasten(Option<querschnitt::Querschnitt>),
    /// Querschnitt gespeichert: (.sli, Querschnitt, gleich damit bauen)
    QsGespeichert(String, querschnitt::Querschnitt, bool),
    Messen,
    /// Objekte aus Ordnern anderer Karten in die geoeffnete Karte holen (dann neu laden)
    ObjekteHolen,
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
    /// Fahrtrichtungen der gewaehlten vorhandenen Strassen fuer die KI
    SplineEinbahn(netz::Einbahn),
    /// Spurpfeile/Verbinder einer Kreuzung aendern
    Spuren(spuren::Aenderung),
    /// Abbiegespuren einer eigenen Kreuzung wieder wie vorgeschlagen
    SpurenZurueck(u32),
    /// Objekt zum Platzieren uebernehmen (Pipette): .sco, Drehung
    Uebernehmen(std::path::PathBuf, f64),
    /// Pipette an/aus
    Pipette,
    /// World Editor: Kachel anfuegen / wegnehmen
    KachelNeu((i32, i32)),
    KachelLoeschen((i32, i32)),
    /// World Editor: Einsetzpunkte (Index in der Liste) und Haltestellen
    WeltModus(WeltModus),
    PunktUmbenennen(usize, String),
    PunktLoeschen(usize),
    HaltUmbenennen(orte::Haltestelle, String),
    HaltLoeschen(orte::Haltestelle),
    HaltAufnehmen(orte::Haltestelle),
    Hinfahren(DVec3),
    /// Strasse bauen mit diesem Querschnitt (Pipette)
    StrasseUebernehmen(String),
    SplineLoeschen,
    StrassenModus(strasse::Modus),
    EigeneAendern(strasse::KantenAenderung),
    /// Vorfahrt/Ampel einer Kreuzung setzen (None: wieder vermuten)
    KreuzungRegel(u32, Option<netz::Regel>),
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
            testlauf_knoten: None,
            bericht_melden: Some(protokoll::neuer_bericht()),
            eigene_auswahl: vec![],
            eigene_maus: None,
            kreuzung_wahl: None,
            kreuzung_maus: None,
            verlauf: vec![],
            verlauf_redo: vec![],
            stapel: (0, 0, 0, 0, 0, 0),
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
            katalog_editor: None,
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
            knoten: knoten::Knotenwerkzeug::default(),
            hilfe: hilfsansicht::Hilfsansicht::default(),
            spur_modus: false,
            spur_ziel: None,
            spur_zufahrt: None,
            pipette: false,
            pip_ziel: None,
            welt: welt::Welt::default(),
            welt_wahl: None,
            welt_unter: None,
            welt_frage: None,
            welt_modus: WeltModus::Kacheln,
            orte: orte::Orte::default(),
            orte_name: String::new(),
            orte_setzen: false,
            orte_umbenennen: None,
            gelaende: gelaende::Gelaende::default(),
            gelaende_unter: None,
            luftbild: luftbild::Luftbild::default(),
            ort_text: String::new(),
            ort_treffer: Vec::new(),
            ort_wahl: None,
            ort_job: None,
            neue_karte_job: None,
            ort_setzen_job: None,
            ort_gelaende: true,
            baukasten: None,
            messen: false,
            mess_a: None,
            mess_b: None,
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
            self.luftbild.karte(geo::Bezug::lesen(&self.root.join("maps").join(&self.karten[i].ordner)));
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
        self.luftbild.karte(geo::Bezug::lesen(&self.root.join("maps").join(&self.karten[i].ordner)));
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
        // Gelaende formen: der Pinsel wirkt, solange die Taste gedrueckt ist (auch ohne Mausbewegung)
        if self.gelaende.malt() {
            self.gelaende_unter = self.gelaende_treffer();
            if let (Some(g), Some(v)) = (self.gelaende_unter, self.viewer.as_mut()) {
                let strasse = &self.strasse;
                self.gelaende.malen(v, g.truncate(), dt as f64, &|q| strasse.kante_unter(q).is_some());
            }
        }
        // Kacheln im Hintergrund: Lesen und Zerlegen im Worker, hier nur ein paar ms Hochladen
        let (ziel, weite) = (self.kam.ziel, self.sichtweite());
        if let Some(v) = self.viewer.as_mut() {
            v.stream(ziel, weite, std::time::Duration::from_millis(STREAM_BUDGET_MS));
            self.luftbild.aktualisieren(v);
            self.hilfe.aktualisieren(v);
            self.bearb.unsichtbare = self.hilfe.an;
            if self.bearb.werkzeug == Werkzeug::Strasse {
                self.anschluesse.aktualisieren(v);
            }
            // die Splines der Karte braucht auch das Strassenwerkzeug (Abzweige mitten aus vorhandenen Strassen)
            if self.pipette || matches!(self.bearb.werkzeug, Werkzeug::Aendern | Werkzeug::Strasse | Werkzeug::Kreuzung | Werkzeug::Knoten) {
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
        self.jobs_pruefen();
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

    /// Hintergrundauftraege der Kartenauswahl: Ortssuche, neue Karte anlegen
    fn jobs_pruefen(&mut self) {
        if self.ort_job.as_ref().is_some_and(|j| j.is_finished()) {
            match self.ort_job.take().unwrap().join() {
                Ok(Ok(orte)) => {
                    self.meldung = if orte.is_empty() { "Ort nicht gefunden".into() } else { format!("{} Orte gefunden - einen waehlen", orte.len()) };
                    if orte.len() == 1 {
                        self.ort_wahl = orte.first().cloned();
                    }
                    self.ort_treffer = orte;
                }
                Ok(Err(e)) => self.meldung = format!("Ortssuche fehlgeschlagen: {e:#}"),
                Err(_) => self.meldung = "Ortssuche fehlgeschlagen".into(),
            }
        }
        if self.ort_setzen_job.as_ref().is_some_and(|j| j.is_finished()) {
            let erg = self.ort_setzen_job.take().unwrap().join();
            let ordner = self.sitzungsordner();
            match (erg, ordner) {
                (Ok(Ok((b, daten))), Some(ordner)) => {
                    let _ = std::fs::create_dir_all(&ordner);
                    match b.schreiben(&ordner) {
                        Ok(()) => {
                            let mut m = format!("Ort festgelegt: {} (Mitte der Kachel 0 0)", b.ort.split(',').next().unwrap_or(""));
                            if !daten.is_empty() {
                                if let Some(v) = self.viewer.as_mut() {
                                    let neu: Vec<((i32, i32), Vec<f32>)> = daten.into_iter().filter_map(|d| Some((d.kachel, d.gelaende?))).collect();
                                    match self.gelaende.uebernehmen(v, &ordner, neu) {
                                        Ok(n) => m += &format!(" - echtes Gelaende fuer {n} Kachel(n), Strg+Z nimmt es zurueck"),
                                        Err(e) => m += &format!(" - Gelaende nicht uebernommen: {e:#}"),
                                    }
                                }
                            }
                            self.luftbild.karte(Some(b));
                            // die Ortsdatei kommt beim Speichern in die Karte
                            self.welt.aenderungen += 1;
                            self.meldung = m;
                        }
                        Err(e) => self.meldung = format!("Ort nicht festgelegt: {e:#}"),
                    }
                }
                (Ok(Err(e)), _) => self.meldung = format!("Ort nicht festgelegt: {e:#}"),
                _ => self.meldung = "Ort nicht festgelegt".into(),
            }
        }
        if self.neue_karte_job.as_ref().is_some_and(|j| j.1.is_finished()) {
            let (ordner, job) = self.neue_karte_job.take().unwrap();
            match job.join() {
                Ok(Ok(())) => {
                    self.karten = karten::finden(&self.root);
                    self.kartenwahl = Some(ordner.clone());
                    self.meldung = format!("Karte angelegt (maps\\{ordner}) - wird geoeffnet");
                    if let Some(i) = self.karten.iter().position(|k| k.ordner == ordner) {
                        self.kartenwahl_offen = false;
                        self.ausfuehren(UiAktion::Karte(i));
                    }
                }
                Ok(Err(e)) => self.meldung = format!("Karte nicht angelegt: {e:#}"),
                Err(_) => self.meldung = "Karte nicht angelegt (Absturz beim Anlegen)".into(),
            }
        }
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
                if let Some((o, _)) = &self.neue_karte_job {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(format!("lege {o} an ..."));
                    });
                } else if ui.button(egui::RichText::new("+ Neue Karte erstellen ...").strong()).on_hover_text("Karte aus der OMSI-Vorlage, auf Wunsch an einem echten Ort (Gelaende, Luftbild): Kachel mit einem Stueck Strasse und einem Einsetzpunkt").clicked() {
                    self.karten_dialog = Some(KartenDialog::Neu(karten::NeueKarte::default(), false));
                }
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
            KartenDialog::Neu(n, von_hand) => {
                egui::Window::new("Neue Karte erstellen").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                    egui::Grid::new("neue_karte").num_columns(2).show(ui, |ui| {
                        ui.label("Anzeigename");
                        if ui.add(egui::TextEdit::singleline(&mut n.anzeige).hint_text("z. B. Aschaffenburg Innenstadt").desired_width(280.0)).changed() && !*von_hand {
                            n.ordner = karten::ordner_aus(&n.anzeige);
                        }
                        ui.end_row();
                        ui.label("Ordner (maps\\...)");
                        if ui.add(egui::TextEdit::singleline(&mut n.ordner).desired_width(280.0)).changed() {
                            *von_hand = true;
                        }
                        ui.end_row();
                        ui.label("Beschreibung");
                        ui.add(egui::TextEdit::multiline(&mut n.beschreibung).desired_rows(3).desired_width(280.0));
                        ui.end_row();
                        ui.label("Ort");
                        ui.horizontal(|ui| {
                            let r = ui.add(egui::TextEdit::singleline(&mut self.ort_text).hint_text("Ort, Adresse oder 49.97, 9.14").desired_width(200.0));
                            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            if (ui.add_enabled(self.ort_job.is_none(), egui::Button::new("Suchen")).clicked() || enter) && !self.ort_text.trim().is_empty() {
                                aktionen.push(UiAktion::OrtSuchen(self.ort_text.clone()));
                            }
                            if self.ort_job.is_some() {
                                ui.spinner();
                            }
                        });
                        ui.end_row();
                        ui.label("Anfangsgebiet");
                        ui.horizontal(|ui| {
                            for (g, t) in [(1, "1 Kachel"), (3, "3 x 3"), (5, "5 x 5")] {
                                if ui.selectable_label(n.groesse.max(1) == g, t).clicked() {
                                    n.groesse = g;
                                }
                            }
                            ui.label(egui::RichText::new(format!("({:.1} x {:.1} km)", n.groesse.max(1) as f64 * 0.3, n.groesse.max(1) as f64 * 0.3)).weak());
                        });
                        ui.end_row();
                    });
                    if !self.ort_treffer.is_empty() {
                        egui::ScrollArea::vertical().id_salt("orte").max_height(110.0).show(ui, |ui| {
                            for o in &self.ort_treffer {
                                let text = if o.name.chars().count() > 90 { format!("{}...", o.name.chars().take(90).collect::<String>()) } else { o.name.clone() };
                                if ui.selectable_label(self.ort_wahl.as_ref() == Some(o), text).on_hover_text(format!("{:.5}, {:.5}", o.lat, o.lon)).clicked() {
                                    self.ort_wahl = Some(o.clone());
                                }
                            }
                        });
                    }
                    match self.ort_wahl.clone() {
                        Some(o) => {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(format!("Ort: {} ({:.5}, {:.5})", o.name.split(',').next().unwrap_or(""), o.lat, o.lon)).strong());
                                if ui.small_button("ohne Ort").clicked() {
                                    self.ort_wahl = None;
                                }
                            });
                            ui.label(egui::RichText::new("Der Ort liegt in der Mitte der Kachel 0 0. Gelaende: DGM1 Bayern (1 m), sonst weltweit Terrain Tiles (~30 m). Luftbild (DOP40, nur Bayern) mit Regler oben im Editor. Die Karte liegt im UTM-Gitter: im World Editor angefuegte Kacheln bekommen das echte Gelaende und Luftbild ihrer Stelle.").small().weak());
                        }
                        None if !self.ort_text.trim().is_empty() => {
                            ui.colored_label(egui::Color32::from_rgb(255, 200, 80), "Noch kein Ort gewaehlt: \"Suchen\" und einen Treffer anklicken - sonst nimmt \"Erstellen\" den ersten Treffer.");
                        }
                        None => {
                            ui.label(egui::RichText::new("Ohne Ort: flaches Gelaende, kein Luftbild.").small().weak());
                        }
                    }
                    ui.label(egui::RichText::new("Die Karte entsteht aus OMSIs Vorlage (template\\NewMap): Kachel 0 0 mit einem 120 m langen Stueck Strasse und einem Einsetzpunkt \"Start\" darauf. Weitere Kacheln: World Editor; Strassen: Strasse bauen.").small().weak());
                    let ordner_ok = speichern::name_ok(&n.ordner) && !self.root.join("maps").join(n.ordner.trim()).exists();
                    if !n.ordner.is_empty() && !ordner_ok {
                        ui.colored_label(egui::Color32::from_rgb(255, 120, 90), "Ordnername ungueltig oder schon vorhanden");
                    }
                    ui.horizontal(|ui| {
                        if ui.add_enabled(ordner_ok && !n.anzeige.trim().is_empty() && self.neue_karte_job.is_none(), egui::Button::new(egui::RichText::new("Erstellen und oeffnen").strong())).clicked() {
                            aktionen.push(UiAktion::KarteNeu(n.clone(), self.ort_wahl.clone(), self.ort_text.clone()));
                            zu = true;
                        }
                        if ui.button("Abbrechen").clicked() {
                            zu = true;
                        }
                    });
                });
            }
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
                    ui.label("Sie kommt mit ihren eigenen Objekten (Aschaffenburg) in den Papierkorb.");
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
            KartenDialog::FremdeObjekte(n, fehlend) => {
                let (n, fehlend) = (*n, *fehlend);
                egui::Window::new("Objekte anderer Karten").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                    ui.label(format!("Diese Karte nutzt {n} Kreuzungsobjekt(e) aus den Ordnern anderer Karten (Aschaffenburg)."));
                    if fehlend > 0 {
                        ui.colored_label(egui::Color32::from_rgb(255, 120, 90), format!("{fehlend} davon fehlen (ihre Karte wurde geloescht oder umbenannt) - sie werden im Papierkorb gesucht."));
                    }
                    ui.label("\"In die Karte holen\" kopiert sie in den eigenen Ordner der Karte; danach haengt sie von keiner anderen Karte mehr ab. Die betroffenen Kacheln werden vorher gesichert.");
                    ui.horizontal(|ui| {
                        if ui.button("In die Karte holen").clicked() {
                            aktionen.push(UiAktion::ObjekteHolen);
                            zu = true;
                        }
                        if ui.button("Spaeter").clicked() {
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
        self.jobs_pruefen();
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
                    self.welt.aenderungen = 0;
                    self.gelaende.aenderungen = 0;
                    self.orte.aenderungen = 0;
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
                    // nutzt die Karte Objekte aus Ordnern anderer Karten? (aeltere Speicherungen; fehlende nach Loeschen)
                    if let Some(k) = self.karte.clone() {
                        let fremde = speichern::fremde_objekte(&self.root, &k);
                        if !fremde.is_empty() {
                            let fehlend = fremde.iter().filter(|(b, o, d)| !self.root.join("Sceneryobjects").join(b).join(o).join(d).is_file()).count();
                            log::warn!("Karte {k} nutzt {} Objekte anderer Karten, {fehlend} fehlen: {fremde:?}", fremde.len());
                            self.karten_dialog = Some(KartenDialog::FremdeObjekte(fremde.len(), fehlend));
                        }
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
        let objekte = if self.bearb.werkzeug == Werkzeug::Objekte || (self.bearb.werkzeug == Werkzeug::Platzieren && self.bearb.unter_maus.is_some()) {
            self.viewer.as_ref().map(|v| self.bearb.objekte(v)).unwrap_or_default()
        } else {
            Vec::new()
        };
        let spur_ansicht = if self.bearb.werkzeug == Werkzeug::Kreuzung && self.spur_modus { self.spur_ansicht() } else { None };
        if let (Werkzeug::Welt, true, Some(v), Some(k)) = (self.bearb.werkzeug, self.welt_modus.orte(), self.viewer.as_ref(), self.karte.as_ref()) {
            let liste = self.welt.anfangsliste(v);
            self.orte.laden(&self.root.join("maps").join(k), &liste);
        }
        let orte_halte: Vec<orte::Haltestelle> = match (self.bearb.werkzeug, self.welt_modus, self.viewer.as_ref()) {
            (Werkzeug::Welt, WeltModus::Haltestellen, Some(v)) => self.orte.haltestellen(v),
            _ => Vec::new(),
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
                    for (wz, t) in [(Werkzeug::Ansehen, "Ansehen"), (Werkzeug::Objekte, "Objekte (O)"), (Werkzeug::Platzieren, "Platzieren (P)"), (Werkzeug::Strasse, "Strasse bauen (B)"), (Werkzeug::Aendern, "Aendern (U)"), (Werkzeug::Knoten, "Knoten (N)"), (Werkzeug::Kreuzung, "Kreuzungen (X)")] {
                        if ui.selectable_label(self.bearb.werkzeug == wz, t).clicked() {
                            aktionen.push(UiAktion::Werkzeug(wz));
                        }
                    }
                    if ui.selectable_label(self.bearb.werkzeug == Werkzeug::Welt, "World Editor").on_hover_text("Kacheln hinzufuegen/loeschen, spaeter Gelaende ...").clicked() {
                        aktionen.push(UiAktion::Werkzeug(if self.bearb.werkzeug == Werkzeug::Welt { Werkzeug::Ansehen } else { Werkzeug::Welt }));
                    }
                    ui.separator();
                    if ui.selectable_label(self.messen, "Messen (M)").on_hover_text("Strecke messen (waagerecht, mit Hoehenunterschied) - z. B. Strassenbreiten im Luftbild").clicked() {
                        aktionen.push(UiAktion::Messen);
                    }
                    if ui.selectable_label(self.pipette, "\u{1F58C} Pipette (I)").on_hover_text("Klick auf ein Objekt, einen Baum, eine Strasse oder einen Kreisverkehr: wechselt ins passende Werkzeug und uebernimmt es").clicked() {
                        aktionen.push(UiAktion::Pipette);
                    }
                    ui.separator();
                    if self.luftbild.bezug.is_some() {
                        let mut an = self.luftbild.deckkraft > 0.0;
                        if ui.checkbox(&mut an, "Luftbild").on_hover_text("Luftbild ueber dem Gelaende (Karte mit Ort)").changed() {
                            self.luftbild.deckkraft = if an { 0.6 } else { 0.0 };
                        }
                        let mut prozent = self.luftbild.deckkraft * 100.0;
                        if ui.add(egui::Slider::new(&mut prozent, 0.0..=100.0).integer().suffix(" %").show_value(true)).on_hover_text("Deckkraft des Luftbilds").changed() {
                            self.luftbild.deckkraft = prozent / 100.0;
                        }
                        if self.luftbild.laedt() {
                            ui.spinner();
                        }
                        ui.separator();
                    } else if self.viewer.is_some() && ui.button("Luftbild ...").on_hover_text("Die Karte hat noch keinen Ort: World Editor -> Kacheln bearbeiten -> Ort der Karte festlegen").clicked() {
                        aktionen.push(UiAktion::Werkzeug(Werkzeug::Welt));
                        aktionen.push(UiAktion::WeltModus(WeltModus::Kacheln));
                    }
                    ui.checkbox(&mut self.hilfe.an, "Pfade (H)").on_hover_text("Pfade (blau Strasse, gelb Kreuzung, gruen Gehweg, orange Gleis, lila unsichtbare Strasse) und unsichtbare Objekte zeigen - wie \"Show paths\" im nEditor");
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
                    let geaendert = self.geaendert() > 0;
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
            let messung = self.messung();
            if let Some(bk) = self.baukasten.as_mut() {
                let mut offen = true;
                if let Some(baukasten::Ergebnis::Gespeichert(rel, q, bauen)) = bk.ui(ctx, &self.root, &mut offen, messung) {
                    aktionen.push(UiAktion::QsGespeichert(rel, q, bauen));
                }
                if !offen {
                    self.baukasten = None;
                }
            }
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
                        if ui.button("Pipette: zum Platzieren uebernehmen").on_hover_text("wechselt ins Platzieren mit diesem Objekt und seiner Drehung (Taste I: Pipette)").clicked() {
                            aktionen.push(UiAktion::Uebernehmen(o.sco.clone(), o.richtung));
                        }
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
                    ui.horizontal(|ui| {
                        if ui.selectable_label(self.pipette, "\u{1F58C} Pipette (I)").on_hover_text("naechster Klick auf ein Objekt im Bild uebernimmt es samt Drehung").clicked() {
                            aktionen.push(UiAktion::Pipette);
                        }
                        ui.label(egui::RichText::new(if self.pipette { "Klick auf ein Objekt im Bild" } else { "oder Strg+Klick auf ein Objekt" }).small().weak());
                    });
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
                        ui.label("Art");
                        let txt = match self.katalog_editor { None => "alle", Some(false) => "sichtbare Objekte", Some(true) => "Editor-Objekte (unsichtbar)" };
                        egui::ComboBox::from_id_salt("art").width(220.0).selected_text(txt).show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.katalog_editor, None, "alle");
                            ui.selectable_value(&mut self.katalog_editor, Some(false), "sichtbare Objekte");
                            ui.selectable_value(&mut self.katalog_editor, Some(true), format!("Editor-Objekte (unsichtbar, {})", kat.eintraege.iter().filter(|e| e.editor).count()))
                                .on_hover_text("Objekte, die das Spiel nicht zeichnet: Einstiegspunkte, Haltestellen-Marken, Schallquellen, unsichtbare Kreuzungen ... Mit H (Pfade) werden sie im Bild gezeigt.");
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
                        .filter(|e| e.passt(&suche, self.katalog_ordner.as_deref(), self.katalog_gruppe.as_deref(), self.katalog_herkunft.as_deref(), self.katalog_editor))
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
                        ui.label(egui::RichText::new("Kreisverkehr").strong());
                        ui.label("Klick setzt die Mitte, die Maus die Groesse (Durchmesser 24 bis 120 m), Klick baut. Er wird ein Objekt wie in Rheinhausen: runde Ringfahrbahn (7 m), Mittelinsel, Gehweg aussen, Ring mit Vorfahrt.");
                        ui.label("Liegt er ueber vorhandenen Strassen, faellt alles im Ring weg und sie werden Zufahrten. Weitere Zufahrten: eine Strasse auf den Kreisverkehr ziehen.");
                    } else {
                        ui.horizontal(|ui| {
                            ui.label("Querschnitt");
                            if ui.button("Querschnitt-Baukasten ...").on_hover_text("eigenen Querschnitt aus Fahrspuren, Gehwegen, Rad- und Parkstreifen ... zusammenstellen").clicked() {
                                aktionen.push(UiAktion::Baukasten(None));
                            }
                            // gewaehlter eigener Querschnitt: bearbeiten
                            let eigen = self.strasse.sli.as_ref().and_then(|rel| {
                                let p = self.root.join(rel.replace('\\', "/")).with_extension("qs.json");
                                serde_json::from_slice::<serde_json::Value>(&std::fs::read(p).ok()?).ok().and_then(|v| querschnitt::Querschnitt::aus_json(&v))
                            });
                            if let Some(q) = eigen {
                                if ui.small_button("bearbeiten").on_hover_text("den gewaehlten eigenen Querschnitt im Baukasten oeffnen").clicked() {
                                    aktionen.push(UiAktion::Baukasten(Some(q)));
                                }
                            }
                        });
                        if let Some(rel) = self.qs_raster(ui, self.strasse.sli.clone()) {
                            aktionen.push(UiAktion::Querschnitt(rel));
                        }
                    }
                    ui.separator();
                    ui.label(egui::RichText::new("Klick: Start / naechster Punkt (rastet an Knoten ein, gruen = freies Ende: tangential weiter). Bild auf/ab: Hoehe. Entf: Strasse unter der Maus loeschen.").small().weak());
                });
            }
            if self.bearb.werkzeug == Werkzeug::Knoten {
                egui::Panel::right("knoten").default_size(360.0).show(ctx, |ui| {
                    ui.heading("Knoten: Verlauf aendern");
                    ui.label("Knoten mit der linken Maustaste greifen und ziehen - die Strasse legt sich in glatten Boegen neu, die Enden zu den Nachbarn bleiben verbunden.");
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("Griffe").strong());
                    ui.label("blau: Verbindung zweier Splines oder freies Ende der Karte");
                    ui.label("gruen: Knoten eigener Strassen");
                    ui.label("orange Quadrat: Kreuzung - wird mit ihren Strassen verschoben");
                    ui.label("Umschalt + irgendwo auf einer Strasse ziehen: setzt dort einen neuen Knoten");
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("Beim Ziehen").strong());
                    ui.label("Mausrad: Richtung am Knoten drehen (Umschalt: fein)");
                    ui.label("Bild auf/ab: Hoehe (Umschalt: fein), Pos1: Hoehe zuruecksetzen");
                    ui.label("Esc: abbrechen");
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("Die Vorschau ist blau; orange: enge Kurve; rot: so nicht moeglich. Strg+Z nimmt jeden Zug zurueck.").small().weak());
                });
            }
            if self.bearb.werkzeug == Werkzeug::Welt {
                egui::Panel::right("welt").default_size(360.0).show(ctx, |ui| {
                    ui.heading("World Editor");
                    for (m, t) in [(WeltModus::Kacheln, "Kacheln bearbeiten"), (WeltModus::Gelaende, "Gelaende formen"), (WeltModus::Einsetzpunkte, "Einsetzpunkte"), (WeltModus::Haltestellen, "Haltestellen")] {
                        if ui.selectable_label(self.welt_modus == m, t).clicked() && self.welt_modus != m {
                            aktionen.push(UiAktion::WeltModus(m));
                        }
                    }
                    ui.separator();
                    if self.welt_modus == WeltModus::Gelaende {
                        let unter = self.gelaende_unter;
                        let ziel = self.gelaende.strich_ziel();
                        let p = &mut self.gelaende.pinsel;
                        ui.label(egui::RichText::new("Gelaende formen").strong());
                        ui.horizontal_wrapped(|ui| {
                            for m in gelaende::Modus::ALLE {
                                if ui.selectable_label(p.modus == m, m.name()).clicked() {
                                    p.modus = m;
                                }
                            }
                        });
                        ui.label(p.modus.hilfe());
                        ui.add_space(4.0);
                        ui.add(egui::Slider::new(&mut p.radius, gelaende::RADIUS.0..=gelaende::RADIUS.1).logarithmic(true).integer().suffix(" m").text("Radius"));
                        let mut prozent = p.staerke * 100.0;
                        if ui.add(egui::Slider::new(&mut prozent, 5.0..=100.0).integer().suffix(" %").text("Staerke")).changed() {
                            p.staerke = prozent / 100.0;
                        }
                        if p.modus == gelaende::Modus::Ebnen {
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut p.fest, "feste Zielhoehe");
                                ui.add_enabled(p.fest, egui::DragValue::new(&mut p.ziel).speed(0.1).suffix(" m"));
                            });
                            ui.horizontal_wrapped(|ui| {
                                ui.label("Hoehe einrasten:");
                                for (r, t) in [(0.0, "aus"), (0.5, "0,5 m"), (1.0, "1 m"), (2.5, "2,5 m"), (5.0, "5 m")] {
                                    if ui.selectable_label(p.raster == r, t).clicked() {
                                        p.raster = r;
                                    }
                                }
                            });
                            ui.label(egui::RichText::new(if p.fest { "Strg+Klick ins Gelaende greift eine neue Zielhoehe ab." } else { "Ziel ist die Hoehe, wo du ansetzt (Strg+Klick: als feste Hoehe abgreifen)." }).small().weak());
                        }
                        ui.add_space(4.0);
                        ui.checkbox(&mut p.schuetzen, "Strassen schuetzen").on_hover_text("Unter Strassen, Gehwegen und Kreuzungen, die auf dem Boden liegen, bleibt das Gelaende, wie es ist. In OMSI wird es dort nicht ausgeschnitten - angehobener Boden laege sonst auf der Fahrbahn. Unter Bruecken wird trotzdem geformt.");
                        ui.separator();
                        match (unter, ziel) {
                            (_, Some(z)) => ui.label(format!("Zielhoehe {z:.2} m")),
                            (Some(g), None) => ui.label(format!("Gelaende unter der Maus: {:.2} m", g.z)),
                            _ => ui.label(""),
                        };
                        ui.label("Linke Maustaste halten: formen");
                        ui.label("Strg+Mausrad: Radius, Umschalt+Mausrad: Staerke");
                        if p.modus == gelaende::Modus::Heben || p.modus == gelaende::Modus::Senken {
                            ui.label("Strg+Maustaste: umgekehrt (heben/senken)");
                        }
                        ui.separator();
                        ui.label(egui::RichText::new("Objekte und Baeume stellen sich nach jedem Strich auf den neuen Boden. Strg+Z nimmt jeden Strich zurueck; die Karte bleibt bis zum Speichern unveraendert.").small().weak());
                        return;
                    }
                    if self.welt_modus.orte() {
                        let punkte = self.welt_modus == WeltModus::Einsetzpunkte;
                        ui.label(egui::RichText::new(if punkte { "Einsetzpunkte" } else { "Haltestellen" }).strong());
                        ui.label(if punkte { "Wo man bei \"Freie Fahrt\" startet (unsichtbares Objekt entrypoint_bus.sco + Eintrag in global.cfg)." }
                                 else { "Haltestellen-Objekte (bus_stop.sco, Name = erster Text) und ihre Liste fuer die Fahrplaene (TTData/Busstops.cfg)." });
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut self.orte_name).hint_text("Name").desired_width(170.0));
                            let t = if self.orte_setzen { "Klick ins Bild ..." } else if punkte { "+ Einsetzpunkt setzen" } else { "+ Haltestelle setzen" };
                            if ui.selectable_label(self.orte_setzen, t).on_hover_text("Klick ins Bild setzt ihn auf die naechste Fahrspur, in deren Richtung").clicked() {
                                self.orte_setzen = !self.orte_setzen;
                            }
                        });
                        ui.separator();
                        let pos_jetzt: std::collections::HashMap<i64, DVec3> = self.viewer.as_ref().map(|v| v.hidden_objects().into_iter().map(|x| (x.0, x.1)).collect()).unwrap_or_default();
                        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                            if punkte {
                                if self.orte.punkte().is_empty() {
                                    ui.label(egui::RichText::new("keine Einsetzpunkte").weak());
                                }
                                for (i, p) in self.orte.punkte().iter().enumerate() {
                                    let pos = pos_jetzt.get(&p.id).copied().unwrap_or(p.pos);
                                    ui.horizontal(|ui| {
                                        match self.orte_umbenennen.as_mut().filter(|x| x.0 == i as i64) {
                                            Some((_, text)) => {
                                                let r = ui.add(egui::TextEdit::singleline(text).desired_width(170.0));
                                                if ui.button("OK").clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                                                    aktionen.push(UiAktion::PunktUmbenennen(i, text.clone()));
                                                }
                                            }
                                            None => {
                                                ui.label(&p.name);
                                                if ui.small_button("hin").clicked() {
                                                    aktionen.push(UiAktion::Hinfahren(pos));
                                                }
                                                if ui.small_button("umbenennen").clicked() {
                                                    self.orte_umbenennen = Some((i as i64, p.name.clone()));
                                                }
                                                if ui.small_button("loeschen").clicked() {
                                                    aktionen.push(UiAktion::PunktLoeschen(i));
                                                }
                                            }
                                        }
                                    });
                                }
                            } else {
                                if orte_halte.is_empty() {
                                    ui.label(egui::RichText::new("keine Haltestellen").weak());
                                }
                                for h in &orte_halte {
                                    let pos = pos_jetzt.get(&h.id).copied().unwrap_or(h.pos);
                                    let gelistet = self.orte.busstops().iter().any(|b| b.id == h.id);
                                    ui.horizontal(|ui| {
                                        match self.orte_umbenennen.as_mut().filter(|x| x.0 == h.id) {
                                            Some((_, text)) => {
                                                let r = ui.add(egui::TextEdit::singleline(text).desired_width(170.0));
                                                if ui.button("OK").clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                                                    aktionen.push(UiAktion::HaltUmbenennen(h.clone(), text.clone()));
                                                }
                                            }
                                            None => {
                                                ui.label(&h.name);
                                                if !gelistet && ui.small_button("in Busstops.cfg").on_hover_text("fehlt in der Haltestellenliste der Fahrplaene - aufnehmen").clicked() {
                                                    aktionen.push(UiAktion::HaltAufnehmen(h.clone()));
                                                }
                                                if ui.small_button("hin").clicked() {
                                                    aktionen.push(UiAktion::Hinfahren(pos));
                                                }
                                                if ui.small_button("umbenennen").clicked() {
                                                    self.orte_umbenennen = Some((h.id, h.name.clone()));
                                                }
                                                if ui.small_button("loeschen").clicked() {
                                                    aktionen.push(UiAktion::HaltLoeschen(h.clone()));
                                                }
                                            }
                                        }
                                    });
                                }
                            }
                        });
                        ui.separator();
                        ui.label(egui::RichText::new("Verschieben und drehen: Werkzeug Objekte mit Pfad-Ansicht (H). Strg+Z nimmt jede Aenderung zurueck; geschrieben wird beim Speichern (Einsetzpunkte in global.cfg, Haltestellen in TTData/Busstops.cfg).").small().weak());
                        return;
                    }
                    ui.label(egui::RichText::new("Kacheln bearbeiten").strong());
                    let n = self.viewer.as_ref().map(|v| v.map_tiles().len()).unwrap_or(0);
                    ui.label(format!("{n} Kacheln (je 300 x 300 m)"));
                    ui.label("Im Bild: weiss = Kacheln der Karte, + = freie Plaetze am Rand. Klick auf ein + legt dort eine neue Kachel an (Gelaende schliesst an die Nachbarn an). Klick auf eine Kachel waehlt sie.");
                    if let (Some(k), Some(v)) = (self.welt_wahl, self.viewer.as_ref()) {
                        if v.map_tiles().contains(&k) {
                            ui.separator();
                            let (o, sp) = welt::inhalt(v, k);
                            ui.label(egui::RichText::new(format!("Kachel {} {}", k.0, k.1)).strong());
                            ui.label(format!("{o} Objekte, {sp} Splines"));
                            if ui.button("Kachel loeschen (Entf)").clicked() {
                                self.welt_frage = Some(k);
                            }
                        }
                    }
                    ui.separator();
                    ui.label(egui::RichText::new("Ort der Karte").strong());
                    match self.luftbild.bezug.clone() {
                        Some(b) => {
                            ui.label(format!("{}", b.ort.split(',').take(2).collect::<Vec<_>>().join(",")));
                            ui.label(egui::RichText::new(format!("UTM {} {:.0} / {:.0}, NN {:.0} m = 0. Neue Kacheln bekommen das echte Gelaende und Luftbild ihrer Stelle.", b.zone, b.ost0, b.nord0, b.nn0)).small().weak());
                        }
                        None => {
                            ui.label(egui::RichText::new("Die Karte hat keinen Ort. Mit Ort: Luftbild (Regler oben), echtes Gelaende fuer neue Kacheln. Der Ort kommt in die Mitte der Kachel 0 0.").small());
                            ui.horizontal(|ui| {
                                let r = ui.add(egui::TextEdit::singleline(&mut self.ort_text).hint_text("Ort, Adresse oder 49.97, 9.14").desired_width(190.0));
                                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                                if (ui.add_enabled(self.ort_job.is_none(), egui::Button::new("Suchen")).clicked() || enter) && !self.ort_text.trim().is_empty() {
                                    aktionen.push(UiAktion::OrtSuchen(self.ort_text.clone()));
                                }
                                if self.ort_job.is_some() || self.ort_setzen_job.is_some() {
                                    ui.spinner();
                                }
                            });
                            egui::ScrollArea::vertical().id_salt("orte_welt").max_height(100.0).show(ui, |ui| {
                                for o in &self.ort_treffer {
                                    let text = if o.name.chars().count() > 60 { format!("{}...", o.name.chars().take(60).collect::<String>()) } else { o.name.clone() };
                                    if ui.selectable_label(self.ort_wahl.as_ref() == Some(o), text).clicked() {
                                        self.ort_wahl = Some(o.clone());
                                    }
                                }
                            });
                            ui.checkbox(&mut self.ort_gelaende, "Gelaende der vorhandenen Kacheln ersetzen").on_hover_text("Echtes Gelaende fuer alle Kacheln der Karte (Strg+Z nimmt es zurueck). Vorhandene Strassen behalten ihre Hoehe - sinnvoll vor allem bei neuen, noch leeren Karten.");
                            if ui.add_enabled(self.ort_wahl.is_some() && self.ort_setzen_job.is_none(), egui::Button::new("Ort festlegen")).clicked() {
                                if let Some(o) = self.ort_wahl.clone() {
                                    aktionen.push(UiAktion::OrtFestlegen(o, self.ort_gelaende));
                                }
                            }
                        }
                    }
                    ui.separator();
                    ui.label(egui::RichText::new("Die Karte bleibt bis zum Speichern unveraendert. Strg+Z nimmt jede Kachel zurueck. Neue Kacheln kommen ans Ende der Kachelliste; beim Loeschen werden Einsetzpunkte und Haltestellenliste beim Speichern umnummeriert.").small().weak());
                });
            }
            if let Some(k) = self.welt_frage {
                let (o, sp) = self.viewer.as_ref().map(|v| welt::inhalt(v, k)).unwrap_or((0, 0));
                let fahrplan = self.karte.as_ref().is_some_and(|kt| std::fs::read_dir(self.root.join("maps").join(kt).join("TTData"))
                    .map(|r| r.flatten().any(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("ttr")))).unwrap_or(false));
                egui::Window::new("Kachel loeschen?").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                    ui.label(format!("Kachel {} {} mit {o} Objekten und {sp} Splines aus der Karte nehmen?", k.0, k.1));
                    if fahrplan {
                        ui.colored_label(egui::Color32::from_rgb(255, 150, 80), "Die Karte hat Fahrwege (TTData/*.ttr): sie nennen Kacheln ueber ihre Nummer, die sich fuer die Kacheln hinter dieser verschiebt - Fahrplaene danach pruefen.");
                    }
                    ui.label(egui::RichText::new("Strg+Z holt sie zurueck; die Datei bleibt im Kartenordner.").small().weak());
                    ui.horizontal(|ui| {
                        if ui.button("Loeschen").clicked() {
                            aktionen.push(UiAktion::KachelLoeschen(k));
                        }
                        if ui.button("Abbrechen").clicked() {
                            self.welt_frage = None;
                        }
                    });
                });
            }
            if self.bearb.werkzeug == Werkzeug::Kreuzung {
                egui::Panel::right("kreuzungen").default_size(360.0).show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        if ui.selectable_label(!self.spur_modus, "Vorfahrt und Ampel").clicked() {
                            self.spur_modus = false;
                        }
                        if ui.selectable_label(self.spur_modus, "Spuren (Pfeile/Verbinder)").clicked() {
                            self.spur_modus = true;
                        }
                    });
                    ui.separator();
                    if self.spur_modus {
                        ui.heading("Spurpfeile und Verbinder");
                        let Some(an) = spur_ansicht.as_ref() else {
                            ui.label("Klick auf eine Kreuzung waehlt sie - eigene oder vorhandene der Karte (die wird dabei nicht ersetzt).");
                            ui.label(egui::RichText::new("Wie Traffic Manager: President Edition: je Zufahrtsspur festlegen, wohin sie fahren darf; jede Abbiegespur einzeln an- und abschalten.").small().weak());
                            return;
                        };
                        let eigen = matches!(an.ziel, spuren::Ziel::Netz(_));
                        ui.label(egui::RichText::new(match an.ziel {
                            spuren::Ziel::Netz(_) => "eigene Kreuzung".to_string(),
                            spuren::Ziel::Karte { objekt, .. } => format!("Kreuzung der Karte (Objekt {objekt})"),
                        }).strong());
                        ui.label(format!("{} Zufahrtsspuren, {} Abbiegespuren ({} fuer die KI gesperrt)", an.zufahrten.len(), an.verbindungen.len(),
                                         an.verbindungen.iter().filter(|x| !x.erlaubt).count()));
                        ui.separator();
                        match self.spur_zufahrt.filter(|z| *z < an.zufahrten.len()) {
                            None => {
                                ui.label("Klick auf einen weissen Punkt (Zufahrtsspur) waehlt sie; Klick auf eine Linie schaltet diese Abbiegespur an/aus.");
                            }
                            Some(z) => {
                                ui.label(egui::RichText::new(format!("Zufahrtsspur {} - darf fahren:", z + 1)).strong());
                                let jetzt = an.pfeile(z);
                                let moeglich = an.moeglich(z);
                                ui.horizontal_wrapped(|ui| {
                                    for art in spuren::Art::ALLE {
                                        if !moeglich.contains(&art) {
                                            continue;
                                        }
                                        let ist = jetzt.contains(&art);
                                        if ui.selectable_label(ist, art.zeichen()).clicked() {
                                            if let Some(a) = an.pfeil(z, art, !ist) {
                                                aktionen.push(UiAktion::Spuren(a));
                                            }
                                        }
                                    }
                                });
                                if !eigen {
                                    ui.label(egui::RichText::new("Kreuzung der Karte: nur vorhandene Abbiegespuren lassen sich sperren/freigeben ([rule] no_cars), neue gehen nicht.").small().weak());
                                }
                            }
                        }
                        if let (true, spuren::Ziel::Netz(k)) = (eigen, an.ziel) {
                            ui.separator();
                            if ui.button("Vorschlag wiederherstellen").on_hover_text("Abbiegespuren wieder so, wie der Editor sie bildet").clicked() {
                                aktionen.push(UiAktion::SpurenZurueck(k));
                            }
                        }
                        ui.separator();
                        ui.label(egui::RichText::new("Im Bild: gruen = erlaubt, rot = fuer die KI gesperrt. Eigene Kreuzungen werden mit genau diesen Abbiegespuren neu gebaut. Strg+Z nimmt jede Aenderung zurueck.").small().weak());
                        return;
                    }
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
                        if let Some(art) = einbahn_knoepfe(ui, e.as_ref().map(|e| e.einbahn)) {
                            aktionen.push(UiAktion::EigeneAendern(strasse::KantenAenderung::Einbahn(art)));
                        }
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
                            // jetziger Stand: welche Fahrzeugpfade sind gesperrt?
                            let stand = self.viewer.as_ref().and_then(|v| v.spline_lanes(&s.sli)).map(|(p, _)| {
                                let pfade: Vec<(u8, u8)> = p.iter().map(|x| (x.0, x.4)).collect();
                                let mut g: Vec<usize> = s.gesperrt.iter().copied().filter(|i| pfade.get(*i).is_some_and(|x| x.0 == 0)).collect();
                                g.sort_unstable();
                                [netz::Einbahn::Beide, netz::Einbahn::Vor, netz::Einbahn::Zurueck, netz::Einbahn::Gesperrt].into_iter().find(|a| a.gesperrt(&pfade, s.gespiegelt) == g)
                            });
                            if let Some(art) = einbahn_knoepfe(ui, stand.flatten()) {
                                aktionen.push(UiAktion::SplineEinbahn(art));
                            }
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
                    if self.luftbild.deckkraft > 0.0 && !self.luftbild.quellen.is_empty() {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(egui::RichText::new(self.luftbild.quellen.join("; ")).small().weak());
                        });
                    }
                });
            });
            // Markierungen ueber dem 3D-Bild (unter den Panels)
            let maler = ui_maler(ctx);
            if let (true, Some(v)) = (self.hilfe.an, self.viewer.as_ref()) {
                let r = (self.kam.abstand * 2.5 + 150.0).min(1500.0);
                let pt = |p: DVec3| bearbeiten::projizieren(&self.kam, p + DVec3::Z * 0.15, bw, bh).map(|(x, y, _)| egui::pos2(x, y));
                for pf in hilfsansicht::pfade(v, self.kam.ziel, r) {
                    let farbe = egui::Color32::from_rgb(pf.farbe[0], pf.farbe[1], pf.farbe[2]);
                    let strich = egui::Stroke::new(if pf.unsichtbar { 2.5 } else { 1.6 }, farbe);
                    let mut linie: Vec<egui::Pos2> = Vec::new();
                    for q in pf.punkte {
                        match pt(*q) {
                            Some(x) => linie.push(x),
                            None => {
                                if linie.len() > 1 {
                                    maler.add(egui::Shape::line(std::mem::take(&mut linie), strich));
                                }
                                linie.clear();
                            }
                        }
                    }
                    // Pfeil in Fahrtrichtung in der Mitte
                    if linie.len() > 1 {
                        let i = (linie.len() - 1) / 2;
                        let (a, b) = (linie[i], linie[i + 1]);
                        let d = b - a;
                        if d.length() > 0.5 {
                            let d = d.normalized() * 7.0;
                            let m = a + (b - a) * 0.5;
                            let n = egui::vec2(-d.y, d.x) * 0.6;
                            maler.add(egui::Shape::line(vec![m - d + n, m, m - d - n], strich));
                        }
                        maler.add(egui::Shape::line(linie, strich));
                    }
                }
                let maus = self.boden_unter_maus;
                for (pos, richtung, name, modell) in &self.hilfe.objekte {
                    if (pos.truncate() - self.kam.ziel.truncate()).length() > r {
                        continue;
                    }
                    let Some(m) = pt(*pos) else { continue };
                    let c = if *modell { egui::Color32::from_rgb(120, 230, 255) } else { egui::Color32::from_rgb(235, 80, 235) };
                    let e = 6.0;
                    maler.add(egui::Shape::closed_line(vec![m + egui::vec2(0.0, -e), m + egui::vec2(e, 0.0), m + egui::vec2(0.0, e), m + egui::vec2(-e, 0.0)], egui::Stroke::new(2.0, c)));
                    if let Some(q) = pt(*pos + (crate::netz::dir(*richtung) * 2.0).extend(0.0)) {
                        maler.line_segment([m, q], egui::Stroke::new(1.5, c));
                    }
                    if maus.is_some_and(|g| (g.truncate() - pos.truncate()).length() < 15.0) {
                        maler.text(m + egui::vec2(9.0, -9.0), egui::Align2::LEFT_BOTTOM, name, egui::FontId::proportional(13.0), c);
                    }
                }
            }
            if let Some(w) = self.bearb.unter_maus.filter(|w| Some(*w) != self.bearb.wahl) {
                if let Some(o) = objekte.iter().find(|o| o.wahl == w) {
                    bearbeiten::markieren(&maler, &self.kam, o, bw, bh, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 170), 1.5);
                }
            }
            if let Some(o) = gewaehlt.as_ref() {
                bearbeiten::markieren(&maler, &self.kam, o, bw, bh, egui::Color32::from_rgb(255, 60, 220), 3.0);
            }
            // Messung: Linie mit Laenge (waagerecht) und Hoehenunterschied; waehrend des Messens bis zur Maus
            if self.messen || self.baukasten.is_some() {
                let ende = self.mess_b.or(if self.messen { self.boden_unter_maus } else { None });
                if let (Some(a), Some(b)) = (self.mess_a, ende) {
                    let pt = |p: DVec3| bearbeiten::projizieren(&self.kam, p + DVec3::Z * 0.3, bw, bh).map(|(x, y, _)| egui::pos2(x, y));
                    let c = egui::Color32::from_rgb(255, 230, 60);
                    if let (Some(pa), Some(pb)) = (pt(a), pt(b)) {
                        maler.line_segment([pa, pb], egui::Stroke::new(2.5, c));
                        maler.circle_filled(pa, 4.0, c);
                        maler.circle_filled(pb, 4.0, c);
                        let d = (b - a).truncate().length();
                        let dh = b.z - a.z;
                        let text = if dh.abs() >= 0.05 { format!("{d:.2} m  (Hoehe {dh:+.2} m, {:.1} %)", if d > 0.01 { dh / d * 100.0 } else { 0.0 }) } else { format!("{d:.2} m") };
                        let mitte = egui::pos2((pa.x + pb.x) / 2.0, (pa.y + pb.y) / 2.0);
                        let r = maler.text(mitte + egui::vec2(8.0, -8.0), egui::Align2::LEFT_BOTTOM, &text, egui::FontId::proportional(15.0), c);
                        maler.rect_filled(r.expand(3.0), 3.0, egui::Color32::from_black_alpha(150));
                        maler.text(mitte + egui::vec2(8.0, -8.0), egui::Align2::LEFT_BOTTOM, &text, egui::FontId::proportional(15.0), c);
                    }
                }
            }
            if let (Werkzeug::Welt, WeltModus::Gelaende, Some(v), Some(g)) = (self.bearb.werkzeug, self.welt_modus, self.viewer.as_ref(), self.gelaende_unter) {
                let pinsel = &self.gelaende.pinsel;
                let ring = |r: f64| -> Vec<egui::Pos2> {
                    (0..=72).filter_map(|i| {
                        let a = i as f64 / 72.0 * std::f64::consts::TAU;
                        let (x, y) = (g.x + r * a.cos(), g.y + r * a.sin());
                        let z = v.terrain_height(x, y).unwrap_or(g.z) + 0.3;
                        bearbeiten::projizieren(&self.kam, DVec3::new(x, y, z), bw, bh).map(|(a, b, _)| egui::pos2(a, b))
                    }).collect()
                };
                let c = match pinsel.modus {
                    gelaende::Modus::Heben => egui::Color32::from_rgb(120, 230, 120),
                    gelaende::Modus::Senken => egui::Color32::from_rgb(255, 140, 90),
                    gelaende::Modus::Glaetten => egui::Color32::from_rgb(110, 190, 255),
                    gelaende::Modus::Ebnen => egui::Color32::from_rgb(255, 220, 80),
                };
                maler.add(egui::Shape::line(ring(pinsel.radius), egui::Stroke::new(2.5, c)));
                maler.add(egui::Shape::line(ring(pinsel.radius * if pinsel.modus == gelaende::Modus::Ebnen { 0.7 } else { 0.5 }), egui::Stroke::new(1.0, c.gamma_multiply(0.6))));
                if let Some(m) = bearbeiten::projizieren(&self.kam, g + DVec3::Z * 0.3, bw, bh).map(|(a, b, _)| egui::pos2(a, b)) {
                    maler.circle_filled(m, 3.0, c);
                    let text = match (pinsel.modus, self.gelaende.strich_ziel()) {
                        (gelaende::Modus::Ebnen, Some(z)) => format!("{:.2} m -> {z:.2} m", g.z),
                        (gelaende::Modus::Ebnen, None) => format!("{:.2} m -> {:.2} m", g.z, pinsel.zielhoehe(g.z)),
                        _ => format!("{:.2} m", g.z),
                    };
                    maler.text(m + egui::vec2(10.0, 8.0), egui::Align2::LEFT_TOP, text, egui::FontId::proportional(13.0), c);
                }
            }
            if let (Werkzeug::Welt, true, Some(v)) = (self.bearb.werkzeug, self.welt_modus.orte(), self.viewer.as_ref()) {
                let pt = |p: DVec3| bearbeiten::projizieren(&self.kam, p + DVec3::Z * 0.5, bw, bh).map(|(x, y, _)| egui::pos2(x, y));
                let jetzt: std::collections::HashMap<i64, (DVec3, f64)> = v.hidden_objects().into_iter().map(|x| (x.0, (x.1, x.2))).collect();
                let pfeil = |m: egui::Pos2, p: DVec3, rot: f64, c: egui::Color32| {
                    if let Some(q) = pt(p + (netz::dir(rot) * 6.0).extend(0.0)) {
                        maler.line_segment([m, q], egui::Stroke::new(2.5, c));
                    }
                };
                if self.welt_modus == WeltModus::Einsetzpunkte {
                    let c = egui::Color32::from_rgb(255, 200, 40);
                    for p in self.orte.punkte() {
                        let (pos, rot) = jetzt.get(&p.id).copied().unwrap_or((p.pos, p.rot));
                        if let Some(m) = pt(pos) {
                            maler.circle_filled(m, 6.0, c);
                            pfeil(m, pos, rot, c);
                            maler.text(m + egui::vec2(9.0, -9.0), egui::Align2::LEFT_BOTTOM, &p.name, egui::FontId::proportional(13.0), c);
                        }
                    }
                } else {
                    let c = egui::Color32::from_rgb(90, 170, 255);
                    for h in &orte_halte {
                        let pos = jetzt.get(&h.id).map(|x| x.0).unwrap_or(h.pos);
                        if let Some(m) = pt(pos) {
                            maler.rect_filled(egui::Rect::from_center_size(m, egui::vec2(11.0, 11.0)), 2.0, c);
                            maler.text(m + egui::vec2(9.0, -9.0), egui::Align2::LEFT_BOTTOM, &h.name, egui::FontId::proportional(13.0), c);
                        }
                    }
                }
                // Vorschau beim Setzen: auf der naechsten Fahrspur
                if self.orte_setzen {
                    if let Some(g) = self.boden_unter_maus {
                        let (pos, rot, auf_spur) = match orte::spur_bei(v, g.truncate()) { Some((p, r)) => (p, r, true), None => (g, 0.0, false) };
                        let c = if auf_spur { egui::Color32::WHITE } else { egui::Color32::from_rgb(255, 120, 90) };
                        if let Some(m) = pt(pos) {
                            maler.circle_stroke(m, 8.0, egui::Stroke::new(2.5, c));
                            pfeil(m, pos, rot, c);
                            maler.text(m + egui::vec2(12.0, 8.0), egui::Align2::LEFT_TOP, if auf_spur { "Klick setzt hier (auf der Fahrspur)" } else { "keine Fahrspur in 8 m - Klick setzt trotzdem" }, egui::FontId::proportional(13.0), c);
                        }
                    }
                }
            }
            if let (Werkzeug::Welt, true, Some(v)) = (self.bearb.werkzeug, self.welt_modus == WeltModus::Kacheln, self.viewer.as_ref()) {
                let ts = 300.0;
                let kacheln: std::collections::HashSet<(i32, i32)> = v.map_tiles().into_iter().collect();
                let ziel = self.kam.ziel;
                let r = ((self.kam.abstand * 3.0 + 600.0) / ts).ceil() as i32;
                let (cx, cy) = ((ziel.x / ts).floor() as i32, (ziel.y / ts).floor() as i32);
                let hoehe = |x: f64, y: f64| v.terrain_height(x, y).unwrap_or(ziel.z) + 0.5;
                let pt = |x: f64, y: f64| bearbeiten::projizieren(&self.kam, DVec3::new(x, y, hoehe(x, y)), bw, bh).map(|(a, b, _)| egui::pos2(a, b));
                let rand = |k: (i32, i32)| -> Vec<egui::Pos2> {
                    let (x0, y0) = (k.0 as f64 * ts, k.1 as f64 * ts);
                    let ecken = [(x0, y0), (x0 + ts, y0), (x0 + ts, y0 + ts), (x0, y0 + ts), (x0, y0)];
                    ecken.windows(2).flat_map(|w| (0..10).map(move |i| {
                        let t = i as f64 / 10.0;
                        (w[0].0 + (w[1].0 - w[0].0) * t, w[0].1 + (w[1].1 - w[0].1) * t)
                    })).chain(std::iter::once((x0, y0))).filter_map(|(x, y)| pt(x, y)).collect()
                };
                for x in cx - r..=cx + r {
                    for y in cy - r..=cy + r {
                        let k = (x, y);
                        let da = kacheln.contains(&k);
                        let frei = !da && [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().any(|d| kacheln.contains(&(x + d.0, y + d.1)));
                        if !da && !frei {
                            continue;
                        }
                        let unter = self.welt_unter == Some(k);
                        let gewaehlt = self.welt_wahl == Some(k) && da;
                        let farbe = if gewaehlt { egui::Color32::from_rgb(255, 60, 220) } else if da {
                            egui::Color32::from_rgba_unmultiplied(255, 255, 255, if unter { 255 } else { 120 })
                        } else {
                            egui::Color32::from_rgba_unmultiplied(90, 230, 120, if unter { 255 } else { 90 })
                        };
                        let linie = rand(k);
                        if linie.len() > 1 {
                            maler.add(egui::Shape::line(linie, egui::Stroke::new(if unter || gewaehlt { 3.0 } else { 1.2 }, farbe)));
                        }
                        let mx = (x as f64 + 0.5) * ts;
                        let my = (y as f64 + 0.5) * ts;
                        if let Some(m) = pt(mx, my) {
                            if frei {
                                maler.text(m, egui::Align2::CENTER_CENTER, if unter { "+ Kachel anlegen" } else { "+" }, egui::FontId::proportional(if unter { 16.0 } else { 22.0 }), farbe);
                            } else if unter || gewaehlt {
                                maler.text(m, egui::Align2::CENTER_CENTER, format!("Kachel {x} {y}"), egui::FontId::proportional(14.0), farbe);
                            }
                        }
                    }
                }
            }
            if let (true, Some(z)) = (self.pipette, self.pip_ziel.as_ref()) {
                let c = egui::Color32::from_rgb(255, 220, 60);
                let pt = |p: DVec3| bearbeiten::projizieren(&self.kam, p, bw, bh).map(|(x, y, _)| egui::pos2(x, y));
                let text = match z {
                    PipZiel::Objekt(o) => {
                        bearbeiten::markieren(&maler, &self.kam, o, bw, bh, c, 2.5);
                        o.sco.file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_default()
                    }
                    PipZiel::Strasse { sli, punkte, halb } => {
                        for seite in [-*halb, *halb] {
                            let linie: Vec<egui::Pos2> = punkte.iter().filter_map(|(q, h)| pt(*q + (crate::netz::rechts(*h) * seite).extend(0.3))).collect();
                            if linie.len() > 1 {
                                maler.add(egui::Shape::line(linie, egui::Stroke::new(2.5, c)));
                            }
                        }
                        format!("Strasse: {}", sli.rsplit('\\').next().unwrap_or(sli))
                    }
                    PipZiel::Kreisel(m, k) => {
                        let linie: Vec<egui::Pos2> = (0..=96).filter_map(|i| pt(*m + (netz::dir(i as f64 * 3.75) * k.aussen()).extend(0.3))).collect();
                        if linie.len() > 1 {
                            maler.add(egui::Shape::line(linie, egui::Stroke::new(2.5, c)));
                        }
                        "Kreisverkehr".to_string()
                    }
                };
                if let Some(m) = self.maus {
                    maler.text(egui::pos2(m.0 + 16.0, m.1 + 16.0), egui::Align2::LEFT_TOP, format!("Pipette: {text}"), egui::FontId::proportional(14.0), c);
                }
            } else if self.pipette {
                if let Some(m) = self.maus {
                    maler.text(egui::pos2(m.0 + 16.0, m.1 + 16.0), egui::Align2::LEFT_TOP, "Pipette", egui::FontId::proportional(14.0), egui::Color32::from_rgb(255, 220, 60));
                }
            }
            if let Some(an) = spur_ansicht.as_ref() {
                let pt = |p: DVec3| bearbeiten::projizieren(&self.kam, p + DVec3::Z * 0.35, bw, bh).map(|(x, y, _)| egui::pos2(x, y));
                for vb in &an.verbindungen {
                    let gewaehlt = self.spur_zufahrt == Some(vb.zufahrt);
                    let blass = self.spur_zufahrt.is_some() && !gewaehlt;
                    let (r, g, b) = if vb.erlaubt { (70, 220, 90) } else { (235, 60, 60) };
                    let farbe = if blass { egui::Color32::from_rgba_unmultiplied(r, g, b, 70) } else { egui::Color32::from_rgb(r, g, b) };
                    let linie: Vec<egui::Pos2> = vb.punkte.iter().filter_map(|p| pt(*p)).collect();
                    if linie.len() < 2 {
                        continue;
                    }
                    let n = linie.len();
                    let (a, e) = (linie[n.saturating_sub(3)], linie[n - 1]);
                    let dick = if gewaehlt { 3.5 } else { 2.0 };
                    maler.add(egui::Shape::line(linie, egui::Stroke::new(dick, farbe)));
                    let d = e - a;
                    if d.length() > 0.5 {
                        let d = d.normalized() * 9.0;
                        let q = egui::vec2(-d.y, d.x) * 0.55;
                        maler.add(egui::Shape::line(vec![e - d + q, e, e - d - q], egui::Stroke::new(dick, farbe)));
                    }
                }
                for (i, z) in an.zufahrten.iter().enumerate() {
                    if let Some(m) = pt(z.pos) {
                        if self.spur_zufahrt == Some(i) {
                            maler.circle_filled(m, 7.0, egui::Color32::from_rgb(255, 60, 220));
                        } else {
                            maler.circle_stroke(m, 6.0, egui::Stroke::new(2.0, egui::Color32::WHITE));
                        }
                    }
                }
            } else if self.bearb.werkzeug == Werkzeug::Kreuzung && self.spur_modus {
                if let Some(KreuzungsZiel::Vorhanden(k)) = &self.kreuzung_maus {
                    if let Some((x, y, _)) = bearbeiten::projizieren(&self.kam, k.pos + DVec3::Z * 0.3, bw, bh) {
                        maler.circle_stroke(egui::pos2(x, y), 10.0, egui::Stroke::new(2.5, egui::Color32::from_rgb(255, 170, 40)));
                    }
                }
            }
            if self.bearb.werkzeug == Werkzeug::Kreuzung && !self.spur_modus {
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
            if self.bearb.werkzeug == Werkzeug::Knoten {
                let pt = |p: DVec3| bearbeiten::projizieren(&self.kam, p + DVec3::Z * 0.3, bw, bh).map(|(x, y, _)| egui::pos2(x, y));
                if let Some(plan) = &self.knoten.plan {
                    let farbe = if plan.fehler.is_some() { egui::Color32::from_rgb(255, 70, 60) }
                                else if plan.warnung.is_some() { egui::Color32::from_rgb(255, 170, 40) } else { egui::Color32::from_rgb(90, 220, 255) };
                    for (punkte, w) in &plan.linien {
                        for seite in [-*w, 0.0, *w] {
                            let linie: Vec<egui::Pos2> = punkte.iter().filter_map(|(q, h)| pt(*q + (crate::netz::rechts(*h) * seite).extend(0.0))).collect();
                            if linie.len() > 1 {
                                maler.add(egui::Shape::line(linie, egui::Stroke::new(if seite == 0.0 { 1.0 } else { 2.5 }, farbe)));
                            }
                        }
                    }
                    if let Some(m) = pt(plan.ziel) {
                        maler.circle_filled(m, 6.0, farbe);
                        let mut text = String::new();
                        if self.knoten.dreh != 0.0 {
                            text.push_str(&format!("gedreht {:+.1} Grad  ", self.knoten.dreh));
                        }
                        if self.knoten.hoehe != 0.0 {
                            text.push_str(&format!("Hoehe {:+.2} m  ", self.knoten.hoehe));
                        }
                        if let Some(f) = plan.fehler.as_ref().or(plan.warnung.as_ref()) {
                            text.push_str(f);
                        }
                        if !text.is_empty() {
                            maler.text(m + egui::vec2(12.0, 10.0), egui::Align2::LEFT_TOP, text, egui::FontId::proportional(14.0), farbe);
                        }
                    }
                } else {
                    for (i, g) in self.knoten.griffe.iter().enumerate() {
                        let unter = Some(i) == self.knoten.unter_maus;
                        let Some(m) = pt(g.pos()) else { continue };
                        match g {
                            knoten::Griff::Mitte { .. } => {
                                if Some(i) == self.knoten.mitte && self.knoten.unter_maus.is_none() && self.umschalt {
                                    maler.circle_stroke(m, 6.0, egui::Stroke::new(2.0, egui::Color32::WHITE));
                                    maler.text(m + egui::vec2(10.0, 8.0), egui::Align2::LEFT_TOP, g.text(), egui::FontId::proportional(13.0), egui::Color32::WHITE);
                                }
                            }
                            knoten::Griff::Kreuzung { k, .. } => {
                                let c = if unter { egui::Color32::WHITE } else { egui::Color32::from_rgb(255, 170, 40) };
                                maler.rect_stroke(egui::Rect::from_center_size(m, egui::vec2(18.0, 18.0)), 2.0, egui::Stroke::new(2.5, c), egui::StrokeKind::Middle);
                                for a in &k.arme {
                                    if let Some(q) = pt(a.pos) {
                                        maler.line_segment([m, q], egui::Stroke::new(1.5, c));
                                    }
                                }
                                if unter {
                                    maler.text(m + egui::vec2(14.0, 10.0), egui::Align2::LEFT_TOP, "Kreuzung - Ziehen verschiebt sie mit ihren Strassen", egui::FontId::proportional(13.0), c);
                                }
                            }
                            _ => {
                                let eigen = matches!(g, knoten::Griff::Netz { .. });
                                let c = if unter { egui::Color32::WHITE } else if eigen { egui::Color32::from_rgb(120, 255, 140) } else { egui::Color32::from_rgb(90, 220, 255) };
                                maler.circle_stroke(m, if unter { 8.0 } else { 5.0 }, egui::Stroke::new(if unter { 3.0 } else { 2.0 }, c));
                                if unter {
                                    maler.text(m + egui::vec2(10.0, 8.0), egui::Align2::LEFT_TOP, g.text(), egui::FontId::proportional(13.0), c);
                                }
                            }
                        }
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
                        // Pfeile in Splinerichtung
                        let k = &s.kurve;
                        let n = (k.length / 12.0).ceil().max(1.0) as usize;
                        for i in 0..n {
                            let x = k.length * (i as f64 + 0.5) / n as f64;
                            let (q, h) = (k.point_at(x), k.heading_at(x));
                            let spitze = q + (crate::netz::dir(h) * 2.0).extend(0.3);
                            let fuss = |seite: f64| q + (crate::netz::rechts(h) * seite - crate::netz::dir(h) * 0.5).extend(0.3);
                            let pts: Vec<egui::Pos2> = [fuss(-1.5), spitze, fuss(1.5)].iter().filter_map(|p| bearbeiten::projizieren(&self.kam, *p, bw, bh).map(|(x, y, _)| egui::pos2(x, y))).collect();
                            if pts.len() == 3 {
                                maler.add(egui::Shape::line(pts, egui::Stroke::new(dick, farbe)));
                            }
                        }
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
                // Kreisverkehre: wo Zufahrten enden (Strasse hierher ziehen), in der Vorschau auch der geplante
                let mut ringe: Vec<(DVec3, netz::Kreisel, bool)> = self.strasse.kreisel().into_iter().map(|(p, k)| (p, k, false)).collect();
                if let Some((m, r)) = self.strasse.plan.as_ref().and_then(|p| p.kreisel).filter(|_| self.strasse.baut()) {
                    ringe.push((m, netz::Kreisel { r, breite: strasse::KREISEL_BREITE }, true));
                }
                for (m, kr, geplant) in ringe {
                    let c = if geplant { egui::Color32::from_rgb(255, 170, 40) } else { egui::Color32::from_rgb(90, 220, 255) };
                    for (rad, dick) in [(kr.aussen(), 2.0), (kr.arm_abstand(), 1.0)] {
                        let linie: Vec<egui::Pos2> = (0..=96).filter_map(|i| {
                            let q = m + (netz::dir(i as f64 * 3.75) * rad).extend(0.3);
                            bearbeiten::projizieren(&self.kam, q, bw, bh).map(|(x, y, _)| egui::pos2(x, y))
                        }).collect();
                        if linie.len() > 1 {
                            maler.add(egui::Shape::line(linie, egui::Stroke::new(dick, c)));
                        }
                    }
                    if let Some((x, y, _)) = bearbeiten::projizieren(&self.kam, m + DVec3::Z * 0.3, bw, bh) {
                        let t = if geplant { "Kreisverkehr (Zufahrten enden am aeusseren Kreis)" } else { "Kreisverkehr - Strasse hierher ziehen: Zufahrt" };
                        maler.text(egui::pos2(x, y), egui::Align2::CENTER_CENTER, t, egui::FontId::proportional(13.0), c);
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
        if let (Some(bk), Some(v)) = (self.baukasten.as_mut(), self.viewer.as_mut()) {
            bk.vorschau_rendern(v, &gui.ctx, &self.root);
        }
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
                if self.geaendert() > 0 {
                    self.verwerfen_frage = Some(i);
                } else {
                    self.zu_laden = Some(i);
                    self.meldung = format!("lade {} ...", self.karten[i].ordner);
                }
            }
            UiAktion::Verwerfen(i) => {
                self.verwerfen_frage = None;
                self.bearb.aenderungen = 0;
                self.welt.aenderungen = 0;
                self.gelaende.aenderungen = 0;
                self.orte.aenderungen = 0;
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
            UiAktion::SplineEinbahn(art) => {
                if let (Some(v), Some(a)) = (self.viewer.as_mut(), self.aendern.as_mut()) {
                    match a.einbahn(v, art) {
                        Ok(n) => self.meldung = format!("{n} Spline(s): {}", art.text()),
                        Err(e) => self.meldung = format!("Aendern fehlgeschlagen: {e:#}"),
                    }
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
            UiAktion::Spuren(a) => {
                if let Some(v) = self.viewer.as_mut() {
                    match a {
                        spuren::Aenderung::Netz(k, liste) => self.meldung = self.strasse.spuren_setzen(v, k, &liste),
                        spuren::Aenderung::Karte { kachel, objekt, sperren, frei } => {
                            if let Some(ae) = self.aendern.as_mut() {
                                self.meldung = match ae.objekt_pfade(v, kachel, objekt, &sperren, &frei) {
                                    Ok(()) => format!("Kreuzung der Karte: {} Pfad(e) fuer die KI gesperrt, {} frei", sperren.len(), frei.len()),
                                    Err(e) => format!("Spuren aendern fehlgeschlagen: {e:#}"),
                                };
                            }
                        }
                    }
                }
            }
            UiAktion::WeltModus(m) => {
                if let Some(v) = self.viewer.as_mut() {
                    self.gelaende.abbrechen(v);
                }
                self.welt_modus = m;
                self.orte_setzen = false;
                self.orte_umbenennen = None;
                self.gelaende_unter = None;
                if m.orte() {
                    if let (Some(v), Some(k)) = (self.viewer.as_ref(), self.karte.clone()) {
                        let liste = self.welt.anfangsliste(v);
                        self.orte.laden(&self.root.join("maps").join(k), &liste);
                    }
                }
            }
            UiAktion::PunktUmbenennen(i, name) => {
                self.orte_umbenennen = None;
                self.meldung = match self.orte.punkt_umbenennen(i, &name) {
                    Ok(()) => format!("Einsetzpunkt umbenannt: \"{name}\""),
                    Err(e) => format!("{e:#}"),
                };
            }
            UiAktion::PunktLoeschen(i) => {
                let (Some(v), Some(a)) = (self.viewer.as_mut(), self.aendern.as_mut()) else { return };
                self.meldung = self.orte.punkt_loeschen(v, a, i).unwrap_or_else(|e| format!("Einsetzpunkt nicht geloescht: {e:#}"));
            }
            UiAktion::HaltUmbenennen(h, name) => {
                self.orte_umbenennen = None;
                let (Some(v), Some(a)) = (self.viewer.as_mut(), self.aendern.as_mut()) else { return };
                self.meldung = self.orte.halt_umbenennen(v, a, &h, &name).unwrap_or_else(|e| format!("nicht umbenannt: {e:#}"));
            }
            UiAktion::HaltLoeschen(h) => {
                let (Some(v), Some(a)) = (self.viewer.as_mut(), self.aendern.as_mut()) else { return };
                self.meldung = self.orte.halt_loeschen(v, a, &h).unwrap_or_else(|e| format!("Haltestelle nicht geloescht: {e:#}"));
            }
            UiAktion::HaltAufnehmen(h) => {
                self.meldung = match self.orte.halt_aufnehmen(&h) {
                    Ok(()) => format!("\"{}\" in Busstops.cfg aufgenommen", h.name),
                    Err(e) => format!("{e:#}"),
                };
            }
            UiAktion::Hinfahren(p) => {
                self.kam.ziel = p;
                self.kam.abstand = self.kam.abstand.min(120.0);
            }
            UiAktion::KachelNeu(k) => {
                let (Some(v), Some(karte)) = (self.viewer.as_mut(), self.karte.clone()) else { return };
                let Some(a) = self.aendern.as_ref() else { return };
                let ordner = a.sitzung.join("maps").join(&karte);
                // Karte mit Ort: das Gelaende der Stelle aus den Geodaten
                let echt = match self.luftbild.bezug.as_ref() {
                    Some(b) => match geo::kacheln(b, &[k], true, false) {
                        Ok((d, _)) => d.into_iter().next().and_then(|d| d.gelaende),
                        Err(e) => {
                            log::warn!("Gelaende fuer Kachel {k:?}: {e:#}");
                            None
                        }
                    },
                    None => None,
                };
                self.meldung = match self.welt.hinzufuegen(v, &self.root, &ordner, k, echt) {
                    Ok(m) => m,
                    Err(e) => format!("Kachel nicht angelegt: {e:#}"),
                };
                self.welt_wahl = Some(k);
            }
            UiAktion::KachelLoeschen(k) => {
                self.welt_frage = None;
                let Some(v) = self.viewer.as_mut() else { return };
                self.meldung = match self.welt.loeschen(v, k) {
                    Ok(m) => m,
                    Err(e) => format!("Kachel nicht weggenommen: {e:#}"),
                };
                self.welt_wahl = None;
            }
            UiAktion::Pipette => {
                self.pipette = !self.pipette;
                self.pip_ziel = None;
                self.meldung = if self.pipette {
                    "Pipette: Klick auf ein Objekt, einen Baum, eine Strasse oder einen Kreisverkehr uebernimmt es (Esc bricht ab)".into()
                } else {
                    "Pipette aus".into()
                };
            }
            UiAktion::StrasseUebernehmen(sli) => {
                if self.bearb.werkzeug != Werkzeug::Strasse {
                    self.ausfuehren(UiAktion::Werkzeug(Werkzeug::Strasse));
                }
                if self.strasse.modus == strasse::Modus::Kreisel {
                    self.strasse.modus = strasse::Modus::Kurve;
                }
                let name = sli.rsplit('\\').next().unwrap_or(&sli).to_string();
                // im Querschnitt-Raster zeigen: Suche auf die Datei, Filter aus
                self.qs_suche = name.trim_end_matches(".sli").to_lowercase();
                self.qs_herkunft = None;
                self.qs_ordner = None;
                self.qs_spuren = None;
                self.qs_gehweg = None;
                self.strasse.sli = Some(sli.clone());
                self.pipette = false;
                self.meldung = format!("Querschnitt uebernommen: {name} - Klick setzt den Start");
            }
            UiAktion::Uebernehmen(sco, richtung) => {
                let rel = self.viewer.as_ref().map(|v| bearbeiten::relativ(&v.root, &sco)).unwrap_or_default();
                let datei = sco.file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
                if self.bearb.werkzeug != Werkzeug::Platzieren {
                    self.ausfuehren(UiAktion::Werkzeug(Werkzeug::Platzieren));
                }
                // im Katalog zeigen: Suche auf die Datei, Filter aus
                self.katalog_suche = datei.to_lowercase();
                self.katalog_ordner = None;
                self.katalog_gruppe = None;
                self.katalog_herkunft = None;
                self.katalog_editor = None;
                self.bearb.platzier_richtung = richtung.rem_euclid(360.0);
                self.bearb.unter_maus = None;
                self.pipette = false;
                self.ausfuehren(UiAktion::Platzier(Some(rel.clone())));
                self.meldung = format!("uebernommen: {datei} ({rel}) - Klick setzt es; Pipette (I) oder Strg+Klick nimmt ein anderes");
            }
            UiAktion::SpurenZurueck(k) => {
                if let Some(v) = self.viewer.as_mut() {
                    self.meldung = self.strasse.spuren_zuruecksetzen(v, k);
                }
            }
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
                self.knoten.abbrechen();
                if let Some(v) = self.viewer.as_mut() {
                    self.gelaende.abbrechen(v);
                }
                self.knoten.griffe.clear();
                self.knoten.unter_maus = None;
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
                self.pipette = false;
                self.pip_ziel = None;
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
            UiAktion::KarteNeu(n, ort, text) => {
                if self.neue_karte_job.is_some() {
                    return;
                }
                let root = self.root.clone();
                let text = if ort.is_none() { text.trim().to_string() } else { String::new() };
                self.meldung = match (&ort, text.is_empty()) {
                    (Some(o), _) => format!("lege \"{}\" an - Gelaende fuer {} wird geholt ...", n.anzeige, o.name),
                    (None, false) => format!("lege \"{}\" an - suche \"{text}\" und hole das Gelaende ...", n.anzeige),
                    (None, true) => format!("lege \"{}\" an ...", n.anzeige),
                };
                protokoll::aktion(&format!("Neue Karte: {} ({})", n.ordner, ort.as_ref().map(|o| o.name.as_str()).unwrap_or("ohne Ort")));
                self.neue_karte_job = Some((n.ordner.trim().to_string(), std::thread::spawn(move || -> Result<()> {
                    let mut n = n;
                    // Ort eingetippt, aber kein Treffer gewaehlt: der erste Treffer
                    let ort = match ort {
                        Some(o) => Some(o),
                        None if !text.is_empty() => Some(geo::suchen(&text)?.into_iter().next().with_context(|| format!("Ort \"{text}\" nicht gefunden"))?),
                        None => None,
                    };
                    if let Some(o) = ort {
                        n.bezug = Some(geo::Bezug::fuer(&o)?);
                    }
                    karten::neue_karte(&root, &n).map(|_| ())
                })));
            }
            UiAktion::OrtFestlegen(ort, gelaende) => {
                let Some(v) = self.viewer.as_ref() else { return };
                if self.ort_setzen_job.is_some() {
                    return;
                }
                let kacheln = v.map_tiles();
                self.meldung = format!("lege den Ort fest: {} ...", ort.name);
                self.ort_setzen_job = Some(std::thread::spawn(move || {
                    let b = geo::Bezug::fuer(&ort)?;
                    let daten = if gelaende { geo::kacheln(&b, &kacheln, true, false)?.0 } else { Vec::new() };
                    Ok((b, daten))
                }));
            }
            UiAktion::Baukasten(q) => {
                self.baukasten = Some(baukasten::Baukasten::neu(&self.root, q));
            }
            UiAktion::QsGespeichert(rel, q, bauen) => {
                if let Some(v) = self.viewer.as_mut() {
                    v.forget_spline_type(&rel);
                }
                if let Some(a) = self.aendern.as_mut() {
                    a.breite_vergessen(&rel);
                }
                let halb = (-q.links()).max(q.gesamtbreite() + q.links());
                self.strasse.netz.breiten.insert(rel.clone(), halb);
                let (vor, zurueck, gehwege) = q.spuren();
                let neu = strasse::Querschnitt { rel: rel.clone(), name: q.datei(), ordner: speichern::EIGEN.to_string(), vor, zurueck, gehwege,
                                                 breite: q.gesamtbreite() as f32, herkunft: strasse::EIGENE_HERKUNFT.to_string() };
                if let Some(liste) = self.querschnitte.as_mut() {
                    liste.retain(|x| !x.rel.eq_ignore_ascii_case(&rel));
                    liste.push(neu);
                }
                self.vorschau.vergessen(&rel);
                self.meldung = format!("Querschnitt \"{}\" gespeichert ({rel})", q.name);
                if bauen {
                    self.strasse.sli = Some(rel);
                    if self.bearb.werkzeug != Werkzeug::Strasse {
                        self.ausfuehren(UiAktion::Werkzeug(Werkzeug::Strasse));
                    }
                    if self.strasse.modus == strasse::Modus::Kreisel {
                        self.strasse.modus = strasse::Modus::Gerade;
                    }
                }
            }
            UiAktion::Messen => {
                self.messen = !self.messen;
                if self.messen {
                    self.mess_a = None;
                    self.mess_b = None;
                    self.pipette = false;
                }
                self.meldung = if self.messen { "Messen: Klick setzt den Anfang, zweiter Klick das Ende (Esc/M: aus)".into() } else { "Messen aus".into() };
            }
            UiAktion::OrtSuchen(text) => {
                if self.ort_job.is_none() && !text.trim().is_empty() {
                    self.meldung = format!("suche \"{}\" ...", text.trim());
                    self.ort_treffer.clear();
                    self.ort_job = Some(std::thread::spawn(move || geo::suchen(&text)));
                }
            }
            UiAktion::ObjekteHolen => {
                let Some(k) = self.karte.clone() else { return };
                match speichern::objekte_reparieren(&self.root, &k) {
                    Ok((n, fehlend, sicherung)) => {
                        self.meldung = if fehlend.is_empty() {
                            format!("{n} Objekt(e) in die Karte geholt (Sicherung der Kacheln: {})", sicherung.display())
                        } else {
                            format!("{n} Objekt(e) geholt, nicht gefunden: {} (Sicherung: {})", fehlend.join(", "), sicherung.display())
                        };
                        // frisch laden, damit die Kreuzungen erscheinen (Aenderungen gibt es keine: der Dialog kommt beim Oeffnen)
                        self.kamera_merken = Some(self.kam.clone());
                        self.speichern_meldung = Some(self.meldung.clone());
                        self.zu_laden = self.karten.iter().position(|x| x.ordner == k);
                    }
                    Err(e) => self.meldung = format!("Objekte holen fehlgeschlagen: {e:#}"),
                }
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
                if self.geaendert() == 0 {
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
                    Ok(mut paket) => {
                        paket.kacheln = self.welt.speicherliste(v);
                        paket.orte = self.orte.daten(v).map(|d| (d, v.map_tile_refs()));
                        // Objekte aus dem World Editor stehen schon in den Kacheln: [NextIDCode] hinter ihre IDs
                        if paket.orte.is_some() {
                            paket.naechste_id = Some(paket.naechste_id.unwrap_or(0).max(v.next_object_id()));
                        }
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
                    Ok(mut paket) => {
                        paket.kacheln = self.welt.speicherliste(v);
                        paket.orte = self.orte.daten(v).map(|d| (d, v.map_tile_refs()));
                        // Objekte aus dem World Editor stehen schon in den Kacheln: [NextIDCode] hinter ihre IDs
                        if paket.orte.is_some() {
                            paket.naechste_id = Some(paket.naechste_id.unwrap_or(0).max(v.next_object_id()));
                        }
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
        let (b0, s0, a0, w0, o0, g0) = self.stapel;
        let mut neu = false;
        if jetzt.1 > s0 {
            for _ in s0..jetzt.1 {
                self.verlauf.push(Quelle::Strasse);
            }
            neu = true;
        } else if jetzt.4 > o0 {
            // Einsetzpunkt/Haltestelle: die gleichzeitig geschriebenen Kacheln gehoeren zu diesem Schritt
            for _ in o0..jetzt.4 {
                self.verlauf.push(Quelle::Orte);
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
        if jetzt.5 > g0 {
            for _ in g0..jetzt.5 {
                self.verlauf.push(Quelle::Gelaende);
            }
            neu = true;
        }
        if jetzt.3 > w0 {
            for _ in w0..jetzt.3 {
                self.verlauf.push(Quelle::Welt);
            }
            neu = true;
        }
        if neu {
            self.verlauf_redo.clear();
        }
        self.stapel = jetzt;
    }

    fn stapel_jetzt(&self) -> (usize, usize, usize, usize, usize, usize) {
        (self.bearb.undo_len(), self.strasse.undo_len(), self.aendern.as_ref().map(|a| a.undo_len()).unwrap_or(0), self.welt.undo_len(), self.orte.undo_len(),
         self.gelaende.undo_len())
    }

    /// letzte Messung (waagerechte Strecke in m)
    fn messung(&self) -> Option<f64> {
        Some((self.mess_b? - self.mess_a?).truncate().length())
    }

    /// Sitzungsordner der geoeffneten Karte (Kopien geaenderter Kacheldateien)
    fn sitzungsordner(&self) -> Option<std::path::PathBuf> {
        Some(self.aendern.as_ref()?.sitzung.join("maps").join(self.karte.as_ref()?))
    }

    /// Punkt auf dem Gelaende (ohne Strassen) unter der Maus
    fn gelaende_treffer(&self) -> Option<DVec3> {
        let (v, p) = (self.viewer.as_ref()?, self.maus?);
        let (bw, bh) = self.bildgroesse();
        let (o, d) = self.kam.strahl(p.0, p.1, bw, bh);
        treffer(o, d, |x, y| v.terrain_height(x, y), 6000.0)
    }

    /// Aenderungen seit dem Oeffnen (alle Werkzeuge)
    fn geaendert(&self) -> usize {
        let aendern = self.aendern.as_ref().map(|a| a.aenderungen).unwrap_or(0);
        self.welt.aenderungen + self.orte.aenderungen + self.gelaende.aenderungen + self.strasse.aenderungen + self.bearb.aenderungen + aendern
    }

    /// letzten Schritt (welches Werkzeug auch immer) zuruecknehmen bzw. wiederholen
    fn verlauf_schritt(&mut self, zurueck: bool) {
        self.verlauf_pruefen();
        let Some(q) = (if zurueck { self.verlauf.pop() } else { self.verlauf_redo.pop() }) else { return };
        let ordner = self.sitzungsordner();
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
            Quelle::Orte => match if zurueck { self.orte.rueckgaengig(v, self.aendern.as_mut()) } else { self.orte.wiederholen(v, self.aendern.as_mut()) } {
                Ok(b) => b,
                Err(e) => {
                    self.meldung = format!("{} fehlgeschlagen: {e:#}", if zurueck { "Rueckgaengig" } else { "Wiederholen" });
                    false
                }
            },
            Quelle::Gelaende => match ordner.map(|o| if zurueck { self.gelaende.rueckgaengig(v, &o) } else { self.gelaende.wiederholen(v, &o) }) {
                Some(Ok(b)) => b,
                Some(Err(e)) => {
                    self.meldung = format!("{} fehlgeschlagen: {e:#}", if zurueck { "Rueckgaengig" } else { "Wiederholen" });
                    false
                }
                None => false,
            },
            Quelle::Welt => match if zurueck { self.welt.rueckgaengig(v) } else { self.welt.wiederholen(v) } {
                Ok(b) => b,
                Err(e) => {
                    self.meldung = format!("{} fehlgeschlagen: {e:#}", if zurueck { "Rueckgaengig" } else { "Wiederholen" });
                    false
                }
            },
        };
        if ok {
            if zurueck { self.verlauf_redo.push(q) } else { self.verlauf.push(q) }
            if q != Quelle::Objekte {
                self.meldung = format!("{} ({})", if zurueck { "rueckgaengig" } else { "wiederholt" },
                                       match q { Quelle::Strasse => "Strasse bauen", Quelle::Aendern => "Aendern", Quelle::Objekte => "Objekte", Quelle::Welt => "Kacheln", Quelle::Orte => "Einsetzpunkte/Haltestellen", Quelle::Gelaende => "Gelaende" });
            }
        }
        self.anschluesse.vergessen();
        self.eigene_auswahl.retain(|id| self.strasse.netz.kante(*id).is_some());
        self.stapel = self.stapel_jetzt();
    }

    /// Zug im Werkzeug "Knoten" mit der letzten Mausstelle neu rechnen (nach Drehen oder Hoehe)
    fn knoten_neu_rechnen(&mut self) {
        if let (Some(v), Some(a)) = (self.viewer.as_ref(), self.aendern.as_mut()) {
            self.knoten.ziehen(v, a, &self.strasse, None);
        }
    }

    /// Testlauf: ist die Strasse fuer den Knoten-Test (wieder) geladen? (nach Rueckgaengig kommt ihre Kachel aus dem
    /// Hintergrund-Streaming zurueck)
    fn testlauf_geladen(&mut self) -> bool {
        match (self.viewer.as_ref(), self.aendern.as_mut(), self.testlauf_knoten) {
            (Some(v), Some(a), Some(id)) => {
                a.aktualisieren(v);
                a.spline(id).is_some()
            }
            _ => false,
        }
    }

    /// Pipette: was liegt unter dem Bildpunkt? Objekte (auch Baeume) zuerst, wenn sie nahe am Zeiger sind, dann
    /// Kreisverkehre, eigene Strassen, Strassen der Karte
    fn pipette_suchen(&mut self, maus: (f32, f32)) -> Option<PipZiel> {
        let (bw, bh) = self.bildgroesse();
        let v = self.viewer.as_ref()?;
        if let Some(w) = self.bearb.suchen(v, &self.kam, maus, bw, bh) {
            if let Some(o) = self.bearb.objekte(v).into_iter().find(|o| o.wahl == w) {
                let nah = [0.3, 2.0, 6.0].iter().any(|dz| bearbeiten::projizieren(&self.kam, o.pos + DVec3::Z * *dz, bw, bh)
                    .is_some_and(|(x, y, _)| (x - maus.0).hypot(y - maus.1) < 18.0));
                if nah {
                    return Some(PipZiel::Objekt(o));
                }
            }
        }
        let g = self.boden_unter_maus?;
        if let Some((m, k)) = self.strasse.kreisel().into_iter().find(|(m, k)| (m.truncate() - g.truncate()).length() < k.aussen()) {
            return Some(PipZiel::Kreisel(m, k));
        }
        if let Some(id) = self.strasse.kante_unter(g.truncate()) {
            let sli = self.strasse.netz.kante(id)?.sli.clone();
            let (punkte, halb) = self.strasse.kante_umriss(id)?;
            return Some(PipZiel::Strasse { sli, punkte, halb });
        }
        let a = self.aendern.as_mut()?;
        a.aktualisieren(v);
        let id = a.suchen(v, g.truncate())?;
        let s = a.spline(id)?.clone();
        let (l, r) = a.breite(v, &s.sli);
        Some(PipZiel::Strasse { sli: s.sli.clone(), punkte: s.punkte(), halb: l.max(r) as f64 })
    }

    /// Pipette: Klick - uebernehmen, was unter der Maus liegt
    fn pipette_klicken(&mut self) {
        let ziel = self.maus.and_then(|m| self.pipette_suchen(m));
        self.pip_ziel = None;
        match ziel {
            Some(PipZiel::Objekt(o)) => self.ausfuehren(UiAktion::Uebernehmen(o.sco.clone(), o.richtung)),
            Some(PipZiel::Strasse { sli, .. }) => self.ausfuehren(UiAktion::StrasseUebernehmen(sli)),
            Some(PipZiel::Kreisel(_, k)) => {
                if self.bearb.werkzeug != Werkzeug::Strasse {
                    self.ausfuehren(UiAktion::Werkzeug(Werkzeug::Strasse));
                }
                self.ausfuehren(UiAktion::StrassenModus(strasse::Modus::Kreisel));
                self.pipette = false;
                self.meldung = format!("Kreisverkehr (Durchmesser {:.0} m) - Klick setzt die Mitte des neuen", 2.0 * k.r);
            }
            None => self.meldung = "Pipette: hier liegt nichts - Objekt, Baum, Strasse oder Kreisverkehr anklicken (Esc bricht ab)".into(),
        }
    }

    /// Spurpfeile/Verbinder der gewaehlten Kreuzung
    fn spur_ansicht(&self) -> Option<spuren::Ansicht> {
        match self.spur_ziel? {
            spuren::Ziel::Netz(k) => spuren::Ansicht::netz(&self.strasse, k),
            spuren::Ziel::Karte { kachel, objekt } => spuren::Ansicht::karte(self.viewer.as_ref()?, kachel, objekt),
        }
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
        self.knoten = knoten::Knotenwerkzeug::default();
        self.spur_ziel = None;
        self.spur_zufahrt = None;
        self.welt = welt::Welt::default();
        self.gelaende.zuruecksetzen();
        self.gelaende_unter = None;
        self.orte = orte::Orte::default();
        self.orte_setzen = false;
        self.orte_umbenennen = None;
        self.welt_wahl = None;
        self.welt_frage = None;
        if let Some(v) = self.viewer.as_mut() {
            self.hilfe.leeren(v);
        }
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
            KeyCode::KeyI if !self.strg => Some(UiAktion::Pipette),
            KeyCode::KeyM if !self.strg => Some(UiAktion::Messen),
            KeyCode::Escape if self.messen => Some(UiAktion::Messen),
            KeyCode::Delete if self.bearb.werkzeug == Werkzeug::Welt && self.welt_wahl.is_some() => {
                self.welt_frage = self.welt_wahl;
                None
            }
            KeyCode::Escape if self.pipette => {
                self.pipette = false;
                self.meldung = "Pipette aus".into();
                None
            }
            KeyCode::KeyH if !self.strg => {
                self.hilfe.an = !self.hilfe.an;
                self.meldung = if self.hilfe.an { "Hilfsansicht: Pfade und unsichtbare Objekte".into() } else { "Hilfsansicht aus".into() };
                None
            }
            KeyCode::KeyN if !self.strg => {
                let w = if self.bearb.werkzeug == Werkzeug::Knoten { Werkzeug::Ansehen } else { Werkzeug::Knoten };
                Some(UiAktion::Werkzeug(w))
            }
            KeyCode::Escape if self.bearb.werkzeug == Werkzeug::Knoten => {
                self.knoten.abbrechen();
                self.meldung = "Knoten ziehen abgebrochen".into();
                None
            }
            KeyCode::PageUp | KeyCode::PageDown | KeyCode::Home if self.bearb.werkzeug == Werkzeug::Knoten => {
                if self.knoten.zieht() {
                    self.knoten.hoehe = match k {
                        KeyCode::Home => 0.0,
                        KeyCode::PageUp => self.knoten.hoehe + if fein { 0.25 } else { 1.0 },
                        _ => self.knoten.hoehe - if fein { 0.25 } else { 1.0 },
                    };
                    self.knoten_neu_rechnen();
                }
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

/// Knoepfe fuer die Fahrtrichtungen der KI (Einbahn) -> gewaehlte Art
fn einbahn_knoepfe(ui: &mut egui::Ui, stand: Option<netz::Einbahn>) -> Option<netz::Einbahn> {
    let mut wahl = None;
    ui.label(egui::RichText::new("Fahrtrichtung der KI (Optik bleibt)").strong());
    ui.horizontal(|ui| {
        for (art, t) in [(netz::Einbahn::Beide, "beide"), (netz::Einbahn::Vor, "Einbahn \u{2192} Pfeil"), (netz::Einbahn::Zurueck, "Einbahn \u{2190} gegen")] {
            if ui.selectable_label(stand == Some(art), t).on_hover_text(art.text()).clicked() {
                wahl = Some(art);
            }
        }
    });
    if ui.selectable_label(stand == Some(netz::Einbahn::Gesperrt), "\u{26D4} fuer KI-Verkehr sperren")
        .on_hover_text("alle Fahrspuren und die Kreuzungspfade hinein bekommen [rule] no_cars; nochmal klicken gibt frei").clicked() {
        // zweiter Klick auf die aktive Sperre gibt wieder frei
        wahl = Some(if stand == Some(netz::Einbahn::Gesperrt) { netz::Einbahn::Beide } else { netz::Einbahn::Gesperrt });
    }
    if stand.is_none() {
        ui.label(egui::RichText::new("Spuren teilweise gesperrt (Regeln der Karte)").small().weak());
    }
    ui.label(egui::RichText::new("Einbahn: die Spuren der Gegenrichtung bekommen [rule] no_cars - die KI faehrt dort nicht, der Spieler schon. Der Pfeil im Bild zeigt die Splinerichtung; mit H sieht man gesperrte Spuren rot.").small().weak());
    wahl
}

/// Maler fuer Markierungen ueber dem 3D-Bild: auf die freie Bildflaeche zwischen den Leisten zugeschnitten (sonst
/// liefen Linien wie die der Pfad-Ansicht ueber Werkzeugleiste und Seitenleisten)
fn ui_maler(ui: &egui::Ui) -> egui::Painter {
    let frei = ui.available_rect_before_wrap();
    ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("auswahl"))).with_clip_rect(frei)
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
                        if let Some(bk) = self.baukasten.as_ref() {
                            println!("Testlauf Baukasten: Vorschau {}, {} Teile, Messung {:?}", if bk.hat_vorschau() { "da" } else { "fehlt" }, bk.q.teile.len(), self.messung());
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
                            self.testlauf_knoten = gefunden;
                        } else {
                            println!("Testlauf Aendern: keine Strasse am Blickpunkt");
                        }
                        println!("Testlauf Strasse: {} Knoten, {} Kanten | {}", self.strasse.netz.knoten.len(), self.strasse.netz.kanten.len(), meldungen.join(" | "));
                        println!("Testlauf Kopie: {:?}, Katalog {} Objekte, platziert: {:?}",
                                 kopie, self.katalog.as_ref().map(|k| k.eintraege.len()).unwrap_or(0), platziert);
                        println!("Testlauf Bearbeiten: {} Objekte, Objekt {:?} gewaehlt, Aenderungen {}, Meldung: {}",
                                 self.viewer.as_ref().map(|v| v.objects().len()).unwrap_or(0), gefunden, self.bearb.aenderungen, self.meldung);
                    } else if s > t * 0.8 && self.baukasten.is_none() && self.viewer.is_some() {
                        // Baukasten oeffnen (Fenster, Skizze, 3D-Vorschau) und eine Messung von 7 m
                        self.ausfuehren(UiAktion::Baukasten(None));
                        self.mess_a = Some(self.kam.ziel);
                        self.mess_b = Some(self.kam.ziel + DVec3::new(7.0, 0.0, 0.5));
                    } else if s > t * 0.33 && self.testlauf_knoten.is_some() && (s > t * 0.5 || self.testlauf_geladen()) {
                        // Knoten: Stelle auf der Strasse vom Aendern-Test 3 m zur Seite ziehen, dann rueckgaengig (die
                        // Kachel ist inzwischen wieder geladen)
                        let id = self.testlauf_knoten.take().unwrap();
                        self.hilfe.an = true;
                        self.ausfuehren(UiAktion::Werkzeug(Werkzeug::Knoten));
                        let mut m = String::from("kein Griff");
                        if let (Some(v), Some(a)) = (self.viewer.as_mut(), self.aendern.as_mut()) {
                            a.aktualisieren(v);
                            if let Some(sp) = a.spline(id).cloned() {
                                let p = sp.kurve.point_at(sp.kurve.length / 2.0);
                                self.knoten.suchen(v, a, &self.strasse, p, 1.0, 80.0);
                                if self.knoten.greifen(p, true) {
                                    let q = p.truncate() + crate::netz::rechts(sp.kurve.heading_at(sp.kurve.length / 2.0)) * 1.0;
                                    self.knoten.ziehen(v, a, &self.strasse, Some(q.extend(p.z)));
                                    m = self.knoten.loslassen(v, a, &mut self.strasse).unwrap_or_default();
                                }
                            } else {
                                m = format!("Spline {id} nicht geladen");
                            }
                        }
                        self.verlauf_pruefen();
                        self.ausfuehren(UiAktion::Rueckgaengig);
                        println!("Testlauf Knoten: {m} | {}", self.meldung);
                    } else if s > t * 0.36 && self.testlauf_abzweig.is_some() {
                        let id = self.testlauf_abzweig.take().unwrap();
                        if let Some(v) = self.viewer.as_ref() {
                            let n = hilfsansicht::pfade(v, self.kam.ziel, 1500.0).len();
                            println!("Testlauf Hilfsansicht: {n} Pfade, {} unsichtbare Objekte ({} mit Modell gezeigt)", self.hilfe.objekte.len(),
                                     self.hilfe.objekte.iter().filter(|o| o.3).count());
                        }
                        self.hilfe.an = false;
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
                    if self.pipette && !egui_will {
                        self.pip_ziel = self.pipette_suchen(p);
                    }
                    if self.bearb.werkzeug == Werkzeug::Welt {
                        self.welt_unter = if egui_will { None } else { self.boden_unter_maus.map(|g| ((g.x / 300.0).floor() as i32, (g.y / 300.0).floor() as i32)) };
                        if self.welt_modus == WeltModus::Gelaende && !self.gelaende.malt() {
                            self.gelaende_unter = if egui_will { None } else { self.gelaende_treffer() };
                        }
                    }
                    // Pipette im Platzieren: ohne gewaehltes Objekt oder mit Strg zeigt es das Objekt unter der Maus
                    if self.bearb.werkzeug == Werkzeug::Platzieren {
                        self.bearb.unter_maus = if !egui_will && !self.pipette && (self.platzier.is_none() || self.strg) {
                            self.viewer.as_ref().and_then(|v| self.bearb.suchen(v, &self.kam, p, bw, bh))
                        } else {
                            None
                        };
                    }
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
                    if self.bearb.werkzeug == Werkzeug::Knoten {
                        let fang = (12.0 * self.kam.m_pro_px(bh)).max(1.5);
                        let umkreis = (self.kam.abstand * 0.6).clamp(60.0, 400.0);
                        if let (Some(g), Some(v), Some(a)) = (self.boden_unter_maus, self.viewer.as_ref(), self.aendern.as_mut()) {
                            if self.knoten.zieht() {
                                self.knoten.ziehen(v, a, &self.strasse, Some(g));
                            } else if !egui_will {
                                self.knoten.suchen(v, a, &self.strasse, g, fang, umkreis);
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
                if state == ElementState::Pressed && !egui_will && button == MouseButton::Left && self.messen {
                    if let Some(g) = self.boden_unter_maus {
                        if self.mess_a.is_none() || self.mess_b.is_some() {
                            self.mess_a = Some(g);
                            self.mess_b = None;
                        } else {
                            self.mess_b = Some(g);
                            if let Some(m) = self.messung() {
                                self.meldung = format!("Messung: {m:.2} m");
                            }
                        }
                    }
                    return;
                }
                if state == ElementState::Pressed && !egui_will && button == MouseButton::Left && self.pipette {
                    self.pipette_klicken();
                    return;
                }
                if state == ElementState::Pressed && !egui_will {
                    if matches!(button, MouseButton::Right | MouseButton::Middle) {
                        if let Some(p) = self.maus {
                            self.ziehen = Some((button, p));
                        }
                    }
                    if button == MouseButton::Right {
                        self.rechts_start = self.maus;
                    }
                    if button == MouseButton::Left && self.bearb.werkzeug == Werkzeug::Welt && self.welt_modus == WeltModus::Gelaende {
                        if let Some(g) = self.gelaende_treffer() {
                            let p = &mut self.gelaende.pinsel;
                            if self.strg && p.modus == gelaende::Modus::Ebnen {
                                p.fest = true;
                                p.ziel = (g.z * 100.0).round() / 100.0;
                                self.meldung = format!("Zielhoehe abgegriffen: {:.2} m (eingerastet {:.2} m)", p.ziel, p.zielhoehe(p.ziel));
                            } else {
                                self.gelaende.ansetzen(g.z, self.strg);
                                self.gelaende_unter = Some(g);
                            }
                        }
                    }
                    if button == MouseButton::Left && self.bearb.werkzeug == Werkzeug::Welt && self.welt_modus.orte() && self.orte_setzen {
                        if let (Some(g), Some(v), Some(a)) = (self.boden_unter_maus, self.viewer.as_mut(), self.aendern.as_mut()) {
                            let (pos, rot) = orte::spur_bei(v, g.truncate()).unwrap_or((g, 0.0));
                            let punkte = self.welt_modus == WeltModus::Einsetzpunkte;
                            let name = if self.orte_name.trim().is_empty() {
                                if punkte { format!("Einsetzpunkt {}", self.orte.punkte().len() + 1) } else { "Neue Haltestelle".to_string() }
                            } else { self.orte_name.trim().to_string() };
                            let erg = if punkte { self.orte.punkt_neu(v, a, pos, rot, &name) } else { self.orte.halt_neu(v, a, pos, rot, &name) };
                            self.meldung = erg.unwrap_or_else(|e| format!("nicht gesetzt: {e:#}"));
                        }
                        self.orte_setzen = false;
                    }
                    if button == MouseButton::Left && self.bearb.werkzeug == Werkzeug::Welt && self.welt_modus == WeltModus::Kacheln {
                        if let (Some(k), Some(v)) = (self.welt_unter, self.viewer.as_ref()) {
                            let kacheln = v.map_tiles();
                            if kacheln.contains(&k) {
                                self.welt_wahl = Some(k);
                            } else if [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().any(|d| kacheln.contains(&(k.0 + d.0, k.1 + d.1))) {
                                self.ausfuehren(UiAktion::KachelNeu(k));
                            } else {
                                self.welt_wahl = None;
                            }
                        }
                    }
                    if button == MouseButton::Left && self.bearb.werkzeug == Werkzeug::Kreuzung && self.spur_modus {
                        let (bw, bh) = self.bildgroesse();
                        let kam = self.kam.clone();
                        let bild = |p: DVec3| bearbeiten::projizieren(&kam, p + DVec3::Z * 0.35, bw, bh).map(|(x, y, _)| glam::DVec2::new(x as f64, y as f64));
                        let maus = self.maus.map(|m| glam::DVec2::new(m.0 as f64, m.1 as f64)).unwrap_or_default();
                        let mut erledigt = false;
                        if let Some(an) = self.spur_ansicht() {
                            if let Some(z) = an.zufahrt_bei(bild, maus) {
                                self.spur_zufahrt = Some(z);
                                erledigt = true;
                            } else if let Some(i) = an.verbindung_bei(bild, maus, self.spur_zufahrt) {
                                if let Some(a) = an.schalten(i) {
                                    self.ausfuehren(UiAktion::Spuren(a));
                                }
                                erledigt = true;
                            }
                        }
                        if !erledigt {
                            let neu = match self.kreuzung_maus.clone() {
                                Some(KreuzungsZiel::Netz(k)) => Some(spuren::Ziel::Netz(k)),
                                Some(KreuzungsZiel::Vorhanden(k)) => Some(spuren::Ziel::Karte { kachel: k.kachel, objekt: k.objekt }),
                                None => None,
                            };
                            if neu != self.spur_ziel {
                                self.spur_zufahrt = None;
                            }
                            self.spur_ziel = neu;
                        }
                    }
                    if button == MouseButton::Left && self.bearb.werkzeug == Werkzeug::Kreuzung && !self.spur_modus {
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
                    if button == MouseButton::Left && self.bearb.werkzeug == Werkzeug::Knoten {
                        if let Some(g) = self.boden_unter_maus {
                            if self.knoten.greifen(g, self.umschalt) {
                                self.knoten_neu_rechnen();
                            }
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
                    // Pipette: Objekt unter der Maus zum Platzieren uebernehmen
                    let pipette = if button == MouseButton::Left && self.bearb.werkzeug == Werkzeug::Platzieren && (self.platzier.is_none() || self.strg) {
                        let (bw, bh) = self.bildgroesse();
                        match (self.viewer.as_ref(), self.maus) {
                            (Some(v), Some(m)) => self.bearb.suchen(v, &self.kam, m, bw, bh).and_then(|w| self.bearb.objekte(v).into_iter().find(|o| o.wahl == w)),
                            _ => None,
                        }
                    } else {
                        None
                    };
                    if let Some(o) = pipette {
                        self.ausfuehren(UiAktion::Uebernehmen(o.sco.clone(), o.richtung));
                    } else if button == MouseButton::Left && self.bearb.werkzeug == Werkzeug::Platzieren {
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
                    if button == MouseButton::Left && self.gelaende.malt() {
                        let ordner = self.sitzungsordner();
                        if let (Some(v), Some(o)) = (self.viewer.as_mut(), ordner) {
                            match self.gelaende.loslassen(v, &o) {
                                Ok(Some(m)) => self.meldung = m,
                                Ok(None) => {}
                                Err(e) => self.meldung = format!("Gelaende nicht geschrieben: {e:#}"),
                            }
                        }
                    }
                    if button == MouseButton::Left && self.knoten.zieht() {
                        if let (Some(v), Some(a)) = (self.viewer.as_mut(), self.aendern.as_mut()) {
                            if let Some(m) = self.knoten.loslassen(v, a, &mut self.strasse) {
                                self.meldung = m;
                            }
                        }
                        self.anschluesse.vergessen();
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
            WindowEvent::MouseWheel { delta, .. } if !egui_will && (self.strg || self.umschalt) && self.bearb.werkzeug == Werkzeug::Welt && self.welt_modus == WeltModus::Gelaende => {
                let y = match delta {
                    MouseScrollDelta::LineDelta(x, y) => if y != 0.0 { y } else { x },
                    MouseScrollDelta::PixelDelta(p) => (if p.y != 0.0 { p.y } else { p.x }) as f32 / 60.0,
                };
                let p = &mut self.gelaende.pinsel;
                if self.strg {
                    p.radius = (p.radius * 1.15f64.powf(y as f64)).clamp(gelaende::RADIUS.0, gelaende::RADIUS.1).round();
                    self.meldung = format!("Pinsel: Radius {:.0} m", p.radius);
                } else {
                    p.staerke = ((p.staerke + 0.05 * y as f64) * 20.0).round().clamp(1.0, 20.0) / 20.0;
                    self.meldung = format!("Pinsel: Staerke {:.0} %", p.staerke * 100.0);
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
            WindowEvent::MouseWheel { delta, .. } if self.knoten.zieht() => {
                let y = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 60.0,
                };
                let schritt = if self.umschalt { 0.5 } else { 3.0 };
                self.knoten.dreh = (self.knoten.dreh - y as f64 * schritt).clamp(-90.0, 90.0);
                self.knoten_neu_rechnen();
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
