//! Timeline and layer commands.

use crate::params::*;
use crate::{CommandSpec, EngineError, MAX_FRAME, Result, Session};
use anicraft_doc::{Keyframe, Layer, LayerId, LayerKind, Tween, TweenKind, evaluate_layer_elements};
use serde_json::{Value, json};
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c("timeline.insert_frame", "Frame", "Insert/Timeline", Some("F5"), "{layer? | layers?: [..], frame?, count?}", true, insert_frame as crate::RunFn),
        c("timeline.remove_frame", "Remove Frame", "Insert/Timeline", Some("Shift+F5"), "{layer? | layers?: [..], frame?, count?}", true, remove_frame),
        c("timeline.insert_keyframe", "Keyframe", "Insert/Timeline", Some("F6"), "{layer? | layers?: [..], frame?}", true, insert_keyframe),
        c("timeline.insert_blank_keyframe", "Blank Keyframe", "Insert/Timeline", Some("F7"), "{layer? | layers?: [..], frame?}", true, insert_blank_keyframe),
        c("timeline.clear_keyframe", "Clear Keyframe", "Insert/Timeline", Some("Shift+F6"), "{layer? | layers?: [..], frame?, count?}", true, clear_keyframe),
        c("timeline.move_keyframes", "Move Keyframes", "", None, "{layer? | layers?: [..], from, to?, by, layer_by? (rows down; negative = up)}", true, move_keyframes),
        c("timeline.create_tween", "Create Classic Tween", "Insert", None, "{layer?, frame?, easing?}", true, create_tween),
        c("timeline.create_shape_tween", "Create Shape Tween", "Insert", None, "{layer?, frame?, easing?}", true, create_shape_tween),
        c("timeline.remove_tween", "Remove Tween", "Insert", None, "{layer?, frame?}", true, remove_tween),
        c("timeline.set_easing", "Set Tween Easing", "", None, "{easing, layer?, frame?}", true, set_easing),
        c("timeline.set_label", "Frame Label", "", None, "{label, layer?, frame?}", true, set_label),
        c("timeline.goto", "Go to Frame", "", None, "{frame}", false, goto),
        c("timeline.next_frame", "Next Frame", "Control", Some("."), "{}", false, |s, _| step(s, 1)),
        c("timeline.prev_frame", "Previous Frame", "Control", Some(","), "{}", false, |s, _| step(s, -1)),
        c("timeline.first_frame", "Go to First Frame", "Control", Some("Home"), "{}", false, |s, _| {
            s.set_frame(0);
            Ok(json!({"frame": 0}))
        }),
        c("timeline.last_frame", "Go to Last Frame", "Control", Some("End"), "{}", false, |s, _| {
            let f = s.frame_count().saturating_sub(1);
            s.set_frame(f);
            Ok(json!({"frame": f}))
        }),
        c("layer.new", "New Layer", "Insert/Timeline", None, "{name?}", true, new_layer),
        c("layer.delete", "Delete Layer", "Modify/Timeline", None, "{layer?}", true, delete_layer),
        c("layer.rename", "Rename Layer", "", None, "{name, layer?}", true, rename_layer),
        c("layer.toggle_visible", "Show/Hide Layer", "", None, "{layer?}", true, |s, p| toggle(s, p, |l| l.visible = !l.visible)),
        c("layer.toggle_lock", "Lock/Unlock Layer", "", None, "{layer?}", true, |s, p| toggle(s, p, |l| l.locked = !l.locked)),
        c("layer.toggle_outline", "Outline Layer", "", None, "{layer?}", true, |s, p| toggle(s, p, |l| l.outline = !l.outline)),
        c("layer.select", "Select Layer", "", None, "{layer: id} or {index}", false, select_layer),
        c("layer.move_up", "Move Layer Up", "Modify/Timeline", None, "{layer?}", true, |s, p| move_layer(s, p, -1)),
        c("layer.move_down", "Move Layer Down", "Modify/Timeline", None, "{layer?}", true, |s, p| move_layer(s, p, 1)),
        c("layer.toggle_mask", "Mask", "Modify/Timeline", None, "{layer?}", true, toggle_mask),
        c("layer.set_type", "Layer Type", "", None, "{type: normal|mask|masked|guide, layer?}", true, set_layer_type),
    ]
}

