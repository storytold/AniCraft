//! Bones and inverse kinematics end to end (ADR 0043): building an armature with the Bone
//! tool's commands, posing by angle and by IK, pose tweens, limits and pins, removal, undo,
//! the file format, refusals, analytic renders and plug-in policy.

use super::*;
use anicraft_doc::{BlendMode, ColorEffect, Document, ElementKind, LoopMode, Paint, Rgba, Shape, Stage, Symbol, SymbolId, SymbolKind, Timeline};
use anicraft_geom::{Affine, Path};
use anicraft_render::render_frame;
use serde_json::json;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    match s.execute(id, &p) {
        Ok(v) => v,
        Err(e) => panic!("{id} {p}: {e}"),
    }
}

pub(crate) const ROOT: u64 = 10;
pub(crate) const ARM: u64 = 11;
pub(crate) const HAND: u64 = 12;
const SHAPE: u64 = 13;

/// A 500 × 300 stage, 20 frames, with three instances of a 100 × 10 black bar (registration at
/// its left end) along y = 100 from x = 100, 200 and 300, and a plain shape below them.
pub(crate) fn three_bars() -> Session {
    let mut d = Document { stage: Stage { width: 500, height: 300, ..Stage::default() }, ..Document::default() };
    let bar = SymbolId(1);
    let mut inner = Layer::new(LayerId(2), "bar");
    inner.keyframes[0] = Arc::new({
        let mut k = Keyframe::new(0, 1);
        k.elements.push(Arc::new(Element { id: ElementId(3), name: String::new(), transform: Affine::IDENTITY, color: ColorEffect::default(), kind: ElementKind::Shape(Shape::filled(Path::rect(0.0, -5.0, 100.0, 10.0), Paint::Solid(Rgba::BLACK))) }));
        k
    });
    d.library.push(Arc::new(Symbol { id: bar, name: "Bar".into(), kind: SymbolKind::Graphic, linkage: None, scale_grid_kept: None, scale_grid: None, timeline: Timeline { layers: vec![Arc::new(inner)] } }));
    let inst = |id: u64, x: f32| Arc::new(Element { id: ElementId(id), name: String::new(), transform: Affine::translate(x, 100.0), color: ColorEffect::default(), kind: ElementKind::Symbol { symbol: bar, loop_mode: LoopMode::Loop, first_frame: 0, blend: BlendMode::Normal, filters: Vec::new(), events: Vec::new() } });
    let shape = Arc::new(Element { id: ElementId(SHAPE), name: String::new(), transform: Affine::translate(100.0, 250.0), color: ColorEffect::default(), kind: ElementKind::Shape(Shape::filled(Path::rect(0.0, 0.0, 20.0, 20.0), Paint::Solid(Rgba::BLACK))) });
    let l = Arc::make_mut(&mut d.timeline_mut(0).layers[0]);
    l.keyframes = vec![Arc::new({
        let mut k = Keyframe::new(0, 20);
        k.elements = vec![inst(ROOT, 100.0), inst(ARM, 200.0), inst(HAND, 300.0), shape];
        k
    })];
    d.next_id = 100;
    Session::new(d)
}

/// [`three_bars`] rigged as a chain: shoulder (200, 100) → elbow (300, 100) → wrist (400, 100).
pub(crate) fn rigged() -> (Session, u64, u64) {
    let mut s = three_bars();
    let b1 = run(&mut s, "bone.add", json!({"from": ROOT, "to": ARM, "head": [200, 100], "tail": [300, 100]}))["bone"].as_u64().unwrap();
    let b2 = run(&mut s, "bone.add", json!({"from": ARM, "to": HAND, "tail": [400, 100]}))["bone"].as_u64().unwrap();
    (s, b1, b2)
}

fn bone_at(s: &mut Session, frame: u32, bone: u64) -> Value {
    let v = run(s, "bone.info", json!({"frame": frame}));
    v["armatures"][0]["bones"].as_array().unwrap().iter().find(|b| b["id"] == bone).cloned().unwrap()
}

fn pt(v: &Value) -> Vec2 {
    Vec2::new(v[0].as_f64().unwrap() as f32, v[1].as_f64().unwrap() as f32)
}

fn transform_of(s: &Session, frame: u32, id: u64) -> Affine {
    let tl = s.doc.timeline(s.scene);
    tl.layers.iter().find_map(|l| evaluate_layer_elements(l, frame).and_then(|(_, e)| e.iter().find(|e| e.id.0 == id).map(|e| e.transform))).unwrap()
}

