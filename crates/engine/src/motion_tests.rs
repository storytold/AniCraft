//! Motion tweens through the command registry (ADR 0016): creation, auto-key, property keys and
//! eases, F6 inside a span, removal, conversion, frames clipboard and the file round trip.

use super::*;
use anicraft_doc::{Easing, Property};
use serde_json::json;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    match s.execute(id, &p) {
        Ok(v) => v,
        Err(e) => panic!("{id} {p}: {e}"),
    }
}

/// A 10 × 10 square centred on (5, 5), selected, with a motion tween over a one-second span.
fn tweened() -> Session {
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 10, "h": 10}));
    let v = run(&mut s, "timeline.create_motion_tween", json!({}));
    assert_eq!(v["frames"], 24, "a one-frame span grows to a second at 24 fps");
    assert_eq!(v["converted"], false);
    s
}

fn pos(s: &mut Session, frame: u32) -> (f32, f32) {
    s.set_frame(frame);
    let t = s.visible_elements().first().map(|e| e.transform.translation()).unwrap_or_default();
    (t.x, t.y)
}

fn layer(s: &Session) -> &Layer {
    s.active_layer().expect("active layer")
}

fn motion(s: &Session) -> &anicraft_doc::MotionTween {
    layer(s).keyframes[0].motion.as_ref().expect("motion tween")
}

#[test]
fn moving_the_object_inside_a_motion_tween_adds_property_keys_not_keyframes() {
    let mut s = tweened();
    let steps = s.history_len();
    s.set_frame(12);
    run(&mut s, "element.translate", json!({"dx": 100, "dy": 0}));
    // One keyframe on the timeline still, one undo step, an X track keyed at 0 and 12.
    assert_eq!(layer(&s).keyframes.len(), 1);
    assert_eq!(s.history_len(), steps + 1);
    let x = motion(&s).track(Property::X).expect("x track");
    assert_eq!(x.keys.iter().map(|k| (k.frame, k.value)).collect::<Vec<_>>(), vec![(0, 5.0), (12, 105.0)]);
    assert!(motion(&s).track(Property::Y).is_none(), "Y did not change, so it has no track");
    assert_eq!(pos(&mut s, 0), (5.0, 5.0));
    assert_eq!(pos(&mut s, 6), (55.0, 5.0));
    assert_eq!(pos(&mut s, 20), (105.0, 5.0), "holds after the last key");
    // Moving again at the same frame updates that key; moving elsewhere adds another.
    s.set_frame(12);
    run(&mut s, "element.translate", json!({"dx": -5, "dy": 0}));
    s.set_frame(23);
    run(&mut s, "element.translate", json!({"dx": 0, "dy": 50}));
    // Y's first key pins frame 0, so it runs over the whole span; X kept its own keys.
    let (x12, y12) = pos(&mut s, 12);
    assert!(x12 == 100.0 && (y12 - (5.0 + 50.0 * 12.0 / 23.0)).abs() < 1e-3, "{x12} {y12}");
    assert_eq!(pos(&mut s, 23), (100.0, 55.0));
    assert_eq!(motion(&s).track(Property::Y).map(|t| t.keys.len()), Some(2));
    // The stored object is still its first-frame self.
    assert_eq!(layer(&s).keyframes[0].elements[0].transform.translation(), anicraft_geom::Vec2::new(5.0, 5.0));
    // Undo takes the keys back one edit at a time.
    run(&mut s, "edit.undo", json!({}));
    assert!(motion(&s).track(Property::Y).is_none());
}

#[test]
fn on_the_first_frame_an_unkeyed_property_moves_the_object_itself() {
    let mut s = tweened();
    run(&mut s, "element.translate", json!({"dx": 10, "dy": 0}));
    assert!(motion(&s).tracks.is_empty(), "no keys: the base moved");
    assert_eq!(pos(&mut s, 0), (15.0, 5.0));
    // Once X is keyed, the first frame keys it too, and the rest of the curve stays.
    s.set_frame(10);
    run(&mut s, "element.translate", json!({"dx": 100, "dy": 0}));
    s.set_frame(0);
    run(&mut s, "element.translate", json!({"dx": -15, "dy": 0}));
    let x = motion(&s).track(Property::X).expect("x track");
    assert_eq!(x.keys.iter().map(|k| (k.frame, k.value)).collect::<Vec<_>>(), vec![(0, 0.0), (10, 115.0)]);
}

