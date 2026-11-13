//! The Selection tool on raw shapes (ADR 0041; issue #42 §2 and §4): bending an edge by dragging
//! it, moving a corner or an end together with every raw shape that shares it, snapping what is
//! dragged to points, lines and fill edges, and a marquee that selects the parts of fills and
//! strokes under it.
//!
//! Positions and moves are in stage space. A layer's merged drawing stores a shared edge or
//! point once in each shape that has it, so every edit here changes all the copies alike and
//! then merges the shapes again (ADR 0031), unless asked not to while a drag is still going.

use crate::cmds_draw::MAX_EXTENT;
use crate::merge::{self, Part, mergeable, reach};
use crate::params::*;
use crate::{CommandSpec, CutOrigin, EngineError, Result, Session};
use anicraft_doc::{Element, ElementId, ElementKind, LayerId};
use anicraft_geom::planar::SNAP;
use anicraft_geom::raw::{self as rawgeom, Cubic};
use anicraft_geom::walk::cubic_point;
use anicraft_geom::{Path, PathCmd, Rect, Vec2};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c(
            "path.bend_segment",
            "Bend Segment",
            "",
            None,
            "{x, y, dx, dy, id?, cmd?, t?, slop?, merge?: bool} the segment at (x, y) (of shape `id`, else the one there) bends so that point moves by (dx, dy), its ends staying; a raw shape's copies of the edge bend with it and merge again (unless merge is false)",
            true,
            bend_segment as crate::RunFn,
        ),
        c(
            "raw.move_point",
            "Move Point",
            "",
            None,
            "{x, y, to_x, to_y, with?: [[x, y], ..], points?: [[id, cmd], ..], snap?, slop?, merge?: bool} moves the corner or end at (x, y), with every raw shape that shares it, to (to_x, to_y), snapped within `snap` to points, lines and fill edges (while Snap to Objects is on); the anchors at `with` move by as much; returns the points to pass on while a drag goes on",
            true,
            move_point,
        ),
        c(
            "raw.select_rect",
            "Select Parts",
            "",
            None,
            "{x0, y0, x1, y1, add?: bool} the marquee, on every visible, unlocked layer: the parts of raw fills and strokes under the rectangle (cut at its edges, per layer) and the objects wholly inside it",
            true,
            select_rect,
        ),
        c("raw.merge", "Merge Shapes", "", None, "{ids?: [..]} the raw shapes (default: the selection) merge again with the raw shapes they touch, in place", true, merge_cmd),
        c("raw.hit", "Selection Tool Target", "", None, "{x, y, slop?, point_slop?} what a Selection-tool drag from (x, y) takes: a point, a segment to bend, a fill, or an object", false, hit),
    ]
}

/// Most points one move takes along.
const MAX_POINTS: usize = 10_000;
/// Most anchors and segments looked at when snapping.
const MAX_TARGETS: usize = 50_000;

// ----- what the Selection tool takes -----------------------------------------------------------

/// What a Selection-tool drag starting at a point takes (Animate's corner and curve cursors).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Grab {
    /// A corner or end that raw shapes share: the drag moves it.
    Point { at: Vec2 },
    /// A raw stroke segment or fill edge, at parameter `t` of command `cmd`: the drag bends it.
    Segment { id: ElementId, cmd: usize, t: f32, at: Vec2 },
    /// The inside of an unselected raw fill: the drag selects and moves it.
    Fill { id: ElementId },
    /// Anything else (an object, or something selected): the drag moves the selection.
    Object { id: ElementId },
}

impl Grab {
    pub fn key(&self) -> &'static str {
        match self {
            Grab::Point { .. } => "point",
            Grab::Segment { .. } => "segment",
            Grab::Fill { .. } => "fill",
            Grab::Object { .. } => "object",
        }
    }
}

/// A raw shape's anchors in stage space.
fn stage_anchors(e: &Element) -> Vec<(usize, Vec2)> {
    match mergeable(e) {
        Some(sh) => sh.path.editable_anchors().into_iter().map(|(i, p)| (i, e.transform.apply(p))).collect(),
        None => Vec::new(),
    }
}

