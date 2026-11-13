//! Retiming through the command registry (ADR 0083): property keys moved in time, span
//! stretching, whole spans moved and copied, Split and Join Motion.

use super::*;
use anicraft_doc::Property;
use serde_json::json;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    match s.execute(id, &p) {
        Ok(v) => v,
        Err(e) => panic!("{id} {p}: {e}"),
    }
}

/// A square at (5, 5) with a 24-frame motion tween: X keyed 5 → 105 at frame 12, and Y
/// 5 → 55 at frame 23.
fn tweened() -> Session {
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 10, "h": 10}));
    run(&mut s, "timeline.create_motion_tween", json!({}));
    s.set_frame(12);
    run(&mut s, "element.translate", json!({"dx": 100, "dy": 0}));
    s.set_frame(23);
    run(&mut s, "element.translate", json!({"dx": 0, "dy": 50}));
    s
}

fn pos(s: &mut Session, frame: u32) -> (f32, f32) {
    s.set_frame(frame);
    let t = s.visible_elements().first().map(|e| e.transform.translation()).unwrap_or_default();
    (t.x, t.y)
}

fn keys(s: &Session, p: Property) -> Vec<(u32, f32)> {
    let l = s.active_layer().expect("layer");
    let k = l.keyframes.first().expect("key");
    k.motion.as_ref().and_then(|m| m.track(p)).map(|t| t.keys.iter().map(|k| (k.frame, k.value)).collect()).unwrap_or_default()
}

fn spans(s: &Session) -> Vec<(u32, u32, bool)> {
    s.active_layer().expect("layer").keyframes.iter().map(|k| (k.start, k.duration, k.motion.is_some())).collect()
}

#[test]
fn a_moved_key_keeps_its_value_at_its_new_frame() {
    let mut s = tweened();
    let steps = s.history_len();
    let v = run(&mut s, "motion.move_keys", json!({"keys": [{"property": "x", "frame": 12}], "by": 4}));
    assert_eq!(v["by"], 4);
    assert_eq!(v["keys"], json!([{"property": "x", "frame": 16}]));
    assert_eq!(keys(&s, Property::X), vec![(0, 5.0), (16, 105.0)]);
    assert_eq!(pos(&mut s, 16).0, 105.0);
    assert_eq!(pos(&mut s, 8).0, 55.0, "halfway to the key that is now at 16");
    assert_eq!(s.history_len(), steps + 1);
    // Past the end of the span or a neighbour it stops: never crosses or lands on one.
    let v = run(&mut s, "motion.move_keys", json!({"keys": [{"property": "x", "frame": 16}], "by": 100}));
    assert_eq!(v["by"], 7, "the span's last frame is 23");
    let v = run(&mut s, "motion.move_keys", json!({"keys": [{"property": "x", "frame": 23}], "by": -100}));
    assert_eq!(v["by"], -22, "the key at frame 0 stays first");
    // A key that isn't there is an error, not a silent no-op.
    assert!(s.execute("motion.move_keys", &json!({"keys": [{"property": "y", "frame": 5}], "by": 1})).is_err());
    assert!(s.execute("motion.move_keys", &json!({"keys": [], "by": 1})).is_err());
}

#[test]
fn a_drag_is_one_undo_step() {
    let mut s = tweened();
    let steps = s.history_len();
    s.begin_gesture("Move property keyframes");
    for _ in 0..5 {
        run(&mut s, "motion.move_keys", json!({"keys": [{"property": "x", "frame": 12}, {"property": "y", "frame": 23}], "by": 0}));
    }
    let mut at = (12, 23);
    for _ in 0..3 {
        let v = run(&mut s, "motion.move_keys", json!({"keys": [{"property": "x", "frame": at.0}, {"property": "y", "frame": at.1}], "by": -1}));
        at = (v["keys"][0]["frame"].as_u64().unwrap_or(0) as u32, v["keys"][1]["frame"].as_u64().unwrap_or(0) as u32);
    }
    s.end_gesture();
    assert_eq!(at, (9, 20));
    assert_eq!(s.history_len(), steps + 1);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(keys(&s, Property::X), vec![(0, 5.0), (12, 105.0)]);
}

