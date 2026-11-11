//! The Subselection tool on raw shapes (ADR 0042; issue #42 §3).
//!
//! Clicking a raw stroke or fill shows the anchors of every raw shape **connected** to it: the
//! strokes and fills that meet it at a point or along an edge, and the ones those meet, and so
//! on. A layer's merged drawing stores a shared corner once in each shape that has it (ADR 0031),
//! so the anchors are listed once per position. Moving one moves every shape that shares it
//! (`raw.move_point`); dragging a handle changes the curve in every shape that has that edge
//! (`raw.move_handle`).
//!
//! Which anchors are selected is view state, kept by the editor (as for drawing objects): the
//! commands here report anchors and handles by stage position, and take positions back.

use crate::cmds_raw::{check_extent, edit_edge, follow_edge, remerge_in_place};
use crate::merge::{mergeable, reach};
use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::{Element, ElementId, ElementKind};
use anicraft_geom::planar::SNAP;
use anicraft_geom::raw::{self as rawgeom, Cubic};
use anicraft_geom::{Path, PathCmd, Rect, Vec2};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c(
            "raw.subselect",
            "Subselect",
            "",
            None,
            "{x, y, add?: bool, slop?} the Subselection tool's click: the raw stroke or fill at (x, y) and every raw shape connected to it are selected, to show their anchors (an object there is selected alone); returns the anchors and, on an edge, its ends",
            false,
            subselect as crate::RunFn,
        ),
        c(
            "raw.subselect_rect",
            "Subselect Rectangle",
            "",
            None,
            "{x0, y0, x1, y1, add?: bool} the Subselection tool's marquee, on the active layer only: the shapes it touches are selected whole (raw ones with every raw shape connected to them); returns the raw anchors inside it",
            false,
            subselect_rect,
        ),
        c("raw.points", "Raw Points", "", None, "{ids?: [..]} the anchors of raw shapes (default: the selection), once per position, and the handles of their curved segments", false, points_cmd),
        c(
            "raw.move_handle",
            "Move Handle",
            "",
            None,
            "{id, cmd, slot: 1|2, dx, dy, smooth?: bool, merge?: bool} moves a handle (control point `slot` of segment `cmd` of shape `id`, as `raw.points` lists it) by (dx, dy): the curve changes in every raw shape that has that edge, and they merge again (unless merge is false); at a smooth anchor the other handle turns to stay opposite, keeping its length (unless smooth is false: Alt breaks the pair)",
            true,
            move_handle,
        ),
    ]
}

/// Most raw shapes one connected set takes in (keeps a click's cost bounded).
const MAX_SHAPES: usize = 20_000;
/// Most anchors listed.
const MAX_ANCHORS: usize = 50_000;
/// Raw shapes are connected where a point of one lies this near the other.
const TOUCH: f32 = 2.0 * SNAP;
/// Flattening tolerance for the connection test.
const FLAT: f32 = 0.05;

// ----- connected raw shapes ----------------------------------------------------------------------

/// A raw shape's stage-space geometry for the connection test.
struct Geo {
    id: ElementId,
    bounds: Rect,
    anchors: Vec<Vec2>,
    lines: Vec<(Vec2, Vec2)>,
}

fn geo(e: &Element) -> Option<Geo> {
    let sh = mergeable(e)?;
    let m = e.transform;
    let anchors = sh.path.editable_anchors().into_iter().map(|(_, p)| m.apply(p)).collect();
    let mut lines = Vec::new();
    for pl in sh.path.flatten(&m, FLAT) {
        lines.extend(pl.points.windows(2).filter_map(|w| Some((*w.first()?, *w.get(1)?))));
        if pl.closed
            && let (Some(a), Some(b)) = (pl.points.last(), pl.points.first())
        {
            lines.push((*a, *b));
        }
    }
    Some(Geo { id: e.id, bounds: sh.path.bounds().transformed(&m).inflate(TOUCH), anchors, lines })
}

/// Whether a point of `a` lies on `b`, within [`TOUCH`].
fn lands_on(a: &Geo, b: &Geo) -> bool {
    a.anchors.iter().filter(|p| b.bounds.contains(**p)).any(|p| b.lines.iter().any(|(u, v)| anicraft_geom::segment_distance(*p, *u, *v) <= TOUCH))
}

fn touch(a: &Geo, b: &Geo) -> bool {
    !a.bounds.intersect(&b.bounds).is_empty() && (lands_on(a, b) || lands_on(b, a))
}

