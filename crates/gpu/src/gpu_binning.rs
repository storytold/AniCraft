//! Tile binning on the GPU (ADR 0048): the host half.
//!
//! [`crate::binning`] is the reference: it splits every fill's edges into tile pieces on the
//! CPU. Here the CPU only lays out what the GPU needs, in time proportional to the fills and
//! their points, and `bin.wgsl` does the splitting, the backdrops and the tile lists:
//!
//! * the fills' points and polygons, one flat buffer, with each fill's tile range;
//! * one GPU invocation per row of tiles of each fill (an *op-row*), and room for each tile
//!   of each fill's bounds (an *op-tile*): its piece count, backdrop, place and cursor;
//! * per tile row, the fills overlapping it in drawing order, and per tile where its entries
//!   go: room for every fill whose bounds cover it (the GPU keeps those with pieces or a
//!   backdrop there, in order);
//! * an upper bound of the pieces, from each edge's rows and columns, so every buffer is sized
//!   before the GPU runs and nothing is read back.
//!
//! The passes count, scan (prefix sum), scatter and list; their output is the same tile lists
//! as the CPU's, bit for bit in everything but rounding (a test compares them per pixel).

use crate::binning::{HAIRLINE_MARGIN, Range, TILE, hairline_tiles};
use anicraft_render::Fill;
use rayon::prelude::*;

/// An op that is a hairline (`bin.wgsl` › `HAIRLINE_BIT`).
const HAIRLINE_BIT: u32 = 1 << 31;
/// `u32`s per op in the input (`bin.wgsl` › `Op`).
const OP_WORDS: usize = 8;
/// Values one scan workgroup handles (`scan.wgsl`).
const SCAN_BLOCK: u32 = 1024;

/// When and how far the GPU bins.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// The largest buffer the device binds (bytes).
    pub binding: u64,
    /// Fewer edges than this bin on the CPU, where it takes less than setting up the passes.
    pub min_edges: u64,
}

/// Most edge visits the GPU's per-row passes may make (each row of a fill walks all its
/// edges): far above real scenes (the 50k-shape 4K stress frame makes about 6 M), it keeps a
/// fill with very many edges over very many rows on the CPU, which splits each edge once.
pub const MAX_VISITS: u64 = 1 << 26;

/// What the GPU needs to bin a scene, laid out by the CPU.
pub struct Plan {
    /// The ops, polygons, points, op-row owners (and each op's first op-row), the rows' fills
    /// and the rows' starts, at [`Plan::at`].
    pub(crate) input: Vec<u32>,
    pub(crate) at: [u32; 6],
    pub(crate) op_rows: u32,
    /// Tiles of the fills' bounds: the entries' room and the working values' length.
    pub op_tiles: u32,
    /// Room for pieces: an upper bound.
    pub piece_cap: u32,
    /// `[start, 0]` per tile.
    pub(crate) headers: Vec<u32>,
    /// Edges, and edge visits by the per-row passes.
    pub edges: u64,
    pub visits: u64,
}

/// What one fill needs: its range and how much room.
struct Info {
    range: Option<Range>,
    polys: u32,
    points: u32,
    edges: u64,
    pieces: u64,
}

fn info(f: &Fill, tiles_x: u32, tiles_y: u32) -> Info {
    let Some(r) = Range::of(f, tiles_x, tiles_y) else { return Info { range: None, polys: 0, points: 0, edges: 0, pieces: 0 } };
    let mut pieces = 0u64;
    let mut edges = 0u64;
    for poly in &f.polys {
        if f.hairline {
            let mut seg = |a, b| {
                pieces += hairline_pieces(a, b, &r);
                edges += 1;
            };
            match poly.as_slice() {
                [] => {}
                [p] => seg(*p, *p),
                _ => poly.windows(2).for_each(|w| seg(w[0], w[1])),
            }
        } else {
            let n = poly.len();
            if n < 2 {
                continue;
            }
            for i in 0..n {
                if let (Some(&a), Some(&b)) = (poly.get(i), poly.get((i + 1) % n)) {
                    pieces += edge_pieces(a, b, &r);
                }
            }
            edges += n as u64;
        }
    }
    let points = f.polys.iter().map(Vec::len).sum::<usize>() as u32;
    Info { range: Some(r), polys: f.polys.len() as u32, points, edges, pieces }
}