#[test]
fn stretching_a_span_scales_its_keys_in_proportion() {
    let mut s = tweened();
    run(&mut s, "timeline.insert_blank_keyframe", json!({"frame": 24}));
    run(&mut s, "timeline.insert_frame", json!({"frame": 24, "count": 4}));
    let before: Vec<(f32, f32)> = (0..24).map(|f| pos(&mut s, f)).collect();
    // 24 frames (0..=23) to 47 (0..=46): every key's offset doubles.
    let v = run(&mut s, "timeline.resize_span", json!({"frame": 0, "frames": 47}));
    assert_eq!(v["scaled"], true);
    assert_eq!(keys(&s, Property::X), vec![(0, 5.0), (24, 105.0)]);
    assert_eq!(keys(&s, Property::Y), vec![(0, 5.0), (46, 55.0)]);
    for (f, want) in before.iter().enumerate() {
        assert_eq!(pos(&mut s, 2 * f as u32), *want, "frame {}", 2 * f);
    }
    // Later frames moved with the end.
    assert_eq!(spans(&s), vec![(0, 47, true), (47, 5, false)]);
    // Shorter again, and back where it was.
    run(&mut s, "timeline.resize_span", json!({"frame": 0, "frames": 24}));
    assert_eq!(keys(&s, Property::X), vec![(0, 5.0), (12, 105.0)]);
    assert_eq!(spans(&s), vec![(0, 24, true), (24, 5, false)]);
    // Keeping the keys where they are (Shift-drag).
    let v = run(&mut s, "timeline.resize_span", json!({"frame": 0, "frames": 30, "scale_keys": false}));
    assert_eq!(v["scaled"], false);
    assert_eq!(keys(&s, Property::X), vec![(0, 5.0), (12, 105.0)]);
    assert!(s.execute("timeline.resize_span", &json!({"frame": 0, "frames": 0})).is_err());
}

#[test]
fn classic_and_shape_tween_spans_stretch_too() {
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 10, "h": 10}));
    run(&mut s, "timeline.insert_keyframe", json!({"frame": 10}));
    run(&mut s, "timeline.create_tween", json!({"frame": 0}));
    s.set_frame(10);
    run(&mut s, "element.translate", json!({"dx": 100, "dy": 0}));
    run(&mut s, "timeline.resize_span", json!({"frame": 0, "frames": 20}));
    assert_eq!(spans(&s), vec![(0, 20, false), (20, 1, false)]);
    assert_eq!(pos(&mut s, 10).0, 55.0, "halfway through the longer tween");
    run(&mut s, "timeline.create_shape_tween", json!({"frame": 0}));
    run(&mut s, "timeline.resize_span", json!({"frame": 0, "frames": 5}));
    assert_eq!(spans(&s), vec![(0, 5, false), (5, 1, false)]);
}

#[test]
fn a_whole_span_moves_and_alt_drag_copies_it() {
    let mut s = tweened();
    let before: Vec<(f32, f32)> = (0..24).map(|f| pos(&mut s, f)).collect();
    run(&mut s, "timeline.move_spans", json!({"from": 0, "to": 23, "by": 10}));
    // It keeps its length and its motion; the frames it left are empty.
    assert_eq!(spans(&s), vec![(0, 10, false), (10, 24, true)]);
    for (f, want) in before.iter().enumerate() {
        assert_eq!(pos(&mut s, 10 + f as u32), *want);
    }
    run(&mut s, "timeline.move_spans", json!({"from": 10, "to": 33, "by": 30, "copy": true}));
    // Past the layer's end: the frames between are a gap (empty).
    assert_eq!(spans(&s), vec![(0, 10, false), (10, 24, true), (40, 24, true)]);
    for (f, want) in before.iter().enumerate() {
        assert_eq!(pos(&mut s, 40 + f as u32), *want);
    }
    assert!(s.execute("timeline.move_spans", &json!({"from": 10, "to": 33, "by": 0, "copy": true})).is_err());
}

