//! Drawing parity leftovers (ADR 0080): PolyStar options, Paint Selection and Paint Inside,
//! drawing objects edited in place, Envelope.

use crate::Session;
use crate::cmds_erase::measure_els;
use anicraft_doc::{ElementKind, Paint, Rgba, Shape};
use anicraft_geom::{Affine, Vec2};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn blank(object: bool) -> Session {
    let mut s = Session::new(anicraft_doc::Document::default());
    run(&mut s, "draw.object_drawing", json!({"on": object}));
    s
}

/// The keyframe's shapes, bottom to top, with their transforms.
fn shapes(s: &Session) -> Vec<(Shape, Affine)> {
    s.visible_elements()
        .iter()
        .filter_map(|e| match &e.kind {
            ElementKind::Shape(sh) => Some((sh.clone(), e.transform)),
            _ => None,
        })
        .collect()
}

/// Total fill area on the active layer, raw shapes and drawing objects (even-odd loops summed
/// by sign; exact for polygons, within the flattening for curves).
fn fill_area(s: &Session) -> f64 {
    let raw = measure_els(&s.visible_elements()).0;
    let objects: f64 = shapes(s).iter().filter(|(sh, _)| sh.fill.is_some() && !sh.merge).map(|(sh, m)| area_of(&sh.path, m)).sum();
    raw + objects
}

fn area_of(path: &anicraft_geom::Path, m: &Affine) -> f64 {
    path.flatten(m, 0.005)
        .iter()
        .map(|pl| {
            let n = pl.points.len();
            (0..n).map(|i| {
                let (a, b) = (pl.points[i], pl.points[(i + 1) % n]);
                f64::from(a.x) * f64::from(b.y) - f64::from(b.x) * f64::from(a.y)
            }).sum::<f64>() * 0.5
        })
        .sum::<f64>()
        .abs()
}

/// What colour covers stage point `p` (the topmost shape containing it).
fn top_at(s: &Session, p: Vec2) -> Option<Rgba> {
    shapes(s).iter().rev().find(|(sh, m)| sh.fill.is_some() && sh.path.contains(p, m, sh.fill_rule)).and_then(|(sh, _)| match sh.fill {
        Some(Paint::Solid(c)) => Some(c),
        _ => None,
    })
}

// ----- PolyStar ----------------------------------------------------------------------------------

/// A polygon of n sides has area n/2 · r² · sin(2π/n); a star at point size 1 covers the same.
#[test]
fn polystar_polygon_and_star_areas() {
    for object in [false, true] {
        for n in [3u32, 6, 32] {
            let mut s = blank(object);
            run(&mut s, "draw.star", json!({"cx": 100, "cy": 100, "outer": 50, "shape": "polygon", "sides": n, "stroke": null, "fill": "#336699"}));
            let want = f64::from(n) / 2.0 * 2500.0 * (std::f64::consts::TAU / f64::from(n)).sin();
            assert!((fill_area(&s) - want).abs() < want * 1e-4, "polygon {n} object {object}: {} vs {want}", fill_area(&s));
            let mut t = blank(object);
            let v = run(&mut t, "draw.star", json!({"cx": 100, "cy": 100, "outer": 50, "sides": n, "point_size": 1.0, "stroke": null, "fill": "#336699"}));
            assert_eq!(v["kind"], "star");
            assert!((fill_area(&t) - want).abs() < want * 1e-4, "star {n} at size 1");
        }
    }
}

