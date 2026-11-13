//! Files dropped onto the window from the desktop or a browser (ADR 0059). Documents open (the
//! unsaved-changes guard as with File › Open); drawings, images and sounds dropped on the stage
//! go onto the active layer at the playhead where they were dropped, dropped on the Library
//! go into it, into the folder under the pointer (an SVG as a graphic symbol), and dropped on a
//! layer's frame in the timeline go onto that layer at that frame (ADR 0065). While files are
//! dragged over the window a highlight shows where they will land. The routing is `anicraft_engine::cmds_drop::route`;
//! the import is the `file.drop` command, one undo step.
//!
//! Where a drop lands needs the pointer. Browsers report it while dragging (our own `dragover`
//! listener, `web::drag_pos`), but winit 0.30 gives no position with dragged files, and Windows
//! and macOS send no pointer moves during a drag. So a drop waits a moment for the pointer's
//! first move after it (the system sends one as the drag ends); without one it lands at the
//! centre of the view, and says so. (winit 0.30.13, which eframe 0.36 and 0.37 both use, still
//! has no drag position; ADR 0065.)

use crate::app::App;
use crate::theme::{T, Type, color, font};
use anicraft_engine::cmds_drop::{DropAction, DropKind, DropTarget, kind_of, route};
use anicraft_engine::doc::{FolderId, LayerId, LayerKind};
use egui::{Pos2, Rect, Stroke, StrokeKind, pos2, vec2};
use serde_json::json;
use std::path::{Path, PathBuf};

/// How long a drop waits for the pointer to say where it is, in seconds.
const WAIT_FOR_POINTER: f64 = 0.35;

/// Where the stage was drawn this frame and how it maps to the edited space.
#[derive(Clone, Copy)]
pub struct StageGeo {
    pub canvas: Rect,
    pub origin: Pos2,
    pub zoom: f32,
    /// Previewing through the camera: artwork edits are paused.
    pub paused: bool,
}

impl StageGeo {
    pub fn to_stage(self, p: Pos2) -> (f32, f32) {
        let z = self.zoom.max(1e-6);
        ((p.x - self.origin.x) / z, (p.y - self.origin.y) / z)
    }
}

/// Where the timeline's rows and frames were drawn this frame (set by `timeline::show`).
#[derive(Clone, Debug, Default)]
pub struct TimelineGeo {
    /// Each visible row (the layer column and its frames), and its layer.
    pub rows: Vec<(Rect, LayerId)>,
    /// Where the frame cells start (left of it is the layer column).
    pub cells_left: f32,
    /// The screen x of frame 0's left edge, and a frame's width.
    pub x0: f32,
    pub cell_w: f32,
}

impl TimelineGeo {
    /// The layer and frame under `p` (over the layer column: the playhead's frame), and the
    /// frame's cell.
    pub fn at(&self, p: Pos2, playhead: u32) -> Option<(LayerId, u32, Rect)> {
        let (row, layer) = *self.rows.iter().find(|(r, _)| r.contains(p))?;
        let cw = self.cell_w.max(1.0);
        let frame = if p.x >= self.cells_left { ((p.x - self.x0) / cw).floor().clamp(0.0, anicraft_engine::MAX_FRAME as f32) as u32 } else { playhead };
        let left = self.x0 + frame as f32 * cw;
        Some((layer, frame, Rect::from_min_max(pos2(left.max(self.cells_left), row.top()), pos2((left + cw).max(self.cells_left), row.bottom()))))
    }
}

/// Where a drop at a point lands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Spot {
    /// On the stage, at this point of the edited space.
    Stage(f32, f32),
    /// On the Library, into this folder (`None`: the top level).
    Library(Option<FolderId>),
    /// On a layer's frame in the timeline.
    Timeline(LayerId, u32),
    /// Somewhere else on the window.
    Window,
    /// The start screen is up: documents open, nothing imports.
    Start,
    /// The pointer's position isn't known (a drop on the desktop without a pointer move).
    Unknown,
}

/// What a drop does, worked out from its files and where it landed.
#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    /// The document to open (the first one dropped).
    pub open: Option<PathBuf>,
    /// Files to import, and where.
    pub import: Vec<PathBuf>,
    pub to: Option<DropTarget>,
    /// Files left out, each with why (in the interface language).
    pub left_out: Vec<(PathBuf, String)>,
}

/// Why a file can't go there, in the interface language.
fn refusal(kind: DropKind, start: bool) -> String {
    match kind {
        DropKind::Unsupported => tr!("AniCraft opens .anicraft, .fla and .xfl documents, and imports SVG drawings, PNG and JPEG images, and WAV, Ogg and MP3 sounds").into(),
        _ if start => tr!("Open or create a document first, then drop drawings, images and sounds on its stage").into(),
        _ => tr!("Drop drawings, images and sounds on the stage or a layer's frame to place them, or on the Library to add them").into(),
    }
}

