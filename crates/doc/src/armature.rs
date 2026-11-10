//! Armatures (ADR 0043): bones that join symbol instances on an armature layer, poses that
//! turn them, and inverse kinematics that finds a pose from a dragged bone end.
//!
//! An armature is a tree of [`Bone`]s on a [`LayerKind::Armature`](crate::LayerKind) layer.
//! Each bone starts at a **joint** on its *parent* instance (its head) and ends on its *child*
//! instance (its tail); the child turns about the joint, carrying everything below it. An
//! instance is the child of at most one bone, and the instance no bone ends in is the root: it
//! keeps its own transform, so moving it moves the whole armature.
//!
//! A **pose** is a joint angle per bone ([`Pose`]), stored on each keyframe of the armature
//! layer and tweened between keyframes with the keyframe's easing. Angles are relative: 0 is the
//! bone as it was drawn, measured against its parent, so turning a parent carries its children.
//! Forward kinematics ([`place`]) turns a pose into the instances' transforms; it is pure, and
//! [`crate::evaluate_layer_elements`] applies it, so every renderer, export and the player see
//! the posed instances.
//!
//! Inverse kinematics ([`solve`]) is cyclic coordinate descent (CCD): sweeping from the dragged
//! bone up its chain, each joint turns so the effector points at the target, clamped to the
//! joint's limits. It works on joint angles directly, so limits are a clamp per step and the
//! result is a pose by construction.

use crate::{Element, ElementId, Keyframe, Layer};
use anicraft_geom::{Affine, Vec2};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Most bones in one armature.
pub const MAX_BONES: usize = 256;
/// Longest bone name kept (bytes).
pub const MAX_NAME: usize = 256;
/// Joint limits stay within a turn either way (radians).
pub const MAX_LIMIT: f32 = std::f32::consts::TAU;
/// Largest joint angle kept (radians): many turns, never infinity.
pub const MAX_ANGLE: f32 = 64.0 * std::f32::consts::TAU;
/// IK stops once the effector is this close to its target (layer units, pixels).
pub const IK_TOLERANCE: f32 = 0.01;
/// Most CCD sweeps one solve takes.
pub const IK_SWEEPS: usize = 2048;
/// How often a stalled solve bends its joints to get going again, and by how much (radians).
const IK_KICKS: usize = 4;
const IK_KICK: f32 = 0.3;

/// Unique id of a bone within a document (from the document's id counter).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BoneId(pub u64);

/// How far a joint may turn either way from where the bone was drawn (radians, `min ≤ 0 ≤ max`
/// is usual but not required).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Limits {
    pub min: f32,
    pub max: f32,
}

impl Limits {
    /// `a` kept inside the limits.
    pub fn clamp(&self, a: f32) -> f32 {
        a.clamp(self.min, self.max)
    }

    /// Finite, ordered and within [`MAX_LIMIT`]; `None` when nothing sane is left.
    pub fn sanitized(self) -> Option<Limits> {
        if !(self.min.is_finite() && self.max.is_finite()) {
            return None;
        }
        let (a, b) = (self.min.clamp(-MAX_LIMIT, MAX_LIMIT), self.max.clamp(-MAX_LIMIT, MAX_LIMIT));
        Some(Limits { min: a.min(b), max: a.max(b) })
    }
}

/// A bone: from a joint on its parent instance to its child instance, which it turns.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bone {
    pub id: BoneId,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// The instance the bone starts on (the joint is a point on it).
    pub parent: ElementId,
    /// The instance the bone moves.
    pub child: ElementId,
    /// The joint, in the parent instance's local space: the child turns about it.
    pub head: Vec2,
    /// The bone's end, in the child instance's local space (where a drag pulls it).
    pub tail: Vec2,
    /// The child's transform in the parent's local space at joint angle 0 (as it was drawn).
    pub rest: Affine,
    /// How far the joint may turn; `None`: freely.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<Limits>,
    /// Pinned: inverse kinematics never turns it (nor the bones it hangs from), so its end stays
    /// where it is while the rest of the armature follows a drag.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pinned: bool,
}

