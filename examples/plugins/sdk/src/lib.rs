//! Guest-side helpers for AniCraft WebAssembly plug-ins (ABI v1, `docs/plugins.md`).
//!
//! A command plug-in names its manifest and its `run` function with [`plugin!`] and talks to
//! AniCraft with [`command`], [`document`], [`selected_elements`] and [`log`]. An exporter
//! names its export function with [`exporter!`] and reads frames with [`movie`] and [`frame`];
//! an ease plug-in names its ease function with [`ease!`]. The only `unsafe` a plug-in needs is
//! here: the two host imports and the raw pointers the exports receive. Everything else is
//! plain Rust; `std` is fine as long as nothing touches WASI (files, clock, randomness), since
//! AniCraft refuses modules with any other import.
//!
//! Built for anything but WebAssembly (a plug-in's own `cargo test`), the host imports are
//! stand-ins that answer every request with an error, so the pure parts can be unit-tested.

use serde_json::{Value, json};

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "anicraft")]
unsafe extern "C" {
    /// Sends `len` bytes of JSON at `ptr`; returns the length of the response.
    #[link_name = "call"]
    fn host_call(ptr: *const u8, len: usize) -> i32;
    /// Copies up to `cap` bytes of the last response to `ptr`; returns the bytes copied.
    #[link_name = "response"]
    fn host_response(ptr: *mut u8, cap: usize) -> i32;
}

/// Outside AniCraft (native unit tests) there is no host: no response.
#[cfg(not(target_arch = "wasm32"))]
unsafe fn host_call(_: *const u8, _: usize) -> i32 {
    0
}
#[cfg(not(target_arch = "wasm32"))]
unsafe fn host_response(_: *mut u8, _: usize) -> i32 {
    0
}

/// Sends one request (`{"op": …}`) and returns its result, or the host's error message.
pub fn request(req: &Value) -> Result<Value, String> {
    let bytes = serde_json::to_vec(req).map_err(|e| e.to_string())?;
    // SAFETY: the host reads `bytes.len()` bytes at `bytes.as_ptr()`, which `bytes` owns
    // for the whole call.
    let n = unsafe { host_call(bytes.as_ptr(), bytes.len()) };
    let mut buf = vec![0u8; n.max(0) as usize];
    // SAFETY: the host writes at most `buf.len()` bytes into `buf`.
    let got = unsafe { host_response(buf.as_mut_ptr(), buf.len()) };
    buf.truncate(got.max(0) as usize);
    let v: Value = serde_json::from_slice(&buf).map_err(|e| format!("unreadable response: {e}"))?;
    if v["ok"] == true { Ok(v["result"].clone()) } else { Err(v["error"].as_str().unwrap_or("the host refused the request").to_string()) }
}

/// Runs an AniCraft command by id (`draw.star`, `element.translate`, …; see
/// `anicraft-cli commands`).
pub fn command(id: &str, params: Value) -> Result<Value, String> {
    request(&json!({"op": "command", "id": id, "params": params})).map_err(|e| format!("{id}: {e}"))
}

/// The compact document view: stage, scene, frame, layers and the selection.
pub fn document() -> Result<Value, String> {
    request(&json!({"op": "document"}))
}

/// The selected elements on the active layer at the playhead (ids, kinds, stage bounds,
/// transforms; with `paths`, shape outlines as SVG path data).
pub fn selected_elements(paths: bool) -> Result<Vec<Value>, String> {
    let v = request(&json!({"op": "elements", "selected": true, "paths": paths}))?;
    Ok(v["elements"].as_array().cloned().unwrap_or_default())
}

/// A line in the run's report (and the error message if the run fails).
pub fn log(message: &str) {
    let _ = request(&json!({"op": "log", "message": message}));
}

/// An exporter's movie: `{name, width, height, fps, background, from, to, frames,
/// total_frames, scenes}` (frames are 0-based movie frames).
pub fn movie() -> Result<Value, String> {
    request(&json!({"op": "movie"}))
}

/// Frame `index` of the export (0 = the first frame exported), evaluated: `{index, frame,
/// scene, scene_frame, items: [{layer, element, path, transform, fill, stroke, fill_rule,
/// alpha, color?, clip?, masks?, group?}], clips, groups}`.
pub fn frame(index: u64) -> Result<Value, String> {
    request(&json!({"op": "frame", "index": index}))
}

/// A small deterministic random generator (SplitMix64). Plug-ins have no source of
/// randomness, by design: a seed parameter makes every run, and every preview, repeatable.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed ^ 0x9e37_79b9_7f4a_7c15)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    /// Uniform in `0..1`.
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    /// Uniform in `-1..1`.
    pub fn signed(&mut self) -> f64 {
        self.unit() * 2.0 - 1.0
    }
}

