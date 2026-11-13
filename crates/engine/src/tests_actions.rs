//! Interactivity through the engine (ADR 0024): frame actions and event handlers as commands,
//! instance names, the file round trip, frame scripts on the plug-in host (with their fuel
//! limit), and headless Test Movie.

use crate::plugin_tests::{plugin, plugin_then};
use crate::player::{Movie, ScriptRunner};
use crate::{Session, format};
use anicraft_doc::actions::{Action, FrameScript};
use anicraft_doc::{ElementKind, SymbolKind, sample};
use anicraft_plugins::Limits;
use serde_json::{Value, json};

/// A session with a movie clip instance named on the active layer, selected.
fn with_instance(kind: &str) -> (Session, u64) {
    let mut s = Session::default();
    let id = s.execute("draw.rect", &json!({"x": 100, "y": 100, "w": 80, "h": 40})).unwrap()["element"].as_u64().unwrap();
    s.execute("element.select", &json!({"ids": [id]})).unwrap();
    let v = s.execute("modify.convert_to_symbol", &json!({"name": "Thing"})).unwrap();
    let inst = v["instance"].as_u64().or_else(|| s.selection.first().map(|(_, e)| e.0)).unwrap();
    s.execute("symbol.set_kind", &json!({"kind": kind})).unwrap();
    (s, inst)
}

#[test]
fn frame_actions_are_set_listed_cleared_and_undone_as_commands() {
    let mut s = Session::new(sample::interactive());
    let actions_layer = s.doc.timeline(0).layers[0].id.0;
    let v = s.execute("actions.set", &json!({"layer": actions_layer, "frame": 5, "actions": [{"op": "stop"}, {"op": "trace", "message": "hi"}]})).unwrap();
    assert_eq!(v["keyframe"], 1, "goes on the keyframe covering the frame");
    assert_eq!(v["summary"], "Stop; Trace “hi”");
    let list = s.execute("actions.list", &json!({})).unwrap();
    assert_eq!(list["frames"].as_array().unwrap().len(), 3);
    assert_eq!(list["instances"][0]["name"], "play_btn");
    assert_eq!(list["instances"][0]["events"][0]["event"], "click");
    // One undo step, named after the command.
    assert_eq!(s.undo_label(), Some("Set Actions"));
    s.undo().unwrap();
    assert!(s.doc.timeline(0).layers[0].keyframes[1].script.as_ref().is_some_and(|x| matches!(x, FrameScript::Actions(a) if a.len() == 1)));
    // Bad lists are refused whole.
    for bad in [json!({"actions": [{"op": "explode"}]}), json!({"actions": [{"op": "stop", "target": "a b"}]}), json!({"actions": "stop"}), json!({}), json!({"plugin": "bad id!"})] {
        assert!(s.execute("actions.set", &bad).is_err(), "accepted {bad}");
    }
    s.execute("actions.clear", &json!({"layer": actions_layer, "frame": 0})).unwrap();
    assert!(s.doc.timeline(0).layers[0].keyframes[0].script.is_none());
    // An empty list clears too; a frame script reference replaces actions.
    s.execute("actions.set", &json!({"layer": actions_layer, "frame": 1, "plugin": "org.example.menu", "entry": "intro"})).unwrap();
    assert!(matches!(&s.doc.timeline(0).layers[0].keyframes[1].script, Some(FrameScript::Plugin(p)) if p.entry == "intro"));
    s.execute("actions.set", &json!({"layer": actions_layer, "frame": 1, "actions": []})).unwrap();
    assert!(s.doc.timeline(0).layers[0].keyframes[1].script.is_none());
}

#[test]
fn movie_clip_timelines_take_actions_by_symbol() {
    let mut s = Session::new(sample::interactive());
    let spinner = s.doc.library.iter().find(|x| x.kind == SymbolKind::MovieClip).unwrap().id.0;
    let button = s.doc.library.iter().find(|x| x.kind == SymbolKind::Button).unwrap().id.0;
    s.execute("actions.set", &json!({"symbol": spinner, "frame": 6, "actions": [{"op": "stop"}]})).unwrap();
    assert!(s.doc.symbol(anicraft_doc::SymbolId(spinner)).unwrap().timeline.layers[0].keyframes[0].script.is_some());
    let e = s.execute("actions.set", &json!({"symbol": button, "actions": [{"op": "stop"}]})).unwrap_err().to_string();
    assert!(e.contains("not a movie clip"), "{e}");
}

