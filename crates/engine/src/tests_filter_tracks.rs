//! Motion tween tracks for filters (ADR 0092), through the command registry: keys, eases and
//! retiming on a filter's numbers, auto-key when a filter is edited inside the tween, tracks
//! following their filter, F6, undo and the file round trip.

use super::*;
use anicraft_doc::{ElementKind, FilterEntry, FilterParam, FilterProp, Property};
use serde_json::json;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    match s.execute(id, &p) {
        Ok(v) => v,
        Err(e) => panic!("{id} {p}: {e}"),
    }
}

/// A red 100 × 100 movie clip at (100, 100) with `filters`, selected, in a one-second motion
/// tween.
fn tweened(filters: Value) -> Session {
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 100, "y": 100, "w": 100, "h": 100, "fill": "#ff0000", "stroke": null}));
    run(&mut s, "modify.convert_to_symbol", json!({"name": "Box"}));
    run(&mut s, "symbol.set_kind", json!({"kind": "movie_clip"}));
    run(&mut s, "element.set_filters", json!({ "filters": filters }));
    run(&mut s, "timeline.create_motion_tween", json!({}));
    s
}

/// The object's filters as the base holds them.
fn base_list(s: &Session) -> Vec<FilterEntry> {
    match &s.doc.timeline(s.scene).layers[0].keyframes[0].elements[0].kind {
        ElementKind::Symbol { filters, .. } => filters.clone(),
        _ => Vec::new(),
    }
}

fn value(s: &mut Session, frame: u32, key: &str) -> f64 {
    let v = run(s, "motion.info", json!({ "frame": frame }));
    v["values"][key].as_f64().unwrap_or_else(|| panic!("no {key} in {}", v["values"]))
}

fn track_keys(s: &mut Session, key: &str) -> Vec<(u64, f64)> {
    let v = run(s, "motion.info", json!({}));
    let t = v["tracks"].as_array().and_then(|ts| ts.iter().find(|t| t["property"] == key).cloned());
    t.map(|t| t["keys"].as_array().map(|ks| ks.iter().map(|k| (k["frame"].as_u64().unwrap_or(0), k["value"].as_f64().unwrap_or(0.0))).collect()).unwrap_or_default()).unwrap_or_default()
}

/// The green of a pixel: 1 on the white stage, less where the red box's blur reaches.
fn green_at(s: &Session, frame: u32, x: u32, y: u32) -> f32 {
    anicraft_render::render_frame(&s.doc, frame, 1.0).unwrap().pixel(x, y).unwrap()[1]
}

#[test]
fn a_blur_tweens_by_commands_with_eases_and_retiming() {
    let mut s = tweened(json!([{"type": "blur", "blur_x": 0, "blur_y": 0}]));
    run(&mut s, "motion.set_key", json!({"property": "filter.0.blur_x", "frame": 12, "value": 40}));
    run(&mut s, "motion.set_key", json!({"property": "filter.0.blur_y", "frame": 12, "value": 40}));
    assert_eq!(track_keys(&mut s, "filter.0.blur_x"), vec![(0, 0.0), (12, 40.0)]);
    assert_eq!(value(&mut s, 6, "filter.0.blur_x"), 20.0);
    let info = run(&mut s, "motion.info", json!({}));
    let t = info["tracks"].as_array().unwrap().iter().find(|t| t["property"] == "filter.0.blur_x").unwrap().clone();
    assert_eq!((t["label"].as_str(), t["unit"].as_str(), &t["filter"]), (Some("Blur X"), Some("px"), &json!({"slot": 0, "kind": "Blur"})));
    // The render follows: at frame 0 the box's edge is sharp, at frame 12 the blur spills out.
    assert!((green_at(&s, 0, 95, 150) - 1.0).abs() < 1e-6);
    assert!(green_at(&s, 12, 95, 150) < 0.95);
    // Eases like any property's segment.
    run(&mut s, "motion.set_ease", json!({"property": "filter.0.blur_x", "frame": 3, "easing": "CubicOut"}));
    assert!(value(&mut s, 6, "filter.0.blur_x") > 30.0, "cubic out is past halfway at half time");
    // Retimed: the key moves with its value and ease.
    let v = run(&mut s, "motion.move_keys", json!({"keys": [{"property": "filter.0.blur_x", "frame": 12}], "by": 4}));
    assert_eq!(v["keys"], json!([{"property": "filter.0.blur_x", "frame": 16}]));
    assert_eq!(value(&mut s, 16, "filter.0.blur_x"), 40.0);
    // Removed.
    let v = run(&mut s, "motion.remove_key", json!({"property": "filter.0.blur_y", "frame": 12}));
    assert_eq!(v["removed"], json!(["filter.0.blur_y"]));
    // What can't be keyed says why.
    for (p, want) in [(json!({"property": "filter.1.blur_x", "value": 3}), "no filter 1"), (json!({"property": "filter.0.angle", "value": 3}), "has no Angle"), (json!({"property": "filter.0.blur_x", "value": 900}), "between"), (json!({"property": "filter.0.sparkle", "value": 1}), "unknown property")] {
        let e = s.execute("motion.set_key", &p).expect_err("refused").to_string();
        assert!(e.contains(want), "{p}: {e}");
    }
}

