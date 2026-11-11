//! Edit › Find and Replace (ADR 0058): a window that stays open while you work. What to find
//! (text, a font, a colour, a symbol, a sound or a bitmap), where (this scene and the symbols it
//! uses, or the whole document), and the results listed as you type; clicking one, Find Next
//! (F3) or Find Previous (Shift+F3) goes there (`find.goto`); Replace and Replace All are
//! `find.replace`, one undo step each. The list draws only the rows in view.

use crate::a11y::{self, Named};
use crate::app::App;
use crate::theme::{T, TARGET, Type, color, font, text};
use anicraft_engine::StateId;
use anicraft_engine::find::{self, Field, Hit, Kind, Query, TextOptions, Where};
use egui::{Align2, Rect, Sense, WidgetType, pos2, vec2};
use serde_json::{Value, json};
use std::sync::Arc;

/// The window's fields and results (view state).
pub struct FindState {
    pub open: bool,
    /// Put the keyboard in the Find field on the next frame.
    pub focus: bool,
    pub kind: Kind,
    /// Search the whole document (off: this scene and the symbols it uses).
    pub document: bool,
    pub text: String,
    pub with: String,
    pub opts: TextOptions,
    pub within: find::Within,
    pub color: [u8; 3],
    pub with_color: [u8; 3],
    pub font: String,
    pub with_font: String,
    /// A Library id (symbol, sound or bitmap) to find, and to replace with.
    pub item: Option<u64>,
    pub with_item: Option<u64>,
    /// The result the last Find Next or click went to.
    pub current: Option<usize>,
    results: Option<Results>,
}

impl Default for FindState {
    fn default() -> Self {
        FindState {
            open: false,
            focus: false,
            kind: Kind::Text,
            document: false,
            text: String::new(),
            with: String::new(),
            opts: TextOptions::default(),
            within: find::Within::default(),
            color: [0, 0, 0],
            with_color: [255, 255, 255],
            font: String::new(),
            with_font: String::new(),
            item: None,
            with_item: None,
            current: None,
            results: None,
        }
    }
}

/// The hits, and whether the list stopped at its limit; or why the query isn't one.
type Found = Result<(Arc<Vec<Hit>>, bool), String>;

/// A search's results and what they were found for.
struct Results {
    key: (String, StateId, usize),
    found: Found,
}

impl FindState {
    /// The query as `find.search` params (`with` added for `find.replace`).
    pub fn params(&self) -> Value {
        let hex = |c: [u8; 3]| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]);
        let mut p = json!({"kind": self.kind.key(), "scope": if self.document { "document" } else { "scene" }});
        let w = self.within;
        match self.kind {
            Kind::Text => {
                p["text"] = json!(self.text);
                p["match_case"] = json!(self.opts.match_case);
                p["whole_word"] = json!(self.opts.whole_word);
                p["regex"] = json!(self.opts.regex);
                let places: Vec<&str> = [(w.text, "text"), (w.labels, "labels"), (w.layers, "layers"), (w.instances, "instances"), (w.scripts, "scripts")].into_iter().filter(|(on, _)| *on).map(|(_, k)| k).collect();
                p["in"] = json!(places);
                p["with"] = json!(self.with);
            }
            Kind::Color => {
                p["color"] = json!(hex(self.color));
                let places: Vec<&str> = [(w.fills, "fills"), (w.strokes, "strokes"), (w.text_color, "text")].into_iter().filter(|(on, _)| *on).map(|(_, k)| k).collect();
                p["in"] = json!(places);
                p["with"] = json!(hex(self.with_color));
            }
            Kind::Font => {
                p["font"] = json!(self.font);
                p["with"] = json!(self.with_font);
            }
            Kind::Symbol | Kind::Sound | Kind::Bitmap => {
                p[self.kind.key()] = json!(self.item);
                p["with"] = json!(self.with_item);
            }
        }
        p
    }

    /// Whether there is something to look for yet (an empty field shows a hint, not an error).
    fn ready(&self) -> bool {
        match self.kind {
            Kind::Text => !self.text.is_empty(),
            Kind::Font => !self.font.trim().is_empty(),
            Kind::Color => true,
            Kind::Symbol | Kind::Sound | Kind::Bitmap => self.item.is_some(),
        }
    }
}

