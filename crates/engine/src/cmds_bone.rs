//! Bone commands (ADR 0043): join symbol instances into an armature, take bones away, limit and
//! pin joints, pose the armature at a frame by joint angle or by inverse kinematics, and read
//! it back.
//!
//! The armature lives on an armature layer: `bone.add` moves the instances there (making the
//! layer for the first bone). Poses go through [`Session::editable_keyframe_in`], so inside a
//! pose tween they key the frame with the pose it shows, as editing any tweened object does;
//! a drag sends many `bone.solve` calls inside one gesture: one undo step. After every pose
//! edit the keyframe's instances are placed by the pose, so the file holds what shows.

use crate::cmds_timeline::insert_keyframe_at;
use crate::params::*;
use crate::{CommandSpec, EngineError, KeyEdit, MAX_FRAME, Result, Session};
use anicraft_doc::armature::{self, Armature, Bone, BoneId, Limits, MAX_BONES, MAX_LIMIT, MAX_NAME, MAX_SLIDE, Spring};
use anicraft_doc::skin::{self, Binding, Influence, MAX_SKIN_POINTS, Skin};
use anicraft_doc::weights::{self, Brush, Dab, MAX_DABS, MAX_RADIUS};
use anicraft_doc::{Element, ElementId, ElementKind, Keyframe, Layer, LayerId, LayerKind, Rgba, Shape, Tween, evaluate_layer_elements};
use anicraft_geom::{Affine, Vec2};
use serde_json::{Value, json};
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, params, mutates, run| CommandSpec { id, label, menu: "", shortcut: None, params, mutates, run };
    vec![
        c(
            "bone.add",
            "Add Bone",
            "{from (instance id: the bone starts on it), to (instance id: the bone moves it), head?: [x, y] (the joint on a root instance, stage px, default its registration point; from an instance that hangs from a bone the joint is always that bone's end, so chains stay joined), tail?: [x, y] (default `to`'s registration point), name?} → {bone, layer}. Both instances go to an armature layer (made for the first bone)",
            true,
            add as crate::RunFn,
        ),
        c(
            "bone.add_in_shape",
            "Add Bone in Shape",
            "{shape? (shape id: the first bone, or another bone from the shape), parent? (bone id: continue the chain from its end), head?: [x, y] (the joint, stage px; default the parent bone's end), tail: [x, y] (stage px), name?} → {bone, layer, created, points}. The first bone moves the shape to an armature layer of its own, as an object (merge drawing leaves it alone); the shape's points are bound to the nearest bones",
            true,
            add_in_shape,
        ),
        c("bone.bind", "Bind Points", "{bone, points?: [point index…] (default every point; see bone.info with points), mode?: \"only\" (the points follow just this bone; default) | \"add\" (they share it with the bones they follow) | \"remove\" | \"auto\" (back to automatic binding)} → {bone, changed, bound (how many points the bone moves)}. A shape armature's binding: which points of the shape follow which bone", true, bind),
        c("bone.paint_weights", "Paint Weights", "{bone, at: [[x, y]…] (dab centres, stage px, at most 4096), radius? (px, 1–2000, default 20), strength? (0–100 at the centre, default 25), mode?: \"add\" (default) | \"subtract\" | \"smooth\", frame?} → {bone, changed, bound}. The Bind tool's brush: raises, lowers or evens out how much the bone moves the shape's points within the radius of each dab (a smooth falloff to the rim); each point's weights stay normalised (its other bones share what is left) and the points become hand-bound. A drag sends many in one gesture: one undo step", true, paint_weights),
        c("bone.set_weight", "Set Point Weight", "{bone, points: [point index…] (see bone.info with points), weight (0–100: the bone's share of each point)} sets exactly how much the bone moves the points; their other bones share the rest in proportion (a point that followed only this bone gives the rest to its nearest other bones). Hand-bound", true, set_weight),
        c("bone.remove", "Remove Bone", "{bone} removes it and the bones below it; the instances they moved go to a layer of their own as they look at the playhead (with the last bone, the whole armature does); asks first when the armature has more than one pose", true, remove),
        c("bone.set_limits", "Joint Limits", "{bone, constrain?: bool (false: turns freely), min?, max? (°, −360…360, from where it was drawn)} limits how far the joint turns, in every pose", true, set_limits),
        c("bone.set_translation", "Joint Translation", "{bone, axis: \"x\" | \"y\" (along the axes of what the bone starts on, as it is turned), enabled?: bool (false: the joint doesn't slide that way), constrain?: bool (false: slides freely), min?, max? (px, ±10,000, from where it was drawn)} lets the joint slide, in every pose; posing (bone.solve, the Selection tool) slides it only where turning can't reach", true, set_translation),
        c("bone.set_spring", "Bone Spring", "{bone, strength? (0–100: 0 is no spring; stiffer swings faster and lags less), damping? (0–100: how quickly the swing dies down; 100 absorbs it all)} secondary motion: the bone lags behind and swings past its pose as the bones it hangs from move, then settles. Evaluated, never stored: the keyed poses don't change", true, set_spring),
        c("bone.enable_springs", "Enable Springs", "{enabled: bool, layer? (the armature layer) | bone?} turns the armature's springs on or off (off: it plays its keyed poses exactly, as while posing)", true, enable_springs),
        c("bone.pin", "Pin Bone", "{bone, pinned: bool} a pinned bone (and the bones it hangs from) stays put while dragging bones below it", true, pin),
        c("bone.pose", "Pose Bone", "{bone?, angle? (°, the joint angle: 0 as drawn), rotate_by? (°), slide_x?, slide_y? (px: how far the joint slides, within its translation limits), dx?, dy? (moves the whole armature), layer? (the armature layer, when no bone is given), frame?} keys the pose at the frame", true, pose),
        c("bone.solve", "Drag Bone", "{bone, x, y (stage px: where the bone's end goes), local?: [x, y] (the point dragged, in the bone's instance's space; default its end), frame?} → {reached, error (px), chain}; inverse kinematics turns the bone and the bones above it (limits respected, pinned bones kept)", true, solve),
        c("bone.info", "Armatures", "{frame?, layer?, points?: bool} → every armature on the scene at the frame: its bones with joint and end (stage px, as keyed), joint angle and limits (°), slide and translation limits (px), spring (0–100) and its swing at the frame (°, what the spring adds to the angle), pinned; whether springs are on; a shape armature's shape, how many points each bone moves, and with points the shape's points (stage px) and the bones each follows", false, info),
    ]
}

/// Where an instance is at the playhead.
struct Found {
    layer: LayerId,
    index: usize,
    kind: LayerKind,
    el: Arc<Element>,
}