impl Session {
    /// What a Selection-tool drag from `p` takes: a shared point within `point_slop`, a segment
    /// within `slop`, the inside of a fill, or the object there. Raw shapes that are selected
    /// count as objects (a drag moves the selection). `None` over empty space.
    pub fn raw_grab(&self, p: Vec2, slop: f32, point_slop: f32) -> Option<(LayerId, Grab)> {
        let (layer, id) = self.hit_test(p, slop)?;
        let object = Some((layer, Grab::Object { id }));
        if self.selection.contains(layer, id) {
            return object;
        }
        let els = self.elements_on(layer);
        let el = els.iter().find(|e| e.id == id)?;
        let Some(sh) = mergeable(el) else { return object };
        // A corner or end there, of any raw shape on the layer.
        let mut best: Option<(f32, Vec2)> = None;
        for e in els.iter().filter(|e| mergeable(e).is_some_and(|x| reach(x, &e.transform).inflate(point_slop).contains(p))) {
            for (_, q) in stage_anchors(e) {
                let d = q.distance(p);
                if d <= point_slop && best.is_none_or(|b| d < b.0) {
                    best = Some((d, q));
                }
            }
        }
        if let Some((_, at)) = best {
            return Some((layer, Grab::Point { at }));
        }
        // The edge of a fill bends; its inside moves.
        if sh.fill.is_some() && sh.path.distance_to_edge(p, &el.transform) > slop {
            return Some((layer, Grab::Fill { id }));
        }
        let (cmd, t, _) = sh.path.nearest_segment(p, &el.transform)?;
        let at = sh.path.segment_point(cmd, t).map(|q| el.transform.apply(q))?;
        Some((layer, Grab::Segment { id, cmd, t, at }))
    }
}

fn hit(s: &mut Session, p: &Value) -> Result<Value> {
    let at = Vec2::new(f32_req(p, "x")?, f32_req(p, "y")?);
    let slop = opt_f32(p, "slop")?.unwrap_or(2.0).clamp(0.0, 100.0);
    let point_slop = opt_f32(p, "point_slop")?.unwrap_or(slop * 1.5).clamp(0.0, 100.0);
    Ok(match s.raw_grab(at, slop, point_slop) {
        None => json!({"kind": null}),
        Some((layer, g)) => {
            let mut v = json!({"kind": g.key(), "layer": layer.0});
            match g {
                Grab::Point { at } => {
                    v["x"] = json!(at.x);
                    v["y"] = json!(at.y);
                }
                Grab::Segment { id, cmd, t, at } => {
                    v["id"] = json!(id.0);
                    v["cmd"] = json!(cmd);
                    v["t"] = json!(t);
                    v["x"] = json!(at.x);
                    v["y"] = json!(at.y);
                }
                Grab::Fill { id } | Grab::Object { id } => v["id"] = json!(id.0),
            }
            v
        }
    })
}

/// Makes the layer of what is at `at` (within `slop`) the active one, as a click does.
pub(crate) fn activate_layer_at(s: &mut Session, at: Vec2, slop: f32) {
    if let Some((layer, _)) = s.hit_test(at, slop)
        && layer != s.layer
    {
        s.layer = layer;
        s.selection.clear();
    }
}

pub(crate) fn check_extent(path: &Path) -> Result<()> {
    let b = path.bounds();
    if b.is_empty() {
        return Ok(());
    }
    if !(b.min.is_finite() && b.max.is_finite()) || b.width() > MAX_EXTENT || b.height() > MAX_EXTENT || b.min.x.abs().max(b.min.y.abs()).max(b.max.x.abs()).max(b.max.y.abs()) > MAX_EXTENT {
        return Err(EngineError::Param("that would make the shape too large".into()));
    }
    Ok(())
}

// ----- bending ----------------------------------------------------------------------------------

