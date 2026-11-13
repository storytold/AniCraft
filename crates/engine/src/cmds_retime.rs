//! Retiming tweens (ADR 0083): property keys dragged in time in the Motion Editor, a span's
//! length changed with its keys stretched in proportion, and Split / Join Motion.
//!
//! Moving whole spans (and Alt-drag copies) is `timeline.move_spans`, beside the other frame
//! moves in `cmds_timeline`.

use crate::cmds_timeline::{shift_after, target};
use crate::params::*;
use crate::{CommandSpec, EngineError, MAX_FRAME, Result, Session};
use anicraft_doc::{Element, KeyRef, Keyframe, Layer, LayerKind, Property};
use serde_json::{Value, json};
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c("motion.move_keys", "Move Property Keyframes", "", None, "{keys: [{property, frame}, ..] (frames as motion.info lists them), by (frames; negative = earlier), layer?, frame? (in the span; default: the first key's)}: values and eases travel with their keys; no key moves outside the span or onto or past a key of its property that isn't moving (the move stops there); a pinned path point's X and Y keys move together → {by, keys}", true, move_keys as crate::RunFn),
        c("timeline.resize_span", "Span Length", "", None, "{frames (the new length), layer?, frame?, scale_keys?: false (keep a motion tween's keys where they are; default: they stretch in proportion, as dragging the span's end in Animate)}: later frames move with the end, as with F5 / Shift+F5 → {start, frames, scaled}", true, resize_span),
        c("motion.split", "Split Motion", "Modify/Timeline", None, "{layer?, frame?}: the motion tween becomes two at the frame, each moving as before → {first, second}", true, split),
        c("motion.join", "Join Motion", "Modify/Timeline", None, "{layer?, frame?, to? (the last frame of the spans to join; default: the span after the one at the frame)}: adjacent motion tweens of one object become one, keeping every frame as it was → {start, frames, joined}", true, join),
    ]
}

/// The most keys one `motion.move_keys` call names.
const MAX_MOVED: usize = 100_000;

fn keys_param(p: &Value) -> Result<Vec<(Property, u32)>> {
    let list = p.get("keys").and_then(Value::as_array).ok_or_else(|| EngineError::Param("param 'keys' is required: a list of {property, frame}".into()))?;
    if list.is_empty() {
        return Err(EngineError::Param("param 'keys' lists no key".into()));
    }
    if list.len() > MAX_MOVED {
        return Err(EngineError::Param(format!("param 'keys' lists more than {MAX_MOVED} keys")));
    }
    let mut out = Vec::with_capacity(list.len());
    for k in list {
        let name = opt_str(k, "property")?.ok_or_else(|| EngineError::Param("each key needs a 'property'".into()))?;
        let prop = Property::from_key(name).ok_or_else(|| EngineError::Param(format!("unknown property '{name}'")))?;
        let frame = opt_u32(k, "frame")?.ok_or_else(|| EngineError::Param("each key needs a 'frame'".into()))?;
        if frame > MAX_FRAME {
            return Err(EngineError::Param(format!("key frames must be at most {MAX_FRAME}")));
        }
        out.push((prop, frame));
    }
    Ok(out)
}

