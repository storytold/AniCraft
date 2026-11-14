//! Eases through the command registry (ADR 0081): presets by name, custom eases edited point by
//! point (one undo step per drag), saved eases on classic tweens and motion properties, copy
//! and paste, property groups, and the file round trip.

use super::*;
use anicraft_doc::{CustomEase, EaseGroup, Easing, Property, TweenKind};
use serde_json::json;

pub(crate) fn run(s: &mut Session, id: &str, p: Value) -> Value {
    match s.execute(id, &p) {
        Ok(v) => v,
        Err(e) => panic!("{id} {p}: {e}"),
    }
}

/// A 10 × 10 square at the origin, tweened (classic) 100 to the right over ten frames.
pub(crate) fn tweened() -> Session {
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 10, "h": 10}));
    run(&mut s, "timeline.goto", json!({"frame": 10}));
    run(&mut s, "timeline.insert_keyframe", json!({}));
    run(&mut s, "edit.select_all", json!({}));
    run(&mut s, "element.translate", json!({"dx": 100, "dy": 0}));
    run(&mut s, "timeline.create_tween", json!({"frame": 0}));
    s
}

/// How far the square has moved at frame `f` (it starts centred on x = 5).
pub(crate) fn x_at(s: &mut Session, f: u32) -> f32 {
    s.set_frame(f);
    s.visible_elements().first().map(|e| e.transform.translation().x - 5.0).unwrap_or(f32::NAN)
}

fn tween(s: &Session) -> anicraft_doc::Tween {
    s.active_layer().and_then(|l| l.keyframes.first()).and_then(|k| k.tween.clone()).expect("a tween")
}

#[test]
fn every_preset_applies_by_name_and_drives_the_tween() {
    let mut s = tweened();
    for e in Easing::named() {
        let name = e.name().unwrap();
        let v = run(&mut s, "timeline.set_easing", json!({"easing": name, "frame": 0}));
        assert_eq!(v["easing"], e.label());
        assert_eq!(tween(&s).easing, e);
        assert!((x_at(&mut s, 5) - 100.0 * e.apply(0.5)).abs() < 1e-3, "{name}");
    }
    run(&mut s, "timeline.set_easing", json!({"easing": {"classic": 100}, "frame": 0}));
    assert!((x_at(&mut s, 5) - 75.0).abs() < 1e-3);
    // The list: every named preset and Classic Ease.
    let l = run(&mut s, "ease.list", json!({}));
    assert_eq!(l["presets"].as_array().unwrap().len(), 29);
    assert!(s.execute("timeline.set_easing", &json!({"easing": "Wobbly"})).is_err());
}

#[test]
fn point_edits_shape_a_custom_ease_and_a_drag_is_one_undo_step() {
    let mut s = tweened();
    // A point on the line keeps the motion as it was; the preset became a custom curve.
    let v = run(&mut s, "ease.add_point", json!({"t": 0.5, "frame": 0}));
    assert_eq!((v["index"].as_u64(), v["points"].as_u64()), (Some(1), Some(3)));
    assert!(matches!(tween(&s).easing, Easing::Custom(_)));
    assert!((x_at(&mut s, 3) - 30.0).abs() < 1e-2);
    // Dragging it up and past its neighbour, as a gesture of many moves: one undo step.
    let steps = s.history_len();
    s.begin_gesture("Ease point");
    for k in 0..=20 {
        run(&mut s, "ease.move_point", json!({"index": 1, "x": 0.5 + k as f32 * 0.05, "y": 0.5 + k as f32 * 0.05, "frame": 0}));
    }
    s.end_gesture();
    assert_eq!(s.history_len(), steps + 1);
    let c = tween(&s).easing.custom().cloned().unwrap();
    assert_eq!(c.points()[1].at, anicraft_geom::Vec2::new(1.0, 1.5), "x stops at the end; y overshoots");
    assert!(x_at(&mut s, 9) > 100.0, "overshoots past the second keyframe");
    // Corner, smooth, handles, delete; the ends can't move or go.
    run(&mut s, "ease.move_point", json!({"index": 1, "x": 0.4, "y": 1.2, "frame": 0}));
    run(&mut s, "ease.convert_point", json!({"index": 1, "smooth": true, "frame": 0}));
    run(&mut s, "ease.move_handle", json!({"index": 1, "handle": "out", "x": 0.6, "y": 1.4, "frame": 0}));
    let p = tween(&s).easing.custom().unwrap().points()[1];
    assert!(p.smooth && (p.h_in.normalized().dot(p.out.normalized()) + 1.0).abs() < 1e-4);
    assert!(s.execute("ease.move_point", &json!({"index": 0, "x": 0.5, "y": 0.5, "frame": 0})).is_err());
    assert!(s.execute("ease.delete_point", &json!({"index": 2, "frame": 0})).is_err());
    assert!(s.execute("ease.move_handle", &json!({"index": 0, "handle": "in", "x": 0, "y": 0, "frame": 0})).is_err());
    run(&mut s, "ease.delete_point", json!({"index": 1, "frame": 0}));
    assert_eq!(tween(&s).easing.custom().unwrap().points().len(), 2);
    // Undo walks back through the edits.
    s.undo().unwrap();
    assert_eq!(tween(&s).easing.custom().unwrap().points().len(), 3);
    // Info gives the curve as points and the ease as params take it back.
    let i = run(&mut s, "ease.info", json!({"frame": 0}));
    assert_eq!(i["kind"], "custom");
    assert_eq!(i["points"].as_array().unwrap().len(), 3);
    let back = run(&mut s, "timeline.set_easing", json!({"easing": i["easing"].clone(), "frame": 0}));
    assert_eq!(back["easing"], "Custom");
}