/// The results for the window's query, searched again when the query or the document changes.
fn results(app: &mut App) -> Option<&Found> {
    if !app.find.ready() {
        app.find.results = None;
        return None;
    }
    let mut p = app.find.params();
    if let Some(o) = p.as_object_mut() {
        o.remove("with");
    }
    let key = (p.to_string(), app.s.state, app.s.scene);
    if app.find.results.as_ref().is_none_or(|r| r.key != key) {
        let found = Query::from_params(&app.s, &p).map(|q| find::search(&app.s.doc, app.s.scene, &q)).map(|(h, t)| (Arc::new(h), t)).map_err(|e| crate::i18n::t_dyn(&e.to_string()).into_owned());
        app.find.results = Some(Results { key, found });
    }
    app.find.results.as_ref().map(|r| &r.found)
}

fn hits(app: &mut App) -> Arc<Vec<Hit>> {
    match results(app) {
        Some(Ok((h, _))) => h.clone(),
        _ => Arc::default(),
    }
}

/// Edit › Find and Replace (Ctrl+F): opens the window, the Find field ready for typing.
pub fn open(app: &mut App) {
    app.find.open = true;
    app.find.focus = true;
}

/// Goes to result `i` and remembers it.
fn go(app: &mut App, i: usize, list: &[Hit]) {
    let Some(h) = list.get(i) else { return };
    app.playing = false;
    if app.run("find.goto", h.to_json(&app.s.doc, app.find.kind)).is_some() {
        app.find.current = Some(i);
        app.status_ok(tr!("Result {n} of {total}", n = i + 1, total = list.len()));
    }
}

/// Find Next (F3) or Find Previous (Shift+F3): the next result after the current one, round
/// to the first.
pub fn step(app: &mut App, forward: bool) {
    let list = hits(app);
    if list.is_empty() {
        if !app.find.open {
            open(app);
        }
        app.status_ok(if app.find.ready() { tr!("Nothing found") } else { tr!("Type what to find in Find and Replace") });
        return;
    }
    let n = list.len();
    let i = match app.find.current {
        Some(c) if forward => (c + 1) % n,
        Some(c) => (c + n - 1) % n,
        None if forward => 0,
        None => n - 1,
    };
    go(app, i, &list);
}

/// Replace: the current result (the first when none is), then on to the next.
fn replace_one(app: &mut App) {
    let list = hits(app);
    let i = app.find.current.filter(|i| *i < list.len()).unwrap_or(0);
    let Some(h) = list.get(i) else { return };
    let mut p = app.find.params();
    p["at"] = h.to_json(&app.s.doc, app.find.kind);
    if app.perform("find.replace", p).is_some() {
        // The replaced one is gone (or no longer matches): the one now at its place is next.
        let after = hits(app);
        if after.is_empty() {
            app.find.current = None;
        } else {
            go(app, i.min(after.len() - 1), &after);
        }
    }
}

fn kind_label(k: Kind) -> &'static str {
    match k {
        Kind::Text => tr_noop!("Text"),
        Kind::Font => tr_noop!("Font"),
        Kind::Color => tr_noop!("Colour"),
        Kind::Symbol => tr_noop!("Symbol"),
        Kind::Sound => tr_noop!("Sound"),
        Kind::Bitmap => tr_noop!("Bitmap"),
    }
}

fn field_label(f: Field) -> &'static str {
    match f {
        Field::Text => tr_noop!("Text field"),
        Field::Label => tr_noop!("Frame label"),
        Field::LayerName => tr_noop!("Layer name"),
        Field::InstanceName => tr_noop!("Instance name"),
        Field::Script => tr_noop!("Script"),
        Field::Font => tr_noop!("Font"),
        Field::Fill => tr_noop!("Fill"),
        Field::Stroke => tr_noop!("Stroke"),
        Field::TextColor => tr_noop!("Text colour"),
        Field::Instance => tr_noop!("Instance"),
        Field::Sound => tr_noop!("Sound"),
        Field::Bitmap => tr_noop!("Bitmap"),
    }
}

