//! Moho channel objects: `{type, ref, mute, when[], val[], interp[]}` parallel arrays, the
//! carrier of every animatable value in a `.mohoproj`.
//!
//! Stage 1 is a static import: each channel is read at the document's start frame (Moho frames
//! are 1-based). A channel with more than one key is animation the document model can't hold
//! yet, so the importer reports it and the full channel travels in the sidecar.

use crate::json::{self, array, num};
use anicraft_doc::Rgba;
use anicraft_geom::Vec2;
use serde_json::Value;

/// `arr[i]`, when `i` is in range.
fn at(arr: &[Value], i: usize) -> Option<&Value> {
    arr.get(i)
}

/// A read-only view of one channel object.
#[derive(Clone, Copy, Debug)]
pub struct Channel<'a> {
    when: &'a [Value],
    val: &'a [Value],
    interp: &'a [Value],
}

impl<'a> Channel<'a> {
    /// Parses a channel object; `None` when `v` isn't one (missing or malformed `when`/`val`).
    pub fn parse(v: &'a Value) -> Option<Channel<'a>> {
        let when = array(json::get(v, "when"))?;
        let val = array(json::get(v, "val"))?;
        if when.is_empty() || when.len() != val.len() || when.len() > 100_000 {
            return None;
        }
        let interp = array(json::get(v, "interp")).unwrap_or(&[]);
        Some(Channel { when, val, interp })
    }

    /// The channel's key count.
    pub fn len(&self) -> usize {
        self.when.len()
    }

    /// Whether the channel has no keys (it never does after a successful parse).
    pub fn is_empty(&self) -> bool {
        self.when.is_empty()
    }

    /// Key `i`'s frame, as Moho stores it (0-based; the corpus's −1000000 "always" sentinel
    /// sorts first).
    pub fn frame(&self, i: usize) -> Option<u32> {
        let f = num(Some(self.when.get(i)?))?;
        if f < 0.0 {
            // Moho's −1000000 "always" sentinel: before the first frame.
            return Some(0);
        }
        u32::try_from(f as i64).ok()
    }

    /// The channel's `interp[]` entries, one per key (may be shorter).
    pub fn interp(&self) -> Option<&'a [Value]> {
        Some(self.interp)
    }

    /// The channel's raw values.
    pub fn val(&self) -> &'a [Value] {
        self.val
    }

    /// Whether the channel holds animation (more than one key): stage 1 draws its first state
    /// and reports the rest.
    pub fn is_animated(&self) -> bool {
        self.when.len() > 1
    }

    /// The channel's value at Moho frame `frame` (1-based): the last key at or before it, or
    /// the first key when all of them lie after it (sentinels like `when: -1000000` sort
    /// first, so an ordinary frame picks the real key).
    pub fn sample(&self, frame: f64) -> Option<&'a Value> {
        let mut picked = at(self.val, 0)?;
        for (i, k) in self.when.iter().enumerate() {
            let Some(t) = num(Some(k)) else { continue };
            if t <= frame {
                picked = at(self.val, i).unwrap_or(picked);
            }
        }
        Some(picked)
    }
}

/// Numbers: `sample_f64(ch, frame).unwrap_or(default)` is the usual call site.
pub fn sample_f64(ch: &Channel, frame: f64) -> Option<f64> {
    num(ch.sample(frame))
}

/// The raw value at key `i` (no interpolation): the importer's per-key conversions.
pub fn sample_f64_at(ch: &Channel, i: usize) -> Option<f64> {
    num(ch.val().get(i))
}

pub fn sample_bool_at(ch: &Channel, i: usize) -> Option<bool> {
    ch.val().get(i)?.as_bool()
}

pub fn sample_vec2_at(ch: &Channel, i: usize) -> Option<Vec2> {
    json::vec2(ch.val().get(i))
}

pub fn sample_vec3_at(ch: &Channel, i: usize) -> Option<(f32, f32, f32)> {
    json::vec3(ch.val().get(i))
}

pub fn sample_bool(ch: &Channel, frame: f64) -> Option<bool> {
    ch.sample(frame)?.as_bool()
}

pub fn sample_vec2(ch: &Channel, frame: f64) -> Option<Vec2> {
    json::vec2(ch.sample(frame))
}

pub fn sample_vec3(ch: &Channel, frame: f64) -> Option<(f32, f32, f32)> {
    json::vec3(ch.sample(frame))
}

/// Style colours are 0..1 floats.
pub fn sample_color01(ch: &Channel, frame: f64) -> Option<Rgba> {
    json::color01(ch.sample(frame))
}

pub fn sample_string(ch: &Channel, frame: f64) -> Option<String> {
    json::string(ch.sample(frame))
}

/// The raw string at key `i` (no interpolation): marker labels.
pub fn sample_string_at(ch: &Channel, i: usize) -> Option<String> {
    json::string(ch.val().get(i))
}
