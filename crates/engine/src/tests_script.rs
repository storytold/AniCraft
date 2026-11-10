//! Kept scripts through the engine (ADR 0013 tier 1): `script.*` commands with undo, the file
//! round trip and its repairs, and that nothing runs them (Test Movie, the HTML export).

use crate::{Session, format};
use anicraft_doc::actions::{FrameScript, Language};
use anicraft_doc::{LayerKind, SymbolId, SymbolKind, sample};
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
    assert_eq!(v["version"], crate::format::FORMAT_VERSION, "the fork carries the maop_maohou sidecar");
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
fn nothing_runs_a_kept_script() {
    let mut s = Session::new(sample::interactive());
    let layer = s.doc.timeline(0).layers[0].id.0;
    // A script that would stop the movie and trace, on the show keyframe instead of its actions.
    s.execute("script.set", &json!({"layer": layer, "frame": 1, "text": "trace(\"xyzzy-kept\"); stop();"})).unwrap();
    let v = s.execute("control.test_movie", &json!({"frames": 20, "events": [{"tick": 2, "type": "click", "x": 640, "y": 400}]})).unwrap();
    let lines: Vec<&str> = v["output"].as_array().unwrap().iter().filter_map(|l| l["line"].as_str()).collect();
    assert!(lines.is_empty(), "{lines:?}");
    assert!(v["state"]["frame"].as_u64().unwrap() > 10, "it played on: {}", v["state"]);
    // The HTML export draws the movie and says the script isn't in it.
    let out = anicraft_render::html::to_html(&s.doc, &anicraft_render::html::HtmlOptions::default()).unwrap();
    assert!(out.report.keys().any(|k| k.starts_with("kept frame scripts")), "{:?}", out.report);
    assert!(!out.html.contains("xyzzy-kept"), "the script's text isn't exported");
    // Plug-ins may read and write kept scripts, as they may actions.
    for id in ["script.set", "script.clear", "script.list", "script.set_class"] {
        assert!(crate::cmds_plugin::plugin_may_call(id).is_ok(), "{id}");
    }
}
