//! Easing curves for tweens (ADR 0016, ADR 0081).
//!
//! An [`Easing`] maps linear tween progress `t ∈ [0, 1]` to eased progress, and every one is a
//! pure function of `t`. There are three kinds:
//!
//! * **Presets**: no ease (linear), the Classic Ease slider (strength −100 … +100), and nine
//!   families, each In, Out and In Out: Quad, Cubic, Quart, Quint, Sine, Back, Circ, Bounce and
//!   Elastic. The formulas are the classic public easing equations (Robert Penner's, as widely
//!   published; credited in `ATTRIBUTION.md`): `In(t)` per family, `Out(t) = 1 − In(1 − t)` and
//!   `InOut` as In over the first half and Out over the second.
//! * **One cubic Bézier** (`CubicBezier`, CSS-style handles relative to the segment's box), as
//!   the Motion Editor's handles and older files make.
//! * **A custom ease** ([`CustomEase`]): a curve of several Bézier segments from (0, 0) to
//!   (1, 1), as the graph editor draws it. Time never runs backwards (x is monotonic along the
//!   whole curve); values may overshoot 0..1.
//! * **A sampled ease** ([`SampledEase`], ADR 0088): a table of values an ease plug-in's
//!   function gave when it loaded, read back by linear interpolation. The document keeps the
//!   table, so it plays the same on a computer without the plug-in.

use anicraft_geom::Vec2;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Most points (anchors) a custom ease keeps; longer curves are thinned evenly.
pub const MAX_EASE_POINTS: usize = 64;
/// Values a custom ease (and its handles) may reach: overshoot well past 0..1, never unbounded.
pub const MAX_EASE_Y: f32 = 10.0;
/// Most points read from a file or a command before thinning (bounds the sort).
const MAX_EASE_INPUT: usize = 65_536;
/// The Back family's overshoot (about 10 %).
const BACK: f64 = 1.701_58;

/// The families of named eases.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EaseFamily {
    Quad,
    Cubic,
    Quart,
    Quint,
    Sine,
    Back,
    Circ,
    Bounce,
    Elastic,
}

impl EaseFamily {
    pub const ALL: [EaseFamily; 9] = [EaseFamily::Quad, EaseFamily::Cubic, EaseFamily::Quart, EaseFamily::Quint, EaseFamily::Sine, EaseFamily::Back, EaseFamily::Circ, EaseFamily::Bounce, EaseFamily::Elastic];

    /// The family's name as the ease menus show it.
    pub fn label(self) -> &'static str {
        match self {
            EaseFamily::Quad => "Quad",
            EaseFamily::Cubic => "Cubic",
            EaseFamily::Quart => "Quart",
            EaseFamily::Quint => "Quint",
            EaseFamily::Sine => "Sine",
            EaseFamily::Back => "Back",
            EaseFamily::Circ => "Circ",
            EaseFamily::Bounce => "Bounce",
            EaseFamily::Elastic => "Elastic",
        }
    }

    /// The ease in, `In(t)`, in double precision.
    fn ease_in(self, t: f64) -> f64 {
        use std::f64::consts::PI;
        match self {
            EaseFamily::Quad => t * t,
            EaseFamily::Cubic => t * t * t,
            EaseFamily::Quart => t * t * t * t,
            EaseFamily::Quint => t * t * t * t * t,
            EaseFamily::Sine => 1.0 - (t * PI * 0.5).cos(),
            EaseFamily::Back => (BACK + 1.0) * t * t * t - BACK * t * t,
            EaseFamily::Circ => 1.0 - (1.0 - t * t).max(0.0).sqrt(),
            EaseFamily::Bounce => 1.0 - bounce_out(1.0 - t),
            EaseFamily::Elastic => {
                if t <= 0.0 {
                    0.0
                } else if t >= 1.0 {
                    1.0
                } else {
                    -(2f64.powf(10.0 * t - 10.0)) * ((10.0 * t - 10.75) * (2.0 * PI / 3.0)).sin()
                }
            }
        }
    }

    /// Where the ease in changes formula (other than 0 and 1): Bounce is four parabolas.
    fn breaks_in(self) -> &'static [f64] {
        match self {
            // 1 − the Out breakpoints (1/2.75, 2/2.75, 2.5/2.75), in order.
            EaseFamily::Bounce => &[0.25 / 2.75, 0.75 / 2.75, 1.75 / 2.75],
            _ => &[],
        }
    }

    /// Bézier segments per ease in that draw it to within a hair (see [`Easing::to_custom`]);
    /// cubic polynomials (Quad, Cubic, Back) and the parabolas of Bounce are exact with one.
    fn pieces_in(self) -> usize {
        match self {
            EaseFamily::Quad | EaseFamily::Cubic | EaseFamily::Back | EaseFamily::Bounce | EaseFamily::Circ => 1,
            EaseFamily::Quart | EaseFamily::Quint | EaseFamily::Sine => 4,
            EaseFamily::Elastic => 24,
        }
    }
}

fn bounce_out(t: f64) -> f64 {
    const N: f64 = 7.5625;
    const D: f64 = 2.75;
    if t < 1.0 / D {
        N * t * t
    } else if t < 2.0 / D {
        let u = t - 1.5 / D;
        N * u * u + 0.75
    } else if t < 2.5 / D {
        let u = t - 2.25 / D;
        N * u * u + 0.9375
    } else {
        let u = t - 2.625 / D;
        N * u * u + 0.984_375
    }
}

/// Which way a named ease eases.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EaseDir {
    In,
    Out,
    InOut,
}

impl EaseDir {
    pub const ALL: [EaseDir; 3] = [EaseDir::In, EaseDir::Out, EaseDir::InOut];

