//! Armatures, poses and inverse kinematics (ADR 0043).

use super::*;
use crate::{BlendMode, ColorEffect, Document, Easing, ElementKind, Keyframe, LayerId, LayerKind, LoopMode, Paint, Rgba, Shape, Symbol, SymbolId, SymbolKind, Timeline, Tween, evaluate, evaluate_layer_elements};
use anicraft_geom::Path;
use std::f32::consts::FRAC_PI_2;

const ROOT: ElementId = ElementId(10);
const ARM: ElementId = ElementId(11);
const HAND: ElementId = ElementId(12);
const B1: BoneId = BoneId(20);
const B2: BoneId = BoneId(21);
const SEG: SymbolId = SymbolId(1);

fn instance(id: ElementId, x: f32, y: f32) -> Arc<Element> {
    let kind = ElementKind::Symbol { symbol: SEG, loop_mode: LoopMode::Loop, first_frame: 0, blend: BlendMode::Normal, filters: Vec::new(), events: Vec::new() };
    Arc::new(Element { id, name: String::new(), transform: Affine::translate(x, y), color: ColorEffect::default(), kind })
}

fn bone(id: BoneId, parent: ElementId, child: ElementId) -> Bone {
    // Every segment is 100 px long from its registration point; bones join end to start.
    Bone::new(id, parent, child, Vec2::new(100.0, 0.0), Vec2::new(100.0, 0.0), Affine::translate(100.0, 0.0))
}

/// A three-segment chain lying along y = 100: the root from x = 100, the arm from 200 (its
/// joint), the hand from 300; the hand's end at 400. The arm and hand reach 200 px from the
/// shoulder joint at (200, 100).
fn chain() -> (Armature, Vec<Arc<Element>>) {
    let arm = Armature { bones: vec![bone(B1, ROOT, ARM), bone(B2, ARM, HAND)], ..Default::default() };
    (arm, vec![instance(ROOT, 100.0, 100.0), instance(ARM, 200.0, 100.0), instance(HAND, 300.0, 100.0)])
}

fn hand_end(arm: &Armature, els: &[Arc<Element>], pose: &Pose) -> Vec2 {
    place(arm, els, pose)[&HAND].apply(Vec2::new(100.0, 0.0))
}

fn close(a: Vec2, b: Vec2, tol: f32) -> bool {
    a.distance(b) <= tol
}

#[test]
fn forward_kinematics_turns_children_about_their_joints() {
    let (arm, els) = chain();
    let rest = place(&arm, &els, &Pose::default());
    assert!(close(rest[&HAND].translation(), Vec2::new(300.0, 100.0), 1e-4), "the rest pose is the drawing");
    let mut pose = Pose::default();
    pose.set(B1, FRAC_PI_2);
    let w = place(&arm, &els, &pose);
    // The arm turns a quarter clockwise about the shoulder (200, 100): it now points down.
    assert!(close(w[&ARM].translation(), Vec2::new(200.0, 100.0), 1e-4));
    assert!(close(w[&HAND].translation(), Vec2::new(200.0, 200.0), 1e-3), "{:?}", w[&HAND]);
    assert!(close(hand_end(&arm, &els, &pose), Vec2::new(200.0, 300.0), 1e-3));
    // The elbow turns relative to the arm: a quarter back makes the hand point right again.
    pose.set(B2, -FRAC_PI_2);
    assert!(close(hand_end(&arm, &els, &pose), Vec2::new(300.0, 200.0), 1e-3));
    // The root keeps its own transform.
    assert_eq!(place(&arm, &els, &pose)[&ROOT], Affine::translate(100.0, 100.0));
}

#[test]
fn ik_reaches_a_reachable_target_within_half_a_pixel() {
    let (arm, els) = chain();
    for target in [Vec2::new(300.0, 180.0), Vec2::new(150.0, 200.0), Vec2::new(250.0, 20.0), Vec2::new(390.0, 110.0), Vec2::new(210.0, 120.0)] {
        let s = solve(&arm, &els, &Pose::default(), B2, Vec2::new(100.0, 0.0), target).unwrap();
        let end = hand_end(&arm, &els, &s.pose);
        assert!(close(end, target, 0.5), "{target:?}: ended at {end:?} ({} px off)", s.error);
        assert!((s.error - end.distance(target)).abs() < 1e-3);
        assert_eq!(s.chain, 2);
        // The root never moves.
        assert_eq!(place(&arm, &els, &s.pose)[&ROOT], Affine::translate(100.0, 100.0));
    }
}

