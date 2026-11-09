//! ABI v1 and sandbox tests, on modules written in WebAssembly text (compiled by `wat`), so no
//! binary fixtures are committed. Hostile modules must end in an error, quickly, every time.

use anicraft_plugins::{Error, Host, Limits, Plugin, Registry};
use serde_json::{Map, Value, json};
use std::time::{Duration, Instant};

const MANIFEST: &str = r#"{"id":"org.test.probe","name":"Probe","version":"1.0.0","kind":"command","author":"Tests","params":{"count":{"type":"int","min":1,"max":10,"default":2}}}"#;

/// WAT string literal contents for `s`.
fn esc(s: &str) -> String {
    s.bytes().map(|b| if b == b'"' || b == b'\\' || !(0x20..0x7f).contains(&b) { format!("\\{b:02x}") } else { (b as char).to_string() }).collect()
}

/// A plug-in module: the manifest at 1024, `data` strings at their offsets, `body` as
/// `ac_run`'s code (params: $p $len; locals $n $m), `extra` module fields.
fn module_with(manifest: &str, data: &[(u32, &str)], body: &str, extra: &str) -> Vec<u8> {
    let mut d = format!("(data (i32.const 1024) \"{}\")\n", esc(manifest));
    for (at, s) in data {
        d.push_str(&format!("(data (i32.const {at}) \"{}\")\n", esc(s)));
    }
    let text = format!(
        r#"(module
  (import "anicraft" "call" (func $call (param i32 i32) (result i32)))
  (import "anicraft" "response" (func $response (param i32 i32) (result i32)))
  (memory (export "memory") 2)
  (global $heap (mut i32) (i32.const 65536))
  {d}
  (func (export "ac_abi_version") (result i32) i32.const 1)
  (func (export "ac_manifest") (result i64)
    (i64.or (i64.shl (i64.const {len}) (i64.const 32)) (i64.const 1024)))
  (func (export "ac_alloc") (param $size i32) (result i32) (local $p i32)
    global.get $heap local.set $p
    (global.set $heap (i32.add (global.get $heap) (local.get $size)))
    local.get $p)
  (func (export "ac_run") (param $p i32) (param $len i32) (result i32) (local $n i32) (local $m i32)
    {body})
  {extra})"#,
        len = manifest.len()
    );
    wat::parse_str(&text).unwrap_or_else(|e| panic!("bad test WAT: {e}\n{text}"))
}

fn module(data: &[(u32, &str)], body: &str) -> Vec<u8> {
    module_with(MANIFEST, data, body, "")
}

/// A host that records requests and answers `echo` with its request.
#[derive(Default)]
struct Rec {
    ops: Vec<String>,
    delay: Option<Duration>,
}

impl Host for Rec {
    fn request(&mut self, op: &str, req: &Value) -> Result<Value, String> {
        self.ops.push(op.to_string());
        if let Some(d) = self.delay {
            std::thread::sleep(d);
        }
        match op {
            "echo" => Ok(req.clone()),
            _ => Err(format!("unknown op '{op}'")),
        }
    }
}

fn run(bytes: &[u8], limits: Limits) -> (Result<anicraft_plugins::RunReport, Error>, Rec, Duration) {
    let p = Plugin::load(bytes, limits).expect("loads");
    let mut host = Rec::default();
    let t = Instant::now();
    let r = p.run(&mut host, &p.manifest().defaults());
    (r, host, t.elapsed())
}

fn small() -> Limits {
    Limits { fuel: 20_000_000, wall_time_ms: 2_000, ..Limits::default() }
}

