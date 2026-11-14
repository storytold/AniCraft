//! Drawing, selection, element and symbol commands.

use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Selection, Session};
use anicraft_doc::{ColorEffect, ColorStyle, Element, ElementId, ElementKind, Layer, LayerId, LibraryEntry, LibraryItem, LoopMode, Paint, ScaleGrid, Shape, Stroke, Symbol, SymbolId, SymbolKind, Timeline};
use anicraft_geom::{Affine, Decomposed, Path, Vec2};
use serde_json::{Value, json};
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c("draw.rect", "Rectangle", "", None, "{x, y, w, h, radius?, fill?, stroke?, stroke_width?, profile?, cap?, join?, style?, dash?, object?}", true, draw_rect as crate::RunFn),
        c("draw.ellipse", "Oval", "", None, "{x, y, w, h, fill?, stroke?, stroke_width?, profile?, cap?, join?, style?, dash?, object?}", true, draw_ellipse),
        c("draw.star", "PolyStar", "", None, "{cx, cy, outer (the corners' radius), shape?: star|polygon (default star; the PolyStar Tool Settings, ADR 0080), sides? (or points; 3..64, default 5), point_size? (0..1, default 0.5: how deep a star's points are, 1 is the polygon through its tips), inner? (an exact inner radius instead of point_size), fill?, stroke?, stroke_width?, profile?, cap?, join?, style?, dash? (the line style), object?}", true, draw_star),
        c("draw.line", "Line", "", None, "{x0, y0, x1, y1, stroke?, stroke_width?, profile? (a width profile name or [[t, left, right], ..]), cap?, join?, miter?, style?, dash?, dash_offset?, dot_spacing? (the line style, as stroke.set_style; default: the tool's), object? (true: a drawing object; default: Object Drawing, off: it merges with the raw shapes it touches)}", true, draw_line),
        c("draw.object_drawing", "Object Drawing", "", Some("J"), "{on?: bool} (default: toggle)", false, object_drawing),
        c("draw.snap_to_objects", "Snap to Objects", "View/Snapping", Some("Ctrl+Shift+/"), "{on?: bool} (default: toggle) dragged points and selections, and the points the Line, Rectangle, Oval, PolyStar and Pen tools draw, snap to points, lines and fill edges (while it is off, the `snap` params of element.translate, raw.move_point and draw.snap only snap to guides, the grid and pixels, as View › Guides, Grid and Snapping say)", false, snap_to_objects),
        c("edit.select_all", "Select All", "Edit", Some("Ctrl+A"), "{}", false, select_all),
        c("edit.deselect", "Deselect All", "Edit", Some("Ctrl+Shift+A"), "{}", false, |s, _| {
            s.selection.clear();
            Ok(json!({"selected": 0}))
        }),
        c("edit.delete", "Clear", "Edit", Some("Delete"), "{} (on every layer with selected items)", true, delete_selection),
        c("edit.duplicate", "Duplicate", "Edit", Some("Ctrl+D"), "{dx?, dy?} (each copy on its original's layer)", true, |s, p| s.each_layer(true, |s| duplicate(s, p))),
        c("element.select", "Select Elements", "", None, "{ids: [..], add?: bool, toggle?: bool, layer?} selects elements on any visible, unlocked layer at the playhead (ids are unique; `layer` looks only there); `toggle` takes each one that is selected out of the selection and adds the others (Shift+Space on a stage element); the active layer becomes the first one's unless it has some of them", false, select),
        c("element.pick", "Select at Point", "", None, "{x, y, add?: bool, view?: stage|camera} the topmost element there on a visible, unlocked layer; `add` (Shift) adds it to the selection, or lets go of it, on any layer; with `view: camera` the point is on the movie's picture as the camera shows it (each layer through its depth)", false, pick),
        c("element.translate", "Move", "", None, "{dx, dy, snap?, from?: [x, y]} moves the selection, on every layer; with `snap` (a distance) one of its corners or ends, or its box's corners or centre (the nearest to `from`), then snaps within that distance by the snap engine's priority (ADR 0076): a point, guide, line or fill edge of the rest, the grid, a whole pixel", true, translate),
        c("element.set_transform", "Transform", "", None, "{x?, y?, sx?, sy?, rotation? (degrees)}", true, |s, p| s.each_layer(true, |s| set_transform(s, p))),
        c("element.set_fill", "Fill Colour", "", None, "{color: \"#rrggbb\" | null}", true, |s, p| s.each_layer(false, |s| set_fill(s, p))),
        c("element.set_stroke", "Stroke", "", None, "{color?: \"#rrggbb\" | null, width?, profile?: uniform|taper_both|taper_start|taper_end|bulge|pinch | [[t, left, right], ..], cap?, join?, miter?, style?, dash?, dash_offset?, dot_spacing? (as stroke.set_style)}", true, |s, p| s.each_layer(false, |s| set_stroke(s, p))),
        c("element.set_alpha", "Alpha", "", None, "{alpha: 0..1} sets the alpha multiplier and keeps the rest of the colour effect (None becomes Alpha; Brightness and Tint become Advanced below 1)", true, |s, p| s.each_layer(true, |s| set_alpha(s, p))),
        c("element.set_color_effect", "Colour Effect", "", None, "{style?: none|brightness|tint|advanced|alpha (default: the style the values belong to, else each element's own), alpha? (%, 0..100), brightness? (%, -100..100), tint? (#rrggbb), tint_amount? (%, 0..100), red_multiplier?, green_multiplier?, blue_multiplier?, alpha_multiplier? (%, -100..100), red_offset?, green_offset?, blue_offset?, alpha_offset? (-255..255)} on every selected element (ADR 0056); values not given keep each element's own. A new style starts from the look it can keep: Advanced is exactly the effect before, Alpha keeps the opacity, Brightness and Tint start at 0 %. The camera has no opacity (no Alpha style, alpha multiplier or offset)", true, |s, p| s.each_layer(true, |s| set_color_effect(s, p))),
        c("element.set_blend", "Blending", "", None, "{mode: normal|layer|darken|multiply|lighten|screen|overlay|hardlight|add|subtract|difference|invert|alpha|erase} on the selected movie clip and button instances", true, |s, p| s.each_layer(false, |s| set_blend(s, p))),
        c("element.set_filters", "Filters", "", None, "{filters: [{type: blur|drop_shadow|glow|bevel|gradient_glow|gradient_bevel|adjust_color, …values, enabled?: bool (default true; false switches it off, keeping its settings)}]} replaces the selected movie clip and button instances' filters (in order; [] removes them). Bevel: shadow_color, highlight_color, side: inner|outer|full, knockout; gradient filters: stops: [{offset 0..1, color \"#rrggbbaa\"}] (Gradient Glow's start and Gradient Bevel's middle stay transparent)", true, |s, p| s.each_layer(false, |s| crate::cmds_filter::set_filters(s, p))),
        c("element.set_loop", "Looping", "", None, "{loop?: loop|play_once|single_frame, first_frame?}", true, |s, p| s.each_layer(false, |s| set_loop(s, p))),
        c("symbol.set_kind", "Symbol Type", "", None, "{kind: graphic|movie_clip|button, symbol?: id (default: the symbols of every selected instance)}", true, set_symbol_kind),
        c(
            "symbol.set_scale_grid",
            "9-Slice Scaling",
            "",
            None,
            "{symbol?: id (default: the symbol open for editing, else the selected instance's), on?: bool, left?, right?, top?, bottom? (guide positions in the symbol's space)} turns a symbol's guides for 9-slice scaling on or off, or moves them (ADR 0055). Turning them on brings back the guides it had when they were turned off (ADR 0057), else puts them a quarter of the way in from each edge of the symbol's artwork; moving one turns them on. Scaled instances then keep the corners outside the guides unscaled, stretch the edges one way and the centre both ways → {symbol, scale_grid: {left, right, top, bottom} | null}",
            true,
            set_scale_grid,
        ),
        c("element.rename", "Instance Name", "", None, "{name}", true, |s, p| s.each_layer(true, |s| rename(s, p))),
        c("arrange.bring_to_front", "Bring to Front", "Modify/Arrange", Some("Ctrl+Shift+Up"), "{} (within each layer)", true, |s, _| s.each_layer(true, |s| arrange(s, true))),
        c("arrange.bring_forward", "Bring Forward", "Modify/Arrange", Some("Ctrl+Up"), "{} (within each layer)", true, |s, _| s.each_layer(true, |s| arrange_step(s, true))),
        c("arrange.send_backward", "Send Backward", "Modify/Arrange", Some("Ctrl+Down"), "{} (within each layer)", true, |s, _| s.each_layer(true, |s| arrange_step(s, false))),
        c("arrange.send_to_back", "Send to Back", "Modify/Arrange", Some("Ctrl+Shift+Down"), "{} (within each layer)", true, |s, _| s.each_layer(true, |s| arrange(s, false))),
        c("modify.convert_to_symbol", "Convert to Symbol…", "Modify", Some("F8"), "{name? (default: the next free “Symbol n”; a name the folder holds gets a number), kind?: graphic|movie_clip|button (default: graphic), folder?: Library folder id (default: the top level)} → {symbol, instance, name, layer, folder} (a selection on several layers becomes one symbol with those layers inside; the instance lands on the topmost)", true, convert_to_symbol),
        c("modify.break_apart", "Break Apart", "Modify", Some("Ctrl+B"), "{} the selected symbol instances become their contents, text becomes shapes, and drawing objects become raw shapes that merge with the raw shapes they touch", true, |s, p| s.each_layer(false, |s| break_apart(s, p))),
        c("library.place", "Place on Stage", "", None, "{symbol | bitmap | sound: id, x?, y?} or {items: [{symbol | bitmap | sound: id}, ..], x?, y?} → {element, elements, sound?}: symbols and bitmaps centred on x, y (default: the stage centre; inside an open symbol, in its space) in the active layer's keyframe at the playhead, selected; a sound goes onto that keyframe, its span grown to fit (one sound per keyframe). One undo step", true, place),
    ]
}

