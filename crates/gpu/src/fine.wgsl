// AniCraft GPU stage: fine rasterization (ADR 0019, ADR 0029).
//
// One workgroup per 16 × 16 tile, one invocation per pixel. Each pixel walks its tile's entries
// in drawing order. An entry's coverage is its backdrop (winding from edges left of the tile,
// per pixel row) plus the exact area each of the tile's edge pieces covers in the pixel: the
// same integral as the CPU oracle's accumulation buffer (crates/render/src/raster.rs). Then the
// fill rule, the paint (solid, or the oracle's own 256-entry gradient table) and src-over on
// premultiplied colour, all in f32, as the oracle does.
//
// Masks and groups (ADR 0029) are control entries in the same list, mirroring the oracle's band
// compositor (crates/render/src/lib.rs): clip artwork unites into a level's coverage, levels
// multiply into the clip coverage that clipped fills are scaled by; a group pushes the colour
// so far and continues on transparent, and its close blends it into the colour below at the
// group's alpha, in its blend mode (the W3C separable formulas, as the oracle's `blend`).
//
// Two entry points: `main` writes a storage buffer (premultiplied RGBA8 or f32, for readback),
// `main_texture` writes premultiplied RGBA8 into a texture the window shows directly.

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
@group(0) @binding(5) var out_texture: texture_storage_2d<rgba8unorm, write>;

const NONE: u32 = 0xffffffffu;
const PAINT_VEC4S: u32 = 4u;
// Must match binning.rs.
const CTRL: u32 = 0x80000000u;
const OPEN: u32 = 0x80000001u;
const CLOSE: u32 = 0x80000002u;
const MASK_RESET: u32 = 0x80000003u;
const LEVEL_BEGIN: u32 = 0x80000004u;
const LEVEL_END: u32 = 0x80000005u;
const CLIPPED: u32 = 1u;
const ART: u32 = 2u;
const MAX_GROUP_DEPTH: u32 = 8u;

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

fn unpremul(c: f32, a: f32) -> f32 {
    if a > 1e-6 {
        return clamp(c / a, 0.0, 1.0);
    }
    return 0.0;
}

fn hard(cb: f32, cs: f32) -> f32 {
    if cs <= 0.5 {
        return cb * 2.0 * cs;
    }
    let t = 2.0 * cs - 1.0;
    return cb + t - cb * t;
}

// The separable blend function B(cb, cs) for mode `m` (binning.rs › blend_code).
fn blend_channel(m: u32, cb: f32, cs: f32) -> f32 {
    switch m {
        case 1u: { return cb * cs; }
        case 2u: { return cb + cs - cb * cs; }
        case 3u: { return min(cb, cs); }
        case 4u: { return max(cb, cs); }
        case 5u: { return abs(cb - cs); }
        case 6u: { return min(cb + cs, 1.0); }
        case 7u: { return max(cb - cs, 0.0); }
        case 8u: { return 1.0 - cb; }
        case 9u: { return hard(cb, cs); }
        case 10u: { return hard(cs, cb); }
        default: { return cs; }
    }
}

// Source s over backdrop b (premultiplied) in mode m, as the oracle's `blend`.
fn blend(m: u32, s: vec4<f32>, b: vec4<f32>) -> vec4<f32> {
    let sa = clamp(s.a, 0.0, 1.0);
    let ba = clamp(b.a, 0.0, 1.0);
    if m == 11u {
        return vec4<f32>(b.rgb * sa, ba * sa);
    }
    if m == 12u {
        let k = 1.0 - sa;
        return vec4<f32>(b.rgb * k, ba * k);
    }
    var o = vec4<f32>(0.0, 0.0, 0.0, sa + ba - sa * ba);
    for (var i = 0u; i < 3u; i++) {
        let cs = unpremul(s[i], sa);
        let cb = unpremul(b[i], ba);
        o[i] = (1.0 - ba) * s[i] + (1.0 - sa) * b[i] + sa * ba * blend_channel(m, cb, cs);
    }
    return o;
}

