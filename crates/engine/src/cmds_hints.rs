//! Shape hints (ADR 0023): lettered points that tell a shape tween which point of the start
//! shape becomes which point of the end shape.
//!
//! Every command finds its tween the way the stage does ([`anicraft_doc::Layer::hint_tween_at`]):
//! `frame` is any frame of the shape tween's span, or the first frame of the keyframe it ends
//! on (that frame shows and edits the hints' end side).

use crate::cmds_timeline::target;
use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::{HintSide, Keyframe, Layer, LayerId, MAX_SHAPE_HINTS, ShapeHint, hint_index, hint_letter, nearest_outline, placed_on, shapes_centre};
use anicraft_geom::Vec2;
use serde_json::{Value, json};

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c("tween.add_hint", "Add Shape Hint", "Modify/Shape", Some("Ctrl+Shift+H"), "{layer?, frame? (in the shape tween, or the keyframe it ends on)} → the new hint's letter; it starts at the shapes' centre, unplaced", true, add_hint as crate::RunFn),
        c("tween.move_hint", "Move Shape Hint", "", None, "{hint: a..z, x, y, side?: start|end (default: the side the frame shows), layer?, frame?} (snaps to the nearest shape edge on that keyframe)", true, move_hint),
        c("tween.remove_hint", "Remove Shape Hint", "", None, "{hint: a..z, layer?, frame?} (later letters move down one)", true, remove_hint),
        c("tween.clear_hints", "Remove All Hints", "Modify/Shape", None, "{layer?, frame?}", true, clear_hints),
        c("tween.hints", "Shape Hints", "", None, "{layer?, frame?} → the shape tween's hints: letter, start and end points, and whether each is on an edge", false, list_hints),
    ]
}

/// The layer, the index of the shape tween's keyframe and the side `frame` shows.
fn find(s: &Session, p: &Value) -> Result<(LayerId, usize, HintSide)> {
    let (lid, frame) = target(s, p)?;
    let lid = lid.unwrap_or(s.layer);
    let tl = s.timeline();
    let layer = tl.layer(lid).ok_or_else(|| EngineError::State(format!("no layer with id {}", lid.0)))?;
    let (i, side) = layer.hint_tween_at(frame).ok_or_else(|| EngineError::State(format!("frame {} is not in a shape tween; select the span and use Insert › Create Shape Tween", frame + 1)))?;
    Ok((lid, i, side))
}

/// The same, for an edit: refuses locked layers.
fn find_mut<'a>(s: &'a mut Session, p: &Value) -> Result<(&'a mut Layer, usize, HintSide)> {
    let (lid, i, side) = find(s, p)?;
    if s.timeline().locked_by_folder(lid) {
        return Err(EngineError::State("the layer is in a locked folder".into()));
    }
    let layer = s.layer_mut(Some(lid))?;
    if layer.locked {
        return Err(EngineError::State(format!("layer '{}' is locked", layer.name)));
    }
    Ok((layer, i, side))
}

fn keyframe_mut(layer: &mut Layer, i: usize) -> Result<&mut Keyframe> {
    layer.keyframe_mut(i).ok_or_else(|| EngineError::State("the keyframe went away".into()))
}

/// `hint` as a letter (`"a"`) or an index (`0`), checked against the tween's hints.
fn hint_param(p: &Value, count: usize) -> Result<usize> {
    let i = match p.get("hint") {
        Some(Value::String(l)) => hint_index(l).ok_or_else(|| EngineError::Param("param 'hint' must be a letter a to z".into()))?,
        Some(v) => v.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| EngineError::Param("param 'hint' must be a letter a to z".into()))?,
        None => return Err(EngineError::Param("param 'hint' is required (a letter a to z)".into())),
    };
    if i >= count {
        return Err(EngineError::State(match count {
            0 => "this shape tween has no hints; add one with Modify › Shape › Add Shape Hint".to_string(),
            n => format!("there is no hint '{}' (this tween has a to {})", hint_letter(i), hint_letter(n - 1)),
        }));
    }
    Ok(i)
}

fn point(v: Vec2) -> Value {
    json!({"x": v.x, "y": v.y})
}

