//! A small Lottie player for tests (ADR 0089), written from the public Lottie format
//! description and independent of the exporter's code: it reads the JSON, evaluates every
//! keyframed property at a frame (holds, and Bézier eases solved on their own), and turns the
//! shape layers into a display list for the CPU oracle to draw.
//!
//! It plays what AniCraft writes: shape layers (paths, fills, linear and radial gradient
//! fills, strokes with caps, joins and dashes, group transforms with position — whole or
//! split — anchor, scale, rotation, skew and opacity), layer masks (added), track mattes
//! (alpha and luma) and the solid stage layer. Group opacity fades each shape, as Lottie's
//! shape groups do. A stroke under a stretched or skewed group would need Lottie's stretched
//! pen, which the oracle doesn't draw: it fails the test rather than being drawn wrongly.

use anicraft_engine::doc::{Cap, ColorTransform, Dash, DisplayItem, DisplayList, ElementId, GradientStop, Join, LayerId, MaskMode, Paint, Rgba, Stroke, StrokeStyle};
use anicraft_engine::geom::{Affine, FillRule, Path, PathCmd, Vec2};
use serde_json::Value;
use std::sync::Arc;

/// Numbers of a value: a number, or an array of numbers.
fn nums(v: &Value) -> Vec<f64> {
    match v {
        Value::Array(a) => a.iter().filter_map(Value::as_f64).collect(),
        Value::Number(x) => x.as_f64().into_iter().collect(),
        _ => Vec::new(),
    }
}

/// An ease handle's coordinate: a number or the first of an array.
fn first(v: &Value) -> f64 {
    nums(v).first().copied().unwrap_or(0.0)
}

/// CSS / Lottie cubic-bezier ease from (0,0) to (1,1) through (x1,y1), (x2,y2), at time x:
/// x(s) solved by bisection (monotonic for x handles within 0..1).
fn ease(x1: f64, y1: f64, x2: f64, y2: f64, x: f64) -> f64 {
    let b = |s: f64, p1: f64, p2: f64| 3.0 * (1.0 - s) * (1.0 - s) * s * p1 + 3.0 * (1.0 - s) * s * s * p2 + s * s * s;
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..80 {
        let mid = 0.5 * (lo + hi);
        if b(mid, x1, x2) < x { lo = mid } else { hi = mid }
    }
    b(0.5 * (lo + hi), y1, y2)
}

/// The keyframe pair around `t` and how far along: `(from, to, eased share)`.
fn segment(keys: &[Value], t: f64) -> (&Value, &Value, f64) {
    let start = |k: &Value| k["t"].as_f64().unwrap();
    let k0 = &keys[0];
    if t <= start(k0) {
        return (k0, k0, 0.0);
    }
    let i = keys.iter().rposition(|k| start(k) <= t).unwrap();
    let a = &keys[i];
    let Some(b) = keys.get(i + 1) else { return (a, a, 0.0) };
    if a["h"] == 1 {
        return (a, a, 0.0);
    }
    let x = (t - start(a)) / (start(b) - start(a));
    let y = ease(first(&a["o"]["x"]), first(&a["o"]["y"]), first(&a["i"]["x"]), first(&a["i"]["y"]), x);
    (a, b, y)
}

fn mix(a: &[f64], b: &[f64], y: f64) -> Vec<f64> {
    a.iter().zip(b).map(|(p, q)| p + (q - p) * y).collect()
}

/// A property (`{a, k}`, or a split position `{s: true, x, y}`) at frame `t`.
pub fn prop(p: &Value, t: f64) -> Vec<f64> {
    if p["s"] == true {
        return [prop(&p["x"], t), prop(&p["y"], t)].concat();
    }
    if p["a"] != 1 {
        return nums(&p["k"]);
    }
    let keys = p["k"].as_array().unwrap();
    let (a, b, y) = segment(keys, t);
    mix(&nums(&a["s"]), &nums(&b["s"]), y)
}

/// Points of a path.
type Points = Vec<[f64; 2]>;

/// A path property at frame `t`: vertices, in and out tangents, closed.
fn shape(p: &Value, t: f64) -> (Points, Points, Points, bool) {
    let flat = |b: &Value, k: &str| -> Vec<f64> { b[k].as_array().unwrap().iter().flat_map(nums).collect() };
    let parts = |b: &Value| (flat(b, "v"), flat(b, "i"), flat(b, "o"), b["c"] == true);
    let (v, i, o, c) = if p["a"] != 1 {
        parts(&p["k"])
    } else {
        let keys = p["k"].as_array().unwrap();
        let (a, b, y) = segment(keys, t);
        let (pa, pb) = (parts(&a["s"][0]), parts(&b["s"][0]));
        (mix(&pa.0, &pb.0, y), mix(&pa.1, &pb.1, y), mix(&pa.2, &pb.2, y), pa.3)
    };
    let pairs = |f: Vec<f64>| f.chunks(2).map(|c| [c[0], c[1]]).collect::<Vec<_>>();
    (pairs(v), pairs(i), pairs(o), c)
}

