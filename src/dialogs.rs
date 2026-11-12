//! Modal and floating dialogs: destructive-action confirmation (ADR 0004), the unsaved-changes
//! guard on close, the command palette (Ctrl+K) and the keyboard-shortcut sheet (F1).

use crate::app::{App, Tool};
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

fn confirm_modal(app: &mut App, ctx: &egui::Context) {
    let Some(p) = &mut app.confirm else { return };
    let mut decision: Option<bool> = None;
    let irreversible = p.conf.irreversible;
    egui::Modal::new(egui::Id::new("confirm")).show(ctx, |ui| {
        ui.set_width(420.0);
        ui.add_space(4.0);
        // The engine's English, translated when a catalog has that exact text.
        ui.label(text(Type::Title, crate::i18n::t_dyn(&p.conf.title)));
        ui.add_space(6.0);
        ui.label(text(Type::Body, crate::i18n::t_dyn(&p.conf.message)).color(T.text_dim));
        ui.add_space(4.0);
        if irreversible {
            ui.label(text(Type::Caption, tr!("This cannot be undone.")).color(T.error));
        } else {
            crate::theme::checkbox(ui, &mut p.dont_ask, text(Type::Caption, tr!("Don't ask again this session")).color(T.text_dim));
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.add(egui::Button::new(tr!("Cancel")).min_size(egui::vec2(96.0, crate::theme::TARGET + 4.0))).clicked() {
                decision = Some(false);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(danger_button(&crate::i18n::t_dyn(&p.conf.action))).clicked() {
                    decision = Some(true);
                }
            });
        });
        ui.add_space(2.0);
        let hint = if irreversible { tr!("Esc cancels · click the red button to proceed") } else { tr!("Enter confirms · Esc cancels") };
        ui.label(text(Type::Caption, hint));
    });
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
            app.status_ok(tr!("Cancelled"));
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
    let name = app.s.path.as_ref().and_then(|p| p.file_name()).map_or_else(|| tr!("Untitled").to_string(), |n| n.to_string_lossy().into_owned());
    egui::Modal::new(egui::Id::new("close_guard")).show(ctx, |ui| {
        ui.set_width(440.0);
        ui.label(text(Type::Title, tr!("Save changes to “{name}” before closing?", name = name)));
        ui.add_space(6.0);
        ui.label(text(Type::Body, tr!("If you don't save, your changes since the last save will be lost.")).color(T.text_dim));
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.add(danger_button(tr!("Don't Save"))).clicked() {
                choice = Some(Choice::Discard);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(egui::Button::new(text(Type::Body, tr!("Save")).strong().color(T.on_fill)).fill(T.accent).min_size(egui::vec2(96.0, crate::theme::TARGET + 4.0))).clicked() {
                    choice = Some(Choice::Save);
                }
                if ui.add(egui::Button::new(tr!("Cancel")).min_size(egui::vec2(96.0, crate::theme::TARGET + 4.0))).clicked() {
                    choice = Some(Choice::Cancel);
                }
            });
        });
        ui.label(text(Type::Caption, tr!("Enter saves · Esc cancels")));
    });
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
    /// The menu path to the entry, in the interface language (`local`) or in English, so
    /// either finds it.
    fn title(&self, local: bool) -> String {
        let t = |s: &'static str| if local { crate::i18n::t(s) } else { s };
        let path = |parts: &[&'static str], last: &str| {
            let mut v: Vec<&str> = parts.iter().map(|p| t(p)).collect();
            v.push(last);
            v.join(" › ")
        };
        match self {
            Entry::Command(c) => {
                let label = if local { crate::i18n::t_cmd(c.id, c.label) } else { c.label };
                path(&c.menu.split('/').filter(|m| !m.is_empty()).collect::<Vec<_>>(), label)
            }
            Entry::Tool(tool) => path(&[tr_noop!("Tool")], t(tool.label())),
            Entry::View(name, _, _) => path(&[tr_noop!("View")], t(name)),
            // The plug-in's own name: not ours to translate.
            Entry::Plugin(_, label) => path(&[tr_noop!("Commands"), tr_noop!("Plug-ins")], label),
        }
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
    if matches!(c.id, "file.open" | "file.recovery_list" | "file.save_as" | "file.import" | "file.export_png" | "file.export_gif" | "file.export_apng" | "file.export_webp" | "file.export_svg" | "file.export_sprite_sheet" | "file.export_video" | "file.export_lottie" | "file.export_html" | "file.export_png_sequence" | "modify.convert_to_symbol" | "layer.rename" | "scene.rename" | "plugin.install") {
        return true;
    }
    let inner = c.params.trim().trim_start_matches('{').trim_end_matches('}');
    inner.split(',').map(str::trim).filter(|k| !k.is_empty()).all(|k| k.split(':').next().is_some_and(|name| name.trim().ends_with('?')))
}

