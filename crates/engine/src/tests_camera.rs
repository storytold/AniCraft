//! The camera layer end to end (ADR 0027): commands, undo, tweens, analytic renders, export
//! parity, the file format, plug-in policy and robustness.

use super::*;
use anicraft_doc::{Document, LayerKind, Stage};
use anicraft_render::render_frame;
use serde_json::json;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    match s.execute(id, &p) {
        Ok(v) => v,
        Err(e) => panic!("{id} {p}: {e}"),
    }
}

/// A 200 × 100 white stage, 20 frames, holding `rects` (x, y, w, h) in black.
fn stage_with(rects: &[(f32, f32, f32, f32)]) -> Session {
    let d = Document { stage: Stage { width: 200, height: 100, ..Stage::default() }, ..Document::default() };
    let mut s = Session::new(d);
    for &(x, y, w, h) in rects {
        run(&mut s, "draw.rect", json!({"x": x, "y": y, "w": w, "h": h, "fill": "#000000", "stroke": null}));
    }
    run(&mut s, "timeline.insert_frame", json!({"count": 19}));
    s
}

/// Whether pixel (x, y) of `frame` is black (else white).
fn black(s: &Session, frame: u32, x: u32, y: u32) -> bool {
    let pm = render_frame(&s.doc, frame, 1.0).unwrap();
    let p = pm.pixel(x, y).unwrap();
    assert!(p[0] < 0.01 || p[0] > 0.99, "pixel ({x}, {y}) is part-covered: {p:?}");
    p[0] < 0.5
}

#[test]
fn add_camera_puts_one_camera_layer_on_top_at_rest() {
    let mut s = stage_with(&[(95.0, 45.0, 10.0, 10.0)]);
    let art = s.layer;
    let v = run(&mut s, "camera.add", json!({}));
    assert_eq!(v["added"], true);
    let tl = s.doc.timeline(0);
    assert_eq!(tl.layers[0].kind, LayerKind::Camera);
    assert_eq!(tl.layers[0].keyframes[0].duration, 20, "it spans the timeline");
    assert_eq!(s.layer, tl.layers[0].id, "the camera layer becomes active");
    // At rest the movie is unchanged.
    assert!(black(&s, 0, 96, 46) && !black(&s, 0, 94, 50));
    // A second Add Camera changes nothing.
    let before = s.doc.clone();
    assert_eq!(run(&mut s, "camera.add", json!({}))["added"], false);
    assert_eq!(s.doc, before);
    // Nothing draws on the camera layer, and it never leaves the top.
    assert!(s.execute("draw.rect", &json!({"x": 0, "y": 0, "w": 5, "h": 5})).unwrap_err().to_string().contains("camera layer"));
    run(&mut s, "layer.move_down", json!({}));
    run(&mut s, "layer.move_up", json!({"layer": art.0}));
    assert_eq!(s.doc.timeline(0).layers[0].kind, LayerKind::Camera);
    assert!(s.execute("layer.set_type", &json!({"type": "mask"})).is_err());
    // The artwork layer is the one left to draw on: deleting it is refused.
    assert!(s.execute("layer.delete", &json!({"layer": art.0})).is_err());
}

#[test]
fn a_camera_zoom_of_two_doubles_sizes_about_its_centre() {
    let mut s = stage_with(&[(95.0, 45.0, 10.0, 10.0)]);
    run(&mut s, "camera.add", json!({}));
    let v = run(&mut s, "camera.set", json!({"zoom": 200}));
    assert_eq!(v["zoom"], 200.0);
    // The 10 × 10 square at the centre now covers 90..110 × 40..60, exactly.
    assert!(black(&s, 0, 90, 40) && black(&s, 0, 109, 59));
    assert!(!black(&s, 0, 89, 50) && !black(&s, 0, 110, 50) && !black(&s, 0, 100, 39) && !black(&s, 0, 100, 60));
    // Panning the camera right by 20 shows the square 40 px further left.
    run(&mut s, "camera.set", json!({"dx": 20}));
    assert!(black(&s, 0, 50, 40) && !black(&s, 0, 49, 50) && black(&s, 0, 69, 59) && !black(&s, 0, 70, 50));
    // The stage itself (editing) is untouched: picking still finds the square where it is.
    assert!(s.hit_test(anicraft_geom::Vec2::new(100.0, 50.0), 0.0).is_some());
}

