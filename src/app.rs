//! The AniCraft shell: view state plus the frame loop. Document changes go through
//! `Session::execute`, so everything here is also reachable headless.

use crate::a11y::Named;
use crate::theme::{self, T};
use crate::{autosave, dialogs, keymap, menus, motion_editor, panels, pathtools, platform, prefs, stage, timeline};
use std::collections::HashSet;
use anicraft_engine::{Session, commands};
use egui::{Key, Modifiers};
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Select,
    Subselect,
    Transform,
    Width,
    Pen,
    Text,
    Pencil,
    Brush,
    Eraser,
    Rect,
    Oval,
    Star,
    Line,
    PaintBucket,
    InkBottle,
    Eyedropper,
    Camera,
    Hand,
    Zoom,
}

/// Paint Bucket gap size: how wide an opening in an outline still counts as closed. In screen
/// points, so it feels the same at any zoom.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GapSize {
    #[default]
    None,
    Small,
    Medium,
    Large,
}

impl GapSize {
    pub const ALL: [GapSize; 4] = [GapSize::None, GapSize::Small, GapSize::Medium, GapSize::Large];
    pub fn label(self) -> &'static str {
        match self {
            GapSize::None => "Don't close gaps",
            GapSize::Small => "Close small gaps",
            GapSize::Medium => "Close medium gaps",
            GapSize::Large => "Close large gaps",
        }
    }
    pub fn screen_px(self) -> f32 {
        match self {
            GapSize::None => 0.0,
            GapSize::Small => 4.0,
            GapSize::Medium => 10.0,
            GapSize::Large => 20.0,
        }
    }
}

impl Tool {
    pub const ALL: [Tool; 19] = [Tool::Select, Tool::Subselect, Tool::Transform, Tool::Width, Tool::Pen, Tool::Text, Tool::Pencil, Tool::Brush, Tool::Eraser, Tool::Rect, Tool::Oval, Tool::Star, Tool::Line, Tool::PaintBucket, Tool::InkBottle, Tool::Eyedropper, Tool::Camera, Tool::Hand, Tool::Zoom];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "Selection",
            Tool::Subselect => "Subselection",
            Tool::Transform => "Free Transform",
            Tool::Width => "Width",
            Tool::Pen => "Pen",
            Tool::Text => "Text",
            Tool::Pencil => "Pencil",
            Tool::Brush => "Brush",
            Tool::Eraser => "Eraser",
            Tool::Rect => "Rectangle",
            Tool::Oval => "Oval",
            Tool::Star => "PolyStar",
            Tool::Line => "Line",
            Tool::PaintBucket => "Paint Bucket",
            Tool::InkBottle => "Ink Bottle",
            Tool::Eyedropper => "Eyedropper",
            Tool::Camera => "Camera",
            Tool::Hand => "Hand",
            Tool::Zoom => "Zoom",
        }
    }
    /// Default shortcut in the command-registry notation; the live key is in `App::keymap`.
    pub fn shortcut(self) -> Option<&'static str> {
        match self {
            Tool::Select => Some("V"),
            Tool::Subselect => Some("A"),
            Tool::Transform => Some("Q"),
            Tool::Width => Some("U"),
            Tool::Pen => Some("P"),
            Tool::Text => Some("T"),
            Tool::Pencil => Some("Shift+Y"),
            Tool::Brush => Some("B"),
            Tool::Eraser => Some("E"),
            Tool::Rect => Some("R"),
            Tool::Oval => Some("O"),
            Tool::Star => None,
            Tool::Line => Some("N"),
            Tool::PaintBucket => Some("K"),
            Tool::InkBottle => Some("S"),
            Tool::Eyedropper => Some("I"),
            Tool::Camera => Some("C"),
            Tool::Hand => Some("H"),
            Tool::Zoom => Some("Z"),
        }
    }
    pub fn hint(self) -> &'static str {
        match self {
            Tool::Select => "Click to select, drag to move, drag on empty space to marquee-select. Shift adds to the selection.",
            Tool::Subselect => "Click a shape to show its points. Drag points or handles to reshape (Shift adds to the point selection, drag on empty space to marquee points). Double-click an edge to add a point; Delete removes points. Right-click to make a point smooth or a corner. A motion path's points edit the same way (its ends stay).",
            Tool::Pen => "Click to place corner points, drag to pull smooth handles (Alt moves one handle). Click the first point to close; Enter, Esc or a click on the last point finishes. Backspace removes the last point. Alt-click a selected shape or a motion path to add a point to it.",
            Tool::Text => "Click to type new text, or click text to edit it. Esc or a click elsewhere finishes. The options set the font, size and colour; double-click text with the Selection tool to edit it too.",
            Tool::Pencil => "Drag to draw freehand. The options bar sets Smooth, Straighten or Ink.",
            Tool::Brush => "Drag to paint filled shapes in the fill colour (a click paints a dab). [ and ] change the size; the options set the shape, smoothing and paint mode.",
            Tool::Eraser => "Drag to erase lines and fills drawn with Object Drawing off (drawing objects and symbols are kept). [ and ] change the size; the options set the shape and erase mode, and the Faucet deletes a whole fill or line with a click. Double-click the tool to erase everything on the stage.",
            Tool::Width => "Hover a stroke to see its width points. Drag a point's handle to widen or narrow the stroke there (Alt: one side only), drag its dot to slide it along, or drag anywhere on the stroke to add a point. Click a point and press Delete to remove it.",
            Tool::Transform => "Drag a corner or edge to scale (Shift keeps proportions, Alt scales from the centre), just outside a corner to rotate (Shift snaps 15°), inside to move.",
            Tool::Rect => "Drag to draw a rectangle. Shift draws a square.",
            Tool::Oval => "Drag to draw an oval. Shift draws a circle.",
            Tool::Star => "Drag from the centre to draw a star.",
            Tool::Line => "Drag to draw a line. Shift snaps to 45°.",
            Tool::PaintBucket => "Click inside an outline to fill it (on the layer's lines), or on a fill to recolour it. The gap size closes small openings.",
            Tool::InkBottle => "Click a shape to give it the current stroke colour and width.",
            Tool::Eyedropper => "Click a fill to pick its colour, gradient or bitmap (then pour it with the Paint Bucket), or a line to pick its colour, width and profile (then apply it with the Ink Bottle). With shapes selected, the picked style goes onto them. Shift-click picks into the other swatch only.",
            Tool::Camera => "Drag on the stage to pan the camera, drag the round handle to turn it, and use the wheel to zoom (each drag is one undo step). Picking the tool adds a camera if the scene has none.",
            Tool::Hand => "Drag to pan the stage. Space works with any tool.",
            Tool::Zoom => "Click to zoom in, Alt-click to zoom out. Ctrl+wheel zooms anywhere.",
        }
    }
}

/// What the timeline dock shows: the timeline, or the Motion Editor beside it (as in Animate).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BottomTab {
    #[default]
    Timeline,
    MotionEditor,
}

/// Text being typed on the stage (view state until it is committed as one command).
pub struct TextEditing {
    /// Top-left of the text box, in stage space.
    pub at: egui::Pos2,
    /// The text element being edited, or `None` for new text.
    pub element: Option<anicraft_engine::doc::ElementId>,
    pub text: String,
    /// The text when editing started (unchanged text commits nothing).
    pub original: String,
    /// Focus the field on its first frame.
    pub focus: bool,
}

/// Stage camera.
pub struct StageView {
    pub zoom: f32,
    pub pan: egui::Vec2,
    pub fit: bool,
    /// The canvas size and zoom of the last fit: while the view is still as fitted, it fits
    /// again when the canvas changes size (window resized, panels or interface scale changed).
    pub fitted: Option<(egui::Vec2, f32)>,
}

/// In-progress pointer gesture on the stage.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Drag {
    None,
    Move { last: egui::Pos2 },
    Marquee { start: egui::Pos2, cur: egui::Pos2 },
    Shape { start: egui::Pos2, cur: egui::Pos2 },
    /// Subselection: dragging the selected points (stage position of the last frame).
    Points { last: egui::Pos2 },
    /// Subselection: marquee over the selected shapes' anchors.
    PointMarquee { start: egui::Pos2, cur: egui::Pos2 },
    /// Pencil stroke; the samples are in `App::pencil`.
    Pencil,
    /// Brush stroke; the samples are in `App::brush_samples`.
    Brush,
    /// Eraser stroke; the samples are in `App::brush_samples` too (ADR 0034).
    Eraser,
    /// Width tool: resizing (or sliding) width point `index` of shape `id`, found on subpath
    /// `sub`; `start` is its left and right width when the drag began.
    Width { id: anicraft_engine::doc::ElementId, sub: usize, index: usize, grip: crate::widthtool::Grip, start: (f32, f32) },
    /// Free transform: `applied` is the transform already sent to the engine this gesture.
    Transform { kind: XfKind, start: egui::Pos2, bounds: [f32; 4], applied: anicraft_engine::geom::Affine },
    /// Dragging shape hint `index` on the side the frame shows (ADR 0023).
    Hint { index: usize, side: anicraft_engine::doc::HintSide },
    /// Editing the motion path (ADR 0030) from stage point `start`; the path as it was when the
    /// drag began is `App::motion_drag`.
    MotionPath { grab: crate::motion_path::Grab, start: egui::Pos2 },
    /// Moving the whole motion path (ADR 0038), from stage point `last` last frame.
    MotionMove { last: egui::Pos2 },
    /// Camera tool (ADR 0027): panning from screen point `last`, turning from pointer angle
    /// `last` (radians about the camera centre), or zooming with the wheel until `until`
    /// (seconds; the turns of one burst are one undo step).
    CameraPan { last: egui::Pos2 },
    CameraRotate { last: f32 },
    CameraWheel { until: f64 },
    Pan,
}

/// What a free-transform drag does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum XfKind {
    Move,
    /// Handle position: -1/0/1 on each axis (corner, edge or centre).
    Scale { hx: i8, hy: i8 },
    Rotate,
}

/// A small text prompt standing in for native file dialogs (M0).
pub struct Prompt {
    pub title: &'static str,
    pub cmd: &'static str,
    pub key: &'static str,
    pub text: String,
}

pub struct Status {
    pub text: String,
    pub error: bool,
    pub at: f64,
}

/// `--screenshot` state.
pub struct Shot {
    pub path: PathBuf,
    pub frames: u32,
    pub requested: bool,
}

pub const SHOT_FRAMES: u32 = 120;

/// The item picked in the Library panel (view state).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibraryItem {
    Symbol(anicraft_engine::doc::SymbolId),
    Bitmap(anicraft_engine::doc::BitmapId),
    Sound(anicraft_engine::doc::SoundId),
}

impl LibraryItem {
    /// The first item in the Library: its first symbol, else its first bitmap, else sound.
    pub fn first(doc: &anicraft_engine::doc::Document) -> Option<LibraryItem> {
        doc.library.first().map(|s| LibraryItem::Symbol(s.id)).or_else(|| doc.bitmaps.first().map(|b| LibraryItem::Bitmap(b.id))).or_else(|| doc.sounds.first().map(|s| LibraryItem::Sound(s.id)))
    }
}

