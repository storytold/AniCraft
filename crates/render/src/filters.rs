//! Filters on a group's composited pixels: premultiplied RGBA, `w × h`, row by row.
//!
//! Blur is repeated box blurs (one per quality step; three approach a Gaussian). Drop Shadow and
//! Glow blur the artwork's opacity, offset it and colour it, then put it under the artwork
//! (outer), or inside it over the artwork (inner); Knockout keeps only the shadow or glow.
//! Adjust Color is a colour matrix built from the standard matrix operations for brightness,
//! contrast, saturation and hue rotation (Haeberli, "Matrix Operations for Image Processing",
//! 1993), applied to unpremultiplied colour.

use crate::paint::Premul;
use anicraft_doc::{Filter, Rgba};

/// Applies `filters` in order. Sizes are stage pixels, multiplied by `scale` (the view's).
pub(crate) fn apply(buf: &mut [Premul], w: usize, h: usize, filters: &[Filter], scale: f32) {
    if buf.len() != w * h || w == 0 || h == 0 {
        return;
    }
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    for f in filters {
        match f.sanitized() {
            Filter::Blur { blur_x, blur_y, quality } => blur(buf, w, h, blur_x * scale, blur_y * scale, quality),
            Filter::DropShadow { blur_x, blur_y, strength, quality, angle, distance, color, inner, knockout, hide_object } => {
                let a = angle.to_radians();
                let (dx, dy) = ((distance * scale * a.cos()).round() as i64, (distance * scale * a.sin()).round() as i64);
                shadow(buf, w, h, Shadow { blur_x: blur_x * scale, blur_y: blur_y * scale, strength, quality, dx, dy, color, inner, knockout: knockout || (hide_object && inner), hide_object });
            }
            Filter::Glow { blur_x, blur_y, strength, quality, color, inner, knockout } => {
                shadow(buf, w, h, Shadow { blur_x: blur_x * scale, blur_y: blur_y * scale, strength, quality, dx: 0, dy: 0, color, inner, knockout, hide_object: false });
            }
            Filter::AdjustColor { brightness, contrast, saturation, hue } => adjust(buf, &color_matrix(brightness, contrast, saturation, hue)),
        }
    }
}

/// Box blur of width `n` along rows (`stride` 1) or columns, `passes` times, of any number of
/// channels per pixel. Outside the buffer is transparent.
fn box_pass<const C: usize>(buf: &mut [[f32; C]], w: usize, h: usize, n: usize, horizontal: bool, tmp: &mut Vec<[f32; C]>) {
    if n <= 1 {
        return;
    }
    let (len, lines) = if horizontal { (w, h) } else { (h, w) };
    let lo = n / 2;
    let hi = n - 1 - lo;
    let inv = 1.0 / n as f32;
    let idx = |line: usize, i: usize| if horizontal { line * w + i } else { i * w + line };
    tmp.clear();
    tmp.resize(len, [0.0; C]);
    for line in 0..lines {
        for (i, t) in tmp.iter_mut().enumerate() {
            if let Some(v) = buf.get(idx(line, i)) {
                *t = *v;
            }
        }
        let mut sum = [0.0f32; C];
        // Window [i - lo, i + hi]; start with [-lo, hi] for i = 0.
        for v in tmp.iter().take(hi.min(len.saturating_sub(1)) + 1) {
            for c in 0..C {
                sum[c] += v[c];
            }
        }
        for i in 0..len {
            if let Some(out) = buf.get_mut(idx(line, i)) {
                for c in 0..C {
                    out[c] = sum[c] * inv;
                }
            }
            if let Some(add) = tmp.get(i + hi + 1) {
                for c in 0..C {
                    sum[c] += add[c];
                }
            }
            if i >= lo
                && let Some(sub) = tmp.get(i - lo)
            {
                for c in 0..C {
                    sum[c] -= sub[c];
                }
            }
        }
    }
}

fn blur_channels<const C: usize>(buf: &mut [[f32; C]], w: usize, h: usize, bx: f32, by: f32, passes: u8) {
    let (nx, ny) = (bx.round().max(0.0) as usize, by.round().max(0.0) as usize);
    let mut tmp = Vec::new();
    for _ in 0..passes.max(1) {
        box_pass(buf, w, h, nx, true, &mut tmp);
        box_pass(buf, w, h, ny, false, &mut tmp);
    }
}

fn blur(buf: &mut [Premul], w: usize, h: usize, bx: f32, by: f32, passes: u8) {
    blur_channels(buf, w, h, bx, by, passes);
}

struct Shadow {
    blur_x: f32,
    blur_y: f32,
    strength: f32,
    quality: u8,
    dx: i64,
    dy: i64,
    color: Rgba,
    inner: bool,
    knockout: bool,
    hide_object: bool,
}

