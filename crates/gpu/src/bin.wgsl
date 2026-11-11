// AniCraft GPU stage: tile binning on the GPU (ADR 0048).
//
// The same tile lists as the CPU binning (binning.rs, the reference), from the prepared fills'
// flattened polygons, in four kinds of dispatch:
//
// 1. `count`, one invocation per row of tiles of each fill (an "op-row"): its edges split at
//    tile rows and columns (binning.rs › split_edge, split_row), plus the steps where the
//    outline passes from one column to another (› Track, steps), or a hairline's segments by
//    reach (› hairline_tiles), counted per tile of the fill (an "op-tile"); and the backdrops: the
//    directions of the pieces starting on the row's top edge, summed along the row. Each
//    op-row is one invocation's alone, so nothing needs atomics and integers add up the same
//    in any order: the counts and backdrops are exact.
// 2. `scan_blocks` and `scan_add`: an exclusive prefix sum of the counts (the public
//    work-efficient scan: per workgroup, then the workgroups' totals, then added back), which
//    places every op-tile's pieces.
// 3. `scatter`: the same walk again, writing each piece where its op-tile's pieces go, in the
//    walk's order (the same every frame).
// 4. `tiles`, one invocation per tile: its fills in drawing order (the fills overlapping its
//    row, from the CPU, which only needs their bounds), keeping those with pieces or a
//    backdrop there, as the entries the fine pass reads.
//
// The binding layout fits the smallest devices (four storage buffers per stage): one buffer of
// inputs, one of working values (counts, backdrops, offsets, cursors, scan totals), the pieces
// (`geo`) and the fine pass's index.

