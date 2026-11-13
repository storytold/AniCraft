//! The Swatches panel's commands (ADR 0078): the document's saved colours and gradients.
//!
//! Swatches live in the document (`Document::swatches`), so every change here is an undo step
//! and is saved with the file, as Animate keeps them per document. Applying a swatch is
//! `paint.set_fill` (fill) or `element.set_stroke` (stroke) underneath, so the selection and
//! the tool colour change exactly as they do from the Colour panel.
//!
//! A swatch set moves between documents and computers as a small JSON file
//! (`swatch.export` / `swatch.import`); GIMP palettes (`.gpl`, an open plain-text format)
//! import too. Adobe's `.aco`, `.ase` and `.clr` formats are proprietary and not read.

use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::swatch::{MAX_PALETTE_BYTES, clean_name, default_set, parse_gpl};
use anicraft_doc::{GradientStop, MAX_SWATCHES, Rgba, Spread, Swatch, SwatchPaint};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, params, mutates, run| CommandSpec { id, label, menu: "", shortcut: None, params, mutates, run };
    vec![
        c("swatch.list", "Swatches", "{} the document's swatches, in the Swatches panel's order → {swatches: [{index, name, type, color | stops, ..}], count}", false, list as crate::RunFn),
        c(
            "swatch.add",
            "Add Swatch",
            "{from?: fill|stroke (default fill: the fill swatch's colour or gradient; stroke: the stroke colour), or the swatch itself: type?: solid|linear|radial, color? (\"#rrggbb\" | \"#rrggbbaa\", solid), stops?: [{offset: 0..1, color}, ..] (2 to 15), spread?: extend|reflect|repeat, linear_rgb?: bool, focal?: -0.98..0.98 (radial); name?, at? (index; default: the end)} → {index, count}",
            true,
            add,
        ),
        c("swatch.remove", "Delete Swatch", "{index} → {count}", true, remove),
        c("swatch.move", "Move Swatch", "{from, to} (indices: the swatch at `from` ends up at `to`) → {index}", true, move_swatch),
        c("swatch.duplicate", "Duplicate Swatch", "{index} puts a copy right after it → {index, count}", true, duplicate),
        c("swatch.rename", "Rename Swatch", "{index, name} (empty: no name, the hex shows on hover)", true, rename),
        c(
            "swatch.apply",
            "Apply Swatch",
            "{index, target?: fill|stroke (default fill), selection?: bool (default true)} sets the fill swatch (as paint.set_fill: a gradient is fitted to each shape) or the stroke colour, and unless `selection` is false the selected shapes' fills or strokes. A gradient swatch can't be a stroke (strokes are one colour) → {target, changed}",
            true,
            apply,
        ),
        c("swatch.clear", "Clear Swatches", "{} removes every swatch but black and white (Animate's Clear Colors; undoable)", true, clear),
        c("swatch.reset", "Default Swatches", "{} replaces the swatches with a new document's set (undoable)", true, reset),
        c("swatch.import", "Import Swatches", "{path: a swatch file (.json, from swatch.export) or a GIMP palette (.gpl), replace?: bool (default false: added at the end)} → {imported, count}", true, import),
        c("swatch.export", "Export Swatches", "{path} writes the document's swatches to a swatch file (JSON) → {written, swatches}", false, export),
    ]
}

/// `#rrggbb`, or `#rrggbbaa` when not opaque.
pub fn hex(c: Rgba) -> String {
    let c = c.sanitized();
    if c.a >= 1.0 { c.to_hex() } else { format!("{}{:02x}", c.to_hex(), (c.a * 255.0).round() as u8) }
}

fn stops_json(stops: &[GradientStop]) -> Value {
    json!(stops.iter().map(|s| json!({"offset": s.offset, "color": hex(s.color)})).collect::<Vec<_>>())
}

