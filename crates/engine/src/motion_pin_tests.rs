//! Motion path anchors pinned to frames (ADR 0049) through the command registry: auto-key
//! pinning where the object is put, `motion.pin_point` / `motion.unpin_point`, the X/Y keys of a
//! pinned frame as its anchor's place, point edits keeping pins on their anchors, F6, undo and
//! the file round trip.

use super::*;
use anicraft_doc::{Pin, Property};
use anicraft_geom::Vec2;
use anicraft_geom::edit::ANCHOR;
use serde_json::json;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    match s.execute(id, &p) {
        Ok(v) => v,
        Err(e) => panic!("{id} {p}: {e}"),
    }
}

fn fails(s: &mut Session, id: &str, p: Value) -> String {
    match s.execute(id, &p) {
        Ok(v) => panic!("{id} {p} should fail, gave {v}"),
        Err(e) => e.to_string(),
    }
}

/// A 10 × 10 square centred on (5, 5) with a motion tween over frames 0..24 travelling the hump
/// from (5, 5) up and over to (105, 5), reached on frame 23.
fn on_hump() -> Session {
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 10, "h": 10}));
    run(&mut s, "timeline.create_motion_tween", json!({}));
    s.set_frame(23);
    run(&mut s, "element.translate", json!({"dx": 100, "dy": 0}));
    s.set_frame(0);
    run(&mut s, "motion.set_path", json!({"d": "M5 5 C38 -60 72 -60 105 5"}));
    s
}

fn pos(s: &mut Session, frame: u32) -> Vec2 {
    s.set_frame(frame);
    s.visible_elements().first().map(|e| e.transform.translation()).unwrap_or_default()
}

fn motion(s: &Session) -> &anicraft_doc::MotionTween {
    s.active_layer().and_then(|l| l.keyframes.first()).and_then(|k| k.motion.as_ref()).expect("motion tween")
}

fn anchor(s: &Session, i: usize) -> Vec2 {
    motion(s).path.as_ref().and_then(|p| p.point(i, ANCHOR)).expect("anchor")
}

fn key_frames(s: &Session, p: Property) -> Vec<u32> {
    motion(s).track(p).map(|t| t.keys.iter().map(|k| k.frame).collect()).unwrap_or_default()
}

#[test]
fn moving_the_object_at_a_middle_frame_pins_an_anchor_there() {
    let mut s = on_hump();
    let was = pos(&mut s, 9);
    // A drag on the stage: one undo step however many moves, one pin.
    let steps = s.history_len();
    s.set_frame(9);
    s.begin_gesture("Move");
    for _ in 0..4 {
        run(&mut s, "element.translate", json!({"dx": 5, "dy": 10}));
    }
    s.end_gesture();
    assert_eq!(s.history_len(), steps + 1);
    let want = was + Vec2::new(20.0, 40.0);
    assert!(pos(&mut s, 9).distance(want) < 1e-3, "{:?} vs {want:?}", pos(&mut s, 9));
    assert!(anchor(&s, 1).distance(want) < 1e-3, "the new anchor is where the object was put");
    assert_eq!(motion(&s).pins, vec![Pin { frame: 9, anchor: 1 }]);
    assert!(pos(&mut s, 0).distance(Vec2::new(5.0, 5.0)) < 1e-3 && pos(&mut s, 23).distance(Vec2::new(105.0, 5.0)) < 1e-3, "the ends stay");
    // motion.info reports every anchor tied to a frame, ends included.
    let info = run(&mut s, "motion.info", json!({}));
    let pins: Vec<(u64, u64, bool)> = info["pins"].as_array().unwrap().iter().map(|p| (p["anchor"].as_u64().unwrap(), p["frame"].as_u64().unwrap(), p["end"].as_bool().unwrap())).collect();
    assert_eq!(pins, vec![(0, 0, true), (1, 9, false), (2, 23, true)]);
    // A later frame pins a later anchor; the first pin holds.
    s.set_frame(16);
    run(&mut s, "element.translate", json!({"dx": 0, "dy": -30}));
    assert_eq!(motion(&s).pins, vec![Pin { frame: 9, anchor: 1 }, Pin { frame: 16, anchor: 2 }]);
    assert!(pos(&mut s, 9).distance(want) < 1e-3);
    assert!(pos(&mut s, 16).distance(anchor(&s, 2)) < 1e-3);
    // Undo takes the pin back with its anchor and keys.
    s.undo().expect("undo");
    assert_eq!(motion(&s).pins, vec![Pin { frame: 9, anchor: 1 }]);
    assert_eq!(key_frames(&s, Property::X), vec![0, 9, 23]);
}

