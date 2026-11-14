//! Merge drawing (ADR 0031): what a layer's raw shapes become when they are drawn over each other.
//!
//! The shapes, bottom to top, are cut into a planar map:
//! - every segment is cut where it meets another (exact sub-curves by de Casteljau);
//! - points closer than [`SNAP`] become one vertex (anchors meet at their midpoint; a point that
//!   lands on a line goes onto it, and the line stays), and pieces that run along each other
//!   become one edge;
//! - each face of the map takes the fill of the topmost shape covering it (winding at a point
//!   inside the face).
//!
//! What comes out:
//! - faces of one fill that no visible stroke separates are one fill piece (the edges between
//!   them go);
//! - an edge shows the stroke of the topmost shape stroking it, unless a later shape's fill
//!   covers both of its sides;
//! - visible strokes are chained through points where exactly two of one style meet smoothly,
//!   so a crossing or a corner starts a new piece;
//! - consecutive pieces of one original segment are joined back into one exact sub-curve, and
//!   straight runs into one line.
//!
//! Styles are the caller's numbers: equal numbers are equal styles. Arithmetic is `f64` with no
//! platform maths (no `atan2`, `hypot` or fused ops), so results are the same everywhere.

use crate::{FillRule, Path, PathCmd, Vec2};
use std::collections::HashMap;

/// Points this close (stage units) are one point, and edges this close one edge.
pub const SNAP: f32 = 0.2;
const TOL: f64 = SNAP as f64;
/// Subdivision stops when a curve's control points are this close to its chord.
const FLAT: f64 = 1e-3;
/// Flattening tolerance for areas, containment and interior points.
const FLAT_FILL: f64 = 0.01;
/// Most segments merged at once; beyond it [`merge`] declines.
pub const MAX_SEGMENTS: usize = 20_000;
/// Most candidate pairs looked at, and subdivision steps per pair.
const MAX_PAIRS: usize = 4_000_000;
const PAIR_BUDGET: u32 = 4096;
/// Strokes meeting at a sharper turn than this (cos 20°) are separate pieces.
const SMOOTH_COS: f64 = 0.94;

/// A shape going in: its path (in the common space), fill and stroke style numbers.
pub struct Item<'a> {
    pub path: &'a Path,
    pub fill: Option<u32>,
    pub fill_rule: FillRule,
    pub stroke: Option<u32>,
}

/// A shape coming out: one fill region or one stroke run, and the topmost item it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    pub style: u32,
    pub path: Path,
    pub item: usize,
    /// Inside the cutter ([`merge_cut`]); always false from [`merge`].
    pub inside: bool,
    /// For a stroke piece: the stretches of its items' paths it runs along, in order, each as
    /// `[from, to]` in path positions (see [`Piece::position`]); `to < from` where it runs
    /// backwards. What a width profile needs to follow its stroke into each piece (ADR 0093).
    /// Empty for fills.
    pub spans: Vec<[f64; 2]>,
}

