//! Undo/redo history over persistent document snapshots.
//!
//! Design (see `docs/adr/0001-undo-persistent-snapshots.md`): every undo step is a snapshot of
//! the structurally-shared [`Document`], so a step costs memory only for the nodes the edit
//! actually copied. History is bounded by an estimate of that unique memory, not by step count,
//! so a thousand small nudges are cheap while one huge import is still evictable.

use anicraft_doc::{Document, Element, ElementKind, Keyframe, Layer, LayerId, Symbol};
use std::collections::{HashSet, VecDeque};
use std::mem::size_of;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Globally unique id of a document *state*. Equal ids mean identical content, so render caches
/// can key on it and an undo back to a cached state is a cache hit.
pub type StateId = u64;

static NEXT_STATE: AtomicU64 = AtomicU64::new(1);

pub fn fresh_state() -> StateId {
    NEXT_STATE.fetch_add(1, Ordering::Relaxed)
}

/// Everything restored by undo/redo.
#[derive(Clone)]
pub struct Snapshot {
    pub label: String,
    pub doc: Document,
    pub state: StateId,
    pub scene: usize,
    /// The symbols open for editing (ADR 0045).
    pub editing: Vec<crate::editing::EditLevel>,
    pub frame: u32,
    pub layer: LayerId,
    pub selection: crate::Selection,
    /// Estimated bytes this snapshot keeps alive that the neighbouring state does not share.
    pub bytes: usize,
}

/// Default memory budget for undo history.
pub const DEFAULT_BUDGET_BYTES: usize = 512 * 1024 * 1024;
/// Hard cap on steps, whatever their size.
pub const MAX_STEPS: usize = 10_000;

pub struct History {
    undo: VecDeque<Snapshot>,
    redo: Vec<Snapshot>,
    pub budget_bytes: usize,
    pub max_steps: usize,
    undo_bytes: usize,
    redo_bytes: usize,
    /// Steps dropped from the old end because of the budget.
    pub evicted: usize,
}

impl Default for History {
    fn default() -> Self {
        History { undo: VecDeque::new(), redo: Vec::new(), budget_bytes: DEFAULT_BUDGET_BYTES, max_steps: MAX_STEPS, undo_bytes: 0, redo_bytes: 0, evicted: 0 }
    }
}

impl History {
    /// Records `before` as an undo step leading to `current`, and clears redo.
    pub fn record(&mut self, mut before: Snapshot, current: &Document) {
        before.bytes = unique_bytes(&before.doc, current);
        self.undo_bytes += before.bytes;
        self.undo.push_back(before);
        self.redo.clear();
        self.redo_bytes = 0;
        self.enforce_budget();
    }

    fn enforce_budget(&mut self) {
        while self.undo.len() > 1 && (self.undo_bytes + self.redo_bytes > self.budget_bytes || self.undo.len() > self.max_steps) {
            if let Some(old) = self.undo.pop_front() {
                self.undo_bytes = self.undo_bytes.saturating_sub(old.bytes);
                self.evicted += 1;
            }
        }
    }

    /// Pops the newest undo step and pushes `current` onto the redo stack.
    pub fn undo(&mut self, mut current: Snapshot) -> Option<Snapshot> {
        let s = self.undo.pop_back()?;
        self.undo_bytes = self.undo_bytes.saturating_sub(s.bytes);
        current.bytes = unique_bytes(&current.doc, &s.doc);
        current.label = s.label.clone();
        self.redo_bytes += current.bytes;
        self.redo.push(current);
        Some(s)
    }

    /// Pops the newest redo step and pushes `current` back onto the undo stack.
    pub fn redo(&mut self, mut current: Snapshot) -> Option<Snapshot> {
        let s = self.redo.pop()?;
        self.redo_bytes = self.redo_bytes.saturating_sub(s.bytes);
        current.bytes = unique_bytes(&current.doc, &s.doc);
        current.label = s.label.clone();
        self.undo_bytes += current.bytes;
        self.undo.push_back(current);
        self.enforce_budget();
        Some(s)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }
    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.back().map(|s| s.label.as_str())
    }
    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|s| s.label.as_str())
    }
    /// Labels, oldest first.
    pub fn undo_labels(&self) -> impl Iterator<Item = &str> {
        self.undo.iter().map(|s| s.label.as_str())
    }
    /// Labels of redoable steps, next-to-redo first.
    pub fn redo_labels(&self) -> impl Iterator<Item = &str> {
        self.redo.iter().rev().map(|s| s.label.as_str())
    }
    /// Estimated memory held by history.
    pub fn bytes(&self) -> usize {
        self.undo_bytes + self.redo_bytes
    }
}

// ----- unique-memory estimate ------------------------------------------------------------------

fn ptr<T>(a: &Arc<T>) -> usize {
    Arc::as_ptr(a) as *const u8 as usize
}

fn path_bytes(e: &Element) -> usize {
    match &e.kind {
        ElementKind::Shape(s) => size_of::<anicraft_geom::Path>() + s.path.cmds.len() * size_of::<anicraft_geom::PathCmd>(),
        ElementKind::Text(t) => size_of::<anicraft_geom::Path>() + t.outline.cmds.len() * size_of::<anicraft_geom::PathCmd>(),
        ElementKind::Symbol { .. } | ElementKind::Camera => 0,
    }
}

