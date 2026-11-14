use super::*;

fn all() -> Vec<Easing> {
    let mut v: Vec<Easing> = Easing::named().collect();
    v.extend([Easing::Classic(-60.0), Easing::Classic(100.0), Easing::CubicBezier(0.25, 0.1, 0.25, 1.0), Easing::Custom(Arc::new(wave()))]);
    v
}

/// Three segments that overshoot both ways.
fn wave() -> CustomEase {
    CustomEase::from_flat(&[(0.0, 0.0), (0.1, 0.6), (0.2, 1.3), (0.3, 1.3), (0.4, 1.3), (0.5, -0.2), (0.6, 0.4), (0.7, 1.0), (0.9, 1.0), (1.0, 1.0)].map(|(x, y)| Vec2::new(x, y))).unwrap()
}

#[test]
fn endpoints_fixed() {
    for e in all() {
        assert!(e.apply(0.0).abs() < 1e-4, "{e:?}");
        assert!((e.apply(1.0) - 1.0).abs() < 1e-4, "{e:?}");
    }
}

#[test]
fn presets_at_known_times() {
    use std::f32::consts::PI;
    let cases: [(Easing, f32, f32); 22] = [
        (Easing::Linear, 0.3, 0.3),
        (Easing::QuadIn, 0.5, 0.25),
        (Easing::QuadOut, 0.5, 0.75),
        (Easing::QuadInOut, 0.25, 0.125),
        (Easing::CubicIn, 0.5, 0.125),
        (Easing::CubicOut, 0.5, 0.875),
        (Easing::QuartIn, 0.5, 0.0625),
        (Easing::QuartInOut, 0.75, 0.96875),
        (Easing::QuintIn, 0.5, 0.03125),
        (Easing::QuintOut, 0.5, 0.96875),
        (Easing::SineIn, 0.5, 1.0 - std::f32::consts::FRAC_1_SQRT_2),
        (Easing::SineOut, 1.0 / 3.0, 0.5),
        (Easing::SineInOut, 0.25, 0.5 * (1.0 - std::f32::consts::FRAC_1_SQRT_2)),
        // (c + 1)/8 − c/4 with c = 1.70158: dips below zero first.
        (Easing::BackIn, 0.5, -0.087_697_5),
        (Easing::BackOut, 0.5, 1.087_697_5),
        (Easing::CircIn, 0.6, 0.2),
        (Easing::CircOut, 0.5, 0.866_025_4),
        (Easing::BounceOut, 0.5, 0.765_625),
        (Easing::BounceIn, 0.5, 0.234_375),
        (Easing::BounceOut, 1.0 / 2.75, 1.0),
        // −2⁻⁵ · sin(−5.75 · 2π/3) = −1/64; Out is its mirror.
        (Easing::ElasticIn, 0.5, -0.015_625),
        (Easing::ElasticOut, 0.5, 1.015_625),
    ];
    for (e, t, want) in cases {
        assert!((e.apply(t) - want).abs() < 1e-5, "{e:?} at {t}: {} vs {want}", e.apply(t));
    }
    assert!((Easing::Classic(-100.0).apply(0.5) - 0.25).abs() < 1e-6);
    assert!((Easing::Classic(100.0).apply(0.5) - 0.75).abs() < 1e-6);
    assert!((Easing::SineIn.apply(1.0 / 3.0) - (1.0 - (PI / 6.0).cos())).abs() < 1e-6);
}

#[test]
fn out_mirrors_in_and_in_out_is_symmetric() {
    for f in EaseFamily::ALL {
        let (i, o, io) = (Easing::preset(f, EaseDir::In), Easing::preset(f, EaseDir::Out), Easing::preset(f, EaseDir::InOut));
        for k in 0..=40 {
            let t = k as f32 / 40.0;
            assert!((o.apply(t) - (1.0 - i.apply(1.0 - t))).abs() < 1e-5, "{f:?} at {t}");
            assert!((io.apply(t) + io.apply(1.0 - t) - 1.0).abs() < 1e-5, "{f:?} at {t}");
            assert!((io.apply(t * 0.5) - 0.5 * i.apply(t)).abs() < 1e-5, "{f:?} at {t}");
        }
        assert_eq!(Easing::preset(f, EaseDir::In).family(), Some((f, EaseDir::In)));
    }
}

