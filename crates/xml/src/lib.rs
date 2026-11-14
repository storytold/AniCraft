//! Compatibility path for AniCraft's shared XML reader.
//!
//! The bounded parser is maintained by `craft-xfl`; this crate keeps the existing AniCraft
//! import paths stable while allowing SVG and XFL consumers to share one implementation.

pub use craft_xfl::xml::*;
