//! Stroke caps, joins, the miter limit, and dashed and dotted lines (ADR 0046).
//!
//! A [`StrokeStyle`] sits on every [`crate::Stroke`]. Its default is what every stroke had
//! before: round caps, round joins, solid, so a stroke with the default style is written to a
//! file exactly as before and older files open unchanged. Dash and dot lengths are in the
//! shape's own units, like the stroke width: scaling the shape scales them.

use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// How an open end of a stroke (and each dash) ends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cap {
    /// Flat, exactly at the end (Animate's "None", SVG's `butt`).
    Butt,
    /// A half disc past the end.
    #[default]
    Round,
    /// Flat, half the width past the end.
    Square,
}

impl Cap {
    pub const ALL: [Cap; 3] = [Cap::Butt, Cap::Round, Cap::Square];

    /// The name used in commands and files.
    pub fn key(self) -> &'static str {
        match self {
            Cap::Butt => "butt",
            Cap::Round => "round",
            Cap::Square => "square",
        }
    }

    /// As Animate's Properties name it.
    pub fn label(self) -> &'static str {
        match self {
            Cap::Butt => "None",
            Cap::Round => "Round",
            Cap::Square => "Square",
        }
    }

    /// A cap by name; `none` (Animate's word) is `butt`.
    pub fn from_key(s: &str) -> Option<Cap> {
        match s.trim().to_ascii_lowercase().as_str() {
            "butt" | "none" | "flat" => Some(Cap::Butt),
            "round" => Some(Cap::Round),
            "square" => Some(Cap::Square),
            _ => None,
        }
    }
}

/// How a stroke turns its corners.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Join {
    /// Sharp: the outer edges meet in a point, unless that point is farther than the miter limit
    /// allows, where the corner is bevelled.
    Miter,
    #[default]
    Round,
    /// Cut straight across the outer corner.
    Bevel,
}

impl Join {
    pub const ALL: [Join; 3] = [Join::Miter, Join::Round, Join::Bevel];

    pub fn key(self) -> &'static str {
        match self {
            Join::Miter => "miter",
            Join::Round => "round",
            Join::Bevel => "bevel",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Join::Miter => "Miter",
            Join::Round => "Round",
            Join::Bevel => "Bevel",
        }
    }

    pub fn from_key(s: &str) -> Option<Join> {
        match s.trim().to_ascii_lowercase().as_str() {
            "miter" | "mitre" => Some(Join::Miter),
            "round" => Some(Join::Round),
            "bevel" => Some(Join::Bevel),
            _ => None,
        }
    }
}

/// The miter limit new strokes get (Animate's default). SVG's own default is 4; the SVG
/// importer sets that when a file leaves it out.
pub const DEFAULT_MITER_LIMIT: f32 = 3.0;
/// The miter limit's range: a ratio of the corner's point length to the stroke width.
pub const MIN_MITER_LIMIT: f32 = 1.0;
pub const MAX_MITER_LIMIT: f32 = 100.0;
/// Most on and off lengths in a dash pattern.
pub const MAX_DASH_ENTRIES: usize = 16;
/// Longest dash, gap or dot spacing (shape units).
pub const MAX_DASH_LENGTH: f32 = 10_000.0;
/// A pattern shorter than this (shape units) draws solid.
const MIN_PERIOD: f32 = 1e-3;

/// A broken line: dashes, or round dots.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Dash {
    /// Dashes: lengths that alternate on and off along each subpath, starting with a dash
    /// (an even count, as SVG's `stroke-dasharray` repeats an odd one), each dash capped with
    /// the stroke's cap. `offset` is how far into the pattern each subpath starts.
    Dashed {
        lengths: Arc<[f32]>,
        #[serde(default)]
        offset: f32,
    },
    /// Round dots one stroke width across, `spacing` apart edge to edge.
    Dotted { spacing: f32 },
}

/// The line styles Properties offers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DashKind {
    #[default]
    Solid,
    Dashed,
    Dotted,
}

