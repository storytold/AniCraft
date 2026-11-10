use super::*;

fn rect(x: f32, y: f32, w: f32, h: f32) -> Path {
    Path::rect(x, y, w, h)
}

fn fill(path: &Path, style: u32) -> Item<'_> {
    Item { path, fill: Some(style), fill_rule: FillRule::NonZero, stroke: None }
}

fn stroked(path: &Path, fill: Option<u32>, stroke: u32) -> Item<'_> {
    Item { path, fill, fill_rule: FillRule::NonZero, stroke: Some(stroke) }
}

/// The area a filled path covers (subpaths' signed areas add up; holes wind the other way).
fn area(p: &Path) -> f32 {
    p.flatten(&crate::Affine::IDENTITY, 0.01)
        .iter()
        .map(|pl| {
            let n = pl.points.len();
            (0..n).map(|i| pl.points[i].cross(pl.points[(i + 1) % n])).sum::<f32>() * 0.5
        })
        .sum::<f32>()
        .abs()
}

fn subpaths(p: &Path) -> usize {
    p.cmds.iter().filter(|c| matches!(c, PathCmd::MoveTo(_))).count()
}

fn run(items: &[Item]) -> Merged {
    merge(items).unwrap_or_default()
}

fn near(a: f32, b: f32, tol: f32) -> bool {
    (a - b).abs() <= tol
}

#[test]
fn same_fills_that_overlap_become_one() {
    let (a, b) = (rect(0.0, 0.0, 100.0, 100.0), rect(50.0, 50.0, 100.0, 100.0));
    let m = run(&[fill(&a, 1), fill(&b, 1)]);
    assert_eq!(m.fills.len(), 1, "{m:?}");
    assert!(m.strokes.is_empty());
    assert!(near(area(&m.fills[0].path), 17_500.0, 1.0), "{}", area(&m.fills[0].path));
    assert_eq!(m.fills[0].item, 1);
}

#[test]
fn a_fill_on_top_cuts_the_one_below() {
    let (a, b) = (rect(0.0, 0.0, 100.0, 100.0), rect(50.0, 50.0, 100.0, 100.0));
    let m = run(&[fill(&a, 1), fill(&b, 2)]);
    assert_eq!(m.fills.len(), 2, "{m:?}");
    let red = m.fills.iter().find(|p| p.style == 1).map(|p| area(&p.path)).unwrap_or(0.0);
    let blue = m.fills.iter().find(|p| p.style == 2).map(|p| area(&p.path)).unwrap_or(0.0);
    assert!(near(red, 7_500.0, 1.0), "{red}");
    assert!(near(blue, 10_000.0, 1.0), "{blue}");
}

#[test]
fn a_fill_inside_another_colour_leaves_a_hole() {
    let (a, b) = (rect(0.0, 0.0, 100.0, 100.0), rect(25.0, 25.0, 50.0, 50.0));
    let m = run(&[fill(&a, 1), fill(&b, 2)]);
    let big = m.fills.iter().find(|p| p.style == 1).map(|p| p.path.clone()).unwrap_or_default();
    assert_eq!(subpaths(&big), 2, "outline and hole");
    assert!(near(area(&big), 7_500.0, 1.0), "{}", area(&big));
    // The same colour inside just disappears into it.
    let m = run(&[fill(&a, 1), fill(&b, 1)]);
    assert_eq!(m.fills.len(), 1);
    assert_eq!(subpaths(&m.fills[0].path), 1);
}

#[test]
fn side_by_side_same_fills_are_one_rectangle_with_four_corners() {
    let (a, b) = (rect(0.0, 0.0, 10.0, 10.0), rect(10.0, 0.0, 10.0, 10.0));
    let m = run(&[fill(&a, 1), fill(&b, 1)]);
    assert_eq!(m.fills.len(), 1, "{m:?}");
    let cmds = &m.fills[0].path.cmds;
    assert_eq!(cmds.iter().filter(|c| matches!(c, PathCmd::LineTo(_))).count(), 4, "{cmds:?}");
    assert!(near(area(&m.fills[0].path), 200.0, 0.01));
}

#[test]
fn a_line_across_a_fill_splits_it_and_stays_one_stroke() {
    let a = rect(0.0, 0.0, 100.0, 100.0);
    let l = Path::line(Vec2::new(-20.0, 50.0), Vec2::new(120.0, 50.0));
    let m = run(&[fill(&a, 1), stroked(&l, None, 7)]);
    assert_eq!(m.fills.len(), 2, "{m:?}");
    for f in &m.fills {
        assert!(near(area(&f.path), 5_000.0, 1.0));
    }
    assert_eq!(m.strokes.len(), 1, "{m:?}");
    assert_eq!(m.strokes[0].style, 7);
    // A line that only pokes in does not split it.
    let l = Path::line(Vec2::new(-20.0, 50.0), Vec2::new(50.0, 50.0));
    let m = run(&[fill(&a, 1), stroked(&l, None, 7)]);
    assert_eq!(m.fills.len(), 1);
    assert!(near(area(&m.fills[0].path), 10_000.0, 1.0));
}