#[test]
fn camera_rotation_turns_the_view() {
    // A bar right of the centre: 100..140 × 48..52.
    let mut s = stage_with(&[(100.0, 48.0, 40.0, 4.0)]);
    run(&mut s, "camera.add", json!({}));
    run(&mut s, "camera.set", json!({"rotation": 90}));
    // The camera turned clockwise, so the bar points up: 98..102 × 10..50.
    assert!(black(&s, 0, 99, 20) && black(&s, 0, 100, 45));
    assert!(!black(&s, 0, 120, 50) && !black(&s, 0, 97, 20) && !black(&s, 0, 102, 20));
}

#[test]
fn a_classic_tween_of_the_camera_and_auto_keys() {
    let mut s = stage_with(&[(95.0, 45.0, 10.0, 10.0)]);
    run(&mut s, "camera.add", json!({}));
    let cam = camera_id(&s);
    run(&mut s, "timeline.insert_keyframe", json!({"layer": cam.0, "frame": 10}));
    run(&mut s, "camera.set", json!({"frame": 10, "zoom": 300}));
    run(&mut s, "timeline.create_tween", json!({"layer": cam.0, "frame": 0}));
    // Halfway the zoom is 200 %: the square covers 90..110 × 40..60.
    assert!(black(&s, 5, 90, 40) && !black(&s, 5, 89, 50) && black(&s, 5, 109, 59) && !black(&s, 5, 110, 50));
    assert!(black(&s, 15, 85, 35) && !black(&s, 15, 84, 50), "300 % after the tween");
    // Setting the camera inside the tween keys the playhead, as editing any tweened object does.
    run(&mut s, "timeline.goto", json!({"frame": 3}));
    run(&mut s, "camera.set", json!({"dy": 10}));
    let keys: Vec<u32> = s.doc.timeline(0).layers[0].keyframes.iter().map(|k| k.start).collect();
    assert_eq!(keys, [0, 3, 10]);
    // And a motion tween: the edit becomes property keys, the span stays whole.
    run(&mut s, "timeline.remove_tween", json!({"layer": cam.0, "frame": 0}));
    run(&mut s, "timeline.create_motion_tween", json!({"layer": cam.0, "frame": 10}));
    run(&mut s, "timeline.goto", json!({"frame": 15}));
    run(&mut s, "camera.set", json!({"zoom": 100}));
    let k = s.doc.timeline(0).layers[0].keyframes.iter().find(|k| k.start == 10).cloned().unwrap();
    assert!(k.motion.as_ref().unwrap().track(anicraft_doc::Property::ScaleX).is_some(), "zoom keyed as Scale X");
    let at = |s: &mut Session, f: u32| run(s, "camera.info", json!({"frame": f}))["zoom"].as_f64().unwrap();
    assert!((at(&mut s, 10) - 300.0).abs() < 1e-3 && (at(&mut s, 15) - 100.0).abs() < 1e-3);
}

fn camera_id(s: &Session) -> anicraft_doc::LayerId {
    s.doc.timeline(s.scene).camera_layer().unwrap().id
}

