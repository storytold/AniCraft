//! WebAssembly plug-ins (ADR 0017, `docs/plugins.md`): `plugin.install / list / run / remove /
//! reload`, and the host side of the ABI, which lets a running plug-in read the document and
//! run registry commands. A run is atomic and one undo step: the commands it makes run nested
//! (no history of their own) inside `plugin.run`, and any failure restores the document.

use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::{ElementId, ElementKind, Paint};
use anicraft_geom::Rect;
use anicraft_plugins::{Host, Manifest, ParamKind, Plugin, SelectionNeed};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c("plugin.install", "Install Plug-in…", "Commands", None, "{path: a .wasm plug-in, confirm? (when it replaces an installed plug-in's file)} validates and installs it; with a plug-ins folder it is copied there", false, install as crate::RunFn),
        c("plugin.list", "Manage Plug-ins…", "Commands", None, "{} lists the installed plug-ins (the app shows them in a window)", false, list),
        c("plugin.run", "Run Plug-in", "", None, "{id, params?: {…} (see plugin.list; missing ones take their defaults)} one undo step", true, run),
        c("plugin.remove", "Remove Plug-in", "", None, "{id, confirm? (when it deletes the plug-in's file from the plug-ins folder)}", false, remove),
        c("plugin.reload", "Reload Plug-ins", "Commands", None, "{folder? (makes it the plug-ins folder first)} re-reads every .wasm in the plug-ins folder", false, reload),
    ]
}

/// Most elements the `elements` request lists.
const MAX_ELEMENTS: usize = 10_000;

/// Whether a running plug-in may call command `id`, and if not, why. Plug-ins edit the
/// document; they never touch files, the clipboard, history or other plug-ins. A command in
/// a namespace this list doesn't know is refused until someone decides (a test checks that
/// every registered namespace is known).
pub fn plugin_may_call(id: &str) -> std::result::Result<(), &'static str> {
    const DOCUMENT: &[&str] = &["draw", "edit", "element", "symbol", "arrange", "modify", "library", "text", "scene", "path", "paint", "timeline", "layer", "align", "doc", "motion", "sound", "stroke", "tween", "actions", "instance", "camera", "bone", "warp", "script", "raw", "frames", "find", "explorer", "ease"];
    match id {
        "edit.undo" | "edit.redo" => Err("a plug-in run is one undo step; it can't undo or redo"),
        "edit.cut" | "edit.copy" | "edit.paste" | "edit.paste_in_place" | "timeline.cut_frames" | "timeline.copy_frames" | "timeline.paste_frames" | "frames.cut" | "frames.copy" | "frames.paste" | "ease.copy" | "ease.paste" => {
            Err("plug-ins don't use your clipboard")
        }
        _ => match id.split('.').next().unwrap_or("") {
            "file" => Err("plug-ins can't open, save, import, export or publish files"),
            // Reading the settings is harmless; changing them changes what Publish writes where.
            "publish" if id == "publish.settings_get" => Ok(()),
            "publish" => Err("plug-ins can't change what Publish writes"),
            "prefs" => Err("plug-ins can't read or change your preferences"),
            // Listing the brushes is harmless (and painting with one is `draw.brush`); the
            // user's own brushes are their preferences, and a brush set is a file.
            "brush" if id == "brush.preset_list" => Ok(()),
            "brush" => Err("plug-ins can't change your brushes or read and write brush sets"),
            // Filter lists are the document's (ADR 0091); copied filters are the artist's
            // clipboard, and the presets their preferences (listing and applying one is fine).
            "filter" if matches!(id, "filter.copy" | "filter.paste") => Err("plug-ins don't use your clipboard"),
            "filter" if matches!(id, "filter.preset_save" | "filter.preset_rename" | "filter.preset_delete" | "filter.preset_export" | "filter.preset_import") => Err("plug-ins can't change your filter presets or read and write preset files"),
            "filter" => Ok(()),
            // The document's swatches are document data; swatch files are files.
            "swatch" if matches!(id, "swatch.import" | "swatch.export") => Err("plug-ins can't read or write swatch files"),
            "swatch" => Ok(()),
            "plugin" => Err("plug-ins can't install, run or remove plug-ins"),
            "control" => Err("plug-ins can't play the movie"),
            // A scene's guides are the document's (ADR 0076); whether rulers, grid and guides
            // show and snap is how you work.
            "view" if matches!(id, "view.aids" | "view.guide_add" | "view.guide_move" | "view.guide_delete" | "view.guides_clear") => Ok(()),
            "view" => Err("plug-ins can't change how your stage shows and snaps"),
            ns if DOCUMENT.contains(&ns) => Ok(()),
            _ => Err("plug-ins can't call this command"),
        },
    }
}

/// The session as a plug-in sees it.
pub(crate) struct SessionHost<'a> {
    pub s: &'a mut Session,
}

