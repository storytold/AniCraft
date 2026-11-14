//! The stage: tiled pictures from the render service, tool interaction and screen-space
//! overlays (ADR 0007).
//!
//! The stage is cut into `TILE`-pixel tiles on a device-pixel grid at the current zoom. Each
//! frame asks for the visible tiles as one batch (plus a one-tile margin as look-ahead), keeps
//! finished tiles as textures, and draws any older tiles that cover a gap (previous zoom or
//! state, scaled) underneath, so panning and zooming never reveal an empty stage.

use crate::app::{App, Drag, Tool, XfKind};
use crate::prefs::RenderMode;
use crate::theme::{T, to_color32};
use anicraft_engine::geom::{Affine, Vec2};
use anicraft_engine::render::View;
use anicraft_engine::render_service::{DEFAULT_CACHE_BYTES, FrameImage, FrameJob, RenderService, make_job_for};
use crate::a11y::Named;
use anicraft_engine::cmds_raw::Grab;
use egui::{Color32, CursorIcon, Key, Modifiers, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use serde_json::json;

/// Frames rendered ahead of the playhead during playback.
const LOOKAHEAD: u32 = 12;
/// In Auto mode, edits render on the UI thread when the visible tiles last took at most this
/// long in total (keeps 60 fps interaction; heavier scenes fall back to the background worker).
const IMMEDIATE_BUDGET_MS: f32 = 12.0;
/// Tile edge in device pixels: big enough to keep per-tile overhead low, small enough that a
/// pan only needs a strip of new tiles.
pub const TILE: u32 = 512;
/// Tile textures kept on the GPU (≈ 1 MB each); least recently used ones are dropped.
const MAX_TILE_TEXTURES: usize = 192;

/// A finished tile on the GPU.
struct TileTex {
    key: u64,
    /// Stage-space rectangle `[x0, y0, x1, y1]` this tile covers.
    rect: [f32; 4],
    tex: TileImage,
    used: u64,
}

/// A tile's texture: uploaded from the CPU's bytes, or drawn there by the GPU stage (ADR 0029).
enum TileImage {
    Uploaded(egui::TextureHandle),
    Drawn(crate::gpu_stage::NativeTile),
}

impl TileImage {
    fn id(&self) -> egui::TextureId {
        match self {
            TileImage::Uploaded(t) => t.id(),
            TileImage::Drawn(t) => t.id(),
        }
    }
}

pub struct Cache {
    tiles: Vec<TileTex>,
    tick: u64,
    /// Render time of the visible tiles the last time they were (re)rendered.
    pub ms: f32,
    /// `(ready, needed)` visible tiles this frame.
    pub ready: (usize, usize),
    pub canvas: Rect,
    pub service: Option<RenderService>,
    /// Document state of the picture on screen (an edit makes it differ).
    pub shown_state: Option<anicraft_engine::StateId>,
    /// The GPU stage (ADR 0019), when this build has it and Preferences ask for it.
    pub gpu: crate::gpu_stage::GpuStage,
    /// Who drew the newest visible tiles: the GPU, or the CPU and why (status bar).
    pub drawn_by: Option<(bool, Option<String>)>,
    /// The selection pattern's texture and the colour it was made in (`selection_pattern`).
    pattern: Option<(Color32, egui::TextureHandle)>,
}

impl Default for Cache {
    fn default() -> Self {
        Cache { tiles: Vec::new(), tick: 0, ms: 0.0, ready: (0, 0), canvas: Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)), service: None, shown_state: None, gpu: Default::default(), drawn_by: None, pattern: None }
    }
}

impl Cache {
    /// `(images, MB, hit %)` of the render cache, for the status bar.
    pub fn stats(&self) -> Option<(usize, f32, f32)> {
        self.service.as_ref().map(|s| {
            let (n, bytes, hits, misses) = s.cache_stats();
            let total = (hits + misses).max(1);
            (n, bytes as f32 / (1024.0 * 1024.0), hits as f32 * 100.0 / total as f32)
        })
    }

    fn has(&mut self, key: u64) -> bool {
        self.tick += 1;
        let tick = self.tick;
        match self.tiles.iter_mut().find(|t| t.key == key) {
            Some(t) => {
                t.used = tick;
                true
            }
            None => false,
        }
    }

    fn upload(&mut self, ctx: &egui::Context, img: &FrameImage, rect: [f32; 4], keep: &[u64]) -> Option<String> {
        if let Some(e) = &img.error {
            return Some(e.clone());
        }
        if self.tiles.iter().any(|t| t.key == img.key) {
            return None;
        }
        let tex = match &img.native {
            // Already on the window's device: shown as it is, nothing to upload.
            Some(native) => match self.gpu.register(native) {
                Some(t) => TileImage::Drawn(t),
                // English, as the render service's own errors are: translated where it shows.
                None => return Some(tr_noop!("The GPU stage drew a picture the window can't show.").into()),
            },
            None => {
                let image = egui::ColorImage::from_rgba_premultiplied([img.width as usize, img.height as usize], &img.rgba_premul);
                TileImage::Uploaded(ctx.load_texture(format!("tile-{:x}", img.key), image, egui::TextureOptions::LINEAR))
            }
        };
        self.tick += 1;
        self.tiles.push(TileTex { key: img.key, rect, tex, used: self.tick });
        // Evict least recently used tiles that are not on screen.
        while self.tiles.len() > MAX_TILE_TEXTURES {
            let victim = self.tiles.iter().enumerate().filter(|(_, t)| !keep.contains(&t.key)).min_by_key(|(_, t)| t.used).map(|(i, _)| i);
            match victim {
                Some(i) => {
                    self.tiles.swap_remove(i);
                }
                None => break,
            }
        }
        None
    }
}

/// One tile of the stage at device scale `z`.
struct TileSpec {
    rect: [f32; 4],
    view: View,
}

/// Most tiles along one side: a screen's worth and the look-ahead ring, many times over.
const MAX_TILES_PER_AXIS: usize = 64;

/// The tile spans `(offset, length)` in device pixels along one axis of a stage `len` long that
/// overlap `[lo, hi)`. The stage is cut into `TILE`s from its edge; with `pasteboard`, so is the
/// space before and after it (from the stage's edges outwards), so no tile straddles the edge.
fn spans(len: u32, lo: f32, hi: f32, pasteboard: bool) -> Vec<(i64, u32)> {
    let t = i64::from(TILE);
    let len = i64::from(len);
    let (lo, hi) = (lo.floor().clamp(-1e9, 1e9) as i64, hi.ceil().clamp(-1e9, 1e9) as i64);
    let mut out = Vec::new();
    if pasteboard && lo < 0 {
        // Before the stage: [-(k+1)T, -kT), nearest the stage first.
        let first = (-hi).max(0) / t;
        let last = (-lo - 1) / t;
        let first = first.max(last - MAX_TILES_PER_AXIS as i64 + 1);
        out.extend((first..=last).rev().map(|k| (-(k + 1) * t, TILE)));
    }
    if hi > 0 && lo < len {
        let first = lo.max(0) / t;
        let last = (hi.min(len) - 1) / t;
        out.extend((first..=last).take(MAX_TILES_PER_AXIS).map(|k| (k * t, (t.min(len - k * t)) as u32)));
    }
    if pasteboard && hi > len {
        // After it: [len + kT, len + (k+1)T).
        let first = (lo - len).max(0) / t;
        let last = (hi - len - 1) / t;
        out.extend((first..=last).take(MAX_TILES_PER_AXIS).map(|k| (len + k * t, TILE)));
    }
    out
}

/// Tiles covering stage-device-pixel rect `[x0, x1) × [y0, y1)` (and `margin` tiles around it),
/// nearest to its centre first. The stage's tiles are filled with `bg`; with `pasteboard` the
/// view beyond the stage gets tiles too, transparent over the pasteboard (#245), so artwork
/// there shows while the stage keeps its exact colour and blending.
fn tiles_for(z: f32, stage_px: (u32, u32), px: [f32; 4], margin: u32, bg: Option<anicraft_engine::doc::Rgba>, pasteboard: bool) -> Vec<TileSpec> {
    let (sw, sh) = stage_px;
    if sw == 0 || sh == 0 {
        return Vec::new();
    }
    let m = (margin * TILE) as f32;
    let xs = spans(sw, px[0] - m, px[2] + m, pasteboard);
    let ys = spans(sh, px[1] - m, px[3] + m, pasteboard);
    let (cx, cy) = ((px[0] + px[2]) / 2.0, (px[1] + px[3]) / 2.0);
    let mut out = Vec::new();
    for &(oy, h) in &ys {
        for &(ox, w) in &xs {
            let inside = ox >= 0 && oy >= 0 && ox < i64::from(sw) && oy < i64::from(sh);
            let (fx, fy) = (ox as f32, oy as f32);
            let view = View { transform: Affine::translate(-fx, -fy).mul(&Affine::scale(z, z)), width: w, height: h, background: if inside { bg } else { None } };
            let rect = [fx / z, fy / z, (fx + w as f32) / z, (fy + h as f32) / z];
            out.push(TileSpec { rect, view });
        }
    }
    let d = |s: &TileSpec| {
        let (mx, my) = ((s.rect[0] + s.rect[2]) / 2.0 * z - cx, (s.rect[1] + s.rect[3]) / 2.0 * z - cy);
        mx * mx + my * my
    };
    out.sort_by(|a, b| d(a).total_cmp(&d(b)));
    out
}

