//! Tile binning: the CPU half of the GPU stage (ADR 0019, ADR 0029).
//!
//! The surface is cut into 16 × 16 pixel tiles. Every fill's edges are split at tile rows and
//! columns; each piece lands in the one tile it crosses. Everything left of a tile reaches it
//! as the fill's winding number along the tile's left edge (ADR 0048), which is
//!
//! * its **backdrop**, the winding number just below the tile's top-left corner: an integer,
//!   the sum of the directions of the pieces further left that start on the tile row's top
//!   edge (they are the edges crossing it), plus
//! * a step wherever the outline crosses the tile's left edge, kept as one more piece: a
//!   vertical line on that edge from the crossing down to the tile's bottom, upwards where the
//!   outline crosses rightwards (the winding there drops by one), downwards where it crosses
//!   leftwards.
//!
//! These are the backdrops and "y-edges" of Vello's tiles (its public design notes). A tile of
//! a fill then needs only its own pieces and its backdrop: the fine shader computes each
//! pixel's exact area coverage from those, the same integral the CPU oracle's accumulation
//! buffer computes (ADR 0002). Tiles with neither (outside the fill) get no entry; interior
//! tiles get a backdrop and no pieces. Backdrops are integers, so they add up the same in any
//! order, and the GPU computes the same lists ([`crate::gpu_binning`]).
//!
//! A **hairline** (outline mode, ADR 0048) is binned by distance instead: each segment, whole,
//! into every tile with a pixel centre within its reach; the shader takes the nearest.
//!
//! Each tile then lists its entries in drawing order, so the shader composites every pixel in
//! one pass, front to back in the display list's order. Masks and groups (ADR 0029) are more
//! entries in the same list: before a clipped fill, the clip's coverage is computed from its
//! artwork's entries in this tile (one level per clip in its chain, the levels multiplied); and
//! a group opens before its first fill in the tile and closes (blending into what is below at
//! its alpha and in its blend mode) when its run ends, as the oracle's band compositor does per
//! band.
//!
//! Bitmap fills (ADR 0032) take one of [`MAX_IMAGES`] texture slots per dispatch, one per
//! distinct image; a filtered group's layer is drawn and filtered by the GPU before the pass and
//! copied into the paint buffer after the gradient tables, where its fill reads it.

use anicraft_doc::{BlendMode, Pixels};
use anicraft_geom::{FillRule, Vec2};
use anicraft_render::paint::Shader;
use anicraft_render::{Fill, NeedsFull, Scene};
use rayon::prelude::*;
use std::sync::Arc;

/// A `vec4<f32>` as the shader sees it.
type Vec4 = [f32; 4];

/// Tile side in pixels (the shader's workgroup is one tile, one invocation per pixel).
pub const TILE: u32 = 16;
const T: f32 = TILE as f32;
/// "No clip" or "no group" in the tables.
pub const NONE: u32 = u32::MAX;
/// Fill rule code of a hairline in an op header (`0` non-zero, `1` even-odd).
pub const HAIRLINE: u32 = 2;
/// How far around a hairline segment its tiles are taken (`bin.wgsl` › `M`): a pixel centre
/// within its reach is within `REACH` of a point of it, so the tile holding that centre holds a
/// point within `REACH` of the segment's box; a quarter pixel more absorbs rounding.
pub(crate) const HAIRLINE_MARGIN: f32 = anicraft_render::hairline::REACH + 0.25;
/// `vec4`s of paint data per fill.
pub const PAINT_VEC4S: usize = 4;
/// Gradient table entries (the oracle's LUT size).
pub const RAMP: usize = 256;
/// Deepest nesting of groups the shader's per-pixel stack holds (`fine.wgsl`).
pub const MAX_GROUP_DEPTH: usize = 8;
/// Longest chain of nested clips the binning expands (each level is one more pass over its
/// artwork per clip switch).
pub const MAX_CLIP_DEPTH: usize = 16;

/// Distinct bitmaps one dispatch samples (texture slots in `fine.wgsl`; every device has at
/// least 16 sampled textures per shader stage).
pub const MAX_IMAGES: usize = 16;

/// Paint kinds in an op header.
const SOLID: u32 = 0;
const LINEAR: u32 = 1;
const RADIAL: u32 = 2;
const BITMAP: u32 = 3;
const IMAGE: u32 = 4;

/// Op header flag: the fill is drawn inside the current clip.
pub const CLIPPED: u32 = 1;
/// Op header flag: the op is clip artwork (its coverage joins the current level's union; in a
/// soft mask its paint composites into the level's colour).
pub const ART: u32 = 2;

/// An entry whose first word has this bit is a control entry, not a fill.
pub const CTRL: u32 = 0x8000_0000;
/// `[OPEN, group, 0, 0]`: push the colour so far and continue on transparent.
pub const OPEN: u32 = CTRL | 1;
/// `[CLOSE, alpha bits, blend mode, 0]`: blend the group into the colour below it.
pub const CLOSE: u32 = CTRL | 2;
/// The clip coverage starts at 1 (a new clip's chain follows).
pub const MASK_RESET: u32 = CTRL | 3;
/// `[LEVEL_BEGIN, mask mode, 0, 0]`: one level of the clip chain starts: its union of artwork
/// coverage starts at 0, or for a soft mask ([`mask_code`]) its artwork's colour starts
/// transparent.
pub const LEVEL_BEGIN: u32 = CTRL | 4;
/// The level ends: the clip coverage is multiplied by its union (or by the soft mask's
/// opacity or luminance, ADR 0070).
pub const LEVEL_END: u32 = CTRL | 5;

