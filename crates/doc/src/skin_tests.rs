//! Shape armatures: binding, skinning, posing and files (ADR 0071).

use super::*;
use crate::armature::{Bone, place, pose_at, solve};
use crate::{ColorEffect, Document, Easing, Keyframe, Layer, LayerId, LayerKind, Paint, Rgba, Shape, Tween, evaluate, evaluate_layer_elements};
use std::f32::consts::FRAC_PI_2;

const TAIL: ElementId = ElementId(10);
const B1: BoneId = BoneId(20);
const B2: BoneId = BoneId(21);
const N1: ElementId = ElementId(30);
const N2: ElementId = ElementId(31);

fn bone(id: BoneId, parent: ElementId, child: ElementId, head: Vec2, tail: Vec2) -> Bone {
    Bone::new(id, parent, child, head, tail, Affine::IDENTITY)
}

/// A tail 200 px long and 20 px thick along y = 0 (anchors every 25 px along both edges), with
/// a cubic tip, drawn at (100, 100) on the stage; two bones along its middle meet at x = 100.
fn outline() -> Path {
    let mut p = Path::new();
    p.move_to(Vec2::new(0.0, -10.0));
    for i in 1..=8 {
        p.line_to(Vec2::new(25.0 * i as f32, -10.0));
    }
    p.cubic_to(Vec2::new(215.0, -10.0), Vec2::new(215.0, 10.0), Vec2::new(200.0, 10.0));
    for i in (0..8).rev() {
        p.line_to(Vec2::new(25.0 * i as f32, 10.0));
    }
    p.close();
    p
}

fn rig() -> (Armature, Vec<Arc<Element>>) {
    let path = Arc::new(outline());
    let mut arm = Armature {
        bones: vec![bone(B1, TAIL, N1, Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)), bone(B2, N1, N2, Vec2::new(100.0, 0.0), Vec2::new(200.0, 0.0))],
        skin: Some(Skin::new(TAIL, path.clone())),
        ..Default::default()
    };
    rebind(&mut arm);
    let shape = Shape { path, ..Shape::filled(Path::new(), Paint::Solid(Rgba::BLACK)) }.with_stroke(Rgba::BLACK, 3.0);
    let el = Arc::new(Element { id: TAIL, name: String::new(), transform: Affine::translate(100.0, 100.0), color: ColorEffect::default(), kind: ElementKind::Shape(shape) });
    (arm, vec![el])
}

fn path_of(els: &[Arc<Element>]) -> Arc<Path> {
    match &els[0].kind {
        ElementKind::Shape(s) => s.path.clone(),
        _ => panic!("not a shape"),
    }
}

fn close(a: Vec2, b: Vec2, tol: f32) -> bool {
    a.distance(b) <= tol
}

/// `p` turned by `a` about `j`.
fn turned(p: Vec2, j: Vec2, a: f32) -> Vec2 {
    let (s, c) = a.sin_cos();
    let d = p - j;
    Vec2::new(j.x + d.x * c - d.y * s, j.y + d.x * s + d.y * c)
}

#[test]
fn automatic_weights_sum_to_one_and_follow_the_nearest_bone() {
    let (arm, _) = rig();
    let skin = arm.skin.as_ref().unwrap();
    let pts = anchors(&skin.rest);
    assert_eq!(skin.points.len(), pts.len(), "one binding per anchor");
    for (b, p) in skin.points.iter().zip(&pts) {
        let sum: f32 = b.bones.iter().map(|i| i.1).sum();
        assert!((sum - 1.0).abs() < 1e-5, "{p:?}: weights sum to {sum}");
        assert!(b.bones.len() <= MAX_INFLUENCES && b.bones.iter().all(|i| i.1 > 0.0));
    }
    // Far along each bone an anchor follows it alone; at the joint the two share it equally.
    let at = |x: f32| skin.points.iter().zip(&pts).find(|(_, p)| **p == Vec2::new(x, -10.0)).map(|(b, _)| b.clone()).unwrap();
    assert_eq!(at(25.0).bones, vec![Influence(B1, 1.0)]);
    assert_eq!(at(175.0).bones, vec![Influence(B2, 1.0)]);
    let joint = at(100.0);
    assert!((joint.weight(B1) - 0.5).abs() < 1e-5 && (joint.weight(B2) - 0.5).abs() < 1e-5, "{joint:?}");
    // In between the nearer bone weighs more.
    let near = at(125.0);
    assert!(near.weight(B2) > near.weight(B1) && near.weight(B1) > 0.0, "{near:?}");
}

