//! The Actions panel (Window › Actions, F9; ADR 0024): what a keyframe does when the playhead
//! enters it, and what a button or movie clip instance does on pointer events, as a structured
//! list. Targets come from the document's instance names, frames and labels from its
//! timelines, scenes from its scene list, so nothing needs typing but a trace message.
//! Every change is one `actions.set` (or `instance.set_name`) command, so it undoes in one step.
//!
//! A keyframe may instead keep a **script** written in another language (an Animate document's
//! ActionScript; ADR 0013 tier 1): the Script view shows and edits it in a monospace editor
//! with line numbers (`script.set`, typing coalesced into one undo step). Nothing runs it yet.

use crate::a11y::Named;
use crate::app::App;
use crate::icons::Icon;
use crate::theme::{FORM_LABEL_DOCK, T, Type, font, form_row, text, with_type};
use anicraft_engine::doc::actions::{Action, ButtonEvent, FrameRef, FrameScript, KINDS, Language, Property, Seg, SourceScript, Target};
use anicraft_engine::doc::{Document, Element, ElementKind, LayerId, LayerKind, SymbolId, SymbolKind, Timeline};
use anicraft_engine::plugins::Kind;
use serde_json::{Value, json};

/// Which list the panel edits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Frame,
    Instance,
}

/// What the panel addresses (view state, not saved).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub tab: Tab,
    /// A movie clip's timeline instead of the scene's (which follows the timeline panel).
    pub symbol: Option<SymbolId>,
    pub layer: Option<LayerId>,
    pub frame: u32,
    /// The keyframe whose Script view is open while it keeps no script yet (the editor stays
    /// up while the first line is typed, or after the last is deleted).
    pub script_at: Option<KeyAt>,
}

/// A keyframe: its timeline (a symbol's, or the scene's), layer and first frame.
pub type KeyAt = (Option<SymbolId>, LayerId, u32);

/// What a keyframe holds, as the Frame tab's Type row offers it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Holds {
    Actions,
    Plugin,
    Script,
}

/// Opens the panel on a tab (it floats, or wherever it was docked: ADR 0075).
pub fn open(app: &mut App, tab: Tab) {
    app.show_panel(crate::dock::Panel::Actions);
    app.actions.tab = tab;
}

/// The selected movie clip or button instance on the active layer, if exactly one is selected.
fn selected_instance(app: &App) -> Option<(Element, SymbolKind)> {
    let sel = app.s.selected_elements();
    let [(_, e)] = sel.as_slice() else { return None };
    let ElementKind::Symbol { symbol, .. } = &e.kind else { return None };
    let kind = app.s.doc.symbol(*symbol)?.kind;
    (kind != SymbolKind::Graphic).then(|| ((**e).clone(), kind))
}

/// The panel's contents, wherever it is.
pub fn panel(app: &mut App, ui: &mut egui::Ui) {
    let inst = selected_instance(app);
    ui.horizontal_wrapped(|ui| {
        if ui.selectable_label(app.actions.tab == Tab::Frame, tr!("Frame")).on_hover_text(tr!("What runs when the playhead enters a keyframe")).clicked() {
            app.actions.tab = Tab::Frame;
        }
        let label = match &inst {
            Some((e, k)) => match (*k == SymbolKind::Button, e.name.is_empty()) {
                (true, true) => tr!("Button (unnamed)").to_string(),
                (true, false) => tr!("Button “{name}”", name = e.name),
                (false, true) => tr!("Movie clip (unnamed)").to_string(),
                (false, false) => tr!("Movie clip “{name}”", name = e.name),
            },
            None => tr!("Instance").to_string(),
        };
        let r = ui.add_enabled(inst.is_some(), egui::Button::selectable(app.actions.tab == Tab::Instance, label)).on_hover_text(tr!("What the selected button or movie clip does on clicks and the pointer"));
        let r = r.on_disabled_hover_text(tr!("Select one button or movie clip instance on the stage"));
        if r.clicked() {
            app.actions.tab = Tab::Instance;
        }
    });
    ui.separator();
    match (app.actions.tab, inst) {
        (Tab::Instance, Some((e, _))) => instance_tab(app, ui, &e),
        (Tab::Instance, None) => {
            ui.label(text(Type::Caption, tr!("Select one button or movie clip instance on the stage to give it events (a graphic can't have them: set its symbol's Type in Properties).")));
        }
        (Tab::Frame, _) => frame_tab(app, ui),
    }
}

/// Where a frame edit goes: the params naming the timeline, layer and keyframe.
struct FrameTarget {
    base: Value,
    script: Option<FrameScript>,
    timeline: Timeline,
    main: bool,
    /// Whether the keyframe's actions run (the main timeline or a movie clip, not a guide).
    runs: bool,
    at: KeyAt,
}

/// Library symbols the Frame tab can show: movie clips (their frames run actions), and any
/// other symbol with a kept script on its timeline (shown, not run).
fn symbol_timelines(doc: &Document) -> Vec<(SymbolId, String, SymbolKind)> {
    doc.library.iter().filter(|s| s.kind == SymbolKind::MovieClip || keeps_script(&s.timeline)).map(|s| (s.id, s.name.clone(), s.kind)).collect()
}

/// Whether the document uses scripts at all (a kept script, a linkage class or a document
/// class), so the class fields in Properties are worth showing.
pub fn scripted(doc: &Document) -> bool {
    !doc.document_class.is_empty() || doc.library.iter().any(|s| s.linkage.is_some()) || doc.timelines().any(keeps_script)
}

