//! Merge drawing on the active keyframe (ADR 0031): raw shapes drawn or dropped onto the layer cut
//! and join the raw shapes they touch, through `anicraft_geom::planar`. The result is again raw
//! shapes: one element per fill region and per stroke run, fills under strokes.

use crate::params::*;
use crate::{CommandSpec, Result, Session};
use anicraft_doc::{ColorEffect, Element, ElementId, ElementKind, Paint, Shape, Stroke};
use anicraft_geom::planar::{self, Item, SNAP};
use anicraft_geom::{Affine, Path, PathCmd, Rect, Vec2};
use serde_json::{Value, json};
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c("draw.drop", "Drop", "", None, "{} the selected raw shapes cut and join the raw shapes they touch (after a move)", true, drop as crate::RunFn),
        c("element.select_connected", "Select Connected", "", None, "{x, y, add?, slop?} a raw stroke and the strokes joined to it, or a raw fill and the strokes on and in it", false, select_connected),
    ]
}

/// A raw shape the merge can take: a solid fill (or none) and a stroke of one width (or none).
fn mergeable(e: &Element) -> Option<&Shape> {
    match &e.kind {
        ElementKind::Shape(s) if s.merge && matches!(s.fill, None | Some(Paint::Solid(_))) && s.stroke.as_ref().is_none_or(|st| st.profile.is_none()) => Some(s),
        _ => None,
    }
}

/// Whether a new shape with this style can join the drawing (other paints and width profiles are
/// drawn as objects for now).
pub(crate) fn can_merge(fill: Option<&Paint>, stroke: Option<&Stroke>) -> bool {
    matches!(fill, None | Some(Paint::Solid(_))) && stroke.is_none_or(|st| st.profile.is_none())
}

fn reach(s: &Shape, m: &Affine) -> Rect {
    let pad = s.stroke.as_ref().map_or(0.0, Stroke::reach) * m.mean_scale();
    s.path.bounds().transformed(m).inflate(pad + 2.0 * SNAP)
}

/// Adds `new` (raw shapes, in keyframe space) to the active keyframe, merged with the raw shapes
/// they touch. Returns the ids of the pieces that came from `new` (what to select).
pub(crate) fn merge_in(s: &mut Session, new: Vec<Element>) -> Result<Vec<ElementId>> {
    // Anything that can't merge goes on whole.
    let (new, whole): (Vec<Element>, Vec<Element>) = new.into_iter().partition(|e| mergeable(e).is_some());
    let mut kept: Vec<ElementId> = whole.iter().map(|e| e.id).collect();
    s.editable_keyframe()?.elements.extend(whole.into_iter().map(Arc::new));
    let existing = s.editable_keyframe()?.elements.clone();
    let area = new.iter().filter_map(|e| mergeable(e).map(|sh| reach(sh, &e.transform))).fold(Rect::EMPTY, |r, b| r.union(&b));
    let touching: Vec<usize> = existing
        .iter()
        .enumerate()
        .filter(|(_, e)| mergeable(e).is_some_and(|sh| !reach(sh, &e.transform).intersect(&area).is_empty()))
        .map(|(i, _)| i)
        .collect();
    let all: Vec<&Element> = touching.iter().filter_map(|&i| existing.get(i).map(|e| e.as_ref())).chain(&new).collect();
    // Style tables: equal entries are one style to the merge.
    let mut fills: Vec<(Paint, ColorEffect)> = Vec::new();
    let mut strokes: Vec<(Stroke, ColorEffect)> = Vec::new();
    let mut paths: Vec<Path> = Vec::with_capacity(all.len());
    let mut styles: Vec<(Option<u32>, Option<u32>, anicraft_geom::FillRule)> = Vec::with_capacity(all.len());
    for e in &all {
        let Some(sh) = mergeable(e) else { continue };
        paths.push(sh.path.transformed(&e.transform));
        let fill = sh.fill.clone().map(|f| index_of(&mut fills, (f, e.color)));
        let stroke = sh.stroke.clone().map(|st| index_of(&mut strokes, (Stroke { width: st.width * e.transform.mean_scale(), ..st }, e.color)));
        styles.push((fill, stroke, sh.fill_rule));
    }
    let items: Vec<Item> = paths.iter().zip(&styles).map(|(path, &(fill, stroke, fill_rule))| Item { path, fill, fill_rule, stroke }).collect();
    let Some(merged) = planar::merge(&items) else {
        // Too much to merge at once: keep the new shapes whole, as objects.
        kept.extend(new.iter().map(|e| e.id));
        let kf = s.editable_keyframe()?;
        kf.elements.extend(new.into_iter().map(|mut e| {
            if let ElementKind::Shape(sh) = &mut e.kind {
                sh.merge = false;
            }
            Arc::new(e)
        }));
        return Ok(kept);
    };
    let first_new = touching.len();
    let mut out: Vec<Arc<Element>> = Vec::with_capacity(merged.fills.len() + merged.strokes.len());
    let mut picked = Vec::new();
    for (piece, is_fill) in merged.fills.iter().map(|p| (p, true)).chain(merged.strokes.iter().map(|p| (p, false))) {
        let (fill, stroke, color) = if is_fill {
            let Some((f, c)) = fills.get(piece.style as usize) else { continue };
            (Some(f.clone()), None, *c)
        } else {
            let Some((st, c)) = strokes.get(piece.style as usize) else { continue };
            (None, Some(st.clone()), *c)
        };
        // Local origin at the centre of the piece, as the drawing tools place shapes.
        let c = piece.path.bounds().center();
        let id = ElementId(s.doc.alloc_id());
        if piece.item >= first_new {
            picked.push(id);
        }
        out.push(Arc::new(Element {
            id,
            name: String::new(),
            transform: Affine::translate(c.x, c.y),
            color,
            kind: ElementKind::Shape(Shape { path: Arc::new(piece.path.transformed(&Affine::translate(-c.x, -c.y))), fill, stroke, fill_rule: Default::default(), merge: true }),
        }));
    }
    let kf = s.editable_keyframe()?;
    if kf.elements.len() - touching.len() + out.len() > 100_000 {
        return Err(crate::EngineError::State("too many elements on this keyframe".into()));
    }
    // The drawing goes where the lowest shape it took in was (raw shapes sit under objects).
    let at = touching.first().copied().unwrap_or(kf.elements.len());
    for &i in touching.iter().rev() {
        if i < kf.elements.len() {
            kf.elements.remove(i);
        }
    }
    let at = at.min(kf.elements.len());
    kf.elements.splice(at..at, out);
    kept.extend(picked);
    Ok(kept)
}