// A group's colour `s` composited into `below` at `alpha`, in mode `m` (the oracle's
// `close_group`, per pixel).
fn close_group(s: vec4<f32>, below: vec4<f32>, alpha: f32, m: u32) -> vec4<f32> {
    if alpha <= 0.0 {
        return below;
    }
    if m != 0u {
        if s.a > 0.0 {
            return blend(m, s * alpha, below);
        }
        return below;
    }
    let sa = s.a * alpha;
    if sa <= 0.0 {
        return below;
    }
    let inv = 1.0 - sa;
    return vec4<f32>(s.rgb * alpha + below.rgb * inv, sa + below.a * inv);
}

// The finished colour of pixel `lid` in tile `wg`.
fn pixel(wg: vec3<u32>, lid: vec3<u32>) -> vec4<f32> {
    let tile = wg.y * params.size.z + wg.x;
    let x = wg.x * 16u + lid.x;
    let y = wg.y * 16u + lid.y;
    let px = f32(lid.x);
    let py = f32(lid.y);
    let centre = vec2<f32>(f32(x) + 0.5, f32(y) + 0.5);

    var c = params.bg;
    var stack: array<vec4<f32>, MAX_GROUP_DEPTH>;
    var depth = 0u;
    var mask = 1.0;
    var level = 0.0;
    let start = index[tile * 2u];
    let count = index[tile * 2u + 1u];
    for (var i = 0u; i < count; i++) {
        let e = params.at.x + (start + i) * 4u;
        let op = index[e];
        if (op & CTRL) != 0u {
            switch op {
                case OPEN: {
                    if depth < MAX_GROUP_DEPTH {
                        stack[depth] = c;
                        depth += 1u;
                    }
                    c = vec4<f32>(0.0);
                }
                case CLOSE: {
                    if depth > 0u {
                        depth -= 1u;
                        c = close_group(c, stack[depth], bitcast<f32>(index[e + 1u]), index[e + 2u]);
                    }
                }
                case MASK_RESET: { mask = 1.0; }
                case LEVEL_BEGIN: { level = 0.0; }
                case LEVEL_END: { mask *= level; }
                default: {}
            }
            continue;
        }
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
        var cov = fold(w, index[h + 1u]);
        let flags = index[h + 3u];
        if (flags & ART) != 0u {
            let k = clamp(cov, 0.0, 1.0);
            level = level + k - level * k;
            continue;
        }
        if (flags & CLIPPED) != 0u {
            cov *= mask;
        }
        if cov <= 1e-4 {
            continue;
        }
        let s = shade(op, index[h], index[h + 2u], centre);
        c = s * cov + c * (1.0 - s.a * cov);
    }
    return c;
}

// Premultiplied RGBA8 steps, rounded as the oracle's `Pixmap::to_premul_rgba8`.
fn quantize(c: vec4<f32>) -> vec4<u32> {
    return vec4<u32>(clamp(c, vec4<f32>(0.0), vec4<f32>(1.0)) * 255.0 + 0.5);
}

@compute @workgroup_size(16, 16)
fn main(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>) {
    let c = pixel(wg, lid);
    let x = wg.x * 16u + lid.x;
    let y = wg.y * 16u + lid.y;
    if x >= params.size.x || y >= params.size.y {
        return;
    }
    let at = y * params.size.x + x;
    if params.size.w == 0u {
        let q = quantize(c);
        out[at] = q.x | (q.y << 8u) | (q.z << 16u) | (q.w << 24u);
    } else {
        out[at * 4u] = bitcast<u32>(c.x);
        out[at * 4u + 1u] = bitcast<u32>(c.y);
        out[at * 4u + 2u] = bitcast<u32>(c.z);
        out[at * 4u + 3u] = bitcast<u32>(c.w);
    }
}

@compute @workgroup_size(16, 16)
fn main_texture(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>) {
    let c = pixel(wg, lid);
    let x = wg.x * 16u + lid.x;
    let y = wg.y * 16u + lid.y;
    if x >= params.size.x || y >= params.size.y {
        return;
    }
    // Exactly k / 255, so the texture's unorm conversion lands on the oracle's byte k.
    textureStore(out_texture, vec2<u32>(x, y), vec4<f32>(quantize(c)) / 255.0);
}
