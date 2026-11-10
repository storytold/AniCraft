//! Motion paths through the command registry (ADR 0030): setting, clearing and orienting, the
//! object moved on a path reshaping it, one undo step each, and the file round trip.

use super::*;
use anicraft_doc::Property;
use anicraft_geom::Vec2;
use serde_json::json;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    match s.execute(id, &p) {
        Ok(v) => v,
        Err(e) => panic!("{id} {p}: {e}"),
    }
}

/// A 10 × 10 square centred on (5, 5) with a motion tween over frames 0..24, moved 100 to the
/// right by frame 23.
fn moving() -> Session {
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 10, "h": 10}));
    run(&mut s, "timeline.create_motion_tween", json!({}));
    s.set_frame(23);
    run(&mut s, "element.translate", json!({"dx": 100, "dy": 0}));
    s.set_frame(0);
    s
}

fn pos(s: &mut Session, frame: u32) -> Vec2 {
    s.set_frame(frame);
    s.visible_elements().first().map(|e| e.transform.translation()).unwrap_or_default()
}

fn rotation(s: &mut Session, frame: u32) -> f32 {
    s.set_frame(frame);
    s.visible_elements().first().map_or(f32::NAN, |e| e.transform.decompose().rotation.to_degrees())
}

fn motion(s: &Session) -> &anicraft_doc::MotionTween {
    s.active_layer().and_then(|l| l.keyframes.first()).and_then(|k| k.motion.as_ref()).expect("motion tween")
}

/// A hump from the start (5, 5) up and over to the end (105, 5).
const HUMP: &str = "M5 5 C38 -60 72 -60 105 5";

#[test]
fn a_path_bends_the_motion_and_keeps_its_ends_and_timing() {
    let mut s = moving();
    let straight = pos(&mut s, 12);
    let steps = s.history_len();
    let v = run(&mut s, "motion.set_path", json!({"d": HUMP}));
    assert_eq!(v["segments"], 1);
    assert_eq!(s.history_len(), steps + 1, "one undo step");
    assert!(pos(&mut s, 0).distance(Vec2::new(5.0, 5.0)) < 1e-3 && pos(&mut s, 23).distance(Vec2::new(105.0, 5.0)) < 1e-3);
    let mid = pos(&mut s, 12);
    assert!(mid.y < -30.0 && (mid.x - straight.x).abs() < 5.0, "over the hump: {mid:?}");
    let info = run(&mut s, "motion.info", json!({"frame": 12}));
    assert!(info["path"].as_str().is_some_and(|d| d.starts_with('M')));
    assert!((info["position"][1].as_f64().unwrap() as f32 - mid.y).abs() < 1e-3);
    assert_eq!(info["values"]["y"], 5.0, "on a path, X/Y are its timing");
    // Undo puts the straight motion back.
    run(&mut s, "edit.undo", json!({}));
    assert!(motion(&s).path.is_none() && pos(&mut s, 12).distance(straight) < 1e-3);
}

#[test]
fn a_path_on_a_tween_that_does_not_move_yet_spans_it() {
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 10, "h": 10}));
    run(&mut s, "timeline.create_motion_tween", json!({}));
    run(&mut s, "motion.set_path", json!({"d": "M0 0 L230 0"}));
    assert_eq!(motion(&s).position_range(), Some((0, 23)));
    assert!(pos(&mut s, 0).distance(Vec2::new(0.0, 0.0)) < 1e-3);
    assert!(pos(&mut s, 10).distance(Vec2::new(100.0, 0.0)) < 1e-2);
    assert!(pos(&mut s, 23).distance(Vec2::new(230.0, 0.0)) < 1e-3);
}

#[test]
fn bad_paths_and_places_are_refused() {
    let mut s = moving();
    for d in ["", "M0 0", "M0 0 L0 0", "M0 0 L1e30 0", "M 0 0 C", "nonsense"] {
        assert!(s.execute("motion.set_path", &json!({"d": d})).is_err(), "{d:?}");
    }
    assert!(s.execute("motion.set_path", &json!({})).is_err());
    assert!(s.execute("motion.set_path", &json!({"d": "x".repeat(2_000_000)})).is_err());
    assert!(s.execute("motion.clear_path", &json!({})).is_err(), "nothing to remove");
    assert!(s.execute("motion.orient", &json!({})).is_err() && s.execute("motion.orient", &json!({"orient": "yes"})).is_err());
    // Not in a motion tween.
    let mut plain = Session::default();
    run(&mut plain, "draw.rect", json!({"x": 0, "y": 0, "w": 10, "h": 10}));
    for id in ["motion.set_path", "motion.clear_path", "motion.orient"] {
        assert!(plain.execute(id, &json!({"d": HUMP, "orient": true})).is_err(), "{id}");
    }
    assert!(motion(&s).path.is_none(), "nothing was changed by the refusals");
}

#[test]
fn clearing_the_path_goes_back_to_the_straight_motion() {
    let mut s = moving();
    let straight = pos(&mut s, 12);
    run(&mut s, "motion.set_path", json!({"d": HUMP}));
    run(&mut s, "motion.orient", json!({"orient": true}));
    run(&mut s, "motion.clear_path", json!({}));
    assert!(motion(&s).path.is_none() && !motion(&s).orient, "orient goes with the path");
    assert!(pos(&mut s, 12).distance(straight) < 1e-3);
}

