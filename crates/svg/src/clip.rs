//! Clip paths and masks → mask layers (ADR 0021, ADR 0070).
//!
//! While the drawing is read, every drawn item collects the clip regions of the elements
//! around it: a `clipPath` (the union of its shapes' fills), a `mask` whose content is opaque
//! (the union of its fills, cut to the mask's region), or any other `mask`: a soft region, its
//! content as drawn, revealing by luminance (the default) or by alpha (`mask-type: alpha`),
//! onto a soft mask layer. At the end, [`Builder::layers`] resolves each item's regions:
//! * a convex region that contains the item cuts nothing and is dropped; a region that misses
//!   it hides it (it is left out);
//! * one region left is the item's mask; several are intersected exactly when all but one are
//!   convex polygons (`anicraft_geom::clip`), else one is kept and the report counts it;
//! * consecutive items with the same mask share one masked layer under one mask layer.

use crate::{Builder, ImportedElement, ImportedLayer, ImportedMask, Item, PaintSpec, Report, Style, declarations, get, hidden, length, local, name_of, paint_spec, transform};
use anicraft_doc::{ElementKind, MaskMode, Paint, Rgba, Shape, Stroke};
use anicraft_geom::clip::{clip_convex, convex_polygon};
use anicraft_geom::{Affine, FillRule, Path, Rect, Vec2};
use anicraft_xml::Element as X;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

/// Most mask layers one drawing makes; clips past these are drawn unclipped and reported.
pub const MAX_MASKS: usize = 250;
/// Deepest chain of clip paths and masks that refer to further ones.
const MAX_NESTING: usize = 8;

/// A clip region in drawing space: the union of its parts' fills.
#[derive(Debug)]
pub(crate) struct Region {
    /// Identifies the region: the clip element and the space it was used in.
    key: String,
    name: String,
    parts: Vec<ImportedElement>,
    /// The corners, when the region is one convex polygon.
    convex: Option<Vec<Vec2>>,
    /// Conservative drawing-space bounds.
    bounds: Rect,
    /// Coverage, or a soft mask's mode (then `parts` are drawn as they look and `convex` is
    /// `None`: a soft region never "cuts nothing").
    mode: MaskMode,
}

/// Conservative drawing-space bounds of an element's artwork.
fn element_bounds(e: &ImportedElement) -> Rect {
    match &e.kind {
        ElementKind::Shape(s) => s.path.bounds().inflate(s.stroke.as_ref().map_or(0.0, Stroke::reach)).transformed(&e.transform),
        ElementKind::Text(t) => t.outline.bounds().union(&Rect::new(0.0, 0.0, t.box_width.max(0.0), t.box_height.max(0.0))).transformed(&e.transform),
        ElementKind::Symbol { .. } | ElementKind::Camera | ElementKind::Group { .. } => Rect::EMPTY,
    }
}

/// A fill-only shape for mask artwork (its colour doesn't matter: a mask is its fills' area).
fn mask_part(name: String, path: Path, transform: Affine, rule: FillRule) -> ImportedElement {
    let shape = Shape { path: Arc::new(path), fill: Some(Paint::Solid(Rgba::BLACK)), stroke: None, fill_rule: rule, merge: false, warp: None };
    ImportedElement { name, transform, alpha: 1.0, kind: ElementKind::Shape(shape) }
}

impl Region {
    fn new(key: String, name: &str, parts: Vec<ImportedElement>) -> Rc<Region> {
        let bounds = parts.iter().fold(Rect::EMPTY, |r, p| r.union(&element_bounds(p)));
        let convex = match parts.as_slice() {
            [ImportedElement { kind: ElementKind::Shape(s), transform, .. }] => convex_polygon(&s.path.transformed(transform)),
            _ => None,
        };
        Rc::new(Region { key, name: name.chars().take(128).collect(), parts, convex, bounds, mode: MaskMode::Coverage })
    }

    /// A soft mask's region: its content as drawn.
    fn soft(key: String, name: &str, parts: Vec<ImportedElement>, mode: MaskMode) -> Rc<Region> {
        let bounds = parts.iter().fold(Rect::EMPTY, |r, p| r.union(&element_bounds(p)));
        Rc::new(Region { key, name: name.chars().take(128).collect(), parts, convex: None, bounds, mode })
    }

