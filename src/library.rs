//! The Library panel (ADR 0050): folders, sorting and search, inline rename, drag into and out
//! of folders, multiple selection, an item preview (a symbol's first frame, a bitmap, a
//! sound's waveform with play) and the item's properties. Items drag onto the stage, or Place on
//! Stage (Enter) puts them at the centre of the view (ADR 0053). Every change is a `library.*`
//! command; what is sorted, filtered, expanded or selected is view state, kept here.

use crate::a11y::{self, Named};
use crate::app::App;
use crate::icons::{self, Icon};
use crate::theme::{T, TARGET, Type, color, font, text, with_type};
use anicraft_engine::StateId;
use anicraft_engine::cmds_library::entry_json;
use anicraft_engine::doc::library::MAX_FOLDER_DEPTH;
use anicraft_engine::doc::{Document, FolderId, LibraryEntry, LibraryItem, SymbolKind};
use egui::{Align2, Key, Modifiers, Pos2, Rect, Sense, Stroke, StrokeKind, WidgetType, pos2, vec2};
use serde_json::json;
use std::collections::{HashMap, HashSet};

/// How the list is ordered (folders always come first, by name).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Name,
    Type,
    Uses,
    /// When the item came into the Library (ids grow with time).
    Added,
}

impl Sort {
    const ALL: [Sort; 4] = [Sort::Name, Sort::Type, Sort::Uses, Sort::Added];
    fn label(self) -> &'static str {
        match self {
            Sort::Name => tr!("Name"),
            Sort::Type => tr!("Type"),
            Sort::Uses => tr!("Use count"),
            Sort::Added => tr!("Date added"),
        }
    }
}

/// What the Library panel shows and holds between frames.
#[derive(Default)]
pub struct LibraryView {
    /// The selected rows (items and folders).
    pub sel: Vec<LibraryEntry>,
    /// Where a Shift-click range starts.
    anchor: Option<LibraryEntry>,
    pub expanded: HashSet<FolderId>,
    pub sort: Sort,
    pub descending: bool,
    pub filter: String,
    /// The row being renamed and the text so far; `true` until the field has taken focus.
    pub renaming: Option<(LibraryEntry, String, bool)>,
    /// The list was clicked last: F2 and Delete act on it (not on the stage).
    pub keys: bool,
    /// Where the list was drawn last frame (`None` before the first).
    list_rect: Option<Rect>,
    uses: Option<(StateId, HashMap<LibraryItem, usize>, HashSet<LibraryItem>)>,
    preview: Option<Preview>,
    /// Where files dropped from the desktop land this frame (ADR 0059).
    pub drop_zone: DropZone,
}

/// The Library as a target for files dropped from the desktop (ADR 0059): the panel, and each
/// row with the folder a file dropped on it goes into. Cleared every frame before the panels
/// draw, so a hidden panel takes nothing.
#[derive(Default)]
pub struct DropZone {
    pub panel: Option<Rect>,
    pub rows: Vec<(Rect, Option<FolderId>)>,
}

impl DropZone {
    /// The folder a file dropped at `p` goes into (`None` inside: the top level), or `None`
    /// outside the panel.
    pub fn folder_at(&self, p: Pos2) -> Option<Option<FolderId>> {
        if !self.panel.is_some_and(|r| r.contains(p)) {
            return None;
        }
        Some(self.rows.iter().find(|(r, _)| r.contains(p)).and_then(|(_, f)| *f))
    }

    /// The row a drop at `p` lands on (a folder row, or the row inside that folder).
    pub fn row_at(&self, p: Pos2) -> Option<Rect> {
        self.rows.iter().find(|(r, _)| r.contains(p)).map(|(r, _)| *r)
    }
}

/// The current item's preview picture, and what it was made from.
struct Preview {
    key: (LibraryItem, StateId),
    texture: Option<egui::TextureHandle>,
}

/// The first item in the Library: its first symbol, else its first bitmap, else sound.
pub fn first_item(doc: &Document) -> Option<LibraryItem> {
    doc.library_items().next()
}

/// One visible row.
struct Row {
    entry: LibraryEntry,
    depth: usize,
    name: String,
    /// What it is, for the tooltip and screen readers (“Movie clip · 24 frames”).
    detail: String,
    uses: Option<usize>,
    open: bool,
}

impl LibraryView {
    /// Use counts and unused items, worked out once per document state.
    fn counts(&mut self, s: &anicraft_engine::Session) -> (HashMap<LibraryItem, usize>, HashSet<LibraryItem>) {
        match &self.uses {
            Some((st, u, n)) if *st == s.state => (u.clone(), n.clone()),
            _ => {
                let (u, n): (HashMap<LibraryItem, usize>, HashSet<LibraryItem>) = (s.doc.library_uses(), s.doc.unused_items().into_iter().collect());
                self.uses = Some((s.state, u.clone(), n.clone()));
                (u, n)
            }
        }
    }

    /// Only one row selected: `e`.
    fn select_only(&mut self, e: LibraryEntry) {
        self.sel = vec![e];
        self.anchor = Some(e);
    }
}

fn kind_rank(doc: &Document, e: LibraryEntry) -> u8 {
    match e {
        LibraryEntry::Folder(_) => 0,
        LibraryEntry::Item(LibraryItem::Symbol(id)) => match doc.symbol(id).map(|s| s.kind) {
            Some(SymbolKind::Graphic) => 1,
            Some(SymbolKind::MovieClip) => 2,
            _ => 3,
        },
        LibraryEntry::Item(LibraryItem::Bitmap(_)) => 4,
        LibraryEntry::Item(LibraryItem::Sound(_)) => 5,
    }
}

fn raw_id(e: LibraryEntry) -> u64 {
    match e {
        LibraryEntry::Item(i) => i.raw(),
        LibraryEntry::Folder(f) => f.0,
    }
}

fn detail(doc: &Document, e: LibraryEntry) -> String {
    match e {
        LibraryEntry::Folder(f) => {
            let n = doc.library_children(Some(f)).len();
            trn!("Folder · {n} item", "Folder · {n} items", n)
        }
        LibraryEntry::Item(LibraryItem::Symbol(id)) => doc.symbol(id).map_or_else(String::new, |s| {
            let n = s.timeline.frame_count();
            let frames = trn!("{n} frame", "{n} frames", n);
            tr!("{kind} · {frames}", kind = crate::panels::symbol_kind_label(s.kind), frames = frames)
        }),
        LibraryEntry::Item(LibraryItem::Bitmap(id)) => doc.bitmap(id).map_or_else(String::new, |b| tr!("Bitmap · {width}×{height}", width = b.width, height = b.height)),
        LibraryEntry::Item(LibraryItem::Sound(id)) => doc.sound(id).map_or_else(String::new, |x| tr!("Sound · {seconds} s", seconds = format!("{:.1}", x.seconds()))),
    }
}

