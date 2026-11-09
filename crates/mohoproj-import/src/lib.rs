//! Moho document → AniCraft document (`.mohoproj` bare JSON, `.moho` zip package).
//!
//! What maps (stage 1 of the Moho parity plan): stage size, frame rate and background; the
//! inline layer tree flattened to AniCraft's list with parent links (Moho stores layers
//! bottom-first, AniCraft's index 0 is the top); layer types (Mesh/Image/Text layers, Group/
//! Bone/Switch containers as folders); static layer transforms baked into element transforms;
//! mesh shapes as paths (solid, linear and radial gradient fills, solid strokes); text layers
//! laid out with this computer's fonts (`text`, ADR 0014); image layers whose file travels in
//! the package or sits beside it. Everything else — bones, channels with keyframes, switch
//! animation, layer effect stacks, reflected/angle gradients, physics — is counted in the
//! report instead of guessed at, and the untouched original JSON is kept on the document as
//! the `maop_maohou` sidecar so later milestones can re-project it without the source file.
//!
//! Curve facts used here are corpus-derived (maop-corpus, Moho Pro 14.4 native saves):
//! `layers[]` is bottom-first (CharWalk: right-limb layers before Body/Head, pupils after
//! eyes); a shape's boundary lists `(curve, segment, flag)` runs where `segment` indexes the
//! curve's segments and the entries chain into one outline; a closed curve of n points has n
//! segments; coordinates are inches, y up, origin at the canvas centre, at the Moho scale of
//! 320 px/inch (CharWalk's character measures ~1.1 inches on a 432 px canvas).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod anim;
pub mod channel;
pub mod import;
pub mod json;
pub mod layer;
pub mod mesh;
pub mod text;
pub mod zip;

pub use import::{import_path, import_with, import_with_fonts, Reader, Report, PROJECT_ENTRY};
pub use zip::ZipError;

#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod tests;

#[derive(Debug, thiserror::Error)]
pub enum MohoError {
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Zip(#[from] ZipError),
    #[error("invalid JSON: {0}")]
    Json(String),
    #[error("this isn't a Moho document: {0}")]
    NotMoho(String),
}
