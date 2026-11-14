//! Painted weights and reshaping (ADR 0090).

use super::*;
use crate::armature::{Armature, Bone, Pose};
use crate::skin::{deform, rebind, rest_segments};
use crate::ElementId;
use anicraft_geom::Affine;

const SHAPE: ElementId = ElementId(10);
const B1: BoneId = BoneId(20);
const B2: BoneId = BoneId(21);

/// A bar 200 px long, 20 px thick along y = 0 with anchors every 25 px on both edges, and two
/// bones along its middle meeting at x = 100 (the shape drawn at the origin).
fn rig() -> Armature {
    let mut p = Path::new();
    p.move_to(Vec2::new(0.0, -10.0));
    for i in 1..=8 {
        p.line_to(Vec2::new(25.0 * i as f32, -10.0));
    }
    for i in (0..=8).rev() {
        p.line_to(Vec2::new(25.0 * i as f32, 10.0));
    }
    p.close();
    let bones = vec![
        Bone::new(B1, SHAPE, ElementId(30), Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0), Affine::IDENTITY),
        Bone::new(B2, ElementId(30), ElementId(31), Vec2::new(100.0, 0.0), Vec2::new(200.0, 0.0), Affine::IDENTITY),
    ];
    let mut arm = Armature { bones, skin: Some(Skin::new(SHAPE, Arc::new(p))), ..Default::default() };
    rebind(&mut arm);
    arm
}

fn skin(arm: &Armature) -> &Skin {
    arm.skin.as_ref().expect("a skin")
}

fn sums_to_one(b: &Binding) -> bool {
    b.bones.is_empty() || ((b.bones.iter().map(|i| i.1).sum::<f32>() - 1.0).abs() < 1e-5 && b.bones.len() <= crate::skin::MAX_INFLUENCES && b.bones.iter().all(|i| i.1 > 0.0))
}

fn paint_on(arm: &mut Armature, bone: BoneId, dabs: &[Vec2], radius: f32, strength: f32, brush: Brush) -> usize {
    let segments = rest_segments(arm);
    let shown = anchors(&skin(arm).rest);
    let k = arm.skin.as_mut().expect("a skin");
    paint(k, &segments, &shown, bone, dabs, Dab { brush, radius, strength })
}

#[test]
fn falloff_is_one_at_the_centre_and_zero_at_the_rim() {
    assert_eq!(falloff(0.0, 10.0), 1.0);
    assert_eq!(falloff(10.0, 10.0), 0.0);
    assert_eq!(falloff(20.0, 10.0), 0.0);
    assert!(falloff(5.0, 10.0) > 0.5 && falloff(5.0, 10.0) < 0.6);
    assert_eq!(falloff(1.0, 0.0), 0.0);
    assert_eq!(falloff(f32::NAN, 10.0), 0.0);
}

#[test]
fn painted_weights_stay_normalised() {
    let mut arm = rig();
    for brush in [Brush::Add, Brush::Subtract, Brush::Smooth, Brush::Add] {
        for bone in [B1, B2] {
            let dabs: Vec<Vec2> = (0..40).map(|i| Vec2::new(5.0 * i as f32, if i % 2 == 0 { -10.0 } else { 10.0 })).collect();
            paint_on(&mut arm, bone, &dabs, 30.0, 0.35, brush);
            assert!(skin(&arm).points.iter().all(sums_to_one), "{brush:?} on {bone:?}: {:?}", skin(&arm).points);
        }
    }
}

#[test]
fn painting_is_local_to_the_radius() {
    let mut arm = rig();
    let before = skin(&arm).points.clone();
    let centre = Vec2::new(25.0, -10.0);
    let changed = paint_on(&mut arm, B2, &[centre], 30.0, 1.0, Brush::Add);
    let pts = anchors(&skin(&arm).rest);
    assert!(changed > 0);
    for (i, p) in pts.iter().enumerate() {
        if p.distance(centre) >= 30.0 {
            assert_eq!(skin(&arm).points[i], before[i], "point {i} at {p:?} is outside the brush");
        }
    }
    // The point under the centre now follows B2 entirely (strength 100 at the centre), and is
    // hand-bound, so a new bone doesn't undo it.
    assert_eq!(skin(&arm).points[1].weight(B2), 1.0);
    assert!(skin(&arm).points[1].manual);
}

