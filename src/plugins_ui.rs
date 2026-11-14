//! Plug-ins in the desktop app (ADR 0017): the Commands menu, the dialog generated from a
//! plug-in's parameters (with a live preview on the stage), and the Plug-ins window.
//!
//! The dialog previews inside one gesture: every change puts the document back where the
//! dialog opened and runs the plug-in again, so OK records exactly one undo step named after
//! the plug-in, and Cancel leaves nothing behind.

use crate::a11y::{self, Named};
use crate::app::App;
use crate::platform;
use crate::theme::{self, FORM_LABEL_DOCK, T, TARGET, Type, form_row, text};
use anicraft_engine::plugins::{ParamKind, Plugin, SelectionNeed};
use anicraft_engine::{CommandSpec, Session};
use egui::Key;
use serde_json::{Map, Value, json};
use std::sync::Arc;

/// An open plug-in dialog.
pub struct Dialog {
    pub id: String,
    pub values: Map<String, Value>,
    pub preview: bool,
    /// The values the stage shows right now (`None`: the document is as it was).
    applied: Option<Map<String, Value>>,
    error: Option<String>,
}

/// Loads the plug-ins at start-up: `ANICRAFT_PLUGINS` when set, otherwise the plug-ins folder
/// in the settings folder (screenshot runs use only `ANICRAFT_PLUGINS`, to stay repeatable).
pub fn load_at_start(app: &mut App) {
    let env = std::env::var("ANICRAFT_PLUGINS").ok().filter(|v| !v.trim().is_empty()).map(std::path::PathBuf::from);
    let folder = env.or_else(|| if app.shot.is_some() { None } else { platform::plugins_dir() });
    let Some(folder) = folder else { return };
    match app.s.execute("plugin.reload", &json!({"folder": folder.display().to_string()})) {
        Ok(v) => {
            let failed: Vec<String> = v["failed"].as_array().into_iter().flatten().map(|f| format!("{} ({})", f["file"].as_str().unwrap_or(""), f["error"].as_str().unwrap_or(""))).collect();
            if !failed.is_empty() {
                app.status_err(tr!("Some plug-ins could not be loaded: {list}", list = failed.join("; ")));
            }
        }
        Err(e) => app.status_err(tr!("Plug-ins: {error}", error = e)),
    }
}

fn plugin(app: &App, id: &str) -> Option<Arc<Plugin>> {
    app.s.plugins.get(id)
}

/// Why `p` can't run now (it works on the selection and nothing is selected).
fn blocked(s: &Session, p: &Plugin) -> Option<&'static str> {
    match p.manifest().kind {
        anicraft_engine::plugins::Kind::FrameScript => return Some(tr!("A frame script: it runs while the movie plays. Pick it for a keyframe in the Actions panel (F9)")),
        anicraft_engine::plugins::Kind::Exporter => return Some(tr!("An export format: find it in File › Export")),
        anicraft_engine::plugins::Kind::Ease => return Some(tr!("Eases: pick them in a tween's ease menu")),
        anicraft_engine::plugins::Kind::Command => {}
    }
    (p.manifest().selection == SelectionNeed::Required && s.selection.is_empty()).then(|| tr!("Select some artwork first"))
}

/// Plug-ins that run as commands (frame scripts run from keyframes instead).
pub fn is_command(p: &Plugin) -> bool {
    p.manifest().kind == anicraft_engine::plugins::Kind::Command
}

/// The menu label: an ellipsis when it asks for settings first.
pub fn label(p: &Plugin) -> String {
    let m = p.manifest();
    if m.params.is_empty() { m.name.clone() } else { format!("{}…", m.name) }
}

/// Runs a plug-in from a menu or the palette: straight away when it has no settings,
/// otherwise through its dialog.
pub fn run_or_ask(app: &mut App, id: &str) {
    let Some(p) = plugin(app, id) else {
        app.status_err(tr!("The plug-in “{id}” is no longer installed", id = id));
        return;
    };
    if let Some(why) = blocked(&app.s, &p) {
        app.status_err(tr!("{plugin}: {reason}", plugin = p.manifest().name, reason = why));
        return;
    }
    if p.manifest().params.is_empty() {
        app.request("plugin.run", json!({"id": id}));
        return;
    }
    open(app, id);
}

/// Opens the dialog for `id` and shows the preview with the defaults.
pub fn open(app: &mut App, id: &str) {
    let Some(p) = plugin(app, id) else { return };
    app.playing = false;
    app.s.begin_gesture(&p.manifest().name);
    app.plugin_dialog = Some(Dialog { id: id.to_string(), values: p.manifest().defaults(), preview: true, applied: None, error: None });
    preview(app);
}

