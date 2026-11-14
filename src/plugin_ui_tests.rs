//! Exporter and ease plug-ins in the shell (ADR 0088): the ease menu's Plug-in eases, the
//! Plug-ins window's buttons for them, and the screenshot states that show them.

use crate::app::App;
use crate::plugin_fixture_tests::{EASES, EXPORTER, eases, exporter, install, temp};
use anicraft_engine::Session;
use anicraft_engine::doc::{Easing, sample};
use eframe::App as _;
use serde_json::json;

fn app(ctx: &egui::Context) -> App {
    App::with_context(ctx, Session::new(sample::demo()), Some(std::path::PathBuf::from("unused.png")))
}

fn input(events: Vec<egui::Event>) -> egui::RawInput {
    egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 940.0))), events, ..Default::default() }
}

/// The ease menu in a panel, opened the way a screenshot opens it (a click the input hook
/// injects), until it has drawn its contents; what it picked, if anything.
fn ease_menu(ctx: &egui::Context, app: &mut App, ease: &Easing, extra: Vec<egui::Event>) -> Option<serde_json::Value> {
    app.shot_ease_menu.set(Some("t_ease"));
    let mut picked = None;
    for i in 0..8 {
        let mut raw = input(if i == 7 { extra.clone() } else { vec![] });
        app.raw_input_hook(ctx, &mut raw);
        let mut out = ctx.run_ui(raw, |ui| {
            if let Some(crate::ease_editor::Picked::Ease(v)) = crate::ease_editor::picker(app, ui, "t_ease", ease, 120.0, "Easing") {
                picked = Some(v);
            }
        });
        out.textures_delta.clear();
    }
    picked
}

#[test]
fn the_ease_menu_lists_plugin_eases_and_names_the_tweens() {
    let ctx = egui::Context::default();
    let mut app = app(&ctx);
    let dir = temp("ease-menu");
    install(&mut app, &dir, "steps.wasm", &eases());
    let e = anicraft_engine::params::plugin_ease(&app.s, EASES, "steps4").expect("the ease");
    assert_eq!(crate::ease_editor::label(&app.s.doc, &e), "Stepped 4");
    // The menu opens and draws the plug-in's eases under its name (nothing picked yet).
    assert_eq!(ease_menu(&ctx, &mut app, &e, vec![]), None);
    assert!(egui::Popup::is_any_open(&ctx), "the menu is open");
    // Without plug-ins there's no section, and the menu still draws.
    app.s.execute("plugin.remove", &json!({"id": EASES})).expect("removes");
    assert_eq!(ease_menu(&ctx, &mut app, &Easing::Linear, vec![]), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_plugins_window_offers_export_and_counts_eases() {
    let ctx = egui::Context::default();
    let mut app = app(&ctx);
    let dir = temp("plugins-window");
    install(&mut app, &dir, "dump.wasm", &exporter(false));
    install(&mut app, &dir, "steps.wasm", &eases());
    app.show_plugins = true;
    for _ in 0..3 {
        let mut out = ctx.run_ui(input(vec![]), |ui| crate::plugins_ui::show(&mut app, ui.ctx()));
        out.textures_delta.clear();
    }
    // Neither runs as a command, and both say where to find them.
    for id in [EXPORTER, EASES] {
        crate::plugins_ui::run_or_ask(&mut app, id);
        assert!(app.status.as_ref().is_some_and(|s| s.error && (s.text.contains("File › Export") || s.text.contains("ease menu"))), "{id}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_screenshot_states_show_the_plugins() {
    let ctx = egui::Context::default();
    let mut app = app(&ctx);
    let dir = temp("shot-states");
    install(&mut app, &dir, "dump.wasm", &exporter(false));
    install(&mut app, &dir, "steps.wasm", &eases());
    app.open_panel(Some("pluginexportwindow"));
    assert_eq!(app.plugin_export.as_ref().map(|e| e.id.as_str()), Some(EXPORTER));
    app.plugin_export = None;
    app.open_panel(Some("pluginexport"));
    assert!(app.key_script.len() > 10, "F10, down to Export, Enter");
    app.open_panel(Some("plugineases"));
    assert_eq!(app.shot_ease_menu.get(), Some("tl_ease"));
    let ball = app.s.active_layer().and_then(|l| l.keyframes.first()).and_then(|k| k.tween.as_ref()).map(|t| t.easing.label());
    assert_eq!(ball.as_deref(), Some("Stepped 4"), "the first plug-in ease, on the ball's tween");
    // The click the hook injects: press, release, a frame, then a scroll; then nothing.
    app.shot_click.set(Some(egui::pos2(100.0, 100.0)));
    let kinds: Vec<usize> = (0..5)
        .map(|_| {
            let mut raw = input(vec![]);
            app.raw_input_hook(&ctx, &mut raw);
            raw.events.len()
        })
        .collect();
    assert_eq!(kinds, [2, 2, 0, 2, 0]);
    let _ = std::fs::remove_dir_all(&dir);
}
