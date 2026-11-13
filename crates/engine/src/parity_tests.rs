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
