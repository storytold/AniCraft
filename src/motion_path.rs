//! Motion paths on the stage (ADR 0030, 0038). When a motion tween's object is selected, or a
//! frame of its span is, the stage shows the path its registration point travels, in the motion
//! colour, with a dot on every frame (larger on position keys).
//!
//! * **Selection (V)**: drag the path to bend it, or drag either end to move where the motion
//!   starts or ends. Dragging the object itself still moves it (auto-key).
//! * **Subselection (A)**: drag any anchor, or a selected anchor's handles (Alt breaks a smooth
//!   pair). A click on an anchor selects it and shows its handles (Shift adds to the selection);
//!   a double-click on the path adds an anchor there; Delete removes the selected anchors (the
//!   ends stay); right-click makes them smooth or corners.
//! * **Pen (P)**: Alt-click the path to add an anchor, as on shapes.
//! * **Ctrl-drag** (⌘ on macOS) the path or its object, with Selection or Subselection, to move
//!   the whole path; the timing stays.
//!
//! Anchors pinned to frames (ADR 0049) are diamonds, the keyframe mark, with their frame number
//! beside them; the others are squares that only shape the path. Selection takes the ends and the
//! pinned anchors; the right-click menu pins a Subselection anchor to the playhead's frame or
//! unpins it.
//!
//! A tween without a path of its own shows the straight path its X/Y curves trace; the first
//! edit makes it a path. Every change is a `motion.*` command, and a drag is one undo step (a
//! session gesture). Only the anchor selection and the drag in progress live here.

use crate::app::{App, Drag, Tool};
use crate::pathtools::Xf;
use crate::theme::{T, Type, font};
use anicraft_engine::doc::{ElementId, Pin, Props};
use anicraft_engine::geom::edit::{ANCHOR, C1, C2};
use anicraft_engine::geom::{Affine, Path, PathCmd, Vec2};
use egui::{Key, Modifiers, Painter, Pos2, Rect, Stroke, StrokeKind, pos2, vec2};
use serde_json::json;

/// Grab radius for anchors, handles and the path, in screen points.
const PICK: f32 = 6.0;

/// Most selected anchors whose handles show (a marquee of hundreds would bury the path).
const MAX_SHOWN_HANDLES: usize = 64;

/// What a press on the path took hold of.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Grab {
    /// An anchor (the path command that owns it).
    Anchor(usize),
    /// Handle `slot` (`C1`/`C2`) of command `cmd`.
    Handle(usize, u8),
    /// Segment `cmd` at curve parameter `t`.
    Segment(usize, f32),
}

/// The motion path the stage shows now.
pub struct Shown {
    /// The tween's own path, or the straight one its X/Y curves trace.
    pub path: Path,
    /// The registration point on every frame of the span.
    pub dots: Vec<Vec2>,
    /// Frames (from the span's start) with a position key.
    pub keys: Vec<u32>,
    /// The playhead's frame in the span.
    pub offset: u32,
    /// The span's first frame.
    pub start: u32,
    /// The anchors tied to frames (from the span's start), ends included, in order (ADR 0049).
    pub pins: Vec<Pin>,
    /// The object travelling it.
    pub target: ElementId,
}

impl Shown {
    /// The pin on anchor `a`, if it is tied to a frame.
    pub fn pin(&self, a: usize) -> Option<Pin> {
        self.pins.iter().find(|p| p.anchor as usize == a).copied()
    }

    /// Whether anchor `a` is an end (always pinned).
    fn is_end(&self, a: usize) -> bool {
        is_end(a, self.path.cmds.len())
    }

    /// Why anchor `a` can't be pinned to the playhead's frame, or `None` when it can.
    fn unpinnable(&self, a: usize) -> Option<String> {
        let (Some(first), Some(last)) = (self.pins.first(), self.pins.last()) else { return Some(tr!("The object doesn't move yet").into()) };
        let f = self.offset;
        if !(first.frame < f && f < last.frame) {
            return Some(tr!("Move the playhead between frames {first} and {last}, where the object moves along the path", first = self.start + first.frame + 1, last = self.start + last.frame + 1));
        }
        let a = a as u32;
        self.pins.iter().find(|p| p.frame != f && p.anchor != a && (p.frame < f) != (p.anchor < a)).map(|p| tr!("Point {anchor} is pinned to frame {frame}: pins keep the path's order", anchor = p.anchor, frame = self.start + p.frame + 1))
    }
}

/// The motion path of the active layer's motion tween at the playhead, when its object or a
/// frame of its span is selected and it moves at all.
pub fn shown(app: &App) -> Option<Shown> {
    let tl = app.s.timeline();
    let layer = app.s.active_layer().filter(|l| l.visible && l.kind == anicraft_engine::doc::LayerKind::Normal)?;
    let k = layer.keyframe_at(app.s.frame)?;
    let m = k.motion.as_ref()?;
    let target = m.target_index(&k.elements).and_then(|i| k.elements.get(i))?;
    let span_selected = app.frame_sel.is_some_and(|sel| {
        let (a, b) = sel.frames();
        sel.layers(&tl.layers).contains(&layer.id) && a < k.end() && b >= k.start
    });
    if !app.s.selection.contains(layer.id, target.id) && !span_selected {
        return None;
    }
    let base = Props::of(target);
    let path = m.path.clone().or_else(|| m.path_from_tracks(&base))?;
    let dots = m.positions(&base, k.duration);
    let mut keys: Vec<u32> = [anicraft_engine::doc::Property::X, anicraft_engine::doc::Property::Y].into_iter().filter_map(|p| m.track(p)).flat_map(|t| t.keys.iter().map(|k| k.frame)).filter(|f| *f < k.duration).collect();
    keys.sort_unstable();
    keys.dedup();
    let pins = m.pinned_anchors(&base);
    Some(Shown { path, dots, keys, offset: app.s.frame - k.start, start: k.start, pins, target: target.id })
}

