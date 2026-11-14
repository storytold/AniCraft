//! Motion tweens (ADR 0016): creating one on a span, property keyframes, per-segment eases,
//! and auto-keying edits made to the tweened object.
//!
//! Auto-key works for every command without the commands knowing: when a command asks for the
//! keyframe to edit ([`Session::editable_keyframe`]) and the playhead is inside a motion tween,
//! [`begin_edit`] swaps the target for how it looks at the playhead, and after the command
//! [`finish_edit`] turns whatever changed in its transform or colour effect into property keys
//! and puts the base object back (with any other change, a new fill say, kept).
//!
//! Filters (ADR 0092): a filter's numbers tween as properties too, addressed by the filter's place
//! (`filter.0.blur_x`). An edit of a filter's number at the playhead keys it there; an edit that
//! reorders, removes or adds filters takes the tracks along with their filters.
//!
//! On a motion path, moving the object at a frame pins the path there (ADR 0049): the anchor
//! pinned to that frame moves (an end, at the first or last position key), or a new anchor is
//! added where the object is, pinned to the frame, and moved; after the last position key the
//! path carries on to where the object was put.

use crate::cmds_timeline::target;
use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::{Element, ElementId, ElementKind, FilterEntry, Keyframe, Layer, LayerId, LayerKind, MotionTween, Pin, PinError, Property, Props, filter_properties, own_value};
use anicraft_geom::{Affine, Vec2};
use serde_json::{Value, json};
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c("motion.set_key", "Property Keyframe", "", None, "{property: x|y|scale_x|scale_y|rotation|skew|alpha|brightness|tint (amount)|tint_red|tint_green|tint_blue (0..255)|red_multiplier|green_multiplier|blue_multiplier|alpha_multiplier (%)|red_offset|green_offset|blue_offset|alpha_offset (-255..255)|filter.<slot>.<blur_x|blur_y|strength (%)|angle|distance|brightness|contrast|saturation|hue> (a number of the object's filter at that place, 0 the first; ADR 0092), value (px, %, °), frame?, layer?} (colour properties of several styles compose into Advanced, ADR 0056)", true, set_key as crate::RunFn),
        c("motion.remove_key", "Remove Property Keyframe", "", None, "{property? (default: every property), frame?, layer?}", true, remove_key),
        c("motion.set_ease", "Property Ease", "", None, "{easing (as timeline.set_easing takes it, saved eases too), property? (default: every property), frame? (inside the segment), layer?}", true, set_ease),
        c("motion.info", "Motion Tween", "", None, "{layer?, frame?} → the span's properties, keys, eases and values at the frame, its motion path, its anchors pinned to frames (pins) and where the object is", false, info),
        c("motion.set_path", "Edit Motion Path", "", None, "{d: SVG path data in stage space (its first subpath; ends are the start and end positions), layer?, frame?}", true, set_path),
        c("motion.clear_path", "Remove Motion Path", "", None, "{layer?, frame?}: back to the straight path the X/Y curves trace", true, clear_path),
        c("motion.orient", "Orient to Path", "", None, "{orient: bool, layer?, frame?}: turn the object as its motion path turns", true, orient),
        c("motion.add_point", "Add Motion Path Point", "", None, "{x, y, slop? (stage units, default 4) | segment, t (0..1), layer?, frame?}: an anchor on the motion path, its shape unchanged → {anchor}", true, add_point),
        c("motion.delete_points", "Delete Motion Path Points", "", None, "{anchors: [index, ..] (as motion.info lists them; the ends stay), layer?, frame?}: the path refits through the rest", true, delete_points),
        c("motion.convert_point", "Convert Motion Path Point", "", None, "{anchor, smooth: bool, layer?, frame?}: smooth (handles in line) or corner (handles on the anchor)", true, convert_point),
        c("motion.move_path", "Move Motion Path", "", None, "{dx, dy, layer?, frame?}: the whole path with its object, keeping the timing", true, move_path),
        c("motion.pin_point", "Pin Motion Path Point", "", None, "{anchor? (default: a new anchor where the object is), frame? (the frame to pin it to, between the first and last position keys), layer?}: the object passes the anchor exactly on that frame → {anchor, frame, pins}", true, pin_point),
        c("motion.unpin_point", "Unpin Motion Path Point", "", None, "{anchor? | frame?, layer?}: the anchor shapes the path only again; the X/Y keys of its frame go → {anchor, frame, pins}", true, unpin_point),
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
    let Some(layer) = crate::editing::edited_mut(&mut s.doc, scene, &s.editing).ok().and_then(|tl| tl.layer_mut(e.layer)) else { return };
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
            if p.style().is_some() {
                base_color = true;
            } else {
                base_xf = true;
            }
        } else {
            m.set_key(p, e.offset, v, base_props.get(p));
        }
    }
    let tracks_xf = m.tracks.iter().any(|t| t.property.is_transform());
    let tracks_color = m.tracks.iter().any(|t| t.property.style().is_some());
    let filters = key_filters(m, &e, &after);
    let mut base = after;
    if let (Some(f), ElementKind::Symbol { filters: slot, .. }) = (filters, &mut base.kind) {
        *slot = f;
    }
    // Exact where nothing is keyed on the first frame: the edited object is the new base.
    base.transform = match (base_xf, e.offset == 0 && !tracks_xf) {
        (true, true) => base.transform,
        (true, false) => new_base.transform(),
        (false, _) => e.base.transform,
    };
    base.color = match (base_color, e.offset == 0 && !tracks_color) {
        (true, true) => base.color,
        // The edit's style, from the new values (ADR 0056).
        (true, false) => new_base.color_effect(&[base.color.style()]),
        (false, _) => e.base.color,
    };
    if let Some(slot) = kf.elements.get_mut(idx) {
        *slot = Arc::new(base);
    }
}