#[test]
fn the_rest_pose_reproduces_the_shape_exactly() {
    let (arm, mut els) = rig();
    let drawn = path_of(&els);
    // No pose, and a pose that lists every bone at 0.
    for pose in [Pose::default(), Pose(vec![crate::JointAngle::new(B1, 0.0), crate::JointAngle::new(B2, 0.0)])] {
        let rest = deform(&arm, &pose).unwrap();
        assert!(Arc::ptr_eq(&rest, &drawn), "the very path as drawn");
        assert!(!apply(&arm, &mut els, &pose), "nothing changes");
    }
    // A pose that turns one bone leaves the anchors only the other bone moves bit for bit.
    let mut pose = Pose::default();
    pose.set(B2, 0.7);
    let bent = deform(&arm, &pose).unwrap();
    assert_eq!(bent.cmds[1], drawn.cmds[1], "x = 25 follows the first bone only");
    assert_eq!(bent.cmds[0], drawn.cmds[0]);
}

#[test]
fn a_quarter_bend_turns_bound_points_about_the_joint() {
    let (arm, mut els) = rig();
    let drawn = path_of(&els);
    let mut pose = Pose::default();
    pose.set(B2, FRAC_PI_2);
    assert!(apply(&arm, &mut els, &pose));
    let bent = path_of(&els);
    let skin = arm.skin.as_ref().unwrap();
    let joint = Vec2::new(100.0, 0.0);
    let (rest, now) = (anchors(&drawn), anchors(&bent));
    for ((b, r), n) in skin.points.iter().zip(&rest).zip(&now) {
        let w2 = b.weight(B2);
        if w2 == 1.0 {
            assert!(close(*n, turned(*r, joint, FRAC_PI_2), 1e-3), "{r:?} → {n:?}");
        } else if w2 == 0.0 {
            assert_eq!(n, r, "{r:?} follows the first bone, which didn't turn");
        } else {
            // A shared anchor moves part of the way: the blend of rest and the full turn.
            let want = *r * (1.0 - w2) + turned(*r, joint, FRAC_PI_2) * w2;
            assert!(close(*n, want, 1e-3), "{r:?}: {n:?} vs {want:?}");
        }
    }
    // The tip (200, ±10) swings down to (90…110, 100), and its cubic handles turn with it.
    assert!(close(now[8], Vec2::new(110.0, 100.0), 1e-3), "{:?}", now[8]);
    let PathCmd::CubicTo(c1, c2, end) = bent.cmds[9] else { panic!("{:?}", bent.cmds[9]) };
    assert!(close(c1, turned(Vec2::new(215.0, -10.0), joint, FRAC_PI_2), 1e-3));
    assert!(close(c2, turned(Vec2::new(215.0, 10.0), joint, FRAC_PI_2), 1e-3));
    assert!(close(end, Vec2::new(90.0, 100.0), 1e-3));
    // The shape keeps its transform and its stroke's width.
    assert_eq!(els[0].transform, Affine::translate(100.0, 100.0));
    let ElementKind::Shape(s) = &els[0].kind else { panic!("not a shape") };
    assert_eq!(s.stroke.as_ref().map(|s| s.width), Some(3.0));
    // Turning the first bone carries the second: the tip goes round the root joint too.
    let mut pose = Pose::default();
    pose.set(B1, FRAC_PI_2);
    let tip = anchors(&deform(&arm, &pose).unwrap())[8];
    assert!(close(tip, turned(Vec2::new(200.0, -10.0), Vec2::new(0.0, 0.0), FRAC_PI_2), 1e-3), "{tip:?}");
}

#[test]
fn joint_nodes_are_placed_by_forward_kinematics() {
    let (arm, els) = rig();
    let mut pose = Pose::default();
    pose.set(B2, FRAC_PI_2);
    let w = place(&arm, &els, &pose);
    // The second bone's end, on the stage: the shape is at (100, 100), the joint at (200, 100).
    assert!(close(w[&N2].apply(Vec2::new(200.0, 0.0)), Vec2::new(200.0, 200.0), 1e-3));
    let at = crate::armature::bones_at(&arm, &els, &pose);
    assert_eq!(at.len(), 2, "bones of a shape show although their nodes are no elements");
    assert!(close(at[1].head, Vec2::new(200.0, 100.0), 1e-4) && close(at[1].tail, Vec2::new(200.0, 200.0), 1e-3));
    assert_eq!(arm.roots(), vec![TAIL]);
}