/// The anchors the tool can take: the ends and the pinned anchors with Selection (the motion's
/// keyframes), every anchor with Subselection (and shown to the Pen, which adds between them).
fn grabbable_anchors(path: &Path, pins: &[Pin], tool: Tool) -> Vec<(usize, Vec2)> {
    let all = path.anchors();
    match tool {
        Tool::Subselect | Tool::Pen => all,
        _ => {
            let n = all.len();
            all.into_iter().enumerate().filter(|(i, _)| *i == 0 || *i + 1 == n || pins.iter().any(|p| p.anchor as usize == *i)).map(|(_, a)| a).collect()
        }
    }
}

/// The handles shown for the selected anchors: `(cmd, slot, anchor)`.
fn handles(path: &Path, anchors: &[usize]) -> Vec<(usize, u8, usize)> {
    let mut v = Vec::new();
    for &a in anchors.iter().take(MAX_SHOWN_HANDLES) {
        let (i, o) = path.anchor_segments(a);
        if let Some(i) = i.filter(|&i| matches!(path.cmds.get(i), Some(PathCmd::CubicTo(..)))) {
            v.push((i, C2, a));
        }
        if let Some(o) = o.filter(|&o| matches!(path.cmds.get(o), Some(PathCmd::CubicTo(..)))) {
            v.push((o, C1, a));
        }
    }
    v
}

/// The stretch of path under screen point `p`: `(cmd, t)`.
fn on_segment(shown: &Shown, p: Pos2, xf: Xf) -> Option<(usize, f32)> {
    let (cmd, t, d) = shown.path.nearest_segment(xf.to_stage(p), &Affine::IDENTITY)?;
    (d * xf.zoom <= PICK).then_some((cmd, t))
}

/// The handle, anchor or (with Selection) the stretch of path under screen point `p`.
/// `segments` is false while the press should first go to the object under it.
pub fn hit(app: &App, shown: &Shown, p: Pos2, xf: Xf, segments: bool) -> Option<Grab> {
    let near = |q: Vec2| xf.to_screen(q).distance(p) <= PICK;
    if app.tool == Tool::Subselect {
        for (cmd, slot, _) in handles(&shown.path, &app.motion_anchors) {
            if shown.path.point(cmd, slot).is_some_and(near) {
                return Some(Grab::Handle(cmd, slot));
            }
        }
    }
    if let Some((cmd, _)) = grabbable_anchors(&shown.path, &shown.pins, app.tool).into_iter().find(|(_, q)| near(*q)) {
        return Some(Grab::Anchor(cmd));
    }
    if segments && app.tool == Tool::Select {
        return on_segment(shown, p, xf).map(|(cmd, t)| Grab::Segment(cmd, t));
    }
    None
}

/// Starts dragging `grab`: one undo step until the drag ends. An anchor not yet selected becomes
/// the selection.
pub fn drag_start(app: &mut App, shown: Shown, grab: Grab, at: Vec2) -> Drag {
    if let Grab::Anchor(a) = grab
        && !app.motion_anchors.contains(&a)
    {
        app.motion_anchors = vec![a];
    }
    app.motion_drag = Some(shown.path);
    // Undo labels stay English in the history and are translated where they show.
    app.s.begin_gesture(tr_noop!("Edit Motion Path"));
    Drag::MotionPath { grab, start: pos2(at.x, at.y) }
}

/// The path `start` with `grab` dragged by `d`. A smooth anchor's handles stay in line unless
/// `alt` breaks them.
pub fn dragged(start: &Path, grab: Grab, d: Vec2, alt: bool) -> Path {
    let mut p = start.clone();
    match grab {
        Grab::Anchor(cmd) => {
            p.move_anchor(cmd, d);
        }
        Grab::Segment(cmd, t) => {
            p.bend_segment(cmd, t, d);
        }
        Grab::Handle(cmd, slot) => {
            let Some(h) = start.point(cmd, slot) else { return p };
            // The anchor this handle belongs to, and its other handle.
            let anchor = if slot == C2 { cmd } else { cmd.saturating_sub(1) };
            let other = if slot == C2 { (cmd + 1, C1) } else { (anchor, C2) };
            let (Some(a), new) = (start.point(anchor, ANCHOR), h + d) else { return p };
            p.set_point(cmd, slot, new);
            if !alt
                && let Some(o) = start.point(other.0, other.1).filter(|_| matches!(start.cmds.get(other.0), Some(PathCmd::CubicTo(..))))
            {
                let (u, v) = (h - a, o - a);
                let smooth = u.length() > 1e-3 && v.length() > 1e-3 && u.normalized().dot(v.normalized()) < -0.999;
                let dir = (new - a).normalized();
                if smooth && dir != Vec2::ZERO {
                    p.set_point(other.0, other.1, a - dir * v.length());
                }
            }
        }
    }
    p
}

/// Follows the pointer: the path from the drag's start, moved by the whole way so far.
pub fn drag_to(app: &mut App, grab: Grab, start: Pos2, cur: Pos2, alt: bool) {
    let Some(from) = &app.motion_drag else { return };
    let p = dragged(from, grab, Vec2::new(cur.x - start.x, cur.y - start.y), alt);
    let d = p.to_svg();
    app.run("motion.set_path", json!({"d": d}));
}

