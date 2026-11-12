//! Files dropped onto the window (ADR 0059). What a file is, and where it was dropped, decide
//! what happens: documents open (`file.open`, which asks first over unsaved work); drawings,
//! images and sounds dropped on the stage go onto the active layer at the playhead at the drop
//! point, and dropped on the Library go into the Library (into the folder under the pointer;
//! an SVG as a graphic symbol, ADR 0065). Dropped on a layer's frame in the timeline, a sound
//! goes onto the keyframe there and a drawing or image onto that layer at that frame, in a
//! keyframe made there if there is none (ADR 0065).
//! The routing is a pure function the shell and tests share; `file.drop` does the import, so a
//! drop of several files is one undo step however many it brings in.

use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::{FolderId, LibraryEntry, LibraryItem};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Most files one drop brings in.
pub const MAX_DROPPED: usize = 64;

/// What a dropped file is, by its name (and, for an XFL folder, what is inside it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropKind {
    /// `.anicraft`, `.fla`, `.xfl` (or an XFL folder): opens as the document.
    Document,
    /// `.svg`.
    Drawing,
    /// `.png`, `.jpg`, `.jpeg`.
    Image,
    /// `.wav`, `.ogg`, `.mp3`.
    Sound,
    /// Anything else.
    Unsupported,
}

/// Where a file was dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropTarget {
    Stage,
    Library,
    /// A layer's frame in the timeline.
    Timeline,
    /// Anywhere else on the window (panels, the timeline, the menu bar).
    Window,
}

/// What a drop does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropAction {
    /// Open it as the document (`file.open`).
    Open,
    /// `file.drop` with `to: "stage"`.
    ImportToStage,
    /// `file.drop` with `to: "library"`.
    ImportToLibrary,
    /// `file.drop` with `to: "timeline"`.
    ImportToTimeline,
    /// Nothing; the message (English, translated where it is shown) says why.
    Refuse(&'static str),
}

pub fn kind_of(path: &Path) -> DropKind {
    if crate::cmds_file::is_animate(path) {
        return DropKind::Document;
    }
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("anicraft") => DropKind::Document,
        Some("svg") => DropKind::Drawing,
        Some("png" | "jpg" | "jpeg") => DropKind::Image,
        _ if crate::cmds_sound::is_sound(path) => DropKind::Sound,
        _ => DropKind::Unsupported,
    }
}

/// The message for a file AniCraft can't take.
pub const UNSUPPORTED: &str = "AniCraft opens .anicraft, .fla and .xfl documents, and imports SVG drawings, PNG and JPEG images, and WAV, Ogg and MP3 sounds";

/// What dropping a file of `kind` on `target` does.
pub fn route(kind: DropKind, target: DropTarget) -> DropAction {
    match (kind, target) {
        (DropKind::Document, _) => DropAction::Open,
        (DropKind::Unsupported, _) => DropAction::Refuse(UNSUPPORTED),
        (_, DropTarget::Library) => DropAction::ImportToLibrary,
        (_, DropTarget::Timeline) => DropAction::ImportToTimeline,
        (_, DropTarget::Stage) => DropAction::ImportToStage,
        // Not over the stage, the timeline or the Library: drop it on one of them.
        (_, DropTarget::Window) => DropAction::Refuse(ELSEWHERE),
    }
}

/// The message for media dropped where nothing takes them.
pub const ELSEWHERE: &str = "Drop drawings, images and sounds on the stage or a layer's frame to place them, or on the Library to add them";

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "file.drop",
        label: "Import Dropped Files",
        menu: "",
        shortcut: None,
        params: "{paths: [..] (or path; SVG, PNG, JPEG, WAV, OGG, MP3; at most 64), to: stage | library | timeline, x?, y? (stage and timeline: the point in the edited space to centre on, default the stage centre), folder? (library: a folder id), layer, frame (timeline: a layer id and a frame, from 0)} → {elements, items, layers}: on the stage each drawing, image and sound goes onto the active layer's keyframe at the playhead, centred on the drop point (a drawing with several layers or masks gets its own layers, as Import to Stage); in the Library images and sounds are added, and drawings become graphic symbols, into `folder`; on the timeline the playhead goes to the frame, a sound goes onto the layer's keyframe there (one sound per drop) and drawings and images onto the layer in a keyframe starting at that frame (made if needed, keeping what the frame showed; a drawing with several layers or masks goes in as a graphic symbol). One undo step",
        mutates: true,
        run: drop,
    }]
}