/// At most how many tiles a hairline segment reaches (`binning.rs` › `hairline_tiles`), with a
/// pixel more reach than the binning's, whatever the GPU's rounding: the tiles of its padded
/// box (most segments are short), or row by row for a long one, where the box would be loose.
fn hairline_pieces(a: anicraft_geom::Vec2, b: anicraft_geom::Vec2, r: &Range) -> u64 {
    if !(a.is_finite() && b.is_finite()) {
        return 0;
    }
    let (t, m) = (TILE as f32, HAIRLINE_MARGIN + 1.0);
    let span = |lo: f32, hi: f32, base: f32, n: i32| (((hi + m - base) / t).floor().min((n - 1) as f32) - ((lo - m - base) / t).floor().max(0.0) + 1.0).max(0.0);
    let rows = span(a.y.min(b.y), a.y.max(b.y), r.ylo, r.nrows);
    let cols = span(a.x.min(b.x), a.x.max(b.x), r.xlo, r.ncols);
    if rows * cols <= 16.0 {
        return (rows * cols) as u64;
    }
    let mut n = 0u64;
    hairline_tiles(a, b, r, m, |_, _| n += 1);
    n
}

/// At most how many pieces edge `a → b` gives (`binning.rs` › `split_edge` and `steps`): one
/// per tile row it spans, one more per column boundary it crosses, a left-edge step per column
/// boundary it passes, and one for where it meets the next edge; with slack for rounding.
fn edge_pieces(a: anicraft_geom::Vec2, b: anicraft_geom::Vec2, r: &Range) -> u64 {
    if !(a.is_finite() && b.is_finite()) {
        return 0;
    }
    let t = TILE as f32;
    let (y0, y1) = (a.y.min(b.y).max(r.ylo), a.y.max(b.y).min(r.yhi));
    if y1 < y0 {
        return 0;
    }
    let rows = ((y1 - r.ylo) / t).floor() - ((y0 - r.ylo) / t).floor() + 1.0;
    let (x0, x1) = (a.x.min(b.x).max(r.xlo), a.x.max(b.x).min(r.xhi));
    let cols = if x1 >= x0 { ((x1 - r.xlo) / t).floor() - ((x0 - r.xlo) / t).floor() + 1.0 } else { 1.0 };
    (rows.clamp(0.0, r.nrows as f32 + 1.0) + 2.0 * cols.clamp(0.0, r.ncols as f32 + 1.0)) as u64 + 3
}

