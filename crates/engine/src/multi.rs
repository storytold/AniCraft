//! Edit Multiple Frames, complete (ADR 0051; ADR 0047 began it with moves).
//!
//! With Edit Multiple Frames on, the artwork on every keyframe within the onion markers is edited
//! as if it were on one frame. These commands are that editing, on a list of [`FrameItem`]s (an
//! element on one keyframe), or on everything the markers show when no list is given:
//!
//! * `frames.edit_multiple` moves, `frames.transform` scales, rotates and skews about one shared
//!   pivot, `frames.align` aligns: each item on its own keyframe, as one undo step;
//! * `frames.delete`, `frames.copy`, `frames.cut`; `frames.paste` pastes in place onto every
//!   keyframe within the markers (artwork copied from several keyframes goes back keyframe by
//!   keyframe);
//! * raw (merge-drawing) shapes that moved or landed go back into the drawing on their own
//!   keyframe, cut and joined with what they touch, as `draw.drop` does on one frame
//!   (`merge: false` holds that back while a drag is under way; `frames.merge` does it when the
//!   drag ends).
//!
//! Every command answers with `items`: what the selection is afterwards (merged pieces have new
//! ids), so a front end keeps the multi-frame selection without asking again.
//!
//! The timeline is the one being edited, so all of this works inside a symbol open in place
//! (ADR 0045) on the symbol's own keyframes.

use crate::cmds_modify::{ClipLayer, Clipboard, collect_symbols, import_symbols};
use crate::onion::{FrameItem, multi_editable};
use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::{Element, ElementId, LayerId, Timeline};
use anicraft_geom::{Affine, Rect, Vec2};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c("frames.edit_multiple", "Move on Multiple Frames", "", None, "{first, last, dx, dy, items?: [[layer, frame, id], ..], merge?: bool} moves artwork on every keyframe within frames first..=last (0-based) on visible, unlocked layers, as Edit Multiple Frames does: the items given (a keyframe's first frame and an element id), or all of them; a motion tween's path and X/Y keys move with its object; raw shapes merge again where they land unless merge is false; one undo step; answers the items after the move", true, move_cmd as crate::RunFn),
        c("frames.transform", "Transform on Multiple Frames", "", None, "{first, last, items?, merge?, a, b, c, d, e, f} (an affine in stage space) | {rotate? (degrees), scale_x?, scale_y?, skew_x?, skew_y? (degrees), pivot?: [x, y] (default: the centre of every item's bounds together)} transforms the items on each keyframe about one shared pivot, as Free Transform does with Edit Multiple Frames on; one undo step", true, transform_cmd),
        c("frames.align", "Align on Multiple Frames", "", None, "{first, last, edge: left|hcenter|right|top|vcenter|bottom, items?, to_stage? (default: one item aligns to the stage), merge?} lines the items up with the edge of their bounds together (or the stage's), each on its own keyframe", true, align_cmd),
        c("frames.delete", "Clear on Multiple Frames", "", None, "{first, last, items?} deletes the items (default: everything shown) from their keyframes", true, delete_cmd),
        c("frames.copy", "Copy on Multiple Frames", "", None, "{first, last, items?} copies the items (default: everything shown), remembering each one's keyframe: Paste on Multiple Frames puts them back keyframe by keyframe, Paste puts them all on the current frame", false, copy_cmd),
        c("frames.cut", "Cut on Multiple Frames", "", None, "{first, last, items?} frames.copy, then frames.delete", true, cut_cmd),
        c("frames.paste", "Paste in Place on Multiple Frames", "", None, "{first, last, layer?} pastes in place onto the keyframes within the markers: artwork copied from several keyframes onto the keyframe covering the frame each came from (on the layers it came from when it came from several, else on `layer` or the active layer); artwork copied on one frame onto every keyframe of the layer; raw shapes merge where they land", true, paste_cmd),
        c("frames.merge", "Merge on Multiple Frames", "", None, "{first, last, items} the raw shapes among the items merge again with what they touch on their keyframes (the end of a drag made with merge: false); answers the items after the merge", true, merge_cmd),
    ]
}

// ----- params and the chosen items -----------------------------------------------------------

/// Most items a command takes.
const MAX_ITEMS: usize = 100_000;