// ----- drawing ----------------------------------------------------------------------------

/// Max path size accepted from params (extent in stage units).
pub(crate) const MAX_EXTENT: f32 = 1_000_000.0;

pub(crate) fn style(s: &Session, p: &Value, closed: bool) -> Result<(Option<Paint>, Option<Stroke>)> {
    let fill = if !closed || p.get("fill").is_some_and(Value::is_null) {
        None
    } else {
        match opt_color(p, "fill")? {
            Some(c) => Some(Paint::Solid(c)),
            // The fill swatch, in stage space (`add_shape` maps it into the shape).
            None => s.style.use_fill.then(|| s.style.fill_paint()),
        }
    };
    let width = match opt_f32(p, "stroke_width")? {
        Some(w) if (0.0..=1000.0).contains(&w) => w,
        Some(_) => return Err(EngineError::Param("param 'stroke_width' must be in 0..=1000".into())),
        None => s.style.stroke_width,
    };
    let profile = opt_profile(p, "profile")?.unwrap_or_else(|| s.style.profile());
    let line = opt_stroke_style(p, &s.style.line, width)?.unwrap_or_else(|| s.style.line.clone());
    let stroke = if p.get("stroke").is_some_and(Value::is_null) || width <= 0.0 {
        None
    } else {
        match opt_color(p, "stroke")? {
            Some(c) => Some(Stroke::new(c, width)),
            None => (s.style.use_stroke || !closed).then_some(Stroke::new(s.style.stroke, width)),
        }
    }
    .map(|st| st.with_profile(profile).with_style(line));
    if fill.is_none() && stroke.is_none() {
        return Err(EngineError::Param("shape would be invisible: give it a fill or a stroke".into()));
    }
    Ok((fill, stroke))
}

fn rect_params(p: &Value) -> Result<(f32, f32, f32, f32)> {
    let (x, y, w, h) = (f32_req(p, "x")?, f32_req(p, "y")?, f32_req(p, "w")?, f32_req(p, "h")?);
    if w.abs() < 0.5 || h.abs() < 0.5 {
        return Err(EngineError::Param("shape is too small (width and height must be at least 0.5)".into()));
    }
    if w.abs() > MAX_EXTENT || h.abs() > MAX_EXTENT {
        return Err(EngineError::Param("shape is too large".into()));
    }
    // normalise negative sizes (dragging up/left)
    Ok((x.min(x + w), y.min(y + h), w.abs(), h.abs()))
}

/// Adds a shape whose local origin is its centre, placed with a translation. Unless `object`
/// (Object Drawing), it merges with the raw shapes it touches (ADR 0031), and the pieces of it
/// that come out are selected.
pub(crate) fn add_shape(s: &mut Session, name: &str, local: Path, origin: Vec2, fill: Option<Paint>, stroke: Option<Stroke>, object: bool) -> Result<Value> {
    // A gradient or bitmap from the fill swatch is in stage space: with Lock Fill it stays where
    // it is on the stage, else it is fitted to the new shape (ADR 0074).
    let fill = fill.map(|f| if s.style.lock_fill || matches!(f, Paint::Solid(_)) { f.transformed(&Affine::translate(-origin.x, -origin.y)) } else { f.fitted(&local.bounds()) });
    let merge = !object;
    let id = ElementId(s.doc.alloc_id());
    let el = Element {
        id,
        name: String::new(),
        transform: Affine::translate(origin.x, origin.y),
        color: ColorEffect::default(),
        kind: ElementKind::Shape(Shape { path: Arc::new(local), fill, stroke, fill_rule: Default::default(), merge, warp: None }),
    };
    if merge {
        let merged = crate::merge::merge_in(s, vec![el])?;
        s.select_only(merged);
        let ids: Vec<u64> = s.selection.id_list();
        return Ok(json!({"element": ids.first(), "elements": ids, "kind": name, "merged": true}));
    }
    let kf = s.editable_keyframe()?;
    if kf.elements.len() >= 100_000 {
        return Err(EngineError::State("too many elements on this keyframe".into()));
    }
    kf.elements.push(Arc::new(el));
    s.select_only([id]);
    Ok(json!({"element": id.0, "kind": name}))
}

/// The `object` param: draw an object instead of merging (default: the Object Drawing setting).
pub(crate) fn object_param(s: &Session, p: &Value) -> Result<bool> {
    Ok(opt_bool(p, "object")?.unwrap_or(s.style.object_drawing))
}

