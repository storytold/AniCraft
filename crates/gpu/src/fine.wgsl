// AniCraft GPU stage: fine rasterization (ADR 0019, ADR 0029).
//
// One workgroup per 16 × 16 tile, one invocation per pixel. Each pixel walks its tile's entries
// in drawing order. An entry's coverage is its backdrop (the winding number just below the
// tile's top-left corner, an integer; ADR 0048) plus the exact area each of the tile's edge
// pieces covers in the pixel, the steps on the tile's left edge included: the same integral as
// the CPU oracle's accumulation buffer (crates/render/src/raster.rs). Then the fill rule, the
// paint (solid, or the oracle's own 256-entry gradient table) and src-over on premultiplied
// colour, all in f32, as the oracle does. A hairline (ADR 0048) is 1 − the distance from the
// pixel centre to its nearest segment (crates/render/src/hairline.rs).
//
// Masks and groups (ADR 0029) are control entries in the same list, mirroring the oracle's band
// compositor (crates/render/src/lib.rs): clip artwork unites into a level's coverage, levels
// multiply into the clip coverage that clipped fills are scaled by; a group pushes the colour
// so far and continues on transparent, and its close blends it into the colour below at the
// group's alpha, in its blend mode (the W3C separable formulas, as the oracle's `blend`).
//
// Bitmap fills (ADR 0031) sample one of 16 texture slots (straight RGBA8, one texture per
// image) with the oracle's bilinear filter, colour effect and repeat rules
// (crates/render/src/paint.rs › sample), by hand: hardware filtering rounds its weights. A
// filtered group's layer (drawn and filtered by filters.wgsl) sits in `paints` after the gradient
// tables, and its fill reads it pixel for pixel.
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
@group(0) @binding(6) var img0: texture_2d<f32>;
@group(0) @binding(7) var img1: texture_2d<f32>;
@group(0) @binding(8) var img2: texture_2d<f32>;
@group(0) @binding(9) var img3: texture_2d<f32>;
@group(0) @binding(10) var img4: texture_2d<f32>;
@group(0) @binding(11) var img5: texture_2d<f32>;
@group(0) @binding(12) var img6: texture_2d<f32>;
@group(0) @binding(13) var img7: texture_2d<f32>;
@group(0) @binding(14) var img8: texture_2d<f32>;
@group(0) @binding(15) var img9: texture_2d<f32>;
@group(0) @binding(16) var img10: texture_2d<f32>;
@group(0) @binding(17) var img11: texture_2d<f32>;
@group(0) @binding(18) var img12: texture_2d<f32>;
@group(0) @binding(19) var img13: texture_2d<f32>;
@group(0) @binding(20) var img14: texture_2d<f32>;
@group(0) @binding(21) var img15: texture_2d<f32>;

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
const HAIRLINE: u32 = 2u;

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

// Distance from `p` to the segment `s` (x0, y0, x1, y1): hairline.rs › seg_dist, step by step.
fn seg_dist(p: vec2<f32>, s: vec4<f32>) -> f32 {
    let dx = s.z - s.x;
    let dy = s.w - s.y;
    let qx = p.x - s.x;
    let qy = p.y - s.y;
    let l = dx * dx + dy * dy;
    var t = 0.0;
    if l > 0.0 {
        t = clamp((qx * dx + qy * dy) / l, 0.0, 1.0);
    }
    let ex = qx - dx * t;
    let ey = qy - dy * t;
    return sqrt(ex * ex + ey * ey);
}

fn fold(w: f32, rule: u32) -> f32 {
    if rule == 0u {
        return min(abs(w), 1.0);
    }
    let t = abs(w) % 2.0;
    return select(t, 2.0 - t, t > 1.0);
}

