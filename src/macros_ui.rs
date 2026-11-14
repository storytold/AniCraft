//! Saved commands in the desktop app (ADR 0097): Save As Command from the steps picked in the
//! History panel, the saved commands at the end of the Commands menu with their shortcuts, Run
//! Command… for a file, and Commands › Manage Saved Commands (run, rename, reorder, shortcut,
//! delete, add from a file). The `macro.*` commands do the work; the files live in the settings
//! folder (`platform::commands_dir`).

use crate::a11y::{self, Named};
use crate::app::App;
use crate::keymap;
use crate::theme::{T, Type, text};
use anicraft_engine::cmds_macro::{self, Saved};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// What the window keeps for saved commands between frames (view state, not saved).
#[derive(Default)]
pub struct Macros {
    /// The saved commands, as last read from the folder, in their order.
    pub list: Vec<Saved>,
    /// Files in the folder that aren't saved commands, with why.
    pub problems: Vec<(String, String)>,
    /// History steps picked for Save As Command (as `macro.history` numbers them).
    pub picked: BTreeSet<usize>,
    /// The step picked last (the anchor of a Shift+click range).
    anchor: Option<usize>,
    /// The history the picks belong to: (steps freed, steps listed).
    picked_for: (usize, usize),
    /// The Save As Command dialog, while it is open.
    pub save: Option<SaveDialog>,
    /// Commands › Manage Saved Commands.
    pub manage: Manage,
}

/// A picked step as Save As Command keeps it.
#[derive(Clone, Debug)]
pub struct StepView {
    pub index: usize,
    pub label: String,
    /// Why it can't be kept (`None`: it can).
    pub refused: Option<String>,
    /// What was rewritten so it replays elsewhere ("acts on the selection").
    pub note: Option<String>,
}

/// The Save As Command dialog.
pub struct SaveDialog {
    pub name: String,
    pub steps: Vec<StepView>,
    focus: bool,
}

/// The Manage Saved Commands window.
#[derive(Default)]
pub struct Manage {
    pub open: bool,
    /// The command being renamed and the name typed so far.
    renaming: Option<(String, String)>,
    focus: bool,
}

/// The folder the saved commands are in. Screenshot runs never touch the real one: they use a
/// temporary folder (unless `ANICRAFT_COMMANDS_DIR` names one). None in a browser.
pub fn folder(app: &App) -> Option<PathBuf> {
    if app.shot.is_some() && std::env::var_os("ANICRAFT_COMMANDS_DIR").is_none() && !crate::platform::WEB {
        return Some(std::env::temp_dir().join(format!("anicraft-commands-shot-{}", std::process::id())));
    }
    crate::platform::commands_dir()
}

fn folder_str(app: &App) -> Option<String> {
    folder(app).map(|f| f.display().to_string())
}

/// Reads the folder again, and lets the shortcut map bind what is in it.
pub fn rescan(app: &mut App) {
    let (list, problems) = folder(app).map(|f| cmds_macro::saved_commands(&f)).unwrap_or_default();
    app.macros.list = list;
    app.macros.problems = problems;
    keymap::set_macros(app.macros.list.iter().map(|c| c.name.as_str()));
    app.refresh_keymap();
}

/// Runs the saved command called `name` (from the menu, a shortcut or the Manage window).
pub fn run_saved(app: &mut App, name: &str) {
    let Some(folder) = folder_str(app) else { return };
    app.playing = false;
    app.request("macro.run", json!({"folder": folder, "name": name}));
}

/// Whether the steps picked so far still belong to this history; if not, they are let go.
fn sync_picks(app: &mut App) {
    let h = app.s.history();
    let now = (h.evicted, h.undo_len() + h.redo_len());
    let was = app.macros.picked_for;
    // New steps keep the numbers of the old ones; a freed step or a dropped redo moves them.
    if now.0 != was.0 || now.1 < was.1 {
        app.macros.picked.clear();
        app.macros.anchor = None;
    }
    app.macros.picked_for = now;
}

/// Picks history step `i` for Save As Command: Ctrl toggles it, Shift picks the range from the
/// step picked last.
pub fn pick(app: &mut App, i: usize, toggle: bool, range: bool) {
    sync_picks(app);
    let m = &mut app.macros;
    match (range, m.anchor) {
        (true, Some(a)) => {
            m.picked.extend(a.min(i)..=a.max(i));
        }
        _ if toggle && m.picked.contains(&i) => {
            m.picked.remove(&i);
        }
        _ => {
            m.picked.insert(i);
        }
    }
    m.anchor = Some(i);
}

