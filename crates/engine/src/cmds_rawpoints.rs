//! Adding and deleting the anchors of raw drawings, making them smooth or corners, and where a
//! point being drawn snaps (ADR 0045; issue #42 §1 and §3).
//!
//! A layer's merged drawing stores a shared point once in each shape that has it (ADR 0031), so
//! these work on positions, as the Subselection tool shows them (`raw.points`), and change every
//! copy alike: an added point is added to every shape with that edge; a deleted one is joined
//! across in every shape that has it, so the drawing stays connected.

use crate::cmds_raw::{activate_layer_at, check_extent, follow_edge, positions_param, remerge_in_place, snap_drawn};
use crate::merge::{mergeable, reach};
use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::{ElementId, ElementKind, Keyframe};
use anicraft_geom::planar::SNAP;
use anicraft_geom::raw::{self as rawgeom, Seg, Sub};
use anicraft_geom::{Path, Vec2};
use serde_json::{Value, json};
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c(
            "raw.add_point",
            "Add Anchor Point",
            "",
            None,
            "{x, y, slop?} adds an anchor on the raw line or fill edge at (x, y): every raw shape with that edge gets it, and the curve keeps its shape; returns where it is",
            true,
            add_point as crate::RunFn,
        ),
        c(
            "raw.delete_points",
            "Delete Anchor Points",
            "",
            None,
            "{points: [[x, y], ..]} deletes the raw anchors at those positions (as raw.points lists them): every shape that has one is joined across it, so the drawing stays connected; a free end shortens its line; a shape left with nothing is removed; a point where three or more edges meet is kept",
            true,
            delete_points,
        ),
        c(
            "raw.convert_points",
            "Convert Anchor Points",
            "",
            None,
            "{points: [[x, y], ..], smooth: bool} makes the raw anchors at those positions smooth (their handles in one line) or corners (handles drawn in), in every shape that has them",
            true,
            convert_points,
        ),
        c(
            "draw.snap",
            "Snap Point",
            "",
            None,
            "{x, y, snap, moving?: bool} where a point drawn at (x, y) snaps within `snap` stage units, by the snap engine's priority (ADR 0076): an anchor (Snap to Objects), a guide (Snap to Guides), a line or fill edge, the grid (Snap to Grid), a whole pixel (Snap to Pixels); `moving`: the selection is what moves (a Free Transform handle), so it isn't a target; returns {x, y, snapped: {x, y, to: point|guide|line|grid|pixel} | null}",
            false,
            snap_cmd,
        ),
    ]
}

/// Ends of segments this near a position are at it (the copies of a shared point lie within
/// `SNAP` of each other, ADR 0042).
const ENDS: f32 = 1.5 * SNAP;

/// The raw shapes on `kf` near `at`: index, whether filled, and stage-space subpaths.
fn near(kf: &Keyframe, at: Vec2) -> Vec<(usize, bool, Vec<Sub>)> {
    kf.elements
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            let sh = mergeable(e)?;
            reach(sh, &e.transform).contains(at).then(|| (i, sh.fill.is_some(), rawgeom::subpaths(&sh.path.transformed(&e.transform))))
        })
        .collect()
}

/// The distinct edges leaving `at` among `shapes`.
fn edges(shapes: &[(usize, bool, Vec<Sub>)], at: Vec2) -> Vec<Seg> {
    rawgeom::distinct_edges(shapes.iter().flat_map(|(_, _, subs)| rawgeom::edges_at(subs, at, ENDS)), ENDS)
}

fn points(p: &Value) -> Result<Vec<Vec2>> {
    let pts = positions_param(p, "points")?;
    if pts.is_empty() {
        return Err(EngineError::Param("param 'points' is required: the positions of the anchors, as raw.points lists them".into()));
    }
    Ok(pts)
}

// ----- adding ------------------------------------------------------------------------------------

fn add_point(s: &mut Session, p: &Value) -> Result<Value> {
    let at = Vec2::new(f32_req(p, "x")?, f32_req(p, "y")?);
    let slop = opt_f32(p, "slop")?.unwrap_or(4.0).clamp(0.0, 100.0);
    activate_layer_at(s, at, slop);
    // The nearest raw line or fill edge.
    let mut best: Option<(f32, Vec2)> = None;
    for e in s.visible_elements() {
        let Some(sh) = mergeable(&e) else { continue };
        if !reach(sh, &e.transform).inflate(slop).contains(at) {
            continue;
        }
        let mut path = (*sh.path).clone();
        path.close_explicitly();
        let half = sh.stroke.as_ref().map_or(0.0, |st| st.width * 0.5 * e.transform.mean_scale());
        if let Some((cmd, t, d)) = path.nearest_segment(at, &e.transform)
            && d <= slop + half
            && best.is_none_or(|b| d < b.0)
            && let Some(q) = path.segment_point(cmd, t)
        {
            best = Some((d, e.transform.apply(q)));
        }
    }
    let (_, q) = best.ok_or_else(|| EngineError::State("no line or fill edge of a merged drawing there".into()))?;
    let kf = s.editable_keyframe()?;
    if near(kf, q).iter().any(|(_, _, subs)| subs.iter().flat_map(|sb| &sb.segs).any(|g| g.c[0].distance(q) <= 2.0 * SNAP || g.c[3].distance(q) <= 2.0 * SNAP)) {
        return Err(EngineError::State("there is a point there already".into()));
    }
    let mut changed = Vec::new();
    for e in kf.elements.iter_mut() {
        let Some(sh) = mergeable(e) else { continue };
        if !reach(sh, &e.transform).contains(q) {
            continue;
        }
        let Some(inv) = e.transform.inverse() else { continue };
        let mut path = (*sh.path).clone();
        path.close_explicitly();
        let len = path.cmds.len();
        // Splits the segment through the point (de Casteljau: the curve is unchanged).
        rawgeom::anchors_at(&mut path, inv.apply(q), SNAP / e.transform.mean_scale().max(1e-6));
        if path.cmds.len() == len {
            continue;
        }
        if let ElementKind::Shape(x) = &mut Arc::make_mut(e).kind {
            x.path = Arc::new(path);
        }
        changed.push(e.id);
    }
    if changed.is_empty() {
        return Err(EngineError::State("cannot add a point there".into()));
    }
    // Nothing moved, so nothing merges: the point stays until a merge finds it on a straight
    // line that it no longer bends (issue #42 §1).
    Ok(json!({"x": q.x, "y": q.y, "changed": changed.iter().map(|e| e.0).collect::<Vec<_>>()}))
}

