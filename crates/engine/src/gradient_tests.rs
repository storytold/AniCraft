//! The fill swatch's gradients, Lock Fill and the Gradient Transform tool's command (ADR 0074).

use crate::Session;
use anicraft_doc::{ElementId, ElementKind, Paint, Rgba, Shape, Spread};
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

/// A 100 × 100 square at (x, 100), selected (drawing commands select what they draw).
fn square(s: &mut Session, x: f32) -> u64 {
    run(s, "draw.rect", json!({"x": x, "y": 100, "w": 100, "h": 100, "fill": "#ff0000", "stroke": null}))["element"].as_u64().unwrap()
}

/// A 100 × 100 square at (x, 100) in the fill swatch.
fn poured(s: &mut Session, x: f32) -> u64 {
    run(s, "draw.rect", json!({"x": x, "y": 100, "w": 100, "h": 100, "stroke": null}))["element"].as_u64().unwrap()
}

/// Where a shape's fill lies on the stage.
fn stage_frame(s: &Session, id: u64) -> Affine {
    let (sh, m) = shape(s, id);
    m.mul(&sh.fill.and_then(|f| f.frame(None)).expect("a gradient"))
}

fn near(a: &Affine, b: &Affine) -> bool {
    a.m.iter().zip(b.m).all(|(x, y)| (x - y).abs() <= 1e-3 * x.abs().max(y.abs()).max(1.0))
}

const RAMP: &str = r##"[{"offset": 0, "color": "#ff8800"}, {"offset": 0.5, "color": "#ffffff80"}, {"offset": 1, "color": "#0044ff"}]"##;

#[test]
fn set_fill_makes_gradients_for_the_swatch_and_the_selection_in_one_step() {
    let mut s = Session::default();
    let a = square(&mut s, 100.0);
    let steps = s.history_len();
    let stops: Value = serde_json::from_str(RAMP).unwrap();
    let r = run(&mut s, "paint.set_fill", json!({"type": "linear", "stops": stops, "spread": "reflect", "linear_rgb": true}));
    assert_eq!(r["changed"], 1);
    assert_eq!(r["fill"]["type"], "linear");
    assert_eq!(r["fill"]["stops"][1]["color"], "#ffffff80", "alpha survives as #rrggbbaa");
    assert_eq!(s.history_len(), steps + 1, "one undo step");
    // The selected square's fill is fitted to it: across its width, through its middle.
    match shape(&s, a).0.fill {
        Some(Paint::Linear { start, end, stops, spread, linear_rgb }) => {
            assert_eq!((start, end), (Vec2::new(-50.0, 0.0), Vec2::new(50.0, 0.0)));
            assert_eq!((stops.len(), spread, linear_rgb), (3, Spread::Reflect, true));
            assert!((stops[1].color.a - 128.0 / 255.0).abs() < 1e-6);
        }
        other => panic!("{other:?}"),
    }
    // The swatch holds it too (across the stage), so the next shape gets it, fitted.
    assert!(matches!(s.style.fill_paint, Some(Paint::Linear { .. })));
    assert_eq!(s.style.fill, Rgba::hex(0xff8800), "the swatch shows the first colour");
    let b = run(&mut s, "draw.rect", json!({"x": 300, "y": 300, "w": 200, "h": 40, "stroke": null}))["element"].as_u64().unwrap();
    assert!(near(&stage_frame(&s, b), &Affine { m: [100.0, 0.0, 0.0, 100.0, 400.0, 320.0] }), "{:?}", stage_frame(&s, b));
    // Undo puts the solid colour back.
    s.undo().unwrap();
    s.undo().unwrap();
    assert_eq!(shape(&s, a).0.fill, Some(Paint::Solid(Rgba::hex(0xff0000))));
}

#[test]
fn editing_stops_keeps_where_a_gradient_lies() {
    let mut s = Session::default();
    let a = square(&mut s, 100.0);
    run(&mut s, "paint.set_fill", json!({"type": "radial"}));
    // A white-to-black ring round the square's centre, touching its sides.
    assert!(near(&stage_frame(&s, a), &Affine { m: [50.0, 0.0, 0.0, 50.0, 150.0, 150.0] }));
    run(&mut s, "paint.transform_fill", json!({"move": [10, 0], "scale_x": 2}));
    let moved = stage_frame(&s, a);
    // New stops, overflow and focal point: the ring stays put.
    let stops: Value = serde_json::from_str(RAMP).unwrap();
    run(&mut s, "paint.set_fill", json!({"stops": stops, "spread": "repeat", "focal": 0.5}));
    assert!(near(&stage_frame(&s, a), &moved));
    match shape(&s, a).0.fill {
        Some(Paint::Radial { stops, spread, focal, .. }) => assert_eq!((stops.len(), spread, focal), (3, Spread::Repeat, 0.5)),
        other => panic!("{other:?}"),
    }
    // Changing kind fits it again; back to solid, and the swatch is solid too.
    run(&mut s, "paint.set_fill", json!({"type": "linear"}));
    assert!(near(&stage_frame(&s, a), &Affine { m: [50.0, 0.0, 0.0, 50.0, 150.0, 150.0] }));
    assert!(matches!(shape(&s, a).0.fill, Some(Paint::Linear { spread: Spread::Repeat, .. })), "the gradient keeps its colours and overflow");
    run(&mut s, "paint.set_fill", json!({"type": "solid", "color": "#00ff00"}));
    assert_eq!(shape(&s, a).0.fill, Some(Paint::Solid(Rgba::hex(0x00ff00))));
    assert!(s.style.fill_paint.is_none());
}