fn to_v(p: Pos2) -> Vec2 {
    Vec2::new(p.x, p.y)
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    // View › Rulers along the top and the left (ADR 0076); the stage view in what is left.
    let (canvas, bars) = crate::rulers::layout(app, ui.max_rect());
    let now = ui.input(|i| i.time);
    crate::camera_tool::on_tool_change(app);
    crate::shape_envelope::settle(app);
    crate::camera_tool::tick(app, ui.ctx(), now);
    app.stage_cache.canvas = canvas;
    let resp = ui.allocate_rect(canvas, Sense::click_and_drag());
    // The drawing surface: Ctrl+F6 lands here, and screen readers hear the tool in hand.
    crate::a11y::anchor(ui, crate::a11y::Region::Stage, canvas, &resp);
    let ppp = ui.ctx().pixels_per_point();
    let size = vec2(app.s.doc.stage.width as f32, app.s.doc.stage.height as f32);
    // Still as fitted (nobody zoomed or panned since) but the canvas changed size: fit again, as
    // Fit in Window stays fitted when the window or the interface scale changes.
    if let Some((was, z)) = app.view.fitted
        && was != vec2(canvas.width(), canvas.height())
        && app.view.zoom == z
        && app.view.pan == egui::Vec2::ZERO
    {
        app.view.fit = true;
    }
    if app.view.fit && canvas.width() > 10.0 && canvas.height() > 10.0 {
        app.view.zoom = ((canvas.width() - 64.0) / size.x).min((canvas.height() - 64.0) / size.y).clamp(0.05, 64.0);
        app.view.pan = egui::Vec2::ZERO;
        app.view.fit = false;
        app.view.fitted = Some((vec2(canvas.width(), canvas.height()), app.view.zoom));
    }

    // ---- camera input -------------------------------------------------------------------
    if resp.hovered() {
        let (zoom_delta, scroll) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta));
        if let Some(p) = resp.hover_pos() {
            if (zoom_delta - 1.0).abs() > 1e-4 {
                let z = app.view.zoom * zoom_delta;
                app.zoom_about(canvas, p, z);
            } else if app.tool == Tool::Camera && scroll.y != 0.0 {
                // With the Camera tool the wheel zooms the camera, not the view.
                crate::camera_tool::wheel(app, scroll.y, now);
            } else if scroll != egui::Vec2::ZERO {
                app.view.pan += scroll;
            }
        }
    }
    if crate::a11y::keyboard(ui.ctx()) != crate::a11y::Keyboard::Text {
        let (zin, zout, fit, one) = ui.input_mut(|i| {
            (
                i.consume_key(Modifiers::COMMAND, Key::Equals) || i.consume_key(Modifiers::COMMAND, Key::Plus),
                i.consume_key(Modifiers::COMMAND, Key::Minus),
                i.consume_key(Modifiers::COMMAND, Key::Num0),
                i.consume_key(Modifiers::COMMAND, Key::Num1),
            )
        });
        let c = canvas.center();
        if zin {
            let z = app.view.zoom * 1.5;
            app.zoom_about(canvas, c, z);
        }
        if zout {
            let z = app.view.zoom / 1.5;
            app.zoom_about(canvas, c, z);
        }
        if fit {
            app.view.fit = true;
        }
        if one {
            app.zoom_about(canvas, c, 1.0);
        }
    }

    let st_zoom = app.view.zoom;
    // Snap the stage origin to whole device pixels: tile edges then land exactly on screen
    // pixels, so texture filtering can't show seams between tiles.
    let raw = canvas.center() + app.view.pan - size * st_zoom * 0.5;
    let st_origin = pos2((raw.x * ppp).round() / ppp, (raw.y * ppp).round() / ppp);
    let st_screen = |p: Vec2| pos2(st_origin.x + p.x * st_zoom, st_origin.y + p.y * st_zoom);
    let stage_rect = Rect::from_min_size(st_origin, size * st_zoom);
    // Rulers, grid and guides are the stage's, in stage pixels (ADR 0076).
    let geo = crate::rulers::Geo { canvas, origin: st_origin, zoom: st_zoom };
    // While a symbol is open (ADR 0045), tools and overlays work in its space, which the stage
    // shows moved and scaled where its instance stands.
    let edit_view = app.s.edit_view();
    let (origin, zoom) = match edit_view {
        Some(v) => (st_origin + vec2(v.origin.x, v.origin.y) * st_zoom, st_zoom * v.scale),
        None => (st_origin, st_zoom),
    };
    let to_screen = |p: Vec2| pos2(origin.x + p.x * zoom, origin.y + p.y * zoom);
    let to_stage = |p: Pos2| pos2((p.x - origin.x) / zoom, (p.y - origin.y) / zoom);
    let painter = ui.painter_at(canvas);
    // View > Preview Through Camera: the stage shows the movie's picture (ADR 0027). The
    // Selection tool picks and moves artwork there, the pointer mapped back through the camera
    // and each layer's depth (ADR 0082); the other artwork tools pause (what they'd edit is in
    // stage space, not where it shows).
    let preview = crate::camera_tool::previewing(app);
    let cam_geo = crate::camera_tool::Geo { origin: st_origin, zoom: st_zoom, stage_rect, preview };
    // Library items dragged onto the stage land at the pointer (ADR 0053).
    crate::library::stage_drop(app, ui, &resp, to_stage, preview);
    // Files dropped from the desktop land here too (ADR 0059).
    app.drop.stage = Some(crate::drop::StageGeo { canvas, origin, zoom, paused: preview });


    // ---- tools ----------------------------------------------------------------------------
    let (shift, alt, space, command) = ui.input(|i| (i.modifiers.shift, i.modifiers.alt, i.key_down(Key::Space), i.modifiers.command));
    let pointer = resp.hover_pos().or_else(|| resp.interact_pointer_pos());
    app.cursor_stage = pointer.filter(|p| canvas.contains(*p)).map(to_stage);
    let panning = app.tool == Tool::Hand || space || resp.dragged_by(egui::PointerButton::Middle);
    let paused = preview && !panning && !matches!(app.tool, Tool::Camera | Tool::Hand | Tool::Zoom | Tool::Select);
    // The Selection tool on the movie's picture: points there are the picture's (ADR 0082).
    let through = preview && !panning && app.tool == Tool::Select;
    // A drag from a ruler makes a guide (ADR 0076).
    crate::rulers::input(app, ui, &bars, geo, preview);
    let guide_hot = if app.drag == Drag::None && !panning && !preview { pointer.and_then(|p| crate::rulers::guide_at(app, geo, p)) } else { None };
    if paused && (resp.drag_started() || resp.clicked()) {
        app.status_ok(tr!("Previewing through the camera: the Selection tool picks and moves artwork here; other tools wait. Turn off View › Preview Through Camera to draw."));
    }
    // Selection box in screen space (free transform handles live on it).
    // Edit Multiple Frames takes the Selection and Free Transform tools' input (ADR 0047); Free
    // Transform's box then holds the selection on every keyframe (ADR 0051).
    let multi = crate::onion::editing(app) && !preview;
    let sel_bounds = if multi { crate::onion::selection_bounds(app) } else { bounds_arr(app.s.selection_bounds()) };
    let sel_rect = sel_bounds.map(|b| Rect::from_min_max(to_screen(Vec2::new(b[0], b[1])), to_screen(Vec2::new(b[2], b[3]))));
    // Shape hint markers take the pointer first with the selection tools (ADR 0023).
    let hint_tools = matches!(app.tool, Tool::Select | Tool::Transform | Tool::Subselect) && !panning && !preview;
    let hint_marks = if hint_tools { crate::shape_hints::markers(app) } else { Vec::new() };
    let hint_at = |p: Pos2| crate::shape_hints::hit(&hint_marks, p, to_screen);
    let hint_hot = pointer.and_then(hint_at);

    let xf = crate::pathtools::Xf { origin, zoom };
    // Pen pressure, when a tablet reports it (the Brush tool's width follows it).
    let pressure = crate::brushtool::frame_pressure(ui.ctx());
    // Tilt, where the toolkit reports it: the brushes' Tilt options show once it has (ADR 0093).
    crate::brushtool::note_tilt(app, crate::brushtool::frame_tilt(ui.ctx()));
    let now = ui.input(|i| i.time);
    // The motion path takes the pointer with the selection tools (ADR 0030): its ends and
    // handles before the object under them, the path itself only where no object is.
    let mp_tools = matches!(app.tool, Tool::Select | Tool::Subselect) && !panning && !preview;
    let mp_grab = |app: &App, p: Pos2| {
        let shown = crate::motion_path::shown(app)?;
        let open = app.s.hit_test(to_v(to_stage(p)), 3.0 / zoom).is_none();
        crate::motion_path::hit(app, &shown, p, xf, open).map(|g| (shown, g))
    };
    let mp_hot = if mp_tools && app.drag == Drag::None { pointer.and_then(|p| mp_grab(app, p)).map(|(_, g)| g) } else { None };
    // Ctrl (⌘) with the path or its object under the pointer: a drag moves the whole path
    // (ADR 0038). The Pen with Alt over the path adds a point there.
    let mp_move = mp_tools && command && app.drag == Drag::None && pointer.is_some_and(|p| crate::motion_path::move_hit(app, p, xf));
    let pen_hot = if app.tool == Tool::Pen && alt && !panning && !preview && app.drag == Drag::None { pointer.and_then(|p| crate::motion_path::pen_hot(app, p, xf)) } else { None };
    // The open symbol's 9-slice guides take the Selection tool's press first (ADR 0055).
    let slice_hot = if app.drag == Drag::None && !panning && !preview { pointer.and_then(|p| crate::scale_grid::hit(app, p, xf)) } else { None };
    if resp.hovered() {
        let icon = match app.tool {
            _ if panning => CursorIcon::Grab,
            _ if let Drag::Guide { index, .. } = app.drag => crate::rulers::cursor(app, index),
            _ if let Some(i) = guide_hot => crate::rulers::cursor(app, i),
            _ if let Drag::SliceGuide { guide } = app.drag => guide.cursor(),
            _ if let Some(guide) = slice_hot => guide.cursor(),
            _ if matches!(app.drag, Drag::Hint { .. } | Drag::CameraPan { .. } | Drag::BonePose { .. } | Drag::WarpPin { .. }) => CursorIcon::Grabbing,
            _ if matches!(app.drag, Drag::ArmatureMove { .. }) => CursorIcon::Move,
            Tool::Select if app.drag == Drag::None && pointer.is_some_and(|p| crate::bone_tool::hit(app, xf, p).is_some()) => CursorIcon::Grab,
            _ if paused => CursorIcon::NotAllowed,
            Tool::Camera if app.drag == Drag::None && pointer.is_some_and(|p| crate::camera_tool::on_knob(app, &cam_geo, p)) => CursorIcon::Alias,
            Tool::Camera => CursorIcon::Grab,
            _ if hint_hot.is_some() && app.drag == Drag::None => CursorIcon::Grab,
            _ if matches!(app.drag, Drag::MotionPath { .. }) => CursorIcon::Grabbing,
            _ if mp_move || matches!(app.drag, Drag::MotionMove { .. }) => CursorIcon::Move,
            _ if mp_hot.is_some() => CursorIcon::Grab,
            _ if pen_hot.is_some() => CursorIcon::Copy,
            Tool::Width => crate::widthtool::cursor(app, pointer, xf),
            Tool::GradientTransform => crate::gradient_tool::cursor(app, pointer, xf),
            Tool::Transform if matches!(app.drag, Drag::Envelope { .. }) => CursorIcon::Grabbing,
            Tool::Transform if app.drag == Drag::None && pointer.is_some_and(|p| crate::shape_envelope::over_handle(app, xf, p)) => CursorIcon::Grab,
            Tool::Transform => {
                let zone = match app.drag {
                    Drag::Transform { kind, .. } | Drag::MultiTransform { kind, .. } => Some(kind),
                    _ => sel_rect.zip(pointer).and_then(|(r, p)| xf_zone(r, p)),
                };
                match zone {
                    Some(XfKind::Scale { hx, hy }) if hx * hy > 0 => CursorIcon::ResizeNwSe,
                    Some(XfKind::Scale { hx, hy }) if hx * hy < 0 => CursorIcon::ResizeNeSw,
                    Some(XfKind::Scale { hx: 0, .. }) => CursorIcon::ResizeVertical,
                    Some(XfKind::Scale { .. }) => CursorIcon::ResizeHorizontal,
                    Some(XfKind::Rotate) => CursorIcon::Alias,
                    Some(XfKind::Move) => CursorIcon::Move,
                    None => CursorIcon::Default,
                }
            }
            Tool::Rect | Tool::Oval | Tool::Star | Tool::Line | Tool::Pen | Tool::Pencil | Tool::Brush | Tool::PaintBrush | Tool::Eraser | Tool::PaintBucket | Tool::InkBottle | Tool::Eyedropper | Tool::Bone | Tool::Bind | Tool::Warp => CursorIcon::Crosshair,
            Tool::Text => CursorIcon::Text,
            Tool::Zoom => {
                if alt {
                    CursorIcon::ZoomOut
                } else {
                    CursorIcon::ZoomIn
                }
            }
            _ => CursorIcon::Default,
        };
        ui.ctx().set_cursor_icon(icon);
    }

    if resp.clicked() || resp.drag_started() || resp.secondary_clicked() {
        // Ctrl+X/C/V go back to artwork.
        app.timeline_focus = false;
    }
    if app.tool == Tool::Pen && !panning && !paused {
        crate::pathtools::pen_input(app, ui, &resp, xf);
    }

    if resp.drag_started() {
        let press = ui.input(|i| i.pointer.press_origin()).unwrap_or(canvas.center());
        let sp = to_stage(press);
        app.drag = if panning {
            Drag::Pan
        } else if paused {
            Drag::None
        } else if through {
            crate::camera_tool::select_press(app, to_v(sp), shift)
        } else if let Some(i) = crate::rulers::guide_at(app, geo, press) {
            // The Selection tool on a guide moves it; onto a ruler, deletes it (ADR 0076).
            crate::rulers::drag_start(app, i)
        } else if let Some(guide) = crate::scale_grid::hit(app, press, xf) {
            crate::scale_grid::drag_start(app, guide)
        } else if let Some(m) = hint_at(press) {
            // Undo labels stay English in the history and are translated where they show.
            app.s.begin_gesture(tr_noop!("Move Shape Hint"));
            Drag::Hint { index: m.index, side: m.side }
        } else if mp_tools && command && crate::motion_path::move_hit(app, press, xf) {
            crate::motion_path::move_start(app, to_v(sp))
        } else if let Some((shown, grab)) = mp_tools.then(|| mp_grab(app, press)).flatten() {
            crate::motion_path::drag_start(app, shown, grab, to_v(sp))
        } else {
            let tool = app.tool;
            match tool {
                // Edit Multiple Frames (ADR 0047): artwork on every keyframe between the markers;
                // Free Transform's handles transform it all about one pivot (ADR 0051).
                // An open envelope's handles (ADR 0080).
                Tool::Transform if !multi && let Some(d) = crate::shape_envelope::drag_start(app, xf, press) => d,
                Tool::Transform if multi && let (Some(kind), Some(bounds)) = (sel_rect.and_then(|r| xf_zone(r, press)), sel_bounds) => {
                    app.s.begin_gesture(tr_noop!("Free Transform"));
                    Drag::MultiTransform { kind, start: sp, bounds, applied: Affine::IDENTITY }
                }
                Tool::Select | Tool::Transform if multi => crate::onion::drag_start(app, sp, shift, zoom),
                // On an armature, a drag poses it (ADR 0043).
                Tool::Select if let Some(d) = crate::bone_tool::select_drag_start(app, xf, press) => d,
                Tool::Select => select_press(app, sp, shift, command, alt, zoom),
                Tool::Transform => {
                    let zone = sel_rect.and_then(|r| xf_zone(r, press));
                    match (zone, sel_bounds) {
                        (Some(kind), Some(bounds)) => {
                            app.s.begin_gesture(tr_noop!("Free Transform"));
                            Drag::Transform { kind, start: sp, bounds, applied: Affine::IDENTITY }
                        }
                        // Not on the box: grab whatever is under the pointer and move it.
                        _ => match app.s.hit_test(to_v(sp), 3.0 / zoom) {
                            Some(_) => {
                                app.run("element.pick", json!({"x": sp.x, "y": sp.y, "add": shift}));
                                match bounds_arr(app.s.selection_bounds()) {
                                    Some(bounds) => {
                                        app.s.begin_gesture(tr_noop!("Free Transform"));
                                        Drag::Transform { kind: XfKind::Move, start: sp, bounds, applied: Affine::IDENTITY }
                                    }
                                    None => Drag::None,
                                }
                            }
                            None => Drag::Marquee { start: sp, cur: sp },
                        },
                    }
                }
                Tool::Subselect => crate::pathtools::subselect_drag_start(app, press, shift, xf),
                Tool::Bone => crate::bone_tool::drag_start(app, xf, press),
                Tool::Bind => crate::bind_tool::drag_start(app, xf, press),
                Tool::Warp => crate::warp_tool::drag_start(app, xf, press),
                Tool::Width => crate::widthtool::drag_start(app, press, xf),
                Tool::GradientTransform => crate::gradient_tool::drag_start(app, press, xf),
                Tool::Pencil => {
                    app.pencil = vec![to_v(sp)];
                    Drag::Pencil
                }
                Tool::Brush | Tool::PaintBrush => crate::brushtool::start(app, to_v(sp), pressure, now),
                // With the Faucet on, the eraser deletes on a click instead.
                Tool::Eraser if !app.eraser.faucet => crate::erasertool::start(app, to_v(sp)),
                Tool::Pen | Tool::Text | Tool::PaintBucket | Tool::InkBottle | Tool::Eyedropper | Tool::Eraser => Drag::None,
                // The press snaps to points, lines and fill edges (ADR 0045).
                Tool::Rect | Tool::Oval | Tool::Star | Tool::Line => {
                    let start = snap_draw(app, sp, zoom);
                    Drag::Shape { start, cur: start }
                }
                Tool::Camera => crate::camera_tool::drag_start(app, &cam_geo, press),
                Tool::Hand => Drag::Pan,
                Tool::Zoom => Drag::None,
            }
        };
    }

    if resp.dragged()
        && let Some(p) = resp.interact_pointer_pos()
        && matches!(app.drag, Drag::CameraPan { .. } | Drag::CameraRotate { .. })
    {
        let mut d = app.drag;
        crate::camera_tool::drag_to(app, &cam_geo, &mut d, p);
        app.drag = d;
    }
    if resp.dragged()
        && let Some(c) = resp.interact_pointer_pos().map(to_stage)
        && matches!(app.drag, Drag::Bone { .. } | Drag::BonePose { .. } | Drag::ArmatureMove { .. })
    {
        let mut d = app.drag;
        crate::bone_tool::drag_to(app, &mut d, c);
        app.drag = d;
    }
    if resp.dragged() {
        let mut cur = resp.interact_pointer_pos().map(to_stage);
        // A shape's other end snaps too, unless Shift constrains it (ADR 0045).
        if let (Drag::Shape { .. }, Some(c)) = (app.drag, cur) {
            cur = Some(if shift {
                app.snap_ring = None;
                c
            } else {
                snap_draw(app, c, zoom)
            });
        }
        match (&mut app.drag, cur) {
            (Drag::Pan, _) => app.view.pan += resp.drag_delta(),
            (Drag::CameraMove { last }, Some(c)) => {
                let mut l = *last;
                crate::camera_tool::move_to(app, &mut l, to_v(c));
                app.drag = Drag::CameraMove { last: l };
            }
            (Drag::Move { last }, Some(c)) => {
                let d = c - *last;
                if d.length_sq() > 0.0 {
                    *last = c;
                    app.run("element.translate", json!({"dx": d.x, "dy": d.y}));
                }
            }
            (Drag::SnapMove { .. }, Some(c)) => snap_move(app, c, zoom, shift),
            (Drag::MultiMove { last }, Some(c)) => {
                let mut l = *last;
                crate::onion::drag_to(app, &mut l, c);
                app.drag = Drag::MultiMove { last: l };
            }
            (Drag::MultiMarquee { cur: m, .. }, Some(c)) => *m = c,
            (Drag::MultiTransform { kind, start, bounds, applied }, Some(c)) => {
                let (kind, start, bounds, applied) = (*kind, *start, *bounds, *applied);
                let total = xf_total(kind, bounds, start, c, shift, alt);
                // Only the change since the last frame; the gesture makes it one undo step.
                if let Some(inv) = applied.inverse() {
                    let delta = total.mul(&inv);
                    if delta.m.iter().zip(Affine::IDENTITY.m).any(|(a, b)| (a - b).abs() > 1e-6) && crate::onion::transform_by(app, &delta) {
                        app.drag = Drag::MultiTransform { kind, start, bounds, applied: total };
                    }
                }
            }
            (Drag::Bend { id, cmd, t, last }, Some(c)) => {
                let d = c - *last;
                if d.length_sq() > 0.0 {
                    let p = json!({"id": id.0, "cmd": *cmd, "t": *t, "x": last.x, "y": last.y, "dx": d.x, "dy": d.y, "merge": false});
                    *last = c;
                    if let Some(v) = app.run("path.bend_segment", p) {
                        touched(app, &v["changed"]);
                    }
                }
            }
            (Drag::RawPoint { .. }, Some(c)) => move_raw_point(app, c, zoom),
            (Drag::RawHandle { id, cmd, slot, last }, Some(c)) => {
                // Subselection (ADR 0042): the curve changes in every raw shape that has it.
                let d = c - *last;
                if d.length_sq() > 0.0 {
                    // A smooth anchor's other handle turns with it; Alt breaks the pair (ADR 0045).
                    let p = json!({"id": id.0, "cmd": *cmd, "slot": *slot, "dx": d.x, "dy": d.y, "smooth": !alt, "merge": false});
                    *last = c;
                    if let Some(v) = app.run("raw.move_handle", p) {
                        touched(app, &v["changed"]);
                    }
                }
            }
            (Drag::Transform { kind, start, bounds, applied }, Some(c)) => {
                // The handle being dragged (or the box, moving) snaps; Shift constrains instead.
                let c = if shift { c } else { xf_snap(&mut app.s, &mut app.snap_ring, *kind, *bounds, *start, c, zoom) };
                let total = xf_total(*kind, *bounds, *start, c, shift, alt);
                // Send only the change since the last frame; the gesture makes it one undo step.
                if let Some(inv) = applied.inverse() {
                    let delta = total.mul(&inv);
                    if delta.m.iter().zip(Affine::IDENTITY.m).any(|(a, b)| (a - b).abs() > 1e-6) {
                        let [a, b, cc, d, e, f] = delta.m;
                        let ok = app.s.execute("modify.transform", &json!({"a": a, "b": b, "c": cc, "d": d, "e": e, "f": f})).is_ok();
                        if ok {
                            *applied = total;
                        }
                    }
                }
            }
            (Drag::MotionPath { grab, start }, Some(c)) => {
                let (grab, start) = (*grab, *start);
                crate::motion_path::drag_to(app, grab, start, c, alt);
            }
            (Drag::MotionMove { last }, Some(c)) => {
                let d = c - *last;
                *last = c;
                crate::motion_path::move_to(app, Vec2::new(d.x, d.y));
            }
            (Drag::SliceGuide { guide }, Some(c)) => {
                let guide = *guide;
                crate::scale_grid::drag_to(app, guide, c);
            }
            (Drag::Guide { index, .. }, _) => {
                let index = *index;
                if let Some(p) = resp.interact_pointer_pos() {
                    crate::rulers::drag_to(app, index, geo, p);
                }
            }
            (Drag::Hint { index, side }, Some(c)) => {
                // Live: the hint snaps to the nearest edge as the pointer moves.
                let (index, side) = (*index, *side);
                app.run("tween.move_hint", json!({"hint": index, "side": side.key(), "x": c.x, "y": c.y}));
            }
            (Drag::Points { last }, Some(c)) => {
                let d = c - *last;
                if d.length_sq() > 0.0 {
                    *last = c;
                    crate::pathtools::move_points(app, Vec2::new(d.x, d.y), !alt);
                }
            }
            (Drag::PointMarquee { cur: m, .. } | Drag::BindMarquee { cur: m, .. }, Some(c)) => *m = c,
            (Drag::BindPaint { last }, Some(c)) => {
                let from = *last;
                let to = crate::bind_tool::paint_to(app, from, c);
                if let Drag::BindPaint { last } = &mut app.drag {
                    *last = to;
                }
            }
            (Drag::WarpPin { element, pin }, Some(c)) => {
                let (element, pin) = (*element, *pin);
                crate::warp_tool::drag_to(app, element, pin, c);
            }
            (d @ Drag::Width { .. }, Some(c)) => {
                let d = *d;
                crate::widthtool::drag_to(app, d, to_v(c), alt, zoom);
            }
            (d @ Drag::Gradient { .. }, Some(c)) => {
                let d = *d;
                crate::gradient_tool::drag_to(app, d, to_v(c), shift);
            }
            (d @ Drag::Envelope { .. }, Some(c)) => {
                let d = *d;
                crate::shape_envelope::drag_to(app, d, to_v(c));
            }
            (Drag::Pencil, Some(c)) => crate::pathtools::pencil_sample(app, to_v(c), xf),
            (Drag::Brush, Some(c)) => crate::brushtool::sample(app, to_v(c), pressure, xf, now),
            (Drag::Eraser, Some(c)) => crate::brushtool::sample(app, to_v(c), None, xf, now),
            (Drag::Marquee { cur: m, .. }, Some(c)) => *m = c,
            (Drag::Shape { start, cur: m }, Some(c)) => *m = constrain(app.tool, *start, c, shift),
            _ => {}
        }
    }

    if resp.drag_stopped() {
        match app.drag {
            Drag::Move { .. } | Drag::SnapMove { .. } | Drag::CameraMove { .. } => {
                app.drop_raw();
                app.s.end_gesture();
            }
            Drag::Bend { .. } | Drag::RawPoint { .. } | Drag::RawHandle { .. } => end_raw_drag(app),
            Drag::Transform { .. } | Drag::SliceGuide { .. } | Drag::Points { .. } | Drag::Hint { .. } | Drag::MotionMove { .. } | Drag::CameraPan { .. } | Drag::CameraRotate { .. } | Drag::BonePose { .. } | Drag::ArmatureMove { .. } | Drag::WarpPin { .. } => app.s.end_gesture(),
            Drag::Bone { from, head, cur } => crate::bone_tool::drag_end(app, xf, from, head, cur),
            Drag::Guide { index, .. } => crate::rulers::drag_end(app, index, geo, ui.input(|i| i.pointer.latest_pos())),
            Drag::Width { .. } => crate::widthtool::drag_end(app),
            Drag::Gradient { .. } => crate::gradient_tool::drag_end(app),
            Drag::Envelope { .. } => crate::shape_envelope::drag_end(app),
            Drag::MotionPath { .. } => crate::motion_path::drag_end(app),
            Drag::PointMarquee { start, cur } => crate::pathtools::marquee_points(app, start, cur, shift),
            Drag::BindMarquee { start, cur } => crate::bind_tool::drag_end(app, start, cur, shift, command),
            Drag::BindPaint { .. } => crate::bind_tool::paint_end(app),
            Drag::Pencil => crate::pathtools::commit_pencil(app, xf),
            Drag::Brush => crate::brushtool::commit(app, xf),
            Drag::Eraser => crate::erasertool::commit(app, xf),
            // Raw shapes merge where they landed, in the drag's step (ADR 0051).
            Drag::MultiMove { .. } | Drag::MultiTransform { .. } => crate::onion::drag_end(app),
            Drag::MultiMarquee { start, cur } => crate::onion::marquee(app, start, cur, shift),
            Drag::Marquee { start, cur } => {
                // The parts of raw fills and strokes under it, cut at its edges, and the objects
                // wholly inside it (ADR 0041).
                app.run("raw.select_rect", json!({"x0": start.x, "y0": start.y, "x1": cur.x, "y1": cur.y, "add": shift}));
            }
            Drag::Shape { start, cur } => create_shape(app, start, cur),
            Drag::Pan | Drag::None | Drag::CameraWheel { .. } => {}
        }
        app.drag = Drag::None;
        app.snap_ring = None;
    }

    // Attach to Path (ADR 0070) waits for a click on a shape; Escape gives up.
    if app.path_pick.is_some() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.path_pick = None;
        app.status_ok(tr!("Attach to Path cancelled."));
    }
    if resp.clicked()
        && !paused
        && let Some(p) = resp.interact_pointer_pos() {
            let sp = to_stage(p);
            match app.tool {
                // Previewing: pick what the movie shows there (ADR 0082).
                Tool::Select if through => {
                    app.bone_sel = None;
                    app.run("element.pick", json!({"x": sp.x, "y": sp.y, "add": shift, "view": "camera"}));
                }
                _ if !panning && let Some(text) = app.path_pick.take() => {
                    app.s.selection = anicraft_engine::Selection::only(app.s.layer, [text]);
                    if app.run("text.attach_path", json!({"x": sp.x, "y": sp.y})).is_some() {
                        app.status_ok(tr!("The text follows the shape's outline. {undo} to undo.", undo = app.undo_key()));
                    }
                }
                Tool::Subselect if !panning => {
                    if !crate::motion_path::click(app, p, xf, shift) {
                        crate::pathtools::subselect_click(app, p, shift, xf);
                    }
                }
                Tool::Width if !panning => crate::widthtool::click(app, p, xf),
                Tool::GradientTransform if !panning => crate::gradient_tool::pick(app, p, xf),
                Tool::PaintBucket if !panning => {
                    // The gap size is set in screen points, so it feels the same at any zoom.
                    let gap = app.bucket_gap.screen_px() / zoom.max(1e-3);
                    if let Some(v) = app.run("paint.bucket", json!({"x": sp.x, "y": sp.y, "gap": gap})) {
                        app.status_ok(if v.get("recolored").is_some() { tr!("Recoloured the fill. {undo} to undo.", undo = app.undo_key()) } else { tr!("Filled the area. {undo} to undo.", undo = app.undo_key()) });
                    }
                }
                Tool::Brush if !panning => {
                    // A click paints a dab.
                    crate::brushtool::start(app, to_v(sp), pressure, now);
                    crate::brushtool::commit(app, xf);
                }
                Tool::Eraser if !panning => {
                    if app.eraser.faucet {
                        crate::erasertool::faucet(app, to_v(sp), zoom);
                    } else {
                        // A click erases a dab.
                        crate::erasertool::start(app, to_v(sp));
                        crate::erasertool::commit(app, xf);
                    }
                }
                Tool::Eyedropper if !panning => crate::erasertool::eyedropper(app, to_v(sp), shift, zoom),
                Tool::InkBottle if !panning => {
                    app.run("paint.ink", json!({"x": sp.x, "y": sp.y, "slop": 4.0 / zoom.max(1e-3)}));
                }
                Tool::Text if !panning => {
                    commit_text(app);
                    if !open_text_at(app, sp, 3.0 / zoom.max(1e-3)) {
                        app.text_edit = Some(crate::app::TextEditing { at: sp, element: None, text: String::new(), original: String::new(), focus: true });
                    }
                }
                Tool::Bone if !panning => crate::bone_tool::click(app, xf, p),
                Tool::Bind if !panning => crate::bind_tool::click(app, xf, p, shift, command),
                Tool::Warp if !panning => crate::warp_tool::click(app, xf, p),
                Tool::Select | Tool::Transform if !panning && multi => crate::onion::click(app, sp, shift, zoom),
                // A click on a bone picks it (ADR 0043).
                Tool::Select if !panning && let Some((layer, bone, _)) = crate::bone_tool::hit(app, xf, p) => crate::bone_tool::select(app, layer, Some(bone)),
                // A click on a hint marker leaves the selection alone.
                Tool::Select | Tool::Transform if !panning && hint_at(p).is_none() => {
                    app.bone_sel = None;
                    app.run("element.pick", json!({"x": sp.x, "y": sp.y, "add": shift}));
                }
                Tool::Zoom => {
                    let z = if alt { app.view.zoom / 2.0 } else { app.view.zoom * 2.0 };
                    app.zoom_about(canvas, p, z);
                }
                _ => {}
            }
        }

    // Right-click: select what is under the cursor (keeping an existing multi-selection), then
    // offer the actions that apply to it.
    if resp.secondary_clicked() {
        app.hint_menu = resp.interact_pointer_pos().and_then(hint_at).map(|m| m.index);
    }
    if resp.secondary_clicked()
        && app.hint_menu.is_none()
        && !preview
        && let Some(p) = resp.interact_pointer_pos()
    {
        let sp = to_stage(p);
        let under = app.s.hit_test(to_v(sp), 3.0 / zoom);
        // On the motion path, the menu is for its points: the selection stays.
        let on_path = !panning && crate::motion_path::secondary_click(app, p, xf);
        if !on_path && under.is_none_or(|(l, e)| !app.s.selection.contains(l, e)) {
            app.run("element.pick", json!({"x": sp.x, "y": sp.y, "add": false}));
        }
    }
    // Double-clicking text with the Selection tool edits it, as in Animate.
    if app.tool == Tool::Select
        && !preview
        && resp.double_clicked()
        && let Some(p) = resp.interact_pointer_pos()
    {
        commit_text(app);
        let sp = to_stage(p);
        let slop = 3.0 / zoom.max(1e-3);
        if !open_text_at(app, sp, slop) {
            match app.s.hit_test(to_v(sp), slop) {
                // A symbol instance (ADR 0045), a group (ADR 0073) or a drawing object (ADR 0080)
                // opens for editing in place.
                Some((layer, id)) if app.s.elements_on(layer).iter().any(|e| e.id == id && anicraft_engine::editing::opens(e)) => {
                    app.run("symbol.edit", json!({"id": id.0}));
                }
                // Raw artwork (ADR 0031): a stroke and the strokes joined to it, or a fill and
                // its outline.
                Some(_) => {
                    app.run("element.select_connected", json!({"x": sp.x, "y": sp.y, "add": shift, "slop": slop}));
                }
                // Empty space inside a symbol or group goes back up a level, as in Animate.
                None if !app.s.editing.is_empty() => {
                    app.run("symbol.exit", json!({}));
                }
                None => {}
            }
        }
    }
    if app.tool == Tool::Subselect
        && !preview
        && resp.double_clicked()
        && let Some(p) = resp.interact_pointer_pos()
        // On the motion path first, else on a selected shape's outline.
        && !crate::motion_path::double_click(app, p, xf)
    {
        crate::pathtools::subselect_double_click(app, p, xf);
    }

    resp.context_menu(|ui| {
        ui.set_min_width(220.0);
        if crate::shape_hints::menu(app, ui) || crate::motion_path::menu(app, ui) || crate::pathtools::point_menu(app, ui) {
            return;
        }
        if multi {
            crate::onion::context_menu(app, ui);
            return;
        }
        if app.s.selection.is_empty() {
            for id in ["edit.paste", "edit.paste_in_place", "edit.select_all", "timeline.insert_keyframe", "timeline.insert_blank_keyframe"] {
                crate::menus::cmd_item(app, ui, id);
            }
            ui.separator();
            for id in ["edit.undo", "edit.redo"] {
                crate::menus::cmd_item(app, ui, id);
            }
        } else {
            for id in ["edit.cut", "edit.copy", "edit.paste", "edit.duplicate", "edit.delete"] {
                crate::menus::cmd_item(app, ui, id);
            }
            ui.separator();
            ui.menu_button(tr!("Transform"), |ui| {
                for id in ["modify.rotate_cw", "modify.rotate_ccw", "modify.flip_h", "modify.flip_v"] {
                    crate::menus::cmd_item(app, ui, id);
                }
            });
            ui.menu_button(tr!("Align"), |ui| {
                for id in ["align.left", "align.hcenter", "align.right", "align.top", "align.vcenter", "align.bottom", "align.distribute_h", "align.distribute_v"] {
                    crate::menus::cmd_item(app, ui, id);
                }
            });
            ui.separator();
            for id in ["modify.convert_to_symbol", "modify.break_apart", "modify.group", "modify.ungroup"] {
                crate::menus::cmd_item(app, ui, id);
            }
            // Modify › Combine Objects (ADR 0073), as in Animate's menu.
            ui.menu_button(tr!("Combine Objects"), |ui| {
                for id in ["modify.union", "modify.intersect", "modify.punch", "modify.crop"] {
                    crate::menus::cmd_item(app, ui, id);
                }
            });
            ui.menu_button(tr!("Arrange"), |ui| {
                for id in ["arrange.bring_to_front", "arrange.send_to_back"] {
                    crate::menus::cmd_item(app, ui, id);
                }
            });
            ui.separator();
            for id in ["timeline.insert_keyframe", "timeline.create_tween", "timeline.create_shape_tween", "edit.deselect"] {
                crate::menus::cmd_item(app, ui, id);
            }
        }
    });

    // ---- render the frame -----------------------------------------------------------------
    painter.rect_filled(stage_rect.translate(vec2(3.0, 4.0)), 0.0, T.stage_shadow);
    // Stage colour first, so a picture still rendering never shows as a hole.
    painter.rect_filled(stage_rect, 0.0, crate::theme::to_color32(app.s.doc.stage.background));
    if !preview && crate::rulers::grid_under(app) {
        crate::rulers::paint_grid(app, &painter, geo, stage_rect, ppp);
    }
    // What gets rendered: the stage, and the pasteboard around it unless View › Clip Content
    // Outside the Stage is on (#251). Previewing through the camera shows what the movie shows.
    let pasteboard = !app.view.clip_to_stage && !preview;
    let vis = if pasteboard { canvas } else { stage_rect.intersect(canvas) };
    if vis.width() >= 1.0 && vis.height() >= 1.0 {
        if app.stage_cache.service.is_none() {
            let ctx = ui.ctx().clone();
            app.stage_cache.service = Some(RenderService::new(DEFAULT_CACHE_BYTES, Some(Box::new(move || ctx.request_repaint()))));
        }
        if let Some(svc) = &app.stage_cache.service {
            // The stage's size in pixels: a change brings a GPU retry forward (ADR 0092).
            let size = ((vis.width() * ppp).round() as u32, (vis.height() * ppp).round() as u32);
            if let Some(n) = app.stage_cache.gpu.sync(app.prefs.stage_renderer, svc, ui.ctx(), size) {
                let text = crate::gpu_stage::notice_text(&n);
                match n {
                    crate::gpu_stage::Notice::Back => app.status_ok(text),
                    crate::gpu_stage::Notice::Stopped { .. } => app.status_err(text),
                }
            }
        }
        // Jobs are keyed by who draws them, so a renderer switch re-renders (old tiles stand in).
        let raster = app.stage_cache.service.as_ref().map_or(0, RenderService::raster_id);
        // Tiles on a device-pixel grid at this zoom (ADR 0007), of what the stage shows: the
        // document, or an open symbol in place (ADR 0045).
        let z = st_zoom * ppp;
        // The grid under the artwork (ADR 0076): the stage colour is painted under it, and the
        // tiles come without it.
        let grid_under = crate::rulers::grid_under(app) && !preview;
        let bg = (!grid_under).then_some(app.s.doc.stage.background);
        let stage_px = ((size.x * z).ceil().max(1.0) as u32, (size.y * z).ceil().max(1.0) as u32);
        let px = [(vis.min.x - st_origin.x) * ppp, (vis.min.y - st_origin.y) * ppp, (vis.max.x - st_origin.x) * ppp, (vis.max.y - st_origin.y) * ppp];
        let visible = tiles_for(z, stage_px, px, 0, bg, pasteboard);
        let pic = app.s.stage_picture();
        let editing = edit_view.is_some();
        // Onion skin is off while a symbol is open in place (`stage_skin`, ADR 0045).
        let onion = crate::onion::stage_skin(app).copied();
        let onion = onion.as_ref();
        let state = pic.state;
        // Render jobs take movie frames: the current scene starts at `start`.
        let start = app.s.doc.scene_start(app.s.scene);
        let frame = app.s.frame;
        // Inside a symbol open in place, onion skin composes the symbol's own frames (ADR 0051).
        let jobs: Vec<(TileSpec, FrameJob)> = visible.into_iter().map(|t| { let j = make_job_for(raster, state, &pic.doc, pic.frame, t.view, onion).through_camera(preview).inside(pic.front.clone()); (t, j) }).collect();
        let keys: Vec<u64> = jobs.iter().map(|(_, j)| j.key).collect();
        let missing: Vec<usize> = (0..jobs.len()).filter(|&i| !app.stage_cache.has(keys[i])).collect();

        if !missing.is_empty()
            && let Some(svc) = &app.stage_cache.service
        {
            // Latency vs throughput (ADR 0006): an edit (the document changed under the picture
            // on screen) renders the missing visible tiles right here, in one parallel batch,
            // when they were cheap last time. Pans, zooms, playback and heavy scenes go to the
            // background worker as one batch; older tiles cover the gap meanwhile.
            // The very first picture has nothing older to stand in for it, so it counts too.
            let edited = app.stage_cache.shown_state != Some(state);
            let immediate = !app.playing
                && edited
                && match app.prefs.render_mode {
                    RenderMode::Immediate => true,
                    RenderMode::Background => false,
                    RenderMode::Auto => app.stage_cache.ms <= IMMEDIATE_BUDGET_MS,
                };
            let batch: Vec<FrameJob> = missing.iter().map(|&i| jobs[i].1.clone()).collect();
            let started = web_time::Instant::now();
            let results: Vec<Option<std::sync::Arc<FrameImage>>> = if immediate { svc.render_now_batch(batch).into_iter().map(Some).collect() } else { svc.request_batch(batch) };
            let wall = started.elapsed().as_secs_f32() * 1000.0;
            let mut ms = 0.0f32;
            let mut error = None;
            for (&i, img) in missing.iter().zip(results) {
                if let Some(img) = img {
                    ms += img.ms;
                    if img.error.is_none() {
                        app.stage_cache.drawn_by = Some((img.gpu, img.fallback.clone()));
                    }
                    if let Some(e) = app.stage_cache.upload(ui.ctx(), &img, jobs[i].0.rect, &keys) {
                        error = Some(e);
                    }
                }
            }
            if ms > 0.0 {
                // Wall-clock cost of showing the whole view (tiles render in parallel, so the
                // sum of per-tile times would overstate it). Measured directly when rendered
                // here; estimated from per-tile times and core count when they came from the
                // worker.
                let cores = std::thread::available_parallelism().map_or(1, |n| n.get()) as f32;
                let tiles = missing.len().max(1) as f32;
                let batch_ms = if immediate { wall } else { ms / cores.min(tiles) };
                app.stage_cache.ms = batch_ms * keys.len() as f32 / tiles;
            }
            if let Some(e) = error {
                app.status_err(crate::i18n::t_dyn(&e).into_owned());
            }
        }
        let ready = keys.iter().filter(|k| app.stage_cache.tiles.iter().any(|t| t.key == **k)).count();
        app.stage_cache.ready = (ready, keys.len());
        if ready == keys.len() {
            app.stage_cache.shown_state = Some(state);
        }

        // Look-ahead: the ring of tiles around the view (a pan reveals them instantly) and,
        // during playback, the next frames' visible tiles.
        if let Some(svc) = &app.stage_cache.service {
            let mut ahead: Vec<FrameJob> = Vec::new();
            if app.playing && !editing {
                let n = app.s.frame_count().max(1);
                for d in 1..=LOOKAHEAD.min(n - 1) {
                    let f = (frame + d) % n;
                    ahead.extend(jobs.iter().map(|(t, _)| make_job_for(raster, state, &app.s.doc, start + f, t.view, None).through_camera(preview)));
                }
            } else {
                ahead.extend(tiles_for(z, stage_px, px, 1, bg, pasteboard).into_iter().filter(|t| !jobs.iter().any(|(v, _)| v.rect == t.rect)).map(|t| make_job_for(raster, state, &pic.doc, pic.frame, t.view, onion).through_camera(preview)));
            }
            svc.prefetch(ahead);
        }

        // Draw: tiles from earlier zooms/states that cover the view go first (scaled, as a
        // placeholder), then the current tiles on top.
        let view_rect = [(vis.min.x - st_origin.x) / st_zoom, (vis.min.y - st_origin.y) / st_zoom, (vis.max.x - st_origin.x) / st_zoom, (vis.max.y - st_origin.y) / st_zoom];
        let overlaps = |r: &[f32; 4]| r[0] < view_rect[2] && r[2] > view_rect[0] && r[1] < view_rect[3] && r[3] > view_rect[1];
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        let mut stale: Vec<&TileTex> = app.stage_cache.tiles.iter().filter(|t| !keys.contains(&t.key) && overlaps(&t.rect)).collect();
        if ready < keys.len() {
            stale.sort_by_key(|t| t.used);
            for t in stale.iter().rev().take(96).rev() {
                let r = Rect::from_min_max(st_screen(Vec2::new(t.rect[0], t.rect[1])), st_screen(Vec2::new(t.rect[2], t.rect[3])));
                painter.image(t.tex.id(), r, uv, Color32::WHITE);
            }
        }
        for t in app.stage_cache.tiles.iter().filter(|t| keys.contains(&t.key)) {
            let r = Rect::from_min_max(st_screen(Vec2::new(t.rect[0], t.rect[1])), st_screen(Vec2::new(t.rect[2], t.rect[3])));
            painter.image(t.tex.id(), r, uv, Color32::WHITE);
        }

        // While a background render catches up with a drag, outline where the selection is now,
        // so the artwork never seems to lag the pointer.
        if ready < keys.len() && !preview && matches!(app.drag, Drag::Move { .. } | Drag::SnapMove { .. } | Drag::Transform { .. } | Drag::Bend { .. } | Drag::RawPoint { .. } | Drag::RawHandle { .. }) {
            let to_px = Affine::translate(origin.x, origin.y).mul(&Affine::scale(zoom, zoom));
            let dl = app.s.display_list();
            let stroke = Stroke::new(1.5, T.selection);
            for item in dl.items.iter().filter(|i| app.s.selection.contains(i.layer, i.element)).take(2_000) {
                for pl in item.path.flatten(&to_px.mul(&item.transform), 0.5) {
                    let pts: Vec<Pos2> = pl.points.iter().map(|p| pos2(p.x, p.y)).collect();
                    painter.add(if pl.closed { egui::Shape::closed_line(pts, stroke) } else { egui::Shape::line(pts, stroke) });
                }
            }
        }
    }
    painter.rect_stroke(stage_rect, 0.0, Stroke::new(1.0, T.stage_edge), StrokeKind::Outside);
    // The grid over the artwork, and the guides (ADR 0076): the stage's, not the camera's.
    if !preview {
        if !crate::rulers::grid_under(app) {
            crate::rulers::paint_grid(app, &painter, geo, stage_rect, ppp);
        }
        crate::rulers::paint_guides(app, &painter, geo, ppp);
    }

    // ---- overlays -------------------------------------------------------------------------
    // The camera frame (or, previewing, the Camera tool's handle) comes first, under the
    // selection.
    crate::camera_tool::draw(app, &painter, &cam_geo, pointer);
    if preview {
        // The movie's picture: stage-space overlays don't belong on it.
        painter.text(stage_rect.left_top() + vec2(0.0, -6.0), egui::Align2::LEFT_BOTTOM, tr!("Camera view"), crate::theme::font(crate::theme::Type::Caption), T.text_dim);
        let badge = format!("{:.0}%", st_zoom * 100.0);
        painter.text(canvas.right_bottom() + vec2(-10.0, -8.0), egui::Align2::RIGHT_BOTTOM, badge, crate::theme::font(crate::theme::Type::Data), T.text_dim);
        crate::a11y::surface(ui, &resp, &tr!("Stage · previewing through the camera · {tool} tool · frame {frame}", tool = crate::i18n::t(app.tool.label()), frame = app.s.frame + 1));
        return;
    }
    // Mask artwork is never painted; while its layer is unlocked (editable), its edge shows in
    // the layer colour so it can be seen, picked and reshaped. Locking the mask hides the edge.
    // Guide artwork never renders either: it always shows here, as a line in the guide's layer
    // colour, so the path a tween follows is visible while animating.
    {
        let to_px = Affine::translate(origin.x, origin.y).mul(&Affine::scale(zoom, zoom));
        let dl = app.s.display_list();
        for item in dl.items.iter().filter(|i| i.masks.is_some() || i.guide).take(2_000) {
            let Some(layer) = app.s.timeline().layer(item.layer).filter(|l| item.guide || !l.locked) else { continue };
            let stroke = Stroke::new(if item.guide { 2.0 } else { 1.5 }, to_color32(layer.color));
            for pl in item.path.flatten(&to_px.mul(&item.transform), 0.5) {
                let pts: Vec<Pos2> = pl.points.iter().map(|p| pos2(p.x, p.y)).collect();
                painter.add(if pl.closed { egui::Shape::closed_line(pts, stroke) } else { egui::Shape::line(pts, stroke) });
            }
        }
    }
    // Armatures' bones over their artwork (ADR 0043).
    crate::bone_tool::draw(app, &painter, xf);
    crate::bind_tool::draw(app, &painter, xf);
    crate::warp_tool::draw(app, &painter, xf);
    let sel_stroke = Stroke::new(1.0, T.selection);
    let raw = raw_ids(app);
    if app.tool == Tool::Select && app.drag == Drag::None
        && !multi
        && !preview
        && app.palette.is_none()
        && !app.modal_open()
        && let Some(sp) = app.cursor_stage
            && let Some((layer, el)) = app.s.hit_test(to_v(sp), 3.0 / zoom)
                && !app.s.selection.contains(layer, el) && !raw.contains(&(layer, el)) {
                    let b = app.s.element_bounds_on(layer, el);
                    if !b.is_empty() {
                        let r = Rect::from_min_max(to_screen(b.min), to_screen(b.max));
                        painter.rect_stroke(r, 0.0, Stroke::new(1.0, T.selection.gamma_multiply(0.55)), StrokeKind::Middle);
                    }
                }
    let transforming = app.tool == Tool::Transform;
    // An open envelope (ADR 0080) shows its mesh instead of boxes.
    let enveloped = crate::shape_envelope::shown(app).is_some();
    // Selected raw shapes (ADR 0031) show Animate's dot pattern over themselves, with no box.
    if app.tool == Tool::Select && !preview {
        let to_px = Affine::translate(origin.x, origin.y).mul(&Affine::scale(zoom, zoom));
        let (mut spans, mut dots) = (Vec::new(), egui::Mesh::default());
        let mut budget = MAX_DOTS;
        for item in app.s.display_list().items.iter().filter(|i| app.s.selection.contains(i.layer, i.element) && raw.contains(&(i.layer, i.element))) {
            selection_pattern(&mut spans, &mut dots, item, &to_px.mul(&item.transform), canvas, &mut budget);
        }
        if !spans.is_empty() {
            let mut mesh = egui::Mesh::with_texture(pattern_texture(&mut app.stage_cache.pattern, ui.ctx()));
            pattern_mesh(&mut mesh, &spans);
            painter.add(egui::Shape::mesh(mesh));
        }
        painter.add(egui::Shape::mesh(dots));
    }
    if multi {
        let hover = pointer.filter(|p| canvas.contains(*p) && !panning && !app.modal_open() && app.palette.is_none() && matches!(app.tool, Tool::Select | Tool::Transform)).map(to_stage);
        crate::onion::draw(app, &painter, to_screen, hover, zoom);
        crate::onion::stage_badge(app, &painter, canvas);
    }
    // Every selected element, on whichever layer it is (ADR 0044).
    for (layer, e) in app.s.selected_elements() {
        let id = e.id;
        let b = app.s.element_bounds_on(layer, id);
        if b.is_empty() {
            continue;
        }
        // Previewing: the box where the movie shows it, through the camera and the layer's
        // depth (ADR 0082).
        if preview {
            let v = app.s.camera_view(layer).unwrap_or(Affine::IDENTITY);
            let q = [b.min, Vec2::new(b.max.x, b.min.y), b.max, Vec2::new(b.min.x, b.max.y)].map(|p| to_screen(v.apply(p)));
            painter.add(egui::Shape::closed_line(q.to_vec(), sel_stroke));
            continue;
        }
        let r = Rect::from_min_max(to_screen(b.min), to_screen(b.max)).expand(1.0);
        if matches!(app.tool, Tool::Subselect | Tool::Width | Tool::GradientTransform) {
            // Subselection and Width show outlines and points instead (below); Gradient
            // Transform shows the fill's handles (ADR 0074).
            continue;
        }
        if app.tool == Tool::Select && raw.contains(&(layer, id)) {
            continue; // dots above
        }
        if transforming && enveloped {
            continue;
        }
        if transforming {
            // Free transform shows one box for the whole selection (below); members get a hint.
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, T.selection.gamma_multiply(0.45)), StrokeKind::Middle);
            continue;
        }
        // A plain box, as Animate's Selection tool shows: handles are Free Transform's (Q),
        // where they work. Handles drawn here could not be dragged (#205).
        painter.rect_stroke(r, 0.0, sel_stroke, StrokeKind::Middle);
        // Text on a path shows the path it follows (ADR 0070).
        if let anicraft_engine::doc::ElementKind::Text(t) = &e.kind
            && let Some(on) = &t.on_path
            && let Some(item) = app.s.display_list().items.iter().find(|i| i.element == id)
        {
            let to_px = Affine::translate(origin.x, origin.y).mul(&Affine::scale(zoom, zoom)).mul(&item.transform);
            for pl in on.path.flatten(&to_px, 0.5) {
                let pts: Vec<Pos2> = pl.points.iter().map(|p| pos2(p.x, p.y)).collect();
                painter.add(if pl.closed { egui::Shape::closed_line(pts, sel_stroke) } else { egui::Shape::line(pts, sel_stroke) });
            }
        }
        // registration point
        let o = to_screen(e.transform.translation());
        painter.circle(o, 3.5, Color32::WHITE, Stroke::new(1.0, Color32::BLACK));
    }
    // Free transform box: recomputed every frame so it follows the artwork while you drag.
    let xf_box = if multi { crate::onion::selection_bounds(app) } else { bounds_arr(app.s.selection_bounds()) };
    if transforming && !enveloped && let Some(b) = xf_box {
        let r = Rect::from_min_max(to_screen(Vec2::new(b[0], b[1])), to_screen(Vec2::new(b[2], b[3]))).expand(1.0);
        painter.rect_stroke(r, 0.0, Stroke::new(1.5, T.selection), StrokeKind::Middle);
        for (h, _, _) in handles(r) {
            let hr = Rect::from_center_size(h, vec2(HANDLE, HANDLE));
            painter.rect_filled(hr, 0.0, Color32::WHITE);
            painter.rect_stroke(hr, 0.0, Stroke::new(1.0, T.selection), StrokeKind::Middle);
        }
        // transformation point (centre)
        let c = r.center();
        painter.circle(c, 4.0, Color32::WHITE, Stroke::new(1.0, T.selection));
        painter.line_segment([c - vec2(6.0, 0.0), c + vec2(6.0, 0.0)], Stroke::new(1.0, T.selection));
        painter.line_segment([c - vec2(0.0, 6.0), c + vec2(0.0, 6.0)], Stroke::new(1.0, T.selection));
    }
    crate::pathtools::draw_overlay(app, &painter, xf, pointer.filter(|p| canvas.contains(*p)));
    crate::widthtool::draw_overlay(app, &painter, xf, pointer.filter(|p| canvas.contains(*p)));
    crate::gradient_tool::draw_overlay(app, &painter, xf, pointer.filter(|p| canvas.contains(*p) && !panning));
    crate::shape_envelope::draw_overlay(app, &painter, xf, pointer.filter(|p| canvas.contains(*p) && !panning));
    crate::brushtool::draw_overlay(app, &painter, xf, pointer.filter(|p| canvas.contains(*p) && !panning && !paused));
    crate::erasertool::draw_overlay(app, &painter, xf, pointer.filter(|p| canvas.contains(*p) && !panning && !paused));
    // After this frame's edits, so a drag on the path shows where it is now.
    if !preview
        && let Some(shown) = crate::motion_path::shown(app)
    {
        let hot = match app.drag {
            Drag::MotionPath { grab, .. } => Some(grab),
            _ => mp_hot.or(pen_hot),
        };
        crate::motion_path::draw(app, &painter, &shown, xf, hot);
    }
    match app.drag {
        Drag::Marquee { start, cur } => {
            let r = Rect::from_two_pos(to_screen(Vec2::new(start.x, start.y)), to_screen(Vec2::new(cur.x, cur.y)));
            painter.rect_filled(r, 0.0, T.selection.gamma_multiply(0.12));
            painter.rect_stroke(r, 0.0, sel_stroke, StrokeKind::Middle);
        }
        Drag::PointMarquee { start, cur } => {
            let r = Rect::from_two_pos(to_screen(Vec2::new(start.x, start.y)), to_screen(Vec2::new(cur.x, cur.y)));
            painter.rect_filled(r, 0.0, T.selection.gamma_multiply(0.12));
            painter.rect_stroke(r, 0.0, sel_stroke, StrokeKind::Middle);
        }
        Drag::Shape { start, cur } => draw_preview(app, &painter, to_screen(Vec2::new(start.x, start.y)), to_screen(Vec2::new(cur.x, cur.y)), zoom),
        _ => {}
    }
    // The drawing tools show where a press would start, snapped (ADR 0045).
    let draws = matches!(app.tool, Tool::Line | Tool::Rect | Tool::Oval | Tool::Star | Tool::Pen) && !panning && !paused && app.pen.as_ref().is_none_or(|p| !p.dragging);
    if draws && app.drag == Drag::None {
        match pointer.filter(|p| canvas.contains(*p) && resp.hovered()) {
            Some(p) => {
                snap_draw(app, to_stage(p), zoom);
            }
            None => app.snap_ring = None,
        }
    }
    // The snap ring where a dragged point, selection or drawn point snapped (ADR 0041, 0045).
    if let Some(at) = app.snap_ring.filter(|_| matches!(app.drag, Drag::SnapMove { .. } | Drag::RawPoint { .. } | Drag::Shape { .. } | Drag::Transform { .. }) || (draws && app.drag == Drag::None)) {
        let c = to_screen(at);
        painter.circle_stroke(c, SNAP_RING + 1.0, Stroke::new(3.0, Color32::WHITE));
        painter.circle_stroke(c, SNAP_RING, Stroke::new(2.0, T.selection));
    }
    // What a drag from here does to raw shapes: Animate's corner and curve marks by the pointer.
    if app.tool == Tool::Select
        && app.drag == Drag::None
        && !panning
        && !multi
        && !app.modal_open()
        && let Some(p) = pointer.filter(|p| canvas.contains(*p))
        && let Some((_, grab)) = app.s.raw_grab(to_v(to_stage(p)), GRAB_EDGE / zoom, GRAB_POINT / zoom)
    {
        // With Ctrl (⌘) an edge takes a new corner point there: the corner mark (#200).
        let grab = match grab {
            Grab::Segment { at, .. } if command => Grab::Point { at },
            g => g,
        };
        grab_mark(&painter, p, grab);
    }

    // Shape hints sit on top of the artwork and the selection (fresh: a drag just moved one).
    if hint_tools {
        let marks = crate::shape_hints::markers(app);
        let hot = match app.drag {
            Drag::Hint { index, .. } => Some(index),
            _ => hint_hot.map(|m| m.index),
        };
        crate::shape_hints::draw(&painter, &marks, to_screen, hot);
    }

    text_editor(app, ui, to_screen(Vec2::new(0.0, 0.0)), zoom);

    // Inside a symbol with 9-slice guides: the guides, across the stage (ADR 0055).
    let slice_drag = match app.drag {
        Drag::SliceGuide { guide } => Some(guide),
        _ => slice_hot,
    };
    crate::scale_grid::draw(app, &painter, xf, canvas, slice_drag);
    // Inside a symbol: its registration point (ADR 0045; the way back is in the edit bar).
    if edit_view.is_some() {
        let c = to_screen(Vec2::ZERO);
        for (a, b) in [(vec2(-8.0, 0.0), vec2(8.0, 0.0)), (vec2(0.0, -8.0), vec2(0.0, 8.0))] {
            painter.line_segment([c + a, c + b], Stroke::new(3.0, Color32::WHITE));
            painter.line_segment([c + a, c + b], Stroke::new(1.0, T.ink));
        }
    }

    // zoom badge
    let badge = format!("{:.0}%", st_zoom * 100.0);
    painter.text(canvas.right_bottom() + vec2(-10.0, -8.0), egui::Align2::RIGHT_BOTTOM, badge, crate::theme::font(crate::theme::Type::Data), T.text_dim);
    // Last, so the focus ring is drawn over the artwork.
    let selected = app.s.selection.len();
    let on = app.s.selected_layers().len();
    let (tool, frame) = (crate::i18n::t(app.tool.label()), app.s.frame + 1);
    let name = match (selected, on) {
        (0, _) => tr!("Stage · {tool} tool · frame {frame}", tool = tool, frame = frame),
        (n, 0 | 1) => trn!("Stage · {tool} tool · frame {frame} · {n} element selected", "Stage · {tool} tool · frame {frame} · {n} elements selected", n, tool = tool, frame = frame),
        // A selection across layers (ADR 0044): the layers' phrase is its own plural.
        (n, l) => trn!(
            "Stage · {tool} tool · frame {frame} · {n} element selected {on_layers}",
            "Stage · {tool} tool · frame {frame} · {n} elements selected {on_layers}",
            n,
            tool = tool,
            frame = frame,
            on_layers = trn!("on {n} layer", "on {n} layers", l)
        ),
    };
    crate::a11y::surface(ui, &resp, &name);
    // Its elements, one by one, while it has the keyboard (ADR 0064).
    crate::stage_a11y::show(app, ui, &resp, to_screen);
    // The rulers last, with the pointer's place marked on them (ADR 0076).
    crate::rulers::paint(ui, &bars, geo, pointer.or_else(|| ui.input(|i| i.pointer.latest_pos())));
}

