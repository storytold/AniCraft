//! Typed, validating access to JSON command params. Every getter returns an error the caller
//! can show, never panics, and rejects non-finite numbers.

use crate::{EngineError, Result};
use anicraft_doc::{CustomEase, Document, EasePoint, Easing, ProfilePreset, Rgba, SampledEase, WidthPoint, WidthProfile};
use anicraft_geom::Vec2;
use std::sync::Arc;
use serde_json::{Value, json};

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

/// Easing from a preset name (`"Linear"`, `"QuadIn"`, `"ElasticOut"`…), `{"classic": n}`,
/// `{"bezier": [x1, y1, x2, y2]}` or `{"custom": …}` (a custom ease: `{"points": [{at, in,
/// out, smooth}, ..]}` as `ease.info` gives it, or the flat list `[[x, y], ..]` of 3n + 1
/// points XFL uses). Saved eases (`{"saved": name}`) need the document: [`opt_easing_in`].
pub fn opt_easing(p: &Value, key: &str) -> Result<Option<Easing>> {
    let Some(v) = p.get(key) else { return Ok(None) };
    if v.is_null() {
        return Ok(None);
    }
    if let Some(s) = v.as_str() {
        return Easing::from_name(s).map(Some).ok_or_else(|| bad(key, "a preset name: Linear, Smooth, or a family (Quad, Cubic, Quart, Quint, Sine, Back, Circ, Bounce, Elastic) with In, Out or InOut, e.g. BackOut"));
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
    if let Some(c) = v.get("custom") {
        return custom_ease(c).map(|c| Some(Easing::Custom(Arc::new(c)))).ok_or_else(|| bad(key, "{\"custom\": {\"points\": [{\"at\": {\"x\", \"y\"}, \"in\": {..}, \"out\": {..}, \"smooth\"}, ..]}} or {\"custom\": [[x, y], ..]} (3n + 1 points)"));
    }
    if let Some(t) = v.get("sampled") {
        return sampled_ease(t).map(|t| Some(Easing::Sampled(Arc::new(t)))).ok_or_else(|| bad(key, "{\"sampled\": {\"name\", \"plugin\", \"ease\", \"values\": [0, …, 1]}} (as ease.info gives a plug-in's ease)"));
    }
    Err(bad(key, "an easing name, {\"classic\": n}, {\"bezier\": [x1, y1, x2, y2]}, {\"custom\": …}, {\"saved\": name} or {\"plugin\": id, \"ease\": id}"))
}

/// Most values read from a sampled ease param before it is thinned to the cap.
const MAX_SAMPLED_PARAM: usize = 65_536;

fn sampled_ease(v: &Value) -> Option<SampledEase> {
    let o = v.as_object()?;
    let values = o.get("values")?.as_array()?;
    if values.len() > MAX_SAMPLED_PARAM {
        return None;
    }
    let values: Option<Vec<f32>> = values.iter().map(|x| x.as_f64().map(|f| f as f32)).collect();
    let s = |k: &str| o.get(k).and_then(Value::as_str).unwrap_or("");
    Some(SampledEase::new(s("name"), s("plugin"), s("ease"), values?))
}

/// Most points read from a custom ease param before it is thinned to the cap.
const MAX_EASE_PARAM: usize = 16_384;

fn custom_ease(v: &Value) -> Option<CustomEase> {
    match v {
        Value::Array(flat) if flat.len() <= MAX_EASE_PARAM * 3 => {
            let pts: Option<Vec<Vec2>> = flat.iter().map(|q| match q.as_array().map(Vec::as_slice) {
                Some([x, y]) => Some(Vec2::new(x.as_f64()? as f32, y.as_f64()? as f32)),
                _ => None,
            }).collect();
            CustomEase::from_flat(&pts?)
        }
        Value::Object(o) => {
            let pts = o.get("points")?.as_array()?;
            if pts.len() > MAX_EASE_PARAM {
                return None;
            }
            serde_json::from_value::<Vec<EasePoint>>(Value::Array(pts.clone())).ok().map(CustomEase::new)
        }
        _ => None,
    }
}

/// [`opt_easing`], `{"saved": name}` for an ease saved in the document (ADR 0081), and
/// `{"plugin": id, "ease": id}` for an installed ease plug-in's (ADR 0088; the tween keeps
/// its sampled curve).
pub fn opt_easing_in(s: &crate::Session, p: &Value, key: &str) -> Result<Option<Easing>> {
    if let Some(name) = p.get(key).and_then(|v| v.get("saved")) {
        let name = name.as_str().ok_or_else(|| bad(key, "{\"saved\": name}"))?;
        return saved_ease(&s.doc, name).map(Some);
    }
    if let Some(v) = p.get(key).filter(|v| v.get("plugin").is_some()) {
        let (Some(plugin), Some(ease)) = (v.get("plugin").and_then(Value::as_str), v.get("ease").and_then(Value::as_str)) else {
            return Err(bad(key, "{\"plugin\": an ease plug-in's id, \"ease\": one of its eases (see ease.list)}"));
        };
        return plugin_ease(s, plugin, ease).map(Some);
    }
    opt_easing(p, key)
}

/// Ease `ease` (its id, or its name in any case) of the installed ease plug-in `plugin`, as the
/// sampled curve a tween keeps.
pub fn plugin_ease(s: &crate::Session, plugin: &str, ease: &str) -> Result<Easing> {
    let p = s.plugins.get(plugin).filter(|p| p.manifest().kind == anicraft_plugins::Kind::Ease).ok_or_else(|| EngineError::State(format!("no ease plug-in “{plugin}” is installed (see ease.list)")))?;
    let want = ease.trim().to_lowercase();
    let c = p.eases().iter().find(|c| c.id == ease || c.name.to_lowercase() == want).ok_or_else(|| {
        let ids: Vec<&str> = p.eases().iter().map(|c| c.id.as_str()).collect();
        EngineError::State(format!("{} has no ease '{ease}' (it has {})", p.manifest().name, ids.join(", ")))
    })?;
    Ok(Easing::Sampled(Arc::new(SampledEase::new(&c.name, p.id(), &c.id, c.values.clone()))))
}

/// The ease saved as `name` (any case).
pub fn saved_ease(doc: &Document, name: &str) -> Result<Easing> {
    let want = name.trim().to_lowercase();
    doc.eases.iter().find(|e| e.name.to_lowercase() == want).map(|e| e.ease.clone()).ok_or_else(|| {
        let names: Vec<&str> = doc.eases.iter().map(|e| e.name.as_str()).take(20).collect();
        EngineError::State(if names.is_empty() { format!("no ease is saved as '{name}' (save one with ease.save)") } else { format!("no ease is saved as '{name}' (saved: {})", names.join(", ")) })
    })
}

/// An ease as commands take it back (see [`opt_easing`]): a name for presets, else its values.
pub fn easing_json(e: &Easing) -> Value {
    match e {
        Easing::Classic(s) => json!({"classic": s}),
        Easing::CubicBezier(a, b, c, d) => json!({"bezier": [a, b, c, d]}),
        Easing::Custom(c) => json!({"custom": serde_json::to_value(&**c).unwrap_or(Value::Null)}),
        Easing::Sampled(t) => json!({"sampled": serde_json::to_value(&**t).unwrap_or(Value::Null)}),
        named => named.name().map_or(Value::Null, Value::from),
    }
}

/// A width profile (ADR 0018): a preset name (`"uniform"`, `"taper_both"`, `"taper_start"`,
/// `"taper_end"`, `"bulge"`, `"pinch"`) or width points as `[[t, left, right], ..]` (or
/// `{t, left, right}` objects; factors of the nominal width, 1 = as set). `Some(None)` is
/// uniform; `None` when the param is absent.
pub fn opt_profile(p: &Value, key: &str) -> Result<Option<Option<WidthProfile>>> {
    let want = "a profile name (uniform, taper_both, taper_start, taper_end, bulge, pinch) or a list of [t, left, right] width points";
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(name)) => ProfilePreset::from_key(name).map(|pr| Some(pr.profile())).ok_or_else(|| bad(key, want)),
        Some(Value::Array(list)) => {
            if list.len() > anicraft_doc::MAX_WIDTH_POINTS {
                return Err(bad(key, &format!("at most {} width points", anicraft_doc::MAX_WIDTH_POINTS)));
            }
            let num = |v: Option<&Value>| v.and_then(Value::as_f64).filter(|f| f.is_finite()).map(|f| f as f32);
            let mut pts = Vec::with_capacity(list.len());
            for v in list {
                let (t, l, r) = match v {
                    Value::Array(a) => (num(a.first()), num(a.get(1)), num(a.get(2))),
                    Value::Object(_) => (num(v.get("t")), num(v.get("left")), num(v.get("right"))),
                    _ => (None, None, None),
                };
                let (Some(t), Some(l), Some(r)) = (t, l, r) else { return Err(bad(key, want)) };
                if !(0.0..=1.0).contains(&t) || !(0.0..=anicraft_doc::MAX_WIDTH_FACTOR).contains(&l) || !(0.0..=anicraft_doc::MAX_WIDTH_FACTOR).contains(&r) {
                    return Err(bad(key, &format!("width points with t in 0..=1 and widths in 0..={}", anicraft_doc::MAX_WIDTH_FACTOR)));
                }
                pts.push(WidthPoint::new(t, l, r));
            }
            Ok(Some(WidthProfile::new(pts)))
        }
        Some(_) => Err(bad(key, want)),
    }
}

