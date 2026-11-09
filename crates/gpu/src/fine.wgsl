// AniCraft GPU stage: fine rasterization (ADR 0019).
//
// One workgroup per 16 × 16 tile, one invocation per pixel. Each pixel walks its tile's entries
// in drawing order. An entry's coverage is its backdrop (winding from edges left of the tile,
// per pixel row) plus the exact area each of the tile's edge pieces covers in the pixel: the
// same integral as the CPU oracle's accumulation buffer (crates/render/src/raster.rs). Then the
// fill rule, the paint (solid, or the oracle's own 256-entry gradient table) and src-over on
// premultiplied colour, all in f32, as the oracle does.

struct Params {
    // width, height, tiles across, output format (0: premultiplied RGBA8, 1: f32 RGBA)
    size: vec4<u32>,
    // where entries and op headers start in `index`
    at: vec4<u32>,
    // premultiplied background (zero when transparent)
    bg: vec4<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> index: array<u32>;
@group(0) @binding(2) var<storage, read> geo: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read> paints: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read_write> out: array<u32>;

const NONE: u32 = 0xffffffffu;
const PAINT_VEC4S: u32 = 4u;

// Mean of clamp(u, 0, 1) for u running linearly from u0 to u1.
fn avg_clamp(u0: f32, u1: f32) -> f32 {
    let lo = min(u0, u1);
    let hi = max(u0, u1);
    if hi <= 0.0 {
        return 0.0;
    }
    if lo >= 1.0 {
        return 1.0;
    }
    let w = hi - lo;
    if w < 1e-7 {
        return clamp(lo, 0.0, 1.0);
    }
    let a = max(lo, 0.0);
    let b = min(hi, 1.0);
    return ((b - a) * (b + a) * 0.5 + max(hi - 1.0, 0.0)) / w;
}

// Signed area of pixel (px, py) to the right of piece s = (x0, y0, x1, y1), tile-relative.
fn piece_cover(s: vec4<f32>, px: f32, py: f32) -> f32 {
    let ylo = min(s.y, s.w);
    let yhi = max(s.y, s.w);
    let ya = max(ylo, py);
    let yb = min(yhi, py + 1.0);
    if yb <= ya {
        return 0.0;
    }
    let dxdy = (s.z - s.x) / (s.w - s.y);
    let xa = s.x + (ya - s.y) * dxdy;
    let xb = s.x + (yb - s.y) * dxdy;
    let dir = select(-1.0, 1.0, s.w > s.y);
    return dir * (yb - ya) * avg_clamp(px + 1.0 - xa, px + 1.0 - xb);
}

fn fold(w: f32, rule: u32) -> f32 {
    if rule == 0u {
        return min(abs(w), 1.0);
    }
    let t = abs(w) % 2.0;
    return select(t, 2.0 - t, t > 1.0);
}

fn shade(op: u32, kind: u32, ramp: u32, p: vec2<f32>) -> vec4<f32> {
    let base = op * PAINT_VEC4S;
    if kind == 0u {
        return paints[base];
    }
    let m = paints[base + 1u];
    let n = paints[base + 2u];
    let g = paints[base + 3u];
    // Pixel back to paint space: x' = a x + c y + e, y' = b x + d y + f.
    let q = vec2<f32>(m.x * p.x + m.z * p.y + n.x, m.y * p.x + m.w * p.y + n.y);
    var t: f32;
    if kind == 1u {
        t = dot(q - n.zw, g.xy);
    } else {
        t = length(q - n.zw) * g.z;
    }
    t = clamp(t, 0.0, 1.0);
    let i = u32(t * 255.0 + 0.5) & 255u;
    return paints[ramp + i];
}

@compute @workgroup_size(16, 16)
fn main(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>) {
    let width = params.size.x;
    let height = params.size.y;
    let tile = wg.y * params.size.z + wg.x;
    let x = wg.x * 16u + lid.x;
    let y = wg.y * 16u + lid.y;
    let px = f32(lid.x);
    let py = f32(lid.y);
    let centre = vec2<f32>(f32(x) + 0.5, f32(y) + 0.5);

    var c = params.bg;
    let start = index[tile * 2u];
    let count = index[tile * 2u + 1u];
    for (var i = 0u; i < count; i++) {
        let e = params.at.x + (start + i) * 4u;
        let op = index[e];
        let first = index[e + 1u];
        let pieces = index[e + 2u];
        let bd = index[e + 3u];
        var w = 0.0;
        if bd != NONE {
            w = geo[bd + (lid.y >> 2u)][lid.y & 3u];
        }
        for (var k = 0u; k < pieces; k++) {
            w += piece_cover(geo[first + k], px, py);
        }
        let h = params.at.y + op * 4u;
        let cov = fold(w, index[h + 1u]);
        if cov <= 1e-4 {
            continue;
        }
        let s = shade(op, index[h], index[h + 2u], centre);
        c = s * cov + c * (1.0 - s.a * cov);
    }

    if x >= width || y >= height {
        return;
    }
    let at = y * width + x;
    if params.size.w == 0u {
        let q = vec4<u32>(clamp(c, vec4<f32>(0.0), vec4<f32>(1.0)) * 255.0 + 0.5);
        out[at] = q.x | (q.y << 8u) | (q.z << 16u) | (q.w << 24u);
    } else {
        out[at * 4u] = bitcast<u32>(c.x);
        out[at * 4u + 1u] = bitcast<u32>(c.y);
        out[at * 4u + 2u] = bitcast<u32>(c.z);
        out[at * 4u + 3u] = bitcast<u32>(c.w);
    }
}
