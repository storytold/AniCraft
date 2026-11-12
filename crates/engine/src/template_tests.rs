//! Templates (ADR 0065): every built-in template opens as a new untitled document and saves,
//! and Save as Template keeps the document with a picture in the user's folder, round trip.

use super::*;
use serde_json::json;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    match s.execute(id, &p) {
        Ok(v) => v,
        Err(e) => panic!("{id} {p}: {e}"),
    }
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("anicraft-templates-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `panic_hunt_every_command` leaves `file.*` out (they touch the disk): the template commands
/// and timeline drops get hostile params here, with every folder inside a temporary one.
#[test]
fn template_and_timeline_drop_commands_never_panic_on_hostile_params() {
    let dir = temp_dir("fuzz");
    let folder = dir.display().to_string();
    let junk = [
        Value::Null,
        json!([]),
        json!("x"),
        json!({}),
        json!({"folder": folder}),
        json!({"folder": folder, "name": 7}),
        json!({"folder": folder, "name": "a", "category": ["x"]}),
        json!({"folder": folder, "name": "\u{0}", "category": "\u{202e}"}),
        json!({"folder": folder, "name": "é".repeat(64), "category": "ü".repeat(65)}),
        json!({"folder": 3, "name": "a"}),
        json!({"folder": folder, "name": "ok", "confirm": "yes"}),
        json!({"template": 3}),
        json!({"template": ""}),
        json!({"path": ""}),
        json!({"path": dir.join("nope.anicraft").display().to_string()}),
        json!({"path": folder, "confirm": true}),
        json!({"paths": ["a.png"], "to": "timeline", "layer": -1, "frame": 1e30}),
        json!({"paths": ["a.wav"], "to": "timeline", "layer": "x", "frame": "y"}),
        json!({"paths": ["a.svg"], "to": "timeline", "layer": 1, "frame": 0, "x": f64::NAN}),
    ];
    for id in ["file.templates", "file.new_from_template", "file.save_as_template", "file.delete_template", "file.drop"] {
        for p in &junk {
            let mut s = Session::new(anicraft_doc::sample::demo());
            let _ = s.execute(id, p);
            let mut confirmed = p.clone();
            if let Some(o) = confirmed.as_object_mut() {
                o.insert("confirm".into(), json!(true));
            }
            let _ = s.execute(id, &confirmed);
        }
    }
}

#[test]
fn every_built_in_template_opens_untitled_and_survives_a_save() {
    let dir = temp_dir("builtin");
    let list = run(&mut Session::default(), "file.templates", json!({}));
    let builtin = list["builtin"].as_array().unwrap();
    assert_eq!(builtin.len(), anicraft_doc::templates::TEMPLATES.len());
    assert!(list["user"].as_array().unwrap().is_empty());
    for t in builtin {
        let id = t["id"].as_str().unwrap();
        let mut s = Session::new(anicraft_doc::sample::demo());
        s.path = Some(dir.join("was.anicraft"));
        let v = run(&mut s, "file.new_from_template", json!({"template": id}));
        assert_eq!(v["template"], t["name"]);
        assert!(s.path.is_none() && !s.is_dirty(), "{id}: a new, untitled document");
        assert_eq!((u64::from(s.doc.stage.width), u64::from(s.doc.stage.height)), (t["width"].as_u64().unwrap(), t["height"].as_u64().unwrap()));
        // Drawing works straight away on a drawing layer, and it saves and opens again.
        let active = s.active_layer().unwrap();
        assert!(active.kind == anicraft_doc::LayerKind::Normal && !active.locked, "{id}: the active layer takes drawing");
        run(&mut s, "draw.rect", json!({"x": 10, "y": 10, "w": 20, "h": 20}));
        let path = dir.join(format!("{id}.anicraft"));
        run(&mut s, "file.save_as", json!({"path": path.display().to_string()}));
        let saved = s.doc.clone();
        run(&mut s, "file.open", json!({"path": path.display().to_string()}));
        assert!(s.doc == saved, "{id}: round trip");
        // Thumbnails render (the shell draws these on the start screen).
        let png = crate::cmds_template::picture_png(&s.doc).unwrap();
        assert!(png.len() > 100);
    }
    let mut s = Session::default();
    assert!(s.execute("file.new_from_template", &json!({"template": "nope"})).is_err());
    assert!(s.execute("file.new_from_template", &json!({})).is_err());
    assert!(s.execute("file.new_from_template", &json!({"template": "storyboard", "path": "x.anicraft"})).is_err());
    // Over unsaved work it asks first, like New.
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 5, "h": 5}));
    assert!(s.confirmation("file.new_from_template", &json!({"template": "storyboard"})).is_some_and(|c| c.irreversible));
    assert!(s.execute("file.new_from_template", &json!({"template": "storyboard"})).is_err());
    run(&mut s, "file.new_from_template", json!({"template": "storyboard", "confirm": true}));
    assert_eq!(s.doc.stage.width, 1920);
}

