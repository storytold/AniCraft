//! Window › Movie Explorer (ADR 0058), a tab of the Library's dock: the movie as a tree of
//! scenes, layers, keyframes and what is on them, instances opening into their symbols, and the
//! Library's symbol definitions. The Show buttons hide kinds of rows; the search keeps matching
//! rows with the rows above them. A double-click (or Enter) goes there with `find.goto`, opening
//! symbols in place as a double-click on the stage does (ADR 0045). The tree comes from
//! `explorer::tree` (what `explorer.tree` pages for scripts), made again only when the document
//! or the view changes; the list draws only the rows in view.

use crate::a11y::{self, Named};
use crate::app::App;
use crate::icons::{self, Icon};
use crate::theme::{T, TARGET, Type, color, font, text};
use anicraft_engine::StateId;
use anicraft_engine::doc::{LayerKind, SymbolKind};
use anicraft_engine::explorer::{self, NodeKind, Options, Row, Show, Tree};
use anicraft_engine::find::Scope;
use egui::{Key, Rect, Sense, Stroke, StrokeKind, WidgetType, pos2, vec2};
use std::collections::HashSet;
use std::sync::Arc;

/// What the Movie Explorer shows and holds between frames (view state).
#[derive(Default)]
pub struct ExplorerView {
    pub show: ShowState,
    /// Every scene (off: only the current one).
    pub this_scene: bool,
    pub hide_definitions: bool,
    pub filter: String,
    /// Open rows; `None` until a row is opened or closed (then: the top rows are open).
    pub expanded: Option<HashSet<String>>,
    /// The selected row, by key.
    pub selected: Option<String>,
    /// Bumped when anything above changes, so the tree is made again.
    rev: u64,
    cache: Option<((StateId, usize, u64), Arc<Tree>)>,
}

/// The Show menu's kinds, kept as the engine's [`Show`] (all on to start).
#[derive(Default)]
pub struct ShowState(pub Show);

impl ExplorerView {
    fn options(&self) -> Options {
        Options {
            scope: if self.this_scene { Scope::Scene } else { Scope::Document },
            show: self.show.0,
            definitions: !self.hide_definitions,
            expanded: self.expanded.clone(),
            all: false,
            filter: self.filter.clone(),
        }
    }

    fn changed(&mut self) {
        self.rev += 1;
    }

    /// Opens or closes `row`.
    fn toggle(&mut self, rows: &[Row], key: &str) {
        let set = self.expanded.get_or_insert_with(|| rows.iter().filter(|r| r.open).map(|r| r.key.clone()).collect());
        if !set.remove(key) {
            set.insert(key.to_string());
        }
        self.changed();
    }
}

/// The tree for the current view, made again only when the document or the view changed.
fn tree(app: &mut App) -> Arc<Tree> {
    let key = (app.s.state, app.s.scene, app.explorer.rev);
    if let Some((k, t)) = &app.explorer.cache
        && *k == key
    {
        return t.clone();
    }
    let t = Arc::new(explorer::tree(&app.s.doc, app.s.scene, &app.explorer.options()));
    app.explorer.cache = Some((key, t.clone()));
    t
}

/// Window › Movie Explorer: the panel comes forward, wherever it is (ADR 0075).
pub fn open(app: &mut App) {
    app.show_panel(crate::dock::Panel::MovieExplorer);
    app.explorer.changed();
}

/// Opens every row above `depth` (screenshots).
pub fn expand_to(app: &mut App, depth: usize) {
    let all = explorer::tree(&app.s.doc, app.s.scene, &Options { all: true, ..app.explorer.options() });
    app.explorer.expanded = Some(all.rows.iter().filter(|r| r.depth < depth && r.children).map(|r| r.key.clone()).collect());
    app.explorer.changed();
}