// ----- raw shapes (ADR 0031, 0041) ----------------------------------------------------------------

/// How near (screen pixels) the Selection tool takes a raw edge to bend, and a corner or end to
/// move (Animate's curve and corner pointers).
const GRAB_EDGE: f32 = 3.0;
const GRAB_POINT: f32 = 6.0;
/// Dragged points and selections snap within this many screen pixels (issue #42).
const SNAP_PX: f32 = 10.0;
/// The snap ring's radius (screen pixels).
const SNAP_RING: f32 = 6.0;

/// Where a point drawn at stage point `p` goes (the Line, Rectangle, Oval, PolyStar and Pen tools,
/// ADR 0045): snapped within 10 screen pixels to an anchor, else a line or fill edge, while Snap
/// to Objects is on. The snap ring shows where it snapped.
pub fn snap_draw(app: &mut App, p: Pos2, zoom: f32) -> Pos2 {
    let v = app.s.execute("draw.snap", &json!({"x": p.x, "y": p.y, "snap": SNAP_PX / zoom.max(1e-3)}));
    app.snap_ring = v.as_ref().ok().and_then(ring_at);
    v.as_ref().ok().and_then(snapped_at).map_or(p, |q| pos2(q.x, q.y))
}

/// A Selection-tool move, this frame: one of the selection's corners (the one nearest the
/// pointer) snaps within 10 screen pixels.
fn snap_move(app: &mut App, c: Pos2, zoom: f32, shift: bool) {
    let Drag::SnapMove { start, applied } = app.drag else { return };
    // Shift keeps the move horizontal, vertical or at 45° (Animate), and wins over snapping.
    let total = if shift { constrain_45(c - start) } else { c - start };
    let d = total - applied;
    if d.length_sq() <= 0.0 {
        return;
    }
    let snap = if shift { 0.0 } else { SNAP_PX / zoom };
    let p = json!({"dx": d.x, "dy": d.y, "snap": snap, "from": [c.x, c.y]});
    if let Some(v) = app.run("element.translate", p) {
        let f = |k: &str| v.get(k).and_then(serde_json::Value::as_f64).unwrap_or(0.0) as f32;
        app.drag = Drag::SnapMove { start, applied: applied + vec2(f("dx"), f("dy")) };
        app.snap_ring = ring_at(&v);
        // Subselection's shown points go with the drawing.
        crate::pathtools::raw_follow(app, Vec2::new(f("dx"), f("dy")), true);
    }
}

