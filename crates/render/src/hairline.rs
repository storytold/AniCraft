//! Hairlines (ADR 0048): the one-pixel lines of outline mode (an outlined layer, onion skin's
//! outlines), drawn straight from the flattened path instead of as stroke polygons.
//!
//! A pixel's coverage is `1 − d`, clamped to `0..=1`, where `d` is the distance from the
//! pixel's centre to the nearest point of the line. Across a horizontal or vertical line that
//! is exactly the area a one-pixel-wide line covers in each pixel (a line through a pixel
//! centre covers it fully; one on a pixel boundary covers the two pixels beside it half each);
//! in other directions it is the line seen through a one-pixel tent filter. Overlaps and joins
//! never darken (the nearest point counts, not a sum), ends and corners are round, and each
//! segment is one edge to bin and test, where a stroked hairline was four plus a disc at every
//! bend. The GPU stage computes the same distance (`fine.wgsl` › `seg_dist`).

use crate::raster::IRect;
use anicraft_geom::Vec2;

/// How far a hairline reaches from its line, in pixels: beyond this its coverage is 0.
pub const REACH: f32 = 1.0;

/// Distance from `p` to the segment `a → b` (a point when they coincide). The GPU computes it
/// with the same steps (`fine.wgsl` › `seg_dist`).
#[inline]
pub fn seg_dist(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let (qx, qy) = (p.x - a.x, p.y - a.y);
    let l = dx * dx + dy * dy;
    let t = if l > 0.0 { ((qx * dx + qy * dy) / l).clamp(0.0, 1.0) } else { 0.0 };
    let (ex, ey) = (qx - dx * t, qy - dy * t);
    (ex * ex + ey * ey).sqrt()
}

/// A hairline's coverage at distance `d` from it.
#[inline]
pub fn coverage(d: f32) -> f32 {
    (REACH - d).clamp(0.0, 1.0)
}

/// Pixel bounds of hairlines along `polys`, within `surface`.
pub fn bounds(polys: &[Vec<Vec2>], surface: IRect) -> Option<IRect> {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
    for p in polys.iter().flatten().filter(|p| p.is_finite()) {
        x0 = x0.min(p.x);
        y0 = y0.min(p.y);
        x1 = x1.max(p.x);
        y1 = y1.max(p.y);
    }
    if !(x1 >= x0 && y1 >= y0) {
        return None;
    }
    let b = IRect {
        x0: (x0 - REACH).floor().max(surface.x0 as f32) as i32,
        y0: (y0 - REACH).floor().max(surface.y0 as f32) as i32,
        x1: (x1 + REACH).ceil().min(surface.x1 as f32) as i32,
        y1: (y1 + REACH).ceil().min(surface.y1 as f32) as i32,
    };
    (!b.is_empty()).then_some(b)
}

/// Adds segment `a → b` to `acc` (coverage over `b`, row by row, `bounds.width()` per row),
/// keeping the larger coverage in every pixel it reaches.
pub fn add_segment(acc: &mut [f32], bounds: IRect, a: Vec2, b: Vec2) {
    if !(a.is_finite() && b.is_finite()) || bounds.is_empty() {
        return;
    }
    let w = bounds.width();
    // Pixel rows whose centres lie within reach of the segment.
    let ylo = ((a.y.min(b.y) - REACH - 0.5).floor()).max(bounds.y0 as f32);
    let yhi = ((a.y.max(b.y) + REACH + 0.5).ceil()).min(bounds.y1 as f32);
    if yhi <= ylo {
        return;
    }
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    for py in ylo as i32..yhi as i32 {
        let yc = py as f32 + 0.5;
        // The part of the segment within reach of this row's centres, then the pixels within
        // reach of that part.
        let (xa, xb) = if dy.abs() < 1e-12 {
            (a.x.min(b.x), a.x.max(b.x))
        } else {
            let t0 = ((yc - REACH - a.y) / dy).clamp(0.0, 1.0);
            let t1 = ((yc + REACH - a.y) / dy).clamp(0.0, 1.0);
            let (u, v) = (a.x + dx * t0, a.x + dx * t1);
            (u.min(v), u.max(v))
        };
        let x0 = (xa - REACH - 0.5).floor().max(bounds.x0 as f32);
        let x1 = (xb + REACH + 0.5).ceil().min(bounds.x1 as f32);
        if x1 <= x0 {
            continue;
        }
        let row = (py - bounds.y0) as usize * w;
        for px in x0 as i32..x1 as i32 {
            let c = coverage(seg_dist(Vec2::new(px as f32 + 0.5, yc), a, b));
            if let Some(v) = acc.get_mut(row + (px - bounds.x0) as usize)
                && c > *v
            {
                *v = c;
            }
        }
    }
}

