//! Tile binning: the CPU half of the GPU stage (ADR 0014).
//!
//! The surface is cut into 16 × 16 pixel tiles. Every fill's edges are split at tile rows and
//! columns; each piece lands in the one tile it crosses. What a piece contributes to the tiles
//! to its right is a plain winding change per pixel row (it lies wholly to their left), so it
//! is summed into a **backdrop** of 16 values, one per pixel row, carried across the tile row
//! as in the public sparse-strips and Vello designs. A tile of a fill then needs only its own
//! pieces and its backdrop: the fine shader computes each pixel's exact area coverage from
//! those, the same integral the CPU oracle's accumulation buffer computes (ADR 0002). Tiles
//! with neither (outside the fill) get no entry; interior tiles get a backdrop and no pieces.
//!
//! Each tile then lists its entries in drawing order, so the shader composites every pixel in
//! one pass, front to back in the display list's order.

use anicraft_geom::{FillRule, Vec2};
use anicraft_render::Fill;
use anicraft_render::paint::Shader;
use rayon::prelude::*;

/// A `vec4<f32>` as the shader sees it.
type Vec4 = [f32; 4];

/// Tile side in pixels (the shader's workgroup is one tile, one invocation per pixel).
pub const TILE: u32 = 16;
const T: f32 = TILE as f32;
/// "No backdrop" in an entry.
pub const NONE: u32 = u32::MAX;
/// `vec4`s of paint data per fill.
pub const PAINT_VEC4S: usize = 4;
/// Gradient table entries (the oracle's LUT size).
pub const RAMP: usize = 256;

/// Paint kinds in an op header.
const SOLID: u32 = 0;
const LINEAR: u32 = 1;
const RADIAL: u32 = 2;

/// A scene ready for the fine shader.
///
/// * `index` (u32): `[start, count]` per tile into the entry list; then each entry as
///   `[op, first piece, pieces, backdrop]` (the backdrop's first `vec4` in `geo`, or [`NONE`]);
///   then each op's header `[kind, fill rule, ramp start in paints, 0]`.
/// * `geo` (`vec4<f32>`): pieces `(x0, y0, x1, y1)` relative to their tile's top-left, in the
///   edge's direction; then backdrops, four `vec4`s (16 rows) each.
/// * `paints` (`vec4<f32>`): per op its colour, then the inverse paint transform and gradient
///   parameters; then the gradient tables (256 premultiplied colours each).
pub struct Binned {
    pub tiles_x: u32,
    pub tiles_y: u32,
    pub index: Vec<u32>,
    pub entries_at: u32,
    pub ops_at: u32,
    pub geo: Vec<[f32; 4]>,
    pub paints: Vec<[f32; 4]>,
    pub entries: usize,
    pub pieces: usize,
}

/// A chunk of fills' share of the scene (binned in parallel; chunking keeps allocations few).
#[derive(Default)]
struct Local {
    /// `[op, tile, first piece, pieces, backdrop or NONE]`, piece and backdrop indices local.
    entries: Vec<[u32; 5]>,
    pieces: Vec<[f32; 4]>,
    /// Backdrops, four `vec4`s (16 pixel rows) each.
    backdrops: Vec<[f32; 4]>,
}

/// An edge piece inside one tile row, in absolute pixels; `col` is the tile column relative to
/// the fill's first one, or −1 for the part left of the fill's tiles (backdrop only).
#[derive(Clone, Copy)]
struct Piece {
    row: u32,
    col: i32,
    seg: [f32; 4],
}

/// Fills binned per parallel task.
const CHUNK: usize = 16;