    pub fn label(self) -> &'static str {
        match self {
            EaseDir::In => "In",
            EaseDir::Out => "Out",
            EaseDir::InOut => "In Out",
        }
    }
}

/// Maps linear tween progress `t ∈ [0, 1]` to eased progress.
///
/// The unit variants are the named presets (their names are the file format and the
/// commands' preset names); older files hold the first seven and `Classic`/`CubicBezier`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Easing {
    #[default]
    Linear,
    QuadIn,
    QuadOut,
    QuadInOut,
    CubicIn,
    CubicOut,
    CubicInOut,
    /// The traditional single-slider ease: −100 (ease in) … 0 (linear) … +100 (ease out).
    Classic(f32),
    /// CSS-style cubic Bézier through (0,0), (x1,y1), (x2,y2), (1,1).
    CubicBezier(f32, f32, f32, f32),
    QuartIn,
    QuartOut,
    QuartInOut,
    QuintIn,
    QuintOut,
    QuintInOut,
    SineIn,
    SineOut,
    SineInOut,
    BackIn,
    BackOut,
    BackInOut,
    CircIn,
    CircOut,
    CircInOut,
    BounceIn,
    BounceOut,
    BounceInOut,
    ElasticIn,
    ElasticOut,
    ElasticInOut,
    /// A curve of several Bézier segments (the graph editor's), shared when copied.
    Custom(Arc<CustomEase>),
    /// An ease plug-in's curve, sampled into a table when the plug-in loaded (ADR 0088).
    Sampled(Arc<SampledEase>),
}

/// A named ease the artist saved in the document (ADR 0081): any ease, usually a custom
/// curve, offered with the presets to every classic tween and motion-tween property.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedEase {
    pub name: String,
    pub ease: Easing,
}

/// Most saved eases a document keeps, and the longest name.
pub const MAX_SAVED_EASES: usize = 256;
pub const MAX_EASE_NAME: usize = 64;

/// A saved ease's name as kept: trimmed, at most [`MAX_EASE_NAME`] characters; `None` when
/// nothing is left.
pub fn ease_name(name: &str) -> Option<String> {
    let n: String = name.trim().chars().filter(|c| !c.is_control()).take(MAX_EASE_NAME).collect();
    let n = n.trim().to_string();
    (!n.is_empty()).then_some(n)
}

/// Saved eases as a document keeps them: named, each name once (the first wins, ignoring
/// case), at most [`MAX_SAVED_EASES`].
pub fn sanitized_eases(eases: &[SavedEase]) -> Vec<SavedEase> {
    let mut out: Vec<SavedEase> = Vec::new();
    for e in eases {
        let Some(name) = ease_name(&e.name) else { continue };
        if out.len() >= MAX_SAVED_EASES || out.iter().any(|o| o.name.to_lowercase() == name.to_lowercase()) {
            continue;
        }
        out.push(SavedEase { name, ease: e.ease.clone() });
    }
    out
}

