//! The Camera tool and the camera frame on the stage (ADR 0027).
//!
//! While editing, the stage is in stage space and the camera shows as its frame: an outline in
//! the camera layer's colour of what the movie will show, with the centre marked. With the
//! Camera tool, a drag pans the camera, the round handle above the frame turns it, and the
//! wheel zooms it; each drag (or burst of wheel turns) is one undo step. With View › Preview
//! Through Camera the stage shows the movie's picture instead: there a drag moves the picture
//! under the pointer (the camera the other way), as panning a view does, and the Selection
//! tool picks and drags artwork where the movie shows it (ADR 0082): the press picks through
//! the camera (`element.pick {view: camera}`), and each move is mapped back to the stage
//! through the inverse of the picked layer's view (camera and depth), so the artwork follows
//! the pointer on the picture.

use crate::app::{App, Drag, Tool};
use crate::theme::to_color32;
use anicraft_engine::doc::Camera;
use anicraft_engine::geom::{Affine, Vec2};
use egui::{Color32, Pos2, Rect, Stroke, pos2, vec2};
use serde_json::json;

/// How far the rotate handle sits outside the frame, and its grab radius (screen points).
const KNOB_OFFSET: f32 = 26.0;
const KNOB_GRAB: f32 = 12.0;
/// How long after the last wheel turn a zoom burst ends (seconds).
const WHEEL_BURST: f64 = 0.45;

/// The stage on screen: `screen = origin + stage · zoom`; `preview` when it shows the movie.
#[derive(Clone, Copy)]
pub struct Geo {
    pub origin: Pos2,
    pub zoom: f32,
    pub stage_rect: Rect,
    pub preview: bool,
}

impl Geo {
    fn to_screen(self, p: Vec2) -> Pos2 {
        pos2(self.origin.x + p.x * self.zoom, self.origin.y + p.y * self.zoom)
    }
}

/// The scene's camera at the playhead (the rest camera on a blank camera keyframe).
pub fn camera(app: &App) -> Option<Camera> {
    app.s.timeline().camera_state_at(app.s.frame, &app.s.doc.stage)
}

/// Whether the stage shows the movie through the camera this frame.
pub fn previewing(app: &App) -> bool {
    app.camera_preview && app.s.timeline().camera_layer().is_some()
}

/// Picking the Camera tool adds a camera when the scene has none (as Animate does), once per
/// pick: undoing the add leaves the tool in hand without adding it again.
pub fn on_tool_change(app: &mut App) {
    if app.tool == Tool::Camera && app.last_tool != Tool::Camera && app.s.timeline().camera_layer().is_none() && app.run("camera.add", json!({})).is_some() {
        app.status_ok(tr!("Added a camera. Drag on the stage to pan it, turn it with the round handle, zoom with the wheel. {undo} to undo.", undo = app.undo_key()));
    }
    app.last_tool = app.tool;
}

/// The camera's centre, rotate handle and the middle of the frame's top edge, on screen.
fn handles(cam: &Camera, app: &App, g: &Geo) -> (Pos2, Pos2, Pos2) {
    if g.preview {
        let top = g.stage_rect.center_top();
        return (g.stage_rect.center(), top - vec2(0.0, KNOB_OFFSET), top);
    }
    let f = cam.frame(&app.s.doc.stage);
    let centre = g.to_screen(cam.center);
    let top = g.to_screen((f[0] + f[1]) * 0.5);
    let up = top - centre;
    let dir = if up.length() > 1e-3 { up / up.length() } else { vec2(0.0, -1.0) };
    (centre, top + dir * KNOB_OFFSET, top)
}

/// Whether screen point `p` is on the rotate handle.
pub fn on_knob(app: &App, g: &Geo, p: Pos2) -> bool {
    camera(app).is_some_and(|c| handles(&c, app, g).1.distance(p) <= KNOB_GRAB)
}

/// A Camera tool drag starting at screen point `press`.
pub fn drag_start(app: &mut App, g: &Geo, press: Pos2) -> Drag {
    let Some(cam) = camera(app) else { return Drag::None };
    let (centre, _, _) = handles(&cam, app, g);
    if on_knob(app, g, press) {
        // Undo labels stay English in the history and are translated where they show.
        app.s.begin_gesture(tr_noop!("Rotate Camera"));
        let d = press - centre;
        Drag::CameraRotate { last: d.y.atan2(d.x) }
    } else {
        app.s.begin_gesture(tr_noop!("Move Camera"));
        Drag::CameraPan { last: press }
    }
}