pub struct App {
    pub s: Session,
    pub tool: Tool,
    pub view: StageView,
    pub stage_cache: stage::Cache,
    pub drag: Drag,
    /// The shape hint a right-click landed on (its menu is showing).
    pub hint_menu: Option<usize>,
    pub playing: bool,
    pub looping: bool,
    play_acc: f64,
    pub onion: bool,
    /// View › Preview Through Camera: the stage shows what the movie shows (ADR 0027).
    pub camera_preview: bool,
    /// The tool in hand last frame (picking the Camera tool adds a camera).
    pub last_tool: Tool,
    pub status: Option<Status>,
    pub prompt: Option<Prompt>,
    pub show_about: bool,
    pub show_files: bool,
    pub show_licenses: bool,
    pub shot: Option<Shot>,
    pub cursor_stage: Option<egui::Pos2>,
    pub tl_scroll: f32,
    /// Timeline frame width (zoom), in points.
    pub tl_cell_w: f32,
    /// Timeline vertical scroll, in rows.
    pub tl_vscroll: f32,
    pub tl_last_layer: Option<anicraft_engine::doc::LayerId>,
    /// Folders shown closed on the timeline (view state: not saved, not undoable).
    pub tl_collapsed: HashSet<anicraft_engine::doc::LayerId>,
    pub library_sel: Option<LibraryItem>,
    /// Text being typed on the stage.
    pub text_edit: Option<TextEditing>,
    /// Font families on this computer, listed once on first need.
    pub font_families: Option<Vec<String>>,
    pub star_points: u32,
    pub rect_radius: f32,
    /// Pen path being drawn (view state until it is committed as one command).
    pub pen: Option<pathtools::PenPath>,
    /// Pencil samples of the stroke in progress (stage space).
    pub pencil: Vec<anicraft_engine::geom::Vec2>,
    pub pencil_mode: pathtools::PencilMode,
    /// Brush tool options, the samples of the stroke in progress (stage space, pen pressure
    /// when there is one) and the pen's last pressure (ADR 0033).
    pub brush: crate::brushtool::BrushOptions,
    pub brush_samples: Vec<(anicraft_engine::geom::Vec2, Option<f32>)>,
    pub pen_pressure: Option<f32>,
    /// Eraser tool options (ADR 0034); its stroke's samples are `brush_samples`.
    pub eraser: crate::erasertool::EraserOptions,
    pub bucket_gap: GapSize,
    /// Align panel: align to the stage rather than the selection's bounds.
    pub align_to_stage: bool,
    /// Subselection: selected anchors and handles.
    pub points: Vec<pathtools::PointRef>,
    /// Width tool: the selected width point (shape, index).
    pub width_sel: Option<(anicraft_engine::doc::ElementId, usize)>,
    /// Subselection: the selected motion path anchors, whose handles show (ADR 0030, 0038).
    pub motion_anchors: Vec<usize>,
    /// The motion path as it was when the drag on it began.
    pub motion_drag: Option<anicraft_engine::geom::Path>,
    /// Selected frames on the timeline, and the drag on the frame grid.
    pub frame_sel: Option<timeline::FrameSel>,
    pub tl_drag: timeline::TlDrag,
    /// Timeline or Motion Editor, and the Motion Editor's view state: the property whose curve
    /// it shows, the picked key (frames into the span), its drag and its list scroll.
    pub bottom: BottomTab,
    pub me_prop: anicraft_engine::doc::Property,
    pub me_key: Option<u32>,
    pub me_drag: Option<motion_editor::MeDrag>,
    pub me_vscroll: f32,
    /// The frame grid was clicked more recently than the stage: Ctrl+X/C/V act on frames.
    pub timeline_focus: bool,
    /// A destructive command waiting for the user's decision (ADR 0004).
    pub confirm: Option<dialogs::PendingConfirm>,
    /// Undoable-tier confirmations the user chose not to see again this session.
    pub skip_confirm: HashSet<String>,
    pub palette: Option<dialogs::Palette>,
    /// The open plug-in dialog (its preview is an open gesture).
    pub plugin_dialog: Option<crate::plugins_ui::Dialog>,
    /// Commands › Manage Plug-ins.
    pub show_plugins: bool,
    /// Window › Actions (F9), and what it addresses.
    pub show_actions: bool,
    pub actions: crate::actions_panel::State,
    /// Control › Test Movie: the movie playing in its own window.
    pub test_movie: Option<crate::test_movie::TestMovie>,
    /// Edit Envelope: a keyframe sound's volume per channel, in its own window (ADR 0039).
    pub envelope: Option<crate::envelope::EnvelopeEditor>,
    pub show_shortcuts: bool,
    /// The window close was intercepted because of unsaved changes.
    pub close_dialog: bool,
    pub allow_close: bool,
    /// Close the window once the pending Save As succeeds.
    pub close_after_save: bool,
    pub prefs: prefs::Prefs,
    pub show_prefs: bool,
    /// Installed fonts, scanned off the UI thread on first need.
    pub system_fonts: Option<std::sync::Arc<prefs::SystemFonts>>,
    fonts_rx: Option<std::sync::mpsc::Receiver<prefs::SystemFonts>>,
    /// Re-check which glyphs the active fonts have (after fonts change).
    glyph_check: bool,
    /// Document revision at the end of the last frame (edits schedule a repaint).
    last_revision: u64,
    title: String,
    /// Keyboard shortcuts in effect (defaults plus the user's changes) and their lookup table.
    pub keymap: keymap::Keymap,
    shortcuts: Vec<(Modifiers, Key, &'static str)>,
    /// The shortcut editor is waiting for a key press: global shortcuts pause.
    pub key_capture: Option<dialogs::KeyCapture>,
    /// Crash-recovery autosave and the copies earlier sessions left behind (ADR 0015).
    pub recovery: autosave::Recovery,
    /// Sound output for playback, scrubbing and library previews.
    pub sound: crate::sound::SoundOut,
    /// Playback follows the sound's clock (the movie has sounds and the device plays them).
    audio_clock: bool,
    /// The scene and frame last heard, so a moved playhead plays a snippet (scrubbing).
    heard: Option<(usize, u32)>,
    /// The panel Ctrl+F6 last moved focus to (where the next press continues from).
    pub last_panel: Option<crate::a11y::Region>,
}

/// How finished this version is, shown next to the version number (docs/release.md).
pub const RELEASE_STAGE: &str = "early alpha";

/// View-state keys, as Animate users know them: `(keys, action)`.
pub const VIEW_SHORTCUTS: &[(&str, &str)] = &[
    ("Ctrl+2", "view.fit"),
    ("Alt+Shift+O", "view.onion"),
    ("Ctrl+Alt+A", "view.select_all_frames"),
    ("Ctrl+Alt+8", "view.align_to_stage"),
    // Keyboard focus between the panels (ADR 0022). F6 alone is Insert Keyframe, as animators
    // expect; Ctrl+F6 is the next-pane key of Windows apps.
    ("Ctrl+F6", "view.next_panel"),
    ("Ctrl+Shift+F6", "view.prev_panel"),
    ("F9", "view.actions"),
    // Brush size, as in paint programs.
    ("[", "view.brush_smaller"),
    ("]", "view.brush_larger"),
];

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, session: Session, shot: Option<PathBuf>) -> App {
        #[cfg_attr(not(gpu_stage), allow(unused_mut))]
        let mut app = App::with_context(&cc.egui_ctx, session, shot);
        // The GPU stage shares the window's device when it runs on wgpu (ADR 0029).
        #[cfg(gpu_stage)]
        app.stage_cache.gpu.set_window_device(cc.wgpu_render_state.as_ref());
        app
    }

    /// The app on an egui context (the window's, or a headless one in tests).
    pub fn with_context(ctx: &egui::Context, session: Session, shot: Option<PathBuf>) -> App {
        theme::apply(ctx);
        let shot_mode = shot.is_some();
        // Screenshot runs use defaults so automated checks are reproducible.
        let prefs = if shot_mode { prefs::Prefs::default() } else { prefs::Prefs::load() };
        let keymap = keymap::Keymap::new(&prefs.shortcuts);
        let shortcuts = keymap.table();
        let library_sel = LibraryItem::first(&session.doc);
        let (recovery, recovery_err) = autosave::Recovery::start(shot_mode);
        // Text lays out with the fonts on this computer plus the embedded interface font (so new
        // text has the same default everywhere). Scan them now, off the UI thread.
        anicraft_engine::text::Fonts::register(crate::theme::embedded_fonts());
        // (No thread in a browser, ADR 0036: there the library is only the embedded faces and
        // is built on first use.)
        let _ = std::thread::Builder::new().name("anicraft-font-scan".into()).spawn(|| {
            let _ = anicraft_engine::text::Fonts::get();
        });
        let mut session = session;
        // The editor draws as Animate does: merge drawing, until Object Drawing (J) is switched on.
        session.style.object_drawing = false;
        let mut app = App {
            s: session,
            tool: Tool::Select,
            view: StageView { zoom: 1.0, pan: egui::Vec2::ZERO, fit: true, fitted: None },
            stage_cache: stage::Cache::default(),
            drag: Drag::None,
            hint_menu: None,
            playing: false,
            looping: true,
            play_acc: 0.0,
            onion: false,
            camera_preview: false,
            last_tool: Tool::Select,
            status: None,
            prompt: None,
            show_about: false,
            show_files: false,
            show_licenses: false,
            shot: shot.map(|path| Shot { path, frames: 0, requested: false }),
            cursor_stage: None,
            tl_scroll: 0.0,
            tl_cell_w: timeline::CELL_DEFAULT,
            tl_vscroll: 0.0,
            tl_last_layer: None,
            tl_collapsed: HashSet::new(),
            library_sel,
            text_edit: None,
            font_families: None,
            star_points: 5,
            rect_radius: 0.0,
            pen: None,
            pencil: Vec::new(),
            pencil_mode: pathtools::PencilMode::default(),
            brush: crate::brushtool::BrushOptions::default(),
            brush_samples: Vec::new(),
            pen_pressure: None,
            eraser: crate::erasertool::EraserOptions::default(),
            bucket_gap: GapSize::default(),
            align_to_stage: false,
            points: Vec::new(),
            width_sel: None,
            motion_anchors: Vec::new(),
            motion_drag: None,
            frame_sel: None,
            tl_drag: timeline::TlDrag::None,
            bottom: BottomTab::Timeline,
            me_prop: anicraft_engine::doc::Property::X,
            me_key: None,
            me_drag: None,
            me_vscroll: 0.0,
            timeline_focus: false,
            confirm: None,
            skip_confirm: HashSet::new(),
            palette: None,
            plugin_dialog: None,
            show_plugins: false,
            show_actions: false,
            actions: crate::actions_panel::State::default(),
            test_movie: None,
            envelope: None,
            show_shortcuts: false,
            close_dialog: false,
            allow_close: false,
            close_after_save: false,
            prefs,
            show_prefs: false,
            system_fonts: None,
            fonts_rx: None,
            glyph_check: true,
            last_revision: 0,
            title: String::new(),
            keymap,
            shortcuts,
            key_capture: None,
            recovery,
            sound: crate::sound::SoundOut::new(),
            audio_clock: false,
            heard: None,
            last_panel: None,
        };
        app.apply_prefs(ctx);
        app.decode_sounds();
        app.status_ok("Welcome to AniCraft. Press Enter to play the sample animation.");
        if let Some(e) = recovery_err {
            app.status_err(e);
        }
        crate::plugins_ui::load_at_start(&mut app);
        app
    }

    /// View › Preview Through Camera (ADR 0027). With no camera yet there is nothing to look
    /// through, so it says how to add one.
    pub fn toggle_camera_preview(&mut self) {
        self.camera_preview = !self.camera_preview;
        if self.camera_preview && self.s.doc.timeline(self.s.scene).camera_layer().is_none() {
            self.status_ok("Preview through camera is on; add a camera (the camera button on the timeline, or the Camera tool) to look through it.");
        }
    }

    pub fn status_ok(&mut self, text: impl Into<String>) {
        self.status = Some(Status { text: text.into(), error: false, at: 0.0 });
    }
    pub fn status_err(&mut self, text: impl Into<String>) {
        self.status = Some(Status { text: text.into(), error: true, at: 0.0 });
    }

    /// Runs an engine command, reporting failures in the status bar.
    /// After a move, selected raw shapes merge where they land (ADR 0031).
    pub fn drop_raw(&mut self) {
        let sel = &self.s.selection;
        let raw = self.s.visible_elements().iter().any(|e| sel.contains(&e.id) && matches!(&e.kind, anicraft_engine::doc::ElementKind::Shape(sh) if sh.merge));
        if raw {
            self.run("draw.drop", json!({}));
        }
    }

    pub fn run(&mut self, id: &str, params: Value) -> Option<Value> {
        let result = self.s.execute(id, &params);
        if platform::WEB && id.starts_with("file.") {
            // In a browser, what a save or an export wrote is in memory: hand it to the
            // browser as downloads (ADR 0036).
            platform::deliver(anicraft_engine::vfs::take_written());
        }
        match result {
            Ok(v) => Some(v),
            Err(e) => {
                self.status_err(e.to_string());
                None
            }
        }
    }

    /// A file the browser handed over: it goes where the command reads it, and the command runs
    /// (through `request`, so opening over unsaved work asks first).
    fn open_picked(&mut self, p: platform::Picked) {
        // Only the name: the browser never says which folder it came from.
        let name: String = std::path::Path::new(&p.name).file_name().map_or_else(|| "file".into(), |n| n.to_string_lossy().chars().take(128).collect());
        let path = std::path::PathBuf::from(&name);
        match p.data.and_then(|data| anicraft_engine::vfs::put(&path, data).map_err(|e| e.to_string())) {
            Ok(()) => self.request(p.cmd, json!({"path": name})),
            Err(e) => self.status_err(format!("Could not open {name}: {e}")),
        }
    }

    // ----- preferences ----------------------------------------------------------------------

    /// Applies scale and fonts from `self.prefs`. System fonts need the font scan; until it
    /// finishes the bundled fonts stay in place, and they are swapped in when it completes.
    pub fn apply_prefs(&mut self, ctx: &egui::Context) {
        ctx.set_zoom_factor(self.prefs.ui_scale);
        theme::apply_palette(ctx, self.prefs.theme);
        self.keymap = keymap::Keymap::new(&self.prefs.shortcuts);
        self.shortcuts = self.keymap.table();
        let wants_system = [&self.prefs.ui_font, &self.prefs.data_font].iter().any(|f| matches!(f, prefs::FontChoice::System(_)));
        if wants_system && self.system_fonts.is_none() {
            self.scan_system_fonts();
        }
        let load = |choice: &prefs::FontChoice, sys: Option<&prefs::SystemFonts>| -> Option<theme::UserFont> {
            match (choice, sys) {
                (prefs::FontChoice::System(name), Some(s)) => s.regular(name).map(|(b, i)| (name.clone(), b, i)),
                _ => None,
            }
        };
        let sys = self.system_fonts.clone();
        let ui_font = load(&self.prefs.ui_font, sys.as_deref());
        let data_font = load(&self.prefs.data_font, sys.as_deref());
        if sys.is_some() {
            let missing: Vec<String> = [(&self.prefs.ui_font, ui_font.is_some()), (&self.prefs.data_font, data_font.is_some())]
                .into_iter()
                .filter_map(|(c, ok)| match c {
                    prefs::FontChoice::System(name) if !ok => Some(name.clone()),
                    _ => None,
                })
                .collect();
            for name in missing {
                self.status_err(format!("Font “{name}” is not installed; using the bundled font"));
            }
        }
        theme::install_fonts(ctx, ui_font.as_ref(), data_font.as_ref());
        self.glyph_check = true;
    }

    /// `--theme` / `--scale`: a palette and interface scale for this run only (clamped like the
    /// preference), not saved.
    pub fn preview_look(&mut self, ctx: &egui::Context, theme: Option<theme::Palette>, scale: Option<f32>) {
        if theme.is_none() && scale.is_none() {
            return;
        }
        let mut p = self.prefs.clone();
        p.theme = theme.unwrap_or(p.theme);
        p.ui_scale = scale.unwrap_or(p.ui_scale);
        self.prefs = p.sanitized();
        self.apply_prefs(ctx);
    }

    pub fn open_prefs(&mut self) {
        self.show_prefs = true;
        self.scan_system_fonts();
    }

    /// Changes preferences: applies them live and saves them.
    pub fn set_prefs(&mut self, ctx: &egui::Context, p: prefs::Prefs) {
        let p = p.sanitized();
        if p == self.prefs {
            return;
        }
        self.prefs = p;
        self.apply_prefs(ctx);
        if self.shot.is_none()
            && let Err(e) = self.prefs.save()
        {
            self.status_err(format!("Could not save preferences: {e}"));
        }
    }

    /// Starts scanning installed fonts on a background thread (once).
    pub fn scan_system_fonts(&mut self) {
        if self.system_fonts.is_some() || self.fonts_rx.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new().name("anicraft-fonts".into()).spawn(move || {
            let _ = tx.send(prefs::SystemFonts::scan());
        });
        if spawned.is_ok() {
            self.fonts_rx = Some(rx);
        }
    }

    fn poll_font_scan(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.fonts_rx else { return };
        match rx.try_recv() {
            Ok(fonts) => {
                self.system_fonts = Some(std::sync::Arc::new(fonts));
                self.fonts_rx = None;
                self.apply_prefs(ctx);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(50)),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.fonts_rx = None,
        }
    }

    /// `--open` at start-up: shows a panel, for visual checks and documentation screenshots.
    /// Imports a two-second beat made here (decaying 220 Hz pulses every half second) to the
    /// stage, for screenshots; whether it worked.
    fn import_demo_beat(&mut self) -> bool {
        let rate = 22_050u32;
        let beat: Vec<f32> = (0..rate * 2)
            .map(|i| {
                let t = i as f32 / rate as f32;
                let env = (-(t % 0.5) * 9.0).exp() * if ((t / 0.5) as u32).is_multiple_of(2) { 0.9 } else { 0.5 };
                env * (t * 220.0 * std::f32::consts::TAU).sin()
            })
            .collect();
        let path = std::env::temp_dir().join("anicraft-beat.wav");
        if std::fs::write(&path, anicraft_audio::wav::encode(rate, 1, &beat, anicraft_audio::wav::Bits::Int16)).is_err() {
            return false;
        }
        self.perform("file.import", json!({"path": path.display().to_string()}));
        true
    }

    pub fn open_panel(&mut self, which: Option<&str>) {
        match which {
            Some("palette") => self.palette = Some(dialogs::Palette { query: "key".into(), selected: 0 }),
            Some("shortcuts") => self.show_shortcuts = true,
            Some("lastscene") => {
                let last = self.s.doc.scenes.len().saturating_sub(1);
                self.run("scene.select", json!({"scene": last}));
            }
            Some("files") => self.show_files = true,
            Some("recovery") => {
                // Work left behind by a session that didn't close cleanly (sample entries
                // unless ANICRAFT_RECOVERY_DIR holds real ones).
                if self.recovery.found.is_empty() {
                    self.recovery.found = autosave::sample_entries();
                }
                self.recovery.show = true;
            }
            Some("licenses") => self.show_licenses = true,
            Some("about") => self.show_about = true,
            Some("zoom") => {
                // 300% on the ball: many tiles, edges cross shapes (seam check).
                self.view.fit = false;
                self.view.zoom = 3.0;
                self.view.pan = egui::vec2(1200.0, 300.0);
            }
            Some("prefs") => {
                self.show_prefs = true;
                self.scan_system_fonts();
            }
            Some("confirm") => self.request("layer.delete", json!({})),
            Some("plugins") => {
                // The first generator's dialog, previewing (run with ANICRAFT_PLUGINS set).
                let first = self.s.plugins.list().find(|p| crate::plugins_ui::is_command(p) && !p.manifest().params.is_empty() && p.manifest().selection == anicraft_engine::plugins::SelectionNeed::Optional).map(|p| p.id().to_string());
                match first {
                    Some(id) => crate::plugins_ui::open(self, &id),
                    None => self.show_plugins = true,
                }
            }
            Some("filters") => {
                // A movie clip with a bevel and a gradient glow, selected: Properties › Filters
                // shows both (scrolled to the glow's ramp editor).
                self.run("draw.rect", json!({"x": 960, "y": 470, "w": 200, "h": 130, "fill": "#e8553b", "stroke": null}));
                self.run("modify.convert_to_symbol", json!({"name": "Badge"}));
                self.run("symbol.set_kind", json!({"kind": "movie_clip"}));
                self.run(
                    "element.set_filters",
                    json!({"filters": [
                        {"type": "bevel", "blur_x": 8, "blur_y": 8, "distance": 6, "strength": 1.2, "quality": 2},
                        {"type": "gradient_glow", "blur_x": 28, "blur_y": 28, "quality": 2, "distance": 0, "stops": [{"offset": 0, "color": "#ffffff00"}, {"offset": 0.55, "color": "#ffd36b"}, {"offset": 1, "color": "#ff4d3d"}]},
                    ]}),
                );
            }
            Some(which @ ("actions" | "testmovie")) => {
                // The interactive sample: its Play button selected with the Actions panel on
                // its events, or playing in Test Movie with the pointer over the button.
                self.perform("file.new_sample", json!({"which": "interactive"}));
                self.run("layer.select", json!({"index": 1}));
                if which == "actions" {
                    if let Some(id) = self.s.visible_elements().iter().find(|e| e.name == "play_btn").map(|e| e.id.0) {
                        self.run("element.select", json!({"ids": [id]}));
                    }
                    crate::actions_panel::open(self, crate::actions_panel::Tab::Instance);
                } else {
                    self.run("layer.select", json!({"index": 0}));
                    crate::actions_panel::open(self, crate::actions_panel::Tab::Frame);
                    crate::test_movie::open(self);
                    crate::test_movie::demo_pointer(self);
                }
            }
            Some("script") => {
                // The interactive sample as an imported Animate document might bring it: a
                // frame script kept on the Actions layer's first keyframe (in place of its
                // Stop action), a linkage class and a document class; the Actions panel shows
                // the Script view (ADR 0013 tier 1).
                self.perform("file.new_sample", json!({"which": "interactive"}));
                self.run("layer.select", json!({"index": 0}));
                let code = "import flash.events.MouseEvent;\n\nstop();\nplay_btn.addEventListener(MouseEvent.CLICK, onPlay);\n\nfunction onPlay(event:MouseEvent):void {\n\tgotoAndPlay(\"show\");\n}\n";
                self.run("script.set", json!({"frame": 0, "text": code, "language": "actionscript3"}));
                self.run("script.set_class", json!({"class": "com.example.Menu"}));
                if let Some(button) = self.s.doc.library.iter().find(|s| s.kind == anicraft_engine::doc::SymbolKind::Button).map(|s| s.id.0) {
                    self.run("script.set_class", json!({"symbol": button, "class": "com.example.PlayButton", "base": "flash.display.SimpleButton"}));
                }
                crate::actions_panel::open(self, crate::actions_panel::Tab::Frame);
            }
            Some("exporthtml") => {
                // File › Export › Export HTML Canvas… on the interactive sample (the in-app
                // prompt: screenshots never open native dialogs).
                self.perform("file.new_sample", json!({"which": "interactive"}));
                self.invoke("file.export_html");
            }
            Some("select") => {
                // The top layer's artwork selected, for its Properties.
                self.run("layer.select", json!({"index": 0}));
                self.run("edit.select_all", json!({}));
            }
            Some("transform") => {
                self.tool = Tool::Transform;
                self.run("layer.select", json!({"index": 0}));
                self.run("edit.select_all", json!({}));
            }
            Some("points") => {
                // A pencil stroke and a pen path being edited, with one smooth point selected.
                let wave: Vec<[f32; 2]> = (0..=240).map(|i| [140.0 + i as f32 * 1.5, 330.0 + (i as f32 * 0.06).sin() * 28.0]).collect();
                self.run("draw.freehand", json!({"points": wave, "stroke_width": 3}));
                if let Some(v) = self.run("draw.path", json!({"d": "M 520 90 L 700 120 L 660 260 L 540 230 Z"})) {
                    let id = anicraft_engine::doc::ElementId(v["element"].as_u64().unwrap_or(0));
                    self.run("path.convert_point", json!({"id": id.0, "cmd": 1, "smooth": true}));
                    self.points = vec![(id, 1, anicraft_engine::geom::edit::ANCHOR)];
                }
                self.tool = Tool::Subselect;
            }
            Some("merge") => {
                // Merge drawing (ADR 0031) on a new layer: a rectangle and an oval that cut each
                // other, a line through both, and the oval's fill with its outline selected.
                self.run("layer.new", json!({}));
                for (id, p) in [
                    ("draw.rect", json!({"x": 180, "y": 120, "w": 300, "h": 200, "fill": "#f2c14e", "stroke": "#1b2a4a", "stroke_width": 3})),
                    ("draw.ellipse", json!({"x": 360, "y": 210, "w": 240, "h": 240, "fill": "#3d8bfd", "stroke": "#1b2a4a", "stroke_width": 3})),
                    ("draw.line", json!({"x0": 140, "y0": 420, "x1": 680, "y1": 140, "stroke": "#d6336c", "stroke_width": 3})),
                ] {
                    let mut p = p;
                    p["object"] = json!(false);
                    self.run(id, p);
                }
                self.run("element.select_connected", json!({"x": 520, "y": 400}));
                self.tool = Tool::Select;
            }
            Some(which @ ("brush" | "brushing")) => {
                // Brushed artwork on a new layer (ADR 0033): a pressure-tapered wave, a flat
                // calligraphic tip at 45°, square dabs, and a pink stroke painted behind a raw
                // oval; "brushing" also has a stroke in progress (the live preview).
                self.run("layer.new", json!({"name": "Paint"}));
                self.run("draw.object_drawing", json!({"on": false}));
                let wave: Vec<[f32; 3]> = (0..=120)
                    .map(|i| {
                        let t = i as f32 / 120.0;
                        [120.0 + t * 520.0, 470.0 + (t * 9.0).sin() * 46.0, (t * std::f32::consts::PI).sin()]
                    })
                    .collect();
                self.run("draw.brush", json!({"points": wave, "size": 34, "color": "#1b2a4a"}));
                let loops: Vec<[f32; 2]> = (0..=160)
                    .map(|i| {
                        let t = i as f32 / 160.0 * std::f32::consts::TAU * 2.0;
                        [190.0 + i as f32 * 2.6 + 34.0 * t.cos(), 170.0 + 40.0 * t.sin()]
                    })
                    .collect();
                self.run("draw.brush", json!({"points": loops, "size": 22, "roundness": 25, "angle": 45, "color": "#e8553b"}));
                for (k, x) in [700.0f32, 740.0, 780.0].into_iter().enumerate() {
                    self.run("draw.brush", json!({"points": [[x, 120.0 + k as f32 * 18.0]], "size": 24, "shape": "square", "angle": 15.0 * k as f32, "color": "#3d8bfd"}));
                }
                self.run("draw.ellipse", json!({"x": 820, "y": 380, "w": 220, "h": 160, "fill": "#f2c14e", "stroke": "#1b2a4a", "stroke_width": 3}));
                let behind: Vec<[f32; 2]> = (0..=60).map(|i| [760.0 + i as f32 * 6.0, 560.0 - i as f32 * 3.4]).collect();
                self.run("draw.brush", json!({"points": behind, "size": 40, "mode": "behind", "color": "#d6336c"}));
                self.run("edit.deselect", json!({}));
                self.tool = Tool::Brush;
                self.brush.size = 24.0;
                self.brush.mode = crate::brushtool::BrushOptions::default().mode;
                if which == "brushing" {
                    self.brush_samples = (0..=40).map(|i| (anicraft_engine::geom::Vec2::new(150.0 + i as f32 * 9.0, 610.0 - (i as f32 * 0.16).sin() * 30.0), Some(0.3 + i as f32 / 60.0))).collect();
                    self.drag = Drag::Brush;
                }
            }
            Some(which @ ("eraser" | "erasing")) => {
                // Erased artwork on a new layer (ADR 0034): a band through an outlined raw
                // rectangle (Erase Normal), the outline of a raw oval erased but not its fill
                // (Erase Lines), a fill deleted by the Faucet, and a drawing object the eraser
                // passed over untouched; "erasing" also has a stroke in progress.
                self.run("layer.new", json!({"name": "Ink"}));
                self.run("draw.object_drawing", json!({"on": false}));
                self.run("draw.rect", json!({"x": 140, "y": 150, "w": 380, "h": 230, "fill": "#f2c14e", "stroke": "#1b2a4a", "stroke_width": 4}));
                let band: Vec<[f32; 2]> = (0..=80).map(|i| [100.0 + i as f32 * 5.5, 265.0 + (i as f32 * 0.12).sin() * 60.0]).collect();
                self.run("draw.erase", json!({"points": band, "size": 30}));
                self.run("draw.ellipse", json!({"x": 600, "y": 160, "w": 260, "h": 200, "fill": "#3d8bfd", "stroke": "#1b2a4a", "stroke_width": 6}));
                self.run("draw.erase", json!({"points": [[560, 260], [900, 260]], "size": 40, "mode": "lines"}));
                self.run("draw.rect", json!({"x": 940, "y": 160, "w": 200, "h": 200, "fill": "#e8553b", "stroke": "#1b2a4a", "stroke_width": 4}));
                self.run("draw.faucet", json!({"x": 1040, "y": 260}));
                self.run("draw.rect", json!({"x": 300, "y": 430, "w": 300, "h": 120, "fill": "#8a79c9", "stroke": null, "object": true}));
                self.run("draw.erase", json!({"points": [[260, 490], [640, 490]], "size": 30}));
                self.run("edit.deselect", json!({}));
                self.tool = Tool::Eraser;
                self.eraser = crate::erasertool::EraserOptions { size: 30.0, ..Default::default() };
                if which == "erasing" {
                    self.brush_samples = (0..=40).map(|i| (anicraft_engine::geom::Vec2::new(700.0 + i as f32 * 9.0, 470.0 - (i as f32 * 0.2).sin() * 40.0), None)).collect();
                    self.drag = Drag::Eraser;
                }
            }
            Some("eyedropper") => {
                // The sample's sky gradient picked with the Eyedropper: the fill swatch holds it.
                self.run("paint.pick", json!({"x": 640, "y": 40}));
                self.tool = Tool::Eyedropper;
            }
            Some("frames") => {
                // Frames 8–20 selected on the top three layers, with a keyframe drag in progress:
                // 6 frames later and one layer down.
                let layers = &self.s.doc.timeline(self.s.scene).layers;
                if let (Some(a), Some(b)) = (layers.first(), layers.get(2)) {
                    self.frame_sel = Some(timeline::FrameSel { anchor: (a.id, 7), head: (b.id, 19) });
                    self.tl_drag = timeline::TlDrag::Move { grab: 10, by: 6, grab_row: 0, rows_by: 1 };
                }
                self.run("timeline.goto", json!({"frame": 10}));
            }
            Some("bucket") => {
                // A hand-drawn outline with no fill, filled by the bucket underneath its line.
                let loop_pts: Vec<[f32; 2]> = (0..=72)
                    .map(|i| {
                        let a = i as f32 / 72.0 * std::f32::consts::TAU;
                        let r = 110.0 + 18.0 * (a * 5.0).sin();
                        [640.0 + r * a.cos(), 330.0 + r * a.sin()]
                    })
                    .collect();
                self.run("draw.freehand", json!({"points": loop_pts, "closed": true, "stroke": "#14141a", "stroke_width": 4}));
                self.run("paint.bucket", json!({"x": 640, "y": 330, "color": "#ff9f43"}));
                self.tool = Tool::PaintBucket;
                self.bucket_gap = GapSize::Small;
                self.run("edit.deselect", json!({}));
            }
            Some("sound") => {
                // A two-second beat imported to the stage, stream sync, its keyframe's Sound
                // properties showing.
                if self.import_demo_beat() {
                    self.run("sound.set", json!({"frame": 0, "sync": "stream"}));
                    self.run("timeline.goto", json!({"frame": 14}));
                }
            }
            Some("envelope") => {
                // The beat with a custom envelope (in from the left, out to the right), its
                // envelope editor open on a selected point, the layer at 80 %.
                if self.import_demo_beat() {
                    let pts = json!([[0.0, 0.0, 0.0], [0.4, 1.0, 0.25], [1.2, 0.6, 1.0], [2.0, 0.0, 0.1]]);
                    self.run("sound.set", json!({"frame": 0, "envelope": pts}));
                    self.run("layer.set_volume", json!({"volume": 0.8}));
                    self.run("timeline.goto", json!({"frame": 10}));
                    crate::envelope::open(self);
                    crate::envelope::select(self, 1);
                }
            }
            Some(which @ ("hints" | "hintsend")) => {
                // A square shape-tweening into a circle on a new layer, with three shape hints:
                // a placed on both keyframes, b on the first only, c just added (unplaced).
                self.run("layer.new", json!({"name": "Morph"}));
                self.run("timeline.goto", json!({"frame": 0}));
                self.run("draw.rect", json!({"x": 420, "y": 220, "w": 200, "h": 200, "fill": "#ff9f43"}));
                self.run("timeline.insert_blank_keyframe", json!({"frame": 15}));
                self.run("timeline.goto", json!({"frame": 15}));
                self.run("draw.ellipse", json!({"x": 750, "y": 210, "w": 220, "h": 220, "fill": "#ff9f43"}));
                self.run("timeline.create_shape_tween", json!({"frame": 0}));
                self.run("edit.deselect", json!({}));
                let d = 110.0 * std::f32::consts::FRAC_1_SQRT_2;
                for (start, end) in [(Some((420.0, 220.0)), Some((860.0 - d, 320.0 - d))), (Some((620.0, 330.0)), None), (None, None)] {
                    let Some(v) = self.run("tween.add_hint", json!({"frame": 0})) else { break };
                    let hint = v["hint"].clone();
                    if let Some((x, y)) = start {
                        self.run("tween.move_hint", json!({"hint": hint, "frame": 0, "side": "start", "x": x, "y": y}));
                    }
                    if let Some((x, y)) = end {
                        self.run("tween.move_hint", json!({"hint": hint, "frame": 0, "side": "end", "x": x, "y": y}));
                    }
                }
                self.run("timeline.goto", json!({"frame": if which == "hints" { 0 } else { 15 }}));
            }
            Some(which @ ("camera" | "cameraview")) => {
                // The sample seen by a camera that pushes in and turns over the first second,
                // the far background a layer deep; the Camera tool in hand at frame 12.
                self.run("camera.add", json!({}));
                self.run("timeline.insert_keyframe", json!({"frame": 23}));
                self.run("camera.set", json!({"frame": 23, "x": 760, "y": 380, "zoom": 160, "rotation": -8}));
                self.run("timeline.create_tween", json!({"frame": 0}));
                if let Some(back) = self.s.doc.timeline(0).layers.last().map(|l| l.id.0) {
                    self.run("layer.set_depth", json!({"layer": back, "depth": 1500}));
                }
                self.run("timeline.goto", json!({"frame": 12}));
                self.tool = Tool::Camera;
                self.last_tool = Tool::Camera;
                self.camera_preview = which == "cameraview";
            }
            Some("tlwide") => {
                // Zoomed in: wide frames, playhead on frame 1 and frame 4.
                self.tl_cell_w = 48.0;
                self.run("timeline.goto", json!({"frame": 3}));
            }
            Some("tlzoom") => {
                // A long timeline zoomed out: the ruler thins its labels, the scrollbar shows.
                self.run("timeline.insert_frame", json!({"frame": 47, "count": 600}));
                self.tl_cell_w = 4.0;
                self.tl_scroll = 120.0;
            }
            Some("span") => {
                // Stretching the Star layer's span by its end, with the length readout.
                if let Some(l) = self.s.doc.timeline(self.s.scene).layers.get(1)
                    && let Some(k) = l.keyframes.first()
                {
                    self.tl_drag = timeline::TlDrag::Span { layer: l.id, start: k.start, end: k.end(), to: k.end() + 6 };
                }
            }
            Some("width") => {
                // A tapered brush-like stroke being shaped with the Width tool: one width point
                // pushed out on its left side and selected.
                let wave: Vec<[f32; 2]> = (0..=200).map(|i| [180.0 + i as f32 * 4.0, 420.0 - (i as f32 * 0.035).sin() * 110.0]).collect();
                if let Some(v) = self.run("draw.freehand", json!({"points": wave, "stroke": "#1b2a4a", "stroke_width": 14, "profile": "taper_both"})) {
                    let id = anicraft_engine::doc::ElementId(v["element"].as_u64().unwrap_or(0));
                    if let Some(w) = self.run("stroke.set_width_point", json!({"id": id.0, "t": 0.3, "left": 2.4, "right": 0.9})) {
                        self.width_sel = w["index"].as_u64().map(|i| (id, i as usize));
                    }
                }
                self.tool = Tool::Width;
            }
            Some(which @ ("motion" | "motionspan")) => {
                // The Star layer's span as a motion tween: it moves, turns and fades out, with
                // an eased X and a custom Bezier on Y, shown in the Motion Editor.
                self.run("layer.select", json!({"index": 1}));
                if self.run("timeline.create_motion_tween", json!({"frame": 0})).is_some() {
                    let frames = self.s.active_layer().and_then(|l| l.keyframes.first()).map_or(24, |k| k.duration);
                    let (mid, end) = (frames / 2, frames.saturating_sub(1));
                    let x0 = self.run("motion.info", json!({"frame": 0})).and_then(|v| v["values"]["x"].as_f64()).unwrap_or(0.0);
                    let y0 = self.run("motion.info", json!({"frame": 0})).and_then(|v| v["values"]["y"].as_f64()).unwrap_or(0.0);
                    for (p, f, v) in [("x", mid, x0 + 260.0), ("x", end, x0 + 420.0), ("y", mid, y0 + 160.0), ("y", end, y0), ("rotation", end, 270.0), ("alpha", end, 20.0)] {
                        self.run("motion.set_key", json!({"property": p, "frame": f, "value": v}));
                    }
                    self.run("motion.set_ease", json!({"property": "x", "frame": 1, "easing": "CubicOut"}));
                    self.run("motion.set_ease", json!({"property": "y", "frame": 1, "easing": {"bezier": [0.2, 1.6, 0.6, 1.0]}}));
                    self.s.mark_saved();
                }
                if which == "motion" {
                    self.bottom = BottomTab::MotionEditor;
                }
                self.me_prop = anicraft_engine::doc::Property::Y;
                self.me_key = Some(0);
                self.run("timeline.goto", json!({"frame": 6}));
            }
            Some(which @ ("motionpath" | "motionpoints" | "motionedit")) => {
                // The Star layer's span as a motion tween along a curved, oriented motion path,
                // the star selected half way along it (ADR 0030); with Subselection, the
                // middle anchor's handles show.
                self.run("layer.select", json!({"index": 1}));
                if self.run("timeline.create_motion_tween", json!({"frame": 0})).is_some() {
                    let info = self.run("motion.info", json!({"frame": 0}));
                    let at = |k: &str| info.as_ref().and_then(|v| v["values"][k].as_f64()).unwrap_or(0.0);
                    let (x, y) = (at("x"), at("y"));
                    let d = format!("M{x} {y} C{} {} {} {} {} {} C{} {} {} {} {} {}", x + 70.0, y - 150.0, x + 170.0, y - 150.0, x + 230.0, y - 20.0, x + 290.0, y + 110.0, x + 380.0, y + 130.0, x + 430.0, y + 10.0);
                    self.run("motion.set_path", json!({"d": d}));
                    self.run("motion.orient", json!({"orient": true}));
                    self.s.mark_saved();
                }
                self.run("timeline.goto", json!({"frame": 9}));
                self.run("edit.select_all", json!({}));
                if which != "motionpath" {
                    // The span selected instead of the star: Properties shows the tween's rows.
                    self.run("element.select", json!({"ids": []}));
                    self.frame_sel = Some(timeline::FrameSel::cell(self.s.layer, 9));
                    self.tool = Tool::Subselect;
                    self.motion_anchors = vec![1];
                }
                if which == "motionedit" {
                    // Points edited (ADR 0038): one added on the second segment, the middle
                    // anchor made a corner, both selected with their handles showing.
                    self.run("motion.add_point", json!({"segment": 2, "t": 0.45}));
                    self.run("motion.convert_point", json!({"anchor": 1, "smooth": false}));
                    self.motion_anchors = vec![1, 2];
                    self.status_ok(format!("Added a point to the motion path. {} to undo.", platform::shortcut_label("Ctrl+Z")));
                }
            }
            Some("pen") => {
                // A pen path still being drawn: a corner, a smooth point with handles, a corner.
                self.tool = Tool::Pen;
                let k = |x: f32, y: f32| [anicraft_engine::geom::Vec2::new(x, y); 3];
                let mut curve = k(260.0, 140.0);
                curve[1] = anicraft_engine::geom::Vec2::new(220.0, 180.0);
                curve[2] = anicraft_engine::geom::Vec2::new(300.0, 100.0);
                self.pen = Some(pathtools::PenPath { knots: vec![k(120.0, 160.0), curve, k(380.0, 150.0)], dragging: false });
            }
            _ => {}
        }
    }

    /// Any modal (prompt, confirmation, close guard) is open: global shortcuts pause.
    pub fn modal_open(&self) -> bool {
        self.prompt.is_some() || self.confirm.is_some() || self.close_dialog || self.plugin_dialog.is_some()
    }

    /// The single entry point for user-initiated commands: asks first when the engine says the
    /// command needs confirmation (ADR 0004), otherwise runs it.
    pub fn request(&mut self, id: &str, params: Value) {
        if let Some(c) = self.s.confirmation(id, &params)
            && (c.irreversible || !self.skip_confirm.contains(id))
        {
            self.playing = false;
            self.confirm = Some(dialogs::PendingConfirm { id: id.to_string(), params, conf: c, dont_ask: false });
            return;
        }
        self.perform(id, params);
    }

    /// Runs a command and reports the outcome; routine destructive edits say how to undo.
    pub fn perform(&mut self, id: &str, params: Value) -> Option<Value> {
        let selected = self.s.selection.len();
        // A browser's autosaved copy opens as a file (ADR 0036): it is a restore all the same.
        let restored = autosave::restored_session(id, &params);
        let v = self.run(id, params)?;
        if let Some(session) = restored {
            self.view.fit = true;
            self.frame_sel = None;
            self.library_sel = LibraryItem::first(&self.s.doc);
            self.decode_sounds();
            self.recovery.restored(&session);
            let name = self.s.path.as_deref().and_then(std::path::Path::file_name).map_or_else(|| "Untitled".to_string(), |n| n.to_string_lossy().into_owned());
            self.status_ok(format!("Restored “{name}”. It isn't saved yet: Save to keep it."));
            return Some(v);
        }
        match id {
            "file.new" | "file.new_sample" | "file.open" => {
                self.view.fit = true;
                self.library_sel = LibraryItem::first(&self.s.doc);
                self.decode_sounds();
                match v.get("report").and_then(Value::as_str) {
                    // An imported file: say what came across and what didn't, and that saving
                    // makes a new .anicraft document.
                    Some(r) => self.status_ok(format!("{r} Save to keep it as an AniCraft document.")),
                    None => self.status_ok(if id == "file.open" { "Opened" } else { "New document" }),
                }
            }
            "file.recover" => {
                self.view.fit = true;
                self.frame_sel = None;
                self.library_sel = LibraryItem::first(&self.s.doc);
                self.recovery.restored(v["recovered"].as_str().unwrap_or(""));
                let name = v["name"].as_str().unwrap_or("Untitled");
                self.status_ok(if v["original_newer"].as_bool() == Some(true) {
                    format!("Restored “{name}” as a new document (the file on disk is newer). Save to keep it.")
                } else {
                    format!("Restored “{name}”. It isn't saved yet: Save to keep it.")
                });
            }
            "file.discard_recovery" => {
                self.recovery.discarded(v["discarded"].as_str().unwrap_or(""));
                self.status_ok("Discarded the autosaved copy");
            }
            "file.save" | "file.save_as" => {
                // In a browser the file goes to the downloads (ADR 0036).
                self.status_ok(format!("Saved {}{}", v["saved"].as_str().unwrap_or(""), if platform::WEB { " to your downloads" } else { "" }));
                if self.close_after_save {
                    self.allow_close = true;
                }
            }
            "file.import" | "file.import_to_library" if v.get("sound").is_some() => {
                let id = anicraft_engine::doc::SoundId(v["sound"].as_u64().unwrap_or(0));
                self.library_sel = Some(LibraryItem::Sound(id));
                let name = v["name"].as_str().unwrap_or("");
                let secs = v["seconds"].as_f64().unwrap_or(0.0);
                self.status_ok(if v.get("layer").is_some() {
                    format!("{name} ({secs:.2} s) is on a new layer from frame {}. Set its sync in Properties. Ctrl+Z to undo.", self.s.frame + 1)
                } else {
                    format!("{name} ({secs:.2} s) is in the Library. Select a keyframe and pick it under Sound in Properties.")
                });
            }
            "file.import_to_library" => self.status_ok("Image added to the Library"),
            "file.export_audio" => {
                let n = v["sounds"].as_u64().unwrap_or(0);
                let note = if n == 0 { " No sounds play in that range, so the file is silent." } else { "" };
                self.status_ok(format!("Exported {} ({:.2} s).{note}", v["written"].as_str().unwrap_or(""), v["seconds"].as_f64().unwrap_or(0.0)));
            }
            "sound.delete" => self.status_ok("Deleted sound — Ctrl+Z to undo"),
            "file.import" => {
                if let Some(b) = self.s.doc.bitmaps.last().filter(|b| v["bitmap"].as_u64() == Some(b.id.0)) {
                    self.library_sel = Some(LibraryItem::Bitmap(b.id));
                }
                let n = v["elements"].as_u64().unwrap_or(0);
                let report = v["report"].as_str().unwrap_or("");
                let what = if n == 1 { "element" } else { "elements" };
                let undo = platform::shortcut_label("Ctrl+Z");
                // A drawing with clip paths or masks comes in as a folder of layers (ADR 0021).
                let place = match (v["layers"].as_u64().unwrap_or(1), v["masks"].as_u64().unwrap_or(0)) {
                    (0 | 1, _) => "on a new layer".to_string(),
                    (l, 1) => format!("in a new folder of {l} layers with a mask"),
                    (l, m) => format!("in a new folder of {l} layers with {m} masks"),
                };
                self.status_ok(format!("{n} {what} {place}. {report} {undo} to undo."));
            }
            "file.export_sprite_sheet" => self.status_ok(format!("Exported {} frames to {} ({}×{} grid) and its frame map {}", v["frames"], v["written"].as_str().unwrap_or(""), v["columns"], v["rows"], v["map"].as_str().unwrap_or(""))),
            "file.export_html" => {
                let report = v["report"].as_str().filter(|r| !r.is_empty()).map(|r| format!(" {r}")).unwrap_or_default();
                let kb = v["bytes"].as_u64().unwrap_or(0).div_ceil(1024);
                self.status_ok(format!("Exported {} ({} frames, {kb} KB): open it in a web browser.{report}", v["written"].as_str().unwrap_or(""), v["frames"]));
            }
            "file.export_lottie" => {
                let report = v["report"].as_str().filter(|r| !r.is_empty()).map(|r| format!(" {r}")).unwrap_or_default();
                self.status_ok(format!("Exported {} ({} frames, {} layers).{report}", v["written"].as_str().unwrap_or(""), v["frames"], v["layers"]));
            }
            "file.export_png" | "file.export_png_sequence" | "file.export_gif" | "file.export_svg" | "file.export_video" => self.status_ok(format!("Exported {}", v.get("written").or_else(|| v.get("dir")).and_then(Value::as_str).unwrap_or(""))),
            "edit.undo" => self.status_ok(format!("Undid {}", v["undone"].as_str().unwrap_or(""))),
            "edit.redo" => self.status_ok(format!("Redid {}", v["redone"].as_str().unwrap_or(""))),
            "edit.delete" => self.status_ok(format!("Deleted {} {} — Ctrl+Z to undo", selected, if selected == 1 { "element" } else { "elements" })),
            "layer.delete" => self.status_ok("Deleted layer — Ctrl+Z to undo"),
            "draw.erase_all" => {
                let n = v["erased"].as_u64().unwrap_or(0);
                self.status_ok(format!("Erased everything at frame {} ({n} {}). {} to undo.", self.s.frame + 1, if n == 1 { "element" } else { "elements" }, platform::shortcut_label("Ctrl+Z")));
            }
            "scene.delete" => {
                self.frame_sel = None;
                self.status_ok("Deleted scene — Ctrl+Z to undo");
            }
            "scene.new" | "scene.duplicate" | "scene.select" | "scene.next" | "scene.prev" => {
                // Another scene's timeline: frame selections and scroll belong to the old one.
                self.frame_sel = None;
                self.tl_scroll = 0.0;
                self.playing = false;
                let (n, of) = (v["scene"].as_u64().unwrap_or(0) + 1, v["scenes"].as_u64().unwrap_or(1));
                self.status_ok(format!("{} (scene {n} of {of})", v["name"].as_str().unwrap_or("")));
            }
            "timeline.clear_keyframe" => {
                let n = v["cleared"].as_u64().unwrap_or(1);
                self.status_ok(format!("Cleared {} — Ctrl+Z to undo", if n == 1 { "keyframe".to_string() } else { format!("{n} keyframes") }));
            }
            "timeline.copy_frames" | "timeline.cut_frames" | "timeline.paste_frames" => {
                let n = v["copied"].as_u64().or_else(|| v["cut"].as_u64()).or_else(|| v["pasted"].as_u64()).unwrap_or(0);
                let what = format!("{n} {}", if n == 1 { "frame" } else { "frames" });
                self.status_ok(match id {
                    "timeline.copy_frames" => format!("Copied {what}"),
                    "timeline.cut_frames" => format!("Cut {what} — Ctrl+Z to undo"),
                    _ => format!("Pasted {what} — Ctrl+Z to undo"),
                });
            }
            "plugin.run" | "plugin.install" | "plugin.remove" | "plugin.reload" => crate::plugins_ui::status(self, id, &v),
            "timeline.remove_frame" => {
                let n = v["removed"].as_u64().unwrap_or(1);
                self.status_ok(format!("Removed {} — Ctrl+Z to undo", if n == 1 { "frame".to_string() } else { format!("{n} frames") }));
            }
            _ => {}
        }
        Some(v)
    }

    /// Invokes a command from a menu, shortcut, palette or context menu, asking for a path or
    /// name first when it needs one.
    pub fn invoke(&mut self, id: &'static str) {
        // Ctrl+X/C/V act on frames when the timeline was used last and has frames to act on
        // (Animate's Ctrl+Alt+X/C/V work anywhere).
        let id: &'static str = match id {
            "edit.cut" if self.timeline_focus && self.frame_sel.is_some() => "timeline.cut_frames",
            "edit.copy" if self.timeline_focus && self.frame_sel.is_some() => "timeline.copy_frames",
            "edit.paste" if self.timeline_focus && !self.s.frame_clipboard.is_empty() => "timeline.paste_frames",
            other => other,
        };
        if let Some(why) = platform::unavailable(id) {
            self.status_err(why);
            return;
        }
        if id == "timeline.paste_frames"
            && let Some(p) = timeline::paste_params(self)
        {
            self.request(id, p);
            return;
        }
        if id == "file.recovery_list" {
            if !self.recovery.rescan() {
                return; // a browser answers on a later frame (autosave::tick)
            }
            self.recovery.show = !self.recovery.found.is_empty();
            if !self.recovery.show {
                self.status_ok(if self.recovery.active() { "No autosaved work to recover" } else { "Autosave is off on this system, so there is nothing to recover" });
            }
            return;
        }
        if id == "tween.add_hint" {
            crate::shape_hints::add(self);
            return;
        }
        if id == "control.test_movie" {
            // Control › Test Movie: the player in its own window (headless, the command plays
            // the movie and reports).
            crate::test_movie::open(self);
            return;
        }
        if id == "plugin.list" {
            // Commands › Manage Plug-ins (headless, the same command lists them).
            self.show_plugins = true;
            return;
        }
        if id == "edit.paste" {
            // Paste in the centre of what you are looking at.
            let c = self.view_center_stage();
            self.request(id, json!({"x": c.x, "y": c.y}));
            return;
        }
        let prompt = match id {
            "file.open" => Some(("Open document", "path", "animation.anicraft".to_string())),
            "file.save" if self.s.path.is_none() => Some(("Save document as", "path", "animation.anicraft".to_string())),
            "file.save_as" => Some(("Save document as", "path", self.s.path.as_ref().map_or_else(|| "animation.anicraft".to_string(), |p| p.display().to_string()))),
            "layer.rename" => Some(("Rename layer", "name", self.s.active_layer().map_or_else(String::new, |l| l.name.clone()))),
            "scene.rename" => Some(("Rename scene", "name", self.s.doc.scenes.get(self.s.scene).map_or_else(String::new, |s| s.name.clone()))),
            "file.export_png" => Some(("Export frame as PNG", "path", format!("frame_{:04}.png", self.s.frame + 1))),
            "file.export_png_sequence" => Some(("Export PNG sequence to folder", "dir", "frames".to_string())),
            "file.export_gif" => Some(("Export animated GIF", "path", "animation.gif".to_string())),
            "file.export_svg" => Some(("Export frame as SVG", "path", format!("frame_{:04}.svg", self.s.frame + 1))),
            "file.export_sprite_sheet" => Some(("Export sprite sheet (PNG + JSON frame map)", "path", "sprites.png".to_string())),
            "file.export_video" => Some(("Export video (.webm or .mp4, made with your ffmpeg)", "path", "animation.webm".to_string())),
            "file.export_lottie" => Some(("Export Lottie animation (JSON, for web and apps)", "path", "animation.json".to_string())),
            "file.export_html" => Some(("Export HTML canvas (one web page)", "path", "movie.html".to_string())),
            "file.export_audio" => Some(("Export audio (the movie's soundtrack as a WAV)", "path", "soundtrack.wav".to_string())),
            "file.import_to_library" => Some(("Import to library (WAV, OGG or MP3 sound; PNG or JPEG image)", "path", "sound.wav".to_string())),
            "file.import" => Some(("Import to stage (SVG, PNG or JPEG; WAV, OGG or MP3 onto a new layer)", "path", "drawing.svg".to_string())),
            "plugin.install" => Some(("Install plug-in (.wasm file)", "path", "plugin.wasm".to_string())),
            "modify.convert_to_symbol" => Some(("Convert to symbol — name", "name", format!("Symbol {}", self.s.doc.library.len() + 1))),
            _ => None,
        };
        if let Some((title, key, text)) = prompt {
            let is_file = matches!(key, "path" | "dir") && (id.starts_with("file.") || id == "plugin.install");
            if is_file && platform::WEB && platform::reads_file(id) {
                // The browser's file picker (ADR 0036). The file arrives on a later frame
                // (`open_picked`); a save or export asks for the download's name below.
                self.playing = false;
                platform::pick_file(id);
                return;
            }
            if is_file && self.prefs.native_dialogs && self.shot.is_none() && !platform::WEB {
                // The platform's own dialog (it blocks, as native dialogs do). Cancel = no-op.
                self.playing = false;
                #[cfg(not(target_arch = "wasm32"))]
                match platform::file_dialog(id, title, &text) {
                    Some(path) => self.request(id, json!({ key: path.display().to_string() })),
                    None => self.close_after_save = false,
                }
                return;
            }
            self.prompt = Some(Prompt { title, cmd: id, key, text });
            return;
        }
        // Frame commands act on the frame selection (F5 over 3 frames on 2 layers inserts 3
        // frames on each).
        if timeline::RANGE_COMMANDS.contains(&id)
            && let Some(p) = timeline::range_params(self)
        {
            self.request(id, p);
            return;
        }
        self.request(id, json!({}));
    }

    /// The stage point at the centre of the visible canvas.
    pub fn view_center_stage(&self) -> egui::Vec2 {
        let size = egui::vec2(self.s.doc.stage.width as f32, self.s.doc.stage.height as f32);
        size * 0.5 - self.view.pan / self.view.zoom.max(1e-3)
    }

    pub fn zoom_about(&mut self, canvas: egui::Rect, screen: egui::Pos2, new_zoom: f32) {
        let new_zoom = new_zoom.clamp(0.05, 64.0);
        let size = egui::vec2(self.s.doc.stage.width as f32, self.s.doc.stage.height as f32);
        let origin = canvas.center() + self.view.pan - size * self.view.zoom * 0.5;
        let stage_pt = (screen - origin) / self.view.zoom;
        let new_origin = screen - stage_pt * new_zoom;
        self.view.pan = new_origin - canvas.center() + size * new_zoom * 0.5;
        self.view.zoom = new_zoom;
    }

    pub fn toggle_play(&mut self) {
        self.playing = !self.playing;
        self.play_acc = 0.0;
        if self.playing && self.s.frame + 1 >= self.s.frame_count() {
            self.s.set_frame(0);
        }
        self.start_sound();
    }

    /// The playhead as a movie frame (scenes in order).
    fn movie_frame(&self) -> u32 {
        self.s.doc.scene_start(self.s.scene).saturating_add(self.s.frame)
    }

    /// Starts the sound from the playhead, or stops it; the playhead follows the sound's clock
    /// while it plays.
    fn start_sound(&mut self) {
        self.audio_clock = self.playing && self.shot.is_none() && self.sound.play(&self.s.doc, self.s.state, self.movie_frame());
        if !self.playing {
            self.sound.stop();
        }
        // Said once: the device could not be opened, so playback is silent.
        if let Some(e) = self.sound.error.take() {
            self.status_err(format!("Playing without sound: {e}"));
        }
    }

    /// Decodes the document's sounds on a background thread (each once; every copy of the
    /// document shares the result), so opening a file never waits on it.
    pub fn decode_sounds(&self) {
        let sounds = self.s.doc.sounds.clone();
        if !sounds.is_empty() {
            let _ = std::thread::Builder::new().name("anicraft-decode".into()).spawn(move || {
                for x in sounds {
                    let _ = x.decoded();
                }
            });
        }
    }

    /// Scrubbing: a moved playhead plays a frame's worth of the stream sounds there.
    fn scrub_sound(&mut self) {
        let here = (self.s.scene, self.s.frame);
        if !self.playing && self.shot.is_none() && self.heard.is_some_and(|h| h != here) {
            self.sound.scrub(&self.s.doc, self.s.state, self.movie_frame());
        }
        self.heard = Some(here);
    }

    fn tick_playback(&mut self, ctx: &egui::Context) {
        if !self.playing {
            if self.audio_clock {
                self.sound.stop();
                self.audio_clock = false;
            }
            return;
        }
        let n = self.s.frame_count();
        if self.audio_clock {
            // Picture follows sound: the frame is wherever the sound has got to (edits made
            // while playing are heard at once).
            self.sound.sync(&self.s.doc, self.s.state);
            if let Some(mf) = self.sound.frame(self.s.doc.stage.fps) {
                let local = mf.saturating_sub(self.s.doc.scene_start(self.s.scene));
                if local >= n {
                    if self.looping {
                        self.s.set_frame(0);
                        self.start_sound();
                    } else {
                        self.s.set_frame(n.saturating_sub(1));
                        self.playing = false;
                        self.start_sound();
                    }
                } else if local != self.s.frame {
                    self.s.set_frame(local);
                }
                self.heard = Some((self.s.scene, self.s.frame));
                ctx.request_repaint();
                return;
            }
            // The device went away: carry on silently on the wall clock.
            self.audio_clock = false;
            self.play_acc = 0.0;
        }
        let fps = f64::from(self.s.doc.stage.fps.max(1.0));
        self.play_acc += f64::from(ctx.input(|i| i.stable_dt)).min(0.25);
        let n = self.s.frame_count();
        while self.play_acc >= 1.0 / fps {
            self.play_acc -= 1.0 / fps;
            if self.s.frame + 1 >= n {
                if self.looping {
                    self.s.set_frame(0);
                } else {
                    self.playing = false;
                    break;
                }
            } else {
                self.s.set_frame(self.s.frame + 1);
            }
        }
        self.heard = Some((self.s.scene, self.s.frame));
        ctx.request_repaint();
    }

    /// Ctrl+F6 / Ctrl+Shift+F6: keyboard focus to the next or previous panel (Stage, Timeline,
    /// Properties, Library, Tools). Cut, copy and paste follow it to frames or artwork.
    pub fn focus_panel(&mut self, ctx: &egui::Context, forward: bool) {
        match crate::a11y::cycle_focus(ctx, self.last_panel, forward) {
            Some(region) => {
                self.last_panel = Some(region);
                match region {
                    crate::a11y::Region::Timeline => self.timeline_focus = true,
                    crate::a11y::Region::Stage => self.timeline_focus = false,
                    _ => {}
                }
                let next = self.keymap.primary("view.next_panel").map(platform::shortcut_label).unwrap_or_default();
                self.status_ok(format!("{}{}", region.label(), if next.is_empty() { String::new() } else { format!(" · {next} for the next panel") }));
            }
            None => self.status_err("No panel to move to yet"),
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        // While the shortcut editor records a key, every key press belongs to it.
        if self.modal_open() || self.key_capture.is_some() {
            return;
        }
        // Palette, shortcut sheet, preferences and quit work even while a text field has focus.
        // Preferences: Ctrl+U (the shortcut Animate users know) and Ctrl/⌘+, (platform norm).
        let (palette, sheet, prefs_key, quit) = ctx.input_mut(|i| {
            // The shortcut sheet first: Ctrl+Alt+Shift+K (Animate's Keyboard Shortcuts) must not
            // be taken by Ctrl+K, which egui would match with the extra modifiers ignored.
            let sheet = i.consume_key(Modifiers::COMMAND | Modifiers::ALT | Modifiers::SHIFT, Key::K) || i.consume_key(Modifiers::NONE, Key::F1);
            (
                i.consume_key(Modifiers::COMMAND, Key::K) || i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::P),
                sheet,
                i.consume_key(Modifiers::COMMAND, Key::U) || i.consume_key(Modifiers::COMMAND, Key::Comma),
                i.consume_key(Modifiers::COMMAND, Key::Q),
            )
        });
        if prefs_key {
            self.open_prefs();
        }
        if quit {
            // Goes through the close guard, so unsaved work is never lost.
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if palette {
            self.palette = if self.palette.is_some() { None } else { Some(dialogs::Palette::default()) };
        }
        if sheet {
            self.show_shortcuts = !self.show_shortcuts;
        }
        // Moving between panels (Ctrl+F6) works from anywhere, a text field included.
        let panel_step = ctx.input_mut(|i| {
            self.shortcuts.iter().filter(|(_, _, id)| matches!(*id, "view.next_panel" | "view.prev_panel")).find_map(|(m, k, id)| i.consume_key(*m, *k).then_some(*id == "view.next_panel"))
        });
        if let Some(forward) = panel_step {
            self.focus_panel(ctx, forward);
        }
        // Who has the keyboard (ADR 0022): nothing focused, or the stage or timeline, gets every
        // shortcut. A focused control (button, checkbox, menu) keeps Enter, Space and the arrows
        // for itself (press, move to the next control) but the other shortcuts still work. A
        // text field keeps every key.
        let keys = crate::a11y::keyboard(ctx);
        if keys == crate::a11y::Keyboard::Text || self.palette.is_some() {
            return;
        }
        let control = keys == crate::a11y::Keyboard::Control;
        if !control {
            crate::motion_path::consume_keys(self, ctx);
            pathtools::consume_keys(self, ctx);
            crate::widthtool::consume_keys(self, ctx);
        }
        let mut fire: Vec<&'static str> = Vec::new();
        let mut tool = None;
        let mut play = false;
        let mut nudge = egui::Vec2::ZERO;
        let surface_focused = ctx.memory(|m| m.focused().is_some());
        ctx.input_mut(|i| {
            for (m, k, id) in &self.shortcuts {
                if i.consume_key(*m, *k) {
                    fire.push(id);
                }
            }
            if control {
                return;
            }
            if i.consume_key(Modifiers::NONE, Key::Enter) {
                play = true;
            }
            // Space pans (held, `key_down`); its press must not "click" a focused stage.
            if surface_focused {
                i.consume_key(Modifiers::NONE, Key::Space);
            }
            // Screen pixels, as in Animate: a press moves less the further you zoom in.
            let step = if i.modifiers.shift { 10.0 } else { 1.0 } / self.view.zoom.max(1e-3);
            for (k, d) in [(Key::ArrowLeft, egui::vec2(-step, 0.0)), (Key::ArrowRight, egui::vec2(step, 0.0)), (Key::ArrowUp, egui::vec2(0.0, -step)), (Key::ArrowDown, egui::vec2(0.0, step))] {
                if i.key_pressed(k) && !i.modifiers.command {
                    nudge += d;
                }
            }
        });
        for id in fire {
            match id {
                // View-state keys (Animate's), handled here rather than in the engine.
                "view.fit" => self.view.fit = true,
                "view.onion" => self.onion = !self.onion,
                "view.align_to_stage" => {
                    self.align_to_stage = !self.align_to_stage;
                    self.status_ok(if self.align_to_stage { "Align: to stage" } else { "Align: to the selection" });
                }
                "view.select_all_frames" => {
                    let layers = &self.s.doc.timeline(self.s.scene).layers;
                    if let (Some(a), Some(b)) = (layers.first(), layers.last()) {
                        let last = self.s.frame_count().saturating_sub(1);
                        self.frame_sel = Some(timeline::FrameSel { anchor: (a.id, 0), head: (b.id, last) });
                        self.timeline_focus = true;
                        self.run("timeline.goto", json!({"frame": 0}));
                    }
                }
                "view.actions" => self.show_actions = !self.show_actions,
                "view.brush_smaller" => crate::brushtool::resize(self, false),
                "view.brush_larger" => crate::brushtool::resize(self, true),
                id if id.starts_with("tool.") => tool = keymap::tool_for(id).or(tool),
                _ => self.invoke(id),
            }
        }
        if let Some(t) = tool {
            self.tool = t;
        }
        if play {
            self.toggle_play();
        }
        if nudge != egui::Vec2::ZERO && !self.s.selection.is_empty() {
            // One undo step: the move, and raw shapes merging where they land (ADR 0031).
            let own = !self.s.in_gesture();
            if own {
                self.s.begin_gesture("Move");
            }
            self.run("element.translate", json!({"dx": nudge.x, "dy": nudge.y}));
            self.drop_raw();
            if own {
                self.s.end_gesture();
            }
        }
    }

    fn handle_screenshot(&mut self, ctx: &egui::Context) {
        let Some(shot) = &mut self.shot else { return };
        shot.frames += 1;
        ctx.request_repaint();
        let image = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = image {
            let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
            let result = anicraft_render::encode_png(img.size[0] as u32, img.size[1] as u32, &rgba).map_err(|e| e.to_string()).and_then(|png| {
                if let Some(dir) = shot.path.parent()
                    && !dir.as_os_str().is_empty()
                {
                    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                }
                std::fs::write(&shot.path, png).map_err(|e| e.to_string())
            });
            match result {
                Ok(()) => println!("screenshot saved to {}", shot.path.display()),
                Err(e) => eprintln!("screenshot failed: {e}"),
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if shot.frames >= SHOT_FRAMES && !shot.requested {
            shot.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        if shot.frames > SHOT_FRAMES + 600 {
            eprintln!("screenshot: the renderer never delivered a frame");
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let name = self.s.path.as_ref().and_then(|p| p.file_name()).map_or_else(|| "Untitled".to_string(), |n| n.to_string_lossy().into_owned());
        let title = format!("AniCraft — {name}{}", if self.s.is_dirty() { " •" } else { "" });
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    fn prompt_window(&mut self, ctx: &egui::Context) {
        let Some(p) = &mut self.prompt else { return };
        let mut submit = false;
        let mut cancel = false;
        egui::Window::new(p.title).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, -80.0]).show(ctx, |ui| {
            ui.set_min_width(360.0);
            let r = ui.add(egui::TextEdit::singleline(&mut p.text).desired_width(f32::INFINITY)).named(p.title);
            r.request_focus();
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                submit = true;
            }
            ui.horizontal(|ui| {
                if ui.button("OK").clicked() {
                    submit = true;
                }
                if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                    cancel = true;
                }
            });
        });
        if submit {
            if let Some(p) = self.prompt.take() {
                // Through `request`, so opening over unsaved work or saving over a file asks first.
                self.request(p.cmd, json!({ p.key: p.text }));
            }
        } else if cancel {
            self.prompt = None;
            self.close_after_save = false;
        }
    }

    fn about_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_about;
        let shown = egui::Window::new("About AniCraft").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            ui.add(egui::Image::new((app_mark(ctx).id(), egui::vec2(64.0, 64.0))));
            ui.heading("AniCraft");
            ui.label(format!("Version {} · {RELEASE_STAGE}", env!("CARGO_PKG_VERSION")));
            ui.label("Open-source 2D vector animation studio, written in Rust.");
            ui.label(format!("{} engine commands — all available headless via anicraft-cli.", commands().len()));
            ui.colored_label(T.text_dim, "MIT OR Apache-2.0");
            ui.label(theme::text(theme::Type::Caption, format!("Typefaces: {}", theme::font_summary())));
        });
        crate::a11y::name_title_bar(&shown, "About AniCraft");
        self.show_about = open;
    }

    /// Help › Project Files: where the docs, source and saved files live, with buttons to open them.
    fn files_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_files;
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let entries: [(&str, PathBuf); 12] = [
            ("Project folder", root.clone()),
            ("Requirements & technology", root.join("docs").join("requirements.md")),
            ("Architecture philosophy", root.join("docs").join("architecture.md")),
            ("Roadmap", root.join("docs").join("roadmap.md")),
            ("Performance & budgets", root.join("docs").join("performance.md")),
            ("Decision records (ADRs)", root.join("docs").join("adr")),
            ("Legal compliance", root.join("docs").join("legal").join("compliance.md")),
            ("README", root.join("README.md")),
            ("Agent / contributor guide", root.join("AGENTS.md")),
            ("Engine crates (source)", root.join("crates")),
            ("Desktop app (source)", root.join("src")),
            ("Screenshots", root.join("shots")),
        ];
        let mut opened: Option<Result<(), String>> = None;
        let shown = egui::Window::new("Project Files").open(&mut open).collapsible(false).resizable(false).default_width(620.0).show(ctx, |ui| {
            ui.label(theme::text(theme::Type::Label, "Where AniCraft's documents and files are on this computer"));
            ui.add_space(4.0);
            egui::Grid::new("files").num_columns(3).spacing([10.0, 6.0]).striped(true).show(ui, |ui| {
                for (label, path) in &entries {
                    ui.label(theme::text(theme::Type::Body, *label).strong());
                    ui.label(theme::text(theme::Type::Path, path.display().to_string()).color(if path.exists() { T.text } else { T.text_faint }));
                    if ui.add_enabled(path.exists(), egui::Button::new("Open")).named(&format!("Open {label}")).clicked() {
                        opened = Some(open_in_system(path));
                    }
                    ui.end_row();
                }
                let doc = self.s.path.as_ref().map_or_else(|| "not saved yet (File › Save)".to_string(), |p| p.display().to_string());
                ui.label(theme::text(theme::Type::Body, "Current document").strong());
                ui.label(theme::text(theme::Type::Path, doc));
                ui.end_row();
            });
            ui.add_space(4.0);
            ui.label(theme::text(theme::Type::Caption, "Saved documents and exports go where you type in the save box; a plain name lands in the folder AniCraft was started from."));
        });
        crate::a11y::name_title_bar(&shown, "Project Files");
        match opened {
            Some(Err(e)) => self.status_err(e),
            Some(Ok(())) => self.status_ok("Opened in your system's file viewer"),
            None => {}
        }
        self.show_files = open;
    }
}

