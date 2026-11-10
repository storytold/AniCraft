//! Drawing, selection, element and symbol commands.

use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::{ColorEffect, Element, ElementId, ElementKind, Layer, LayerId, LoopMode, Paint, Shape, Stroke, Symbol, SymbolId, SymbolKind, Timeline};
use anicraft_geom::{Affine, Decomposed, Path, Vec2};
use serde_json::{Value, json};
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c("draw.rect", "Rectangle", "", None, "{x, y, w, h, radius?, fill?, stroke?, stroke_width?, profile?, object?}", true, draw_rect as crate::RunFn),
        c("draw.ellipse", "Oval", "", None, "{x, y, w, h, fill?, stroke?, stroke_width?, profile?, object?}", true, draw_ellipse),
        c("draw.star", "PolyStar", "", None, "{cx, cy, outer, inner?, points?, fill?, stroke?, profile?, object?}", true, draw_star),
        c("draw.line", "Line", "", None, "{x0, y0, x1, y1, stroke?, stroke_width?, profile? (a width profile name or [[t, left, right], ..]), object? (true: a drawing object; default: Object Drawing, off: it merges with the raw shapes it touches)}", true, draw_line),
        c("draw.object_drawing", "Object Drawing", "", Some("J"), "{on?: bool} (default: toggle)", false, object_drawing),
        c("edit.select_all", "Select All", "Edit", Some("Ctrl+A"), "{}", false, select_all),
        c("edit.deselect", "Deselect All", "Edit", Some("Ctrl+Shift+A"), "{}", false, |s, _| {
            s.selection.clear();
            Ok(json!({"selected": 0}))
        }),
        c("edit.delete", "Clear", "Edit", Some("Delete"), "{}", true, delete),
        c("edit.duplicate", "Duplicate", "Edit", Some("Ctrl+D"), "{dx?, dy?}", true, duplicate),
        c("element.select", "Select Elements", "", None, "{ids: [..], add?: bool}", false, select),
        c("element.pick", "Select at Point", "", None, "{x, y, add?: bool}", false, pick),
        c("element.translate", "Move", "", None, "{dx, dy}", true, translate),
        c("element.set_transform", "Transform", "", None, "{x?, y?, sx?, sy?, rotation? (degrees)}", true, set_transform),
        c("element.set_fill", "Fill Colour", "", None, "{color: \"#rrggbb\" | null}", true, set_fill),
        c("element.set_stroke", "Stroke", "", None, "{color?: \"#rrggbb\" | null, width?, profile?: uniform|taper_both|taper_start|taper_end|bulge|pinch | [[t, left, right], ..]}", true, set_stroke),
        c("element.set_alpha", "Alpha", "", None, "{alpha: 0..1}", true, set_alpha),
        c("element.set_blend", "Blending", "", None, "{mode: normal|layer|darken|multiply|lighten|screen|overlay|hardlight|add|subtract|difference|invert|alpha|erase} on the selected movie clip and button instances", true, set_blend),
        c("element.set_filters", "Filters", "", None, "{filters: [{type: blur|drop_shadow|glow|bevel|gradient_glow|gradient_bevel|adjust_color, …values}]} replaces the selected movie clip and button instances' filters (in order; [] removes them). Bevel: shadow_color, highlight_color, side: inner|outer|full, knockout; gradient filters: stops: [{offset 0..1, color \"#rrggbbaa\"}] (Gradient Glow's start and Gradient Bevel's middle stay transparent)", true, set_filters),
        c("element.set_loop", "Looping", "", None, "{loop?: loop|play_once|single_frame, first_frame?}", true, set_loop),
        c("symbol.set_kind", "Symbol Type", "", None, "{kind: graphic|movie_clip|button, symbol?: id (default: the selected instance's)}", true, set_symbol_kind),
        c("element.rename", "Instance Name", "", None, "{name}", true, rename),
        c("arrange.bring_to_front", "Bring to Front", "Modify/Arrange", Some("Ctrl+Shift+Up"), "{}", true, |s, _| arrange(s, true)),
        c("arrange.bring_forward", "Bring Forward", "Modify/Arrange", Some("Ctrl+Up"), "{}", true, |s, _| arrange_step(s, true)),
        c("arrange.send_backward", "Send Backward", "Modify/Arrange", Some("Ctrl+Down"), "{}", true, |s, _| arrange_step(s, false)),
        c("arrange.send_to_back", "Send to Back", "Modify/Arrange", Some("Ctrl+Shift+Down"), "{}", true, |s, _| arrange(s, false)),
        c("modify.convert_to_symbol", "Convert to Symbol…", "Modify", Some("F8"), "{name?}", true, convert_to_symbol),
        c("modify.break_apart", "Break Apart", "Modify", Some("Ctrl+B"), "{}", true, break_apart),
        c("library.place", "Place Instance", "", None, "{symbol | bitmap, x?, y?}", true, place),
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
    let stroke = if p.get("stroke").is_some_and(Value::is_null) || width <= 0.0 {
        None
    } else {
        match opt_color(p, "stroke")? {
            Some(c) => Some(Stroke::new(c, width)),
            None => (s.style.use_stroke || !closed).then_some(Stroke::new(s.style.stroke, width)),
        }
    }
    .map(|st| st.with_profile(profile));
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
    // A gradient or bitmap from the fill swatch is in stage space: it stays where it is on the
    // stage (Animate's Lock Fill after the Eyedropper).
    let fill = fill.map(|f| f.transformed(&Affine::translate(-origin.x, -origin.y)));
    let merge = !object && crate::merge::can_merge(fill.as_ref(), stroke.as_ref());
    let id = ElementId(s.doc.alloc_id());
    let el = Element {
        id,
        name: String::new(),
        transform: Affine::translate(origin.x, origin.y),
        color: ColorEffect::default(),
        kind: ElementKind::Shape(Shape { path: Arc::new(local), fill, stroke, fill_rule: Default::default(), merge }),
    };
    if merge {
        s.selection = crate::merge::merge_in(s, vec![el])?;
        let ids: Vec<u64> = s.selection.iter().map(|e| e.0).collect();
        return Ok(json!({"element": ids.first(), "elements": ids, "kind": name, "merged": true}));
    }
    let kf = s.editable_keyframe()?;
    if kf.elements.len() >= 100_000 {
        return Err(EngineError::State("too many elements on this keyframe".into()));
    }
    kf.elements.push(Arc::new(el));
    s.selection = vec![id];
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
    let inner = opt_f32(p, "inner")?.unwrap_or(outer * 0.5).clamp(0.0, MAX_EXTENT);
    let points = opt_u32(p, "points")?.unwrap_or(5);
    if !(3..=64).contains(&points) {
        return Err(EngineError::Param("param 'points' must be between 3 and 64".into()));
    }
    let (fill, stroke) = style(s, p, true)?;
    add_shape(s, "star", Path::star(0.0, 0.0, outer, inner, points), Vec2::new(cx, cy), fill, stroke, object_param(s, p)?)
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