/// Bins `fills` for a `width × height` surface.
pub fn bin(fills: &[Fill], width: u32, height: u32) -> Binned {
    let tiles_x = width.div_ceil(TILE);
    let tiles_y = height.div_ceil(TILE);
    let (tx, ty) = (tiles_x as usize, tiles_y as usize);
    // Phase 1, parallel over chunks of fills: pieces, backdrops and entries; each chunk's
    // entries then sorted (stably, so drawing order holds) by tile row, with row offsets.
    let locals: Vec<(Local, Vec<u32>)> = fills
        .par_chunks(CHUNK)
        .enumerate()
        .map(|(c, chunk)| {
            let mut out = Local::default();
            let mut scratch = Vec::new();
            for (i, f) in chunk.iter().enumerate() {
                bin_fill(f, (c * CHUNK + i) as u32, tiles_x, tiles_y, &mut out, &mut scratch);
            }
            out.entries.sort_by_key(|e| e[1] / tiles_x.max(1));
            let mut rows = vec![0u32; ty + 1];
            for e in &out.entries {
                if let Some(r) = rows.get_mut((e[1] / tiles_x.max(1)) as usize + 1) {
                    *r += 1;
                }
            }
            for r in 1..rows.len() {
                rows[r] += rows[r - 1];
            }
            (out, rows)
        })
        .collect();

    // Where each chunk's pieces and backdrops go in `geo`, and each tile row's entries.
    let pieces: usize = locals.iter().map(|(l, _)| l.pieces.len()).sum();
    let mut piece_at = Vec::with_capacity(locals.len());
    let mut bd_at = Vec::with_capacity(locals.len());
    let (mut p, mut b) = (0usize, pieces);
    for (l, _) in &locals {
        piece_at.push(p as u32);
        bd_at.push(b as u32);
        p += l.pieces.len();
        b += l.backdrops.len();
    }
    let mut row_at = vec![0usize; ty + 1];
    for r in 0..ty {
        let n: usize = locals.iter().map(|(_, rows)| (rows[r + 1] - rows[r]) as usize).sum();
        row_at[r + 1] = row_at[r] + n;
    }
    let total = row_at[ty];
    let entries_at = 2 * tx * ty;
    let ops_at = entries_at + 4 * total;
    let mut index = vec![0u32; ops_at + 4 * fills.len()];

    // Phase 2, parallel over tile rows: each row owns its tiles' headers and its entries.
    {
        let (headers, rest) = index.split_at_mut(entries_at);
        let (mut entries, _) = rest.split_at_mut(4 * total);
        let mut row_entries: Vec<&mut [u32]> = Vec::with_capacity(ty);
        for r in 0..ty {
            let (mine, after) = std::mem::take(&mut entries).split_at_mut(4 * (row_at[r + 1] - row_at[r]));
            row_entries.push(mine);
            entries = after;
        }
        headers.par_chunks_mut(2 * tx.max(1)).zip(row_entries).enumerate().for_each(|(r, (head, out))| {
            let first_tile = (r * tx) as u32;
            let mut count = vec![0u32; tx];
            let slices = || locals.iter().enumerate().map(|(c, (l, rows))| (c, l.entries.get(rows[r] as usize..rows[r + 1] as usize).unwrap_or(&[])));
            for (_, es) in slices() {
                for e in es {
                    if let Some(n) = count.get_mut((e[1] - first_tile) as usize) {
                        *n += 1;
                    }
                }
            }
            let mut cursor = vec![0u32; tx];
            let mut run = 0u32;
            for (t, &n) in count.iter().enumerate() {
                if let Some(h) = head.get_mut(2 * t..2 * t + 2) {
                    h.copy_from_slice(&[row_at[r] as u32 + run, n]);
                }
                cursor[t] = run;
                run += n;
            }
            for (c, es) in slices() {
                for &[op, tile, s0, n, bd] in es {
                    let Some(cur) = cursor.get_mut((tile - first_tile) as usize) else { continue };
                    let at = 4 * *cur as usize;
                    *cur += 1;
                    let bd = if bd == NONE { NONE } else { bd_at[c] + bd };
                    if let Some(slot) = out.get_mut(at..at + 4) {
                        slot.copy_from_slice(&[op, piece_at[c] + s0, n, bd]);
                    }
                }
            }
        });
    }

    // Pieces, then backdrops, copied in parallel into their places.
    let mut geo: Vec<[f32; 4]> = vec![[0.0; 4]; b];
    {
        let (mut piece_dst, mut bd_dst) = geo.split_at_mut(pieces);
        let mut jobs: Vec<(&mut [Vec4], &[Vec4])> = Vec::with_capacity(2 * locals.len());
        for (l, _) in &locals {
            let (a, rest) = std::mem::take(&mut piece_dst).split_at_mut(l.pieces.len());
            jobs.push((a, &l.pieces));
            piece_dst = rest;
            let (a, rest) = std::mem::take(&mut bd_dst).split_at_mut(l.backdrops.len());
            jobs.push((a, &l.backdrops));
            bd_dst = rest;
        }
        jobs.into_par_iter().for_each(|(d, s)| d.copy_from_slice(s));
    }
    // Tens of thousands of small buffers: free them on the pool, off the frame's path.
    rayon::spawn(move || drop(locals));

    // Paints: a fixed record per op, then the gradient tables.
    let mut paints: Vec<[f32; 4]> = Vec::with_capacity(PAINT_VEC4S * fills.len());
    let mut ramps: Vec<[f32; 4]> = Vec::new();
    let ramps_at = (PAINT_VEC4S * fills.len()) as u32;
    for (op, f) in fills.iter().enumerate() {
        let rule = match f.rule {
            FillRule::NonZero => 0,
            FillRule::EvenOdd => 1,
        };
        let ramp = ramps_at + ramps.len() as u32;
        let (kind, rec) = match &f.shader {
            Shader::Solid(c) => (SOLID, [*c, [0.0; 4], [0.0; 4], [0.0; 4]]),
            Shader::Linear { inv, start, dir, lut } => {
                ramps.extend_from_slice(&lut[..]);
                let m = inv.m;
                (LINEAR, [[0.0; 4], [m[0], m[1], m[2], m[3]], [m[4], m[5], start.x, start.y], [dir.x, dir.y, 0.0, 0.0]])
            }
            Shader::Radial { inv, center, inv_radius, lut } => {
                ramps.extend_from_slice(&lut[..]);
                let m = inv.m;
                (RADIAL, [[0.0; 4], [m[0], m[1], m[2], m[3]], [m[4], m[5], center.x, center.y], [0.0, 0.0, *inv_radius, 0.0]])
            }
            // `plain_fills` never hands these over; draw nothing rather than guess.
            Shader::Bitmap { .. } | Shader::Image { .. } => (SOLID, [[0.0; 4]; 4]),
        };
        paints.extend_from_slice(&rec);
        let at = ops_at + 4 * op;
        if let Some(slot) = index.get_mut(at..at + 4) {
            slot.copy_from_slice(&[kind, rule, ramp, 0]);
        }
    }
    paints.extend_from_slice(&ramps);

    Binned { tiles_x, tiles_y, index, entries_at: entries_at as u32, ops_at: ops_at as u32, geo, paints, entries: total, pieces }
}

