//! Plug-ins through the engine: install, run as one undo step, atomic failures, the command
//! policy, and hostile requests through the host side of the ABI.

use crate::cmds_plugin::{SessionHost, plugin_may_call};
use crate::{Session, commands};
use anicraft_doc::sample;
use anicraft_plugins::Host;
use serde_json::{Value, json};
use std::path::PathBuf;

/// WAT string literal contents for `s`.
fn esc(s: &str) -> String {
    s.bytes().map(|b| if b == b'"' || b == b'\\' || !(0x20..0x7f).contains(&b) { format!("\\{b:02x}") } else { (b as char).to_string() }).collect()
}

/// A plug-in that sends `requests` in order, then returns `code`. With `check`, it returns 1
/// as soon as an answer isn't `{"ok":true…` (byte 6 is `t`).
pub(crate) fn plugin(manifest: &str, requests: &[&str], check: bool, code: i32) -> Vec<u8> {
    plugin_then(manifest, requests, check, "", code)
}

/// [`plugin`], running `tail` before it returns.
pub(crate) fn plugin_then(manifest: &str, requests: &[&str], check: bool, tail: &str, code: i32) -> Vec<u8> {
    let mut data = format!("(data (i32.const 1024) \"{}\")\n", esc(manifest));
    let mut body = String::new();
    let mut at = 8192;
    for r in requests {
        data.push_str(&format!("(data (i32.const {at}) \"{}\")\n", esc(r)));
        body.push_str(&format!("(local.set $n (call $call (i32.const {at}) (i32.const {})))\n", r.len()));
        if check {
            body.push_str("(drop (call $response (i32.const 60000) (i32.const 16)))\n(if (i32.ne (i32.load8_u (i32.const 60006)) (i32.const 116)) (then (return (i32.const 1))))\n");
        }
        at += r.len() + 16;
    }
    let text = format!(
        r#"(module
  (import "anicraft" "call" (func $call (param i32 i32) (result i32)))
  (import "anicraft" "response" (func $response (param i32 i32) (result i32)))
  (memory (export "memory") 2)
  {data}
  (func (export "ac_abi_version") (result i32) i32.const 1)
  (func (export "ac_manifest") (result i64) (i64.or (i64.shl (i64.const {len}) (i64.const 32)) (i64.const 1024)))
  (func (export "ac_alloc") (param i32) (result i32) i32.const 70000)
  (func (export "ac_run") (param i32 i32) (result i32) (local $n i32)
    {body}
    {tail}
    i32.const {code}))"#,
        len = manifest.len()
    );
    wat::parse_str(&text).expect("test WAT")
}

const STARS: &str = r#"{"id":"org.test.stars","name":"Test Stars","version":"1.0","kind":"command","params":{"count":{"type":"int","min":1,"max":5,"default":3}}}"#;
const NUDGE: &str = r#"{"id":"org.test.nudge","name":"Nudge","kind":"command","selection":"required"}"#;