#[test]
fn inverse_kinematics_bends_the_shape() {
    let (arm, els) = rig();
    // Drag the tail's end (stage (300, 100)) up to (250, 0): both bones turn.
    let target = Vec2::new(250.0, 0.0);
    let s = solve(&arm, &els, &Pose::default(), B2, Vec2::new(200.0, 0.0), target).unwrap();
    assert!(s.error <= 0.01, "{} px off", s.error);
    assert_eq!(s.chain, 2);
    let tip = place(&arm, &els, &s.pose)[&N2].apply(Vec2::new(200.0, 0.0));
    assert!(close(tip, target, 0.02));
    // The outline's anchors on the bone's end follow it (they're bound to the end bone only).
    let mut moved = els.clone();
    apply(&arm, &mut moved, &s.pose);
    let tip_anchor = moved[0].transform.apply(anchors(&path_of(&moved))[8]);
    assert!(tip_anchor.distance(target) < 11.0, "{tip_anchor:?}");
}

fn tail_layer(easing: Easing) -> Layer {
    let (arm, els) = rig();
    let mut l = Layer::new(LayerId(1), "Armature_1");
    l.kind = LayerKind::Armature;
    l.armature = Some(Arc::new(arm));
    let mut a = Keyframe::new(0, 10);
    a.elements = els.clone();
    a.tween = Some(Tween { easing, ..Tween::default() });
    let mut b = Keyframe::new(10, 5);
    b.elements = els;
    b.pose.set(B2, FRAC_PI_2);
    l.keyframes = vec![Arc::new(a), Arc::new(b)];
    l
}

#[test]
fn poses_tween_the_outline_with_easing() {
    let l = tail_layer(Easing::Linear);
    let tip = |f: u32| {
        let (_, els) = evaluate_layer_elements(&l, f).unwrap();
        anchors(&path_of(&els))[8]
    };
    let rest = Vec2::new(200.0, -10.0);
    let joint = Vec2::new(100.0, 0.0);
    assert_eq!(tip(0), rest, "exact at the first key");
    assert!(close(tip(10), turned(rest, joint, FRAC_PI_2), 1e-3), "exact at the second key");
    // Half way the tip has turned 45°: angles tween, the outline follows (not a straight morph).
    assert!(close(tip(5), turned(rest, joint, FRAC_PI_2 * 0.5), 1e-3), "{:?}", tip(5));
    let eased = tail_layer(Easing::QuadIn);
    assert!((pose_at(&eased, 5).unwrap().angle(B2) - FRAC_PI_2 * 0.25).abs() < 1e-6);
}

