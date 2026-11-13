//! Kept scripts through the engine (ADR 0013 tier 1): `script.*` commands with undo, the file
//! round trip and its repairs, and that nothing runs them (Test Movie, the HTML export).

use crate::{Session, format};
use anicraft_doc::actions::{FrameScript, Language};
use anicraft_doc::{ElementKind, LayerKind, SymbolId, SymbolKind, sample};
use serde_json::{Value, json};

const AS3: &str = "import flash.events.MouseEvent;\r\n\tstop(); // “wait”\n";

fn kept(s: &Session, layer: usize, key: usize) -> Option<(Language, String)> {
    s.doc.timeline(0).layers[layer].keyframes[key].script.as_ref().and_then(FrameScript::source).map(|x| (x.language, x.text.clone()))
}

#[test]
fn a_kept_script_replaces_actions_in_one_undo_step_and_lists_verbatim() {
    let mut s = Session::new(sample::interactive());
    let layer = s.doc.timeline(0).layers[0].id.0;
    let v = s.execute("script.set", &json!({"layer": layer, "frame": 0, "text": AS3})).unwrap();
    assert_eq!((v["language"].as_str(), v["lines"].as_u64(), v["replaced"].as_str()), (Some("actionscript3"), Some(2), Some("actions")));
    assert_eq!(kept(&s, 0, 0), Some((Language::ActionScript3, AS3.to_string())), "byte for byte");
    assert_eq!(s.undo_label(), Some("Set Frame Script"));
    // The language alone; then the text again keeps that language.
    s.execute("script.set", &json!({"layer": layer, "frame": 0, "language": "actionscript1or2"})).unwrap();
    s.execute("script.set", &json!({"layer": layer, "frame": 0, "text": "stop();"})).unwrap();
    assert_eq!(kept(&s, 0, 0), Some((Language::ActionScript1or2, "stop();".into())));
    let list = s.execute("script.list", &json!({})).unwrap();
    assert_eq!(list["frames"].as_array().map(Vec::len), Some(1));
    assert_eq!((list["frames"][0]["text"].as_str(), list["frames"][0]["scene"].as_u64(), list["frames"][0]["layer_name"].as_str()), (Some("stop();"), Some(0), Some("Actions")));
    assert!(s.execute("script.list", &json!({"text": false})).unwrap()["frames"][0].get("text").is_none());
    // `actions.list` shows it too (without the text), with the timeline's "a" marker source.
    let a = s.execute("actions.list", &json!({})).unwrap();
    assert!(a["frames"].as_array().unwrap().iter().any(|f| f["script"]["language"] == "actionscript1or2"), "{a}");
    // Undo goes back step by step, to the actions it replaced.
    for _ in 0..3 {
        s.undo().unwrap();
    }
    assert!(matches!(&s.doc.timeline(0).layers[0].keyframes[0].script, Some(FrameScript::Actions(a)) if a.len() == 1));
    // Clearing a kept script leaves actions alone; an empty text clears too.
    assert_eq!(s.execute("script.clear", &json!({"layer": layer, "frame": 0})).unwrap()["cleared"], 0);
    assert!(s.doc.timeline(0).layers[0].keyframes[0].script.is_some());
    s.execute("script.set", &json!({"layer": layer, "frame": 5, "text": "trace(1);"})).unwrap();
    assert_eq!(s.execute("script.set", &json!({"layer": layer, "frame": 5, "text": "  \n"})).unwrap()["cleared"], 1);
    assert!(s.doc.timeline(0).layers[0].keyframes[1].script.is_none());
}

#[test]
fn bad_script_params_are_refused_whole() {
    let mut s = Session::new(sample::interactive());
    let layer = s.doc.timeline(0).layers[0].id.0;
    let before = s.doc.clone();
    for bad in [
        json!({}),
        json!({"layer": layer, "text": 5}),
        json!({"layer": layer, "text": "x", "language": "cobol"}),
        json!({"layer": layer, "text": "x".repeat(anicraft_doc::actions::MAX_SOURCE_BYTES + 1)}),
        json!({"layer": layer, "frame": 0, "language": "other"}),
        json!({"layer": u64::MAX, "text": "x"}),
        json!({"symbol": u64::MAX, "text": "x"}),
        json!({"layer": layer, "frame": 99_999, "text": "x"}),
    ] {
        assert!(s.execute("script.set", &bad).is_err(), "accepted {}", bad.to_string().chars().take(80).collect::<String>());
    }
    for bad in [json!({"class": "two words"}), json!({"class": "1x"}), json!({"class": "", "base": "Base"}), json!({"class": "Main", "base": "Base"}), json!({"symbol": u64::MAX, "class": "A"}), json!({})] {
        assert!(s.execute("script.set_class", &bad).is_err(), "accepted {bad}");
    }
    assert!(s.doc == before, "a refused command changes nothing");
}

