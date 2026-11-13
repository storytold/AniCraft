//! Paint Bucket, Ink Bottle and Eyedropper; the fill swatch's gradients and bitmaps, Lock Fill
//! and the Gradient Transform tool (ADR 0074).
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
use anicraft_doc::{BitmapId, ColorEffect, DisplayItem, Element, ElementId, ElementKind, GradientStop, MAX_FOCAL, MAX_GRADIENT_STOPS, Paint, Rgba, Shape, Spread, Stroke};
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
        c(
            "paint.bucket",
            "Paint Bucket",
            "",
            None,
            "{x, y, color?, gap? (stage units: how wide an opening still counts as closed), object?: bool} fills the closed area under the point, or recolours the fill there; unless `object` (default: the Object Drawing setting), the fill is raw and merges with the drawing (an area closed by raw lines and fills is filled exactly up to them)",
            true,
            bucket as crate::RunFn,
        ),
        c("paint.ink", "Ink Bottle", "", None, "{x, y, color?, width?, profile?, cap?, join?, miter?, style?, dash?, dash_offset?, dot_spacing?} (the tool's line style unless given; see stroke.set_style)", true, ink),
        c("paint.pick", "Eyedropper", "", None, "{x, y, other?: bool (into the other swatch only: a fill's colour into the stroke, a line's into the fill), slop?} picks the style under the point on any visible layer into the tool settings: a fill's paint (solid, gradient or bitmap, kept where it is on the stage) into the fill swatch, a line's colour, width and profile into the stroke settings", false, pick_style),
        c("paint.apply_style", "Apply Style", "", None, "{fill?: bool, stroke?: bool} (default both) gives the selected shapes the fill swatch's paint (shapes with a fill) and the stroke settings (shapes with a line, and drawing objects without one)", true, |s, p| s.each_layer(false, |s| apply_style(s, p))),
        c(
            "paint.set_fill",
            "Fill",
            "",
            None,
            "{type?: solid|linear|radial|bitmap (default: the swatch's), color? (solid), stops?: [{offset: 0..1, color: \"#rrggbb\" | \"#rrggbbaa\"}, ..] (2 to 15), spread?: extend|reflect|repeat, linear_rgb?: bool, focal?: -0.98..0.98 (radial), bitmap?: id, selection?: bool (default true)} sets the fill swatch (what new shapes, the Paint Bucket and the Brush pour) and, unless `selection` is false, the fills of the selected shapes. A shape keeping its kind of fill keeps where its gradient or bitmap lies; one changing kind gets it fitted to its bounds. A new gradient starts white to black → {fill: {type, ..}, changed}",
            true,
            set_fill,
        ),
        c(
            "paint.transform_fill",
            "Gradient Transform",
            "",
            None,
            "{element?: id (default: the selected shapes with a gradient or bitmap fill), frame?: [a, b, c, d, e, f] (the fill's frame in the space being edited: the box -1..1 onto a linear gradient's span, a radial gradient's ring or one copy of a bitmap), or changes made in this order: scale_x?, scale_y?, scale? (along the frame's own axes), skew_x?, skew_y? (degrees, along its axes), rotate? (degrees, about its centre), move?: [dx, dy], center?: [x, y]; focal?: -0.98..0.98 (radial)} moves, turns, scales or skews where a gradient or bitmap fill lies, as the Gradient Transform tool (F) does → {changed, frames: [[a, b, c, d, e, f], ..]}",
            true,
            |s, p| s.each_layer(false, |s| transform_fill(s, p)),
        ),
        c("paint.lock_fill", "Lock Fill", "", None, "{on?: bool (default: toggle)} Lock Fill: a gradient or bitmap from the fill swatch is poured where it lies on the stage, running on across shapes; off, each shape gets it fitted to its bounds → {on}", false, lock_fill),
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
        ElementKind::Group { .. } => Err(EngineError::State("a group keeps its members' colours; double-click it to paint inside, or ungroup it (Ctrl+Shift+G)".into())),
    }
}

