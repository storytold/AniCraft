//! The Eraser's commands (ADR 0034). The eraser sweeps the Brush's tip (ADR 0033) and takes
//! the area it covers out of the raw shapes on the active layer (ADR 0031): the swept outline
//! goes into the planar merge as a fill whose regions come out empty, placed by the erase mode.
//! Drawing objects, symbols and text are left alone, as in Animate.

use crate::cmds_brush::{bounded_samples, ranged, tip};
use crate::merge::{self, Part, mergeable, reach};
use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::{Element, ElementId, ElementKind, LayerKind};
use anicraft_geom::brush::outline;
use anicraft_geom::planar::SNAP;
use anicraft_geom::{Affine, Path, PathCmd, Vec2};
use serde_json::{Value, json};

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c("draw.erase", "Eraser", "", None, "{points: [[x, y] or [x, y, pressure], ..] (one point erases a dab), size? (stage units, default 10), shape?: round|square, angle?, roundness?, mode?: normal|fills|lines|selected_fills|inside (Erase Normal takes fills and lines, Erase Fills leaves lines, Erase Lines leaves fills, Erase Selected Fills only the selected fills, Erase Inside only the fill the stroke starts on), unit? (stage units per screen point)} erases from the raw shapes on the active layer; drawing objects, symbols and text are kept", true, erase as crate::RunFn),
        c("draw.faucet", "Faucet", "", None, "{x, y, slop?} deletes the whole raw fill or stroke under the point on the active layer", true, faucet),
        c("draw.erase_all", "Erase Everything", "", None, "{} clears every visible, unlocked layer at the playhead (asks first; undoable)", true, erase_all),
    ]
}

/// An erase mode (Animate's names).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EraseMode {
    #[default]
    Normal,
    Fills,
    Lines,
    SelectedFills,
    Inside,
}

impl EraseMode {
    pub const ALL: [EraseMode; 5] = [EraseMode::Normal, EraseMode::Fills, EraseMode::Lines, EraseMode::SelectedFills, EraseMode::Inside];

    pub fn key(self) -> &'static str {
        match self {
            EraseMode::Normal => "normal",
            EraseMode::Fills => "fills",
            EraseMode::Lines => "lines",
            EraseMode::SelectedFills => "selected_fills",
            EraseMode::Inside => "inside",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            EraseMode::Normal => "Erase Normal",
            EraseMode::Fills => "Erase Fills",
            EraseMode::Lines => "Erase Lines",
            EraseMode::SelectedFills => "Erase Selected Fills",
            EraseMode::Inside => "Erase Inside",
        }
    }
    pub fn hint(self) -> &'static str {
        match self {
            EraseMode::Normal => "Erases lines and fills on the layer.",
            EraseMode::Fills => "Erases fills only, leaving lines alone.",
            EraseMode::Lines => "Erases lines only, leaving fills alone.",
            EraseMode::SelectedFills => "Erases only the selected fills.",
            EraseMode::Inside => "Erases only inside the fill where the stroke starts.",
        }
    }
    pub fn from_key(k: &str) -> Option<EraseMode> {
        EraseMode::ALL.into_iter().find(|m| m.key() == k)
    }
}

fn erase(s: &mut Session, p: &Value) -> Result<Value> {
    let pts = bounded_samples(p)?;
    let tip = tip(p)?;
    let unit = ranged(p, "unit", 0.001..=1000.0, 1.0)?;
    let mode_key = opt_str(p, "mode")?.unwrap_or("normal");
    let mode = EraseMode::from_key(mode_key).ok_or_else(|| EngineError::Param(format!("unknown erase mode '{mode_key}' (normal, fills, lines, selected_fills or inside)")))?;
    let start = pts.first().map_or(Vec2::ZERO, |x| x.at);
    // The eraser follows the hand (a third of a screen point), fitted as finely as the brush.
    let path = outline(&pts, &tip, unit * 0.3, (unit * 0.35).min(tip.size * 0.01));
    if !path.cmds.iter().any(|c| matches!(c, PathCmd::CubicTo(..) | PathCmd::LineTo(..))) {
        return Err(EngineError::Param("the stroke is too small to erase with".into()));
    }
    erase_with(s, &path, mode, start)
}