/// The visible rows: folders (open ones with what they hold), then items, ordered by `sort`.
/// With a search, only matching items show, inside every folder that leads to one.
fn rows(doc: &Document, v: &LibraryView, uses: &HashMap<LibraryItem, usize>) -> Vec<Row> {
    let query = v.filter.trim().to_lowercase();
    let matches = |e: LibraryEntry| query.is_empty() || doc.entry_name(e).is_some_and(|n| n.to_lowercase().contains(&query));
    // Folders that lead to a match (searching opens them all).
    let mut leads: HashSet<FolderId> = HashSet::new();
    if !query.is_empty() {
        for e in doc.library_items().map(LibraryEntry::Item).chain(doc.library_tree.folders.iter().map(|f| LibraryEntry::Folder(f.id))) {
            if matches(e)
                && let Some(p) = doc.parent_of(e)
            {
                leads.extend(doc.folder_chain(p));
            }
        }
    }
    let count = |e: LibraryEntry| match e {
        LibraryEntry::Item(i) => uses.get(&i).copied().unwrap_or(0),
        LibraryEntry::Folder(_) => 0,
    };
    let mut out = Vec::new();
    // Depth-first, in order: each level is sorted, then pushed in reverse.
    let mut pending: Vec<(LibraryEntry, usize)> = level(doc, v, None, &count).into_iter().rev().map(|e| (e, 0)).collect();
    while let Some((e, depth)) = pending.pop() {
        if out.len() > 100_000 {
            break;
        }
        let shown = match e {
            LibraryEntry::Folder(f) => query.is_empty() || leads.contains(&f) || matches(e),
            LibraryEntry::Item(_) => matches(e),
        };
        if !shown {
            continue;
        }
        let open = match e {
            LibraryEntry::Folder(f) => (!query.is_empty() && leads.contains(&f)) || (query.is_empty() && v.expanded.contains(&f)),
            LibraryEntry::Item(_) => false,
        };
        out.push(Row { entry: e, depth, name: doc.entry_name(e).unwrap_or("").to_string(), detail: detail(doc, e), uses: matches!(e, LibraryEntry::Item(_)).then(|| count(e)), open });
        if let LibraryEntry::Folder(f) = e
            && open
            && depth + 1 < MAX_FOLDER_DEPTH
        {
            pending.extend(level(doc, v, Some(f), &count).into_iter().rev().map(|c| (c, depth + 1)));
        }
    }
    out
}

/// What folder `parent` holds, folders first (by name), then items in the chosen order.
fn level(doc: &Document, v: &LibraryView, parent: Option<FolderId>, count: &dyn Fn(LibraryEntry) -> usize) -> Vec<LibraryEntry> {
    let mut all = doc.library_children(parent);
    let name = |e: &LibraryEntry| doc.entry_name(*e).unwrap_or("").to_lowercase();
    all.sort_by(|a, b| {
        let folders = matches!(b, LibraryEntry::Folder(_)).cmp(&matches!(a, LibraryEntry::Folder(_)));
        let by = match v.sort {
            Sort::Name => std::cmp::Ordering::Equal,
            Sort::Type => kind_rank(doc, *a).cmp(&kind_rank(doc, *b)),
            Sort::Uses => count(*a).cmp(&count(*b)),
            Sort::Added => raw_id(*a).cmp(&raw_id(*b)),
        };
        let by = by.then_with(|| name(a).cmp(&name(b)));
        folders.then(if v.descending && !matches!(a, LibraryEntry::Folder(_)) { by.reverse() } else { by })
    });
    all
}

/// F2 renames the selected row and Delete deletes the selection, while the list has the
/// keyboard (it was clicked last). Called before the global shortcuts.
pub fn consume_keys(app: &mut App, ctx: &egui::Context) {
    let clicked_elsewhere = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !app.library.list_rect.is_some_and(|r| r.contains(p))));
    if clicked_elsewhere {
        app.library.keys = false;
    }
    if !app.library.keys || app.library.renaming.is_some() {
        return;
    }
    let (rename, delete, place) = ctx.input_mut(|i| (i.consume_key(Modifiers::NONE, Key::F2), i.consume_key(Modifiers::NONE, Key::Delete) || i.consume_key(Modifiers::NONE, Key::Backspace), i.consume_key(Modifiers::NONE, Key::Enter)));
    if rename && let [one] = app.library.sel.as_slice() {
        let e = *one;
        start_rename(app, e);
    }
    if delete {
        delete_selection(app);
    }
    // Enter: the keyboard's way to do what dragging onto the stage does (ADR 0053).
    if place {
        let items = selected_items(app);
        if !items.is_empty() {
            place_items(app, &items, None);
        }
    }
}

/// The selected rows' items (folders left out), in selection order.
fn selected_items(app: &App) -> Vec<LibraryItem> {
    app.library.sel.iter().filter_map(|e| match e {
        LibraryEntry::Item(i) if app.s.doc.has_item(*i) => Some(*i),
        _ => None,
    }).collect()
}

fn start_rename(app: &mut App, e: LibraryEntry) {
    let name = app.s.doc.entry_name(e).unwrap_or("").to_string();
    app.library.renaming = Some((e, name, true));
}

fn delete_selection(app: &mut App) {
    let items: Vec<serde_json::Value> = app.library.sel.iter().filter(|e| app.s.doc.has_entry(**e)).map(|e| entry_json(*e)).collect();
    if !items.is_empty() {
        app.request("library.delete", json!({"items": items}));
    }
}

/// The centre of what the stage shows, in the space being edited (inside an open symbol, its
/// own, ADR 0045): where Place on Stage puts things.
fn view_centre(app: &App) -> (f32, f32) {
    let c = app.view_center_stage();
    match app.s.edit_view() {
        Some(v) if v.scale.abs() > 1e-6 => ((c.x - v.origin.x) / v.scale, (c.y - v.origin.y) / v.scale),
        _ => (c.x, c.y),
    }
}

/// Places items at `at` (in the edited space; default: the centre of the view): symbols and
/// bitmaps on the active layer at the playhead, a sound on its keyframe there. One undo step
/// (`library.place`, ADR 0053). The stage then has the keyboard again.
pub fn place_items(app: &mut App, items: &[LibraryItem], at: Option<(f32, f32)>) {
    let (x, y) = at.unwrap_or_else(|| view_centre(app));
    let list: Vec<serde_json::Value> = items.iter().map(|i| entry_json(LibraryEntry::Item(*i))).collect();
    app.run("library.place", json!({"items": list, "x": x, "y": y}));
    app.library.keys = false;
}

fn place(app: &mut App, item: LibraryItem) {
    place_items(app, &[item], None);
}

