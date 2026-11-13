// AniCraft GPU stage: the prefix sum for the GPU binning (ADR 0048, bin.wgsl).
//
// An exclusive scan of `n` values in `values` (the binning's working buffer): each workgroup
// scans 1024 of them (four per invocation, then the invocations' totals in workgroup memory)
// and leaves its total; the totals are scanned the same way, level by level, and added back.
// The public work-efficient scan (Blelloch's "Prefix sums and their applications", 1990), in
// its simplest form.

// `src`, `dst` and the workgroups' totals (`sums`) in `work`, and how many values.
struct ScanParams {
    src: u32,
    dst: u32,
    sums: u32,
    n: u32,
}

@group(0) @binding(0) var<uniform> sp: ScanParams;
@group(0) @binding(1) var<storage, read_write> values: array<u32>;

var<workgroup> sh: array<u32, 256>;

fn load(i: u32) -> u32 {
    if i < sp.n {
        return values[sp.src + i];
    }
    return 0u;
}

fn store(i: u32, v: u32) {
    if i < sp.n {
        values[sp.dst + i] = v;
    }
}

// Each workgroup scans 1024 values (four per invocation), exclusive, and leaves its total.
@compute @workgroup_size(256)
fn scan_blocks(@builtin(workgroup_id) wg: vec3<u32>, @builtin(num_workgroups) nw: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>) {
    let block = wg.y * nw.x + wg.x;
    let base = block * 1024u + lid.x * 4u;
    // Four values by name, not a loop over an array: FXC (Direct3D's compiler) must unroll
    // loops that index local arrays.
    let v0 = load(base);
    let v1 = load(base + 1u);
    let v2 = load(base + 2u);
    let v3 = load(base + 3u);
    let total = v0 + v1 + v2 + v3;
    sh[lid.x] = total;
    workgroupBarrier();
    for (var off = 1u; off < 256u; off <<= 1u) {
        var t = 0u;
        if lid.x >= off {
            t = sh[lid.x - off];
        }
        workgroupBarrier();
        sh[lid.x] += t;
        workgroupBarrier();
    }
    let acc = sh[lid.x] - total;
    store(base, acc);
    store(base + 1u, acc + v0);
    store(base + 2u, acc + v0 + v1);
    store(base + 3u, acc + v0 + v1 + v2);
    // (A dispatch spread over x and y may have a few workgroups past the end.)
    if lid.x == 255u && block * 1024u < sp.n {
        values[sp.sums + block] = sh[255];
    }
}

// Adds each workgroup's scanned total to its values.
@compute @workgroup_size(256)
fn scan_add(@builtin(workgroup_id) wg: vec3<u32>, @builtin(num_workgroups) nw: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>) {
    let block = wg.y * nw.x + wg.x;
    if block * 1024u >= sp.n {
        return;
    }
    let add = values[sp.sums + block];
    for (var i = 0u; i < 4u; i++) {
        let at = block * 1024u + lid.x * 4u + i;
        if at < sp.n {
            values[sp.dst + at] += add;
        }
    }
}