fn bend_segment(s: &mut Session, p: &Value) -> Result<Value> {
    let at = Vec2::new(f32_req(p, "x")?, f32_req(p, "y")?);
    let d = Vec2::new(f32_req(p, "dx")?, f32_req(p, "dy")?);
    let slop = opt_f32(p, "slop")?.unwrap_or(2.0).clamp(0.0, 100.0);
    let merge_after = opt_bool(p, "merge")?.unwrap_or(true);
    // The shape: `id`, else the one under the point (its layer becomes the active one).
    let id = match opt_u64(p, "id")? {
        Some(i) => ElementId(i),
        None => {
            activate_layer_at(s, at, slop);
            s.hit_test(at, slop).map(|(_, e)| e).ok_or_else(|| EngineError::State("no shape there".into()))?
        }
    };
    let els = s.visible_elements();
    let el = els.iter().find(|e| e.id == id).ok_or_else(|| EngineError::State(format!("element {} is not on the active layer at this frame", id.0)))?;
    let ElementKind::Shape(sh) = &el.kind else { return Err(EngineError::State("only shapes have segments to bend".into())) };
    let m = el.transform;
    let mut path = (*sh.path).clone();
    path.close_explicitly();
    let (cmd, t) = match opt_u64(p, "cmd")? {
        Some(c) => (usize::try_from(c).unwrap_or(usize::MAX), opt_f32(p, "t")?.ok_or_else(|| EngineError::Param("param 't' is required with 'cmd'".into()))?.clamp(0.0, 1.0)),
        None => {
            let (cmd, t, dist) = path.nearest_segment(at, &m).ok_or_else(|| EngineError::State("the shape has no segments".into()))?;
            if dist > slop {
                return Err(EngineError::State("no segment there".into()));
            }
            (cmd, t)
        }
    };
    let old = rawgeom::segment_cubic(&path, cmd).map(|c| rawgeom::transform(&c, &m)).ok_or_else(|| EngineError::Param(format!("no segment {cmd} on this shape")))?;
    let new = rawgeom::bend(&old, t, d).ok_or_else(|| EngineError::Param("cannot bend that segment".into()))?;
    let raw = mergeable(el).is_some();
    let changed = edit_edge(s, id, cmd, &old, &new, raw)?;
    let point = cubic_point(&new, t.clamp(0.02, 0.98));
    if raw && merge_after {
        remerge_in_place(s, &changed)?;
    }
    Ok(json!({"id": id.0, "cmd": cmd, "t": t, "x": point.x, "y": point.y, "changed": changed.iter().map(|e| e.0).collect::<Vec<_>>(), "merged": raw && merge_after}))
}

/// Replaces segment `cmd` of shape `id` (stage space: `old`) with `new` (same ends). When `raw`,
/// every raw shape's copy of the edge changes alike (a fill and its outline, two fills that
/// meet there); an object only changes itself. Returns the shapes that changed.
pub(crate) fn edit_edge(s: &mut Session, id: ElementId, cmd: usize, old: &Cubic, new: &Cubic, raw: bool) -> Result<Vec<ElementId>> {
    let area = new.iter().chain(old).fold(Rect::EMPTY, |mut r, q| {
        r.include(*q);
        r
    });
    let kf = s.editable_keyframe()?;
    let mut changed = Vec::new();
    for e in kf.elements.iter_mut() {
        let target = e.id == id;
        if !target && !(raw && mergeable(e).is_some_and(|x| !reach(x, &e.transform).intersect(&area).is_empty())) {
            continue;
        }
        let (Some(inv), ElementKind::Shape(x)) = (e.transform.inverse(), &e.kind) else { continue };
        let (ol, nl) = (rawgeom::transform(old, &inv), rawgeom::transform(new, &inv));
        let mut edited = (*x.path).clone();
        let n = if target {
            edited.close_explicitly();
            let end = edited.point(cmd, anicraft_geom::edit::ANCHOR);
            match (edited.cmds.get_mut(cmd), end) {
                (Some(c), Some(end)) => {
                    *c = PathCmd::CubicTo(nl[1], nl[2], end);
                    1
                }
                _ => 0,
            }
        } else {
            rawgeom::follow(&mut edited, &ol, &nl, SNAP / e.transform.mean_scale().max(1e-6))
        };
        if n == 0 {
            continue;
        }
        check_extent(&edited)?;
        if let ElementKind::Shape(x) = &mut Arc::make_mut(e).kind {
            x.path = Arc::new(edited);
        }
        changed.push(e.id);
    }
    Ok(changed)
}

/// Changes every copy of the edge `old` (stage space) into `new` (same ends): in every raw shape on
/// the editable keyframe, or only in `only` (a drawing object edits itself alone). Returns the
/// shapes that changed.
pub(crate) fn follow_edge(s: &mut Session, only: Option<ElementId>, old: &Cubic, new: &Cubic) -> Result<Vec<ElementId>> {
    let area = new.iter().chain(old).fold(Rect::EMPTY, |mut r, q| {
        r.include(*q);
        r
    });
    let kf = s.editable_keyframe()?;
    let mut changed = Vec::new();
    for e in kf.elements.iter_mut() {
        let take = match only {
            Some(id) => e.id == id,
            None => mergeable(e).is_some_and(|x| !reach(x, &e.transform).intersect(&area).is_empty()),
        };
        if !take {
            continue;
        }
        let (Some(inv), ElementKind::Shape(x)) = (e.transform.inverse(), &e.kind) else { continue };
        let mut edited = (*x.path).clone();
        if rawgeom::follow(&mut edited, &rawgeom::transform(old, &inv), &rawgeom::transform(new, &inv), SNAP / e.transform.mean_scale().max(1e-6)) == 0 {
            continue;
        }
        check_extent(&edited)?;
        if let ElementKind::Shape(x) = &mut Arc::make_mut(e).kind {
            x.path = Arc::new(edited);
        }
        changed.push(e.id);
    }
    Ok(changed)
}