    /// Whether the region is convex and holds all of `r`.
    fn contains(&self, r: &Rect) -> bool {
        let Some(poly) = &self.convex else { return false };
        if !(r.min.x <= r.max.x && r.min.y <= r.max.y) {
            return false;
        }
        let n = poly.len();
        let area2: f32 = (0..n).map(|i| poly[i].cross(poly[(i + 1) % n])).sum();
        let s = area2.signum();
        let corners = [r.min, Vec2::new(r.max.x, r.min.y), r.max, Vec2::new(r.min.x, r.max.y)];
        (0..n).all(|i| {
            let (a, b) = (poly[i], poly[(i + 1) % n]);
            let tol = 1e-3 * (b - a).length();
            corners.iter().all(|p| s * (b - a).cross(*p - a) >= -tol)
        })
    }

    /// This region cut to the convex region `other`, exactly. A soft region keeps its parts'
    /// paints: unstroked shapes are cut in their own space; stroked shapes, text and symbols
    /// reaching outside are kept whole (reported).
    fn intersect(&self, other: &Region, report: &mut Report) -> Rc<Region> {
        let Some(poly) = &other.convex else { return Rc::new(Region { key: self.key.clone(), name: self.name.clone(), parts: self.parts.clone(), convex: self.convex.clone(), bounds: self.bounds, mode: self.mode }) };
        if self.mode.is_soft() {
            let mut parts = Vec::with_capacity(self.parts.len());
            for p in &self.parts {
                if other.contains(&element_bounds(p)) {
                    parts.push(p.clone());
                    continue;
                }
                let local = p.transform.inverse().map(|inv| poly.iter().map(|q| inv.apply(*q)).collect::<Vec<_>>());
                match (&p.kind, local) {
                    (ElementKind::Shape(s), Some(lp)) if s.stroke.is_none() => {
                        let cut = clip_convex(&s.path, &lp);
                        if !cut.is_empty() {
                            parts.push(ImportedElement { kind: ElementKind::Shape(Shape { path: Arc::new(cut), ..s.clone() }), ..p.clone() });
                        }
                    }
                    _ => {
                        report.skip("soft mask artwork crossing its mask's edge (kept whole)");
                        parts.push(p.clone());
                    }
                }
            }
            return Region::soft(format!("{}&{}", self.key, other.key), &self.name, parts, self.mode);
        }
        let mut parts = Vec::new();
        for p in &self.parts {
            let (path, rule) = match &p.kind {
                ElementKind::Shape(s) => (s.path.transformed(&p.transform), s.fill_rule),
                ElementKind::Text(t) => (t.outline.transformed(&p.transform), FillRule::NonZero),
                ElementKind::Symbol { .. } | ElementKind::Camera | ElementKind::Group { .. } => continue,
            };
            let cut = clip_convex(&path, poly);
            if !cut.is_empty() {
                parts.push(mask_part(p.name.clone(), cut, Affine::IDENTITY, rule));
            }
        }
        Region::new(format!("{}&{}", self.key, other.key), &self.name, parts)
    }
}

impl Region {
    /// Whether nothing of what this region masks shows.
    fn hides_all(&self) -> bool {
        self.parts.is_empty()
    }
}

/// What an item's clips come to.
enum Effective {
    /// Nothing of it shows.
    Hidden,
    Unclipped,
    Masked(Rc<Region>),
}

/// The id in `url(#id)`.
fn url(v: &str) -> Option<String> {
    match paint_spec(v) {
        Some(PaintSpec::Url(id, _)) => Some(id),
        _ => None,
    }
}

/// A mask region attribute: a fraction or percentage of the bounding box.
fn fraction(v: Option<&str>, default: f32) -> f32 {
    v.map(str::trim)
        .and_then(|v| match v.strip_suffix('%') {
            Some(p) => p.trim().parse::<f32>().ok().map(|p| p / 100.0),
            None => v.parse::<f32>().ok(),
        })
        .filter(|x| x.is_finite())
        .unwrap_or(default)
}