impl Easing {
    /// Named eases with fixed shapes, for labels and the `smooth` command name (the menus list
    /// [`Easing::LINEAR_LABEL`], Classic Ease and the families instead).
    pub const PRESETS: [(&'static str, Easing); 2] = [("Linear", Easing::Linear), ("Smooth (bezier)", Easing::CubicBezier(0.25, 0.1, 0.25, 1.0))];
    pub const LINEAR_LABEL: &'static str = "Linear";

    /// The named ease of `family` easing `dir`.
    pub fn preset(family: EaseFamily, dir: EaseDir) -> Easing {
        use EaseDir::*;
        use EaseFamily::*;
        match (family, dir) {
            (Quad, In) => Easing::QuadIn,
            (Quad, Out) => Easing::QuadOut,
            (Quad, InOut) => Easing::QuadInOut,
            (Cubic, In) => Easing::CubicIn,
            (Cubic, Out) => Easing::CubicOut,
            (Cubic, InOut) => Easing::CubicInOut,
            (Quart, In) => Easing::QuartIn,
            (Quart, Out) => Easing::QuartOut,
            (Quart, InOut) => Easing::QuartInOut,
            (Quint, In) => Easing::QuintIn,
            (Quint, Out) => Easing::QuintOut,
            (Quint, InOut) => Easing::QuintInOut,
            (Sine, In) => Easing::SineIn,
            (Sine, Out) => Easing::SineOut,
            (Sine, InOut) => Easing::SineInOut,
            (Back, In) => Easing::BackIn,
            (Back, Out) => Easing::BackOut,
            (Back, InOut) => Easing::BackInOut,
            (Circ, In) => Easing::CircIn,
            (Circ, Out) => Easing::CircOut,
            (Circ, InOut) => Easing::CircInOut,
            (Bounce, In) => Easing::BounceIn,
            (Bounce, Out) => Easing::BounceOut,
            (Bounce, InOut) => Easing::BounceInOut,
            (Elastic, In) => Easing::ElasticIn,
            (Elastic, Out) => Easing::ElasticOut,
            (Elastic, InOut) => Easing::ElasticInOut,
        }
    }

    /// The family and direction of a named ease.
    pub fn family(&self) -> Option<(EaseFamily, EaseDir)> {
        use EaseDir::*;
        use EaseFamily::*;
        Some(match self {
            Easing::QuadIn => (Quad, In),
            Easing::QuadOut => (Quad, Out),
            Easing::QuadInOut => (Quad, InOut),
            Easing::CubicIn => (Cubic, In),
            Easing::CubicOut => (Cubic, Out),
            Easing::CubicInOut => (Cubic, InOut),
            Easing::QuartIn => (Quart, In),
            Easing::QuartOut => (Quart, Out),
            Easing::QuartInOut => (Quart, InOut),
            Easing::QuintIn => (Quint, In),
            Easing::QuintOut => (Quint, Out),
            Easing::QuintInOut => (Quint, InOut),
            Easing::SineIn => (Sine, In),
            Easing::SineOut => (Sine, Out),
            Easing::SineInOut => (Sine, InOut),
            Easing::BackIn => (Back, In),
            Easing::BackOut => (Back, Out),
            Easing::BackInOut => (Back, InOut),
            Easing::CircIn => (Circ, In),
            Easing::CircOut => (Circ, Out),
            Easing::CircInOut => (Circ, InOut),
            Easing::BounceIn => (Bounce, In),
            Easing::BounceOut => (Bounce, Out),
            Easing::BounceInOut => (Bounce, InOut),
            Easing::ElasticIn => (Elastic, In),
            Easing::ElasticOut => (Elastic, Out),
            Easing::ElasticInOut => (Elastic, InOut),
            Easing::Linear | Easing::Classic(_) | Easing::CubicBezier(..) | Easing::Custom(_) | Easing::Sampled(_) => return None,
        })
    }

    /// Every named preset: no ease, then each family In, Out, In Out.
    pub fn named() -> impl Iterator<Item = Easing> {
        std::iter::once(Easing::Linear).chain(EaseFamily::ALL.into_iter().flat_map(|f| EaseDir::ALL.into_iter().map(move |d| Easing::preset(f, d))))
    }

    /// The preset's name in commands and files (`"QuadIn"`, `"ElasticInOut"`), `None` for
    /// eases with values.
    pub fn name(&self) -> Option<String> {
        match self {
            Easing::Linear => Some("Linear".into()),
            _ => self.family().map(|(f, d)| format!("{}{}", f.label(), d.label().replace(' ', ""))),
        }
    }

    /// The preset named `name`, ignoring case, spaces, `-` and `_` (`"back in out"`), with
    /// the older aliases (`none`, `easein`, `easeout`, `easeinout`, `smooth`).
    pub fn from_name(name: &str) -> Option<Easing> {
        let key: String = name.chars().filter(|c| !matches!(c, '-' | '_' | ' ')).flat_map(char::to_lowercase).collect();
        match key.as_str() {
            "none" | "noease" => return Some(Easing::Linear),
            "easein" => return Some(Easing::QuadIn),
            "easeout" => return Some(Easing::QuadOut),
            "easeinout" => return Some(Easing::QuadInOut),
            "smooth" => return Some(Easing::CubicBezier(0.25, 0.1, 0.25, 1.0)),
            _ => {}
        }
        Easing::named().find(|e| e.name().is_some_and(|n| n.to_lowercase() == key))
    }

    /// The English label menus and reports show.
    pub fn label(&self) -> String {
        match self {
            Easing::Linear => Easing::LINEAR_LABEL.to_string(),
            Easing::Classic(s) => format!("Classic {s:+.0}"),
            Easing::CubicBezier(a, b, c, d) => {
                Easing::PRESETS.iter().find(|(_, e)| e == self).map_or_else(|| format!("Bezier({a:.2}, {b:.2}, {c:.2}, {d:.2})"), |(n, _)| (*n).to_string())
            }
            Easing::Custom(_) => "Custom".to_string(),
            Easing::Sampled(s) => s.name().to_string(),
            other => other.family().map_or_else(|| "Custom".to_string(), |(f, d)| format!("{} {}", f.label(), d.label())),
        }
    }

    /// The single cubic Bézier `(x1, y1, x2, y2)` that draws this ease, as the Motion Editor's
    /// handles: exact for linear, the classic slider, the Quad, Cubic and Back eases in and
    /// out, and a custom ease of one segment; the Quad and Cubic in-outs get their usual
    /// one-curve approximations. `None` for eases one curve can't draw (Bounce, Elastic, the
    /// other families, and custom eases of several segments): they are edited in the graph.
    pub fn as_bezier(&self) -> Option<(f32, f32, f32, f32)> {
        const THIRD: f32 = 1.0 / 3.0;
        const B: f32 = BACK as f32;
        Some(match self {
            Easing::Linear => (THIRD, THIRD, 2.0 * THIRD, 2.0 * THIRD),
            // t², t³ and (c+1)t³ − ct² and their mirrors, with x(s) = s.
            Easing::QuadIn => (THIRD, 0.0, 2.0 * THIRD, THIRD),
            Easing::QuadOut => (THIRD, 2.0 * THIRD, 2.0 * THIRD, 1.0),
            Easing::CubicIn => (THIRD, 0.0, 2.0 * THIRD, 0.0),
            Easing::CubicOut => (THIRD, 1.0, 2.0 * THIRD, 1.0),
            Easing::BackIn => (THIRD, 0.0, 2.0 * THIRD, -B * THIRD),
            Easing::BackOut => (THIRD, 1.0 + B * THIRD, 2.0 * THIRD, 1.0),
            Easing::QuadInOut => (0.45, 0.0, 0.55, 1.0),
            Easing::CubicInOut => (0.65, 0.0, 0.35, 1.0),
            Easing::Classic(strength) => {
                // The quadratic 2(1 − t)t·c + t², raised to a cubic.
                let c = 0.5 + 0.5 * classic_strength(*strength);
                (THIRD, 2.0 * c * THIRD, 2.0 * THIRD, (2.0 * c + 1.0) * THIRD)
            }
            Easing::CubicBezier(a, b, c, d) => (*a, *b, *c, *d),
            Easing::Custom(c) => match c.points() {
                [a, b] => (a.out.x, a.out.y, b.at.x + b.h_in.x, b.at.y + b.h_in.y),
                _ => return None,
            },
            _ => return None,
        })
    }

    pub fn apply(&self, t: f32) -> f32 {
        let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
        match self {
            Easing::Linear => t,
            Easing::Classic(strength) => {
                // Quadratic Bézier with the middle control point moved by `strength`.
                let c = 0.5 + 0.5 * classic_strength(*strength); // y of control point at x = 0.5
                2.0 * (1.0 - t) * t * c + t * t
            }
            Easing::CubicBezier(x1, y1, x2, y2) => cubic_bezier(t, *x1, *y1, *x2, *y2),
            Easing::Custom(c) => c.apply(t),
            Easing::Sampled(s) => s.apply(t),
            named => match named.family() {
                Some((f, d)) => eval_named(f, d, f64::from(t)) as f32,
                None => t,
            },
        }
    }

    /// This ease as a custom curve for the graph editor: exact for linear, Classic, a Bézier,
    /// the Quad, Cubic, Back and Bounce families and custom eases; Circ is two circular arcs,
    /// and Quart, Quint, Sine and Elastic are fitted by matching values and slopes at a few
    /// points (all within 0.5 % of the range everywhere, see the tests). A sampled ease becomes
    /// the straight segments through 48 of its samples.
    pub fn to_custom(&self) -> CustomEase {
        match self {
            Easing::Custom(c) => (**c).clone(),
            Easing::Sampled(s) => s.to_polyline(),
            Easing::Linear => CustomEase::linear(),
            Easing::Classic(_) | Easing::CubicBezier(..) => {
                let Some((x1, y1, x2, y2)) = self.as_bezier() else { return CustomEase::linear() };
                CustomEase::from_bezier(x1, y1, x2, y2)
            }
            named => match named.family() {
                Some((f, d)) => fit_named(f, d),
                None => CustomEase::linear(),
            },
        }
    }

    /// The custom curve, when this is one.
    pub fn custom(&self) -> Option<&CustomEase> {
        match self {
            Easing::Custom(c) => Some(c),
            _ => None,
        }
    }

    /// The sampled curve, when this is a plug-in's ease.
    pub fn sampled(&self) -> Option<&SampledEase> {
        match self {
            Easing::Sampled(s) => Some(s),
            _ => None,
        }
    }
}

fn classic_strength(s: f32) -> f32 {
    if s.is_finite() { s.clamp(-100.0, 100.0) / 100.0 } else { 0.0 }
}

fn eval_named(f: EaseFamily, d: EaseDir, t: f64) -> f64 {
    match d {
        EaseDir::In => f.ease_in(t),
        EaseDir::Out => 1.0 - f.ease_in(1.0 - t),
        EaseDir::InOut => {
            if t < 0.5 {
                0.5 * f.ease_in(2.0 * t)
            } else {
                1.0 - 0.5 * f.ease_in(2.0 - 2.0 * t)
            }
        }
    }
}

/// The slope of `y` at `x`, from inside `[lo, hi]`: one-sided, a step in from `x` (so a
/// value pinned at an end, as Elastic's are, doesn't count), exact for parabolas but for that
/// step.
fn slope(y: &dyn Fn(f64) -> f64, x: f64, lo: f64, hi: f64) -> f64 {
    const H: f64 = 1e-6;
    let s = if x + 3.0 * H <= hi {
        let d = (-3.0 * y(x + H) + 4.0 * y(x + 2.0 * H) - y(x + 3.0 * H)) / (2.0 * H);
        let d2 = (y(x + H) - 2.0 * y(x + 2.0 * H) + y(x + 3.0 * H)) / (H * H);
        d - d2 * H
    } else if x - 3.0 * H >= lo {
        let d = (3.0 * y(x - H) - 4.0 * y(x - 2.0 * H) + y(x - 3.0 * H)) / (2.0 * H);
        let d2 = (y(x - H) - 2.0 * y(x - 2.0 * H) + y(x - 3.0 * H)) / (H * H);
        d + d2 * H
    } else {
        0.0
    };
    if s.is_finite() { s } else { 0.0 }
}

/// The ease in of `f` as Bézier points: Hermite segments (value and slope matched at each
/// end, so exact for cubic polynomials and for each parabola of Bounce), except Circ, whose
/// quarter circle is two 45° arcs (within 4·10⁻⁶ of the circle).
fn fit_in(f: EaseFamily) -> Vec<EasePoint> {
    let pt = |x: f64, y: f64, i: (f64, f64), o: (f64, f64)| EasePoint { at: Vec2::new(x as f32, y as f32), h_in: Vec2::new(i.0 as f32, i.1 as f32), out: Vec2::new(o.0 as f32, o.1 as f32), smooth: false };
    if f == EaseFamily::Circ {
        // The circle about (0, 1) from (0, 0) to (1, 1); each arc's handles are k = 4/3·tan(φ/4)
        // long along the tangents.
        let k = 4.0 / 3.0 * (std::f64::consts::PI / 16.0).tan();
        let c = std::f64::consts::FRAC_1_SQRT_2;
        return vec![pt(0.0, 0.0, (0.0, 0.0), (k, 0.0)), pt(c, 1.0 - c, (-k * c, -k * c), (k * c, k * c)), pt(1.0, 1.0, (0.0, -k), (0.0, 0.0))];
    }
    let y = move |t: f64| f.ease_in(t);
    let mut xs: Vec<f64> = Vec::new();
    let pieces = f.pieces_in();
    let bounds: Vec<f64> = std::iter::once(0.0).chain(f.breaks_in().iter().copied()).chain(std::iter::once(1.0)).collect();
    for w in bounds.windows(2) {
        if let [a, c] = w {
            xs.extend((0..pieces).map(|k| a + (c - a) * k as f64 / pieces as f64));
        }
    }
    xs.push(1.0);
    let mut pts: Vec<EasePoint> = xs.iter().map(|&x| pt(x, y(x), (0.0, 0.0), (0.0, 0.0))).collect();
    for i in 0..xs.len().saturating_sub(1) {
        let (Some(&a), Some(&b)) = (xs.get(i), xs.get(i + 1)) else { continue };
        let h = (b - a) / 3.0;
        let (sa, sb) = (slope(&y, a, a, b), slope(&y, b, a, b));
        if let Some(p) = pts.get_mut(i) {
            p.out = Vec2::new(h as f32, (sa * h) as f32);
        }
        if let Some(p) = pts.get_mut(i + 1) {
            p.h_in = Vec2::new(-h as f32, (-sb * h) as f32);
        }
    }
    pts
}

/// The curve turned half a turn about (½, ½): an ease in becomes the ease out.
fn mirrored(pts: &[EasePoint]) -> Vec<EasePoint> {
    pts.iter().rev().map(|p| EasePoint { at: Vec2::new(1.0 - p.at.x, 1.0 - p.at.y), h_in: -p.out, out: -p.h_in, smooth: p.smooth }).collect()
}

fn scaled(pts: &[EasePoint], by: f32, from: Vec2) -> Vec<EasePoint> {
    pts.iter().map(|p| EasePoint { at: from + p.at * by, h_in: p.h_in * by, out: p.out * by, smooth: p.smooth }).collect()
}

/// A named ease as a custom curve (see [`Easing::to_custom`]).
fn fit_named(f: EaseFamily, d: EaseDir) -> CustomEase {
    let ease_in = fit_in(f);
    let mut pts = match d {
        EaseDir::In => ease_in,
        EaseDir::Out => mirrored(&ease_in),
        EaseDir::InOut => {
            let mut a = scaled(&ease_in, 0.5, Vec2::ZERO);
            let b = scaled(&mirrored(&ease_in), 0.5, Vec2::new(0.5, 0.5));
            // The two halves meet at (½, ½): one point, the first half's in, the second's out.
            if let (Some(last), Some(first)) = (a.last_mut(), b.first()) {
                last.out = first.out;
            }
            a.extend(b.into_iter().skip(1));
            a
        }
    };
    // Points where the slope carries on are smooth (Bounce's floor hits are corners).
    let n = pts.len();
    for (i, p) in pts.iter_mut().enumerate() {
        p.smooth = i > 0 && i + 1 < n && p.h_in.length() > 1e-9 && p.out.length() > 1e-9 && (p.h_in.normalized().dot(p.out.normalized()) + 1.0).abs() < 1e-3;
    }
    CustomEase::new(pts)
}

fn cubic_bezier(x: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    if ![x1, y1, x2, y2].iter().all(|v| v.is_finite()) {
        return x;
    }
    // x control points must stay in [0, 1] so x(t) is monotonic.
    let (x1, x2) = (x1.clamp(0.0, 1.0), x2.clamp(0.0, 1.0));
    let s = solve_monotonic(f64::from(x), [0.0, f64::from(x1), f64::from(x2), 1.0]);
    bez(s, [0.0, f64::from(y1), f64::from(y2), 1.0]) as f32
}

fn bez(t: f64, p: [f64; 4]) -> f64 {
    let mt = 1.0 - t;
    mt * mt * mt * p[0] + 3.0 * mt * mt * t * p[1] + 3.0 * mt * t * t * p[2] + t * t * t * p[3]
}

fn dbez(t: f64, p: [f64; 4]) -> f64 {
    let mt = 1.0 - t;
    3.0 * mt * mt * (p[1] - p[0]) + 6.0 * mt * t * (p[2] - p[1]) + 3.0 * t * t * (p[3] - p[2])
}

/// The parameter where the Bézier `p` (non-decreasing: its inner points lie between its ends)
/// reaches `x`: Newton steps kept inside a shrinking bracket, halving when a step leaves it.
fn solve_monotonic(x: f64, p: [f64; 4]) -> f64 {
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    let span = p[3] - p[0];
    let mut s = if span > 1e-12 { ((x - p[0]) / span).clamp(0.0, 1.0) } else { 0.5 };
    for _ in 0..48 {
        let err = bez(s, p) - x;
        if err.abs() < 1e-10 {
            break;
        }
        if err < 0.0 {
            lo = s;
        } else {
            hi = s;
        }
        let d = dbez(s, p);
        let next = if d.abs() > 1e-12 { s - err / d } else { f64::NAN };
        s = if next > lo && next < hi { next } else { 0.5 * (lo + hi) };
    }
    s
}

/// A point of a custom ease: where it is (x = time, y = progress), its handles as offsets
/// from it (in towards the previous point, out towards the next), and whether it is smooth
/// (handles in line, moved together) or a corner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EasePoint {
    pub at: Vec2,
    #[serde(rename = "in", default)]
    pub h_in: Vec2,
    #[serde(default)]
    pub out: Vec2,
    #[serde(default, skip_serializing_if = "is_false")]
    pub smooth: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Which handle of a point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    In,
    Out,
}

/// A custom ease: Bézier segments through its points, from (0, 0) to (1, 1).
///
/// Always valid, however it was made (a file, a command, an edit): 2 to [`MAX_EASE_POINTS`]
/// finite points sorted by x, the ends fixed at (0, 0) and (1, 1), every handle's x between
/// its point and the neighbour it reaches towards (so each segment's x runs forwards, and time
/// never runs backwards), and every value within ±[`MAX_EASE_Y`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "CustomEaseFile")]
pub struct CustomEase {
    points: Vec<EasePoint>,
}

#[derive(Deserialize)]
struct CustomEaseFile {
    #[serde(default)]
    points: Vec<EasePoint>,
}

impl From<CustomEaseFile> for CustomEase {
    fn from(f: CustomEaseFile) -> CustomEase {
        CustomEase::new(f.points)
    }
}

impl Default for CustomEase {
    fn default() -> CustomEase {
        CustomEase::linear()
    }
}

impl CustomEase {
    /// The straight line, its handles a third of the way along.
    pub fn linear() -> CustomEase {
        CustomEase::from_bezier(1.0 / 3.0, 1.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0)
    }