/// The filters of an edited target (ADR 0092). The command saw them as they look at the
/// playhead (`e.before`) and left them as `after` has them. When the list kept its shape (the
/// same kinds in the same order), each number it changed becomes a key at the playhead (on the
/// span's first frame, an untracked number changes the object's own), and the tracked numbers
/// keep the object's own values underneath. When the shape changed (filters reordered, removed
/// or added), each filter is found again by what it holds, its tracks go where it went, and
/// those of filters that went go too. The base's new list, or `None` when there are no filters
/// to look after.
fn key_filters(m: &mut MotionTween, e: &MotionEdit, after: &Element) -> Option<Vec<FilterEntry>> {
    let list = |el: &Element| match &el.kind {
        ElementKind::Symbol { filters, .. } => Some(filters.clone()),
        _ => None,
    };
    let (before, base, mut now) = (list(&e.before)?, list(&e.base)?, list(after)?);
    if !anicraft_doc::filters_pair_up(&before, &now) {
        // Where each filter went: the first one left that holds the same.
        let mut taken = vec![false; now.len()];
        let map: Vec<Option<usize>> = before
            .iter()
            .map(|b| {
                let i = now.iter().enumerate().position(|(i, n)| !taken.get(i).copied().unwrap_or(true) && n.filter == b.filter)?;
                if let Some(t) = taken.get_mut(i) {
                    *t = true;
                }
                Some(i)
            })
            .collect();
        m.remap_filters(&map);
        // A filter that moved is the base's underneath (with its switch as the edit left it),
        // not the playhead's.
        for (old, new) in map.iter().enumerate() {
            if let (Some(n), Some(b)) = (new, base.get(old))
                && let Some(slot) = now.get_mut(*n)
            {
                let enabled = slot.enabled;
                *slot = FilterEntry { enabled, ..b.clone() };
            }
        }
        return Some(now);
    }
    for (slot, (was, is)) in before.iter().zip(&now).enumerate().take(usize::from(anicraft_doc::MAX_FILTER_SLOTS)) {
        for &param in is.filter.params() {
            let p = Property::Filter(anicraft_doc::FilterProp { slot: slot as u8, param });
            let (Some(v0), Some(v1)) = (was.filter.param(param), is.filter.param(param)) else { continue };
            if (v1 - v0).abs() > 1e-4 && !(e.offset == 0 && m.track(p).is_none()) {
                let own = base.get(slot).and_then(|b| b.filter.param(param)).unwrap_or(v0);
                m.set_key(p, e.offset, v1, own);
            }
        }
    }
    // Tracked numbers: the object's own underneath (the curves give the rest).
    for t in &m.tracks {
        if let Some(f) = t.property.filter()
            && let (Some(slot), Some(own)) = (now.get_mut(usize::from(f.slot)), base.get(usize::from(f.slot)).and_then(|b| b.filter.param(f.param)))
        {
            slot.filter.set_param(f.param, own);
        }
    }
    Some(now)
}