/// What dropping `files` at `spot` does. One document opens (AniCraft has one document
/// window); other files in the same drop are left out then.
pub fn plan(files: &[PathBuf], spot: Spot) -> Plan {
    let mut out = Plan::default();
    if let Some(doc) = files.iter().find(|f| kind_of(f) == DropKind::Document) {
        out.open = Some(doc.clone());
        for f in files.iter().filter(|f| *f != doc) {
            out.left_out.push((f.clone(), tr!("AniCraft opens one document at a time; drop the others on their own").into()));
        }
        return out;
    }
    let to = match spot {
        Spot::Library(_) => DropTarget::Library,
        Spot::Timeline(..) => DropTarget::Timeline,
        // Without a position, the stage (at the centre of the view).
        Spot::Stage(..) | Spot::Unknown => DropTarget::Stage,
        Spot::Window | Spot::Start => DropTarget::Window,
    };
    for f in files {
        let kind = kind_of(f);
        match route(kind, to) {
            DropAction::ImportToStage | DropAction::ImportToLibrary | DropAction::ImportToTimeline => out.import.push(f.clone()),
            DropAction::Open => {}
            DropAction::Refuse(_) => out.left_out.push((f.clone(), refusal(kind, spot == Spot::Start))),
        }
    }
    if !out.import.is_empty() {
        out.to = Some(to);
    }
    out
}

/// A drop waiting for the pointer (desktop), or for the browser to read the files (web).
struct Waiting {
    files: Vec<PathBuf>,
    since: f64,
    /// Browser files still being read, by ticket.
    #[cfg(target_arch = "wasm32")]
    ticket: Option<u64>,
}

/// The drop state the window keeps between frames.
#[derive(Default)]
pub struct DropState {
    /// Where the stage was drawn this frame (set by `stage::show`).
    pub stage: Option<StageGeo>,
    /// Where the timeline's rows were drawn this frame (set by `timeline::show`).
    pub timeline: Option<TimelineGeo>,
    waiting: Option<Waiting>,
    /// The pointer moved while files were dragged over the window: its position is current.
    tracked: bool,
    /// A drag shown without one (screenshots and tests): the files and where they hover.
    pub preview: Option<(Vec<PathBuf>, PreviewAt)>,
    #[cfg(target_arch = "wasm32")]
    web_spot: Option<Spot>,
}

/// Where a previewed drag hovers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewAt {
    Stage,
    Library,
    Timeline,
}

/// Where a drop at `p` lands, from this frame's layout.
pub fn spot_at(app: &App, p: Option<Pos2>) -> Spot {
    if app.start.is_some() {
        return Spot::Start;
    }
    let Some(p) = p else { return Spot::Unknown };
    if let Some(folder) = app.library.drop_zone.folder_at(p) {
        return Spot::Library(folder);
    }
    if let Some((layer, frame, _)) = app.drop.timeline.as_ref().and_then(|t| t.at(p, app.s.frame)) {
        return Spot::Timeline(layer, frame);
    }
    match app.drop.stage {
        Some(g) if g.canvas.contains(p) => {
            let (x, y) = g.to_stage(p);
            Spot::Stage(x, y)
        }
        _ => Spot::Window,
    }
}

/// Before the panels draw: forget last frame's targets (a hidden panel takes nothing).
pub fn begin_frame(app: &mut App) {
    app.drop.stage = None;
    app.drop.timeline = None;
    app.library.drop_zone = Default::default();
}

/// The pointer's position over a dragged file, if it is known.
fn drag_pointer(app: &App, ctx: &egui::Context) -> Option<Pos2> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = app;
        let z = ctx.zoom_factor().max(1e-3);
        crate::web::drag_pos().map(|(x, y)| egui::pos2(x / z, y / z))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if app.drop.tracked { ctx.input(|i| i.pointer.latest_pos()) } else { None }
    }
}

