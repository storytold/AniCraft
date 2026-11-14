//! Saved commands in the desktop app (ADR 0097): picking steps in the History panel, Save As
//! Command, a saved command's shortcut running it, and a rename or delete taking its keys
//! along.

use crate::app::App;
use crate::keymap;
use anicraft_engine::Session;
use anicraft_engine::doc::sample;
use serde_json::json;

/// A test app: a screenshot path, so the saved commands live in a temporary folder.
fn app(ctx: &egui::Context) -> App {
    App::with_context(ctx, Session::new(sample::demo()), Some(std::path::PathBuf::from("unused.png")))
}

fn frame(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) {
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 940.0))), events, ..Default::default() };
    ctx.run_ui(raw, |ui| app.frame(ui)).textures_delta.clear();
}

#[test]
fn picked_steps_save_as_a_command_and_its_shortcut_runs_it() {
    let ctx = egui::Context::default();
    let mut app = app(&ctx);
    // (The only test here that writes to the folder.)
    if let Some(f) = crate::macros_ui::folder(&app) {
        let _ = std::fs::remove_dir_all(&f);
    }
    app.run("edit.select_all", json!({}));
    app.run("modify.rotate_cw", json!({}));
    app.run("element.set_color_effect", json!({"style": "alpha", "alpha": 50}));
    let n = app.s.history_len();
    // Ctrl+click the two steps; a plain pick of a third, then Ctrl+click takes it out again.
    crate::macros_ui::pick(&mut app, n - 2, true, false);
    crate::macros_ui::pick(&mut app, n - 1, true, false);
    assert_eq!(crate::macros_ui::picked(&mut app), [n - 2, n - 1]);
    crate::macros_ui::open_save(&mut app);
    let d = app.macros.save.as_mut().expect("the dialog");
    assert_eq!(d.steps.len(), 2);
    assert!(d.steps.iter().all(|s| s.refused.is_none()));
    d.name = "Spin and Fade".into();
    frame(&ctx, &mut app, vec![]);
    let folder = crate::macros_ui::folder(&app).expect("a folder").display().to_string();
    app.request("macro.save", json!({"folder": folder, "name": "Spin and Fade", "history": [n - 2, n - 1]}));
    assert!(app.macros.save.is_none(), "saved, the dialog closes");
    assert!(crate::macros_ui::picked(&mut app).is_empty());
    assert_eq!(app.macros.list.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Spin and Fade"]);

    // A key of its own, through the shortcut map.
    let id = keymap::macro_id("Spin and Fade");
    app.prefs.shortcuts = keymap::assign(&app.prefs.shortcuts, &id, None, "Ctrl+Alt+R", false).expect("a free key");
    app.refresh_keymap();
    assert!(app.shortcuts.iter().any(|(_, k, a)| *k == egui::Key::R && *a == id));
    let steps = app.s.history_len();
    let before = app.s.selected_elements().first().map(|(_, e)| e.transform);
    // (The dialog's name field gives the keyboard back once it is gone.)
    for _ in 0..3 {
        frame(&ctx, &mut app, vec![]);
    }
    let key = egui::Event::Key { key: egui::Key::R, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND | egui::Modifiers::ALT };
    frame(&ctx, &mut app, vec![key]);
    assert_eq!(app.s.history_len(), steps + 1, "the key ran it as one step");
    assert_eq!(app.s.undo_label(), Some("Spin and Fade"));
    assert_ne!(app.s.selected_elements().first().map(|(_, e)| e.transform), before);

    // A rename takes its key along; a delete (confirmed) drops it.
    app.request("macro.rename", json!({"folder": folder, "name": "Spin and Fade", "to": "Twirl"}));
    assert_eq!(app.keymap.keys(&keymap::macro_id("Twirl")), ["Ctrl+Alt+R"]);
    assert!(!app.prefs.shortcuts.contains_key(&id));
    app.request("macro.delete", json!({"folder": folder, "name": "Twirl"}));
    assert!(app.confirm.is_some(), "deleting a file asks first");
    app.confirm = None;
    app.perform("macro.delete", json!({"folder": folder, "name": "Twirl", "confirm": true}));
    assert!(app.macros.list.is_empty());
    assert!(app.prefs.shortcuts.keys().all(|k| keymap::macro_name(k).is_none()));

    // The windows and the menu draw, and their buttons work (same folder: this test owns it).
    app.open_panel(Some("managecommands"));
    assert!(app.macros.manage.open);
    assert_eq!(app.macros.list.len(), 3, "the sample commands");
    for _ in 0..3 {
        frame(&ctx, &mut app, vec![]);
    }
    let cmds = anicraft_engine::commands();
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| crate::menus::menu_contents(&mut app, ui, "Commands", &cmds));
    out.textures_delta.clear();
    // Reorder and rename through the same requests the buttons make.
    let first = app.macros.list[0].name.clone();
    app.request("macro.move", json!({"folder": folder, "name": first, "index": 2}));
    assert_eq!(app.macros.list[2].name, first);
    app.macros.manage.renaming = Some((first.clone(), "Renamed".into()));
    frame(&ctx, &mut app, vec![]);
    crate::macros_ui::run_saved(&mut app, "Quarter Turn");
    assert_eq!(app.s.undo_label(), Some("Quarter Turn"));
    // A file shared from elsewhere comes in through Add from File (the folder is filled in).
    let shared = std::env::temp_dir().join(format!("anicraft-shared-{}.json", std::process::id()));
    std::fs::write(&shared, r#"[{"cmd": "modify.rotate_ccw"}]"#).unwrap();
    app.request("macro.import", json!({"path": shared.display().to_string()}));
    assert!(app.macros.list.iter().any(|c| c.name.starts_with("anicraft-shared")));
    let _ = std::fs::remove_file(&shared);
    app.macros.manage.open = false;
    // History with steps picked, and Save As Command over it.
    app.open_panel(Some("savecommand"));
    assert!(app.macros.save.is_some());
    for _ in 0..3 {
        frame(&ctx, &mut app, vec![]);
    }
    crate::macros_ui::clear_picks(&mut app);
    crate::macros_ui::open_save(&mut app);
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn a_step_that_cant_be_kept_is_named_in_the_dialog() {
    let ctx = egui::Context::default();
    let mut app = app(&ctx);
    app.run("edit.select_all", json!({}));
    app.run("edit.copy", json!({}));
    app.run("edit.paste", json!({}));
    let n = app.s.history_len();
    crate::macros_ui::pick(&mut app, n - 1, false, false);
    crate::macros_ui::open_save(&mut app);
    let d = app.macros.save.as_ref().expect("the dialog");
    assert!(d.steps[0].refused.as_deref().is_some_and(|w| w.contains("clipboard")), "{:?}", d.steps);
    // The dialog draws, and Save stays off.
    frame(&ctx, &mut app, vec![]);
    assert!(app.macros.save.is_some());
}

#[test]
fn saved_commands_take_shortcuts_like_any_action() {
    keymap::set_macros(["Spin"]);
    let none = std::collections::BTreeMap::new();
    let o = keymap::assign(&none, "macro:Spin", None, "Ctrl+Alt+R", false).expect("free");
    let m = keymap::Keymap::new(&o);
    assert_eq!(m.owner("Ctrl+Alt+R").map(|a| a.id), Some("macro:Spin"));
    assert!(matches!(keymap::assign(&o, "macro:Spin", None, "Ctrl+Z", false), Err(keymap::Refusal::Taken { id: "edit.undo", .. })));
    // Kept while the folder hasn't been read yet, bound once it is.
    keymap::set_macros([]);
    let kept = keymap::sanitize(o.clone());
    assert!(kept.contains_key("macro:Spin"));
    assert!(keymap::Keymap::new(&kept).keys("macro:Spin").is_empty());
    // A clash from a hand-edited file stays with the action listed first.
    keymap::set_macros(["Spin"]);
    let bad = std::collections::BTreeMap::from([("macro:Spin".to_string(), vec!["Ctrl+Z".to_string()])]);
    let m = keymap::Keymap::new(&bad);
    assert_eq!(m.owner("Ctrl+Z").map(|a| a.id), Some("edit.undo"));
    assert!(m.keys("macro:Spin").is_empty());
}
