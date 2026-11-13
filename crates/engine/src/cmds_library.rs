//! Library management (ADR 0050): folders, renaming, moving, duplicating and deleting items,
//! use counts and unused items, bitmap smoothing, and Swap Symbol.
//!
//! An item or folder is named in params as `{"symbol": id}`, `{"bitmap": id}`, `{"sound": id}`
//! or `{"folder": id}`. Sorting and filtering the list are views (the shell's); `library.items`
//! gives a script everything it needs to do the same.

use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::library::{MAX_FOLDER_DEPTH, MAX_FOLDERS};
use anicraft_doc::{BitmapId, ElementKind, FolderId, LibraryEntry, LibraryFolder, LibraryItem, Paint, SoundId, SymbolId, SymbolKind, Timeline};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c("library.items", "List Library", "", None, "{} every folder (id, name, parent, path) and item (symbol | bitmap | sound id, kind, name, folder, path, uses, unused) in Library order; `uses` counts instances, bitmap fills and keyframe sounds once per keyframe", false, items as crate::RunFn),
        c("library.new_folder", "New Folder", "", None, "{name? (default: the next free “Folder n”), parent?: folder id} → {folder, name}; names are unique within a folder (case aside) and can't hold “/”", true, new_folder),
        c("library.rename", "Rename", "", None, "{symbol | bitmap | sound | folder: id, name} renames a Library item or folder; the name must be free in its folder (case aside) and can't hold “/”", true, rename),
        c("library.move", "Move to Folder", "", None, "{items: [{symbol | bitmap | sound | folder: id}, ..], to: folder id | null (the top level)} moves items and folders (with what they hold) into a folder; a folder can't go inside itself, and a name the folder already has stops the move", true, move_to),
        c("library.duplicate", "Duplicate Symbol", "", None, "{symbol, name? (default: “<name> copy”)} → {symbol, name}: a copy in the same folder, with its own layers and elements and no linkage class", true, duplicate),
        c("library.delete", "Delete", "", None, "{items: [{symbol | bitmap | sound | folder: id}, ..]} deletes items and folders with everything in them; instances of deleted symbols, shapes filled with deleted bitmaps and keyframes' deleted sounds go too (asks first when anything deleted is in use or a folder isn't empty)", true, delete),
        c("library.unused", "Select Unused Items", "", None, "{} → {items: [..]} the items nothing in the movie reaches: not in a scene, nor inside a symbol that is; symbols with a linkage class count as used", false, unused),
        c("library.delete_unused", "Delete Unused Items", "", None, "{} deletes every item `library.unused` lists (asks first)", true, delete_unused),
        c("library.set_smoothing", "Allow Smoothing", "", None, "{bitmap, on?: bool (default: toggle)} whether the bitmap is filtered when scaled or turned (off: hard-edged pixels)", true, set_smoothing),
        c("element.swap_bitmap", "Swap Bitmap", "", None, "{bitmap} the selected bitmaps and bitmap-filled shapes show another bitmap: a placed bitmap keeps its transform and top-left corner and takes the new bitmap's size; a bitmap fill keeps its shape and fill transform", true, |s, p| s.each_layer(false, |s| swap_bitmap(s, p))),
        c("element.swap_symbol", "Swap Symbol", "", None, "{symbol} the selected instances show another symbol, keeping their transform, colour effect, blending, filters, looping, actions and instance name", true, |s, p| s.each_layer(false, |s| swap_symbol(s, p))),
    ]
}

/// Most entries one command takes.
const MAX_ENTRIES: usize = 100_000;

/// The entry a JSON object names: `{"symbol": id}`, `{"bitmap": id}`, `{"sound": id}` or
/// `{"folder": id}`.
pub(crate) fn entry_of(v: &Value) -> Result<LibraryEntry> {
    let id = |k: &str| v.get(k).and_then(Value::as_u64);
    match (id("symbol"), id("bitmap"), id("sound"), id("folder")) {
        (Some(i), None, None, None) => Ok(LibraryEntry::Item(LibraryItem::Symbol(SymbolId(i)))),
        (None, Some(i), None, None) => Ok(LibraryEntry::Item(LibraryItem::Bitmap(BitmapId(i)))),
        (None, None, Some(i), None) => Ok(LibraryEntry::Item(LibraryItem::Sound(SoundId(i)))),
        (None, None, None, Some(i)) => Ok(LibraryEntry::Folder(FolderId(i))),
        _ => Err(EngineError::Param("name one Library entry: {\"symbol\" | \"bitmap\" | \"sound\" | \"folder\": id}".into())),
    }
}