#[test]
fn instances_get_names_and_event_handlers() {
    let (mut s, inst) = with_instance("button");
    s.execute("instance.set_name", &json!({"name": "start_btn"})).unwrap();
    for bad in ["two words", "1st", "root", "x".repeat(65).as_str()] {
        assert!(s.execute("instance.set_name", &json!({"name": bad})).is_err(), "accepted {bad:?}");
    }
    s.execute("actions.set", &json!({"element": inst, "event": "click", "actions": [{"op": "goto_and_stop", "frame": 3}]})).unwrap();
    s.execute("actions.set", &json!({"element": inst, "event": "over", "actions": [{"op": "trace", "message": "over"}]})).unwrap();
    let e = s.visible_elements().into_iter().find(|e| e.id.0 == inst).unwrap();
    assert_eq!(e.name, "start_btn");
    let ElementKind::Symbol { events, .. } = &e.kind else { panic!("an instance") };
    assert_eq!(events.iter().map(|h| h.event.key()).collect::<Vec<_>>(), ["click", "over"]);
    s.execute("actions.clear", &json!({"element": inst, "event": "click"})).unwrap();
    s.execute("actions.set", &json!({"element": inst, "event": "over", "actions": []})).unwrap();
    let e = s.visible_elements().into_iter().find(|e| e.id.0 == inst).unwrap();
    assert!(matches!(&e.kind, ElementKind::Symbol { events, .. } if events.is_empty()));
    // Graphics and shapes can't be addressed.
    let (mut g, gi) = with_instance("graphic");
    assert!(g.execute("instance.set_name", &json!({"element": gi, "name": "x"})).unwrap_err().to_string().contains("graphic"));
    let shape = g.execute("draw.rect", &json!({"x": 0, "y": 0, "w": 5, "h": 5})).unwrap()["element"].as_u64().unwrap();
    assert!(g.execute("actions.set", &json!({"element": shape, "event": "click", "actions": []})).is_err());
}

/// A `.anicraft` text from its JSON, the path table first (as the reader streams it).
fn file(v: &Value) -> String {
    format!(r#"{{"format":{},"version":{},"paths":{},"document":{}}}"#, v["format"], v["version"], v["paths"], v["document"])
}

#[test]
fn actions_and_handlers_survive_the_file_and_older_files_still_open() {
    let d = sample::interactive();
    let text = format::to_json(&d).unwrap();
    let mut v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["document"]["scenes"][0]["timeline"]["layers"][0]["keyframes"][0]["script"], json!({"actions": [{"op": "stop"}]}), "compact actions");
    assert_eq!(v["document"]["scenes"][0]["timeline"]["layers"][1]["keyframes"][0]["elements"][1]["kind"]["Symbol"]["events"][0]["event"], "click");
    let back = format::from_json(&text).unwrap();
    assert_eq!(back, d);
    // A file without the fields (as written before they existed) opens with none.
    v["document"]["scenes"][0]["timeline"]["layers"][0]["keyframes"][0].as_object_mut().unwrap().remove("script");
    let d2 = format::from_json(&file(&v)).unwrap();
    assert!(d2.timeline(0).layers[0].keyframes[0].script.is_none());
    // Hostile scripts are repaired on load, not trusted.
    v["document"]["scenes"][0]["timeline"]["layers"][0]["keyframes"][0]["script"] = json!({"actions": [{"op": "stop", "target": "no such..path"}, {"op": "set_property", "property": "alpha", "value": 1e300}]});
    let d3 = format::from_json(&file(&v)).unwrap();
    let a = d3.timeline(0).layers[0].keyframes[0].script.as_ref().unwrap().actions().to_vec();
    assert!(matches!(a.as_slice(), [Action::SetProperty { value, .. }] if *value == 1.0), "{a:?}");
}

#[test]
fn test_movie_plays_headless_with_clicks() {
    let mut s = Session::new(sample::interactive());
    let still = s.execute("control.test_movie", &json!({"frames": 10})).unwrap();
    assert_eq!(still["state"]["frame"], 0, "the menu stops on its first frame");
    assert!(still["output"].as_array().unwrap().is_empty());
    let v = s.execute("control.test_movie", &json!({"frames": 50, "events": [{"tick": 2, "type": "click", "x": 640, "y": 400}]})).unwrap();
    let lines: Vec<&str> = v["output"].as_array().unwrap().iter().filter_map(|l| l["line"].as_str()).collect();
    assert_eq!(lines, ["The show starts", "Back to the menu"]);
    assert_eq!(v["state"]["playing"], false);
    assert!(s.execute("control.test_movie", &json!({"events": [{"tick": 1, "type": "wiggle", "x": 0, "y": 0}]})).is_err());
    assert!(s.execute("control.test_movie", &json!({"events": [{"type": "move"}]})).is_err());
    assert!(!s.is_dirty(), "testing never edits the document");
}

const SCRIPT: &str = r#"{"id":"org.test.fs","name":"Stopper","kind":"frame-script","entries":["main","loop"]}"#;