/// Instance `id` on the scene at the playhead (as it shows: tweened, posed).
fn find_instance(s: &Session, id: ElementId) -> Option<Found> {
    let tl = s.timeline();
    tl.layers.iter().enumerate().find_map(|(index, l)| {
        let (_, els) = evaluate_layer_elements(l, s.frame)?;
        let el = els.iter().find(|e| e.id == id)?.clone();
        Some(Found { layer: l.id, index, kind: l.kind, el })
    })
}

/// The armature layer holding bone `id`, and its armature.
fn find_bone(s: &Session, id: BoneId) -> Result<(LayerId, Arc<Armature>)> {
    s
        .timeline()
        .layers
        .iter()
        .filter(|l| l.kind == LayerKind::Armature)
        .find_map(|l| l.armature.as_ref().filter(|a| a.bone(id).is_some()).map(|a| (l.id, a.clone())))
        .ok_or_else(|| EngineError::State(format!("no bone with id {} in this scene", id.0)))
}

fn bone_param(p: &Value) -> Result<BoneId> {
    opt_u64(p, "bone")?.map(BoneId).ok_or_else(|| EngineError::Param("param 'bone' (a bone id; see bone.info) is required".into()))
}

/// Largest stage coordinate a bone point may have.
const MAX_COORD: f32 = 1_000_000.0;

fn opt_point(p: &Value, key: &str) -> Result<Option<Vec2>> {
    let bad = || EngineError::Param(format!("param '{key}' must be [x, y] (finite numbers within ±1,000,000)"));
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(a)) if a.len() == 2 => {
            let n = |i: usize| a.get(i).and_then(Value::as_f64).filter(|v| v.is_finite() && v.abs() <= f64::from(MAX_COORD)).map(|v| v as f32).ok_or_else(bad);
            Ok(Some(Vec2::new(n(0)?, n(1)?)))
        }
        Some(_) => Err(bad()),
    }
}

fn unlocked(s: &Session, id: LayerId) -> Result<()> {
    let tl = s.timeline();
    let l = tl.layer(id).ok_or_else(|| EngineError::State(format!("no layer with id {}", id.0)))?;
    if l.locked || tl.locked_by_folder(id) {
        return Err(EngineError::State(format!("layer '{}' is locked", l.name)));
    }
    Ok(())
}

/// Takes instance `id` off the keyframe covering the playhead on layer `layer`.
fn take_off(s: &mut Session, layer: LayerId, id: ElementId) -> Result<()> {
    let frame = s.frame;
    let l = s.layer_mut(Some(layer))?;
    if let Some(k) = l.keyframe_index_at(frame).and_then(|i| l.keyframe_mut(i)) {
        k.elements.retain(|e| e.id != id);
    }
    Ok(())
}

/// A layer name `base_N` no layer of the scene has yet.
fn fresh_name(s: &Session, base: &str) -> String {
    let tl = s.timeline();
    (1..).map(|n| format!("{base}{n}")).find(|n| tl.layers.iter().all(|l| &l.name != n)).unwrap_or_else(|| base.to_string())
}

/// Places the instances of every keyframe of `layer` by its pose (after the armature changed).
fn rebake(layer: &mut Layer) {
    let Some(arm) = layer.armature.clone() else { return };
    for k in &mut layer.keyframes {
        let k = Arc::make_mut(k);
        let pose = k.pose.allowed(&arm);
        armature::apply(&arm, &mut k.elements, &pose);
    }
}

/// A new armature layer at `index` holding `arm` and `elements` on one pose that spans from the
/// keyframe of `from` at the playhead to the end of the timeline (a classic tween on it, so
/// poses keyed later tween).
fn armature_layer(s: &mut Session, from: LayerId, index: usize, elements: Vec<Arc<Element>>, arm: Armature) -> Result<LayerId> {
    let (start, end) = s.timeline().layer(from).and_then(|l| l.keyframe_at(s.frame)).map_or((s.frame, s.frame + 1), |k| (k.start, k.end()));
    let end = end.max(s.frame_count());
    if s.timeline().layers.len() >= 1000 {
        return Err(EngineError::State("too many layers (limit 1000)".into()));
    }
    let lid = LayerId(s.doc.alloc_id());
    let mut layer = Layer::new(lid, fresh_name(s, "Armature_"));
    layer.kind = LayerKind::Armature;
    // Bones show in the layer colour: a green that reads on light and dark artwork.
    layer.color = Rgba::hex(0x2fb67c);
    layer.armature = Some(Arc::new(arm));
    let mut k = Keyframe::new(start, end.saturating_sub(start).max(1));
    k.elements = elements;
    k.tween = Some(Tween::default());
    layer.keyframes = vec![Arc::new(k)];
    let tl = crate::editing::edited_mut(&mut s.doc, s.scene, &s.editing)?;
    layer.parent = tl.container_at(index, LayerKind::Armature);
    tl.layers.insert(index.min(tl.layers.len()), Arc::new(layer));
    tl.repair_masks();
    Ok(lid)
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    let req = |k: &str| opt_u64(p, k)?.map(ElementId).ok_or_else(|| EngineError::Param(format!("param '{k}' (an instance id) is required")));
    let (from, to) = (req("from")?, req("to")?);
    let (head, tail) = (opt_point(p, "head")?, opt_point(p, "tail")?);
    let name = opt_str(p, "name")?.unwrap_or("").trim().to_string();
    if name.len() > MAX_NAME {
        return Err(EngineError::Param(format!("param 'name' must be at most {MAX_NAME} bytes")));
    }
    if from == to {
        return Err(EngineError::Param("a bone joins two different instances".into()));
    }
    let missing = |id: ElementId| EngineError::State(format!("no instance with id {} on the stage at frame {}", id.0, s.frame + 1));
    let f = find_instance(s, from).ok_or_else(|| missing(from))?;
    let t = find_instance(s, to).ok_or_else(|| missing(to))?;
    for x in [&f, &t] {
        if !matches!(x.el.kind, ElementKind::Symbol { .. }) {
            return Err(EngineError::State("bones join symbol instances: convert the artwork to a symbol first (Modify › Convert to Symbol, F8)".into()));
        }
        if !matches!(x.kind, LayerKind::Normal | LayerKind::Armature) {
            return Err(EngineError::State("bones join instances on normal layers (not on guide or mask layers)".into()));
        }
        unlocked(s, x.layer)?;
    }
    let tl = s.timeline();
    let arm_of = |x: &Found| (x.kind == LayerKind::Armature).then(|| tl.layer(x.layer).and_then(|l| l.armature.clone()).unwrap_or_default());
    let (fa, ta) = (arm_of(&f), arm_of(&t));
    match (&fa, &ta) {
        (_, Some(a)) if f.layer == t.layer && a.bone_into(to).is_some() => return Err(EngineError::State("that instance already hangs from a bone; each instance has one bone into it".into())),
        (_, Some(_)) if f.layer == t.layer => return Err(EngineError::State("that instance is the armature's root: drag from the root to the other instances instead".into())),
        (None, Some(_)) => return Err(EngineError::State("start the bone on the armature: drag from one of its instances to the new one".into())),
        (Some(_), Some(_)) => return Err(EngineError::State("those instances are in different armatures".into())),
        _ => {}
    }
    if fa.as_ref().is_some_and(|a| a.bones.len() >= MAX_BONES) {
        return Err(EngineError::State(format!("an armature has at most {MAX_BONES} bones")));
    }
    let (wf, wt) = (f.el.transform, t.el.transform);
    let singular = || EngineError::State("an instance scaled to nothing can't hold a bone".into());
    let (inv_f, inv_t) = (wf.inverse().ok_or_else(singular)?, wt.inverse().ok_or_else(singular)?);
    // The joint. An instance that already hangs from a bone has one joint, that bone's end: every
    // bone from it starts there, wherever on the instance the drag began, so the chain stays
    // joined end to start and turns about the joints it shows, as in Animate (#239). A root's
    // bones start where given (a torso's shoulders and hips), else at its registration point.
    let into_from = fa.as_ref().and_then(|a| a.bone_into(from)).map(|b| wf.apply(b.tail));
    let head = into_from.or(head).unwrap_or(wf.translation());
    let tail = tail.unwrap_or(wt.translation());
    let rest = inv_f.mul(&wt);
    if !rest.is_finite() || rest.inverse().is_none() {
        return Err(singular());
    }
    let id = BoneId(s.doc.alloc_id());
    let bone = Bone { name, ..Bone::new(id, from, to, inv_f.apply(head), inv_t.apply(tail), rest) };
    take_off(s, t.layer, to)?;
    let lid = match fa {
        Some(_) => {
            // Another bone on this armature: the new instance joins every pose, at angle 0.
            let l = s.layer_mut(Some(f.layer))?;
            let mut arm = l.armature.as_deref().cloned().unwrap_or_default();
            arm.bones.push(bone);
            l.armature = Some(Arc::new(arm));
            for k in &mut l.keyframes {
                Arc::make_mut(k).elements.push(t.el.clone());
            }
            rebake(l);
            f.layer
        }
        None => {
            // The first bone: both instances move to a new armature layer above them.
            take_off(s, f.layer, from)?;
            let arm = Armature { bones: vec![bone], ..Default::default() };
            armature_layer(s, f.layer, f.index.min(t.index), vec![f.el.clone(), t.el.clone()], arm)?
        }
    };
    s.layer = lid;
    s.selection.clear();
    Ok(json!({"bone": id.0, "layer": lid.0, "created": lid != f.layer}))
}