/// Exports the ABI v1 entry points for a plug-in: `plugin!(MANIFEST, run)` where `MANIFEST`
/// is the manifest JSON and `run: fn(&serde_json::Value) -> Result<(), String>` gets the
/// validated parameters.
#[macro_export]
macro_rules! plugin {
    ($manifest:expr, $run:path) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn ac_abi_version() -> i32 {
            1
        }
        #[unsafe(no_mangle)]
        pub extern "C" fn ac_manifest() -> i64 {
            $crate::manifest_ptr($manifest)
        }
        #[unsafe(no_mangle)]
        pub extern "C" fn ac_alloc(size: i32) -> i32 {
            $crate::alloc(size)
        }
        #[unsafe(no_mangle)]
        pub extern "C" fn ac_run(ptr: i32, len: i32) -> i32 {
            $crate::run(ptr, len, $run)
        }
    };
}

/// Exports the entry points of an exporter plug-in: `exporter!(MANIFEST, export)` where
/// `export: fn(&serde_json::Value) -> Result<Vec<u8>, String>` gets the validated options and
/// returns the file's bytes.
#[macro_export]
macro_rules! exporter {
    ($manifest:expr, $export:path) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn ac_abi_version() -> i32 {
            1
        }
        #[unsafe(no_mangle)]
        pub extern "C" fn ac_manifest() -> i64 {
            $crate::manifest_ptr($manifest)
        }
        #[unsafe(no_mangle)]
        pub extern "C" fn ac_alloc(size: i32) -> i32 {
            $crate::alloc(size)
        }
        #[unsafe(no_mangle)]
        pub extern "C" fn ac_export(ptr: i32, len: i32) -> i64 {
            $crate::export(ptr, len, $export)
        }
    };
}

/// Exports the entry points of an ease plug-in: `ease!(MANIFEST, ease)` where
/// `ease: fn(usize, f64) -> f64` is the eased progress of the manifest's ease number `index` at
/// time `t` (0..=1). AniCraft samples it when the plug-in loads; it gets no host calls.
#[macro_export]
macro_rules! ease {
    ($manifest:expr, $ease:path) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn ac_abi_version() -> i32 {
            1
        }
        #[unsafe(no_mangle)]
        pub extern "C" fn ac_manifest() -> i64 {
            $crate::manifest_ptr($manifest)
        }
        #[unsafe(no_mangle)]
        pub extern "C" fn ac_ease(index: i32, t: f64) -> f64 {
            $ease(index.max(0) as usize, t)
        }
    };
}

#[doc(hidden)]
pub fn export(ptr: i32, len: i32, f: fn(&Value) -> Result<Vec<u8>, String>) -> i64 {
    // SAFETY: the host wrote `len` bytes of options into the block it got from `ac_alloc`.
    let bytes = unsafe { std::slice::from_raw_parts(ptr as usize as *const u8, len.max(0) as usize) };
    let options: Value = serde_json::from_slice(bytes).unwrap_or(Value::Null);
    match f(&options) {
        Ok(out) => {
            // The instance is dropped after the export, so the file is never freed.
            let out = std::mem::ManuallyDrop::new(out);
            ((out.len() as i64) << 32) | (out.as_ptr() as usize as i64)
        }
        Err(e) => {
            log(&e);
            -1
        }
    }
}

#[doc(hidden)]
pub fn manifest_ptr(m: &'static str) -> i64 {
    ((m.len() as i64) << 32) | (m.as_ptr() as usize as i64)
}

#[doc(hidden)]
pub fn alloc(size: i32) -> i32 {
    // A fresh instance runs once and is dropped, so the block is never freed.
    let mut v = std::mem::ManuallyDrop::new(Vec::<u8>::with_capacity(size.max(1) as usize));
    v.as_mut_ptr() as usize as i32
}

#[doc(hidden)]
pub fn run(ptr: i32, len: i32, f: fn(&Value) -> Result<(), String>) -> i32 {
    // SAFETY: the host wrote `len` bytes of parameters into the block it got from `ac_alloc`.
    let bytes = unsafe { std::slice::from_raw_parts(ptr as usize as *const u8, len.max(0) as usize) };
    let params: Value = serde_json::from_slice(bytes).unwrap_or(Value::Null);
    match f(&params) {
        Ok(()) => 0,
        Err(e) => {
            log(&e);
            1
        }
    }
}