impl Piece {
    /// A place on an item's path as a path position: `k + t` is parameter `t` along the segment
    /// that command `k` draws (a `Close` draws the closing line).
    pub fn position(cmd: usize, t: f64) -> f64 {
        cmd as f64 + t.clamp(0.0, 1.0)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Merged {
    pub fills: Vec<Piece>,
    pub strokes: Vec<Piece>,
}

/// Merges `items` (bottom to top). `None` when there is too much to merge at once.
pub fn merge(items: &[Item]) -> Option<Merged> {
    merge_with(items, None)
}

/// [`merge`], also cutting everything along `cutter` (a closed outline that only cuts, such as
/// the Selection tool's marquee, ADR 0041): fills are split at its edges and stroke runs end
/// there, nothing of the cutter itself comes out, and each piece says whether it lies inside it.
pub fn merge_cut(items: &[Item], cutter: &Path) -> Option<Merged> {
    merge_with(items, Some(cutter))
}

/// The area `paths` cover together (each filled by its own rule), as one path of loops that
/// don't overlap: even-odd and non-zero fill it alike. Empty when they cover nothing; `None`
/// when there is too much to merge at once.
pub fn union(paths: &[(&Path, FillRule)]) -> Option<Path> {
    let items: Vec<Item> = paths.iter().map(|&(path, fill_rule)| Item { path, fill: Some(0), fill_rule, stroke: None }).collect();
    let merged = merge(&items)?;
    Some(join(merged.fills.into_iter().map(|p| p.path)))
}

/// The part of `subject` (filled by `rule`) inside `region` (filled even-odd, such as a
/// [`union`]): a boolean intersection, exact to [`SNAP`], as one path of loops that don't
/// overlap (ADR 0080: the Brush's Paint Selection and Paint Inside). Empty when they don't
/// overlap; `None` when there is too much to merge at once.
pub fn intersect(subject: &Path, rule: FillRule, region: &Path) -> Option<Path> {
    let merged = merge_cut(&[Item { path: subject, fill: Some(0), fill_rule: rule, stroke: None }], region)?;
    Some(join(merged.fills.into_iter().filter(|p| p.inside).map(|p| p.path)))
}

/// Pieces' paths, one after another.
fn join(pieces: impl Iterator<Item = Path>) -> Path {
    let mut out = Path::new();
    for p in pieces {
        out.cmds.extend(p.cmds);
    }
    out
}

/// The item number the cutter's segments carry.
const CUTTER: usize = usize::MAX;

fn merge_with(items: &[Item], cutter: Option<&Path>) -> Option<Merged> {
    let mut segs = collect(items)?;
    if let Some(path) = cutter {
        let edge = Item { path, fill: None, fill_rule: FillRule::EvenOdd, stroke: Some(0) };
        segs.extend(collect(&[edge])?.into_iter().map(|s| Seg { item: CUTTER, ..s }));
        if segs.len() > MAX_SEGMENTS {
            return None;
        }
    }
    let splits = cut(&segs)?;
    let map = Map::build(&segs, &splits);
    Some(map.output(items, &segs))
}

// ----- points and curves ------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct P {
    x: f64,
    y: f64,
}

impl P {
    fn new(x: f64, y: f64) -> P {
        P { x, y }
    }
    fn of(v: Vec2) -> P {
        P::new(f64::from(v.x), f64::from(v.y))
    }
    fn vec2(self) -> Vec2 {
        Vec2::new(self.x as f32, self.y as f32)
    }
    fn add(self, o: P) -> P {
        P::new(self.x + o.x, self.y + o.y)
    }
    fn sub(self, o: P) -> P {
        P::new(self.x - o.x, self.y - o.y)
    }
    fn mul(self, s: f64) -> P {
        P::new(self.x * s, self.y * s)
    }
    fn lerp(self, o: P, t: f64) -> P {
        self.add(o.sub(self).mul(t))
    }
    fn dot(self, o: P) -> f64 {
        self.x * o.x + self.y * o.y
    }
    fn cross(self, o: P) -> f64 {
        self.x * o.y - self.y * o.x
    }
    fn len(self) -> f64 {
        self.dot(self).sqrt()
    }
    fn dist(self, o: P) -> f64 {
        self.sub(o).len()
    }
    fn finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

type Cubic = [P; 4];

fn line_cubic(a: P, b: P) -> Cubic {
    [a, a.lerp(b, 1.0 / 3.0), a.lerp(b, 2.0 / 3.0), b]
}

fn eval(c: &Cubic, t: f64) -> P {
    let (a, b, d) = (c[0].lerp(c[1], t), c[1].lerp(c[2], t), c[2].lerp(c[3], t));
    let (e, f) = (a.lerp(b, t), b.lerp(d, t));
    e.lerp(f, t)
}

fn split(c: &Cubic, t: f64) -> (Cubic, Cubic) {
    let (a, b, d) = (c[0].lerp(c[1], t), c[1].lerp(c[2], t), c[2].lerp(c[3], t));
    let (e, f) = (a.lerp(b, t), b.lerp(d, t));
    let m = e.lerp(f, t);
    ([c[0], a, e, m], [m, f, d, c[3]])
}

fn reversed(c: &Cubic) -> Cubic {
    [c[3], c[2], c[1], c[0]]
}

/// The part of `c` from `t0` to `t1` (reversed when `t1 < t0`).
fn sub(c: &Cubic, t0: f64, t1: f64) -> Cubic {
    if t1 < t0 {
        return reversed(&sub(c, t1, t0));
    }
    let left = if t1 >= 1.0 { *c } else { split(c, t1).0 };
    if t0 <= 0.0 || t1 <= 0.0 {
        return left;
    }
    split(&left, (t0 / t1).clamp(0.0, 1.0)).1
}

fn bbox(c: &Cubic) -> (P, P) {
    let mut lo = c[0];
    let mut hi = c[0];
    for p in &c[1..] {
        lo = P::new(lo.x.min(p.x), lo.y.min(p.y));
        hi = P::new(hi.x.max(p.x), hi.y.max(p.y));
    }
    (lo, hi)
}

fn boxes_meet(a: (P, P), b: (P, P), pad: f64) -> bool {
    a.0.x <= b.1.x + pad && b.0.x <= a.1.x + pad && a.0.y <= b.1.y + pad && b.0.y <= a.1.y + pad
}

/// Farthest a control point is from the chord.
fn flatness(c: &Cubic) -> f64 {
    let d = c[3].sub(c[0]);
    let l = d.len();
    if l < 1e-12 {
        return c[1].dist(c[0]).max(c[2].dist(c[0]));
    }
    (d.cross(c[1].sub(c[0])).abs() / l).max(d.cross(c[2].sub(c[0])).abs() / l)
}

fn hull_len(c: &Cubic) -> f64 {
    c[0].dist(c[1]) + c[1].dist(c[2]) + c[2].dist(c[3])
}

/// Points along `c` after its start, within [`FLAT_FILL`] of the curve.
fn flatten_into(c: &Cubic, line: bool, out: &mut Vec<P>) {
    if line {
        out.push(c[3]);
        return;
    }
    let dd = c[0].sub(c[1].mul(2.0)).add(c[2]).len().max(c[1].sub(c[2].mul(2.0)).add(c[3]).len());
    let n = (0.75 * dd / FLAT_FILL).sqrt().ceil();
    let n = if n.is_finite() && n >= 1.0 { (n as usize).min(256) } else { 1 };
    for i in 1..=n {
        out.push(eval(c, i as f64 / n as f64));
    }
}

/// The parameter on `c` nearest `p`, and the distance.
fn nearest(c: &Cubic, line: bool, p: P) -> (f64, f64) {
    if line {
        let d = c[3].sub(c[0]);
        let l2 = d.dot(d);
        let t = if l2 > 1e-24 { (p.sub(c[0]).dot(d) / l2).clamp(0.0, 1.0) } else { 0.0 };
        return (t, eval(c, t).dist(p));
    }
    const N: usize = 32;
    let mut best = (0.0, f64::INFINITY);
    for i in 0..=N {
        let t = i as f64 / N as f64;
        let d = eval(c, t).dist(p);
        if d < best.1 {
            best = (t, d);
        }
    }
    // Golden-section search around the best sample.
    let (mut lo, mut hi) = ((best.0 - 1.0 / N as f64).max(0.0), (best.0 + 1.0 / N as f64).min(1.0));
    const G: f64 = 0.618_033_988_749_894_8;
    for _ in 0..40 {
        let (m1, m2) = (hi - (hi - lo) * G, lo + (hi - lo) * G);
        if eval(c, m1).dist(p) < eval(c, m2).dist(p) {
            hi = m2;
        } else {
            lo = m1;
        }
    }
    let t = 0.5 * (lo + hi);
    let d = eval(c, t).dist(p);
    if d < best.1 { (t, d) } else { best }
}

/// Where segments `a0→a1` and `b0→b1` cross, as fractions along each.
fn seg_seg(a0: P, a1: P, b0: P, b1: P) -> Option<(f64, f64)> {
    let (r, s) = (a1.sub(a0), b1.sub(b0));
    let d = r.cross(s);
    if d.abs() <= 1e-12 * (r.len() * s.len()).max(1e-300) {
        return None;
    }
    let q = b0.sub(a0);
    let (t, u) = (q.cross(s) / d, q.cross(r) / d);
    const E: f64 = 1e-9;
    ((-E..=1.0 + E).contains(&t) && (-E..=1.0 + E).contains(&u)).then(|| (t.clamp(0.0, 1.0), u.clamp(0.0, 1.0)))
}

// ----- the segments going in --------------------------------------------------------------------

struct Seg {
    c: Cubic,
    line: bool,
    item: usize,
    stroke: bool,
    /// Its ends as path positions on its item ([`Piece::position`]).
    pos: [f64; 2],
}

impl Seg {
    /// Path position of parameter `t`.
    fn at(&self, t: f64) -> f64 {
        self.pos[0] + (self.pos[1] - self.pos[0]) * t
    }
}

/// Every segment of every item. A filled subpath left open gets its closing edge (unstroked).
fn collect(items: &[Item]) -> Option<Vec<Seg>> {
    let mut segs = Vec::new();
    for (item, it) in items.iter().enumerate() {
        if it.fill.is_none() && it.stroke.is_none() {
            continue;
        }
        let stroke = it.stroke.is_some();
        let mut push = |c: Cubic, line: bool, stroke: bool, k: usize| {
            if !c.iter().all(|p| p.finite()) || hull_len(&c) <= 1e-9 {
                return;
            }
            let k = k as f64;
            // A segment that comes back to where it began (a loop) is two, so no edge is a loop.
            if c[0].dist(c[3]) < TOL && hull_len(&c) > 2.0 * TOL {
                let (a, b) = split(&c, 0.5);
                segs.push(Seg { c: a, line, item, stroke, pos: [k, k + 0.5] });
                segs.push(Seg { c: b, line, item, stroke, pos: [k + 0.5, k + 1.0] });
            } else {
                segs.push(Seg { c, line, item, stroke, pos: [k, k + 1.0] });
            }
        };
        let (mut start, mut last) = (P::default(), P::default());
        let (mut open, mut drawn) = (false, false);
        let close = |push: &mut dyn FnMut(Cubic, bool, bool, usize), last: P, start: P, stroked: bool, k: usize| {
            if last.dist(start) > 0.0 {
                push(line_cubic(last, start), true, stroked, k);
            }
        };
        for (k, cmd) in it.path.cmds.iter().enumerate() {
            let to = match *cmd {
                PathCmd::MoveTo(p) => {
                    if open && drawn && it.fill.is_some() {
                        close(&mut push, last, start, false, k);
                    }
                    start = P::of(p);
                    last = start;
                    (open, drawn) = (true, false);
                    continue;
                }
                PathCmd::Close => {
                    if open {
                        close(&mut push, last, start, stroke, k);
                    }
                    last = start;
                    (open, drawn) = (false, false);
                    continue;
                }
                PathCmd::LineTo(p) => (line_cubic(last, P::of(p)), true),
                PathCmd::QuadTo(q, p) => {
                    let (q, p) = (P::of(q), P::of(p));
                    ([last, last.lerp(q, 2.0 / 3.0), p.lerp(q, 2.0 / 3.0), p], false)
                }
                PathCmd::CubicTo(a, b, p) => ([last, P::of(a), P::of(b), P::of(p)], false),
            };
            push(to.0, to.1, stroke, k);
            last = to.0[3];
            (open, drawn) = (true, true);
        }
        if open && drawn && it.fill.is_some() {
            close(&mut push, last, start, false, it.path.cmds.len());
        }
        if segs.len() > MAX_SEGMENTS {
            return None;
        }
    }
    Some(segs)
}

/// Where each segment is cut: its parameters, sorted, with 0 and 1.
fn cut(segs: &[Seg]) -> Option<Vec<Vec<f64>>> {
    let mut splits: Vec<Vec<f64>> = vec![vec![0.0, 1.0]; segs.len()];
    let boxes: Vec<(P, P)> = segs.iter().map(|s| bbox(&s.c)).collect();
    let mut order: Vec<usize> = (0..segs.len()).collect();
    order.sort_by(|&a, &b| boxes[a].0.x.total_cmp(&boxes[b].0.x));
    let mut pairs = 0usize;
    for (k, &i) in order.iter().enumerate() {
        for &j in order.get(k + 1..).unwrap_or(&[]) {
            if boxes[j].0.x > boxes[i].1.x + TOL {
                break;
            }
            if !boxes_meet(boxes[i], boxes[j], TOL) {
                continue;
            }
            pairs += 1;
            if pairs > MAX_PAIRS {
                return None;
            }
            for (ta, tb) in meet(&segs[i], &segs[j])? {
                splits[i].push(ta);
                splits[j].push(tb);
            }
        }
    }
    for s in &mut splits {
        s.sort_by(f64::total_cmp);
        s.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
    }
    Some(splits)
}

/// Where two segments meet: crossings, ends lying on the other, and the ends of a stretch where
/// they run along each other. `None` when the search ran out of steps (a crossing may be missed,
/// so the merge declines rather than leave it uncut).
fn meet(a: &Seg, b: &Seg) -> Option<Vec<(f64, f64)>> {
    let mut ends = Vec::new();
    for (t, p) in [(0.0, a.c[0]), (1.0, a.c[3])] {
        let (u, d) = nearest(&b.c, b.line, p);
        if d < TOL {
            ends.push((t, u));
        }
    }
    for (u, p) in [(0.0, b.c[0]), (1.0, b.c[3])] {
        let (t, d) = nearest(&a.c, a.line, p);
        if d < TOL {
            ends.push((t, u));
        }
    }
    if ends.len() >= 2 && coincide(a, b, &ends) {
        return Some(ends);
    }
    let mut hits = Vec::new();
    let mut budget = PAIR_BUDGET;
    cross(&a.c, (0.0, 1.0), &b.c, (0.0, 1.0), 0, &mut budget, &mut hits);
    if budget == 0 {
        return None;
    }
    let mut out: Vec<(f64, f64)> = Vec::new();
    for h in ends.into_iter().chain(hits) {
        let p = eval(&a.c, h.0);
        if !out.iter().any(|o| eval(&a.c, o.0).dist(p) < TOL) {
            out.push(h);
        }
    }
    Some(out)
}

/// Whether `a` runs along `b` between the extreme ends that touch it.
fn coincide(a: &Seg, b: &Seg, ends: &[(f64, f64)]) -> bool {
    let lo = ends.iter().map(|e| e.0).fold(f64::INFINITY, f64::min);
    let hi = ends.iter().map(|e| e.0).fold(f64::NEG_INFINITY, f64::max);
    if eval(&a.c, lo).dist(eval(&a.c, hi)) < TOL {
        return false;
    }
    (1..=5).all(|k| nearest(&b.c, b.line, eval(&a.c, lo + (hi - lo) * f64::from(k) / 6.0)).1 < TOL)
}

/// Crossings by recursive subdivision (bounding boxes, then chords once both are flat).
fn cross(a: &Cubic, ra: (f64, f64), b: &Cubic, rb: (f64, f64), depth: u32, budget: &mut u32, out: &mut Vec<(f64, f64)>) {
    if *budget == 0 || !boxes_meet(bbox(a), bbox(b), FLAT) {
        return;
    }
    *budget -= 1;
    let (fa, fb) = (flatness(a) <= FLAT, flatness(b) <= FLAT);
    if (fa && fb) || depth >= 48 {
        if let Some((s, t)) = seg_seg(a[0], a[3], b[0], b[3]) {
            out.push((ra.0 + s * (ra.1 - ra.0), rb.0 + t * (rb.1 - rb.0)));
        }
        return;
    }
    let size = |c: &Cubic| {
        let (lo, hi) = bbox(c);
        (hi.x - lo.x).max(hi.y - lo.y)
    };
    if !fa && (fb || size(a) >= size(b)) {
        let (l, r) = split(a, 0.5);
        let m = 0.5 * (ra.0 + ra.1);
        cross(&l, (ra.0, m), b, rb, depth + 1, budget, out);
        cross(&r, (m, ra.1), b, rb, depth + 1, budget, out);
    } else {
        let (l, r) = split(b, 0.5);
        let m = 0.5 * (rb.0 + rb.1);
        cross(a, ra, &l, (rb.0, m), depth + 1, budget, out);
        cross(a, ra, &r, (m, rb.1), depth + 1, budget, out);
    }
}

// ----- the planar map ---------------------------------------------------------------------------

struct Edge {
    c: Cubic,
    line: bool,
    v: [usize; 2],
    /// The segment it is part of, and where along it.
    src: usize,
    t: [f64; 2],
    /// Items stroking it.
    strokes: Vec<usize>,
}

/// One half of an edge: `2 · edge` runs `v[0] → v[1]`, `2 · edge + 1` back.
type Half = usize;

struct Map {
    verts: Vec<P>,
    edges: Vec<Edge>,
    /// Outgoing halves of each vertex, counter-clockwise.
    around: Vec<Vec<Half>>,
    /// Where each half sits in its origin's `around`.
    slot: Vec<usize>,
}

/// Greedy clustering: a point joins the nearest vertex whose first point is within [`TOL`].
struct Verts {
    pts: Vec<P>,
    grid: HashMap<(i64, i64), Vec<usize>>,
}

/// What fell into one vertex, to decide where it goes (issue #42: anchor points closer than
/// [`SNAP`] meet at their midpoint; a point that lands on a line goes onto it, and the line stays).
#[derive(Default)]
struct Cluster {
    /// A point inside a segment, away from its ends (where an end lies on a line, or lines
    /// cross): the vertex stays there.
    on_line: Option<P>,
    /// Segment ends, each position once.
    ends: Vec<P>,
}

impl Cluster {
    fn add(&mut self, p: P, end: bool, on_line: bool) {
        if on_line {
            self.on_line.get_or_insert(p);
        } else if end && !self.ends.iter().any(|q| q.dist(p) < 1e-9) && self.ends.len() < 64 {
            self.ends.push(p);
        }
    }
    fn place(&self, seed: P) -> P {
        if let Some(p) = self.on_line {
            return p;
        }
        if self.ends.is_empty() {
            return seed;
        }
        let n = self.ends.len() as f64;
        let sum = self.ends.iter().fold(P::default(), |s, p| s.add(*p));
        sum.mul(1.0 / n)
    }
}

impl Verts {
    fn cell(p: P) -> (i64, i64) {
        ((p.x / TOL).floor() as i64, (p.y / TOL).floor() as i64)
    }
    fn at(&mut self, p: P) -> usize {
        let (cx, cy) = Verts::cell(p);
        let mut best: Option<(f64, usize)> = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for &i in self.grid.get(&(cx + dx, cy + dy)).map_or(&[][..], Vec::as_slice) {
                    let d = self.pts[i].dist(p);
                    if d < TOL && best.is_none_or(|b| d < b.0) {
                        best = Some((d, i));
                    }
                }
            }
        }
        if let Some((_, i)) = best {
            return i;
        }
        self.pts.push(p);
        self.grid.entry((cx, cy)).or_default().push(self.pts.len() - 1);
        self.pts.len() - 1
    }
}

