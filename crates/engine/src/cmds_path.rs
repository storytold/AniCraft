//! Path drawing (Pen, Pencil) and point editing (Subselection).
//!
//! Points are addressed as `(cmd, slot)` on the shape's path in its local space: slot 0 is the
//! anchor a command ends on, 1 and 2 its control points (`anicraft_geom::edit`). A closed
//! subpath's start is one anchor even though the path stores it twice; commands accept either
//! copy. Deltas and positions in params are in stage space.

use crate::cmds_draw::{MAX_EXTENT, add_shape, object_param, style};
use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::{Element, ElementId, ElementKind, Shape};
use anicraft_geom::edit::{ANCHOR, C1, C2, polyline_path, simplify};
use anicraft_geom::{Affine, Path, PathCmd, Vec2, fit_curve};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c("draw.path", "Pen Path", "", None, "{d: SVG path data (stage space), fill?, stroke?, stroke_width?, object?}", true, draw_path as crate::RunFn),
        c(
            "draw.freehand",
            "Pencil Stroke",
            "",
            None,
            "{points: [[x, y], ..], mode?: \"smooth\"|\"straighten\"|\"ink\", tolerance? (stage units), closed?, stroke?, stroke_width?, object?}",
            true,
            draw_freehand,
        ),
        c("path.move_points", "Move Points", "", None, "{id, points: [[cmd, slot], ..], dx, dy, smooth?: bool} (a handle moved alone at a smooth anchor turns the other one with it, keeping its length, unless smooth is false: Alt breaks the pair)", true, move_points),
        c("path.add_point", "Add Anchor Point", "", None, "{id, x, y, slop?}", true, add_point),
        c("path.delete_points", "Delete Anchor Points", "", None, "{id, cmds: [..]}", true, delete_points),
        c("path.convert_point", "Convert Anchor Point", "", None, "{id, cmd, smooth: bool}", true, convert_point),
    ]
}

/// Most commands one path may have (keeps one edit's cost bounded).
const MAX_CMDS: usize = 100_000;

fn check_path(path: &Path) -> Result<()> {
    if path.cmds.len() > MAX_CMDS {
        return Err(EngineError::Param(format!("path has more than {MAX_CMDS} commands")));
    }
    if path.anchors().len() < 2 {
        return Err(EngineError::Param("a path needs at least two points".into()));
    }
    let b = path.bounds();
    if !(b.min.is_finite() && b.max.is_finite()) || b.width() > MAX_EXTENT || b.height() > MAX_EXTENT {
        return Err(EngineError::Param("path is too large".into()));
    }
    if b.width().max(b.height()) < 0.5 {
        return Err(EngineError::Param("path is too small".into()));
    }
    Ok(())
}

/// Adds a stage-space path as a shape whose local origin is its bounds centre.
fn add_path(s: &mut Session, kind: &str, mut path: Path, p: &Value) -> Result<Value> {
    check_path(&path)?;
    path.close_explicitly();
    let closed = path.cmds.iter().any(|c| matches!(c, PathCmd::Close));
    let (fill, stroke) = style(s, p, closed)?;
    let c = path.bounds().center();
    let local = path.transformed(&Affine::translate(-c.x, -c.y));
    add_shape(s, kind, local, c, fill, stroke, object_param(s, p)?)
}

fn draw_path(s: &mut Session, p: &Value) -> Result<Value> {
    let d = str_req(p, "d")?;
    if d.len() > 4_000_000 {
        return Err(EngineError::Param("param 'd' is too long".into()));
    }
    // Full SVG path data: relative commands, H/V, S/T shorthands and arcs too.
    let path = anicraft_geom::svg::parse_path_data(d).map_err(|e| EngineError::Param(format!("param 'd': {e}")))?;
    add_path(s, "path", path, p)
}

fn points_param(p: &Value, key: &str) -> Result<Vec<Vec2>> {
    let bad = || EngineError::Param(format!("param '{key}' must be a list of [x, y] pairs"));
    let list = p.get(key).and_then(Value::as_array).ok_or_else(bad)?;
    if list.len() > MAX_CMDS {
        return Err(EngineError::Param(format!("param '{key}' has more than {MAX_CMDS} points")));
    }
    list.iter()
        .map(|v| {
            let xy = v.as_array().filter(|a| a.len() == 2).ok_or_else(bad)?;
            let n = |i: usize| xy.get(i).and_then(Value::as_f64).map(|f| f as f32).filter(|f| f.is_finite()).ok_or_else(bad);
            Ok(Vec2::new(n(0)?, n(1)?))
        })
        .collect()
}

