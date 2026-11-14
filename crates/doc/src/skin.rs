//! Shape armatures (ADR 0071): bones drawn inside one shape, whose outline bends with them.
//!
//! A shape armature is an [`Armature`] with a [`Skin`]. Its bones join *joint nodes* instead of
//! symbol instances: the root bones start on the shape itself (its element id) and every other
//! bone starts on the node of the bone it hangs from. A node is a frame that the bone into it
//! turns; at rest every node's frame is the shape's own local space, so a bone's head and tail
//! are points of the shape as it was drawn, and a node's transform under a pose is the bone's
//! **skinning matrix**. Forward and inverse kinematics are ADR 0043's ([`crate::armature::place`],
//! [`crate::armature::solve`]): nothing about posing is new.
//!
//! The skin keeps the outline as it was drawn (`rest`) and one [`Binding`] per anchor of it
//! (every `MoveTo`, `LineTo`, `QuadTo` and `CubicTo` end, in path order): the bones the anchor
//! follows and how much. Automatic binding ([`auto_binding`]) weighs each bone by the inverse
//! fourth power of the anchor's distance to the bone's segment, keeps the strongest
//! [`MAX_INFLUENCES`] and normalises them to sum to 1; the Bind tool (`bone.bind`) overrides
//! anchors by hand. The posed outline ([`deform`]) is linear blend skinning: each anchor moves
//! by the weighted sum of its bones' matrices, and each handle by the blend of the anchor it
//! belongs to, so curves stay smooth through a bend. Stroke width is drawn after the outline
//! (the shape's own transform is untouched), so strokes keep their width.
//!
//! Linear blend skinning is the standard method (N. Magnenat-Thalmann, R. Laperrière and
//! D. Thalmann, *Joint-dependent local deformations for hand animation and object grasping*,
//! Graphics Interface 1988; J. P. Lewis, M. Cordner and N. Fong, *Pose space deformation*,
//! SIGGRAPH 2000, §2, which names it skeleton subspace deformation).

use crate::armature::{Armature, BoneId, Pose};
use crate::{Element, ElementId, ElementKind};
use anicraft_geom::{Affine, Path, PathCmd, Vec2, segment_distance};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Most bones one anchor follows.
pub const MAX_INFLUENCES: usize = 4;
/// Most anchors a shape with bones may have (a hand-drawn shape has tens to a few thousand).
pub const MAX_SKIN_POINTS: usize = 20_000;
/// Distances below this count as this (px), so an anchor on a bone has a finite weight.
const NEAR: f32 = 0.5;
/// A bone whose share is below this after normalising is dropped from the anchor.
const FAINT: f32 = 0.01;

/// One bone's share of an anchor: `[bone, weight]` in files.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Influence(pub BoneId, pub f32);

/// The bones one anchor follows. Weights are positive and sum to 1; an anchor with none stays
/// where it was drawn.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Binding {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bones: Vec<Influence>,
    /// Set with the Bind tool: automatic binding (after adding or removing a bone) leaves it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub manual: bool,
}

impl Binding {
    /// The share bone `b` has of this anchor (0 when it doesn't follow it).
    pub fn weight(&self, b: BoneId) -> f32 {
        self.bones.iter().find(|i| i.0 == b).map_or(0.0, |i| i.1)
    }

    /// Keeps positive, finite weights of bones in `keep` (one each, the strongest
    /// [`MAX_INFLUENCES`]) and scales them to sum to 1. Returns whether anything changed.
    pub fn normalize(&mut self, keep: impl Fn(BoneId) -> bool) -> bool {
        let before = self.bones.clone();
        let mut seen = Vec::with_capacity(self.bones.len());
        self.bones.retain(|i| i.1.is_finite() && i.1 > 0.0 && keep(i.0) && !seen.contains(&i.0) && {
            seen.push(i.0);
            true
        });
        self.bones.sort_by(|a, b| b.1.total_cmp(&a.1));
        self.bones.truncate(MAX_INFLUENCES);
        let sum: f32 = self.bones.iter().map(|i| i.1).sum();
        if sum > 0.0 && sum.is_finite() {
            for i in &mut self.bones {
                i.1 /= sum;
            }
        } else {
            self.bones.clear();
        }
        self.bones != before
    }
}

/// How a shape is bound to its armature's bones.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Skin {
    /// The shape the bones are inside (an element of the armature layer's keyframes).
    pub shape: ElementId,
    /// The outline as it was drawn, in the shape's local space: the pose of every bone at 0.
    pub rest: Arc<Path>,
    /// One per anchor of `rest`, in path order.
    #[serde(default)]
    pub points: Vec<Binding>,
}

impl Skin {
    /// A skin for `shape` drawn as `rest`, its anchors not yet bound.
    pub fn new(shape: ElementId, rest: Arc<Path>) -> Skin {
        let points = vec![Binding::default(); anchors(&rest).len()];
        Skin { shape, rest, points }
    }
}