/// The `first` and `last` params, in order.
pub(crate) fn range_param(p: &Value) -> Result<(u32, u32)> {
    let req = |k: &str| opt_u32(p, k)?.ok_or_else(|| EngineError::Param(format!("param '{k}' is required (a frame, 0-based)")));
    let (a, b) = (req("first")?, req("last")?);
    if a.max(b) > crate::MAX_FRAME {
        return Err(EngineError::Param(format!("frames go up to {}", crate::MAX_FRAME)));
    }
    Ok((a.min(b), a.max(b)))
}

fn items_param(p: &Value) -> Result<Option<Vec<FrameItem>>> {
    let Some(v) = p.get("items").filter(|v| !v.is_null()) else { return Ok(None) };
    let bad = || EngineError::Param("param 'items' must be a list of [layer, frame, id]".into());
    let list = v.as_array().ok_or_else(bad)?;
    if list.len() > MAX_ITEMS {
        return Err(EngineError::Param(format!("param 'items' has more than {MAX_ITEMS} items")));
    }
    list.iter()
        .map(|t| {
            let t = t.as_array().filter(|t| t.len() == 3).ok_or_else(bad)?;
            let n = |i: usize| t.get(i).and_then(Value::as_u64).ok_or_else(bad);
            let frame = u32::try_from(n(1)?).map_err(|_| bad())?;
            Ok(FrameItem { layer: LayerId(n(0)?), frame, element: ElementId(n(2)?) })
        })
        .collect::<Result<Vec<_>>>()
        .map(Some)
}

/// The items a command acts on, with their stage bounds: the `items` given (each must be shown
/// in `first..=last`), or everything shown.
fn chosen(s: &Session, p: &Value, first: u32, last: u32) -> Result<Vec<(FrameItem, Rect)>> {
    let shown = s.multi_frame_items(first, last);
    let Some(list) = items_param(p)? else { return Ok(shown) };
    let index: std::collections::HashMap<FrameItem, Rect> = shown.into_iter().collect();
    let mut out = Vec::with_capacity(list.len());
    let mut seen = HashSet::new();
    for it in list {
        let Some(&b) = index.get(&it) else {
            return Err(EngineError::State(format!(
                "element {} on layer {} at frame {} is not on a keyframe Edit Multiple Frames shows in frames {first}..={last} (on a visible, unlocked layer)",
                it.element.0, it.layer.0, it.frame
            )));
        };
        if seen.insert(it) {
            out.push((it, b));
        }
    }
    Ok(out)
}

/// The items keyframe by keyframe: (layer, keyframe start) → ids, in the order first seen.
fn by_keyframe(items: &[FrameItem]) -> BTreeMap<(LayerId, u32), Vec<ElementId>> {
    let mut out: BTreeMap<(LayerId, u32), Vec<ElementId>> = BTreeMap::new();
    for it in items {
        let ids = out.entry((it.layer, it.frame)).or_default();
        if !ids.contains(&it.element) {
            ids.push(it.element);
        }
    }
    out
}

fn items_json(items: &[FrameItem]) -> Value {
    json!(items.iter().map(FrameItem::to_json).collect::<Vec<_>>())
}

fn merge_param(p: &Value) -> Result<bool> {
    Ok(opt_bool(p, "merge")?.unwrap_or(true))
}

/// Runs `f` with the playhead on frame `start` of `layer` (a keyframe's first frame), so the
/// editing helpers that work at the playhead work on that keyframe; both come back after.
fn at_keyframe<T>(s: &mut Session, layer: LayerId, start: u32, f: impl FnOnce(&mut Session) -> Result<T>) -> Result<T> {
    let frame = s.frame;
    s.frame = start;
    let r = s.with_layer(layer, f);
    s.frame = frame;
    r
}

// ----- moves and transforms ------------------------------------------------------------------

/// Whether `m` only moves (no scale, rotation or skew).
fn translation_only(m: &Affine) -> bool {
    let [a, b, c, d, _, _] = m.m;
    (a - 1.0).abs() < 1e-6 && b.abs() < 1e-6 && c.abs() < 1e-6 && (d - 1.0).abs() < 1e-6
}