/// `entry` as params name it.
pub fn entry_json(e: LibraryEntry) -> Value {
    match e {
        LibraryEntry::Item(LibraryItem::Symbol(i)) => json!({"symbol": i.0}),
        LibraryEntry::Item(LibraryItem::Bitmap(i)) => json!({"bitmap": i.0}),
        LibraryEntry::Item(LibraryItem::Sound(i)) => json!({"sound": i.0}),
        LibraryEntry::Folder(f) => json!({"folder": f.0}),
    }
}

/// The entries in list `key`, each checked to exist (no repeats).
pub(crate) fn entries_of(s: &Session, p: &Value, key: &str) -> Result<Vec<LibraryEntry>> {
    let list = p.get(key).and_then(Value::as_array).ok_or_else(|| EngineError::Param(format!("param '{key}' must be a list of Library entries")))?;
    if list.len() > MAX_ENTRIES {
        return Err(EngineError::Param(format!("at most {MAX_ENTRIES} entries")));
    }
    let mut out = Vec::with_capacity(list.len());
    let mut seen = HashSet::new();
    for v in list {
        let e = entry_of(v)?;
        if !s.doc.has_entry(e) {
            return Err(EngineError::State(format!("the Library has no {}", describe(e))));
        }
        if seen.insert(e) {
            out.push(e);
        }
    }
    Ok(out)
}

fn describe(e: LibraryEntry) -> String {
    match e {
        LibraryEntry::Item(LibraryItem::Symbol(i)) => format!("symbol {}", i.0),
        LibraryEntry::Item(LibraryItem::Bitmap(i)) => format!("bitmap {}", i.0),
        LibraryEntry::Item(LibraryItem::Sound(i)) => format!("sound {}", i.0),
        LibraryEntry::Folder(f) => format!("folder {}", f.0),
    }
}

pub(crate) fn opt_folder(s: &Session, p: &Value, key: &str) -> Result<Option<FolderId>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            let id = v.as_u64().map(FolderId).ok_or_else(|| EngineError::Param(format!("param '{key}' must be a folder id or null")))?;
            if s.doc.folder(id).is_none() {
                return Err(EngineError::State(format!("the Library has no folder {}", id.0)));
            }
            Ok(Some(id))
        }
    }
}

/// The symbol type `key` names (`graphic`, `movie_clip` or `button`).
pub(crate) fn kind_of(key: &str) -> Result<SymbolKind> {
    match key {
        "graphic" => Ok(SymbolKind::Graphic),
        "movie_clip" | "movie clip" => Ok(SymbolKind::MovieClip),
        "button" => Ok(SymbolKind::Button),
        _ => Err(EngineError::Param(format!("unknown symbol type '{key}' (graphic, movie_clip or button)"))),
    }
}

pub(crate) fn kind_key(k: SymbolKind) -> &'static str {
    match k {
        SymbolKind::Graphic => "graphic",
        SymbolKind::MovieClip => "movie_clip",
        SymbolKind::Button => "button",
    }
}

