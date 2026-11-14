//! Exporter and ease plug-ins through the engine (ADR 0088): `file.export_plugin` and
//! `file.export_formats`, progress and Cancel, overwrite confirmation, the document never
//! reaching an exporter; plug-in eases on tweens, kept as sampled curves that play the same
//! once the plug-in is gone, through the file format too.

use crate::ease_tests::{run, tweened, x_at};
use crate::plugin_export;
use crate::plugin_tests::{esc, install, request_calls, temp};
use crate::publish::Control;
use crate::{EngineError, Session};
use anicraft_doc::{Easing, sample};
use serde_json::{Value, json};
use std::path::Path;

const EXPORTER: &str = r#"{"id":"org.test.dump","name":"Frame Dump","kind":"exporter","format":{"name":"Frame Dump","extension":"dump"},"params":{"mode":{"type":"choice","options":["first","last"],"default":"first"}}}"#;

/// An exporter that sends `requests` in order and returns the last answer as its file.
fn exporter(requests: &[&str]) -> Vec<u8> {
    let (data, body) = request_calls(EXPORTER, requests, false);
    let text = format!(
        r#"(module
  (import "anicraft" "call" (func $call (param i32 i32) (result i32)))
  (import "anicraft" "response" (func $response (param i32 i32) (result i32)))
  (memory (export "memory") 64)
  {data}
  (func (export "ac_abi_version") (result i32) i32.const 1)
  (func (export "ac_manifest") (result i64) (i64.or (i64.shl (i64.const {len}) (i64.const 32)) (i64.const 1024)))
  (func (export "ac_alloc") (param i32) (result i32) i32.const 70000)
  (func (export "ac_export") (param i32 i32) (result i64) (local $n i32)
    {body}
    (local.set $n (call $response (i32.const 1048576) (local.get $n)))
    (i64.or (i64.shl (i64.extend_i32_u (local.get $n)) (i64.const 32)) (i64.const 1048576))))"#,
        len = EXPORTER.len()
    );
    wat::parse_str(&text).expect("test WAT")
}

const EASES: &str = r#"{"id":"org.test.steps","name":"Test Steps","kind":"ease","eases":[{"id":"steps4","name":"Stepped 4"}]}"#;

/// Four steps: floor(4t) / 4.
fn ease_plugin() -> Vec<u8> {
    let text = format!(
        r#"(module (memory (export "memory") 1) (data (i32.const 1024) "{}")
  (func (export "ac_abi_version") (result i32) i32.const 1)
  (func (export "ac_manifest") (result i64) (i64.or (i64.shl (i64.const {}) (i64.const 32)) (i64.const 1024)))
  (func (export "ac_ease") (param $i i32) (param $t f64) (result f64) (f64.div (f64.floor (f64.mul (local.get $t) (f64.const 4))) (f64.const 4))))"#,
        esc(EASES),
        EASES.len()
    );
    wat::parse_str(&text).expect("test WAT")
}

/// The answer the dump exporter wrote: `{"ok": …, "result"|"error": …}`.
fn dumped(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).expect("written")).expect("JSON")
}

