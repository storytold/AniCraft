//! Filters on a group's composited pixels: premultiplied RGBA, `w × h`, row by row.
//!
//! Blur is repeated box blurs (one per quality step; three approach a Gaussian). Drop Shadow and
//! Glow blur the artwork's opacity, offset it and colour it, then put it under the artwork
//! (outer), or inside it over the artwork (inner); Knockout keeps only the shadow or glow.
//! Adjust Color is a colour matrix built from the standard matrix operations for brightness,
//! contrast, saturation and hue rotation (Haeberli, "Matrix Operations for Image Processing",
//! 1993), applied to unpremultiplied colour.
//!
//! Bevel and Gradient Bevel are emboss by offset differences, as the image-processing
//! literature describes it: the blurred opacity sampled a distance towards the light minus the
//! same sampled away from it is a signed field, positive on edges that face the light
//! (highlight) and negative on those that face away (shadow). Gradient Bevel maps that field
//! onto its ramp (highlight end 0, flat 0.5, shadow end 1), Gradient Glow maps the glow's
//! intensity onto its ramp (0 to 1). Inner keeps the effect inside the artwork (over it), Outer
//! outside (under it), Full both; Knockout keeps only the effect. ADR 0025.

use crate::paint::{Premul, premultiply};
use anicraft_doc::{Filter, FilterSide, GradientStop, Rgba, ramp_at};

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
            Filter::Bevel { blur_x, blur_y, strength, quality, angle, distance, shadow_color, highlight_color, side, knockout } => {
                let (dx, dy) = offset(angle, distance * scale);
                let field = bevel_field(buf, w, h, blur_x * scale, blur_y * scale, quality, dx, dy);
                let (hl, sh) = (premultiply(highlight_color), premultiply(shadow_color));
                let effect: Vec<Premul> = field
                    .iter()
                    .map(|&d| {
                        let v = (d * strength).clamp(-1.0, 1.0);
                        if v >= 0.0 { scaled(hl, v) } else { scaled(sh, -v) }
                    })
                    .collect();
                composite(buf, &effect, &effect, side, knockout);
            }
            Filter::GradientBevel { blur_x, blur_y, strength, quality, angle, distance, stops, side, knockout } => {
                let (dx, dy) = offset(angle, distance * scale);
                let field = bevel_field(buf, w, h, blur_x * scale, blur_y * scale, quality, dx, dy);
                let lut = ramp_lut(&stops);
                let effect: Vec<Premul> = field.iter().map(|&d| lut_at(&lut, 0.5 - 0.5 * (d * strength).clamp(-1.0, 1.0))).collect();
                composite(buf, &effect, &effect, side, knockout);
            }
            Filter::GradientGlow { blur_x, blur_y, strength, quality, angle, distance, stops, side, knockout } => {
                let (dx, dy) = offset(angle, distance * scale);
                let lut = ramp_lut(&stops);
                let glow = |inner: bool| -> Vec<Premul> {
                    let mut cov = shifted_coverage(buf, w, h, dx, dy, inner);
                    blur_channels(&mut cov, w, h, blur_x * scale, blur_y * scale, quality);
                    cov.iter().map(|k| lut_at(&lut, (k[0] * strength).clamp(0.0, 1.0))).collect()
                };
                let none = Vec::new();
                let inner = if side == FilterSide::Outer { None } else { Some(glow(true)) };
                let outer = if side == FilterSide::Inner { None } else { Some(glow(false)) };
                composite(buf, inner.as_ref().unwrap_or(&none), outer.as_ref().unwrap_or(&none), side, knockout);
            }
            Filter::AdjustColor { brightness, contrast, saturation, hue } => adjust(buf, &color_matrix(brightness, contrast, saturation, hue)),
        }
    }
}

/// Whole-pixel offset of `distance` along `angle` (degrees, clockwise from the right).
fn offset(angle: f32, distance: f32) -> (i64, i64) {
    let a = angle.to_radians();
    ((distance * a.cos()).round() as i64, (distance * a.sin()).round() as i64)
}

fn scaled(c: Premul, k: f32) -> Premul {
    [c[0] * k, c[1] * k, c[2] * k, c[3] * k]
}