#[test]
fn editing_a_filter_inside_the_tween_keys_it_and_undo_takes_it_back() {
    let mut s = tweened(json!([{"type": "drop_shadow", "distance": 4}]));
    let steps = s.history_len();
    s.set_frame(10);
    run(&mut s, "element.set_filters", json!({"filters": [{"type": "drop_shadow", "distance": 24}]}));
    assert_eq!(s.history_len(), steps + 1, "one undo step");
    assert_eq!(track_keys(&mut s, "filter.0.distance"), vec![(0, 4.0), (10, 24.0)]);
    assert!(track_keys(&mut s, "filter.0.blur_x").is_empty(), "only the number that changed is keyed");
    // The object keeps its own distance underneath; the curve gives the rest.
    assert_eq!(base_list(&s)[0].filter.param(FilterParam::Distance), Some(4.0));
    assert_eq!(value(&mut s, 5, "filter.0.distance"), 14.0);
    // Switching the filter off keeps the tween (the list keeps its shape).
    run(&mut s, "filter.enable", json!({"enabled": false, "index": 0}));
    assert!(!base_list(&s)[0].enabled && track_keys(&mut s, "filter.0.distance").len() == 2);
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert!(track_keys(&mut s, "filter.0.distance").is_empty());
    // On the span's first frame an untracked number changes the object itself.
    s.set_frame(0);
    run(&mut s, "element.set_filters", json!({"filters": [{"type": "drop_shadow", "distance": 9}]}));
    assert!(track_keys(&mut s, "filter.0.distance").is_empty());
    assert_eq!(base_list(&s)[0].filter.param(FilterParam::Distance), Some(9.0));
}

#[test]
fn tracks_follow_their_filter_when_the_list_is_reordered_or_cut() {
    let mut s = tweened(json!([{"type": "blur"}, {"type": "glow", "strength": 1}]));
    run(&mut s, "motion.set_key", json!({"property": "filter.1.strength", "frame": 10, "value": 300}));
    run(&mut s, "motion.set_key", json!({"property": "filter.0.blur_x", "frame": 10, "value": 30}));
    // Reordered at a frame inside the tween: the glow goes first, its track with it, and the
    // object's own values stay underneath (not the playhead's).
    s.set_frame(5);
    run(&mut s, "filter.move", json!({"from": 1, "to": 0}));
    assert_eq!(track_keys(&mut s, "filter.0.strength"), vec![(0, 100.0), (10, 300.0)]);
    assert_eq!(track_keys(&mut s, "filter.1.blur_x"), vec![(0, 5.0), (10, 30.0)]);
    let base = base_list(&s);
    assert_eq!((base[0].filter.label(), base[0].filter.param(FilterParam::Strength)), ("Glow", Some(100.0)));
    assert_eq!(base[1].filter.param(FilterParam::BlurX), Some(5.0));
    assert_eq!(value(&mut s, 10, "filter.0.strength"), 300.0);
    // The glow removed: its track goes; the blur's moves up.
    run(&mut s, "filter.remove", json!({"index": 0}));
    assert!(track_keys(&mut s, "filter.0.strength").is_empty());
    assert_eq!(track_keys(&mut s, "filter.0.blur_x"), vec![(0, 5.0), (10, 30.0)]);
    // A filter pasted after it leaves the tracks where they are.
    run(&mut s, "filter.copy", json!({}));
    run(&mut s, "filter.paste", json!({}));
    assert_eq!(base_list(&s).len(), 2);
    assert_eq!(track_keys(&mut s, "filter.0.blur_x").len(), 2);
}

#[test]
fn f6_keys_the_filters_and_the_file_keeps_their_tracks() {
    let mut s = tweened(json!([{"type": "adjust_color", "hue": 0}]));
    run(&mut s, "motion.set_key", json!({"property": "filter.0.hue", "frame": 20, "value": 120}));
    s.set_frame(10);
    run(&mut s, "timeline.insert_keyframe", json!({}));
    let hue = track_keys(&mut s, "filter.0.hue");
    assert!(hue.contains(&(10, 60.0)), "{hue:?}");
    assert_eq!(track_keys(&mut s, "filter.0.saturation"), vec![(0, 0.0), (10, 0.0)]);
    // The document round-trips through JSON (the file format), tracks and all.
    let json = serde_json::to_string(&s.doc).unwrap();
    assert!(json.contains("\"filter_tracks\"") && json.contains("filter.0.hue"));
    let back: anicraft_doc::Document = serde_json::from_str(&json).unwrap();
    assert!(back == s.doc);
    let m = back.timeline(0).layers[0].keyframes[0].motion.clone().unwrap();
    assert!(m.track(Property::Filter(FilterProp { slot: 0, param: FilterParam::Hue })).is_some());
}
