//! Shared bitmap references in the native format. Outside a table scope, embed PNG inline.

use crate::{Bitmap, Document, ElementKind, Paint};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

enum Ctx {
    Write { index: HashMap<usize, u32>, images: Vec<Arc<Bitmap>> },
    Read { images: Vec<Arc<Bitmap>> },
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

/// Every distinct image of `doc` (main timeline and library), in first-use order. Walking the
/// tree up front lets a writer emit the table *before* the document, so readers can stream.
pub fn build_table(doc: &Document) -> Vec<Arc<Bitmap>> {
    let mut seen: HashMap<usize, ()> = HashMap::new();
    let mut out = Vec::new();
    let timelines = std::iter::once(&doc.timeline).chain(doc.library.iter().map(|s| &s.timeline));
    for tl in timelines {
        for layer in &tl.layers {
            for k in &layer.keyframes {
                for e in &k.elements {
                    if let ElementKind::Shape(s) = &e.kind
                        && let Some(Paint::Bitmap { image, .. }) = &s.fill
                        && seen.insert(Arc::as_ptr(image) as usize, ()).is_none()
                    {
                        out.push(image.clone());
                    }
                }
            }
        }
    }
    out
}

/// Runs `f` (which serializes a document) with shapes referring to `images` by index.
pub fn write_with_images<R>(images: &[Arc<Bitmap>], f: impl FnOnce() -> R) -> R {
    let index = images.iter().enumerate().map(|(i, p)| (Arc::as_ptr(p) as usize, i as u32)).collect();
    CTX.with(|c| c.replace(Some(Ctx::Write { index, images: images.to_vec() })));
    let _guard = Guard;
    f()
}

/// Scope in which deserialized image references resolve against a table. Clears on drop.
pub struct ReadScope(Guard);

/// Makes `images` available to image references until the returned scope is dropped.
pub fn begin_read(images: Vec<Arc<Bitmap>>) -> ReadScope {
    CTX.with(|c| c.replace(Some(Ctx::Read { images })));
    ReadScope(Guard)
}

/// Decode a table incrementally with a total pixel-memory budget, including compressed bombs.
pub fn read_table<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Arc<Bitmap>>, D::Error> {
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = Vec<Arc<Bitmap>>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { f.write_str("a bounded bitmap table") }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut images = Vec::new();
            let mut bytes = 0usize;
            while let Some(image) = seq.next_element::<Bitmap>()? {
                bytes += image.bytes();
                if bytes > 256 * 1024 * 1024 || images.len() >= 4096 {
                    return Err(serde::de::Error::custom("bitmap table exceeds 256 MiB or 4096 assets"));
                }
                images.push(Arc::new(image));
            }
            Ok(images)
        }
    }
    d.deserialize_seq(Visitor)
}

pub fn serialize<S: Serializer>(p: &Arc<Bitmap>, s: S) -> Result<S::Ok, S::Error> {
    let idx = CTX.with(|c| match &mut *c.borrow_mut() {
        Some(Ctx::Write { index, images }) => {
            let key = Arc::as_ptr(p) as usize;
            Some(*index.entry(key).or_insert_with(|| {
                images.push(p.clone());
                (images.len() - 1) as u32
            }))
        }
        _ => None,
    });
    match idx {
        Some(i) => s.serialize_u32(i),
        None => p.serialize(s),
    }
}

pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Arc<Bitmap>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Repr {
        Ref(u32),
        Inline(Bitmap),
    }
    match Repr::deserialize(d)? {
        Repr::Inline(p) => CTX.with(|c| match &*c.borrow() {
            Some(Ctx::Read { .. }) => Err(serde::de::Error::custom("native documents must reference the bitmap table")),
            _ => Ok(Arc::new(p)),
        }),
        Repr::Ref(i) => CTX.with(|c| match &*c.borrow() {
            Some(Ctx::Read { images }) => images.get(i as usize).cloned().ok_or_else(|| serde::de::Error::custom(format!("image reference {i} is out of range"))),
            _ => Err(serde::de::Error::custom("image reference outside an image table")),
        }),
    }
}
