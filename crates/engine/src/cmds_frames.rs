//! Copy, cut and paste frames (Edit › Timeline).
//!
//! Design notes (principal engineer / senior designer review):
//! * Frames have their own clipboard, separate from copied artwork, as in Animate: copying
//!   frames never replaces copied shapes, and the reverse.
//! * A copy is cut exactly at the selection edges by splitting a scratch copy of the layer with
//!   the same keyframe logic as F6, so a span that starts before the selection, or is
//!   mid-tween, comes out as it looks there.
//! * Paste inserts (later frames move right), which is what Animate's Paste Frames does and so
//!   what its users expect. Several copied layers land on the target layer and the ones below
//!   it; missing layers are created. Pasted elements get fresh ids, mapped consistently within
//!   one paste so tweens between pasted keys still pair up, and symbols they need come along
//!   (also from another document).

use crate::cmds_modify::{collect_symbols, import_symbols};
use crate::cmds_timeline::{count_param, insert_keyframe_at, remove_frame, shift_after, target, target_layers};
use crate::{CommandSpec, EngineError, MAX_FRAME, Result, Session};
use anicraft_doc::{Element, ElementId, Keyframe, Layer, LayerId, Symbol};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c("timeline.cut_frames", "Cut Frames", "Edit/Timeline", Some("Ctrl+Alt+X"), "{layer? | layers?: [..], frame?, count?}", true, cut_frames as crate::RunFn),
        c("timeline.copy_frames", "Copy Frames", "Edit/Timeline", Some("Ctrl+Alt+C"), "{layer? | layers?: [..], frame?, count?}", false, copy_frames),
        c("timeline.paste_frames", "Paste Frames", "Edit/Timeline", Some("Ctrl+Alt+V"), "{layer?, frame?} (top layer and first frame; default: active layer, playhead)", true, paste_frames),
    ]
}

/// One copied layer's frames; keyframe starts are relative to the first copied frame.
#[derive(Clone, Debug)]
pub struct FrameSlice {
    pub name: String,
    pub keys: Vec<Arc<Keyframe>>,
}

/// Copied frames: one slice per layer (top to bottom), `count` frames wide.
#[derive(Clone, Debug, Default)]
pub struct FrameClipboard {
    pub layers: Vec<FrameSlice>,
    pub count: u32,
    pub symbols: Vec<Arc<Symbol>>,
    /// Sounds the copied keyframes play, so they paste into other documents too.
    pub sounds: Vec<Arc<anicraft_doc::Sound>>,
}

impl FrameClipboard {
    pub fn is_empty(&self) -> bool {
        self.layers.iter().all(|l| l.keys.is_empty())
    }
}

/// The keyframes covering `frame..frame + count` of `layer`, cut at both edges.
fn slice(layer: &Layer, frame: u32, count: u32) -> Result<Vec<Arc<Keyframe>>> {
    let end = layer.end_frame();
    if frame >= end {
        return Ok(Vec::new());
    }
    let stop = frame.saturating_add(count).min(end);
    let mut l = layer.clone();
    if l.keyframe_index_at(frame).is_some() {
        insert_keyframe_at(&mut l, frame, true)?;
    }
    if stop < end && l.keyframe_index_at(stop).is_some() {
        insert_keyframe_at(&mut l, stop, true)?;
    }
    Ok(l.keyframes
        .iter()
        .filter(|k| k.start >= frame && k.start < stop)
        .map(|k| {
            let mut k = (**k).clone();
            k.duration = k.duration.min(stop - k.start);
            k.start -= frame;
            Arc::new(k)
        })
        .collect())
}

