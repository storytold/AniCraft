//! Pen (P), Pencil (Shift+Y) and Subselection (A).
//!
//! The Pen's interaction model comes from PhotoCraft's Pen (`vector_ui.rs`): knots kept as
//! `[anchor, in, out]` while drawing, a press adds a knot, dragging pulls mirrored handles,
//! pressing the first knot closes the path, and the finished path is one command, so one undo
//! step. Added here: Alt breaks the handles, a click on the last knot (or a double-click)
//! finishes an open path, Backspace takes back the last knot, a tiny drag stays a corner, and
//! switching tools keeps the path instead of dropping it.
//!
//! Only view state lives here. Every document change is an engine command (`draw.path`,
//! `draw.freehand`, `path.*`), so agents and the CLI draw and edit exactly the same way.

use crate::app::{App, Drag, Tool};
use crate::theme::T;
use anicraft_engine::doc::{ElementId, ElementKind};
use anicraft_engine::geom::edit::{ANCHOR, C1, C2};
use anicraft_engine::geom::{Affine, Path, PathCmd, Vec2};
use egui::{Color32, Key, Modifiers, Pos2, Rect, Stroke, StrokeKind, pos2, vec2};
use serde_json::json;
use std::sync::Arc;

/// Screen ↔ stage mapping of the current frame.
#[derive(Clone, Copy)]
pub struct Xf {
    pub origin: Pos2,
    pub zoom: f32,
}

impl Xf {
    pub fn to_screen(self, p: Vec2) -> Pos2 {
        pos2(self.origin.x + p.x * self.zoom, self.origin.y + p.y * self.zoom)
    }
    pub fn to_stage(self, p: Pos2) -> Vec2 {
        Vec2::new((p.x - self.origin.x) / self.zoom, (p.y - self.origin.y) / self.zoom)
    }
    /// Stage units per screen point.
    pub fn unit(self) -> f32 {
        1.0 / self.zoom.max(1e-3)
    }
}

/// Pick radius for points and knots, in screen points.
const PICK: f32 = 6.0;

// ----- Pen ---------------------------------------------------------------------------------

/// Pen path under construction: knots as `[anchor, in, out]` in stage space.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PenPath {
    pub knots: Vec<[Vec2; 3]>,
    pub dragging: bool,
}

impl PenPath {
    /// SVG path data for `draw.path`. Straight where both handles sit on their anchors.
    pub fn to_svg(&self, closed: bool) -> String {
        let mut d = String::new();
        let Some(first) = self.knots.first() else { return d };
        d.push_str(&format!("M{} {}", first[0].x, first[0].y));
        let seg = |d: &mut String, a: &[Vec2; 3], b: &[Vec2; 3]| {
            if a[2] == a[0] && b[1] == b[0] {
                d.push_str(&format!(" L{} {}", b[0].x, b[0].y));
            } else {
                d.push_str(&format!(" C{} {} {} {} {} {}", a[2].x, a[2].y, b[1].x, b[1].y, b[0].x, b[0].y));
            }
        };
        for w in self.knots.windows(2) {
            seg(&mut d, &w[0], &w[1]);
        }
        if closed {
            if let Some(last) = self.knots.last()
                && (last[2] != last[0] || first[1] != first[0])
            {
                seg(&mut d, last, first);
            }
            d.push_str(" Z");
        }
        d
    }
}

/// Pointer handling for the Pen. Runs on raw press/release (not egui's drag threshold), so a
/// quick click places a point exactly where it was pressed.
pub fn pen_input(app: &mut App, ui: &egui::Ui, resp: &egui::Response, xf: Xf) {
    let (pressed, down, alt) = ui.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down(), i.modifiers.alt));
    let Some(pos) = resp.interact_pointer_pos().or_else(|| resp.hover_pos()) else { return };
    let p = xf.to_stage(pos);
    // Alt-click on a path, with no path being drawn, adds a point to it: the motion path first
    // (ADR 0038), then a selected shape's outline.
    if pressed && resp.hovered() && alt && app.pen.as_ref().is_none_or(|pen| pen.knots.is_empty()) {
        app.pen = None;
        if crate::motion_path::pen_alt_click(app, pos, xf) || add_point_on_selected(app, pos, xf) {
            return;
        }
    }
    if pressed && resp.hovered() {
        let tol = PICK * xf.unit();
        let pen = app.pen.get_or_insert_with(PenPath::default);
        let n = pen.knots.len();
        if n >= 2 && pen.knots.first().is_some_and(|k| k[0].distance(p) <= tol) {
            commit_pen(app, true);
            return;
        }
        if n >= 2 && pen.knots.last().is_some_and(|k| k[0].distance(p) <= tol) {
            commit_pen(app, false);
            return;
        }
        pen.knots.push([p; 3]);
        pen.dragging = true;
        return;
    }
    let Some(pen) = app.pen.as_mut() else { return };
    if !down {
        pen.dragging = false;
        return;
    }
    if pen.dragging
        && let Some(k) = pen.knots.last_mut()
    {
        let a = k[0];
        if a.distance(p) * xf.zoom < 2.0 {
            // A press that barely moved is a corner.
            *k = [a; 3];
        } else {
            k[2] = p;
            if !alt {
                k[1] = a * 2.0 - p;
            }
        }
    }
}

