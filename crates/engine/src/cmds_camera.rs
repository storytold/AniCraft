//! Camera commands (ADR 0027): add and remove the scene's camera layer, set the camera at a
//! frame (pan, zoom, rotation, tint), read it back, and give layers a depth for parallax.
//!
//! `camera.set` edits the camera like any instance: through [`Session::editable_keyframe`], so
//! inside a classic tween it keys the playhead and inside a motion tween it becomes property
//! keys (auto-key). A drag sends many `camera.set` calls inside one gesture: one undo step.

use crate::params::*;
use crate::{CommandSpec, EngineError, MAX_FRAME, Result, Session};
use anicraft_doc::camera::{self, Camera, MAX_ZOOM, MIN_ZOOM};
use anicraft_doc::{ElementId, ElementKind, Keyframe, Layer, LayerId, LayerKind};
use anicraft_geom::Vec2;
use serde_json::{Value, json};
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c("camera.add", "Add Camera", "Insert/Timeline", None, "{} (a camera layer on top of the scene, at rest: it sees the stage as it is; nothing happens if there is one)", true, add as crate::RunFn),
        c("camera.remove", "Remove Camera", "Modify/Timeline", None, "{} (removes the camera layer and its moves; asks first when the camera is animated)", true, remove),
        c(
            "camera.set",
            "Camera",
            "",
            None,
            "{x?, y? (camera centre, stage px), dx?, dy? (pan by), zoom? (%, 5–2000), zoom_by? (factor), rotation? (°), rotate_by? (°), tint? (#rrggbb), tint_amount? (%), reset?: bool, frame?}",
            true,
            set,
        ),
        c("camera.info", "Camera Info", "", None, "{frame?} → the camera at the frame (x, y, zoom %, rotation °, tint) and its frame on the stage", false, info),
        c("layer.set_depth", "Layer Depth", "", None, "{depth (px behind the stage, −500…10000; deeper layers move less when the camera pans), layer?}", true, set_depth),
    ]
}

/// The scene's camera layer id.
pub(crate) fn camera_layer(s: &Session) -> Option<LayerId> {
    s.timeline().camera_layer().map(|l| l.id)
}

fn no_camera() -> EngineError {
    EngineError::State("this scene has no camera; add one first (Add Camera on the timeline, or camera.add)".into())
}

fn add(s: &mut Session, _: &Value) -> Result<Value> {
    // A scene has the camera, not a symbol (ADR 0027, 0045).
    if !s.editing.is_empty() {
        return Err(EngineError::State("the camera belongs to a scene: close the symbol you are editing first (Edit › Edit Document)".into()));
    }
    if let Some(id) = camera_layer(s) {
        s.layer = id;
        s.selection.clear();
        return Ok(json!({"layer": id.0, "added": false}));
    }
    if s.timeline().layers.len() >= 1000 {
        return Err(EngineError::State("too many layers (limit 1000)".into()));
    }
    let id = LayerId(s.doc.alloc_id());
    let el = ElementId(s.doc.alloc_id());
    let frames = s.frame_count().max(1);
    let mut layer = Layer::new(id, "Camera");
    layer.kind = LayerKind::Camera;
    // The frame outline on the stage is drawn in the layer colour: an orange that reads on
    // light and dark artwork alike.
    layer.color = anicraft_doc::Rgba::hex(0xe8833a);
    let mut k = Keyframe::new(0, frames);
    k.elements.push(Arc::new(Camera::rest(&s.doc.stage).to_element(el)));
    layer.keyframes = vec![Arc::new(k)];
    let tl = crate::editing::edited_mut(&mut s.doc, s.scene, &s.editing);
    tl.layers.insert(0, Arc::new(layer));
    tl.repair_masks();
    s.layer = id;
    s.selection.clear();
    Ok(json!({"layer": id.0, "added": true}))
}

fn remove(s: &mut Session, _: &Value) -> Result<Value> {
    let id = camera_layer(s).ok_or_else(no_camera)?;
    let tl = crate::editing::edited_mut(&mut s.doc, s.scene, &s.editing);
    tl.layers.retain(|l| l.id != id);
    tl.repair_masks();
    if s.layer == id {
        s.layer = s.timeline().layers.first().map_or(LayerId(0), |l| l.id);
        s.selection.clear();
    }
    Ok(json!({"removed": id.0}))
}

/// How many keyframes the camera moves over (0 when it doesn't move: one keyframe holds it and
/// nothing tweens it).
pub(crate) fn camera_moves(l: &Layer) -> usize {
    let keys = l.keyframes.iter().filter(|k| k.elements.iter().any(|e| matches!(e.kind, ElementKind::Camera))).count();
    let tweened = l.keyframes.iter().any(|k| k.tween.is_some() || k.motion.is_some());
    if keys > 1 || tweened { keys.max(1) } else { 0 }
}

/// Frames moved or pasted onto a layer must suit it: the camera layer holds only the camera,
/// and a camera on any other layer would do nothing.
pub(crate) fn keys_fit<'a>(kind: LayerKind, mut keys: impl Iterator<Item = &'a Keyframe>) -> Result<()> {
    let camera = kind == LayerKind::Camera;
    if keys.any(|k| k.elements.iter().any(|e| matches!(e.kind, ElementKind::Camera) != camera)) {
        return Err(EngineError::State(if camera { "the camera layer holds only the camera; put artwork on another layer".into() } else { "camera keyframes belong on the camera layer".into() }));
    }
    Ok(())
}