/// The mask command: turns a layer into a mask over the normal layer directly under it, or a
/// mask back into a normal layer (its masked layers become normal too).
fn toggle_mask(s: &mut Session, p: &Value) -> Result<Value> {
    let id = opt_u64(p, "layer")?.map_or(s.layer, LayerId);
    let is_mask = s.doc.timeline.layer(id).ok_or_else(|| EngineError::State(format!("no layer with id {}", id.0)))?.kind == LayerKind::Mask;
    let ty = if is_mask { "normal" } else { "mask" };
    set_type(s, id, ty)?;
    if !is_mask {
        let tl = &mut s.doc.timeline;
        if let Some(i) = tl.layers.iter().position(|l| l.id == id)
            && let Some(below) = tl.layers.get_mut(i + 1)
            && below.kind == LayerKind::Normal
            && below.parent.is_none()
        {
            Arc::make_mut(below).parent = Some(id);
        }
    }
    Ok(layer_type_json(s, id))
}

fn set_layer_type(s: &mut Session, p: &Value) -> Result<Value> {
    let id = opt_u64(p, "layer")?.map_or(s.layer, LayerId);
    set_type(s, id, str_req(p, "type")?)?;
    Ok(layer_type_json(s, id))
}

fn set_type(s: &mut Session, id: LayerId, ty: &str) -> Result<()> {
    let tl = &mut s.doc.timeline;
    let i = tl.layers.iter().position(|l| l.id == id).ok_or_else(|| EngineError::State(format!("no layer with id {}", id.0)))?;
    let (kind, parent) = match ty {
        "normal" => (LayerKind::Normal, None),
        "mask" => (LayerKind::Mask, None),
        "guide" => (LayerKind::Guide, None),
        "masked" => {
            let m = tl.mask_above(i).ok_or_else(|| EngineError::State("only a layer directly under a mask layer (or its masked layers) can be masked".into()))?;
            (LayerKind::Normal, Some(m))
        }
        other => return Err(EngineError::Param(format!("unknown layer type '{other}' (normal, mask, masked or guide)"))),
    };
    let l = tl.layer_mut(id).ok_or_else(|| EngineError::State(format!("no layer with id {}", id.0)))?;
    l.kind = kind;
    l.parent = parent;
    tl.repair_masks();
    Ok(())
}

fn layer_type_json(s: &Session, id: LayerId) -> Value {
    let Some(l) = s.doc.timeline.layer(id) else { return json!({}) };
    let ty = match (l.kind, l.parent) {
        (LayerKind::Mask, _) => "mask",
        (LayerKind::Guide, _) => "guide",
        (LayerKind::Normal, Some(_)) => "masked",
        (LayerKind::Normal, None) => "normal",
    };
    json!({"layer": id.0, "type": ty, "masked_by": l.parent.map(|p| p.0)})
}

pub(crate) fn target(s: &Session, p: &Value) -> Result<(Option<LayerId>, u32)> {
    let layer = opt_u64(p, "layer")?.map(LayerId);
    let frame = opt_u32(p, "frame")?.unwrap_or(s.frame);
    if frame > MAX_FRAME {
        return Err(EngineError::Param(format!("param 'frame' must be at most {MAX_FRAME}")));
    }
    Ok((layer, frame))
}

/// Layers a range command acts on: `layers: [ids]`, else `layer`, else the active layer.
pub(crate) fn target_layers(s: &Session, p: &Value) -> Result<Vec<LayerId>> {
    let ids = match opt_u64_list(p, "layers")? {
        Some(v) if !v.is_empty() => v.into_iter().map(LayerId).collect(),
        Some(_) => return Err(EngineError::Param("param 'layers' must not be empty".into())),
        None => vec![opt_u64(p, "layer")?.map_or(s.layer, LayerId)],
    };
    let mut out: Vec<LayerId> = Vec::with_capacity(ids.len());
    for id in ids {
        if s.doc.timeline.layer(id).is_none() {
            return Err(EngineError::State(format!("no layer with id {}", id.0)));
        }
        if !out.contains(&id) {
            out.push(id);
        }
    }
    Ok(out)
}