fn paths(p: &Value) -> Result<Vec<PathBuf>> {
    let list: Vec<PathBuf> = match (p.get("paths"), opt_str(p, "path")?) {
        (Some(v), None) => {
            let a = v.as_array().ok_or_else(|| EngineError::Param("param 'paths' must be a list of file paths".into()))?;
            a.iter().map(|x| x.as_str().map(PathBuf::from).ok_or_else(|| EngineError::Param("param 'paths' must be a list of file paths".into()))).collect::<Result<_>>()?
        }
        (None, Some(one)) => vec![PathBuf::from(one)],
        _ => return Err(EngineError::Param("name the files with 'paths' (or one with 'path')".into())),
    };
    if list.is_empty() || list.len() > MAX_DROPPED {
        return Err(EngineError::Param(format!("drop 1 to {MAX_DROPPED} files at once")));
    }
    Ok(list)
}

fn drop(s: &mut Session, p: &Value) -> Result<Value> {
    let files = paths(p)?;
    let target = match str_req(p, "to")? {
        "stage" => DropTarget::Stage,
        "library" => DropTarget::Library,
        "timeline" => DropTarget::Timeline,
        other => return Err(EngineError::Param(format!("param 'to' must be stage, library or timeline, not '{other}'"))),
    };
    // Everything is checked before anything is imported (the command is atomic anyway).
    for f in &files {
        match route(kind_of(f), target) {
            DropAction::Open => return Err(EngineError::Param(format!("{} is a document: open it with file.open", f.display()))),
            DropAction::Refuse(why) => return Err(EngineError::Param(format!("{}: {why}", f.display()))),
            DropAction::ImportToStage | DropAction::ImportToLibrary | DropAction::ImportToTimeline => {}
        }
    }
    let folder = match opt_u64(p, "folder")? {
        Some(f) if target == DropTarget::Library => {
            let id = FolderId(f);
            if s.doc.folder(id).is_none() {
                return Err(EngineError::State(format!("no Library folder with id {f}")));
            }
            Some(id)
        }
        Some(_) => return Err(EngineError::Param("'folder' is for drops on the Library".into())),
        None => None,
    };
    let x = opt_f32(p, "x")?.unwrap_or(s.doc.stage.width as f32 / 2.0);
    let y = opt_f32(p, "y")?.unwrap_or(s.doc.stage.height as f32 / 2.0);
    if !(x.is_finite() && y.is_finite()) || x.abs() > 1.0e6 || y.abs() > 1.0e6 {
        return Err(EngineError::Param("'x' and 'y' must be within a million pixels of the origin".into()));
    }
    if target == DropTarget::Timeline {
        return onto_timeline(s, p, &files, x, y);
    }
    let (mut elements, mut items, mut layers) = (Vec::new(), Vec::new(), 0u64);
    for f in &files {
        let path = f.display().to_string();
        if target == DropTarget::Library {
            let v = crate::cmds_sound::import_to_library(s, &json!({"path": path}))?;
            let item = match (v["bitmap"].as_u64(), v["sound"].as_u64(), v["symbol"].as_u64()) {
                (Some(b), _, _) => LibraryItem::Bitmap(anicraft_doc::BitmapId(b)),
                (None, Some(snd), _) => LibraryItem::Sound(anicraft_doc::SoundId(snd)),
                (None, None, Some(sym)) => LibraryItem::Symbol(anicraft_doc::SymbolId(sym)),
                _ => return Err(EngineError::State(format!("{path} added nothing to the Library"))),
            };
            if let Some(folder) = folder {
                // Into the folder under the pointer, numbered if it has an item of that name.
                let name = s.doc.item_name(item).unwrap_or("").to_string();
                let free = s.doc.unique_name(Some(folder), &name);
                s.doc.place_entry(LibraryEntry::Item(item), Some(folder));
                if free != name {
                    let mut params = crate::cmds_library::entry_json(LibraryEntry::Item(item));
                    params["name"] = json!(free);
                    crate::cmds_library::rename(s, &params)?;
                }
            }
            items.push(crate::cmds_library::entry_json(LibraryEntry::Item(item)));
            continue;
        }
        match kind_of(f) {
            DropKind::Drawing => {
                // A drawing's own shapes, on the active layer (or its own layers when it has
                // several or masks), centred on the drop point.
                let v = crate::cmds_file::import_to_stage(s, &json!({"path": path, "cx": x, "cy": y, "layer": "active"}))?;
                layers += v["layers"].as_u64().unwrap_or(0);
                elements.extend(s.selection.ids());
            }
            _ => {
                // An image or a sound: into the Library, then placed as Place on Stage does.
                let v = crate::cmds_sound::import_to_library(s, &json!({"path": path}))?;
                let item = match (v["bitmap"].as_u64(), v["sound"].as_u64()) {
                    (Some(b), _) => json!({"bitmap": b}),
                    (None, Some(snd)) => json!({"sound": snd}),
                    _ => return Err(EngineError::State(format!("{path} added nothing to the Library"))),
                };
                items.push(item.clone());
                let mut place = item;
                place["x"] = json!(x);
                place["y"] = json!(y);
                let placed = crate::cmds_draw::place(s, &place)?;
                if let Some(list) = placed["elements"].as_array() {
                    elements.extend(list.iter().filter_map(Value::as_u64).map(anicraft_doc::ElementId));
                }
            }
        }
    }
    if !elements.is_empty() {
        s.select_only(elements.iter().copied());
    }
    Ok(json!({"elements": elements.iter().map(|e| e.0).collect::<Vec<_>>(), "items": items, "layers": layers, "files": files.len()}))
}