fn kind_name(k: NodeKind) -> &'static str {
    match k {
        NodeKind::Scene => tr_noop!("Scene"),
        NodeKind::Definitions => tr_noop!("Symbol definitions"),
        NodeKind::Symbol(SymbolKind::Graphic) | NodeKind::Instance(SymbolKind::Graphic) => tr_noop!("Graphic"),
        NodeKind::Symbol(SymbolKind::MovieClip) | NodeKind::Instance(SymbolKind::MovieClip) => tr_noop!("Movie clip"),
        NodeKind::Symbol(SymbolKind::Button) | NodeKind::Instance(SymbolKind::Button) => tr_noop!("Button"),
        NodeKind::Layer(LayerKind::Folder) => tr_noop!("Folder"),
        NodeKind::Layer(LayerKind::Mask) => tr_noop!("Mask layer"),
        NodeKind::Layer(LayerKind::Guide) => tr_noop!("Guide layer"),
        NodeKind::Layer(LayerKind::Camera) => tr_noop!("Camera layer"),
        NodeKind::Layer(LayerKind::Armature) => tr_noop!("Armature layer"),
        NodeKind::Layer(LayerKind::Normal) => tr_noop!("Layer"),
        NodeKind::Frame => tr_noop!("Keyframe"),
        NodeKind::Script => tr_noop!("Frame script"),
        NodeKind::Handler => tr_noop!("Actions"),
        NodeKind::Sound => tr_noop!("Sound"),
        NodeKind::Text => tr_noop!("Text"),
        NodeKind::Bitmap => tr_noop!("Bitmap"),
    }
}

/// What a row says: its document text, or for rows without a name what they are.
fn row_text(r: &Row) -> String {
    match r.kind {
        NodeKind::Definitions => crate::i18n::t(kind_name(r.kind)).to_string(),
        NodeKind::Frame => match (r.frame, r.name.is_empty()) {
            // The label is document text, as written.
            (Some(f), false) => tr!("Frame {frame}: {label}", frame = f + 1, label = r.name),
            (Some(f), true) => tr!("Frame {frame}", frame = f + 1),
            _ => crate::i18n::t(kind_name(r.kind)).to_string(),
        },
        NodeKind::Handler => {
            let event = anicraft_engine::doc::actions::ButtonEvent::ALL.iter().find(|e| e.key() == r.name).map_or(r.name.as_str(), |e| crate::i18n::t(e.label()));
            tr!("On {event}: {actions}", event = event, actions = r.extra)
        }
        NodeKind::Instance(_) if !r.extra.is_empty() => tr!("{instance} ({symbol})", instance = r.extra, symbol = r.name),
        NodeKind::Bitmap if !r.extra.is_empty() => tr!("{instance} ({bitmap})", instance = r.extra, bitmap = r.name),
        _ if r.name.is_empty() => crate::i18n::t(kind_name(r.kind)).to_string(),
        _ => r.name.clone(),
    }
}

/// The row's marker: the Library's symbol square, or the icon the timeline uses.
fn marker(p: &egui::Painter, at: Rect, k: NodeKind, c: egui::Color32) {
    let symbol = |kind: SymbolKind| match kind {
        SymbolKind::Graphic => T.kind_graphic,
        SymbolKind::MovieClip => T.kind_movie_clip,
        SymbolKind::Button => T.kind_button,
    };
    match k {
        NodeKind::Symbol(s) | NodeKind::Instance(s) => {
            let r = at.shrink(1.0);
            p.rect_stroke(r, 2.0, Stroke::new(1.5, symbol(s)), StrokeKind::Inside);
            p.circle_filled(r.center(), 2.5, symbol(s));
        }
        NodeKind::Scene => icons::paint(p, at, Icon::Play, c),
        NodeKind::Definitions => icons::paint(p, at, Icon::Folder, c),
        NodeKind::Layer(LayerKind::Folder) => icons::paint(p, at, Icon::Folder, c),
        NodeKind::Layer(LayerKind::Mask) => icons::paint(p, at, Icon::Mask, c),
        NodeKind::Layer(LayerKind::Guide) => icons::paint(p, at, Icon::Guide, c),
        NodeKind::Layer(LayerKind::Camera) => icons::paint(p, at, Icon::Camera, c),
        NodeKind::Layer(LayerKind::Armature) => icons::paint(p, at, Icon::Bone, c),
        NodeKind::Layer(LayerKind::Normal) => {
            p.rect_stroke(at.shrink(2.0), 1.0, Stroke::new(1.2, c), StrokeKind::Inside);
        }
        NodeKind::Frame => {
            p.circle_filled(at.center(), 3.0, c);
        }
        NodeKind::Script | NodeKind::Handler => {
            p.text(at.center(), egui::Align2::CENTER_CENTER, "{ }", font(Type::DataSmall), T.accent_text);
        }
        NodeKind::Sound => icons::paint(p, at, Icon::Sound, c),
        NodeKind::Text => icons::paint(p, at, Icon::Type, c),
        NodeKind::Bitmap => icons::paint(p, at, Icon::Image, c),
    }
}