/// A grid over the shapes' bounds, so each shape is only tested against its neighbours.
struct Grid {
    cell: f32,
    cells: HashMap<(i64, i64), Vec<usize>>,
    /// Shapes too large for the grid: tested against everything.
    big: Vec<usize>,
}

/// Most cells one shape is put in before it counts as big.
const MAX_CELLS: i64 = 256;

impl Grid {
    fn new(geos: &[Geo]) -> Grid {
        let sum: f32 = geos.iter().map(|g| g.bounds.width().max(g.bounds.height())).sum();
        let cell = (sum / geos.len().max(1) as f32).clamp(1.0, 1.0e6);
        let mut grid = Grid { cell, cells: HashMap::new(), big: Vec::new() };
        for (i, g) in geos.iter().enumerate() {
            match grid.span(&g.bounds) {
                Some(cells) => {
                    for k in cells {
                        grid.cells.entry(k).or_default().push(i);
                    }
                }
                None => grid.big.push(i),
            }
        }
        grid
    }

    /// The cells `r` covers, unless that is too many.
    fn span(&self, r: &Rect) -> Option<Vec<(i64, i64)>> {
        let k = |v: f32| (v / self.cell).floor() as i64;
        let (x0, y0, x1, y1) = (k(r.min.x), k(r.min.y), k(r.max.x), k(r.max.y));
        if (x1 - x0 + 1).saturating_mul(y1 - y0 + 1) > MAX_CELLS {
            return None;
        }
        Some((x0..=x1).flat_map(|x| (y0..=y1).map(move |y| (x, y))).collect())
    }

    /// The shapes that may touch `r` (every shape when `r` is too large).
    fn near(&self, r: &Rect, n: usize) -> Vec<usize> {
        let mut out: Vec<usize> = match self.span(r) {
            Some(cells) => cells.iter().filter_map(|k| self.cells.get(k)).flatten().copied().chain(self.big.iter().copied()).collect(),
            None => (0..n).collect(),
        };
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// The raw shapes among `els` connected to the `seeds` (themselves included): sharing a point
/// or an edge, directly or through others. Objects among the seeds are left out.
pub fn connected(els: &[Arc<Element>], seeds: &[ElementId]) -> Vec<ElementId> {
    let geos: Vec<Geo> = els.iter().filter_map(|e| geo(e)).take(MAX_SHAPES).collect();
    let grid = Grid::new(&geos);
    let mut seen = vec![false; geos.len()];
    let mut todo: Vec<usize> = geos.iter().enumerate().filter(|(_, g)| seeds.contains(&g.id)).map(|(i, _)| i).collect();
    for &i in &todo {
        if let Some(s) = seen.get_mut(i) {
            *s = true;
        }
    }
    let mut out = Vec::new();
    while let Some(i) = todo.pop() {
        let Some(g) = geos.get(i) else { continue };
        out.push(g.id);
        for j in grid.near(&g.bounds, geos.len()) {
            if seen.get(j).copied().unwrap_or(true) {
                continue;
            }
            if geos.get(j).is_some_and(|h| touch(g, h)) {
                if let Some(s) = seen.get_mut(j) {
                    *s = true;
                }
                todo.push(j);
            }
        }
    }
    // In stacking order, as the layer has them.
    els.iter().map(|e| e.id).filter(|id| out.contains(id)).collect()
}

// ----- anchors and handles -----------------------------------------------------------------------

/// A handle of a curved segment, as the Subselection tool shows it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Handle {
    /// Where the handle is, and the anchor its direction line starts from (stage space).
    pub at: Vec2,
    pub anchor: Vec2,
    /// The shape, segment and control point (1: the segment's first, 2: its second) that
    /// `raw.move_handle` takes.
    pub id: ElementId,
    pub cmd: usize,
    pub slot: u8,
}

/// The anchors and handles of a set of raw shapes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RawPoints {
    /// Every anchor once (shapes share corners), in stage space.
    pub anchors: Vec<Vec2>,
    /// The handles of curved segments, once per position.
    pub handles: Vec<Handle>,
}

/// Positions seen so far, within [`SNAP`] of each other counting as one.
#[derive(Default)]
struct Seen(HashMap<(i64, i64), Vec<Vec2>>);