#[test]
fn the_bone_tool_moves_instances_onto_an_armature_layer() {
    let (mut s, b1, b2) = rigged();
    let tl = s.doc.timeline(0);
    assert_eq!(tl.layers.len(), 2);
    let arm = &tl.layers[0];
    assert_eq!(arm.kind, LayerKind::Armature);
    assert_eq!(arm.name, "Armature_1");
    assert_eq!(s.layer, arm.id, "the armature layer is active");
    let ids: Vec<u64> = arm.keyframes[0].elements.iter().map(|e| e.id.0).collect();
    assert_eq!(ids, vec![ROOT, ARM, HAND]);
    assert_eq!((arm.keyframes[0].start, arm.keyframes[0].duration), (0, 20), "the pose spans the timeline");
    let rest: Vec<u64> = tl.layers[1].keyframes[0].elements.iter().map(|e| e.id.0).collect();
    assert_eq!(rest, vec![SHAPE], "the instances left their layer; the shape stayed");
    // Nothing moved on the stage.
    assert_eq!(transform_of(&s, 0, HAND), Affine::translate(300.0, 100.0));
    let b = bone_at(&mut s, 0, b2);
    assert!(pt(&b["head"]).distance(Vec2::new(300.0, 100.0)) < 1e-3, "the second bone starts where the first ends");
    assert!(pt(&b["tail"]).distance(Vec2::new(400.0, 100.0)) < 1e-3);
    assert_eq!(b["parent_bone"], b1);
    assert_eq!(bone_at(&mut s, 0, b1)["parent_bone"], Value::Null);
}

#[test]
fn bone_add_refuses_what_cant_be_joined() {
    let mut s = three_bars();
    let bad = |s: &mut Session, p: Value| s.execute("bone.add", &p).unwrap_err().to_string();
    assert!(bad(&mut s, json!({"from": ROOT, "to": ROOT})).contains("two different"));
    assert!(bad(&mut s, json!({"from": ROOT, "to": SHAPE})).contains("symbol"));
    assert!(bad(&mut s, json!({"from": ROOT, "to": 999})).contains("no instance"));
    assert!(bad(&mut s, json!({"from": ROOT})).contains("required"));
    assert!(bad(&mut s, json!({"from": ROOT, "to": ARM, "head": [1, "x"]})).contains("[x, y]"));
    run(&mut s, "bone.add", json!({"from": ROOT, "to": ARM}));
    assert!(bad(&mut s, json!({"from": HAND, "to": ARM})).contains("armature"), "start on the armature");
    assert!(bad(&mut s, json!({"from": ARM, "to": ROOT})).contains("root"));
    run(&mut s, "bone.add", json!({"from": ROOT, "to": HAND}));
    assert!(bad(&mut s, json!({"from": ARM, "to": HAND})).contains("already"));
    assert_eq!(s.history_len(), 2, "refusals leave no undo steps");
}

#[test]
fn ik_drags_reach_and_one_drag_is_one_undo_step() {
    let (mut s, b1, b2) = rigged();
    let steps = s.history_len();
    s.begin_gesture("Drag Bone");
    let mut last = Value::Null;
    for i in 0..=20 {
        let t = i as f32 / 20.0;
        last = run(&mut s, "bone.solve", json!({"bone": b2, "x": 400.0 - 100.0 * t, "y": 100.0 + 80.0 * t}));
    }
    s.end_gesture();
    assert_eq!(s.history_len(), steps + 1, "one drag, one undo step");
    assert_eq!(last["reached"], true, "{last}");
    let end = pt(&bone_at(&mut s, 0, b2)["tail"]);
    assert!(end.distance(Vec2::new(300.0, 180.0)) < 0.5, "{end:?}");
    // The root stays where it was; the shoulder and elbow turned.
    assert_eq!(transform_of(&s, 0, ROOT), Affine::translate(100.0, 100.0));
    assert!(bone_at(&mut s, 0, b1)["angle"].as_f64().unwrap().abs() > 1.0);
    // An unreachable target: the chain points straight at it, the root still put.
    let v = run(&mut s, "bone.solve", json!({"bone": b2, "x": 200, "y": 320}));
    assert_eq!(v["reached"], false);
    assert!(bone_at(&mut s, 0, b2)["angle"].as_f64().unwrap().abs() < 0.1, "straight");
    let tail = pt(&bone_at(&mut s, 0, b2)["tail"]);
    assert!(tail.distance(Vec2::new(200.0, 300.0)) < 0.05, "{tail:?}");
    assert_eq!(transform_of(&s, 0, ROOT), Affine::translate(100.0, 100.0));
    s.undo().unwrap();
    s.undo().unwrap();
    assert_eq!(transform_of(&s, 0, HAND), Affine::translate(300.0, 100.0), "undo puts the pose back");
}