/// Free Transform (ADR 0076): the pointer `c`, moved so that the handle being dragged (a scale
/// handle, or while moving the box's corner or centre nearest where it was pressed) lands where
/// the snap engine puts it: an object's point or line, a guide, the grid or a pixel. The
/// selection itself is no target. Rotation doesn't snap.
fn xf_snap(s: &mut anicraft_engine::Session, ring: &mut Option<Vec2>, kind: XfKind, b: [f32; 4], start: Pos2, c: Pos2, zoom: f32) -> Pos2 {
    let side = |h: i8, lo: f32, hi: f32| match h {
        ..0 => lo,
        0 => (lo + hi) * 0.5,
        _ => hi,
    };
    let handle = match kind {
        XfKind::Scale { hx, hy } => pos2(side(hx, b[0], b[2]), side(hy, b[1], b[3])),
        XfKind::Move => {
            let pts = [pos2(b[0], b[1]), pos2(b[2], b[1]), pos2(b[2], b[3]), pos2(b[0], b[3]), pos2((b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5)];
            pts.into_iter().min_by(|p, q| p.distance(start).total_cmp(&q.distance(start))).unwrap_or(start)
        }
        XfKind::Rotate => {
            *ring = None;
            return c;
        }
    };
    let h = handle + (c - start);
    let v = s.execute("draw.snap", &json!({"x": h.x, "y": h.y, "snap": SNAP_PX / zoom.max(1e-3), "moving": true}));
    let v = v.ok();
    *ring = v.as_ref().and_then(ring_at);
    v.as_ref().and_then(snapped_at).map_or(c, |q| c + (pos2(q.x, q.y) - h))
}

/// `d` along the nearest of the horizontal, vertical and 45° directions.
pub(crate) fn constrain_45(d: egui::Vec2) -> egui::Vec2 {
    let r = std::f32::consts::FRAC_1_SQRT_2;
    let dirs = [vec2(1.0, 0.0), vec2(0.0, 1.0), vec2(r, r), vec2(r, -r)];
    let best = dirs.into_iter().map(|u| (u, d.dot(u))).max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()));
    best.map_or(d, |(u, k)| u * k)
}

