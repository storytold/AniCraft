//! Signed-area coverage accumulation rasterizer.
//!
//! Each edge deposits the signed area it sweeps into a per-pixel accumulation buffer; a running
//! sum along a row then yields exact analytic coverage (no supersampling). The buffer covers only
//! the shape's pixel bounding box. Edges left of the box collapse onto its left edge, which keeps
//! the winding of every pixel to their right correct.

use anicraft_geom::{FillRule, Vec2};

/// Integer pixel rectangle `[x0, x1) × [y0, y1)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IRect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl IRect {
    pub fn width(&self) -> usize {
        (self.x1 - self.x0).max(0) as usize
    }
    pub fn height(&self) -> usize {
        (self.y1 - self.y0).max(0) as usize
    }
    pub fn is_empty(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }
    pub fn intersect(&self, o: &IRect) -> IRect {
        IRect { x0: self.x0.max(o.x0), y0: self.y0.max(o.y0), x1: self.x1.min(o.x1), y1: self.y1.min(o.y1) }
    }
    /// Smallest rectangle holding both (an empty side is ignored).
    pub fn union(&self, o: &IRect) -> IRect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        IRect { x0: self.x0.min(o.x0), y0: self.y0.min(o.y0), x1: self.x1.max(o.x1), y1: self.y1.max(o.y1) }
    }
}

/// Coverage accumulation buffer over a pixel rectangle.
pub struct Coverage {
    pub bounds: IRect,
    stride: usize,
    acc: Vec<f32>,
    /// The buffer holds coverage itself (hairlines, ADR 0048), not area to sum along rows.
    direct: bool,
}

impl Coverage {
    pub fn new(bounds: IRect) -> Coverage {
        let stride = bounds.width() + 2;
        Coverage { bounds, stride, acc: vec![0.0; stride * bounds.height()], direct: false }
    }

    /// Re-targets the buffer at `bounds`, reusing its allocation (one buffer per render band).
    pub fn reset(&mut self, bounds: IRect) {
        self.direct = false;
        self.bounds = bounds;
        self.stride = bounds.width() + 2;
        self.acc.clear();
        self.acc.resize(self.stride * bounds.height(), 0.0);
    }

    /// Adds one edge, in pixel coordinates of the target surface.
    pub fn add_line(&mut self, a: Vec2, b: Vec2) {
        if !(a.is_finite() && b.is_finite()) {
            return;
        }
        let o = Vec2::new(self.bounds.x0 as f32, self.bounds.y0 as f32);
        let (a, b) = (a - o, b - o);
        // Horizontal, or wholly above or below the buffer (common: a band sees every edge of
        // the shapes crossing it): nothing to deposit, so skip the splitting below.
        if a.y == b.y || a.y.max(b.y) <= 0.0 || a.y.min(b.y) >= self.bounds.height() as f32 {
            return;
        }
        let w = self.bounds.width() as f32;
        // Split at x = 0 and x = w so every piece lies fully inside or outside [0, w].
        let mut ts = [0.0f32, 1.0, 1.0, 1.0];
        let mut n = 1;
        let dx = b.x - a.x;
        if dx != 0.0 {
            for edge in [0.0, w] {
                let t = (edge - a.x) / dx;
                if t > 0.0 && t < 1.0 {
                    ts[n] = t;
                    n += 1;
                }
            }
        }
        ts[n] = 1.0;
        let (head, _) = ts.split_at_mut(n + 1);
        head.sort_by(f32::total_cmp);
        let clamp = |p: Vec2| Vec2::new(p.x.clamp(0.0, w), p.y);
        for win in head.windows(2) {
            let (t0, t1) = (win[0], win[1]);
            if t1 <= t0 {
                continue;
            }
            self.raw_line(clamp(a.lerp(b, t0)), clamp(a.lerp(b, t1)));
        }
    }