impl<'a> Builder<'a> {
    /// Applies an element's `clip-path` and `mask` to what it drew (`items[start..]`); `m` is its
    /// user space.
    pub(crate) fn clip(&mut self, d: &[(String, String)], m: &Affine, start: usize, depth: usize) {
        let clip = get(d, "clip-path").filter(|v| *v != "none").map(str::to_string);
        let mask = get(d, "mask").filter(|v| *v != "none").map(str::to_string);
        if clip.is_none() && mask.is_none() {
            return;
        }
        let bbox = self.user_bbox(start, m);
        let mut regions: Vec<Rc<Region>> = Vec::new();
        if let Some(v) = clip {
            match url(&v) {
                Some(id) => match self.clip_path(&id, m, bbox) {
                    Some(r) => regions.extend(r),
                    None => self.report.skip("references to missing clip paths (drawn unclipped)"),
                },
                None => self.report.skip("CSS clip shapes (drawn unclipped)"),
            }
        }
        if let Some(v) = mask {
            match url(&v) {
                Some(id) => match self.mask(&id, m, bbox, depth) {
                    Some(r) => regions.extend(r),
                    None => self.report.skip("references to missing masks (drawn unmasked)"),
                },
                None => self.report.skip("CSS mask images (drawn unmasked)"),
            }
        }
        for it in self.items.iter_mut().skip(start) {
            it.clips.extend(regions.iter().cloned());
        }
    }

    /// The bounding box of `items[start..]` in the user space `m` maps from (the box
    /// `objectBoundingBox` units refer to).
    fn user_bbox(&self, start: usize, m: &Affine) -> Option<Rect> {
        let inv = m.inverse()?;
        let mut r = Rect::EMPTY;
        for it in self.items.iter().skip(start) {
            let to_user = inv.mul(&it.el.transform);
            match &it.el.kind {
                ElementKind::Shape(s) => {
                    for line in s.path.flatten(&to_user, 0.05) {
                        for p in line.points {
                            r.include(p);
                        }
                    }
                }
                ElementKind::Text(t) => r = r.union(&Rect::new(0.0, 0.0, t.box_width, t.box_height).transformed(&to_user)),
                ElementKind::Symbol { .. } | ElementKind::Camera | ElementKind::Group { .. } => {}
            }
        }
        (r.min.x <= r.max.x && r.min.y <= r.max.y).then_some(r)
    }

    /// Whether `target` is already being read (a loop), or the chain is too long. Reports it.
    fn looping(&mut self, target: &X) -> bool {
        if self.resolving.iter().any(|r| std::ptr::eq(*r, target)) || self.resolving.len() >= MAX_NESTING {
            self.report.skip("clip paths or masks that refer to themselves (ignored)");
            return true;
        }
        false
    }

    /// The regions of `clipPath` `id` used in user space `m`: its own, and those of the clip
    /// paths it is clipped by. `None` when there is no such clip path.
    fn clip_path(&mut self, id: &str, m: &Affine, bbox: Option<Rect>) -> Option<Vec<Rc<Region>>> {
        let target = self.ids.get(id).copied().filter(|t| local(&t.name) == "clipPath")?;
        if self.looping(target) {
            return Some(Vec::new());
        }
        let units = if target.attr("clipPathUnits") == Some("objectBoundingBox") {
            match bbox {
                Some(b) => Affine::translate(b.min.x, b.min.y).mul(&Affine::scale(b.width(), b.height())),
                // Nothing to size the clip by: it clips everything.
                None => return Some(vec![Region::new(format!("clip:{target:p}:empty"), id, Vec::new())]),
            }
        } else {
            Affine::IDENTITY
        };
        let base = m.mul(&units).mul(&target.attr("transform").map_or(Affine::IDENTITY, transform));
        let style = Style::default().with(target, &mut Report::default());
        self.resolving.push(target);
        let mut parts = Vec::new();
        for c in &target.children {
            self.clip_child(c, &base, &style, &mut parts, 0);
        }
        let mut out = vec![Region::new(format!("clip:{target:p}:{:?}", base.m), id, parts)];
        let inner = get(&declarations(target), "clip-path").and_then(url);
        if let Some(inner) = inner
            && let Some(r) = self.clip_path(&inner, m, bbox)
        {
            out.extend(r);
        }
        self.resolving.pop();
        Some(out)
    }

