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
        c("draw.rect", "Rectangle", "", None, "{x, y, w, h, radius?, fill?, stroke?, stroke_width?}", true, draw_rect as crate::RunFn),
        c("draw.ellipse", "Oval", "", None, "{x, y, w, h, fill?, stroke?, stroke_width?}", true, draw_ellipse),
        c("draw.star", "PolyStar", "", None, "{cx, cy, outer, inner?, points?, fill?, stroke?}", true, draw_star),
        c("draw.line", "Line", "", None, "{x0, y0, x1, y1, stroke?, stroke_width?}", true, draw_line),
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
        c("element.set_stroke", "Stroke", "", None, "{color?: \"#rrggbb\" | null, width?}", true, set_stroke),
        c("element.set_alpha", "Alpha", "", None, "{alpha: 0..1}", true, set_alpha),
        c("element.rename", "Instance Name", "", None, "{name}", true, rename),
        c("arrange.bring_to_front", "Bring to Front", "Modify/Arrange", Some("Ctrl+Shift+Up"), "{}", true, |s, _| arrange(s, true)),
        c("arrange.bring_forward", "Bring Forward", "Modify/Arrange", Some("Ctrl+Up"), "{}", true, |s, _| arrange_step(s, true)),
        c("arrange.send_backward", "Send Backward", "Modify/Arrange", Some("Ctrl+Down"), "{}", true, |s, _| arrange_step(s, false)),
        c("arrange.send_to_back", "Send to Back", "Modify/Arrange", Some("Ctrl+Shift+Down"), "{}", true, |s, _| arrange(s, false)),
        c("modify.convert_to_symbol", "Convert to Symbol…", "Modify", Some("F8"), "{name?}", true, convert_to_symbol),
        c("modify.break_apart", "Break Apart", "Modify", Some("Ctrl+B"), "{}", true, break_apart),
        c("library.place", "Place Instance", "", None, "{symbol, x?, y?}", true, place),
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
            None => s.style.use_fill.then_some(Paint::Solid(s.style.fill)),
        }
    };
    let width = match opt_f32(p, "stroke_width")? {
        Some(w) if (0.0..=1000.0).contains(&w) => w,
        Some(_) => return Err(EngineError::Param("param 'stroke_width' must be in 0..=1000".into())),
        None => s.style.stroke_width,
    };
    let stroke = if p.get("stroke").is_some_and(Value::is_null) || width <= 0.0 {
        None
    } else {
        match opt_color(p, "stroke")? {
            Some(c) => Some(Stroke { color: c, width }),
            None => (s.style.use_stroke || !closed).then_some(Stroke { color: s.style.stroke, width }),
        }
    };
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

/// Adds a shape whose local origin is its centre, placed with a translation.
pub(crate) fn add_shape(s: &mut Session, name: &str, local: Path, origin: Vec2, fill: Option<Paint>, stroke: Option<Stroke>) -> Result<Value> {
    let id = ElementId(s.doc.alloc_id());
    let el = Element {
        id,
        name: String::new(),
        transform: Affine::translate(origin.x, origin.y),
        color: ColorEffect::default(),
        kind: ElementKind::Shape(Shape { path: Arc::new(local), fill, stroke, fill_rule: Default::default() }),
    };
    let kf = s.editable_keyframe()?;
    if kf.elements.len() >= 100_000 {
        return Err(EngineError::State("too many elements on this keyframe".into()));
    }
    kf.elements.push(Arc::new(el));
    s.selection = vec![id];
    Ok(json!({"element": id.0, "kind": name}))
}

fn draw_rect(s: &mut Session, p: &Value) -> Result<Value> {
    let (x, y, w, h) = rect_params(p)?;
    let r = opt_f32(p, "radius")?.unwrap_or(0.0).max(0.0);
    let (fill, stroke) = style(s, p, true)?;
    add_shape(s, "rect", Path::rounded_rect(-w / 2.0, -h / 2.0, w, h, r), Vec2::new(x + w / 2.0, y + h / 2.0), fill, stroke)
}

fn draw_ellipse(s: &mut Session, p: &Value) -> Result<Value> {
    let (x, y, w, h) = rect_params(p)?;
    let (fill, stroke) = style(s, p, true)?;
    add_shape(s, "ellipse", Path::ellipse(0.0, 0.0, w / 2.0, h / 2.0), Vec2::new(x + w / 2.0, y + h / 2.0), fill, stroke)
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
    add_shape(s, "star", Path::star(0.0, 0.0, outer, inner, points), Vec2::new(cx, cy), fill, stroke)
}

fn draw_line(s: &mut Session, p: &Value) -> Result<Value> {
    let a = Vec2::new(f32_req(p, "x0")?, f32_req(p, "y0")?);
    let b = Vec2::new(f32_req(p, "x1")?, f32_req(p, "y1")?);
    if a.distance(b) < 0.5 {
        return Err(EngineError::Param("line is too short".into()));
    }
    let (_, stroke) = style(s, p, false)?;
    let c = (a + b) * 0.5;
    add_shape(s, "line", Path::line(a - c, b - c), c, None, stroke)
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
            ElementKind::Symbol { .. } => None,
        })
        .collect();
    if shapes.is_empty() {
        return Err(EngineError::State("the selection has no shapes (symbol instances use alpha/tint)".into()));
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
    let default = s.style;
    for sh in shapes_mut(s)? {
        if remove {
            sh.stroke = None;
            continue;
        }
        let mut st = sh.stroke.unwrap_or(Stroke { color: default.stroke, width: default.stroke_width });
        if let Some(c) = color {
            st.color = c;
        }
        if let Some(w) = width {
            st.width = w;
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

fn convert_to_symbol(s: &mut Session, p: &Value) -> Result<Value> {
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
        kind: ElementKind::Symbol { symbol: sym_id, loop_mode: LoopMode::Loop, first_frame: 0 },
    };
    let mut elements = rest;
    elements.insert(first_pos.min(elements.len()), Arc::new(instance));
    kf.elements = elements;
    let mut layer = Layer::new(layer_id, "Layer 1");
    if let Some(k) = layer.keyframe_mut(0) {
        k.elements = children;
    }
    s.doc.library.push(Arc::new(Symbol { id: sym_id, name: name.clone(), kind: SymbolKind::Graphic, timeline: Timeline { layers: vec![Arc::new(layer)] } }));
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
        return Err(EngineError::State("the selection has no symbol instances to break apart".into()));
    }
    s.editable_keyframe()?.elements = out;
    s.selection = fresh_ids.clone();
    Ok(json!({"created": fresh_ids.len()}))
}

fn place(s: &mut Session, p: &Value) -> Result<Value> {
    let sym = SymbolId(opt_u64(p, "symbol")?.ok_or_else(|| EngineError::Param("param 'symbol' is required".into()))?);
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
        kind: ElementKind::Symbol { symbol: sym, loop_mode: LoopMode::Loop, first_frame: 0 },
    };
    s.editable_keyframe()?.elements.push(Arc::new(el));
    s.selection = vec![id];
    Ok(json!({"element": id.0}))
}
