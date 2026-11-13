//! Shape hints (ADR 0023): lettered points that steer a shape tween's morph.
//!
//! A hint lives on the keyframe that starts the shape tween: a point on that keyframe's
//! outline and the point on the next keyframe's outline it becomes, both in the layer's own
//! space. Whether a point is *placed* is never stored: it is placed while it lies on a shape's
//! outline (within [`HINT_TOLERANCE`]). A hint just added sits at the shapes' centre, so it is
//! unplaced until dragged onto an edge; an edit that moves the edge away unplaces it again.
//! The morph uses only hints placed on both keyframes, on the same pair of shapes.

use crate::{Element, ElementKind, Keyframe, Layer, TweenKind};
use anicraft_geom::{Rect, Vec2, morph};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Most hints a shape tween holds: one per letter, `a` to `z`.
pub const MAX_SHAPE_HINTS: usize = morph::MAX_HINTS;

/// How far (in layer units) a hint may sit from an outline and still count as placed on it.
pub const HINT_TOLERANCE: f32 = 0.5;

/// One shape hint: where it sits on the start keyframe and on the end keyframe (layer space).
/// Its letter is its position in the list.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShapeHint {
    pub start: anicraft_geom::Vec2,
    pub end: anicraft_geom::Vec2,
}

/// The letter of hint `i` (`a` for the first).
pub fn hint_letter(i: usize) -> char {
    char::from(b'a' + (i.min(25) as u8))
}

/// The hint index for a letter (`a`..`z`, either case).
pub fn hint_index(letter: &str) -> Option<usize> {
    let mut it = letter.chars();
    let c = it.next()?.to_ascii_lowercase();
    if it.next().is_some() || !c.is_ascii_lowercase() {
        return None;
    }
    Some((c as u8 - b'a') as usize)
}

/// Which keyframe of a shape tween a hint point belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HintSide {
    Start,
    End,
}

impl HintSide {
    pub fn key(self) -> &'static str {
        match self {
            HintSide::Start => "start",
            HintSide::End => "end",
        }
    }
    pub fn from_key(k: &str) -> Option<HintSide> {
        match k {
            "start" => Some(HintSide::Start),
            "end" => Some(HintSide::End),
            _ => None,
        }
    }
}

impl ShapeHint {
    pub fn at(&self, side: HintSide) -> Vec2 {
        match side {
            HintSide::Start => self.start,
            HintSide::End => self.end,
        }
    }
    pub fn at_mut(&mut self, side: HintSide) -> &mut Vec2 {
        match side {
            HintSide::Start => &mut self.start,
            HintSide::End => &mut self.end,
        }
    }
}

impl Layer {
    /// Whether keyframe `i` starts a shape tween that runs into the next keyframe.
    pub fn is_shape_tween(&self, i: usize) -> bool {
        let (Some(k), Some(next)) = (self.keyframes.get(i), self.keyframes.get(i + 1)) else { return false };
        k.motion.is_none() && k.tween.as_ref().is_some_and(|t| t.kind == TweenKind::Shape) && next.start == k.end()
    }

    /// The shape tween whose hints `frame` shows or edits, and which side: a frame inside a
    /// shape tween's span is its start side; the first frame of the keyframe a shape tween
    /// ends on is its end side (unless that keyframe starts a shape tween of its own).
    pub fn hint_tween_at(&self, frame: u32) -> Option<(usize, HintSide)> {
        let i = self.keyframe_index_at(frame)?;
        if self.is_shape_tween(i) {
            return Some((i, HintSide::Start));
        }
        let k = self.keyframes.get(i)?;
        let prev = i.checked_sub(1)?;
        (k.start == frame && self.is_shape_tween(prev)).then_some((prev, HintSide::End))
    }

    /// The keyframe a side of shape tween `i` shows: `i` itself, or the next one.
    pub fn hint_keyframe(&self, i: usize, side: HintSide) -> Option<&Keyframe> {
        self.keyframes.get(if side == HintSide::Start { i } else { i + 1 }).map(|k| &**k)
    }
}

/// The shape outline among `elements` nearest `p` (layer space): the element's index, the
/// point on its outline and the distance. Only shapes take hints.
pub fn nearest_outline(elements: &[Arc<Element>], p: Vec2) -> Option<(usize, Vec2, f32)> {
    let mut best: Option<(usize, Vec2, f32)> = None;
    for (i, e) in elements.iter().enumerate() {
        let ElementKind::Shape(s) = &e.kind else { continue };
        let Some(inv) = e.transform.inverse() else { continue };
        let Some((q, _)) = morph::nearest_point(&s.path, inv.apply(p)) else { continue };
        let q = e.transform.apply(q);
        let d = q.distance(p);
        if d.is_finite() && best.is_none_or(|b| d < b.2) {
            best = Some((i, q, d));
        }
    }
    best
}

/// The shape element hint point `p` is placed on, if it lies on an outline.
pub fn placed_on(elements: &[Arc<Element>], p: Vec2) -> Option<usize> {
    nearest_outline(elements, p).filter(|b| b.2 <= HINT_TOLERANCE).map(|b| b.0)
}

/// The centre of the shapes among `elements` (layer space): where a new hint appears.
pub fn shapes_centre(elements: &[Arc<Element>]) -> Option<Vec2> {
    let r = elements.iter().fold(Rect::EMPTY, |r, e| match &e.kind {
        ElementKind::Shape(s) => r.union(&s.path.transformed(&e.transform).bounds()),
        _ => r,
    });
    (!r.is_empty() && r.center().is_finite()).then(|| r.center())
}

/// The hints a shape tween from `kf` to `next` applies: those placed on the same element pair
/// on both keyframes, as that pair's index and the points in each element's own coordinates.
pub(crate) fn hint_pairs(kf: &Keyframe, next: &Keyframe) -> Vec<(usize, Vec2, Vec2)> {
    kf.hints
        .iter()
        .take(MAX_SHAPE_HINTS)
        .filter_map(|h| {
            let i = placed_on(&kf.elements, h.start)?;
            (placed_on(&next.elements, h.end)? == i).then_some(())?;
            let (a, b) = (kf.elements.get(i)?, next.elements.get(i)?);
            Some((i, a.transform.inverse()?.apply(h.start), b.transform.inverse()?.apply(h.end)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_round_trip() {
        assert_eq!(hint_letter(0), 'a');
        assert_eq!(hint_letter(25), 'z');
        assert_eq!(hint_index("a"), Some(0));
        assert_eq!(hint_index("Z"), Some(25));
        for bad in ["", "ab", "1", "é", "-"] {
            assert_eq!(hint_index(bad), None, "{bad}");
        }
    }
}