impl Seen {
    fn key(p: Vec2) -> (i64, i64) {
        ((p.x / SNAP).floor() as i64, (p.y / SNAP).floor() as i64)
    }
    /// Whether `p` is new (and remembers it).
    fn insert(&mut self, p: Vec2) -> bool {
        let (kx, ky) = Seen::key(p);
        let near = (-1..=1).any(|dx| (-1..=1).any(|dy| self.0.get(&(kx + dx, ky + dy)).is_some_and(|v| v.iter().any(|q| q.distance(p) <= SNAP))));
        if !near {
            self.0.entry((kx, ky)).or_default().push(p);
        }
        !near
    }
}

/// The path as the raw commands address it (a closed subpath's last side written out).
fn explicit(path: &Path) -> Path {
    let mut p = path.clone();
    p.close_explicitly();
    p
}

impl Session {
    /// The raw shapes on the active layer connected to `ids` (themselves included).
    pub fn raw_connected(&self, ids: &[ElementId]) -> Vec<ElementId> {
        connected(&self.visible_elements(), ids)
    }

    /// The anchors (once per position) and curve handles of the raw shapes `ids` on the active
    /// layer.
    pub fn raw_points(&self, ids: &[ElementId]) -> RawPoints {
        let mut out = RawPoints::default();
        let (mut anchors, mut handles) = (Seen::default(), HashMap::<((i64, i64), (i64, i64)), Vec<(Vec2, Vec2)>>::new());
        for e in self.visible_elements().iter().filter(|e| ids.contains(&e.id)) {
            let Some(sh) = mergeable(e) else { continue };
            let m = e.transform;
            let path = explicit(&sh.path);
            for (_, q) in path.editable_anchors() {
                let q = m.apply(q);
                if out.anchors.len() < MAX_ANCHORS && anchors.insert(q) {
                    out.anchors.push(q);
                }
            }
            for k in 0..path.cmds.len().min(MAX_ANCHORS) {
                if !matches!(path.cmds.get(k), Some(PathCmd::CubicTo(..) | PathCmd::QuadTo(..))) {
                    continue;
                }
                let Some(c) = rawgeom::segment_cubic(&path, k).map(|c| rawgeom::transform(&c, &m)) else { continue };
                for (slot, at, anchor) in [(1u8, c[1], c[0]), (2u8, c[2], c[3])] {
                    // A handle on its anchor has no direction to show.
                    if at.distance(anchor) <= SNAP || out.handles.len() >= MAX_ANCHORS {
                        continue;
                    }
                    // One edge shared by two shapes has its handles in both: show them once.
                    let key = (Seen::key(anchor), Seen::key(at));
                    let list = handles.entry(key).or_default();
                    if list.iter().any(|(a, h)| a.distance(anchor) <= SNAP && h.distance(at) <= SNAP) {
                        continue;
                    }
                    list.push((anchor, at));
                    out.handles.push(Handle { at, anchor, id: e.id, cmd: k, slot });
                }
            }
        }
        out
    }
}

fn points_json(pts: &RawPoints) -> Value {
    json!({
        "anchors": pts.anchors.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>(),
        "handles": pts.handles.iter().map(|h| json!({"x": h.at.x, "y": h.at.y, "ax": h.anchor.x, "ay": h.anchor.y, "id": h.id.0, "cmd": h.cmd, "slot": h.slot})).collect::<Vec<_>>(),
    })
}

fn points_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let ids: Vec<ElementId> = match opt_u64_list(p, "ids")? {
        Some(v) => v.into_iter().map(ElementId).collect(),
        None => s.selection.on(s.layer),
    };
    Ok(points_json(&s.raw_points(&ids)))
}

// ----- clicking and the marquee ------------------------------------------------------------------

fn subselect(s: &mut Session, p: &Value) -> Result<Value> {
    let at = Vec2::new(f32_req(p, "x")?, f32_req(p, "y")?);
    let add = opt_bool(p, "add")?.unwrap_or(false);
    let slop = opt_f32(p, "slop")?.unwrap_or(2.0).clamp(0.0, 100.0);
    let Some((layer, hit)) = s.hit_test(at, slop) else {
        if !add {
            s.selection.clear();
        }
        return Ok(json!({"selected": s.selection.id_list(), "raw": false, "ends": []}));
    };
    if layer != s.layer {
        s.layer = layer;
        s.selection.clear();
    }
    let els = s.visible_elements();
    let el = els.iter().find(|e| e.id == hit);
    let raw = el.is_some_and(|e| mergeable(e).is_some());
    let picked = if raw { connected(&els, &[hit]) } else { vec![hit] };
    // On an edge (not inside a fill): its ends, whose handles show.
    let mut ends = Vec::new();
    if let Some(e) = el.filter(|_| raw)
        && let Some(sh) = mergeable(e)
    {
        let path = explicit(&sh.path);
        if let Some((cmd, _, d)) = path.nearest_segment(at, &e.transform)
            && d <= slop + sh.stroke.as_ref().map_or(0.0, |st| st.width * 0.5 * e.transform.mean_scale())
            && let Some(c) = rawgeom::segment_cubic(&path, cmd)
        {
            ends = vec![e.transform.apply(c[0]), e.transform.apply(c[3])];
        }
    }
    if !add {
        s.selection.clear();
    }
    s.selection.extend_on(s.layer, picked);
    let mut v = if raw { points_json(&s.raw_points(&s.selection.on(s.layer))) } else { json!({}) };
    v["selected"] = json!(s.selection.id_list());
    v["raw"] = json!(raw);
    v["ends"] = json!(ends.iter().map(|q| [q.x, q.y]).collect::<Vec<_>>());
    Ok(v)
}

