//! The Selection tool on raw shapes (ADR 0041) through the commands: bending, shared points,
//! snapping and the marquee.

use crate::Session;
use anicraft_doc::{Element, ElementKind};
use anicraft_geom::{Affine, Vec2};
use serde_json::{Value, json};
use std::sync::Arc;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// A blank document drawing raw shapes, with a 100 × 80 rectangle (fill and outline) at the origin.
fn rect_session() -> Session {
    let mut s = Session::new(anicraft_doc::Document::default());
    run(&mut s, "draw.object_drawing", json!({"on": false}));
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 80, "fill": "#ff0000", "stroke": "#000000"}));
    run(&mut s, "edit.deselect", json!({}));
    s
}

fn raw(s: &Session) -> Vec<Arc<Element>> {
    s.visible_elements().into_iter().filter(|e| matches!(&e.kind, ElementKind::Shape(sh) if sh.merge)).collect()
}

fn is_fill(e: &Element) -> bool {
    matches!(&e.kind, ElementKind::Shape(sh) if sh.fill.is_some())
}

fn count(s: &Session) -> (usize, usize) {
    let r = raw(s);
    (r.iter().filter(|e| is_fill(e)).count(), r.iter().filter(|e| !is_fill(e)).count())
}

fn polylines(e: &Element) -> Vec<anicraft_geom::Polyline> {
    match &e.kind {
        ElementKind::Shape(sh) => sh.path.flatten(&e.transform, 0.01),
        _ => Vec::new(),
    }
}

fn area(e: &Element) -> f32 {
    polylines(e)
        .iter()
        .map(|pl| {
            let n = pl.points.len();
            (0..n).map(|i| pl.points[i].cross(pl.points[(i + 1) % n])).sum::<f32>() * 0.5
        })
        .sum::<f32>()
        .abs()
}

fn length(e: &Element) -> f32 {
    polylines(e).iter().map(|pl| pl.points.windows(2).map(|w| w[0].distance(w[1])).sum::<f32>()).sum()
}

fn fill_area(s: &Session) -> f32 {
    raw(s).iter().filter(|e| is_fill(e)).map(|e| area(e)).sum()
}

/// Stage-space anchors of a raw shape.
fn anchors(e: &Element) -> Vec<Vec2> {
    match &e.kind {
        ElementKind::Shape(sh) => sh.path.anchors().into_iter().map(|(_, p)| e.transform.apply(p)).collect(),
        _ => Vec::new(),
    }
}

/// Every stroke end lies on a fill corner or another stroke's end: the drawing is still joined
/// (for drawings without loose ends).
fn joined(s: &Session) -> bool {
    let r = raw(s);
    let corners: Vec<Vec2> = r.iter().flat_map(|e| anchors(e)).collect();
    r.iter().filter(|e| !is_fill(e)).all(|e| {
        let a = anchors(e);
        [a.first(), a.last()].into_iter().flatten().all(|p| corners.iter().filter(|q| q.distance(*p) < 1e-3).count() >= 2)
    })
}

fn has_anchor(s: &Session, p: Vec2) -> usize {
    raw(s).iter().filter(|e| anchors(e).iter().any(|q| q.distance(p) < 1e-3)).count()
}

// ----- bending -----------------------------------------------------------------------------------

#[test]
fn dragging_a_segment_bends_it_through_the_dragged_point_and_the_fill_follows() {
    let mut s = rect_session();
    let history = s.history_len();
    // Drag the top edge's middle up 30: the edge becomes a curve through (50, −30).
    let r = run(&mut s, "path.bend_segment", json!({"x": 50, "y": 0, "dx": 0, "dy": -30}));
    assert_eq!(r["merged"], true);
    assert!((r["x"].as_f64().unwrap_or(0.0) - 50.0).abs() < 1e-3 && (r["y"].as_f64().unwrap_or(0.0) + 30.0).abs() < 1e-3, "{r}");
    assert_eq!(count(&s), (1, 4), "still a fill and four sides");
    // The top is a curve through the dragged point.
    let top = raw(&s).into_iter().find(|e| !is_fill(e) && polylines(e).iter().any(|pl| pl.points.iter().any(|p| p.y < -29.0)));
    let Some(top) = top else { panic!("no bent stroke") };
    // (A symmetric pull: the dragged point is the curve's peak.)
    let peak = polylines(&top).iter().flat_map(|pl| pl.points.clone()).min_by(|a, b| a.y.total_cmp(&b.y)).unwrap_or(Vec2::ZERO);
    assert!((peak.y + 30.0).abs() < 0.02 && (peak.x - 50.0).abs() < 2.0, "the curve peaks at the dragged point: {peak:?}");
    // The fill bulges with it, by exactly the area under the cubic: L·(h₁ + h₂)/4 with both
    // handles raised 40 (a pull of 30 at the middle needs 30 / 0.75).
    assert!((fill_area(&s) - (8000.0 + 100.0 * 80.0 / 4.0)).abs() < 2.0, "{}", fill_area(&s));
    assert!(joined(&s));
    // One undo step takes it back.
    assert_eq!(s.history_len(), history + 1);
    run(&mut s, "edit.undo", json!({}));
    assert!((fill_area(&s) - 8000.0).abs() < 0.5);
}

