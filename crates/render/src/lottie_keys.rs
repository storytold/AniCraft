//! Lottie keyframes from per-frame values (ADR 0089).
//!
//! The Lottie exporter samples every frame through `evaluate`, so each animated property is a
//! list of values, one per frame. This module writes such a list as few Lottie keyframes as
//! draw the same frames: holds where a value stays put, and where it travels, keyframes eased
//! by Lottie's own cubic Bézier eases (`o`/`i` handles), taken from the eases the document's
//! tweens use (linear, Classic Ease, Bézier and custom curves, and the presets that are cubics
//! piece by piece). A curve of several cubic pieces becomes several keys, one per piece, at the
//! times its points fall (fractional frames). Every span is checked before it is written: the
//! keys, as written (rounded), are evaluated the way a Lottie player evaluates them at every
//! frame of the span and must land on the sampled value within a tolerance far below a pixel.
//! What no ease draws (Elastic, Sine, nested tweens, morphs, motion along a curve) stays a hold
//! key per frame, exactly as before.
//!
//! Lottie keyframe semantics follow the public Lottie format description: a key's value holds
//! (`h: 1`) or travels to the next key's value along the cubic Bézier from (0, 0) to (1, 1)
//! through `o` and `i` (time across, share of the value change up).

use anicraft_doc::{Document, Easing, ElementKind, Timeline};

/// A cubic ease piece in the unit box: start, two handles, end (absolute points).
pub(crate) type Seg = [(f64, f64); 4];

/// A Lottie ease: the out handle of a key, then the in handle of the next.
type Handles = ((f64, f64), (f64, f64));

/// A piece as Lottie keys it: its points and handles (`None`: it doesn't change value).
type Piece = (Seg, Option<Handles>);

/// Most distinct eases tried per export (a document may hold thousands of tweens).
const MAX_EASES: usize = 64;
/// Longest span (in frames) one fit may cover.
const MAX_SPAN: usize = 2400;
/// Work allowed per property (frames checked), so a long movie that no ease draws stays fast.
const BUDGET: u64 = 2_000_000;

/// The eases a fit may use, each as its cubic pieces. Linear always comes first.
#[derive(Clone, Debug, Default)]
pub(crate) struct Eases {
    curves: Vec<Vec<Seg>>,
}

impl Eases {
    /// None: every change becomes hold keys, frame by frame (the export of earlier builds).
    pub(crate) fn none() -> Eases {
        Eases::default()
    }

    /// Linear only.
    #[cfg(test)]
    pub(crate) fn linear() -> Eases {
        Eases { curves: vec![segments(&Easing::Linear).unwrap_or_default()] }
    }

    /// Linear and every ease the document's tweens use (classic tweens, their property
    /// groups, motion-tween property keys), across scenes and symbols.
    pub(crate) fn of(doc: &Document) -> Eases {
        let mut out = Eases { curves: Vec::new() };
        out.add(&Easing::Linear);
        for tl in doc.scenes.iter().map(|s| &s.timeline).chain(doc.library.iter().map(|s| &s.timeline)) {
            out.add_timeline(tl, 0);
        }
        out
    }

    fn add_timeline(&mut self, tl: &Timeline, depth: usize) {
        if depth > 8 {
            return;
        }
        for layer in &tl.layers {
            for kf in &layer.keyframes {
                if let Some(t) = &kf.tween {
                    self.add(&t.easing);
                    for (_, e) in &t.groups {
                        self.add(e);
                    }
                }
                if let Some(m) = &kf.motion {
                    for k in m.tracks.iter().flat_map(|t| &t.keys) {
                        self.add(&k.ease);
                    }
                }
                // Groups hold their members' timelines (ADR 0073).
                for el in &kf.elements {
                    if let ElementKind::Group { members } = &el.kind {
                        self.add_timeline(members, depth + 1);
                    }
                }
            }
        }
    }

