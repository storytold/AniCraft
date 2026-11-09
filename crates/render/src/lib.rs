//! AniCraft CPU oracle renderer (layer L2).
//!
//! Deterministic: the same display list and view always produce the same bytes, whatever the
//! thread count. Each output row is written by exactly one task and shapes composite in order.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod paint;
pub mod raster;
pub mod stroke;
mod filters;
pub mod lottie;
pub mod svg;

use anicraft_doc::{BlendMode, ColorEffect, DisplayItem, DisplayList, Document, Paint, Rgba, evaluate};
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
    // The background is filled band by band, in parallel, just before each band draws (a
    // serial fill of a 1080p float buffer cost as much as all the drawing).
    let background = view.background.map(premultiply);
    let surface = IRect { x0: 0, y0: 0, x1: view.width as i32, y1: view.height as i32 };

    // Filtered groups are drawn on their own, filtered, and come back as one image each.
    let runs = filtered_runs(dl);
    let mut in_run = vec![false; dl.items.len()];
    for r in &runs {
        for f in in_run.iter_mut().take(r.end).skip(r.start) {
            *f = true;
        }
    }

    // Phase 1 (parallel over items): cull, flatten, stroke and build shaders. Output keeps
    // display-list order.
    let mut prepared: Vec<Vec<Prepared>> = dl.items.par_iter().zip(&in_run).map(|(item, &skip)| if skip { Vec::new() } else { prepare(item, &view.transform, surface) }).collect();
    for r in &runs {
        if let (Some(op), Some(slot)) = (filtered_group(dl, r, view, surface)?, prepared.get_mut(r.start)) {
            slot.push(op);
        }
    }
    let mut stats = RenderStats { items: dl.items.len(), ..Default::default() };
    let mut ops: Vec<Prepared> = Vec::with_capacity(prepared.len());
    let mut mask_ops: Vec<Prepared> = Vec::new();
    for (i, p) in prepared.into_iter().enumerate() {
        if p.is_empty() && !in_run.get(i).copied().unwrap_or(false) {
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
    let chains = group_chains(&dl.groups);
    pm.data.par_chunks_mut(BAND_ROWS * w.max(1)).enumerate().for_each(|(bi, rows)| {
        if let Some(bg) = background {
            rows.fill(bg);
        }
        let y0 = (bi * BAND_ROWS) as i32;
        let band = IRect { x0: 0, y0, x1: surface.x1, y1: (y0 + BAND_ROWS as i32).min(surface.y1) };
        let mut cov = Coverage::new(IRect { x0: 0, y0: 0, x1: 0, y1: 0 });
        let mut row = Vec::new();
        let mut band_clips = BandClips::default();
        // Open groups, outermost first: each composites into its own band buffer, which blends
        // into the one below at the group's alpha when the group's run of ops ends.
        let mut open: Vec<OpenGroup> = Vec::new();
        for op in &ops {
            let b = IRect { x0: op.bounds.x0, y0: op.bounds.y0.max(band.y0), x1: op.bounds.x1, y1: op.bounds.y1.min(band.y1) };
            if b.is_empty() {
                continue;
            }
            let want = op.group.and_then(|g| chains.get(g as usize)).map_or(&[][..], Vec::as_slice);
            let keep = open.iter().zip(want).take_while(|(o, g)| o.id == **g).count();
            while open.len() > keep {
                close_group(&mut open, rows, &dl.groups, band.y0, w);
            }
            for &g in want.iter().skip(keep) {
                open.push(OpenGroup { id: g, buf: vec![[0.0; 4]; rows.len()], dirty: NO_PIXELS });
            }
            let mask = op.clip.map(|c| band_clips.get(&clips, c, band, w, &mut cov, &mut row));
            cov.reset(b);
            for poly in &op.polys {
                cov.add_polygon(poly);
            }
            match open.last_mut() {
                Some(top) => {
                    top.dirty = top.dirty.union(&b);
                    composite(&cov, op, b, band.y0, w, &mut top.buf, &mut row, mask.as_deref().map(Vec::as_slice));
                }
                None => composite(&cov, op, b, band.y0, w, rows, &mut row, mask.as_deref().map(Vec::as_slice)),
            }
        }
        while !open.is_empty() {
            close_group(&mut open, rows, &dl.groups, band.y0, w);
        }
    });
    // Tens of thousands of small polygon buffers: free them on the pool, off the frame's
    // critical path (it cost about 2 ms at 1080p).
    rayon::spawn(move || drop((ops, clips)));
    Ok((pm, stats))
}

/// Why a display list needs the full renderer rather than plain fills (see [`plain_fills`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NeedsFull {
    /// Mask layers: items drawn inside a clip, or mask artwork.
    Clips,
    /// Group opacity, blend modes or filters (symbol instances composited on their own).
    Groups,
    /// Bitmap fills.
    Bitmaps,
}