/// The plan for binning `fills` (the scene's, in drawing order) on a `tiles_x × tiles_y`
/// surface; `None` when the CPU should: too few edges to be worth it, too many visits, or a
/// buffer larger than the device binds.
pub fn plan(fills: &[Fill], tiles_x: u32, tiles_y: u32, lim: &Limits) -> Option<Plan> {
    let infos: Vec<Info> = fills.par_iter().map(|f| info(f, tiles_x, tiles_y)).collect();
    let n = fills.len();
    let (mut polys, mut points, mut op_tiles, mut op_rows, mut pieces, mut edges, mut visits) = (0u64, 0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
    for i in &infos {
        polys += u64::from(i.polys);
        points += u64::from(i.points);
        pieces += i.pieces;
        edges += i.edges;
        if let Some(r) = &i.range {
            op_tiles += (r.ncols as u64) * (r.nrows as u64);
            op_rows += r.nrows as u64;
            visits += (r.nrows as u64) * i.edges;
        }
    }
    let tiles = u64::from(tiles_x) * u64::from(tiles_y);
    let words = OP_WORDS as u64 * n as u64 + 2 * polys + 2 * points + op_rows + n as u64 + op_rows + u64::from(tiles_y) + 1;
    let work_words = 4 * op_tiles + 2 * op_tiles.div_ceil(u64::from(SCAN_BLOCK)) + 64;
    let fits = |bytes: u64| bytes <= lim.binding;
    if edges < lim.min_edges || visits > MAX_VISITS || pieces >= u64::from(u32::MAX) || op_tiles >= u64::from(u32::MAX) / 4 {
        return None;
    }
    if !(fits(4 * words) && fits(4 * work_words) && fits(16 * pieces) && fits(4 * (2 * tiles + 4 * op_tiles + 4 * n as u64))) {
        return None;
    }

    // Where everything goes.
    let at_ops = 0usize;
    let at_polys = at_ops + OP_WORDS * n;
    let at_points = at_polys + 2 * polys as usize;
    let at_row_of = at_points + 2 * points as usize;
    let at_row_ops = at_row_of + op_rows as usize + n;
    let at_row_start = at_row_ops + op_rows as usize;
    let mut input = vec![0u32; at_row_start + tiles_y as usize + 1];

    // Per op: its first polygon, point, op-tile and op-row.
    let mut firsts = Vec::with_capacity(n);
    let (mut p0, mut q0, mut t0, mut r0) = (0u32, 0u32, 0u32, 0u32);
    for i in &infos {
        firsts.push((p0, q0, t0, r0));
        p0 += i.polys;
        q0 += i.points;
        if let Some(r) = &i.range {
            t0 += (r.ncols * r.nrows) as u32;
            r0 += r.nrows as u32;
        }
    }
    {
        let (head, rest) = input.split_at_mut(at_polys);
        let (poly_words, rest) = rest.split_at_mut(at_points - at_polys);
        let (point_words, rest) = rest.split_at_mut(at_row_of - at_points);
        let (row_of, rest) = rest.split_at_mut(op_rows as usize);
        let (row_base, rest) = rest.split_at_mut(n);
        let (row_ops, row_start) = rest.split_at_mut(op_rows as usize);
        // Each op's slices of the polygon and point tables, then filled in parallel.
        let mut jobs: Vec<Job> = Vec::with_capacity(n);
        let (mut pw, mut qw) = (poly_words, point_words);
        for ((h, f), (i, first)) in head.chunks_mut(OP_WORDS).zip(fills).zip(infos.iter().zip(&firsts)) {
            let (a, b) = std::mem::take(&mut pw).split_at_mut(2 * i.polys as usize);
            let (c, d) = std::mem::take(&mut qw).split_at_mut(2 * i.points as usize);
            pw = b;
            qw = d;
            jobs.push((h, a, c, f, i, *first));
        }
        jobs.into_par_iter().for_each(|(h, pw, qw, f, i, (p0, q0, t0, _))| {
            let (tx0, ty0, ncols, nrows) = i.range.map_or((0, 0, 0, 0), |r| (r.tx0, r.ty0, r.ncols as u32, r.nrows as u32));
            let flags = if f.hairline { HAIRLINE_BIT } else { 0 };
            h.copy_from_slice(&[p0, i.polys, tx0, ty0, ncols, nrows, t0, flags]);
            let mut q = q0;
            for (slot, poly) in pw.as_chunks_mut::<2>().0.iter_mut().zip(&f.polys) {
                slot.copy_from_slice(&[q, poly.len() as u32]);
                q += poly.len() as u32;
            }
            for (slot, p) in qw.as_chunks_mut::<2>().0.iter_mut().zip(f.polys.iter().flatten()) {
                slot.copy_from_slice(&[p.x.to_bits(), p.y.to_bits()]);
            }
        });
        // Op-rows in op order: who owns each, and each op's first.
        let mut k = 0usize;
        for (op, (i, first)) in infos.iter().zip(&firsts).enumerate() {
            if let Some(b) = row_base.get_mut(op) {
                *b = first.3;
            }
            for _ in 0..i.range.map_or(0, |r| r.nrows) {
                if let Some(o) = row_of.get_mut(k) {
                    *o = op as u32;
                }
                k += 1;
            }
        }
        // Each tile row's fills, in drawing order (a counting sort by row).
        for i in &infos {
            if let Some(r) = &i.range {
                for row in r.ty0..r.ty0 + r.nrows as u32 {
                    if let Some(c) = row_start.get_mut(row as usize + 1) {
                        *c += 1;
                    }
                }
            }
        }
        for j in 1..row_start.len() {
            row_start[j] += row_start[j - 1];
        }
        let mut cursor: Vec<u32> = row_start.to_vec();
        for (op, i) in infos.iter().enumerate() {
            if let Some(r) = &i.range {
                for row in r.ty0..r.ty0 + r.nrows as u32 {
                    if let Some(c) = cursor.get_mut(row as usize)
                        && let Some(slot) = row_ops.get_mut(*c as usize)
                    {
                        *slot = op as u32;
                        *c += 1;
                    }
                }
            }
        }
    }
    // Where each tile's entries go: room for every fill whose bounds cover it (a 2D running
    // sum over the bounds' corners).
    let (w, h) = (tiles_x as usize + 1, tiles_y as usize + 1);
    let mut cover = vec![0i64; w * h];
    for r in infos.iter().filter_map(|i| i.range) {
        let (x0, y0, x1, y1) = (r.tx0 as usize, r.ty0 as usize, r.tx0 as usize + r.ncols as usize, r.ty0 as usize + r.nrows as usize);
        for (x, y, d) in [(x0, y0, 1), (x1, y0, -1), (x0, y1, -1), (x1, y1, 1)] {
            if let Some(c) = cover.get_mut(y * w + x) {
                *c += d;
            }
        }
    }
    for y in 0..h {
        for x in 0..w {
            let left = if x > 0 { cover[y * w + x - 1] } else { 0 };
            let up = if y > 0 { cover[(y - 1) * w + x] } else { 0 };
            let diag = if x > 0 && y > 0 { cover[(y - 1) * w + x - 1] } else { 0 };
            cover[y * w + x] += left + up - diag;
        }
    }
    let mut headers = vec![0u32; 2 * tiles as usize];
    let mut start = 0u64;
    for ty in 0..tiles_y as usize {
        for tx in 0..tiles_x as usize {
            if let Some(hd) = headers.get_mut(2 * (ty * tiles_x as usize + tx)) {
                *hd = start as u32;
            }
            start += cover.get(ty * w + tx).copied().unwrap_or(0).max(0) as u64;
        }
    }
    if start != op_tiles {
        return None; // the room per tile must add up to the op-tiles (it always does)
    }
    let at = [at_ops, at_polys, at_points, at_row_of, at_row_ops, at_row_start].map(|a| a as u32);
    Some(Plan { input, at, op_rows: op_rows as u32, op_tiles: op_tiles as u32, piece_cap: pieces as u32, headers, edges, visits })
}

/// One op's share of the input to fill in: its record, polygon and point words, the fill, its
/// sizes, and its first polygon, point, op-tile and op-row.
type Job<'a> = (&'a mut [u32], &'a mut [u32], &'a mut [u32], &'a Fill, &'a Info, (u32, u32, u32, u32));