#[test]
fn add_raises_subtract_lowers_and_smooth_evens_out() {
    let mut arm = rig();
    let at = Vec2::new(100.0, -10.0);
    let w0 = skin(&arm).points[4].weight(B1);
    paint_on(&mut arm, B1, &[at], 20.0, 0.2, Brush::Add);
    let w1 = skin(&arm).points[4].weight(B1);
    assert!(w1 > w0, "{w0} → {w1}");
    paint_on(&mut arm, B1, &[at, at, at], 20.0, 0.2, Brush::Subtract);
    let w2 = skin(&arm).points[4].weight(B1);
    assert!(w2 < w1, "{w1} → {w2}");
    // A point that follows only B1 gives what it loses to B2, its nearest other bone.
    let mut arm = rig();
    assert_eq!(skin(&arm).points[1].weight(B1), 1.0);
    paint_on(&mut arm, B1, &[Vec2::new(25.0, -10.0)], 5.0, 0.5, Brush::Subtract);
    let b = &skin(&arm).points[1];
    assert!((b.weight(B1) - 0.5).abs() < 1e-5 && (b.weight(B2) - 0.5).abs() < 1e-5, "{b:?}");
    // Smoothing a sharp step makes the bone's shares closer to each other.
    let mut arm = rig();
    let spread = |arm: &Armature| {
        let w: Vec<f32> = skin(arm).points.iter().map(|b| b.weight(B1)).collect();
        w.iter().cloned().fold(f32::MIN, f32::max) - w.iter().cloned().fold(f32::MAX, f32::min)
    };
    let before = spread(&arm);
    let dabs: Vec<Vec2> = (0..=16).map(|i| Vec2::new(12.5 * i as f32, 0.0)).collect();
    for _ in 0..10 {
        paint_on(&mut arm, B1, &dabs, 400.0, 1.0, Brush::Smooth);
    }
    assert!(spread(&arm) < before * 0.5, "{before} → {}", spread(&arm));
    assert!(skin(&arm).points.iter().all(sums_to_one));
}

#[test]
fn nothing_paints_without_strength_or_radius() {
    let mut arm = rig();
    let before = skin(&arm).points.clone();
    assert_eq!(paint_on(&mut arm, B1, &[Vec2::new(25.0, -10.0)], 0.0, 1.0, Brush::Add), 0);
    assert_eq!(paint_on(&mut arm, B1, &[Vec2::new(25.0, -10.0)], 30.0, 0.0, Brush::Add), 0);
    assert_eq!(paint_on(&mut arm, B1, &[Vec2::new(f32::NAN, 0.0)], 30.0, 1.0, Brush::Add), 0);
    assert_eq!(skin(&arm).points, before);
}

#[test]
fn with_share_keeps_the_other_bones_proportions() {
    let b = Binding { bones: vec![Influence(B1, 0.5), Influence(B2, 0.3), Influence(BoneId(9), 0.2)], manual: false };
    let x = with_share(&b, B1, 0.8, Vec::new);
    assert!((x.weight(B1) - 0.8).abs() < 1e-6);
    assert!((x.weight(B2) - 0.12).abs() < 1e-6 && (x.weight(BoneId(9)) - 0.08).abs() < 1e-6);
    assert!(x.manual && sums_to_one(&x));
    // Without other bones, the fallback takes the rest; without any, the bone keeps it all.
    let only = Binding { bones: vec![Influence(B1, 1.0)], manual: false };
    assert!((with_share(&only, B1, 0.25, || vec![Influence(B2, 1.0)]).weight(B2) - 0.75).abs() < 1e-6);
    assert_eq!(with_share(&only, B1, 0.25, Vec::new).weight(B1), 1.0);
    assert!(with_share(&only, B1, 0.0, Vec::new).bones.is_empty());
}

/// The shape's rest outline with its edges' anchors where `edit` puts them.
fn edited(arm: &Armature, edit: impl FnOnce(&mut Path)) -> Path {
    let mut p = (*skin(arm).rest).clone();
    edit(&mut p);
    p
}

#[test]
fn moving_points_keeps_every_binding() {
    let arm = rig();
    let p = edited(&arm, |p| {
        if let Some(PathCmd::LineTo(q)) = p.cmds.get_mut(3) {
            q.y -= 15.0;
        }
    });
    let out = reshape(skin(&arm), &p, &rest_segments(&arm));
    assert_eq!(out, skin(&arm).points);
}

