//! The filter list of movie clip and button instances (ADR 0025, 0091): replacing it
//! (`element.set_filters`), reordering, switching filters on and off, removing them, copying
//! and pasting them between instances, and the user's filter presets.
//!
//! Every edit acts on each selected movie clip and button instance (graphics take no filters,
//! as in Animate) and is one undo step. A filter at an index acts where an instance has one;
//! the command fails only when it acts nowhere. Copied filters and presets live in the session,
//! outside the document and its undo history.

use crate::filter_library::{FilterLibrary, MAX_FILE_BYTES, MAX_FILTERS, MAX_PRESETS};
use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::{ElementKind, FilterEntry, SymbolId, SymbolKind};
use serde_json::{Value, json};

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, params, mutates, run| CommandSpec { id, label, menu: "", shortcut: None, params, mutates, run };
    vec![
        c("filter.move", "Move Filter", "{from, to} moves the filter at index `from` (0 is the first, applied first) to index `to` in the selected movie clip and button instances' lists; the order changes how they draw", true, (|s, p| s.each_layer(false, |s| move_filter(s, p))) as crate::RunFn),
        c("filter.enable", "Enable Filter", "{enabled: bool, index? (default: every filter)} switches a filter on or off; a filter switched off keeps its settings and draws nothing", true, |s, p| s.each_layer(false, |s| enable(s, p))),
        c("filter.remove", "Remove Filter", "{index? (default: every filter)} removes a filter from the selected movie clip and button instances", true, |s, p| s.each_layer(false, |s| remove(s, p))),
        c("filter.copy", "Copy Filters", "{index? (default: every filter)} copies the selected instance's filters (one, or all), for Paste Filters", false, copy),
        c("filter.paste", "Paste Filters", "{replace? (default false: added after the instances' own)} pastes the copied filters onto the selected movie clip and button instances", true, |s, p| s.each_layer(false, |s| paste(s, p))),
        c("filter.preset_list", "Filter Presets", "{} your filter presets, each a name and its filters", false, preset_list),
        c("filter.preset_save", "Save Filter Preset", "{name, filters? (a list as element.set_filters takes; default: the selected instance's)} saves a filter preset of your own (kept in your preferences; a name you have already is replaced, after asking)", false, preset_save),
        c("filter.preset_apply", "Apply Filter Preset", "{name} gives the selected movie clip and button instances the preset's filters, replacing theirs (as Animate does)", true, |s, p| s.each_layer(false, |s| preset_apply(s, p))),
        c("filter.preset_rename", "Rename Filter Preset", "{name, to}", false, preset_rename),
        c("filter.preset_delete", "Delete Filter Preset", "{name} deletes a filter preset (asks first: it can't be undone; artwork that used it keeps its filters)", false, preset_delete),
        c("filter.preset_export", "Export Filter Presets", "{path, names? (default: all)} writes your filter presets to a small JSON file", false, preset_export),
        c("filter.preset_import", "Import Filter Presets", "{path} adds the presets in a filter preset file to yours (a name you have already gets a number)", false, preset_import),
    ]
}

/// A filter list from command params (`element.set_filters`, `filter.preset_save`): colours may
/// be "#rrggbb" or "#rrggbbaa", values are brought into range, at most [`MAX_FILTERS`].
pub(crate) fn parse_list(list: Option<&Value>) -> Result<Vec<FilterEntry>> {
    let mut list = list.cloned().ok_or_else(|| EngineError::Param("param 'filters' is required (a list; [] removes them)".into()))?;
    fn hex_colors(v: &mut Value, keys: &[&str]) -> Result<()> {
        for k in keys {
            if v.get(*k).is_some_and(Value::is_string) {
                let c = opt_color(v, k)?.unwrap_or(anicraft_doc::Rgba::BLACK);
                v[*k] = json!({"r": c.r, "g": c.g, "b": c.b, "a": c.a});
            }
        }
        Ok(())
    }
    if list.as_array().is_some_and(|a| a.len() > MAX_FILTERS) {
        return Err(EngineError::Param(format!("at most {MAX_FILTERS} filters per instance")));
    }
    for f in list.as_array_mut().into_iter().flatten() {
        hex_colors(f, &["color", "shadow_color", "highlight_color"])?;
        if let Some(stops) = f.get_mut("stops").and_then(Value::as_array_mut) {
            if stops.len() > anicraft_doc::MAX_RAMP_STOPS {
                return Err(EngineError::Param(format!("a ramp has at most {} stops", anicraft_doc::MAX_RAMP_STOPS)));
            }
            for st in stops.iter_mut() {
                hex_colors(st, &["color"])?;
            }
        }
    }
    let filters: Vec<FilterEntry> = serde_json::from_value(list).map_err(|e| EngineError::Param(format!("'filters' must be a list of {{type: blur|drop_shadow|glow|bevel|gradient_glow|gradient_bevel|adjust_color, …values, enabled?: bool}}: {e}")))?;
    Ok(filters.iter().map(FilterEntry::sanitized).collect())
}

