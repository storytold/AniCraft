//! Clipboard (cut/copy/paste), transforms (free transform, rotate 90°, flip) and alignment.

use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::{Element, ElementId, ElementKind, Symbol, SymbolId};
use anicraft_geom::{Affine, Rect, Vec2};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c("edit.cut", "Cut", "Edit", Some("Ctrl+X"), "{}", true, cut as crate::RunFn),
        c("edit.copy", "Copy", "Edit", Some("Ctrl+C"), "{}", false, copy),
        c("edit.paste", "Paste in Center", "Edit", Some("Ctrl+V"), "{x?, y?} (centre of the pasted artwork; default: offset 10 px)", true, paste),
        c("edit.paste_in_place", "Paste in Place", "Edit", Some("Ctrl+Shift+V"), "{}", true, paste_in_place),
        c("modify.transform", "Transform", "", None, "{a, b, c, d, e, f} affine applied in stage space (free transform)", true, transform),
        c("modify.rotate_cw", "Rotate 90° CW", "Modify/Transform", Some("Ctrl+Shift+9"), "{}", true, |s, _| about_center(s, Affine::rotate(std::f32::consts::FRAC_PI_2))),
        c("modify.rotate_ccw", "Rotate 90° CCW", "Modify/Transform", Some("Ctrl+Shift+7"), "{}", true, |s, _| about_center(s, Affine::rotate(-std::f32::consts::FRAC_PI_2))),
        c("modify.flip_h", "Flip Horizontal", "Modify/Transform", None, "{}", true, |s, _| about_center(s, Affine::scale(-1.0, 1.0))),
        c("modify.flip_v", "Flip Vertical", "Modify/Transform", None, "{}", true, |s, _| about_center(s, Affine::scale(1.0, -1.0))),
        c("modify.remove_transform", "Remove Transform", "Modify/Transform", Some("Ctrl+Shift+Z"), "{}", true, remove_transform),
        c("align.left", "Left", "Modify/Align", Some("Ctrl+Alt+1"), "{to_stage?}", true, |s, p| align(s, p, Edge::Left)),
        c("align.hcenter", "Horizontal Center", "Modify/Align", Some("Ctrl+Alt+2"), "{to_stage?}", true, |s, p| align(s, p, Edge::HCenter)),
        c("align.right", "Right", "Modify/Align", Some("Ctrl+Alt+3"), "{to_stage?}", true, |s, p| align(s, p, Edge::Right)),
        c("align.top", "Top", "Modify/Align", Some("Ctrl+Alt+4"), "{to_stage?}", true, |s, p| align(s, p, Edge::Top)),
        c("align.vcenter", "Vertical Center", "Modify/Align", Some("Ctrl+Alt+5"), "{to_stage?}", true, |s, p| align(s, p, Edge::VCenter)),
        c("align.bottom", "Bottom", "Modify/Align", Some("Ctrl+Alt+6"), "{to_stage?}", true, |s, p| align(s, p, Edge::Bottom)),
        c("align.distribute_h", "Distribute Horizontal Centers", "Modify/Align", None, "{}", true, |s, _| distribute(s, true)),
        c("align.distribute_v", "Distribute Vertical Centers", "Modify/Align", None, "{}", true, |s, _| distribute(s, false)),
    ]
}

// ----- clipboard --------------------------------------------------------------------------------

/// Copied artwork: elements as they look at the playhead, plus every symbol they need, so a
/// paste into another document is complete.
#[derive(Clone, Default)]
pub struct Clipboard {
    pub elements: Vec<Element>,
    pub symbols: Vec<Arc<Symbol>>,
}

impl Clipboard {
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }
}

fn symbol_refs(e: &Element) -> Option<SymbolId> {
    match &e.kind {
        ElementKind::Symbol { symbol, .. } => Some(*symbol),
        ElementKind::Shape(_) | ElementKind::Text(_) | ElementKind::Camera => None,
    }
}

/// Symbols reachable from `roots` (bounded walk; libraries may be cyclic).
pub(crate) fn collect_symbols(s: &Session, roots: &[Element]) -> Vec<Arc<Symbol>> {
    let mut seen: HashSet<SymbolId> = HashSet::new();
    let mut stack: Vec<SymbolId> = roots.iter().filter_map(symbol_refs).collect();
    let mut out = Vec::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) || seen.len() > 10_000 {
            continue;
        }
        let Some(sym) = s.doc.library.iter().find(|x| x.id == id) else { continue };
        for l in &sym.timeline.layers {
            for k in &l.keyframes {
                stack.extend(k.elements.iter().filter_map(|e| symbol_refs(e)));
            }
        }
        out.push(sym.clone());
    }
    out
}