/// What the Library rows being dragged hold that can be placed (folders left out).
fn dragged_items(app: &App, ctx: &egui::Context) -> Vec<LibraryItem> {
    egui::DragAndDrop::payload::<Vec<LibraryEntry>>(ctx).map_or_else(Vec::new, |p| {
        p.iter().filter_map(|e| match e {
            LibraryEntry::Item(i) if app.s.doc.has_item(*i) => Some(*i),
            _ => None,
        }).collect()
    })
}

/// Library rows dragged over the stage (ADR 0053): a mark shows where they land and on which
/// layer; letting go places them there, sounds onto the active layer's keyframe at the playhead,
/// in one undo step. `to_stage` maps a screen point into the edited space.
pub fn stage_drop(app: &mut App, ui: &egui::Ui, resp: &egui::Response, to_stage: impl Fn(Pos2) -> Pos2, paused: bool) {
    if resp.dnd_hover_payload::<Vec<LibraryEntry>>().is_none() {
        return;
    }
    let items = dragged_items(app, ui.ctx());
    let Some(p) = ui.input(|i| i.pointer.latest_pos()) else { return };
    if items.is_empty() {
        return;
    }
    if let Some(_payload) = resp.dnd_release_payload::<Vec<LibraryEntry>>() {
        if paused {
            app.status_ok(tr!("Previewing through the camera: artwork edits are paused. Turn off View › Preview Through Camera to edit."));
            return;
        }
        let at = to_stage(p);
        place_items(app, &items, Some((at.x, at.y)));
        return;
    }
    ui.ctx().set_cursor_icon(egui::CursorIcon::Copy);
    let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("library_drop")));
    let only_sounds = items.iter().all(|i| matches!(i, LibraryItem::Sound(_)));
    if !only_sounds {
        let r = 7.0;
        painter.line_segment([p - vec2(r, 0.0), p + vec2(r, 0.0)], Stroke::new(1.5, T.accent));
        painter.line_segment([p - vec2(0.0, r), p + vec2(0.0, r)], Stroke::new(1.5, T.accent));
    }
    let layer = app.s.active_layer().map(|l| l.name.clone()).unwrap_or_default();
    let frame = app.s.frame + 1;
    let hint = if only_sounds { tr!("Put on {layer} at frame {frame}", layer = layer, frame = frame) } else { tr!("Place on {layer}", layer = layer) };
    let galley = painter.layout_no_wrap(hint, font(Type::Caption), color(Type::Body));
    let at = p + vec2(14.0, 10.0);
    painter.rect_filled(Rect::from_min_size(at, galley.size()).expand(4.0), T.rc(), T.panel);
    painter.galley(at, galley, color(Type::Body));
}

/// While Library rows are dragged, their names follow the pointer, so it is clear what is being
/// carried.
fn drag_ghost(app: &App, ctx: &egui::Context) {
    let items = dragged_items(app, ctx);
    let (Some(first), Some(p)) = (items.first(), ctx.input(|i| i.pointer.latest_pos())) else { return };
    let name = app.s.doc.item_name(*first).unwrap_or("").to_string();
    let label = if items.len() > 1 { trn!("{name} and {n} more", "{name} and {n} more", items.len() - 1, name = name) } else { name };
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("library_ghost")));
    let galley = painter.layout_no_wrap(label, font(Type::Body), color(Type::Body));
    let at = p + vec2(14.0, -galley.size().y - 8.0);
    painter.rect_filled(Rect::from_min_size(at, galley.size()).expand(4.0), T.rc(), T.accent_dim);
    painter.galley(at, galley, color(Type::Body));
}

/// A new folder in the selected row's folder (or the selected folder), opened, selected and
/// ready to be named.
fn new_folder(app: &mut App) {
    let parent = match app.library.sel.as_slice() {
        [LibraryEntry::Folder(f)] => Some(*f),
        [one] => app.s.doc.parent_of(*one),
        _ => None,
    };
    if let Some(p) = parent {
        app.library.expanded.insert(p);
    }
    if let Some(v) = app.run("library.new_folder", json!({"parent": parent.map(|f| f.0)}))
        && let Some(id) = v["folder"].as_u64()
    {
        let e = LibraryEntry::Folder(FolderId(id));
        app.library.select_only(e);
        start_rename(app, e);
    }
}

/// Moves `what` into `to` (`None`: the top level), opening the folder so they stay in view.
fn move_into(app: &mut App, what: &[LibraryEntry], to: Option<FolderId>) {
    let items: Vec<serde_json::Value> = what.iter().filter(|e| app.s.doc.parent_of(**e) != to && **e != to.map_or(**e, LibraryEntry::Folder)).map(|e| entry_json(*e)).collect();
    if items.is_empty() {
        return;
    }
    if app.run("library.move", json!({"items": items, "to": to.map(|f| f.0)})).is_some()
        && let Some(f) = to
    {
        app.library.expanded.insert(f);
    }
}

pub fn panel(app: &mut App, ui: &mut egui::Ui) {
    // Rows that are gone (an undo, a deletion) leave the selection.
    let doc = &app.s.doc;
    app.library.sel.retain(|e| doc.has_entry(*e));
    if app.library.renaming.as_ref().is_some_and(|(e, _, _)| !doc.has_entry(*e)) {
        app.library.renaming = None;
    }
    if doc.library_items().next().is_none() && doc.library_tree.folders.is_empty() {
        let key = |id: &str| app.keymap.primary(id).map(crate::platform::shortcut_label).unwrap_or_default();
        ui.label(text(Type::Caption, tr!("No symbols, images or sounds yet. Select artwork and press {convert} to make a symbol, or import an image or a sound with {import}.", convert = key("modify.convert_to_symbol"), import = key("file.import"))));
        if ui.button(tr!("New Folder")).clicked() {
            new_folder(app);
        }
        return;
    }
    toolbar(app, ui);
    drag_ghost(app, ui.ctx());
    ui.add_space(4.0);
    let (uses, unused) = app.library.counts(&app.s);
    let list = rows(&app.s.doc, &app.library, &uses);
    let total = app.s.doc.library_items().count();
    let summary = if app.library.filter.trim().is_empty() {
        trn!("{n} item", "{n} items", total)
    } else {
        tr!("{shown} of {total}", shown = list.iter().filter(|r| matches!(r.entry, LibraryEntry::Item(_))).count(), total = total)
    };
    ui.horizontal(|ui| {
        ui.label(text(Type::Caption, summary));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(text(Type::Caption, tr!("Uses"))));
    });
    list_ui(app, ui, &list, &unused);
    ui.add_space(4.0);
    actions_row(app, ui);
    ui.add_space(6.0);
    preview(app, ui);
    properties(app, ui);
}