impl Host for SessionHost<'_> {
    fn request(&mut self, op: &str, req: &Value) -> std::result::Result<Value, String> {
        match op {
            "command" => {
                let id = req.get("id").and_then(Value::as_str).ok_or("a command request needs an \"id\", e.g. {\"op\":\"command\",\"id\":\"draw.rect\",\"params\":{…}}")?;
                if crate::find_command(id).is_none() {
                    return Err(format!("unknown command '{id}'"));
                }
                plugin_may_call(id).map_err(|why| format!("'{id}': {why}"))?;
                let params = req.get("params").cloned().unwrap_or_else(|| json!({}));
                self.s.execute_nested(id, &params).map_err(|e| e.to_string())
            }
            "document" => Ok(document_view(self.s)),
            "elements" => elements_view(self.s, req),
            _ => Err(format!("unknown op '{op}' (ABI v1 has command, document, elements and log)")),
        }
    }
}

/// The compact document view (`{"op":"document"}`).
fn document_view(s: &Session) -> Value {
    let st = &s.doc.stage;
    let layers: Vec<Value> = s
        .timeline()
        .layers
        .iter()
        .map(|l| json!({"id": l.id.0, "name": l.name, "kind": format!("{:?}", l.kind).to_lowercase(), "visible": l.visible, "locked": l.locked, "outline": l.outline, "onion": l.onion, "height": u32::from(l.height()) * 100, "parent": l.parent.map(|p| p.0)}))
        .collect();
    json!({
        "abi": anicraft_plugins::ABI_VERSION,
        "stage": {"width": st.width, "height": st.height, "fps": st.fps, "background": st.background.to_hex()},
        "scene": s.scene,
        "scenes": s.doc.scenes.len(),
        "frame": s.frame,
        "frames": s.frame_count(),
        "layer": s.layer.0,
        "layers": layers,
        "selection": s.selection.id_list(),
    })
}

fn paint_json(p: &Paint) -> Value {
    match p {
        Paint::Solid(c) => json!(c.to_hex()),
        Paint::Linear { .. } => json!("linear-gradient"),
        Paint::Radial { .. } => json!("radial-gradient"),
        Paint::Bitmap { .. } => json!("bitmap"),
    }
}

/// The elements on the active layer at the playhead (`{"op":"elements", "selected"?, "paths"?}`).
fn elements_view(s: &Session, req: &Value) -> std::result::Result<Value, String> {
    let only_selected = req.get("selected").and_then(Value::as_bool).unwrap_or(false);
    let paths = req.get("paths").and_then(Value::as_bool).unwrap_or(false);
    // Stage bounds of every top-level element in one pass over the display list.
    let mut bounds: HashMap<ElementId, Rect> = HashMap::new();
    for item in s.display_list().items.iter().filter(|i| i.layer == s.layer) {
        let pad = item.stroke.as_ref().map_or(0.0, |st| st.width * 0.5);
        let r = item.path.bounds().inflate(pad).transformed(&item.transform);
        let e = bounds.entry(item.element).or_insert(Rect::EMPTY);
        *e = e.union(&r);
    }
    let mut out = Vec::new();
    for e in s.visible_elements().iter().filter(|e| !only_selected || s.selection.contains(s.layer, e.id)) {
        if out.len() >= MAX_ELEMENTS {
            break;
        }
        let b = bounds.get(&e.id).copied().unwrap_or(Rect::EMPTY);
        let mut v = json!({
            "id": e.id.0,
            "name": e.name,
            "selected": s.selection.contains(s.layer, e.id),
            "transform": e.transform.m,
            "bounds": if b.is_empty() { Value::Null } else { json!([b.min.x, b.min.y, b.max.x - b.min.x, b.max.y - b.min.y]) },
            "alpha": e.color.alpha(),
            "color_effect": crate::color_effect_json(&e.color),
        });
        match &e.kind {
            ElementKind::Shape(sh) => {
                v["kind"] = json!("shape");
                v["fill"] = sh.fill.as_ref().map_or(Value::Null, paint_json);
                v["stroke"] = sh.stroke.as_ref().map_or(Value::Null, |st| json!({"color": st.color.to_hex(), "width": st.width}));
                if paths {
                    v["path"] = json!(sh.path.to_svg());
                }
            }
            ElementKind::Symbol { symbol, .. } => {
                v["kind"] = json!("symbol");
                v["symbol"] = json!(symbol.0);
            }
            ElementKind::Text(t) => {
                v["kind"] = json!("text");
                v["text"] = json!(t.text);
            }
            ElementKind::Camera => v["kind"] = json!("camera"),
            ElementKind::Group { .. } => {
                v["kind"] = json!("group");
                v["members"] = json!(e.kind.group_members().len());
            }
        }
        out.push(v);
    }
    Ok(json!({"layer": s.layer.0, "frame": s.frame, "elements": out}))
}