fn bucket(s: &mut Session, p: &Value) -> Result<Value> {
    let at = Vec2::new(f32_req(p, "x")?, f32_req(p, "y")?);
    // The fill swatch: a gradient or bitmap stays where it is on the stage with Lock Fill, else
    // it is fitted to the shape it fills (ADR 0074).
    let given = opt_color(p, "color")?.map(Paint::Solid);
    let gap = opt_f32(p, "gap")?.unwrap_or(0.0);
    if !(0.0..=1000.0).contains(&gap) {
        return Err(EngineError::Param("param 'gap' must be between 0 and 1000".into()));
    }
    let object = crate::cmds_draw::object_param(s, p)?;
    let items = layer_items(s);
    // 1. On a fill: recolour it. A raw fill then joins the fills of its new colour it touches.
    if let Some((id, true)) = pick(&items, at, 0.0) {
        let to_local = items.iter().find(|i| i.element == id).and_then(|i| i.transform.inverse()).unwrap_or(Affine::IDENTITY);
        let style = s.style.clone();
        let sh = element_shape_mut(s, id)?;
        sh.fill = Some(match &given {
            Some(c) => c.clone(),
            None => style.fill_paint_for(&to_local, &sh.path.bounds()),
        });
        if sh.merge {
            crate::cmds_raw::remerge_in_place(s, &[id])?;
            return Ok(json!({"recolored": id.0, "merged": true}));
        }
        return Ok(json!({"recolored": id.0}));
    }
    // 2. Find the enclosed area: exactly, when raw lines and fills close it off (ADR 0042),
    //    else from the rasterized layer.
    let exact = if object { None } else { crate::merge::enclosed_area(&s.visible_elements(), at) };
    let was_exact = exact.is_some();
    let path = match exact {
        Some(p) => p,
        None => enclosed_region(&items, at, gap)?.ok_or_else(|| EngineError::State("this area isn't closed. Close the outline, or choose a larger gap size".into()))?,
    };
    let c = path.bounds().center();
    let local = path.transformed(&Affine::translate(-c.x, -c.y));
    let id = ElementId(s.doc.alloc_id());
    let el = Element {
        id,
        name: String::new(),
        transform: Affine::translate(c.x, c.y),
        color: ColorEffect::default(),
        kind: ElementKind::Shape(Shape { fill: Some(given.unwrap_or_else(|| s.style.fill_paint_for(&Affine::translate(-c.x, -c.y), &local.bounds()))), path: Arc::new(local), stroke: None, fill_rule: FillRule::EvenOdd, merge: !object, warp: None }),
    };
    if !object {
        // Raw: behind the drawing, so it takes only what is empty and joins the fills of its
        // colour it meets; the lines that bound it stay on top.
        let ids = crate::merge::merge_in_order(s, vec![el], crate::merge::Order::Behind)?;
        s.selection.clear();
        return Ok(json!({"element": ids.first().map(|e| e.0), "elements": ids.iter().map(|e| e.0).collect::<Vec<_>>(), "merged": true, "exact": was_exact}));
    }
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
        None => s.style.profile(),
    };
    // The Ink Bottle carries the whole line: caps, joins and dashes too.
    let line = opt_stroke_style(p, &s.style.line, width)?.unwrap_or_else(|| s.style.line.clone());
    element_shape_mut(s, id)?.stroke = Some(Stroke::new(color, width).with_profile(profile).with_style(line));
    Ok(json!({"element": id.0}))
}

/// The kind of a paint, as commands report it.
fn paint_kind(p: &Paint) -> &'static str {
    match p {
        Paint::Solid(_) => "solid",
        Paint::Linear { .. } => "linear",
        Paint::Radial { .. } => "radial",
        Paint::Bitmap { .. } => "bitmap",
    }
}

/// What the Eyedropper found: a fill's paint (stage space), or a line's stroke and the scale it
/// is drawn at.
enum Picked {
    Fill(Paint),
    Line(Stroke, f32),
}

