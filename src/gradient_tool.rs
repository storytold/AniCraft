//! Gradient Transform tool (F): where a gradient or bitmap fill lies (ADR 0074).
//!
//! Click a shape with a gradient or bitmap fill to show its handles, drawn as Animate draws
//! them on the fill's frame (the box −1..1 mapped onto the fill, `Paint::frame`):
//!
//! * linear: the two edges of the ramp, the centre (move), a square on the right edge (width
//!   along the ramp) and a knob on its corner (rotate);
//! * radial: the ring, the centre (move), the focal point (a triangle that slides along the
//!   gradient's axis), a square on the ring (width: an ellipse), a circle below it (size) and a
//!   knob below that (rotate);
//! * bitmap: the box of one copy of the image, the centre, squares for width and height,
//!   diamonds to skew each way, a circle to scale and a knob to rotate.
//!
//! Each drag is one undo step: `paint.transform_fill` with the frame worked out from the frame
//! the drag started on, so agents and the CLI move fills exactly the same way.

use crate::app::{App, Drag, Tool};
use crate::pathtools::Xf;
use crate::theme::T;
use anicraft_engine::doc::{Element, ElementId, ElementKind, MAX_FOCAL, Paint};
use anicraft_engine::geom::{Affine, Vec2};
use egui::{Color32, CursorIcon, Pos2, Stroke as Line, StrokeKind, pos2, vec2};
use serde_json::json;

/// Pick radius for handles, in screen points.
const PICK: f32 = 7.0;
/// How far round the ring (radians, clockwise on screen) the size and rotate handles sit from
/// the width handle on a radial gradient.
const SIZE_AT: f32 = 0.45;
const TURN_AT: f32 = 0.9;

/// What a handle does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grip {
    Move,
    Focal,
    Width,
    Height,
    Size,
    Rotate,
    SkewX,
    SkewY,
}

/// The kind of fill the handles are for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Linear,
    Radial,
    Bitmap,
}

/// The fill being edited: a shape on the active layer with a gradient or bitmap fill.
struct Target {
    id: ElementId,
    kind: Kind,
    /// The fill's frame in the space being edited.
    frame: Affine,
    focal: f32,
}

fn target_of(app: &App, e: &Element) -> Option<Target> {
    let ElementKind::Shape(sh) = &e.kind else { return None };
    let (kind, focal) = match sh.fill.as_ref()? {
        Paint::Linear { .. } => (Kind::Linear, 0.0),
        Paint::Radial { focal, .. } => (Kind::Radial, *focal),
        Paint::Bitmap { .. } => (Kind::Bitmap, 0.0),
        Paint::Solid(_) => return None,
    };
    let frame = anicraft_engine::cmds_paint::fill_frame(&app.s.doc, e)?;
    frame.inverse()?;
    Some(Target { id: e.id, kind, frame, focal })
}

/// The selected shape whose fill the tool edits (the topmost with a gradient or bitmap).
fn target(app: &App) -> Option<Target> {
    app.s.selected_elements().into_iter().filter(|(l, _)| *l == app.s.layer).rev().find_map(|(_, e)| target_of(app, &e))
}

fn target_by_id(app: &App, id: ElementId) -> Option<Target> {
    app.s.visible_elements().into_iter().find(|e| e.id == id).and_then(|e| target_of(app, &e))
}

/// Where each handle sits, in the frame's own space.
fn handles(t: &Target) -> Vec<(Grip, Vec2)> {
    let ring = |a: f32| Vec2::new(a.cos(), a.sin());
    match t.kind {
        Kind::Linear => vec![(Grip::Move, Vec2::ZERO), (Grip::Width, Vec2::new(1.0, 0.0)), (Grip::Rotate, Vec2::new(1.0, -1.0))],
        Kind::Radial => vec![(Grip::Move, Vec2::ZERO), (Grip::Focal, Vec2::new(t.focal, 0.0)), (Grip::Width, Vec2::new(1.0, 0.0)), (Grip::Size, ring(SIZE_AT)), (Grip::Rotate, ring(TURN_AT))],
        Kind::Bitmap => vec![
            (Grip::Move, Vec2::ZERO),
            (Grip::Width, Vec2::new(-1.0, 0.0)),
            (Grip::Height, Vec2::new(0.0, 1.0)),
            (Grip::SkewX, Vec2::new(0.0, -1.0)),
            (Grip::SkewY, Vec2::new(1.0, 0.0)),
            (Grip::Size, Vec2::new(1.0, 1.0)),
            (Grip::Rotate, Vec2::new(1.0, -1.0)),
        ],
    }
}

