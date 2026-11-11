//! How the Library is organised (ADR 0050): folders, the folder each item is in, names unique
//! within a folder, how often each item is used, and which items nothing reaches.
//!
//! The organisation is kept apart from the items themselves ([`Document::library_tree`]): moving
//! or renaming a folder touches one small table, never a symbol's timeline, and documents from
//! before folders open with every item at the top level.

use crate::{BitmapId, Document, ElementId, ElementKind, LayerId, Paint, SoundId, Symbol, SymbolId, Timeline};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

/// Unique id of a Library folder (from the document's id counter).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FolderId(pub u64);

/// A symbol, bitmap or sound in the Library.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LibraryItem {
    Symbol(SymbolId),
    Bitmap(BitmapId),
    Sound(SoundId),
}

impl LibraryItem {
    /// The id inside, whatever the kind (ids are unique across kinds within a document).
    pub fn raw(self) -> u64 {
        match self {
            LibraryItem::Symbol(i) => i.0,
            LibraryItem::Bitmap(i) => i.0,
            LibraryItem::Sound(i) => i.0,
        }
    }
}

/// One row of the Library: an item or a folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LibraryEntry {
    Item(LibraryItem),
    Folder(FolderId),
}

/// A Library folder. Folders nest; `parent` is `None` at the top level.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LibraryFolder {
    pub id: FolderId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<FolderId>,
}

/// The Library's folders and where each item is.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LibraryTree {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folders: Vec<LibraryFolder>,
    /// The folder each item is in. An item not listed is at the top level.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty", with = "placed_list")]
    pub placed: BTreeMap<LibraryItem, FolderId>,
}

impl LibraryTree {
    pub fn is_empty(&self) -> bool {
        self.folders.is_empty() && self.placed.is_empty()
    }
}

/// `placed` as a list of `{"symbol": 3, "folder": 7}` (JSON object keys must be strings).
mod placed_list {
    use super::{FolderId, LibraryItem};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::collections::BTreeMap;

    #[derive(Serialize, Deserialize)]
    struct Placed {
        #[serde(flatten)]
        item: LibraryItem,
        folder: FolderId,
    }

    pub fn serialize<S: Serializer>(m: &BTreeMap<LibraryItem, FolderId>, s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(m.iter().map(|(item, folder)| Placed { item: *item, folder: *folder }))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<BTreeMap<LibraryItem, FolderId>, D::Error> {
        let v: Vec<Placed> = Vec::deserialize(d)?;
        Ok(v.into_iter().map(|p| (p.item, p.folder)).collect())
    }
}

/// Most folders in a Library.
pub const MAX_FOLDERS: usize = 10_000;
/// Deepest folder nesting (a top-level folder is at depth 1).
pub const MAX_FOLDER_DEPTH: usize = 32;
/// Longest Library name (characters).
pub const MAX_NAME_CHARS: usize = 256;

/// Why a name can't be used for a Library item or folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameError {
    Empty,
    TooLong,
    /// `/` separates folders in a path (`Characters/Ball`).
    Slash,
    /// The folder already holds an item or folder of that name (case aside).
    Taken(String),
}

impl std::fmt::Display for NameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NameError::Empty => write!(f, "a Library name can't be empty"),
            NameError::TooLong => write!(f, "a Library name is at most {MAX_NAME_CHARS} characters"),
            NameError::Slash => write!(f, "a Library name can't contain “/” (it separates folders)"),
            NameError::Taken(n) => write!(f, "this folder already has an item named “{n}”"),
        }
    }
}

/// Library names compare without case, as in Animate.
fn same_name(a: &str, b: &str) -> bool {
    a == b || a.to_lowercase() == b.to_lowercase()
}

/// A name trimmed and checked for the Library's rules (not for clashes).
pub fn clean_name(name: &str) -> Result<String, NameError> {
    let n = name.trim();
    if n.is_empty() {
        return Err(NameError::Empty);
    }
    if n.chars().count() > MAX_NAME_CHARS {
        return Err(NameError::TooLong);
    }
    if n.contains('/') {
        return Err(NameError::Slash);
    }
    Ok(n.to_string())
}

