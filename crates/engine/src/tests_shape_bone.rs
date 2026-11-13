//! Shape armatures end to end (ADR 0071): bones drawn inside a shape, automatic binding, the
//! Bind tool's command, posing by angle and by IK, pose tweens, removal, undo, refusals, the
//! file format, analytic renders and plug-in policy.

use super::*;
use anicraft_doc::{ColorEffect, Document, ElementKind, Paint, Rgba, Shape, Stage};
use anicraft_geom::{Affine, Path, PathCmd};
use anicraft_render::render_frame;
use serde_json::json;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    match s.execute(id, &p) {
        Ok(v) => v,
        Err(e) => panic!("{id} {p}: {e}"),
    }
}

const TAIL: u64 = 10;

/// A 400 × 300 stage, 20 frames, with a black tail 200 × 20 px lying along y = 150 from
/// x = 100 (anchors every 25 px along both edges, part of the layer's merged drawing).
fn tail() -> Session {
    let mut d = Document { stage: Stage { width: 400, height: 300, ..Stage::default() }, ..Document::default() };
    let mut p = Path::new();
    p.move_to(Vec2::new(0.0, -10.0));
    for i in 1..=8 {
        p.line_to(Vec2::new(25.0 * i as f32, -10.0));
    }
    for i in (0..=8).rev() {
        p.line_to(Vec2::new(25.0 * i as f32, 10.0));
    }
    p.close();
    let shape = Shape { merge: true, ..Shape::filled(p, Paint::Solid(Rgba::BLACK)) };
    let el = Arc::new(Element { id: ElementId(TAIL), name: String::new(), transform: Affine::translate(100.0, 150.0), color: ColorEffect::default(), kind: ElementKind::Shape(shape) });
    let l = Arc::make_mut(&mut d.timeline_mut(0).layers[0]);
    l.keyframes = vec![Arc::new({
        let mut k = Keyframe::new(0, 20);
        k.elements = vec![el];
        k
    })];
    d.next_id = 100;
    Session::new(d)
}

/// [`tail`] with two bones along its middle: (100, 150) → (200, 150) → (300, 150).
fn rigged() -> (Session, u64, u64) {
    let mut s = tail();
    let v = run(&mut s, "bone.add_in_shape", json!({"shape": TAIL, "head": [100, 150], "tail": [200, 150]}));
    assert_eq!(v["created"], true);
    let b1 = v["bone"].as_u64().unwrap();
    let b2 = run(&mut s, "bone.add_in_shape", json!({"parent": b1, "tail": [300, 150]}))["bone"].as_u64().unwrap();
    (s, b1, b2)
}

fn shape_at(s: &Session, frame: u32) -> (Affine, Arc<Path>, bool) {
    let tl = s.doc.timeline(s.scene);
    tl.layers
        .iter()
        .find_map(|l| {
            let (_, els) = evaluate_layer_elements(l, frame)?;
            els.iter().find(|e| e.id.0 == TAIL).and_then(|e| match &e.kind {
                ElementKind::Shape(sh) => Some((e.transform, sh.path.clone(), sh.merge)),
                _ => None,
            })
        })
        .unwrap()
}

/// The tail's anchors on the stage at `frame`.
fn points(s: &Session, frame: u32) -> Vec<Vec2> {
    let (m, p, _) = shape_at(s, frame);
    anicraft_doc::skin::anchors(&p).into_iter().map(|a| m.apply(a)).collect()
}

fn turned(p: Vec2, j: Vec2, a: f32) -> Vec2 {
    let (sn, c) = a.sin_cos();
    let d = p - j;
    Vec2::new(j.x + d.x * c - d.y * sn, j.y + d.x * sn + d.y * c)
}

fn black(s: &Session, frame: u32, x: u32, y: u32) -> bool {
    render_frame(&s.doc, frame, 1.0).unwrap().pixel(x, y).unwrap()[0] < 0.1
}

