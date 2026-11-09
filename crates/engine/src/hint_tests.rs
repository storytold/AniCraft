//! Shape hints through the command registry (ADR 0023): adding, snapping, moving on either
//! keyframe, removing, clearing, limits, undo, what removes them, the file round trip and a
//! fuzz on a document that has a shape tween.

use super::*;
use anicraft_geom::{PathCmd, Vec2};
use serde_json::json;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    match s.execute(id, &p) {
        Ok(v) => v,
        Err(e) => panic!("{id} {p}: {e}"),
    }
}

/// A 100 × 100 square at the origin on frame 1 shape-tweening to a circle centred on
/// (250, 50) on frame 11.
fn square_to_circle() -> Session {
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 100}));
    run(&mut s, "timeline.insert_blank_keyframe", json!({"frame": 10}));
    s.set_frame(10);
    run(&mut s, "draw.ellipse", json!({"x": 200, "y": 0, "w": 100, "h": 100}));
    run(&mut s, "timeline.create_shape_tween", json!({"frame": 0}));
    s.set_frame(0);
    s
}

fn hints(s: &mut Session, frame: u32) -> Value {
    run(s, "tween.hints", json!({"frame": frame}))
}

/// Whether the shape at `frame` has an anchor at `q` (stage space).
fn anchor_at(s: &Session, frame: u32, q: Vec2) -> bool {
    let dl = anicraft_doc::evaluate(&s.doc, frame);
    dl.items.iter().any(|i| {
        i.path.cmds.iter().any(|c| match *c {
            PathCmd::MoveTo(a) | PathCmd::LineTo(a) | PathCmd::CubicTo(_, _, a) => i.transform.apply(a).distance(q) < 1e-2,
            _ => false,
        })
    })
}

#[test]
fn a_hint_added_then_placed_on_both_keyframes_steers_the_morph() {
    let mut s = square_to_circle();
    let (corner, bottom) = (Vec2::new(0.0, 0.0), Vec2::new(250.0, 100.0));
    let half = corner.lerp(bottom, 0.5);
    assert!(!anchor_at(&s, 5, half), "without a hint the corner goes elsewhere");
    // Added at the shapes' centres, unplaced on both keyframes.
    let v = run(&mut s, "tween.add_hint", json!({}));
    assert_eq!(v["hint"], "a");
    assert_eq!((v["start"]["x"].as_f64(), v["end"]["x"].as_f64()), (Some(50.0), Some(250.0)));
    let h = &hints(&mut s, 0)["hints"][0];
    assert_eq!((h["start_placed"].as_bool(), h["end_placed"].as_bool(), h["active"].as_bool()), (Some(false), Some(false), Some(false)));
    // Dragged near the corner on the first keyframe: it snaps onto it.
    let v = run(&mut s, "tween.move_hint", json!({"hint": "a", "x": -6, "y": -4}));
    assert_eq!((v["side"].as_str(), v["x"].as_f64(), v["y"].as_f64()), (Some("start"), Some(0.0), Some(0.0)));
    assert_eq!(hints(&mut s, 0)["hints"][0]["start_placed"], true);
    assert!(!anchor_at(&s, 5, half), "half placed: the morph doesn't use it yet");
    // On the end keyframe the frame shows the end side.
    let v = run(&mut s, "tween.move_hint", json!({"hint": "a", "frame": 10, "x": 250, "y": 120}));
    assert_eq!(v["side"], "end");
    let list = hints(&mut s, 10);
    assert_eq!(list["side"], "end");
    assert_eq!(list["hints"][0]["active"], true);
    assert!(anchor_at(&s, 5, half), "the corner travels to the circle's bottom");
    // Undo takes back the end move (one step per command).
    s.undo().expect("undo");
    assert!(!anchor_at(&s, 5, half));
    assert_eq!(hints(&mut s, 0)["hints"][0]["end_placed"], false);
}

