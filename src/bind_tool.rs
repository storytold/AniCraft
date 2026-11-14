//! The Bind tool (ADR 0071, ADR 0090): which points of a shape follow which of its bones, and
//! how much.
//!
//! With the tool in hand, every shape armature on the stage shows its points: a square for a
//! point that follows one bone, a triangle for one that several bones share (as in Animate).
//! Click a bone to pick it: the points it moves fill in the selection colour. Then a click on a
//! point makes it follow only that bone, Shift-click shares it with the bones it follows,
//! Ctrl-click (⌘ on macOS) takes it from the bone, and a box dragged on empty stage does the
//! same to every point inside it. Each is one `bone.bind`, one undo step.
//!
//! **Paint Weights** (Properties › Bone, or the tool's options) turns the pointer into a brush
//! (ADR 0090): dragging over the shape raises (Add), lowers (Subtract) or evens out (Smooth) the
//! picked bone's share of the points under it, with a soft falloff to the brush's rim. The points
//! show as a heat map of that share (blue none, red all) and the outline between them in the
//! same colours, live as the stroke goes. A stroke is one gesture, so one undo step; `[` and `]`
//! change the brush size. A point clicked or painted becomes the *focused* point, whose exact
//! share Properties › Bone shows and edits as a number (for the keyboard, `bone.set_weight`).

use crate::app::{App, Drag, Tool};
use crate::bone_tool;
use crate::pathtools::Xf;
use crate::theme::{T, to_color32};
use anicraft_engine::doc::skin::points_shown;
use anicraft_engine::doc::weights::Brush;
use anicraft_engine::doc::{BoneId, ElementKind, LayerId, LayerKind, evaluate_layer_elements};
use anicraft_engine::geom::{PathCmd, Vec2};
use egui::{Color32, Pos2, Rect, Stroke, StrokeKind, pos2, vec2};
use serde_json::json;

/// How close (screen points) a click must be to a point to bind it, and the size points draw.
const GRAB: f32 = 6.0;
const SIZE: f32 = 3.5;
/// Brush size limits (px on the stage).
pub const RADIUS_MIN: f32 = 1.0;
pub const RADIUS_MAX: f32 = 500.0;
/// Most dabs one pointer move sends (a fast flick across the stage).
const MOST_DABS: usize = 256;

/// The Bind tool's options (view state, kept for the session).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BindOptions {
    /// Paint weights instead of binding points by clicks and boxes.
    pub paint: bool,
    pub brush: Brush,
    /// The brush's radius on the stage (px).
    pub radius: f32,
    /// Its strength at the centre, 0–100.
    pub strength: f32,
    /// The point whose weights Properties shows and edits (index into the shape's points).
    pub point: Option<usize>,
    /// Where the brush shows (stage) while no pointer is over the stage: the screenshot's.
    pub preview: Option<Vec2>,
}

impl Default for BindOptions {
    fn default() -> Self {
        BindOptions { paint: false, brush: Brush::Add, radius: 20.0, strength: 25.0, point: None, preview: None }
    }
}

/// A brush's label and hint, for Properties.
pub fn brush_label(b: Brush) -> (&'static str, &'static str) {
    match b {
        Brush::Add => (tr_noop!("Add"), tr_noop!("Raises how much the bone moves the points under the brush")),
        Brush::Subtract => (tr_noop!("Subtract"), tr_noop!("Lowers how much the bone moves the points under the brush; their other bones take up the rest")),
        Brush::Smooth => (tr_noop!("Smooth"), tr_noop!("Evens out the bone's share across the points under the brush, softening a hard edge between bones")),
    }
}

/// One shape armature's points as the stage shows them at the playhead.
struct Points {
    layer: LayerId,
    color: Color32,
    locked: bool,
    /// Stage position, the bones it follows with their weights.
    points: Vec<(Vec2, Vec<(BoneId, f32)>)>,
    /// The outline on the stage: per segment, its points (flattened) and the anchors it joins.
    outline: Vec<(Vec<Vec2>, usize, usize)>,
}