/// Where a result is, for its row: the scene or symbol, the layer and the frame.
fn location(app: &App, h: &Hit) -> String {
    let doc = &app.s.doc;
    // Scene, symbol and layer names are document text, as written.
    let place = match h.at {
        Where::Scene(i) => doc.scenes.get(i).map_or_else(String::new, |s| s.name.clone()),
        Where::Symbol(id) => doc.symbol(id).map_or_else(String::new, |s| s.name.clone()),
    };
    let layer = find::timeline_of(doc, h.at).and_then(|t| t.layer(h.layer)).map_or_else(String::new, |l| l.name.clone());
    match h.frame {
        Some(f) => tr!("{place} › {layer}, frame {frame}", place = place, layer = layer, frame = f + 1),
        None => tr!("{place} › {layer}", place = place, layer = layer),
    }
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    if !app.find.open {
        return;
    }
    let mut open = app.find.open;
    let at = ctx.content_rect().right_top() + vec2(-720.0, 110.0);
    let shown = egui::Window::new(tr!("Find and Replace")).id(egui::Id::new("find_replace")).open(&mut open).default_pos(at).default_width(400.0).resizable(true).collapsible(false).show(ctx, |ui| panel(app, ui));
    a11y::name_title_bar(&shown, tr!("Find and Replace"));
    app.find.open = open;
}

/// The query's "find" and "replace with" rows for the kind chosen.
fn query_rows(app: &mut App, ui: &mut egui::Ui) -> bool {
    let mut enter = false;
    let label_w = 92.0;
    let row = |ui: &mut egui::Ui, label: &str, add: &mut dyn FnMut(&mut egui::Ui)| {
        ui.horizontal(|ui| {
            ui.add_sized(vec2(label_w, TARGET), egui::Label::new(text(Type::Label, label)));
            add(ui);
        });
    };
    let w = (ui.available_width() - label_w - 8.0).max(120.0);
    match app.find.kind {
        Kind::Text => {
            row(ui, tr!("Find"), &mut |ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut app.find.text).desired_width(w).hint_text(tr!("Text to find"))).named(tr!("Text to find"));
                if app.find.focus {
                    r.request_focus();
                    app.find.focus = false;
                }
                enter |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            });
            row(ui, tr!("Replace with"), &mut |ui| {
                ui.add(egui::TextEdit::singleline(&mut app.find.with).desired_width(w).hint_text(if app.find.opts.regex { tr!("$1 puts back a group") } else { tr!("Replacement text") })).set_name(tr!("Replace with"));
            });
        }
        Kind::Font => {
            let used = doc_fonts(&app.s.doc);
            let families = app.font_families.get_or_insert_with(|| anicraft_engine::text::Fonts::get().families().to_vec()).clone();
            row(ui, tr!("Find"), &mut |ui| {
                font_combo(ui, "find_font", &mut app.find.font, &used, tr!("Font to find"), w);
            });
            row(ui, tr!("Replace with"), &mut |ui| {
                font_combo(ui, "with_font", &mut app.find.with_font, &families, tr!("Font to replace with"), w);
            });
        }
        Kind::Color => {
            row(ui, tr!("Find"), &mut |ui| {
                ui.color_edit_button_srgb(&mut app.find.color).named(tr!("Colour to find"));
                ui.label(text(Type::Data, format!("#{:02x}{:02x}{:02x}", app.find.color[0], app.find.color[1], app.find.color[2])));
            });
            row(ui, tr!("Replace with"), &mut |ui| {
                ui.color_edit_button_srgb(&mut app.find.with_color).named(tr!("Colour to replace with"));
                ui.label(text(Type::Data, format!("#{:02x}{:02x}{:02x}", app.find.with_color[0], app.find.with_color[1], app.find.with_color[2])));
            });
        }
        Kind::Symbol | Kind::Sound | Kind::Bitmap => {
            let items = library_items(app, app.find.kind);
            row(ui, tr!("Find"), &mut |ui| item_combo(ui, "find_item", &mut app.find.item, &items, tr!("Item to find"), w));
            row(ui, tr!("Replace with"), &mut |ui| item_combo(ui, "with_item", &mut app.find.with_item, &items, tr!("Item to replace with"), w));
        }
    }
    enter
}

/// The fonts text in the document uses, A to Z.
fn doc_fonts(doc: &anicraft_engine::doc::Document) -> Vec<String> {
    let mut v: Vec<String> = doc.timelines().flat_map(|t| &t.layers).flat_map(|l| &l.keyframes).flat_map(|k| &k.elements).filter_map(|e| match &e.kind {
        anicraft_engine::doc::ElementKind::Text(t) => Some(t.font.clone()),
        _ => None,
    }).collect();
    v.sort_unstable();
    v.dedup();
    v
}

