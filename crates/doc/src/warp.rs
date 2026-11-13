//! Asset Warp (ADR 0079): a deformation mesh over a shape or a bitmap, bent by pins.
//!
//! A [`Warp`] sits on a [`Shape`] (a bitmap on the stage is a shape filled with its image). It
//! keeps the artwork's outline as it was drawn (`rest`), a mesh **resolution**, and the
//! **pins**: each a point of the artwork as drawn (`rest`) and where it is now (`at`), both in
//! the shape's own space. The mesh is not stored: it is a grid over the rest outline's bounds,
//! `resolution` cells along the longer side (square-ish cells, each split into two triangles,
//! the diagonals alternating so the grid has no preferred direction), so it is always the same
//! for the same artwork.
//!
//! **Deformation** is the rigid moving-least-squares deformation of S. Schaefer, T. McPhail and
//! J. Warren, *Image deformation using moving least squares* (SIGGRAPH 2006, §2.3), evaluated at
//! the mesh's vertices: each vertex `v` moves by the rotation and translation that best carry
//! the pins near it (weighted `1 / |p − v|²`) to where they are. It interpolates the pins
//! exactly, is as rigid as possible locally (no shear, no scale), is a closed form per vertex
//! (no solver, no iterations, nothing to converge), and depends only on the pins, so it is
//! deterministic and its cost is bounded: vertices × pins. Between vertices the mesh is
//! piecewise affine, so a bitmap is drawn as textured triangles and a shape's outline maps
//! through the same triangles: the shape and the bitmap bend the same way.
//!
//! **The outline.** Straight edges are split where they cross the mesh's edges, so a line
//! maps exactly to the mesh's image of it (a bitmap's outline is exactly the mesh's border).
//! Curves are split into pieces no longer than half a cell, each end mapped through the mesh
//! and each handle by the triangle of the end it belongs to, so curves stay smooth. **At rest**
//! (every pin where it was placed) the outline is the drawing, bit for bit: the very `Arc` of
//! the rest path.
//!
//! **The file holds what shows.** Commands write the bent outline into the shape's `path` and
//! its fingerprint into `shown`. A shape whose path no longer has that fingerprint was edited
//! by another tool since: its warp is stale, [`Shape::live_warp`] ignores it, and the next warp
//! command starts again from the shape as it now is ([`Warp::rebased`]). Pins tween between
//! keyframes with the keyframe's easing (as bone poses do, ADR 0043): `evaluate` blends the
//! pins and bends the rest outline, so a tween never accumulates drift.

use crate::Shape;
use anicraft_geom::{Path, PathCmd, Vec2};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Fewest and most cells along the mesh's longer side, and the resolution a new warp has.
pub const MIN_RESOLUTION: u32 = 2;
pub const MAX_RESOLUTION: u32 = 48;
pub const DEFAULT_RESOLUTION: u32 = 12;
/// Most pins on one warp.
pub const MAX_PINS: usize = 128;
/// Most anchors a warped outline may have (Optimize first beyond it).
pub const MAX_WARP_POINTS: usize = 20_000;
/// Most commands a bent outline may have; past it curves and lines are not split.
const MAX_BENT_CMDS: usize = 400_000;
/// Most pieces one curve is split into.
const MAX_SPLIT: usize = 32;
/// Largest artwork a warp covers (local units on either side).
pub const MAX_EXTENT: f32 = 1_000_000.0;
/// Two pins closer than this (local units, as drawn) are one place.
pub const PIN_SPACING: f32 = 0.5;

/// Unique id of a pin within a document (from the document's id counter).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PinId(pub u64);

/// A pin: a point of the artwork as drawn, and where it is now (the shape's space).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pin {
    pub id: PinId,
    pub rest: Vec2,
    pub at: Vec2,
}

fn default_resolution() -> u32 {
    DEFAULT_RESOLUTION
}