/// Search, sort, and the panel's menu.
fn toolbar(app: &mut App, ui: &mut egui::Ui) {
    let search = ui.add(egui::TextEdit::singleline(&mut app.library.filter).hint_text(tr!("Search")).desired_width(ui.available_width()));
    search.set_name(tr!("Search the Library"));
    ui.horizontal(|ui| {
        ui.label(text(Type::Label, tr!("Sort")));
        egui::ComboBox::from_id_salt("lib_sort")
            .width(110.0)
            .selected_text(app.library.sort.label())
            .show_ui(ui, |ui| {
                for s in Sort::ALL {
                    if ui.selectable_label(app.library.sort == s, s.label()).clicked() {
                        app.library.sort = s;
                    }
                }
            })
            .response
            .set_name(tr!("Sort by"));
        let (icon, tip) = if app.library.descending { (Icon::ChevronDown, tr!("Descending: click for ascending")) } else { (Icon::ChevronUp, tr!("Ascending: click for descending")) };
        if crate::timeline::icon_button(ui, tip, None, icon).clicked() {
            app.library.descending = !app.library.descending;
        }
        let menu = ui.menu_button(tr!("More"), |ui| {
            let n_unused = app.library.counts(&app.s).1.len();
            if ui.add_enabled(n_unused > 0, egui::Button::new(tr!("Select Unused Items"))).clicked() {
                ui.close();
                select_unused(app);
            }
            if ui.add_enabled(n_unused > 0, egui::Button::new(tr!("Delete Unused Items…"))).clicked() {
                ui.close();
                app.request("library.delete_unused", json!({}));
            }
            ui.separator();
            if ui.button(tr!("Expand All Folders")).clicked() {
                ui.close();
                app.library.expanded = app.s.doc.library_tree.folders.iter().map(|f| f.id).collect();
            }
            if ui.button(tr!("Collapse All Folders")).clicked() {
                ui.close();
                app.library.expanded.clear();
            }
        });
        menu.response.on_hover_text(tr!("Library options")).set_name(tr!("Library options"));
    });
}

/// Selects every unused item, opening the folders they are in.
pub fn select_unused(app: &mut App) {
    let unused = app.s.doc.unused_items();
    for i in &unused {
        if let Some(p) = app.s.doc.parent_of(LibraryEntry::Item(*i)) {
            app.library.expanded.extend(app.s.doc.folder_chain(p));
        }
    }
    app.library.sel = unused.iter().map(|i| LibraryEntry::Item(*i)).collect();
    app.library.keys = true;
    app.status_ok(trn!("{n} unused item selected", "{n} unused items selected", unused.len()));
}

