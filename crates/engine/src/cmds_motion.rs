//! Motion tweens (ADR 0016): creating one on a span, property keyframes, per-segment eases,
//! and auto-keying edits made to the tweened object.
//!
//! Auto-key works for every command without the commands knowing: when a command asks for the
//! keyframe to edit ([`Session::editable_keyframe`]) and the playhead is inside a motion tween,
//! [`begin_edit`] swaps the target for how it looks at the playhead, and after the command
//! [`finish_edit`] turns whatever changed in its transform or colour effect into property keys
//! and puts the base object back (with any other change, a new fill say, kept).

use crate::cmds_timeline::target;
use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::{ColorEffect, Element, ElementId, Keyframe, Layer, LayerId, LayerKind, MotionTween, Property, Props};
use anicraft_geom::{Affine, Vec2};
use serde_json::{Value, json};
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c("motion.set_key", "Property Keyframe", "", None, "{property: x|y|scale_x|scale_y|rotation|skew|alpha|tint, value (px, %, °), frame?, layer?}", true, set_key as crate::RunFn),
        c("motion.remove_key", "Remove Property Keyframe", "", None, "{property? (default: every property), frame?, layer?}", true, remove_key),
        c("motion.set_ease", "Property Ease", "", None, "{easing, property? (default: every property), frame? (inside the segment), layer?}", true, set_ease),
        c("motion.info", "Motion Tween", "", None, "{layer?, frame?} → the span's properties, keys, eases and values at the frame, its motion path and where the object is", false, info),
        c("motion.set_path", "Edit Motion Path", "", None, "{d: SVG path data in stage space (its first subpath; ends are the start and end positions), layer?, frame?}", true, set_path),
        c("motion.clear_path", "Remove Motion Path", "", None, "{layer?, frame?}: back to the straight path the X/Y curves trace", true, clear_path),
        c("motion.orient", "Orient to Path", "", None, "{orient: bool, layer?, frame?}: turn the object as its motion path turns", true, orient),
        c("motion.add_point", "Add Motion Path Point", "", None, "{x, y, slop? (stage units, default 4) | segment, t (0..1), layer?, frame?}: an anchor on the motion path, its shape unchanged → {anchor}", true, add_point),
        c("motion.delete_points", "Delete Motion Path Points", "", None, "{anchors: [index, ..] (as motion.info lists them; the ends stay), layer?, frame?}: the path refits through the rest", true, delete_points),
        c("motion.convert_point", "Convert Motion Path Point", "", None, "{anchor, smooth: bool, layer?, frame?}: smooth (handles in line) or corner (handles on the anchor)", true, convert_point),
        c("motion.move_path", "Move Motion Path", "", None, "{dx, dy, layer?, frame?}: the whole path with its object, keeping the timing", true, move_path),
    ]
}

/// The most path data `motion.set_path` reads.
const MAX_PATH_DATA: usize = 1_000_000;

/// A command editing a motion tween's target (see the module docs).
#[derive(Clone, Debug)]
pub(crate) struct MotionEdit {
    layer: LayerId,
    /// The span's first frame and the playhead's offset into it.
    start: u32,
    offset: u32,
    target: ElementId,
    /// The target as the command found it (at the playhead), and its base.
    before: Element,
    base: Element,
}

/// Starts an edit of the motion tween on `kf` at `frame`: the target is replaced by how it
/// looks there. `None` when `kf` has no motion tween (or no target).
pub(crate) fn begin_edit(layer: LayerId, kf: &mut Keyframe, frame: u32) -> Option<MotionEdit> {
    let m = kf.motion.as_ref()?;
    let i = m.target_index(&kf.elements)?;
    let offset = frame.checked_sub(kf.start)?;
    let base = (**kf.elements.get(i)?).clone();
    let before = m.apply(&base, offset as f32);
    if before != base
        && let Some(slot) = kf.elements.get_mut(i)
    {
        *slot = Arc::new(before.clone());
    }
    Some(MotionEdit { layer, start: kf.start, offset, target: base.id, before, base })
}

