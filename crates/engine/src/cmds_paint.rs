//! Paint Bucket and Ink Bottle.
//!
//! AniCraft shapes are independent objects (like object drawing), so a fill for an area
//! enclosed by several shapes has to be found, not looked up:
//!
//! 1. Click on a filled shape: it is recoloured.
//! 2. Otherwise the active layer at the playhead is rasterized into a "wall" mask with the
//!    renderer's own coverage rasterizer (fills and stroke outlines), walls are thickened by the
//!    gap size so small openings close, and the area is flood-filled from the click.
//! 3. An area that reaches the edge of the artwork is not enclosed: nothing is added and the
//!    message suggests a larger gap.
//! 4. The region is grown back under the walls (so no hairline shows between fill and line),
//!    traced into closed contours (`geom::trace`), fitted with curves (`geom::fit_curve`) and
//!    added as a fill-only shape at the bottom of the layer, below the lines that bound it.
//!
//! The grid is at most `MAX_GRID` cells on its long side (cell size grows for big artwork), so
//! the cost is bounded whatever the document.

use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::{ColorEffect, DisplayItem, Element, ElementId, ElementKind, Paint, Shape, Stroke};
use anicraft_geom::trace::contours;
use anicraft_geom::{Affine, FillRule, Path, Rect, Vec2, fit_curve};
use anicraft_render::raster::{Coverage, IRect};
use anicraft_render::stroke::stroke_polygons;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c("paint.bucket", "Paint Bucket", "", None, "{x, y, color?, gap? (stage units: how wide an opening still counts as closed)}", true, bucket as crate::RunFn),
        c("paint.ink", "Ink Bottle", "", None, "{x, y, color?, width?, profile?}", true, ink),
    ]
}

/// Longest side of the flood-fill grid, in cells.
const MAX_GRID: f32 = 1400.0;

/// Items of the active layer at the playhead, back to front.
fn layer_items(s: &Session) -> Vec<DisplayItem> {
    s.display_list().items.iter().filter(|i| i.layer == s.layer && i.outline.is_none()).cloned().collect()
}

/// Topmost item of the active layer under `p`: `(element, inside its fill)`.
fn pick(items: &[DisplayItem], p: Vec2, slop: f32) -> Option<(ElementId, bool)> {
    items.iter().rev().find_map(|it| {
        let hw = it.stroke.as_ref().map_or(0.0, |s| s.reach() * it.transform.mean_scale());
        if !it.path.bounds().transformed(&it.transform).inflate(hw + slop).contains(p) {
            return None;
        }
        let inside = it.fill.is_some() && it.path.contains(p, &it.transform, it.fill_rule);
        (inside || it.path.distance_to_edge(p, &it.transform) <= hw + slop).then_some((it.element, inside))
    })
}

fn element_shape_mut(s: &mut Session, id: ElementId) -> Result<&mut Shape> {
    let kf = s.editable_keyframe()?;
    let el = kf.elements_mut_where(|e| e == id).into_iter().next().ok_or_else(|| EngineError::State("that shape is not on the active layer at this frame".into()))?;
    match &mut el.kind {
        ElementKind::Shape(sh) => Ok(sh),
        ElementKind::Symbol { .. } => Err(EngineError::State("symbol instances keep their own colours; break it apart (Ctrl+B) or edit the symbol to paint it".into())),
        ElementKind::Text(_) => Err(EngineError::State("text has its own colour (Properties); break it apart (Ctrl+B) to paint it as a shape".into())),
        ElementKind::Camera => Err(EngineError::State("the camera has no paint (its tint is in Properties)".into())),
    }
}

fn bucket(s: &mut Session, p: &Value) -> Result<Value> {
    let at = Vec2::new(f32_req(p, "x")?, f32_req(p, "y")?);
    let color = opt_color(p, "color")?.unwrap_or(s.style.fill);
    let gap = opt_f32(p, "gap")?.unwrap_or(0.0);
    if !(0.0..=1000.0).contains(&gap) {
        return Err(EngineError::Param("param 'gap' must be between 0 and 1000".into()));
    }
    let items = layer_items(s);
    // 1. On a fill: recolour it.
    if let Some((id, true)) = pick(&items, at, 0.0) {
        element_shape_mut(s, id)?.fill = Some(Paint::Solid(color));
        return Ok(json!({"recolored": id.0}));
    }
    // 2. Find the enclosed area.
    let Some(path) = enclosed_region(&items, at, gap)? else {
        return Err(EngineError::State("this area isn't closed. Close the outline, or choose a larger gap size".into()));
    };
    let c = path.bounds().center();
    let local = path.transformed(&Affine::translate(-c.x, -c.y));
    let id = ElementId(s.doc.alloc_id());
    let el = Element {
        id,
        name: String::new(),
        transform: Affine::translate(c.x, c.y),
        color: ColorEffect::default(),
        kind: ElementKind::Shape(Shape { path: Arc::new(local), fill: Some(Paint::Solid(color)), stroke: None, fill_rule: FillRule::EvenOdd, merge: false }),
    };
    let kf = s.editable_keyframe()?;
    if kf.elements.len() >= 100_000 {
        return Err(EngineError::State("too many elements on this keyframe".into()));
    }
    // Under everything on the layer, so the lines that bound it stay on top.
    kf.elements.insert(0, Arc::new(el));
    Ok(json!({"element": id.0}))
}