#[test]
fn ik_respects_joint_limits() {
    let (mut arm, els) = chain();
    let lim = Limits { min: -10f32.to_radians(), max: 10f32.to_radians() };
    arm.bone_mut(B2).unwrap().limits = Some(lim);
    // A target that needs a sharp elbow: the elbow stops at its limit.
    let s = solve(&arm, &els, &Pose::default(), B2, Vec2::new(100.0, 0.0), Vec2::new(200.0, 160.0)).unwrap();
    let a = s.pose.angle(B2);
    assert!((lim.min..=lim.max).contains(&a), "elbow at {}°", a.to_degrees());
    // Evaluation clamps too, whatever a file says.
    let mut wild = Pose::default();
    wild.set(B2, 3.0);
    assert_eq!(wild.allowed(&arm).angle(B2), lim.max);
    // A joint that may not turn at all keeps its angle.
    arm.bone_mut(B1).unwrap().limits = Some(Limits { min: 0.0, max: 0.0 });
    let s = solve(&arm, &els, &Pose::default(), B2, Vec2::new(100.0, 0.0), Vec2::new(300.0, 300.0)).unwrap();
    assert_eq!(s.pose.angle(B1), 0.0);
}

#[test]
fn an_unreachable_target_stretches_the_chain_straight_and_the_root_stays() {
    let (arm, els) = chain();
    let target = Vec2::new(700.0, 500.0);
    let mut start = Pose::default();
    start.set(B1, 0.7);
    start.set(B2, -1.2);
    let s = solve(&arm, &els, &start, B2, Vec2::new(100.0, 0.0), target).unwrap();
    let w = place(&arm, &els, &s.pose);
    let (shoulder, elbow, end) = (Vec2::new(200.0, 100.0), w[&HAND].translation(), hand_end(&arm, &els, &s.pose));
    let (d1, d2, dt) = ((elbow - shoulder).normalized(), (end - elbow).normalized(), (target - shoulder).normalized());
    assert!(d1.cross(d2).abs() < 1e-3 && d1.dot(d2) > 0.999, "straight: {d1:?} {d2:?}");
    assert!(d1.dot(dt) > 0.999, "pointing at the target");
    assert!((s.error - (target.distance(shoulder) - 200.0)).abs() < 0.05, "{}", s.error);
    assert_eq!(w[&ROOT], Affine::translate(100.0, 100.0), "the root stays put");
}

#[test]
fn pinned_bones_stay_put() {
    let (mut arm, els) = chain();
    arm.bone_mut(B1).unwrap().pinned = true;
    let s = solve(&arm, &els, &Pose::default(), B2, Vec2::new(100.0, 0.0), Vec2::new(250.0, 180.0)).unwrap();
    assert_eq!(s.pose.angle(B1), 0.0, "the pinned shoulder never turns");
    assert_eq!(s.chain, 1);
    // Dragging the pinned bone itself moves nothing.
    let s = solve(&arm, &els, &Pose::default(), B1, Vec2::new(100.0, 0.0), Vec2::new(0.0, 0.0)).unwrap();
    assert_eq!((s.chain, s.pose.angle(B1)), (0, 0.0));
    // A pin locks the bones it hangs from too.
    arm.bone_mut(B1).unwrap().pinned = false;
    arm.bone_mut(B2).unwrap().pinned = true;
    assert!(movable_chain(&arm, B1).is_empty(), "turning the shoulder would move the pinned hand");
    assert!(movable_chain(&arm, B2).is_empty());
}

/// An armature layer: the chain at frame 0 posed straight, at frame 10 with the shoulder a
/// quarter turned, tweened with `easing`.
fn armature_layer(easing: Easing) -> Layer {
    let (arm, els) = chain();
    let mut l = Layer::new(LayerId(1), "Armature_1");
    l.kind = LayerKind::Armature;
    l.armature = Some(Arc::new(arm));
    let mut a = Keyframe::new(0, 10);
    a.elements = els.clone();
    a.tween = Some(Tween { easing, ..Tween::default() });
    let mut b = Keyframe::new(10, 5);
    b.elements = els;
    b.pose.set(B1, FRAC_PI_2);
    l.keyframes = vec![Arc::new(a), Arc::new(b)];
    l
}