/// Turns what a command changed on the target into property keys at the playhead, and puts
/// the base back. On the span's first frame a property without keys changes the base itself.
pub(crate) fn finish_edit(s: &mut Session, e: MotionEdit) {
    let scene = s.scene;
    let Some(layer) = s.doc.timeline_mut(scene).layer_mut(e.layer) else { return };
    let Some(kf) = layer.keyframe_starting_at(e.start).and_then(|i| layer.keyframe_mut(i)) else { return };
    let Some(idx) = kf.elements.iter().position(|x| x.id == e.target) else { return };
    let Some(after) = kf.elements.get(idx).map(|x| (**x).clone()) else { return };
    let last = kf.duration.saturating_sub(1);
    let Some(m) = kf.motion.as_mut() else { return };
    m.target = e.target;
    let (now_dec, was_dec) = (Props::of(&after), Props::of(&e.before));
    let base_props = Props::of(&e.base);
    // The values the curves give here (rotation unwrapped: a key may hold 540°).
    let was = m.props_at(&base_props, e.offset as f32);
    let mut new_base = base_props;
    let (mut base_xf, mut base_color) = (false, false);
    // On a motion path, moving the object reshapes the path (ADR 0030); X/Y keep the timing.
    let on_path = m.path.is_some();
    if on_path {
        let d = Vec2::new(now_dec.get(Property::X) - was_dec.get(Property::X), now_dec.get(Property::Y) - was_dec.get(Property::Y));
        if d.is_finite() && d.length() > 1e-4 {
            move_on_path(m, &base_props, e.offset, d, last);
        }
    }
    for p in Property::ALL {
        if on_path && matches!(p, Property::X | Property::Y) {
            continue;
        }
        let mut d = now_dec.get(p) - was_dec.get(p);
        if p == Property::Rotation {
            // The turn the edit made, the short way round.
            d = (d + 180.0).rem_euclid(360.0) - 180.0;
        }
        if !d.is_finite() || d.abs() <= 1e-4 {
            continue;
        }
        let v = was.get(p) + d;
        if e.offset == 0 && m.track(p).is_none() {
            new_base.set(p, v);
            if matches!(p, Property::Alpha | Property::Tint) {
                base_color = true;
            } else {
                base_xf = true;
            }
        } else {
            m.set_key(p, e.offset, v, base_props.get(p));
        }
    }
    let tracks_xf = m.tracks.iter().any(|t| !matches!(t.property, Property::Alpha | Property::Tint));
    let tracks_color = m.tracks.iter().any(|t| matches!(t.property, Property::Alpha | Property::Tint));
    let mut base = after;
    // Exact where nothing is keyed on the first frame: the edited object is the new base.
    base.transform = match (base_xf, e.offset == 0 && !tracks_xf) {
        (true, true) => base.transform,
        (true, false) => new_base.transform(),
        (false, _) => e.base.transform,
    };
    base.color = match (base_color, e.offset == 0 && !tracks_color) {
        (true, true) => base.color,
        (true, false) => ColorEffect { alpha: (new_base.get(Property::Alpha) / 100.0).clamp(0.0, 1.0), tint: base.color.tint, tint_amount: (new_base.get(Property::Tint) / 100.0).clamp(0.0, 1.0) },
        (false, _) => ColorEffect { tint: base.color.tint, ..e.base.color },
    };
    if let Some(slot) = kf.elements.get_mut(idx) {
        *slot = Arc::new(base);
    }
}

/// Most bends [`move_on_path`] makes to land the object where it was put; each takes 15–35%
/// off the miss on typical paths (the path's length changes as it bends, sliding the object).
const BEND_STEPS: usize = 48;