#[test]
fn transforms_and_colour_edits_key_their_own_properties() {
    let mut s = tweened();
    s.set_frame(8);
    run(&mut s, "element.set_alpha", json!({"alpha": 0.25}));
    run(&mut s, "element.set_transform", json!({"sx": 2.0}));
    let m = motion(&s);
    assert_eq!(m.track(Property::Alpha).map(|t| t.keys.iter().map(|k| k.value).collect::<Vec<_>>()), Some(vec![100.0, 25.0]));
    assert!(m.track(Property::ScaleX).is_some_and(|t| (t.keys[1].value - 200.0).abs() < 1e-3));
    assert!(m.track(Property::ScaleY).is_none() && m.track(Property::X).is_none());
    s.set_frame(4);
    let e = s.visible_elements()[0].clone();
    assert!((e.color.alpha() - 0.625).abs() < 1e-5 && (e.transform.decompose().sx - 1.5).abs() < 1e-4);
    // Non-tweenable edits change the object itself, whatever the frame.
    run(&mut s, "edit.select_all", json!({}));
    run(&mut s, "element.set_fill", json!({"color": "#ff0000"}));
    s.set_frame(0);
    let fill = match &s.visible_elements()[0].kind {
        ElementKind::Shape(sh) => sh.fill.clone(),
        _ => None,
    };
    assert_eq!(fill, Some(anicraft_doc::Paint::Solid(anicraft_doc::Rgba::hex(0xff0000))));
    assert_eq!(layer(&s).keyframes.len(), 1);
}

#[test]
fn rotating_past_half_a_turn_keeps_turning_the_same_way() {
    let mut s = tweened();
    run(&mut s, "motion.set_key", json!({"property": "rotation", "frame": 10, "value": 170}));
    s.set_frame(10);
    run(&mut s, "edit.select_all", json!({}));
    // 20° more about the centre: 190°, not −170° (which would spin back the long way).
    let (c, r) = (5.0f32, 20f32.to_radians());
    let m = anicraft_geom::Affine::translate(c, c).mul(&anicraft_geom::Affine::rotate(r)).mul(&anicraft_geom::Affine::translate(-c, -c));
    let [a, b, cc, d, e, f] = m.m;
    run(&mut s, "modify.transform", json!({"a": a, "b": b, "c": cc, "d": d, "e": e, "f": f}));
    let rot = motion(&s).track(Property::Rotation).and_then(|t| t.keys.last()).map(|k| k.value).unwrap_or(0.0);
    assert!((rot - 190.0).abs() < 1e-2, "{rot}");
}

#[test]
fn property_key_commands_and_segment_eases() {
    let mut s = tweened();
    run(&mut s, "motion.set_key", json!({"property": "x", "frame": 20, "value": 205}));
    run(&mut s, "motion.set_key", json!({"property": "Y", "frame": 20, "value": 105}));
    run(&mut s, "motion.set_ease", json!({"property": "x", "frame": 3, "easing": {"bezier": [0.4, 0.0, 0.2, 1.0]}}));
    let info = run(&mut s, "motion.info", json!({"frame": 10}));
    assert_eq!(info["start"], 0);
    assert_eq!(info["frames"], 24);
    assert_eq!(info["tracks"][0]["property"], "x");
    assert_eq!(info["tracks"][0]["keys"][0]["bezier"], json!([0.4f32, 0.0, 0.2f32, 1.0]));
    // X follows its Bézier; Y stays linear.
    let ease = Easing::CubicBezier(0.4, 0.0, 0.2, 1.0).apply(0.5);
    let (x, y) = pos(&mut s, 10);
    assert!((x - (5.0 + 200.0 * ease)).abs() < 1e-3 && (y - 55.0).abs() < 1e-3, "{x} {y}");
    assert!((info["values"]["x"].as_f64().unwrap_or(0.0) as f32 - x).abs() < 1e-3);
    // Every property's segment at once.
    let v = run(&mut s, "motion.set_ease", json!({"frame": 5, "easing": "CubicOut"}));
    assert_eq!(v["properties"], json!(["x", "y"]));
    // Remove one property's key, then the rest at that frame.
    run(&mut s, "motion.remove_key", json!({"property": "y", "frame": 20}));
    assert_eq!(motion(&s).track(Property::Y).map(|t| t.keys.len()), Some(1));
    let v = run(&mut s, "motion.remove_key", json!({"frame": 0}));
    assert_eq!(v["removed"], json!(["x", "y"]));
    // Errors say what to do.
    for (id, p) in [
        ("motion.set_key", json!({"property": "alpha", "value": 140})),
        ("motion.set_key", json!({"property": "size", "value": 1})),
        ("motion.set_key", json!({"property": "x"})),
        ("motion.remove_key", json!({"frame": 7})),
        ("motion.set_ease", json!({"frame": 22, "easing": "Linear"})),
        ("motion.set_key", json!({"property": "x", "value": 1, "frame": 40})),
    ] {
        assert!(s.execute(id, &p).is_err(), "{id} {p}");
    }
    run(&mut s, "timeline.insert_blank_keyframe", json!({"frame": 30}));
    let e = s.execute("motion.set_key", &json!({"property": "x", "value": 1, "frame": 30})).unwrap_err().to_string();
    assert!(e.contains("Create Motion Tween"), "{e}");
}

#[test]
fn f6_inside_a_motion_tween_keys_every_property_without_splitting_the_span() {
    let mut s = tweened();
    run(&mut s, "motion.set_key", json!({"property": "x", "frame": 20, "value": 105}));
    run(&mut s, "timeline.insert_keyframe", json!({"frame": 10}));
    assert_eq!(layer(&s).keyframes.len(), 1);
    let m = motion(&s);
    assert_eq!(m.tracks.len(), Property::ALL.len());
    assert!(m.tracks.iter().all(|t| t.key_at(10).is_some()));
    assert!((m.track(Property::X).and_then(|t| t.keys.get(1)).map_or(0.0, |k| k.value) - 55.0).abs() < 1e-3);
    // The motion is unchanged by the new keys.
    assert!((pos(&mut s, 15).0 - 80.0).abs() < 1e-3);
}