/// After the panels drew: the highlight while files hover, and what was dropped.
pub fn frame(app: &mut App, ctx: &egui::Context) {
    let (hovered, dropped, moved, now) = ctx.input(|i| {
        let hovered: Vec<PathBuf> = i.raw.hovered_files.iter().map(|h| h.path.clone().unwrap_or_else(|| PathBuf::from(guess_name(&h.mime)))).collect();
        let moved = i.events.iter().any(|e| matches!(e, egui::Event::PointerMoved(_)));
        (hovered, i.raw.dropped_files.clone(), moved, i.time)
    });
    if !hovered.is_empty() {
        app.drop.tracked |= moved;
        let p = drag_pointer(app, ctx);
        highlight(app, ctx, &hovered, p);
    } else if let Some((files, at)) = app.drop.preview.clone() {
        let p = preview_point(app, at);
        highlight(app, ctx, &files, p);
    }
    if !dropped.is_empty() {
        let files: Vec<PathBuf> = dropped.iter().take(anicraft_engine::cmds_drop::MAX_DROPPED).map(|f| f.path().to_path_buf()).collect();
        if dropped.len() > anicraft_engine::cmds_drop::MAX_DROPPED {
            app.status_err(tr!("Drop at most {n} files at once", n = anicraft_engine::cmds_drop::MAX_DROPPED));
            app.drop.tracked = false;
            return;
        }
        app.playing = false;
        #[cfg(target_arch = "wasm32")]
        {
            // The browser said where the drop is; it reads the files' bytes, which arrive later.
            let spot = spot_at(app, drag_pointer(app, ctx));
            let handles: Vec<web_sys::File> = dropped.iter().filter_map(|f| f.web_file().cloned()).collect();
            let ticket = crate::web::read_dropped(handles);
            app.drop.web_spot = Some(spot);
            app.drop.waiting = Some(Waiting { files, since: now, ticket: Some(ticket) });
            crate::web::clear_drag_pos();
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            if app.drop.tracked {
                let spot = spot_at(app, ctx.input(|i| i.pointer.latest_pos()));
                run(app, &files, spot);
            } else {
                app.drop.waiting = Some(Waiting { files, since: now });
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
            }
        }
        app.drop.tracked = false;
        return;
    }
    if hovered.is_empty() {
        // A drag that left the window (or was cancelled) starts over.
        app.drop.tracked = false;
    }
    resolve_waiting(app, ctx, moved, now);
}

#[cfg(not(target_arch = "wasm32"))]
fn resolve_waiting(app: &mut App, ctx: &egui::Context, moved: bool, now: f64) {
    let Some(w) = &app.drop.waiting else { return };
    let timed_out = now - w.since > WAIT_FOR_POINTER;
    if !moved && !timed_out {
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
        return;
    }
    let Some(w) = app.drop.waiting.take() else { return };
    let p = if moved { ctx.input(|i| i.pointer.latest_pos()) } else { None };
    run(app, &w.files, spot_at(app, p));
}

#[cfg(target_arch = "wasm32")]
fn resolve_waiting(app: &mut App, _ctx: &egui::Context, _moved: bool, _now: f64) {
    let Some((ticket, read)) = crate::web::take_dropped() else { return };
    if app.drop.waiting.as_ref().and_then(|w| w.ticket) != Some(ticket) {
        return;
    }
    let (Some(w), Some(spot)) = (app.drop.waiting.take(), app.drop.web_spot.take()) else { return };
    let _ = WAIT_FOR_POINTER;
    let mut files = Vec::new();
    for (name, data) in read {
        // Only the name: the browser never says which folder a file came from.
        let path = PathBuf::from(&name);
        match data.and_then(|d| anicraft_engine::vfs::put(&path, d).map_err(|e| e.to_string())) {
            Ok(()) => files.push(path),
            Err(e) => app.status_err(tr!("Could not open {name}: {e}", name = name, e = e)),
        }
    }
    let _ = w.files;
    if !files.is_empty() {
        run(app, &files, spot);
    }
}

