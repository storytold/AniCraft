//! Bones, second slice (ADR 0084): joint translation, springs and editing armature members in
//! place, end to end through the commands.

use super::*;
use crate::tests_bone::{ARM, HAND, ROOT, rigged};
use anicraft_doc::{BoneId, SymbolKind, evaluate};
use anicraft_geom::Affine;
use serde_json::json;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    match s.execute(id, &p) {
        Ok(v) => v,
        Err(e) => panic!("{id} {p}: {e}"),
    }
}

fn bone_at(s: &mut Session, frame: u32, bone: u64) -> Value {
    let v = run(s, "bone.info", json!({"frame": frame}));
    v["armatures"][0]["bones"].as_array().unwrap().iter().find(|b| b["id"] == bone).cloned().unwrap()
}

fn num(v: &Value) -> f32 {
    v.as_f64().unwrap() as f32
}

/// Where element `id` is drawn at movie frame `frame` (what every export sees).
fn drawn(s: &Session, frame: u32, id: u64) -> Affine {
    evaluate(&s.doc, frame).items.iter().find(|i| i.element.0 == id).map(|i| i.transform).unwrap()
}

#[test]
fn a_sliding_joint_poses_within_its_limits() {
    let (mut s, b1, b2) = rigged();
    let v = run(&mut s, "bone.set_translation", json!({"bone": b2, "axis": "x", "min": 0, "max": 60}));
    assert_eq!((v["enabled"].clone(), v["constrain"].clone()), (json!(true), json!(true)));
    // Out of reach of the turning joints: the elbow slides out, no farther than its limit.
    let r = run(&mut s, "bone.solve", json!({"bone": b2, "x": 450, "y": 100}));
    assert_eq!(r["reached"], true, "{r}");
    let slide = bone_at(&mut s, 0, b2)["slide"].clone();
    assert!((num(&slide[0]) - 50.0).abs() < 0.5 && num(&slide[1]) == 0.0, "{slide}");
    let r = run(&mut s, "bone.solve", json!({"bone": b2, "x": 700, "y": 100}));
    assert_eq!(r["reached"], false);
    assert_eq!(num(&bone_at(&mut s, 0, b2)["slide"][0]), 60.0, "stops at the limit");
    // Within reach, it turns as before and doesn't slide.
    run(&mut s, "bone.pose", json!({"bone": b2, "slide_x": 0}));
    run(&mut s, "bone.solve", json!({"bone": b2, "x": 320, "y": 160}));
    assert_eq!(num(&bone_at(&mut s, 0, b2)["slide"][0]), 0.0);
    // Posing by number keeps to the limits; a joint that doesn't slide refuses.
    assert_eq!(run(&mut s, "bone.pose", json!({"bone": b2, "slide_x": 500}))["slide"], json!([60.0, 0.0]));
    assert!(s.execute("bone.pose", &json!({"bone": b1, "slide_x": 5})).unwrap_err().to_string().contains("translation"));
    // The hand moved with its sliding joint: the elbow is 60 px along the arm.
    let elbow = bone_at(&mut s, 0, b2)["head"].clone();
    let shoulder = bone_at(&mut s, 0, b1)["head"].clone();
    let d = Vec2::new(num(&elbow[0]) - num(&shoulder[0]), num(&elbow[1]) - num(&shoulder[1])).length();
    assert!((d - 160.0).abs() < 1e-2, "{d}");
    // Narrowing the limits brings every pose inside; turning it off slides nothing.
    run(&mut s, "bone.set_translation", json!({"bone": b2, "axis": "x", "max": 20}));
    assert_eq!(num(&bone_at(&mut s, 0, b2)["slide"][0]), 20.0);
    run(&mut s, "bone.set_translation", json!({"bone": b2, "axis": "x", "enabled": false}));
    assert_eq!(bone_at(&mut s, 0, b2)["slide"], json!([0.0, 0.0]));
    assert_eq!(bone_at(&mut s, 0, b2)["slide_x"], Value::Null);
    // Free sliding, and the refusals.
    assert_eq!(run(&mut s, "bone.set_translation", json!({"bone": b2, "axis": "y", "enabled": true}))["constrain"], false);
    for p in [json!({"bone": b2, "axis": "z", "enabled": true}), json!({"bone": b2, "axis": "x", "min": 10, "max": -10}), json!({"bone": b2, "axis": "x", "min": -1e9}), json!({"bone": b2, "axis": "x"})] {
        assert!(s.execute("bone.set_translation", &p).is_err(), "{p}");
    }
}