fn path_of(p: &Value, t: f64) -> Path {
    let (v, i, o, closed) = shape(p, t);
    let pt = |q: [f64; 2]| Vec2::new(q[0] as f32, q[1] as f32);
    let mut path = Path::new();
    if v.is_empty() {
        return path;
    }
    path.cmds.push(PathCmd::MoveTo(pt(v[0])));
    let n = v.len();
    let segs = if closed { n } else { n - 1 };
    for k in 0..segs {
        let (a, b) = (k, (k + 1) % n);
        let c1 = [v[a][0] + o[a][0], v[a][1] + o[a][1]];
        let c2 = [v[b][0] + i[b][0], v[b][1] + i[b][1]];
        path.cmds.push(PathCmd::CubicTo(pt(c1), pt(c2), pt(v[b])));
    }
    if closed {
        path.cmds.push(PathCmd::Close);
    }
    path
}

/// A group transform (`tr`) at `t`: T(p) · R(r) · R(−sa) · Shear(−tan sk) · R(sa) · S(s) · T(−a),
/// and its opacity.
fn transform(tr: &Value, t: f64) -> (Affine, f64) {
    let get = |k: &str, d: &[f64]| if tr.get(k).is_some() { prop(&tr[k], t) } else { d.to_vec() };
    let (p, a, s) = (get("p", &[0.0, 0.0]), get("a", &[0.0, 0.0]), get("s", &[100.0, 100.0]));
    let (r, sk, sa, o) = (get("r", &[0.0])[0], get("sk", &[0.0])[0], get("sa", &[0.0])[0], get("o", &[100.0])[0]);
    let rot = |deg: f64| Affine::rotate(deg.to_radians() as f32);
    let shear = Affine { m: [1.0, 0.0, -(sk.to_radians().tan()) as f32, 1.0, 0.0, 0.0] };
    let m = Affine::translate(p[0] as f32, p[1] as f32)
        .mul(&rot(r))
        .mul(&rot(-sa))
        .mul(&shear)
        .mul(&rot(sa))
        .mul(&Affine::scale((s[0] / 100.0) as f32, (s[1] / 100.0) as f32))
        .mul(&Affine::translate(-a[0] as f32, -a[1] as f32));
    (m, o / 100.0)
}

fn item(path: Path, transform: Affine, fill: Option<Paint>, stroke: Option<Stroke>, rule: FillRule) -> DisplayItem {
    DisplayItem {
        path: Arc::new(path),
        transform,
        fill,
        stroke,
        fill_rule: rule,
        color: ColorTransform::IDENTITY,
        layer: LayerId(0),
        element: ElementId(0),
        outline: None,
        clip: None,
        masks: None,
        guide: false,
        group: None,
        image: None,
        smooth: true,
        warp: None,
    }
}

fn rgba(c: &[f64], alpha: f64) -> Rgba {
    Rgba::new(c[0] as f32, c[1] as f32, c[2] as f32, alpha.clamp(0.0, 1.0) as f32)
}

