//! Analytic tests of caps, joins, the miter limit and dashes (ADR 0046).

use super::*;

fn area(polys: &[Vec<Vec2>]) -> f32 {
    polys.iter().map(|p| signed_area(p)).sum()
}

fn styled(points: &[(f32, f32)], closed: bool, hw: f32, o: &Outline) -> Vec<Vec<Vec2>> {
    let pl = Polyline { points: points.iter().map(|&(x, y)| Vec2::new(x, y)).collect(), closed };
    let polys = styled_polygons(&[pl], hw, o, None, None, false);
    assert!(polys.iter().all(|p| signed_area(p) >= 0.0 && p.iter().all(|q| q.is_finite())), "positive and finite");
    polys
}

fn outline(cap: Cap, join: Join) -> Outline<'static> {
    Outline { cap, join, ..Outline::ROUND }
}

/// Whether `q` is inside the union (non-zero winding over every polygon).
fn covered(polys: &[Vec<Vec2>], q: Vec2) -> bool {
    let mut wn = 0i32;
    for p in polys {
        for (i, &a) in p.iter().enumerate() {
            let b = p[(i + 1) % p.len()];
            let side = (b - a).cross(q - a);
            if a.y <= q.y && b.y > q.y && side > 0.0 {
                wn += 1;
            } else if a.y > q.y && b.y <= q.y && side < 0.0 {
                wn -= 1;
            }
        }
    }
    wn != 0
}

/// The union's area by sampling a grid `step` apart over `[x0, x1] × [y0, y1]`.
fn union_area(polys: &[Vec<Vec2>], (x0, y0, x1, y1): (f32, f32, f32, f32), step: f32) -> f32 {
    let mut n = 0usize;
    let (nx, ny) = (((x1 - x0) / step) as usize, ((y1 - y0) / step) as usize);
    for j in 0..ny {
        for i in 0..nx {
            if covered(polys, Vec2::new(x0 + (i as f32 + 0.5) * step, y0 + (j as f32 + 0.5) * step)) {
                n += 1;
            }
        }
    }
    n as f32 * step * step
}

#[test]
fn butt_square_and_round_caps_differ_by_their_ends() {
    let line = [(0.0, 0.0), (100.0, 0.0)];
    // A butt-capped line is exactly its rectangle; square caps add half a width at each end.
    let butt = styled(&line, false, 5.0, &outline(Cap::Butt, Join::Miter));
    assert_eq!(butt.len(), 1);
    assert!((area(&butt) - 1000.0).abs() < 1e-3, "{}", area(&butt));
    let square = styled(&line, false, 5.0, &outline(Cap::Square, Join::Miter));
    assert!((area(&square) - 1100.0).abs() < 1e-3, "{}", area(&square));
    let xs = |p: &[Vec<Vec2>]| p.iter().flatten().fold((f32::MAX, f32::MIN), |(lo, hi), q| (lo.min(q.x), hi.max(q.x)));
    assert_eq!(xs(&butt), (0.0, 100.0));
    assert_eq!(xs(&square), (-5.0, 105.0));
    // Round: the rectangle plus a disc (half at each end), within the arc tolerance.
    let round = styled(&line, false, 5.0, &outline(Cap::Round, Join::Bevel));
    let want = 1000.0 + std::f32::consts::PI * 25.0;
    assert!((area(&round) - want).abs() < 0.01 * want, "{} vs {want}", area(&round));
}