#[test]
fn an_exporter_reads_evaluated_frames_and_its_file_is_written() {
    let dir = temp("export");
    let mut s = Session::new(sample::demo());
    install(&mut s, &dir, "dump.wasm", &exporter(&[r#"{"op":"movie"}"#, r#"{"op":"frame","index":2}"#]));
    // Listed with the built-in formats.
    let f = run(&mut s, "file.export_formats", json!({}));
    let fmts = f["formats"].as_array().expect("formats");
    assert!(fmts.iter().any(|x| x["id"] == "gif" && x["command"] == "file.export_gif"));
    let mine = fmts.iter().find(|x| x["id"] == "org.test.dump").expect("the plug-in's format");
    assert_eq!((mine["name"].as_str(), mine["extensions"][0].as_str(), mine["plugin"].as_bool()), (Some("Frame Dump"), Some("dump"), Some(true)));
    assert_eq!(mine["options"][0]["name"], "mode");
    let (rev, undo) = (s.revision, s.history_len());
    let out = dir.join("movie.dump");
    let v = run(&mut s, "file.export_plugin", json!({"format": "org.test.dump", "path": out.display().to_string(), "from": 3, "to": 9}));
    assert_eq!((v["frames"].as_u64(), v["frames_read"].as_u64(), v["calls"].as_u64()), (Some(7), Some(1), Some(2)));
    assert_eq!(v["options"]["mode"], "first");
    // Frame index 2 of an export from frame 3 is movie frame 5, evaluated as the stage shows it.
    let d = dumped(&out);
    assert_eq!(d["ok"], true, "{d}");
    let r = &d["result"];
    assert_eq!((r["index"].as_u64(), r["frame"].as_u64()), (Some(2), Some(5)));
    let items = r["items"].as_array().expect("items");
    let dl = anicraft_doc::evaluate(&s.doc, 5);
    assert_eq!(items.len(), dl.items.iter().filter(|i| !i.guide).count());
    let first = &items[0];
    assert!(first["path"].as_str().is_some_and(|p| p.starts_with('M')), "{first}");
    assert_eq!(first["transform"].as_array().map(Vec::len), Some(6));
    assert!(first["fill"]["type"].is_string() || first["fill"].is_null());
    // Exporting changes nothing in the document and adds no undo step.
    assert_eq!((s.revision, s.history_len()), (rev, undo));
    // The file is there now: replacing it needs a confirmation (ADR 0004).
    let again = s.execute("file.export_plugin", &json!({"format": "org.test.dump", "path": out.display().to_string()}));
    assert!(matches!(again, Err(EngineError::NeedsConfirmation(_))), "{again:?}");
    run(&mut s, "file.export_plugin", json!({"format": "org.test.dump", "path": out.display().to_string(), "confirm": true}));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_movie_request_describes_the_range_and_bad_requests_are_answers() {
    let dir = temp("movie");
    let mut s = Session::new(sample::demo());
    let out = dir.join("a.dump");
    let cases = [
        (r#"{"op":"movie"}"#, None),
        (r#"{"op":"frame","index":99}"#, Some("past the end")),
        (r#"{"op":"frame"}"#, Some("needs an \"index\"")),
        (r#"{"op":"document"}"#, Some("can't read or change the document")),
        (r#"{"op":"command","id":"draw.rect","params":{}}"#, Some("can't read or change the document")),
        (r#"{"op":"teleport"}"#, Some("unknown op")),
    ];
    for (i, (req, err)) in cases.iter().enumerate() {
        install(&mut s, &dir, &format!("dump{i}.wasm"), &exporter(&[req]));
        run(&mut s, "file.export_plugin", json!({"format": "org.test.dump", "path": out.display().to_string(), "to": 4, "confirm": true}));
        let d = dumped(&out);
        match err {
            None => {
                let m = &d["result"];
                assert_eq!((m["from"].as_u64(), m["to"].as_u64(), m["frames"].as_u64(), m["name"].as_str()), (Some(0), Some(4), Some(5), Some("a")));
                assert_eq!((m["width"].as_f64(), m["total_frames"].as_u64()), (Some(f64::from(s.doc.stage.width)), Some(u64::from(s.doc.frame_count()))));
                assert!(m["background"].as_str().is_some_and(|b| b.len() == 9));
            }
            Some(want) => assert!(d["ok"] == false && d["error"].as_str().is_some_and(|e| e.contains(want)), "{req}: {d}"),
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn export_params_are_checked() {
    let dir = temp("params");
    let mut s = Session::new(sample::demo());
    install(&mut s, &dir, "dump.wasm", &exporter(&[r#"{"op":"movie"}"#]));
    let path = dir.join("x.dump").display().to_string();
    let err = |s: &mut Session, p: Value| s.execute("file.export_plugin", &p).expect_err("refused").to_string();
    assert!(err(&mut s, json!({"format": "org.none", "path": path})).contains("installed: org.test.dump"));
    assert!(err(&mut s, json!({"format": "org.test.dump"})).contains("path"));
    assert!(err(&mut s, json!({"format": "org.test.dump", "path": " "})).contains("empty"));
    assert!(err(&mut s, json!({"format": "org.test.dump", "path": path, "options": {"mode": "middle"}})).contains("mode"));
    assert!(err(&mut s, json!({"format": "org.test.dump", "path": path, "from": 5, "to": 2})).contains("before"));
    // An exporter is not a command, and a command plug-in can't export.
    assert!(s.execute("plugin.run", &json!({"id": "org.test.dump"})).expect_err("not a command").to_string().contains("File › Export"));
    assert!(crate::cmds_plugin::plugin_may_call("file.export_plugin").is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn progress_counts_frames_read_and_cancel_stops_before_anything_is_written() {
    let dir = temp("cancel");
    let mut s = Session::new(sample::demo());
    let reqs: Vec<String> = (0..4).map(|i| format!(r#"{{"op":"frame","index":{i}}}"#)).collect();
    let refs: Vec<&str> = reqs.iter().map(String::as_str).chain([r#"{"op":"frame","index":1}"#]).collect();
    install(&mut s, &dir, "dump.wasm", &exporter(&refs));
    let out = dir.join("p.dump");
    let (plugin, job) = plugin_export::plan(&s, &json!({"format": "org.test.dump", "path": out.display().to_string(), "from": 0, "to": 9})).expect("plan");
    let control = Control::default();
    let v = plugin_export::run(&plugin, &s.doc, &job, Some(&control)).expect("exports");
    // Frame 1 twice counts once.
    assert_eq!(control.progress(), (4, 10));
    assert_eq!(v["frames_read"], 4);
    let out2 = dir.join("cancelled.dump");
    let job2 = plugin_export::Job { path: out2.clone(), ..job };
    let control = Control::default();
    control.cancel();
    assert!(matches!(plugin_export::run(&plugin, &s.doc, &job2, Some(&control)), Err(EngineError::Cancelled)));
    assert!(!out2.exists(), "a cancelled export writes nothing");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_plugin_ease_drives_a_tween_and_the_document_keeps_its_curve() {
    let dir = temp("ease");
    let mut s = tweened();
    install(&mut s, &dir, "steps.wasm", &ease_plugin());
    let l = run(&mut s, "ease.list", json!({}));
    assert_eq!(l["plugins"], json!([{"name": "Stepped 4", "plugin": "org.test.steps", "plugin_name": "Test Steps", "ease": "steps4", "easing": {"plugin": "org.test.steps", "ease": "steps4"}}]));
    let v = run(&mut s, "timeline.set_easing", json!({"frame": 0, "easing": {"plugin": "org.test.steps", "ease": "steps4"}}));
    assert_eq!(v["easing"], "Stepped 4");
    // Four steps of 25 over ten frames: frames 0-2 at 0, 3-4 at 25, 5-7 at 50, 8-9 at 75.
    let xs: Vec<f32> = (0..10).map(|f| x_at(&mut s, f)).collect();
    assert_eq!(xs, [0.0, 0.0, 0.0, 25.0, 25.0, 50.0, 50.0, 50.0, 75.0, 75.0]);
    let info = run(&mut s, "ease.info", json!({"frame": 0}));
    assert_eq!((info["kind"].as_str(), info["label"].as_str()), (Some("plugin"), Some("Stepped 4")));
    assert_eq!(info["easing"]["sampled"]["values"].as_array().map(Vec::len), Some(257));
    // By name, any case; unknown eases and plug-ins are errors.
    run(&mut s, "ease.set", json!({"frame": 0, "easing": {"plugin": "org.test.steps", "ease": "stepped 4"}}));
    assert!(s.execute("ease.set", &json!({"frame": 0, "easing": {"plugin": "org.test.steps", "ease": "smooth"}})).expect_err("no such ease").to_string().contains("steps4"));
    assert!(s.execute("ease.set", &json!({"frame": 0, "easing": {"plugin": "org.none", "ease": "x"}})).is_err());
    // The file keeps the table: the plug-in removed, the movie plays the same, and again
    // after a save and load.
    run(&mut s, "plugin.remove", json!({"id": "org.test.steps"}));
    let again: Vec<f32> = (0..10).map(|f| x_at(&mut s, f)).collect();
    assert_eq!(again, xs);
    let text = crate::format::to_json(&s.doc).expect("saves");
    assert!(text.contains("\"Sampled\""));
    let mut back = Session::new(crate::format::from_json(&text).expect("loads"));
    let reloaded: Vec<f32> = (0..10).map(|f| x_at(&mut back, f)).collect();
    assert_eq!(reloaded, xs);
    // Its sampled form is an easing param too (copy it to another tween with ease.info).
    let mut t = tweened();
    run(&mut t, "ease.set", json!({"frame": 0, "easing": info["easing"]}));
    assert_eq!(x_at(&mut t, 3), 25.0);
    // Editing a point makes it a custom curve drawn like it.
    run(&mut t, "ease.move_point", json!({"frame": 0, "index": 10, "x": 0.16, "y": 0.4}));
    assert!(matches!(t.active_layer().and_then(|l| l.keyframes.first()).and_then(|k| k.tween.as_ref()).map(|w| &w.easing), Some(Easing::Custom(_))));
    let _ = std::fs::remove_dir_all(&dir);
}