#[test]
fn limits_and_pins_hold_in_every_pose() {
    let (mut s, b1, b2) = rigged();
    run(&mut s, "bone.pose", json!({"bone": b2, "angle": 60}));
    let v = run(&mut s, "bone.set_limits", json!({"bone": b2, "min": -20, "max": 20}));
    assert_eq!(v["constrain"], true);
    assert!((bone_at(&mut s, 0, b2)["angle"].as_f64().unwrap() - 20.0).abs() < 1e-3, "the pose came inside the new limits");
    run(&mut s, "bone.solve", json!({"bone": b2, "x": 230, "y": 160}));
    let a = bone_at(&mut s, 0, b2)["angle"].as_f64().unwrap();
    assert!((-20.0 - 1e-3..=20.0 + 1e-3).contains(&a), "{a}");
    assert!(s.execute("bone.set_limits", &json!({"bone": b2, "min": 30, "max": 10})).is_err());
    assert!(s.execute("bone.set_limits", &json!({"bone": b2, "min": -400})).is_err());
    run(&mut s, "bone.set_limits", json!({"bone": b2, "constrain": false}));
    assert_eq!(bone_at(&mut s, 0, b2)["min"], Value::Null);
    // Pinned: the shoulder keeps its angle while the elbow follows the drag.
    let before = bone_at(&mut s, 0, b1)["angle"].as_f64().unwrap();
    run(&mut s, "bone.pin", json!({"bone": b1, "pinned": true}));
    run(&mut s, "bone.solve", json!({"bone": b2, "x": 350, "y": 60}));
    assert_eq!(bone_at(&mut s, 0, b1)["angle"].as_f64().unwrap(), before);
    assert!(s.execute("bone.solve", &json!({"bone": b1, "x": 0, "y": 0})).unwrap_err().to_string().contains("pinned"));
}

#[test]
fn poses_key_frames_and_tween_with_easing() {
    let (mut s, b1, _) = rigged();
    run(&mut s, "bone.pose", json!({"bone": b1, "angle": 90, "frame": 10}));
    let l = s.doc.timeline(0).layers[0].clone();
    assert_eq!(l.keyframes.iter().map(|k| k.start).collect::<Vec<_>>(), vec![0, 10], "a pose keyed at frame 10");
    let a = |s: &mut Session, f| bone_at(s, f, b1)["angle"].as_f64().unwrap();
    assert_eq!(a(&mut s, 0), 0.0, "exact at the first key");
    assert!((a(&mut s, 10) - 90.0).abs() < 1e-4, "exact at the second key");
    assert!((a(&mut s, 5) - 45.0).abs() < 1e-3, "half way");
    // The hand rides the shoulder: at frame 10 it hangs straight down from it.
    assert!(transform_of(&s, 10, HAND).translation().distance(Vec2::new(200.0, 200.0)) < 1e-3);
    // The span's easing shapes the pose tween.
    run(&mut s, "timeline.set_easing", json!({"frame": 0, "easing": "QuadIn"}));
    assert!((a(&mut s, 5) - 22.5).abs() < 1e-3, "{}", a(&mut s, 5));
    // Posing inside the tween keys that frame with the pose it showed.
    run(&mut s, "bone.pose", json!({"bone": b1, "rotate_by": 10, "frame": 5}));
    assert!((a(&mut s, 5) - 32.5).abs() < 1e-3);
    assert_eq!(s.doc.timeline(0).layers[0].keyframes.len(), 3);
    // Moving the root moves the whole armature, in that pose.
    run(&mut s, "bone.pose", json!({"dx": 10, "dy": 5, "frame": 10}));
    assert!(transform_of(&s, 10, HAND).translation().distance(Vec2::new(210.0, 205.0)) < 1e-3);
}