// ----- moving shared points ---------------------------------------------------------------------

fn points_param(p: &Value) -> Result<Option<Vec<(ElementId, usize)>>> {
    let Some(v) = p.get("points").filter(|v| !v.is_null()) else { return Ok(None) };
    let bad = || EngineError::Param("param 'points' must be a list of [id, cmd] pairs".into());
    let list = v.as_array().ok_or_else(bad)?;
    if list.len() > MAX_POINTS {
        return Err(EngineError::Param(format!("param 'points' has more than {MAX_POINTS} points")));
    }
    let mut out = Vec::with_capacity(list.len());
    for pair in list {
        let pair = pair.as_array().filter(|a| a.len() == 2).ok_or_else(bad)?;
        let id = pair.first().and_then(Value::as_u64).ok_or_else(bad)?;
        let cmd = pair.get(1).and_then(Value::as_u64).ok_or_else(bad)?;
        out.push((ElementId(id), usize::try_from(cmd).unwrap_or(usize::MAX)));
    }
    if out.is_empty() {
        return Err(EngineError::Param("param 'points' is empty".into()));
    }
    Ok(Some(out))
}

/// A list of `[x, y]` stage positions (empty when absent).
pub(crate) fn positions_param(p: &Value, key: &str) -> Result<Vec<Vec2>> {
    let Some(v) = p.get(key).filter(|v| !v.is_null()) else { return Ok(Vec::new()) };
    let bad = || EngineError::Param(format!("param '{key}' must be a list of [x, y] pairs"));
    let list = v.as_array().ok_or_else(bad)?;
    if list.len() > MAX_POINTS {
        return Err(EngineError::Param(format!("param '{key}' has more than {MAX_POINTS} points")));
    }
    list.iter()
        .map(|xy| {
            let xy = xy.as_array().filter(|a| a.len() == 2).ok_or_else(bad)?;
            let n = |i: usize| xy.get(i).and_then(Value::as_f64).map(|f| f as f32).filter(|f| f.is_finite()).ok_or_else(bad);
            Ok(Vec2::new(n(0)?, n(1)?))
        })
        .collect()
}

/// Every raw shape's anchor at `at` (within [`SNAP`]) on the editable keyframe; shapes whose edge
/// passes there get an anchor there first.
fn shared_points(s: &mut Session, at: Vec2) -> Result<Vec<(ElementId, usize)>> {
    let kf = s.editable_keyframe()?;
    let mut out = Vec::new();
    for e in kf.elements.iter_mut() {
        let Some(sh) = mergeable(e) else { continue };
        if !reach(sh, &e.transform).contains(at) {
            continue;
        }
        let Some(inv) = e.transform.inverse() else { continue };
        let mut path = (*sh.path).clone();
        let len = path.cmds.len();
        let cmds = rawgeom::anchors_at(&mut path, inv.apply(at), SNAP / e.transform.mean_scale().max(1e-6));
        if path.cmds.len() != len
            && let ElementKind::Shape(x) = &mut Arc::make_mut(e).kind
        {
            x.path = Arc::new(path);
        }
        out.extend(cmds.into_iter().map(|c| (e.id, c)));
        if out.len() > MAX_POINTS {
            return Err(EngineError::State("too many shapes meet there".into()));
        }
    }
    Ok(out)
}