fn shadow(buf: &mut [Premul], w: usize, h: usize, s: Shadow) {
    // The shadow's coverage: the artwork's opacity (or, inside, its transparency), offset.
    let mut cov: Vec<[f32; 1]> = vec![[if s.inner { 1.0 } else { 0.0 }]; w * h];
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            let (sx, sy) = (x - s.dx, y - s.dy);
            let a = if sx >= 0 && sy >= 0 && (sx as usize) < w && (sy as usize) < h { buf.get(sy as usize * w + sx as usize).map_or(0.0, |p| p[3]) } else { 0.0 };
            if let Some(c) = cov.get_mut(y as usize * w + x as usize) {
                c[0] = if s.inner { 1.0 - a } else { a };
            }
        }
    }
    blur_channels(&mut cov, w, h, s.blur_x, s.blur_y, s.quality);
    let c = s.color.sanitized();
    for (p, k) in buf.iter_mut().zip(&cov) {
        let sa = (k[0] * s.strength).clamp(0.0, 1.0) * c.a;
        let src = *p;
        let ba = src[3];
        *p = if s.inner {
            // Inside the artwork, over it.
            let a = sa * ba;
            if s.knockout { [c.r * a, c.g * a, c.b * a, a] } else { [c.r * a + src[0] * (1.0 - sa), c.g * a + src[1] * (1.0 - sa), c.b * a + src[2] * (1.0 - sa), ba] }
        } else if s.hide_object {
            [c.r * sa, c.g * sa, c.b * sa, sa]
        } else if s.knockout {
            let a = sa * (1.0 - ba);
            [c.r * a, c.g * a, c.b * a, a]
        } else {
            // Under the artwork.
            let k = 1.0 - ba;
            [src[0] + c.r * sa * k, src[1] + c.g * sa * k, src[2] + c.b * sa * k, ba + sa * k]
        };
    }
}

/// A 4 × 5 colour matrix (rows r, g, b, a; columns r, g, b, a, offset), straight colour 0..1.
type Matrix = [[f32; 5]; 4];

const IDENTITY: Matrix = [[1.0, 0.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0, 0.0]];

/// `a` applied after `b`.
fn mul(a: &Matrix, b: &Matrix) -> Matrix {
    let mut out = [[0.0; 5]; 4];
    for (i, row) in out.iter_mut().enumerate() {
        for j in 0..5 {
            let mut v: f32 = (0..4).map(|k| a[i][k] * b[k][j]).sum();
            if j == 4 {
                v += a[i][4];
            }
            row[j] = v;
        }
    }
    out
}

/// Brightness, contrast and saturation in −100..=100, hue in degrees.
pub(crate) fn color_matrix(brightness: f32, contrast: f32, saturation: f32, hue: f32) -> Matrix {
    let mut m = IDENTITY;
    // Brightness: an offset (100 adds the full range).
    let b = brightness / 100.0;
    m = mul(&[[1.0, 0.0, 0.0, 0.0, b], [0.0, 1.0, 0.0, 0.0, b], [0.0, 0.0, 1.0, 0.0, b], [0.0, 0.0, 0.0, 1.0, 0.0]], &m);
    // Contrast: scale about middle grey (−100 flattens to grey, +100 is ×10).
    let c = if contrast < 0.0 { 1.0 + contrast / 100.0 } else { 1.0 + contrast / 100.0 * 9.0 };
    let o = 0.5 * (1.0 - c);
    m = mul(&[[c, 0.0, 0.0, 0.0, o], [0.0, c, 0.0, 0.0, o], [0.0, 0.0, c, 0.0, o], [0.0, 0.0, 0.0, 1.0, 0.0]], &m);
    // Saturation: towards or away from luminance (Haeberli's weights).
    let s = if saturation < 0.0 { 1.0 + saturation / 100.0 } else { 1.0 + saturation / 100.0 * 2.0 };
    let (lr, lg, lb) = (0.3086, 0.6094, 0.0820);
    let k = 1.0 - s;
    m = mul(&[[k * lr + s, k * lg, k * lb, 0.0, 0.0], [k * lr, k * lg + s, k * lb, 0.0, 0.0], [k * lr, k * lg, k * lb + s, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0, 0.0]], &m);
    // Hue: rotation about the grey axis, keeping luminance.
    if hue != 0.0 {
        let (sin, cos) = hue.to_radians().sin_cos();
        let (lr, lg, lb) = (0.213, 0.715, 0.072);
        let r = [
            [lr + cos * (1.0 - lr) + sin * -lr, lg + cos * -lg + sin * -lg, lb + cos * -lb + sin * (1.0 - lb), 0.0, 0.0],
            [lr + cos * -lr + sin * 0.143, lg + cos * (1.0 - lg) + sin * 0.140, lb + cos * -lb + sin * -0.283, 0.0, 0.0],
            [lr + cos * -lr + sin * -(1.0 - lr), lg + cos * -lg + sin * lg, lb + cos * (1.0 - lb) + sin * lb, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0, 0.0],
        ];
        m = mul(&r, &m);
    }
    m
}