/// Applies each item's own stage-space affine to it on its keyframe (`t' = m · t`). A motion
/// tween's object carries its path and X/Y keys along; it can only move (refused otherwise).
/// Nothing changes when anything is refused (the session rolls back a failed command).
fn apply_each(s: &mut Session, moves: &[(FrameItem, Affine)]) -> Result<usize> {
    let mut by: BTreeMap<(LayerId, u32), Vec<(ElementId, Affine)>> = BTreeMap::new();
    for (it, m) in moves {
        if !m.is_finite() || m.determinant().abs() < 1e-8 {
            return Err(EngineError::Param("the transform is degenerate (it would collapse the artwork)".into()));
        }
        by.entry((it.layer, it.frame)).or_default().push((it.element, *m));
    }
    let mut n = 0usize;
    for ((layer, start), list) in &by {
        let l = s.layer_mut(Some(*layer))?;
        let Some(ki) = l.keyframe_starting_at(*start) else { continue };
        let Some(kf) = l.keyframe_mut(ki) else { continue };
        if let Some(mt) = kf.motion.as_mut()
            && let Some((_, m)) = list.iter().find(|(id, _)| *id == mt.target)
        {
            if !translation_only(m) {
                return Err(EngineError::State("a motion tween's object can only be moved on several frames at once; scale, rotate or skew it on its own keyframe".into()));
            }
            let d = m.translation();
            if d != Vec2::ZERO && !mt.translate(d) {
                return Err(EngineError::State("the motion path would move out of range".into()));
            }
        }
        for e in kf.elements_mut_where(|id| list.iter().any(|(x, _)| *x == id)) {
            let Some((_, m)) = list.iter().find(|(x, _)| *x == e.id) else { continue };
            let t = m.mul(&e.transform);
            let d = t.decompose();
            if !t.is_finite() || d.sx.abs() > 10_000.0 || d.sy.abs() > 10_000.0 || d.sx.abs() < 1e-4 || d.sy.abs() < 1e-4 {
                return Err(EngineError::Param("the transform would make an element too large or too small".into()));
            }
            e.transform = t;
            n += 1;
        }
    }
    Ok(n)
}

/// The raw shapes among `items` go back into the drawing on their keyframes, on top of what they
/// touch, cutting and joining it (`draw.drop` on each keyframe). The items afterwards: objects as
/// they were, each raw shape's pieces with their new ids.
fn remerge(s: &mut Session, items: &[FrameItem]) -> Result<Vec<FrameItem>> {
    // Each keyframe's raw shapes merged: (layer, start) → the pieces (ids gone, ids new).
    let mut merged: BTreeMap<(LayerId, u32), (Vec<ElementId>, Vec<ElementId>)> = BTreeMap::new();
    for ((layer, start), ids) in by_keyframe(items) {
        let l = s.layer_mut(Some(layer))?;
        let Some(ki) = l.keyframe_starting_at(start) else { continue };
        let Some(kf) = l.keyframe_mut(ki) else { continue };
        let mut moving: Vec<Element> = Vec::new();
        kf.elements.retain(|e| {
            let take = ids.contains(&e.id) && crate::merge::mergeable(e).is_some();
            if take {
                moving.push((**e).clone());
            }
            !take
        });
        if moving.is_empty() {
            continue;
        }
        let gone: Vec<ElementId> = moving.iter().map(|e| e.id).collect();
        let fresh = at_keyframe(s, layer, start, |s| crate::merge::merge_in(s, moving))?;
        merged.insert((layer, start), (gone, fresh));
    }
    // In the order given: objects as they were, each keyframe's pieces where its first raw
    // shape was.
    let mut out: Vec<FrameItem> = Vec::with_capacity(items.len());
    for it in items {
        match merged.get_mut(&(it.layer, it.frame)) {
            Some((gone, fresh)) if gone.contains(&it.element) => out.extend(std::mem::take(fresh).into_iter().map(|element| FrameItem { element, ..*it })),
            _ => out.push(*it),
        }
    }
    Ok(out)
}

/// `remerge` when `merge`, else the items as they are.
fn finish(s: &mut Session, items: Vec<FrameItem>, merge: bool) -> Result<Vec<FrameItem>> {
    if merge { remerge(s, &items) } else { Ok(items) }
}

