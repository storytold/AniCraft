//! Full SVG path data (SVG 1.1 §8.3 / SVG 2 "paths"): every command in absolute and relative
//! form (M L H V C S Q T A Z), implicit repeats, packed numbers and arc flags.
//!
//! [`crate::Path::from_svg`] stays the strict parser for AniCraft's own file format; this one
//! reads SVG written by other tools (icon sets now, SVG import later). Elliptical arcs become
//! cubic Béziers using the endpoint-to-centre conversion of the SVG implementation notes
//! (W3C SVG 1.1 Appendix F.6, Royalty-Free Patent Policy), at most a quarter turn per cubic.
//! Never panics; non-finite numbers and oversized paths are errors.

use crate::path::MAX_PATH_CMDS;
use crate::{Path, PathCmd, Vec2};

struct Cursor<'a> {
    s: &'a [u8],
    i: usize,
}

impl Cursor<'_> {
    fn skip(&mut self) {
        while self.s.get(self.i).is_some_and(|c| c.is_ascii_whitespace() || *c == b',') {
            self.i += 1;
        }
    }
    fn letter(&mut self) -> Option<u8> {
        self.skip();
        self.s.get(self.i).copied().filter(u8::is_ascii_alphabetic).filter(|c| !matches!(c, b'e' | b'E'))
    }
    fn number(&mut self) -> Result<f64, String> {
        self.skip();
        let start = self.i;
        let at = |i: usize| self.s.get(i).copied();
        let mut i = self.i;
        if matches!(at(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let mut digits = false;
        while at(i).is_some_and(|c| c.is_ascii_digit()) {
            i += 1;
            digits = true;
        }
        if at(i) == Some(b'.') {
            i += 1;
            while at(i).is_some_and(|c| c.is_ascii_digit()) {
                i += 1;
                digits = true;
            }
        }
        if !digits {
            return Err(format!("expected a number at byte {start}"));
        }
        if matches!(at(i), Some(b'e' | b'E')) {
            let mut j = i + 1;
            if matches!(at(j), Some(b'+' | b'-')) {
                j += 1;
            }
            if at(j).is_some_and(|c| c.is_ascii_digit()) {
                while at(j).is_some_and(|c| c.is_ascii_digit()) {
                    j += 1;
                }
                i = j;
            }
        }
        let tok = std::str::from_utf8(self.s.get(start..i).unwrap_or_default()).unwrap_or("");
        let v: f64 = tok.parse().map_err(|_| format!("bad number '{tok}'"))?;
        if !v.is_finite() {
            return Err(format!("number '{tok}' is not finite"));
        }
        self.i = i;
        Ok(v)
    }
    /// An arc flag: a single `0` or `1`, which may be packed against the next number.
    fn flag(&mut self) -> Result<bool, String> {
        self.skip();
        match self.s.get(self.i) {
            Some(b'0') => {
                self.i += 1;
                Ok(false)
            }
            Some(b'1') => {
                self.i += 1;
                Ok(true)
            }
            _ => Err(format!("expected an arc flag (0 or 1) at byte {}", self.i)),
        }
    }
    fn done(&mut self) -> bool {
        self.skip();
        self.i >= self.s.len()
    }
}

#[derive(Clone, Copy)]
struct P(f64, f64);

impl P {
    fn v(self) -> Vec2 {
        Vec2::new(self.0 as f32, self.1 as f32)
    }
}

/// Parses SVG path data (`d` attribute) into a [`Path`].
pub fn parse_path_data(d: &str) -> Result<Path, String> {
    let mut c = Cursor { s: d.as_bytes(), i: 0 };
    let mut path = Path::new();
    let (mut cur, mut start) = (P(0.0, 0.0), P(0.0, 0.0));
    // The last control point of a C/S (for S) or Q/T (for T), for reflection.
    let (mut last_cubic, mut last_quad): (Option<P>, Option<P>) = (None, None);
    let mut cmd: Option<u8> = None;
    while !c.done() {
        if let Some(l) = c.letter() {
            c.i += 1;
            cmd = Some(l);
        }
        let Some(k) = cmd else { return Err("path data must start with a command".into()) };
        let rel = k.is_ascii_lowercase();
        let off = |p: P, cur: P| if rel { P(p.0 + cur.0, p.1 + cur.1) } else { p };
        let pt = |c: &mut Cursor| -> Result<P, String> { Ok(P(c.number()?, c.number()?)) };
        let (mut nc, mut nq) = (None, None);
        match k.to_ascii_uppercase() {
            b'M' => {
                let p = off(pt(&mut c)?, cur);
                path.move_to(p.v());
                cur = p;
                start = p;
                // Further pairs after a moveto are linetos.
                cmd = Some(if rel { b'l' } else { b'L' });
            }
            b'L' => {
                cur = off(pt(&mut c)?, cur);
                path.line_to(cur.v());
            }
            b'H' => {
                let x = c.number()?;
                cur = P(if rel { cur.0 + x } else { x }, cur.1);
                path.line_to(cur.v());
            }
            b'V' => {
                let y = c.number()?;
                cur = P(cur.0, if rel { cur.1 + y } else { y });
                path.line_to(cur.v());
            }
            b'C' => {
                let (c1, c2, p) = (off(pt(&mut c)?, cur), off(pt(&mut c)?, cur), off(pt(&mut c)?, cur));
                path.cubic_to(c1.v(), c2.v(), p.v());
                nc = Some(c2);
                cur = p;
            }
            b'S' => {
                let c1 = last_cubic.map_or(cur, |l| P(2.0 * cur.0 - l.0, 2.0 * cur.1 - l.1));
                let (c2, p) = (off(pt(&mut c)?, cur), off(pt(&mut c)?, cur));
                path.cubic_to(c1.v(), c2.v(), p.v());
                nc = Some(c2);
                cur = p;
            }
            b'Q' => {
                let (q, p) = (off(pt(&mut c)?, cur), off(pt(&mut c)?, cur));
                path.quad_to(q.v(), p.v());
                nq = Some(q);
                cur = p;
            }
            b'T' => {
                let q = last_quad.map_or(cur, |l| P(2.0 * cur.0 - l.0, 2.0 * cur.1 - l.1));
                let p = off(pt(&mut c)?, cur);
                path.quad_to(q.v(), p.v());
                nq = Some(q);
                cur = p;
            }
            b'A' => {
                let (rx, ry, rot) = (c.number()?, c.number()?, c.number()?);
                let (large, sweep) = (c.flag()?, c.flag()?);
                let p = off(pt(&mut c)?, cur);
                arc_to(&mut path, cur, rx, ry, rot, large, sweep, p);
                cur = p;
            }
            b'Z' => {
                path.close();
                cur = start;
                // Numbers may not follow Z without a new command.
                cmd = None;
            }
            other => return Err(format!("unknown path command '{}'", other as char)),
        }
        last_cubic = nc;
        last_quad = nq;
        if path.cmds.len() > MAX_PATH_CMDS {
            return Err(format!("path has more than {MAX_PATH_CMDS} commands"));
        }
    }
    // Coordinates are stored as f32: a finite f64 can still overflow to infinity there.
    let finite = |v: &Vec2| v.x.is_finite() && v.y.is_finite();
    let ok = path.cmds.iter().all(|cmd| match cmd {
        PathCmd::MoveTo(a) | PathCmd::LineTo(a) => finite(a),
        PathCmd::QuadTo(a, b) => finite(a) && finite(b),
        PathCmd::CubicTo(a, b, e) => finite(a) && finite(b) && finite(e),
        PathCmd::Close => true,
    });
    if !ok {
        return Err("path coordinate is not finite as f32".into());
    }
    Ok(path)
}

/// Appends the SVG elliptical arc from `p0` to `p1` as cubics (F.6.5 / F.6.6).
#[allow(clippy::too_many_arguments)]
fn arc_to(path: &mut Path, p0: P, rx: f64, ry: f64, rot_deg: f64, large: bool, sweep: bool, p1: P) {
    if (p0.0 - p1.0).abs() < 1e-12 && (p0.1 - p1.1).abs() < 1e-12 {
        return;
    }
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx < 1e-12 || ry < 1e-12 {
        path.line_to(p1.v());
        return;
    }
    let (sin, cos) = rot_deg.to_radians().sin_cos();
    let (dx, dy) = ((p0.0 - p1.0) * 0.5, (p0.1 - p1.1) * 0.5);
    let (x1, y1) = (cos * dx + sin * dy, -sin * dx + cos * dy);
    // Radii too small to reach: scale them up (F.6.6).
    let lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if lambda > 1.0 {
        let s = lambda.sqrt();
        rx *= s;
        ry *= s;
    }
    let num = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut coef = if den > 0.0 { (num / den).max(0.0).sqrt() } else { 0.0 };
    if large == sweep {
        coef = -coef;
    }
    let (cxp, cyp) = (coef * rx * y1 / ry, -coef * ry * x1 / rx);
    let (cx, cy) = (cos * cxp - sin * cyp + (p0.0 + p1.0) * 0.5, sin * cxp + cos * cyp + (p0.1 + p1.1) * 0.5);
    let angle = |ux: f64, uy: f64, vx: f64, vy: f64| (ux * vy - uy * vx).atan2(ux * vx + uy * vy);
    let (ux, uy) = ((x1 - cxp) / rx, (y1 - cyp) / ry);
    let (vx, vy) = ((-x1 - cxp) / rx, (-y1 - cyp) / ry);
    let theta1 = angle(1.0, 0.0, ux, uy);
    let mut delta = angle(ux, uy, vx, vy);
    let tau = std::f64::consts::TAU;
    if !sweep && delta > 0.0 {
        delta -= tau;
    } else if sweep && delta < 0.0 {
        delta += tau;
    }
    let n = (delta.abs() / std::f64::consts::FRAC_PI_2).ceil().clamp(1.0, 4.0) as usize;
    let step = delta / n as f64;
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    // A point / tangent on the unit circle, mapped onto the rotated ellipse.
    let map = |x: f64, y: f64| P(cx + rx * cos * x - ry * sin * y, cy + rx * sin * x + ry * cos * y);
    let mut t = theta1;
    for i in 0..n {
        let (s0, c0) = t.sin_cos();
        let t1 = t + step;
        let (s1, c1) = t1.sin_cos();
        let a = map(c0 - k * s0, s0 + k * c0);
        let b = map(c1 + k * s1, s1 - k * c1);
        // Land exactly on the requested end point.
        let e = if i + 1 == n { p1 } else { map(c1, s1) };
        path.cubic_to(a.v(), b.v(), e.v());
        t = t1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PathCmd;

    fn ends(p: &Path) -> Vec<Vec2> {
        p.anchors().into_iter().map(|(_, a)| a).collect()
    }

    #[test]
    fn relative_and_shorthand_commands() {
        let p = parse_path_data("m10 10 h5 v5 H10 z m 1,1 l2-2").unwrap();
        assert_eq!(ends(&p), vec![Vec2::new(10.0, 10.0), Vec2::new(15.0, 10.0), Vec2::new(15.0, 15.0), Vec2::new(10.0, 15.0), Vec2::new(11.0, 11.0), Vec2::new(13.0, 9.0)]);
        assert!(matches!(p.cmds[4], PathCmd::Close));
        // Implicit lineto after moveto, packed negative numbers.
        let q = parse_path_data("M0 0 10 0-5.5.5").unwrap();
        assert_eq!(ends(&q), vec![Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0), Vec2::new(-5.5, 0.5)]);
        // S reflects the previous control point.
        let s = parse_path_data("M0 0 C 0 10 10 10 10 0 S 20 -10 20 0").unwrap();
        assert!(matches!(s.cmds[2], PathCmd::CubicTo(c1, _, _) if (c1 - Vec2::new(10.0, -10.0)).length() < 1e-5));
    }

    #[test]
    fn arcs_land_on_the_ellipse() {
        // A half circle of radius 5 from (0,0) to (10,0), sweeping through (5,-5) or (5,5).
        for (sweep, mid_y) in [("1", -5.0), ("0", 5.0)] {
            let p = parse_path_data(&format!("M0 0 A5 5 0 0 {sweep} 10 0")).unwrap();
            let lines = p.flatten(&crate::Affine::IDENTITY, 0.01);
            let pts = &lines[0].points;
            assert!(pts.iter().all(|q| (q.distance(Vec2::new(5.0, 0.0)) - 5.0).abs() < 0.05), "on the circle");
            assert!(pts.iter().any(|q| (q.y - mid_y).abs() < 0.05), "through the right side ({sweep})");
            assert_eq!(pts.last().copied(), Some(Vec2::new(10.0, 0.0)));
        }
        // Packed flags as icon sets write them, and radii too small get scaled up.
        let p = parse_path_data("M2 2a1 1 0 011 1a.1.1 0 0 0 4 0").unwrap();
        assert_eq!(ends(&p).last().copied(), Some(Vec2::new(7.0, 3.0)));
        // A real icon outline (rounded square corner arcs).
        let lock = parse_path_data("M7 11V7a5 5 0 0 1 10 0v4").unwrap();
        assert_eq!(ends(&lock).last().copied(), Some(Vec2::new(17.0, 11.0)));
    }

    #[test]
    fn numbers_overflowing_f32_are_errors() {
        for bad in ["M0 0 L1e39 0", "M0 0 v , 3.4028236692e38 z", "M0 0 l3e38 0 3e38 0"] {
            assert!(parse_path_data(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn bad_input_is_an_error() {
        for bad in ["", "10 10", "M", "M 1", "M0 0 A1 1 0 2 0 3 3", "M0 0 Z 4 4", "M0 0 L1e999 0", "M0 0 X1 1"] {
            if bad.is_empty() {
                assert!(parse_path_data(bad).unwrap().is_empty());
                continue;
            }
            assert!(parse_path_data(bad).is_err(), "{bad}");
        }
    }
}