/// Whether the shape `e` (stage space) has a part in `r`: a point, a line through it, or a fill
/// around it. Anything else counts by its bounds.
fn touches_rect(e: &Element, r: &Rect, bounds: &Rect) -> bool {
    if bounds.intersect(r).is_empty() {
        return false;
    }
    let ElementKind::Shape(sh) = &e.kind else { return true };
    let m = e.transform;
    let crosses = |a: Vec2, b: Vec2| {
        if r.contains(a) || r.contains(b) {
            return true;
        }
        let (p, q, u, v) = (r.min, Vec2::new(r.max.x, r.min.y), r.max, Vec2::new(r.min.x, r.max.y));
        [(p, q), (q, u), (u, v), (v, p)].into_iter().any(|(c, d)| segments_cross(a, b, c, d))
    };
    for pl in sh.path.flatten(&m, 0.25) {
        if pl.points.windows(2).any(|w| matches!(w, [a, b] if crosses(*a, *b))) || pl.points.first().is_some_and(|p| r.contains(*p)) {
            return true;
        }
        if pl.closed && matches!((pl.points.last(), pl.points.first()), (Some(a), Some(b)) if crosses(*a, *b)) {
            return true;
        }
    }
    sh.fill.is_some() && sh.path.contains(r.center(), &m, sh.fill_rule)
}