/// A swatch as commands and swatch files write it.
pub fn swatch_json(s: &Swatch) -> Value {
    let mut v = match &s.paint {
        SwatchPaint::Solid { color } => json!({"type": "solid", "color": hex(*color)}),
        SwatchPaint::Linear { stops, spread, linear_rgb } => json!({"type": "linear", "stops": stops_json(stops), "spread": spread.key(), "linear_rgb": linear_rgb}),
        SwatchPaint::Radial { stops, spread, linear_rgb, focal } => json!({"type": "radial", "stops": stops_json(stops), "spread": spread.key(), "linear_rgb": linear_rgb, "focal": focal}),
    };
    if !s.name.is_empty()
        && let Some(o) = v.as_object_mut()
    {
        o.insert("name".into(), json!(s.name));
    }
    v
}

/// A swatch from its JSON (see `swatch.add`). `None` when `v` names no paint (no type, colour
/// or stops), so the caller can fall back to the current one.
pub fn swatch_from_json(v: &Value) -> Result<Option<Swatch>> {
    let kind = opt_str(v, "type")?;
    let color = opt_color(v, "color")?;
    let stops = crate::cmds_paint::opt_stops(v)?;
    let spread = match opt_str(v, "spread")? {
        None => Spread::Pad,
        Some(k) => Spread::from_key(k).ok_or_else(|| EngineError::Param(format!("unknown spread '{k}' (extend, reflect or repeat)")))?,
    };
    let linear_rgb = opt_bool(v, "linear_rgb")?.unwrap_or(false);
    let focal = crate::cmds_paint::opt_focal(v)?.unwrap_or(0.0);
    let kind = match (kind, &color, &stops) {
        (None, None, None) => return Ok(None),
        (Some(k), ..) => k,
        (None, _, Some(_)) => "linear",
        (None, Some(_), None) => "solid",
    };
    let need_stops = || stops.clone().ok_or_else(|| EngineError::Param(format!("a {kind} swatch needs 'stops' (2 to 15 like {{\"offset\": 0, \"color\": \"#ff0000\"}})")));
    let paint = match kind {
        "solid" => SwatchPaint::Solid { color: color.ok_or_else(|| EngineError::Param("a solid swatch needs 'color' (like \"#ff8800\")".into()))? },
        "linear" => SwatchPaint::Linear { stops: need_stops()?, spread, linear_rgb },
        "radial" => SwatchPaint::Radial { stops: need_stops()?, spread, linear_rgb, focal },
        other => return Err(EngineError::Param(format!("unknown swatch type '{other}' (solid, linear or radial)"))),
    };
    let name = opt_str(v, "name")?.unwrap_or("");
    Swatch::new(name, paint).sanitized().map(Some).ok_or_else(|| EngineError::Param("that swatch can't be kept (a gradient needs two stops)".into()))
}

fn index_param(s: &Session, p: &Value, key: &str) -> Result<usize> {
    let n = s.doc.swatches.len();
    let i = opt_u64(p, key)?.ok_or_else(|| EngineError::Param(format!("param '{key}' is required (a swatch's index)")))?;
    usize::try_from(i).ok().filter(|i| *i < n).ok_or_else(|| if n == 0 { EngineError::State("the document has no swatches".into()) } else { EngineError::Param(format!("param '{key}' must be a swatch index, 0 to {}", n - 1)) })
}

fn swatches_mut(s: &mut Session) -> &mut Vec<Swatch> {
    Arc::make_mut(&mut s.doc.swatches)
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let v: Vec<Value> = s
        .doc
        .swatches
        .iter()
        .enumerate()
        .map(|(i, sw)| {
            let mut j = swatch_json(sw);
            if let Some(o) = j.as_object_mut() {
                o.insert("index".into(), json!(i));
            }
            j
        })
        .collect();
    Ok(json!({"count": v.len(), "swatches": v}))
}

