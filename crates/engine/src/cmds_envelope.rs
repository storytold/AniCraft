//! Envelope (ADR 0080): Modify › Transform › Envelope bends the selected shapes and drawing
//! objects by a mesh of handles around them (`anicraft_geom::envelope`, a bicubic Bézier patch).
//!
//! The envelope is open while its shapes are the ones it made: each move bends the shapes as they
//! were when it opened (so moves never compound their approximation), and is one undo step. Like
//! Animate, it is baked: the shapes stay ordinary paths, so there is nothing to delete (no Delete
//! Envelope). Undo and redo go back to the mesh that made the shapes they restore: the session
//! keeps each mesh with the shapes it made, and finds the one whose shapes are on the stage.

use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::{Element, ElementId, ElementKind, LayerId, Shape};
use anicraft_geom::envelope::{self, Mesh};
use anicraft_geom::{Affine, Rect, Vec2};
use serde_json::{Value, json};
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c(
            "modify.envelope",
            "Envelope",
            "Modify/Transform",
            None,
            "{} opens an envelope around the selected shapes and drawing objects on the active layer (ADR 0080): a mesh of 4 × 4 Bézier control points over their bounds, at rest → {points: [[x, y] × 16] (row by row from the top-left), bounds: [x, y, w, h], shapes}. Then modify.envelope_move bends them. Symbols, text, groups and bitmaps can't be bent: break them apart first",
            false,
            open as crate::RunFn,
        ),
        c(
            "modify.envelope_move",
            "Envelope",
            "",
            None,
            "{points: [[x, y] × 16] (the whole mesh) | index: 0..15, x, y (one control point; a corner or side handle moves the inner points with it, as a Coons patch), tolerance? (stage units, default 0.05)} bends the open envelope's shapes from where they were when it opened → {points, shapes}. One undo step",
            true,
            move_points,
        ),
        c(
            "modify.envelope_close",
            "Close Envelope",
            "",
            None,
            "{} closes the open envelope: bent raw shapes merge with the drawing they now touch (as after a move). Nothing to close is no error → {closed}",
            true,
            close,
        ),
    ]
}

/// Most meshes remembered for undo and redo.
const MAX_STATES: usize = 256;
/// Most shapes one envelope bends.
const MAX_SHAPES: usize = 10_000;

/// An open envelope.
#[derive(Clone, Debug)]
pub struct Envelope {
    pub layer: LayerId,
    /// The rest rectangle (keyframe space) and the shapes as they were when it opened.
    pub rest: Rect,
    originals: Vec<Arc<Element>>,
    /// Each mesh used, with the shapes it made (the first is the rest mesh and the originals).
    states: Vec<(Mesh, Vec<Arc<Element>>)>,
    /// The gesture (by the state it started from) whose move made the last of `states`: a
    /// drag's moves are one undo step, so only its last needs remembering.
    live: Option<crate::history::StateId>,
}

impl Envelope {
    #[cfg(test)]
    pub(crate) fn states_len(&self) -> usize {
        self.states.len()
    }

    /// The mesh that made the shapes now on `els`, if they are still what one of its moves made.
    fn current(&self, els: &[Arc<Element>]) -> Option<Mesh> {
        let now: Vec<Option<&Arc<Element>>> = self.originals.iter().map(|o| els.iter().find(|e| e.id == o.id)).collect();
        self.states
            .iter()
            .rev()
            .find(|(_, made)| made.len() == now.len() && made.iter().zip(&now).all(|(m, n)| n.is_some_and(|n| Arc::ptr_eq(m, n) || **m == **n)))
            .map(|(mesh, _)| *mesh)
    }
}

impl Session {
    /// The open envelope's layer, rest rectangle and current mesh (keyframe space), while its
    /// shapes are selected and still the ones it made (an edit to them, or a new selection,
    /// ends it).
    pub fn envelope(&self) -> Option<(LayerId, Rect, Mesh)> {
        let env = self.envelope.as_ref()?;
        if env.layer != self.layer || !env.originals.iter().all(|o| self.selection.contains(env.layer, o.id)) {
            return None;
        }
        let mesh = env.current(&self.elements_on(env.layer))?;
        Some((env.layer, env.rest, mesh))
    }
}