/// The artwork's opacity (or, `inverse`, its transparency) moved by `(dx, dy)`; outside the
/// buffer the artwork is transparent.
fn shifted_coverage(buf: &[Premul], w: usize, h: usize, dx: i64, dy: i64, inverse: bool) -> Vec<[f32; 1]> {
    let mut cov = vec![[0.0f32]; w * h];
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            let a = alpha_at(buf, w, h, x - dx, y - dy);
            if let Some(c) = cov.get_mut(y as usize * w + x as usize) {
                c[0] = if inverse { 1.0 - a } else { a };
            }
        }
    }
    cov
}

fn alpha_at(buf: &[Premul], w: usize, h: usize, x: i64, y: i64) -> f32 {
    if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h { buf.get(y as usize * w + x as usize).map_or(0.0, |p| p[3]) } else { 0.0 }
}

/// The bevel's signed field in −1..=1: the blurred opacity a distance towards the light
/// (`−(dx, dy)`) minus the blurred opacity a distance away from it. Positive where an edge faces
/// the light (highlight), negative where it faces away (shadow), 0 where flat.
#[allow(clippy::too_many_arguments)]
fn bevel_field(buf: &[Premul], w: usize, h: usize, bx: f32, by: f32, quality: u8, dx: i64, dy: i64) -> Vec<f32> {
    let mut b: Vec<[f32; 1]> = buf.iter().map(|p| [p[3]]).collect();
    blur_channels(&mut b, w, h, bx, by, quality);
    let at = |x: i64, y: i64| if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h { b.get(y as usize * w + x as usize).map_or(0.0, |v| v[0]) } else { 0.0 };
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            out.push((at(x + dx, y + dy) - at(x - dx, y - dy)).clamp(-1.0, 1.0));
        }
    }
    out
}

/// Steps in a gradient filter's ramp table: even, so 0, ½ and 1 (the ends and the bevel's
/// flat middle) are samples exactly.
pub(crate) const RAMP_STEPS: usize = 256;

/// A gradient filter's ramp (sanitized stops) as `RAMP_STEPS + 1` premultiplied colours.
fn ramp_lut(stops: &[GradientStop]) -> Vec<Premul> {
    (0..=RAMP_STEPS).map(|i| premultiply(ramp_at(stops, i as f32 / RAMP_STEPS as f32))).collect()
}

fn lut_at(lut: &[Premul], t: f32) -> Premul {
    let i = if t.is_finite() { (t.clamp(0.0, 1.0) * RAMP_STEPS as f32).round() as usize } else { 0 };
    lut.get(i).copied().unwrap_or([0.0; 4])
}

/// Puts an effect with the artwork: `inner` atop it (inside its edge, over it), `outer` under
/// it (outside its edge), as `side` says; Knockout keeps only the effect, cut to its side.
/// Both are premultiplied, one per pixel (an empty list is no effect).
fn composite(buf: &mut [Premul], inner: &[Premul], outer: &[Premul], side: FilterSide, knockout: bool) {
    let zero = [0.0f32; 4];
    for (i, p) in buf.iter_mut().enumerate() {
        let ei = if side == FilterSide::Outer { zero } else { inner.get(i).copied().unwrap_or(zero) };
        let eo = if side == FilterSide::Inner { zero } else { outer.get(i).copied().unwrap_or(zero) };
        let src = *p;
        let ba = src[3];
        for c in 0..4 {
            p[c] = if knockout { ei[c] * ba + eo[c] * (1.0 - ba) } else { ei[c] * ba + src[c] * (1.0 - ei[3]) + eo[c] * (1.0 - ba) };
        }
    }
}

