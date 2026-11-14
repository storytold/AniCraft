//! Window › Workspace › Manage Workspaces… (ADR 0087): every workspace, with Rename and
//! Delete for your own and Reset for each one changed since it was built or saved. A delete
//! asks with a second click (the layout can't be brought back), never with a dialog.

use crate::a11y::Named;
use crate::app::App;
use crate::dock::Workspaces;
use crate::theme::{T, Type, text};

/// The window's state (view state, not saved).
#[derive(Default)]
pub struct Manage {
    pub open: bool,
    /// The workspace being renamed and the name typed so far.
    renaming: Option<(String, String)>,
    /// The workspace whose Delete was clicked once.
    confirm: Option<String>,
    focus: bool,
}

pub fn open(app: &mut App) {
    app.manage_workspaces = Manage { open: true, ..Manage::default() };
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    if !app.manage_workspaces.open {
        return;
    }
    let mut open = true;
    let title = tr!("Manage Workspaces");
    let shown = egui::Window::new(title).id(egui::Id::new("manage_workspaces")).open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
        ui.set_min_width(380.0);
        let current = app.prefs.workspaces.current.clone();
        for key in app.prefs.workspaces.keys() {
            let builtin = crate::dock::built_in(&key).is_some();
            let name = crate::i18n::t_dyn(Workspaces::name(&key)).into_owned();
            ui.horizontal(|ui| {
                ui.set_min_height(crate::theme::TARGET);
                if let Some((k, typed)) = &mut app.manage_workspaces.renaming
                    && *k == key
                {
                    let r = ui.add(egui::TextEdit::singleline(typed).desired_width(200.0)).named(&tr!("New name for “{name}”", name = name));
                    if app.manage_workspaces.focus {
                        r.request_focus();
                        app.manage_workspaces.focus = false;
                    }
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let typed = typed.clone();
                    if ui.button(tr!("Rename")).clicked() || enter {
                        if app.rename_workspace(&key, &typed) {
                            app.manage_workspaces.renaming = None;
                        }
                    } else if ui.button(tr!("Cancel")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        app.manage_workspaces.renaming = None;
                    }
                    return;
                }
                ui.label(text(Type::Body, &name));
                if key == current {
                    ui.label(text(Type::Caption, tr!("in use")).color(T.text_dim));
                }
                if builtin {
                    ui.label(text(Type::Caption, tr!("built in")).color(T.text_faint));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !builtin {
                        let sure = app.manage_workspaces.confirm.as_deref() == Some(key.as_str());
                        let label = if sure { tr!("Delete for Good") } else { tr!("Delete") };
                        let tip = if sure { tr!("Click again to delete it; this can't be undone") } else { tr!("Delete this workspace (asks with a second click)") };
                        if ui.button(label).on_hover_text(tip).clicked() {
                            if sure {
                                app.manage_workspaces.confirm = None;
                                app.delete_workspace(&key);
                            } else {
                                app.manage_workspaces.confirm = Some(key.clone());
                            }
                        }
                        if ui.button(tr!("Rename…")).clicked() {
                            app.manage_workspaces.renaming = Some((key.clone(), key.clone()));
                            app.manage_workspaces.confirm = None;
                            app.manage_workspaces.focus = true;
                        }
                    }
                    let changed = key == current || app.prefs.workspaces.changed(&key);
                    if ui.add_enabled(changed, egui::Button::new(tr!("Reset"))).on_hover_text(tr!("Back to the workspace as built or saved")).clicked() {
                        if key == current {
                            app.reset_workspace();
                        } else {
                            app.prefs.workspaces.forget_changes(&key);
                            app.dock_view.save = true;
                        }
                    }
                });
            });
        }
        ui.separator();
        ui.label(text(Type::Caption, tr!("Built-in workspaces can be reset but not deleted. Save Workspace… adds one of your own.")));
    });
    crate::a11y::name_title_bar(&shown, title);
    if !open {
        app.manage_workspaces = Manage::default();
    }
}
