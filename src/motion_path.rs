//! Motion paths on the stage (ADR 0030). When a motion tween's object is selected, or a frame
//! of its span is, the stage shows the path its registration point travels, in the motion
//! colour, with a dot on every frame (larger on position keys).
//!
//! * **Selection (V)**: drag the path to bend it, or drag either end to move where the motion
//!   starts or ends. Dragging the object itself still moves it (auto-key).
//! * **Subselection (A)**: drag any anchor, or a selected anchor's handles (Alt breaks a smooth
//!   pair). A click on an anchor selects it and shows its handles.
//!
//! A tween without a path of its own shows the straight path its X/Y curves trace; the first
//! edit makes it a path. Every change is `motion.set_path`, and a drag is one undo step (a
//! session gesture). Only the anchor selection and the drag in progress live here.

use crate::app::{App, Drag, Tool};
use crate::pathtools::Xf;
use crate::theme::T;
use anicraft_engine::doc::Props;
use anicraft_engine::geom::edit::{ANCHOR, C1, C2};
use anicraft_engine::geom::{Affine, Path, PathCmd, Vec2};
use egui::{Painter, Pos2, Rect, Stroke, StrokeKind, pos2, vec2};
use serde_json::json;

/// Grab radius for anchors, handles and the path, in screen points.
const PICK: f32 = 6.0;

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
}

/// The motion path of the active layer's motion tween at the playhead, when its object or a
/// frame of its span is selected and it moves at all.
pub fn shown(app: &App) -> Option<Shown> {
    let tl = app.s.doc.timeline(app.s.scene);
    let layer = app.s.active_layer().filter(|l| l.visible && l.kind == anicraft_engine::doc::LayerKind::Normal)?;
    let k = layer.keyframe_at(app.s.frame)?;
    let m = k.motion.as_ref()?;
    let target = m.target_index(&k.elements).and_then(|i| k.elements.get(i))?;
    let span_selected = app.frame_sel.is_some_and(|sel| {
        let (a, b) = sel.frames();
        sel.layers(&tl.layers).contains(&layer.id) && a < k.end() && b >= k.start
    });
    if !app.s.selection.contains(&target.id) && !span_selected {
        return None;
    }
    let base = Props::of(target);
    let path = m.path.clone().or_else(|| m.path_from_tracks(&base))?;
    let dots = m.positions(&base, k.duration);
    let mut keys: Vec<u32> = [anicraft_engine::doc::Property::X, anicraft_engine::doc::Property::Y].into_iter().filter_map(|p| m.track(p)).flat_map(|t| t.keys.iter().map(|k| k.frame)).filter(|f| *f < k.duration).collect();
    keys.sort_unstable();
    keys.dedup();
    Some(Shown { path, dots, keys, offset: app.s.frame - k.start })
}

/// The anchors the tool can take: both ends with Selection, every anchor with Subselection.
fn grabbable_anchors(path: &Path, tool: Tool) -> Vec<(usize, Vec2)> {
    let all = path.anchors();
    match tool {
        Tool::Subselect => all,
        _ => {
            let n = all.len();
            all.into_iter().enumerate().filter(|(i, _)| *i == 0 || *i + 1 == n).map(|(_, a)| a).collect()
        }
    }
}

/// The handles shown for the selected anchor: `(cmd, slot)`.
fn handles(path: &Path, anchor: Option<usize>) -> Vec<(usize, u8)> {
    let Some(a) = anchor else { return Vec::new() };
    let (i, o) = path.anchor_segments(a);
    let mut v = Vec::new();
    if let Some(i) = i.filter(|&i| matches!(path.cmds.get(i), Some(PathCmd::CubicTo(..)))) {
        v.push((i, C2));
    }
    if let Some(o) = o.filter(|&o| matches!(path.cmds.get(o), Some(PathCmd::CubicTo(..)))) {
        v.push((o, C1));
    }
    v
}

