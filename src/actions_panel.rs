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

/// Opens the panel on a tab.
pub fn open(app: &mut App, tab: Tab) {
    app.show_actions = true;
    app.actions.tab = tab;
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    if !app.show_actions {
        return;
    }
    let mut open = app.show_actions;
    let at = ctx.content_rect().right_top() + egui::vec2(-760.0, 96.0);
    let shown = egui::Window::new("Actions").id(egui::Id::new("actions_panel")).open(&mut open).default_pos(at).default_width(420.0).resizable(true).collapsible(false).show(ctx, |ui| {
        egui::ScrollArea::vertical().max_height(ctx.content_rect().height() * 0.7).auto_shrink([false, true]).show(ui, |ui| panel(app, ui));
    });
    crate::a11y::name_title_bar(&shown, "Actions");
    app.show_actions = open;
}

/// The selected movie clip or button instance on the active layer, if exactly one is selected.
fn selected_instance(app: &App) -> Option<(Element, SymbolKind)> {
    let [id] = app.s.selection.as_slice() else { return None };
    let e = app.s.visible_elements().into_iter().find(|e| e.id == *id)?;
    let ElementKind::Symbol { symbol, .. } = &e.kind else { return None };
    let kind = app.s.doc.symbol(*symbol)?.kind;
    (kind != SymbolKind::Graphic).then(|| ((*e).clone(), kind))
}