#[test]
fn monotonic_families_never_turn_back_and_the_others_overshoot() {
    for e in all() {
        let overshoots = matches!(e.family(), Some((EaseFamily::Back | EaseFamily::Elastic, _))) || e.custom().is_some();
        // Bounce turns back (off the floor) without leaving 0..1.
        let turns = overshoots || matches!(e.family(), Some((EaseFamily::Bounce, _)));
        let (mut lo, mut hi, mut prev, mut turned) = (0.0f32, 1.0f32, -1e-6f32, false);
        for i in 0..=400 {
            let v = e.apply(i as f32 / 400.0);
            turned |= v < prev - 1e-4;
            prev = v;
            lo = lo.min(v);
            hi = hi.max(v);
        }
        assert_eq!(turned, turns, "{e:?}");
        assert_eq!(lo < -1e-3 || hi > 1.0 + 1e-3, overshoots, "{e:?}: {lo}..{hi}");
    }
}

#[test]
fn hostile_input_is_clamped() {
    assert_eq!(Easing::Linear.apply(f32::NAN), 0.0);
    assert_eq!(Easing::QuadIn.apply(5.0), 1.0);
    assert_eq!(Easing::ElasticOut.apply(f32::INFINITY), 0.0);
    assert!(Easing::CubicBezier(f32::NAN, 0.0, 1.0, 1.0).apply(0.5).is_finite());
    assert!(Easing::Classic(f32::INFINITY).apply(0.5).is_finite());
    assert!(Easing::Custom(Arc::new(wave())).apply(f32::NAN).is_finite());
}

#[test]
fn bezier_forms_draw_the_same_curves() {
    // Exact for these.
    for e in [Easing::Linear, Easing::QuadIn, Easing::QuadOut, Easing::CubicIn, Easing::CubicOut, Easing::BackIn, Easing::BackOut, Easing::Classic(-60.0), Easing::Classic(100.0), Easing::CubicBezier(0.2, 1.4, 0.6, -0.3)] {
        let (a, b, c, d) = e.as_bezier().unwrap();
        let bez = Easing::CubicBezier(a, b, c, d);
        for i in 0..=20 {
            let t = i as f32 / 20.0;
            assert!((e.apply(t) - bez.apply(t)).abs() < 1e-3, "{e:?} at {t}");
        }
    }
    for e in [Easing::QuadInOut, Easing::CubicInOut] {
        let (a, b, c, d) = e.as_bezier().unwrap();
        let bez = Easing::CubicBezier(a, b, c, d);
        assert!((0..=20).all(|i| (e.apply(i as f32 / 20.0) - bez.apply(i as f32 / 20.0)).abs() < 0.08), "{e:?}");
    }
    // One curve can't draw a bounce or a multi-segment ease.
    assert_eq!(Easing::BounceOut.as_bezier(), None);
    assert_eq!(Easing::Custom(Arc::new(wave())).as_bezier(), None);
    let one = Easing::Custom(Arc::new(CustomEase::from_bezier(0.1, 0.9, 0.3, 1.2)));
    assert_eq!(one.as_bezier(), Some((0.1, 0.9, 0.3, 1.2)));
}

#[test]
fn every_preset_becomes_a_custom_curve_for_the_graph() {
    for e in all() {
        let c = e.to_custom();
        assert!(c.points().len() <= MAX_EASE_POINTS, "{e:?}");
        let exact = matches!(e.family(), Some((EaseFamily::Quad | EaseFamily::Cubic | EaseFamily::Back | EaseFamily::Bounce, _))) || e.family().is_none();
        let tol = if exact { 2e-4 } else { 5e-3 };
        let mut worst = 0.0f32;
        for i in 0..=500 {
            let t = i as f32 / 500.0;
            worst = worst.max((c.apply(t) - e.apply(t)).abs());
        }
        assert!(worst < tol, "{e:?}: off by {worst} with {} points", c.points().len());
    }
}

#[test]
fn ease_in_is_slow_at_start() {
    assert!(Easing::QuadIn.apply(0.25) < 0.25);
    assert!(Easing::QuadOut.apply(0.25) > 0.25);
    assert!(Easing::Classic(-100.0).apply(0.25) < 0.25);
    assert!(Easing::Classic(100.0).apply(0.25) > 0.25);
}

