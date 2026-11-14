//! Merge drawing on the active keyframe (ADR 0031): raw shapes drawn or dropped onto the layer cut
//! and join the raw shapes they touch, through `anicraft_geom::planar`. The result is again raw
//! shapes: one element per fill region and per stroke run, fills under strokes.

use crate::params::*;
use crate::{CommandSpec, Result, Session};
use anicraft_doc::{ColorEffect, Document, Element, ElementId, ElementKind, Paint, Shape, Stroke, Timeline};
use anicraft_geom::planar::{self, Item, SNAP};
use anicraft_geom::{Affine, Path, PathCmd, Rect, Vec2};
use serde_json::{Value, json};
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c("draw.drop", "Drop", "", None, "{} the selected raw shapes cut and join the raw shapes they touch (after a move), on each layer", true, (|s, p| s.each_layer(true, |s| drop(s, p))) as crate::RunFn),
        c("element.select_connected", "Select Connected", "", None, "{x, y, add?, slop?} a raw stroke and the strokes joined to it, or a raw fill and the strokes on and in it", false, select_connected),
    ]
}

/// A raw shape the merge can take: any fill (solid, gradient or bitmap: the paint is carried
/// into each piece, ADR 0042) or none, and any stroke or none. A stroke's width profile is
/// carried into each piece of it (ADR 0093): the merge says which stretch of the stroke each
/// piece runs along, and the piece keeps the widths it had there.
pub(crate) fn mergeable(e: &Element) -> Option<&Shape> {
    match &e.kind {
        ElementKind::Shape(s) if s.merge => Some(s),
        _ => None,
    }
}

pub(crate) fn reach(s: &Shape, m: &Affine) -> Rect {
    let pad = s.stroke.as_ref().map_or(0.0, Stroke::reach) * m.mean_scale();
    s.path.bounds().transformed(m).inflate(pad + 2.0 * SNAP)
}

/// Where new raw shapes go in the drawing's stacking (the Brush tool's paint modes, ADR 0033).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Order {
    /// On top: a fill covers the fills and lines under it (every other drawing tool).
    #[default]
    Top,
    /// Over the fills but under the lines, so lines stay (Paint Fills).
    UnderLines,
    /// Under everything, so only empty areas take it (Paint Behind).
    Behind,
}

/// Adds `new` (raw shapes, in keyframe space) to the active keyframe, merged with the raw shapes
/// they touch. Returns the ids of the pieces that came from `new` (what to select).
pub(crate) fn merge_in(s: &mut Session, new: Vec<Element>) -> Result<Vec<ElementId>> {
    merge_in_order(s, new, Order::Top)
}

/// `merge_in`, with the new shapes placed by `order` among the ones they touch.
pub(crate) fn merge_in_order(s: &mut Session, new: Vec<Element>, order: Order) -> Result<Vec<ElementId>> {
    // Anything that can't merge goes on whole.
    let (new, whole): (Vec<Element>, Vec<Element>) = new.into_iter().partition(|e| mergeable(e).is_some());
    let mut kept: Vec<ElementId> = whole.iter().map(|e| e.id).collect();
    add_whole(s.editable_keyframe()?, whole, order);
    let existing = s.editable_keyframe()?.elements.clone();
    let area = new.iter().filter_map(|e| mergeable(e).map(|sh| reach(sh, &e.transform))).fold(Rect::EMPTY, |r, b| r.union(&b));
    let touching: Vec<usize> = existing
        .iter()
        .enumerate()
        .filter(|(_, e)| mergeable(e).is_some_and(|sh| !reach(sh, &e.transform).intersect(&area).is_empty()))
        .map(|(i, _)| i)
        .collect();
    let old: Vec<&Element> = touching.iter().filter_map(|&i| existing.get(i).map(|e| e.as_ref())).collect();
    let is_line = |e: &&Element| mergeable(e).is_some_and(|sh| sh.fill.is_none());
    // Bottom to top, the new shapes marked (their pieces are selected).
    let all: Vec<Part> = match order {
        Order::Top => old.iter().map(|e| Part::Shape(e, false)).chain(new.iter().map(|e| Part::Shape(e, true))).collect(),
        Order::Behind => new.iter().map(|e| Part::Shape(e, true)).chain(old.iter().map(|e| Part::Shape(e, false))).collect(),
        Order::UnderLines => old.iter().filter(|e| !is_line(e)).map(|e| Part::Shape(e, false)).chain(new.iter().map(|e| Part::Shape(e, true))).chain(old.iter().filter(|e| is_line(e)).map(|e| Part::Shape(e, false))).collect(),
    };
    let Some((out, picked)) = remerge(s, &all) else {
        // Too much to merge at once: keep the new shapes whole, as objects.
        kept.extend(new.iter().map(|e| e.id));
        let objects = new
            .into_iter()
            .map(|mut e| {
                if let ElementKind::Shape(sh) = &mut e.kind {
                    sh.merge = false;
                }
                e
            })
            .collect();
        add_whole(s.editable_keyframe()?, objects, order);
        return Ok(kept);
    };
    replace(s, &touching, out, order == Order::Behind)?;
    kept.extend(picked);
    Ok(kept)
}

