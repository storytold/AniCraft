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
use anicraft_doc::armature::{self, Armature, Bone, BoneId, Limits, MAX_BONES, MAX_LIMIT, MAX_NAME};
use anicraft_doc::{Element, ElementId, ElementKind, Keyframe, Layer, LayerId, LayerKind, Rgba, Tween, evaluate_layer_elements};
use anicraft_geom::{Affine, Vec2};
use serde_json::{Value, json};
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, params, mutates, run| CommandSpec { id, label, menu: "", shortcut: None, params, mutates, run };
    vec![
        c(
            "bone.add",
            "Add Bone",
            "{from (instance id: the bone starts on it), to (instance id: the bone moves it), head?: [x, y] (the joint, stage px; default the end of the bone into `from`, else its registration point), tail?: [x, y] (default `to`'s registration point), name?} → {bone, layer}. Both instances go to an armature layer (made for the first bone)",
            true,
            add as crate::RunFn,
        ),
        c("bone.remove", "Remove Bone", "{bone} removes it and the bones below it; the instances they moved go to a layer of their own as they look at the playhead (with the last bone, the whole armature does); asks first when the armature has more than one pose", true, remove),
        c("bone.set_limits", "Joint Limits", "{bone, constrain?: bool (false: turns freely), min?, max? (°, −360…360, from where it was drawn)} limits how far the joint turns, in every pose", true, set_limits),
        c("bone.pin", "Pin Bone", "{bone, pinned: bool} a pinned bone (and the bones it hangs from) stays put while dragging bones below it", true, pin),
        c("bone.pose", "Pose Bone", "{bone?, angle? (°, the joint angle: 0 as drawn), rotate_by? (°), dx?, dy? (moves the whole armature), layer? (the armature layer, when no bone is given), frame?} keys the pose at the frame", true, pose),
        c("bone.solve", "Drag Bone", "{bone, x, y (stage px: where the bone's end goes), local?: [x, y] (the point dragged, in the bone's instance's space; default its end), frame?} → {reached, error (px), chain}; inverse kinematics turns the bone and the bones above it (limits respected, pinned bones kept)", true, solve),
        c("bone.info", "Armatures", "{frame?, layer?} → every armature on the scene at the frame: its bones with joint and end (stage px), joint angle and limits (°), pinned", false, info),
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
    // The joint: given, else the end of the bone into `from` (a chain joins end to start), else
    // its registration point.
    let into_from = fa.as_ref().and_then(|a| a.bone_into(from)).map(|b| wf.apply(b.tail));
    let head = head.or(into_from).unwrap_or(wf.translation());
    let tail = tail.unwrap_or(wt.translation());
    let rest = inv_f.mul(&wt);
    if !rest.is_finite() || rest.inverse().is_none() {
        return Err(singular());
    }
    let id = BoneId(s.doc.alloc_id());
    let bone = Bone { id, name, parent: from, child: to, head: inv_f.apply(head), tail: inv_t.apply(tail), rest, limits: None, pinned: false };
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
            // The first bone: both instances move to a new armature layer above them, whose one
            // pose spans from their keyframe to the end of the timeline.
            let (start, end) = s.timeline().layer(f.layer).and_then(|l| l.keyframe_at(s.frame)).map_or((s.frame, s.frame + 1), |k| (k.start, k.end()));
            let end = end.max(s.frame_count());
            take_off(s, f.layer, from)?;
            if s.timeline().layers.len() >= 1000 {
                return Err(EngineError::State("too many layers (limit 1000)".into()));
            }
            let lid = LayerId(s.doc.alloc_id());
            let mut layer = Layer::new(lid, fresh_name(s, "Armature_"));
            layer.kind = LayerKind::Armature;
            // Bones show in the layer colour: a green that reads on light and dark artwork.
            layer.color = Rgba::hex(0x2fb67c);
            layer.armature = Some(Arc::new(Armature { bones: vec![bone] }));
            let mut k = Keyframe::new(start, end.saturating_sub(start).max(1));
            k.elements = vec![f.el.clone(), t.el.clone()];
            k.tween = Some(Tween::default());
            layer.keyframes = vec![Arc::new(k)];
            let index = f.index.min(t.index);
            let tl = crate::editing::edited_mut(&mut s.doc, s.scene, &s.editing);
            layer.parent = tl.container_at(index, LayerKind::Armature);
            tl.layers.insert(index.min(tl.layers.len()), Arc::new(layer));
            tl.repair_masks();
            lid
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
    let freed: Vec<ElementId> = if dissolve { arm.members() } else { arm.bones.iter().filter(|b| gone.contains(&b.id)).map(|b| b.child).collect() };
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
    let tl = crate::editing::edited_mut(&mut s.doc, s.scene, &s.editing);
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
        tl.layers.insert((index + 1).min(tl.layers.len()), Arc::new(freed_layer));
    }
    tl.repair_masks();
    s.layer = if dissolve { new_id } else { lid };
    s.selection.clear();
    Ok(json!({"removed": gone.iter().map(|b| b.0).collect::<Vec<_>>(), "freed": freed.iter().map(|e| e.0).collect::<Vec<_>>(), "layer": new_id.0, "armature_removed": dissolve}))
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
    let frame = opt_u32(p, "frame")?.unwrap_or(s.frame);
    let lid = match (bone, opt_u64(p, "layer")?) {
        (Some(b), _) => find_bone(s, b)?.0,
        (None, Some(l)) => LayerId(l),
        (None, None) => s.layer,
    };
    if bone.is_none() && (angle.is_some() || rotate_by.is_some()) {
        return Err(EngineError::Param("param 'bone' says which joint 'angle' or 'rotate_by' turns".into()));
    }
    let v = edit_pose(s, lid, frame, |arm, kf| {
        let mut out = json!({"layer": lid.0});
        if let Some(b) = bone.and_then(|b| arm.bone(b)) {
            let cur = kf.pose.angle(b.id);
            let next = b.allow(angle.map_or(cur, f32::to_radians) + rotate_by.unwrap_or(0.0).to_radians());
            kf.pose.set(b.id, next);
            out["bone"] = json!(b.id.0);
            out["angle"] = json!(next.to_degrees());
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
            let (_, els) = evaluate_layer_elements(l, frame).unwrap_or((0, std::borrow::Cow::Borrowed(&[])));
            let at = armature::bones_at(arm, &els, &pose);
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
                    })
                })
                .collect();
            Some(json!({"layer": l.id.0, "name": l.name, "roots": arm.roots().iter().map(|e| e.0).collect::<Vec<_>>(), "poses": poses(l), "bones": bones}))
        })
        .collect();
    Ok(json!({"frame": frame, "armatures": list}))
}