#[test]
fn bones_inside_a_shape_move_it_to_an_armature_layer_as_drawn() {
    let before = render_frame(&tail().doc, 0, 1.0).unwrap().data;
    let drawn = points(&tail(), 0);
    let (mut s, b1, b2) = rigged();
    let tl = s.doc.timeline(0);
    assert_eq!(tl.layers.len(), 2);
    let arm = &tl.layers[0];
    assert_eq!(arm.kind, LayerKind::Armature);
    assert_eq!(s.layer, arm.id, "the armature layer is active");
    assert!(tl.layers[1].keyframes[0].elements.is_empty(), "the shape left its layer");
    let (m, _, merge) = shape_at(&s, 0);
    assert_eq!(m, Affine::translate(100.0, 150.0));
    assert!(!merge, "an armature's shape is an object: merge drawing leaves it");
    // The rest pose is the drawing, exactly.
    assert_eq!(points(&s, 0), drawn);
    assert_eq!(render_frame(&s.doc, 0, 1.0).unwrap().data, before);
    let v = run(&mut s, "bone.info", json!({"points": true}));
    let a = &v["armatures"][0];
    assert_eq!(a["shape"], TAIL);
    assert_eq!(a["roots"], json!([TAIL]));
    let bones = a["bones"].as_array().unwrap();
    assert_eq!((bones[0]["id"].as_u64(), bones[1]["id"].as_u64()), (Some(b1), Some(b2)));
    assert_eq!(bones[1]["parent_bone"], b1);
    assert_eq!(bones[1]["head"], json!([200.0, 150.0]), "the chain joins end to start");
    // Every point is bound, its weights summing to 1.
    let pts = a["points"].as_array().unwrap();
    assert_eq!(pts.len(), drawn.len());
    for p in pts {
        let sum: f64 = p["bones"].as_array().unwrap().iter().map(|i| i[1].as_f64().unwrap()).sum();
        assert!((sum - 1.0).abs() < 1e-5, "{p}");
    }
    let (n1, n2) = (bones[0]["points"].as_u64().unwrap(), bones[1]["points"].as_u64().unwrap());
    assert!(n1 > 0 && n2 > 0 && n1 + n2 >= drawn.len() as u64, "{n1} + {n2}");
}

#[test]
fn a_quarter_bend_swings_the_tail_and_tweens_through_the_middle() {
    let (mut s, _, b2) = rigged();
    let drawn = points(&s, 0);
    // Key the bend at frame 10 (the rest pose at 0 tweens into it).
    run(&mut s, "bone.pose", json!({"bone": b2, "angle": 90, "frame": 10}));
    let joint = Vec2::new(200.0, 150.0);
    let bent = points(&s, 10);
    // The tip's corners (the 8th and 9th points) follow the second bone alone.
    for i in [8, 9] {
        assert!(bent[i].distance(turned(drawn[i], joint, std::f32::consts::FRAC_PI_2)) < 1e-3, "{i}: {:?}", bent[i]);
    }
    // The root half didn't move.
    assert_eq!(bent[1], drawn[1]);
    assert_eq!(bent[0], drawn[0]);
    // Half way the tip has turned 45°.
    let mid = points(&s, 5);
    assert!(mid[8].distance(turned(drawn[8], joint, std::f32::consts::FRAC_PI_4)) < 1e-3, "{:?}", mid[8]);
    // The render shows it: the tail now hangs down from x = 200, its old tip area is empty.
    assert!(black(&s, 0, 280, 150) && !black(&s, 0, 200, 230));
    assert!(!black(&s, 10, 280, 150) && black(&s, 10, 200, 230));
}

#[test]
fn dragging_a_shape_bone_bends_it_by_ik_in_one_undo_step() {
    let (mut s, _, b2) = rigged();
    let steps = s.history_len();
    s.begin_gesture("Pose Armature");
    for t in [[290.0, 120.0], [270.0, 90.0], [250.0, 60.0]] {
        run(&mut s, "bone.solve", json!({"bone": b2, "x": t[0], "y": t[1]}));
    }
    s.end_gesture();
    assert_eq!(s.history_len(), steps + 1);
    let v = run(&mut s, "bone.info", json!({}));
    let tail = &v["armatures"][0]["bones"][1]["tail"];
    let end = Vec2::new(tail[0].as_f64().unwrap() as f32, tail[1].as_f64().unwrap() as f32);
    assert!(end.distance(Vec2::new(250.0, 60.0)) < 0.5, "{end:?}");
    // The outline follows the bones; the file holds what shows.
    let shown = points(&s, 0);
    assert!(shown[8].distance(Vec2::new(250.0, 60.0)) < 11.0, "{:?}", shown[8]);
    let k = &s.doc.timeline(0).layers[0].keyframes[0];
    let ElementKind::Shape(sh) = &k.elements[0].kind else { panic!("not a shape") };
    assert_eq!(anicraft_doc::skin::anchors(&sh.path).iter().map(|p| Affine::translate(100.0, 150.0).apply(*p)).collect::<Vec<_>>(), shown);
    s.undo().unwrap();
    assert_eq!(points(&s, 0), points(&rigged().0, 0), "undo puts the rest pose back");
}