#[test]
fn poses_tween_and_are_exact_at_keyframes() {
    let l = armature_layer(Easing::Linear);
    assert_eq!(pose_at(&l, 0).unwrap().angle(B1), 0.0);
    assert_eq!(pose_at(&l, 10).unwrap().angle(B1), FRAC_PI_2, "exact at the key");
    assert_eq!(pose_at(&l, 14).unwrap().angle(B1), FRAC_PI_2, "held after the last key");
    assert!((pose_at(&l, 5).unwrap().angle(B1) - FRAC_PI_2 * 0.5).abs() < 1e-6);
    // The existing easing shapes the tween.
    let eased = armature_layer(Easing::QuadIn);
    assert!((pose_at(&eased, 5).unwrap().angle(B1) - FRAC_PI_2 * 0.25).abs() < 1e-6);
    // Evaluation places the instances by the pose: at the key, exactly forward kinematics.
    let (_, els) = evaluate_layer_elements(&l, 10).unwrap();
    let hand = els.iter().find(|e| e.id == HAND).unwrap();
    assert!(close(hand.transform.translation(), Vec2::new(200.0, 200.0), 1e-3));
    // Half way the hand has swung through 45° about the shoulder.
    let (_, els) = evaluate_layer_elements(&l, 5).unwrap();
    let hand = els.iter().find(|e| e.id == HAND).unwrap();
    let r = std::f32::consts::FRAC_1_SQRT_2 * 100.0;
    assert!(close(hand.transform.translation(), Vec2::new(200.0 + r, 100.0 + r), 1e-3), "{:?}", hand.transform);
    // Without the tween the first pose holds.
    let mut held = armature_layer(Easing::Linear);
    Arc::make_mut(&mut held.keyframes[0]).tween = None;
    assert_eq!(pose_at(&held, 9).unwrap().angle(B1), 0.0);
}

#[test]
fn every_export_sees_the_pose() {
    let mut d = Document::default();
    let mut seg = Layer::new(LayerId(2), "seg");
    let mut k = Keyframe::new(0, 1);
    k.elements.push(Arc::new(Element { id: ElementId(3), name: String::new(), transform: Affine::IDENTITY, color: ColorEffect::default(), kind: ElementKind::Shape(Shape::filled(Path::rect(0.0, -5.0, 100.0, 10.0), Paint::Solid(Rgba::BLACK))) }));
    seg.keyframes = vec![Arc::new(k)];
    d.library.push(Arc::new(Symbol { id: SEG, name: "Seg".into(), kind: SymbolKind::Graphic, linkage: None, scale_grid_kept: None, scale_grid: None, timeline: Timeline { layers: vec![Arc::new(seg)] } }));
    d.timeline_mut(0).layers = vec![Arc::new(armature_layer(Easing::Linear))];
    d.sanitize();
    let dl = evaluate(&d, 10);
    let hand = dl.items.iter().find(|i| i.element == HAND).unwrap();
    assert!(close(hand.transform.apply(Vec2::new(100.0, 0.0)), Vec2::new(200.0, 300.0), 1e-3), "the display list draws the posed hand");
    // The file keeps the armature and poses.
    let back: Document = serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap();
    assert_eq!(back, d);
    assert_eq!(back.timeline(0).layers[0].keyframes[1].pose.angle(B1), FRAC_PI_2);
}

