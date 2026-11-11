//! Reshaping raw shapes with the Selection tool (ADR 0031): a drag from a corner or a line end
//! moves that point on every raw shape that shares it, and a drag from a segment bends it,
//! together with the fill edges that run along it. Paths are addressed with every edge an
//! explicit segment (`Path::close_explicitly`).

use crate::cmds_draw::MAX_EXTENT;
use crate::merge::{drop_ids, mergeable};
use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::{Element, ElementId, ElementKind};
use anicraft_geom::edit::ANCHOR;
use anicraft_geom::planar::SNAP;
use anicraft_geom::{Path, Vec2};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c(
            "draw.grab",
            "Grab Raw Shape",
            "",
            None,
            "{x, y, slop?} what a drag from x, y would reshape: {what: \"point\"|\"curve\"|null, x, y, parts: [[id, cmd, t?], ..]}",
            false,
            grab as crate::RunFn,
        ),
        c(
            "draw.reshape",
            "Reshape",
            "",
            None,
            "{x, y, to_x, to_y, parts? (from draw.grab; default: what is at x, y), slop?, snap? (a moved point lands on a raw point or edge this close), drop? (default true: merge afterwards)} moves the raw point at x, y, or bends the raw segment through it, to to_x, to_y",
            true,
            reshape,
        ),
    ]
}

/// A raw shape's anchor (`t` is `None`) or the point at `t` on one of its segments.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Part {
    id: ElementId,
    cmd: usize,
    t: Option<f32>,
}

/// A raw shape on the active layer, with its explicit path and how far its stroke reaches out.
struct Raw {
    el: Arc<Element>,
    path: Path,
    reach: f32,
}

/// The raw shapes on the active layer whose outline comes within `r` of `p`.
fn raw_near(s: &Session, p: Vec2, r: f32) -> Vec<Raw> {
    s.visible_elements()
        .into_iter()
        .filter_map(|el| {
            let sh = mergeable(&el)?;
            let reach = sh.stroke.as_ref().map_or(0.0, |st| st.width * 0.5) * el.transform.mean_scale();
            if !sh.path.bounds().transformed(&el.transform).inflate(reach + r).contains(p) {
                return None;
            }
            let mut path = (*sh.path).clone();
            path.close_explicitly();
            Some(Raw { el, path, reach })
        })
        .collect()
}

/// What a drag from `p` takes: the nearest raw corner or line end within `slop` and every raw
/// anchor on it, else the nearest raw segment within `slop` and the segments along it.
fn grab_at(s: &Session, p: Vec2, slop: f32) -> Option<(Vec2, Vec<Part>)> {
    let raws = raw_near(s, p, slop);
    let mut corner: Option<(f32, Vec2)> = None;
    for r in &raws {
        for (_, q) in r.path.anchors() {
            let k = r.el.transform.apply(q);
            let d = k.distance(p) - r.reach;
            if d <= slop && corner.is_none_or(|b| d < b.0) {
                corner = Some((d, k));
            }
        }
    }
    if let Some((_, at)) = corner {
        let parts = raw_near(s, at, SNAP)
            .iter()
            .flat_map(|r| r.path.anchors().into_iter().filter(|(_, q)| r.el.transform.apply(*q).distance(at) <= SNAP).map(|(cmd, _)| Part { id: r.el.id, cmd, t: None }))
            .collect();
        return Some((at, parts));
    }
    let mut best: Option<(f32, &Raw, usize, f32)> = None;
    for r in &raws {
        if let Some((cmd, t, d)) = r.path.nearest_segment(p, &r.el.transform)
            && d - r.reach <= slop
            && best.is_none_or(|b| d - r.reach < b.0)
        {
            best = Some((d - r.reach, r, cmd, t));
        }
    }
    let (_, hit, cmd, t) = best?;
    let m = hit.el.transform;
    let at = m.apply(hit.path.segment_point(cmd, t)?);
    let (a, b) = (m.apply(hit.path.segment_start(cmd)?), m.apply(hit.path.point(cmd, ANCHOR)?));
    // The same edge on the other raw shapes (a fill's border under a stroke), either way round.
    let mut parts = Vec::new();
    for r in &raw_near(s, at, 4.0 * SNAP) {
        let m = r.el.transform;
        for (k, s0, s1) in r.path.segment_ends() {
            let (s0, s1) = (m.apply(s0), m.apply(s1));
            let forward = (s0.distance(a) <= SNAP && s1.distance(b) <= SNAP).then_some(t);
            let backward = (s0.distance(b) <= SNAP && s1.distance(a) <= SNAP).then_some(1.0 - t);
            let on = |u: f32| r.path.segment_point(k, u).is_some_and(|q| m.apply(q).distance(at) <= 4.0 * SNAP);
            if let Some(u) = forward.filter(|&u| on(u)).or(backward.filter(|&u| on(u))) {
                parts.push(Part { id: r.el.id, cmd: k, t: Some(u) });
            }
        }
    }
    Some((at, parts))
}

