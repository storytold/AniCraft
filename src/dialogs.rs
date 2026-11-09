//! Modal and floating dialogs: destructive-action confirmation (ADR 0004), the unsaved-changes
//! guard on close, the command palette (Ctrl+K) and the keyboard-shortcut sheet (F1).

use crate::app::{App, Tool};
use crate::i18n::{self, tl};
use crate::prefs::{FontChoice, RenderMode, StageRenderer};
use crate::theme::{T, Type, text, with_type};
use anicraft_engine::confirm::Confirmation;
use anicraft_engine::{CommandSpec, commands};
use crate::a11y::{self, Named};
use egui::Key;
use serde_json::{Value, json};

/// A command waiting for the user's yes/no.
pub struct PendingConfirm {
    pub id: String,
    pub params: Value,
    pub conf: Confirmation,
    pub dont_ask: bool,
}

#[derive(Default)]
pub struct Palette {
    pub query: String,
    pub selected: usize,
}

pub fn show_all(app: &mut App, ctx: &egui::Context) {
    preferences(app, ctx);
    licenses(app, ctx);
    confirm_modal(app, ctx);
    close_modal(app, ctx);
    palette(app, ctx);
    shortcut_sheet(app, ctx);
}

fn danger_button(label: &str) -> egui::Button<'static> {
    egui::Button::new(text(Type::Body, label).color(T.on_fill).strong()).fill(T.danger).min_size(egui::vec2(120.0, crate::theme::TARGET + 4.0))
}

/// The engine's confirmation message in the interface language: its template and counts
/// translated, then filled in.
fn confirm_message(c: &Confirmation) -> String {
    use anicraft_engine::confirm::Arg;
    let values: Vec<(&str, String)> = c
        .args
        .iter()
        .map(|(k, v)| match v {
            Arg::Text(t) => (*k, t.clone()),
            Arg::Count(n, one, other) => (*k, i18n::trn(*n as u64, one, other)),
        })
        .collect();
    let refs: Vec<(&str, &str)> = values.iter().map(|(k, v)| (*k, v.as_str())).collect();
    i18n::fmt(c.template, &refs)
}

fn confirm_modal(app: &mut App, ctx: &egui::Context) {
    let Some(p) = &mut app.confirm else { return };
    let mut decision: Option<bool> = None;
    let irreversible = p.conf.irreversible;
    egui::Modal::new(egui::Id::unique("confirm")).show(ctx, |ui| crate::theme::reading(ui, |ui| {
        ui.set_width(420.0);
        ui.add_space(4.0);
        ui.label(text(Type::Title, tl!(&p.conf.title)));
        ui.add_space(6.0);
        ui.label(text(Type::Body, confirm_message(&p.conf)).color(T.text_dim));
        ui.add_space(4.0);
        if irreversible {
            ui.label(text(Type::Caption, tl!("This cannot be undone.")).color(T.error));
        } else {
            crate::theme::checkbox(ui, &mut p.dont_ask, text(Type::Caption, tl!("Don't ask again this session")).color(T.text_dim));
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.add(egui::Button::new(tl!("Cancel")).min_size(egui::vec2(96.0, crate::theme::TARGET + 4.0))).clicked() {
                decision = Some(false);
            }
            ui.with_layout(crate::theme::end_row(egui::Align::Center), |ui| {
                if ui.add(danger_button(tl!(&p.conf.action))).clicked() {
                    decision = Some(true);
                }
            });
        });
        ui.add_space(2.0);
        let hint = tl!(if irreversible { "Esc cancels · click the red button to proceed" } else { "Enter confirms · Esc cancels" });
        ui.label(text(Type::Caption, hint));
    }));
    // Irreversible actions are never one keystroke away: Enter does not confirm them.
    let (esc, enter) = ctx.input_mut(|i| (i.consume_key(egui::Modifiers::NONE, Key::Escape), i.consume_key(egui::Modifiers::NONE, Key::Enter)));
    if esc {
        decision = Some(false);
    } else if enter && !irreversible {
        decision = Some(true);
    }
    match decision {
        Some(true) => {
            if let Some(p) = app.confirm.take() {
                if p.dont_ask {
                    app.skip_confirm.insert(p.id.clone());
                }
                let mut params = p.params;
                if let Some(obj) = params.as_object_mut() {
                    obj.insert("confirm".into(), json!(true));
                }
                app.perform(&p.id, params);
            }
        }
        Some(false) => {
            app.confirm = None;
            app.close_after_save = false;
            app.status_ok(tl!("Cancelled"));
        }
        None => {}
    }
}

fn close_modal(app: &mut App, ctx: &egui::Context) {
    if !app.close_dialog || app.prompt.is_some() || app.confirm.is_some() {
        return;
    }
    #[derive(PartialEq)]
    enum Choice {
        Save,
        Discard,
        Cancel,
    }
    let mut choice = None;
    let name = app.s.path.as_ref().and_then(|p| p.file_name()).map_or_else(|| tl!("Untitled").to_string(), |n| n.to_string_lossy().into_owned());
    egui::Modal::new(egui::Id::unique("close_guard")).show(ctx, |ui| crate::theme::reading(ui, |ui| {
        ui.set_width(440.0);
        ui.label(text(Type::Title, i18n::fmt("Save changes to “{name}” before closing?", &[("name", &name)])));
        ui.add_space(6.0);
        ui.label(text(Type::Body, tl!("If you don't save, your changes since the last save will be lost.")).color(T.text_dim));
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.add(danger_button(tl!("Don't Save"))).clicked() {
                choice = Some(Choice::Discard);
            }
            ui.with_layout(crate::theme::end_row(egui::Align::Center), |ui| {
                if ui.add(egui::Button::new(text(Type::Body, tl!("Save")).strong().color(T.on_fill)).fill(T.accent).min_size(egui::vec2(96.0, crate::theme::TARGET + 4.0))).clicked() {
                    choice = Some(Choice::Save);
                }
                if ui.add(egui::Button::new(tl!("Cancel")).min_size(egui::vec2(96.0, crate::theme::TARGET + 4.0))).clicked() {
                    choice = Some(Choice::Cancel);
                }
            });
        });
        ui.label(text(Type::Caption, tl!("Enter saves · Esc cancels")));
    }));
    let (esc, enter) = ctx.input_mut(|i| (i.consume_key(egui::Modifiers::NONE, Key::Escape), i.consume_key(egui::Modifiers::NONE, Key::Enter)));
    if esc {
        choice = Some(Choice::Cancel);
    } else if enter {
        choice = Some(Choice::Save);
    }
    match choice {
        Some(Choice::Save) => {
            app.close_dialog = false;
            app.close_after_save = true;
            if app.s.path.is_some() {
                app.perform("file.save", json!({}));
            } else {
                app.invoke("file.save_as");
            }
        }
        Some(Choice::Discard) => {
            app.allow_close = true;
        }
        Some(Choice::Cancel) => {
            app.close_dialog = false;
        }
        None => {}
    }
}