/// Frames a range command spans (`count`, default 1).
pub(crate) fn count_param(p: &Value) -> Result<u32> {
    let n = opt_u32(p, "count")?.unwrap_or(1);
    if !(1..=10_000).contains(&n) {
        return Err(EngineError::Param("param 'count' must be between 1 and 10,000".into()));
    }
    Ok(n)
}

/// Shifts keyframes starting at or after `from` by `delta` frames.
pub(crate) fn shift_after(layer: &mut Layer, from: u32, delta: i64) {
    for k in layer.keyframes.iter_mut().filter(|k| k.start >= from).map(Arc::make_mut) {
        k.start = (i64::from(k.start) + delta).clamp(0, i64::from(MAX_FRAME)) as u32;
    }
}

/// F5 over a range: `count` frames on each target layer.
fn insert_frame(s: &mut Session, p: &Value) -> Result<Value> {
    let (_, frame) = target(s, p)?;
    let count = count_param(p)?;
    let mut frames = 0;
    for id in target_layers(s, p)? {
        let layer = s.layer_mut(Some(id))?;
        for _ in 0..count {
            insert_frame_at(layer, frame)?;
        }
        frames = frames.max(layer.end_frame());
    }
    Ok(json!({"frames": frames}))
}

fn insert_frame_at(layer: &mut Layer, frame: u32) -> Result<()> {
    if layer.end_frame() >= MAX_FRAME {
        return Err(EngineError::State("the timeline is at its maximum length".into()));
    }
    match layer.keyframe_index_at(frame) {
        Some(i) => {
            if let Some(k) = layer.keyframe_mut(i) {
                k.duration += 1;
            }
            shift_after(layer, frame + 1, 1);
        }
        None => match layer.keyframes.iter().rposition(|k| k.start <= frame) {
            // extend the previous keyframe up to and including `frame`
            Some(i) => {
                let next_start = layer.keyframes.get(i + 1).map(|k| k.start);
                if let Some(k) = layer.keyframe_mut(i) {
                    let want = frame + 1 - k.start;
                    k.duration = next_start.map_or(want, |n| want.min(n - k.start));
                }
            }
            None => layer.keyframes.insert(0, Arc::new(Keyframe::new(frame, 1))),
        },
    }
    layer.normalize();
    Ok(())
}

/// Shift+F5 over a range: removes up to `count` frames on each target layer. Layers that run
/// out of frames stop early; it fails only when nothing could be removed.
pub(crate) fn remove_frame(s: &mut Session, p: &Value) -> Result<Value> {
    let (_, frame) = target(s, p)?;
    let count = count_param(p)?;
    let mut removed = 0u32;
    let mut first_err = None;
    for id in target_layers(s, p)? {
        let layer = s.layer_mut(Some(id))?;
        for _ in 0..count {
            match remove_frame_at(layer, frame) {
                Ok(()) => removed += 1,
                Err(e) => {
                    first_err.get_or_insert(e);
                    break;
                }
            }
        }
    }
    match (removed, first_err) {
        (0, Some(e)) => Err(e),
        _ => Ok(json!({"removed": removed, "frames": s.doc.frame_count()})),
    }
}

fn remove_frame_at(layer: &mut Layer, frame: u32) -> Result<()> {
    let i = layer.keyframe_index_at(frame).ok_or_else(|| EngineError::State(format!("frame {} is empty on this layer", frame + 1)))?;
    let dur = layer.keyframes.get(i).map_or(1, |k| k.duration);
    if dur > 1 {
        if let Some(k) = layer.keyframe_mut(i) {
            k.duration -= 1;
        }
    } else {
        if layer.keyframes.len() == 1 {
            return Err(EngineError::State("cannot remove the layer's only frame".into()));
        }
        layer.keyframes.remove(i);
    }
    shift_after(layer, frame + 1, -1);
    layer.normalize();
    Ok(())
}