fn draw_rect(s: &mut Session, p: &Value) -> Result<Value> {
    let (x, y, w, h) = rect_params(p)?;
    let r = opt_f32(p, "radius")?.unwrap_or(0.0).max(0.0);
    let (fill, stroke) = style(s, p, true)?;
    add_shape(s, "rect", Path::rounded_rect(-w / 2.0, -h / 2.0, w, h, r), Vec2::new(x + w / 2.0, y + h / 2.0), fill, stroke, object_param(s, p)?)
}

fn draw_ellipse(s: &mut Session, p: &Value) -> Result<Value> {
    let (x, y, w, h) = rect_params(p)?;
    let (fill, stroke) = style(s, p, true)?;
    add_shape(s, "ellipse", Path::ellipse(0.0, 0.0, w / 2.0, h / 2.0), Vec2::new(x + w / 2.0, y + h / 2.0), fill, stroke, object_param(s, p)?)
}

fn draw_star(s: &mut Session, p: &Value) -> Result<Value> {
    let (cx, cy, outer) = (f32_req(p, "cx")?, f32_req(p, "cy")?, f32_req(p, "outer")?);
    if !(0.5..=MAX_EXTENT).contains(&outer) {
        return Err(EngineError::Param("param 'outer' must be between 0.5 and 1,000,000".into()));
    }
    // PolyStar's Tool Settings (ADR 0080): a polygon or a star, its sides (or points) and the
    // star's point size.
    let polygon = match opt_str(p, "shape")?.unwrap_or("star") {
        "star" => false,
        "polygon" => true,
        other => return Err(EngineError::Param(format!("unknown PolyStar style '{other}' (polygon or star)"))),
    };
    let points = match (opt_u32(p, "sides")?, opt_u32(p, "points")?) {
        (Some(n), _) | (None, Some(n)) => n,
        (None, None) => 5,
    };
    if !(3..=64).contains(&points) {
        return Err(EngineError::Param("param 'sides' (or 'points') must be between 3 and 64".into()));
    }
    let point_size = match opt_f32(p, "point_size")? {
        None => 0.5,
        Some(v) if (0.0..=1.0).contains(&v) => v,
        Some(_) => return Err(EngineError::Param("param 'point_size' must be between 0 and 1".into())),
    };
    let (fill, stroke) = style(s, p, true)?;
    let object = object_param(s, p)?;
    if polygon {
        return add_shape(s, "polygon", Path::polystar(0.0, 0.0, outer, points, false, point_size), Vec2::new(cx, cy), fill, stroke, object);
    }
    // An explicit inner radius (scripts written before the point size) still wins.
    let path = match opt_f32(p, "inner")? {
        Some(inner) => Path::star(0.0, 0.0, outer, inner.clamp(0.0, MAX_EXTENT), points),
        None => Path::polystar(0.0, 0.0, outer, points, true, point_size),
    };
    add_shape(s, "star", path, Vec2::new(cx, cy), fill, stroke, object)
}

fn draw_line(s: &mut Session, p: &Value) -> Result<Value> {
    let a = Vec2::new(f32_req(p, "x0")?, f32_req(p, "y0")?);
    let b = Vec2::new(f32_req(p, "x1")?, f32_req(p, "y1")?);
    if a.distance(b) < 0.5 {
        return Err(EngineError::Param("line is too short".into()));
    }
    let (_, stroke) = style(s, p, false)?;
    let c = (a + b) * 0.5;
    add_shape(s, "line", Path::line(a - c, b - c), c, None, stroke, object_param(s, p)?)
}

fn snap_to_objects(s: &mut Session, p: &Value) -> Result<Value> {
    s.style.snap_objects = opt_bool(p, "on")?.unwrap_or(!s.style.snap_objects);
    Ok(json!({"snap_to_objects": s.style.snap_objects}))
}

fn object_drawing(s: &mut Session, p: &Value) -> Result<Value> {
    s.style.object_drawing = opt_bool(p, "on")?.unwrap_or(!s.style.object_drawing);
    Ok(json!({"object_drawing": s.style.object_drawing}))
}

// ----- selection --------------------------------------------------------------------------

/// Everything on every visible, unlocked layer at the playhead, as Animate's Select All.
fn select_all(s: &mut Session, _: &Value) -> Result<Value> {
    let layers: Vec<LayerId> = s.timeline().layers.iter().map(|l| l.id).collect();
    let mut sel = Selection::default();
    for l in layers.into_iter().filter(|l| s.gathers(*l)) {
        sel.extend_on(l, s.elements_on(l).iter().map(|e| e.id));
    }
    s.selection = sel;
    Ok(json!({"selected": s.selection.len()}))
}

/// Most ids `element.select` takes.
const MAX_SELECT: usize = 100_000;

fn select(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = opt_u64_list(p, "ids")?.ok_or_else(|| EngineError::Param("param 'ids' is required".into()))?;
    if ids.len() > MAX_SELECT {
        return Err(EngineError::Param(format!("param 'ids' has more than {MAX_SELECT} ids")));
    }
    let add = opt_bool(p, "add")?.unwrap_or(false);
    let toggle = opt_bool(p, "toggle")?.unwrap_or(false);
    // Where each element is at the playhead: on `layer`, else the active layer first, then the
    // others top to bottom.
    let order: Vec<LayerId> = match opt_u64(p, "layer")?.map(LayerId) {
        Some(l) if s.timeline().layer(l).is_none() => return Err(EngineError::State(format!("no layer with id {}", l.0))),
        Some(l) => vec![l],
        None => std::iter::once(s.layer).chain(s.timeline().layers.iter().map(|l| l.id).filter(|l| *l != s.layer)).collect(),
    };
    let mut at: std::collections::HashMap<ElementId, LayerId> = std::collections::HashMap::new();
    for l in order {
        for e in s.elements_on(l) {
            at.entry(e.id).or_insert(l);
        }
    }
    let mut picked: Vec<(LayerId, ElementId)> = Vec::with_capacity(ids.len());
    for id in ids.into_iter().map(ElementId) {
        let Some(&l) = at.get(&id) else {
            return Err(EngineError::State(format!("element {} is not on the active layer, or any layer, at this frame", id.0)));
        };
        if !s.selectable(l) {
            let name = s.timeline().layer(l).map(|x| x.name.clone()).unwrap_or_default();
            return Err(EngineError::State(format!("element {} is on layer '{name}', which is locked or hidden", id.0)));
        }
        picked.push((l, id));
    }
    if toggle {
        for &(l, id) in &picked {
            s.selection.toggle(l, id);
        }
    } else {
        if !add {
            s.selection.clear();
        }
        for &(l, id) in &picked {
            s.selection.insert(l, id);
        }
    }
    // As a click does: the active layer is where the artwork is.
    if let Some(&(l, _)) = picked.first()
        && !picked.iter().any(|(x, _)| *x == s.layer)
    {
        s.layer = l;
    }
    Ok(json!({"selected": s.selection.id_list()}))
}