/// `c` with its ends moved to `a` and `b` (the handles move with them).
fn pinned(c: &Cubic, line: bool, a: P, b: P) -> Cubic {
    if line {
        return line_cubic(a, b);
    }
    [a, c[1].add(a.sub(c[0])), c[2].add(b.sub(c[3])), b]
}

impl Map {
    fn build(segs: &[Seg], splits: &[Vec<f64>]) -> Map {
        let mut vs = Verts { pts: Vec::new(), grid: HashMap::new() };
        // Every cut point into a vertex first, then each vertex placed (see `Cluster`), so the
        // pieces are pinned to where their vertices end up.
        let mut clusters: Vec<Cluster> = Vec::new();
        let mut ids: Vec<Vec<usize>> = Vec::with_capacity(segs.len());
        for (s, ts) in segs.iter().zip(splits) {
            let mut row = Vec::with_capacity(ts.len());
            for &t in ts {
                let end = t <= 0.0 || t >= 1.0;
                let p = if t <= 0.0 { s.c[0] } else if t >= 1.0 { s.c[3] } else { eval(&s.c, t) };
                let v = vs.at(p);
                if clusters.len() <= v {
                    clusters.resize_with(v + 1, Cluster::default);
                }
                let on_line = !end && p.dist(s.c[0]) >= TOL && p.dist(s.c[3]) >= TOL;
                clusters[v].add(p, end, on_line);
                row.push(v);
            }
            ids.push(row);
        }
        for (p, c) in vs.pts.iter_mut().zip(&clusters) {
            *p = c.place(*p);
        }
        let mut edges: Vec<Edge> = Vec::new();
        let mut by_ends: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
        for (si, ((s, ts), row)) in segs.iter().zip(splits).zip(&ids).enumerate() {
            for (w, v) in ts.windows(2).zip(row.windows(2)) {
                let (t0, t1) = (w[0], w[1]);
                let c = sub(&s.c, t0, t1);
                let (a, b) = (v[0], v[1]);
                if a == b {
                    continue; // shorter than SNAP (longer loops were halved when collected)
                }
                let c = pinned(&c, s.line, vs.pts[a], vs.pts[b]);
                let key = (a.min(b), a.max(b));
                let strokes = if s.stroke { vec![s.item] } else { Vec::new() };
                // Pieces that run along each other are one edge, stroked by all of them.
                let same = by_ends.get(&key).and_then(|list| {
                    list.iter().copied().find(|&e| (1..=3).all(|k| nearest(&edges[e].c, edges[e].line, eval(&c, f64::from(k) / 4.0)).1 < TOL))
                });
                match same {
                    Some(e) => {
                        for i in strokes {
                            if !edges[e].strokes.contains(&i) {
                                edges[e].strokes.push(i);
                            }
                        }
                    }
                    None => {
                        by_ends.entry(key).or_default().push(edges.len());
                        edges.push(Edge { c, line: s.line, v: [a, b], src: si, t: [t0, t1], strokes });
                    }
                }
            }
        }
        let mut map = Map { verts: vs.pts, edges, around: Vec::new(), slot: Vec::new() };
        map.sort_around();
        map
    }