/// Inserts a keyframe at `frame` (F6 when `copy`, F7 otherwise). Inside a tween the new key
/// captures the tweened state, so the motion is unchanged.
pub(crate) fn insert_keyframe_at(layer: &mut Layer, frame: u32, copy: bool) -> Result<()> {
    if frame > MAX_FRAME {
        return Err(EngineError::Param(format!("frame must be at most {MAX_FRAME}")));
    }
    if layer.keyframe_starting_at(frame).is_some() {
        if !copy {
            // F7 on a keyframe clears it
            if let Some(k) = layer.keyframe_starting_at(frame).and_then(|i| layer.keyframe_mut(i)) {
                k.elements.clear();
                k.tween = None;
            }
        }
        return Ok(());
    }
    let elements = if copy { evaluate_layer_elements(layer, frame).map(|(_, e)| e.into_owned()) } else { None };
    match layer.keyframe_index_at(frame) {
        Some(i) => {
            let Some(k) = layer.keyframe_mut(i) else { return Ok(()) };
            let end = k.end();
            let tween = if copy { k.tween } else { None };
            k.duration = frame - k.start;
            let mut nk = Keyframe::new(frame, end - frame);
            nk.elements = elements.unwrap_or_default();
            nk.tween = tween;
            layer.keyframes.insert(i + 1, Arc::new(nk));
        }
        None => {
            // Past the end or in a gap: extend the previous key and copy its last state.
            let prev = layer.keyframes.iter().rposition(|k| k.start < frame);
            let mut nk = Keyframe::new(frame, 1);
            if let Some(i) = prev {
                let next_start = layer.keyframes.get(i + 1).map(|k| k.start);
                if let Some(k) = layer.keyframe_mut(i) {
                    if copy {
                        nk.elements = k.elements.clone();
                    }
                    k.duration = frame - k.start;
                }
                if let Some(n) = next_start {
                    nk.duration = n - frame;
                }
            }
            let pos = layer.keyframes.partition_point(|k| k.start < frame);
            layer.keyframes.insert(pos, Arc::new(nk));
        }
    }
    layer.normalize();
    Ok(())
}

fn insert_keyframe(s: &mut Session, p: &Value) -> Result<Value> {
    let (_, frame) = target(s, p)?;
    for id in target_layers(s, p)? {
        insert_keyframe_at(s.layer_mut(Some(id))?, frame, true)?;
    }
    Ok(json!({"keyframe": frame}))
}

fn insert_blank_keyframe(s: &mut Session, p: &Value) -> Result<Value> {
    let (_, frame) = target(s, p)?;
    for id in target_layers(s, p)? {
        insert_keyframe_at(s.layer_mut(Some(id))?, frame, false)?;
    }
    Ok(json!({"keyframe": frame}))
}

/// Shift+F6 over a range: clears every keyframe starting in `frame..frame + count` on each
/// target layer.
fn clear_keyframe(s: &mut Session, p: &Value) -> Result<Value> {
    let (_, frame) = target(s, p)?;
    let count = count_param(p)?;
    let mut cleared = 0;
    for id in target_layers(s, p)? {
        let layer = s.layer_mut(Some(id))?;
        let starts: Vec<u32> = layer.keyframes.iter().map(|k| k.start).filter(|&f| f >= frame && f - frame < count).collect();
        // Last first, so each removal merges into a span that is still there.
        for f in starts.into_iter().rev() {
            clear_keyframe_at(layer, f)?;
            cleared += 1;
        }
    }
    if cleared == 0 {
        return Err(EngineError::State(if count == 1 { format!("frame {} is not a keyframe", frame + 1) } else { "no keyframes in the selected frames".into() }));
    }
    Ok(json!({"cleared": cleared}))
}