fn copy_frames(s: &mut Session, p: &Value) -> Result<Value> {
    let (_, frame) = target(s, p)?;
    let count = count_param(p)?;
    let mut ids = target_layers(s, p)?;
    // Top to bottom, whatever order the params gave.
    let order: Vec<LayerId> = s.timeline().layers.iter().map(|l| l.id).collect();
    ids.sort_by_key(|id| order.iter().position(|o| o == id));
    let mut layers = Vec::with_capacity(ids.len());
    for id in ids {
        let l = s.timeline().layer(id).ok_or_else(|| EngineError::State(format!("no layer with id {}", id.0)))?;
        layers.push(FrameSlice { name: l.name.clone(), keys: slice(l, frame, count)? });
    }
    let clip = FrameClipboard { layers, count, symbols: Vec::new(), sounds: Vec::new() };
    if clip.is_empty() {
        return Err(EngineError::State("no frames there to copy".into()));
    }
    let elements: Vec<Element> = clip.layers.iter().flat_map(|l| l.keys.iter().flat_map(|k| k.elements.iter().map(|e| (**e).clone()))).collect();
    let symbols = collect_symbols(s, &elements);
    let n = clip.layers.len();
    let mut sounds: Vec<Arc<anicraft_doc::Sound>> = Vec::new();
    for id in clip.layers.iter().flat_map(|l| l.keys.iter().filter_map(|k| k.sound.as_ref().map(|f| f.sound))) {
        if let Some(x) = s.doc.sounds.iter().find(|x| x.id == id) && !sounds.iter().any(|y| y.id == id) {
            sounds.push(x.clone());
        }
    }
    s.frame_clipboard = FrameClipboard { symbols, sounds, ..clip };
    Ok(json!({"copied": count, "layers": n}))
}

fn cut_frames(s: &mut Session, p: &Value) -> Result<Value> {
    copy_frames(s, p)?;
    remove_frame(s, p)?;
    Ok(json!({"cut": s.frame_clipboard.count, "layers": s.frame_clipboard.layers.len()}))
}

fn paste_frames(s: &mut Session, p: &Value) -> Result<Value> {
    if s.frame_clipboard.is_empty() {
        return Err(EngineError::State("no frames have been copied".into()));
    }
    let (top, frame) = target(s, p)?;
    let top = top.unwrap_or(s.layer);
    let clip = s.frame_clipboard.clone();
    if frame.saturating_add(clip.count) > MAX_FRAME || s.frame_count().saturating_add(clip.count) > MAX_FRAME {
        return Err(EngineError::State("the timeline would grow past its maximum length".into()));
    }
    let mut row = s.timeline().layers.iter().position(|l| l.id == top).ok_or_else(|| EngineError::State(format!("no layer with id {}", top.0)))?;
    import_symbols(s, &clip.symbols);
    for x in &clip.sounds {
        if s.doc.sound(x.id).is_none() {
            s.doc.next_id = s.doc.next_id.max(x.id.0.saturating_add(1));
            s.doc.sounds.push(x.clone());
        }
    }
    let mut remap: HashMap<ElementId, ElementId> = HashMap::new();
    let mut pasted = Vec::new();
    for sl in &clip.layers {
        // Missing layers below are created, named after the copied ones.
        if s.timeline().layers.get(row).is_none() {
            if s.timeline().layers.len() >= 1000 {
                return Err(EngineError::State("too many layers (limit 1000)".into()));
            }
            let id = LayerId(s.doc.alloc_id());
            let mut l = Layer::new(id, sl.name.clone());
            if !sl.keys.is_empty() {
                // The pasted keys are its frames (no stray blank frame at 1).
                l.keyframes.clear();
            }
            crate::editing::edited_mut(&mut s.doc, s.scene, &s.editing)?.layers.push(Arc::new(l));
        }
        let mut keys = Vec::with_capacity(sl.keys.len());
        for k in &sl.keys {
            let mut k = (**k).clone();
            k.start += frame;
            for e in &mut k.elements {
                let old = e.id;
                let new = *remap.entry(old).or_insert_with(|| ElementId(s.doc.alloc_id()));
                Arc::make_mut(e).id = new;
            }
            if let Some(m) = k.motion.as_mut() {
                m.target = *remap.entry(m.target).or_insert_with(|| ElementId(s.doc.alloc_id()));
            }
            keys.push(Arc::new(k));
        }
        let Some(layer) = crate::editing::edited_mut(&mut s.doc, s.scene, &s.editing).ok().and_then(|tl| tl.layers.get_mut(row).map(Arc::make_mut)) else { continue };
        crate::cmds_camera::keys_fit(layer.kind, keys.iter().map(|k| &**k))?;
        if layer.keyframe_index_at(frame).is_some() {
            insert_keyframe_at(layer, frame, true)?;
        }
        shift_after(layer, frame, i64::from(clip.count));
        layer.keyframes.extend(keys);
        layer.normalize();
        pasted.push(layer.id.0);
        row += 1;
    }
    s.selection.clear();
    Ok(json!({"pasted": clip.count, "layers": pasted}))
}