#[test]
fn binding_points_by_hand() {
    let (mut s, b1, b2) = rigged();
    let tip = 8usize;
    // The tip follows only the first bone: bending the second leaves it.
    let v = run(&mut s, "bone.bind", json!({"bone": b1, "points": [tip, 9]}));
    assert_eq!(v["changed"], 2);
    run(&mut s, "bone.pose", json!({"bone": b2, "angle": 90}));
    let drawn = points(&tail(), 0);
    let now = points(&s, 0);
    assert_eq!(now[tip], drawn[tip], "bound to the bone that didn't turn");
    assert_ne!(now[7], drawn[7], "its neighbours bend");
    // Shared: half way between staying and turning.
    run(&mut s, "bone.bind", json!({"bone": b2, "points": [tip], "mode": "add"}));
    let want = (drawn[tip] + turned(drawn[tip], Vec2::new(200.0, 150.0), std::f32::consts::FRAC_PI_2)) * 0.5;
    assert!(points(&s, 0)[tip].distance(want) < 1e-3);
    let info = run(&mut s, "bone.info", json!({"points": true}));
    let p = &info["armatures"][0]["points"][tip];
    assert_eq!(p["manual"], true);
    assert_eq!(p["bones"].as_array().unwrap().len(), 2);
    // Removed from both: it stays where it was drawn.
    run(&mut s, "bone.bind", json!({"bone": b1, "points": [tip], "mode": "remove"}));
    run(&mut s, "bone.bind", json!({"bone": b2, "points": [tip], "mode": "remove"}));
    assert_eq!(points(&s, 0)[tip], drawn[tip]);
    // Automatic again: it follows the second bone round.
    run(&mut s, "bone.bind", json!({"bone": b2, "points": [tip], "mode": "auto"}));
    assert!(points(&s, 0)[tip].distance(turned(drawn[tip], Vec2::new(200.0, 150.0), std::f32::consts::FRAC_PI_2)) < 1e-3);
    // Each bind is one undo step.
    s.undo().unwrap();
    assert_eq!(points(&s, 0)[tip], drawn[tip]);
}

#[test]
fn shape_bones_refuse_what_cant_be_done() {
    let mut s = tail();
    let steps = s.history_len();
    for (p, want) in [
        (json!({"shape": TAIL, "head": [100, 50], "tail": [200, 50]}), "inside"),
        (json!({"shape": TAIL, "head": [150, 150], "tail": [150.5, 150]}), "length"),
        (json!({"shape": TAIL, "tail": [200, 150]}), "head"),
        (json!({"shape": TAIL, "head": [150, 150]}), "tail"),
        (json!({"shape": 999, "head": [150, 150], "tail": [200, 150]}), "no shape"),
        (json!({"head": [150, 150], "tail": [200, 150]}), "shape"),
        (json!({"parent": 999, "tail": [200, 150]}), "no bone"),
    ] {
        let e = s.execute("bone.add_in_shape", &p).unwrap_err().to_string();
        assert!(e.contains(want), "{p}: {e}");
    }
    assert_eq!(s.history_len(), steps, "refusals leave no undo steps");
    let (mut s, b1, _) = rigged();
    for (p, want) in [(json!({"bone": b1, "points": [9999]}), "below"), (json!({"bone": b1, "mode": "glue"}), "mode"), (json!({"bone": b1, "points": "all"}), "points"), (json!({"bone": 4242}), "no bone")] {
        let e = s.execute("bone.bind", &p).unwrap_err().to_string();
        assert!(e.contains(want), "{p}: {e}");
    }
    // The shape's outline is the armature's: artwork edits are refused on its layer.
    s.select_only([ElementId(TAIL)]);
    for (id, p) in [("element.translate", json!({"dx": 5, "dy": 0})), ("edit.delete", json!({})), ("draw.rect", json!({"x": 0, "y": 0, "w": 5, "h": 5}))] {
        let e = s.execute(id, &p).unwrap_err().to_string();
        assert!(e.contains("armature"), "{id}: {e}");
    }
    // Bones of symbol instances bind nothing; instances don't join a shape's armature.
    let (mut inst, ib1, _) = crate::tests_bone::rigged();
    assert!(inst.execute("bone.bind", &json!({"bone": ib1})).unwrap_err().to_string().contains("instances"));
    assert!(inst.execute("bone.add_in_shape", &json!({"parent": ib1, "tail": [0, 0]})).unwrap_err().to_string().contains("instances"));
    let e = inst.execute("bone.add_in_shape", &json!({"shape": crate::tests_bone::ROOT, "head": [150, 100], "tail": [190, 100]})).unwrap_err().to_string();
    assert!(e.contains("shape"), "{e}");
}