/// Shows the dialog's current values on the stage (or the untouched document).
fn preview(app: &mut App) {
    let Some(d) = &app.plugin_dialog else { return };
    let (id, values, on) = (d.id.clone(), d.values.clone(), d.preview);
    app.s.revert_gesture();
    let result = if on { app.s.execute("plugin.run", &json!({"id": id, "params": values})).map(|_| ()) } else { Ok(()) };
    if let Some(d) = &mut app.plugin_dialog {
        match result {
            Ok(()) => {
                d.applied = on.then_some(values);
                d.error = None;
            }
            Err(e) => {
                d.applied = None;
                d.error = Some(e.to_string());
            }
        }
    }
}

/// The Commands menu: the installed plug-ins, then the plug-in commands from the registry.
pub fn menu(app: &mut App, ui: &mut egui::Ui, cmds: &[CommandSpec], item: impl Fn(&mut App, &mut egui::Ui, &CommandSpec)) {
    ui.menu_button(tr!("Plug-ins"), |ui| {
        ui.set_min_width(220.0);
        let list: Vec<Arc<Plugin>> = app.s.plugins.list().filter(|p| is_command(p)).cloned().collect();
        if list.is_empty() {
            ui.label(text(Type::Caption, tr!("No plug-ins installed yet")));
        }
        for p in list {
            let why = blocked(&app.s, &p);
            let r = ui.add_enabled(why.is_none(), egui::Button::new(label(&p)));
            let r = if p.manifest().description.is_empty() { r } else { r.on_hover_text(&p.manifest().description) };
            let r = match why {
                Some(w) => r.on_disabled_hover_text(w),
                None => r,
            };
            if r.clicked() {
                ui.close();
                run_or_ask(app, p.id());
            }
        }
    });
    ui.separator();
    for spec in cmds.iter().filter(|c| c.menu == "Commands") {
        item(app, ui, spec);
    }
}

fn hex_rgb(s: &str) -> [u8; 3] {
    let h = s.trim_start_matches('#');
    let byte = |i: usize| h.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok()).unwrap_or(0);
    [byte(0), byte(2), byte(4)]
}

/// One parameter's control; true when the value changed.
/// Named for screen readers with the parameter's label.
pub(crate) fn control(ui: &mut egui::Ui, name: &str, kind: &ParamKind, v: &mut Value) -> bool {
    match kind {
        ParamKind::Number { min, max, .. } => {
            let mut f = v.as_f64().unwrap_or(*min);
            let changed = a11y::slider(ui, &mut f, *min..=*max, name, a11y::SliderLook { max_decimals: Some(2), ..Default::default() }).changed();
            *v = json!(f);
            changed
        }
        ParamKind::Int { min, max, .. } => {
            let mut n = v.as_i64().unwrap_or(*min);
            let changed = a11y::slider(ui, &mut n, *min..=*max, name, a11y::SliderLook::default()).changed();
            *v = json!(n);
            changed
        }
        ParamKind::Bool { .. } => {
            let mut b = v.as_bool().unwrap_or(false);
            let changed = theme::checkbox(ui, &mut b, "").named(name).changed();
            *v = json!(b);
            changed
        }
        ParamKind::Choice { options, .. } => {
            let mut cur = v.as_str().unwrap_or("").to_string();
            let mut changed = false;
            egui::ComboBox::from_id_salt(ui.next_auto_id()).selected_text(text(Type::Body, cur.clone())).show_ui(ui, |ui| {
                for o in options {
                    changed |= ui.selectable_value(&mut cur, o.clone(), text(Type::Body, o.clone())).changed();
                }
            })
            .response
            .set_name(name);
            *v = json!(cur);
            changed
        }
        ParamKind::Color { .. } => {
            let mut c = hex_rgb(v.as_str().unwrap_or("#000000"));
            let changed = crate::colour::button(ui, &mut c).named(name).changed();
            let h = format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]);
            ui.label(text(Type::Data, h.clone()));
            *v = json!(h);
            changed
        }
    }
}