fn mesh_json(m: &Mesh) -> Value {
    json!(m.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>())
}

/// What an envelope can bend: shapes (raw and drawing objects).
fn bendable(e: &Element) -> bool {
    matches!(e.kind, ElementKind::Shape(_))
}

/// The tight bounds of shapes' outlines in keyframe space.
fn outline_bounds(els: &[Arc<Element>]) -> Rect {
    let mut r = Rect::EMPTY;
    for e in els {
        if let ElementKind::Shape(sh) = &e.kind {
            for pl in sh.path.flatten(&e.transform, 0.05) {
                pl.points.iter().for_each(|p| r.include(*p));
            }
        }
    }
    r
}

fn open(s: &mut Session, _: &Value) -> Result<Value> {
    let here = s.layer;
    let sel = s.selection.on(here);
    let els = s.elements_on(here);
    let picked: Vec<Arc<Element>> = els.iter().filter(|e| sel.contains(&e.id)).cloned().collect();
    if picked.is_empty() {
        return Err(EngineError::State("select the shapes or drawing objects to bend with an envelope".into()));
    }
    if !picked.iter().all(|e| bendable(e)) {
        return Err(EngineError::State("an envelope bends shapes and drawing objects only: break symbols, text, groups and bitmaps apart first (Modify › Break Apart)".into()));
    }
    // A warp (ADR 0079) bends the outline as drawn on every keyframe of its layer: baking an
    // envelope into one of them would fight it.
    if picked.iter().any(|e| matches!(&e.kind, ElementKind::Shape(sh) if sh.warp.is_some())) {
        return Err(EngineError::State("an envelope can't bend warped artwork (Asset Warp): remove its warp first (Modify › Asset Warp › Remove Warp)".into()));
    }
    if picked.len() > MAX_SHAPES {
        return Err(EngineError::State(format!("an envelope bends at most {MAX_SHAPES} shapes at once")));
    }
    let mut rest = outline_bounds(&picked);
    // A straight line has no height (or width): give the mesh some room.
    if rest.is_empty() || !rest.min.x.is_finite() {
        return Err(EngineError::State("the selection has no outline to bend".into()));
    }
    let pad = |lo: f32, hi: f32| if hi - lo < 1.0 { ((lo + hi) * 0.5 - 0.5, (lo + hi) * 0.5 + 0.5) } else { (lo, hi) };
    let ((x0, x1), (y0, y1)) = (pad(rest.min.x, rest.max.x), pad(rest.min.y, rest.max.y));
    rest = Rect::new(x0, y0, x1, y1);
    let mesh = envelope::rest_mesh(&rest);
    s.envelope = Some(Envelope { layer: here, rest, originals: picked.clone(), states: vec![(mesh, picked.clone())], live: None });
    Ok(json!({"points": mesh_json(&mesh), "bounds": [rest.min.x, rest.min.y, rest.width(), rest.height()], "shapes": picked.len()}))
}

/// The mesh asked for: the whole of it, or the current one with one point moved.
fn asked(p: &Value, current: &Mesh) -> Result<Mesh> {
    if let Some(list) = p.get("points") {
        let bad = || EngineError::Param("param 'points' must be 16 points [x, y], row by row from the top-left".into());
        let list = list.as_array().filter(|l| l.len() == 16).ok_or_else(bad)?;
        let mut m = [Vec2::ZERO; 16];
        for (slot, v) in m.iter_mut().zip(list) {
            let xy = v.as_array().filter(|a| a.len() == 2).ok_or_else(bad)?;
            let n = |i: usize| xy.get(i).and_then(Value::as_f64).map(|f| f as f32).filter(|f| f.is_finite() && f.abs() <= crate::cmds_draw::MAX_EXTENT);
            *slot = Vec2::new(n(0).ok_or_else(bad)?, n(1).ok_or_else(bad)?);
        }
        return Ok(m);
    }
    let i = opt_u32(p, "index")?.ok_or_else(|| EngineError::Param("give the whole mesh ('points') or one point ('index', 'x', 'y')".into()))? as usize;
    if i >= 16 {
        return Err(EngineError::Param("param 'index' must be 0 to 15".into()));
    }
    let to = Vec2::new(f32_req(p, "x")?, f32_req(p, "y")?);
    if !(to.x.abs() <= crate::cmds_draw::MAX_EXTENT && to.y.abs() <= crate::cmds_draw::MAX_EXTENT) {
        return Err(EngineError::Param("the point is too far away".into()));
    }
    let mut m = *current;
    if let Some(slot) = m.get_mut(i) {
        *slot = to;
    }
    // A boundary handle bends the inside with it; an inner point moves alone.
    if envelope::BOUNDARY.contains(&i) {
        envelope::coons_interior(&mut m);
    }
    Ok(m)
}