/// Finishes the Pen path as a shape (one undo step). Fewer than two knots are dropped.
pub fn commit_pen(app: &mut App, closed: bool) {
    let Some(pen) = app.pen.take() else { return };
    if pen.knots.len() < 2 {
        return;
    }
    app.run("draw.path", json!({"d": pen.to_svg(closed)}));
}

// ----- Pencil ------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PencilMode {
    #[default]
    Smooth,
    Straighten,
    Ink,
}

impl PencilMode {
    pub fn label(self) -> &'static str {
        match self {
            PencilMode::Smooth => "Smooth",
            PencilMode::Straighten => "Straighten",
            PencilMode::Ink => "Ink",
        }
    }
    pub fn hint(self) -> &'static str {
        match self {
            PencilMode::Smooth => "Fits smooth curves to the stroke, keeping sharp corners.",
            PencilMode::Straighten => "Turns the stroke into straight segments.",
            PencilMode::Ink => "Follows the hand closely with little smoothing.",
        }
    }
    fn id(self) -> &'static str {
        match self {
            PencilMode::Smooth => "smooth",
            PencilMode::Straighten => "straighten",
            PencilMode::Ink => "ink",
        }
    }
}

/// Adds a pointer sample to the stroke (closer samples than half a screen point add nothing).
pub fn pencil_sample(app: &mut App, p: Vec2, xf: Xf) {
    if app.pencil.last().is_none_or(|l| l.distance(p) * xf.zoom >= 0.5) && app.pencil.len() < 100_000 {
        app.pencil.push(p);
    }
}

/// Sends the stroke to the engine; smoothing is relative to the screen, so a stroke looks the
/// same at any zoom.
pub fn commit_pencil(app: &mut App, xf: Xf) {
    let pts = std::mem::take(&mut app.pencil);
    if pts.len() < 2 {
        return;
    }
    let points: Vec<[f32; 2]> = pts.iter().map(|p| [p.x, p.y]).collect();
    app.run("draw.freehand", json!({"points": points, "mode": app.pencil_mode.id(), "tolerance": 1.5 * xf.unit()}));
}

// ----- Subselection --------------------------------------------------------------------------

/// A selected point: element, command, slot.
pub type PointRef = (ElementId, usize, u8);

/// Selected shapes on the active layer with their path and transform.
fn selected_shapes(app: &App) -> Vec<(ElementId, Arc<Path>, Affine)> {
    app.s
        .visible_elements()
        .into_iter()
        .filter(|e| app.s.selection.contains(&e.id))
        .filter_map(|e| match &e.kind {
            ElementKind::Shape(sh) => Some((e.id, sh.path.clone(), e.transform)),
            ElementKind::Symbol { .. } | ElementKind::Text(_) | ElementKind::Camera => None,
        })
        .take(64)
        .collect()
}

/// Handles shown for the selected anchors: `(cmd, slot, anchor cmd)`.
fn shown_handles(path: &Path, anchors: impl Iterator<Item = usize>) -> Vec<(usize, u8, usize)> {
    let mut v = Vec::new();
    for a in anchors {
        let (i, o) = path.anchor_segments(a);
        if let Some(i) = i.filter(|&i| matches!(path.cmds.get(i), Some(PathCmd::CubicTo(..)))) {
            v.push((i, C2, a));
        }
        if let Some(o) = o.filter(|&o| matches!(path.cmds.get(o), Some(PathCmd::CubicTo(..)) | Some(PathCmd::QuadTo(..)))) {
            v.push((o, C1, a));
        }
    }
    v
}

/// Drops point references that no longer exist (after undo, deselection or a frame change).
pub fn upkeep(app: &mut App) {
    if app.points.is_empty() {
        return;
    }
    let shapes = selected_shapes(app);
    app.points.retain(|(id, cmd, slot)| shapes.iter().any(|(e, p, _)| e == id && p.point(*cmd, *slot).is_some()));
}

