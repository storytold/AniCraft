// AniCraft GPU stage: filters on a filtered group's layer (ADR 0031).
//
// The same steps as the CPU oracle's filters (crates/render/src/filters.rs), resolved by it
// (`filters::resolve`: box widths, whole-pixel offsets, premultiplied colours, ramp tables, the
// colour matrix), in f32 on premultiplied colour:
//
// * `box4` / `box1`: one box-blur pass along rows or columns, one invocation per line with the
//   oracle's running sum (an even width covers one more pixel each side at half weight), from
//   one buffer half or coverage slot into another. Outside the layer is transparent.
// * `coverage`: the layer's opacity (or its transparency) moved by (dx, dy) into a slot.
// * `shadow`, `bevel`, `gradient_glow`: compose the blurred coverage with the layer, per pixel.
// * `adjust`: the colour matrix on unpremultiplied colour.
//
// `img` holds two `w × h` halves (the layer is half 0); `cov` three `w × h` coverage slots.

struct Params {
    // width, height, box width, 1 along rows / 0 along columns
    size: vec4<u32>,
    // source, destination (halves or slots), flags, side (0 inner, 1 outer, 2 full)
    io: vec4<u32>,
    // dx, dy
    offset: vec4<i32>,
    // shadow colour (straight) or bevel highlight (premultiplied)
    c0: vec4<f32>,
    // bevel shadow (premultiplied)
    c1: vec4<f32>,
    // strength
    k: vec4<f32>,
    // the colour matrix's rows (r, g, b, a: columns r, g, b, a), then its offsets
    m: array<vec4<f32>, 5>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read_write> img: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read_write> cov: array<f32>;
@group(0) @binding(3) var<storage, read> ramp: array<vec4<f32>>;

const INNER: u32 = 1u;
const KNOCKOUT: u32 = 2u;
const HIDE: u32 = 4u;
const INVERSE: u32 = 8u;
const RAMP: u32 = 16u;
const SIDE_INNER: u32 = 0u;
const SIDE_OUTER: u32 = 1u;
// Ramp table steps (filters.rs › RAMP_STEPS).
const RAMP_STEPS: f32 = 256.0;

fn npx() -> u32 {
    return params.size.x * params.size.y;
}

// Pixel (x, y) of the layer, transparent outside.
fn alpha_at(x: i32, y: i32) -> f32 {
    let w = i32(params.size.x);
    let h = i32(params.size.y);
    if x < 0 || y < 0 || x >= w || y >= h {
        return 0.0;
    }
    return img[u32(y * w + x)].w;
}

fn cov_at(slot: u32, x: i32, y: i32) -> f32 {
    let w = i32(params.size.x);
    let h = i32(params.size.y);
    if x < 0 || y < 0 || x >= w || y >= h {
        return 0.0;
    }
    return cov[slot * npx() + u32(y * w + x)];
}

// The line `line` and element `i` of a box pass, as an index into a `w × h` buffer.
fn line_index(line: u32, i: u32) -> u32 {
    if params.size.w == 1u {
        return line * params.size.x + i;
    }
    return i * params.size.x + line;
}

fn lines() -> vec2<u32> {
    // (lines, length)
    if params.size.w == 1u {
        return vec2<u32>(params.size.y, params.size.x);
    }
    return vec2<u32>(params.size.x, params.size.y);
}

@compute @workgroup_size(64)
fn box4(@builtin(global_invocation_id) gid: vec3<u32>) {
    let ll = lines();
    let line = gid.x;
    if line >= ll.x {
        return;
    }
    let len = ll.y;
    let src = params.io.x * npx();
    let dst = params.io.y * npx();
    let n = params.size.z;
    if n <= 1u {
        for (var i = 0u; i < len; i++) {
            img[dst + line_index(line, i)] = img[src + line_index(line, i)];
        }
        return;
    }
    let r = n / 2u;
    let even = (n % 2u) == 0u;
    let inv = 1.0 / f32(n);
    var sum = vec4<f32>(0.0);
    for (var i = 0u; i <= min(r, len - 1u); i++) {
        sum += img[src + line_index(line, i)];
    }
    for (var i = 0u; i < len; i++) {
        var ends = vec4<f32>(0.0);
        if even {
            var lo = vec4<f32>(0.0);
            var hi = vec4<f32>(0.0);
            if i >= r {
                lo = img[src + line_index(line, i - r)];
            }
            if i + r < len {
                hi = img[src + line_index(line, i + r)];
            }
            ends = 0.5 * (lo + hi);
        }
        img[dst + line_index(line, i)] = (sum - ends) * inv;
        if i + r + 1u < len {
            sum += img[src + line_index(line, i + r + 1u)];
        }
        if i >= r {
            sum -= img[src + line_index(line, i - r)];
        }
    }
}

@compute @workgroup_size(64)
fn box1(@builtin(global_invocation_id) gid: vec3<u32>) {
    let ll = lines();
    let line = gid.x;
    if line >= ll.x {
        return;
    }
    let len = ll.y;
    let src = params.io.x * npx();
    let dst = params.io.y * npx();
    let n = params.size.z;
    if n <= 1u {
        for (var i = 0u; i < len; i++) {
            cov[dst + line_index(line, i)] = cov[src + line_index(line, i)];
        }
        return;
    }
    let r = n / 2u;
    let even = (n % 2u) == 0u;
    let inv = 1.0 / f32(n);
    var sum = 0.0;
    for (var i = 0u; i <= min(r, len - 1u); i++) {
        sum += cov[src + line_index(line, i)];
    }
    for (var i = 0u; i < len; i++) {
        var ends = 0.0;
        if even {
            var lo = 0.0;
            var hi = 0.0;
            if i >= r {
                lo = cov[src + line_index(line, i - r)];
            }
            if i + r < len {
                hi = cov[src + line_index(line, i + r)];
            }
            ends = 0.5 * (lo + hi);
        }
        cov[dst + line_index(line, i)] = (sum - ends) * inv;
        if i + r + 1u < len {
            sum += cov[src + line_index(line, i + r + 1u)];
        }
        if i >= r {
            sum -= cov[src + line_index(line, i - r)];
        }
    }
}

// Pixel (x, y) of a per-pixel pass, or none past the layer's edge.
fn here(gid: vec3<u32>) -> bool {
    return gid.x < params.size.x && gid.y < params.size.y;
}

@compute @workgroup_size(16, 16)
fn coverage(@builtin(global_invocation_id) gid: vec3<u32>) {
    if !here(gid) {
        return;
    }
    let a = alpha_at(i32(gid.x) - params.offset.x, i32(gid.y) - params.offset.y);
    let i = gid.y * params.size.x + gid.x;
    cov[params.io.y * npx() + i] = select(a, 1.0 - a, (params.io.z & INVERSE) != 0u);
}

@compute @workgroup_size(16, 16)
fn shadow(@builtin(global_invocation_id) gid: vec3<u32>) {
    if !here(gid) {
        return;
    }
    let i = gid.y * params.size.x + gid.x;
    let flags = params.io.z;
    let c = params.c0;
    let sa = clamp(cov[i] * params.k.x, 0.0, 1.0) * c.a;
    let src = img[i];
    let ba = src.a;
    var out: vec4<f32>;
    if (flags & INNER) != 0u {
        // Inside the artwork, over it.
        let a = sa * ba;
        if (flags & KNOCKOUT) != 0u {
            out = vec4<f32>(c.rgb * a, a);
        } else {
            out = vec4<f32>(c.rgb * a + src.rgb * (1.0 - sa), ba);
        }
    } else if (flags & HIDE) != 0u {
        out = vec4<f32>(c.rgb * sa, sa);
    } else if (flags & KNOCKOUT) != 0u {
        let a = sa * (1.0 - ba);
        out = vec4<f32>(c.rgb * a, a);
    } else {
        // Under the artwork.
        let k = 1.0 - ba;
        out = vec4<f32>(src.rgb + c.rgb * sa * k, ba + sa * k);
    }
    img[i] = out;
}

fn ramp_at(t: f32) -> vec4<f32> {
    // Round half up, as Rust's `round` does for t >= 0.
    let i = u32(floor(clamp(t, 0.0, 1.0) * RAMP_STEPS + 0.5));
    return ramp[min(i, u32(RAMP_STEPS))];
}

// An effect with the artwork: `ei` atop it (inside its edge), `eo` under it (outside), as the
// side says; Knockout keeps only the effect (filters.rs › composite).
fn compose(src: vec4<f32>, inner: vec4<f32>, outer: vec4<f32>) -> vec4<f32> {
    let side = params.io.w;
    let ei = select(inner, vec4<f32>(0.0), side == SIDE_OUTER);
    let eo = select(outer, vec4<f32>(0.0), side == SIDE_INNER);
    let ba = src.a;
    if (params.io.z & KNOCKOUT) != 0u {
        return ei * ba + eo * (1.0 - ba);
    }
    return ei * ba + src * (1.0 - ei.a) + eo * (1.0 - ba);
}

@compute @workgroup_size(16, 16)
fn bevel(@builtin(global_invocation_id) gid: vec3<u32>) {
    if !here(gid) {
        return;
    }
    let x = i32(gid.x);
    let y = i32(gid.y);
    let dx = params.offset.x;
    let dy = params.offset.y;
    let d = clamp(cov_at(0u, x + dx, y + dy) - cov_at(0u, x - dx, y - dy), -1.0, 1.0);
    let v = clamp(d * params.k.x, -1.0, 1.0);
    var effect: vec4<f32>;
    if (params.io.z & RAMP) != 0u {
        effect = ramp_at(0.5 - 0.5 * v);
    } else if v >= 0.0 {
        effect = params.c0 * v;
    } else {
        effect = params.c1 * -v;
    }
    let i = gid.y * params.size.x + gid.x;
    img[i] = compose(img[i], effect, effect);
}

@compute @workgroup_size(16, 16)
fn gradient_glow(@builtin(global_invocation_id) gid: vec3<u32>) {
    if !here(gid) {
        return;
    }
    let i = gid.y * params.size.x + gid.x;
    let side = params.io.w;
    var ei = vec4<f32>(0.0);
    var eo = vec4<f32>(0.0);
    if side != SIDE_OUTER {
        ei = ramp_at(clamp(cov[i] * params.k.x, 0.0, 1.0));
    }
    if side != SIDE_INNER {
        eo = ramp_at(clamp(cov[npx() + i] * params.k.x, 0.0, 1.0));
    }
    img[i] = compose(img[i], ei, eo);
}

@compute @workgroup_size(16, 16)
fn adjust(@builtin(global_invocation_id) gid: vec3<u32>) {
    if !here(gid) {
        return;
    }
    let i = gid.y * params.size.x + gid.x;
    let p = img[i];
    let a = p.a;
    if a <= 1e-6 {
        return;
    }
    let r = p.r / a;
    let g = p.g / a;
    let b = p.b / a;
    // All four rows at once (the matrix is stored by columns), summed in the oracle's order.
    let o = clamp(params.m[0] * r + params.m[1] * g + params.m[2] * b + params.m[3] * a + params.m[4], vec4<f32>(0.0), vec4<f32>(1.0));
    img[i] = vec4<f32>(o.rgb * o.a, o.a);
}