fn move_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let (first, last) = range_param(p)?;
    let d = Vec2::new(f32_req(p, "dx")?, f32_req(p, "dy")?);
    if d.x.abs() > crate::cmds_draw::MAX_EXTENT || d.y.abs() > crate::cmds_draw::MAX_EXTENT {
        return Err(EngineError::Param("the move is too large".into()));
    }
    let merge = merge_param(p)?;
    let items: Vec<FrameItem> = chosen(s, p, first, last)?.into_iter().map(|(it, _)| it).collect();
    let keyframes = by_keyframe(&items).len();
    let mv = Affine::translate(d.x, d.y);
    let moved = if d == Vec2::ZERO { 0 } else { apply_each(s, &items.iter().map(|it| (*it, mv)).collect::<Vec<_>>())? };
    let items = if moved > 0 { finish(s, items, merge)? } else { items };
    Ok(json!({"moved": moved, "keyframes": keyframes, "dx": d.x, "dy": d.y, "items": items_json(&items)}))
}

/// The transform a `frames.transform` call asks for, given the items' bounds together.
fn transform_param(p: &Value, bounds: Rect) -> Result<Affine> {
    let keys = ["a", "b", "c", "d", "e", "f"];
    if keys.iter().any(|k| p.get(*k).is_some_and(|v| !v.is_null())) {
        let mut m = [0.0f32; 6];
        for (slot, k) in m.iter_mut().zip(keys) {
            *slot = f32_req(p, k)?;
        }
        return Ok(Affine { m });
    }
    let deg = |k: &str| -> Result<f32> {
        let v = opt_f32(p, k)?.unwrap_or(0.0);
        if v.abs() > 36_000.0 {
            return Err(EngineError::Param(format!("param '{k}' is out of range")));
        }
        Ok(v.to_radians())
    };
    let factor = |k: &str| -> Result<f32> {
        let v = opt_f32(p, k)?.unwrap_or(1.0);
        if !(1e-3..=1e3).contains(&v.abs()) {
            return Err(EngineError::Param(format!("param '{k}' must be a scale between 0.001 and 1000 (negative flips)")));
        }
        Ok(v)
    };
    let (sx, sy, r, kx, ky) = (factor("scale_x")?, factor("scale_y")?, deg("rotate")?, deg("skew_x")?, deg("skew_y")?);
    let pivot = match p.get("pivot").filter(|v| !v.is_null()) {
        Some(v) => {
            let bad = || EngineError::Param("param 'pivot' must be [x, y]".into());
            let a = v.as_array().filter(|a| a.len() == 2).ok_or_else(bad)?;
            let n = |i: usize| a.get(i).and_then(Value::as_f64).map(|x| x as f32).filter(|x| x.is_finite() && x.abs() <= crate::cmds_draw::MAX_EXTENT).ok_or_else(bad);
            Vec2::new(n(0)?, n(1)?)
        }
        None => bounds.center(),
    };
    // Scale, then skew, then rotate, about the pivot.
    let skew = Affine { m: [1.0, ky.tan(), kx.tan(), 1.0, 0.0, 0.0] };
    let m = Affine::rotate(r).mul(&skew).mul(&Affine::scale(sx, sy));
    Ok(Affine::translate(pivot.x, pivot.y).mul(&m).mul(&Affine::translate(-pivot.x, -pivot.y)))
}

fn transform_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let (first, last) = range_param(p)?;
    let merge = merge_param(p)?;
    let chosen = chosen(s, p, first, last)?;
    if chosen.is_empty() {
        return Err(EngineError::State("nothing to transform on the frames between the markers".into()));
    }
    let bounds = chosen.iter().fold(Rect::EMPTY, |r, (_, b)| r.union(b));
    let m = transform_param(p, bounds)?;
    let items: Vec<FrameItem> = chosen.into_iter().map(|(it, _)| it).collect();
    let n = apply_each(s, &items.iter().map(|it| (*it, m)).collect::<Vec<_>>())?;
    let items = finish(s, items, merge)?;
    Ok(json!({"transformed": n, "keyframes": by_keyframe(&items).len(), "matrix": m.m, "items": items_json(&items)}))
}