/// A warp mesh on a shape (see the module docs).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Warp {
    /// The outline as it was drawn, in the shape's space.
    #[serde(with = "crate::path_table")]
    pub rest: Arc<Path>,
    /// Cells along the mesh's longer side ([`MIN_RESOLUTION`]..=[`MAX_RESOLUTION`]).
    #[serde(default = "default_resolution")]
    pub resolution: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pins: Vec<Pin>,
    /// Fingerprint of the outline the warp gave the shape ([`fingerprint`]).
    #[serde(default)]
    pub shown: u64,
}

impl PartialEq for Warp {
    fn eq(&self, o: &Warp) -> bool {
        crate::arc_eq(&self.rest, &o.rest) && self.resolution == o.resolution && self.pins == o.pins && self.shown == o.shown
    }
}

/// The deformation mesh as drawn: every vertex as drawn (`rest`) and bent (`shown`), and the
/// triangles (indices into both), in the shape's space, in drawing order.
#[derive(Clone, Debug, PartialEq)]
pub struct WarpMesh {
    pub rest: Vec<Vec2>,
    pub shown: Vec<Vec2>,
    pub tris: Vec<[u32; 3]>,
}

/// The mesh's grid: `nx × ny` cells from `min` to `max`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    pub min: Vec2,
    pub max: Vec2,
    pub nx: u32,
    pub ny: u32,
}

impl Grid {
    /// The grid over `rest`'s control-point bounds with `resolution` cells along the longer side
    /// (an extent under 1 unit is widened to 1, about its middle). `None` for an empty or
    /// unbounded outline.
    pub fn of(rest: &Path, resolution: u32) -> Option<Grid> {
        let b = rest.bounds();
        if !(b.min.is_finite() && b.max.is_finite()) || b.max.x < b.min.x || b.max.y < b.min.y {
            return None;
        }
        if b.min.x.abs().max(b.min.y.abs()).max(b.max.x.abs()).max(b.max.y.abs()) > MAX_EXTENT {
            return None;
        }
        let widen = |lo: f32, hi: f32| if hi - lo < 1.0 { ((lo + hi) * 0.5 - 0.5, (lo + hi) * 0.5 + 0.5) } else { (lo, hi) };
        let (x0, x1) = widen(b.min.x, b.max.x);
        let (y0, y1) = widen(b.min.y, b.max.y);
        let (w, h) = (x1 - x0, y1 - y0);
        let r = resolution.clamp(MIN_RESOLUTION, MAX_RESOLUTION);
        let size = w.max(h) / r as f32;
        let cells = |len: f32| {
            let n = (len / size).ceil();
            if n.is_finite() { (n as u32).clamp(1, r) } else { 1 }
        };
        Some(Grid { min: Vec2::new(x0, y0), max: Vec2::new(x1, y1), nx: cells(w), ny: cells(h) })
    }

    /// The x of column line `i` (exactly the bounds at either end).
    pub fn x(&self, i: u32) -> f32 {
        if i >= self.nx { self.max.x } else { self.min.x + (self.max.x - self.min.x) * i as f32 / self.nx as f32 }
    }

    /// The y of row line `j`.
    pub fn y(&self, j: u32) -> f32 {
        if j >= self.ny { self.max.y } else { self.min.y + (self.max.y - self.min.y) * j as f32 / self.ny as f32 }
    }

    /// Vertices, row by row: `(nx + 1) × (ny + 1)`.
    pub fn vertices(&self) -> Vec<Vec2> {
        let mut out = Vec::with_capacity(((self.nx + 1) * (self.ny + 1)) as usize);
        for j in 0..=self.ny {
            for i in 0..=self.nx {
                out.push(Vec2::new(self.x(i), self.y(j)));
            }
        }
        out
    }

    fn vertex(&self, i: u32, j: u32) -> u32 {
        j * (self.nx + 1) + i
    }

    /// Whether cell `(i, j)`'s diagonal runs from its top-left to its bottom-right corner (else
    /// from its top-right to its bottom-left): they alternate like a chequerboard.
    fn falling(i: u32, j: u32) -> bool {
        (i + j).is_multiple_of(2)
    }