/// A shared point's drag, this frame: the point (and everything on it) goes under the pointer,
/// snapping within 10 screen pixels.
fn move_raw_point(app: &mut App, c: Pos2, zoom: f32) {
    let Drag::RawPoint { at, off } = app.drag else { return };
    let to = c + off;
    if to == at {
        return;
    }
    // The other selected anchors (Subselection, ADR 0042) move along; once the first frame has
    // found every shape's copy of the points, `points` carries them.
    let with: Vec<[f32; 2]> = app.raw_with.iter().map(|q| [q.x, q.y]).collect();
    let p = json!({"x": at.x, "y": at.y, "to_x": to.x, "to_y": to.y, "snap": SNAP_PX / zoom, "points": app.raw_points, "with": with, "merge": false});
    let Some(v) = app.run("raw.move_point", p) else { return };
    let f = |k: &str| v.get(k).and_then(serde_json::Value::as_f64).unwrap_or(0.0) as f32;
    crate::pathtools::raw_follow(app, Vec2::new(f("x") - at.x, f("y") - at.y), false);
    app.drag = Drag::RawPoint { at: pos2(f("x"), f("y")), off };
    app.snap_ring = ring_at(&v);
    if let Some(points) = v.get("points").filter(|p| p.as_array().is_some_and(|a| !a.is_empty())) {
        let ids: Vec<serde_json::Value> = points.as_array().into_iter().flatten().filter_map(|pt| pt.get(0).cloned()).collect();
        app.raw_points = points.clone();
        touched(app, &serde_json::Value::Array(ids));
    }
}

