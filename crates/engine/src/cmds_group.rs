//! Groups and Combine Objects (ADR 0073): Modify › Group (Ctrl+G) and Ungroup (Ctrl+Shift+G), and
//! Modify › Combine Objects › Union, Intersect, Punch and Crop on drawing objects.
//!
//! A group is an element holding other elements without a Library symbol
//! ([`ElementKind::Group`]): its transform and colour effect apply to its members, raw shapes
//! inside merge only with each other, and double-clicking it opens it in place like a symbol
//! (`symbol.edit`, ADR 0045). Combine Objects is exact boolean geometry on the planar map of merge
//! drawing (`anicraft_geom::planar`, ADR 0031).

use crate::merge::{Part, remerge_with};
use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::{ColorEffect, Element, ElementId, ElementKind, LayerId, MAX_GROUP_DEPTH, Shape};
use anicraft_geom::planar::{self, Item};
use anicraft_geom::{Affine, FillRule, Path};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c(
            "modify.group",
            "Group",
            "Modify",
            Some("Ctrl+G"),
            "{} puts the selected elements on each layer into one group there (ADR 0073), where the topmost of them was, and selects it → {group, members} (one layer) or {layers: [..]}. Members keep their places exactly: the group's transform starts as the identity. Raw shapes inside merge only with each other; double-click the group (symbol.edit) to edit it in place",
            true,
            (|s, p| s.each_layer(true, |s| group(s, p))) as crate::RunFn,
        ),
        c(
            "modify.ungroup",
            "Ungroup",
            "Modify",
            Some("Ctrl+Shift+G"),
            "{} puts each selected group's members back on its layer where it was, each placed and coloured as the group showed it, and selects them; raw shapes among them merge with the drawing there → {members}",
            true,
            |s, p| s.each_layer(true, |s| ungroup(s, p)),
        ),
        c(
            "modify.combine",
            "Combine Objects",
            "",
            None,
            "{op: union|intersect|punch|crop} combines the selected shapes on the active layer, bottom to top as they are stacked (ADR 0073). union: one drawing object of everything (a group of the merged pieces when they have more than one colour or keep lines); intersect: the area every one covers, in the topmost's fill and stroke; punch: the topmost's area cut out of each of the others; crop: each of the others cut to the topmost's area. Punch and crop remove the topmost. A result of one fill (or one line) is a drawing object, one of several is a group of raw shapes → {op, created}",
            true,
            combine_cmd,
        ),
        c("modify.union", "Union", "Modify/Combine Objects", None, "{} combine with op union", true, |s, _| combine(s, Op::Union)),
        c("modify.intersect", "Intersect", "Modify/Combine Objects", None, "{} combine with op intersect", true, |s, _| combine(s, Op::Intersect)),
        c("modify.punch", "Punch", "Modify/Combine Objects", None, "{} combine with op punch", true, |s, _| combine(s, Op::Punch)),
        c("modify.crop", "Crop", "Modify/Combine Objects", None, "{} combine with op crop", true, |s, _| combine(s, Op::Crop)),
    ]
}

// ----- Group and Ungroup ----------------------------------------------------------------------

/// How deep groups nest inside `e` (0 for anything that is not a group).
pub(crate) fn group_depth(e: &Element) -> usize {
    fn depth(e: &Element, at: usize) -> usize {
        if at > MAX_GROUP_DEPTH {
            return at;
        }
        match &e.kind {
            ElementKind::Group { .. } => 1 + e.kind.group_members().iter().map(|m| depth(m, at + 1)).max().unwrap_or(0),
            _ => 0,
        }
    }
    depth(e, 0)
}

/// How many groups are open around the timeline being edited (inside the innermost open symbol).
pub(crate) fn open_groups(s: &Session) -> usize {
    s.editing.iter().rev().take_while(|l| l.open.is_group()).count()
}

