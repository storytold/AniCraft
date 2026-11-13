//! Bone springs (ADR 0084): secondary motion that is a pure function of time.

use super::*;
use crate::armature::{Bone, BoneId, Limits, Spring};
use crate::{BlendMode, ColorEffect, Document, Element, ElementKind, Keyframe, LayerId, LayerKind, LoopMode, Paint, Rgba, Shape, Symbol, SymbolId, SymbolKind, Timeline, Tween, evaluate};
use anicraft_geom::Path;
use std::sync::Arc;

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
    Bone::new(id, parent, child, Vec2::new(100.0, 0.0), Vec2::new(100.0, 0.0), Affine::translate(100.0, 0.0))
}

/// A tail of two bones lying to the right of its body (the root), which drops 300 px between
/// frames 0 and 10 and then holds still until frame 300. Both bones have `spring`.
fn tail(spring: Option<Spring>) -> Layer {
    let mut b1 = bone(B1, ROOT, ARM);
    let mut b2 = bone(B2, ARM, HAND);
    b1.spring = spring;
    b2.spring = spring;
    let arm = crate::armature::Armature { bones: vec![b1, b2], ..Default::default() };
    let mut l = Layer::new(LayerId(1), "Armature_1");
    l.kind = LayerKind::Armature;
    l.armature = Some(Arc::new(arm));
    let mut a = Keyframe::new(0, 10);
    a.elements = vec![instance(ROOT, 100.0, 100.0), instance(ARM, 200.0, 100.0), instance(HAND, 300.0, 100.0)];
    a.tween = Some(Tween::default());
    let mut b = Keyframe::new(10, 290);
    b.elements = vec![instance(ROOT, 100.0, 400.0), instance(ARM, 200.0, 400.0), instance(HAND, 300.0, 400.0)];
    l.keyframes = vec![Arc::new(a), Arc::new(b)];
    l
}

const BOUNCY: Spring = Spring { strength: 0.4, damping: 0.25 };

fn bits(p: &Pose) -> Vec<(u64, u32, u32, u32)> {
    p.0.iter().map(|j| (j.bone.0, j.angle.to_bits(), j.x.to_bits(), j.y.to_bits())).collect()
}

#[test]
fn a_spring_swings_the_tail_behind_the_motion_and_back() {
    let l = tail(Some(BOUNCY));
    // As the body drops, the tail lags: it turns up (a negative angle, y down).
    let early = sprung_pose(&l, 5, 24.0).unwrap();
    assert!(early.angle(B1) < -0.05, "lags at {}°", early.angle(B1).to_degrees());
    // Once the body stops, the tail swings on past the keyed pose (down), then settles.
    let swing: Vec<f32> = (10..60).map(|f| sprung_pose(&l, f, 24.0).unwrap().angle(B1)).collect();
    assert!(swing.iter().any(|a| *a > 0.05), "overshoots: {swing:?}");
    let late = sprung_pose(&l, 110, 24.0).unwrap();
    assert!(late.angle(B1).abs() < 0.01 && late.angle(B2).abs() < 0.01, "settling: {}° {}°", late.angle(B1).to_degrees(), late.angle(B2).to_degrees());
    // The keyed pose is untouched: springs are evaluated, never stored.
    assert_eq!(crate::armature::pose_at(&l, 5).unwrap().angle(B1), 0.0);
}

#[test]
fn the_spring_settles_to_the_keyed_pose_exactly() {
    let l = tail(Some(BOUNCY));
    // A window's length after the body stopped, the shown pose is the keyed one bit for bit.
    let after = 10 + (24.0 * SPRING_SECONDS) as u32 + 3;
    assert_eq!(bits(&sprung_pose(&l, after, 24.0).unwrap()), bits(&crate::armature::pose_at(&l, after).unwrap().allowed(l.armature.as_deref().unwrap())));
    // Before anything moves nothing swings.
    assert_eq!(sprung_pose(&l, 0, 24.0).unwrap().angle(B1), 0.0);
}