fn texel(slot: u32, p: vec2<i32>) -> vec4<f32> {
    switch slot {
        case 0u: { return textureLoad(img0, p, 0); }
        case 1u: { return textureLoad(img1, p, 0); }
        case 2u: { return textureLoad(img2, p, 0); }
        case 3u: { return textureLoad(img3, p, 0); }
        case 4u: { return textureLoad(img4, p, 0); }
        case 5u: { return textureLoad(img5, p, 0); }
        case 6u: { return textureLoad(img6, p, 0); }
        case 7u: { return textureLoad(img7, p, 0); }
        case 8u: { return textureLoad(img8, p, 0); }
        case 9u: { return textureLoad(img9, p, 0); }
        case 10u: { return textureLoad(img10, p, 0); }
        case 11u: { return textureLoad(img11, p, 0); }
        case 12u: { return textureLoad(img12, p, 0); }
        case 13u: { return textureLoad(img13, p, 0); }
        case 14u: { return textureLoad(img14, p, 0); }
        case 15u: { return textureLoad(img15, p, 0); }
        default: { return vec4<f32>(0.0); }
    }
}

// Finite (not NaN or infinite), from the bits: comparisons with NaN may be optimized away.
fn finite(v: f32) -> bool {
    return (bitcast<u32>(v) & 0x7f800000u) != 0x7f800000u;
}

// Rust's `rem_euclid` for i32 and `n` >= 1, without `%` on a negative operand: naga writes
// WGSL's `%` as GLSL's, which leaves negative operands undefined, so on the GL backend
// (llvmpipe in the FreeBSD VM) the texel left of or above a repeating bitmap's edge, -1, came
// out wrong. The unsigned remainder is defined everywhere.
fn wrap(i: i32, n: i32) -> i32 {
    let r = i32(u32(abs(i)) % u32(n));
    return select(r, select(n - r, 0, r == 0), i < 0);
}

// One bitmap tap at integer (x, y) (wrapped when repeating, then clamped to the edges as
// `Pixels::at` does), through the colour effect, premultiplied.
// The colour transform (ColorTransform::apply): each straight channel `c·mul + add`, clamped.
fn tap(slot: u32, x: i32, y: i32, size: vec2<i32>, repeat: bool, mul: vec4<f32>, add: vec4<f32>) -> vec4<f32> {
    var q = vec2<i32>(x, y);
    if repeat {
        q = vec2<i32>(wrap(q.x, size.x), wrap(q.y, size.y));
    }
    q = clamp(q, vec2<i32>(0), size - vec2<i32>(1));
    let c = clamp(texel(slot, q) * mul + add, vec4<f32>(0.0), vec4<f32>(1.0));
    return vec4<f32>(c.rgb * c.a, c.a);
}

// Rust's `rem_euclid` for f32.
fn rem_euclid(u: f32, n: f32) -> f32 {
    let r = u - n * trunc(u / n);
    return select(r, r + n, r < 0.0);
}

// The oracle's sample of a bitmap at pixel centre `p` (paint.rs › sample): bilinear, or the
// pixel `p` falls in when the bitmap doesn't allow smoothing (flag bit 1).
fn bitmap(base: u32, ramp: u32, p: vec2<f32>) -> vec4<f32> {
    let h0 = paints[base];
    let m = paints[base + 1u];
    let n = paints[base + 2u];
    let mul = paints[ramp];
    let add = paints[ramp + 1u];
    let slot = bitcast<u32>(h0.x);
    let flags = bitcast<u32>(h0.y);
    let repeat = (flags & 1u) != 0u;
    let nearest = (flags & 2u) != 0u;
    let size = vec2<i32>(i32(bitcast<u32>(h0.z)), i32(bitcast<u32>(h0.w)));
    let w = f32(size.x);
    let h = f32(size.y);
    var u = m.x * p.x + m.z * p.y + n.x;
    var v = m.y * p.x + m.w * p.y + n.y;
    if !(finite(u) && finite(v)) || w < 1.0 || h < 1.0 {
        return vec4<f32>(0.0);
    }
    if repeat {
        u = rem_euclid(u, w);
        v = rem_euclid(v, h);
    } else if u < 0.0 || v < 0.0 || u >= w || v >= h {
        return vec4<f32>(0.0);
    }
    if nearest {
        return tap(slot, i32(floor(u)), i32(floor(v)), size, repeat, mul, add);
    }
    let fx_ = u - 0.5;
    let fy_ = v - 0.5;
    let x0 = floor(fx_);
    let y0 = floor(fy_);
    let tx = fx_ - x0;
    let ty = fy_ - y0;
    let xi = i32(x0);
    let yi = i32(y0);
    let a = tap(slot, xi, yi, size, repeat, mul, add);
    let b = tap(slot, xi + 1, yi, size, repeat, mul, add);
    let c = tap(slot, xi, yi + 1, size, repeat, mul, add);
    let d = tap(slot, xi + 1, yi + 1, size, repeat, mul, add);
    let top = a + (b - a) * tx;
    let bottom = c + (d - c) * tx;
    return top + (bottom - top) * ty;
}

