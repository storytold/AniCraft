//! Lenient, bounded access into the `.mohoproj` JSON tree.
//!
//! Moho always writes every key, but the reader takes nothing for granted: a missing key, a
//! wrong type, a NaN or a value out of range reads as absent (the caller's default), never as
//! a panic. The value helpers all take `Option<&Value>`, so `get(...)` results chain straight
//! in and chains like `get(x, "a").and_then(|y| get(y, "b"))` compose. Unknown keys are
//! ignored here and preserved whole in the import sidecar.

use anicraft_doc::Rgba;
use anicraft_geom::Vec2;
use serde_json::Value;

/// Largest document we parse (the engine's own file limit).
pub const MAX_JSON_BYTES: usize = 1024 * 1024 * 1024;

/// `v[key]`, when `v` is an object that has it.
pub fn get<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    v.as_object()?.get(key)
}

/// A finite number.
pub fn num(v: Option<&Value>) -> Option<f64> {
    v?.as_f64().filter(|x| x.is_finite())
}

pub fn integer(v: Option<&Value>) -> Option<i64> {
    v?.as_i64()
}

pub fn boolean(v: Option<&Value>) -> Option<bool> {
    v?.as_bool()
}

/// A string, cut to a sane length.
pub fn string(v: Option<&Value>) -> Option<String> {
    Some(v?.as_str()?.chars().take(4096).collect())
}

pub fn array(v: Option<&Value>) -> Option<&[Value]> {
    v?.as_array().map(Vec::as_slice)
}

/// A `{x, y}` object (finite).
pub fn vec2(v: Option<&Value>) -> Option<Vec2> {
    let o = v?.as_object()?;
    let x = num(o.get("x"))? as f32;
    let y = num(o.get("y"))? as f32;
    Some(Vec2::new(x, y))
}

/// A `{x, y, z}` object; `z` defaults to 0 so 2D-only callers need no fallback.
pub fn vec3(v: Option<&Value>) -> Option<(f32, f32, f32)> {
    let o = v?.as_object()?;
    let x = num(o.get("x"))? as f32;
    let y = num(o.get("y"))? as f32;
    let z = num(o.get("z")).unwrap_or(0.0) as f32;
    Some((x, y, z))
}

/// A style/shape colour: `{r, g, b, a}` floats in 0..1.
pub fn color01(v: Option<&Value>) -> Option<Rgba> {
    let o = v?.as_object()?;
    let c = |k: &str| num(o.get(k)).map(|x| x as f32);
    Some(Rgba::new(c("r")?, c("g")?, c("b")?, c("a")?).sanitized())
}

/// A project/text colour: `{r, g, b, a}` ints in 0..255.
pub fn color255(v: Option<&Value>) -> Option<Rgba> {
    let o = v?.as_object()?;
    let c = |k: &str| num(o.get(k)).map(|x| (x as f32 / 255.0).clamp(0.0, 1.0));
    Some(Rgba::new(c("r")?, c("g")?, c("b")?, c("a")?))
}