fn pick(s: &mut Session, p: &Value) -> Result<Value> {
    let pt = Vec2::new(f32_req(p, "x")?, f32_req(p, "y")?);
    let add = opt_bool(p, "add")?.unwrap_or(false);
    // `view: "camera"`: the point is on the movie's picture, seen through the camera (ADR 0082).
    let through = match opt_str(p, "view")? {
        None | Some("stage") => false,
        Some("camera") => true,
        Some(v) => return Err(EngineError::Param(format!("unknown view '{v}' (stage or camera)"))),
    };
    match if through { s.hit_test_camera(pt, 2.0) } else { s.hit_test(pt, 2.0) } {
        Some((layer, el)) => {
            // Shift adds it to (or takes it from) the selection, whichever layers that is on.
            if add {
                s.selection.toggle(layer, el);
            } else if !s.selection.contains(layer, el) {
                s.selection = Selection::only(layer, [el]);
            }
            s.layer = layer;
            Ok(json!({"layer": layer.0, "element": el.0}))
        }
        None => {
            if !add {
                s.selection.clear();
            }
            Ok(json!({"element": null}))
        }
    }
}

// ----- element edits ----------------------------------------------------------------------

fn translate(s: &mut Session, p: &Value) -> Result<Value> {
    let (mut dx, mut dy) = (f32_req(p, "dx")?, f32_req(p, "dy")?);
    let snap = opt_f32(p, "snap")?.unwrap_or(0.0).clamp(0.0, 1000.0);
    let from = match p.get("from").filter(|v| !v.is_null()) {
        None => None,
        Some(v) => {
            let xy = v.as_array().filter(|a| a.len() == 2).and_then(|a| Some(Vec2::new(a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32)));
            Some(xy.filter(|q| q.is_finite()).ok_or_else(|| EngineError::Param("param 'from' must be [x, y]".into()))?)
        }
    };
    let n = move_by(s, Vec2::new(dx, dy))?;
    // Snapping while dragging (ADR 0041): a little more, onto what is near.
    let mut snapped = Value::Null;
    if s.snaps(snap)
        && let Some((extra, at, kind)) = crate::cmds_raw::snap_selection(s, snap, from)
    {
        move_by(s, extra)?;
        (dx, dy) = (dx + extra.x, dy + extra.y);
        snapped = json!({"x": at.x, "y": at.y, "to": kind.key()});
    }
    Ok(json!({"moved": n, "dx": dx, "dy": dy, "snapped": snapped}))
}

/// Moves the selection by `d` on every layer it is on; how many elements moved.
fn move_by(s: &mut Session, d: Vec2) -> Result<u64> {
    let v = s.each_layer(true, |s| {
        let els = s.selected_elements_mut()?;
        let n = els.len();
        for e in els {
            e.transform = Affine::translate(d.x, d.y).mul(&e.transform);
        }
        Ok(json!({"moved": n}))
    })?;
    Ok(v["moved"].as_u64().unwrap_or(0))
}

fn set_transform(s: &mut Session, p: &Value) -> Result<Value> {
    let x = opt_f32(p, "x")?;
    let y = opt_f32(p, "y")?;
    let sx = opt_f32(p, "sx")?;
    let sy = opt_f32(p, "sy")?;
    let rot = opt_f32(p, "rotation")?;
    for v in [sx, sy].into_iter().flatten() {
        if v.abs() < 1e-3 || v.abs() > 1000.0 {
            return Err(EngineError::Param("scale must be between 0.001 and 1000 in magnitude".into()));
        }
    }
    let els = s.selected_elements_mut()?;
    for e in els {
        let mut d: Decomposed = e.transform.decompose();
        if let Some(v) = x {
            d.tx = v;
        }
        if let Some(v) = y {
            d.ty = v;
        }
        if let Some(v) = sx {
            d.sx = v;
        }
        if let Some(v) = sy {
            d.sy = v;
        }
        if let Some(v) = rot {
            d.rotation = v.to_radians();
        }
        e.transform = d.compose();
    }
    Ok(json!({"ok": true}))
}

fn shapes_mut(s: &mut Session) -> Result<Vec<&mut Shape>> {
    let shapes: Vec<&mut Shape> = s
        .selected_elements_mut()?
        .into_iter()
        .filter_map(|e| match &mut e.kind {
            ElementKind::Shape(sh) => Some(sh),
            ElementKind::Symbol { .. } | ElementKind::Text(_) | ElementKind::Camera | ElementKind::Group { .. } => None,
        })
        .collect();
    if shapes.is_empty() {
        return Err(EngineError::State("the selection has no shapes (symbol instances use alpha/tint; text has its own colour)".into()));
    }
    Ok(shapes)
}

fn set_fill(s: &mut Session, p: &Value) -> Result<Value> {
    let fill = if p.get("color").is_some_and(Value::is_null) {
        None
    } else {
        Some(Paint::Solid(opt_color(p, "color")?.ok_or_else(|| EngineError::Param("param 'color' is required".into()))?))
    };
    for sh in shapes_mut(s)? {
        sh.fill = fill.clone();
    }
    Ok(json!({"ok": true}))
}

pub(crate) fn set_stroke(s: &mut Session, p: &Value) -> Result<Value> {
    let remove = p.get("color").is_some_and(Value::is_null);
    let color = opt_color(p, "color")?;
    let width = opt_f32(p, "width")?;
    if width.is_some_and(|w| !(0.0..=1000.0).contains(&w)) {
        return Err(EngineError::Param("param 'width' must be in 0..=1000".into()));
    }
    let profile = opt_profile(p, "profile")?;
    let default = s.style.clone();
    // Checked once up front, so a bad style param changes nothing.
    opt_stroke_style(p, &default.line, default.stroke_width)?;
    for sh in shapes_mut(s)? {
        if remove {
            sh.stroke = None;
            continue;
        }
        let mut st = sh.stroke.take().unwrap_or_else(|| default.new_stroke(default.stroke, default.stroke_width));
        if let Some(c) = color {
            st.color = c;
        }
        if let Some(w) = width {
            st.width = w;
        }
        if let Some(pr) = &profile {
            st = st.with_profile(pr.clone());
        }
        if let Some(line) = opt_stroke_style(p, &st.style, st.width)? {
            st = st.with_style(line);
        }
        sh.stroke = (st.width > 0.0).then_some(st);
    }
    Ok(json!({"ok": true}))
}

fn set_alpha(s: &mut Session, p: &Value) -> Result<Value> {
    let a = f32_req(p, "alpha")?;
    if !(0.0..=1.0).contains(&a) {
        return Err(EngineError::Param("param 'alpha' must be between 0 and 1".into()));
    }
    for e in s.selected_props_mut()? {
        e.color = e.color.with_alpha(a);
    }
    Ok(json!({"alpha": a}))
}

/// Each colour effect value `element.set_color_effect` takes: its key, style, range and scale
/// (the model's value is the param's divided by it).
const EFFECT_VALUES: [(&str, ColorStyle, f32, f32, f32); 11] = [
    ("alpha", ColorStyle::Alpha, 0.0, 100.0, 100.0),
    ("brightness", ColorStyle::Brightness, -100.0, 100.0, 100.0),
    ("tint_amount", ColorStyle::Tint, 0.0, 100.0, 100.0),
    ("red_multiplier", ColorStyle::Advanced, -100.0, 100.0, 100.0),
    ("green_multiplier", ColorStyle::Advanced, -100.0, 100.0, 100.0),
    ("blue_multiplier", ColorStyle::Advanced, -100.0, 100.0, 100.0),
    ("alpha_multiplier", ColorStyle::Advanced, -100.0, 100.0, 100.0),
    ("red_offset", ColorStyle::Advanced, -255.0, 255.0, 255.0),
    ("green_offset", ColorStyle::Advanced, -255.0, 255.0, 255.0),
    ("blue_offset", ColorStyle::Advanced, -255.0, 255.0, 255.0),
    ("alpha_offset", ColorStyle::Advanced, -255.0, 255.0, 255.0),
];

