//! Asset Warp end to end (ADR 0079): meshes on a shape and a bitmap, pins added, moved and
//! removed, keyframed and tweened, undo, refusals, stale artwork, the file format, renders and
//! exports, and plug-in policy.

use super::*;
use anicraft_doc::warp::PinId;
use anicraft_doc::{Document, ElementKind, Shape, Stage};
use anicraft_geom::{Path, Vec2};
use anicraft_render::render_frame;
use serde_json::json;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    match s.execute(id, &p) {
        Ok(v) => v,
        Err(e) => panic!("{id} {p}: {e}"),
    }
}

fn err(s: &mut Session, id: &str, p: Value) -> String {
    match s.execute(id, &p) {
        Ok(v) => panic!("{id} {p} should fail, gave {v}"),
        Err(e) => e.to_string(),
    }
}

/// The shape of element `id` as it shows at `frame`.
fn shape_at(s: &Session, id: u64, frame: u32) -> Shape {
    s.timeline()
        .layers
        .iter()
        .find_map(|l| {
            let (_, els) = evaluate_layer_elements(l, frame)?;
            els.iter().find(|e| e.id.0 == id).and_then(|e| match &e.kind {
                ElementKind::Shape(sh) => Some(sh.clone()),
                _ => None,
            })
        })
        .unwrap_or_else(|| panic!("no shape {id} at frame {frame}"))
}

/// A 400 × 300 stage, 20 frames, with a blue 200 × 80 drawing object at (100, 100) and a
/// 40 × 20 bitmap placed at (300, 230). Returns the session, the rectangle and the bitmap.
pub(crate) fn warped() -> (Session, u64, u64) {
    let d = Document { stage: Stage { width: 400, height: 300, ..Stage::default() }, ..Document::default() };
    let mut s = Session::new(d);
    let rect = run(&mut s, "draw.rect", json!({"x": 100, "y": 100, "w": 200, "h": 80, "fill": "#3080ff", "object": true}))["element"].as_u64().unwrap();
    let bm = anicraft_doc::BitmapId(s.doc.alloc_id());
    let mut rgba = Vec::new();
    for y in 0..20u32 {
        for x in 0..40u32 {
            rgba.extend_from_slice(&[(x * 6) as u8, (y * 12) as u8, 128, 255]);
        }
    }
    let png = anicraft_render::encode_png(40, 20, &rgba).unwrap();
    s.doc.bitmaps.push(Arc::new(anicraft_doc::Bitmap::from_png(bm, "ramp.png", png).unwrap()));
    let bitmap = run(&mut s, "library.place", json!({"bitmap": bm.0, "x": 300, "y": 230}))["element"].as_u64().unwrap();
    run(&mut s, "timeline.insert_frame", json!({"frame": 19}));
    run(&mut s, "timeline.goto", json!({"frame": 0}));
    (s, rect, bitmap)
}

fn pin_add(s: &mut Session, el: u64, x: f32, y: f32) -> u64 {
    run(s, "warp.pin_add", json!({"element": el, "x": x, "y": y}))["pin"].as_u64().unwrap()
}