/// The point under screen position `at`: shown handles first (they sit on top), then anchors.
fn point_at(app: &App, at: Pos2, xf: Xf) -> Option<PointRef> {
    let near = |q: Vec2, m: &Affine| xf.to_screen(m.apply(q)).distance(at) <= PICK;
    for (id, path, m) in selected_shapes(app) {
        let sel = app.points.iter().filter(|p| p.0 == id && p.2 == ANCHOR).map(|p| p.1);
        for (cmd, slot, _) in shown_handles(&path, sel) {
            if path.point(cmd, slot).is_some_and(|q| near(q, &m)) {
                return Some((id, cmd, slot));
            }
        }
        if let Some((cmd, _)) = path.editable_anchors().into_iter().find(|(_, q)| near(*q, &m)) {
            return Some((id, cmd, ANCHOR));
        }
    }
    None
}

/// Press with the Subselection tool: grab a point, else a shape (moves it whole, as Animate
/// does when you drag an outline), else start a marquee over points.
pub fn subselect_drag_start(app: &mut App, press: Pos2, shift: bool, xf: Xf) -> Drag {
    let sp = xf.to_stage(press);
    if let Some(pt) = point_at(app, press, xf) {
        if !app.points.contains(&pt) {
            if !shift {
                app.points.clear();
            }
            app.points.push(pt);
        }
        app.s.begin_gesture("Move Points");
        return Drag::Points { last: pos2(sp.x, sp.y) };
    }
    if let Some((_, el)) = app.s.hit_test(sp, 3.0 * xf.unit()) {
        if !app.s.selection.contains(&el) {
            app.run("element.pick", json!({"x": sp.x, "y": sp.y, "add": shift}));
            app.points.clear();
        }
        app.s.begin_gesture("Move");
        return Drag::Move { last: pos2(sp.x, sp.y) };
    }
    if !shift {
        app.points.clear();
    }
    if selected_shapes(app).is_empty() { Drag::Marquee { start: pos2(sp.x, sp.y), cur: pos2(sp.x, sp.y) } } else { Drag::PointMarquee { start: pos2(sp.x, sp.y), cur: pos2(sp.x, sp.y) } }
}

/// Moves the selected points by a stage-space delta (inside the open gesture).
pub fn move_points(app: &mut App, d: Vec2) {
    let mut ids: Vec<ElementId> = app.points.iter().map(|p| p.0).collect();
    ids.dedup();
    for id in ids {
        let pts: Vec<[u64; 2]> = app.points.iter().filter(|p| p.0 == id).map(|p| [p.1 as u64, u64::from(p.2)]).collect();
        app.run("path.move_points", json!({"id": id.0, "points": pts, "dx": d.x, "dy": d.y}));
    }
}

/// Selects the anchors inside a stage-space marquee.
pub fn marquee_points(app: &mut App, a: Pos2, b: Pos2, shift: bool) {
    let r = Rect::from_two_pos(a, b);
    if !shift {
        app.points.clear();
    }
    for (id, path, m) in selected_shapes(app) {
        for (cmd, q) in path.editable_anchors() {
            let s = m.apply(q);
            if r.contains(pos2(s.x, s.y)) && !app.points.contains(&(id, cmd, ANCHOR)) {
                app.points.push((id, cmd, ANCHOR));
            }
        }
    }
}

/// Click with the Subselection tool: a point toggles into the point selection, a shape is
/// selected (showing its points), empty space clears both.
pub fn subselect_click(app: &mut App, at: Pos2, shift: bool, xf: Xf) {
    if let Some(pt) = point_at(app, at, xf) {
        if shift {
            if let Some(i) = app.points.iter().position(|p| *p == pt) {
                app.points.remove(i);
            } else {
                app.points.push(pt);
            }
        } else {
            app.points = vec![pt];
        }
        return;
    }
    let sp = xf.to_stage(at);
    app.run("element.pick", json!({"x": sp.x, "y": sp.y, "add": shift}));
    app.points.clear();
}

/// Double-click on a selected shape's outline adds a point there and selects it.
pub fn subselect_double_click(app: &mut App, at: Pos2, xf: Xf) {
    add_point_on_selected(app, at, xf);
}