fn items(s: &mut Session, _: &Value) -> Result<Value> {
    let d = &s.doc;
    let uses = d.library_uses();
    let unused: HashSet<LibraryItem> = d.unused_items().into_iter().collect();
    let folders: Vec<Value> = d
        .library_tree
        .folders
        .iter()
        .map(|f| json!({"folder": f.id.0, "name": f.name, "parent": f.parent.map(|p| p.0), "path": d.entry_path(LibraryEntry::Folder(f.id))}))
        .collect();
    let items: Vec<Value> = d
        .library_items()
        .map(|i| {
            let e = LibraryEntry::Item(i);
            let mut v = entry_json(e);
            v["name"] = json!(d.item_name(i));
            v["folder"] = json!(d.parent_of(e).map(|f| f.0));
            v["path"] = json!(d.entry_path(e));
            v["uses"] = json!(uses.get(&i).copied().unwrap_or(0));
            v["unused"] = json!(unused.contains(&i));
            match i {
                LibraryItem::Symbol(id) => {
                    if let Some(sym) = d.symbol(id) {
                        v["kind"] = json!(kind_key(sym.kind));
                        v["frames"] = json!(sym.timeline.frame_count());
                        v["linkage"] = json!(sym.linkage.as_ref().map(|l| &l.class));
                    }
                }
                LibraryItem::Bitmap(id) => {
                    if let Some(b) = d.bitmap(id) {
                        v["kind"] = json!("bitmap");
                        v["width"] = json!(b.width);
                        v["height"] = json!(b.height);
                        v["smoothing"] = json!(b.smoothing);
                    }
                }
                LibraryItem::Sound(id) => {
                    if let Some(x) = d.sound(id) {
                        v["kind"] = json!("sound");
                        v["seconds"] = json!(x.seconds());
                    }
                }
            }
            v
        })
        .collect();
    Ok(json!({"folders": folders, "items": items, "unused": unused.len()}))
}

fn new_folder(s: &mut Session, p: &Value) -> Result<Value> {
    let parent = opt_folder(s, p, "parent")?;
    if s.doc.library_tree.folders.len() >= MAX_FOLDERS {
        return Err(EngineError::State(format!("the Library holds at most {MAX_FOLDERS} folders")));
    }
    if parent.is_some_and(|f| s.doc.folder_depth(f) >= MAX_FOLDER_DEPTH) {
        return Err(EngineError::State(format!("folders nest at most {MAX_FOLDER_DEPTH} deep")));
    }
    let name = match opt_str(p, "name")? {
        Some(n) => s.doc.check_name(parent, n, None).map_err(|e| EngineError::Param(e.to_string()))?,
        None => s.doc.numbered_name(parent, "Folder", 1),
    };
    let id = FolderId(s.doc.alloc_id());
    Arc::make_mut(&mut s.doc.library_tree).folders.push(LibraryFolder { id, name: name.clone(), parent });
    Ok(json!({"folder": id.0, "name": name}))
}

pub(crate) fn rename(s: &mut Session, p: &Value) -> Result<Value> {
    let e = entry_of(p)?;
    if !s.doc.has_entry(e) {
        return Err(EngineError::State(format!("the Library has no {}", describe(e))));
    }
    let name = s.doc.check_name(s.doc.parent_of(e), str_req(p, "name")?, Some(e)).map_err(|err| EngineError::Param(err.to_string()))?;
    let d = &mut s.doc;
    match e {
        LibraryEntry::Item(LibraryItem::Symbol(id)) => {
            if let Some(x) = d.library.iter_mut().find(|x| x.id == id) {
                Arc::make_mut(x).name = name.clone();
            }
        }
        LibraryEntry::Item(LibraryItem::Bitmap(id)) => {
            if let Some(x) = d.bitmaps.iter_mut().find(|x| x.id == id) {
                Arc::make_mut(x).name = name.clone();
            }
        }
        LibraryEntry::Item(LibraryItem::Sound(id)) => {
            if let Some(x) = d.sounds.iter_mut().find(|x| x.id == id) {
                Arc::make_mut(x).name = name.clone();
            }
        }
        LibraryEntry::Folder(id) => {
            if let Some(f) = Arc::make_mut(&mut d.library_tree).folders.iter_mut().find(|f| f.id == id) {
                f.name = name.clone();
            }
        }
    }
    let mut v = entry_json(e);
    v["name"] = json!(name);
    Ok(v)
}