#[test]
fn artwork_edits_are_refused_on_the_armature_layer() {
    let (mut s, _, _) = rigged();
    s.select_only([ElementId(HAND)]);
    for (id, p) in [("draw.rect", json!({"x": 0, "y": 0, "w": 5, "h": 5})), ("element.translate", json!({"dx": 5, "dy": 0})), ("edit.delete", json!({})), ("timeline.create_motion_tween", json!({}))] {
        let e = s.execute(id, &p).unwrap_err().to_string();
        assert!(e.contains("armature"), "{id}: {e}");
    }
    assert!(s.execute("layer.set_type", &json!({"type": "normal"})).is_err());
}

#[test]
fn removing_bones_frees_the_instances_where_they_are() {
    let (mut s, b1, b2) = rigged();
    run(&mut s, "bone.pose", json!({"bone": b1, "angle": 90}));
    run(&mut s, "bone.pose", json!({"bone": b1, "angle": 0, "frame": 10}));
    // Two poses: removing asks first (undoable tier).
    let c = s.confirmation("bone.remove", &json!({"bone": b2})).unwrap();
    assert!(!c.irreversible && c.message.contains("2 keyframes"), "{}", c.message);
    let hand = transform_of(&s, 0, HAND);
    let v = run(&mut s, "bone.remove", json!({"bone": b2}));
    assert_eq!(v["armature_removed"], false);
    let tl = s.doc.timeline(0);
    assert_eq!(tl.layers.len(), 3);
    assert_eq!(tl.layers[1].keyframes[0].elements[0].id.0, HAND, "the hand is on a layer of its own");
    assert!(tl.layers[0].keyframes.iter().all(|k| k.pose.0.iter().all(|j| j.bone.0 != b2)));
    assert_eq!(transform_of(&s, 0, HAND), hand, "where it was");
    // The last bone takes the armature with it; the instances stay as they look.
    let arm = transform_of(&s, 0, ARM);
    let v = run(&mut s, "bone.remove", json!({"bone": b1}));
    assert_eq!(v["armature_removed"], true);
    assert!(s.doc.timeline(0).layers.iter().all(|l| l.kind != LayerKind::Armature));
    assert_eq!(transform_of(&s, 0, ARM), arm);
    s.undo().unwrap();
    s.undo().unwrap();
    assert_eq!(s.doc.timeline(0).layers[0].armature.as_ref().unwrap().bones.len(), 2);
}

#[test]
fn armatures_survive_a_file_round_trip_and_render_the_pose() {
    let (mut s, b1, b2) = rigged();
    run(&mut s, "bone.set_limits", json!({"bone": b2, "min": -45, "max": 45}));
    run(&mut s, "bone.pin", json!({"bone": b1, "pinned": true}));
    run(&mut s, "bone.pin", json!({"bone": b1, "pinned": false}));
    run(&mut s, "bone.pose", json!({"bone": b1, "angle": 90, "frame": 10}));
    let text = crate::format::to_json(&s.doc).unwrap();
    let back = crate::format::from_json(&text).unwrap();
    assert_eq!(back, s.doc);
    for f in [0, 5, 10] {
        assert_eq!(render_frame(&back, f, 1.0).unwrap().data, render_frame(&s.doc, f, 1.0).unwrap().data);
    }
    // At frame 10 the arm hangs from the shoulder: the bar covers (200, 150), not (250, 100).
    let pm = render_frame(&s.doc, 10, 1.0).unwrap();
    assert!(pm.pixel(200, 150).unwrap()[0] < 0.1, "the arm points down");
    assert!(pm.pixel(250, 100).unwrap()[0] > 0.9, "and is no longer along the line");
    // Older files (no armature) load as before; a file's armature on another kind of layer is
    // dropped.
    let old = crate::format::from_json(&crate::format::to_json(&Document::default()).unwrap()).unwrap();
    assert!(old.timeline(0).layers.iter().all(|l| l.armature.is_none()));
    let mut odd = s.doc.clone();
    Arc::make_mut(&mut odd.timeline_mut(0).layers[0]).kind = LayerKind::Normal;
    let odd = crate::format::from_json(&crate::format::to_json(&odd).unwrap()).unwrap();
    assert!(odd.timeline(0).layers[0].armature.is_none());
}

#[test]
fn plug_ins_may_pose_armatures() {
    for id in ["bone.add", "bone.remove", "bone.set_limits", "bone.pin", "bone.pose", "bone.solve", "bone.info"] {
        assert!(crate::cmds_plugin::plugin_may_call(id).is_ok(), "{id}");
    }
}