/// The object on a motion path was moved by `d` at `offset`: at or before the first position
/// key the path's start moves, at or after the last its end, in between the path bends so it
/// passes where the object was put (a few bends, as the path's length changes with each);
/// without position keys the whole path moves.
fn move_on_path(m: &mut MotionTween, base: &Props, offset: u32, d: Vec2, last: u32) {
    let Some(mut path) = m.path.clone() else { return };
    let f = offset as f32;
    match m.position_range() {
        Some((f0, f1)) if f1 > f0 && offset > f0 && offset < f1 => {
            let Some(target) = m.on_path(base).map(|op| op.point(f) + d) else { return };
            let mut trial = m.clone();
            for _ in 0..BEND_STEPS {
                let Some(op) = trial.on_path(base) else { return };
                let miss = target - op.point(f);
                if miss.length() < 2e-3 {
                    break;
                }
                let (cmd, t) = op.walk().locate(op.distance(f));
                if !path.bend_segment(cmd, t, miss) {
                    break;
                }
                trial.path = Some(path.clone());
            }
        }
        Some((f0, f1)) if f1 > f0 => {
            let anchor = if offset <= f0 { 0 } else { path.cmds.len().saturating_sub(1) };
            path.move_anchor(anchor, d);
        }
        _ => path = path.transformed(&Affine::translate(d.x, d.y)),
    }
    m.set_path(&path, base, last);
}

/// The motion tween's object values (its base) on `kf`.
fn base_of(kf: &Keyframe) -> Result<Props> {
    kf.motion.as_ref().and_then(|m| m.target_index(&kf.elements)).and_then(|i| kf.elements.get(i)).map(|e| Props::of(e)).ok_or_else(|| EngineError::State("the motion tween's object is no longer on its keyframe".into()))
}

fn set_path(s: &mut Session, p: &Value) -> Result<Value> {
    let d = str_req(p, "d")?;
    if d.len() > MAX_PATH_DATA {
        return Err(EngineError::Param("param 'd' is too long".into()));
    }
    let path = anicraft_geom::svg::parse_path_data(d).map_err(|e| EngineError::Param(format!("param 'd': {e}")))?;
    let (kf, _) = motion_span(s, p)?;
    let base = base_of(kf)?;
    let last = kf.duration.saturating_sub(1);
    let m = kf.motion.as_mut().ok_or_else(|| EngineError::State("no motion tween here".into()))?;
    if last == 0 && m.position_range().is_none_or(|(a, b)| a == b) {
        return Err(EngineError::State("a one-frame span has no room to move along a path: make the span longer first".into()));
    }
    if !m.set_path(&path, &base, last) {
        return Err(EngineError::Param("param 'd' needs a segment of some length, with finite points within ±1000000".into()));
    }
    Ok(path_summary(m))
}

fn path_summary(m: &MotionTween) -> Value {
    let Some(path) = &m.path else { return json!({"path": null, "orient": m.orient}) };
    let len = anicraft_geom::walk::CurveWalk::new(path).map_or(0.0, |w| w.length());
    let (a, b) = anicraft_doc::motion::path_ends(path).unwrap_or_default();
    // Anchor `i` is path command `i` (a motion path is a move and cubics).
    let anchors: Vec<[f32; 2]> = path.anchors().into_iter().map(|(_, q)| [q.x, q.y]).collect();
    json!({"path": path.to_svg(), "segments": path.cmds.len().saturating_sub(1), "length": len, "start": [a.x, a.y], "end": [b.x, b.y], "anchors": anchors, "orient": m.orient})
}