pub fn drag_end(app: &mut App) {
    app.motion_drag = None;
    app.s.end_gesture();
}

// ----- moving the whole path ---------------------------------------------------------------

/// Whether a Ctrl-press at screen point `p` takes the whole path: on the path, an anchor, or
/// the object travelling it.
pub fn move_hit(app: &App, p: Pos2, xf: Xf) -> bool {
    let Some(shown) = shown(app) else { return false };
    let near_anchor = shown.path.anchors().iter().any(|(_, q)| xf.to_screen(*q).distance(p) <= PICK);
    near_anchor || on_segment(&shown, p, xf).is_some() || app.s.hit_test(xf.to_stage(p), 3.0 * xf.unit()).is_some_and(|(_, e)| e == shown.target)
}

/// Starts moving the whole path from stage point `at`: one undo step until the drag ends.
pub fn move_start(app: &mut App, at: Vec2) -> Drag {
    app.s.begin_gesture(tr_noop!("Move Motion Path"));
    Drag::MotionMove { last: pos2(at.x, at.y) }
}

/// Moves the whole path by the pointer's move since the last frame.
pub fn move_to(app: &mut App, d: Vec2) {
    if d.x != 0.0 || d.y != 0.0 {
        app.run("motion.move_path", json!({"dx": d.x, "dy": d.y}));
    }
}

// ----- adding, deleting and converting points --------------------------------------------

/// Adds an anchor where screen point `p` is on the path and selects it; false when `p` isn't on
/// the path (or is on an anchor already).
fn add_at(app: &mut App, p: Pos2, xf: Xf) -> bool {
    let Some(shown) = shown(app) else { return false };
    if shown.path.anchors().iter().any(|(_, q)| xf.to_screen(*q).distance(p) <= PICK) || on_segment(&shown, p, xf).is_none() {
        return false;
    }
    let sp = xf.to_stage(p);
    if let Some(v) = app.run("motion.add_point", json!({"x": sp.x, "y": sp.y, "slop": PICK * xf.unit()})) {
        app.motion_anchors = v["anchor"].as_u64().map(|a| vec![a as usize]).unwrap_or_default();
        app.points.clear();
        app.status_ok(tr!("Added a point to the motion path. {undo} to undo.", undo = app.undo_key()));
    }
    true
}

/// Double-click with Subselection: on the path it adds an anchor there. False when the
/// double-click wasn't on the path.
pub fn double_click(app: &mut App, p: Pos2, xf: Xf) -> bool {
    app.tool == Tool::Subselect && add_at(app, p, xf)
}

/// Alt-click with the Pen (no path being drawn): on the path it adds an anchor there, as on a
/// shape. False when the click wasn't on the path.
pub fn pen_alt_click(app: &mut App, p: Pos2, xf: Xf) -> bool {
    app.tool == Tool::Pen && app.pen.is_none() && add_at(app, p, xf)
}

/// With the Pen and Alt held: the stretch of path an Alt-click would add a point to.
pub fn pen_hot(app: &App, p: Pos2, xf: Xf) -> Option<Grab> {
    if app.pen.is_some() {
        return None;
    }
    let shown = shown(app)?;
    if shown.path.anchors().iter().any(|(_, q)| xf.to_screen(*q).distance(p) <= PICK) {
        return None;
    }
    on_segment(&shown, p, xf).map(|(cmd, t)| Grab::Segment(cmd, t))
}

/// The selected anchors that are on the path now (an older selection may name others).
fn selected(app: &App) -> Vec<usize> {
    let Some(shown) = shown(app) else { return Vec::new() };
    let n = shown.path.cmds.len();
    app.motion_anchors.iter().copied().filter(|a| *a < n).collect()
}

/// Whether anchor `a` of an `n`-command path is an end.
fn is_end(a: usize, n: usize) -> bool {
    a == 0 || a + 1 == n
}

/// Deletes the selected anchors; the ends stay (the status says why).
pub fn delete_selected(app: &mut App) {
    let anchors = selected(app);
    if anchors.is_empty() {
        return;
    }
    if let Some(v) = app.run("motion.delete_points", json!({"anchors": anchors})) {
        let n = v["deleted"].as_u64().unwrap_or(0);
        let undo = app.undo_key();
        app.status_ok(if v["kept_ends"].as_bool() == Some(true) {
            trn!("Deleted {n} point from the motion path; its ends stay, as they are where the motion starts and ends. {undo} to undo.", "Deleted {n} points from the motion path; its ends stay, as they are where the motion starts and ends. {undo} to undo.", n, undo = undo)
        } else {
            trn!("Deleted {n} point from the motion path. {undo} to undo.", "Deleted {n} points from the motion path. {undo} to undo.", n, undo = undo)
        });
        app.motion_anchors.clear();
    }
}

/// Pins anchor `a` to the playhead's frame (ADR 0049): the object passes it exactly then.
pub fn pin_anchor(app: &mut App, a: usize) {
    if let Some(v) = app.run("motion.pin_point", json!({"anchor": a})) {
        let frame = v["frame"].as_u64().unwrap_or(0) + 1;
        app.status_ok(tr!("Pinned point {anchor} to frame {frame}: the object passes it then. {undo} to undo.", anchor = a, frame = frame, undo = app.undo_key()));
    }
}

/// Lets anchor `a`'s pin go: it shapes the path only, and its frame's X/Y keys go.
pub fn unpin_anchor(app: &mut App, a: usize) {
    if let Some(v) = app.run("motion.unpin_point", json!({"anchor": a})) {
        let frame = v["frame"].as_u64().unwrap_or(0) + 1;
        app.status_ok(tr!("Unpinned point {anchor} from frame {frame}: it shapes the path only. {undo} to undo.", anchor = a, frame = frame, undo = app.undo_key()));
    }
}