#[test]
fn names_round_trip_and_older_aliases_still_read() {
    for e in Easing::named() {
        let n = e.name().unwrap();
        assert_eq!(Easing::from_name(&n), Some(e.clone()), "{n}");
        // The file form is the same name.
        assert_eq!(serde_json::to_string(&e).unwrap(), format!("\"{n}\""));
    }
    assert_eq!(Easing::from_name("back in-out"), Some(Easing::BackInOut));
    assert_eq!(Easing::from_name("ease_in"), Some(Easing::QuadIn));
    assert_eq!(Easing::from_name("Smooth"), Some(Easing::CubicBezier(0.25, 0.1, 0.25, 1.0)));
    assert_eq!(Easing::from_name("wobble"), None);
    assert_eq!(Easing::named().count(), 28);
    assert_eq!(Easing::BounceInOut.label(), "Bounce In Out");
    // Older files.
    let old: Vec<Easing> = serde_json::from_str(r#"["QuadIn", {"Classic": 30.0}, {"CubicBezier": [0.1, 0.2, 0.3, 0.4]}, "Linear"]"#).unwrap();
    assert_eq!(old, [Easing::QuadIn, Easing::Classic(30.0), Easing::CubicBezier(0.1, 0.2, 0.3, 0.4), Easing::Linear]);
}

#[test]
fn a_multi_segment_curve_evaluates_continuously_through_its_points() {
    let c = wave();
    assert_eq!(c.points().len(), 4);
    for p in c.points() {
        assert!((c.apply(p.at.x) - p.at.y).abs() < 1e-5 || p.at.x == 0.0, "{p:?}");
    }
    let mut prev = c.apply(0.0);
    for i in 1..=10_000 {
        let v = c.apply(i as f32 / 10_000.0);
        assert!((v - prev).abs() < 2e-3, "a jump at {i}: {prev} → {v}");
        prev = v;
    }
    // Rises past the end value, then falls back below where it was, as drawn.
    assert!(c.apply(0.3) > 1.2 && c.apply(0.55) < 0.4);
}

#[test]
fn adding_a_point_keeps_the_shape() {
    let c = wave();
    let (d, i) = c.insert_at(0.42).unwrap();
    assert_eq!((d.points().len(), i), (5, 2));
    assert!(d.points()[2].smooth);
    for k in 0..=200 {
        let t = k as f32 / 200.0;
        assert!((c.apply(t) - d.apply(t)).abs() < 1e-4, "at {t}");
    }
    // Not at an end or on a point.
    assert!(c.insert_at(0.0).is_none() && c.insert_at(1.0).is_none() && c.insert_at(0.3).is_none() && c.insert_at(f32::NAN).is_none());
}

#[test]
fn point_edits_keep_time_running_forwards() {
    let c = wave();
    // Dragging a point past its neighbour stops at the neighbour.
    let d = c.move_point(1, Vec2::new(0.95, 2.0)).unwrap();
    assert_eq!(d.points()[1].at, Vec2::new(0.6, 2.0));
    // The ends don't move.
    assert!(c.move_point(0, Vec2::new(0.5, 0.5)).is_none() && c.move_point(3, Vec2::new(0.5, 0.5)).is_none());
    // A handle dragged backwards in time stops at its point.
    let h = c.move_handle(1, Handle::Out, Vec2::new(0.0, 5.0)).unwrap();
    assert_eq!(h.points()[1].out.x, 0.0);
    // A smooth point's other handle turns with it.
    let s = c.set_smooth(1, true).unwrap();
    let t = s.move_handle(1, Handle::Out, Vec2::new(0.45, 1.5)).unwrap();
    let p = t.points()[1];
    assert!(p.smooth && (p.h_in.normalized().dot(p.out.normalized()) + 1.0).abs() < 1e-4, "{p:?}");
    // A corner has its handles on the point.
    let k = s.set_smooth(1, false).unwrap();
    assert_eq!((k.points()[1].h_in, k.points()[1].out), (Vec2::ZERO, Vec2::ZERO));
    // Removing.
    assert_eq!(c.remove(2).unwrap().points().len(), 3);
    assert!(c.remove(0).is_none() && c.remove(3).is_none());
}

/// Every segment's x control points lie between its ends, so x never decreases.
fn assert_forwards(c: &CustomEase) {
    let pts = c.points();
    assert!(pts.len() >= 2 && pts.len() <= MAX_EASE_POINTS);
    assert_eq!((pts[0].at, pts[pts.len() - 1].at), (Vec2::ZERO, Vec2::new(1.0, 1.0)));
    for i in 0..pts.len() - 1 {
        let [a, b, cc, d] = c.segment(i).unwrap();
        assert!(a.x <= b.x && b.x <= d.x && a.x <= cc.x && cc.x <= d.x, "segment {i}: {a:?} {b:?} {cc:?} {d:?}");
        for v in [a, b, cc, d] {
            assert!(v.is_finite() && v.y.abs() <= MAX_EASE_Y + 1e-4);
        }
    }
    for k in 0..=100 {
        assert!(c.apply(k as f32 / 100.0).is_finite());
    }
}

#[test]
fn x_stays_monotonic_after_any_edit() {
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut rnd = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % 10_000) as f32 / 10_000.0
    };
    let mut c = wave();
    for _ in 0..3000 {
        let n = c.points().len();
        let i = (rnd() * n as f32) as usize % n;
        let to = Vec2::new(rnd() * 1.6 - 0.3, rnd() * 30.0 - 15.0);
        let next = match (rnd() * 6.0) as u32 {
            0 => c.insert_at(rnd()).map(|r| r.0),
            1 => c.move_point(i, to),
            2 => c.move_handle(i, Handle::In, to),
            3 => c.move_handle(i, Handle::Out, to),
            4 => c.set_smooth(i, rnd() > 0.5),
            _ => if n > 6 { c.remove(i) } else { None },
        };
        if let Some(d) = next {
            assert_forwards(&d);
            c = d;
        }
    }
}