#[test]
fn an_inserted_point_blends_its_neighbours() {
    let mut arm = rig();
    // Make the neighbours different by hand: anchor 3 (x = 75) follows B1, anchor 4 (x = 100) B2.
    if let Some(k) = arm.skin.as_mut() {
        k.points[3] = Binding { bones: vec![Influence(B1, 1.0)], manual: true };
        k.points[4] = Binding { bones: vec![Influence(B2, 1.0)], manual: true };
    }
    // A point a quarter of the way from x = 75 to x = 100.
    let p = edited(&arm, |p| {
        p.cmds.insert(4, PathCmd::LineTo(Vec2::new(81.25, -10.0)));
    });
    let out = reshape(skin(&arm), &p, &rest_segments(&arm));
    assert_eq!(out.len(), anchors(&p).len());
    let b = &out[4];
    assert!((b.weight(B1) - 0.75).abs() < 1e-5 && (b.weight(B2) - 0.25).abs() < 1e-5, "{b:?}");
    assert!(b.manual && sums_to_one(b));
    // Every other anchor keeps its binding.
    assert_eq!(&out[..4], &skin(&arm).points[..4]);
    assert_eq!(&out[5..], &skin(&arm).points[4..]);
}

#[test]
fn a_deleted_point_drops_its_binding() {
    let mut arm = rig();
    if let Some(k) = arm.skin.as_mut() {
        k.points[5] = Binding { bones: vec![Influence(B1, 0.6), Influence(B2, 0.4)], manual: true };
    }
    let p = edited(&arm, |p| {
        p.cmds.remove(4);
    });
    let out = reshape(skin(&arm), &p, &rest_segments(&arm));
    let mut want = skin(&arm).points.clone();
    want.remove(4);
    assert_eq!(out, want);
}

#[test]
fn a_new_subpath_is_bound_automatically_and_closing_points_copy_the_start() {
    let arm = rig();
    let p = edited(&arm, |p| {
        p.move_to(Vec2::new(150.0, -5.0));
        p.line_to(Vec2::new(160.0, -5.0));
        p.line_to(Vec2::new(160.0, 5.0));
        p.close();
    });
    let segments = rest_segments(&arm);
    let out = reshape(skin(&arm), &p, &segments);
    let n = skin(&arm).points.len();
    assert_eq!(out.len(), n + 3);
    assert_eq!(out[n], auto_binding(&segments, Vec2::new(150.0, -5.0)));
    // An explicit closing point at the start of a closed subpath takes the start's weights.
    let p = edited(&arm, |p| {
        let close = p.cmds.len() - 1;
        p.cmds.insert(close, PathCmd::LineTo(Vec2::new(0.0, -10.0)));
    });
    let out = reshape(skin(&arm), &p, &segments);
    assert_eq!(out.last(), skin(&arm).points.first());
}

#[test]
fn the_rest_pose_is_the_reshaped_outline_bit_for_bit() {
    let arm = rig();
    let p = Arc::new(edited(&arm, |p| {
        p.cmds.insert(4, PathCmd::LineTo(Vec2::new(87.5, -10.0)));
        if let Some(PathCmd::LineTo(q)) = p.cmds.get_mut(10) {
            q.y -= 3.0;
        }
    }));
    let mut arm2 = arm.clone();
    arm2.skin = Some(reshaped(skin(&arm), p.clone(), &rest_segments(&arm)));
    let rest = deform(&arm2, &Pose::default()).expect("an outline");
    assert!(Arc::ptr_eq(&rest, &p));
    // Bent, the new point moves with the bones like its neighbours: between them.
    let mut pose = Pose::default();
    pose.set(B2, std::f32::consts::FRAC_PI_2);
    let bent = anchors(&deform(&arm2, &pose).expect("an outline"));
    assert_eq!(bent.len(), anchors(&p).len());
    assert!(bent.iter().all(|q| q.x.is_finite() && q.y.is_finite()));
}

#[test]
fn painting_a_large_shape_is_fast() {
    // 20,000 points in a row, 32 dabs of radius 40.
    let mut p = Path::new();
    p.move_to(Vec2::new(0.0, 0.0));
    for i in 1..20_000 {
        p.line_to(Vec2::new(i as f32 * 0.1, (i % 7) as f32));
    }
    let bones = vec![Bone::new(B1, SHAPE, ElementId(30), Vec2::new(0.0, 0.0), Vec2::new(1000.0, 0.0), Affine::IDENTITY), Bone::new(B2, ElementId(30), ElementId(31), Vec2::new(1000.0, 0.0), Vec2::new(2000.0, 0.0), Affine::IDENTITY)];
    let mut arm = Armature { bones, skin: Some(Skin::new(SHAPE, Arc::new(p))), ..Default::default() };
    rebind(&mut arm);
    let dabs: Vec<Vec2> = (0..32).map(|i| Vec2::new(900.0 + 10.0 * i as f32, 3.0)).collect();
    let t = std::time::Instant::now();
    let n = paint_on(&mut arm, B2, &dabs, 40.0, 0.3, Brush::Add);
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    eprintln!("painting 32 dabs over 20,000 points changed {n} in {ms:.2} ms");
    assert!(n > 0);
    assert!(ms < 2000.0);
}