#[test]
fn time_never_runs_backwards_after_any_command_edit() {
    let mut s = tweened();
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut rnd = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % 10_000) as f64 / 10_000.0
    };
    for _ in 0..400 {
        let n = tween(&s).easing.to_custom().points().len() as f64;
        let i = (rnd() * n) as u64;
        let (x, y) = (rnd() * 1.6 - 0.3, rnd() * 30.0 - 15.0);
        let (id, p) = match (rnd() * 5.0) as u32 {
            0 => ("ease.add_point", json!({"t": rnd(), "frame": 0})),
            1 => ("ease.move_point", json!({"index": i, "x": x, "y": y, "frame": 0})),
            2 => ("ease.move_handle", json!({"index": i, "handle": if rnd() > 0.5 { "in" } else { "out" }, "x": x, "y": y, "frame": 0})),
            3 => ("ease.convert_point", json!({"index": i, "smooth": rnd() > 0.5, "frame": 0})),
            _ => ("ease.delete_point", json!({"index": i, "frame": 0})),
        };
        let _ = s.execute(id, &p);
        let c = tween(&s).easing.to_custom();
        for k in 0..c.points().len() - 1 {
            let [a, b, cc, d] = c.segment(k).unwrap();
            assert!(a.x <= d.x && (a.x..=d.x).contains(&b.x) && (a.x..=d.x).contains(&cc.x), "{id} {p}: segment {k}");
        }
        assert!((0..=10).all(|f| x_at(&mut s, f).is_finite()));
    }
}

#[test]
fn saved_eases_reach_classic_tweens_and_motion_properties() {
    let mut s = tweened();
    run(&mut s, "ease.add_point", json!({"t": 0.3, "frame": 0}));
    run(&mut s, "ease.move_point", json!({"index": 1, "x": 0.3, "y": 0.9, "frame": 0}));
    let curve = tween(&s).easing.clone();
    run(&mut s, "ease.save", json!({"name": "  Snap  ", "frame": 0}));
    assert_eq!(s.doc.eases.len(), 1);
    assert_eq!(s.doc.eases[0].name, "Snap");
    assert!(s.execute("ease.save", &json!({"name": "snap", "easing": "QuadIn"})).is_err(), "a name is saved once (any case)");
    run(&mut s, "ease.save", json!({"name": "Bouncy", "easing": "BounceOut"}));
    // Another tween takes it by name.
    run(&mut s, "timeline.set_easing", json!({"easing": "Linear", "frame": 0}));
    run(&mut s, "timeline.set_easing", json!({"easing": {"saved": "snap"}, "frame": 0}));
    assert_eq!(tween(&s).easing, curve);
    assert_eq!(cmds_ease::saved_name(&s.doc, &tween(&s).easing), Some("Snap"));
    // A motion tween's property too.
    let mut m = Session::default();
    m.doc.eases = s.doc.eases.clone();
    run(&mut m, "draw.rect", json!({"x": 0, "y": 0, "w": 10, "h": 10}));
    run(&mut m, "timeline.create_motion_tween", json!({}));
    run(&mut m, "motion.set_key", json!({"property": "x", "frame": 20, "value": 205}));
    run(&mut m, "motion.set_ease", json!({"property": "x", "frame": 3, "easing": {"saved": "Bouncy"}}));
    let ease = m.active_layer().unwrap().keyframes[0].motion.as_ref().unwrap().track(Property::X).unwrap().keys[0].ease.clone();
    assert_eq!(ease, Easing::BounceOut);
    // Point edits on a motion segment address it by property.
    run(&mut m, "ease.add_point", json!({"property": "x", "frame": 3, "t": 0.5}));
    assert!(matches!(m.active_layer().unwrap().keyframes[0].motion.as_ref().unwrap().track(Property::X).unwrap().keys[0].ease, Easing::Custom(_)));
    // Edit the saved one directly, rename, delete: tweens keep their curves.
    run(&mut s, "ease.set", json!({"saved": "Snap", "easing": "SineOut"}));
    assert_eq!(s.doc.eases[0].ease, Easing::SineOut);
    assert_eq!(tween(&s).easing, curve, "a tween keeps the curve it was given");
    run(&mut s, "ease.rename", json!({"name": "snap", "to": "Settle"}));
    assert!(s.execute("ease.rename", &json!({"name": "Settle", "to": "bouncy"})).is_err());
    run(&mut s, "ease.delete", json!({"name": "SETTLE"}));
    assert_eq!(s.doc.eases.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["Bouncy"]);
    assert!(s.execute("timeline.set_easing", &json!({"easing": {"saved": "Settle"}})).is_err());
    let l = run(&mut s, "ease.list", json!({}));
    assert_eq!(l["saved"][0]["name"], "Bouncy");
}