/// Erases `eraser` (keyframe space) from the active layer's raw shapes, as `mode` says.
pub(crate) fn erase_with(s: &mut Session, eraser: &Path, mode: EraseMode, start: Vec2) -> Result<Value> {
    // Nothing may change when nothing is erased (no keyframe made, no undo step).
    let before = s.doc.clone();
    let els = s.editable_keyframe()?.elements.clone();
    let area = eraser.bounds().inflate(2.0 * SNAP);
    // What the eraser passes over and keeps (ADR 0093): symbols, and drawing objects, groups and
    // text. Each opens for editing in place, where its raw shapes erase.
    let kept = kept_under(s, &els, eraser);
    let nothing = |why: &str| {
        let why = match kept {
            (0, 0) => why.to_string(),
            (_, 0) => "the eraser keeps symbols on the stage; double-click one with the Selection tool (or select it and use Edit Symbols) to erase inside it".to_string(),
            (0, _) => "the eraser keeps drawing objects; double-click one with the Selection tool to erase inside it".to_string(),
            _ => "the eraser keeps symbols and drawing objects; double-click one with the Selection tool to erase inside it".to_string(),
        };
        Ok(json!({"erased": false, "mode": mode.key(), "elements": [], "reason": why, "kept": {"symbols": kept.0, "objects": kept.1}}))
    };
    let touching: Vec<usize> = (0..els.len()).filter(|&i| els.get(i).and_then(|e| mergeable(e).map(|sh| !reach(sh, &e.transform).intersect(&area).is_empty())).unwrap_or(false)).collect();
    let el = |i: &usize| els.get(*i).map(|e| e.as_ref());
    let is_fill = |i: &usize| el(i).and_then(mergeable).is_some_and(|sh| sh.fill.is_some());
    let fills: Vec<usize> = touching.iter().copied().filter(is_fill).collect();
    let lines: Vec<usize> = touching.iter().copied().filter(|i| !is_fill(i)).collect();
    // What the eraser takes from (it sits right above them), and what goes into the merge.
    let (targets, members): (Vec<usize>, Vec<usize>) = match mode {
        EraseMode::Normal => (touching.clone(), touching.clone()),
        EraseMode::Fills => (fills.clone(), touching.clone()),
        // Only the lines take part, so the fills stay exactly as they are.
        EraseMode::Lines => (lines.clone(), lines.clone()),
        EraseMode::SelectedFills => (fills.iter().copied().filter(|i| el(i).is_some_and(|e| s.selection.contains(s.layer, e.id))).collect(), touching.clone()),
        EraseMode::Inside => {
            // The topmost raw fill under the start of the stroke.
            let at = fills.iter().rev().copied().find(|i| el(i).is_some_and(|e| mergeable(e).is_some_and(|sh| sh.path.contains(start, &e.transform, sh.fill_rule))));
            (at.into_iter().collect(), touching.clone())
        }
    };
    if targets.is_empty() {
        s.doc = before;
        return nothing(match mode {
            EraseMode::SelectedFills if s.selection.on(s.layer).is_empty() => "select the fills to erase first",
            EraseMode::Inside => "start the stroke on a fill to erase inside it",
            _ => "nothing raw to erase there",
        });
    }
    // Bottom to top: the targets, the eraser, then the rest (fills before lines, so lines still
    // cut the fills they cross and nothing hides them).
    let rest: Vec<usize> = members.iter().copied().filter(|i| !targets.contains(i)).collect();
    let (rest_fills, rest_lines): (Vec<usize>, Vec<usize>) = rest.into_iter().partition(is_fill);
    let here = s.layer;
    let selected = s.selection.clone();
    let shape = |i: &usize| el(i).map(|e| Part::Shape(e, selected.contains(here, e.id)));
    let parts: Vec<Part> = targets.iter().filter_map(shape).chain(std::iter::once(Part::Eraser(eraser))).chain(rest_fills.iter().filter_map(shape)).chain(rest_lines.iter().filter_map(shape)).collect();
    let Some((out, picked)) = merge::remerge(s, &parts) else {
        s.doc = before;
        return Err(EngineError::State("there is too much artwork under the eraser to erase at once; erase a smaller area".into()));
    };
    // Erased nothing (the eraser only came near): leave the drawing as it was.
    let was = measure(members.iter().filter_map(el));
    let now = measure(out.iter().map(|e| e.as_ref()));
    let close = |a: f64, b: f64| (a - b).abs() <= 1e-3 + 1e-7 * a.abs().max(b.abs());
    if close(was.0, now.0) && close(was.1, now.1) {
        s.doc = before;
        return nothing("nothing raw to erase there");
    }
    let ids: Vec<u64> = out.iter().map(|e| e.id.0).collect();
    let gone: Vec<ElementId> = members.iter().filter_map(el).map(|e| e.id).collect();
    merge::replace(s, &members, out, false)?;
    // What stays selected: the rest of the selection, and what is left of selected shapes.
    s.selection.retain(|l, id| l != here || !gone.contains(&id));
    s.selection.extend_on(here, picked);
    Ok(json!({"erased": true, "mode": mode.key(), "elements": ids, "area_erased": (was.0 - now.0).max(0.0), "length_erased": (was.1 - now.1).max(0.0), "kept": {"symbols": kept.0, "objects": kept.1}}))
}