/// The binning pipelines (`bin.wgsl`, `scan.wgsl`).
pub(crate) struct Pipelines {
    count: wgpu::ComputePipeline,
    scatter: wgpu::ComputePipeline,
    tiles: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    scan_blocks: wgpu::ComputePipeline,
    scan_add: wgpu::ComputePipeline,
    scan_layout: wgpu::BindGroupLayout,
}

impl Pipelines {
    pub(crate) fn new(device: &wgpu::Device) -> Pipelines {
        let entry = |binding: u32, ty: wgpu::BufferBindingType| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None }, count: None };
        let rw = wgpu::BufferBindingType::Storage { read_only: false };
        let bin_entries = [entry(0, wgpu::BufferBindingType::Uniform), entry(1, wgpu::BufferBindingType::Storage { read_only: true }), entry(2, rw), entry(3, rw), entry(4, rw)];
        let ([count, scatter, tiles], layout) = pipelines(device, "anicraft-bin", include_str!("bin.wgsl"), &bin_entries, ["count", "scatter", "tiles"]);
        let ([scan_blocks, scan_add], scan_layout) = pipelines(device, "anicraft-scan", include_str!("scan.wgsl"), &[entry(0, wgpu::BufferBindingType::Uniform), entry(1, rw)], ["scan_blocks", "scan_add"]);
        Pipelines { count, scatter, tiles, layout, scan_blocks, scan_add, scan_layout }
    }
}

/// One shader's entry points as pipelines sharing one bind group layout.
fn pipelines<const N: usize>(device: &wgpu::Device, label: &str, src: &str, entries: &[wgpu::BindGroupLayoutEntry], points: [&str; N]) -> ([wgpu::ComputePipeline; N], wgpu::BindGroupLayout) {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Wgsl(src.into()) });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(label), entries });
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(label), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
    let pipes = points.map(|p| device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(p), layout: Some(&pl), module: &module, entry_point: Some(p), compilation_options: Default::default(), cache: None }));
    (pipes, layout)
}

/// Workgroups for `n` invocations of `width`, spread over x and y (at most 65,535 each way).
fn groups(n: u32, width: u32) -> (u32, u32) {
    let g = n.div_ceil(width).max(1);
    let x = g.min(65_535);
    (x, g.div_ceil(x))
}

