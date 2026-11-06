//! XFL shape edges → AniCraft paths.
//!
//! A `DOMShape` stores its outline as `<Edge>` elements. Each carries an `edges` string of
//! drawing commands in twips (1/20 px): `!x y` moves, `|x y` or `/x y` draws a line, `[cx cy x y`
//! or `]cx cy x y` draws a quadratic curve; `Sn` marks selection state and is skipped. Numbers
//! are decimal, or `#` hex fixed point (signed 24.8). An edge names the fill style on its
//! left (`fillStyle0`) and right (`fillStyle1`) and an optional stroke style.
//!
//! Fill regions are rebuilt per fill style: its right-side edges as drawn, its left-side edges
//! reversed, then chained end to start into closed loops. Strokes are the stroked edges in
//! drawing order. Written from the public description of the format; no Animate code involved.

use anicraft_geom::{Path, Vec2};
use std::collections::HashMap;

/// One segment in pixels: a line (`ctrl` = None) or a quadratic curve.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seg {
    pub from: Vec2,
    pub ctrl: Option<Vec2>,
    pub to: Vec2,
}

impl Seg {
    fn reversed(self) -> Seg {
        Seg { from: self.to, ctrl: self.ctrl, to: self.from }
    }
}

/// An `<Edge>`: its fill styles (left, right), stroke style and segments.
#[derive(Clone, Debug, Default)]
pub struct Edge {
    pub fill_left: u32,
    pub fill_right: u32,
    pub stroke: u32,
    pub segs: Vec<Seg>,
}

const TWIPS: f32 = 20.0;
const MAX_SEGS: usize = 2_000_000;

fn number(s: &[u8], i: &mut usize) -> Option<f32> {
    while s.get(*i).is_some_and(|c| c.is_ascii_whitespace()) {
        *i += 1;
    }
    let start = *i;
    if s.get(*i) == Some(&b'#') {
        *i += 1;
        let h0 = *i;
        while s.get(*i).is_some_and(u8::is_ascii_hexdigit) {
            *i += 1;
        }
        let int = std::str::from_utf8(s.get(h0..*i)?).ok()?;
        let mut frac = 0u32;
        if s.get(*i) == Some(&b'.') {
            *i += 1;
            let f0 = *i;
            while s.get(*i).is_some_and(u8::is_ascii_hexdigit) && *i - f0 < 2 {
                *i += 1;
            }
            let f = std::str::from_utf8(s.get(f0..*i)?).ok()?;
            // One digit means sixteenths of 256ths: "#A.8" = 10.5.
            frac = u32::from_str_radix(f, 16).ok()? << (if f.len() == 1 { 4 } else { 0 });
        }
        let v = u32::from_str_radix(if int.is_empty() { "0" } else { int }, 16).ok()?;
        let fixed = (v.wrapping_shl(8) | frac) as i32;
        return Some(fixed as f32 / 256.0);
    }
    if matches!(s.get(*i), Some(b'-' | b'+')) {
        *i += 1;
    }
    while s.get(*i).is_some_and(|c| c.is_ascii_digit() || *c == b'.' || *c == b'e' || *c == b'E') {
        *i += 1;
    }
    std::str::from_utf8(s.get(start..*i)?).ok()?.parse::<f32>().ok().filter(|v| v.is_finite())
}

/// Decodes an `edges` attribute into segments (pixels). Stops quietly at anything it can't
/// read, keeping what came before.
pub fn parse(s: &str) -> Vec<Seg> {
    let b = s.as_bytes();
    let mut i = 0;
    let mut cur = Vec2::ZERO;
    let mut out = Vec::new();
    let pt = |b: &[u8], i: &mut usize| -> Option<Vec2> { Some(Vec2::new(number(b, i)? / TWIPS, number(b, i)? / TWIPS)) };
    while let Some(&c) = b.get(i) {
        i += 1;
        match c {
            b'!' => match pt(b, &mut i) {
                Some(p) => cur = p,
                None => break,
            },
            b'|' | b'/' => match pt(b, &mut i) {
                Some(p) => {
                    out.push(Seg { from: cur, ctrl: None, to: p });
                    cur = p;
                }
                None => break,
            },
            b'[' | b']' => match (pt(b, &mut i), pt(b, &mut i)) {
                (Some(q), Some(p)) => {
                    out.push(Seg { from: cur, ctrl: Some(q), to: p });
                    cur = p;
                }
                _ => break,
            },
            b'S' => {
                while b.get(i).is_some_and(u8::is_ascii_digit) {
                    i += 1;
                }
            }
            c if c.is_ascii_whitespace() => {}
            _ => break,
        }
        if out.len() >= MAX_SEGS {
            break;
        }
    }
    out
}