fn set(s: &mut Session, p: &Value) -> Result<Value> {
    let id = camera_layer(s).ok_or_else(no_camera)?;
    let frame = opt_u32(p, "frame")?.unwrap_or(s.frame);
    if frame > MAX_FRAME {
        return Err(EngineError::Param(format!("param 'frame' must be at most {MAX_FRAME}")));
    }
    let num = |k: &str| opt_f32(p, k);
    let (x, y, dx, dy) = (num("x")?, num("y")?, num("dx")?, num("dy")?);
    let (zoom, zoom_by, rotation, rotate_by) = (num("zoom")?, num("zoom_by")?, num("rotation")?, num("rotate_by")?);
    let tint = opt_color(p, "tint")?;
    let tint_amount = num("tint_amount")?;
    let reset = opt_bool(p, "reset")?.unwrap_or(false);
    if let Some(z) = zoom
        && !(MIN_ZOOM * 100.0..=MAX_ZOOM * 100.0).contains(&z)
    {
        return Err(EngineError::Param(format!("param 'zoom' must be between {} and {} (%)", MIN_ZOOM * 100.0, MAX_ZOOM * 100.0)));
    }
    if zoom_by.is_some_and(|f| f <= 0.0) {
        return Err(EngineError::Param("param 'zoom_by' must be above 0".into()));
    }
    if tint_amount.is_some_and(|a| !(0.0..=100.0).contains(&a)) {
        return Err(EngineError::Param("param 'tint_amount' must be between 0 and 100 (%)".into()));
    }
    let stage = s.doc.stage;
    // A camera is created on a keyframe that has none (a blank camera keyframe is at rest).
    // (Its id is taken only when it is used, so a no-op leaves the document unchanged.)
    let fresh = ElementId(s.doc.next_id.max(1));
    let mut used = false;
    let (layer, at) = (s.layer, s.frame);
    s.layer = id;
    s.frame = frame;
    let out: Result<Camera> = (|| {
        let kf = s.editable_keyframe_in(crate::KeyEdit::Camera)?;
        let i = match kf.elements.iter().position(|e| matches!(e.kind, ElementKind::Camera)) {
            Some(i) => i,
            None => {
                kf.elements.push(Arc::new(Camera::rest(&stage).to_element(fresh)));
                used = true;
                kf.elements.len() - 1
            }
        };
        let slot = kf.elements.get_mut(i).ok_or_else(|| EngineError::State("the camera went away".into()))?;
        let el = Arc::make_mut(slot);
        let mut c = if reset { Camera::rest(&stage) } else { Camera::of_element(el, &stage) };
        c.center = Vec2::new(x.unwrap_or(c.center.x), y.unwrap_or(c.center.y)) + Vec2::new(dx.unwrap_or(0.0), dy.unwrap_or(0.0));
        if let Some(z) = zoom {
            c.zoom = z / 100.0;
        }
        c.zoom = (c.zoom * zoom_by.unwrap_or(1.0)).clamp(MIN_ZOOM, MAX_ZOOM);
        if let Some(r) = rotation {
            c.rotation = r.to_radians();
        }
        c.rotation += rotate_by.unwrap_or(0.0).to_radians();
        if let Some(t) = tint {
            c.tint = t.with_alpha(1.0);
        }
        if let Some(a) = tint_amount {
            c.tint_amount = a / 100.0;
        }
        // Keep the centre on a sane plane (a long drag can't fling it to infinity).
        let lim = 1_000_000.0;
        c.center = Vec2::new(c.center.x.clamp(-lim, lim), c.center.y.clamp(-lim, lim));
        el.transform = c.transform();
        el.color = c.to_element(el.id).color;
        Ok(c)
    })();
    s.layer = layer;
    s.frame = at;
    let c = out?;
    if used {
        s.doc.alloc_id();
    }
    Ok(camera_json(&c, &stage))
}

fn camera_json(c: &Camera, stage: &anicraft_doc::Stage) -> Value {
    let f = c.frame(stage);
    json!({
        "x": c.center.x, "y": c.center.y, "zoom": c.zoom * 100.0, "rotation": c.rotation.to_degrees(),
        "tint": c.tint.to_hex(), "tint_amount": c.tint_amount * 100.0,
        "frame": f.iter().map(|p| json!([p.x, p.y])).collect::<Vec<_>>(),
    })
}

fn info(s: &mut Session, p: &Value) -> Result<Value> {
    let frame = opt_u32(p, "frame")?.unwrap_or(s.frame).min(MAX_FRAME);
    let Some(id) = camera_layer(s) else { return Ok(json!({"camera": false})) };
    let tl = s.timeline();
    let on = tl.camera_layer().is_some_and(|l| l.visible);
    let mut v = s.timeline().camera_state_at(frame, &s.doc.stage).map_or_else(|| json!({}), |c| camera_json(&c, &s.doc.stage));
    v["camera"] = json!(true);
    v["layer"] = json!(id.0);
    v["on"] = json!(on);
    Ok(v)
}

fn set_depth(s: &mut Session, p: &Value) -> Result<Value> {
    let depth = f32_req(p, "depth")?;
    if !(camera::MIN_DEPTH..=camera::MAX_DEPTH).contains(&depth) {
        return Err(EngineError::Param(format!("param 'depth' must be between {} and {}", camera::MIN_DEPTH, camera::MAX_DEPTH)));
    }
    let lid = opt_u64(p, "layer")?.map(LayerId);
    let layer = s.layer_mut(lid)?;
    if matches!(layer.kind, LayerKind::Camera | LayerKind::Folder) {
        return Err(EngineError::State(format!("'{}' has no depth of its own (only layers with artwork do)", layer.name)));
    }
    layer.depth = depth;
    Ok(json!({"layer": layer.id.0, "depth": depth}))
}