#[test]
fn every_export_sees_the_bent_shape_and_files_keep_it() {
    let mut d = Document::default();
    d.timeline_mut(0).layers = vec![Arc::new(tail_layer(Easing::Linear))];
    d.sanitize();
    let dl = evaluate(&d, 10);
    let item = dl.items.iter().find(|i| i.element == TAIL).unwrap();
    let tip = item.transform.apply(anchors(&item.path)[8]);
    assert!(close(tip, Vec2::new(210.0, 200.0), 1e-3), "the display list draws the bent tail: {tip:?}");
    let json = serde_json::to_string(&d).unwrap();
    let back: Document = serde_json::from_str(&json).unwrap();
    assert_eq!(back, d);
    assert!(back.timeline(0).layers[0].armature.as_ref().unwrap().skin.is_some());
    // The joint nodes' ids are counted, so new ids never collide with them.
    assert!(d.next_id > N2.0);
    // An armature written before shape armatures reads as one without a skin.
    let old: Armature = serde_json::from_str(r#"{"bones": []}"#).unwrap();
    assert!(old.skin.is_none());
    assert!(!serde_json::to_string(&old).unwrap().contains("skin"));
}

#[test]
fn sanitize_repairs_hostile_skins() {
    let (mut arm, _) = rig();
    let skin = arm.skin.as_mut().unwrap();
    skin.points.truncate(3);
    skin.points[0] = Binding { bones: vec![Influence(B1, f32::NAN), Influence(BoneId(99), 1.0), Influence(B2, 3.0), Influence(B2, 1.0)], manual: true };
    skin.points[1] = Binding { bones: vec![Influence(B1, -1.0)], manual: false };
    skin.points[2] = Binding { bones: vec![], manual: true };
    assert!(arm.sanitize());
    let skin = arm.skin.as_ref().unwrap();
    assert_eq!(skin.points.len(), anchors(&skin.rest).len(), "one per anchor again");
    assert_eq!(skin.points[0].bones, vec![Influence(B2, 1.0)], "unknown bones, NaN and repeats go; the rest sums to 1");
    assert!(!skin.points[1].bones.is_empty(), "an automatic anchor left with nothing is bound again");
    assert!(skin.points[2].bones.is_empty(), "an anchor unbound by hand stays unbound");
    assert!(!arm.sanitize(), "sanitizing twice changes nothing");
    // A bone that starts somewhere other than the shape, and a shape that is a bone's node.
    let (mut arm, _) = rig();
    arm.bones.push(bone(BoneId(40), ElementId(77), ElementId(78), Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0)));
    arm.sanitize();
    assert_eq!(arm.bones.len(), 2, "every bone of a shape armature hangs from the shape");
    let (mut arm, _) = rig();
    arm.bones.push(bone(BoneId(41), N2, TAIL, Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0)));
    arm.sanitize();
    assert!(arm.skin.is_none() || arm.bones.iter().all(|b| b.child != TAIL));
}

#[test]
fn hand_binding_survives_new_bones() {
    let (mut arm, _) = rig();
    let skin = arm.skin.as_mut().unwrap();
    skin.points[1] = Binding { bones: vec![Influence(B2, 1.0)], manual: true };
    skin.points[2] = Binding { bones: vec![], manual: true };
    arm.bones.push(bone(BoneId(22), N2, ElementId(32), Vec2::new(200.0, 0.0), Vec2::new(210.0, 0.0)));
    rebind(&mut arm);
    let skin = arm.skin.as_ref().unwrap();
    assert_eq!(skin.points[1].bones, vec![Influence(B2, 1.0)], "kept as bound by hand");
    assert!(skin.points[2].bones.is_empty(), "kept unbound");
    // Taking its bone away sends a hand-bound anchor back to automatic.
    let mut arm = arm.clone();
    arm.bones.retain(|b| b.id != B2 && b.id != BoneId(22));
    rebind(&mut arm);
    assert_eq!(arm.skin.as_ref().unwrap().points[1].bones, vec![Influence(B1, 1.0)]);
}

/// Skinning cost: a shape of 2,000 anchors on 16 bones, posed (debug builds are slower).
#[test]
fn skinning_is_fast() {
    let mut p = Path::new();
    p.move_to(Vec2::new(0.0, -10.0));
    for i in 1..1000 {
        p.line_to(Vec2::new(i as f32 * 0.8, -10.0));
    }
    for i in (0..1000).rev() {
        p.line_to(Vec2::new(i as f32 * 0.8, 10.0));
    }
    p.close();
    let mut bones = Vec::new();
    for i in 0..16u64 {
        let parent = if i == 0 { TAIL } else { ElementId(100 + i - 1) };
        bones.push(bone(BoneId(200 + i), parent, ElementId(100 + i), Vec2::new(50.0 * i as f32, 0.0), Vec2::new(50.0 * (i + 1) as f32, 0.0)));
    }
    let mut arm = Armature { bones, skin: Some(Skin::new(TAIL, Arc::new(p))), ..Default::default() };
    let t = std::time::Instant::now();
    rebind(&mut arm);
    let bind = t.elapsed();
    let pose = Pose(arm.bones.iter().map(|b| crate::JointAngle::new(b.id, 0.1)).collect());
    let t = std::time::Instant::now();
    for _ in 0..10 {
        std::hint::black_box(deform(&arm, &pose));
    }
    let per = t.elapsed() / 10;
    println!("2,000 anchors × 16 bones: bind {bind:?}, deform {per:?}");
    assert!(per.as_millis() < 50, "{per:?}");
}