fn group(s: &mut Session, _: &Value) -> Result<Value> {
    let sel = s.selection.on(s.layer);
    if sel.is_empty() {
        return Err(EngineError::State("select what to group".into()));
    }
    let layer_id = LayerId(s.doc.alloc_id());
    let id = ElementId(s.doc.alloc_id());
    let around = open_groups(s);
    let kf = s.editable_keyframe()?;
    let picked: Vec<Arc<Element>> = kf.elements.iter().filter(|e| sel.contains(&e.id)).cloned().collect();
    if picked.is_empty() {
        return Err(EngineError::State("the selection is not on this keyframe".into()));
    }
    if picked.iter().any(|e| matches!(e.kind, ElementKind::Camera)) {
        return Err(EngineError::State("the camera can't be grouped".into()));
    }
    let deepest = picked.iter().map(|e| group_depth(e)).max().unwrap_or(0);
    if around + deepest + 1 > MAX_GROUP_DEPTH {
        return Err(EngineError::State(format!("groups nest at most {MAX_GROUP_DEPTH} deep")));
    }
    // Where the topmost member was, among what stays.
    let top = kf.elements.iter().rposition(|e| sel.contains(&e.id)).unwrap_or(0);
    let below = kf.elements.iter().take(top).filter(|e| !sel.contains(&e.id)).count();
    kf.elements.retain(|e| !sel.contains(&e.id));
    let n = picked.len();
    let g = Element { id, name: String::new(), transform: Affine::IDENTITY, color: ColorEffect::None, kind: ElementKind::group(layer_id, picked) };
    let at = below.min(kf.elements.len());
    kf.elements.insert(at, Arc::new(g));
    s.select_only([id]);
    Ok(json!({"group": id.0, "members": n}))
}

fn ungroup(s: &mut Session, _: &Value) -> Result<Value> {
    let sel = s.selection.on(s.layer);
    let kf = s.editable_keyframe()?;
    if !kf.elements.iter().any(|e| sel.contains(&e.id) && matches!(e.kind, ElementKind::Group { .. })) {
        return Err(EngineError::State("select a group to ungroup".into()));
    }
    let taken: HashSet<ElementId> = kf.elements.iter().filter(|e| !(sel.contains(&e.id) && matches!(e.kind, ElementKind::Group { .. }))).map(|e| e.id).collect();
    let old = std::mem::take(&mut kf.elements);
    let mut out = Vec::with_capacity(old.len());
    let mut members: Vec<(ElementId, bool)> = Vec::new();
    let mut clash: Vec<usize> = Vec::new();
    let mut seen = taken;
    for e in old {
        if !(sel.contains(&e.id) && matches!(e.kind, ElementKind::Group { .. })) {
            out.push(e);
            continue;
        }
        for m in e.kind.group_members() {
            let mut m = Arc::unwrap_or_clone(m);
            m.transform = e.transform.mul(&m.transform);
            m.color = e.color.nested(&m.color);
            // Members keep their ids, unless a copy of this group was ungrouped here already.
            if !seen.insert(m.id) {
                clash.push(out.len());
            }
            members.push((m.id, crate::merge::mergeable(&m).is_some()));
            out.push(Arc::new(m));
        }
    }
    for i in clash {
        let fresh = ElementId(s.doc.alloc_id());
        if let Some(e) = out.get_mut(i) {
            let old = e.id;
            Arc::make_mut(e).id = fresh;
            // (The member list follows: the first entry still named the old id is this one.)
            if let Some(m) = members.iter_mut().rev().find(|m| m.0 == old) {
                m.0 = fresh;
            }
        }
    }
    s.editable_keyframe()?.elements = out;
    // Raw shapes merge with the drawing they now stand in (Animate: what was grouped cuts and joins
    // the shapes under it once it is apart), on top, as a dropped selection does.
    let raw: Vec<ElementId> = members.iter().filter(|m| m.1).map(|m| m.0).collect();
    let mut selected: Vec<ElementId> = members.iter().filter(|m| !m.1).map(|m| m.0).collect();
    if !raw.is_empty() {
        let kf = s.editable_keyframe()?;
        let mut moving = Vec::new();
        kf.elements.retain(|e| {
            let take = raw.contains(&e.id);
            if take {
                moving.push((**e).clone());
            }
            !take
        });
        selected.extend(crate::merge::merge_in(s, moving)?);
    }
    let n = selected.len();
    s.select_only(selected);
    Ok(json!({"members": n}))
}