/// The fine pass's index and pieces, binned on the GPU into `enc` from `plan`: the index with
/// the tile headers at 0 and `op_heads` at `ops_at`. Only those are uploaded (through `queue`,
/// ahead of `enc`'s submission): the entries' room starts zeroed on the GPU.
#[allow(clippy::too_many_arguments)]
pub(crate) fn record(device: &wgpu::Device, queue: &wgpu::Queue, pipes: &Pipelines, enc: &mut wgpu::CommandEncoder, plan: &Plan, tiles_x: u32, tiles_y: u32, entries_at: u32, ops_at: u32, op_heads: &[u32]) -> Result<(wgpu::Buffer, wgpu::Buffer), String> {
    use wgpu::util::DeviceExt;
    let index_len = (ops_at as usize + op_heads.len()).max(4);
    let index = device.create_buffer(&wgpu::BufferDescriptor { label: Some("index"), size: 4 * index_len as u64, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    if !plan.headers.is_empty() && plan.headers.len() <= index_len {
        queue.write_buffer(&index, 0, bytemuck::cast_slice(&plan.headers));
    }
    if !op_heads.is_empty() && ops_at as usize + op_heads.len() <= index_len {
        queue.write_buffer(&index, 4 * u64::from(ops_at), bytemuck::cast_slice(op_heads));
    }
    let geo = device.create_buffer(&wgpu::BufferDescriptor { label: Some("geo"), size: 16 * u64::from(plan.piece_cap.max(1)), usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
    if plan.op_rows == 0 {
        return Ok((index, geo));
    }
    let n = plan.op_tiles;
    // Counts, backdrops, offsets, cursors, then the scan's totals level by level. New buffers
    // start zeroed.
    let mut levels: Vec<[u32; 4]> = Vec::new();
    let (mut src, mut dst, mut len, mut sums) = (0u32, 2 * n, n, 4 * n);
    loop {
        let blocks = len.div_ceil(SCAN_BLOCK);
        levels.push([src, dst, sums, len]);
        if blocks <= 1 {
            break;
        }
        (src, dst, len) = (sums, sums, blocks);
        sums += blocks;
    }
    let work_len = (sums + 2) as u64;
    let work = device.create_buffer(&wgpu::BufferDescriptor { label: Some("bin-work"), size: 4 * work_len, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false });
    let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("bin-input"), contents: bytemuck::cast_slice(&plan.input), usage: wgpu::BufferUsages::STORAGE });
    let tiles = tiles_x * tiles_y;
    let a = plan.at;
    let params: [u32; 16] = [tiles_x, tiles_y, plan.op_rows, tiles, n, plan.piece_cap, entries_at, 0, a[0], a[1], a[2], a[3], a[4], a[5], 0, 0];
    let ubuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("bin-params"), contents: bytemuck::cast_slice(&params), usage: wgpu::BufferUsages::UNIFORM });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("anicraft-bin"),
        layout: &pipes.layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: input.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: work.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: geo.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 4, resource: index.as_entire_binding() },
        ],
    });
    let scans: Vec<(wgpu::BindGroup, u32)> = levels
        .iter()
        .map(|l| {
            let u = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("scan-params"), contents: bytemuck::cast_slice(l), usage: wgpu::BufferUsages::UNIFORM });
            let b = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("anicraft-scan"),
                layout: &pipes.scan_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: u.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: work.as_entire_binding() }],
            });
            (b, l[3].div_ceil(SCAN_BLOCK))
        })
        .collect();
    let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("anicraft-bin"), timestamp_writes: None });
    let rows = groups(plan.op_rows, 64);
    pass.set_pipeline(&pipes.count);
    pass.set_bind_group(0, &bind, &[]);
    pass.dispatch_workgroups(rows.0, rows.1, 1);
    // The scan, level by level up, then each level's totals added back down.
    pass.set_pipeline(&pipes.scan_blocks);
    for (b, blocks) in &scans {
        let g = groups(*blocks, 1);
        pass.set_bind_group(0, b, &[]);
        pass.dispatch_workgroups(g.0, g.1, 1);
    }
    pass.set_pipeline(&pipes.scan_add);
    for (b, blocks) in scans.iter().rev().skip(1) {
        let g = groups(*blocks, 1);
        pass.set_bind_group(0, b, &[]);
        pass.dispatch_workgroups(g.0, g.1, 1);
    }
    pass.set_pipeline(&pipes.scatter);
    pass.set_bind_group(0, &bind, &[]);
    pass.dispatch_workgroups(rows.0, rows.1, 1);
    let t = groups(tiles, 64);
    pass.set_pipeline(&pipes.tiles);
    pass.dispatch_workgroups(t.0, t.1, 1);
    drop(pass);
    Ok((index, geo))
}
