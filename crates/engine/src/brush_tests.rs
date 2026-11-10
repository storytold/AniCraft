//! The Brush tool's command (ADR 0033).

use crate::Session;
use anicraft_doc::{ElementKind, Paint, Rgba, Shape};
use anicraft_geom::{Affine, FillRule, Vec2};
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

fn fill_of(sh: &Shape) -> Option<Rgba> {
    match sh.fill {
        Some(Paint::Solid(c)) => Some(c),
        _ => None,
    }
}

/// What colour covers stage point `p` (the topmost shape containing it).
fn top_at(s: &Session, p: Vec2) -> Option<Rgba> {
    shapes(s).iter().rev().find(|(sh, m)| sh.path.contains(p, m, sh.fill_rule)).and_then(|(sh, _)| fill_of(sh))
}

fn area(sh: &Shape) -> f32 {
    sh.path
        .flatten(&Affine::IDENTITY, 0.01)
        .iter()
        .map(|pl| {
            let n = pl.points.len();
            ((0..n).map(|i| pl.points[i].cross(pl.points[(i + 1) % n])).sum::<f32>() * 0.5).abs()
        })
        .sum()
}

/// The curve's own bounds (control points reach past it).
fn bounds(sh: &Shape) -> anicraft_geom::Rect {
    let mut r = anicraft_geom::Rect::EMPTY;
    sh.path.flatten(&Affine::IDENTITY, 0.01).iter().flat_map(|pl| pl.points.iter()).for_each(|p| r.include(*p));
    r
}