/// Mask modes as the shader numbers them (`fine.wgsl` › `LEVEL_END`).
pub fn mask_code(m: anicraft_doc::MaskMode) -> u32 {
    match m {
        anicraft_doc::MaskMode::Coverage => 0,
        anicraft_doc::MaskMode::Alpha => 1,
        anicraft_doc::MaskMode::Luminance => 2,
    }
}

/// Blend modes as the shader numbers them (`fine.wgsl` › `blend`).
pub fn blend_code(m: BlendMode) -> u32 {
    match m {
        BlendMode::Normal | BlendMode::Layer => 0,
        BlendMode::Multiply => 1,
        BlendMode::Screen => 2,
        BlendMode::Darken => 3,
        BlendMode::Lighten => 4,
        BlendMode::Difference => 5,
        BlendMode::Add => 6,
        BlendMode::Subtract => 7,
        BlendMode::Invert => 8,
        BlendMode::HardLight => 9,
        BlendMode::Overlay => 10,
        BlendMode::Alpha => 11,
        BlendMode::Erase => 12,
    }
}

/// A scene ready for the fine shader.
///
/// * `index` (u32): `[start, count]` per tile into the entry list; then each entry as
///   `[op, first piece, pieces, backdrop]` (the backdrop an `i32`), or a control entry ([`CTRL`]
///   set); then, at `ops_at`, each op's header `[kind, fill rule (or HAIRLINE), ramp start in
///   paints, flags]` ([`Binned::op_heads`]).
/// * `geo` (`vec4<f32>`): pieces `(x0, y0, x1, y1)` relative to their tile's top-left, in the
///   edge's direction (left-edge steps included); a hairline's are its segments, whole.
/// * `paints` (`vec4<f32>`): per op its colour, then the inverse paint transform and gradient,
///   bitmap or layer parameters; then the gradient tables (256 premultiplied colours each);
///   then (filled in on the GPU, [`Binned::layer_at`]) the filtered layers' pixels.
///
/// Ops are the scene's fills in drawing order, then every clip's artwork, clip by clip.
pub struct Binned {
    pub tiles_x: u32,
    pub tiles_y: u32,
    pub index: Vec<u32>,
    pub entries_at: u32,
    pub ops_at: u32,
    /// The op headers, which go at `ops_at` in the index.
    pub op_heads: Vec<u32>,
    pub geo: Vec<[f32; 4]>,
    pub paints: Vec<[f32; 4]>,
    /// The paint buffer's whole length in `vec4`s: `paints`, then room for the layers.
    pub paints_len: usize,
    /// Per scene layer, where its pixels go in the paint buffer (`vec4`s); `None` when no fill
    /// shows it (then it isn't drawn).
    pub layer_at: Vec<Option<u32>>,
    /// The bitmaps, in texture-slot order.
    pub images: Vec<Arc<Pixels>>,
    /// Entries (fills, artwork and control) and edge pieces in all tiles.
    pub entries: usize,
    pub pieces: usize,
    /// Pieces plus entries the fine pass walks, summed over tiles (each for 256 pixels): what
    /// bounds how long the dispatch runs.
    pub work: u64,
    /// Binned on the GPU: `index` holds only the tile headers (`[start, 0]`, room for every
    /// tile the fills' bounds cover), `geo` is empty, and the GPU computes the entries and
    /// pieces from this plan (`entries` and `pieces` are then upper bounds).
    pub gpu: Option<crate::gpu_binning::Plan>,
}

/// A chunk of fills' share of the scene (binned in parallel; chunking keeps allocations few).
#[derive(Default)]
struct Local {
    /// `[op, tile, first piece, pieces, backdrop]`, piece indices local.
    entries: Vec<[u32; 5]>,
    pieces: Vec<[f32; 4]>,
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

/// What the per-tile streams need to know about the ops, clips and groups.
struct Tables<'a> {
    /// Fills come first in the op list; artwork after.
    fills: u32,
    /// Per fill: its clip and its group (or [`NONE`]).
    op_clip: Vec<u32>,
    op_group: Vec<u32>,
    /// Per clip: its chain, outermost first, and its artwork's op range.
    clip_chain: Vec<Vec<u32>>,
    art_ops: Vec<(u32, u32)>,
    scene: &'a Scene,
}

/// One tile row's entries, built in parallel and then copied into place.
struct RowOut {
    stream: Vec<[u32; 4]>,
    /// `[start, count]` per tile, `start` local to the row.
    heads: Vec<[u32; 2]>,
    work: u64,
}

/// Bins `scene` for a `width × height` surface. `Err` when its nesting is deeper than the
/// shader holds ([`MAX_GROUP_DEPTH`], [`MAX_CLIP_DEPTH`]) or it has more bitmaps than
/// [`MAX_IMAGES`]; the caller draws it on the CPU.
pub fn bin(scene: &Scene, width: u32, height: u32) -> Result<Binned, NeedsFull> {
    bin_with(scene, width, height, None)
}