fn move_point(s: &mut Session, p: &Value) -> Result<Value> {
    let from = Vec2::new(f32_req(p, "x")?, f32_req(p, "y")?);
    let to = Vec2::new(f32_req(p, "to_x")?, f32_req(p, "to_y")?);
    let radius = opt_f32(p, "snap")?.unwrap_or(0.0).clamp(0.0, 1000.0);
    let slop = opt_f32(p, "slop")?.unwrap_or(2.0).clamp(0.0, 100.0);
    let merge_after = opt_bool(p, "merge")?.unwrap_or(true);
    let points = match points_param(p)? {
        Some(points) => points,
        None => {
            activate_layer_at(s, from, slop);
            // The nearest corner or end of a raw shape, then everything that shares it.
            let els = s.visible_elements();
            let at = els.iter().flat_map(|e| stage_anchors(e)).map(|(_, q)| q).filter(|q| q.distance(from) <= slop).min_by(|a, b| a.distance(from).total_cmp(&b.distance(from)));
            let at = at.ok_or_else(|| EngineError::State("no corner or end of a raw shape there".into()))?;
            let mut points = shared_points(s, at)?;
            // Other selected anchors (the Subselection tool) move by as much.
            for w in positions_param(p, "with")? {
                for pt in shared_points(s, w)? {
                    if !points.contains(&pt) {
                        points.push(pt);
                    }
                }
                if points.len() > MAX_POINTS {
                    return Err(EngineError::State("too many points to move at once".into()));
                }
            }
            points
        }
    };
    let els = s.visible_elements();
    let find = |id: ElementId| els.iter().find(|e| e.id == id);
    let &(id0, cmd0) = points.first().ok_or_else(|| EngineError::State("no point there".into()))?;
    let cur = find(id0).and_then(|e| match &e.kind {
        ElementKind::Shape(sh) => sh.path.point(cmd0, anicraft_geom::edit::ANCHOR).map(|q| e.transform.apply(q)),
        _ => None,
    });
    let cur = cur.ok_or_else(|| EngineError::Param(format!("no point ({}, {cmd0}) on the active layer", id0.0)))?;
    let snapped = if s.snaps(radius) { snap_point(s, &els, to, radius, &points) } else { None };
    let target = snapped.map_or(to, |(q, _)| q);
    let d = target - cur;
    let kf = s.editable_keyframe()?;
    let mut moved: BTreeSet<ElementId> = BTreeSet::new();
    for e in kf.elements.iter_mut() {
        let cmds: Vec<usize> = points.iter().filter(|(id, _)| *id == e.id).map(|(_, c)| *c).collect();
        if cmds.is_empty() {
            continue;
        }
        let (Some(inv), ElementKind::Shape(sh)) = (e.transform.inverse(), &e.kind) else { continue };
        let local = inv.apply_vector(d);
        let mut path = (*sh.path).clone();
        for cmd in cmds {
            if !path.move_anchor(cmd, local) && local.length() > 0.0 {
                return Err(EngineError::Param(format!("no point ({}, {cmd}) to move", e.id.0)));
            }
        }
        check_extent(&path)?;
        if let ElementKind::Shape(sh) = &mut Arc::make_mut(e).kind {
            sh.path = Arc::new(path);
        }
        moved.insert(e.id);
    }
    if points.iter().any(|(id, _)| !moved.contains(id)) {
        return Err(EngineError::Param("a point to move is not on the active layer at this frame".into()));
    }
    let ids: Vec<ElementId> = moved.into_iter().collect();
    if merge_after {
        remerge_in_place(s, &ids)?;
    }
    Ok(json!({
        "x": target.x, "y": target.y,
        "snapped": snapped.map(|(q, kind)| json!({"x": q.x, "y": q.y, "to": kind.key()})),
        "points": if merge_after { json!([]) } else { json!(points.iter().map(|(id, c)| [id.0, *c as u64]).collect::<Vec<_>>()) },
        "moved": ids.len(),
        "merged": merge_after,
    }))
}

// ----- snapping ---------------------------------------------------------------------------------

/// What a dragged point snapped to (the snap engine's, ADR 0076).
pub use crate::snap::SnapTo;
use crate::snap::{ObjectTargets, resolve};

/// Anchors and segments to snap to, in stage space.
#[derive(Default)]
struct Targets {
    points: Vec<Vec2>,
    segs: Vec<Cubic>,
}

/// The shapes' anchors and segments, leaving out the anchors `skip_point(id, cmd)` and the
/// segments `skip_seg(id, cmd)` says move along.
fn targets(els: &[Arc<Element>], skip_el: impl Fn(ElementId) -> bool, skip_point: impl Fn(ElementId, usize) -> bool, skip_seg: impl Fn(ElementId, usize) -> bool) -> Targets {
    let mut t = Targets::default();
    for e in els.iter().filter(|e| !skip_el(e.id)) {
        let ElementKind::Shape(sh) = &e.kind else { continue };
        if t.points.len() + t.segs.len() > MAX_TARGETS {
            break;
        }
        let mut path = (*sh.path).clone();
        path.close_explicitly();
        for (i, q) in path.editable_anchors() {
            if !skip_point(e.id, i) {
                t.points.push(e.transform.apply(q));
            }
        }
        for k in 0..path.cmds.len().min(MAX_TARGETS) {
            if !skip_seg(e.id, k)
                && let Some(c) = rawgeom::segment_cubic(&path, k)
            {
                t.segs.push(rawgeom::transform(&c, &e.transform));
            }
        }
    }
    t
}