/// Moves the keyframes starting in `from..=to` by `by` frames (dragging a frame selection).
/// Keyframes they land on are replaced; the span before a key moved right grows to fill the
/// frames it left, and the layer keeps its length (no frames are lost by moving left).
fn move_keyframes(s: &mut Session, p: &Value) -> Result<Value> {
    let from = opt_u32(p, "from")?.ok_or_else(|| EngineError::Param("param 'from' is required".into()))?;
    let to = opt_u32(p, "to")?.unwrap_or(from);
    let by = p.get("by").and_then(Value::as_i64).ok_or_else(|| EngineError::Param("param 'by' must be a whole number of frames".into()))?;
    if to < from || to > MAX_FRAME || by.unsigned_abs() > u64::from(MAX_FRAME) {
        return Err(EngineError::Param(format!("frames must be in order and within {MAX_FRAME}")));
    }
    let layer_by = match p.get("layer_by") {
        None | Some(Value::Null) => 0,
        Some(v) => v.as_i64().ok_or_else(|| EngineError::Param("param 'layer_by' must be a whole number of layers".into()))?,
    };
    if layer_by != 0 {
        return move_across_layers(s, p, from, to, by, layer_by);
    }
    let mut moved = 0;
    for id in target_layers(s, p)? {
        moved += move_keys(s.layer_mut(Some(id))?, from, to, by)?;
    }
    Ok(json!({"moved": moved}))
}

/// Dragging keyframes onto other layers (`layer_by` rows down, negative = up): the frames are
/// lifted out of every source layer first (leaving blank keyframes, as in Animate), then
/// placed on the target layers, replacing what was there; frames after the landing range
/// keep their content. Lifting everything before placing anything makes overlapping row
/// ranges (dragging two rows down by one) come out right.
fn move_across_layers(s: &mut Session, p: &Value, from: u32, to: u32, by: i64, layer_by: i64) -> Result<Value> {
    let order: Vec<LayerId> = s.doc.timeline.layers.iter().map(|l| l.id).collect();
    let mut rows: Vec<usize> = target_layers(s, p)?.iter().filter_map(|id| order.iter().position(|o| o == id)).collect();
    rows.sort_unstable();
    let mut plan = Vec::with_capacity(rows.len());
    for &r in &rows {
        let t = i64::try_from(r).unwrap_or(i64::MAX).checked_add(layer_by).filter(|t| *t >= 0 && (*t as usize) < order.len());
        let Some(t) = t else { return Err(EngineError::State("there is no layer there to move the frames to".into())) };
        let (Some(&src), Some(&dst)) = (order.get(r), order.get(t as usize)) else { continue };
        plan.push((src, dst));
    }
    // 1. Lift.
    let mut lifted: Vec<(LayerId, Vec<Keyframe>)> = Vec::new();
    for &(src, dst) in &plan {
        let layer = s.layer_mut(Some(src))?;
        let keys: Vec<Keyframe> = layer.keyframes.iter().filter(|k| k.start >= from && k.start <= to).map(|k| (**k).clone()).collect();
        if let (Some(first), Some(last)) = (keys.first().map(|k| k.start), keys.last().map(Keyframe::end)) {
            layer.keyframes.retain(|k| !(k.start >= from && k.start <= to));
            // The frames stay; their content leaves.
            layer.keyframes.push(Arc::new(Keyframe::new(first, last - first)));
            layer.normalize();
        }
        lifted.push((dst, keys));
    }
    // 2. Place.
    let mut moved = 0;
    for (dst, keys) in lifted {
        let (Some(first), Some(last)) = (keys.first().map(|k| k.start), keys.last().map(Keyframe::end)) else { continue };
        let (a, b) = (i64::from(first) + by, i64::from(last) + by);
        if a < 0 || b > i64::from(MAX_FRAME) + 1 {
            return Err(EngineError::State(if a < 0 { "keyframes cannot move before frame 1".into() } else { "keyframes would move past the end of the timeline".into() }));
        }
        let (a, b) = (a as u32, b as u32);
        let layer = s.layer_mut(Some(dst))?;
        // Cut the target at both ends of the landing range so what lies outside it stays.
        if layer.keyframe_index_at(a).is_some_and(|i| layer.keyframes.get(i).is_some_and(|k| k.start < a)) {
            insert_keyframe_at(layer, a, true)?;
        }
        if b < layer.end_frame() && layer.keyframe_index_at(b).is_some() {
            insert_keyframe_at(layer, b, true)?;
        }
        layer.keyframes.retain(|k| !(k.start >= a && k.start < b));
        for mut k in keys {
            k.start = (i64::from(k.start) + by) as u32;
            layer.keyframes.push(Arc::new(k));
            moved += 1;
        }
        layer.normalize();
    }
    Ok(json!({"moved": moved}))
}