#[test]
fn a_straight_segment_stays_straight_until_it_is_dragged() {
    let mut s = rect_session();
    run(&mut s, "path.bend_segment", json!({"x": 50, "y": 0, "dx": 0, "dy": -30}));
    // The other three sides are still lines.
    let lines = raw(&s).iter().filter(|e| !is_fill(e)).filter(|e| matches!(&e.kind, ElementKind::Shape(sh) if sh.path.cmds.iter().all(|c| !matches!(c, anicraft_geom::PathCmd::CubicTo(..))))).count();
    assert_eq!(lines, 3);
}

#[test]
fn a_drag_in_steps_bends_like_one_move_and_merges_at_the_end() {
    let mut s = rect_session();
    s.begin_gesture("Bend");
    let first = run(&mut s, "path.bend_segment", json!({"x": 100, "y": 40, "dx": 10, "dy": 0, "merge": false}));
    let (id, cmd, t) = (first["id"].clone(), first["cmd"].clone(), first["t"].clone());
    // Later steps name the segment and pass the point where it now is.
    run(&mut s, "path.bend_segment", json!({"x": 110, "y": 40, "dx": 10, "dy": 0, "id": id, "cmd": cmd, "t": t, "merge": false}));
    let ids: Vec<u64> = raw(&s).iter().map(|e| e.id.0).collect();
    run(&mut s, "raw.merge", json!({"ids": ids}));
    s.end_gesture();
    let far = raw(&s).iter().flat_map(|e| polylines(e)).flat_map(|pl| pl.points).fold(0.0f32, |m, p| m.max(p.x));
    assert!((far - 120.0).abs() < 0.1, "the side's middle went to x = 120: {far}");
    assert!(joined(&s));
    assert!((fill_area(&s) - (8000.0 + 80.0 * (2.0 * 20.0 / 0.75) / 4.0)).abs() < 2.0, "{}", fill_area(&s));
}

#[test]
fn bending_an_object_bends_only_it() {
    let mut s = Session::new(anicraft_doc::Document::default());
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 80, "fill": "#ff0000", "stroke": null}));
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 80, "fill": "#00ff00", "stroke": null}));
    let r = run(&mut s, "path.bend_segment", json!({"x": 0, "y": 40, "dx": -20, "dy": 0}));
    assert_eq!(r["merged"], false);
    assert_eq!(r["changed"].as_array().map(Vec::len), Some(1), "objects don't share edges");
    // The closing edge of an object's outline can be bent too.
    let areas: Vec<f32> = s.visible_elements().iter().map(|e| area(e)).collect();
    assert!(areas.iter().any(|a| (a - 8000.0).abs() < 0.5) && areas.iter().any(|a| (a - (8000.0 + 80.0 * (40.0 / 0.75) / 4.0)).abs() < 2.0), "{areas:?}");
}

// ----- shared points ------------------------------------------------------------------------------