    /// One segment with CSS-style handles.
    pub fn from_bezier(x1: f32, y1: f32, x2: f32, y2: f32) -> CustomEase {
        CustomEase::new(vec![EasePoint { at: Vec2::ZERO, h_in: Vec2::ZERO, out: Vec2::new(x1, y1), smooth: false }, EasePoint { at: Vec2::new(1.0, 1.0), h_in: Vec2::new(x2 - 1.0, y2 - 1.0), out: Vec2::ZERO, smooth: false }])
    }

    /// The curve of absolute points `[p0, c1, c2, p1, c3, c4, p2, …]` (3n + 1 of them, as XFL
    /// writes a custom ease): `None` when the count is wrong.
    pub fn from_flat(pts: &[Vec2]) -> Option<CustomEase> {
        if pts.len() < 4 || pts.len() % 3 != 1 || pts.len() > MAX_EASE_INPUT {
            return None;
        }
        let mut out: Vec<EasePoint> = Vec::with_capacity(pts.len() / 3 + 1);
        for (i, at) in pts.iter().enumerate().step_by(3) {
            let h_in = if i == 0 { Vec2::ZERO } else { pts.get(i - 1).map_or(Vec2::ZERO, |c| *c - *at) };
            let h_out = pts.get(i + 1).map_or(Vec2::ZERO, |c| *c - *at);
            // Handles in line (opposite ways) make a smooth point, as the editor would draw it.
            let smooth = h_in.length() > 1e-6 && h_out.length() > 1e-6 && (h_in.normalized().dot(h_out.normalized()) + 1.0).abs() < 1e-3;
            out.push(EasePoint { at: *at, h_in, out: h_out, smooth });
        }
        Some(CustomEase::new(out))
    }