    fn add(&mut self, e: &Easing) {
        if let Some(s) = segments(e).filter(|s| !s.is_empty())
            && self.curves.len() < MAX_EASES
            && !self.curves.contains(&s)
        {
            self.curves.push(s);
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.curves.is_empty()
    }
}

/// An ease as cubic pieces, when cubic pieces draw it exactly: linear, Classic Ease, a Bézier,
/// Quad, Cubic and Back in and out (one cubic each), their in-outs and Bounce (several, ADR
/// 0081's exact custom forms), and custom curves. `None` for Quart, Quint, Sine, Circ and
/// Elastic, which no cubic draws exactly.
pub(crate) fn segments(e: &Easing) -> Option<Vec<Seg>> {
    use anicraft_doc::EaseFamily as F;
    let one = |(x1, y1, x2, y2): (f32, f32, f32, f32)| vec![[(0.0, 0.0), (f64::from(x1).clamp(0.0, 1.0), f64::from(y1)), (f64::from(x2).clamp(0.0, 1.0), f64::from(y2)), (1.0, 1.0)]];
    let pieces = |c: &anicraft_doc::CustomEase| -> Vec<Seg> { (0..c.points().len().saturating_sub(1)).filter_map(|i| c.segment(i)).map(|s| s.map(|p| (f64::from(p.x), f64::from(p.y)))).collect() };
    match e {
        Easing::Custom(c) => Some(pieces(c)),
        Easing::Linear | Easing::Classic(_) | Easing::CubicBezier(..) | Easing::QuadIn | Easing::QuadOut | Easing::CubicIn | Easing::CubicOut | Easing::BackIn | Easing::BackOut => e.as_bezier().map(one),
        _ => match e.family() {
            Some((F::Quad | F::Cubic | F::Back | F::Bounce, _)) => Some(pieces(&e.to_custom())),
            _ => None,
        },
    }
}

/// How a key leaves for the next one.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Out {
    /// The value holds until the next key.
    Hold,
    /// It travels along the ease with these handles (`o`, then `i`).
    Ease((f64, f64), (f64, f64)),
    /// The last key.
    End,
}

/// One Lottie keyframe.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Key {
    pub t: f64,
    pub s: Vec<f64>,
    pub out: Out,
}

/// Lottie numbers as the exporter writes them (three decimals).
pub(crate) fn r3(v: f64) -> f64 {
    let r = (v * 1000.0).round() / 1000.0;
    if r.is_finite() && r != 0.0 { r } else { 0.0 }
}

fn r6(v: f64) -> f64 {
    let r = (v * 1e6).round() / 1e6;
    if r.is_finite() && r != 0.0 { r } else { 0.0 }
}

/// The point at `s` on the cubic through `p`.
fn bez(s: f64, p: [f64; 4]) -> f64 {
    let u = 1.0 - s;
    u * u * u * p[0] + 3.0 * u * u * s * p[1] + 3.0 * u * s * s * p[2] + s * s * s * p[3]
}

/// A Lottie ease (CSS-style handles, x within 0..1) at time share `x`: the share of the value
/// change. Bisection with Newton steps: x(s) is monotonic for handles within 0..1.
pub(crate) fn ease_at(o: (f64, f64), i: (f64, f64), x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    let xs = [0.0, o.0, i.0, 1.0];
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    let mut s = x;
    for _ in 0..60 {
        let fx = bez(s, xs) - x;
        if fx.abs() < 1e-12 {
            break;
        }
        if fx > 0.0 {
            hi = s;
        } else {
            lo = s;
        }
        let u = 1.0 - s;
        let d = 3.0 * u * u * (xs[1] - xs[0]) + 6.0 * u * s * (xs[2] - xs[1]) + 3.0 * s * s * (xs[3] - xs[2]);
        let next = if d.abs() > 1e-12 { s - fx / d } else { f64::NAN };
        s = if next.is_finite() && next > lo && next < hi { next } else { 0.5 * (lo + hi) };
    }
    bez(s, [0.0, o.1, i.1, 1.0])
}