/// A drop on a layer's frame in the timeline (ADR 0065).
fn onto_timeline(s: &mut Session, p: &Value, files: &[PathBuf], x: f32, y: f32) -> Result<Value> {
    let layer = anicraft_doc::LayerId(opt_u64(p, "layer")?.ok_or_else(|| EngineError::Param("a drop on the timeline names its 'layer' (an id)".into()))?);
    let frame = opt_u32(p, "frame")?.ok_or_else(|| EngineError::Param("a drop on the timeline names its 'frame' (from 0)".into()))?;
    if frame > crate::MAX_FRAME {
        return Err(EngineError::Param(format!("'frame' must be at most {}", crate::MAX_FRAME)));
    }
    let tl = s.timeline();
    let l = tl.layer(layer).ok_or_else(|| EngineError::State(format!("no layer with id {} here", layer.0)))?;
    let refuse = match l.kind {
        anicraft_doc::LayerKind::Folder => Some("is a folder: drop on one of its layers"),
        anicraft_doc::LayerKind::Camera => Some("is the camera layer: drop on a layer below it"),
        anicraft_doc::LayerKind::Armature => Some("holds an armature, posed by its bones: drop on another layer"),
        _ if l.locked || tl.locked_by_folder(layer) => Some("is locked: unlock it to drop onto it"),
        _ => None,
    };
    if let Some(why) = refuse {
        return Err(EngineError::State(format!("“{}” {why}", l.name)));
    }
    if files.iter().filter(|f| kind_of(f) == DropKind::Sound).count() > 1 {
        return Err(EngineError::Param("a keyframe holds one sound: drop sounds one at a time, or on different layers".into()));
    }
    s.layer = layer;
    s.frame = frame;
    // Artwork goes into a keyframe starting at the frame: made there when the frame is inside
    // another's span (keeping what it showed, as Insert Keyframe does).
    let artwork = files.iter().any(|f| kind_of(f) != DropKind::Sound);
    let made = artwork && s.timeline().layer(layer).is_some_and(|l| l.keyframe_starting_at(frame).is_none());
    if made {
        crate::cmds_timeline::insert_keyframe_at(s.layer_mut(Some(layer))?, frame, true)?;
    }
    let (mut elements, mut items) = (Vec::new(), Vec::new());
    let placed_ids = |v: &Value| -> Vec<anicraft_doc::ElementId> { v["elements"].as_array().into_iter().flatten().filter_map(Value::as_u64).map(anicraft_doc::ElementId).collect() };
    for f in files {
        let path = f.display().to_string();
        let nothing = || EngineError::State(format!("{path} added nothing to the Library"));
        match kind_of(f) {
            DropKind::Drawing if crate::cmds_file::one_layer(&crate::cmds_file::read_svg(f)?) => {
                crate::cmds_file::import_to_stage(s, &json!({"path": path, "cx": x, "cy": y, "layer": "active"}))?;
                elements.extend(s.selection.ids());
            }
            DropKind::Drawing => {
                // Its layers can't all go on one layer: a graphic symbol holding them goes there.
                let v = crate::cmds_file::import_svg_symbol(s, f, None)?;
                let sym = v["symbol"].as_u64().ok_or_else(nothing)?;
                items.push(json!({"symbol": sym}));
                elements.extend(placed_ids(&crate::cmds_draw::place(s, &json!({"symbol": sym, "x": x, "y": y}))?));
            }
            DropKind::Sound => {
                let v = crate::cmds_sound::import_to_library(s, &json!({"path": path}))?;
                let snd = v["sound"].as_u64().ok_or_else(nothing)?;
                items.push(json!({"sound": snd}));
                // Onto the keyframe the frame is in (one is made past the layer's end), as long
                // as the sound when it is the layer's last.
                crate::cmds_sound::place(s, &json!({"sound": snd, "layer": layer.0, "frame": frame, "extend": true}))?;
            }
            _ => {
                let v = crate::cmds_sound::import_to_library(s, &json!({"path": path}))?;
                let b = v["bitmap"].as_u64().ok_or_else(nothing)?;
                items.push(json!({"bitmap": b}));
                elements.extend(placed_ids(&crate::cmds_draw::place(s, &json!({"bitmap": b, "x": x, "y": y}))?));
            }
        }
    }
    if !elements.is_empty() {
        s.select_only(elements.iter().copied());
    }
    Ok(json!({"elements": elements.iter().map(|e| e.0).collect::<Vec<_>>(), "items": items, "layers": 0, "files": files.len(), "layer": layer.0, "frame": frame, "keyframe_made": made}))
}