/// How many keyframes of an armature layer hold a pose of their own (more than one: the
/// armature is animated).
pub(crate) fn poses(l: &Layer) -> usize {
    l.keyframes.iter().filter(|k| !k.elements.is_empty()).count()
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    let id = bone_param(p)?;
    let (lid, arm) = find_bone(s, id)?;
    unlocked(s, lid)?;
    let gone = arm.subtree(id);
    let mut left = (*arm).clone();
    left.bones.retain(|b| !gone.contains(&b.id));
    let dissolve = left.is_empty();
    // A shape armature's bones move no instances: only without bones does its shape go.
    let skinned = arm.skin.is_some();
    skin::rebind(&mut left);
    let freed: Vec<ElementId> = if dissolve { arm.members() } else if skinned { Vec::new() } else { arm.bones.iter().filter(|b| gone.contains(&b.id)).map(|b| b.child).collect() };
    let tl = s.timeline();
    let index = tl.layers.iter().position(|l| l.id == lid).ok_or_else(|| EngineError::State("the armature layer went away".into()))?;
    let layer = tl.layer(lid).ok_or_else(|| EngineError::State("the armature layer went away".into()))?;
    let (start, end) = (layer.keyframes.first().map_or(0, |k| k.start), layer.end_frame());
    // The freed instances as they look at the playhead (or at the armature's first frame).
    let at = s.frame.clamp(start, end.saturating_sub(1).max(start));
    let shown: Vec<Arc<Element>> = evaluate_layer_elements(layer, at).map(|(_, e)| e.iter().filter(|e| freed.contains(&e.id)).cloned().collect()).unwrap_or_default();
    let folder = layer.parent.filter(|p| tl.layer(*p).is_some_and(|x| x.kind == LayerKind::Folder));
    let new_id = LayerId(s.doc.alloc_id());
    let mut freed_layer = Layer::new(new_id, fresh_name(s, "Layer "));
    freed_layer.parent = folder;
    let mut k = Keyframe::new(start, end.saturating_sub(start).max(1));
    k.elements = shown;
    freed_layer.keyframes = vec![Arc::new(k)];
    let tl = crate::editing::edited_mut(&mut s.doc, s.scene, &s.editing)?;
    if dissolve {
        tl.layers.retain(|l| l.id != lid);
        tl.layers.insert(index.min(tl.layers.len()), Arc::new(freed_layer));
    } else {
        let l = tl.layer_mut(lid).ok_or_else(|| EngineError::State("the armature layer went away".into()))?;
        for k in &mut l.keyframes {
            let k = Arc::make_mut(k);
            k.elements.retain(|e| !freed.contains(&e.id));
            k.pose.retain(|b| !gone.contains(&b));
        }
        l.armature = Some(Arc::new(left));
        rebake(l);
        if !skinned {
            tl.layers.insert((index + 1).min(tl.layers.len()), Arc::new(freed_layer));
        }
    }
    tl.repair_masks();
    s.layer = if dissolve { new_id } else { lid };
    s.selection.clear();
    let layer = if dissolve || !skinned { new_id } else { lid };
    Ok(json!({"removed": gone.iter().map(|b| b.0).collect::<Vec<_>>(), "freed": freed.iter().map(|e| e.0).collect::<Vec<_>>(), "layer": layer.0, "armature_removed": dissolve}))
}