// ----- deleting ----------------------------------------------------------------------------------

fn delete_points(s: &mut Session, p: &Value) -> Result<Value> {
    let pts = points(p)?;
    let (mut deleted, mut kept) = (0usize, 0usize);
    let mut changed: Vec<ElementId> = Vec::new();
    let mut removed: Vec<ElementId> = Vec::new();
    for at in pts {
        let kf = s.editable_keyframe()?;
        let shapes = near(kf, at);
        let edges = edges(&shapes, at);
        let joined = match edges.as_slice() {
            [] => continue,
            [_] => None,
            [a, b] => Some(rawgeom::join(a, b).ok_or_else(|| EngineError::Param("cannot join the lines across that point".into()))?),
            _ => {
                kept += 1;
                continue;
            }
        };
        let mut gone: Vec<ElementId> = Vec::new();
        let mut any = false;
        for (i, filled, mut subs) in shapes {
            if !rawgeom::delete_anchor(&mut subs, at, &edges, joined.as_ref(), ENDS) {
                continue;
            }
            rawgeom::tidy(&mut subs, filled);
            let Some(e) = kf.elements.get_mut(i) else { continue };
            any = true;
            if subs.is_empty() {
                gone.push(e.id);
                continue;
            }
            let Some(inv) = e.transform.inverse() else { continue };
            let path: Path = rawgeom::from_subpaths(&subs).transformed(&inv);
            check_extent(&path)?;
            if let ElementKind::Shape(x) = &mut Arc::make_mut(e).kind {
                x.path = Arc::new(path);
            }
            if !changed.contains(&e.id) {
                changed.push(e.id);
            }
        }
        kf.elements.retain(|e| !gone.contains(&e.id));
        changed.retain(|id| !gone.contains(id));
        removed.extend(gone);
        deleted += usize::from(any);
    }
    if deleted == 0 {
        return Err(EngineError::State(if kept > 0 { "a point where three or more lines meet can't be deleted; delete a line there instead".into() } else { "no point of a merged drawing there".into() }));
    }
    let here = s.layer;
    s.selection.retain(|l, id| l != here || !removed.contains(&id));
    // Joined runs and fills merge again with what they touch, as after any edit.
    remerge_in_place(s, &changed)?;
    Ok(json!({"deleted": deleted, "kept": kept, "removed": removed.iter().map(|e| e.0).collect::<Vec<_>>(), "selected": s.selection.id_list()}))
}

// ----- smooth and corner -------------------------------------------------------------------------

fn convert_points(s: &mut Session, p: &Value) -> Result<Value> {
    let pts = points(p)?;
    let smooth = opt_bool(p, "smooth")?.ok_or_else(|| EngineError::Param("param 'smooth' is required".into()))?;
    let mut changed: Vec<ElementId> = Vec::new();
    let mut converted = 0usize;
    for at in pts {
        let kf = s.editable_keyframe()?;
        let edges = edges(&near(kf, at), at);
        let edits: Vec<(Seg, Seg)> = if smooth {
            match edges.as_slice() {
                [a, b] => rawgeom::smooth_pair(a, b).map(|(na, nb)| vec![(*a, na), (*b, nb)]).unwrap_or_default(),
                _ => Vec::new(),
            }
        } else {
            edges.iter().filter_map(|e| rawgeom::corner(e).map(|n| (*e, n))).collect()
        };
        converted += usize::from(!edits.is_empty());
        for (old, new) in edits {
            for id in follow_edge(s, None, &old.c, &new.c)? {
                if !changed.contains(&id) {
                    changed.push(id);
                }
            }
        }
    }
    remerge_in_place(s, &changed)?;
    Ok(json!({"converted": converted, "changed": changed.len()}))
}

// ----- snapping while drawing --------------------------------------------------------------------

fn snap_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let at = Vec2::new(f32_req(p, "x")?, f32_req(p, "y")?);
    let radius = opt_f32(p, "snap")?.unwrap_or(0.0).clamp(0.0, 1000.0);
    let moving = opt_bool(p, "moving")?.unwrap_or(false);
    Ok(match snap_drawn(s, at, radius, moving) {
        Some((q, kind)) => json!({"x": q.x, "y": q.y, "snapped": {"x": q.x, "y": q.y, "to": kind.key()}}),
        None => json!({"x": at.x, "y": at.y, "snapped": null}),
    })
}
