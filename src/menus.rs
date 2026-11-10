//! Menu bar generated from the engine's command registry, plus a few view-only items.

use crate::app::App;
use crate::platform::{self, Os};
use crate::theme::{Type, text};
use anicraft_engine::{CommandSpec, commands};

// Commands sits where Animate has it: after Modify, before Control.
const TOP_MENUS: [&str; 9] = ["File", "Edit", "View", "Insert", "Modify", "Commands", "Control", "Window", "Help"];

pub fn bar(app: &mut App, ui: &mut egui::Ui) {
    let cmds = commands();
    egui::MenuBar::new().ui(ui, |ui| {
        // macOS: leave room for the traffic lights over the unified title bar.
        ui.add_space(platform::menu_inset());
        if platform::OS == Os::Mac {
            // The application menu, as every Mac app has (About, Settings, Quit).
            ui.menu_button(text(Type::Body, "AniCraft").strong(), |ui| {
                ui.set_min_width(220.0);
                if ui_item(ui, "About AniCraft", "", None) {
                    app.show_about = true;
                }
                ui.separator();
                if ui_item(ui, "Settings…", "Ctrl+,", None) {
                    app.open_prefs();
                }
                ui.separator();
                if ui_item(ui, "Quit AniCraft", "Ctrl+Q", None) {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
        }
        // Windows and Linux: no name before the menus; the window title already shows it.
        for top in TOP_MENUS {
            ui.menu_button(top, |ui| {
                ui.set_min_width(230.0);
                menu_contents(app, ui, top, &cmds);
            });
        }
    });
}

fn enabled(app: &App, id: &str) -> bool {
    match id {
        "edit.undo" => app.s.can_undo(),
        "edit.redo" => app.s.can_redo(),
        "edit.delete" | "edit.duplicate" | "modify.convert_to_symbol" | "modify.break_apart" | "arrange.bring_to_front" | "arrange.send_to_back" => !app.s.selection.is_empty(),
        "layer.delete" => app.s.doc.timeline(app.s.scene).layers.len() > 1,
        "scene.delete" => app.s.doc.scenes.len() > 1,
        "scene.prev" | "scene.move_up" => app.s.scene > 0,
        "scene.next" | "scene.move_down" => app.s.scene + 1 < app.s.doc.scenes.len(),
        "timeline.clear_keyframe" => match app.frame_sel {
            Some(sel) => sel.has_keys(&app.s.doc.timeline(app.s.scene).layers),
            None => app.s.active_layer().is_some_and(|l| l.keyframe_starting_at(app.s.frame).is_some()),
        },
        "timeline.paste_frames" => !app.s.frame_clipboard.is_empty(),
        "tween.add_hint" => app.s.active_layer().and_then(|l| l.hint_tween_at(app.s.frame)).is_some(),
        "tween.clear_hints" => app.s.active_layer().and_then(|l| l.hint_tween_at(app.s.frame).and_then(|(i, _)| l.keyframes.get(i))).is_some_and(|k| !k.hints.is_empty()),
        "timeline.remove_frame" | "timeline.create_tween" | "timeline.create_shape_tween" => app.s.active_layer().is_some_and(|l| l.keyframe_at(app.s.frame).is_some()),
        _ => true,
    }
}

fn item(app: &mut App, ui: &mut egui::Ui, spec: &CommandSpec) {
    let label = match spec.id {
        "edit.undo" => app.s.undo_label().map_or_else(|| "Undo".to_string(), |l| format!("Undo {l}")),
        _ => spec.label.to_string(),
    };
    let button = egui::Button::new(label).shortcut_text(text(Type::Key, platform::shortcut_label(app.keymap.primary(spec.id).unwrap_or(""))));
    // What this build can't do stays in its usual place, dimmed, saying why (the web build).
    let missing = platform::unavailable(spec.id);
    let r = ui.add_enabled(enabled(app, spec.id) && missing.is_none(), button);
    let r = match missing {
        Some(why) => r.on_disabled_hover_text(why),
        None => r,
    };
    if r.clicked() {
        app.invoke(spec.id);
        ui.close();
    }
}

/// A registry command as a menu item (label and shortcut from the registry); used by context
/// menus so every surface shows the same names and keys.
pub fn cmd_item(app: &mut App, ui: &mut egui::Ui, id: &str) {
    if let Some(spec) = anicraft_engine::find_command(id) {
        item(app, ui, &spec);
    }
}

/// A check item for state that is not a registry command's own (context menus).
pub fn check_item(ui: &mut egui::Ui, label: &str, checked: bool, enabled: bool) -> bool {
    let label = if checked { format!("✔ {label}") } else { format!("   {label}") };
    let clicked = ui.add_enabled(enabled, egui::Button::new(label)).clicked();
    if clicked {
        ui.close();
    }
    clicked
}

fn ui_item(ui: &mut egui::Ui, label: &str, shortcut: &str, checked: Option<bool>) -> bool {
    let label_text = match checked {
        Some(true) => format!("✔ {label}"),
        Some(false) => format!("   {label}"),
        None => label.to_string(),
    };
    let clicked = ui.add(egui::Button::new(label_text).shortcut_text(text(Type::Key, platform::shortcut_label(shortcut)))).clicked();
    if clicked {
        ui.close();
    }
    clicked
}

fn menu_contents(app: &mut App, ui: &mut egui::Ui, top: &str, cmds: &[CommandSpec]) {
    if top == "Commands" {
        // Installed plug-ins first (they change at run time), then the registry's items.
        crate::plugins_ui::menu(app, ui, cmds, item);
        return;
    }
    // Direct items first, then one submenu per "Top/Sub" path, in registry order.
    for spec in cmds.iter().filter(|c| c.menu == top) {
        item(app, ui, spec);
    }
    let mut subs: Vec<&str> = Vec::new();
    for c in cmds {
        if let Some(sub) = c.menu.strip_prefix(top).and_then(|r| r.strip_prefix('/'))
            && !subs.contains(&sub)
        {
            subs.push(sub);
        }
    }
    for sub in subs {
        let path = format!("{top}/{sub}");
        ui.menu_button(sub, |ui| {
            ui.set_min_width(220.0);
            for spec in cmds.iter().filter(|c| c.menu == path) {
                item(app, ui, spec);
            }
        });
    }

    match top {
        // Windows / Linux conventions; on macOS these live in the AniCraft (application) menu.
        // A web page is closed with its tab.
        "File" if platform::OS != Os::Mac && !platform::WEB => {
            ui.separator();
            if ui_item(ui, "Exit", "Ctrl+Q", None) {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        "Edit" if platform::OS != Os::Mac => {
            ui.separator();
            if ui_item(ui, "Preferences…", "Ctrl+U", None) {
                app.open_prefs();
            }
        }
        "View" => {
            let canvas = app.stage_cache.canvas;
            let center = canvas.center();
            if ui_item(ui, "Zoom In", "Ctrl+=", None) {
                let z = app.view.zoom * 1.5;
                app.zoom_about(canvas, center, z);
            }
            if ui_item(ui, "Zoom Out", "Ctrl+-", None) {
                let z = app.view.zoom / 1.5;
                app.zoom_about(canvas, center, z);
            }
            if ui_item(ui, "Fit in Window", "Ctrl+0", None) {
                app.view.fit = true;
            }
            if ui_item(ui, "100%", "Ctrl+1", None) {
                app.zoom_about(canvas, center, 1.0);
            }
            ui.separator();
            if ui_item(ui, "Onion Skin", "", Some(app.onion)) {
                app.onion = !app.onion;
            }
            if ui_item(ui, "Preview Through Camera", "", Some(app.camera_preview)) {
                app.toggle_camera_preview();
            }
            ui.menu_button("Stage Rendering", |ui| {
                for (mode, name, tip) in crate::prefs::RenderMode::ALL {
                    if ui_item(ui, name, "", Some(app.prefs.render_mode == mode)) {
                        let mut p = app.prefs.clone();
                        p.render_mode = mode;
                        let ctx = ui.ctx().clone();
                        app.set_prefs(&ctx, p);
                    }
                    ui.label(text(Type::Caption, tip));
                }
                if crate::gpu_stage::AVAILABLE {
                    ui.separator();
                    for (choice, name, tip) in crate::prefs::StageRenderer::ALL {
                        if ui_item(ui, name, "", Some(app.prefs.stage_renderer == choice)) {
                            let mut p = app.prefs.clone();
                            p.stage_renderer = choice;
                            let ctx = ui.ctx().clone();
                            app.set_prefs(&ctx, p);
                        }
                        ui.label(text(Type::Caption, tip));
                    }
                }
            });
        }
        "Control" => {
            ui.separator();
            if ui_item(ui, if app.playing { "Stop" } else { "Play" }, "Enter", None) {
                app.toggle_play();
            }
            if ui_item(ui, "Loop Playback", "", Some(app.looping)) {
                app.looping = !app.looping;
            }
        }
        "Window" => {
            use crate::app::BottomTab;
            if ui_item(ui, "Timeline", "", Some(app.bottom == BottomTab::Timeline)) {
                app.bottom = BottomTab::Timeline;
            }
            if ui_item(ui, "Motion Editor", "", Some(app.bottom == BottomTab::MotionEditor)) {
                app.bottom = BottomTab::MotionEditor;
            }
            if ui_item(ui, "Actions", app.keymap.primary("view.actions").unwrap_or(""), Some(app.show_actions)) {
                app.show_actions = !app.show_actions;
            }
            ui.separator();
            if ui_item(ui, "Onion Skin", "", Some(app.onion)) {
                app.onion = !app.onion;
            }
            ui.label(text(Type::Caption, "Panels are docked: Tools, Stage, Timeline, Properties, Library"));
        }
        "Help" => {
            if ui_item(ui, "Command Palette…", "Ctrl+K", None) {
                app.palette = Some(crate::dialogs::Palette::default());
            }
            if ui_item(ui, "Keyboard Shortcuts", "F1", None) {
                app.show_shortcuts = true;
            }
            ui.separator();
            // (The web build has no folders to show.)
            if !platform::WEB && ui_item(ui, "Project Files & Docs…", "", None) {
                app.show_files = true;
            }
            if ui_item(ui, "Licenses & Notices", "", None) {
                app.show_licenses = true;
            }
            if ui_item(ui, "About AniCraft", "", None) {
                app.show_about = true;
            }
            ui.label(text(Type::Caption, format!("{} commands are scriptable via anicraft-cli", cmds.len())));
        }
        _ => {}
    }
}
