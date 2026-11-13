//! Clipboard (cut/copy/paste), transforms (free transform, rotate 90°, flip) and alignment.

use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::{Element, ElementId, ElementKind, Layer, LayerId, LayerKind, Symbol, SymbolId};
use anicraft_geom::{Affine, Rect, Vec2};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c("edit.cut", "Cut", "Edit", Some("Ctrl+X"), "{}", true, cut as crate::RunFn),
        c("edit.copy", "Copy", "Edit", Some("Ctrl+C"), "{} (the selection on every layer, each layer's artwork kept apart)", false, copy),
        c("edit.paste", "Paste in Center", "Edit", Some("Ctrl+V"), "{x?, y?} (centre of the pasted artwork; default: offset 10 px). Artwork copied from one layer lands on the active layer; from several, back on those layers when they are all here, else on the active layer and the ones below it, creating missing ones", true, paste),
        c("edit.paste_in_place", "Paste in Place", "Edit", Some("Ctrl+Shift+V"), "{} (layers as edit.paste)", true, paste_in_place),
        c("modify.transform", "Transform", "", None, "{a, b, c, d, e, f} affine applied in stage space (free transform), to the selection on every layer", true, |s, p| s.each_layer(true, |s| transform(s, p))),
        c("modify.rotate_cw", "Rotate 90° CW", "Modify/Transform", Some("Ctrl+Shift+9"), "{}", true, |s, _| about_center(s, Affine::rotate(std::f32::consts::FRAC_PI_2))),
        c("modify.rotate_ccw", "Rotate 90° CCW", "Modify/Transform", Some("Ctrl+Shift+7"), "{}", true, |s, _| about_center(s, Affine::rotate(-std::f32::consts::FRAC_PI_2))),
        c("modify.flip_h", "Flip Horizontal", "Modify/Transform", None, "{}", true, |s, _| about_center(s, Affine::scale(-1.0, 1.0))),
        c("modify.flip_v", "Flip Vertical", "Modify/Transform", None, "{}", true, |s, _| about_center(s, Affine::scale(1.0, -1.0))),
        c("modify.remove_transform", "Remove Transform", "Modify/Transform", Some("Ctrl+Shift+Z"), "{}", true, |s, p| s.each_layer(true, |s| remove_transform(s, p))),
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

/// Copied artwork: elements as they look at the playhead, kept apart by the layer they came from,
/// plus every symbol they need, so a paste into another document is complete.
#[derive(Clone, Default)]
pub struct Clipboard {
    /// One entry per layer the selection was on, top to bottom.
    pub layers: Vec<ClipLayer>,
    pub symbols: Vec<Arc<Symbol>>,
}

/// One layer's copied artwork.
#[derive(Clone)]
pub struct ClipLayer {
    /// The layer it came from, and its name (for a layer a paste has to create).
    pub layer: LayerId,
    pub name: String,
    pub elements: Vec<Element>,
    /// The keyframe (its first frame) it was copied from with Edit Multiple Frames (ADR 0051),
    /// where Paste in Place on Multiple Frames puts it back; `None` for a copy on one frame.
    pub frame: Option<u32>,
}

impl Clipboard {
    pub fn is_empty(&self) -> bool {
        self.layers.iter().all(|l| l.elements.is_empty())
    }
    /// Every copied element, layer by layer.
    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.layers.iter().flat_map(|l| l.elements.iter())
    }
    pub fn len(&self) -> usize {
        self.layers.iter().map(|l| l.elements.len()).sum()
    }
}

/// The symbols `e` places: itself, or what is inside it as a group (ADR 0073).
fn symbol_refs(e: &Element) -> Vec<SymbolId> {
    match &e.kind {
        ElementKind::Symbol { symbol, .. } => vec![*symbol],
        ElementKind::Group { .. } => anicraft_doc::expand_groups(std::slice::from_ref(&Arc::new(e.clone()))).iter().filter_map(|m| if let ElementKind::Symbol { symbol, .. } = m.kind { Some(symbol) } else { None }).collect(),
        ElementKind::Shape(_) | ElementKind::Text(_) | ElementKind::Camera => Vec::new(),
    }
}

/// Symbols reachable from `roots` (bounded walk; libraries may be cyclic).
pub(crate) fn collect_symbols(s: &Session, roots: &[Element]) -> Vec<Arc<Symbol>> {
    let mut seen: HashSet<SymbolId> = HashSet::new();
    let mut stack: Vec<SymbolId> = roots.iter().flat_map(symbol_refs).collect();
    let mut out = Vec::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) || seen.len() > 10_000 {
            continue;
        }
        let Some(sym) = s.doc.library.iter().find(|x| x.id == id) else { continue };
        for l in &sym.timeline.layers {
            for k in &l.keyframes {
                stack.extend(k.elements.iter().flat_map(|e| symbol_refs(e)));
            }
        }
        out.push(sym.clone());
    }
    out
}