/// A star's area is n · r · r_in · sin(π/n), r_in = r · size · cos(π/n); the old `inner` and
/// `points` params still work.
#[test]
fn polystar_star_follows_point_size() {
    let mut s = blank(true);
    run(&mut s, "draw.star", json!({"cx": 0, "cy": 0, "outer": 40, "sides": 5, "point_size": 0.3, "stroke": null, "fill": "#ff0000"}));
    let half = std::f64::consts::PI / 5.0;
    let inner = 40.0 * 0.3 * half.cos();
    let want = 5.0 * 40.0 * inner * half.sin();
    assert!((fill_area(&s) - want).abs() < want * 1e-4);
    let mut t = blank(true);
    run(&mut t, "draw.star", json!({"cx": 0, "cy": 0, "outer": 40, "inner": 10, "points": 6, "stroke": null, "fill": "#ff0000"}));
    let want = 6.0 * 40.0 * 10.0 * (std::f64::consts::PI / 6.0).sin();
    assert!((fill_area(&t) - want).abs() < want * 1e-4);
}

#[test]
fn polystar_refuses_bad_options() {
    let mut s = blank(false);
    for p in [
        json!({"cx": 0, "cy": 0, "outer": 40, "shape": "hexagon"}),
        json!({"cx": 0, "cy": 0, "outer": 40, "sides": 2}),
        json!({"cx": 0, "cy": 0, "outer": 40, "sides": 65}),
        json!({"cx": 0, "cy": 0, "outer": 40, "point_size": 1.5}),
        json!({"cx": 0, "cy": 0, "outer": 40, "point_size": -0.1}),
    ] {
        assert!(s.execute("draw.star", &p).is_err(), "{p}");
    }
    assert!(s.visible_elements().is_empty());
}

/// Merge drawing: a polygon drawn over a raw fill of another colour cuts into it, as the other
/// shape tools do (ADR 0031).
#[test]
fn polystar_merges_like_the_other_shape_tools() {
    let mut s = blank(false);
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 200, "h": 200, "stroke": null, "fill": "#0000ff"}));
    run(&mut s, "draw.star", json!({"cx": 100, "cy": 100, "outer": 50, "shape": "polygon", "sides": 4, "stroke": null, "fill": "#ff0000"}));
    // The square's area stays: the polygon replaced what it covers.
    assert!((fill_area(&s) - 40_000.0).abs() < 1.0, "{}", fill_area(&s));
    assert_eq!(top_at(&s, Vec2::new(100.0, 100.0)), Some(Rgba::hex(0xff0000)));
    assert_eq!(top_at(&s, Vec2::new(5.0, 5.0)), Some(Rgba::hex(0x0000ff)));
    assert!(shapes(&s).iter().all(|(sh, _)| sh.merge));
}

// ----- Paint Selection and Paint Inside ----------------------------------------------------------

/// Fill area of one colour on the active layer.
fn area_in(s: &Session, color: u32) -> f64 {
    shapes(s).iter().filter(|(sh, _)| matches!(sh.fill, Some(Paint::Solid(c)) if c == Rgba::hex(color))).map(|(sh, m)| area_of(&sh.path, m)).sum()
}

/// A blue square at 0..100 and a green one at 200..300 (raw fills, no lines).
fn two_squares() -> Session {
    let mut s = blank(false);
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 100, "stroke": null, "fill": "#0000ff"}));
    run(&mut s, "draw.rect", json!({"x": 200, "y": 0, "w": 100, "h": 100, "stroke": null, "fill": "#00ff00"}));
    run(&mut s, "edit.deselect", json!({}));
    s
}

fn brush(s: &mut Session, mode: &str, from: [f32; 2], to: [f32; 2]) -> Value {
    let pts: Vec<[f32; 2]> = (0..=40).map(|i| {
        let t = i as f32 / 40.0;
        [from[0] + (to[0] - from[0]) * t, from[1] + (to[1] - from[1]) * t]
    }).collect();
    run(s, "draw.brush", json!({"points": pts, "size": 20, "smoothing": 0, "mode": mode, "color": "#ff0000"}))
}

fn blue_id(s: &Session) -> u64 {
    s.visible_elements().iter().find(|e| matches!(&e.kind, ElementKind::Shape(sh) if sh.fill == Some(Paint::Solid(Rgba::hex(0x0000ff))))).map(|e| e.id.0).expect("the blue square")
}

