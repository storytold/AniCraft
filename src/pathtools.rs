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
use anicraft_engine::cmds_rawsub::{Handle, RawPoints};
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
        // A new knot snaps to points, lines and fill edges (ADR 0045); handles never do.
        let p = crate::stage::snap_draw(app, pos2(p.x, p.y), xf.zoom);
        let p = Vec2::new(p.x, p.y);
        let Some(pen) = app.pen.as_mut() else { return };
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
    app.draw("draw.path", json!({"d": pen.to_svg(closed)}));
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
    /// The English name, translated where it is drawn (`crate::i18n::t`).
    pub fn label(self) -> &'static str {
        match self {
            PencilMode::Smooth => tr_noop!("Smooth"),
            PencilMode::Straighten => tr_noop!("Straighten"),
            PencilMode::Ink => tr_noop!("Ink"),
        }
    }
    /// The English hint, translated where it is drawn (`crate::i18n::t`).
    pub fn hint(self) -> &'static str {
        match self {
            PencilMode::Smooth => tr_noop!("Fits smooth curves to the stroke, keeping sharp corners."),
            PencilMode::Straighten => tr_noop!("Turns the stroke into straight segments."),
            PencilMode::Ink => tr_noop!("Follows the hand closely with little smoothing."),
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
    app.draw("draw.freehand", json!({"points": points, "mode": app.pencil_mode.id(), "tolerance": 1.5 * xf.unit()}));
}

// ----- Subselection --------------------------------------------------------------------------

/// A selected point: element, command, slot.
pub type PointRef = (ElementId, usize, u8);