#[test]
fn seeking_equals_playing_bit_for_bit() {
    let l = tail(Some(BOUNCY));
    let played: Vec<_> = (0..140).map(|f| bits(&sprung_pose(&l, f, 24.0).unwrap())).collect();
    // Any order, any number of times: the same bits.
    let mut order: Vec<u32> = (0..140).collect();
    order.reverse();
    order.rotate_left(37);
    for f in order {
        assert_eq!(bits(&sprung_pose(&l, f, 24.0).unwrap()), played[f as usize], "frame {f}");
    }
}

#[test]
fn damping_one_strength_zero_and_springs_off_are_no_spring() {
    let keyed = tail(None);
    let cases = [tail(Some(Spring { strength: 0.6, damping: 1.0 })), tail(Some(Spring { strength: 0.0, damping: 0.2 })), {
        let mut l = tail(Some(BOUNCY));
        let mut arm = (**l.armature.as_ref().unwrap()).clone();
        arm.springs_off = true;
        l.armature = Some(Arc::new(arm));
        l
    }];
    for l in &cases {
        for f in 0..60 {
            assert_eq!(bits(&sprung_pose(l, f, 24.0).unwrap()), bits(&sprung_pose(&keyed, f, 24.0).unwrap()), "frame {f}");
            let (a, b) = (crate::eval::evaluate_shown(l, f, None, 24.0).unwrap().1, crate::eval::evaluate_guided(&keyed, f, None).unwrap().1);
            assert!(a.iter().zip(b.iter()).all(|(x, y)| x.transform == y.transform));
        }
    }
}

#[test]
fn heavier_damping_settles_sooner_and_a_stiffer_spring_lags_less() {
    let swing = |s: Spring| (10..80).map(|f| sprung_pose(&tail(Some(s)), f, 24.0).unwrap().angle(B1).abs()).fold(0.0f32, f32::max);
    let after = |s: Spring, f: u32| sprung_pose(&tail(Some(s)), f, 24.0).unwrap().angle(B1).abs();
    assert!(after(Spring { damping: 0.6, ..BOUNCY }, 40) < after(BOUNCY, 40) + 1e-6);
    let lag = |s: Spring| sprung_pose(&tail(Some(s)), 5, 24.0).unwrap().angle(B1).abs();
    assert!(lag(Spring { strength: 0.9, ..BOUNCY }) < lag(BOUNCY));
    assert!(swing(BOUNCY) <= MAX_SWING);
}

#[test]
fn the_same_motion_in_seconds_swings_alike_at_any_frame_rate() {
    // At 48 fps the drop takes 20 frames: the same second-based motion.
    let mut l48 = tail(Some(BOUNCY));
    Arc::make_mut(&mut l48.keyframes[0]).duration = 20;
    Arc::make_mut(&mut l48.keyframes[1]).start = 20;
    let (a, b) = (sprung_pose(&tail(Some(BOUNCY)), 20, 24.0).unwrap().angle(B1), sprung_pose(&l48, 40, 48.0).unwrap().angle(B1));
    assert!((a - b).abs() < 0.25 * a.abs().max(b.abs()), "{a} vs {b}");
}

#[test]
fn joint_limits_stop_a_spring() {
    let mut l = tail(Some(Spring { strength: 0.2, damping: 0.0 }));
    let mut arm = (**l.armature.as_ref().unwrap()).clone();
    let lim = Limits { min: -0.05, max: 0.05 };
    arm.bone_mut(B1).unwrap().limits = Some(lim);
    l.armature = Some(Arc::new(arm));
    for f in 0..120 {
        let a = sprung_pose(&l, f, 24.0).unwrap().angle(B1);
        assert!((lim.min..=lim.max).contains(&a), "frame {f}: {a}");
    }
}

