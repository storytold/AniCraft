//! The AniCraft shell: view state plus the frame loop. Document changes go through
//! `Session::execute`, so everything here is also reachable headless.

use crate::theme::{self, T};
use crate::{dialogs, keymap, menus, panels, pathtools, platform, prefs, stage, timeline};
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
    Pen,
    Pencil,
    Rect,
    Oval,
    Star,
    Line,
    PaintBucket,
    InkBottle,
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
    pub const ALL: [Tool; 13] = [Tool::Select, Tool::Subselect, Tool::Transform, Tool::Pen, Tool::Pencil, Tool::Rect, Tool::Oval, Tool::Star, Tool::Line, Tool::PaintBucket, Tool::InkBottle, Tool::Hand, Tool::Zoom];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "Selection",
            Tool::Subselect => "Subselection",
            Tool::Transform => "Free Transform",
            Tool::Pen => "Pen",
            Tool::Pencil => "Pencil",
            Tool::Rect => "Rectangle",
            Tool::Oval => "Oval",
            Tool::Star => "PolyStar",
            Tool::Line => "Line",
            Tool::PaintBucket => "Paint Bucket",
            Tool::InkBottle => "Ink Bottle",
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
            Tool::Pen => Some("P"),
            Tool::Pencil => Some("Shift+Y"),
            Tool::Rect => Some("R"),
            Tool::Oval => Some("O"),
            Tool::Star => None,
            Tool::Line => Some("N"),
            Tool::PaintBucket => Some("K"),
            Tool::InkBottle => Some("S"),
            Tool::Hand => Some("H"),
            Tool::Zoom => Some("Z"),
        }
    }
    pub fn hint(self) -> &'static str {
        match self {
            Tool::Select => "Click to select, drag to move, drag on empty space to marquee-select. Shift adds to the selection.",
            Tool::Subselect => "Click a shape to show its points. Drag points or handles to reshape (Shift adds to the point selection, drag on empty space to marquee points). Double-click an edge to add a point; Delete removes points. Right-click to make a point smooth or a corner.",
            Tool::Pen => "Click to place corner points, drag to pull smooth handles (Alt moves one handle). Click the first point to close; Enter, Esc or a click on the last point finishes. Backspace removes the last point.",
            Tool::Pencil => "Drag to draw freehand. The options bar sets Smooth, Straighten or Ink.",
            Tool::Transform => "Drag a corner or edge to scale (Shift keeps proportions, Alt scales from the centre), just outside a corner to rotate (Shift snaps 15°), inside to move.",
            Tool::Rect => "Drag to draw a rectangle. Shift draws a square.",
            Tool::Oval => "Drag to draw an oval. Shift draws a circle.",
            Tool::Star => "Drag from the centre to draw a star.",
            Tool::Line => "Drag to draw a line. Shift snaps to 45°.",
            Tool::PaintBucket => "Click inside an outline to fill it (on the layer's lines), or on a fill to recolour it. The gap size closes small openings.",
            Tool::InkBottle => "Click a shape to give it the current stroke colour and width.",
            Tool::Hand => "Drag to pan the stage. Space works with any tool.",
            Tool::Zoom => "Click to zoom in, Alt-click to zoom out. Ctrl+wheel zooms anywhere.",
        }
    }
}

