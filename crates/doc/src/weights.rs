//! Painted weights and reshaping a bound shape (ADR 0090), on top of shape armatures (ADR 0071).
//!
//! **Painting.** The Bind tool's brush changes how much one bone moves the points under it: each
//! dab reaches the points within its radius with a smooth falloff (`(1 − (d/r)²)²`, 1 at the
//! centre, 0 at the rim, with no kink at either), times the brush's strength. *Add* raises the
//! bone's share, *Subtract* lowers it, *Smooth* pulls it toward the falloff-weighted mean of the
//! bone's share at the points around it. After every change the point's weights are normalised
//! ([`with_share`]): the bone keeps the share it was painted to and the point's other bones share
//! what is left in the proportions they had, so the weights always sum to 1. A point that
//! followed only the painted bone gives what it loses to the bones automatic binding would give
//! it (its nearest other bones), so subtracting is never a no-op on a multi-bone shape. Painted
//! points become hand-bound (`manual`), so adding a bone later doesn't undo the painting.
//!
//! **Reshaping.** Editing the outline at rest (Subselection: moving, adding, deleting or
//! converting points) gives the skin a new rest outline. [`reshape`] carries the binding over:
//! anchors are matched old to new in path order by exact position (an edit that keeps the number
//! of anchors keeps every binding by index), deleted anchors drop theirs, and each new anchor
//! gets the weights of its matched neighbours along its subpath, blended by its distance along
//! the outline between them. A new subpath with no neighbours is bound automatically. Nothing
//! else changes, so the rest pose shows the edited outline bit for bit.

use crate::armature::BoneId;
use crate::skin::{Binding, Influence, Skin, anchors, auto_binding};
use anicraft_geom::{Path, PathCmd, Vec2};
use std::collections::HashMap;
use std::sync::Arc;

/// Largest brush radius (px).
pub const MAX_RADIUS: f32 = 2000.0;
/// Most dabs one paint command takes.
pub const MAX_DABS: usize = 4096;

/// What a dab does to the painted bone's share.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Brush {
    Add,
    Subtract,
    Smooth,
}

impl Brush {
    pub fn key(self) -> &'static str {
        match self {
            Brush::Add => "add",
            Brush::Subtract => "subtract",
            Brush::Smooth => "smooth",
        }
    }

    pub fn parse(s: &str) -> Option<Brush> {
        match s {
            "add" => Some(Brush::Add),
            "subtract" => Some(Brush::Subtract),
            "smooth" => Some(Brush::Smooth),
            _ => None,
        }
    }
}

/// One brush: what it does, its radius (px) and its strength at the centre (0…1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dab {
    pub brush: Brush,
    pub radius: f32,
    pub strength: f32,
}

/// How much a dab of radius `r` reaches a point `d` away: `(1 − (d/r)²)²` inside, 0 outside.
pub fn falloff(d: f32, r: f32) -> f32 {
    if !(r > 0.0 && d.is_finite()) || d >= r {
        return 0.0;
    }
    let x = d / r;
    let u = 1.0 - x * x;
    u * u
}

/// `b` with bone `bone`'s share set to `w` (clamped to 0…1) and the point's other bones sharing
/// the rest in their old proportions; when it has no other bones, `fallback` names the bones
/// that take the rest (by weight). Normalised (the strongest four, summing to 1) and hand-bound.
/// With no other bone at all the bone keeps the whole point, or none of it when `w` is 0.
pub fn with_share(b: &Binding, bone: BoneId, w: f32, fallback: impl FnOnce() -> Vec<Influence>) -> Binding {
    let w = if w.is_finite() { w.clamp(0.0, 1.0) } else { b.weight(bone) };
    let mut others: Vec<Influence> = b.bones.iter().copied().filter(|i| i.0 != bone && i.1.is_finite() && i.1 > 0.0).collect();
    if others.is_empty() && w < 1.0 {
        others = fallback().into_iter().filter(|i| i.0 != bone && i.1.is_finite() && i.1 > 0.0).collect();
    }
    let rest: f32 = others.iter().map(|i| i.1).sum();
    let mut out = Binding { bones: Vec::with_capacity(others.len() + 1), manual: true };
    if w > 0.0 {
        out.bones.push(Influence(bone, w));
    }
    if rest > 0.0 && rest.is_finite() {
        out.bones.extend(others.iter().map(|i| Influence(i.0, i.1 / rest * (1.0 - w))));
    }
    out.normalize(|_| true);
    out
}