fn element_bytes(e: &Element, shared_paths: &HashSet<usize>) -> usize {
    let mut b = size_of::<Element>() + e.name.len();
    if let ElementKind::Shape(s) = &e.kind {
        if !shared_paths.contains(&ptr(&s.path)) {
            b += path_bytes(e);
        }
        if let Some(anicraft_doc::Paint::Linear { stops, .. } | anicraft_doc::Paint::Radial { stops, .. }) = &s.fill {
            b += stops.len() * size_of::<anicraft_doc::GradientStop>();
        }
    }
    if let ElementKind::Text(t) = &e.kind {
        b += size_of::<anicraft_doc::TextBlock>() + t.text.len() + t.font.len();
        if !shared_paths.contains(&ptr(&t.outline)) {
            b += path_bytes(e);
        }
    }
    b
}

fn keyframe_bytes(k: &Keyframe, other: Option<&Keyframe>) -> usize {
    let shared_els: HashSet<usize> = other.map(|o| o.elements.iter().map(ptr).collect()).unwrap_or_default();
    let shared_paths: HashSet<usize> = other
        .map(|o| {
            o.elements
                .iter()
                .filter_map(|e| match &e.kind {
                    ElementKind::Shape(s) => Some(ptr(&s.path)),
                    ElementKind::Text(t) => Some(ptr(&t.outline)),
                    ElementKind::Symbol { .. } | ElementKind::Camera => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let mut b = size_of::<Keyframe>() + k.label.len() + k.elements.len() * size_of::<usize>();
    // A kept script's text (up to 1 MiB), unless the other keyframe shares it.
    if let Some(anicraft_doc::FrameScript::Source(src)) = &k.script
        && !matches!(other.and_then(|o| o.script.as_ref()), Some(anicraft_doc::FrameScript::Source(o)) if Arc::ptr_eq(o, src))
    {
        b += src.text.len();
    }
    for e in &k.elements {
        if !shared_els.contains(&ptr(e)) {
            b += element_bytes(e, &shared_paths);
        }
    }
    b
}

fn layer_bytes(l: &Layer, other: Option<&Layer>) -> usize {
    let shared: HashSet<usize> = other.map(|o| o.keyframes.iter().map(ptr).collect()).unwrap_or_default();
    let mut b = size_of::<Layer>() + l.name.len() + l.keyframes.len() * size_of::<usize>();
    for k in &l.keyframes {
        if shared.contains(&ptr(k)) {
            continue;
        }
        let peer = other.and_then(|o| o.keyframes.iter().find(|p| p.start == k.start)).map(|p| &**p);
        b += keyframe_bytes(k, peer);
    }
    b
}

fn symbol_bytes(s: &Symbol, other: Option<&Symbol>) -> usize {
    let shared: HashSet<usize> = other.map(|o| o.timeline.layers.iter().map(ptr).collect()).unwrap_or_default();
    let mut b = size_of::<Symbol>() + s.name.len();
    for l in &s.timeline.layers {
        if !shared.contains(&ptr(l)) {
            let peer = other.and_then(|o| o.timeline.layers.iter().find(|p| p.id == l.id)).map(|p| &**p);
            b += layer_bytes(l, peer);
        }
    }
    b
}

/// Estimated bytes reachable from `old` that are not shared with `new`. Costs O(changed) after
/// an O(layers + symbols) pointer scan.
pub fn unique_bytes(old: &Document, new: &Document) -> usize {
    let layers = |d: &Document| d.scenes.iter().map(|s| s.timeline.layers.len()).sum::<usize>();
    let mut b = size_of::<Snapshot>() + (old.scenes.len() + layers(old) + old.library.len()) * size_of::<usize>();
    let shared_layers: HashSet<usize> = new.scenes.iter().flat_map(|s| s.timeline.layers.iter().map(ptr)).collect();
    for l in old.scenes.iter().flat_map(|s| s.timeline.layers.iter()) {
        if !shared_layers.contains(&ptr(l)) {
            let peer = new.scenes.iter().flat_map(|s| s.timeline.layers.iter()).find(|p| p.id == l.id).map(|p| &**p);
            b += layer_bytes(l, peer);
        }
    }
    let shared_syms: HashSet<usize> = new.library.iter().map(ptr).collect();
    for s in &old.library {
        if !shared_syms.contains(&ptr(s)) {
            let peer = new.library.iter().find(|p| p.id == s.id).map(|p| &**p);
            b += symbol_bytes(s, peer);
        }
    }
    b
}

/// Estimated total bytes of a document (as if nothing were shared). Used for reporting.
pub fn document_bytes(doc: &Document) -> usize {
    let empty = Document { stage: doc.stage, scenes: Vec::new(), library: Vec::new(), bitmaps: Vec::new(), sounds: Vec::new(), library_tree: Default::default(), document_class: String::new(), publish: Default::default(), next_id: doc.next_id };
    unique_bytes(doc, &empty)
}
