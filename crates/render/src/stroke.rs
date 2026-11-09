//! Stroker: polylines → positively-wound polygons (segment quads plus round joins and caps),
//! of one width or following a width profile.
//!
//! Every emitted polygon has the same orientation, so overlaps only add winding and the
//! non-zero rule gives a seamless union without computing an outline offset.

use anicraft_doc::width::left_normal;
use anicraft_doc::{DisplayItem, DisplayList, WidthProfile};
use anicraft_geom::{Affine, Path, Polyline, Vec2};

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

/// Arc tolerance for joins and caps, in output units.
const ARC_TOL: f32 = 0.1;
/// Most points a variable-width polyline is resampled to.
const MAX_SAMPLES: usize = 8192;
/// Widths below this (output units) draw nothing.
const NO_WIDTH: f32 = 1e-4;

/// A fan from `c`: an arc of `sweep` radians starting at angle `a0`, its radius going from
/// `r0` to `r1`.
fn fan(c: Vec2, a0: f32, sweep: f32, r0: f32, r1: f32) -> Option<Vec<Vec2>> {
    let r = r0.max(r1);
    if !(r > NO_WIDTH && sweep.is_finite() && a0.is_finite()) {
        return None;
    }
    let step = 2.0 * (1.0 - ARC_TOL / r).clamp(-1.0, 1.0).acos();
    let n = if step > 1e-3 { (sweep.abs() / step).ceil() as usize } else { 64 };
    let n = n.clamp(2, 64);
    let mut poly = Vec::with_capacity(n + 2);
    poly.push(c);
    for i in 0..=n {
        let k = i as f32 / n as f32;
        let a = a0 + sweep * k;
        let rr = r0 + (r1 - r0) * k;
        poly.push(Vec2::new(c.x + rr * a.cos(), c.y + rr * a.sin()));
    }
    Some(oriented(poly))
}

fn angle(v: Vec2) -> f32 {
    v.y.atan2(v.x)
}