impl ObjectTargets for Targets {
    /// The nearest anchor within `radius` of `p`.
    fn nearest_point(&self, p: Vec2, radius: f32) -> Option<(Vec2, f32)> {
        self.points.iter().map(|q| (*q, q.distance(p))).filter(|(_, d)| *d <= radius).min_by(|a, b| a.1.total_cmp(&b.1))
    }
    /// The nearest point on a segment within `radius` of `p`.
    fn nearest_line(&self, p: Vec2, radius: f32) -> Option<(Vec2, f32)> {
        let near = |c: &Cubic| {
            let (lo, hi) = c.iter().fold((Vec2::new(f32::INFINITY, f32::INFINITY), Vec2::new(f32::NEG_INFINITY, f32::NEG_INFINITY)), |(lo, hi), q| (Vec2::new(lo.x.min(q.x), lo.y.min(q.y)), Vec2::new(hi.x.max(q.x), hi.y.max(q.y))));
            p.x >= lo.x - radius && p.x <= hi.x + radius && p.y >= lo.y - radius && p.y <= hi.y + radius
        };
        self.segs
            .iter()
            .filter(|c| near(c))
            .map(|c| {
                let (u, d) = rawgeom::nearest(c, p);
                (cubic_point(c, u), d)
            })
            .filter(|(_, d)| *d <= radius)
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }
}

/// Where a point being moved (`moving`: the anchors that go with it) snaps near `to`: the
/// artwork that stays put, the guides, the grid and pixels (ADR 0076).
fn snap_point(s: &Session, els: &[Arc<Element>], to: Vec2, radius: f32, moving: &[(ElementId, usize)]) -> Option<(Vec2, SnapTo)> {
    if !s.style.snap_objects {
        return resolve(s, to, radius, None);
    }
    // The segments leaving the moving anchors move too.
    let mut segs: BTreeSet<(ElementId, usize)> = BTreeSet::new();
    for e in els {
        let ElementKind::Shape(sh) = &e.kind else { continue };
        for (_, cmd) in moving.iter().filter(|(id, _)| *id == e.id) {
            let (i, o) = sh.path.anchor_segments(*cmd);
            segs.extend(i.into_iter().chain(o).map(|k| (e.id, k)));
        }
    }
    let t = targets(els, |_| false, |id, c| moving.contains(&(id, c)), |id, k| segs.contains(&(id, k)));
    resolve(s, to, radius, Some(&t))
}

/// Where a point drawn at `p` snaps (the Line, Rectangle, Oval, PolyStar and Pen tools, ADR
/// 0045; the Free Transform tool's handles): the nearest anchor within `radius`, else a guide,
/// a line or fill edge of what the stage shows near it, the grid or a pixel (ADR 0076). With
/// `moving`, the selection is what moves, so it isn't snapped to.
pub(crate) fn snap_drawn(s: &Session, p: Vec2, radius: f32, moving: bool) -> Option<(Vec2, SnapTo)> {
    if !(radius > 0.0 && p.is_finite()) {
        return None;
    }
    if !s.style.snap_objects {
        return resolve(s, p, radius, None);
    }
    let dl = s.display_list();
    let mut t = Targets::default();
    for item in dl.items.iter().filter(|i| !(moving && s.selection.contains(i.layer, i.element))) {
        if t.points.len() + t.segs.len() > MAX_TARGETS {
            break;
        }
        let pad = item.stroke.as_ref().map_or(0.0, |st| st.reach() * item.transform.mean_scale());
        if !item.path.bounds().transformed(&item.transform).inflate(pad + radius).contains(p) {
            continue;
        }
        let mut path = (*item.path).clone();
        path.close_explicitly();
        t.points.extend(path.editable_anchors().into_iter().map(|(_, q)| item.transform.apply(q)));
        t.segs.extend((0..path.cmds.len().min(MAX_TARGETS)).filter_map(|k| rawgeom::segment_cubic(&path, k)).map(|c| rawgeom::transform(&c, &item.transform)));
    }
    resolve(s, p, radius, Some(&t))
}

