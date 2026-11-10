//! The engine inside WebAssembly (ADR 0036), as the web editor runs it: one thread, no file
//! system, no `std::time` clock. Run in Node by wasm-bindgen's test runner (docs/web.md):
//!
//! ```sh
//! CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner \
//!     cargo test -p anicraft-engine --target wasm32-unknown-unknown --test wasm
//! ```
//!
//! A build can't catch what traps at run time (a clock or thread call that compiles but isn't
//! there); these do. Natively this file is empty.
#![cfg(target_arch = "wasm32")]

use anicraft_engine::plugins::{Host, Limits, Plugin};
use anicraft_engine::render_service::RenderService;
use anicraft_engine::{Session, vfs};
use serde_json::{Map, Value, json};
use std::path::Path;
use wasm_bindgen_test::wasm_bindgen_test;

fn names() -> Vec<String> {
    vfs::take_written().into_iter().map(|(p, _)| p.display().to_string()).collect()
}

#[wasm_bindgen_test]
fn saves_opens_and_exports_through_memory() {
    assert!(vfs::in_memory(), "the browser has no disk: files live in memory");
    let mut s = Session::new(anicraft_engine::doc::sample::demo());
    s.execute("file.save", &json!({"path": "walk.anicraft"})).unwrap();
    let saved = vfs::take_written();
    assert_eq!(saved.len(), 1);
    vfs::put(Path::new("walk.anicraft"), saved[0].1.clone()).unwrap();
    let mut t = Session::default();
    t.execute("file.open", &json!({"path": "walk.anicraft"})).unwrap();
    assert_eq!(t.frame_count(), s.frame_count());
    // Parallel renders (rayon on the one thread), timing (web clock), encoders.
    s.execute("file.export_png", &json!({"path": "f.png"})).unwrap();
    s.execute("file.export_gif", &json!({"path": "m.gif", "to": 4})).unwrap();
    s.execute("file.export_html", &json!({"path": "m.html"})).unwrap();
    s.execute("file.export_png_sequence", &json!({"dir": "seq", "to": 2})).unwrap();
    assert_eq!(names().len(), 6);
    assert!(s.execute("file.export_video", &json!({"path": "m.webm"})).is_err());
}

#[wasm_bindgen_test]
fn the_stage_renders_without_a_thread() {
    let s = Session::new(anicraft_engine::doc::sample::demo());
    let svc = RenderService::new(64 << 20, None);
    let view = anicraft_engine::render::View::stage(&s.doc, 0.5);
    // No worker thread can start, so a request renders right away.
    let img = svc.request(svc.job(s.state, &s.doc, 3, view, false)).expect("rendered on this thread");
    assert!(img.error.is_none() && img.width > 0 && img.rgba_premul.iter().any(|&b| b != 0));
}

struct Quiet;
impl Host for Quiet {
    fn request(&mut self, _: &str, _: &Value) -> Result<Value, String> {
        Ok(Value::Null)
    }
}

#[wasm_bindgen_test]
fn plugins_run_in_the_browser_too() {
    let manifest = r#"{"id":"org.test.web","name":"Web","version":"1.0.0","kind":"command","author":"Tests"}"#;
    let text = format!(
        r#"(module
  (import "anicraft" "call" (func (param i32 i32) (result i32)))
  (import "anicraft" "response" (func (param i32 i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 1024) "{manifest}")
  (func (export "ac_abi_version") (result i32) i32.const 1)
  (func (export "ac_manifest") (result i64) (i64.or (i64.shl (i64.const {len}) (i64.const 32)) (i64.const 1024)))
  (func (export "ac_alloc") (param i32) (result i32) i32.const 4096)
  (func (export "ac_run") (param i32 i32) (result i32) i32.const 0))"#,
        manifest = manifest.replace('"', "\\22"),
        len = manifest.len()
    );
    let plugin = Plugin::load(&wat::parse_str(&text).unwrap(), Limits::default()).unwrap();
    assert_eq!(plugin.id(), "org.test.web");
    plugin.run(&mut Quiet, &Map::new()).unwrap();
}