/// Adds an open polyline (a closed one repeats its first point at the end).
pub fn add_polyline(acc: &mut [f32], bounds: IRect, pts: &[Vec2]) {
    match pts {
        [] => {}
        [p] => add_segment(acc, bounds, *p, *p),
        _ => {
            for w in pts.windows(2) {
                add_segment(acc, bounds, w[0], w[1]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw(pts: &[Vec2], w: i32, h: i32) -> Vec<f32> {
        let b = IRect { x0: 0, y0: 0, x1: w, y1: h };
        let mut acc = vec![0.0; (w * h) as usize];
        add_polyline(&mut acc, b, pts);
        acc
    }

    #[test]
    fn axis_aligned_lines_cover_exactly_one_pixel_of_area_per_row_or_column() {
        // Through pixel centres: the row is covered fully, nothing else.
        let a = draw(&[Vec2::new(-5.0, 4.5), Vec2::new(30.0, 4.5)], 20, 10);
        for (i, v) in a.iter().enumerate() {
            let want = if i / 20 == 4 { 1.0 } else { 0.0 };
            assert!((v - want).abs() < 1e-6, "pixel {i}: {v}");
        }
        // On a pixel boundary: the two rows beside it, half each. A quarter off: 3/4 and 1/4.
        for (y, rows) in [(4.0f32, [(3usize, 0.5f32), (4, 0.5)]), (4.25, [(3, 0.25), (4, 0.75)])] {
            let a = draw(&[Vec2::new(7.5, -5.0), Vec2::new(7.5, 30.0)], 10, 20);
            assert!(a.chunks(10).all(|r| (r[7] - 1.0).abs() < 1e-6 && r[6] == 0.0), "vertical through centres");
            let a = draw(&[Vec2::new(-5.0, y), Vec2::new(30.0, y)], 20, 10);
            for (r, want) in rows {
                assert!(a[r * 20..r * 20 + 20].iter().all(|v| (v - want).abs() < 1e-6), "y {y}, row {r}");
            }
            assert!((a.iter().sum::<f32>() - 20.0).abs() < 1e-4, "one pixel of area per column");
        }
    }

    #[test]
    fn joins_and_overlaps_never_darken_and_ends_are_round() {
        // A corner and the same segment twice: still at most 1 anywhere, 1 on the line.
        let a = draw(&[Vec2::new(2.5, 2.5), Vec2::new(12.5, 2.5), Vec2::new(12.5, 12.5), Vec2::new(2.5, 2.5)], 16, 16);
        assert!(a.iter().all(|v| (0.0..=1.0).contains(v)));
        assert_eq!(a[2 * 16 + 12], 1.0);
        assert_eq!(a[8 * 16 + 12], 1.0);
        // Round end: the pixel centre half a pixel past the end, diagonally, is √0.5 away.
        let a = draw(&[Vec2::new(4.0, 4.0), Vec2::new(10.0, 4.0)], 16, 10);
        assert!((a[4 * 16 + 10] - (1.0 - 0.5f32.sqrt())).abs() < 1e-6, "{}", a[4 * 16 + 10]);
        assert_eq!(a[4 * 16 + 11], 0.0, "beyond reach past the end");
        // A single point is a dot.
        let a = draw(&[Vec2::new(3.5, 3.5)], 8, 8);
        assert_eq!(a[3 * 8 + 3], 1.0);
        assert_eq!(a.iter().filter(|v| **v > 0.0).count(), 1);
    }

    #[test]
    fn far_and_broken_points_stay_bounded() {
        let a = draw(&[Vec2::new(-1e9, -1e9), Vec2::new(1e9, 1e9), Vec2::new(f32::NAN, 0.0), Vec2::new(3.0, f32::INFINITY)], 32, 32);
        assert!(a.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
        assert!(a.iter().any(|v| *v > 0.9), "the long diagonal shows");
        assert!(bounds(&[vec![Vec2::new(f32::NAN, 1.0)]], IRect { x0: 0, y0: 0, x1: 8, y1: 8 }).is_none());
    }
}
