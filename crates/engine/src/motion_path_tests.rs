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
    // In between: an anchor where the object was put, pinned to that frame (ADR 0049; ADR 0030
    // bent the path instead and added no key).
    let want = pos(&mut s, 9) + Vec2::new(0.0, 40.0);
    s.set_frame(9);
    run(&mut s, "element.translate", json!({"dx": 0, "dy": 40}));
    assert!(pos(&mut s, 9).distance(want) < 1e-3, "{:?} vs {want:?}", pos(&mut s, 9));
    assert_eq!(keys(&s), Some(vec![0, 9, 23]), "a position key on the pinned frame");
    assert_eq!(motion(&s).pins, vec![anicraft_doc::Pin { frame: 9, anchor: 1 }]);
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

// ----- point editing and moving the whole path (ADR 0038) ---------------------------------

/// The object's place on every frame of the span.
fn track_of(s: &mut Session) -> Vec<Vec2> {
    (0..24).map(|f| pos(s, f)).collect()
}

fn path_d(s: &Session) -> String {
    motion(s).path.as_ref().map(|p| p.to_svg()).unwrap_or_default()
}

/// An S through (55, -40): up from the start, over, and down to the end, with a smooth middle.
const S_CURVE: &str = "M5 5 C5 -40 30 -40 55 -40 C80 -40 105 -40 105 5";

#[test]
fn adding_a_point_leaves_the_curve_and_the_motion_unchanged() {
    let mut s = moving();
    run(&mut s, "motion.set_path", json!({"d": HUMP}));
    let before = track_of(&mut s);
    let keys = motion(&s).tracks.clone();
    let steps = s.history_len();
    // On the path where the object is at frame 9, as a click there would be.
    let at = pos(&mut s, 9);
    let v = run(&mut s, "motion.add_point", json!({"x": at.x, "y": at.y}));
    assert_eq!((v["anchor"].as_u64(), v["segments"].as_u64()), (Some(1), Some(2)));
    assert_eq!(s.history_len(), steps + 1, "one undo step");
    let new = Vec2::new(v["anchors"][1][0].as_f64().unwrap_or(0.0) as f32, v["anchors"][1][1].as_f64().unwrap_or(0.0) as f32);
    assert!(new.distance(at) < 0.05, "the new anchor is where the object passes: {new:?} vs {at:?}");
    assert_eq!(motion(&s).tracks, keys, "the timing keeps its keys");
    for (f, (a, b)) in before.iter().zip(track_of(&mut s)).enumerate() {
        assert!(a.distance(b) < 0.01, "frame {f}: {a:?} vs {b:?}");
    }
    // By segment and curve parameter too.
    let v = run(&mut s, "motion.add_point", json!({"segment": 2, "t": 0.5}));
    assert_eq!((v["anchor"].as_u64(), v["segments"].as_u64()), (Some(2), Some(3)));
    for (f, (a, b)) in before.iter().zip(track_of(&mut s)).enumerate() {
        assert!(a.distance(b) < 0.01, "frame {f}: {a:?} vs {b:?}");
    }
    // Refused: off the path, on an anchor, no such segment, mixed or missing params.
    let d = path_d(&s);
    for p in [
        json!({"x": 55, "y": 300}),
        json!({"segment": 1, "t": 0.0}),
        json!({"segment": 0, "t": 0.5}),
        json!({"segment": 9, "t": 0.5}),
        json!({"segment": 1, "t": 1.5}),
        json!({"segment": 1}),
        json!({"x": 5, "y": 5, "segment": 1, "t": 0.5}),
        json!({}),
        json!({"x": 5, "y": 5, "slop": -1}),
    ] {
        assert!(s.execute("motion.add_point", &p).is_err(), "{p}");
    }
    assert_eq!(path_d(&s), d, "refusals change nothing");
}