/// The value `keys` show at time `t`, as a Lottie player computes it.
pub(crate) fn value_at(keys: &[Key], t: f64) -> Option<Vec<f64>> {
    let first = keys.first()?;
    if t <= first.t {
        return Some(first.s.clone());
    }
    let k = keys.partition_point(|k| k.t <= t).saturating_sub(1);
    let (a, b) = (keys.get(k)?, keys.get(k + 1));
    let Some(b) = b else { return Some(a.s.clone()) };
    match a.out {
        Out::Hold | Out::End => Some(a.s.clone()),
        Out::Ease(o, i) => {
            let span = b.t - a.t;
            let x = if span > 0.0 { (t - a.t) / span } else { 1.0 };
            let y = ease_at(o, i, x);
            Some(a.s.iter().zip(&b.s).map(|(p, q)| p + (q - p) * y).collect())
        }
    }
}

/// Splits a cubic piece in two halves (de Casteljau at s = ½).
fn halves(p: &Seg) -> (Seg, Seg) {
    let mid = |a: (f64, f64), b: (f64, f64)| ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5);
    let (p01, p12, p23) = (mid(p[0], p[1]), mid(p[1], p[2]), mid(p[2], p[3]));
    let (p012, p123) = (mid(p01, p12), mid(p12, p23));
    let c = mid(p012, p123);
    ([p[0], p01, p012, c], [c, p123, p23, p[3]])
}

/// Lottie handles for a piece: `None` when the piece doesn't change value but bends (it is
/// split), `Some(None)` when it is flat (any ease).
fn handles(p: &Seg) -> Option<Option<Handles>> {
    let (dx, dy) = (p[3].0 - p[0].0, p[3].1 - p[0].1);
    let bend = (p[1].1 - p[0].1).abs().max((p[2].1 - p[0].1).abs());
    if dy.abs() < 1e-9 && bend < 1e-9 {
        return Some(None);
    }
    if dx <= 1e-9 || dy.abs() < 1e-9 || bend / dy.abs() > 50.0 {
        return None;
    }
    let o = (((p[1].0 - p[0].0) / dx).clamp(0.0, 1.0), (p[1].1 - p[0].1) / dy);
    let i = (((p[2].0 - p[0].0) / dx).clamp(0.0, 1.0), (p[2].1 - p[0].1) / dy);
    Some(Some(((r6(o.0), r6(o.1)), (r6(i.0), r6(i.1)))))
}

/// The pieces of a curve as Lottie can key them (pieces that bend without changing value are
/// halved, a few times at most); `None` when that isn't enough.
fn keyable(curve: &[Seg]) -> Option<Vec<Piece>> {
    fn push(p: &Seg, depth: u32, out: &mut Vec<Piece>) -> Option<()> {
        match handles(p) {
            Some(h) => out.push((*p, h)),
            None if depth < 4 => {
                let (a, b) = halves(p);
                push(&a, depth + 1, out)?;
                push(&b, depth + 1, out)?;
            }
            None => return None,
        }
        Some(())
    }
    let mut out = Vec::with_capacity(curve.len());
    for p in curve {
        push(p, 0, &mut out)?;
    }
    Some(out)
}

/// What a fit compares against: the sampled values (one per frame from `first`) and how far
/// a written value may be from them.
pub(crate) struct Series<'a> {
    pub first: u32,
    pub values: &'a [Vec<f64>],
    /// Absolute tolerance, in the property's own units.
    pub tol: f64,
}

impl Series<'_> {
    fn close(&self, got: &[f64], want: &[f64]) -> bool {
        got.len() == want.len() && got.iter().zip(want).all(|(g, w)| (g - w).abs() <= self.tol + 2e-6 * w.abs())
    }
}

