//! Mask → outlines: closed iso-contours of a grid of values (Paint Bucket regions).
//!
//! Marching squares at level 0.5 over cell centres, with linear interpolation along cell edges
//! and saddles resolved by the cell average, gives closed, non-intersecting loops: outer
//! boundaries and holes, which fill correctly with the even-odd rule whatever their direction.
//!
//! Ported from PhotoCraft's mask tracer (`vector::trace::contours`, same project family, MIT OR
//! Apache-2.0) and reviewed: values are read through a bounds-checked accessor (no indexing that
//! a bad size could push out of range), the grid size is validated, and points come back in
//! cell units for the caller to map. Curve fitting is `crate::fit_curve`.

use crate::Vec2;
use std::collections::HashMap;

/// Largest grid traced (cells).
pub const MAX_CELLS: usize = 16_000_000;

/// Closed contours at level 0.5 of `values` (row-major, `w × h`); cell `(x, y)` has its centre
/// at `(x + 0.5, y + 0.5)`. Values outside the grid count as 0. Empty when the sizes disagree.
pub fn contours(values: &[f32], w: usize, h: usize) -> Vec<Vec<Vec2>> {
    if w == 0 || h == 0 || w.saturating_mul(h) != values.len() || values.len() > MAX_CELLS {
        return Vec::new();
    }
    let (wi, hi) = (w as i64, h as i64);
    let get = |x: i64, y: i64| -> f32 {
        if x < 0 || y < 0 || x >= wi || y >= hi {
            return 0.0;
        }
        values.get((y * wi + x) as usize).copied().unwrap_or(0.0)
    };
    let lvl = 0.5f32;
    let point = |a: (i64, i64), b: (i64, i64)| -> Vec2 {
        let (va, vb) = (get(a.0, a.1), get(b.0, b.1));
        let t = if (vb - va).abs() > 1e-9 { ((lvl - va) / (vb - va)).clamp(0.0, 1.0) } else { 0.5 };
        let (ax, ay) = (a.0 as f32 + 0.5, a.1 as f32 + 0.5);
        let (bx, by) = (b.0 as f32 + 0.5, b.1 as f32 + 0.5);
        Vec2::new(ax + (bx - ax) * t, ay + (by - ay) * t)
    };
    // Edge ids: horizontal edge between (x,y)-(x+1,y) = (x, y, 0); vertical (x,y)-(x,y+1) = (x, y, 1).
    // Segments go edge → edge with the inside on the left (y down).
    let mut next: HashMap<(i64, i64, u8), (i64, i64, u8)> = HashMap::new();
    for cy in -1..hi {
        for cx in -1..wi {
            let tl = get(cx, cy) >= lvl;
            let tr = get(cx + 1, cy) >= lvl;
            let br = get(cx + 1, cy + 1) >= lvl;
            let bl = get(cx, cy + 1) >= lvl;
            let idx = u8::from(tl) | (u8::from(tr) << 1) | (u8::from(br) << 2) | (u8::from(bl) << 3);
            if idx == 0 || idx == 15 {
                continue;
            }
            let top = (cx, cy, 0u8);
            let bottom = (cx, cy + 1, 0u8);
            let left = (cx, cy, 1u8);
            let right = (cx + 1, cy, 1u8);
            let centre = || (get(cx, cy) + get(cx + 1, cy) + get(cx + 1, cy + 1) + get(cx, cy + 1)) / 4.0 >= lvl;
            let mut seg = |a, b| {
                next.insert(a, b);
            };
            match idx {
                1 => seg(left, top),
                2 => seg(top, right),
                3 => seg(left, right),
                4 => seg(right, bottom),
                5 => {
                    if centre() {
                        seg(left, bottom);
                        seg(right, top);
                    } else {
                        seg(left, top);
                        seg(right, bottom);
                    }
                }
                6 => seg(top, bottom),
                7 => seg(left, bottom),
                8 => seg(bottom, left),
                9 => seg(bottom, top),
                10 => {
                    if centre() {
                        seg(top, left);
                        seg(bottom, right);
                    } else {
                        seg(top, right);
                        seg(bottom, left);
                    }
                }
                11 => seg(bottom, right),
                12 => seg(right, left),
                13 => seg(right, top),
                14 => seg(top, left),
                _ => {}
            }
        }
    }
    let edge_point = |e: (i64, i64, u8)| if e.2 == 0 { point((e.0, e.1), (e.0 + 1, e.1)) } else { point((e.0, e.1), (e.0, e.1 + 1)) };
    let mut keys: Vec<(i64, i64, u8)> = next.keys().copied().collect();
    keys.sort_unstable_by_key(|k| (k.1, k.0, k.2));
    let mut out = Vec::new();
    for start in keys {
        if !next.contains_key(&start) {
            continue;
        }
        let mut lp = Vec::new();
        let mut cur = start;
        while let Some(n) = next.remove(&cur) {
            lp.push(edge_point(cur));
            cur = n;
            if cur == start {
                break;
            }
        }
        // Drop consecutive duplicates (interpolation can land on a corner).
        lp.dedup_by(|a, b| a.distance(*b) < 1e-6);
        if lp.len() >= 3 {
            out.push(lp);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(w: usize, h: usize, inside: impl Fn(usize, usize) -> bool) -> Vec<f32> {
        (0..w * h).map(|i| if inside(i % w, i / w) { 1.0 } else { 0.0 }).collect()
    }

    #[test]
    fn a_square_and_a_ring() {
        let sq = grid(10, 10, |x, y| (2..8).contains(&x) && (2..8).contains(&y));
        let loops = contours(&sq, 10, 10);
        assert_eq!(loops.len(), 1);
        let xs: Vec<f32> = loops[0].iter().map(|p| p.x).collect();
        let (lo, hi) = (xs.iter().copied().fold(f32::INFINITY, f32::min), xs.iter().copied().fold(0.0, f32::max));
        assert!((lo - 2.0).abs() < 0.01 && (hi - 8.0).abs() < 0.01, "{lo}..{hi}: edges halfway between cells");
        // A ring has an outer boundary and a hole.
        let ring = grid(20, 20, |x, y| (2..18).contains(&x) && (2..18).contains(&y) && !((7..13).contains(&x) && (7..13).contains(&y)));
        assert_eq!(contours(&ring, 20, 20).len(), 2);
    }

    #[test]
    fn mismatched_sizes_are_empty_not_a_panic() {
        assert!(contours(&[1.0; 10], 3, 3).is_empty());
        assert!(contours(&[], 0, 0).is_empty());
        assert_eq!(contours(&[1.0], 1, 1).len(), 1, "a single cell touching the border");
    }
}