    /// The two triangles of cell `(i, j)`, as vertex indices.
    fn cell_tris(&self, i: u32, j: u32) -> [[u32; 3]; 2] {
        let (a, b, c, d) = (self.vertex(i, j), self.vertex(i + 1, j), self.vertex(i, j + 1), self.vertex(i + 1, j + 1));
        if Grid::falling(i, j) { [[a, b, d], [a, d, c]] } else { [[a, b, c], [b, d, c]] }
    }

    /// Every triangle, cell by cell, row by row.
    pub fn triangles(&self) -> Vec<[u32; 3]> {
        let mut out = Vec::with_capacity((2 * self.nx * self.ny) as usize);
        for j in 0..self.ny {
            for i in 0..self.nx {
                out.extend(self.cell_tris(i, j));
            }
        }
        out
    }

    /// The cell `p` falls in (clamped to the grid, so points just outside use the edge cells).
    fn cell(&self, p: Vec2) -> (u32, u32) {
        let at = |v: f32, lo: f32, hi: f32, n: u32| {
            let k = ((v - lo) / (hi - lo) * n as f32).floor();
            if k.is_finite() { (k.max(0.0) as u32).min(n - 1) } else { 0 }
        };
        (at(p.x, self.min.x, self.max.x, self.nx), at(p.y, self.min.y, self.max.y, self.ny))
    }

    /// The triangle (vertex indices) of the grid that holds `p`.
    pub fn locate(&self, p: Vec2) -> [u32; 3] {
        let (i, j) = self.cell(p);
        let [t0, t1] = self.cell_tris(i, j);
        let (x0, y0) = (self.x(i), self.y(j));
        let (cw, ch) = ((self.x(i + 1) - x0).max(1e-12), (self.y(j + 1) - y0).max(1e-12));
        let (u, v) = ((p.x - x0) / cw, (p.y - y0) / ch);
        let first = if Grid::falling(i, j) { u >= v } else { u + v <= 1.0 };
        if first { t0 } else { t1 }
    }

    /// The parameters in `(0, 1)` where the segment `a → b` crosses an edge of the mesh, in
    /// order: column and row lines, then each cell's diagonal.
    fn crossings(&self, a: Vec2, b: Vec2, out: &mut Vec<f32>) {
        out.clear();
        let d = b - a;
        if d.x != 0.0 {
            for i in 1..self.nx {
                let t = (self.x(i) - a.x) / d.x;
                if t > 0.0 && t < 1.0 {
                    out.push(t);
                }
            }
        }
        if d.y != 0.0 {
            for j in 1..self.ny {
                let t = (self.y(j) - a.y) / d.y;
                if t > 0.0 && t < 1.0 {
                    out.push(t);
                }
            }
        }
        out.sort_by(f32::total_cmp);
        out.dedup_by(|x, y| (*x - *y).abs() < 1e-6);
        // Inside each cell the segment may still cross the cell's diagonal.
        let mut diag = Vec::new();
        let mut t0 = 0.0f32;
        for &t1 in out.iter().chain(std::iter::once(&1.0)) {
            let mid = a + d * ((t0 + t1) * 0.5);
            let (i, j) = self.cell(mid);
            let (p, q) = if Grid::falling(i, j) { (Vec2::new(self.x(i), self.y(j)), Vec2::new(self.x(i + 1), self.y(j + 1))) } else { (Vec2::new(self.x(i + 1), self.y(j)), Vec2::new(self.x(i), self.y(j + 1))) };
            let e = q - p;
            let den = d.cross(e);
            if den.abs() > 1e-12 {
                let t = (p - a).cross(e) / den;
                if t > t0 + 1e-6 && t < t1 - 1e-6 {
                    diag.push(t);
                }
            }
            t0 = t1;
        }
        out.extend(diag);
        out.sort_by(f32::total_cmp);
    }
}

/// Barycentric coordinates of `p` in triangle `(a, b, c)` (extrapolated outside it); `None` for
/// a triangle with no area.
fn barycentric(a: Vec2, b: Vec2, c: Vec2, p: Vec2) -> Option<(f32, f32, f32)> {
    let det = (b - a).cross(c - a);
    if det.abs() <= 1e-12 || det.is_nan() {
        return None;
    }
    let l1 = (p - a).cross(c - a) / det;
    let l2 = (b - a).cross(p - a) / det;
    Some((1.0 - l1 - l2, l1, l2))
}

