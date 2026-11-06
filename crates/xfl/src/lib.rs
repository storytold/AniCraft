//! AniCraft XFL import (between `doc` and `engine`): opens Animate documents, as an XFL folder
//! or a `.fla` archive, into the AniCraft document model.
//!
//! Clean room: written from the publicly documented structure of XFL (an XML package:
//! `DOMDocument.xml`, `LIBRARY/*.xml`) and the published ZIP format. No Animate code, and no
//! third-party converter code. The test corpus is self-authored.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod edges;
pub mod import;
/// The shared XML reader (`anicraft-xml`), under its old path.
pub use anicraft_xml as xml;
pub mod zip;

pub use import::{Report, XflError, import_path, import_with};

#[cfg(test)]
mod tests;