    /// A valid curve from any points (see the type's docs): non-finite points are dropped, the
    /// rest sorted by x, a curve longer than [`MAX_EASE_POINTS`] thinned evenly, the ends
    /// pinned and every handle brought within reach.
    pub fn new(mut pts: Vec<EasePoint>) -> CustomEase {
        pts.truncate(MAX_EASE_INPUT);
        pts.retain(|p| p.at.is_finite());
        for p in pts.iter_mut() {
            p.at = Vec2::new(p.at.x.clamp(0.0, 1.0), p.at.y.clamp(-MAX_EASE_Y, MAX_EASE_Y));
            if !p.h_in.is_finite() {
                p.h_in = Vec2::ZERO;
            }
            if !p.out.is_finite() {
                p.out = Vec2::ZERO;
            }
        }
        pts.sort_by(|a, b| a.at.x.total_cmp(&b.at.x));
        if pts.len() > MAX_EASE_POINTS {
            let n = pts.len();
            let keep: Vec<usize> = (0..MAX_EASE_POINTS).map(|k| k * (n - 1) / (MAX_EASE_POINTS - 1)).collect();
            pts = keep.into_iter().filter_map(|i| pts.get(i).copied()).collect();
        }
        if pts.len() < 2 {
            return CustomEase::linear();
        }
        if let Some(p) = pts.first_mut() {
            p.at = Vec2::ZERO;
        }
        if let Some(p) = pts.last_mut() {
            p.at = Vec2::new(1.0, 1.0);
        }
        let mut c = CustomEase { points: pts };
        c.clamp_handles();
        c
    }

