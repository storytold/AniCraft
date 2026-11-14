//! Exporter and ease plug-ins (ADR 0088) on modules written in WebAssembly text: what they
//! return, and hostile ones (endless loops, huge or misplaced output, NaN eases, eases that
//! miss their ends, host calls from an ease, Cancel) ending in a clear error.

use anicraft_plugins::{EASE_SAMPLES, Error, Host, Kind, Limits, Plugin};
use serde_json::{Map, Value, json};
use std::time::{Duration, Instant};

/// WAT string literal contents for `s`.
fn esc(s: &str) -> String {
    s.bytes().map(|b| if b == b'"' || b == b'\\' || !(0x20..0x7f).contains(&b) { format!("\\{b:02x}") } else { (b as char).to_string() }).collect()
}

fn wat(text: &str) -> Vec<u8> {
    wat::parse_str(text).unwrap_or_else(|e| panic!("bad test WAT: {e}\n{text}"))
}

/// The exports every plug-in has, with `manifest` at 1024 and the imports.
fn base(manifest: &str) -> String {
    format!(
        r#"(import "anicraft" "call" (func $call (param i32 i32) (result i32)))
  (import "anicraft" "response" (func $response (param i32 i32) (result i32)))
  (memory (export "memory") 2)
  (data (i32.const 1024) "{}")
  (func (export "ac_abi_version") (result i32) i32.const 1)
  (func (export "ac_manifest") (result i64) (i64.or (i64.shl (i64.const {}) (i64.const 32)) (i64.const 1024)))"#,
        esc(manifest),
        manifest.len()
    )
}

// ----- eases ---------------------------------------------------------------------------------

const EASES: &str = r#"{"id":"org.test.eases","name":"Test Eases","kind":"ease","eases":[{"id":"steps4","name":"Stepped 4"},{"id":"linear","name":" Straight "}]}"#;

/// An ease plug-in whose `ac_ease(index, t)` is `body` (locals `$i`, `$t`).
fn ease_module(manifest: &str, body: &str) -> Vec<u8> {
    wat(&format!("(module {}\n  (func (export \"ac_ease\") (param $i i32) (param $t f64) (result f64) {body}))", base(manifest)))
}

/// Ease 0: four steps, floor(4t)/4; ease 1: t.
const STEPS: &str = "(if (result f64) (i32.eqz (local.get $i)) (then (f64.div (f64.floor (f64.mul (local.get $t) (f64.const 4))) (f64.const 4))) (else (local.get $t)))";

#[test]
fn an_ease_plugin_is_sampled_once_when_it_loads() {
    let t = Instant::now();
    let p = Plugin::load(&ease_module(EASES, STEPS), Limits::default()).expect("loads");
    let ms = t.elapsed().as_secs_f64() * 1e3;
    assert_eq!(p.manifest().kind, Kind::Ease);
    let e = p.eases();
    assert_eq!(e.len(), 2);
    assert_eq!((e[0].id.as_str(), e[0].name.as_str(), e[1].name.as_str()), ("steps4", "Stepped 4", "Straight"));
    let v = &e[0].values;
    assert_eq!(v.len(), EASE_SAMPLES);
    let at = |t: f64| v[(t * (EASE_SAMPLES - 1) as f64).round() as usize];
    assert_eq!((at(0.0), at(0.2), at(0.25), at(0.6), at(0.99), at(1.0)), (0.0, 0.0, 0.25, 0.5, 0.75, 1.0));
    assert!(e[1].values.iter().enumerate().all(|(i, y)| (f64::from(*y) - i as f64 / 256.0).abs() < 1e-6));
    // Two eases, 514 calls into the interpreter: a few milliseconds even in a debug build.
    assert!(ms < 2_000.0, "{ms} ms");
}

#[test]
fn hostile_eases_stop_the_load_with_a_clear_message() {
    let cases = [
        ("NaN", "f64.const nan", "not a number"),
        ("infinity", "(f64.div (f64.const 1) (f64.const 0))", "not a number"),
        ("overshoot", "(if (result f64) (f64.eq (local.get $t) (f64.const 0.5)) (then (f64.const 50)) (else (local.get $t)))", "outside"),
        ("half way", "(f64.mul (local.get $t) (f64.const 0.5))", "ends at 1"),
        ("late start", "(f64.add (local.get $t) (f64.const 0.25))", "starts at 0"),
        ("endless", "(loop $l (br $l)) f64.const 0", "instruction budget"),
        ("trap", "unreachable", "stopped with an error"),
        ("host call", "(drop (call $call (i32.const 0) (i32.const 0))) local.get $t", "only while"),
    ];
    for (what, body, want) in cases {
        let t = Instant::now();
        let err = Plugin::load(&ease_module(EASES, body), Limits::default()).expect_err(what).to_string();
        assert!(err.contains(want), "{what}: {err}");
        assert!(t.elapsed() < Duration::from_secs(10), "{what} took {:?}", t.elapsed());
        if what == "NaN" {
            assert!(err.contains("Stepped 4") && err.contains("t = 0.000"), "{err}");
        }
    }
    // An ease plug-in without `ac_ease`, or with the wrong signature.
    let no_fn = wat(&format!("(module {})", base(EASES)));
    assert!(Plugin::load(&no_fn, Limits::default()).expect_err("no ac_ease").to_string().contains("ac_ease"));
    let wrong = wat(&format!("(module {} (func (export \"ac_ease\") (param i32 f32) (result f32) f32.const 0))", base(EASES)));
    assert!(Plugin::load(&wrong, Limits::default()).expect_err("f32 ac_ease").to_string().contains("ac_ease"));
}