/// Splits one fill's edges into tile pieces and walks each tile row for backdrops.
fn bin_fill(f: &Fill, op: u32, tiles_x: u32, tiles_y: u32, out: &mut Local, pieces: &mut Vec<Piece>) {
    let b = f.bounds;
    if b.is_empty() {
        return;
    }
    let tx0 = b.x0.max(0) as u32 / TILE;
    let tx1 = (b.x1.max(0) as u32).div_ceil(TILE).min(tiles_x);
    let ty0 = b.y0.max(0) as u32 / TILE;
    let ty1 = (b.y1.max(0) as u32).div_ceil(TILE).min(tiles_y);
    if tx1 <= tx0 || ty1 <= ty0 {
        return;
    }
    let ncols = (tx1 - tx0) as i32;
    let range = Range { ylo: (ty0 * TILE) as f32, yhi: (ty1 * TILE) as f32, xlo: (tx0 * TILE) as f32, xhi: (tx1 * TILE) as f32, ncols };
    pieces.clear();
    for poly in &f.polys {
        let n = poly.len();
        if n < 2 {
            continue;
        }
        for i in 0..n {
            let (Some(&a), Some(&c)) = (poly.get(i), poly.get((i + 1) % n)) else { continue };
            split_edge(a, c, &range, pieces);
        }
    }
    if pieces.is_empty() {
        return;
    }
    // Unstable (no allocation) yet deterministic: the order within a tile only affects rounding.
    pieces.sort_unstable_by_key(|p| (p.row, p.col));

    let mut i = 0usize;
    while let Some(first) = pieces.get(i) {
        let row = first.row;
        let row_y0 = ((ty0 + row) * TILE) as f32;
        let mut wind = [0.0f32; 16];
        // Parts left of the fill's tiles only shift the winding.
        while let Some(p) = pieces.get(i).filter(|p| p.row == row && p.col < 0) {
            add_rows(&mut wind, &p.seg, row_y0);
            i += 1;
        }
        let mut last_bd: Option<(u32, [f32; 16])> = None;
        for col in 0..ncols {
            let start = i;
            while pieces.get(i).is_some_and(|p| p.row == row && p.col == col) {
                i += 1;
            }
            let here = pieces.get(start..i).unwrap_or(&[]);
            let empty_wind = wind.iter().all(|w| w.abs() < 1e-6);
            if empty_wind {
                wind = [0.0; 16];
            }
            if here.is_empty() && empty_wind {
                // Nothing here; and nothing further right either once the row's pieces are used.
                if !pieces.get(i).is_some_and(|p| p.row == row) {
                    break;
                }
                continue;
            }
            let tile_x = tx0 + col as u32;
            let tile = (ty0 + row) * tiles_x + tile_x;
            let bd = if empty_wind {
                NONE
            } else {
                match last_bd {
                    Some((k, w)) if w == wind => k,
                    _ => {
                        let k = out.backdrops.len() as u32;
                        out.backdrops.extend_from_slice(wind.as_chunks::<4>().0);
                        last_bd = Some((k, wind));
                        k
                    }
                }
            };
            let (ox, oy) = ((tile_x * TILE) as f32, row_y0);
            let s0 = out.pieces.len() as u32;
            for p in here {
                out.pieces.push([p.seg[0] - ox, p.seg[1] - oy, p.seg[2] - ox, p.seg[3] - oy]);
                add_rows(&mut wind, &p.seg, row_y0);
            }
            out.entries.push([op, tile, s0, here.len() as u32, bd]);
        }
        // Skip whatever is left of this row (pieces right of the range never get here).
        while pieces.get(i).is_some_and(|p| p.row == row) {
            i += 1;
        }
    }
}

