//! Width tool (U): varies a stroke's width along its length (ADR 0018).
//!
//! Hover a stroke on the active layer to see its width points: each is a cross-section across
//! the stroke with a handle on either side and a dot on the line. Drag a handle to widen or
//! narrow the stroke there (both sides together; Alt for that side only), drag the dot to slide
//! the point along the stroke, and drag anywhere else on the stroke to add a point there and
//! size it. Click a point to select it; Delete removes it. Each drag is one undo step.
//!
//! Only view state lives here; every change is a `stroke.*` command, so agents and the CLI
//! edit widths exactly the same way.

use crate::app::{App, Drag, Tool};
use crate::pathtools::Xf;
use crate::theme::T;
use anicraft_engine::doc::width::{nearest_on_stroke, stroke_walks};
use anicraft_engine::doc::{ElementId, ElementKind, Stroke};
use anicraft_engine::geom::walk::PathWalk;
use anicraft_engine::geom::{Affine, Vec2};
use egui::{Color32, Key, Modifiers, Pos2, Rect, Stroke as Line, StrokeKind, vec2};
use serde_json::json;

/// Pick radius for handles, in screen points.
const PICK: f32 = 7.0;
/// Most stroked shapes looked at per frame.
const MAX_TARGETS: usize = 512;

/// What part of a width point is grabbed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grip {
    /// The handle on the left (+1) or right (−1) side.
    Side(i8),
    /// The dot on the line: slides the point along the stroke.
    Along,
}

/// A stroked shape on the active layer, measured for the tool.
struct Target {
    id: ElementId,
    /// Local → stage.
    m: Affine,
    walks: Vec<PathWalk>,
    stroke: Stroke,
}

impl Target {
    /// Nominal half-width in local units.
    fn hw(&self) -> f32 {
        self.stroke.width.max(0.0) * 0.5
    }
    fn factors(&self, t: f32) -> (f32, f32) {
        self.stroke.profile.as_ref().map_or((1.0, 1.0), |p| p.at(t))
    }
    /// Centre, unit left normal and the two handle positions (local) at `t` on subpath `sub`.
    fn section(&self, sub: usize, t: f32, (l, r): (f32, f32)) -> Option<(Vec2, Vec2, Vec2, Vec2)> {
        let w = self.walks.get(sub)?;
        let s = t * w.length();
        let c = w.at(s);
        let n = anicraft_engine::doc::width::left_normal(w.tangent(s));
        let hw = self.hw();
        Some((c, n, c + n * (l * hw), c - n * (r * hw)))
    }
}

fn targets(app: &App, zoom: f32) -> Vec<Target> {
    if app.s.active_layer().is_none_or(|l| l.locked) {
        return Vec::new();
    }
    app.s
        .visible_elements()
        .into_iter()
        .rev()
        .filter_map(|e| match &e.kind {
            ElementKind::Shape(sh) => {
                let stroke = sh.stroke.clone()?;
                let scale = e.transform.mean_scale().max(1e-3);
                // A quarter of a screen point, in the shape's units.
                let walks = stroke_walks(&sh.path, 0.25 / (zoom.max(1e-3) * scale));
                Some(Target { id: e.id, m: e.transform, walks, stroke })
            }
            ElementKind::Symbol { .. } | ElementKind::Text(_) | ElementKind::Camera | ElementKind::Group { .. } => None,
        })
        .take(MAX_TARGETS)
        .collect()
}

/// What is under the pointer: a width point's grip, or a place on a stroke.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub id: ElementId,
    pub sub: usize,
    pub t: f32,
    /// The width point and the grip, when one is under the pointer.
    pub point: Option<(usize, Grip)>,
    /// Which side of the line the pointer is on (local space: +1 left).
    pub side: i8,
}

fn hit(app: &App, at: Pos2, xf: Xf) -> Option<Hit> {
    let ts = targets(app, xf.zoom);
    let screen = |t: &Target, q: Vec2| xf.to_screen(t.m.apply(q));
    // Grips first (they sit on top of the stroke), topmost shape first.
    for t in &ts {
        let Some(pr) = &t.stroke.profile else { continue };
        for (i, p) in pr.points().iter().enumerate() {
            for sub in 0..t.walks.len().min(64) {
                let Some((c, _, lh, rh)) = t.section(sub, p.t, (p.left, p.right)) else { continue };
                for (q, grip) in [(lh, Grip::Side(1)), (rh, Grip::Side(-1)), (c, Grip::Along)] {
                    if screen(t, q).distance(at) <= PICK {
                        return Some(Hit { id: t.id, sub, t: p.t, point: Some((i, grip)), side: if let Grip::Side(s) = grip { s } else { 1 } });
                    }
                }
            }
        }
    }
    // Then the stroke itself, as wide as it is drawn there.
    let sp = xf.to_stage(at);
    for t in &ts {
        let Some(inv) = t.m.inverse() else { continue };
        let Some((sub, tt, d, side)) = nearest_on_stroke(&t.walks, inv.apply(sp)) else { continue };
        let (l, r) = t.factors(tt);
        let scale = t.m.mean_scale() * xf.zoom;
        let here = (if side > 0.0 { l } else { r }) * t.hw();
        if (d - here) * scale <= PICK {
            return Some(Hit { id: t.id, sub, t: tt, point: None, side: if side > 0.0 { 1 } else { -1 } });
        }
    }
    None
}