struct Params {
    // tiles across, tiles down, op-rows, tiles
    size: vec4<u32>,
    // op-tiles, piece capacity, where the entries start in `index`, 0
    more: vec4<u32>,
    // where the ops, polygons, points and op-row owners start in `input`
    at: vec4<u32>,
    // where the rows' fills and the rows' starts are in `input`
    at2: vec4<u32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> input: array<u32>;
@group(0) @binding(2) var<storage, read_write> work: array<u32>;
@group(0) @binding(3) var<storage, read_write> geo: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read_write> index: array<u32>;

const T: f32 = 16.0;
// gpu_binning.rs › HAIRLINE_BIT: the op is a hairline.
const HAIRLINE_BIT: u32 = 0x80000000u;
// How far around a hairline segment its tiles are taken (binning.rs › HAIRLINE_MARGIN).
const M: f32 = 1.25;

// What to do with a piece.
const COUNT: u32 = 0u;
const WRITE: u32 = 1u;

struct Op {
    first_poly: u32,
    polys: u32,
    tx0: u32,
    ty0: u32,
    ncols: u32,
    nrows: u32,
    tile_base: u32,
    flags: u32,
}

fn op_at(i: u32) -> Op {
    let b = params.at.x + i * 8u;
    return Op(input[b], input[b + 1u], input[b + 2u], input[b + 3u], input[b + 4u], input[b + 5u], input[b + 6u], input[b + 7u]);
}

fn point(i: u32) -> vec2<f32> {
    let b = params.at.z + i * 2u;
    return vec2<f32>(bitcast<f32>(input[b]), bitcast<f32>(input[b + 1u]));
}

// Finite (not NaN or infinite), from the bits: comparisons with NaN may be optimized away.
fn finite(v: f32) -> bool {
    return (bitcast<u32>(v) & 0x7f800000u) != 0x7f800000u;
}

fn finite2(p: vec2<f32>) -> bool {
    return finite(p.x) && finite(p.y);
}

// The working values: counts, backdrops, offsets and cursors, `params.more.x` each.
fn counts_at(t: u32) -> u32 {
    return t;
}
fn backdrop_at(t: u32) -> u32 {
    return params.more.x + t;
}
fn offset_at(t: u32) -> u32 {
    return 2u * params.more.x + t;
}
fn cursor_at(t: u32) -> u32 {
    return 3u * params.more.x + t;
}

// The walk of one op-row: what it is and where its pieces go.
struct Walk {
    mode: u32,
    // the op-tile of the row's first column
    tile0: u32,
    ncols: i32,
    hairline: bool,
    row_y0: f32,
    // the row's top-left in pixels
    ox: f32,
}

// The fill's range (binning.rs › Range).
struct Rng {
    xlo: f32,
    xhi: f32,
    ylo: f32,
    yhi: f32,
    ncols: i32,
    nrows: i32,
}

// binning.rs › top_crossing.
fn top_crossing(s: vec4<f32>, row_y0: f32) -> i32 {
    if s.y == s.w || min(s.y, s.w) != row_y0 {
        return 0;
    }
    return select(-1, 1, s.w > s.y);
}

// One piece in column `col` of the walk's row (−1: left of the fill's tiles).
fn emit(w: Walk, col: i32, s: vec4<f32>) {
    if w.mode == COUNT {
        if col >= 0 && col < w.ncols {
            let t = counts_at(w.tile0 + u32(col));
            work[t] = work[t] + 1u;
        }
        if !w.hairline {
            let c = top_crossing(s, w.row_y0);
            if c != 0 && col + 1 < w.ncols {
                let t = backdrop_at(w.tile0 + u32(col + 1));
                work[t] = bitcast<u32>(bitcast<i32>(work[t]) + c);
            }
        }
        return;
    }
    if col < 0 || col >= w.ncols {
        return;
    }
    let tile = w.tile0 + u32(col);
    let k = work[cursor_at(tile)];
    work[cursor_at(tile)] = k + 1u;
    let at = work[offset_at(tile)] + k;
    if at < params.more.y {
        let ox = w.ox + f32(u32(col) * 16u);
        geo[at] = vec4<f32>(s.x - ox, s.y - w.row_y0, s.z - ox, s.w - w.row_y0);
    }
}

// The polygon's last piece so far in this invocation's row, and its first (binning.rs › Track).
var<private> tr_last: bool;
var<private> tr_last_col: i32;
var<private> tr_last_y: f32;
var<private> tr_first: bool;
var<private> tr_first_col: i32;

// binning.rs › steps: where the outline passes from column `a` (its piece ending at height
// `y`) to column `b`, a vertical piece on each tile boundary between them, from there to the
// row's bottom.
fn steps(w: Walk, r: Rng, a: i32, y: f32, b: i32) {
    if a == b {
        return;
    }
    let top = w.row_y0;
    let bottom = top + T;
    if !(y > top && y < bottom) {
        return;
    }
    let right = a < b;
    let lo = min(a, b);
    let hi = max(a, b);
    for (var k = max(lo + 1, 0); k <= min(hi, r.ncols - 1); k++) {
        let x = r.xlo + f32(k) * T;
        if right {
            emit(w, k, vec4<f32>(x, bottom, x, y));
        } else {
            emit(w, k, vec4<f32>(x, y, x, bottom));
        }
    }
}

// A piece of the polygon in this row, in the direction of travel (column −1: left of the
// fill's tiles; `ncols`: right of them, drawing nothing): kept, and seen by the steps.
fn piece(w: Walk, r: Rng, col: i32, s: vec4<f32>) {
    if col < r.ncols {
        emit(w, col, s);
    }
    if tr_last {
        steps(w, r, tr_last_col, tr_last_y, col);
    } else {
        tr_first = true;
        tr_first_col = col;
    }
    tr_last = true;
    tr_last_col = col;
    tr_last_y = s.w;
}

fn push(w: Walk, r: Rng, col: i32, u: vec2<f32>, v: vec2<f32>, down: bool) {
    if (down && v.y <= u.y) || (!down && v.y >= u.y) {
        return;
    }
    piece(w, r, col, vec4<f32>(u, v));
}

// binning.rs › split_row: a piece inside one tile row, from `s` to `e` (the edge's direction),
// split at the fill's left and right edges and at tile columns.
fn split_row(w: Walk, s: vec2<f32>, e: vec2<f32>, r: Rng) {
    let down = e.y > s.y;
    let dx = e.x - s.x;
    let lo = min(s.x, e.x);
    let hi = max(s.x, e.x);
    if hi <= r.xlo && lo < r.xlo {
        push(w, r, -1, s, e, down);
        return;
    }
    if lo >= r.xhi {
        push(w, r, r.ncols, s, e, down);
        return;
    }
    let cl = i32(floor((lo - r.xlo) / T));
    if dx == 0.0 || (lo >= r.xlo && hi < r.xhi && cl == i32(floor((hi - r.xlo) / T))) {
        push(w, r, clamp(cl, 0, r.ncols - 1), s, e, down);
        return;
    }
    let kmin = max(i32(floor((lo - r.xlo) / T)) + 1, 0);
    let kmax = min(i32(ceil((hi - r.xlo) / T)) - 1, r.ncols);
    let count = max(kmax - kmin + 1, 0);
    var prev = s;
    for (var j = 0; j <= count; j++) {
        var next = e;
        if j != count {
            let k = select(kmax - j, kmin + j, dx > 0.0);
            let x = r.xlo + f32(k) * T;
            if !(x > lo && x < hi) {
                continue;
            }
            let t = clamp((x - s.x) / dx, 0.0, 1.0);
            next = vec2<f32>(x, s.y + (e.y - s.y) * t);
        }
        let mid = 0.5 * (prev.x + next.x);
        if mid < r.xlo {
            push(w, r, -1, prev, next, down);
        } else if mid < r.xhi {
            push(w, r, clamp(i32(floor((mid - r.xlo) / T)), 0, r.ncols - 1), prev, next, down);
        } else {
            push(w, r, r.ncols, prev, next, down);
        }
        prev = next;
    }
}

// binning.rs › split_edge, for tile row `row` only.
fn split_edge(w: Walk, a: vec2<f32>, b: vec2<f32>, r: Rng, row: u32) {
    if !(finite2(a) && finite2(b)) || a.y == b.y {
        return;
    }
    let down = a.y < b.y;
    let p = select(b, a, down);
    let q = select(a, b, down);
    let y0 = max(p.y, r.ylo);
    let y1 = min(q.y, r.yhi);
    if y1 <= y0 {
        return;
    }
    let first = u32(max(floor((y0 - r.ylo) / T), 0.0));
    let last = u32(max(ceil((y1 - r.ylo) / T), 0.0));
    if row < first || row >= last {
        return;
    }
    let dxdy = (q.x - p.x) / (q.y - p.y);
    let ry0 = r.ylo + f32(row * 16u);
    let ys = max(y0, ry0);
    let ye = min(y1, ry0 + T);
    if ye <= ys {
        return;
    }
    let top = vec2<f32>(p.x + (ys - p.y) * dxdy, ys);
    let bottom = vec2<f32>(p.x + (ye - p.y) * dxdy, ye);
    if down {
        split_row(w, top, bottom, r);
    } else {
        split_row(w, bottom, top, r);
    }
}

// binning.rs › hairline_tiles, for tile row `row` only: the segment, whole, into every column
// within reach.
fn hairline(w: Walk, a: vec2<f32>, b: vec2<f32>, r: Rng, row: u32) {
    if !(finite2(a) && finite2(b)) {
        return;
    }
    let r0 = max(floor((min(a.y, b.y) - M - r.ylo) / T), 0.0);
    let r1 = min(floor((max(a.y, b.y) + M - r.ylo) / T), f32(r.nrows - 1));
    let rf = f32(row);
    if !(r1 >= r0) || rf < r0 || rf > r1 {
        return;
    }
    let top = r.ylo + rf * T;
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    var xa = min(a.x, b.x);
    var xb = max(a.x, b.x);
    if dy != 0.0 {
        let t0 = clamp((top - M - a.y) / dy, 0.0, 1.0);
        let t1 = clamp((top + T + M - a.y) / dy, 0.0, 1.0);
        let u = a.x + dx * t0;
        let v = a.x + dx * t1;
        xa = min(u, v);
        xb = max(u, v);
    }
    let c0 = max(floor((xa - M - r.xlo) / T), 0.0);
    let c1 = min(floor((xb + M - r.xlo) / T), f32(r.ncols - 1));
    if !(c1 >= c0) {
        return;
    }
    for (var col = i32(c0); col <= i32(c1); col++) {
        emit(w, col, vec4<f32>(a, b));
    }
}

// The invocation's index in a dispatch of 64-wide workgroups spread over x and y.
fn linear(wg: vec3<u32>, n: vec3<u32>, lid: vec3<u32>, width: u32) -> u32 {
    return (wg.y * n.x + wg.x) * width + lid.x;
}

fn walk(mode: u32, t: u32) {
    if t >= params.size.z {
        return;
    }
    let opi = input[params.at.w + t];
    let o = op_at(opi);
    // This op-row's row within the fill: how far `t` is past the fill's first op-row, which
    // `row_of` holds in the op's own slot of rows (op-rows are in op order).
    let row = t - input[params.at.w + params.size.z + opi];
    let ylo = f32(o.ty0 * 16u);
    let r = Rng(f32(o.tx0 * 16u), f32((o.tx0 + o.ncols) * 16u), ylo, f32((o.ty0 + o.nrows) * 16u), i32(o.ncols), i32(o.nrows));
    let hair = (o.flags & HAIRLINE_BIT) != 0u;
    let w = Walk(mode, o.tile_base + row * o.ncols, i32(o.ncols), hair, f32((o.ty0 + row) * 16u), r.xlo);
    for (var pi = 0u; pi < o.polys; pi++) {
        let pb = params.at.y + (o.first_poly + pi) * 2u;
        let start = input[pb];
        let n = input[pb + 1u];
        if hair {
            if n == 1u {
                let p = point(start);
                hairline(w, p, p, r, row);
            }
            for (var i = 0u; i + 1u < n; i++) {
                hairline(w, point(start + i), point(start + i + 1u), r, row);
            }
        } else if n >= 2u {
            tr_last = false;
            tr_first = false;
            for (var i = 0u; i < n; i++) {
                let a = point(start + i);
                let b = point(start + select(i + 1u, 0u, i + 1u == n));
                split_edge(w, a, b, r, row);
            }
            if tr_last && tr_first {
                steps(w, r, tr_last_col, tr_last_y, tr_first_col);
            }
        }
    }
    if mode == COUNT && !hair {
        // The backdrop deltas, summed along the row.
        var acc = 0;
        for (var c = 0u; c < o.ncols; c++) {
            let at = backdrop_at(w.tile0 + c);
            acc += bitcast<i32>(work[at]);
            work[at] = bitcast<u32>(acc);
        }
    }
}

@compute @workgroup_size(64)
fn count(@builtin(workgroup_id) wg: vec3<u32>, @builtin(num_workgroups) n: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>) {
    walk(COUNT, linear(wg, n, lid, 64u));
}

@compute @workgroup_size(64)
fn scatter(@builtin(workgroup_id) wg: vec3<u32>, @builtin(num_workgroups) n: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>) {
    walk(WRITE, linear(wg, n, lid, 64u));
}

@compute @workgroup_size(64)
fn tiles(@builtin(workgroup_id) wg: vec3<u32>, @builtin(num_workgroups) n: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>) {
    let t = linear(wg, n, lid, 64u);
    if t >= params.size.w {
        return;
    }
    let tx = t % params.size.x;
    let ty = t / params.size.x;
    let start = index[2u * t];
    let rs = params.at2.x + input[params.at2.y + ty];
    let re = params.at2.x + input[params.at2.y + ty + 1u];
    var k = 0u;
    for (var i = rs; i < re; i++) {
        let opi = input[i];
        let o = op_at(opi);
        if tx < o.tx0 || tx >= o.tx0 + o.ncols {
            continue;
        }
        let ot = o.tile_base + (ty - o.ty0) * o.ncols + (tx - o.tx0);
        var c = work[counts_at(ot)];
        let bd = work[backdrop_at(ot)];
        if c == 0u && bd == 0u {
            continue;
        }
        let off = min(work[offset_at(ot)], params.more.y);
        c = min(c, params.more.y - off);
        let e = params.more.z + (start + k) * 4u;
        index[e] = opi;
        index[e + 1u] = off;
        index[e + 2u] = c;
        index[e + 3u] = bd;
        k++;
    }
    index[2u * t + 1u] = k;
}