impl Bone {
    /// `a` as this joint allows it (finite, within its limits).
    pub fn allow(&self, a: f32) -> f32 {
        let a = if a.is_finite() { a.clamp(-MAX_ANGLE, MAX_ANGLE) } else { 0.0 };
        self.limits.map_or(a, |l| l.clamp(a))
    }

    fn is_sane(&self) -> bool {
        self.parent != self.child && self.head.x.is_finite() && self.head.y.is_finite() && self.tail.x.is_finite() && self.tail.y.is_finite() && self.rest.is_finite() && self.rest.inverse().is_some()
    }
}

/// The bones of an armature layer, each listed after the bone it hangs from.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Armature {
    #[serde(default)]
    pub bones: Vec<Bone>,
}

impl Armature {
    pub fn is_empty(&self) -> bool {
        self.bones.is_empty()
    }

    pub fn bone(&self, id: BoneId) -> Option<&Bone> {
        self.bones.iter().find(|b| b.id == id)
    }

    pub fn bone_mut(&mut self, id: BoneId) -> Option<&mut Bone> {
        self.bones.iter_mut().find(|b| b.id == id)
    }

    /// The bone that moves instance `el` (ends in it), if any.
    pub fn bone_into(&self, el: ElementId) -> Option<&Bone> {
        self.bones.iter().find(|b| b.child == el)
    }

    /// The bone `b` hangs from: the one that moves its parent instance.
    pub fn parent_bone(&self, b: &Bone) -> Option<&Bone> {
        self.bone_into(b.parent)
    }

    /// Whether instance `el` is part of the armature.
    pub fn contains(&self, el: ElementId) -> bool {
        self.bones.iter().any(|b| b.parent == el || b.child == el)
    }

    /// Every instance in the armature, roots first.
    pub fn members(&self) -> Vec<ElementId> {
        let mut out: Vec<ElementId> = self.roots();
        for b in &self.bones {
            if !out.contains(&b.child) {
                out.push(b.child);
            }
        }
        out
    }

    /// The instances no bone moves (one for an armature made with the Bone tool).
    pub fn roots(&self) -> Vec<ElementId> {
        let mut out = Vec::new();
        for b in &self.bones {
            if self.bone_into(b.parent).is_none() && !out.contains(&b.parent) {
                out.push(b.parent);
            }
        }
        out
    }

    /// Bone `id` and every bone below it, in armature order.
    pub fn subtree(&self, id: BoneId) -> Vec<BoneId> {
        let mut inside: HashSet<ElementId> = HashSet::new();
        let mut out = Vec::new();
        for b in &self.bones {
            if b.id == id || inside.contains(&b.parent) {
                inside.insert(b.child);
                out.push(b.id);
            }
        }
        out
    }

    /// The chain from bone `id` up to its root, `id` first (bounded by the bone count).
    pub fn chain(&self, id: BoneId) -> Vec<&Bone> {
        let mut out = Vec::new();
        let mut cur = self.bone(id);
        while let Some(b) = cur {
            if out.len() >= self.bones.len() || out.iter().any(|o: &&Bone| o.id == b.id) {
                break;
            }
            out.push(b);
            cur = self.parent_bone(b);
        }
        out
    }