impl std::fmt::Display for NeedsFull {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            NeedsFull::Clips => "masks",
            NeedsFull::Groups => "group opacity, blend modes or filters",
            NeedsFull::Bitmaps => "bitmap fills",
        })
    }
}

/// One fill operation in surface pixels, exactly as this renderer prepares it: flattened at
/// [`TOLERANCE`], strokes already outlined by the stroker, the paint's shader (with its gradient
/// table) built, and pixel bounds clamped to the surface. Drawing every fill in order, with
/// analytic coverage, the fill rule and src-over, reproduces [`render`] for a display list that
/// [`plain_fills`] accepts. Other renderers (the GPU stage, ADR 0019) start from these, so the
/// geometry and the colours are the oracle's own.
pub struct Fill {
    pub polys: Vec<Vec<Vec2>>,
    pub rule: FillRule,
    pub shader: Shader,
    pub bounds: IRect,
}

/// The display list as plain fills, in drawing order, with the same culling and statistics as
/// [`render`]; or why it needs the full renderer (masks, groups, bitmaps).
pub fn plain_fills(dl: &DisplayList, view: &View) -> Result<(Vec<Fill>, RenderStats), NeedsFull> {
    for it in &dl.items {
        if it.clip.is_some() || it.masks.is_some() {
            return Err(NeedsFull::Clips);
        }
        if it.group.is_some() {
            return Err(NeedsFull::Groups);
        }
        if !it.guide && it.outline.is_none() && matches!(it.fill, Some(Paint::Bitmap { .. })) {
            return Err(NeedsFull::Bitmaps);
        }
    }
    let surface = IRect { x0: 0, y0: 0, x1: view.width as i32, y1: view.height as i32 };
    let prepared: Vec<Vec<Prepared>> = dl.items.par_iter().map(|item| prepare(item, &view.transform, surface)).collect();
    let mut stats = RenderStats { items: dl.items.len(), ..Default::default() };
    let mut out = Vec::with_capacity(prepared.len());
    for p in prepared {
        if p.is_empty() {
            stats.culled += 1;
        }
        for op in p {
            if matches!(op.shader, Shader::Bitmap { .. } | Shader::Image { .. }) {
                return Err(NeedsFull::Bitmaps);
            }
            stats.edges += op.polys.iter().map(Vec::len).sum::<usize>();
            stats.pixels_touched += (op.bounds.width() * op.bounds.height()) as u64;
            out.push(Fill { polys: op.polys, rule: op.rule, shader: op.shader, bounds: op.bounds });
        }
    }
    stats.drawn = out.len();
    Ok((out, stats))
}

/// A run of items inside a filtered group that has no filtered group around it.
struct FilteredRun {
    group: u32,
    start: usize,
    end: usize,
}

fn filtered_runs(dl: &DisplayList) -> Vec<FilteredRun> {
    if dl.groups.iter().all(|g| g.filters.is_empty()) {
        return Vec::new();
    }
    let chains = group_chains(&dl.groups);
    let top = |i: usize| -> Option<u32> {
        let g = dl.items.get(i)?.group?;
        chains.get(g as usize)?.iter().copied().find(|&c| dl.groups.get(c as usize).is_some_and(|x| !x.filters.is_empty()))
    };
    let mut out: Vec<FilteredRun> = Vec::new();
    for i in 0..dl.items.len() {
        match (top(i), out.last_mut()) {
            (Some(g), Some(last)) if last.group == g && last.end == i => last.end = i + 1,
            (Some(g), _) => out.push(FilteredRun { group: g, start: i, end: i + 1 }),
            (None, _) => {}
        }
    }
    out
}

/// Largest buffer a filtered group is drawn into (pixels).
const MAX_FILTER_PIXELS: i64 = 64 * 1024 * 1024;