/// A session whose second keyframe on the Actions layer runs `entry` of the test script.
fn scripted(module: &[u8], entry: &str) -> Session {
    let mut s = Session::new(sample::interactive());
    s.plugins.install_bytes(module).unwrap();
    let layer = s.doc.timeline(0).layers[0].id.0;
    s.execute("actions.set", &json!({"layer": layer, "frame": 0, "actions": [{"op": "trace", "message": "menu"}]})).unwrap();
    s.execute("actions.set", &json!({"layer": layer, "frame": 1, "plugin": "org.test.fs", "entry": entry})).unwrap();
    s
}

fn play(s: &Session, steps: u32) -> (Movie, u64) {
    let mut runner = ScriptRunner::new(&s.plugins);
    let mut m = Movie::start(&s.doc, &mut runner);
    for _ in 0..steps {
        m.step(&s.doc, &[], &mut runner);
    }
    (m, runner.fuel_used)
}

#[test]
fn frame_scripts_read_the_player_and_emit_actions() {
    let module = plugin(SCRIPT, &[r#"{"op":"player"}"#, r#"{"op":"log","message":"stopping"}"#, r#"{"op":"action","action":{"op":"stop"}}"#], true, 0);
    let s = scripted(&module, "main");
    let (m, fuel) = play(&s, 6);
    assert_eq!((m.state.root.frame, m.state.root.playing), (1, false), "the script's stop held frame 2");
    assert_eq!(m.output.iter().map(|(_, l)| l.as_str()).collect::<Vec<_>>(), ["menu", "stopping"]);
    assert!(fuel > 0);
    // Commands are out of reach, and a bad action is an answer the script sees, not a crash.
    let sneaky = plugin(SCRIPT, &[r#"{"op":"command","id":"draw.rect","params":{"x":0,"y":0,"w":9,"h":9}}"#], true, 0);
    let s2 = scripted(&sneaky, "main");
    let (m, _) = play(&s2, 3);
    assert!(m.output.iter().any(|(_, l)| l.contains("returned error code 1")), "{:?}", m.output);
    assert_eq!(s2.doc, scripted(&sneaky, "main").doc, "nothing was drawn");
    let bad = plugin(SCRIPT, &[r#"{"op":"action","action":{"op":"stop","target":"!!"}}"#], true, 0);
    let (m, _) = play(&scripted(&bad, "main"), 3);
    assert!(m.state.root.playing, "the refused action didn't run");
    // Unknown entry points and command plug-ins are reported.
    let s3 = scripted(&module, "nope");
    let (m, _) = play(&s3, 2);
    assert!(m.output.iter().any(|(_, l)| l.contains("no entry point “nope”")));
    // plugin.run won't run a frame script as a command.
    let mut s4 = scripted(&module, "main");
    assert!(s4.execute("plugin.run", &json!({"id": "org.test.fs"})).unwrap_err().to_string().contains("frame script"));
}

#[test]
fn a_runaway_frame_script_costs_one_frames_budget_and_the_movie_goes_on() {
    let spin = plugin_then(SCRIPT, &[], false, "(loop $l (br $l))", 0);
    let s = scripted(&spin, "loop");
    let t0 = std::time::Instant::now();
    let mut runner = ScriptRunner::new(&s.plugins).with_limits(Limits { fuel: 2_000_000, ..Limits::frame_script() });
    let mut m = Movie::start(&s.doc, &mut runner);
    for _ in 0..4 {
        m.step(&s.doc, &[], &mut runner);
    }
    assert!(m.output.iter().any(|(_, l)| l.contains("instruction budget")), "{:?}", m.output);
    assert_eq!(m.state.root.frame, 4, "playback carried on");
    assert_eq!(runner.fuel_used, 2_000_000, "exactly one frame's budget");
    assert!(t0.elapsed().as_millis() < 2_000);
    // The default budget per frame is 20 M instructions, shared by the frame's scripts.
    assert_eq!(Limits::frame_script().fuel, 20_000_000);
}

#[test]
fn every_new_command_shrugs_off_hostile_params() {
    let hostile = [
        Value::Null,
        json!({"actions": [{"op": "goto_and_stop", "frame": "x".repeat(10_000)}]}),
        json!({"actions": (0..300).map(|_| json!({"op": "stop"})).collect::<Vec<_>>()}),
        json!({"element": u64::MAX, "event": "click", "actions": []}),
        json!({"symbol": u64::MAX, "frame": u32::MAX}),
        json!({"frames": u64::MAX, "events": [{"tick": 1, "type": "click", "x": f64::MAX, "y": 0}]}),
        json!({"name": "é"}),
    ];
    for id in ["actions.set", "actions.clear", "actions.list", "instance.set_name", "control.test_movie"] {
        let mut s = Session::new(sample::interactive());
        for p in &hostile {
            let _ = s.execute(id, p);
        }
    }
}