fn move_keys(layer: &mut Layer, from: u32, to: u32, by: i64) -> Result<usize> {
    if by == 0 {
        return Ok(0);
    }
    let end = layer.end_frame();
    let (mut moving, mut staying): (Vec<Arc<Keyframe>>, Vec<Arc<Keyframe>>) = std::mem::take(&mut layer.keyframes).into_iter().partition(|k| k.start >= from && k.start <= to);
    if moving.is_empty() {
        layer.keyframes = staying;
        return Ok(0);
    }
    let first = moving.first().map_or(0, |k| i64::from(k.start));
    let last_end = moving.last().map_or(0, |k| i64::from(k.end()));
    if first + by < 0 || last_end + by > i64::from(MAX_FRAME) + 1 {
        layer.keyframes = moving.into_iter().chain(staying).collect();
        layer.normalize();
        return Err(EngineError::State(if first + by < 0 { "keyframes cannot move before frame 1".into() } else { "keyframes would move past the end of the timeline".into() }));
    }
    for k in moving.iter_mut().map(Arc::make_mut) {
        k.start = (i64::from(k.start) + by) as u32;
    }
    let landed: Vec<u32> = moving.iter().map(|k| k.start).collect();
    staying.retain(|k| !landed.contains(&k.start));
    let n = moving.len();
    let new_end = end.max((last_end + by) as u32);
    let mut keys: Vec<Arc<Keyframe>> = staying.into_iter().chain(moving).collect();
    keys.sort_by_key(|k| k.start);
    // Each span runs to the next key; the last runs to the layer's end.
    let starts: Vec<u32> = keys.iter().map(|k| k.start).collect();
    for (i, k) in keys.iter_mut().enumerate() {
        let stop = starts.get(i + 1).copied().unwrap_or(new_end);
        let want = stop.saturating_sub(k.start).max(1);
        if k.duration != want {
            Arc::make_mut(k).duration = want;
        }
    }
    layer.keyframes = keys;
    layer.normalize();
    Ok(n)
}

fn clear_keyframe_at(layer: &mut Layer, frame: u32) -> Result<()> {
    let i = layer.keyframe_starting_at(frame).ok_or_else(|| EngineError::State(format!("frame {} is not a keyframe", frame + 1)))?;
    if i == 0 {
        if let Some(k) = layer.keyframe_mut(0) {
            k.elements.clear();
            k.tween = None;
        }
    } else {
        let removed = layer.keyframes.remove(i);
        if let Some(prev) = layer.keyframe_mut(i - 1) {
            prev.duration = removed.end() - prev.start;
        }
    }
    layer.normalize();
    Ok(())
}

fn keyframe_mut<'a>(s: &'a mut Session, p: &Value) -> Result<&'a mut Keyframe> {
    let (lid, frame) = target(s, p)?;
    let layer = s.layer_mut(lid)?;
    let i = layer.keyframe_index_at(frame).ok_or_else(|| EngineError::State(format!("frame {} is empty on this layer", frame + 1)))?;
    layer.keyframe_mut(i).ok_or_else(|| EngineError::State("keyframe vanished".into()))
}

fn create_tween(s: &mut Session, p: &Value) -> Result<Value> {
    tween_of_kind(s, p, TweenKind::Classic)
}

/// Create Shape Tween: like a classic tween, and shapes of any structure morph into each other.
fn create_shape_tween(s: &mut Session, p: &Value) -> Result<Value> {
    tween_of_kind(s, p, TweenKind::Shape)
}

