//! AniCraft CPU oracle renderer (layer L2).
//!
//! Deterministic: the same display list and view always produce the same bytes, whatever the
//! thread count. Each output row is written by exactly one task and shapes composite in order.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod paint;
pub mod raster;
pub mod stroke;
pub mod svg;

use anicraft_doc::{ColorEffect, DisplayItem, DisplayList, Document, Paint, Rgba, evaluate};
use anicraft_geom::{Affine, FillRule, Rect, Vec2};
use paint::{Premul, Shader, premultiply};
use raster::{Coverage, IRect};
use rayon::prelude::*;

/// Largest surface we allocate (pixels): 4096 × 4096.
pub const MAX_PIXELS: u64 = 4096 * 4096;
/// Flattening tolerance in output pixels.
pub const TOLERANCE: f32 = 0.1;

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("image {0}×{1} is too large (limit {MAX_PIXELS} pixels)")]
    TooLarge(u32, u32),
    #[error("PNG encoding failed: {0}")]
    Png(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Premultiplied f32 RGBA surface.
#[derive(Clone, Debug, PartialEq)]
pub struct Pixmap {
    pub width: u32,
    pub height: u32,
    pub data: Vec<Premul>,
}

impl Pixmap {
    pub fn new(width: u32, height: u32) -> Result<Pixmap, RenderError> {
        let n = u64::from(width) * u64::from(height);
        if n > MAX_PIXELS {
            return Err(RenderError::TooLarge(width, height));
        }
        Ok(Pixmap { width, height, data: vec![[0.0; 4]; n as usize] })
    }

    pub fn fill(&mut self, c: Rgba) {
        let p = premultiply(c);
        self.data.fill(p);
    }

    /// Straight-alpha 8-bit RGBA bytes.
    pub fn to_rgba8(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.data.len() * 4);
        for p in &self.data {
            let a = p[3].clamp(0.0, 1.0);
            let un = |v: f32| if a > 1e-6 { (v / a).clamp(0.0, 1.0) } else { 0.0 };
            let q = |v: f32| (v * 255.0 + 0.5) as u8;
            out.extend_from_slice(&[q(un(p[0])), q(un(p[1])), q(un(p[2])), q(a)]);
        }
        out
    }

    /// Premultiplied 8-bit RGBA (what GPU textures with premultiplied alpha expect).
    pub fn to_premul_rgba8(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.data.len() * 4);
        for p in &self.data {
            let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            out.extend_from_slice(&[q(p[0]), q(p[1]), q(p[2]), q(p[3])]);
        }
        out
    }

    pub fn encode_png(&self) -> Result<Vec<u8>, RenderError> {
        encode_png(self.width, self.height, &self.to_rgba8())
    }

    pub fn pixel(&self, x: u32, y: u32) -> Option<Premul> {
        if x >= self.width || y >= self.height {
            return None;
        }
        self.data.get(y as usize * self.width as usize + x as usize).copied()
    }
}

/// Encodes straight-alpha RGBA8 as PNG.
pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, RenderError> {
    let mut buf = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut buf, width, height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().map_err(|e| RenderError::Png(e.to_string()))?;
        w.write_image_data(rgba).map_err(|e| RenderError::Png(e.to_string()))?;
    }
    Ok(buf)
}

/// Where and how to draw.
#[derive(Clone, Copy, Debug)]
pub struct View {
    /// Stage → pixel transform.
    pub transform: Affine,
    pub width: u32,
    pub height: u32,
    /// Fill colour before drawing (`None` = transparent).
    pub background: Option<Rgba>,
}

impl View {
    /// The whole stage at `scale`.
    pub fn stage(doc: &Document, scale: f32) -> View {
        let s = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        View {
            transform: Affine::scale(s, s),
            width: ((doc.stage.width as f32 * s).round() as u32).max(1),
            height: ((doc.stage.height as f32 * s).round() as u32).max(1),
            background: Some(doc.stage.background),
        }
    }
}