/// What the stroke style params take (`stroke.set_style`, `element.set_stroke`, the drawing
/// commands, `paint.ink`).
pub const STROKE_STYLE_PARAMS: &str = "cap?: none|round|square, join?: miter|round|bevel, miter? (limit, 1..=100), style?: solid|dashed|dotted, dash? ([on, off, ..] lengths; implies dashed), dash_offset?, dot_spacing? (implies dotted)";

/// The stroke style params (ADR 0046) applied to `base`, for a stroke `width` wide (which sizes
/// a new dash pattern or dot spacing); `None` when none are given.
pub fn opt_stroke_style(p: &Value, base: &anicraft_doc::StrokeStyle, width: f32) -> Result<Option<anicraft_doc::StrokeStyle>> {
    use anicraft_doc::stroke_style::{MAX_DASH_ENTRIES, MAX_DASH_LENGTH, MAX_MITER_LIMIT, MIN_MITER_LIMIT};
    use anicraft_doc::{Cap, Dash, DashKind, Join};
    let keys = ["cap", "join", "miter", "style", "dash", "dash_offset", "dot_spacing"];
    if !keys.iter().any(|k| p.get(*k).is_some_and(|v| !v.is_null())) {
        return Ok(None);
    }
    let mut st = base.clone();
    if let Some(c) = opt_str(p, "cap")? {
        st.cap = Cap::from_key(c).ok_or_else(|| bad("cap", "none, round or square"))?;
    }
    if let Some(j) = opt_str(p, "join")? {
        st.join = Join::from_key(j).ok_or_else(|| bad("join", "miter, round or bevel"))?;
    }
    if let Some(m) = opt_f32(p, "miter")? {
        if !(MIN_MITER_LIMIT..=MAX_MITER_LIMIT).contains(&m) {
            return Err(bad("miter", &format!("a miter limit in {MIN_MITER_LIMIT}..={MAX_MITER_LIMIT}")));
        }
        st.miter_limit = m;
    }
    let len_ok = |v: f32| (0.0..=MAX_DASH_LENGTH).contains(&v);
    let dash = match p.get("dash") {
        None | Some(Value::Null) => None,
        Some(Value::Array(list)) => {
            if list.is_empty() || list.len() > MAX_DASH_ENTRIES {
                return Err(bad("dash", &format!("a list of 1 to {MAX_DASH_ENTRIES} lengths")));
            }
            let mut v = Vec::with_capacity(list.len());
            for x in list {
                match x.as_f64().filter(|f| f.is_finite()).map(|f| f as f32) {
                    Some(f) if len_ok(f) => v.push(f),
                    _ => return Err(bad("dash", &format!("lengths in 0..={MAX_DASH_LENGTH}"))),
                }
            }
            Some(v)
        }
        Some(_) => return Err(bad("dash", "a list of [on, off, ..] lengths")),
    };
    let offset = opt_f32(p, "dash_offset")?;
    if offset.is_some_and(|o| o.abs() > MAX_DASH_LENGTH) {
        return Err(bad("dash_offset", &format!("in -{MAX_DASH_LENGTH}..={MAX_DASH_LENGTH}")));
    }
    let spacing = opt_f32(p, "dot_spacing")?;
    if spacing.is_some_and(|s| !len_ok(s)) {
        return Err(bad("dot_spacing", &format!("in 0..={MAX_DASH_LENGTH}")));
    }
    let kind = match opt_str(p, "style")? {
        Some(k) => DashKind::from_key(k).ok_or_else(|| bad("style", "solid, dashed or dotted"))?,
        None if dash.is_some() || offset.is_some() => DashKind::Dashed,
        None if spacing.is_some() => DashKind::Dotted,
        None => st.dash_kind(),
    };
    // A new pattern is sized by the stroke: dashes three widths long with two-width gaps, dots
    // one width apart.
    let w = if width.is_finite() { width.clamp(0.0, MAX_DASH_LENGTH) } else { 1.0 };
    st.dash = match kind {
        DashKind::Solid => None,
        DashKind::Dashed => {
            let (lengths, old_offset) = match &st.dash {
                Some(Dash::Dashed { lengths, offset }) => (lengths.to_vec(), *offset),
                _ => (vec![(3.0 * w).max(2.0), (2.0 * w).max(2.0)], 0.0),
            };
            let lengths = dash.unwrap_or(lengths);
            let d = Dash::Dashed { lengths: lengths.into(), offset: offset.unwrap_or(old_offset) }.sanitized();
            Some(d.ok_or_else(|| bad("dash", "a pattern with a gap (some off length above 0)"))?)
        }
        DashKind::Dotted => {
            let old = match &st.dash {
                Some(Dash::Dotted { spacing }) => *spacing,
                _ => w.max(2.0),
            };
            Dash::dotted(spacing.unwrap_or(old))
        }
    };
    Ok(Some(st.sanitized()))
}