/// The app icon (`assets/app-icon/`; the PNG carries its rounded corners) as a texture, decoded on
/// first use and kept in the context. 128 px stays sharp at 64 pt on a 2× display.
fn app_mark(ctx: &egui::Context) -> egui::TextureHandle {
    const PNG: &[u8] = include_bytes!("../assets/app-icon/hicolor/128x128/apps/ai.storyteller.anicraft.png");
    let id = egui::Id::new("anicraft-app-mark");
    if let Some(tex) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return tex;
    }
    let (w, h, rgba) = anicraft_engine::doc::decode_png(PNG).map_or_else(|| (1, 1, vec![0; 4]), |p| (p.width as usize, p.height as usize, p.rgba));
    let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
    let options = egui::TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..egui::TextureOptions::LINEAR };
    let tex = ctx.load_texture("anicraft-app-mark", image, options);
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    tex
}

/// Opens a file or folder with the operating system's default handler.
pub(crate) fn open_in_system(path: &std::path::Path) -> Result<(), String> {
    let mut cmd = if cfg!(target_os = "windows") {
        let mut c = std::process::Command::new("explorer");
        c.arg(path);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg(path);
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(path);
        c
    };
    cmd.spawn().map(|_| ()).map_err(|e| format!("could not open {}: {e}", path.display()))
}

