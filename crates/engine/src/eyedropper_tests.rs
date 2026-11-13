//! The Eyedropper's commands (ADR 0034).

use crate::Session;
use anicraft_doc::{ElementId, ElementKind, GradientStop, Paint, ProfilePreset, Rgba, Shape};
use anicraft_geom::{Affine, Vec2};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn shape(s: &Session, id: u64) -> (Shape, Affine) {
    s.visible_elements()
        .iter()
        .find(|e| e.id == ElementId(id))
        .and_then(|e| match &e.kind {
            ElementKind::Shape(sh) => Some((sh.clone(), e.transform)),
            _ => None,
        })
        .expect("a shape")
}

fn rect(s: &mut Session, x: f32, fill: &str) -> u64 {
    run(s, "draw.rect", json!({"x": x, "y": 100, "w": 100, "h": 100, "fill": fill, "stroke": null}))["element"].as_u64().unwrap()
}

/// A linear gradient across the shape's own width (local space: the shape is centred on 0).
fn gradient() -> Paint {
    Paint::linear(Vec2::new(-50.0, 0.0), Vec2::new(50.0, 0.0), vec![GradientStop { offset: 0.0, color: Rgba::hex(0xff8800) }, GradientStop { offset: 1.0, color: Rgba::hex(0x0044ff) }])
}

#[test]
fn picking_a_solid_fill_sets_the_fill_colour_and_records_no_step() {
    let mut s = Session::default();
    rect(&mut s, 100.0, "#ff0000");
    let steps = s.history_len();
    let r = run(&mut s, "paint.pick", json!({"x": 150, "y": 150}));
    assert_eq!((r["picked"].as_str(), r["into"].as_str(), r["paint"].as_str()), (Some("fill"), Some("fill"), Some("solid")));
    assert_eq!(s.style.fill, Rgba::hex(0xff0000));
    assert!(s.style.fill_paint.is_none());
    assert_eq!(s.history_len(), steps, "picking changes the tool, not the document");
    assert!(s.execute("paint.pick", &json!({"x": 900, "y": 900})).is_err(), "nothing there");
    // Shift-click: into the other swatch only.
    s.style.set_fill_color(Rgba::hex(0x00ff00));
    let r = run(&mut s, "paint.pick", json!({"x": 150, "y": 150, "other": true}));
    assert_eq!(r["into"], "stroke");
    assert_eq!((s.style.stroke, s.style.fill), (Rgba::hex(0xff0000), Rgba::hex(0x00ff00)));
}

#[test]
fn a_picked_gradient_stays_where_it_is_on_the_stage() {
    let mut s = Session::default();
    let a = rect(&mut s, 100.0, "#ff0000");
    let b = rect(&mut s, 300.0, "#00ff00");
    for e in s.editable_keyframe().unwrap().elements_mut_where(|id| id == ElementId(a)) {
        if let ElementKind::Shape(sh) = &mut e.kind {
            sh.fill = Some(gradient());
        }
    }
    let r = run(&mut s, "paint.pick", json!({"x": 150, "y": 150}));
    assert_eq!(r["paint"], "linear");
    assert_eq!(s.style.fill, Rgba::hex(0xff8800), "the swatch shows its first colour");
    // In stage space: the shape's centre is (150, 150).
    match &s.style.fill_paint {
        Some(Paint::Linear { start, end, .. }) => assert!(start.distance(Vec2::new(100.0, 150.0)) < 1e-3 && end.distance(Vec2::new(200.0, 150.0)) < 1e-3, "{start:?} {end:?}"),
        other => panic!("{other:?}"),
    }
    // The Paint Bucket pours it into the other shape, mapped into that shape's space, so the
    // gradient carries on across the stage (Animate's Lock Fill).
    run(&mut s, "paint.bucket", json!({"x": 350, "y": 150}));
    let (sh, m) = shape(&s, b);
    match sh.fill {
        Some(Paint::Linear { start, .. }) => assert!(m.apply(start).distance(Vec2::new(100.0, 150.0)) < 1e-3, "{:?}", m.apply(start)),
        other => panic!("{other:?}"),
    }
    // Picking a colour in the swatch drops the gradient.
    s.style.set_fill_color(Rgba::hex(0x123456));
    assert!(s.style.fill_paint.is_none());
}