/// Selected shapes on the active layer with their path and transform.
fn selected_shapes(app: &App) -> Vec<(ElementId, Arc<Path>, Affine)> {
    app.s
        .visible_elements()
        .into_iter()
        .filter(|e| app.s.selection.contains(app.s.layer, e.id))
        .filter_map(|e| match &e.kind {
            // Raw shapes show their shared points instead (ADR 0042, below).
            ElementKind::Shape(sh) if sh.merge => None,
            ElementKind::Shape(sh) => Some((e.id, sh.path.clone(), e.transform)),
            ElementKind::Symbol { .. } | ElementKind::Text(_) | ElementKind::Camera | ElementKind::Group { .. } => None,
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
    raw_upkeep(app);
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
    if let Some(d) = raw_drag_start(app, press, shift, xf) {
        return d;
    }
    if let Some(pt) = point_at(app, press, xf) {
        if !app.points.contains(&pt) {
            if !shift {
                app.points.clear();
            }
            app.points.push(pt);
        }
        // Undo labels stay English in the history and are translated where they show.
        app.s.begin_gesture(tr_noop!("Move Points"));
        return Drag::Points { last: pos2(sp.x, sp.y) };
    }
    if let Some((layer, el)) = app.s.hit_test(sp, 3.0 * xf.unit()) {
        if !app.s.selection.contains(layer, el) {
            app.run("element.pick", json!({"x": sp.x, "y": sp.y, "add": shift}));
            app.points.clear();
        }
        app.s.begin_gesture(tr_noop!("Move"));
        return Drag::Move { last: pos2(sp.x, sp.y) };
    }
    if !shift {
        app.points.clear();
        app.raw_anchors.clear();
        app.raw_ends.clear();
    }
    // A marquee shows the points of the shapes it touches on the active layer and selects the
    // points inside it; it never cuts (that is the Selection tool's).
    Drag::PointMarquee { start: pos2(sp.x, sp.y), cur: pos2(sp.x, sp.y) }
}

/// Moves the selected points by a stage-space delta (inside the open gesture). A handle moved
/// alone at a smooth anchor turns the other one with it unless `smooth` is false (Alt).
pub fn move_points(app: &mut App, d: Vec2, smooth: bool) {
    let mut ids: Vec<ElementId> = app.points.iter().map(|p| p.0).collect();
    ids.dedup();
    for id in ids {
        let pts: Vec<[u64; 2]> = app.points.iter().filter(|p| p.0 == id).map(|p| [p.1 as u64, u64::from(p.2)]).collect();
        app.run("path.move_points", json!({"id": id.0, "points": pts, "dx": d.x, "dy": d.y, "smooth": smooth}));
    }
}

/// The Subselection marquee: the shapes it touches on the active layer are selected whole (raw
/// ones with everything connected to them, ADR 0042) and the anchors inside it are selected.
pub fn marquee_points(app: &mut App, a: Pos2, b: Pos2, shift: bool) {
    let r = Rect::from_two_pos(a, b);
    if !shift {
        app.points.clear();
        app.raw_anchors.clear();
        app.raw_ends.clear();
    }
    if let Some(v) = app.run("raw.subselect_rect", json!({"x0": a.x, "y0": a.y, "x1": b.x, "y1": b.y, "add": shift})) {
        for q in positions(&v["anchors"]) {
            if !app.raw_anchors.iter().any(|p| p.distance(q) <= SNAP) {
                app.raw_anchors.push(q);
            }
        }
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
    if raw_click(app, at, shift, xf) {
        return;
    }
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
    app.raw_anchors.clear();
    app.raw_ends.clear();
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
            app.status_ok(tr!("Added a point. {undo} to undo.", undo = app.undo_key()));
            return true;
        }
    }
    raw_add_point(app, at, xf)
}

/// Adds a point on the raw line or fill edge at screen point `at` (ADR 0045): every raw shape with
/// that edge gets it, and it shows selected with everything connected to it. False when there
/// is no raw edge there.
fn raw_add_point(app: &mut App, at: Pos2, xf: Xf) -> bool {
    let sp = xf.to_stage(at);
    let Ok(v) = app.s.execute("raw.add_point", &json!({"x": sp.x, "y": sp.y, "slop": PICK * xf.unit()})) else { return false };
    let q = Vec2::new(v["x"].as_f64().unwrap_or(0.0) as f32, v["y"].as_f64().unwrap_or(0.0) as f32);
    // Show the shapes on it, as a Subselection click there would.
    if let Some(r) = app.run("raw.subselect", json!({"x": q.x, "y": q.y, "slop": 2.0 * xf.unit()})) {
        app.raw_ends = positions(&r["ends"]);
    }
    app.points.clear();
    app.motion_anchors.clear();
    app.raw_anchors = vec![q];
    app.status_ok(tr!("Added a point. {undo} to undo.", undo = app.undo_key()));
    true
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

/// Deletes the selected raw anchors (ADR 0045): the shapes that share each are joined across it.
fn raw_delete(app: &mut App) {
    let pts: Vec<[f32; 2]> = app.raw_anchors.iter().map(|q| [q.x, q.y]).collect();
    let Some(v) = app.run("raw.delete_points", json!({"points": pts})) else { return };
    app.raw_anchors.clear();
    app.raw_ends.clear();
    let (n, kept) = (v["deleted"].as_u64().unwrap_or(0) as usize, v["kept"].as_u64().unwrap_or(0) as usize);
    let undo = app.undo_key();
    app.status_ok(if kept > 0 {
        trn!("Deleted {n} point; kept {kept} where three or more lines meet. {undo} to undo.", "Deleted {n} points; kept {kept} where three or more lines meet. {undo} to undo.", n, kept = kept, undo = undo)
    } else {
        trn!("Deleted {n} point. {undo} to undo.", "Deleted {n} points. {undo} to undo.", n, undo = undo)
    });
}

/// Makes the selected raw anchors smooth or corners, in every shape that has them.
fn raw_convert(app: &mut App, smooth: bool) {
    let pts: Vec<[f32; 2]> = app.raw_anchors.iter().map(|q| [q.x, q.y]).collect();
    app.run("raw.convert_points", json!({"points": pts, "smooth": smooth}));
}

pub fn convert_points(app: &mut App, smooth: bool) {
    for (id, cmds) in selected_anchors(app) {
        for cmd in cmds {
            app.run("path.convert_point", json!({"id": id.0, "cmd": cmd, "smooth": smooth}));
        }
    }
}

/// Right-click menu for selected points (of drawing objects, or raw anchors, ADR 0045); false
/// when no anchor is selected.
pub fn point_menu(app: &mut App, ui: &mut egui::Ui) -> bool {
    let raw = !app.raw_anchors.is_empty();
    if app.tool != Tool::Subselect || (!raw && selected_anchors(app).is_empty()) {
        return false;
    }
    if ui.button(tr!("Make Smooth")).clicked() {
        if raw { raw_convert(app, true) } else { convert_points(app, true) }
        ui.close();
    }
    if ui.button(tr!("Make Corner")).clicked() {
        if raw { raw_convert(app, false) } else { convert_points(app, false) }
        ui.close();
    }
    ui.separator();
    let label = if raw { trn!("Delete Point", "Delete Points", app.raw_anchors.len()) } else { tr!("Delete Point").to_string() };
    if ui.button(label).clicked() {
        if raw { raw_delete(app) } else { delete_points(app) }
        ui.close();
    }
    true
}

// ----- Subselection on raw shapes (ADR 0042) -------------------------------------------------

/// Two positions this close are one shared point.
const SNAP: f32 = anicraft_engine::geom::planar::SNAP;
/// How near (screen pixels) a press takes an anchor of a raw shape that isn't shown yet.
const GRAB_POINT: f32 = 6.0;

fn is_raw(e: &anicraft_engine::doc::Element) -> bool {
    matches!(&e.kind, ElementKind::Shape(sh) if sh.merge)
}

/// The selected raw shapes on the active layer: Subselection shows their points.
fn raw_selected(app: &App) -> Vec<ElementId> {
    app.s.visible_elements().iter().filter(|e| app.s.selection.contains(app.s.layer, e.id) && is_raw(e)).map(|e| e.id).collect()
}

/// The anchors and handles of the selected raw shapes.
pub fn raw_shown(app: &App) -> RawPoints {
    let ids = raw_selected(app);
    if ids.is_empty() { RawPoints::default() } else { app.s.raw_points(&ids) }
}

/// The handles that show: those of the selected anchors' curves and of the clicked edge.
fn visible_handles<'a>(app: &'a App, pts: &'a RawPoints) -> impl Iterator<Item = &'a Handle> + 'a {
    pts.handles.iter().filter(|h| app.raw_anchors.iter().chain(&app.raw_ends).any(|a| a.distance(h.anchor) <= SNAP))
}

fn positions(v: &serde_json::Value) -> Vec<Vec2> {
    v.as_array().into_iter().flatten().filter_map(|p| Some(Vec2::new(p.get(0)?.as_f64()? as f32, p.get(1)?.as_f64()? as f32))).collect()
}

/// What of the shown raw points is under a screen position: a handle (on top), else an anchor.
enum RawHit {
    Handle(Handle),
    Anchor(Vec2),
}

fn raw_point_at(app: &App, pts: &RawPoints, at: Pos2, xf: Xf) -> Option<RawHit> {
    let d = |q: Vec2| xf.to_screen(q).distance(at);
    if let Some(h) = visible_handles(app, pts).filter(|h| d(h.at) <= PICK).min_by(|a, b| d(a.at).total_cmp(&d(b.at))) {
        return Some(RawHit::Handle(*h));
    }
    pts.anchors.iter().copied().filter(|q| d(*q) <= PICK).min_by(|a, b| d(*a).total_cmp(&d(*b))).map(RawHit::Anchor)
}

/// Keeps the selected anchors on the points that are still there (after a drag merged, undo, or
/// a frame change), and drops them with their shapes.
fn raw_upkeep(app: &mut App) {
    if app.raw_anchors.is_empty() && app.raw_ends.is_empty() {
        return;
    }
    // A drag moves them itself.
    if matches!(app.drag, Drag::RawPoint { .. } | Drag::RawHandle { .. } | Drag::SnapMove { .. }) {
        return;
    }
    let pts = raw_shown(app);
    let settle = |list: &mut Vec<Vec2>| {
        let mut out: Vec<Vec2> = Vec::with_capacity(list.len());
        for q in list.iter() {
            // A merge may have met it with a point nearby (at most `SNAP` away, ADR 0041).
            if let Some(p) = pts.anchors.iter().copied().filter(|p| p.distance(*q) <= 2.5 * SNAP).min_by(|a, b| a.distance(*q).total_cmp(&b.distance(*q)))
                && !out.contains(&p)
            {
                out.push(p);
            }
        }
        *list = out;
    };
    settle(&mut app.raw_anchors);
    settle(&mut app.raw_ends);
}

/// Shows the connected raw shapes at stage point `sp` (their layer becomes the active one).
/// False when there is no raw shape there.
fn raw_show_at(app: &mut App, sp: Vec2, shift: bool, xf: Xf) -> bool {
    let slop = 3.0 * xf.unit();
    let Some((layer, id)) = app.s.hit_test(sp, slop) else { return false };
    // On the active layer, look first; another layer's shape is for the engine to say.
    if layer == app.s.layer && !app.s.visible_elements().iter().any(|e| e.id == id && is_raw(e)) {
        return false;
    }
    let Some(v) = app.run("raw.subselect", json!({"x": sp.x, "y": sp.y, "add": shift, "slop": slop})) else { return false };
    if v["raw"] != json!(true) {
        return false;
    }
    if !shift {
        app.raw_anchors.clear();
    }
    app.raw_ends = positions(&v["ends"]);
    app.points.clear();
    true
}

/// A press with Subselection on raw shapes: a shown handle bends its curve, a shown anchor moves
/// (with the other selected anchors), an anchor of a raw shape not shown yet shows its shapes and
/// moves, and a raw line or fill shows its connected shapes and moves all of them. `None` when
/// the press is not on raw artwork.
fn raw_drag_start(app: &mut App, press: Pos2, shift: bool, xf: Xf) -> Option<Drag> {
    let sp = xf.to_stage(press);
    let pts = raw_shown(app);
    let anchor = match raw_point_at(app, &pts, press, xf) {
        Some(RawHit::Handle(h)) => {
            app.s.begin_gesture(tr_noop!("Move Handle"));
            app.raw_touched.clear();
            return Some(Drag::RawHandle { id: h.id, cmd: h.cmd, slot: h.slot, last: pos2(sp.x, sp.y) });
        }
        Some(RawHit::Anchor(q)) => Some(q),
        // An unselected raw shape's corner or end: show its shapes, then take the point.
        None => match app.s.raw_grab(sp, 3.0 * xf.unit(), GRAB_POINT * xf.unit()) {
            Some((_, anicraft_engine::cmds_raw::Grab::Point { at })) if raw_show_at(app, at, false, xf) => Some(at),
            _ => None,
        },
    };
    if let Some(q) = anchor {
        if !app.raw_anchors.iter().any(|p| p.distance(q) <= SNAP) {
            if !shift {
                app.raw_anchors.clear();
            }
            app.raw_anchors.push(q);
        }
        app.raw_with = app.raw_anchors.iter().copied().filter(|p| p.distance(q) > SNAP).collect();
        app.raw_points = serde_json::Value::Null;
        app.raw_touched.clear();
        app.s.begin_gesture(tr_noop!("Move Points"));
        return Some(Drag::RawPoint { at: pos2(q.x, q.y), off: pos2(q.x, q.y) - pos2(sp.x, sp.y) });
    }
    // A raw line or fill: everything connected to it moves (Animate's Subselection drag).
    let on_shown = app.s.hit_test(sp, 3.0 * xf.unit()).is_some_and(|(l, id)| l == app.s.layer && app.s.selection.contains(l, id) && app.s.visible_elements().iter().any(|e| e.id == id && is_raw(e)));
    if !on_shown && !raw_show_at(app, sp, shift, xf) {
        return None;
    }
    app.s.begin_gesture(tr_noop!("Move"));
    Some(Drag::SnapMove { start: pos2(sp.x, sp.y), applied: egui::Vec2::ZERO })
}

/// A click with Subselection on raw shapes: an anchor toggles into the selection (Shift adds or
/// removes it), a raw line or fill shows everything connected to it. False when it wasn't on
/// raw artwork.
fn raw_click(app: &mut App, at: Pos2, shift: bool, xf: Xf) -> bool {
    let pts = raw_shown(app);
    match raw_point_at(app, &pts, at, xf) {
        Some(RawHit::Anchor(q)) => {
            match app.raw_anchors.iter().position(|p| p.distance(q) <= SNAP) {
                Some(i) if shift => {
                    app.raw_anchors.remove(i);
                }
                _ if shift => app.raw_anchors.push(q),
                _ => app.raw_anchors = vec![q],
            }
            true
        }
        Some(RawHit::Handle(_)) => true,
        None => raw_show_at(app, xf.to_stage(at), shift, xf),
    }
}

/// The selected raw anchors moved by the arrow keys: every shape sharing them moves too, and
/// they merge where they land (points within 0.2 px meet, ADR 0041). One undo step a press.
fn raw_nudge(app: &mut App, d: Vec2) {
    let Some((&first, rest)) = app.raw_anchors.split_first() else { return };
    let with: Vec<[f32; 2]> = rest.iter().map(|q| [q.x, q.y]).collect();
    let p = json!({"x": first.x, "y": first.y, "to_x": first.x + d.x, "to_y": first.y + d.y, "with": with});
    if app.run("raw.move_point", p).is_some() {
        for q in app.raw_anchors.iter_mut().chain(app.raw_ends.iter_mut()) {
            *q = *q + d;
        }
    }
}

/// While raw points are dragged, the selected anchors follow by `d` (the move this frame), and
/// the clicked edge's ends too when they move (`all`: the whole drawing moves).
pub fn raw_follow(app: &mut App, d: Vec2, all: bool) {
    if d == Vec2::ZERO {
        return;
    }
    let moving = app.raw_anchors.clone();
    for q in app.raw_anchors.iter_mut() {
        *q = *q + d;
    }
    for q in app.raw_ends.iter_mut().filter(|q| all || moving.iter().any(|m| m.distance(**q) <= SNAP)) {
        *q = *q + d;
    }
}

/// The raw shapes' outlines, handles and anchors (hollow squares; selected ones solid).
fn draw_raw(app: &App, painter: &egui::Painter, xf: Xf) {
    let ids = raw_selected(app);
    if ids.is_empty() {
        return;
    }
    let line = Stroke::new(1.0, T.selection);
    let to_px = Affine::translate(xf.origin.x, xf.origin.y).mul(&Affine::scale(xf.zoom, xf.zoom));
    for e in app.s.visible_elements().iter().filter(|e| ids.contains(&e.id)) {
        let ElementKind::Shape(sh) = &e.kind else { continue };
        for pl in sh.path.flatten(&to_px.mul(&e.transform), 0.35) {
            let pts: Vec<Pos2> = pl.points.iter().map(|p| pos2(p.x, p.y)).collect();
            painter.add(if pl.closed { egui::Shape::closed_line(pts, line) } else { egui::Shape::line(pts, line) });
        }
    }
    let pts = app.s.raw_points(&ids);
    for h in visible_handles(app, &pts) {
        let (hs, a) = (xf.to_screen(h.at), xf.to_screen(h.anchor));
        painter.line_segment([a, hs], line);
        painter.circle_filled(hs, 3.0, T.selection);
    }
    for q in pts.anchors.iter().take(20_000) {
        anchor_box(painter, xf.to_screen(*q), app.raw_anchors.iter().any(|p| p.distance(*q) <= SNAP));
    }
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
        if let Some(d) = arrow_nudge(ctx, app.view.zoom) {
            move_points_step(app, d);
        }
    }
    if app.tool == Tool::Subselect && !app.raw_anchors.is_empty() {
        if take(Key::Delete) || take(Key::Backspace) {
            raw_delete(app);
        }
        if take(Key::Escape) {
            app.raw_anchors.clear();
        }
        if let Some(d) = arrow_nudge(ctx, app.view.zoom) {
            raw_nudge(app, d);
        }
    }
}