/// Edits the motion path at the target frame with `edit`: the tween's own path, or the straight
/// one its X/Y curves trace (which the edit makes its own, as on the stage). The ends' keys stay
/// unless the edit moved an end, so the timing is kept. Returns what `edit` returned and the
/// path's summary.
fn edit_path<T>(s: &mut Session, p: &Value, edit: impl FnOnce(&mut anicraft_geom::Path) -> Result<T>) -> Result<(T, Value)> {
    let (kf, _) = motion_span(s, p)?;
    let base = base_of(kf)?;
    let last = kf.duration.saturating_sub(1);
    let m = kf.motion.as_mut().ok_or_else(|| EngineError::State("no motion tween here".into()))?;
    let mut path = m.path.clone().or_else(|| m.path_from_tracks(&base)).ok_or_else(|| EngineError::State("the object doesn't move yet, so it has no path: move it at a later frame of the tween first".into()))?;
    let out = edit(&mut path)?;
    if !m.set_path(&path, &base, last) {
        return Err(EngineError::State("that would leave the motion path with no length".into()));
    }
    Ok((out, path_summary(m)))
}

/// An anchor index param, checked against the path.
fn anchor_index(p: &Value, key: &str, path: &anicraft_geom::Path) -> Result<usize> {
    let i = opt_u64(p, key)?.ok_or_else(|| EngineError::Param(format!("param '{key}' is required")))?;
    let n = path.cmds.len();
    usize::try_from(i).ok().filter(|i| *i < n).ok_or_else(|| EngineError::Param(format!("no anchor {i} on this motion path (it has {n}, from 0 at the start)")))
}

/// Where `motion.add_point` adds: near a stage point, or at a curve parameter of a segment.
enum PointAt {
    Near(Vec2),
    On(usize, f32),
}

fn add_point(s: &mut Session, p: &Value) -> Result<Value> {
    let at = match (opt_f32(p, "x")?, opt_f32(p, "y")?, opt_u64(p, "segment")?) {
        (Some(x), Some(y), None) => PointAt::Near(Vec2::new(x, y)),
        (None, None, Some(seg)) => PointAt::On(usize::try_from(seg).unwrap_or(usize::MAX), opt_f32(p, "t")?.ok_or_else(|| EngineError::Param("param 't' is required with 'segment' (0..1 along it)".into()))?),
        _ => return Err(EngineError::Param("give either x and y (a point on the path) or segment and t".into())),
    };
    let slop = opt_f32(p, "slop")?.unwrap_or(4.0);
    if !(0.0..=1000.0).contains(&slop) {
        return Err(EngineError::Param("param 'slop' must be between 0 and 1000".into()));
    }
    let (anchor, mut v) = edit_path(s, p, |path| {
        if path.cmds.len() > anicraft_doc::motion::MAX_PATH_SEGMENTS {
            return Err(EngineError::State(format!("a motion path holds at most {} segments", anicraft_doc::motion::MAX_PATH_SEGMENTS)));
        }
        let (cmd, t) = match at {
            PointAt::Near(q) => {
                let (cmd, t, dist) = path.nearest_segment(q, &Affine::IDENTITY).ok_or_else(|| EngineError::State("the motion path has no segments".into()))?;
                if dist > slop {
                    return Err(EngineError::State(format!("({}, {}) is not on the motion path ({dist:.1} px away)", q.x, q.y)));
                }
                (cmd, t)
            }
            PointAt::On(seg, t) => {
                if !(0.0..=1.0).contains(&t) {
                    return Err(EngineError::Param("param 't' must be between 0 and 1".into()));
                }
                if seg == 0 || seg >= path.cmds.len() {
                    return Err(EngineError::Param(format!("no segment {seg} on this motion path (1 to {})", path.cmds.len().saturating_sub(1))));
                }
                (seg, t)
            }
        };
        // At an existing anchor there is nothing to add.
        if !(1e-3..=1.0 - 1e-3).contains(&t) {
            return Err(EngineError::State("there is already an anchor there".into()));
        }
        if !path.split_segment(cmd, t) {
            return Err(EngineError::State("cannot add a point there".into()));
        }
        // The new anchor ends the first half.
        Ok(cmd)
    })?;
    v["anchor"] = json!(anchor);
    Ok(v)
}