/// A class-name field (a symbol's linkage class, the document class). It edits a draft and
/// commits when focus leaves (Enter, Tab or a click elsewhere), since a half-typed name
/// ("com.") isn't a class name yet; a draft that isn't one shows in the error colour.
pub fn class_field(ui: &mut egui::Ui, field: &str, current: &str, name: &str, hint: &str) -> Option<String> {
    let id = ui.make_persistent_id(field);
    let mut draft: String = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_else(|| current.to_string());
    let ok = draft.trim().is_empty() || anicraft_engine::doc::actions::valid_class_name(draft.trim());
    let mut edit = egui::TextEdit::singleline(&mut draft).desired_width(crate::theme::fit(ui, 170.0)).font(font(Type::Path)).hint_text(tr!("none"));
    if !ok {
        edit = edit.text_color(T.error);
    }
    let r = ui.add(edit).named(name).on_hover_text(hint);
    if r.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, draft.clone()));
    }
    if r.lost_focus() {
        ui.data_mut(|d| d.remove::<String>(id));
        let v = draft.trim().to_string();
        if v != current {
            return Some(v);
        }
    }
    None
}

fn keeps_script(tl: &Timeline) -> bool {
    tl.layers.iter().any(|l| l.keyframes.iter().any(|k| is_kept(k)))
}

fn is_kept(k: &anicraft_engine::doc::Keyframe) -> bool {
    matches!(k.script, Some(FrameScript::Source(_)))
}

/// A symbol's timeline as the Timeline picker names it: one template per kind.
fn kind_name(name: &str, k: SymbolKind) -> String {
    match k {
        SymbolKind::MovieClip => tr!("{name} (movie clip)", name = name),
        SymbolKind::Button => tr!("{name} (button)", name = name),
        SymbolKind::Graphic => tr!("{name} (graphic)", name = name),
    }
}

fn frame_tab(app: &mut App, ui: &mut egui::Ui) {
    let doc = app.s.doc.clone();
    let scene_name = doc.scenes.get(app.s.scene).map_or_else(|| tr!("Scene").to_string(), |s| s.name.clone());
    // Which timeline: the scene being edited, or a symbol's.
    let symbols = symbol_timelines(&doc);
    if app.actions.symbol.is_some_and(|id| !symbols.iter().any(|(c, _, _)| *c == id)) {
        app.actions.symbol = None;
    }
    let current = app.actions.symbol.and_then(|id| symbols.iter().find(|(c, _, _)| *c == id)).map_or(scene_name.clone(), |(_, n, k)| kind_name(n, *k));
    form_row(ui, FORM_LABEL_DOCK, tr!("Timeline"), |ui| {
        egui::ComboBox::from_id_salt("act_timeline").width(crate::theme::fit(ui, 220.0)).selected_text(current).show_ui(ui, |ui| {
            if ui.selectable_label(app.actions.symbol.is_none(), tr!("{name} (main timeline)", name = scene_name)).clicked() {
                app.actions.symbol = None;
            }
            for (id, n, k) in &symbols {
                if ui.selectable_label(app.actions.symbol == Some(*id), kind_name(n, *k)).clicked() {
                    app.actions = State { tab: Tab::Frame, symbol: Some(*id), ..State::default() };
                }
            }
        })
        .response
        .set_name(tr!("Timeline"));
    });
    let Some(t) = frame_target(app, ui, &doc) else { return };
    let source = t.script.as_ref().and_then(FrameScript::source).cloned();
    let kind = match &t.script {
        Some(FrameScript::Plugin(_)) => Holds::Plugin,
        Some(FrameScript::Source(_)) => Holds::Script,
        _ if app.actions.script_at == Some(t.at) => Holds::Script,
        _ => Holds::Actions,
    };
    if !t.runs {
        // Graphics, buttons and guides never run frame scripts; a kept one is still shown.
        ui.label(text(Type::Caption, tr!("Frames here don't run (graphic and button timelines, guide layers); the script is kept with the document.")));
        script_view(app, ui, &t, source.as_ref());
        return;
    }
    let mut to = None;
    form_row(ui, FORM_LABEL_DOCK, tr!("Type"), |ui| {
        for (k, label, hint) in [
            (Holds::Actions, tr!("Actions"), tr!("A list of actions, in order")),
            (Holds::Plugin, tr!("Plug-in"), tr!("An entry point of a frame-script plug-in (WebAssembly)")),
            (Holds::Script, tr!("Script"), tr!("Code in a scripting language (such as an Animate document's ActionScript), kept with the document. ActionScript 3.0 runs when it uses only timeline control, mouse listeners, trace and instance properties")),
        ] {
            if ui.selectable_label(kind == k, label).on_hover_text(hint).clicked() && kind != k {
                to = Some(k);
            }
        }
    });
    if let Some(to) = to {
        switch_kind(app, &t, kind, to);
        return;
    }
    match (kind, &t.script) {
        (Holds::Plugin, Some(FrameScript::Plugin(ps))) => plugin_rows(app, ui, &t.base, &ps.id, &ps.entry),
        (Holds::Script, _) => script_view(app, ui, &t, source.as_ref()),
        _ => {
            let list = t.script.as_ref().map(|s| s.actions().to_vec()).unwrap_or_default();
            let cx = Choices::new(&doc, &t.timeline, t.main);
            if list.is_empty() {
                ui.label(text(Type::Caption, tr!("No actions yet. Add one: Stop on a frame holds the movie there; a button's Click can then go on.")));
            }
            if let Some((new, live)) = edit_list(ui, "frame", &list, &cx) {
                let mut p = t.base.clone();
                p["actions"] = serde_json::to_value(&new).unwrap_or(Value::Null);
                run(app, live, tr_noop!("Edit actions"), "actions.set", p);
            }
        }
    }
}