/// A few points along a segment from `a` (and its controls) to its end, for drawing.
fn along(cmd: &PathCmd, a: Vec2) -> Vec<Vec2> {
    const N: usize = 8;
    let lerp = |p: Vec2, q: Vec2, t: f32| p + (q - p) * t;
    match *cmd {
        PathCmd::LineTo(b) => vec![a, b],
        PathCmd::QuadTo(c, b) => (0..=N).map(|i| i as f32 / N as f32).map(|t| lerp(lerp(a, c, t), lerp(c, b, t), t)).collect(),
        PathCmd::CubicTo(c1, c2, b) => (0..=N)
            .map(|i| i as f32 / N as f32)
            .map(|t| {
                let (p, q, r) = (lerp(a, c1, t), lerp(c1, c2, t), lerp(c2, b, t));
                lerp(lerp(p, q, t), lerp(q, r, t), t)
            })
            .collect(),
        PathCmd::MoveTo(_) | PathCmd::Close => Vec::new(),
    }
}

fn shown(app: &App) -> Vec<Points> {
    let tl = app.s.timeline();
    tl.layers
        .iter()
        .enumerate()
        .filter(|(i, l)| l.kind == LayerKind::Armature && l.visible && !tl.hidden_by_folder(*i))
        .filter_map(|(_, l)| {
            let arm = l.armature.as_deref()?;
            let skin = arm.skin.as_ref()?;
            let (_, els) = evaluate_layer_elements(l, app.s.frame)?;
            let at = points_shown(arm, &els);
            let points = at.into_iter().zip(&skin.points).map(|(p, b)| (p, b.bones.iter().map(|i| (i.0, i.1)).collect())).collect();
            // The outline as it shows, segment by segment, with the anchors at its ends.
            let mut outline = Vec::new();
            if let Some(e) = els.iter().find(|e| e.id == skin.shape)
                && let ElementKind::Shape(sh) = &e.kind
            {
                let (mut idx, mut start, mut last, mut start_idx) = (0usize, Vec2::ZERO, Vec2::ZERO, 0usize);
                for c in &sh.path.cmds {
                    match *c {
                        PathCmd::MoveTo(p) => {
                            (start, last, start_idx) = (p, p, idx);
                            idx += 1;
                        }
                        PathCmd::Close => {
                            if last != start {
                                outline.push((vec![e.transform.apply(last), e.transform.apply(start)], idx.saturating_sub(1), start_idx));
                            }
                            last = start;
                        }
                        PathCmd::LineTo(p) | PathCmd::QuadTo(_, p) | PathCmd::CubicTo(_, _, p) => {
                            let pts = along(c, last).into_iter().map(|q| e.transform.apply(q)).collect();
                            outline.push((pts, idx.saturating_sub(1), idx));
                            last = p;
                            idx += 1;
                        }
                    }
                }
            }
            Some(Points { layer: l.id, color: to_color32(l.color), locked: l.locked || tl.locked_by_folder(l.id), points, outline })
        })
        .collect()
}

/// What a press with `command` and `shift` held does to the points: Ctrl/⌘ takes them from the
/// bone, Shift shares it, otherwise they follow only it.
fn mode(shift: bool, command: bool) -> &'static str {
    if command {
        "remove"
    } else if shift {
        "add"
    } else {
        "only"
    }
}

/// The picked bone, when it is inside a shape on layer `layer`.
fn picked_in(app: &App, layer: LayerId) -> Option<BoneId> {
    let id = app.bone_sel?;
    app.s.timeline().layer(layer)?.armature.as_ref().filter(|a| a.skin.is_some() && a.bone(id).is_some()).map(|_| id)
}

/// The picked bone when it is inside a shape (any layer).
fn picked_shape_bone(app: &App) -> Option<BoneId> {
    let id = app.bone_sel?;
    app.s.timeline().layers.iter().any(|l| l.armature.as_ref().is_some_and(|a| a.skin.is_some() && a.bone(id).is_some())).then_some(id)
}