fn copy(s: &mut Session, _: &Value) -> Result<Value> {
    let sel = s.selection.clone();
    if sel.is_empty() {
        return Err(EngineError::State("nothing is selected".into()));
    }
    // What you see is what you copy: the evaluated state at the playhead (tweens applied).
    let elements: Vec<Element> = s.visible_elements().iter().filter(|e| sel.contains(&e.id)).map(|e| (**e).clone()).collect();
    let symbols = collect_symbols(s, &elements);
    let n = elements.len();
    s.clipboard = Clipboard { elements, symbols };
    Ok(json!({"copied": n}))
}

fn cut(s: &mut Session, p: &Value) -> Result<Value> {
    copy(s, p)?;
    crate::cmds_draw::delete(s, p)?;
    Ok(json!({"cut": s.clipboard.elements.len()}))
}

fn clipboard_bounds(s: &Session) -> Rect {
    s.clipboard.elements.iter().fold(Rect::EMPTY, |r, e| r.union(&clip_element_bounds(s, e)))
}

fn clip_element_bounds(s: &Session, e: &Element) -> Rect {
    // Symbols may come from another document: look them up in the clipboard first.
    let mut doc = s.doc.clone();
    for sym in &s.clipboard.symbols {
        if doc.symbol(sym.id).is_none() {
            doc.library.push(sym.clone());
        }
    }
    e.bounds(&doc)
}

fn paste_with(s: &mut Session, offset: Vec2) -> Result<Value> {
    if s.clipboard.is_empty() {
        return Err(EngineError::State("the clipboard is empty".into()));
    }
    import_symbols(s, &s.clipboard.symbols.clone());
    let mut fresh = Vec::new();
    let mut out = Vec::new();
    for e in s.clipboard.elements.clone() {
        let mut e = e;
        e.id = ElementId(s.doc.alloc_id());
        e.transform = Affine::translate(offset.x, offset.y).mul(&e.transform);
        fresh.push(e.id);
        out.push(Arc::new(e));
    }
    let kf = s.editable_keyframe()?;
    if kf.elements.len() + out.len() > 100_000 {
        return Err(EngineError::State("too many elements on this keyframe".into()));
    }
    kf.elements.extend(out);
    s.selection = fresh.clone();
    Ok(json!({"pasted": fresh.iter().map(|e| e.0).collect::<Vec<_>>()}))
}

/// Adds the symbols this document lacks (from a clipboard, maybe another document's) and
/// moves the id counter past every id they use, so new ids stay unique.
pub(crate) fn import_symbols(s: &mut Session, symbols: &[Arc<Symbol>]) {
    let mut max_id = 0u64;
    for sym in symbols.iter().cloned() {
        if s.doc.symbol(sym.id).is_none() {
            max_id = max_id.max(sym.id.0);
            for l in &sym.timeline.layers {
                max_id = max_id.max(l.id.0);
                for k in &l.keyframes {
                    for e in &k.elements {
                        max_id = max_id.max(e.id.0);
                    }
                }
            }
            s.doc.library.push(sym);
        }
    }
    s.doc.next_id = s.doc.next_id.max(max_id.saturating_add(1));
}

fn paste(s: &mut Session, p: &Value) -> Result<Value> {
    let offset = match (opt_f32(p, "x")?, opt_f32(p, "y")?) {
        (Some(x), Some(y)) => {
            let b = clipboard_bounds(s);
            if b.is_empty() { Vec2::new(10.0, 10.0) } else { Vec2::new(x, y) - b.center() }
        }
        _ => Vec2::new(10.0, 10.0),
    };
    paste_with(s, offset)
}

fn paste_in_place(s: &mut Session, _: &Value) -> Result<Value> {
    paste_with(s, Vec2::ZERO)
}

// ----- transforms -------------------------------------------------------------------------------

/// Applies `m` (stage space) to every selected element: `t' = m · t`.
pub(crate) fn apply(s: &mut Session, m: &Affine) -> Result<usize> {
    if !m.is_finite() || m.determinant().abs() < 1e-8 {
        return Err(EngineError::Param("the transform is degenerate (it would collapse the artwork)".into()));
    }
    let els = s.selected_elements_mut()?;
    let n = els.len();
    for e in els {
        let t = m.mul(&e.transform);
        let d = t.decompose();
        if !t.is_finite() || d.sx.abs() > 10_000.0 || d.sy.abs() > 10_000.0 || d.sx.abs() < 1e-4 || d.sy.abs() < 1e-4 {
            return Err(EngineError::Param("the transform would make an element too large or too small".into()));
        }
        e.transform = t;
    }
    Ok(n)
}