/// Polygons covering a variable-width stroke of `lines` (output space), all positively wound
/// like [`stroke_polygons`], so non-zero filling gives their union.
///
/// `hw` is the nominal half-width in output units; the profile scales it on each side. The
/// profile's positions are fractions of each subpath's arc length measured in the shape's
/// own space: `to_local` maps output points back there (so the widths stay put whatever the
/// zoom or transform). `flip` swaps the sides, for transforms that mirror.
///
/// Each segment is a trapezoid whose ends are perpendicular to it, with the half-widths of
/// its two end points on each side; the outer side of every bend gets a round join of that
/// side's width, and open ends a round cap across their cross-section. Long segments are subdivided first so the widths follow the profile.
pub fn variable_polygons(lines: &[Polyline], hw: f32, profile: &WidthProfile, to_local: Option<&Affine>, flip: bool) -> Vec<Vec<Vec2>> {
    let mut out = Vec::new();
    if !(hw.is_finite() && hw > 0.0) {
        return out;
    }
    for pl in lines {
        let pts = resample(pl);
        let n = pts.len();
        if n < 2 {
            continue;
        }
        // Arc-length positions in the shape's space.
        let local: Vec<Vec2> = match to_local {
            Some(m) => pts.iter().map(|p| m.apply(*p)).collect(),
            None => pts.clone(),
        };
        let mut cum = Vec::with_capacity(n);
        let mut d = 0.0f32;
        cum.push(0.0);
        for w in local.windows(2) {
            d += w[0].distance(w[1]);
            cum.push(d);
        }
        let total = if d.is_finite() && d > 0.0 { d } else { 1.0 };
        let widths: Vec<(f32, f32)> = cum
            .iter()
            .map(|c| {
                let (l, r) = profile.at(c / total);
                if flip { (r * hw, l * hw) } else { (l * hw, r * hw) }
            })
            .collect();
        let w = |i: usize| widths.get(i).copied().unwrap_or((0.0, 0.0));
        let p = |i: usize| pts.get(i).copied().unwrap_or(Vec2::ZERO);
        // Direction of the segment from point i to i + 1.
        let dir = |i: usize| (p(i + 1) - p(i)).normalized();
        for i in 0..n - 1 {
            let (a, b) = (p(i), p(i + 1));
            if a.distance(b) < 1e-6 {
                continue;
            }
            let ((la, ra), (lb, rb)) = (w(i), w(i + 1));
            if la.max(ra).max(lb).max(rb) <= NO_WIDTH {
                continue;
            }
            let nrm = left_normal(dir(i));
            out.push(oriented(vec![a + nrm * la, b + nrm * lb, b - nrm * rb, a - nrm * ra]));
        }
        // Joins at bends (with a closed subpath's start, where its last segment meets its first).
        let closed = pl.closed && p(0).distance(p(n - 1)) < 1e-4;
        let joints = (1..n - 1).map(|i| (i, i - 1, i)).chain(closed.then_some((0, n - 2, 0)));
        for (at, before, after) in joints {
            let (u, v) = (dir(before), dir(after));
            let (l, r) = w(at);
            // Nearly straight: the trapezoids already meet seamlessly.
            if u.dot(v) > 0.0 && u.cross(v).abs() * l.max(r) < 0.05 {
                continue;
            }
            // The gap opens on the outside of the bend, around the direction u − v.
            let mid = (u - v).normalized();
            let (n0, n1) = (left_normal(u), left_normal(v));
            let (s, radius) = if mid.dot(n0) >= 0.0 { (1.0, l) } else { (-1.0, r) };
            let (a0, a1, am) = (angle(n0 * s), angle(n1 * s), angle(mid));
            let wrap = |x: f32| (x + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
            let mut sweep = wrap(a1 - a0);
            let to_mid = wrap(am - a0);
            // Go round the side that passes through the gap's middle.
            if to_mid * sweep < 0.0 || to_mid.abs() > sweep.abs() {
                sweep -= std::f32::consts::TAU.copysign(sweep);
            }
            out.extend(fan(p(at), a0, sweep, radius, radius));
        }
        if !closed {
            // Caps: half a disc across the end's cross-section (from its left edge to its
            // right), so a one-sided width stays on its side.
            let cap = |at: Vec2, nrm: Vec2, (l, r): (f32, f32), sweep: f32| fan(at + nrm * ((l - r) * 0.5), angle(nrm), sweep, (l + r) * 0.5, (l + r) * 0.5);
            // (Turning the left normal by −90° on screen points backwards.)
            out.extend(cap(p(0), left_normal(dir(0)), w(0), -std::f32::consts::PI));
            out.extend(cap(p(n - 1), left_normal(dir(n - 2)), w(n - 1), std::f32::consts::PI));
        }
    }
    out
}

/// A polyline's points with long segments subdivided (so widths can follow a profile), the
/// start repeated at the end when it is closed, and repeated points dropped.
fn resample(pl: &Polyline) -> Vec<Vec2> {
    let mut src: Vec<Vec2> = pl.points.iter().copied().filter(|p| p.is_finite()).collect();
    if pl.closed
        && let (Some(&first), Some(&last)) = (src.first(), src.last())
        && first.distance(last) > 1e-6
    {
        src.push(first);
    }
    let len: f32 = src.windows(2).map(|w| w[0].distance(w[1])).sum();
    // About 256 samples along the subpath, but none closer than ¾ px or farther than 8 px.
    let step = (len / 256.0).clamp(0.75, 8.0);
    let mut out: Vec<Vec2> = Vec::with_capacity(src.len().min(MAX_SAMPLES));
    for (i, &b) in src.iter().enumerate() {
        if let Some(&a) = i.checked_sub(1).and_then(|j| src.get(j)) {
            let d = a.distance(b);
            let k = if src.len() < MAX_SAMPLES && d.is_finite() { ((d / step).ceil() as usize).clamp(1, MAX_SAMPLES) } else { 1 };
            for j in 1..k {
                if out.len() >= MAX_SAMPLES {
                    break;
                }
                out.push(a.lerp(b, j as f32 / k as f32));
            }
        }
        if out.last().is_none_or(|q| q.distance(b) > 1e-6) {
            out.push(b);
        }
    }
    out
}

/// A variable-width stroke as a filled outline in `m`'s output space, for vector exports: the
/// same polygons the renderer fills, as closed subpaths (fill them non-zero).
pub fn variable_outline(path: &Path, m: &Affine, width: f32, profile: &WidthProfile, tolerance: f32) -> Path {
    let lines = path.flatten(m, tolerance);
    let to_local = m.inverse();
    let flip = m.determinant() < 0.0;
    let mut out = Path::new();
    for poly in variable_polygons(&lines, width * 0.5 * m.mean_scale(), profile, to_local.as_ref(), flip) {
        let mut it = poly.into_iter();
        let Some(first) = it.next() else { continue };
        out.move_to(first);
        for q in it {
            out.line_to(q);
        }
        out.close();
    }
    out
}

/// `dl` with every variable-width stroke turned into a filled outline item, drawn right after
/// its shape (same layer, element, clip, group and colour effect), so vector exports show what
/// the renderer shows. Borrowed unchanged when there are none.
pub fn outline_variable_strokes(dl: &DisplayList) -> std::borrow::Cow<'_, DisplayList> {
    let varies = |i: &DisplayItem| i.masks.is_none() && !i.guide && i.outline.is_none() && i.stroke.as_ref().is_some_and(|s| s.profile.is_some());
    if !dl.items.iter().any(varies) {
        return std::borrow::Cow::Borrowed(dl);
    }
    let mut items = Vec::with_capacity(dl.items.len() + 8);
    for it in &dl.items {
        let Some((st, profile)) = it.stroke.as_ref().and_then(|s| s.profile.as_ref().map(|p| (s, p))).filter(|_| varies(it)) else {
            items.push(it.clone());
            continue;
        };
        if it.fill.is_some() {
            items.push(DisplayItem { stroke: None, ..it.clone() });
        }
        if !it.transform.is_finite() {
            continue;
        }
        let outline = variable_outline(&it.path, &it.transform, st.width, profile, crate::TOLERANCE);
        if !outline.is_empty() {
            items.push(DisplayItem { path: std::sync::Arc::new(outline), transform: Affine::IDENTITY, fill: Some(anicraft_doc::Paint::Solid(st.color)), stroke: None, fill_rule: anicraft_geom::FillRule::NonZero, image: None, ..it.clone() });
        }
    }
    std::borrow::Cow::Owned(DisplayList { items, clip_parents: dl.clip_parents.clone(), groups: dl.groups.clone() })
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

    fn area(polys: &[Vec<Vec2>]) -> f32 {
        polys.iter().map(|p| signed_area(p)).sum()
    }

    #[test]
    fn variable_polygons_are_positive_and_follow_the_profile() {
        let line = Polyline { points: vec![Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)], closed: false };
        let taper = anicraft_doc::ProfilePreset::TaperBoth.profile().unwrap();
        let polys = variable_polygons(std::slice::from_ref(&line), 5.0, &taper, None, false);
        assert!(polys.len() > 50, "the line is subdivided so the width can vary: {}", polys.len());
        assert!(polys.iter().all(|p| signed_area(p) >= 0.0 && p.iter().all(|q| q.is_finite())));
        // Zero width at both ends: no caps, and the outline stays within the profile.
        let half = |x: f32| polys.iter().flatten().filter(|q| (q.x - x).abs() < 0.2).map(|q| q.y.abs()).fold(0.0f32, f32::max);
        assert!(half(0.0) < 0.2 && half(100.0) < 0.2, "{} {}", half(0.0), half(100.0));
        assert!((half(50.0) - 5.0).abs() < 0.05, "full width in the middle: {}", half(50.0));
        // One side only: the other side stays on the line.
        let one = WidthProfile::new(vec![anicraft_doc::WidthPoint::new(0.5, 2.0, 0.0)]).unwrap();
        let polys = variable_polygons(std::slice::from_ref(&line), 5.0, &one, None, false);
        let ys: Vec<f32> = polys.iter().flatten().map(|q| q.y).collect();
        assert!(ys.iter().all(|&y| y < 1e-3) && ys.iter().any(|&y| (y + 10.0).abs() < 1e-3), "heading +x on a y-down screen, the left is -y");
        // Mirrored transforms swap the sides.
        let flipped = variable_polygons(&[line], 5.0, &one, None, true);
        assert!(flipped.iter().flatten().all(|q| q.y > -1e-3));
        // Degenerate input is safe.
        assert!(variable_polygons(&[], 1.0, &one, None, false).is_empty());
        assert!(variable_polygons(&[Polyline { points: vec![Vec2::ZERO; 3], closed: true }], 1.0, &one, None, false).iter().flatten().all(|q| q.is_finite()));
        assert!(variable_polygons(&[Polyline { points: vec![Vec2::ZERO, Vec2::new(f32::INFINITY, 0.0)], closed: false }], f32::NAN, &one, None, false).is_empty());
    }

    #[test]
    fn a_sharp_bend_gets_a_join_on_its_outer_side() {
        // Right 20, then straight back: the outer side of the U-turn is the front.
        let pl = Polyline { points: vec![Vec2::new(0.0, 0.0), Vec2::new(20.0, 0.0), Vec2::new(20.0, 10.0)], closed: false };
        let fat = WidthProfile::new(vec![anicraft_doc::WidthPoint::new(0.0, 2.0, 2.0)]).unwrap();
        let polys = variable_polygons(&[pl], 2.0, &fat, None, false);
        // The corner's outer quarter (x > 20, y < 0) is covered by the join, within radius 4.
        let reach = polys.iter().flatten().filter(|q| q.x > 20.0 && q.y < 0.0).map(|q| q.distance(Vec2::new(20.0, 0.0))).fold(0.0f32, f32::max);
        assert!((reach - 4.0).abs() < 0.01, "{reach}");
        assert!(area(&polys) > 0.0);
    }
}