    fn origin(&self, h: Half) -> usize {
        self.edges[h / 2].v[h % 2]
    }
    fn dest(&self, h: Half) -> usize {
        self.edges[h / 2].v[1 - h % 2]
    }
    /// The half's curve, in its direction.
    fn curve(&self, h: Half) -> Cubic {
        let c = self.edges[h / 2].c;
        if h.is_multiple_of(2) { c } else { reversed(&c) }
    }
    /// The next half round the face on this half's left.
    fn next(&self, h: Half) -> Half {
        let t = h ^ 1;
        let list = &self.around[self.origin(t)];
        let i = self.slot[t];
        list[(i + list.len() - 1) % list.len()]
    }

    /// Orders each vertex's outgoing halves by angle, by where each first gets to a small
    /// circle round the vertex (so curves leaving along one tangent still sort by how they bend).
    fn sort_around(&mut self) {
        let mut around: Vec<Vec<Half>> = vec![Vec::new(); self.verts.len()];
        for h in 0..self.edges.len() * 2 {
            around[self.origin(h)].push(h);
        }
        let mut slot = vec![0; self.edges.len() * 2];
        for (v, list) in around.iter_mut().enumerate() {
            let o = self.verts[v];
            let reach = |c: &Cubic| (1..=16).map(|k| eval(c, f64::from(k) / 16.0).dist(o)).fold(0.0, f64::max);
            let r = 0.5 * list.iter().map(|&h| reach(&self.curve(h))).fold(f64::INFINITY, f64::min);
            let dirs: Vec<(Half, P)> = list.iter().map(|&h| (h, at_radius(&self.curve(h), o, r).sub(o))).collect();
            let mut dirs = dirs;
            dirs.sort_by(|a, b| angle_cmp(a.1, b.1));
            *list = dirs.into_iter().map(|d| d.0).collect();
            for (i, &h) in list.iter().enumerate() {
                slot[h] = i;
            }
        }
        self.around = around;
        self.slot = slot;
    }