fn parts_json(parts: &[Part]) -> Value {
    parts
        .iter()
        .map(|q| match q.t {
            Some(t) => json!([q.id.0, q.cmd, t]),
            None => json!([q.id.0, q.cmd]),
        })
        .collect()
}

/// Most parts one reshape takes.
const MAX_PARTS: usize = 10_000;

fn parts_param(v: &Value) -> Result<Vec<Part>> {
    let bad = || EngineError::Param("param 'parts' must be a list of [id, cmd] or [id, cmd, t] (as draw.grab returns)".into());
    let list = v.as_array().ok_or_else(bad)?;
    if list.len() > MAX_PARTS {
        return Err(EngineError::Param(format!("param 'parts' has more than {MAX_PARTS} entries")));
    }
    list.iter()
        .map(|q| {
            let a = q.as_array().filter(|a| matches!(a.len(), 2 | 3)).ok_or_else(bad)?;
            let id = a.first().and_then(Value::as_u64).ok_or_else(bad)?;
            let cmd = a.get(1).and_then(Value::as_u64).and_then(|c| usize::try_from(c).ok()).ok_or_else(bad)?;
            let t = match a.get(2) {
                None => None,
                Some(t) => Some(t.as_f64().map(|t| t as f32).filter(|t| (0.0..=1.0).contains(t)).ok_or_else(bad)?),
            };
            Ok(Part { id: ElementId(id), cmd, t })
        })
        .collect()
}

fn grab(s: &mut Session, p: &Value) -> Result<Value> {
    let at = Vec2::new(f32_req(p, "x")?, f32_req(p, "y")?);
    let slop = opt_f32(p, "slop")?.unwrap_or(4.0).clamp(0.0, 100.0);
    Ok(match grab_at(s, at, slop) {
        Some((at, parts)) => json!({"what": what(&parts), "x": at.x, "y": at.y, "parts": parts_json(&parts)}),
        None => json!({"what": null}),
    })
}

fn what(parts: &[Part]) -> &'static str {
    if parts.first().is_some_and(|q| q.t.is_some()) { "curve" } else { "point" }
}

/// Where a point moved to `target` lands: on the nearest raw anchor within `r` that is not one
/// of `parts`, else on the nearest raw edge within `r` that does not move with it. `at` is where
/// the moving point is now.
fn snap_to(s: &Session, parts: &[Part], at: Vec2, target: Vec2, r: f32) -> Option<Vec2> {
    let raws = raw_near(s, target, r);
    let mut moving: HashSet<(ElementId, usize)> = HashSet::new();
    for q in parts {
        if let Some(raw) = raws.iter().find(|x| x.el.id == q.id) {
            let c = raw.path.canonical_anchor(q.cmd);
            moving.insert((q.id, c));
            moving.extend(raw.path.linked_anchor(c).map(|k| (q.id, k)));
        }
    }
    let mut best: Option<(f32, Vec2)> = None;
    for x in &raws {
        for (cmd, q) in x.path.anchors() {
            let k = x.el.transform.apply(q);
            let d = k.distance(target);
            if d <= r && !moving.contains(&(x.el.id, cmd)) && best.is_none_or(|b| d < b.0) {
                best = Some((d, k));
            }
        }
    }
    if best.is_some() {
        return best.map(|b| b.1);
    }
    for x in &raws {
        let m = x.el.transform;
        // The segments that start or end at the moving point move with it.
        let moves: HashSet<usize> = x.path.segment_ends().into_iter().filter(|(_, a, b)| m.apply(*a).distance(at) <= SNAP || m.apply(*b).distance(at) <= SNAP).map(|(k, ..)| k).collect();
        if let Some((cmd, t, d)) = x.path.nearest_segment_where(target, &m, |k| !moves.contains(&k))
            && d <= r
            && best.is_none_or(|b| d < b.0)
            && let Some(q) = x.path.segment_point(cmd, t)
        {
            best = Some((d, m.apply(q)));
        }
    }
    best.map(|b| b.1)
}