/// Changes what the keyframe holds. What it held goes (one undo step, with a hint).
fn switch_kind(app: &mut App, t: &FrameTarget, from: Holds, to: Holds) {
    app.actions.script_at = None;
    let had = match &t.script {
        Some(FrameScript::Actions(a)) => !a.is_empty(),
        Some(_) => true,
        None => false,
    };
    match to {
        Holds::Actions => {
            if had {
                set_frame(app, &t.base, json!({"actions": []}));
            }
        }
        Holds::Plugin => {
            if let Some((id, entry)) = frame_scripts(app).first().map(|(id, _, e)| (id.clone(), e.first().cloned().unwrap_or_else(|| "main".into()))) {
                set_frame(app, &t.base, json!({"plugin": id, "entry": entry}));
                app.status_ok(tr!("Frame script set — {undo} to undo", undo = app.undo_key()));
            } else {
                app.status_err(tr!("No frame-script plug-ins are installed (Commands › Install Plug-in…)"));
            }
            return;
        }
        Holds::Script => {
            // The editor opens empty; the first line typed keeps the script.
            if had {
                set_frame(app, &t.base, json!({"actions": []}));
            }
            app.actions.script_at = Some(t.at);
        }
    }
    if had {
        let removed = match from {
            Holds::Actions => tr!("Removed the actions — {undo} to undo", undo = app.undo_key()),
            Holds::Plugin => tr!("Removed the plug-in frame script — {undo} to undo", undo = app.undo_key()),
            Holds::Script => tr!("Removed the script — {undo} to undo", undo = app.undo_key()),
        };
        app.status_ok(removed);
    }
}

/// Space above the first line of the script editor (and its line numbers).
const EDITOR_MARGIN: i8 = 4;

/// The character index of a 1-based line and column in `text` (lines split at `\n`).
fn char_index(text: &str, line: u32, column: u32) -> usize {
    let mut index = 0;
    for (i, l) in text.split('\n').enumerate() {
        if i + 1 == line as usize {
            return index + (column.max(1) as usize - 1).min(l.chars().count());
        }
        index += l.chars().count() + 1;
    }
    index
}

/// Under the editor: whether the script runs (ADR 0013 tiers 2 and 2.5), what in it is
/// ignored, and each problem that keeps it from running, where it is (`links`: names it takes
/// from other frames that its timeline doesn't have). Clicking a problem puts the cursor there.
fn script_status(ui: &mut egui::Ui, src: &SourceScript, links: &[anicraft_engine::doc::actions::as3::Problem], text_shown: &str, editor: egui::Id) {
    let row = |ui: &mut egui::Ui, p: &anicraft_engine::doc::actions::as3::Problem, color: egui::Color32| {
        // The engine marks code with backticks (for the CLI and agents); here it reads plainly.
        let message = p.message.replace('`', "");
        let label = match p.pos() {
            Some(at) => tr!("Line {line}, column {column}: {message}", line = at.line, column = at.column, message = message.as_str()),
            None => message,
        };
        let r = ui.add(egui::Label::new(text(Type::Caption, label).color(color)).sense(egui::Sense::click()).wrap());
        if let Some(at) = p.pos()
            && r.on_hover_text(tr!("Show this line in the script")).clicked()
        {
            let i = char_index(text_shown, at.line, at.column);
            if let Some(mut st) = egui::text_edit::TextEditState::load(ui.ctx(), editor) {
                st.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(i))));
                st.store(ui.ctx(), editor);
            }
            ui.ctx().memory_mut(|m| m.request_focus(editor));
        }
    };
    let problems = match src.compiled() {
        Ok(c) if links.is_empty() => {
            ui.horizontal_wrapped(|ui| {
                ui.label(text(Type::Body, tr!("Runs in Test Movie and the HTML canvas export")).color(T.ok));
                ui.label(text(Type::Data, trn!("{n} step", "{n} steps", c.steps)).color(T.text_dim));
            });
            for n in &c.notes {
                row(ui, n, T.text_dim);
            }
            return;
        }
        Ok(_) => links,
        Err(problems) => problems.as_slice(),
    };
    ui.label(text(Type::Body, trn!("Doesn't run: {n} problem", "Doesn't run: {n} problems", problems.len())).color(T.error));
    // A few rows tall, so the code stays in view however many there are.
    egui::ScrollArea::vertical().id_salt("act_script_problems").max_height(96.0).auto_shrink([false, true]).show(ui, |ui| {
        for p in problems {
            row(ui, p, T.error);
        }
    });
}

