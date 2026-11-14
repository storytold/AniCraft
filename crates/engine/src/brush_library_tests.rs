//! The brush library, pressure curves and the Paint Brush (ADR 0068).

use crate::Session;
use crate::brush_library::BUILTIN;
use anicraft_doc::{ElementKind, Shape};
use anicraft_geom::Affine;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn blank() -> Session {
    let mut s = Session::new(anicraft_doc::Document::default());
    run(&mut s, "draw.object_drawing", json!({"on": true}));
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

/// A 400-unit wave with the pressure swelling from 0.3 to 1 and back.
fn wave() -> Vec<[f32; 3]> {
    (0..=80)
        .map(|i| {
            let t = i as f32 / 80.0;
            [100.0 + 400.0 * t, 200.0 + 30.0 * (t * std::f32::consts::TAU).sin(), 0.3 + 0.7 * (t * std::f32::consts::PI).sin()]
        })
        .collect()
}

/// The wave's length (polyline).
fn wave_length() -> f32 {
    wave().windows(2).map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt()).sum()
}

fn painted(preset: &str, extra: Value) -> (Shape, Value) {
    let mut s = blank();
    let mut p = json!({"points": wave(), "preset": preset, "color": "#203040"});
    if let (Some(o), Some(e)) = (p.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    let r = run(&mut s, "draw.brush", p);
    let all = shapes(&s);
    assert_eq!(all.len(), 1, "{preset}: one shape");
    (all[0].clone(), r)
}

#[test]
fn every_preset_paints_within_its_area_bounds() {
    let len = wave_length();
    // (key, least, most) area as a part of length × size: what each preset's settings allow.
    let bounds = [
        ("pencil", 0.4, 0.65),       // a firm curve thins the light parts; short tapers
        ("ink_pen", 0.7, 0.9),       // a soft curve keeps it wide; tapered ends
        ("calligraphy", 0.45, 0.7),  // a 15 % flat nib: thick and thin by direction
        ("round_taper", 0.55, 0.75), // swells from a point and back
        ("marker", 0.75, 0.95),      // a square chisel at 45 %, no pressure
        ("chalk", 0.35, 0.55),       // its edge worn in
        // Dots 2.2 sizes apart, each a quarter π of size²: π / 8.8 = 0.357 of length × size.
        ("dotted", 0.33, 0.38),
        ("charcoal", 0.38, 0.52), // a 60 % oval with the paper grain through it
        ("spatter", 0.4, 0.65),  // dabs 0.6 sizes apart, 60–100 % of the size, thrown aside
    ];
    assert_eq!(bounds.len(), BUILTIN.len(), "every built-in has bounds");
    for (key, lo, hi) in bounds {
        let b = crate::brush_library::builtin(key).unwrap();
        let (sh, _) = painted(key, json!({}));
        let a = area(&sh) / (len * b.size);
        assert!((lo..=hi).contains(&a), "{key}: {a:.3} of length × size, want {lo}..{hi}");
        assert!(sh.stroke.is_none() && sh.fill.is_some(), "{key} paints a fill");
    }
    // Dots are separate loops; chalk's edge is ragged (many more anchors than a plain stroke).
    let (dots, _) = painted("dotted", json!({}));
    let loops = dots.path.cmds.iter().filter(|c| matches!(c, anicraft_geom::PathCmd::MoveTo(_))).count();
    let want = (wave_length() / (8.0 * 2.2)).floor() as usize + 1;
    assert!(loops.abs_diff(want) <= 2, "{loops} dots, about {want}");
    let (chalk, _) = painted("chalk", json!({}));
    let (plain, _) = painted("chalk", json!({"jitter": 0}));
    assert!(chalk.path.anchors().len() > 2 * plain.path.anchors().len());
}

#[test]
fn the_same_points_and_settings_give_identical_outlines() {
    for (key, _) in BUILTIN {
        for as_stroke in [false, true] {
            let (a, _) = painted(key, json!({"as_stroke": as_stroke}));
            let (b, _) = painted(key, json!({"as_stroke": as_stroke}));
            assert_eq!(a.path, b.path, "{key} as_stroke {as_stroke}");
            assert_eq!(a.stroke, b.stroke);
        }
    }
}

#[test]
fn pressure_curves_map_before_the_size() {
    // A steady half pressure: soft paints wider than linear, firm narrower.
    let pts: Vec<[f32; 3]> = (0..=40).map(|i| [100.0 + i as f32 * 10.0, 100.0, 0.5]).collect();
    let area_with = |curve: Value| {
        let mut s = blank();
        run(&mut s, "draw.brush", json!({"points": pts, "size": 20, "curve": curve}));
        area(&shapes(&s)[0])
    };
    let (lin, soft, firm) = (area_with(json!("linear")), area_with(json!("soft")), area_with(json!("firm")));
    // Linear: factor 0.1 + 0.9 × 0.5 = 0.55 of 20 across 400.
    assert!((lin - 400.0 * 11.0).abs() / (400.0 * 11.0) < 0.06, "{lin}");
    assert!(soft > lin * 1.15 && firm < lin * 0.85, "{soft} {lin} {firm}");
    // A curve of points: flat at 1 paints full size at any pressure.
    let full = area_with(json!([[0, 1], [1, 1]]));
    assert!((full - 400.0 * 20.0).abs() / (400.0 * 20.0) < 0.06, "{full}");
    // Pressure off: full size too.
    let mut s = blank();
    run(&mut s, "draw.brush", json!({"points": pts, "size": 20, "pressure_size": false}));
    assert!((area(&shapes(&s)[0]) - full).abs() < full * 0.01);
}

#[test]
fn pressure_can_set_the_opacity() {
    let pts: Vec<[f32; 3]> = (0..=10).map(|i| [i as f32 * 10.0, 50.0, 0.4]).collect();
    let mut s = blank();
    run(&mut s, "draw.brush", json!({"points": pts, "pressure_opacity": true, "color": "#ff0000"}));
    let Some(anicraft_doc::Paint::Solid(c)) = shapes(&s)[0].fill else { panic!("solid") };
    assert!((c.a - 0.4).abs() < 1e-3, "{}", c.a);
    // A mouse (no pressure) paints opaque.
    run(&mut s, "draw.brush", json!({"points": [[0, 100], [100, 100]], "pressure_opacity": true}));
    let Some(anicraft_doc::Paint::Solid(c)) = shapes(&s)[1].fill else { panic!("solid") };
    assert_eq!(c.a, 1.0);
}

#[test]
fn the_paint_brush_draws_a_width_profiled_stroke() {
    let mut s = blank();
    s.style.stroke = anicraft_doc::Rgba::hex(0x123456);
    let r = run(&mut s, "draw.brush", json!({"points": wave(), "size": 16, "as_stroke": true, "taper_end": 30}));
    assert_eq!(r["kind"], "paint_brush");
    let sh = &shapes(&s)[0];
    assert!(sh.fill.is_none(), "a line, not a fill");
    let st = sh.stroke.as_ref().unwrap();
    assert_eq!((st.width, st.color), (16.0, anicraft_doc::Rgba::hex(0x123456)), "the brush's size in the stroke colour");
    let prof = st.profile.as_ref().expect("pressure and a taper make a profile");
    let n = prof.points().len();
    assert!((3..=anicraft_doc::MAX_WIDTH_POINTS).contains(&n), "{n} width points");
    assert_eq!(r["width_points"], n);
    // Thin at the light start, full in the middle, tapered to a point at the end.
    let (l0, _) = prof.at(0.0);
    let (lm, rm) = prof.at(0.5);
    let (l1, _) = prof.at(1.0);
    assert!(l0 < 0.5 && lm > 0.9 && (lm - rm).abs() < 1e-6 && l1 < 0.1, "{l0} {lm} {l1}");
    // The Width tool edits it afterwards.
    let id = r["element"].as_u64().unwrap();
    run(&mut s, "stroke.set_width_point", json!({"id": id, "t": 0.5, "left": 3, "right": 3}));
    let sh = &shapes(&s)[0];
    assert!(sh.stroke.as_ref().and_then(|st| st.profile.as_ref()).is_some_and(|p| p.at(0.5).0 > 2.9));
    // A mouse stroke with no taper is a plain line of one width (it merges off Object Drawing).
    let mut s = blank();
    run(&mut s, "draw.object_drawing", json!({"on": false}));
    let r = run(&mut s, "draw.brush", json!({"points": [[0, 0], [50, 10], [100, 0]], "as_stroke": true}));
    assert_eq!(r["merged"], true);
    assert!(shapes(&s).iter().all(|sh| sh.stroke.as_ref().is_none_or(|st| st.profile.is_none())));
    // One point is no line.
    assert!(blank().execute("draw.brush", &json!({"points": [[5, 5]], "as_stroke": true})).is_err());
}

#[test]
fn user_brushes_are_saved_listed_renamed_and_deleted() {
    let mut s = blank();
    let v = run(&mut s, "brush.preset_list", json!({}));
    assert_eq!(v["presets"].as_array().map(Vec::len), Some(BUILTIN.len()));
    assert_eq!(v["presets"][0]["key"], "pencil");
    let v = run(&mut s, "brush.preset_save", json!({"name": "Wet Ink", "from": "ink_pen", "size": 9, "curve": [[0, 0], [0.5, 0.8], [1, 1]]}));
    assert_eq!((v["name"].as_str(), v["replaced"].as_bool()), (Some("Wet Ink"), Some(false)));
    let list = run(&mut s, "brush.preset_list", json!({}));
    let mine = &list["presets"][BUILTIN.len()];
    assert_eq!((mine["name"].as_str(), mine["builtin"].as_bool(), mine["size"].as_f64(), mine["taper_end"].as_f64()), (Some("Wet Ink"), Some(false), Some(9.0), Some(18.0)), "from the ink pen, size changed");
    // Painting with it by name equals painting with its settings.
    run(&mut s, "draw.brush", json!({"points": wave(), "preset": "wet ink"}));
    let mut t = blank();
    run(&mut t, "draw.brush", json!({"points": wave(), "preset": "ink_pen", "size": 9, "curve": [[0, 0], [0.5, 0.8], [1, 1]]}));
    assert_eq!(shapes(&s)[0].path, shapes(&t)[0].path);
    assert!(s.execute("draw.brush", &json!({"points": wave(), "preset": "nope"})).is_err());
    // Brushes aren't the document's: saving and renaming leave it as it is.
    let doc = s.doc.clone();
    run(&mut s, "brush.preset_rename", json!({"name": "Wet Ink", "to": "Dry Ink"}));
    run(&mut s, "brush.preset_save", json!({"name": "Spare"}));
    run(&mut s, "brush.preset_delete", json!({"name": "Spare", "confirm": true}));
    assert!(s.doc == doc);
    assert!(s.execute("brush.preset_save", &json!({"name": "Marker"})).is_err(), "built-in names are taken");
    // Deleting can't be undone, so it asks first.
    assert!(s.confirmation("brush.preset_delete", &json!({"name": "dry ink"})).is_some_and(|c| c.irreversible));
    assert!(s.confirmation("brush.preset_delete", &json!({"name": "nope"})).is_none());
    assert!(s.execute("brush.preset_delete", &json!({"name": "Dry Ink"})).is_err(), "needs confirm");
    run(&mut s, "brush.preset_delete", json!({"name": "Dry Ink", "confirm": true}));
    assert!(s.brushes.is_empty());
    // They survive New and Open.
    run(&mut s, "brush.preset_save", json!({"name": "Keep"}));
    s.replace_document(anicraft_doc::Document::default(), None);
    assert_eq!(s.brushes.user().len(), 1);
}

#[test]
fn brush_sets_export_and_import_as_json() {
    crate::vfs::use_memory();
    let mut s = blank();
    assert!(s.execute("brush.preset_export", &json!({"path": "none.json"})).is_err(), "nothing of yours yet");
    run(&mut s, "brush.preset_save", json!({"name": "Grit", "from": "chalk", "jitter": 30}));
    run(&mut s, "brush.preset_save", json!({"name": "Dots", "from": "dotted", "spacing": 150}));
    let v = run(&mut s, "brush.preset_export", json!({"path": "set.json"}));
    assert_eq!(v["brushes"], 2);
    let text = crate::vfs::read_to_string(std::path::Path::new("set.json")).unwrap();
    assert!(text.contains("\"format\": \"anicraft-brushes\"") && text.contains("\"Grit\""));
    // Exporting over the file asks first.
    assert!(s.confirmation("brush.preset_export", &json!({"path": "set.json"})).is_some_and(|c| c.irreversible));
    // Another computer: the set comes in whole; names already there get a number.
    let mut t = blank();
    run(&mut t, "brush.preset_save", json!({"name": "Grit"}));
    let v = run(&mut t, "brush.preset_import", json!({"path": "set.json"}));
    assert_eq!(v["imported"], json!(["Grit 2", "Dots"]));
    assert_eq!(t.brushes.find("Grit 2").map(|b| b.jitter), Some(30.0));
    // Built-ins can be exported by name too.
    let v = run(&mut s, "brush.preset_export", json!({"path": "two.json", "names": ["chalk", "Dots"], "confirm": true}));
    assert_eq!(v["brushes"], 2);
    // Junk is refused.
    crate::vfs::put(std::path::Path::new("junk.json"), b"{\"format\": 3}".to_vec()).unwrap();
    for bad in [json!({"path": "junk.json"}), json!({"path": "missing.json"}), json!({})] {
        assert!(t.execute("brush.preset_import", &bad).is_err(), "{bad}");
    }
}

#[test]
fn plug_ins_may_list_brushes_but_not_change_them() {
    use crate::cmds_plugin::plugin_may_call;
    assert!(plugin_may_call("brush.preset_list").is_ok());
    for id in ["brush.preset_save", "brush.preset_rename", "brush.preset_delete", "brush.preset_export", "brush.preset_import"] {
        assert!(plugin_may_call(id).is_err(), "{id}");
    }
}

#[test]
fn previews_are_drawn_by_the_renderer() {
    for (key, _) in BUILTIN {
        let b = crate::brush_library::builtin(key).unwrap();
        for as_stroke in [false, true] {
            let px = crate::cmds_brush::preview(&b, as_stroke, 160, 48, 2.0, anicraft_doc::Rgba::hex(0x000000)).unwrap();
            let rgba = px.to_premul_rgba8();
            let inked = rgba.chunks(4).filter(|p| p[3] > 128).count();
            assert!(inked > 60, "{key} as_stroke {as_stroke}: {inked} pixels");
            assert!(rgba.chunks(4).filter(|p| p[3] == 0).count() > inked, "{key}: mostly transparent");
        }
    }
    assert!(crate::cmds_brush::preview(&crate::brush_library::BrushPreset::default(), false, 2, 2, 1.0, anicraft_doc::Rgba::hex(0)).is_err());
}

/// Measured numbers for ADR 0068: `cargo test --release -p anicraft-engine preset_timings -- --ignored --nocapture`.
#[test]
#[ignore]
fn preset_timings() {
    for (key, _) in BUILTIN {
        for as_stroke in [false, true] {
            let mut ms: Vec<f64> = (0..15)
                .map(|_| {
                    let mut s = blank();
                    let t = std::time::Instant::now();
                    run(&mut s, "draw.brush", json!({"points": wave(), "preset": key, "as_stroke": as_stroke}));
                    t.elapsed().as_secs_f64() * 1000.0
                })
                .collect();
            ms.sort_by(f64::total_cmp);
            let (sh, _) = painted(key, json!({"as_stroke": as_stroke}));
            eprintln!("{key:12} stroke {as_stroke:5}: median {:6.2} ms, {} anchors", ms[7], sh.path.anchors().len());
        }
    }
    let b = crate::brush_library::builtin("chalk").unwrap();
    let t = std::time::Instant::now();
    for _ in 0..10 {
        crate::cmds_brush::preview(&b, false, 300, 80, 2.0, anicraft_doc::Rgba::hex(0)).unwrap();
    }
    eprintln!("chalk preview 300x80: {:.2} ms each", t.elapsed().as_secs_f64() * 100.0);
}

#[test]
fn simplified_profiles_stay_close_and_small() {
    let pts: Vec<(f32, f32)> = (0..256).map(|i| i as f32 / 255.0).map(|t| (t, 0.5 + 0.4 * (t * 9.0).sin())).collect();
    let kept = crate::cmds_brush::simplify_profile(&pts, 0.015, 64);
    assert!(kept.len() <= 64 && kept.len() > 8, "{}", kept.len());
    assert_eq!((kept.first(), kept.last()), (pts.first(), pts.last()));
    let noisy: Vec<(f32, f32)> = (0..256).map(|i| (i as f32 / 255.0, if i % 2 == 0 { 0.1 } else { 1.0 })).collect();
    assert!(crate::cmds_brush::simplify_profile(&noisy, 0.015, 64).len() <= 64);
}