/// Pencil: "smooth" fits curves, "straighten" keeps the stroke's main straight runs, "ink"
/// follows the hand closely with short straight segments.
fn draw_freehand(s: &mut Session, p: &Value) -> Result<Value> {
    let pts = points_param(p, "points")?;
    let tol = opt_f32(p, "tolerance")?.unwrap_or(1.5);
    if !(0.01..=1000.0).contains(&tol) {
        return Err(EngineError::Param("param 'tolerance' must be between 0.01 and 1000".into()));
    }
    let closed = opt_bool(p, "closed")?.unwrap_or(false);
    let path = match opt_str(p, "mode")?.unwrap_or("smooth") {
        "smooth" => fit_curve(&pts, tol, tol * 2.0, closed),
        "straighten" => polyline_path(&simplify(&pts, tol * 4.0), closed),
        "ink" => polyline_path(&simplify(&pts, tol * 0.35), closed),
        other => return Err(EngineError::Param(format!("unknown pencil mode '{other}' (smooth, straighten or ink)"))),
    };
    // A pencil stroke is a line: no fill unless it was closed and a fill was asked for.
    let mut style_params = p.clone();
    if !(closed && p.get("fill").is_some()) {
        style_params["fill"] = Value::Null;
    }
    add_path(s, "pencil", path, &style_params)
}

// ----- point editing ----------------------------------------------------------------------

/// Runs `edit` on shape element `id` (param `id`) in the editable keyframe (copy-on-write), with
/// its transform. The shape of a shape armature is reshaped at rest instead, keeping its binding
/// (ADR 0090).
fn edit_shape<T>(s: &mut Session, p: &Value, edit: impl FnOnce(&mut Shape, Affine) -> Result<T>) -> Result<T> {
    let id = ElementId(opt_u64(p, "id")?.ok_or_else(|| EngineError::Param("param 'id' is required".into()))?);
    if !s.visible_elements().iter().any(|e| e.id == id) {
        return Err(EngineError::State(format!("element {} is not on the active layer at this frame", id.0)));
    }
    if crate::cmds_bone::is_skin_shape(s, id) {
        return crate::cmds_bone::reshape(s, id, edit);
    }
    let kf = s.editable_keyframe()?;
    let el: &mut Element = kf.elements_mut_where(|e| e == id).into_iter().next().ok_or_else(|| EngineError::State(format!("no element {}", id.0)))?;
    let m = el.transform;
    match &mut el.kind {
        ElementKind::Shape(sh) => edit(sh, m),
        ElementKind::Symbol { .. } => Err(EngineError::State("symbol instances have no points to edit (double-click to edit the symbol)".into())),
        ElementKind::Text(_) => Err(EngineError::State("text has no points to edit; break it apart (Ctrl+B) to edit its outlines".into())),
        ElementKind::Camera => Err(EngineError::State("the camera has no points to edit".into())),
        ElementKind::Group { .. } => Err(EngineError::State("a group has no points of its own (double-click it to edit what is inside)".into())),
    }
}

fn index_list(p: &Value, key: &str) -> Result<Vec<usize>> {
    let v = opt_u64_list(p, key)?.ok_or_else(|| EngineError::Param(format!("param '{key}' is required")))?;
    Ok(v.into_iter().map(|i| usize::try_from(i).unwrap_or(usize::MAX)).collect())
}

fn move_points(s: &mut Session, p: &Value) -> Result<Value> {
    let d = Vec2::new(f32_req(p, "dx")?, f32_req(p, "dy")?);
    let bad = || EngineError::Param("param 'points' must be a list of [cmd, slot] pairs".into());
    let list = p.get("points").and_then(Value::as_array).ok_or_else(bad)?;
    let mut wanted = Vec::with_capacity(list.len().min(MAX_CMDS));
    for v in list.iter().take(MAX_CMDS) {
        let pair = v.as_array().filter(|a| a.len() == 2).ok_or_else(bad)?;
        let cmd = pair.first().and_then(Value::as_u64).ok_or_else(bad)?;
        let slot = pair.get(1).and_then(Value::as_u64).filter(|s| *s <= 2).ok_or_else(bad)?;
        wanted.push((usize::try_from(cmd).unwrap_or(usize::MAX), slot as u8));
    }
    let smooth = opt_bool(p, "smooth")?.unwrap_or(true);
    edit_shape(s, p, |sh, m| {
        let inv = m.inverse().ok_or_else(|| EngineError::State("the shape is scaled to zero".into()))?;
        let local = inv.apply_vector(d);
        if !local.is_finite() {
            return Err(EngineError::Param("the move is too large".into()));
        }
        let path = Arc::make_mut(&mut sh.path);
        // Anchors bring their handles and their twin at a closed start; each point moves once.
        let mut set = BTreeSet::new();
        for (cmd, slot) in wanted {
            if path.point(cmd, slot).is_none() {
                return Err(EngineError::Param(format!("no point ({cmd}, {slot}) on this path")));
            }
            if slot == ANCHOR {
                set.extend(path.anchor_group(cmd));
            } else {
                set.insert((cmd, slot));
            }
        }
        // The other handle of a smooth anchor whose handle moves alone (ADR 0045), as it was.
        let partners: Vec<(usize, u8, Vec2, Vec2)> = if smooth { set.iter().filter_map(|&(cmd, slot)| partner(path, &set, cmd, slot)).collect() } else { Vec::new() };
        for &(cmd, slot) in &set {
            if let Some(q) = path.point(cmd, slot) {
                path.set_point(cmd, slot, q + local);
            }
        }
        for (cmd, slot, anchor, handle) in partners {
            if let Some(k) = path.point(cmd, slot) {
                path.set_point(cmd, slot, anicraft_geom::raw::turned_opposite(anchor, handle + local, k));
            }
        }
        check_path(path)?;
        Ok(json!({"moved": set.len()}))
    })
}