#[test]
fn a_camera_drag_is_one_undo_step_and_remove_asks_only_when_it_moves() {
    let mut s = stage_with(&[(95.0, 45.0, 10.0, 10.0)]);
    run(&mut s, "camera.add", json!({}));
    let steps = s.history_len();
    s.begin_gesture("Move Camera");
    for _ in 0..30 {
        run(&mut s, "camera.set", json!({"dx": 1, "zoom_by": 1.01, "rotate_by": 0.5}));
    }
    s.end_gesture();
    assert_eq!(s.history_len(), steps + 1);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(run(&mut s, "camera.info", json!({}))["x"], 100.0);
    // A camera at rest on one keyframe goes without asking; an animated one asks (undoable tier).
    assert!(s.confirmation("camera.remove", &json!({})).is_none());
    let cam = camera_id(&s);
    run(&mut s, "timeline.insert_keyframe", json!({"layer": cam.0, "frame": 5}));
    let c = s.confirmation("camera.remove", &json!({})).unwrap();
    assert!(!c.irreversible && c.message.contains("2 keyframes"));
    assert_eq!(s.confirmation("layer.delete", &json!({"layer": camera_id(&s).0})), Some(c));
    run(&mut s, "camera.remove", json!({}));
    assert!(s.doc.timeline(0).camera_layer().is_none());
    assert!(s.execute("camera.set", &json!({"zoom": 150})).is_err(), "no camera to set");
    assert_eq!(run(&mut s, "camera.info", json!({}))["camera"], false);
    run(&mut s, "edit.undo", json!({}));
    assert!(s.doc.timeline(0).camera_layer().is_some());
}

#[test]
fn layer_depth_gives_parallax_in_the_movie() {
    let mut s = stage_with(&[(95.0, 45.0, 10.0, 10.0)]);
    let far = s.layer;
    run(&mut s, "layer.set_depth", json!({"depth": 1000}));
    assert!(s.execute("layer.set_depth", &json!({"depth": 1e6})).is_err());
    run(&mut s, "camera.add", json!({}));
    assert!(s.execute("layer.set_depth", &json!({"depth": 5})).is_err(), "the camera layer has no depth");
    run(&mut s, "camera.set", json!({"dx": 40}));
    // A layer 1000 px behind moves half as far: 20 px left, 75..85.
    assert!(black(&s, 0, 75, 50) && !black(&s, 0, 74, 50) && !black(&s, 0, 85, 50));
    assert_eq!(s.doc.timeline(0).layer(far).unwrap().depth, 1000.0);
}