fn move_to(s: &mut Session, p: &Value) -> Result<Value> {
    let list = entries_of(s, p, "items")?;
    if p.get("to").is_none() {
        return Err(EngineError::Param("param 'to' is required: a folder id, or null for the top level".into()));
    }
    let to = opt_folder(s, p, "to")?;
    let mut moved = 0;
    for e in list {
        if s.doc.parent_of(e) == to {
            continue;
        }
        if let LibraryEntry::Folder(f) = e {
            if to.is_some_and(|t| s.doc.folder_within(t, f)) {
                return Err(EngineError::State(format!("“{}” can't go inside itself", s.doc.entry_name(e).unwrap_or(""))));
            }
            if to.map_or(0, |t| s.doc.folder_depth(t)) + s.doc.folder_height(f) > MAX_FOLDER_DEPTH {
                return Err(EngineError::State(format!("folders nest at most {MAX_FOLDER_DEPTH} deep")));
            }
        }
        let name = s.doc.entry_name(e).unwrap_or("").to_string();
        if s.doc.name_taken(to, &name, Some(e)) {
            let place = to.and_then(|t| s.doc.folder(t)).map_or_else(|| "The top level of the Library".to_string(), |f| format!("“{}”", f.name));
            return Err(EngineError::State(format!("{place} already has an item named “{name}”; rename one of them first")));
        }
        s.doc.place_entry(e, to);
        moved += 1;
    }
    Ok(json!({"moved": moved, "to": to.map(|f| f.0)}))
}

fn duplicate(s: &mut Session, p: &Value) -> Result<Value> {
    let id = SymbolId(opt_u64(p, "symbol")?.ok_or_else(|| EngineError::Param("param 'symbol' is required".into()))?);
    let Some(src) = s.doc.symbol(id) else {
        return Err(EngineError::State(format!("no symbol with id {}", id.0)));
    };
    let parent = s.doc.parent_of(LibraryEntry::Item(LibraryItem::Symbol(id)));
    let name = match opt_str(p, "name")? {
        Some(n) => s.doc.check_name(parent, n, None).map_err(|e| EngineError::Param(e.to_string()))?,
        None => s.doc.unique_name(parent, &format!("{} copy", src.name)),
    };
    let copy = s.doc.duplicate_symbol(id, name.clone()).ok_or_else(|| EngineError::State(format!("no symbol with id {}", id.0)))?;
    Ok(json!({"symbol": copy.0, "name": name}))
}

/// What deleting `entries` takes: the items (folders expanded into what they hold) and the
/// folders.
pub(crate) fn deletion(s: &Session, entries: &[LibraryEntry]) -> (Vec<LibraryItem>, Vec<FolderId>) {
    let mut items: Vec<LibraryItem> = Vec::new();
    let mut folders: Vec<FolderId> = Vec::new();
    let add = |e: LibraryEntry, items: &mut Vec<LibraryItem>, folders: &mut Vec<FolderId>| match e {
        LibraryEntry::Item(i) if !items.contains(&i) => items.push(i),
        LibraryEntry::Folder(f) if !folders.contains(&f) => folders.push(f),
        _ => {}
    };
    for &e in entries {
        add(e, &mut items, &mut folders);
        if let LibraryEntry::Folder(f) = e {
            for inner in s.doc.folder_contents(f) {
                add(inner, &mut items, &mut folders);
            }
        }
    }
    (items, folders)
}

/// Takes every reference to `gone` off `tl` (instances, bitmap-filled shapes, keyframe sounds),
/// un-sharing only the layers and keyframes that had one. How many references went.
fn strip(tl: &mut Timeline, gone: &HashSet<LibraryItem>) -> usize {
    strip_in(tl, gone, 0)
}

/// [`strip`], inside groups too (ADR 0073): a group keeps its other members.
fn strip_in(tl: &mut Timeline, gone: &HashSet<LibraryItem>, depth: usize) -> usize {
    let refers = |e: &anicraft_doc::Element| match &e.kind {
        ElementKind::Symbol { symbol, .. } => gone.contains(&LibraryItem::Symbol(*symbol)),
        ElementKind::Shape(sh) => matches!(&sh.fill, Some(Paint::Bitmap { bitmap, .. }) if gone.contains(&LibraryItem::Bitmap(*bitmap))),
        ElementKind::Text(_) | ElementKind::Camera | ElementKind::Group { .. } => false,
    };
    let inside = |e: &anicraft_doc::Element| matches!(&e.kind, ElementKind::Group { members } if depth < anicraft_doc::MAX_GROUP_DEPTH && strip_needed(members, gone));
    let sounds = |k: &anicraft_doc::Keyframe| k.sound.as_ref().is_some_and(|f| gone.contains(&LibraryItem::Sound(f.sound)));
    let hit = |k: &anicraft_doc::Keyframe| sounds(k) || k.elements.iter().any(|e| refers(e) || inside(e));
    let mut n = 0;
    for l in tl.layers.iter_mut().filter(|l| l.keyframes.iter().any(|k| hit(k))) {
        for k in Arc::make_mut(l).keyframes.iter_mut().filter(|k| hit(k)) {
            let k = Arc::make_mut(k);
            if sounds(k) {
                k.sound = None;
                n += 1;
            }
            let before = k.elements.len();
            k.elements.retain(|e| !refers(e));
            n += before - k.elements.len();
            for e in k.elements.iter_mut().filter(|e| inside(e)) {
                if let ElementKind::Group { members } = &mut Arc::make_mut(e).kind {
                    n += strip_in(members, gone, depth + 1);
                }
            }
        }
    }
    n
}