/// The picked steps, in order (after letting go of picks the history no longer has).
pub fn picked(app: &mut App) -> Vec<usize> {
    sync_picks(app);
    app.macros.picked.iter().copied().collect()
}

pub fn clear_picks(app: &mut App) {
    app.macros.picked.clear();
    app.macros.anchor = None;
}

/// Opens Save As Command for the picked steps.
pub fn open_save(app: &mut App) {
    if folder(app).is_none() {
        app.status_err(tr!("Saved commands are kept in a folder on your computer: save them in the desktop app."));
        return;
    }
    let picked = picked(app);
    if picked.is_empty() {
        app.status_err(tr!("Pick the steps to save first: Ctrl+click them in the History panel (Shift+click picks a range)."));
        return;
    }
    let listed = app.s.execute("macro.history", &json!({})).unwrap_or(Value::Null);
    let rows = listed["steps"].as_array().cloned().unwrap_or_default();
    let steps: Vec<StepView> = picked
        .iter()
        .filter_map(|&i| rows.get(i))
        .map(|r| StepView {
            index: r["index"].as_u64().and_then(|n| usize::try_from(n).ok()).unwrap_or(0),
            label: r["label"].as_str().unwrap_or("").to_string(),
            refused: r["why"].as_str().map(str::to_string),
            note: r["rewritten"].as_str().map(str::to_string),
        })
        .collect();
    let n = app.macros.list.len() + 1;
    app.macros.save = Some(SaveDialog { name: tr!("Command {n}", n = n), steps, focus: true });
}

/// A step's undo label in the interface language (the engine's English is the key).
fn step_label(label: &str) -> String {
    crate::i18n::t_dyn(label).into_owned()
}

fn save_window(app: &mut App, ctx: &egui::Context) {
    let Some(d) = &mut app.macros.save else { return };
    let mut open = true;
    let mut save = false;
    let mut cancel = false;
    let title = tr!("Save As Command");
    let shown = egui::Window::new(title).id(egui::Id::new("save_as_command")).open(&mut open).collapsible(false).resizable(false).pivot(egui::Align2::CENTER_CENTER).default_pos(ctx.content_rect().center()).show(ctx, |ui| {
        ui.set_min_width(380.0);
        ui.horizontal(|ui| {
            ui.label(text(Type::Label, tr!("Name")));
            let r = ui.add(egui::TextEdit::singleline(&mut d.name).desired_width(280.0)).named(tr!("Saved command name"));
            if d.focus {
                r.request_focus();
                d.focus = false;
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                save = true;
            }
        });
        ui.add_space(6.0);
        ui.label(text(Type::Label, trn!("{n} step, run in order as one undo step:", "{n} steps, run in order as one undo step:", d.steps.len())));
        egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
            for st in &d.steps {
                ui.horizontal(|ui| {
                    ui.label(text(Type::Data, (st.index + 1).to_string()).color(T.text_dim));
                    ui.label(text(Type::Body, step_label(&st.label)));
                    match (&st.refused, &st.note) {
                        (Some(why), _) => {
                            ui.label(text(Type::Caption, tr!("can't be kept: {why}", why = crate::i18n::t_dyn(why))).color(T.error));
                        }
                        (None, Some(note)) => {
                            ui.label(text(Type::Caption, crate::i18n::t_dyn(note)).color(T.text_dim));
                        }
                        (None, None) => {}
                    }
                });
            }
        });
        let refused = d.steps.iter().filter(|s| s.refused.is_some()).count();
        ui.add_space(4.0);
        if refused > 0 {
            ui.label(text(Type::Caption, trn!("Take the step that can't be kept out of the picks (Ctrl+click it) to save the others.", "Take the {n} steps that can't be kept out of the picks (Ctrl+click them) to save the others.", refused)).color(T.error));
        } else {
            ui.label(text(Type::Caption, tr!("It runs on whatever is selected when you choose it in the Commands menu.")));
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let ok = refused == 0 && !d.name.trim().is_empty();
            if ui.add_enabled(ok, egui::Button::new(tr!("Save"))).clicked() {
                save = true;
            }
            if ui.button(tr!("Cancel")).clicked() {
                cancel = true;
            }
        });
    });
    a11y::name_title_bar(&shown, title);
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        cancel = true;
    }
    if !open || cancel {
        app.macros.save = None;
        return;
    }
    let ready = app.macros.save.as_ref().filter(|d| save && !d.name.trim().is_empty() && d.steps.iter().all(|s| s.refused.is_none()));
    if let Some(d) = ready {
        let history: Vec<usize> = d.steps.iter().map(|s| s.index).collect();
        let name = d.name.trim().to_string();
        if let Some(folder) = folder_str(app) {
            app.request("macro.save", json!({"folder": folder, "name": name, "history": history}));
        }
    }
}