/// The handle under screen point `at` (the focal point and centre last, so a handle on top of
/// them wins only where it is drawn).
fn hit(t: &Target, at: Pos2, xf: Xf) -> Option<Grip> {
    let hs = handles(t);
    let near = |g: Grip| hs.iter().find(|(h, _)| *h == g).map(|(_, u)| xf.to_screen(t.frame.apply(*u)).distance(at));
    // The rest before the centre and focal point, which may sit under them.
    let order = [Grip::Width, Grip::Height, Grip::Size, Grip::Rotate, Grip::SkewX, Grip::SkewY, Grip::Focal, Grip::Move];
    order.into_iter().filter_map(|g| near(g).map(|d| (g, d))).filter(|(_, d)| *d <= PICK).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(g, _)| g)
}

/// Press with the tool: grab a handle of the selected fill, else select the filled shape under
/// the pointer (its handles show), else deselect.
pub fn drag_start(app: &mut App, press: Pos2, xf: Xf) -> Drag {
    if let Some(t) = target(app)
        && let Some(grip) = hit(&t, press, xf)
    {
        // Undo labels stay English in the history and are translated where they show.
        app.s.begin_gesture(tr_noop!("Gradient Transform"));
        let at = xf.to_stage(press);
        return Drag::Gradient { id: t.id, grip, frame: t.frame, press: pos2(at.x, at.y) };
    }
    pick(app, press, xf);
    Drag::None
}

/// Click (or a press that grabbed nothing): select the filled shape under the pointer.
pub fn pick(app: &mut App, at: Pos2, xf: Xf) {
    let p = xf.to_stage(at);
    if app.s.hit_test(p, 3.0 * xf.unit()).is_none() {
        app.run("edit.deselect", json!({}));
        return;
    }
    app.run("element.pick", json!({"x": p.x, "y": p.y}));
    if target(app).is_none() && !app.s.selection.is_empty() {
        app.status_ok(tr!("That fill is a solid colour. Click its Fill swatch and choose Linear, Radial or Bitmap to give it a gradient."));
    }
}

/// The frame a drag of `grip` from frame `f0` (pressed at stage point `p0`) asks for with the
/// pointer at stage point `p` (`snap`: Shift, rotation in 15° steps); for the focal point of a
/// `radial` gradient, the new focal ratio instead. `None`: nothing to do.
pub fn dragged_frame(radial: bool, grip: Grip, f0: &Affine, p0: Vec2, p: Vec2, snap: bool) -> Option<(Affine, Option<f32>)> {
    let inv = f0.inverse()?;
    let u = inv.apply(p);
    let c = f0.apply(Vec2::ZERO);
    // A scale factor held clear of zero (the frame never collapses), keeping its sign.
    let factor = |v: f32, h: f32| {
        let k = if h.abs() > 1e-6 { v / h } else { 1.0 };
        if k.abs() < 0.02 { 0.02f32.copysign(if k == 0.0 { 1.0 } else { k }) } else { k.clamp(-1e4, 1e4) }
    };
    let f = match grip {
        Grip::Move => Affine::translate(p.x - p0.x, p.y - p0.y).mul(f0),
        Grip::Focal => return radial.then(|| (*f0, Some(u.x.clamp(-MAX_FOCAL, MAX_FOCAL)))),
        // Width: along the frame's own x (the linear ramp's length, a radial ring's width; on a
        // bitmap the handle is on the left).
        Grip::Width => {
            let h = inv.apply(p0).x.signum();
            let h = if h == 0.0 { 1.0 } else { h };
            f0.mul(&Affine::scale(factor(u.x, h), 1.0))
        }
        Grip::Height => f0.mul(&Affine::scale(1.0, factor(u.y, 1.0))),
        Grip::Size => {
            let u0 = inv.apply(p0);
            let k = factor(u.length(), u0.length().max(1e-3)).abs();
            f0.mul(&Affine::scale(k, k))
        }
        Grip::Rotate => {
            let (a, b) = (p0 - c, p - c);
            let mut turn = a.cross(b).atan2(a.dot(b));
            if snap {
                let step = 15f32.to_radians();
                turn = (turn / step).round() * step;
            }
            Affine::translate(c.x, c.y).mul(&Affine::rotate(turn)).mul(&Affine::translate(-c.x, -c.y)).mul(f0)
        }
        // Skew: the top edge slides along x, the right edge along y (in the frame's own axes).
        Grip::SkewX => f0.mul(&Affine { m: [1.0, 0.0, (-u.x).clamp(-10.0, 10.0), 1.0, 0.0, 0.0] }),
        Grip::SkewY => f0.mul(&Affine { m: [1.0, u.y.clamp(-10.0, 10.0), 0.0, 1.0, 0.0, 0.0] }),
    };
    (f.is_finite() && f.determinant().abs() > 1e-6).then_some((f, None))
}