/// The keys of span `a..=b` (indices into the series) eased by `curve`, without the end key,
/// when they draw every frame of it.
fn try_span(s: &Series<'_>, a: usize, b: usize, curve: &[Piece], work: &mut u64) -> Option<Vec<Key>> {
    let (va, vb) = (s.values.get(a)?, s.values.get(b)?);
    let d = (b - a) as f64;
    // A quick look at the first inside frame, on the unrounded curve, turns most misfits away
    // before any key is built (rounding moves values by well under the margin).
    *work += 1;
    let u = 1.0 / d;
    let (p, h) = curve.iter().rev().find(|(p, _)| p[0].0 <= u).or(curve.first())?;
    let dx = p[3].0 - p[0].0;
    let local = match h {
        Some((o, i)) if dx > 0.0 => ease_at(*o, *i, (u - p[0].0) / dx),
        _ => 0.0,
    };
    let y = p[0].1 + (p[3].1 - p[0].1) * local;
    let next = s.values.get(a + 1)?;
    let loose = Series { tol: 4.0 * s.tol + 2e-3, ..*s };
    if !loose.close(&va.iter().zip(vb).map(|(p, q)| p + (q - p) * y).collect::<Vec<f64>>(), next) {
        return None;
    }
    let t0 = f64::from(s.first) + a as f64;
    let at = |y: f64| -> Vec<f64> { va.iter().zip(vb).map(|(p, q)| r3(p + (q - p) * y)).collect() };
    let mut keys: Vec<Key> = Vec::with_capacity(curve.len() + 1);
    for (k, (p, h)) in curve.iter().enumerate() {
        // Key times to 1/10000 frame: the first is the span's start exactly.
        let t = if k == 0 { t0 } else { ((t0 + p[0].0 * d) * 1e4).round() / 1e4 };
        if keys.last().is_some_and(|l: &Key| t <= l.t) {
            return None;
        }
        let value = if k == 0 { va.iter().map(|v| r3(*v)).collect() } else { at(p[0].1) };
        let out = match h {
            Some((o, i)) => Out::Ease(*o, *i),
            None => Out::Ease((0.0, 0.0), (1.0, 1.0)),
        };
        keys.push(Key { t, s: value, out });
    }
    let t1 = t0 + d;
    if keys.last().is_some_and(|l| t1 <= l.t) {
        return None;
    }
    keys.push(Key { t: t1, s: vb.iter().map(|v| r3(*v)).collect(), out: Out::End });
    // Check every frame of the span, the first inside frame first (most misfits fail there).
    for f in (a + 1..b).chain(std::iter::once(b)) {
        *work += 1;
        let got = value_at(&keys, f64::from(s.first) + f as f64)?;
        if !s.close(&got, s.values.get(f)?) {
            return None;
        }
    }
    keys.pop();
    Some(keys)
}