/// Carries out a drop.
pub fn run(app: &mut App, files: &[PathBuf], spot: Spot) {
    let plan = plan(files, spot);
    let names = |list: &[PathBuf]| list.iter().map(|p| crate::recent::display_name(p)).collect::<Vec<_>>().join(", ");
    if let Some(doc) = &plan.open {
        app.start = None;
        app.request("file.open", json!({"path": doc.display().to_string()}));
        if let Some((_, why)) = plan.left_out.first() {
            app.status_ok(tr!("Left out {files}: {why}", files = names(&plan.left_out.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>()), why = why));
        }
        return;
    }
    let Some(to) = plan.to else {
        // Nothing to import: one message says why.
        if let Some((p, why)) = plan.left_out.first() {
            app.status_err(tr!("Can't import {file}: {why}", file = crate::recent::display_name(p), why = why));
        }
        return;
    };
    let paths: Vec<String> = plan.import.iter().map(|p| p.display().to_string()).collect();
    let n = plan.import.len();
    let done = match (to, spot) {
        (DropTarget::Library, Spot::Library(folder)) => {
            let v = app.run("file.drop", json!({"paths": paths, "to": "library", "folder": folder.map(|f| f.0)}));
            if let Some(f) = folder {
                app.library.expanded.extend(app.s.doc.folder_chain(f));
            }
            v.map(|v| {
                // The last file added is the Library's current item.
                let last = v["items"].as_array().and_then(|a| a.last()).cloned().unwrap_or_default();
                match (last["bitmap"].as_u64(), last["sound"].as_u64()) {
                    (Some(b), _) => app.library_sel = Some(anicraft_engine::doc::LibraryItem::Bitmap(anicraft_engine::doc::BitmapId(b))),
                    (None, Some(snd)) => app.library_sel = Some(anicraft_engine::doc::LibraryItem::Sound(anicraft_engine::doc::SoundId(snd))),
                    _ => {}
                }
                match folder.and_then(|f| app.s.doc.folder(f)).map(|f| f.name.clone()) {
                    Some(name) => trn!("Added {n} file to “{folder}” in the Library", "Added {n} files to “{folder}” in the Library", n, folder = name),
                    None => trn!("Added {n} file to the Library", "Added {n} files to the Library", n),
                }
            })
        }
        (DropTarget::Timeline, Spot::Timeline(layer, frame)) => {
            // Centred in the view, in the edited space, as Paste puts things.
            let c = view_centre(app);
            let v = app.run("file.drop", json!({"paths": paths, "to": "timeline", "layer": layer.0, "frame": frame, "x": c.0, "y": c.1}));
            v.map(|v| {
                let name = app.s.timeline().layer(layer).map(|l| l.name.clone()).unwrap_or_default();
                let (frame, undo) = (frame + 1, app.undo_key());
                if v["keyframe_made"].as_bool() == Some(true) {
                    trn!("Placed {n} file on {layer} at frame {frame}, in a new keyframe — {undo} to undo", "Placed {n} files on {layer} at frame {frame}, in a new keyframe — {undo} to undo", n, layer = name, frame = frame, undo = undo)
                } else {
                    trn!("Placed {n} file on {layer} at frame {frame} — {undo} to undo", "Placed {n} files on {layer} at frame {frame} — {undo} to undo", n, layer = name, frame = frame, undo = undo)
                }
            })
        }
        _ => {
            let geo = app.drop.stage;
            if geo.is_some_and(|g| g.paused) {
                app.status_ok(tr!("Previewing through the camera: artwork edits are paused. Turn off View › Preview Through Camera to edit."));
                return;
            }
            let (centred, (x, y)) = match spot {
                Spot::Stage(x, y) => (false, (x, y)),
                _ => (true, view_centre(app)),
            };
            let v = app.run("file.drop", json!({"paths": paths, "to": "stage", "x": x, "y": y}));
            v.map(|_| {
                let layer = app.s.active_layer().map(|l| l.name.clone()).unwrap_or_default();
                let (frame, undo) = (app.s.frame + 1, app.undo_key());
                if centred {
                    trn!("Placed {n} file on {layer} at frame {frame}, in the middle of the view — {undo} to undo", "Placed {n} files on {layer} at frame {frame}, in the middle of the view — {undo} to undo", n, layer = layer, frame = frame, undo = undo)
                } else {
                    trn!("Placed {n} file on {layer} at frame {frame} — {undo} to undo", "Placed {n} files on {layer} at frame {frame} — {undo} to undo", n, layer = layer, frame = frame, undo = undo)
                }
            })
        }
    };
    if let Some(msg) = done {
        app.decode_sounds();
        match plan.left_out.first() {
            Some((p, why)) => app.status_ok(tr!("{done}. Left out {file}: {why}", done = msg, file = crate::recent::display_name(p), why = why)),
            None => app.status_ok(msg),
        }
    }
}

/// Files the system asked the running app to open (macOS: a double-click in Finder, a drop on
/// the Dock icon, Open With; ADR 0062). The first document opens as a drop on the window would,
/// through the unsaved-changes guard; nothing is imported this way.
pub fn open_from_system(app: &mut App, files: &[PathBuf]) {
    let files: Vec<PathBuf> = files.iter().take(anicraft_engine::cmds_drop::MAX_DROPPED).cloned().collect();
    if files.iter().any(|f| kind_of(f) == DropKind::Document) {
        app.playing = false;
        run(app, &files, Spot::Window);
    } else if let Some(f) = files.first() {
        app.status_err(tr!("Can't open {file}: {why}", file = crate::recent::display_name(f), why = refusal(DropKind::Unsupported, false)));
    }
}

/// The middle of what the stage shows, in the edited space.
fn view_centre(app: &App) -> (f32, f32) {
    let c = app.view_center_stage();
    match app.s.edit_view() {
        Some(v) if v.scale.abs() > 1e-6 => ((c.x - v.origin.x) / v.scale, (c.y - v.origin.y) / v.scale),
        _ => (c.x, c.y),
    }
}