/// Pointer at stage point `at` while dragging: the fill follows, inside the drag's gesture.
pub fn drag_to(app: &mut App, d: Drag, at: Vec2, shift: bool) {
    let Drag::Gradient { id, grip, frame, press } = d else { return };
    let Some(t) = target_by_id(app, id) else { return };
    let Some((f, focal)) = dragged_frame(t.kind == Kind::Radial, grip, &frame, Vec2::new(press.x, press.y), at, shift) else { return };
    let p = match focal {
        Some(v) => json!({"element": id.0, "focal": v}),
        None => json!({"element": id.0, "frame": f.m}),
    };
    let _ = app.s.execute("paint.transform_fill", &p);
}

pub fn drag_end(app: &mut App) {
    app.s.end_gesture();
    app.status_ok(tr!("Transformed the fill. {undo} to undo.", undo = app.undo_key()));
}

/// The cursor while hovering.
pub fn cursor(app: &App, at: Option<Pos2>, xf: Xf) -> CursorIcon {
    let grip = match app.drag {
        Drag::Gradient { grip, .. } => Some(grip),
        _ => at.and_then(|p| target(app).and_then(|t| hit(&t, p, xf))),
    };
    match grip {
        Some(Grip::Move | Grip::Focal) => CursorIcon::Move,
        Some(Grip::Rotate) => CursorIcon::Alias,
        Some(Grip::Width | Grip::SkewY) => CursorIcon::ResizeHorizontal,
        Some(Grip::Height | Grip::SkewX) => CursorIcon::ResizeVertical,
        Some(Grip::Size) => CursorIcon::ResizeNwSe,
        None => CursorIcon::Default,
    }
}