/// [`rigged`] with the body (the root) dropping 150 px from frame 0 to frame 8, then holding,
/// and both bones springy.
fn springy() -> (Session, u64, u64) {
    let (mut s, b1, b2) = rigged();
    run(&mut s, "bone.pose", json!({"dx": 0, "dy": 150, "frame": 8}));
    for b in [b1, b2] {
        run(&mut s, "bone.set_spring", json!({"bone": b, "strength": 40, "damping": 25}));
    }
    (s, b1, b2)
}

#[test]
fn springs_swing_every_export_and_never_change_the_keyed_poses() {
    let (mut s, b1, _) = springy();
    let keyed: Vec<_> = s.doc.timeline(0).layers[0].keyframes.iter().map(|k| k.pose.clone()).collect();
    // Mid-swing: the hand is drawn away from where the keyed pose has it.
    let swing = num(&bone_at(&mut s, 12, b1)["swing"]);
    assert!(swing.abs() > 1.0, "{swing}°");
    let mut still = s.doc.clone();
    Arc::make_mut(Arc::make_mut(&mut still.timeline_mut(0).layers[0]).armature.as_mut().unwrap()).springs_off = true;
    let hand = |d: &anicraft_doc::Document, f| evaluate(d, f).items.iter().find(|i| i.element.0 == HAND).map(|i| i.transform.translation()).unwrap();
    assert!(hand(&s.doc, 12).distance(hand(&still, 12)) > 1.0);
    // The keyed poses (and what commands read) are untouched.
    assert_eq!(s.doc.timeline(0).layers[0].keyframes.iter().map(|k| k.pose.clone()).collect::<Vec<_>>(), keyed);
    assert_eq!(bone_at(&mut s, 12, b1)["angle"], json!(0.0));
    // A raster render is the display list: the swung hand shows in the pixels.
    let (a, b) = (anicraft_render::render_frame(&s.doc, 12, 1.0).unwrap(), anicraft_render::render_frame(&still, 12, 1.0).unwrap());
    assert_ne!(a.data, b.data);
    // Turning the springs off plays the keyed poses exactly.
    run(&mut s, "bone.enable_springs", json!({"bone": b1, "enabled": false}));
    assert_eq!(drawn(&s, 12, HAND), evaluate(&still, 12).items.iter().find(|i| i.element.0 == HAND).unwrap().transform);
    assert_eq!(bone_at(&mut s, 12, b1)["swing"], json!(0.0));
    assert_eq!(run(&mut s, "bone.info", json!({}))["armatures"][0]["springs"], false);
    s.undo().unwrap();
    assert!(drawn(&s, 12, HAND) != evaluate(&still, 12).items.iter().find(|i| i.element.0 == HAND).unwrap().transform, "undo turns them back on");
}

#[test]
fn seeking_any_frame_draws_what_playing_draws() {
    let (s, _, _) = springy();
    let bits = |s: &Session, f: u32| evaluate(&s.doc, f).items.iter().map(|i| i.transform.m.map(f32::to_bits)).collect::<Vec<_>>();
    let played: Vec<_> = (0..20).map(|f| bits(&s, f)).collect();
    for f in [19, 3, 12, 0, 12, 7, 18, 1] {
        assert_eq!(bits(&s, f), played[f as usize], "frame {f}");
    }
}

#[test]
fn damping_one_hundred_is_no_spring_and_spring_values_are_checked() {
    let (mut s, b1, b2) = springy();
    let mut plain = rigged().0;
    run(&mut plain, "bone.pose", json!({"dx": 0, "dy": 150, "frame": 8}));
    for b in [b1, b2] {
        run(&mut s, "bone.set_spring", json!({"bone": b, "damping": 100}));
    }
    for f in 0..20 {
        assert_eq!(drawn(&s, f, HAND), drawn(&plain, f, HAND), "frame {f}");
    }
    let v = run(&mut s, "bone.info", json!({}));
    assert_eq!(v["armatures"][0]["bones"][0]["spring"], json!({"strength": 40.0, "damping": 100.0}));
    for p in [json!({"bone": b1, "strength": 101}), json!({"bone": b1, "damping": -1}), json!({"bone": b1}), json!({"bone": 999_999, "strength": 5})] {
        assert!(s.execute("bone.set_spring", &p).is_err(), "{p}");
    }
    // Strength and damping both 0: no spring left in the file.
    run(&mut s, "bone.set_spring", json!({"bone": b1, "strength": 0, "damping": 0}));
    assert_eq!(s.doc.timeline(0).layers[0].armature.as_ref().unwrap().bone(BoneId(b1)).unwrap().spring, None);
}