fn segments_cross(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> bool {
    let side = |p: Vec2, q: Vec2, x: Vec2| (q - p).cross(x - p);
    let (d1, d2, d3, d4) = (side(c, d, a), side(c, d, b), side(a, b, c), side(a, b, d));
    (d1 * d2 <= 0.0) && (d3 * d4 <= 0.0)
}

fn subselect_rect(s: &mut Session, p: &Value) -> Result<Value> {
    let a = Vec2::new(f32_req(p, "x0")?, f32_req(p, "y0")?);
    let b = Vec2::new(f32_req(p, "x1")?, f32_req(p, "y1")?);
    let add = opt_bool(p, "add")?.unwrap_or(false);
    let r = Rect::from_points(a, b);
    if !add {
        s.selection.clear();
    }
    let usable = s.active_layer().is_some_and(|l| l.visible && !l.locked) && !s.timeline().locked_by_folder(s.layer);
    let empty = json!({"selected": s.selection.id_list(), "anchors": []});
    if !usable || !(r.width() > 0.0 || r.height() > 0.0) {
        return Ok(empty);
    }
    let els = s.visible_elements();
    let (mut seeds, mut objects) = (Vec::new(), Vec::new());
    for e in &els {
        if !touches_rect(e, &r, &s.element_bounds(e.id)) {
            continue;
        }
        if mergeable(e).is_some() { seeds.push(e.id) } else { objects.push(e.id) }
    }
    let raw = connected(&els, &seeds);
    // Subselection's marquee stays on the active layer, as in Animate (ADR 0042, 0044).
    s.selection.extend_on(s.layer, raw.iter().chain(&objects).copied());
    let inside: Vec<[f32; 2]> = s.raw_points(&raw).anchors.into_iter().filter(|q| r.contains(*q)).map(|q| [q.x, q.y]).collect();
    Ok(json!({"selected": s.selection.id_list(), "anchors": inside}))
}

// ----- handles -----------------------------------------------------------------------------------

/// The other curve at the anchor of handle `slot` of `old` (stage space) when the anchor is smooth
/// there: the one edge leaving it whose handle lies opposite this one ([`rawgeom::smooth_at`]), in
/// any raw shape on the layer (or in `el` alone, a drawing object). Oriented from the anchor.
fn opposite_handle(els: &[Arc<Element>], el: &Element, raw: bool, old: &Cubic, slot: usize) -> Option<rawgeom::Seg> {
    let (a, h) = if slot == 1 { (old[0], old[1]) } else { (old[3], old[2]) };
    let dragged = rawgeom::Seg { c: if slot == 1 { *old } else { [old[3], old[2], old[1], old[0]] }, line: false };
    let tol = 1.5 * SNAP;
    let mut subs = Vec::new();
    let from = |e: &Element| match &e.kind {
        ElementKind::Shape(sh) => rawgeom::subpaths(&sh.path.transformed(&e.transform)),
        _ => Vec::new(),
    };
    if raw {
        for e in els.iter().filter(|e| mergeable(e).is_some_and(|sh| reach(sh, &e.transform).contains(a))) {
            subs.extend(from(e));
        }
    } else {
        subs = from(el);
    }
    let edges = rawgeom::distinct_edges(rawgeom::edges_at(&subs, a, tol), tol);
    let mut found = edges.into_iter().filter(|f| !rawgeom::same_edge(f, &dragged, tol) && !f.line && rawgeom::smooth_at(a, h, f.c[1]));
    let f = found.next()?;
    // Two candidates: a junction, not a smooth pair.
    found.next().is_none().then_some(f)
}

fn move_handle(s: &mut Session, p: &Value) -> Result<Value> {
    let id = ElementId(opt_u64(p, "id")?.ok_or_else(|| EngineError::Param("param 'id' is required".into()))?);
    let cmd = opt_u64(p, "cmd")?.ok_or_else(|| EngineError::Param("param 'cmd' is required".into()))?;
    let cmd = usize::try_from(cmd).unwrap_or(usize::MAX);
    let slot = opt_u64(p, "slot")?.filter(|k| matches!(k, 1 | 2)).ok_or_else(|| EngineError::Param("param 'slot' must be 1 or 2".into()))? as usize;
    let d = Vec2::new(f32_req(p, "dx")?, f32_req(p, "dy")?);
    let merge_after = opt_bool(p, "merge")?.unwrap_or(true);
    let els = s.visible_elements();
    let el = els.iter().find(|e| e.id == id).ok_or_else(|| EngineError::State(format!("element {} is not on the active layer at this frame", id.0)))?;
    let ElementKind::Shape(sh) = &el.kind else { return Err(EngineError::State("only shapes have handles".into())) };
    let path = explicit(&sh.path);
    if !matches!(path.cmds.get(cmd), Some(PathCmd::CubicTo(..) | PathCmd::QuadTo(..))) {
        return Err(EngineError::Param(format!("segment {cmd} of this shape is not a curve (it has no handles)")));
    }
    let old: Cubic = rawgeom::segment_cubic(&path, cmd).map(|c| rawgeom::transform(&c, &el.transform)).ok_or_else(|| EngineError::Param(format!("no segment {cmd} on this shape")))?;
    let mut new = old;
    if let Some(h) = new.get_mut(slot) {
        *h = *h + d;
    }
    if !new.iter().all(|q| q.is_finite()) {
        return Err(EngineError::Param("the move is too large".into()));
    }
    let mut bounds = Path::new();
    bounds.move_to(new[0]).cubic_to(new[1], new[2], new[3]);
    check_extent(&bounds)?;
    let raw = mergeable(el).is_some();
    // A smooth anchor's other handle turns with this one (ADR 0045).
    let partner = if opt_bool(p, "smooth")?.unwrap_or(true) { opposite_handle(&els, el, raw, &old, slot).map(|f| (f, new[slot])) } else { None };
    let mut changed = edit_edge(s, id, cmd, &old, &new, raw)?;
    if let Some((f, h)) = partner {
        let a = f.c[0];
        let turned = rawgeom::Seg { c: [a, rawgeom::turned_opposite(a, h, f.c[1]), f.c[2], f.c[3]], line: false };
        for e in follow_edge(s, (!raw).then_some(id), &f.c, &turned.c)? {
            if !changed.contains(&e) {
                changed.push(e);
            }
        }
    }
    if raw && merge_after {
        remerge_in_place(s, &changed)?;
    }
    let at = new.get(slot).copied().unwrap_or(new[0]);
    Ok(json!({"x": at.x, "y": at.y, "changed": changed.iter().map(|e| e.0).collect::<Vec<_>>(), "merged": raw && merge_after}))
}
