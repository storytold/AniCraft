//! 9-slice scaling of symbol instances (ADR 0055), as part of evaluation.
//!
//! A symbol with a [`ScaleGrid`] is drawn so that scaling an instance keeps the corners outside
//! its guides unscaled: the instance's transform is decomposed (`T · R · Skew · S`), and instead
//! of `S` the symbol's own shapes go through the 9-slice map of its box scaled by `S`
//! ([`anicraft_geom::slice`]); translation, rotation and skew apply after. Strokes keep their
//! width. Nested symbols, text and bitmap fills inside scale normally, as in Animate.

use crate::{Document, ElementKind, LayerKind, ScaleGrid, Symbol};
use anicraft_geom::slice::{Axis, Slice9};
use anicraft_geom::{Affine, Rect};

/// How a sliced instance's shapes are drawn (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Slicing {
    /// Symbol space to sliced space.
    pub map: Slice9,
    /// The instance's transform without its scale: sliced space to the parent's space.
    pub post: Affine,
    /// The scale left out of `post` (what plain scaling would have applied).
    pub scale: Affine,
}

/// Instance scale this close to 1 on both axes draws the symbol as it is.
const UNSCALED: f32 = 1e-4;

/// The box 9-slice scaling divides: the control-point bounds of everything on the symbol's
/// keyframes (guide layers left out), so the box doesn't move as the symbol plays. Strokes don't
/// widen it: they keep their width.
pub fn symbol_box(doc: &Document, sym: &Symbol) -> Rect {
    let mut r = Rect::EMPTY;
    for layer in sym.timeline.layers.iter().filter(|l| !matches!(l.kind, LayerKind::Guide | LayerKind::Folder | LayerKind::Camera)) {
        for k in &layer.keyframes {
            for e in &k.elements {
                let b = match &e.kind {
                    ElementKind::Shape(s) => s.path.bounds().transformed(&e.transform),
                    _ => e.bounds(doc),
                };
                if b.min.x.is_finite() && b.max.x.is_finite() && b.min.y.is_finite() && b.max.y.is_finite() {
                    r = r.union(&b);
                }
            }
        }
    }
    r
}

/// The slicing of an instance of `sym` placed by `transform`, or `None` when it draws normally:
/// no guides, no scale to absorb, or nothing to divide.
pub(crate) fn slicing(doc: &Document, sym: &Symbol, transform: &Affine) -> Option<Slicing> {
    let grid = sym.scale_grid.filter(|g| g.check().is_ok())?;
    let d = transform.decompose();
    let (sx, sy) = (d.sx, d.sy.abs());
    if ((sx - 1.0).abs() < UNSCALED && (sy - 1.0).abs() < UNSCALED) || !sx.is_finite() || !sy.is_finite() {
        return None;
    }
    let b = symbol_box(doc, sym);
    if b.is_empty() {
        return None;
    }
    map_for(&grid, &b, sx, sy).map(|map| {
        // `transform` is T·R·K·S; S = scale(sx, ±sy). Keep a flip in `post` (sliced magnitudes).
        let flip = if d.sy < 0.0 { -1.0 } else { 1.0 };
        let rest = anicraft_geom::Decomposed { sx: 1.0, sy: flip, ..d }.compose();
        Slicing { map, post: rest, scale: Affine::scale(sx, sy) }
    })
}

/// Whether an instance of `sym` placed by `transform` is drawn 9-sliced (exporters that place
/// symbols themselves say when they can't).
pub fn slices(doc: &Document, sym: &Symbol, transform: &Affine) -> bool {
    slicing(doc, sym, transform).is_some()
}

/// The 9-slice map of box `b` with guides `grid`, scaled by `sx`, `sy` (both at least 0). An
/// axis with no extent (a symbol only a line high) scales plainly.
pub fn map_for(grid: &ScaleGrid, b: &Rect, sx: f32, sy: f32) -> Option<Slice9> {
    let axis = |lo: f32, a: f32, c: f32, hi: f32, k: f32| if hi > lo { Axis::new(lo, a, c, hi, k) } else { Axis::new(lo, lo, lo + 1.0, lo + 1.0, k) };
    Some(Slice9 { x: axis(b.min.x, grid.left, grid.right, b.max.x, sx)?, y: axis(b.min.y, grid.top, grid.bottom, b.max.y, sy)? })
}

#[cfg(test)]
#[path = "nine_slice_tests.rs"]
mod tests;