/// The bones automatic binding gives a point at `p` (as drawn) leaving out `bone`: who takes a
/// point's weight when the only bone it followed gives some up.
pub fn others_at(segments: &[(BoneId, Vec2, Vec2)], bone: BoneId, p: Vec2) -> Vec<Influence> {
    let without: Vec<(BoneId, Vec2, Vec2)> = segments.iter().copied().filter(|s| s.0 != bone).collect();
    if without.is_empty() {
        return Vec::new();
    }
    auto_binding(&without, p).bones
}

/// Paints bone `bone`'s share onto the skin's points: one dab of `with` at each of `dabs`,
/// reaching the points within its radius (both measured where `shown` has the points: on the
/// stage as they show). `segments` are the bones as drawn
/// ([`crate::skin::rest_segments`]). Returns how many points changed.
pub fn paint(skin: &mut Skin, segments: &[(BoneId, Vec2, Vec2)], shown: &[Vec2], bone: BoneId, dabs: &[Vec2], with: Dab) -> usize {
    let brush = with.brush;
    let radius = if with.radius.is_finite() { with.radius.clamp(0.0, MAX_RADIUS) } else { 0.0 };
    let strength = if with.strength.is_finite() { with.strength.clamp(0.0, 1.0) } else { 0.0 };
    if radius <= 0.0 || strength <= 0.0 {
        return 0;
    }
    let rest = anchors(&skin.rest);
    let n = skin.points.len().min(shown.len()).min(rest.len());
    let mut changed = vec![false; n];
    let mut near: Vec<(usize, f32)> = Vec::new();
    for &c in dabs.iter().take(MAX_DABS) {
        if !(c.x.is_finite() && c.y.is_finite()) {
            continue;
        }
        near.clear();
        for (i, p) in shown.iter().take(n).enumerate() {
            // A cheap box test first: most points are far from a dab.
            if (p.x - c.x).abs() >= radius || (p.y - c.y).abs() >= radius {
                continue;
            }
            let f = falloff(p.distance(c), radius);
            if f > 0.0 {
                near.push((i, f));
            }
        }
        // Smoothing pulls toward the mean before this dab changed anything.
        let mean = if brush == Brush::Smooth {
            let (sum, wsum) = near.iter().fold((0.0f32, 0.0f32), |(s, t), &(i, f)| (s + f * skin.points.get(i).map_or(0.0, |b| b.weight(bone)), t + f));
            if wsum > 0.0 { sum / wsum } else { 0.0 }
        } else {
            0.0
        };
        for &(i, f) in &near {
            let (Some(b), Some(&at)) = (skin.points.get_mut(i), rest.get(i)) else { continue };
            let w = b.weight(bone);
            let target = match brush {
                Brush::Add => w + strength * f,
                Brush::Subtract => w - strength * f,
                Brush::Smooth => w + strength * f * (mean - w),
            }
            .clamp(0.0, 1.0);
            if (target - w).abs() <= 1e-6 {
                continue;
            }
            let next = with_share(b, bone, target, || others_at(segments, bone, at));
            if next != *b {
                *b = next;
                if let Some(c) = changed.get_mut(i) {
                    *c = true;
                }
            }
        }
    }
    changed.iter().filter(|c| **c).count()
}

/// Which subpath each anchor of `path` is on, and each subpath's anchor range and whether it is
/// closed (a `Close`, or ending where it starts).
fn subpaths(path: &Path) -> (Vec<usize>, Vec<(usize, usize, bool)>) {
    let mut of = Vec::new();
    let mut subs: Vec<(usize, usize, bool)> = Vec::new();
    for c in &path.cmds {
        match c {
            PathCmd::MoveTo(_) => {
                subs.push((of.len(), of.len() + 1, false));
                of.push(subs.len() - 1);
            }
            PathCmd::Close => {
                if let Some(s) = subs.last_mut() {
                    s.2 = true;
                }
            }
            _ => {
                if subs.is_empty() {
                    subs.push((0, 0, false));
                }
                let k = subs.len() - 1;
                if let Some(s) = subs.last_mut() {
                    s.1 = of.len() + 1;
                }
                of.push(k);
            }
        }
    }
    let pts = anchors(path);
    for s in &mut subs {
        if let (Some(a), Some(b)) = (pts.get(s.0), pts.get(s.1.saturating_sub(1))) {
            s.2 |= s.1 > s.0 + 1 && a == b;
        }
    }
    (of, subs)
}