/// Statistics from one render (for the status bar and perf logs).
#[derive(Clone, Copy, Debug, Default)]
pub struct RenderStats {
    pub items: usize,
    pub drawn: usize,
    /// Items skipped because they lie outside the view.
    pub culled: usize,
    pub edges: usize,
    pub pixels_touched: u64,
}

/// Renders frame `frame` of `doc` at `scale` with the stage background.
pub fn render_frame(doc: &Document, frame: u32, scale: f32) -> Result<Pixmap, RenderError> {
    let dl = evaluate(doc, frame);
    render(&dl, &View::stage(doc, scale)).map(|(p, _)| p)
}

/// Renders a display list.
pub fn render(dl: &DisplayList, view: &View) -> Result<(Pixmap, RenderStats), RenderError> {
    let mut pm = Pixmap::new(view.width, view.height)?;
    if let Some(bg) = view.background {
        pm.fill(bg);
    }
    let surface = IRect { x0: 0, y0: 0, x1: view.width as i32, y1: view.height as i32 };

    // Phase 1 (parallel over items): cull, flatten, stroke and build shaders. Output keeps
    // display-list order.
    let prepared: Vec<Vec<Prepared>> = dl.items.par_iter().map(|item| prepare(item, &view.transform, surface)).collect();
    let mut stats = RenderStats { items: dl.items.len(), ..Default::default() };
    let mut ops: Vec<Prepared> = Vec::with_capacity(prepared.len());
    let mut mask_ops: Vec<Prepared> = Vec::new();
    for p in prepared {
        if p.is_empty() {
            stats.culled += 1;
        }
        for op in p {
            if op.masks.is_some() { mask_ops.push(op) } else { ops.push(op) }
        }
    }
    let clips = Clips::new(&dl.clip_parents, mask_ops);
    // A clipped op can only touch its clip's area; an op whose clip is empty draws nothing.
    ops.retain_mut(|op| {
        let Some(c) = op.clip else { return true };
        op.bounds = op.bounds.intersect(&clips.bounds(c));
        !op.bounds.is_empty()
    });
    stats.drawn = ops.len();
    for op in &ops {
        stats.edges += op.polys.iter().map(Vec::len).sum::<usize>();
        stats.pixels_touched += (op.bounds.width() * op.bounds.height()) as u64;
    }

    // Phase 2 (parallel over horizontal bands): each band rasterizes every op that touches
    // it, in order, into its own rows. A row is owned by exactly one band, so the result is
    // identical to a serial render whatever the thread count.
    let w = view.width as usize;
    pm.data.par_chunks_mut(BAND_ROWS * w.max(1)).enumerate().for_each(|(bi, rows)| {
        let y0 = (bi * BAND_ROWS) as i32;
        let band = IRect { x0: 0, y0, x1: surface.x1, y1: (y0 + BAND_ROWS as i32).min(surface.y1) };
        let mut cov = Coverage::new(IRect { x0: 0, y0: 0, x1: 0, y1: 0 });
        let mut row = Vec::new();
        let mut band_clips = BandClips::default();
        for op in &ops {
            let b = IRect { x0: op.bounds.x0, y0: op.bounds.y0.max(band.y0), x1: op.bounds.x1, y1: op.bounds.y1.min(band.y1) };
            if b.is_empty() {
                continue;
            }
            let mask = op.clip.map(|c| band_clips.get(&clips, c, band, w, &mut cov, &mut row));
            cov.reset(b);
            for poly in &op.polys {
                cov.add_polygon(poly);
            }
            composite(&cov, op, b, band.y0, w, rows, &mut row, mask.as_deref().map(Vec::as_slice));
        }
    });
    Ok((pm, stats))
}

/// Rows per parallel band.
const BAND_ROWS: usize = 32;

/// One fill operation, ready to rasterize: polygons in pixel space, its paint and bounds.
struct Prepared {
    polys: Vec<Vec<Vec2>>,
    rule: FillRule,
    shader: Shader,
    bounds: IRect,
    /// Drawn only inside this clip.
    clip: Option<u32>,
    /// Mask artwork for this clip (not drawn).
    masks: Option<u32>,
}