fn stroke(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<[f32; 2]> {
    (0..=30).map(|i| i as f32 / 30.0).map(|t| [x0 + (x1 - x0) * t, y0 + (y1 - y0) * t]).collect()
}

const RED: &str = "#ff0000";
const BLUE: &str = "#0000ff";

#[test]
fn a_straight_stroke_is_one_filled_shape_of_the_analytic_area() {
    let mut s = blank(true);
    let r = run(&mut s, "draw.brush", json!({"points": stroke(100.0, 100.0, 400.0, 100.0), "size": 24, "color": RED}));
    assert_eq!(r["merged"], false);
    let all = shapes(&s);
    assert_eq!(all.len(), 1);
    let (sh, m) = &all[0];
    assert!(sh.stroke.is_none() && fill_of(sh) == Some(Rgba::hex(0xff0000)) && sh.fill_rule == FillRule::EvenOdd);
    // Length × size, plus a circle for the two round ends.
    let want = 300.0 * 24.0 + std::f32::consts::PI * 144.0;
    assert!((area(sh) - want).abs() / want < 0.01, "area {} want {want}", area(sh));
    assert!(sh.path.contains(Vec2::new(250.0, 111.0), m, sh.fill_rule) && !sh.path.contains(Vec2::new(250.0, 113.0), m, sh.fill_rule));
    assert_eq!(s.selection.len(), 1, "what was painted is selected");
    // One undo step.
    run(&mut s, "edit.undo", json!({}));
    assert!(shapes(&s).is_empty());
}

#[test]
fn the_fill_colour_by_default_and_a_dab_for_one_point() {
    let mut s = blank(true);
    let r = run(&mut s, "draw.brush", json!({"points": [[50, 50]], "size": 10}));
    let (sh, _) = &shapes(&s)[0];
    assert_eq!(fill_of(sh), Some(s.style.fill));
    assert!((area(sh) - std::f32::consts::PI * 25.0).abs() < 0.8, "{}", area(sh));
    assert!(r["element"].is_u64());
}

#[test]
fn pressure_and_smoothing() {
    let mut s = blank(true);
    // Pressure ramps 0 → 1: thin at the start, full at the end.
    let pts: Vec<Value> = (0..=40).map(|i| json!([i as f32 * 10.0, 200.0, i as f32 / 40.0])).collect();
    run(&mut s, "draw.brush", json!({"points": pts, "size": 30}));
    let (sh, m) = &shapes(&s)[0];
    let inside = |x: f32, y: f32| sh.path.contains(Vec2::new(x, y), m, sh.fill_rule);
    assert!(!inside(20.0, 210.0) && inside(390.0, 210.0));
    // A shaky hand: more smoothing, fewer anchors.
    let shaky: Vec<[f32; 2]> = (0..=150).map(|i| [i as f32 * 3.0, 300.0 + ((i * 7919) % 11) as f32 * 0.4]).collect();
    let anchors = |s: &mut Session, smoothing: f32| {
        run(s, "draw.brush", json!({"points": shaky, "size": 14, "smoothing": smoothing}));
        shapes(s).last().map_or(0, |(sh, _)| sh.path.anchors().len())
    };
    let (rough, smooth) = (anchors(&mut s, 0.0), anchors(&mut s, 100.0));
    assert!(smooth * 2 < rough, "{rough} anchors unsmoothed, {smooth} smoothed");
}

#[test]
fn shapes_angle_and_roundness() {
    let mut s = blank(true);
    run(&mut s, "draw.brush", json!({"points": [[0, 0]], "size": 20, "shape": "square"}));
    let b = bounds(&shapes(&s)[0].0);
    // Square-ish: corners softened by well under a pixel.
    assert!((b.width() - 20.0).abs() < 0.5 && (b.height() - 20.0).abs() < 0.5, "{b:?}");
    // A flat oval tip turned upright paints tall and narrow.
    run(&mut s, "draw.brush", json!({"points": [[100, 100]], "size": 20, "roundness": 25, "angle": 90}));
    let b = bounds(&shapes(&s)[1].0);
    assert!((b.height() - 20.0).abs() < 0.5 && (b.width() - 5.0).abs() < 0.5, "{b:?}");
}

#[test]
fn paint_normal_covers_and_merges_with_raw_shapes() {
    let mut s = blank(false);
    run(&mut s, "draw.rect", json!({"x": 100, "y": 100, "w": 200, "h": 100, "fill": BLUE, "stroke": null}));
    let r = run(&mut s, "draw.brush", json!({"points": stroke(50.0, 150.0, 350.0, 150.0), "size": 20, "color": RED}));
    assert_eq!(r["merged"], true);
    assert!(shapes(&s).iter().all(|(sh, _)| sh.merge), "raw shapes");
    // The red band cuts the blue rectangle in two.
    let blue = shapes(&s).iter().filter(|(sh, _)| fill_of(sh) == Some(Rgba::hex(0x0000ff))).count();
    assert_eq!(blue, 2, "the rectangle is cut in two");
    assert_eq!(top_at(&s, Vec2::new(200.0, 150.0)), Some(Rgba::hex(0xff0000)));
    assert_eq!(top_at(&s, Vec2::new(200.0, 120.0)), Some(Rgba::hex(0x0000ff)));
    // The same colour joins: another red stroke across the first is one red region.
    run(&mut s, "draw.brush", json!({"points": stroke(200.0, 60.0, 200.0, 240.0), "size": 20, "color": RED}));
    let reds = shapes(&s).iter().filter(|(sh, _)| fill_of(sh) == Some(Rgba::hex(0xff0000))).count();
    assert_eq!(reds, 1, "brush strokes of one colour join");
}

#[test]
fn paint_behind_only_fills_empty_areas() {
    for object in [false, true] {
        let mut s = blank(object);
        run(&mut s, "draw.rect", json!({"x": 100, "y": 100, "w": 200, "h": 100, "fill": BLUE, "stroke": null}));
        run(&mut s, "draw.brush", json!({"points": stroke(50.0, 150.0, 350.0, 150.0), "size": 20, "color": RED, "mode": "behind"}));
        assert_eq!(top_at(&s, Vec2::new(200.0, 150.0)), Some(Rgba::hex(0x0000ff)), "object {object}: the rectangle stays on top");
        assert_eq!(top_at(&s, Vec2::new(70.0, 150.0)), Some(Rgba::hex(0xff0000)), "object {object}: empty areas take the paint");
        let first = shapes(&s).first().and_then(|(sh, _)| fill_of(sh));
        assert_eq!(first, Some(Rgba::hex(0xff0000)), "object {object}: painted at the bottom");
        // The CPU renderer agrees (the oracle).
        let px = |x, y| anicraft_render::render_frame(&s.doc, 0, 1.0).unwrap().pixel(x, y).unwrap();
        let (on, off) = (px(200, 150), px(70, 150));
        assert!(on[2] > 0.9 && on[0] < 0.1 && off[0] > 0.9 && off[2] < 0.1, "{on:?} {off:?}");
    }
}

#[test]
fn paint_fills_leaves_lines() {
    let mut s = blank(false);
    run(&mut s, "draw.line", json!({"x0": 200, "y0": 50, "x1": 200, "y1": 250, "stroke": "#000000", "stroke_width": 4}));
    run(&mut s, "draw.brush", json!({"points": stroke(100.0, 150.0, 300.0, 150.0), "size": 30, "color": RED, "mode": "fills"}));
    // The line is still on top where the paint crosses it.
    let lines: Vec<(Shape, Affine)> = shapes(&s).into_iter().filter(|(sh, _)| sh.stroke.is_some()).collect();
    assert!(!lines.is_empty());
    let on_top = shapes(&s).iter().rposition(|(sh, _)| sh.stroke.is_some()) > shapes(&s).iter().rposition(|(sh, _)| sh.fill.is_some());
    assert!(on_top, "lines stay above the paint");
    // Paint Normal over the same line: a fill on both sides hides it.
    let mut n = blank(false);
    run(&mut n, "draw.line", json!({"x0": 200, "y0": 50, "x1": 200, "y1": 250, "stroke": "#000000", "stroke_width": 4}));
    run(&mut n, "draw.brush", json!({"points": stroke(100.0, 150.0, 300.0, 150.0), "size": 30, "color": RED}));
    let hidden = shapes(&n).iter().filter(|(sh, _)| sh.stroke.is_some()).all(|(sh, m)| !sh.path.bounds().transformed(m).contains(Vec2::new(200.0, 150.0)));
    assert!(hidden, "Paint Normal covers the line where it crosses it");
}

#[test]
fn round_trips_through_the_file_format() {
    let mut s = blank(false);
    run(&mut s, "draw.brush", json!({"points": [[10, 10, 0.2], [60, 40, 0.8], [120, 20, 1.0]], "size": 16, "color": RED}));
    let json = crate::format::to_json(&s.doc).unwrap();
    let back = crate::format::from_json(&json).unwrap();
    assert_eq!(back, s.doc);
}

#[test]
fn bad_params_are_errors() {
    let mut s = blank(true);
    for p in [
        json!({}),
        json!({"points": []}),
        json!({"points": [[0, 0, "hard"]]}),
        json!({"points": [[0, 0]], "size": 0}),
        json!({"points": [[0, 0]], "size": 1e9}),
        json!({"points": [[0, 0]], "shape": "star"}),
        json!({"points": [[0, 0]], "mode": "inside"}),
        json!({"points": [[0, 0]], "roundness": 1}),
        json!({"points": [[0, 0]], "smoothing": 101}),
        json!({"points": [[0, 0], [1e7, 0]]}),
        json!({"points": [["a", 0]]}),
    ] {
        assert!(s.execute("draw.brush", &p).is_err(), "{p}");
    }
    assert!(crate::cmds_plugin::plugin_may_call("draw.brush").is_ok(), "plug-ins may paint");
}

#[test]
fn a_calligraphic_stroke_merges_whole() {
    // A flat tip at 45° along a wave: the raw shape covers what the object would.
    let wave: Vec<[f32; 2]> = (0..=160)
        .map(|i| {
            let t = i as f32 / 160.0 * std::f32::consts::TAU * 2.0;
            [190.0 + i as f32 * 2.6 + 34.0 * t.cos(), 170.0 + 40.0 * t.sin()]
        })
        .collect();
    let areas: Vec<f32> = [true, false]
        .into_iter()
        .map(|object| {
            let mut s = blank(object);
            run(&mut s, "draw.brush", json!({"points": wave, "size": 22, "roundness": 25, "angle": 45, "color": RED}));
            shapes(&s).iter().map(|(sh, _)| area(sh)).sum()
        })
        .collect();
    assert!(areas[0] > 7000.0 && (areas[0] - areas[1]).abs() < 1.0, "{areas:?}");
}