    /// Brings every handle within reach (see the type's docs).
    fn clamp_handles(&mut self) {
        let xs: Vec<f32> = self.points.iter().map(|p| p.at.x).collect();
        let n = self.points.len();
        for (i, p) in self.points.iter_mut().enumerate() {
            let prev = if i == 0 { p.at.x } else { xs.get(i - 1).copied().unwrap_or(p.at.x) };
            let next = xs.get(i + 1).copied().unwrap_or(p.at.x);
            let y = |h: f32, at: f32| h.clamp(-MAX_EASE_Y - at, MAX_EASE_Y - at);
            p.h_in = if i == 0 { Vec2::ZERO } else { Vec2::new(p.h_in.x.clamp(prev - p.at.x, 0.0), y(p.h_in.y, p.at.y)) };
            p.out = if i + 1 == n { Vec2::ZERO } else { Vec2::new(p.out.x.clamp(0.0, next - p.at.x), y(p.out.y, p.at.y)) };
        }
    }

    pub fn points(&self) -> &[EasePoint] {
        &self.points
    }

    /// The control points of segment `i` (from point `i` to point `i + 1`), absolute; the
    /// inner points' x always between the ends' (so x runs forwards along it).
    pub fn segment(&self, i: usize) -> Option<[Vec2; 4]> {
        let (a, b) = (self.points.get(i)?, self.points.get(i + 1)?);
        let inner = |p: Vec2| Vec2::new(p.x.clamp(a.at.x, b.at.x.max(a.at.x)), p.y);
        Some([a.at, inner(a.at + a.out), inner(b.at + b.h_in), b.at])
    }

    /// The segment that time `t` falls in.
    fn segment_at(&self, t: f32) -> usize {
        self.points.partition_point(|p| p.at.x <= t).saturating_sub(1).min(self.points.len().saturating_sub(2))
    }

