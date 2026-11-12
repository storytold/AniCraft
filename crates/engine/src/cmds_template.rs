//! Templates (ADR 0065): new documents from the templates AniCraft ships with
//! (`anicraft_doc::templates`, built by code) and from the user's own, which File › Save as
//! Template keeps as plain `.anicraft` files with a picture beside them:
//! `<folder>/<category>/<name>.anicraft` and `<name>.png`. The shell passes the folder (its
//! settings folder's `Templates`); the engine never looks for it, as with recent files.

use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session, format};
use anicraft_doc::templates::{self, TEMPLATES};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Longest template or category name, in characters.
pub const MAX_NAME_CHARS: usize = 64;
/// Most categories and templates listed (a folder with more is not one we made).
const MAX_CATEGORIES: usize = 64;
const MAX_TEMPLATES: usize = 500;
/// The category a template is saved in when none is given.
pub const DEFAULT_CATEGORY: &str = "My Templates";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "file.new_from_template",
            label: "New from Template…",
            menu: "File",
            shortcut: None,
            params: "{template? (a built-in template's id, from file.templates) | path? (a template file: any .anicraft opens as a new, untitled document)}",
            mutates: false,
            run: new_from_template,
        },
        CommandSpec {
            id: "file.templates",
            label: "List Templates",
            menu: "",
            shortcut: None,
            params: "{folder? (the user templates folder)} → {builtin: [{id, name, category, description, width, height, fps, frames}], user: [{name, category, path, thumbnail}]}",
            mutates: false,
            run: list,
        },
        CommandSpec {
            id: "file.save_as_template",
            label: "Save as Template…",
            menu: "File",
            shortcut: None,
            params: "{folder (the user templates folder), name, category? (default “My Templates”), confirm? (to replace a template of that name)} → {path, thumbnail}: the document as it is, kept as <folder>/<category>/<name>.anicraft with a picture of its first frame; the open document is unchanged",
            mutates: false,
            run: save_as_template,
        },
        CommandSpec { id: "file.delete_template", label: "Delete Template", menu: "", shortcut: None, params: "{folder, name, category?, confirm}: removes a user template and its picture (cannot be undone)", mutates: false, run: delete_template },
    ]
}

/// A name usable as a file or folder name on every system, or why not.
fn clean_name(raw: &str, what: &str) -> Result<String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(EngineError::Param(format!("give the {what} a name")));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(EngineError::Param(format!("a {what} name has at most {MAX_NAME_CHARS} characters")));
    }
    if name.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')) {
        return Err(EngineError::Param(format!("a {what} name can't use / \\ : * ? \" < > |")));
    }
    if name.starts_with('.') || name.ends_with('.') {
        return Err(EngineError::Param(format!("a {what} name can't start or end with a dot")));
    }
    // Names Windows keeps for devices, with or without an extension.
    let stem = name.split('.').next().unwrap_or(name).trim().to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") || ((stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4 && stem.ends_with(|c: char| c.is_ascii_digit()));
    if reserved {
        return Err(EngineError::Param(format!("“{name}” is a name the system keeps for itself: pick another {what} name")));
    }
    Ok(name.to_string())
}

/// Where a user template of this category and name is kept.
pub fn template_path(folder: &Path, category: &str, name: &str) -> Result<PathBuf> {
    let category = clean_name(category, "category")?;
    let name = clean_name(name, "template")?;
    Ok(folder.join(category).join(format!("{name}.anicraft")))
}

/// The picture kept beside a template file.
pub fn thumbnail_path(template: &Path) -> PathBuf {
    template.with_extension("png")
}

/// A user template found in the folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserTemplate {
    pub name: String,
    pub category: String,
    pub path: PathBuf,
    /// Its picture, when there is one.
    pub thumbnail: Option<PathBuf>,
}

/// The user's templates, by category then name (ignoring case).
pub fn user_templates(folder: &Path) -> Vec<UserTemplate> {
    let mut out = Vec::new();
    for dir in crate::vfs::list(folder, MAX_CATEGORIES).into_iter().filter(|d| crate::vfs::is_dir(d)) {
        let Some(category) = dir.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
        for f in crate::vfs::list(&dir, MAX_TEMPLATES) {
            if out.len() >= MAX_TEMPLATES {
                break;
            }
            let is_template = f.extension().is_some_and(|e| e.eq_ignore_ascii_case("anicraft")) && crate::vfs::is_file(&f);
            let Some(name) = f.file_stem().map(|n| n.to_string_lossy().into_owned()).filter(|_| is_template) else { continue };
            let thumb = thumbnail_path(&f);
            out.push(UserTemplate { name, category: category.clone(), thumbnail: crate::vfs::is_file(&thumb).then_some(thumb), path: f });
        }
    }
    out.sort_by_key(|t| (t.category.to_lowercase(), t.name.to_lowercase()));
    out
}

