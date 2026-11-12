//! Lottie export: the movie as a Lottie (Bodymovin JSON) animation, from the same evaluated
//! frames the renderer draws, written from the public Lottie format description.
//!
//! Frame-accurate rather than re-keyed: each frame is evaluated, and every piece of artwork
//! that persists from frame to frame (same layer, element and path structure) becomes one
//! shape layer, its outline baked into stage space. Whatever changes (outline, colours,
//! gradient points, stroke width, opacity, masks; colour effects of every style are baked into
//! the colours, ADR 0056) is written as hold keyframes at the frames
//! where it changes, so any Lottie player shows exactly AniCraft's frames. Strokes keep their
//! caps, joins, miter limit and dashes (dash lengths scaled like the width, dots as zero-length
//! round dashes; ADR 0046); variable-width strokes are filled outlines (the polygons the
//! renderer fills, dashes included).
//!
//! Not carried across (counted in the report): bitmap fills; nested masks keep only the
//! innermost mask; a symbol fading as one group is faded shape by shape; stretched radial
//! gradients become circles; and draw-order changes that one fixed layer order can't show.

use anicraft_doc::{DisplayItem, Document, Paint, Rgba, evaluate};
use anicraft_geom::{Affine, FillRule, Path, PathCmd, Vec2};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};

/// The result of an export.
pub struct LottieExport {
    pub json: String,
    /// Shape layers written (the background layer not counted).
    pub layers: usize,
    /// What couldn't be carried across exactly, by kind, with how many shape layers (or, for
    /// artwork left out, elements) it touched.
    pub report: BTreeMap<String, usize>,
}

/// Lottie numbers: three decimals are well below a pixel and keep files small.
fn n(v: f32) -> Value {
    let r = (f64::from(v) * 1000.0).round() / 1000.0;
    if r.is_finite() { json!(if r == 0.0 { 0.0 } else { r }) } else { json!(0.0) }
}

fn pt(p: Vec2) -> Value {
    json!([n(p.x), n(p.y)])
}

/// One Lottie bezier path: vertices with in and out tangents relative to them.
#[derive(Clone, PartialEq)]
struct Bez {
    closed: bool,
    v: Vec<Vec2>,
    i: Vec<Vec2>,
    o: Vec<Vec2>,
}

impl Bez {
    fn new(p: Vec2) -> Bez {
        Bez { closed: false, v: vec![p], i: vec![Vec2::ZERO], o: vec![Vec2::ZERO] }
    }
    fn to_value(&self) -> Value {
        json!({"c": self.closed, "v": self.v.iter().map(|p| pt(*p)).collect::<Vec<_>>(), "i": self.i.iter().map(|p| pt(*p)).collect::<Vec<_>>(), "o": self.o.iter().map(|p| pt(*p)).collect::<Vec<_>>()})
    }
}

/// A path's subpaths as Lottie beziers in stage space (quadratics become cubics).
fn beziers(path: &Path, m: &Affine) -> Vec<Bez> {
    let mut out: Vec<Bez> = Vec::new();
    let mut cur: Option<Bez> = None;
    let mut start = Vec2::ZERO;
    let finish = |cur: &mut Option<Bez>, out: &mut Vec<Bez>| {
        if let Some(mut b) = cur.take() {
            if b.closed && b.v.len() > 1 && b.v.first().zip(b.v.last()).is_some_and(|(a, z)| (*a - *z).length() < 1e-4) {
                // The closing point repeats the first: its in-tangent belongs to the first.
                let last_in = b.i.pop().unwrap_or(Vec2::ZERO);
                b.v.pop();
                b.o.pop();
                if let Some(first) = b.i.first_mut() {
                    *first = last_in;
                }
            }
            if b.v.len() > 1 || b.closed {
                out.push(b);
            }
        }
    };
    for cmd in &path.cmds {
        match *cmd {
            PathCmd::MoveTo(p) => {
                finish(&mut cur, &mut out);
                start = p;
                cur = Some(Bez::new(p));
            }
            PathCmd::Close => {
                if let Some(b) = cur.as_mut() {
                    b.closed = true;
                }
                finish(&mut cur, &mut out);
            }
            _ => {
                let b = cur.get_or_insert_with(|| Bez::new(start));
                if b.closed {
                    continue;
                }
                let p0 = b.v.last().copied().unwrap_or(start);
                let (c1, c2, p) = match *cmd {
                    PathCmd::LineTo(p) => (p0, p, p),
                    PathCmd::QuadTo(c, p) => (p0 + (c - p0) * (2.0 / 3.0), p + (c - p) * (2.0 / 3.0), p),
                    PathCmd::CubicTo(c1, c2, p) => (c1, c2, p),
                    PathCmd::MoveTo(_) | PathCmd::Close => continue,
                };
                if let Some(o) = b.o.last_mut() {
                    *o = c1 - p0;
                }
                b.v.push(p);
                b.i.push(c2 - p);
                b.o.push(Vec2::ZERO);
            }
        }
    }
    finish(&mut cur, &mut out);
    for b in &mut out {
        for p in &mut b.v {
            *p = m.apply(*p);
        }
        for t in b.i.iter_mut().chain(b.o.iter_mut()) {
            *t = m.apply_vector(*t);
        }
    }
    out
}