#[test]
fn deleting_a_point_keeps_the_ends_and_the_timing() {
    let mut s = moving();
    run(&mut s, "motion.set_path", json!({"d": S_CURVE}));
    let keys = motion(&s).tracks.clone();
    let steps = s.history_len();
    let v = run(&mut s, "motion.delete_points", json!({"anchors": [1]}));
    assert_eq!((v["deleted"].as_u64(), v["segments"].as_u64(), v["kept_ends"].as_bool()), (Some(1), Some(1), Some(false)));
    assert_eq!(s.history_len(), steps + 1, "one undo step");
    assert_eq!(motion(&s).tracks, keys, "the ends didn't move, so neither did a key");
    assert!(pos(&mut s, 0).distance(Vec2::new(5.0, 5.0)) < 1e-3 && pos(&mut s, 23).distance(Vec2::new(105.0, 5.0)) < 1e-3, "the ends stay");
    // The one cubic left still leaves straight up and comes straight down, over the top.
    let Some(anicraft_geom::PathCmd::CubicTo(c1, c2, _)) = motion(&s).path.as_ref().and_then(|p| p.cmds.get(1)).copied() else { panic!("a cubic") };
    assert!((c1.x - 5.0).abs() < 1e-3 && c1.y < 5.0 && (c2.x - 105.0).abs() < 1e-3 && c2.y < 5.0, "{c1:?} {c2:?}");
    assert!(pos(&mut s, 12).y < -25.0, "still over the top: {:?}", pos(&mut s, 12));
    // The ends can't go; asking for an end and a middle point deletes the middle one.
    let d = path_d(&s);
    let e = s.execute("motion.delete_points", &json!({"anchors": [0, 1]})).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(e.contains("ends"), "{e}");
    assert_eq!(path_d(&s), d);
    run(&mut s, "motion.add_point", json!({"segment": 1, "t": 0.4}));
    let v = run(&mut s, "motion.delete_points", json!({"anchors": [0, 1, 2]}));
    assert_eq!((v["deleted"].as_u64(), v["kept_ends"].as_bool()), (Some(1), Some(true)));
    for p in [json!({"anchors": [7]}), json!({"anchors": []}), json!({"anchors": "1"}), json!({})] {
        assert!(s.execute("motion.delete_points", &p).is_err(), "{p}");
    }
}

#[test]
fn a_point_added_then_deleted_gives_the_path_back() {
    let mut s = moving();
    run(&mut s, "motion.set_path", json!({"d": HUMP}));
    let before = track_of(&mut s);
    run(&mut s, "motion.add_point", json!({"segment": 1, "t": 0.3}));
    run(&mut s, "motion.delete_points", json!({"anchors": [1]}));
    for (f, (a, b)) in before.iter().zip(track_of(&mut s)).enumerate() {
        assert!(a.distance(b) < 0.02, "frame {f}: {a:?} vs {b:?}");
    }
}

#[test]
fn converting_a_point_makes_it_a_corner_or_smooth() {
    let mut s = moving();
    run(&mut s, "motion.set_path", json!({"d": S_CURVE}));
    let point = |s: &Session, cmd: usize, slot: u8| motion(s).path.as_ref().and_then(|p| p.point(cmd, slot)).unwrap_or_default();
    let v = run(&mut s, "motion.convert_point", json!({"anchor": 1, "smooth": false}));
    assert_eq!(v["changed"], true);
    let a = point(&s, 1, 0);
    assert!(point(&s, 1, 2).distance(a) < 1e-4 && point(&s, 2, 1).distance(a) < 1e-4, "both handles on the anchor");
    run(&mut s, "motion.convert_point", json!({"anchor": 1, "smooth": true}));
    let (i, o) = (point(&s, 1, 2) - a, point(&s, 2, 1) - a);
    assert!(i.length() > 1.0 && o.length() > 1.0 && i.normalized().dot(o.normalized()) < -0.9999, "in line again: {i:?} {o:?}");
    assert!(pos(&mut s, 0).distance(Vec2::new(5.0, 5.0)) < 1e-3 && pos(&mut s, 23).distance(Vec2::new(105.0, 5.0)) < 1e-3);
    for p in [json!({"anchor": 9, "smooth": true}), json!({"anchor": 1}), json!({"smooth": true})] {
        assert!(s.execute("motion.convert_point", &p).is_err(), "{p}");
    }
}