fn tween_of_kind(s: &mut Session, p: &Value, kind: TweenKind) -> Result<Value> {
    let k = keyframe_mut(s, p)?;
    // Switching kinds keeps the easing the span already had, unless one is given.
    let easing = match opt_easing(p, "easing")? {
        Some(e) => e,
        None => k.tween.map(|t| t.easing).unwrap_or_default(),
    };
    k.tween = Some(Tween { easing, kind });
    let kind = if kind == TweenKind::Shape { "shape" } else { "classic" };
    Ok(json!({"tween": k.start, "easing": easing.label(), "kind": kind}))
}

fn remove_tween(s: &mut Session, p: &Value) -> Result<Value> {
    let k = keyframe_mut(s, p)?;
    k.tween = None;
    Ok(json!({"removed": k.start}))
}

fn set_easing(s: &mut Session, p: &Value) -> Result<Value> {
    let easing = opt_easing(p, "easing")?.ok_or_else(|| EngineError::Param("param 'easing' is required".into()))?;
    let k = keyframe_mut(s, p)?;
    let t = k.tween.as_mut().ok_or_else(|| EngineError::State("this span has no tween; create one first".into()))?;
    t.easing = easing;
    Ok(json!({"easing": easing.label()}))
}

fn set_label(s: &mut Session, p: &Value) -> Result<Value> {
    let label = str_req(p, "label")?.chars().take(256).collect::<String>();
    let k = keyframe_mut(s, p)?;
    k.label = label;
    Ok(json!({"label": k.label}))
}

fn goto(s: &mut Session, p: &Value) -> Result<Value> {
    let f = opt_u32(p, "frame")?.ok_or_else(|| EngineError::Param("param 'frame' is required".into()))?;
    s.set_frame(f);
    Ok(json!({"frame": s.frame}))
}

fn step(s: &mut Session, d: i64) -> Result<Value> {
    let n = i64::from(s.frame_count().max(1));
    let f = (i64::from(s.frame) + d).rem_euclid(n);
    s.set_frame(f as u32);
    Ok(json!({"frame": s.frame}))
}

pub(crate) fn new_layer(s: &mut Session, p: &Value) -> Result<Value> {
    let n = s.doc.timeline.layers.len() + 1;
    let name = opt_str(p, "name")?.map(|x| x.chars().take(128).collect()).unwrap_or_else(|| format!("Layer {n}"));
    if s.doc.timeline.layers.len() >= 1000 {
        return Err(EngineError::State("too many layers (limit 1000)".into()));
    }
    let id = LayerId(s.doc.alloc_id());
    let mut layer = Layer::new(id, name);
    // Match the timeline length so the new layer spans the existing frames.
    if let Some(k) = layer.keyframe_mut(0) {
        k.duration = s.doc.frame_count().max(1);
    }
    let palette = [0x4fa3ff, 0xff6b4a, 0x66bb6a, 0xffd54a, 0xba68c8, 0x4dd0e1, 0xff8a65];
    layer.color = anicraft_doc::Rgba::hex(palette[n % palette.len()]);
    let at = s.active_layer_index().unwrap_or(0);
    // Added above a masked layer, it lands inside the mask's group, so it is masked too.
    layer.parent = s.doc.timeline.layers.get(at).and_then(|l| l.parent);
    s.doc.timeline.layers.insert(at, Arc::new(layer));
    s.layer = id;
    s.selection.clear();
    Ok(json!({"layer": id.0}))
}

fn delete_layer(s: &mut Session, p: &Value) -> Result<Value> {
    let id = opt_u64(p, "layer")?.map_or(s.layer, LayerId);
    if s.doc.timeline.layers.len() <= 1 {
        return Err(EngineError::State("a document needs at least one layer".into()));
    }
    let i = s.doc.timeline.layers.iter().position(|l| l.id == id).ok_or_else(|| EngineError::State(format!("no layer with id {}", id.0)))?;
    s.doc.timeline.layers.remove(i);
    // Layers masked by a deleted mask become normal layers.
    s.doc.timeline.repair_masks();
    if s.layer == id {
        let j = i.min(s.doc.timeline.layers.len() - 1);
        s.layer = s.doc.timeline.layers.get(j).map_or(LayerId(0), |l| l.id);
        s.selection.clear();
    }
    Ok(json!({"deleted": id.0}))
}