// ----- Combine Objects ------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Union,
    Intersect,
    Punch,
    Crop,
}

fn combine_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let op = match str_req(p, "op")?.trim().to_ascii_lowercase().as_str() {
        "union" => Op::Union,
        "intersect" => Op::Intersect,
        "punch" => Op::Punch,
        "crop" => Op::Crop,
        other => return Err(EngineError::Param(format!("unknown op '{other}' (union, intersect, punch or crop)"))),
    };
    combine(s, op)
}

/// `e`'s shape, as a raw shape in keyframe space would merge it.
fn as_raw(e: &Element, sh: &Shape) -> Element {
    let mut raw = e.clone();
    let mut sh = sh.clone();
    sh.merge = true;
    raw.kind = ElementKind::Shape(sh);
    raw
}

/// `paths` one after another, as one path.
fn joined<'a>(paths: impl IntoIterator<Item = &'a Path>) -> Path {
    let mut out = Path::default();
    for p in paths {
        out.cmds.extend(p.cmds.iter().copied());
    }
    out
}

/// The area `path` (keyframe space) fills with `rule`, as outlines that don't overlap, so even-odd
/// and non-zero agree on it: open subpaths closed, overlaps resolved. `None` when it is too large
/// to merge, empty when it covers nothing.
fn area(path: &Path, rule: FillRule) -> Option<Path> {
    let m = planar::merge(&[Item { path, fill: Some(0), fill_rule: rule, stroke: None }])?;
    Some(joined(m.fills.iter().map(|p| &p.path)))
}

/// A drawing object of `path` (keyframe space), its local origin at the centre of its bounds as the
/// drawing tools place shapes.
fn object(doc: &mut anicraft_doc::Document, path: &Path, fill: Option<anicraft_doc::Paint>, stroke: Option<anicraft_doc::Stroke>, color: ColorEffect) -> Element {
    let c = path.bounds().center();
    let to_local = Affine::translate(-c.x, -c.y);
    Element {
        id: ElementId(doc.alloc_id()),
        name: String::new(),
        transform: Affine::translate(c.x, c.y),
        color,
        kind: ElementKind::Shape(Shape { path: Arc::new(path.transformed(&to_local)), fill: fill.map(|f| f.transformed(&to_local)), stroke, fill_rule: FillRule::NonZero, merge: false, warp: None }),
    }
}

/// Raw pieces (from a merge) as one result: a lone piece becomes a drawing object, several a group
/// of the raw pieces (they keep merging with each other inside it). `None` for no pieces.
fn result_of(s: &mut Session, pieces: Vec<Arc<Element>>) -> Option<Element> {
    match pieces.len() {
        0 => None,
        1 => pieces.into_iter().next().map(|p| {
            let mut e = Arc::unwrap_or_clone(p);
            if let ElementKind::Shape(sh) = &mut e.kind {
                sh.merge = false;
            }
            e
        }),
        _ => {
            // Fills of one look, with no line left, are one object.
            let first = pieces.first().and_then(|p| match &p.kind {
                ElementKind::Shape(sh) if sh.stroke.is_none() => sh.fill.clone().map(|f| (f.transformed(&p.transform), p.color)),
                _ => None,
            });
            let same = first.as_ref().is_some_and(|(f0, c0)| {
                pieces.iter().all(|p| matches!(&p.kind, ElementKind::Shape(sh) if sh.stroke.is_none() && p.color == *c0 && sh.fill.as_ref().is_some_and(|f| crate::merge::same_paint(&f.transformed(&p.transform), f0))))
            });
            if let (true, Some((fill, color))) = (same, first) {
                let outline: Vec<Path> = pieces.iter().filter_map(|p| match &p.kind {
                    ElementKind::Shape(sh) => Some(sh.path.transformed(&p.transform)),
                    _ => None,
                }).collect();
                return Some(object(&mut s.doc, &joined(outline.iter()), Some(fill), None, color));
            }
            let layer = LayerId(s.doc.alloc_id());
            Some(Element { id: ElementId(s.doc.alloc_id()), name: String::new(), transform: Affine::IDENTITY, color: ColorEffect::None, kind: ElementKind::group(layer, pieces) })
        }
    }
}

