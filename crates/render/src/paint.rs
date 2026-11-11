//! Paint shaders: solid colours and gradients evaluated per pixel through a 256-entry LUT.

use anicraft_doc::{ColorTransform, GradientStop, Paint, Pixels, Rgba};
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
    Linear { inv: Affine, start: Vec2, dir: Vec2, lut: Box<[Premul; LUT]> },
    Radial { inv: Affine, center: Vec2, inv_radius: f32, lut: Box<[Premul; LUT]> },
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

fn build_lut(stops: &[GradientStop], fx: &ColorTransform) -> Box<[Premul; LUT]> {
    let mut sorted: Vec<GradientStop> = stops.iter().copied().filter(|s| s.offset.is_finite()).collect();
    sorted.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    let mut lut = Box::new([[0.0; 4]; LUT]);
    for (i, slot) in lut.iter_mut().enumerate() {
        let t = i as f32 / (LUT - 1) as f32;
        *slot = premultiply(fx.apply(sample_stops(&sorted, t)));
    }
    lut
}

impl Shader {
    /// `to_pixel` maps the paint's local space to pixels.
    /// `image` holds a [`Paint::Bitmap`]'s pixels (an image that can't be read shows grey).
    pub fn new(paint: &Paint, to_pixel: &Affine, fx: &ColorTransform, image: Option<&Arc<Pixels>>) -> Shader {
        let inv = to_pixel.inverse();
        match (paint, inv) {
            (Paint::Solid(c), _) => Shader::Solid(premultiply(fx.apply(*c))),
            (Paint::Linear { start, end, stops }, Some(inv)) => {
                let d = *end - *start;
                let len2 = d.dot(d);
                if len2 < 1e-12 {
                    return Shader::Solid(premultiply(fx.apply(stops.last().map_or(Rgba::TRANSPARENT, |s| s.color))));
                }
                Shader::Linear { inv, start: *start, dir: d * (1.0 / len2), lut: build_lut(stops, fx) }
            }
            (Paint::Radial { center, radius, stops }, Some(inv)) => {
                if !(radius.is_finite() && *radius > 1e-6) {
                    return Shader::Solid(premultiply(fx.apply(stops.last().map_or(Rgba::TRANSPARENT, |s| s.color))));
                }
                Shader::Radial { inv, center: *center, inv_radius: 1.0 / radius, lut: build_lut(stops, fx) }
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
        let lookup = |lut: &[Premul; LUT], t: f32| {
            let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
            lut[(t * (LUT - 1) as f32 + 0.5) as usize & (LUT - 1)]
        };
        match self {
            Shader::Solid(c) => *c,
            Shader::Linear { inv, start, dir, lut } => lookup(lut, (inv.apply(p) - *start).dot(*dir)),
            Shader::Radial { inv, center, inv_radius, lut } => lookup(lut, (inv.apply(p) - *center).length() * inv_radius),
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
fn sample(px: &Pixels, q: Vec2, repeat: bool, smooth: bool, fx: &ColorTransform) -> Premul {
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
        let paint = Paint::Linear {
            start: Vec2::new(0.0, 0.0),
            end: Vec2::new(100.0, 0.0),
            stops: vec![GradientStop { offset: 0.0, color: Rgba::BLACK }, GradientStop { offset: 1.0, color: Rgba::WHITE }],
        };
        let s = Shader::new(&paint, &Affine::IDENTITY, &ColorTransform::default(), None);
        assert!(s.at(0, 0)[0] < 0.02);
        assert!(s.at(99, 0)[0] > 0.98);
        assert!((s.at(49, 0)[0] - 0.5).abs() < 0.02);
        // gradient follows the transform
        let s2 = Shader::new(&paint, &Affine::scale(2.0, 1.0), &ColorTransform::default(), None);
        assert!((s2.at(99, 0)[0] - 0.5).abs() < 0.02);
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