#[test]
fn a_plugin_loads_reads_its_params_and_talks_to_the_host() {
    // Sends a log line and an echo request, reads the echo back and checks its envelope.
    let log = r#"{"op":"log","message":"hello"}"#;
    let echo = r#"{"op":"echo","x":1}"#;
    let body = format!(
        r#"
    (drop (call $call (i32.const 4096) (i32.const {})))
    (local.set $n (call $call (i32.const 8192) (i32.const {})))
    (local.set $m (call $response (i32.const 16384) (local.get $n)))
    (if (i32.ne (local.get $m) (local.get $n)) (then (return (i32.const 2))))
    ;; {{"ok":true,...}}
    (if (i32.ne (i32.load8_u (i32.const 16390)) (i32.const 116)) (then (return (i32.const 3))))
    ;; the params arrived as JSON: {{"count":2}}
    (if (i32.ne (i32.load8_u (local.get $p)) (i32.const 123)) (then (return (i32.const 4))))
    (if (i32.ne (local.get $len) (i32.const 11)) (then (return (i32.const 5))))
    i32.const 0"#,
        log.len(),
        echo.len()
    );
    let bytes = module(&[(4096, log), (8192, echo)], &body);
    let p = Plugin::load(&bytes, Limits::default()).unwrap();
    assert_eq!(p.manifest().name, "Probe");
    assert_eq!(p.manifest().params.len(), 1);
    let mut host = Rec::default();
    let report = p.run(&mut host, &p.manifest().defaults()).unwrap();
    assert_eq!(report.log, ["hello"]);
    assert_eq!(report.calls, 2);
    assert_eq!(host.ops, ["echo"]);
    assert!(report.fuel > 0 && report.fuel < 10_000, "fuel {}", report.fuel);
}