    /// The eased value at time `t` (0 at the start, 1 at the end, anything between, overshoot
    /// included).
    pub fn apply(&self, t: f32) -> f32 {
        let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
        if t <= 0.0 {
            return 0.0;
        }
        if t >= 1.0 {
            return 1.0;
        }
        let Some([p0, p1, p2, p3]) = self.segment(self.segment_at(t)) else { return t };
        if p3.x - p0.x <= 1e-9 {
            return p3.y;
        }
        let s = solve_monotonic(f64::from(t), [p0.x, p1.x, p2.x, p3.x].map(f64::from));
        bez(s, [p0.y, p1.y, p2.y, p3.y].map(f64::from)) as f32
    }

    /// Adds a point on the curve at time `t`, leaving its shape exactly as it was (the segment
    /// is split in two): the new curve and the point's index. `None` at the ends, on an
    /// existing point, or when the curve is full.
    pub fn insert_at(&self, t: f32) -> Option<(CustomEase, usize)> {
        if !t.is_finite() || t <= 1e-4 || t >= 1.0 - 1e-4 || self.points.len() >= MAX_EASE_POINTS || self.points.iter().any(|p| (p.at.x - t).abs() < 1e-4) {
            return None;
        }
        let i = self.segment_at(t);
        let [p0, p1, p2, p3] = self.segment(i)?;
        let s = solve_monotonic(f64::from(t), [p0.x, p1.x, p2.x, p3.x].map(f64::from)) as f32;
        // de Casteljau at s.
        let (a, b, c) = (p0.lerp(p1, s), p1.lerp(p2, s), p2.lerp(p3, s));
        let (d, e) = (a.lerp(b, s), b.lerp(c, s));
        let m = d.lerp(e, s);
        let mut pts = self.points.clone();
        if let Some(q) = pts.get_mut(i) {
            q.out = a - p0;
        }
        if let Some(q) = pts.get_mut(i + 1) {
            q.h_in = c - p3;
        }
        pts.insert(i + 1, EasePoint { at: m, h_in: d - m, out: e - m, smooth: true });
        Some((CustomEase::new(pts), i + 1))
    }

    /// Moves point `i` (not an end) to `to`, its handles with it; its x stays between its
    /// neighbours' so time keeps running forwards. `None` for an end or a missing point.
    pub fn move_point(&self, i: usize, to: Vec2) -> Option<CustomEase> {
        if i == 0 || i + 1 >= self.points.len() || !to.is_finite() {
            return None;
        }
        let (lo, hi) = (self.points.get(i - 1)?.at.x, self.points.get(i + 1)?.at.x);
        let mut c = self.clone();
        let p = c.points.get_mut(i)?;
        p.at = Vec2::new(to.x.clamp(lo, hi), to.y.clamp(-MAX_EASE_Y, MAX_EASE_Y));
        c.clamp_handles();
        Some(c)
    }

    /// Moves handle `which` of point `i` to `to` (absolute); a smooth point's other handle
    /// turns to stay in line, keeping its length. `None` for a handle the point doesn't have
    /// (the first point's in, the last's out).
    pub fn move_handle(&self, i: usize, which: Handle, to: Vec2) -> Option<CustomEase> {
        let n = self.points.len();
        if !to.is_finite() || i >= n || (which == Handle::In && i == 0) || (which == Handle::Out && i + 1 == n) {
            return None;
        }
        let mut c = self.clone();
        let p = c.points.get_mut(i)?;
        let off = to - p.at;
        match which {
            Handle::In => p.h_in = off,
            Handle::Out => p.out = off,
        }
        c.clamp_handles();
        if let Some(p) = c.points.get_mut(i).filter(|p| p.smooth && i > 0 && i + 1 < n) {
            let (moved, other) = match which {
                Handle::In => (p.h_in, &mut p.out),
                Handle::Out => (p.out, &mut p.h_in),
            };
            let dir = moved.normalized();
            if dir != Vec2::ZERO {
                *other = -dir * other.length().max(1e-3);
            }
        }
        c.clamp_handles();
        Some(c)
    }

    /// Removes point `i` (not an end). `None` for an end or a missing point.
    pub fn remove(&self, i: usize) -> Option<CustomEase> {
        if i == 0 || i + 1 >= self.points.len() {
            return None;
        }
        let mut pts = self.points.clone();
        pts.remove(i);
        Some(CustomEase::new(pts))
    }

    /// Makes point `i` smooth (its handles in line with its neighbours' chord, a third of the
    /// way to each) or a corner (its handles on the point, so the curve turns sharply there).
    pub fn set_smooth(&self, i: usize, smooth: bool) -> Option<CustomEase> {
        let n = self.points.len();
        let prev = if i == 0 { None } else { self.points.get(i - 1).map(|p| p.at) };
        let next = self.points.get(i + 1).map(|p| p.at);
        let mut c = self.clone();
        let p = c.points.get_mut(i)?;
        p.smooth = smooth && i > 0 && i + 1 < n;
        if smooth {
            let (a, b) = (prev.unwrap_or(p.at), next.unwrap_or(p.at));
            let chord = b - a;
            let dir = if chord.x > 1e-6 { chord * (1.0 / chord.x) } else { Vec2::new(1.0, 0.0) };
            if prev.is_some() {
                p.h_in = -dir * ((p.at.x - a.x) / 3.0);
            }
            if next.is_some() {
                p.out = dir * ((b.x - p.at.x) / 3.0);
            }
        } else {
            p.h_in = Vec2::ZERO;
            p.out = Vec2::ZERO;
        }
        c.clamp_handles();
        Some(c)
    }