/// How many symbol instances, and how many other elements that are not raw shapes (drawing
/// objects, groups, text), the eraser's outline reaches on the keyframe: what it keeps.
fn kept_under(s: &Session, els: &[std::sync::Arc<Element>], eraser: &Path) -> (usize, usize) {
    let area = eraser.bounds();
    let (mut symbols, mut objects) = (0, 0);
    for e in els.iter().take(100_000) {
        if mergeable(e).is_some() || matches!(e.kind, ElementKind::Camera) {
            continue;
        }
        let b = match &e.kind {
            ElementKind::Shape(sh) => reach(sh, &e.transform),
            _ => s.element_bounds(e.id),
        };
        if b.intersect(&area).is_empty() {
            continue;
        }
        if matches!(e.kind, ElementKind::Symbol { .. }) {
            symbols += 1;
        } else {
            objects += 1;
        }
    }
    (symbols, objects)
}

/// Total fill area and stroke length of raw shapes, in keyframe space. Fill areas are exact
/// (Green's theorem with Gauss–Legendre nodes, exact for cubics); lengths are by Gauss–Legendre
/// too, so a shape that comes back from the merge unchanged measures exactly the same.
fn measure<'a>(els: impl Iterator<Item = &'a Element>) -> (f64, f64) {
    let (mut area, mut length) = (0.0, 0.0);
    for e in els {
        let Some(sh) = mergeable(e) else { continue };
        if sh.fill.is_some() {
            area += signed_area(&sh.path, &e.transform).abs();
        }
        if sh.stroke.is_some() {
            length += path_length(&sh.path, &e.transform);
        }
    }
    (area, length)
}

/// The path's segments as cubics, in the space `m` maps to (closing segments included).
fn cubics(path: &Path, m: &Affine) -> Vec<[Vec2; 4]> {
    let mut out = Vec::new();
    let (mut start, mut cur) = (Vec2::ZERO, Vec2::ZERO);
    let line = |a: Vec2, b: Vec2| [a, a.lerp(b, 1.0 / 3.0), a.lerp(b, 2.0 / 3.0), b];
    for c in &path.cmds {
        match *c {
            PathCmd::MoveTo(p) => {
                start = m.apply(p);
                cur = start;
            }
            PathCmd::LineTo(p) => {
                let p = m.apply(p);
                out.push(line(cur, p));
                cur = p;
            }
            PathCmd::QuadTo(q, p) => {
                let (q, p) = (m.apply(q), m.apply(p));
                out.push([cur, cur.lerp(q, 2.0 / 3.0), p.lerp(q, 2.0 / 3.0), p]);
                cur = p;
            }
            PathCmd::CubicTo(a, b, p) => {
                let (a, b, p) = (m.apply(a), m.apply(b), m.apply(p));
                out.push([cur, a, b, p]);
                cur = p;
            }
            PathCmd::Close => {
                if cur != start {
                    out.push(line(cur, start));
                }
                cur = start;
            }
        }
    }
    out
}