fn delete_points(s: &mut Session, p: &Value) -> Result<Value> {
    let wanted = opt_u64_list(p, "anchors")?.ok_or_else(|| EngineError::Param("param 'anchors' is required (a list of anchor indices)".into()))?;
    if wanted.is_empty() {
        return Err(EngineError::Param("param 'anchors' lists no anchor".into()));
    }
    let ((deleted, kept_ends), mut v) = edit_path(s, p, |path| {
        let n = path.cmds.len();
        let mut set = std::collections::BTreeSet::new();
        for &i in wanted.iter().take(100_000) {
            match usize::try_from(i).ok().filter(|i| *i < n) {
                Some(i) => set.insert(i),
                None => return Err(EngineError::Param(format!("no anchor {i} on this motion path (it has {n}, from 0 at the start)"))),
            };
        }
        let ends = set.remove(&0) | set.remove(&n.saturating_sub(1));
        if set.is_empty() {
            return Err(EngineError::State("the ends of a motion path can't be deleted: they are where the motion starts and ends. Drag them to move them, or Straighten the path".into()));
        }
        // Last first, so the indices still to go stay where they were.
        let mut deleted = 0;
        for i in set.into_iter().rev() {
            if path.merge_at(i) {
                deleted += 1;
            }
        }
        Ok((deleted, ends))
    })?;
    v["deleted"] = json!(deleted);
    v["kept_ends"] = json!(kept_ends);
    Ok(v)
}

fn convert_point(s: &mut Session, p: &Value) -> Result<Value> {
    let smooth = opt_bool(p, "smooth")?.ok_or_else(|| EngineError::Param("param 'smooth' is required (true: smooth, false: corner)".into()))?;
    let (changed, mut v) = edit_path(s, p, |path| {
        let a = anchor_index(p, "anchor", path)?;
        Ok(if smooth { path.make_smooth(a) } else { path.make_corner(a) })
    })?;
    v["changed"] = json!(changed);
    Ok(v)
}

fn move_path(s: &mut Session, p: &Value) -> Result<Value> {
    let d = Vec2::new(f32_req(p, "dx")?, f32_req(p, "dy")?);
    if !d.is_finite() {
        return Err(EngineError::Param("params 'dx' and 'dy' must be finite".into()));
    }
    let (kf, _) = motion_span(s, p)?;
    let i = kf.motion.as_ref().and_then(|m| m.target_index(&kf.elements)).ok_or_else(|| EngineError::State("the motion tween's object is no longer on its keyframe".into()))?;
    let m = kf.motion.as_mut().ok_or_else(|| EngineError::State("no motion tween here".into()))?;
    if !m.translate(d) {
        return Err(EngineError::Param("that would move the motion path beyond ±1000000 px".into()));
    }
    // The object's own place moves too (an axis without keys is where it is).
    if let Some(slot) = kf.elements.get_mut(i) {
        let mut el = (**slot).clone();
        el.transform = Affine::translate(d.x, d.y).mul(&el.transform);
        *slot = Arc::new(el);
    }
    let mut v = kf.motion.as_ref().map_or(Value::Null, path_summary);
    v["moved"] = json!([d.x, d.y]);
    Ok(v)
}

fn clear_path(s: &mut Session, p: &Value) -> Result<Value> {
    let (kf, offset) = motion_span(s, p)?;
    let frame = kf.start + offset;
    let m = kf.motion.as_mut().ok_or_else(|| EngineError::State("no motion tween here".into()))?;
    if m.path.take().is_none() {
        return Err(EngineError::State(format!("the motion tween at frame {} has no motion path to remove", frame + 1)));
    }
    // Orient to path has nothing left to turn with.
    m.orient = false;
    Ok(json!({"cleared": true}))
}