/// A colour effect change as `element.set_color_effect` and `camera.set` take it (ADR 0056):
/// a style, values in percent (or −255..255 for offsets), or both.
pub(crate) struct EffectEdit {
    style: Option<ColorStyle>,
    tint: Option<anicraft_doc::Rgba>,
    values: Vec<(usize, f32)>,
}

impl EffectEdit {
    /// The change `p` asks for; `None` when it names no style and no value.
    pub(crate) fn parse(p: &Value) -> Result<Option<EffectEdit>> {
        let style = match opt_str(p, "style")? {
            Some(k) => Some(ColorStyle::from_key(k).ok_or_else(|| EngineError::Param(format!("unknown style '{k}' (none, brightness, tint, advanced or alpha)")))?),
            None => None,
        };
        let tint = opt_color(p, "tint")?;
        let mut values: Vec<(usize, f32)> = Vec::new();
        let mut families: Vec<ColorStyle> = tint.map(|_| ColorStyle::Tint).into_iter().collect();
        for (i, (key, fam, lo, hi, scale)) in EFFECT_VALUES.iter().enumerate() {
            if let Some(v) = opt_f32(p, key)? {
                if !(*lo..=*hi).contains(&v) {
                    return Err(EngineError::Param(format!("param '{key}' must be between {lo} and {hi}")));
                }
                values.push((i, v / scale));
                if !families.contains(fam) {
                    families.push(*fam);
                }
            }
        }
        let style = match (style, families.as_slice()) {
            (Some(st), f) if f.iter().any(|x| *x != st) => return Err(EngineError::Param(format!("the {} style has none of these values: give only its own", st.key()))),
            (Some(st), _) => Some(st),
            (None, []) => return Ok(None),
            (None, [one]) => Some(*one),
            (None, _) => return Err(EngineError::Param("these values belong to different styles: give one style's values".into())),
        };
        Ok(Some(EffectEdit { style, tint, values }))
    }

    /// Whether it sets an alpha value (the Alpha style, or an alpha multiplier or offset).
    pub(crate) fn touches_alpha(&self) -> bool {
        self.style == Some(ColorStyle::Alpha) || self.values.iter().any(|(i, _)| matches!(i, 0 | 6 | 10))
    }

    /// `fx` changed: into the style (a new one starting from the look it can keep), with the
    /// values given set and the rest kept.
    pub(crate) fn apply(&self, fx: &ColorEffect) -> ColorEffect {
        let mut fx = fx.to_style(self.style.unwrap_or(fx.style()));
        if let ColorEffect::Tint { color, .. } = &mut fx
            && let Some(t) = self.tint
        {
            *color = t.with_alpha(1.0);
        }
        for &(i, v) in &self.values {
            match &mut fx {
                ColorEffect::Alpha(a) if i == 0 => *a = v,
                ColorEffect::Brightness(b) if i == 1 => *b = v,
                ColorEffect::Tint { amount, .. } if i == 2 => *amount = v,
                ColorEffect::Advanced(t) => {
                    if let Some(m) = i.checked_sub(3).and_then(|j| t.mul.get_mut(j)) {
                        *m = v;
                    } else if let Some(o) = i.checked_sub(7).and_then(|j| t.add.get_mut(j)) {
                        *o = v;
                    }
                }
                _ => {}
            }
        }
        fx.sanitized()
    }
}

fn set_color_effect(s: &mut Session, p: &Value) -> Result<Value> {
    let edit = EffectEdit::parse(p)?.ok_or_else(|| EngineError::Param("give a style, or a value to set".into()))?;
    let mut first = None;
    for e in s.selected_props_mut()? {
        if matches!(e.kind, ElementKind::Camera) && edit.touches_alpha() {
            return Err(EngineError::State("the camera has no opacity: its colour effect is None, Brightness, Tint or Advanced (without alpha)".into()));
        }
        e.color = edit.apply(&e.color);
        if matches!(e.kind, ElementKind::Camera) {
            e.color = anicraft_doc::camera::camera_effect(&e.color);
        }
        first.get_or_insert(e.color);
    }
    Ok(first.map_or(Value::Null, |fx| crate::color_effect_json(&fx)))
}

/// Blending for the selected movie clip and button instances (Animate offers it on those only).
fn set_blend(s: &mut Session, p: &Value) -> Result<Value> {
    let key = str_req(p, "mode")?;
    let mode = anicraft_doc::BlendMode::from_key(key).ok_or_else(|| {
        let all: Vec<&str> = anicraft_doc::BlendMode::ALL.iter().map(|m| m.key()).collect();
        EngineError::Param(format!("unknown blend mode '{key}' ({})", all.join(", ")))
    })?;
    let kinds: std::collections::HashMap<SymbolId, SymbolKind> = s.doc.library.iter().map(|x| (x.id, x.kind)).collect();
    let mut set = 0;
    let mut graphics = 0;
    for e in s.selected_props_mut()? {
        if let ElementKind::Symbol { symbol, blend, .. } = &mut e.kind {
            if kinds.get(symbol) == Some(&SymbolKind::Graphic) {
                graphics += 1;
                continue;
            }
            *blend = mode;
            set += 1;
        }
    }
    if set == 0 {
        return Err(EngineError::State(if graphics > 0 {
            "blending applies to movie clip and button instances, as in Animate; set the symbol's Type to Movie Clip first".into()
        } else {
            "select a movie clip or button instance".into()
        }));
    }
    Ok(json!({"mode": mode.key(), "set": set}))
}

/// Looping for the selected graphic instances: loop, play once or a single frame, from
/// `first_frame` (1-based in the UI, 0-based here).
fn set_loop(s: &mut Session, p: &Value) -> Result<Value> {
    let mode = match opt_str(p, "loop")? {
        None => None,
        Some("loop") => Some(LoopMode::Loop),
        Some("play_once" | "play once") => Some(LoopMode::PlayOnce),
        Some("single_frame" | "single frame") => Some(LoopMode::SingleFrame),
        Some(other) => return Err(EngineError::Param(format!("unknown loop '{other}' (loop, play_once or single_frame)"))),
    };
    let first = opt_u32(p, "first_frame")?;
    if mode.is_none() && first.is_none() {
        return Err(EngineError::Param("pass 'loop' and/or 'first_frame'".into()));
    }
    let mut n = 0;
    for e in s.selected_elements_mut()? {
        if let ElementKind::Symbol { loop_mode, first_frame, .. } = &mut e.kind {
            if let Some(m) = mode {
                *loop_mode = m;
            }
            if let Some(f) = first {
                *first_frame = f.min(crate::MAX_FRAME);
            }
            n += 1;
        }
    }
    if n == 0 {
        return Err(EngineError::State("select a symbol instance first".into()));
    }
    Ok(json!({"instances": n}))
}