#[test]
fn letters_go_up_to_z_and_removing_one_moves_the_rest_down() {
    let mut s = square_to_circle();
    for i in 0..26u8 {
        let v = run(&mut s, "tween.add_hint", json!({"frame": 4}));
        assert_eq!(v["hint"], char::from(b'a' + i).to_string());
    }
    let e = s.execute("tween.add_hint", &json!({})).unwrap_err().to_string();
    assert!(e.contains("26"), "{e}");
    run(&mut s, "tween.move_hint", json!({"hint": "c", "x": 100, "y": 50}));
    let v = run(&mut s, "tween.remove_hint", json!({"hint": "b"}));
    assert_eq!((v["removed"].as_str(), v["count"].as_u64()), (Some("b"), Some(25)));
    // What was c is now b, still where it was put.
    let list = hints(&mut s, 0);
    assert_eq!(list["hints"][1]["start"]["x"], 100.0);
    assert_eq!(list["hints"][1]["start_placed"], true);
    // Index form works too; bad letters are refused.
    run(&mut s, "tween.remove_hint", json!({"hint": 0}));
    for bad in [json!({"hint": "zz"}), json!({"hint": "z"}), json!({"hint": -1}), json!({})] {
        assert!(s.execute("tween.remove_hint", &bad).is_err(), "{bad}");
    }
    assert!(s.execute("tween.move_hint", &json!({"hint": "a", "x": 0, "y": 0, "side": "middle"})).is_err());
    let v = run(&mut s, "tween.clear_hints", json!({}));
    assert_eq!(v["removed"], 24);
    assert!(s.execute("tween.clear_hints", &json!({})).is_err(), "nothing left to clear");
}

#[test]
fn hints_need_a_shape_tween_and_go_with_it() {
    let mut s = square_to_circle();
    run(&mut s, "tween.add_hint", json!({}));
    // Frame 12 is past the tween; the end keyframe's own span (frame 11 onward) has no tween.
    s.set_frame(10);
    run(&mut s, "timeline.insert_frame", json!({"frame": 11, "count": 3}));
    assert!(s.execute("tween.add_hint", &json!({"frame": 12})).is_err());
    // A classic tween keeps no hints; nor does a removed one.
    run(&mut s, "timeline.create_tween", json!({"frame": 0}));
    assert!(s.active_layer().unwrap().keyframes[0].hints.is_empty());
    s.undo().expect("undo");
    assert_eq!(s.active_layer().unwrap().keyframes[0].hints.len(), 1);
    run(&mut s, "timeline.remove_tween", json!({"frame": 0}));
    assert!(s.active_layer().unwrap().keyframes[0].hints.is_empty());
    assert!(s.execute("tween.add_hint", &json!({"frame": 0})).is_err());
    // A locked layer refuses edits.
    s.undo().expect("undo");
    run(&mut s, "layer.toggle_lock", json!({}));
    assert!(s.execute("tween.add_hint", &json!({})).is_err());
    assert!(s.execute("tween.hints", &json!({})).is_ok(), "reading is fine");
}

#[test]
fn hints_round_trip_through_the_file_format() {
    let mut s = square_to_circle();
    run(&mut s, "tween.add_hint", json!({}));
    run(&mut s, "tween.move_hint", json!({"hint": "a", "x": 0, "y": 0}));
    run(&mut s, "tween.move_hint", json!({"hint": "a", "side": "end", "x": 250, "y": 100}));
    let text = format::to_json(&s.doc).expect("save");
    let back = format::from_json(&text).expect("load");
    assert_eq!(back, s.doc);
    let k = &back.timeline(0).layers[0].keyframes[0];
    assert_eq!(k.hints.len(), 1);
    assert!(k.hints[0].end.distance(Vec2::new(250.0, 100.0)) < 1e-3);
}

#[test]
fn hint_commands_survive_hostile_params_on_a_shape_tween() {
    let hostile = [
        Value::Null,
        json!({}),
        json!({"hint": u64::MAX, "x": 1e30, "y": -1e30}),
        json!({"hint": "a", "x": 1e7, "y": -1e7, "side": "end"}),
        json!({"hint": "A", "x": 50, "y": 50, "side": "start", "frame": 10}),
        json!({"hint": "é", "x": "1", "y": null}),
        json!({"hint": 25, "frame": u64::MAX, "layer": u64::MAX}),
        json!({"hint": "a", "x": 5, "y": 5, "frame": 99_999}),
        json!({"hint": [], "side": 3}),
    ];
    let ids: Vec<&str> = commands().iter().map(|c| c.id).filter(|id| id.starts_with("tween.")).collect();
    assert_eq!(ids.len(), 5);
    let mut s = square_to_circle();
    for _ in 0..3 {
        run(&mut s, "tween.add_hint", json!({}));
    }
    for id in &ids {
        for p in &hostile {
            let _ = s.execute(id, p);
        }
    }
    // Hints thrown far away and then the shapes edited: the morph still evaluates and renders.
    for f in [0, 3, 5, 9, 10, 11] {
        let _ = anicraft_render::render_frame(&s.doc, f, 0.1);
    }
}