fn orient(s: &mut Session, p: &Value) -> Result<Value> {
    let on = opt_bool(p, "orient")?.ok_or_else(|| EngineError::Param("param 'orient' is required (true or false)".into()))?;
    let (kf, _) = motion_span(s, p)?;
    let base = base_of(kf)?;
    let last = kf.duration.saturating_sub(1);
    let m = kf.motion.as_mut().ok_or_else(|| EngineError::State("no motion tween here".into()))?;
    // The straight path the X/Y curves trace becomes the path to turn along.
    let mut created = false;
    if on && m.path.is_none() {
        let path = m.path_from_tracks(&base).ok_or_else(|| EngineError::State("the object doesn't move yet: move it at a later frame of the tween first, then it can turn with its path".into()))?;
        created = m.set_path(&path, &base, last);
    }
    m.orient = on;
    let mut v = path_summary(m);
    v["path_created"] = json!(created);
    Ok(v)
}

/// Insert Keyframe (F6) inside a motion tween: every property gets a key at `frame` holding its
/// value there; the timeline is not split. Whether `frame` was in a motion tween.
pub(crate) fn key_all_at(layer: &mut Layer, frame: u32) -> bool {
    let Some(i) = layer.keyframe_index_at(frame) else { return false };
    let Some(kf) = layer.keyframe_mut(i) else { return false };
    if frame <= kf.start {
        return kf.motion.is_some();
    }
    let offset = frame - kf.start;
    let Some(m) = kf.motion.as_mut() else { return false };
    let Some(base) = m.target_index(&kf.elements).and_then(|t| kf.elements.get(t)).map(|e| Props::of(e)) else { return true };
    let now = m.props_at(&base, offset as f32);
    for p in Property::ALL {
        m.set_key(p, offset, now.get(p), base.get(p));
    }
    true
}

/// The base as it looks on the span's first frame: what a span keeps when its motion tween is
/// removed or replaced by another kind of tween.
pub(crate) fn bake_first_frame(kf: &mut Keyframe) {
    let Some(m) = kf.motion.take() else { return };
    if let Some(i) = m.target_index(&kf.elements)
        && let Some(slot) = kf.elements.get_mut(i)
    {
        let first = m.apply(slot, 0.0);
        if first != **slot {
            *slot = Arc::new(first);
        }
    }
}

/// Create Motion Tween: the span at the frame tweens its object property by property. Several
/// objects become one graphic symbol first (as Animate offers), a one-frame span grows to a
/// second, and a classic or shape tween on the span is replaced.
pub(crate) fn create(s: &mut Session, p: &Value) -> Result<Value> {
    let (lid, frame) = target(s, p)?;
    let lid = lid.unwrap_or(s.layer);
    let (start, ids) = {
        let tl = s.doc.timeline(s.scene);
        let layer = tl.layer(lid).ok_or_else(|| EngineError::State(format!("no layer with id {}", lid.0)))?;
        if layer.kind == LayerKind::Folder {
            return Err(EngineError::State("a folder holds layers, not frames; pick a layer inside it".into()));
        }
        let k = layer.keyframe_at(frame).ok_or_else(|| EngineError::State(format!("frame {} is empty on this layer", frame + 1)))?;
        if let Some(m) = &k.motion {
            return Ok(json!({"motion": k.start, "target": m.target.0, "frames": k.duration, "converted": false}));
        }
        if k.elements.is_empty() && layer.kind != LayerKind::Camera {
            return Err(EngineError::State("there is nothing on this keyframe to tween: draw or place something first".into()));
        }
        (k.start, k.elements.iter().map(|e| e.id).collect::<Vec<_>>())
    };
    // A camera keyframe without a camera holds the camera at rest: that is what moves.
    if ids.is_empty() {
        let el = anicraft_doc::Camera::rest(&s.doc.stage).to_element(ElementId(s.doc.alloc_id()));
        let layer = s.layer_mut(Some(lid))?;
        if let Some(k) = layer.keyframe_starting_at(start).and_then(|i| layer.keyframe_mut(i)) {
            k.elements.push(Arc::new(el));
        }
    }
    let converted = ids.len() > 1;
    if converted {
        // A motion tween moves one object: the key's artwork becomes one instance.
        let (layer, frame, sel) = (s.layer, s.frame, s.selection.clone());
        let name = format!("Tween {}", s.doc.library.len() + 1);
        s.layer = lid;
        s.frame = start;
        s.selection = ids;
        let r = crate::cmds_draw::convert_to_symbol(s, &json!({"name": name}));
        s.frame = frame;
        if layer != lid {
            s.layer = layer;
            s.selection = sel;
        }
        r?;
    }
    let seconds = (s.doc.stage.fps.round() as u32).clamp(2, 240);
    let layer = s.layer_mut(Some(lid))?;
    let i = layer.keyframe_starting_at(start).ok_or_else(|| EngineError::State("the keyframe went away".into()))?;
    let k = layer.keyframe_mut(i).ok_or_else(|| EngineError::State("the keyframe went away".into()))?;
    let target = k.elements.first().map(|e| e.id).ok_or_else(|| EngineError::State("there is nothing on this keyframe to tween".into()))?;
    k.tween = None;
    k.hints.clear();
    k.motion = Some(MotionTween::new(target));
    // A single frame has no room to move in: it becomes a one-second span.
    if k.duration == 1 {
        for _ in 1..seconds {
            if crate::cmds_timeline::insert_frame_at(layer, start).is_err() {
                break;
            }
        }
    }
    let frames = layer.keyframe_starting_at(start).and_then(|i| layer.keyframes.get(i)).map_or(1, |k| k.duration);
    Ok(json!({"motion": start, "target": target.0, "frames": frames, "converted": converted}))
}