    /// Repairs a loaded armature: drops bones with non-finite numbers, repeated ids, a second
    /// bone into one instance and bones in cycles; keeps every bone after its parent; keeps at
    /// most [`MAX_BONES`]. Returns whether anything changed.
    pub fn sanitize(&mut self) -> bool {
        let before = self.bones.clone();
        let mut seen_id = HashSet::new();
        let mut seen_child = HashSet::new();
        let mut kept: Vec<Bone> = Vec::new();
        for mut b in std::mem::take(&mut self.bones) {
            if !b.is_sane() || !seen_id.insert(b.id) || !seen_child.insert(b.child) {
                continue;
            }
            b.limits = b.limits.and_then(Limits::sanitized);
            if b.name.len() > MAX_NAME {
                let cut = (0..=MAX_NAME).rev().find(|&i| b.name.is_char_boundary(i)).unwrap_or(0);
                b.name.truncate(cut);
            }
            kept.push(b);
        }
        // Order: a bone after the bone moving its parent. What never gets placed is a cycle.
        let children: HashSet<ElementId> = kept.iter().map(|b| b.child).collect();
        let mut placed: HashSet<ElementId> = kept.iter().map(|b| b.parent).filter(|p| !children.contains(p)).collect();
        let mut ordered = Vec::with_capacity(kept.len());
        let mut rest = kept;
        loop {
            let (ready, wait): (Vec<Bone>, Vec<Bone>) = rest.into_iter().partition(|b| placed.contains(&b.parent));
            if ready.is_empty() {
                break;
            }
            for b in ready {
                placed.insert(b.child);
                ordered.push(b);
            }
            rest = wait;
        }
        ordered.truncate(MAX_BONES);
        self.bones = ordered;
        self.bones != before
    }
}

/// One joint's angle in a pose (radians, 0 as drawn).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct JointAngle {
    pub bone: BoneId,
    pub angle: f32,
}

/// A pose: joint angles by bone. A bone it doesn't list is as drawn (angle 0).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Pose(pub Vec<JointAngle>);

impl Pose {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The angle of bone `b` (0 when the pose doesn't list it).
    pub fn angle(&self, b: BoneId) -> f32 {
        self.0.iter().find(|j| j.bone == b).map_or(0.0, |j| j.angle)
    }

    pub fn set(&mut self, b: BoneId, angle: f32) {
        match self.0.iter_mut().find(|j| j.bone == b) {
            Some(j) => j.angle = angle,
            None => self.0.push(JointAngle { bone: b, angle }),
        }
    }

    /// Keeps only the bones in `keep`.
    pub fn retain(&mut self, keep: impl Fn(BoneId) -> bool) {
        self.0.retain(|j| keep(j.bone));
    }

    /// Every bone of `arm` at the angle its joint allows: the pose as it shows.
    pub fn allowed(&self, arm: &Armature) -> Pose {
        Pose(arm.bones.iter().map(|b| JointAngle { bone: b.id, angle: b.allow(self.angle(b.id)) }).collect())
    }

    /// The pose `t` of the way to `o`, joint by joint (angles blend linearly, so a joint that
    /// turned a whole way round between keys turns that way in the tween).
    pub fn lerp(&self, o: &Pose, t: f32, arm: &Armature) -> Pose {
        Pose(arm.bones.iter().map(|b| JointAngle { bone: b.id, angle: b.allow(anicraft_geom::lerp(self.angle(b.id), o.angle(b.id), t)) }).collect())
    }

    /// Repairs a loaded pose: finite angles in range, one per bone, at most [`MAX_BONES`].
    /// Returns whether anything changed.
    pub fn sanitize(&mut self) -> bool {
        let before = self.0.len();
        let mut changed = false;
        let mut seen = HashSet::new();
        self.0.retain(|j| seen.insert(j.bone));
        self.0.truncate(MAX_BONES);
        for j in &mut self.0 {
            let a = if j.angle.is_finite() { j.angle.clamp(-MAX_ANGLE, MAX_ANGLE) } else { 0.0 };
            if a.to_bits() != j.angle.to_bits() {
                j.angle = a;
                changed = true;
            }
        }
        changed || self.0.len() != before
    }
}

/// `R(angle)` about point `j`.
fn turn_about(j: Vec2, angle: f32) -> Affine {
    Affine::translate(j.x, j.y).mul(&Affine::rotate(angle)).mul(&Affine::translate(-j.x, -j.y))
}

/// Forward kinematics: the transform (in the layer's space) of every instance in `elements`
/// once `pose` turns the armature's joints. A root keeps its own transform; a child is placed
/// from its parent: `R_joint(angle) · parent · rest`. An instance whose parent isn't on the
/// keyframe keeps its own transform.
pub fn place(arm: &Armature, elements: &[Arc<Element>], pose: &Pose) -> HashMap<ElementId, Affine> {
    let mut world: HashMap<ElementId, Affine> = elements.iter().map(|e| (e.id, e.transform)).collect();
    for b in &arm.bones {
        let Some(wp) = world.get(&b.parent).copied() else { continue };
        if !world.contains_key(&b.child) {
            continue;
        }
        let j = wp.apply(b.head);
        let wc = turn_about(j, b.allow(pose.angle(b.id))).mul(&wp.mul(&b.rest));
        if wc.is_finite() {
            world.insert(b.child, wc);
        }
    }
    world
}

/// Puts the instances of `elements` where `pose` has them (copy-on-write: only instances that
/// move are cloned). Returns whether any moved.
pub fn apply(arm: &Armature, elements: &mut [Arc<Element>], pose: &Pose) -> bool {
    if arm.is_empty() {
        return false;
    }
    let world = place(arm, elements, pose);
    let mut moved = false;
    for e in elements.iter_mut() {
        if arm.bone_into(e.id).is_none() {
            continue;
        }
        if let Some(m) = world.get(&e.id)
            && *m != e.transform
        {
            Arc::make_mut(e).transform = *m;
            moved = true;
        }
    }
    moved
}

/// The pose layer `layer` has at `frame`: its covering keyframe's, or, inside a tween, the pose
/// between that keyframe's and the next one's at the tween's eased time. Exact at keyframes.
/// `None` without an armature or on an empty frame.
pub fn pose_at(layer: &Layer, frame: u32) -> Option<Pose> {
    let arm = layer.armature.as_deref().filter(|_| layer.kind == crate::LayerKind::Armature)?;
    let idx = layer.keyframe_index_at(frame)?;
    let kf: &Keyframe = layer.keyframes.get(idx)?;
    let here = kf.pose.allowed(arm);
    let (Some(tween), None) = (kf.tween, &kf.motion) else { return Some(here) };
    let Some(next) = layer.keyframes.get(idx + 1) else { return Some(here) };
    if next.start != kf.end() || frame == kf.start {
        return Some(here);
    }
    let t = tween.easing.apply((frame - kf.start) as f32 / kf.duration.max(1) as f32);
    Some(kf.pose.lerp(&next.pose, t, arm))
}

/// Where a bone is drawn: its joint (head) and end (tail), in the layer's space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoneAt {
    pub id: BoneId,
    pub head: Vec2,
    pub tail: Vec2,
    /// The joint angle (radians, as the joint allows it).
    pub angle: f32,
}