impl eframe::App for App {
    /// A clean exit (saved, or the user chose Don't Save) removes this session's recovery
    /// copy; otherwise the work is written once more and offered at the next start.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let clean = !self.s.is_dirty() || self.allow_close || self.shot.is_some();
        self.recovery.finish(&self.s, clean);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frame(ui);
    }
}

impl App {
    /// One frame of the whole interface. Needs no window, so tests run it headless.
    pub fn frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.handle_screenshot(&ctx);
        self.poll_font_scan(&ctx);
        if let Some(p) = platform::picked_file() {
            self.open_picked(p);
        }
        if self.glyph_check {
            // Fonts installed last frame are live now: can they draw the macOS key symbols?
            let ok = ctx.fonts_mut(|f| f.has_glyphs(&theme::font(theme::Type::Key), "⌘⇧⌥"));
            platform::set_mac_glyphs(ok);
            self.glyph_check = false;
        }
        self.handle_shortcuts(&ctx);
        // Switching away from a path tool keeps its work: a Pen path in progress is finished.
        if self.tool != Tool::Pen && self.pen.is_some() {
            pathtools::commit_pen(self, false);
        }
        if self.tool != Tool::Subselect {
            self.points.clear();
        }
        pathtools::upkeep(self);
        crate::motion_path::upkeep(self);
        crate::widthtool::upkeep(self);
        self.tick_playback(&ctx);
        self.scrub_sound();
        self.update_title(&ctx);
        if let Some(st) = &mut self.status
            && st.at == 0.0 {
                st.at = ctx.input(|i| i.time);
            }