fn index_of<T: PartialEq>(table: &mut Vec<T>, v: T) -> u32 {
    match table.iter().position(|x| *x == v) {
        Some(i) => i as u32,
        None => {
            table.push(v);
            (table.len() - 1) as u32
        }
    }
}

/// `draw.drop`: the selected raw shapes go back into the drawing, on top (Animate merges a moved
/// shape when it is let go).
fn drop(s: &mut Session, _: &Value) -> Result<Value> {
    let sel = s.selection.clone();
    let kf = s.editable_keyframe()?;
    let mut moving = Vec::new();
    kf.elements.retain(|e| {
        let take = sel.contains(&e.id) && mergeable(e).is_some();
        if take {
            moving.push((**e).clone());
        }
        !take
    });
    let n = moving.len();
    if n == 0 {
        return Ok(json!({"dropped": 0}));
    }
    let mut keep: Vec<ElementId> = sel.into_iter().filter(|id| !moving.iter().any(|e| e.id == *id)).collect();
    keep.extend(merge_in(s, moving)?);
    s.selection = keep;
    Ok(json!({"dropped": n, "selected": s.selection.iter().map(|e| e.0).collect::<Vec<_>>()}))
}

/// The ends of every open subpath (where raw strokes join), in keyframe space.
fn ends(path: &Path, m: &Affine) -> Vec<Vec2> {
    let mut out = Vec::new();
    let (mut start, mut last, mut open) = (None, Vec2::ZERO, false);
    let flush = |start: Option<Vec2>, last: Vec2, open: bool, out: &mut Vec<Vec2>| {
        if let (Some(a), true) = (start, open) {
            out.push(m.apply(a));
            out.push(m.apply(last));
        }
    };
    for c in &path.cmds {
        match *c {
            PathCmd::MoveTo(p) => {
                flush(start, last, open, &mut out);
                (start, last, open) = (Some(p), p, false);
            }
            PathCmd::LineTo(p) | PathCmd::QuadTo(_, p) | PathCmd::CubicTo(_, _, p) => (last, open) = (p, true),
            PathCmd::Close => open = false,
        }
    }
    flush(start, last, open, &mut out);
    out
}

/// A point about halfway along a stroke, in keyframe space.
fn middle(path: &Path, m: &Affine) -> Option<Vec2> {
    let pl = path.flatten(m, 0.1).into_iter().next()?;
    pl.points.get(pl.points.len() / 2).copied()
}

/// `element.select_connected`: Animate's double-click on raw artwork.
fn select_connected(s: &mut Session, p: &Value) -> Result<Value> {
    let pt = Vec2::new(f32_req(p, "x")?, f32_req(p, "y")?);
    let add = opt_bool(p, "add")?.unwrap_or(false);
    let slop = opt_f32(p, "slop")?.unwrap_or(2.0).clamp(0.0, 100.0);
    let Some((layer, hit)) = s.hit_test(pt, slop) else {
        if !add {
            s.selection.clear();
        }
        return Ok(json!({"selected": []}));
    };
    if layer != s.layer {
        s.layer = layer;
        s.selection.clear();
    }
    let els = s.visible_elements();
    let raw: Vec<(&Element, &Shape)> = els.iter().filter_map(|e| mergeable(e).map(|sh| (e.as_ref(), sh))).collect();
    let mut picked = vec![hit];
    if let Some((e, sh)) = raw.iter().find(|(e, _)| e.id == hit) {
        if sh.fill.is_some() {
            // The fill, and the strokes along its edge or inside it.
            for (o, osh) in raw.iter().filter(|(_, x)| x.fill.is_none()) {
                if middle(&osh.path, &o.transform).is_some_and(|q| sh.path.contains(q, &e.transform, sh.fill_rule) || sh.path.distance_to_edge(q, &e.transform) <= 2.0 * SNAP) {
                    picked.push(o.id);
                }
            }
        } else {
            // The stroke, and every stroke joined to it end to end.
            let mut frontier = ends(&sh.path, &e.transform);
            let mut todo: Vec<(&Element, Vec<Vec2>)> = raw.iter().filter(|(o, x)| x.fill.is_none() && o.id != hit).map(|(o, x)| (*o, ends(&x.path, &o.transform))).collect();
            while let Some(q) = frontier.pop() {
                let mut i = 0;
                while i < todo.len() {
                    if todo[i].1.iter().any(|v| v.distance(q) <= SNAP) {
                        let (o, more) = todo.swap_remove(i);
                        picked.push(o.id);
                        frontier.extend(more);
                    } else {
                        i += 1;
                    }
                }
            }
        }
    }
    if !add {
        s.selection.clear();
    }
    for id in picked {
        if !s.selection.contains(&id) {
            s.selection.push(id);
        }
    }
    Ok(json!({"selected": s.selection.iter().map(|e| e.0).collect::<Vec<_>>()}))
}