#[test]
fn save_as_template_keeps_the_document_and_a_picture_and_opens_as_new() {
    let dir = temp_dir("user");
    let folder = dir.display().to_string();
    let mut s = Session::new(anicraft_doc::sample::demo());
    let mine = dir.join("walk.anicraft");
    run(&mut s, "file.save_as", json!({"path": mine.display().to_string()}));
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 5, "h": 5}));
    let doc = s.doc.clone();
    let v = run(&mut s, "file.save_as_template", json!({"folder": folder, "name": " Walk cycle ", "category": "Characters"}));
    let path = dir.join("Characters").join("Walk cycle.anicraft");
    assert_eq!(v["path"].as_str(), Some(path.display().to_string().as_str()));
    assert!(path.is_file() && path.with_extension("png").is_file(), "{v}");
    assert_eq!(s.path.as_deref(), Some(mine.as_path()), "the open document keeps its own file");
    assert!(s.is_dirty(), "and its unsaved changes");
    // Listed, with its picture; a default category when none is given.
    run(&mut s, "file.save_as_template", json!({"folder": folder, "name": "Blank"}));
    let list = run(&mut s, "file.templates", json!({"folder": folder}));
    let user: Vec<(String, String, bool)> = list["user"].as_array().unwrap().iter().map(|t| (t["category"].as_str().unwrap().to_string(), t["name"].as_str().unwrap().to_string(), t["thumbnail"].is_string())).collect();
    assert_eq!(user, [("Characters".to_string(), "Walk cycle".to_string(), true), (crate::cmds_template::DEFAULT_CATEGORY.to_string(), "Blank".to_string(), true)]);
    // Opened from the template: the same document, new and untitled.
    let mut other = Session::default();
    run(&mut other, "file.new_from_template", json!({"path": path.display().to_string()}));
    assert!(other.doc == doc && other.path.is_none() && !other.is_dirty());
    assert!(other.recent.is_empty(), "a template is not a recent file");
    // The same name again replaces it only when confirmed.
    let again = json!({"folder": folder, "name": "Walk cycle", "category": "Characters"});
    assert!(s.confirmation("file.save_as_template", &again).is_some_and(|c| c.irreversible));
    assert!(s.execute("file.save_as_template", &again).is_err());
    let mut confirmed = again.clone();
    confirmed["confirm"] = json!(true);
    run(&mut s, "file.save_as_template", confirmed);
    // Names that can't be file names are refused before anything is written.
    for (name, category) in [("", "x"), ("  ", "x"), ("a/b", "x"), ("a", "..\\up"), ("CON", "x"), ("ok", "lpt1"), (".hidden", "x"), ("x", "dot."), ("tab\there", "x")] {
        let p = json!({"folder": folder, "name": name, "category": category});
        assert!(s.execute("file.save_as_template", &p).is_err(), "{p}");
    }
    let long = "x".repeat(crate::cmds_template::MAX_NAME_CHARS + 1);
    assert!(s.execute("file.save_as_template", &json!({"folder": folder, "name": long})).is_err());
    // Deleting always confirms, and takes the picture and the emptied category with it.
    let del = json!({"folder": folder, "name": "Walk cycle", "category": "Characters"});
    assert!(s.execute("file.delete_template", &del).is_err());
    assert!(path.is_file());
    let mut del = del;
    del["confirm"] = json!(true);
    run(&mut s, "file.delete_template", del.clone());
    assert!(!path.exists() && !path.with_extension("png").exists() && !dir.join("Characters").exists());
    assert!(s.execute("file.delete_template", &del).is_err(), "gone already");
    assert_eq!(run(&mut s, "file.templates", json!({"folder": folder}))["user"].as_array().map(Vec::len), Some(1));
    // A folder that isn't there lists nothing, and a non-template file doesn't open.
    assert_eq!(run(&mut s, "file.templates", json!({"folder": dir.join("nope").display().to_string()}))["user"].as_array().map(Vec::len), Some(0));
    assert!(s.execute("file.new_from_template", &json!({"path": dir.join("x.png").display().to_string(), "confirm": true})).is_err());
}
