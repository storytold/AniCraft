//! Asset Warp commands (ADR 0079): a warp mesh on a shape or a bitmap, pins on it, and moving
//! them to bend the artwork.
//!
//! The mesh belongs to the artwork: `warp.add` (and the first `warp.pin_add`) puts it on the
//! element at the playhead and on every keyframe of its layer that holds the same artwork, so
//! keyframes made from it bend the same mesh. Pins are added to and removed from all of those
//! keyframes (where the artwork shows there, so nothing moves), while `warp.pin_move` moves a
//! pin on the keyframe at the playhead only: that is the pose, and poses tween with classic
//! tweens. Inside a tween a move keys the frame with what it shows, as editing any tweened
//! object does. A drag sends many `warp.pin_move` calls inside one gesture: one undo step.
//!
//! A shape edited since by another tool (its outline no longer the one its warp gave it) starts
//! again from how it now looks: its pins stay where they show and become its rest.

use crate::params::*;
use crate::{CommandSpec, EngineError, KeyEdit, MAX_FRAME, Result, Session};
use anicraft_doc::warp::{self, MAX_EXTENT, MAX_PINS, MAX_RESOLUTION, MAX_WARP_POINTS, MIN_RESOLUTION, PIN_SPACING, Pin, PinId, Warp};
use anicraft_doc::{Element, ElementId, ElementKind, LayerId, Paint, Shape, evaluate_layer_elements};
use anicraft_geom::{Affine, Path, Vec2};
use serde_json::{Value, json};
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, params, mutates, run| CommandSpec { id, label, menu, shortcut: None, params, mutates, run };
    vec![
        c(
            "warp.add",
            "Add Warp Mesh",
            "Modify/Asset Warp",
            "{element? (a shape or bitmap; default the one selected), resolution? (cells along the longer side, 2…48; default 12)} → {element, created, resolution, columns, rows}. Covers the artwork with a warp mesh on every keyframe of its layer that holds it (it becomes a drawing object); on artwork that has one, changes the mesh's resolution",
            true,
            add as crate::RunFn,
        ),
        c(
            "warp.pin_add",
            "Add Warp Pin",
            "",
            "{element?, x, y (stage px, on the artwork)} → {element, pin, created}. Pins the artwork there (adding the mesh first if it has none), on every keyframe that bends the same mesh; nothing moves",
            true,
            pin_add,
        ),
        c("warp.pin_move", "Move Warp Pin", "", "{element?, pin, x?, y? (stage px: where the pin goes), dx?, dy? (stage px: by how much), frame?} → {element, pin, at}. Bends the artwork at the frame (keying it inside a tween); pins tween between keyframes", true, pin_move),
        c("warp.pin_remove", "Remove Warp Pin", "", "{element?, pin} removes the pin from every keyframe that bends the same mesh; the artwork follows the pins that are left", true, pin_remove),
        c("warp.remove", "Remove Warp", "Modify/Asset Warp", "{element?} takes the warp mesh and its pins off the artwork on every keyframe of its layer: it shows as drawn again", true, remove),
        c("warp.info", "Warp Mesh", "", "{element?, frame?} → the artwork's warp at the frame: resolution, the mesh's columns, rows, vertices and triangles, whether it is bent, and its pins (id, where they show in stage px, and where they were placed on the artwork as drawn, in its own space)", false, info),
    ]
}

/// The element a warp command acts on: `element`, or the one selected element on the active
/// layer.
fn element_param(s: &Session, p: &Value) -> Result<ElementId> {
    if let Some(id) = opt_u64(p, "element")? {
        return Ok(ElementId(id));
    }
    match s.selection.on(s.layer).as_slice() {
        [one] => Ok(*one),
        [] => Err(EngineError::State("select a shape or a bitmap first (or give 'element')".into())),
        _ => Err(EngineError::State("select just one shape or bitmap to warp".into())),
    }
}