/// The rigid moving-least-squares deformation (Schaefer et al. 2006, §2.3) of `v` by pins
/// `(from, to)`, with weights `1 / |from − v|²`: the rotation and translation that best carry
/// the pins near `v` where they go, applied to `v`. A point on a pin goes with it; with one pin
/// everything moves with it; with none nothing moves. Computed in `f64`.
pub fn mls_rigid(pins: &[(Vec2, Vec2)], v: Vec2) -> Vec2 {
    if pins.is_empty() {
        return v;
    }
    let (vx, vy) = (f64::from(v.x), f64::from(v.y));
    let (mut sw, mut px, mut py, mut qx, mut qy) = (0.0f64, 0.0, 0.0, 0.0, 0.0);
    let mut ws = Vec::with_capacity(pins.len());
    for &(p, q) in pins {
        let (dx, dy) = (f64::from(p.x) - vx, f64::from(p.y) - vy);
        let d2 = dx * dx + dy * dy;
        if d2 < 1e-12 {
            return q;
        }
        let w = 1.0 / d2;
        ws.push(w);
        sw += w;
        px += w * f64::from(p.x);
        py += w * f64::from(p.y);
        qx += w * f64::from(q.x);
        qy += w * f64::from(q.y);
    }
    if !(sw > 0.0 && sw.is_finite()) {
        return v;
    }
    let (px, py, qx, qy) = (px / sw, py / sw, qx / sw, qy / sw);
    // The best rotation: its cosine and sine are the weighted dot and cross of the centred pins.
    let (mut dot, mut cross) = (0.0f64, 0.0f64);
    for (&(p, q), &w) in pins.iter().zip(&ws) {
        let (ax, ay) = (f64::from(p.x) - px, f64::from(p.y) - py);
        let (bx, by) = (f64::from(q.x) - qx, f64::from(q.y) - qy);
        dot += w * (ax * bx + ay * by);
        cross += w * (ax * by - ay * bx);
    }
    let r = dot.hypot(cross);
    let (c, s) = if r > 1e-18 && r.is_finite() { (dot / r, cross / r) } else { (1.0, 0.0) };
    let (dx, dy) = (vx - px, vy - py);
    let out = Vec2::new((qx + c * dx - s * dy) as f32, (qy + s * dx + c * dy) as f32);
    if out.is_finite() { out } else { v }
}

/// A fingerprint of `path`'s commands and coordinates (FNV-1a over their bits): how a warp
/// knows the outline is still the one it gave the shape.
pub fn fingerprint(path: &Path) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bits: u32| {
        for b in bits.to_le_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for c in &path.cmds {
        let (tag, pts): (u32, [Option<Vec2>; 3]) = match *c {
            PathCmd::MoveTo(p) => (1, [Some(p), None, None]),
            PathCmd::LineTo(p) => (2, [Some(p), None, None]),
            PathCmd::QuadTo(a, p) => (3, [Some(a), Some(p), None]),
            PathCmd::CubicTo(a, b, p) => (4, [Some(a), Some(b), Some(p)]),
            PathCmd::Close => (5, [None, None, None]),
        };
        eat(tag);
        for p in pts.into_iter().flatten() {
            eat(p.x.to_bits());
            eat(p.y.to_bits());
        }
    }
    h
}

/// How many anchors (command ends) `path` has.
pub fn anchor_count(path: &Path) -> usize {
    path.cmds.iter().filter(|c| !matches!(c, PathCmd::Close)).count()
}

/// The bent mesh: the grid's vertices as drawn and where the pins carry them.
struct Bent {
    grid: Grid,
    rest: Vec<Vec2>,
    shown: Vec<Vec2>,
}

impl Bent {
    fn at(&self, i: u32) -> (Vec2, Vec2) {
        let i = i as usize;
        (self.rest.get(i).copied().unwrap_or_default(), self.shown.get(i).copied().unwrap_or_default())
    }