/// Paint Selection: a stroke across both squares paints only the selected one, exactly over
/// the 100 × 20 band inside it.
#[test]
fn paint_selection_clips_to_the_selected_fills() {
    let mut s = two_squares();
    let blue = blue_id(&s);
    run(&mut s, "element.select", json!({"ids": [blue]}));
    let v = brush(&mut s, "selection", [-60.0, 50.0], [360.0, 50.0]);
    assert_eq!(v["merged"], true, "{v}");
    let red = area_in(&s, 0xff0000);
    assert!((red - 2000.0).abs() < 2.0, "the band inside the blue square: {red}");
    assert!((area_in(&s, 0x0000ff) - 8000.0).abs() < 2.0, "the rest of the blue square: {}", area_in(&s, 0x0000ff));
    assert!((area_in(&s, 0x00ff00) - 10_000.0).abs() < 0.5, "the green square is not selected");
    assert_eq!(top_at(&s, Vec2::new(250.0, 50.0)), Some(Rgba::hex(0x00ff00)));
    assert_eq!(top_at(&s, Vec2::new(150.0, 50.0)), None, "nothing between them");
    assert_eq!(top_at(&s, Vec2::new(50.0, 50.0)), Some(Rgba::hex(0xff0000)));
    // The selection stays on the blue square's pieces and the paint, for the next stroke.
    let sel = s.selection.on(s.layer);
    let els = s.visible_elements();
    assert!(sel.len() >= 2, "{sel:?}");
    for id in &sel {
        let e = els.iter().find(|e| e.id == *id).expect("selected element");
        let ElementKind::Shape(sh) = &e.kind else { panic!("a shape") };
        let b = sh.path.bounds().transformed(&e.transform);
        assert!(b.min.x > -0.5 && b.max.x < 100.5, "inside the blue square: {b:?}");
    }
    let v = brush(&mut s, "selection", [-60.0, 20.0], [360.0, 20.0]);
    assert!(v["elements"].as_array().is_some_and(|a| !a.is_empty()), "a second stroke paints too");
    assert!((area_in(&s, 0xff0000) - 4000.0).abs() < 4.0, "{}", area_in(&s, 0xff0000));
}

/// With nothing selected (or a stroke that misses the selection), nothing changes and there is
/// no undo step.
#[test]
fn paint_selection_without_a_selection_changes_nothing() {
    let mut s = two_squares();
    let steps = s.history_len();
    let doc = s.doc.clone();
    let v = brush(&mut s, "selection", [-60.0, 50.0], [360.0, 50.0]);
    assert_eq!(v["painted"], false);
    assert_eq!(s.history_len(), steps);
    assert!(s.doc == doc);
    let blue = blue_id(&s);
    run(&mut s, "element.select", json!({"ids": [blue]}));
    let v = brush(&mut s, "selection", [150.0, 300.0], [160.0, 400.0]);
    assert_eq!(v["painted"], false, "{v}");
    assert_eq!(s.history_len(), steps);
}

/// Paint Inside: started in the blue square, the stroke stays inside it (a half-round start
/// cap and a 50 × 20 band) and never reaches the green one.
#[test]
fn paint_inside_clips_to_the_fill_it_starts_on() {
    let mut s = two_squares();
    brush(&mut s, "inside", [50.0, 50.0], [250.0, 50.0]);
    let want = 50.0 * 20.0 + std::f64::consts::PI * 100.0 / 2.0;
    let red = area_in(&s, 0xff0000);
    assert!((red - want).abs() < want * 0.01, "{red} vs {want}");
    assert!((area_in(&s, 0x00ff00) - 10_000.0).abs() < 0.5);
    assert!((area_in(&s, 0x0000ff) + red - 10_000.0).abs() < 2.0, "the paint replaced its part of the blue square");
}