fn panel(app: &mut App, ui: &mut egui::Ui) {
    let inst = selected_instance(app);
    ui.horizontal(|ui| {
        if ui.selectable_label(app.actions.tab == Tab::Frame, "Frame").on_hover_text("What runs when the playhead enters a keyframe").clicked() {
            app.actions.tab = Tab::Frame;
        }
        let label = match &inst {
            Some((e, k)) => format!("{} {}", if *k == SymbolKind::Button { "Button" } else { "Movie clip" }, if e.name.is_empty() { "(unnamed)".to_string() } else { format!("“{}”", e.name) }),
            None => "Instance".to_string(),
        };
        let r = ui.add_enabled(inst.is_some(), egui::Button::selectable(app.actions.tab == Tab::Instance, label)).on_hover_text("What the selected button or movie clip does on clicks and the pointer");
        let r = r.on_disabled_hover_text("Select one button or movie clip instance on the stage");
        if r.clicked() {
            app.actions.tab = Tab::Instance;
        }
    });
    ui.separator();
    match (app.actions.tab, inst) {
        (Tab::Instance, Some((e, _))) => instance_tab(app, ui, &e),
        (Tab::Instance, None) => {
            ui.label(text(Type::Caption, "Select one button or movie clip instance on the stage to give it events (a graphic can't have them: set its symbol's Type in Properties)."));
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

/// Kept scripts in the document (ADR 0013 tier 1), in every scene and symbol.
pub fn kept_scripts(doc: &Document) -> usize {
    doc.timelines().flat_map(|tl| tl.layers.iter()).flat_map(|l| l.keyframes.iter()).filter(|k| is_kept(k)).count()
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
    let mut edit = egui::TextEdit::singleline(&mut draft).desired_width(170.0).font(font(Type::Path)).hint_text("none");
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

fn kind_name(k: SymbolKind) -> &'static str {
    match k {
        SymbolKind::MovieClip => "movie clip",
        SymbolKind::Button => "button",
        SymbolKind::Graphic => "graphic",
    }
}

fn frame_tab(app: &mut App, ui: &mut egui::Ui) {
    let doc = app.s.doc.clone();
    let scene_name = doc.scenes.get(app.s.scene).map_or_else(|| "Scene".to_string(), |s| s.name.clone());
    // Which timeline: the scene being edited, or a symbol's.
    let symbols = symbol_timelines(&doc);
    if app.actions.symbol.is_some_and(|id| !symbols.iter().any(|(c, _, _)| *c == id)) {
        app.actions.symbol = None;
    }
    let current = app.actions.symbol.and_then(|id| symbols.iter().find(|(c, _, _)| *c == id)).map_or(scene_name.clone(), |(_, n, k)| format!("{n} ({})", kind_name(*k)));
    form_row(ui, FORM_LABEL_DOCK, "Timeline", |ui| {
        egui::ComboBox::from_id_salt("act_timeline").width(220.0).selected_text(current).show_ui(ui, |ui| {
            if ui.selectable_label(app.actions.symbol.is_none(), format!("{scene_name} (main timeline)")).clicked() {
                app.actions.symbol = None;
            }
            for (id, n, k) in &symbols {
                if ui.selectable_label(app.actions.symbol == Some(*id), format!("{n} ({})", kind_name(*k))).clicked() {
                    app.actions = State { tab: Tab::Frame, symbol: Some(*id), ..State::default() };
                }
            }
        })
        .response
        .set_name("Timeline");
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
        ui.label(text(Type::Caption, "Frames here don't run (graphic and button timelines, guide layers); the script is kept with the document."));
        script_view(app, ui, &t, source.as_ref());
        return;
    }
    let mut to = None;
    form_row(ui, FORM_LABEL_DOCK, "Type", |ui| {
        for (k, label, hint) in [
            (Holds::Actions, "Actions", "A list of actions, in order"),
            (Holds::Plugin, "Plug-in", "An entry point of a frame-script plug-in (WebAssembly)"),
            (Holds::Script, "Script", "Code in a scripting language (such as an Animate document's ActionScript), kept with the document. It doesn't run yet"),
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
                ui.label(text(Type::Caption, "No actions yet. Add one: Stop on a frame holds the movie there; a button's Click can then go on."));
            }
            if let Some((new, live)) = edit_list(ui, "frame", &list, &cx) {
                let mut p = t.base.clone();
                p["actions"] = serde_json::to_value(&new).unwrap_or(Value::Null);
                run(app, live, "Edit actions", "actions.set", p);
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
                app.status_ok("Frame script set — Ctrl+Z to undo");
            } else {
                app.status_err("No frame-script plug-ins are installed (Commands › Install Plug-in…)");
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
        let what = match from {
            Holds::Actions => "the actions",
            Holds::Plugin => "the plug-in frame script",
            Holds::Script => "the script",
        };
        app.status_ok(format!("Removed {what} — Ctrl+Z to undo"));
    }
}

/// Space above the first line of the script editor (and its line numbers).
const EDITOR_MARGIN: i8 = 4;

/// The Script view: the kept script's language, and its text in a monospace editor with line
/// numbers. Typing edits the document live (`script.set`, one undo step per burst of typing).
fn script_view(app: &mut App, ui: &mut egui::Ui, t: &FrameTarget, src: Option<&SourceScript>) {
    let language = src.map_or(Language::default(), |s| s.language);
    let mut pick = None;
    form_row(ui, FORM_LABEL_DOCK, "Language", |ui| {
        ui.add_enabled_ui(src.is_some(), |ui| {
            egui::ComboBox::from_id_salt("act_language").width(180.0).selected_text(language.label()).show_ui(ui, |ui| {
                for l in Language::ALL {
                    if ui.selectable_label(l == language, l.label()).clicked() && l != language {
                        pick = Some(l);
                    }
                }
            })
            .response
            .set_name("Script language");
        });
        if let Some(s) = src {
            let n = s.lines();
            ui.label(text(Type::Data, format!("{n} {}", if n == 1 { "line" } else { "lines" })).color(T.text_dim));
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
    let changed = egui::ScrollArea::both()
        .id_salt("act_script")
        .max_height(320.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                // Line numbers, right-aligned in a gutter, on the same rows as the text.
                let digits = rows.to_string().len().max(2);
                let numbers: String = (1..=rows).map(|i| format!("{i:>digits$}")).collect::<Vec<_>>().join("\n");
                ui.vertical(|ui| {
                    ui.add_space(f32::from(EDITOR_MARGIN));
                    ui.add(egui::Label::new(text(Type::Path, numbers).color(T.text_faint)).selectable(false));
                });
                let editor = egui::TextEdit::multiline(&mut edited)
                    .code_editor()
                    .font(code.clone())
                    .margin(egui::Margin::symmetric(6, EDITOR_MARGIN))
                    .desired_rows(12)
                    .desired_width(f32::INFINITY)
                    .hint_text("// Type or paste a frame script");
                ui.add(editor).named("Frame script").changed()
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
        run(app, true, "Edit script", "script.set", p);
    }
    ui.label(text(Type::Caption, "Kept with the document as written. AniCraft doesn't run scripts yet: Test Movie and exports skip it. Use actions for what should happen while the movie plays."));
    if src.is_some() && ui.small_button("Remove script").on_hover_text("Delete this keyframe's script").clicked() {
        app.run("script.clear", t.base.clone());
        app.status_ok("Removed the script — Ctrl+Z to undo");
    }
}

/// The keyframe the Frame tab edits (and the pickers that choose it), or `None` when there is
/// none.
fn frame_target(app: &mut App, ui: &mut egui::Ui, doc: &Document) -> Option<FrameTarget> {
    match app.actions.symbol {
        None => {
            let tl = doc.timeline(app.s.scene);
            let layer = tl.layer(app.s.layer)?;
            let frame = app.s.frame;
            form_row(ui, FORM_LABEL_DOCK, "Keyframe", |ui| {
                ui.label(text(Type::Data, format!("{}", frame + 1)));
                ui.label(text(Type::Body, layer.name.as_str()).color(T.text_dim));
            });
            if layer.kind == LayerKind::Folder || (layer.kind == LayerKind::Guide && !layer.keyframe_at(frame).is_some_and(is_kept)) {
                ui.label(text(Type::Caption, "Folders and guide layers don't run actions. Pick another layer in the timeline."));
                return None;
            }
            let Some(k) = layer.keyframe_at(frame) else {
                let key = app.keymap.primary("timeline.insert_blank_keyframe").map(crate::platform::shortcut_label).unwrap_or_default();
                ui.label(text(Type::Caption, format!("Frame {} is empty on this layer. Add a blank keyframe ({key}) to give it actions.", frame + 1)));
                return None;
            };
            if k.start != frame {
                let key = app.keymap.primary("timeline.insert_keyframe").map(crate::platform::shortcut_label).unwrap_or_default();
                ui.label(text(Type::Caption, format!("These run when the playhead enters frame {} (the keyframe). For frame {}, add a keyframe there ({key}).", k.start + 1, frame + 1)));
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
            form_row(ui, FORM_LABEL_DOCK, "Layer", |ui| {
                egui::ComboBox::from_id_salt("act_layer").width(180.0).selected_text(layer.name.as_str()).show_ui(ui, |ui| {
                    for (id, n) in &layers {
                        if ui.selectable_label(*id == lid, n.as_str()).clicked() {
                            app.actions.layer = Some(*id);
                        }
                    }
                })
                .response
                .set_name("Layer");
            });
            let k = layer.keyframe_at(app.actions.frame).or_else(|| layer.keyframes.first().map(|k| &**k))?;
            app.actions.frame = k.start;
            let shown = |k: &anicraft_engine::doc::Keyframe| if k.label.is_empty() { format!("{}", k.start + 1) } else { format!("{} “{}”", k.start + 1, k.label) };
            form_row(ui, FORM_LABEL_DOCK, "Keyframe", |ui| {
                with_type(ui, Type::Data, |ui| {
                    egui::ComboBox::from_id_salt("act_key").width(180.0).selected_text(shown(k)).show_ui(ui, |ui| {
                        for kk in &layer.keyframes {
                            let mark = if kk.script.is_some() { " ·a" } else { "" };
                            if ui.selectable_label(kk.start == k.start, format!("{}{mark}", shown(kk))).clicked() {
                                app.actions.frame = kk.start;
                            }
                        }
                    })
                    .response
                    .set_name("Keyframe");
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
    form_row(ui, FORM_LABEL_DOCK, "Plug-in", |ui| {
        egui::ComboBox::from_id_salt("act_plugin").width(220.0).selected_text(shown).show_ui(ui, |ui| {
            for (pid, name, entries) in &all {
                if ui.selectable_label(pid == id, name.as_str()).on_hover_text(pid.as_str()).clicked() {
                    pick = Some((pid.clone(), entries.first().cloned().unwrap_or_else(|| "main".into())));
                }
            }
        })
        .response
        .set_name("Plug-in");
    });
    let entries = installed.map(|(_, _, e)| e.clone()).unwrap_or_default();
    form_row(ui, FORM_LABEL_DOCK, "Entry", |ui| {
        with_type(ui, Type::Data, |ui| {
            egui::ComboBox::from_id_salt("act_entry").width(160.0).selected_text(entry).show_ui(ui, |ui| {
                for e in &entries {
                    if ui.selectable_label(e == entry, e.as_str()).clicked() {
                        pick = Some((id.to_string(), e.clone()));
                    }
                }
            })
            .response
            .set_name("Entry point");
        });
    });
    if installed.is_none() {
        ui.label(text(Type::Caption, format!("“{id}” isn't installed here: Test Movie says so and plays on.")).color(T.error));
    }
    ui.label(text(Type::Caption, "The plug-in runs each time the playhead enters this keyframe, reads where the movie is, and sends actions back. It can't change the document."));
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
    form_row(ui, FORM_LABEL_DOCK, "Name", |ui| {
        if ui.add(egui::TextEdit::singleline(&mut name).desired_width(200.0).hint_text("instance name")).named("Instance name").changed() {
            run(app, true, "Instance name", "instance.set_name", json!({"element": e.id.0, "name": name}));
        }
    });
    if e.name.is_empty() {
        ui.label(text(Type::Caption, "Name it to address it from actions (letters, digits and _). Its events work without a name."));
    }
    let ElementKind::Symbol { events, .. } = &e.kind else { return };
    let tl = doc.timeline(app.s.scene).clone();
    let cx = Choices::new(&doc, &tl, true);
    ui.add_space(4.0);
    ui.label(text(Type::Caption, "Its actions run on the timeline it is on: “this” is the main timeline."));
    for h in events {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(text(Type::Body, format!("On {}", h.event.label().to_lowercase())).strong()).on_hover_text(h.event.hint());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Remove event").clicked() {
                    app.run("actions.clear", json!({"element": e.id.0, "event": h.event.key()}));
                    app.status_ok(format!("Removed the {} event — Ctrl+Z to undo", h.event.label().to_lowercase()));
                }
            });
        });
        if let Some((new, live)) = edit_list(ui, h.event.key(), &h.actions, &cx) {
            run(app, live, "Edit event", "actions.set", json!({"element": e.id.0, "event": h.event.key(), "actions": new}));
        }
    }
    let unused: Vec<ButtonEvent> = ButtonEvent::ALL.into_iter().filter(|ev| !events.iter().any(|h| h.event == *ev)).collect();
    if !unused.is_empty() {
        ui.add_space(6.0);
        ui.menu_button("Add event", |ui| {
            for ev in unused {
                if ui.button(ev.label()).on_hover_text(ev.hint()).clicked() {
                    // A new event starts with the action people want most: go on playing.
                    let first = Action::Play { target: Target::this() };
                    app.run("actions.set", json!({"element": e.id.0, "event": ev.key(), "actions": [first]}));
                    ui.close();
                }
            }
        });
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
    ui.menu_button("Add action", |ui| {
        for (op, label) in KINDS {
            if ui.button(label).clicked() {
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
        ui.label(text(Type::Body, a.label()).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("Remove").on_hover_text("Remove this action").clicked() {
                change = Some(Change::Remove);
            }
            if i + 1 < n && crate::timeline::icon_button(ui, "Move down", None, Icon::ChevronDown).clicked() {
                change = Some(Change::Down);
            }
            if i > 0 && crate::timeline::icon_button(ui, "Move up", None, Icon::ChevronUp).clicked() {
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
        form_row(ui, FORM_LABEL_DOCK, "Target", |ui| {
            let shown = t.to_string();
            egui::ComboBox::from_id_salt("target").width(200.0).selected_text(shown.clone()).show_ui(ui, |ui| {
                let mut opts: Vec<(String, String)> = vec![("".into(), "this".into())];
                if !main_here {
                    opts.push(("parent".into(), "parent".into()));
                    opts.push(("root".into(), "root (main timeline)".into()));
                }
                for (p, k) in &cx.targets {
                    opts.push((p.clone(), format!("{p} · {}", if *k == SymbolKind::Button { "button" } else { "movie clip" })));
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
            .set_name("Target");
        });
    }
    let target = a.target().cloned().unwrap_or_default();
    match a {
        Action::GotoAndPlay { frame, scene, .. } | Action::GotoAndStop { frame, scene, .. } => {
            let resolved = cx.timeline_of(&target);
            let labels = cx.labels(&target);
            form_row(ui, FORM_LABEL_DOCK, "Go to", |ui| {
                let is_label = matches!(frame, FrameRef::Label(_));
                if ui.selectable_label(!is_label, "Frame").clicked() && is_label {
                    *frame = FrameRef::Number(0);
                    changed = Some(false);
                }
                if ui.selectable_label(is_label, "Label").on_hover_text("Frame labels survive frames being added or moved").clicked() && !is_label {
                    *frame = FrameRef::Label(labels.first().cloned().unwrap_or_else(|| "label".into()));
                    changed = Some(false);
                }
                match frame {
                    FrameRef::Number(n) => {
                        let max = resolved.map_or(anicraft_engine::MAX_FRAME, |(tl, _)| tl.frame_count());
                        let mut shown = *n + 1;
                        if with_type(ui, Type::Data, |ui| ui.add(egui::DragValue::new(&mut shown).range(1..=max.max(1)).speed(0.2))).named("Frame number").changed() {
                            *n = shown.saturating_sub(1);
                            changed = Some(true);
                        }
                    }
                    FrameRef::Label(l) => {
                        egui::ComboBox::from_id_salt("label").width(140.0).selected_text(l.as_str()).show_ui(ui, |ui| {
                            if labels.is_empty() {
                                ui.label(text(Type::Caption, "No frame labels yet: name a keyframe in Properties › Label"));
                            }
                            for x in &labels {
                                if ui.selectable_label(x == l, x.as_str()).clicked() && x != l {
                                    *l = x.clone();
                                    changed = Some(false);
                                }
                            }
                        })
                        .response
                        .set_name("Frame label");
                    }
                }
            });
            if resolved.is_some_and(|(_, main)| main) && cx.doc.scenes.len() > 1 {
                form_row(ui, FORM_LABEL_DOCK, "Scene", |ui| {
                    let shown = scene.clone().unwrap_or_else(|| "This scene".into());
                    egui::ComboBox::from_id_salt("scene").width(160.0).selected_text(shown).show_ui(ui, |ui| {
                        if ui.selectable_label(scene.is_none(), "This scene").clicked() && scene.is_some() {
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
                    .set_name("Scene");
                });
            }
        }
        Action::Trace { message } => {
            form_row(ui, FORM_LABEL_DOCK, "Message", |ui| {
                if ui.add(egui::TextEdit::singleline(message).desired_width(220.0).hint_text("written to Output")).named("Trace message").changed() {
                    changed = Some(true);
                }
            });
        }
        Action::SetProperty { property, value, .. } => {
            form_row(ui, FORM_LABEL_DOCK, "Property", |ui| {
                egui::ComboBox::from_id_salt("prop").width(110.0).selected_text(property.label()).show_ui(ui, |ui| {
                    for p in Property::ALL {
                        if ui.selectable_label(*property == p, p.label()).clicked() && *property != p {
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
                .set_name("Property");
                match property {
                    Property::Visible => {
                        let mut on = *value != 0.0;
                        if crate::theme::checkbox(ui, &mut on, "Shown").changed() {
                            *value = if on { 1.0 } else { 0.0 };
                            changed = Some(false);
                        }
                    }
                    Property::Alpha => {
                        let mut pct = *value * 100.0;
                        if with_type(ui, Type::Data, |ui| ui.add(egui::DragValue::new(&mut pct).range(0.0..=100.0).suffix(" %").speed(0.5).max_decimals(0))).named("Alpha").changed() {
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
                        if with_type(ui, Type::Data, |ui| ui.add(egui::DragValue::new(value).range(lo..=hi).suffix(suffix).speed(0.5).max_decimals(2))).named(p.label()).changed() {
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