/// Where element `id` is at `frame`, as it shows (tweened): its layer and the element.
fn find(s: &Session, id: ElementId, frame: u32) -> Result<(LayerId, Arc<Element>)> {
    s.timeline()
        .layers
        .iter()
        .find_map(|l| {
            let (_, els) = evaluate_layer_elements(l, frame)?;
            Some((l.id, els.iter().find(|e| e.id == id)?.clone()))
        })
        .ok_or_else(|| EngineError::State(format!("no element with id {} on the stage at frame {}", id.0, frame + 1)))
}

fn shape_of(el: &Element) -> Result<&Shape> {
    match &el.kind {
        ElementKind::Shape(sh) => Ok(sh),
        _ => Err(EngineError::State("Asset Warp bends shapes and bitmaps; break a symbol or text apart first (Modify › Break Apart)".into())),
    }
}

fn frame_param(s: &Session, p: &Value) -> Result<u32> {
    let f = opt_u32(p, "frame")?.unwrap_or(s.frame);
    if f > MAX_FRAME {
        return Err(EngineError::Param(format!("param 'frame' must be at most {MAX_FRAME}")));
    }
    Ok(f)
}

/// The warp a command starts from on `sh`: its own while it shows it, rebased when the shape
/// was edited since, `None` without one.
fn current(sh: &Shape) -> Option<Warp> {
    let w = sh.warp.as_deref()?;
    Some(if w.shows(&sh.path) { w.clone() } else { w.rebased(sh.path.clone()) })
}

fn check_artwork(path: &Path) -> Result<()> {
    if warp::anchor_count(path) > MAX_WARP_POINTS {
        return Err(EngineError::State(format!("that artwork has more than {MAX_WARP_POINTS} points: simplify it first (Modify › Shape › Optimize)")));
    }
    if Warp::new(Arc::new(path.clone()), MIN_RESOLUTION).grid().is_none() {
        return Err(EngineError::State("that artwork has no extent to warp".into()));
    }
    Ok(())
}

/// Runs `edit` on element `id` in the keyframe of `layer` that an edit at `frame` changes (keying
/// the frame inside a classic or shape tween), with the shape and the stage → local map.
fn edit_at<T>(s: &mut Session, layer: LayerId, frame: u32, id: ElementId, edit: impl FnOnce(&mut Shape, &Affine) -> Result<T>) -> Result<T> {
    let tl = s.timeline();
    let l = tl.layer(layer).ok_or_else(|| EngineError::State("the layer went away".into()))?;
    if l.keyframe_at(frame).is_some_and(|k| k.motion.is_some()) {
        return Err(EngineError::State("Asset Warp pins tween with classic tweens: this frame is in a motion tween".into()));
    }
    let (was_layer, was_frame) = (s.layer, s.frame);
    s.layer = layer;
    s.frame = frame;
    let out = (|| {
        let kf = s.editable_keyframe_in(KeyEdit::Artwork)?;
        let e = kf.elements.iter_mut().find(|e| e.id == id).ok_or_else(|| EngineError::State("that element isn't on this keyframe".into()))?;
        let el = Arc::make_mut(e);
        let inv = el.transform.inverse().ok_or_else(|| EngineError::State("artwork scaled to nothing can't be warped".into()))?;
        let ElementKind::Shape(sh) = &mut el.kind else { return Err(EngineError::State("Asset Warp bends shapes and bitmaps".into())) };
        edit(sh, &inv)
    })();
    s.layer = was_layer;
    s.frame = was_frame;
    out
}