    /// The flat list of absolute points XFL writes (`[p0, c1, c2, p1, …]`).
    pub fn to_flat(&self) -> Vec<Vec2> {
        let mut out = Vec::with_capacity(self.points.len() * 3);
        for (i, p) in self.points.iter().enumerate() {
            if i > 0 {
                out.push(p.at + p.h_in);
            }
            out.push(p.at);
            if i + 1 < self.points.len() {
                out.push(p.at + p.out);
            }
        }
        out
    }
}

/// Most samples a sampled ease keeps (longer tables are thinned evenly); plug-in eases are
/// sampled at 257 points.
pub const MAX_EASE_SAMPLES: usize = 1025;
/// Points of the custom curve a sampled ease becomes when edited (of [`MAX_EASE_POINTS`]).
pub const POLYLINE_POINTS: usize = 48;
/// Longest plug-in or ease id a sampled ease keeps.
const MAX_EASE_ID: usize = 64;

/// An ease plug-in's curve as a table (ADR 0088): `values[i]` is the eased progress at
/// `t = i / (n - 1)`, read between samples by linear interpolation, so evaluation is a pure,
/// constant-time function of `t` that needs no plug-in. It remembers its name and where it came
/// from (the plug-in's id and the ease's id), for the ease menu.
///
/// Always valid, however it was made (a plug-in, a file, a command): 2 to
/// [`MAX_EASE_SAMPLES`] finite values within ±[`MAX_EASE_Y`], starting at exactly 0 and ending
/// at exactly 1.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "SampledEaseFile")]
pub struct SampledEase {
    name: String,
    plugin: String,
    ease: String,
    values: Vec<f32>,
}

#[derive(Deserialize)]
struct SampledEaseFile {
    #[serde(default)]
    name: String,
    #[serde(default)]
    plugin: String,
    #[serde(default)]
    ease: String,
    #[serde(default)]
    values: Vec<f32>,
}

impl From<SampledEaseFile> for SampledEase {
    fn from(f: SampledEaseFile) -> SampledEase {
        SampledEase::new(&f.name, &f.plugin, &f.ease, f.values)
    }
}

/// An id as a sampled ease keeps it: `[A-Za-z0-9._-]`, at most [`MAX_EASE_ID`] characters.
fn ease_id(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')).take(MAX_EASE_ID).collect()
}

impl SampledEase {
    /// A valid table from any values (see the type's docs): a longer table is thinned evenly,
    /// a value that isn't finite becomes the straight line's, values are clamped, the ends
    /// pinned; fewer than two values make the straight line.
    pub fn new(name: &str, plugin: &str, ease: &str, mut values: Vec<f32>) -> SampledEase {
        if values.len() > MAX_EASE_SAMPLES {
            let n = values.len();
            values = (0..MAX_EASE_SAMPLES).filter_map(|k| values.get(k * (n - 1) / (MAX_EASE_SAMPLES - 1)).copied()).collect();
        }
        if values.len() < 2 {
            values = vec![0.0, 1.0];
        }
        let last = (values.len() - 1) as f32;
        for (i, v) in values.iter_mut().enumerate() {
            *v = if v.is_finite() { v.clamp(-MAX_EASE_Y, MAX_EASE_Y) } else { i as f32 / last };
        }
        if let Some(v) = values.first_mut() {
            *v = 0.0;
        }
        if let Some(v) = values.last_mut() {
            *v = 1.0;
        }
        SampledEase { name: ease_name(name).unwrap_or_else(|| "Plug-in ease".to_string()), plugin: ease_id(plugin), ease: ease_id(ease), values }
    }

    /// The name the ease menus show.
    pub fn name(&self) -> &str {
        &self.name
    }
    /// The id of the plug-in it came from.
    pub fn plugin(&self) -> &str {
        &self.plugin
    }
    /// Its id within that plug-in.
    pub fn ease(&self) -> &str {
        &self.ease
    }
    pub fn values(&self) -> &[f32] {
        &self.values
    }

    /// The eased value at time `t`, between the two nearest samples.
    pub fn apply(&self, t: f32) -> f32 {
        let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
        let last = self.values.len().saturating_sub(1);
        if last == 0 {
            return t;
        }
        let x = t * last as f32;
        let i = (x.floor() as usize).min(last - 1);
        let f = x - i as f32;
        let (a, b) = (self.values.get(i).copied().unwrap_or(0.0), self.values.get(i + 1).copied().unwrap_or(1.0));
        a + (b - a) * f
    }

    /// The custom curve of straight segments through [`POLYLINE_POINTS`] of its samples, for
    /// the graph editor: editing a plug-in's ease makes it a custom ease drawn like it, with
    /// room left to add points.
    pub fn to_polyline(&self) -> CustomEase {
        let n = self.values.len();
        let m = POLYLINE_POINTS.min(n).max(2);
        let at: Vec<Vec2> = (0..m)
            .map(|k| {
                let i = k * (n - 1) / (m - 1);
                Vec2::new(i as f32 / (n - 1).max(1) as f32, self.values.get(i).copied().unwrap_or(0.0))
            })
            .collect();
        let pts = at
            .iter()
            .enumerate()
            .map(|(k, p)| {
                let prev = if k == 0 { *p } else { at.get(k - 1).copied().unwrap_or(*p) };
                let next = at.get(k + 1).copied().unwrap_or(*p);
                EasePoint { at: *p, h_in: (prev - *p) * (1.0 / 3.0), out: (next - *p) * (1.0 / 3.0), smooth: false }
            })
            .collect();
        CustomEase::new(pts)
    }
}

#[cfg(test)]
#[path = "easing_tests.rs"]
mod tests;