#[test]
fn crossing_lines_cut_each_other() {
    let (a, b) = (Path::line(Vec2::new(0.0, 0.0), Vec2::new(100.0, 100.0)), Path::line(Vec2::new(100.0, 0.0), Vec2::new(0.0, 100.0)));
    let m = run(&[stroked(&a, None, 1), stroked(&b, None, 1)]);
    assert!(m.fills.is_empty());
    assert_eq!(m.strokes.len(), 4, "{m:?}");
}

#[test]
fn a_stroked_rectangle_is_a_fill_and_four_sides() {
    let r = rect(0.0, 0.0, 40.0, 30.0);
    let m = run(&[stroked(&r, Some(1), 2)]);
    assert_eq!(m.fills.len(), 1);
    assert_eq!(m.strokes.len(), 4, "corners start new pieces: {m:?}");
    // A smooth outline is one piece, closed.
    let o = Path::ellipse(0.0, 0.0, 30.0, 20.0);
    let m = run(&[stroked(&o, Some(1), 2)]);
    assert_eq!(m.fills.len(), 1);
    assert_eq!(m.strokes.len(), 1, "{m:?}");
    assert!(matches!(m.strokes[0].path.cmds.last(), Some(PathCmd::Close)));
    assert_eq!(m.strokes[0].path.cmds.iter().filter(|c| matches!(c, PathCmd::CubicTo(..))).count(), 4, "the oval's own four curves");
}

#[test]
fn a_fill_on_top_removes_the_stroke_under_it() {
    let l = Path::line(Vec2::new(0.0, 50.0), Vec2::new(100.0, 50.0));
    let r = rect(30.0, 0.0, 40.0, 100.0);
    let m = run(&[stroked(&l, None, 1), fill(&r, 2)]);
    assert_eq!(m.fills.len(), 1);
    assert_eq!(m.strokes.len(), 2, "the line's two outside parts: {m:?}");
    let lens: Vec<f32> = m.strokes.iter().map(|s| s.path.bounds().width()).collect();
    assert!(lens.iter().all(|w| near(*w, 30.0, 0.01)), "{lens:?}");
}

#[test]
fn merging_the_result_again_changes_nothing() {
    let r = rect(0.0, 0.0, 60.0, 60.0);
    let o = Path::ellipse(60.0, 60.0, 30.0, 30.0);
    let first = run(&[stroked(&r, Some(1), 3), stroked(&o, Some(2), 3)]);
    assert!(!first.fills.is_empty() && !first.strokes.is_empty());
    let mut items: Vec<Item> = first.fills.iter().map(|p| fill(&p.path, p.style)).collect();
    items.extend(first.strokes.iter().map(|p| stroked(&p.path, None, p.style)));
    let again = run(&items);
    assert_eq!(again.fills.len(), first.fills.len(), "{again:?}");
    assert_eq!(again.strokes.len(), first.strokes.len(), "{again:?}");
    for (a, b) in first.fills.iter().zip(&again.fills) {
        assert_eq!(a.style, b.style);
        assert!(near(area(&a.path), area(&b.path), 0.5));
    }
}

#[test]
fn touching_ends_and_tiny_gaps_snap_together() {
    // Two lines whose ends are 0.1 apart join (and, meeting at a corner, stay two pieces).
    let (a, b) = (Path::line(Vec2::new(0.0, 0.0), Vec2::new(50.0, 0.0)), Path::line(Vec2::new(50.1, 0.0), Vec2::new(50.1, 50.0)));
    let m = run(&[stroked(&a, None, 1), stroked(&b, None, 1)]);
    assert_eq!(m.strokes.len(), 2);
    let end = |p: &Path| p.anchors().into_iter().map(|(_, v)| v).collect::<Vec<_>>();
    let shared = end(&m.strokes[0].path).iter().any(|p| end(&m.strokes[1].path).iter().any(|q| p.distance(*q) < 1e-4));
    assert!(shared, "{m:?}");
    // In line, they are one.
    let c = Path::line(Vec2::new(50.1, 0.0), Vec2::new(100.0, 0.0));
    let m = run(&[stroked(&a, None, 1), stroked(&c, None, 1)]);
    assert_eq!(m.strokes.len(), 1, "{m:?}");
}

#[test]
fn bad_input_never_panics() {
    let mut p = Path::new();
    p.move_to(Vec2::new(f32::NAN, 0.0)).line_to(Vec2::new(1.0, 1.0)).cubic_to(Vec2::new(f32::INFINITY, 0.0), Vec2::ZERO, Vec2::ZERO).close();
    let empty = Path::new();
    let dot = Path::line(Vec2::ZERO, Vec2::new(0.01, 0.0));
    let _ = run(&[fill(&p, 1), fill(&empty, 1), stroked(&dot, Some(1), 1), stroked(&p, None, 2)]);
    // The same shape twice is the shape once.
    let r = rect(0.0, 0.0, 10.0, 10.0);
    let m = run(&[stroked(&r, Some(1), 2), stroked(&r, Some(1), 2)]);
    assert_eq!((m.fills.len(), m.strokes.len()), (1, 4), "{m:?}");
}