    /// Faces: every cycle of halves, its polygon and signed area; which face each half bounds.
    fn faces(&self) -> Faces {
        let n = self.edges.len() * 2;
        let mut cycle_of = vec![usize::MAX; n];
        let mut cycles: Vec<Vec<Half>> = Vec::new();
        for h0 in 0..n {
            if cycle_of[h0] != usize::MAX {
                continue;
            }
            let mut cyc = Vec::new();
            let mut h = h0;
            while cycle_of[h] == usize::MAX && cyc.len() <= n {
                cycle_of[h] = cycles.len();
                cyc.push(h);
                h = self.next(h);
            }
            cycles.push(cyc);
        }
        let polys: Vec<Vec<P>> = cycles.iter().map(|c| self.polygon(c)).collect();
        let area: Vec<f64> = polys.iter().map(|p| signed_area(p)).collect();
        // Components, to find which face holds each component's outside.
        let mut comp = Dsu::new(self.verts.len());
        for e in &self.edges {
            comp.union(e.v[0], e.v[1]);
        }
        let roots = comp.roots();
        let comps: Vec<usize> = cycles.iter().map(|c| c.first().and_then(|&h| roots.get(self.origin(h)).copied()).unwrap_or(usize::MAX)).collect();
        let mut face_of_cycle: Vec<Option<usize>> = vec![None; cycles.len()];
        let mut faces: Vec<Vec<usize>> = Vec::new();
        for (i, a) in area.iter().enumerate() {
            if *a > 0.0 {
                face_of_cycle[i] = Some(faces.len());
                faces.push(vec![i]);
            }
        }
        for i in 0..cycles.len() {
            if area[i] > 0.0 {
                continue;
            }
            let Some(&h) = cycles[i].first() else { continue };
            let p = self.verts[self.origin(h)];
            let holder = (0..cycles.len())
                .filter(|&j| area[j] > 0.0 && comps[j] != comps[i] && inside(&polys[j], p))
                .min_by(|&a, &b| area[a].total_cmp(&area[b]));
            if let Some(f) = holder.and_then(|j| face_of_cycle[j]) {
                face_of_cycle[i] = Some(f);
                faces[f].push(i);
            }
        }
        let face_of_half = cycle_of.iter().map(|&c| face_of_cycle.get(c).copied().flatten()).collect();
        let samples = faces.iter().map(|f| interior_point(&f.iter().map(|&c| polys[c].as_slice()).collect::<Vec<_>>())).collect();
        Faces { face_of_half, samples }
    }