/// Continues a Camera tool drag to screen point `cur`.
pub fn drag_to(app: &mut App, g: &Geo, drag: &mut Drag, cur: Pos2) {
    let Some(cam) = camera(app) else { return };
    match drag {
        Drag::CameraPan { last } => {
            let d = (cur - *last) / g.zoom.max(1e-3);
            if d.length_sq() <= 0.0 {
                return;
            }
            *last = cur;
            // Editing: the frame follows the pointer. Previewing: the picture does, so the
            // camera moves the other way (turned and zoomed as the camera is).
            let v = if g.preview { Affine::rotate(cam.rotation).apply_vector(Vec2::new(-d.x, -d.y)) * (1.0 / cam.zoom) } else { Vec2::new(d.x, d.y) };
            app.run("camera.set", json!({"dx": v.x, "dy": v.y}));
        }
        Drag::CameraRotate { last } => {
            let (centre, _, _) = handles(&cam, app, g);
            let d = cur - centre;
            if d.length() < 4.0 {
                return;
            }
            let a = d.y.atan2(d.x);
            let mut turn = a - *last;
            // The short way round, so crossing ±180° doesn't spin the camera.
            turn = (turn + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
            *last = a;
            let deg = if g.preview { -turn.to_degrees() } else { turn.to_degrees() };
            if deg.abs() > 1e-4 {
                app.run("camera.set", json!({"rotate_by": deg}));
            }
        }
        _ => {}
    }
}

/// One wheel turn with the Camera tool: zooms the camera; turns close together are one step.
pub fn wheel(app: &mut App, scroll_y: f32, now: f64) {
    if camera(app).is_none() || scroll_y == 0.0 {
        return;
    }
    if !matches!(app.drag, Drag::CameraWheel { .. }) {
        app.s.begin_gesture(tr_noop!("Zoom Camera"));
    }
    app.drag = Drag::CameraWheel { until: now + WHEEL_BURST };
    let by = (scroll_y * 0.0025).clamp(-0.5, 0.5).exp();
    app.run("camera.set", json!({"zoom_by": by}));
}

/// Ends a wheel burst once the wheel has been still for a moment (the gesture then closes).
pub fn tick(app: &mut App, ctx: &egui::Context, now: f64) {
    if let Drag::CameraWheel { until } = app.drag {
        if now >= until {
            app.drag = Drag::None;
        } else {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64((until - now).max(0.01)));
        }
    }
}

/// The camera frame (editing) or the camera-view marks (previewing), and the Camera tool's
/// handles.
pub fn draw(app: &App, painter: &egui::Painter, g: &Geo, hot: Option<Pos2>) {
    let tl = app.s.timeline();
    let Some(layer) = tl.camera_layer() else { return };
    let Some(cam) = camera(app) else { return };
    let color = to_color32(layer.color);
    let color = if layer.visible { color } else { color.gamma_multiply(0.45) };
    let tool = app.tool == Tool::Camera;
    if !g.preview {
        let f = cam.frame(&app.s.doc.stage).map(|p| g.to_screen(p));
        let stroke = Stroke::new(if tool { 2.0 } else { 1.5 }, color);
        painter.add(egui::Shape::closed_line(f.to_vec(), stroke));
        // The centre, as a small cross.
        let c = g.to_screen(cam.center);
        painter.line_segment([c - vec2(6.0, 0.0), c + vec2(6.0, 0.0)], stroke);
        painter.line_segment([c - vec2(0.0, 6.0), c + vec2(0.0, 6.0)], stroke);
    }
    if tool {
        let (_, knob, edge) = handles(&cam, app, g);
        painter.line_segment([edge, knob], Stroke::new(1.5, color));
        let hot = hot.is_some_and(|p| p.distance(knob) <= KNOB_GRAB);
        painter.circle(knob, if hot { 6.5 } else { 5.5 }, Color32::WHITE, Stroke::new(1.5, color));
    }
}

/// A Selection tool press on the camera preview at picture point `at`: picks what the movie
/// shows there and, on artwork, starts moving the selection (one undo step).
pub fn select_press(app: &mut App, at: Vec2, add: bool) -> Drag {
    app.bone_sel = None;
    let hit = app.run("element.pick", json!({"x": at.x, "y": at.y, "add": add, "view": "camera"}));
    if hit.is_none_or(|v| v["element"].is_null()) || app.s.selection.is_empty() {
        return Drag::None;
    }
    app.s.begin_gesture(tr_noop!("Move"));
    Drag::CameraMove { last: at }
}

/// Continues a move on the preview to picture point `cur`: the picture's step mapped back to
/// the stage through the active layer's view, so the artwork stays under the pointer.
pub fn move_to(app: &mut App, last: &mut Vec2, cur: Vec2) {
    let d = cur - *last;
    if d.length() <= 0.0 {
        return;
    }
    *last = cur;
    let back = app.s.camera_view(app.s.layer).and_then(|v| v.inverse()).unwrap_or(Affine::IDENTITY);
    let s = back.apply_vector(d);
    if s.x.is_finite() && s.y.is_finite() {
        app.run("element.translate", json!({"dx": s.x, "dy": s.y}));
    }
}
