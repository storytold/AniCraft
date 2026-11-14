//! Lottie export: the movie as a Lottie (Bodymovin JSON) animation, from the same evaluated
//! frames the renderer draws, written from the public Lottie format description.
//!
//! Every frame is evaluated, and every piece of artwork that persists from frame to frame
//! (same layer, element and path structure) becomes one shape layer. What it shows at each
//! frame (outline, transform, colours, gradient points, stroke width, opacity, masks; colour
//! effects of every style are baked into the colours, ADR 0056) is then written as Lottie
//! keyframes (ADR 0089): a value that stays put is one value; one that travels the way a tween
//! of the document eases it (linear, Classic Ease, Bézier and custom curves, Quad, Cubic, Back
//! and Bounce) is a few keys with Lottie's own Bézier eases, a curve of several pieces split
//! into one key per piece; and what no Lottie ease draws (Elastic, Sine and the other curved
//! presets, a tween inside a tweened symbol, a morph, motion along a curved path) is a hold key
//! at every frame where it changes. Each eased span is checked at every frame against what was
//! sampled, so any Lottie player shows exactly AniCraft's frames.
//!
//! Artwork whose placement changes keeps its outline in its own space under a group transform
//! (position, scale, rotation, skew) that carries the motion, so a turning or growing object is
//! a couple of transform keys rather than an outline per frame; artwork that stays in place,
//! hairlines (outline mode), and strokes under a stretch or skew (which Lottie would draw with
//! a stretched pen) keep their outline in stage space, as before. Strokes keep their caps,
//! joins, miter limit and dashes (dash lengths scaled like the width, dots as zero-length round
//! dashes; ADR 0046); variable-width strokes are filled outlines (the polygons the renderer
//! fills, dashes included).
//!
//! Soft masks (ADR 0070) become track mattes (alpha, or luma for luminance masks): a matte layer
//! with the mask's artwork as drawn, just above each layer it shows through.
//!
//! Warped shapes (ADR 0079) are their bent outlines like any other.
//!
//! Not carried across (counted in the report): bitmap fills (warped or not); nested masks keep only the
//! innermost mask; a symbol fading as one group is faded shape by shape; stretched radial
//! gradients become circles; and draw-order changes that one fixed layer order can't show.

use crate::lottie_keys::{Eases, Key as Kf, Out, Series, fit};
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

/// The report line for artwork keyed frame by frame while it moves: exact at every frame,
/// held between frames (ADR 0089).
pub const STEPPED: &str = "motion keyed frame by frame (Elastic, Sine and similar eases, motion guides, tweens inside tweening symbols: no Lottie ease draws them)";

/// How animated values are keyed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LottieKeys {
    /// Keyframes with Lottie's Bézier eases wherever they draw the frames exactly, artwork that
    /// moves under an animated group transform (ADR 0089).
    #[default]
    Eased,
    /// A hold key at every frame where something changes, every outline in stage space: the
    /// export of earlier builds (kept to compare against, and for players that ease poorly).
    Frames,
}

/// How far a written value may be from the sampled one, in Lottie's units: pixels (positions,
/// outlines, widths), degrees, percent (scale), colour channels (0..1) and opacity (0..100).
/// Values are written to three decimals; these are a little above that and far below what
/// shows (2/1000 of a pixel, 1/1000 of a degree).
const TOL_PX: f64 = 2e-3;
const TOL_DEG: f64 = 1e-3;
const TOL_PCT: f64 = 2e-3;
const TOL_COLOR: f64 = 1.5e-3;
const TOL_OPACITY: f64 = 0.02;

/// Lottie numbers: three decimals are well below a pixel and keep files small.
fn n(v: f32) -> Value {
    num(f64::from(v))
}

fn num(v: f64) -> Value {
    let r = crate::lottie_keys::r3(v);
    if r.fract() == 0.0 && r.abs() < 1e15 { json!(r as i64) } else { json!(r) }
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

    /// The path through `m`.
    fn transformed(&self, m: &Affine) -> Bez {
        Bez { closed: self.closed, v: self.v.iter().map(|p| m.apply(*p)).collect(), i: self.i.iter().map(|t| m.apply_vector(*t)).collect(), o: self.o.iter().map(|t| m.apply_vector(*t)).collect() }
    }

    /// Writes every cubic whose handles sit a third and two thirds along its chord as the
    /// straight line it draws (no tangents). A shape tween blends copies of its outlines whose
    /// lines are such cubics while its keyframes hold plain lines; one way of writing both lets
    /// the tween's outline be keyed (ADR 0089).
    fn straighten(&mut self) {
        let n = self.v.len();
        let segs = if self.closed { n } else { n.saturating_sub(1) };
        for k in 0..segs {
            let j = (k + 1) % n.max(1);
            let (Some(&a), Some(&b), Some(&o), Some(&i)) = (self.v.get(k), self.v.get(j), self.o.get(k), self.i.get(j)) else { continue };
            let third = (b - a) * (1.0 / 3.0);
            let tol = 1e-4 * (third.length() + 1.0);
            if (o - third).length() <= tol && (i + third).length() <= tol {
                if let Some(o) = self.o.get_mut(k) {
                    *o = Vec2::ZERO;
                }
                if let Some(i) = self.i.get_mut(j) {
                    *i = Vec2::ZERO;
                }
            }
        }
    }

    /// Vertices, in tangents, out tangents, each x then y.
    fn flat(&self) -> Vec<f64> {
        self.v.iter().chain(&self.i).chain(&self.o).flat_map(|p| [f64::from(p.x), f64::from(p.y)]).collect()
    }

    /// The path of [`Bez::flat`] values, as Lottie writes it.
    fn flat_value(closed: bool, f: &[f64]) -> Value {
        let pts: Vec<Value> = f.chunks(2).map(|c| json!([num(c.first().copied().unwrap_or(0.0)), num(c.get(1).copied().unwrap_or(0.0))])).collect();
        let k = pts.len() / 3;
        let (v, rest) = pts.split_at(k.min(pts.len()));
        let (i, o) = rest.split_at(k.min(rest.len()));
        json!({"c": closed, "v": v, "i": i, "o": o})
    }
}

/// A path's subpaths as Lottie beziers through `m` (quadratics become cubics).
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
        b.straighten();
    }
    if *m != Affine::IDENTITY {
        for b in &mut out {
            *b = b.transformed(m);
        }
    }
    out
}