#[test]
fn pins_bend_a_shape_and_the_rest_pose_is_the_drawing() {
    let (mut s, rect, _) = warped();
    let drawn = shape_at(&s, rect, 0).path.clone();
    let before = render_frame(&s.doc, 0, 1.0).unwrap().data;
    let v = run(&mut s, "warp.add", json!({"element": rect}));
    assert_eq!((v["created"].as_bool(), v["resolution"].as_u64(), v["columns"].as_u64(), v["rows"].as_u64()), (Some(true), Some(12), Some(12), Some(5)));
    let a = pin_add(&mut s, rect, 110.0, 140.0);
    let b = pin_add(&mut s, rect, 200.0, 140.0);
    let c = pin_add(&mut s, rect, 290.0, 140.0);
    // Pins alone move nothing: the outline is the drawing, bit for bit, and so is the render.
    let sh = shape_at(&s, rect, 0);
    assert_eq!(*sh.path, *drawn);
    assert_eq!(render_frame(&s.doc, 0, 1.0).unwrap().data, before);
    let info = run(&mut s, "warp.info", json!({"element": rect}));
    assert_eq!((info["pins"].as_array().map(Vec::len), info["bent"].as_bool()), (Some(3), Some(false)));
    // Dragging the right pin down bends the right end most.
    run(&mut s, "warp.pin_move", json!({"element": rect, "pin": c, "x": 290, "y": 200}));
    let w = shape_at(&s, rect, 0).live_warp().cloned().unwrap();
    // The rectangle's own space is centred on it.
    let local = |x: f32, y: f32| Vec2::new(x - 200.0, y - 140.0);
    let moved = |x: f32| w.map(local(x, 140.0)).distance(local(x, 140.0));
    assert!(moved(280.0) > 40.0 && moved(250.0) > moved(150.0) && moved(120.0) < 3.0, "{} {} {}", moved(280.0), moved(250.0), moved(120.0));
    assert!(w.pin(PinId(a)).is_some() && w.pin(PinId(b)).is_some());
    assert_ne!(render_frame(&s.doc, 0, 1.0).unwrap().data, before);
    // Off the artwork there is nothing to pin; a pin can't go twice in one place.
    assert!(err(&mut s, "warp.pin_add", json!({"element": rect, "x": 10, "y": 10})).contains("on the artwork"));
    assert!(err(&mut s, "warp.pin_add", json!({"element": rect, "x": 110, "y": 140})).contains("already"));
}

#[test]
fn a_drag_is_one_undo_step_and_remove_restores_the_drawing() {
    let (mut s, rect, _) = warped();
    let drawn = shape_at(&s, rect, 0).path.clone();
    pin_add(&mut s, rect, 110.0, 140.0);
    let p = pin_add(&mut s, rect, 290.0, 140.0);
    let at = |s: &Session| shape_at(s, rect, 0).live_warp().and_then(|w| w.pin(PinId(p)).map(|q| q.at)).unwrap();
    s.begin_gesture("Move Pin");
    for i in 1..=10 {
        run(&mut s, "warp.pin_move", json!({"element": rect, "pin": p, "dx": 0, "dy": 3}));
        assert_eq!(at(&s).y, 3.0 * i as f32);
    }
    s.end_gesture();
    s.undo().unwrap();
    assert_eq!(at(&s), Vec2::new(90.0, 0.0), "the whole drag undoes at once");
    s.redo().unwrap();
    assert_eq!(at(&s).y, 30.0);
    // Removing a pin: the artwork follows the pins that are left (one pin: it just moves).
    run(&mut s, "warp.pin_remove", json!({"element": rect, "pin": p}));
    assert!(shape_at(&s, rect, 0).live_warp().unwrap().at_rest());
    run(&mut s, "warp.remove", json!({"element": rect}));
    let sh = shape_at(&s, rect, 0);
    assert!(sh.warp.is_none() && *sh.path == *drawn);
    assert!(err(&mut s, "warp.remove", json!({"element": rect})).contains("no warp"));
}