pub fn panel(app: &mut App, ui: &mut egui::Ui) {
    // The search, and the Show menu: kinds of rows, which scenes, the Library's symbols.
    ui.horizontal(|ui| {
        // The search takes what the Show button (as wide as its translation) leaves.
        let menu_w_id = ui.id().with("explorer_show_w");
        let menu_w = ui.data(|d| d.get_temp::<f32>(menu_w_id)).unwrap_or(72.0) + ui.spacing().item_spacing.x;
        let search = ui.add(egui::TextEdit::singleline(&mut app.explorer.filter).hint_text(tr!("Search")).desired_width((ui.available_width() - menu_w).max(crate::theme::MIN_FIELD)));
        search.set_name(tr!("Search the Movie Explorer"));
        if search.changed() {
            app.explorer.changed();
        }
        let menu = ui.menu_button(tr!("Show"), |ui| {
            let mut changed = false;
            let s = &mut app.explorer.show.0;
            for (on, label, tip) in [
                (&mut s.text, tr!("Text"), tr!("Text fields")),
                (&mut s.symbols, tr!("Symbols"), tr!("Buttons, movie clips and graphics")),
                (&mut s.scripts, tr!("Scripts"), tr!("Frame scripts and instance actions")),
                (&mut s.media, tr!("Sounds and bitmaps"), tr!("Sounds on keyframes, and bitmaps")),
                (&mut s.frames, tr!("Layers and frames"), tr!("Off: what is on them, straight under the scene")),
            ] {
                changed |= ui.checkbox(on, label).on_hover_text(tip).changed();
            }
            ui.separator();
            changed |= ui.radio_value(&mut app.explorer.this_scene, false, tr!("All scenes")).changed();
            changed |= ui.radio_value(&mut app.explorer.this_scene, true, tr!("This scene")).changed();
            ui.separator();
            let mut defs = !app.explorer.hide_definitions;
            if ui.checkbox(&mut defs, tr!("Symbol definitions")).on_hover_text(tr!("Every symbol in the Library, with what is inside it")).changed() {
                app.explorer.hide_definitions = !defs;
                changed = true;
            }
            if changed {
                app.explorer.changed();
            }
        });
        let w = menu.response.rect.width();
        if ui.data(|d| d.get_temp::<f32>(menu_w_id)) != Some(w) {
            ui.data_mut(|d| d.insert_temp(menu_w_id, w));
            ui.ctx().request_discard("the Show button's width changed"); // i18n-exempt: a debug reason, never shown
        }
        menu.response.on_hover_text(tr!("What the Movie Explorer shows")).set_name(tr!("Show in the Movie Explorer"));
    });
    ui.add_space(4.0);
    let t = tree(app);
    if t.rows.is_empty() {
        ui.label(text(Type::Caption, if app.explorer.filter.trim().is_empty() { tr!("Nothing to show: turn on more Show buttons.") } else { tr!("Nothing matches the search.") }));
        return;
    }
    let summary = if t.truncated { trn!("The first {n} row", "The first {n} rows", t.rows.len()) } else { trn!("{n} row", "{n} rows", t.rows.len()) };
    ui.label(text(Type::Caption, summary));
    list(app, ui, &t);
}