/// The Script view: the kept script's language, and its text in a monospace editor with line
/// numbers; under it, whether it runs and what keeps it from running. Typing edits the
/// document live (`script.set`, one undo step per burst of typing).
fn script_view(app: &mut App, ui: &mut egui::Ui, t: &FrameTarget, src: Option<&SourceScript>) {
    let language = src.map_or(Language::default(), |s| s.language);
    let mut pick = None;
    form_row(ui, FORM_LABEL_DOCK, tr!("Language"), |ui| {
        ui.add_enabled_ui(src.is_some(), |ui| {
            egui::ComboBox::from_id_salt("act_language").width(crate::theme::fit(ui, 180.0)).selected_text(crate::i18n::t(language.label())).show_ui(ui, |ui| {
                for l in Language::ALL {
                    if ui.selectable_label(l == language, crate::i18n::t(l.label())).clicked() && l != language {
                        pick = Some(l);
                    }
                }
            })
            .response
            .set_name(tr!("Script language"));
        });
        if let Some(s) = src {
            ui.label(text(Type::Data, trn!("{n} line", "{n} lines", s.lines())).color(T.text_dim));
        }
    });
    if let Some(l) = pick {
        let mut p = t.base.clone();
        p["language"] = json!(l.key());
        app.run("script.set", p);
    }
    // The text, with line endings shown as typed (a file's CR LF reads as one break).
    let shown = src.map(|s| if s.text.contains('\r') { s.text.replace("\r\n", "\n") } else { s.text.clone() }).unwrap_or_default();
    let mut edited = shown.clone();
    let code = font(Type::Path);
    let rows = shown.split('\n').count().max(1);
    let editor_id = ui.make_persistent_id("act_script_editor");
    // Names it takes from other frames of its timeline that aren't there (ADR 0052).
    let links = match src.map(|s| s.compiled()) {
        Some(Ok(c)) if t.runs => {
            let which = t.base.get("symbol").and_then(Value::as_u64).map_or(anicraft_engine::doc::player::TimelineRef::Root, |id| anicraft_engine::doc::player::TimelineRef::Symbol(SymbolId(id)));
            anicraft_engine::doc::player::link_problems(&app.s.doc, which, c)
        }
        _ => Vec::new(),
    };
    // The lines that hold a problem, marked in the gutter.
    let bad: std::collections::BTreeSet<u32> = match src.map(|s| s.compiled()) {
        Some(Err(p)) => p.iter().map(|p| p.line).collect(),
        _ => links.iter().map(|p| p.line).collect(),
    };
    // Whether it runs, and what keeps it from running, above the code.
    if let Some(s) = src {
        script_status(ui, s, &links, &shown, editor_id);
    }
    let changed = egui::ScrollArea::both()
        .id_salt("act_script")
        .max_height(320.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                // Line numbers, right-aligned in a gutter, on the same rows as the text.
                let digits = rows.to_string().len().max(2);
                let mut numbers = egui::text::LayoutJob::default();
                for i in 1..=rows {
                    let color = if bad.contains(&(i as u32)) { T.error } else { T.text_faint };
                    let sep = if i < rows { "\n" } else { "" };
                    numbers.append(&format!("{i:>digits$}{sep}"), 0.0, egui::TextFormat { font_id: code.clone(), color, ..Default::default() });
                }
                ui.vertical(|ui| {
                    ui.add_space(f32::from(EDITOR_MARGIN));
                    ui.add(egui::Label::new(numbers).selectable(false));
                });
                let editor = egui::TextEdit::multiline(&mut edited)
                    .id(editor_id)
                    .code_editor()
                    .font(code.clone())
                    .margin(egui::Margin::symmetric(6, EDITOR_MARGIN))
                    .desired_rows(12)
                    .desired_width(f32::INFINITY)
                    .hint_text(tr!("// Type or paste a frame script"));
                ui.add(editor).named(tr!("Frame script")).changed()
            })
            .inner
        })
        .inner;
    if changed && edited != shown {
        app.actions.script_at = Some(t.at);
        let mut p = t.base.clone();
        p["text"] = json!(edited);
        if src.is_none() {
            p["language"] = json!(language.key());
        }
        run(app, true, tr_noop!("Edit script"), "script.set", p);
    }
    match src.map(SourceScript::runs) {
        None => {
            ui.label(text(Type::Caption, tr!("Kept with the document as written. ActionScript 3.0 that controls the timeline runs in Test Movie and the HTML canvas export.")));
        }
        Some(false) => {
            ui.label(text(Type::Caption, tr!("A script runs only when all of it is in the ActionScript 3.0 subset AniCraft runs: timeline control (stop, play, go to a frame or label), mouse listeners, trace, and setting x, y, alpha, visible, rotation and scale on named instances. Until then it is kept as written and doesn't run.")));
        }
        Some(true) => {}
    }
    if src.is_some() && ui.small_button(tr!("Remove script")).on_hover_text(tr!("Delete this keyframe's script")).clicked() {
        app.run("script.clear", t.base.clone());
        app.status_ok(tr!("Removed the script — {undo} to undo", undo = app.undo_key()));
    }
}

