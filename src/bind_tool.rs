//! The Bind tool (ADR 0071): which points of a shape follow which of its bones.
//!
//! With the tool in hand, every shape armature on the stage shows its points: a square for a
//! point that follows one bone, a triangle for one that several bones share (as in Animate).
//! Click a bone to pick it: the points it moves fill in the selection colour. Then a click on a
//! point makes it follow only that bone, Shift-click shares it with the bones it follows,
//! Ctrl-click (⌘ on macOS) takes it from the bone, and a box dragged on empty stage does the
//! same to every point inside it. Each is one `bone.bind`, one undo step.

use crate::app::{App, Drag};
use crate::bone_tool;
use crate::pathtools::Xf;
use crate::theme::{T, to_color32};
use anicraft_engine::doc::skin::points_shown;
use anicraft_engine::doc::{BoneId, LayerId, LayerKind, evaluate_layer_elements};
use anicraft_engine::geom::Vec2;
use egui::{Color32, Pos2, Rect, Stroke, StrokeKind, pos2, vec2};
use serde_json::json;

/// How close (screen points) a click must be to a point to bind it, and the size points draw.
const GRAB: f32 = 6.0;
const SIZE: f32 = 3.5;

/// One shape armature's points as the stage shows them at the playhead.
struct Points {
    layer: LayerId,
    color: Color32,
    locked: bool,
    /// Stage position, the bones it follows.
    points: Vec<(Vec2, Vec<BoneId>)>,
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
            let points = at.into_iter().zip(&skin.points).map(|(p, b)| (p, b.bones.iter().map(|i| i.0).collect())).collect();
            Some(Points { layer: l.id, color: to_color32(l.color), locked: l.locked || tl.locked_by_folder(l.id), points })
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

/// A Bind tool click at screen point `p`: picks the bone there, or binds the point there to the
/// picked bone.
pub fn click(app: &mut App, xf: Xf, p: Pos2, shift: bool, command: bool) {
    if let Some((layer, id, _)) = bone_tool::hit(app, xf, p) {
        bone_tool::select(app, layer, Some(id));
        return;
    }
    for s in shown(app).into_iter().rev().filter(|s| !s.locked) {
        let Some(i) = s.points.iter().position(|(at, _)| xf.to_screen(*at).distance(p) <= GRAB) else { continue };
        match picked_in(app, s.layer) {
            Some(bone) => bind(app, bone, vec![i], shift, command),
            None => app.status_ok(tr!("Pick a bone of this shape first (click it), then click the points it should move.")),
        }
        return;
    }
    app.bone_sel = None;
}

/// A Bind tool drag from screen point `press`: a box that binds the points inside it (a drag
/// that starts on a bone or a point does nothing more than a click).
pub fn drag_start(app: &mut App, xf: Xf, press: Pos2) -> Drag {
    let sp = xf.to_stage(press);
    if app.bone_sel.is_none() {
        app.status_ok(tr!("Pick a bone of a shape first (click it), then drag a box around the points it should move."));
        return Drag::None;
    }
    Drag::BindMarquee { start: pos2(sp.x, sp.y), cur: pos2(sp.x, sp.y) }
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

/// Every shape armature's points (with the Bind tool in hand), and the box being dragged.
pub fn draw(app: &App, painter: &egui::Painter, xf: Xf) {
    if app.tool != crate::app::Tool::Bind {
        return;
    }
    for s in shown(app) {
        let picked = picked_in(app, s.layer);
        let fade = |c: Color32| if s.locked { c.gamma_multiply(0.5) } else { c };
        for (at, bones) in &s.points {
            let c = xf.to_screen(*at);
            let mine = picked.is_some_and(|b| bones.contains(&b));
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
    if let Drag::BindMarquee { start, cur } = app.drag {
        let r = Rect::from_two_pos(xf.to_screen(Vec2::new(start.x, start.y)), xf.to_screen(Vec2::new(cur.x, cur.y)));
        painter.rect_filled(r, 0.0, T.selection.gamma_multiply(0.12));
        painter.rect_stroke(r, 0.0, Stroke::new(1.0, T.selection), StrokeKind::Middle);
    }
}
