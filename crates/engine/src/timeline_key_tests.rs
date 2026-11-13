//! F5, F6 and F7 as the keys work in the editor (user reports #242 and #254).

use super::*;
use anicraft_doc::{Keyframe, LayerId};
use serde_json::json;
use std::sync::Arc;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    match s.execute(id, &p) {
        Ok(v) => v,
        Err(e) => panic!("{id} {p}: {e}"),
    }
}

fn spans(s: &Session, layer: LayerId) -> Vec<(u32, u32)> {
    s.timeline().layer(layer).unwrap().keyframes.iter().map(|k| (k.start, k.duration)).collect()
}

/// A layer whose keyframes are exactly `keys` (start, duration).
fn layer_with(s: &mut Session, keys: &[(u32, u32)]) -> LayerId {
    let id = LayerId(run(s, "layer.new", json!({}))["layer"].as_u64().unwrap());
    let l = s.layer_mut(Some(id)).unwrap();
    l.keyframes = keys.iter().map(|&(a, d)| Arc::new(Keyframe::new(a, d))).collect();
    id
}

#[test]
fn f5_with_no_frame_selection_extends_every_layer_that_reaches_the_playhead() {
    // The report's example: playhead at frame 5 (index 4).
    let mut s = Session::default();
    let l1 = layer_with(&mut s, &[(0, 8)]);
    let l2 = layer_with(&mut s, &[(0, 4)]);
    let l3 = layer_with(&mut s, &[(0, 6)]);
    let l4 = layer_with(&mut s, &[(0, 2), (2, 7)]);
    // Content only after the playhead, and a gap under it: both move along.
    let later = layer_with(&mut s, &[(6, 3)]);
    let gap = layer_with(&mut s, &[(0, 2), (7, 2)]);
    let first = s.timeline().layers.iter().find(|l| ![l1, l2, l3, l4, later, gap].contains(&l.id)).map(|l| l.id).unwrap();
    s.layer_mut(Some(first)).unwrap().keyframes = vec![Arc::new(Keyframe::new(0, 1))];
    s.layer = l2;
    s.set_frame(4);
    let v = run(&mut s, "timeline.insert_frame", json!({"all_layers": true}));
    assert_eq!(spans(&s, l1), vec![(0, 9)], "frames 1–8: extended");
    assert_eq!(spans(&s, l2), vec![(0, 4)], "frames 1–4 end before the playhead: unchanged, even when active");
    assert_eq!(spans(&s, l3), vec![(0, 7)], "frames 1–6: extended");
    assert_eq!(spans(&s, l4), vec![(0, 2), (2, 8)], "frames 1–9: extended");
    assert_eq!(spans(&s, later), vec![(7, 3)], "a key after the playhead moves one frame later");
    assert_eq!(spans(&s, gap), vec![(0, 2), (8, 2)], "a gap under the playhead grows by one");
    assert_eq!(spans(&s, first), vec![(0, 1)]);
    assert_eq!(v["layers"].as_array().unwrap().len(), 5);
    assert_eq!(s.frame, 4, "without advance the playhead stays");
    // One undo step takes it back on every layer.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(spans(&s, l1), vec![(0, 8)]);
    assert_eq!(spans(&s, l4), vec![(0, 2), (2, 7)]);
    assert_eq!(spans(&s, gap), vec![(0, 2), (7, 2)]);
}

#[test]
fn f5_past_every_layer_lengthens_the_active_one() {
    let mut s = Session::default();
    let a = layer_with(&mut s, &[(0, 3)]);
    let b = layer_with(&mut s, &[(0, 5)]);
    s.layer = a;
    s.set_frame(9);
    run(&mut s, "timeline.insert_frame", json!({"all_layers": true, "advance": true}));
    assert_eq!(spans(&s, a), vec![(0, 10)], "extended up to the playhead");
    assert_eq!(spans(&s, b), vec![(0, 5)]);
    assert_eq!(s.frame, 9, "the frame made is the playhead's own");
}

#[test]
fn f5_advance_moves_the_playhead_onto_the_new_frame() {
    let mut s = Session::default();
    let a = layer_with(&mut s, &[(0, 1)]);
    s.layer = a;
    for want in 1..4 {
        let v = run(&mut s, "timeline.insert_frame", json!({"layer": a.0, "advance": true}));
        assert_eq!(s.frame, want);
        assert_eq!(v["playhead"], json!(want));
    }
    assert_eq!(spans(&s, a), vec![(0, 4)], "holding F5 walks along, one frame each");
}

#[test]
fn f6_on_a_keyframe_keys_the_next_frame_and_follows() {
    let mut s = Session::default();
    let a = layer_with(&mut s, &[(0, 1)]);
    s.layer = a;
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 10, "h": 10}));
    for want in 1..4 {
        run(&mut s, "timeline.insert_keyframe", json!({"advance": true}));
        assert_eq!(s.frame, want, "F6 on a keyframe makes the next one and goes there");
    }
    assert_eq!(spans(&s, a), vec![(0, 1), (1, 1), (2, 1), (3, 1)]);
    assert_eq!(s.timeline().layer(a).unwrap().keyframes[3].elements.len(), 1, "a copy of the drawing");
    // Inside a span (not on its key) F6 keys the playhead's own frame.
    run(&mut s, "timeline.insert_frame", json!({"layer": a.0, "frame": 3, "count": 3}));
    s.set_frame(5);
    run(&mut s, "timeline.insert_keyframe", json!({"advance": true}));
    assert_eq!(s.frame, 5);
    assert_eq!(spans(&s, a), vec![(0, 1), (1, 1), (2, 1), (3, 2), (5, 2)]);
    // Without advance (scripts), the old behaviour: nothing new on a keyframe.
    run(&mut s, "timeline.insert_keyframe", json!({}));
    assert_eq!(s.frame, 5);
    assert_eq!(spans(&s, a).len(), 5);
}

#[test]
fn f7_on_a_keyframe_makes_a_blank_one_next_instead_of_clearing() {
    let mut s = Session::default();
    let a = layer_with(&mut s, &[(0, 1)]);
    s.layer = a;
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 10, "h": 10}));
    run(&mut s, "timeline.insert_blank_keyframe", json!({"advance": true}));
    assert_eq!(s.frame, 1);
    let l = s.timeline().layer(a).unwrap();
    assert_eq!(l.keyframes[0].elements.len(), 1, "the drawing is kept");
    assert!(l.keyframes[1].elements.is_empty(), "a blank key on the next frame");
    // Across two layers it advances only when both are on a keyframe.
    let b = layer_with(&mut s, &[(0, 4)]);
    run(&mut s, "timeline.insert_blank_keyframe", json!({"layers": [a.0, b.0], "frame": 1, "advance": true}));
    assert_eq!(s.frame, 1);
    assert_eq!(spans(&s, b), vec![(0, 1), (1, 3)]);
}

#[test]
fn advance_at_the_last_frame_stays_in_range() {
    let mut s = Session::default();
    let a = layer_with(&mut s, &[(MAX_FRAME - 1, 2)]);
    s.layer = a;
    s.set_frame(MAX_FRAME);
    assert!(s.execute("timeline.insert_frame", &json!({"advance": true})).is_err());
    s.layer_mut(Some(a)).unwrap().keyframes = vec![Arc::new(Keyframe::new(MAX_FRAME, 1))];
    let _ = s.execute("timeline.insert_keyframe", &json!({"advance": true}));
    assert!(s.frame <= MAX_FRAME);
}