/// Grid over stage space: origin and cell size.
struct Grid {
    origin: Vec2,
    cell: f32,
}

impl Grid {
    fn to_px(&self) -> Affine {
        Affine::scale(1.0 / self.cell, 1.0 / self.cell).mul(&Affine::translate(-self.origin.x, -self.origin.y))
    }
}

/// The area around `at` enclosed by the layer's artwork, as a stage-space path; `None` when it
/// is not enclosed (or there is no artwork around it).
fn enclosed_region(items: &[DisplayItem], at: Vec2, gap: f32) -> Result<Option<Path>> {
    let mut bounds = Rect::EMPTY;
    for it in items {
        let pad = it.stroke.as_ref().map_or(0.0, |s| s.reach() * it.transform.mean_scale());
        bounds = bounds.union(&it.path.bounds().inflate(pad).transformed(&it.transform));
    }
    if bounds.is_empty() || !bounds.contains(at) || !(bounds.min.is_finite() && bounds.max.is_finite()) {
        return Ok(None);
    }
    let extent = bounds.width().max(bounds.height());
    let cell = (extent / MAX_GRID).max(0.5);
    // One free cell of margin all round: anything that reaches it is outside.
    let origin = bounds.min - Vec2::new(cell, cell);
    let w = (bounds.width() / cell).ceil() as usize + 3;
    let h = (bounds.height() / cell).ceil() as usize + 3;
    let g = Grid { origin, cell };
    let m = g.to_px();
    let mut wall = vec![0.0f32; w * h];
    let mut row = Vec::new();
    for it in items {
        let to_px = m.mul(&it.transform);
        let mut polys: Vec<(Vec<Vec<Vec2>>, FillRule)> = Vec::new();
        let lines = it.path.flatten(&to_px, 0.2);
        if it.fill.is_some() {
            polys.push((lines.iter().map(|l| l.points.clone()).collect(), it.fill_rule));
        }
        if let Some(st) = &it.stroke {
            let hw = (st.width * 0.5 * it.transform.mean_scale() / cell).max(0.5);
            let walls = match &st.profile {
                // A variable-width line walls off what it covers.
                Some(pr) => anicraft_render::stroke::variable_polygons(&lines, hw, pr, to_px.inverse().as_ref(), to_px.determinant() < 0.0),
                None => stroke_polygons(&lines, hw),
            };
            polys.push((walls, FillRule::NonZero));
        }
        for (pp, rule) in polys {
            let mut bb = Rect::EMPTY;
            for q in pp.iter().flatten() {
                bb.include(*q);
            }
            if bb.is_empty() {
                continue;
            }
            let ir = IRect { x0: (bb.min.x.floor() as i32).max(0), y0: (bb.min.y.floor() as i32).max(0), x1: (bb.max.x.ceil() as i32 + 1).min(w as i32), y1: (bb.max.y.ceil() as i32 + 1).min(h as i32) };
            if ir.is_empty() {
                continue;
            }
            let mut cov = Coverage::new(ir);
            for poly in &pp {
                cov.add_polygon(poly);
            }
            row.resize(ir.width(), 0.0);
            for y in 0..ir.height() {
                cov.resolve_row(y, rule, &mut row);
                let base = (ir.y0 as usize + y) * w + ir.x0 as usize;
                for (x, v) in row.iter().enumerate() {
                    if let Some(cellv) = wall.get_mut(base + x) {
                        *cellv = cellv.max(*v);
                    }
                }
            }
        }
    }
    // Any coverage is a wall, so hairlines cannot leak; then close gaps.
    let mut blocked: Vec<bool> = wall.iter().map(|v| *v > 0.02).collect();
    let gap_cells = gap / cell * 0.5;
    if gap_cells > 0.0 {
        let d = distance(&blocked, w, h);
        for (b, dv) in blocked.iter_mut().zip(&d) {
            *b = *dv <= gap_cells;
        }
    }
    // Flood fill from the click.
    let cx = ((at.x - origin.x) / cell) as usize;
    let cy = ((at.y - origin.y) / cell) as usize;
    let start = cy * w + cx;
    if blocked.get(start).copied().unwrap_or(true) {
        return Ok(None);
    }
    let mut region = vec![false; w * h];
    let mut q = VecDeque::from([start]);
    if let Some(r) = region.get_mut(start) {
        *r = true;
    }
    while let Some(i) = q.pop_front() {
        let (x, y) = (i % w, i / w);
        if x == 0 || y == 0 || x + 1 >= w || y + 1 >= h {
            // Reached the margin: open to the outside.
            return Ok(None);
        }
        for n in [i - 1, i + 1, i - w, i + w] {
            if !blocked.get(n).copied().unwrap_or(true) && !region.get(n).copied().unwrap_or(true) {
                if let Some(r) = region.get_mut(n) {
                    *r = true;
                }
                q.push_back(n);
            }
        }
    }
    // Grow back: free cells up to the closed gap (the wall thickening took them), and cells
    // under a line a little further, so the fill tucks under its outline. Never into open space
    // beyond a line.
    let d = distance(&region, w, h);
    let mask: Vec<f32> = d
        .iter()
        .zip(&wall)
        .map(|(dv, wv)| {
            let under_line = *wv > 0.02;
            let keep = *dv == 0.0 || (!under_line && *dv <= gap_cells) || (under_line && *dv <= gap_cells + 1.5);
            if keep { 1.0 } else { 0.0 }
        })
        .collect();
    let loops = contours(&mask, w, h);
    if loops.is_empty() {
        return Ok(None);
    }
    let mut path = Path::new();
    for lp in loops {
        let pts: Vec<Vec2> = lp.iter().map(|q| origin + *q * cell).collect();
        let part = fit_curve(&pts, (cell * 0.6).max(0.25), cell * 3.0, true);
        path.cmds.extend(part.cmds);
    }
    Ok((!path.is_empty()).then_some(path))
}