/// The tool's overlay: the selected fill's frame and handles, the hovered one highlighted.
pub fn draw_overlay(app: &App, painter: &egui::Painter, xf: Xf, hover: Option<Pos2>) {
    if app.tool != Tool::GradientTransform {
        return;
    }
    let t = match app.drag {
        Drag::Gradient { id, .. } => target_by_id(app, id),
        _ => target(app),
    };
    let Some(t) = t else { return };
    let to_px = |u: Vec2| xf.to_screen(t.frame.apply(u));
    let line = Line::new(1.0, T.selection);
    let faint = Line::new(1.0, T.selection.gamma_multiply(0.7));
    match t.kind {
        Kind::Linear => {
            // The two edges of the ramp, across the shape.
            for x in [-1.0, 1.0] {
                painter.line_segment([to_px(Vec2::new(x, -1.0)), to_px(Vec2::new(x, 1.0))], line);
            }
            painter.add(egui::Shape::dashed_line(&[to_px(Vec2::new(-1.0, 0.0)), to_px(Vec2::new(1.0, 0.0))], faint, 4.0, 3.0));
        }
        Kind::Radial => {
            let ring: Vec<Pos2> = (0..96).map(|k| to_px(Vec2::new((k as f32 / 96.0 * std::f32::consts::TAU).cos(), (k as f32 / 96.0 * std::f32::consts::TAU).sin()))).collect();
            painter.add(egui::Shape::closed_line(ring, line));
            painter.add(egui::Shape::dashed_line(&[to_px(Vec2::ZERO), to_px(Vec2::new(1.0, 0.0))], faint, 4.0, 3.0));
        }
        Kind::Bitmap => {
            let corners = [Vec2::new(-1.0, -1.0), Vec2::new(1.0, -1.0), Vec2::new(1.0, 1.0), Vec2::new(-1.0, 1.0), Vec2::new(-1.0, -1.0)].map(to_px);
            painter.add(egui::Shape::dashed_line(&corners, line, 5.0, 3.0));
        }
    }
    let hot = match app.drag {
        Drag::Gradient { grip, .. } => Some(grip),
        _ => hover.and_then(|p| hit(&t, p, xf)),
    };
    for (g, u) in handles(&t) {
        let c = to_px(u);
        let fill = if hot == Some(g) { T.selection } else { Color32::WHITE };
        let edge = Line::new(1.0, T.selection);
        match g {
            Grip::Move => {
                painter.circle(c, 4.5, fill, edge);
            }
            Grip::Focal => {
                // An inverted triangle, as Animate marks the focal point.
                let pts = vec![c + vec2(-5.0, -4.0), c + vec2(5.0, -4.0), c + vec2(0.0, 4.5)];
                painter.add(egui::Shape::convex_polygon(pts, fill, edge));
            }
            Grip::Width | Grip::Height => {
                painter.rect(egui::Rect::from_center_size(c, vec2(8.0, 8.0)), 0.0, fill, edge, StrokeKind::Middle);
            }
            Grip::SkewX | Grip::SkewY => {
                let pts = vec![c + vec2(0.0, -5.0), c + vec2(5.0, 0.0), c + vec2(0.0, 5.0), c + vec2(-5.0, 0.0)];
                painter.add(egui::Shape::convex_polygon(pts, fill, edge));
            }
            Grip::Size => {
                painter.circle(c, 4.5, fill, edge);
                painter.circle_stroke(c, 1.5, edge);
            }
            Grip::Rotate => {
                // A knob joined to where it turns from.
                painter.circle(c, 5.0, fill, edge);
                painter.circle_filled(c, 1.5, T.selection);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: &Affine, b: &Affine) -> bool {
        a.m.iter().zip(b.m).all(|(x, y)| (x - y).abs() < 1e-3)
    }

    fn frames(ctx: &egui::Context, app: &mut App) {
        let screen = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1600.0, 1000.0))), ..Default::default() };
        for _ in 0..2 {
            let mut out = ctx.run_ui(screen.clone(), |ui| app.frame(ui));
            out.textures_delta.clear();
        }
    }

    #[test]
    fn a_drag_on_a_handle_is_one_undo_step_on_every_kind_of_fill() {
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, anicraft_engine::Session::default(), Some(std::path::PathBuf::from("unused.png")));
        let xf = Xf { origin: pos2(0.0, 0.0), zoom: 1.0 };
        app.tool = Tool::GradientTransform;
        // A solid fill has no handles: picking it says how to get a gradient.
        app.run("draw.rect", json!({"x": 100, "y": 100, "w": 200, "h": 100, "fill": "#ff0000", "stroke": null}));
        pick(&mut app, pos2(150.0, 150.0), xf);
        assert!(target(&app).is_none());
        let png = anicraft_engine::render::encode_png(4, 2, &[200; 32]).unwrap_or_default();
        let id = app.s.doc.alloc_id();
        app.s.doc.bitmaps.push(std::sync::Arc::new(anicraft_engine::doc::Bitmap::from_png(anicraft_engine::doc::BitmapId(id), "tile.png", png).unwrap()));
        for (kind, extra) in [("linear", json!({})), ("radial", json!({"focal": 0.3})), ("bitmap", json!({"bitmap": id}))] {
            let mut p = json!({"type": kind});
            if let (Some(o), Some(e)) = (p.as_object_mut(), extra.as_object()) {
                o.extend(e.clone());
            }
            app.run("paint.set_fill", p);
            frames(&ctx, &mut app);
            let t = target(&app).expect("a gradient or bitmap to transform");
            // Every handle shows, with a cursor that says what it does.
            for (g, u) in handles(&t) {
                let at = xf.to_screen(t.frame.apply(u));
                assert_eq!(hit(&t, at, xf), Some(g), "{kind} {g:?}");
                assert_ne!(cursor(&app, Some(at), xf), CursorIcon::Default);
            }
            // Grab the width handle and pull it out: the fill widens, in one step.
            let (_, w) = handles(&t).into_iter().find(|(g, _)| *g == Grip::Width).expect("a width handle");
            let press = xf.to_screen(t.frame.apply(w));
            let steps = app.s.history_len();
            let d = drag_start(&mut app, press, xf);
            assert!(matches!(d, Drag::Gradient { grip: Grip::Width, .. }), "{kind}: {d:?}");
            app.drag = d;
            frames(&ctx, &mut app);
            for k in 1..=3 {
                let to = t.frame.apply(w * (1.0 + 0.2 * k as f32));
                drag_to(&mut app, d, to, false);
            }
            assert_eq!(cursor(&app, None, xf), CursorIcon::ResizeHorizontal);
            app.drag = Drag::None;
            drag_end(&mut app);
            assert_eq!(app.s.history_len(), steps + 1, "{kind}: one step");
            let after = target(&app).expect("still a fill");
            let len = |f: &Affine| Vec2::new(f.m[0], f.m[1]).length();
            assert!((len(&after.frame) / len(&t.frame) - 1.6).abs() < 1e-3, "{kind}: {:?}", after.frame);
        }
        // The focal point slides along the radial gradient's axis.
        app.run("paint.set_fill", json!({"type": "radial", "focal": 0.0}));
        let t = target(&app).expect("radial");
        let press = xf.to_screen(t.frame.apply(Vec2::ZERO));
        let d = Drag::Gradient { id: t.id, grip: Grip::Focal, frame: t.frame, press: pos2(press.x, press.y) };
        drag_to(&mut app, d, t.frame.apply(Vec2::new(0.5, 0.4)), false);
        assert!((target(&app).expect("radial").focal - 0.5).abs() < 1e-4);
        // A press away from any handle selects what is under it; on nothing, deselects.
        assert_eq!(drag_start(&mut app, pos2(5.0, 5.0), xf), Drag::None);
        assert!(app.s.selection.is_empty());
        frames(&ctx, &mut app);
    }

    #[test]
    fn handle_drags_make_the_frames_animate_does() {
        // A ring of radius 50 round (100, 100).
        let f0 = Affine { m: [50.0, 0.0, 0.0, 50.0, 100.0, 100.0] };
        let at = |x: f32, y: f32| Vec2::new(x, y);
        // Move: by the pointer's travel.
        let (f, _) = dragged_frame(true, Grip::Move, &f0, at(100.0, 100.0), at(130.0, 90.0), false).unwrap();
        assert!(near(&f, &Affine { m: [50.0, 0.0, 0.0, 50.0, 130.0, 90.0] }));
        // Width: the ring's x radius follows the pointer, its height stays.
        let (f, _) = dragged_frame(true, Grip::Width, &f0, at(150.0, 100.0), at(200.0, 100.0), false).unwrap();
        assert!(near(&f, &Affine { m: [100.0, 0.0, 0.0, 50.0, 100.0, 100.0] }));
        // Size: both, by the distance from the centre.
        let (f, _) = dragged_frame(true, Grip::Size, &f0, at(150.0, 100.0), at(125.0, 100.0), false).unwrap();
        assert!(near(&f, &Affine { m: [25.0, 0.0, 0.0, 25.0, 100.0, 100.0] }));
        // Rotate: a quarter turn about the centre (Shift snaps 80° to 75°).
        let (f, _) = dragged_frame(true, Grip::Rotate, &f0, at(150.0, 100.0), at(100.0, 150.0), false).unwrap();
        assert!(near(&f, &Affine { m: [0.0, 50.0, -50.0, 0.0, 100.0, 100.0] }), "{f:?}");
        let a = 80f32.to_radians();
        let (f, _) = dragged_frame(true, Grip::Rotate, &f0, at(150.0, 100.0), at(100.0 + 50.0 * a.cos(), 100.0 + 50.0 * a.sin()), true).unwrap();
        assert!((f.m[1].atan2(f.m[0]).to_degrees() - 75.0).abs() < 1e-3);
        // Focal point: along the axis only, held inside the ring.
        let (f, focal) = dragged_frame(true, Grip::Focal, &f0, at(100.0, 100.0), at(125.0, 140.0), false).unwrap();
        assert!(near(&f, &f0));
        assert_eq!(focal, Some(0.5));
        assert_eq!(dragged_frame(true, Grip::Focal, &f0, at(100.0, 100.0), at(400.0, 100.0), false).unwrap().1, Some(MAX_FOCAL));
        assert!(dragged_frame(false, Grip::Focal, &f0, at(100.0, 100.0), at(120.0, 100.0), false).is_none(), "no focal point on a linear gradient");
        // Skews slide the top edge along x and the right edge along y.
        let (f, _) = dragged_frame(false, Grip::SkewX, &f0, at(100.0, 50.0), at(125.0, 50.0), false).unwrap();
        assert!(f.apply(Vec2::new(0.0, -1.0)).distance(at(125.0, 50.0)) < 1e-3);
        let (f, _) = dragged_frame(false, Grip::SkewY, &f0, at(150.0, 100.0), at(150.0, 80.0), false).unwrap();
        assert!(f.apply(Vec2::new(1.0, 0.0)).distance(at(150.0, 80.0)) < 1e-3);
        // Dragging a width handle onto the centre never collapses the frame.
        let (f, _) = dragged_frame(true, Grip::Width, &f0, at(150.0, 100.0), at(100.0, 100.0), false).unwrap();
        assert!(f.determinant().abs() > 1e-6);
    }
}