/// `paint.pick`: the Eyedropper. The topmost thing drawn under the point on any visible layer
/// (locked ones too: picking reads, it doesn't change them): on a line, its stroke; in a fill,
/// its paint.
fn pick_style(s: &mut Session, p: &Value) -> Result<Value> {
    let at = Vec2::new(f32_req(p, "x")?, f32_req(p, "y")?);
    let other = opt_bool(p, "other")?.unwrap_or(false);
    let slop = opt_f32(p, "slop")?.unwrap_or(2.0).clamp(0.0, 100.0);
    let dl = s.display_list();
    let hit = dl.items.iter().rev().find_map(|it| {
        // Mask artwork isn't drawn, and masked-away parts can't be picked.
        if it.masks.is_some() || it.clip.is_some_and(|c| !dl.clip_contains(c, at)) {
            return None;
        }
        let hw = it.stroke.as_ref().map_or(0.0, |st| st.reach() * it.transform.mean_scale());
        if !it.path.bounds().transformed(&it.transform).inflate(hw + slop).contains(at) {
            return None;
        }
        if let Some(st) = &it.stroke
            && it.path.distance_to_edge(at, &it.transform) <= hw + slop
        {
            return Some(Picked::Line(st.clone(), it.transform.mean_scale()));
        }
        match &it.fill {
            Some(f) if it.path.contains(at, &it.transform, it.fill_rule) => Some(Picked::Fill(f.transformed(&it.transform))),
            _ => None,
        }
    });
    let st = &mut s.style;
    match hit {
        Some(Picked::Fill(paint)) => {
            let color = paint.primary_color();
            if other {
                st.stroke = color;
                st.use_stroke = true;
                return Ok(json!({"picked": "fill", "into": "stroke", "paint": paint_kind(&paint), "color": color.to_hex()}));
            }
            st.fill = color;
            st.fill_paint = (!matches!(paint, Paint::Solid(_))).then(|| paint.clone());
            // As in Animate, a picked gradient or bitmap is locked where it was on the stage.
            if st.fill_paint.is_some() {
                st.lock_fill = true;
            }
            st.use_fill = true;
            Ok(json!({"picked": "fill", "into": "fill", "paint": paint_kind(&paint), "color": color.to_hex()}))
        }
        Some(Picked::Line(stroke, scale)) => {
            if other {
                st.set_fill_color(stroke.color);
                st.use_fill = true;
                return Ok(json!({"picked": "line", "into": "fill", "paint": "solid", "color": stroke.color.to_hex()}));
            }
            st.stroke = stroke.color;
            st.stroke_width = (stroke.width * scale).clamp(0.1, 1000.0);
            st.use_stroke = true;
            // Caps, joins and dashes as drawn (dashes scale with the shape, like the width).
            st.line = stroke.style.scaled(scale).sanitized();
            let preset = anicraft_doc::ProfilePreset::ALL.into_iter().find(|pr| pr.profile() == stroke.profile);
            let profile = match preset {
                Some(pr) => {
                    st.set_profile(pr);
                    pr.key()
                }
                None => {
                    st.custom_profile = stroke.profile.clone();
                    "custom"
                }
            };
            Ok(json!({"picked": "line", "into": "stroke", "color": stroke.color.to_hex(), "width": st.stroke_width, "profile": profile, "cap": st.line.cap.key(), "join": st.line.join.key(), "style": st.line.dash_kind().key()}))
        }
        None => Err(EngineError::State("click a fill or a line to pick its colour".into())),
    }
}

/// `paint.apply_style`: what the Eyedropper does with a selection (Animate applies the picked
/// style to it).
fn apply_style(s: &mut Session, p: &Value) -> Result<Value> {
    let fill = opt_bool(p, "fill")?.unwrap_or(true);
    let stroke = opt_bool(p, "stroke")?.unwrap_or(true);
    if !fill && !stroke {
        return Err(EngineError::Param("apply the fill, the stroke or both".into()));
    }
    let style = s.style.clone();
    let line = style.new_stroke(style.stroke, style.stroke_width);
    let mut changed = 0;
    for e in s.selected_elements_mut()? {
        let to_local = e.transform.inverse();
        let ElementKind::Shape(sh) = &mut e.kind else { continue };
        let mut hit = false;
        if fill
            && sh.fill.is_some()
            && let Some(m) = to_local
        {
            sh.fill = Some(style.fill_paint_for(&m, &sh.path.bounds()));
            hit = true;
        }
        // Raw fills stay fill-only: the lines round them are shapes of their own (ADR 0031).
        if stroke && (sh.stroke.is_some() || !sh.merge) {
            sh.stroke = Some(line.clone());
            hit = true;
        }
        changed += usize::from(hit);
    }
    if changed == 0 {
        return Err(EngineError::State("none of the selected items is a shape that takes this style".into()));
    }
    Ok(json!({"changed": changed}))
}

