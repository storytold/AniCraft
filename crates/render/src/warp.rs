//! Warped bitmaps (ADR 0079): an image drawn through the triangles of its warp mesh.
//!
//! The oracle prepares a warped bitmap fill as an image of its own, in surface pixels over the
//! fill's bounds: each mesh triangle carries its pixels' centres back to the image as drawn
//! (the triangle's affine map, exact) and samples it there, bilinear with Allow Smoothing,
//! else the nearest pixel, exactly as an unwarped bitmap is sampled. Triangles are written in
//! mesh order, so where the mesh folds over itself the later one shows. A pixel at the
//! outline's edge whose centre falls in no triangle takes its neighbour's colour, so the
//! outline's antialiasing has a colour to fade. The fill then draws that image as a
//! [`Shader::Bitmap`](crate::paint::Shader) of one image pixel per surface pixel, nearest, with
//! the item's colour effect: the GPU stage takes the very same prepared fill, so the two agree.
//!
//! Every pixel is written by one task (rows in bands, in parallel), so the result is the same
//! whatever the thread count.

use crate::paint::sample;
use crate::raster::IRect;
use anicraft_doc::{ColorTransform, Pixels, warp::WarpMesh};
use anicraft_geom::{Affine, Vec2};
use rayon::prelude::*;
use std::sync::Arc;

/// Most pixels a warped bitmap's prepared image may have (256 MB of RGBA).
pub const MAX_TEXTURE_PIXELS: usize = 1 << 26;
/// Rows per parallel band.
const BAND: usize = 32;

/// One triangle in surface pixels: its corners, the map from a pixel back to the image, and
/// the rows it touches.
struct Tri {
    p: [Vec2; 3],
    to_image: Affine,
    y0: i32,
    y1: i32,
}

/// The image of the bitmap `px` (its fill matrix `matrix`, tiled when `repeat`) as `mesh`
/// bends it under `m` (local → surface pixels), over `b` (surface pixels): straight RGBA8,
/// `b`'s size, transparent where no triangle is. `None` when it can't be drawn (a collapsed
/// fill matrix, an empty or oversized area).
pub fn texture(mesh: &WarpMesh, m: &Affine, matrix: &Affine, repeat: bool, px: &Pixels, smooth: bool, b: IRect) -> Option<Arc<Pixels>> {
    let (w, h) = (b.width(), b.height());
    if w == 0 || h == 0 || w.checked_mul(h)? > MAX_TEXTURE_PIXELS {
        return None;
    }
    let to_image = matrix.inverse()?;
    let corner = |pts: &[Vec2], i: u32| pts.get(i as usize).copied();
    let tris: Vec<Tri> = mesh
        .tris
        .iter()
        .filter_map(|t| {
            let [s0, s1, s2] = [corner(&mesh.shown, t[0])?, corner(&mesh.shown, t[1])?, corner(&mesh.shown, t[2])?].map(|v| m.apply(v));
            let [r0, r1, r2] = [corner(&mesh.rest, t[0])?, corner(&mesh.rest, t[1])?, corner(&mesh.rest, t[2])?];
            // Surface → local as drawn: R · S⁻¹ (S and R take the unit triangle to each).
            let s = Affine { m: [s1.x - s0.x, s1.y - s0.y, s2.x - s0.x, s2.y - s0.y, s0.x, s0.y] };
            let r = Affine { m: [r1.x - r0.x, r1.y - r0.y, r2.x - r0.x, r2.y - r0.y, r0.x, r0.y] };
            let map = to_image.mul(&r.mul(&s.inverse()?));
            if !map.is_finite() {
                return None;
            }
            let (lo, hi) = (s0.y.min(s1.y).min(s2.y), s0.y.max(s1.y).max(s2.y));
            if !(lo.is_finite() && hi.is_finite()) {
                return None;
            }
            Some(Tri { p: [s0, s1, s2], to_image: map, y0: (lo.floor() as i32).max(b.y0), y1: (hi.ceil() as i32 + 1).min(b.y1) })
        })
        .filter(|t| t.y0 < t.y1)
        .collect();
    let fx = ColorTransform::default();
    let mut rgba = vec![0u8; w * h * 4];
    let mut set = vec![false; w * h];
    rgba.par_chunks_mut(w * 4 * BAND).zip(set.par_chunks_mut(w * BAND)).enumerate().for_each(|(band, (out, seen))| {
        let top = b.y0 + (band * BAND) as i32;
        let rows = seen.len() / w;
        let bottom = top + rows as i32;
        for t in tris.iter().filter(|t| t.y0 < bottom && t.y1 > top) {
            fill_tri(t, b, top, bottom, w, px, repeat, smooth, &fx, out, seen);
        }
    });
    bleed(&mut rgba, &set, w, h);
    Some(Arc::new(Pixels { width: w as u32, height: h as u32, rgba }))
}

