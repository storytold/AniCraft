//! Envelope (ADR 0080): Modify › Transform › Envelope puts a mesh of handles around the selected
//! shapes and drawing objects, with the Free Transform tool, as in Animate. Squares are the
//! corners, circles the tangent handles of each side (a corner's tangents move with it).
//! Dragging one bends the shapes through `modify.envelope_move`: one undo step per drag. The
//! envelope ends when the selection changes or another tool is chosen; bent raw shapes then
//! merge with the drawing they lie on (`modify.envelope_close`).

use crate::app::{App, Drag, Tool};
use crate::pathtools::Xf;
use crate::theme::T;
use anicraft_engine::geom::envelope::{BOUNDARY, CORNERS, Mesh, coons_interior};
use anicraft_engine::geom::{Rect, Vec2};
use egui::{Color32, Pos2, Stroke, StrokeKind, vec2};
use serde_json::json;

/// Pick radius for handles, in screen points.
const PICK: f32 = 7.0;
/// Handle size, in screen points.
const HANDLE: f32 = 7.0;

/// The open envelope's rest rectangle and mesh, while the Free Transform tool shows it.
pub fn shown(app: &App) -> Option<(Rect, Mesh)> {
    (app.tool == Tool::Transform).then(|| app.s.envelope()).flatten().map(|(_, rest, mesh)| (rest, mesh))
}

/// Opens an envelope on the selection (Modify › Transform › Envelope) and takes the Free
/// Transform tool, which shows its handles.
pub fn open(app: &mut App) {
    if app.run("modify.envelope", json!({})).is_some() {
        app.tool = Tool::Transform;
        app.envelope_open = true;
        app.status_ok(tr!("Drag the handles to bend the shapes. Squares move a corner, circles bend a side."));
    }
}

/// Ends an envelope that is no longer shown (another tool, another selection): bent raw shapes
/// merge where they lie. Runs every frame, before the stage handles input.
pub fn settle(app: &mut App) {
    if app.envelope_open && app.drag == Drag::None && shown(app).is_none() {
        app.envelope_open = false;
        // Ended by a new selection: that stays. By another tool: what the merge selected.
        let keep = (app.tool == Tool::Transform).then(|| app.s.selection.clone());
        app.run("modify.envelope_close", json!({}));
        if let Some(keep) = keep {
            app.s.selection = keep;
        }
    }
}

/// The handle under screen point `p`.
fn hit(mesh: &Mesh, xf: Xf, p: Pos2) -> Option<usize> {
    BOUNDARY
        .iter()
        .copied()
        .filter_map(|i| mesh.get(i).map(|q| (i, xf.to_screen(*q).distance(p))))
        .filter(|(_, d)| *d <= PICK)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// Whether a drag from `press` grabs a handle (the cursor shows a grab there).
pub fn over_handle(app: &App, xf: Xf, p: Pos2) -> bool {
    shown(app).is_some_and(|(_, mesh)| hit(&mesh, xf, p).is_some())
}

/// A drag on a handle starts one undo step.
pub fn drag_start(app: &mut App, xf: Xf, press: Pos2) -> Option<Drag> {
    let (_, mesh) = shown(app)?;
    let index = hit(&mesh, xf, press)?;
    app.s.begin_gesture(tr_noop!("Envelope"));
    Some(Drag::Envelope { index, mesh, press: xf.to_stage(press) })
}

/// The tangent handles beside corner `i` (they move with it).
fn tangents(i: usize) -> [usize; 2] {
    match i {
        0 => [1, 4],
        3 => [2, 7],
        15 => [11, 14],
        12 => [8, 13],
        _ => [i, i],
    }
}

/// `start` with handle `index` moved by `d` (a corner takes its tangents along), the inside
/// following the sides.
pub fn moved(start: &Mesh, index: usize, d: Vec2) -> Mesh {
    let mut m = *start;
    let mut grip = vec![index];
    if CORNERS.contains(&index) {
        grip.extend(tangents(index));
    }
    grip.sort_unstable();
    grip.dedup();
    for i in grip {
        if let (Some(slot), Some(s)) = (m.get_mut(i), start.get(i)) {
            *slot = *s + d;
        }
    }
    coons_interior(&mut m);
    m
}

pub fn drag_to(app: &mut App, drag: Drag, cur: Vec2) {
    let Drag::Envelope { index, mesh, press } = drag else { return };
    let m = moved(&mesh, index, cur - press);
    let points: Vec<[f32; 2]> = m.iter().map(|p| [p.x, p.y]).collect();
    app.run("modify.envelope_move", json!({"points": points, "tolerance": 0.05}));
}

pub fn drag_end(app: &mut App) {
    app.s.end_gesture();
}

/// The mesh's outline (its four sides as cubics), tangent lines and handles.
pub fn draw_overlay(app: &App, painter: &egui::Painter, xf: Xf, pointer: Option<Pos2>) {
    let Some((_, mesh)) = shown(app) else { return };
    let at = |i: usize| xf.to_screen(mesh.get(i).copied().unwrap_or(Vec2::ZERO));
    let line = Stroke::new(1.5, T.selection);
    let thin = Stroke::new(1.0, T.selection);
    // The sides, clockwise: top, right, bottom, left.
    for side in [[0, 1, 2, 3], [3, 7, 11, 15], [15, 14, 13, 12], [12, 8, 4, 0]] {
        let [a, b, c, d] = side.map(at);
        painter.add(egui::epaint::CubicBezierShape::from_points_stroke([a, b, c, d], false, Color32::TRANSPARENT, line));
        painter.line_segment([a, b], thin);
        painter.line_segment([d, c], thin);
    }
    let hot = pointer.and_then(|p| hit(&mesh, xf, p));
    for i in BOUNDARY {
        let p = at(i);
        let fill = if hot == Some(i) { T.selection } else { Color32::WHITE };
        if CORNERS.contains(&i) {
            let r = egui::Rect::from_center_size(p, vec2(HANDLE, HANDLE));
            painter.rect_filled(r, 0.0, fill);
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, T.selection), StrokeKind::Middle);
        } else {
            painter.circle(p, HANDLE * 0.45, fill, Stroke::new(1.0, T.selection));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anicraft_engine::geom::envelope::rest_mesh;

    /// A corner drags its two tangents along; a tangent moves alone; the inside follows.
    #[test]
    fn corners_take_their_tangents() {
        let rest = rest_mesh(&Rect::new(0.0, 0.0, 90.0, 60.0));
        let d = Vec2::new(10.0, -5.0);
        let m = moved(&rest, 0, d);
        for i in [0usize, 1, 4] {
            assert_eq!(m[i], rest[i] + d, "{i}");
        }
        assert_eq!(m[2], rest[2]);
        assert_eq!(m[3], rest[3]);
        let t = moved(&rest, 2, d);
        assert_eq!(t[2], rest[2] + d);
        assert_eq!(t[1], rest[1]);
        assert_ne!(t[6], rest[6], "the inner points follow the side");
        // No move: the rest mesh.
        assert_eq!(moved(&rest, 7, Vec2::ZERO), rest);
    }
}