#[test]
fn any_symbol_and_guide_layers_keep_scripts_and_classes() {
    let mut s = Session::new(sample::interactive());
    // A button: actions don't run there, but a script it came with is still kept.
    let button = s.doc.library.iter().find(|x| x.kind == SymbolKind::Button).unwrap().id.0;
    assert!(s.execute("actions.set", &json!({"symbol": button, "actions": [{"op": "stop"}]})).is_err());
    s.execute("script.set", &json!({"symbol": button, "text": "stop();"})).unwrap();
    assert!(s.doc.symbol(SymbolId(button)).unwrap().timeline.layers[0].keyframes[0].script.is_some());
    // A guide layer.
    s.execute("layer.select", &json!({"index": 0})).unwrap();
    s.execute("layer.set_type", &json!({"type": "guide"})).unwrap();
    assert_eq!(s.doc.timeline(0).layers[0].kind, LayerKind::Guide);
    s.execute("script.set", &json!({"frame": 0, "text": "// notes"})).unwrap();
    assert!(s.execute("actions.set", &json!({"frame": 0, "actions": [{"op": "stop"}]})).is_err(), "guides don't run actions");
    // Linkage classes and the document class.
    s.execute("script.set_class", &json!({"symbol": button, "class": "ui.PlayButton", "base": "flash.display.SimpleButton"})).unwrap();
    s.execute("script.set_class", &json!({"class": " com.example.Main "})).unwrap();
    let list = s.execute("script.list", &json!({})).unwrap();
    assert_eq!(list["document_class"], "com.example.Main");
    assert_eq!(list["classes"], json!([{"symbol": button, "name": "Play Button", "class": "ui.PlayButton", "base": "flash.display.SimpleButton"}]));
    assert_eq!(s.undo_label(), Some("Set Script Class"));
    s.execute("script.set_class", &json!({"symbol": button, "class": ""})).unwrap();
    assert!(s.doc.symbol(SymbolId(button)).unwrap().linkage.is_none());
}

/// A `.anicraft` text from its JSON, the path table first (as the reader streams it).
fn file(v: &Value) -> String {
    format!(r#"{{"format":{},"version":{},"paths":{},"document":{}}}"#, v["format"], v["version"], v["paths"], v["document"])
}

#[test]
fn kept_scripts_survive_the_file_and_hostile_ones_are_dropped_whole() {
    let mut s = Session::new(sample::interactive());
    let layer = s.doc.timeline(0).layers[0].id.0;
    let spinner = s.doc.library.iter().find(|x| x.kind == SymbolKind::MovieClip).unwrap().id.0;
    s.execute("script.set", &json!({"layer": layer, "frame": 0, "text": AS3})).unwrap();
    s.execute("script.set_class", &json!({"symbol": spinner, "class": "Spinner", "base": "flash.display.MovieClip"})).unwrap();
    s.execute("script.set_class", &json!({"class": "Main"})).unwrap();
    let text = format::to_json(&s.doc).unwrap();
    let mut v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["document"]["scenes"][0]["timeline"]["layers"][0]["keyframes"][0]["script"], json!({"source": {"language": "actionscript3", "text": AS3}}));
    assert_eq!(v["document"]["document_class"], "Main");
    let back = format::from_json(&text).unwrap();
    assert!(back == s.doc, "the file opens as the same document");
    // A file from before (no fields) opens with none; the version didn't change.
    let mut old = v.clone();
    old["document"].as_object_mut().unwrap().remove("document_class");
    let d = format::from_json(&file(&old)).unwrap();
    assert!(d.document_class.is_empty());
    assert_eq!(v["version"], 3);
    // Hostile: an empty script, one too long, a class that isn't a name, an unknown language.
    let script = &mut v["document"]["scenes"][0]["timeline"]["layers"][0]["keyframes"];
    script[0]["script"] = json!({"source": {"language": "other", "text": ""}});
    script[1]["script"] = json!({"source": {"text": "x".repeat(anicraft_doc::actions::MAX_SOURCE_BYTES + 1)}});
    v["document"]["document_class"] = json!("not a class");
    let sym = v["document"]["library"].as_array_mut().unwrap().iter_mut().find(|x| x["id"] == spinner).unwrap();
    sym["linkage"] = json!({"class": "a..b"});
    let d = format::from_json(&file(&v)).unwrap();
    assert!(d.timeline(0).layers[0].keyframes[0].script.is_none() && d.timeline(0).layers[0].keyframes[1].script.is_none());
    assert!(d.document_class.is_empty() && d.symbol(SymbolId(spinner)).unwrap().linkage.is_none());
    v["document"]["scenes"][0]["timeline"]["layers"][0]["keyframes"][2]["script"] = json!({"source": {"language": "cobol", "text": "x"}});
    assert!(format::from_json(&file(&v)).is_err(), "an unknown language is a malformed file, as an unknown action is");
}