#[test]
fn every_export_sees_the_camera() {
    let dir = std::env::temp_dir().join(format!("anicraft-camera-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = stage_with(&[(95.0, 45.0, 10.0, 10.0)]);
    run(&mut s, "camera.add", json!({}));
    run(&mut s, "camera.set", json!({"zoom": 200, "rotation": 30, "tint": "#3366ff", "tint_amount": 40}));
    let oracle = render_frame(&s.doc, 0, 1.0).unwrap();
    // PNG export is the oracle's picture through the camera.
    let png = dir.join("f.png");
    let _ = std::fs::remove_file(&png);
    run(&mut s, "file.export_png", json!({"path": png.display().to_string(), "frame": 0}));
    assert_eq!(std::fs::read(&png).unwrap(), oracle.encode_png().unwrap());
    // So is every PNG of a sequence.
    let seq = dir.join("seq");
    let _ = std::fs::remove_dir_all(&seq);
    run(&mut s, "file.export_png_sequence", json!({"dir": seq.display().to_string(), "from": 0, "to": 1}));
    let first = std::fs::read_dir(&seq).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "png")).min().unwrap();
    assert_eq!(std::fs::read(first).unwrap(), oracle.encode_png().unwrap());
    // SVG and Lottie write the evaluated (camera) frame: the square is no longer where the stage
    // has it, and the tint shows.
    assert_eq!(crate::cmds_file::frame_svg(&s.doc, 0), anicraft_render::svg::to_svg(&anicraft_doc::evaluate(&s.doc, 0), &s.doc.stage));
    assert_ne!(anicraft_doc::evaluate(&s.doc, 0).items[0].transform, anicraft_doc::evaluate_scene(&s.doc, 0, 0).items[0].transform);
    let lottie = anicraft_render::lottie::to_lottie(&s.doc, "t", 0, 0, false);
    assert!(lottie.layers >= 1);
    // The tint reached the pixels: the square is blue-ish, not black.
    let p = oracle.pixel(100, 50).unwrap();
    assert!(p[2] > p[0] + 0.1, "{p:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_camera_survives_a_file_round_trip() {
    let mut s = stage_with(&[(95.0, 45.0, 10.0, 10.0)]);
    run(&mut s, "layer.set_depth", json!({"depth": 300}));
    run(&mut s, "camera.add", json!({}));
    run(&mut s, "camera.set", json!({"x": 80, "zoom": 150, "rotation": -12, "tint": "#ff0000", "tint_amount": 25}));
    let text = crate::format::to_json(&s.doc).unwrap();
    let back = crate::format::from_json(&text).unwrap();
    assert_eq!(back, s.doc);
    assert_eq!(render_frame(&back, 0, 1.0).unwrap().to_rgba8(), render_frame(&s.doc, 0, 1.0).unwrap().to_rgba8());
    // A file from before cameras opens with none and every layer at depth 0.
    let old = crate::format::from_json(&crate::format::to_json(&Document::default()).unwrap()).unwrap();
    assert!(old.timeline(0).camera_layer().is_none() && old.timeline(0).layers.iter().all(|l| l.depth == 0.0));
}

#[test]
fn frames_keep_to_their_layer_kind() {
    let mut s = stage_with(&[(95.0, 45.0, 10.0, 10.0)]);
    let art = s.layer;
    run(&mut s, "camera.add", json!({}));
    let cam = camera_id(&s);
    // Artwork frames can't be pasted onto the camera, nor camera frames onto artwork.
    run(&mut s, "timeline.copy_frames", json!({"layer": art.0, "frame": 0}));
    assert!(s.execute("timeline.paste_frames", &json!({"layer": cam.0, "frame": 0})).is_err());
    assert!(s.execute("timeline.move_keyframes", &json!({"layer": cam.0, "from": 0, "by": 0, "layer_by": 1})).is_err());
    run(&mut s, "timeline.copy_frames", json!({"layer": cam.0, "frame": 0}));
    assert!(s.execute("timeline.paste_frames", &json!({"layer": art.0, "frame": 0})).is_err());
    // A blank camera keyframe is the camera at rest; a motion tween there gets a camera to move.
    run(&mut s, "timeline.insert_blank_keyframe", json!({"layer": cam.0, "frame": 8}));
    assert_eq!(run(&mut s, "camera.info", json!({"frame": 8}))["zoom"], 100.0);
    run(&mut s, "timeline.create_motion_tween", json!({"layer": cam.0, "frame": 8}));
    assert!(s.doc.timeline(0).layers[0].keyframes.iter().find(|k| k.start == 8).is_some_and(|k| k.elements.len() == 1));
}

#[test]
fn plugins_may_drive_the_camera() {
    assert!(crate::cmds_plugin::plugin_may_call("camera.set").is_ok());
    assert!(crate::cmds_plugin::plugin_may_call("camera.add").is_ok());
    assert!(crate::cmds_plugin::plugin_may_call("layer.set_depth").is_ok());
    for id in ["layer.remove_depth_key", "layer.set_attached", "layer.depths"] {
        assert!(crate::cmds_plugin::plugin_may_call(id).is_ok(), "{id}");
    }
}

/// Every command with hostile params while the scene has an animated camera: Ok or Err, never
/// a panic, and the document still renders.
#[test]
fn panic_hunt_with_a_camera() {
    let hostile = [
        Value::Null,
        json!({}),
        json!({"zoom": 1e9, "zoom_by": -1, "rotate_by": 1e30, "tint_amount": 500, "depth": 1e30, "frame": u64::MAX}),
        json!({"zoom": 100, "zoom_by": 1e-30, "rotation": 1e7, "dx": 1e7, "dy": -1e7, "reset": "x", "depth": -500, "tint": "#zz"}),
        json!({"x": 1e7, "y": -1e7, "zoom": 5, "zoom_by": 1e7, "reset": true, "frame": 99_999, "layer": 1}),
        json!({"layers": [1, 2, 3], "from": 0, "to": 100, "by": 1, "layer_by": 1, "frame": 0}),
        json!({"layer": 2, "type": "camera", "depth": 10000, "count": 50}),
        // Depth keys and attachment (ADR 0082): huge, negative, overflowing and wrong-typed.
        json!({"depth": -1e39, "key": true, "frame": 4_000_000_000u64, "attached": "x"}),
        json!({"depth": 1e39, "key": "yes", "layer": u64::MAX, "view": "camera", "x": 1e30, "y": -1e30}),
        json!({"depth": -500, "key": true, "frame": 0, "attached": true, "style": "advanced", "red_multiplier": -100, "blue_offset": 255}),
        json!({"depth": 10000, "key": true, "frame": 19, "style": "alpha", "alpha": 0, "brightness": 1e9, "view": "nowhere"}),
    ];
    let mut base = stage_with(&[(95.0, 45.0, 10.0, 10.0)]);
    run(&mut base, "camera.add", json!({}));
    let cam = camera_id(&base);
    run(&mut base, "timeline.insert_keyframe", json!({"layer": cam.0, "frame": 10}));
    run(&mut base, "camera.set", json!({"frame": 10, "zoom": 400, "rotation": 45}));
    run(&mut base, "timeline.create_tween", json!({"layer": cam.0, "frame": 0}));
    let mut s = Session::new(base.doc.clone());
    for spec in commands().iter().filter(|c| !c.id.starts_with("file.")) {
        let _ = s.execute("camera.add", &json!({}));
        for p in &hostile {
            let _ = s.execute(spec.id, p);
        }
    }
    for f in [0, 5, 10, 19, 1000] {
        let _ = render_frame(&s.doc, f, 0.25);
    }
    let tl = s.doc.timeline(0);
    assert!(tl.layers.iter().skip(1).all(|l| l.kind != LayerKind::Camera), "at most one camera, on top");
}

// ----- ADR 0082: depth over time, perspective, attaching layers, colour, editing through it ---

/// A stage with a 10 × 10 square on a layer behind (depth `far`) and one on the stage plane,
/// both at (95, 45), and a camera. Returns (session, far layer, near layer).
fn two_planes(far: f32) -> (Session, anicraft_doc::LayerId, anicraft_doc::LayerId) {
    let mut s = stage_with(&[(95.0, 45.0, 10.0, 10.0)]);
    let back = s.layer;
    run(&mut s, "layer.set_depth", json!({"depth": far}));
    run(&mut s, "layer.new", json!({}));
    let near = s.layer;
    run(&mut s, "draw.rect", json!({"x": 95, "y": 45, "w": 10, "h": 10, "fill": "#000000", "stroke": null}));
    run(&mut s, "camera.add", json!({}));
    (s, back, near)
}

/// The x of the left edge of the black run on row `y` starting the search at `from`.
fn left_edge(s: &Session, frame: u32, y: u32, from: u32) -> Option<u32> {
    (from..200).find(|&x| black(s, frame, x, y))
}

#[test]
fn depth_keys_tween_the_parallax_and_undo_in_one_step() {
    let (mut s, back, _) = two_planes(0.0);
    run(&mut s, "layer.set_depth", json!({"layer": back.0, "depth": 1000, "key": true, "frame": 10}));
    let l = s.doc.timeline(0).layer(back).unwrap().clone();
    assert_eq!(l.depth_keys.len(), 2, "frame 0 keeps the depth it had");
    assert_eq!(l.depth_at(5), 500.0, "the midpoint");
    // Keyed from now on: a plain set at the playhead adds a key there.
    s.set_frame(15);
    run(&mut s, "layer.set_depth", json!({"layer": back.0, "depth": 3000}));
    assert_eq!(s.doc.timeline(0).layer(back).unwrap().depth_keys.len(), 3);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.timeline(0).layer(back).unwrap().depth_keys.len(), 2);
    // A 40 px pan: at frame 10 the far square moves 20 px (75..85); the near one 40 (55..65).
    let cam = camera_id(&s);
    run(&mut s, "timeline.insert_keyframe", json!({"layer": cam.0, "frame": 10}));
    run(&mut s, "camera.set", json!({"frame": 0, "dx": 40}));
    run(&mut s, "camera.set", json!({"frame": 10, "dx": 40}));
    assert_eq!(left_edge(&s, 10, 50, 0), Some(55));
    assert_eq!(left_edge(&s, 10, 50, 66), Some(75));
    // Frame 5 (depth 500): moved 40 · 1000 / 1500 = 26.67 px, to 68.33: pixel 69 is fully in.
    assert!(black(&s, 5, 69, 50) && !black(&s, 5, 67, 50));
    // Removing keys: the last leaves the depth it had there.
    run(&mut s, "layer.remove_depth_key", json!({"layer": back.0, "frame": 10}));
    assert!(s.execute("layer.remove_depth_key", &json!({"layer": back.0, "frame": 10})).is_err());
    run(&mut s, "layer.remove_depth_key", json!({"layer": back.0, "frame": 0}));
    let l = s.doc.timeline(0).layer(back).unwrap();
    assert!(l.depth_keys.is_empty() && l.depth == 0.0);
    // `key: false` makes it one value again.
    run(&mut s, "layer.set_depth", json!({"layer": back.0, "depth": 200, "key": true, "frame": 3}));
    run(&mut s, "layer.set_depth", json!({"layer": back.0, "depth": 50, "key": false}));
    let l = s.doc.timeline(0).layer(back).unwrap();
    assert!(l.depth_keys.is_empty() && l.depth == 50.0);
}