// ----- the fill swatch, Lock Fill and the Gradient Transform tool (ADR 0074) ---------------

/// Animate's default gradient: white to black.
fn default_stops() -> Vec<GradientStop> {
    vec![GradientStop { offset: 0.0, color: Rgba::WHITE }, GradientStop { offset: 1.0, color: Rgba::BLACK }]
}

/// The `stops` param: 2 to 15 stops, each `{offset, color}`, put in order of offset.
pub(crate) fn opt_stops(p: &Value) -> Result<Option<Vec<GradientStop>>> {
    let bad = || EngineError::Param(format!("param 'stops' must be a list of 2 to {MAX_GRADIENT_STOPS} stops like {{\"offset\": 0.5, \"color\": \"#ff8800\"}}"));
    let list = match p.get("stops") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Array(a)) if (2..=MAX_GRADIENT_STOPS).contains(&a.len()) => a,
        Some(_) => return Err(bad()),
    };
    let mut stops = Vec::with_capacity(list.len());
    for v in list {
        let offset = opt_f32(v, "offset").map_err(|_| bad())?.ok_or_else(bad)?;
        if !(0.0..=1.0).contains(&offset) {
            return Err(EngineError::Param("a stop's 'offset' must be between 0 and 1".into()));
        }
        let color = opt_color(v, "color").map_err(|_| bad())?.ok_or_else(bad)?;
        stops.push(GradientStop { offset, color });
    }
    stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    Ok(Some(stops))
}

/// The kinds of fill `paint.set_fill` makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FillKind {
    Solid,
    Linear,
    Radial,
    Bitmap,
}

fn fill_kind(p: &Paint) -> FillKind {
    match p {
        Paint::Solid(_) => FillKind::Solid,
        Paint::Linear { .. } => FillKind::Linear,
        Paint::Radial { .. } => FillKind::Radial,
        Paint::Bitmap { .. } => FillKind::Bitmap,
    }
}

/// What `paint.set_fill` asks for: a kind of fill and the parts given.
struct FillSpec {
    kind: FillKind,
    color: Option<Rgba>,
    stops: Option<Vec<GradientStop>>,
    spread: Option<Spread>,
    linear_rgb: Option<bool>,
    focal: Option<f32>,
    bitmap: Option<BitmapId>,
}

impl FillSpec {
    /// `base` with these changes. A fill of the same kind keeps its geometry; another kind
    /// comes out on the unit box (to be placed); a gradient keeps the colours and options of
    /// the gradient it replaces, else starts white to black. `None`: a bitmap fill with no
    /// bitmap given or held.
    fn apply(&self, base: &Paint) -> Option<Paint> {
        let mut p = match self.kind {
            FillKind::Solid => return Some(Paint::Solid(self.color.unwrap_or_else(|| base.primary_color()))),
            k if k == fill_kind(base) => base.clone(),
            FillKind::Linear | FillKind::Radial => {
                let stops = base.stops().map_or_else(default_stops, <[GradientStop]>::to_vec);
                let mut q = if self.kind == FillKind::Linear { Paint::linear(Vec2::new(-1.0, 0.0), Vec2::new(1.0, 0.0), stops) } else { Paint::radial(Vec2::ZERO, 1.0, stops) };
                if let (Paint::Linear { spread: a, linear_rgb: b, .. } | Paint::Radial { spread: a, linear_rgb: b, .. }, Paint::Linear { spread, linear_rgb, .. } | Paint::Radial { spread, linear_rgb, .. }) = (&mut q, base) {
                    *a = *spread;
                    *b = *linear_rgb;
                }
                q
            }
            FillKind::Bitmap => Paint::Bitmap { bitmap: self.bitmap?, repeat: true, matrix: Affine::IDENTITY },
        };
        match &mut p {
            Paint::Linear { stops, spread, linear_rgb, .. } => self.set_common(stops, spread, linear_rgb),
            Paint::Radial { stops, spread, linear_rgb, focal, .. } => {
                self.set_common(stops, spread, linear_rgb);
                if let Some(f) = self.focal {
                    *focal = f;
                }
            }
            Paint::Bitmap { bitmap, .. } => {
                if let Some(b) = self.bitmap {
                    *bitmap = b;
                }
            }
            Paint::Solid(_) => {}
        }
        Some(p)
    }