fn list_ui(app: &mut App, ui: &mut egui::Ui, list: &[Row], unused: &HashSet<LibraryItem>) {
    let mut toggle: Option<FolderId> = None;
    let mut clicked: Option<(LibraryEntry, Modifiers)> = None;
    let mut activate: Option<LibraryEntry> = None;
    let mut rename: Option<LibraryEntry> = None;
    let mut dropped: Option<(Vec<LibraryEntry>, Option<FolderId>)> = None;
    let scroll = egui::ScrollArea::vertical().id_salt("lib").max_height(200.0).auto_shrink([false, true]).show(ui, |ui| {
        for row in list {
            let selected = app.library.sel.contains(&row.entry);
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), TARGET), Sense::click_and_drag());
            let p = ui.painter();
            if selected || resp.hovered() {
                p.rect_filled(r, T.rc(), if selected { T.accent_dim } else { T.hover });
            }
            if selected {
                p.rect_filled(Rect::from_min_size(r.left_top(), vec2(3.0, r.height())), 0.0, T.accent);
            }
            let indent = 6.0 + row.depth as f32 * 14.0;
            let chevron = Rect::from_center_size(pos2(r.left() + indent + 6.0, r.center().y), vec2(12.0, 12.0));
            let icon = Rect::from_center_size(pos2(r.left() + indent + 22.0, r.center().y), vec2(14.0, 14.0));
            match row.entry {
                LibraryEntry::Folder(_) => {
                    icons::paint(p, chevron, if row.open { Icon::ChevronDown } else { Icon::ChevronRight }, T.icon);
                    icons::paint(p, icon.expand(1.0), if row.open { Icon::FolderOpen } else { Icon::Folder }, color(Type::Body));
                }
                LibraryEntry::Item(LibraryItem::Symbol(id)) => {
                    let kind_color = match app.s.doc.symbol(id).map(|s| s.kind) {
                        Some(SymbolKind::Graphic) | None => T.kind_graphic,
                        Some(SymbolKind::MovieClip) => T.kind_movie_clip,
                        Some(SymbolKind::Button) => T.kind_button,
                    };
                    p.rect_stroke(icon, 2.0, Stroke::new(1.5, kind_color), StrokeKind::Inside);
                    p.circle_filled(icon.center(), 2.5, kind_color);
                }
                LibraryEntry::Item(LibraryItem::Sound(_)) => icons::paint(p, icon.expand(1.0), Icon::Sound, color(Type::Body)),
                LibraryEntry::Item(LibraryItem::Bitmap(_)) => icons::paint(p, icon.expand(1.0), Icon::Image, color(Type::Body)),
            }
            let name_rect = Rect::from_min_max(pos2(icon.right() + 8.0, r.top()), pos2(r.right() - 40.0, r.bottom()));
            let uses_text = row.uses.map(|n| n.to_string()).unwrap_or_default();
            let is_unused = matches!(row.entry, LibraryEntry::Item(i) if unused.contains(&i));
            if let Some(n) = row.uses {
                p.text(r.right_center() - vec2(8.0, 0.0), Align2::RIGHT_CENTER, &uses_text, font(Type::Data), if n == 0 { T.text_faint } else { color(Type::Caption) });
            }
            // Renaming: a field over the name; Enter or clicking away keeps it, Esc doesn't.
            let editing = app.library.renaming.as_ref().is_some_and(|(e, _, _)| *e == row.entry);
            if editing {
                let mut commit = None;
                if let Some((e, buf, focus)) = &mut app.library.renaming {
                    let edit = ui.put(name_rect.shrink2(vec2(0.0, 3.0)), egui::TextEdit::singleline(buf).font(font(Type::Body)));
                    edit.set_name(tr!("Library name"));
                    if *focus {
                        edit.request_focus();
                        *focus = false;
                    }
                    let esc = ui.input(|i| i.key_pressed(Key::Escape));
                    if edit.lost_focus() {
                        commit = Some((*e, buf.clone(), !esc));
                    }
                }
                if let Some((e, name, keep)) = commit {
                    app.library.renaming = None;
                    if keep && app.s.doc.entry_name(e) != Some(name.as_str()) {
                        let mut params = entry_json(e);
                        params["name"] = json!(name);
                        app.run("library.rename", params);
                    }
                }
            } else {
                let c = if is_unused { T.text_dim } else { color(Type::Body) };
                let galley = ui.painter().layout_no_wrap(row.name.clone(), font(Type::Body), c);
                let clip = ui.painter().with_clip_rect(name_rect.intersect(ui.clip_rect()));
                clip.galley(pos2(name_rect.left(), r.center().y - galley.size().y / 2.0), galley, c);
            }
            let used_note = match row.uses {
                Some(0) => tr!("not used").to_string(),
                Some(n) => trn!("used {n} time", "used {n} times", n),
                None => String::new(),
            };
            let name = if used_note.is_empty() { format!("{} · {}", row.name, row.detail) } else { format!("{} · {} · {}", row.name, row.detail, used_note) };
            a11y::control(ui, &resp, WidgetType::SelectableLabel, &name, Some(selected));
            let resp = resp.on_hover_text(&name);
            // Dragging rows: the selection if the row is in it, else just the row.
            if resp.drag_started() {
                let what = if selected { app.library.sel.clone() } else { vec![row.entry] };
                resp.dnd_set_drag_payload(what);
            }
            let target = match row.entry {
                LibraryEntry::Folder(f) => Some(f),
                e => app.s.doc.parent_of(e),
            };
            app.library.drop_zone.rows.push((r.intersect(ui.clip_rect()), target));
            if let Some(payload) = resp.dnd_hover_payload::<Vec<LibraryEntry>>()
                && !payload.contains(&row.entry)
            {
                let hl = if let LibraryEntry::Folder(_) = row.entry { r } else { Rect::from_min_size(pos2(r.left(), r.top()), vec2(r.width(), 2.0)) };
                ui.painter().rect_stroke(hl, T.rc(), Stroke::new(1.5, T.accent), StrokeKind::Inside);
            }
            if let Some(payload) = resp.dnd_release_payload::<Vec<LibraryEntry>>() {
                dropped = Some(((*payload).clone(), target));
            }
            if let LibraryEntry::Folder(f) = row.entry
                && resp.clicked()
                && ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| p.x < icon.right() + 2.0)
            {
                toggle = Some(f);
            } else if resp.clicked() {
                clicked = Some((row.entry, ui.input(|i| i.modifiers)));
            }
            if resp.double_clicked() {
                let on_name = ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| p.x >= name_rect.left());
                if on_name {
                    rename = Some(row.entry);
                } else {
                    activate = Some(row.entry);
                }
            }
            resp.context_menu(|ui| row_menu(app, ui, row.entry));
        }
        // Below the rows: dropping there moves to the top level.
        let rest = ui.available_rect_before_wrap();
        let (r, resp) = ui.allocate_exact_size(vec2(rest.width(), (rest.height()).max(TARGET)), Sense::click() - Sense::FOCUSABLE);
        a11y::surface(ui, &resp, tr!("Library: drop here for the top level"));
        if resp.dnd_hover_payload::<Vec<LibraryEntry>>().is_some() {
            ui.painter().rect_stroke(r, T.rc(), Stroke::new(1.5, T.accent), StrokeKind::Inside);
        }
        if let Some(payload) = resp.dnd_release_payload::<Vec<LibraryEntry>>() {
            dropped = Some(((*payload).clone(), None));
        }
        if resp.clicked() {
            app.library.sel.clear();
            app.library.keys = true;
        }
        resp.context_menu(|ui| {
            if ui.button(tr!("New Folder")).clicked() {
                ui.close();
                app.library.sel.clear();
                new_folder(app);
            }
        });
    });
    app.library.list_rect = Some(scroll.inner_rect);
    if let Some(f) = toggle {
        if !app.library.expanded.remove(&f) {
            app.library.expanded.insert(f);
        }
        app.library.select_only(LibraryEntry::Folder(f));
        app.library.keys = true;
    }
    if let Some((e, m)) = clicked {
        app.library.keys = true;
        if m.command {
            if let Some(i) = app.library.sel.iter().position(|x| *x == e) {
                app.library.sel.remove(i);
            } else {
                app.library.sel.push(e);
            }
            app.library.anchor = Some(e);
        } else if m.shift
            && let Some(a) = app.library.anchor
            && let (Some(i), Some(j)) = (list.iter().position(|r| r.entry == a), list.iter().position(|r| r.entry == e))
        {
            let (lo, hi) = (i.min(j), i.max(j));
            app.library.sel = list.get(lo..=hi).map(|rs| rs.iter().map(|r| r.entry).collect()).unwrap_or_default();
        } else {
            app.library.select_only(e);
        }
        if let LibraryEntry::Item(i) = e {
            app.library_sel = Some(i);
        }
    }
    if let Some(e) = rename {
        app.library.select_only(e);
        start_rename(app, e);
    } else if let Some(e) = activate {
        match e {
            // A symbol opens for editing, as in Animate (ADR 0045); images and sounds are placed.
            LibraryEntry::Item(LibraryItem::Symbol(id)) => {
                app.run("symbol.edit", json!({"symbol": id.0}));
            }
            LibraryEntry::Item(i) => place(app, i),
            LibraryEntry::Folder(f) => {
                if !app.library.expanded.remove(&f) {
                    app.library.expanded.insert(f);
                }
            }
        }
    }
    if let Some((what, to)) = dropped {
        move_into(app, &what, to);
    }
}

/// A row's right-click menu.
fn row_menu(app: &mut App, ui: &mut egui::Ui, e: LibraryEntry) {
    if !app.library.sel.contains(&e) {
        app.library.select_only(e);
        if let LibraryEntry::Item(i) = e {
            app.library_sel = Some(i);
        }
    }
    let many = app.library.sel.len() > 1;
    if let LibraryEntry::Item(LibraryItem::Symbol(id)) = e
        && !many
    {
        if ui.button(tr!("Edit Symbol")).clicked() {
            ui.close();
            app.run("symbol.edit", json!({"symbol": id.0}));
        }
        if ui.button(tr!("Duplicate Symbol")).clicked() {
            ui.close();
            if let Some(v) = app.run("library.duplicate", json!({"symbol": id.0}))
                && let Some(n) = v["symbol"].as_u64()
            {
                let copy = LibraryEntry::Item(LibraryItem::Symbol(anicraft_engine::doc::SymbolId(n)));
                app.library.select_only(copy);
                start_rename(app, copy);
            }
        }
    }
    let items = selected_items(app);
    if !items.is_empty() {
        let label = if items.iter().all(|i| matches!(i, LibraryItem::Sound(_))) { tr!("Place on Timeline") } else { tr!("Place on Stage") };
        if ui.button(label).on_hover_text(tr!("At the centre of the view, on the active layer (Enter; or drag onto the stage)")).clicked() {
            ui.close();
            place_items(app, &items, None);
        }
    }
    if !many && ui.button(tr!("Rename")).clicked() {
        ui.close();
        start_rename(app, e);
    }
    if ui.button(tr!("New Folder")).clicked() {
        ui.close();
        new_folder(app);
    }
    let folders: Vec<(FolderId, String)> = app.s.doc.library_tree.folders.iter().map(|f| (f.id, app.s.doc.entry_path(LibraryEntry::Folder(f.id)))).collect();
    ui.menu_button(tr!("Move to"), |ui| {
        let what = app.library.sel.clone();
        if ui.button(tr!("Library (top level)")).clicked() {
            ui.close();
            move_into(app, &what, None);
        }
        egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
            for (f, path) in &folders {
                let inside_moved = what.iter().any(|w| matches!(w, LibraryEntry::Folder(m) if app.s.doc.folder_within(*f, *m)));
                if ui.add_enabled(!inside_moved, egui::Button::new(path.as_str())).clicked() {
                    ui.close();
                    move_into(app, &what, Some(*f));
                }
            }
        });
    });
    ui.separator();
    if ui.button(tr!("Delete")).clicked() {
        ui.close();
        delete_selection(app);
    }
}