/// The fill's tile range, in pixels and tiles.
struct Range {
    ylo: f32,
    yhi: f32,
    xlo: f32,
    xhi: f32,
    ncols: i32,
}

/// Adds the winding a piece gives everything to its right, per pixel row of its tile row.
fn add_rows(wind: &mut [f32; 16], seg: &[f32; 4], row_y0: f32) {
    let (ya, yb, dir) = if seg[1] < seg[3] { (seg[1], seg[3], 1.0) } else { (seg[3], seg[1], -1.0) };
    let r0 = ((ya - row_y0).floor().max(0.0)) as usize;
    let r1 = ((yb - row_y0).ceil().min(T)) as usize;
    for r in r0..r1 {
        let lo = row_y0 + r as f32;
        let d = yb.min(lo + 1.0) - ya.max(lo);
        if d > 0.0
            && let Some(w) = wind.get_mut(r)
        {
            *w += dir * d;
        }
    }
}

/// Splits edge `a → b` at tile rows and columns inside the fill's range.
fn split_edge(a: Vec2, b: Vec2, r: &Range, out: &mut Vec<Piece>) {
    if !(a.is_finite() && b.is_finite()) || a.y == b.y {
        return;
    }
    // Work top to bottom; restore the direction when storing.
    let (p, q, down) = if a.y < b.y { (a, b, true) } else { (b, a, false) };
    let y0 = p.y.max(r.ylo);
    let y1 = q.y.min(r.yhi);
    if y1 <= y0 {
        return;
    }
    let dxdy = (q.x - p.x) / (q.y - p.y);
    let x_at = |y: f32| p.x + (y - p.y) * dxdy;
    let first = ((y0 - r.ylo) / T).floor().max(0.0) as u32;
    let last = ((y1 - r.ylo) / T).ceil().max(0.0) as u32;
    for row in first..last {
        let ry0 = r.ylo + (row * TILE) as f32;
        let ys = y0.max(ry0);
        let ye = y1.min(ry0 + T);
        if ye <= ys {
            continue;
        }
        let (xs, xe) = (x_at(ys), x_at(ye));
        split_row(row, Vec2::new(xs, ys), Vec2::new(xe, ye), down, r, out);
    }
}