#[test]
fn moving_after_the_last_position_key_carries_the_path_on() {
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 10, "h": 10}));
    run(&mut s, "timeline.create_motion_tween", json!({}));
    s.set_frame(12);
    run(&mut s, "element.translate", json!({"dx": 60, "dy": 0}));
    run(&mut s, "motion.set_path", json!({"d": "M5 5 C25 -30 45 -30 65 5"}));
    let before: Vec<Vec2> = (0..=12).map(|f| pos(&mut s, f)).collect();
    s.set_frame(20);
    run(&mut s, "element.translate", json!({"dx": 40, "dy": 40}));
    assert!(pos(&mut s, 20).distance(Vec2::new(105.0, 45.0)) < 1e-3);
    assert_eq!(motion(&s).position_range(), Some((0, 20)));
    assert_eq!(motion(&s).pins, vec![Pin { frame: 12, anchor: 1 }], "the old end stays pinned to its frame");
    for (f, b) in before.iter().enumerate() {
        assert!(pos(&mut s, f as u32).distance(*b) < 1e-2, "frame {f} as it was");
    }
}

#[test]
fn pin_and_unpin_an_anchor_by_command() {
    let mut s = on_hump();
    let v = run(&mut s, "motion.add_point", json!({"segment": 1, "t": 0.5}));
    assert_eq!(v["anchor"], 1);
    let unpinned = motion(&s).clone();
    let off = pos(&mut s, 6);
    assert!(off.distance(anchor(&s, 1)) > 5.0, "an anchor alone is reached when the timing says");
    let steps = s.history_len();
    let v = run(&mut s, "motion.pin_point", json!({"anchor": 1, "frame": 6}));
    assert_eq!(s.history_len(), steps + 1);
    assert_eq!((v["anchor"].as_u64(), v["frame"].as_u64()), (Some(1), Some(6)));
    assert_eq!(v["pins"].as_array().map(Vec::len), Some(3));
    assert!(pos(&mut s, 6).distance(anchor(&s, 1)) < 1e-3, "passed exactly on its frame");
    assert_eq!(key_frames(&s, Property::Y), vec![0, 6, 23]);
    // What can't be pinned is refused with the reason.
    assert!(fails(&mut s, "motion.pin_point", json!({"anchor": 0, "frame": 6})).contains("always pinned"));
    assert!(fails(&mut s, "motion.pin_point", json!({"anchor": 7, "frame": 6})).contains("no such anchor"));
    assert!(fails(&mut s, "motion.unpin_point", json!({"anchor": 2})).contains("always pinned"));
    assert!(fails(&mut s, "motion.unpin_point", json!({"frame": 3})).contains("no anchor is pinned to frame 4"));
    // Unpinning (by anchor or by frame) gives the tween back as it was.
    run(&mut s, "motion.unpin_point", json!({"anchor": 1}));
    assert_eq!(*motion(&s), unpinned);
    assert!(pos(&mut s, 6).distance(off) < 1e-3, "ADR 0038's timing again");
    run(&mut s, "motion.pin_point", json!({"anchor": 1, "frame": 6}));
    run(&mut s, "motion.unpin_point", json!({"frame": 6}));
    assert_eq!(*motion(&s), unpinned);
}

#[test]
fn pin_point_without_an_anchor_pins_where_the_object_is() {
    let mut s = on_hump();
    let was = pos(&mut s, 10);
    let v = run(&mut s, "motion.pin_point", json!({"frame": 10}));
    assert_eq!(v["anchor"], 1);
    assert!(pos(&mut s, 10).distance(was) < 0.01, "nothing moves on that frame");
    assert!(anchor(&s, 1).distance(was) < 0.01);
    // Outside the motion, or on an end: refused.
    assert!(fails(&mut s, "motion.pin_point", json!({"frame": 0})).contains("outside the motion"));
    // A straight motion's middle keys are pinned as soon as its path is the tween's own.
    let mut t = Session::default();
    run(&mut t, "draw.rect", json!({"x": 0, "y": 0, "w": 10, "h": 10}));
    run(&mut t, "timeline.create_motion_tween", json!({}));
    t.set_frame(10);
    run(&mut t, "element.translate", json!({"dx": 100, "dy": 0}));
    t.set_frame(20);
    run(&mut t, "element.translate", json!({"dx": 0, "dy": 100}));
    let info = run(&mut t, "motion.info", json!({}));
    assert_eq!(info["pins"].as_array().map(Vec::len), Some(3), "shown pinned while straight");
    run(&mut t, "motion.pin_point", json!({"frame": 15}));
    assert_eq!(motion(&t).pins, vec![Pin { frame: 10, anchor: 1 }, Pin { frame: 15, anchor: 2 }]);
}