    #[inline]
    fn deposit(&mut self, row: usize, xi: i64, v: f32) {
        if xi >= 0
            && (xi as usize) < self.stride
            && let Some(cell) = self.acc.get_mut(row + xi as usize)
        {
            *cell += v;
        }
    }

    fn raw_line(&mut self, p0: Vec2, p1: Vec2) {
        if (p0.y - p1.y).abs() < 1e-9 {
            return;
        }
        let (dir, p0, p1) = if p0.y < p1.y { (1.0f32, p0, p1) } else { (-1.0f32, p1, p0) };
        let h = self.bounds.height();
        let hf = h as f32;
        if p1.y <= 0.0 || p0.y >= hf {
            return;
        }
        let dxdy = (p1.x - p0.x) / (p1.y - p0.y);
        let mut x = p0.x;
        let ystart = if p0.y < 0.0 {
            x -= p0.y * dxdy;
            0
        } else {
            p0.y as usize
        };
        let yend = (p1.y.ceil() as usize).min(h);
        for y in ystart..yend {
            let row = y * self.stride;
            let dy = ((y + 1) as f32).min(p1.y) - (y as f32).max(p0.y);
            let xnext = x + dxdy * dy;
            let d = dy * dir;
            let (x0, x1) = if x < xnext { (x, xnext) } else { (xnext, x) };
            let x0floor = x0.floor();
            let x0i = x0floor as i64;
            let x1ceil = x1.ceil();
            let x1i = x1ceil as i64;
            if x1i <= x0i + 1 {
                let xmf = 0.5 * (x + xnext) - x0floor;
                self.deposit(row, x0i, d - d * xmf);
                self.deposit(row, x0i + 1, d * xmf);
            } else {
                let s = 1.0 / (x1 - x0);
                let x0f = x0 - x0floor;
                let a0 = 0.5 * s * (1.0 - x0f) * (1.0 - x0f);
                let x1f = x1 - x1ceil + 1.0;
                let am = 0.5 * s * x1f * x1f;
                self.deposit(row, x0i, d * a0);
                if x1i == x0i + 2 {
                    self.deposit(row, x0i + 1, d * (1.0 - a0 - am));
                } else {
                    let a1 = s * (1.5 - x0f);
                    self.deposit(row, x0i + 1, d * (a1 - a0));
                    for xi in x0i + 2..x1i - 1 {
                        self.deposit(row, xi, d * s);
                    }
                    let a2 = a1 + (x1i - x0i - 3) as f32 * s;
                    self.deposit(row, x1i - 1, d * (1.0 - a2 - am));
                }
                self.deposit(row, x1i, d * am);
            }
            x = xnext;
        }
    }

    /// Adds a closed polygon (the closing edge is implicit).
    pub fn add_polygon(&mut self, pts: &[Vec2]) {
        let n = pts.len();
        if n < 2 {
            return;
        }
        for i in 0..n {
            let (Some(&a), Some(&b)) = (pts.get(i), pts.get((i + 1) % n)) else { continue };
            self.add_line(a, b);
        }
    }

    /// Re-targets the buffer at `bounds` for hairlines ([`Coverage::add_hairline`]): it then
    /// holds each pixel's coverage, and [`Coverage::resolve_row`] reads it as it is.
    pub fn reset_direct(&mut self, bounds: IRect) {
        self.direct = true;
        self.bounds = bounds;
        self.stride = bounds.width();
        self.acc.clear();
        self.acc.resize(self.stride * bounds.height(), 0.0);
    }

    /// Adds a hairline along an open polyline ([`crate::hairline`]); the buffer must be in
    /// direct mode ([`Coverage::reset_direct`]).
    pub fn add_hairline(&mut self, pts: &[Vec2]) {
        if self.direct {
            crate::hairline::add_polyline(&mut self.acc, self.bounds, pts);
        }
    }