/// The binding of a new rest outline `path` for `skin`'s bindings (see the module docs):
/// matched anchors keep theirs, new ones blend their neighbours', deleted ones drop theirs.
/// `segments` bind a new subpath that has no neighbours. One binding per anchor of `path`.
pub fn reshape(skin: &Skin, path: &Path, segments: &[(BoneId, Vec2, Vec2)]) -> Vec<Binding> {
    let old = anchors(&skin.rest);
    let new = anchors(path);
    let none = Binding::default();
    if old.len() == new.len() {
        // Points moved or turned smooth: the same anchors, the same bindings.
        return (0..new.len()).map(|i| skin.points.get(i).cloned().unwrap_or_default()).collect();
    }
    // Match in path order by exact position: each new anchor takes the first old anchor after
    // the last match that sits where it does.
    let key = |p: Vec2| (p.x.to_bits(), p.y.to_bits());
    let mut at: HashMap<(u32, u32), Vec<usize>> = HashMap::with_capacity(old.len());
    for (i, p) in old.iter().enumerate() {
        at.entry(key(*p)).or_default().push(i);
    }
    let mut matched: Vec<Option<usize>> = Vec::with_capacity(new.len());
    let mut next_old = 0usize;
    for p in &new {
        let m = at.get(&key(*p)).and_then(|v| {
            let k = v.partition_point(|&i| i < next_old);
            v.get(k).copied()
        });
        if let Some(i) = m {
            next_old = i + 1;
        }
        matched.push(m);
    }
    let (of, subs) = subpaths(path);
    let binding = |i: usize| skin.points.get(i).unwrap_or(&none);
    (0..new.len())
        .map(|j| {
            if let Some(i) = matched.get(j).copied().flatten() {
                return binding(i).clone();
            }
            let p = new.get(j).copied().unwrap_or(Vec2::ZERO);
            let Some(&(s, e, closed)) = of.get(j).and_then(|k| subs.get(*k)) else { return auto_binding(segments, p) };
            let len = e.saturating_sub(s).max(1);
            // Walks from `j` one way along its subpath (wrapping round a closed one) to the
            // nearest matched anchor, adding up the distance between anchors on the way.
            let walk = |forward: bool| -> Option<(usize, f32)> {
                let (mut k, mut d) = (j, 0.0f32);
                for _ in 1..len {
                    let step = if forward {
                        if k + 1 < e { k + 1 } else if closed { s } else { return None }
                    } else if k > s {
                        k - 1
                    } else if closed {
                        e - 1
                    } else {
                        return None;
                    };
                    d += new.get(k).zip(new.get(step)).map_or(0.0, |(a, b)| a.distance(*b));
                    k = step;
                    if let Some(i) = matched.get(k).copied().flatten() {
                        return Some((i, d));
                    }
                }
                None
            };
            match (walk(false), walk(true)) {
                (Some((a, da)), Some((b, db))) => {
                    let t = if da + db > 0.0 { da / (da + db) } else { 0.5 };
                    blend(binding(a), binding(b), t)
                }
                (Some((a, _)), None) | (None, Some((a, _))) => binding(a).clone(),
                (None, None) => auto_binding(segments, p),
            }
        })
        .collect()
}

/// `(1 − t)·a + t·b`, normalised; hand-bound when either is.
fn blend(a: &Binding, b: &Binding, t: f32) -> Binding {
    let mut bones: Vec<Influence> = a.bones.iter().map(|i| Influence(i.0, i.1 * (1.0 - t))).collect();
    for i in &b.bones {
        match bones.iter_mut().find(|x| x.0 == i.0) {
            Some(x) => x.1 += i.1 * t,
            None => bones.push(Influence(i.0, i.1 * t)),
        }
    }
    let mut out = Binding { bones, manual: a.manual || b.manual };
    out.normalize(|_| true);
    out
}

/// `skin` with its rest outline replaced by `path` and its binding carried over ([`reshape`]).
pub fn reshaped(skin: &Skin, path: Arc<Path>, segments: &[(BoneId, Vec2, Vec2)]) -> Skin {
    let points = reshape(skin, &path, segments);
    Skin { shape: skin.shape, rest: path, points }
}

#[cfg(test)]
#[path = "weights_tests.rs"]
mod tests;
