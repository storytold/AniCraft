//! Sandboxed WebAssembly plug-ins for AniCraft (layer L0; ADR 0017, `docs/plugins.md`).
//!
//! A plug-in is a core WebAssembly module that implements the AniCraft plug-in ABI (v1). It
//! runs in [`wasmi`], a pure-Rust interpreter, with:
//!
//! - **no host imports but the AniCraft ABI**: the module may import only `anicraft.call` and
//!   `anicraft.response`, a JSON request/response channel to its [`Host`]. No WASI, file
//!   system, network, clock or randomness; a module importing anything else is rejected;
//! - an **instruction budget** (fuel), metered in slices so a wall-clock deadline is enforced
//!   too (native builds), so endless loops end with an error;
//! - a **linear-memory cap**, a **call-depth cap** and wasmi's strict module limits;
//! - **bounded traffic**: request, response, log and host-call counts are capped;
//! - a **fresh instance per run**, so no state survives between runs.
//!
//! Four kinds share the host (ADR 0017, 0024, 0088): commands, frame scripts, exporters
//! ([`Plugin::export`]: frames in, bytes out) and eases (sampled once at load,
//! [`Plugin::eases`]).
//!
//! This crate knows nothing about documents: the engine implements [`Host`] over its command
//! registry. Every failure (malformed module, trap, exhausted budget, bad output) is an
//! [`Error`]; nothing a module does can panic or hang the host.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod manifest;
pub mod registry;
mod runtime;

pub use manifest::{EaseDecl, ExportFormat, Kind, Manifest, ParamKind, ParamSpec, SelectionNeed};
pub use registry::{FolderReport, Registry};
pub use runtime::{ABI_VERSION, EASE_SAMPLES, IMPORT_MODULE, Limits, MAX_EASE_VALUE, Plugin, RunReport, SampledCurve};

use serde_json::Value;

/// A plug-in error. Every way a module can misbehave ends here, never in a panic.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("not a valid plug-in module: {0}")]
    Module(String),
    #[error("plug-in ABI: {0}")]
    Abi(String),
    #[error("plug-in manifest: {0}")]
    Manifest(String),
    #[error("plug-in stopped with an error: {0}")]
    Trap(String),
    #[error("plug-in exceeded its {0}")]
    Limit(String),
    #[error("plug-in parameters: {0}")]
    Params(String),
    #[error("plug-in failed: {0}")]
    Failed(String),
    #[error("no plug-in “{0}” is installed")]
    NotFound(String),
    #[error("{0}")]
    Io(String),
    #[error("cancelled")]
    Cancelled,
}

pub type Result<T> = std::result::Result<T, Error>;

/// What a running plug-in talks to. The runtime parses each `anicraft.call` request, handles
/// `log` itself and hands every other operation here; the answer goes back to the plug-in as
/// `{"ok": true, "result": …}` or `{"ok": false, "error": "…"}`.
pub trait Host {
    /// Handles one request: `op` is its `"op"` field, `request` the whole JSON object.
    fn request(&mut self, op: &str, request: &Value) -> std::result::Result<Value, String>;

    /// Whether the run should stop now (an export's Cancel): checked at every request and
    /// every slice of fuel, so even a busy loop stops within milliseconds.
    fn cancelled(&self) -> bool {
        false
    }
}