// ----- command palette --------------------------------------------------------------------------

#[derive(Clone)]
enum Entry {
    Command(CommandSpec),
    Tool(Tool),
    View(&'static str, &'static str, fn(&mut App)),
    /// An installed plug-in: `(id, menu label)`.
    Plugin(String, String),
}

impl Entry {
    /// The title in English (`tr` = identity) or in the interface language (`tr` = `i18n::t`).
    fn title_with(&self, tr: impl Fn(&str) -> String) -> String {
        let path = |menu: &str| menu.split('/').map(&tr).collect::<Vec<_>>().join(" › ");
        match self {
            Entry::Command(c) if c.menu.is_empty() => tr(c.label),
            Entry::Command(c) => format!("{} › {}", path(c.menu), tr(c.label)),
            Entry::Tool(t) => format!("{} › {}", tr("Tool"), tr(t.label())),
            Entry::View(name, _, _) => format!("{} › {}", tr("View"), tr(name)),
            // The plug-in's own name stays as its author wrote it.
            Entry::Plugin(_, label) => format!("{} › {} › {label}", tr("Commands"), tr("Plug-ins")),
        }
    }

    fn title(&self) -> String {
        match self {
            // A command's own `@id` row first, as in the menus.
            Entry::Command(c) if c.menu.is_empty() => crate::i18n::t_id(c.id, c.label).to_string(),
            Entry::Command(c) => format!("{} › {}", c.menu.split('/').map(crate::i18n::t).collect::<Vec<_>>().join(" › "), crate::i18n::t_id(c.id, c.label)),
            _ => self.title_with(|s| crate::i18n::t(s).to_string()),
        }
    }

    /// The English title too, so the palette finds a command by either name.
    fn title_en(&self) -> String {
        self.title_with(str::to_string)
    }
    fn shortcut(&self, km: &crate::keymap::Keymap) -> String {
        match self {
            Entry::Command(c) => km.primary(c.id).unwrap_or("").to_string(),
            Entry::Tool(t) => km.primary(crate::keymap::tool_id(*t)).unwrap_or("").to_string(),
            Entry::View(_, sc, _) => (*sc).to_string(),
            Entry::Plugin(..) => String::new(),
        }
    }
}

/// Commands the palette can run without extra input (everything optional, or prompted).
fn runnable(c: &CommandSpec) -> bool {
    if matches!(c.id, "file.open" | "file.recovery_list" | "file.save_as" | "file.import" | "file.export_png" | "file.export_gif" | "file.export_svg" | "file.export_sprite_sheet" | "file.export_video" | "file.export_lottie" | "file.export_png_sequence" | "modify.convert_to_symbol" | "layer.rename" | "scene.rename" | "plugin.install") {
        return true;
    }
    let inner = c.params.trim().trim_start_matches('{').trim_end_matches('}');
    inner.split(',').map(str::trim).filter(|k| !k.is_empty()).all(|k| k.split(':').next().is_some_and(|name| name.trim().ends_with('?')))
}

fn entries() -> Vec<Entry> {
    let mut v: Vec<Entry> = commands().into_iter().filter(runnable).map(Entry::Command).collect();
    v.extend(Tool::ALL.iter().map(|t| Entry::Tool(*t)));
    v.push(Entry::View("Play / Stop", "Enter", App::toggle_play));
    v.push(Entry::View("Onion Skin", "", |a| a.onion = !a.onion));
    v.push(Entry::View("Fit Stage in Window", "Ctrl+0", |a| a.view.fit = true));
    v.push(Entry::View("Keyboard Shortcuts", "F1", |a| a.show_shortcuts = true));
    v.push(Entry::View("Project Files & Docs", "", |a| a.show_files = true));
    v.push(Entry::View("Licenses & Notices", "", |a| a.show_licenses = true));
    v.push(Entry::View("Preferences", "Ctrl+U", App::open_prefs));
    v
}

/// Subsequence match score (higher is better); `None` when not all query chars appear in order.
fn fuzzy(query: &str, text: &str) -> Option<i32> {
    let text = text.to_lowercase();
    let mut score = 0;
    let mut last = 0usize;
    let mut chars = text.char_indices();
    for q in query.to_lowercase().chars().filter(|c| !c.is_whitespace()) {
        let (i, _) = chars.by_ref().find(|(_, c)| *c == q)?;
        score += if i == last { 3 } else { 1 };
        if i == 0 || text[..i].ends_with([' ', '›', '.', '_']) {
            score += 4;
        }
        last = i + q.len_utf8();
    }
    Some(score - (text.len() as i32) / 16)
}