/// Why a layer can't take a drop (the engine refuses the same), in the interface language.
fn layer_refusal(app: &App, layer: LayerId) -> Option<String> {
    let tl = app.s.timeline();
    let l = tl.layer(layer)?;
    match l.kind {
        LayerKind::Folder => Some(tr!("{layer} is a folder: drop on one of its layers", layer = l.name)),
        LayerKind::Camera => Some(tr!("{layer} is the camera layer: drop on a layer below it", layer = l.name)),
        LayerKind::Armature => Some(tr!("{layer} holds an armature: drop on another layer", layer = l.name)),
        _ if l.locked || tl.locked_by_folder(layer) => Some(tr!("{layer} is locked: unlock it to drop onto it", layer = l.name)),
        _ => None,
    }
}

/// A name for a file the browser describes only by its type while it is dragged.
fn guess_name(mime: &str) -> &'static str {
    match mime {
        "image/svg+xml" => "drawing.svg",
        "image/png" => "image.png",
        "image/jpeg" => "image.jpg",
        "audio/wav" | "audio/x-wav" | "audio/wave" => "sound.wav",
        "audio/ogg" => "sound.ogg",
        "audio/mpeg" | "audio/mp3" => "sound.mp3",
        _ => "",
    }
}

/// Where a previewed drag hovers on screen: the stage's middle, or the Library's first folder.
fn preview_point(app: &App, at: PreviewAt) -> Option<Pos2> {
    match at {
        PreviewAt::Stage => app.drop.stage.map(|g| g.canvas.center() + vec2(-g.canvas.width() * 0.12, -g.canvas.height() * 0.08)),
        PreviewAt::Library => app.library.drop_zone.rows.iter().find(|(_, f)| f.is_some()).map(|(r, _)| r.center()),
        // A few frames past the playhead on the second row (the first when there is one).
        PreviewAt::Timeline => app.drop.timeline.as_ref().and_then(|t| {
            let (row, _) = t.rows.get(1).or_else(|| t.rows.first())?;
            Some(pos2(t.x0 + (app.s.frame as f32 + 8.5) * t.cell_w, row.center().y))
        }),
    }
}