/// A Selection-tool press at `sp` (stage) on raw shapes (ADR 0041): an unselected corner or end
/// moves, an unselected segment or fill edge bends; anything else is picked (unless it already
/// is) and moved. With `corner` (Ctrl, ⌘ on macOS), as in Animate, an edge gets a corner point
/// where it was pressed and the drag pulls that point, so the edge peaks there (#200).
fn select_press(app: &mut App, sp: Pos2, shift: bool, corner: bool, copy: bool, zoom: f32) -> Drag {
    let point_drag = |app: &mut App, at: Vec2| {
        app.raw_points = serde_json::Value::Null;
        app.raw_with.clear();
        app.raw_touched.clear();
        Drag::RawPoint { at: pos2(at.x, at.y), off: pos2(at.x, at.y) - sp }
    };
    match app.s.raw_grab(to_v(sp), GRAB_EDGE / zoom, GRAB_POINT / zoom) {
        Some((_, Grab::Point { at })) => {
            app.s.begin_gesture(tr_noop!("Move Point"));
            point_drag(app, at)
        }
        Some((layer, Grab::Segment { id, cmd, t, at })) => {
            if layer != app.s.layer {
                app.s.layer = layer;
                app.s.selection.clear();
            }
            if corner {
                // The new point, its corner and its move are one undo step. Where no point can
                // go (one is there already), the press bends the edge as without Ctrl.
                app.s.begin_gesture(tr_noop!("Add Corner Point"));
                if let Ok(v) = app.s.execute("raw.add_point", &json!({"x": at.x, "y": at.y, "slop": GRAB_EDGE / zoom.max(1e-3)})) {
                    let f = |k: &str| v.get(k).and_then(serde_json::Value::as_f64).map(|x| x as f32);
                    if let (Some(x), Some(y)) = (f("x"), f("y")) {
                        // A curved edge splits into two curves meeting smoothly: drawing the new
                        // point's handles in makes it a corner, so the sides meet in a peak.
                        let _ = app.s.execute("raw.convert_points", &json!({"points": [[x, y]], "smooth": false}));
                        let drag = point_drag(app, Vec2::new(x, y));
                        touched(app, &v["changed"]);
                        return drag;
                    }
                }
                app.s.end_gesture();
            }
            app.s.begin_gesture(tr_noop!("Bend"));
            app.raw_touched.clear();
            Drag::Bend { id, cmd, t, last: sp }
        }
        Some((layer, Grab::Fill { id } | Grab::Object { id })) => {
            if !app.s.selection.contains(layer, id) {
                app.run("element.pick", json!({"x": sp.x, "y": sp.y, "add": shift}));
            }
            if copy {
                // Alt (Option) drags a copy away and leaves the original whole (Animate, ADR 0073).
                app.s.begin_gesture(tr_noop!("Copy"));
                app.run("edit.duplicate", json!({"dx": 0, "dy": 0}));
            } else {
                app.s.begin_gesture(tr_noop!("Move"));
            }
            Drag::SnapMove { start: sp, applied: egui::Vec2::ZERO }
        }
        None => Drag::Marquee { start: sp, cur: sp },
    }
}

/// The end of a drag that bent an edge or moved raw points or handles: what it changed merges
/// again with what it touches, in the same step.
fn end_raw_drag(app: &mut App) {
    if !app.raw_touched.is_empty() {
        let ids = std::mem::take(&mut app.raw_touched);
        app.run("raw.merge", json!({"ids": ids}));
    }
    app.raw_points = serde_json::Value::Null;
    app.s.end_gesture();
}

/// Where a command's result says the drag snapped.
fn snapped_at(v: &serde_json::Value) -> Option<Vec2> {
    let s = v.get("snapped")?;
    Some(Vec2::new(s.get("x")?.as_f64()? as f32, s.get("y")?.as_f64()? as f32))
}

/// Where the snap ring goes: where it snapped, unless only to a whole pixel (Snap to Pixels
/// moves every point a little; a ring on each would say nothing).
fn ring_at(v: &serde_json::Value) -> Option<Vec2> {
    snapped_at(v).filter(|_| v["snapped"]["to"] != "pixel")
}

/// Remembers the raw shapes a drag changed (`ids`: a list of element ids).
fn touched(app: &mut App, ids: &serde_json::Value) {
    for id in ids.as_array().into_iter().flatten().filter_map(serde_json::Value::as_u64) {
        if !app.raw_touched.contains(&id) {
            app.raw_touched.push(id);
        }
    }
}

/// The mark beside the pointer: a corner for a point that moves, an arc for an edge that bends.
fn grab_mark(painter: &egui::Painter, p: Pos2, grab: Grab) {
    let o = p + vec2(12.0, 14.0);
    let pts: Vec<Pos2> = match grab {
        Grab::Point { .. } => vec![o, o + vec2(0.0, 7.0), o + vec2(7.0, 7.0)],
        Grab::Segment { .. } => (0..=8)
            .map(|i| {
                let t = i as f32 / 8.0;
                o + vec2(8.0 * t, 7.0 - 7.0 * (4.0 * t * (1.0 - t)))
            })
            .collect(),
        Grab::Fill { .. } | Grab::Object { .. } => return,
    };
    painter.add(egui::Shape::line(pts.clone(), Stroke::new(3.0, Color32::WHITE)));
    painter.add(egui::Shape::line(pts, Stroke::new(1.5, T.selection)));
}

/// The raw shapes (parts of the layer's merged drawing) on the active layer.
/// The raw shapes at the playhead, on every layer, with their layers.
fn raw_ids(app: &App) -> std::collections::HashSet<(anicraft_engine::doc::LayerId, anicraft_engine::doc::ElementId)> {
    let layers: Vec<_> = app.s.timeline().layers.iter().map(|l| l.id).collect();
    layers.into_iter().flat_map(|l| app.s.elements_on(l).into_iter().filter(|e| matches!(&e.kind, anicraft_engine::doc::ElementKind::Shape(sh) if sh.merge)).map(move |e| (l, e.id))).collect()
}

/// Spacing of the selection pattern's dots (points).
const GAP: f32 = 3.0;
/// Most single dots drawn in a frame along hairline strokes (wider strokes and fills are spans).
const MAX_DOTS: usize = 60_000;
/// Most pattern spans in a frame: far more than a full-screen drawing needs (a 4K view is ~720
/// rows), so the pattern never stops part way down a shape; it only bounds a pathological one.
const MAX_SPANS: usize = 400_000;

/// The selection pattern's tile: two by two dot cells, dark and light in a checker, at four
/// texels a point so the 1.5-point dots stay crisp. Drawn repeating, on the screen grid.
fn pattern_image(dark: Color32) -> egui::ColorImage {
    const PER_PT: usize = 4;
    let cell = (GAP as usize) * PER_PT;
    let size = 2 * cell;
    let (lo, hi) = (cell / 4, cell - cell / 4);
    let pixels = (0..size * size)
        .map(|i| {
            let (x, y) = (i % size, i / size);
            let inside = (lo..hi).contains(&(x % cell)) && (lo..hi).contains(&(y % cell));
            match inside {
                false => Color32::TRANSPARENT,
                true if (x / cell + y / cell).is_multiple_of(2) => dark,
                true => Color32::WHITE,
            }
        })
        .collect();
    egui::ColorImage::new([size, size], pixels)
}

/// The pattern texture for the theme's selection colour (made again when the theme changes).
fn pattern_texture(cache: &mut Option<(Color32, egui::TextureHandle)>, ctx: &egui::Context) -> egui::TextureId {
    let dark = T.selection;
    if let Some((c, t)) = cache.as_ref()
        && *c == dark
    {
        return t.id();
    }
    let t = ctx.load_texture("selection-pattern", pattern_image(dark), egui::TextureOptions::NEAREST_REPEAT);
    let id = t.id();
    *cache = Some((dark, t));
    id
}

/// Adds the pattern over `spans` to `mesh` (textured with [`pattern_texture`]). The texture
/// coordinates come from screen positions, so the dots sit on a fixed screen grid and don't
/// crawl when the view pans; a dot's centre is on a multiple of [`GAP`].
fn pattern_mesh(mesh: &mut egui::Mesh, spans: &[Rect]) {
    let tile = 2.0 * GAP;
    let uv = |p: Pos2| pos2((p.x + GAP * 0.5) / tile, (p.y + GAP * 0.5) / tile);
    for r in spans {
        mesh.add_rect_with_uv(*r, Rect::from_min_max(uv(r.min), uv(r.max)), Color32::WHITE);
    }
}

/// Animate's selection look for raw shapes: fine dots over the selected fill or stroke, light and
/// dark in a checker so they show on any colour. `m` maps the item's path to the screen. Fills
/// and strokes wider than the dot spacing become `spans` (one rectangle per row of dots per run
/// inside them, for the pattern texture), so a shape filling the view costs a few hundred
/// rectangles however far it is zoomed in (#204: single dots under one budget ran out part way
/// down a big or zoomed-in shape, and the shading stopped there). Hairline strokes get one row of
/// dots along them in `mesh`.
fn selection_pattern(spans: &mut Vec<Rect>, mesh: &mut egui::Mesh, item: &anicraft_engine::doc::DisplayItem, m: &Affine, clip: Rect, budget: &mut usize) {
    use anicraft_engine::geom::FillRule;
    if item.fill.is_some() {
        pattern_spans(&item.path.flatten(m, 0.5), item.fill_rule, clip, spans);
    }
    let Some(st) = &item.stroke else { return };
    let half = (st.reach() * m.mean_scale()).max(0.5);
    if half >= GAP {
        // The stroke's own outline (caps, joins and dashes as drawn), filled non-zero.
        let outline = anicraft_engine::render::stroke::stroke_outline(&item.path, m, st, 0.5);
        pattern_spans(&outline.flatten(&Affine::IDENTITY, 0.5), FillRule::NonZero, clip, spans);
        return;
    }
    let mut dot = |p: Pos2| {
        if clip.contains(p) && *budget > 0 {
            *budget -= 1;
            let dark = ((p.x / GAP).round() as i64 + (p.y / GAP).round() as i64).rem_euclid(2) == 0;
            mesh.add_colored_rect(Rect::from_center_size(p, vec2(1.5, 1.5)), if dark { T.selection } else { Color32::WHITE });
        }
        *budget > 0
    };
    // Along the stroke; segments off the screen are skipped.
    let near = clip.expand(half + GAP);
    for pl in &item.path.flatten(m, 0.5) {
        let n = pl.points.len();
        let segs = if pl.closed { n } else { n.saturating_sub(1) };
        let mut carry = 0.0;
        for i in 0..segs {
            let (Some(&a), Some(&c)) = (pl.points.get(i), pl.points.get((i + 1) % n)) else { continue };
            let d = c - a;
            let len = d.length();
            if len <= 1e-6 {
                continue;
            }
            if !near.intersects(Rect::from_two_pos(pos2(a.x, a.y), pos2(c.x, c.y))) {
                carry = (carry - len).rem_euclid(GAP);
                continue;
            }
            let dir = d * (1.0 / len);
            let mut t = carry;
            while t < len {
                let p = a + dir * t;
                if !dot(pos2(p.x, p.y)) {
                    return;
                }
                t += GAP;
            }
            carry = t - len;
        }
    }
}

/// The runs inside the closed outlines `lines` (screen space, filled by `rule`) along each row
/// of dots, as rectangles one row of dots tall, clipped to `clip`. Rows are scanned with an
/// active edge list, so the cost follows the outline's edges and the rows on screen, not the
/// area covered.
pub(crate) fn pattern_spans(lines: &[anicraft_engine::geom::Polyline], rule: anicraft_engine::geom::FillRule, clip: Rect, out: &mut Vec<Rect>) {
    use anicraft_engine::geom::FillRule;
    // Every non-horizontal edge near the view, as (top, bottom, x at top, dx/dy, winding).
    let mut edges: Vec<(f32, f32, f32, f32, i32)> = Vec::new();
    for pl in lines {
        let n = pl.points.len();
        if n < 2 {
            continue;
        }
        for i in 0..n {
            let (Some(&a), Some(&c)) = (pl.points.get(i), pl.points.get((i + 1) % n)) else { continue };
            if a.y == c.y || !(a.is_finite() && c.is_finite()) {
                continue;
            }
            let (top, bottom, w) = if a.y < c.y { (a, c, 1) } else { (c, a, -1) };
            if bottom.y < clip.top() - GAP || top.y > clip.bottom() + GAP {
                continue;
            }
            edges.push((top.y, bottom.y, top.x, (bottom.x - top.x) / (bottom.y - top.y), w));
        }
    }
    if edges.is_empty() {
        return;
    }
    edges.sort_by(|a, b| a.0.total_cmp(&b.0));
    let last = edges.iter().fold(f32::NEG_INFINITY, |m, e| m.max(e.1)).min(clip.bottom() + GAP * 0.5);
    let mut y = (edges.first().map_or(0.0, |e| e.0).max(clip.top() - GAP * 0.5) / GAP).ceil() * GAP;
    let inside = |w: i32| match rule {
        FillRule::NonZero => w != 0,
        FillRule::EvenOdd => w % 2 != 0,
    };
    let (mut next, mut active, mut xs): (usize, Vec<usize>, Vec<(f32, i32)>) = (0, Vec::new(), Vec::new());
    while y <= last && out.len() < MAX_SPANS {
        while edges.get(next).is_some_and(|e| e.0 <= y) {
            active.push(next);
            next += 1;
        }
        // Half-open in y, so a vertex shared by two edges counts once.
        active.retain(|&i| edges.get(i).is_some_and(|e| e.1 > y));
        xs.clear();
        xs.extend(active.iter().filter_map(|&i| edges.get(i)).map(|e| (e.2 + (y - e.0) * e.3, e.4)));
        xs.sort_by(|a, b| a.0.total_cmp(&b.0));
        let (top, bottom) = ((y - GAP * 0.5).max(clip.top()), (y + GAP * 0.5).min(clip.bottom()));
        let (mut w, mut from) = (0, 0.0);
        for &(x, d) in &xs {
            let was = inside(w);
            w += d;
            match (was, inside(w)) {
                (false, true) => from = x,
                (true, false) => {
                    let (x0, x1) = (from.max(clip.left()), x.min(clip.right()));
                    if x1 > x0 && bottom > top {
                        out.push(Rect::from_min_max(pos2(x0, top), pos2(x1, bottom)));
                    }
                }
                _ => {}
            }
        }
        y += GAP;
    }
}