/// Most bends [`move_on_path`] makes to land the object where it was put off the path; each
/// takes 15–35% off the miss on typical paths (the path's length changes as it bends, sliding the
/// object).
const BEND_STEPS: usize = 48;

/// The object on a motion path was moved by `d` at `offset` (ADR 0049): the anchor pinned to
/// that frame moves with it (before the first position key, the start); after the last position
/// key the path carries on to where it was put; in between, a new anchor where the object is gets
/// pinned to the frame and moved. Where the object is off the path (an overshooting ease) the
/// path bends instead (ADR 0030); without position keys the whole path moves.
fn move_on_path(m: &mut MotionTween, base: &Props, offset: u32, d: Vec2, last: u32) {
    let Some(mut path) = m.path.clone() else { return };
    match m.position_range() {
        Some((f0, f1)) if f1 > f0 => {
            let pinned = m.pinned_anchors(base).into_iter().find(|p| p.frame == offset).map(|p| p.anchor as usize);
            if let Some(a) = pinned.or((offset < f0).then_some(0)) {
                m.move_anchor(a, d, base, last);
                return;
            }
            if offset > f1 {
                let end = anicraft_doc::motion::path_ends(&path).map(|(_, b)| b).unwrap_or_default();
                if !m.extend_to(offset, end + d, base) {
                    m.move_anchor(path.cmds.len().saturating_sub(1), d, base, last);
                }
                return;
            }
            if let Ok(a) = m.pin_here(offset, base) {
                m.move_anchor(a, d, base, last);
                return;
            }
            let f = offset as f32;
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
    let (start, last) = (kf.start, kf.duration.saturating_sub(1));
    let m = kf.motion.as_mut().ok_or_else(|| EngineError::State("no motion tween here".into()))?;
    if last == 0 && m.position_range().is_none_or(|(a, b)| a == b) {
        return Err(EngineError::State("a one-frame span has no room to move along a path: make the span longer first".into()));
    }
    if !m.set_path(&path, &base, last) {
        return Err(EngineError::Param("param 'd' needs a segment of some length, with finite points within ±1000000".into()));
    }
    Ok(path_summary(m, start, &base))
}

fn path_summary(m: &MotionTween, start: u32, base: &Props) -> Value {
    let pins = pins_json(m, start, base);
    let Some(path) = &m.path else { return json!({"path": null, "orient": m.orient, "pins": pins}) };
    let len = anicraft_geom::walk::CurveWalk::new(path).map_or(0.0, |w| w.length());
    let (a, b) = anicraft_doc::motion::path_ends(path).unwrap_or_default();
    // Anchor `i` is path command `i` (a motion path is a move and cubics).
    let anchors: Vec<[f32; 2]> = path.anchors().into_iter().map(|(_, q)| [q.x, q.y]).collect();
    json!({"path": path.to_svg(), "segments": path.cmds.len().saturating_sub(1), "length": len, "start": [a.x, a.y], "end": [b.x, b.y], "anchors": anchors, "orient": m.orient, "pins": pins})
}

/// The anchors tied to frames (ADR 0049), ends included: `{anchor, frame, end, at}`, frames
/// counted from the timeline's start.
fn pins_json(m: &MotionTween, start: u32, base: &Props) -> Value {
    let path = m.path.clone().or_else(|| m.path_from_tracks(base));
    let pins = m.pinned_anchors(base);
    let n = pins.len();
    let v: Vec<Value> = pins
        .iter()
        .enumerate()
        .map(|(i, q)| {
            let at = path.as_ref().and_then(|p| p.point(q.anchor as usize, anicraft_geom::edit::ANCHOR)).map(|v| json!([v.x, v.y]));
            json!({"anchor": q.anchor, "frame": start + q.frame, "end": i == 0 || i + 1 == n, "at": at})
        })
        .collect();
    Value::Array(v)
}

/// Edits the motion path at the target frame with `edit`: the tween's own path, or the straight
/// one its X/Y curves trace (which the edit makes its own, as on the stage, its position keys
/// pinned to their anchors). The ends' and pins' keys stay unless the edit moved their anchors,
/// so the timing is kept. Returns what `edit` returned and the path's summary.
fn edit_path<T>(s: &mut Session, p: &Value, edit: impl FnOnce(&mut MotionTween) -> Result<T>) -> Result<(T, Value)> {
    let (kf, _) = motion_span(s, p)?;
    let base = base_of(kf)?;
    let (start, last) = (kf.start, kf.duration.saturating_sub(1));
    let m = kf.motion.as_mut().ok_or_else(|| EngineError::State("no motion tween here".into()))?;
    if m.path.is_none() {
        let (path, pins) = m.path_and_pins_from_tracks(&base).ok_or_else(|| EngineError::State("the object doesn't move yet, so it has no path: move it at a later frame of the tween first".into()))?;
        m.path = Some(path);
        m.pins = pins;
    }
    let out = edit(m)?;
    let path = m.path.clone().ok_or_else(|| EngineError::State("the motion path went away".into()))?;
    if !m.set_path(&path, &base, last) {
        return Err(EngineError::State("that would leave the motion path with no length".into()));
    }
    Ok((out, path_summary(m, start, &base)))
}

/// The motion path being edited (there is one inside [`edit_path`]).
fn path_mut(m: &mut MotionTween) -> Result<&mut anicraft_geom::Path> {
    m.path.as_mut().ok_or_else(|| EngineError::State("no motion path here".into()))
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
    let (anchor, mut v) = edit_path(s, p, |m| {
        let path = path_mut(m)?;
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
        // Pins after it stay on their anchors.
        if !m.split_path(cmd, t) {
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
    let ((deleted, kept_ends, unpinned), mut v) = edit_path(s, p, |m| {
        let n = path_mut(m)?.cmds.len();
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
        // Last first, so the indices still to go stay where they were. A pinned anchor's pin
        // goes with it (and its X/Y keys).
        let (mut deleted, mut unpinned) = (0, 0);
        for i in set.into_iter().rev() {
            let pinned = m.live_pins().iter().any(|q| q.anchor as usize == i);
            if m.delete_anchor(i) {
                deleted += 1;
                unpinned += usize::from(pinned);
            }
        }
        Ok((deleted, ends, unpinned))
    })?;
    v["deleted"] = json!(deleted);
    v["kept_ends"] = json!(kept_ends);
    v["unpinned"] = json!(unpinned);
    Ok(v)
}

fn convert_point(s: &mut Session, p: &Value) -> Result<Value> {
    let smooth = opt_bool(p, "smooth")?.ok_or_else(|| EngineError::Param("param 'smooth' is required (true: smooth, false: corner)".into()))?;
    let (changed, mut v) = edit_path(s, p, |m| {
        let path = path_mut(m)?;
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
    let start = kf.start;
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
    let base = base_of(kf)?;
    let mut v = kf.motion.as_ref().map_or(Value::Null, |m| path_summary(m, start, &base));
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
    // Orient to path has nothing left to turn with, and pins nothing to pin. Their keys stay:
    // the straight path runs through them.
    m.orient = false;
    m.pins.clear();
    Ok(json!({"cleared": true}))
}

fn orient(s: &mut Session, p: &Value) -> Result<Value> {
    let on = opt_bool(p, "orient")?.ok_or_else(|| EngineError::Param("param 'orient' is required (true or false)".into()))?;
    let (kf, _) = motion_span(s, p)?;
    let base = base_of(kf)?;
    let (start, last) = (kf.start, kf.duration.saturating_sub(1));
    let m = kf.motion.as_mut().ok_or_else(|| EngineError::State("no motion tween here".into()))?;
    // The straight path the X/Y curves trace becomes the path to turn along.
    let mut created = false;
    if on && m.path.is_none() {
        let path = m.path_from_tracks(&base).ok_or_else(|| EngineError::State("the object doesn't move yet: move it at a later frame of the tween first, then it can turn with its path".into()))?;
        created = m.set_path(&path, &base, last);
    }
    m.orient = on;
    let mut v = path_summary(m, start, &base);
    v["path_created"] = json!(created);
    Ok(v)
}

/// Why a pin can't be made, in words an artist (or an agent) can act on. `start` is the span's
/// first frame (messages count frames from 1, as the timeline does).
fn pin_error(e: PinError, start: u32, frame: u32) -> EngineError {
    let shown = |f: u32| start + f + 1;
    match e {
        PinError::NoPath => EngineError::State("the object doesn't move yet, so it has no path: move it at a later frame of the tween first".into()),
        PinError::NoAnchor => EngineError::Param("no such anchor on this motion path (motion.info lists them, from 0 at the start)".into()),
        PinError::End => EngineError::State("the ends of a motion path are always pinned, to the first and last position keys: move the object on those frames to move them".into()),
        PinError::Outside(f0, f1) => EngineError::State(format!("frame {} is outside the motion (frames {} to {}): move the object there to carry the path on", shown(frame), shown(f0), shown(f1))),
        PinError::Order(q) => EngineError::State(format!("anchor {} is pinned to frame {}: pins keep the path's order, a later frame on a later anchor", q.anchor, shown(q.frame))),
        PinError::OffPath => EngineError::State(format!("at frame {} the object is beyond an end of the path (its ease overshoots): pin an anchor of the path instead", shown(frame))),
        PinError::Full => EngineError::State(format!("a motion path holds at most {} segments", anicraft_doc::motion::MAX_PATH_SEGMENTS)),
    }
}

/// A straight motion's path becomes the tween's own, its position keys pinned to its anchors
/// (nothing moves).
fn own_path(m: &mut MotionTween, base: &Props, last: u32) -> std::result::Result<(), PinError> {
    if m.path.is_none() {
        let (path, pins) = m.path_and_pins_from_tracks(base).ok_or(PinError::NoPath)?;
        m.path = Some(path.clone());
        m.pins = pins;
        m.set_path(&path, base, last);
    }
    Ok(())
}

fn pin_point(s: &mut Session, p: &Value) -> Result<Value> {
    let anchor = opt_u64(p, "anchor")?.map(|a| usize::try_from(a).unwrap_or(usize::MAX));
    let (kf, offset) = motion_span(s, p)?;
    let base = base_of(kf)?;
    let (start, last) = (kf.start, kf.duration.saturating_sub(1));
    let m = kf.motion.as_mut().ok_or_else(|| EngineError::State("no motion tween here".into()))?;
    own_path(m, &base, last).map_err(|e| pin_error(e, start, offset))?;
    let a = match anchor {
        Some(a) => m.pin(a, offset, &base).map(|_| a),
        None => m.pin_here(offset, &base),
    }
    .map_err(|e| pin_error(e, start, offset))?;
    let mut v = path_summary(m, start, &base);
    v["anchor"] = json!(a);
    v["frame"] = json!(start + offset);
    Ok(v)
}

fn unpin_point(s: &mut Session, p: &Value) -> Result<Value> {
    let anchor = opt_u64(p, "anchor")?;
    let (kf, offset) = motion_span(s, p)?;
    let base = base_of(kf)?;
    let (start, last) = (kf.start, kf.duration.saturating_sub(1));
    let m = kf.motion.as_mut().ok_or_else(|| EngineError::State("no motion tween here".into()))?;
    own_path(m, &base, last).map_err(|e| pin_error(e, start, offset))?;
    let all = m.pinned_anchors(&base);
    let found: Option<Pin> = match anchor {
        Some(a) => all.iter().find(|q| u64::from(q.anchor) == a).copied(),
        None => all.iter().find(|q| q.frame == offset).copied(),
    };
    let Some(pin) = found else {
        return Err(EngineError::State(match anchor {
            Some(a) => format!("anchor {a} isn't pinned to a frame"),
            None => format!("no anchor is pinned to frame {}", start + offset + 1),
        }));
    };
    if all.first() == Some(&pin) || all.last() == Some(&pin) {
        return Err(pin_error(PinError::End, start, pin.frame));
    }
    m.unpin(pin.frame);
    let mut v = path_summary(m, start, &base);
    v["anchor"] = json!(pin.anchor);
    v["frame"] = json!(start + pin.frame);
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
    let Some(target) = m.target_index(&kf.elements).and_then(|t| kf.elements.get(t)).cloned() else { return true };
    let base = Props::of(&target);
    let now = m.props_at(&base, offset as f32);
    // The filters' numbers too (ADR 0092), as they are on that frame.
    let at = m.apply(&target, offset as f32);
    for p in filter_properties(&target) {
        if let (Some(v), Some(own)) = (own_value(&at, p), own_value(&target, p)) {
            m.set_key(p, offset, v, own);
        }
    }
    // On a motion path the frame's place is an anchor pinned to it (ADR 0049), where the
    // object is: nothing moves on this frame.
    let pinned = m.path.is_some() && m.pin_here(offset, &base).is_ok();
    for p in Property::ALL {
        if pinned && matches!(p, Property::X | Property::Y) {
            continue;
        }
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
        let tl = s.timeline();
        let layer = tl.layer(lid).ok_or_else(|| EngineError::State(format!("no layer with id {}", lid.0)))?;
        if layer.kind == LayerKind::Folder {
            return Err(EngineError::State("a folder holds layers, not frames; pick a layer inside it".into()));
        }
        if layer.kind == LayerKind::Armature {
            return Err(EngineError::State("an armature layer tweens its poses: add a pose at another frame and Create Classic Tween between them".into()));
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
        s.selection = crate::Selection::only(lid, ids);
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
            let all: Vec<String> = Property::ALL.iter().map(|p| p.key().into_owned()).collect();
            EngineError::Param(format!("unknown property '{k}' ({}, or filter.<slot>.<blur_x|blur_y|strength|angle|distance|brightness|contrast|saturation|hue>, slot 0 to 15)", all.join(", ")))
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
    let target = kf.motion.as_ref().and_then(|m| m.target_index(&kf.elements)).and_then(|i| kf.elements.get(i)).cloned();
    let target = target.ok_or_else(|| EngineError::State("the motion tween's object is no longer on its keyframe".into()))?;
    let base = Props::of(&target);
    let own = own_value(&target, prop).ok_or_else(|| match prop.filter() {
        Some(f) => EngineError::State(match &target.kind {
            ElementKind::Symbol { filters, .. } => match filters.get(usize::from(f.slot)) {
                Some(e) => format!("filter {} of the tween's object is a {}, which has no {}", f.slot, e.filter.label(), f.param.label()),
                None => format!("the tween's object has no filter {} (it has {}; 0 is the first)", f.slot, filters.len()),
            },
            _ => "filters apply to movie clip and button instances: the tween's object has none".into(),
        }),
        None => EngineError::State("the motion tween's object has no such property".into()),
    })?;
    let (start, last) = (kf.start, kf.duration.saturating_sub(1));
    let m = kf.motion.as_mut().ok_or_else(|| EngineError::State("no motion tween here".into()))?;
    // On a path, the X/Y keys of a pinned frame are its anchor's place: setting one moves the
    // anchor (ADR 0049).
    let pinned = matches!(prop, Property::X | Property::Y).then(|| m.pinned_anchors(&base).into_iter().find(|q| q.frame == offset)).flatten().filter(|_| m.path.is_some());
    if let Some(pin) = pinned {
        let at = m.path.as_ref().and_then(|p| p.point(pin.anchor as usize, anicraft_geom::edit::ANCHOR)).unwrap_or_default();
        let d = if prop == Property::X { Vec2::new(value - at.x, 0.0) } else { Vec2::new(0.0, value - at.y) };
        if d != Vec2::ZERO && !m.move_anchor(pin.anchor as usize, d, &base, last) {
            return Err(EngineError::Param(format!("{} must keep the motion path within ±1000000 px", prop.label())));
        }
        return Ok(json!({"property": prop.key(), "frame": start + offset, "value": value, "anchor": pin.anchor}));
    }
    m.set_key(prop, offset, value, own);
    Ok(json!({"property": prop.key(), "frame": start + offset, "value": value}))
}

fn remove_key(s: &mut Session, p: &Value) -> Result<Value> {
    let only = property(p)?;
    let (kf, offset) = motion_span(s, p)?;
    let start = kf.start;
    let m = kf.motion.as_mut().ok_or_else(|| EngineError::State("no motion tween here".into()))?;
    // Removing a pinned frame's X or Y key lets the pin go, with both keys (ADR 0049).
    let pin = m.live_pins().into_iter().find(|q| q.frame == offset).filter(|_| only.is_none_or(|o| matches!(o, Property::X | Property::Y)));
    let keyed: Vec<Property> = m.tracks.iter().map(|t| t.property).collect();
    let mut removed: Vec<std::borrow::Cow<'static, str>> = keyed.into_iter().filter(|x| only.is_none_or(|o| o == *x)).filter(|x| m.remove_key(*x, offset)).map(Property::key).collect();
    if pin.is_some() {
        for k in [Property::X.key(), Property::Y.key()] {
            if !removed.contains(&k) {
                removed.push(k);
            }
        }
    }
    if removed.is_empty() {
        return Err(EngineError::State(format!("there is no property keyframe at frame {}", start + offset + 1)));
    }
    Ok(json!({"removed": removed, "frame": start + offset, "unpinned": pin.map(|q| q.anchor)}))
}

fn set_ease(s: &mut Session, p: &Value) -> Result<Value> {
    let easing = opt_easing_in(s, p, "easing")?.ok_or_else(|| EngineError::Param("param 'easing' is required".into()))?;
    let only = property(p)?;
    let label = easing.label();
    let (kf, offset) = motion_span(s, p)?;
    let start = kf.start;
    let m = kf.motion.as_mut().ok_or_else(|| EngineError::State("no motion tween here".into()))?;
    let mut set = Vec::new();
    for t in m.tracks.iter_mut().filter(|t| only.is_none_or(|o| o == t.property)) {
        if let Some(k) = t.segment_at(offset).and_then(|i| t.keys.get_mut(i)) {
            k.ease = easing.clone();
            set.push(t.property.key());
        }
    }
    if set.is_empty() {
        return Err(EngineError::State(format!("frame {} is not between two property keyframes{}", start + offset + 1, only.map_or_else(String::new, |o| format!(" of {}", o.label())))));
    }
    Ok(json!({"easing": label, "properties": set}))
}

fn info(s: &mut Session, p: &Value) -> Result<Value> {
    let (lid, frame) = target(s, p)?;
    let lid = lid.unwrap_or(s.layer);
    let layer = s.timeline().layer(lid).ok_or_else(|| EngineError::State(format!("no layer with id {}", lid.0)))?;
    let kf = layer.keyframe_at(frame).ok_or_else(|| EngineError::State(format!("frame {} is empty on this layer", frame + 1)))?;
    let m = kf.motion.as_ref().ok_or_else(|| EngineError::State(format!("frame {} is not in a motion tween", frame + 1)))?;
    let target = m.target_index(&kf.elements).and_then(|i| kf.elements.get(i));
    let base = target.map(|e| Props::of(e));
    let now = base.map(|b| m.props_at(&b, (frame - kf.start) as f32));
    let filter_of = |slot: u8| match target.map(|e| &e.kind) {
        Some(ElementKind::Symbol { filters, .. }) => filters.get(usize::from(slot)).map(|e| e.filter.label()),
        _ => None,
    };
    let tracks: Vec<Value> = m
        .tracks
        .iter()
        .map(|t| {
            let keys: Vec<Value> = t.keys.iter().map(|k| json!({"frame": kf.start + k.frame, "value": k.value, "ease": k.ease.label(), "easing": easing_json(&k.ease), "bezier": k.ease.as_bezier()})).collect();
            let mut v = json!({"property": t.property.key(), "label": t.property.label(), "unit": t.property.unit(), "keys": keys});
            // A filter's number: which filter (ADR 0092); its kind `null` when it isn't there now.
            if let Some(f) = t.property.filter() {
                v["filter"] = json!({"slot": f.slot, "kind": filter_of(f.slot)});
            }
            v
        })
        .collect();
    let mut values: serde_json::Map<String, Value> = Property::ALL.iter().filter_map(|p| now.map(|n| (p.key().into_owned(), json!(n.get(*p))))).collect();
    // And the filters' numbers at the frame.
    if let Some(e) = target {
        let at = m.apply(e, (frame - kf.start) as f32);
        for p in filter_properties(e) {
            if let Some(v) = own_value(&at, p) {
                values.insert(p.key().into_owned(), json!(v));
            }
        }
    }
    // Where the object is (on a motion path the X/Y values are its timing, not its place).
    let position = m.target_index(&kf.elements).and_then(|i| kf.elements.get(i)).map(|e| m.apply(e, (frame - kf.start) as f32).transform.translation()).map(|q| json!([q.x, q.y]));
    let path = m.path.as_ref().map(|p| p.to_svg());
    // The anchors point edits address: the path's own, or the straight path's (ADR 0038).
    let anchors: Vec<[f32; 2]> = m.path.clone().or_else(|| base.and_then(|b| m.path_from_tracks(&b))).map(|p| p.anchors().into_iter().map(|(_, q)| [q.x, q.y]).collect()).unwrap_or_default();
    // And those tied to frames, ends included (ADR 0049).
    let pins = base.map_or(Value::Array(Vec::new()), |b| pins_json(m, kf.start, &b));
    Ok(json!({"layer": lid.0, "start": kf.start, "frames": kf.duration, "target": m.target.0, "tracks": tracks, "values": values, "position": position, "path": path, "anchors": anchors, "pins": pins, "orient": m.orient}))
}