#[test]
fn split_then_join_motion_is_a_round_trip() {
    let mut s = tweened();
    let before: Vec<(f32, f32)> = (0..24).map(|f| pos(&mut s, f)).collect();
    let original = s.active_layer().map(|l| l.keyframes.clone()).unwrap_or_default();
    let v = run(&mut s, "motion.split", json!({"frame": 8}));
    assert_eq!(v, json!({"first": 0, "second": 8}));
    assert_eq!(spans(&s), vec![(0, 8, true), (8, 16, true)]);
    for (f, want) in before.iter().enumerate() {
        let (x, y) = pos(&mut s, f as u32);
        assert!((x - want.0).abs() < 1e-3 && (y - want.1).abs() < 1e-3, "split, frame {f}: {x},{y} vs {want:?}");
    }
    let v = run(&mut s, "motion.join", json!({"frame": 0}));
    assert_eq!(v["frames"], 24);
    let now = s.active_layer().map(|l| l.keyframes.clone()).unwrap_or_default();
    assert_eq!(now, original, "the very tween comes back");
    // On the first frame there is nothing to split; elsewhere than a motion tween neither.
    assert!(s.execute("motion.split", &json!({"frame": 0})).is_err());
    assert!(s.execute("motion.join", &json!({"frame": 0})).is_err(), "nothing after it to join");
}

#[test]
fn joining_two_tweens_of_one_object_keeps_every_frame() {
    let mut s = tweened();
    run(&mut s, "motion.split", json!({"frame": 8}));
    // The second half changes on its own, so it is no longer the first's continuation.
    s.set_frame(20);
    run(&mut s, "modify.rotate_cw", json!({}));
    let before: Vec<(f32, f32)> = (0..24).map(|f| pos(&mut s, f)).collect();
    let turned: Vec<f32> = (0..24).map(|f| {
        s.set_frame(f);
        s.visible_elements().first().map_or(0.0, |e| e.transform.decompose().rotation)
    }).collect();
    run(&mut s, "motion.join", json!({"frame": 3}));
    assert_eq!(spans(&s), vec![(0, 24, true)]);
    for f in 0..24u32 {
        let (x, y) = pos(&mut s, f);
        let want = before[f as usize];
        assert!((x - want.0).abs() < 1e-2 && (y - want.1).abs() < 1e-2, "frame {f}: {x},{y} vs {want:?}");
        let r = s.visible_elements().first().map_or(0.0, |e| e.transform.decompose().rotation);
        assert!((r - turned[f as usize]).abs() < 1e-3, "frame {f}: {r} vs {}", turned[f as usize]);
    }
}

#[test]
fn join_motion_refuses_what_it_cannot_keep() {
    let mut s = tweened();
    run(&mut s, "motion.split", json!({"frame": 8}));
    // Actions on the second key would be dropped.
    let k = s.layer_mut(None).ok().and_then(|l| l.keyframe_mut(1)).map(|k| {
        k.script = Some(anicraft_doc::FrameScript::Actions(Vec::new()));
    });
    assert!(k.is_some());
    assert!(s.execute("motion.join", &json!({"frame": 0})).is_err());
}

#[test]
fn retimed_tweens_survive_the_file_round_trip() {
    let mut s = tweened();
    run(&mut s, "motion.move_keys", json!({"keys": [{"property": "x", "frame": 12}], "by": 3}));
    run(&mut s, "timeline.resize_span", json!({"frame": 0, "frames": 40}));
    let text = crate::format::to_json(&s.doc).expect("save");
    let doc = crate::format::from_json(&text).expect("load");
    assert_eq!(doc.scenes, s.doc.scenes);
}