#[test]
fn picking_a_line_takes_its_colour_width_and_profile() {
    let mut s = Session::default();
    run(&mut s, "draw.line", json!({"x0": 100, "y0": 100, "x1": 300, "y1": 100, "stroke": "#336699", "stroke_width": 6, "profile": "taper_both"}));
    let r = run(&mut s, "paint.pick", json!({"x": 200, "y": 101}));
    assert_eq!((r["picked"].as_str(), r["profile"].as_str()), (Some("line"), Some("taper_both")));
    assert_eq!(s.style.stroke, Rgba::hex(0x336699));
    assert!((s.style.stroke_width - 6.0).abs() < 1e-4);
    assert_eq!(s.style.stroke_profile, ProfilePreset::TaperBoth);
    // A custom profile, on a line drawn twice the size: the width is as it shows on the stage.
    run(&mut s, "draw.line", json!({"x0": 100, "y0": 300, "x1": 300, "y1": 300, "stroke": "#000000", "stroke_width": 4, "profile": [[0, 1, 1], [0.3, 2, 2], [1, 0.5, 0.5]]}));
    run(&mut s, "element.set_transform", json!({"sx": 2, "sy": 2}));
    let r = run(&mut s, "paint.pick", json!({"x": 200, "y": 300}));
    assert_eq!(r["profile"], "custom");
    assert!(s.style.custom_profile.is_some() && (s.style.stroke_width - 8.0).abs() < 1e-3, "{}", s.style.stroke_width);
    // New lines get it.
    let id = run(&mut s, "draw.line", json!({"x0": 0, "y0": 0, "x1": 50, "y1": 0}))["element"].as_u64().unwrap();
    assert_eq!(shape(&s, id).0.stroke.and_then(|st| st.profile), s.style.custom_profile);
    // Shift-click on a line: its colour into the fill.
    run(&mut s, "paint.pick", json!({"x": 200, "y": 101, "other": true}));
    assert_eq!(s.style.fill, Rgba::hex(0x336699));
}

#[test]
fn applying_the_picked_style_to_the_selection_is_one_step() {
    let mut s = Session::default();
    let a = rect(&mut s, 100.0, "#ff0000");
    let b = rect(&mut s, 300.0, "#00ff00");
    run(&mut s, "element.select", json!({"ids": [b]}));
    run(&mut s, "paint.pick", json!({"x": 150, "y": 150}));
    let steps = s.history_len();
    let r = run(&mut s, "paint.apply_style", json!({"fill": true, "stroke": false}));
    assert_eq!(r["changed"], 1);
    assert_eq!(s.history_len(), steps + 1);
    assert_eq!(shape(&s, b).0.fill, Some(Paint::Solid(Rgba::hex(0xff0000))));
    assert_eq!(shape(&s, a).0.fill, Some(Paint::Solid(Rgba::hex(0xff0000))), "the source is unchanged");
    // The stroke settings go onto a drawing object that had none.
    run(&mut s, "paint.apply_style", json!({"fill": false}));
    assert!(shape(&s, b).0.stroke.is_some());
    run(&mut s, "edit.deselect", json!({}));
    assert!(s.execute("paint.apply_style", &json!({})).is_err(), "nothing selected");
    assert!(s.execute("paint.apply_style", &json!({"fill": false, "stroke": false})).is_err());
}

#[test]
fn picking_reads_locked_layers_too() {
    let mut s = Session::default();
    rect(&mut s, 100.0, "#ff0000");
    run(&mut s, "layer.toggle_lock", json!({}));
    run(&mut s, "paint.pick", json!({"x": 150, "y": 150}));
    assert_eq!(s.style.fill, Rgba::hex(0xff0000));
}
