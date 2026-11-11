//! Layer view properties saved in the document (ADR 0061): each layer's onion skin toggle, its
//! colour (outlines and the timeline bar), View as Outlines, its row height, and Layer
//! Properties, which sets several of them (and the name, type, show and lock) as one step.
//!
//! Behaviour learned from Adobe's public help pages on layers and onion skinning and from
//! watching the program; nothing was copied.

use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::{Layer, LayerId, LayerKind, MAX_LAYER_HEIGHT};
use serde_json::{Value, json};
use std::sync::Arc;

/// Layer colours offered next to the colour picker (sRGB): those new layers take in turn,
/// then a few more, all bright enough to read as outlines on a dark or a light stage.
pub const LAYER_COLORS: [u32; 12] = [0x4fa3ff, 0xff6b4a, 0x66bb6a, 0xffd54a, 0xba68c8, 0x4dd0e1, 0xff8a65, 0xf06292, 0xaed581, 0x9575cd, 0xe0e0e0, 0x8d6e63];

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c(
            "layer.set_onion",
            "Onion Skin Layer",
            "",
            None,
            "{onion?: bool (default: the other way), solo?: bool, layer?} whether onion skin draws the layer's frames around the playhead. solo: only this layer is onion-skinned (every other layer off); solo again on the only layer on turns them all back on (Alt-click)",
            true,
            set_onion as crate::RunFn,
        ),
        c("layer.set_color", "Layer Colour", "", None, "{color: \"#rrggbb\", layer?} the layer's colour: its outlines in outline mode and its bar on the timeline", true, set_color),
        c("layer.set_height", "Layer Height", "", None, "{height: 100 | 200 | 300 (percent of a normal row), layer?} the layer's row height on the timeline", true, set_height),
        c(
            "layer.set_properties",
            "Layer Properties",
            "Modify/Timeline",
            None,
            "{layer?, name?, type?: normal|mask|masked|guide|guided, color?, outline?: bool (view as outlines), height?: 100|200|300, visible?, locked?, onion?} sets several at once, as one undo step; answers with the layer's properties",
            true,
            set_properties,
        ),
        c("layer.properties", "Get Layer Properties", "", None, "{layer?} the layer's name, type, colour, outline, height (percent), visible, locked and onion", false, |s, p| properties(s, layer_id(s, p)?)),
    ]
}

fn layer_id(s: &Session, p: &Value) -> Result<LayerId> {
    let id = opt_u64(p, "layer")?.map_or(s.layer, LayerId);
    s.timeline().layer(id).map(|_| id).ok_or_else(|| EngineError::State(format!("no layer with id {}", id.0)))
}

/// Whether a layer of this kind has frames onion skin can show (folders and the camera don't).
fn onion_able(l: &Layer) -> bool {
    !matches!(l.kind, LayerKind::Folder | LayerKind::Camera)
}

fn set_onion(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_id(s, p)?;
    let solo = opt_bool(p, "solo")?.unwrap_or(false);
    let on = opt_bool(p, "onion")?;
    let tl = crate::editing::edited_mut(&mut s.doc, s.scene, &s.editing);
    if solo {
        // Animate's Alt-click: this layer alone; again on the only one left on, every layer.
        let alone = tl.layers.iter().filter(|l| onion_able(l)).all(|l| l.onion == (l.id == id));
        for l in tl.layers.iter_mut().filter(|l| onion_able(l)) {
            let want = alone || l.id == id;
            if l.onion != want {
                Arc::make_mut(l).onion = want;
            }
        }
    } else {
        let l = tl.layer_mut(id).ok_or_else(|| EngineError::State(format!("no layer with id {}", id.0)))?;
        l.onion = on.unwrap_or(!l.onion);
    }
    let on: Vec<u64> = tl.layers.iter().filter(|l| onion_able(l) && l.onion).map(|l| l.id.0).collect();
    let onion = tl.layer(id).is_some_and(|l| l.onion);
    Ok(json!({"layer": id.0, "onion": onion, "onion_layers": on}))
}

fn set_color(s: &mut Session, p: &Value) -> Result<Value> {
    let color = opt_color(p, "color")?.ok_or_else(|| EngineError::Param("param 'color' (like \"#ff8800\") is required".into()))?;
    let lid = opt_u64(p, "layer")?.map(LayerId);
    let l = s.layer_mut(lid)?;
    // A layer colour is opaque: it draws hairlines and a bar.
    l.color = color.with_alpha(1.0);
    Ok(json!({"color": l.color.to_hex()}))
}

/// `height` in percent (100, 200 or 300) as rows.
fn height_param(p: &Value) -> Result<Option<u8>> {
    match opt_u32(p, "height")? {
        None => Ok(None),
        Some(h) if h > 0 && h % 100 == 0 && h / 100 <= u32::from(MAX_LAYER_HEIGHT) => Ok(u8::try_from(h / 100).ok()),
        Some(_) => Err(EngineError::Param("param 'height' must be 100, 200 or 300 (percent)".into())),
    }
}

fn set_height(s: &mut Session, p: &Value) -> Result<Value> {
    let rows = height_param(p)?.ok_or_else(|| EngineError::Param("param 'height' (100, 200 or 300) is required".into()))?;
    let lid = opt_u64(p, "layer")?.map(LayerId);
    let l = s.layer_mut(lid)?;
    l.height_rows = rows;
    Ok(json!({"height": u32::from(l.height()) * 100}))
}

fn set_properties(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_id(s, p)?;
    // Every param is read (and checked) before anything changes.
    let name: Option<String> = opt_str(p, "name")?.map(|n| n.chars().take(128).collect());
    if name.as_ref().is_some_and(|n| n.trim().is_empty()) {
        return Err(EngineError::Param("layer name must not be empty".into()));
    }
    let ty = opt_str(p, "type")?;
    let color = opt_color(p, "color")?;
    let outline = opt_bool(p, "outline")?;
    let height = height_param(p)?;
    let (visible, locked, onion) = (opt_bool(p, "visible")?, opt_bool(p, "locked")?, opt_bool(p, "onion")?);
    if let Some(ty) = ty {
        let now = crate::cmds_timeline::layer_type_json(s, id);
        if now.get("type").and_then(Value::as_str) != Some(ty) {
            crate::cmds_timeline::set_type(s, id, ty)?;
        }
    }
    let l = s.layer_mut(Some(id))?;
    if let Some(n) = name {
        l.name = n;
    }
    if let Some(c) = color {
        l.color = c.with_alpha(1.0);
    }
    if let Some(o) = outline {
        l.outline = o;
    }
    if let Some(h) = height {
        l.height_rows = h;
    }
    if let Some(v) = visible {
        l.visible = v;
    }
    if let Some(v) = locked {
        l.locked = v;
    }
    if let Some(v) = onion {
        l.onion = v;
    }
    properties(s, id)
}

fn properties(s: &Session, id: LayerId) -> Result<Value> {
    let l = s.timeline().layer(id).ok_or_else(|| EngineError::State(format!("no layer with id {}", id.0)))?;
    let ty = crate::cmds_timeline::layer_type_json(s, id).get("type").cloned().unwrap_or(Value::Null);
    Ok(json!({
        "layer": id.0,
        "name": l.name,
        "type": ty,
        "color": l.color.to_hex(),
        "outline": l.outline,
        "height": u32::from(l.height()) * 100,
        "visible": l.visible,
        "locked": l.locked,
        "onion": l.onion,
    }))
}

#[cfg(test)]
#[path = "cmds_layer_tests.rs"]
mod tests;
