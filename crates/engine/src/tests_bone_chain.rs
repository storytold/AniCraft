//! A bone chain stays one chain (user report #239): bones join end to start wherever the Bone
//! tool's drag began on an instance, lengths hold, the joints stay joined through many IK drags,
//! a small pointer move is a small turn, and grabbing a bone along its length doesn't snap its
//! end to the pointer.

use super::*;
use crate::tests_bone::{ARM, HAND, ROOT, three_bars};
use anicraft_doc::armature;
use anicraft_geom::Vec2;
use serde_json::json;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn bones(s: &mut Session) -> Vec<Value> {
    run(s, "bone.info", json!({}))["armatures"][0]["bones"].as_array().cloned().unwrap_or_default()
}

fn pt(v: &Value) -> Vec2 {
    Vec2::new(v[0].as_f64().unwrap_or(f64::NAN) as f32, v[1].as_f64().unwrap_or(f64::NAN) as f32)
}

/// The reporter's steps: a bone from the first bar to the second, then one from the second bar
/// to the third, started on the bar a little way from the first bone's end (as a quick drag does).
fn drawn_by_hand() -> (Session, u64, u64) {
    let mut s = three_bars();
    let b1 = run(&mut s, "bone.add", json!({"from": ROOT, "to": ARM, "head": [150, 100], "tail": [250, 100]}))["bone"].as_u64().unwrap();
    let b2 = run(&mut s, "bone.add", json!({"from": ARM, "to": HAND, "head": [262, 103], "tail": [350, 100]}))["bone"].as_u64().unwrap();
    (s, b1, b2)
}

#[test]
fn a_bone_from_an_instance_on_a_chain_starts_at_its_joint() {
    let (mut s, _, _) = drawn_by_hand();
    let bs = bones(&mut s);
    assert!(pt(&bs[1]["head"]).distance(pt(&bs[0]["tail"])) < 1e-3, "the second bone starts where the first ends: {} vs {}", bs[1]["head"], bs[0]["tail"]);
    // A root's bones still start where they were drawn (a torso's shoulders and hips).
    let mut s = three_bars();
    run(&mut s, "bone.add", json!({"from": ROOT, "to": ARM, "head": [150, 100]}));
    run(&mut s, "bone.add", json!({"from": ROOT, "to": HAND, "head": [180, 100]}));
    let bs = bones(&mut s);
    assert!(pt(&bs[0]["head"]).distance(Vec2::new(150.0, 100.0)) < 1e-3);
    assert!(pt(&bs[1]["head"]).distance(Vec2::new(180.0, 100.0)) < 1e-3);
}

#[test]
fn repeated_ik_drags_keep_the_chain_joined_and_its_lengths() {
    let (mut s, _, b2) = drawn_by_hand();
    let lengths = |bs: &[Value]| bs.iter().map(|b| pt(&b["head"]).distance(pt(&b["tail"]))).collect::<Vec<f32>>();
    let first = lengths(&bones(&mut s));
    let mut prev: Option<Vec<f64>> = None;
    // Round and round, in and out of reach, several times over (400 small pointer moves).
    for i in 0..400 {
        let t = i as f32 / 400.0 * std::f32::consts::TAU * 3.0;
        let r = 40.0 + 170.0 * (t * 0.5).sin().abs();
        let (x, y) = (150.0 + r * t.cos(), 100.0 + r * t.sin());
        run(&mut s, "bone.solve", json!({"bone": b2, "x": x, "y": y}));
        let bs = bones(&mut s);
        assert!(pt(&bs[0]["tail"]).distance(pt(&bs[1]["head"])) < 1e-3, "the elbow came apart at step {i}");
        for (a, b) in lengths(&bs).iter().zip(&first) {
            assert!((a - b).abs() < 1e-2, "a bone changed length at step {i}: {a} vs {b}");
        }
        let angles: Vec<f64> = bs.iter().map(|b| b["angle"].as_f64().unwrap_or(0.0)).collect();
        if let Some(p) = &prev {
            let turn = angles.iter().zip(p).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
            // About 4 px of pointer travel per step at 40–210 px from the shoulder.
            assert!(turn < 30.0, "a small move turned a joint {turn:.1}° at step {i}");
        }
        prev = Some(angles);
    }
}

#[test]
fn grabbing_a_bone_along_its_length_moves_from_where_it_was_grabbed() {
    let (mut s, b1, _) = drawn_by_hand();
    let (l, arm) = {
        let l = s.timeline().layers.iter().find(|l| l.armature.is_some()).cloned().unwrap();
        let arm = l.armature.clone().unwrap();
        (l, arm)
    };
    let (_, els) = anicraft_doc::evaluate_layer_elements(&l, 0).unwrap();
    let pose = armature::pose_at(&l, 0).unwrap_or_default();
    // Pressed a quarter of the way along the first bone, near the shoulder.
    let press = Vec2::new(175.0, 100.0);
    let local = armature::grab_point(&arm, &els, &pose, anicraft_doc::BoneId(b1), press).unwrap();
    let v = run(&mut s, "bone.solve", json!({"bone": b1, "x": press.x, "y": press.y, "local": [local.x, local.y]}));
    assert!(v["error"].as_f64().unwrap() < 0.05, "{v}");
    let bs = bones(&mut s);
    assert!(bs[0]["angle"].as_f64().unwrap().abs() < 1e-3, "pressing alone turns nothing");
    // A small drag down turns the bone a little about the shoulder: the grabbed point follows.
    run(&mut s, "bone.solve", json!({"bone": b1, "x": 175.0, "y": 105.0, "local": [local.x, local.y]}));
    let a = bones(&mut s)[0]["angle"].as_f64().unwrap();
    assert!((a - 5.0f64.atan2(25.0).to_degrees()).abs() < 0.5, "{a}°");
    // The bone's end, by contrast, would have been pulled 75 px onto the pointer.
    assert!(armature::grab_point(&arm, &els, &pose, anicraft_doc::BoneId(999), press).is_none());
}