/// The drop highlight: what lands where, drawn over everything.
fn highlight(app: &App, ctx: &egui::Context, files: &[PathBuf], p: Option<Pos2>) {
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("file_drop")));
    let screen = ctx.content_rect().shrink(3.0);
    // Only the names are known while a browser drags (and sometimes not even those).
    let known: Vec<&PathBuf> = files.iter().filter(|f| !f.as_os_str().is_empty()).collect();
    let docs: Vec<&&PathBuf> = known.iter().filter(|f| kind_of(f) == DropKind::Document).collect();
    let stroke = Stroke::new(2.0, T.accent);
    let label = |at: Pos2, text: String, ok: bool| {
        let c = if ok { color(Type::Body) } else { T.error };
        let galley = painter.layout_no_wrap(text, font(Type::Label), c);
        let r = Rect::from_min_size(at, vec2(galley.rect.width(), galley.rect.height())).expand(6.0);
        // Kept on screen.
        let r = r.translate(vec2((screen.right() - r.right()).min(0.0), (screen.bottom() - r.bottom()).min(0.0)));
        painter.rect_filled(r, T.rc(), T.panel);
        painter.rect_stroke(r, T.rc(), Stroke::new(1.0, if ok { T.accent } else { T.error }), StrokeKind::Inside);
        painter.galley(r.min + vec2(6.0, 6.0), galley, c);
    };
    if let Some(doc) = docs.first() {
        painter.rect_stroke(screen, T.rc(), stroke, StrokeKind::Inside);
        let text = if app.s.is_dirty() { tr!("Open {name} (you'll be asked about unsaved changes)", name = crate::recent::display_name(doc)) } else { tr!("Open {name}", name = crate::recent::display_name(doc)) };
        label(screen.center_top() + vec2(-120.0, 12.0), text, true);
        return;
    }
    let spot = spot_at(app, p);
    let what = match known.len() {
        0 => tr!("these files").to_string(),
        1 => crate::recent::display_name(known[0]),
        n => trn!("{n} file", "{n} files", n),
    };
    let fits = |to: DropTarget| known.iter().all(|f| fits_one(f, to));
    match (spot, p) {
        (Spot::Stage(..), Some(p)) => {
            if let Some(g) = app.drop.stage {
                painter.rect_stroke(g.canvas.shrink(1.0), crate::theme::SQUARE, stroke, StrokeKind::Inside);
            }
            let r = 8.0;
            painter.line_segment([p - vec2(r, 0.0), p + vec2(r, 0.0)], Stroke::new(1.5, T.accent));
            painter.line_segment([p - vec2(0.0, r), p + vec2(0.0, r)], Stroke::new(1.5, T.accent));
            let ok = fits(DropTarget::Stage);
            let layer = app.s.active_layer().map(|l| l.name.clone()).unwrap_or_default();
            let text = if ok { tr!("Place {what} on {layer} at frame {frame}", what = what, layer = layer, frame = app.s.frame + 1) } else { refusal(DropKind::Unsupported, false) };
            label(p + vec2(14.0, 12.0), text, ok);
        }
        (Spot::Timeline(layer, frame), Some(p)) => {
            // The row outlined and the frame's cell filled: where it lands.
            if let Some((row, cell)) = app.drop.timeline.as_ref().and_then(|t| Some((t.rows.iter().find(|(_, l)| *l == layer)?.0, t.at(p, app.s.frame)?.2))) {
                painter.rect_stroke(row, crate::theme::SQUARE, Stroke::new(1.5, T.accent), StrokeKind::Inside);
                painter.rect_filled(cell, crate::theme::SQUARE, T.accent.gamma_multiply(0.45));
                painter.rect_stroke(cell, crate::theme::SQUARE, stroke, StrokeKind::Inside);
            }
            let name = app.s.timeline().layer(layer).map(|l| l.name.clone()).unwrap_or_default();
            let sounds_only = !known.is_empty() && known.iter().all(|f| kind_of(f) == DropKind::Sound);
            let (ok, text) = match layer_refusal(app, layer) {
                Some(why) => (false, why),
                None if !fits(DropTarget::Timeline) => (false, refusal(DropKind::Unsupported, false)),
                None if sounds_only => (true, tr!("Put {what} on {layer}'s keyframe at frame {frame}", what = what, layer = name, frame = frame + 1)),
                None => (true, tr!("Place {what} on {layer} at frame {frame}", what = what, layer = name, frame = frame + 1)),
            };
            label(p + vec2(14.0, 12.0), text, ok);
        }
        (Spot::Library(folder), Some(p)) => {
            let zone = &app.library.drop_zone;
            match zone.row_at(p).filter(|_| folder.is_some()) {
                Some(row) => {
                    painter.rect_stroke(row, T.rc(), stroke, StrokeKind::Inside);
                }
                None => {
                    if let Some(panel) = zone.panel {
                        painter.rect_stroke(panel, T.rc(), stroke, StrokeKind::Inside);
                    }
                }
            }
            let ok = fits(DropTarget::Library);
            let text = match (ok, folder.and_then(|f| app.s.doc.folder(f))) {
                (true, Some(f)) => tr!("Add {what} to “{folder}”", what = what, folder = f.name),
                (true, None) => tr!("Add {what} to the Library", what = what),
                (false, _) => match known.iter().find(|f| !fits_one(f, DropTarget::Library)) {
                    Some(f) => refusal(kind_of(f), false),
                    None => refusal(DropKind::Unsupported, false),
                },
            };
            label(p + vec2(14.0, 12.0), text, ok);
        }
        (Spot::Unknown, _) => {
            // The desktop doesn't say where the pointer is during a drag: say what each place does.
            painter.rect_stroke(screen, T.rc(), Stroke::new(1.5, T.accent), StrokeKind::Inside);
            label(screen.center_top() + vec2(-180.0, 12.0), tr!("Drop {what} on the stage or a layer's frame to place it, or on the Library to add it", what = what), true);
        }
        (spot, p) => {
            let at = p.unwrap_or(screen.center_top()) + vec2(14.0, 12.0);
            label(at, refusal(known.first().map_or(DropKind::Image, |f| kind_of(f)), spot == Spot::Start), false);
        }
    }
}

