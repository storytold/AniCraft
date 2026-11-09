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
            ElementKind::Symbol { .. } | ElementKind::Text(_) | ElementKind::Camera => None,
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
    if !app.s.selection.contains(&h.id) {
        app.run("element.select", json!({"ids": [h.id.0]}));
    }
    app.s.begin_gesture("Width");
    let (index, grip) = match h.point {
        Some(p) => p,
        None => {
            let Some(v) = app.run("stroke.set_width_point", json!({"id": h.id.0, "t": h.t})) else {
                app.s.end_gesture();
                return Drag::None;
            };
            let Some(i) = v["index"].as_u64() else {
                app.s.end_gesture();
                return Drag::None;
            };
            (i as usize, Grip::Side(h.side))
        }
    };
    app.width_sel = Some((h.id, index));
    let (l, r) = current_point(app, h.id, index).unwrap_or((1.0, 1.0));
    Drag::Width { id: h.id, sub: h.sub, index, grip, start: (l, r) }
}

/// The left and right factors of width point `index` on shape `id`.
fn current_point(app: &App, id: ElementId, index: usize) -> Option<(f32, f32)> {
    let el = app.s.visible_elements().into_iter().find(|e| e.id == id)?;
    let ElementKind::Shape(sh) = &el.kind else { return None };
    let p = *sh.stroke.as_ref()?.profile.as_ref()?.points().get(index)?;
    Some((p.left, p.right))
}

/// Pointer at stage point `at` while dragging: resize or slide the grabbed point.
pub fn drag_to(app: &mut App, d: Drag, at: Vec2, alt: bool, zoom: f32) {
    let Drag::Width { id, sub, index, grip, start } = d else { return };
    let Some(t) = targets(app, zoom).into_iter().find(|t| t.id == id) else { return };
    let Some(inv) = t.m.inverse() else { return };
    let local = inv.apply(at);
    let p = match grip {
        Grip::Along => {
            let Some(w) = t.walks.get(sub) else { return };
            let len = w.length();
            let tt = if len > 0.0 { w.project(local) / len } else { 0.0 };
            json!({"id": id.0, "index": index, "t": tt})
        }
        Grip::Side(side) => {
            let pt = t.stroke.profile.as_ref().and_then(|p| p.points().get(index).copied());
            let Some(pt) = pt else { return };
            let Some((c, n, _, _)) = t.section(sub, pt.t, (pt.left, pt.right)) else { return };
            let hw = t.hw();
            if hw <= 0.0 {
                return;
            }
            let f = ((local - c).dot(n) * f32::from(side) / hw).clamp(0.0, anicraft_engine::doc::MAX_WIDTH_FACTOR);
            let (l0, r0) = start;
            // Both sides move together (keeping their difference); Alt moves just this one.
            let (l, r) = match (side > 0, alt) {
                (true, true) => (f, pt.right),
                (false, true) => (pt.left, f),
                (true, false) => (f, (r0 + f - l0).max(0.0)),
                (false, false) => ((l0 + f - r0).max(0.0), f),
            };
            json!({"id": id.0, "index": index, "left": l, "right": r})
        }
    };
    // Inside the gesture: one undo step for the whole drag.
    let _ = app.s.execute("stroke.set_width_point", &p);
}

pub fn drag_end(app: &mut App) {
    app.s.end_gesture();
    app.status_ok("Changed the stroke's width. Ctrl+Z to undo.");
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
            app.status_ok("Removed the width point. Ctrl+Z to undo.");
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

/// The cursor while hovering: a grab hand over a point, a crosshair on a stroke.
pub fn cursor(app: &App, at: Option<Pos2>, xf: Xf) -> egui::CursorIcon {
    if matches!(app.drag, Drag::Width { .. }) {
        return egui::CursorIcon::Grabbing;
    }
    match at.and_then(|p| hit(app, p, xf)) {
        Some(Hit { point: Some(_), .. }) => egui::CursorIcon::Grab,
        Some(_) => egui::CursorIcon::Crosshair,
        None => egui::CursorIcon::Default,
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
        Drag::Width { index, .. } => Some(index),
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
    // Where a drag would add a point: a faint cross-section at the pointer.
    if let Some(h) = h.filter(|h| h.point.is_none() && h.id == t.id)
        && let Some((c, _, lh, rh)) = t.section(h.sub, h.t, t.factors(h.t))
    {
        let faint = Line::new(1.0, T.selection.gamma_multiply(0.6));
        painter.add(egui::Shape::dashed_line(&[to_px(lh), to_px(rh)], faint, 3.0, 2.0));
        painter.rect_stroke(Rect::from_center_size(to_px(c), vec2(5.0, 5.0)), 0.0, faint, StrokeKind::Middle);
    }
}