    /// Resolves row `y` (relative to the bounds) into coverage values in `out` (len = width).
    pub fn resolve_row(&self, y: usize, rule: FillRule, out: &mut [f32]) {
        let start = y * self.stride;
        let Some(row) = self.acc.get(start..start + self.stride) else {
            out.fill(0.0);
            return;
        };
        if self.direct {
            for (o, a) in out.iter_mut().zip(row) {
                *o = *a;
            }
            return;
        }
        let mut sum = 0.0f32;
        for (o, a) in out.iter_mut().zip(row) {
            sum += a;
            *o = fold(sum, rule);
        }
    }
}

#[inline]
fn fold(winding: f32, rule: FillRule) -> f32 {
    match rule {
        FillRule::NonZero => winding.abs().min(1.0),
        FillRule::EvenOdd => {
            let t = winding.abs() % 2.0;
            if t > 1.0 { 2.0 - t } else { t }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn total(c: &Coverage, rule: FillRule) -> f32 {
        let mut row = vec![0.0; c.bounds.width()];
        let mut s = 0.0;
        for y in 0..c.bounds.height() {
            c.resolve_row(y, rule, &mut row);
            s += row.iter().sum::<f32>();
        }
        s
    }

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<Vec2> {
        vec![Vec2::new(x0, y0), Vec2::new(x1, y0), Vec2::new(x1, y1), Vec2::new(x0, y1)]
    }

    #[test]
    fn rect_coverage_is_exact() {
        let mut c = Coverage::new(IRect { x0: 0, y0: 0, x1: 20, y1: 20 });
        c.add_polygon(&rect(2.25, 3.5, 12.75, 9.0));
        let area = 10.5 * 5.5;
        assert!((total(&c, FillRule::NonZero) - area).abs() < 1e-3);
    }

    #[test]
    fn partial_pixel_is_fractional() {
        let mut c = Coverage::new(IRect { x0: 0, y0: 0, x1: 4, y1: 1 });
        c.add_polygon(&rect(1.0, 0.0, 2.5, 1.0));
        let mut row = vec![0.0; 4];
        c.resolve_row(0, FillRule::NonZero, &mut row);
        assert!((row[0]).abs() < 1e-5);
        assert!((row[1] - 1.0).abs() < 1e-5);
        assert!((row[2] - 0.5).abs() < 1e-5);
        assert!((row[3]).abs() < 1e-5);
    }

    #[test]
    fn shapes_left_of_bounds_keep_winding() {
        // Rectangle starting far left of the buffer: pixels inside must be fully covered.
        let mut c = Coverage::new(IRect { x0: 10, y0: 0, x1: 20, y1: 2 });
        c.add_polygon(&rect(-100.0, 0.0, 15.0, 2.0));
        let mut row = vec![0.0; 10];
        c.resolve_row(0, FillRule::NonZero, &mut row);
        assert!(row[..5].iter().all(|v| (v - 1.0).abs() < 1e-4), "{row:?}");
        assert!(row[5..].iter().all(|v| v.abs() < 1e-4), "{row:?}");
    }

    #[test]
    fn even_odd_hole() {
        let mut c = Coverage::new(IRect { x0: 0, y0: 0, x1: 10, y1: 10 });
        c.add_polygon(&rect(0.0, 0.0, 10.0, 10.0));
        c.add_polygon(&rect(2.0, 2.0, 8.0, 8.0));
        assert!((total(&c, FillRule::EvenOdd) - 64.0).abs() < 1e-2);
        assert!((total(&c, FillRule::NonZero) - 100.0).abs() < 1e-2);
    }

    #[test]
    fn non_finite_edges_ignored() {
        let mut c = Coverage::new(IRect { x0: 0, y0: 0, x1: 4, y1: 4 });
        c.add_line(Vec2::new(f32::NAN, 0.0), Vec2::new(1.0, 3.0));
        c.add_line(Vec2::new(0.0, -1e30), Vec2::new(1e30, 1e30));
        assert!(total(&c, FillRule::NonZero).is_finite());
    }
}