#[test]
fn set_fill_checks_its_params() {
    let mut s = Session::default();
    square(&mut s, 100.0);
    for bad in [
        json!({"type": "conic"}),
        json!({"stops": [{"offset": 0, "color": "#ff0000"}]}),
        json!({"stops": [{"offset": 0, "color": "#ff0000"}, {"offset": 1.5, "color": "#ff0000"}]}),
        json!({"stops": [{"offset": 0, "color": "red"}, {"offset": 1, "color": "#ff0000"}]}),
        json!({"stops": (0..16).map(|i| json!({"offset": i as f32 / 15.0, "color": "#000000"})).collect::<Vec<_>>()}),
        json!({"spread": "mirror"}),
        json!({"focal": 1.0}),
        json!({"type": "bitmap"}),
        json!({"type": "bitmap", "bitmap": 999}),
    ] {
        let before = s.doc.clone();
        assert!(s.execute("paint.set_fill", &bad).is_err(), "{bad}");
        assert!(s.doc == before, "{bad} changed nothing");
    }
    // Unselected: only the swatch.
    s.selection.clear();
    let r = run(&mut s, "paint.set_fill", json!({"type": "radial"}));
    assert_eq!(r["changed"], 0);
    assert!(matches!(s.style.fill_paint, Some(Paint::Radial { .. })));
}

#[test]
fn transform_fill_moves_turns_scales_and_skews_analytically() {
    let mut s = Session::default();
    let a = square(&mut s, 100.0);
    run(&mut s, "paint.set_fill", json!({"type": "linear"}));
    let f0 = stage_frame(&s, a);
    // Scale along the gradient (its own x), then turn 90° about its centre, then move.
    run(&mut s, "paint.transform_fill", json!({"scale_x": 0.5, "rotate": 90, "move": [5, -5]}));
    let want = Affine::translate(5.0, -5.0).mul(&Affine::translate(150.0, 150.0)).mul(&Affine::rotate(std::f32::consts::FRAC_PI_2)).mul(&Affine::translate(-150.0, -150.0)).mul(&f0).mul(&Affine::scale(0.5, 1.0));
    let got = stage_frame(&s, a);
    // A linear gradient keeps only its axis: compare the ends.
    for x in [-1.0, 1.0] {
        assert!(got.apply(Vec2::new(x, 0.0)).distance(want.apply(Vec2::new(x, 0.0))) < 1e-3, "{got:?} vs {want:?}");
    }
    match shape(&s, a).0.fill {
        Some(Paint::Linear { start, end, .. }) => {
            assert!(start.distance(Vec2::new(5.0, -30.0)) < 1e-3 && end.distance(Vec2::new(5.0, 20.0)) < 1e-3, "{start:?} {end:?}");
        }
        other => panic!("{other:?}"),
    }
    // A radial gradient takes any frame: skew and an absolute frame are kept exactly.
    run(&mut s, "paint.set_fill", json!({"type": "radial"}));
    let f1 = stage_frame(&s, a);
    run(&mut s, "paint.transform_fill", json!({"skew_x": 30, "focal": -0.25}));
    let want = f1.mul(&Affine { m: [1.0, 0.0, 30f32.to_radians().tan(), 1.0, 0.0, 0.0] });
    assert!(near(&stage_frame(&s, a), &want), "{:?}", stage_frame(&s, a));
    assert!(matches!(shape(&s, a).0.fill, Some(Paint::Radial { focal, .. }) if focal == -0.25));
    let frame = [40.0, 10.0, -5.0, 20.0, 160.0, 140.0];
    let r = run(&mut s, "paint.transform_fill", json!({"element": a, "frame": frame}));
    assert_eq!(r["changed"], 1);
    assert!(near(&stage_frame(&s, a), &Affine { m: frame }));
    // The result reports the frames, so a script can carry on from them.
    assert_eq!(r["frames"][0].as_array().map(Vec::len), Some(6));
}