    fn set_common(&self, stops: &mut Vec<GradientStop>, spread: &mut Spread, linear_rgb: &mut bool) {
        if let Some(st) = &self.stops {
            stops.clone_from(st);
        }
        if let Some(sp) = self.spread {
            *spread = sp;
        }
        if let Some(l) = self.linear_rgb {
            *linear_rgb = l;
        }
    }
}

/// `#rrggbb`, or `#rrggbbaa` when not opaque.
fn hex_alpha(c: Rgba) -> String {
    let c = c.sanitized();
    if c.a >= 1.0 { c.to_hex() } else { format!("{}{:02x}", c.to_hex(), (c.a * 255.0).round() as u8) }
}

/// A fill as `paint.set_fill` reports it (in its own params' terms).
pub fn fill_json(p: &Paint) -> Value {
    let stops = |st: &[GradientStop]| -> Vec<Value> { st.iter().map(|s| json!({"offset": s.offset, "color": hex_alpha(s.color)})).collect() };
    match p {
        Paint::Solid(c) => json!({"type": "solid", "color": hex_alpha(*c)}),
        Paint::Linear { stops: st, spread, linear_rgb, .. } => json!({"type": "linear", "stops": stops(st), "spread": spread.key(), "linear_rgb": linear_rgb}),
        Paint::Radial { stops: st, spread, linear_rgb, focal, .. } => json!({"type": "radial", "stops": stops(st), "spread": spread.key(), "linear_rgb": linear_rgb, "focal": focal}),
        Paint::Bitmap { bitmap, repeat, .. } => json!({"type": "bitmap", "bitmap": bitmap.0, "repeat": repeat}),
    }
}

pub(crate) fn opt_focal(p: &Value) -> Result<Option<f32>> {
    let f = opt_f32(p, "focal")?;
    if f.is_some_and(|f| !(-MAX_FOCAL..=MAX_FOCAL).contains(&f)) {
        return Err(EngineError::Param(format!("param 'focal' must be between -{MAX_FOCAL} and {MAX_FOCAL}")));
    }
    Ok(f)
}

/// `paint.set_fill`: the Colour panel's fill (type, stops, overflow, linear RGB, bitmap).
pub(crate) fn set_fill(s: &mut Session, p: &Value) -> Result<Value> {
    let swatch = s.style.fill_paint();
    let kind = match opt_str(p, "type")? {
        None => fill_kind(&swatch),
        Some("solid") => FillKind::Solid,
        Some("linear") => FillKind::Linear,
        Some("radial") => FillKind::Radial,
        Some("bitmap") => FillKind::Bitmap,
        Some(other) => return Err(EngineError::Param(format!("unknown fill type '{other}' (solid, linear, radial or bitmap)"))),
    };
    let spread = match opt_str(p, "spread")? {
        None => None,
        Some(k) => Some(Spread::from_key(k).ok_or_else(|| EngineError::Param(format!("unknown spread '{k}' (extend, reflect or repeat)")))?),
    };
    let bitmap = match opt_u64(p, "bitmap")? {
        None => None,
        Some(id) => Some(s.doc.bitmap(BitmapId(id)).ok_or_else(|| EngineError::Param(format!("there is no bitmap {id} in the Library")))?.id),
    };
    let spec = FillSpec { kind, color: opt_color(p, "color")?, stops: opt_stops(p)?, spread, linear_rgb: opt_bool(p, "linear_rgb")?, focal: opt_focal(p)?, bitmap };
    let new = spec.apply(&swatch).ok_or_else(|| EngineError::Param("a bitmap fill needs 'bitmap' (a Library bitmap's id)".into()))?;
    // The swatch keeps where its fill lies; a new kind lies across the stage.
    let stage = Rect::new(0.0, 0.0, s.doc.stage.width as f32, s.doc.stage.height as f32);
    let placed = if fill_kind(&swatch) == kind { new } else { new.fitted(&stage) };
    match &placed {
        Paint::Solid(c) => s.style.set_fill_color(*c),
        other => {
            s.style.fill = other.primary_color();
            s.style.fill_paint = Some(other.clone());
        }
    }
    s.style.use_fill = true;
    let changed = if opt_bool(p, "selection")?.unwrap_or(true) && !s.selection.is_empty() {
        // Counts from several layers are added up.
        s.each_layer(false, |s| set_selected_fills(s, &spec))?.get("changed").and_then(Value::as_u64).unwrap_or(0)
    } else {
        0
    };
    Ok(json!({"fill": fill_json(&placed), "changed": changed}))
}