fn set_limits(s: &mut Session, p: &Value) -> Result<Value> {
    let id = bone_param(p)?;
    let constrain = opt_bool(p, "constrain")?;
    let (min, max) = (opt_f32(p, "min")?, opt_f32(p, "max")?);
    let deg = MAX_LIMIT.to_degrees();
    for v in [min, max].into_iter().flatten() {
        if !(-deg..=deg).contains(&v) {
            return Err(EngineError::Param(format!("joint limits must be between −{deg:.0}° and {deg:.0}°")));
        }
    }
    let (lid, arm) = find_bone(s, id)?;
    unlocked(s, lid)?;
    let cur = arm.bone(id).and_then(|b| b.limits);
    let limits = if constrain == Some(false) {
        None
    } else {
        // Turning limits on without numbers: a quarter turn either way.
        let base = cur.unwrap_or(Limits { min: -std::f32::consts::FRAC_PI_2, max: std::f32::consts::FRAC_PI_2 });
        let l = Limits { min: min.map_or(base.min, f32::to_radians), max: max.map_or(base.max, f32::to_radians) };
        if l.min > l.max {
            return Err(EngineError::Param("the joint's min must be at most its max".into()));
        }
        Some(l)
    };
    let l = s.layer_mut(Some(lid))?;
    let mut arm = (*arm).clone();
    if let Some(b) = arm.bone_mut(id) {
        b.limits = limits;
    }
    // Every pose keeps to the new limits.
    for k in &mut l.keyframes {
        if let Some(b) = arm.bone(id) {
            let a = k.pose.angle(id);
            if b.allow(a) != a {
                Arc::make_mut(k).pose.set(id, b.allow(a));
            }
        }
    }
    l.armature = Some(Arc::new(arm));
    rebake(l);
    Ok(json!({"bone": id.0, "constrain": limits.is_some(), "min": limits.map(|l| l.min.to_degrees()), "max": limits.map(|l| l.max.to_degrees())}))
}

/// The translation limits of an axis that slides freely.
const FREE: Limits = Limits { min: -MAX_SLIDE, max: MAX_SLIDE };

fn set_translation(s: &mut Session, p: &Value) -> Result<Value> {
    let id = bone_param(p)?;
    let axis = str_req(p, "axis")?;
    let along_x = match axis {
        "x" => true,
        "y" => false,
        _ => return Err(EngineError::Param("param 'axis' must be \"x\" or \"y\"".into())),
    };
    let (enabled, constrain) = (opt_bool(p, "enabled")?, opt_bool(p, "constrain")?);
    let (min, max) = (opt_f32(p, "min")?, opt_f32(p, "max")?);
    for v in [min, max].into_iter().flatten() {
        if !(-MAX_SLIDE..=MAX_SLIDE).contains(&v) {
            return Err(EngineError::Param("joint translation limits must be within ±10,000 px".into()));
        }
    }
    let (lid, arm) = find_bone(s, id)?;
    unlocked(s, lid)?;
    let b = arm.bone(id).ok_or_else(|| EngineError::State("no such bone".into()))?;
    let cur = if along_x { b.slide_x } else { b.slide_y };
    let limits = if enabled == Some(false) {
        None
    } else if enabled.is_none() && constrain.is_none() && min.is_none() && max.is_none() {
        return Err(EngineError::Param("give 'enabled', 'constrain' or limits ('min', 'max')".into()));
    } else if constrain == Some(false) {
        Some(FREE)
    } else if constrain.is_none() && min.is_none() && max.is_none() {
        // Enabling: freely, or as it was.
        Some(cur.unwrap_or(FREE))
    } else {
        // Constraining without numbers: 50 px either way.
        let base = cur.filter(|l| *l != FREE).unwrap_or(Limits { min: -50.0, max: 50.0 });
        let l = Limits { min: min.unwrap_or(base.min), max: max.unwrap_or(base.max) };
        if l.min > l.max {
            return Err(EngineError::Param("the joint's min must be at most its max".into()));
        }
        Some(l)
    };
    let mut arm = (*arm).clone();
    if let Some(b) = arm.bone_mut(id) {
        if along_x {
            b.slide_x = limits;
        } else {
            b.slide_y = limits;
        }
    }
    let l = s.layer_mut(Some(lid))?;
    // Every pose keeps to the new limits.
    if let Some(b) = arm.bone(id) {
        for k in &mut l.keyframes {
            let v = k.pose.slide(id);
            if b.allow_slide(v) != v {
                Arc::make_mut(k).pose.set_slide(id, b.allow_slide(v));
            }
        }
    }
    l.armature = Some(Arc::new(arm));
    rebake(l);
    Ok(json!({"bone": id.0, "axis": axis, "enabled": limits.is_some(), "constrain": limits.is_some_and(|l| l != FREE), "min": limits.map(|l| l.min), "max": limits.map(|l| l.max)}))
}

fn set_spring(s: &mut Session, p: &Value) -> Result<Value> {
    let id = bone_param(p)?;
    let (strength, damping) = (opt_f32(p, "strength")?, opt_f32(p, "damping")?);
    if strength.is_none() && damping.is_none() {
        return Err(EngineError::Param("give 'strength' or 'damping' (0–100)".into()));
    }
    for (k, v) in [("strength", strength), ("damping", damping)] {
        if v.is_some_and(|v| !(0.0..=100.0).contains(&v)) {
            return Err(EngineError::Param(format!("param '{k}' must be between 0 and 100")));
        }
    }
    let (lid, arm) = find_bone(s, id)?;
    unlocked(s, lid)?;
    let mut arm = (*arm).clone();
    let springs = !arm.springs_off;
    let b = arm.bone_mut(id).ok_or_else(|| EngineError::State("no such bone".into()))?;
    let cur = b.spring.unwrap_or(Spring { strength: 0.0, damping: 0.0 });
    let next = Spring { strength: strength.map_or(cur.strength, |v| v / 100.0), damping: damping.map_or(cur.damping, |v| v / 100.0) };
    // Nothing set is no spring: the file says nothing.
    b.spring = (next.strength > 0.0 || next.damping > 0.0).then_some(next);
    s.layer_mut(Some(lid))?.armature = Some(Arc::new(arm));
    Ok(json!({"bone": id.0, "strength": next.strength * 100.0, "damping": next.damping * 100.0, "springs": springs}))
}

fn enable_springs(s: &mut Session, p: &Value) -> Result<Value> {
    let enabled = opt_bool(p, "enabled")?.ok_or_else(|| EngineError::Param("param 'enabled' (true or false) is required".into()))?;
    let lid = match (opt_u64(p, "bone")?.map(BoneId), opt_u64(p, "layer")?) {
        (Some(b), _) => find_bone(s, b)?.0,
        (None, Some(l)) => LayerId(l),
        (None, None) => s.layer,
    };
    unlocked(s, lid)?;
    let l = s.layer_mut(Some(lid))?;
    let mut arm = l.armature.as_deref().cloned().filter(|_| l.kind == LayerKind::Armature).ok_or_else(|| EngineError::State("that is not an armature layer".into()))?;
    arm.springs_off = !enabled;
    l.armature = Some(Arc::new(arm));
    Ok(json!({"layer": lid.0, "springs": enabled}))
}

fn pin(s: &mut Session, p: &Value) -> Result<Value> {
    let id = bone_param(p)?;
    let pinned = opt_bool(p, "pinned")?.ok_or_else(|| EngineError::Param("param 'pinned' (true or false) is required".into()))?;
    let (lid, arm) = find_bone(s, id)?;
    unlocked(s, lid)?;
    let mut arm = (*arm).clone();
    if let Some(b) = arm.bone_mut(id) {
        b.pinned = pinned;
    }
    s.layer_mut(Some(lid))?.armature = Some(Arc::new(arm));
    Ok(json!({"bone": id.0, "pinned": pinned}))
}