/// The saved commands at the end of the Commands menu, as Animate lists them.
pub fn menu_items(app: &mut App, ui: &mut egui::Ui) {
    if folder(app).is_none() {
        return;
    }
    ui.separator();
    if app.macros.list.is_empty() {
        ui.label(text(Type::Caption, tr!("No saved commands yet: pick steps in the History panel and Save As Command")));
        return;
    }
    let names: Vec<String> = app.macros.list.iter().map(|c| c.name.clone()).collect();
    for name in names {
        let key = crate::platform::shortcut_label(app.keymap.primary(&keymap::macro_id(&name)).unwrap_or(""));
        if ui.add(egui::Button::new(name.as_str()).shortcut_text(text(Type::Key, key))).clicked() {
            ui.input_mut(|i| {
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
                i.consume_key(egui::Modifiers::NONE, egui::Key::Space);
            });
            ui.close();
            run_saved(app, &name);
        }
    }
}

/// Commands › Manage Saved Commands…
pub fn open_manage(app: &mut App) {
    rescan(app);
    app.macros.manage = Manage { open: true, ..Manage::default() };
}

/// Opens the shortcut editor on the saved command `name`, already waiting for its keys.
fn assign_shortcut(app: &mut App, ctx: &egui::Context, name: &str) {
    let id = keymap::macro_id(name);
    let Some(action) = app.keymap.actions.iter().find(|a| a.id == id).map(|a| a.id) else { return };
    crate::dialogs::shortcuts_for(app, ctx, name, action);
}

enum Act {
    Run(String),
    Move(String, usize),
    Rename(String, String),
    Delete(String),
    Shortcut(String),
}