/// Where a gradient lies, in the artwork's own space.
#[derive(Clone, Copy, PartialEq)]
enum Ramp {
    /// From one point to another.
    Linear(Vec2, Vec2),
    /// The unit circle through this frame (Lottie draws circles: a stretched one is reported).
    Radial(Affine),
}

/// Fill of one frame, in the artwork's own space.
#[derive(Clone, PartialEq)]
enum Fill {
    Solid { color: Rgba, rule: FillRule },
    Gradient { ramp: Ramp, stops: Vec<(f32, Rgba)>, rule: FillRule },
}

impl Fill {
    /// The fill's kind and rule, which a layer keeps (its values may change).
    fn code(&self) -> u8 {
        match self {
            Fill::Solid { rule: r, .. } => 1 + rule(*r),
            Fill::Gradient { ramp, rule: r, .. } => 3 + u8::from(matches!(ramp, Ramp::Radial(_))) * 2 + rule(*r),
        }
    }
}

/// One piece of artwork in one frame.
#[derive(Clone, PartialEq)]
struct Sample {
    /// The outline in the artwork's own space.
    paths: Vec<Bez>,
    /// Own space → stage.
    xf: Affine,
    fill: Option<Fill>,
    stroke: Option<Line>,
    /// Group opacity (a fading symbol), applied to the shape as a whole.
    opacity: f32,
    /// Mask outlines this artwork shows through (their union), in stage space.
    masks: Vec<Bez>,
    /// A soft mask (ADR 0070) this artwork shows through: a track matte.
    matte: Option<Matte>,
}

/// A soft mask as a track matte: its artwork as drawn, and whether its luminance (not its
/// opacity) reveals.
#[derive(Clone, PartialEq)]
struct Matte {
    luma: bool,
    art: Vec<Sample>,
}

/// What a sample's look must keep for its values to be keyframed: its structure, fill kind,
/// gradient stop count and stroke style.
type Look = (Structure, u8, usize, Option<(u8, u8, usize)>);

fn look(s: &Sample) -> Look {
    let stops = match &s.fill {
        Some(Fill::Gradient { stops, .. }) => stops.len(),
        _ => 0,
    };
    (s.paths.iter().map(|b| (b.closed, b.v.len())).collect(), s.fill.as_ref().map_or(0, Fill::code), stops, s.stroke.as_ref().map(|l| (l.cap, l.join, l.dash.len())))
}

/// A stroke in one frame, in the artwork's own space (a hairline's width is in stage pixels).
#[derive(Clone, PartialEq)]
struct Line {
    color: Rgba,
    width: f32,
    /// Lottie's line cap and join numbers (1 butt / miter, 2 round, 3 square / bevel).
    cap: u8,
    join: u8,
    miter: f32,
    /// On and off lengths, and the offset into them.
    dash: Vec<f32>,
    offset: f32,
    /// Outline mode's line: as wide on the stage whatever the scale.
    hairline: bool,
}

impl Line {
    /// A round, solid line (outline mode's hairline).
    fn hairline(color: Rgba, width: f32) -> Line {
        Line { color, width, cap: 2, join: 2, miter: 4.0, dash: Vec::new(), offset: 0.0, hairline: true }
    }

    fn of(st: &anicraft_doc::Stroke, color: Rgba) -> Line {
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
        let (dash, offset) = style.dash.as_ref().map_or((Vec::new(), 0.0), |d| d.pattern(st.width));
        Line { color, width: st.width, cap, join, miter: style.miter_limit, dash, offset, hairline: false }
    }