/// After the selection moved: the extra move that snaps one of its anchors or its bounding
/// box's corners or centre (the nearest to `from` when given, else whichever snaps best) onto
/// an anchor or line of the shapes that did not move, a guide, the grid or a pixel, within
/// `radius` (ADR 0041, 0076).
pub(crate) fn snap_selection(s: &Session, radius: f32, from: Option<Vec2>) -> Option<(Vec2, Vec2, SnapTo)> {
    let mut own: Vec<Vec2> = Vec::new();
    // The box's corners and centre: what symbols, text and bitmaps snap by. The selection has
    // just moved, so the display list is looked at again.
    s.dl_cache.replace(None);
    let b = s.selection_bounds();
    if !b.is_empty() {
        let c = Vec2::new((b.min.x + b.max.x) * 0.5, (b.min.y + b.max.y) * 0.5);
        own.extend([b.min, Vec2::new(b.max.x, b.min.y), b.max, Vec2::new(b.min.x, b.max.y), c]);
    }
    for (_, e) in s.selected_elements() {
        if let ElementKind::Shape(sh) = &e.kind {
            own.extend(sh.path.editable_anchors().into_iter().map(|(_, q)| e.transform.apply(q)));
        }
        if own.len() > 4 * MAX_POINTS {
            break;
        }
    }
    let candidates: Vec<Vec2> = match from {
        Some(f) => own.iter().copied().min_by(|a, b| a.distance(f).total_cmp(&b.distance(f))).into_iter().collect(),
        None => own.into_iter().take(512).collect(),
    };
    if candidates.is_empty() {
        return None;
    }
    // What stays put on the layers the selection is on (ADR 0044).
    let t = s.style.snap_objects.then(|| {
        let rest: Vec<Arc<Element>> = s.selected_layers().into_iter().flat_map(|l| s.elements_on(l).into_iter().filter(move |e| !s.selection.contains(l, e.id))).collect();
        targets(&rest, |_| false, |_, _| false, |_, _| false)
    });
    let t = t.as_ref().map(|t| t as &dyn ObjectTargets);
    candidates
        .into_iter()
        .filter_map(|c| resolve(s, c, radius, t).map(|(q, k)| (c, q, k, q.distance(c))))
        .min_by(|a, b| a.2.rank().cmp(&b.2.rank()).then(a.3.total_cmp(&b.3)))
        .map(|(c, q, k, _)| (q - c, q, k))
}

// ----- the marquee ------------------------------------------------------------------------------

fn contains(outer: &Rect, inner: &Rect) -> bool {
    inner.min.x >= outer.min.x && inner.min.y >= outer.min.y && inner.max.x <= outer.max.x && inner.max.y <= outer.max.y
}

/// Whether the playhead is on a keyframe whose artwork can be cut here (not inside a tween, where
/// an edit would add a keyframe).
fn on_editable_key(s: &Session) -> bool {
    s.active_layer().and_then(|l| l.keyframe_at(s.frame)).is_some_and(|k| k.motion.is_none() && (k.tween.is_none() || k.start == s.frame))
}

/// The marquee (ADR 0041, 0044): on every visible, unlocked layer, the parts of raw shapes under
/// the rectangle (cut at its edges) and the objects wholly inside it.
fn select_rect(s: &mut Session, p: &Value) -> Result<Value> {
    let a = Vec2::new(f32_req(p, "x0")?, f32_req(p, "y0")?);
    let b = Vec2::new(f32_req(p, "x1")?, f32_req(p, "y1")?);
    let add = opt_bool(p, "add")?.unwrap_or(false);
    let r = Rect::from_points(a, b);
    if !add {
        s.selection.clear();
    }
    if r.width() <= 0.0 || r.height() <= 0.0 {
        return Ok(json!({"selected": s.selection.id_list(), "cut": 0}));
    }
    // What each layer has under the marquee, from one display list: wholly inside, or (raw
    // shapes) partly.
    let mut found: Vec<(LayerId, Vec<ElementId>, Vec<ElementId>)> = Vec::new();
    let layers: Vec<LayerId> = s.timeline().layers.iter().map(|l| l.id).filter(|l| s.gathers(*l)).collect();
    for l in layers {
        let (mut picked, mut partial) = (Vec::new(), Vec::new());
        for e in s.elements_on(l) {
            let bounds = s.element_bounds_on(l, e.id);
            if bounds.is_empty() || bounds.intersect(&r).is_empty() {
                continue;
            }
            if contains(&r, &bounds) {
                picked.push(e.id);
            } else if mergeable(&e).is_some() {
                partial.push(e.id);
            }
        }
        if !picked.is_empty() || !partial.is_empty() {
            found.push((l, picked, partial));
        }
    }
    let mut cut = 0;
    let (mut origin, mut inside): (CutOrigin, Vec<ElementId>) = (Vec::new(), Vec::new());
    let mut hit: Vec<LayerId> = Vec::new();
    for (l, picked, partial) in found {
        let (n, o, i) = s.with_layer(l, |s| marquee_layer(s, &r, picked, &partial))?;
        cut += n;
        origin.extend(o);
        inside.extend(i);
        hit.push(l);
    }
    if !inside.is_empty() {
        s.pending_cut = Some((origin, inside));
    }
    // The active layer stays when the marquee took something there, else the topmost that it
    // did becomes active, as a click would make it.
    if !hit.contains(&s.layer)
        && let Some(&top) = hit.first()
    {
        s.layer = top;
    }
    Ok(json!({"selected": s.selection.id_list(), "cut": cut}))
}