// A filtered layer's pixel at (x, y): `w × h` premultiplied colours at `at` in `paints`, its
// top-left at (x0, y0); transparent outside (paint.rs › Shader::Image).
fn layer(base: u32, x: u32, y: u32) -> vec4<f32> {
    let h0 = paints[base];
    let o = paints[base + 1u];
    let dx = i32(x) - bitcast<i32>(o.x);
    let dy = i32(y) - bitcast<i32>(o.y);
    let w = i32(bitcast<u32>(h0.y));
    let h = i32(bitcast<u32>(h0.z));
    if dx < 0 || dy < 0 || dx >= w || dy >= h {
        return vec4<f32>(0.0);
    }
    return paints[bitcast<u32>(h0.x) + u32(dy * w + dx)];
}

fn shade(op: u32, kind: u32, ramp: u32, p: vec2<f32>, x: u32, y: u32) -> vec4<f32> {
    let base = op * PAINT_VEC4S;
    if kind == 0u {
        return paints[base];
    }
    if kind == 3u {
        return bitmap(base, ramp, p);
    }
    if kind == 4u {
        return layer(base, x, y);
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
    // Per channel, without a loop over components: FXC (Direct3D's shader compiler) must unroll
    // loops that index vectors, and can't unroll these.
    let f = vec3<f32>(
        blend_channel(m, unpremul(b.r, ba), unpremul(s.r, sa)),
        blend_channel(m, unpremul(b.g, ba), unpremul(s.g, sa)),
        blend_channel(m, unpremul(b.b, ba), unpremul(s.b, sa)),
    );
    return vec4<f32>((1.0 - ba) * s.rgb + (1.0 - sa) * b.rgb + sa * ba * f, sa + ba - sa * ba);
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
        let h = params.at.y + op * 4u;
        let rule = index[h + 1u];
        var cov = 0.0;
        if rule == HAIRLINE {
            var d = 2.0;
            let pc = vec2<f32>(px + 0.5, py + 0.5);
            for (var k = 0u; k < pieces; k++) {
                let sg = geo[first + k];
                // Out of reach of the segment's box: at least 1 away, so it can't be nearest
                // within reach (the coverage comes out the same, without the division).
                let lo = min(sg.xy, sg.zw) - vec2<f32>(1.0);
                let hi = max(sg.xy, sg.zw) + vec2<f32>(1.0);
                if any(pc < lo) || any(pc > hi) {
                    continue;
                }
                d = min(d, seg_dist(pc, sg));
            }
            cov = clamp(1.0 - d, 0.0, 1.0);
        } else {
            var w = f32(bitcast<i32>(index[e + 3u]));
            for (var k = 0u; k < pieces; k++) {
                w += piece_cover(geo[first + k], px, py);
            }
            cov = fold(w, rule);
        }
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
        let s = shade(op, index[h], index[h + 2u], centre, x, y);
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