/// Runs `f` on every other keyframe of `layer` (not the one starting at `skip`) whose element
/// `id` is warped on the same mesh as `rest` (it shows its warp), then bends it again.
fn each_other(s: &mut Session, layer: LayerId, skip: Option<u32>, id: ElementId, rest: &Arc<Path>, resolution: u32, mut f: impl FnMut(&mut Warp, &mut Shape)) -> Result<usize> {
    let l = s.layer_mut(Some(layer))?;
    let mut n = 0;
    for k in &mut l.keyframes {
        if Some(k.start) == skip {
            continue;
        }
        let same = |e: &Arc<Element>| e.id == id && matches!(&e.kind, ElementKind::Shape(sh) if sh.live_warp().is_some_and(|w| anicraft_doc::arc_eq(&w.rest, rest) && w.resolution == resolution));
        if !k.elements.iter().any(same) {
            continue;
        }
        for e in Arc::make_mut(k).elements.iter_mut().filter(|e| same(e)) {
            if let ElementKind::Shape(sh) = &mut Arc::make_mut(e).kind
                && let Some(mut w) = sh.warp.as_deref().cloned()
            {
                f(&mut w, sh);
                if sh.warp.is_some() {
                    sh.set_warp(w);
                }
                n += 1;
            }
        }
    }
    Ok(n)
}

/// The start of the keyframe of `layer` covering `frame`.
fn key_start(s: &Session, layer: LayerId, frame: u32) -> Option<u32> {
    s.timeline().layer(layer).and_then(|l| l.keyframe_at(frame)).map(|k| k.start)
}

fn resolution_param(p: &Value) -> Result<Option<u32>> {
    match opt_u32(p, "resolution")? {
        Some(r) if !(MIN_RESOLUTION..=MAX_RESOLUTION).contains(&r) => Err(EngineError::Param(format!("param 'resolution' must be {MIN_RESOLUTION}…{MAX_RESOLUTION}"))),
        r => Ok(r),
    }
}

/// Puts a warp on element `id` at the playhead (and on every keyframe holding the same
/// artwork), or changes its resolution. Returns whether it was made, and the warp.
fn ensure(s: &mut Session, layer: LayerId, id: ElementId, resolution: Option<u32>) -> Result<(bool, Warp)> {
    let frame = s.frame;
    // The artwork as it is on the keyframe at the playhead (before any keying), to find copies.
    let before: Option<Arc<Path>> = s.timeline().layer(layer).and_then(|l| l.keyframe_at(frame)).and_then(|k| k.elements.iter().find(|e| e.id == id)).and_then(|e| match &e.kind {
        ElementKind::Shape(sh) => Some(sh.path.clone()),
        _ => None,
    });
    let (created, w, old) = edit_at(s, layer, frame, id, |sh, _| {
        let old = sh.live_warp().map(|w| (w.rest.clone(), w.resolution));
        let (created, mut w) = match current(sh) {
            Some(w) => (false, w),
            None => {
                check_artwork(&sh.path)?;
                (true, Warp::new(sh.path.clone(), resolution.unwrap_or(warp::DEFAULT_RESOLUTION)))
            }
        };
        if let Some(r) = resolution {
            w.resolution = r;
        }
        // A warped shape is a drawing object: drawing over it never cuts its outline.
        sh.merge = false;
        sh.set_warp(w.clone());
        Ok((created, w, old))
    })?;
    let skip = key_start(s, layer, frame);
    match old {
        // A new resolution: every keyframe on the same mesh takes it.
        Some((rest, r)) if !created && Some(r) != resolution && resolution.is_some() => {
            each_other(s, layer, skip, id, &rest, r, |x, _| x.resolution = w.resolution)?;
        }
        Some(_) => {}
        None => {
            // A new mesh: every other keyframe holding the very artwork it covers gets it too.
            let Some(path) = before else { return Ok((created, w)) };
            let l = s.layer_mut(Some(layer))?;
            for k in &mut l.keyframes {
                if Some(k.start) == skip {
                    continue;
                }
                let same = |e: &Arc<Element>| e.id == id && matches!(&e.kind, ElementKind::Shape(sh) if sh.warp.is_none() && (Arc::ptr_eq(&sh.path, &path) || *sh.path == *path));
                if k.elements.iter().any(same) {
                    for e in Arc::make_mut(k).elements.iter_mut().filter(|e| same(e)) {
                        if let ElementKind::Shape(sh) = &mut Arc::make_mut(e).kind {
                            sh.merge = false;
                            let mut x = w.clone();
                            x.rest = sh.path.clone();
                            sh.set_warp(x);
                        }
                    }
                }
            }
        }
    }
    Ok((created, w))
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    let id = element_param(s, p)?;
    let resolution = resolution_param(p)?;
    let (layer, el) = find(s, id, s.frame)?;
    shape_of(&el)?;
    let (created, w) = ensure(s, layer, id, resolution)?;
    let g = w.grid();
    Ok(json!({"element": id.0, "created": created, "resolution": w.resolution, "columns": g.map_or(0, |g| g.nx), "rows": g.map_or(0, |g| g.ny)}))
}