#[test]
fn miter_bevel_and_round_corners() {
    // Right 50 then down 50 (y-down): the outer corner is up and to the right of (50, 0).
    let l = [(0.0, 0.0), (50.0, 0.0), (50.0, 50.0)];
    let corner = |join| {
        let polys = styled(&l, false, 5.0, &outline(Cap::Butt, join));
        // The corner piece alone: everything but the two segment rectangles.
        area(&polys) - 2.0 * 500.0
    };
    assert!((corner(Join::Miter) - 25.0).abs() < 1e-3, "a 5 × 5 square: {}", corner(Join::Miter));
    assert!((corner(Join::Bevel) - 12.5).abs() < 1e-3, "half of it: {}", corner(Join::Bevel));
    let round = corner(Join::Round);
    assert!((round - std::f32::consts::PI * 25.0 / 4.0).abs() < 0.6, "a quarter disc, within the arc tolerance: {round}");
    // The miter reaches the corner (55, -5); the bevel stops at the diagonal.
    let miter = styled(&l, false, 5.0, &outline(Cap::Butt, Join::Miter));
    let bevel = styled(&l, false, 5.0, &outline(Cap::Butt, Join::Bevel));
    assert!(covered(&miter, Vec2::new(54.5, -4.5)) && !covered(&bevel, Vec2::new(54.5, -4.5)));
    assert!(covered(&bevel, Vec2::new(52.0, -2.0)));
    // A closed square with miters is a full frame: the outer square less the inner one.
    let sq = styled(&[(0.0, 0.0), (40.0, 0.0), (40.0, 40.0), (0.0, 40.0)], true, 2.0, &outline(Cap::Round, Join::Miter));
    let a = union_area(&sq, (-4.0, -4.0, 44.0, 44.0), 0.25);
    assert!((a - (44.0 * 44.0 - 36.0 * 36.0)).abs() < 4.0, "{a}");
    assert!(covered(&sq, Vec2::new(-1.9, -1.9)), "sharp outer corner, also where the path closes");
}

#[test]
fn the_miter_limit_bevels_sharp_corners() {
    // A 160° turn (a 20° corner): the miter is 1 / sin(10°) ≈ 5.76 half-widths long.
    let back = 20f32.to_radians();
    let l = [(0.0, 0.0), (50.0, 0.0), (50.0 - 30.0 * back.cos(), 30.0 * back.sin())];
    let ratio = 1.0 / 10f32.to_radians().sin();
    let reach = |limit: f32| {
        let o = Outline { miter_limit: limit, ..outline(Cap::Butt, Join::Miter) };
        styled(&l, false, 2.0, &o).iter().flatten().map(|q| q.distance(Vec2::new(50.0, 0.0))).filter(|d| *d < 20.0).fold(0.0f32, f32::max)
    };
    assert!((reach(6.0) - ratio * 2.0).abs() < 1e-3, "under the limit: a point {} px out", reach(6.0));
    assert!(reach(5.5) < 2.0 + 1e-3, "over the limit: bevelled, {} px out", reach(5.5));
    // Animate's default (3) and SVG's (4) both bevel it.
    assert!(reach(3.0) < 2.0 + 1e-3 && reach(4.0) < 2.0 + 1e-3);
}

#[test]
fn dashes_follow_curves_by_arc_length() {
    // A circle of radius 50 flattened finely, dashed 10 on, 10 off from its start.
    let r = 50.0f32;
    let ring: Vec<(f32, f32)> = (0..720)
        .map(|i| {
            let a = i as f32 / 720.0 * std::f32::consts::TAU;
            (r * a.cos(), r * a.sin())
        })
        .collect();
    let pattern = [10.0, 10.0];
    let o = Outline { dash: Some((&pattern, 0.0)), ..outline(Cap::Butt, Join::Round) };
    let polys = styled(&ring, true, 1.5, &o);
    let on_ring = |s: f32| Vec2::new(r * (s / r).cos(), r * (s / r).sin());
    for k in 0..15 {
        let s = k as f32 * 20.0;
        assert!(covered(&polys, on_ring(s + 5.0)), "dash {k} is drawn at arc length {}", s + 5.0);
        assert!(!covered(&polys, on_ring(s + 15.0)), "gap {k} is empty at arc length {}", s + 15.0);
        assert!(covered(&polys, on_ring(s + 0.3)) && !covered(&polys, on_ring(s + 10.3)), "dash {k} runs from {s} to {}", s + 10.0);
    }
    // The offset moves the pattern along: 5 in, the first dash is half gone.
    let o = Outline { dash: Some((&pattern, 5.0)), ..o };
    let shifted = styled(&ring, true, 1.5, &o);
    assert!(covered(&shifted, on_ring(2.0)) && !covered(&shifted, on_ring(7.0)) && covered(&shifted, on_ring(17.0)));
    // Butt-capped dashes: about the on share of the ring's area (π((r+w)² − (r−w)²)).
    let ring_area = std::f32::consts::PI * ((r + 1.5).powi(2) - (r - 1.5).powi(2));
    assert!((area(&polys) - ring_area * 0.5).abs() < 0.03 * ring_area, "{} vs {}", area(&polys), ring_area * 0.5);
}