/// A symbol's kind, for every instance of it: a graphic keeps step with the timeline it is
/// placed on, a movie clip plays its own timeline on a loop, a button shows its Up frame.
fn set_symbol_kind(s: &mut Session, p: &Value) -> Result<Value> {
    let kind = crate::cmds_library::kind_of(str_req(p, "kind")?)?;
    let ids: Vec<SymbolId> = match opt_u64(p, "symbol")? {
        Some(id) => vec![SymbolId(id)],
        None => {
            // The symbols of the selected instances, on every layer (ADR 0053: a "mixed" Type
            // row sets them all), in selection order.
            let mut ids = Vec::new();
            for (layer, sel) in s.selection.iter() {
                if let Some(ElementKind::Symbol { symbol, .. }) = s.elements_on(layer).iter().find(|e| e.id == sel).map(|e| &e.kind)
                    && !ids.contains(symbol)
                {
                    ids.push(*symbol);
                }
            }
            if ids.is_empty() {
                return Err(EngineError::State(if s.selection.is_empty() { "select a symbol instance, or pass 'symbol'".into() } else { "the selection is not a symbol instance".into() }));
            }
            ids
        }
    };
    for id in &ids {
        let sym = s.doc.library.iter_mut().find(|x| x.id == *id).ok_or_else(|| EngineError::State(format!("no symbol with id {}", id.0)))?;
        if sym.kind != kind {
            std::sync::Arc::make_mut(sym).kind = kind;
        }
    }
    Ok(json!({"symbol": ids.first().map(|i| i.0), "symbols": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "kind": format!("{kind:?}")}))
}

/// The symbol a command means: `symbol`, else the selected instance's.
fn selected_symbol(s: &Session, p: &Value) -> Result<SymbolId> {
    Ok(match opt_u64(p, "symbol")? {
        Some(id) => SymbolId(id),
        None => {
            // The symbol of the selected instance.
            let (layer, sel) = s.selection.first().ok_or_else(|| EngineError::State("select a symbol instance, or pass 'symbol'".into()))?;
            s.elements_on(layer)
                .iter()
                .find(|e| e.id == sel)
                .and_then(|e| match e.kind {
                    ElementKind::Symbol { symbol, .. } => Some(symbol),
                    ElementKind::Shape(_) | ElementKind::Text(_) | ElementKind::Camera | ElementKind::Group { .. } => None,
                })
                .ok_or_else(|| EngineError::State("the selection is not a symbol instance".into()))?
        }
    })
}

/// 9-slice guides (ADR 0055): on, off or moved, on one symbol.
fn set_scale_grid(s: &mut Session, p: &Value) -> Result<Value> {
    let on = opt_bool(p, "on")?;
    let edges = [opt_f32(p, "left")?, opt_f32(p, "right")?, opt_f32(p, "top")?, opt_f32(p, "bottom")?];
    if on.is_none() && edges.iter().all(Option::is_none) {
        return Err(EngineError::Param("pass 'on', or a guide to move: 'left', 'right', 'top' or 'bottom'".into()));
    }
    // The open symbol is the one being edited; otherwise the selected instance's.
    let id = match (opt_u64(p, "symbol")?, s.editing_symbol()) {
        (None, Some(open)) => open,
        _ => selected_symbol(s, p)?,
    };
    let sym = s.doc.symbol(id).ok_or_else(|| EngineError::State(format!("no symbol with id {}", id.0)))?;
    let grid = if on == Some(false) {
        if edges.iter().any(Option::is_some) {
            return Err(EngineError::Param("guides can't be moved while turning them off".into()));
        }
        None
    } else {
        // Switched on again, the guides come back where they were (ADR 0057).
        let start = sym.scale_grid.or(sym.scale_grid_kept).unwrap_or_else(|| {
            let b = anicraft_doc::symbol_box(&s.doc, sym);
            ScaleGrid::inset(&if b.is_empty() { anicraft_geom::Rect::new(0.0, 0.0, 100.0, 100.0) } else { b })
        });
        let [l, r, t, b] = edges;
        let g = ScaleGrid { left: l.unwrap_or(start.left), right: r.unwrap_or(start.right), top: t.unwrap_or(start.top), bottom: b.unwrap_or(start.bottom) };
        g.check().map_err(EngineError::Param)?;
        Some(g)
    };
    // Off, the guides are kept (only hidden), so switching back on finds them.
    let kept = if grid.is_none() { sym.scale_grid.or(sym.scale_grid_kept) } else { None };
    if let Some(sym) = s.doc.library.iter_mut().find(|x| x.id == id)
        && (sym.scale_grid != grid || sym.scale_grid_kept != kept)
    {
        let sym = Arc::make_mut(sym);
        sym.scale_grid = grid;
        sym.scale_grid_kept = kept;
    }
    Ok(json!({"symbol": id.0, "scale_grid": grid.map(|g| json!({"left": g.left, "right": g.right, "top": g.top, "bottom": g.bottom}))}))
}

fn rename(s: &mut Session, p: &Value) -> Result<Value> {
    let name: String = str_req(p, "name")?.chars().take(128).collect();
    for e in s.selected_elements_mut()? {
        e.name = name.clone();
    }
    Ok(json!({"name": name}))
}

/// Clear: the selection goes, on every layer it is on.
pub(crate) fn delete_selection(s: &mut Session, p: &Value) -> Result<Value> {
    s.each_layer(true, |s| delete(s, p))
}

fn delete(s: &mut Session, _: &Value) -> Result<Value> {
    let sel = s.selection.on(s.layer);
    if sel.is_empty() {
        return Err(EngineError::State("nothing is selected".into()));
    }
    let kf = s.editable_keyframe()?;
    let before = kf.elements.len();
    kf.elements.retain(|e| !sel.contains(&e.id));
    let n = before - kf.elements.len();
    s.selection.clear();
    Ok(json!({"deleted": n}))
}

fn duplicate(s: &mut Session, p: &Value) -> Result<Value> {
    let dx = opt_f32(p, "dx")?.unwrap_or(10.0);
    let dy = opt_f32(p, "dy")?.unwrap_or(10.0);
    let sel = s.selection.on(s.layer);
    if sel.is_empty() {
        return Err(EngineError::State("nothing is selected".into()));
    }
    let copies: Vec<Element> = s.editable_keyframe()?.elements.iter().filter(|e| sel.contains(&e.id)).map(|e| (**e).clone()).collect();
    let mut new_ids = Vec::new();
    let mut out = Vec::new();
    for mut e in copies {
        e.id = ElementId(s.doc.alloc_id());
        e.transform = Affine::translate(dx, dy).mul(&e.transform);
        new_ids.push(e.id);
        out.push(Arc::new(e));
    }
    s.editable_keyframe()?.elements.extend(out);
    s.select_only(new_ids.iter().copied());
    Ok(json!({"created": new_ids.iter().map(|e| e.0).collect::<Vec<_>>()}))
}