/// A template's picture: its first frame fitting the recent files' thumbnail size, guides
/// drawn, their lines widened to stay visible at that size.
pub fn picture_png(doc: &anicraft_doc::Document) -> Result<Vec<u8>> {
    let (w, h) = (doc.stage.width.max(1) as f32, doc.stage.height.max(1) as f32);
    let scale = (crate::cmds_recent::THUMB_W as f32 / w).min(crate::cmds_recent::THUMB_H as f32 / h).min(1.0);
    crate::cmds_recent::thumbnail_png(&templates::with_guides_shown(doc, 1.5 / scale.max(1e-3)))
}

fn new_from_template(s: &mut Session, p: &Value) -> Result<Value> {
    let (doc, name) = match (opt_str(p, "template")?, opt_str(p, "path")?) {
        (Some(id), None) => {
            let t = templates::find(id).ok_or_else(|| EngineError::Param(format!("no template '{id}' (file.templates lists them)")))?;
            ((t.build)(), t.name.to_string())
        }
        (None, Some(path)) => {
            let path = PathBuf::from(path);
            if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("anicraft")) {
                return Err(EngineError::Param("a template is an .anicraft document".into()));
            }
            let name = path.file_stem().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            (format::load(&path)?, name)
        }
        _ => return Err(EngineError::Param("name a built-in 'template' or a template file's 'path'".into())),
    };
    // A new document: untitled, so Save asks for a name and never writes over the template.
    s.replace_document(doc, None);
    // Ready to draw: the top layer that takes artwork is active (not a locked guide).
    if let Some(l) = s.timeline().layers.iter().find(|l| l.kind == anicraft_doc::LayerKind::Normal && !l.locked) {
        s.layer = l.id;
    }
    let mut info = s.info();
    info["template"] = json!(name);
    Ok(info)
}

fn list(_: &mut Session, p: &Value) -> Result<Value> {
    let builtin: Vec<Value> = TEMPLATES
        .iter()
        .map(|t| {
            let d = (t.build)();
            json!({"id": t.id, "name": t.name, "category": t.category, "description": t.description, "width": d.stage.width, "height": d.stage.height, "fps": d.stage.fps, "frames": d.frame_count()})
        })
        .collect();
    let user: Vec<Value> = match opt_str(p, "folder")? {
        Some(f) => user_templates(Path::new(f)).into_iter().map(|t| json!({"name": t.name, "category": t.category, "path": t.path.display().to_string(), "thumbnail": t.thumbnail.map(|p| p.display().to_string())})).collect(),
        None => Vec::new(),
    };
    Ok(json!({"builtin": builtin, "user": user}))
}

fn category(p: &Value) -> Result<&str> {
    Ok(opt_str(p, "category")?.filter(|c| !c.trim().is_empty()).unwrap_or(DEFAULT_CATEGORY))
}

/// The template file the params name.
pub(crate) fn target(p: &Value) -> Result<PathBuf> {
    template_path(Path::new(str_req(p, "folder")?), category(p)?, str_req(p, "name")?)
}

fn save_as_template(s: &mut Session, p: &Value) -> Result<Value> {
    let path = target(p)?;
    if let Some(dir) = path.parent() {
        crate::vfs::create_dir_all(dir)?;
    }
    format::save(&s.doc, &path)?;
    // Its picture shows the guides too (they are much of what a template is).
    let thumb = thumbnail_path(&path);
    let picture = picture_png(&s.doc).and_then(|png| Ok(crate::vfs::write(&thumb, png)?));
    Ok(json!({"path": path.display().to_string(), "thumbnail": picture.is_ok().then(|| thumb.display().to_string())}))
}

fn delete_template(_: &mut Session, p: &Value) -> Result<Value> {
    let path = target(p)?;
    if !crate::vfs::is_file(&path) {
        return Err(EngineError::State(format!("there is no template “{}” in “{}”", str_req(p, "name")?.trim(), category(p)?.trim())));
    }
    crate::vfs::remove_file(&path)?;
    let thumb = thumbnail_path(&path);
    if crate::vfs::is_file(&thumb) {
        crate::vfs::remove_file(&thumb)?;
    }
    // An emptied category goes with its last template.
    if let Some(dir) = path.parent()
        && crate::vfs::list(dir, 1).is_empty()
    {
        let _ = std::fs::remove_dir(dir);
    }
    Ok(json!({"deleted": path.display().to_string()}))
}
