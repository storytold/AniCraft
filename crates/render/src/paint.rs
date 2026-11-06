//! Paint shaders: solid colours and gradients evaluated per pixel through a 256-entry LUT.

use anicraft_doc::{ColorEffect, GradientStop, Paint, Rgba};
use anicraft_geom::{Affine, Vec2};
use std::sync::Arc;

/// Premultiplied RGBA.
pub type Premul = [f32; 4];

pub fn premultiply(c: Rgba) -> Premul {
    let c = c.sanitized();
    [c.r * c.a, c.g * c.a, c.b * c.a, c.a]
}

const LUT: usize = 256;

pub enum Shader {
    Solid(Premul),
    Bitmap { image: Arc<anicraft_doc::Bitmap>, inv: Affine, fx: ColorEffect },
    Linear { inv: Affine, start: Vec2, dir: Vec2, lut: Box<[Premul; LUT]> },
    Radial { inv: Affine, center: Vec2, inv_radius: f32, lut: Box<[Premul; LUT]> },
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

fn build_lut(stops: &[GradientStop], fx: &ColorEffect) -> Box<[Premul; LUT]> {
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
    pub fn new(paint: &Paint, to_pixel: &Affine, fx: &ColorEffect) -> Shader {
        let inv = to_pixel.inverse();
        match (paint, inv) {
            (Paint::Solid(c), _) => Shader::Solid(premultiply(fx.apply(*c))),
            (Paint::Bitmap { image, transform }, _) => match to_pixel.mul(transform).inverse() {
                Some(inv) => Shader::Bitmap { image: image.clone(), inv, fx: *fx },
                None => Shader::Solid([0.0; 4]),
            },
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
            (p, None) => Shader::Solid(premultiply(fx.apply(p.primary_color()))),
        }
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
            Shader::Bitmap { image, inv, fx } => {
                let p = inv.apply(p);
                if !p.is_finite() || p.x < 0.0 || p.y < 0.0 || p.x >= image.width() as f32 || p.y >= image.height() as f32 {
                    return [0.0; 4];
                }
                // Bilinear in premultiplied space prevents dark fringes at transparent pixels.
                let (x, y) = (p.x - 0.5, p.y - 0.5);
                let (x0, y0) = (x.floor(), y.floor());
                let (tx, ty) = (x - x0, y - y0);
                let sample = |dx: f32, dy: f32| {
                    let x = (x0 + dx).clamp(0.0, image.width() as f32 - 1.0) as u32;
                    let y = (y0 + dy).clamp(0.0, image.height() as f32 - 1.0) as u32;
                    let [r, g, b, a] = image.pixel(x, y);
                    premultiply(fx.apply(Rgba::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0)))
                };
                let (a, b, c, d) = (sample(0.0, 0.0), sample(1.0, 0.0), sample(0.0, 1.0), sample(1.0, 1.0));
                std::array::from_fn(|i| (a[i] * (1.0 - tx) + b[i] * tx) * (1.0 - ty) + (c[i] * (1.0 - tx) + d[i] * tx) * ty)
            }
            Shader::Linear { inv, start, dir, lut } => lookup(lut, (inv.apply(p) - *start).dot(*dir)),
            Shader::Radial { inv, center, inv_radius, lut } => lookup(lut, (inv.apply(p) - *center).length() * inv_radius),
        }
    }
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
        let s = Shader::new(&paint, &Affine::IDENTITY, &ColorEffect::default());
        assert!(s.at(0, 0)[0] < 0.02);
        assert!(s.at(99, 0)[0] > 0.98);
        assert!((s.at(49, 0)[0] - 0.5).abs() < 0.02);
        // gradient follows the transform
        let s2 = Shader::new(&paint, &Affine::scale(2.0, 1.0), &ColorEffect::default());
        assert!((s2.at(99, 0)[0] - 0.5).abs() < 0.02);
    }

    #[test]
    fn bitmap_sampling_uses_premultiplied_interpolation_and_colour_effects() {
        let image = Arc::new(anicraft_doc::Bitmap::new(2, 1, vec![255, 0, 0, 255, 0, 0, 0, 0]).unwrap());
        let p = Paint::Bitmap { image, transform: Affine::IDENTITY };
        let shader = Shader::new(&p, &Affine::scale(2.0, 2.0), &ColorEffect::default());
        assert_eq!(shader.at(0, 0), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(shader.at(1, 0), [0.75, 0.0, 0.0, 0.75], "no dark halo at transparent pixels");
        assert_eq!(shader.at(4, 0), [0.0; 4], "no repeating outside the image");
        let shader = Shader::new(&p, &Affine::IDENTITY, &ColorEffect { alpha: 0.5, tint: Rgba::WHITE, tint_amount: 1.0 });
        assert_eq!(shader.at(0, 0), [0.5; 4]);
    }

    #[test]
    fn alpha_effect_premultiplies() {
        let s = Shader::new(&Paint::Solid(Rgba::WHITE), &Affine::IDENTITY, &ColorEffect { alpha: 0.5, ..Default::default() });
        assert_eq!(s.at(0, 0), [0.5, 0.5, 0.5, 0.5]);
    }
}