fn stage_point(p: &Value) -> Result<Vec2> {
    let v = Vec2::new(f32_req(p, "x")?, f32_req(p, "y")?);
    if v.x.abs() > MAX_EXTENT || v.y.abs() > MAX_EXTENT {
        return Err(EngineError::Param("the point must be within ±1,000,000 px".into()));
    }
    Ok(v)
}

fn pin_add(s: &mut Session, p: &Value) -> Result<Value> {
    let id = element_param(s, p)?;
    let at = stage_point(p)?;
    let (layer, el) = find(s, id, s.frame)?;
    shape_of(&el)?;
    let (created, _) = ensure(s, layer, id, None)?;
    let pin = PinId(s.doc.alloc_id());
    let frame = s.frame;
    let (rest, mesh) = edit_at(s, layer, frame, id, |sh, inv| {
        let mut w = current(sh).ok_or_else(|| EngineError::State("the artwork has no warp mesh".into()))?;
        if w.pins.len() >= MAX_PINS {
            return Err(EngineError::State(format!("a warp has at most {MAX_PINS} pins")));
        }
        let local = inv.apply(at);
        let r = w.unmap(local).ok_or_else(|| EngineError::State("click on the artwork to pin it".into()))?;
        if w.pins.iter().any(|q| q.rest.distance(r) < PIN_SPACING) {
            return Err(EngineError::State("there is a pin there already".into()));
        }
        w.pins.push(Pin { id: pin, rest: r, at: local });
        let mesh = (w.rest.clone(), w.resolution);
        sh.set_warp(w);
        Ok((r, mesh))
    })?;
    // The same pin on every other keyframe bending this mesh, where that point shows there.
    let skip = key_start(s, layer, frame);
    each_other(s, layer, skip, id, &mesh.0, mesh.1, |w, _| {
        if w.pins.len() < MAX_PINS && w.pin(pin).is_none() {
            let shown = w.map(rest);
            w.pins.push(Pin { id: pin, rest, at: shown });
        }
    })?;
    Ok(json!({"element": id.0, "pin": pin.0, "created": created}))
}

fn pin_id(p: &Value) -> Result<PinId> {
    opt_u64(p, "pin")?.map(PinId).ok_or_else(|| EngineError::Param("param 'pin' (a pin id; see warp.info) is required".into()))
}

fn pin_move(s: &mut Session, p: &Value) -> Result<Value> {
    let id = element_param(s, p)?;
    let pin = pin_id(p)?;
    let frame = frame_param(s, p)?;
    let to = match (opt_f32(p, "x")?, opt_f32(p, "y")?) {
        (Some(x), Some(y)) => Some(Vec2::new(x, y)),
        (None, None) => None,
        _ => return Err(EngineError::Param("give both 'x' and 'y' (or 'dx' and 'dy')".into())),
    };
    let by = Vec2::new(opt_f32(p, "dx")?.unwrap_or(0.0), opt_f32(p, "dy")?.unwrap_or(0.0));
    let (layer, el) = find(s, id, frame)?;
    shape_of(&el)?;
    let xf = el.transform;
    let at = edit_at(s, layer, frame, id, |sh, inv| {
        let mut w = current(sh).ok_or_else(|| EngineError::State("the artwork has no warp mesh: add pins with the Warp tool first".into()))?;
        let q = w.pins.iter_mut().find(|q| q.id == pin).ok_or_else(|| EngineError::State(format!("no pin with id {} on that artwork", pin.0)))?;
        let local = match to {
            Some(t) => inv.apply(t),
            None => q.at + inv.apply_vector(by),
        };
        if !(local.is_finite() && local.x.abs() <= MAX_EXTENT && local.y.abs() <= MAX_EXTENT) {
            return Err(EngineError::Param("the pin must stay within ±1,000,000 px".into()));
        }
        q.at = local;
        sh.set_warp(w);
        Ok(local)
    })?;
    let shown = xf.apply(at);
    Ok(json!({"element": id.0, "pin": pin.0, "at": [shown.x, shown.y]}))
}