fn entries() -> Vec<Entry> {
    let mut v: Vec<Entry> = commands().into_iter().filter(runnable).map(Entry::Command).collect();
    v.extend(Tool::ALL.iter().map(|t| Entry::Tool(*t)));
    // English keys, translated where they are drawn (`Entry::title`).
    v.push(Entry::View(tr_noop!("Play / Stop"), "Enter", App::toggle_play));
    v.push(Entry::View(tr_noop!("Onion Skin"), "", crate::onion::toggle_ghosts));
    v.push(Entry::View(tr_noop!("Preview Through Camera"), "", App::toggle_camera_preview));
    v.push(Entry::View(tr_noop!("Fit Stage in Window"), "Ctrl+0", |a| a.view.fit = true));
    v.push(Entry::View(tr_noop!("Keyboard Shortcuts"), "F1", |a| a.show_shortcuts = true));
    v.push(Entry::View(tr_noop!("User Guide"), "", App::open_user_guide));
    v.push(Entry::View(tr_noop!("Project Files & Docs"), "", |a| a.show_files = true));
    v.push(Entry::View(tr_noop!("Licenses & Notices"), "", |a| a.show_licenses = true));
    v.push(Entry::View(tr_noop!("Preferences"), "Ctrl+U", App::open_prefs));
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
    // Found by its name in the interface language or in English, whichever matches better.
    let mut matches: Vec<(i32, Entry, String)> =
        all.into_iter().filter_map(|e| {
            let key = e.shortcut(km);
            let local = fuzzy(&p.query, &format!("{} {key}", e.title(true)));
            let english = if crate::i18n::code() == crate::i18n::ENGLISH { None } else { fuzzy(&p.query, &format!("{} {key}", e.title(false))) };
            local.max(english).map(|s| (s, e, key))
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
    egui::Area::new(egui::Id::new("palette")).order(egui::Order::Foreground).anchor(egui::Align2::CENTER_TOP, [0.0, 70.0]).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).fill(T.panel_alt).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
            ui.set_width(560.0);
            let r = ui.add(egui::TextEdit::singleline(&mut p.query).hint_text(tr!("Type a command, tool or setting…")).desired_width(f32::INFINITY).font(egui::TextStyle::Heading)).named(tr!("Command palette"));
            r.request_focus();
            if r.changed() {
                p.selected = 0;
            }
            ui.add_space(6.0);
            if matches.is_empty() {
                ui.label(text(Type::Caption, tr!("No matching commands")));
            }
            for (i, (_, e, key)) in matches.iter().enumerate() {
                let sel = i == selected;
                let resp = ui
                    .horizontal(|ui| {
                        let lab = ui.selectable_label(sel, e.title(true));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(text(Type::Key, crate::platform::shortcut_label(key)));
                        });
                        lab
                    })
                    .inner;
                if resp.clicked() {
                    run = Some(e.clone());
                }
            }
            ui.add_space(4.0);
            ui.label(text(Type::Caption, tr!("Up/Down choose · Enter run · Esc close")));
        });
    });
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
    let filter_id = egui::Id::new(("font_filter", id));
    let shown = match current {
        FontChoice::Bundled => tr!("Default ({font})", font = bundled),
        FontChoice::System(name) => name.clone(),
    };
    egui::ComboBox::from_id_salt(id).width(260.0).selected_text(text(Type::Body, shown)).show_ui(ui, |ui| {
        if ui.selectable_label(*current == FontChoice::Bundled, text(Type::Body, tr!("Default (bundled: {font})", font = bundled))).clicked() {
            picked = Some(FontChoice::Bundled);
        }
        ui.separator();
        match families {
            None => {
                ui.label(text(Type::Caption, tr!("Scanning installed fonts…")));
            }
            Some(list) => {
                let mut filter: String = ui.ctx().data_mut(|d| d.get_temp(filter_id).unwrap_or_default());
                ui.add(egui::TextEdit::singleline(&mut filter).hint_text(tr!("Filter fonts…")).desired_width(240.0)).set_name(tr!("Filter fonts"));
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
    let title = if crate::platform::OS == crate::platform::Os::Mac { tr!("Settings") } else { tr!("Preferences") };
    let shown = egui::Window::new(title)
        // An id of its own: the window keeps its place when the language changes.
        .id(egui::Id::new("prefs_window"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .default_width(560.0)
        .show(ctx, |ui| {
            // Taller than small windows: the sections scroll, the footer stays.
            let max_h = (ctx.content_rect().height() - 160.0).max(200.0);
            egui::ScrollArea::vertical().max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
                // A two-column form: labels in a fixed left column, controls and the notes that
                // explain them in the right one.
                use crate::theme::{FORM_LABEL_DIALOG as LW, Section, form_note, form_row, section};
                // Under a radio button or checkbox, notes line up with its text, not its box.
                // (`form_note` adds one item spacing after its indent, like a row's label column.)
                let toggle_indent = (ui.spacing().icon_width + ui.spacing().icon_spacing - ui.spacing().item_spacing.x).max(0.0);

                section(ui, "prefs_appearance", Section::Rule, tr!("Appearance"), None, |ui| {
                    // The interface language (ADR 0040): first, so someone who can't read the
                    // current one finds it at the top. Each language is named in itself.
                    form_row(ui, LW, tr!("Language"), |ui| {
                        let auto = crate::i18n::AUTO;
                        let current = if p.language == auto { tr!("System language ({name})", name = crate::i18n::system_choice()) } else { crate::i18n::name_of(&p.language).into_owned() };
                        egui::ComboBox::from_id_salt("pref_language").width(260.0).selected_text(text(Type::Body, current)).show_ui(ui, |ui| {
                            ui.selectable_value(&mut p.language, auto.to_string(), text(Type::Body, tr!("System language ({name})", name = crate::i18n::system_choice())));
                            ui.separator();
                            for (code, name) in crate::i18n::offered() {
                                ui.selectable_value(&mut p.language, code.to_string(), text(Type::Body, name));
                            }
                        })
                        .response
                        .set_name(tr!("Interface language"));
                    });
                    let note = if crate::i18n::offered().len() <= 2 {
                        tr!("This build has the interface in English only. Builds with translations list their languages here.")
                    } else {
                        tr!("Menus, panels and dialogs. Your documents, layer names and the command line stay as they are.")
                    };
                    form_note(ui, LW, text(Type::Caption, note));
                    ui.add_space(4.0);
                    form_row(ui, LW, tr!("Theme"), |ui| {
                        ui.vertical(|ui| {
                            // Follow system (ADR 0064), then each palette by hand.
                            let was = if p.follows_system() { None } else { Some(p.theme) };
                            let mut choice = was;
                            crate::theme::radio_value(ui, &mut choice, None, text(Type::Body, tr!("Follow system")));
                            let says = match app.system_contrast {
                                Some(true) => tr!("High contrast while the system asks for more contrast (it does now), graphite otherwise."),
                                Some(false) => tr!("High contrast while the system asks for more contrast (it doesn't now), graphite otherwise."),
                                None => tr!("High contrast while the system asks for more contrast, graphite otherwise. This system doesn't say, so it is graphite."),
                            };
                            form_note(ui, toggle_indent, text(Type::Caption, says));
                            ui.add_space(2.0);
                            for (palette, name, tip) in crate::theme::Palette::ALL {
                                crate::theme::radio_value(ui, &mut choice, Some(palette), text(Type::Body, crate::i18n::t(name)));
                                form_note(ui, toggle_indent, text(Type::Caption, crate::i18n::t(tip)));
                                ui.add_space(2.0);
                            }
                            if choice != was {
                                p.theme_follows_system = Some(choice.is_none());
                                if let Some(t) = choice {
                                    p.theme = t;
                                }
                            }
                        });
                    });
                    ui.add_space(4.0);
                    form_row(ui, LW, tr!("Interface font"), |ui| {
                        if let Some(c) = font_picker(ui, "pref_ui_font", tr!("Interface font"), &p.ui_font, sys.as_ref().map(|s| s.families.as_slice()), bundled_ui) {
                            p.ui_font = c;
                        }
                    });
                    form_row(ui, LW, tr!("Data font"), |ui| {
                        let list = sys.as_ref().map(|s| s.monospace.as_slice());
                        if let Some(c) = font_picker(ui, "pref_data_font", tr!("Data font"), &p.data_font, list, bundled_mono) {
                            p.data_font = c;
                        }
                    });
                    form_note(ui, LW, text(Type::Caption, tr!("The data font shows numbers, timecode, rulers and shortcuts; only monospaced families are listed so digits stay aligned.")));
                    ui.add_space(4.0);
                    form_row(ui, LW, tr!("Preview"), |ui| {
                        ui.vertical(|ui| {
                            ui.label(text(Type::Body, tr!("Layer names, menus and dialog text read like this.")));
                            ui.label(text(Type::Data, tr!("Frame {frame} · {seconds} s · X {x} Y {y} · {keys}", frame = "0042", seconds = "1.75", x = "1280.5", y = "720.0", keys = crate::platform::shortcut_label("Ctrl+Shift+S"))));
                        });
                    });
                    ui.add_space(4.0);
                    form_row(ui, LW, tr!("Interface scale"), |ui| {
                        let mut pct = p.ui_scale * 100.0;
                        let r = with_type(ui, Type::Data, |ui| {
                            a11y::slider(ui, &mut pct, (crate::prefs::SCALE_RANGE.start() * 100.0)..=(crate::prefs::SCALE_RANGE.end() * 100.0), tr!("Interface scale"), a11y::SliderLook { suffix: " %", step: Some(5.0), max_decimals: Some(0), ..Default::default() })
                        });
                        // Apply on release so the slider doesn't move under the pointer while dragging.
                        if r.drag_stopped() || (r.changed() && !r.dragged()) {
                            p.ui_scale = pct / 100.0;
                        }
                    });
                });

                section(ui, "prefs_stage", Section::Rule, tr!("Stage"), None, |ui| {
                    form_row(ui, LW, tr!("Rendering"), |ui| {
                        ui.vertical(|ui| {
                            for (mode, name, tip) in RenderMode::ALL {
                                crate::theme::radio_value(ui, &mut p.render_mode, mode, text(Type::Body, crate::i18n::t(name)));
                                form_note(ui, toggle_indent, text(Type::Caption, crate::i18n::t(tip)));
                                ui.add_space(2.0);
                            }
                        });
                    });
                    if crate::gpu_stage::AVAILABLE {
                        ui.add_space(4.0);
                        form_row(ui, LW, tr!("Renderer"), |ui| {
                            ui.vertical(|ui| {
                                for (choice, name, tip) in StageRenderer::ALL {
                                    crate::theme::radio_value(ui, &mut p.stage_renderer, choice, text(Type::Body, crate::i18n::t(name)));
                                    form_note(ui, toggle_indent, text(Type::Caption, crate::i18n::t(tip)));
                                    ui.add_space(2.0);
                                }
                            });
                        });
                        use crate::gpu_stage::State;
                        let status = match app.stage_cache.gpu.state() {
                            State::Off => None,
                            State::Starting => Some(tr!("Starting the graphics card…").to_string()),
                            State::Running { adapter, direct: true } => Some(tr!("Drawing on {adapter}, straight into the window.", adapter = adapter)),
                            State::Running { adapter, direct: false } => Some(tr!("Drawing on {adapter}, then copied to the window (slower: the window's graphics device can't be shared).", adapter = adapter)),
                            State::Failed(why) => Some(tr!("The graphics card can't draw the stage, so the CPU does: {why}", why = why)),
                        };
                        if crate::gpu_stage::wanted(p.stage_renderer) != p.stage_renderer {
                            form_note(ui, LW, text(Type::Caption, tr!("ANICRAFT_STAGE_RENDERER is set, and it takes precedence over this choice.")));
                        }
                        if let Some(s) = status {
                            form_note(ui, LW, text(Type::Caption, s));
                        }
                    }
                });

                section(ui, "prefs_timeline", Section::Rule, tr!("Timeline"), None, |ui| {
                    form_row(ui, LW, tr!("Extra start gap"), |ui| {
                        with_type(ui, Type::Data, |ui| {
                            ui.add(egui::DragValue::new(&mut p.timeline_start_gap).range(crate::prefs::GAP_RANGE).speed(0.5).max_decimals(0).suffix(" pt")).set_name(tr!("Extra start gap"));
                        });
                    });
                    form_note(ui, LW, text(Type::Caption, tr!("The timeline always leaves room before frame 1 for the playhead; this adds more. It stays the same at every zoom.")));
                });

                // Test Movie in a window of its own, or inside the editor (ADR 0062).
                section(ui, "prefs_test_movie", Section::Rule, tr!("Test Movie"), None, |ui| {
                    form_row(ui, LW, tr!("Window"), |ui| {
                        ui.vertical(|ui| {
                            crate::theme::checkbox(ui, &mut p.test_movie_window, text(Type::Body, tr!("Play Test Movie in a window of its own")));
                            let note = match crate::platform::movie_host(ui.ctx().embed_viewports(), true) {
                                crate::platform::MovieHost::Inside(why) => why.note().map(crate::i18n::t),
                                crate::platform::MovieHost::Window if !crate::platform::window_positions() => Some(tr!("Off: it floats inside the editor. On Wayland the desktop places the window, so it can't open where it was last.")),
                                crate::platform::MovieHost::Window => Some(tr!("Off: it floats inside the editor. The window opens where it was last.")),
                            };
                            if let Some(note) = note {
                                form_note(ui, toggle_indent, text(Type::Caption, note));
                            }
                        });
                    });
                });

                section(ui, "prefs_files", Section::Rule, tr!("Files"), None, |ui| {
                    form_row(ui, LW, tr!("File dialogs"), |ui| {
                        ui.vertical(|ui| {
                            crate::theme::checkbox(ui, &mut p.native_dialogs, text(Type::Body, tr!("Use the system's open and save dialogs")));
                            form_note(ui, toggle_indent, text(Type::Caption, tr!("Turn off on Linux desktops without an XDG desktop portal; AniCraft then asks for a path instead.")));
                        });
                    });
                    ui.add_space(4.0);
                    form_row(ui, LW, tr!("Autosave"), |ui| {
                        ui.vertical(|ui| {
                            crate::theme::checkbox(ui, &mut p.autosave, text(Type::Body, tr!("Keep a recovery copy of unsaved work")));
                            form_note(
                                ui,
                                toggle_indent,
                                text(Type::Caption, tr!("If AniCraft closes unexpectedly, the next start offers to restore it. Copies stay in the settings folder, never over your files, and go away when you save.")),
                            );
                        });
                    });
                    ui.add_enabled_ui(p.autosave, |ui| {
                        form_row(ui, LW, tr!("Every"), |ui| {
                            let shown = crate::prefs::AUTOSAVE_INTERVALS.iter().find(|(s, _)| *s == p.autosave_secs).map_or_else(|| trn!("{n} second", "{n} seconds", p.autosave_secs), |(_, l)| crate::i18n::t(l).to_string());
                            egui::ComboBox::from_id_salt("pref_autosave_secs").width(160.0).selected_text(text(Type::Body, shown)).show_ui(ui, |ui| {
                                for (secs, label) in crate::prefs::AUTOSAVE_INTERVALS {
                                    ui.selectable_value(&mut p.autosave_secs, secs, text(Type::Body, crate::i18n::t(label)));
                                }
                            })
                            .response
                            .set_name(tr!("Autosave every"));
                        });
                        form_row(ui, LW, tr!("Or after"), |ui| {
                            with_type(ui, Type::Data, |ui| {
                                ui.add(egui::DragValue::new(&mut p.autosave_edits).range(crate::prefs::AUTOSAVE_EDITS_RANGE).speed(0.5).suffix(format!(" {}", tr!("changes")))).set_name(tr!("Autosave after this many changes"));
                            });
                        });
                        form_note(ui, LW, text(Type::Caption, tr!("Whichever comes first. 0 changes: by time only. Copies are written in the background, so drawing never waits.")));
                    });
                });

                section(ui, "prefs_safety", Section::Rule, tr!("Safety"), None, |ui| {
                    form_row(ui, LW, tr!("Confirmations"), |ui| {
                        let n = app.skip_confirm.len();
                        if ui.add_enabled(n > 0, egui::Button::new(text(Type::Body, tr!("Show all again")))).clicked() {
                            reset_confirms = true;
                        }
                        ui.label(text(Type::Caption, trn!("{n} silenced this session", "{n} silenced this session", n)));
                    });
                });

            });
            ui.separator();
            ui.label(text(Type::Caption, tr!("{os} · settings stored in {place}", os = crate::platform::OS.name(), place = crate::platform::settings_place())));
        });
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
/// Titles are English keys, translated where they are drawn; the licence texts stay as written.
const NOTICES: &[(&str, &str)] = &[
    (tr_noop!("NOTICE"), include_str!("../NOTICE")),
    (tr_noop!("AniCraft: MIT License"), include_str!("../LICENSE-MIT")),
    (tr_noop!("AniCraft: Apache License 2.0"), include_str!("../LICENSE-APACHE")),
    (tr_noop!("Font: Ubuntu Light (Ubuntu Font Licence 1.0)"), include_str!("../licenses/fonts/UFL-1.0-Ubuntu-font.txt")),
    (tr_noop!("Font: Hack (MIT, Bitstream Vera)"), include_str!("../licenses/fonts/LICENSE-Hack.txt")),
    (tr_noop!("Font: Noto Emoji (SIL OFL 1.1)"), include_str!("../licenses/fonts/OFL-1.1-NotoEmoji.txt")),
    (tr_noop!("Font: emoji-icon-font (MIT)"), include_str!("../licenses/fonts/LICENSE-emoji-icon-font.txt")),
    (tr_noop!("Icons: Lucide (ISC)"), include_str!("../licenses/icons/LICENSE-lucide.txt")),
    (tr_noop!("Rust crates: licence inventory"), include_str!("../docs/legal/third-party-inventory.txt")),
];

fn licenses(app: &mut App, ctx: &egui::Context) {
    if !app.show_licenses {
        return;
    }
    let mut open = true;
    let title = tr!("Licenses & Notices");
    let shown = egui::Window::new(title)
        .id(egui::Id::new("licenses_window"))
        .open(&mut open)
        .collapsible(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .default_width(680.0)
        .default_height(560.0)
        .show(ctx, |ui| {
            ui.label(text(Type::Body, tr!("AniCraft is free software: MIT OR Apache-2.0, at your option.")).strong());
            ui.label(
                text(Type::Body, tr!("It is an independent project, not affiliated with or endorsed by Adobe. Bundled fonts and Rust crates keep their own permissive licences, listed below."))
                    .color(T.text_dim),
            );
            ui.add_space(6.0);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for (title, body) in NOTICES {
                    egui::CollapsingHeader::new(crate::i18n::t(title)).id_salt(("notice", *title)).default_open(*title == "NOTICE").show(ui, |ui| {
                        ui.add(egui::Label::new(text(Type::Path, *body)).wrap());
                    });
                }
            });
        });
    a11y::name_title_bar(&shown, title);
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
    /// The capture, the chord, and the id of the action that has it now.
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

/// Fixed keys: handled before the shortcut map, shown read-only. Groups and names are English
/// keys (groups match [`GROUP_ORDER`]), translated where they are drawn.
pub(crate) fn fixed_rows() -> Vec<(&'static str, &'static str, Vec<&'static str>)> {
    let quit = if crate::platform::OS == crate::platform::Os::Mac { tr_noop!("Quit") } else { tr_noop!("Exit") };
    let (essentials, view, editing) = (tr_noop!("Essentials"), tr_noop!("View"), tr_noop!("Editing"));
    // Keys in registry notation; the words among them are translated by `fixed_key`.
    vec![
        (essentials, tr_noop!("Command palette"), vec!["Ctrl+K"]),
        (essentials, tr_noop!("Keyboard shortcuts (this editor)"), vec!["F1", "Ctrl+Alt+Shift+K"]),
        (essentials, tr_noop!("Play / Stop"), vec!["Enter"]),
        (essentials, tr_noop!("Preferences"), vec!["Ctrl+U", "Ctrl+,"]),
        (essentials, quit, vec!["Ctrl+Q"]),
        (view, tr_noop!("Zoom in / out"), vec!["Ctrl+=", "Ctrl+-"]),
        (view, tr_noop!("Fit stage in window"), vec!["Ctrl+0"]),
        (view, tr_noop!("Actual size (100%)"), vec!["Ctrl+1"]),
        (view, tr_noop!("Zoom at the cursor"), vec!["Ctrl+Wheel"]),
        (view, tr_noop!("Pan with any tool"), vec!["Space+Drag", "Middle-drag"]), // i18n-exempt: keys, see fixed_key
        (editing, tr_noop!("Nudge selection 1 px"), vec!["Arrow keys"]), // i18n-exempt: keys, see fixed_key
        (editing, tr_noop!("Nudge selection 10 px"), vec!["Shift+Arrow keys"]),
        (editing, tr_noop!("Add to / remove from selection"), vec!["Shift+Click"]),
        (editing, tr_noop!("Constrain shape / 45° line"), vec!["Shift+Drag"]),
        (editing, tr_noop!("Context actions"), vec!["Right-click"]), // i18n-exempt: keys, see fixed_key
    ]
}

/// A fixed key as shown: the platform's modifier names, and the pointer gestures and key
/// groups that are words (not key names) in the interface language.
fn fixed_key(k: &str) -> String {
    let (mods, last) = k.rsplit_once('+').unwrap_or(("", k));
    let word = match last {
        "Wheel" => tr!("Wheel"),
        "Drag" => tr!("Drag"),
        "Middle-drag" => tr!("Middle-drag"),
        "Arrow keys" => tr!("Arrow keys"),
        "Click" => tr!("Click"),
        "Right-click" => tr!("Right-click"),
        other => other,
    };
    crate::platform::shortcut_label(&if mods.is_empty() { word.to_string() } else { format!("{mods}+{word}") })
}

/// Section order, by group key (English; the titles are translated where they are drawn).
pub(crate) const GROUP_ORDER: [&str; 13] = [
    tr_noop!("Essentials"),
    tr_noop!("Tools"),
    tr_noop!("File"),
    tr_noop!("Edit"),
    tr_noop!("View"),
    tr_noop!("Insert"),
    tr_noop!("Modify"),
    tr_noop!("Commands"),
    tr_noop!("Control"),
    tr_noop!("Window"),
    tr_noop!("Help"),
    tr_noop!("Other"),
    tr_noop!("Editing"),
];

/// The action-name column: left-aligned, one row tall, cut off with an ellipsis when long.
fn name_cell(ui: &mut egui::Ui, label: &str) {
    ui.allocate_ui_with_layout(egui::vec2(240.0, crate::theme::TARGET), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_width(240.0);
        ui.add(egui::Label::new(label).truncate());
    });
}

/// An action's name in the interface language (a command's as in the menus).
fn action_name(a: &crate::keymap::Action) -> &'static str {
    crate::i18n::t_cmd(a.id, a.label)
}

fn action_label(km: &crate::keymap::Keymap, id: &str) -> &'static str {
    km.actions.iter().find(|a| a.id == id).map_or_else(|| tr!("this action"), action_name)
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
    let state_id = egui::Id::new("shortcut_editor");
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
                    st.notice = Some((tr!("{action}: {key}", action = action_label(&app.keymap, cap.id), key = key(&chord)), false));
                    let mut p = app.prefs.clone();
                    p.shortcuts = o;
                    app.set_prefs(ctx, p);
                }
                Err(crate::keymap::Refusal::Taken { id, label }) => {
                    st.notice = Some((tr!("{key} is already used by {action}.", key = key(&chord), action = crate::i18n::t_cmd(id, label)), true));
                    st.steal = Some((cap, chord, id));
                }
                Err(crate::keymap::Refusal::Reserved(why)) => st.notice = Some((tr!("{key} is kept for {purpose}; choose another key.", key = key(&chord), purpose = crate::i18n::t(why)), true)),
                Err(crate::keymap::Refusal::TooMany) => st.notice = Some((trn!("An action can have up to {n} key.", "An action can have up to {n} keys.", crate::keymap::MAX_KEYS), true)),
            }
        }
    }

    let mut open = true;
    let mut acts: Vec<SheetAct> = Vec::new();
    let capture = app.key_capture;
    let changed = app.prefs.shortcuts.len();
    let title = tr!("Keyboard Shortcuts");
    let shown = egui::Window::new(title)
        .id(egui::Id::new("shortcuts_window"))
        .open(&mut open)
        .collapsible(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .default_width(660.0)
        .default_height(600.0)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut st.filter).hint_text(tr!("Filter by name or key…")).desired_width(ui.available_width() - 120.0)).set_name(tr!("Filter shortcuts"));
                let reset_all = ui.add_enabled(changed > 0, egui::Button::new(tr!("Reset all…"))).on_hover_text(tr!("Put every shortcut back to the defaults"));
                if reset_all.clicked() {
                    st.confirm_reset_all = true;
                }
            });
            // One message line: what is happening, what just happened, or what needs a decision.
            if let Some(cap) = capture {
                ui.label(text(Type::Label, tr!("Press the new keys for {action}. Esc cancels.", action = action_label(&app.keymap, cap.id))).color(T.accent_text));
            } else if st.confirm_reset_all {
                ui.horizontal(|ui| {
                    ui.label(text(Type::Label, trn!("Reset {n} changed shortcut to the defaults?", "Reset {n} changed shortcuts to the defaults?", changed)));
                    if ui.add(danger_button(tr!("Reset all"))).clicked() {
                        acts.push(SheetAct::ResetAll);
                    }
                    if ui.button(tr!("Cancel")).clicked() {
                        acts.push(SheetAct::Dismiss);
                    }
                });
            } else if let Some((msg, err)) = &st.notice {
                ui.horizontal(|ui| {
                    ui.label(text(Type::Label, msg.clone()).color(if *err { T.error } else { T.ok }));
                    if let Some((cap, _, _)) = &st.steal {
                        if ui.button(tr!("Use it for {action}", action = action_label(&app.keymap, cap.id))).clicked() {
                            acts.push(SheetAct::Steal);
                        }
                        if ui.button(tr!("Cancel")).clicked() {
                            acts.push(SheetAct::Dismiss);
                        }
                    }
                });
            } else {
                ui.label(text(Type::Caption, tr!("Click a key to change it or + to add one; right-click a key to remove it. Changes save as you go.")));
            }
            ui.add_space(4.0);

            // Names match in the interface language and in English.
            let f = st.filter.to_lowercase();
            let hit = |s: &str| f.is_empty() || s.to_lowercase().contains(&f);
            let named = |s: &'static str| hit(s) || hit(crate::i18n::t(s));
            let fixed = fixed_rows();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for group in GROUP_ORDER {
                    let actions: Vec<&crate::keymap::Action> = app
                        .keymap
                        .actions
                        .iter()
                        .filter(|a| a.group == group || (group == "Other" && !GROUP_ORDER.contains(&a.group)))
                        .filter(|a| hit(a.label) || hit(action_name(a)) || named(group) || app.keymap.keys(a.id).iter().any(|k| hit(k) || hit(&key(k))))
                        .collect();
                    let fixed: Vec<_> = fixed.iter().filter(|(g, l, ks)| *g == group && (named(l) || named(g) || ks.iter().any(|k| hit(k) || hit(&fixed_key(k))))).collect();
                    if actions.is_empty() && fixed.is_empty() {
                        continue;
                    }
                    let count = (actions.len() + fixed.len()).to_string();
                    crate::theme::section(ui, &format!("sheet_{group}"), crate::theme::Section::Rule, crate::i18n::t(group), Some(&count), |ui| {
                        egui::Grid::new(("sc", group)).num_columns(3).striped(true).min_col_width(60.0).show(ui, |ui| {
                            for (_, label, keys) in &fixed {
                                name_cell(ui, crate::i18n::t(label));
                                ui.horizontal(|ui| {
                                    ui.label(text(Type::Key, keys.iter().map(|k| fixed_key(k)).collect::<Vec<_>>().join(" · ")).color(T.text_dim));
                                    ui.add_space(6.0);
                                    ui.label(text(Type::Caption, tr!("fixed")));
                                });
                                // An empty cell (an empty label would be an empty, clickable node for screen readers).
                                ui.allocate_space(egui::Vec2::ZERO);
                                ui.end_row();
                            }
                            for a in actions {
                                let name = action_name(a);
                                name_cell(ui, name);
                                ui.horizontal(|ui| {
                                    let keys = app.keymap.keys(a.id);
                                    for (i, k) in keys.iter().enumerate() {
                                        let recording = capture == Some(KeyCapture { id: a.id, replace: Some(i) });
                                        let chip = if recording {
                                            egui::Button::new(text(Type::Key, tr!("Press keys…")).color(T.on_fill)).fill(T.accent)
                                        } else {
                                            egui::Button::new(text(Type::Key, key(k)).color(T.text))
                                        };
                                        let r = ui.add(chip).named(&tr!("{action} shortcut {key}", action = name, key = key(k))).on_hover_text(tr!("Click to change · right-click to remove"));
                                        if r.clicked() {
                                            acts.push(SheetAct::Capture(KeyCapture { id: a.id, replace: Some(i) }));
                                        }
                                        r.context_menu(|ui| {
                                            if ui.button(tr!("Change…")).clicked() {
                                                acts.push(SheetAct::Capture(KeyCapture { id: a.id, replace: Some(i) }));
                                                ui.close();
                                            }
                                            if ui.button(tr!("Remove")).clicked() {
                                                acts.push(SheetAct::Remove(a.id, i));
                                                ui.close();
                                            }
                                        });
                                    }
                                    if capture == Some(KeyCapture { id: a.id, replace: None }) {
                                        ui.add(egui::Button::new(text(Type::Key, tr!("Press keys…")).color(T.on_fill)).fill(T.accent));
                                    } else if keys.len() < crate::keymap::MAX_KEYS {
                                        let (tip, a11y_name) = if keys.is_empty() {
                                            (tr!("Add a shortcut"), tr!("Add a shortcut for {action}", action = name))
                                        } else {
                                            (tr!("Add another shortcut"), tr!("Add another shortcut for {action}", action = name))
                                        };
                                        if ui.add(egui::Button::new("+").min_size(egui::vec2(crate::theme::TARGET, crate::theme::TARGET))).named(&a11y_name).on_hover_text(tip).clicked() {
                                            acts.push(SheetAct::Capture(KeyCapture { id: a.id, replace: None }));
                                        }
                                    }
                                });
                                if app.keymap.is_default(a.id) {
                                    // An empty cell (an empty label would be an empty, clickable node for screen readers).
                                    ui.allocate_space(egui::Vec2::ZERO);
                                } else {
                                    let defaults = if a.defaults.is_empty() { tr!("none").to_string() } else { a.defaults.iter().map(|k| key(k)).collect::<Vec<_>>().join(" · ") };
                                    if ui.button(tr!("Reset")).named(&tr!("Reset {action}", action = name)).on_hover_text(tr!("Back to the default ({keys})", keys = defaults)).clicked() {
                                        acts.push(SheetAct::Reset(a.id));
                                    }
                                }
                                ui.end_row();
                            }
                        });
                    });
                }
            });
        });
    a11y::name_title_bar(&shown, title);

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
                st.notice = Some((tr!("{action}: key removed", action = action_label(&app.keymap, id)), false));
            }
            SheetAct::Reset(id) => {
                p.shortcuts = crate::keymap::reset(&p.shortcuts, id);
                st.notice = Some((tr!("{action}: back to the default", action = action_label(&app.keymap, id)), false));
            }
            SheetAct::ResetAll => {
                p.shortcuts.clear();
                st.confirm_reset_all = false;
                st.notice = Some((tr!("Every shortcut is back to the defaults.").into(), false));
            }
            SheetAct::Steal => {
                let Some((cap, chord, from)) = st.steal.take() else { continue };
                match crate::keymap::assign(&p.shortcuts, cap.id, cap.replace, &chord, true) {
                    Ok(o) => {
                        p.shortcuts = o;
                        st.notice = Some((tr!("{action}: {key} (removed from {other})", action = action_label(&app.keymap, cap.id), key = key(&chord), other = action_label(&app.keymap, from)), false));
                    }
                    Err(_) => st.notice = Some((tr!("{key} could not be moved.", key = key(&chord)), true)),
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