/// The clips of a display list: their mask artwork and where each can be non-zero.
struct Clips {
    parents: Vec<Option<u32>>,
    /// Mask artwork, grouped by clip.
    art: Vec<Vec<Prepared>>,
    bounds: Vec<IRect>,
}

const NO_PIXELS: IRect = IRect { x0: 0, y0: 0, x1: 0, y1: 0 };

impl Clips {
    fn new(parents: &[Option<u32>], mask_ops: Vec<Prepared>) -> Clips {
        let n = parents.len();
        let mut art: Vec<Vec<Prepared>> = (0..n).map(|_| Vec::new()).collect();
        for op in mask_ops {
            if let Some(a) = op.masks.and_then(|c| art.get_mut(c as usize)) {
                a.push(op);
            }
        }
        // A parent always precedes its children, so one pass in order intersects the chain.
        // A malformed parent link (not earlier in the list) masks everything out.
        let mut bounds: Vec<IRect> = Vec::with_capacity(n);
        let mut clean: Vec<Option<u32>> = Vec::with_capacity(n);
        for (i, a) in art.iter().enumerate() {
            let own = a.iter().fold(NO_PIXELS, |r, op| r.union(&op.bounds));
            let parent = parents.get(i).copied().flatten();
            let b = match parent {
                None => own,
                Some(p) => bounds.get(p as usize).filter(|_| (p as usize) < i).map_or(NO_PIXELS, |pb| own.intersect(pb)),
            };
            bounds.push(b);
            clean.push(parent.filter(|&p| (p as usize) < i));
        }
        Clips { parents: clean, art, bounds }
    }

    fn bounds(&self, c: u32) -> IRect {
        self.bounds.get(c as usize).copied().unwrap_or(NO_PIXELS)
    }
}

/// Clip coverage for one band, computed on first use: `w × band height` values in `0..=1`.
#[derive(Default)]
struct BandClips {
    cov: Vec<Option<std::rc::Rc<Vec<f32>>>>,
}

impl BandClips {
    fn get(&mut self, clips: &Clips, c: u32, band: IRect, w: usize, cov: &mut Coverage, row: &mut Vec<f32>) -> std::rc::Rc<Vec<f32>> {
        let i = c as usize;
        if self.cov.len() < clips.parents.len() {
            self.cov.resize(clips.parents.len(), None);
        }
        if let Some(Some(done)) = self.cov.get(i) {
            return done.clone();
        }
        let parent = clips.parents.get(i).copied().flatten().map(|p| self.get(clips, p, band, w, cov, row));
        let mut buf = vec![0.0f32; w * band.height()];
        // Union of the artwork's coverage (a ∪ b = a + b − ab), anti-aliased like any fill.
        for op in clips.art.get(i).into_iter().flatten() {
            let b = op.bounds.intersect(&band);
            if b.is_empty() {
                continue;
            }
            cov.reset(b);
            for poly in &op.polys {
                cov.add_polygon(poly);
            }
            row.clear();
            row.resize(b.width(), 0.0);
            for ry in 0..b.height() {
                cov.resolve_row(ry, op.rule, row);
                let start = (b.y0 + ry as i32 - band.y0) as usize * w + b.x0 as usize;
                let Some(dst) = buf.get_mut(start..start + b.width()) else { continue };
                for (d, &k) in dst.iter_mut().zip(row.iter()) {
                    let k = k.clamp(0.0, 1.0);
                    *d = *d + k - *d * k;
                }
            }
        }
        if let Some(p) = parent {
            for (d, m) in buf.iter_mut().zip(p.iter()) {
                *d *= m;
            }
        }
        let buf = std::rc::Rc::new(buf);
        if let Some(slot) = self.cov.get_mut(i) {
            *slot = Some(buf.clone());
        }
        buf
    }
}