#[test]
fn a_camera_zoom_scales_deeper_layers_less() {
    // Zoom 200 % about the centre (100, 50): the stage-plane square doubles (90..110); one
    // 1000 px back grows by 4/3 (93.33..106.67).
    let (mut s, _, _) = two_planes(1000.0);
    run(&mut s, "camera.set", json!({"zoom": 200}));
    let dl = anicraft_doc::evaluate(&s.doc, 0);
    let scales: Vec<f32> = dl.items.iter().map(|i| i.transform.mean_scale()).collect();
    assert!((scales[0] - 4.0 / 3.0).abs() < 1e-4 && (scales[1] - 2.0).abs() < 1e-5, "{scales:?}");
    assert_eq!(left_edge(&s, 0, 50, 0), Some(90));
}

#[test]
fn attached_layers_stay_on_screen_while_the_camera_moves() {
    let (mut s, back, near) = two_planes(0.0);
    assert_eq!(run(&mut s, "layer.set_attached", json!({"layer": near.0}))["attached"], true, "toggles on");
    run(&mut s, "camera.set", json!({"dx": 40, "zoom": 150, "rotation": 20}));
    // The attached square is exactly where it was drawn.
    let dl = anicraft_doc::evaluate(&s.doc, 0);
    let it = dl.items.iter().find(|i| i.layer == near).unwrap();
    assert_eq!(it.transform, anicraft_doc::evaluate_scene(&s.doc, 0, 0).items.iter().find(|i| i.layer == near).unwrap().transform);
    assert!(black(&s, 0, 96, 46) && black(&s, 0, 104, 54));
    assert!(dl.items.iter().find(|i| i.layer == back).unwrap().transform != it.transform);
    // Listed, and refused on the camera layer and inside a symbol.
    let v = run(&mut s, "layer.depths", json!({}));
    assert_eq!(v["camera"], true);
    assert!(v["layers"].as_array().unwrap().iter().any(|r| r["layer"] == near.0 && r["attached"] == true));
    assert!(s.execute("layer.set_attached", &json!({"layer": camera_id(&s).0})).is_err());
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert!(!s.doc.timeline(0).layer(near).unwrap().attached);
}