fn reshape(s: &mut Session, p: &Value) -> Result<Value> {
    let at = Vec2::new(f32_req(p, "x")?, f32_req(p, "y")?);
    let to = Vec2::new(f32_req(p, "to_x")?, f32_req(p, "to_y")?);
    if to.x.abs() > MAX_EXTENT || to.y.abs() > MAX_EXTENT {
        return Err(EngineError::Param("the move is too large".into()));
    }
    let slop = opt_f32(p, "slop")?.unwrap_or(4.0).clamp(0.0, 100.0);
    let snap = opt_f32(p, "snap")?.unwrap_or(0.0).clamp(0.0, 1000.0);
    let drop = opt_bool(p, "drop")?.unwrap_or(true);
    let parts = match p.get("parts") {
        Some(v) if !v.is_null() => parts_param(v)?,
        _ => match grab_at(s, at, slop) {
            Some((_, parts)) => parts,
            None => return Ok(json!({"what": null})),
        },
    };
    let curve = what(&parts) == "curve";
    if parts.iter().any(|q| q.t.is_some() != curve) {
        return Err(EngineError::Param("param 'parts' mixes points and segments".into()));
    }
    let raw: HashSet<ElementId> = s.visible_elements().iter().filter(|e| mergeable(e).is_some()).map(|e| e.id).collect();
    if let Some(q) = parts.iter().find(|q| !raw.contains(&q.id)) {
        return Err(EngineError::State(format!("element {} is not a raw shape on the active layer at this frame", q.id.0)));
    }
    let snapped = if curve || snap == 0.0 { None } else { snap_to(s, &parts, at, to, snap) };
    let target = snapped.unwrap_or(to);
    let d = target - at;
    let ids: HashSet<ElementId> = parts.iter().map(|q| q.id).collect();
    if d != Vec2::ZERO {
        let kf = s.editable_keyframe()?;
        for el in kf.elements_mut_where(|id| ids.contains(&id)) {
            let local = el.transform.inverse().map(|inv| inv.apply_vector(d)).filter(|v| v.is_finite()).ok_or_else(|| EngineError::State("a shape is scaled to zero".into()))?;
            let id = el.id;
            let ElementKind::Shape(sh) = &mut el.kind else { continue };
            let path = Arc::make_mut(&mut sh.path);
            path.close_explicitly();
            // Each point and segment moves once, whichever copy of it the parts name.
            let mut done = HashSet::new();
            for q in parts.iter().filter(|q| q.id == id) {
                let ok = match q.t {
                    None => {
                        let c = path.canonical_anchor(q.cmd);
                        path.point(c, ANCHOR).is_some() && (!done.insert(c) || path.move_anchor(c, local))
                    }
                    Some(t) => !done.insert(q.cmd) || path.bend_segment(q.cmd, t, local),
                };
                if !ok {
                    return Err(EngineError::Param(format!("element {} has no {} {}", id.0, if curve { "segment" } else { "point" }, q.cmd)));
                }
            }
            let b = path.bounds();
            if !(b.min.is_finite() && b.max.is_finite()) || b.width() > MAX_EXTENT || b.height() > MAX_EXTENT {
                return Err(EngineError::Param("the move is too large".into()));
            }
        }
    }
    let out = json!({"what": what(&parts), "x": target.x, "y": target.y, "parts": parts_json(&parts), "snapped": snapped.is_some()});
    if drop {
        let ids: Vec<ElementId> = ids.into_iter().collect();
        drop_ids(s, &ids)?;
    }
    Ok(out)
}
