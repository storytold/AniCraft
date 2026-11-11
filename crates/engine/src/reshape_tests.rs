//! Reshaping raw shapes (ADR 0031): moving shared points, bending segments, snapping.

use crate::Session;
use anicraft_doc::{Element, ElementKind, Shape};
use anicraft_geom::Vec2;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn blank() -> Session {
    let mut s = Session::new(anicraft_doc::Document::default());
    run(&mut s, "draw.object_drawing", json!({"on": false}));
    s
}

fn shapes(s: &Session) -> Vec<(Element, Shape)> {
    s.visible_elements()
        .iter()
        .filter_map(|e| match &e.kind {
            ElementKind::Shape(sh) => Some(((**e).clone(), sh.clone())),
            _ => None,
        })
        .collect()
}

fn count(s: &Session) -> (usize, usize) {
    let sh = shapes(s);
    (sh.iter().filter(|(_, x)| x.fill.is_some()).count(), sh.iter().filter(|(_, x)| x.stroke.is_some()).count())
}

/// Whether some anchor of the shape lies at `p` (stage space).
fn has_anchor(e: &Element, sh: &Shape, p: Vec2) -> bool {
    sh.path.anchors().iter().any(|(_, q)| e.transform.apply(*q).distance(p) < 0.01)
}

/// How near the shape's outline comes to `p` (stage space, to within the flattening tolerance).
fn gap(e: &Element, sh: &Shape, p: Vec2) -> f32 {
    sh.path.distance_to_edge(p, &e.transform)
}

#[test]
fn grabbing_finds_a_corner_with_everything_on_it_or_a_side_with_the_fill_edge_under_it() {
    let mut s = blank();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 80, "fill": "#ff0000", "stroke": "#000000"}));
    let g = run(&mut s, "draw.grab", json!({"x": 1, "y": 1}));
    assert_eq!(g["what"], "point");
    assert_eq!((g["x"].as_f64(), g["y"].as_f64()), (Some(0.0), Some(0.0)));
    // The fill's corner (stored twice on a closed outline) and the ends of two sides.
    let ids: std::collections::HashSet<u64> = g["parts"].as_array().into_iter().flatten().filter_map(|p| p[0].as_u64()).collect();
    assert_eq!(ids.len(), 3);
    let g = run(&mut s, "draw.grab", json!({"x": 50, "y": 1}));
    assert_eq!(g["what"], "curve");
    assert_eq!(g["parts"].as_array().map(Vec::len), Some(2), "the top side and the fill's top edge");
    assert_eq!(run(&mut s, "draw.grab", json!({"x": 50, "y": 40}))["what"], Value::Null, "inside the fill: a move, not a reshape");
    // Objects are not reshaped this way.
    run(&mut s, "draw.rect", json!({"x": 300, "y": 0, "w": 50, "h": 50, "fill": "#00ff00", "stroke": null, "object": true}));
    assert_eq!(run(&mut s, "draw.grab", json!({"x": 300, "y": 0}))["what"], Value::Null);
}

#[test]
fn dragging_a_corner_moves_it_on_the_fill_and_both_sides() {
    let mut s = blank();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 80, "fill": "#ff0000", "stroke": "#000000"}));
    let r = run(&mut s, "draw.reshape", json!({"x": 0, "y": 0, "to_x": -20, "to_y": -10}));
    assert_eq!(r["what"], "point");
    let corner = Vec2::new(-20.0, -10.0);
    let on: Vec<_> = shapes(&s).into_iter().filter(|(e, sh)| has_anchor(e, sh, corner)).collect();
    assert_eq!(on.len(), 3, "the fill and the two sides that met there");
    assert_eq!(count(&s), (1, 4), "still one fill and four sides after the merge");
    run(&mut s, "edit.undo", json!({}));
    assert!(shapes(&s).iter().any(|(e, sh)| has_anchor(e, sh, Vec2::ZERO)), "one undo step");
}

#[test]
fn dragging_a_side_bends_it_and_the_fill_follows() {
    let mut s = blank();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 80, "fill": "#ff0000", "stroke": "#000000"}));
    run(&mut s, "element.select", json!({"ids": []}));
    let r = run(&mut s, "draw.reshape", json!({"x": 50, "y": 0, "to_x": 50, "to_y": -30, "drop": false}));
    assert_eq!(r["what"], "curve");
    let top = Vec2::new(50.0, -30.0);
    let through: Vec<_> = shapes(&s).into_iter().filter(|(e, sh)| gap(e, sh, top) < 0.5).collect();
    assert_eq!(through.len(), 2, "the side and the fill's edge both pass where the side was dragged");
    // The ends stay where they were.
    assert!(shapes(&s).iter().filter(|(e, sh)| has_anchor(e, sh, Vec2::ZERO)).count() == 3);
    run(&mut s, "draw.drop", json!({"ids": through.iter().map(|(e, _)| e.id.0).collect::<Vec<_>>()}));
    assert_eq!(count(&s), (1, 4));
    assert!(s.selection.is_empty(), "nothing was selected, so nothing is");
}