    /// The line drawn through a transform of mean scale `k` (the renderer widens strokes by
    /// the mean scale, dashes too).
    fn scaled(&self, k: f32) -> Line {
        if self.hairline {
            return self.clone();
        }
        Line { width: self.width * k, dash: self.dash.iter().map(|x| x * k).collect(), offset: self.offset * k, ..self.clone() }
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
    /// A track matte's kind (luma or alpha) and the look of each piece of its artwork.
    matte: Option<(bool, Vec<Look>)>,
}

impl Key {
    /// The same in all but the element (and its count): artwork that can continue a track.
    fn same_look(&self, o: &Key) -> bool {
        Key { element: o.element, occurrence: o.occurrence, ..self.clone() } == *o
    }
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

fn rgb(c: Rgba) -> Vec<f64> {
    let c = c.sanitized();
    vec![f64::from(c.r), f64::from(c.g), f64::from(c.b), 1.0]
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
    let paths = beziers(&item.path, &Affine::IDENTITY);
    if paths.is_empty() {
        return None;
    }
    let fx = &item.color;
    let (fill, stroke) = if let Some(c) = item.outline {
        // Outline mode: a 1 px hairline in the layer colour, as on the stage.
        (None, Some(Line::hairline(c, 1.0)))
    } else {
        let fill = match &item.fill {
            Some(Paint::Solid(c)) => Some(Fill::Solid { color: fx.apply(*c), rule: item.fill_rule }),
            Some(p @ (Paint::Linear { .. } | Paint::Radial { .. })) => {
                // What Lottie's gradients can't show is drawn the nearest way, and said.
                if let Paint::Linear { spread, linear_rgb, .. } | Paint::Radial { spread, linear_rgb, .. } = p {
                    if !spread.is_pad() {
                        report.insert("reflected or repeated gradients (padded)");
                    }
                    if *linear_rgb {
                        report.insert("linear-RGB gradients (blended in sRGB)");
                    }
                }
                let stops = p.stops().unwrap_or(&[]).iter().map(|s| (s.offset, fx.apply(s.color))).collect();
                let ramp = match p {
                    Paint::Linear { start, end, .. } => Ramp::Linear(*start, *end),
                    _ => {
                        if matches!(p, Paint::Radial { focal, .. } if *focal != 0.0) {
                            report.insert("radial gradient focal points (drawn centred)");
                        }
                        Ramp::Radial(p.frame(None).unwrap_or(Affine::IDENTITY))
                    }
                };
                Some(Fill::Gradient { ramp, stops, rule: item.fill_rule })
            }
            Some(Paint::Bitmap { .. }) => {
                report.insert("bitmap fills (not exported yet)");
                None
            }
            None => None,
        };
        let stroke = item.stroke.as_ref().and_then(|s| {
            let w = s.width * m.mean_scale();
            (w.is_finite() && w > 0.0).then(|| Line::of(s, fx.apply(s.color)))
        });
        (fill, stroke)
    };
    (fill.is_some() || stroke.is_some()).then_some(Sample { paths, xf: m, fill, stroke, opacity, masks, matte: None })
}

/// The movie's frames `from..=to` (movie frames, 0-based) as a Lottie animation named `name`,
/// keyed with eases (ADR 0089). With `background`, the stage colour is a solid layer at the
/// bottom; otherwise the animation is transparent.
pub fn to_lottie(doc: &Document, name: &str, from: u32, to: u32, background: bool) -> LottieExport {
    to_lottie_keyed(doc, name, from, to, background, LottieKeys::Eased)
}

/// [`to_lottie`], keyed the way `keys` says.
pub fn to_lottie_keyed(doc: &Document, name: &str, from: u32, to: u32, background: bool, keys: LottieKeys) -> LottieExport {
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
        // Soft masks' artwork as drawn, per clip (ADR 0070).
        let mut soft_art: Vec<Vec<Sample>> = vec![Vec::new(); dl.clip_parents.len()];
        for item in dl.items.iter().filter(|i| i.masks.is_some_and(|c| dl.clip_mode(c).is_soft()) && !i.guide) {
            let mut ignored = HashSet::new();
            if let (Some(a), Some(s)) = (item.masks.and_then(|c| soft_art.get_mut(c as usize)), sample(item, 1.0, Vec::new(), &mut ignored)) {
                a.push(s);
            }
        }
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
        // This frame's pieces, in draw order: (item index, layer, key, sample, report flags).
        let mut pending: Vec<(usize, u64, Key, Sample, HashSet<&'static str>)> = Vec::new();
        for (index, item) in dl.items.iter().enumerate() {
            if item.guide || item.masks.is_some() {
                continue;
            }
            let mut flags: HashSet<&'static str> = HashSet::new();
            let mut matte = None;
            let masks = match item.clip {
                None => Vec::new(),
                Some(c) => {
                    if dl.clip_parents.get(c as usize).copied().flatten().is_some() {
                        flags.insert("nested masks (only the innermost is kept)");
                    }
                    let art = if dl.clip_mode(c).is_soft() { soft_art.get(c as usize).map(Vec::len) } else { clip_art.get(c as usize).map(Vec::len) };
                    // A clip with no mask artwork hides everything in it.
                    if art.unwrap_or(0) == 0 {
                        continue;
                    }
                    if dl.clip_mode(c).is_soft() {
                        // A soft mask is a track matte: alpha, or luma.
                        matte = soft_art.get(c as usize).map(|a| Matte { luma: dl.clip_mode(c) == anicraft_doc::MaskMode::Luminance, art: a.clone() });
                        Vec::new()
                    } else {
                        clip_art.get(c as usize).cloned().unwrap_or_default()
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
            let Some(mut s) = sample(item, opacity, masks, &mut flags) else {
                // Not exported: counted by element (ids above every track index).
                record(&mut notes, flags, usize::MAX - (item.element.0 as usize).min(usize::MAX / 2));
                continue;
            };
            s.matte = matte;
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
                fill: s.fill.as_ref().map_or(0, Fill::code),
                stops: match &s.fill {
                    Some(Fill::Gradient { stops, .. }) => stops.len(),
                    _ => 0,
                },
                stroke: s.stroke.as_ref().map(|l| (l.cap, l.join, l.dash.len())),
                masks: s.masks.iter().map(|b| (b.closed, b.v.len())).collect(),
                blend,
                matte: s.matte.as_ref().map(|m| (m.luma, m.art.iter().map(look).collect())),
            };
            pending.push((index, item.layer.0, key, s, flags));
        }
        // Each piece continues the track it belongs to: first the same element, then (a
        // classic tween's next keyframe holds its own copy of the object, as XFL files do)
        // artwork that looks the same on the same layer, whose track the frame didn't continue.
        let mut taken: HashSet<usize> = HashSet::new();
        let mut assigned: Vec<Option<usize>> = pending
            .iter()
            .map(|(_, _, key, _, _)| {
                let ti = open.get(key).copied().filter(|&ti| tracks.get(ti).is_some_and(|tr| tr.last + 1 == t) && !taken.contains(&ti))?;
                taken.insert(ti);
                Some(ti)
            })
            .collect();
        for (slot, (_, _, key, _, _)) in assigned.iter_mut().zip(&pending) {
            if slot.is_some() {
                continue;
            }
            let found = open.iter().filter(|(k, ti)| k.same_look(key) && !taken.contains(ti) && tracks.get(**ti).is_some_and(|tr| tr.last + 1 == t)).map(|(_, ti)| *ti).min();
            if let Some(ti) = found {
                taken.insert(ti);
                *slot = Some(ti);
            }
        }
        for ((index, layer_id, key, s, flags), slot) in pending.into_iter().zip(assigned) {
            let ti = match slot {
                Some(ti) => ti,
                None => {
                    tracks.push(Track { key: key.clone(), name: names.get(&layer_id).cloned().unwrap_or_default(), first: t, last: t, frames: Vec::new(), first_index: index });
                    tracks.len() - 1
                }
            };
            open.insert(key.clone(), ti);
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

    let eases = match keys {
        LottieKeys::Eased => Eases::of(doc),
        LottieKeys::Frames => Eases::none(),
    };
    let frames = to - from + 1;
    let mut layers: Vec<Value> = Vec::with_capacity(tracks.len() + 1);
    // Lottie lists layers top first; a track matte comes just above the layer it mattes.
    for &ti in bottom_up.iter().rev() {
        let Some(tr) = tracks.get(ti) else { continue };
        let mut w = Writer { eases: &eases, stepped: false, flags: HashSet::new() };
        if let Some(m) = matte_layer(tr, layers.len() + 1, &mut w) {
            layers.push(m);
            let mut l = layer(tr, layers.len() + 1, &mut w);
            l["tt"] = json!(if tr.key.matte.as_ref().is_some_and(|(luma, _)| *luma) { 3 } else { 1 });
            layers.push(l);
        } else {
            layers.push(layer(tr, layers.len() + 1, &mut w));
        }
        if w.stepped {
            w.flags.insert(STEPPED);
        }
        record(&mut notes, w.flags, ti);
    }
    let shape_layers = layers.iter().filter(|l| l.get("td").is_none()).count();
    let mattes = layers.len() - shape_layers;
    if background {
        let bg = doc.stage.background.sanitized();
        layers.push(json!({"ddd": 0, "ind": shape_layers + mattes + 1, "ty": 1, "nm": "Stage", "sr": 1, "ks": static_ks(), "ao": 0, "ip": 0, "op": frames, "st": 0, "bm": 0,
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

/// How a property's values are written.
#[derive(Clone, Copy)]
enum Enc {
    /// One number (`k: v`, keys `s: [v]`).
    Scalar,
    /// A list of numbers.
    Vector,
    /// A path of this many vertices, open or closed (keys `s: [path]`).
    Shape(bool),
}

impl Enc {
    fn value(self, v: &[f64]) -> Value {
        match self {
            Enc::Scalar => num(v.first().copied().unwrap_or(0.0)),
            Enc::Vector => Value::Array(v.iter().map(|x| num(*x)).collect()),
            Enc::Shape(closed) => Bez::flat_value(closed, v),
        }
    }

    fn key(self, v: &[f64]) -> Value {
        match self {
            Enc::Scalar | Enc::Vector => Value::Array(v.iter().map(|x| num(*x)).collect()),
            Enc::Shape(_) => json!([self.value(v)]),
        }
    }
}

/// Writes a track's properties as keyframes.
struct Writer<'a> {
    eases: &'a Eases,
    /// Some property had to be keyed frame by frame while it moved.
    stepped: bool,
    /// What this track couldn't carry across.
    flags: HashSet<&'static str>,
}

impl Writer<'_> {
    /// The keys of one property's values (one per frame from `first`).
    fn keys(&mut self, first: u32, values: &[Vec<f64>], tol: f64) -> Vec<Kf> {
        let (keys, stepped) = fit(&Series { first, values, tol }, self.eases);
        self.stepped |= stepped;
        keys
    }

    /// A property: static when it never changes, else its keyframes.
    fn prop(&mut self, first: u32, values: &[Vec<f64>], tol: f64, enc: Enc) -> Value {
        let keys = self.keys(first, values, tol);
        write_keys(&keys, enc)
    }
}

fn write_keys(keys: &[Kf], enc: Enc) -> Value {
    match keys {
        [] => json!({"a": 0, "k": 0}),
        [k] => json!({"a": 0, "k": enc.value(&k.s)}),
        _ => json!({"a": 1, "k": keys.iter().map(|k| {
            let mut o = json!({"t": num(k.t), "s": enc.key(&k.s)});
            match k.out {
                Out::Hold => o["h"] = json!(1),
                Out::Ease(out, inn) => {
                    o["o"] = json!({"x": out.0, "y": out.1});
                    o["i"] = json!({"x": inn.0, "y": inn.1});
                }
                Out::End => {}
            }
            o
        }).collect::<Vec<_>>()}),
    }
}

/// A Lottie group transform, one frame: position, rotation (degrees), skew (degrees, about the
/// x axis), scale (percent).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Tr {
    p: [f64; 2],
    r: f64,
    sk: f64,
    s: [f64; 2],
}

/// The group transforms that place a track's outline at each frame, when it moves (its own
/// space then carries the outline); `None` when it should stay in stage space: it doesn't move,
/// it is a hairline, a stroke would be stretched or skewed (Lottie stretches the pen, the
/// renderer doesn't), or a frame's matrix can't be written as Lottie's.
fn transforms(frames: &[(u32, Sample)]) -> Option<Vec<Tr>> {
    let first = frames.first()?.1.xf;
    if frames.iter().all(|(_, s)| s.xf == first) {
        return None;
    }
    // Lottie's transform is T(p) · R(r) · Skew(sk) · S(s): ours, QR-style, with the shear's
    // angle (Lottie skews x by −tan(sk) per unit of y).
    let mut raw: Vec<Option<(f64, f64)>> = Vec::with_capacity(frames.len());
    for (_, s) in frames {
        if !s.xf.is_finite() || s.stroke.as_ref().is_some_and(|l| l.hairline) {
            return None;
        }
        let [a, b, c, d, _, _] = s.xf.m.map(f64::from);
        let sx = a.hypot(b);
        let det = a * d - b * c;
        if sx > 1e-6 && det.abs() > 1e-9 {
            let shear = (a * c + b * d) / det;
            let sy = det / sx;
            if s.stroke.is_some() && ((sx - sy.abs()).abs() > 1e-4 * sx || shear.abs() > 1e-4) {
                return None;
            }
            raw.push(Some((b.atan2(a), shear)));
        } else {
            raw.push(None);
        }
    }
    // A frame squashed flat has no rotation or skew of its own: its neighbour's.
    let near = |i: usize| -> (f64, f64) {
        (0..raw.len()).filter_map(|j| raw.get(j).copied().flatten().map(|v| (i.abs_diff(j), v))).min_by_key(|(d, _)| *d).map_or((0.0, 0.0), |(_, v)| v)
    };
    let mut out: Vec<Tr> = Vec::with_capacity(frames.len());
    let mut prev_r: Option<f64> = None;
    for (i, (_, s)) in frames.iter().enumerate() {
        let (rot, shear) = raw.get(i).copied().flatten().unwrap_or_else(|| near(i));
        let [a, b, c, d, e, f] = s.xf.m.map(f64::from);
        let (cr, sr) = (rot.cos(), rot.sin());
        // R(−r) · M = Shear · S: its diagonal holds the scales.
        let sx = cr * a + sr * b;
        let sy = -sr * c + cr * d;
        // Rotation keeps turning the same way: the nearest turn to the last frame's.
        let mut r = rot.to_degrees();
        if let Some(p) = prev_r {
            r += 360.0 * ((p - r) / 360.0).round();
        }
        prev_r = Some(r);
        let tr = Tr { p: [e, f], r, sk: -shear.atan().to_degrees(), s: [sx * 100.0, sy * 100.0] };
        // The written transform must give the frame's matrix back.
        let m = compose(&tr);
        let scale = a.abs().max(b.abs()).max(c.abs()).max(d.abs()).max(1.0);
        if m.iter().zip([a, b, c, d, e, f]).enumerate().any(|(k, (x, y))| (x - y).abs() > if k < 4 { 1e-5 * scale } else { 1e-3 }) {
            return None;
        }
        out.push(tr);
    }
    Some(out)
}

/// The matrix `[a b c d e f]` of a Lottie group transform (anchor at the origin).
fn compose(t: &Tr) -> [f64; 6] {
    let (s, c) = t.r.to_radians().sin_cos();
    let k = -t.sk.to_radians().tan();
    let (sx, sy) = (t.s[0] / 100.0, t.s[1] / 100.0);
    // R · [1 k; 0 1] · diag(sx, sy)
    [c * sx, s * sx, (c * k - s) * sy, (s * k + c) * sy, t.p[0], t.p[1]]
}

/// The track matte a track shows through (a soft mask, ADR 0070), as a matte layer (`td`): each
/// piece of the mask's artwork as a group, top first, with its paint and opacity.
fn matte_layer(tr: &Track, ind: usize, w: &mut Writer<'_>) -> Option<Value> {
    let (_, looks) = tr.key.matte.as_ref()?;
    let mut groups: Vec<Value> = Vec::with_capacity(looks.len());
    for (k, (shape, _, _, stroke)) in looks.iter().enumerate().rev() {
        let frames: Vec<(u32, Sample)> = tr.frames.iter().filter_map(|(t, s)| Some((*t, s.matte.as_ref()?.art.get(k)?.clone()))).collect();
        let key = Key { shape: shape.clone(), stroke: *stroke, masks: Vec::new(), blend: 0, matte: None, ..tr.key.clone() };
        let piece = Track { key, name: String::new(), first: tr.first, last: tr.last, frames, first_index: 0 };
        if let Some(g) = layer(&piece, 0, w).get_mut("shapes").and_then(|v| v.get_mut(0)).map(Value::take) {
            groups.push(g);
        }
    }
    Some(json!({
        "ddd": 0, "ind": ind, "ty": 4, "nm": format!("{} matte", tr.name), "sr": 1, "ks": static_ks(), "ao": 0,
        "ip": tr.first, "op": tr.last + 1, "st": 0, "bm": 0, "td": 1, "shapes": groups
    }))
}

/// One track as a shape layer: its outlines, then (drawn on top first, as Lottie lists them)
/// the stroke and the fill, and the group transform carrying its placement and opacity.
fn layer(tr: &Track, ind: usize, w: &mut Writer<'_>) -> Value {
    // Frame by frame there is one sample per frame; fill any gap with the frame before.
    let mut f: Vec<(u32, Sample)> = Vec::with_capacity(tr.frames.len());
    for (t, s) in &tr.frames {
        while let Some((lt, ls)) = f.last().cloned().filter(|(lt, _)| lt + 1 < *t) {
            f.push((lt + 1, ls));
        }
        f.push((*t, s.clone()));
    }
    let first = f.first().map_or(tr.first, |(t, _)| *t);
    let moving = if w.eases.is_empty() { None } else { transforms(&f) };
    // Stage-space artwork: everything through its frame's transform, as the renderer draws it.
    if moving.is_none() {
        for (_, s) in &mut f {
            bake(s);
        }
    }
    let mut items: Vec<Value> = Vec::new();
    let subpaths = f.first().map_or(0, |(_, s)| s.paths.len());
    for k in 0..subpaths {
        let values: Vec<Vec<f64>> = f.iter().map(|(_, s)| s.paths.get(k).map(Bez::flat).unwrap_or_default()).collect();
        let closed = f.first().and_then(|(_, s)| s.paths.get(k)).is_some_and(|b| b.closed);
        items.push(json!({"ty": "sh", "nm": format!("Path {}", k + 1), "ks": w.prop(first, &values, TOL_PX, Enc::Shape(closed))}));
    }
    if let Some((cap, join, dashes)) = tr.key.stroke {
        let lines: Vec<&Line> = f.iter().filter_map(|(_, s)| s.stroke.as_ref()).collect();
        let values = |g: &dyn Fn(&Line) -> Vec<f64>| lines.iter().map(|l| g(l)).collect::<Vec<Vec<f64>>>();
        let color = w.prop(first, &values(&|l| rgb(l.color)), TOL_COLOR, Enc::Vector);
        let opacity = w.prop(first, &values(&|l| vec![f64::from(l.color.sanitized().a) * 100.0]), TOL_OPACITY, Enc::Scalar);
        let width = w.prop(first, &values(&|l| vec![f64::from(l.width)]), TOL_PX, Enc::Scalar);
        // The miter limit is a plain number in Lottie: the first frame's.
        // (Round and bevel joins don't use it: Lottie's usual 4, as before styles.)
        let miter = if join == 1 { lines.first().map_or(json!(4), |l| n(l.miter)) } else { json!(4) };
        let mut st = json!({"ty": "st", "nm": "Stroke", "c": color, "o": opacity, "w": width, "lc": cap, "lj": join, "ml": miter});
        if dashes > 0 {
            // Dash, gap, dash, gap…, then the offset; each animates like any value.
            let mut d: Vec<Value> = (0..dashes)
                .map(|k| {
                    let (code, name) = if k % 2 == 0 { ("d", "dash") } else { ("g", "gap") };
                    json!({"n": code, "nm": name, "v": w.prop(first, &values(&|l| vec![f64::from(l.dash.get(k).copied().unwrap_or(0.0))]), TOL_PX, Enc::Scalar)})
                })
                .collect();
            d.push(json!({"n": "o", "nm": "offset", "v": w.prop(first, &values(&|l| vec![f64::from(l.offset)]), TOL_PX, Enc::Scalar)}));
            st["d"] = Value::Array(d);
        }
        items.push(st);
    }
    match f.first().and_then(|(_, s)| s.fill.clone()) {
        Some(Fill::Solid { rule: r, .. }) => {
            let solid = |g: &dyn Fn(Rgba) -> Vec<f64>| -> Vec<Vec<f64>> {
                f.iter().filter_map(|(_, s)| match &s.fill {
                    Some(Fill::Solid { color, .. }) => Some(g(*color)),
                    _ => None,
                }).collect()
            };
            let color = w.prop(first, &solid(&rgb), TOL_COLOR, Enc::Vector);
            let opacity = w.prop(first, &solid(&|c| vec![f64::from(c.sanitized().a) * 100.0]), TOL_OPACITY, Enc::Scalar);
            items.push(json!({"ty": "fl", "nm": "Fill", "c": color, "o": opacity, "r": rule(r)}));
        }
        Some(Fill::Gradient { ramp, stops, rule: r }) => {
            let radial = matches!(ramp, Ramp::Radial(_));
            let count = stops.len();
            let mut start = Vec::new();
            let mut end = Vec::new();
            let mut colors = Vec::new();
            for (_, s) in &f {
                if let Some(Fill::Gradient { ramp, stops, .. }) = &s.fill {
                    let (a, b) = match ramp {
                        Ramp::Linear(a, b) => (*a, *b),
                        Ramp::Radial(frame) => {
                            // Lottie's radial gradients are circles: a stretched or skewed one is
                            // drawn with its mean radius.
                            let (ux, uy) = (frame.apply_vector(Vec2::new(1.0, 0.0)), frame.apply_vector(Vec2::new(0.0, 1.0)));
                            let (lx, ly) = (ux.length(), uy.length());
                            if (lx - ly).abs() > 0.01 * lx.max(ly) || ux.dot(uy).abs() > 0.01 * lx * ly {
                                w.flags.insert("stretched radial gradients (drawn as circles)");
                            }
                            let c = frame.apply(Vec2::ZERO);
                            (c, c + Vec2::new(frame.mean_scale(), 0.0))
                        }
                    };
                    start.push(vec![f64::from(a.x), f64::from(a.y)]);
                    end.push(vec![f64::from(b.x), f64::from(b.y)]);
                    // Colour stops, then opacity stops: [offset, r, g, b, …, offset, a, …].
                    let mut k: Vec<f64> = Vec::with_capacity(count * 6);
                    for (o, c) in stops {
                        let c = c.sanitized();
                        k.extend([f64::from(o.clamp(0.0, 1.0)), f64::from(c.r), f64::from(c.g), f64::from(c.b)]);
                    }
                    for (o, c) in stops {
                        k.extend([f64::from(o.clamp(0.0, 1.0)), f64::from(c.sanitized().a)]);
                    }
                    colors.push(k);
                }
            }
            let (s, e, g) = (w.prop(first, &start, TOL_PX, Enc::Vector), w.prop(first, &end, TOL_PX, Enc::Vector), w.prop(first, &colors, TOL_COLOR, Enc::Vector));
            items.push(json!({"ty": "gf", "nm": "Gradient Fill", "t": if radial { 2 } else { 1 }, "s": s, "e": e,
                "g": {"p": count, "k": g}, "o": {"a": 0, "k": 100}, "r": rule(r), "h": {"a": 0, "k": 0}, "a": {"a": 0, "k": 0}}));
        }
        None => {}
    }
    let opacity: Vec<Vec<f64>> = f.iter().map(|(_, s)| vec![f64::from(s.opacity) * 100.0]).collect();
    let o = w.prop(first, &opacity, TOL_OPACITY, Enc::Scalar);
    let mut group_tr = json!({"ty": "tr", "p": {"a": 0, "k": [0, 0]}, "a": {"a": 0, "k": [0, 0]}, "s": {"a": 0, "k": [100, 100]}, "r": {"a": 0, "k": 0}, "o": o, "sk": {"a": 0, "k": 0}, "sa": {"a": 0, "k": 0}});
    if let Some(trs) = &moving {
        let col = |g: &dyn Fn(&Tr) -> Vec<f64>| trs.iter().map(g).collect::<Vec<Vec<f64>>>();
        // Position as one point, or x and y apart when their timings differ (a motion tween
        // keys them separately) and that takes fewer keys.
        let fit_p = |g: &dyn Fn(&Tr) -> Vec<f64>| fit(&Series { first, values: &col(g), tol: TOL_PX }, w.eases);
        let (p, joint_steps) = fit_p(&|t| t.p.to_vec());
        let ((px, x_steps), (py, y_steps)) = (fit_p(&|t| vec![t.p[0]]), fit_p(&|t| vec![t.p[1]]));
        group_tr["p"] = if px.len() + py.len() < p.len() {
            w.stepped |= x_steps || y_steps;
            json!({"s": true, "x": write_keys(&px, Enc::Scalar), "y": write_keys(&py, Enc::Scalar)})
        } else {
            w.stepped |= joint_steps;
            write_keys(&p, Enc::Vector)
        };
        group_tr["r"] = w.prop(first, &col(&|t| vec![t.r]), TOL_DEG, Enc::Scalar);
        group_tr["sk"] = w.prop(first, &col(&|t| vec![t.sk]), TOL_DEG, Enc::Scalar);
        group_tr["s"] = w.prop(first, &col(&|t| t.s.to_vec()), TOL_PCT, Enc::Vector);
    }
    items.push(group_tr);
    let mut out = json!({
        "ddd": 0, "ind": ind, "ty": 4, "nm": tr.name, "sr": 1, "ks": static_ks(), "ao": 0,
        "ip": tr.first, "op": tr.last + 1, "st": 0, "bm": tr.key.blend,
        "shapes": [{"ty": "gr", "nm": "Artwork", "it": items}]
    });
    let masks = f.first().map_or(0, |(_, s)| s.masks.len());
    if masks > 0 {
        let list: Vec<Value> = (0..masks)
            .map(|k| {
                let values: Vec<Vec<f64>> = f.iter().map(|(_, s)| s.masks.get(k).map(Bez::flat).unwrap_or_default()).collect();
                json!({"mode": "a", "inv": false, "pt": w.prop(first, &values, TOL_PX, Enc::Shape(true)), "o": {"a": 0, "k": 100}, "x": {"a": 0, "k": 0}, "nm": format!("Mask {}", k + 1)})
            })
            .collect();
        out["hasMask"] = json!(true);
        out["masksProperties"] = Value::Array(list);
    }
    out
}

/// A sample in stage space: the outline, gradient and stroke through its transform (strokes
/// widened by its mean scale, as the renderer draws them).
fn bake(s: &mut Sample) {
    let m = s.xf;
    if m == Affine::IDENTITY {
        return;
    }
    for b in &mut s.paths {
        *b = b.transformed(&m);
    }
    if let Some(Fill::Gradient { ramp, .. }) = &mut s.fill {
        *ramp = match *ramp {
            Ramp::Linear(a, b) => Ramp::Linear(m.apply(a), m.apply(b)),
            Ramp::Radial(f) => Ramp::Radial(m.mul(&f)),
        };
    }
    if let Some(l) = &mut s.stroke {
        *l = l.scaled(m.mean_scale());
    }
    s.xf = Affine::IDENTITY;
}
#[cfg(test)]
mod tests {
    use super::*;
    use anicraft_doc::sample;

    fn parse(e: &LottieExport) -> Value {
        serde_json::from_str(&e.json).unwrap()
    }

    /// A mask layer (in `mode`) whose artwork is a white-to-clear gradient, over a masked red square.
    pub(crate) fn soft_mask_doc(mode: anicraft_doc::MaskMode) -> Document {
        use anicraft_doc::{ColorEffect, Element, ElementId, ElementKind, GradientStop, Layer, LayerId, LayerKind, Scene, Shape};
        use std::sync::Arc;
        let mut d = Document { scenes: vec![Arc::new(Scene::new("Scene 1"))], ..Document::default() };
        let stops = vec![GradientStop { offset: 0.0, color: Rgba::WHITE }, GradientStop { offset: 1.0, color: Rgba::new(1.0, 1.0, 1.0, 0.0) }];
        let art = Shape::filled(Path::rect(0.0, 0.0, 200.0, 200.0), Paint::linear(Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0), stops));
        let square = Shape::filled(Path::rect(20.0, 20.0, 160.0, 160.0), Paint::Solid(Rgba::new(1.0, 0.0, 0.0, 1.0)));
        for (id, kind, parent, shape) in [(1, LayerKind::Mask, None, art), (2, LayerKind::Normal, Some(LayerId(1)), square)] {
            let mut l = Layer::new(LayerId(id), format!("L{id}"));
            l.kind = kind;
            l.parent = parent;
            l.mask_mode = if kind == LayerKind::Mask { mode } else { Default::default() };
            Arc::make_mut(&mut l.keyframes[0]).elements.push(Arc::new(Element { id: ElementId(100 + id), name: String::new(), transform: Affine::IDENTITY, color: ColorEffect::default(), kind: ElementKind::Shape(shape) }));
            d.timeline_mut(0).layers.push(Arc::new(l));
        }
        d
    }

    #[test]
    fn soft_masks_become_track_mattes() {
        use anicraft_doc::MaskMode;
        for (mode, tt) in [(MaskMode::Alpha, 1), (MaskMode::Luminance, 3)] {
            let e = to_lottie(&soft_mask_doc(mode), "soft", 0, 0, false);
            let v = parse(&e);
            let layers = v["layers"].as_array().unwrap();
            assert_eq!((e.layers, layers.len()), (1, 2), "{mode:?}: one shape layer and its matte");
            // The matte comes first (above), marked td; the layer under it uses it.
            assert_eq!(layers[0]["td"], 1);
            assert_eq!(layers[1]["tt"], tt);
            assert!(layers[1].get("masksProperties").is_none(), "no layer mask for a soft mask");
            // The matte holds the mask's artwork as drawn: its gradient fill.
            let it = &layers[0]["shapes"][0]["it"];
            assert!(it.as_array().unwrap().iter().any(|x| x["ty"] == "gf"), "{it}");
            assert_eq!((layers[0]["ind"].as_u64(), layers[1]["ind"].as_u64()), (Some(1), Some(2)));
        }
        // A coverage mask stays a layer mask.
        let v = parse(&to_lottie(&soft_mask_doc(MaskMode::Coverage), "hard", 0, 0, false));
        assert_eq!(v["layers"].as_array().unwrap().len(), 1);
        assert_eq!(v["layers"][0]["hasMask"], true);
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
        // The sample animates: something moves, so some value is keyframed.
        assert!(e.json.contains("\"a\":1"), "keyframes for what changes");
        // Exporting a range starts at its first frame.
        let part = parse(&to_lottie(&doc, "part", 10, 19, false));
        assert_eq!(part["op"], 10);
        assert!(part["layers"].as_array().unwrap().iter().all(|l| l["ty"] == 4), "transparent: no stage layer");
    }

    /// A property's keyframes as written (`a: 0` is one key).
    fn keys_of(p: &Value) -> Vec<Kf> {
        let vec = |v: &Value| -> Vec<f64> { if let Some(a) = v.as_array() { a.iter().map(|x| x.as_f64().unwrap()).collect() } else { vec![v.as_f64().unwrap()] } };
        if p["a"] == 0 {
            return vec![Kf { t: 0.0, s: vec(&p["k"]), out: Out::End }];
        }
        p["k"].as_array().unwrap().iter().map(|k| {
            let out = if k["h"] == 1 { Out::Hold } else if k.get("o").is_some() { Out::Ease((k["o"]["x"].as_f64().unwrap(), k["o"]["y"].as_f64().unwrap()), (k["i"]["x"].as_f64().unwrap(), k["i"]["y"].as_f64().unwrap())) } else { Out::End };
            Kf { t: k["t"].as_f64().unwrap(), s: vec(&k["s"]), out }
        }).collect()
    }

    /// A square on layer 1 tweening from `a` to `b` over ten frames with `ease`.
    fn tweened(a: Affine, b: Affine, ease: anicraft_doc::Easing, stroke: bool) -> Document {
        use anicraft_doc::{ColorEffect, Element, ElementId, ElementKind, Keyframe, Shape, Stroke, Tween, TweenKind};
        use std::sync::Arc;
        let mut shape = Shape::filled(Path::rect(-5.0, -5.0, 10.0, 10.0), Paint::Solid(Rgba::BLACK));
        if stroke {
            shape.stroke = Some(Stroke::new(Rgba::WHITE, 2.0));
        }
        let sq = |m: Affine| Arc::new(Element { id: ElementId(70), name: String::new(), transform: m, color: ColorEffect::default(), kind: ElementKind::Shape(shape.clone()) });
        let mut doc = Document::default();
        let mut k0 = Keyframe::new(0, 10);
        k0.elements = vec![sq(a)];
        k0.tween = Some(Tween::new(ease, TweenKind::Classic));
        let mut k1 = Keyframe::new(10, 5);
        k1.elements = vec![sq(b)];
        Arc::make_mut(&mut doc.timeline_mut(0).layers[0]).keyframes = vec![Arc::new(k0), Arc::new(k1)];
        doc
    }

    /// The artwork group's items of layer `l`.
    fn items(v: &Value, l: usize) -> Vec<Value> {
        v["layers"][l]["shapes"][0]["it"].as_array().unwrap().clone()
    }

    #[test]
    fn custom_eases_become_eased_keys_one_per_piece() {
        // ADR 0089: a custom curve of two cubic pieces is three position keys with Lottie
        // eases, the second at the time the curve's middle point falls; the outline stays put.
        use anicraft_doc::{CustomEase, Easing};
        use std::sync::Arc;
        let curve = CustomEase::from_flat(&[(0.0, 0.0), (0.2, 1.4), (0.3, 1.4), (0.4, 1.4), (0.6, 0.5), (0.8, 1.0), (1.0, 1.0)].map(|(x, y)| Vec2::new(x, y))).unwrap();
        let ease = Easing::Custom(Arc::new(curve));
        let doc = tweened(Affine::IDENTITY, Affine::translate(100.0, 0.0), ease.clone(), false);
        let e = to_lottie(&doc, "eased", 0, 14, false);
        assert!(e.report.is_empty(), "{:?}", e.report);
        let v = parse(&e);
        let it = items(&v, 0);
        assert_eq!(it.iter().find(|i| i["ty"] == "sh").unwrap()["ks"]["a"], 0, "the outline doesn't change");
        let p = keys_of(&it.last().unwrap()["p"]);
        assert_eq!(p.iter().map(|k| k.t).collect::<Vec<_>>(), [0.0, 4.0, 10.0], "{p:?}");
        assert!(matches!(p[0].out, Out::Ease(..)) && matches!(p[1].out, Out::Ease(..)));
        for f in 0..=14 {
            let x = crate::lottie_keys::value_at(&p, f64::from(f)).unwrap()[0];
            let want = evaluate(&doc, f).items[0].transform.m[4];
            assert!((x - f64::from(want)).abs() < 2e-3, "frame {f}: {x} vs {want}");
        }
    }

    #[test]
    fn turning_and_growing_are_transform_keys() {
        use anicraft_doc::Easing;
        let a = Affine::translate(50.0, 60.0);
        let b = Affine::translate(150.0, 90.0).mul(&Affine::rotate(2.5)).mul(&Affine::scale(2.0, 0.5));
        let doc = tweened(a, b, Easing::Classic(-70.0), false);
        let e = to_lottie(&doc, "turn", 0, 14, false);
        let v = parse(&e);
        let it = items(&v, 0);
        let tr = it.last().unwrap();
        for (prop, count) in [("p", 2), ("r", 2), ("s", 2)] {
            assert_eq!(keys_of(&tr[prop]).len(), count, "{prop}: {}", tr[prop]);
        }
        assert_eq!(tr["sk"]["a"], 0);
        // Composed the way Lottie composes it, the group transform is the frame's matrix.
        let at = |p: &str, f: u32| crate::lottie_keys::value_at(&keys_of(&tr[p]), f64::from(f)).unwrap();
        for f in 0..=14 {
            let (p, r, s) = (at("p", f), at("r", f)[0], at("s", f));
            let m = compose(&Tr { p: [p[0], p[1]], r, sk: 0.0, s: [s[0], s[1]] });
            let want = evaluate(&doc, f).items[0].transform.m;
            for k in 0..6 {
                assert!((m[k] - f64::from(want[k])).abs() < 1e-3, "frame {f}, m[{k}]: {m:?} vs {want:?}");
            }
        }
        // Earlier builds' keying: an outline per frame in stage space.
        let old = parse(&to_lottie_keyed(&doc, "turn", 0, 14, false, LottieKeys::Frames));
        let sh = items(&old, 0).into_iter().find(|i| i["ty"] == "sh").unwrap();
        assert_eq!(sh["ks"]["k"].as_array().unwrap().len(), 11);
        assert!(e.json.len() * 2 < old.to_string().len(), "{} vs {}", e.json.len(), old.to_string().len());
    }

    #[test]
    fn eases_no_cubic_draws_are_keyed_frame_by_frame_and_reported() {
        let doc = tweened(Affine::IDENTITY, Affine::translate(100.0, 0.0), anicraft_doc::Easing::ElasticOut, false);
        let e = to_lottie(&doc, "elastic", 0, 14, false);
        assert_eq!(e.report.len(), 1, "{:?}", e.report);
        assert_eq!(e.report.keys().next().map(String::as_str), Some(STEPPED));
        let p = keys_of(&items(&parse(&e), 0).last().unwrap()["p"]);
        assert_eq!(p.len(), 11, "a hold key at each frame of the motion: {p:?}");
        for f in 0..=14 {
            let x = crate::lottie_keys::value_at(&p, f64::from(f)).unwrap()[0];
            assert!((x - f64::from(evaluate(&doc, f).items[0].transform.m[4])).abs() < 2e-3);
        }
    }

    #[test]
    fn strokes_stretched_by_a_tween_stay_in_stage_space() {
        use anicraft_doc::Easing;
        // Growing evenly: the stroke scales with the group, as the renderer widens it.
        let even = parse(&to_lottie(&tweened(Affine::IDENTITY, Affine::scale(3.0, 3.0), Easing::Linear, true), "s", 0, 10, false));
        let it = items(&even, 0);
        assert_eq!(keys_of(&it.last().unwrap()["s"]).len(), 2);
        assert_eq!(it.iter().find(|i| i["ty"] == "st").unwrap()["w"]["k"], 2, "the stroke's own width");
        // Stretching: Lottie would stretch the pen too, so the outline is baked per frame.
        let stretched = parse(&to_lottie(&tweened(Affine::IDENTITY, Affine::scale(3.0, 1.0), Easing::Linear, true), "s", 0, 10, false));
        let it = items(&stretched, 0);
        assert_eq!(it.last().unwrap()["s"]["k"], json!([100, 100]));
        assert_eq!(it.iter().find(|i| i["ty"] == "sh").unwrap()["ks"]["a"], 1);
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
        let shape = Shape { path: std::sync::Arc::new(Path::line(Vec2::new(10.0, 10.0), Vec2::new(200.0, 10.0))), fill: None, stroke: Some(Stroke::new(Rgba::hex(0x00ff00), 10.0).with_profile(ProfilePreset::TaperEnd.profile())), fill_rule: FillRule::NonZero, merge: false, warp: None };
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
            let shape = Shape { path: std::sync::Arc::new(Path::line(Vec2::new(10.0, 10.0), Vec2::new(200.0, 10.0))), fill: None, stroke: Some(st), fill_rule: FillRule::NonZero, merge: false, warp: None };
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
        assert_eq!((st["lc"].clone(), st["lj"].clone(), st["ml"].clone(), st["w"]["k"].clone()), (json!(1), json!(1), json!(6), json!(6)));
        let d = st["d"].as_array().unwrap();
        let got: Vec<(&str, f64)> = d.iter().map(|e| (e["n"].as_str().unwrap(), e["v"]["k"].as_f64().unwrap())).collect();
        assert_eq!(got, [("d", 8.0), ("g", 3.0), ("o", 2.0)]);
        // Dots: zero-length dashes with round caps, whatever the cap.
        let st = export(Stroke::new(Rgba::BLACK, 3.0).with_style(StrokeStyle { cap: Cap::Square, join: Join::Bevel, dash: Dash::dotted(2.0), ..StrokeStyle::default() }));
        assert_eq!((st["lc"].clone(), st["lj"].clone()), (json!(2), json!(3)));
        assert_eq!((st["d"][0]["v"]["k"].as_f64(), st["d"][1]["v"]["k"].as_f64()), (Some(0.0), Some(10.0)));
    }
}