/// Makes the selected anchors smooth or corners (one undo step).
pub fn convert_selected(app: &mut App, smooth: bool) {
    let anchors = selected(app);
    if anchors.is_empty() {
        return;
    }
    app.s.begin_gesture(if smooth { tr_noop!("Make Smooth") } else { tr_noop!("Make Corner") });
    for a in anchors {
        app.run("motion.convert_point", json!({"anchor": a, "smooth": smooth}));
    }
    app.s.end_gesture();
}

/// A click with Subselection: on an anchor it selects it (Shift adds or removes), on a handle
/// or the path it keeps the selection. False when the click wasn't on the path, which then
/// drops the anchor selection.
pub fn click(app: &mut App, p: Pos2, xf: Xf, shift: bool) -> bool {
    let Some(shown) = shown(app) else { return false };
    match hit(app, &shown, p, xf, false) {
        Some(Grab::Anchor(a)) => {
            if !shift {
                app.motion_anchors = vec![a];
            } else if let Some(i) = app.motion_anchors.iter().position(|x| *x == a) {
                app.motion_anchors.remove(i);
            } else {
                app.motion_anchors.push(a);
            }
            app.points.clear();
            true
        }
        Some(_) => true,
        None if on_segment(&shown, p, xf).is_some() => {
            if !shift {
                app.motion_anchors.clear();
            }
            true
        }
        None => {
            app.motion_anchors.clear();
            false
        }
    }
}

/// A right-click with Subselection on an anchor selects it (keeping a selection it is part of),
/// so the menu acts on it; on the path it keeps the selection. False when it wasn't on the path.
pub fn secondary_click(app: &mut App, p: Pos2, xf: Xf) -> bool {
    if app.tool != Tool::Subselect {
        return false;
    }
    let Some(shown) = shown(app) else { return false };
    match hit(app, &shown, p, xf, false) {
        Some(Grab::Anchor(a)) => {
            if !app.motion_anchors.contains(&a) {
                app.motion_anchors = vec![a];
            }
            app.points.clear();
            true
        }
        Some(_) => true,
        None => on_segment(&shown, p, xf).is_some(),
    }
}

/// The right-click menu for selected motion path anchors; false when none is selected.
pub fn menu(app: &mut App, ui: &mut egui::Ui) -> bool {
    if app.tool != Tool::Subselect {
        return false;
    }
    let anchors = selected(app);
    let Some(shown) = shown(app) else { return false };
    let n = shown.path.cmds.len();
    if anchors.is_empty() {
        return false;
    }
    if ui.button(tr!("Make Smooth")).clicked() {
        convert_selected(app, true);
        ui.close();
    }
    if ui.button(tr!("Make Corner")).clicked() {
        convert_selected(app, false);
        ui.close();
    }
    ui.separator();
    // Pinning to frames (ADR 0049): one anchor at a time, to the playhead's frame.
    if let [a] = anchors[..] {
        let frame = shown.start + shown.offset + 1;
        match shown.pin(a) {
            Some(p) if shown.is_end(a) => {
                ui.add_enabled(false, egui::Button::new(tr!("Pinned to Frame {frame}", frame = shown.start + p.frame + 1))).on_disabled_hover_text(tr!("The ends of a motion path are always pinned, to the first and last position keys"));
            }
            Some(p) => {
                if ui.button(tr!("Unpin from Frame {frame}", frame = shown.start + p.frame + 1)).on_hover_text(tr!("The point shapes the path only; its frame's X and Y keyframes go")).clicked() {
                    unpin_anchor(app, a);
                    ui.close();
                }
            }
            None => {
                let why = shown.unpinnable(a);
                let resp = ui.add_enabled(why.is_none(), egui::Button::new(tr!("Pin to Frame {frame}", frame = frame)));
                let resp = match why {
                    Some(why) => resp.on_disabled_hover_text(why),
                    None => resp.on_hover_text(tr!("The object passes this point exactly on frame {frame}", frame = frame)),
                };
                if resp.clicked() {
                    pin_anchor(app, a);
                    ui.close();
                }
            }
        }
        ui.separator();
    }
    let deletable = anchors.iter().any(|a| !is_end(*a, n));
    let label = if anchors.len() > 1 { tr!("Delete Points") } else { tr!("Delete Point") };
    let resp = ui.add_enabled(deletable, egui::Button::new(label));
    let resp = if deletable { resp } else { resp.on_disabled_hover_text(tr!("The ends of a motion path are where the motion starts and ends: drag them to move them")) };
    if resp.clicked() {
        delete_selected(app);
        ui.close();
    }
    if ui.button(tr!("Straighten Path")).on_hover_text(tr!("Back to the straight path the X and Y curves trace")).clicked() {
        app.run("motion.clear_path", json!({}));
        app.motion_anchors.clear();
        ui.close();
    }
    true
}

/// Keys for the selected anchors with Subselection, taken before the global shortcuts (so
/// Delete removes points, not the object).
pub fn consume_keys(app: &mut App, ctx: &egui::Context) {
    if app.tool != Tool::Subselect || app.motion_anchors.is_empty() || shown(app).is_none() {
        return;
    }
    let take = |k: Key| ctx.input_mut(|i| i.consume_key(Modifiers::NONE, k));
    if take(Key::Delete) || take(Key::Backspace) {
        delete_selected(app);
    }
    if take(Key::Escape) {
        app.motion_anchors.clear();
    }
}