#[test]
fn scripts_in_the_subset_run_and_the_rest_are_kept_and_said_so() {
    let mut s = Session::new(sample::interactive());
    let layer = s.doc.timeline(0).layers[0].id.0;
    // A script that stops the movie and traces, on the show keyframe instead of its actions.
    let v = s.execute("script.set", &json!({"layer": layer, "frame": 1, "text": "trace(\"xyzzy-\" + 1); stop();"})).unwrap();
    assert_eq!((v["runs"].as_bool(), v["problems"].as_array().map(Vec::len)), (Some(true), Some(0)), "{v}");
    let v = s.execute("control.test_movie", &json!({"frames": 20, "events": [{"tick": 2, "type": "click", "x": 640, "y": 400}]})).unwrap();
    let lines: Vec<&str> = v["output"].as_array().unwrap().iter().filter_map(|l| l["line"].as_str()).collect();
    assert_eq!(lines.first().copied(), Some("Frame script runs: Scene 1 › Actions, frame 2"), "{lines:?}");
    assert!(lines.contains(&"xyzzy-1"), "{lines:?}");
    assert!(v["state"]["frame"].as_u64().unwrap() < 10 && v["state"]["playing"] == false, "it stopped: {}", v["state"]);
    // The HTML export runs it too, as the same actions.
    let out = anicraft_render::html::to_html(&s.doc, &anicraft_render::html::HtmlOptions::default()).unwrap();
    assert!(out.html.contains("xyzzy-") && !out.report.keys().any(|k| k.starts_with("kept frame scripts")), "{:?}", out.report);
    // One outside the subset doesn't run at all, and Test Movie says where and why.
    let v = s.execute("script.set", &json!({"layer": layer, "frame": 1, "text": "stop();
var n:int = 1; switch (n) { }"})).unwrap();
    assert_eq!(v["runs"], false);
    assert_eq!(v["problems"][0], json!({"line": 2, "column": 16, "message": "`switch` statements aren't supported yet"}));
    let v = s.execute("control.test_movie", &json!({"frames": 20, "events": [{"tick": 2, "type": "click", "x": 640, "y": 400}]})).unwrap();
    assert_eq!(v["output"][0]["line"], "Frame script not run: Scene 1 › Actions, frame 2: line 2, column 16: `switch` statements aren't supported yet");
    assert!(v["state"]["frame"].as_u64().unwrap() > 10, "it played on: {}", v["state"]);
    let out = anicraft_render::html::to_html(&s.doc, &anicraft_render::html::HtmlOptions::default()).unwrap();
    assert!(out.report.keys().any(|k| k.starts_with("kept frame scripts")), "{:?}", out.report);
    assert!(!out.html.contains("var n:int"), "a kept script's text isn't exported");
    let list = s.execute("script.list", &json!({"text": false})).unwrap();
    assert_eq!((list["frames"][0]["runs"].as_bool(), list["frames"][0]["problems"][0]["line"].as_u64()), (Some(false), Some(2)));
    // Plug-ins may read and write kept scripts, as they may actions.
    for id in ["script.set", "script.clear", "script.list", "script.set_class", "script.check"] {
        assert!(crate::cmds_plugin::plugin_may_call(id).is_ok(), "{id}");
    }
}

#[test]
fn script_check_says_what_a_text_would_do_and_changes_nothing() {
    let mut s = Session::new(sample::interactive());
    let before = s.doc.clone();
    let v = s.execute("script.check", &json!({"text": "stop();
btn.addEventListener(MouseEvent.CLICK, go);
function go(e:MouseEvent):void { gotoAndPlay(\"run\"); }
navigateToURL(new URLRequest(\"x\"));"})).unwrap();
    assert_eq!(v["runs"], true);
    assert_eq!(v["steps"], 3);
    assert_eq!(v["program"]["body"][1]["s"], "listen");
    assert_eq!(v["program"]["body"][1]["at"]["path"], "btn");
    assert_eq!(v["program"]["functions"][0]["name"], "go");
    assert_eq!(v["notes"][0]["line"], 4);
    // Names from other frames are listed; the text alone can't say whether they are there.
    let v = s.execute("script.check", &json!({"text": "score += bonus(2);"})).unwrap();
    assert_eq!((v["runs"].as_bool(), v["uses"].clone()), (Some(true), json!([{"name": "score", "line": 1, "column": 1, "call": false}, {"name": "bonus", "line": 1, "column": 10, "call": true}])));
    let v = s.execute("script.check", &json!({"text": "stop();", "language": "actionscript1or2"})).unwrap();
    assert_eq!((v["runs"].as_bool(), v["problems"][0]["line"].as_u64()), (Some(false), Some(0)));
    assert!(s.execute("script.check", &json!({})).is_err());
    assert!(s.doc == before && s.undo_label().is_none());
}

#[test]
fn scripts_link_through_their_timeline_and_test_movie_plays_keys() {
    let mut s = Session::new(sample::interactive());
    let layer = s.doc.timeline(0).layers[0].id.0;
    // Frame 2 uses a variable and a function that no frame declares yet: it doesn't run.
    let v = s.execute("script.set", &json!({"layer": layer, "frame": 1, "text": "stop();\nstage.addEventListener(KeyboardEvent.KEY_DOWN, onKey);\nfunction onKey(e:KeyboardEvent):void { hits++; trace(\"key \" + e.keyCode + \" \" + hits + \" \" + Math.floor(Math.random() * 1000)); }"})).unwrap();
    assert_eq!(v["runs"], false, "{v}");
    assert_eq!(v["problems"][0]["message"], "`hits` isn't a variable of this timeline (declare it with `var` in one of its frame scripts)");
    let out = s.execute("control.test_movie", &json!({"frames": 5})).unwrap();
    assert!(out["output"][0]["line"].as_str().unwrap().starts_with("Frame script not run: Scene 1 › Actions, frame 2: line 3"), "{}", out["output"]);
    // Declaring it on frame 1 links frame 2.
    s.execute("script.set", &json!({"layer": layer, "frame": 0, "text": "var hits:int = 0;"})).unwrap();
    let list = s.execute("script.list", &json!({"text": false})).unwrap();
    assert!(list["frames"].as_array().unwrap().iter().all(|f| f["runs"] == true), "{list}");
    let play = |s: &mut Session, seed: u64| {
        let v = s.execute("control.test_movie", &json!({"frames": 6, "seed": seed, "events": [{"tick": 3, "type": "key_down", "key": "LEFT"}, {"tick": 4, "type": "key", "key": "a"}, {"tick": 5, "type": "key_down", "key": 32}]})).unwrap();
        v["output"].as_array().unwrap().iter().filter_map(|l| l["line"].as_str().map(str::to_string)).filter(|l| l.starts_with("key ")).collect::<Vec<_>>()
    };
    let a = play(&mut s, 1);
    assert_eq!(a.len(), 3, "{a:?}");
    assert!(a[0].starts_with("key 37 1 ") && a[1].starts_with("key 65 2 ") && a[2].starts_with("key 32 3 "), "{a:?}");
    assert_eq!(play(&mut s, 1), a, "the same seed plays the same");
    assert_ne!(play(&mut s, 2), a, "another seed draws other numbers");
    for bad in [json!({"tick": 1, "type": "key_down"}), json!({"tick": 1, "type": "key_down", "key": 0}), json!({"tick": 1, "type": "key_up", "key": "WIGGLE"})] {
        assert!(s.execute("control.test_movie", &json!({"events": [bad]})).is_err());
    }
}

/// Dynamic and input text fields (ADR 0066), end to end through commands: Text type, instance
/// names, the file format, and a headless Test Movie that types into a field.
#[test]
fn text_fields_are_made_named_saved_and_typed_into() {
    if anicraft_text::Fonts::get().families().is_empty() {
        return; // A computer without fonts: the player tests cover fields with a test layout.
    }
    let mut s = Session::default();
    let ok = |s: &mut Session, id: &str, p: Value| s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"));
    ok(&mut s, "text.create", json!({"text": "0", "x": 20, "y": 20, "size": 20}));
    assert!(s.execute("instance.set_name", &json!({"name": "score_txt"})).unwrap_err().to_string().contains("static text has no instance name"));
    assert!(s.execute("text.field", &json!({"type": "dynamic", "lines": "password"})).unwrap_err().to_string().contains("only input text"));
    ok(&mut s, "text.field", json!({"type": "dynamic", "width": 200, "border": true}));
    ok(&mut s, "instance.set_name", json!({"name": "score_txt"}));
    let field = |s: &Session, name: &str| s.doc.timeline(0).layers[0].keyframes[0].elements.iter().find(|e| e.name == name).cloned().unwrap();
    let ElementKind::Text(t) = &field(&s, "score_txt").kind else { panic!("text") };
    assert_eq!((t.kind(), t.box_width), (anicraft_doc::TextKind::Dynamic, 200.0));
    assert!(t.outline.bounds().min.x >= anicraft_doc::FIELD_GUTTER - 0.5, "the text sits inside the gutter");
    // An empty input field, with a script reading it when Enter is pressed.
    ok(&mut s, "text.create", json!({"text": "", "type": "input", "x": 20, "y": 80, "size": 20}));
    ok(&mut s, "text.field", json!({"width": 200, "height": 30, "max_chars": 5}));
    ok(&mut s, "instance.set_name", json!({"name": "name_txt"}));
    ok(&mut s, "script.set", json!({"frame": 0, "text": "stop();\nstage.addEventListener(KeyboardEvent.KEY_DOWN, k);\nfunction k(e:KeyboardEvent):void { if (e.keyCode == Keyboard.ENTER) { score_txt.text = \"Hi \" + name_txt.text; trace(name_txt.length); } }"}));
    // Saved and opened again, the fields are the same.
    let dir = std::env::temp_dir().join(format!("anicraft-fields-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("fields.anicraft");
    ok(&mut s, "file.save", json!({"path": file.display().to_string()}));
    let mut back = Session::default();
    ok(&mut back, "file.open", json!({"path": file.display().to_string()}));
    assert_eq!(field(&back, "name_txt").kind, field(&s, "name_txt").kind);
    std::fs::remove_dir_all(&dir).ok();
    // Headless Test Movie: click the input field, type past its limit, press Enter.
    let v = ok(&mut back, "control.test_movie", json!({"frames": 3, "events": [{"tick": 1, "type": "click", "x": 40, "y": 92}, {"tick": 1, "type": "text", "text": "Adalovelace"}, {"tick": 2, "type": "key", "key": "ENTER"}]}));
    let fields = v["state"]["fields"].as_array().unwrap();
    let text_of = |n: &str| fields.iter().find(|f| f["name"] == n).map(|f| f["text"].clone()).unwrap();
    assert_eq!(text_of("name_txt"), "Adalo", "at most 5 characters");
    assert_eq!(text_of("score_txt"), "Hi Adalo");
    assert!(v["output"].as_array().unwrap().iter().any(|l| l["line"] == "5"), "{}", v["output"]);
    assert!(back.execute("control.test_movie", &json!({"events": [{"type": "text"}]})).is_err());
    // Plug-ins may set text types (document content, like the rest of text.*).
    assert!(crate::cmds_plugin::plugin_may_call("text.field").is_ok());
}