#[test]
fn dash_intervals_walk_the_pattern() {
    let mut budget = 100;
    assert_eq!(dash_intervals(&[3.0, 2.0], 0.0, 12.0, &mut budget), vec![(0.0, 3.0), (5.0, 8.0), (10.0, 12.0)]);
    assert_eq!(dash_intervals(&[3.0, 2.0], 4.0, 6.0, &mut budget), vec![(1.0, 4.0)]);
    assert_eq!(dash_intervals(&[3.0, 2.0], -1.0, 6.0, &mut budget), vec![(1.0, 4.0)], "a negative offset counts back from the end of the period");
    // Dots: zero-length dashes, the last one on the very end.
    assert_eq!(dash_intervals(&[0.0, 5.0], 0.0, 10.0, &mut budget), vec![(0.0, 0.0), (5.0, 5.0), (10.0, 10.0)]);
    // The budget bounds the walk; the rest is one stretch.
    let mut small = 2;
    assert_eq!(dash_intervals(&[1.0, 1.0], 0.0, 1e9, &mut small), vec![(0.0, 1.0), (2.0, 3.0), (4.0, 1e9)]);
    assert!(dash_intervals(&[1.0, 1.0], f32::NAN, f32::INFINITY, &mut budget).is_empty());
    assert!(dash_intervals(&[0.0, 0.0], 0.0, 5.0, &mut budget).is_empty());
}

#[test]
fn dashes_keep_the_width_profile_of_their_place() {
    let line = Polyline { points: vec![Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)], closed: false };
    let taper = anicraft_doc::ProfilePreset::TaperBoth.profile().unwrap();
    let pattern = [10.0, 10.0];
    let o = Outline { dash: Some((&pattern, 0.0)), ..outline(Cap::Butt, Join::Round) };
    let polys = styled_polygons(std::slice::from_ref(&line), 5.0, &o, Some(&taper), None, false);
    assert!(polys.iter().all(|p| signed_area(p) >= 0.0));
    // In the dash from 40 to 50 the half-width is the profile's there; 50..60 is a gap.
    for x in [42.0f32, 45.0, 48.0] {
        let want = taper.at(x / 100.0).0 * 5.0;
        assert!(covered(&polys, Vec2::new(x, -(want - 0.1))) && !covered(&polys, Vec2::new(x, -(want + 0.1))), "half-width {want} at {x}");
    }
    assert!(!covered(&polys, Vec2::new(55.0, 0.0)));
}

#[test]
fn dots_are_discs_one_width_across() {
    let pattern = [0.0, 10.0];
    let o = Outline { dash: Some((&pattern, 0.0)), ..outline(Cap::Round, Join::Round) };
    let line = [(0.0, 0.0), (100.0, 0.0)];
    let polys = styled(&line, false, 2.0, &o);
    // Eleven dots (0, 10, … 100), each two half discs of radius 2.
    assert_eq!(polys.len(), 22);
    let want = 11.0 * std::f32::consts::PI * 4.0;
    assert!((area(&polys) - want).abs() < 0.08 * want, "within the arc tolerance: {} vs {want}", area(&polys));
    assert!(covered(&polys, Vec2::new(30.0, 1.5)) && !covered(&polys, Vec2::new(35.0, 0.0)));
    // Butt-capped dots draw nothing; square ones are squares.
    let o = Outline { cap: Cap::Butt, ..o };
    assert!(styled(&line, false, 2.0, &o).is_empty());
    let o = Outline { cap: Cap::Square, ..o };
    assert!((area(&styled(&line, false, 2.0, &o)) - 11.0 * 16.0).abs() < 1e-2);
}