/// The motion tween covering the target frame, its keyframe and the frame's offset into it.
fn motion_span<'a>(s: &'a mut Session, p: &Value) -> Result<(&'a mut Keyframe, u32)> {
    let (lid, frame) = target(s, p)?;
    let layer = s.layer_mut(lid)?;
    let i = layer.keyframe_index_at(frame).ok_or_else(|| EngineError::State(format!("frame {} is empty on this layer", frame + 1)))?;
    let kf = layer.keyframe_mut(i).ok_or_else(|| EngineError::State("keyframe vanished".into()))?;
    if kf.motion.is_none() {
        return Err(EngineError::State(format!("frame {} is not in a motion tween; select the span and use Insert › Create Motion Tween", frame + 1)));
    }
    let offset = frame - kf.start;
    Ok((kf, offset))
}

fn property(p: &Value) -> Result<Option<Property>> {
    match opt_str(p, "property")? {
        None => Ok(None),
        Some(k) => Property::from_key(k).map(Some).ok_or_else(|| {
            let all: Vec<&str> = Property::ALL.iter().map(|p| p.key()).collect();
            EngineError::Param(format!("unknown property '{k}' ({})", all.join(", ")))
        }),
    }
}

fn set_key(s: &mut Session, p: &Value) -> Result<Value> {
    let prop = property(p)?.ok_or_else(|| EngineError::Param("param 'property' is required".into()))?;
    let value = f32_req(p, "value")?;
    let r = prop.range();
    if !r.contains(&value) {
        return Err(EngineError::Param(format!("{} must be between {} and {} {}", prop.label(), r.start(), r.end(), prop.unit())));
    }
    let (kf, offset) = motion_span(s, p)?;
    let base = kf.motion.as_ref().and_then(|m| m.target_index(&kf.elements)).and_then(|i| kf.elements.get(i)).map(|e| Props::of(e));
    let base = base.ok_or_else(|| EngineError::State("the motion tween's object is no longer on its keyframe".into()))?;
    let start = kf.start;
    let m = kf.motion.as_mut().ok_or_else(|| EngineError::State("no motion tween here".into()))?;
    m.set_key(prop, offset, value, base.get(prop));
    Ok(json!({"property": prop.key(), "frame": start + offset, "value": value}))
}