#[test]
fn editing_points_of_a_straight_motion_makes_its_path() {
    let mut s = moving();
    assert!(motion(&s).path.is_none());
    let info = run(&mut s, "motion.info", json!({}));
    assert_eq!(info["anchors"], json!([[5.0, 5.0], [105.0, 5.0]]), "the straight path's anchors");
    let before = track_of(&mut s);
    run(&mut s, "motion.add_point", json!({"x": 55, "y": 5}));
    assert!(motion(&s).path.is_some(), "the straight path became the tween's own");
    for (f, (a, b)) in before.iter().zip(track_of(&mut s)).enumerate() {
        assert!(a.distance(b) < 0.01, "frame {f}: {a:?} vs {b:?}");
    }
    // A tween that doesn't move has no path to edit.
    let mut still = Session::default();
    run(&mut still, "draw.rect", json!({"x": 0, "y": 0, "w": 10, "h": 10}));
    run(&mut still, "timeline.create_motion_tween", json!({}));
    for (id, p) in [("motion.add_point", json!({"x": 5, "y": 5})), ("motion.delete_points", json!({"anchors": [1]})), ("motion.convert_point", json!({"anchor": 0, "smooth": true}))] {
        assert!(still.execute(id, &p).is_err(), "{id}");
    }
}

#[test]
fn moving_the_whole_path_keeps_its_shape_and_timing() {
    let mut s = moving();
    run(&mut s, "motion.set_path", json!({"d": S_CURVE}));
    run(&mut s, "motion.orient", json!({"orient": true}));
    s.set_frame(12);
    run(&mut s, "modify.rotate_cw", json!({}));
    let before = track_of(&mut s);
    let turns: Vec<f32> = (0..24).map(|f| rotation(&mut s, f)).collect();
    let keys = motion(&s).key_frames();
    let steps = s.history_len();
    let d = Vec2::new(-40.5, 120.25);
    let v = run(&mut s, "motion.move_path", json!({"dx": d.x, "dy": d.y}));
    assert_eq!(s.history_len(), steps + 1, "one undo step");
    assert!(v["path"].as_str().is_some_and(|p| p.starts_with("M-35.5 125.25")), "{}", v["path"]);
    for (f, a) in before.iter().enumerate() {
        let b = pos(&mut s, f as u32);
        assert!((*a + d).distance(b) < 1e-3, "frame {f}: {a:?} + d vs {b:?}");
        assert!((rotation(&mut s, f as u32) - turns[f]).abs() < 1e-3, "frame {f}: the same turn");
    }
    assert_eq!(motion(&s).key_frames(), keys, "no key moved in time");
    // A drag is a gesture: one undo step for many moves; undo puts it back.
    let steps = s.history_len();
    s.begin_gesture("Move Motion Path");
    for _ in 0..6 {
        run(&mut s, "motion.move_path", json!({"dx": 2.5, "dy": -1}));
    }
    s.end_gesture();
    assert_eq!(s.history_len(), steps + 1);
    assert!(pos(&mut s, 23).distance(before[23] + d + Vec2::new(15.0, -6.0)) < 1e-3);
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert!(pos(&mut s, 23).distance(before[23]) < 1e-3);
    for p in [json!({"dx": 2e6, "dy": 0}), json!({"dx": 1}), json!({"dx": "1", "dy": 0})] {
        assert!(s.execute("motion.move_path", &p).is_err(), "{p}");
    }
}

#[test]
fn moving_a_straight_motion_moves_it_all() {
    // No path: the X/Y keys move; an axis without keys moves with the object itself.
    let mut s = moving();
    let before = track_of(&mut s);
    run(&mut s, "motion.move_path", json!({"dx": 10, "dy": 30}));
    assert!(motion(&s).path.is_none(), "no path was made");
    for (f, a) in before.iter().enumerate() {
        assert!((*a + Vec2::new(10.0, 30.0)).distance(pos(&mut s, f as u32)) < 1e-3, "frame {f}");
    }
}