fn dialog(app: &mut App, ctx: &egui::Context) {
    let Some(d) = &mut app.plugin_dialog else { return };
    let Some(p) = app.s.plugins.get(&d.id) else {
        app.s.revert_gesture();
        app.s.end_gesture();
        app.plugin_dialog = None;
        return;
    };
    let m = p.manifest();
    let mut changed = false;
    let (mut ok, mut cancel) = (false, false);
    let id = egui::Id::new("plugin_dialog");
    // Beside the stage, with no dimming, so the preview stays in view.
    let area = egui::Modal::default_area(id).anchor(egui::Align2::RIGHT_TOP, [-10.0, 96.0]);
    egui::Modal::new(id).area(area).backdrop_color(egui::Color32::TRANSPARENT).show(ctx, |ui| {
        ui.set_width(340.0);
        ui.label(text(Type::Title, m.name.clone()));
        let by = [m.author.as_str(), m.version.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" · ");
        if !by.is_empty() {
            ui.label(text(Type::Caption, tr!("Plug-in · {by}", by = by)));
        }
        if !m.description.is_empty() {
            ui.add_space(2.0);
            ui.label(text(Type::Label, m.description.clone()));
        }
        ui.add_space(8.0);
        for (key, spec) in &m.params {
            let v = d.values.entry(key.clone()).or_insert(Value::Null);
            form_row(ui, FORM_LABEL_DOCK + 40.0, &spec.label, |ui| changed |= control(ui, &spec.label, &spec.kind, v));
        }
        ui.add_space(6.0);
        if theme::checkbox(ui, &mut d.preview, tr!("Preview on the stage")).changed() {
            changed = true;
        }
        if let Some(e) = &d.error {
            ui.add_space(4.0);
            ui.label(text(Type::Caption, e.clone()).color(T.error));
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.add(egui::Button::new(tr!("Cancel")).min_size(egui::vec2(96.0, TARGET + 4.0))).clicked() {
                cancel = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let run = egui::Button::new(text(Type::Body, tr!("OK")).strong()).fill(T.accent_dim).min_size(egui::vec2(96.0, TARGET + 4.0));
                if ui.add(run).clicked() {
                    ok = true;
                }
            });
        });
        ui.add_space(2.0);
        ui.label(text(Type::Caption, tr!("Enter applies · Esc cancels · one undo step")));
    });
    let (esc, enter) = ctx.input_mut(|i| (i.consume_key(egui::Modifiers::NONE, Key::Escape), i.consume_key(egui::Modifiers::NONE, Key::Enter)));
    cancel |= esc;
    ok |= enter && !cancel;
    if changed {
        preview(app);
    }
    if cancel {
        app.s.revert_gesture();
        app.s.end_gesture();
        app.plugin_dialog = None;
        app.status_ok(tr!("Cancelled"));
    } else if ok {
        finish(app);
    }
}

/// OK: makes sure the document shows the final values, then closes the step.
fn finish(app: &mut App) {
    let Some(d) = &mut app.plugin_dialog else { return };
    if d.applied.as_ref() != Some(&d.values) {
        d.preview = true;
        preview(app);
    }
    let Some(d) = &app.plugin_dialog else { return };
    if d.error.is_some() {
        return;
    }
    let name = app.s.plugins.get(&d.id).map_or_else(String::new, |p| p.manifest().name.clone());
    app.s.end_gesture();
    app.plugin_dialog = None;
    app.status_ok(tr!("{name} — {undo} to undo", name = name, undo = app.undo_key()));
}