/// Press with the Width tool: grab a width point, else add one where the stroke was pressed.
/// Returns the drag (the gesture is open: one undo step).
pub fn drag_start(app: &mut App, press: Pos2, xf: Xf) -> Drag {
    let Some(h) = hit(app, press, xf) else { return Drag::None };
    if !app.s.selection.contains(app.s.layer, h.id) {
        app.run("element.select", json!({"ids": [h.id.0]}));
    }
    // Undo labels stay English in the history and are translated where they show.
    app.s.begin_gesture(tr_noop!("Width"));
    // A press on the stroke itself makes a point there once the drag sizes it. It is not
    // added yet: at the width the stroke already has, a first point leaves it uniform, so the
    // engine keeps no point for it (#250).
    let (index, grip) = match h.point {
        Some((i, g)) => (Some(i), g),
        None => (None, Grip::Side(h.side)),
    };
    app.width_sel = index.map(|i| (h.id, i));
    let (l, r) = index.and_then(|i| current_point(app, h.id, i)).map_or_else(|| factors_at(app, h.id, h.t), |(l, r, _)| (l, r));
    Drag::Width { id: h.id, sub: h.sub, index, t: h.t, grip, start: (l, r) }
}

/// The left and right factors of shape `id`'s stroke at `t` (1, 1 when uniform).
fn factors_at(app: &App, id: ElementId, t: f32) -> (f32, f32) {
    let el = app.s.visible_elements().into_iter().find(|e| e.id == id);
    let profile = el.as_ref().and_then(|e| match &e.kind {
        ElementKind::Shape(sh) => sh.stroke.as_ref().and_then(|st| st.profile.clone()),
        _ => None,
    });
    profile.map_or((1.0, 1.0), |p| p.at(t))
}

/// The left and right factors and the position of width point `index` on shape `id`.
fn current_point(app: &App, id: ElementId, index: usize) -> Option<(f32, f32, f32)> {
    let el = app.s.visible_elements().into_iter().find(|e| e.id == id)?;
    let ElementKind::Shape(sh) = &el.kind else { return None };
    let p = *sh.stroke.as_ref()?.profile.as_ref()?.points().get(index)?;
    Some((p.left, p.right, p.t))
}

/// Pointer at stage point `at` while dragging: resize or slide the grabbed point.
pub fn drag_to(app: &mut App, d: Drag, at: Vec2, alt: bool, zoom: f32) {
    let Drag::Width { id, sub, index, t: pt_t, grip, start } = d else { return };
    let Some(t) = targets(app, zoom).into_iter().find(|t| t.id == id) else { return };
    let Some(inv) = t.m.inverse() else { return };
    let local = inv.apply(at);
    let p = match (grip, index) {
        (Grip::Along, Some(index)) => {
            let Some(w) = t.walks.get(sub) else { return };
            let len = w.length();
            let tt = if len > 0.0 { w.project(local) / len } else { 0.0 };
            json!({"id": id.0, "index": index, "t": tt})
        }
        (Grip::Along, None) => return,
        (Grip::Side(side), _) => {
            // The point is addressed by where it is, not by its index: sized back to the
            // stroke's own width it leaves the stroke uniform (no points), and dragging on
            // brings it back at the same place.
            let (cl, cr) = t.factors(pt_t);
            let Some((c, n, _, _)) = t.section(sub, pt_t, (cl, cr)) else { return };
            let hw = t.hw();
            if hw <= 0.0 {
                return;
            }
            let f = ((local - c).dot(n) * f32::from(side) / hw).clamp(0.0, anicraft_engine::doc::MAX_WIDTH_FACTOR);
            let (l0, r0) = start;
            // Both sides move together (keeping their difference); Alt moves just this one.
            let (l, r) = match (side > 0, alt) {
                (true, true) => (f, r0),
                (false, true) => (l0, f),
                (true, false) => (f, (r0 + f - l0).max(0.0)),
                (false, false) => ((l0 + f - r0).max(0.0), f),
            };
            json!({"id": id.0, "t": pt_t, "left": l, "right": r})
        }
    };
    // Inside the gesture: one undo step for the whole drag.
    if let Ok(v) = app.s.execute("stroke.set_width_point", &p) {
        app.width_sel = v["index"].as_u64().map(|i| (id, i as usize));
    }
}