/// [`bin`], or with `gpu`, a plan for binning on the GPU instead ([`Binned::gpu`]) when the
/// scene can take it and fits `gpu`'s limits ([`crate::gpu_binning`]).
pub fn bin_with(scene: &Scene, width: u32, height: u32, gpu: Option<&crate::gpu_binning::Limits>) -> Result<Binned, NeedsFull> {
    if scene.groups.iter().any(|g| g.chain.len() > MAX_GROUP_DEPTH) {
        return Err(NeedsFull::Groups);
    }
    let mut clip_chain: Vec<Vec<u32>> = Vec::with_capacity(scene.clips.len());
    for (i, c) in scene.clips.iter().enumerate() {
        let mut chain = c.parent.filter(|&p| (p as usize) < i).and_then(|p| clip_chain.get(p as usize)).cloned().unwrap_or_default();
        chain.push(i as u32);
        if chain.len() > MAX_CLIP_DEPTH {
            return Err(NeedsFull::Clips);
        }
        clip_chain.push(chain);
    }
    let mut ops: Vec<&Fill> = scene.fills.iter().collect();
    let mut art_ops = Vec::with_capacity(scene.clips.len());
    for c in &scene.clips {
        let a0 = ops.len() as u32;
        ops.extend(c.art.iter());
        art_ops.push((a0, ops.len() as u32));
    }
    if ops.len() >= CTRL as usize {
        return Err(NeedsFull::Groups);
    }
    let nf = scene.fills.len();
    let op_clip: Vec<u32> = scene.fills.iter().map(|f| f.clip.filter(|&c| (c as usize) < scene.clips.len()).unwrap_or(NONE)).collect();
    let op_group: Vec<u32> = scene.fills.iter().map(|f| f.group.filter(|&g| (g as usize) < scene.groups.len()).unwrap_or(NONE)).collect();
    let tables = Tables { fills: nf as u32, op_clip, op_group, clip_chain, art_ops, scene };

    let tiles_x = width.div_ceil(TILE);
    let tiles_y = height.div_ceil(TILE);
    // On the GPU when asked and the scene can take it (no fill in a mask or a group, for now):
    // then the fills alone are the ops (no mask artwork draws).
    let plan = gpu.filter(|_| scene.fills.iter().all(|f| f.clip.is_none() && f.group.is_none())).and_then(|lim| crate::gpu_binning::plan(&scene.fills, tiles_x, tiles_y, lim));
    let lists = match &plan {
        Some(p) => {
            ops.truncate(nf);
            Lists::on_gpu(p, tiles_x, tiles_y)
        }
        None => cpu_lists(&ops, &tables, tiles_x, tiles_y),
    };

    // Paints: a fixed record per op, then the gradient tables, then the layers' pixels.
    let mut paints: Vec<[f32; 4]> = Vec::with_capacity(PAINT_VEC4S * ops.len());
    let mut ramps: Vec<[f32; 4]> = Vec::new();
    let mut images: Vec<Arc<Pixels>> = Vec::new();
    let ramps_at = (PAINT_VEC4S * ops.len()) as u32;
    // Where each layer shown by a fill goes, after the tables (the tables' size is known first).
    // Gradients' ramps, and each bitmap's colour transform (two entries).
    let table_len: usize = ops
        .iter()
        .filter(|f| f.layer.is_none())
        .map(|f| match &f.shader {
            Shader::Linear { .. } | Shader::Radial { .. } => RAMP,
            Shader::Bitmap { pixels, .. } if pixels.width > 0 && pixels.height > 0 => 2,
            _ => 0,
        })
        .sum();
    let mut layer_at: Vec<Option<u32>> = vec![None; scene.layers.len()];
    let mut cursor = PAINT_VEC4S * ops.len() + table_len;
    for f in &ops {
        let Some(l) = f.layer else { continue };
        let (Some(slot), Some(layer)) = (layer_at.get_mut(l as usize), scene.layers.get(l as usize)) else { continue };
        if slot.is_none() {
            *slot = Some(cursor as u32);
            cursor += layer.bounds.width() * layer.bounds.height();
        }
    }
    if cursor >= u32::MAX as usize {
        return Err(NeedsFull::Filters);
    }
    let paints_len = cursor;
    let mut op_heads: Vec<u32> = Vec::with_capacity(4 * ops.len());
    let u = f32::from_bits;
    for (op, f) in ops.iter().enumerate() {
        let rule = match f.rule {
            _ if f.hairline => HAIRLINE,
            FillRule::NonZero => 0,
            FillRule::EvenOdd => 1,
        };
        let ramp = ramps_at + ramps.len() as u32;
        let layer = f.layer.and_then(|l| Some((layer_at.get(l as usize).copied().flatten()?, scene.layers.get(l as usize)?.bounds)));
        let (kind, rec) = match (&f.shader, layer) {
            // A layer's pixels at `at`, `w × h`, its top-left at (x0, y0) in surface pixels.
            (_, Some((at, b))) => (IMAGE, [[u(at), u(b.width() as u32), u(b.height() as u32), 0.0], [u(b.x0 as u32), u(b.y0 as u32), 0.0, 0.0], [0.0; 4], [0.0; 4]]),
            (_, None) if f.layer.is_some() => (SOLID, [[0.0; 4]; 4]),
            (Shader::Solid(c), _) => (SOLID, [*c, [0.0; 4], [0.0; 4], [0.0; 4]]),
            // A gradient's first vec4 holds what its colour slot doesn't need: the spread
            // (0 pad, 1 reflect, 2 repeat) and a radial gradient's focal point (ADR 0074).
            (Shader::Linear { inv, start, dir, spread, lut }, _) => {
                ramps.extend_from_slice(&lut[..]);
                let m = inv.m;
                (LINEAR, [[spread_code(*spread), 0.0, 0.0, 0.0], [m[0], m[1], m[2], m[3]], [m[4], m[5], start.x, start.y], [dir.x, dir.y, 0.0, 0.0]])
            }
            (Shader::Radial { inv, center, inv_radius, focal, spread, lut }, _) => {
                ramps.extend_from_slice(&lut[..]);
                let m = inv.m;
                (RADIAL, [[spread_code(*spread), *focal, 0.0, 0.0], [m[0], m[1], m[2], m[3]], [m[4], m[5], center.x, center.y], [0.0, 0.0, *inv_radius, 0.0]])
            }
            // The oracle's sample of a bitmap (crates/render/src/paint.rs › sample).
            (Shader::Bitmap { inv, pixels, repeat, fx, smooth }, _) => {
                if pixels.width == 0 || pixels.height == 0 {
                    (SOLID, [[0.0; 4]; 4])
                } else {
                    let slot = match images.iter().position(|p| Arc::ptr_eq(p, pixels)) {
                        Some(i) => i,
                        None if images.len() < MAX_IMAGES => {
                            images.push(pixels.clone());
                            images.len() - 1
                        }
                        None => return Err(NeedsFull::Bitmaps),
                    };
                    let m = inv.m;
                    // The colour transform (ADR 0056) rides in the ramp table: multiplies, then
                    // offsets, sanitized as the oracle applies them.
                    let fx = fx.sanitized();
                    ramps.extend_from_slice(&[fx.mul, fx.add]);
                    // Bit 0: repeat; bit 1: the nearest pixel instead of bilinear (no smoothing).
                    (BITMAP, [[u(slot as u32), u(u32::from(*repeat) | (u32::from(!*smooth) << 1)), u(pixels.width), u(pixels.height)], [m[0], m[1], m[2], m[3]], [m[4], m[5], 0.0, 0.0], [0.0; 4]])
                }
            }
            // `prepare_scene` never hands these over; draw nothing rather than guess.
            (Shader::Image { .. }, _) => (SOLID, [[0.0; 4]; 4]),
        };
        paints.extend_from_slice(&rec);
        let flags = if op >= nf {
            ART
        } else if tables.op_clip.get(op).is_some_and(|&c| c != NONE) {
            CLIPPED
        } else {
            0
        };
        op_heads.extend_from_slice(&[kind, rule, ramp, flags]);
    }
    paints.extend_from_slice(&ramps);
    if paints.len() != PAINT_VEC4S * ops.len() + table_len {
        // The tables came out another size than counted; never let a layer overlap them.
        return Err(NeedsFull::Filters);
    }

    let Lists { index, geo, entries_at, ops_at, entries, pieces, work } = lists;
    Ok(Binned { tiles_x, tiles_y, index, entries_at, ops_at, op_heads, geo, paints, paints_len, layer_at, images, entries, pieces, work, gpu: plan })
}