/// Deletes `items` and `folders` from the Library and every use of the items from the movie.
fn remove(s: &mut Session, items: &[LibraryItem], folders: &[FolderId]) -> Value {
    let gone: HashSet<LibraryItem> = items.iter().copied().collect();
    let mut uses = 0;
    if !gone.is_empty() {
        for scene in &mut s.doc.scenes {
            if strip_needed(&scene.timeline, &gone) {
                uses += strip(&mut Arc::make_mut(scene).timeline, &gone);
            }
        }
        s.doc.library.retain(|x| !gone.contains(&LibraryItem::Symbol(x.id)));
        s.doc.bitmaps.retain(|x| !gone.contains(&LibraryItem::Bitmap(x.id)));
        s.doc.sounds.retain(|x| !gone.contains(&LibraryItem::Sound(x.id)));
        for sym in &mut s.doc.library {
            if strip_needed(&sym.timeline, &gone) {
                uses += strip(&mut Arc::make_mut(sym).timeline, &gone);
            }
        }
        // The fill swatch may hold a deleted bitmap (picked with the Eyedropper).
        if let Some(Paint::Bitmap { bitmap, .. }) = &s.style.fill_paint
            && gone.contains(&LibraryItem::Bitmap(*bitmap))
        {
            s.style.fill_paint = None;
        }
    }
    let dead: HashSet<FolderId> = folders.iter().copied().collect();
    if !dead.is_empty() || s.doc.library_tree.placed.keys().any(|i| gone.contains(i)) {
        let t = Arc::make_mut(&mut s.doc.library_tree);
        t.placed.retain(|i, f| !gone.contains(i) && !dead.contains(f));
        t.folders.retain(|f| !dead.contains(&f.id));
        // A folder whose parent went (it was named on its own) goes to the top level.
        let ids: HashSet<FolderId> = t.folders.iter().map(|f| f.id).collect();
        for f in &mut t.folders {
            if f.parent.is_some_and(|p| !ids.contains(&p)) {
                f.parent = None;
            }
        }
    }
    json!({"deleted": items.len(), "folders": folders.len(), "uses": uses})
}

fn strip_needed(tl: &Timeline, gone: &HashSet<LibraryItem>) -> bool {
    let mut any = false;
    anicraft_doc::library::visit_refs(tl, |i| any |= gone.contains(&i));
    any
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    let list = entries_of(s, p, "items")?;
    if list.is_empty() {
        return Err(EngineError::State("nothing to delete".into()));
    }
    let (items, folders) = deletion(s, &list);
    Ok(remove(s, &items, &folders))
}

fn unused(s: &mut Session, _: &Value) -> Result<Value> {
    let list: Vec<Value> = s.doc.unused_items().into_iter().map(|i| entry_json(LibraryEntry::Item(i))).collect();
    Ok(json!({"items": list}))
}

fn delete_unused(s: &mut Session, _: &Value) -> Result<Value> {
    let items = s.doc.unused_items();
    Ok(remove(s, &items, &[]))
}