        let bar = |fill| egui::Frame::NONE.fill(fill).inner_margin(egui::Margin::symmetric(8, 2));
        egui::Panel::top("menu_bar").frame(bar(T.chrome)).show(ui, |ui| menus::bar(self, ui));
        egui::Panel::top("edit_bar").exact_size(theme::TARGET + 8.0).frame(bar(T.panel)).show(ui, |ui| panels::edit_bar(self, ui));
        egui::Panel::bottom("status_bar").exact_size(28.0).frame(bar(T.chrome)).show(ui, |ui| panels::status_bar(self, ui));
        // Docks start at their usual size, but never take most of a small window or a large
        // interface scale (200 % on a laptop leaves under 1000 × 600 points), and the stage
        // always keeps room when the scale goes up later.
        let room = ui.available_rect_before_wrap();
        let room = egui::vec2(room.width(), room.height());
        let (dock_w, timeline_h) = ((room.x * 0.3).clamp(220.0, 292.0), (room.y * 0.42).clamp(200.0, 292.0));
        egui::Panel::left("tools").resizable(false).exact_size(theme::TOOL_BUTTON + 12.0).frame(egui::Frame::NONE.fill(T.panel).inner_margin(egui::Margin::symmetric(0, 8))).show(ui, |ui| panels::tools(self, ui));
        egui::Panel::right("properties").resizable(true).default_size(dock_w).max_size((room.x * 0.5).max(220.0)).frame(egui::Frame::NONE.fill(T.panel).inner_margin(egui::Margin::same(0))).show(ui, |ui| panels::right(self, ui));
        egui::Panel::bottom("timeline").resizable(true).default_size(timeline_h).max_size((room.y * 0.6).max(200.0)).frame(egui::Frame::NONE.fill(T.panel).inner_margin(egui::Margin::same(0))).show(ui, |ui| timeline::show(self, ui));
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(T.pasteboard)).show(ui, |ui| stage::show(self, ui));
        crate::a11y::name_splitter(&ctx, "properties", "Resize the right-hand panels");
        crate::a11y::name_splitter(&ctx, "timeline", "Resize the timeline");

        self.prompt_window(&ctx);
        self.about_window(&ctx);
        dialogs::show_all(self, &ctx);
        autosave::window(self, &ctx);
        crate::plugins_ui::show(self, &ctx);
        crate::actions_panel::show(self, &ctx);
        crate::test_movie::show(self, &ctx);
        crate::envelope::show(self, &ctx);

        // Never lose work on close: intercept it while there are unsaved changes.
        if self.allow_close {
            if self.close_after_save || self.close_dialog {
                self.close_after_save = false;
                self.close_dialog = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        } else if ctx.input(|i| i.viewport().close_requested()) && self.shot.is_none() && self.s.is_dirty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.playing = false;
            self.close_dialog = true;
        }
        self.files_window(&ctx);

        // Close a coalesced edit (property drags) once nothing is being dragged or typed.
        // (A plug-in dialog keeps its preview gesture open until OK or Cancel.)
        if self.s.in_gesture() && self.plugin_dialog.is_none() && self.drag == Drag::None && !ctx.egui_is_using_pointer() && ctx.memory(|m| m.focused().is_none()) {
            self.s.end_gesture();
        }

        // After this frame's edits: start a recovery copy when one is due (off this thread).
        autosave::tick(self, &ctx);
        // A browser asks before leaving a page with unsaved work (its close guard).
        platform::set_unsaved(self.s.is_dirty());

        // An edit late in this frame (after the stage drew) must be shown without waiting for
        // the next input event: egui only repaints on demand. This was the "drag commits after a
        // pause" bug.
        if self.s.revision != self.last_revision {
            self.last_revision = self.s.revision;
            ctx.request_repaint();
        }
    }
}