#[test]
fn moving_a_corner_moves_every_shape_that_shares_it() {
    let mut s = rect_session();
    let history = s.history_len();
    let r = run(&mut s, "raw.move_point", json!({"x": 100, "y": 0, "to_x": 130, "to_y": -20}));
    assert_eq!(r["moved"], 3, "the fill and the two sides that meet there: {r}");
    let to = Vec2::new(130.0, -20.0);
    assert_eq!(has_anchor(&s, Vec2::new(100.0, 0.0)), 0, "nothing stays behind");
    assert_eq!(has_anchor(&s, to), 3);
    assert!(joined(&s));
    // The fill is the quadrilateral through the new corner (shoelace).
    let quad = [Vec2::new(0.0, 0.0), to, Vec2::new(100.0, 80.0), Vec2::new(0.0, 80.0)];
    let want = (0..4).map(|i| quad[i].cross(quad[(i + 1) % 4])).sum::<f32>().abs() * 0.5;
    assert!((fill_area(&s) - want).abs() < 0.5, "{} vs {want}", fill_area(&s));
    assert_eq!(s.history_len(), history + 1, "one undo step");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(has_anchor(&s, Vec2::new(100.0, 0.0)), 3);
}

#[test]
fn a_line_ending_on_an_edge_drags_the_edge_with_it() {
    let mut s = rect_session();
    // A line from inside the rectangle up to its top edge: they meet at (50, 0).
    run(&mut s, "draw.line", json!({"x0": 50, "y0": -40, "x1": 50, "y1": 0, "stroke": "#000000"}));
    run(&mut s, "edit.deselect", json!({}));
    let r = run(&mut s, "raw.move_point", json!({"x": 50, "y": -40, "to_x": 50, "to_y": -60}));
    assert_eq!(r["moved"], 1, "a free end moves alone");
    // The point where it meets the edge: the fill and both halves of the top move with it.
    let r = run(&mut s, "raw.move_point", json!({"x": 50, "y": 0, "to_x": 50, "to_y": 10}));
    assert!(r["moved"].as_u64().unwrap_or(0) >= 3, "{r}");
    assert!(has_anchor(&s, Vec2::new(50.0, 10.0)) >= 3);
    assert_eq!(has_anchor(&s, Vec2::new(50.0, 0.0)), 0, "nothing stays behind");
    // The fill lost the triangle the dent took: 100 wide, 10 deep.
    assert!((fill_area(&s) - (8000.0 - 0.5 * 100.0 * 10.0)).abs() < 0.5, "{}", fill_area(&s));
}

#[test]
fn a_point_drag_passes_its_points_on_and_merges_at_the_end() {
    let mut s = rect_session();
    s.begin_gesture("Move Point");
    let r = run(&mut s, "raw.move_point", json!({"x": 0, "y": 80, "to_x": -5, "to_y": 85, "merge": false}));
    let points = r["points"].clone();
    assert_eq!(points.as_array().map(Vec::len), Some(3));
    run(&mut s, "raw.move_point", json!({"x": -5, "y": 85, "to_x": -10, "to_y": 90, "points": points, "merge": false}));
    let ids: Vec<u64> = raw(&s).iter().map(|e| e.id.0).collect();
    run(&mut s, "raw.merge", json!({"ids": ids}));
    s.end_gesture();
    assert_eq!(has_anchor(&s, Vec2::new(-10.0, 90.0)), 3);
    assert!(joined(&s));
}

// ----- snapping -------------------------------------------------------------------------------------

/// A raw rectangle and a separate raw line whose free end is dragged.
fn snap_session() -> Session {
    let mut s = rect_session();
    run(&mut s, "draw.line", json!({"x0": 200, "y0": 0, "x1": 300, "y1": 0, "stroke": "#0000ff"}));
    run(&mut s, "edit.deselect", json!({}));
    s
}

#[test]
fn a_dragged_point_snaps_within_ten_screen_pixels_at_any_zoom() {
    // The drop point is 8 document units from the rectangle's corner (100, 0).
    let drop = json!({"x": 200, "y": 0, "to_x": 108, "to_y": 0});
    for (zoom, snaps) in [(1.0, true), (2.0, false), (0.5, true), (4.0, false), (0.8, true)] {
        let mut s = snap_session();
        let mut p = drop.clone();
        p["snap"] = json!(10.0 / zoom);
        let r = run(&mut s, "raw.move_point", p);
        assert_eq!(r["snapped"].is_object(), snaps, "zoom {zoom}: {r}");
        if snaps {
            assert_eq!(r["snapped"]["to"], "point");
            assert_eq!(has_anchor(&s, Vec2::new(100.0, 0.0)), 4, "zoom {zoom}: the line's end joined the corner");
        } else {
            assert_eq!(has_anchor(&s, Vec2::new(108.0, 0.0)), 1, "zoom {zoom}: where it was let go");
        }
    }
}