/// The tile lists (or, binned on the GPU, room for them).
struct Lists {
    index: Vec<u32>,
    geo: Vec<[f32; 4]>,
    entries_at: u32,
    ops_at: u32,
    entries: usize,
    pieces: usize,
    work: u64,
}

impl Lists {
    /// Headers `[start, 0]` with room for every tile its fills' bounds cover; the GPU fills in
    /// the entries and the counts. Entries and pieces are upper bounds.
    fn on_gpu(p: &crate::gpu_binning::Plan, tiles_x: u32, tiles_y: u32) -> Lists {
        let entries_at = 2 * tiles_x * tiles_y;
        let ops_at = entries_at + 4 * p.op_tiles;
        let work = u64::from(p.op_tiles) + u64::from(p.piece_cap);
        Lists { index: p.headers.clone(), geo: Vec::new(), entries_at, ops_at, entries: p.op_tiles as usize, pieces: p.piece_cap as usize, work }
    }
}

/// The tile lists, binned on the CPU (in parallel per chunk of fills, then per tile row).
fn cpu_lists(ops: &[&Fill], tables: &Tables, tiles_x: u32, tiles_y: u32) -> Lists {
    let (tx, ty) = (tiles_x as usize, tiles_y as usize);
    // Phase 1, parallel over chunks of ops: pieces, backdrops and entries; each chunk's
    // entries then sorted (stably, so drawing order holds) by tile row, with row offsets.
    let locals: Vec<(Local, Vec<u32>)> = ops
        .par_chunks(CHUNK)
        .enumerate()
        .map(|(c, chunk)| {
            let mut out = Local::default();
            let mut scratch = (Vec::new(), Track::default());
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

    // Where each chunk's pieces go in `geo`.
    let pieces: usize = locals.iter().map(|(l, _)| l.pieces.len()).sum();
    let mut piece_at = Vec::with_capacity(locals.len());
    let mut p = 0usize;
    for (l, _) in &locals {
        piece_at.push(p as u32);
        p += l.pieces.len();
    }

    // Phase 2, parallel over tile rows: each tile's stream of entries.
    let rows: Vec<RowOut> = (0..ty).into_par_iter().map(|r| row_streams(r, tx, &locals, &piece_at, tables)).collect();
    let mut row_at = vec![0usize; ty + 1];
    for (r, out) in rows.iter().enumerate() {
        row_at[r + 1] = row_at[r] + out.stream.len();
    }
    let total = row_at[ty];
    let work = rows.iter().map(|r| r.work).sum();
    let entries_at = 2 * tx * ty;
    let ops_at = entries_at + 4 * total;
    let mut index = vec![0u32; ops_at];
    // Phase 3, parallel over tile rows: headers and streams copied into place.
    {
        let (headers, rest) = index.split_at_mut(entries_at);
        let (mut entries, _) = rest.split_at_mut(4 * total);
        let mut row_entries: Vec<&mut [u32]> = Vec::with_capacity(ty);
        for r in 0..ty {
            let (mine, after) = std::mem::take(&mut entries).split_at_mut(4 * (row_at[r + 1] - row_at[r]));
            row_entries.push(mine);
            entries = after;
        }
        headers.par_chunks_mut(2 * tx.max(1)).zip(row_entries).zip(&rows).enumerate().for_each(|(r, ((head, out), row))| {
            for (h, &[s, n]) in head.as_chunks_mut::<2>().0.iter_mut().zip(&row.heads) {
                *h = [row_at[r] as u32 + s, n];
            }
            for (d, s) in out.as_chunks_mut::<4>().0.iter_mut().zip(&row.stream) {
                *d = *s;
            }
        });
    }
    // Pieces copied in parallel into their places.
    let mut geo: Vec<[f32; 4]> = vec![[0.0; 4]; pieces];
    {
        let mut piece_dst = &mut geo[..];
        let mut jobs: Vec<(&mut [Vec4], &[Vec4])> = Vec::with_capacity(locals.len());
        for (l, _) in &locals {
            let (a, rest) = std::mem::take(&mut piece_dst).split_at_mut(l.pieces.len());
            jobs.push((a, &l.pieces));
            piece_dst = rest;
        }
        jobs.into_par_iter().for_each(|(d, s)| d.copy_from_slice(s));
    }
    // Tens of thousands of small buffers: free them on the pool, off the frame's path.
    anicraft_render::drop_in_background((locals, rows));

    Lists { index, geo, entries_at: entries_at as u32, ops_at: ops_at as u32, entries: total, pieces, work }
}

/// Tile row `r`'s streams: per tile, its fills in drawing order with the clip and group
/// entries they need.
fn row_streams(r: usize, tx: usize, locals: &[(Local, Vec<u32>)], piece_at: &[u32], t: &Tables) -> RowOut {
    let first_tile = (r * tx) as u32;
    let slices = || {
        locals.iter().enumerate().map(move |(c, (l, rows))| {
            let (a, b) = (rows.get(r).copied().unwrap_or(0) as usize, rows.get(r + 1).copied().unwrap_or(0) as usize);
            (c, l.entries.get(a..b).unwrap_or(&[]))
        })
    };
    // Counting sort by tile, fills and artwork apart, keeping drawing order within a tile.
    let mut count = vec![[0u32; 2]; tx + 1];
    for (_, es) in slices() {
        for e in es {
            if let Some(n) = count.get_mut(e[1].wrapping_sub(first_tile) as usize + 1) {
                n[usize::from(e[0] >= t.fills)] += 1;
            }
        }
    }
    for i in 1..count.len() {
        let prev = count[i - 1];
        count[i][0] += prev[0];
        count[i][1] += prev[1];
    }
    let (nfill, nart) = (count[tx][0] as usize, count[tx][1] as usize);
    let mut fe = vec![[0u32; 4]; nfill];
    let mut ae = vec![[0u32; 4]; nart];
    let mut cursor: Vec<[u32; 2]> = count[..tx].to_vec();
    for (c, es) in slices() {
        let p0 = piece_at.get(c).copied().unwrap_or(0);
        for &[op, tile, s0, n, bd] in es {
            let Some(cur) = cursor.get_mut(tile.wrapping_sub(first_tile) as usize) else { continue };
            let art = usize::from(op >= t.fills);
            let at = cur[art] as usize;
            cur[art] += 1;
            let slot = if art == 1 { ae.get_mut(at) } else { fe.get_mut(at) };
            if let Some(slot) = slot {
                *slot = [op, p0 + s0, n, bd];
            }
        }
    }

    let mut out = RowOut { stream: Vec::with_capacity(nfill), heads: Vec::with_capacity(tx), work: 0 };
    let plain = t.scene.clips.is_empty() && t.scene.groups.is_empty();
    for tile in 0..tx {
        let start = out.stream.len();
        let fills = fe.get(count[tile][0] as usize..count[tile + 1][0] as usize).unwrap_or(&[]);
        if plain {
            out.stream.extend_from_slice(fills);
        } else {
            let arts = ae.get(count[tile][1] as usize..count[tile + 1][1] as usize).unwrap_or(&[]);
            tile_stream(fills, arts, t, &mut out.stream);
        }
        let mine = out.stream.get(start..).unwrap_or(&[]);
        out.work += mine.iter().map(|e| if e[0] & CTRL == 0 { u64::from(e[2]) + 1 } else { 1 }).sum::<u64>();
        out.heads.push([start as u32, mine.len() as u32]);
    }
    out
}

/// One tile's stream: its fills, each preceded by the clip coverage it needs (when that clip
/// isn't the one computed last) and the group opens and closes its group chain needs.
fn tile_stream(fills: &[[u32; 4]], arts: &[[u32; 4]], t: &Tables, out: &mut Vec<[u32; 4]>) {
    // The artwork of clip `l` in this tile (artwork entries are in op order, clip by clip).
    let level = |l: u32| -> &[[u32; 4]] {
        let Some(&(a0, a1)) = t.art_ops.get(l as usize) else { return &[] };
        let lo = arts.partition_point(|e| e[0] < a0);
        let hi = arts.partition_point(|e| e[0] < a1);
        arts.get(lo..hi).unwrap_or(&[])
    };
    let close = |g: u32, out: &mut Vec<[u32; 4]>| {
        let (alpha, mode) = t.scene.groups.get(g as usize).map_or((1.0f32, 0), |x| (x.alpha, blend_code(x.blend)));
        out.push([CLOSE, alpha.to_bits(), mode, 0]);
    };
    let mut open: Vec<u32> = Vec::new();
    let mut clip = NONE;
    for e in fills {
        let op = e[0] as usize;
        let c = t.op_clip.get(op).copied().unwrap_or(NONE);
        if c != NONE && c != clip {
            let chain = t.clip_chain.get(c as usize).map_or(&[][..], Vec::as_slice);
            // A level with no artwork here hides the fill in the whole tile.
            if chain.is_empty() || chain.iter().any(|&l| level(l).is_empty()) {
                continue;
            }
            out.push([MASK_RESET, 0, 0, 0]);
            for &l in chain {
                let mode = t.scene.clips.get(l as usize).map_or(0, |c| mask_code(c.mode));
                out.push([LEVEL_BEGIN, mode, 0, 0]);
                out.extend_from_slice(level(l));
                out.push([LEVEL_END, 0, 0, 0]);
            }
            clip = c;
        }
        let g = t.op_group.get(op).copied().unwrap_or(NONE);
        let want = if g == NONE { &[][..] } else { t.scene.groups.get(g as usize).map_or(&[][..], |x| x.chain.as_slice()) };
        let keep = open.iter().zip(want).take_while(|(a, b)| a == b).count();
        while open.len() > keep {
            if let Some(g) = open.pop() {
                close(g, out);
            }
        }
        for &g in want.iter().skip(keep) {
            out.push([OPEN, g, 0, 0]);
            open.push(g);
        }
        out.push(*e);
    }
    while let Some(g) = open.pop() {
        close(g, out);
    }
}

/// Splits one fill's edges into tile pieces (with the steps where its outline crosses a tile's
/// left edge) and walks each tile row for the backdrops.
fn bin_fill(f: &Fill, op: u32, tiles_x: u32, tiles_y: u32, out: &mut Local, (pieces, track): &mut (Vec<Piece>, Track)) {
    let Some(range) = Range::of(f, tiles_x, tiles_y) else { return };
    pieces.clear();
    if f.hairline {
        for poly in &f.polys {
            for_segments(poly, |a, b| hairline_tiles(a, b, &range, HAIRLINE_MARGIN, |row, col| pieces.push(Piece { row, col, seg: [a.x, a.y, b.x, b.y] })));
        }
    } else {
        for poly in &f.polys {
            let n = poly.len();
            if n < 2 {
                continue;
            }
            track.start(range.nrows as usize);
            let mut keep = |p: Piece| pieces.push(p);
            for i in 0..n {
                let (Some(&a), Some(&c)) = (poly.get(i), poly.get((i + 1) % n)) else { continue };
                split_edge(a, c, &range, |row, col, seg| {
                    if col < range.ncols {
                        keep(Piece { row, col, seg });
                    }
                    track.see(row, Seen { col, end_y: seg[3] }, &range, &mut keep);
                });
            }
            track.finish(&range, &mut keep);
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
        let row_y0 = range.row_y0(row);
        // Pieces left of the fill's tiles only count where they start on the row's top edge.
        let mut backdrop = 0i32;
        while let Some(p) = pieces.get(i).filter(|p| p.row == row && p.col < 0) {
            backdrop += top_crossing(&p.seg, row_y0);
            i += 1;
        }
        for col in 0..range.ncols {
            let start = i;
            while pieces.get(i).is_some_and(|p| p.row == row && p.col == col) {
                i += 1;
            }
            let here = pieces.get(start..i).unwrap_or(&[]);
            if here.is_empty() && backdrop == 0 {
                // Nothing here; and nothing further right either once the row's pieces are used.
                if !pieces.get(i).is_some_and(|p| p.row == row) {
                    break;
                }
                continue;
            }
            let tile_x = range.tx0 + col as u32;
            let tile = (range.ty0 + row) * tiles_x + tile_x;
            let (ox, oy) = ((tile_x * TILE) as f32, row_y0);
            let s0 = out.pieces.len() as u32;
            for p in here {
                out.pieces.push([p.seg[0] - ox, p.seg[1] - oy, p.seg[2] - ox, p.seg[3] - oy]);
            }
            out.entries.push([op, tile, s0, here.len() as u32, backdrop as u32]);
            if !f.hairline {
                backdrop += here.iter().map(|p| top_crossing(&p.seg, row_y0)).sum::<i32>();
            }
        }
        // Skip whatever is left of this row (pieces right of the range never get here).
        while pieces.get(i).is_some_and(|p| p.row == row) {
            i += 1;
        }
    }
}

/// The edges of a closed polygon (fills) or the segments of an open polyline (hairlines; a
/// single point is a dot).
fn for_segments(poly: &[Vec2], mut f: impl FnMut(Vec2, Vec2)) {
    match poly {
        [] => {}
        [p] => f(*p, *p),
        _ => {
            for w in poly.windows(2) {
                f(w[0], w[1]);
            }
        }
    }
}

/// The direction (`1` down, `−1` up) of a piece that starts on its tile row's top edge at
/// `row_y0`, which means its edge crosses that edge; `0` for any other piece.
pub(crate) fn top_crossing(seg: &[f32; 4], row_y0: f32) -> i32 {
    if seg[1] == seg[3] || seg[1].min(seg[3]) != row_y0 {
        return 0;
    }
    if seg[3] > seg[1] { 1 } else { -1 }
}

/// A fill's tile range, in pixels and tiles.
#[derive(Clone, Copy)]
#[derive(Debug)]
pub(crate) struct Range {
    pub(crate) tx0: u32,
    pub(crate) ty0: u32,
    pub(crate) ylo: f32,
    pub(crate) yhi: f32,
    pub(crate) xlo: f32,
    pub(crate) xhi: f32,
    pub(crate) ncols: i32,
    pub(crate) nrows: i32,
}

impl Range {
    /// The tiles `f`'s bounds cover, within the surface's; `None` when there are none.
    pub(crate) fn of(f: &Fill, tiles_x: u32, tiles_y: u32) -> Option<Range> {
        let b = f.bounds;
        if b.is_empty() {
            return None;
        }
        let tx0 = b.x0.max(0) as u32 / TILE;
        let tx1 = (b.x1.max(0) as u32).div_ceil(TILE).min(tiles_x);
        let ty0 = b.y0.max(0) as u32 / TILE;
        let ty1 = (b.y1.max(0) as u32).div_ceil(TILE).min(tiles_y);
        if tx1 <= tx0 || ty1 <= ty0 {
            return None;
        }
        let (ncols, nrows) = ((tx1 - tx0) as i32, (ty1 - ty0) as i32);
        Some(Range { tx0, ty0, ylo: (ty0 * TILE) as f32, yhi: (ty1 * TILE) as f32, xlo: (tx0 * TILE) as f32, xhi: (tx1 * TILE) as f32, ncols, nrows })
    }

    fn row_y0(&self, row: u32) -> f32 {
        ((self.ty0 + row) * TILE) as f32
    }
}

/// Splits edge `a → b` at tile rows and columns inside the fill's range, row by row, each row's
/// pieces in the edge's direction of travel. Parts right of the fill's tiles come as column
/// `ncols` (they draw nothing, but the steps on the tiles' left edges need to see them).
fn split_edge(a: Vec2, b: Vec2, r: &Range, mut out: impl FnMut(u32, i32, [f32; 4])) {
    if !(a.is_finite() && b.is_finite()) || a.y == b.y {
        return;
    }
    let down = a.y < b.y;
    let (p, q) = if down { (a, b) } else { (b, a) };
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
        let (top, bottom) = (Vec2::new(x_at(ys), ys), Vec2::new(x_at(ye), ye));
        if down {
            split_row(row, top, bottom, r, &mut out);
        } else {
            split_row(row, bottom, top, r, &mut out);
        }
    }
}

/// Splits a piece inside one tile row, from `s` to `e` (the edge's direction), at the fill's
/// left and right edges and at tile columns: column −1 left of the fill's tiles, `ncols` right.
fn split_row(row: u32, s: Vec2, e: Vec2, r: &Range, out: &mut impl FnMut(u32, i32, [f32; 4])) {
    let down = e.y > s.y;
    let mut push = |col: i32, u: Vec2, v: Vec2| {
        if (down && v.y <= u.y) || (!down && v.y >= u.y) {
            return;
        }
        out(row, col, [u.x, u.y, v.x, v.y]);
    };
    // Points where the piece crosses a column boundary (or the range's edges), in order.
    let dx = e.x - s.x;
    let at_x = |x: f32| -> Vec2 {
        let t = ((x - s.x) / dx).clamp(0.0, 1.0);
        Vec2::new(x, s.y + (e.y - s.y) * t)
    };
    let col_of = |x: f32| ((x - r.xlo) / T).floor() as i32;
    let (lo, hi) = (s.x.min(e.x), s.x.max(e.x));
    // Left of the fill's tiles (a vertical piece on their left edge is in the first column:
    // the backdrop and the left-edge steps count only what lies strictly left of a tile).
    if hi <= r.xlo && lo < r.xlo {
        push(-1, s, e);
        return;
    }
    if lo >= r.xhi {
        push(r.ncols, s, e);
        return;
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
        } else {
            push(r.ncols, prev, next);
        }
        prev = next;
    }
}

/// A piece as the left-edge steps see it: its column and the height where it ends.
#[derive(Clone, Copy)]
struct Seen {
    col: i32,
    end_y: f32,
}

/// Per tile row of a fill, the last piece of the polygon walked so far there and its first:
/// consecutive pieces of a row in different columns are where the outline crosses tiles' left
/// edges.
#[derive(Default)]
struct Track {
    last: Vec<Option<Seen>>,
    first: Vec<Option<Seen>>,
    touched: Vec<u32>,
}

impl Track {
    /// Ready for a polygon of a fill with `nrows` tile rows.
    fn start(&mut self, nrows: usize) {
        for &r in &self.touched {
            if let Some(l) = self.last.get_mut(r as usize) {
                *l = None;
            }
            if let Some(f) = self.first.get_mut(r as usize) {
                *f = None;
            }
        }
        self.touched.clear();
        if self.last.len() < nrows {
            self.last.resize(nrows, None);
            self.first.resize(nrows, None);
        }
    }

    /// The next piece of the polygon in tile row `row`; the steps between it and the row's
    /// previous piece go to `out`.
    fn see(&mut self, row: u32, s: Seen, r: &Range, out: &mut impl FnMut(Piece)) {
        let (Some(last), Some(first)) = (self.last.get_mut(row as usize), self.first.get_mut(row as usize)) else { return };
        match *last {
            Some(p) => steps(p, s, row, r, out),
            None => {
                *first = Some(s);
                self.touched.push(row);
            }
        }
        *last = Some(s);
    }

    /// The polygon is closed: each row's last piece is followed by its first.
    fn finish(&mut self, r: &Range, out: &mut impl FnMut(Piece)) {
        for &row in &self.touched {
            if let (Some(Some(l)), Some(Some(f))) = (self.last.get(row as usize), self.first.get(row as usize)) {
                steps(*l, *f, row, r, out);
            }
        }
    }
}

/// The steps in the winding along the tiles' left edges between consecutive pieces `a` and `b`
/// of one tile row (ADR 0048): where the outline passes from column `a.col` to `b.col`, at the
/// height where `a` ends, each tile boundary between them gets a vertical piece on its left
/// edge from there to the row's bottom: upwards when the outline runs right (the winding left
/// of the tile drops by one), downwards when it runs left. On the row's top edge the backdrop
/// has the step already, on its bottom edge it changes nothing.
fn steps(a: Seen, b: Seen, row: u32, r: &Range, out: &mut impl FnMut(Piece)) {
    if a.col == b.col {
        return;
    }
    let top = r.ylo + (row * TILE) as f32;
    let (y, bottom) = (a.end_y, top + T);
    if !(y > top && y < bottom) {
        return;
    }
    let right = a.col < b.col;
    let (lo, hi) = if right { (a.col, b.col) } else { (b.col, a.col) };
    for k in (lo + 1).max(0)..=hi.min(r.ncols - 1) {
        let x = r.xlo + k as f32 * T;
        let seg = if right { [x, bottom, x, y] } else { [x, y, x, bottom] };
        out(Piece { row, col: k, seg });
    }
}

/// Every tile of the range within `margin` of segment `a → b` (rows and columns padded by it),
/// as `(row, col)` relative to the range. With [`HAIRLINE_MARGIN`], every tile with a pixel
/// centre within a hairline's reach (`bin.wgsl` › `hairline` uses the same).
pub(crate) fn hairline_tiles(a: Vec2, b: Vec2, r: &Range, margin: f32, mut out: impl FnMut(u32, i32)) {
    if !(a.is_finite() && b.is_finite()) {
        return;
    }
    let m = margin;
    let (ymin, ymax) = (a.y.min(b.y), a.y.max(b.y));
    let r0 = (((ymin - m - r.ylo) / T).floor()).max(0.0);
    let r1 = (((ymax + m - r.ylo) / T).floor()).min((r.nrows - 1) as f32);
    if r1 < r0 {
        return;
    }
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    for row in r0 as i32..=r1 as i32 {
        let top = r.ylo + row as f32 * T;
        // The segment's part within the row's band, then the columns within reach of it.
        let (xa, xb) = if dy == 0.0 {
            (a.x.min(b.x), a.x.max(b.x))
        } else {
            let t0 = ((top - m - a.y) / dy).clamp(0.0, 1.0);
            let t1 = ((top + T + m - a.y) / dy).clamp(0.0, 1.0);
            let (u, v) = (a.x + dx * t0, a.x + dx * t1);
            (u.min(v), u.max(v))
        };
        let c0 = (((xa - m - r.xlo) / T).floor()).max(0.0);
        let c1 = (((xb + m - r.xlo) / T).floor()).min((r.ncols - 1) as f32);
        if c1 < c0 {
            continue;
        }
        for col in c0 as i32..=c1 as i32 {
            out(row as u32, col);
        }
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

/// How `fine.wgsl` reads a gradient's spread.
fn spread_code(s: anicraft_doc::Spread) -> f32 {
    match s {
        anicraft_doc::Spread::Pad => 0.0,
        anicraft_doc::Spread::Reflect => 1.0,
        anicraft_doc::Spread::Repeat => 2.0,
    }
}