/// Writes the pixels of rows `top..bottom` whose centres `t` holds.
#[allow(clippy::too_many_arguments)]
fn fill_tri(t: &Tri, b: IRect, top: i32, bottom: i32, w: usize, px: &Pixels, repeat: bool, smooth: bool, fx: &ColorTransform, out: &mut [u8], seen: &mut [bool]) {
    let [a, p1, p2] = t.p;
    let area = (p1 - a).cross(p2 - a);
    if area.abs() <= 1e-9 || area.is_nan() {
        return;
    }
    // Edge functions, oriented so the inside is positive; centres on an edge count (a hair of
    // slack so the triangles of a cell leave no crack between them).
    let sign = area.signum();
    let edges = [(a, p1), (p1, p2), (p2, a)];
    let slack = -1e-4 * area.abs();
    let (xs0, xs1) = (a.x.min(p1.x).min(p2.x), a.x.max(p1.x).max(p2.x));
    let x0 = (xs0.floor() as i32).max(b.x0);
    let x1 = (xs1.ceil() as i32 + 1).min(b.x1);
    for y in t.y0.max(top)..t.y1.min(bottom) {
        let cy = y as f32 + 0.5;
        let row = (y - top) as usize;
        for x in x0..x1 {
            let c = Vec2::new(x as f32 + 0.5, cy);
            if edges.iter().any(|&(e0, e1)| sign * (e1 - e0).cross(c - e0) < slack) {
                continue;
            }
            let i = row * w + (x - b.x0) as usize;
            let s = sample(px, t.to_image.apply(c), repeat, smooth, fx);
            let a = s[3];
            let straight = |v: f32| if a > 0.0 { (v / a).clamp(0.0, 1.0) } else { 0.0 };
            let q = |v: f32| (v * 255.0 + 0.5) as u8;
            if let (Some(o), Some(flag)) = (out.get_mut(i * 4..i * 4 + 4), seen.get_mut(i)) {
                o.copy_from_slice(&[q(straight(s[0])), q(straight(s[1])), q(straight(s[2])), q(a.clamp(0.0, 1.0))]);
                *flag = true;
            }
        }
    }
}

/// Gives each pixel no triangle wrote the colour of a written neighbour (left, right, up,
/// down, the first there is), so the outline's antialiased edge has a colour to fade.
fn bleed(rgba: &mut [u8], set: &[bool], w: usize, h: usize) {
    let src = rgba.to_vec();
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if set.get(i).copied().unwrap_or(true) {
                continue;
            }
            let near = [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), (y > 0).then(|| i - w), (y + 1 < h).then(|| i + w)];
            let Some(n) = near.into_iter().flatten().find(|&n| set.get(n).copied().unwrap_or(false)) else { continue };
            if let (Some(from), Some(to)) = (src.get(n * 4..n * 4 + 4), rgba.get_mut(i * 4..i * 4 + 4)) {
                to.copy_from_slice(from);
            }
        }
    }
}