fn bind(app: &mut App, bone: BoneId, points: Vec<usize>, shift: bool, command: bool) {
    let n = points.len();
    let m = mode(shift, command);
    if let Some(v) = app.run("bone.bind", json!({"bone": bone.0, "points": points, "mode": m})) {
        let bound = v["bound"].as_u64().unwrap_or(0);
        let msg = match m {
            "remove" => trn!("Took {n} point from the bone; it follows {bound} points.", "Took {n} points from the bone; it follows {bound} points.", n, bound = bound),
            "add" => trn!("{n} point now shares the bone; it moves {bound} points.", "{n} points now share the bone; it moves {bound} points.", n, bound = bound),
            _ => trn!("{n} point now follows only this bone; it moves {bound} points.", "{n} points now follow only this bone; it moves {bound} points.", n, bound = bound),
        };
        app.status_ok(msg);
    }
}

/// The nearest unlocked shape point within grabbing reach of screen point `p`: its layer and index.
fn point_at(app: &App, xf: Xf, p: Pos2) -> Option<(LayerId, usize)> {
    shown(app).into_iter().rev().filter(|s| !s.locked).find_map(|s| s.points.iter().position(|(at, _)| xf.to_screen(*at).distance(p) <= GRAB).map(|i| (s.layer, i)))
}

/// Sends dabs at stage points `at` with the brush in hand (inside the stroke's gesture).
fn dab(app: &mut App, bone: BoneId, at: &[Vec2]) {
    if at.is_empty() {
        return;
    }
    let o = app.bind;
    let pts: Vec<[f32; 2]> = at.iter().map(|p| [p.x, p.y]).collect();
    app.run("bone.paint_weights", json!({"bone": bone.0, "at": pts, "radius": o.radius, "strength": o.strength, "mode": o.brush.key()}));
}

/// A Bind tool click at screen point `p`: picks the bone there, or binds the point there to the
/// picked bone (with Paint Weights on, paints one dab there instead).
pub fn click(app: &mut App, xf: Xf, p: Pos2, shift: bool, command: bool) {
    if let Some((layer, id, _)) = bone_tool::hit(app, xf, p) {
        bone_tool::select(app, layer, Some(id));
        return;
    }
    let hit = point_at(app, xf, p);
    if app.bind.paint {
        let Some(bone) = picked_shape_bone(app) else {
            app.status_ok(tr!("Pick a bone of a shape first (click it), then paint its weights."));
            return;
        };
        if let Some((_, i)) = hit {
            app.bind.point = Some(i);
        }
        app.s.begin_gesture(tr_noop!("Paint Weights"));
        dab(app, bone, &[xf.to_stage(p)]);
        app.s.end_gesture();
        return;
    }
    if let Some((layer, i)) = hit {
        match picked_in(app, layer) {
            Some(bone) => {
                app.bind.point = Some(i);
                bind(app, bone, vec![i], shift, command);
            }
            None => app.status_ok(tr!("Pick a bone of this shape first (click it), then click the points it should move.")),
        }
        return;
    }
    app.bone_sel = None;
    app.bind.point = None;
}

/// A Bind tool drag from screen point `press`: a box that binds the points inside it (a drag
/// that starts on a bone or a point does nothing more than a click), or with Paint Weights on a
/// brush stroke.
pub fn drag_start(app: &mut App, xf: Xf, press: Pos2) -> Drag {
    let sp = xf.to_stage(press);
    if app.bone_sel.is_none() {
        app.status_ok(if app.bind.paint { tr!("Pick a bone of a shape first (click it), then paint its weights.") } else { tr!("Pick a bone of a shape first (click it), then drag a box around the points it should move.") });
        return Drag::None;
    }
    if app.bind.paint {
        let Some(bone) = picked_shape_bone(app) else { return Drag::None };
        // One stroke, one undo step.
        app.s.begin_gesture(tr_noop!("Paint Weights"));
        dab(app, bone, &[sp]);
        return Drag::BindPaint { last: pos2(sp.x, sp.y) };
    }
    Drag::BindMarquee { start: pos2(sp.x, sp.y), cur: pos2(sp.x, sp.y) }
}