/// The keyframe the Frame tab edits (and the pickers that choose it), or `None` when there is
/// none.
fn frame_target(app: &mut App, ui: &mut egui::Ui, doc: &Document) -> Option<FrameTarget> {
    match app.actions.symbol {
        None => {
            let tl = app.s.timeline().clone();
            let layer = tl.layer(app.s.layer)?;
            let frame = app.s.frame;
            form_row(ui, FORM_LABEL_DOCK, tr!("Keyframe"), |ui| {
                ui.label(text(Type::Data, format!("{}", frame + 1)));
                ui.label(text(Type::Body, layer.name.as_str()).color(T.text_dim));
            });
            if layer.kind == LayerKind::Folder || (layer.kind == LayerKind::Guide && !layer.keyframe_at(frame).is_some_and(is_kept)) {
                ui.label(text(Type::Caption, tr!("Folders and guide layers don't run actions. Pick another layer in the timeline.")));
                return None;
            }
            let Some(k) = layer.keyframe_at(frame) else {
                let key = app.keymap.primary("timeline.insert_blank_keyframe").map(crate::platform::shortcut_label).unwrap_or_default();
                ui.label(text(Type::Caption, tr!("Frame {frame} is empty on this layer. Add a blank keyframe ({key}) to give it actions.", frame = frame + 1, key = key)));
                return None;
            };
            if k.start != frame {
                let key = app.keymap.primary("timeline.insert_keyframe").map(crate::platform::shortcut_label).unwrap_or_default();
                ui.label(text(Type::Caption, tr!("These run when the playhead enters frame {keyframe} (the keyframe). For frame {frame}, add a keyframe there ({key}).", keyframe = k.start + 1, frame = frame + 1, key = key)));
            }
            let runs = layer.kind != LayerKind::Guide;
            Some(FrameTarget { base: json!({"layer": layer.id.0, "frame": k.start}), script: k.script.clone(), timeline: tl.clone(), main: true, runs, at: (None, layer.id, k.start) })
        }
        Some(sid) => {
            let sym = doc.symbol(sid)?;
            let clip = sym.kind == SymbolKind::MovieClip;
            // A movie clip's layers that run; on other symbols, the layers that keep a script.
            let offered = |l: &anicraft_engine::doc::Layer| if clip { !matches!(l.kind, LayerKind::Folder | LayerKind::Guide) } else { l.keyframes.iter().any(|k| is_kept(k)) };
            let layers: Vec<(LayerId, String)> = sym.timeline.layers.iter().filter(|l| offered(l)).map(|l| (l.id, l.name.clone())).collect();
            let lid = app.actions.layer.filter(|l| layers.iter().any(|(x, _)| x == l)).or_else(|| layers.first().map(|(l, _)| *l))?;
            app.actions.layer = Some(lid);
            let layer = sym.timeline.layer(lid)?;
            form_row(ui, FORM_LABEL_DOCK, tr!("Layer"), |ui| {
                egui::ComboBox::from_id_salt("act_layer").width(crate::theme::fit(ui, 180.0)).selected_text(layer.name.as_str()).show_ui(ui, |ui| {
                    for (id, n) in &layers {
                        if ui.selectable_label(*id == lid, n.as_str()).clicked() {
                            app.actions.layer = Some(*id);
                        }
                    }
                })
                .response
                .set_name(tr!("Layer"));
            });
            let k = layer.keyframe_at(app.actions.frame).or_else(|| layer.keyframes.first().map(|k| &**k))?;
            app.actions.frame = k.start;
            let shown = |k: &anicraft_engine::doc::Keyframe| if k.label.is_empty() { format!("{}", k.start + 1) } else { format!("{} “{}”", k.start + 1, k.label) };
            form_row(ui, FORM_LABEL_DOCK, tr!("Keyframe"), |ui| {
                with_type(ui, Type::Data, |ui| {
                    egui::ComboBox::from_id_salt("act_key").width(crate::theme::fit(ui, 180.0)).selected_text(shown(k)).show_ui(ui, |ui| {
                        for kk in &layer.keyframes {
                            let mark = if kk.script.is_some() { " ·a" } else { "" };
                            if ui.selectable_label(kk.start == k.start, format!("{}{mark}", shown(kk))).clicked() {
                                app.actions.frame = kk.start;
                            }
                        }
                    })
                    .response
                    .set_name(tr!("Keyframe"));
                });
            });
            Some(FrameTarget { base: json!({"symbol": sid.0, "layer": lid.0, "frame": k.start}), script: k.script.clone(), timeline: sym.timeline.clone(), main: false, runs: clip, at: (Some(sid), lid, k.start) })
        }
    }
}

/// Installed frame-script plug-ins: `(id, name, entries)`.
fn frame_scripts(app: &App) -> Vec<(String, String, Vec<String>)> {
    app.s.plugins.list().filter(|p| p.manifest().kind == Kind::FrameScript).map(|p| (p.id().to_string(), p.manifest().name.clone(), p.manifest().entries.clone())).collect()
}

fn plugin_rows(app: &mut App, ui: &mut egui::Ui, base: &Value, id: &str, entry: &str) {
    let all = frame_scripts(app);
    let installed = all.iter().find(|(p, _, _)| p == id);
    let shown = installed.map_or_else(|| id.to_string(), |(_, n, _)| n.clone());
    let mut pick: Option<(String, String)> = None;
    form_row(ui, FORM_LABEL_DOCK, tr!("Plug-in"), |ui| {
        egui::ComboBox::from_id_salt("act_plugin").width(crate::theme::fit(ui, 220.0)).selected_text(shown).show_ui(ui, |ui| {
            for (pid, name, entries) in &all {
                if ui.selectable_label(pid == id, name.as_str()).on_hover_text(pid.as_str()).clicked() {
                    pick = Some((pid.clone(), entries.first().cloned().unwrap_or_else(|| "main".into())));
                }
            }
        })
        .response
        .set_name(tr!("Plug-in"));
    });
    let entries = installed.map(|(_, _, e)| e.clone()).unwrap_or_default();
    form_row(ui, FORM_LABEL_DOCK, tr!("Entry"), |ui| {
        with_type(ui, Type::Data, |ui| {
            egui::ComboBox::from_id_salt("act_entry").width(crate::theme::fit(ui, 160.0)).selected_text(entry).show_ui(ui, |ui| {
                for e in &entries {
                    if ui.selectable_label(e == entry, e.as_str()).clicked() {
                        pick = Some((id.to_string(), e.clone()));
                    }
                }
            })
            .response
            .set_name(tr!("Entry point"));
        });
    });
    if installed.is_none() {
        ui.label(text(Type::Caption, tr!("“{id}” isn't installed here: Test Movie says so and plays on.", id = id)).color(T.error));
    }
    ui.label(text(Type::Caption, tr!("The plug-in runs each time the playhead enters this keyframe, reads where the movie is, and sends actions back. It can't change the document.")));
    if let Some((pid, e)) = pick {
        set_frame(app, base, json!({"plugin": pid, "entry": e}));
    }
}