/// The anchors of `path` in order: every command's end point.
pub fn anchors(path: &Path) -> Vec<Vec2> {
    path.cmds
        .iter()
        .filter_map(|c| match *c {
            PathCmd::MoveTo(p) | PathCmd::LineTo(p) | PathCmd::QuadTo(_, p) | PathCmd::CubicTo(_, _, p) => Some(p),
            PathCmd::Close => None,
        })
        .collect()
}

/// Where every bone of a shape armature lies as drawn, in the shape's space: its joint and end.
pub fn rest_segments(arm: &Armature) -> Vec<(BoneId, Vec2, Vec2)> {
    // Every node's rest frame is the shape's space (`rest` is the identity for shape bones), so
    // composing the rests places each bone; a hand-edited file with other rests still works.
    let mut node: HashMap<ElementId, Affine> = HashMap::new();
    let root = arm.skin.as_ref().map(|s| s.shape);
    arm.bones
        .iter()
        .filter_map(|b| {
            let wp = if Some(b.parent) == root { Affine::IDENTITY } else { *node.get(&b.parent)? };
            let wc = wp.mul(&b.rest);
            node.insert(b.child, wc);
            Some((b.id, wp.apply(b.head), wc.apply(b.tail)))
        })
        .collect()
}

/// Automatic binding of an anchor at `p` (the shape's space, as drawn): each bone weighs
/// `1 / max(d, 0.5)⁴` where `d` is the distance from `p` to the bone's segment; the strongest
/// [`MAX_INFLUENCES`] are kept, faint ones (under 1 %) dropped, and the rest sum to 1. An anchor
/// near one bone follows it alone; one near a joint shares it between the bones that meet
/// there, so the outline bends smoothly. Empty only without bones.
pub fn auto_binding(segments: &[(BoneId, Vec2, Vec2)], p: Vec2) -> Binding {
    let mut bones: Vec<Influence> = segments
        .iter()
        .map(|&(id, a, b)| {
            let d = segment_distance(p, a, b).max(NEAR);
            Influence(id, 1.0 / (d * d * d * d))
        })
        .filter(|i| i.1.is_finite() && i.1 > 0.0)
        .collect();
    // Weighed relative to the nearest bone, so a far-away shape keeps finite numbers.
    let top = bones.iter().map(|i| i.1).fold(0.0f32, f32::max);
    if top > 0.0 {
        for i in &mut bones {
            i.1 /= top;
        }
    }
    let mut out = Binding { bones, manual: false };
    out.normalize(|_| true);
    let before = out.bones.len();
    out.bones.retain(|i| i.1 >= FAINT);
    if out.bones.len() != before {
        out.normalize(|_| true);
    }
    out
}

/// Binds every anchor not bound by hand afresh (after the bones changed), and repairs the hand
/// bound ones: bones that went away leave them, and one left with none goes back to automatic
/// (an anchor unbound by hand on purpose stays unbound). Keeps one binding per anchor.
pub fn rebind(arm: &mut Armature) {
    let segments = rest_segments(arm);
    let ids: Vec<BoneId> = arm.bones.iter().map(|b| b.id).collect();
    let Some(skin) = arm.skin.as_mut() else { return };
    let pts = anchors(&skin.rest);
    skin.points.resize(pts.len(), Binding::default());
    for (b, &p) in skin.points.iter_mut().zip(&pts) {
        if b.manual {
            let had = !b.bones.is_empty();
            b.normalize(|id| ids.contains(&id));
            if !had || !b.bones.is_empty() {
                continue;
            }
        }
        *b = auto_binding(&segments, p);
    }
}

/// Every bone's skinning matrix under `pose`: how the pose moves the shape's space near it (the
/// identity at rest, exactly).
pub fn matrices(arm: &Armature, pose: &Pose) -> HashMap<BoneId, Affine> {
    let Some(skin) = &arm.skin else { return HashMap::new() };
    let mut node: HashMap<ElementId, Affine> = HashMap::new();
    node.insert(skin.shape, Affine::IDENTITY);
    let mut out = HashMap::with_capacity(arm.bones.len());
    for b in &arm.bones {
        let Some(wp) = node.get(&b.parent).copied() else { continue };
        let (a, s) = (b.allow(pose.angle(b.id)), b.allow_slide(pose.slide(b.id)));
        let wc = if a == 0.0 && s == anicraft_geom::Vec2::ZERO { wp.mul(&b.rest) } else { crate::armature::joint(b, &wp, a, s).1 };
        if wc.is_finite() {
            node.insert(b.child, wc);
            out.insert(b.id, wc);
        }
    }
    out
}

/// The blend of `binding`'s bone matrices; `None` when it doesn't move (no bones, or every one
/// of them at rest), so a point at rest is kept bit for bit.
fn blend(binding: &Binding, m: &HashMap<BoneId, Affine>) -> Option<Affine> {
    let mut moved = false;
    let mut acc = [0.0f32; 6];
    let mut total = 0.0f32;
    for &Influence(b, w) in &binding.bones {
        let x = m.get(&b).copied().unwrap_or(Affine::IDENTITY);
        moved |= x != Affine::IDENTITY;
        for (a, v) in acc.iter_mut().zip(x.m) {
            *a += w * v;
        }
        total += w;
    }
    if !moved || total <= 0.0 {
        return None;
    }
    // Weights sum to 1; dividing keeps a hand-edited file's rounding from scaling the shape.
    Some(Affine { m: acc.map(|v| v / total) })
}