    /// `p` (as drawn) mapped through triangle `tri`'s affine map (extrapolated outside it).
    fn through(&self, tri: [u32; 3], p: Vec2) -> Vec2 {
        let [(ra, sa), (rb, sb), (rc, sc)] = tri.map(|i| self.at(i));
        match barycentric(ra, rb, rc, p) {
            Some((l0, l1, l2)) => {
                let out = sa * l0 + sb * l1 + sc * l2;
                if out.is_finite() { out } else { p }
            }
            None => p + (sa - ra),
        }
    }

    /// `p` (as drawn) where the mesh carries it, and the triangle that did.
    fn map(&self, p: Vec2) -> (Vec2, [u32; 3]) {
        let tri = self.grid.locate(p);
        (self.through(tri, p), tri)
    }
}

impl Warp {
    /// A warp of the outline `rest` with no pins.
    pub fn new(rest: Arc<Path>, resolution: u32) -> Warp {
        let shown = fingerprint(&rest);
        Warp { rest, resolution: resolution.clamp(MIN_RESOLUTION, MAX_RESOLUTION), pins: Vec::new(), shown }
    }

    /// Every pin where it was placed: the artwork shows as drawn.
    pub fn at_rest(&self) -> bool {
        self.pins.iter().all(|p| p.at == p.rest)
    }

    pub fn pin(&self, id: PinId) -> Option<&Pin> {
        self.pins.iter().find(|p| p.id == id)
    }

    pub fn grid(&self) -> Option<Grid> {
        Grid::of(&self.rest, self.resolution)
    }

    fn bent(&self) -> Option<Bent> {
        let grid = self.grid()?;
        let rest = grid.vertices();
        let pins: Vec<(Vec2, Vec2)> = self.pins.iter().map(|p| (p.rest, p.at)).collect();
        let shown = if self.at_rest() { rest.clone() } else { rest.iter().map(|&v| mls_rigid(&pins, v)).collect() };
        Some(Bent { grid, rest, shown })
    }

    /// The mesh as it shows (`None` for an outline with no extent).
    pub fn mesh(&self) -> Option<WarpMesh> {
        let b = self.bent()?;
        Some(WarpMesh { tris: b.grid.triangles(), rest: b.rest, shown: b.shown })
    }

    /// Where point `p` of the artwork as drawn shows now.
    pub fn map(&self, p: Vec2) -> Vec2 {
        if self.at_rest() {
            return p;
        }
        self.bent().map_or(p, |b| b.map(p).0)
    }

    /// The point of the artwork as drawn that shows at `p` (the topmost triangle there, as the
    /// mesh draws); `None` off the mesh.
    pub fn unmap(&self, p: Vec2) -> Option<Vec2> {
        let b = self.bent()?;
        if self.at_rest() {
            return (p.x >= b.grid.min.x && p.x <= b.grid.max.x && p.y >= b.grid.min.y && p.y <= b.grid.max.y).then_some(p);
        }
        let tris = b.grid.triangles();
        for tri in tris.iter().rev() {
            let [(ra, sa), (rb, sb), (rc, sc)] = tri.map(|i| b.at(i));
            let Some((l0, l1, l2)) = barycentric(sa, sb, sc, p) else { continue };
            let eps = -1e-4;
            if l0 >= eps && l1 >= eps && l2 >= eps {
                let r = ra * l0 + rb * l1 + rc * l2;
                return r.is_finite().then_some(r);
            }
        }
        None
    }

    /// The outline as the pins bend it (the rest path itself, shared, at rest).
    pub fn outline(&self) -> Arc<Path> {
        if self.at_rest() {
            return self.rest.clone();
        }
        match self.bent() {
            Some(b) => Arc::new(bend(&self.rest, &b)),
            None => self.rest.clone(),
        }
    }

    /// Whether `path` is the outline this warp last gave its shape.
    pub fn shows(&self, path: &Path) -> bool {
        fingerprint(path) == self.shown
    }