fn move_keys(s: &mut Session, p: &Value) -> Result<Value> {
    let keys = keys_param(p)?;
    let by = p.get("by").and_then(Value::as_i64).ok_or_else(|| EngineError::Param("param 'by' must be a whole number of frames".into()))?;
    if by.unsigned_abs() > u64::from(MAX_FRAME) {
        return Err(EngineError::Param(format!("param 'by' must be within ±{MAX_FRAME}")));
    }
    let lid = opt_u64(p, "layer")?.map(anicraft_doc::LayerId);
    let at = match opt_u32(p, "frame")? {
        Some(f) if f > MAX_FRAME => return Err(EngineError::Param(format!("param 'frame' must be at most {MAX_FRAME}"))),
        Some(f) => f,
        None => keys.first().map_or(s.frame, |k| k.1),
    };
    let layer = s.layer_mut(lid)?;
    let i = layer.keyframe_index_at(at).ok_or_else(|| EngineError::State(format!("frame {} is empty on this layer", at + 1)))?;
    let kf = layer.keyframe_mut(i).ok_or_else(|| EngineError::State("keyframe vanished".into()))?;
    let (start, last) = (kf.start, kf.duration.saturating_sub(1));
    let m = kf.motion.as_mut().ok_or_else(|| EngineError::State(format!("frame {} is not in a motion tween", at + 1)))?;
    let mut picked: Vec<KeyRef> = Vec::with_capacity(keys.len());
    for (prop, f) in keys {
        let off = f.checked_sub(start).filter(|o| m.track(prop).is_some_and(|t| t.key_at(*o).is_some()));
        let Some(off) = off else { return Err(EngineError::State(format!("there is no {} keyframe at frame {}", prop.label(), f + 1))) };
        picked.push((prop, off));
    }
    let (moved, now) = m.move_keys(&picked, by, last);
    let keys: Vec<Value> = now.iter().map(|(p, f)| json!({"property": p.key(), "frame": start + f})).collect();
    Ok(json!({"by": moved, "keys": keys}))
}

fn resize_span(s: &mut Session, p: &Value) -> Result<Value> {
    let frames = opt_u32(p, "frames")?.ok_or_else(|| EngineError::Param("param 'frames' (the span's new length) is required".into()))?;
    if !(1..=MAX_FRAME).contains(&frames) {
        return Err(EngineError::Param(format!("param 'frames' must be between 1 and {MAX_FRAME}")));
    }
    let scale = opt_bool(p, "scale_keys")?.unwrap_or(true);
    let (lid, frame) = target(s, p)?;
    let layer = s.layer_mut(lid)?;
    if layer.kind == LayerKind::Folder {
        return Err(EngineError::State("a folder holds layers, not frames; pick a layer inside it".into()));
    }
    let i = layer.keyframe_index_at(frame).ok_or_else(|| EngineError::State(format!("frame {} is empty on this layer", frame + 1)))?;
    let (start, old, end) = layer.keyframes.get(i).map(|k| (k.start, k.duration, k.end())).ok_or_else(|| EngineError::State("keyframe vanished".into()))?;
    let grow = i64::from(frames) - i64::from(old);
    if grow > 0 && i64::from(layer.end_frame()) + grow > i64::from(MAX_FRAME) + 1 {
        return Err(EngineError::State("the timeline is at its maximum length".into()));
    }
    let scaled = resize(layer, i, frames, scale);
    // Later frames move with the end, as with F5 / Shift+F5.
    shift_after(layer, end, grow);
    layer.normalize();
    Ok(json!({"start": start, "frames": frames, "scaled": scaled}))
}

/// Sets keyframe `i`'s length to `frames`, stretching its motion tween's keys when `scale`.
/// Whether keys were stretched.
fn resize(layer: &mut Layer, i: usize, frames: u32, scale: bool) -> bool {
    let Some(k) = layer.keyframe_mut(i) else { return false };
    let old = k.duration;
    k.duration = frames;
    match k.motion.as_mut() {
        Some(m) if scale && old > 1 && frames > 1 && old != frames => {
            m.stretch(old, frames);
            true
        }
        _ => false,
    }
}

fn split(s: &mut Session, p: &Value) -> Result<Value> {
    let (lid, frame) = target(s, p)?;
    let layer = s.layer_mut(lid)?;
    let k = layer.keyframe_at(frame).ok_or_else(|| EngineError::State(format!("frame {} is empty on this layer", frame + 1)))?;
    if k.motion.is_none() {
        return Err(EngineError::State(format!("frame {} is not in a motion tween", frame + 1)));
    }
    let first = k.start;
    if frame == first {
        return Err(EngineError::State(format!("frame {} is the tween's first frame: split it at a later frame", frame + 1)));
    }
    // The cut a copied keyframe makes inside a span keeps the motion in both halves.
    crate::cmds_timeline::insert_keyframe_at(layer, frame, true)?;
    Ok(json!({"first": first, "second": frame}))
}