/// An arrow key press (taken): 1 screen pixel, Shift 10, as Animate's (issue #42 section 4).
fn arrow_nudge(ctx: &egui::Context, zoom: f32) -> Option<Vec2> {
    let mut d = Vec2::ZERO;
    ctx.input_mut(|i| {
        for (k, v) in [(Key::ArrowLeft, Vec2::new(-1.0, 0.0)), (Key::ArrowRight, Vec2::new(1.0, 0.0)), (Key::ArrowUp, Vec2::new(0.0, -1.0)), (Key::ArrowDown, Vec2::new(0.0, 1.0))] {
            // Shift first: a plain chord also matches a press with Shift held.
            for (m, step) in [(Modifiers::SHIFT, 10.0), (Modifiers::NONE, 1.0)] {
                while i.consume_key(m, k) {
                    d = d + v * (step / zoom.max(1e-3));
                }
            }
        }
    });
    (d != Vec2::ZERO).then_some(d)
}

/// Selected points of drawing objects nudged by the arrow keys: one undo step.
fn move_points_step(app: &mut App, d: Vec2) {
    let own = !app.s.in_gesture();
    if own {
        app.s.begin_gesture(tr_noop!("Move Points"));
    }
    move_points(app, d, true);
    if own {
        app.s.end_gesture();
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
        draw_raw(app, painter, xf);
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

    /// Subselection on a merged drawing (ADR 0042), on a headless app: a raw rectangle with
    /// its outline at the stage origin, stage and screen coordinates the same.
    mod raw {
        use super::*;

        const XF: Xf = Xf { origin: Pos2::ZERO, zoom: 1.0 };

        fn app_and_ctx() -> (App, egui::Context) {
            let ctx = egui::Context::default();
            let session = anicraft_engine::Session::new(anicraft_engine::doc::Document::default());
            let mut app = App::with_context(&ctx, session, Some(std::path::PathBuf::from("unused.png")));
            app.run("draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 80, "fill": "#ff0000", "stroke": "#000000", "object": false}));
            app.run("edit.deselect", json!({}));
            app.tool = Tool::Subselect;
            app.view.zoom = 1.0;
            (app, ctx)
        }

        fn has(list: &[Vec2], x: f32, y: f32) -> bool {
            list.iter().any(|q| q.distance(Vec2::new(x, y)) < 1e-2)
        }

        #[test]
        fn a_click_shows_the_connected_points_and_selects_anchors() {
            let (mut app, _) = app_and_ctx();
            subselect_click(&mut app, pos2(50.0, 0.0), false, XF);
            assert_eq!(app.s.selection.len(), 5, "the fill and its four sides");
            assert_eq!(raw_shown(&app).anchors.len(), 4);
            assert!(has(&app.raw_ends, 0.0, 0.0) && has(&app.raw_ends, 100.0, 0.0), "{:?}", app.raw_ends);
            subselect_click(&mut app, pos2(100.0, 0.0), false, XF);
            assert_eq!(app.raw_anchors.len(), 1);
            subselect_click(&mut app, pos2(0.0, 0.0), true, XF);
            assert_eq!(app.raw_anchors.len(), 2, "Shift adds");
            subselect_click(&mut app, pos2(0.0, 0.0), true, XF);
            assert_eq!(app.raw_anchors.len(), 1, "and removes");
            subselect_click(&mut app, pos2(500.0, 500.0), false, XF);
            assert!(app.s.selection.is_empty() && app.raw_anchors.is_empty());
        }

        #[test]
        fn arrow_keys_move_the_selected_anchors_with_every_shape_on_them() {
            let (mut app, ctx) = app_and_ctx();
            subselect_click(&mut app, pos2(50.0, 0.0), false, XF);
            subselect_click(&mut app, pos2(100.0, 0.0), false, XF);
            subselect_click(&mut app, pos2(0.0, 0.0), true, XF);
            let mut input = egui::RawInput::default();
            input.events.push(egui::Event::Key { key: Key::ArrowUp, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::SHIFT });
            let mut out = ctx.run_ui(input, |ui| consume_keys(&mut app, ui.ctx()));
            out.textures_delta.clear();
            assert!(has(&app.raw_anchors, 0.0, -10.0) && has(&app.raw_anchors, 100.0, -10.0), "{:?}", app.raw_anchors);
            upkeep(&mut app);
            assert_eq!(app.raw_anchors.len(), 2, "still on the moved points");
            let pts = raw_shown(&app);
            assert!(has(&pts.anchors, 0.0, -10.0) && has(&pts.anchors, 100.0, 80.0) && pts.anchors.len() == 4, "{:?}", pts.anchors);
            assert!(app.s.can_undo());
        }

        #[test]
        fn delete_removes_selected_raw_anchors_and_a_double_click_adds_one() {
            let (mut app, ctx) = app_and_ctx();
            subselect_click(&mut app, pos2(50.0, 0.0), false, XF);
            subselect_click(&mut app, pos2(0.0, 0.0), false, XF);
            assert_eq!(app.raw_anchors.len(), 1);
            let mut input = egui::RawInput::default();
            input.events.push(egui::Event::Key { key: Key::Delete, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
            let mut out = ctx.run_ui(input, |ui| consume_keys(&mut app, ui.ctx()));
            out.textures_delta.clear();
            assert!(app.raw_anchors.is_empty());
            let pts = raw_shown(&app);
            assert_eq!(pts.anchors.len(), 3, "a triangle: {:?}", pts.anchors);
            assert!(!has(&pts.anchors, 0.0, 0.0));
            // A double-click on the right side adds a point there, shared and selected.
            subselect_double_click(&mut app, pos2(101.0, 40.0), XF);
            assert!(has(&app.raw_anchors, 100.0, 40.0), "{:?}", app.raw_anchors);
            assert_eq!(raw_shown(&app).anchors.len(), 4);
            // Make Smooth reaches the engine for raw anchors: the corner at (100, 80) rounds.
            app.raw_anchors = vec![Vec2::new(100.0, 80.0)];
            raw_convert(&mut app, true);
            assert!(!raw_shown(&app).handles.is_empty(), "smooth: handles appear");
        }

        #[test]
        fn a_press_takes_an_anchor_or_moves_everything_connected() {
            let (mut app, _) = app_and_ctx();
            // An anchor of a drawing not shown yet: it shows, and the point is taken.
            assert!(matches!(subselect_drag_start(&mut app, pos2(100.0, 80.0), false, XF), Drag::RawPoint { .. }));
            assert_eq!(app.s.selection.len(), 5);
            assert!(has(&app.raw_anchors, 100.0, 80.0));
            app.s.end_gesture();
            // With another anchor selected, Shift: both move.
            assert!(matches!(subselect_drag_start(&mut app, pos2(0.0, 80.0), true, XF), Drag::RawPoint { .. }));
            assert_eq!(app.raw_with.len(), 1);
            app.s.end_gesture();
            // Inside the fill: the whole connected drawing moves.
            assert!(matches!(subselect_drag_start(&mut app, pos2(50.0, 40.0), false, XF), Drag::SnapMove { .. }));
            app.s.end_gesture();
            // Empty space: a marquee, which selects the anchors inside it.
            assert!(matches!(subselect_drag_start(&mut app, pos2(300.0, 300.0), false, XF), Drag::PointMarquee { .. }));
            marquee_points(&mut app, pos2(90.0, -10.0), pos2(120.0, 10.0), false);
            assert_eq!(app.s.selection.len(), 5);
            assert_eq!(app.raw_anchors.len(), 1);
            assert!(has(&app.raw_anchors, 100.0, 0.0));
        }
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