#[test]
fn bad_requests_get_an_error_answer_and_the_run_goes_on() {
    let body = r#"
    (local.set $n (call $call (i32.const 4096) (i32.const 3)))
    (drop (call $response (i32.const 16384) (local.get $n)))
    ;; {"ok":false
    (if (i32.ne (i32.load8_u (i32.const 16390)) (i32.const 102)) (then (return (i32.const 9))))
    (drop (call $call (i32.const 5000) (i32.const 2)))
    (drop (call $call (i32.const 6000) (i32.const 13)))
    i32.const 0"#;
    let (r, host, _) = run(&module(&[(4096, "nop"), (5000, "{}"), (6000, r#"{"op":"what"}"#)], body), Limits::default());
    r.unwrap();
    assert_eq!(host.ops, ["what"]);
}

#[test]
fn a_nonzero_return_is_a_failure_with_the_last_log_line() {
    let msg = r#"{"op":"log","message":"select something first"}"#;
    let body = format!("(drop (call $call (i32.const 4096) (i32.const {}))) i32.const 7", msg.len());
    let (r, _, _) = run(&module(&[(4096, msg)], &body), Limits::default());
    let e = r.unwrap_err();
    assert!(matches!(&e, Error::Failed(m) if m.contains("code 7") && m.contains("select something first")), "{e}");
}

#[test]
fn modules_are_validated_at_load() {
    let ok = module(&[], "i32.const 0");
    Plugin::load(&ok, Limits::default()).unwrap();
    let wat = |t: &str| wat::parse_str(t).unwrap();
    let base_exports = format!(
        r#"(memory (export "memory") 1) (data (i32.const 1024) "{}")
        (func (export "ac_abi_version") (result i32) i32.const 1)
        (func (export "ac_manifest") (result i64) (i64.or (i64.shl (i64.const {}) (i64.const 32)) (i64.const 1024)))
        (func (export "ac_alloc") (param i32) (result i32) i32.const 4096)"#,
        esc(MANIFEST),
        MANIFEST.len()
    );
    let cases: Vec<(&str, Vec<u8>, &str)> = vec![
        ("not wasm", b"MZ\x90\x00 definitely not wasm".to_vec(), "\\0asm"),
        ("truncated", ok[..ok.len() / 2].to_vec(), "valid"),
        ("wasi import", wat(&format!(r#"(module (import "wasi_snapshot_preview1" "fd_write" (func (param i32 i32 i32 i32) (result i32))) {base_exports} (func (export "ac_run") (param i32 i32) (result i32) i32.const 0))"#)), "wasi_snapshot_preview1.fd_write"),
        ("unknown anicraft import", wat(&format!(r#"(module (import "anicraft" "open_file" (func (param i32) (result i32))) {base_exports} (func (export "ac_run") (param i32 i32) (result i32) i32.const 0))"#)), "anicraft.open_file"),
        ("wrong import type", wat(&format!(r#"(module (import "anicraft" "call" (func (param i64) (result i32))) {base_exports} (func (export "ac_run") (param i32 i32) (result i32) i32.const 0))"#)), "must be a function"),
        ("imported memory", wat(r#"(module (import "anicraft" "memory" (memory 1)))"#), "anicraft.memory"),
        ("missing ac_run", wat(&format!("(module {base_exports})")), "ac_run"),
        ("wrong ac_run type", wat(&format!(r#"(module {base_exports} (func (export "ac_run") (result i32) i32.const 0))"#)), "ac_run"),
        ("no memory export", wat(r#"(module (func (export "ac_abi_version") (result i32) i32.const 1))"#), "memory"),
        ("abi 2", wat(&format!(r#"(module {} (func (export "ac_run") (param i32 i32) (result i32) i32.const 0))"#, base_exports.replace("(result i32) i32.const 1)", "(result i32) i32.const 2)"))), "returned 2"),
        ("manifest outside memory", wat(r#"(module (memory (export "memory") 1) (func (export "ac_abi_version") (result i32) i32.const 1) (func (export "ac_manifest") (result i64) (i64.or (i64.shl (i64.const 100) (i64.const 32)) (i64.const 65500))) (func (export "ac_alloc") (param i32) (result i32) i32.const 0) (func (export "ac_run") (param i32 i32) (result i32) i32.const 0))"#), "outside"),
        ("huge manifest", wat(r#"(module (memory (export "memory") 1) (func (export "ac_abi_version") (result i32) i32.const 1) (func (export "ac_manifest") (result i64) i64.const 0x7fffffff00000000) (func (export "ac_alloc") (param i32) (result i32) i32.const 0) (func (export "ac_run") (param i32 i32) (result i32) i32.const 0))"#), "limit"),
        ("bad manifest", module_with(r#"{"id":"bad id","name":"X","kind":"command"}"#, &[], "i32.const 0", ""), "invalid id"),
        ("memory over the cap", wat(&format!(r#"(module {} (func (export "ac_run") (param i32 i32) (result i32) i32.const 0))"#, base_exports.replace("(memory (export \"memory\") 1)", "(memory (export \"memory\") 5000)"))), "start"),
        ("endless start function", wat(&format!(r#"(module {base_exports} (func $s (loop $l (br $l))) (start $s) (func (export "ac_run") (param i32 i32) (result i32) i32.const 0))"#)), "instruction budget"),
        ("host call while loading", wat(&format!(r#"(module (import "anicraft" "call" (func $c (param i32 i32) (result i32))) {} (func (export "ac_run") (param i32 i32) (result i32) i32.const 0))"#, base_exports.replace("(result i32) i32.const 1)", "(result i32) (drop (call $c (i32.const 0) (i32.const 0))) i32.const 1)"))), "only while"),
    ];
    for (name, bytes, want) in cases {
        let t = Instant::now();
        let e = Plugin::load(&bytes, small()).expect_err(name);
        assert!(e.to_string().contains(want), "{name}: {e}");
        assert!(t.elapsed() < Duration::from_secs(5), "{name} took {:?}", t.elapsed());
    }
    let big = vec![0u8; 1 << 10];
    assert!(Plugin::load(&big, Limits { max_module_bytes: 100, ..Limits::default() }).is_err());
}


#[test]
fn an_endless_loop_runs_out_of_fuel() {
    let (r, _, t) = run(&module(&[], "(loop $l (br $l)) i32.const 0"), small());
    assert!(matches!(r, Err(Error::Limit(ref m)) if m.contains("instruction")), "{r:?}");
    println!("20 M instructions of a busy loop: {t:?}");
    assert!(t < Duration::from_secs(5));
}

#[test]
fn a_long_run_hits_the_time_budget() {
    let (r, _, t) = run(&module(&[], "(loop $l (br $l)) i32.const 0"), Limits { fuel: u64::MAX / 2, wall_time_ms: 200, ..Limits::default() });
    assert!(matches!(r, Err(Error::Limit(ref m)) if m.contains("time")), "{r:?}");
    assert!(t < Duration::from_secs(2), "{t:?}");
}

#[test]
fn slow_host_calls_count_against_the_time_budget() {
    let log = r#"{"op":"echo"}"#;
    let body = format!("(loop $l (drop (call $call (i32.const 4096) (i32.const {}))) (br $l)) i32.const 0", log.len());
    let p = Plugin::load(&module(&[(4096, log)], &body), Limits { wall_time_ms: 150, ..Limits::default() }).unwrap();
    let mut host = Rec { delay: Some(Duration::from_millis(20)), ..Rec::default() };
    let t = Instant::now();
    let r = p.run(&mut host, &Map::new());
    assert!(matches!(r, Err(Error::Limit(ref m)) if m.contains("time")), "{r:?}");
    assert!(t.elapsed() < Duration::from_secs(2));
}

#[test]
fn memory_growth_stops_at_the_cap() {
    // Grows until memory.grow fails, then reports how far it got as the error code.
    let body = "(block $done (loop $l (br_if $done (i32.eq (memory.grow (i32.const 16)) (i32.const -1))) (br $l))) memory.size";
    let (r, _, _) = run(&module(&[], body), Limits { max_memory_bytes: 8 << 20, ..small() });
    // 8 MiB is 128 pages: from 2 pages in steps of 16 it stops at 114 (130 would be over).
    assert!(matches!(r, Err(Error::Failed(ref m)) if m.contains("code 114")), "{r:?}");
}

#[test]
fn deep_recursion_traps() {
    let extra = r#"(func $down (param i32) (result i32) (call $down (i32.add (local.get 0) (i32.const 1))))"#;
    let (r, _, t) = run(&module_with(MANIFEST, &[], "(call $down (i32.const 0))", extra), small());
    assert!(matches!(r, Err(Error::Trap(_)) | Err(Error::Limit(_))), "{r:?}");
    assert!(t < Duration::from_secs(5));
}

#[test]
fn traps_and_bad_pointers_end_the_run() {
    let cases = [
        ("unreachable", "unreachable"),
        ("divide by zero", "(i32.div_s (i32.const 1) (i32.const 0))"),
        ("request outside memory", "(call $call (i32.const 131000) (i32.const 1000))"),
        ("huge request", "(call $call (i32.const 0) (i32.const 0x7fffffff))"),
        ("response outside memory", "(drop (call $call (i32.const 0) (i32.const 2))) (call $response (i32.const 131060) (i32.const 1000))"),
        ("load outside memory", "(i32.load (i32.const 0x7ffffff0))"),
    ];
    for (name, body) in cases {
        let (r, _, _) = run(&module(&[(0, "{}")], body), small());
        assert!(matches!(r, Err(Error::Trap(_)) | Err(Error::Abi(_))), "{name}: {r:?}");
    }
    // A negative capacity copies nothing.
    let (r, _, _) = run(&module(&[(0, "{}")], "(drop (call $call (i32.const 0) (i32.const 2))) (call $response (i32.const 0) (i32.const -5))"), small());
    r.unwrap();
}

#[test]
fn a_flood_of_host_calls_is_cut_off_and_the_log_stays_bounded() {
    let log = r#"{"op":"log","message":"spam spam spam"}"#;
    let body = format!("(loop $l (drop (call $call (i32.const 4096) (i32.const {}))) (br $l)) i32.const 0", log.len());
    let limits = Limits { max_calls: 5_000, max_log_lines: 50, ..small() };
    let p = Plugin::load(&module(&[(4096, log)], &body), limits).unwrap();
    let r = p.run(&mut Rec::default(), &Map::new());
    assert!(matches!(r, Err(Error::Limit(ref m)) if m.contains("host-call")), "{r:?}");
    // A run within the call budget keeps at most 50 lines and counts the rest.
    let body = format!("(local.set $n (i32.const 0)) (loop $l (drop (call $call (i32.const 4096) (i32.const {}))) (local.set $n (i32.add (local.get $n) (i32.const 1))) (br_if $l (i32.lt_u (local.get $n) (i32.const 400)))) i32.const 0", log.len());
    let p = Plugin::load(&module(&[(4096, log)], &body), Limits { max_log_lines: 50, ..small() }).unwrap();
    let rep = p.run(&mut Rec::default(), &Map::new()).unwrap();
    assert_eq!((rep.log.len(), rep.log_dropped), (50, 350));
}

#[test]
fn oversized_responses_become_errors() {
    struct Big;
    impl Host for Big {
        fn request(&mut self, _: &str, _: &Value) -> Result<Value, String> {
            Ok(json!("x".repeat(100_000)))
        }
    }
    let req = r#"{"op":"big"}"#;
    let body = format!("(local.set $n (call $call (i32.const 4096) (i32.const {}))) (drop (call $response (i32.const 16384) (i32.const 64))) (if (i32.ne (i32.load8_u (i32.const 16390)) (i32.const 102)) (then (return (i32.const 1)))) (local.get $n) (i32.const 1000) i32.gt_u", req.len());
    let p = Plugin::load(&module(&[(4096, req)], &body), Limits { max_response_bytes: 10_000, ..Limits::default() }).unwrap();
    // Returns 0: the answer was a short error, not 100 kB.
    p.run(&mut Big, &Map::new()).unwrap();
}

#[test]
fn every_run_starts_from_a_fresh_instance() {
    // Counts runs in a global: always 1 in a fresh instance.
    let extra = "(global $runs (mut i32) (i32.const 0))";
    let body = "(global.set $runs (i32.add (global.get $runs) (i32.const 1))) (i32.ne (global.get $runs) (i32.const 1))";
    let p = Plugin::load(&module_with(MANIFEST, &[], body, extra), Limits::default()).unwrap();
    for _ in 0..3 {
        p.run(&mut Rec::default(), &Map::new()).unwrap();
    }
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("anicraft-plugins-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn the_registry_installs_into_its_folder_reloads_and_removes() {
    let root = temp_dir("registry");
    let folder = root.join("plugins");
    let src = root.join("downloaded-probe.wasm");
    std::fs::write(&src, module(&[], "i32.const 0")).unwrap();
    let mut reg = Registry::new(Some(folder.clone()));
    assert!(reg.install_overwrites(&src).is_none());
    let inst = reg.install_file(&src).unwrap();
    let dest = folder.join("org.test.probe.wasm");
    assert_eq!(inst.copied_to.as_deref(), Some(dest.as_path()));
    assert!(dest.exists() && inst.replaced.is_none());
    assert!(reg.owns_file(&inst.plugin));
    // Installing it again would replace the copy.
    assert!(reg.install_overwrites(&src).is_some());
    assert!(reg.would_replace(&src).is_some());
    // A broken file in the folder is reported, the good one still loads.
    std::fs::write(folder.join("broken.wasm"), b"\0asm\x01\0\0\0garbage").unwrap();
    std::fs::write(folder.join("notes.txt"), b"ignored").unwrap();
    let rev = reg.revision();
    let rep = reg.reload().unwrap();
    assert!(reg.revision() > rev);
    assert_eq!(rep.loaded, ["org.test.probe"]);
    assert_eq!(rep.failed.len(), 1);
    assert!(rep.failed[0].0.ends_with("broken.wasm"));
    // Removing deletes the copy, not the original.
    let rm = reg.remove("org.test.probe").unwrap();
    assert_eq!(rm.deleted.as_deref(), Some(dest.as_path()));
    assert!(!dest.exists() && src.exists());
    assert!(reg.is_empty());
    assert!(matches!(reg.remove("org.test.probe"), Err(Error::NotFound(_))));
    // Without a folder, installs stay in memory and removing deletes nothing.
    let mut mem = Registry::default();
    let inst = mem.install_file(&src).unwrap();
    assert!(inst.copied_to.is_none() && !mem.owns_file(&inst.plugin));
    assert!(mem.remove("org.test.probe").unwrap().deleted.is_none());
    assert!(src.exists());
    assert!(mem.reload().is_err());
    assert!(mem.install_file(&root).is_err());
    assert!(mem.install_file(&root.join("missing.wasm")).is_err());
    let _ = std::fs::remove_dir_all(&root);
}