fn rename_layer(s: &mut Session, p: &Value) -> Result<Value> {
    let name: String = str_req(p, "name")?.chars().take(128).collect();
    if name.trim().is_empty() {
        return Err(EngineError::Param("layer name must not be empty".into()));
    }
    let lid = opt_u64(p, "layer")?.map(LayerId);
    let layer = s.layer_mut(lid)?;
    layer.name = name;
    Ok(json!({"name": layer.name}))
}

fn toggle(s: &mut Session, p: &Value, f: fn(&mut Layer)) -> Result<Value> {
    let lid = opt_u64(p, "layer")?.map(LayerId);
    let layer = s.layer_mut(lid)?;
    f(layer);
    Ok(json!({"visible": layer.visible, "locked": layer.locked, "outline": layer.outline}))
}

fn select_layer(s: &mut Session, p: &Value) -> Result<Value> {
    let id = if let Some(id) = opt_u64(p, "layer")? {
        LayerId(id)
    } else if let Some(i) = opt_u64(p, "index")? {
        usize::try_from(i).ok().and_then(|i| s.doc.timeline.layers.get(i)).map(|l| l.id).ok_or_else(|| EngineError::Param("layer index out of range".into()))?
    } else {
        return Err(EngineError::Param("pass {\"layer\": id} or {\"index\": n}".into()));
    };
    if s.doc.timeline.layer(id).is_none() {
        return Err(EngineError::State(format!("no layer with id {}", id.0)));
    }
    if s.layer != id {
        s.layer = id;
        s.selection.clear();
    }
    Ok(json!({"layer": id.0}))
}

/// Moves a layer one step. A mask layer moves with its masked layers and steps over a whole
/// neighbouring mask group. Any other layer swaps with its neighbour and then belongs to the
/// mask group it landed in, if any: moving a layer in under a mask masks it, moving it out
/// frees it.
fn move_layer(s: &mut Session, p: &Value, d: i64) -> Result<Value> {
    let id = opt_u64(p, "layer")?.map_or(s.layer, LayerId);
    let tl = &mut s.doc.timeline;
    let i = tl.layers.iter().position(|l| l.id == id).ok_or_else(|| EngineError::State(format!("no layer with id {}", id.0)))?;
    let n = tl.layers.len();
    if tl.layers.get(i).is_some_and(|l| l.kind == LayerKind::Mask) {
        let block = tl.group_range(i);
        // The neighbouring unit: a layer, or a whole mask group.
        let unit = if d < 0 {
            let Some(a) = i.checked_sub(1) else { return Ok(json!({"index": i})) };
            let top = match tl.layers.get(a).and_then(|l| l.parent) {
                Some(m) => tl.layers.iter().position(|l| l.id == m).unwrap_or(a),
                None => a,
            };
            top..i
        } else {
            if block.end >= n {
                return Ok(json!({"index": i}));
            }
            tl.group_range(block.end)
        };
        let (lo, hi) = (unit.start.min(block.start), unit.end.max(block.end));
        let Some(span) = tl.layers.get_mut(lo..hi) else { return Ok(json!({"index": i})) };
        // Rotate so the block and the unit trade places.
        if d < 0 { span.rotate_left(unit.len()) } else { span.rotate_right(unit.len()) }
        let at = if d < 0 { unit.start } else { block.start + unit.len() };
        tl.repair_masks();
        return Ok(json!({"index": at}));
    }
    let j = i as i64 + d;
    if j < 0 || j as usize >= n {
        return Ok(json!({"index": i}));
    }
    let j = j as usize;
    tl.layers.swap(i, j);
    let join = tl.mask_above(j).filter(|_| tl.layers.get(j).is_some_and(|l| l.kind == LayerKind::Normal));
    if let Some(l) = tl.layers.get_mut(j)
        && l.parent != join
    {
        Arc::make_mut(l).parent = join;
    }
    tl.repair_masks();
    Ok(json!({"index": j}))
}