/// `shape` (placed by `m`) bent in keyframe space, as an element placed at its centre.
fn bent(e: &Element, sh: &Shape, mesh: &Mesh, rest: &Rect, tol: f32) -> Option<Element> {
    let path = envelope::warp(&sh.path.transformed(&e.transform), mesh, rest, tol)?;
    let c = path.bounds().center();
    let to_local = Affine::translate(-c.x, -c.y);
    let k = e.transform.mean_scale();
    let shape = Shape {
        path: Arc::new(path.transformed(&to_local)),
        // Gradients and bitmaps stay where they were on the stage (they don't bend, as in Animate).
        fill: sh.fill.as_ref().map(|f| f.transformed(&to_local.mul(&e.transform))),
        stroke: sh.stroke.as_ref().map(|st| st.scaled(k)),
        ..sh.clone()
    };
    Some(Element { id: e.id, name: e.name.clone(), transform: Affine::translate(c.x, c.y), color: e.color, kind: ElementKind::Shape(shape) })
}

fn move_points(s: &mut Session, p: &Value) -> Result<Value> {
    let none = || EngineError::State("no envelope is open: select shapes and choose Modify › Transform › Envelope".into());
    let (_, rest, current) = s.envelope().ok_or_else(none)?;
    let mesh = asked(p, &current)?;
    let tol = match opt_f32(p, "tolerance")? {
        None => 0.05,
        Some(t) if (0.001..=100.0).contains(&t) => t,
        Some(_) => return Err(EngineError::Param("param 'tolerance' must be between 0.001 and 100".into())),
    };
    let originals = s.envelope.as_ref().map(|e| e.originals.clone()).unwrap_or_default();
    let made: Vec<Arc<Element>> = originals
        .iter()
        .map(|o| match &o.kind {
            ElementKind::Shape(sh) => bent(o, sh, &mesh, &rest, tol).map(Arc::new).ok_or_else(|| EngineError::State("the envelope bends those shapes too far to draw; move the handles back a little".into())),
            _ => Ok(o.clone()),
        })
        .collect::<Result<_>>()?;
    let kf = s.editable_keyframe()?;
    for m in &made {
        if let Some(slot) = kf.elements.iter_mut().find(|e| e.id == m.id) {
            *slot = m.clone();
        }
    }
    let gesture = s.gesture.as_ref().map(|g| g.state);
    if let Some(env) = s.envelope.as_mut() {
        // Another move of the same drag replaces the one before it.
        if gesture.is_some() && env.live == gesture && env.states.len() > 1 {
            env.states.pop();
        }
        env.live = gesture;
        if env.states.len() >= MAX_STATES {
            // Keep the rest state; forget the oldest move.
            env.states.remove(1);
        }
        env.states.push((mesh, made.clone()));
    }
    Ok(json!({"points": mesh_json(&mesh), "shapes": made.len()}))
}

fn close(s: &mut Session, _: &Value) -> Result<Value> {
    let Some(env) = s.envelope.take() else { return Ok(json!({"closed": false})) };
    let ids: Vec<ElementId> = env.originals.iter().map(|e| e.id).collect();
    // Bent raw shapes merge where they now lie, as after a move (ADR 0031).
    let here = s.elements_on(env.layer);
    let raw = here.iter().any(|e| ids.contains(&e.id) && crate::merge::mergeable(e).is_some());
    if raw && env.layer == s.layer {
        let keep = s.selection.clone();
        s.selection = crate::Selection::only(env.layer, ids.iter().copied());
        crate::merge::drop_selected(s)?;
        // What the merge selected, else what was selected.
        if s.selection.is_empty() {
            s.selection = keep;
        }
    }
    Ok(json!({"closed": true}))
}