fn join(s: &mut Session, p: &Value) -> Result<Value> {
    let (lid, frame) = target(s, p)?;
    let to = opt_u32(p, "to")?;
    let layer = s.layer_mut(lid)?;
    let i = layer.keyframe_index_at(frame).ok_or_else(|| EngineError::State(format!("frame {} is empty on this layer", frame + 1)))?;
    let (start, end) = layer.keyframes.get(i).filter(|k| k.motion.is_some()).map(|k| (k.start, k.end())).ok_or_else(|| EngineError::State(format!("frame {} is not in a motion tween", frame + 1)))?;
    let to = to.unwrap_or(end);
    if to < end {
        return Err(EngineError::State("there is only one motion tween in those frames: Join Motion joins it with the ones after it".into()));
    }
    let mut joined = 0;
    while let Some(next) = layer.keyframes.get(i + 1).filter(|n| n.start <= to) {
        let next = Arc::clone(next);
        let Some(kf) = layer.keyframes.get(i) else { break };
        let merged = join_pair(kf, &next)?;
        layer.keyframes.remove(i + 1);
        if let Some(slot) = layer.keyframes.get_mut(i) {
            *slot = Arc::new(merged);
        }
        joined += 1;
    }
    if joined == 0 {
        return Err(EngineError::State(format!("no motion tween starts right after frame {}", end)));
    }
    layer.normalize();
    let frames = layer.keyframes.get(i).map_or(0, |k| k.duration);
    Ok(json!({"start": start, "frames": frames, "joined": joined + 1}))
}

/// The span `a` and the span `b` right after it, as one motion tween span.
fn join_pair(a: &Keyframe, b: &Keyframe) -> Result<Keyframe> {
    let shown = b.start + 1;
    if b.start != a.end() {
        return Err(EngineError::State(format!("the span at frame {shown} doesn't start where the tween before it ends")));
    }
    let (Some(ma), Some(mb)) = (&a.motion, &b.motion) else {
        return Err(EngineError::State(format!("the span at frame {shown} has no motion tween: Join Motion joins motion tweens")));
    };
    if b.sound.is_some() || b.script.is_some() {
        return Err(EngineError::State(format!("the keyframe at frame {shown} has a sound or actions, which joining would drop: move them first")));
    }
    let lost = || EngineError::State("a motion tween's object is no longer on its keyframe".into());
    let ia = ma.target_index(&a.elements).ok_or_else(lost)?;
    let ib = mb.target_index(&b.elements).ok_or_else(lost)?;
    let (ta, tb) = (a.elements.get(ia).ok_or_else(lost)?, b.elements.get(ib).ok_or_else(lost)?);
    if ta.kind != tb.kind {
        return Err(EngineError::State(format!("the tween at frame {shown} animates another object: Join Motion joins the tweens of one object")));
    }
    let others = |k: &Keyframe, t: usize| -> Vec<Element> { k.elements.iter().enumerate().filter(|(j, _)| *j != t).map(|(_, e)| (**e).clone()).collect() };
    if others(a, ia) != others(b, ib) {
        return Err(EngineError::State(format!("the span at frame {shown} holds other artwork than the tween before it: Join Motion joins spans that differ only in the tweened object")));
    }
    let m = ma.join(ta, a.duration, mb, tb).ok_or_else(|| EngineError::State("Join Motion can't join tweens with motion paths yet (unless one was split from the other): remove a path first".into()))?;
    let mut k = a.clone();
    k.duration = a.duration.saturating_add(b.duration);
    k.motion = Some(m);
    if k.label.is_empty() {
        k.label.clone_from(&b.label);
    }
    Ok(k)
}