/// Adds a point where screen point `at` is on a selected shape's outline and selects it (the
/// Subselection double-click, the Pen's Alt-click). False when it wasn't on one.
fn add_point_on_selected(app: &mut App, at: Pos2, xf: Xf) -> bool {
    let sp = xf.to_stage(at);
    for (id, _, _) in selected_shapes(app) {
        let r = app.s.execute("path.add_point", &json!({"id": id.0, "x": sp.x, "y": sp.y, "slop": PICK * xf.unit()}));
        if let Ok(v) = r {
            app.points = v["cmd"].as_u64().map(|c| vec![(id, c as usize, ANCHOR)]).unwrap_or_default();
            app.motion_anchors.clear();
            app.status_ok(format!("Added a point. {} to undo.", crate::platform::shortcut_label("Ctrl+Z")));
            return true;
        }
    }
    false
}

/// Anchors in the point selection, grouped per element.
fn selected_anchors(app: &App) -> Vec<(ElementId, Vec<u64>)> {
    let mut out: Vec<(ElementId, Vec<u64>)> = Vec::new();
    for &(id, cmd, slot) in &app.points {
        if slot != ANCHOR {
            continue;
        }
        match out.iter_mut().find(|(e, _)| *e == id) {
            Some((_, v)) => v.push(cmd as u64),
            None => out.push((id, vec![cmd as u64])),
        }
    }
    out
}

pub fn delete_points(app: &mut App) {
    for (id, cmds) in selected_anchors(app) {
        app.run("path.delete_points", json!({"id": id.0, "cmds": cmds}));
    }
    app.points.clear();
}

pub fn convert_points(app: &mut App, smooth: bool) {
    for (id, cmds) in selected_anchors(app) {
        for cmd in cmds {
            app.run("path.convert_point", json!({"id": id.0, "cmd": cmd, "smooth": smooth}));
        }
    }
}

/// Right-click menu for selected points; false when no anchor is selected.
pub fn point_menu(app: &mut App, ui: &mut egui::Ui) -> bool {
    if app.tool != Tool::Subselect || selected_anchors(app).is_empty() {
        return false;
    }
    if ui.button("Make Smooth").clicked() {
        convert_points(app, true);
        ui.close();
    }
    if ui.button("Make Corner").clicked() {
        convert_points(app, false);
        ui.close();
    }
    ui.separator();
    if ui.button("Delete Point").clicked() {
        delete_points(app);
        ui.close();
    }
    true
}

// ----- keys ----------------------------------------------------------------------------------

/// Keys the path tools own while they are busy, taken before the global shortcuts (so Enter
/// finishes a path instead of starting playback, and Delete removes points, not the shape).
pub fn consume_keys(app: &mut App, ctx: &egui::Context) {
    let take = |k: Key| ctx.input_mut(|i| i.consume_key(Modifiers::NONE, k));
    if app.pen.is_some() {
        if take(Key::Enter) || take(Key::Escape) {
            commit_pen(app, false);
        }
        if (take(Key::Backspace) || take(Key::Delete))
            && let Some(pen) = app.pen.as_mut()
        {
            pen.knots.pop();
            if pen.knots.is_empty() {
                app.pen = None;
            }
        }
    }
    if app.tool == Tool::Subselect && !app.points.is_empty() {
        if take(Key::Delete) || take(Key::Backspace) {
            delete_points(app);
        }
        if take(Key::Escape) {
            app.points.clear();
        }
    }
}

// ----- drawing -------------------------------------------------------------------------------

fn bezier(p0: Vec2, c0: Vec2, c1: Vec2, p1: Vec2, t: f32) -> Vec2 {
    let u = 1.0 - t;
    p0 * (u * u * u) + c0 * (3.0 * u * u * t) + c1 * (3.0 * u * t * t) + p1 * (t * t * t)
}

fn anchor_box(painter: &egui::Painter, c: Pos2, filled: bool) {
    let r = Rect::from_center_size(c, vec2(6.0, 6.0));
    painter.rect_filled(r, 0.0, if filled { T.selection } else { Color32::WHITE });
    painter.rect_stroke(r, 0.0, Stroke::new(1.0, T.selection), StrokeKind::Middle);
}