/// Box blur of width `n` along rows or columns, of any number of channels per pixel, centred
/// on each pixel: an odd width covers `n / 2` pixels each side; an even width covers one more
/// each side at half weight (so the blur never shifts and mirrored edges stay mirrored).
/// Outside the buffer is transparent.
fn box_pass<const C: usize>(buf: &mut [[f32; C]], w: usize, h: usize, n: usize, horizontal: bool, tmp: &mut Vec<[f32; C]>) {
    if n <= 1 {
        return;
    }
    let (len, lines) = if horizontal { (w, h) } else { (h, w) };
    let r = n / 2;
    let even = n.is_multiple_of(2);
    let (lo, hi) = (r, r);
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
                let ends = |c: usize| if even { 0.5 * (i.checked_sub(lo).and_then(|j| tmp.get(j)).map_or(0.0, |v| v[c]) + tmp.get(i + hi).map_or(0.0, |v| v[c])) } else { 0.0 };
                for c in 0..C {
                    out[c] = (sum[c] - ends(c)) * inv;
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

    fn bevel(side: FilterSide, knockout: bool) -> Filter {
        Filter::Bevel { blur_x: 3.0, blur_y: 3.0, strength: 1.0, quality: 1, angle: 45.0, distance: 3.0, shadow_color: Rgba::BLACK, highlight_color: Rgba::WHITE, side, knockout }
    }

    /// Red square `x0..x1` on a 40 × 40 buffer, through `f`.
    fn filtered(f: &Filter) -> Vec<Premul> {
        let mut b = square(40, 40, 12, 28);
        apply(&mut b, 40, 40, std::slice::from_ref(f), 1.0);
        b
    }

    #[test]
    fn bevels_light_the_top_left_and_shade_the_bottom_right_symmetrically() {
        let w = 40;
        let b = filtered(&bevel(FilterSide::Inner, false));
        // Inside the top-left edge: white over red; inside the bottom-right: darkened red.
        let tl = b[13 * w + 13];
        let br = b[26 * w + 26];
        assert!(tl[1] > 0.3 && (tl[3] - 1.0).abs() < 1e-5, "highlight: {tl:?}");
        assert!(br[0] < 0.7 && br[1] < 1e-5 && (br[3] - 1.0).abs() < 1e-5, "shadow: {br:?}");
        assert_eq!(b[20 * w + 20], [1.0, 0.0, 0.0, 1.0], "the flat middle is untouched");
        assert_eq!(b[5 * w + 5], [0.0; 4], "nothing outside an inner bevel");
        // Point symmetry: highlight at a pixel is the shadow at its mirror (white ↔ black).
        for (x, y) in [(13, 13), (14, 20), (20, 13), (12, 27)] {
            let (p, q) = (b[y * w + x], b[(39 - y) * w + (39 - x)]);
            let lit = p[1]; // white added (green was 0)
            let dark = 1.0 - q[0]; // red taken away by black
            assert!((lit - dark).abs() < 1e-5, "({x}, {y}): {p:?} vs {q:?}");
        }
        // Even blur widths stay centred: the same symmetry.
        let even = filtered(&Filter::Bevel { blur_x: 4.0, blur_y: 6.0, strength: 1.0, quality: 2, angle: 45.0, distance: 3.0, shadow_color: Rgba::BLACK, highlight_color: Rgba::WHITE, side: FilterSide::Full, knockout: true });
        for (x, y) in [(13, 13), (11, 20), (20, 10), (12, 27), (9, 9)] {
            let (p, q) = (even[y * w + x], even[(39 - y) * w + (39 - x)]);
            assert!((p[3] - q[3]).abs() < 1e-5, "even blur, ({x}, {y}): {p:?} vs {q:?}");
        }
        // The other diagonal corners see as much light as shade: they stay red.
        let tr = b[13 * w + 26];
        assert!(tr[1] < 0.2 && tr[0] > 0.8, "{tr:?}");
    }

    #[test]
    fn outer_full_and_knockout_bevels() {
        let w = 40;
        let outer = filtered(&bevel(FilterSide::Outer, false));
        assert_eq!(outer[20 * w + 20], [1.0, 0.0, 0.0, 1.0], "the artwork is on top");
        let out_tl = outer[11 * w + 11];
        let out_br = outer[28 * w + 28];
        assert!(out_tl[3] > 0.2 && (out_tl[0] - out_tl[3]).abs() < 1e-5 && (out_tl[1] - out_tl[3]).abs() < 1e-5, "white outside the lit edge: {out_tl:?}");
        assert!(out_br[3] > 0.2 && out_br[0] < 1e-5, "black outside the shaded edge: {out_br:?}");
        assert!((out_tl[3] - out_br[3]).abs() < 1e-5, "symmetric: {out_tl:?} {out_br:?}");
        assert_eq!(outer[13 * w + 13], [1.0, 0.0, 0.0, 1.0], "inside untouched");
        // Full: both, and knockout: only the effect (the flat middle and far outside clear).
        let full = filtered(&bevel(FilterSide::Full, false));
        assert!(full[13 * w + 13][1] > 0.3 && full[11 * w + 11][3] > 0.2);
        let ko = filtered(&bevel(FilterSide::Full, true));
        assert_eq!(ko[20 * w + 20][3], 0.0, "the artwork is knocked out");
        assert_eq!(ko[2 * w + 2][3], 0.0);
        assert!((ko[13 * w + 13][3] - full[13 * w + 13][1]).abs() < 1e-5, "the highlight alone: {:?}", ko[13 * w + 13]);
        let ko_in = filtered(&bevel(FilterSide::Inner, true));
        assert_eq!(ko_in[11 * w + 11][3], 0.0, "inner knockout has nothing outside");
        // Strength 0 changes nothing.
        let Filter::Bevel { blur_x, blur_y, quality, angle, distance, shadow_color, highlight_color, .. } = bevel(FilterSide::Full, false) else { panic!() };
        let none = filtered(&Filter::Bevel { strength: 0.0, blur_x, blur_y, quality, angle, distance, shadow_color, highlight_color, side: FilterSide::Full, knockout: false });
        assert_eq!(none, square(40, 40, 12, 28));
    }

    #[test]
    fn gradient_ramps_map_their_ends() {
        let w = 40;
        let st = |offset: f32, c: u32, a: f32| GradientStop { offset, color: Rgba::hex(c).with_alpha(a) };
        // A hard-edged glow (no blur) at full strength takes the ramp's end colour; past its
        // reach, the transparent start.
        let glow = |side, knockout, stops: Vec<GradientStop>| Filter::GradientGlow { blur_x: 0.0, blur_y: 0.0, strength: 1.0, quality: 1, angle: 0.0, distance: 4.0, stops, side, knockout }.sanitized();
        let ramp = vec![st(0.0, 0xffffff, 0.0), st(1.0, 0x0000ff, 1.0)];
        let g = filtered(&glow(FilterSide::Outer, false, ramp.clone()));
        assert_eq!(g[20 * w + 30], [0.0, 0.0, 1.0, 1.0], "blue glow, offset right");
        assert_eq!(g[20 * w + 10], [0.0; 4], "nothing on the far side");
        assert_eq!(g[20 * w + 20], [1.0, 0.0, 0.0, 1.0]);
        // Blurred, the ramp's middle shows where the glow is half: here green.
        let mid = vec![st(0.0, 0xffffff, 0.0), st(0.5, 0x00ff00, 1.0), st(1.0, 0x0000ff, 1.0)];
        let soft = filtered(&Filter::GradientGlow { blur_x: 9.0, blur_y: 0.0, strength: 1.0, quality: 1, angle: 0.0, distance: 0.0, stops: mid, side: FilterSide::Outer, knockout: false });
        let edge = soft[20 * w + 28]; // just outside the right edge: 4 of 9 covered
        assert!(edge[1] > edge[2] && edge[1] > 0.4, "{edge:?}");
        // Inner gradient glow: the ramp's end inside the left edge (the transparency moved
        // right into it), nothing outside; knockout keeps it alone.
        let gi = filtered(&glow(FilterSide::Inner, true, ramp));
        assert_eq!(gi[20 * w + 13], [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(gi[20 * w + 20][3], 0.0);
        assert_eq!(gi[20 * w + 5][3], 0.0);
        // Gradient bevel: the highlight end on the lit edge, the shadow end on the other,
        // the transparent middle in the flat.
        let gb = Filter::GradientBevel { blur_x: 0.0, blur_y: 0.0, strength: 1.0, quality: 1, angle: 0.0, distance: 3.0, stops: vec![st(0.0, 0x00ff00, 1.0), st(0.5, 0, 0.0), st(1.0, 0x0000ff, 1.0)], side: FilterSide::Full, knockout: true };
        let b = filtered(&gb);
        assert_eq!(b[20 * w + 13], [0.0, 1.0, 0.0, 1.0], "lit left edge (light from the left)");
        assert_eq!(b[20 * w + 26], [0.0, 0.0, 1.0, 1.0], "shaded right edge");
        assert_eq!(b[20 * w + 20][3], 0.0, "flat: the transparent middle");
        assert_eq!(b[20 * w + 10], [0.0, 1.0, 0.0, 1.0], "full: outside too");
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