fn set_smoothing(s: &mut Session, p: &Value) -> Result<Value> {
    let id = BitmapId(opt_u64(p, "bitmap")?.ok_or_else(|| EngineError::Param("param 'bitmap' is required".into()))?);
    let on = opt_bool(p, "on")?;
    let b = s.doc.bitmaps.iter_mut().find(|b| b.id == id).ok_or_else(|| EngineError::State(format!("no bitmap with id {}", id.0)))?;
    let on = on.unwrap_or(!b.smoothing);
    if b.smoothing != on {
        Arc::make_mut(b).smoothing = on;
    }
    Ok(json!({"bitmap": id.0, "smoothing": on}))
}

fn swap_symbol(s: &mut Session, p: &Value) -> Result<Value> {
    let to = SymbolId(opt_u64(p, "symbol")?.ok_or_else(|| EngineError::Param("param 'symbol' is required".into()))?);
    if s.doc.symbol(to).is_none() {
        return Err(EngineError::State(format!("no symbol with id {}", to.0)));
    }
    // Inside a symbol open for editing (ADR 0045), never swap in that symbol or one that holds it.
    if crate::editing::would_nest(s, to) {
        return Err(EngineError::State("a symbol can't be placed inside itself".into()));
    }
    let mut swapped = 0;
    for e in s.selected_elements_mut()? {
        if let ElementKind::Symbol { symbol, .. } = &mut e.kind {
            *symbol = to;
            swapped += 1;
        }
    }
    if swapped == 0 {
        return Err(EngineError::State("select a symbol instance to swap".into()));
    }
    Ok(json!({"swapped": swapped, "symbol": to.0}))
}

/// Swap Bitmap (ADR 0053), as Animate's Properties offer on a bitmap. A bitmap placed from the
/// Library is a rectangle of the bitmap's size filled with it: it keeps its transform (so its
/// top-left corner stays put) and becomes the new bitmap's size, and its name follows when it was
/// the old bitmap's. Any other shape filled with a bitmap keeps its outline and fill transform.
fn swap_bitmap(s: &mut Session, p: &Value) -> Result<Value> {
    let to = BitmapId(opt_u64(p, "bitmap")?.ok_or_else(|| EngineError::Param("param 'bitmap' is required".into()))?);
    if s.doc.bitmap(to).is_none() {
        return Err(EngineError::State(format!("no bitmap with id {}", to.0)));
    }
    let sizes: std::collections::HashMap<BitmapId, (String, f32, f32)> = s.doc.bitmaps.iter().map(|b| (b.id, (b.name.clone(), b.width as f32, b.height as f32))).collect();
    let mut swapped = 0;
    for e in s.selected_elements_mut()? {
        if swap_bitmap_of(e, to, &sizes) {
            swapped += 1;
        }
    }
    if swapped == 0 {
        return Err(EngineError::State("select a bitmap, or a shape filled with one, to swap".into()));
    }
    Ok(json!({"swapped": swapped, "bitmap": to.0}))
}

/// Makes a bitmap-filled shape show bitmap `to` (Swap Bitmap, and Find and Replace, ADR 0058):
/// a placed bitmap (a rectangle of the old bitmap's size, filled once at the origin) takes the
/// new one's size and, when it had the old one's name, its name; any other shape keeps its
/// outline and fill transform. `sizes` are every bitmap's name, width and height. False when `e`
/// isn't filled with a bitmap.
pub(crate) fn swap_bitmap_of(e: &mut anicraft_doc::Element, to: BitmapId, sizes: &std::collections::HashMap<BitmapId, (String, f32, f32)>) -> bool {
    let ElementKind::Shape(sh) = &mut e.kind else { return false };
    let Some(Paint::Bitmap { bitmap, repeat, matrix }) = &mut sh.fill else { return false };
    let old = *bitmap;
    *bitmap = to;
    if old == to {
        return true;
    }
    if let (Some((name, w, h)), Some(new)) = (sizes.get(&old), sizes.get(&to))
        && !*repeat
        && *matrix == anicraft_geom::Affine::IDENTITY
        && *sh.path == anicraft_geom::Path::rect(0.0, 0.0, *w, *h)
    {
        sh.path = Arc::new(anicraft_geom::Path::rect(0.0, 0.0, new.1, new.2));
        if e.name == *name {
            e.name.clone_from(&new.0);
        }
    }
    true
}

#[cfg(test)]
#[path = "library_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "place_tests.rs"]
mod place_tests;