const TOO_LARGE: &str = "the shapes are too complex to combine at once";

fn combine(s: &mut Session, op: Op) -> Result<Value> {
    let sel = s.selection.on(s.layer);
    if s.selected_layers().len() > 1 {
        return Err(EngineError::State("Combine Objects works on one layer: select shapes on one layer".into()));
    }
    let els = s.editable_keyframe()?.elements.clone();
    // The selected shapes, bottom to top, with their index.
    let mut shapes: Vec<(usize, Arc<Element>)> = Vec::new();
    for (i, e) in els.iter().enumerate().filter(|(_, e)| sel.contains(&e.id)) {
        match &e.kind {
            ElementKind::Shape(_) => shapes.push((i, e.clone())),
            _ => return Err(EngineError::State("Combine Objects works on shapes and drawing objects: break symbols, text and groups apart first (Ctrl+B)".into())),
        }
    }
    if shapes.len() < 2 {
        return Err(EngineError::State("select two or more shapes to combine".into()));
    }
    let shape = |e: &Element| match &e.kind {
        ElementKind::Shape(sh) => Some(sh.clone()),
        _ => None,
    };
    // (index in the keyframe, what replaces it)
    let mut results: Vec<(usize, Option<Element>)> = Vec::new();
    match op {
        Op::Union => {
            let raw: Vec<Element> = shapes.iter().filter_map(|(_, e)| shape(e).map(|sh| as_raw(e, &sh))).collect();
            let parts: Vec<Part> = raw.iter().map(|e| Part::Shape(e, false)).collect();
            let merged = remerge_with(s, &parts, None).ok_or_else(|| EngineError::State(TOO_LARGE.into()))?;
            let top = shapes.last().map_or(0, |x| x.0);
            let one = result_of(s, merged.out);
            results.push((top, one));
        }
        Op::Intersect => {
            let (bottom, rest) = shapes.split_first().ok_or_else(|| EngineError::State("select two or more shapes to combine".into()))?;
            let sh = shape(&bottom.1).filter(|sh| sh.fill.is_some()).ok_or_else(|| EngineError::State("Intersect needs shapes with fills".into()))?;
            let mut region = area(&sh.path.transformed(&bottom.1.transform), sh.fill_rule).ok_or_else(|| EngineError::State(TOO_LARGE.into()))?;
            for (_, e) in rest {
                let sh = shape(e).filter(|sh| sh.fill.is_some()).ok_or_else(|| EngineError::State("Intersect needs shapes with fills".into()))?;
                let cutter = area(&sh.path.transformed(&e.transform), sh.fill_rule).ok_or_else(|| EngineError::State(TOO_LARGE.into()))?;
                let m = planar::merge_cut(&[Item { path: &region, fill: Some(0), fill_rule: FillRule::EvenOdd, stroke: None }], &cutter).ok_or_else(|| EngineError::State(TOO_LARGE.into()))?;
                region = joined(m.fills.iter().filter(|p| p.inside).map(|p| &p.path));
            }
            if region.cmds.is_empty() {
                return Err(EngineError::State("the shapes don't all overlap: nothing is left to intersect".into()));
            }
            // In the topmost's fill and stroke (Animate).
            let (top_i, top) = shapes.last().map(|(i, e)| (*i, e.clone())).ok_or_else(|| EngineError::State("nothing to combine".into()))?;
            let tsh = shape(&top).ok_or_else(|| EngineError::State("nothing to combine".into()))?;
            let fill = tsh.fill.map(|f| f.transformed(&top.transform));
            let stroke = tsh.stroke.map(|st| st.scaled(top.transform.mean_scale()));
            let made = object(&mut s.doc, &region, fill, stroke, top.color);
            results.push((top_i, Some(made)));
        }
        Op::Punch | Op::Crop => {
            let (top, lower) = shapes.split_last().ok_or_else(|| EngineError::State("select two or more shapes to combine".into()))?;
            let tsh = shape(&top.1).filter(|sh| sh.fill.is_some()).ok_or_else(|| EngineError::State("the topmost shape cuts with its fill: give it one".into()))?;
            let cutter = area(&tsh.path.transformed(&top.1.transform), tsh.fill_rule).ok_or_else(|| EngineError::State(TOO_LARGE.into()))?;
            results.push((top.0, None));
            for (i, e) in lower {
                let Some(sh) = shape(e) else { continue };
                let raw = as_raw(e, &sh);
                let merged = remerge_with(s, &[Part::Shape(&raw, false)], Some(&cutter)).ok_or_else(|| EngineError::State(TOO_LARGE.into()))?;
                let keep: Vec<Arc<Element>> = merged.out.into_iter().filter(|p| merged.inside.contains(&p.id) == (op == Op::Crop)).collect();
                let one = result_of(s, keep);
                results.push((*i, one));
            }
        }
    }
    // The sources go; each result takes its source's place.
    let gone: HashSet<usize> = shapes.iter().map(|x| x.0).collect();
    let mut created = Vec::new();
    let mut out = Vec::with_capacity(els.len());
    for (i, e) in els.into_iter().enumerate() {
        if !gone.contains(&i) {
            out.push(e);
            continue;
        }
        if let Some(r) = results.iter_mut().find(|(at, _)| *at == i).and_then(|(_, r)| r.take()) {
            created.push(r.id);
            out.push(Arc::new(r));
        }
    }
    if created.is_empty() && op != Op::Punch {
        return Err(EngineError::State("nothing would be left".into()));
    }
    s.editable_keyframe()?.elements = out;
    s.select_only(created.iter().copied());
    Ok(json!({"op": format!("{op:?}").to_ascii_lowercase(), "created": created.iter().map(|e| e.0).collect::<Vec<_>>()}))
}

