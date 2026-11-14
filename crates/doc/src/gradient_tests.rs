//! Gradient options, fill frames and fitting (ADR 0074).

use super::*;

fn stops() -> Vec<GradientStop> {
    vec![GradientStop { offset: 0.0, color: Rgba::BLACK }, GradientStop { offset: 1.0, color: Rgba::WHITE }]
}

fn near(a: &Affine, b: &Affine) -> bool {
    a.m.iter().zip(b.m).all(|(x, y)| (x - y).abs() <= 1e-4 * x.abs().max(y.abs()).max(1.0))
}

#[test]
fn spread_maps_positions_onto_the_ramp() {
    for (t, pad, reflect, repeat) in [(-0.25, 0.0, 0.25, 0.75), (0.0, 0.0, 0.0, 0.0), (0.4, 0.4, 0.4, 0.4), (1.25, 1.0, 0.75, 0.25), (2.5, 1.0, 0.5, 0.5), (-1.5, 0.0, 0.5, 0.5)] {
        assert!((Spread::Pad.apply(t) - pad).abs() < 1e-6, "pad {t}");
        assert!((Spread::Reflect.apply(t) - reflect).abs() < 1e-6, "reflect {t}");
        assert!((Spread::Repeat.apply(t) - repeat).abs() < 1e-6, "repeat {t}");
    }
    for s in Spread::ALL {
        assert_eq!(s.apply(f32::NAN), 0.0);
        assert!((0.0..=1.0).contains(&s.apply(f32::INFINITY)));
        assert_eq!(Spread::from_key(s.key()), Some(s));
    }
    assert_eq!(Spread::from_key("pad"), Some(Spread::Pad));
    assert_eq!(Spread::from_key("mirror"), None);
}

#[test]
fn frames_round_trip_for_every_fill_kind() {
    // Linear: x from start to end, y square to it at the same length.
    let lin = Paint::linear(Vec2::new(10.0, 20.0), Vec2::new(30.0, 20.0), stops());
    let f = lin.frame(None).unwrap();
    assert!(near(&f, &Affine { m: [10.0, 0.0, 0.0, 10.0, 20.0, 20.0] }), "{f:?}");
    assert_eq!(lin.with_frame(&f, None), lin);
    // Radial: a circle stays a plain circle, with no matrix.
    let rad = Paint::radial(Vec2::new(5.0, 6.0), 4.0, stops());
    let f = rad.frame(None).unwrap();
    assert!(near(&f, &Affine { m: [4.0, 0.0, 0.0, 4.0, 5.0, 6.0] }));
    assert_eq!(rad.with_frame(&f, None), rad);
    // A turned ellipse keeps its centre and its x axis's length as the radius.
    let ell = Affine::translate(50.0, 40.0).mul(&Affine::rotate(0.5)).mul(&Affine::scale(30.0, 10.0));
    let p = rad.with_frame(&ell, None);
    match &p {
        Paint::Radial { center, radius, matrix, .. } => {
            assert!(center.distance(Vec2::new(50.0, 40.0)) < 1e-4);
            assert!((radius - 30.0).abs() < 1e-4);
            assert!(near(&matrix.mul(&Affine::translate(center.x, center.y)), &Affine::translate(center.x, center.y).mul(&Affine { m: [matrix.m[0], matrix.m[1], matrix.m[2], matrix.m[3], 0.0, 0.0] })), "the matrix turns about the centre");
        }
        other => panic!("{other:?}"),
    }
    assert!(near(&p.frame(None).unwrap(), &ell));
    // Bitmap: the box onto one copy of the image.
    let bmp = Paint::Bitmap { bitmap: BitmapId(1), repeat: true, matrix: Affine::translate(3.0, 4.0).mul(&Affine::scale(2.0, 2.0)) };
    let f = bmp.frame(Some((10.0, 6.0))).unwrap();
    assert!(f.apply(Vec2::new(-1.0, -1.0)).distance(Vec2::new(3.0, 4.0)) < 1e-4, "top-left of the image");
    assert!(f.apply(Vec2::new(1.0, 1.0)).distance(Vec2::new(23.0, 16.0)) < 1e-4, "bottom-right");
    assert_eq!(bmp.frame(None), None, "a bitmap needs its size");
    assert!(near(&match bmp.with_frame(&f, Some((10.0, 6.0))) {
        Paint::Bitmap { matrix, .. } => matrix,
        _ => Affine::IDENTITY,
    }, &Affine::translate(3.0, 4.0).mul(&Affine::scale(2.0, 2.0))));
    // Solid colours have no frame; a frame that isn't finite changes nothing.
    assert_eq!(Paint::Solid(Rgba::WHITE).frame(None), None);
    assert_eq!(rad.with_frame(&Affine { m: [f32::NAN, 0.0, 0.0, 1.0, 0.0, 0.0] }, None), rad);
}

