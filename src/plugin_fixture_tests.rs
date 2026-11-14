//! Test fixtures for the shell's plug-in tests: an exporter and an ease plug-in written in
//! WebAssembly text (compiled by `wat`, so no binaries are committed), installed into an app.

use crate::app::App;
use serde_json::json;

fn esc(s: &str) -> String {
    s.bytes().map(|b| if b == b'"' || b == b'\\' || !(0x20..0x7f).contains(&b) { format!("\\{b:02x}") } else { (b as char).to_string() }).collect()
}

/// A module: `manifest` at 1024, `body` as its last function, the imports and `ac_alloc`.
fn module(manifest: &str, body: &str) -> Vec<u8> {
    let text = format!(
        r#"(module
  (import "anicraft" "call" (func $call (param i32 i32) (result i32)))
  (import "anicraft" "response" (func $response (param i32 i32) (result i32)))
  (memory (export "memory") 8)
  (data (i32.const 1024) "{}")
  (data (i32.const 8192) "{}")
  (func (export "ac_abi_version") (result i32) i32.const 1)
  (func (export "ac_manifest") (result i64) (i64.or (i64.shl (i64.const {}) (i64.const 32)) (i64.const 1024)))
  (func (export "ac_alloc") (param i32) (result i32) i32.const 60000)
  {body})"#,
        esc(manifest),
        esc(r#"{"op":"frame","index":0}"#),
        manifest.len()
    );
    wat::parse_str(&text).unwrap_or_else(|e| panic!("bad test WAT: {e}\n{text}"))
}

/// The id of [`exporter`] (and of [`endless_exporter`]).
pub const EXPORTER: &str = "org.test.dump";

/// An exporter whose file is its answer to the first frame request; with `endless`, one that
/// keeps asking for that frame (until Cancel).
pub fn exporter(endless: bool) -> Vec<u8> {
    let m = r#"{"id":"org.test.dump","name":"Frame Dump","kind":"exporter","description":"Writes the first frame","format":{"name":"Frame Dump","extension":"dump"},"params":{"mode":{"type":"choice","options":["a","b"],"default":"a"}}}"#;
    let ask = "(local.set $n (call $call (i32.const 8192) (i32.const 24)))";
    let body = if endless { format!("(loop $l {ask} (br $l))") } else { ask.to_string() };
    module(
        m,
        &format!(
            r#"(func (export "ac_export") (param i32 i32) (result i64) (local $n i32)
    {body}
    (local.set $n (call $response (i32.const 100000) (local.get $n)))
    (i64.or (i64.shl (i64.extend_i32_u (local.get $n)) (i64.const 32)) (i64.const 100000)))"#
        ),
    )
}

/// The id of [`eases`].
pub const EASES: &str = "org.test.steps";

/// An ease plug-in with two eases: four steps, and the straight line.
pub fn eases() -> Vec<u8> {
    let m = r#"{"id":"org.test.steps","name":"Test Steps","kind":"ease","eases":[{"id":"steps4","name":"Stepped 4"},{"id":"line","name":"Straight"}]}"#;
    module(m, r#"(func (export "ac_ease") (param $i i32) (param $t f64) (result f64) (if (result f64) (i32.eqz (local.get $i)) (then (f64.div (f64.floor (f64.mul (local.get $t) (f64.const 4))) (f64.const 4))) (else (local.get $t))))"#)
}

/// A scratch folder for one test.
pub fn temp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("anicraft-shell-plugins-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("temp dir");
    d
}

/// Installs `bytes` as a plug-in.
pub fn install(app: &mut App, dir: &std::path::Path, file: &str, bytes: &[u8]) {
    let path = dir.join(file);
    std::fs::write(&path, bytes).expect("write module");
    app.s.execute("plugin.install", &json!({"path": path.display().to_string()})).expect("installs");
}