/// Path tool overlays: point editing on selected shapes, the Pen path being drawn and the
/// Pencil stroke in progress.
pub fn draw_overlay(app: &App, painter: &egui::Painter, xf: Xf, hover: Option<Pos2>) {
    let line = Stroke::new(1.0, T.selection);
    if app.tool == Tool::Subselect {
        for (id, path, m) in selected_shapes(app) {
            let to_px = Affine::translate(xf.origin.x, xf.origin.y).mul(&Affine::scale(xf.zoom, xf.zoom)).mul(&m);
            for pl in path.flatten(&to_px, 0.35) {
                let pts: Vec<Pos2> = pl.points.iter().map(|p| pos2(p.x, p.y)).collect();
                painter.add(if pl.closed { egui::Shape::closed_line(pts, line) } else { egui::Shape::line(pts, line) });
            }
            let sel = app.points.iter().filter(|p| p.0 == id && p.2 == ANCHOR).map(|p| p.1);
            for (cmd, slot, anchor) in shown_handles(&path, sel) {
                if let (Some(h), Some(a)) = (path.point(cmd, slot), path.point(anchor, ANCHOR)) {
                    let (hs, as_) = (xf.to_screen(m.apply(h)), xf.to_screen(m.apply(a)));
                    painter.line_segment([as_, hs], line);
                    let picked = app.points.contains(&(id, cmd, slot));
                    painter.circle(hs, 3.0, if picked { T.selection } else { Color32::WHITE }, line);
                }
            }
            for (cmd, q) in path.editable_anchors().into_iter().take(20_000) {
                anchor_box(painter, xf.to_screen(m.apply(q)), app.points.contains(&(id, cmd, ANCHOR)));
            }
        }
    }
    if let Some(pen) = &app.pen {
        let stroke = Stroke::new(1.5, T.selection);
        let mut pts = Vec::new();
        for w in pen.knots.windows(2) {
            for k in 0..=16 {
                pts.push(xf.to_screen(bezier(w[0][0], w[0][2], w[1][1], w[1][0], k as f32 / 16.0)));
            }
        }
        painter.add(egui::Shape::line(pts, stroke));
        let first = pen.knots.first().map(|k| xf.to_screen(k[0]));
        if let (Some(last), Some(h)) = (pen.knots.last(), hover)
            && !pen.dragging
        {
            painter.add(egui::Shape::dashed_line(&[xf.to_screen(last[0]), h], line, 4.0, 3.0));
            // Closing cue: a ring on the first point when a click would close the path.
            if pen.knots.len() >= 2
                && let Some(f) = first
                && f.distance(h) <= PICK
            {
                painter.circle_stroke(f, 6.0, Stroke::new(1.5, T.selection));
            }
        }
        for (i, k) in pen.knots.iter().enumerate() {
            let c = xf.to_screen(k[0]);
            let last = i + 1 == pen.knots.len();
            if last && k[2] != k[0] {
                for hnd in [k[1], k[2]] {
                    let hp = xf.to_screen(hnd);
                    painter.line_segment([c, hp], line);
                    painter.circle_filled(hp, 3.0, T.selection);
                }
            }
            anchor_box(painter, c, last);
        }
    }
    if app.drag == Drag::Pencil && app.pencil.len() > 1 {
        let w = (app.s.style.stroke_width * xf.zoom).max(1.0);
        let pts: Vec<Pos2> = app.pencil.iter().map(|p| xf.to_screen(*p)).collect();
        painter.add(egui::Shape::line(pts, Stroke::new(w, crate::theme::to_color32(app.s.style.stroke))));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn knot(x: f32, y: f32) -> [Vec2; 3] {
        [Vec2::new(x, y); 3]
    }

    #[test]
    fn pen_svg_is_straight_for_corners_and_curved_for_handles() {
        let mut pen = PenPath { knots: vec![knot(0.0, 0.0), knot(100.0, 0.0), knot(100.0, 50.0)], dragging: false };
        assert_eq!(pen.to_svg(false), "M0 0 L100 0 L100 50");
        assert_eq!(pen.to_svg(true), "M0 0 L100 0 L100 50 Z");
        pen.knots[1] = [Vec2::new(100.0, 0.0), Vec2::new(80.0, -10.0), Vec2::new(120.0, 10.0)];
        let d = pen.to_svg(false);
        assert!(d.contains(" C0 0 80 -10 100 0") && d.contains(" C120 10 100 50 100 50"), "{d}");
        // The engine takes it.
        let mut s = anicraft_engine::Session::default();
        assert!(s.execute("draw.path", &json!({"d": pen.to_svg(true)})).is_ok());
    }

    #[test]
    fn handles_shown_for_a_selected_smooth_anchor() {
        let mut p = Path::new();
        p.move_to(Vec2::new(0.0, 0.0)).line_to(Vec2::new(10.0, 10.0)).line_to(Vec2::new(20.0, 0.0));
        assert!(shown_handles(&p, [1usize].into_iter()).is_empty(), "lines have no handles");
        assert!(p.make_smooth(1));
        let h = shown_handles(&p, [1usize].into_iter());
        assert_eq!(h, vec![(1, C2, 1), (2, C1, 1)]);
    }
}