impl DashKind {
    pub const ALL: [DashKind; 3] = [DashKind::Solid, DashKind::Dashed, DashKind::Dotted];

    pub fn key(self) -> &'static str {
        match self {
            DashKind::Solid => "solid",
            DashKind::Dashed => "dashed",
            DashKind::Dotted => "dotted",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            DashKind::Solid => "Solid",
            DashKind::Dashed => "Dashed",
            DashKind::Dotted => "Dotted",
        }
    }

    pub fn from_key(s: &str) -> Option<DashKind> {
        let s = s.trim().to_ascii_lowercase();
        DashKind::ALL.into_iter().find(|k| k.key() == s)
    }
}

fn length(v: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, MAX_DASH_LENGTH) } else { 0.0 }
}

impl Dash {
    /// Dashes of `on` with gaps of `off` (sanitized; `None` when that draws solid).
    pub fn dashed(on: f32, off: f32) -> Option<Dash> {
        Dash::Dashed { lengths: vec![on, off].into(), offset: 0.0 }.sanitized()
    }

    /// Dots `spacing` apart (sanitized).
    pub fn dotted(spacing: f32) -> Option<Dash> {
        Dash::Dotted { spacing }.sanitized()
    }

    /// The pattern as stored, cleaned up (a file, a command): lengths finite and in range, an
    /// odd count repeated, at most [`MAX_DASH_ENTRIES`], the offset within one period.
    /// `None` when it draws solid (no gaps, or nothing at all to repeat).
    pub fn sanitized(self) -> Option<Dash> {
        match self {
            Dash::Dashed { lengths, offset } => {
                let mut v: Vec<f32> = lengths.iter().take(MAX_DASH_ENTRIES).map(|&x| length(x)).collect();
                if v.len() % 2 == 1 {
                    let again = v.clone();
                    v.extend(again);
                    v.truncate(MAX_DASH_ENTRIES);
                }
                let period: f32 = v.iter().sum();
                let gaps: f32 = v.iter().skip(1).step_by(2).sum();
                if period.is_nan() || period <= MIN_PERIOD || gaps <= 0.0 {
                    return None;
                }
                let offset = if offset.is_finite() { offset.rem_euclid(period) } else { 0.0 };
                Some(Dash::Dashed { lengths: v.into(), offset })
            }
            Dash::Dotted { spacing } => Some(Dash::Dotted { spacing: length(spacing) }),
        }
    }

    pub fn kind(&self) -> DashKind {
        match self {
            Dash::Dashed { .. } => DashKind::Dashed,
            Dash::Dotted { .. } => DashKind::Dotted,
        }
    }

    /// The on/off lengths and the start offset for a stroke `width` wide (shape units). Dots are
    /// zero-length dashes drawn with round caps, one width plus the spacing apart.
    pub fn pattern(&self, width: f32) -> (Vec<f32>, f32) {
        match self {
            Dash::Dashed { lengths, offset } => (lengths.to_vec(), *offset),
            Dash::Dotted { spacing } => (vec![0.0, length(width) + spacing], 0.0),
        }
    }

    /// The same pattern `k` times as long (the shape scaled by `k`).
    pub fn scaled(&self, k: f32) -> Dash {
        let k = if k.is_finite() && k > 0.0 { k } else { 1.0 };
        match self {
            Dash::Dashed { lengths, offset } => Dash::Dashed { lengths: lengths.iter().map(|&x| length(x * k)).collect(), offset: offset * k },
            Dash::Dotted { spacing } => Dash::Dotted { spacing: length(spacing * k) },
        }
    }

