//! Path de-duplication for serialization.
//!
//! In memory, a shape tweened across many keyframes shares one `Arc<Path>`. Serialized naively,
//! every keyframe would repeat the path. Inside [`write_with_paths`], each shared path is written
//! once to a table (built by [`build_table`]) and shapes refer to it by index. Inside a
//! [`begin_read`] scope the references resolve back to shared `Arc`s, so loading restores the
//! sharing (and its memory savings) too.
//! Outside those scopes, paths serialize inline, so documents embedded in other JSON still work.

use crate::{Document, ElementKind};
use anicraft_geom::Path;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

enum Ctx {
    Write { index: HashMap<usize, u32>, paths: Vec<Arc<Path>> },
    Read { paths: Vec<Arc<Path>> },
}

thread_local! {
    static CTX: RefCell<Option<Ctx>> = const { RefCell::new(None) };
}

/// Clears the context even if serialization unwinds.
struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        CTX.with(|c| c.replace(None));
    }
}

/// Every distinct path of `doc` (scenes and library), in first-use order. Walking the
/// tree up front lets a writer emit the table *before* the document, so readers can stream.
pub fn build_table(doc: &Document) -> Vec<Arc<Path>> {
    let mut seen: HashMap<usize, ()> = HashMap::new();
    let mut out = Vec::new();
    let timelines = doc.timelines();
    for tl in timelines {
        for layer in &tl.layers {
            for k in &layer.keyframes {
                for e in &k.elements {
                    // A warped shape's outline as drawn (ADR 0079) is written once too.
                    if let ElementKind::Shape(s) = &e.kind
                        && let Some(w) = &s.warp
                        && seen.insert(Arc::as_ptr(&w.rest) as usize, ()).is_none()
                    {
                        out.push(w.rest.clone());
                    }
                    let path = match &e.kind {
                        ElementKind::Shape(s) => &s.path,
                        ElementKind::Text(t) => &t.outline,
                        ElementKind::Symbol { .. } | ElementKind::Camera | ElementKind::Group { .. } => continue,
                    };
                    if seen.insert(Arc::as_ptr(path) as usize, ()).is_none() {
                        out.push(path.clone());
                    }
                }
            }
        }
    }
    out
}

/// Runs `f` (which serializes a document) with shapes referring to `paths` by index.
pub fn write_with_paths<R>(paths: &[Arc<Path>], f: impl FnOnce() -> R) -> R {
    let index = paths.iter().enumerate().map(|(i, p)| (Arc::as_ptr(p) as usize, i as u32)).collect();
    CTX.with(|c| c.replace(Some(Ctx::Write { index, paths: paths.to_vec() })));
    let _guard = Guard;
    f()
}

/// Scope in which deserialized path references resolve against a table. Clears on drop.
pub struct ReadScope(Guard);

/// Makes `paths` available to path references until the returned scope is dropped.
pub fn begin_read(paths: Vec<Arc<Path>>) -> ReadScope {
    CTX.with(|c| c.replace(Some(Ctx::Read { paths })));
    ReadScope(Guard)
}

pub fn serialize<S: Serializer>(p: &Arc<Path>, s: S) -> Result<S::Ok, S::Error> {
    let idx = CTX.with(|c| match &mut *c.borrow_mut() {
        Some(Ctx::Write { index, paths }) => {
            let key = Arc::as_ptr(p) as usize;
            Some(*index.entry(key).or_insert_with(|| {
                paths.push(p.clone());
                (paths.len() - 1) as u32
            }))
        }
        _ => None,
    });
    match idx {
        Some(i) => s.serialize_u32(i),
        None => p.serialize(s),
    }
}

pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Arc<Path>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Repr {
        Ref(u32),
        Inline(Path),
    }
    match Repr::deserialize(d)? {
        Repr::Inline(p) => Ok(Arc::new(p)),
        Repr::Ref(i) => CTX.with(|c| match &*c.borrow() {
            Some(Ctx::Read { paths }) => paths.get(i as usize).cloned().ok_or_else(|| serde::de::Error::custom(format!("path reference {i} is out of range"))),
            _ => Err(serde::de::Error::custom("path reference outside a path table")),
        }),
    }
}