    fn polygon(&self, cycle: &[Half]) -> Vec<P> {
        let mut pts = Vec::new();
        if let Some(&h) = cycle.first() {
            pts.push(self.verts[self.origin(h)]);
        }
        for &h in cycle {
            flatten_into(&self.curve(h), self.edges[h / 2].line, &mut pts);
        }
        pts
    }

    fn output(&self, items: &[Item], segs: &[Seg]) -> Merged {
        let Faces { face_of_half, samples } = self.faces();
        let face = |h: Half| face_of_half.get(h).copied().flatten();
        // Which fill items cover each face, bottom to top.
        let regions: Vec<Region> = (0..items.len()).map(|i| Region::new(segs, i, items.get(i).is_some_and(|it| it.fill.is_some()))).collect();
        let covers: Vec<Vec<usize>> = samples
            .iter()
            .map(|s| match s {
                Some(p) => (0..items.len()).filter(|&i| regions[i].covers(*p, items[i].fill_rule)).collect(),
                None => Vec::new(),
            })
            .collect();
        let fill_of = |f: usize| covers[f].last().and_then(|&i| items[i].fill.map(|s| (s, i)));
        // An edge's stroke shows unless a later fill covers both its sides.
        let stroke_of: Vec<Option<usize>> = self
            .edges
            .iter()
            .enumerate()
            .map(|(e, edge)| {
                let z = edge.strokes.iter().copied().filter(|&i| i < items.len()).max()?;
                let (l, r) = (face(2 * e), face(2 * e + 1));
                let hidden = l.zip(r).is_some_and(|(l, r)| covers[l].iter().any(|&i| i > z && covers[r].contains(&i)));
                (!hidden).then_some(z)
            })
            .collect();
        // The cutter's edges divide like a stroke that never shows.
        let cut_edge: Vec<bool> = self.edges.iter().map(|e| e.strokes.contains(&CUTTER)).collect();
        let cutter = Region::new(segs, CUTTER, cut_edge.contains(&true));
        // Faces of one fill with no stroke between them are one region.
        let mut group = Dsu::new(samples.len());
        for (e, stroke) in stroke_of.iter().enumerate() {
            if let (None, false, Some(l), Some(r)) = (stroke, cut_edge[e], face(2 * e), face(2 * e + 1))
                && l != r
                && fill_of(l).map(|f| f.0) == fill_of(r).map(|f| f.0)
                && fill_of(l).is_some()
            {
                group.union(l, r);
            }
        }
        let roots = group.roots();
        let gid = |f: Option<usize>| f.filter(|&f| fill_of(f).is_some()).and_then(|f| roots.get(f).copied());
        // Each region's style and topmost item, and whether it is inside the cutter (the
        // cutter's edges bound every region, so one sample tells).
        let mut top: HashMap<usize, (u32, usize, bool)> = HashMap::new();
        for (f, sample) in samples.iter().enumerate() {
            if let (Some(g), Some((style, item))) = (gid(Some(f)), fill_of(f)) {
                let inside = sample.is_some_and(|p| cutter.covers(p, FillRule::EvenOdd));
                let t = top.entry(g).or_insert((style, item, inside));
                t.1 = t.1.max(item);
            }
        }
        let mut out = Merged::default();
        let mut piece_of: HashMap<usize, usize> = HashMap::new();
        let mut done = vec![false; self.edges.len() * 2];
        for h0 in 0..self.edges.len() * 2 {
            let Some(g) = gid(face(h0)) else { continue };
            if done[h0] || gid(face(h0 ^ 1)) == Some(g) {
                continue;
            }
            // One boundary loop of the region: round each vertex past the edges inside it.
            let mut halves = Vec::new();
            let mut h = h0;
            while !done[h] && halves.len() <= done.len() {
                done[h] = true;
                halves.push(h);
                let mut n = self.next(h);
                let mut guard = 0;
                while gid(face(n ^ 1)) == Some(g) && guard <= done.len() {
                    n = self.next(n ^ 1);
                    guard += 1;
                }
                h = n;
            }
            let path = self.emit(&halves, true, segs);
            match piece_of.get(&g) {
                Some(&i) => out.fills[i].path.cmds.extend(path.cmds),
                None => {
                    let (style, item, inside) = top.get(&g).copied().unwrap_or((0, 0, false));
                    piece_of.insert(g, out.fills.len());
                    out.fills.push(Piece { style, path, item, inside, spans: Vec::new() });
                }
            }
        }
        self.strokes(items, &stroke_of, &cut_edge, &cutter, segs, &mut out);
        out
    }