/// Runs `edit` on the filter list of each selected movie clip and button instance; `edit`
/// says whether it changed that list. Fails when no instance takes filters, or none changed
/// (`nothing` says why).
fn edit_lists(s: &mut Session, nothing: &str, mut edit: impl FnMut(&mut Vec<FilterEntry>) -> bool) -> Result<usize> {
    let kinds: std::collections::HashMap<SymbolId, SymbolKind> = s.doc.library.iter().map(|x| (x.id, x.kind)).collect();
    let (mut set, mut clips, mut graphics) = (0, 0, 0);
    for e in s.selected_props_mut()? {
        if let ElementKind::Symbol { symbol, filters, .. } = &mut e.kind {
            if kinds.get(symbol) == Some(&SymbolKind::Graphic) {
                graphics += 1;
                continue;
            }
            clips += 1;
            if edit(filters) {
                set += 1;
            }
        }
    }
    if clips == 0 {
        return Err(EngineError::State(if graphics > 0 {
            "filters apply to movie clip and button instances, as in Animate; set the symbol's Type to Movie Clip first".into()
        } else {
            "select a movie clip or button instance".into()
        }));
    }
    if set == 0 {
        return Err(EngineError::State(nothing.into()));
    }
    Ok(set)
}

/// Filters for the selected movie clip and button instances (Animate offers them on those only).
pub(crate) fn set_filters(s: &mut Session, p: &Value) -> Result<Value> {
    let filters = parse_list(p.get("filters"))?;
    let set = edit_lists(s, "", |f| {
        f.clone_from(&filters);
        true
    })?;
    Ok(json!({"filters": filters.len(), "set": set}))
}

fn index(p: &Value, key: &str) -> Result<Option<usize>> {
    Ok(opt_u32(p, key)?.map(|i| i as usize))
}

fn index_req(p: &Value, key: &str) -> Result<usize> {
    index(p, key)?.ok_or_else(|| EngineError::Param(format!("param '{key}' is required (a filter's index, 0 is the first)")))
}

fn no_filter(i: usize) -> String {
    format!("the selection has no filter {} (0 is the first)", i)
}

fn move_filter(s: &mut Session, p: &Value) -> Result<Value> {
    let (from, to) = (index_req(p, "from")?, index_req(p, "to")?);
    let set = edit_lists(s, &no_filter(from), |f| {
        if from >= f.len() {
            return false;
        }
        let to = to.min(f.len() - 1);
        if from == to {
            return true;
        }
        let x = f.remove(from);
        f.insert(to, x);
        true
    })?;
    Ok(json!({"moved": set}))
}

fn enable(s: &mut Session, p: &Value) -> Result<Value> {
    let on = opt_bool(p, "enabled")?.ok_or_else(|| EngineError::Param("param 'enabled' is required (true or false)".into()))?;
    let at = index(p, "index")?;
    let nothing = at.map_or_else(|| "the selection has no filters".to_string(), no_filter);
    let set = edit_lists(s, &nothing, |f| match at {
        Some(i) => f.get_mut(i).map(|e| e.enabled = on).is_some(),
        None => {
            f.iter_mut().for_each(|e| e.enabled = on);
            !f.is_empty()
        }
    })?;
    Ok(json!({"enabled": on, "set": set}))
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    let at = index(p, "index")?;
    let nothing = at.map_or_else(|| "the selection has no filters".to_string(), no_filter);
    let set = edit_lists(s, &nothing, |f| match at {
        Some(i) if i < f.len() => {
            f.remove(i);
            true
        }
        Some(_) => false,
        None => {
            let any = !f.is_empty();
            f.clear();
            any
        }
    })?;
    Ok(json!({"removed": set}))
}

/// The filters of the first selected movie clip or button instance, as it shows now.
fn selected_list(s: &Session) -> Result<Vec<FilterEntry>> {
    let kinds: std::collections::HashMap<SymbolId, SymbolKind> = s.doc.library.iter().map(|x| (x.id, x.kind)).collect();
    s.selected_elements()
        .into_iter()
        .find_map(|(_, e)| match &e.kind {
            ElementKind::Symbol { symbol, filters, .. } if kinds.get(symbol).is_some_and(|k| *k != SymbolKind::Graphic) => Some(filters.clone()),
            _ => None,
        })
        .ok_or_else(|| EngineError::State("select a movie clip or button instance".into()))
}

fn copy(s: &mut Session, p: &Value) -> Result<Value> {
    let list = selected_list(s)?;
    let chosen = match index(p, "index")? {
        Some(i) => vec![list.get(i).cloned().ok_or_else(|| EngineError::Param(no_filter(i)))?],
        None if list.is_empty() => return Err(EngineError::State("the instance has no filters to copy".into())),
        None => list,
    };
    let n = chosen.len();
    s.filter_clipboard = Some(chosen);
    Ok(json!({"copied": n}))
}