/// Every bone's joint and end with the instances in `elements` placed as they show (posed:
/// what [`crate::evaluate_layer_elements`] gives on an armature layer).
pub fn bones_at(arm: &Armature, elements: &[Arc<Element>], pose: &Pose) -> Vec<BoneAt> {
    let world: HashMap<ElementId, Affine> = elements.iter().map(|e| (e.id, e.transform)).collect();
    arm.bones
        .iter()
        .filter_map(|b| {
            let (wp, wc) = (world.get(&b.parent)?, world.get(&b.child)?);
            Some(BoneAt { id: b.id, head: wp.apply(b.head), tail: wc.apply(b.tail), angle: b.allow(pose.angle(b.id)) })
        })
        .collect()
}

/// What a solve came to.
#[derive(Clone, Debug, PartialEq)]
pub struct Solved {
    /// The new pose (every bone of the armature).
    pub pose: Pose,
    /// How far the effector ended from the target.
    pub error: f32,
    /// How many bones the drag could turn (0: the bone is pinned, or hangs from a pinned one).
    pub chain: usize,
}

/// The bones a drag of bone `id` may turn: `id` and the bones it hangs from, up to the first
/// that must stay put: a pinned bone, or one a pinned bone hangs from (turning it would move
/// the pinned bone).
pub fn movable_chain(arm: &Armature, id: BoneId) -> Vec<&Bone> {
    let frozen: HashSet<BoneId> = arm.bones.iter().filter(|b| b.pinned).flat_map(|p| arm.chain(p.id)).map(|b| b.id).collect();
    arm.chain(id).into_iter().take_while(|b| !frozen.contains(&b.id)).collect()
}