fn palette(app: &mut App, ctx: &egui::Context) {
    let km = &app.keymap;
    let Some(p) = &mut app.palette else { return };
    let mut all = entries();
    all.extend(app.s.plugins.list().filter(|p| crate::plugins_ui::is_command(p)).map(|p| Entry::Plugin(p.id().to_string(), crate::plugins_ui::label(p))));
    let mut matches: Vec<(i32, Entry, String)> =
        all.into_iter().filter_map(|e| {
            let key = e.shortcut(km);
            fuzzy(&p.query, &format!("{} {} {key}", e.title(), e.title_en())).map(|s| (s, e, key))
        }).collect();
    matches.sort_by_key(|m| std::cmp::Reverse(m.0));
    matches.truncate(14);
    p.selected = p.selected.min(matches.len().saturating_sub(1));
    let (up, down, enter, esc) = ctx.input_mut(|i| {
        (
            i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
            i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
            i.consume_key(egui::Modifiers::NONE, Key::Enter),
            i.consume_key(egui::Modifiers::NONE, Key::Escape),
        )
    });
    if up {
        p.selected = p.selected.saturating_sub(1);
    }
    if down && p.selected + 1 < matches.len() {
        p.selected += 1;
    }
    let mut run: Option<Entry> = None;
    let mut close = esc;
    let selected = p.selected;
    egui::Area::new(egui::Id::unique("palette")).order(egui::Order::Foreground).anchor(egui::Align2::CENTER_TOP, [0.0, 70.0]).show(ctx, |ui| crate::theme::reading(ui, |ui| {
        egui::Frame::popup(ui.style()).fill(T.panel_alt).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
            ui.set_width(560.0);
            let r = ui.add(egui::TextEdit::singleline(&mut p.query).hint_text(tl!("Type a command, tool or setting…")).desired_width(f32::INFINITY).font(egui::TextStyle::Heading)).named(tl!("Command palette"));
            r.request_focus();
            if r.changed() {
                p.selected = 0;
            }
            ui.add_space(6.0);
            if matches.is_empty() {
                ui.label(text(Type::Caption, tl!("No matching commands")));
            }
            for (i, (_, e, key)) in matches.iter().enumerate() {
                let sel = i == selected;
                let resp = ui
                    .horizontal(|ui| {
                        let lab = ui.selectable_label(sel, e.title());
                        // No label for an entry without a shortcut: an empty one is an unnamed
                        // node for screen readers (storytold's egui fork exposes it).
                        if !key.is_empty() {
                            ui.with_layout(crate::theme::end_row(egui::Align::Center), |ui| {
                                ui.label(text(Type::Key, crate::platform::shortcut_label(key)));
                            });
                        }
                        lab
                    })
                    .inner;
                if resp.clicked() {
                    run = Some(e.clone());
                }
            }
            ui.add_space(4.0);
            ui.label(text(Type::Caption, tl!("Up/Down choose · Enter run · Esc close")));
        });
    }));
    if enter {
        run = matches.get(selected).map(|(_, e, _)| e.clone());
    }
    if run.is_some() {
        close = true;
    }
    if close {
        app.palette = None;
    }
    match run {
        Some(Entry::Command(c)) => app.invoke(c.id),
        Some(Entry::Tool(t)) => app.tool = t,
        Some(Entry::View(_, _, f)) => f(app),
        Some(Entry::Plugin(id, _)) => crate::plugins_ui::run_or_ask(app, &id),
        None => {}
    }
}

// ----- preferences ------------------------------------------------------------------------------

/// A font picker: the bundled default plus every installed family, filterable.
fn font_picker(ui: &mut egui::Ui, id: &str, name: &str, current: &FontChoice, families: Option<&[String]>, bundled: &str) -> Option<FontChoice> {
    let mut picked = None;
    let filter_id = egui::Id::unique(("font_filter", id));
    let shown = match current {
        FontChoice::Bundled => i18n::fmt("Default ({font})", &[("font", bundled)]),
        FontChoice::System(name) => name.clone(),
    };
    egui::ComboBox::from_id_salt(id).width(260.0).selected_text(text(Type::Body, shown)).show_ui(ui, |ui| {
        if ui.selectable_label(*current == FontChoice::Bundled, text(Type::Body, i18n::fmt("Default (bundled: {font})", &[("font", bundled)]))).clicked() {
            picked = Some(FontChoice::Bundled);
        }
        ui.separator();
        match families {
            None => {
                ui.label(text(Type::Caption, tl!("Scanning installed fonts…")));
            }
            Some(list) => {
                let mut filter: String = ui.ctx().data_mut(|d| d.get_temp(filter_id).unwrap_or_default());
                ui.add(egui::TextEdit::singleline(&mut filter).hint_text(tl!("Filter fonts…")).desired_width(240.0)).set_name(tl!("Filter fonts"));
                let f = filter.to_lowercase();
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for fam in list.iter().filter(|n| f.is_empty() || n.to_lowercase().contains(&f)) {
                        let on = matches!(current, FontChoice::System(n) if n == fam);
                        if ui.selectable_label(on, text(Type::Body, fam.as_str())).clicked() {
                            picked = Some(FontChoice::System(fam.clone()));
                        }
                    }
                });
                ui.ctx().data_mut(|d| d.insert_temp(filter_id, filter));
            }
        }
    })
    .response
    .set_name(name);
    picked
}