/// One entry of a merge, bottom to top.
pub(crate) enum Part<'a> {
    /// A raw shape, and whether it is marked (the pieces it ends up on top of are reported).
    Shape(&'a Element, bool),
    /// The eraser (ADR 0034): a fill whose regions come out empty. Above a shape it takes away
    /// what it covers, fills and the lines it covers on both sides; under a shape, nothing of it.
    Eraser(&'a Path),
}

/// The fill style the eraser paints: regions of it are dropped.
const ERASED: u32 = u32::MAX;

/// Merges `parts` into raw shapes (one element per fill region and per stroke run, fills first)
/// and returns them with the ids of the pieces whose topmost source part was marked. `None` when
/// there is too much to merge at once.
pub(crate) fn remerge(s: &mut Session, parts: &[Part]) -> Option<(Vec<Arc<Element>>, Vec<ElementId>)> {
    remerge_with(s, parts, None).map(|r| (r.out, r.picked))
}

/// What [`remerge_with`] makes.
pub(crate) struct Remerged {
    pub out: Vec<Arc<Element>>,
    /// Pieces whose topmost source part was marked.
    pub picked: Vec<ElementId>,
    /// Pieces inside the cutter (ADR 0041).
    pub inside: Vec<ElementId>,
    /// Each piece and the index in `parts` of its topmost source.
    pub from: Vec<(ElementId, usize)>,
}

/// [`remerge`], cutting along `cutter` too when there is one ([`planar::merge_cut`]).
pub(crate) fn remerge_with(s: &mut Session, parts: &[Part], cutter: Option<&Path>) -> Option<Remerged> {
    remerge_doc(&mut s.doc, parts, cutter)
}

/// [`remerge_with`] for any document (new pieces take ids from `doc`).
fn remerge_doc(doc: &mut Document, parts: &[Part], cutter: Option<&Path>) -> Option<Remerged> {
    // Style tables: equal entries are one style to the merge.
    let mut fills: Vec<(Paint, ColorEffect)> = Vec::new();
    let mut strokes: Vec<(Stroke, ColorEffect)> = Vec::new();
    // A stroke with a width profile is a style of its own (its pieces never chain into another
    // stroke's), with the path its profile runs along (an index into `paths`).
    let mut profiled: Vec<(u32, usize)> = Vec::new();
    let mut paths: Vec<Path> = Vec::with_capacity(parts.len());
    let mut styles: Vec<(Option<u32>, Option<u32>, anicraft_geom::FillRule)> = Vec::with_capacity(parts.len());
    let mut marked: Vec<bool> = Vec::with_capacity(parts.len());
    for part in parts {
        match part {
            Part::Shape(e, mark) => {
                let Some(sh) = mergeable(e) else { continue };
                paths.push(sh.path.transformed(&e.transform));
                // Gradients and bitmaps are placed in keyframe space, so every piece shows its
                // part of the paint where it was.
                let fill = sh.fill.as_ref().map(|f| fill_index(&mut fills, (f.transformed(&e.transform), e.color)));
                let stroke = sh.stroke.clone().map(|st| {
                    let st = (st.scaled(e.transform.mean_scale()), e.color);
                    if st.0.profile.is_some() {
                        strokes.push(st);
                        let i = (strokes.len() - 1) as u32;
                        profiled.push((i, paths.len() - 1));
                        i
                    } else {
                        index_of(&mut strokes, st)
                    }
                });
                styles.push((fill, stroke, sh.fill_rule));
                marked.push(*mark);
            }
            Part::Eraser(path) => {
                paths.push((*path).clone());
                styles.push((Some(ERASED), None, anicraft_geom::FillRule::NonZero));
                marked.push(false);
            }
        }
    }
    let items: Vec<Item> = paths.iter().zip(&styles).map(|(path, &(fill, stroke, fill_rule))| Item { path, fill, fill_rule, stroke }).collect();
    let merged = match cutter {
        Some(c) => planar::merge_cut(&items, c)?,
        None => planar::merge(&items)?,
    };
    // Item numbers back to part numbers (parts that are not raw shapes made no item).
    let part_of: Vec<usize> = parts.iter().enumerate().filter(|(_, p)| !matches!(p, Part::Shape(e, _) if mergeable(e).is_none())).map(|(i, _)| i).collect();
    let mut out: Vec<Arc<Element>> = Vec::with_capacity(merged.fills.len() + merged.strokes.len());
    let (mut picked, mut inside, mut from) = (Vec::new(), Vec::new(), Vec::new());
    for (piece, is_fill) in merged.fills.iter().map(|p| (p, true)).chain(merged.strokes.iter().map(|p| (p, false))) {
        let (fill, stroke, color) = if is_fill {
            // Erased regions (and anything without a style) are left out.
            let Some((f, c)) = fills.get(piece.style as usize).filter(|_| piece.style != ERASED) else { continue };
            (Some(f.clone()), None, *c)
        } else {
            let Some((st, c)) = strokes.get(piece.style as usize) else { continue };
            let mut st = st.clone();
            // Each piece of a variable-width stroke keeps the widths it had (ADR 0093).
            if let (Some(profile), Some(path)) = (&st.profile, profiled.iter().find(|(i, _)| *i == piece.style).and_then(|(_, p)| paths.get(*p))) {
                st.profile = profile.along(path, &piece.spans);
            }
            (None, Some(st), *c)
        };
        // Local origin at the centre of the piece, as the drawing tools place shapes.
        let c = piece.path.bounds().center();
        let id = ElementId(doc.alloc_id());
        if marked.get(piece.item).copied().unwrap_or(false) {
            picked.push(id);
        }
        if piece.inside {
            inside.push(id);
        }
        if let Some(&part) = part_of.get(piece.item) {
            from.push((id, part));
        }
        out.push(Arc::new(Element {
            id,
            name: String::new(),
            transform: Affine::translate(c.x, c.y),
            color,
            kind: ElementKind::Shape(Shape { path: Arc::new(piece.path.transformed(&Affine::translate(-c.x, -c.y))), fill: fill.map(|f| f.transformed(&Affine::translate(-c.x, -c.y))), stroke, fill_rule: Default::default(), merge: true, warp: None }),
        }));
    }
    Some(Remerged { out, picked, inside, from })
}

/// Replaces the keyframe's elements at `taken` (ascending indices) with `out`, placed where the
/// lowest of them was (raw shapes sit under objects); taking nothing, on top, or at the bottom
/// when `behind`.
pub(crate) fn replace(s: &mut Session, taken: &[usize], out: Vec<Arc<Element>>, behind: bool) -> Result<()> {
    let kf = s.editable_keyframe()?;
    if kf.elements.len().saturating_sub(taken.len()) + out.len() > 100_000 {
        return Err(crate::EngineError::State("too many elements on this keyframe".into()));
    }
    let at = taken.first().copied().unwrap_or(if behind { 0 } else { kf.elements.len() });
    for &i in taken.iter().rev() {
        if i < kf.elements.len() {
            kf.elements.remove(i);
        }
    }
    let at = at.min(kf.elements.len());
    kf.elements.splice(at..at, out);
    Ok(())
}

/// Shapes added whole (objects): on top, or at the bottom when painting behind.
fn add_whole(kf: &mut anicraft_doc::Keyframe, els: Vec<Element>, order: Order) {
    let els = els.into_iter().map(Arc::new);
    if order == Order::Behind {
        kf.elements.splice(0..0, els);
    } else {
        kf.elements.extend(els);
    }
}

/// The fill table's entry for `v`: paints that differ only by rounding (a gradient carried into a
/// piece and back) are one style, so pieces of one fill join again.
fn fill_index(table: &mut Vec<(Paint, ColorEffect)>, v: (Paint, ColorEffect)) -> u32 {
    match table.iter().position(|x| x.1 == v.1 && same_paint(&x.0, &v.0)) {
        Some(i) => i as u32,
        None => {
            table.push(v);
            (table.len() - 1) as u32
        }
    }
}

/// Whether two paints look the same (positions within a thousandth of a unit).
pub(crate) fn same_paint(x: &Paint, y: &Paint) -> bool {
    const EPS: f32 = 1e-3;
    let near = |p: Vec2, q: Vec2| p.distance(q) <= EPS;
    match (x, y) {
        (Paint::Solid(x), Paint::Solid(y)) => x == y,
        (Paint::Linear { start: s0, end: e0, stops: a, spread: p0, linear_rgb: l0 }, Paint::Linear { start: s1, end: e1, stops: b, spread: p1, linear_rgb: l1 }) => near(*s0, *s1) && near(*e0, *e1) && a == b && p0 == p1 && l0 == l1,
        (Paint::Radial { stops: a, spread: p0, linear_rgb: l0, focal: f0, .. }, Paint::Radial { stops: b, spread: p1, linear_rgb: l1, focal: f1, .. }) => {
            // The same ring: the frames agree (centre, axes and size).
            let same_frame = match (x.frame(None), y.frame(None)) {
                (Some(m0), Some(m1)) => m0.m.iter().zip(m1.m).all(|(u, v)| (u - v).abs() <= EPS * u.abs().max(v.abs()).max(1.0)),
                _ => false,
            };
            same_frame && a == b && p0 == p1 && l0 == l1 && (f0 - f1).abs() <= EPS
        }
        (Paint::Bitmap { bitmap: b0, repeat: r0, matrix: m0 }, Paint::Bitmap { bitmap: b1, repeat: r1, matrix: m1 }) => {
            b0 == b1 && r0 == r1 && m0.m.iter().zip(m1.m).all(|(x, y)| (x - y).abs() <= EPS * x.abs().max(y.abs()).max(1.0))
        }
        _ => false,
    }
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
    drop_selected(s)
}

/// [`drop`] on the active layer (also when an envelope closes, ADR 0080).
pub(crate) fn drop_selected(s: &mut Session) -> Result<Value> {
    let sel = s.selection.on(s.layer);
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
    s.select_only(keep);
    Ok(json!({"dropped": n, "selected": s.selection.id_list()}))
}

// ----- the Paint Bucket and imported drawings (ADR 0042) ---------------------------------------

/// The probe's fill style: what is left of it after the merge is empty area.
const PROBE: u32 = u32::MAX - 1;

/// The empty area around `at` that the raw strokes and fills of `els` close off, as Animate's
/// Paint Bucket fills it: exactly up to the lines and fill edges, holes and all. `None` when the
/// raw shapes leave it open (it reaches past them), when `at` is on a fill, or when there is too
/// much to merge at once.
///
/// A fill under everything (the probe) is merged with the raw shapes: what remains of it are the
/// empty areas, divided by every line and fill edge, and the one under `at` is the answer unless
/// it reaches the probe's border. The probe starts as a small window around `at`, with only the
/// shapes that reach into it, and grows while the area reaches its border, so a fill costs what
/// is near it rather than the whole drawing (shapes wholly outside a window cannot touch an area
/// inside it).
pub(crate) fn enclosed_area(els: &[Arc<Element>], at: Vec2) -> Option<Path> {
    let raw: Vec<(&Element, &Shape, Rect)> = els.iter().filter_map(|e| mergeable(e).map(|sh| (e.as_ref(), sh, reach(sh, &e.transform)))).collect();
    let area = raw.iter().fold(Rect::EMPTY, |r, (_, _, b)| r.union(b));
    if area.is_empty() || !area.contains(at) {
        return None;
    }
    let all = area.inflate(8.0);
    let mut half = 64.0f32;
    loop {
        let window = Rect::new(at.x - half, at.y - half, at.x + half, at.y + half);
        let last = window.min.x <= all.min.x && window.min.y <= all.min.y && window.max.x >= all.max.x && window.max.y >= all.max.y;
        let outer = if last { all } else { window };
        let near: Vec<&(&Element, &Shape, Rect)> = raw.iter().filter(|(_, _, b)| !b.intersect(&outer).is_empty()).collect();
        match probe(&near, &outer, at)? {
            Some(path) => return Some(path),
            None if last => return None,
            // It reaches the window's border: look wider.
            None => half *= 4.0,
        }
    }
}

/// One probe merge over `outer`: `Some(Some(area))` when the empty area at `at` is closed off
/// inside it, `Some(None)` when it reaches the border, `None` when `at` is not in an empty area
/// or the merge declines.
fn probe(raw: &[&(&Element, &Shape, Rect)], outer: &Rect, at: Vec2) -> Option<Option<Path>> {
    let rect = Path::rect(outer.min.x, outer.min.y, outer.width(), outer.height());
    let paths: Vec<Path> = raw.iter().map(|(e, sh, _)| sh.path.transformed(&e.transform)).collect();
    let mut items = vec![Item { path: &rect, fill: Some(PROBE), fill_rule: anicraft_geom::FillRule::NonZero, stroke: None }];
    for (i, ((_, sh, _), path)) in raw.iter().map(|x| **x).zip(&paths).enumerate() {
        let n = u32::try_from(i).ok()?;
        items.push(Item { path, fill: sh.fill.as_ref().map(|_| n), fill_rule: sh.fill_rule, stroke: sh.stroke.as_ref().map(|_| n) });
    }
    let merged = planar::merge(&items)?;
    let piece = merged.fills.into_iter().find(|p| p.style == PROBE && p.path.contains(at, &Affine::IDENTITY, anicraft_geom::FillRule::default()))?;
    let b = piece.path.bounds();
    let open = b.min.x <= outer.min.x + 1.0 || b.min.y <= outer.min.y + 1.0 || b.max.x >= outer.max.x - 1.0 || b.max.y >= outer.max.y - 1.0;
    Some((!open).then_some(piece.path))
}

/// Merges every keyframe's raw shapes in `doc` into one merged drawing each (scenes and
/// symbols): what an imported Animate file's raw shapes need, as their styles arrive one shape
/// per style. Keyframes with too much to merge at once keep their shapes as they are. Returns
/// how many keyframes changed.
pub(crate) fn merge_document(doc: &mut Document) -> usize {
    let mut n = 0;
    let mut scenes = doc.scenes.clone();
    for sc in scenes.iter_mut() {
        let mut tl = sc.timeline.clone();
        let k = merge_timeline(doc, &mut tl);
        if k > 0 {
            Arc::make_mut(sc).timeline = tl;
            n += k;
        }
    }
    let mut library = doc.library.clone();
    for sym in library.iter_mut() {
        let mut tl = sym.timeline.clone();
        let k = merge_timeline(doc, &mut tl);
        if k > 0 {
            Arc::make_mut(sym).timeline = tl;
            n += k;
        }
    }
    if n > 0 {
        doc.scenes = scenes;
        doc.library = library;
    }
    n
}

fn merge_timeline(doc: &mut Document, tl: &mut Timeline) -> usize {
    merge_timeline_at(doc, tl, 0)
}

/// [`merge_timeline`], inside groups too (ADR 0073): a group's raw shapes merge with each other.
fn merge_timeline_at(doc: &mut Document, tl: &mut Timeline, depth: usize) -> usize {
    let mut n = 0;
    let is_group = |e: &Element| matches!(e.kind, ElementKind::Group { .. });
    for layer in tl.layers.iter_mut() {
        if depth < anicraft_doc::MAX_GROUP_DEPTH && layer.keyframes.iter().any(|k| k.elements.iter().any(|e| is_group(e))) {
            let layer = Arc::make_mut(layer);
            for kf in layer.keyframes.iter_mut().filter(|k| k.elements.iter().any(|e| is_group(e))) {
                for e in Arc::make_mut(kf).elements.iter_mut().filter(|e| is_group(e)) {
                    let ElementKind::Group { members } = &e.kind else { continue };
                    let mut members = members.clone();
                    let k = merge_timeline_at(doc, &mut members, depth + 1);
                    if k > 0 {
                        Arc::make_mut(e).kind = ElementKind::Group { members };
                        n += k;
                    }
                }
            }
        }
        if !layer.keyframes.iter().any(|k| k.elements.iter().any(|e| mergeable(e).is_some())) {
            continue;
        }
        let layer = Arc::make_mut(layer);
        // A tween pairs its keyframes' shapes in order, so the keyframes it joins keep theirs.
        let shape_tween = |k: &anicraft_doc::Keyframe| k.tween.is_some() || k.motion.is_some();
        let morphing: Vec<bool> = (0..layer.keyframes.len()).map(|i| layer.keyframes.get(i).is_some_and(|k| shape_tween(k)) || i.checked_sub(1).and_then(|j| layer.keyframes.get(j)).is_some_and(|k| shape_tween(k))).collect();
        for (kf, morph) in layer.keyframes.iter_mut().zip(morphing) {
            let idx: Vec<usize> = kf.elements.iter().enumerate().filter(|(_, e)| mergeable(e).is_some()).map(|(i, _)| i).collect();
            if idx.is_empty() || morph {
                continue;
            }
            let kf = Arc::make_mut(kf);
            let existing = kf.elements.clone();
            let parts: Vec<Part> = idx.iter().filter_map(|&i| existing.get(i)).map(|e| Part::Shape(e.as_ref(), false)).collect();
            let Some(rm) = remerge_doc(doc, &parts, None) else { continue };
            let at = idx.first().copied().unwrap_or(0);
            for &i in idx.iter().rev() {
                if i < kf.elements.len() {
                    kf.elements.remove(i);
                }
            }
            let at = at.min(kf.elements.len());
            kf.elements.splice(at..at, rm.out);
            n += 1;
        }
    }
    n
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
    s.selection.extend_on(s.layer, picked);
    Ok(json!({"selected": s.selection.id_list()}))
}