fn remove_key(s: &mut Session, p: &Value) -> Result<Value> {
    let only = property(p)?;
    let (kf, offset) = motion_span(s, p)?;
    let start = kf.start;
    let m = kf.motion.as_mut().ok_or_else(|| EngineError::State("no motion tween here".into()))?;
    let removed: Vec<&str> = Property::ALL.into_iter().filter(|x| only.is_none_or(|o| o == *x)).filter(|x| m.remove_key(*x, offset)).map(Property::key).collect();
    if removed.is_empty() {
        return Err(EngineError::State(format!("there is no property keyframe at frame {}", start + offset + 1)));
    }
    Ok(json!({"removed": removed, "frame": start + offset}))
}

fn set_ease(s: &mut Session, p: &Value) -> Result<Value> {
    let easing = opt_easing(p, "easing")?.ok_or_else(|| EngineError::Param("param 'easing' is required".into()))?;
    let only = property(p)?;
    let (kf, offset) = motion_span(s, p)?;
    let start = kf.start;
    let m = kf.motion.as_mut().ok_or_else(|| EngineError::State("no motion tween here".into()))?;
    let mut set = Vec::new();
    for t in m.tracks.iter_mut().filter(|t| only.is_none_or(|o| o == t.property)) {
        if let Some(k) = t.segment_at(offset).and_then(|i| t.keys.get_mut(i)) {
            k.ease = easing;
            set.push(t.property.key());
        }
    }
    if set.is_empty() {
        return Err(EngineError::State(format!("frame {} is not between two property keyframes{}", start + offset + 1, only.map_or_else(String::new, |o| format!(" of {}", o.label())))));
    }
    Ok(json!({"easing": easing.label(), "properties": set}))
}

fn info(s: &mut Session, p: &Value) -> Result<Value> {
    let (lid, frame) = target(s, p)?;
    let lid = lid.unwrap_or(s.layer);
    let layer = s.doc.timeline(s.scene).layer(lid).ok_or_else(|| EngineError::State(format!("no layer with id {}", lid.0)))?;
    let kf = layer.keyframe_at(frame).ok_or_else(|| EngineError::State(format!("frame {} is empty on this layer", frame + 1)))?;
    let m = kf.motion.as_ref().ok_or_else(|| EngineError::State(format!("frame {} is not in a motion tween", frame + 1)))?;
    let base = m.target_index(&kf.elements).and_then(|i| kf.elements.get(i)).map(|e| Props::of(e));
    let now = base.map(|b| m.props_at(&b, (frame - kf.start) as f32));
    let tracks: Vec<Value> = m
        .tracks
        .iter()
        .map(|t| {
            let keys: Vec<Value> = t.keys.iter().map(|k| json!({"frame": kf.start + k.frame, "value": k.value, "ease": k.ease.label(), "bezier": k.ease.as_bezier()})).collect();
            json!({"property": t.property.key(), "label": t.property.label(), "unit": t.property.unit(), "keys": keys})
        })
        .collect();
    let values: serde_json::Map<String, Value> = Property::ALL.iter().filter_map(|p| now.map(|n| (p.key().to_string(), json!(n.get(*p))))).collect();
    // Where the object is (on a motion path the X/Y values are its timing, not its place).
    let position = m.target_index(&kf.elements).and_then(|i| kf.elements.get(i)).map(|e| m.apply(e, (frame - kf.start) as f32).transform.translation()).map(|q| json!([q.x, q.y]));
    let path = path_summary(m);
    // The anchors point edits address: the path's own, or the straight path's (ADR 0038).
    let anchors: Vec<[f32; 2]> = m.path.clone().or_else(|| base.and_then(|b| m.path_from_tracks(&b))).map(|p| p.anchors().into_iter().map(|(_, q)| [q.x, q.y]).collect()).unwrap_or_default();
    Ok(json!({"layer": lid.0, "start": kf.start, "frames": kf.duration, "target": m.target.0, "tracks": tracks, "values": values, "position": position, "path": path["path"], "anchors": anchors, "orient": m.orient}))
}