/// Forgets selected anchors the path no longer has.
pub fn upkeep(app: &mut App) {
    if app.motion_anchors.is_empty() {
        return;
    }
    let n = shown(app).map_or(0, |s| s.path.cmds.len());
    app.motion_anchors.retain(|a| *a < n);
}

fn anchor_box(painter: &Painter, c: Pos2, filled: bool) {
    let r = Rect::from_center_size(c, vec2(7.0, 7.0));
    painter.rect_filled(r, 0.0, if filled { T.tween_motion } else { T.ink });
    painter.rect_stroke(r, 0.0, Stroke::new(1.5, T.tween_motion), StrokeKind::Middle);
}

/// A pinned anchor: a diamond (the keyframe mark) with its frame number in a tag beside it.
fn pinned_anchor(painter: &Painter, c: Pos2, filled: bool, frame: u32) {
    let r = 5.5;
    let pts = vec![c + vec2(0.0, -r), c + vec2(r, 0.0), c + vec2(0.0, r), c + vec2(-r, 0.0)];
    painter.add(egui::Shape::convex_polygon(pts, if filled { T.tween_motion } else { T.ink }, Stroke::new(1.5, T.tween_motion)));
    let g = painter.layout_no_wrap(frame.to_string(), font(Type::DataSmall), T.ink);
    let tag = Rect::from_min_size(c + vec2(r + 3.0, -r - g.size().y - 1.0), g.size() + vec2(6.0, 2.0));
    painter.rect_filled(tag, T.rc(), T.tween_motion);
    painter.galley(tag.min + vec2(3.0, 1.0), g, T.ink);
}