#[test]
fn a_bent_line_cuts_what_it_now_crosses() {
    let mut s = blank();
    run(&mut s, "draw.line", json!({"x0": 0, "y0": 0, "x1": 100, "y1": 0, "stroke": "#000000"}));
    run(&mut s, "draw.line", json!({"x0": 0, "y0": 40, "x1": 100, "y1": 40, "stroke": "#000000"}));
    assert_eq!(count(&s), (0, 2));
    // Pull the top line's middle well below the other one: it crosses it twice.
    run(&mut s, "draw.reshape", json!({"x": 50, "y": 0, "to_x": 50, "to_y": 80}));
    assert_eq!(count(&s), (0, 6), "each line is cut into three");
}

#[test]
fn a_dragged_end_snaps_onto_a_nearby_point_or_line_and_joins_it() {
    let mut s = blank();
    run(&mut s, "draw.line", json!({"x0": 0, "y0": 0, "x1": 100, "y1": 0, "stroke": "#000000"}));
    run(&mut s, "draw.line", json!({"x0": 200, "y0": 50, "x1": 200, "y1": 150, "stroke": "#000000"}));
    // Dropped 6 units from the other line's end, within the snap distance: it lands on it.
    let r = run(&mut s, "draw.reshape", json!({"x": 100, "y": 0, "to_x": 196, "to_y": 46, "snap": 10}));
    assert_eq!(r["snapped"], true);
    assert_eq!((r["x"].as_f64(), r["y"].as_f64()), (Some(200.0), Some(50.0)));
    let sel = run(&mut s, "element.select_connected", json!({"x": 200, "y": 100}));
    assert_eq!(sel["selected"].as_array().map(Vec::len), Some(2), "the two lines are joined end to end");
    // Onto a line, away from its points: the end lands on the line.
    let mut s = blank();
    run(&mut s, "draw.line", json!({"x0": 0, "y0": 0, "x1": 0, "y1": 100, "stroke": "#000000"}));
    run(&mut s, "draw.line", json!({"x0": 20, "y0": 50, "x1": 80, "y1": 50, "stroke": "#000000"}));
    let r = run(&mut s, "draw.reshape", json!({"x": 20, "y": 50, "to_x": 5, "to_y": 52, "snap": 10}));
    assert_eq!(r["snapped"], true);
    assert!(r["x"].as_f64().is_some_and(|x| x.abs() < 1e-3) && r["y"].as_f64().is_some_and(|y| (y - 52.0).abs() < 0.1));
    assert_eq!(count(&s), (0, 3), "the line it landed on is cut there");
    // Without snapping it goes exactly where it was put.
    let r = run(&mut s, "draw.reshape", json!({"x": 80, "y": 50, "to_x": 85, "to_y": 55}));
    assert_eq!(r["snapped"], false);
}

#[test]
fn a_live_drag_keeps_its_parts_and_bad_parts_are_refused() {
    let mut s = blank();
    run(&mut s, "draw.line", json!({"x0": 0, "y0": 0, "x1": 100, "y1": 0, "stroke": "#000000"}));
    run(&mut s, "draw.line", json!({"x0": 0, "y0": 20, "x1": 100, "y1": 20, "stroke": "#000000"}));
    let g = run(&mut s, "draw.grab", json!({"x": 100, "y": 0}));
    // Step by step through the other line's end and past it: only the grabbed end moves.
    s.begin_gesture("Reshape");
    let mut at = (100.0, 0.0);
    for to in [(100.0, 10.0), (100.0, 20.0), (100.0, 30.0)] {
        let r = run(&mut s, "draw.reshape", json!({"x": at.0, "y": at.1, "to_x": to.0, "to_y": to.1, "parts": g["parts"], "snap": 3, "drop": false}));
        at = (r["x"].as_f64().unwrap_or(0.0), r["y"].as_f64().unwrap_or(0.0));
    }
    s.end_gesture();
    assert_eq!(at, (100.0, 30.0));
    assert!(shapes(&s).iter().any(|(e, sh)| has_anchor(e, sh, Vec2::new(100.0, 20.0))), "the other line stayed");
    for parts in [json!([[1]]), json!([[1, 0, 2.0]]), json!("x"), json!([[999_999, 0]])] {
        assert!(s.execute("draw.reshape", &json!({"x": 0, "y": 0, "to_x": 1, "to_y": 1, "parts": parts})).is_err(), "{parts}");
    }
    assert_eq!(run(&mut s, "draw.reshape", json!({"x": 500, "y": 500, "to_x": 1, "to_y": 1}))["what"], Value::Null);
}