/// Paint Inside started on empty stage inside a closed outline paints only that empty area,
/// under the line; started on open stage, only empty areas (as Paint Behind).
#[test]
fn paint_inside_empty_areas() {
    let mut s = blank(false);
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 100, "fill": null, "stroke": "#000000", "stroke_width": 2}));
    let lines = measure_els(&s.visible_elements()).1;
    brush(&mut s, "inside", [50.0, 50.0], [250.0, 50.0]);
    let want = 50.0 * 20.0 + std::f64::consts::PI * 100.0 / 2.0;
    let red = area_in(&s, 0xff0000);
    assert!((red - want).abs() < want * 0.01, "{red} vs {want}");
    assert_eq!(top_at(&s, Vec2::new(200.0, 50.0)), None, "nothing outside the outline");
    assert!((measure_els(&s.visible_elements()).1 - lines).abs() < 0.5, "the outline is whole");

    let mut t = two_squares();
    brush(&mut t, "inside", [-100.0, 50.0], [50.0, 50.0]);
    assert!((area_in(&t, 0x0000ff) - 10_000.0).abs() < 0.5, "the blue square is untouched");
    assert!(area_in(&t, 0xff0000) > 1000.0, "the empty stage took the paint");
}

/// A line across the selected fill stays: clipped paint goes under lines.
#[test]
fn paint_selection_leaves_lines() {
    let mut s = two_squares();
    run(&mut s, "draw.line", json!({"x0": 50, "y0": -20, "x1": 50, "y1": 120, "stroke": "#000000", "stroke_width": 2}));
    let lines = measure_els(&s.visible_elements()).1;
    run(&mut s, "edit.select_all", json!({}));
    brush(&mut s, "selection", [-60.0, 50.0], [360.0, 50.0]);
    assert!((measure_els(&s.visible_elements()).1 - lines).abs() < 0.5, "the line is whole");
    // Both squares were selected: the band in each.
    assert!((area_in(&s, 0xff0000) - 4000.0).abs() < 4.0, "{}", area_in(&s, 0xff0000));
}

/// As drawing objects, the clipped paint is an object of exactly the clipped outline.
#[test]
fn paint_inside_as_a_drawing_object() {
    let mut s = blank(true);
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 100, "stroke": null, "fill": "#0000ff"}));
    brush(&mut s, "inside", [50.0, 50.0], [250.0, 50.0]);
    let want = 50.0 * 20.0 + std::f64::consts::PI * 100.0 / 2.0;
    assert!((area_in(&s, 0xff0000) - want).abs() < want * 0.01);
    assert!((area_in(&s, 0x0000ff) - 10_000.0).abs() < 0.5, "objects don't cut each other");
}

// ----- drawing objects edited in place -----------------------------------------------------------

/// A drawing object (an orange rectangle at 100..300 × 100..200) beside a raw blue square on the
/// same layer; returns the object's id.
fn object_and_raw() -> (Session, u64) {
    let mut s = blank(false);
    run(&mut s, "draw.rect", json!({"x": 400, "y": 100, "w": 100, "h": 100, "stroke": null, "fill": "#0000ff"}));
    let v = run(&mut s, "draw.rect", json!({"x": 100, "y": 100, "w": 200, "h": 100, "stroke": null, "fill": "#ff8800", "object": true}));
    let id = v["element"].as_u64().expect("the object");
    run(&mut s, "edit.deselect", json!({}));
    (s, id)
}