/// Stage camera.
pub struct StageView {
    pub zoom: f32,
    pub pan: egui::Vec2,
    pub fit: bool,
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
    /// Free transform: `applied` is the transform already sent to the engine this gesture.
    Transform { kind: XfKind, start: egui::Pos2, bounds: [f32; 4], applied: anicraft_engine::geom::Affine },
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

pub struct App {
    pub s: Session,
    pub tool: Tool,
    pub view: StageView,
    pub stage_cache: stage::Cache,
    pub drag: Drag,
    pub playing: bool,
    pub looping: bool,
    play_acc: f64,
    pub onion: bool,
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
    pub library_sel: Option<usize>,
    pub star_points: u32,
    pub rect_radius: f32,
    /// Pen path being drawn (view state until it is committed as one command).
    pub pen: Option<pathtools::PenPath>,
    /// Pencil samples of the stroke in progress (stage space).
    pub pencil: Vec<anicraft_engine::geom::Vec2>,
    pub pencil_mode: pathtools::PencilMode,
    pub bucket_gap: GapSize,
    /// Align panel: align to the stage rather than the selection's bounds.
    pub align_to_stage: bool,
    /// Subselection: selected anchors and handles.
    pub points: Vec<pathtools::PointRef>,
    /// Selected frames on the timeline, and the drag on the frame grid.
    pub frame_sel: Option<timeline::FrameSel>,
    pub tl_drag: timeline::TlDrag,
    /// The frame grid was clicked more recently than the stage: Ctrl+X/C/V act on frames.
    pub timeline_focus: bool,
    /// A destructive command waiting for the user's decision (ADR 0004).
    pub confirm: Option<dialogs::PendingConfirm>,
    /// Undoable-tier confirmations the user chose not to see again this session.
    pub skip_confirm: HashSet<String>,
    pub palette: Option<dialogs::Palette>,
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
}

/// How finished this version is, shown next to the version number (docs/release.md).
pub const RELEASE_STAGE: &str = "early alpha";

/// View-state keys, as Animate users know them: `(keys, action)`.
pub const VIEW_SHORTCUTS: &[(&str, &str)] = &[
    ("Ctrl+2", "view.fit"),
    ("Alt+Shift+O", "view.onion"),
    ("Ctrl+Alt+A", "view.select_all_frames"),
    ("Ctrl+Alt+8", "view.align_to_stage"),
    ("Ctrl+Enter", "view.test"),
];

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, session: Session, shot: Option<PathBuf>) -> App {
        theme::apply(&cc.egui_ctx);
        let shot_mode = shot.is_some();
        // Screenshot runs use defaults so automated checks are reproducible.
        let prefs = if shot_mode { prefs::Prefs::default() } else { prefs::Prefs::load() };
        let keymap = keymap::Keymap::new(&prefs.shortcuts);
        let shortcuts = keymap.table();
        let mut app = App {
            s: session,
            tool: Tool::Select,
            view: StageView { zoom: 1.0, pan: egui::Vec2::ZERO, fit: true },
            stage_cache: stage::Cache::default(),
            drag: Drag::None,
            playing: false,
            looping: true,
            play_acc: 0.0,
            onion: false,
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
            library_sel: Some(0),
            star_points: 5,
            rect_radius: 0.0,
            pen: None,
            pencil: Vec::new(),
            pencil_mode: pathtools::PencilMode::default(),
            bucket_gap: GapSize::default(),
            align_to_stage: false,
            points: Vec::new(),
            frame_sel: None,
            tl_drag: timeline::TlDrag::None,
            timeline_focus: false,
            confirm: None,
            skip_confirm: HashSet::new(),
            palette: None,
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
        };
        app.apply_prefs(&cc.egui_ctx);
        app.status_ok("Welcome to AniCraft. Press Enter to play the sample animation.");
        app
    }

    pub fn status_ok(&mut self, text: impl Into<String>) {
        self.status = Some(Status { text: text.into(), error: false, at: 0.0 });
    }
    pub fn status_err(&mut self, text: impl Into<String>) {
        self.status = Some(Status { text: text.into(), error: true, at: 0.0 });
    }

    /// Runs an engine command, reporting failures in the status bar.
    pub fn run(&mut self, id: &str, params: Value) -> Option<Value> {
        match self.s.execute(id, &params) {
            Ok(v) => Some(v),
            Err(e) => {
                self.status_err(e.to_string());
                None
            }
        }
    }

    // ----- preferences ----------------------------------------------------------------------

    /// Applies scale and fonts from `self.prefs`. System fonts need the font scan; until it
    /// finishes the bundled fonts stay in place, and they are swapped in when it completes.
    pub fn apply_prefs(&mut self, ctx: &egui::Context) {
        ctx.set_zoom_factor(self.prefs.ui_scale);
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
    pub fn open_panel(&mut self, which: Option<&str>) {
        match which {
            Some("palette") => self.palette = Some(dialogs::Palette { query: "key".into(), selected: 0 }),
            Some("shortcuts") => self.show_shortcuts = true,
            Some("files") => self.show_files = true,
            Some("licenses") => self.show_licenses = true,
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
            Some("frames") => {
                // Frames 8–20 selected on the top three layers, with a keyframe drag in progress:
                // 6 frames later and one layer down.
                let layers = &self.s.doc.timeline.layers;
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
                if let Some(l) = self.s.doc.timeline.layers.get(1)
                    && let Some(k) = l.keyframes.first()
                {
                    self.tl_drag = timeline::TlDrag::Span { layer: l.id, start: k.start, end: k.end(), to: k.end() + 6 };
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
        self.prompt.is_some() || self.confirm.is_some() || self.close_dialog
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
        let v = self.run(id, params)?;
        match id {
            "file.new" | "file.new_sample" | "file.open" => {
                self.view.fit = true;
                self.library_sel = (!self.s.doc.library.is_empty()).then_some(0);
                match v.get("report").and_then(Value::as_str) {
                    // An imported file: say what came across and what didn't, and that saving
                    // makes a new .anicraft document.
                    Some(r) => self.status_ok(format!("{r} Save to keep it as an AniCraft document.")),
                    None => self.status_ok(if id == "file.open" { "Opened" } else { "New document" }),
                }
            }
            "file.save" | "file.save_as" => {
                self.status_ok(format!("Saved {}", v["saved"].as_str().unwrap_or("")));
                if self.close_after_save {
                    self.allow_close = true;
                }
            }
            "file.export_png" | "file.export_png_sequence" | "file.export_gif" | "file.export_svg" => self.status_ok(format!("Exported {}", v.get("written").or_else(|| v.get("dir")).and_then(Value::as_str).unwrap_or(""))),
            "edit.undo" => self.status_ok(format!("Undid {}", v["undone"].as_str().unwrap_or(""))),
            "edit.redo" => self.status_ok(format!("Redid {}", v["redone"].as_str().unwrap_or(""))),
            "edit.delete" => self.status_ok(format!("Deleted {} {} — Ctrl+Z to undo", selected, if selected == 1 { "element" } else { "elements" })),
            "layer.delete" => self.status_ok("Deleted layer — Ctrl+Z to undo"),
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
        if id == "timeline.paste_frames"
            && let Some(p) = timeline::paste_params(self)
        {
            self.request(id, p);
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
            "file.export_png" => Some(("Export frame as PNG", "path", format!("frame_{:04}.png", self.s.frame + 1))),
            "file.export_png_sequence" => Some(("Export PNG sequence to folder", "dir", "frames".to_string())),
            "file.export_gif" => Some(("Export animated GIF", "path", "animation.gif".to_string())),
            "file.export_svg" => Some(("Export frame as SVG", "path", format!("frame_{:04}.svg", self.s.frame + 1))),
            "modify.convert_to_symbol" => Some(("Convert to symbol — name", "name", format!("Symbol {}", self.s.doc.library.len() + 1))),
            _ => None,
        };
        if let Some((title, key, text)) = prompt {
            let is_file = matches!(key, "path" | "dir") && id.starts_with("file.");
            if is_file && self.prefs.native_dialogs && self.shot.is_none() {
                // The platform's own dialog (it blocks, as native dialogs do). Cancel = no-op.
                self.playing = false;
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
    }

    fn tick_playback(&mut self, ctx: &egui::Context) {
        if !self.playing {
            return;
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
        ctx.request_repaint();
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
        if ctx.egui_wants_keyboard_input() || self.palette.is_some() {
            return;
        }
        pathtools::consume_keys(self, ctx);
        let mut fire: Vec<&'static str> = Vec::new();
        let mut tool = None;
        let mut play = false;
        let mut nudge = egui::Vec2::ZERO;
        ctx.input_mut(|i| {
            for (m, k, id) in &self.shortcuts {
                if i.consume_key(*m, *k) {
                    fire.push(id);
                }
            }
            if i.consume_key(Modifiers::NONE, Key::Enter) {
                play = true;
            }
            let step = if i.modifiers.shift { 10.0 } else { 1.0 };
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
                    let layers = &self.s.doc.timeline.layers;
                    if let (Some(a), Some(b)) = (layers.first(), layers.last()) {
                        let last = self.s.frame_count().saturating_sub(1);
                        self.frame_sel = Some(timeline::FrameSel { anchor: (a.id, 0), head: (b.id, last) });
                        self.timeline_focus = true;
                        self.run("timeline.goto", json!({"frame": 0}));
                    }
                }
                "view.test" => {
                    // Test: play from frame 1.
                    self.run("timeline.goto", json!({"frame": 0}));
                    if !self.playing {
                        self.toggle_play();
                    }
                }
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
            self.run("element.translate", json!({"dx": nudge.x, "dy": nudge.y}));
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
            let r = ui.add(egui::TextEdit::singleline(&mut p.text).desired_width(f32::INFINITY));
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
        egui::Window::new("About AniCraft").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            ui.heading("AniCraft");
            ui.label(format!("Version {} · {RELEASE_STAGE}", env!("CARGO_PKG_VERSION")));
            ui.label("Open-source 2D vector animation studio, written in Rust.");
            ui.label(format!("{} engine commands — all available headless via anicraft-cli.", commands().len()));
            ui.colored_label(T.text_dim, "MIT OR Apache-2.0");
            ui.label(theme::text(theme::Type::Caption, format!("Typefaces: {}", theme::font_summary())));
        });
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
        egui::Window::new("Project Files").open(&mut open).collapsible(false).resizable(false).default_width(620.0).show(ctx, |ui| {
            ui.label(theme::text(theme::Type::Label, "Where AniCraft's documents and files are on this computer"));
            ui.add_space(4.0);
            egui::Grid::new("files").num_columns(3).spacing([10.0, 6.0]).striped(true).show(ui, |ui| {
                for (label, path) in &entries {
                    ui.label(theme::text(theme::Type::Body, *label).strong());
                    ui.label(theme::text(theme::Type::Path, path.display().to_string()).color(if path.exists() { T.text } else { T.text_faint }));
                    if ui.add_enabled(path.exists(), egui::Button::new("Open")).clicked() {
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
        match opened {
            Some(Err(e)) => self.status_err(e),
            Some(Ok(())) => self.status_ok("Opened in your system's file viewer"),
            None => {}
        }
        self.show_files = open;
    }
}

/// Opens a file or folder with the operating system's default handler.
fn open_in_system(path: &std::path::Path) -> Result<(), String> {
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
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_screenshot(&ctx);
        self.poll_font_scan(&ctx);
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
        self.tick_playback(&ctx);
        self.update_title(&ctx);
        if let Some(st) = &mut self.status
            && st.at == 0.0 {
                st.at = ctx.input(|i| i.time);
            }

        let bar = |fill| egui::Frame::NONE.fill(fill).inner_margin(egui::Margin::symmetric(8, 2));
        egui::Panel::top("menu_bar").frame(bar(T.chrome)).show(ui, |ui| menus::bar(self, ui));
        egui::Panel::top("edit_bar").exact_size(theme::TARGET + 8.0).frame(bar(T.panel)).show(ui, |ui| panels::edit_bar(self, ui));
        egui::Panel::bottom("status_bar").exact_size(28.0).frame(bar(T.chrome)).show(ui, |ui| panels::status_bar(self, ui));
        egui::Panel::left("tools").resizable(false).exact_size(theme::TOOL_BUTTON + 12.0).frame(egui::Frame::NONE.fill(T.panel).inner_margin(egui::Margin::symmetric(6, 8))).show(ui, |ui| panels::tools(self, ui));
        egui::Panel::right("properties").resizable(true).default_size(292.0).frame(egui::Frame::NONE.fill(T.panel).inner_margin(egui::Margin::same(0))).show(ui, |ui| panels::right(self, ui));
        egui::Panel::bottom("timeline").resizable(true).default_size(292.0).frame(egui::Frame::NONE.fill(T.panel).inner_margin(egui::Margin::same(0))).show(ui, |ui| timeline::show(self, ui));
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(T.pasteboard)).show(ui, |ui| stage::show(self, ui));

        self.prompt_window(&ctx);
        self.about_window(&ctx);
        dialogs::show_all(self, &ctx);

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
        if self.s.in_gesture() && self.drag == Drag::None && !ctx.egui_is_using_pointer() && ctx.memory(|m| m.focused().is_none()) {
            self.s.end_gesture();
        }

        // An edit late in this frame (after the stage drew) must be shown without waiting for
        // the next input event: egui only repaints on demand. This was the "drag commits after a
        // pause" bug.
        if self.s.revision != self.last_revision {
            self.last_revision = self.s.revision;
            ctx.request_repaint();
        }
    }
}