/// The brush stroke reaching stage point `cur` from `last`: a dab every quarter of the brush's
/// radius along the way, so a stroke paints alike however fast the pointer moves. Returns where
/// the last dab went.
pub fn paint_to(app: &mut App, last: Pos2, cur: Pos2) -> Pos2 {
    let Some(bone) = picked_shape_bone(app) else { return last };
    let spacing = (app.bind.radius * 0.25).max(0.5);
    let (a, b) = (Vec2::new(last.x, last.y), Vec2::new(cur.x, cur.y));
    let d = a.distance(b);
    if d.is_nan() || d < spacing {
        return last;
    }
    let dir = (b - a) * (1.0 / d);
    let n = ((d / spacing) as usize).min(MOST_DABS);
    let at: Vec<Vec2> = (1..=n).map(|k| a + dir * (spacing * k as f32)).collect();
    let end = at.last().copied().unwrap_or(a);
    dab(app, bone, &at);
    pos2(end.x, end.y)
}

/// Ends a brush stroke: one undo step, and a hint.
pub fn paint_end(app: &mut App) {
    app.s.end_gesture();
    if let Some(bone) = picked_shape_bone(app) {
        let n = app.s.timeline().layers.iter().find_map(|l| l.armature.as_ref().and_then(|a| a.skin.as_ref()).filter(|_| l.armature.as_ref().is_some_and(|a| a.bone(bone).is_some())).map(|k| k.points.iter().filter(|b| b.weight(bone) > 0.0).count()));
        if let Some(n) = n {
            app.status_ok(trn!("The bone moves {n} point. {undo} to undo the stroke.", "The bone moves {n} points. {undo} to undo the stroke.", n, undo = app.undo_key()));
        }
    }
}

/// Ends a box drag: binds every point inside it (on the picked bone's shape) to the bone.
pub fn drag_end(app: &mut App, start: Pos2, cur: Pos2, shift: bool, command: bool) {
    let r = Rect::from_two_pos(start, cur);
    for s in shown(app).into_iter().filter(|s| !s.locked) {
        let Some(bone) = picked_in(app, s.layer) else { continue };
        let inside: Vec<usize> = s.points.iter().enumerate().filter(|(_, (at, _))| r.contains(pos2(at.x, at.y))).map(|(i, _)| i).collect();
        if inside.is_empty() {
            app.status_ok(tr!("No points of the bone's shape inside the box."));
        } else {
            bind(app, bone, inside, shift, command);
        }
        return;
    }
}

