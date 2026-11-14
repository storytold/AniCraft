//! Bone springs (ADR 0084): secondary motion as a pure function of time.
//!
//! A bone with a [`Spring`](crate::armature::Spring) lags behind the pose it is keyed to and
//! swings past it when the bones it hangs from move (or its own keyed angle changes), then
//! settles back. Each sprung bone carries an angle `φ` off its keyed angle, driven by how the
//! keyed motion *accelerates* (the joint's linear acceleration across the bone and the keyed
//! direction's angular acceleration: the inertia of a pendulum on a moving pivot) and pulled
//! back by a damped spring:
//!
//! ```text
//! φ'' = −θ_key'' − (a_joint · n) / L − ω² φ − 2ζω φ'
//! ```
//!
//! where `θ_key` is the bone's world direction as keyed (relative to its parent as it swings, so
//! springs chain), `n` the bone's normal and `L` its length. Strength sets the natural frequency
//! `ω` (half a swing to ten swings a second), damping the ratio `ζ`.
//!
//! **No simulation state is stored.** [`sprung_pose`] re-runs the motion from a fixed start for
//! every frame it is asked about: at most [`SPRING_SECONDS`] back (and never before the run of
//! keyframes the frame is on), assuming the armature was still before that. Every step is a
//! fixed function of the frames before it, so seeking to a frame gives exactly the bits playing
//! to it does, and every export, the player and the stage agree. Steps are bounded (at most
//! [`MAX_SPRING_FRAMES`] frames of [`SUBSTEPS`] each), and the integrator is backward Euler, which
//! is stable for any strength, damping and frame rate. Damping 1, strength 0 or springs turned
//! off on the armature give the keyed pose bit for bit.

use crate::armature::{self, Armature, Pose};
use crate::{ElementId, Layer};
use anicraft_geom::{Affine, Vec2};
use std::collections::HashMap;
use std::f32::consts::{PI, TAU};

/// How far back a spring's motion is followed (seconds). Motion in the older half of that
/// window counts less and less ([`taper`]), so every swing has died away this long after the
/// motion that caused it, smoothly, with no jump when it drops out of the window.
pub const SPRING_SECONDS: f32 = 6.0;
/// Most frames one spring evaluation follows (bounds the work at high frame rates).
pub const MAX_SPRING_FRAMES: u32 = 720;
/// Integration steps per frame.
pub const SUBSTEPS: u32 = 4;
/// Farthest a spring swings a bone off its keyed angle (radians, half a turn either way).
pub const MAX_SWING: f32 = PI;

/// Natural frequency (radians a second) of strength `s` (0…1): 0.5 Hz up to 10 Hz, evenly
/// spread in pitch.
pub fn omega(strength: f32) -> f32 {
    TAU * 0.5 * 20f32.powf(strength.clamp(0.0, 1.0))
}

/// Damping ratio of damping `d` (0…1): 0 swings on, ⅔ is critical (no overshoot), 1 is
/// infinite (the swing never starts).
pub fn zeta(damping: f32) -> f32 {
    let d = damping.clamp(0.0, 1.0);
    if d >= 1.0 { f32::INFINITY } else { 0.5 * d / (1.0 - d) }
}

/// How much motion `age` frames old still drives a spring, in a window of `window` frames: all of
/// it in the newer half, then easing smoothly to none at the window's start.
pub fn taper(age: u32, window: u32) -> f32 {
    let x = age as f32 / window.max(1) as f32;
    if x <= 0.5 {
        return 1.0;
    }
    let s = ((x - 0.5) * 2.0).clamp(0.0, 1.0);
    1.0 - s * s * (3.0 - 2.0 * s)
}

/// The frame rate springs run at, from the stage's (sane, 1–240 fps).
fn sane_fps(fps: f32) -> f32 {
    if fps.is_finite() && fps > 0.0 { fps.clamp(1.0, 240.0) } else { 24.0 }
}

/// The first frame of the unbroken run of non-empty keyframes `frame` is on.
fn run_start(layer: &Layer, frame: u32) -> Option<u32> {
    let mut i = layer.keyframe_index_at(frame)?;
    while i > 0 {
        let (Some(prev), Some(cur)) = (layer.keyframes.get(i - 1), layer.keyframes.get(i)) else { break };
        if prev.end() != cur.start || prev.elements.is_empty() {
            break;
        }
        i -= 1;
    }
    layer.keyframes.get(i).map(|k| k.start)
}

/// Wraps an angle difference into (−π, π].
fn wrap(a: f32) -> f32 {
    let w = (a + PI).rem_euclid(TAU) - PI;
    if w <= -PI { w + TAU } else { w }
}

/// What a sprung bone remembers between steps of one evaluation (never stored anywhere).
#[derive(Clone, Copy, Default)]
struct Swing {
    phi: f32,
    vel: f32,
    /// The joint and keyed direction one and two frames back.
    hist: Option<[(Vec2, f32); 2]>,
}

