//! Painted weights and reshaping a bound shape end to end (ADR 0090): the brush, numeric
//! weights, undo per stroke, Subselection edits at rest keeping the binding, refusals and
//! plug-in policy.

use super::*;
use anicraft_doc::{BoneId, ColorEffect, Document, ElementKind, Paint, Rgba, Shape, Stage};
use anicraft_geom::{Affine, Path};
use serde_json::json;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    match s.execute(id, &p) {
        Ok(v) => v,
        Err(e) => panic!("{id} {p}: {e}"),
    }
}

const TAIL: u64 = 10;

/// The tail outline of `tests_shape_bone`: 200 × 20 px, anchors every 25 px on both edges.
fn outline() -> Path {
    let mut p = Path::new();
    p.move_to(Vec2::new(0.0, -10.0));
    for i in 1..=8 {
        p.line_to(Vec2::new(25.0 * i as f32, -10.0));
    }
    for i in (0..=8).rev() {
        p.line_to(Vec2::new(25.0 * i as f32, 10.0));
    }
    p.close();
    p
}

/// A tail at (100, 150) with two bones along its middle: (100, 150) → (200, 150) → (300, 150).
fn rigged() -> (Session, u64, u64) {
    let mut d = Document { stage: Stage { width: 400, height: 300, ..Stage::default() }, ..Document::default() };
    let shape = Shape { merge: true, ..Shape::filled(outline(), Paint::Solid(Rgba::BLACK)) };
    let el = Arc::new(Element { id: ElementId(TAIL), name: String::new(), transform: Affine::translate(100.0, 150.0), color: ColorEffect::default(), kind: ElementKind::Shape(shape) });
    let l = Arc::make_mut(&mut d.timeline_mut(0).layers[0]);
    l.keyframes = vec![Arc::new({
        let mut k = Keyframe::new(0, 20);
        k.elements = vec![el];
        k
    })];
    d.next_id = 100;
    let mut s = Session::new(d);
    let b1 = run(&mut s, "bone.add_in_shape", json!({"shape": TAIL, "head": [100, 150], "tail": [200, 150]}))["bone"].as_u64().unwrap();
    let b2 = run(&mut s, "bone.add_in_shape", json!({"parent": b1, "tail": [300, 150]}))["bone"].as_u64().unwrap();
    (s, b1, b2)
}

fn skin(s: &Session) -> anicraft_doc::skin::Skin {
    s.timeline().layers.iter().find_map(|l| l.armature.as_ref().and_then(|a| a.skin.clone())).unwrap()
}

fn weight(s: &Session, point: usize, bone: u64) -> f32 {
    skin(s).points[point].weight(BoneId(bone))
}

fn shown_path(s: &Session, frame: u32) -> Arc<Path> {
    s.timeline()
        .layers
        .iter()
        .find_map(|l| {
            let (_, els) = evaluate_layer_elements(l, frame)?;
            els.iter().find(|e| e.id.0 == TAIL).and_then(|e| match &e.kind {
                ElementKind::Shape(sh) => Some(sh.path.clone()),
                _ => None,
            })
        })
        .unwrap()
}

fn normalised(s: &Session) -> bool {
    skin(s).points.iter().all(|b| b.bones.is_empty() || ((b.bones.iter().map(|i| i.1).sum::<f32>() - 1.0).abs() < 1e-5 && b.bones.len() <= 4))
}

#[test]
fn a_brush_stroke_paints_locally_and_undoes_in_one_step() {
    let (mut s, _, b2) = rigged();
    let before = skin(&s).points.clone();
    assert_eq!(weight(&s, 1, b2), 0.0);
    // A stroke of three moves over the point at (125, 140), as the Bind tool sends it.
    s.begin_gesture("Paint Weights");
    for x in [120.0, 125.0, 130.0] {
        run(&mut s, "bone.paint_weights", json!({"bone": b2, "at": [[x, 140.0]], "radius": 12, "strength": 50}));
    }
    s.end_gesture();
    assert!(weight(&s, 1, b2) > 0.5, "{}", weight(&s, 1, b2));
    assert!(normalised(&s));
    // Only points within the radius of a dab changed.
    let rest = anicraft_doc::skin::anchors(&skin(&s).rest);
    for (i, p) in rest.iter().enumerate() {
        let at = *p + Vec2::new(100.0, 150.0);
        if [120.0, 125.0, 130.0].iter().all(|x| at.distance(Vec2::new(*x, 140.0)) >= 12.0) {
            assert_eq!(skin(&s).points[i], before[i], "point {i}");
        }
    }
    s.undo().unwrap();
    assert_eq!(skin(&s).points, before);
}

