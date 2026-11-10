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
}

impl Default for Cache {
    fn default() -> Self {
        Cache { tiles: Vec::new(), tick: 0, ms: 0.0, ready: (0, 0), canvas: Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)), service: None, shown_state: None, gpu: Default::default(), drawn_by: None }
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
                None => return Some("The GPU stage drew a picture the window can't show.".into()),
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

/// Tiles covering stage-device-pixel rect `[x0, x1) × [y0, y1)`, nearest to `centre` first.
fn tiles_for(z: f32, stage_px: (u32, u32), px: [f32; 4], margin: u32, bg: anicraft_engine::doc::Rgba) -> Vec<TileSpec> {
    let (sw, sh) = stage_px;
    if sw == 0 || sh == 0 {
        return Vec::new();
    }
    let t = TILE as f32;
    let last = (sw.div_ceil(TILE) - 1, sh.div_ceil(TILE) - 1);
    let tx0 = ((px[0] / t).floor().max(0.0) as u32).saturating_sub(margin);
    let ty0 = ((px[1] / t).floor().max(0.0) as u32).saturating_sub(margin);
    let tx1 = (((px[2] - 1.0).max(0.0) / t).floor() as u32 + margin).min(last.0);
    let ty1 = (((px[3] - 1.0).max(0.0) / t).floor() as u32 + margin).min(last.1);
    let (cx, cy) = ((px[0] + px[2]) / 2.0, (px[1] + px[3]) / 2.0);
    let mut out = Vec::new();
    for ty in ty0..=ty1 {
        for tx in tx0..=tx1 {
            let (ox, oy) = (tx * TILE, ty * TILE);
            let (w, h) = (TILE.min(sw - ox), TILE.min(sh - oy));
            let view = View { transform: Affine::translate(-(ox as f32), -(oy as f32)).mul(&Affine::scale(z, z)), width: w, height: h, background: Some(bg) };
            let rect = [ox as f32 / z, oy as f32 / z, (ox + w) as f32 / z, (oy + h) as f32 / z];
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
    let canvas = ui.max_rect();
    let now = ui.input(|i| i.time);
    crate::camera_tool::on_tool_change(app);
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

    let zoom = app.view.zoom;
    // Snap the stage origin to whole device pixels: tile edges then land exactly on screen
    // pixels, so texture filtering can't show seams between tiles.
    let raw = canvas.center() + app.view.pan - size * zoom * 0.5;
    let origin = pos2((raw.x * ppp).round() / ppp, (raw.y * ppp).round() / ppp);
    let to_screen = |p: Vec2| pos2(origin.x + p.x * zoom, origin.y + p.y * zoom);
    let to_stage = |p: Pos2| pos2((p.x - origin.x) / zoom, (p.y - origin.y) / zoom);
    let stage_rect = Rect::from_min_size(origin, size * zoom);
    let painter = ui.painter_at(canvas);
    // View > Preview Through Camera: the stage shows the movie's picture (ADR 0027). Artwork
    // tools pause meanwhile (what they'd edit is in stage space, not where it shows).
    let preview = crate::camera_tool::previewing(app);
    let cam_geo = crate::camera_tool::Geo { origin, zoom, stage_rect, preview };


    // ---- tools ----------------------------------------------------------------------------
    let (shift, alt, space, command) = ui.input(|i| (i.modifiers.shift, i.modifiers.alt, i.key_down(Key::Space), i.modifiers.command));
    let pointer = resp.hover_pos().or_else(|| resp.interact_pointer_pos());
    app.cursor_stage = pointer.filter(|p| canvas.contains(*p)).map(to_stage);
    let panning = app.tool == Tool::Hand || space || resp.dragged_by(egui::PointerButton::Middle);
    let paused = preview && !panning && !matches!(app.tool, Tool::Camera | Tool::Hand | Tool::Zoom);
    if paused && (resp.drag_started() || resp.clicked()) {
        app.status_ok("Previewing through the camera: artwork edits are paused. Turn off View › Preview Through Camera to edit.");
    }
    // Selection box in screen space (free transform handles live on it).
    let sel_bounds = bounds_arr(app.s.selection_bounds());
    let sel_rect = sel_bounds.map(|b| Rect::from_min_max(to_screen(Vec2::new(b[0], b[1])), to_screen(Vec2::new(b[2], b[3]))));
    // Shape hint markers take the pointer first with the selection tools (ADR 0023).
    let hint_tools = matches!(app.tool, Tool::Select | Tool::Transform | Tool::Subselect) && !panning && !preview;
    let hint_marks = if hint_tools { crate::shape_hints::markers(app) } else { Vec::new() };
    let hint_at = |p: Pos2| crate::shape_hints::hit(&hint_marks, p, to_screen);
    let hint_hot = pointer.and_then(hint_at);

    let xf = crate::pathtools::Xf { origin, zoom };
    // Pen pressure, when a tablet reports it (the Brush tool's width follows it).
    let pressure = crate::brushtool::frame_pressure(ui.ctx());
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
    if resp.hovered() {
        let icon = match app.tool {
            _ if panning => CursorIcon::Grab,
            _ if matches!(app.drag, Drag::Hint { .. } | Drag::CameraPan { .. }) => CursorIcon::Grabbing,
            _ if paused => CursorIcon::NotAllowed,
            Tool::Camera if app.drag == Drag::None && pointer.is_some_and(|p| crate::camera_tool::on_knob(app, &cam_geo, p)) => CursorIcon::Alias,
            Tool::Camera => CursorIcon::Grab,
            _ if hint_hot.is_some() && app.drag == Drag::None => CursorIcon::Grab,
            _ if matches!(app.drag, Drag::MotionPath { .. }) => CursorIcon::Grabbing,
            _ if mp_move || matches!(app.drag, Drag::MotionMove { .. }) => CursorIcon::Move,
            _ if mp_hot.is_some() => CursorIcon::Grab,
            _ if pen_hot.is_some() => CursorIcon::Copy,
            Tool::Width => crate::widthtool::cursor(app, pointer, xf),
            Tool::Transform => {
                let zone = match app.drag {
                    Drag::Transform { kind, .. } => Some(kind),
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
            Tool::Rect | Tool::Oval | Tool::Star | Tool::Line | Tool::Pen | Tool::Pencil | Tool::Brush | Tool::Eraser | Tool::PaintBucket | Tool::InkBottle | Tool::Eyedropper => CursorIcon::Crosshair,
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
        } else if let Some(m) = hint_at(press) {
            app.s.begin_gesture("Move Shape Hint");
            Drag::Hint { index: m.index, side: m.side }
        } else if mp_tools && command && crate::motion_path::move_hit(app, press, xf) {
            crate::motion_path::move_start(app, to_v(sp))
        } else if let Some((shown, grab)) = mp_tools.then(|| mp_grab(app, press)).flatten() {
            crate::motion_path::drag_start(app, shown, grab, to_v(sp))
        } else {
            match app.tool {
                Tool::Select => match app.s.hit_test(to_v(sp), 3.0 / zoom) {
                    Some((_, el)) => {
                        if !app.s.selection.contains(&el) {
                            app.run("element.pick", json!({"x": sp.x, "y": sp.y, "add": shift}));
                        }
                        app.s.begin_gesture("Move");
                        Drag::Move { last: sp }
                    }
                    None => Drag::Marquee { start: sp, cur: sp },
                },
                Tool::Transform => {
                    let zone = sel_rect.and_then(|r| xf_zone(r, press));
                    match (zone, sel_bounds) {
                        (Some(kind), Some(bounds)) => {
                            app.s.begin_gesture("Free Transform");
                            Drag::Transform { kind, start: sp, bounds, applied: Affine::IDENTITY }
                        }
                        // Not on the box: grab whatever is under the pointer and move it.
                        _ => match app.s.hit_test(to_v(sp), 3.0 / zoom) {
                            Some(_) => {
                                app.run("element.pick", json!({"x": sp.x, "y": sp.y, "add": shift}));
                                match bounds_arr(app.s.selection_bounds()) {
                                    Some(bounds) => {
                                        app.s.begin_gesture("Free Transform");
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
                Tool::Width => crate::widthtool::drag_start(app, press, xf),
                Tool::Pencil => {
                    app.pencil = vec![to_v(sp)];
                    Drag::Pencil
                }
                Tool::Brush => crate::brushtool::start(app, to_v(sp), pressure),
                // With the Faucet on, the eraser deletes on a click instead.
                Tool::Eraser if !app.eraser.faucet => crate::erasertool::start(app, to_v(sp)),
                Tool::Pen | Tool::Text | Tool::PaintBucket | Tool::InkBottle | Tool::Eyedropper | Tool::Eraser => Drag::None,
                Tool::Rect | Tool::Oval | Tool::Star | Tool::Line => Drag::Shape { start: sp, cur: sp },
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
    if resp.dragged() {
        let cur = resp.interact_pointer_pos().map(to_stage);
        match (&mut app.drag, cur) {
            (Drag::Pan, _) => app.view.pan += resp.drag_delta(),
            (Drag::Move { last }, Some(c)) => {
                let d = c - *last;
                if d.length_sq() > 0.0 {
                    *last = c;
                    app.run("element.translate", json!({"dx": d.x, "dy": d.y}));
                }
            }
            (Drag::Transform { kind, start, bounds, applied }, Some(c)) => {
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
            (Drag::Hint { index, side }, Some(c)) => {
                // Live: the hint snaps to the nearest edge as the pointer moves.
                let (index, side) = (*index, *side);
                app.run("tween.move_hint", json!({"hint": index, "side": side.key(), "x": c.x, "y": c.y}));
            }
            (Drag::Points { last }, Some(c)) => {
                let d = c - *last;
                if d.length_sq() > 0.0 {
                    *last = c;
                    crate::pathtools::move_points(app, Vec2::new(d.x, d.y));
                }
            }
            (Drag::PointMarquee { cur: m, .. }, Some(c)) => *m = c,
            (d @ Drag::Width { .. }, Some(c)) => {
                let d = *d;
                crate::widthtool::drag_to(app, d, to_v(c), alt, zoom);
            }
            (Drag::Pencil, Some(c)) => crate::pathtools::pencil_sample(app, to_v(c), xf),
            (Drag::Brush, Some(c)) => crate::brushtool::sample(app, to_v(c), pressure, xf),
            (Drag::Eraser, Some(c)) => crate::brushtool::sample(app, to_v(c), None, xf),
            (Drag::Marquee { cur: m, .. }, Some(c)) => *m = c,
            (Drag::Shape { start, cur: m }, Some(c)) => *m = constrain(app.tool, *start, c, shift),
            _ => {}
        }
    }

    if resp.drag_stopped() {
        match app.drag {
            Drag::Move { .. } => {
                app.drop_raw();
                app.s.end_gesture();
            }
            Drag::Transform { .. } | Drag::Points { .. } | Drag::Hint { .. } | Drag::MotionMove { .. } | Drag::CameraPan { .. } | Drag::CameraRotate { .. } => app.s.end_gesture(),
            Drag::Width { .. } => crate::widthtool::drag_end(app),
            Drag::MotionPath { .. } => crate::motion_path::drag_end(app),
            Drag::PointMarquee { start, cur } => crate::pathtools::marquee_points(app, start, cur, shift),
            Drag::Pencil => crate::pathtools::commit_pencil(app, xf),
            Drag::Brush => crate::brushtool::commit(app, xf),
            Drag::Eraser => crate::erasertool::commit(app, xf),
            Drag::Marquee { start, cur } => {
                let r = Rect::from_two_pos(start, cur);
                let ids: Vec<u64> = app
                    .s
                    .visible_elements()
                    .iter()
                    .filter(|e| {
                        let b = app.s.element_bounds(e.id);
                        !b.is_empty() && r.intersects(Rect::from_min_max(pos2(b.min.x, b.min.y), pos2(b.max.x, b.max.y)))
                    })
                    .map(|e| e.id.0)
                    .collect();
                let locked = app.s.active_layer().is_some_and(|l| l.locked);
                if !locked {
                    app.run("element.select", json!({"ids": ids, "add": shift}));
                }
            }
            Drag::Shape { start, cur } => create_shape(app, start, cur),
            Drag::Pan | Drag::None | Drag::CameraWheel { .. } => {}
        }
        app.drag = Drag::None;
    }

    if resp.clicked()
        && !paused
        && let Some(p) = resp.interact_pointer_pos() {
            let sp = to_stage(p);
            match app.tool {
                Tool::Subselect if !panning => {
                    if !crate::motion_path::click(app, p, xf, shift) {
                        crate::pathtools::subselect_click(app, p, shift, xf);
                    }
                }
                Tool::Width if !panning => crate::widthtool::click(app, p, xf),
                Tool::PaintBucket if !panning => {
                    // The gap size is set in screen points, so it feels the same at any zoom.
                    let gap = app.bucket_gap.screen_px() / zoom.max(1e-3);
                    if let Some(v) = app.run("paint.bucket", json!({"x": sp.x, "y": sp.y, "gap": gap})) {
                        app.status_ok(if v.get("recolored").is_some() { "Recoloured the fill. Ctrl+Z to undo." } else { "Filled the area. Ctrl+Z to undo." });
                    }
                }
                Tool::Brush if !panning => {
                    // A click paints a dab.
                    crate::brushtool::start(app, to_v(sp), pressure);
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
                // A click on a hint marker leaves the selection alone.
                Tool::Select | Tool::Transform if !panning && hint_at(p).is_none() => {
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
        let under = app.s.hit_test(to_v(sp), 3.0 / zoom).map(|(_, e)| e);
        // On the motion path, the menu is for its points: the selection stays.
        let on_path = !panning && crate::motion_path::secondary_click(app, p, xf);
        if !on_path && under.is_none_or(|e| !app.s.selection.contains(&e)) {
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
        // Raw artwork (ADR 0031): a stroke and the strokes joined to it, or a fill and its outline.
        if !open_text_at(app, sp, 3.0 / zoom.max(1e-3)) {
            app.run("element.select_connected", json!({"x": sp.x, "y": sp.y, "add": shift, "slop": 3.0 / zoom.max(1e-3)}));
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
            ui.menu_button("Transform", |ui| {
                for id in ["modify.rotate_cw", "modify.rotate_ccw", "modify.flip_h", "modify.flip_v"] {
                    crate::menus::cmd_item(app, ui, id);
                }
            });
            ui.menu_button("Align", |ui| {
                for id in ["align.left", "align.hcenter", "align.right", "align.top", "align.vcenter", "align.bottom", "align.distribute_h", "align.distribute_v"] {
                    crate::menus::cmd_item(app, ui, id);
                }
            });
            ui.separator();
            for id in ["modify.convert_to_symbol", "modify.break_apart"] {
                crate::menus::cmd_item(app, ui, id);
            }
            ui.menu_button("Arrange", |ui| {
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
    let vis = stage_rect.intersect(canvas);
    if vis.width() >= 1.0 && vis.height() >= 1.0 {
        if app.stage_cache.service.is_none() {
            let ctx = ui.ctx().clone();
            app.stage_cache.service = Some(RenderService::new(DEFAULT_CACHE_BYTES, Some(Box::new(move || ctx.request_repaint()))));
        }
        if let Some(svc) = &app.stage_cache.service {
            app.stage_cache.gpu.sync(app.prefs.stage_renderer, svc, ui.ctx());
        }
        // Jobs are keyed by who draws them, so a renderer switch re-renders (old tiles stand in).
        let raster = app.stage_cache.service.as_ref().map_or(0, RenderService::raster_id);
        // Tiles on a device-pixel grid at this zoom (ADR 0007).
        let z = zoom * ppp;
        let bg = app.s.doc.stage.background;
        let stage_px = ((size.x * z).ceil().max(1.0) as u32, (size.y * z).ceil().max(1.0) as u32);
        let px = [(vis.min.x - origin.x) * ppp, (vis.min.y - origin.y) * ppp, (vis.max.x - origin.x) * ppp, (vis.max.y - origin.y) * ppp];
        let visible = tiles_for(z, stage_px, px, 0, bg);
        let onion = app.onion && !app.playing;
        let state = app.s.state;
        // Render jobs take movie frames: the current scene starts at `start`.
        let start = app.s.doc.scene_start(app.s.scene);
        let frame = app.s.frame;
        let jobs: Vec<(TileSpec, FrameJob)> = visible.into_iter().map(|t| { let j = make_job_for(raster, state, &app.s.doc, start + frame, t.view, onion).through_camera(preview); (t, j) }).collect();
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
                app.status_err(e);
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
            if app.playing {
                let n = app.s.frame_count().max(1);
                for d in 1..=LOOKAHEAD.min(n - 1) {
                    let f = (frame + d) % n;
                    ahead.extend(jobs.iter().map(|(t, _)| make_job_for(raster, state, &app.s.doc, start + f, t.view, false).through_camera(preview)));
                }
            } else {
                ahead.extend(tiles_for(z, stage_px, px, 1, bg).into_iter().filter(|t| !jobs.iter().any(|(v, _)| v.rect == t.rect)).map(|t| make_job_for(raster, state, &app.s.doc, start + frame, t.view, onion).through_camera(preview)));
            }
            svc.prefetch(ahead);
        }

        // Draw: tiles from earlier zooms/states that cover the view go first (scaled, as a
        // placeholder), then the current tiles on top.
        let view_rect = [(vis.min.x - origin.x) / zoom, (vis.min.y - origin.y) / zoom, (vis.max.x - origin.x) / zoom, (vis.max.y - origin.y) / zoom];
        let overlaps = |r: &[f32; 4]| r[0] < view_rect[2] && r[2] > view_rect[0] && r[1] < view_rect[3] && r[3] > view_rect[1];
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        let mut stale: Vec<&TileTex> = app.stage_cache.tiles.iter().filter(|t| !keys.contains(&t.key) && overlaps(&t.rect)).collect();
        if ready < keys.len() {
            stale.sort_by_key(|t| t.used);
            for t in stale.iter().rev().take(96).rev() {
                let r = Rect::from_min_max(to_screen(Vec2::new(t.rect[0], t.rect[1])), to_screen(Vec2::new(t.rect[2], t.rect[3])));
                painter.image(t.tex.id(), r, uv, Color32::WHITE);
            }
        }
        for t in app.stage_cache.tiles.iter().filter(|t| keys.contains(&t.key)) {
            let r = Rect::from_min_max(to_screen(Vec2::new(t.rect[0], t.rect[1])), to_screen(Vec2::new(t.rect[2], t.rect[3])));
            painter.image(t.tex.id(), r, uv, Color32::WHITE);
        }

        // While a background render catches up with a drag, outline where the selection is now,
        // so the artwork never seems to lag the pointer.
        if ready < keys.len() && !preview && matches!(app.drag, Drag::Move { .. } | Drag::Transform { .. }) {
            let to_px = Affine::translate(origin.x, origin.y).mul(&Affine::scale(zoom, zoom));
            let dl = app.s.display_list();
            let stroke = Stroke::new(1.5, T.selection);
            for item in dl.items.iter().filter(|i| app.s.selection.contains(&i.element)).take(2_000) {
                for pl in item.path.flatten(&to_px.mul(&item.transform), 0.5) {
                    let pts: Vec<Pos2> = pl.points.iter().map(|p| pos2(p.x, p.y)).collect();
                    painter.add(if pl.closed { egui::Shape::closed_line(pts, stroke) } else { egui::Shape::line(pts, stroke) });
                }
            }
        }
    }
    painter.rect_stroke(stage_rect, 0.0, Stroke::new(1.0, Color32::from_black_alpha(160)), StrokeKind::Outside);

    // ---- overlays -------------------------------------------------------------------------
    // The camera frame (or, previewing, the Camera tool's handle) comes first, under the
    // selection.
    crate::camera_tool::draw(app, &painter, &cam_geo, pointer);
    if preview {
        // The movie's picture: stage-space overlays don't belong on it.
        painter.text(stage_rect.left_top() + vec2(0.0, -6.0), egui::Align2::LEFT_BOTTOM, "Camera view", crate::theme::font(crate::theme::Type::Caption), T.text_dim);
        let badge = format!("{:.0}%", zoom * 100.0);
        painter.text(canvas.right_bottom() + vec2(-10.0, -8.0), egui::Align2::RIGHT_BOTTOM, badge, crate::theme::font(crate::theme::Type::Data), T.text_dim);
        crate::a11y::surface(ui, &resp, &format!("Stage · previewing through the camera · {} tool · frame {}", app.tool.label(), app.s.frame + 1));
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
            let Some(layer) = app.s.doc.timeline(app.s.scene).layer(item.layer).filter(|l| item.guide || !l.locked) else { continue };
            let stroke = Stroke::new(if item.guide { 2.0 } else { 1.5 }, to_color32(layer.color));
            for pl in item.path.flatten(&to_px.mul(&item.transform), 0.5) {
                let pts: Vec<Pos2> = pl.points.iter().map(|p| pos2(p.x, p.y)).collect();
                painter.add(if pl.closed { egui::Shape::closed_line(pts, stroke) } else { egui::Shape::line(pts, stroke) });
            }
        }
    }
    let sel_stroke = Stroke::new(1.0, T.selection);
    let raw = raw_ids(app);
    if app.tool == Tool::Select && app.drag == Drag::None
        && app.palette.is_none()
        && !app.modal_open()
        && let Some(sp) = app.cursor_stage
            && let Some((_, el)) = app.s.hit_test(to_v(sp), 3.0 / zoom)
                && !app.s.selection.contains(&el) && !raw.contains(&el) {
                    let b = app.s.element_bounds(el);
                    if !b.is_empty() {
                        let r = Rect::from_min_max(to_screen(b.min), to_screen(b.max));
                        painter.rect_stroke(r, 0.0, Stroke::new(1.0, T.selection.gamma_multiply(0.55)), StrokeKind::Middle);
                    }
                }
    let transforming = app.tool == Tool::Transform;
    // Selected raw shapes (ADR 0031) show Animate's dot pattern over themselves, with no box.
    if app.tool == Tool::Select {
        let to_px = Affine::translate(origin.x, origin.y).mul(&Affine::scale(zoom, zoom));
        let mut mesh = egui::Mesh::default();
        let mut budget = MAX_DOTS;
        for item in app.s.display_list().items.iter().filter(|i| app.s.selection.contains(&i.element) && raw.contains(&i.element)) {
            selection_dots(&mut mesh, item, &to_px.mul(&item.transform), canvas, &mut budget);
        }
        painter.add(egui::Shape::mesh(mesh));
    }
    for id in app.s.selection.clone() {
        let b = app.s.element_bounds(id);
        if b.is_empty() {
            continue;
        }
        let r = Rect::from_min_max(to_screen(b.min), to_screen(b.max)).expand(1.0);
        if matches!(app.tool, Tool::Subselect | Tool::Width) {
            // Subselection and Width show outlines and points instead (below).
            continue;
        }
        if app.tool == Tool::Select && raw.contains(&id) {
            continue; // dots above
        }
        if transforming {
            // Free transform shows one box for the whole selection (below); members get a hint.
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, T.selection.gamma_multiply(0.45)), StrokeKind::Middle);
            continue;
        }
        painter.rect_stroke(r, 0.0, sel_stroke, StrokeKind::Middle);
        for c in [r.left_top(), r.right_top(), r.left_bottom(), r.right_bottom(), r.center_top(), r.center_bottom(), r.left_center(), r.right_center()] {
            let h = Rect::from_center_size(c, vec2(6.0, 6.0));
            painter.rect_filled(h, 0.0, Color32::WHITE);
            painter.rect_stroke(h, 0.0, sel_stroke, StrokeKind::Middle);
        }
        // registration point
        if let Some(e) = app.s.visible_elements().iter().find(|e| e.id == id) {
            let o = to_screen(e.transform.translation());
            painter.circle(o, 3.5, Color32::WHITE, Stroke::new(1.0, Color32::BLACK));
        }
    }
    // Free transform box: recomputed every frame so it follows the artwork while you drag.
    if transforming && let Some(b) = bounds_arr(app.s.selection_bounds()) {
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
            painter.rect_stroke(r, 0.0, sel_stroke, StrokeKind::Middle);
        }
        Drag::Shape { start, cur } => draw_preview(app, &painter, to_screen(Vec2::new(start.x, start.y)), to_screen(Vec2::new(cur.x, cur.y)), zoom),
        _ => {}
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

    // zoom badge
    let badge = format!("{:.0}%", zoom * 100.0);
    painter.text(canvas.right_bottom() + vec2(-10.0, -8.0), egui::Align2::RIGHT_BOTTOM, badge, crate::theme::font(crate::theme::Type::Data), T.text_dim);
    // Last, so the focus ring is drawn over the artwork.
    let selected = app.s.selection.len();
    let what = match selected {
        0 => String::new(),
        1 => " · 1 element selected".to_string(),
        n => format!(" · {n} elements selected"),
    };
    crate::a11y::surface(ui, &resp, &format!("Stage · {} tool · frame {}{what}", app.tool.label(), app.s.frame + 1));
}

// ----- raw shapes (ADR 0031) ---------------------------------------------------------------------

/// The raw shapes (parts of the layer's merged drawing) on the active layer.
fn raw_ids(app: &App) -> std::collections::HashSet<anicraft_engine::doc::ElementId> {
    app.s.visible_elements().iter().filter(|e| matches!(&e.kind, anicraft_engine::doc::ElementKind::Shape(sh) if sh.merge)).map(|e| e.id).collect()
}

/// Most selection dots drawn in a frame.
const MAX_DOTS: usize = 60_000;

/// Animate's selection look for raw shapes: fine dots over the selected fill or stroke, light and
/// dark in a checker so they show on any colour. They sit on a fixed screen grid, so they don't
/// crawl when the view pans; `m` maps the item's path to the screen.
fn selection_dots(mesh: &mut egui::Mesh, item: &anicraft_engine::doc::DisplayItem, m: &Affine, clip: Rect, budget: &mut usize) {
    const GAP: f32 = 3.0;
    let mut dot = |p: Pos2| {
        if clip.contains(p) && *budget > 0 {
            *budget -= 1;
            let dark = ((p.x / GAP).round() as i64 + (p.y / GAP).round() as i64).rem_euclid(2) == 0;
            mesh.add_colored_rect(Rect::from_center_size(p, vec2(1.5, 1.5)), if dark { T.selection } else { Color32::WHITE });
        }
        *budget > 0
    };
    let lines = item.path.flatten(m, 0.5);
    if item.fill.is_some() {
        let b = item.path.bounds().transformed(m);
        let mut y = (b.min.y.max(clip.top()) / GAP).ceil() * GAP;
        let bottom = b.max.y.min(clip.bottom());
        let mut xs: Vec<f32> = Vec::new();
        while y <= bottom {
            // Where this row crosses the outline; pairs of crossings are inside (even-odd: raw
            // fills never overlap themselves).
            xs.clear();
            for pl in &lines {
                let n = pl.points.len();
                for i in 0..n {
                    let (a, c) = (pl.points[i], pl.points[(i + 1) % n]);
                    if (a.y > y) != (c.y > y) {
                        xs.push(a.x + (y - a.y) * (c.x - a.x) / (c.y - a.y));
                    }
                }
            }
            xs.sort_by(f32::total_cmp);
            for [x0, x1] in xs.as_chunks::<2>().0 {
                let mut x = (x0.max(clip.left()) / GAP).ceil() * GAP;
                while x <= x1.min(clip.right()) {
                    if !dot(pos2(x, y)) {
                        return;
                    }
                    x += GAP;
                }
            }
            y += GAP;
        }
    }
    if let Some(st) = &item.stroke {
        // Along the stroke, in rows across its width; segments off the screen are skipped.
        let half = (st.reach() * m.mean_scale()).max(0.5);
        let rows = (half / GAP).floor() as i32;
        let near = clip.expand(half + GAP);
        for pl in &lines {
            let n = pl.points.len();
            let segs = if pl.closed { n } else { n.saturating_sub(1) };
            let mut carry = 0.0;
            for i in 0..segs {
                let (a, c) = (pl.points[i], pl.points[(i + 1) % n]);
                let d = c - a;
                let len = d.length();
                if len <= 1e-6 {
                    continue;
                }
                if !near.intersects(Rect::from_two_pos(pos2(a.x, a.y), pos2(c.x, c.y))) {
                    carry = (carry - len).rem_euclid(GAP);
                    continue;
                }
                let (dir, normal) = (d * (1.0 / len), d.perp() * (1.0 / len));
                let mut t = carry;
                while t < len {
                    let p = a + dir * t;
                    for k in -rows..=rows {
                        let q = p + normal * (k as f32 * GAP);
                        if !dot(pos2(q.x, q.y)) {
                            return;
                        }
                    }
                    t += GAP;
                }
                carry = t - len;
            }
        }
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
            let r = ui.add(egui::TextEdit::multiline(&mut te.text).font(crate::theme::stage_text_font(size)).desired_rows(1).desired_width(260.0f32.max(size * 8.0)).hint_text("Type text")).named("Text on the stage");
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
            app.run("draw.rect", json!({"x": start.x, "y": start.y, "w": d.x, "h": d.y, "radius": app.rect_radius}));
        }
        Tool::Oval => {
            app.run("draw.ellipse", json!({"x": start.x, "y": start.y, "w": d.x, "h": d.y}));
        }
        Tool::Star => {
            let r = d.length();
            app.run("draw.star", json!({"cx": start.x, "cy": start.y, "outer": r, "inner": r * 0.45, "points": app.star_points}));
        }
        Tool::Line => {
            app.run("draw.line", json!({"x0": start.x, "y0": start.y, "x1": cur.x, "y1": cur.y}));
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
            let rr = (b - a).length();
            let n = app.star_points.clamp(3, 64) as usize;
            let pts: Vec<Pos2> = (0..n * 2)
                .map(|i| {
                    let r = if i % 2 == 0 { rr } else { rr * 0.45 };
                    let t = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / n as f32;
                    a + vec2(r * t.cos(), r * t.sin())
                })
                .collect();
            painter.add(egui::Shape::closed_line(pts, stroke));
        }
        Tool::Line => {
            painter.line_segment([a, b], stroke);
        }
        _ => {}
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