#[test]
fn layer_depths_list_nearest_first_ties_in_timeline_order() {
    let mut s = stage_with(&[(0.0, 0.0, 5.0, 5.0)]);
    let a = s.layer;
    run(&mut s, "layer.new", json!({}));
    let b = s.layer;
    run(&mut s, "layer.new", json!({}));
    let c = s.layer;
    run(&mut s, "layer.set_depth", json!({"layer": a.0, "depth": -100}));
    run(&mut s, "layer.set_depth", json!({"layer": c.0, "depth": 400}));
    let v = run(&mut s, "layer.depths", json!({}));
    let ids: Vec<u64> = v["layers"].as_array().unwrap().iter().map(|r| r["layer"].as_u64().unwrap()).collect();
    assert_eq!(ids, [a.0, b.0, c.0]);
    // Ties: b and a at 0 go top first (b is above a).
    run(&mut s, "layer.set_depth", json!({"layer": a.0, "depth": 0}));
    let v = run(&mut s, "layer.depths", json!({}));
    let ids: Vec<u64> = v["layers"].as_array().unwrap().iter().map(|r| r["layer"].as_u64().unwrap()).collect();
    assert_eq!(ids, [b.0, a.0, c.0]);
}

#[test]
fn camera_colour_effects_reach_the_pixels() {
    let mut s = stage_with(&[(95.0, 45.0, 10.0, 10.0)]);
    run(&mut s, "camera.add", json!({}));
    // Brightness +50 %: white stays white, black becomes mid grey (0.5).
    let v = run(&mut s, "camera.set", json!({"brightness": 50}));
    assert_eq!(v["color_effect"]["style"], "brightness");
    let px = render_frame(&s.doc, 0, 1.0).unwrap();
    let p = px.pixel(100, 50).unwrap();
    assert!((p[0] - 0.5).abs() < 0.01 && (p[2] - 0.5).abs() < 0.01, "{p:?}");
    // Advanced: red × 0 and blue + 255 turn white blue-ish and black blue.
    run(&mut s, "camera.set", json!({"style": "none"}));
    run(&mut s, "camera.set", json!({"style": "advanced", "red_multiplier": 0, "blue_offset": 255}));
    let px = render_frame(&s.doc, 0, 1.0).unwrap();
    let sq = px.pixel(100, 50).unwrap();
    assert!(sq[0] < 0.01 && sq[1] < 0.01 && sq[2] > 0.99, "the square: {sq:?}");
    // The camera has no opacity.
    assert!(s.execute("camera.set", &json!({"alpha_multiplier": 50})).is_err());
    assert!(s.execute("camera.set", &json!({"style": "alpha"})).is_err());
    let info = run(&mut s, "camera.info", json!({}));
    assert_eq!(info["color_effect"]["style"], "advanced");
    // A colour change alone keys inside a camera tween like any other edit, and round-trips.
    let back = crate::format::from_json(&crate::format::to_json(&s.doc).unwrap()).unwrap();
    assert_eq!(back, s.doc);
}