pub fn drag_end(app: &mut App) {
    app.s.end_gesture();
    app.status_ok(tr!("Changed the stroke's width. {undo} to undo.", undo = app.undo_key()));
}

/// Click: select the width point under the pointer, or the stroked shape.
pub fn click(app: &mut App, at: Pos2, xf: Xf) {
    match hit(app, at, xf) {
        Some(h) => {
            app.run("element.select", json!({"ids": [h.id.0]}));
            app.width_sel = h.point.map(|(i, _)| (h.id, i));
        }
        None => {
            app.width_sel = None;
            app.run("edit.deselect", json!({}));
        }
    }
}

/// Delete removes the selected width point; Esc deselects it.
pub fn consume_keys(app: &mut App, ctx: &egui::Context) {
    if app.tool != Tool::Width {
        return;
    }
    let Some((id, index)) = app.width_sel else { return };
    let take = |k: Key| ctx.input_mut(|i| i.consume_key(Modifiers::NONE, k));
    if take(Key::Delete) || take(Key::Backspace) {
        app.width_sel = None;
        if app.run("stroke.delete_width_point", json!({"id": id.0, "index": index})).is_some() {
            app.status_ok(tr!("Removed the width point. {undo} to undo.", undo = app.undo_key()));
        }
    } else if take(Key::Escape) {
        app.width_sel = None;
    }
}

/// Drops a selected width point that no longer exists.
pub fn upkeep(app: &mut App) {
    if app.tool != Tool::Width {
        app.width_sel = None;
        return;
    }
    if let Some((id, i)) = app.width_sel
        && current_point(app, id, i).is_none()
    {
        app.width_sel = None;
    }
}

/// The cursor while hovering: a grab hand over a point, the add cursor on a stroke (a drag
/// there adds a point, as the Pen's add cursor), and a crosshair elsewhere, so the tool in
/// hand shows on the stage (#250).
pub fn cursor(app: &App, at: Option<Pos2>, xf: Xf) -> egui::CursorIcon {
    if matches!(app.drag, Drag::Width { .. }) {
        return egui::CursorIcon::Grabbing;
    }
    match at.and_then(|p| hit(app, p, xf)) {
        Some(Hit { point: Some(_), .. }) => egui::CursorIcon::Grab,
        Some(_) => egui::CursorIcon::Copy,
        None => egui::CursorIcon::Crosshair,
    }
}

fn diamond(painter: &egui::Painter, c: Pos2, r: f32, fill: Color32) {
    let pts = vec![c + vec2(0.0, -r), c + vec2(r, 0.0), c + vec2(0.0, r), c + vec2(-r, 0.0)];
    painter.add(egui::Shape::convex_polygon(pts, fill, Line::new(1.0, T.selection)));
}