fn set_frame(app: &mut App, base: &Value, extra: Value) {
    let mut p = base.clone();
    if let (Some(o), Some(e)) = (p.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    app.run("actions.set", p);
}

/// Runs an edit: continuous ones (typing, dragging a number) coalesce into one undo step.
fn run(app: &mut App, live: bool, label: &str, id: &str, p: Value) {
    if live {
        app.s.begin_gesture(label);
    }
    app.run(id, p);
}

fn instance_tab(app: &mut App, ui: &mut egui::Ui, e: &Element) {
    let doc = app.s.doc.clone();
    let mut name = e.name.clone();
    form_row(ui, FORM_LABEL_DOCK, tr!("Name"), |ui| {
        if ui.add(egui::TextEdit::singleline(&mut name).desired_width(crate::theme::fit(ui, 200.0)).hint_text(tr!("instance name"))).named(tr!("Instance name")).changed() {
            run(app, true, tr_noop!("Instance name"), "instance.set_name", json!({"element": e.id.0, "name": name}));
        }
    });
    if e.name.is_empty() {
        ui.label(text(Type::Caption, tr!("Name it to address it from actions (letters, digits and _). Its events work without a name.")));
    }
    let ElementKind::Symbol { events, .. } = &e.kind else { return };
    let tl = app.s.timeline().clone();
    let cx = Choices::new(&doc, &tl, true);
    ui.add_space(4.0);
    ui.label(text(Type::Caption, tr!("Its actions run on the timeline it is on: “this” is the main timeline.")));
    for h in events {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(text(Type::Body, event_heading(h.event)).strong()).on_hover_text(crate::i18n::t(h.event.hint()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button(tr!("Remove event")).clicked() {
                    app.run("actions.clear", json!({"element": e.id.0, "event": h.event.key()}));
                    app.status_ok(event_removed(h.event, &app.undo_key()));
                }
            });
        });
        if let Some((new, live)) = edit_list(ui, h.event.key(), &h.actions, &cx) {
            run(app, live, tr_noop!("Edit event"), "actions.set", json!({"element": e.id.0, "event": h.event.key(), "actions": new}));
        }
    }
    let unused: Vec<ButtonEvent> = ButtonEvent::ALL.into_iter().filter(|ev| !events.iter().any(|h| h.event == *ev)).collect();
    if !unused.is_empty() {
        ui.add_space(6.0);
        ui.menu_button(tr!("Add event"), |ui| {
            for ev in unused {
                if ui.button(crate::i18n::t(ev.label())).on_hover_text(crate::i18n::t(ev.hint())).clicked() {
                    // A new event starts with the action people want most: go on playing.
                    let first = Action::Play { target: Target::this() };
                    app.run("actions.set", json!({"element": e.id.0, "event": ev.key(), "actions": [first]}));
                    ui.close();
                }
            }
        });
    }
}

/// An event's heading in the Instance tab ("On click"): one sentence per event.
fn event_heading(ev: ButtonEvent) -> &'static str {
    match ev {
        ButtonEvent::Click => tr!("On click"),
        ButtonEvent::Press => tr!("On press"),
        ButtonEvent::Release => tr!("On release"),
        ButtonEvent::Over => tr!("On pointer over"),
        ButtonEvent::Out => tr!("On pointer out"),
    }
}

/// The status message after an event is removed: one sentence per event.
fn event_removed(ev: ButtonEvent, undo: &str) -> String {
    match ev {
        ButtonEvent::Click => tr!("Removed the click event — {undo} to undo", undo = undo),
        ButtonEvent::Press => tr!("Removed the press event — {undo} to undo", undo = undo),
        ButtonEvent::Release => tr!("Removed the release event — {undo} to undo", undo = undo),
        ButtonEvent::Over => tr!("Removed the pointer over event — {undo} to undo", undo = undo),
        ButtonEvent::Out => tr!("Removed the pointer out event — {undo} to undo", undo = undo),
    }
}

/// What the pickers offer, from the document.
struct Choices<'a> {
    doc: &'a Document,
    /// The timeline the actions are on, and whether it is a scene's main timeline.
    tl: &'a Timeline,
    main: bool,
    /// Instance paths reachable from it, with their kind.
    targets: Vec<(String, SymbolKind)>,
}

impl<'a> Choices<'a> {
    fn new(doc: &'a Document, tl: &'a Timeline, main: bool) -> Self {
        let mut targets = Vec::new();
        collect_targets(doc, tl, "", 0, &mut targets);
        Choices { doc, tl, main, targets }
    }

    /// The timeline `t` names, as far as the document says (a name on any keyframe).
    fn timeline_of(&self, t: &Target) -> Option<(&'a Timeline, bool)> {
        let mut cur = (self.tl, self.main);
        for seg in t.segments().ok()? {
            cur = match seg {
                Seg::Root => (self.doc.timeline(0), true),
                Seg::Parent => return None,
                Seg::Child(name) => (named(self.doc, cur.0, name)?, false),
            };
        }
        Some(cur)
    }

    fn labels(&self, t: &Target) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let tls: Vec<&Timeline> = match self.timeline_of(t) {
            Some((tl, true)) => self.doc.scenes.iter().map(|s| &s.timeline).chain(std::iter::once(tl)).collect(),
            Some((tl, false)) => vec![tl],
            None => self.doc.timelines().collect(),
        };
        for tl in tls {
            for l in &tl.layers {
                for k in &l.keyframes {
                    if !k.label.is_empty() && !out.contains(&k.label) {
                        out.push(k.label.clone());
                    }
                }
            }
        }
        out
    }
}