#[test]
fn painted_weights_bend_the_shape_and_stay_normalised() {
    let (mut s, b1, b2) = rigged();
    for (mode, bone) in [("add", b2), ("subtract", b1), ("smooth", b2), ("subtract", b2), ("add", b1)] {
        let at: Vec<[f32; 2]> = (0..30).map(|i| [100.0 + 7.0 * i as f32, if i % 2 == 0 { 140.0 } else { 160.0 }]).collect();
        run(&mut s, "bone.paint_weights", json!({"bone": bone, "at": at, "radius": 30, "strength": 40, "mode": mode}));
        assert!(normalised(&s), "{mode}");
    }
    // Point 1 painted onto B2 entirely now turns with B2.
    run(&mut s, "bone.set_weight", json!({"bone": b2, "points": [1], "weight": 100}));
    run(&mut s, "bone.pose", json!({"bone": b2, "angle": 90, "frame": 10}));
    let bent = anicraft_doc::skin::anchors(&shown_path(&s, 10));
    let rest = anicraft_doc::skin::anchors(&skin(&s).rest);
    assert!(bent[1].distance(rest[1]) > 10.0, "{:?} vs {:?}", bent[1], rest[1]);
}

#[test]
fn weights_are_editable_numerically_per_point() {
    let (mut s, b1, b2) = rigged();
    let v = run(&mut s, "bone.set_weight", json!({"bone": b1, "points": [1, 2], "weight": 30}));
    assert_eq!(v["changed"], 2);
    assert!((weight(&s, 1, b1) - 0.3).abs() < 1e-6 && (weight(&s, 1, b2) - 0.7).abs() < 1e-6);
    assert!(skin(&s).points[1].manual);
    // bone.info reports them.
    let info = run(&mut s, "bone.info", json!({"points": true}));
    let bones = info["armatures"][0]["points"][1]["bones"].as_array().unwrap().clone();
    assert_eq!(bones.len(), 2);
    // Refusals: out-of-range weights and points, and bones of instance armatures.
    assert!(s.execute("bone.set_weight", &json!({"bone": b1, "points": [1], "weight": 101})).is_err());
    assert!(s.execute("bone.set_weight", &json!({"bone": b1, "points": [999], "weight": 10})).is_err());
    assert!(s.execute("bone.set_weight", &json!({"bone": b1, "weight": 10})).is_err());
    assert!(s.execute("bone.paint_weights", &json!({"bone": b1, "at": [[1, 2]], "radius": 0})).is_err());
    assert!(s.execute("bone.paint_weights", &json!({"bone": b1, "at": [[1, 2]], "mode": "blur"})).is_err());
    assert!(s.execute("bone.paint_weights", &json!({"bone": b1, "at": [[1, "x"]]})).is_err());
    assert!(s.execute("bone.paint_weights", &json!({"bone": 4242, "at": [[1, 2]]})).is_err());
}