impl Document {
    /// Every item in the Library: symbols, then bitmaps, then sounds.
    pub fn library_items(&self) -> impl Iterator<Item = LibraryItem> + '_ {
        self.library
            .iter()
            .map(|s| LibraryItem::Symbol(s.id))
            .chain(self.bitmaps.iter().map(|b| LibraryItem::Bitmap(b.id)))
            .chain(self.sounds.iter().map(|s| LibraryItem::Sound(s.id)))
    }

    pub fn has_item(&self, item: LibraryItem) -> bool {
        self.item_name(item).is_some()
    }

    pub fn item_name(&self, item: LibraryItem) -> Option<&str> {
        match item {
            LibraryItem::Symbol(id) => self.symbol(id).map(|s| s.name.as_str()),
            LibraryItem::Bitmap(id) => self.bitmap(id).map(|b| b.name.as_str()),
            LibraryItem::Sound(id) => self.sound(id).map(|s| s.name.as_str()),
        }
    }

    pub fn folder(&self, id: FolderId) -> Option<&LibraryFolder> {
        self.library_tree.folders.iter().find(|f| f.id == id)
    }

    pub fn has_entry(&self, e: LibraryEntry) -> bool {
        match e {
            LibraryEntry::Item(i) => self.has_item(i),
            LibraryEntry::Folder(f) => self.folder(f).is_some(),
        }
    }

    pub fn entry_name(&self, e: LibraryEntry) -> Option<&str> {
        match e {
            LibraryEntry::Item(i) => self.item_name(i),
            LibraryEntry::Folder(f) => self.folder(f).map(|f| f.name.as_str()),
        }
    }

    /// The folder `e` is in (`None`: the top level).
    pub fn parent_of(&self, e: LibraryEntry) -> Option<FolderId> {
        match e {
            LibraryEntry::Item(i) => self.library_tree.placed.get(&i).copied(),
            LibraryEntry::Folder(f) => self.folder(f).and_then(|f| f.parent),
        }
    }

    /// The folders from the top level down to `id` (inclusive). Bounded, so a damaged tree can't
    /// loop.
    pub fn folder_chain(&self, id: FolderId) -> Vec<FolderId> {
        let mut out = Vec::new();
        let mut at = Some(id);
        while let Some(f) = at.and_then(|i| self.folder(i)) {
            if out.len() >= MAX_FOLDER_DEPTH || out.contains(&f.id) {
                break;
            }
            out.push(f.id);
            at = f.parent;
        }
        out.reverse();
        out
    }

    /// `e`'s place as a path, `Characters/Hero/Ball` (`/` joins folder names).
    pub fn entry_path(&self, e: LibraryEntry) -> String {
        let mut parts: Vec<&str> = self.parent_of(e).map(|p| self.folder_chain(p)).unwrap_or_default().into_iter().filter_map(|f| self.folder(f).map(|x| x.name.as_str())).collect();
        parts.push(self.entry_name(e).unwrap_or(""));
        parts.join("/")
    }

    /// Whether `inner` is `outer` or inside it, at any depth.
    pub fn folder_within(&self, inner: FolderId, outer: FolderId) -> bool {
        self.folder_chain(inner).contains(&outer)
    }

    /// The folders and items directly in `parent` (`None`: the top level), folders first.
    pub fn library_children(&self, parent: Option<FolderId>) -> Vec<LibraryEntry> {
        let folders = self.library_tree.folders.iter().filter(|f| f.parent == parent).map(|f| LibraryEntry::Folder(f.id));
        let items = self.library_items().filter(|i| self.library_tree.placed.get(i).copied() == parent).map(LibraryEntry::Item);
        folders.chain(items).collect()
    }

    /// Everything inside folder `id` at any depth (not the folder itself): folders, then items.
    pub fn folder_contents(&self, id: FolderId) -> Vec<LibraryEntry> {
        let inside: Vec<FolderId> = self.library_tree.folders.iter().filter(|f| f.id != id && self.folder_within(f.id, id)).map(|f| f.id).collect();
        let mut all = inside.clone();
        all.push(id);
        let items = self.library_items().filter(|i| self.library_tree.placed.get(i).is_some_and(|f| all.contains(f)));
        inside.into_iter().map(LibraryEntry::Folder).chain(items.map(LibraryEntry::Item)).collect()
    }

    /// How deep folder `id` is (1 at the top level).
    pub fn folder_depth(&self, id: FolderId) -> usize {
        self.folder_chain(id).len()
    }

    /// How many folder levels `id` holds, itself included (1 for a folder without subfolders).
    pub fn folder_height(&self, id: FolderId) -> usize {
        let base = self.folder_depth(id);
        self.library_tree.folders.iter().filter(|f| self.folder_within(f.id, id)).map(|f| self.folder_depth(f.id) + 1 - base).max().unwrap_or(1)
    }

    /// Whether folder `parent` already holds something named `name` (case aside), other than
    /// `except`.
    pub fn name_taken(&self, parent: Option<FolderId>, name: &str, except: Option<LibraryEntry>) -> bool {
        self.library_children(parent).into_iter().filter(|e| Some(*e) != except).any(|e| self.entry_name(e).is_some_and(|n| same_name(n, name)))
    }

    /// `name` checked for the Library's rules and for a clash in `parent`.
    pub fn check_name(&self, parent: Option<FolderId>, name: &str, except: Option<LibraryEntry>) -> Result<String, NameError> {
        let n = clean_name(name)?;
        if self.name_taken(parent, &n, except) {
            return Err(NameError::Taken(n));
        }
        Ok(n)
    }

    /// `wanted`, or `wanted 2`, `wanted 3`, … whichever `parent` doesn't hold yet.
    pub fn unique_name(&self, parent: Option<FolderId>, wanted: &str) -> String {
        let base: String = wanted.trim().replace('/', "-").chars().take(MAX_NAME_CHARS - 8).collect();
        let base = if base.is_empty() { "Item".to_string() } else { base };
        if !self.name_taken(parent, &base, None) {
            return base;
        }
        let siblings = self.library_children(parent).len() + 2;
        (2..=siblings).map(|n| format!("{base} {n}")).find(|c| !self.name_taken(parent, c, None)).unwrap_or(base)
    }

    /// `stem 1`, `stem 2`, … the first that `parent` doesn't hold, counting from `start`.
    pub fn numbered_name(&self, parent: Option<FolderId>, stem: &str, start: usize) -> String {
        let end = start + self.library_children(parent).len() + 1;
        (start.max(1)..=end).map(|n| format!("{stem} {n}")).find(|c| !self.name_taken(parent, c, None)).unwrap_or_else(|| format!("{stem} {end}"))
    }

    /// Moves `e` into folder `to` (`None`: the top level). No checks: see `library.move`.
    pub fn place_entry(&mut self, e: LibraryEntry, to: Option<FolderId>) {
        let tree = Arc::make_mut(&mut self.library_tree);
        match e {
            LibraryEntry::Item(i) => match to {
                Some(f) => {
                    tree.placed.insert(i, f);
                }
                None => {
                    tree.placed.remove(&i);
                }
            },
            LibraryEntry::Folder(id) => {
                if let Some(f) = tree.folders.iter_mut().find(|f| f.id == id) {
                    f.parent = to;
                }
            }
        }
    }

    /// The folder at `path` (`Characters/Hero`), made where missing. `None` for an empty path,
    /// or when the Library can't hold more folders or nesting.
    pub fn ensure_folder_path(&mut self, path: &str) -> Option<FolderId> {
        let mut parent: Option<FolderId> = None;
        for (depth, part) in path.split('/').map(str::trim).filter(|p| !p.is_empty()).enumerate() {
            if depth >= MAX_FOLDER_DEPTH {
                break;
            }
            let part: String = part.chars().take(MAX_NAME_CHARS).collect();
            let found = self.library_tree.folders.iter().find(|f| f.parent == parent && same_name(&f.name, &part)).map(|f| f.id);
            parent = Some(match found {
                Some(id) => id,
                None => {
                    if self.library_tree.folders.len() >= MAX_FOLDERS {
                        return parent;
                    }
                    let id = FolderId(self.alloc_id());
                    Arc::make_mut(&mut self.library_tree).folders.push(LibraryFolder { id, name: part, parent });
                    id
                }
            });
        }
        parent
    }

    /// How many times each item is used: symbol instances, shapes filled with a bitmap and
    /// keyframes playing a sound, counted once per keyframe on every timeline (scenes and
    /// symbols). Items used nowhere are left out.
    pub fn library_uses(&self) -> HashMap<LibraryItem, usize> {
        let mut uses: HashMap<LibraryItem, usize> = HashMap::new();
        for tl in self.timelines() {
            visit_refs(tl, |item| *uses.entry(item).or_default() += 1);
        }
        uses
    }

    /// The items nothing in the movie reaches: not placed in any scene, nor inside a symbol
    /// that is (at any depth). Symbols with a linkage class count as reached, since scripts
    /// can make them by name. In Library order.
    pub fn unused_items(&self) -> Vec<LibraryItem> {
        let mut reached: HashSet<LibraryItem> = HashSet::new();
        let mut stack: Vec<SymbolId> = self.library.iter().filter(|s| s.linkage.is_some()).map(|s| s.id).collect();
        for s in &stack {
            reached.insert(LibraryItem::Symbol(*s));
        }
        let reach = |item: LibraryItem, reached: &mut HashSet<LibraryItem>, stack: &mut Vec<SymbolId>| {
            if reached.insert(item)
                && let LibraryItem::Symbol(id) = item
            {
                stack.push(id);
            }
        };
        for scene in &self.scenes {
            visit_refs(&scene.timeline, |item| reach(item, &mut reached, &mut stack));
        }
        // Each symbol is walked at most once (it enters the stack when first reached).
        while let Some(id) = stack.pop() {
            if let Some(sym) = self.symbol(id) {
                visit_refs(&sym.timeline, |item| reach(item, &mut reached, &mut stack));
            }
        }
        self.library_items().filter(|i| !reached.contains(i)).collect()
    }

    /// Whether symbol `outer` is `inner` or holds an instance of it at any depth (so placing
    /// `inner` inside `outer` would make a symbol contain itself).
    pub fn symbol_contains(&self, outer: SymbolId, inner: SymbolId) -> bool {
        let mut seen: HashSet<SymbolId> = HashSet::new();
        let mut stack = vec![outer];
        while let Some(id) = stack.pop() {
            if id == inner {
                return true;
            }
            if !seen.insert(id) {
                continue;
            }
            if let Some(sym) = self.symbol(id) {
                visit_refs(&sym.timeline, |item| {
                    if let LibraryItem::Symbol(s) = item {
                        stack.push(s);
                    }
                });
            }
        }
        false
    }

    /// A copy of symbol `id` named `name`, with fresh layer and element ids, in the same
    /// folder, right after it in the Library. Its linkage class stays with the original (a
    /// class names one symbol). The new symbol's id, or `None` if there is no such symbol.
    pub fn duplicate_symbol(&mut self, id: SymbolId, name: String) -> Option<SymbolId> {
        let at = self.library.iter().position(|s| s.id == id)?;
        let src = self.library.get(at)?.clone();
        let new_id = SymbolId(self.alloc_id());
        let timeline = fresh_ids(&src.timeline, &mut || self.alloc_id());
        let copy = Symbol { id: new_id, name, kind: src.kind, linkage: None, scale_grid_kept: src.scale_grid_kept, scale_grid: src.scale_grid, timeline };
        self.library.insert(at + 1, Arc::new(copy));
        if let Some(f) = self.library_tree.placed.get(&LibraryItem::Symbol(id)).copied() {
            Arc::make_mut(&mut self.library_tree).placed.insert(LibraryItem::Symbol(new_id), f);
        }
        Some(new_id)
    }

    /// Repairs a loaded Library tree: folders with unique ids and usable names, parents that
    /// exist, no cycles, nesting within [`MAX_FOLDER_DEPTH`], and only placements of items and
    /// folders that exist. Touches the tree only if something needs it.
    pub(crate) fn sanitize_library_tree(&mut self, max_id: &mut u64) {
        let ok = |d: &Document| {
            let t = &d.library_tree;
            let mut ids = HashSet::new();
            t.folders.len() <= MAX_FOLDERS
                && t.folders.iter().all(|f| ids.insert(f.id) && clean_name(&f.name).as_deref() == Ok(f.name.as_str()) && f.parent.is_none_or(|p| p != f.id && d.folder(p).is_some()))
                && t.folders.iter().all(|f| d.folder_chain(f.id).first().is_some_and(|top| d.folder(*top).is_some_and(|x| x.parent.is_none())))
                && t.placed.iter().all(|(i, f)| d.has_item(*i) && d.folder(*f).is_some())
        };
        for f in &self.library_tree.folders {
            *max_id = (*max_id).max(f.id.0);
        }
        if ok(self) {
            return;
        }
        let mut tree = (*self.library_tree).clone();
        let mut seen = HashSet::new();
        tree.folders.retain(|f| seen.insert(f.id));
        tree.folders.truncate(MAX_FOLDERS);
        for (i, f) in tree.folders.iter_mut().enumerate() {
            let name: String = f.name.trim().replace('/', "-").chars().take(MAX_NAME_CHARS).collect();
            f.name = if name.is_empty() { format!("Folder {}", i + 1) } else { name };
        }
        let ids: HashSet<FolderId> = tree.folders.iter().map(|f| f.id).collect();
        for f in &mut tree.folders {
            if f.parent.is_some_and(|p| p == f.id || !ids.contains(&p)) {
                f.parent = None;
            }
        }
        // Cycles and over-deep chains: the first folder found on one goes to the top level.
        for i in 0..tree.folders.len() {
            let Some(start) = tree.folders.get(i).map(|f| f.id) else { continue };
            let mut at = tree.folders.get(i).and_then(|f| f.parent);
            let mut steps = 1;
            let mut bad = false;
            while let Some(p) = at {
                steps += 1;
                if p == start || steps > MAX_FOLDER_DEPTH {
                    bad = true;
                    break;
                }
                at = tree.folders.iter().find(|f| f.id == p).and_then(|f| f.parent);
            }
            if bad && let Some(f) = tree.folders.get_mut(i) {
                f.parent = None;
            }
        }
        let items: HashSet<LibraryItem> = self.library_items().collect();
        tree.placed.retain(|i, f| items.contains(i) && ids.contains(f));
        self.library_tree = Arc::new(tree);
    }
}