fn adjust(buf: &mut [Premul], m: &Matrix) {
    for p in buf.iter_mut() {
        let a = p[3];
        if a <= 1e-6 {
            continue;
        }
        let (r, g, b) = (p[0] / a, p[1] / a, p[2] / a);
        let ch = |row: &[f32; 5]| (row[0] * r + row[1] * g + row[2] * b + row[3] * a + row[4]).clamp(0.0, 1.0);
        let out_a = ch(&m[3]);
        *p = [ch(&m[0]) * out_a, ch(&m[1]) * out_a, ch(&m[2]) * out_a, out_a];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(w: usize, h: usize, x0: usize, x1: usize) -> Vec<Premul> {
        (0..w * h).map(|i| if (x0..x1).contains(&(i % w)) && (x0..x1).contains(&(i / w)) { [1.0, 0.0, 0.0, 1.0] } else { [0.0; 4] }).collect()
    }

    #[test]
    fn blur_spreads_and_keeps_the_total() {
        let (w, h) = (40, 40);
        let mut b = square(w, h, 15, 25);
        let before: f32 = b.iter().map(|p| p[3]).sum();
        apply(&mut b, w, h, &[Filter::Blur { blur_x: 8.0, blur_y: 8.0, quality: 3 }], 1.0);
        let after: f32 = b.iter().map(|p| p[3]).sum();
        assert!((before - after).abs() < 0.5, "{before} vs {after}");
        assert!(b[20 * w + 12][3] > 0.05, "spreads outside");
        assert!(b[20 * w + 20][3] < 0.9 && b[20 * w + 20][3] > 0.3, "softens inside: {}", b[20 * w + 20][3]);
        assert!((b[20 * w + 20][0] - b[20 * w + 20][3]).abs() < 1e-5, "stays premultiplied red");
    }

    #[test]
    fn drop_shadows_sit_under_and_offset() {
        let (w, h) = (40, 40);
        let mut b = square(w, h, 10, 20);
        let ds = Filter::DropShadow { blur_x: 0.0, blur_y: 0.0, strength: 1.0, quality: 1, angle: 0.0, distance: 5.0, color: Rgba::BLACK, inner: false, knockout: false, hide_object: false };
        apply(&mut b, w, h, std::slice::from_ref(&ds), 1.0);
        assert_eq!(b[15 * w + 22], [0.0, 0.0, 0.0, 1.0], "shadow to the right");
        assert_eq!(b[15 * w + 15], [1.0, 0.0, 0.0, 1.0], "artwork on top");
        assert_eq!(b[15 * w + 27][3], 0.0);
        // Knockout: only the shadow outside the artwork; hide object: the whole shadow.
        let mut k = square(w, h, 10, 20);
        let Filter::DropShadow { .. } = ds else { panic!() };
        apply(&mut k, w, h, &[Filter::DropShadow { knockout: true, blur_x: 0.0, blur_y: 0.0, strength: 1.0, quality: 1, angle: 0.0, distance: 5.0, color: Rgba::BLACK, inner: false, hide_object: false }], 1.0);
        assert_eq!(k[15 * w + 15][3], 0.0);
        assert_eq!(k[15 * w + 22][3], 1.0);
        // Inner glow: inside the edge only.
        let mut g = square(w, h, 10, 30);
        apply(&mut g, w, h, &[Filter::Glow { blur_x: 6.0, blur_y: 6.0, strength: 1.0, quality: 2, color: Rgba::WHITE, inner: true, knockout: false }], 1.0);
        assert!(g[20 * w + 11][1] > 0.2, "near the edge glows: {:?}", g[20 * w + 11]);
        assert!(g[20 * w + 20][1] < 0.01, "the middle doesn't");
        assert_eq!(g[20 * w + 5][3], 0.0, "nothing outside");
    }

    #[test]
    fn adjust_color_matrix() {
        let mut p = vec![[0.5, 0.25, 0.0, 1.0]];
        apply(&mut p, 1, 1, &[Filter::AdjustColor { brightness: 0.0, contrast: 0.0, saturation: -100.0, hue: 0.0 }], 1.0);
        assert!((p[0][0] - p[0][1]).abs() < 1e-4 && (p[0][1] - p[0][2]).abs() < 1e-4, "greyscale: {:?}", p[0]);
        let mut q = vec![[0.2, 0.2, 0.2, 1.0]];
        apply(&mut q, 1, 1, &[Filter::AdjustColor { brightness: 50.0, contrast: 0.0, saturation: 0.0, hue: 0.0 }], 1.0);
        assert!((q[0][0] - 0.7).abs() < 1e-4);
        // Hue rotation keeps grey grey and turns red away from red.
        let mut r = vec![[1.0, 0.0, 0.0, 1.0], [0.5, 0.5, 0.5, 1.0]];
        apply(&mut r, 2, 1, &[Filter::AdjustColor { brightness: 0.0, contrast: 0.0, saturation: 0.0, hue: 120.0 }], 1.0);
        assert!(r[0][1] > r[0][0], "red turns green: {:?}", r[0]);
        assert!((r[1][0] - 0.5).abs() < 1e-3 && (r[1][2] - 0.5).abs() < 1e-3);
        assert_eq!(color_matrix(0.0, 0.0, 0.0, 0.0), IDENTITY);
    }
}