    /// The warp of `path` (the shape as it now is, edited since by another tool), its pins kept
    /// where they show: they are its new rest, so nothing moves.
    pub fn rebased(&self, path: Arc<Path>) -> Warp {
        let mut w = Warp::new(path, self.resolution);
        w.pins = self.pins.iter().map(|p| Pin { id: p.id, rest: p.at, at: p.at }).collect();
        w
    }

    /// This warp's pins blended `t` of the way to `o`'s (pins pair by id; one without a partner
    /// holds). `None` unless both bend the same artwork on the same mesh.
    pub fn lerp(&self, o: &Warp, t: f32) -> Option<Warp> {
        if !(crate::arc_eq(&self.rest, &o.rest) && self.resolution == o.resolution) {
            return None;
        }
        let mut w = self.clone();
        for p in &mut w.pins {
            if let Some(q) = o.pin(p.id) {
                p.at = p.at.lerp(q.at, t);
            }
        }
        Some(w)
    }

    /// Repairs a loaded warp: the resolution in range, finite pins (one per id, at most
    /// [`MAX_PINS`]). `false` when nothing usable is left (no extent, too many points): the
    /// shape keeps the outline it shows.
    pub fn sanitize(&mut self) -> bool {
        if self.grid().is_none() || anchor_count(&self.rest) > MAX_WARP_POINTS {
            return false;
        }
        self.resolution = self.resolution.clamp(MIN_RESOLUTION, MAX_RESOLUTION);
        let near = |v: Vec2| v.is_finite() && v.x.abs() <= MAX_EXTENT && v.y.abs() <= MAX_EXTENT;
        let mut seen = Vec::with_capacity(self.pins.len());
        self.pins.retain(|p| {
            let keep = near(p.rest) && near(p.at) && !seen.contains(&p.id);
            seen.push(p.id);
            keep
        });
        self.pins.truncate(MAX_PINS);
        true
    }
}

impl Shape {
    /// The shape's warp while its outline is still the one the warp gave it.
    pub fn live_warp(&self) -> Option<&Warp> {
        self.warp.as_deref().filter(|w| w.shows(&self.path))
    }

    /// Sets the shape's warp to `w` and its outline to what `w` bends it into.
    pub fn set_warp(&mut self, mut w: Warp) {
        let path = w.outline();
        w.shown = fingerprint(&path);
        self.path = path;
        self.warp = Some(Arc::new(w));
    }
}

/// Splits cubic `(p0, c1, c2, p3)` at `t`.
fn split_cubic(p0: Vec2, c1: Vec2, c2: Vec2, p3: Vec2, t: f32) -> ([Vec2; 4], [Vec2; 4]) {
    let (a, b, c) = (p0.lerp(c1, t), c1.lerp(c2, t), c2.lerp(p3, t));
    let (d, e) = (a.lerp(b, t), b.lerp(c, t));
    let f = d.lerp(e, t);
    ([p0, a, d, f], [f, e, c, p3])
}

fn split_quad(p0: Vec2, c: Vec2, p2: Vec2, t: f32) -> ([Vec2; 3], [Vec2; 3]) {
    let (a, b) = (p0.lerp(c, t), c.lerp(p2, t));
    let m = a.lerp(b, t);
    ([p0, a, m], [m, b, p2])
}

/// How many pieces a curve with control polygon length `len` is split into.
fn pieces(len: f32, step: f32) -> usize {
    let n = (len / step).ceil();
    if n.is_finite() { (n as usize).clamp(1, MAX_SPLIT) } else { 1 }
}

