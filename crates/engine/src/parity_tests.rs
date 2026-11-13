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