/// Fill of one frame.
#[derive(Clone, PartialEq)]
enum Fill {
    Solid { color: Rgba, rule: FillRule },
    Gradient { radial: bool, start: Vec2, end: Vec2, stops: Vec<(f32, Rgba)>, rule: FillRule },
}

/// One piece of artwork in one frame, in stage space.
#[derive(Clone, PartialEq)]
struct Sample {
    paths: Vec<Bez>,
    fill: Option<Fill>,
    stroke: Option<Line>,
    /// Group opacity (a fading symbol), applied to the shape as a whole.
    opacity: f32,
    /// Mask outlines this artwork shows through (their union).
    masks: Vec<Bez>,
}

/// A stroke in one frame, in stage space.
#[derive(Clone, PartialEq)]
struct Line {
    color: Rgba,
    width: f32,
    /// Lottie's line cap and join numbers (1 butt / miter, 2 round, 3 square / bevel).
    cap: u8,
    join: u8,
    miter: f32,
    /// On and off lengths (stage units), and the offset into them.
    dash: Vec<f32>,
    offset: f32,
}

impl Line {
    /// A round, solid line (outline mode's hairline).
    fn round(color: Rgba, width: f32) -> Line {
        Line { color, width, cap: 2, join: 2, miter: 4.0, dash: Vec::new(), offset: 0.0 }
    }

    fn of(st: &anicraft_doc::Stroke, color: Rgba, k: f32) -> Line {
        use anicraft_doc::{Cap, Join};
        let style = &st.style;
        let cap = match style.effective_cap() {
            Cap::Butt => 1,
            Cap::Round => 2,
            Cap::Square => 3,
        };
        let join = match style.join {
            Join::Miter => 1,
            Join::Round => 2,
            Join::Bevel => 3,
        };
        let (dash, offset) = style.dash.as_ref().map_or((Vec::new(), 0.0), |d| {
            let (l, o) = d.pattern(st.width);
            (l.into_iter().map(|x| x * k).collect(), o * k)
        });
        Line { color, width: st.width * k, cap, join, miter: style.miter_limit, dash, offset }
    }
}

/// Subpaths as (closed, vertex count): what must match for values to be keyframed.
type Structure = Vec<(bool, usize)>;

/// Artwork that persists from frame to frame becomes one layer: same layer, element and
/// occurrence, and the same structure (so its values can be keyframed).
#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    layer: u64,
    element: u64,
    occurrence: u32,
    shape: Structure,
    fill: u8,
    stops: usize,
    /// The stroke's cap, join and dash entry count (its lengths may change from frame to frame).
    stroke: Option<(u8, u8, usize)>,
    masks: Structure,
    /// Lottie layer blend mode (`bm`).
    blend: u8,
}

struct Track {
    key: Key,
    name: String,
    first: u32,
    last: u32,
    frames: Vec<(u32, Sample)>,
    /// Draw position in its first frame (tie-break for the layer order).
    first_index: usize,
}

/// A property: static when it never changes, else hold keyframes where it changes.
fn prop(values: &[(u32, Value)]) -> Value {
    let mut keys: Vec<(u32, &Value)> = Vec::new();
    for (t, v) in values {
        if keys.last().is_none_or(|(_, last)| *last != v) {
            keys.push((*t, v));
        }
    }
    match keys.as_slice() {
        [] => json!({"a": 0, "k": 0}),
        [(_, v)] => json!({"a": 0, "k": v}),
        _ => json!({"a": 1, "k": keys.iter().map(|(t, v)| json!({"t": t, "s": if v.is_array() { (*v).clone() } else { json!([v]) }, "h": 1})).collect::<Vec<_>>()}),
    }
}

/// A shape property animates `[path]` values in keyframes, so it is built separately.
fn shape_prop(values: &[(u32, Value)]) -> Value {
    let mut keys: Vec<(u32, &Value)> = Vec::new();
    for (t, v) in values {
        if keys.last().is_none_or(|(_, last)| *last != v) {
            keys.push((*t, v));
        }
    }
    match keys.as_slice() {
        [(_, v)] => json!({"a": 0, "k": v}),
        _ => json!({"a": 1, "k": keys.iter().map(|(t, v)| json!({"t": t, "s": [v], "h": 1})).collect::<Vec<_>>()}),
    }
}

fn rgb(c: Rgba) -> Value {
    let c = c.sanitized();
    json!([n(c.r), n(c.g), n(c.b), 1])
}