fn object_drawing(s: &mut Session, p: &Value) -> Result<Value> {
    s.style.object_drawing = opt_bool(p, "on")?.unwrap_or(!s.style.object_drawing);
    Ok(json!({"object_drawing": s.style.object_drawing}))
}

// ----- selection --------------------------------------------------------------------------

fn select_all(s: &mut Session, _: &Value) -> Result<Value> {
    s.selection = s.visible_elements().iter().map(|e| e.id).collect();
    Ok(json!({"selected": s.selection.len()}))
}

fn select(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = opt_u64_list(p, "ids")?.ok_or_else(|| EngineError::Param("param 'ids' is required".into()))?;
    let add = opt_bool(p, "add")?.unwrap_or(false);
    let present: Vec<ElementId> = s.visible_elements().iter().map(|e| e.id).collect();
    if !add {
        s.selection.clear();
    }
    for id in ids.into_iter().map(ElementId) {
        if !present.contains(&id) {
            return Err(EngineError::State(format!("element {} is not on the active layer at this frame", id.0)));
        }
        if !s.selection.contains(&id) {
            s.selection.push(id);
        }
    }
    Ok(json!({"selected": s.selection.iter().map(|e| e.0).collect::<Vec<_>>()}))
}

fn pick(s: &mut Session, p: &Value) -> Result<Value> {
    let pt = Vec2::new(f32_req(p, "x")?, f32_req(p, "y")?);
    let add = opt_bool(p, "add")?.unwrap_or(false);
    match s.hit_test(pt, 2.0) {
        Some((layer, el)) => {
            if layer != s.layer {
                s.layer = layer;
                s.selection.clear();
            }
            if add {
                if let Some(i) = s.selection.iter().position(|e| *e == el) {
                    s.selection.remove(i);
                } else {
                    s.selection.push(el);
                }
            } else if !s.selection.contains(&el) {
                s.selection = vec![el];
            }
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
    let (dx, dy) = (f32_req(p, "dx")?, f32_req(p, "dy")?);
    let els = s.selected_elements_mut()?;
    let n = els.len();
    for e in els {
        e.transform = Affine::translate(dx, dy).mul(&e.transform);
    }
    Ok(json!({"moved": n}))
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
            ElementKind::Symbol { .. } | ElementKind::Text(_) | ElementKind::Camera => None,
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

fn set_stroke(s: &mut Session, p: &Value) -> Result<Value> {
    let remove = p.get("color").is_some_and(Value::is_null);
    let color = opt_color(p, "color")?;
    let width = opt_f32(p, "width")?;
    if width.is_some_and(|w| !(0.0..=1000.0).contains(&w)) {
        return Err(EngineError::Param("param 'width' must be in 0..=1000".into()));
    }
    let profile = opt_profile(p, "profile")?;
    let default = s.style.clone();
    for sh in shapes_mut(s)? {
        if remove {
            sh.stroke = None;
            continue;
        }
        let mut st = sh.stroke.take().unwrap_or_else(|| Stroke::new(default.stroke, default.stroke_width).with_profile(default.profile()));
        if let Some(c) = color {
            st.color = c;
        }
        if let Some(w) = width {
            st.width = w;
        }
        if let Some(pr) = &profile {
            st = st.with_profile(pr.clone());
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
    for e in s.selected_elements_mut()? {
        e.color.alpha = a;
    }
    Ok(json!({"alpha": a}))
}

/// Most filters on one instance.
const MAX_FILTERS: usize = 16;

/// Filters for the selected movie clip and button instances (Animate offers them on those only).
fn set_filters(s: &mut Session, p: &Value) -> Result<Value> {
    let mut list = p.get("filters").cloned().ok_or_else(|| EngineError::Param("param 'filters' is required (a list; [] removes them)".into()))?;
    // Colours may be written "#rrggbb" (or "#rrggbbaa"), as everywhere in commands: the filter's
    // own and its ramp stops'.
    fn hex_colors(v: &mut Value, keys: &[&str]) -> Result<()> {
        for k in keys {
            if v.get(*k).is_some_and(Value::is_string) {
                let c = opt_color(v, k)?.unwrap_or(anicraft_doc::Rgba::BLACK);
                v[*k] = json!({"r": c.r, "g": c.g, "b": c.b, "a": c.a});
            }
        }
        Ok(())
    }
    for f in list.as_array_mut().into_iter().flatten() {
        hex_colors(f, &["color", "shadow_color", "highlight_color"])?;
        if let Some(stops) = f.get_mut("stops").and_then(Value::as_array_mut) {
            if stops.len() > anicraft_doc::MAX_RAMP_STOPS {
                return Err(EngineError::Param(format!("a ramp has at most {} stops", anicraft_doc::MAX_RAMP_STOPS)));
            }
            for st in stops.iter_mut() {
                hex_colors(st, &["color"])?;
            }
        }
    }
    let filters: Vec<anicraft_doc::Filter> = serde_json::from_value(list).map_err(|e| EngineError::Param(format!("'filters' must be a list of {{type: blur|drop_shadow|glow|bevel|gradient_glow|gradient_bevel|adjust_color, …}}: {e}")))?;
    if filters.len() > MAX_FILTERS {
        return Err(EngineError::Param(format!("at most {MAX_FILTERS} filters per instance")));
    }
    let filters: Vec<anicraft_doc::Filter> = filters.iter().map(anicraft_doc::Filter::sanitized).collect();
    let kinds: std::collections::HashMap<SymbolId, SymbolKind> = s.doc.library.iter().map(|x| (x.id, x.kind)).collect();
    let (mut set, mut graphics) = (0, 0);
    for e in s.selected_elements_mut()? {
        if let ElementKind::Symbol { symbol, filters: f, .. } = &mut e.kind {
            if kinds.get(symbol) == Some(&SymbolKind::Graphic) {
                graphics += 1;
                continue;
            }
            f.clone_from(&filters);
            set += 1;
        }
    }
    if set == 0 {
        return Err(EngineError::State(if graphics > 0 {
            "filters apply to movie clip and button instances, as in Animate; set the symbol's Type to Movie Clip first".into()
        } else {
            "select a movie clip or button instance".into()
        }));
    }
    Ok(json!({"filters": filters.len(), "set": set}))
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
    for e in s.selected_elements_mut()? {
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
    let kind = match str_req(p, "kind")? {
        "graphic" => SymbolKind::Graphic,
        "movie_clip" | "movie clip" => SymbolKind::MovieClip,
        "button" => SymbolKind::Button,
        other => return Err(EngineError::Param(format!("unknown symbol kind '{other}' (graphic, movie_clip or button)"))),
    };
    let id = match opt_u64(p, "symbol")? {
        Some(id) => SymbolId(id),
        None => {
            // The symbol of the selected instance.
            let sel = s.selection.first().copied().ok_or_else(|| EngineError::State("select a symbol instance, or pass 'symbol'".into()))?;
            s.visible_elements()
                .iter()
                .find(|e| e.id == sel)
                .and_then(|e| match e.kind {
                    ElementKind::Symbol { symbol, .. } => Some(symbol),
                    ElementKind::Shape(_) | ElementKind::Text(_) | ElementKind::Camera => None,
                })
                .ok_or_else(|| EngineError::State("the selection is not a symbol instance".into()))?
        }
    };
    let sym = s.doc.library.iter_mut().find(|x| x.id == id).ok_or_else(|| EngineError::State(format!("no symbol with id {}", id.0)))?;
    std::sync::Arc::make_mut(sym).kind = kind;
    Ok(json!({"symbol": id.0, "kind": format!("{kind:?}")}))
}

fn rename(s: &mut Session, p: &Value) -> Result<Value> {
    let name: String = str_req(p, "name")?.chars().take(128).collect();
    for e in s.selected_elements_mut()? {
        e.name = name.clone();
    }
    Ok(json!({"name": name}))
}

pub(crate) fn delete(s: &mut Session, _: &Value) -> Result<Value> {
    let sel = s.selection.clone();
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
    let sel = s.selection.clone();
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
    s.selection = new_ids.clone();
    Ok(json!({"created": new_ids.iter().map(|e| e.0).collect::<Vec<_>>()}))
}

fn arrange(s: &mut Session, front: bool) -> Result<Value> {
    let sel = s.selection.clone();
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
    let sel = s.selection.clone();
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

pub(crate) fn convert_to_symbol(s: &mut Session, p: &Value) -> Result<Value> {
    let sel = s.selection.clone();
    if sel.is_empty() {
        return Err(EngineError::State("select something to convert".into()));
    }
    let n = s.doc.library.len() + 1;
    let name: String = opt_str(p, "name")?.map(|x| x.chars().take(128).collect()).unwrap_or_else(|| format!("Symbol {n}"));
    let bounds = s.selection_bounds();
    let center = if bounds.is_empty() { Vec2::ZERO } else { bounds.center() };
    let sym_id = SymbolId(s.doc.alloc_id());
    let layer_id = LayerId(s.doc.alloc_id());
    let inst_id = ElementId(s.doc.alloc_id());
    let kf = s.editable_keyframe()?;
    let first_pos = kf.elements.iter().position(|e| sel.contains(&e.id)).unwrap_or(kf.elements.len());
    let (picked, rest): (Vec<Arc<Element>>, Vec<Arc<Element>>) = kf.elements.drain(..).partition(|e| sel.contains(&e.id));
    // Children are stored relative to the registration point (the selection centre).
    let to_local = Affine::translate(-center.x, -center.y);
    let children: Vec<Arc<Element>> = picked
        .into_iter()
        .map(|e| {
            let mut e = Arc::unwrap_or_clone(e);
            e.transform = to_local.mul(&e.transform);
            Arc::new(e)
        })
        .collect();
    let instance = Element {
        id: inst_id,
        name: String::new(),
        transform: Affine::translate(center.x, center.y),
        color: ColorEffect::default(),
        kind: ElementKind::Symbol { symbol: sym_id, loop_mode: LoopMode::Loop, first_frame: 0, blend: anicraft_doc::BlendMode::Normal, filters: Vec::new(), events: Vec::new() },
    };
    let mut elements = rest;
    elements.insert(first_pos.min(elements.len()), Arc::new(instance));
    kf.elements = elements;
    let mut layer = Layer::new(layer_id, "Layer 1");
    if let Some(k) = layer.keyframe_mut(0) {
        k.elements = children;
    }
    s.doc.library.push(Arc::new(Symbol { id: sym_id, name: name.clone(), kind: SymbolKind::Graphic, linkage: None, timeline: Timeline { layers: vec![Arc::new(layer)] } }));
    s.selection = vec![inst_id];
    Ok(json!({"symbol": sym_id.0, "instance": inst_id.0, "name": name}))
}

fn break_apart(s: &mut Session, _: &Value) -> Result<Value> {
    let sel = s.selection.clone();
    if sel.is_empty() {
        return Err(EngineError::State("nothing is selected".into()));
    }
    let library = s.doc.library.clone();
    let mut fresh_ids = Vec::new();
    let kf_elements: Vec<Arc<Element>> = s.editable_keyframe()?.elements.clone();
    let mut out = Vec::new();
    for e in kf_elements {
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
                c.color = e.color.concat(&child.color);
                fresh_ids.push(c.id);
                out.push(Arc::new(c));
            }
        }
    }
    if fresh_ids.is_empty() {
        return Err(EngineError::State("the selection has no symbol instances or text to break apart".into()));
    }
    s.editable_keyframe()?.elements = out;
    s.selection = fresh_ids.clone();
    Ok(json!({"created": fresh_ids.len()}))
}

/// Places a Library item centred on `x`/`y` (default: the stage centre): an instance of
/// `symbol`, or a rectangle of `bitmap`'s size showing it, in the current keyframe, selected.
fn place(s: &mut Session, p: &Value) -> Result<Value> {
    if let Some(bitmap) = opt_u64(p, "bitmap")? {
        return place_bitmap(s, p, anicraft_doc::BitmapId(bitmap));
    }
    let sym = SymbolId(opt_u64(p, "symbol")?.ok_or_else(|| EngineError::Param("param 'symbol' or 'bitmap' is required".into()))?);
    if s.doc.symbol(sym).is_none() {
        return Err(EngineError::State(format!("no symbol with id {}", sym.0)));
    }
    let x = opt_f32(p, "x")?.unwrap_or(s.doc.stage.width as f32 / 2.0);
    let y = opt_f32(p, "y")?.unwrap_or(s.doc.stage.height as f32 / 2.0);
    let id = ElementId(s.doc.alloc_id());
    let el = Element {
        id,
        name: String::new(),
        transform: Affine::translate(x, y),
        color: ColorEffect::default(),
        kind: ElementKind::Symbol { symbol: sym, loop_mode: LoopMode::Loop, first_frame: 0, blend: anicraft_doc::BlendMode::Normal, filters: Vec::new(), events: Vec::new() },
    };
    s.editable_keyframe()?.elements.push(Arc::new(el));
    s.selection = vec![id];
    Ok(json!({"element": id.0}))
}

fn place_bitmap(s: &mut Session, p: &Value, bitmap: anicraft_doc::BitmapId) -> Result<Value> {
    let (name, w, h) = match s.doc.bitmap(bitmap) {
        Some(b) => (b.name.clone(), b.width as f32, b.height as f32),
        None => return Err(EngineError::State(format!("no bitmap with id {}", bitmap.0))),
    };
    let x = opt_f32(p, "x")?.unwrap_or(s.doc.stage.width as f32 / 2.0);
    let y = opt_f32(p, "y")?.unwrap_or(s.doc.stage.height as f32 / 2.0);
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
        }),
    };
    s.editable_keyframe()?.elements.push(Arc::new(el));
    s.selection = vec![id];
    Ok(json!({"element": id.0}))
}