/// New Folder and Delete, and placing the current item.
fn actions_row(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        if crate::timeline::icon_button(ui, tr!("New Folder"), None, Icon::FolderPlus).clicked() {
            new_folder(app);
        }
        let can_delete = !app.library.sel.is_empty();
        let del = ui.add_enabled(can_delete, egui::Button::new(tr!("Delete"))).on_hover_text(tr!("Deletes the selected items and folders (Delete)"));
        if del.clicked() {
            delete_selection(app);
        }
        // The selected rows, else the item previewed.
        let mut items = selected_items(app);
        if items.is_empty()
            && let Some(i) = app.library_sel.filter(|i| app.s.doc.has_item(*i))
        {
            items.push(i);
        }
        if !items.is_empty() && items.iter().all(|i| matches!(i, LibraryItem::Sound(_))) {
            if ui.button(tr!("Place on timeline")).on_hover_text(tr!("Puts it on the active layer's keyframe at the playhead (Enter, or drag it onto the stage)")).clicked() {
                place_items(app, &items, None);
            }
        } else if ui.add_enabled(!items.is_empty(), egui::Button::new(tr!("Place on stage"))).on_hover_text(tr!("Adds it at the centre of the view, on the active layer (Enter, or drag it onto the stage)")).clicked() {
            place_items(app, &items, None);
        }
    });
}

/// The current item's settings: a symbol's type and linkage class, a bitmap's smoothing.
fn properties(app: &mut App, ui: &mut egui::Ui) {
    let Some(item) = app.library_sel.filter(|i| app.s.doc.has_item(*i)) else { return };
    ui.add_space(6.0);
    match item {
        LibraryItem::Symbol(id) => {
            let Some((kind, linkage)) = app.s.doc.symbol(id).map(|s| (s.kind, s.linkage.clone())) else { return };
            crate::theme::form_row(ui, crate::theme::FORM_LABEL_DOCK, tr!("Type"), |ui| {
                egui::ComboBox::from_id_salt("lib_symkind")
                    .width(150.0)
                    .selected_text(crate::panels::symbol_kind_label(kind))
                    .show_ui(ui, |ui| {
                        for (k, key) in [(SymbolKind::Graphic, "graphic"), (SymbolKind::MovieClip, "movie_clip"), (SymbolKind::Button, "button")] {
                            if ui.selectable_label(kind == k, crate::panels::symbol_kind_label(k)).clicked() && kind != k {
                                app.run("symbol.set_kind", json!({"symbol": id.0, "kind": key}));
                            }
                        }
                    })
                    .response
                    .set_name(tr!("Symbol type"));
            });
            // The class scripts create it by (ADR 0013 tier 1: kept, not run).
            let (class, base) = linkage.map_or((String::new(), String::new()), |l| (l.class, l.base));
            crate::theme::form_row(ui, crate::theme::FORM_LABEL_DOCK, tr!("Class"), |ui| {
                if let Some(v) = crate::actions_panel::class_field(ui, "lib_symclass", &class, tr!("Linkage class"), tr!("The class scripts create this symbol by (its linkage). Kept with the document; scripts don't run yet")) {
                    let base = if v.is_empty() { String::new() } else { base.clone() };
                    app.run("script.set_class", json!({"symbol": id.0, "class": v, "base": base}));
                }
            });
            // 9-slice scaling (ADR 0055), as in Animate's Symbol Properties.
            crate::panels::scale_grid_props(app, ui, id, "library");
        }
        LibraryItem::Bitmap(id) => {
            let Some(mut on) = app.s.doc.bitmap(id).map(|b| b.smoothing) else { return };
            if crate::theme::checkbox(ui, &mut on, tr!("Allow smoothing")).on_hover_text(tr!("Smooth when scaled or turned; off keeps hard pixel edges, for pixel art")).changed() {
                app.run("library.set_smoothing", json!({"bitmap": id.0, "on": on}));
            }
        }
        LibraryItem::Sound(id) => {
            if let Some(x) = app.s.doc.sound(id) {
                let info = format!("{} · {} Hz · {}", x.format.label(), x.rate, if x.channels > 1 { tr!("Stereo") } else { tr!("Mono") });
                with_type(ui, Type::Data, |ui| ui.label(text(Type::Caption, info)));
            }
        }
    }
}

// ----- preview --------------------------------------------------------------------------------

const PREVIEW_H: f32 = 72.0;

/// The current item's picture: a symbol's first frame, a bitmap, or a sound's waveform with a
/// play button.
fn preview(app: &mut App, ui: &mut egui::Ui) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), PREVIEW_H), Sense::hover());
    ui.painter().rect_filled(r, T.rc(), T.field);
    let Some(item) = app.library_sel.filter(|i| app.s.doc.has_item(*i)) else {
        ui.painter().text(r.center(), Align2::CENTER_CENTER, tr!("Select an item to preview it"), font(Type::Caption), color(Type::Caption));
        return;
    };
    let inner = r.shrink(6.0);
    match item {
        LibraryItem::Sound(id) => {
            waveform(app, ui, inner, id);
            let play = Rect::from_min_size(pos2(r.right() - TARGET - 4.0, r.top() + 4.0), vec2(TARGET, TARGET));
            let resp = ui.put(play, egui::Button::new("▶").min_size(vec2(TARGET, TARGET)));
            a11y::control(ui, &resp, WidgetType::Button, tr!("Play the sound"), None);
            if resp.on_hover_text(tr!("Plays the sound on its own")).clicked()
                && let Some(dec) = app.s.doc.sound(id).and_then(|x| x.decoded())
            {
                app.playing = false;
                app.sound.preview(dec.pcm.clone());
                if let Some(e) = app.sound.error.take() {
                    app.status_err(tr!("No sound: {error}", error = crate::i18n::t_dyn(&e.to_string())));
                }
            }
        }
        _ => {
            let key = (item, app.s.state);
            if app.library.preview.as_ref().is_none_or(|p| p.key != key) {
                let texture = picture(ui.ctx(), &app.s.doc, item, inner);
                app.library.preview = Some(Preview { key, texture });
            }
            if let Some(tex) = app.library.preview.as_ref().and_then(|p| p.texture.as_ref()) {
                let size = tex.size_vec2() / ui.ctx().pixels_per_point();
                let fit = (inner.width() / size.x.max(1.0)).min(inner.height() / size.y.max(1.0)).min(1.0);
                let at = Rect::from_center_size(inner.center(), size * fit);
                ui.painter().image(tex.id(), at, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
            }
        }
    }
}