fn font_combo(ui: &mut egui::Ui, id: &str, value: &mut String, options: &[String], name: &str, w: f32) {
    let shown = if value.is_empty() { tr!("Choose a font").to_string() } else { value.clone() };
    egui::ComboBox::from_id_salt(id).width(w).selected_text(shown).show_ui(ui, |ui| {
        if options.is_empty() {
            ui.label(text(Type::Caption, tr!("No text uses a font yet")));
        }
        for f in options.iter().take(2_000) {
            if ui.selectable_label(f == value, f.as_str()).clicked() {
                value.clone_from(f);
            }
        }
    })
    .response
    .set_name(name);
}

/// The Library's items of `kind`: id and name (document text).
fn library_items(app: &App, kind: Kind) -> Vec<(u64, String)> {
    let d = &app.s.doc;
    match kind {
        Kind::Symbol => d.library.iter().map(|s| (s.id.0, s.name.clone())).collect(),
        Kind::Sound => d.sounds.iter().map(|s| (s.id.0, s.name.clone())).collect(),
        Kind::Bitmap => d.bitmaps.iter().map(|b| (b.id.0, b.name.clone())).collect(),
        _ => Vec::new(),
    }
}

fn item_combo(ui: &mut egui::Ui, id: &str, value: &mut Option<u64>, items: &[(u64, String)], name: &str, w: f32) {
    let shown = value.and_then(|v| items.iter().find(|(i, _)| *i == v)).map_or_else(|| tr!("Choose from the Library").to_string(), |(_, n)| n.clone());
    egui::ComboBox::from_id_salt(id).width(w).selected_text(shown).show_ui(ui, |ui| {
        if items.is_empty() {
            ui.label(text(Type::Caption, tr!("The Library has none")));
        }
        for (i, n) in items {
            if ui.selectable_label(*value == Some(*i), n.as_str()).clicked() {
                *value = Some(*i);
            }
        }
    })
    .response
    .set_name(name);
}

fn panel(app: &mut App, ui: &mut egui::Ui) {
    // What to find, and where.
    ui.horizontal(|ui| {
        ui.label(text(Type::Label, tr!("Search for")));
        let before = app.find.kind;
        egui::ComboBox::from_id_salt("find_kind").width(110.0).selected_text(crate::i18n::t(kind_label(app.find.kind))).show_ui(ui, |ui| {
            for k in Kind::ALL {
                ui.selectable_value(&mut app.find.kind, k, crate::i18n::t(kind_label(k)));
            }
        })
        .response
        .set_name(tr!("What to search for"));
        if app.find.kind != before {
            app.find.current = None;
        }
        ui.label(text(Type::Label, tr!("in")));
        let scope = if app.find.document { tr!("The whole document") } else { tr!("This scene") };
        egui::ComboBox::from_id_salt("find_scope").width(150.0).selected_text(scope).show_ui(ui, |ui| {
            ui.selectable_value(&mut app.find.document, false, tr!("This scene")).on_hover_text(tr!("The current scene and every symbol in it"));
            ui.selectable_value(&mut app.find.document, true, tr!("The whole document")).on_hover_text(tr!("Every scene, and every symbol in the Library"));
        })
        .response
        .set_name(tr!("Where to search"));
    });
    ui.add_space(4.0);
    let enter = query_rows(app, ui);
    // Options and where to look.
    match app.find.kind {
        Kind::Text => {
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut app.find.opts.match_case, tr!("Match case"));
                ui.checkbox(&mut app.find.opts.whole_word, tr!("Whole word"));
                ui.checkbox(&mut app.find.opts.regex, tr!("Regular expression")).on_hover_text(tr!("A pattern such as h.llo or \\d+ (Rust regex syntax; runs in linear time)"));
            });
            ui.horizontal_wrapped(|ui| {
                ui.label(text(Type::Label, tr!("Look in")));
                let w = &mut app.find.within;
                ui.checkbox(&mut w.text, tr!("Text fields"));
                ui.checkbox(&mut w.labels, tr!("Frame labels"));
                ui.checkbox(&mut w.layers, tr!("Layer names"));
                ui.checkbox(&mut w.instances, tr!("Instance names"));
                ui.checkbox(&mut w.scripts, tr!("Scripts"));
            });
        }
        Kind::Color => {
            ui.horizontal_wrapped(|ui| {
                ui.label(text(Type::Label, tr!("Look in")));
                let w = &mut app.find.within;
                ui.checkbox(&mut w.fills, tr!("Fills"));
                ui.checkbox(&mut w.strokes, tr!("Strokes"));
                ui.checkbox(&mut w.text_color, tr!("Text"));
            });
        }
        _ => {}
    }
    ui.add_space(4.0);
    let list = hits(app);
    let (err, truncated) = match results(app) {
        Some(Err(e)) => (Some(e.clone()), false),
        Some(Ok((_, t))) => (None, *t),
        None => (None, false),
    };
    // The buttons: Find Next first (Enter in the Find field), then Replace and Replace All.
    let any = !list.is_empty();
    let next = |k: &str| app.keymap.primary(k).map(crate::platform::shortcut_label).unwrap_or_default();
    let (next_key, prev_key) = (next("view.find_next"), next("view.find_prev"));
    ui.horizontal(|ui| {
        if ui.add_enabled(any, egui::Button::new(tr!("Find Next"))).on_hover_text(a11y::with_key(tr!("Go to the next result"), Some(&next_key))).clicked() || (enter && any) {
            step(app, true);
        }
        if ui.add_enabled(any, egui::Button::new(tr!("Find Previous"))).on_hover_text(a11y::with_key(tr!("Go to the previous result"), Some(&prev_key))).clicked() {
            step(app, false);
        }
        if ui.add_enabled(any, egui::Button::new(tr!("Replace"))).on_hover_text(tr!("Replace the current result, then go to the next")).clicked() {
            replace_one(app);
        }
        if ui.add_enabled(any, egui::Button::new(tr!("Replace All"))).on_hover_text(tr!("Replace every result, as one step you can undo")).clicked() {
            let p = app.find.params();
            app.request("find.replace", p);
        }
    });
    ui.add_space(4.0);
    // What was found.
    if let Some(e) = err {
        ui.label(text(Type::Caption, e).color(T.error));
        return;
    }
    if !app.find.ready() {
        ui.label(text(Type::Caption, match app.find.kind {
            Kind::Text => tr!("Type what to find: results show as you type."),
            Kind::Font => tr!("Choose a font the document's text uses."),
            _ => tr!("Choose a Library item."),
        }));
        return;
    }
    let summary = if list.is_empty() {
        tr!("Nothing found").to_string()
    } else if truncated {
        trn!("The first {n} result", "The first {n} results", list.len())
    } else {
        trn!("{n} result", "{n} results", list.len())
    };
    ui.label(text(Type::Caption, summary));
    results_list(app, ui, &list);
}