#[test]
fn round_solid_strokes_take_the_old_paths_exactly() {
    let pl = Polyline { points: vec![Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0)], closed: false };
    assert_eq!(styled_polygons(std::slice::from_ref(&pl), 2.0, &Outline::ROUND, None, None, false), stroke_polygons(std::slice::from_ref(&pl), 2.0));
    let taper = anicraft_doc::ProfilePreset::TaperEnd.profile().unwrap();
    assert_eq!(styled_polygons(std::slice::from_ref(&pl), 2.0, &Outline::ROUND, Some(&taper), None, false), variable_polygons(&[pl], 2.0, &taper, None, false));
}

#[test]
fn styled_degenerate_inputs_are_safe() {
    let pattern = [1.0, 1.0];
    for o in [outline(Cap::Square, Join::Miter), Outline { dash: Some((&pattern, 0.0)), ..outline(Cap::Butt, Join::Bevel) }] {
        assert!(styled_polygons(&[], 1.0, &o, None, None, false).is_empty());
        for pts in [vec![Vec2::ZERO; 3], vec![Vec2::ZERO, Vec2::new(f32::NAN, 1.0), Vec2::new(5.0, 0.0), Vec2::new(5.0, 0.0), Vec2::ZERO], vec![Vec2::ZERO, Vec2::new(1e30, 0.0), Vec2::new(-1e30, 0.0)]] {
            for closed in [false, true] {
                let polys = styled_polygons(&[Polyline { points: pts.clone(), closed }], 1.0, &o, None, None, false);
                assert!(polys.iter().flatten().all(|q| q.is_finite()));
            }
        }
        assert!(styled_polygons(&[Polyline { points: vec![Vec2::ZERO, Vec2::new(5.0, 0.0)], closed: false }], f32::NAN, &o, None, None, false).is_empty());
    }
    // A U-turn with a miter join: bevelled, never a point at infinity.
    let polys = styled(&[(0.0, 0.0), (10.0, 0.0), (0.0, 0.0)], false, 1.0, &outline(Cap::Butt, Join::Miter));
    assert!(polys.iter().flatten().all(|q| q.length() < 20.0));
}

#[test]
fn fine_patterns_draw_solid_and_lighter_on_raster() {
    // A 0.2-unit pattern at 1:1 is finer than a pixel: solid, at the share that is on.
    let st = anicraft_doc::Stroke::new(anicraft_doc::Rgba::BLACK, 4.0).with_style(anicraft_doc::StrokeStyle { cap: Cap::Butt, dash: anicraft_doc::Dash::dashed(0.05, 0.15), ..Default::default() });
    let lines = [Polyline { points: vec![Vec2::ZERO, Vec2::new(50.0, 0.0)], closed: false }];
    let (polys, alpha) = stroke_geometry(&lines, &st, &Affine::IDENTITY, true).unwrap();
    assert_eq!(polys.len(), 1);
    assert!((alpha - 0.25).abs() < 1e-4, "{alpha}");
    // Vector outlines keep every dash.
    let (exact, a) = stroke_geometry(&lines, &st, &Affine::IDENTITY, false).unwrap();
    assert_eq!((exact.len(), a), (250, 1.0));
    // Zoomed in 10×, the pattern is two pixels: real dashes, and their lengths scale.
    let (zoomed, a) = stroke_geometry(&[Polyline { points: vec![Vec2::ZERO, Vec2::new(500.0, 0.0)], closed: false }], &st, &Affine::scale(10.0, 10.0), true).unwrap();
    assert_eq!((zoomed.len(), a), (250, 1.0));
}
