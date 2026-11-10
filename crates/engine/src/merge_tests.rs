//! Merge drawing (ADR 0031) through the commands.

use crate::Session;
use anicraft_doc::{ElementKind, Paint, Rgba, Shape};
use serde_json::json;

fn run(s: &mut Session, id: &str, p: serde_json::Value) -> serde_json::Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// A blank document (no sample artwork), drawing raw shapes.
fn blank() -> Session {
    let mut s = Session::new(anicraft_doc::Document::default());
    run(&mut s, "draw.object_drawing", json!({"on": false}));
    s
}

fn shapes(s: &Session) -> Vec<Shape> {
    s.visible_elements()
        .iter()
        .filter_map(|e| match &e.kind {
            ElementKind::Shape(sh) => Some(sh.clone()),
            _ => None,
        })
        .collect()
}

fn count(s: &Session) -> (usize, usize) {
    let sh = shapes(s);
    (sh.iter().filter(|x| x.fill.is_some()).count(), sh.iter().filter(|x| x.stroke.is_some()).count())
}

#[test]
fn a_raw_rectangle_is_a_fill_and_its_sides_selected_together() {
    let mut s = blank();
    let r = run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 80, "fill": "#ff0000", "stroke": "#000000"}));
    assert_eq!(r["merged"], true);
    assert_eq!(count(&s), (1, 4));
    assert!(shapes(&s).iter().all(|x| x.merge));
    assert_eq!(s.selection.len(), 5, "what was drawn is selected");
    // Fills sit under the strokes.
    let fills_first = shapes(&s).iter().position(|x| x.stroke.is_some()).unwrap_or(0);
    assert!(shapes(&s)[..fills_first].iter().all(|x| x.fill.is_some()));
}

#[test]
fn a_shape_on_top_cuts_the_one_below_and_undo_takes_it_back() {
    let mut s = blank();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 100, "fill": "#ff0000", "stroke": null}));
    run(&mut s, "draw.rect", json!({"x": 50, "y": 50, "w": 100, "h": 100, "fill": "#0000ff", "stroke": null}));
    assert_eq!(count(&s), (2, 0));
    let red = shapes(&s).into_iter().find(|x| x.fill == Some(Paint::Solid(Rgba::hex(0xff0000)))).map(|x| x.path.bounds());
    assert!(red.is_some_and(|b| (b.width() - 100.0).abs() < 0.01), "the red keeps its outline, less the corner");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(count(&s), (1, 0));
    // The same colour joins it.
    run(&mut s, "draw.rect", json!({"x": 50, "y": 50, "w": 100, "h": 100, "fill": "#ff0000", "stroke": null}));
    assert_eq!(count(&s), (1, 0));
    assert_eq!(s.selection.len(), 1);
}

#[test]
fn a_line_across_a_raw_fill_splits_it() {
    let mut s = blank();
    run(&mut s, "draw.ellipse", json!({"x": 0, "y": 0, "w": 100, "h": 100, "fill": "#00ff00", "stroke": null}));
    run(&mut s, "draw.line", json!({"x0": -10, "y0": 50, "x1": 110, "y1": 50, "stroke": "#000000"}));
    assert_eq!(count(&s), (2, 1));
    assert_eq!(s.selection.len(), 1, "the line");
}

#[test]
fn objects_keep_to_themselves() {
    let mut s = blank();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 100, "fill": "#ff0000", "stroke": null}));
    let r = run(&mut s, "draw.rect", json!({"x": 50, "y": 50, "w": 100, "h": 100, "fill": "#0000ff", "stroke": null, "object": true}));
    assert!(r["merged"].is_null());
    assert_eq!(count(&s), (2, 0));
    // Raw shapes drawn over an object do not cut it either.
    run(&mut s, "draw.line", json!({"x0": -10, "y0": 60, "x1": 200, "y1": 60, "stroke": "#000000"}));
    let object = shapes(&s).into_iter().find(|x| !x.merge).map(|x| x.path.bounds());
    assert!(object.is_some_and(|b| (b.width() - 100.0).abs() < 0.01));
    // A new session (scripts) draws objects unless asked.
    let mut fresh = Session::new(anicraft_doc::Document::default());
    run(&mut fresh, "draw.rect", json!({"x": 0, "y": 0, "w": 10, "h": 10, "stroke": "#000000"}));
    assert_eq!(fresh.visible_elements().len(), 1);
    assert_eq!(run(&mut fresh, "draw.object_drawing", json!({}))["object_drawing"], false);
}

#[test]
fn raw_shapes_survive_save_and_load() {
    let mut s = blank();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 40, "h": 40, "fill": "#ff0000", "stroke": "#000000"}));
    let back = crate::format::from_json(&crate::format::to_json(&s.doc).unwrap_or_default()).unwrap_or_default();
    let s2 = Session::new(back);
    assert_eq!(count(&s2), (1, 4));
    assert!(shapes(&s2).iter().all(|x| x.merge));
}

#[test]
fn dropping_a_moved_fill_cuts_what_it_lands_on() {
    let mut s = blank();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 100, "fill": "#ff0000", "stroke": null}));
    run(&mut s, "draw.rect", json!({"x": 200, "y": 0, "w": 50, "h": 50, "fill": "#0000ff", "stroke": null}));
    assert_eq!(count(&s), (2, 0));
    // Move the blue square into the red one and let go.
    run(&mut s, "element.translate", json!({"dx": -175, "dy": 25}));
    let r = run(&mut s, "draw.drop", json!({}));
    assert_eq!(r["dropped"], 1);
    let red = shapes(&s).into_iter().find(|x| x.fill == Some(Paint::Solid(Rgba::hex(0xff0000)))).map(|x| x.path.cmds.iter().filter(|c| matches!(c, anicraft_geom::PathCmd::MoveTo(_))).count());
    assert_eq!(red, Some(2), "the red square has a hole where the blue one landed");
    assert_eq!(s.selection.len(), 1, "the dropped square stays selected");
}

#[test]
fn double_click_selects_connected_strokes_or_a_fill_with_its_outline() {
    let mut s = blank();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 100, "fill": "#ff0000", "stroke": "#000000"}));
    run(&mut s, "draw.line", json!({"x0": 300, "y0": 0, "x1": 400, "y1": 0, "stroke": "#000000"}));
    // One side of the outline: the other three come with it, not the fill or the far line.
    let r = run(&mut s, "element.select_connected", json!({"x": 50, "y": 0}));
    assert_eq!(r["selected"].as_array().map(Vec::len), Some(4));
    // The fill: it and its four sides.
    let r = run(&mut s, "element.select_connected", json!({"x": 50, "y": 50}));
    assert_eq!(r["selected"].as_array().map(Vec::len), Some(5));
    let r = run(&mut s, "element.select_connected", json!({"x": 350, "y": 0}));
    assert_eq!(r["selected"].as_array().map(Vec::len), Some(1));
}
