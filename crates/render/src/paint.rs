//! Paint shaders: solid colours and gradients evaluated per pixel through a 256-entry LUT.

use anicraft_doc::{ColorTransform, GradientStop, MAX_FOCAL, Paint, Pixels, Rgba, Spread};
use std::sync::Arc;
use anicraft_geom::{Affine, Vec2};

/// Premultiplied RGBA.
pub type Premul = [f32; 4];

pub fn premultiply(c: Rgba) -> Premul {
    let c = c.sanitized();
    [c.r * c.a, c.g * c.a, c.b * c.a, c.a]
}

const LUT: usize = 256;

pub enum Shader {
    Solid(Premul),
    Linear { inv: Affine, start: Vec2, dir: Vec2, spread: Spread, lut: Box<[Premul; LUT]> },
    /// `inv` maps pixels into the gradient's own space (its matrix undone), where the ring is
    /// the circle of radius `1 / inv_radius` round `center` and the focal point sits `focal`
    /// of the radius along x.
    Radial { inv: Affine, center: Vec2, inv_radius: f32, focal: f32, spread: Spread, lut: Box<[Premul; LUT]> },
    /// Samples of a bitmap: `inv` maps pixels back to the bitmap's own pixel grid. Bilinear
    /// when `smooth` (Allow Smoothing, ADR 0050), else the nearest pixel.
    Bitmap { inv: Affine, pixels: Arc<Pixels>, repeat: bool, fx: ColorTransform, smooth: bool },
    /// Premultiplied pixels already in surface space (a filtered group): `w` wide, its top-left
    /// at `(x0, y0)`; transparent outside.
    Image { data: Arc<Vec<Premul>>, x0: i32, y0: i32, w: usize },
}

fn sample_stops(stops: &[GradientStop], t: f32) -> Rgba {
    let Some(first) = stops.first() else { return Rgba::TRANSPARENT };
    if t <= first.offset {
        return first.color;
    }
    for w in stops.windows(2) {
        let (a, b) = (w[0], w[1]);
        if t <= b.offset {
            let span = b.offset - a.offset;
            let k = if span > 1e-6 { (t - a.offset) / span } else { 1.0 };
            return a.color.lerp(b.color, k);
        }
    }
    stops.last().map_or(first.color, |s| s.color)
}