fn align_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let (first, last) = range_param(p)?;
    let edge = str_req(p, "edge")?;
    let merge = merge_param(p)?;
    let chosen: Vec<(FrameItem, Rect)> = chosen(s, p, first, last)?.into_iter().filter(|(_, b)| !b.is_empty()).collect();
    if chosen.is_empty() {
        return Err(EngineError::State("nothing to align on the frames between the markers".into()));
    }
    let to_stage = opt_bool(p, "to_stage")?.unwrap_or(chosen.len() == 1);
    let target = if to_stage { Rect::new(0.0, 0.0, s.doc.stage.width as f32, s.doc.stage.height as f32) } else { chosen.iter().fold(Rect::EMPTY, |r, (_, b)| r.union(b)) };
    let d = |b: &Rect| -> Result<Vec2> {
        Ok(match edge {
            "left" => Vec2::new(target.min.x - b.min.x, 0.0),
            "hcenter" => Vec2::new(target.center().x - b.center().x, 0.0),
            "right" => Vec2::new(target.max.x - b.max.x, 0.0),
            "top" => Vec2::new(0.0, target.min.y - b.min.y),
            "vcenter" => Vec2::new(0.0, target.center().y - b.center().y),
            "bottom" => Vec2::new(0.0, target.max.y - b.max.y),
            _ => return Err(EngineError::Param(format!("unknown edge '{edge}' (left, hcenter, right, top, vcenter or bottom)"))),
        })
    };
    let mut moves = Vec::with_capacity(chosen.len());
    for (it, b) in &chosen {
        let v = d(b)?;
        if v != Vec2::ZERO {
            moves.push((*it, Affine::translate(v.x, v.y)));
        }
    }
    let n = apply_each(s, &moves)?;
    let moved: Vec<FrameItem> = moves.iter().map(|(it, _)| *it).collect();
    let mut items: Vec<FrameItem> = chosen.iter().map(|(it, _)| *it).filter(|it| !moved.contains(it)).collect();
    items.extend(finish(s, moved, merge)?);
    Ok(json!({"moved": n, "to_stage": to_stage, "items": items_json(&items)}))
}

fn merge_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let (first, last) = range_param(p)?;
    if items_param(p)?.is_none() {
        return Err(EngineError::Param("param 'items' is required (the items a drag moved)".into()));
    }
    let items: Vec<FrameItem> = chosen(s, p, first, last)?.into_iter().map(|(it, _)| it).collect();
    let items = remerge(s, &items)?;
    Ok(json!({"items": items_json(&items)}))
}

// ----- delete, copy, cut, paste --------------------------------------------------------------

fn delete_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let (first, last) = range_param(p)?;
    let items: Vec<FrameItem> = chosen(s, p, first, last)?.into_iter().map(|(it, _)| it).collect();
    if items.is_empty() {
        return Err(EngineError::State("nothing to delete on the frames between the markers".into()));
    }
    let groups = by_keyframe(&items);
    let mut deleted = 0usize;
    for ((layer, start), ids) in &groups {
        let l = s.layer_mut(Some(*layer))?;
        let Some(ki) = l.keyframe_starting_at(*start) else { continue };
        let Some(kf) = l.keyframe_mut(ki) else { continue };
        let before = kf.elements.len();
        kf.elements.retain(|e| !ids.contains(&e.id));
        deleted += before - kf.elements.len();
    }
    s.selection.clear();
    Ok(json!({"deleted": deleted, "keyframes": groups.len(), "items": []}))
}

/// The layers' order in `tl`, top first, for sorting.
fn rank(tl: &Timeline, l: LayerId) -> usize {
    tl.layers.iter().position(|x| x.id == l).unwrap_or(usize::MAX)
}

fn copy_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let (first, last) = range_param(p)?;
    let items: Vec<FrameItem> = chosen(s, p, first, last)?.into_iter().map(|(it, _)| it).collect();
    if items.is_empty() {
        return Err(EngineError::State("nothing to copy on the frames between the markers".into()));
    }
    let tl = s.timeline();
    let mut groups: Vec<((LayerId, u32), Vec<ElementId>)> = by_keyframe(&items).into_iter().collect();
    // Top layer first, then frame order (a clipboard lists layers top to bottom).
    groups.sort_by_key(|((l, f), _)| (rank(tl, *l), *f));
    let mut layers = Vec::with_capacity(groups.len());
    for ((layer, start), ids) in groups {
        let Some(l) = tl.layer(layer) else { continue };
        let Some(k) = l.keyframe_starting_at(start).and_then(|i| l.keyframes.get(i)) else { continue };
        // In their stacking order on the keyframe.
        let elements: Vec<Element> = k.elements.iter().filter(|e| ids.contains(&e.id)).map(|e| (**e).clone()).collect();
        if !elements.is_empty() {
            layers.push(ClipLayer { layer, name: l.name.clone(), elements, frame: Some(start) });
        }
    }
    let all: Vec<Element> = layers.iter().flat_map(|c| c.elements.iter().cloned()).collect();
    let symbols = collect_symbols(s, &all);
    let (n, keyframes) = (all.len(), layers.len());
    s.clipboard = Clipboard { layers, symbols };
    Ok(json!({"copied": n, "keyframes": keyframes}))
}