/// Inverse kinematics: the pose that brings point `local` of bone `id`'s child instance (its
/// local space; the bone's tail when dragging the bone end) to `target` (layer space), turning
/// only the bones [`movable_chain`] allows. `elements` are the keyframe's instances (the roots
/// at their own transforms), `pose` the pose to start from. Cyclic coordinate descent: each
/// sweep turns the joints from the dragged bone upward so the effector points at the target,
/// each clamped to its limits, until the effector is within [`IK_TOLERANCE`] or stops moving.
/// An unreachable target leaves the chain stretched straight toward it, the root where it was.
/// `None` when the armature has no bone `id` or its instances aren't on the keyframe.
pub fn solve(arm: &Armature, elements: &[Arc<Element>], pose: &Pose, id: BoneId, local: Vec2, target: Vec2) -> Option<Solved> {
    let bone = arm.bone(id)?;
    if !(local.x.is_finite() && local.y.is_finite() && target.x.is_finite() && target.y.is_finite()) {
        return None;
    }
    let chain: Vec<(BoneId, ElementId, Vec2)> = movable_chain(arm, id).iter().map(|b| (b.id, b.parent, b.head)).collect();
    let mut pose = pose.allowed(arm);
    let effector = |pose: &Pose| -> Option<(HashMap<ElementId, Affine>, Vec2)> {
        let world = place(arm, elements, pose);
        let e = world.get(&bone.child)?.apply(local);
        Some((world, e))
    };
    let (_, start) = effector(&pose)?;
    let mut error = start.distance(target);
    let mut best = (pose.clone(), error);
    let mut kicks = 0;
    for _ in 0..IK_SWEEPS {
        if error < best.1 {
            best = (pose.clone(), error);
        }
        if error <= IK_TOLERANCE || chain.is_empty() {
            break;
        }
        let Some((world, mut e)) = effector(&pose) else { break };
        let mut turned = 0.0f32;
        // From the dragged bone up: turning a joint moves only what is below it, so the joints
        // above keep the places this sweep began with.
        for &(b, parent, head) in &chain {
            let Some(wp) = world.get(&parent) else { continue };
            let Some(joint) = arm.bone(b) else { continue };
            let j = wp.apply(head);
            let (u, v) = (e - j, target - j);
            if u.length() < 1e-6 || v.length() < 1e-6 {
                continue;
            }
            let want = u.cross(v).atan2(u.dot(v));
            let cur = pose.angle(b);
            let next = joint.allow(cur + want);
            let by = next - cur;
            if by != 0.0 {
                pose.set(b, next);
                e = turn_about(j, by).apply(e);
                turned = turned.max(by.abs());
            }
        }
        let Some((_, e)) = effector(&pose) else { break };
        let was = error;
        error = e.distance(target);
        // Stuck: limits, a chain stretched straight at an unreachable target, or CCD's blind
        // spot, a straight chain whose target lies on its own line (every joint sees angle 0).
        // A few kicks bend the joints a little and descent goes on; the best pose seen wins.
        if turned < 1e-7 || ((was - error).abs() < 1e-6 * was.max(1.0) && turned < 1e-4) {
            if error < best.1 {
                best = (pose.clone(), error);
            }
            if kicks >= IK_KICKS {
                break;
            }
            kicks += 1;
            for (i, &(b, _, _)) in chain.iter().enumerate() {
                let Some(joint) = arm.bone(b) else { continue };
                let cur = pose.angle(b);
                let kick = IK_KICK * if (i + kicks) % 2 == 0 { 1.0 } else { -1.0 };
                let next = joint.allow(cur + kick);
                pose.set(b, if next != cur { next } else { joint.allow(cur - kick) });
            }
            error = effector(&pose).map_or(error, |(_, e)| e.distance(target));
            continue;
        }
    }
    if error < best.1 {
        best = (pose, error);
    }
    Some(Solved { pose: best.0, error: best.1, chain: chain.len() })
}

#[cfg(test)]
#[path = "armature_tests.rs"]
mod tests;