fn fits_one(f: &Path, to: DropTarget) -> bool {
    matches!(route(kind_of(f), to), DropAction::ImportToStage | DropAction::ImportToLibrary | DropAction::ImportToTimeline)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn drops_are_routed_by_type_and_target() {
        // A document opens wherever it lands, and only one at a time.
        for spot in [Spot::Stage(1.0, 2.0), Spot::Library(None), Spot::Timeline(LayerId(1), 0), Spot::Window, Spot::Start, Spot::Unknown] {
            let plan = plan(&[p("walk.anicraft"), p("photo.png"), p("old.fla")], spot);
            assert_eq!(plan.open, Some(p("walk.anicraft")), "{spot:?}");
            assert!(plan.import.is_empty());
            assert_eq!(plan.left_out.len(), 2);
        }
        let plan_at = |spot| plan(&[p("a.svg"), p("b.png"), p("c.mp3"), p("d.psd")], spot);
        // On the stage: everything AniCraft reads, the rest left out with a reason.
        let on_stage = plan_at(Spot::Stage(10.0, 20.0));
        assert_eq!((on_stage.to, on_stage.import.len(), on_stage.left_out.len()), (Some(DropTarget::Stage), 3, 1));
        assert!(on_stage.left_out[0].1.contains("SVG"), "says what AniCraft reads");
        // On the Library: drawings (as graphic symbols), images and sounds.
        let in_library = plan_at(Spot::Library(Some(FolderId(7))));
        assert_eq!((in_library.to, in_library.import.len(), in_library.left_out.len()), (Some(DropTarget::Library), 3, 1));
        // On a layer's frame: the same three.
        let on_frame = plan_at(Spot::Timeline(LayerId(3), 9));
        assert_eq!((on_frame.to, on_frame.import.len(), on_frame.left_out.len()), (Some(DropTarget::Timeline), 3, 1));
        // Elsewhere on the window, or over the start screen: nothing imports.
        for spot in [Spot::Window, Spot::Start] {
            let plan = plan_at(spot);
            assert!(plan.import.is_empty() && plan.to.is_none() && plan.left_out.len() == 4, "{spot:?}");
        }
        // No pointer position: the stage, at the middle of the view.
        assert_eq!(plan_at(Spot::Unknown).to, Some(DropTarget::Stage));
        // Only unsupported files: nothing to do but say why.
        let plan = plan(&[p("notes.txt")], Spot::Stage(0.0, 0.0));
        assert!(plan.to.is_none() && plan.open.is_none() && plan.left_out.len() == 1);
    }

    #[test]
    fn a_drop_lands_where_the_layout_says() {
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, anicraft_engine::Session::new(anicraft_engine::doc::sample::demo()), Some(PathBuf::from("unused.png")));
        app.run("library.new_folder", json!({"name": "Props"}));
        let screen = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 940.0))), ..Default::default() };
        for _ in 0..3 {
            let mut out = ctx.run_ui(screen.clone(), |ui| app.frame(ui));
            out.textures_delta.clear();
        }
        let geo = app.drop.stage.expect("the stage drew");
        // The middle of the canvas maps into the stage.
        match spot_at(&app, Some(geo.canvas.center())) {
            Spot::Stage(x, y) => assert!(x.is_finite() && y.is_finite()),
            other => panic!("{other:?}"),
        }
        let (row, folder) = *app.library.drop_zone.rows.iter().find(|(_, f)| f.is_some()).expect("the folder row drew");
        assert_eq!(spot_at(&app, Some(row.center())), Spot::Library(folder));
        assert_eq!(spot_at(&app, Some(egui::pos2(1.0, 1.0))), Spot::Window, "the menu bar takes nothing");
        // A timeline row: its layer, and the frame under the pointer (the playhead's over the
        // layer column).
        let tl = app.drop.timeline.clone().expect("the timeline drew");
        let (row, layer) = tl.rows[1];
        let x = tl.x0 + 6.5 * tl.cell_w;
        assert_eq!(spot_at(&app, Some(egui::pos2(x, row.center().y))), Spot::Timeline(layer, 6));
        assert_eq!(spot_at(&app, Some(egui::pos2(row.left() + 30.0, row.center().y))), Spot::Timeline(layer, app.s.frame));
        assert_eq!(spot_at(&app, None), Spot::Unknown);
        crate::start::open(&mut app);
        assert_eq!(spot_at(&app, Some(geo.canvas.center())), Spot::Start);
    }

    #[test]
    fn dropped_media_land_on_the_stage_or_in_a_library_folder() {
        let dir = std::env::temp_dir().join(format!("anicraft-shell-drop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("dot.png");
        std::fs::write(&png, anicraft_engine::render::encode_png(2, 2, &[200u8; 16]).unwrap()).unwrap();
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, anicraft_engine::Session::new(anicraft_engine::doc::sample::demo()), Some(PathBuf::from("unused.png")));
        let bitmaps = app.s.doc.bitmaps.len();
        run(&mut app, std::slice::from_ref(&png), Spot::Stage(50.0, 40.0));
        assert_eq!(app.s.doc.bitmaps.len(), bitmaps + 1);
        assert_eq!(app.s.selection.len(), 1);
        assert!(app.status.as_ref().is_some_and(|s| !s.error), "{:?}", app.status.as_ref().map(|s| &s.text));
        let folder = app.run("library.new_folder", json!({"name": "Props"})).and_then(|v| v["folder"].as_u64()).map(FolderId).unwrap();
        run(&mut app, std::slice::from_ref(&png), Spot::Library(Some(folder)));
        assert_eq!(app.s.doc.folder_contents(folder).len(), 1);
        assert!(app.library.expanded.contains(&folder), "the folder opens to show it");
        // On a layer's frame: onto that layer, in a keyframe made there, one undo step.
        let star = app.s.timeline().layers[1].id;
        let undo = app.s.history_len();
        run(&mut app, std::slice::from_ref(&png), Spot::Timeline(star, 9));
        assert_eq!(app.s.history_len(), undo + 1);
        assert!(app.s.timeline().layer(star).and_then(|l| l.keyframe_at(9)).is_some_and(|k| k.start == 9 && k.elements.iter().any(|e| app.s.selection.contains(star, e.id))));
        assert!(app.status.as_ref().is_some_and(|s| !s.error && s.text.contains("new keyframe")), "{:?}", app.status.as_ref().map(|s| &s.text));
        // A locked layer says why, and nothing changes.
        app.s.layer = star;
        app.run("layer.toggle_lock", json!({}));
        let before = app.s.doc.clone();
        run(&mut app, std::slice::from_ref(&png), Spot::Timeline(star, 12));
        assert!(app.s.doc == before && app.status.as_ref().is_some_and(|s| s.error && s.text.contains("locked")));
        // An unsupported file: one clear message, nothing changes.
        let before = app.s.doc.clone();
        run(&mut app, &[dir.join("notes.txt")], Spot::Stage(0.0, 0.0));
        assert!(app.s.doc == before);
        assert!(app.status.as_ref().is_some_and(|s| s.error && s.text.contains("notes.txt")));
        // A document over unsaved work asks first.
        run(&mut app, &[dir.join("other.anicraft")], Spot::Stage(0.0, 0.0));
        assert!(app.confirm.is_some());
    }

    /// Documents the system hands the running app (macOS Finder, the Dock; ADR 0062) arrive as
    /// requests, are read at the start of a frame, and open through the unsaved-changes guard.
    #[test]
    fn documents_the_system_hands_over_open_through_the_guard() {
        use crate::platform::{SystemRequest, push_request};
        let dir = std::env::temp_dir().join(format!("anicraft-shell-system-open-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (walk, other) = (dir.join("walk.anicraft"), dir.join("other.anicraft"));
        for f in [&walk, &other] {
            let mut s = anicraft_engine::Session::new(anicraft_engine::doc::sample::demo());
            s.execute("file.save", &json!({"path": f.display().to_string()})).unwrap();
        }
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, anicraft_engine::Session::new(anicraft_engine::doc::sample::demo()), Some(PathBuf::from("unused.png")));
        crate::start::open(&mut app);
        let frame = |app: &mut App| {
            let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 940.0))), ..Default::default() }, |ui| app.frame(ui));
            out.textures_delta.clear();
            out
        };
        // At launch (macOS sends the launching document this way too): the start screen gives
        // way to it; a picture beside it is left out, and the status says so.
        push_request(SystemRequest::Open(vec![dir.join("photo.png"), walk.clone()]));
        frame(&mut app);
        assert_eq!(app.s.path.as_deref(), Some(walk.as_path()));
        assert!(app.start.is_none(), "the start screen gave way");
        assert!(app.status.as_ref().is_some_and(|s| s.text.contains("photo.png")), "{:?}", app.status.as_ref().map(|s| &s.text));
        // While running, over unsaved work: it asks first, and nothing changes until it's answered.
        app.run("layer.new", json!({}));
        assert!(app.s.is_dirty());
        push_request(SystemRequest::Open(vec![other.clone()]));
        frame(&mut app);
        assert!(app.confirm.as_ref().is_some_and(|c| c.id == "file.open"), "the unsaved-changes guard asks");
        assert_eq!(app.s.path.as_deref(), Some(walk.as_path()));
        app.confirm = None;
        // Only files AniCraft can't open: one clear message, nothing else.
        push_request(SystemRequest::Open(vec![dir.join("notes.txt")]));
        frame(&mut app);
        assert!(app.status.as_ref().is_some_and(|s| s.error && s.text.contains("notes.txt")));
        assert!(app.confirm.is_none());
        // The Dock's Quit closes the editor's window, which the close guard intercepts.
        push_request(SystemRequest::Quit);
        let out = frame(&mut app);
        let closes = out.viewport_output.get(&egui::ViewportId::ROOT).is_some_and(|v| v.commands.contains(&egui::ViewportCommand::Close));
        assert!(closes, "a quit request closes the editor's window");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