#[test]
fn a_dragged_point_snaps_onto_a_line_without_changing_the_line() {
    let mut s = snap_session();
    // 6 units below the rectangle's bottom edge, away from its corners.
    let r = run(&mut s, "raw.move_point", json!({"x": 200, "y": 0, "to_x": 40, "to_y": 86, "snap": 10}));
    assert_eq!(r["snapped"]["to"], "line", "{r}");
    assert!((r["y"].as_f64().unwrap_or(0.0) - 80.0).abs() < 1e-3);
    // The bottom is still straight along y = 80 and the fill the same.
    assert!((fill_area(&s) - 8000.0).abs() < 0.5, "{}", fill_area(&s));
    let bottom: Vec<Vec2> = raw(&s).iter().flat_map(|e| anchors(e)).filter(|p| (p.y - 80.0).abs() < 1.0).collect();
    assert!(bottom.iter().all(|p| (p.y - 80.0).abs() < 1e-3), "{bottom:?}");
    assert!(has_anchor(&s, Vec2::new(40.0, 80.0)) >= 2, "the edge gained a point where the line meets it");
}

#[test]
fn a_dragged_selection_snaps_its_nearest_corner() {
    let mut s = snap_session();
    // Select the loose line and drag it by its left end towards the rectangle's corner.
    run(&mut s, "element.pick", json!({"x": 250, "y": 0}));
    let r = run(&mut s, "element.translate", json!({"dx": -93, "dy": 4, "snap": 10, "from": [107, 4]}));
    assert_eq!(r["snapped"]["to"], "point", "{r}");
    assert!((r["dx"].as_f64().unwrap_or(0.0) + 100.0).abs() < 1e-3 && r["dy"].as_f64().unwrap_or(1.0).abs() < 1e-3, "{r}");
    // Out of reach, nothing snaps.
    let r = run(&mut s, "element.translate", json!({"dx": 0, "dy": -40, "snap": 10, "from": [100, -40]}));
    assert!(r["snapped"].is_null(), "{r}");
}

#[test]
fn arrow_key_nudges_join_ends_within_point_two_at_their_midpoint() {
    let mut s = snap_session();
    // Nudge the loose line left until its end is 0.15 from the corner (100, 0) and drop it, as
    // the arrow keys do at 1000 % (0.1 per press).
    run(&mut s, "element.pick", json!({"x": 250, "y": 0}));
    run(&mut s, "element.translate", json!({"dx": -99.85, "dy": 0}));
    run(&mut s, "draw.drop", json!({}));
    let mid = Vec2::new(100.075, 0.0);
    assert_eq!(has_anchor(&s, mid), 4, "the corner and the end meet halfway");
    assert_eq!(has_anchor(&s, Vec2::new(100.0, 0.0)), 0);
}

#[test]
fn arrow_key_nudges_put_an_end_onto_a_line_that_stays_straight() {
    let mut s = rect_session();
    run(&mut s, "draw.line", json!({"x0": 40, "y0": -60, "x1": 40, "y1": -10, "stroke": "#0000ff"}));
    // Down until the end is 0.12 above the top edge, then let go.
    run(&mut s, "element.translate", json!({"dx": 0, "dy": 9.88}));
    run(&mut s, "draw.drop", json!({}));
    let top: Vec<Vec2> = raw(&s).iter().flat_map(|e| anchors(e)).filter(|p| p.y.abs() < 1.0).collect();
    assert!(top.iter().all(|p| p.y.abs() < 1e-4), "the top stays on y = 0: {top:?}");
    assert!(has_anchor(&s, Vec2::new(40.0, 0.0)) >= 2, "the end is on the edge");
    assert!((fill_area(&s) - 8000.0).abs() < 0.5);
}

// ----- the marquee ----------------------------------------------------------------------------------