/// sRGB-encoded channel → linear light (IEC 61966-2-1).
fn to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// Linear light → sRGB-encoded channel.
fn from_linear(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// The ramp's 256 colours. With `linear_rgb` the stops mix in linear light (Animate's Linear
/// RGB, SVG's `color-interpolation: linearRGB`); the colours at the stops are the same.
fn build_lut(stops: &[GradientStop], fx: &ColorTransform, linear_rgb: bool) -> Box<[Premul; LUT]> {
    let mut sorted: Vec<GradientStop> = stops.iter().copied().filter(|s| s.offset.is_finite()).map(|s| GradientStop { color: s.color.sanitized(), ..s }).collect();
    sorted.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    if linear_rgb {
        for s in &mut sorted {
            s.color = Rgba::new(to_linear(s.color.r), to_linear(s.color.g), to_linear(s.color.b), s.color.a);
        }
    }
    let mut lut = Box::new([[0.0; 4]; LUT]);
    for (i, slot) in lut.iter_mut().enumerate() {
        let t = i as f32 / (LUT - 1) as f32;
        let mut c = sample_stops(&sorted, t);
        if linear_rgb {
            c = Rgba::new(from_linear(c.r), from_linear(c.g), from_linear(c.b), c.a);
        }
        *slot = premultiply(fx.apply(c));
    }
    lut
}

/// Where on a focal radial ramp the point `u` falls (in the gradient's unit space: the ring is
/// the unit circle, the focal point at `(focal, 0)`): the `t` with `u = f + t·(q − f)` for `q`
/// on the ring. With `focal = 0` it is `|u|`. Solves `(1 − f²)t² − 2(f·d)t − d·d = 0`, `d = u − f`.
#[inline]
pub fn focal_t(u: Vec2, focal: f32) -> f32 {
    if focal == 0.0 {
        return u.length();
    }
    let (dx, dy) = (u.x - focal, u.y);
    let a = 1.0 - focal * focal;
    let fd = focal * dx;
    let dd = dx * dx + dy * dy;
    (fd + (fd * fd + a * dd).max(0.0).sqrt()) / a
}

impl Shader {
    /// `to_pixel` maps the paint's local space to pixels.
    /// `image` holds a [`Paint::Bitmap`]'s pixels (an image that can't be read shows grey).
    pub fn new(paint: &Paint, to_pixel: &Affine, fx: &ColorTransform, image: Option<&Arc<Pixels>>) -> Shader {
        let inv = to_pixel.inverse();
        match (paint, inv) {
            (Paint::Solid(c), _) => Shader::Solid(premultiply(fx.apply(*c))),
            (Paint::Linear { start, end, stops, spread, linear_rgb }, Some(inv)) => {
                let d = *end - *start;
                let len2 = d.dot(d);
                if len2 < 1e-12 {
                    return Shader::Solid(premultiply(fx.apply(stops.last().map_or(Rgba::TRANSPARENT, |s| s.color))));
                }
                Shader::Linear { inv, start: *start, dir: d * (1.0 / len2), spread: *spread, lut: build_lut(stops, fx, *linear_rgb) }
            }
            (Paint::Radial { center, radius, stops, spread, linear_rgb, focal, matrix }, Some(_)) => {
                // Pixels → shape space → the gradient's own space.
                let inv = match to_pixel.mul(matrix).inverse() {
                    Some(i) if radius.is_finite() && *radius > 1e-6 => i,
                    _ => return Shader::Solid(premultiply(fx.apply(stops.last().map_or(Rgba::TRANSPARENT, |s| s.color)))),
                };
                let focal = if focal.is_finite() { focal.clamp(-MAX_FOCAL, MAX_FOCAL) } else { 0.0 };
                Shader::Radial { inv, center: *center, inv_radius: 1.0 / radius, focal, spread: *spread, lut: build_lut(stops, fx, *linear_rgb) }
            }
            (Paint::Bitmap { repeat, matrix, .. }, Some(inv)) => match (image, matrix.inverse()) {
                // Pixel → shape space → image pixels.
                (Some(px), Some(to_image)) => Shader::Bitmap { inv: to_image.mul(&inv), pixels: px.clone(), repeat: *repeat, fx: *fx, smooth: true },
                // A collapsed image matrix shows nothing.
                (Some(_), None) => Shader::Solid([0.0; 4]),
                (None, _) => Shader::Solid(premultiply(fx.apply(paint.primary_color()))),
            },
            (p, None) => Shader::Solid(premultiply(fx.apply(p.primary_color()))),
        }
    }

    /// This shader with a bitmap's Allow Smoothing (other shaders are unchanged).
    pub fn smoothed(mut self, on: bool) -> Shader {
        if let Shader::Bitmap { smooth, .. } = &mut self {
            *smooth = on;
        }
        self
    }

    #[inline]
    pub fn is_solid(&self) -> Option<Premul> {
        match self {
            Shader::Solid(c) => Some(*c),
            _ => None,
        }
    }

    /// Colour at pixel centre `(x + 0.5, y + 0.5)`.
    #[inline]
    pub fn at(&self, x: i32, y: i32) -> Premul {
        let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
        let lookup = |lut: &[Premul; LUT], spread: Spread, t: f32| {
            let t = spread.apply(t);
            lut[(t * (LUT - 1) as f32 + 0.5) as usize & (LUT - 1)]
        };
        match self {
            Shader::Solid(c) => *c,
            Shader::Linear { inv, start, dir, spread, lut } => lookup(lut, *spread, (inv.apply(p) - *start).dot(*dir)),
            Shader::Radial { inv, center, inv_radius, focal, spread, lut } => lookup(lut, *spread, focal_t((inv.apply(p) - *center) * *inv_radius, *focal)),
            Shader::Bitmap { inv, pixels, repeat, fx, smooth } => sample(pixels, inv.apply(p), *repeat, *smooth, fx),
            Shader::Image { data, x0, y0, w } => {
                let (dx, dy) = (x - x0, y - y0);
                if dx < 0 || dy < 0 || dx as usize >= *w {
                    return [0.0; 4];
                }
                data.get(dy as usize * w + dx as usize).copied().unwrap_or([0.0; 4])
            }
        }
    }
}

/// A sample at bitmap position `q` (pixel (i, j) covers [i, i+1) × [j, j+1)), bilinear when
/// `smooth`, else the pixel `q` is in, with the colour effect applied and premultiplied.
/// Outside a non-repeating bitmap: transparent.
pub(crate) fn sample(px: &Pixels, q: Vec2, repeat: bool, smooth: bool, fx: &ColorTransform) -> Premul {
    let (w, h) = (px.width as f32, px.height as f32);
    if !(q.x.is_finite() && q.y.is_finite()) || w < 1.0 || h < 1.0 {
        return [0.0; 4];
    }
    let (mut u, mut v) = (q.x, q.y);
    if repeat {
        u = u.rem_euclid(w);
        v = v.rem_euclid(h);
    } else if u < 0.0 || v < 0.0 || u >= w || v >= h {
        return [0.0; 4];
    }
    if !smooth {
        let c = px.at(u.floor() as i64, v.floor() as i64);
        return premultiply(fx.apply(Rgba::new(c[0], c[1], c[2], c[3])));
    }
    let (fx_, fy_) = (u - 0.5, v - 0.5);
    let (x0, y0) = (fx_.floor(), fy_.floor());
    let (tx, ty) = (fx_ - x0, fy_ - y0);
    let wrap = |i: i64, n: u32| if repeat { i.rem_euclid(i64::from(n)) } else { i };
    let tap = |dx: i64, dy: i64| {
        let c = px.at(wrap(x0 as i64 + dx, px.width), wrap(y0 as i64 + dy, px.height));
        premultiply(fx.apply(Rgba::new(c[0], c[1], c[2], c[3])))
    };
    let (a, b, c, d) = (tap(0, 0), tap(1, 0), tap(0, 1), tap(1, 1));
    let mut out = [0.0; 4];
    for k in 0..4 {
        let top = a[k] + (b[k] - a[k]) * tx;
        let bottom = c[k] + (d[k] - c[k]) * tx;
        out[k] = top + (bottom - top) * ty;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_gradient_endpoints() {
        let paint = Paint::linear(Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0), vec![GradientStop { offset: 0.0, color: Rgba::BLACK }, GradientStop { offset: 1.0, color: Rgba::WHITE }]);
        let s = Shader::new(&paint, &Affine::IDENTITY, &ColorTransform::default(), None);
        assert!(s.at(0, 0)[0] < 0.02);
        assert!(s.at(99, 0)[0] > 0.98);
        assert!((s.at(49, 0)[0] - 0.5).abs() < 0.02);
        // gradient follows the transform
        let s2 = Shader::new(&paint, &Affine::scale(2.0, 1.0), &ColorTransform::default(), None);
        assert!((s2.at(99, 0)[0] - 0.5).abs() < 0.02);
    }

    fn bw() -> Vec<GradientStop> {
        vec![GradientStop { offset: 0.0, color: Rgba::BLACK }, GradientStop { offset: 1.0, color: Rgba::WHITE }]
    }

    fn shade(p: &Paint, x: i32, y: i32) -> f32 {
        Shader::new(p, &Affine::IDENTITY, &ColorTransform::default(), None).at(x, y)[0]
    }

    #[test]
    fn spread_reflects_and_repeats_past_the_ends() {
        // A ramp over x = 0..100 (pixel centres at x + 0.5).
        let mut p = Paint::linear(Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0), bw());
        let set = |p: &mut Paint, s: Spread| {
            if let Paint::Linear { spread, .. } = p {
                *spread = s;
            }
        };
        assert!(shade(&p, 149, 0) > 0.99, "pad keeps the end colour");
        assert!(shade(&p, -50, 0) < 0.01);
        set(&mut p, Spread::Reflect);
        assert!((shade(&p, 149, 0) - 0.505).abs() < 0.01, "reflect: halfway back at 1.5");
        assert!((shade(&p, -51, 0) - 0.505).abs() < 0.01, "and mirrored before the start");
        assert!(shade(&p, 199, 0) < 0.01, "back at the start colour at 2");
        set(&mut p, Spread::Repeat);
        assert!((shade(&p, 149, 0) - 0.495).abs() < 0.01, "repeat: starts again");
        assert!((shade(&p, 125, 0) - 0.255).abs() < 0.01);
        assert!((shade(&p, -75, 0) - 0.255).abs() < 0.01, "and before the start");
    }

    #[test]
    fn focal_point_moves_where_the_ramp_starts() {
        // Analytic: in unit space with the focal point at (f, 0), t = 0 there, 1 on the ring,
        // and on the axis between them t grows linearly from the focal point.
        for f in [0.0, 0.5, -0.7] {
            assert!(focal_t(Vec2::new(f, 0.0), f).abs() < 1e-6);
            for a in [0.0f32, 1.0, 2.0, 3.0, 4.5] {
                assert!((focal_t(Vec2::new(a.cos(), a.sin()), f) - 1.0).abs() < 1e-5, "ring at {a} with focal {f}");
            }
            // Towards +x the ring is 1 − f away: halfway is t = 0.5.
            assert!((focal_t(Vec2::new(f + (1.0 - f) * 0.5, 0.0), f) - 0.5).abs() < 1e-5);
        }
        // Rendered: radius 100 round (100, 100), focal point at 0.5 → (150, 100) is black.
        let mut p = Paint::radial(Vec2::new(100.0, 100.0), 100.0, bw());
        if let Paint::Radial { focal, .. } = &mut p {
            *focal = 0.5;
        }
        let at = |x: f32, y: f32| shade(&p, (x - 0.5) as i32, (y - 0.5) as i32);
        assert!(at(150.5, 100.5) < 0.02);
        assert!((at(175.5, 100.5) - 0.5).abs() < 0.02, "halfway from the focal point to the ring on its side");
        assert!((at(75.5, 100.5) - 0.5).abs() < 0.02, "and on the far side (75 of 150)");
        assert!(at(100.5, 100.5) > 0.3, "the centre is no longer the start");
        // Out of range values are held inside the ring.
        if let Paint::Radial { focal, .. } = &mut p {
            *focal = 5.0;
        }
        assert!(Shader::new(&p, &Affine::IDENTITY, &ColorTransform::default(), None).at(10, 10)[0].is_finite());
    }

    #[test]
    fn elliptical_radial_gradients_follow_their_matrix() {
        // Twice as wide as tall: the ring reaches x = ±100 but y = ±50.
        let p = Paint::radial(Vec2::ZERO, 1.0, bw()).with_frame(&Affine { m: [100.0, 0.0, 0.0, 50.0, 200.0, 200.0] }, None);
        let at = |x: i32, y: i32| shade(&p, x, y);
        assert!((at(249, 199) - 0.5).abs() < 0.02, "half way along x");
        assert!((at(199, 224) - 0.5).abs() < 0.02, "half way along y is half as far");
        assert!(at(199, 260) > 0.99);
    }

    #[test]
    fn linear_rgb_mixes_in_linear_light() {
        // Black to white: the sRGB midpoint is 0.5; mixed in linear light, the half-intensity
        // grey encodes as ≈ 0.735.
        let mut p = Paint::linear(Vec2::new(0.0, 0.0), Vec2::new(255.0, 0.0), bw());
        assert!((shade(&p, 127, 0) - 0.5).abs() < 0.01);
        if let Paint::Linear { linear_rgb, .. } = &mut p {
            *linear_rgb = true;
        }
        assert!((shade(&p, 127, 0) - 0.735).abs() < 0.01, "{}", shade(&p, 127, 0));
        assert!(shade(&p, -5, 0) < 1e-3 && shade(&p, 300, 0) > 0.999, "the stops keep their colours");
    }

    #[test]
    fn bitmaps_without_smoothing_keep_hard_pixel_edges() {
        // A 2×1 image, black then white, drawn 10× larger: smoothed, the middle is a ramp;
        // without smoothing, each half is one flat colour up to the edge between them.
        let px = Arc::new(Pixels { width: 2, height: 1, rgba: vec![0, 0, 0, 255, 255, 255, 255, 255] });
        let paint = Paint::Bitmap { bitmap: anicraft_doc::BitmapId(1), repeat: false, matrix: Affine::IDENTITY };
        let smooth = Shader::new(&paint, &Affine::scale(10.0, 10.0), &ColorTransform::default(), Some(&px));
        let hard = Shader::new(&paint, &Affine::scale(10.0, 10.0), &ColorTransform::default(), Some(&px)).smoothed(false);
        assert!(smooth.at(9, 5)[0] > 0.3 && smooth.at(9, 5)[0] < 0.7, "{:?}", smooth.at(9, 5));
        assert_eq!(hard.at(9, 5), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(hard.at(10, 5), [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(hard.at(0, 0), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(hard.at(19, 9), [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(hard.at(20, 5), [0.0; 4], "outside stays transparent");
        // Solid shaders ignore it.
        assert!(Shader::new(&Paint::Solid(Rgba::WHITE), &Affine::IDENTITY, &ColorTransform::default(), None).smoothed(false).is_solid().is_some());
    }

    #[test]
    fn bitmaps_sample_bilinear_once_or_tiled() {
        // 2×1: black, white.
        let px = Arc::new(Pixels { width: 2, height: 1, rgba: vec![0, 0, 0, 255, 255, 255, 255, 255] });
        let once = Shader::new(&Paint::Bitmap { bitmap: anicraft_doc::BitmapId(1), repeat: false, matrix: Affine::IDENTITY }, &Affine::scale(10.0, 10.0), &ColorTransform::default(), Some(&px));
        let r = |s: &Shader, x| s.at(x, 3)[0];
        assert!(r(&once, 0) < 0.01, "the black pixel");
        assert!(r(&once, 19) > 0.99, "the white pixel");
        assert!((r(&once, 9) - 0.45).abs() < 0.06, "blended across the seam: {}", r(&once, 9));
        assert_eq!(once.at(25, 3), [0.0; 4], "nothing past the edge");
        let tiled = Shader::new(&Paint::Bitmap { bitmap: anicraft_doc::BitmapId(1), repeat: true, matrix: Affine::IDENTITY }, &Affine::scale(10.0, 10.0), &ColorTransform::default(), Some(&px));
        assert!(r(&tiled, 25) < 0.2 && r(&tiled, 35) > 0.8, "tiles repeat");
        assert!(r(&tiled, 21) > 0.2, "and blend across the wrap-around seam");
        // Opacity applies, premultiplied; a missing image is grey, not a crash.
        let half = Shader::new(&Paint::Bitmap { bitmap: anicraft_doc::BitmapId(1), repeat: false, matrix: Affine::IDENTITY }, &Affine::scale(10.0, 10.0), &ColorTransform::alpha(0.5), Some(&px));
        assert!((half.at(19, 3)[3] - 0.5).abs() < 1e-3 && (half.at(19, 3)[0] - 0.5).abs() < 1e-2);
        assert!(Shader::new(&Paint::Bitmap { bitmap: anicraft_doc::BitmapId(9), repeat: false, matrix: Affine::IDENTITY }, &Affine::IDENTITY, &ColorTransform::default(), None).is_solid().is_some());
        // The fill's image matrix places the image inside the shape: here 10× larger, from x = 40.
        let placed = Paint::Bitmap { bitmap: anicraft_doc::BitmapId(1), repeat: false, matrix: Affine { m: [10.0, 0.0, 0.0, 10.0, 40.0, 0.0] } };
        let placed = Shader::new(&placed, &Affine::IDENTITY, &ColorTransform::default(), Some(&px));
        assert_eq!(placed.at(30, 3), [0.0; 4], "before the image");
        assert!(r(&placed, 42) < 0.01 && r(&placed, 57) > 0.99, "black then white, 10 px each");
        // A collapsed image matrix shows nothing rather than dividing by zero.
        let flat = Paint::Bitmap { bitmap: anicraft_doc::BitmapId(1), repeat: true, matrix: Affine { m: [0.0; 6] } };
        assert_eq!(Shader::new(&flat, &Affine::IDENTITY, &ColorTransform::default(), Some(&px)).at(3, 3), [0.0; 4]);
    }

    #[test]
    fn alpha_effect_premultiplies() {
        let s = Shader::new(&Paint::Solid(Rgba::WHITE), &Affine::IDENTITY, &ColorTransform::alpha(0.5), None);
        assert_eq!(s.at(0, 0), [0.5, 0.5, 0.5, 0.5]);
    }
}