/// `rest` bent through the mesh (see the module docs).
fn bend(rest: &Path, b: &Bent) -> Path {
    let g = &b.grid;
    let step = (((g.max.x - g.min.x) / g.nx as f32).min((g.max.y - g.min.y) / g.ny as f32) * 0.5).max(1e-3);
    // Splitting is skipped past the command budget (each anchor still maps through the mesh).
    let split = rest.cmds.len().saturating_mul(MAX_SPLIT) <= MAX_BENT_CMDS || estimate(rest, g, step) <= MAX_BENT_CMDS;
    let mut out: Vec<PathCmd> = Vec::with_capacity(rest.cmds.len() * 4);
    let mut ts = Vec::new();
    let mut cur = Vec2::ZERO;
    let mut start = Vec2::ZERO;
    // The last anchor's triangle: a cubic's first handle moves with it.
    let mut cur_tri = g.locate(cur);
    let line = |out: &mut Vec<PathCmd>, a: Vec2, p: Vec2, ts: &mut Vec<f32>| {
        if split {
            g.crossings(a, p, ts);
            for &t in ts.iter() {
                out.push(PathCmd::LineTo(b.map(a.lerp(p, t)).0));
            }
        }
        let (q, tri) = b.map(p);
        out.push(PathCmd::LineTo(q));
        tri
    };
    for c in &rest.cmds {
        match *c {
            PathCmd::MoveTo(p) => {
                let (q, tri) = b.map(p);
                out.push(PathCmd::MoveTo(q));
                (cur, start, cur_tri) = (p, p, tri);
            }
            PathCmd::LineTo(p) => {
                cur_tri = line(&mut out, cur, p, &mut ts);
                cur = p;
            }
            PathCmd::QuadTo(c, p) => {
                let n = if split { pieces(cur.distance(c) + c.distance(p), step) } else { 1 };
                let mut rest_q = [cur, c, p];
                for k in 0..n {
                    let left = n - k;
                    let (piece, tail) = if left > 1 { split_quad(rest_q[0], rest_q[1], rest_q[2], 1.0 / left as f32) } else { (rest_q, rest_q) };
                    let (q, tri) = b.map(piece[2]);
                    out.push(PathCmd::QuadTo(b.map(piece[1]).0, q));
                    cur_tri = tri;
                    rest_q = tail;
                }
                cur = p;
            }
            PathCmd::CubicTo(c1, c2, p) => {
                let n = if split { pieces(cur.distance(c1) + c1.distance(c2) + c2.distance(p), step) } else { 1 };
                let mut rest_c = [cur, c1, c2, p];
                for k in 0..n {
                    let left = n - k;
                    let (piece, tail) = if left > 1 { split_cubic(rest_c[0], rest_c[1], rest_c[2], rest_c[3], 1.0 / left as f32) } else { (rest_c, rest_c) };
                    let (q, tri) = b.map(piece[3]);
                    out.push(PathCmd::CubicTo(b.through(cur_tri, piece[1]), b.through(tri, piece[2]), q));
                    cur_tri = tri;
                    rest_c = tail;
                }
                cur = p;
            }
            PathCmd::Close => {
                // The closing edge is a straight line too: split it like one.
                if cur != start {
                    if split {
                        g.crossings(cur, start, &mut ts);
                        for &t in &ts {
                            out.push(PathCmd::LineTo(b.map(cur.lerp(start, t)).0));
                        }
                    }
                    let (q, tri) = b.map(start);
                    out.push(PathCmd::LineTo(q));
                    cur_tri = tri;
                }
                out.push(PathCmd::Close);
                cur = start;
            }
        }
    }
    Path { cmds: out }
}

/// A cheap upper bound of the bent outline's commands.
fn estimate(rest: &Path, g: &Grid, step: f32) -> usize {
    let lines = (g.nx + g.ny) as usize * 2 + 1;
    let mut cur = Vec2::ZERO;
    let mut n = 0usize;
    for c in &rest.cmds {
        n = n.saturating_add(match *c {
            PathCmd::MoveTo(p) => {
                cur = p;
                1
            }
            PathCmd::LineTo(p) => {
                cur = p;
                lines
            }
            PathCmd::QuadTo(a, p) => {
                let k = pieces(cur.distance(a) + a.distance(p), step);
                cur = p;
                k
            }
            PathCmd::CubicTo(a, b, p) => {
                let k = pieces(cur.distance(a) + a.distance(b) + b.distance(p), step);
                cur = p;
                k
            }
            PathCmd::Close => lines + 1,
        });
    }
    n
}

#[cfg(test)]
#[path = "warp_tests.rs"]
mod tests;