/// Runs `edit` on the keyframe of armature layer `lid` that a pose at `frame` changes (keying
/// the frame inside a pose tween, extending the layer past its end), then places the
/// keyframe's instances by its pose.
fn edit_pose<T>(s: &mut Session, lid: LayerId, frame: u32, edit: impl FnOnce(&Armature, &mut Keyframe) -> Result<T>) -> Result<T> {
    if frame > MAX_FRAME {
        return Err(EngineError::Param(format!("param 'frame' must be at most {MAX_FRAME}")));
    }
    let arm = s.timeline().layer(lid).and_then(|l| l.armature.clone()).ok_or_else(|| EngineError::State("that is not an armature layer".into()))?;
    // Past the end a pose carries on from the last one (not a blank keyframe).
    {
        let l = s.layer_mut(Some(lid))?;
        if l.keyframe_at(frame).is_none() && frame >= l.end_frame() && !l.locked {
            insert_keyframe_at(l, frame, true)?;
        }
    }
    let (layer, at) = (s.layer, s.frame);
    s.layer = lid;
    s.frame = frame;
    let out = (|| {
        let kf = s.editable_keyframe_in(KeyEdit::Pose)?;
        let v = edit(&arm, kf)?;
        kf.pose = kf.pose.allowed(&arm);
        armature::apply(&arm, &mut kf.elements, &kf.pose);
        Ok(v)
    })();
    s.layer = layer;
    s.frame = at;
    out
}

fn pose(s: &mut Session, p: &Value) -> Result<Value> {
    let bone = opt_u64(p, "bone")?.map(BoneId);
    let (angle, rotate_by, dx, dy) = (opt_f32(p, "angle")?, opt_f32(p, "rotate_by")?, opt_f32(p, "dx")?, opt_f32(p, "dy")?);
    let (slide_x, slide_y) = (opt_f32(p, "slide_x")?, opt_f32(p, "slide_y")?);
    if [slide_x, slide_y].into_iter().flatten().any(|v| !v.is_finite() || v.abs() > MAX_SLIDE) {
        return Err(EngineError::Param("params 'slide_x' and 'slide_y' must be within ±10,000 px".into()));
    }
    let frame = opt_u32(p, "frame")?.unwrap_or(s.frame);
    let lid = match (bone, opt_u64(p, "layer")?) {
        (Some(b), _) => find_bone(s, b)?.0,
        (None, Some(l)) => LayerId(l),
        (None, None) => s.layer,
    };
    if bone.is_none() && (angle.is_some() || rotate_by.is_some() || slide_x.is_some() || slide_y.is_some()) {
        return Err(EngineError::Param("param 'bone' says which joint 'angle', 'rotate_by' or the slide moves".into()));
    }
    let v = edit_pose(s, lid, frame, |arm, kf| {
        let mut out = json!({"layer": lid.0});
        if let Some(b) = bone.and_then(|b| arm.bone(b)) {
            let cur = kf.pose.angle(b.id);
            let next = b.allow(angle.map_or(cur, f32::to_radians) + rotate_by.unwrap_or(0.0).to_radians());
            kf.pose.set(b.id, next);
            out["bone"] = json!(b.id.0);
            out["angle"] = json!(next.to_degrees());
            if slide_x.is_some() || slide_y.is_some() {
                if !b.slides() {
                    return Err(EngineError::State("that joint doesn't slide: turn on its X or Y translation first (bone.set_translation)".into()));
                }
                let was = kf.pose.slide(b.id);
                let v = b.allow_slide(Vec2::new(slide_x.unwrap_or(was.x), slide_y.unwrap_or(was.y)));
                kf.pose.set_slide(b.id, v);
                out["slide"] = json!([v.x, v.y]);
            }
        }
        if dx.is_some() || dy.is_some() {
            let by = Affine::translate(dx.unwrap_or(0.0), dy.unwrap_or(0.0));
            let roots = arm.roots();
            for e in kf.elements.iter_mut().filter(|e| roots.contains(&e.id)) {
                let m = by.mul(&e.transform);
                let t = m.translation();
                if m.is_finite() && t.x.abs() <= MAX_COORD && t.y.abs() <= MAX_COORD {
                    Arc::make_mut(e).transform = m;
                }
            }
        }
        Ok(out)
    })?;
    Ok(v)
}

fn solve(s: &mut Session, p: &Value) -> Result<Value> {
    let id = bone_param(p)?;
    let target = Vec2::new(f32_req(p, "x")?, f32_req(p, "y")?);
    if target.x.abs() > MAX_COORD || target.y.abs() > MAX_COORD {
        return Err(EngineError::Param("the target must be within ±1,000,000 px".into()));
    }
    let local = opt_point(p, "local")?;
    let frame = opt_u32(p, "frame")?.unwrap_or(s.frame);
    let (lid, arm) = find_bone(s, id)?;
    if armature::movable_chain(&arm, id).is_empty() {
        return Err(EngineError::State("that bone is pinned, or hangs from a pinned bone: unpin it to drag it".into()));
    }
    let tail = arm.bone(id).map_or(Vec2::new(0.0, 0.0), |b| b.tail);
    edit_pose(s, lid, frame, |arm, kf| {
        let r = armature::solve(arm, &kf.elements, &kf.pose, id, local.unwrap_or(tail), target).ok_or_else(|| EngineError::State("the bone's instances aren't on this frame".into()))?;
        kf.pose = r.pose;
        Ok(json!({"bone": id.0, "reached": r.error <= 0.5, "error": r.error, "chain": r.chain}))
    })
}