#[test]
fn radial_gradients_follow_any_transform_exactly() {
    // A skew turns the circle into a tilted ellipse: the frame is carried exactly, focal point
    // and all.
    let mut rad = Paint::radial(Vec2::new(10.0, 0.0), 5.0, stops());
    if let Paint::Radial { focal, .. } = &mut rad {
        *focal = 0.5;
    }
    let skew = Affine { m: [1.0, 0.0, 0.7, 2.0, 3.0, -1.0] };
    let moved = rad.transformed(&skew);
    assert!(near(&moved.frame(None).unwrap(), &skew.mul(&rad.frame(None).unwrap())));
    assert!(matches!(moved, Paint::Radial { focal, .. } if focal == 0.5));
    // A plain move keeps a plain circle.
    assert!(matches!(rad.transformed(&Affine::translate(4.0, 4.0)), Paint::Radial { matrix, .. } if matrix == Affine::IDENTITY));
}

#[test]
fn fitting_places_a_fill_on_the_shape() {
    let b = Rect { min: Vec2::new(0.0, 0.0), max: Vec2::new(200.0, 100.0) };
    match Paint::linear(Vec2::ZERO, Vec2::new(1.0, 1.0), stops()).fitted(&b) {
        Paint::Linear { start, end, .. } => assert_eq!((start, end), (Vec2::new(0.0, 50.0), Vec2::new(200.0, 50.0))),
        other => panic!("{other:?}"),
    }
    match Paint::radial(Vec2::ZERO, 1.0, stops()).fitted(&b) {
        Paint::Radial { center, radius, matrix, .. } => assert_eq!((center, radius, matrix), (Vec2::new(100.0, 50.0), 100.0, Affine::IDENTITY)),
        other => panic!("{other:?}"),
    }
    let bmp = Paint::Bitmap { bitmap: BitmapId(2), repeat: false, matrix: Affine::scale(3.0, 3.0) };
    assert_eq!(bmp.fitted(&b), Paint::Bitmap { bitmap: BitmapId(2), repeat: true, matrix: Affine::IDENTITY });
    // Stops and options stay; an empty box changes nothing.
    let mut rep = Paint::linear(Vec2::ZERO, Vec2::new(1.0, 0.0), stops());
    if let Paint::Linear { spread, linear_rgb, .. } = &mut rep {
        *spread = Spread::Repeat;
        *linear_rgb = true;
    }
    assert!(matches!(rep.fitted(&b), Paint::Linear { spread: Spread::Repeat, linear_rgb: true, .. }));
    assert_eq!(rep.fitted(&Rect::EMPTY), rep);
}

#[test]
fn older_files_open_and_plain_gradients_are_stored_as_before() {
    let old = r#"{"Radial":{"center":{"x":1.0,"y":2.0},"radius":3.0,"stops":[{"offset":0.0,"color":{"r":0.0,"g":0.0,"b":0.0,"a":1.0}}]}}"#;
    let p: Paint = serde_json::from_str(old).unwrap();
    assert!(matches!(p, Paint::Radial { spread: Spread::Pad, linear_rgb: false, focal, matrix, .. } if focal == 0.0 && matrix == Affine::IDENTITY));
    let back = serde_json::to_string(&p).unwrap();
    assert!(!back.contains("spread") && !back.contains("focal") && !back.contains("matrix") && !back.contains("linear_rgb"), "{back}");
    // Set options are written, and read back.
    let mut q = Paint::linear(Vec2::ZERO, Vec2::new(1.0, 0.0), stops());
    if let Paint::Linear { spread, linear_rgb, .. } = &mut q {
        *spread = Spread::Reflect;
        *linear_rgb = true;
    }
    let text = serde_json::to_string(&q).unwrap();
    assert!(text.contains("\"spread\":\"reflect\"") && text.contains("\"linear_rgb\":true"), "{text}");
    assert_eq!(serde_json::from_str::<Paint>(&text).unwrap(), q);
}

#[test]
fn radial_gradients_tween_through_their_frames() {
    let a = Paint::radial(Vec2::ZERO, 10.0, stops());
    let b = a.with_frame(&Affine::translate(100.0, 0.0).mul(&Affine::scale(30.0, 10.0)), None);
    let mid = a.lerp(&b, 0.5);
    assert!(near(&mid.frame(None).unwrap(), &Affine { m: [20.0, 0.0, 0.0, 10.0, 50.0, 0.0] }), "{mid:?}");
}