/// The results, only the rows in view drawn (a search can find 100,000 places).
fn results_list(app: &mut App, ui: &mut egui::Ui, list: &[Hit]) {
    let mut clicked = None;
    let current = app.find.current;
    let kind = app.find.kind;
    egui::ScrollArea::vertical().id_salt("find_results").max_height(260.0).min_scrolled_height(TARGET * 6.0).auto_shrink([false, true]).show_rows(ui, TARGET, list.len(), |ui, range| {
        for i in range {
            let Some(h) = list.get(i) else { continue };
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), TARGET), Sense::click());
            let p = ui.painter();
            let on = current == Some(i);
            if on || resp.hovered() {
                p.rect_filled(r, T.rc(), if on { T.accent_dim } else { T.hover });
            }
            if on {
                p.rect_filled(Rect::from_min_size(r.left_top(), vec2(3.0, r.height())), 0.0, T.accent);
            }
            // What was found (the text around a match), then where.
            let what = match kind {
                Kind::Text => h.context.clone(),
                _ => h.found.clone(),
            };
            let field = crate::i18n::t(field_label(h.field));
            let place = location(app, h);
            let half = r.width() * 0.5;
            let left = Rect::from_min_max(pos2(r.left() + 8.0, r.top()), pos2(r.left() + half, r.bottom()));
            let right = Rect::from_min_max(pos2(r.left() + half + 6.0, r.top()), pos2(r.right() - 6.0, r.bottom()));
            let g = p.layout_no_wrap(what.clone(), font(Type::Body), color(Type::Body));
            p.with_clip_rect(left.intersect(ui.clip_rect())).galley(pos2(left.left(), r.center().y - g.rect.height() / 2.0), g, color(Type::Body));
            let label = format!("{field} · {place}");
            p.with_clip_rect(right.intersect(ui.clip_rect())).text(pos2(right.left(), r.center().y), Align2::LEFT_CENTER, &label, font(Type::Caption), color(Type::Caption));
            let name = tr!("{what}, {field}, {place}", what = what, field = field, place = place);
            a11y::control(ui, &resp, WidgetType::SelectableLabel, &name, Some(on));
            if resp.on_hover_text(&name).clicked() {
                clicked = Some(i);
            }
        }
    });
    if let Some(i) = clicked {
        go(app, i, list);
    }
}