/// Commands › Manage Plug-ins: what's installed, where it came from, and the folder.
fn manager(app: &mut App, ctx: &egui::Context) {
    let mut open = app.show_plugins;
    let mut action: Option<(&'static str, String)> = None;
    let title = tr!("Plug-ins");
    let shown = egui::Window::new(title).id(egui::Id::new("plugins_window")).open(&mut open).collapsible(false).resizable(false).default_width(560.0).show(ctx, |ui| {
        let list: Vec<Arc<Plugin>> = app.s.plugins.list().cloned().collect();
        if list.is_empty() {
            ui.label(text(Type::Label, tr!("No plug-ins installed yet. Plug-ins are WebAssembly (.wasm) files that add commands (generators, selection effects), export formats and eases.")));
        }
        for p in &list {
            let m = p.manifest();
            ui.horizontal(|ui| {
                ui.label(text(Type::Body, m.name.clone()).strong());
                if !m.version.is_empty() {
                    ui.label(text(Type::Data, m.version.clone()));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(tr!("Remove…")).clicked() {
                        action = Some(("plugin.remove", p.id().to_string()));
                    }
                    match m.kind {
                        anicraft_engine::plugins::Kind::Exporter => {
                            if ui.button(tr!("Export…")).on_hover_text(tr!("Export the movie in this format")).clicked() {
                                action = Some(("export", p.id().to_string()));
                            }
                        }
                        anicraft_engine::plugins::Kind::Ease => {
                            ui.label(text(Type::Caption, trn!("{n} ease, in the ease menus", "{n} eases, in the ease menus", p.eases().len())));
                        }
                        _ => {
                            let why = blocked(&app.s, p);
                            let r = ui.add_enabled(why.is_none(), egui::Button::new(if m.params.is_empty() { tr!("Run") } else { tr!("Run…") }));
                            if let Some(w) = why {
                                r.on_disabled_hover_text(w);
                            } else if r.clicked() {
                                action = Some(("plugin.run", p.id().to_string()));
                            }
                        }
                    }
                });
            });
            if !m.description.is_empty() {
                ui.label(text(Type::Label, m.description.clone()));
            }
            let kb = Plugin::size(p).div_ceil(1024);
            // i18n-exempt: an id and a size (data)
            let line = if m.author.is_empty() { format!("{} · {kb} KB", m.id) } else { tr!("by {author} · {id} · {size} KB", author = m.author, id = m.id, size = kb) };
            ui.label(text(Type::Caption, line));
            if let Some(src) = p.source() {
                ui.label(text(Type::Path, src.display().to_string()).color(T.text_faint));
            }
            ui.separator();
        }
        ui.add_space(4.0);
        let folder = app.s.plugins.folder().map(|f| f.to_path_buf());
        form_row(ui, 96.0, tr!("Folder"), |ui| {
            ui.label(text(Type::Path, folder.as_ref().map_or_else(|| tr!("none").to_string(), |f| f.display().to_string())));
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button(tr!("Install Plug-in…")).clicked() {
                action = Some(("plugin.install", String::new()));
            }
            if ui.add_enabled(folder.is_some(), egui::Button::new(tr!("Reload"))).clicked() {
                action = Some(("plugin.reload", String::new()));
            }
            if let Some(f) = &folder
                && ui.add_enabled(f.exists(), egui::Button::new(tr!("Show Folder"))).clicked()
            {
                action = Some(("show", f.display().to_string()));
            }
        });
        ui.label(text(Type::Caption, tr!("Plug-ins run sandboxed: they can change the open document through AniCraft's commands, never your files. An export format only reads the frames; AniCraft writes its file.")));
    });
    a11y::name_title_bar(&shown, title);
    app.show_plugins = open;
    match action {
        Some(("plugin.run", id)) => run_or_ask(app, &id),
        Some(("export", id)) => crate::plugin_export_ui::open(app, &id),
        Some(("plugin.remove", id)) => app.request("plugin.remove", json!({"id": id})),
        Some(("plugin.install", _)) => app.invoke("plugin.install"),
        Some(("plugin.reload", _)) => app.invoke("plugin.reload"),
        Some(("show", f)) => match crate::app::open_in_system(std::path::Path::new(&f)) {
            Ok(()) => app.status_ok(tr!("Opened the plug-ins folder")),
            Err(e) => app.status_err(e),
        },
        _ => {}
    }
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    manager(app, ctx);
    dialog(app, ctx);
}

/// The status line after a plug-in command.
pub fn status(app: &mut App, id: &str, v: &Value) {
    match id {
        "plugin.run" => {
            // The plug-in's name and its last log line are its own text, shown as written.
            let name = v["name"].as_str().unwrap_or(tr!("Plug-in"));
            let msg = match v["log"].as_array().and_then(|l| l.last()).and_then(Value::as_str) {
                Some(m) => tr!("{name}: {message} — {undo} to undo", name = name, message = m, undo = app.undo_key()),
                None => tr!("{name} — {undo} to undo", name = name, undo = app.undo_key()),
            };
            app.status_ok(msg);
        }
        "plugin.install" => {
            let i = &v["installed"];
            let (name, version) = (i["name"].as_str().unwrap_or(""), i["version"].as_str().unwrap_or(""));
            if v["replaced"].is_null() {
                app.status_ok(tr!("Installed {name} {version} — find it in Commands › Plug-ins", name = name, version = version));
            } else {
                app.status_ok(tr!("Updated {name} {version} — find it in Commands › Plug-ins", name = name, version = version));
            }
        }
        "plugin.remove" => match v["name"].as_str() {
            Some(name) => app.status_ok(tr!("Removed {name}", name = name)),
            None => app.status_ok(tr!("Removed the plug-in")),
        },
        "plugin.reload" => {
            let n = v["installed"].as_u64().unwrap_or(0);
            let failed = v["failed"].as_array().map_or(0, Vec::len);
            if failed > 0 {
                app.status_err(trn!("{n} plug-in installed; {failed} could not be loaded", "{n} plug-ins installed; {failed} could not be loaded", n, failed = failed));
            } else {
                app.status_ok(trn!("{n} plug-in installed", "{n} plug-ins installed", n));
            }
        }
        _ => {}
    }
}