    /// Visible strokes, chained through vertices where exactly two of one style meet smoothly
    /// (and never across the cutter).
    fn strokes(&self, items: &[Item], stroke_of: &[Option<usize>], cut_edge: &[bool], cutter: &Region, segs: &[Seg], out: &mut Merged) {
        let style = |e: usize| stroke_of[e].and_then(|z| items[z].stroke);
        let shown: Vec<Vec<Half>> = self.around.iter().map(|l| l.iter().copied().filter(|&h| style(h / 2).is_some()).collect()).collect();
        let on_cut: Vec<bool> = self.around.iter().map(|l| l.iter().any(|&h| cut_edge[h / 2])).collect();
        // The half that carries on from `h` at its end, if the run goes on.
        let carry = |h: Half| -> Option<Half> {
            let v = self.dest(h);
            let list = &shown[v];
            if list.len() != 2 || on_cut[v] {
                return None;
            }
            let n = list.iter().copied().find(|&o| o != (h ^ 1))?;
            let (din, dout) = (tangent(&reversed(&self.curve(h))).mul(-1.0), tangent(&self.curve(n)));
            (n / 2 != h / 2 && style(n / 2) == style(h / 2) && din.dot(dout) >= SMOOTH_COS * din.len() * dout.len()).then_some(n)
        };
        let mut seen = vec![false; self.edges.len()];
        for e in 0..self.edges.len() {
            let Some(st) = style(e) else { continue };
            if seen[e] {
                continue;
            }
            // Back up to where the run starts (or all the way round a loop).
            let mut start = 2 * e;
            let mut closed = false;
            let mut steps = 0;
            while let Some(p) = carry(start ^ 1) {
                start = p ^ 1;
                steps += 1;
                if start / 2 == e || steps > self.edges.len() {
                    closed = start / 2 == e;
                    break;
                }
            }
            let mut halves = vec![start];
            seen[start / 2] = true;
            let mut h = start;
            while let Some(n) = carry(h) {
                if seen[n / 2] {
                    break;
                }
                seen[n / 2] = true;
                halves.push(n);
                h = n;
            }
            let item = halves.iter().filter_map(|&h| stroke_of[h / 2]).max().unwrap_or(0);
            // A run along the cutter's edge counts as inside it.
            let inside = halves.iter().any(|&h| cut_edge[h / 2]) || halves.get(halves.len() / 2).is_some_and(|&h| cutter.covers(eval(&self.curve(h), 0.5), FillRule::EvenOdd));
            let (path, spans) = self.emit_spans(&halves, closed, segs);
            out.strokes.push(Piece { style: st, path, item, inside, spans });
        }
    }

    /// A subpath along `halves`: pieces of one segment joined back, straight runs made one line.
    fn emit(&self, halves: &[Half], closed: bool, segs: &[Seg]) -> Path {
        self.emit_spans(halves, closed, segs).0
    }

    /// [`Map::emit`], with the stretches of the items' paths the subpath runs along.
    fn emit_spans(&self, halves: &[Half], closed: bool, segs: &[Seg]) -> (Path, Vec<[f64; 2]>) {
        let mut runs: Vec<Run> = Vec::new();
        for &h in halves {
            let e = &self.edges[h / 2];
            let (t0, t1) = if h.is_multiple_of(2) { (e.t[0], e.t[1]) } else { (e.t[1], e.t[0]) };
            let spans = segs.get(e.src).map(|s| vec![[s.at(t0), s.at(t1)]]).unwrap_or_default();
            let run = Run { c: self.curve(h), line: e.line, src: e.src, t: [t0, t1], spans };
            match runs.last_mut().and_then(|last| last.join(&run, segs)) {
                Some(j) => {
                    if let Some(last) = runs.last_mut() {
                        *last = j;
                    }
                }
                None => runs.push(run),
            }
        }
        if closed && runs.len() > 1
            && let (Some(last), Some(first)) = (runs.last(), runs.first())
            && let Some(j) = last.join(first, segs)
        {
            runs[0] = j;
            runs.pop();
        }
        let mut path = Path::new();
        if let Some(first) = runs.first() {
            path.move_to(first.c[0].vec2());
        }
        for r in &runs {
            if r.line {
                path.line_to(r.c[3].vec2());
            } else {
                path.cubic_to(r.c[1].vec2(), r.c[2].vec2(), r.c[3].vec2());
            }
        }
        if closed && !runs.is_empty() {
            path.close();
        }
        let mut spans: Vec<[f64; 2]> = Vec::new();
        for sp in runs.into_iter().flat_map(|r| r.spans) {
            push_span(&mut spans, sp);
        }
        (path, spans)
    }
}

/// Adds `sp` to `spans`, extending the last one when `sp` carries straight on from it.
fn push_span(spans: &mut Vec<[f64; 2]>, sp: [f64; 2]) {
    if let Some(last) = spans.last_mut()
        && (last[1] - sp[0]).abs() < 1e-9
        && (last[1] >= last[0]) == (sp[1] >= sp[0])
    {
        last[1] = sp[1];
        return;
    }
    spans.push(sp);
}

/// Two runs' spans, one after the other.
fn joined(a: &[[f64; 2]], b: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut out = a.to_vec();
    for &sp in b {
        push_span(&mut out, sp);
    }
    out
}

struct Faces {
    face_of_half: Vec<Option<usize>>,
    /// A point inside each face.
    samples: Vec<Option<P>>,
}

/// A run of output: a curve and the segment stretch it came from.
struct Run {
    c: Cubic,
    line: bool,
    src: usize,
    t: [f64; 2],
    /// The stretches of item paths it covers ([`Piece::spans`]).
    spans: Vec<[f64; 2]>,
}

impl Run {
    /// `self` then `o` as one run: the same segment's next stretch, or a straight continuation.
    fn join(&self, o: &Run, segs: &[Seg]) -> Option<Run> {
        let forward = |r: &Run| r.t[1] > r.t[0];
        if self.src == o.src && self.t[1] == o.t[0] && forward(self) == forward(o) && self.src != usize::MAX {
            let s = segs.get(self.src)?;
            let c = pinned(&sub(&s.c, self.t[0], o.t[1]), self.line, self.c[0], o.c[3]);
            return Some(Run { c, line: self.line, src: self.src, t: [self.t[0], o.t[1]], spans: joined(&self.spans, &o.spans) });
        }
        if self.line && o.line {
            let (a, b, c) = (self.c[0], self.c[3], o.c[3]);
            let d = c.sub(a);
            let l = d.len();
            if l > 1e-9 && (d.cross(b.sub(a)) / l).abs() < 0.01 && d.dot(b.sub(a)) > 0.0 && d.dot(c.sub(b)) > 0.0 {
                return Some(Run { c: line_cubic(a, c), line: true, src: usize::MAX, t: [0.0, 0.0], spans: joined(&self.spans, &o.spans) });
            }
        }
        None
    }
}

