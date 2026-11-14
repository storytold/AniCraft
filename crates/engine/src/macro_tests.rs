//! Saved commands (ADR 0097): what history records, Save As Command from history steps, a run
//! as one transactional undo step, the folder's order, the policy, and hostile files.

use super::*;
use crate::cmds_macro::{self, Macro, macro_may_call};
use serde_json::json;
use std::path::PathBuf;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    match s.execute(id, &p) {
        Ok(v) => v,
        Err(e) => panic!("{id} {p}: {e}"),
    }
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("anicraft-macros-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The elements on the active layer at the playhead, without their ids (which differ between
/// documents), as JSON.
fn looks(s: &Session) -> Vec<Value> {
    s.visible_elements()
        .iter()
        .map(|e| {
            let mut v = serde_json::to_value(&**e).unwrap();
            v.as_object_mut().unwrap().remove("id");
            v
        })
        .collect()
}

/// A rectangle, selected.
fn with_rect(x: f64) -> Session {
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": x, "y": 20, "w": 80, "h": 40}));
    run(&mut s, "edit.select_all", json!({}));
    s
}

/// Three edits on the selection: a drag (many moves in one step), a quarter turn and a fade.
fn edit(s: &mut Session) {
    s.begin_gesture("Move");
    for _ in 0..30 {
        run(s, "element.translate", json!({"dx": 2, "dy": -1}));
    }
    s.end_gesture();
    run(s, "modify.rotate_cw", json!({}));
    run(s, "element.set_color_effect", json!({"style": "alpha", "alpha": 40}));
}

#[test]
fn history_keeps_each_steps_commands_and_a_drag_as_one_move() {
    let mut s = with_rect(10.0);
    edit(&mut s);
    let steps: Vec<(String, Option<Vec<Recorded>>)> = s.history().steps().map(|(l, c)| (l.to_string(), c.map(<[Recorded]>::to_vec))).collect();
    assert_eq!(steps.len(), 4, "{:?}", steps.iter().map(|s| &s.0).collect::<Vec<_>>());
    let cmds = |i: usize| steps[i].1.clone().unwrap();
    assert_eq!(cmds(0)[0].cmd, "draw.rect");
    // Thirty moves in a drag add up to one.
    assert_eq!(cmds(1).len(), 1);
    assert_eq!(cmds(1)[0].params, json!({"dx": 60.0, "dy": -30.0}));
    assert_eq!(cmds(2)[0].cmd, "modify.rotate_cw");
    assert_eq!(cmds(3)[0].params, json!({"style": "alpha", "alpha": 40}));
    // Undo and redo carry them along.
    s.undo().unwrap();
    s.redo().unwrap();
    assert_eq!(s.history().steps().nth(3).and_then(|(_, c)| c).map(<[Recorded]>::len), Some(1));
}

#[test]
fn a_live_preview_records_only_what_it_ended_on() {
    let mut s = with_rect(10.0);
    s.begin_gesture("Nudge");
    run(&mut s, "modify.rotate_cw", json!({}));
    s.revert_gesture();
    run(&mut s, "element.translate", json!({"dx": 5, "dy": 0}));
    s.end_gesture();
    let last = s.history().steps().last().and_then(|(_, c)| c.map(<[Recorded]>::to_vec)).unwrap();
    assert_eq!(last.iter().map(|r| r.cmd.as_str()).collect::<Vec<_>>(), ["element.translate"]);
}

#[test]
fn record_and_replay_equals_the_original_edits_on_a_fresh_selection() {
    let dir = temp_dir("replay");
    let folder = dir.display().to_string();
    let mut original = with_rect(10.0);
    edit(&mut original);
    let listed = run(&mut original, "macro.history", json!({}));
    let keep: Vec<bool> = listed["steps"].as_array().unwrap().iter().map(|v| v["keep"].as_bool().unwrap()).collect();
    assert_eq!(keep, [true, true, true, true], "{listed}");
    let saved = run(&mut original, "macro.save", json!({"folder": folder, "name": "Turn and Fade", "history": [1, 2, 3]}));
    assert_eq!(saved["steps"], 3);
    assert!(dir.join("Turn and Fade.json").is_file());

    // Elsewhere: another document, another rectangle drawn the same way but with other ids.
    let mut other = Session::default();
    run(&mut other, "draw.ellipse", json!({"x": 300, "y": 300, "w": 10, "h": 10}));
    run(&mut other, "edit.deselect", json!({}));
    run(&mut other, "draw.rect", json!({"x": 10, "y": 20, "w": 80, "h": 40}));
    let rect = other.visible_elements().last().unwrap().id;
    run(&mut other, "element.select", json!({"ids": [rect.0]}));
    let steps = other.history_len();
    let before_run = looks(&other);
    let oval = before_run.first().cloned();
    let v = run(&mut other, "macro.run", json!({"folder": folder, "name": "Turn and Fade"}));
    assert_eq!(v["steps"], 3);
    assert_eq!(other.history_len(), steps + 1, "one undo step");
    assert_eq!(other.undo_label(), Some("Turn and Fade"));
    let mine = looks(&other);
    assert_eq!(mine.last(), looks(&original).last(), "the rectangle ends up as in the original");
    // The oval, not selected, is untouched.
    assert_eq!(mine.first().cloned(), oval);
    // One undo takes the whole run back.
    other.undo().unwrap();
    assert_eq!(looks(&other), before_run);
    // And the run's step can itself be saved again: history keeps its steps, not macro.run.
    other.redo().unwrap();
    let again = run(&mut other, "macro.history", json!({}));
    let last = again["steps"].as_array().unwrap().last().unwrap().clone();
    assert_eq!((last["keep"].as_bool(), last["commands"].as_u64()), (Some(true), Some(3)), "{last}");
}

#[test]
fn a_failing_step_changes_nothing_and_is_named() {
    let mut s = with_rect(10.0);
    let (doc, state, steps) = (s.doc.clone(), s.state, s.history_len());
    let steps_json = json!([
        {"cmd": "element.translate", "params": {"dx": 10, "dy": 0}},
        {"cmd": "modify.rotate_cw"},
        {"cmd": "layer.set_height", "params": {"height": 7}, "label": "Taller row"},
    ]);
    let e = s.execute("macro.run", &json!({"name": "Broken", "steps": steps_json})).unwrap_err().to_string();
    assert!(e.contains("“Broken” stopped at step 3 of 3 (Taller row)") && e.contains("Nothing was changed"), "{e}");
    assert!(s.doc == doc, "the document is as it was");
    assert_eq!((s.state, s.history_len()), (state, steps));
    assert_eq!(s.selection.len(), 1, "the selection too");
}

#[test]
fn steps_tied_to_this_document_act_on_the_selection_or_say_why_they_cant_be_kept() {
    let mut s = with_rect(10.0);
    let layer = s.layer.0;
    let id = s.selection.id_list()[0];
    // A layer named by id: the step acts on the active layer instead.
    run(&mut s, "layer.set_color", json!({"color": "#ff0000", "layer": layer}));
    // An element named by id where the command can take the selection instead.
    run(&mut s, "stroke.set_profile", json!({"profile": "taper_both", "id": id}));
    // An element named by id where the command can't do without it.
    run(&mut s, "path.move_points", json!({"id": id, "points": [[0, 0]], "dx": 3, "dy": 0}));
    let v = run(&mut s, "macro.history", json!({}));
    let steps = v["steps"].as_array().unwrap();
    assert_eq!(steps[1]["rewritten"], "acts on the active layer", "{v}");
    assert_eq!(steps[2]["rewritten"], "acts on the selection", "{v}");
    assert_eq!(steps[3]["keep"], false);
    assert!(steps[3]["why"].as_str().unwrap().contains("particular objects"), "{v}");
    let dir = temp_dir("rewrite");
    let folder = dir.display().to_string();
    let e = s.execute("macro.save", &json!({"folder": folder, "name": "Mixed", "history": [1, 2, 3]})).unwrap_err().to_string();
    assert!(e.starts_with("step 4 “") && e.contains("can't be kept"), "{e}");
    assert!(!dir.join("Mixed.json").exists(), "nothing saved");
    let saved = run(&mut s, "macro.save", json!({"folder": folder, "name": "Mixed", "history": [2, 1]}));
    assert_eq!(saved["rewritten"].as_array().map(Vec::len), Some(2));
    let m = cmds_macro::load(&dir.join("Mixed.json")).unwrap();
    assert_eq!(m.steps[0].params, json!({"color": "#ff0000"}), "in history order, the layer's id gone");
    assert_eq!(m.steps[1].params, json!({"profile": "taper_both"}));
}

#[test]
fn saved_commands_call_only_what_a_plug_in_may() {
    assert!(macro_may_call("element.translate").is_ok());
    for id in ["file.save", "edit.paste", "edit.undo", "plugin.run", "macro.run", "macro.delete", "prefs.set", "view.rulers"] {
        assert!(macro_may_call(id).is_err(), "{id}");
    }
    // Plug-ins can't reach saved commands either, so the two never call each other.
    for c in commands().iter().filter(|c| c.id.starts_with("macro.")) {
        assert!(crate::cmds_plugin::plugin_may_call(c.id).is_err(), "{}", c.id);
    }
    for (step, why) in [(json!({"cmd": "file.save", "params": {"path": "x.anicraft"}}), "files"), (json!({"cmd": "macro.run", "params": {"path": "x.json"}}), "saved commands"), (json!({"cmd": "no.such"}), "unknown command")] {
        let e = Macro::from_json(&json!([step]), "x").unwrap_err().to_string();
        assert!(e.contains("step 1") && e.contains(why), "{e}");
    }
    // A history step that used the clipboard can't be kept.
    let mut s = with_rect(10.0);
    run(&mut s, "edit.copy", json!({}));
    run(&mut s, "edit.paste", json!({}));
    let v = run(&mut s, "macro.history", json!({}));
    let last = v["steps"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(last["keep"], false);
    assert!(last["why"].as_str().unwrap().contains("clipboard"), "{last}");
}

#[test]
fn the_folder_keeps_order_renames_and_deletes_only_when_confirmed() {
    let dir = temp_dir("manage");
    let folder = dir.display().to_string();
    let mut s = with_rect(10.0);
    run(&mut s, "modify.rotate_cw", json!({}));
    for name in ["Alpha", "Bravo", "Charlie"] {
        run(&mut s, "macro.save", json!({"folder": folder, "name": name, "history": [1]}));
    }
    let names = |s: &mut Session| -> Vec<String> { run(s, "macro.list", json!({"folder": folder}))["commands"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap().to_string()).collect() };
    assert_eq!(names(&mut s), ["Alpha", "Bravo", "Charlie"], "in the order saved");
    run(&mut s, "macro.move", json!({"folder": folder, "name": "Charlie", "index": 0}));
    assert_eq!(names(&mut s), ["Charlie", "Alpha", "Bravo"]);
    run(&mut s, "macro.rename", json!({"folder": folder, "name": "Alpha", "to": "Delta"}));
    assert_eq!(names(&mut s), ["Charlie", "Delta", "Bravo"], "a rename keeps its place");
    assert!(s.execute("macro.rename", &json!({"folder": folder, "name": "Delta", "to": "Bravo"})).unwrap_err().to_string().contains("already"));
    assert!(s.execute("macro.rename", &json!({"folder": folder, "name": "Delta", "to": "a/b"})).is_err());
    // Saving over one asks first; deleting always does (a file: undo can't bring it back).
    assert!(matches!(s.execute("macro.save", &json!({"folder": folder, "name": "Bravo", "history": [1]})), Err(EngineError::NeedsConfirmation(_))));
    run(&mut s, "macro.save", json!({"folder": folder, "name": "Bravo", "history": [0, 1], "confirm": true}));
    assert_eq!(names(&mut s), ["Charlie", "Delta", "Bravo"], "a replaced command keeps its place");
    assert!(matches!(s.execute("macro.delete", &json!({"folder": folder, "name": "Charlie"})), Err(EngineError::NeedsConfirmation(_))));
    run(&mut s, "macro.delete", json!({"folder": folder, "name": "Charlie", "confirm": true}));
    assert_eq!(names(&mut s), ["Delta", "Bravo"]);
    // A shared file comes in under its own name, checked.
    let shared = dir.join("shared");
    std::fs::create_dir_all(&shared).unwrap();
    std::fs::write(shared.join("Spin.json"), r#"[{"cmd": "modify.rotate_cw"}, {"command": "modify.rotate_cw"}]"#).unwrap();
    let v = run(&mut s, "macro.import", json!({"folder": folder, "path": shared.join("Spin.json").display().to_string()}));
    assert_eq!(v["steps"], 2);
    assert_eq!(names(&mut s), ["Delta", "Bravo", "Spin"]);
    // ...and runs straight from where it is (Run Command…).
    let before = s.visible_elements()[0].transform;
    run(&mut s, "macro.run", json!({"path": shared.join("Spin.json").display().to_string()}));
    assert_eq!(s.undo_label(), Some("Spin"));
    assert_ne!(s.visible_elements()[0].transform, before);
    // Files that aren't saved commands are listed as problems, never as commands.
    std::fs::write(dir.join("junk.json"), "{nope").unwrap();
    let v = run(&mut s, "macro.list", json!({"folder": folder}));
    assert_eq!(v["problems"][0]["file"], "junk.json");
    assert_eq!(names(&mut s).len(), 3);
}

#[test]
fn hostile_saved_command_files_never_panic() {
    let dir = temp_dir("hostile");
    let deep = format!("{}{}", "[".repeat(5000), "]".repeat(5000));
    let many = serde_json::to_string(&vec![json!({"cmd": "modify.rotate_cw"}); cmds_macro::MAX_STEPS + 1]).unwrap();
    let mut bodies: Vec<Vec<u8>> = vec![
        Vec::new(),
        b"\xef\xbb\xbf".to_vec(),
        b"\xff\xfe\x00\x01garbage".to_vec(),
        deep.into_bytes(),
        many.into_bytes(),
        "x".repeat((cmds_macro::MAX_FILE_BYTES + 10) as usize).into_bytes(),
    ];
    for text in [
        "null",
        "7",
        "\"steps\"",
        "{}",
        r#"{"format": "something-else", "steps": []}"#,
        r#"{"format": "anicraft-command", "version": 99, "steps": []}"#,
        r#"{"format": "anicraft-command", "version": -1, "steps": []}"#,
        r#"{"steps": "no"}"#,
        r#"{"steps": [7, null, "x"]}"#,
        r#"{"steps": [{"cmd": 7}]}"#,
        r#"{"steps": [{"cmd": "element.translate", "params": [1, 2]}]}"#,
        r#"{"steps": [{"cmd": "element.translate", "params": {"dx": 1e308, "dy": -1e308}}]}"#,
        r#"{"steps": [{"cmd": "element.translate", "params": {"dx": "a"}}]}"#,
        r#"{"steps": [{"cmd": "draw.rect", "params": {"x": 0, "y": 0, "w": 1e30, "h": -1}, "label": 5}]}"#,
        r#"{"steps": [{"cmd": "layer.delete"}, {"cmd": "layer.delete"}, {"cmd": "layer.delete"}]}"#,
        r#"{"steps": [{"cmd": "edit.undo"}]}"#,
        r#"{"steps": [{"cmd": "file.export_png", "params": {"path": "/tmp/x.png"}}]}"#,
        r#"{"steps": [{"cmd": "macro.run", "params": {"path": "self.json"}}]}"#,
        r#"{"steps": [{"cmd": "plugin.run", "params": {"id": "x"}}]}"#,
    ] {
        bodies.push(text.as_bytes().to_vec());
    }
    bodies.push(json!({"steps": [{"cmd": "modify.rotate_cw", "label": "\u{0}\u{202e} evil"}]}).to_string().into_bytes());
    // Truncations of a good file at every byte, and a few flipped bytes.
    let good = serde_json::to_vec(&Macro { name: String::new(), steps: vec![cmds_macro::Step { cmd: "element.translate".into(), params: json!({"dx": 3, "dy": 4}), label: "Move".into() }] }.to_json()).unwrap();
    for i in 0..good.len() {
        bodies.push(good[..i].to_vec());
        let mut flipped = good.clone();
        flipped[i] ^= 0x5a;
        bodies.push(flipped);
    }
    let mut s = with_rect(10.0);
    for (i, body) in bodies.iter().enumerate() {
        let path = dir.join(format!("h{i}.json"));
        std::fs::write(&path, body).unwrap();
        let (doc, steps) = (s.doc.clone(), s.history_len());
        match s.execute("macro.run", &json!({"path": path.display().to_string()})) {
            Ok(_) => assert!(s.history_len() <= steps + 1),
            Err(_) => assert!(s.doc == doc && s.history_len() == steps, "a refused file changed the document"),
        }
        let _ = s.execute("macro.import", &json!({"folder": dir.join("in").display().to_string(), "path": path.display().to_string(), "confirm": true}));
    }
    // Listing a folder full of them never panics either.
    let v = run(&mut s, "macro.list", json!({"folder": dir.display().to_string()}));
    assert!(!v["problems"].as_array().unwrap().is_empty());
    // Hostile params, with every folder inside the temporary one.
    let folder = dir.display().to_string();
    for p in [
        Value::Null,
        json!({"folder": folder}),
        json!({"folder": folder, "name": "", "history": [u64::MAX]}),
        json!({"folder": folder, "name": "x", "history": "all"}),
        json!({"folder": folder, "name": "x", "history": []}),
        json!({"folder": folder, "name": "..", "history": [0]}),
        json!({"folder": folder, "name": "CON", "history": [0]}),
        json!({"folder": folder, "name": "a", "to": "", "index": u64::MAX}),
        json!({"folder": "", "name": "a"}),
        json!({"steps": {"steps": []}, "name": "\u{0}".repeat(500)}),
        json!({"steps": []}),
        json!({"path": folder}),
    ] {
        for c in commands().iter().filter(|c| c.id.starts_with("macro.")) {
            let _ = s.execute(c.id, &p);
        }
    }
}

#[test]
fn a_step_too_long_to_keep_says_so() {
    let mut s = with_rect(10.0);
    s.begin_gesture("Scribble");
    for i in 0..(history::MAX_RECORDED + 2) {
        // Alternating commands, so they don't add up into one.
        let id = if i % 2 == 0 { "element.translate" } else { "modify.rotate_cw" };
        run(&mut s, id, json!({"dx": 1, "dy": 0, "snap": 0}));
    }
    s.end_gesture();
    let v = run(&mut s, "macro.history", json!({}));
    let last = v["steps"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(last["keep"], false);
    assert!(last["why"].as_str().unwrap().contains("too many"), "{last}");
}

#[test]
fn steps_run_inside_a_motion_tween_key_it_as_on_their_own() {
    // Each step's edit becomes the tween's keys at the frame it ran at.
    let tweened = || {
        let mut s = Session::default();
        run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 10, "h": 10}));
        run(&mut s, "timeline.create_motion_tween", json!({}));
        s
    };
    let (mut a, mut b) = (tweened(), tweened());
    a.set_frame(5);
    run(&mut a, "edit.select_all", json!({}));
    run(&mut a, "element.translate", json!({"dx": 40, "dy": 0}));
    b.set_frame(5);
    run(&mut b, "edit.select_all", json!({}));
    run(&mut b, "macro.run", json!({"steps": [{"cmd": "element.translate", "params": {"dx": 40, "dy": 0}}]}));
    assert!(a.doc == b.doc, "a step in a saved command keys the tween as the command does alone");
}