/// Turns a display item into fill operations (fill, then stroke), or none when culled.
fn prepare(item: &DisplayItem, view: &Affine, surface: IRect) -> Vec<Prepared> {
    let mut out = Vec::new();
    let m = view.mul(&item.transform);
    if !m.is_finite() {
        return out;
    }
    // Mask artwork is its fill area only: no strokes, no opacity (as animators expect).
    let is_mask = item.masks.is_some();
    if is_mask && item.fill.is_none() {
        return out;
    }
    // Cull before flattening: control-point bounds are a cheap, conservative superset.
    let pad = if is_mask { 1.0 } else { item.stroke.map_or(0.0, |s| s.width * 0.5) + 1.0 };
    let bounds = item.path.bounds().inflate(pad).transformed(&m);
    let surface_rect = Rect::new(0.0, 0.0, surface.x1 as f32, surface.y1 as f32);
    if bounds.is_empty() || bounds.intersect(&surface_rect).is_empty() {
        return out;
    }
    let lines = item.path.flatten(&m, TOLERANCE);
    if lines.is_empty() {
        return out;
    }
    let (clip, masks) = (item.clip, item.masks);
    let mut push = |polys: Vec<Vec<Vec2>>, rule, shader: Shader| {
        if shader.is_solid().is_some_and(|c| c[3] <= 0.0) {
            return;
        }
        if let Some(b) = pixel_bounds(&polys, surface) {
            out.push(Prepared { polys, rule, shader, bounds: b, clip, masks });
        }
    };

    if is_mask {
        push(lines.iter().map(|l| l.points.clone()).collect(), item.fill_rule, Shader::Solid([1.0; 4]));
        return out;
    }
    if let Some(color) = item.outline {
        // Outline mode: hairline in the layer colour, no fill.
        push(stroke::stroke_polygons(&lines, 0.5), FillRule::NonZero, Shader::Solid(premultiply(color)));
        return out;
    }
    if let Some(fill) = &item.fill {
        let polys: Vec<Vec<Vec2>> = lines.iter().map(|l| l.points.clone()).collect();
        push(polys, item.fill_rule, Shader::new(fill, &m, &item.color));
    }
    if let Some(st) = item.stroke {
        let mut fx: ColorEffect = item.color;
        let mut pw = st.width * m.mean_scale();
        if pw.is_finite() && pw > 0.0 {
            if pw < 1.0 {
                // Thin strokes: keep a 1 px footprint and fade by width (avoids dropouts).
                fx.alpha *= pw;
                pw = 1.0;
            }
            push(stroke::stroke_polygons(&lines, pw * 0.5), FillRule::NonZero, Shader::new(&Paint::Solid(st.color), &m, &fx));
        }
    }
    out
}

fn pixel_bounds(polys: &[Vec<Vec2>], surface: IRect) -> Option<IRect> {
    let mut r = Rect::EMPTY;
    for p in polys.iter().flatten() {
        r.include(*p);
    }
    if !(r.max.x >= r.min.x && r.max.y >= r.min.y) {
        return None; // no points
    }
    let b = IRect {
        x0: (r.min.x.floor().max(surface.x0 as f32)) as i32,
        y0: (r.min.y.floor().max(surface.y0 as f32)) as i32,
        x1: (r.max.x.ceil().min(surface.x1 as f32)) as i32,
        y1: (r.max.y.ceil().min(surface.y1 as f32)) as i32,
    };
    (!b.is_empty()).then_some(b)
}