/// The direction a curve leaves its start.
fn tangent(c: &Cubic) -> P {
    for p in &c[1..] {
        let d = p.sub(c[0]);
        if d.len() > 1e-9 {
            return d;
        }
    }
    P::default()
}

/// Where the curve (leaving `o`) first gets `r` away from `o`.
fn at_radius(c: &Cubic, o: P, r: f64) -> P {
    const N: usize = 64;
    let mut prev = 0.0;
    for i in 1..=N {
        let t = i as f64 / N as f64;
        if eval(c, t).dist(o) >= r {
            let (mut lo, mut hi) = (prev, t);
            for _ in 0..30 {
                let m = 0.5 * (lo + hi);
                if eval(c, m).dist(o) >= r {
                    hi = m;
                } else {
                    lo = m;
                }
            }
            return eval(c, hi);
        }
        prev = t;
    }
    c[3]
}

/// Orders directions counter-clockwise from +x, exactly (no trigonometry).
fn angle_cmp(a: P, b: P) -> std::cmp::Ordering {
    let half = |p: P| u8::from(!(p.y > 0.0 || (p.y == 0.0 && p.x > 0.0)));
    half(a).cmp(&half(b)).then_with(|| 0.0f64.total_cmp(&a.cross(b)))
}

fn signed_area(pts: &[P]) -> f64 {
    let n = pts.len();
    (0..n).map(|i| pts[i].cross(pts[(i + 1) % n])).sum::<f64>() * 0.5
}

fn inside(poly: &[P], p: P) -> bool {
    let n = poly.len();
    let mut c = false;
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) * (b.x - a.x) / (b.y - a.y) {
            c = !c;
        }
    }
    c
}

/// A point well inside the region the polygons bound (even-odd): the middle of the widest span
/// along a few horizontal lines.
fn interior_point(polys: &[&[P]]) -> Option<P> {
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for p in polys.iter().flat_map(|p| p.iter()) {
        lo = lo.min(p.y);
        hi = hi.max(p.y);
    }
    let mut best: Option<(f64, P)> = None;
    for k in 1..=9 {
        let y = lo + (hi - lo) * f64::from(k) / 10.0;
        let mut xs: Vec<f64> = Vec::new();
        for poly in polys {
            let n = poly.len();
            for i in 0..n {
                let (a, b) = (poly[i], poly[(i + 1) % n]);
                if (a.y > y) != (b.y > y) {
                    xs.push(a.x + (y - a.y) * (b.x - a.x) / (b.y - a.y));
                }
            }
        }
        xs.sort_by(f64::total_cmp);
        for [x0, x1] in xs.as_chunks::<2>().0 {
            let width = x1 - x0;
            if width > 1e-6 && best.is_none_or(|b| width > b.0) {
                best = Some((width, P::new(0.5 * (x0 + x1), y)));
            }
        }
    }
    best.map(|b| b.1)
}

/// One item's fill outline, flattened, for winding tests.
struct Region {
    lines: Vec<(P, P)>,
    lo: P,
    hi: P,
}

impl Region {
    fn new(segs: &[Seg], item: usize, filled: bool) -> Region {
        let mut lines = Vec::new();
        let (mut lo, mut hi) = (P::new(f64::INFINITY, f64::INFINITY), P::new(f64::NEG_INFINITY, f64::NEG_INFINITY));
        if filled {
            for s in segs.iter().filter(|s| s.item == item) {
                let mut pts = vec![s.c[0]];
                flatten_into(&s.c, s.line, &mut pts);
                for w in pts.windows(2) {
                    lines.push((w[0], w[1]));
                }
                let (a, b) = bbox(&s.c);
                lo = P::new(lo.x.min(a.x), lo.y.min(a.y));
                hi = P::new(hi.x.max(b.x), hi.y.max(b.y));
            }
        }
        Region { lines, lo, hi }
    }

    fn covers(&self, p: P, rule: FillRule) -> bool {
        if self.lines.is_empty() || p.x < self.lo.x || p.x > self.hi.x || p.y < self.lo.y || p.y > self.hi.y {
            return false;
        }
        let mut w = 0i32;
        for &(a, b) in &self.lines {
            if a.y <= p.y {
                if b.y > p.y && b.sub(a).cross(p.sub(a)) > 0.0 {
                    w += 1;
                }
            } else if b.y <= p.y && b.sub(a).cross(p.sub(a)) < 0.0 {
                w -= 1;
            }
        }
        match rule {
            FillRule::NonZero => w != 0,
            FillRule::EvenOdd => w % 2 != 0,
        }
    }
}

/// Union-find (path halving; the smaller index is the root).
struct Dsu(Vec<usize>);

impl Dsu {
    fn new(n: usize) -> Dsu {
        Dsu((0..n).collect())
    }
    fn find(&mut self, mut i: usize) -> usize {
        while let Some(&p) = self.0.get(i) {
            if p == i {
                break;
            }
            let gp = self.0.get(p).copied().unwrap_or(p);
            if let Some(x) = self.0.get_mut(i) {
                *x = gp;
            }
            i = gp;
        }
        i
    }
    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb
            && let Some(x) = self.0.get_mut(ra.max(rb))
        {
            *x = ra.min(rb);
        }
    }
    /// Every element's root.
    fn roots(&mut self) -> Vec<usize> {
        (0..self.0.len()).map(|i| self.find(i)).collect()
    }
}

#[cfg(test)]
#[path = "planar_tests.rs"]
mod tests;