/// The swatch `swatch.add` makes from the current colours: the fill swatch, or the stroke.
fn current(s: &Session, from: &str) -> Result<Swatch> {
    match from {
        "fill" => SwatchPaint::from_paint(&s.style.fill_paint()).map(|p| Swatch { name: String::new(), paint: p }).ok_or_else(|| EngineError::State("a bitmap fill can't be a swatch (swatches hold colours and gradients)".into())),
        "stroke" => Ok(Swatch::solid(s.style.stroke)),
        other => Err(EngineError::Param(format!("param 'from' must be fill or stroke, not '{other}'"))),
    }
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    if s.doc.swatches.len() >= MAX_SWATCHES {
        return Err(EngineError::State(format!("a document keeps at most {MAX_SWATCHES} swatches; delete some first")));
    }
    let mut sw = match (opt_str(p, "from")?, swatch_from_json(p)?) {
        (Some(_), Some(_)) => return Err(EngineError::Param("give either 'from' or the swatch's own type, colour or stops, not both".into())),
        (None, Some(sw)) => sw,
        (from, None) => current(s, from.unwrap_or("fill"))?,
    };
    if let Some(n) = opt_str(p, "name")? {
        sw.name = clean_name(n);
    }
    let n = s.doc.swatches.len();
    let at = match opt_u64(p, "at")? {
        None => n,
        Some(i) => usize::try_from(i).ok().filter(|i| *i <= n).ok_or_else(|| EngineError::Param(format!("param 'at' must be 0 to {n}")))?,
    };
    swatches_mut(s).insert(at, sw);
    Ok(json!({"index": at, "count": n + 1}))
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    let i = index_param(s, p, "index")?;
    let v = swatches_mut(s);
    v.remove(i);
    Ok(json!({"count": v.len()}))
}

fn move_swatch(s: &mut Session, p: &Value) -> Result<Value> {
    let from = index_param(s, p, "from")?;
    let to = index_param(s, p, "to")?;
    if from != to {
        let v = swatches_mut(s);
        let sw = v.remove(from);
        v.insert(to, sw);
    }
    Ok(json!({"index": to}))
}

fn duplicate(s: &mut Session, p: &Value) -> Result<Value> {
    let i = index_param(s, p, "index")?;
    if s.doc.swatches.len() >= MAX_SWATCHES {
        return Err(EngineError::State(format!("a document keeps at most {MAX_SWATCHES} swatches; delete some first")));
    }
    let v = swatches_mut(s);
    let copy = v.get(i).cloned().ok_or_else(|| EngineError::Param("no such swatch".into()))?;
    v.insert(i + 1, copy);
    Ok(json!({"index": i + 1, "count": v.len()}))
}

fn rename(s: &mut Session, p: &Value) -> Result<Value> {
    let i = index_param(s, p, "index")?;
    let name = clean_name(str_req(p, "name")?);
    if let Some(sw) = swatches_mut(s).get_mut(i) {
        sw.name = name.clone();
    }
    Ok(json!({"index": i, "name": name}))
}

fn apply(s: &mut Session, p: &Value) -> Result<Value> {
    let i = index_param(s, p, "index")?;
    let sw = s.doc.swatches.get(i).cloned().ok_or_else(|| EngineError::Param("no such swatch".into()))?;
    let selection = opt_bool(p, "selection")?.unwrap_or(true);
    match opt_str(p, "target")?.unwrap_or("fill") {
        "fill" => {
            let mut fill = swatch_json(&sw);
            if let Some(o) = fill.as_object_mut() {
                o.remove("name");
                o.insert("selection".into(), json!(selection));
            }
            let r = crate::cmds_paint::set_fill(s, &fill)?;
            Ok(json!({"target": "fill", "changed": r.get("changed").cloned().unwrap_or(json!(0))}))
        }
        "stroke" => {
            let SwatchPaint::Solid { color } = sw.paint else {
                return Err(EngineError::State("strokes are one colour: apply a gradient swatch to the fill instead".into()));
            };
            s.style.stroke = color;
            s.style.use_stroke = true;
            let changed = if selection && !s.selection.is_empty() {
                s.each_layer(false, |s| crate::cmds_draw::set_stroke(s, &json!({"color": hex(color)})))?;
                s.selection.len()
            } else {
                0
            };
            Ok(json!({"target": "stroke", "changed": changed}))
        }
        other => Err(EngineError::Param(format!("param 'target' must be fill or stroke, not '{other}'"))),
    }
}