/// Draws a filtered group's items on their own (the group neutral, clips from outside it
/// left for later), applies its filters, and returns the result as one image op in the group,
/// clipped by the clip in force where the instance is.
fn filtered_group(dl: &DisplayList, run: &FilteredRun, view: &View, surface: IRect) -> Result<Option<Prepared>, RenderError> {
    let Some(group) = dl.groups.get(run.group as usize) else { return Ok(None) };
    let items = dl.items.get(run.start..run.end).unwrap_or(&[]);
    let scale = view.transform.mean_scale();
    let margin = (group.filters.iter().map(anicraft_doc::Filter::margin).sum::<f32>() * scale).ceil() + 2.0;
    // Where it can draw: its artwork's bounds, grown by how far the filters spread.
    let mut r = Rect::EMPTY;
    for it in items.iter().filter(|i| i.masks.is_none() && !i.guide) {
        let pad = it.stroke.as_ref().map_or(0.0, anicraft_doc::Stroke::reach) + 1.0;
        r = r.union(&it.path.bounds().inflate(pad).transformed(&view.transform.mul(&it.transform)));
    }
    if r.is_empty() || !r.min.x.is_finite() || !r.max.y.is_finite() {
        return Ok(None);
    }
    let reach = margin as i32;
    let limit = IRect { x0: surface.x0 - reach, y0: surface.y0 - reach, x1: surface.x1 + reach, y1: surface.y1 + reach };
    let b = IRect { x0: r.min.x.floor() as i32 - reach, y0: r.min.y.floor() as i32 - reach, x1: r.max.x.ceil() as i32 + reach, y1: r.max.y.ceil() as i32 + reach }.intersect(&limit);
    if b.is_empty() || (b.width() as i64) * (b.height() as i64) > MAX_FILTER_PIXELS {
        return Ok(None);
    }
    // The group's own clips (masks inside the symbol) apply before filtering; others after.
    let inner: std::collections::HashSet<u32> = items.iter().filter_map(|i| i.masks).collect();
    let mut groups = dl.groups.clone();
    if let Some(g) = groups.get_mut(run.group as usize) {
        *g = anicraft_doc::Group { parent: None, alpha: 1.0, blend: BlendMode::Normal, filters: Default::default(), clip: None };
    }
    let sub = DisplayList {
        items: items.iter().map(|i| DisplayItem { clip: i.clip.filter(|c| inner.contains(c)), ..i.clone() }).collect(),
        clip_parents: dl.clip_parents.iter().map(|p| p.filter(|c| inner.contains(c))).collect(),
        groups,
    };
    let sub_view = View { transform: Affine::translate(-b.x0 as f32, -b.y0 as f32).mul(&view.transform), width: b.width() as u32, height: b.height() as u32, background: None };
    let (mut pm, _) = render(&sub, &sub_view)?;
    filters::apply(&mut pm.data, b.width(), b.height(), &group.filters, scale);
    let shown = b.intersect(&surface);
    if shown.is_empty() {
        return Ok(None);
    }
    let (x0, y0, x1, y1) = (shown.x0 as f32, shown.y0 as f32, shown.x1 as f32, shown.y1 as f32);
    Ok(Some(Prepared {
        polys: vec![vec![Vec2::new(x0, y0), Vec2::new(x1, y0), Vec2::new(x1, y1), Vec2::new(x0, y1)]],
        rule: FillRule::NonZero,
        shader: Shader::Image { data: std::sync::Arc::new(pm.data), x0: b.x0, y0: b.y0, w: b.width() },
        bounds: shown,
        clip: group.clip,
        masks: None,
        group: Some(run.group),
    }))
}

/// A group being composited in one band: its own buffer (laid out like the band's rows) and
/// the part of it drawn into.
struct OpenGroup {
    id: u32,
    buf: Vec<Premul>,
    dirty: IRect,
}

/// Each group's chain of groups, outermost first. A malformed parent link (not earlier in the
/// list) is treated as no parent.
pub(crate) fn group_chains(groups: &[anicraft_doc::Group]) -> Vec<Vec<u32>> {
    let mut out: Vec<Vec<u32>> = Vec::with_capacity(groups.len());
    for (i, g) in groups.iter().enumerate() {
        let mut chain = g.parent.filter(|&p| (p as usize) < i).and_then(|p| out.get(p as usize)).cloned().unwrap_or_default();
        chain.push(i as u32);
        out.push(chain);
    }
    out
}

/// Blends the innermost open group into what is below it, at the group's alpha and in its
/// blend mode.
fn close_group(open: &mut Vec<OpenGroup>, rows: &mut [Premul], groups: &[anicraft_doc::Group], band_y0: i32, w: usize) {
    let Some(g) = open.pop() else { return };
    let alpha = groups.get(g.id as usize).map_or(1.0, |x| if x.alpha.is_finite() { x.alpha.clamp(0.0, 1.0) } else { 1.0 });
    let mode = groups.get(g.id as usize).map_or(BlendMode::Normal, |x| x.blend);
    let dst: &mut [Premul] = match open.last_mut() {
        Some(below) => {
            below.dirty = below.dirty.union(&g.dirty);
            &mut below.buf
        }
        None => rows,
    };
    let d = g.dirty;
    if d.is_empty() || alpha <= 0.0 {
        return;
    }
    if !matches!(mode, BlendMode::Normal | BlendMode::Layer) {
        for py in d.y0..d.y1 {
            let start = (py - band_y0) as usize * w + d.x0 as usize;
            let (Some(src), Some(out)) = (g.buf.get(start..start + d.width()), dst.get_mut(start..start + d.width())) else { continue };
            for (o, s) in out.iter_mut().zip(src) {
                if s[3] > 0.0 {
                    *o = blend(mode, [s[0] * alpha, s[1] * alpha, s[2] * alpha, s[3] * alpha], *o);
                }
            }
        }
        return;
    }
    for py in d.y0..d.y1 {
        let start = (py - band_y0) as usize * w + d.x0 as usize;
        let (Some(src), Some(out)) = (g.buf.get(start..start + d.width()), dst.get_mut(start..start + d.width())) else { continue };
        for (o, s) in out.iter_mut().zip(src) {
            let sa = s[3] * alpha;
            if sa <= 0.0 {
                continue;
            }
            let inv = 1.0 - sa;
            o[0] = s[0] * alpha + o[0] * inv;
            o[1] = s[1] * alpha + o[1] * inv;
            o[2] = s[2] * alpha + o[2] * inv;
            o[3] = sa + o[3] * inv;
        }
    }
}