/// Calls `f` once per Library reference on each keyframe of `tl`: symbol instances, bitmap
/// fills and the keyframe's sound.
pub fn visit_refs(tl: &Timeline, mut f: impl FnMut(LibraryItem)) {
    for l in &tl.layers {
        for k in &l.keyframes {
            for e in &k.elements {
                match &e.kind {
                    ElementKind::Symbol { symbol, .. } => f(LibraryItem::Symbol(*symbol)),
                    ElementKind::Shape(s) => {
                        if let Some(Paint::Bitmap { bitmap, .. }) = &s.fill {
                            f(LibraryItem::Bitmap(*bitmap));
                        }
                    }
                    ElementKind::Text(_) | ElementKind::Camera => {}
                }
            }
            if let Some(s) = &k.sound {
                f(LibraryItem::Sound(s.sound));
            }
        }
    }
}

/// `tl` with new layer and element ids from `alloc`, every reference to them (layer parents,
/// motion tween targets, bone ends) following. An element keeps one id across keyframes, as
/// tweens need.
pub fn fresh_ids(tl: &Timeline, alloc: &mut dyn FnMut() -> u64) -> Timeline {
    let mut layers: HashMap<LayerId, LayerId> = HashMap::new();
    let mut elements: HashMap<ElementId, ElementId> = HashMap::new();
    for l in &tl.layers {
        layers.entry(l.id).or_insert_with(|| LayerId(alloc()));
        for k in &l.keyframes {
            for e in &k.elements {
                elements.entry(e.id).or_insert_with(|| ElementId(alloc()));
            }
        }
    }
    let el = |id: ElementId| elements.get(&id).copied().unwrap_or(id);
    let out = tl
        .layers
        .iter()
        .map(|l| {
            let mut l = (**l).clone();
            l.id = layers.get(&l.id).copied().unwrap_or(l.id);
            l.parent = l.parent.map(|p| layers.get(&p).copied().unwrap_or(p));
            if let Some(a) = &mut l.armature {
                for b in &mut Arc::make_mut(a).bones {
                    b.parent = el(b.parent);
                    b.child = el(b.child);
                }
            }
            for k in &mut l.keyframes {
                let k = Arc::make_mut(k);
                for e in &mut k.elements {
                    Arc::make_mut(e).id = el(e.id);
                }
                if let Some(m) = &mut k.motion {
                    m.target = el(m.target);
                }
            }
            Arc::new(l)
        })
        .collect();
    Timeline { layers: out }
}

#[cfg(test)]
#[path = "library_tests.rs"]
mod tests;