/// The pose layer `layer` shows at `frame` at `fps` frames a second: its keyed pose
/// ([`armature::pose_at`]) with each sprung bone swung by its spring. Exactly the keyed pose
/// when no spring moves anything. `None` where [`armature::pose_at`] is.
pub fn sprung_pose(layer: &Layer, frame: u32, fps: f32) -> Option<Pose> {
    let keyed = armature::pose_at(layer, frame)?;
    let Some(arm) = layer.armature.as_deref().filter(|a| a.has_springs()) else { return Some(keyed) };
    let fps = sane_fps(fps);
    let window = ((fps * SPRING_SECONDS).ceil() as u32).min(MAX_SPRING_FRAMES);
    let run = run_start(layer, frame).unwrap_or(frame);
    let first = run.max(frame.saturating_sub(window));
    // Two frames before the window (where there are any) only give the motion's speed, so a
    // window that starts mid-move doesn't kick the springs as if the move began there.
    let warm = run.max(first.saturating_sub(2));
    let mut swings = vec![Swing::default(); arm.bones.len()];
    for f in warm..=frame {
        let Some(pose) = armature::pose_at(layer, f) else { continue };
        let Some((_, els)) = crate::eval::evaluate_tweened(layer, f, None) else { continue };
        let world: HashMap<ElementId, Affine> = els.iter().map(|e| (e.id, e.transform)).collect();
        let weight = if f >= first { taper(frame - f, window) } else { 0.0 };
        step(arm, world, &pose, &mut swings, 1.0 / fps, weight);
    }
    let mut out = keyed.allowed(arm);
    for (b, s) in arm.bones.iter().zip(&swings) {
        if s.phi != 0.0 && b.live_spring().is_some() {
            out.set(b.id, b.allow(keyed.angle(b.id) + s.phi));
        }
    }
    Some(out)
}

/// One frame: places the armature from the roots in `world` (the keyframe's own transforms)
/// by `pose`, each sprung bone swung by its spring, which is advanced `dt` seconds with the
/// frame's motion counting `weight` (0: the frame only records the motion).
fn step(arm: &Armature, mut world: HashMap<ElementId, Affine>, pose: &Pose, swings: &mut [Swing], dt: f32, weight: f32) {
    let nodes = arm.skin.is_some();
    for (b, sw) in arm.bones.iter().zip(swings.iter_mut()) {
        let Some(wp) = world.get(&b.parent).copied() else { continue };
        if !nodes && !world.contains_key(&b.child) {
            continue;
        }
        let (key, slide) = (b.allow(pose.angle(b.id)), b.allow_slide(pose.slide(b.id)));
        let (j, keyed) = armature::joint(b, &wp, key, slide);
        let mut angle = key;
        if let Some(sp) = b.live_spring() {
            let tip = keyed.apply(b.tail) - j;
            let len = tip.length();
            let theta = tip.y.atan2(tip.x);
            if let Some([(j1, t1), (j2, t2)]) = sw.hist.filter(|_| weight > 0.0) {
                // The keyed motion's accelerations over the last frame, as finite differences.
                let alpha = (wrap(theta - t1) - wrap(t1 - t2)) / (dt * dt);
                let acc = (j - j1 - (j1 - j2)) * (1.0 / (dt * dt));
                let (w, z) = (omega(sp.strength), zeta(sp.damping));
                let h = dt / SUBSTEPS as f32;
                for _ in 0..SUBSTEPS {
                    let dir = Vec2::new((theta + sw.phi).cos(), (theta + sw.phi).sin());
                    let lin = if len > 1e-3 { acc.dot(dir.perp()) / len } else { 0.0 };
                    let drive = -(alpha + lin) * weight;
                    // Backward Euler: stable for any stiffness and damping.
                    sw.vel = (sw.vel + h * (drive - w * w * sw.phi)) / (1.0 + h * 2.0 * z * w + h * h * w * w);
                    sw.phi += h * sw.vel;
                }
                if !(sw.phi.is_finite() && sw.vel.is_finite()) {
                    (sw.phi, sw.vel) = (0.0, 0.0);
                }
                if sw.phi.abs() > MAX_SWING {
                    (sw.phi, sw.vel) = (sw.phi.clamp(-MAX_SWING, MAX_SWING), 0.0);
                }
                // A joint limit stops the swing.
                let allowed = b.allow(key + sw.phi);
                if allowed != key + sw.phi {
                    (sw.phi, sw.vel) = (allowed - key, 0.0);
                }
            }
            sw.hist = Some([(j, theta), sw.hist.map_or((j, theta), |h| h[0])]);
            angle = b.allow(key + sw.phi);
        }
        let wc = if angle == key { keyed } else { armature::joint(b, &wp, angle, slide).1 };
        if wc.is_finite() {
            world.insert(b.child, wc);
        }
    }
}

#[cfg(test)]
#[path = "spring_tests.rs"]
mod tests;