#[cfg(test)]
mod pattern_tests {
    use super::*;
    use anicraft_engine::geom::{FillRule, Path};

    fn area(spans: &[Rect]) -> f32 {
        spans.iter().map(|r| r.area()).sum()
    }

    /// #204: a shape far bigger than the view (zoomed in) is shaded all the way down, not just its
    /// top rows, and costs one span per row rather than one quad per dot.
    #[test]
    fn a_zoomed_in_shape_is_shaded_to_the_bottom_of_the_view() {
        let clip = Rect::from_min_max(pos2(0.0, 0.0), pos2(3840.0, 2160.0));
        let mut spans = Vec::new();
        let big = Path::rect(-5000.0, -4000.0, 20000.0, 15000.0);
        pattern_spans(&big.flatten(&Affine::IDENTITY, 0.5), FillRule::NonZero, clip, &mut spans);
        let bottom = spans.iter().fold(0.0f32, |m, r| m.max(r.max.y));
        assert_eq!(bottom, clip.bottom(), "the pattern reaches the bottom of the view");
        assert!((area(&spans) - clip.area()).abs() < 1.0, "the whole view is covered: {} of {}", area(&spans), clip.area());
        assert_eq!(spans.len(), 721, "one span per row of dots");
        // The single-dot overlay this replaced stopped after 60 000 dots: 46 rows of 1 280.
        assert!(clip.area() / (GAP * GAP) > 60_000.0 * 10.0);
    }

    /// Holes stay unshaded (even-odd and non-zero), and the spans follow the outline.
    #[test]
    fn spans_follow_the_fill_rule_and_the_outline() {
        let clip = Rect::from_min_max(pos2(0.0, 0.0), pos2(400.0, 400.0));
        let mut ring = Path::rect(0.0, 0.0, 300.0, 300.0);
        ring.cmds.extend(Path::rect(100.0, 100.0, 100.0, 100.0).cmds);
        for rule in [FillRule::EvenOdd, FillRule::NonZero] {
            let mut spans = Vec::new();
            pattern_spans(&ring.flatten(&Affine::IDENTITY, 0.5), rule, clip, &mut spans);
            let in_hole = spans.iter().any(|r| r.contains(pos2(150.0, 150.0)));
            assert_eq!(in_hole, rule == FillRule::NonZero, "{rule:?}: both squares wind the same way");
            assert!(spans.iter().all(|r| r.min.x >= 0.0 && r.max.x <= 300.0 && r.max.y <= 301.5));
            assert!(spans.iter().any(|r| r.contains(pos2(50.0, 150.0))));
        }
    }

    fn item(path: Path, fill: bool, stroke: Option<f32>) -> anicraft_engine::doc::DisplayItem {
        use anicraft_engine::doc::{ColorTransform, DisplayItem, ElementId, LayerId, Paint, Rgba, Stroke};
        DisplayItem {
            path: std::sync::Arc::new(path),
            transform: Affine::IDENTITY,
            fill: fill.then_some(Paint::Solid(Rgba::new(1.0, 0.0, 0.0, 1.0))),
            stroke: stroke.map(|w| Stroke::new(Rgba::new(0.0, 0.0, 0.0, 1.0), w)),
            fill_rule: FillRule::NonZero,
            color: ColorTransform::IDENTITY,
            layer: LayerId(0),
            element: ElementId(0),
            outline: None,
            clip: None,
            masks: None,
            guide: false,
            group: None,
            image: None,
            smooth: true,
            warp: None,
        }
    }

    /// A wide stroke is shaded over its whole outline (beyond the fill), a hairline gets dots
    /// along it, and nothing off the view is drawn.
    #[test]
    fn strokes_are_shaded_wide_as_spans_and_hairlines_as_dots() {
        let clip = Rect::from_min_max(pos2(0.0, 0.0), pos2(400.0, 400.0));
        let square = || Path::rect(100.0, 100.0, 200.0, 200.0);
        let (mut spans, mut dots, mut budget) = (Vec::new(), egui::Mesh::default(), MAX_DOTS);
        selection_pattern(&mut spans, &mut dots, &item(square(), false, Some(20.0)), &Affine::IDENTITY, clip, &mut budget);
        assert!(spans.iter().any(|r| r.min.x < 100.0) && !spans.iter().any(|r| r.contains(pos2(200.0, 200.0))), "the outline only, its outer half included");
        assert!(dots.vertices.is_empty());
        let mut spans = Vec::new();
        selection_pattern(&mut spans, &mut dots, &item(square(), true, Some(0.5)), &Affine::IDENTITY, clip, &mut budget);
        assert!(spans.iter().any(|r| r.contains(pos2(200.0, 200.0))), "the fill");
        let n = dots.vertices.len() / 4;
        assert!((250..=290).contains(&n), "a dot every {GAP} along the 800-long hairline: {n}");
        // Wholly off the view: nothing.
        let (mut spans, mut dots) = (Vec::new(), egui::Mesh::default());
        selection_pattern(&mut spans, &mut dots, &item(Path::rect(900.0, 900.0, 50.0, 50.0), true, Some(0.5)), &Affine::IDENTITY, clip, &mut budget);
        assert!(spans.is_empty() && dots.vertices.is_empty());
        // The pattern mesh maps each span to the repeating tile.
        let mut mesh = egui::Mesh::default();
        pattern_mesh(&mut mesh, &[Rect::from_min_max(pos2(-1.5, -1.5), pos2(4.5, 1.5))]);
        assert_eq!(mesh.vertices.first().map(|v| v.uv), Some(pos2(0.0, 0.0)));
        assert_eq!(mesh.vertices.last().map(|v| v.uv), Some(pos2(1.0, 0.5)));
    }

    /// The whole window, headless, with a big selected raw shape zoomed in (`--open selectzoom`):
    /// the pattern is drawn with its texture, made once for the theme's colour.
    #[test]
    fn a_selected_raw_shape_draws_its_pattern_in_the_window() {
        let ctx = egui::Context::default();
        let session = anicraft_engine::Session::new(anicraft_engine::doc::sample::demo());
        let mut app = App::with_context(&ctx, session, Some(std::path::PathBuf::from("unused.png")));
        app.open_panel(Some("selectzoom"));
        assert!(!app.s.selection.is_empty() && app.view.zoom == 4.0);
        let input = || egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 940.0))), ..Default::default() };
        for _ in 0..2 {
            let mut out = ctx.run_ui(input(), |ui| app.frame(ui));
            out.textures_delta.clear();
        }
        let first = app.stage_cache.pattern.as_ref().map(|(c, t)| (*c, t.id()));
        assert_eq!(first.map(|f| f.0), Some(T.selection));
        let mut out = ctx.run_ui(input(), |ui| app.frame(ui));
        out.textures_delta.clear();
        assert_eq!(app.stage_cache.pattern.as_ref().map(|(c, t)| (*c, t.id())), first, "made once");
    }

    /// The tile's dots sit on the screen grid in the old checker: dark where the row and column
    /// numbers add up to an even number.
    #[test]
    fn the_pattern_tile_is_a_checker_of_dots() {
        let img = pattern_image(Color32::BLUE);
        let size = img.size[0];
        let at = |pt: (f32, f32)| {
            // Screen point → texel, as `pattern_mesh` maps it.
            let u = ((pt.0 + GAP * 0.5) / (2.0 * GAP)).rem_euclid(1.0);
            let v = ((pt.1 + GAP * 0.5) / (2.0 * GAP)).rem_euclid(1.0);
            img.pixels[(v * size as f32) as usize * size + (u * size as f32) as usize]
        };
        assert_eq!(at((0.0, 0.0)), Color32::BLUE);
        assert_eq!(at((3.0, 0.0)), Color32::WHITE);
        assert_eq!(at((3.0, 3.0)), Color32::BLUE);
        assert_eq!(at((1.4, 0.0)), Color32::TRANSPARENT, "space between dots");
    }
}

// ----- text ----------------------------------------------------------------------------------------

/// Starts editing the text element under stage point `sp`, if there is one.
fn open_text_at(app: &mut App, sp: Pos2, tolerance: f32) -> bool {
    let Some((_, id)) = app.s.hit_test(to_v(sp), tolerance) else { return false };
    let found = app.s.visible_elements().into_iter().find(|e| e.id == id).and_then(|e| match &e.kind {
        anicraft_engine::doc::ElementKind::Text(t) => Some((e.transform.apply(Vec2::ZERO), t.text.clone())),
        _ => None,
    });
    let Some((at, text)) = found else { return false };
    app.run("element.select", json!({"ids": [id.0]}));
    app.text_edit = Some(crate::app::TextEditing { at: pos2(at.x, at.y), element: Some(id), original: text.clone(), text, focus: true });
    true
}

/// Commits the text being typed: new text is created, edited text replaced (one undo step
/// each); emptied text is deleted, and unchanged text commits nothing.
pub fn commit_text(app: &mut App) {
    let Some(te) = app.text_edit.take() else { return };
    match te.element {
        None if te.text.trim().is_empty() => {}
        None => {
            app.run("text.create", json!({"text": te.text, "x": te.at.x, "y": te.at.y}));
        }
        Some(_) if te.text == te.original => {}
        Some(id) => {
            app.run("element.select", json!({"ids": [id.0]}));
            if te.text.trim().is_empty() {
                app.run("edit.delete", json!({}));
            } else {
                app.run("text.edit", json!({"text": te.text}));
            }
        }
    }
}

/// The field text is typed in, over the stage at the text's top-left corner. Esc or a click
/// elsewhere finishes; Enter starts a new line.
fn text_editor(app: &mut App, ui: &mut egui::Ui, stage_origin: Pos2, zoom: f32) {
    let Some(te) = app.text_edit.as_mut() else { return };
    let at = stage_origin + vec2(te.at.x * zoom, te.at.y * zoom);
    let size = (app.s.text_style.size * zoom).clamp(12.0, 64.0);
    let mut done = false;
    egui::Area::new(egui::Id::new("stage_text_edit")).fixed_pos(at).order(egui::Order::Foreground).show(ui.ctx(), |ui| {
        egui::Frame::new().fill(T.panel).stroke(Stroke::new(1.0, T.selection)).inner_margin(4.0).show(ui, |ui| {
            let r = ui.add(egui::TextEdit::multiline(&mut te.text).font(crate::theme::stage_text_font(size)).desired_rows(1).desired_width(260.0f32.max(size * 8.0)).hint_text(tr!("Type text"))).named(tr!("Text on the stage"));
            if te.focus {
                r.request_focus();
                te.focus = false;
            } else if r.lost_focus() {
                done = true;
            }
        });
    });
    if done {
        commit_text(app);
    }
}

// ----- free transform ---------------------------------------------------------------------------

/// Screen-space handle size and the rotation zone just outside each corner.
const HANDLE: f32 = 8.0;
const ROTATE_ZONE: f32 = 22.0;

fn bounds_arr(b: anicraft_engine::geom::Rect) -> Option<[f32; 4]> {
    (!b.is_empty()).then_some([b.min.x, b.min.y, b.max.x, b.max.y])
}

/// Handle centres in screen space with their (hx, hy) codes.
fn handles(r: Rect) -> [(Pos2, i8, i8); 8] {
    let p = |hx: i8, hy: i8| pos2(r.left() + r.width() * (f32::from(hx) + 1.0) / 2.0, r.top() + r.height() * (f32::from(hy) + 1.0) / 2.0);
    [(-1, -1), (0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0)].map(|(x, y)| (p(x, y), x, y))
}

/// What dragging from screen point `p` would do to a selection drawn as `r`.
fn xf_zone(r: Rect, p: Pos2) -> Option<XfKind> {
    for (h, hx, hy) in handles(r) {
        if (p.x - h.x).abs() <= HANDLE && (p.y - h.y).abs() <= HANDLE {
            return Some(XfKind::Scale { hx, hy });
        }
    }
    if !r.contains(p) && [r.left_top(), r.right_top(), r.left_bottom(), r.right_bottom()].iter().any(|c| c.distance(p) <= ROTATE_ZONE) {
        return Some(XfKind::Rotate);
    }
    r.expand(2.0).contains(p).then_some(XfKind::Move)
}

/// The whole transform of a free-transform drag from `start` to `cur` (stage space).
pub(crate) fn xf_total(kind: XfKind, b: [f32; 4], start: Pos2, cur: Pos2, shift: bool, alt: bool) -> Affine {
    let [x0, y0, x1, y1] = b;
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let about = |px: f32, py: f32, m: Affine| Affine::translate(px, py).mul(&m).mul(&Affine::translate(-px, -py));
    match kind {
        XfKind::Move => {
            let mut d = cur - start;
            if shift {
                // constrain to the dominant axis
                if d.x.abs() > d.y.abs() {
                    d.y = 0.0;
                } else {
                    d.x = 0.0;
                }
            }
            Affine::translate(d.x, d.y)
        }
        XfKind::Scale { hx, hy } => {
            let px = if alt { cx } else { [x1, cx, x0][(hx + 1) as usize] };
            let py = if alt { cy } else { [y1, cy, y0][(hy + 1) as usize] };
            let factor = |s: f32, c: f32, p: f32| {
                let den = s - p;
                if den.abs() < 1e-3 { 1.0 } else { (c - p) / den }
            };
            let mut sx = if hx != 0 { factor(start.x, cur.x, px) } else { 1.0 };
            let mut sy = if hy != 0 { factor(start.y, cur.y, py) } else { 1.0 };
            if shift {
                if hx != 0 && hy != 0 {
                    let m = sx.abs().max(sy.abs());
                    sx = m.copysign(sx);
                    sy = m.copysign(sy);
                } else if hx != 0 {
                    sy = sx.abs();
                } else {
                    sx = sy.abs();
                }
            }
            let clamp = |s: f32| if s.abs() < 0.01 { 0.01f32.copysign(if s == 0.0 { 1.0 } else { s }) } else { s };
            about(px, py, Affine::scale(clamp(sx), clamp(sy)))
        }
        XfKind::Rotate => {
            let a0 = (start.y - cy).atan2(start.x - cx);
            let a1 = (cur.y - cy).atan2(cur.x - cx);
            let mut a = a1 - a0;
            if shift {
                let step = 15f32.to_radians();
                a = (a / step).round() * step;
            }
            about(cx, cy, Affine::rotate(a))
        }
    }
}

fn constrain(tool: Tool, start: Pos2, cur: Pos2, shift: bool) -> Pos2 {
    if !shift {
        return cur;
    }
    let d = cur - start;
    match tool {
        Tool::Line => {
            let ang = d.y.atan2(d.x);
            let snap = (ang / std::f32::consts::FRAC_PI_4).round() * std::f32::consts::FRAC_PI_4;
            let len = d.length();
            start + vec2(snap.cos(), snap.sin()) * len
        }
        _ => {
            let m = d.x.abs().max(d.y.abs());
            start + vec2(m * d.x.signum(), m * d.y.signum())
        }
    }
}