/// Composites coverage `cov` (over `b`) of `op` into the band whose first row is `band_y0`,
/// scaled by the band's clip coverage `mask` when the op is clipped.
#[allow(clippy::too_many_arguments)]
fn composite(cov: &Coverage, op: &Prepared, b: IRect, band_y0: i32, w: usize, rows: &mut [Premul], c: &mut Vec<f32>, mask: Option<&[f32]>) {
    let bw = b.width();
    c.clear();
    c.resize(bw, 0.0);
    let solid = op.shader.is_solid();
    for ry in 0..b.height() {
        cov.resolve_row(ry, op.rule, c);
        let py = b.y0 + ry as i32;
        let start = (py - band_y0) as usize * w + b.x0 as usize;
        if let Some(mask) = mask {
            let Some(m) = mask.get(start..start + bw) else { continue };
            for (k, m) in c.iter_mut().zip(m) {
                *k *= m;
            }
        }
        let Some(dst) = rows.get_mut(start..start + bw) else { continue };
        for (i, (d, &k)) in dst.iter_mut().zip(c.iter()).enumerate() {
            if k <= 1e-4 {
                continue;
            }
            let s = match solid {
                Some(s) => s,
                None => op.shader.at(b.x0 + i as i32, py),
            };
            let sa = s[3] * k;
            let inv = 1.0 - sa;
            d[0] = s[0] * k + d[0] * inv;
            d[1] = s[1] * k + d[1] * inv;
            d[2] = s[2] * k + d[2] * inv;
            d[3] = sa + d[3] * inv;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anicraft_doc::{ElementId, LayerId, sample};
    use anicraft_geom::Path;
    use std::sync::Arc;

    fn item(path: Path, fill: Paint) -> DisplayItem {
        DisplayItem {
            path: Arc::new(path),
            transform: Affine::IDENTITY,
            fill: Some(fill),
            stroke: None,
            fill_rule: FillRule::NonZero,
            color: ColorEffect::default(),
            layer: LayerId(0),
            element: ElementId(0),
            outline: None,
            clip: None,
            masks: None,
        }
    }

    fn list(items: Vec<DisplayItem>) -> DisplayList {
        DisplayList { items, clip_parents: Vec::new() }
    }

    #[test]
    fn mask_reveals_only_its_fill_area() {
        // A 40 × 40 square masked by a circle of radius 10: only the circle's area shows, in
        // the square's colour; the mask's own colour, stroke and opacity play no part.
        let mut square = item(Path::rect(0.0, 0.0, 40.0, 40.0), Paint::Solid(Rgba::new(0.0, 0.0, 1.0, 1.0)));
        square.clip = Some(0);
        let mut mask = item(Path::ellipse(20.0, 20.0, 10.0, 10.0), Paint::Solid(Rgba::new(1.0, 0.0, 0.0, 0.2)));
        mask.masks = Some(0);
        mask.stroke = Some(anicraft_doc::Stroke { color: Rgba::BLACK, width: 8.0 });
        let dl = DisplayList { items: vec![square, mask], clip_parents: vec![None] };
        let (pm, _) = render(&dl, &view(40, 40)).unwrap();
        // Pixel for pixel, the same as the circle drawn in the square's colour.
        let circle = item(Path::ellipse(20.0, 20.0, 10.0, 10.0), Paint::Solid(Rgba::new(0.0, 0.0, 1.0, 1.0)));
        let (plain, _) = render(&list(vec![circle]), &view(40, 40)).unwrap();
        assert!(pm.data.iter().zip(&plain.data).all(|(a, b)| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)));
        let area = alpha_sum(&plain);
        assert_eq!(pm.pixel(20, 20).unwrap(), [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(pm.pixel(2, 2).unwrap()[3], 0.0);
        // Two overlapping mask shapes reveal their union, not a cancelled overlap.
        let mut dl2 = dl.clone();
        let mut m2 = item(Path::rect(20.0, 0.0, 20.0, 40.0), Paint::Solid(Rgba::WHITE));
        m2.masks = Some(0);
        dl2.items.push(m2);
        let (pm2, _) = render(&dl2, &view(40, 40)).unwrap();
        assert_eq!(pm2.pixel(25, 20).unwrap()[3], 1.0);
        assert_eq!(pm2.pixel(35, 2).unwrap()[3], 1.0);
        assert!((alpha_sum(&pm2) - (800.0 + area / 2.0)).abs() < 8.0, "{}", alpha_sum(&pm2));
    }

    #[test]
    fn empty_mask_hides_everything_and_unclipped_items_are_untouched() {
        let mut clipped = item(Path::rect(0.0, 0.0, 8.0, 8.0), Paint::Solid(Rgba::BLACK));
        clipped.clip = Some(0);
        let free = item(Path::rect(0.0, 0.0, 2.0, 2.0), Paint::Solid(Rgba::BLACK));
        let (pm, st) = render(&DisplayList { items: vec![clipped, free.clone()], clip_parents: vec![None] }, &view(8, 8)).unwrap();
        assert_eq!(st.drawn, 1);
        assert_eq!(pm, render(&list(vec![free]), &view(8, 8)).unwrap().0);
    }

    #[test]
    fn nested_clips_intersect_and_bad_links_draw_nothing() {
        let mut fill = item(Path::rect(0.0, 0.0, 64.0, 64.0), Paint::Solid(Rgba::BLACK));
        fill.clip = Some(1);
        let mut outer = item(Path::rect(0.0, 0.0, 32.0, 64.0), Paint::Solid(Rgba::BLACK));
        outer.masks = Some(0);
        let mut inner = item(Path::rect(0.0, 16.0, 64.0, 32.0), Paint::Solid(Rgba::BLACK));
        inner.masks = Some(1);
        let items = vec![fill, outer, inner];
        // Clip 1 lies inside clip 0: a 32 × 32 square across two render bands survives.
        let (pm, _) = render(&DisplayList { items: items.clone(), clip_parents: vec![None, Some(0)] }, &view(64, 64)).unwrap();
        assert!((alpha_sum(&pm) - 32.0 * 32.0).abs() < 1e-2, "{}", alpha_sum(&pm));
        // A parent link pointing forward (malformed) or out of range masks everything out.
        for parents in [vec![Some(1), Some(1)], vec![None, Some(9)]] {
            let (pm, _) = render(&DisplayList { items: items.clone(), clip_parents: parents }, &view(64, 64)).unwrap();
            assert_eq!(alpha_sum(&pm), 0.0);
        }
    }

    fn view(w: u32, h: u32) -> View {
        View { transform: Affine::IDENTITY, width: w, height: h, background: None }
    }

    fn alpha_sum(p: &Pixmap) -> f32 {
        p.data.iter().map(|c| c[3]).sum()
    }

    #[test]
    fn circle_area_matches_pi_r2() {
        let r = 40.0;
        let dl = list(vec![item(Path::ellipse(50.0, 50.0, r, r), Paint::Solid(Rgba::BLACK))]);
        let (pm, _) = render(&dl, &view(100, 100)).unwrap();
        let area = std::f32::consts::PI * r * r;
        let err = (alpha_sum(&pm) - area).abs() / area;
        assert!(err < 0.005, "relative error {err}");
    }

    #[test]
    fn rect_is_pixel_exact_and_clipped() {
        let dl = list(vec![item(Path::rect(-10.0, 2.0, 15.0, 3.0), Paint::Solid(Rgba::WHITE))]);
        let (pm, _) = render(&dl, &view(8, 8)).unwrap();
        assert_eq!(pm.pixel(0, 2).unwrap(), [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(pm.pixel(4, 4).unwrap(), [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(pm.pixel(5, 3).unwrap()[3], 0.0);
        assert_eq!(pm.pixel(0, 1).unwrap()[3], 0.0);
        assert!((alpha_sum(&pm) - 15.0).abs() < 1e-3);
    }

    #[test]
    fn stroke_covers_expected_area() {
        let mut it = item(Path::line(Vec2::new(10.0, 20.0), Vec2::new(90.0, 20.0)), Paint::Solid(Rgba::BLACK));
        it.fill = None;
        it.stroke = Some(anicraft_doc::Stroke { color: Rgba::BLACK, width: 4.0 });
        let (pm, _) = render(&list(vec![it]),&view(100, 40)).unwrap();
        // 80 × 4 body plus two round caps (π · 2²)
        let expected = 80.0 * 4.0 + std::f32::consts::PI * 4.0;
        assert!((alpha_sum(&pm) - expected).abs() / expected < 0.01, "{}", alpha_sum(&pm));
    }

    #[test]
    fn src_over_order() {
        let a = item(Path::rect(0.0, 0.0, 4.0, 4.0), Paint::Solid(Rgba::new(1.0, 0.0, 0.0, 1.0)));
        let b = item(Path::rect(0.0, 0.0, 4.0, 4.0), Paint::Solid(Rgba::new(0.0, 0.0, 1.0, 0.5)));
        let (pm, _) = render(&list(vec![a, b]),&view(4, 4)).unwrap();
        let p = pm.pixel(1, 1).unwrap();
        assert!((p[0] - 0.5).abs() < 1e-5 && (p[2] - 0.5).abs() < 1e-5 && (p[3] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn demo_renders_deterministically() {
        let d = sample::demo();
        let a = render_frame(&d, 7, 0.5).unwrap();
        let b = render_frame(&d, 7, 0.5).unwrap();
        assert_eq!(a.to_rgba8(), b.to_rgba8());
        assert_eq!((a.width, a.height), (640, 360));
        // Frames differ because things move.
        let c = render_frame(&d, 20, 0.5).unwrap();
        assert_ne!(a.to_rgba8(), c.to_rgba8());
        let png = a.encode_png().unwrap();
        assert_eq!(&png[1..4], b"PNG");
    }

    #[test]
    fn offscreen_items_are_culled_without_changing_pixels() {
        let on = item(Path::rect(2.0, 2.0, 4.0, 4.0), Paint::Solid(Rgba::BLACK));
        let off = item(Path::ellipse(500.0, 500.0, 10.0, 10.0), Paint::Solid(Rgba::BLACK));
        let (a, sa) = render(&list(vec![on.clone(), off]),&view(16, 16)).unwrap();
        let (b, _) = render(&list(vec![on]),&view(16, 16)).unwrap();
        assert_eq!(sa.culled, 1);
        assert_eq!(sa.drawn, 1);
        assert_eq!(a, b);
    }

    #[test]
    fn band_boundaries_do_not_change_coverage() {
        // A rectangle with fractional edges spanning three 32-row bands keeps its exact area.
        let dl = list(vec![item(Path::rect(3.25, 20.5, 40.0, 61.75), Paint::Solid(Rgba::BLACK))]);
        let (pm, _) = render(&dl, &view(64, 100)).unwrap();
        assert!((alpha_sum(&pm) - 40.0 * 61.75).abs() < 1e-2, "{}", alpha_sum(&pm));
        // and a full row inside the shape is fully covered on both sides of a band edge
        for y in [31, 32] {
            assert_eq!(pm.pixel(10, y).unwrap()[3], 1.0);
        }
    }

    #[test]
    fn many_shapes_render_identically_across_runs() {
        let d = sample::stress(4, 300, 24);
        let a = render_frame(&d, 5, 0.5).unwrap();
        for _ in 0..3 {
            assert_eq!(render_frame(&d, 5, 0.5).unwrap(), a);
        }
    }

    #[test]
    fn bitmap_rotation_and_mask_match_analytic_pixels() {
        let image = std::sync::Arc::new(anicraft_doc::Bitmap::new(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 128]).unwrap());
        let mut bitmap = item(Path::rect(0.0, 0.0, 2.0, 1.0), Paint::Bitmap { image, transform: Affine::IDENTITY });
        bitmap.transform = Affine { m: [0.0, 1.0, -1.0, 0.0, 2.0, 1.0] };
        let mut mask = item(Path::rect(1.0, 2.0, 1.0, 1.0), Paint::Solid(Rgba::WHITE));
        mask.masks = Some(0);
        let pixels = render(&list(vec![bitmap.clone()]), &view(4, 4)).unwrap().0;
        assert_eq!(pixels.pixel(1, 1).unwrap(), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(pixels.to_rgba8()[(2 * 4 + 1) * 4..(2 * 4 + 2) * 4], [0, 255, 0, 128]);
        bitmap.clip = Some(0);
        let pixels = render(&DisplayList { items: vec![bitmap, mask], clip_parents: vec![None] }, &view(4, 4)).unwrap().0;
        assert_eq!(pixels.pixel(1, 1).unwrap(), [0.0; 4]);
        assert!(pixels.pixel(1, 2).unwrap()[3] > 0.5);
    }

    #[test]
    fn huge_surface_is_refused() {
        assert!(matches!(Pixmap::new(100_000, 100_000), Err(RenderError::TooLarge(..))));
    }
}