#[test]
fn copy_and_paste_an_ease_between_tweens() {
    let mut s = tweened();
    run(&mut s, "timeline.set_easing", json!({"easing": "BackOut", "frame": 0}));
    assert!(s.execute("ease.paste", &json!({"frame": 0})).is_err(), "nothing copied yet");
    run(&mut s, "ease.copy", json!({"frame": 0}));
    run(&mut s, "timeline.set_easing", json!({"easing": "Linear", "frame": 0}));
    let steps = s.history_len();
    run(&mut s, "ease.paste", json!({"frame": 0, "group": "rotation"}));
    assert_eq!(s.history_len(), steps + 1);
    assert_eq!(tween(&s).ease(EaseGroup::Rotation), &Easing::BackOut);
    // Copying doesn't change the document.
    let steps = s.history_len();
    run(&mut s, "ease.copy", json!({"frame": 0}));
    assert_eq!(s.history_len(), steps);
}

#[test]
fn classic_tweens_ease_property_groups_separately() {
    let mut s = tweened();
    run(&mut s, "timeline.set_easing", json!({"easing": "QuadIn", "group": "position", "frame": 0}));
    assert!((x_at(&mut s, 5) - 25.0).abs() < 1e-3, "position eased in");
    assert_eq!(tween(&s).ease(EaseGroup::Scale), &Easing::Linear);
    // Separate: every group gets the tween's ease (position keeps its own); one for all clears.
    run(&mut s, "ease.separate", json!({"separate": true, "frame": 0}));
    assert_eq!(tween(&s).groups.len(), 5);
    assert_eq!(tween(&s).ease(EaseGroup::Position), &Easing::QuadIn);
    run(&mut s, "ease.separate", json!({"separate": false, "frame": 0}));
    assert!(tween(&s).groups.is_empty());
    assert!((x_at(&mut s, 5) - 50.0).abs() < 1e-3);
    // Converting keeps groups on classic tweens; a shape tween has one ease.
    run(&mut s, "timeline.set_easing", json!({"easing": "QuadIn", "group": "color", "frame": 0}));
    run(&mut s, "timeline.create_shape_tween", json!({"frame": 0}));
    assert!(tween(&s).groups.is_empty() && tween(&s).kind == TweenKind::Shape);
    assert!(s.execute("timeline.set_easing", &json!({"easing": "QuadIn", "group": "color", "frame": 0})).is_err());
    assert!(s.execute("ease.separate", &json!({"separate": true, "frame": 0})).is_err());
    assert!(s.execute("timeline.set_easing", &json!({"easing": "QuadIn", "group": "sparkle", "frame": 0})).is_err());
}

#[test]
fn eases_survive_the_file_round_trip() {
    let mut s = tweened();
    run(&mut s, "timeline.set_easing", json!({"easing": {"custom": [[0, 0], [0.2, 1.4], [0.3, 1.4], [0.4, 1.4], [0.6, 0.8], [0.8, 1], [1, 1]]}, "frame": 0}));
    run(&mut s, "timeline.set_easing", json!({"easing": "ElasticOut", "group": "rotation", "frame": 0}));
    run(&mut s, "ease.save", json!({"name": "Wave", "frame": 0}));
    run(&mut s, "ease.save", json!({"name": "Classic 40", "easing": {"classic": 40}}));
    let text = format::to_json(&s.doc).expect("save");
    let back = format::from_json(&text).expect("load");
    assert_eq!(back, s.doc);
    assert_eq!(back.eases.len(), 2);
    let t = back.timeline(0).layers[0].keyframes[0].tween.clone().unwrap();
    assert!(matches!(&t.easing, Easing::Custom(c) if c.points().len() == 3));
    assert_eq!(t.ease(EaseGroup::Rotation), &Easing::ElasticOut);
    // A file from before saved eases and custom curves opens unchanged.
    let old = text.replace("\"eases\":", "\"ignored_eases\":");
    assert!(format::from_json(&old).expect("older file").eases.is_empty());
    // A hostile curve in a file is repaired on load, never refused: reversed, out of range.
    let bad = r#"{"Custom": {"points": [{"at": {"x": 2, "y": 99}}, {"at": {"x": 0.5, "y": -40}, "in": {"x": 5, "y": 0}}, {"at": {"x": -1, "y": 0}}]}}"#;
    let e: Easing = serde_json::from_str(bad).unwrap();
    let c = e.custom().unwrap();
    assert_eq!((c.points().len(), c.points()[1].at.y, c.points()[1].h_in.x), (3, -anicraft_doc::MAX_EASE_Y, 0.0));
    let _ = CustomEase::linear();
}