fn clear(s: &mut Session, _: &Value) -> Result<Value> {
    s.doc.swatches = Arc::new(vec![Swatch::solid(Rgba::BLACK), Swatch::solid(Rgba::WHITE)]);
    Ok(json!({"count": 2}))
}

fn reset(s: &mut Session, _: &Value) -> Result<Value> {
    s.doc.swatches = Arc::new(default_set());
    Ok(json!({"count": s.doc.swatches.len()}))
}

/// The JSON swatch file: `{"format": "anicraft-swatches", "version": 1, "swatches": [..]}`.
pub fn file_json(swatches: &[Swatch]) -> String {
    let v = json!({"format": FORMAT, "version": 1, "swatches": swatches.iter().map(swatch_json).collect::<Vec<_>>()});
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

const FORMAT: &str = "anicraft-swatches";

/// The swatches in a swatch file (JSON) or a GIMP palette, told apart by their content.
pub fn parse_file(text: &str) -> Result<Vec<Swatch>> {
    if text.len() > MAX_PALETTE_BYTES {
        return Err(EngineError::Param("the swatch file is too large".into()));
    }
    if text.trim_start_matches('\u{feff}').trim_start().starts_with("GIMP Palette") {
        return parse_gpl(text).map(|(_, v)| v).map_err(EngineError::Param);
    }
    let v: Value = serde_json::from_str(text).map_err(|e| EngineError::Param(format!("this isn't a swatch file or a GIMP palette ({e})")))?;
    if v.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err(EngineError::Param(format!("this JSON isn't a swatch file (its \"format\" must be \"{FORMAT}\")")));
    }
    let list = v.get("swatches").and_then(Value::as_array).ok_or_else(|| EngineError::Param("the swatch file has no \"swatches\" list".into()))?;
    let mut out = Vec::new();
    for item in list.iter().take(MAX_SWATCHES) {
        // A swatch this version can't read is skipped, so a newer file still opens.
        if let Ok(Some(sw)) = swatch_from_json(item) {
            out.push(sw);
        }
    }
    if out.is_empty() {
        return Err(EngineError::Param("the swatch file has no swatches in it".into()));
    }
    Ok(out)
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    let path = PathBuf::from(str_req(p, "path")?);
    if crate::vfs::len(&path)? > MAX_PALETTE_BYTES as u64 {
        return Err(EngineError::Param(format!("{} is too large to be a swatch file", path.display())));
    }
    let found = parse_file(&crate::vfs::read_to_string(&path)?)?;
    let replace = opt_bool(p, "replace")?.unwrap_or(false);
    let v = swatches_mut(s);
    if replace {
        v.clear();
    }
    let room = MAX_SWATCHES.saturating_sub(v.len());
    if room == 0 {
        return Err(EngineError::State(format!("a document keeps at most {MAX_SWATCHES} swatches; delete some first")));
    }
    let n = found.len().min(room);
    v.extend(found.into_iter().take(n));
    Ok(json!({"imported": n, "count": v.len()}))
}

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    let path = PathBuf::from(str_req(p, "path")?);
    if s.doc.swatches.is_empty() {
        return Err(EngineError::State("the document has no swatches to export".into()));
    }
    let text = file_json(&s.doc.swatches);
    crate::vfs::write(&path, text.as_bytes())?;
    Ok(json!({"written": path.display().to_string(), "swatches": s.doc.swatches.len(), "bytes": text.len()}))
}