#[test]
fn reshaping_at_rest_keeps_the_binding_and_is_exact() {
    let (mut s, b1, b2) = rigged();
    let k0 = skin(&s);
    // Hand-bind the neighbours apart: anchor 3 (x = 75) to B1, anchor 4 (x = 100) to B2.
    run(&mut s, "bone.bind", json!({"bone": b1, "points": [3], "mode": "only"}));
    run(&mut s, "bone.bind", json!({"bone": b2, "points": [4], "mode": "only"}));
    // Subselection's double-click on the top edge a quarter of the way from x = 75 to x = 100.
    let lid = s.layer;
    let v = run(&mut s, "path.add_point", json!({"id": TAIL, "x": 181.25, "y": 140.0}));
    assert_eq!(s.layer, lid);
    let cmd = v["cmd"].as_u64().unwrap() as usize;
    let k = skin(&s);
    let n = anicraft_doc::skin::anchors(&k.rest).len();
    assert_eq!(k.points.len(), n);
    assert!(n > k0.points.len());
    // The new anchor blends its neighbours by where it sits between them.
    let i = cmd;
    assert!((k.points[i].weight(BoneId(b1)) - 0.75).abs() < 1e-3 && (k.points[i].weight(BoneId(b2)) - 0.25).abs() < 1e-3, "{:?}", k.points[i]);
    // The rest pose shows exactly the edited outline: what the same edit gives an unbound shape.
    let mut want = outline();
    want.close_explicitly();
    let (c, t, _) = want.nearest_segment(Vec2::new(181.25, 140.0), &Affine::translate(100.0, 150.0)).unwrap();
    assert!(want.split_segment(c, t));
    assert_eq!(*k.rest, want);
    assert_eq!(*shown_path(&s, 0), want);
    assert!(Arc::ptr_eq(&shown_path(&s, 0), &k.rest));
    // Moving a point keeps every binding; deleting one drops its own.
    run(&mut s, "path.move_points", json!({"id": TAIL, "points": [[cmd, 0]], "dx": 0, "dy": -6}));
    assert_eq!(skin(&s).points, k.points);
    run(&mut s, "path.delete_points", json!({"id": TAIL, "cmds": [cmd]}));
    let mut left = k.points.clone();
    left.remove(i);
    assert_eq!(skin(&s).points, left);
    // Undo walks back through each edit.
    s.undo().unwrap();
    s.undo().unwrap();
    s.undo().unwrap();
    assert_eq!(*skin(&s).rest, outline());
}

#[test]
fn a_posed_frame_bends_the_reshaped_outline_and_refuses_reshaping() {
    let (mut s, _, b2) = rigged();
    run(&mut s, "bone.pose", json!({"bone": b2, "angle": 45, "frame": 10}));
    run(&mut s, "path.add_point", json!({"id": TAIL, "x": 281.25, "y": 140.0}));
    let n = anicraft_doc::skin::anchors(&skin(&s).rest).len();
    // Every keyframe holds the new outline, bent by its pose.
    assert_eq!(anicraft_doc::skin::anchors(&shown_path(&s, 10)).len(), n);
    assert_ne!(*shown_path(&s, 10), *skin(&s).rest);
    s.set_frame(10);
    let e = s.execute("path.move_points", &json!({"id": TAIL, "points": [[2, 0]], "dx": 0, "dy": -6})).unwrap_err();
    assert!(e.to_string().contains("rest pose"), "{e}");
    // A locked armature layer refuses too.
    s.set_frame(0);
    let lid = s.layer;
    Arc::make_mut(s.doc.timeline_mut(0).layers.iter_mut().find(|l| l.id == lid).unwrap()).locked = true;
    assert!(s.execute("path.move_points", &json!({"id": TAIL, "points": [[2, 0]], "dx": 0, "dy": -6})).is_err());
}

#[test]
fn files_keep_painted_and_reshaped_bindings() {
    let (mut s, _, b2) = rigged();
    run(&mut s, "bone.paint_weights", json!({"bone": b2, "at": [[150, 140]], "radius": 30, "strength": 60}));
    run(&mut s, "path.add_point", json!({"id": TAIL, "x": 181.25, "y": 140.0}));
    let text = crate::format::to_json(&s.doc).unwrap();
    let back = crate::format::from_json(&text).unwrap();
    let k = back.timeline(0).layers.iter().find_map(|l| l.armature.as_ref().and_then(|a| a.skin.clone())).unwrap();
    assert_eq!(k, skin(&s));
}

#[test]
fn plug_ins_may_paint_weights() {
    for id in ["bone.paint_weights", "bone.set_weight"] {
        assert!(crate::cmds_plugin::plugin_may_call(id).is_ok(), "{id}");
    }
}

#[test]
fn subselection_picks_the_bound_shape_to_show_its_points() {
    let (mut s, _, _) = rigged();
    run(&mut s, "element.pick", json!({"x": 150, "y": 150}));
    assert!(s.selection.contains(s.layer, ElementId(TAIL)));
    // It is a drawing object, so the Subselection tool lists its own points.
    assert!(s.visible_elements().iter().any(|e| e.id.0 == TAIL && matches!(&e.kind, ElementKind::Shape(sh) if !sh.merge)));
}