fn rule(r: FillRule) -> u8 {
    if r == FillRule::EvenOdd { 2 } else { 1 }
}

/// The sample of one display item, or `None` when it draws nothing Lottie can show.
fn sample(item: &DisplayItem, opacity: f32, masks: Vec<Bez>, report: &mut HashSet<&'static str>) -> Option<Sample> {
    let m = item.transform;
    if !m.is_finite() {
        return None;
    }
    let paths = beziers(&item.path, &m);
    if paths.is_empty() {
        return None;
    }
    let fx = &item.color;
    let (fill, stroke) = if let Some(c) = item.outline {
        // Outline mode: a 1 px hairline in the layer colour, as on the stage.
        (None, Some(Line::round(c, 1.0)))
    } else {
        let fill = match &item.fill {
            Some(Paint::Solid(c)) => Some(Fill::Solid { color: fx.apply(*c), rule: item.fill_rule }),
            Some(Paint::Linear { start, end, stops }) => Some(Fill::Gradient { radial: false, start: m.apply(*start), end: m.apply(*end), stops: stops.iter().map(|s| (s.offset, fx.apply(s.color))).collect(), rule: item.fill_rule }),
            Some(Paint::Radial { center, radius, stops }) => {
                // Lottie's radial gradients are circles: a stretched or skewed one is drawn
                // with its mean radius.
                let (ux, uy) = (m.apply_vector(Vec2::new(1.0, 0.0)), m.apply_vector(Vec2::new(0.0, 1.0)));
                let (lx, ly) = (ux.length(), uy.length());
                if (lx - ly).abs() > 0.01 * lx.max(ly) || ux.dot(uy).abs() > 0.01 * lx * ly {
                    report.insert("stretched radial gradients (drawn as circles)");
                }
                let r = radius * m.mean_scale();
                let c = m.apply(*center);
                Some(Fill::Gradient { radial: true, start: c, end: c + Vec2::new(r, 0.0), stops: stops.iter().map(|s| (s.offset, fx.apply(s.color))).collect(), rule: item.fill_rule })
            }
            Some(Paint::Bitmap { .. }) => {
                report.insert("bitmap fills (not exported yet)");
                None
            }
            None => None,
        };
        let stroke = item.stroke.as_ref().and_then(|s| {
            let w = s.width * m.mean_scale();
            (w.is_finite() && w > 0.0).then(|| Line::of(s, fx.apply(s.color), m.mean_scale()))
        });
        (fill, stroke)
    };
    (fill.is_some() || stroke.is_some()).then_some(Sample { paths, fill, stroke, opacity, masks })
}