#[test]
fn transform_fill_turns_bitmap_fills_and_refuses_what_it_cannot_do() {
    let mut s = Session::default();
    // A 4 × 2 bitmap from the library, poured into a square.
    let png = anicraft_render::encode_png(4, 2, &[255; 32]).unwrap();
    let id = s.doc.alloc_id();
    s.doc.bitmaps.push(std::sync::Arc::new(anicraft_doc::Bitmap::from_png(anicraft_doc::BitmapId(id), "tile.png", png).unwrap()));
    let a = square(&mut s, 100.0);
    run(&mut s, "paint.set_fill", json!({"type": "bitmap", "bitmap": id}));
    let (sh, m) = shape(&s, a);
    let fill = sh.fill.clone().unwrap();
    let frame = |s: &Session| {
        let (sh, m) = shape(s, a);
        let f = sh.fill.unwrap();
        m.mul(&f.frame(Some((4.0, 2.0))).unwrap())
    };
    // Tiled from the square's top-left at its own size.
    assert!(near(&m.mul(&fill.frame(Some((4.0, 2.0))).unwrap()), &Affine { m: [2.0, 0.0, 0.0, 1.0, 102.0, 101.0] }));
    let f0 = frame(&s);
    let steps = s.history_len();
    run(&mut s, "paint.transform_fill", json!({"scale": 3, "skew_y": 20, "rotate": -45}));
    assert_eq!(s.history_len(), steps + 1, "one step");
    let c = f0.apply(Vec2::ZERO);
    let want = Affine::translate(c.x, c.y).mul(&Affine::rotate(-std::f32::consts::FRAC_PI_4)).mul(&Affine::translate(-c.x, -c.y)).mul(&f0).mul(&Affine::scale(3.0, 3.0)).mul(&Affine { m: [1.0, 20f32.to_radians().tan(), 0.0, 1.0, 0.0, 0.0] });
    assert!(near(&frame(&s), &want), "{:?} vs {want:?}", frame(&s));
    // Nothing to flatten a fill to, nor a solid fill to transform.
    let before = s.doc.clone();
    assert!(s.execute("paint.transform_fill", &json!({"frame": [1, 0, 2, 0, 0, 0]})).is_err());
    assert!(s.execute("paint.transform_fill", &json!({"scale_x": 0})).is_err());
    assert!(s.execute("paint.transform_fill", &json!({"skew_x": 90})).is_err());
    assert!(s.doc == before);
    let b = square(&mut s, 300.0);
    assert!(s.execute("paint.transform_fill", &json!({"element": b, "move": [1, 1]})).is_err(), "a solid fill has no gradient to move");
    s.selection.clear();
    assert!(s.execute("paint.transform_fill", &json!({"move": [1, 1]})).is_err(), "nothing selected");
}

#[test]
fn lock_fill_runs_one_gradient_across_shapes() {
    let mut s = Session::default();
    s.selection.clear();
    run(&mut s, "paint.set_fill", json!({"type": "linear"}));
    // The swatch spans the stage, left to right.
    let w = s.doc.stage.width as f32;
    match &s.style.fill_paint {
        Some(Paint::Linear { start, end, .. }) => assert_eq!((start.x, end.x), (0.0, w)),
        other => panic!("{other:?}"),
    }
    // Unlocked (the default), each shape gets its own ramp.
    let a = poured(&mut s, 100.0);
    assert!(near(&stage_frame(&s, a), &Affine { m: [50.0, 0.0, 0.0, 50.0, 150.0, 150.0] }));
    // Locked, the swatch's ramp across the stage, in each shape's own space.
    let r = run(&mut s, "paint.lock_fill", json!({}));
    assert_eq!(r["on"], true);
    let b = poured(&mut s, 300.0);
    let f = stage_frame(&s, b);
    assert!(f.apply(Vec2::new(-1.0, 0.0)).x.abs() < 1e-3 && (f.apply(Vec2::new(1.0, 0.0)).x - w).abs() < 1e-3, "{f:?}");
    // The Paint Bucket follows it too: a click on the unlocked square locks its ramp to the stage.
    run(&mut s, "paint.bucket", json!({"x": 150, "y": 150}));
    let f = stage_frame(&s, a);
    assert!(f.apply(Vec2::new(-1.0, 0.0)).x.abs() < 1e-3, "{f:?}");
    assert_eq!(run(&mut s, "paint.lock_fill", json!({"on": false}))["on"], false);
    run(&mut s, "paint.bucket", json!({"x": 350, "y": 150}));
    assert!(near(&stage_frame(&s, b), &Affine { m: [50.0, 0.0, 0.0, 50.0, 350.0, 150.0] }));
    assert!(s.execute("paint.lock_fill", &json!({"on": "yes"})).is_err());
}

#[test]
fn a_raw_fill_keeps_its_id_through_a_drag_of_its_gradient() {
    // The editor draws raw (merge) shapes; the Gradient Transform tool addresses one by id
    // through every step of a drag, so moving where its fill lies never re-merges it.
    let mut s = Session::default();
    s.style.object_drawing = false;
    let a = square(&mut s, 100.0);
    run(&mut s, "paint.set_fill", json!({"type": "radial"}));
    let id = s.selection.id_list().first().copied().unwrap_or(a);
    for k in 1..=3 {
        let r = run(&mut s, "paint.transform_fill", json!({"element": id, "frame": [50.0 + 10.0 * k as f32, 0, 0, 50, 150, 150]}));
        assert_eq!(r["changed"], 1, "step {k}");
    }
    assert!(near(&stage_frame(&s, id), &Affine { m: [80.0, 0.0, 0.0, 50.0, 150.0, 150.0] }));
}