#[test]
fn a_marquee_selects_the_parts_under_it_and_the_parts_add_up() {
    let mut s = rect_session();
    run(&mut s, "draw.rect", json!({"x": 300, "y": 0, "w": 20, "h": 20, "object": true}));
    run(&mut s, "draw.rect", json!({"x": 150, "y": 50, "w": 200, "h": 10, "object": true}));
    let history = s.history_len();
    let dirty = s.is_dirty();
    // The right half of the rectangle, the small object wholly, the long one not.
    let r = run(&mut s, "raw.select_rect", json!({"x0": 50, "y0": -20, "x1": 330, "y1": 45}));
    assert_eq!(r["cut"], 3, "part of the fill, of the top and of the right side: {r}");
    let sel = s.selection.clone();
    let els = s.visible_elements();
    let chosen: Vec<&Arc<Element>> = els.iter().filter(|e| sel.has_id(e.id)).collect();
    assert!(chosen.iter().any(|e| matches!(&e.kind, ElementKind::Shape(sh) if !sh.merge && (area(e) - 400.0).abs() < 0.5)), "the object inside");
    assert!(!chosen.iter().any(|e| (area(e) - 2000.0).abs() < 0.5), "not the one across the edge");
    // Inside: 50 × 45 of the fill (y from 0 to 45), the right side's upper 45 and the top's
    // right 50.
    let inside: f32 = chosen.iter().filter(|e| is_fill(e) && matches!(&e.kind, ElementKind::Shape(sh) if sh.merge)).map(|e| area(e)).sum();
    assert!((inside - 50.0 * 45.0).abs() < 0.5, "{inside}");
    let lines: f32 = chosen.iter().filter(|e| matches!(&e.kind, ElementKind::Shape(sh) if sh.merge && sh.fill.is_none())).map(|e| length(e)).sum();
    assert!((lines - 95.0).abs() < 0.05, "{lines}");
    // Inside and outside add up to the whole.
    assert!((fill_area(&s) - 8000.0).abs() < 0.5, "{}", fill_area(&s));
    let total: f32 = raw(&s).iter().filter(|e| !is_fill(e)).map(|e| length(e)).sum();
    assert!((total - 360.0).abs() < 0.05, "{total}");
    // Selecting is not an edit: no undo step, and a saved document stays saved.
    assert_eq!(s.history_len(), history);
    assert_eq!(s.is_dirty(), dirty);
}

#[test]
fn letting_go_of_a_marquee_puts_the_drawing_back_whole() {
    let mut s = rect_session();
    s.mark_saved();
    let history = s.history_len();
    run(&mut s, "raw.select_rect", json!({"x0": 50, "y0": -20, "x1": 200, "y1": 200}));
    assert_eq!(count(&s), (2, 6));
    assert!(!s.is_dirty());
    // Clicking empty space: the cut goes, the drawing is as it was.
    run(&mut s, "element.pick", json!({"x": 500, "y": 500}));
    assert_eq!(count(&s), (1, 4));
    assert!(!s.is_dirty());
    assert_eq!(s.history_len(), history, "a marquee is not an edit");
    // Clicking the part outside selects the whole shape it came from.
    run(&mut s, "raw.select_rect", json!({"x0": 50, "y0": -20, "x1": 200, "y1": 200}));
    run(&mut s, "element.pick", json!({"x": 20, "y": 40}));
    assert_eq!(count(&s), (1, 4));
    assert_eq!(s.selection.len(), 1);
    assert!(raw(&s).iter().any(|e| s.selection.has_id(e.id) && is_fill(e) && (area(e) - 8000.0).abs() < 0.5));
    // A new marquee replaces the old one.
    run(&mut s, "raw.select_rect", json!({"x0": 50, "y0": -20, "x1": 200, "y1": 200}));
    run(&mut s, "raw.select_rect", json!({"x0": -20, "y0": -20, "x1": 50, "y1": 40}));
    assert_eq!(count(&s), (2, 6), "cut once, at the new marquee (the corner: a fill and two sides)");
}

#[test]
fn moving_marquee_parts_is_one_undo_step_from_the_whole_drawing() {
    let mut s = rect_session();
    let history = s.history_len();
    run(&mut s, "raw.select_rect", json!({"x0": 50, "y0": -20, "x1": 200, "y1": 200}));
    s.begin_gesture("Move");
    run(&mut s, "element.translate", json!({"dx": 40, "dy": 0}));
    run(&mut s, "draw.drop", json!({}));
    s.end_gesture();
    assert_eq!(s.history_len(), history + 1);
    // The right half moved away; the left half stayed.
    assert!((fill_area(&s) - 8000.0).abs() < 0.5);
    let far = raw(&s).iter().flat_map(|e| anchors(e)).fold(0.0f32, |m, p| m.max(p.x));
    assert!((far - 140.0).abs() < 1e-3, "{far}");
    // Undo goes straight back to the whole rectangle.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(count(&s), (1, 4));
    assert!((fill_area(&s) - 8000.0).abs() < 0.5);
    // Deleting parts is an edit too.
    run(&mut s, "raw.select_rect", json!({"x0": 50, "y0": -20, "x1": 200, "y1": 200}));
    run(&mut s, "edit.delete", json!({}));
    assert!((fill_area(&s) - 4000.0).abs() < 0.5, "{}", fill_area(&s));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(count(&s), (1, 4));
}