fn add_hint(s: &mut Session, p: &Value) -> Result<Value> {
    let (layer, i, _) = find_mut(s, p)?;
    let start = layer.hint_keyframe(i, HintSide::Start).and_then(|k| shapes_centre(&k.elements));
    let end = layer.hint_keyframe(i, HintSide::End).and_then(|k| shapes_centre(&k.elements));
    let start = start.ok_or_else(|| EngineError::State("there is no shape on the first keyframe to hint; shape hints work on drawn shapes".into()))?;
    let end = end.unwrap_or(start);
    let k = keyframe_mut(layer, i)?;
    if k.hints.len() >= MAX_SHAPE_HINTS {
        return Err(EngineError::State(format!("a shape tween holds at most {MAX_SHAPE_HINTS} hints (a to z)")));
    }
    k.hints.push(ShapeHint { start, end });
    let n = k.hints.len() - 1;
    Ok(json!({"hint": hint_letter(n).to_string(), "index": n, "start": point(start), "end": point(end), "count": k.hints.len()}))
}

fn move_hint(s: &mut Session, p: &Value) -> Result<Value> {
    let (x, y) = (f32_req(p, "x")?, f32_req(p, "y")?);
    let side = match opt_str(p, "side")? {
        Some(k) => Some(HintSide::from_key(k).ok_or_else(|| EngineError::Param("param 'side' must be start or end".into()))?),
        None => None,
    };
    let (layer, i, shown) = find_mut(s, p)?;
    let side = side.unwrap_or(shown);
    let count = layer.keyframes.get(i).map_or(0, |k| k.hints.len());
    let h = hint_param(p, count)?;
    let els = layer.hint_keyframe(i, side).map(|k| k.elements.clone()).unwrap_or_default();
    // Hints live on edges: the point snaps to the nearest one.
    let (_, at, _) = nearest_outline(&els, Vec2::new(x, y)).ok_or_else(|| EngineError::State("there is no shape on that keyframe to put the hint on".into()))?;
    let k = keyframe_mut(layer, i)?;
    let hint = k.hints.get_mut(h).ok_or_else(|| EngineError::State("the hint went away".into()))?;
    *hint.at_mut(side) = at;
    Ok(json!({"hint": hint_letter(h).to_string(), "side": side.key(), "x": at.x, "y": at.y}))
}

fn remove_hint(s: &mut Session, p: &Value) -> Result<Value> {
    let (layer, i, _) = find_mut(s, p)?;
    let k = keyframe_mut(layer, i)?;
    let h = hint_param(p, k.hints.len())?;
    k.hints.remove(h);
    Ok(json!({"removed": hint_letter(h).to_string(), "count": k.hints.len()}))
}

fn clear_hints(s: &mut Session, p: &Value) -> Result<Value> {
    let (layer, i, _) = find_mut(s, p)?;
    let k = keyframe_mut(layer, i)?;
    let n = k.hints.len();
    if n == 0 {
        return Err(EngineError::State("this shape tween has no hints".into()));
    }
    k.hints.clear();
    Ok(json!({"removed": n}))
}

fn list_hints(s: &mut Session, p: &Value) -> Result<Value> {
    let (lid, i, side) = find(s, p)?;
    let layer = s.timeline().layer(lid).ok_or_else(|| EngineError::State("no such layer".into()))?;
    let (Some(a), Some(b)) = (layer.hint_keyframe(i, HintSide::Start), layer.hint_keyframe(i, HintSide::End)) else { return Err(EngineError::State("the keyframe went away".into())) };
    let hints: Vec<Value> = a
        .hints
        .iter()
        .enumerate()
        .map(|(n, h)| {
            let (on_a, on_b) = (placed_on(&a.elements, h.start), placed_on(&b.elements, h.end));
            json!({
                "hint": hint_letter(n).to_string(),
                "start": point(h.start),
                "end": point(h.end),
                "start_placed": on_a.is_some(),
                "end_placed": on_b.is_some(),
                // The morph uses it: on an edge of the same shape at both ends.
                "active": on_a.is_some() && on_a == on_b,
            })
        })
        .collect();
    Ok(json!({"tween": a.start, "end": b.start, "side": side.key(), "hints": hints}))
}