fn copy(s: &mut Session, _: &Value) -> Result<Value> {
    let layers = s.selected_layers();
    if layers.is_empty() {
        return Err(EngineError::State("nothing is selected".into()));
    }
    // What you see is what you copy: the evaluated state at the playhead (tweens applied).
    let mut copied = Vec::with_capacity(layers.len());
    for l in layers {
        let elements: Vec<Element> = s.elements_on(l).iter().filter(|e| s.selection.contains(l, e.id)).map(|e| (**e).clone()).collect();
        let name = s.timeline().layer(l).map(|x| x.name.clone()).unwrap_or_default();
        if !elements.is_empty() {
            copied.push(ClipLayer { layer: l, name, elements, frame: None });
        }
    }
    let all: Vec<Element> = copied.iter().flat_map(|c| c.elements.iter().cloned()).collect();
    let symbols = collect_symbols(s, &all);
    let n = all.len();
    let layers = copied.len();
    s.clipboard = Clipboard { layers: copied, symbols };
    Ok(json!({"copied": n, "layers": layers}))
}

fn cut(s: &mut Session, p: &Value) -> Result<Value> {
    copy(s, p)?;
    crate::cmds_draw::delete_selection(s, p)?;
    Ok(json!({"cut": s.clipboard.len()}))
}