#[test]
fn merged_drawing_leaves_the_armature_shape_alone() {
    let (mut s, _, _) = rigged();
    let drawn = shape_at(&s, 0).1;
    let other = s.doc.timeline(0).layers[1].id;
    run(&mut s, "layer.select", json!({"layer": other.0}));
    run(&mut s, "draw.rect", json!({"x": 150, "y": 100, "w": 60, "h": 100}));
    assert_eq!(*shape_at(&s, 0).1, *drawn, "a shape drawn over it on another layer doesn't cut it");
}

#[test]
fn removing_shape_bones_rebinds_and_the_last_frees_the_shape_as_it_looks() {
    let (mut s, b1, b2) = rigged();
    run(&mut s, "bone.pose", json!({"bone": b1, "angle": 30}));
    let v = run(&mut s, "bone.remove", json!({"bone": b2}));
    assert_eq!(v["armature_removed"], false);
    let tl = s.doc.timeline(0);
    assert_eq!(tl.layers.len(), 2, "no layer for freed instances: a shape's bones move none");
    let arm = tl.layers[0].armature.as_ref().unwrap();
    assert!(arm.skin.as_ref().unwrap().points.iter().all(|p| p.bones == vec![anicraft_doc::skin::Influence(anicraft_doc::BoneId(b1), 1.0)]), "every point follows the bone that's left");
    let shown = points(&s, 0);
    run(&mut s, "bone.remove", json!({"bone": b1}));
    let tl = s.doc.timeline(0);
    assert!(tl.layers.iter().all(|l| l.kind != LayerKind::Armature));
    assert_eq!(points(&s, 0), shown, "the shape stays as it looked, now a drawing object");
    s.undo().unwrap();
    s.undo().unwrap();
    assert_eq!(s.doc.timeline(0).layers[0].armature.as_ref().unwrap().bones.len(), 2);
}

#[test]
fn shape_armatures_survive_a_file_round_trip_and_render_the_same() {
    let (mut s, b1, b2) = rigged();
    run(&mut s, "bone.bind", json!({"bone": b1, "points": [3], "mode": "add"}));
    run(&mut s, "bone.set_limits", json!({"bone": b2, "min": -100, "max": 100}));
    run(&mut s, "bone.pose", json!({"bone": b2, "angle": 90, "frame": 10}));
    let text = crate::format::to_json(&s.doc).unwrap();
    let back = crate::format::from_json(&text).unwrap();
    assert_eq!(back, s.doc);
    for f in [0, 5, 10] {
        assert_eq!(render_frame(&back, f, 1.0).unwrap().data, render_frame(&s.doc, f, 1.0).unwrap().data, "frame {f}");
    }
    // The file stores the outline as drawn, so a keyframe's bent outline never drifts.
    let skin = back.timeline(0).layers[0].armature.as_ref().unwrap().skin.clone().unwrap();
    assert_eq!(skin.rest.cmds.first(), Some(&PathCmd::MoveTo(Vec2::new(0.0, -10.0))));
    // New ids never collide with the bones' joint nodes.
    let mut back = Session::new(back);
    let v = run(&mut back, "bone.add_in_shape", json!({"parent": b2, "tail": [200, 280]}));
    assert!(v["bone"].as_u64().unwrap() > b2);
}

#[test]
fn plug_ins_may_rig_shapes() {
    for id in ["bone.add_in_shape", "bone.bind"] {
        assert!(crate::cmds_plugin::plugin_may_call(id).is_ok(), "{id}");
    }
}