#[test]
fn hostile_curves_are_repaired() {
    // NaN, reversed x, out-of-range values and handles reaching backwards.
    let nan = EasePoint { at: Vec2::new(f32::NAN, 0.5), ..Default::default() };
    let pts = vec![
        EasePoint { at: Vec2::new(1.0, 1.0), h_in: Vec2::new(-5.0, f32::INFINITY), ..Default::default() },
        EasePoint { at: Vec2::new(0.6, 400.0), h_in: Vec2::new(3.0, 0.0), out: Vec2::new(-2.0, 0.0), smooth: true },
        nan,
        EasePoint { at: Vec2::new(0.2, -1.0), ..Default::default() },
        EasePoint { at: Vec2::new(-3.0, 0.0), out: Vec2::new(f32::NAN, 1.0), ..Default::default() },
    ];
    let c = CustomEase::new(pts);
    assert_eq!(c.points().len(), 4);
    assert_forwards(&c);
    assert_eq!(c.points()[2].at, Vec2::new(0.6, MAX_EASE_Y));
    // Thousands of points are thinned to the cap, and stay a valid curve.
    let many: Vec<EasePoint> = (0..20_000).map(|i| EasePoint { at: Vec2::new(((i * 7919) % 20_000) as f32 / 20_000.0, (i as f32 * 0.37).sin()), out: Vec2::new(0.5, 0.0), h_in: Vec2::new(-0.5, 0.0), smooth: false }).collect();
    let c = CustomEase::new(many);
    assert_eq!(c.points().len(), MAX_EASE_POINTS);
    assert_forwards(&c);
    // Too few points: the line.
    assert_eq!(CustomEase::new(vec![nan]), CustomEase::linear());
    // From a file, the same repairs.
    let f: CustomEase = serde_json::from_str(r#"{"points": [{"at": {"x": 0.9, "y": 3.0}}, {"at": {"x": 0.1, "y": 0.0}, "out": {"x": -1.0, "y": 0.0}}, {"at": {"x": 0.5, "y": 0.5}}]}"#).unwrap();
    assert_forwards(&f);
    let e: Easing = serde_json::from_str(r#"{"Custom": {"points": []}}"#).unwrap();
    assert_eq!(e, Easing::Custom(Arc::new(CustomEase::linear())));
}

#[test]
fn custom_eases_round_trip_through_files_and_flat_lists() {
    let e = Easing::Custom(Arc::new(wave()));
    let back: Easing = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
    assert_eq!(back, e);
    let flat = wave().to_flat();
    assert_eq!(flat.len(), 10);
    assert_eq!(CustomEase::from_flat(&flat), Some(wave()));
    assert!(CustomEase::from_flat(&flat[..9]).is_none());
}

/// Four steps, as the sample ease plug-in samples them.
fn steps() -> SampledEase {
    SampledEase::new("Stepped 4", "org.anicraft.stepped-ease", "steps4", (0..257).map(|i| ((i as f32 / 256.0) * 4.0).floor() / 4.0).collect())
}

#[test]
fn a_sampled_ease_reads_its_table_between_samples() {
    let s = steps();
    let e = Easing::Sampled(Arc::new(s.clone()));
    assert_eq!((e.apply(0.0), e.apply(0.1), e.apply(0.25), e.apply(0.6), e.apply(0.9), e.apply(1.0)), (0.0, 0.0, 0.25, 0.5, 0.75, 1.0));
    // Between two samples, the straight line between them.
    let lin = SampledEase::new("Line", "p", "l", vec![0.0, 0.5, 1.0]);
    assert!((lin.apply(0.25) - 0.25).abs() < 1e-6 && (lin.apply(0.75) - 0.75).abs() < 1e-6);
    // Time outside 0..1, or not a number, is clamped.
    assert_eq!((e.apply(-3.0), e.apply(7.0), e.apply(f32::NAN)), (0.0, 1.0, 0.0));
    assert_eq!(e.label(), "Stepped 4");
    assert!(e.family().is_none() && e.as_bezier().is_none());
    assert_eq!(e.sampled().map(|s| (s.plugin(), s.ease())), Some(("org.anicraft.stepped-ease", "steps4")));
}

#[test]
fn a_sampled_ease_is_always_valid() {
    let s = SampledEase::new("  ", "bad id!/..", "x\u{202e}y", vec![0.3, f32::NAN, 99.0, -f32::INFINITY, 0.2]);
    assert_eq!(s.values(), &[0.0, 0.25, MAX_EASE_Y, 0.75, 1.0]);
    assert_eq!((s.name(), s.plugin(), s.ease()), ("Plug-in ease", "badid..", "xy"));
    assert_eq!(SampledEase::new("x", "p", "e", vec![]).values(), &[0.0, 1.0]);
    assert_eq!(SampledEase::new("x", "p", "e", vec![0.5]).values(), &[0.0, 1.0]);
    let long = SampledEase::new("x", "p", "e", (0..100_000).map(|i| i as f32 / 99_999.0).collect());
    assert_eq!(long.values().len(), MAX_EASE_SAMPLES);
    assert!((long.apply(0.5) - 0.5).abs() < 1e-3);
}

#[test]
fn a_sampled_ease_round_trips_through_the_file_format_and_survives_bad_files() {
    let e = Easing::Sampled(Arc::new(steps()));
    let text = serde_json::to_string(&e).unwrap();
    assert!(text.starts_with("{\"Sampled\":{\"name\":\"Stepped 4\",\"plugin\":\"org.anicraft.stepped-ease\",\"ease\":\"steps4\",\"values\":[0.0,"), "{text}");
    assert_eq!(serde_json::from_str::<Easing>(&text).unwrap(), e);
    // A hand-edited table is made valid on load.
    assert!(serde_json::from_str::<Easing>(r#"{"Sampled":{"values":[5,0.5,"x"]}}"#).is_err(), "a non-number value is a broken file, refused as any other");
    let odd: Easing = serde_json::from_str(r#"{"Sampled":{"name":"Odd","values":[5,0.5,-20]}}"#).unwrap();
    assert_eq!(odd.sampled().unwrap().values(), &[0.0, 0.5, 1.0]);
}

#[test]
fn a_sampled_ease_becomes_a_custom_curve_through_its_samples() {
    let wave = SampledEase::new("Wave", "p", "w", (0..257).map(|i| { let t = i as f32 / 256.0; t + 0.2 * (t * std::f32::consts::TAU).sin() }).collect());
    let c = wave.to_polyline();
    assert_eq!(c.points().len(), POLYLINE_POINTS);
    for i in 0..=100 {
        let t = i as f32 / 100.0;
        assert!((c.apply(t) - wave.apply(t)).abs() < 0.01, "t = {t}: {} vs {}", c.apply(t), wave.apply(t));
    }
    let short = SampledEase::new("Short", "p", "s", vec![0.0, 0.8, 1.0]).to_polyline();
    assert_eq!(short.points().len(), 3);
    assert!((short.apply(0.5) - 0.8).abs() < 1e-3);
}