#[test]
fn overlapping_circles_union_to_the_exact_area() {
    // Two radius-50 circles 50 apart: the union is 2πr² minus the lens, r²(2π/3 − √3/2).
    let (a, b) = (Path::ellipse(0.0, 0.0, 50.0, 50.0), Path::ellipse(50.0, 0.0, 50.0, 50.0));
    let m = run(&[fill(&a, 1), fill(&b, 1)]);
    assert_eq!(m.fills.len(), 1, "{m:?}");
    let r2 = 2500.0f32;
    let lens = r2 * (2.0 * std::f32::consts::PI / 3.0 - 3f32.sqrt() / 2.0);
    // Path::ellipse is the usual four-cubic circle, a little over the true one.
    let want = 2.0 * std::f32::consts::PI * r2 - lens;
    let got = area(&m.fills[0].path);
    assert!((got - want).abs() / want < 1e-3, "{got} vs {want}");
    // Stroked, the top circle covers the bottom one's outline inside it (as in Animate): the
    // bottom keeps a crescent, the top stays whole, and its outline is cut where they cross.
    let m = run(&[stroked(&a, Some(1), 2), stroked(&b, Some(1), 2)]);
    assert_eq!((m.fills.len(), m.strokes.len()), (2, 3), "{m:?}");
    let top = m.fills.iter().find(|p| p.item == 1).map(|p| area(&p.path)).unwrap_or(0.0);
    assert!((top - std::f32::consts::PI * r2).abs() < 10.0, "{top}");
}

#[test]
fn a_pile_of_shapes_merges_quickly_and_merges_again_to_the_same() {
    let mut seed = 0x2545_f491_u32;
    let mut rnd = |n: f32| {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed % 10_000) as f32 / 10_000.0 * n
    };
    let mut paths = Vec::new();
    for i in 0..120 {
        let (x, y, w, h) = (rnd(800.0), rnd(600.0), 5.0 + rnd(150.0), 5.0 + rnd(150.0));
        paths.push(match i % 3 {
            0 => Path::ellipse(x, y, w * 0.5, h * 0.5),
            1 => rect(x, y, w, h),
            _ => Path::line(Vec2::new(x, y), Vec2::new(x + w, y + h)),
        });
    }
    let items: Vec<Item> = paths.iter().enumerate().map(|(i, p)| stroked(p, (i % 3 != 2).then_some((i % 4) as u32), (i % 2) as u32)).collect();
    let t = std::time::Instant::now();
    let first = merge(&items).unwrap_or_default();
    assert!(t.elapsed().as_secs_f32() < 5.0, "took {:?}", t.elapsed());
    assert!(!first.fills.is_empty() && !first.strokes.is_empty());
    for p in first.fills.iter().chain(&first.strokes) {
        assert!(p.path.cmds.iter().all(|c| match c {
            PathCmd::MoveTo(v) | PathCmd::LineTo(v) => v.is_finite(),
            PathCmd::CubicTo(a, b, v) => a.is_finite() && b.is_finite() && v.is_finite(),
            _ => true,
        }));
    }
    let mut again_items: Vec<Item> = first.fills.iter().map(|p| fill(&p.path, p.style)).collect();
    again_items.extend(first.strokes.iter().map(|p| stroked(&p.path, None, p.style)));
    let again = merge(&again_items).unwrap_or_default();
    let total = |m: &Merged| m.fills.iter().map(|p| area(&p.path)).sum::<f32>();
    assert!((total(&first) - total(&again)).abs() / total(&first) < 1e-3, "{} vs {}", total(&first), total(&again));
    assert_eq!(again.fills.len(), first.fills.len());
    assert_eq!(again.strokes.len(), first.strokes.len());
}

#[test]
fn a_search_that_runs_out_of_steps_declines_the_merge() {
    // Two long arcs 0.3 apart (just outside SNAP) keep their boxes overlapping down to the
    // finest subdivision; when the crossing search runs out of steps, nothing is cut wrongly:
    // the merge declines (the engine then keeps the new shape whole).
    let arc = |r: f32| {
        let mut p = Path::new();
        let k = 0.552_284_8 * r;
        p.move_to(Vec2::new(r, 0.0)).cubic_to(Vec2::new(r, k), Vec2::new(k, r), Vec2::new(0.0, r));
        p
    };
    let (a, b) = (arc(5000.0), arc(5000.3));
    let seg = |p: &Path| collect(&[stroked(p, None, 1)]).unwrap_or_default().into_iter().next();
    let (Some(sa), Some(sb)) = (seg(&a), seg(&b)) else { panic!("no segments") };
    assert!(meet(&sa, &sb).is_none(), "the search should run out on these");
    assert!(merge(&[stroked(&a, None, 1), stroked(&b, None, 1)]).is_none());
    // Shorter arcs are searched to the end and merge (they don't cross).
    let (a, b) = (arc(100.0), arc(100.3));
    assert_eq!(merge(&[stroked(&a, None, 1), stroked(&b, None, 1)]).map(|m| m.strokes.len()), Some(2));
}