fn param_json(name: &str, p: &anicraft_plugins::ParamSpec) -> Value {
    let mut v = match &p.kind {
        ParamKind::Number { min, max, default } => json!({"type": "number", "min": min, "max": max, "default": default}),
        ParamKind::Int { min, max, default } => json!({"type": "int", "min": min, "max": max, "default": default}),
        ParamKind::Bool { default } => json!({"type": "bool", "default": default}),
        ParamKind::Choice { options, default } => json!({"type": "choice", "options": options, "default": default}),
        ParamKind::Color { default } => json!({"type": "color", "default": default}),
    };
    v["name"] = json!(name);
    v["label"] = json!(p.label);
    v
}

/// A plug-in as `plugin.list` describes it.
pub fn describe(s: &Session, p: &Plugin) -> Value {
    let m: &Manifest = p.manifest();
    json!({
        "id": m.id,
        "name": m.name,
        "version": m.version,
        "author": m.author,
        "description": m.description,
        "kind": m.kind.key(),
        "entries": m.entries,
        "format": m.format.as_ref().map(|f| json!({"name": f.name, "extension": f.extension})),
        "eases": p.eases().iter().map(|c| json!({"id": c.id, "name": c.name})).collect::<Vec<_>>(),
        "selection": if m.selection == SelectionNeed::Required { "required" } else { "optional" },
        "params": m.params.iter().map(|(k, spec)| param_json(k, spec)).collect::<Vec<_>>(),
        "notation": m.params_notation(),
        "size": p.size(),
        "source": p.source().map(|s| s.display().to_string()),
        "in_folder": s.plugins.owns_file(p),
    })
}

fn plugin_err(e: anicraft_plugins::Error) -> EngineError {
    match e {
        anicraft_plugins::Error::Params(m) => EngineError::Param(m),
        e => EngineError::State(e.to_string()),
    }
}

fn install(s: &mut Session, p: &Value) -> Result<Value> {
    let path = PathBuf::from(str_req(p, "path")?);
    let inst = s.plugins.install_file(&path).map_err(plugin_err)?;
    Ok(json!({
        "installed": describe(s, &inst.plugin),
        "replaced": inst.replaced.map(|o| o.manifest().version.clone()),
        "copied_to": inst.copied_to.map(|d| d.display().to_string()),
    }))
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let plugins: Vec<Value> = s.plugins.list().map(|p| describe(s, p)).collect();
    Ok(json!({"plugins": plugins, "folder": s.plugins.folder().map(|f| f.display().to_string()), "abi": anicraft_plugins::ABI_VERSION}))
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let id = str_req(p, "id")?;
    let plugin = s.plugins.get(id).ok_or_else(|| EngineError::State(format!("no plug-in “{id}” is installed (see plugin.list)")))?;
    let m = plugin.manifest();
    match m.kind {
        anicraft_plugins::Kind::Command => {}
        anicraft_plugins::Kind::FrameScript => return Err(EngineError::State(format!("{} is a frame script: it runs while the movie plays, from a keyframe (Window › Actions)", m.name))),
        anicraft_plugins::Kind::Exporter => return Err(EngineError::State(format!("{} is an export format: use File › Export, or file.export_plugin {{\"format\": \"{}\", \"path\": …}}", m.name, m.id))),
        anicraft_plugins::Kind::Ease => return Err(EngineError::State(format!("{} adds eases: pick one in a tween's ease menu, or ease.set {{\"easing\": {{\"plugin\": \"{}\", \"ease\": …}}}}", m.name, m.id))),
    }
    if m.selection == SelectionNeed::Required && s.selection.is_empty() {
        return Err(EngineError::State(format!("{} works on the selection: select some artwork first", m.name)));
    }
    let params = m.resolve_params(p.get("params").unwrap_or(&Value::Null)).map_err(plugin_err)?;
    let report = plugin.run(&mut SessionHost { s }, &params).map_err(plugin_err)?;
    // The undo step is named after the plug-in ("Undo Star Field").
    s.step_label = Some(m.name.clone());
    Ok(json!({"plugin": m.id, "name": m.name, "params": params, "log": report.log, "log_dropped": report.log_dropped, "calls": report.calls, "fuel": report.fuel}))
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    let id = str_req(p, "id")?;
    let rm = s.plugins.remove(id).map_err(plugin_err)?;
    Ok(json!({"removed": rm.plugin.id(), "name": rm.plugin.manifest().name, "deleted": rm.deleted.map(|d| d.display().to_string())}))
}

fn reload(s: &mut Session, p: &Value) -> Result<Value> {
    if let Some(f) = opt_str(p, "folder")? {
        if f.trim().is_empty() {
            return Err(EngineError::Param("'folder' is empty".into()));
        }
        s.plugins.set_folder(Some(PathBuf::from(f)));
    }
    let rep = s.plugins.reload().map_err(plugin_err)?;
    Ok(json!({
        "folder": s.plugins.folder().map(|f| f.display().to_string()),
        "loaded": rep.loaded,
        "failed": rep.failed.iter().map(|(f, e)| json!({"file": f, "error": e})).collect::<Vec<_>>(),
        "installed": s.plugins.len(),
    }))
}