/// The tool's overlay: the hovered (or dragged, or selected) stroke's width points, and where a
/// drag would add one.
pub fn draw_overlay(app: &App, painter: &egui::Painter, xf: Xf, hover: Option<Pos2>) {
    if app.tool != Tool::Width {
        return;
    }
    let h = if app.drag == Drag::None { hover.and_then(|p| hit(app, p, xf)) } else { None };
    let shown = match app.drag {
        Drag::Width { id, .. } => Some(id),
        _ => h.map(|h| h.id).or(app.width_sel.map(|s| s.0)),
    };
    let Some(t) = shown.and_then(|id| targets(app, xf.zoom).into_iter().find(|t| t.id == id)) else { return };
    let to_px = |q: Vec2| xf.to_screen(t.m.apply(q));
    let line = Line::new(1.0, T.selection);
    // The path the widths are measured along.
    for w in &t.walks {
        let n = 400usize;
        let pts: Vec<Pos2> = (0..=n).map(|k| to_px(w.at(w.length() * k as f32 / n as f32))).collect();
        painter.add(egui::Shape::line(pts, Line::new(1.0, T.selection.gamma_multiply(0.7))));
    }
    let grabbed = match app.drag {
        Drag::Width { index, .. } => index,
        _ => None,
    };
    if let Some(pr) = &t.stroke.profile {
        for (i, p) in pr.points().iter().enumerate() {
            let selected = app.width_sel == Some((t.id, i)) || grabbed == Some(i);
            for sub in 0..t.walks.len().min(64) {
                let Some((c, _, lh, rh)) = t.section(sub, p.t, (p.left, p.right)) else { continue };
                let (c, lh, rh) = (to_px(c), to_px(lh), to_px(rh));
                painter.line_segment([lh, rh], line);
                let fill = if selected { T.selection } else { Color32::WHITE };
                diamond(painter, lh, 4.0, fill);
                diamond(painter, rh, 4.0, fill);
                painter.circle(c, 3.0, fill, line);
            }
        }
    }
    // Where a drag adds a point: a cross-section at the pointer with a white box on the line
    // and handles at the stroke's edges (#250: easy to see on any artwork), and the same while
    // a drag on the stroke is still at its own width (no point kept yet).
    let pending = match app.drag {
        Drag::Width { id, sub, index: None, t: at, .. } if id == t.id && app.width_sel.is_none() => Some((sub, at)),
        _ => h.filter(|h| h.point.is_none() && h.id == t.id).map(|h| (h.sub, h.t)),
    };
    if let Some((sub, at)) = pending
        && let Some((c, _, lh, rh)) = t.section(sub, at, t.factors(at))
    {
        let (c, lh, rh) = (to_px(c), to_px(lh), to_px(rh));
        painter.add(egui::Shape::dashed_line(&[lh, rh], line, 3.0, 2.0));
        diamond(painter, lh, 4.0, Color32::WHITE);
        diamond(painter, rh, 4.0, Color32::WHITE);
        let r = Rect::from_center_size(c, vec2(8.0, 8.0));
        painter.rect_filled(r, 0.0, Color32::WHITE);
        painter.rect_stroke(r, 0.0, Line::new(1.5, T.selection), StrokeKind::Middle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    const XF: Xf = Xf { origin: Pos2::ZERO, zoom: 1.0 };

    fn widths(app: &App) -> Vec<f32> {
        app.s.visible_elements().iter().filter_map(|e| match &e.kind {
            ElementKind::Shape(sh) => sh.stroke.as_ref().map(|st| st.profile.as_ref().map_or(1.0, |p| p.at(0.5).0)),
            _ => None,
        }).collect()
    }

    /// #250: a Pencil squiggle in merge drawing, then a drag on it with the Width tool widens it.
    #[test]
    fn a_drag_on_a_pencil_stroke_widens_it() {
        let ctx = egui::Context::default();
        let session = anicraft_engine::Session::new(anicraft_engine::doc::Document::default());
        let mut app = App::with_context(&ctx, session, Some(std::path::PathBuf::from("unused.png")));
        app.view.zoom = 1.0;
        app.s.style.object_drawing = false;
        app.s.style.stroke_width = 4.0;
        let points: Vec<[f32; 2]> = (0..=40).map(|i| [100.0 + i as f32 * 5.0, 100.0 + (i as f32 * 0.3).sin() * 10.0]).collect();
        app.run("draw.freehand", json!({"points": points, "mode": "ink", "tolerance": 1.5})).expect("drawn");
        app.tool = Tool::Width;
        let mid = app.s.visible_elements().first().map(|e| e.id).expect("a stroke");
        let at = app.s.execute("stroke.width_at", &json!({"x": 200, "y": 100.0 + (20.0f32 * 0.3).sin() * 10.0, "slop": 4})).expect("width_at");
        assert_eq!(at["element"], mid.0, "{at}");
        let press = pos2(200.0, 100.0 + (20.0f32 * 0.3).sin() * 10.0 - 1.0);
        let steps = app.s.history_len();
        let d = drag_start(&mut app, press, XF);
        assert!(matches!(d, Drag::Width { index: None, .. }), "the press takes the stroke: {d:?}");
        drag_to(&mut app, d, Vec2::new(press.x, press.y - 8.0), false, 1.0);
        drag_end(&mut app);
        assert!(widths(&app).iter().any(|w| *w > 2.0), "the stroke is wider there: {:?}", widths(&app));
        assert_eq!(app.s.history_len(), steps + 1, "one undo step");
        assert!(app.width_sel.is_some(), "the new point is selected");
        // The same point dragged back to the stroke's own width (no point left) and out again,
        // in one drag: it keeps following the pointer.
        let y = press.y + 1.0;
        let w = widths(&app)[0] * 2.0;
        let d = drag_start(&mut app, pos2(press.x, y - w), XF);
        assert!(matches!(d, Drag::Width { index: Some(0), grip: Grip::Side(_), .. }), "the handle: {d:?}");
        let Drag::Width { id, sub, t: at, .. } = d else { unreachable!() };
        let tg = targets(&app, 1.0).into_iter().find(|t| t.id == id).expect("target");
        let (_, _, edge, _) = tg.section(sub, at, (1.0, 1.0)).expect("section");
        drag_to(&mut app, d, tg.m.apply(edge), false, 1.0);
        assert!(widths(&app).iter().all(|w| (*w - 1.0).abs() < 1e-3), "back to uniform: {:?}", widths(&app));
        drag_to(&mut app, d, Vec2::new(press.x, y - 12.0), false, 1.0);
        drag_end(&mut app);
        assert!(widths(&app).iter().any(|w| *w > 3.0), "wider again: {:?}", widths(&app));
    }
}