fn clipboard_bounds(s: &Session) -> Rect {
    s.clipboard.elements().fold(Rect::EMPTY, |r, e| r.union(&clip_element_bounds(s, e)))
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

/// The layers a paste puts each copied layer's artwork on (ADR 0044). One copied layer goes on
/// the active layer. Several go back on the layers they came from when every one of them is in
/// this scene (same id and name) and can be drawn on; else on the active layer and the drawable
/// layers below it, in order, with missing layers created at the bottom named after the copied
/// ones, as Paste Frames does.
fn paste_targets(s: &mut Session) -> Result<Vec<LayerId>> {
    // Each layer the artwork came from once (a copy with Edit Multiple Frames has one entry per
    // keyframe, ADR 0051), in order.
    let mut src: Vec<(LayerId, String)> = Vec::new();
    for c in &s.clipboard.layers {
        if !src.iter().any(|(l, _)| *l == c.layer) {
            src.push((c.layer, c.name.clone()));
        }
    }
    let dst = layer_targets(s, &src)?;
    Ok(s.clipboard.layers.iter().map(|c| src.iter().position(|(l, _)| *l == c.layer).and_then(|i| dst.get(i)).copied().unwrap_or(s.layer)).collect())
}

/// [`paste_targets`] for each source layer (its id and name).
fn layer_targets(s: &mut Session, src: &[(LayerId, String)]) -> Result<Vec<LayerId>> {
    let n = src.len();
    if n <= 1 {
        return Ok(vec![s.layer]);
    }
    let tl = s.timeline();
    let drawable = |s: &Session, l: &Layer| s.selectable(l.id) && !matches!(l.kind, LayerKind::Folder | LayerKind::Camera | LayerKind::Armature);
    let home: Vec<LayerId> = src.iter().filter_map(|(id, name)| tl.layer(*id).filter(|l| l.name == *name && drawable(s, l)).map(|l| l.id)).collect();
    if home.len() == n {
        return Ok(home);
    }
    let at = s.active_layer_index().unwrap_or(0);
    let mut out = vec![s.layer];
    out.extend(tl.layers.iter().skip(at + 1).filter(|l| drawable(s, l)).map(|l| l.id).take(n - 1));
    let names: Vec<String> = src.iter().skip(out.len()).map(|(_, name)| name.clone()).collect();
    for name in names {
        if s.timeline().layers.len() >= 1000 {
            return Err(EngineError::State("too many layers (limit 1000)".into()));
        }
        let id = LayerId(s.doc.alloc_id());
        let count = s.timeline().layers.len();
        let mut layer = Layer::new(id, name);
        if let Some(k) = layer.keyframe_mut(0) {
            k.duration = s.frame_count().max(1);
        }
        layer.color = crate::cmds_timeline::layer_color(count + 1);
        let tl = crate::editing::edited_mut(&mut s.doc, s.scene, &s.editing)?;
        tl.layers.push(Arc::new(layer));
        tl.repair_masks();
        out.push(id);
    }
    Ok(out)
}

/// Whether pasting the clipboard would put a symbol inside itself: inside an open symbol, an
/// instance of it (or of one using it).
pub(crate) fn clipboard_nests(s: &Session) -> bool {
    s.clipboard.layers.iter().flat_map(|c| &c.elements).any(|e| matches!(e.kind, anicraft_doc::ElementKind::Symbol { symbol, .. } if crate::editing::would_nest(s, symbol)))
}

fn paste_with(s: &mut Session, offset: Vec2) -> Result<Value> {
    if s.clipboard.is_empty() {
        return Err(EngineError::State("the clipboard is empty".into()));
    }
    if clipboard_nests(s) {
        return Err(EngineError::State("a symbol can't be pasted inside itself".into()));
    }
    import_symbols(s, &s.clipboard.symbols.clone());
    let targets = paste_targets(s)?;
    let copied = s.clipboard.layers.clone();
    let mut sel = crate::Selection::default();
    let mut all = Vec::new();
    let mut merged = false;
    for (c, &layer) in copied.iter().zip(&targets) {
        let (fresh, m) = s.with_layer(layer, |s| paste_layer(s, &c.elements, offset))?;
        merged |= m;
        all.extend(fresh.iter().map(|e| e.0));
        sel.extend_on(layer, fresh);
    }
    s.selection = sel;
    let mut layers: Vec<LayerId> = Vec::new();
    for l in &targets {
        if !layers.contains(l) {
            layers.push(*l);
        }
    }
    if let Some(&first) = layers.first()
        && layers.len() > 1
    {
        s.layer = first;
    }
    let mut v = json!({"pasted": all, "merged": merged});
    if layers.len() > 1 {
        v["layers"] = json!(layers.iter().map(|l| l.0).collect::<Vec<_>>());
    }
    Ok(v)
}

/// Pastes `elements` moved by `offset` into the active layer's keyframe at the playhead; their
/// new ids, and whether raw shapes merged.
pub(crate) fn paste_layer(s: &mut Session, elements: &[Element], offset: Vec2) -> Result<(Vec<ElementId>, bool)> {
    let mut fresh = Vec::new();
    let mut out = Vec::new();
    let mut raw = Vec::new();
    for e in elements.iter().cloned() {
        let mut e = e;
        e.id = ElementId(s.doc.alloc_id());
        e.transform = Affine::translate(offset.x, offset.y).mul(&e.transform);
        // Raw shapes go into the drawing where they land, cutting and joining what they touch
        // (ADR 0042); objects go on top.
        if crate::merge::mergeable(&e).is_some() {
            raw.push(e);
        } else {
            fresh.push(e.id);
            out.push(Arc::new(e));
        }
    }
    let kf = s.editable_keyframe()?;
    if kf.elements.len() + out.len() + raw.len() > 100_000 {
        return Err(EngineError::State("too many elements on this keyframe".into()));
    }
    kf.elements.extend(out);
    let merged = !raw.is_empty();
    if merged {
        fresh.extend(crate::merge::merge_in(s, raw)?);
    }
    Ok((fresh, merged))
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

/// `m` about the centre of the whole selection's bounds, on every layer it is on.
fn about_center(s: &mut Session, m: Affine) -> Result<Value> {
    let b = s.selection_bounds();
    if b.is_empty() {
        return Err(EngineError::State("nothing is selected".into()));
    }
    let c = b.center();
    let full = Affine::translate(c.x, c.y).mul(&m).mul(&Affine::translate(-c.x, -c.y));
    s.each_layer(true, |s| Ok(json!({"transformed": apply(s, &full)?})))
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

/// A selected element: its layer and id.
type Item = (LayerId, ElementId);

fn moves_for(bounds: &[(Item, Rect)], target: Rect, edge: Edge) -> Vec<(Item, Vec2)> {
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

/// Moves each element by its own amount, on its own layer.
fn translate_each(s: &mut Session, moves: &[(Item, Vec2)]) -> Result<usize> {
    let mut layers: Vec<LayerId> = Vec::new();
    for ((l, _), _) in moves {
        if !layers.contains(l) {
            layers.push(*l);
        }
    }
    let mut n = 0;
    for layer in layers {
        let mine: Vec<(ElementId, Vec2)> = moves.iter().filter(|((l, _), d)| *l == layer && (d.x != 0.0 || d.y != 0.0)).map(|((_, id), d)| (*id, *d)).collect();
        if mine.is_empty() {
            continue; // untouched layers stay shared with the undo snapshot
        }
        n += s.with_layer(layer, |s| -> Result<usize> {
            let kf = s.editable_keyframe()?;
            let mut n = 0;
            for (id, d) in &mine {
                for e in kf.elements_mut_where(|e| e == *id) {
                    e.transform = Affine::translate(d.x, d.y).mul(&e.transform);
                    n += 1;
                }
            }
            Ok(n)
        })?;
    }
    Ok(n)
}

/// Each selected element's bounds, over every layer: alignment works across layers.
fn selected_bounds(s: &Session) -> Vec<(Item, Rect)> {
    s.selected_elements().iter().map(|(l, e)| ((*l, e.id), s.element_bounds_on(*l, e.id))).filter(|(_, b)| !b.is_empty()).collect()
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
    let moves: Vec<(Item, Vec2)> = bounds
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