/// The handle, anchor or (with Selection) the stretch of path under screen point `p`.
/// `segments` is false while the press should first go to the object under it.
pub fn hit(app: &App, shown: &Shown, p: Pos2, xf: Xf, segments: bool) -> Option<Grab> {
    let near = |q: Vec2| xf.to_screen(q).distance(p) <= PICK;
    if app.tool == Tool::Subselect {
        for (cmd, slot) in handles(&shown.path, app.motion_anchor) {
            if shown.path.point(cmd, slot).is_some_and(near) {
                return Some(Grab::Handle(cmd, slot));
            }
        }
    }
    if let Some((cmd, _)) = grabbable_anchors(&shown.path, app.tool).into_iter().find(|(_, q)| near(*q)) {
        return Some(Grab::Anchor(cmd));
    }
    if segments && app.tool == Tool::Select {
        let (cmd, t, d) = shown.path.nearest_segment(xf.to_stage(p), &Affine::IDENTITY)?;
        if d * xf.zoom <= PICK {
            return Some(Grab::Segment(cmd, t));
        }
    }
    None
}

/// Starts dragging `grab`: one undo step until the drag ends.
pub fn drag_start(app: &mut App, shown: Shown, grab: Grab, at: Vec2) -> Drag {
    if let Grab::Anchor(a) = grab {
        app.motion_anchor = Some(a);
    }
    app.motion_drag = Some(shown.path);
    app.s.begin_gesture("Edit Motion Path");
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

/// A click with Subselection on an anchor selects it (showing its handles); false when the
/// click wasn't on one.
pub fn click(app: &mut App, p: Pos2, xf: Xf) -> bool {
    let Some(shown) = shown(app) else { return false };
    match hit(app, &shown, p, xf, false) {
        Some(Grab::Anchor(a)) => {
            app.motion_anchor = Some(a);
            true
        }
        Some(_) => true,
        None => false,
    }
}

/// Forgets an anchor selection the path no longer has.
pub fn upkeep(app: &mut App) {
    if let Some(a) = app.motion_anchor
        && shown(app).is_none_or(|s| s.path.point(a, ANCHOR).is_none())
    {
        app.motion_anchor = None;
    }
}

fn anchor_box(painter: &Painter, c: Pos2, filled: bool) {
    let r = Rect::from_center_size(c, vec2(7.0, 7.0));
    painter.rect_filled(r, 0.0, if filled { T.tween_motion } else { T.ink });
    painter.rect_stroke(r, 0.0, Stroke::new(1.5, T.tween_motion), StrokeKind::Middle);
}

/// Draws the path: a line in the motion colour on a dark halo (it reads on any artwork), a dot
/// per frame, larger on position keys and ringed at the playhead, and the anchors the tool can
/// take (with the selected anchor's handles under Subselection).
pub fn draw(app: &App, painter: &Painter, shown: &Shown, xf: Xf, hot: Option<Grab>) {
    let to_px = Affine::translate(xf.origin.x, xf.origin.y).mul(&Affine::scale(xf.zoom, xf.zoom));
    let wide = matches!(hot, Some(Grab::Segment(..))) || matches!(app.drag, Drag::MotionPath { grab: Grab::Segment(..), .. });
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
        for (cmd, slot) in handles(&shown.path, app.motion_anchor) {
            let anchor = if slot == C2 { cmd } else { cmd.saturating_sub(1) };
            if let (Some(h), Some(a)) = (shown.path.point(cmd, slot), shown.path.point(anchor, ANCHOR)) {
                let (hs, as_) = (xf.to_screen(h), xf.to_screen(a));
                painter.line_segment([as_, hs], Stroke::new(1.0, T.tween_motion));
                let lit = hot == Some(Grab::Handle(cmd, slot));
                painter.circle(hs, if lit { 4.5 } else { 3.5 }, T.ink, Stroke::new(1.5, T.tween_motion));
            }
        }
    }
    if matches!(app.tool, Tool::Select | Tool::Subselect) {
        for (cmd, q) in grabbable_anchors(&shown.path, app.tool) {
            anchor_box(painter, xf.to_screen(q), app.motion_anchor == Some(cmd) || hot == Some(Grab::Anchor(cmd)));
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
        // Selection takes only the ends; Subselection every anchor.
        assert_eq!(grabbable_anchors(&p, Tool::Select).iter().map(|a| a.0).collect::<Vec<_>>(), vec![0, 2]);
        assert_eq!(grabbable_anchors(&p, Tool::Subselect).len(), 3);
        assert_eq!(handles(&p, Some(1)), vec![(1, C2), (2, C1)]);
        assert_eq!(handles(&p, Some(0)), vec![(1, C1)]);
    }
}