/// The movie's frames `from..=to` (movie frames, 0-based) as a Lottie animation named `name`.
/// With `background`, the stage colour is a solid layer at the bottom; otherwise the animation
/// is transparent.
pub fn to_lottie(doc: &Document, name: &str, from: u32, to: u32, background: bool) -> LottieExport {
    let names: HashMap<u64, String> = doc.scenes.iter().map(|s| &s.timeline).chain(doc.library.iter().map(|s| &s.timeline)).flat_map(|t| t.layers.iter()).map(|l| (l.id.0, l.name.clone())).collect();
    let mut notes: HashMap<&'static str, HashSet<usize>> = HashMap::new();
    let mut tracks: Vec<Track> = Vec::new();
    // Open track per key: the index of the track it continues.
    let mut open: HashMap<Key, usize> = HashMap::new();
    let mut edges: HashSet<(usize, usize)> = HashSet::new();
    let to = to.max(from);
    for f in from..=to {
        let t = f - from;
        let full = evaluate(doc, f);
        // Variable-width strokes become filled outlines, as the renderer draws them.
        let dl = crate::stroke::outline_variable_strokes(&full);
        // Mask artwork per clip, in stage space.
        let mut clip_art: Vec<Vec<Bez>> = vec![Vec::new(); dl.clip_parents.len()];
        for item in dl.items.iter().filter(|i| i.fill.is_some()) {
            if let Some(a) = item.masks.and_then(|c| clip_art.get_mut(c as usize)) {
                a.extend(beziers(&item.path, &item.transform).into_iter().map(|mut b| {
                    b.closed = true;
                    b
                }));
            }
        }
        let mut seen: HashMap<(u64, u64, Structure), u32> = HashMap::new();
        let mut order: Vec<usize> = Vec::new();
        let mut still_open: HashSet<Key> = HashSet::new();
        for (index, item) in dl.items.iter().enumerate() {
            if item.guide || item.masks.is_some() {
                continue;
            }
            let mut flags: HashSet<&'static str> = HashSet::new();
            let masks = match item.clip {
                None => Vec::new(),
                Some(c) => {
                    if dl.clip_parents.get(c as usize).copied().flatten().is_some() {
                        flags.insert("nested masks (only the innermost is kept)");
                    }
                    match clip_art.get(c as usize) {
                        // A clip with no mask artwork hides everything in it.
                        Some(a) if !a.is_empty() => a.clone(),
                        _ => continue,
                    }
                }
            };
            let mut opacity = 1.0;
            let mut blend_mode = anicraft_doc::BlendMode::Normal;
            let mut g = item.group;
            let mut depth = 0;
            while let Some(gi) = g {
                let Some(group) = dl.groups.get(gi as usize) else { break };
                opacity *= group.alpha.clamp(0.0, 1.0);
                if !group.filters.is_empty() {
                    flags.insert("filters (not exported; the artwork is drawn without them)");
                }
                if blend_mode.is_normal() && !matches!(group.blend, anicraft_doc::BlendMode::Normal | anicraft_doc::BlendMode::Layer) {
                    // The innermost blending group decides.
                    blend_mode = group.blend;
                }
                g = group.parent.filter(|&p| p < gi);
                depth += 1;
                if depth > 64 {
                    break;
                }
            }
            if opacity < 1.0 {
                flags.insert("symbol fades (faded shape by shape)");
            }
            let blend = lottie_blend(blend_mode);
            if !blend_mode.is_normal() {
                flags.insert(if blend.is_some() { "blend modes (applied shape by shape)" } else { "Subtract, Invert, Alpha and Erase blending (Lottie has no equivalent; drawn normally)" });
            }
            let blend = blend.unwrap_or(0);
            let Some(s) = sample(item, opacity, masks, &mut flags) else {
                // Not exported: counted by element (ids above every track index).
                record(&mut notes, flags, usize::MAX - (item.element.0 as usize).min(usize::MAX / 2));
                continue;
            };
            let shape: Structure = s.paths.iter().map(|b| (b.closed, b.v.len())).collect();
            let occurrence = {
                let c = seen.entry((item.layer.0, item.element.0, shape.clone())).or_insert(0);
                *c += 1;
                *c - 1
            };
            let key = Key {
                layer: item.layer.0,
                element: item.element.0,
                occurrence,
                shape,
                fill: match &s.fill {
                    None => 0,
                    Some(Fill::Solid { rule: r, .. }) => 1 + rule(*r),
                    Some(Fill::Gradient { radial, rule: r, .. }) => 3 + u8::from(*radial) * 2 + rule(*r),
                },
                stops: match &s.fill {
                    Some(Fill::Gradient { stops, .. }) => stops.len(),
                    _ => 0,
                },
                stroke: s.stroke.as_ref().map(|l| (l.cap, l.join, l.dash.len())),
                masks: s.masks.iter().map(|b| (b.closed, b.v.len())).collect(),
                blend,
            };
            let ti = match open.get(&key).copied().filter(|&ti| tracks.get(ti).is_some_and(|tr| tr.last + 1 == t)) {
                Some(ti) => ti,
                None => {
                    tracks.push(Track { key: key.clone(), name: names.get(&item.layer.0).cloned().unwrap_or_default(), first: t, last: t, frames: Vec::new(), first_index: index });
                    open.insert(key.clone(), tracks.len() - 1);
                    tracks.len() - 1
                }
            };
            if let Some(tr) = tracks.get_mut(ti) {
                tr.last = t;
                tr.frames.push((t, s));
            }
            still_open.insert(key);
            record(&mut notes, flags, ti);
            order.push(ti);
        }
        open.retain(|k, _| still_open.contains(k));
        for w in order.windows(2) {
            if let [a, b] = *w
                && a != b
            {
                edges.insert((a, b));
            }
        }
    }

    // One fixed layer order that agrees with every frame's draw order (bottom first): a
    // topological order of "drawn below" edges, ties broken by first appearance.
    let mut indegree = vec![0usize; tracks.len()];
    let mut next: Vec<Vec<usize>> = vec![Vec::new(); tracks.len()];
    for &(a, b) in &edges {
        if let (Some(d), Some(nx)) = (indegree.get_mut(b), next.get_mut(a)) {
            *d += 1;
            nx.push(b);
        }
    }
    let rank = |i: usize| tracks.get(i).map_or((u32::MAX, usize::MAX), |t| (t.first, t.first_index));
    let mut ready: std::collections::BTreeSet<((u32, usize), usize)> = (0..tracks.len()).filter(|&i| indegree.get(i) == Some(&0)).map(|i| (rank(i), i)).collect();
    let mut bottom_up: Vec<usize> = Vec::with_capacity(tracks.len());
    while let Some(first) = ready.pop_first() {
        let i = first.1;
        bottom_up.push(i);
        for &j in next.get(i).map_or(&[][..], Vec::as_slice) {
            if let Some(d) = indegree.get_mut(j) {
                *d -= 1;
                if *d == 0 {
                    ready.insert((rank(j), j));
                }
            }
        }
    }
    if bottom_up.len() < tracks.len() {
        // A cycle: the draw order changes over time. The rest go by first appearance.
        let placed: HashSet<usize> = bottom_up.iter().copied().collect();
        let mut rest: Vec<usize> = (0..tracks.len()).filter(|i| !placed.contains(i)).collect();
        rest.sort_by_key(|&i| rank(i));
        notes.entry("draw-order changes (one fixed order is used)").or_default().extend(rest.iter().copied());
        bottom_up.extend(rest);
    }

    let frames = to - from + 1;
    let mut layers: Vec<Value> = Vec::with_capacity(tracks.len() + 1);
    // Lottie lists layers top first.
    for (ind, &ti) in bottom_up.iter().rev().enumerate() {
        let Some(tr) = tracks.get(ti) else { continue };
        layers.push(layer(tr, ind + 1));
    }
    let shape_layers = layers.len();
    if background {
        let bg = doc.stage.background.sanitized();
        layers.push(json!({"ddd": 0, "ind": shape_layers + 1, "ty": 1, "nm": "Stage", "sr": 1, "ks": static_ks(), "ao": 0, "ip": 0, "op": frames, "st": 0, "bm": 0,
            "sc": bg.to_hex(), "sw": doc.stage.width, "sh": doc.stage.height}));
    }
    let root = json!({
        "v": "5.7.0",
        "fr": n(doc.stage.fps),
        "ip": 0,
        "op": frames,
        "w": doc.stage.width,
        "h": doc.stage.height,
        "nm": name,
        "ddd": 0,
        "assets": [],
        "layers": layers,
        "meta": {"g": format!("AniCraft {}", env!("CARGO_PKG_VERSION"))}
    });
    let report = notes.into_iter().map(|(k, v)| (k.to_string(), v.len())).collect();
    LottieExport { json: root.to_string(), layers: shape_layers, report }
}