#[test]
fn sanitize_repairs_hostile_armatures() {
    let (mut arm, _) = chain();
    // Listed child first, a cycle, a second bone into the hand, a bone into itself, NaN.
    let mut cyc = bone(BoneId(30), ElementId(40), ElementId(41));
    cyc.head = Vec2::new(0.0, 0.0);
    let back = bone(BoneId(31), ElementId(41), ElementId(40));
    let twice = bone(BoneId(32), ROOT, HAND);
    let selfie = bone(BoneId(33), ROOT, ROOT);
    let mut nan = bone(BoneId(34), HAND, ElementId(50));
    nan.rest = Affine { m: [f32::NAN, 0.0, 0.0, 1.0, 0.0, 0.0] };
    let mut wide = bone(BoneId(35), HAND, ElementId(51));
    wide.limits = Some(Limits { min: 100.0, max: f32::NEG_INFINITY });
    let mut limited = bone(BoneId(36), HAND, ElementId(52));
    limited.limits = Some(Limits { min: 1.0, max: -1.0 });
    arm.bones.reverse();
    arm.bones.extend([cyc, back, twice, selfie, nan, wide, limited]);
    assert!(arm.sanitize());
    let ids: Vec<u64> = arm.bones.iter().map(|b| b.id.0).collect();
    assert_eq!(ids, vec![20, 21, 35, 36], "ordered parent first; cycles, repeats and NaN gone");
    assert_eq!(arm.bone(BoneId(35)).unwrap().limits, None);
    assert_eq!(arm.bone(BoneId(36)).unwrap().limits, Some(Limits { min: -1.0, max: 1.0 }));
    assert!(!arm.sanitize(), "sanitizing twice changes nothing");
    let mut p = Pose(vec![JointAngle::new(B1, f32::NAN), JointAngle::new(B1, 1.0), JointAngle::new(B2, 1e30)]);
    assert!(p.sanitize());
    assert_eq!(p.0.len(), 2);
    assert_eq!((p.angle(B1), p.angle(B2)), (0.0, MAX_ANGLE));
    // Hostile solves return nothing rather than panicking.
    let (arm, els) = chain();
    assert!(solve(&arm, &els, &Pose::default(), BoneId(999), Vec2::new(0.0, 0.0), Vec2::new(1.0, 1.0)).is_none());
    assert!(solve(&arm, &els, &Pose::default(), B2, Vec2::new(f32::NAN, 0.0), Vec2::new(1.0, 1.0)).is_none());
    assert!(solve(&arm, &[], &Pose::default(), B2, Vec2::new(0.0, 0.0), Vec2::new(1.0, 1.0)).is_none());
}

#[test]
fn tree_queries() {
    let (arm, _) = chain();
    assert_eq!(arm.roots(), vec![ROOT]);
    assert_eq!(arm.members(), vec![ROOT, ARM, HAND]);
    assert_eq!(arm.subtree(B1), vec![B1, B2]);
    assert_eq!(arm.subtree(B2), vec![B2]);
    assert_eq!(arm.chain(B2).iter().map(|b| b.id).collect::<Vec<_>>(), vec![B2, B1]);
    assert_eq!(arm.bone_into(HAND).map(|b| b.id), Some(B2));
    assert!(arm.contains(ROOT) && !arm.contains(ElementId(99)));
}

/// Long chains and targets on a straight chain's own line (CCD's blind spot) still converge.
#[test]
fn long_chains_and_collinear_targets_converge() {
    for n in [2usize, 10, 64] {
        let mut bones = Vec::new();
        let mut els = vec![instance(ElementId(1000), 0.0, 0.0)];
        for i in 0..n {
            let (p, c) = (ElementId(1000 + i as u64), ElementId(1001 + i as u64));
            bones.push(Bone { id: BoneId(5000 + i as u64), ..bone(BoneId(0), p, c) });
            els.push(instance(c, 100.0 * (i + 1) as f32, 0.0));
        }
        let arm = Armature { bones, ..Default::default() };
        let last = BoneId(5000 + n as u64 - 1);
        let reach = 100.0 * n as f32;
        for k in 0..12 {
            // k = 0: straight ahead along the chain.
            let a = k as f32 * 0.55;
            let target = Vec2::new(100.0 + reach * 0.6 * a.cos(), reach * 0.6 * a.sin());
            let s = solve(&arm, &els, &Pose::default(), last, Vec2::new(100.0, 0.0), target).unwrap();
            assert!(s.error <= 0.5, "{n} bones to {target:?}: {} px off", s.error);
        }
    }
}

// Joint translation (ADR 0084).

#[test]
fn a_sliding_joint_moves_what_hangs_from_it_along_its_parents_axes() {
    let (mut arm, els) = chain();
    arm.bone_mut(B2).unwrap().slide_x = Some(Limits { min: -50.0, max: 50.0 });
    let mut pose = Pose::default();
    pose.set_slide(B2, Vec2::new(30.0, 99.0));
    // Only x slides (y isn't enabled), along the arm's x axis: right, at rest.
    assert_eq!(pose.allowed(&arm).slide(B2), Vec2::new(30.0, 0.0));
    assert!(close(hand_end(&arm, &els, &pose), Vec2::new(430.0, 100.0), 1e-3));
    // With the shoulder turned a quarter, the arm's x axis points down: so does the slide.
    pose.set(B1, FRAC_PI_2);
    assert!(close(hand_end(&arm, &els, &pose), Vec2::new(200.0, 330.0), 1e-3), "{:?}", hand_end(&arm, &els, &pose));
    // The joint is drawn where it slid to.
    let posed: Vec<Arc<Element>> = {
        let w = place(&arm, &els, &pose);
        els.iter().map(|e| Arc::new(Element { transform: w[&e.id], ..(**e).clone() })).collect()
    };
    let at = bones_at(&arm, &posed, &pose.allowed(&arm));
    assert!(close(at[1].head, Vec2::new(200.0, 230.0), 1e-3), "{:?}", at[1]);
    // The limits hold, whatever a file says.
    pose.set_slide(B2, Vec2::new(1e9, 0.0));
    assert_eq!(pose.allowed(&arm).slide(B2).x, 50.0);
    // No slide: exactly the bits of turning alone.
    let (plain, _) = chain();
    let p = Pose(vec![JointAngle::new(B1, 0.3), JointAngle::new(B2, -0.2)]);
    assert_eq!(place(&arm, &els, &p), place(&plain, &els, &p));
}