/// The selected shapes with a fill on the active layer get `spec`: the same kind of fill keeps
/// where it lies, another kind is fitted to the shape.
fn set_selected_fills(s: &mut Session, spec: &FillSpec) -> Result<Value> {
    let sel: std::collections::HashSet<ElementId> = s.selection.on(s.layer).into_iter().collect();
    if sel.is_empty() {
        return Ok(json!({"changed": 0}));
    }
    let mut changed = 0u64;
    let mut merged = Vec::new();
    for e in s.editable_keyframe()?.elements_mut_where(|id| sel.contains(&id)) {
        let id = e.id;
        let ElementKind::Shape(sh) = &mut e.kind else { continue };
        let Some(old) = &sh.fill else { continue };
        let Some(mut new) = spec.apply(old) else { continue };
        if fill_kind(old) != spec.kind {
            new = new.fitted(&sh.path.bounds());
        }
        if sh.fill.as_ref() != Some(&new) {
            sh.fill = Some(new);
            changed += 1;
            if sh.merge {
                merged.push(id);
            }
        }
    }
    // A raw fill joins the fills of its new paint it touches (as the Paint Bucket's recolour).
    if !merged.is_empty() {
        crate::cmds_raw::remerge_in_place(s, &merged)?;
    }
    Ok(json!({"changed": changed}))
}

/// The size of the image a bitmap fill shows (`None`: not a bitmap, or missing).
pub fn image_size(doc: &anicraft_doc::Document, p: &Paint) -> Option<(f32, f32)> {
    match p {
        Paint::Bitmap { bitmap, .. } => doc.bitmap(*bitmap).map(|b| (b.width as f32, b.height as f32)),
        _ => None,
    }
}

/// Where element `e`'s gradient or bitmap fill lies: its frame (see [`Paint::frame`]) in the
/// space being edited. `None` for anything else.
pub fn fill_frame(doc: &anicraft_doc::Document, e: &Element) -> Option<Affine> {
    let ElementKind::Shape(sh) = &e.kind else { return None };
    let f = sh.fill.as_ref()?;
    Some(e.transform.mul(&f.frame(image_size(doc, f))?))
}

fn opt_vec2(p: &Value, key: &str) -> Result<Option<Vec2>> {
    let bad = || EngineError::Param(format!("param '{key}' must be [x, y] (finite numbers)"));
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(a)) if a.len() == 2 => {
            let v = |i: usize| a.get(i).and_then(Value::as_f64).filter(|x| x.is_finite() && x.abs() <= f64::from(crate::cmds_draw::MAX_EXTENT)).map(|x| x as f32);
            match (v(0), v(1)) {
                (Some(x), Some(y)) => Ok(Some(Vec2::new(x, y))),
                _ => Err(bad()),
            }
        }
        Some(_) => Err(bad()),
    }
}