/// Lottie's layer blend mode for ours, where it has one.
fn lottie_blend(m: anicraft_doc::BlendMode) -> Option<u8> {
    use anicraft_doc::BlendMode as B;
    Some(match m {
        B::Normal | B::Layer => 0,
        B::Multiply => 1,
        B::Screen => 2,
        B::Overlay => 3,
        B::Darken => 4,
        B::Lighten => 5,
        B::HardLight => 8,
        B::Difference => 10,
        B::Add => 16,
        B::Subtract | B::Invert | B::Alpha | B::Erase => return None,
    })
}

fn record(notes: &mut HashMap<&'static str, HashSet<usize>>, flags: HashSet<&'static str>, track: usize) {
    for f in flags {
        notes.entry(f).or_default().insert(track);
    }
}

fn static_ks() -> Value {
    json!({"o": {"a": 0, "k": 100}, "r": {"a": 0, "k": 0}, "p": {"a": 0, "k": [0, 0, 0]}, "a": {"a": 0, "k": [0, 0, 0]}, "s": {"a": 0, "k": [100, 100, 100]}})
}

/// One track as a shape layer: its outlines, then (drawn on top first, as Lottie lists them)
/// the stroke and the fill, and the group transform carrying its opacity.
fn layer(tr: &Track, ind: usize) -> Value {
    let f = &tr.frames;
    let mut items: Vec<Value> = Vec::new();
    let subpaths = f.first().map_or(0, |(_, s)| s.paths.len());
    for k in 0..subpaths {
        let values: Vec<(u32, Value)> = f.iter().filter_map(|(t, s)| s.paths.get(k).map(|b| (*t, b.to_value()))).collect();
        items.push(json!({"ty": "sh", "nm": format!("Path {}", k + 1), "ks": shape_prop(&values)}));
    }
    if let Some((cap, join, dashes)) = tr.key.stroke {
        let lines: Vec<(u32, &Line)> = f.iter().filter_map(|(t, s)| s.stroke.as_ref().map(|l| (*t, l))).collect();
        let values = |g: &dyn Fn(&Line) -> Value| lines.iter().map(|(t, l)| (*t, g(l))).collect::<Vec<(u32, Value)>>();
        let color = values(&|l| rgb(l.color));
        let opacity = values(&|l| n(l.color.sanitized().a * 100.0));
        let width = values(&|l| n(l.width));
        // The miter limit is a plain number in Lottie: the first frame's.
        // (Round and bevel joins don't use it: Lottie's usual 4, as before styles.)
        let miter = if join == 1 { lines.first().map_or(json!(4), |(_, l)| n(l.miter)) } else { json!(4) };
        let mut st = json!({"ty": "st", "nm": "Stroke", "c": prop(&color), "o": prop(&opacity), "w": prop(&width), "lc": cap, "lj": join, "ml": miter});
        if dashes > 0 {
            // Dash, gap, dash, gap…, then the offset; each animates like any value.
            let mut d: Vec<Value> = (0..dashes)
                .map(|k| {
                    let (code, name) = if k % 2 == 0 { ("d", "dash") } else { ("g", "gap") };
                    json!({"n": code, "nm": name, "v": prop(&values(&|l| n(l.dash.get(k).copied().unwrap_or(0.0))))})
                })
                .collect();
            d.push(json!({"n": "o", "nm": "offset", "v": prop(&values(&|l| n(l.offset)))}));
            st["d"] = Value::Array(d);
        }
        items.push(st);
    }
    match f.first().and_then(|(_, s)| s.fill.as_ref()) {
        Some(Fill::Solid { rule: r, .. }) => {
            let color: Vec<(u32, Value)> = f.iter().filter_map(|(t, s)| match &s.fill {
                Some(Fill::Solid { color, .. }) => Some((*t, rgb(*color))),
                _ => None,
            }).collect();
            let opacity: Vec<(u32, Value)> = f.iter().filter_map(|(t, s)| match &s.fill {
                Some(Fill::Solid { color, .. }) => Some((*t, n(color.sanitized().a * 100.0))),
                _ => None,
            }).collect();
            items.push(json!({"ty": "fl", "nm": "Fill", "c": prop(&color), "o": prop(&opacity), "r": rule(*r)}));
        }
        Some(Fill::Gradient { radial, stops, rule: r, .. }) => {
            let count = stops.len();
            let mut start = Vec::new();
            let mut end = Vec::new();
            let mut colors = Vec::new();
            for (t, s) in f {
                if let Some(Fill::Gradient { start: a, end: b, stops, .. }) = &s.fill {
                    start.push((*t, pt(*a)));
                    end.push((*t, pt(*b)));
                    // Colour stops, then opacity stops: [offset, r, g, b, …, offset, a, …].
                    let mut k: Vec<Value> = Vec::with_capacity(count * 6);
                    for (o, c) in stops {
                        let c = c.sanitized();
                        k.extend([n(o.clamp(0.0, 1.0)), n(c.r), n(c.g), n(c.b)]);
                    }
                    for (o, c) in stops {
                        k.extend([n(o.clamp(0.0, 1.0)), n(c.sanitized().a)]);
                    }
                    colors.push((*t, Value::Array(k)));
                }
            }
            items.push(json!({"ty": "gf", "nm": "Gradient Fill", "t": if *radial { 2 } else { 1 }, "s": prop(&start), "e": prop(&end),
                "g": {"p": count, "k": prop(&colors)}, "o": {"a": 0, "k": 100}, "r": rule(*r), "h": {"a": 0, "k": 0}, "a": {"a": 0, "k": 0}}));
        }
        None => {}
    }
    let opacity: Vec<(u32, Value)> = f.iter().map(|(t, s)| (*t, n(s.opacity * 100.0))).collect();
    items.push(json!({"ty": "tr", "p": {"a": 0, "k": [0, 0]}, "a": {"a": 0, "k": [0, 0]}, "s": {"a": 0, "k": [100, 100]}, "r": {"a": 0, "k": 0}, "o": prop(&opacity), "sk": {"a": 0, "k": 0}, "sa": {"a": 0, "k": 0}}));
    let mut out = json!({
        "ddd": 0, "ind": ind, "ty": 4, "nm": tr.name, "sr": 1, "ks": static_ks(), "ao": 0,
        "ip": tr.first, "op": tr.last + 1, "st": 0, "bm": tr.key.blend,
        "shapes": [{"ty": "gr", "nm": "Artwork", "it": items}]
    });
    let masks = f.first().map_or(0, |(_, s)| s.masks.len());
    if masks > 0 {
        let list: Vec<Value> = (0..masks)
            .map(|k| {
                let values: Vec<(u32, Value)> = f.iter().filter_map(|(t, s)| s.masks.get(k).map(|b| (*t, b.to_value()))).collect();
                json!({"mode": "a", "inv": false, "pt": shape_prop(&values), "o": {"a": 0, "k": 100}, "x": {"a": 0, "k": 0}, "nm": format!("Mask {}", k + 1)})
            })
            .collect();
        out["hasMask"] = json!(true);
        out["masksProperties"] = Value::Array(list);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use anicraft_doc::sample;

    fn parse(e: &LottieExport) -> Value {
        serde_json::from_str(&e.json).unwrap()
    }

    #[test]
    fn beziers_bake_lines_quads_and_closing_points() {
        let mut p = Path::new();
        p.cmds = vec![PathCmd::MoveTo(Vec2::new(0.0, 0.0)), PathCmd::LineTo(Vec2::new(10.0, 0.0)), PathCmd::QuadTo(Vec2::new(10.0, 10.0), Vec2::new(0.0, 10.0)), PathCmd::LineTo(Vec2::new(0.0, 0.0)), PathCmd::Close];
        let b = beziers(&p, &Affine::translate(100.0, 0.0));
        assert_eq!(b.len(), 1);
        let b = &b[0];
        assert!(b.closed);
        assert_eq!(b.v, [Vec2::new(100.0, 0.0), Vec2::new(110.0, 0.0), Vec2::new(100.0, 10.0)], "the repeated closing point is dropped");
        // The quad's control point at 2/3: out-tangent from (10,0), in-tangent into (0,10).
        assert!((b.o[1] - Vec2::new(0.0, 20.0 / 3.0)).length() < 1e-4, "{:?}", b.o[1]);
        assert!((b.i[2] - Vec2::new(20.0 / 3.0, 0.0)).length() < 1e-4, "{:?}", b.i[2]);
        assert_eq!(b.o[2], Vec2::ZERO);
        // Open paths stay open; a lone point is dropped.
        p.cmds = vec![PathCmd::MoveTo(Vec2::ZERO), PathCmd::LineTo(Vec2::new(5.0, 5.0)), PathCmd::MoveTo(Vec2::new(9.0, 9.0))];
        let b = beziers(&p, &Affine::IDENTITY);
        assert_eq!(b.len(), 1);
        assert!(!b[0].closed);
    }

    #[test]
    fn hold_keyframes_only_where_values_change() {
        assert_eq!(prop(&[(0, json!(1)), (1, json!(1))]), json!({"a": 0, "k": 1}));
        assert_eq!(prop(&[(0, json!(1)), (1, json!(1)), (2, json!(5))]), json!({"a": 1, "k": [{"t": 0, "s": [1], "h": 1}, {"t": 2, "s": [5], "h": 1}]}));
        assert_eq!(prop(&[(0, json!([1, 2])), (3, json!([4, 2]))])["k"][1]["s"], json!([4, 2]), "arrays are the value itself");
    }

    #[test]
    fn the_sample_movie_exports_as_layers_that_follow_its_frames() {
        let doc = sample::demo();
        let frames = doc.frame_count();
        let e = to_lottie(&doc, "demo", 0, frames - 1, true);
        let v = parse(&e);
        assert_eq!((v["w"].as_u64(), v["h"].as_u64(), v["op"].as_u64()), (Some(u64::from(doc.stage.width)), Some(u64::from(doc.stage.height)), Some(u64::from(frames))));
        assert_eq!(v["fr"].as_f64(), Some(f64::from(doc.stage.fps)));
        let layers = v["layers"].as_array().unwrap();
        assert_eq!(layers.len(), e.layers + 1, "shape layers and the stage");
        assert_eq!(layers.last().unwrap()["ty"], 1, "the stage colour is at the bottom");
        assert!(e.layers > 0);
        for l in &layers[..e.layers] {
            assert_eq!(l["ty"], 4);
            let (ip, op) = (l["ip"].as_u64().unwrap(), l["op"].as_u64().unwrap());
            assert!(ip < op && op <= u64::from(frames));
            let it = l["shapes"][0]["it"].as_array().unwrap();
            assert_eq!(it.last().unwrap()["ty"], "tr");
            assert!(it.iter().any(|i| i["ty"] == "sh"));
        }
        // The sample animates: something moves, so some outline is keyframed.
        assert!(e.json.contains("\"h\":1"), "hold keyframes for what changes");
        // Exporting a range starts at its first frame.
        let part = parse(&to_lottie(&doc, "part", 10, 19, false));
        assert_eq!(part["op"], 10);
        assert!(part["layers"].as_array().unwrap().iter().all(|l| l["ty"] == 4), "transparent: no stage layer");
    }

    #[test]
    fn every_colour_effect_style_is_baked_into_the_colours() {
        // ADR 0056: Lottie has no colour transform, so the effect is in each fill colour (and
        // tweening effects become colour keyframes); nothing is reported lost.
        use anicraft_doc::{ColorEffect, ColorTransform, Document, Element, ElementId, ElementKind, Shape};
        let mut doc = Document::default();
        let fx = ColorEffect::Advanced(ColorTransform { mul: [-1.0, 0.5, 1.0, 1.0], add: [1.0, 0.0, 0.0, 0.0] });
        let shape = Shape::filled(Path::rect(0.0, 0.0, 20.0, 20.0), Paint::Solid(Rgba::new(0.2, 0.4, 0.6, 1.0)));
        let el = Element { id: ElementId(77), name: String::new(), transform: Affine::IDENTITY, color: fx, kind: ElementKind::Shape(shape) };
        let layer = std::sync::Arc::make_mut(&mut doc.timeline_mut(0).layers[0]);
        layer.keyframe_mut(0).unwrap().elements.push(std::sync::Arc::new(el));
        let e = to_lottie(&doc, "fx", 0, 0, false);
        let v = parse(&e);
        let it = v["layers"][0]["shapes"][0]["it"].as_array().unwrap();
        let fill = it.iter().find(|i| i["ty"] == "fl").expect("a fill");
        let c: Vec<f64> = fill["c"]["k"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
        assert!((c[0] - 0.8).abs() < 1e-3 && (c[1] - 0.2).abs() < 1e-3 && (c[2] - 0.6).abs() < 1e-3, "{c:?}");
        assert!(e.report.is_empty(), "nothing lost: {:?}", e.report);
    }

    #[test]
    fn variable_width_strokes_export_as_filled_outlines() {
        use anicraft_doc::{Document, Element, ElementId, ElementKind, ProfilePreset, Shape, Stroke};
        let mut doc = Document::default();
        let shape = Shape { path: std::sync::Arc::new(Path::line(Vec2::new(10.0, 10.0), Vec2::new(200.0, 10.0))), fill: None, stroke: Some(Stroke::new(Rgba::hex(0x00ff00), 10.0).with_profile(ProfilePreset::TaperEnd.profile())), fill_rule: FillRule::NonZero, merge: false };
        let el = Element { id: ElementId(77), name: String::new(), transform: Affine::IDENTITY, color: Default::default(), kind: ElementKind::Shape(shape) };
        let layer = std::sync::Arc::make_mut(&mut doc.timeline_mut(0).layers[0]);
        layer.keyframe_mut(0).unwrap().elements.push(std::sync::Arc::new(el));
        let e = to_lottie(&doc, "w", 0, 0, false);
        let v = parse(&e);
        let it = v["layers"][0]["shapes"][0]["it"].as_array().unwrap();
        assert!(it.iter().any(|i| i["ty"] == "fl"), "filled outline");
        assert!(!it.iter().any(|i| i["ty"] == "st"), "no Lottie stroke");
        assert!(e.report.is_empty(), "nothing lost: {:?}", e.report);
    }

    #[test]
    fn stroke_caps_joins_and_dashes_carry_across() {
        use anicraft_doc::{Cap, Dash, Document, Element, ElementId, ElementKind, Join, Shape, Stroke, StrokeStyle};
        let export = |st: Stroke| {
            let mut doc = Document::default();
            let shape = Shape { path: std::sync::Arc::new(Path::line(Vec2::new(10.0, 10.0), Vec2::new(200.0, 10.0))), fill: None, stroke: Some(st), fill_rule: FillRule::NonZero, merge: false };
            let el = Element { id: ElementId(77), name: String::new(), transform: Affine::scale(2.0, 2.0), color: Default::default(), kind: ElementKind::Shape(shape) };
            let layer = std::sync::Arc::make_mut(&mut doc.timeline_mut(0).layers[0]);
            layer.keyframe_mut(0).unwrap().elements.push(std::sync::Arc::new(el));
            let v = parse(&to_lottie(&doc, "s", 0, 0, false));
            v["layers"][0]["shapes"][0]["it"].as_array().unwrap().iter().find(|i| i["ty"] == "st").cloned().unwrap()
        };
        // Round and solid as before.
        let st = export(Stroke::new(Rgba::BLACK, 3.0));
        assert_eq!((st["lc"].clone(), st["lj"].clone(), st["ml"].clone(), st.get("d").is_none()), (json!(2), json!(2), json!(4), true));
        // Butt caps, miter joins with their limit, dashes scaled like the width.
        let style = StrokeStyle { cap: Cap::Butt, join: Join::Miter, miter_limit: 6.0, dash: Some(Dash::Dashed { lengths: vec![4.0, 1.5].into(), offset: 1.0 }) };
        let st = export(Stroke::new(Rgba::BLACK, 3.0).with_style(style));
        assert_eq!((st["lc"].clone(), st["lj"].clone(), st["ml"].clone(), st["w"]["k"].clone()), (json!(1), json!(1), json!(6.0), json!(6.0)));
        let d = st["d"].as_array().unwrap();
        let got: Vec<(&str, f64)> = d.iter().map(|e| (e["n"].as_str().unwrap(), e["v"]["k"].as_f64().unwrap())).collect();
        assert_eq!(got, [("d", 8.0), ("g", 3.0), ("o", 2.0)]);
        // Dots: zero-length dashes with round caps, whatever the cap.
        let st = export(Stroke::new(Rgba::BLACK, 3.0).with_style(StrokeStyle { cap: Cap::Square, join: Join::Bevel, dash: Dash::dotted(2.0), ..StrokeStyle::default() }));
        assert_eq!((st["lc"].clone(), st["lj"].clone()), (json!(2), json!(3)));
        assert_eq!((st["d"][0]["v"]["k"].as_f64(), st["d"][1]["v"]["k"].as_f64()), (Some(0.0), Some(10.0)));
    }
}