fn arrange(s: &mut Session, front: bool) -> Result<Value> {
    let sel = s.selection.on(s.layer);
    if sel.is_empty() {
        return Err(EngineError::State("nothing is selected".into()));
    }
    let kf = s.editable_keyframe()?;
    let (picked, rest): (Vec<Arc<Element>>, Vec<Arc<Element>>) = kf.elements.drain(..).partition(|e| sel.contains(&e.id));
    kf.elements = if front { rest.into_iter().chain(picked).collect() } else { picked.into_iter().chain(rest).collect() };
    Ok(json!({"ok": true}))
}

/// Bring Forward / Send Backward: each selected element swaps with the unselected neighbour
/// above (or below) it; a selected block moves as one, keeping its order.
fn arrange_step(s: &mut Session, forward: bool) -> Result<Value> {
    let sel = s.selection.on(s.layer);
    if sel.is_empty() {
        return Err(EngineError::State("nothing is selected".into()));
    }
    let kf = s.editable_keyframe()?;
    let n = kf.elements.len();
    let picked = |els: &Vec<Arc<Element>>, i: usize| els.get(i).is_some_and(|e| sel.contains(&e.id));
    if forward {
        // From the top down, so a selected element never jumps over another selected one.
        for i in (0..n.saturating_sub(1)).rev() {
            if picked(&kf.elements, i) && !picked(&kf.elements, i + 1) {
                kf.elements.swap(i, i + 1);
            }
        }
    } else {
        for i in 1..n {
            if picked(&kf.elements, i) && !picked(&kf.elements, i - 1) {
                kf.elements.swap(i, i - 1);
            }
        }
    }
    Ok(json!({"ok": true}))
}

// ----- symbols ----------------------------------------------------------------------------

/// Convert to Symbol. The registration point is the centre of the selection's bounds. A
/// selection on several layers becomes one symbol with those layers inside it (named and
/// stacked as they are), and its instance lands on the topmost of them, where its first
/// selected element was; the other layers lose what was selected (ADR 0044).
pub(crate) fn convert_to_symbol(s: &mut Session, p: &Value) -> Result<Value> {
    let layers = s.selected_layers();
    let Some(&top) = layers.first() else {
        return Err(EngineError::State("select something to convert".into()));
    };
    // Into the folder asked for (ADR 0053), else the top level; the name is free there: the
    // next free “Symbol n”, or the name asked for with a number when the folder has it already.
    let folder = crate::cmds_library::opt_folder(s, p, "folder")?;
    let kind = opt_str(p, "kind")?.map(crate::cmds_library::kind_of).transpose()?.unwrap_or(SymbolKind::Graphic);
    let name: String = match opt_str(p, "name")? {
        Some(n) => s.doc.unique_name(folder, n),
        None => s.doc.numbered_name(folder, "Symbol", s.doc.library.len() + 1),
    };
    let bounds = s.selection_bounds();
    let center = if bounds.is_empty() { Vec2::ZERO } else { bounds.center() };
    let sym_id = SymbolId(s.doc.alloc_id());
    let first_layer = LayerId(s.doc.alloc_id());
    let inst_id = ElementId(s.doc.alloc_id());
    let instance = Element {
        id: inst_id,
        name: String::new(),
        transform: Affine::translate(center.x, center.y),
        color: ColorEffect::default(),
        kind: ElementKind::Symbol { symbol: sym_id, loop_mode: LoopMode::Loop, first_frame: 0, blend: anicraft_doc::BlendMode::Normal, filters: Vec::new(), events: Vec::new() },
    };
    // Children are stored relative to the registration point (the selection centre).
    let to_local = Affine::translate(-center.x, -center.y);
    let single = layers.len() == 1;
    let mut inner: Vec<Arc<Layer>> = Vec::with_capacity(layers.len());
    for l in layers {
        let sel = s.selection.on(l);
        let source = s.timeline().layer(l).map(|x| x.name.clone()).unwrap_or_default();
        let picked = s.with_layer(l, |s| -> Result<Vec<Arc<Element>>> {
            let kf = s.editable_keyframe()?;
            let first_pos = kf.elements.iter().position(|e| sel.contains(&e.id)).unwrap_or(kf.elements.len());
            let (picked, mut rest): (Vec<Arc<Element>>, Vec<Arc<Element>>) = kf.elements.drain(..).partition(|e| sel.contains(&e.id));
            if l == top {
                rest.insert(first_pos.min(rest.len()), Arc::new(instance.clone()));
            }
            kf.elements = rest;
            Ok(picked)
        })?;
        let children: Vec<Arc<Element>> = picked
            .into_iter()
            .map(|e| {
                let mut e = Arc::unwrap_or_clone(e);
                e.transform = to_local.mul(&e.transform);
                Arc::new(e)
            })
            .collect();
        let id = if inner.is_empty() { first_layer } else { LayerId(s.doc.alloc_id()) };
        let mut layer = Layer::new(id, if single { "Layer 1".to_string() } else { source });
        if let Some(k) = layer.keyframe_mut(0) {
            k.elements = children;
        }
        inner.push(Arc::new(layer));
    }
    s.doc.library.push(Arc::new(Symbol { id: sym_id, name: name.clone(), kind, linkage: None, scale_grid_kept: None, scale_grid: None, timeline: Timeline { layers: inner } }));
    if folder.is_some() {
        s.doc.place_entry(LibraryEntry::Item(LibraryItem::Symbol(sym_id)), folder);
    }
    s.layer = top;
    s.select_only([inst_id]);
    Ok(json!({"symbol": sym_id.0, "instance": inst_id.0, "name": name, "layer": top.0, "folder": folder.map(|f| f.0)}))
}

fn break_apart(s: &mut Session, _: &Value) -> Result<Value> {
    let sel = s.selection.on(s.layer);
    if sel.is_empty() {
        return Err(EngineError::State("nothing is selected".into()));
    }
    let library = s.doc.library.clone();
    let mut fresh_ids = Vec::new();
    let kf_elements: Vec<Arc<Element>> = s.editable_keyframe()?.elements.clone();
    let mut out = Vec::new();
    // Drawing objects (Object Drawing) break apart into raw shapes, which then merge with the
    // raw shapes they touch, as in Animate (#253).
    let mut objects = Vec::new();
    for e in kf_elements {
        if let ElementKind::Shape(sh) = &e.kind
            && !sh.merge
            && sel.contains(&e.id)
        {
            let mut c = (*e).clone();
            if let ElementKind::Shape(sh) = &mut c.kind {
                sh.merge = true;
            }
            objects.push(c);
            continue;
        }
        // Text breaks apart into a shape with the same outlines.
        if let ElementKind::Text(t) = &e.kind
            && sel.contains(&e.id)
        {
            let mut c = (*e).clone();
            c.id = ElementId(s.doc.alloc_id());
            c.kind = ElementKind::Shape(crate::cmds_text::to_shape(t));
            fresh_ids.push(c.id);
            out.push(Arc::new(c));
            continue;
        }
        // A group breaks apart into its members (Animate's Ctrl+B on a group ungroups it).
        if let ElementKind::Group { .. } = &e.kind
            && sel.contains(&e.id)
        {
            for child in e.kind.group_members() {
                let mut c = (*child).clone();
                c.id = ElementId(s.doc.alloc_id());
                c.transform = e.transform.mul(&child.transform);
                c.color = e.color.nested(&child.color);
                // Its raw shapes merge with the drawing they now stand in, as drawing objects do.
                if crate::merge::mergeable(&c).is_some() {
                    objects.push(c);
                    continue;
                }
                fresh_ids.push(c.id);
                out.push(Arc::new(c));
            }
            continue;
        }
        let ElementKind::Symbol { symbol, .. } = &e.kind else {
            out.push(e);
            continue;
        };
        if !sel.contains(&e.id) {
            out.push(e);
            continue;
        }
        let Some(sym) = library.iter().find(|x| x.id == *symbol) else {
            out.push(e);
            continue;
        };
        for layer in sym.timeline.layers.iter().rev() {
            for child in layer.keyframes.first().map(|k| k.elements.as_slice()).unwrap_or(&[]) {
                let mut c = (**child).clone();
                c.id = ElementId(s.doc.alloc_id());
                c.transform = e.transform.mul(&child.transform);
                c.color = e.color.nested(&child.color);
                fresh_ids.push(c.id);
                out.push(Arc::new(c));
            }
        }
    }
    if fresh_ids.is_empty() && objects.is_empty() {
        return Err(EngineError::State("the selection has no symbol instances, groups, text or drawing objects to break apart".into()));
    }
    s.editable_keyframe()?.elements = out;
    if !objects.is_empty() {
        fresh_ids.extend(crate::merge::merge_in(s, objects)?);
    }
    s.select_only(fresh_ids.iter().copied());
    Ok(json!({"created": fresh_ids.len()}))
}