#[test]
fn shift_marquees_add_parts() {
    let mut s = rect_session();
    run(&mut s, "raw.select_rect", json!({"x0": -20, "y0": -20, "x1": 30, "y1": 30}));
    let first = s.selection.len();
    run(&mut s, "raw.select_rect", json!({"x0": 70, "y0": 50, "x1": 130, "y1": 100, "add": true}));
    assert!(s.selection.len() > first);
    assert!((fill_area(&s) - 8000.0).abs() < 0.5);
    // Letting go heals both cuts.
    run(&mut s, "edit.deselect", json!({}));
    assert_eq!(count(&s), (1, 4));
}

#[test]
fn a_marquee_over_a_whole_raw_shape_selects_it_without_cutting() {
    let mut s = rect_session();
    let r = run(&mut s, "raw.select_rect", json!({"x0": -10, "y0": -10, "x1": 110, "y1": 90}));
    assert_eq!(r["cut"], 0);
    assert_eq!(s.selection.len(), 5);
    assert_eq!(count(&s), (1, 4));
}

// ----- what a drag takes ------------------------------------------------------------------------------

#[test]
fn the_selection_tool_knows_corners_segments_and_fills() {
    let mut s = rect_session();
    let kind = |s: &mut Session, x: f32, y: f32| run(s, "raw.hit", json!({"x": x, "y": y, "slop": 3, "point_slop": 5}))["kind"].clone();
    assert_eq!(kind(&mut s, 101.0, 2.0), "point", "a corner");
    assert_eq!(kind(&mut s, 50.0, 1.0), "segment", "an edge");
    assert_eq!(kind(&mut s, 50.0, 40.0), "fill");
    assert_eq!(kind(&mut s, 500.0, 40.0), Value::Null);
    // Once selected, a drag moves it.
    run(&mut s, "element.pick", json!({"x": 50, "y": 40}));
    assert_eq!(kind(&mut s, 50.0, 40.0), "object");
    // An object is an object anywhere on it.
    run(&mut s, "draw.rect", json!({"x": 300, "y": 0, "w": 20, "h": 20, "object": true}));
    assert_eq!(kind(&mut s, 300.0, 0.0), "object");
}

#[test]
fn dragging_a_fill_moves_only_the_fill() {
    let mut s = rect_session();
    // The click-and-drag on a fill: it is selected alone, moved and dropped.
    run(&mut s, "element.pick", json!({"x": 50, "y": 40}));
    assert_eq!(s.selection.len(), 1);
    run(&mut s, "element.translate", json!({"dx": 0, "dy": 200}));
    run(&mut s, "draw.drop", json!({}));
    // The outline stayed where it was, now with nothing inside.
    let strokes: Vec<Vec2> = raw(&s).iter().filter(|e| !is_fill(e)).flat_map(|e| anchors(e)).collect();
    assert!(strokes.iter().all(|p| p.y <= 80.0 + 1e-3), "{strokes:?}");
    let fills: Vec<Vec2> = raw(&s).iter().filter(|e| is_fill(e)).flat_map(|e| anchors(e)).collect();
    assert!(fills.iter().all(|p| p.y >= 200.0 - 1e-3));
}