/// The frame `paint.transform_fill`'s params ask for, from the current one `cur`.
fn new_frame(cur: &Affine, p: &Value) -> Result<Affine> {
    if let Some(v) = p.get("frame").filter(|v| !v.is_null()) {
        let a = v.as_array().filter(|a| a.len() == 6).ok_or_else(|| EngineError::Param("param 'frame' must be [a, b, c, d, e, f]".into()))?;
        let mut m = [0.0f32; 6];
        for (slot, x) in m.iter_mut().zip(a) {
            *slot = x.as_f64().filter(|x| x.is_finite() && x.abs() <= 1e7).ok_or_else(|| EngineError::Param("param 'frame' must hold six finite numbers".into()))? as f32;
        }
        return Ok(Affine { m });
    }
    let scale = |k: &str| -> Result<f32> {
        match opt_f32(p, k)? {
            None => Ok(1.0),
            Some(v) if (1e-4..=1e4).contains(&v.abs()) => Ok(v),
            Some(_) => Err(EngineError::Param(format!("param '{k}' must be a scale between 0.0001 and 10000 (negative flips)"))),
        }
    };
    let both = scale("scale")?;
    let (sx, sy) = (scale("scale_x")? * both, scale("scale_y")? * both);
    let skew = |k: &str| -> Result<f32> {
        match opt_f32(p, k)? {
            None => Ok(0.0),
            Some(d) if d.abs() <= 85.0 => Ok(d.to_radians().tan()),
            Some(_) => Err(EngineError::Param(format!("param '{k}' must be between -85 and 85 degrees"))),
        }
    };
    let own = Affine::scale(sx, sy).mul(&Affine { m: [1.0, skew("skew_y")?, skew("skew_x")?, 1.0, 0.0, 0.0] });
    let mut f = cur.mul(&own);
    if let Some(deg) = opt_f32(p, "rotate")? {
        let c = f.apply(Vec2::ZERO);
        f = Affine::translate(c.x, c.y).mul(&Affine::rotate(deg.to_radians())).mul(&Affine::translate(-c.x, -c.y)).mul(&f);
    }
    if let Some(d) = opt_vec2(p, "move")? {
        f = Affine::translate(d.x, d.y).mul(&f);
    }
    if let Some(at) = opt_vec2(p, "center")? {
        let c = f.apply(Vec2::ZERO);
        f = Affine::translate(at.x - c.x, at.y - c.y).mul(&f);
    }
    Ok(f)
}

/// `paint.transform_fill`: the Gradient Transform tool's edit.
fn transform_fill(s: &mut Session, p: &Value) -> Result<Value> {
    let focal = opt_focal(p)?;
    let only = opt_u64(p, "element")?.map(ElementId);
    let targets: std::collections::HashSet<ElementId> = match only {
        Some(id) => [id].into(),
        None => s.selection.on(s.layer).into_iter().collect(),
    };
    if targets.is_empty() {
        return Err(EngineError::State("select a shape with a gradient or bitmap fill".into()));
    }
    let doc = s.doc.clone();
    let mut frames = Vec::new();
    for e in s.editable_keyframe()?.elements_mut_where(|id| targets.contains(&id)) {
        let (Some(cur), Some(to_local)) = (fill_frame(&doc, e), e.transform.inverse()) else { continue };
        let f = new_frame(&cur, p)?;
        if !f.is_finite() || f.determinant().abs() < 1e-6 {
            return Err(EngineError::Param("that would flatten the fill to nothing".into()));
        }
        let ElementKind::Shape(sh) = &mut e.kind else { continue };
        let Some(fill) = &sh.fill else { continue };
        let mut new = fill.with_frame(&to_local.mul(&f), image_size(&doc, fill));
        if let (Paint::Radial { focal: slot, .. }, Some(v)) = (&mut new, focal) {
            *slot = v;
        }
        frames.push(json!(f.m));
        // Only where the fill lies changes, so a raw fill stays the shape it is (no re-merge:
        // its id holds through a drag, and a moved gradient matches no neighbour's anyway).
        sh.fill = Some(new);
    }
    if frames.is_empty() {
        return Err(EngineError::State(match only {
            Some(_) => "that shape has no gradient or bitmap fill on the active layer at this frame".into(),
            None => "none of the selected shapes has a gradient or bitmap fill".into(),
        }));
    }
    Ok(json!({"changed": frames.len(), "frames": frames}))
}

/// `paint.lock_fill`.
fn lock_fill(s: &mut Session, p: &Value) -> Result<Value> {
    let on = opt_bool(p, "on")?.unwrap_or(!s.style.lock_fill);
    s.style.lock_fill = on;
    Ok(json!({"on": on}))
}