/// Most items one `library.place` takes.
const MAX_PLACE: usize = 1_000;

/// Places Library items at `x`/`y` (default: the stage centre), as a drop from the Library
/// does (ADR 0053): an instance of each symbol, a rectangle of each bitmap's size showing it,
/// in the active layer's keyframe at the playhead, all selected; a sound goes onto that
/// keyframe. One command, so one undo step however many items.
pub(crate) fn place(s: &mut Session, p: &Value) -> Result<Value> {
    let items: Vec<LibraryItem> = match p.get("items") {
        Some(list) => {
            let list = list.as_array().ok_or_else(|| EngineError::Param("param 'items' must be a list of {\"symbol\" | \"bitmap\" | \"sound\": id}".into()))?;
            if list.len() > MAX_PLACE {
                return Err(EngineError::Param(format!("at most {MAX_PLACE} items at once")));
            }
            let mut out = Vec::with_capacity(list.len());
            for v in list {
                match crate::cmds_library::entry_of(v)? {
                    LibraryEntry::Item(i) => out.push(i),
                    LibraryEntry::Folder(_) => return Err(EngineError::Param("a folder can't be placed; name the items in it".into())),
                }
            }
            out
        }
        None => match (opt_u64(p, "symbol")?, opt_u64(p, "bitmap")?, opt_u64(p, "sound")?) {
            (Some(i), None, None) => vec![LibraryItem::Symbol(SymbolId(i))],
            (None, Some(i), None) => vec![LibraryItem::Bitmap(anicraft_doc::BitmapId(i))],
            (None, None, Some(i)) => vec![LibraryItem::Sound(anicraft_doc::SoundId(i))],
            _ => return Err(EngineError::Param("name one item ('symbol', 'bitmap' or 'sound'), or a list in 'items'".into())),
        },
    };
    if items.is_empty() {
        return Err(EngineError::Param("nothing to place".into()));
    }
    let mut sound = None;
    for &i in &items {
        if !s.doc.has_item(i) {
            return Err(EngineError::State(match i {
                LibraryItem::Symbol(id) => format!("no symbol with id {}", id.0),
                LibraryItem::Bitmap(id) => format!("no bitmap with id {}", id.0),
                LibraryItem::Sound(id) => format!("no sound with id {} in the library", id.0),
            }));
        }
        match i {
            LibraryItem::Symbol(id) if crate::editing::would_nest(s, id) => return Err(EngineError::State("a symbol can't be placed inside itself".into())),
            LibraryItem::Sound(id) => {
                if sound.is_some_and(|x| x != id) {
                    return Err(EngineError::State("a keyframe holds one sound; place sounds one at a time, on different keyframes or layers".into()));
                }
                sound = Some(id);
            }
            _ => {}
        }
    }
    let x = opt_f32(p, "x")?.unwrap_or(s.doc.stage.width as f32 / 2.0);
    let y = opt_f32(p, "y")?.unwrap_or(s.doc.stage.height as f32 / 2.0);
    if !(x.is_finite() && y.is_finite()) || x.abs() > 1.0e6 || y.abs() > 1.0e6 {
        return Err(EngineError::Param("'x' and 'y' must be within a million pixels of the origin".into()));
    }
    let mut placed = Vec::new();
    for &i in &items {
        match i {
            LibraryItem::Symbol(id) => placed.push(place_symbol(s, id, x, y)?),
            LibraryItem::Bitmap(id) => placed.push(place_bitmap(s, id, x, y)?),
            LibraryItem::Sound(_) => {}
        }
    }
    let mut out = json!({"element": placed.first().map(|e| e.0), "elements": placed.iter().map(|e| e.0).collect::<Vec<_>>()});
    if let Some(id) = sound {
        if s.active_layer().is_some_and(|l| l.locked) || s.timeline().locked_by_folder(s.layer) {
            return Err(EngineError::State("the active layer is locked; unlock it to put a sound on it".into()));
        }
        out["sound"] = crate::cmds_sound::place(s, &json!({"sound": id.0, "extend": true}))?;
    }
    if !placed.is_empty() {
        s.select_only(placed.iter().copied());
    }
    Ok(out)
}

fn place_symbol(s: &mut Session, sym: SymbolId, x: f32, y: f32) -> Result<ElementId> {
    let id = ElementId(s.doc.alloc_id());
    let el = Element {
        id,
        name: String::new(),
        transform: Affine::translate(x, y),
        color: ColorEffect::default(),
        kind: ElementKind::Symbol { symbol: sym, loop_mode: LoopMode::Loop, first_frame: 0, blend: anicraft_doc::BlendMode::Normal, filters: Vec::new(), events: Vec::new() },
    };
    s.editable_keyframe()?.elements.push(Arc::new(el));
    Ok(id)
}

fn place_bitmap(s: &mut Session, bitmap: anicraft_doc::BitmapId, x: f32, y: f32) -> Result<ElementId> {
    let (name, w, h) = match s.doc.bitmap(bitmap) {
        Some(b) => (b.name.clone(), b.width as f32, b.height as f32),
        None => return Err(EngineError::State(format!("no bitmap with id {}", bitmap.0))),
    };
    let id = ElementId(s.doc.alloc_id());
    let el = Element {
        id,
        name,
        transform: Affine::translate(x - w / 2.0, y - h / 2.0),
        color: ColorEffect::default(),
        kind: ElementKind::Shape(Shape {
            path: Arc::new(Path::rect(0.0, 0.0, w, h)),
            fill: Some(Paint::Bitmap { bitmap, repeat: false, matrix: Affine::IDENTITY }),
            stroke: None,
            fill_rule: anicraft_geom::FillRule::NonZero,
            merge: false,
            warp: None,
        }),
    };
    s.editable_keyframe()?.elements.push(Arc::new(el));
    Ok(id)
}