#[test]
fn raw_edits_never_panic_on_odd_input() {
    let mut s = rect_session();
    for (id, p) in [
        ("path.bend_segment", json!({"x": 50, "y": 0, "dx": 1e7, "dy": -1e7})),
        ("path.bend_segment", json!({"x": 50, "y": 0, "dx": 1, "dy": 1, "id": 999_999})),
        ("path.bend_segment", json!({"x": 50, "y": 0, "dx": 1, "dy": 1, "cmd": 99, "t": 0.5, "id": raw(&s)[1].id.0})),
        ("raw.move_point", json!({"x": 0, "y": 0, "to_x": 1e7, "to_y": 1e7})),
        ("raw.move_point", json!({"x": 0, "y": 0, "to_x": 1, "to_y": 1, "points": [[1, 2], [3]]})),
        ("raw.move_point", json!({"x": 0, "y": 0, "to_x": 1, "to_y": 1, "points": []})),
        ("raw.move_point", json!({"x": 0, "y": 0, "to_x": 1, "to_y": 1, "points": [[u64::MAX, u64::MAX]]})),
        ("raw.select_rect", json!({"x0": 0, "y0": 0, "x1": 0, "y1": 0})),
        ("raw.select_rect", json!({"x0": -1e7, "y0": -1e7, "x1": 1e7, "y1": 1e7})),
        ("raw.merge", json!({"ids": [u64::MAX]})),
    ] {
        let _ = s.execute(id, &p);
    }
    let _ = anicraft_render::render_frame(&s.doc, 0, 0.1);
    let _ = Affine::IDENTITY;
}

/// Per-frame cost of the Selection tool's drags on a busy raw drawing (run by hand:
/// `cargo test -p anicraft-engine raw_drag_timings -- --ignored --nocapture`).
#[test]
#[ignore]
fn raw_drag_timings() {
    let mut s = Session::new(anicraft_doc::Document::default());
    run(&mut s, "draw.object_drawing", json!({"on": false}));
    // 12 × 12 outlined squares, and lines across every row: 144 fills, ~1,000 stroke pieces.
    for i in 0..12 {
        for j in 0..12 {
            run(&mut s, "draw.rect", json!({"x": i * 50, "y": j * 50, "w": 40, "h": 40, "fill": "#ff0000", "stroke": "#000000"}));
        }
        run(&mut s, "draw.line", json!({"x0": -10, "y0": i * 50 + 20, "x1": 610, "y1": i * 50 + 20, "stroke": "#0000ff"}));
    }
    run(&mut s, "edit.deselect", json!({}));
    let n = s.visible_elements().len();
    let time = |s: &mut Session, label: &str, f: &dyn Fn(&mut Session)| {
        let t = std::time::Instant::now();
        let reps = 20;
        for _ in 0..reps {
            f(s);
        }
        eprintln!("{label}: {:.2} ms", t.elapsed().as_secs_f64() * 1000.0 / f64::from(reps));
    };
    eprintln!("{n} raw shapes");
    time(&mut s, "raw_grab (hover)", &|s| {
        let _ = s.raw_grab(Vec2::new(300.0, 300.0), 3.0, 6.0);
    });
    s.begin_gesture("Bend");
    time(&mut s, "bend frame (no merge)", &|s| {
        let _ = s.execute("path.bend_segment", &json!({"x": 320, "y": 300, "dx": 0, "dy": -0.5, "merge": false}));
    });
    s.end_gesture();
    let r = run(&mut s, "raw.move_point", json!({"x": 340, "y": 340, "to_x": 341, "to_y": 341, "snap": 10, "merge": false}));
    let points = r["points"].clone();
    time(&mut s, "point frame with snapping", &|s| {
        let _ = s.execute("raw.move_point", &json!({"x": 341, "y": 341, "to_x": 341, "to_y": 341, "snap": 10, "points": points, "merge": false}));
    });
    // Once: it replaces the shapes it merges.
    let ids: Vec<Value> = points.as_array().into_iter().flatten().map(|p| p[0].clone()).collect();
    let t = std::time::Instant::now();
    run(&mut s, "raw.merge", json!({"ids": ids}));
    eprintln!("merge at release: {:.2} ms", t.elapsed().as_secs_f64() * 1000.0);
    time(&mut s, "marquee cut and heal", &|s| {
        let _ = s.execute("raw.select_rect", &json!({"x0": 125, "y0": 125, "x1": 425, "y1": 425}));
        let _ = s.execute("edit.deselect", &json!({}));
    });
    run(&mut s, "raw.select_rect", json!({"x0": 0, "y0": 0, "x1": 45, "y1": 45}));
    time(&mut s, "selection move frame with snapping", &|s| {
        let _ = s.execute("element.translate", &json!({"dx": 0.5, "dy": 0, "snap": 10, "from": [10, 10]}));
    });
}