    /// `t` of the way to `o` (shape tweens): lengths blend when both are the same kind with
    /// the same number of lengths; otherwise this pattern holds until the end.
    pub fn lerp(a: Option<&Dash>, b: Option<&Dash>, t: f32) -> Option<Dash> {
        let mix = |x: f32, y: f32| x + (y - x) * t;
        match (a, b) {
            (Some(Dash::Dashed { lengths: la, offset: oa }), Some(Dash::Dashed { lengths: lb, offset: ob })) if la.len() == lb.len() => {
                Dash::Dashed { lengths: la.iter().zip(lb.iter()).map(|(&x, &y)| mix(x, y)).collect(), offset: mix(*oa, *ob) }.sanitized()
            }
            (Some(Dash::Dotted { spacing: sa }), Some(Dash::Dotted { spacing: sb })) => Some(Dash::Dotted { spacing: mix(*sa, *sb) }),
            _ => if t >= 1.0 { b.cloned() } else { a.cloned() },
        }
    }
}

/// Caps, joins, miter limit and dashes: everything about a stroke's outline but its width.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StrokeStyle {
    pub cap: Cap,
    pub join: Join,
    /// For miter joins: the longest a corner's point may reach, as a multiple of the stroke
    /// width ([`MIN_MITER_LIMIT`]..=[`MAX_MITER_LIMIT`]); sharper corners are bevelled.
    pub miter_limit: f32,
    /// `None`: a solid line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dash: Option<Dash>,
}

impl Default for StrokeStyle {
    fn default() -> Self {
        StrokeStyle { cap: Cap::Round, join: Join::Round, miter_limit: DEFAULT_MITER_LIMIT, dash: None }
    }
}

impl StrokeStyle {
    /// Round caps and joins, solid: how strokes were before styles (written as nothing).
    pub fn is_default(&self) -> bool {
        *self == StrokeStyle::default()
    }

    /// Cleaned up for use (a file, a command): the miter limit in range, the dash sanitized.
    pub fn sanitized(self) -> StrokeStyle {
        let miter_limit = if self.miter_limit.is_finite() { self.miter_limit.clamp(MIN_MITER_LIMIT, MAX_MITER_LIMIT) } else { DEFAULT_MITER_LIMIT };
        StrokeStyle { miter_limit, dash: self.dash.and_then(Dash::sanitized), ..self }
    }

    pub fn dash_kind(&self) -> DashKind {
        self.dash.as_ref().map_or(DashKind::Solid, Dash::kind)
    }

    /// The cap the ends of each piece get: dots are always round.
    pub fn effective_cap(&self) -> Cap {
        if matches!(self.dash, Some(Dash::Dotted { .. })) { Cap::Round } else { self.cap }
    }

    /// The farthest the outline reaches from the path, as a multiple of the half-width: miter
    /// points reach the miter limit, square caps the corner of their square.
    pub fn reach_factor(&self) -> f32 {
        let join = if self.join == Join::Miter { self.miter_limit.clamp(MIN_MITER_LIMIT, MAX_MITER_LIMIT) } else { 1.0 };
        let cap = if self.effective_cap() == Cap::Square { std::f32::consts::SQRT_2 } else { 1.0 };
        join.max(cap)
    }

    /// The style of the shape scaled by `k`: dashes scale with it.
    pub fn scaled(&self, k: f32) -> StrokeStyle {
        StrokeStyle { dash: self.dash.as_ref().map(|d| d.scaled(k)), ..self.clone() }
    }