#[test]
fn springs_and_slides_survive_the_file() {
    let (mut s, _, b2) = springy();
    run(&mut s, "bone.set_translation", json!({"bone": b2, "axis": "y", "min": -30, "max": 30}));
    run(&mut s, "bone.pose", json!({"bone": b2, "slide_y": 12, "frame": 4}));
    let text = crate::format::to_json(&s.doc).unwrap();
    let back = crate::format::from_json(&text).unwrap();
    assert_eq!(back, s.doc);
    let b = back.timeline(0).layers[0].armature.as_ref().unwrap().bone(BoneId(b2)).cloned().unwrap();
    assert_eq!(b.slide_y, Some(anicraft_doc::Limits { min: -30.0, max: 30.0 }));
    assert!(b.spring.is_some());
}

#[test]
fn a_members_colour_effect_filters_and_blend_edit_in_place_on_every_pose() {
    let (mut s, b1, _) = rigged();
    run(&mut s, "bone.pose", json!({"bone": b1, "angle": 40, "frame": 10}));
    // Blending and filters are for movie clips, as in Animate.
    let sym = s.doc.library[0].id;
    Arc::make_mut(s.doc.library.iter_mut().find(|x| x.id == sym).unwrap()).kind = SymbolKind::MovieClip;
    let lid = s.doc.timeline(0).layers[0].id;
    let poses_before: Vec<_> = s.doc.timeline(0).layers[0].keyframes.iter().map(|k| k.pose.clone()).collect();
    run(&mut s, "element.select", json!({"ids": [ARM], "layer": lid.0}));
    let steps = s.history_len();
    run(&mut s, "element.set_color_effect", json!({"style": "tint", "tint": "#ff0000", "tint_amount": 50}));
    run(&mut s, "element.set_blend", json!({"mode": "multiply"}));
    run(&mut s, "element.set_filters", json!({"filters": [{"type": "glow"}]}));
    assert_eq!(s.history_len(), steps + 3, "one undo step each");
    let l = s.doc.timeline(0).layers[0].clone();
    assert_eq!(l.keyframes.len(), 2, "no frame keyed: the member changed on every pose");
    for k in &l.keyframes {
        let e = k.elements.iter().find(|e| e.id.0 == ARM).unwrap();
        assert!(matches!(e.color, anicraft_doc::ColorEffect::Tint { .. }), "{:?}", e.color);
        let ElementKind::Symbol { blend, filters, .. } = &e.kind else { panic!() };
        assert_eq!((*blend, filters.len()), (anicraft_doc::BlendMode::Multiply, 1));
        // The others are untouched.
        assert_eq!(k.elements.iter().find(|e| e.id.0 == HAND).unwrap().color, anicraft_doc::ColorEffect::default());
    }
    assert_eq!(l.keyframes.iter().map(|k| k.pose.clone()).collect::<Vec<_>>(), poses_before, "the armature is intact");
    // In the tween between the poses the member keeps its look and still moves with its bone.
    let mid = evaluate(&s.doc, 5);
    let arm = mid.items.iter().find(|i| i.element.0 == ARM).unwrap();
    assert!(arm.transform.translation().distance(Vec2::new(200.0, 100.0)) < 1e-3);
    // Moving it is still the bones' job.
    assert!(s.execute("element.translate", &json!({"dx": 5, "dy": 0})).unwrap_err().to_string().contains("armature"));
    s.undo().unwrap();
    s.undo().unwrap();
    s.undo().unwrap();
    assert_eq!(s.doc.timeline(0).layers[0].keyframes[1].elements.iter().find(|e| e.id.0 == ARM).unwrap().color, anicraft_doc::ColorEffect::default());
    // A locked armature layer refuses.
    run(&mut s, "element.select", json!({"ids": [ROOT], "layer": lid.0}));
    run(&mut s, "layer.toggle_lock", json!({"layer": lid.0}));
    assert!(s.execute("element.set_alpha", &json!({"alpha": 0.5})).is_err());
}

#[test]
fn plug_ins_may_set_springs_and_translation() {
    for id in ["bone.set_translation", "bone.set_spring", "bone.enable_springs"] {
        assert!(crate::cmds_plugin::plugin_may_call(id).is_ok(), "{id}");
    }
}