/// The items one shape group draws at `t`, bottom first, faded by `fade`.
fn group(g: &Value, t: f64, fade: f64, out: &mut Vec<DisplayItem>) {
    let it = g["it"].as_array().unwrap();
    let tr = it.iter().find(|x| x["ty"] == "tr").unwrap();
    let (m, o) = transform(tr, t);
    let fade = fade * o;
    let mut path = Path::new();
    for sh in it.iter().filter(|x| x["ty"] == "sh") {
        path.cmds.extend(path_of(&sh["ks"], t).cmds);
    }
    let rule = |x: &Value| if x["r"] == 2 { FillRule::EvenOdd } else { FillRule::NonZero };
    // Listed top first: the stroke (above) comes before the fill.
    for x in it.iter().rev() {
        match x["ty"].as_str().unwrap_or("") {
            "fl" => {
                let c = prop(&x["c"], t);
                let o = prop(&x["o"], t)[0] / 100.0;
                out.push(item(path.clone(), m, Some(Paint::Solid(rgba(&c, o * fade))), None, rule(x)));
            }
            "gf" => {
                let (s, e) = (prop(&x["s"], t), prop(&x["e"], t));
                let count = x["g"]["p"].as_u64().unwrap() as usize;
                let k = prop(&x["g"]["k"], t);
                let o = prop(&x["o"], t)[0] / 100.0;
                let stops: Vec<GradientStop> = (0..count)
                    .map(|j| {
                        let c = &k[j * 4..j * 4 + 4];
                        let a = k.get(count * 4 + j * 2 + 1).copied().unwrap_or(1.0);
                        GradientStop { offset: c[0] as f32, color: rgba(&c[1..], a * o * fade) }
                    })
                    .collect();
                let (s, e) = (Vec2::new(s[0] as f32, s[1] as f32), Vec2::new(e[0] as f32, e[1] as f32));
                let paint = if x["t"] == 2 { Paint::radial(s, (e - s).length(), stops) } else { Paint::linear(s, e, stops) };
                out.push(item(path.clone(), m, Some(paint), None, rule(x)));
            }
            "st" => {
                let d = m.decompose();
                assert!((d.sx.abs() - d.sy.abs()).abs() <= 1e-3 * d.sx.abs().max(1e-3) && d.skew.abs() < 1e-3, "a stroke under a stretched group: {:?}", m);
                let c = prop(&x["c"], t);
                let o = prop(&x["o"], t)[0] / 100.0;
                let cap = match x["lc"].as_u64() { Some(1) => Cap::Butt, Some(3) => Cap::Square, _ => Cap::Round };
                let join = match x["lj"].as_u64() { Some(1) => Join::Miter, Some(3) => Join::Bevel, _ => Join::Round };
                let dash = x.get("d").and_then(Value::as_array).map(|d| {
                    let lengths: Vec<f32> = d.iter().filter(|e| e["n"] != "o").map(|e| prop(&e["v"], t)[0] as f32).collect();
                    let offset = d.iter().find(|e| e["n"] == "o").map_or(0.0, |e| prop(&e["v"], t)[0] as f32);
                    Dash::Dashed { lengths: lengths.into(), offset }
                });
                let style = StrokeStyle { cap, join, miter_limit: x["ml"].as_f64().unwrap_or(4.0) as f32, dash };
                let st = Stroke::new(rgba(&c, o * fade), prop(&x["w"], t)[0] as f32).with_style(style);
                out.push(item(path.clone(), m, None, Some(st), FillRule::NonZero));
            }
            _ => {}
        }
    }
}

/// What a layer draws at `t`, bottom first.
fn layer_items(l: &Value, t: f64) -> Vec<DisplayItem> {
    let mut out = Vec::new();
    match l["ty"].as_u64() {
        Some(1) => {
            let (w, h) = (l["sw"].as_f64().unwrap() as f32, l["sh"].as_f64().unwrap() as f32);
            let c = Rgba::parse_hex(l["sc"].as_str().unwrap()).unwrap();
            out.push(item(Path::rect(0.0, 0.0, w, h), Affine::IDENTITY, Some(Paint::Solid(c)), None, FillRule::NonZero));
        }
        Some(4) => {
            for g in l["shapes"].as_array().unwrap().iter().rev() {
                group(g, t, 1.0, &mut out);
            }
        }
        _ => {}
    }
    out
}

/// The animation at frame `t` as a display list (layer transforms are the identity, as
/// AniCraft writes them).
pub fn frame(anim: &Value, t: f64) -> DisplayList {
    let mut dl = DisplayList::default();
    let layers = anim["layers"].as_array().unwrap();
    let shows = |l: &Value| l["ip"].as_f64().unwrap() <= t && t < l["op"].as_f64().unwrap();
    // Bottom first; a matte (`td`) is the layer just above the one that uses it (`tt`).
    for (i, l) in layers.iter().enumerate().rev() {
        if l["td"] == 1 || !shows(l) {
            continue;
        }
        let mut items = layer_items(l, t);
        if let Some(mode) = l["tt"].as_u64() {
            let matte = &layers[i - 1];
            let c = dl.clip_parents.len() as u32;
            dl.clip_parents.push(None);
            dl.set_clip_mode(c, if mode == 3 { MaskMode::Luminance } else { MaskMode::Alpha });
            if shows(matte) {
                for mut m in layer_items(matte, t) {
                    m.masks = Some(c);
                    dl.items.push(m);
                }
            }
            for it in &mut items {
                it.clip = Some(c);
            }
        }
        if let Some(masks) = l["masksProperties"].as_array() {
            let c = dl.clip_parents.len() as u32;
            dl.clip_parents.push(None);
            for m in masks {
                assert_eq!(m["mode"], "a", "added masks only");
                let mut it = item(path_of(&m["pt"], t), Affine::IDENTITY, Some(Paint::Solid(Rgba::WHITE)), None, FillRule::NonZero);
                it.masks = Some(c);
                dl.items.push(it);
            }
            for it in &mut items {
                assert!(it.clip.is_none(), "a mask and a matte on one layer");
                it.clip = Some(c);
            }
        }
        dl.items.extend(items);
    }
    dl
}