#[test]
fn removing_or_replacing_a_motion_tween_keeps_the_first_frame() {
    let mut s = tweened();
    s.set_frame(0);
    run(&mut s, "motion.set_key", json!({"property": "x", "frame": 0, "value": 50}));
    run(&mut s, "motion.set_key", json!({"property": "x", "frame": 20, "value": 150}));
    run(&mut s, "timeline.remove_tween", json!({}));
    assert!(layer(&s).keyframes[0].motion.is_none());
    assert_eq!(pos(&mut s, 0).0, 50.0);
    assert_eq!(pos(&mut s, 20).0, 50.0);
    // A classic tween replaces a motion tween the same way, and back.
    run(&mut s, "timeline.create_motion_tween", json!({}));
    run(&mut s, "timeline.create_tween", json!({}));
    let k = &layer(&s).keyframes[0];
    assert!(k.motion.is_none() && k.tween.is_some());
    run(&mut s, "timeline.create_motion_tween", json!({}));
    let k = &layer(&s).keyframes[0];
    assert!(k.motion.is_some() && k.tween.is_none());
}

#[test]
fn several_objects_become_one_symbol_to_tween() {
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 10, "h": 10}));
    run(&mut s, "draw.ellipse", json!({"x": 20, "y": 0, "w": 10, "h": 10}));
    run(&mut s, "timeline.insert_frame", json!({"count": 9}));
    let v = run(&mut s, "timeline.create_motion_tween", json!({}));
    assert_eq!(v["converted"], true);
    assert_eq!(v["frames"], 10, "a longer span keeps its length");
    assert_eq!(s.doc.library.len(), 1);
    assert_eq!(layer(&s).keyframes[0].elements.len(), 1);
    // Empty keys have nothing to tween; folders have no frames.
    run(&mut s, "timeline.insert_blank_keyframe", json!({"frame": 20}));
    assert!(s.execute("timeline.create_motion_tween", &json!({"frame": 20})).is_err());
    let f = run(&mut s, "layer.new_folder", json!({}));
    assert!(s.execute("timeline.create_motion_tween", &json!({"layer": f["layer"]})).is_err());
}

#[test]
fn copied_and_split_spans_keep_their_motion() {
    let mut s = tweened();
    run(&mut s, "motion.set_key", json!({"property": "x", "frame": 23, "value": 235}));
    let want: Vec<(f32, f32)> = (0..24).map(|f| pos(&mut s, f)).collect();
    // Copy the span, paste it after itself: the pasted copy tweens its own (fresh) object.
    run(&mut s, "timeline.copy_frames", json!({"frame": 0, "count": 24}));
    run(&mut s, "timeline.paste_frames", json!({"frame": 24}));
    let k = &layer(&s).keyframes[1];
    let m = k.motion.as_ref().expect("pasted motion");
    assert_eq!(m.target, k.elements[0].id);
    assert_ne!(k.elements[0].id, layer(&s).keyframes[0].elements[0].id);
    for f in 0..24 {
        assert_eq!(pos(&mut s, 24 + f), want[f as usize], "pasted frame {f}");
    }
    // Splitting the span (a cut across it) keeps the motion on both sides.
    run(&mut s, "timeline.copy_frames", json!({"frame": 6, "count": 4}));
    let clip = s.frame_clipboard.layers[0].keys.clone();
    assert!(clip.first().is_some_and(|k| k.motion.is_some()));
    let mut l = layer(&s).clone();
    cmds_timeline::insert_keyframe_at(&mut l, 9, true).expect("split");
    for f in 0..24 {
        let e = evaluate_layer_elements(&l, f).map(|(_, e)| e[0].transform.translation()).unwrap_or_default();
        assert!((e.x - want[f as usize].0).abs() < 1e-3, "split frame {f}");
    }
}

#[test]
fn motion_tweens_round_trip_through_the_file_format() {
    let mut s = tweened();
    s.set_frame(12);
    run(&mut s, "element.translate", json!({"dx": 40, "dy": 30}));
    run(&mut s, "motion.set_ease", json!({"frame": 4, "easing": {"bezier": [0.1, 0.9, 0.3, 1.2]}}));
    let text = format::to_json(&s.doc).expect("save");
    assert!(text.contains("\"motion\"") && text.contains("\"scale_x\"") == motion(&s).track(Property::ScaleX).is_some());
    let back = format::from_json(&text).expect("load");
    assert_eq!(back, s.doc);
    let mut t = Session::new(back);
    for f in 0..24 {
        assert_eq!(pos(&mut t, f), pos(&mut s, f));
    }
    // Duplicating a scene re-points the tween at the copied object.
    run(&mut s, "scene.duplicate", json!({}));
    let k = &s.doc.timeline(1).layers[0].keyframes[0];
    assert_eq!(k.motion.as_ref().map(|m| m.target), Some(k.elements[0].id));
}