#[test]
fn pin_poses_tween_between_keyframes_with_easing() {
    let (mut s, rect, _) = warped();
    pin_add(&mut s, rect, 110.0, 140.0);
    let p = pin_add(&mut s, rect, 290.0, 140.0);
    // A keyframe at frame 11 holding the same mesh, a classic tween, then the pose there.
    let lid = s.timeline().layers[0].id.0;
    run(&mut s, "layer.select", json!({"layer": lid}));
    run(&mut s, "timeline.goto", json!({"frame": 10}));
    run(&mut s, "timeline.insert_keyframe", json!({}));
    run(&mut s, "timeline.goto", json!({"frame": 0}));
    run(&mut s, "timeline.create_tween", json!({}));
    run(&mut s, "warp.pin_move", json!({"element": rect, "pin": p, "x": 290, "y": 240, "frame": 10}));
    let pin_y = |s: &Session, f: u32| shape_at(s, rect, f).live_warp().and_then(|w| w.pin(PinId(p)).map(|q| q.at.y)).unwrap();
    assert_eq!((pin_y(&s, 0), pin_y(&s, 10)), (0.0, 100.0));
    assert!((pin_y(&s, 5) - 50.0).abs() < 1e-3, "halfway: {}", pin_y(&s, 5));
    // The tweened outline is the mesh bent by the halfway pins (not a blend of two outlines).
    let mid = shape_at(&s, rect, 5);
    let mut want = shape_at(&s, rect, 0).live_warp().cloned().unwrap();
    if let Some(q) = want.pins.iter_mut().find(|q| q.id == PinId(p)) {
        q.at.y = 50.0;
    }
    assert_eq!(*mid.path, *want.outline());
    // Easing applies to the pins.
    run(&mut s, "timeline.set_easing", json!({"easing": "quad_in"}));
    assert!(pin_y(&s, 5) < 40.0, "eased in: {}", pin_y(&s, 5));
    // A pin added later lands on both keyframes, where the artwork shows on each.
    let late = pin_add(&mut s, rect, 200.0, 110.0);
    for f in [0, 10] {
        assert!(shape_at(&s, rect, f).live_warp().unwrap().pin(PinId(late)).is_some(), "frame {f}");
    }
    // Moving a pin mid-tween keys the frame with what it shows.
    run(&mut s, "warp.pin_move", json!({"element": rect, "pin": late, "dx": 0, "dy": -5, "frame": 4}));
    assert!(s.timeline().layers[0].keyframe_starting_at(4).is_some());
}

#[test]
fn a_bitmap_bends_in_every_renderer_and_export() {
    let (mut s, _, bmp) = warped();
    let before = render_frame(&s.doc, 0, 1.0).unwrap();
    pin_add(&mut s, bmp, 285.0, 230.0);
    let p = pin_add(&mut s, bmp, 315.0, 230.0);
    assert_eq!(render_frame(&s.doc, 0, 1.0).unwrap().data, before.data, "pinned, not yet moved: exactly as drawn");
    run(&mut s, "warp.pin_move", json!({"element": bmp, "pin": p, "x": 315, "y": 270}));
    let after = render_frame(&s.doc, 0, 1.0).unwrap();
    // The bent image covers pixels below where the image was.
    let covered = |pm: &anicraft_render::Pixmap, x: u32, y: u32| pm.pixel(x, y).is_some_and(|c| c[2] > 0.45 && c[0] < 0.95);
    assert!(!covered(&before, 318, 262) && covered(&after, 318, 262));
    let info = run(&mut s, "warp.info", json!({"element": bmp}));
    assert_eq!((info["bitmap"].as_bool(), info["bent"].as_bool()), (Some(true), Some(true)));
    // SVG bakes the bent image; the HTML page reports it.
    let svg = crate::cmds_file::frame_svg(&s.doc, 0);
    assert!(svg.contains("<image x="));
    let html = anicraft_render::html::to_html(&s.doc, &anicraft_render::html::HtmlOptions { title: "t".into(), sound: false, fit: false }).unwrap();
    assert!(html.report.keys().any(|k| k.starts_with("warped bitmaps")), "{:?}", html.report);
}

#[test]
fn warps_survive_the_file_and_stale_artwork_starts_again() {
    let (mut s, rect, bmp) = warped();
    pin_add(&mut s, rect, 110.0, 140.0);
    let p = pin_add(&mut s, rect, 290.0, 140.0);
    run(&mut s, "warp.pin_move", json!({"element": rect, "pin": p, "x": 300, "y": 190}));
    pin_add(&mut s, bmp, 290.0, 230.0);
    let json = crate::format::to_json(&s.doc).unwrap();
    let back = crate::format::from_json(&json).unwrap();
    let mut t = Session::new(back);
    for el in [rect, bmp] {
        assert_eq!(shape_at(&t, el, 0), shape_at(&s, el, 0), "element {el}");
        assert!(shape_at(&t, el, 0).live_warp().is_some());
    }
    assert_eq!(render_frame(&t.doc, 0, 1.0).unwrap().data, render_frame(&s.doc, 0, 1.0).unwrap().data);
    // Pin ids count towards new ids: a new pin never reuses one.
    let q = pin_add(&mut t, rect, 120.0, 130.0);
    assert!(q > p);
    // Older files (no warp) still read; a warped file read by this build keeps its shape.
    let plain = crate::format::to_json(&warped().0.doc).unwrap();
    assert!(!plain.contains("\"warp\""));
    // Another tool changes the outline: the warp no longer shows, and the next pin move
    // starts from the shape as it is, with its pins where they show (nothing jumps).
    let edited = {
        let l = Arc::make_mut(&mut t.doc.timeline_mut(0).layers[0]);
        let k = Arc::make_mut(&mut l.keyframes[0]);
        let e = k.elements.iter_mut().find(|e| e.id.0 == rect).unwrap();
        let ElementKind::Shape(sh) = &mut Arc::make_mut(e).kind else { panic!() };
        sh.path = Arc::new(sh.path.transformed(&anicraft_geom::Affine::translate(5.0, 0.0)));
        sh.path.clone()
    };
    assert!(shape_at(&t, rect, 0).live_warp().is_none());
    assert_eq!(*shape_at(&t, rect, 0).path, *edited);
    run(&mut t, "warp.pin_move", json!({"element": rect, "pin": p, "dx": 0, "dy": 0}));
    let sh = shape_at(&t, rect, 0);
    assert!(sh.live_warp().is_some_and(|w| w.at_rest()) && *sh.path == *edited);
}

