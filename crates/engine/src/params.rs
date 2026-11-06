//! Typed, validating access to JSON command params. Every getter returns an error the caller
//! can show, never panics, and rejects non-finite numbers.

use crate::{EngineError, Result};
use anicraft_doc::{Easing, Rgba};
use serde_json::Value;

fn bad(key: &str, want: &str) -> EngineError {
    EngineError::Param(format!("param '{key}' must be {want}"))
}

pub fn opt_f32(p: &Value, key: &str) -> Result<Option<f32>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            let f = v.as_f64().ok_or_else(|| bad(key, "a number"))?;
            if !f.is_finite() || f.abs() > 1e7 {
                return Err(bad(key, "a finite number within ±10,000,000"));
            }
            Ok(Some(f as f32))
        }
    }
}

pub fn f32_req(p: &Value, key: &str) -> Result<f32> {
    opt_f32(p, key)?.ok_or_else(|| bad(key, "given (a number)"))
}

pub fn opt_u32(p: &Value, key: &str) -> Result<Option<u32>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            let n = v.as_u64().ok_or_else(|| bad(key, "a non-negative integer"))?;
            u32::try_from(n).map(Some).map_err(|_| bad(key, "an integer below 4,294,967,296"))
        }
    }
}

pub fn opt_u64(p: &Value, key: &str) -> Result<Option<u64>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().map(Some).ok_or_else(|| bad(key, "a non-negative integer")),
    }
}

pub fn opt_str<'a>(p: &'a Value, key: &str) -> Result<Option<&'a str>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_str().map(Some).ok_or_else(|| bad(key, "a string")),
    }
}

pub fn str_req<'a>(p: &'a Value, key: &str) -> Result<&'a str> {
    opt_str(p, key)?.ok_or_else(|| bad(key, "given (a string)"))
}

pub fn opt_bool(p: &Value, key: &str) -> Result<Option<bool>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_bool().map(Some).ok_or_else(|| bad(key, "true or false")),
    }
}

pub fn opt_color(p: &Value, key: &str) -> Result<Option<Rgba>> {
    match opt_str(p, key)? {
        None => Ok(None),
        Some(s) => Rgba::parse_hex(s).map(Some).ok_or_else(|| bad(key, "a colour like \"#ff8800\"")),
    }
}

pub fn opt_u64_list(p: &Value, key: &str) -> Result<Option<Vec<u64>>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(a)) => {
            if a.len() > 100_000 {
                return Err(bad(key, "a list of at most 100,000 ids"));
            }
            a.iter().map(|v| v.as_u64().ok_or_else(|| bad(key, "a list of integer ids"))).collect::<Result<Vec<_>>>().map(Some)
        }
        Some(_) => Err(bad(key, "a list of integer ids")),
    }
}

/// Easing from a preset name (`"Linear"`, `"QuadIn"`…), `{"classic": n}` or `{"bezier": [x1,y1,x2,y2]}`.
pub fn opt_easing(p: &Value, key: &str) -> Result<Option<Easing>> {
    let Some(v) = p.get(key) else { return Ok(None) };
    if v.is_null() {
        return Ok(None);
    }
    if let Some(s) = v.as_str() {
        let e = match s.to_ascii_lowercase().replace(['-', '_', ' '], "").as_str() {
            "linear" | "none" => Easing::Linear,
            "quadin" | "easein" => Easing::QuadIn,
            "quadout" | "easeout" => Easing::QuadOut,
            "quadinout" | "easeinout" => Easing::QuadInOut,
            "cubicin" => Easing::CubicIn,
            "cubicout" => Easing::CubicOut,
            "cubicinout" => Easing::CubicInOut,
            "smooth" => Easing::CubicBezier(0.25, 0.1, 0.25, 1.0),
            _ => return Err(bad(key, "one of Linear, QuadIn, QuadOut, QuadInOut, CubicIn, CubicOut, CubicInOut, Smooth")),
        };
        return Ok(Some(e));
    }
    if let Some(c) = v.get("classic") {
        let s = c.as_f64().filter(|f| f.is_finite()).ok_or_else(|| bad(key, "{\"classic\": -100..100}"))?;
        return Ok(Some(Easing::Classic((s as f32).clamp(-100.0, 100.0))));
    }
    if let Some(Value::Array(b)) = v.get("bezier") {
        let n: Vec<f32> = b.iter().filter_map(Value::as_f64).filter(|f| f.is_finite()).map(|f| f as f32).collect();
        if let [x1, y1, x2, y2] = n[..] {
            return Ok(Some(Easing::CubicBezier(x1.clamp(0.0, 1.0), y1.clamp(-5.0, 5.0), x2.clamp(0.0, 1.0), y2.clamp(-5.0, 5.0))));
        }
    }
    Err(bad(key, "an easing name, {\"classic\": n} or {\"bezier\": [x1, y1, x2, y2]}"))
}