    /// One child of a `clipPath`: a shape, text, or a `use` of one. Only its geometry counts.
    fn clip_child(&mut self, c: &'a X, base: &Affine, style: &Style, parts: &mut Vec<ImportedElement>, uses: usize) {
        let name = local(&c.name);
        let d = declarations(c);
        if name == anicraft_xml::TEXT_NODE || get(&d, "display") == Some("none") || hidden(&d) {
            return;
        }
        if get(&d, "clip-path").is_some_and(|v| v != "none") {
            self.report.skip("clip paths on the contents of a clip path (ignored)");
        }
        let style = style.with(c, &mut Report::default());
        let t = base.mul(&c.attr("transform").map_or(Affine::IDENTITY, transform));
        match name {
            "use" if uses < crate::MAX_USE_DEPTH => {
                let href = c.attr("href").or_else(|| c.attr("xlink:href")).unwrap_or("").trim_start_matches('#');
                let Some(target) = self.ids.get(href).copied() else { return };
                let (vw, vh) = self.viewport;
                let x = c.attr("x").and_then(|v| length(v, vw)).unwrap_or(0.0);
                let y = c.attr("y").and_then(|v| length(v, vh)).unwrap_or(0.0);
                self.clip_child(target, &t.mul(&Affine::translate(x, y)), &style, parts, uses + 1);
            }
            "text" => parts.extend(self.text(c, &t, &style, 1.0)),
            "path" | "rect" | "circle" | "ellipse" | "polyline" | "polygon" => {
                if let Some((p, true)) = self.geometry(c, name)
                    && !p.is_empty()
                    && t.is_finite()
                {
                    parts.push(mask_part(name_of(c), p, t, style.clip_rule));
                }
            }
            _ => {}
        }
    }

    /// The regions of `mask` `id` used in user space `m`: its content's fills, cut to its
    /// region. `None` when there is no such mask; no regions when the mask is soft (reported).
    fn mask(&mut self, id: &str, m: &Affine, bbox: Option<Rect>, depth: usize) -> Option<Vec<Rc<Region>>> {
        let target = self.ids.get(id).copied().filter(|t| local(&t.name) == "mask")?;
        if self.looping(target) {
            return Some(Vec::new());
        }
        let everything = || Some(vec![Region::new(format!("mask:{target:p}:empty"), id, Vec::new())]);
        // The mask's region: by default the bounding box grown by 10% on every side.
        let rect = if target.attr("maskUnits") == Some("userSpaceOnUse") {
            let (vw, vh) = self.viewport;
            let len = |k: &str, reference: f32, d: f32| target.attr(k).and_then(|v| length(v, reference)).unwrap_or(d * reference);
            let (x, y) = (len("x", vw, -0.1), len("y", vh, -0.1));
            Rect::new(x, y, x + len("width", vw, 1.2), y + len("height", vh, 1.2))
        } else {
            let Some(b) = bbox else { return everything() };
            let (x, y) = (fraction(target.attr("x"), -0.1), fraction(target.attr("y"), -0.1));
            let (w, h) = (fraction(target.attr("width"), 1.2), fraction(target.attr("height"), 1.2));
            let x0 = b.min.x + x * b.width();
            let y0 = b.min.y + y * b.height();
            Rect::new(x0, y0, x0 + w * b.width(), y0 + h * b.height())
        };
        if rect.is_empty() {
            return everything();
        }
        let content_m = if target.attr("maskContentUnits") == Some("objectBoundingBox") {
            let Some(b) = bbox else { return everything() };
            m.mul(&Affine::translate(b.min.x, b.min.y)).mul(&Affine::scale(b.width(), b.height()))
        } else {
            *m
        };
        let alpha_mask = get(&declarations(target), "mask-type") == Some("alpha");
        let style = Style::default().with(target, &mut Report::default());
        self.resolving.push(target);
        let mark = self.items.len();
        self.children(target, &content_m, &style, 1.0, depth + 1, 0);
        self.resolving.pop();
        let content: Vec<Item> = self.items.drain(mark..).collect();
        // An opaque mask is the coverage of its shapes (an Animate mask): full alpha, no strokes,
        // and for a luminance mask, white. Any other is a soft mask (ADR 0070), its content as
        // drawn.
        let opaque = |c: Rgba| c.a >= 0.999 && (alpha_mask || (c.r >= 0.996 && c.g >= 0.996 && c.b >= 0.996));
        let is_opaque = |it: &Item| {
            let e = &it.el;
            it.clips.is_empty()
                && e.alpha >= 0.999
                && match &e.kind {
                    ElementKind::Shape(s) => s.stroke.is_none() && matches!(s.fill, Some(Paint::Solid(c)) if opaque(c)),
                    ElementKind::Text(t) => opaque(t.color),
                    ElementKind::Symbol { .. } | ElementKind::Camera | ElementKind::Group { .. } => false,
                }
        };
        let area = mask_part(String::new(), Path::rect(rect.min.x, rect.min.y, rect.width(), rect.height()), *m, FillRule::NonZero);
        let area = Region::new(format!("maskarea:{target:p}:{:?}:{rect:?}", m.m), id, vec![area]);
        let key = format!("mask:{target:p}:{:?}", content_m.m);
        if !content.iter().all(is_opaque) {
            if content.iter().any(|it| !it.clips.is_empty()) {
                self.report.skip("clip paths and masks inside a mask (ignored)");
            }
            let mode = if alpha_mask { MaskMode::Alpha } else { MaskMode::Luminance };
            let parts = content.into_iter().map(|it| it.el).collect();
            return Some(vec![Region::soft(key, id, parts, mode), area]);
        }
        let parts = content
            .into_iter()
            .map(|it| match &it.el.kind {
                ElementKind::Shape(s) => mask_part(it.el.name.clone(), (*s.path).clone(), it.el.transform, s.fill_rule),
                _ => it.el,
            })
            .collect();
        Some(vec![Region::new(key, id, parts), area])
    }