fn transform(s: &mut Session, p: &Value) -> Result<Value> {
    let m = Affine { m: [f32_req(p, "a")?, f32_req(p, "b")?, f32_req(p, "c")?, f32_req(p, "d")?, f32_req(p, "e")?, f32_req(p, "f")?] };
    let n = apply(s, &m)?;
    Ok(json!({"transformed": n}))
}

/// `m` applied about the centre of the selection's bounds.
/// Remove Transform: scale, rotation and skew go back to none; each element keeps its
/// position (registration point).
fn remove_transform(s: &mut Session, _: &Value) -> Result<Value> {
    let els = s.selected_elements_mut()?;
    let n = els.len();
    for e in els {
        let t = e.transform.translation();
        e.transform = Affine::translate(t.x, t.y);
    }
    Ok(json!({"reset": n}))
}

fn about_center(s: &mut Session, m: Affine) -> Result<Value> {
    let b = s.selection_bounds();
    if b.is_empty() {
        return Err(EngineError::State("nothing is selected".into()));
    }
    let c = b.center();
    let full = Affine::translate(c.x, c.y).mul(&m).mul(&Affine::translate(-c.x, -c.y));
    let n = apply(s, &full)?;
    Ok(json!({"transformed": n}))
}

// ----- align ------------------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Edge {
    Left,
    HCenter,
    Right,
    Top,
    VCenter,
    Bottom,
}

fn moves_for(bounds: &[(ElementId, Rect)], target: Rect, edge: Edge) -> Vec<(ElementId, Vec2)> {
    bounds
        .iter()
        .map(|(id, b)| {
            let d = match edge {
                Edge::Left => Vec2::new(target.min.x - b.min.x, 0.0),
                Edge::HCenter => Vec2::new(target.center().x - b.center().x, 0.0),
                Edge::Right => Vec2::new(target.max.x - b.max.x, 0.0),
                Edge::Top => Vec2::new(0.0, target.min.y - b.min.y),
                Edge::VCenter => Vec2::new(0.0, target.center().y - b.center().y),
                Edge::Bottom => Vec2::new(0.0, target.max.y - b.max.y),
            };
            (*id, d)
        })
        .collect()
}

fn translate_each(s: &mut Session, moves: &[(ElementId, Vec2)]) -> Result<usize> {
    let kf = s.editable_keyframe()?;
    let mut n = 0;
    for (id, d) in moves {
        if d.x == 0.0 && d.y == 0.0 {
            continue; // untouched elements stay shared with the undo snapshot
        }
        for e in kf.elements_mut_where(|e| e == *id) {
            e.transform = Affine::translate(d.x, d.y).mul(&e.transform);
            n += 1;
        }
    }
    Ok(n)
}

fn selected_bounds(s: &Session) -> Vec<(ElementId, Rect)> {
    s.selection.iter().map(|id| (*id, s.element_bounds(*id))).filter(|(_, b)| !b.is_empty()).collect()
}

fn align(s: &mut Session, p: &Value, edge: Edge) -> Result<Value> {
    let bounds = selected_bounds(s);
    if bounds.is_empty() {
        return Err(EngineError::State("nothing is selected".into()));
    }
    // One element aligns to the stage (there is nothing else to align it with).
    let to_stage = opt_bool(p, "to_stage")?.unwrap_or(bounds.len() == 1);
    let target = if to_stage {
        Rect::new(0.0, 0.0, s.doc.stage.width as f32, s.doc.stage.height as f32)
    } else {
        bounds.iter().fold(Rect::EMPTY, |r, (_, b)| r.union(b))
    };
    let moves = moves_for(&bounds, target, edge);
    let n = translate_each(s, &moves)?;
    Ok(json!({"moved": n, "to_stage": to_stage}))
}

fn distribute(s: &mut Session, horizontal: bool) -> Result<Value> {
    let mut bounds = selected_bounds(s);
    if bounds.len() < 3 {
        return Err(EngineError::State("select at least three elements to distribute".into()));
    }
    let key = |b: &Rect| if horizontal { b.center().x } else { b.center().y };
    bounds.sort_by(|a, b| key(&a.1).total_cmp(&key(&b.1)));
    let (Some(first), Some(last)) = (bounds.first(), bounds.last()) else { return Ok(json!({"moved": 0})) };
    let (a, z) = (key(&first.1), key(&last.1));
    let step = (z - a) / (bounds.len() - 1) as f32;
    let moves: Vec<(ElementId, Vec2)> = bounds
        .iter()
        .enumerate()
        .map(|(i, (id, b))| {
            let d = a + step * i as f32 - key(b);
            (*id, if horizontal { Vec2::new(d, 0.0) } else { Vec2::new(0.0, d) })
        })
        .collect();
    let n = translate_each(s, &moves)?;
    Ok(json!({"moved": n}))
}