fn manage_window(app: &mut App, ctx: &egui::Context) {
    if !app.macros.manage.open {
        return;
    }
    let Some(dir) = folder(app) else {
        app.macros.manage.open = false;
        return;
    };
    let mut open = true;
    let mut acts: Vec<Act> = Vec::new();
    let title = tr!("Manage Saved Commands");
    let shown = egui::Window::new(title).id(egui::Id::new("manage_saved_commands")).open(&mut open).collapsible(false).resizable(false).default_width(760.0).show(ctx, |ui| {
        ui.set_min_width(720.0);
        let list = app.macros.list.clone();
        if list.is_empty() {
            ui.label(text(Type::Label, tr!("No saved commands yet. Pick steps in the History panel (Ctrl+click), then Save As Command: they run again on the selection as one undo step.")));
        }
        let n = list.len();
        for (i, c) in list.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.set_min_height(crate::theme::TARGET);
                let mut renaming = app.macros.manage.renaming.take();
                if let Some((k, typed)) = &mut renaming
                    && *k == c.name
                {
                    let r = ui.add(egui::TextEdit::singleline(typed).desired_width(220.0)).named(&tr!("New name for “{name}”", name = c.name));
                    if app.macros.manage.focus {
                        r.request_focus();
                        app.macros.manage.focus = false;
                    }
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if ui.button(tr!("Rename")).clicked() || enter {
                        acts.push(Act::Rename(c.name.clone(), typed.trim().to_string()));
                    } else if ui.button(tr!("Cancel")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        renaming = None;
                    }
                    app.macros.manage.renaming = renaming;
                    return;
                }
                app.macros.manage.renaming = renaming;
                // Fixed columns, so the buttons line up row under row.
                let cell = |ui: &mut egui::Ui, w: f32, add: &mut dyn FnMut(&mut egui::Ui)| {
                    ui.allocate_ui_with_layout(egui::vec2(w, crate::theme::TARGET), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.set_min_width(w);
                        add(ui);
                    });
                };
                cell(ui, 210.0, &mut |ui| {
                    ui.add(egui::Label::new(text(Type::Body, &c.name)).truncate());
                });
                cell(ui, 64.0, &mut |ui| {
                    ui.label(text(Type::DataSmall, trn!("{n} step", "{n} steps", c.steps)).color(T.text_dim));
                });
                let key = app.keymap.primary(&keymap::macro_id(&c.name)).map(crate::platform::shortcut_label).unwrap_or_default();
                cell(ui, 96.0, &mut |ui| {
                    ui.add(egui::Label::new(text(Type::Key, key.as_str()).color(T.text_dim)).truncate());
                });
                if ui.button(tr!("Run")).named(&tr!("Run “{name}”", name = c.name)).on_hover_text(tr!("Run it on the selection (one undo step)")).clicked() {
                    acts.push(Act::Run(c.name.clone()));
                }
                let gap = crate::theme::TARGET + 4.0;
                if i > 0 {
                    if crate::timeline::icon_button(ui, &tr!("Move “{name}” up", name = c.name), None, crate::icons::Icon::ChevronUp).clicked() {
                        acts.push(Act::Move(c.name.clone(), i - 1));
                    }
                } else {
                    ui.allocate_exact_size(egui::vec2(gap, crate::theme::TARGET), egui::Sense::hover());
                }
                if i + 1 < n {
                    if crate::timeline::icon_button(ui, &tr!("Move “{name}” down", name = c.name), None, crate::icons::Icon::ChevronDown).clicked() {
                        acts.push(Act::Move(c.name.clone(), i + 1));
                    }
                } else {
                    ui.allocate_exact_size(egui::vec2(gap, crate::theme::TARGET), egui::Sense::hover());
                }
                if ui.button(tr!("Shortcut…")).named(&tr!("Shortcut for “{name}”", name = c.name)).on_hover_text(tr!("Give it a key in Keyboard Shortcuts")).clicked() {
                    acts.push(Act::Shortcut(c.name.clone()));
                }
                if ui.button(tr!("Rename…")).named(&tr!("Rename “{name}”", name = c.name)).clicked() {
                    app.macros.manage.renaming = Some((c.name.clone(), c.name.clone()));
                    app.macros.manage.focus = true;
                }
                if ui.button(tr!("Delete…")).named(&tr!("Delete “{name}”", name = c.name)).on_hover_text(tr!("Delete this saved command's file (asks first)")).clicked() {
                    acts.push(Act::Delete(c.name.clone()));
                }
            });
        }
        for (file, why) in &app.macros.problems {
            ui.label(text(Type::Caption, tr!("{file} isn't a saved command: {why}", file = file, why = crate::i18n::t_dyn(why))).color(T.error));
        }
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button(tr!("Add from File…")).on_hover_text(tr!("Copy a saved command someone shared into your commands")).clicked() {
                app.invoke("macro.import");
            }
            ui.label(text(Type::Path, dir.display().to_string()).color(T.text_faint));
        });
        ui.label(text(Type::Caption, tr!("Saved commands are small JSON files: share one by sending it; Commands › Run Command… runs a file without adding it.")));
    });
    a11y::name_title_bar(&shown, title);
    let folder = dir.display().to_string();
    for act in acts {
        match act {
            Act::Run(name) => run_saved(app, &name),
            Act::Move(name, index) => app.request("macro.move", json!({"folder": folder, "name": name, "index": index})),
            Act::Rename(name, to) => app.request("macro.rename", json!({"folder": folder, "name": name, "to": to})),
            Act::Delete(name) => app.request("macro.delete", json!({"folder": folder, "name": name})),
            Act::Shortcut(name) => assign_shortcut(app, ctx, &name),
        }
    }
    if !open {
        app.macros.manage = Manage::default();
    }
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    save_window(app, ctx);
    manage_window(app, ctx);
}

/// Moves (or with `to` None, drops) the shortcut keys kept for the saved command `from`.
fn move_shortcut(app: &mut App, from: &str, to: Option<&str>) {
    let Some(keys) = app.prefs.shortcuts.remove(&keymap::macro_id(from)) else { return };
    if let Some(to) = to {
        app.prefs.shortcuts.insert(keymap::macro_id(to), keys);
    }
    app.save_prefs();
}

/// After a `macro.*` command ran: the list again, the status line, and a renamed or deleted
/// command's shortcut following it.
pub fn done(app: &mut App, id: &str, v: &Value) {
    let name = v["name"].as_str().unwrap_or("").to_string();
    match id {
        "macro.run" => {
            let n = v["steps"].as_u64().unwrap_or(0);
            app.status_ok(trn!("Ran “{name}” ({n} step) — {undo} to undo", "Ran “{name}” ({n} steps) — {undo} to undo", n, name = name, undo = app.undo_key()));
            return;
        }
        "macro.save" => {
            app.macros.save = None;
            clear_picks(app);
            app.status_ok(tr!("Saved “{name}”: find it at the end of the Commands menu", name = name));
        }
        "macro.rename" => {
            app.macros.manage.renaming = None;
            if let Some(from) = v["from"].as_str() {
                move_shortcut(app, from, Some(&name));
            }
        }
        "macro.delete" => {
            let gone = v["deleted"].as_str().unwrap_or("").to_string();
            move_shortcut(app, &gone, None);
            app.status_ok(tr!("Deleted “{name}”", name = gone));
        }
        "macro.import" => app.status_ok(tr!("Added “{name}” to your commands", name = name)),
        _ => {}
    }
    rescan(app);
}
