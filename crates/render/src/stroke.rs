//! Stroker: polylines → positively-wound polygons (segment quads plus round joins and caps).
//!
//! Every emitted polygon has the same orientation, so overlaps only add winding and the
//! non-zero rule gives a seamless union without computing an outline offset.

use anicraft_geom::{Polyline, Vec2};

fn signed_area(p: &[Vec2]) -> f32 {
    let n = p.len();
    let mut a = 0.0;
    for i in 0..n {
        let (Some(u), Some(v)) = (p.get(i), p.get((i + 1) % n)) else { continue };
        a += u.cross(*v);
    }
    a * 0.5
}

fn oriented(mut poly: Vec<Vec2>) -> Vec<Vec2> {
    if signed_area(&poly) < 0.0 {
        poly.reverse();
    }
    poly
}

/// Polygon approximating a circle within ~0.1 px.
fn disc(c: Vec2, r: f32) -> Vec<Vec2> {
    let tol = 0.1f32;
    let n = if r > tol { (std::f32::consts::PI / (1.0 - tol / r).clamp(-1.0, 1.0).acos()).ceil() as usize } else { 6 };
    let n = n.clamp(6, 96);
    (0..n)
        .map(|i| {
            let a = i as f32 * std::f32::consts::TAU / n as f32;
            Vec2::new(c.x + r * a.cos(), c.y + r * a.sin())
        })
        .collect()
}

/// Polygons covering the stroke of `lines` with half-width `hw` (pixels).
pub fn stroke_polygons(lines: &[Polyline], hw: f32) -> Vec<Vec<Vec2>> {
    let mut out = Vec::new();
    if !(hw.is_finite() && hw > 0.0) {
        return out;
    }
    for pl in lines {
        let pts = &pl.points;
        let n = pts.len();
        if n < 2 {
            continue;
        }
        let segs = if pl.closed { n } else { n - 1 };
        for i in 0..segs {
            let (Some(&a), Some(&b)) = (pts.get(i), pts.get((i + 1) % n)) else { continue };
            let d = b - a;
            if d.length() < 1e-6 {
                continue;
            }
            let nrm = d.normalized().perp() * hw;
            out.push(oriented(vec![a + nrm, b + nrm, b - nrm, a - nrm]));
        }
        // Round joins at interior vertices (all vertices when closed) and round caps at ends.
        for (i, &p) in pts.iter().enumerate() {
            let interior = pl.closed || (i > 0 && i + 1 < n);
            let is_end = !pl.closed && (i == 0 || i + 1 == n);
            if interior {
                // Skip joins on nearly straight runs (common on flattened curves): the quads
                // already overlap seamlessly there.
                let prev = pts.get(if i == 0 { n - 1 } else { i - 1 });
                let next = pts.get((i + 1) % n);
                if let (Some(&a), Some(&b)) = (prev, next) {
                    let (u, v) = ((p - a).normalized(), (b - p).normalized());
                    // the uncovered outer wedge is about hw · sin(turn) wide
                    if u.dot(v) > 0.0 && u.cross(v).abs() * hw < 0.05 {
                        continue;
                    }
                }
            }
            if interior || is_end {
                out.push(oriented(disc(p, hw)));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_polygons_positive() {
        let pl = Polyline { points: vec![Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0)], closed: false };
        let polys = stroke_polygons(&[pl], 2.0);
        assert!(polys.len() >= 4);
        assert!(polys.iter().all(|p| signed_area(p) > 0.0));
    }

    #[test]
    fn degenerate_inputs() {
        assert!(stroke_polygons(&[], 1.0).is_empty());
        let pl = Polyline { points: vec![Vec2::ZERO, Vec2::ZERO], closed: false };
        assert!(stroke_polygons(std::slice::from_ref(&pl), f32::NAN).is_empty());
        // zero-length segment still yields caps, no NaN
        let polys = stroke_polygons(&[pl], 1.0);
        assert!(polys.iter().flatten().all(|p| p.is_finite()));
    }
}