#[test]
fn orient_to_path_turns_the_object_with_a_quarter_circle() {
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 10, "h": 10}));
    run(&mut s, "timeline.create_motion_tween", json!({}));
    // A not-moving object has nothing to turn along.
    assert!(s.execute("motion.orient", &json!({"orient": true})).is_err());
    // Centre (5, 105), radius 100: from (5, 5) heading right round to (105, 105) heading down.
    let k = 55.228_48;
    run(&mut s, "motion.set_path", json!({"d": format!("M5 5 C{} 5 105 {} 105 105", 5.0 + k, 105.0 - k)}));
    let v = run(&mut s, "motion.orient", json!({"orient": true}));
    assert_eq!(v["path_created"], false);
    for f in [0u32, 6, 12, 18, 23] {
        let p = pos(&mut s, f);
        assert!((p.distance(Vec2::new(5.0, 105.0)) - 100.0).abs() < 0.05, "frame {f}: on the arc {p:?}");
        // The tangent of a circle is perpendicular to its radius: angle = the arc swept.
        let swept = ((p.x - 5.0) / 100.0).clamp(-1.0, 1.0).asin().to_degrees();
        assert!((rotation(&mut s, f) - swept).abs() < 0.2, "frame {f}: {} vs {swept}", rotation(&mut s, f));
    }
    run(&mut s, "motion.orient", json!({"orient": false}));
    assert!(rotation(&mut s, 12).abs() < 1e-3);
}

#[test]
fn orient_on_a_straight_motion_makes_its_path() {
    let mut s = moving();
    s.set_frame(23);
    run(&mut s, "element.translate", json!({"dx": 0, "dy": 0.0}));
    let v = run(&mut s, "motion.orient", json!({"orient": true}));
    assert_eq!(v["path_created"], true);
    assert!(motion(&s).orient && motion(&s).path.is_some());
    assert!(rotation(&mut s, 12).abs() < 1e-3, "a straight path doesn't turn");
}

#[test]
fn moving_the_object_on_a_path_reshapes_the_path() {
    let mut s = moving();
    run(&mut s, "motion.set_path", json!({"d": HUMP}));
    let keys = |s: &Session| motion(s).track(Property::X).map(|t| t.keys.iter().map(|k| k.frame).collect::<Vec<_>>());
    // At the start: the path's start moves, and the X/Y keys follow it.
    s.set_frame(0);
    let steps = s.history_len();
    run(&mut s, "element.translate", json!({"dx": -5, "dy": 20}));
    assert_eq!(s.history_len(), steps + 1);
    assert!(pos(&mut s, 0).distance(Vec2::new(0.0, 25.0)) < 1e-3);
    assert_eq!(motion(&s).track(Property::X).and_then(|t| t.keys.first()).map(|k| k.value), Some(0.0));
    // At the end: its end.
    s.set_frame(23);
    run(&mut s, "element.translate", json!({"dx": 10, "dy": 0}));
    assert!(pos(&mut s, 23).distance(Vec2::new(115.0, 5.0)) < 1e-3);
    // In between: the path bends through where the object was put; no key is added.
    let before = keys(&s);
    let want = pos(&mut s, 9) + Vec2::new(0.0, 40.0);
    s.set_frame(9);
    run(&mut s, "element.translate", json!({"dx": 0, "dy": 40}));
    assert!(pos(&mut s, 9).distance(want) < 0.01, "{:?} vs {want:?}", pos(&mut s, 9));
    assert_eq!(keys(&s), before, "the timing keeps its keys");
    assert!(pos(&mut s, 0).distance(Vec2::new(0.0, 25.0)) < 1e-3 && pos(&mut s, 23).distance(Vec2::new(115.0, 5.0)) < 1e-3, "the ends stay");
    // A drag on the stage is a gesture: one undo step however many moves.
    let steps = s.history_len();
    s.begin_gesture("Move");
    for _ in 0..5 {
        run(&mut s, "element.translate", json!({"dx": 2, "dy": 0}));
    }
    s.end_gesture();
    assert_eq!(s.history_len(), steps + 1);
}

#[test]
fn turning_the_object_on_an_oriented_path_keys_rotation_on_top() {
    let mut s = moving();
    run(&mut s, "motion.set_path", json!({"d": HUMP}));
    run(&mut s, "motion.orient", json!({"orient": true}));
    let was = rotation(&mut s, 12);
    s.set_frame(12);
    run(&mut s, "modify.rotate_cw", json!({}));
    let now = rotation(&mut s, 12);
    assert!(((now - was - 90.0 + 180.0).rem_euclid(360.0) - 180.0).abs() < 0.05, "{now} vs {}", was + 90.0);
    assert!(pos(&mut s, 0).distance(Vec2::new(5.0, 5.0)) < 1e-3, "turning in place leaves the path alone");
    assert!(motion(&s).track(Property::Rotation).is_some());
}

#[test]
fn motion_paths_round_trip_through_the_file_format() {
    let mut s = moving();
    run(&mut s, "motion.set_path", json!({"d": HUMP}));
    run(&mut s, "motion.orient", json!({"orient": true}));
    let text = format::to_json(&s.doc).expect("save");
    assert!(text.contains("\"orient\"") && text.contains("\"path\""));
    let back = format::from_json(&text).expect("load");
    assert_eq!(back, s.doc);
    // A file from before motion paths has neither field and opens as it was.
    let mut plain = moving();
    let text = format::to_json(&plain.doc).expect("save");
    assert!(!text.contains("\"orient\""));
    let back = format::from_json(&text).expect("load");
    assert_eq!(back, plain.doc);
    assert!(pos(&mut plain, 12).distance(Vec2::new(5.0 + 100.0 * 12.0 / 23.0, 5.0)) < 1e-3);
}