fn pin_remove(s: &mut Session, p: &Value) -> Result<Value> {
    let id = element_param(s, p)?;
    let pin = pin_id(p)?;
    let (layer, el) = find(s, id, s.frame)?;
    shape_of(&el)?;
    let frame = s.frame;
    let mesh = edit_at(s, layer, frame, id, |sh, _| {
        let mut w = current(sh).ok_or_else(|| EngineError::State("the artwork has no warp mesh".into()))?;
        let before = w.pins.len();
        w.pins.retain(|q| q.id != pin);
        if w.pins.len() == before {
            return Err(EngineError::State(format!("no pin with id {} on that artwork", pin.0)));
        }
        let mesh = (w.rest.clone(), w.resolution);
        sh.set_warp(w);
        Ok(mesh)
    })?;
    let skip = key_start(s, layer, frame);
    let n = each_other(s, layer, skip, id, &mesh.0, mesh.1, |w, _| w.pins.retain(|q| q.id != pin))?;
    Ok(json!({"element": id.0, "pin": pin.0, "keyframes": n + 1}))
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    let id = element_param(s, p)?;
    let (layer, el) = find(s, id, s.frame)?;
    let sh = shape_of(&el)?;
    let frame = s.frame;
    let Some(w) = sh.live_warp().cloned() else {
        if sh.warp.is_some() {
            // Edited since by another tool: the stale warp just goes; the shape stays as it is.
            edit_at(s, layer, frame, id, |sh, _| {
                sh.warp = None;
                Ok(())
            })?;
            return Ok(json!({"element": id.0, "keyframes": 1}));
        }
        return Err(EngineError::State("that artwork has no warp mesh".into()));
    };
    edit_at(s, layer, frame, id, |sh, _| {
        sh.path = w.rest.clone();
        sh.warp = None;
        Ok(())
    })?;
    let skip = key_start(s, layer, frame);
    let n = each_other(s, layer, skip, id, &w.rest, w.resolution, |w, sh| {
        sh.path = w.rest.clone();
        sh.warp = None;
    })?;
    Ok(json!({"element": id.0, "keyframes": n + 1}))
}

fn info(s: &mut Session, p: &Value) -> Result<Value> {
    let id = element_param(s, p)?;
    let frame = frame_param(s, p)?;
    let (layer, el) = find(s, id, frame)?;
    let sh = shape_of(&el)?;
    let bitmap = matches!(sh.fill, Some(Paint::Bitmap { .. }));
    let Some(w) = sh.live_warp() else {
        return Ok(json!({"element": id.0, "layer": layer.0, "warped": false, "bitmap": bitmap, "stale": sh.warp.is_some()}));
    };
    let g = w.grid();
    let pins: Vec<Value> = w
        .pins
        .iter()
        .map(|q| {
            let at = el.transform.apply(q.at);
            json!({"pin": q.id.0, "at": [at.x, at.y], "rest": [q.rest.x, q.rest.y]})
        })
        .collect();
    Ok(json!({
        "element": id.0,
        "layer": layer.0,
        "warped": true,
        "bitmap": bitmap,
        "bent": !w.at_rest(),
        "resolution": w.resolution,
        "columns": g.map_or(0, |g| g.nx),
        "rows": g.map_or(0, |g| g.ny),
        "vertices": g.map_or(0, |g| (g.nx + 1) * (g.ny + 1)),
        "triangles": g.map_or(0, |g| 2 * g.nx * g.ny),
        "pins": pins,
    }))
}