/// The fewest keys (greedily, longest span first) that draw every frame of `s`: holds where
/// values stay, eased spans where an ease of `eases` draws them, hold keys frame by frame
/// where nothing does. Also says whether some frames had to be keyed one by one while moving.
pub(crate) fn fit(s: &Series<'_>, eases: &Eases) -> (Vec<Key>, bool) {
    let n = s.values.len();
    let mut keys: Vec<Key> = Vec::new();
    let Some(last) = s.values.last() else { return (keys, false) };
    let curves: Vec<Vec<Piece>> = eases.curves.iter().filter_map(|c| keyable(c)).collect();
    let mut work: u64 = 0;
    let mut stepped = false;
    let mut steps_in_row = 0;
    let mut a = 0usize;
    let rounded = |i: usize| -> Vec<f64> { s.values.get(i).map(|v| v.iter().map(|x| r3(*x)).collect()).unwrap_or_default() };
    while a + 1 < n {
        let Some(va) = s.values.get(a) else { break };
        // How long the value stays where it is.
        let mut hold_end = a;
        while hold_end + 1 < n && s.values.get(hold_end + 1).is_some_and(|v| s.close(va, v)) {
            hold_end += 1;
        }
        let mut best: Option<(usize, Vec<Key>)> = None;
        if !curves.is_empty() {
            let far = (a + MAX_SPAN).min(n - 1);
            'span: for b in (a + 2..=far).rev() {
                if b <= hold_end || work > BUDGET {
                    break;
                }
                for c in &curves {
                    if let Some(k) = try_span(s, a, b, c, &mut work) {
                        best = Some((b, k));
                        break 'span;
                    }
                }
            }
        }
        match best {
            Some((b, k)) => {
                keys.extend(k);
                steps_in_row = 0;
                a = b;
            }
            None => {
                keys.push(Key { t: f64::from(s.first) + a as f64, s: rounded(a), out: Out::Hold });
                if hold_end > a {
                    steps_in_row = 0;
                    a = hold_end;
                } else {
                    steps_in_row += 1;
                    if steps_in_row >= 2 {
                        stepped = true;
                    }
                    a += 1;
                }
            }
        }
    }
    keys.push(Key { t: f64::from(s.first) + (n - 1) as f64, s: last.iter().map(|x| r3(*x)).collect(), out: Out::End });
    // Consecutive holds of one value are one hold.
    let mut out: Vec<Key> = Vec::with_capacity(keys.len());
    for k in keys {
        if k.out == Out::Hold && out.last().is_some_and(|p: &Key| p.out == Out::Hold && p.s == k.s) {
            continue;
        }
        out.push(k);
    }
    // A last key repeating a held value adds nothing.
    if out.len() >= 2
        && let [.., prev, end] = out.as_slice()
        && prev.out == Out::Hold
        && prev.s == end.s
    {
        out.pop();
        if let Some(p) = out.last_mut() {
            p.out = Out::End;
        }
    }
    (out, stepped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anicraft_doc::CustomEase;
    use anicraft_geom::Vec2;
    use std::sync::Arc;

    fn series(e: &Easing, from: f64, to: f64, frames: usize, hold_after: usize) -> Vec<Vec<f64>> {
        let mut v: Vec<Vec<f64>> = (0..=frames).map(|f| vec![from + (to - from) * f64::from(e.apply(f as f32 / frames as f32))]).collect();
        v.extend((0..hold_after).map(|_| vec![to]));
        v
    }

    fn eases(list: &[Easing]) -> Eases {
        let mut e = Eases::linear();
        for x in list {
            e.add(x);
        }
        e
    }

    fn check(keys: &[Key], vals: &[Vec<f64>], tol: f64) {
        for (f, want) in vals.iter().enumerate() {
            let got = value_at(keys, f as f64).unwrap();
            assert!((got[0] - want[0]).abs() <= tol, "frame {f}: {got:?} vs {want:?} with {keys:?}");
        }
    }

    #[test]
    fn linear_motion_is_two_keys_and_a_hold() {
        let v = series(&Easing::Linear, 0.0, 300.0, 24, 10);
        let (keys, stepped) = fit(&Series { first: 0, values: &v, tol: 2e-3 }, &Eases::linear());
        assert!(!stepped);
        assert_eq!(keys.len(), 2, "{keys:?}");
        assert_eq!(keys[0].out, Out::Ease((0.333333, 0.333333), (0.666667, 0.666667)));
        assert_eq!((keys[1].t, keys[1].s.clone(), keys[1].out.clone()), (24.0, vec![300.0], Out::End), "the end of the motion, then it holds");
        check(&keys, &v, 2e-3);
    }

    #[test]
    fn single_cubic_presets_are_one_eased_key() {
        for e in [Easing::Classic(-60.0), Easing::Classic(100.0), Easing::QuadIn, Easing::CubicOut, Easing::BackOut, Easing::CubicBezier(0.1, 0.8, 0.3, 1.2)] {
            let v = series(&e, 10.0, -250.0, 30, 0);
            let (keys, stepped) = fit(&Series { first: 0, values: &v, tol: 2e-3 }, &eases(std::slice::from_ref(&e)));
            assert!(!stepped, "{e:?}");
            assert_eq!(keys.len(), 2, "{e:?}: {keys:?}");
            check(&keys, &v, 2e-3);
        }
    }

    #[test]
    fn multi_piece_eases_split_into_keys_at_their_points() {
        let custom = Easing::Custom(Arc::new(CustomEase::from_flat(&[(0.0, 0.0), (0.2, 1.4), (0.3, 1.4), (0.4, 1.4), (0.6, 0.5), (0.8, 1.0), (1.0, 1.0)].map(|(x, y)| Vec2::new(x, y))).unwrap()));
        for (e, pieces) in [(custom, 2), (Easing::QuadInOut, 2), (Easing::BackInOut, 2), (Easing::BounceOut, 7)] {
            let v = series(&e, 0.0, 100.0, 40, 5);
            let (keys, stepped) = fit(&Series { first: 3, values: &v, tol: 2e-3 }, &eases(std::slice::from_ref(&e)));
            assert!(!stepped, "{e:?}");
            assert_eq!(keys.len(), pieces + 1, "{e:?}: {keys:?}");
            assert_eq!(keys[0].t, 3.0);
            let shifted: Vec<Key> = keys.iter().map(|k| Key { t: k.t - 3.0, ..k.clone() }).collect();
            check(&shifted, &v, 2e-3);
        }
    }

    #[test]
    fn eases_no_cubic_draws_stay_frame_by_frame_and_exact() {
        for e in [Easing::ElasticOut, Easing::SineInOut, Easing::QuartIn, Easing::CircOut] {
            assert!(segments(&e).is_none(), "{e:?} has no exact cubic form");
            let mut list = Eases::linear();
            list.add(&e);
            let v = series(&e, 0.0, 100.0, 20, 0);
            let (keys, stepped) = fit(&Series { first: 0, values: &v, tol: 2e-3 }, &list);
            assert!(stepped, "{e:?}");
            check(&keys, &v, 2e-3);
        }
    }

    #[test]
    fn consecutive_tweens_break_where_the_motion_changes() {
        // Classic Ease 0→100 over 10 frames, hold 5, linear back to 40 over 8, hold.
        let mut v = series(&Easing::Classic(50.0), 0.0, 100.0, 10, 5);
        v.extend(series(&Easing::Linear, 100.0, 40.0, 8, 3).into_iter().skip(1));
        let (keys, stepped) = fit(&Series { first: 0, values: &v, tol: 2e-3 }, &eases(&[Easing::Classic(50.0)]));
        assert!(!stepped);
        let times: Vec<f64> = keys.iter().map(|k| k.t).collect();
        assert_eq!(times, [0.0, 10.0, 15.0, 23.0], "{keys:?}");
        assert_eq!(keys[1].out, Out::Hold);
        check(&keys, &v, 2e-3);
    }

    #[test]
    fn no_eases_means_hold_keys_where_values_change() {
        let v = series(&Easing::Linear, 0.0, 10.0, 4, 2);
        let (keys, _) = fit(&Series { first: 0, values: &v, tol: 2e-3 }, &Eases::none());
        assert_eq!(keys.iter().map(|k| k.t).collect::<Vec<_>>(), [0.0, 1.0, 2.0, 3.0, 4.0]);
        assert!(keys[..4].iter().all(|k| k.out == Out::Hold));
        check(&keys, &v, 1e-9);
        // Constant: one key.
        let (keys, _) = fit(&Series { first: 0, values: &[vec![1.0], vec![1.0], vec![1.0]], tol: 2e-3 }, &Eases::linear());
        assert_eq!(keys.len(), 1);
    }

    #[test]
    fn vectors_share_one_ease() {
        let e = Easing::Classic(-100.0);
        let v: Vec<Vec<f64>> = (0..=12).map(|f| {
            let y = f64::from(e.apply(f as f32 / 12.0));
            vec![5.0 + 100.0 * y, 7.0, -3.0 * y]
        }).collect();
        let (keys, _) = fit(&Series { first: 0, values: &v, tol: 2e-3 }, &eases(&[e]));
        assert_eq!(keys.len(), 2);
    }

    #[test]
    fn ease_solver_matches_the_presets() {
        for e in [Easing::Classic(30.0), Easing::CubicBezier(0.42, 0.0, 0.58, 1.0), Easing::BackIn] {
            let (x1, y1, x2, y2) = e.as_bezier().unwrap();
            for k in 0..=20 {
                let x = f64::from(k) / 20.0;
                let got = ease_at((f64::from(x1), f64::from(y1)), (f64::from(x2), f64::from(y2)), x);
                assert!((got - f64::from(e.apply(x as f32))).abs() < 1e-5, "{e:?} at {x}");
            }
        }
    }
}