/// Every shape armature's points (with the Bind tool in hand), and the box being dragged; with
/// Paint Weights on, the picked bone's weights as a heat map and the brush under the pointer.
pub fn draw(app: &App, painter: &egui::Painter, xf: Xf) {
    if app.tool != Tool::Bind {
        return;
    }
    for s in shown(app) {
        let picked = picked_in(app, s.layer);
        let fade = |c: Color32| if s.locked { c.gamma_multiply(0.5) } else { c };
        if let (true, Some(bone)) = (app.bind.paint, picked) {
            let w = |i: usize| s.points.get(i).and_then(|(_, b)| b.iter().find(|x| x.0 == bone)).map_or(0.0, |x| x.1);
            // The outline, each segment blending the heat of the anchors it joins.
            for (pts, a, b) in &s.outline {
                let n = pts.len().saturating_sub(1).max(1) as f32;
                for (k, seg) in pts.windows(2).enumerate() {
                    if let [p, q] = seg {
                        let t = (k as f32 + 0.5) / n;
                        let c = crate::theme::heat(w(*a) * (1.0 - t) + w(*b) * t);
                        painter.line_segment([xf.to_screen(*p), xf.to_screen(*q)], Stroke::new(3.0, fade(c)));
                    }
                }
            }
            for (i, (at, _)) in s.points.iter().enumerate() {
                let c = xf.to_screen(*at);
                painter.circle(c, SIZE + 0.5, fade(crate::theme::heat(w(i))), Stroke::new(1.0, fade(T.ink)));
            }
        } else {
            for (at, bones) in &s.points {
                let c = xf.to_screen(*at);
                let mine = picked.is_some_and(|b| bones.iter().any(|x| x.0 == b));
                let edge = Stroke::new(1.0, fade(if mine { T.selection } else { s.color }));
                let fill = fade(if mine { T.selection } else { Color32::WHITE });
                if bones.len() > 1 {
                    // Shared by several bones: a triangle.
                    let pts = vec![c + vec2(0.0, -SIZE * 1.2), c + vec2(SIZE * 1.1, SIZE * 0.8), c + vec2(-SIZE * 1.1, SIZE * 0.8)];
                    painter.add(egui::Shape::convex_polygon(pts, fill, edge));
                } else {
                    let r = Rect::from_center_size(c, vec2(SIZE * 2.0, SIZE * 2.0));
                    painter.rect_filled(r, 0.0, fill);
                    painter.rect_stroke(r, 0.0, edge, StrokeKind::Middle);
                }
            }
        }
        // The focused point (its weights are in Properties), ringed.
        if picked.is_some()
            && let Some((at, _)) = app.bind.point.and_then(|i| s.points.get(i))
        {
            painter.circle_stroke(xf.to_screen(*at), SIZE + 4.0, Stroke::new(1.5, T.selection));
        }
    }
    if let Drag::BindMarquee { start, cur } = app.drag {
        let r = Rect::from_two_pos(xf.to_screen(Vec2::new(start.x, start.y)), xf.to_screen(Vec2::new(cur.x, cur.y)));
        painter.rect_filled(r, 0.0, T.selection.gamma_multiply(0.12));
        painter.rect_stroke(r, 0.0, Stroke::new(1.0, T.selection), StrokeKind::Middle);
    }
    // The brush: its reach, and a dot where its strength is full.
    let stroke_at = match app.drag {
        Drag::BindPaint { last } => Some(xf.to_screen(Vec2::new(last.x, last.y))),
        _ => app.bind.preview.map(|p| xf.to_screen(p)),
    };
    if app.bind.paint
        && let Some(p) = painter.ctx().pointer_hover_pos().filter(|p| painter.clip_rect().contains(*p)).or(stroke_at)
    {
        let r = app.bind.radius * xf.zoom;
        painter.circle_stroke(p, r, Stroke::new(1.5, T.ink));
        painter.circle_stroke(p, r, Stroke::new(0.75, Color32::WHITE));
        painter.circle_filled(p, 1.5, T.selection);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anicraft_engine::Session;
    use anicraft_engine::doc::Document;

    /// An app with the `shapeweights` tail: its second bone picked, Paint Weights on.
    fn app() -> (App, BoneId) {
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, Session::new(Document::default()), Some(std::path::PathBuf::from("unused.png")));
        app.open_panel(Some("shapeweights"));
        app.s.end_gesture();
        app.drag = Drag::None;
        let bone = app.bone_sel.expect("a picked bone");
        (app, bone)
    }

    fn weights(app: &App, bone: BoneId) -> Vec<f32> {
        app.s.timeline().layers.iter().find_map(|l| l.armature.as_ref().and_then(|a| a.skin.as_ref())).map(|k| k.points.iter().map(|b| b.weight(bone)).collect()).unwrap_or_default()
    }

    #[test]
    fn a_stroke_dabs_every_quarter_radius_and_undoes_in_one_step() {
        let (mut app, bone) = app();
        app.bind.radius = 40.0;
        let before = weights(&app, bone);
        let undo = app.s.undo_label().map(str::to_string);
        app.s.begin_gesture("Paint Weights");
        // Less than a quarter of the radius: no dab yet, and the stroke stays where it was.
        let last = paint_to(&mut app, pos2(600.0, 280.0), pos2(605.0, 280.0));
        assert_eq!(last, pos2(600.0, 280.0));
        assert_eq!(weights(&app, bone), before);
        // 45 px: four dabs 10 px apart, the stroke at the fourth.
        let last = paint_to(&mut app, last, pos2(645.0, 280.0));
        assert_eq!(last, pos2(640.0, 280.0));
        paint_end(&mut app);
        assert_ne!(weights(&app, bone), before);
        assert_eq!(app.s.undo_label(), Some("Paint Weights"));
        app.s.undo().expect("undo");
        assert_eq!(weights(&app, bone), before);
        assert_eq!(app.s.undo_label().map(str::to_string), undo);
    }

    #[test]
    fn heat_runs_from_blue_to_red() {
        use crate::theme::{HEAT, heat};
        assert_eq!(heat(0.0), HEAT[0]);
        assert_eq!(heat(1.0), HEAT[4]);
        assert_eq!(heat(0.5), HEAT[2]);
        assert_eq!(heat(f32::NAN), HEAT[0]);
        assert_eq!(heat(7.0), HEAT[4]);
    }
}