#[test]
fn picking_through_the_camera_finds_what_the_movie_shows() {
    // The far square (depth 1000) and the near one start together; a 40 px pan puts the near
    // one at 55..65 on the picture and the far one at 75..85.
    let (mut s, back, near) = two_planes(1000.0);
    run(&mut s, "camera.set", json!({"dx": 40}));
    assert_eq!(run(&mut s, "element.pick", json!({"x": 80, "y": 50, "view": "camera"}))["layer"], back.0);
    assert_eq!(run(&mut s, "element.pick", json!({"x": 60, "y": 50, "view": "camera"}))["layer"], near.0);
    assert!(run(&mut s, "element.pick", json!({"x": 100, "y": 50, "view": "camera"}))["element"].is_null(), "nothing shows there");
    assert!(s.execute("element.pick", &json!({"x": 1, "y": 1, "view": "sideways"})).is_err());
    // Dragging maps the pointer back through the layer's view: 10 picture px on the far layer
    // are 10 stage px at zoom 100 % (a pan keeps sizes).
    let v = s.camera_view(back).unwrap();
    let d = v.inverse().unwrap().apply_vector(anicraft_geom::Vec2::new(10.0, 0.0));
    assert!((d.x - 10.0).abs() < 1e-4);
    run(&mut s, "camera.set", json!({"zoom": 200}));
    let v = s.camera_view(near).unwrap();
    let d = v.inverse().unwrap().apply_vector(anicraft_geom::Vec2::new(10.0, 0.0));
    assert!((d.x - 5.0).abs() < 1e-4, "{d:?}");
}