/// The marquee on the active layer: raw shapes partly under `r` are cut at its edges and the
/// parts inside are selected, the rest stay as they are; `picked` (wholly inside) are selected.
/// How many pieces it cut, each piece with the shape it came from, and the pieces inside.
fn marquee_layer(s: &mut Session, r: &Rect, mut picked: Vec<ElementId>, partial: &[ElementId]) -> Result<(usize, CutOrigin, Vec<ElementId>)> {
    let here = s.layer;
    let mut cut = 0;
    let (mut origin, mut inside_all) = (Vec::new(), Vec::new());
    if !partial.is_empty() && on_editable_key(s) {
        let kf = s.editable_keyframe()?;
        let existing = kf.elements.clone();
        let idx: Vec<usize> = existing.iter().enumerate().filter(|(_, e)| partial.contains(&e.id)).map(|(i, _)| i).collect();
        // Parts already selected (an earlier marquee, with Shift) stay selected, cut or not.
        let parts: Vec<Part> = idx.iter().filter_map(|&i| existing.get(i)).map(|e| Part::Shape(e.as_ref(), s.selection.contains(here, e.id))).collect();
        let cutter = Path::rect(r.min.x, r.min.y, r.width(), r.height());
        match merge::remerge_with(s, &parts, Some(&cutter)) {
            Some(rm) if !rm.inside.is_empty() => {
                origin = rm.from.iter().filter_map(|&(piece, part)| idx.get(part).and_then(|&i| existing.get(i)).map(|e| (piece, e.id))).collect();
                cut = rm.inside.len();
                let gone: Vec<ElementId> = idx.iter().filter_map(|&i| existing.get(i)).map(|e| e.id).collect();
                s.selection.retain(|l, id| l != here || !gone.contains(&id));
                let mut inside = rm.inside;
                inside.extend(rm.picked.iter().filter(|id| !inside.contains(id)).copied().collect::<Vec<_>>());
                picked.extend(inside.iter().copied());
                merge::replace(s, &idx, rm.out, false)?;
                inside_all = inside;
            }
            // Only their bounds reached in: nothing of them is under the marquee.
            Some(_) => {}
            // Too much to cut at once: they are selected whole.
            None => picked.extend(partial.iter().copied()),
        }
    } else {
        picked.extend(partial.iter().copied());
    }
    s.selection.extend_on(here, picked);
    Ok((cut, origin, inside_all))
}

// ----- merging again ----------------------------------------------------------------------------

fn merge_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    match opt_u64_list(p, "ids")? {
        Some(v) => remerge_in_place(s, &v.into_iter().map(ElementId).collect::<Vec<_>>())?,
        // The selection's shapes, on each layer they are on.
        None => {
            s.each_layer(true, |s| {
                let ids = s.selection.on(s.layer);
                remerge_in_place(s, &ids)?;
                Ok(json!({}))
            })?;
        }
    }
    Ok(json!({"selected": s.selection.id_list()}))
}

/// Merges the raw shapes `ids` again with the raw shapes they touch, keeping their stacking (an
/// edit changed them in place). Selected shapes' pieces stay selected.
pub(crate) fn remerge_in_place(s: &mut Session, ids: &[ElementId]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let kf = s.editable_keyframe()?;
    let existing = kf.elements.clone();
    let area = existing.iter().filter(|e| ids.contains(&e.id)).filter_map(|e| mergeable(e).map(|sh| reach(sh, &e.transform))).fold(Rect::EMPTY, |r, b| r.union(&b));
    if area.is_empty() {
        return Ok(());
    }
    let touching: Vec<usize> = existing.iter().enumerate().filter(|(_, e)| mergeable(e).is_some_and(|sh| !reach(sh, &e.transform).intersect(&area).is_empty())).map(|(i, _)| i).collect();
    let here = s.layer;
    let parts: Vec<Part> = touching.iter().filter_map(|&i| existing.get(i)).map(|e| Part::Shape(e.as_ref(), s.selection.contains(here, e.id))).collect();
    // Too much to merge at once: the shapes stay as edited.
    let Some((out, picked)) = merge::remerge(s, &parts) else { return Ok(()) };
    let gone: Vec<ElementId> = touching.iter().filter_map(|&i| existing.get(i)).map(|e| e.id).collect();
    merge::replace(s, &touching, out, false)?;
    s.selection.retain(|l, id| l != here || !gone.contains(&id));
    s.selection.extend_on(here, picked);
    Ok(())
}