/// Distance in cells from each cell to the nearest `true` cell (two-pass 3-4 chamfer, ≈ 2 %
/// from Euclidean).
fn distance(src: &[bool], w: usize, h: usize) -> Vec<f32> {
    const FAR: f32 = 1e9;
    let mut d: Vec<f32> = src.iter().map(|b| if *b { 0.0 } else { FAR }).collect();
    let (a, b) = (1.0f32, 4.0f32 / 3.0);
    let at = |d: &Vec<f32>, x: isize, y: isize| -> f32 {
        if x < 0 || y < 0 || x as usize >= w || y as usize >= h { FAR } else { d.get(y as usize * w + x as usize).copied().unwrap_or(FAR) }
    };
    for y in 0..h as isize {
        for x in 0..w as isize {
            let i = y as usize * w + x as usize;
            let v = d.get(i).copied().unwrap_or(FAR).min(at(&d, x - 1, y) + a).min(at(&d, x, y - 1) + a).min(at(&d, x - 1, y - 1) + b).min(at(&d, x + 1, y - 1) + b);
            if let Some(c) = d.get_mut(i) {
                *c = v;
            }
        }
    }
    for y in (0..h as isize).rev() {
        for x in (0..w as isize).rev() {
            let i = y as usize * w + x as usize;
            let v = d.get(i).copied().unwrap_or(FAR).min(at(&d, x + 1, y) + a).min(at(&d, x, y + 1) + a).min(at(&d, x + 1, y + 1) + b).min(at(&d, x - 1, y + 1) + b);
            if let Some(c) = d.get_mut(i) {
                *c = v;
            }
        }
    }
    d
}

fn ink(s: &mut Session, p: &Value) -> Result<Value> {
    let at = Vec2::new(f32_req(p, "x")?, f32_req(p, "y")?);
    let color = opt_color(p, "color")?.unwrap_or(s.style.stroke);
    let width = opt_f32(p, "width")?.unwrap_or(s.style.stroke_width.max(1.0));
    if !(0.1..=1000.0).contains(&width) {
        return Err(EngineError::Param("param 'width' must be between 0.1 and 1000".into()));
    }
    let slop = opt_f32(p, "slop")?.unwrap_or(3.0).clamp(0.0, 100.0);
    let items = layer_items(s);
    let (id, _) = pick(&items, at, slop).ok_or_else(|| EngineError::State("click a shape to give it this stroke".into()))?;
    let profile = match opt_profile(p, "profile")? {
        Some(pr) => pr,
        None => s.style.stroke_profile.profile(),
    };
    element_shape_mut(s, id)?.stroke = Some(Stroke::new(color, width).with_profile(profile));
    Ok(json!({"element": id.0}))
}