    /// `t` of the way to `o` (shape tweens): the miter limit and the dash lengths blend; the
    /// caps and joins of the start hold until the end keyframe.
    pub fn lerp(&self, o: &StrokeStyle, t: f32) -> StrokeStyle {
        let pick = if t >= 1.0 { o } else { self };
        StrokeStyle { cap: pick.cap, join: pick.join, miter_limit: self.miter_limit + (o.miter_limit - self.miter_limit) * t, dash: Dash::lerp(self.dash.as_ref(), o.dash.as_ref(), t) }.sanitized()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dash_patterns_are_sanitized() {
        // An odd count repeats (as in SVG); the offset wraps into one period.
        let d = Dash::Dashed { lengths: vec![4.0, 2.0, 1.0].into(), offset: 15.0 }.sanitized().unwrap();
        assert_eq!(d, Dash::Dashed { lengths: vec![4.0, 2.0, 1.0, 4.0, 2.0, 1.0].into(), offset: 1.0 });
        // No gaps, or nothing at all: solid.
        assert_eq!(Dash::dashed(5.0, 0.0), None);
        assert_eq!(Dash::dashed(0.0, 0.0), None);
        assert_eq!(Dash::Dashed { lengths: vec![].into(), offset: 0.0 }.sanitized(), None);
        // Junk is clamped, never kept.
        let d = Dash::Dashed { lengths: vec![f32::NAN, -3.0, 1e30, 2.0].into(), offset: f32::INFINITY }.sanitized().unwrap();
        assert_eq!(d, Dash::Dashed { lengths: vec![0.0, 0.0, MAX_DASH_LENGTH, 2.0].into(), offset: 0.0 });
        let long = Dash::Dashed { lengths: vec![1.0; 99].into(), offset: 0.0 }.sanitized().unwrap();
        assert!(matches!(long, Dash::Dashed { lengths, .. } if lengths.len() == MAX_DASH_ENTRIES));
        assert_eq!(Dash::dotted(f32::NAN), Some(Dash::Dotted { spacing: 0.0 }));
    }

    #[test]
    fn dots_are_round_zero_length_dashes() {
        let st = StrokeStyle { cap: Cap::Butt, dash: Dash::dotted(3.0), ..StrokeStyle::default() };
        assert_eq!(st.effective_cap(), Cap::Round);
        assert_eq!(st.dash.as_ref().unwrap().pattern(2.0), (vec![0.0, 5.0], 0.0));
    }

    #[test]
    fn style_files_round_trip_and_default_to_round() {
        assert!(StrokeStyle::default().is_default());
        let st = StrokeStyle { cap: Cap::Square, join: Join::Miter, miter_limit: 6.0, dash: Dash::dashed(6.0, 3.0) };
        let s = serde_json::to_string(&st).unwrap();
        assert_eq!(s, r#"{"cap":"square","join":"miter","miter_limit":6.0,"dash":{"kind":"dashed","lengths":[6.0,3.0],"offset":0.0}}"#);
        assert_eq!(serde_json::from_str::<StrokeStyle>(&s).unwrap(), st);
        // Missing fields take the defaults.
        assert_eq!(serde_json::from_str::<StrokeStyle>(r#"{"join":"bevel"}"#).unwrap(), StrokeStyle { join: Join::Bevel, ..StrokeStyle::default() });
        assert_eq!(Cap::from_key("none"), Some(Cap::Butt));
        assert_eq!(Join::from_key("Mitre"), Some(Join::Miter));
    }

    #[test]
    fn reach_covers_miters_and_square_caps() {
        assert_eq!(StrokeStyle::default().reach_factor(), 1.0);
        assert_eq!(StrokeStyle { join: Join::Miter, miter_limit: 5.0, ..StrokeStyle::default() }.reach_factor(), 5.0);
        assert!((StrokeStyle { cap: Cap::Square, ..StrokeStyle::default() }.reach_factor() - 2f32.sqrt()).abs() < 1e-6);
    }

    #[test]
    fn styles_blend_in_tweens() {
        let a = StrokeStyle { dash: Dash::dashed(2.0, 2.0), ..StrokeStyle::default() };
        let b = StrokeStyle { cap: Cap::Butt, miter_limit: 5.0, dash: Dash::dashed(6.0, 4.0), ..StrokeStyle::default() };
        let m = a.lerp(&b, 0.5);
        assert_eq!((m.cap, m.miter_limit, m.dash), (Cap::Round, 4.0, Dash::dashed(4.0, 3.0)));
        assert_eq!(a.lerp(&b, 1.0).cap, Cap::Butt);
        // Different kinds hold the start's until the end.
        let c = StrokeStyle { dash: Dash::dotted(1.0), ..StrokeStyle::default() };
        assert_eq!(a.lerp(&c, 0.9).dash, a.dash);
        assert_eq!(a.lerp(&c, 1.0).dash, c.dash);
    }
}