fn info(s: &mut Session, p: &Value) -> Result<Value> {
    let frame = opt_u32(p, "frame")?.unwrap_or(s.frame).min(MAX_FRAME);
    let only = opt_u64(p, "layer")?.map(LayerId);
    let with_points = opt_bool(p, "points")?.unwrap_or(false);
    let tl = s.timeline();
    let deg = |v: f32| v.to_degrees();
    let pt = |v: Vec2| json!([v.x, v.y]);
    let list: Vec<Value> = tl
        .layers
        .iter()
        .filter(|l| l.kind == LayerKind::Armature && only.is_none_or(|o| o == l.id))
        .filter_map(|l| {
            let arm = l.armature.as_deref()?;
            let pose = armature::pose_at(l, frame).unwrap_or_default();
            let shown = anicraft_doc::spring::sprung_pose(l, frame, s.doc.stage.fps).unwrap_or_default();
            let (_, els) = evaluate_layer_elements(l, frame).unwrap_or((0, std::borrow::Cow::Borrowed(&[])));
            let at = armature::bones_at(arm, &els, &pose);
            let limits = |l: Option<Limits>| l.map(|l| json!({"min": l.min, "max": l.max, "constrain": l != FREE}));
            let bones: Vec<Value> = arm
                .bones
                .iter()
                .map(|b| {
                    let g = at.iter().find(|g| g.id == b.id);
                    json!({
                        "id": b.id.0, "name": b.name, "from": b.parent.0, "to": b.child.0,
                        "parent_bone": arm.parent_bone(b).map(|x| x.id.0),
                        "head": g.map(|g| pt(g.head)), "tail": g.map(|g| pt(g.tail)),
                        "angle": deg(b.allow(pose.angle(b.id))),
                        "min": b.limits.map(|l| deg(l.min)), "max": b.limits.map(|l| deg(l.max)),
                        "pinned": b.pinned,
                        "slide": g.map(|g| pt(g.slide)),
                        "slide_x": limits(b.slide_x), "slide_y": limits(b.slide_y),
                        "spring": b.spring.map(|sp| json!({"strength": sp.strength * 100.0, "damping": sp.damping * 100.0})),
                        "swing": deg(shown.angle(b.id) - b.allow(pose.angle(b.id))),
                        "points": arm.skin.as_ref().map(|k| k.points.iter().filter(|x| x.weight(b.id) > 0.0).count()),
                    })
                })
                .collect();
            let mut out = json!({"layer": l.id.0, "name": l.name, "roots": arm.roots().iter().map(|e| e.0).collect::<Vec<_>>(), "poses": poses(l), "bones": bones, "shape": arm.skin_shape().map(|e| e.0), "springs": !arm.springs_off});
            if with_points && let Some(k) = &arm.skin {
                out["points"] = json!(skin::points_shown(arm, &els).iter().zip(&k.points).map(|(at, b)| json!({"at": pt(*at), "bones": b.bones.iter().map(|i| json!([i.0.0, i.1])).collect::<Vec<_>>(), "manual": b.manual})).collect::<Vec<_>>());
            }
            Some(out)
        })
        .collect();
    Ok(json!({"frame": frame, "armatures": list}))
}

fn add_in_shape(s: &mut Session, p: &Value) -> Result<Value> {
    let shape = opt_u64(p, "shape")?.map(ElementId);
    let parent = opt_u64(p, "parent")?.map(BoneId);
    let head = opt_point(p, "head")?;
    let tail = opt_point(p, "tail")?.ok_or_else(|| EngineError::Param("param 'tail' ([x, y], stage px: where the bone ends) is required".into()))?;
    let name = opt_str(p, "name")?.unwrap_or("").trim().to_string();
    if name.len() > MAX_NAME {
        return Err(EngineError::Param(format!("param 'name' must be at most {MAX_NAME} bytes")));
    }
    let frame = s.frame;
    // The armature the bone joins (none yet for the first bone), the shape, the node the bone
    // starts on, and its joint on the stage.
    let (existing, found, from, head) = match (parent, shape) {
        (Some(b), _) => {
            let (lid, arm) = find_bone(s, b)?;
            let sh = arm.skin_shape().ok_or_else(|| EngineError::State("that bone joins symbol instances: continue it with bone.add".into()))?;
            if shape.is_some_and(|x| x != sh) {
                return Err(EngineError::Param("that bone is in another shape".into()));
            }
            let f = find_instance(s, sh).ok_or_else(|| EngineError::State(format!("the armature's shape isn't on the stage at frame {}", frame + 1)))?;
            let node = arm.bone(b).map(|x| x.child).ok_or_else(|| EngineError::State("no such bone".into()))?;
            let end = s.timeline().layer(lid).and_then(|l| {
                let (_, els) = evaluate_layer_elements(l, frame)?;
                let pose = armature::pose_at(l, frame).unwrap_or_default();
                armature::bones_at(&arm, &els, &pose).into_iter().find(|g| g.id == b).map(|g| g.tail)
            });
            let head = head.or(end).ok_or_else(|| EngineError::State(format!("the bone isn't on the stage at frame {}", frame + 1)))?;
            (Some((lid, arm)), f, node, head)
        }
        (None, Some(sh)) => {
            let f = find_instance(s, sh).ok_or_else(|| EngineError::State(format!("no shape with id {} on the stage at frame {}", sh.0, frame + 1)))?;
            let ElementKind::Shape(shp) = &f.el.kind else {
                return Err(EngineError::State("bones inside a shape need a shape: for symbol instances, drag from one to another (bone.add)".into()));
            };
            let head = head.ok_or_else(|| EngineError::Param("param 'head' ([x, y], stage px: where the bone starts, inside the shape) is required".into()))?;
            let existing = match f.kind {
                LayerKind::Normal => None,
                LayerKind::Armature => {
                    let arm = s.timeline().layer(f.layer).and_then(|l| l.armature.clone()).filter(|a| a.skin_shape() == Some(sh)).ok_or_else(|| EngineError::State("that shape isn't the armature's".into()))?;
                    Some((f.layer, arm))
                }
                _ => return Err(EngineError::State("bones go inside shapes on normal layers (not on guide or mask layers)".into())),
            };
            // A bone starts inside the shape (on its fill, or on its line).
            let reach = shp.stroke.as_ref().map_or(0.0, |k| k.width * 0.5 * f.el.transform.mean_scale()) + 2.0;
            let inside = (shp.fill.is_some() && shp.path.contains(head, &f.el.transform, shp.fill_rule)) || shp.path.distance_to_edge(head, &f.el.transform) <= reach;
            if !inside {
                return Err(EngineError::State("start the bone inside the shape".into()));
            }
            (existing, f, sh, head)
        }
        (None, None) => return Err(EngineError::Param("give 'shape' (the shape's id, for the first bone) or 'parent' (a bone to continue from)".into())),
    };
    unlocked(s, found.layer)?;
    if head.distance(tail) < 1.0 {
        return Err(EngineError::Param("a bone needs a length: drag from its joint to its end".into()));
    }
    let ElementKind::Shape(shp) = &found.el.kind else { return Err(EngineError::State("that isn't a shape".into())) };
    // The node the bone starts on, as placed at the playhead: the bone's points go back through
    // it to the shape as drawn (a new bone is at rest in every pose).
    let node = match &existing {
        Some((lid, arm)) => {
            let l = s.timeline().layer(*lid).ok_or_else(|| EngineError::State("the armature layer went away".into()))?;
            let (_, els) = evaluate_layer_elements(l, frame).ok_or_else(|| EngineError::State("the armature isn't on this frame".into()))?;
            let pose = armature::pose_at(l, frame).unwrap_or_default();
            armature::place(arm, &els, &pose).get(&from).copied()
        }
        None => Some(found.el.transform),
    };
    let inv = node.and_then(|m| m.inverse()).ok_or_else(|| EngineError::State("a shape scaled to nothing can't hold a bone".into()))?;
    let (h, t) = (inv.apply(head), inv.apply(tail));
    if !(h.x.is_finite() && h.y.is_finite() && t.x.is_finite() && t.y.is_finite()) {
        return Err(EngineError::State("a shape scaled to nothing can't hold a bone".into()));
    }
    if existing.as_ref().is_some_and(|(_, a)| a.bones.len() >= MAX_BONES) {
        return Err(EngineError::State(format!("an armature has at most {MAX_BONES} bones")));
    }
    if existing.is_none() && skin::anchors(&shp.path).len() > MAX_SKIN_POINTS {
        return Err(EngineError::State(format!("bones go inside shapes of at most {MAX_SKIN_POINTS} points: simplify it first (Modify › Shape › Optimize)")));
    }
    let id = BoneId(s.doc.alloc_id());
    let child = ElementId(s.doc.alloc_id());
    let bone = Bone { name, ..Bone::new(id, from, child, h, t, Affine::IDENTITY) };
    let (lid, created) = match existing {
        Some((lid, arm)) => {
            let mut arm = (*arm).clone();
            arm.bones.push(bone);
            skin::rebind(&mut arm);
            let l = s.layer_mut(Some(lid))?;
            l.armature = Some(Arc::new(arm));
            rebake(l);
            (lid, false)
        }
        None => {
            // An armature's shape is an object: merge drawing (ADR 0031) no longer cuts it.
            let mut el = (*found.el).clone();
            if let ElementKind::Shape(x) = &mut el.kind {
                x.merge = false;
            }
            let mut arm = Armature { bones: vec![bone], skin: Some(Skin::new(found.el.id, shp.path.clone())), ..Default::default() };
            skin::rebind(&mut arm);
            take_off(s, found.layer, found.el.id)?;
            (armature_layer(s, found.layer, found.index, vec![Arc::new(el)], arm)?, true)
        }
    };
    s.layer = lid;
    s.selection.clear();
    let bound = s.timeline().layer(lid).and_then(|l| l.armature.as_ref()).and_then(|a| a.skin.as_ref()).map_or(0, |k| k.points.iter().filter(|b| b.weight(id) > 0.0).count());
    Ok(json!({"bone": id.0, "layer": lid.0, "created": created, "points": bound}))
}