    /// What an item's clips come to (combinations are cached by key; `None` = hidden).
    fn effective(&mut self, it: &Item, cache: &mut HashMap<String, Option<Rc<Region>>>) -> Effective {
        let b = element_bounds(&it.el);
        let mut keep: Vec<Rc<Region>> = Vec::new();
        for r in &it.clips {
            if r.hides_all() || r.bounds.intersect(&b).is_empty() {
                return Effective::Hidden;
            }
            if r.contains(&b) || keep.iter().any(|k| k.key == r.key) {
                continue;
            }
            keep.push(r.clone());
        }
        // A convex region holding another region whole cuts nothing more.
        let mut i = 0;
        while keep.len() > 1 && i < keep.len() {
            let holds = keep.get(i).is_some_and(|r| keep.iter().enumerate().any(|(j, o)| j != i && r.contains(&o.bounds)));
            if holds {
                keep.remove(i);
            } else {
                i += 1;
            }
        }
        if keep.len() <= 1 {
            return keep.pop().map_or(Effective::Unclipped, Effective::Masked);
        }
        let key = keep.iter().map(|r| r.key.as_str()).collect::<Vec<_>>().join("&");
        let combined = match cache.get(&key) {
            Some(c) => c.clone(),
            None => {
                let (curved, convex): (Vec<Rc<Region>>, Vec<Rc<Region>>) = keep.into_iter().partition(|r| r.convex.is_none());
                if curved.len() > 1 {
                    self.report.skip("curved clips inside curved clips (drawn with the inner one)");
                }
                // A soft mask is kept over a curved clip (it is what the artwork fades by).
                let mut curved = curved;
                if let Some(i) = curved.iter().position(|r| r.mode.is_soft()) {
                    curved.swap(0, i);
                }
                let mut rest = convex.into_iter();
                let base = match curved.first() {
                    Some(c) => Some(c.clone()),
                    None => rest.next(),
                };
                let report = &mut self.report;
                let r = base.map(|base| rest.fold(base, |acc, c| acc.intersect(&c, report))).filter(|r| !r.parts.is_empty());
                cache.insert(key, r.clone());
                r
            }
        };
        match combined {
            Some(r) if !r.bounds.intersect(&b).is_empty() => Effective::Masked(r),
            _ => Effective::Hidden,
        }
    }

    /// The drawing as layers, bottom first: runs of items with the same mask.
    pub(crate) fn layers(&mut self) -> Vec<ImportedLayer> {
        let items = std::mem::take(&mut self.items);
        let mut cache = HashMap::new();
        let mut runs: Vec<(Option<Rc<Region>>, Vec<ImportedElement>)> = Vec::new();
        let mut masks = 0usize;
        for it in items {
            let mut region = match self.effective(&it, &mut cache) {
                Effective::Hidden => continue,
                Effective::Unclipped => None,
                Effective::Masked(r) => Some(r),
            };
            let same = |run: &(Option<Rc<Region>>, Vec<ImportedElement>), r: &Option<Rc<Region>>| match (&run.0, r) {
                (None, None) => true,
                (Some(a), Some(b)) => a.key == b.key,
                _ => false,
            };
            if !runs.last().is_some_and(|run| same(run, &region)) && region.is_some() {
                if masks >= MAX_MASKS {
                    self.report.skip("clip paths and masks past 250 per drawing (drawn unclipped)");
                    region = None;
                } else {
                    masks += 1;
                }
            }
            match runs.last_mut() {
                Some(run) if same(run, &region) => run.1.push(it.el),
                _ => runs.push((region, vec![it.el])),
            }
        }
        runs.into_iter().map(|(r, elements)| ImportedLayer { mask: r.map(|r| ImportedMask { name: r.name.clone(), elements: r.parts.clone(), mode: r.mode }), elements }).collect()
    }
}