/// A texture of `item` fitting `area` (in points): a symbol's first frame rendered by the
/// oracle, or the bitmap, drawn with or without smoothing as it will be on the stage.
fn picture(ctx: &egui::Context, doc: &Document, item: LibraryItem, area: Rect) -> Option<egui::TextureHandle> {
    let ppp = ctx.pixels_per_point().clamp(0.5, 4.0);
    let (w, h) = ((area.width() * ppp).round().max(1.0) as u32, (area.height() * ppp).round().max(1.0) as u32);
    match item {
        LibraryItem::Symbol(id) => {
            let dl = symbol_frame(doc, id);
            let b = dl.items.iter().filter(|i| !i.guide && i.masks.is_none()).fold(anicraft_engine::geom::Rect::EMPTY, |acc, i| {
                let reach = i.stroke.as_ref().map_or(0.0, |s| s.reach());
                acc.union(&i.path.bounds().inflate(reach).transformed(&i.transform))
            });
            if b.is_empty() || !(b.width().is_finite() && b.height().is_finite()) {
                return None;
            }
            let s = ((w as f32 - 4.0) / b.width().max(1e-3)).min((h as f32 - 4.0) / b.height().max(1e-3)).min(4.0 * ppp);
            let c = b.center();
            let transform = anicraft_engine::geom::Affine::translate(w as f32 / 2.0, h as f32 / 2.0).mul(&anicraft_engine::geom::Affine::scale(s, s)).mul(&anicraft_engine::geom::Affine::translate(-c.x, -c.y));
            let view = anicraft_engine::render::View { transform, width: w, height: h, background: None };
            let (px, _) = anicraft_engine::render::render(&dl, &view).ok()?;
            let image = egui::ColorImage::from_rgba_premultiplied([w as usize, h as usize], &px.to_premul_rgba8());
            Some(ctx.load_texture("library-preview", image, egui::TextureOptions::LINEAR))
        }
        LibraryItem::Bitmap(id) => {
            let b = doc.bitmap(id)?;
            let px = b.pixels()?;
            // At most the area's size in pixels (nearest pixel), so a huge image costs little.
            let k = (w as f32 / px.width.max(1) as f32).min(h as f32 / px.height.max(1) as f32).min(1.0);
            let (tw, th) = (((px.width as f32 * k).round() as u32).max(1), ((px.height as f32 * k).round() as u32).max(1));
            let mut rgba = Vec::with_capacity((tw * th * 4) as usize);
            for y in 0..th {
                for x in 0..tw {
                    let c = px.at((x as f32 / k) as i64, (y as f32 / k) as i64);
                    rgba.extend(c.iter().map(|v| (v * 255.0 + 0.5) as u8));
                }
            }
            let image = egui::ColorImage::from_rgba_unmultiplied([tw as usize, th as usize], &rgba);
            let options = if b.smoothing { egui::TextureOptions::LINEAR } else { egui::TextureOptions::NEAREST };
            Some(ctx.load_texture("library-preview", image, options))
        }
        LibraryItem::Sound(_) => None,
    }
}

/// Symbol `id`'s first frame on its own, its registration point at the origin.
fn symbol_frame(doc: &Document, id: anicraft_engine::doc::SymbolId) -> anicraft_engine::doc::DisplayList {
    use anicraft_engine::doc::{Element, ElementId, ElementKind, Layer, LayerId, LoopMode, Scene};
    use std::sync::Arc;
    let mut d = doc.clone();
    let mut layer = Layer::new(LayerId(u64::MAX - 1), "");
    if let Some(k) = layer.keyframe_mut(0) {
        k.elements = vec![Arc::new(Element {
            id: ElementId(u64::MAX - 2),
            name: String::new(),
            transform: Default::default(),
            color: Default::default(),
            kind: ElementKind::Symbol { symbol: id, loop_mode: LoopMode::SingleFrame, first_frame: 0, blend: Default::default(), filters: Vec::new(), events: Vec::new() },
        })];
    }
    let mut scene = Scene::new("");
    scene.timeline.layers.push(Arc::new(layer));
    d.scenes = vec![Arc::new(scene)];
    anicraft_engine::doc::evaluate_scene(&d, 0, 0)
}