fn preferences(app: &mut App, ctx: &egui::Context) {
    if !app.show_prefs {
        return;
    }
    let mut open = true;
    let mut p = app.prefs.clone();
    let sys = app.system_fonts.clone();
    let (bundled_ui, bundled_mono) = crate::theme::bundled_names();
    let mut reset_confirms = false;
    let title = tl!(if crate::platform::OS == crate::platform::Os::Mac { "Settings" } else { "Preferences" });
    let shown = egui::Window::new(title)
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .default_width(560.0)
        .show(ctx, |ui| crate::theme::reading(ui, |ui| {
            // Taller than small windows: the sections scroll, the footer stays.
            let max_h = (ctx.content_rect().height() - 160.0).max(200.0);
            egui::ScrollArea::vertical().max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
                // A two-column form: labels in a fixed left column, controls and the notes that
                // explain them in the right one.
                use crate::theme::{FORM_LABEL_DIALOG as LW, Section, form_note, form_row, section};
                // Under a radio button or checkbox, notes line up with its text, not its box.
                // (`form_note` adds one item spacing after its indent, like a row's label column.)
                let toggle_indent = (ui.spacing().icon_width + ui.spacing().icon_spacing - ui.spacing().item_spacing.x).max(0.0);

                section(ui, "prefs_appearance", Section::Rule, tl!("Appearance"), None, |ui| {
                    form_row(ui, LW, "Interface language", |ui| {
                        // Auto, then English and each language this build has the catalogs of, each by
                        // its own name.
                        let auto = p.language == crate::i18n::AUTO;
                        let current = crate::i18n::from_pref(&p.language);
                        let shown = if auto { format!("{} ({})", tl!("Auto"), crate::i18n::name(current)) } else { crate::i18n::name(current).to_string() };
                        egui::ComboBox::from_id_salt("pref_language").width(260.0).selected_text(text(Type::Body, shown)).show_ui(ui, |ui| {
                            if ui.selectable_label(auto, text(Type::Body, tl!("Auto"))).clicked() {
                                p.language = crate::i18n::AUTO.to_string();
                            }
                            for lang in crate::i18n::languages() {
                                if ui.selectable_label(!auto && lang == current, text(Type::Body, crate::i18n::name(lang))).clicked() {
                                    p.language = crate::i18n::code(lang).to_string();
                                }
                            }
                        })
                        .response
                        .set_name(tl!("Interface language"));
                    });
                    form_note(ui, LW, text(Type::Caption, tl!("Auto follows the system's language. Document text, file names and command ids stay as they are.")));
                    ui.add_space(4.0);
                    form_row(ui, LW, "Theme", |ui| {
                        crate::theme::column(ui, |ui| {
                            for (palette, name, tip) in crate::theme::Palette::ALL {
                                crate::theme::radio_value(ui, &mut p.theme, palette, text(Type::Body, tl!(name)));
                                form_note(ui, toggle_indent, text(Type::Caption, tl!(tip)));
                                ui.add_space(2.0);
                            }
                        });
                    });
                    ui.add_space(4.0);
                    form_row(ui, LW, "Interface font", |ui| {
                        if let Some(c) = font_picker(ui, "pref_ui_font", "Interface font", &p.ui_font, sys.as_ref().map(|s| s.families.as_slice()), bundled_ui) {
                            p.ui_font = c;
                        }
                    });
                    form_row(ui, LW, "Data font", |ui| {
                        let list = sys.as_ref().map(|s| s.monospace.as_slice());
                        if let Some(c) = font_picker(ui, "pref_data_font", "Data font", &p.data_font, list, bundled_mono) {
                            p.data_font = c;
                        }
                    });
                    form_note(ui, LW, text(Type::Caption, tl!("The data font shows numbers, timecode, rulers and shortcuts; only monospaced families are listed so digits stay aligned.")));
                    ui.add_space(4.0);
                    form_row(ui, LW, "Preview", |ui| {
                        crate::theme::column(ui, |ui| {
                            ui.label(text(Type::Body, tl!("Layer names, menus and dialog text read like this.")));
                            ui.label(text(Type::Data, tl!("Frame 0042 · 1.75 s · X 1280.5 Y 720.0 · Ctrl+Shift+S")));
                        });
                    });
                    ui.add_space(4.0);
                    form_row(ui, LW, "Interface scale", |ui| {
                        let mut pct = p.ui_scale * 100.0;
                        let r = with_type(ui, Type::Data, |ui| {
                            a11y::slider(ui, &mut pct, (crate::prefs::SCALE_RANGE.start() * 100.0)..=(crate::prefs::SCALE_RANGE.end() * 100.0), "Interface scale", a11y::SliderLook { suffix: " %", step: Some(5.0), max_decimals: Some(0), ..Default::default() })
                        });
                        // Apply on release so the slider doesn't move under the pointer while dragging.
                        if r.drag_stopped() || (r.changed() && !r.dragged()) {
                            p.ui_scale = pct / 100.0;
                        }
                    });
                });

                section(ui, "prefs_stage", Section::Rule, tl!("Stage"), None, |ui| {
                    form_row(ui, LW, "Rendering", |ui| {
                        crate::theme::column(ui, |ui| {
                            for (mode, name, tip) in RenderMode::ALL {
                                crate::theme::radio_value(ui, &mut p.render_mode, mode, text(Type::Body, tl!(name)));
                                form_note(ui, toggle_indent, text(Type::Caption, tl!(tip)));
                                ui.add_space(2.0);
                            }
                        });
                    });
                    if crate::gpu_stage::AVAILABLE {
                        ui.add_space(4.0);
                        form_row(ui, LW, "Renderer", |ui| {
                            crate::theme::column(ui, |ui| {
                                for (choice, name, tip) in StageRenderer::ALL {
                                    crate::theme::radio_value(ui, &mut p.stage_renderer, choice, text(Type::Body, tl!(name)));
                                    form_note(ui, toggle_indent, text(Type::Caption, tl!(tip)));
                                    ui.add_space(2.0);
                                }
                            });
                        });
                        use crate::gpu_stage::State;
                        let status = match app.stage_cache.gpu.state() {
                            State::Off => None,
                            State::Starting => Some(tl!("Starting the graphics card…").to_string()),
                            // The adapter's name and the driver's reason stay as they are.
                            State::Running(adapter) => Some(i18n::fmt("Drawing on {adapter}.", &[("adapter", &adapter.to_string())])),
                            State::Failed(why) => Some(i18n::fmt("The graphics card can't draw the stage, so the CPU does: {reason}", &[("reason", &why.to_string())])),
                        };
                        if crate::gpu_stage::wanted(p.stage_renderer) != p.stage_renderer {
                            form_note(ui, LW, text(Type::Caption, tl!("ANICRAFT_STAGE_RENDERER is set, and it takes precedence over this choice.")));
                        }
                        if let Some(s) = status {
                            form_note(ui, LW, text(Type::Caption, s));
                        }
                    }
                });

                section(ui, "prefs_timeline", Section::Rule, tl!("Timeline"), None, |ui| {
                    form_row(ui, LW, "Extra start gap", |ui| {
                        with_type(ui, Type::Data, |ui| {
                            ui.add(egui::DragValue::new(&mut p.timeline_start_gap).range(crate::prefs::GAP_RANGE).speed(0.5).max_decimals(0).suffix(" pt")).set_name("Extra start gap");
                        });
                    });
                    form_note(ui, LW, text(Type::Caption, tl!("The timeline always leaves room before frame 1 for the playhead; this adds more. It stays the same at every zoom.")));
                });

                section(ui, "prefs_files", Section::Rule, tl!("Files"), None, |ui| {
                    form_row(ui, LW, "File dialogs", |ui| {
                        crate::theme::column(ui, |ui| {
                            crate::theme::checkbox(ui, &mut p.native_dialogs, text(Type::Body, tl!("Use the system's open and save dialogs")));
                            form_note(ui, toggle_indent, text(Type::Caption, tl!("Turn off on Linux desktops without an XDG desktop portal; AniCraft then asks for a path instead.")));
                        });
                    });
                    ui.add_space(4.0);
                    form_row(ui, LW, "Autosave", |ui| {
                        crate::theme::column(ui, |ui| {
                            crate::theme::checkbox(ui, &mut p.autosave, text(Type::Body, tl!("Keep a recovery copy of unsaved work")));
                            form_note(
                                ui,
                                toggle_indent,
                                text(Type::Caption, tl!("If AniCraft closes unexpectedly, the next start offers to restore it. Copies stay in the settings folder, never over your files, and go away when you save.")),
                            );
                        });
                    });
                    ui.add_enabled_ui(p.autosave, |ui| {
                        form_row(ui, LW, "Every", |ui| {
                            let shown = crate::prefs::AUTOSAVE_INTERVALS
                                .iter()
                                .find(|(s, _)| *s == p.autosave_secs)
                                .map_or_else(|| i18n::fmt("{n} seconds", &[("n", &p.autosave_secs.to_string())]), |(_, l)| tl!(*l).to_string());
                            egui::ComboBox::from_id_salt("pref_autosave_secs").width(160.0).selected_text(text(Type::Body, shown)).show_ui(ui, |ui| {
                                for (secs, label) in crate::prefs::AUTOSAVE_INTERVALS {
                                    ui.selectable_value(&mut p.autosave_secs, secs, text(Type::Body, tl!(label)));
                                }
                            })
                            .response
                            .set_name("Autosave every");
                        });
                        form_row(ui, LW, "Or after", |ui| {
                            with_type(ui, Type::Data, |ui| {
                                ui.add(egui::DragValue::new(&mut p.autosave_edits).range(crate::prefs::AUTOSAVE_EDITS_RANGE).speed(0.5).suffix(format!(" {}", tl!("changes")))).set_name(tl!("Autosave after this many changes"));
                            });
                        });
                        form_note(ui, LW, text(Type::Caption, tl!("Whichever comes first. 0 changes: by time only. Copies are written in the background, so drawing never waits.")));
                    });
                });

                section(ui, "prefs_safety", Section::Rule, tl!("Safety"), None, |ui| {
                    form_row(ui, LW, "Confirmations", |ui| {
                        let n = app.skip_confirm.len();
                        if ui.add_enabled(n > 0, egui::Button::new(text(Type::Body, tl!("Show all again")))).clicked() {
                            reset_confirms = true;
                        }
                        ui.label(text(Type::Caption, i18n::fmt("{n} silenced this session", &[("n", &n.to_string())])));
                    });
                });
            });
            ui.separator();
            ui.label(text(Type::Caption, i18n::fmt("{os} · settings stored in {place}", &[("os", crate::platform::OS.name()), ("place", &crate::platform::config_dir().map_or_else(|| tl!("memory only").to_string(), |d| craft_i18n::ltr(&d.display().to_string())))])));
        }));
    a11y::name_title_bar(&shown, title);
    if reset_confirms {
        app.skip_confirm.clear();
    }
    if p != app.prefs {
        app.set_prefs(ctx, p);
    }
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
        open = false;
    }
    app.show_prefs = open;
}