/// Hash key for an end point (to 1/100 px, which absorbs the format's rounding).
fn key(p: Vec2) -> (i64, i64) {
    ((p.x * 100.0).round() as i64, (p.y * 100.0).round() as i64)
}

fn push_seg(path: &mut Path, s: &Seg) {
    match s.ctrl {
        Some(q) => path.quad_to(q, s.to),
        None => path.line_to(s.to),
    };
}

/// The region of fill style `style`, as closed loops.
pub fn fill_path(edges: &[Edge], style: u32) -> Path {
    let mut segs: Vec<Seg> = Vec::new();
    for e in edges {
        if e.fill_right == style {
            segs.extend(e.segs.iter().copied());
        }
        if e.fill_left == style {
            segs.extend(e.segs.iter().rev().map(|s| s.reversed()));
        }
    }
    let mut by_start: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, s) in segs.iter().enumerate() {
        // Zero-length segments carry no area and would confuse the chaining.
        if key(s.from) != key(s.to) {
            by_start.entry(key(s.from)).or_default().push(i);
        }
    }
    let mut used = vec![false; segs.len()];
    let mut path = Path::new();
    for first in 0..segs.len() {
        if used.get(first).copied().unwrap_or(true) || segs.get(first).is_some_and(|s| key(s.from) == key(s.to)) {
            continue;
        }
        let Some(&start) = segs.get(first) else { continue };
        path.move_to(start.from);
        let mut at = first;
        while let Some(s) = segs.get(at).copied() {
            if let Some(u) = used.get_mut(at) {
                *u = true;
            }
            push_seg(&mut path, &s);
            if key(s.to) == key(start.from) {
                break;
            }
            let next = by_start.get(&key(s.to)).and_then(|v| v.iter().copied().find(|&j| !used.get(j).copied().unwrap_or(true)));
            match next {
                Some(j) => at = j,
                None => break,
            }
        }
        path.close();
    }
    path
}

/// The stroked edges of stroke style `style`, in drawing order.
pub fn stroke_path(edges: &[Edge], style: u32) -> Path {
    let mut path = Path::new();
    let mut last: Option<Vec2> = None;
    for e in edges.iter().filter(|e| e.stroke == style) {
        for s in &e.segs {
            if last.is_none_or(|l| key(l) != key(s.from)) {
                path.move_to(s.from);
            }
            push_seg(&mut path, s);
            last = Some(s.to);
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_commands_and_numbers() {
        let s = parse("!0 0|200 0[400 0 400 200S2/0 200!#A.8 -20");
        assert_eq!(s.len(), 3);
        assert_eq!(s[0].to, Vec2::new(10.0, 0.0), "twips to pixels");
        assert_eq!(s[1].ctrl, Some(Vec2::new(20.0, 0.0)));
        assert_eq!(s[2], Seg { from: Vec2::new(20.0, 10.0), ctrl: None, to: Vec2::new(0.0, 10.0) });
        // Hex fixed point: #A.8 = 10.5 twips; a negative two's-complement value.
        let mut i = 0;
        assert_eq!(number(b"#A.8", &mut i), Some(10.5));
        let mut i = 0;
        assert_eq!(number(b"#FFFFFF.80", &mut i), Some(-0.5));
        assert!(parse("!1 2|x").is_empty(), "stops at garbage, never panics");
    }

    #[test]
    fn a_square_split_into_edges_with_fills_on_either_side() {
        // Top and right drawn clockwise with the fill on the right; bottom and left drawn the
        // other way with the fill on the left. Both describe the same square.
        let e = |l, r, d: &str| Edge { fill_left: l, fill_right: r, stroke: 1, segs: parse(d) };
        let edges = vec![e(0, 1, "!0 0|200 0|200 200"), e(1, 0, "!0 0|0 200|200 200")];
        let p = fill_path(&edges, 1);
        let lines = p.flatten(&anicraft_geom::Affine::IDENTITY, 0.1);
        assert_eq!(lines.len(), 1, "one closed loop");
        assert!(lines[0].closed);
        assert!(p.contains(Vec2::new(5.0, 5.0), &anicraft_geom::Affine::IDENTITY, anicraft_geom::FillRule::EvenOdd));
        assert!(!p.contains(Vec2::new(15.0, 5.0), &anicraft_geom::Affine::IDENTITY, anicraft_geom::FillRule::EvenOdd));
        let st = stroke_path(&edges, 1);
        assert_eq!(st.anchors().len(), 6, "two runs of two segments each, plus their starts");
        assert!(fill_path(&edges, 7).is_empty());
    }
}