fn cut_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let v = copy_cmd(s, p)?;
    delete_cmd(s, p)?;
    Ok(json!({"cut": v["copied"], "keyframes": v["keyframes"], "items": []}))
}

/// The first frames of `layer`'s keyframes that Edit Multiple Frames shows in `first..=last`
/// (the playhead's too); none when it doesn't edit that layer.
fn shown_keyframes(tl: &Timeline, layer: LayerId, first: u32, last: u32) -> Vec<u32> {
    let Some(i) = tl.layers.iter().position(|l| l.id == layer) else { return Vec::new() };
    if !multi_editable(tl, i) {
        return Vec::new();
    }
    tl.layers.get(i).map(|l| l.keyframes.iter().filter(|k| k.start <= last && k.end() > first).map(|k| k.start).collect()).unwrap_or_default()
}

fn paste_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let (first, last) = range_param(p)?;
    if s.clipboard.is_empty() {
        return Err(EngineError::State("the clipboard is empty".into()));
    }
    if crate::cmds_modify::clipboard_nests(s) {
        return Err(EngineError::State("a symbol can't be pasted inside itself".into()));
    }
    let active = match opt_u64(p, "layer")? {
        Some(id) => LayerId(id),
        None => s.layer,
    };
    let clip = s.clipboard.clone();
    // Artwork from several layers goes back on those layers when they are all here and edited
    // with Edit Multiple Frames; else everything goes on the one layer, as a paste does.
    let mut from: Vec<LayerId> = Vec::new();
    for c in &clip.layers {
        if !from.contains(&c.layer) {
            from.push(c.layer);
        }
    }
    let tl = s.timeline();
    let home = from.len() > 1 && from.iter().all(|l| !shown_keyframes(tl, *l, first, last).is_empty());
    let mut plan: Vec<(LayerId, u32, &ClipLayer)> = Vec::new();
    let mut skipped = 0usize;
    for c in &clip.layers {
        let target = if home { c.layer } else { active };
        let keys = shown_keyframes(tl, target, first, last);
        match c.frame {
            // Back onto the keyframe covering the frame it came from, if the markers show it.
            Some(f) => match tl.layer(target).and_then(|l| l.keyframe_at(f)).map(|k| k.start).filter(|k| keys.contains(k)) {
                Some(k) => plan.push((target, k, c)),
                None => skipped += 1,
            },
            // Copied on one frame: onto every keyframe shown.
            None => plan.extend(keys.iter().map(|&k| (target, k, c))),
        }
    }
    if plan.is_empty() {
        let why = if tl.layer(active).is_some_and(|_| shown_keyframes(tl, active, first, last).is_empty()) { "the layer has no keyframe Edit Multiple Frames can paste onto between the markers (it must be visible, unlocked and hold artwork)" } else { "none of the copied keyframes is between the markers" };
        return Err(EngineError::State(why.into()));
    }
    import_symbols(s, &clip.symbols);
    let mut items = Vec::new();
    let mut keyframes: HashSet<(LayerId, u32)> = HashSet::new();
    for (layer, start, c) in plan {
        let (fresh, _) = at_keyframe(s, layer, start, |s| crate::cmds_modify::paste_layer(s, &c.elements, Vec2::ZERO))?;
        keyframes.insert((layer, start));
        items.extend(fresh.into_iter().map(|element| FrameItem { layer, frame: start, element }));
    }
    s.selection.clear();
    Ok(json!({"pasted": items.len(), "keyframes": keyframes.len(), "skipped": skipped, "items": items_json(&items)}))
}

#[cfg(test)]
#[path = "multi_tests.rs"]
mod tests;