// ----- licences ---------------------------------------------------------------------------------

/// Licence texts compiled into the binary, so every copy of the app carries its notices.
const NOTICES: &[(&str, &str)] = &[
    ("NOTICE", include_str!("../NOTICE")),
    ("AniCraft: MIT License", include_str!("../LICENSE-MIT")),
    ("AniCraft: Apache License 2.0", include_str!("../LICENSE-APACHE")),
    ("Font: Ubuntu Light (Ubuntu Font Licence 1.0)", include_str!("../licenses/fonts/UFL-1.0-Ubuntu-font.txt")),
    ("Font: Hack (MIT, Bitstream Vera)", include_str!("../licenses/fonts/LICENSE-Hack.txt")),
    ("Font: Noto Emoji (SIL OFL 1.1)", include_str!("../licenses/fonts/OFL-1.1-NotoEmoji.txt")),
    ("Font: emoji-icon-font (MIT)", include_str!("../licenses/fonts/LICENSE-emoji-icon-font.txt")),
    ("Icons: Lucide (ISC)", include_str!("../licenses/icons/LICENSE-lucide.txt")),
    ("Rust crates: licence inventory", include_str!("../docs/legal/third-party-inventory.txt")),
];

fn licenses(app: &mut App, ctx: &egui::Context) {
    if !app.show_licenses {
        return;
    }
    let mut open = true;
    let shown = egui::Window::new(tl!("Licenses & Notices"))
        .open(&mut open)
        .collapsible(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .default_width(680.0)
        .default_height(560.0)
        .show(ctx, |ui| crate::theme::reading(ui, |ui| {
            ui.label(text(Type::Body, tl!("AniCraft is free software: MIT OR Apache-2.0, at your option.")).strong());
            ui.label(
                text(Type::Body, tl!("It is an independent project, not affiliated with or endorsed by Adobe. Bundled fonts and Rust crates keep their own permissive licences, listed below."))
                    .color(T.text_dim),
            );
            ui.add_space(6.0);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for (title, body) in NOTICES {
                    egui::CollapsingHeader::new(*title).default_open(*title == "NOTICE").show(ui, |ui| {
                        ui.add(egui::Label::new(text(Type::Path, *body)).wrap());
                    });
                }
            });
        }));
    a11y::name_title_bar(&shown, tl!("Licenses & Notices"));
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
        open = false;
    }
    app.show_licenses = open;
}