/// Splits a piece inside one tile row at the fill's left and right edges and at tile columns.
fn split_row(row: u32, s: Vec2, e: Vec2, down: bool, r: &Range, out: &mut Vec<Piece>) {
    let mut push = |col: i32, u: Vec2, v: Vec2| {
        if v.y <= u.y {
            return;
        }
        let seg = if down { [u.x, u.y, v.x, v.y] } else { [v.x, v.y, u.x, u.y] };
        out.push(Piece { row, col, seg });
    };
    // Points where the piece crosses a column boundary (or the range's edges), top to bottom.
    let dx = e.x - s.x;
    let at_x = |x: f32| -> Vec2 {
        let t = ((x - s.x) / dx).clamp(0.0, 1.0);
        Vec2::new(x, s.y + (e.y - s.y) * t)
    };
    let col_of = |x: f32| ((x - r.xlo) / T).floor() as i32;
    let (lo, hi) = (s.x.min(e.x), s.x.max(e.x));
    if hi <= r.xlo {
        push(-1, s, e);
        return;
    }
    if lo >= r.xhi {
        return; // wholly right of the fill's tiles: it covers none of their pixels
    }
    // Most pieces (flattened curves are short) lie inside one tile column.
    let cl = col_of(lo);
    if dx == 0.0 || (lo >= r.xlo && hi < r.xhi && cl == col_of(hi)) {
        push(cl.clamp(0, r.ncols - 1), s, e);
        return;
    }
    // Column boundaries strictly inside (lo, hi) (the range's left and right edges included),
    // visited in the piece's direction of travel.
    let kmin = (((lo - r.xlo) / T).floor() as i32 + 1).max(0);
    let kmax = (((hi - r.xlo) / T).ceil() as i32 - 1).min(r.ncols);
    let count = (kmax - kmin + 1).max(0);
    let mut prev = s;
    for j in 0..=count {
        let next = if j == count {
            e
        } else {
            let k = if dx > 0.0 { kmin + j } else { kmax - j };
            let x = r.xlo + k as f32 * T;
            if !(x > lo && x < hi) {
                continue;
            }
            at_x(x)
        };
        let mid = 0.5 * (prev.x + next.x);
        if mid < r.xlo {
            push(-1, prev, next);
        } else if mid < r.xhi {
            push(col_of(mid).clamp(0, r.ncols - 1), prev, next);
        }
        prev = next;
    }
}

/// Coverage of pixel `(px, py)` (tile-relative) from one piece: the same integral the oracle's
/// accumulation buffer computes. Used by tests to check the binning on the CPU.
#[cfg(test)]
pub(crate) fn piece_cover(s: [f32; 4], px: f32, py: f32) -> f32 {
    let (ylo, yhi) = (s[1].min(s[3]), s[1].max(s[3]));
    let (ya, yb) = (ylo.max(py), yhi.min(py + 1.0));
    if yb <= ya {
        return 0.0;
    }
    let dxdy = (s[2] - s[0]) / (s[3] - s[1]);
    let xa = s[0] + (ya - s[1]) * dxdy;
    let xb = s[0] + (yb - s[1]) * dxdy;
    let dir = if s[3] > s[1] { 1.0 } else { -1.0 };
    dir * (yb - ya) * avg_clamp(px + 1.0 - xa, px + 1.0 - xb)
}

/// Mean of `clamp(u, 0, 1)` for `u` running linearly from `u0` to `u1` (no cancellation when
/// the two are close).
#[cfg(test)]
fn avg_clamp(u0: f32, u1: f32) -> f32 {
    let (lo, hi) = (u0.min(u1), u0.max(u1));
    if hi <= 0.0 {
        return 0.0;
    }
    if lo >= 1.0 {
        return 1.0;
    }
    let w = hi - lo;
    if w < 1e-7 {
        return lo.clamp(0.0, 1.0);
    }
    let (a, b) = (lo.max(0.0), hi.min(1.0));
    ((b - a) * (b + a) * 0.5 + (hi - 1.0).max(0.0)) / w
}

/// The colour a premultiplied record holds (tests).
#[cfg(test)]
pub(crate) fn solid_of(b: &Binned, op: usize) -> anicraft_render::paint::Premul {
    b.paints.get(PAINT_VEC4S * op).copied().unwrap_or([0.0; 4])
}