fn mix(a: Option<Affine>, b: Option<Affine>) -> Option<Affine> {
    match (a, b) {
        (None, None) => None,
        (a, b) => {
            let (a, b) = (a.unwrap_or(Affine::IDENTITY), b.unwrap_or(Affine::IDENTITY));
            let mut m = [0.0f32; 6];
            for (i, v) in m.iter_mut().enumerate() {
                *v = 0.5 * (a.m.get(i).copied().unwrap_or(0.0) + b.m.get(i).copied().unwrap_or(0.0));
            }
            Some(Affine { m })
        }
    }
}

/// The outline `pose` bends the skin's shape into (the shape's space): linear blend skinning of
/// every anchor, each cubic handle moving with the anchor it leaves or enters and a quadratic's
/// control with the mean of its two ends. At rest (or where no bound bone turned) points are
/// exactly as drawn.
pub fn deform(arm: &Armature, pose: &Pose) -> Option<Arc<Path>> {
    let skin = arm.skin.as_ref()?;
    let m = matrices(arm, pose);
    if m.values().all(|x| *x == Affine::IDENTITY) {
        return Some(skin.rest.clone());
    }
    let none = Binding::default();
    let mut idx = 0usize;
    let at = |i: usize| blend(skin.points.get(i).unwrap_or(&none), &m);
    let on = |x: Option<Affine>, p: Vec2| x.map_or(p, |x| x.apply(p));
    let mut prev: Option<Affine> = None;
    let mut cmds = Vec::with_capacity(skin.rest.cmds.len());
    for c in &skin.rest.cmds {
        let next = match *c {
            PathCmd::Close => {
                cmds.push(PathCmd::Close);
                continue;
            }
            _ => at(idx),
        };
        idx += 1;
        cmds.push(match *c {
            PathCmd::MoveTo(p) => PathCmd::MoveTo(on(next, p)),
            PathCmd::LineTo(p) => PathCmd::LineTo(on(next, p)),
            PathCmd::QuadTo(q, p) => PathCmd::QuadTo(on(mix(prev, next), q), on(next, p)),
            PathCmd::CubicTo(c1, c2, p) => PathCmd::CubicTo(on(prev, c1), on(next, c2), on(next, p)),
            PathCmd::Close => PathCmd::Close,
        });
        prev = next;
    }
    Some(Arc::new(Path { cmds }))
}

/// Puts the skin's shape in `elements` in the outline `pose` bends it into (copy-on-write).
/// Returns whether it changed.
pub fn apply(arm: &Armature, elements: &mut [Arc<Element>], pose: &Pose) -> bool {
    let Some(skin) = &arm.skin else { return false };
    let Some(e) = elements.iter_mut().find(|e| e.id == skin.shape) else { return false };
    let ElementKind::Shape(s) = &e.kind else { return false };
    let Some(path) = deform(arm, pose) else { return false };
    if Arc::ptr_eq(&s.path, &path) || *s.path == *path {
        return false;
    }
    if let ElementKind::Shape(s) = &mut Arc::make_mut(e).kind {
        s.path = path;
    }
    true
}

/// The points of a shape armature's shape in `elements` (evaluated: posed), in point order, in
/// the layer's space: the anchors of its outline as it shows.
pub fn points_shown(arm: &Armature, elements: &[Arc<Element>]) -> Vec<Vec2> {
    let Some(skin) = &arm.skin else { return Vec::new() };
    elements
        .iter()
        .find(|e| e.id == skin.shape)
        .and_then(|e| match &e.kind {
            ElementKind::Shape(sh) => Some(anchors(&sh.path).into_iter().map(|p| e.transform.apply(p)).collect()),
            _ => None,
        })
        .unwrap_or_default()
}

/// Repairs a loaded skin against its armature: the rest outline within [`MAX_SKIN_POINTS`]
/// anchors (else the skin goes), one binding per anchor, weights of bones that exist, positive
/// and summing to 1; unbound automatic anchors are bound. Returns whether anything changed.
pub fn sanitize(arm: &mut Armature) -> bool {
    let Some(skin) = &arm.skin else { return false };
    let n = anchors(&skin.rest).len();
    if n > MAX_SKIN_POINTS || arm.bones.iter().any(|b| b.child == skin.shape) {
        arm.skin = None;
        return true;
    }
    let before = arm.skin.clone();
    let ids: Vec<BoneId> = arm.bones.iter().map(|b| b.id).collect();
    let segments = rest_segments(arm);
    let Some(skin) = arm.skin.as_mut() else { return false };
    let pts = anchors(&skin.rest);
    skin.points.resize(n, Binding::default());
    for (b, &p) in skin.points.iter_mut().zip(&pts) {
        b.normalize(|id| ids.contains(&id));
        if b.bones.is_empty() && !b.manual {
            *b = auto_binding(&segments, p);
        }
    }
    arm.skin != before
}

#[cfg(test)]
#[path = "skin_tests.rs"]
mod tests;