/// Double-clicking a drawing object opens it in place; opening and closing it without an edit
/// changes nothing: the same document and state, no undo step, the object selected.
#[test]
fn drawing_object_opens_and_closes_unchanged() {
    let (mut s, id) = object_and_raw();
    let (doc, state, steps) = (s.doc.clone(), s.state, s.history_len());
    let v = run(&mut s, "symbol.edit", json!({"id": id}));
    assert_eq!(v["editing"][0]["drawing"], id, "{v}");
    assert!(s.editing_drawing() && s.editing_group());
    // Inside, its one raw shape, where it showed.
    let inside = s.visible_elements();
    assert_eq!(inside.len(), 1);
    assert!(matches!(&inside[0].kind, ElementKind::Shape(sh) if sh.merge));
    run(&mut s, "symbol.exit", json!({}));
    assert!(s.doc == doc, "the document is as it was");
    assert_eq!(s.state, state, "no unsaved change");
    assert_eq!(s.history_len(), steps);
    assert_eq!(s.selection.id_list(), vec![id]);
    assert!(s.visible_elements().iter().any(|e| e.id.0 == id && matches!(&e.kind, ElementKind::Shape(sh) if !sh.merge)));
}

/// Inside, a line drawn across merges only with the object's own shape: it splits the fill, the
/// raw square outside is untouched, and closing it leaves a group of the pieces (several
/// shapes stay one object, as Combine Objects' Union makes them).
#[test]
fn drawing_object_merges_only_inside() {
    let (mut s, id) = object_and_raw();
    run(&mut s, "symbol.edit", json!({"id": id}));
    // Positions inside are in the object's own space (its centre at the origin).
    run(&mut s, "draw.line", json!({"x0": 0, "y0": -100, "x1": 0, "y1": 100, "stroke": "#000000", "stroke_width": 2, "object": false}));
    let inside = s.visible_elements();
    let fills = inside.iter().filter(|e| matches!(&e.kind, ElementKind::Shape(sh) if sh.fill.is_some())).count();
    assert_eq!(fills, 2, "the line cut the fill in two");
    run(&mut s, "symbol.exit", json!({}));
    let els = s.visible_elements();
    let obj = els.iter().find(|e| e.id.0 == id).expect("the object stays, with its id");
    assert!(matches!(obj.kind, ElementKind::Group { .. }), "several pieces: a group");
    assert!((area_in(&s, 0x0000ff) - 10_000.0).abs() < 0.5, "the raw square outside is whole");
    assert_eq!(els.len(), 2);
}

/// An edit that leaves one shape makes it one drawing object again, moved as edited; undo goes
/// back inside, and closing then gives the object as it was.
#[test]
fn drawing_object_edit_keeps_one_object() {
    let (mut s, id) = object_and_raw();
    let before = s.element_bounds(anicraft_doc::ElementId(id));
    run(&mut s, "symbol.edit", json!({"id": id}));
    run(&mut s, "edit.select_all", json!({}));
    run(&mut s, "element.translate", json!({"dx": 30, "dy": 10}));
    run(&mut s, "symbol.exit", json!({}));
    let els = s.visible_elements();
    let obj = els.iter().find(|e| e.id.0 == id).expect("the object");
    assert!(matches!(&obj.kind, ElementKind::Shape(sh) if !sh.merge), "one drawing object again");
    let after = s.element_bounds(anicraft_doc::ElementId(id));
    assert!((after.min.x - before.min.x - 30.0).abs() < 1e-3 && (after.min.y - before.min.y - 10.0).abs() < 1e-3, "{before:?} → {after:?}");
    // Undo: inside again, before the move.
    run(&mut s, "edit.undo", json!({}));
    assert!(s.editing_drawing());
    run(&mut s, "symbol.exit", json!({}));
    assert!((s.element_bounds(anicraft_doc::ElementId(id)).min.x - before.min.x).abs() < 1e-3);
}

/// Everything inside deleted: closing it removes the object.
#[test]
fn an_emptied_drawing_object_is_gone() {
    let (mut s, id) = object_and_raw();
    run(&mut s, "symbol.edit", json!({"id": id}));
    run(&mut s, "edit.select_all", json!({}));
    run(&mut s, "edit.delete", json!({}));
    run(&mut s, "symbol.exit", json!({}));
    assert!(s.visible_elements().iter().all(|e| e.id.0 != id));
    assert!(s.selection.is_empty());
    assert_eq!(s.visible_elements().len(), 1, "the raw square stays");
}