#[test]
fn warp_refusals() {
    let (mut s, rect, _) = warped();
    // Nothing selected and no element.
    s.selection.clear();
    assert!(err(&mut s, "warp.add", json!({})).contains("select"));
    assert!(err(&mut s, "warp.add", json!({"element": rect, "resolution": 1})).contains("resolution"));
    assert!(err(&mut s, "warp.add", json!({"element": 999_999})).contains("no element"));
    assert!(err(&mut s, "warp.pin_move", json!({"element": rect, "pin": 5, "x": 1, "y": 1})).contains("no warp"));
    // Symbols and text aren't warped.
    let sym = run(&mut s, "draw.ellipse", json!({"x": 10, "y": 10, "w": 30, "h": 30, "object": true}))["element"].as_u64().unwrap();
    run(&mut s, "modify.convert_to_symbol", json!({"name": "Dot"}));
    let inst = s.selection.on(s.layer).first().map(|e| e.0).unwrap_or(sym);
    assert!(err(&mut s, "warp.add", json!({"element": inst})).contains("shapes and bitmaps"));
    // A locked layer.
    let lid = s.timeline().layers[0].id.0;
    run(&mut s, "layer.toggle_lock", json!({"layer": lid}));
    assert!(err(&mut s, "warp.pin_add", json!({"element": rect, "x": 150, "y": 140})).contains("locked"));
}

#[test]
fn plug_ins_may_warp() {
    assert!(crate::cmds_plugin::plugin_may_call("warp.pin_move").is_ok());
    assert!(crate::cmds_plugin::plugin_may_call("warp.add").is_ok());
}

#[test]
fn warp_measured() {
    // A 1,200-anchor outline on a 48 × 48 mesh with 32 pins, bent: release numbers go in the ADR.
    let mut p = Path::new();
    p.move_to(Vec2::new(0.0, 0.0));
    for i in 1..600 {
        p.line_to(Vec2::new(i as f32, (i as f32 * 0.05).sin() * 20.0));
    }
    for i in (0..600).rev() {
        p.line_to(Vec2::new(i as f32, 200.0 + (i as f32 * 0.05).cos() * 20.0));
    }
    p.close();
    let mut w = anicraft_doc::warp::Warp::new(Arc::new(p), 48);
    w.pins = (0..32).map(|i| anicraft_doc::warp::Pin { id: PinId(i), rest: Vec2::new(i as f32 * 18.0, 100.0), at: Vec2::new(i as f32 * 18.0, 100.0 + (i % 5) as f32 * 9.0) }).collect();
    let t = std::time::Instant::now();
    let n = 20;
    let mut cmds = 0;
    for _ in 0..n {
        cmds += w.outline().cmds.len();
    }
    eprintln!("warp: bend 1,200 anchors on 48×48 with 32 pins: {:.3} ms per frame ({} commands)", t.elapsed().as_secs_f64() * 1000.0 / f64::from(n), cmds / n as usize);
}