/// The symbol timeline of the instance named `name` on `tl` (on any keyframe).
fn named<'a>(doc: &'a Document, tl: &Timeline, name: &str) -> Option<&'a Timeline> {
    tl.layers.iter().flat_map(|l| l.keyframes.iter()).flat_map(|k| k.elements.iter()).find_map(|e| match &e.kind {
        ElementKind::Symbol { symbol, .. } if e.name == name => doc.symbol(*symbol).map(|s| &s.timeline),
        _ => None,
    })
}

/// Named movie clip and button instances on `tl` and inside them, as dot paths.
fn collect_targets(doc: &Document, tl: &Timeline, prefix: &str, depth: usize, out: &mut Vec<(String, SymbolKind)>) {
    if depth > 3 || out.len() >= 200 {
        return;
    }
    for e in tl.layers.iter().flat_map(|l| l.keyframes.iter()).flat_map(|k| k.elements.iter()) {
        let ElementKind::Symbol { symbol, .. } = &e.kind else { continue };
        let Some(sym) = doc.symbol(*symbol) else { continue };
        if e.name.is_empty() || sym.kind == SymbolKind::Graphic {
            continue;
        }
        let path = if prefix.is_empty() { e.name.clone() } else { format!("{prefix}.{}", e.name) };
        if out.iter().any(|(p, _)| *p == path) {
            continue;
        }
        out.push((path.clone(), sym.kind));
        if sym.kind == SymbolKind::MovieClip {
            collect_targets(doc, &sym.timeline, &path, depth + 1, out);
        }
    }
}

/// One edit of a list.
enum Change {
    Edit(bool),
    Remove,
    Up,
    Down,
}

/// The rows of an action list, and Add action. Returns the new list when it changed, and
/// whether the change is continuous (typing, dragging).
fn edit_list(ui: &mut egui::Ui, scope: &str, list: &[Action], cx: &Choices<'_>) -> Option<(Vec<Action>, bool)> {
    let mut v = list.to_vec();
    let mut out: Option<(usize, Change)> = None;
    for (i, a) in v.iter_mut().enumerate() {
        ui.push_id((scope, i), |ui| {
            if let Some(c) = action_row(ui, i, list.len(), a, cx) {
                out = Some((i, c));
            }
        });
    }
    let mut added = None;
    ui.add_space(4.0);
    ui.menu_button(tr!("Add action"), |ui| {
        for (op, label) in KINDS {
            if ui.button(crate::i18n::t(label)).clicked() {
                added = Action::new(op);
                ui.close();
            }
        }
    });
    if let Some(a) = added {
        v.push(a);
        return Some((v, false));
    }
    let (i, c) = out?;
    let live = match c {
        Change::Edit(live) => live,
        Change::Remove => {
            v.remove(i);
            false
        }
        Change::Up if i > 0 => {
            v.swap(i, i - 1);
            false
        }
        Change::Down if i + 1 < v.len() => {
            v.swap(i, i + 1);
            false
        }
        Change::Up | Change::Down => return None,
    };
    Some((v, live))
}

fn action_row(ui: &mut egui::Ui, i: usize, n: usize, a: &mut Action, cx: &Choices<'_>) -> Option<Change> {
    let mut change = None;
    ui.horizontal(|ui| {
        ui.label(text(Type::DataSmall, format!("{:>2}", i + 1)).color(T.text_faint));
        ui.label(text(Type::Body, crate::i18n::t(a.label())).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button(tr!("Remove")).on_hover_text(tr!("Remove this action")).clicked() {
                change = Some(Change::Remove);
            }
            if i + 1 < n && crate::timeline::icon_button(ui, tr!("Move down"), None, Icon::ChevronDown).clicked() {
                change = Some(Change::Down);
            }
            if i > 0 && crate::timeline::icon_button(ui, tr!("Move up"), None, Icon::ChevronUp).clicked() {
                change = Some(Change::Up);
            }
        });
    });
    let edited = ui.indent("fields", |ui| fields(ui, a, cx)).inner;
    change.or(edited.map(Change::Edit))
}