fn stars() -> Vec<u8> {
    let star = r##"{"op":"command","id":"draw.star","params":{"cx":100,"cy":80,"outer":20,"inner":8,"points":5,"fill":"#ffee88"}}"##;
    plugin(STARS, &[r#"{"op":"command","id":"layer.new","params":{"name":"Stars"}}"#, star, star, star], true, 0)
}

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("anicraft-engine-plugins-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("temp dir");
    d
}

fn install(s: &mut Session, dir: &std::path::Path, file: &str, bytes: &[u8]) -> Value {
    let path = dir.join(file);
    std::fs::write(&path, bytes).expect("write module");
    s.execute("plugin.install", &json!({"path": path.display().to_string()})).expect("installs")
}

fn shapes(s: &Session) -> usize {
    s.doc.timeline(s.scene).layers.iter().flat_map(|l| l.keyframes.iter()).map(|k| k.elements.len()).sum()
}

#[test]
fn a_run_is_one_undo_step_named_after_the_plugin() {
    let dir = temp("run");
    let mut s = Session::new(sample::demo());
    let v = install(&mut s, &dir, "stars.wasm", &stars());
    assert_eq!(v["installed"]["name"], "Test Stars");
    assert_eq!(v["installed"]["params"][0]["name"], "count");
    let list = s.execute("plugin.list", &json!({})).unwrap();
    assert_eq!(list["plugins"].as_array().map(Vec::len), Some(1));
    let (doc, layers, steps, rev) = (s.doc.clone(), s.doc.timeline(0).layers.len(), s.history_len(), s.revision);
    let n = shapes(&s);
    let out = s.execute("plugin.run", &json!({"id": "org.test.stars", "params": {"count": 99}})).unwrap();
    assert_eq!(out["params"]["count"], 5, "clamped to the manifest's range");
    assert_eq!(out["calls"], 4);
    assert_eq!(s.doc.timeline(0).layers.len(), layers + 1);
    assert_eq!(shapes(&s), n + 3);
    assert!(s.revision > rev);
    assert_eq!(s.history_len(), steps + 1);
    assert_eq!(s.undo_label(), Some("Test Stars"));
    // The new artwork renders.
    let dl = s.display_list();
    assert!(dl.items.len() >= 3);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc == doc, "undo restores the document exactly");
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(shapes(&s), n + 3);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_failing_run_leaves_the_document_untouched() {
    let dir = temp("fail");
    let mut s = Session::default();
    // Draws, then fails.
    let bad = plugin(r#"{"id":"org.test.bad","name":"Bad","kind":"command"}"#, &[r#"{"op":"command","id":"draw.rect","params":{"x":0,"y":0,"w":10,"h":10}}"#, r#"{"op":"log","message":"gave up"}"#], false, 5);
    install(&mut s, &dir, "bad.wasm", &bad);
    let (doc, steps, state) = (s.doc.clone(), s.history_len(), s.state);
    let e = s.execute("plugin.run", &json!({"id": "org.test.bad"})).unwrap_err().to_string();
    assert!(e.contains("code 5") && e.contains("gave up"), "{e}");
    assert!(s.doc == doc && s.history_len() == steps && s.state == state);
    // An endless loop: stopped by its budget, nothing changed, quickly.
    let spin = plugin_then(r#"{"id":"org.test.spin","name":"Spin","kind":"command"}"#, &[r#"{"op":"command","id":"draw.rect","params":{"x":0,"y":0,"w":5,"h":5}}"#], true, "(loop $l (br $l))", 0);
    s.plugins.set_limits(anicraft_plugins::Limits { fuel: 50_000_000, ..Default::default() });
    install(&mut s, &dir, "spin.wasm", &spin);
    install(&mut s, &dir, "spin.wasm", &spin);
    let t = std::time::Instant::now();
    let e = s.execute("plugin.run", &json!({"id": "org.test.spin"})).unwrap_err().to_string();
    assert!(e.contains("instruction budget"), "{e}");
    assert!(t.elapsed() < std::time::Duration::from_secs(5));
    assert!(s.doc == doc && s.history_len() == steps);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn plugins_cannot_reach_files_history_the_clipboard_or_other_plugins() {
    let dir = temp("policy");
    let target = dir.join("stolen.anicraft");
    let save = format!(r#"{{"op":"command","id":"file.save","params":{{"path":"{}","confirm":true}}}}"#, target.display().to_string().replace('\\', "/"));
    let forbidden = [
        save.as_str(),
        r#"{"op":"command","id":"file.export_png","params":{"path":"x.png"}}"#,
        r#"{"op":"command","id":"file.export_apng","params":{"path":"x.png"}}"#,
        r#"{"op":"command","id":"file.export_webp","params":{"path":"x.webp"}}"#,
        r#"{"op":"command","id":"plugin.run","params":{"id":"org.test.sneaky"}}"#,
        r#"{"op":"command","id":"plugin.remove","params":{"id":"org.test.sneaky","confirm":true}}"#,
        r#"{"op":"command","id":"edit.undo"}"#,
        r#"{"op":"command","id":"edit.copy"}"#,
        r#"{"op":"command","id":"no.such"}"#,
        r#"{"op":"command"}"#,
        r#"{"op":"read_file","path":"/etc/passwd"}"#,
    ];
    let m = r#"{"id":"org.test.sneaky","name":"Sneaky","kind":"command"}"#;
    // Every forbidden request must be answered with ok:false: the plug-in returns 1 on an ok.
    for req in forbidden {
        let mut s = Session::default();
        let bytes = plugin(m, &[req], false, 0);
        install(&mut s, &dir, "sneaky.wasm", &bytes);
        let mut host = SessionHost { s: &mut s };
        let v: Value = serde_json::from_str(req).unwrap();
        let op = v["op"].as_str().unwrap_or("");
        assert!(host.request(op, &v).is_err(), "allowed: {req}");
        // And through the real ABI the run completes without the side effect.
        s.execute("plugin.run", &json!({"id": "org.test.sneaky"})).unwrap();
        assert_eq!(s.plugins.len(), 1);
    }
    assert!(!target.exists(), "a plug-in saved a file");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn every_command_namespace_has_a_plugin_policy() {
    // A new namespace must be classified in `plugin_may_call` before plug-ins can use it.
    for c in commands() {
        let ns = c.id.split('.').next().unwrap_or("");
        let verdict = plugin_may_call(c.id);
        match ns {
            "file" | "plugin" | "control" | "prefs" => assert!(verdict.is_err(), "{} must be refused", c.id),
            // Publish Settings (ADR 0060): read yes, change no (it says what Publish writes).
            "publish" => assert_eq!(verdict.is_ok(), c.id == "publish.settings_get", "{}", c.id),
            // Brushes (ADR 0068): list yes; your own brushes and brush set files no.
            "brush" => assert_eq!(verdict.is_ok(), c.id == "brush.preset_list", "{}", c.id),
            _ => assert!(verdict.is_ok() || matches!(c.id, "edit.undo" | "edit.redo") || c.id.contains("copy") || c.id.contains("cut") || c.id.contains("paste"), "{} has no plug-in policy (add its namespace to plugin_may_call)", c.id),
        }
    }
    assert!(plugin_may_call("draw.rect").is_ok());
    assert!(plugin_may_call("future.thing").is_err());
    // Edit Multiple Frames (ADR 0051): document edits yes, the clipboard no.
    assert!(["frames.transform", "frames.delete", "frames.align", "frames.merge", "frames.edit_multiple"].iter().all(|c| plugin_may_call(c).is_ok()));
    assert!(["frames.copy", "frames.cut", "frames.paste"].iter().all(|c| plugin_may_call(c).is_err()));
}

#[test]
fn selection_plugins_need_a_selection_and_see_it() {
    let dir = temp("sel");
    let mut s = Session::default();
    let nudge = plugin(NUDGE, &[r#"{"op":"elements","selected":true,"paths":true}"#, r#"{"op":"command","id":"element.translate","params":{"dx":5,"dy":0}}"#], true, 0);
    install(&mut s, &dir, "nudge.wasm", &nudge);
    let e = s.execute("plugin.run", &json!({"id": "org.test.nudge"})).unwrap_err().to_string();
    assert!(e.contains("select some artwork"), "{e}");
    let a = s.execute("draw.rect", &json!({"x": 10, "y": 10, "w": 20, "h": 20})).unwrap()["element"].as_u64().unwrap();
    s.execute("draw.rect", &json!({"x": 100, "y": 10, "w": 20, "h": 20})).unwrap();
    s.execute("element.select", &json!({"ids": [a]})).unwrap();
    // What the plug-in sees.
    let mut host = SessionHost { s: &mut s };
    let v = host.request("elements", &json!({"op": "elements", "selected": true, "paths": true})).unwrap();
    let els = v["elements"].as_array().unwrap();
    assert_eq!(els.len(), 1);
    assert_eq!(els[0]["id"], a);
    assert_eq!(els[0]["kind"], "shape");
    assert!(els[0]["path"].as_str().is_some_and(|d| d.starts_with('M')));
    let b = els[0]["bounds"].as_array().unwrap();
    assert!((b[0].as_f64().unwrap() - 10.0).abs() < 2.0 && (b[2].as_f64().unwrap() - 20.0).abs() < 3.0, "{b:?}");
    let all = host.request("elements", &json!({"op": "elements"})).unwrap();
    assert_eq!(all["elements"].as_array().map(Vec::len), Some(2));
    let d = host.request("document", &json!({"op": "document"})).unwrap();
    assert_eq!(d["selection"], json!([a]));
    assert_eq!(d["stage"]["width"], s.doc.stage.width);
    s.execute("plugin.run", &json!({"id": "org.test.nudge"})).unwrap();
    assert_eq!(s.undo_label(), Some("Nudge"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn plugins_survive_new_documents_and_removal_from_the_folder_asks_first() {
    let root = temp("folder");
    let folder = root.join("plugins");
    let mut s = Session::default();
    s.execute("plugin.reload", &json!({"folder": folder.display().to_string()})).unwrap();
    let v = install(&mut s, &root, "stars.wasm", &stars());
    assert!(v["copied_to"].as_str().is_some_and(|p| p.ends_with("org.test.stars.wasm")));
    s.execute("draw.rect", &json!({"x": 0, "y": 0, "w": 5, "h": 5})).unwrap();
    s.execute("file.new", &json!({"confirm": true})).unwrap();
    assert_eq!(s.plugins.len(), 1, "plug-ins belong to the session, not the document");
    // Reinstalling over the folder copy is irreversible: it asks.
    let again = json!({"path": root.join("stars.wasm").display().to_string()});
    assert!(s.confirmation("plugin.install", &again).is_some_and(|c| c.irreversible));
    assert!(s.execute("plugin.install", &again).is_err());
    let mut yes = again.clone();
    yes["confirm"] = json!(true);
    s.execute("plugin.install", &yes).unwrap();
    // Removing deletes the folder copy: it asks too.
    let rm = json!({"id": "org.test.stars"});
    assert!(s.confirmation("plugin.remove", &rm).is_some_and(|c| c.irreversible));
    assert!(s.execute("plugin.remove", &rm).is_err());
    let out = s.execute("plugin.remove", &json!({"id": "org.test.stars", "confirm": true})).unwrap();
    assert!(out["deleted"].as_str().is_some());
    assert!(!folder.join("org.test.stars.wasm").exists());
    // A broken module in the folder is reported by reload; the rest load.
    std::fs::write(folder.join("broken.wasm"), b"\0asm\x01\0\0\0nope").unwrap();
    std::fs::write(folder.join("stars.wasm"), stars()).unwrap();
    let r = s.execute("plugin.reload", &json!({})).unwrap();
    assert_eq!(r["loaded"], json!(["org.test.stars"]));
    assert_eq!(r["failed"].as_array().map(Vec::len), Some(1));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_live_preview_reverts_to_the_gesture_start() {
    let dir = temp("preview");
    let mut s = Session::new(sample::demo());
    install(&mut s, &dir, "stars.wasm", &stars());
    let (doc, steps) = (s.doc.clone(), s.history_len());
    s.begin_gesture("Test Stars");
    for count in 1..=3 {
        s.revert_gesture();
        s.execute("plugin.run", &json!({"id": "org.test.stars", "params": {"count": count}})).unwrap();
    }
    // Cancel: back to the start, no step.
    s.revert_gesture();
    s.end_gesture();
    assert!(s.doc == doc && s.history_len() == steps);
    // OK: one step for the last preview.
    s.begin_gesture("Test Stars");
    s.revert_gesture();
    s.execute("plugin.run", &json!({"id": "org.test.stars"})).unwrap();
    s.end_gesture();
    assert_eq!(s.history_len(), steps + 1);
    assert_eq!(s.undo_label(), Some("Test Stars"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hostile_requests_through_the_host_never_panic() {
    let hostile: Vec<Value> = vec![
        Value::Null,
        json!({}),
        json!([]),
        json!("x"),
        json!({"x": 1e300, "y": -1e300, "w": 1e7, "h": 1e7, "fill": "#zz"}),
        json!({"frame": u64::MAX, "layer": u64::MAX, "ids": [u64::MAX], "index": -1}),
        json!({"d": "M 0 0 C", "points": [[0, 0]], "name": "", "count": 1_000_000}),
    ];
    for mut s in [Session::default(), Session::new(sample::demo())] {
        let mut host = SessionHost { s: &mut s };
        for c in commands() {
            for p in &hostile {
                let _ = host.request("command", &json!({"op": "command", "id": c.id, "params": p}));
                let _ = host.request("command", &json!({"op": "command", "id": c.id, "params": p.to_string()}));
            }
        }
        for op in ["document", "elements", "command", "", "log", "\u{202e}"] {
            for p in &hostile {
                let _ = host.request(op, p);
                let _ = host.request(op, &json!({"op": op, "selected": p, "paths": p, "id": p}));
            }
        }
        let _ = anicraft_render::render_frame(&s.doc, 0, 0.1);
    }
}