#[test]
fn ik_slides_a_joint_only_where_turning_cant_reach_and_within_its_limits() {
    let (mut arm, els) = chain();
    arm.bone_mut(B2).unwrap().slide_x = Some(Limits { min: 0.0, max: 80.0 });
    // Within reach: the same solve as without sliding (no slide at all).
    let (plain, _) = chain();
    let t = Vec2::new(300.0, 180.0);
    let (a, b) = (solve(&arm, &els, &Pose::default(), B2, Vec2::new(100.0, 0.0), t).unwrap(), solve(&plain, &els, &Pose::default(), B2, Vec2::new(100.0, 0.0), t).unwrap());
    assert_eq!(a.pose.slide(B2), Vec2::ZERO);
    assert_eq!(a.pose.angle(B1).to_bits(), b.pose.angle(B1).to_bits());
    // Out of reach: the elbow slides out to get there.
    let far = Vec2::new(250.0, 330.0);
    let s = solve(&arm, &els, &Pose::default(), B2, Vec2::new(100.0, 0.0), far).unwrap();
    assert!(s.error <= 0.5, "{} px off, slide {:?}", s.error, s.pose.slide(B2));
    assert!(s.pose.slide(B2).x > 0.0 && s.pose.slide(B2).x <= 80.0);
    // Farther than the slide allows: it stops at its limit, stretched toward the target.
    let s = solve(&arm, &els, &Pose::default(), B2, Vec2::new(100.0, 0.0), Vec2::new(900.0, 100.0)).unwrap();
    assert_eq!(s.pose.slide(B2).x, 80.0);
    assert!((s.error - (900.0 - 480.0)).abs() < 0.05, "{}", s.error);
    // A piston: a joint that doesn't turn but slides along y.
    let (mut arm, els) = chain();
    let b = arm.bone_mut(B2).unwrap();
    b.limits = Some(Limits { min: 0.0, max: 0.0 });
    b.slide_y = Some(Limits { min: -40.0, max: 40.0 });
    arm.bone_mut(B1).unwrap().limits = Some(Limits { min: 0.0, max: 0.0 });
    let s = solve(&arm, &els, &Pose::default(), B2, Vec2::new(100.0, 0.0), Vec2::new(400.0, 125.0)).unwrap();
    assert!((s.pose.slide(B2).y - 25.0).abs() < 0.01 && s.pose.angle(B2) == 0.0, "{:?}", s.pose);
}

#[test]
fn sanitize_repairs_hostile_slides() {
    let (mut arm, _) = chain();
    let b = arm.bone_mut(B2).unwrap();
    b.slide_x = Some(Limits { min: f32::NAN, max: 3.0 });
    b.slide_y = Some(Limits { min: 1e12, max: -1e12 });
    b.spring = Some(Spring { strength: f32::INFINITY, damping: 0.5 });
    assert!(arm.sanitize());
    let b = arm.bone(B2).unwrap();
    assert_eq!((b.slide_x, b.slide_y, b.spring), (None, Some(Limits { min: -MAX_SLIDE, max: MAX_SLIDE }), None));
    let mut p = Pose(vec![JointAngle { bone: B2, angle: 0.0, x: f32::NAN, y: -1e20 }]);
    assert!(p.sanitize());
    assert_eq!(p.slide(B2), Vec2::new(0.0, -MAX_SLIDE));
    // Older poses (no slides) still read; slides of 0 are not written.
    let old: Pose = serde_json::from_str(r#"[{"bone":20,"angle":0.5}]"#).unwrap();
    assert_eq!(old.slide(B1), Vec2::ZERO);
    assert_eq!(serde_json::to_string(&old).unwrap(), r#"[{"bone":20,"angle":0.5}]"#);
}