/// Source `s` over backdrop `b` (both premultiplied) in blend mode `mode`: the W3C
/// compositing formula `Cr = (1 − αb)·Cs + (1 − αs)·Cb + αs·αb·B(cb, cs)` for the separable
/// modes; Alpha and Erase keep or clear the backdrop by the source's opacity.
pub(crate) fn blend(mode: BlendMode, s: Premul, b: Premul) -> Premul {
    let (sa, ba) = (s[3].clamp(0.0, 1.0), b[3].clamp(0.0, 1.0));
    match mode {
        BlendMode::Alpha => return [b[0] * sa, b[1] * sa, b[2] * sa, ba * sa],
        BlendMode::Erase => {
            let k = 1.0 - sa;
            return [b[0] * k, b[1] * k, b[2] * k, ba * k];
        }
        _ => {}
    }
    let un = |c: f32, a: f32| if a > 1e-6 { (c / a).clamp(0.0, 1.0) } else { 0.0 };
    let f = |cb: f32, cs: f32| -> f32 {
        let hard = |cb: f32, cs: f32| if cs <= 0.5 { cb * 2.0 * cs } else { let t = 2.0 * cs - 1.0; cb + t - cb * t };
        match mode {
            BlendMode::Multiply => cb * cs,
            BlendMode::Screen => cb + cs - cb * cs,
            BlendMode::Darken => cb.min(cs),
            BlendMode::Lighten => cb.max(cs),
            BlendMode::Difference => (cb - cs).abs(),
            BlendMode::Add => (cb + cs).min(1.0),
            BlendMode::Subtract => (cb - cs).max(0.0),
            BlendMode::Invert => 1.0 - cb,
            BlendMode::HardLight => hard(cb, cs),
            BlendMode::Overlay => hard(cs, cb),
            BlendMode::Normal | BlendMode::Layer | BlendMode::Alpha | BlendMode::Erase => cs,
        }
    };
    let mut out = [0.0; 4];
    for i in 0..3 {
        let (cs, cb) = (un(s[i], sa), un(b[i], ba));
        out[i] = (1.0 - ba) * s[i] + (1.0 - sa) * b[i] + sa * ba * f(cb, cs);
    }
    out[3] = sa + ba - sa * ba;
    out
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
    /// Composited inside this group (and its parents).
    group: Option<u32>,
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
    // Guide artwork is for editing only: it never renders.
    if item.guide {
        return out;
    }
    // Mask artwork is its fill area only: no strokes, no opacity (as animators expect).
    let is_mask = item.masks.is_some();
    if is_mask && item.fill.is_none() {
        return out;
    }
    // Cull before flattening: control-point bounds are a cheap, conservative superset.
    let pad = if is_mask { 1.0 } else { item.stroke.as_ref().map_or(0.0, anicraft_doc::Stroke::reach) + 1.0 };
    let bounds = item.path.bounds().inflate(pad).transformed(&m);
    let surface_rect = Rect::new(0.0, 0.0, surface.x1 as f32, surface.y1 as f32);
    if bounds.is_empty() || bounds.intersect(&surface_rect).is_empty() {
        return out;
    }
    let lines = item.path.flatten(&m, TOLERANCE);
    if lines.is_empty() {
        return out;
    }
    let (clip, masks, group) = (item.clip, item.masks, item.group);
    let mut push = |polys: Vec<Vec<Vec2>>, rule, shader: Shader| {
        if shader.is_solid().is_some_and(|c| c[3] <= 0.0) {
            return;
        }
        if let Some(b) = pixel_bounds(&polys, surface) {
            out.push(Prepared { polys, rule, shader, bounds: b, clip, masks, group });
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
    // The stroke is outlined from the flattened lines first, so the fill can then take them
    // without a copy; it is still drawn after the fill.
    let stroke = item.stroke.as_ref().and_then(|st| {
        let mut fx: ColorEffect = item.color;
        let mut pw = st.width * m.mean_scale();
        if !(pw.is_finite() && pw > 0.0) {
            return None;
        }
        let shader = |fx: &ColorEffect| Shader::new(&Paint::Solid(st.color), &m, fx, None);
        if let Some(profile) = &st.profile {
            // Variable width (ADR 0018). Thin strokes fade by their widest part instead.
            let widest = pw * profile.max_factor();
            if !(widest.is_finite() && widest > 0.0) {
                return None;
            }
            if widest < 1.0 {
                fx.alpha *= widest;
                pw /= widest;
            }
            let polys = stroke::variable_polygons(&lines, pw * 0.5, profile, m.inverse().as_ref(), m.determinant() < 0.0);
            return Some((polys, shader(&fx)));
        }
        if pw < 1.0 {
            // Thin strokes: keep a 1 px footprint and fade by width (avoids dropouts).
            fx.alpha *= pw;
            pw = 1.0;
        }
        Some((stroke::stroke_polygons(&lines, pw * 0.5), shader(&fx)))
    });
    if let Some(fill) = &item.fill {
        let polys: Vec<Vec<Vec2>> = lines.into_iter().map(|l| l.points).collect();
        push(polys, item.fill_rule, Shader::new(fill, &m, &item.color, item.image.as_ref()));
    }
    if let Some((polys, shader)) = stroke {
        push(polys, FillRule::NonZero, shader);
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
            guide: false,
            group: None,
            image: None,
        }
    }

    fn list(items: Vec<DisplayItem>) -> DisplayList {
        DisplayList { items, clip_parents: Vec::new(), groups: Vec::new() }
    }

    #[test]
    fn mask_reveals_only_its_fill_area() {
        // A 40 × 40 square masked by a circle of radius 10: only the circle's area shows, in
        // the square's colour; the mask's own colour, stroke and opacity play no part.
        let mut square = item(Path::rect(0.0, 0.0, 40.0, 40.0), Paint::Solid(Rgba::new(0.0, 0.0, 1.0, 1.0)));
        square.clip = Some(0);
        let mut mask = item(Path::ellipse(20.0, 20.0, 10.0, 10.0), Paint::Solid(Rgba::new(1.0, 0.0, 0.0, 0.2)));
        mask.masks = Some(0);
        mask.stroke = Some(anicraft_doc::Stroke::new(Rgba::BLACK, 8.0));
        let dl = DisplayList { items: vec![square, mask], clip_parents: vec![None], groups: Vec::new() };
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
    fn groups_fade_as_one() {
        // A red square under a blue one, overlapping, in a group at 50% over white.
        let white = View { background: Some(Rgba::WHITE), ..view(12, 4) };
        let mut red = item(Path::rect(0.0, 0.0, 8.0, 4.0), Paint::Solid(Rgba::new(1.0, 0.0, 0.0, 1.0)));
        let mut blue = item(Path::rect(4.0, 0.0, 8.0, 4.0), Paint::Solid(Rgba::new(0.0, 0.0, 1.0, 1.0)));
        red.group = Some(0);
        blue.group = Some(0);
        let grouped = DisplayList { items: vec![red.clone(), blue.clone()], clip_parents: vec![], groups: vec![anicraft_doc::Group { parent: None, alpha: 0.5, blend: Default::default(), filters: Default::default(), clip: None }] };
        let (pm, _) = render(&grouped, &white).unwrap();
        let near = |p: Premul, q: [f32; 4]| p.iter().zip(q).all(|(a, b)| (a - b).abs() < 1e-4);
        // In the overlap only the top square shows through: half blue, half white, no red.
        assert!(near(pm.pixel(6, 1).unwrap(), [0.5, 0.5, 1.0, 1.0]), "{:?}", pm.pixel(6, 1));
        assert!(near(pm.pixel(1, 1).unwrap(), [1.0, 0.5, 0.5, 1.0]));
        // Fading each square on its own would let the red show through the blue.
        let (mut r2, mut b2) = (red, blue);
        (r2.group, b2.group) = (None, None);
        r2.color.alpha = 0.5;
        b2.color.alpha = 0.5;
        let (each, _) = render(&list(vec![r2, b2]), &white).unwrap();
        assert!(each.pixel(6, 1).unwrap()[0] > 0.2, "per-shape fading mixes in red");
        // Nested groups multiply; a group with a bad parent link is a top-level group.
        let mut inner = item(Path::rect(0.0, 0.0, 12.0, 4.0), Paint::Solid(Rgba::BLACK));
        inner.group = Some(1);
        let nested = DisplayList {
            items: vec![inner],
            clip_parents: vec![],
            groups: vec![anicraft_doc::Group { parent: None, alpha: 0.5, blend: Default::default(), filters: Default::default(), clip: None }, anicraft_doc::Group { parent: Some(0), alpha: 0.5, blend: Default::default(), filters: Default::default(), clip: None }],
        };
        let (n, _) = render(&nested, &white).unwrap();
        assert!(near(n.pixel(3, 2).unwrap(), [0.75, 0.75, 0.75, 1.0]), "{:?}", n.pixel(3, 2));
        let mut bad = nested.clone();
        bad.groups[1].parent = Some(7);
        let (b, _) = render(&bad, &white).unwrap();
        assert!(near(b.pixel(3, 2).unwrap(), [0.5, 0.5, 0.5, 1.0]));
        // Spanning several render bands changes nothing.
        let mut tall = item(Path::rect(0.0, 0.0, 4.0, 100.0), Paint::Solid(Rgba::BLACK));
        tall.group = Some(0);
        let (t, _) = render(&DisplayList { items: vec![tall], clip_parents: vec![], groups: vec![anicraft_doc::Group { parent: None, alpha: 0.25, blend: Default::default(), filters: Default::default(), clip: None }] }, &View { background: Some(Rgba::WHITE), ..view(4, 100) }).unwrap();
        assert!((0..100).all(|y| near(t.pixel(1, y).unwrap(), [0.75, 0.75, 0.75, 1.0])));
    }

    #[test]
    fn plain_fills_match_the_renderers_preparation_and_name_what_needs_more() {
        // A filled, stroked square and an off-screen circle: fill then stroke, one culled.
        let mut sq = item(Path::rect(2.0, 2.0, 8.0, 8.0), Paint::Solid(Rgba::BLACK));
        sq.stroke = Some(anicraft_doc::Stroke { color: Rgba::WHITE, width: 2.0, profile: None });
        let off = item(Path::ellipse(500.0, 500.0, 10.0, 10.0), Paint::Solid(Rgba::BLACK));
        let dl = list(vec![sq.clone(), off]);
        let (fills, st) = plain_fills(&dl, &view(16, 16)).unwrap();
        let (_, rst) = render(&dl, &view(16, 16)).unwrap();
        assert_eq!(fills.len(), 2);
        assert_eq!((st.drawn, st.culled, st.edges), (rst.drawn, rst.culled, rst.edges));
        assert!(fills[0].shader.is_solid() == Some([0.0, 0.0, 0.0, 1.0]) && fills[1].shader.is_solid() == Some([1.0; 4]));
        assert!(fills.iter().all(|f| f.bounds.x0 >= 0 && f.bounds.x1 <= 16));
        // Masks, groups and bitmaps need the full renderer.
        let mut clipped = sq.clone();
        clipped.clip = Some(0);
        assert_eq!(plain_fills(&list(vec![clipped]), &view(16, 16)).err(), Some(NeedsFull::Clips));
        let mut grouped = sq.clone();
        grouped.group = Some(0);
        assert_eq!(plain_fills(&list(vec![grouped]), &view(16, 16)).err(), Some(NeedsFull::Groups));
        let bm = item(Path::rect(0.0, 0.0, 4.0, 4.0), Paint::Bitmap { bitmap: anicraft_doc::BitmapId(1), repeat: false, matrix: Affine::IDENTITY });
        assert_eq!(plain_fills(&list(vec![bm]), &view(16, 16)).err(), Some(NeedsFull::Bitmaps));
    }

    #[test]
    fn guide_artwork_never_renders() {
        let mut g = item(Path::rect(0.0, 0.0, 8.0, 8.0), Paint::Solid(Rgba::BLACK));
        g.guide = true;
        let (pm, st) = render(&list(vec![g]), &view(8, 8)).unwrap();
        assert_eq!((alpha_sum(&pm), st.drawn), (0.0, 0));
    }

    #[test]
    fn empty_mask_hides_everything_and_unclipped_items_are_untouched() {
        let mut clipped = item(Path::rect(0.0, 0.0, 8.0, 8.0), Paint::Solid(Rgba::BLACK));
        clipped.clip = Some(0);
        let free = item(Path::rect(0.0, 0.0, 2.0, 2.0), Paint::Solid(Rgba::BLACK));
        let (pm, st) = render(&DisplayList { items: vec![clipped, free.clone()], clip_parents: vec![None], groups: Vec::new() }, &view(8, 8)).unwrap();
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
        let (pm, _) = render(&DisplayList { items: items.clone(), clip_parents: vec![None, Some(0)], groups: Vec::new() }, &view(64, 64)).unwrap();
        assert!((alpha_sum(&pm) - 32.0 * 32.0).abs() < 1e-2, "{}", alpha_sum(&pm));
        // A parent link pointing forward (malformed) or out of range masks everything out.
        for parents in [vec![Some(1), Some(1)], vec![None, Some(9)]] {
            let (pm, _) = render(&DisplayList { items: items.clone(), clip_parents: parents, groups: Vec::new() }, &view(64, 64)).unwrap();
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
        it.stroke = Some(anicraft_doc::Stroke::new(Rgba::BLACK, 4.0));
        let (pm, _) = render(&list(vec![it]),&view(100, 40)).unwrap();
        // 80 × 4 body plus two round caps (π · 2²)
        let expected = 80.0 * 4.0 + std::f32::consts::PI * 4.0;
        assert!((alpha_sum(&pm) - expected).abs() / expected < 0.01, "{}", alpha_sum(&pm));
    }

    fn stroked(path: Path, width: f32, profile: Option<anicraft_doc::WidthProfile>) -> DisplayItem {
        let mut it = item(path, Paint::Solid(Rgba::BLACK));
        it.fill = None;
        it.stroke = Some(anicraft_doc::Stroke::new(Rgba::BLACK, width).with_profile(profile));
        it
    }

    #[test]
    fn a_tapered_stroke_covers_the_profile_integral() {
        // Taper both ends on an 80-unit line, nominal width 8: the half-width is 4·f(t), f the
        // monotone cubic 0 → 1 → 0. Each half is a Hermite segment (h = ½, ends 0 and 1,
        // tangents 2 and 0), whose integral is h·(0 + 1)/2 + h²·(2 − 0)/12 = 7/24; no caps.
        let taper = anicraft_doc::ProfilePreset::TaperBoth.profile();
        let (pm, _) = render(&list(vec![stroked(Path::line(Vec2::new(10.0, 20.0), Vec2::new(90.0, 20.0)), 8.0, taper.clone())]), &view(100, 40)).unwrap();
        let expected = 2.0 * 4.0 * 80.0 * (7.0 / 12.0);
        assert!((alpha_sum(&pm) - expected).abs() / expected < 0.01, "{} vs {expected}", alpha_sum(&pm));
        // Thin at the ends, full in the middle.
        assert_eq!(pm.pixel(50, 17).unwrap()[3], 1.0);
        assert_eq!(pm.pixel(12, 17).unwrap()[3], 0.0);
        // The profile lives in the shape's space: twice the scale, four times the area.
        let mut big = stroked(Path::line(Vec2::new(10.0, 20.0), Vec2::new(90.0, 20.0)), 8.0, taper);
        big.transform = Affine::scale(2.0, 2.0);
        let (pm2, _) = render(&list(vec![big]), &view(200, 80)).unwrap();
        assert!((alpha_sum(&pm2) / alpha_sum(&pm) - 4.0).abs() < 0.04, "{}", alpha_sum(&pm2) / alpha_sum(&pm));
    }

    #[test]
    fn constant_profiles_match_uniform_strokes_and_one_side_stays_on_its_side() {
        let line = || Path::line(Vec2::new(10.0, 20.0), Vec2::new(90.0, 20.0));
        let p = |l: f32, r: f32| anicraft_doc::WidthProfile::new(vec![anicraft_doc::WidthPoint::new(0.5, l, r)]);
        // 1.5 × a 4-wide stroke is a 6-wide one: body plus round caps.
        let (wide, _) = render(&list(vec![stroked(line(), 4.0, p(1.5, 1.5))]), &view(100, 40)).unwrap();
        let expected = 80.0 * 6.0 + std::f32::consts::PI * 9.0;
        assert!((alpha_sum(&wide) - expected).abs() / expected < 0.01, "{}", alpha_sum(&wide));
        // All on the left (above a line heading right): a 4-wide band, with half-disc caps.
        let (left, _) = render(&list(vec![stroked(line(), 4.0, p(2.0, 0.0))]), &view(100, 40)).unwrap();
        let expected = 80.0 * 4.0 + std::f32::consts::PI * 4.0;
        assert!((alpha_sum(&left) - expected).abs() / expected < 0.01, "{}", alpha_sum(&left));
        assert_eq!(left.pixel(50, 17).unwrap()[3], 1.0);
        assert_eq!(left.pixel(50, 22).unwrap()[3], 0.0, "nothing on the right");
        // A mirror swaps the sides as seen on screen.
        let mut flipped = stroked(line(), 4.0, p(2.0, 0.0));
        flipped.transform = Affine::translate(0.0, 40.0).mul(&Affine::scale(1.0, -1.0));
        let (f, _) = render(&list(vec![flipped]), &view(100, 40)).unwrap();
        assert_eq!(f.pixel(50, 22).unwrap()[3], 1.0);
        assert_eq!(f.pixel(50, 17).unwrap()[3], 0.0);
    }

    /// `cargo test -p anicraft-render --release -- --ignored --nocapture variable_stroke_cost`
    #[test]
    #[ignore = "measurement"]
    fn variable_stroke_cost() {
        let strokes = |profile: Option<anicraft_doc::WidthProfile>| {
            let items = (0..300)
                .map(|i| {
                    let (x, y) = ((i % 20) as f32 * 64.0, (i / 20) as f32 * 48.0);
                    let mut p = Path::new();
                    p.move_to(Vec2::new(x, y + 40.0)).cubic_to(Vec2::new(x + 10.0, y - 20.0), Vec2::new(x + 50.0, y + 80.0), Vec2::new(x + 60.0, y + 10.0));
                    stroked(p, 6.0, profile.clone())
                })
                .collect();
            list(items)
        };
        for (name, dl) in [("uniform", strokes(None)), ("taper_both", strokes(anicraft_doc::ProfilePreset::TaperBoth.profile())), ("bulge", strokes(anicraft_doc::ProfilePreset::Bulge.profile()))] {
            let mut ms: Vec<f64> = (0..9)
                .map(|_| {
                    let t = std::time::Instant::now();
                    let _ = render(&dl, &view(1280, 720)).unwrap();
                    t.elapsed().as_secs_f64() * 1000.0
                })
                .collect();
            ms.sort_by(f64::total_cmp);
            println!("300 strokes, 1280x720, {name}: median {:.2} ms", ms[4]);
        }
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
    fn huge_surface_is_refused() {
        assert!(matches!(Pixmap::new(100_000, 100_000), Err(RenderError::TooLarge(..))));
    }

    #[test]
    fn blend_modes_follow_the_compositing_formulas() {
        let red = [1.0, 0.0, 0.0, 1.0];
        let grey = [0.5, 0.5, 0.5, 1.0];
        let close = |a: Premul, b: [f32; 4]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5);
        assert!(close(blend(BlendMode::Multiply, red, grey), [0.5, 0.0, 0.0, 1.0]));
        assert!(close(blend(BlendMode::Screen, red, grey), [1.0, 0.5, 0.5, 1.0]));
        assert!(close(blend(BlendMode::Darken, red, grey), [0.5, 0.0, 0.0, 1.0]));
        assert!(close(blend(BlendMode::Lighten, red, grey), [1.0, 0.5, 0.5, 1.0]));
        assert!(close(blend(BlendMode::Difference, red, grey), [0.5, 0.5, 0.5, 1.0]));
        assert!(close(blend(BlendMode::Add, red, grey), [1.0, 0.5, 0.5, 1.0]));
        assert!(close(blend(BlendMode::Subtract, red, grey), [0.0, 0.5, 0.5, 1.0]));
        assert!(close(blend(BlendMode::Invert, red, grey), [0.5, 0.5, 0.5, 1.0]));
        assert!(close(blend(BlendMode::Erase, red, grey), [0.0, 0.0, 0.0, 0.0]));
        assert!(close(blend(BlendMode::Alpha, [0.0, 0.0, 0.0, 0.5], grey), [0.25, 0.25, 0.25, 0.5]));
        // Over nothing, a blend mode is just the source; a clear source leaves the backdrop.
        assert!(close(blend(BlendMode::Multiply, red, [0.0; 4]), red));
        assert!(close(blend(BlendMode::Multiply, [0.0; 4], grey), grey));
    }

    #[test]
    fn a_multiply_group_darkens_what_is_below() {
        let bg = item(Path::rect(0.0, 0.0, 20.0, 10.0), Paint::Solid(Rgba::new(0.5, 0.5, 1.0, 1.0)));
        let mut top = item(Path::rect(0.0, 0.0, 10.0, 10.0), Paint::Solid(Rgba::new(1.0, 0.0, 0.0, 1.0)));
        top.group = Some(0);
        let dl = DisplayList { items: vec![bg, top], clip_parents: vec![], groups: vec![anicraft_doc::Group { parent: None, alpha: 1.0, blend: BlendMode::Multiply, filters: Default::default(), clip: None }] };
        let (pm, _) = render(&dl, &View { background: Some(Rgba::WHITE), ..view(20, 10) }).unwrap();
        let p = pm.pixel(5, 5).unwrap();
        assert!((p[0] - 0.5).abs() < 1e-3 && p[1].abs() < 1e-3 && p[2].abs() < 1e-3, "red × lavender: {p:?}");
        let q = pm.pixel(15, 5).unwrap();
        assert!((q[2] - 1.0).abs() < 1e-3, "outside the group unchanged: {q:?}");
    }
}