fn bind(s: &mut Session, p: &Value) -> Result<Value> {
    let id = bone_param(p)?;
    let mode = opt_str(p, "mode")?.unwrap_or("only");
    if !matches!(mode, "only" | "add" | "remove" | "auto") {
        return Err(EngineError::Param("param 'mode' must be \"only\", \"add\", \"remove\" or \"auto\"".into()));
    }
    let (lid, arm) = find_bone(s, id)?;
    unlocked(s, lid)?;
    let n = arm.skin.as_ref().map(|k| k.points.len()).ok_or_else(|| EngineError::State("that bone joins symbol instances: only bones inside a shape bind its points".into()))?;
    let bad = || EngineError::Param(format!("param 'points' must be a list of point indices below {n} (see bone.info with points)"));
    let points: Vec<usize> = match p.get("points") {
        None | Some(Value::Null) => (0..n).collect(),
        Some(Value::Array(a)) if a.len() <= MAX_SKIN_POINTS => a.iter().map(|v| v.as_u64().and_then(|i| usize::try_from(i).ok()).filter(|i| *i < n).ok_or_else(bad)).collect::<Result<_>>()?,
        Some(_) => return Err(bad()),
    };
    let mut arm = (*arm).clone();
    let segments = skin::rest_segments(&arm);
    let Some(k) = arm.skin.as_mut() else { return Err(EngineError::State("that bone isn't inside a shape".into())) };
    let anchors = skin::anchors(&k.rest);
    let mut changed = 0usize;
    for i in points {
        let (Some(b), Some(&at)) = (k.points.get_mut(i), anchors.get(i)) else { continue };
        let next = match mode {
            "only" => Binding { bones: vec![Influence(id, 1.0)], manual: true },
            "add" if b.weight(id) > 0.0 => continue,
            "add" => {
                // An equal share with the bones the point already follows.
                let share = 1.0 / (b.bones.len() + 1) as f32;
                let mut x = Binding { bones: b.bones.iter().map(|i| Influence(i.0, i.1 * (1.0 - share))).collect(), manual: true };
                x.bones.push(Influence(id, share));
                x.normalize(|_| true);
                x
            }
            "remove" => {
                let mut x = Binding { bones: b.bones.iter().copied().filter(|i| i.0 != id).collect(), manual: true };
                x.normalize(|_| true);
                x
            }
            _ => skin::auto_binding(&segments, at),
        };
        if *b != next {
            *b = next;
            changed += 1;
        }
    }
    let bound = k.points.iter().filter(|b| b.weight(id) > 0.0).count();
    let l = s.layer_mut(Some(lid))?;
    l.armature = Some(Arc::new(arm));
    rebake(l);
    Ok(json!({"bone": id.0, "changed": changed, "bound": bound}))
}

/// The point indices in param `key`, each below `n`.
fn point_list(p: &Value, key: &str, n: usize) -> Result<Vec<usize>> {
    let bad = || EngineError::Param(format!("param '{key}' must be a list of point indices below {n} (see bone.info with points)"));
    match p.get(key) {
        Some(Value::Array(a)) if a.len() <= MAX_SKIN_POINTS => a.iter().map(|v| v.as_u64().and_then(|i| usize::try_from(i).ok()).filter(|i| *i < n).ok_or_else(bad)).collect(),
        _ => Err(bad()),
    }
}

/// How many points bone `id` moves.
fn bound_count(arm: &Armature, id: BoneId) -> usize {
    arm.skin.as_ref().map_or(0, |k| k.points.iter().filter(|b| b.weight(id) > 0.0).count())
}

/// A bone inside a shape, its unlocked armature layer and the armature.
fn shape_bone(s: &Session, id: BoneId) -> Result<(LayerId, Arc<Armature>)> {
    let (lid, arm) = find_bone(s, id)?;
    unlocked(s, lid)?;
    if arm.skin.is_none() {
        return Err(EngineError::State("that bone joins symbol instances: only bones inside a shape have weights on its points".into()));
    }
    Ok((lid, arm))
}