/// Raw shapes don't open (they are the drawing itself); the timeline stays one frame inside.
#[test]
fn only_drawing_objects_open() {
    let (mut s, _) = object_and_raw();
    let raw = s.visible_elements().iter().find(|e| matches!(&e.kind, ElementKind::Shape(sh) if sh.merge)).map(|e| e.id.0).expect("raw");
    assert!(s.execute("symbol.edit", &json!({"id": raw})).is_err());
    let id = s.visible_elements().iter().find(|e| matches!(&e.kind, ElementKind::Shape(sh) if !sh.merge)).map(|e| e.id.0).expect("object");
    run(&mut s, "symbol.edit", json!({"id": id}));
    assert!(s.execute("timeline.insert_keyframe", &json!({})).is_err(), "one frame inside");
}

// ----- Envelope ----------------------------------------------------------------------------------

fn points_of(v: &Value) -> Vec<[f32; 2]> {
    v["points"].as_array().expect("points").iter().map(|p| [p[0].as_f64().unwrap_or(0.0) as f32, p[1].as_f64().unwrap_or(0.0) as f32]).collect()
}

/// A 100 × 50 orange drawing object at (10, 20), selected, with an envelope open on it.
fn enveloped() -> (Session, Value) {
    let mut s = blank(true);
    run(&mut s, "draw.rect", json!({"x": 10, "y": 20, "w": 100, "h": 50, "stroke": null, "fill": "#ff8800"}));
    let v = run(&mut s, "modify.envelope", json!({}));
    (s, v)
}

/// At rest the envelope is the identity: its mesh is evenly over the bounds, and moving it to
/// the rest mesh changes nothing.
#[test]
fn envelope_identity_at_rest() {
    let (mut s, v) = enveloped();
    assert_eq!(v["bounds"], json!([10.0, 20.0, 100.0, 50.0]));
    let pts = points_of(&v);
    assert_eq!(pts.len(), 16);
    assert_eq!(pts[0], [10.0, 20.0]);
    assert_eq!(pts[15], [110.0, 70.0]);
    let before = shapes(&s);
    run(&mut s, "modify.envelope_move", json!({"points": v["points"]}));
    let after = shapes(&s);
    assert_eq!(before.len(), after.len());
    let flat = |sh: &Shape, m: &Affine| sh.path.flatten(m, 0.01).into_iter().flat_map(|p| p.points).collect::<Vec<_>>();
    for ((a, ma), (b, mb)) in before.iter().zip(&after) {
        let (pa, pb) = (flat(a, ma), flat(b, mb));
        assert_eq!(pa.len(), pb.len());
        assert!(pa.iter().zip(&pb).all(|(x, y)| x.distance(*y) < 1e-3));
    }
    assert!((fill_area(&s) - 5000.0).abs() < 0.01);
}

/// Lifting the top side's two handles by 30 bends the top edge up: the area grows by
/// lift · width / 2 (∫ B₁ + B₂ = ½), and each move is one undo step.
#[test]
fn envelope_bends_by_the_mesh() {
    let (mut s, v) = enveloped();
    let steps = s.history_len();
    let pts = points_of(&v);
    run(&mut s, "modify.envelope_move", json!({"index": 1, "x": pts[1][0], "y": pts[1][1] - 30.0}));
    let v = run(&mut s, "modify.envelope_move", json!({"index": 2, "x": pts[2][0], "y": pts[2][1] - 30.0}));
    assert_eq!(s.history_len(), steps + 2, "one step per move");
    let want = 5000.0 + 30.0 * 100.0 / 2.0;
    assert!((fill_area(&s) - want).abs() < want * 1e-3, "{} vs {want}", fill_area(&s));
    // The inner points followed the side (Coons).
    let now = points_of(&v);
    assert!(now[5][1] < pts[5][1] - 1.0 && now[6][1] < pts[6][1] - 1.0);
    // Undo: the mesh that made the shapes on the stage.
    run(&mut s, "edit.undo", json!({}));
    let (_, _, mesh) = s.envelope().expect("still open");
    assert!((mesh[2].y - pts[2][1]).abs() < 1e-4 && (mesh[1].y - (pts[1][1] - 30.0)).abs() < 1e-4);
    run(&mut s, "edit.undo", json!({}));
    let (_, rest, mesh) = s.envelope().expect("still open");
    assert_eq!(mesh, anicraft_geom::envelope::rest_mesh(&rest));
    assert!((fill_area(&s) - 5000.0).abs() < 0.01);
}