/// `add` after each list (or in place of it with `replace`), refused past [`MAX_FILTERS`].
fn put(s: &mut Session, add: &[FilterEntry], replace: bool) -> Result<usize> {
    let mut full = false;
    let set = edit_lists(s, "", |f| {
        if replace {
            *f = add.to_vec();
            return true;
        }
        if f.len() + add.len() > MAX_FILTERS {
            full = true;
            return false;
        }
        f.extend(add.iter().cloned());
        true
    });
    match set {
        Err(_) if full => Err(EngineError::State(format!("an instance has at most {MAX_FILTERS} filters; remove some first"))),
        other => other,
    }
}

fn paste(s: &mut Session, p: &Value) -> Result<Value> {
    let add = s.filter_clipboard.clone().ok_or_else(|| EngineError::State("no filters are copied yet (Copy Filters first)".into()))?;
    let replace = opt_bool(p, "replace")?.unwrap_or(false);
    let set = put(s, &add, replace)?;
    Ok(json!({"pasted": add.len(), "set": set}))
}

fn preset_json(p: &crate::filter_library::FilterPreset) -> Value {
    json!({"name": p.name, "filters": p.filters})
}

fn preset_list(s: &mut Session, _: &Value) -> Result<Value> {
    Ok(json!({"presets": s.filter_presets.presets().iter().map(preset_json).collect::<Vec<_>>()}))
}

fn preset_save(s: &mut Session, p: &Value) -> Result<Value> {
    let name = str_req(p, "name")?;
    let filters = match p.get("filters") {
        None | Some(Value::Null) => selected_list(s)?,
        list => parse_list(list)?,
    };
    let (name, replaced) = s.filter_presets.save(name, &filters)?;
    Ok(json!({"name": name, "replaced": replaced, "filters": filters.len(), "presets": s.filter_presets.presets().len()}))
}

fn preset_apply(s: &mut Session, p: &Value) -> Result<Value> {
    let name = str_req(p, "name")?;
    let preset = s.filter_presets.find(name).cloned().ok_or_else(|| EngineError::Param(format!("you have no filter preset named '{}'", name.chars().take(64).collect::<String>())))?;
    let set = put(s, &preset.filters, true)?;
    Ok(json!({"name": preset.name, "filters": preset.filters.len(), "set": set}))
}

fn preset_rename(s: &mut Session, p: &Value) -> Result<Value> {
    let to = s.filter_presets.rename(str_req(p, "name")?, str_req(p, "to")?)?;
    Ok(json!({"name": to}))
}

fn preset_delete(s: &mut Session, p: &Value) -> Result<Value> {
    let gone = s.filter_presets.delete(str_req(p, "name")?)?;
    Ok(json!({"deleted": gone.name, "presets": s.filter_presets.presets().len()}))
}

fn preset_export(s: &mut Session, p: &Value) -> Result<Value> {
    let path = std::path::PathBuf::from(str_req(p, "path")?);
    let bad = || EngineError::Param("param 'names' must be a list of filter preset names".into());
    let chosen: Vec<_> = match p.get("names") {
        None | Some(Value::Null) => s.filter_presets.presets().to_vec(),
        Some(Value::Array(names)) => names
            .iter()
            .take(MAX_PRESETS * 2)
            .map(|n| {
                let n = n.as_str().ok_or_else(bad)?;
                s.filter_presets.find(n).cloned().ok_or_else(|| EngineError::Param(format!("you have no filter preset named '{}'", n.chars().take(64).collect::<String>())))
            })
            .collect::<Result<_>>()?,
        Some(_) => return Err(bad()),
    };
    if chosen.is_empty() {
        return Err(EngineError::State("there are no filter presets to export yet (save one first)".into()));
    }
    let text = FilterLibrary::file_json(&chosen);
    crate::vfs::write(&path, text.as_bytes())?;
    Ok(json!({"written": path.display().to_string(), "presets": chosen.len(), "bytes": text.len()}))
}

fn preset_import(s: &mut Session, p: &Value) -> Result<Value> {
    let path = std::path::PathBuf::from(str_req(p, "path")?);
    if crate::vfs::len(&path)? > MAX_FILE_BYTES {
        return Err(EngineError::Param(format!("{} is too large to be a filter preset file", path.display())));
    }
    let found = FilterLibrary::parse_file(&crate::vfs::read_to_string(&path)?)?;
    if found.is_empty() {
        return Err(EngineError::Param("the file has no filter presets in it".into()));
    }
    let mut names = Vec::with_capacity(found.len());
    for f in found {
        match s.filter_presets.add_unique(f) {
            Ok(n) => names.push(n),
            Err(e) if names.is_empty() => return Err(e),
            // The library filled up part way: say what came in.
            Err(_) => break,
        }
    }
    Ok(json!({"imported": names, "presets": s.filter_presets.presets().len()}))
}