fn create_shape(app: &mut App, start: Pos2, cur: Pos2) {
    let d = cur - start;
    if d.length() < 1.0 {
        return;
    }
    match app.tool {
        Tool::Rect => {
            app.draw("draw.rect", json!({"x": start.x, "y": start.y, "w": d.x, "h": d.y, "radius": app.rect_radius}));
        }
        Tool::Oval => {
            app.draw("draw.ellipse", json!({"x": start.x, "y": start.y, "w": d.x, "h": d.y}));
        }
        Tool::Star => {
            let mut p = app.polystar.params();
            p["cx"] = json!(start.x);
            p["cy"] = json!(start.y);
            p["outer"] = json!(d.length());
            app.draw("draw.star", p);
        }
        Tool::Line => {
            app.draw("draw.line", json!({"x0": start.x, "y0": start.y, "x1": cur.x, "y1": cur.y}));
        }
        _ => {}
    }
}

fn draw_preview(app: &App, painter: &egui::Painter, a: Pos2, b: Pos2, zoom: f32) {
    let st = &app.s.style;
    let fill = if st.use_fill { crate::theme::to_color32(st.fill).gamma_multiply(0.6) } else { Color32::TRANSPARENT };
    let stroke = Stroke::new((st.stroke_width * zoom).max(1.0), if st.use_stroke { crate::theme::to_color32(st.stroke) } else { T.selection });
    match app.tool {
        Tool::Rect => {
            let r = Rect::from_two_pos(a, b);
            let rad = (app.rect_radius * zoom).min(r.width() / 2.0).min(r.height() / 2.0).clamp(0.0, 255.0) as u8;
            painter.rect_filled(r, rad, fill);
            painter.rect_stroke(r, rad, stroke, StrokeKind::Middle);
        }
        Tool::Oval => {
            let r = Rect::from_two_pos(a, b);
            let pts: Vec<Pos2> = (0..64)
                .map(|i| {
                    let t = i as f32 / 64.0 * std::f32::consts::TAU;
                    r.center() + vec2(r.width() / 2.0 * t.cos(), r.height() / 2.0 * t.sin())
                })
                .collect();
            painter.add(egui::Shape::convex_polygon(pts, fill, stroke));
        }
        Tool::Star => {
            // The same corners the command draws (ADR 0080).
            let o = app.polystar;
            let pts: Vec<Pos2> = anicraft_engine::geom::polystar_corners(a.x, a.y, (b - a).length(), o.sides.clamp(3, 32), o.star, o.point_size).into_iter().map(|v| Pos2::new(v.x, v.y)).collect();
            painter.add(egui::Shape::closed_line(pts, stroke));
        }
        Tool::Line => {
            painter.line_segment([a, b], stroke);
        }
        _ => {}
    }
}

#[cfg(test)]
mod draw_tests {
    use super::*;

    fn app() -> App {
        let ctx = egui::Context::default();
        App::with_context(&ctx, anicraft_engine::Session::new(anicraft_engine::doc::Document::default()), Some(std::path::PathBuf::from("unused.png")))
    }

    /// #205, #243: a shape or a line drawn with a tool leaves nothing selected (no box around
    /// the last stroke), in merge drawing and in Object Drawing, and the previous selection
    /// goes too, as in Animate. Undo still takes the shape away in one step.
    #[test]
    fn drawing_tools_leave_nothing_selected() {
        for object in [false, true] {
            let mut app = app();
            app.s.style.object_drawing = object;
            for tool in [Tool::Line, Tool::Rect, Tool::Oval, Tool::Star] {
                app.tool = tool;
                let before = app.s.history_len();
                create_shape(&mut app, pos2(10.0, 10.0), pos2(80.0, 60.0));
                assert_eq!(app.s.history_len(), before + 1, "{tool:?} drew");
                assert!(app.s.selection.is_empty(), "{tool:?}, object {object}: nothing selected");
            }
            app.pencil = (0..20).map(|i| Vec2::new(100.0 + i as f32 * 4.0, 100.0 + (i % 3) as f32)).collect();
            crate::pathtools::commit_pencil(&mut app, crate::pathtools::Xf { origin: Pos2::ZERO, zoom: 1.0 });
            assert!(app.s.selection.is_empty(), "Pencil, object {object}");
            let n = app.s.visible_elements().len();
            app.s.undo().expect("undo");
            assert!(app.s.visible_elements().len() < n, "one undo takes the stroke away");
        }
    }

    /// The area of the raw fill (one on the stage).
    fn fill_area(app: &App) -> f32 {
        let fills: Vec<_> = app.s.visible_elements().into_iter().filter(|e| matches!(&e.kind, anicraft_engine::doc::ElementKind::Shape(sh) if sh.merge && sh.fill.is_some())).collect();
        assert_eq!(fills.len(), 1, "one raw fill");
        let anicraft_engine::doc::ElementKind::Shape(sh) = &fills[0].kind else { return 0.0 };
        sh.path
            .flatten(&fills[0].transform, 0.01)
            .iter()
            .map(|pl| {
                let n = pl.points.len();
                (0..n).filter_map(|i| Some(pl.points.get(i)?.cross(*pl.points.get((i + 1) % n)?))).sum::<f32>() * 0.5
            })
            .sum::<f32>()
            .abs()
    }

    /// Presses the top edge of a raw 100 × 80 rectangle in its middle, drags it 30 up and lets
    /// go, as the stage does.
    fn drag_top_edge(ctrl: bool) -> App {
        let mut app = app();
        app.tool = Tool::Select;
        app.s.style.object_drawing = false;
        app.run("draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 80, "fill": "#ff0000", "stroke": "#000000"}));
        app.run("edit.deselect", json!({}));
        app.drag = select_press(&mut app, pos2(50.0, 0.0), false, ctrl, false, 1.0);
        let to = pos2(50.0, -30.0);
        match app.drag {
            Drag::RawPoint { .. } => move_raw_point(&mut app, to, 1.0),
            Drag::Bend { id, cmd, t, last } => {
                let d = to - last;
                let v = app.run("path.bend_segment", json!({"id": id.0, "cmd": cmd, "t": t, "x": last.x, "y": last.y, "dx": d.x, "dy": d.y, "merge": false})).expect("bent");
                touched(&mut app, &v["changed"]);
            }
            d => panic!("a press on the edge starts {d:?}"),
        }
        end_raw_drag(&mut app);
        app.drag = Drag::None;
        app
    }

    /// #200: Ctrl-drag on an edge makes a corner point with straight sides (Animate); a plain
    /// drag still bends the edge into a curve. One undo takes either back.
    #[test]
    fn ctrl_drag_on_an_edge_makes_a_corner() {
        let mut app = drag_top_edge(true);
        // The rectangle and the triangle (0, 0), (50, -30), (100, 0).
        assert!((fill_area(&app) - 9500.0).abs() < 1.0, "straight sides up to the peak: {}", fill_area(&app));
        let pts = app.s.execute("raw.points", &json!({"ids": app.s.visible_elements().iter().map(|e| e.id.0).collect::<Vec<_>>()})).expect("points");
        assert!(pts.to_string().contains("-30"), "a point at the peak: {pts}");
        app.s.undo().expect("undo");
        assert!((fill_area(&app) - 8000.0).abs() < 1.0, "one undo: the rectangle again");
        let bent = drag_top_edge(false);
        let a = fill_area(&bent);
        assert!(a > 8000.0 + 100.0 && (a - 9500.0).abs() > 50.0, "a plain drag bends it into a curve: {a}");
    }
}

#[cfg(test)]
mod xf_tests {
    use super::*;

    const B: [f32; 4] = [0.0, 0.0, 100.0, 50.0];

    fn apply(m: &Affine, x: f32, y: f32) -> Vec2 {
        m.apply(Vec2::new(x, y))
    }

    #[test]
    fn shift_keeps_a_move_level_upright_or_at_45_degrees() {
        let near = |a: egui::Vec2, b: egui::Vec2| (a - b).length() < 1e-4;
        assert!(near(constrain_45(vec2(10.0, 2.0)), vec2(10.0, 0.0)));
        assert!(near(constrain_45(vec2(-1.0, -9.0)), vec2(0.0, -9.0)));
        assert!(near(constrain_45(vec2(10.0, 9.0)), vec2(9.5, 9.5)));
        assert!(near(constrain_45(vec2(-10.0, 9.0)), vec2(-9.5, 9.5)));
        assert_eq!(constrain_45(egui::Vec2::ZERO), egui::Vec2::ZERO);
    }

    #[test]
    fn corner_scale_keeps_the_opposite_corner_fixed() {
        let m = xf_total(XfKind::Scale { hx: 1, hy: 1 }, B, pos2(100.0, 50.0), pos2(200.0, 100.0), false, false);
        assert_eq!(apply(&m, 0.0, 0.0), Vec2::new(0.0, 0.0));
        assert_eq!(apply(&m, 100.0, 50.0), Vec2::new(200.0, 100.0));
    }

    #[test]
    fn shift_keeps_proportions_and_alt_scales_from_centre() {
        let m = xf_total(XfKind::Scale { hx: 1, hy: 1 }, B, pos2(100.0, 50.0), pos2(300.0, 60.0), true, false);
        let p = apply(&m, 100.0, 50.0);
        assert!((p.x / 100.0 - p.y / 50.0).abs() < 1e-4, "{p:?}");
        let m = xf_total(XfKind::Scale { hx: 1, hy: 0 }, B, pos2(100.0, 25.0), pos2(150.0, 25.0), false, true);
        assert_eq!(apply(&m, 50.0, 25.0), Vec2::new(50.0, 25.0), "centre stays put");
        assert_eq!(apply(&m, 100.0, 0.0).x, 150.0);
    }

    #[test]
    fn edge_scale_only_changes_one_axis_and_never_collapses() {
        let m = xf_total(XfKind::Scale { hx: 0, hy: 1 }, B, pos2(50.0, 50.0), pos2(50.0, 0.0), false, false);
        assert!(m.determinant().abs() > 0.0, "clamped above zero");
        assert_eq!(m.m[0], 1.0);
    }

    #[test]
    fn rotate_snaps_with_shift_and_keeps_centre() {
        let m = xf_total(XfKind::Rotate, B, pos2(110.0, 25.0), pos2(50.0, 100.0), true, false);
        let c = apply(&m, 50.0, 25.0);
        assert!(c.distance(Vec2::new(50.0, 25.0)) < 1e-3);
        let angle = m.decompose().rotation.to_degrees();
        assert!((angle / 15.0 - (angle / 15.0).round()).abs() < 1e-3, "{angle}");
    }

    #[test]
    fn zones() {
        let r = Rect::from_min_max(pos2(100.0, 100.0), pos2(300.0, 200.0));
        assert_eq!(xf_zone(r, pos2(300.0, 200.0)), Some(XfKind::Scale { hx: 1, hy: 1 }));
        assert_eq!(xf_zone(r, pos2(200.0, 100.0)), Some(XfKind::Scale { hx: 0, hy: -1 }));
        assert_eq!(xf_zone(r, pos2(312.0, 212.0)), Some(XfKind::Rotate));
        assert_eq!(xf_zone(r, pos2(200.0, 150.0)), Some(XfKind::Move));
        assert_eq!(xf_zone(r, pos2(500.0, 500.0)), None);
    }
}

#[cfg(test)]
mod pasteboard_tests {
    use super::*;
    use anicraft_engine::doc::Rgba;

    const BG: Rgba = Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };

    /// Every device pixel of `px` is in exactly one tile.
    fn covered_once(tiles: &[TileSpec], z: f32, px: [f32; 4]) {
        for y in (px[1] as i32..px[3] as i32).step_by(97) {
            for x in (px[0] as i32..px[2] as i32).step_by(89) {
                let (sx, sy) = ((x as f32 + 0.5) / z, (y as f32 + 0.5) / z);
                let n = tiles.iter().filter(|t| sx >= t.rect[0] && sx < t.rect[2] && sy >= t.rect[1] && sy < t.rect[3]).count();
                assert_eq!(n, 1, "pixel ({x}, {y}) is in {n} tiles");
            }
        }
    }

    #[test]
    fn the_pasteboard_gets_transparent_tiles_that_never_cross_the_stage_edge() {
        // A 1280 × 720 stage at 100 %, seen with 700 px of pasteboard on each side (#245).
        let (z, stage) = (1.0, (1280, 720));
        let view = [-700.0, -300.0, 1980.0, 1020.0];
        let tiles = tiles_for(z, stage, view, 0, Some(BG), true);
        covered_once(&tiles, z, view);
        for t in &tiles {
            let inside = t.rect[0] >= 0.0 && t.rect[1] >= 0.0 && t.rect[2] <= 1280.0 && t.rect[3] <= 720.0;
            let outside = t.rect[2] <= 0.0 || t.rect[0] >= 1280.0 || t.rect[3] <= 0.0 || t.rect[1] >= 720.0;
            assert!(inside ^ outside, "a tile straddles the stage edge: {:?}", t.rect);
            // The stage keeps its colour (and blends against it); the pasteboard is see-through.
            assert_eq!(t.view.background.is_some(), inside, "{:?}", t.rect);
            assert!(t.view.width <= TILE && t.view.height <= TILE);
        }
        // The stage's own tiles are the same with or without the pasteboard.
        let clipped = tiles_for(z, stage, view, 0, Some(BG), false);
        let rects = |ts: &[TileSpec], f: fn(&TileSpec) -> bool| {
            let mut v: Vec<[i32; 4]> = ts.iter().filter(|t| f(t)).map(|t| t.rect.map(|c| c as i32)).collect();
            v.sort_unstable();
            v
        };
        assert_eq!(rects(&tiles, |t| t.view.background.is_some()), rects(&clipped, |_| true));
        // Clipped (#251), nothing is drawn off the stage.
        assert!(clipped.iter().all(|t| t.rect[0] >= 0.0 && t.rect[1] >= 0.0 && t.rect[2] <= 1280.0 && t.rect[3] <= 720.0));
        covered_once(&clipped, z, [0.0, 0.0, 1280.0, 720.0]);
    }

    #[test]
    fn pasteboard_tiles_work_far_away_zoomed_and_stay_bounded() {
        // Zoomed in, looking only at the pasteboard, left of and below the stage.
        let tiles = tiles_for(4.0, (2000, 1000), [-5000.0, 1500.0, -3000.0, 2600.0], 1, Some(BG), true);
        assert!(!tiles.is_empty());
        assert!(tiles.iter().all(|t| t.view.background.is_none()));
        covered_once(&tiles, 4.0, [-5000.0, 1500.0, -3000.0, 2600.0]);
        // A wild view (zoomed far out on a huge canvas) still makes a bounded number of tiles.
        let n = tiles_for(1.0, (100, 100), [-1e12, -1e12, 1e12, 1e12], 1, Some(BG), true).len();
        assert!(n <= (2 * MAX_TILES_PER_AXIS + 1).pow(2), "{n} tiles");
        // Clipped and looking away from the stage: nothing to draw.
        assert!(tiles_for(1.0, (100, 100), [-900.0, -900.0, -10.0, -10.0], 0, Some(BG), false).is_empty());
    }
}