/// A sound's waveform across `r`: one line per point column, lowest to highest sample.
fn waveform(app: &App, ui: &egui::Ui, r: Rect, id: anicraft_engine::doc::SoundId) {
    let Some(dec) = app.s.doc.sound(id).and_then(|x| x.decoded_if_ready()) else {
        ui.painter().text(r.center(), Align2::CENTER_CENTER, tr!("Reading the sound…"), font(Type::Caption), color(Type::Caption));
        return;
    };
    let len = dec.peaks.frames();
    let cols = r.width().floor().max(1.0) as usize;
    if len == 0 {
        return;
    }
    let (cy, half) = (r.center().y, r.height() / 2.0);
    let stroke = Stroke::new(1.0, T.waveform);
    for c in 0..cols {
        let a = c * len / cols;
        let b = ((c + 1) * len / cols).max(a + 1).min(len);
        if let Some((lo, hi)) = dec.peaks.range(a, b) {
            let x = r.left() + c as f32 + 0.5;
            ui.painter().line_segment([pos2(x, cy - hi.clamp(-1.0, 1.0) * half), pos2(x, cy - lo.clamp(-1.0, 1.0) * half)], stroke);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> (anicraft_engine::Session, u64, u64) {
        let mut s = anicraft_engine::Session::new(anicraft_engine::doc::sample::demo());
        let ball = s.doc.library.iter().find(|x| x.name == "Ball").map(|x| x.id.0).unwrap();
        let f = s.execute("library.new_folder", &json!({"name": "Characters"})).unwrap()["folder"].as_u64().unwrap();
        s.execute("library.duplicate", &json!({"symbol": ball})).unwrap();
        s.execute("library.move", &json!({"items": [{"symbol": ball}], "to": f})).unwrap();
        (s, ball, f)
    }

    fn names(s: &anicraft_engine::Session, v: &mut LibraryView) -> Vec<(usize, String)> {
        let (uses, _) = v.counts(s);
        rows(&s.doc, v, &uses).into_iter().map(|r| (r.depth, r.name)).collect()
    }

    #[test]
    fn folders_come_first_and_open_ones_show_what_they_hold() {
        let (s, _, f) = session();
        let mut v = LibraryView::default();
        let n = |d: usize, x: &str| (d, x.to_string());
        assert_eq!(names(&s, &mut v), [n(0, "Characters"), n(0, "Ball copy"), n(0, "Star")]);
        v.expanded.insert(FolderId(f));
        assert_eq!(names(&s, &mut v), [n(0, "Characters"), n(1, "Ball"), n(0, "Ball copy"), n(0, "Star")]);
        // By use count, most used first when descending; folders stay on top.
        v.sort = Sort::Uses;
        v.descending = true;
        assert_eq!(names(&s, &mut v), [n(0, "Characters"), n(1, "Ball"), n(0, "Star"), n(0, "Ball copy")]);
        // Date added: the copy is newest.
        v.sort = Sort::Added;
        v.descending = false;
        assert_eq!(names(&s, &mut v)[2..], [n(0, "Star"), n(0, "Ball copy")]);
    }

    fn pointer(pos: egui::Pos2, pressed: Option<bool>) -> egui::RawInput {
        let mut events = vec![egui::Event::PointerMoved(pos)];
        if let Some(pressed) = pressed {
            events.push(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        }
        egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1600.0, 940.0))), events, ..Default::default() }
    }

    fn app() -> (egui::Context, App) {
        let ctx = egui::Context::default();
        let app = App::with_context(&ctx, anicraft_engine::Session::new(anicraft_engine::doc::sample::demo()), Some(std::path::PathBuf::from("unused.png")));
        (ctx, app)
    }

    fn frame(ctx: &egui::Context, app: &mut App, input: egui::RawInput) {
        let mut out = ctx.run_ui(input, |ui| app.frame(ui));
        out.textures_delta.clear();
    }

    fn instances(app: &App, symbol: u64) -> Vec<anicraft_engine::geom::Vec2> {
        app.s.timeline().layers.iter().filter_map(|l| l.keyframe_at(app.s.frame)).flat_map(|k| &k.elements).filter(|e| matches!(e.kind, anicraft_engine::doc::ElementKind::Symbol { symbol: x, .. } if x.0 == symbol)).map(|e| e.transform.apply(anicraft_engine::geom::Vec2::ZERO)).collect()
    }

    #[test]
    fn dropping_rows_on_the_stage_places_them_at_the_pointer_in_one_step() {
        let (ctx, mut app) = app();
        for _ in 0..3 {
            frame(&ctx, &mut app, pointer(pos2(5.0, 935.0), None));
        }
        let star = app.s.doc.library.iter().find(|x| x.name == "Star").unwrap().id;
        let before = instances(&app, star.0).len();
        let steps = app.s.history().undo_len();
        let canvas = app.stage_cache.canvas;
        assert!(canvas.width() > 100.0, "the stage was laid out");
        // The press starts outside the stage (as on a Library row), the rows ride along.
        frame(&ctx, &mut app, pointer(pos2(5.0, 935.0), Some(true)));
        egui::DragAndDrop::set_payload(&ctx, vec![LibraryEntry::Item(LibraryItem::Symbol(star)), LibraryEntry::Folder(FolderId(424_242))]);
        let at = canvas.center();
        for _ in 0..2 {
            frame(&ctx, &mut app, pointer(at, None));
        }
        assert_eq!(instances(&app, star.0).len(), before, "hovering places nothing");
        frame(&ctx, &mut app, pointer(at, Some(false)));
        let now = instances(&app, star.0);
        assert_eq!(now.len(), before + 1, "letting go places the symbol (folders are left out)");
        assert_eq!(app.s.history().undo_len(), steps + 1, "one undo step");
        assert!(!app.library.keys, "the stage has the keyboard again");
        // Where the pointer was, in stage space.
        let size = vec2(app.s.doc.stage.width as f32, app.s.doc.stage.height as f32);
        let origin = canvas.center() + app.view.pan - size * app.view.zoom * 0.5;
        let want = (at - origin) / app.view.zoom;
        let got = now.iter().find(|p| (p.x - want.x).abs() < 1.5 && (p.y - want.y).abs() < 1.5);
        assert!(got.is_some(), "{now:?} near {want:?}");
    }

    #[test]
    fn enter_places_the_selected_rows_at_the_centre_of_the_view() {
        let (ctx, mut app) = app();
        for _ in 0..3 {
            frame(&ctx, &mut app, pointer(pos2(5.0, 935.0), None));
        }
        let star = app.s.doc.library.iter().find(|x| x.name == "Star").unwrap().id;
        let before = instances(&app, star.0).len();
        app.library.sel = vec![LibraryEntry::Item(LibraryItem::Symbol(star)), LibraryEntry::Folder(FolderId(424_242))];
        app.library.keys = true;
        let mut input = pointer(pos2(5.0, 935.0), None);
        input.events.push(egui::Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
        frame(&ctx, &mut app, input);
        let now = instances(&app, star.0);
        assert_eq!(now.len(), before + 1);
        let c = app.view_center_stage();
        assert!(now.iter().any(|p| (p.x - c.x).abs() < 0.01 && (p.y - c.y).abs() < 0.01), "{now:?} at {c:?}");
        // Inside a symbol open for editing, the centre of the view in its space.
        app.run("symbol.edit", json!({"symbol": star.0}));
        let ball = app.s.doc.library.iter().find(|x| x.name == "Ball").unwrap().id;
        place_items(&mut app, &[LibraryItem::Symbol(ball)], None);
        let (x, y) = view_centre(&app);
        assert!(instances(&app, ball.0).iter().any(|p| (p.x - x).abs() < 0.01 && (p.y - y).abs() < 0.01));
    }

    #[test]
    fn a_search_shows_matches_inside_closed_folders() {
        let (s, _, _) = session();
        let mut v = LibraryView { filter: "BALL".into(), ..Default::default() };
        let n = |d: usize, x: &str| (d, x.to_string());
        assert_eq!(names(&s, &mut v), [n(0, "Characters"), n(1, "Ball"), n(0, "Ball copy")]);
        v.filter = "nothing like it".into();
        assert!(names(&s, &mut v).is_empty());
    }
}