fn paint_weights(s: &mut Session, p: &Value) -> Result<Value> {
    let id = bone_param(p)?;
    let bad = || EngineError::Param(format!("param 'at' must be a list of at most {MAX_DABS} [x, y] points (stage px, within ±1,000,000)"));
    let list = p.get("at").and_then(Value::as_array).filter(|a| a.len() <= MAX_DABS).ok_or_else(bad)?;
    let dabs: Vec<Vec2> = list
        .iter()
        .map(|v| {
            let a = v.as_array().filter(|a| a.len() == 2).ok_or_else(bad)?;
            let n = |i: usize| a.get(i).and_then(Value::as_f64).filter(|v| v.is_finite() && v.abs() <= f64::from(MAX_COORD)).map(|v| v as f32).ok_or_else(bad);
            Ok(Vec2::new(n(0)?, n(1)?))
        })
        .collect::<Result<_>>()?;
    let radius = opt_f32(p, "radius")?.unwrap_or(20.0);
    if !(1.0..=MAX_RADIUS).contains(&radius) {
        return Err(EngineError::Param(format!("param 'radius' must be between 1 and {MAX_RADIUS} px")));
    }
    let strength = opt_f32(p, "strength")?.unwrap_or(25.0);
    if !(0.0..=100.0).contains(&strength) {
        return Err(EngineError::Param("param 'strength' must be between 0 and 100".into()));
    }
    let brush = match opt_str(p, "mode")? {
        None => Brush::Add,
        Some(m) => Brush::parse(m).ok_or_else(|| EngineError::Param("param 'mode' must be \"add\", \"subtract\" or \"smooth\"".into()))?,
    };
    let frame = opt_u32(p, "frame")?.unwrap_or(s.frame).min(MAX_FRAME);
    let (lid, arm) = shape_bone(s, id)?;
    // The points where the stage shows them at the frame: the brush paints what the artist sees.
    let shown = s.timeline().layer(lid).and_then(|l| evaluate_layer_elements(l, frame)).map(|(_, els)| skin::points_shown(&arm, &els)).unwrap_or_default();
    if shown.is_empty() {
        return Err(EngineError::State(format!("the bone's shape isn't on the stage at frame {}", frame + 1)));
    }
    let mut arm = (*arm).clone();
    let segments = skin::rest_segments(&arm);
    let changed = match arm.skin.as_mut() {
        Some(k) => weights::paint(k, &segments, &shown, id, &dabs, Dab { brush, radius, strength: strength / 100.0 }),
        None => 0,
    };
    let bound = bound_count(&arm, id);
    if changed > 0 {
        let l = s.layer_mut(Some(lid))?;
        l.armature = Some(Arc::new(arm));
        rebake(l);
    }
    Ok(json!({"bone": id.0, "changed": changed, "bound": bound, "mode": brush.key()}))
}

fn set_weight(s: &mut Session, p: &Value) -> Result<Value> {
    let id = bone_param(p)?;
    let w = f32_req(p, "weight")?;
    if !(0.0..=100.0).contains(&w) {
        return Err(EngineError::Param("param 'weight' must be between 0 and 100".into()));
    }
    let (lid, arm) = shape_bone(s, id)?;
    let n = arm.skin.as_ref().map_or(0, |k| k.points.len());
    let points = point_list(p, "points", n)?;
    let mut arm = (*arm).clone();
    let segments = skin::rest_segments(&arm);
    let Some(k) = arm.skin.as_mut() else { return Err(EngineError::State("that bone isn't inside a shape".into())) };
    let rest = skin::anchors(&k.rest);
    let mut changed = 0usize;
    for &i in &points {
        let (Some(b), Some(&at)) = (k.points.get_mut(i), rest.get(i)) else { continue };
        let next = weights::with_share(b, id, w / 100.0, || weights::others_at(&segments, id, at));
        if *b != next {
            *b = next;
            changed += 1;
        }
    }
    let shares: Vec<Value> = points.iter().map(|&i| json!(k.points.get(i).map(|b| b.bones.iter().map(|x| json!([x.0.0, x.1 * 100.0])).collect::<Vec<_>>()).unwrap_or_default())).collect();
    let bound = bound_count(&arm, id);
    let l = s.layer_mut(Some(lid))?;
    l.armature = Some(Arc::new(arm));
    rebake(l);
    Ok(json!({"bone": id.0, "changed": changed, "bound": bound, "weights": shares}))
}

/// Whether element `id` is the shape of the shape armature on the active layer (so point edits
/// reshape the skin instead of a keyframe's artwork).
pub(crate) fn is_skin_shape(s: &Session, id: ElementId) -> bool {
    s.timeline().layer(s.layer).is_some_and(|l| l.kind == LayerKind::Armature && l.armature.as_ref().and_then(|a| a.skin_shape()) == Some(id))
}

/// Reshaping a bound shape (ADR 0090): `edit` changes the shape's outline as drawn (its rest
/// outline, with the shape's transform at the playhead), which needs the armature at rest there.
/// The binding is carried over to the new outline (moved points keep their weights, new points
/// blend their neighbours', deleted ones drop theirs) and every pose bends the new outline. One
/// undo step, like any command.
pub(crate) fn reshape<T>(s: &mut Session, id: ElementId, edit: impl FnOnce(&mut Shape, Affine) -> Result<T>) -> Result<T> {
    let (lid, frame) = (s.layer, s.frame);
    unlocked(s, lid)?;
    let tl = s.timeline();
    let l = tl.layer(lid).ok_or_else(|| EngineError::State("the armature layer went away".into()))?;
    let arm = l.armature.clone().ok_or_else(|| EngineError::State("that is not an armature layer".into()))?;
    let k = arm.skin.as_ref().filter(|k| k.shape == id).ok_or_else(|| EngineError::State("that shape isn't the armature's".into()))?;
    let pose = armature::pose_at(l, frame).ok_or_else(|| EngineError::State(format!("the armature isn't on frame {}", frame + 1)))?;
    if skin::matrices(&arm, &pose).values().any(|m| *m != Affine::IDENTITY) {
        return Err(EngineError::State("reshape a shape with bones at its rest pose: go to a frame where every bone is as drawn (angle 0, no slide), or pose them back".into()));
    }
    let (_, els) = evaluate_layer_elements(l, frame).ok_or_else(|| EngineError::State(format!("the armature isn't on frame {}", frame + 1)))?;
    let el = els.iter().find(|e| e.id == id).ok_or_else(|| EngineError::State(format!("element {} is not on the active layer at this frame", id.0)))?;
    let ElementKind::Shape(shape) = &el.kind else { return Err(EngineError::State("that isn't a shape".into())) };
    let mut shape = shape.clone();
    shape.path = k.rest.clone();
    let out = edit(&mut shape, el.transform)?;
    if *shape.path == *k.rest {
        return Ok(out);
    }
    let n = skin::anchors(&shape.path).len();
    if n > MAX_SKIN_POINTS {
        return Err(EngineError::State(format!("a shape with bones has at most {MAX_SKIN_POINTS} points")));
    }
    let skin = weights::reshaped(k, shape.path.clone(), &skin::rest_segments(&arm));
    let mut arm = (*arm).clone();
    arm.skin = Some(skin);
    let l = s.layer_mut(Some(lid))?;
    l.armature = Some(Arc::new(arm));
    rebake(l);
    Ok(out)
}