/// A cubic's point and derivative at `t`, in f64.
fn point_and_tangent(c: &[Vec2; 4], t: f64) -> ((f64, f64), (f64, f64)) {
    let p = |v: Vec2| (f64::from(v.x), f64::from(v.y));
    let [p0, p1, p2, p3] = c.map(p);
    let u = 1.0 - t;
    let (b0, b1, b2, b3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    let at = (b0 * p0.0 + b1 * p1.0 + b2 * p2.0 + b3 * p3.0, b0 * p0.1 + b1 * p1.1 + b2 * p2.1 + b3 * p3.1);
    let (d0, d1, d2) = (3.0 * u * u, 6.0 * u * t, 3.0 * t * t);
    let d = (d0 * (p1.0 - p0.0) + d1 * (p2.0 - p1.0) + d2 * (p3.0 - p2.0), d0 * (p1.1 - p0.1) + d1 * (p2.1 - p1.1) + d2 * (p3.1 - p2.1));
    (at, d)
}

/// Five-point Gauss–Legendre nodes and weights on 0..1.
const GAUSS: [(f64, f64); 5] = [
    (0.046_910_077_030_668, 0.118_463_442_528_094_5),
    (0.230_765_344_947_158_45, 0.239_314_335_249_683_2),
    (0.5, 0.284_444_444_444_444_4),
    (0.769_234_655_052_841_6, 0.239_314_335_249_683_2),
    (0.953_089_922_969_332, 0.118_463_442_528_094_5),
];

fn signed_area(path: &Path, m: &Affine) -> f64 {
    // ½ ∮ (x dy − y dx): a degree-5 polynomial per cubic, which these nodes integrate exactly.
    cubics(path, m).iter().map(|c| GAUSS.iter().map(|&(t, w)| {
        let ((x, y), (dx, dy)) = point_and_tangent(c, t);
        w * (x * dy - y * dx)
    }).sum::<f64>()).sum::<f64>() * 0.5
}

fn path_length(path: &Path, m: &Affine) -> f64 {
    cubics(path, m).iter().map(|c| GAUSS.iter().map(|&(t, w)| {
        let (_, (dx, dy)) = point_and_tangent(c, t);
        w * (dx * dx + dy * dy).sqrt()
    }).sum::<f64>()).sum()
}

/// `draw.faucet`: Animate's Faucet deletes a whole fill or stroke with one click.
fn faucet(s: &mut Session, p: &Value) -> Result<Value> {
    let at = Vec2::new(f32_req(p, "x")?, f32_req(p, "y")?);
    let slop = opt_f32(p, "slop")?.unwrap_or(2.0).clamp(0.0, 100.0);
    let els = s.visible_elements();
    // Topmost shape on the active layer under the point: in a fill, or on a stroke.
    let hit = els.iter().rev().find(|e| match &e.kind {
        ElementKind::Shape(sh) => {
            let hw = sh.stroke.as_ref().map_or(0.0, |st| st.reach() * e.transform.mean_scale());
            let near = sh.path.bounds().transformed(&e.transform).inflate(hw + slop).contains(at);
            near && ((sh.fill.is_some() && sh.path.contains(at, &e.transform, sh.fill_rule)) || (sh.stroke.is_some() && sh.path.distance_to_edge(at, &e.transform) <= hw + slop))
        }
        ElementKind::Symbol { .. } | ElementKind::Text(_) | ElementKind::Group { .. } => s.element_bounds(e.id).contains(at),
        ElementKind::Camera => false,
    });
    let Some(hit) = hit else {
        return Err(EngineError::State("click a fill or a line on this layer to delete it".into()));
    };
    if mergeable(hit).is_none() {
        return Err(EngineError::State("the faucet deletes raw fills and lines; drawing objects, symbols and text are kept (select one and press Delete)".into()));
    }
    let (id, line) = (hit.id, matches!(&hit.kind, ElementKind::Shape(sh) if sh.fill.is_none()));
    let kf = s.editable_keyframe()?;
    kf.elements.retain(|e| e.id != id);
    let here = s.layer;
    s.selection.remove(here, id);
    Ok(json!({"deleted": id.0, "kind": if line { "line" } else { "fill" }}))
}

/// The layers `draw.erase_all` clears: visible, unlocked layers that hold artwork (not folders,
/// not the camera), with the number of elements at the playhead on each.
pub(crate) fn erasable_layers(s: &Session) -> Vec<(anicraft_doc::LayerId, usize)> {
    let tl = s.timeline();
    tl.layers
        .iter()
        .filter(|l| l.visible && !l.locked && !tl.locked_by_folder(l.id) && !matches!(l.kind, LayerKind::Folder | LayerKind::Camera))
        .filter_map(|l| l.keyframe_at(s.frame).map(|k| (l.id, k.elements.len())))
        .filter(|(_, n)| *n > 0)
        .collect()
}

/// `draw.erase_all`: double-clicking the Eraser clears the stage at this frame: every element on
/// the visible, unlocked layers' keyframes at the playhead (raw shapes, objects, symbols, text).
fn erase_all(s: &mut Session, _: &Value) -> Result<Value> {
    let layers = erasable_layers(s);
    let (scene, frame) = (s.scene, s.frame);
    let mut erased = 0;
    for (id, _) in &layers {
        let Some(layer) = crate::editing::edited_mut(&mut s.doc, scene, &s.editing).ok().and_then(|tl| tl.layer_mut(*id)) else { continue };
        let Some(i) = layer.keyframe_index_at(frame) else { continue };
        let Some(kf) = layer.keyframe_mut(i) else { continue };
        erased += kf.elements.len();
        kf.elements.clear();
        // Nothing is left to tween.
        kf.tween = None;
        kf.motion = None;
    }
    s.selection.clear();
    Ok(json!({"erased": erased, "layers": layers.len()}))
}

#[cfg(test)]
pub(crate) fn measure_els(els: &[std::sync::Arc<Element>]) -> (f64, f64) {
    measure(els.iter().map(|e| e.as_ref()))
}