/// Draws the path: a line in the motion colour on a dark halo (it reads on any artwork), a dot
/// per frame, larger on position keys and ringed at the playhead, and the anchors the tool can
/// take (with the selected anchors' handles under Subselection). `hot` is what the pointer is
/// over; a segment there (or the whole path, being moved) draws wider.
pub fn draw(app: &App, painter: &Painter, shown: &Shown, xf: Xf, hot: Option<Grab>) {
    let to_px = Affine::translate(xf.origin.x, xf.origin.y).mul(&Affine::scale(xf.zoom, xf.zoom));
    let wide = matches!(hot, Some(Grab::Segment(..))) || matches!(app.drag, Drag::MotionPath { grab: Grab::Segment(..), .. } | Drag::MotionMove { .. });
    for pl in shown.path.flatten(&to_px, 0.35) {
        let pts: Vec<Pos2> = pl.points.iter().map(|p| pos2(p.x, p.y)).collect();
        painter.add(egui::Shape::line(pts.clone(), Stroke::new(if wide { 4.5 } else { 3.0 }, T.ink)));
        painter.add(egui::Shape::line(pts, Stroke::new(if wide { 2.5 } else { 1.5 }, T.tween_motion)));
    }
    // Dots thin out when frames crowd closer than three points apart.
    let mut last: Option<Pos2> = None;
    for (f, q) in shown.dots.iter().enumerate() {
        let c = xf.to_screen(*q);
        let key = shown.keys.binary_search(&(f as u32)).is_ok();
        if !key && last.is_some_and(|l| l.distance(c) < 3.0) {
            continue;
        }
        last = Some(c);
        painter.circle(c, if key { 3.5 } else { 2.0 }, T.tween_motion, Stroke::new(1.0, T.ink));
        if f as u32 == shown.offset {
            painter.circle_stroke(c, 5.5, Stroke::new(1.5, T.tween_motion));
        }
    }
    if app.tool == Tool::Subselect {
        for (cmd, slot, anchor) in handles(&shown.path, &app.motion_anchors) {
            if let (Some(h), Some(a)) = (shown.path.point(cmd, slot), shown.path.point(anchor, ANCHOR)) {
                let (hs, as_) = (xf.to_screen(h), xf.to_screen(a));
                painter.line_segment([as_, hs], Stroke::new(1.0, T.tween_motion));
                let lit = hot == Some(Grab::Handle(cmd, slot));
                painter.circle(hs, if lit { 4.5 } else { 3.5 }, T.ink, Stroke::new(1.5, T.tween_motion));
            }
        }
    }
    if matches!(app.tool, Tool::Select | Tool::Subselect | Tool::Pen) {
        for (cmd, q) in grabbable_anchors(&shown.path, &shown.pins, app.tool) {
            let lit = app.motion_anchors.contains(&cmd) || hot == Some(Grab::Anchor(cmd));
            match shown.pin(cmd) {
                Some(p) => pinned_anchor(painter, xf.to_screen(q), lit, shown.start + p.frame + 1),
                None => anchor_box(painter, xf.to_screen(q), lit),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s_curve() -> Path {
        let mut p = Path::new();
        p.move_to(Vec2::new(0.0, 0.0)).cubic_to(Vec2::new(30.0, 0.0), Vec2::new(70.0, 50.0), Vec2::new(100.0, 50.0)).cubic_to(Vec2::new(130.0, 50.0), Vec2::new(170.0, 0.0), Vec2::new(200.0, 0.0));
        p
    }

    #[test]
    fn dragging_a_smooth_handle_keeps_its_partner_in_line() {
        let p = s_curve();
        // Anchor 1 at (100, 50): handles (70, 50) and (130, 50), smooth.
        let q = dragged(&p, Grab::Handle(1, C2), Vec2::new(0.0, -30.0), false);
        assert_eq!(q.point(1, C2), Some(Vec2::new(70.0, 20.0)));
        let (a, h, o) = (Vec2::new(100.0, 50.0), q.point(1, C2).unwrap(), q.point(2, C1).unwrap());
        assert!((h - a).normalized().dot((o - a).normalized()) < -0.9999, "still collinear");
        assert!(((o - a).length() - 30.0).abs() < 1e-3, "the partner keeps its length");
        // Alt breaks the pair.
        let q = dragged(&p, Grab::Handle(1, C2), Vec2::new(0.0, -30.0), true);
        assert_eq!(q.point(2, C1), Some(Vec2::new(130.0, 50.0)));
    }

    #[test]
    fn anchors_and_segments_drag_as_expected() {
        let p = s_curve();
        let q = dragged(&p, Grab::Anchor(2), Vec2::new(10.0, 10.0), false);
        assert_eq!(q.point(2, ANCHOR), Some(Vec2::new(210.0, 10.0)));
        assert_eq!(q.point(2, C2), Some(Vec2::new(180.0, 10.0)), "its handle came along");
        let before = p.segment_point(1, 0.5).unwrap();
        let q = dragged(&p, Grab::Segment(1, 0.5), Vec2::new(0.0, 20.0), false);
        assert!(q.segment_point(1, 0.5).unwrap().distance(before + Vec2::new(0.0, 20.0)) < 1e-3);
        assert_eq!(q.point(1, ANCHOR), p.point(1, ANCHOR));
        // Selection takes the ends and pinned anchors; Subselection every anchor, and the Pen
        // sees them all.
        assert_eq!(grabbable_anchors(&p, &[], Tool::Select).iter().map(|a| a.0).collect::<Vec<_>>(), vec![0, 2]);
        assert_eq!(grabbable_anchors(&p, &[Pin { frame: 5, anchor: 1 }], Tool::Select).len(), 3);
        assert_eq!(grabbable_anchors(&p, &[], Tool::Subselect).len(), 3);
        assert_eq!(grabbable_anchors(&p, &[], Tool::Pen).len(), 3);
        assert_eq!(handles(&p, &[1]), vec![(1, C2, 1), (2, C1, 1)]);
        assert_eq!(handles(&p, &[0]), vec![(1, C1, 0)]);
        assert_eq!(handles(&p, &[0, 2]), vec![(1, C1, 0), (2, C2, 2)], "every selected anchor shows its handles");
        assert!(is_end(0, 3) && is_end(2, 3) && !is_end(1, 3));
    }

    /// The pointer and keyboard paths, on a headless app showing the sample's curved motion
    /// path (the `motionpoints` screenshot setup: the span selected, Subselection, anchor 1).
    mod headless {
        use super::*;

        /// Stage and screen coordinates coincide.
        const XF: Xf = Xf { origin: Pos2::ZERO, zoom: 1.0 };

        #[test]
        fn hit_finds_handles_anchors_and_segments_per_tool() {
            let mut app = app();
            let s = shown(&app).expect("the motion path shows");
            assert_eq!(s.path.cmds.len(), 3);
            let handle = s.path.point(1, C2).unwrap();
            assert_eq!(hit(&app, &s, screen(handle), XF, true), Some(Grab::Handle(1, C2)));
            assert_eq!(hit(&app, &s, screen(anchor(&s, 1)), XF, true), Some(Grab::Anchor(1)));
            assert_eq!(hit(&app, &s, far(), XF, true), None);
            app.tool = Tool::Select;
            assert_eq!(hit(&app, &s, screen(anchor(&s, 1)), XF, false), None, "Selection takes only the ends");
            assert_eq!(hit(&app, &s, screen(anchor(&s, 2)), XF, false), Some(Grab::Anchor(2)));
            let mid = screen(s.path.segment_point(1, 0.5).unwrap());
            assert!(matches!(hit(&app, &s, mid, XF, true), Some(Grab::Segment(1, _))));
            assert_eq!(hit(&app, &s, mid, XF, false), None, "the press goes to the object first");
        }

        #[test]
        fn clicks_select_anchors_and_shift_toggles_them() {
            let mut app = app();
            let s = shown(&app).unwrap();
            assert!(click(&mut app, screen(anchor(&s, 0)), XF, false));
            assert_eq!(app.motion_anchors, vec![0]);
            assert!(click(&mut app, screen(anchor(&s, 2)), XF, true));
            assert_eq!(app.motion_anchors, vec![0, 2]);
            assert!(click(&mut app, screen(anchor(&s, 0)), XF, true));
            assert_eq!(app.motion_anchors, vec![2]);
            assert!(click(&mut app, screen(s.path.segment_point(1, 0.5).unwrap()), XF, true), "on the path");
            assert_eq!(app.motion_anchors, vec![2], "Shift on the path keeps the selection");
            assert!(click(&mut app, screen(s.path.segment_point(1, 0.5).unwrap()), XF, false));
            assert!(app.motion_anchors.is_empty());
            app.motion_anchors = vec![1];
            assert!(!click(&mut app, far(), XF, false), "off the path");
            assert!(app.motion_anchors.is_empty());
        }

        #[test]
        fn a_right_click_selects_the_anchor_under_it() {
            let mut app = app();
            let s = shown(&app).unwrap();
            assert!(secondary_click(&mut app, screen(anchor(&s, 2)), XF));
            assert_eq!(app.motion_anchors, vec![2]);
            app.motion_anchors = vec![1, 2];
            assert!(secondary_click(&mut app, screen(anchor(&s, 2)), XF));
            assert_eq!(app.motion_anchors, vec![1, 2], "a selection it is part of stays");
            assert!(secondary_click(&mut app, screen(s.path.segment_point(0, 0.5).unwrap_or_else(|| s.path.segment_point(1, 0.25).unwrap())), XF));
            assert!(!secondary_click(&mut app, far(), XF));
            app.tool = Tool::Select;
            assert!(!secondary_click(&mut app, screen(anchor(&s, 2)), XF), "only with Subselection");
        }

        #[test]
        fn a_double_click_on_the_path_adds_an_anchor_in_one_undo_step() {
            let mut app = app();
            let s = shown(&app).unwrap();
            assert!(!double_click(&mut app, far(), XF));
            assert!(!double_click(&mut app, screen(anchor(&s, 1)), XF), "an anchor is there already");
            let steps = app.s.history().undo_len();
            assert!(double_click(&mut app, screen(s.path.segment_point(1, 0.5).unwrap()), XF));
            assert_eq!(cmds(&app), 4);
            assert_eq!(app.motion_anchors.len(), 1, "the new anchor is selected");
            assert!(status(&app).contains("Added a point"));
            assert_eq!(app.s.history().undo_len(), steps + 1);
            app.tool = Tool::Select;
            assert!(!double_click(&mut app, screen(s.path.segment_point(1, 0.5).unwrap()), XF), "only with Subselection");
        }

        #[test]
        fn the_pen_adds_with_alt_click_on_the_path_only() {
            let mut app = app();
            app.tool = Tool::Pen;
            let s = shown(&app).unwrap();
            let mid = screen(s.path.segment_point(1, 0.5).unwrap());
            assert!(matches!(pen_hot(&app, mid, XF), Some(Grab::Segment(1, _))));
            assert_eq!(pen_hot(&app, screen(anchor(&s, 1)), XF), None);
            assert_eq!(pen_hot(&app, far(), XF), None);
            assert!(pen_alt_click(&mut app, mid, XF));
            assert_eq!(cmds(&app), 4);
            app.tool = Tool::Subselect;
            assert!(!pen_alt_click(&mut app, mid, XF), "only with the Pen");
        }

        #[test]
        fn deleting_points_keeps_the_ends_and_says_so() {
            let mut app = app();
            app.motion_anchors = vec![0, 1, 9];
            delete_selected(&mut app);
            assert_eq!(cmds(&app), 2);
            assert!(status(&app).contains("its ends stay"), "{}", status(&app));
            assert!(app.motion_anchors.is_empty());
            // Nothing selected: nothing happens.
            let steps = app.s.history().undo_len();
            delete_selected(&mut app);
            assert_eq!(app.s.history().undo_len(), steps);
        }

        #[test]
        fn deleting_a_middle_point_reports_one_point() {
            let mut app = app();
            delete_selected(&mut app);
            assert_eq!(cmds(&app), 2);
            assert!(status(&app).starts_with("Deleted 1 point from the motion path."), "{}", status(&app));
        }

        #[test]
        fn converting_points_is_one_undo_step() {
            let mut app = app();
            let steps = app.s.history().undo_len();
            convert_selected(&mut app, false);
            assert_eq!(app.s.history().undo_len(), steps + 1);
            assert_eq!(app.s.undo_label(), Some("Make Corner"));
            app.motion_anchors.clear();
            convert_selected(&mut app, true);
            assert_eq!(app.s.history().undo_len(), steps + 1, "no anchor, no step");
        }

        #[test]
        fn dragging_an_end_moves_it_in_one_undo_step() {
            let mut app = app();
            let s = shown(&app).unwrap();
            let end = anchor(&s, 2);
            let steps = app.s.history().undo_len();
            let drag = drag_start(&mut app, s, Grab::Anchor(2), end);
            assert_eq!(app.motion_anchors, vec![2], "the grabbed anchor becomes the selection");
            let Drag::MotionPath { grab, start } = drag else { panic!("a motion path drag") };
            drag_to(&mut app, grab, start, start + vec2(10.0, 0.0), false);
            drag_to(&mut app, grab, start, start + vec2(20.0, 5.0), false);
            drag_end(&mut app);
            assert!(app.motion_drag.is_none());
            assert_eq!(app.s.history().undo_len(), steps + 1);
            let moved = anchor(&shown(&app).unwrap(), 2);
            assert!(moved.distance(end + Vec2::new(20.0, 5.0)) < 1e-2, "{moved:?}");
            // With no drag in progress, following the pointer does nothing.
            drag_to(&mut app, grab, start, start + vec2(99.0, 0.0), false);
            assert_eq!(anchor(&shown(&app).unwrap(), 2), moved);
        }

        #[test]
        fn moving_the_whole_path_keeps_its_shape() {
            let mut app = app();
            let s = shown(&app).unwrap();
            assert!(move_hit(&app, screen(anchor(&s, 1)), XF));
            assert!(move_hit(&app, screen(s.path.segment_point(1, 0.5).unwrap()), XF));
            assert!(!move_hit(&app, far(), XF));
            let before: Vec<Vec2> = s.path.anchors().into_iter().map(|(_, q)| q).collect();
            let drag = move_start(&mut app, before[0]);
            assert!(matches!(drag, Drag::MotionMove { .. }));
            move_to(&mut app, Vec2::ZERO);
            move_to(&mut app, Vec2::new(5.0, -3.0));
            app.s.end_gesture();
            assert_eq!(app.s.undo_label(), Some("Move Motion Path"));
            let after: Vec<Vec2> = shown(&app).unwrap().path.anchors().into_iter().map(|(_, q)| q).collect();
            for (a, b) in before.iter().zip(&after) {
                assert!(b.distance(*a + Vec2::new(5.0, -3.0)) < 1e-2, "{a:?} -> {b:?}");
            }
        }

        #[test]
        fn pinning_from_the_menu_pins_to_the_playhead_and_unpinning_lets_go() {
            let mut app = app();
            let s = shown(&app).unwrap();
            assert_eq!(s.pins.len(), 2, "only the ends are pinned");
            assert!(s.is_end(0) && s.pin(1).is_none());
            assert_eq!(s.unpinnable(1), None, "frame 10 is inside the motion");
            let steps = app.s.history().undo_len();
            pin_anchor(&mut app, 1);
            assert_eq!(app.s.history().undo_len(), steps + 1);
            assert!(status(&app).starts_with("Pinned point 1 to frame 10"), "{}", status(&app));
            let s = shown(&app).unwrap();
            assert_eq!(s.pin(1), Some(Pin { frame: 9, anchor: 1 }));
            assert!(s.dots[9].distance(anchor(&s, 1)) < 1e-3, "the object is on the pinned anchor on its frame");
            // Selection now takes the pinned anchor too.
            app.tool = Tool::Select;
            assert_eq!(hit(&app, &s, screen(anchor(&s, 1)), XF, false), Some(Grab::Anchor(1)));
            unpin_anchor(&mut app, 1);
            assert!(status(&app).starts_with("Unpinned point 1 from frame 10"), "{}", status(&app));
            assert!(shown(&app).unwrap().pin(1).is_none());
            // Outside the motion the menu says why it can't pin.
            app.s.set_frame(0);
            assert!(shown(&app).unwrap().unpinnable(1).is_some_and(|w| w.contains("between frames 1 and")));
        }

        #[test]
        fn the_menu_offers_pin_unpin_and_shows_an_ends_frame() {
            let (mut app, ctx) = app_and_ctx();
            for anchors in [vec![1], vec![0], vec![2]] {
                app.motion_anchors = anchors;
                let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                    assert!(menu(&mut app, ui));
                });
                out.textures_delta.clear();
            }
            pin_anchor(&mut app, 1);
            app.motion_anchors = vec![1];
            let s = shown(&app).unwrap();
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                assert!(menu(&mut app, ui));
                draw(&app, ui.painter(), &s, XF, Some(Grab::Anchor(1)));
            });
            out.textures_delta.clear();
        }

        #[test]
        fn upkeep_forgets_anchors_the_path_lost() {
            let mut app = app();
            app.motion_anchors = vec![1, 7];
            upkeep(&mut app);
            assert_eq!(app.motion_anchors, vec![1]);
            app.motion_anchors.clear();
            upkeep(&mut app);
            assert!(app.motion_anchors.is_empty());
        }

        #[test]
        fn delete_and_escape_act_on_the_selected_points() {
            let (mut app, ctx) = app_and_ctx();
            frame_with_key(&ctx, &mut app, Key::Escape);
            assert!(app.motion_anchors.is_empty(), "Escape drops the anchor selection");
            app.motion_anchors = vec![1];
            frame_with_key(&ctx, &mut app, Key::Delete);
            assert_eq!(cmds(&app), 2);
            // Not with Selection: Delete is then the global shortcut's.
            app.tool = Tool::Select;
            app.motion_anchors = vec![0];
            frame_with_key(&ctx, &mut app, Key::Escape);
            assert_eq!(app.motion_anchors, vec![0]);
        }

        #[test]
        fn the_menu_and_the_drawing_need_subselection_and_a_selection() {
            let (mut app, ctx) = app_and_ctx();
            let s = shown(&app).unwrap();
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                assert!(menu(&mut app, ui));
                draw(&app, ui.painter(), &s, XF, Some(Grab::Handle(1, C2)));
                app.motion_anchors.clear();
                assert!(!menu(&mut app, ui), "no anchor selected");
                app.tool = Tool::Select;
                assert!(!menu(&mut app, ui));
                draw(&app, ui.painter(), &s, XF, Some(Grab::Segment(1, 0.5)));
            });
            out.textures_delta.clear();
        }

        // ----- helpers -----

        fn app_and_ctx() -> (App, egui::Context) {
            let ctx = egui::Context::default();
            let session = anicraft_engine::Session::new(anicraft_engine::doc::sample::demo());
            // Screenshot mode: default preferences, no autosave, no plug-in folder.
            let mut app = App::with_context(&ctx, session, Some(std::path::PathBuf::from("unused.png")));
            app.open_panel(Some("motionpoints"));
            assert_eq!(app.tool, Tool::Subselect);
            assert_eq!(app.motion_anchors, vec![1]);
            (app, ctx)
        }

        fn app() -> App {
            app_and_ctx().0
        }

        fn frame_with_key(ctx: &egui::Context, app: &mut App, key: Key) {
            let mut input = egui::RawInput::default();
            input.events.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
            let mut out = ctx.run_ui(input, |ui| consume_keys(app, ui.ctx()));
            // The textures it made have nowhere to go: drop them.
            out.textures_delta.clear();
        }

        fn anchor(s: &Shown, i: usize) -> Vec2 {
            s.path.point(i, ANCHOR).unwrap()
        }

        fn screen(q: Vec2) -> Pos2 {
            XF.to_screen(q)
        }

        fn far() -> Pos2 {
            pos2(-5000.0, -5000.0)
        }

        fn cmds(app: &App) -> usize {
            shown(app).map_or(0, |s| s.path.cmds.len())
        }

        fn status(app: &App) -> String {
            app.status.as_ref().map(|s| s.text.clone()).unwrap_or_default()
        }
    }
}