/// Moves bend the shapes as they were when the envelope opened, so a move and its reverse
/// leave them exactly as they were.
#[test]
fn envelope_moves_do_not_compound() {
    let (mut s, v) = enveloped();
    let pts = points_of(&v);
    let before = shapes(&s);
    run(&mut s, "modify.envelope_move", json!({"index": 15, "x": 160, "y": 120}));
    run(&mut s, "modify.envelope_move", json!({"index": 15, "x": pts[15][0], "y": pts[15][1]}));
    let after = shapes(&s);
    assert!((fill_area(&s) - 5000.0).abs() < 0.01);
    assert_eq!(before[0].0.path.cmds.len(), after[0].0.path.cmds.len());
}

/// Symbols, text and groups can't be bent; nothing open is an error for a move.
#[test]
fn envelope_refuses_what_it_cannot_bend() {
    let mut s = blank(true);
    assert!(s.execute("modify.envelope", &json!({})).is_err(), "nothing selected");
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 50, "h": 50, "fill": "#ff0000"}));
    run(&mut s, "modify.convert_to_symbol", json!({}));
    assert!(s.execute("modify.envelope", &json!({})).is_err(), "a symbol instance");
    assert!(s.execute("modify.envelope_move", &json!({"index": 0, "x": 1, "y": 1})).is_err());
    let (mut t, _) = enveloped();
    for p in [json!({"index": 16, "x": 0, "y": 0}), json!({"points": [[0, 0]]}), json!({"index": 0, "x": 1e9, "y": 0}), json!({})] {
        assert!(t.execute("modify.envelope_move", &p).is_err(), "{p}");
    }
    // A new selection ends it.
    run(&mut t, "edit.deselect", json!({}));
    assert!(t.envelope().is_none());
}

/// Raw shapes bent by an envelope merge with the drawing they land on when it closes.
#[test]
fn envelope_close_merges_raw_shapes() {
    let mut s = blank(false);
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 100, "stroke": null, "fill": "#0000ff"}));
    run(&mut s, "draw.rect", json!({"x": 200, "y": 0, "w": 100, "h": 100, "stroke": null, "fill": "#ff0000"}));
    // The red square is selected (the last drawn): bend its left side over the blue one.
    let v = run(&mut s, "modify.envelope", json!({}));
    let pts = points_of(&v);
    let mut m = pts.clone();
    for i in [0usize, 4, 8, 12] {
        m[i][0] -= 150.0;
    }
    run(&mut s, "modify.envelope_move", json!({"points": m}));
    run(&mut s, "modify.envelope_close", json!({}));
    assert!(s.envelope().is_none());
    // The red covers 50 of the blue square's width now.
    assert!((area_in(&s, 0x0000ff) - 5000.0).abs() < 5.0, "{}", area_in(&s, 0x0000ff));
    assert!((area_in(&s, 0xff0000) - 25_000.0).abs() < 25.0, "{}", area_in(&s, 0xff0000));
}