/// For handle `(cmd, slot)` moving alone: the other handle of its anchor when the anchor is smooth
/// and that handle stays (`(cmd, slot)`), with the anchor and this handle where they are now.
fn partner(path: &Path, moving: &BTreeSet<(usize, u8)>, cmd: usize, slot: u8) -> Option<(usize, u8, Vec2, Vec2)> {
    // Slot 2 is the in-handle of anchor `cmd`; slot 1 the out-handle of the anchor before.
    let anchor = match slot {
        C2 => path.canonical_anchor(cmd),
        C1 => path.canonical_anchor(cmd.checked_sub(1)?),
        _ => return None,
    };
    let (i, o) = path.anchor_segments(anchor);
    let other = if slot == C2 { (o?, C1) } else { (i?, C2) };
    if moving.contains(&other) || path.anchor_group(anchor).iter().any(|g| g.1 == ANCHOR && moving.contains(g)) {
        return None;
    }
    let (a, h, k) = (path.point(anchor, ANCHOR)?, path.point(cmd, slot)?, path.point(other.0, other.1)?);
    // A quadratic's control is both its ends' handle: turning it would bend the other end.
    let cubic = |c: usize| matches!(path.cmds.get(c), Some(PathCmd::CubicTo(..)));
    (cubic(other.0) && anicraft_geom::raw::smooth_at(a, h, k)).then_some((other.0, other.1, a, h))
}

fn add_point(s: &mut Session, p: &Value) -> Result<Value> {
    let at = Vec2::new(f32_req(p, "x")?, f32_req(p, "y")?);
    let slop = opt_f32(p, "slop")?.unwrap_or(4.0).clamp(0.0, 1000.0);
    edit_shape(s, p, |sh, m| {
        let path = Arc::make_mut(&mut sh.path);
        path.close_explicitly();
        let (cmd, t, dist) = path.nearest_segment(at, &m).ok_or_else(|| EngineError::State("the path has no segments".into()))?;
        if dist > slop {
            return Err(EngineError::State("no segment there".into()));
        }
        if path.cmds.len() >= MAX_CMDS || !path.split_segment(cmd, t) {
            return Err(EngineError::State("cannot add a point there".into()));
        }
        Ok(json!({"cmd": cmd}))
    })
}

fn delete_points(s: &mut Session, p: &Value) -> Result<Value> {
    let cmds = index_list(p, "cmds")?;
    edit_shape(s, p, |sh, _| {
        let path = Arc::make_mut(&mut sh.path);
        // Highest first, so earlier indices stay valid; twins collapse onto one anchor.
        let order: BTreeSet<usize> = cmds.iter().map(|&c| path.canonical_anchor(c)).collect();
        let mut removed = 0;
        for cmd in order.into_iter().rev() {
            if path.remove_anchor(cmd) {
                removed += 1;
            }
        }
        if removed == 0 {
            return Err(EngineError::State("a path keeps at least two points (delete the shape instead)".into()));
        }
        Ok(json!({"deleted": removed}))
    })
}

fn convert_point(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = opt_u64(p, "cmd")?.ok_or_else(|| EngineError::Param("param 'cmd' is required".into()))?;
    let cmd = usize::try_from(cmd).unwrap_or(usize::MAX);
    let smooth = opt_bool(p, "smooth")?.ok_or_else(|| EngineError::Param("param 'smooth' is required".into()))?;
    edit_shape(s, p, |sh, _| {
        let path = Arc::make_mut(&mut sh.path);
        if path.point(cmd, ANCHOR).is_none() {
            return Err(EngineError::Param(format!("no anchor {cmd} on this path")));
        }
        let changed = if smooth { path.make_smooth(cmd) } else { path.make_corner(cmd) };
        Ok(json!({"changed": changed}))
    })
}