/// An action's settings. `Some(live)` when one changed.
fn fields(ui: &mut egui::Ui, a: &mut Action, cx: &Choices<'_>) -> Option<bool> {
    let mut changed: Option<bool> = None;
    let main_here = cx.main;
    if let Some(t) = a.target_mut() {
        form_row(ui, FORM_LABEL_DOCK, tr!("Target"), |ui| {
            let shown = t.to_string();
            egui::ComboBox::from_id_salt("target").width(crate::theme::fit(ui, 200.0)).selected_text(shown.clone()).show_ui(ui, |ui| {
                let mut opts: Vec<(String, String)> = vec![("".into(), "this".into())];
                if !main_here {
                    opts.push(("parent".into(), "parent".into()));
                    opts.push(("root".into(), tr!("{name} (main timeline)", name = "root")));
                }
                for (p, k) in &cx.targets {
                    opts.push((p.clone(), if *k == SymbolKind::Button { tr!("{path} · button", path = p) } else { tr!("{path} · movie clip", path = p) }));
                }
                if !opts.iter().any(|(p, _)| Target::new(p.clone()).to_string() == shown) {
                    opts.push((shown.clone(), shown.clone()));
                }
                for (p, label) in opts {
                    let on = Target::new(p.clone()).to_string() == shown;
                    if ui.selectable_label(on, label).clicked() && !on {
                        *t = Target::new(p);
                        changed = Some(false);
                    }
                }
            })
            .response
            .set_name(tr!("Target"));
        });
    }
    let target = a.target().cloned().unwrap_or_default();
    match a {
        Action::GotoAndPlay { frame, scene, .. } | Action::GotoAndStop { frame, scene, .. } => {
            let resolved = cx.timeline_of(&target);
            let labels = cx.labels(&target);
            form_row(ui, FORM_LABEL_DOCK, tr!("Go to"), |ui| {
                let is_label = matches!(frame, FrameRef::Label(_));
                if ui.selectable_label(!is_label, tr!("Frame")).clicked() && is_label {
                    *frame = FrameRef::Number(0);
                    changed = Some(false);
                }
                if ui.selectable_label(is_label, tr!("Label")).on_hover_text(tr!("Frame labels survive frames being added or moved")).clicked() && !is_label {
                    *frame = FrameRef::Label(labels.first().cloned().unwrap_or_else(|| "label".into()));
                    changed = Some(false);
                }
                match frame {
                    FrameRef::Number(n) => {
                        let max = resolved.map_or(anicraft_engine::MAX_FRAME, |(tl, _)| tl.frame_count());
                        let mut shown = *n + 1;
                        if with_type(ui, Type::Data, |ui| ui.add(egui::DragValue::new(&mut shown).range(1..=max.max(1)).speed(0.2))).named(tr!("Frame number")).changed() {
                            *n = shown.saturating_sub(1);
                            changed = Some(true);
                        }
                    }
                    FrameRef::Label(l) => {
                        egui::ComboBox::from_id_salt("label").width(crate::theme::fit(ui, 140.0)).selected_text(l.as_str()).show_ui(ui, |ui| {
                            if labels.is_empty() {
                                ui.label(text(Type::Caption, tr!("No frame labels yet: name a keyframe in Properties › Label")));
                            }
                            for x in &labels {
                                if ui.selectable_label(x == l, x.as_str()).clicked() && x != l {
                                    *l = x.clone();
                                    changed = Some(false);
                                }
                            }
                        })
                        .response
                        .set_name(tr!("Frame label"));
                    }
                }
            });
            if resolved.is_some_and(|(_, main)| main) && cx.doc.scenes.len() > 1 {
                form_row(ui, FORM_LABEL_DOCK, tr!("Scene"), |ui| {
                    let shown = scene.clone().unwrap_or_else(|| tr!("This scene").into());
                    egui::ComboBox::from_id_salt("scene").width(crate::theme::fit(ui, 160.0)).selected_text(shown).show_ui(ui, |ui| {
                        if ui.selectable_label(scene.is_none(), tr!("This scene")).clicked() && scene.is_some() {
                            *scene = None;
                            changed = Some(false);
                        }
                        for s in &cx.doc.scenes {
                            if ui.selectable_label(scene.as_deref() == Some(s.name.as_str()), s.name.as_str()).clicked() {
                                *scene = Some(s.name.clone());
                                changed = Some(false);
                            }
                        }
                    })
                    .response
                    .set_name(tr!("Scene"));
                });
            }
        }
        Action::Trace { message } => {
            form_row(ui, FORM_LABEL_DOCK, tr!("Message"), |ui| {
                if ui.add(egui::TextEdit::singleline(message).desired_width(crate::theme::fit(ui, 220.0)).hint_text(tr!("written to Output"))).named(tr!("Trace message")).changed() {
                    changed = Some(true);
                }
            });
        }
        Action::SetProperty { property, value, .. } => {
            form_row(ui, FORM_LABEL_DOCK, tr!("Property"), |ui| {
                egui::ComboBox::from_id_salt("prop").width(crate::theme::fit(ui, 110.0)).selected_text(crate::i18n::t(property.label())).show_ui(ui, |ui| {
                    for p in Property::ALL {
                        if ui.selectable_label(*property == p, crate::i18n::t(p.label())).clicked() && *property != p {
                            *property = p;
                            *value = match p {
                                Property::Alpha | Property::Visible | Property::ScaleX | Property::ScaleY => 1.0,
                                _ => 0.0,
                            };
                            changed = Some(false);
                        }
                    }
                })
                .response
                .set_name(tr!("Property"));
                match property {
                    Property::Visible => {
                        let mut on = *value != 0.0;
                        if crate::theme::checkbox(ui, &mut on, tr!("Shown")).changed() {
                            *value = if on { 1.0 } else { 0.0 };
                            changed = Some(false);
                        }
                    }
                    Property::Alpha => {
                        let mut pct = *value * 100.0;
                        if with_type(ui, Type::Data, |ui| ui.add(egui::DragValue::new(&mut pct).range(0.0..=100.0).suffix(" %").speed(0.5).max_decimals(0))).named(tr!("Alpha")).changed() {
                            *value = pct / 100.0;
                            changed = Some(true);
                        }
                    }
                    p => {
                        let (lo, hi) = p.range();
                        let suffix = match p {
                            Property::Rotation => "°",
                            Property::X | Property::Y => " px",
                            _ => " ×",
                        };
                        if with_type(ui, Type::Data, |ui| ui.add(egui::DragValue::new(value).range(lo..=hi).suffix(suffix).speed(0.5).max_decimals(2))).named(crate::i18n::t(p.label())).changed() {
                            changed = Some(true);
                        }
                    }
                }
            });
        }
        Action::Stop { .. } | Action::Play { .. } | Action::NextFrame { .. } | Action::PrevFrame { .. } | Action::NextScene | Action::PrevScene => {}
    }
    changed
}