/// A drag's moves are one undo step and remember only the last mesh; undo goes back to before
/// the drag with the envelope still open.
#[test]
fn envelope_drag_is_one_step() {
    let (mut s, v) = enveloped();
    let pts = points_of(&v);
    let steps = s.history_len();
    s.begin_gesture("Envelope");
    for k in 1..=300 {
        run(&mut s, "modify.envelope_move", json!({"index": 15, "x": pts[15][0] + k as f32 * 0.1, "y": pts[15][1]}));
    }
    s.end_gesture();
    assert_eq!(s.history_len(), steps + 1);
    assert!(s.envelope.as_ref().is_some_and(|e| e.states_len() == 2));
    run(&mut s, "edit.undo", json!({}));
    let (_, rest, mesh) = s.envelope().expect("open");
    assert_eq!(mesh, anicraft_geom::envelope::rest_mesh(&rest));
    run(&mut s, "edit.redo", json!({}));
    let (_, _, mesh) = s.envelope().expect("open");
    assert!((mesh[15].x - (pts[15][0] + 30.0)).abs() < 1e-3);
}

/// Timings for ADR 0080 (`cargo test --release -p anicraft-engine parity_timings -- --ignored --nocapture`).
#[test]
#[ignore = "a measurement"]
fn parity_timings() {
    let median = |mut v: Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    };
    let wave: Vec<[f32; 2]> = (0..=400).map(|i| [20.0 + i as f32 * 2.5, 300.0 + (i as f32 * 0.05).sin() * 120.0]).collect();
    for mode in ["normal", "inside", "selection"] {
        let mut times = Vec::new();
        for _ in 0..9 {
            let mut s = blank(false);
            run(&mut s, "draw.ellipse", json!({"x": 100, "y": 100, "w": 800, "h": 400, "fill": "#3d8bfd", "stroke": "#000000", "stroke_width": 2}));
            run(&mut s, "edit.deselect", json!({}));
            if mode == "selection" {
                run(&mut s, "edit.select_all", json!({}));
            }
            let t = std::time::Instant::now();
            run(&mut s, "draw.brush", json!({"points": wave, "size": 30, "mode": mode, "color": "#ff0000"}));
            times.push(t.elapsed().as_secs_f64() * 1e3);
        }
        eprintln!("brush 1000 px wave, size 30, {mode}: {:.2} ms", median(times));
    }
    let mut times = Vec::new();
    for _ in 0..9 {
        let mut s = blank(true);
        run(&mut s, "draw.star", json!({"cx": 400, "cy": 300, "outer": 250, "sides": 32, "point_size": 0.5, "fill": "#ff0000"}));
        run(&mut s, "draw.ellipse", json!({"x": 100, "y": 100, "w": 600, "h": 400, "fill": "#00ff00"}));
        run(&mut s, "edit.select_all", json!({}));
        let v = run(&mut s, "modify.envelope", json!({}));
        let pts = points_of(&v);
        let t = std::time::Instant::now();
        for k in 0..10 {
            run(&mut s, "modify.envelope_move", json!({"index": 1, "x": pts[1][0], "y": pts[1][1] - 10.0 * k as f32}));
        }
        times.push(t.elapsed().as_secs_f64() * 1e3 / 10.0);
    }
    eprintln!("envelope move, a 32-point star and an oval: {:.2} ms", median(times));
}

/// Warped artwork (ADR 0079) neither opens in place nor takes an envelope: its warp keeps the
/// outline as drawn on every keyframe.
#[test]
fn warped_objects_refuse_envelope_and_editing() {
    let mut s = blank(true);
    let v = run(&mut s, "draw.rect", json!({"x": 10, "y": 20, "w": 100, "h": 50, "stroke": null, "fill": "#ff8800"}));
    let id = v["element"].as_u64().expect("the object");
    run(&mut s, "warp.add", json!({"element": id}));
    run(&mut s, "element.select", json!({"ids": [id]}));
    assert!(s.execute("modify.envelope", &json!({})).is_err());
    assert!(s.execute("symbol.edit", &json!({"id": id})).is_err());
    assert!(s.editing.is_empty());
}