/// The rows, only those in view drawn.
fn list(app: &mut App, ui: &mut egui::Ui, t: &Tree) {
    let mut toggle: Option<String> = None;
    let mut select: Option<String> = None;
    let mut go: Option<usize> = None;
    let room = (ui.available_height() - 8.0).clamp(140.0, 320.0);
    let filtering = !app.explorer.filter.trim().is_empty();
    egui::ScrollArea::vertical().id_salt("explorer_rows").max_height(room).auto_shrink([false, true]).show_rows(ui, TARGET, t.rows.len(), |ui, range| {
        for i in range {
            let Some(r) = t.rows.get(i) else { continue };
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), TARGET), Sense::click());
            let on = app.explorer.selected.as_deref() == Some(r.key.as_str());
            let p = ui.painter();
            if on || resp.hovered() {
                p.rect_filled(rect, T.rc(), if on { T.accent_dim } else { T.hover });
            }
            if on {
                p.rect_filled(Rect::from_min_size(rect.left_top(), vec2(3.0, rect.height())), 0.0, T.accent);
            }
            let indent = 6.0 + r.depth.min(24) as f32 * 14.0;
            let chevron = Rect::from_center_size(pos2(rect.left() + indent + 6.0, rect.center().y), vec2(12.0, 12.0));
            let icon = Rect::from_center_size(pos2(rect.left() + indent + 22.0, rect.center().y), vec2(14.0, 14.0));
            if r.children && !filtering {
                icons::paint(p, chevron, if r.open { Icon::ChevronDown } else { Icon::ChevronRight }, T.icon);
            }
            marker(p, icon, r.kind, color(Type::Body));
            let label = row_text(r);
            let name_rect = Rect::from_min_max(pos2(icon.right() + 8.0, rect.top()), rect.max);
            let g = p.layout_no_wrap(label.clone(), font(Type::Body), color(Type::Body));
            p.with_clip_rect(name_rect.intersect(ui.clip_rect())).galley(pos2(name_rect.left(), rect.center().y - g.rect.height() / 2.0), g, color(Type::Body));
            let kind = crate::i18n::t(kind_name(r.kind));
            let name = match (r.children && !filtering, r.open) {
                (true, true) => tr!("{kind}: {row}, open", kind = kind, row = label),
                (true, false) => tr!("{kind}: {row}, closed", kind = kind, row = label),
                _ => tr!("{kind}: {row}", kind = kind, row = label),
            };
            a11y::control(ui, &resp, WidgetType::SelectableLabel, &name, Some(on));
            let resp = resp.on_hover_text(tr!("{row}\nDouble-click to go there", row = name));
            if resp.clicked() {
                let on_chevron = ui.input(|inp| inp.pointer.interact_pos()).is_some_and(|q| q.x < icon.left());
                if on_chevron && r.children && !filtering {
                    toggle = Some(r.key.clone());
                }
                select = Some(r.key.clone());
            }
            if resp.double_clicked() {
                go = Some(i);
            }
            if resp.has_focus() && ui.input(|inp| inp.key_pressed(Key::Enter)) {
                go = Some(i);
            }
        }
    });
    if let Some(k) = select {
        app.explorer.selected = Some(k);
    }
    if let Some(k) = toggle {
        app.explorer.toggle(&t.rows, &k);
    }
    if let Some(r) = go.and_then(|i| t.rows.get(i)) {
        goto(app, r);
    }
}

/// Goes where row `r` is.
fn goto(app: &mut App, r: &Row) {
    app.explorer.selected = Some(r.key.clone());
    if r.kind == NodeKind::Definitions {
        return;
    }
    app.playing = false;
    if app.run("find.goto", r.go.to_json()).is_some() {
        app.status_ok(tr!("Went to {row}", row = row_text(r)));
    }
}