// ----- keyboard shortcuts (editor) ----------------------------------------------------------------

/// The shortcut editor is recording a key for `id`: replacing its key at `replace`, or adding.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeyCapture {
    pub id: &'static str,
    pub replace: Option<usize>,
}

/// Editor state between frames: the last message, a key waiting to be moved here from another
/// action, and the reset-all confirmation.
#[derive(Clone, Default)]
struct SheetState {
    filter: String,
    notice: Option<(String, bool)>,
    steal: Option<(KeyCapture, String, &'static str)>,
    confirm_reset_all: bool,
}

enum SheetAct {
    Capture(KeyCapture),
    Remove(&'static str, usize),
    Reset(&'static str),
    ResetAll,
    Steal,
    Dismiss,
}

/// Fixed keys: handled before the shortcut map, shown read-only.
fn fixed_rows() -> Vec<(&'static str, &'static str, Vec<&'static str>)> {
    let quit = if crate::platform::OS == crate::platform::Os::Mac { "Quit" } else { "Exit" };
    vec![
        ("Essentials", "Command palette", vec!["Ctrl+K"]),
        ("Essentials", "Keyboard shortcuts (this editor)", vec!["F1", "Ctrl+Alt+Shift+K"]),
        ("Essentials", "Play / Stop", vec!["Enter"]),
        ("Essentials", "Preferences", vec!["Ctrl+U", "Ctrl+,"]),
        ("Essentials", quit, vec!["Ctrl+Q"]),
        ("View", "Zoom in / out", vec!["Ctrl+=", "Ctrl+-"]),
        ("View", "Fit stage in window", vec!["Ctrl+0"]),
        ("View", "Actual size (100%)", vec!["Ctrl+1"]),
        ("View", "Zoom at the cursor", vec!["Ctrl+Wheel"]),
        ("View", "Pan with any tool", vec!["Space+Drag", "Middle-drag"]),
        ("Editing", "Nudge selection 1 px", vec!["Arrow keys"]),
        ("Editing", "Nudge selection 10 px", vec!["Shift+Arrow keys"]),
        ("Editing", "Add to / remove from selection", vec!["Shift+Click"]),
        ("Editing", "Constrain shape / 45° line", vec!["Shift+Drag"]),
        ("Editing", "Context actions", vec!["Right-click"]),
    ]
}

const GROUP_ORDER: [&str; 13] = ["Essentials", "Tools", "File", "Edit", "View", "Insert", "Modify", "Commands", "Control", "Window", "Help", "Other", "Editing"];

/// The action-name column: left-aligned, one row tall, cut off with an ellipsis when long.
/// One row of the shortcut sheet's grid, its cells given in reading order (name, keys, reset) and
/// added mirrored in a right-to-left language: egui's `Grid` only runs left to right.
type Cell<'a> = &'a mut dyn FnMut(&mut egui::Ui, &mut Vec<SheetAct>);
fn sheet_row(ui: &mut egui::Ui, acts: &mut Vec<SheetAct>, cells: [Cell<'_>; 3]) {
    let [a, b, c] = cells;
    if crate::theme::rtl() {
        c(ui, acts);
        b(ui, acts);
        a(ui, acts);
    } else {
        a(ui, acts);
        b(ui, acts);
        c(ui, acts);
    }
    ui.end_row();
}

fn name_cell(ui: &mut egui::Ui, label: &str) {
    // Right to left the name is the last column: it runs to the window's right edge, so the sheet
    // lines up at the side reading starts from.
    let w = if crate::theme::rtl() { ui.available_width().max(240.0) } else { 240.0 };
    ui.allocate_ui_with_layout(egui::vec2(w, crate::theme::TARGET), crate::theme::start_row(egui::Align::Center), |ui| {
        ui.set_min_width(w);
        ui.add(egui::Label::new(tl!(label)).truncate());
    });
}

fn action_label(km: &crate::keymap::Keymap, id: &str) -> &'static str {
    km.actions.iter().find(|a| a.id == id).map_or("this action", |a| a.label)
}

/// The next key press while recording, with every other key event taken out of the frame.
fn take_key_press(ctx: &egui::Context) -> Option<(egui::Modifiers, Key)> {
    ctx.input_mut(|i| {
        let mut got = None;
        i.events.retain(|e| match e {
            egui::Event::Key { key, pressed, modifiers, .. } => {
                if *pressed && got.is_none() {
                    got = Some((*modifiers, *key));
                }
                false
            }
            egui::Event::Text(_) => false,
            _ => true,
        });
        got
    })
}

fn shortcut_sheet(app: &mut App, ctx: &egui::Context) {
    if !app.show_shortcuts {
        app.key_capture = None;
        return;
    }
    let state_id = egui::Id::unique("shortcut_editor");
    let mut st: SheetState = ctx.data_mut(|d| d.get_temp(state_id).unwrap_or_default());
    let key = crate::platform::shortcut_label;

    // Recording: the next key press becomes the shortcut (Esc cancels).
    if let Some(cap) = app.key_capture
        && let Some((m, k)) = take_key_press(ctx)
    {
        app.key_capture = None;
        if k == Key::Escape && !m.any() {
            st.notice = None;
        } else {
            let chord = crate::keymap::chord(m, k);
            match crate::keymap::assign(&app.prefs.shortcuts, cap.id, cap.replace, &chord, false) {
                Ok(o) => {
                    st.notice = Some((format!("{}: {}", tl!(action_label(&app.keymap, cap.id)), key(&chord)), false));
                    let mut p = app.prefs.clone();
                    p.shortcuts = o;
                    app.set_prefs(ctx, p);
                }
                Err(crate::keymap::Refusal::Taken { label, .. }) => {
                    st.notice = Some((i18n::fmt("{key} is already used by {label}.", &[("key", &key(&chord)), ("label", tl!(label))]), true));
                    st.steal = Some((cap, chord, label));
                }
                Err(crate::keymap::Refusal::Reserved(why)) => st.notice = Some((i18n::fmt("{key} is kept for {why}; choose another key.", &[("key", &key(&chord)), ("why", tl!(why))]), true)),
                Err(crate::keymap::Refusal::TooMany) => st.notice = Some((i18n::fmt("An action can have up to {n} keys.", &[("n", &crate::keymap::MAX_KEYS.to_string())]), true)),
            }
        }
    }

    let mut open = true;
    let mut acts: Vec<SheetAct> = Vec::new();
    let capture = app.key_capture;
    let changed = app.prefs.shortcuts.len();
    let shown = egui::Window::new(tl!("Keyboard Shortcuts"))
        .open(&mut open)
        .collapsible(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .default_width(660.0)
        .default_height(600.0)
        .show(ctx, |ui| crate::theme::reading(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut st.filter).hint_text(tl!("Filter by name or key…")).desired_width(ui.available_width() - 120.0)).set_name(tl!("Filter shortcuts"));
                let reset_all = ui.add_enabled(changed > 0, egui::Button::new(tl!("Reset all…"))).on_hover_text(tl!("Put every shortcut back to the defaults"));
                if reset_all.clicked() {
                    st.confirm_reset_all = true;
                }
            });
            // One message line: what is happening, what just happened, or what needs a decision.
            if let Some(cap) = capture {
                ui.label(text(Type::Label, i18n::fmt("Press the new keys for {action}. Esc cancels.", &[("action", tl!(action_label(&app.keymap, cap.id)))])).color(T.accent_text));
            } else if st.confirm_reset_all {
                ui.horizontal(|ui| {
                    ui.label(text(Type::Label, i18n::trn(changed as u64, "Reset {n} changed shortcut to the defaults?", "Reset {n} changed shortcuts to the defaults?")));
                    if ui.add(danger_button(tl!("Reset all"))).clicked() {
                        acts.push(SheetAct::ResetAll);
                    }
                    if ui.button(tl!("Cancel")).clicked() {
                        acts.push(SheetAct::Dismiss);
                    }
                });
            } else if let Some((msg, err)) = &st.notice {
                ui.horizontal(|ui| {
                    ui.label(text(Type::Label, msg.clone()).color(if *err { T.error } else { T.ok }));
                    if let Some((cap, _, _)) = &st.steal {
                        if ui.button(i18n::fmt("Use it for {action}", &[("action", tl!(action_label(&app.keymap, cap.id)))])).clicked() {
                            acts.push(SheetAct::Steal);
                        }
                        if ui.button(tl!("Cancel")).clicked() {
                            acts.push(SheetAct::Dismiss);
                        }
                    }
                });
            } else {
                ui.label(text(Type::Caption, tl!("Click a key to change it or + to add one; right-click a key to remove it. Changes save as you go.")));
            }
            ui.add_space(4.0);

            let f = st.filter.to_lowercase();
            let hit = |s: &str| f.is_empty() || s.to_lowercase().contains(&f);
            let fixed = fixed_rows();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for group in GROUP_ORDER {
                    let actions: Vec<&crate::keymap::Action> = app
                        .keymap
                        .actions
                        .iter()
                        .filter(|a| a.group == group || (group == "Other" && !GROUP_ORDER.contains(&a.group)))
                        // The names as shown (translated) and in English, and the keys.
                        .filter(|a| hit(a.label) || hit(tl!(a.label)) || hit(group) || hit(tl!(group)) || app.keymap.keys(a.id).iter().any(|k| hit(k) || hit(&key(k))))
                        .collect();
                    let fixed: Vec<_> = fixed.iter().filter(|(g, l, ks)| *g == group && (hit(l) || hit(tl!(l)) || hit(g) || hit(tl!(g)) || ks.iter().any(|k| hit(k)))).collect();
                    if actions.is_empty() && fixed.is_empty() {
                        continue;
                    }
                    let count = (actions.len() + fixed.len()).to_string();
                    crate::theme::section(ui, &format!("sheet_{group}"), crate::theme::Section::Rule, tl!(group), Some(&count), |ui| {
                        crate::theme::ltr(ui, |ui| egui::Grid::new(("sc", group)).num_columns(3).striped(true).min_col_width(60.0).show(ui, |ui| {
                            for (_, label, keys) in &fixed {
                                sheet_row(ui, &mut acts, [
                                    &mut |ui, _| name_cell(ui, label),
                                    &mut |ui, _| {
                                        ui.horizontal(|ui| {
                                            ui.label(text(Type::Key, keys.iter().map(|k| key(k)).collect::<Vec<_>>().join(" · ")).color(T.text_dim));
                                            ui.add_space(6.0);
                                            ui.label(text(Type::Caption, tl!("fixed")));
                                        });
                                    },
                                    &mut |ui, _| {
                                        // An empty cell (an empty label would be an empty, clickable node for screen readers).
                                        ui.allocate_space(egui::Vec2::ZERO);
                                    },
                                ]);
                            }
                            for a in actions {
                                sheet_row(ui, &mut acts, [
                                    &mut |ui, _| name_cell(ui, a.label),
                                    &mut |ui, acts| {
                                        ui.horizontal(|ui| {
                                            let keys = app.keymap.keys(a.id);
                                            for (i, k) in keys.iter().enumerate() {
                                                let recording = capture == Some(KeyCapture { id: a.id, replace: Some(i) });
                                                let chip = if recording {
                                                    egui::Button::new(text(Type::Key, tl!("Press keys…")).color(T.on_fill)).fill(T.accent)
                                                } else {
                                                    egui::Button::new(text(Type::Key, key(k)).color(T.text))
                                                };
                                                let r = ui.add(chip).named(&i18n::fmt("{action} shortcut {key}", &[("action", tl!(a.label)), ("key", &key(k))])).on_hover_text(tl!("Click to change · right-click to remove"));
                                                if r.clicked() {
                                                    acts.push(SheetAct::Capture(KeyCapture { id: a.id, replace: Some(i) }));
                                                }
                                                r.context_menu(|ui| {
                                                    if ui.button(tl!("Change…")).clicked() {
                                                        acts.push(SheetAct::Capture(KeyCapture { id: a.id, replace: Some(i) }));
                                                        ui.close();
                                                    }
                                                    if ui.button(tl!("Remove")).clicked() {
                                                        acts.push(SheetAct::Remove(a.id, i));
                                                        ui.close();
                                                    }
                                                });
                                            }
                                            if capture == Some(KeyCapture { id: a.id, replace: None }) {
                                                ui.add(egui::Button::new(text(Type::Key, tl!("Press keys…")).color(T.on_fill)).fill(T.accent));
                                            } else if keys.len() < crate::keymap::MAX_KEYS {
                                                let tip = tl!(if keys.is_empty() { "Add a shortcut" } else { "Add another shortcut" });
                                                let name = i18n::fmt(if keys.is_empty() { "Add a shortcut for {action}" } else { "Add another shortcut for {action}" }, &[("action", tl!(a.label))]);
                                                if ui.add(egui::Button::new("+").min_size(egui::vec2(crate::theme::TARGET, crate::theme::TARGET))).named(&name).on_hover_text(tip).clicked() {
                                                    acts.push(SheetAct::Capture(KeyCapture { id: a.id, replace: None }));
                                                }
                                            }
                                        });
                                    },
                                    &mut |ui, acts| {
                                        if app.keymap.is_default(a.id) {
                                            // An empty cell (an empty label would be an empty, clickable node for screen readers).
                                            ui.allocate_space(egui::Vec2::ZERO);
                                        } else if ui.button(tl!("Reset")).named(&i18n::fmt("Reset {action}", &[("action", tl!(a.label))])).on_hover_text(i18n::fmt("Back to the default ({keys})", &[("keys", &if a.defaults.is_empty() { tl!("none").to_string() } else { a.defaults.iter().map(|k| key(k)).collect::<Vec<_>>().join(" · ") })])).clicked() {
                                            acts.push(SheetAct::Reset(a.id));
                                        }
                                    },
                                ]);
                            }
                        }));
                    });
                }
            });
        }));
    a11y::name_title_bar(&shown, tl!("Keyboard Shortcuts"));

    for act in acts {
        let mut p = app.prefs.clone();
        match act {
            SheetAct::Capture(c) => {
                app.key_capture = Some(c);
                st.notice = None;
                st.steal = None;
                st.confirm_reset_all = false;
                continue;
            }
            SheetAct::Remove(id, i) => {
                p.shortcuts = crate::keymap::remove(&p.shortcuts, id, i);
                st.notice = Some((i18n::fmt("{action}: key removed", &[("action", tl!(action_label(&app.keymap, id)))]), false));
            }
            SheetAct::Reset(id) => {
                p.shortcuts = crate::keymap::reset(&p.shortcuts, id);
                st.notice = Some((i18n::fmt("{action}: back to the default", &[("action", tl!(action_label(&app.keymap, id)))]), false));
            }
            SheetAct::ResetAll => {
                p.shortcuts.clear();
                st.confirm_reset_all = false;
                st.notice = Some((tl!("Every shortcut is back to the defaults.").into(), false));
            }
            SheetAct::Steal => {
                let Some((cap, chord, from)) = st.steal.take() else { continue };
                match crate::keymap::assign(&p.shortcuts, cap.id, cap.replace, &chord, true) {
                    Ok(o) => {
                        p.shortcuts = o;
                        let action = tl!(action_label(&app.keymap, cap.id));
                        st.notice = Some((i18n::fmt("{action}: {key} (removed from {from})", &[("action", action), ("key", &key(&chord)), ("from", tl!(from))]), false));
                    }
                    Err(_) => st.notice = Some((i18n::fmt("{key} could not be moved.", &[("key", &key(&chord))]), true)),
                }
            }
            SheetAct::Dismiss => {
                st.steal = None;
                st.notice = None;
                st.confirm_reset_all = false;
                continue;
            }
        }
        app.set_prefs(ctx, p);
    }
    if app.key_capture.is_none() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
        open = false;
    }
    if !open {
        app.key_capture = None;
        st.steal = None;
        st.confirm_reset_all = false;
    }
    ctx.data_mut(|d| d.insert_temp(state_id, st));
    app.show_shortcuts = open;
}