#[test]
fn a_pinned_frames_x_and_y_keys_are_its_anchors_place() {
    let mut s = on_hump();
    run(&mut s, "motion.pin_point", json!({"frame": 8}));
    let a = anchor(&s, 1);
    // The Motion Editor sets X on the pinned frame: the anchor moves, and the object with it.
    let v = run(&mut s, "motion.set_key", json!({"property": "x", "frame": 8, "value": a.x + 12.0}));
    assert_eq!(v["anchor"], 1);
    assert!(anchor(&s, 1).distance(a + Vec2::new(12.0, 0.0)) < 1e-3);
    assert!(pos(&mut s, 8).distance(a + Vec2::new(12.0, 0.0)) < 1e-3);
    // And the ends: X on the last frame moves the path's end.
    run(&mut s, "motion.set_key", json!({"property": "y", "frame": 23, "value": 25.0}));
    assert!(anchor(&s, 2).distance(Vec2::new(105.0, 25.0)) < 1e-3 && pos(&mut s, 23).distance(Vec2::new(105.0, 25.0)) < 1e-3);
    // Removing the pinned frame's X key lets the pin go, with its Y key.
    let v = run(&mut s, "motion.remove_key", json!({"property": "x", "frame": 8}));
    assert_eq!(v["unpinned"], 1);
    assert_eq!(v["removed"], json!(["x", "y"]));
    assert!(motion(&s).pins.is_empty() && !key_frames(&s, Property::Y).contains(&8));
}

#[test]
fn point_edits_keep_pins_on_their_anchors() {
    let mut s = on_hump();
    run(&mut s, "motion.pin_point", json!({"frame": 14}));
    let pinned = anchor(&s, 1);
    // A point added before the pinned anchor: the pin follows it to index 2.
    run(&mut s, "motion.add_point", json!({"segment": 1, "t": 0.5}));
    assert_eq!(motion(&s).pins, vec![Pin { frame: 14, anchor: 2 }]);
    assert!(pos(&mut s, 14).distance(pinned) < 1e-3);
    // Deleting the point before it keeps the pin; deleting the pinned point lets it go.
    let v = run(&mut s, "motion.delete_points", json!({"anchors": [1]}));
    assert_eq!(v["unpinned"], 0);
    assert_eq!(motion(&s).pins, vec![Pin { frame: 14, anchor: 1 }]);
    assert!(pos(&mut s, 14).distance(pinned) < 1e-3);
    let v = run(&mut s, "motion.delete_points", json!({"anchors": [1]}));
    assert_eq!(v["unpinned"], 1);
    assert!(motion(&s).pins.is_empty() && key_frames(&s, Property::X) == vec![0, 23], "its keys went with it");
    // Straighten forgets the pins; their keys stay as the straight path's corners.
    run(&mut s, "motion.pin_point", json!({"frame": 14}));
    run(&mut s, "motion.clear_path", json!({}));
    assert!(motion(&s).pins.is_empty() && key_frames(&s, Property::X).contains(&14));
}

#[test]
fn insert_keyframe_inside_a_path_tween_pins_the_path_there() {
    let mut s = on_hump();
    let was = pos(&mut s, 11);
    s.set_frame(11);
    run(&mut s, "timeline.insert_keyframe", json!({}));
    assert_eq!(motion(&s).pins, vec![Pin { frame: 11, anchor: 1 }]);
    assert!(pos(&mut s, 11).distance(was) < 0.01, "{:?} vs {was:?}", pos(&mut s, 11));
    assert!(key_frames(&s, Property::Rotation).contains(&11), "every other property keyed too");
}

#[test]
fn pins_round_trip_through_the_file_format() {
    let mut s = on_hump();
    run(&mut s, "motion.pin_point", json!({"frame": 7}));
    let text = format::to_json(&s.doc).expect("save");
    assert!(text.contains("\"pins\""));
    let back = format::from_json(&text).expect("load");
    assert_eq!(back, s.doc);
    // A path tween from before pins writes none and reads back the same.
    let plain = on_hump();
    let text = format::to_json(&plain.doc).expect("save");
    assert!(!text.contains("\"pins\""));
    assert_eq!(format::from_json(&text).expect("load"), plain.doc);
}