#[test]
fn hostile_springs_and_frame_rates_stay_bounded() {
    for s in [Spring { strength: 1.0, damping: 0.0 }, Spring { strength: 1e-9, damping: 0.0 }, Spring { strength: 0.5, damping: 0.999_999 }] {
        let l = tail(Some(s));
        for fps in [f32::NAN, 0.0, -5.0, 1.0, 1e9, 240.0] {
            for f in [0, 3, 9, 10, 11, 50, 299, 300, 100_000] {
                if let Some(p) = sprung_pose(&l, f, fps) {
                    assert!(p.0.iter().all(|j| j.angle.is_finite() && j.angle.abs() <= MAX_SWING + 1e-3), "{s:?} at {fps} fps, frame {f}: {p:?}");
                }
            }
        }
    }
    // Loading clamps what a file says.
    assert_eq!(Spring { strength: 7.0, damping: -3.0 }.sanitized(), Some(Spring { strength: 1.0, damping: 0.0 }));
    assert_eq!(Spring { strength: f32::NAN, damping: 0.5 }.sanitized(), None);
    assert!(omega(f32::NAN).is_finite() || omega(f32::NAN).is_nan());
    assert_eq!(zeta(1.0), f32::INFINITY);
}

#[test]
fn every_export_sees_the_spring() {
    let mut d = Document::default();
    let mut seg = Layer::new(LayerId(2), "seg");
    let mut k = Keyframe::new(0, 1);
    k.elements.push(Arc::new(Element { id: ElementId(3), name: String::new(), transform: Affine::IDENTITY, color: ColorEffect::default(), kind: ElementKind::Shape(Shape::filled(Path::rect(0.0, -5.0, 100.0, 10.0), Paint::Solid(Rgba::BLACK))) }));
    seg.keyframes = vec![Arc::new(k)];
    d.library.push(Arc::new(Symbol { id: SEG, name: "Seg".into(), kind: SymbolKind::Graphic, linkage: None, scale_grid_kept: None, scale_grid: None, timeline: Timeline { layers: vec![Arc::new(seg)] } }));
    d.timeline_mut(0).layers = vec![Arc::new(tail(Some(BOUNCY)))];
    d.sanitize();
    let hand = |d: &Document, f: u32| evaluate(d, f).items.iter().find(|i| i.element == HAND).map(|i| i.transform.translation()).unwrap();
    let mut still = d.clone();
    Arc::make_mut(Arc::make_mut(&mut still.timeline_mut(0).layers[0]).armature.as_mut().unwrap()).springs_off = true;
    assert!(hand(&d, 14).distance(hand(&still, 14)) > 2.0, "the display list draws the swing");
    assert_eq!(hand(&d, 200), hand(&still, 200), "and the settled pose exactly");
    // The file keeps the springs; older files (without them) read as before.
    let json = serde_json::to_string(&d).unwrap();
    assert!(json.contains("\"spring\""));
    let back: Document = serde_json::from_str(&json).unwrap();
    assert_eq!(back, d);
    assert!(!serde_json::to_string(&still.timeline(0).layers[0].keyframes[0]).unwrap().contains("\"x\""), "poses without slides are written as before");
}
/// The cost of one frame with springs: the whole window is followed again for every frame.
#[test]
fn springs_are_fast_enough_to_play() {
    // 16 sprung bones in a chain on a moving root.
    let mut bones = Vec::new();
    let mut a = Keyframe::new(0, 10);
    let mut b = Keyframe::new(10, 290);
    a.elements.push(instance(ElementId(1000), 0.0, 0.0));
    b.elements.push(instance(ElementId(1000), 0.0, 300.0));
    for i in 0..16u64 {
        let (p, c) = (ElementId(1000 + i), ElementId(1001 + i));
        bones.push(Bone { spring: Some(BOUNCY), ..bone(BoneId(5000 + i), p, c) });
        a.elements.push(instance(c, 100.0 * (i + 1) as f32, 0.0));
        b.elements.push(instance(c, 100.0 * (i + 1) as f32, 300.0));
    }
    a.tween = Some(Tween::default());
    let mut l = Layer::new(LayerId(1), "Armature_1");
    l.kind = LayerKind::Armature;
    l.armature = Some(Arc::new(crate::armature::Armature { bones, ..Default::default() }));
    l.keyframes = vec![Arc::new(a), Arc::new(b)];
    let t = std::time::Instant::now();
    for f in 150..160 {
        std::hint::black_box(sprung_pose(&l, f, 24.0));
    }
    let per = t.elapsed() / 10;
    println!("16 sprung bones, 24 fps ({} frames followed): {per:?} per frame", (24.0 * SPRING_SECONDS) as u32);
    assert!(per < std::time::Duration::from_millis(250), "{per:?}");
}