/// The area `path` fills even-odd, from its flattening: each loop's area, added or taken away by
/// how many other loops hold it (an analytic check for the tests).
#[cfg(test)]
pub(crate) fn even_odd_area(path: &Path, m: &Affine) -> f64 {
    let loops: Vec<Vec<anicraft_geom::Vec2>> = path.flatten(m, 0.005).into_iter().map(|l| l.points).filter(|p| p.len() > 2).collect();
    let signed = |pts: &[anicraft_geom::Vec2]| -> f64 { pts.iter().zip(pts.iter().cycle().skip(1)).map(|(a, b)| f64::from(a.x) * f64::from(b.y) - f64::from(b.x) * f64::from(a.y)).sum::<f64>() * 0.5 };
    let holds = |pts: &[anicraft_geom::Vec2], p: anicraft_geom::Vec2| pts.iter().zip(pts.iter().cycle().skip(1)).filter(|(a, b)| (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x)).count() % 2 == 1;
    let mut total = 0.0;
    for (i, l) in loops.iter().enumerate() {
        // A point just inside the loop's first edge would do; its middle vertex is near enough for
        // loops that don't touch (the merge's pieces never cross).
        let Some(&p) = l.first() else { continue };
        let depth = loops.iter().enumerate().filter(|(j, o)| *j != i && holds(o, p)).count();
        total += signed(l).abs() * if depth % 2 == 0 { 1.0 } else { -1.0 };
    }
    total
}