#[test]
fn ease_and_exporter_manifests_are_checked() {
    let bad = [
        (r#"{"id":"a","name":"A","kind":"ease"}"#, "1-32"),
        (r#"{"id":"a","name":"A","kind":"ease","eases":[]}"#, "1-32"),
        (r#"{"id":"a","name":"A","kind":"ease","eases":[{"id":"x y","name":"X"}]}"#, "invalid ease id"),
        (r#"{"id":"a","name":"A","kind":"ease","eases":[{"id":"x","name":""}]}"#, "printable"),
        (r#"{"id":"a","name":"A","kind":"ease","eases":[{"id":"x","name":"X"},{"id":"x","name":"Y"}]}"#, "twice"),
        (r#"{"id":"a","name":"A","kind":"ease","eases":[{"id":"x","name":"X","t":1}]}"#, "unknown field"),
        (r#"{"id":"a","name":"A","kind":"ease","eases":[{"id":"x","name":"X"}],"params":{"n":{"type":"int"}}}"#, "no `params`"),
        (r#"{"id":"a","name":"A","kind":"command","eases":[{"id":"x","name":"X"}]}"#, "only an ease"),
        (r#"{"id":"a","name":"A","kind":"exporter"}"#, "needs a `format`"),
        (r#"{"id":"a","name":"A","kind":"exporter","format":{"name":"T","extension":"t x t"}}"#, "extension"),
        (r#"{"id":"a","name":"A","kind":"exporter","format":{"name":"T","extension":""}}"#, "extension"),
        (r#"{"id":"a","name":"A","kind":"exporter","format":{"name":"","extension":"txt"}}"#, "name"),
        (r#"{"id":"a","name":"A","kind":"command","format":{"name":"T","extension":"txt"}}"#, "only an exporter"),
    ];
    for (m, want) in bad {
        let err = anicraft_plugins::Manifest::parse(m.as_bytes()).expect_err(m).to_string();
        assert!(err.contains(want), "{m}: {err}");
    }
    let many: Vec<String> = (0..33).map(|i| format!(r#"{{"id":"e{i}","name":"E{i}"}}"#)).collect();
    assert!(anicraft_plugins::Manifest::parse(format!(r#"{{"id":"a","name":"A","kind":"ease","eases":[{}]}}"#, many.join(",")).as_bytes()).is_err());
    let ok = anicraft_plugins::Manifest::parse(br#"{"id":"a","name":"A","kind":"exporter","format":{"name":" Text ","extension":".TXT"}}"#).expect("exporter");
    let f = ok.format.expect("format");
    assert_eq!((f.name.as_str(), f.extension.as_str()), ("Text", "txt"));
}

// ----- exporters -----------------------------------------------------------------------------

const EXPORTER: &str = r#"{"id":"org.test.text","name":"Test Text","kind":"exporter","format":{"name":"Test Text","extension":"txt"},"params":{"title":{"type":"choice","options":["a","b"],"default":"a"}}}"#;
const FRAME_REQ: &str = r#"{"op":"frame","index":0}"#;
const LOG_REQ: &str = r#"{"op":"log","message":"no frames to write"}"#;

/// An exporter whose `ac_export(params, len)` is `body` (locals `$p`, `$len`, `$n`); requests
/// at 4096 and 4200, "hello" at 8192.
fn exporter(body: &str) -> Vec<u8> {
    wat(&format!(
        r#"(module {}
  (data (i32.const 4096) "{}")
  (data (i32.const 4200) "{}")
  (data (i32.const 8192) "hello")
  (func (export "ac_alloc") (param i32) (result i32) i32.const 30000)
  (func (export "ac_export") (param $p i32) (param $len i32) (result i64) (local $n i32) {body}))"#,
        base(EXPORTER),
        esc(FRAME_REQ),
        esc(LOG_REQ)
    ))
}

/// A host that counts frame requests and can cancel after `cancel_after` of them.
#[derive(Default)]
struct Frames {
    served: u32,
    cancel_after: Option<u32>,
}

impl Host for Frames {
    fn request(&mut self, op: &str, _: &Value) -> Result<Value, String> {
        match op {
            "frame" => {
                self.served += 1;
                Ok(json!({"index": 0, "items": []}))
            }
            _ => Err(format!("unknown op '{op}'")),
        }
    }
    fn cancelled(&self) -> bool {
        self.cancel_after.is_some_and(|n| self.served >= n)
    }
}

fn frame_call() -> String {
    format!("(local.set $n (call $call (i32.const 4096) (i32.const {})))", FRAME_REQ.len())
}

fn options(p: &Plugin) -> Map<String, Value> {
    p.manifest().defaults()
}

#[test]
fn an_exporter_reads_frames_and_returns_its_file() {
    let body = format!("{0} {0} (i64.or (i64.shl (i64.const 5) (i64.const 32)) (i64.const 8192))", frame_call());
    let p = Plugin::load(&exporter(&body), Limits::default()).expect("loads");
    assert_eq!(p.manifest().format.as_ref().map(|f| f.extension.as_str()), Some("txt"));
    let mut host = Frames::default();
    let (bytes, report) = p.export(&mut host, &options(&p), &Limits::exporter()).expect("exports");
    assert_eq!(bytes, b"hello");
    assert_eq!((host.served, report.calls), (2, 2));
    // An empty file is a file.
    let p = Plugin::load(&exporter("i64.const 8192"), Limits::default()).expect("loads");
    assert_eq!(p.export(&mut Frames::default(), &options(&p), &Limits::exporter()).expect("empty").0, b"");
    // An exporter isn't a command, and a command isn't an exporter.
    assert!(p.run(&mut Frames::default(), &options(&p)).is_err());
}

#[test]
fn hostile_exporters_end_in_an_error() {
    let small = Limits { fuel: 30_000_000, wall_time_ms: 5_000, ..Limits::exporter() };
    let cases: Vec<(&str, String, &str)> = vec![
        ("failure", format!("(drop (call $call (i32.const 4200) (i32.const {}))) i64.const -1", LOG_REQ.len()), "could not export: no frames to write"),
        ("huge output", "i64.const 0x7fffffff00000000".into(), "output size"),
        ("output outside memory", "(i64.or (i64.shl (i64.const 100000) (i64.const 32)) (i64.const 120000))".into(), "outside the module's memory"),
        ("endless loop", "(loop $l (br $l)) i64.const 0".into(), "instruction budget"),
        ("endless requests", format!("(loop $l {} (br $l)) i64.const 0", frame_call()), "host-call budget"),
        ("trap", "unreachable".into(), "stopped with an error"),
    ];
    for (what, body, want) in cases {
        let p = Plugin::load(&exporter(&body), Limits::default()).expect(what);
        let t = Instant::now();
        let limits = if what == "endless requests" { Limits { max_calls: 500, ..small.clone() } } else { small.clone() };
        let err = p.export(&mut Frames::default(), &options(&p), &limits).expect_err(what).to_string();
        assert!(err.contains(want), "{what}: {err}");
        assert!(t.elapsed() < Duration::from_secs(10), "{what} took {:?}", t.elapsed());
    }
    // A cap below the file.
    let p = Plugin::load(&exporter("(i64.or (i64.shl (i64.const 5) (i64.const 32)) (i64.const 8192))"), Limits::default()).expect("loads");
    let tight = Limits { max_output_bytes: 4, ..Limits::exporter() };
    assert!(p.export(&mut Frames::default(), &options(&p), &tight).expect_err("cap").to_string().contains("output size"));
    // An exporter needs ac_export.
    let no_fn = wat(&format!("(module {} (func (export \"ac_alloc\") (param i32) (result i32) i32.const 0))", base(EXPORTER)));
    assert!(Plugin::load(&no_fn, Limits::default()).expect_err("no ac_export").to_string().contains("ac_export"));
}

#[test]
fn cancel_stops_an_export_between_requests_and_inside_a_busy_loop() {
    // Between requests: the host cancels after the third frame.
    let p = Plugin::load(&exporter(&format!("(loop $l {} (br $l)) i64.const 0", frame_call())), Limits::default()).expect("loads");
    let mut host = Frames { cancel_after: Some(3), ..Frames::default() };
    assert_eq!(p.export(&mut host, &options(&p), &Limits::exporter()).expect_err("cancelled"), Error::Cancelled);
    assert_eq!(host.served, 3);
    // Inside a loop that never asks for anything: at the next slice of fuel.
    let p = Plugin::load(&exporter("(loop $l (br $l)) i64.const 0"), Limits::default()).expect("loads");
    let mut host = Frames { cancel_after: Some(0), ..Frames::default() };
    let t = Instant::now();
    assert_eq!(p.export(&mut host, &options(&p), &Limits::exporter()).expect_err("cancelled"), Error::Cancelled);
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
}
