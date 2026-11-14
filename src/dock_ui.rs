//! Draws the dock (ADR 0075, ADR 0087): the tools' bar, the four docks around the stage with
//! their tab groups (or collapsed to a strip of icons, each opening its panel over the stage),
//! and dragging a tab to dock, stack or float its panel. Floating windows are `dock_float`.
//!
//! The arrangement itself is `crate::dock` (pure, tested headless); this module only draws it
//! and turns clicks and drags into changes of it. Each change is kept in the preferences.

use crate::a11y::{self, Region};
use crate::app::App;
use crate::dock::{Group, Layout, Panel, Place, Side, Target, Zone};
use crate::icons::Icon;
use crate::theme::{self, T, Type, font, text};
use egui::{Id, Pos2, Rect, Sense, Stroke, StrokeKind, WidgetType, pos2, vec2};
use std::collections::{HashMap, HashSet};

/// A side dock's group header (the band look of Properties and the Library before docking).
const BAND_H: f32 = 32.0;
/// The least a group's body is given by dragging the line between two groups.
const MIN_BODY: f32 = 48.0;
/// The panel menu button.
pub(crate) const MENU_W: f32 = 26.0;
/// A side dock tab's padding around its name, and the narrowest it shrinks to.
const TAB_PAD: f32 = 16.0;
const MIN_TAB: f32 = 44.0;
const TAB_GAP: f32 = 2.0;

/// The widths of tabs wanting `want` points each in a strip of `room` points (ADR 0083):
/// their own when they all fit; else the longest shrink to one cap (their names ending in an
/// ellipsis) so short names stay whole; never narrower than `MIN_TAB` (or their own width),
/// and when even that doesn't fit the strip scrolls.
pub fn fit_tabs(want: &[f32], room: f32) -> Vec<f32> {
    let gaps = TAB_GAP * want.len().saturating_sub(1) as f32;
    let total: f32 = want.iter().sum::<f32>() + gaps;
    if total <= room || want.is_empty() {
        return want.to_vec();
    }
    // The cap c with sum(min(w, c)) = room - gaps, found among the sorted widths.
    let mut sorted: Vec<f32> = want.iter().copied().filter(|w| w.is_finite()).collect();
    sorted.sort_by(f32::total_cmp);
    let mut left = (room - gaps).max(0.0);
    let mut cap = 0.0;
    for (i, w) in sorted.iter().enumerate() {
        let n = (sorted.len() - i) as f32;
        if w * n >= left {
            cap = left / n;
            break;
        }
        left -= w;
        cap = *w;
    }
    want.iter().map(|w| w.min(cap.max(MIN_TAB))).collect()
}

/// How far from an edge of the stage a dragged tab docks on that side, as a part of the stage.
const EDGE: f32 = 0.22;
/// A dock collapsed to icons: its strip's width (or height, above and below the stage).
pub const STRIP: f32 = theme::TOOL_BUTTON + 8.0;
/// The tools' bar: its width (or height) and its grip's.
const BAR: f32 = theme::TOOL_BUTTON + 12.0;
const GRIP: f32 = 10.0;
/// The narrowest a side dock goes: every panel is checked to draw inside it (ADR 0087).
pub const MIN_SIDE: f32 = 200.0;
/// The lowest a dock above or below the stage goes.
const MIN_ROW: f32 = 120.0;

/// What is being dragged: a panel by its tab, or a floating window by its tab strip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drag {
    Panel(Panel),
    Window(u32),
}

/// The dock's view state (not saved): a drag, where groups were drawn this frame (the drop
/// targets), the panel open over the stage from an icon, and floating windows as they are now.
#[derive(Default)]
pub struct DockView {
    pub drag: Option<Drag>,
    /// Bumped when a workspace is switched or reset, so the docks take its sizes.
    pub epoch: u64,
    /// The arrangement changed: written to the preferences once nothing is being dragged.
    pub save: bool,
    /// The panel open over the stage from a dock collapsed to icons (Esc or a click outside
    /// closes it).
    pub flyout: Option<Panel>,
    pub(crate) drawn: Vec<Drawn>,
    pub(crate) stage: Option<Rect>,
    /// Floating windows placed this session (later frames leave them where the user put them).
    pub(crate) placed: HashSet<u32>,
    /// Where windows of their own are now (desktop points), kept when AniCraft quits.
    pub(crate) live: HashMap<u32, ([f32; 2], [f32; 2])>,
    /// Windows of their own on the desktop (points): drop targets for a drag.
    pub(crate) desk: HashMap<u32, Rect>,
    /// A drag that began in a window of its own: the pointer, in the editor's points, and
    /// whether its button is still down, as that window last saw them.
    pub(crate) remote: Option<(Pos2, bool)>,
    /// The window of its own a drag began in.
    pub(crate) from_window: Option<egui::ViewportId>,
    /// The editor window's top-left corner on the desktop (points).
    origin: Option<Pos2>,
    /// Floating panels inside the editor this run, whatever the platform could do (screenshots
    /// of a floating window, which only capture the editor's own window).
    pub inside: bool,
}

/// Floating panels go in windows of their own (else inside the editor, ADR 0062's fallback).
pub(crate) fn own_windows(app: &App, ctx: &egui::Context) -> bool {
    !app.dock_view.inside && matches!(crate::platform::movie_host(ctx.embed_viewports(), true), crate::platform::MovieHost::Window)
}

/// What a drop target is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Over {
    Group(Side),
    /// A dock collapsed to icons.
    Strip(Side),
    /// A floating window inside the editor.
    Window(u32),
}

/// A drop target as drawn this frame.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Drawn {
    pub over: Over,
    pub first: Panel,
    /// The first panel of the group after it, in the same dock.
    pub next: Option<Panel>,
    pub rect: Rect,
    pub header: Rect,
}

/// Where a panel's menu was opened from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Where {
    Docked { side: Side, folded: bool, tabs: usize },
    Floating { window: u32, tabs: usize },
    Bar(Side),
}

/// What the dock's controls asked for this frame, applied once it has drawn.
pub(crate) enum Act {
    Show(Panel),
    Fold(Panel),
    Shares(Side, Vec<f32>),
    Move(Panel, Target),
    MoveWindow(u32, Target),
    Close(Panel),
    CloseGroup(Panel),
    CloseWindow(u32),
    DockBack(Panel),
    DockBackWindow(u32),
    Placed(u32, [f32; 2], [f32; 2]),
    Collapse(Side, bool),
    Flyout(Option<Panel>),
}

/// Where a control is, in its id: a panel's tab or menu is drawn in one place a frame, but a
/// move can take it from a dock to a window between the two.
fn wkey(at: Where) -> u64 {
    match at {
        Where::Docked { side, .. } => side as u64,
        Where::Bar(side) => 10 + side as u64,
        Where::Floating { window, .. } => 100 + u64::from(window),
    }
}

/// Ctrl+F6's stop for a panel (the tools keep their own).
pub(crate) fn region(p: Panel) -> Region {
    if p == Panel::Tools { Region::Tools } else { Region::Panel(p) }
}

/// A panel's icon, in a dock collapsed to icons.
pub fn panel_icon(p: Panel) -> Icon {
    match p {
        Panel::Properties => Icon::PanelProperties,
        Panel::Align => Icon::AlignLeft,
        Panel::Colour => Icon::Ink,
        Panel::Swatches => Icon::PanelSwatches,
        Panel::Library => Icon::PanelLibrary,
        Panel::MovieExplorer => Icon::PanelExplorer,
        Panel::History => Icon::PanelHistory,
        Panel::LayerDepth => Icon::Onion,
        Panel::Timeline => Icon::Layer,
        Panel::MotionEditor => Icon::Guide,
        Panel::Actions => Icon::PanelActions,
        Panel::Output => Icon::PanelOutput,
        Panel::Tools => Icon::Select,
    }
}

// ----- the app's side ---------------------------------------------------------------------------

impl App {
    /// On screen: in front in its group, window or the tools' bar, and, in a dock collapsed to
    /// icons, open over the stage.
    pub fn panel_visible(&self, p: Panel) -> bool {
        self.dock.is_shown(p) && (self.dock.collapsed_side(p).is_none() || self.dock_view.flyout == Some(p))
    }

    /// Brings a panel forward, reopening it where it was if it was closed; in a dock collapsed
    /// to icons it opens over the stage.
    pub fn show_panel(&mut self, p: Panel) {
        if !self.dock.is_shown(p) {
            self.dock.show(p);
            self.dock_changed();
        }
        if self.dock.collapsed_side(p).is_some() {
            self.dock_view.flyout = Some(p);
        }
    }

    /// The Window menu's items: close a panel on screen, bring any other forward.
    pub fn toggle_panel(&mut self, p: Panel) {
        if !self.panel_visible(p) && self.dock.is_open(p) {
            self.show_panel(p);
            return;
        }
        self.dock.toggle(p);
        if self.dock_view.flyout == Some(p) {
            self.dock_view.flyout = None;
        }
        if self.dock.collapsed_side(p).is_some() && self.dock.is_shown(p) {
            self.dock_view.flyout = Some(p);
        }
        self.dock_changed();
    }

    /// Keeps the arrangement as the current workspace's, to be saved.
    pub fn dock_changed(&mut self) {
        self.prefs.workspaces.keep(&self.dock);
        self.dock_view.save = true;
    }

    /// Window › Workspace: switches to a workspace, as it was last arranged.
    pub fn switch_workspace(&mut self, key: &str) {
        self.prefs.workspaces.keep(&self.dock);
        if let Some(l) = self.prefs.workspaces.switch(key) {
            self.set_layout(l);
        }
    }

    /// Window › Workspace › Reset: the current workspace as built or saved.
    pub fn reset_workspace(&mut self) {
        let l = self.prefs.workspaces.reset();
        self.set_layout(l);
        let name = crate::i18n::t_dyn(crate::dock::Workspaces::name(&self.prefs.workspaces.current)).into_owned();
        self.status_ok(tr!("Workspace “{name}” reset", name = name));
    }

    /// Window › Workspace › Save Workspace…: the arrangement as a workspace of your own.
    pub fn save_workspace(&mut self, name: &str) {
        let layout = self.dock.clone();
        match self.prefs.workspaces.save_as(name, &layout) {
            Ok(key) => {
                self.dock_view.save = true;
                self.status_ok(tr!("Workspace “{name}” saved", name = key));
            }
            Err(e) => self.status_err(crate::i18n::t(e).to_string()),
        }
    }

    /// Manage Workspaces: renames a workspace of yours.
    pub fn rename_workspace(&mut self, key: &str, to: &str) -> bool {
        match self.prefs.workspaces.rename(key, to) {
            Ok(new) => {
                self.dock_view.save = true;
                self.status_ok(tr!("Workspace “{old}” renamed “{new}”", old = key, new = new));
                true
            }
            Err(e) => {
                self.status_err(crate::i18n::t(e).to_string());
                false
            }
        }
    }

    /// Manage Workspaces: deletes a workspace of yours (the one in use: back to Essentials).
    pub fn delete_workspace(&mut self, key: &str) {
        match self.prefs.workspaces.delete(key) {
            Ok(layout) => {
                if let Some(l) = layout {
                    self.set_layout(l);
                }
                self.dock_view.save = true;
                self.status_ok(tr!("Workspace “{name}” deleted", name = key));
            }
            Err(e) => self.status_err(crate::i18n::t(e).to_string()),
        }
    }

    fn set_layout(&mut self, l: Layout) {
        self.dock = l;
        self.dock_view.epoch += 1;
        self.dock_view.placed.clear();
        self.dock_view.flyout = None;
        self.dock_changed();
    }
}

/// Writes a changed arrangement to the preferences once nothing is being dragged (not in
/// screenshot runs).
pub fn save_when_settled(app: &mut App, ctx: &egui::Context) {
    if !app.dock_view.save || app.dock_view.drag.is_some() || ctx.input(|i| i.pointer.any_down()) {
        return;
    }
    app.dock_view.save = false;
    if app.shot.is_none()
        && let Err(e) = app.prefs.save()
    {
        app.status_err(tr!("Could not save preferences: {e}", e = e));
    }
}

/// When AniCraft quits: where the windows of their own are, into the preferences.
pub fn remember(app: &mut App) {
    if app.dock_view.live.is_empty() {
        return;
    }
    let live = std::mem::take(&mut app.dock_view.live);
    for f in &mut app.dock.floating {
        if let Some((at, size)) = live.get(&f.id) {
            f.at = crate::platform::window_positions().then_some(*at).or(f.at);
            f.size = *size;
        }
    }
    app.dock.tidy();
    app.dock_changed();
    if app.shot.is_none() {
        let _ = app.prefs.save();
    }
}

// ----- the workspace ------------------------------------------------------------------------------

/// The tools, the docks and the stage, and a dragged tab's drop target over them.
pub fn workspace(app: &mut App, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    // Docks start at their usual size, but never take most of a small window or a large
    // interface scale (200 % on a laptop leaves under 1000 × 600 points), and the stage always
    // keeps room when the scale goes up later.
    let room = ui.available_rect_before_wrap();
    let room = vec2(room.width(), room.height());
    let (dock_w, dock_h) = ((room.x * 0.3).clamp(220.0, 292.0), (room.y * 0.42).clamp(200.0, 292.0));
    let layout = app.dock.clone();
    // Floating windows inside the editor were drawn after the docks last frame: keep them as
    // drop targets (they are drawn over the docks, so they come first).
    app.dock_view.drawn.retain(|d| matches!(d.over, Over::Window(_)));
    let mut acts = Vec::new();
    if let Some(side) = layout.tools_bar {
        tools_bar(app, ui, side, &layout, &mut acts);
    }
    for side in [Side::Left, Side::Right, Side::Bottom, Side::Top] {
        let zone = layout.zone(side);
        if zone.groups.is_empty() {
            continue;
        }
        if zone.collapsed {
            strip(app, ui, side, zone, &layout, &mut acts);
            continue;
        }
        let id = Id::new(("dock_zone", side, app.dock_view.epoch));
        let (auto, max, min) = if side.stacks() { (dock_w, (room.x * 0.5).max(MIN_SIDE + 20.0), MIN_SIDE) } else { (dock_h, (room.y * 0.6).max(200.0), MIN_ROW) };
        let size = zone.size.unwrap_or(auto).clamp(min, max);
        let panel = match side {
            Side::Left => egui::Panel::left(id),
            Side::Right => egui::Panel::right(id),
            Side::Top => egui::Panel::top(id),
            Side::Bottom => egui::Panel::bottom(id),
        };
        let shown = panel.resizable(true).default_size(size).size_range(min..=max).frame(egui::Frame::NONE.fill(T.panel)).show(ui, |ui| zone_ui(app, ui, side, &zone.groups, &layout, &mut acts));
        let resize = id.with("__resize");
        if ctx.is_being_dragged(resize) {
            let r = shown.response.rect;
            app.dock.zone_mut(side).size = Some((if side.stacks() { r.width() } else { r.height() }).round());
            app.dock_changed();
        }
        let name = match side {
            Side::Left => tr!("Resize the left dock"),
            Side::Right => tr!("Resize the right-hand panels"),
            Side::Top => tr!("Resize the dock above the stage"),
            Side::Bottom => tr!("Resize the dock below the stage"),
        };
        a11y::name_resize(&ctx, resize, name);
    }
    let stage = egui::CentralPanel::default().frame(egui::Frame::NONE.fill(T.pasteboard)).show(ui, |ui| crate::stage::show(app, ui));
    app.dock_view.stage = Some(stage.response.rect);
    app.dock_view.origin_from(&ctx);
    flyout(app, &ctx, (dock_w, dock_h), &layout, &mut acts);
    drag_and_drop(app, &ctx, &mut acts);
    apply(app, acts);
    // Ctrl+F6: the stage, each panel as docked (and floating inside the editor), the tools.
    let inside = !own_windows(app, &ctx);
    let mut order = vec![Region::Stage];
    order.extend(app.dock.order().into_iter().filter(|p| inside || !app.dock.is_floating(*p)).map(region));
    a11y::set_order(&ctx, order);
}

impl DockView {
    /// The editor window's top-left corner on the desktop (points), for drags between windows.
    fn origin_from(&mut self, ctx: &egui::Context) {
        if let Some(r) = ctx.input(|i| i.viewport().inner_rect) {
            self.origin = Some(r.min);
        }
    }

    pub(crate) fn origin(&self) -> Pos2 {
        self.origin.unwrap_or(Pos2::ZERO)
    }
}

/// The tools as a slim bar on a side of the editor, outside the docks, with a grip to drag
/// them elsewhere (and their menu).
fn tools_bar(app: &mut App, ui: &mut egui::Ui, side: Side, layout: &Layout, acts: &mut Vec<Act>) {
    let id = if side == Side::Left { Id::new("tools") } else { Id::new(("tools", side)) };
    let margin = if side.stacks() { egui::Margin { left: 0, right: 0, top: 2, bottom: 8 } } else { egui::Margin { left: 2, right: 8, top: 0, bottom: 0 } };
    let frame = egui::Frame::NONE.fill(T.panel).inner_margin(margin);
    let panel = match side {
        Side::Left => egui::Panel::left(id),
        Side::Right => egui::Panel::right(id),
        Side::Top => egui::Panel::top(id),
        Side::Bottom => egui::Panel::bottom(id),
    };
    panel.resizable(false).exact_size(BAR).frame(frame).show(ui, |ui| {
        if side.stacks() {
            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), GRIP), Sense::hover());
            grip(ui, r, true, Where::Bar(side), layout, acts);
            ui.add_space(4.0);
            crate::panels::tools(app, ui, crate::panels::ToolFlow::Column);
        } else {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(GRIP, ui.available_height()), Sense::hover());
                grip(ui, r, false, Where::Bar(side), layout, acts);
                ui.add_space(4.0);
                crate::panels::tools(app, ui, crate::panels::ToolFlow::Row);
            });
        }
    });
}

/// The tools bar's grip: drag it to move the tools, click it for their menu.
fn grip(ui: &mut egui::Ui, r: Rect, across: bool, at: Where, layout: &Layout, acts: &mut Vec<Act>) {
    let resp = ui.interact(r, Id::new("dock_grip_tools"), Sense::click_and_drag());
    a11y::control(ui, &resp, WidgetType::Button, tr!("Tools panel menu"), None);
    let hot = resp.hovered() || resp.has_focus() || resp.dragged();
    if resp.hovered() {
        ui.painter().rect_filled(r, T.rc(), T.hover);
    }
    a11y::focus_ring(ui, &resp);
    // Six dots: a handle, as on a toolbar.
    let c = r.center();
    let colour = if hot { T.icon_hot } else { T.text_faint };
    for i in -1..=1 {
        for j in [-1.5_f32, 1.5] {
            let d = if across { vec2(i as f32 * 4.0, j) } else { vec2(j, i as f32 * 4.0) };
            ui.painter().circle_filled(c + d, 1.0, colour);
        }
    }
    if resp.drag_started() {
        ui.ctx().data_mut(|d| d.insert_temp(Id::new("dock_drag_start"), Some(Drag::Panel(Panel::Tools))));
    }
    let resp = resp.on_hover_text(tr!("Drag to move the tools; click for more"));
    egui::Popup::menu(&resp).show(|ui| panel_menu(ui, at, Panel::Tools, layout, acts));
    resp.context_menu(|ui| panel_menu(ui, at, Panel::Tools, layout, acts));
}

/// One dock: its groups top to bottom (left and right) or side by side (top and bottom), with
/// a line between each two to drag.
fn zone_ui(app: &mut App, ui: &mut egui::Ui, side: Side, groups: &[Group], layout: &Layout, acts: &mut Vec<Act>) {
    let area = ui.max_rect();
    ui.allocate_rect(area, Sense::hover());
    let n = groups.len();
    let along = |r: Rect| if side.stacks() { r.height() } else { r.width() };
    let heads = if side.stacks() { n as f32 * BAND_H } else { 0.0 };
    let free = (along(area) - heads).max(0.0);
    let sum = groups.iter().filter(|g| !g.folded || !side.stacks()).map(|g| g.share).sum::<f32>().max(1e-3);
    let mut bodies = Vec::with_capacity(n);
    let mut at = if side.stacks() { area.top() } else { area.left() };
    for (i, g) in groups.iter().enumerate() {
        let folded = g.folded && side.stacks();
        let body = if folded { 0.0 } else { free * g.share / sum };
        let len = body + if side.stacks() { BAND_H } else { 0.0 };
        let rect = if side.stacks() { Rect::from_min_size(pos2(area.left(), at), vec2(area.width(), len)) } else { Rect::from_min_size(pos2(at, area.top()), vec2(len, area.height())) };
        at += len;
        bodies.push(body);
        let next = groups.get(i + 1).and_then(|g| g.tabs.first().copied());
        group_ui(app, ui, side, g, rect, next, i == 0, layout, acts);
    }
    // The lines between groups: drag to share the dock out differently.
    for i in 0..n.saturating_sub(1) {
        let (Some(a), Some(b)) = (groups.get(i), groups.get(i + 1)) else { continue };
        if side.stacks() && (a.folded || b.folded) {
            continue;
        }
        let edge = bodies.iter().take(i + 1).sum::<f32>() + if side.stacks() { (i + 1) as f32 * BAND_H } else { 0.0 };
        let grab = if side.stacks() {
            Rect::from_center_size(pos2(area.center().x, area.top() + edge), vec2(area.width(), 6.0))
        } else {
            Rect::from_center_size(pos2(area.left() + edge, area.center().y), vec2(6.0, area.height()))
        };
        let id = Id::new(("dock_split", side, i));
        let resp = ui.interact(grab, id, Sense::drag());
        let (an, bn) = (a.shown().map_or("", |p| crate::i18n::t(p.label())), b.shown().map_or("", |p| crate::i18n::t(p.label())));
        ui.ctx().accesskit_node_builder(id, |node| {
            node.set_role(egui::accesskit::Role::Splitter);
            node.set_label(tr!("Resize {first} and {second}", first = an, second = bn));
        });
        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(if side.stacks() { egui::CursorIcon::ResizeVertical } else { egui::CursorIcon::ResizeHorizontal });
            let line = if side.stacks() { Rect::from_center_size(grab.center(), vec2(grab.width(), 2.0)) } else { Rect::from_center_size(grab.center(), vec2(2.0, grab.height())) };
            ui.painter().rect_filled(line, theme::SQUARE, T.accent);
        }
        let d = if side.stacks() { resp.drag_delta().y } else { resp.drag_delta().x };
        if d != 0.0 && free > 0.0 {
            let (Some(&ba), Some(&bb)) = (bodies.get(i), bodies.get(i + 1)) else { continue };
            let total = ba + bb;
            let na = (ba + d).clamp(MIN_BODY.min(total * 0.5), (total - MIN_BODY).max(total * 0.5));
            let mut shares: Vec<f32> = groups.iter().map(|g| g.share).collect();
            if let Some(sa) = shares.get_mut(i) {
                *sa = na / free * sum;
            }
            if let Some(sb) = shares.get_mut(i + 1) {
                *sb = (total - na) / free * sum;
            }
            acts.push(Act::Shares(side, shares));
        }
    }
}

/// A tab group: its header (tabs, fold, menu) and the shown panel.
#[allow(clippy::too_many_arguments)]
fn group_ui(app: &mut App, ui: &mut egui::Ui, side: Side, g: &Group, rect: Rect, next: Option<Panel>, first_group: bool, layout: &Layout, acts: &mut Vec<Act>) {
    let Some(shown) = g.shown() else { return };
    let header = if side.stacks() {
        let header = Rect::from_min_size(rect.min, vec2(rect.width(), BAND_H));
        band_header(ui, side, g, header, first_group, layout, acts);
        header
    } else {
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).id_salt(("dock_header", shown.id())));
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        row_header(app, &mut child, side, g, rect, layout, acts)
    };
    if let Some(first) = g.tabs.first() {
        app.dock_view.drawn.push(Drawn { over: Over::Group(side), first: *first, next, rect, header });
    }
    let body = Rect::from_min_max(pos2(rect.left(), header.bottom()), rect.max);
    if body.height() < 1.0 || body.width() < 1.0 {
        return;
    }
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)).id_salt(("dock_body", shown.id())));
    child.set_clip_rect(body.intersect(ui.clip_rect()));
    panel_body(app, &mut child, shown);
    // Groups side by side are parted by a line.
    if !side.stacks() && rect.left() > ui.max_rect().left() + 1.0 {
        ui.painter().vline(rect.left(), rect.y_range(), Stroke::new(1.0, T.border));
    }
}

/// A side dock's group header: a band (fold chevron, the tabs, collapsing the dock to icons
/// on the first, the panel menu).
fn band_header(ui: &mut egui::Ui, side: Side, g: &Group, rect: Rect, first_group: bool, layout: &Layout, acts: &mut Vec<Act>) {
    let Some(shown) = g.shown() else { return };
    let at = Where::Docked { side, folded: g.folded, tabs: g.tabs.len() };
    let p = ui.painter().clone();
    p.rect_filled(rect, theme::SQUARE, T.panel_alt);
    p.line_segment([rect.left_top(), rect.right_top()], Stroke::new(1.0, T.border));
    p.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, T.border));
    p.rect_filled(Rect::from_min_size(rect.left_top() + vec2(0.0, 1.0), vec2(3.0, rect.height() - 1.0)), theme::SQUARE, T.accent);
    // Fold: the group keeps only its header, giving the others its room.
    let chevron = Rect::from_center_size(pos2(rect.left() + 17.0, rect.center().y), vec2(22.0, 22.0));
    let resp = ui.interact(chevron, Id::new(("dock_fold", shown.id())), Sense::click());
    let open = !g.folded;
    let name = crate::i18n::t(shown.label());
    resp.widget_info(|| egui::WidgetInfo::selected(WidgetType::CollapsingHeader, true, open, name));
    a11y::focus_ring(ui, &resp);
    let hot = resp.hovered() || resp.has_focus();
    crate::icons::paint(&p, Rect::from_center_size(chevron.center(), vec2(14.0, 14.0)), if open { Icon::ChevronDown } else { Icon::ChevronRight }, if hot { T.text } else { T.text_dim });
    if resp.on_hover_text(if open { tr!("Fold this panel group") } else { tr!("Unfold this panel group") }).clicked() {
        acts.push(Act::Fold(shown));
    }
    let mut right = rect.right() - MENU_W - 4.0;
    // The dock's first group: collapse the whole dock to icons (Animate's double arrow).
    if first_group {
        let r = Rect::from_center_size(pos2(right - MENU_W * 0.5 + 2.0, rect.center().y), vec2(MENU_W - 4.0, 24.0));
        right -= MENU_W - 2.0;
        let icon = if side == Side::Left { Icon::First } else { Icon::Last };
        if icon_button(ui, r, Id::new(("dock_collapse", side)), icon, tr!("Collapse to icons"), None).clicked() {
            acts.push(Act::Collapse(side, true));
        }
    }
    // The tab strip ends before the buttons: a name never runs under them (ADR 0083). Tabs too
    // wide for it shrink, their names cut with an ellipsis (the whole name is in the tooltip);
    // at their narrowest the strip scrolls with the wheel, keeping the shown tab in view.
    let strip = Rect::from_min_max(pos2(rect.left() + 30.0, rect.top()), pos2(right, rect.bottom()));
    let active_i = g.active.min(g.tabs.len().saturating_sub(1));
    let names: Vec<&str> = g.tabs.iter().map(|t| crate::i18n::t(t.label())).collect();
    let natural: Vec<f32> = names.iter().map(|n| p.layout_no_wrap(n.to_string(), font(Type::Heading), T.text).rect.width() + TAB_PAD).collect();
    let mut widths = fit_tabs(&natural, strip.width());
    // Too narrow even for the shrunk tabs: the strip scrolls, and the shown tab keeps as much of
    // its name as the strip holds.
    if widths.iter().sum::<f32>() + TAB_GAP * widths.len().saturating_sub(1) as f32 > strip.width()
        && let (Some(w), Some(n)) = (widths.get_mut(active_i), natural.get(active_i))
    {
        *w = n.min(strip.width()).max(*w);
    }
    let total = widths.iter().sum::<f32>() + TAB_GAP * widths.len().saturating_sub(1) as f32;
    let overflow = (total - strip.width()).max(0.0);
    let key = Id::new(("dock_tab_scroll", shown.id(), g.tabs.first().map_or("", |t| t.id())));
    let (mut scroll, last_active) = ui.ctx().data(|d| d.get_temp::<(f32, usize)>(key)).unwrap_or((0.0, usize::MAX));
    if overflow > 0.0 && ui.rect_contains_pointer(strip) {
        let d = ui.input(|i| i.smooth_scroll_delta);
        scroll -= if d.x.abs() > d.y.abs() { d.x } else { d.y };
    }
    if last_active != active_i {
        // The shown tab comes into view.
        let at: f32 = widths.iter().take(active_i).map(|w| w + TAB_GAP).sum();
        let w = widths.get(active_i).copied().unwrap_or(0.0);
        scroll = scroll.min(at).max(at + w - strip.width());
    }
    scroll = scroll.clamp(0.0, overflow);
    ui.ctx().data_mut(|d| d.insert_temp(key, (scroll, active_i)));
    ui.ctx().data_mut(|d| {
        for t in &g.tabs {
            d.remove::<(Rect, bool)>(Id::new(("dock_tab_rect", t.id())));
        }
    });
    let mut x = strip.left() - scroll;
    let several = g.tabs.len() > 1;
    for (i, (tab, w)) in g.tabs.iter().zip(&widths).enumerate() {
        let r = Rect::from_min_size(pos2(x, rect.top() + 3.0), vec2(*w, rect.height() - 6.0));
        x += w + TAB_GAP;
        let shown_r = r.intersect(strip);
        if shown_r.width() < 4.0 {
            continue;
        }
        let name = names.get(i).copied().unwrap_or("");
        let cut = natural.get(i).is_some_and(|n| *n > w + 0.5);
        let galley = if cut { elided(&p, name, w - TAB_PAD) } else { p.layout_no_wrap(name.to_string(), font(Type::Heading), T.text) };
        let resp = tab_widget(ui, *tab, r, shown_r, i == active_i, &galley, several);
        ui.ctx().data_mut(|d| d.insert_temp(Id::new(("dock_tab_rect", tab.id())), (shown_r, cut)));
        tab_actions(ui, at, *tab, &resp, cut, layout, acts);
        if i == active_i {
            a11y::anchor(ui, region(*tab), Rect::from_min_max(rect.min, pos2(rect.right(), ui.max_rect().bottom().max(rect.bottom()))), &resp);
        }
    }
    // More tabs past an edge: the strip fades there.
    for (edge, more) in [(strip.left(), scroll > 0.5), (strip.right(), scroll < overflow - 0.5)] {
        if more {
            let w = 10.0;
            let fade = if edge == strip.left() { Rect::from_min_size(pos2(edge, rect.top() + 1.0), vec2(w, rect.height() - 2.0)) } else { Rect::from_min_size(pos2(edge - w, rect.top() + 1.0), vec2(w, rect.height() - 2.0)) };
            p.rect_filled(fade, theme::SQUARE, T.panel_alt.gamma_multiply(0.85));
            let c = fade.center();
            let (tip, back) = if edge == strip.left() { (c.x - 2.5, c.x + 2.0) } else { (c.x + 2.5, c.x - 2.0) };
            p.add(egui::Shape::convex_polygon(vec![pos2(tip, c.y), pos2(back, c.y - 4.0), pos2(back, c.y + 4.0)], T.text_dim, Stroke::NONE));
        }
    }
    let menu = Rect::from_center_size(pos2(rect.right() - MENU_W * 0.5 - 2.0, rect.center().y), vec2(MENU_W - 2.0, 24.0));
    menu_button(ui, at, shown, menu, layout, acts);
}

/// A small square button drawn with an icon, named for screen readers, with its tooltip.
pub(crate) fn icon_button(ui: &mut egui::Ui, r: Rect, id: Id, icon: Icon, name: &str, selected: Option<bool>) -> egui::Response {
    let resp = ui.interact(r, id, Sense::click());
    a11y::control(ui, &resp, if selected.is_some() { WidgetType::SelectableLabel } else { WidgetType::Button }, name, selected);
    let on = selected == Some(true);
    if on {
        ui.painter().rect_filled(r, T.rc(), T.active);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, T.rc(), T.hover);
    }
    a11y::focus_ring(ui, &resp);
    let tint = if on { T.on_active } else if resp.hovered() || resp.has_focus() { T.icon_hot } else { T.icon };
    let s = (r.height().min(r.width()) * 0.5).clamp(12.0, theme::TOOL_ICON);
    crate::icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(s, s)), icon, tint);
    resp.on_hover_text(name)
}

/// A top or bottom dock's group header: its tabs in a row (and the timeline's own tools beside
/// them, as in Animate), the panel menu at the end. Returns the header's rect.
fn row_header(app: &mut App, ui: &mut egui::Ui, side: Side, g: &Group, group: Rect, layout: &Layout, acts: &mut Vec<Act>) -> Rect {
    let shown = g.shown();
    let at = Where::Docked { side, folded: g.folded, tabs: g.tabs.len() };
    let frame = egui::Frame::NONE.fill(T.panel_alt).inner_margin(egui::Margin::symmetric(10, 5));
    let out = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let anchor = tab_row(app, ui, at, &g.tabs, g.active, layout, acts);
            if let (Some(p), Some(r)) = (shown, &anchor) {
                a11y::anchor(ui, region(p), group, r);
            }
            if shown == Some(Panel::Timeline) {
                ui.add_space(4.0);
                let w = ui.available_width() - MENU_W - 8.0;
                crate::timeline::header_tools(app, ui, w);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (r, _) = ui.allocate_exact_size(vec2(MENU_W - 2.0, 24.0), Sense::hover());
                if let Some(p) = shown {
                    menu_button(ui, at, p, r, layout, acts);
                }
            });
        });
    });
    out.response.rect
}

/// Tabs in a row (above and below the stage, and in floating windows): click to show, drag to
/// move, right-click for the panel's menu. Returns the shown tab's response.
pub(crate) fn tab_row(app: &mut App, ui: &mut egui::Ui, at: Where, tabs: &[Panel], active: usize, layout: &Layout, acts: &mut Vec<Act>) -> Option<egui::Response> {
    let mut anchor = None;
    for (i, tab) in tabs.iter().enumerate() {
        let on = i == active.min(tabs.len().saturating_sub(1));
        // Dragged, the tab moves its panel (a drag begins only once the pointer moves, so a
        // click still shows it).
        let r = ui.selectable_label(on, text(Type::Heading, crate::i18n::t(tab.label()))).interact(Sense::drag());
        if r.drag_started() && app.dock_view.drag.is_none() {
            app.dock_view.drag = Some(Drag::Panel(*tab));
            let vp = ui.ctx().viewport_id();
            app.dock_view.from_window = (matches!(at, Where::Floating { .. }) && vp != egui::ViewportId::ROOT).then_some(vp);
        }
        let r = r.on_hover_text(tab_tip(*tab));
        if r.clicked() {
            acts.push(Act::Show(*tab));
        }
        r.context_menu(|ui| panel_menu(ui, at, *tab, layout, acts));
        if on {
            anchor = Some(r);
        }
    }
    anchor
}

fn tab_tip(p: Panel) -> &'static str {
    match p {
        Panel::Timeline => tr!("Layers and frames"),
        Panel::MotionEditor => tr!("Property curves of the motion tween at the playhead (double-click a motion span)"),
        _ => tr!("Drag the tab to dock the panel elsewhere, stack it with others or float it"),
    }
}

/// `name` in one line no wider than `width`, cut with an ellipsis.
fn elided(p: &egui::Painter, name: &str, width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(name.to_string(), font(Type::Heading), T.text);
    job.wrap = egui::text::TextWrapping { max_width: width.max(1.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    p.layout_job(job)
}

/// A side dock's tab: named and selectable for screen readers, dragged to move its panel.
/// `shown` is the part of `r` inside the tab strip (it scrolls).
fn tab_widget(ui: &mut egui::Ui, tab: Panel, r: Rect, shown: Rect, active: bool, galley: &std::sync::Arc<egui::Galley>, several: bool) -> egui::Response {
    let resp = ui.interact(shown, Id::new(("dock_tab", tab.id())), Sense::click_and_drag());
    let name = crate::i18n::t(tab.label());
    resp.widget_info(|| egui::WidgetInfo::selected(WidgetType::SelectableLabel, true, active, name));
    let p = ui.painter().with_clip_rect(shown.intersect(ui.painter().clip_rect()));
    if resp.hovered() && !active {
        p.rect_filled(r, T.rc(), T.hover);
    }
    let colour = if active { theme::color(Type::Heading) } else { T.text_dim };
    // Drawn inside the strip only: never over the buttons.
    p.galley_with_override_text_color(pos2(r.left() + TAB_PAD * 0.5, r.center().y - galley.rect.height() * 0.5), galley.clone(), colour);
    if active && several {
        p.rect_filled(Rect::from_min_max(pos2(r.left() + 6.0, r.bottom() - 2.0), pos2(r.right() - 6.0, r.bottom())), theme::SQUARE, T.accent);
    }
    if resp.has_focus() {
        p.rect_stroke(r, T.rc(), Stroke::new(2.0, T.accent), StrokeKind::Inside);
    }
    resp
}

/// What a tab's clicks and drags do: show its panel, fold (double-click), its menu (right-click),
/// move it (drag). A `cut` name shows whole in the tooltip.
fn tab_actions(ui: &egui::Ui, at: Where, tab: Panel, resp: &egui::Response, cut: bool, layout: &Layout, acts: &mut Vec<Act>) {
    if resp.drag_started() {
        ui.ctx().data_mut(|d| d.insert_temp(Id::new("dock_drag_start"), Some(Drag::Panel(tab))));
    }
    if resp.double_clicked() && matches!(at, Where::Docked { side, .. } if side.stacks()) {
        acts.push(Act::Fold(tab));
    } else if resp.clicked() {
        acts.push(Act::Show(tab));
    }
    let tip = if cut { format!("{}\n{}", crate::i18n::t(tab.label()), tab_tip(tab)) } else { tab_tip(tab).to_string() };
    let resp = resp.clone().on_hover_text(tip);
    resp.context_menu(|ui| panel_menu(ui, at, tab, layout, acts));
}

/// The ≡ button at the end of a group's header: the shown panel's menu.
pub(crate) fn menu_button(ui: &mut egui::Ui, at: Where, p: Panel, r: Rect, layout: &Layout, acts: &mut Vec<Act>) {
    let resp = ui.interact(r, Id::new(("dock_menu", p.id(), wkey(at))), Sense::click());
    let name = tr!("{panel} panel menu", panel = crate::i18n::t(p.label()));
    a11y::control(ui, &resp, WidgetType::Button, &name, None);
    let hot = resp.hovered() || resp.has_focus();
    if resp.hovered() {
        ui.painter().rect_filled(r, T.rc(), T.hover);
    }
    a11y::focus_ring(ui, &resp);
    // Three short lines: the panel menu, as in other studios.
    let c = r.center();
    for dy in [-4.0, 0.0, 4.0] {
        ui.painter().hline((c.x - 6.0)..=(c.x + 6.0), c.y + dy, Stroke::new(1.5, if hot { T.icon_hot } else { T.icon }));
    }
    let resp = resp.on_hover_text(tr!("Dock, float or close this panel"));
    egui::Popup::menu(&resp).show(|ui| panel_menu(ui, at, p, layout, acts));
}

/// A panel's menu: dock it on any side, float it, stack it with another group or window, fold
/// its group, collapse its dock to icons, close it (or its group or window). Everything a drag
/// does, from the keyboard.
pub(crate) fn panel_menu(ui: &mut egui::Ui, at: Where, p: Panel, layout: &Layout, acts: &mut Vec<Act>) {
    let item = |ui: &mut egui::Ui, label: &str| {
        let c = ui.button(label).clicked();
        if c {
            ui.close();
        }
        c
    };
    for s in Side::ALL {
        let here = match at {
            Where::Docked { side, tabs, .. } => side == s && tabs == 1,
            Where::Bar(side) => side == s,
            Where::Floating { .. } => false,
        };
        if !here && item(ui, crate::i18n::t(s.label())) {
            acts.push(Act::Move(p, if p == Panel::Tools { Target::Bar(s) } else { Target::Split { side: s, before: None } }));
        }
    }
    let alone = matches!(at, Where::Floating { tabs: 1, .. });
    if !alone && item(ui, tr!("Float in a Window")) {
        acts.push(Act::Move(p, Target::Float(None)));
    }
    // Stack with another group or window: the keyboard's way to make tabs.
    let mut others: Vec<(Panel, String)> = Vec::new();
    for side in Side::ALL {
        for g in &layout.zone(side).groups {
            if let (Some(first), Some(shown)) = (g.tabs.first(), g.shown())
                && !g.tabs.contains(&p)
            {
                others.push((*first, crate::i18n::t(shown.label()).to_string()));
            }
        }
    }
    for f in &layout.floating {
        if let (Some(first), Some(shown)) = (f.tabs.first(), f.shown())
            && !f.tabs.contains(&p)
        {
            others.push((*first, tr!("{panel} (floating)", panel = crate::i18n::t(shown.label()))));
        }
    }
    if !others.is_empty() {
        ui.menu_button(tr!("Stack With"), |ui| {
            for (with, name) in &others {
                if item(ui, name) {
                    acts.push(Act::Move(p, Target::Stack(*with)));
                }
            }
        });
    }
    match at {
        Where::Docked { side, folded, .. } => {
            if side.stacks() && item(ui, if folded { tr!("Unfold Group") } else { tr!("Fold Group") }) {
                acts.push(Act::Fold(p));
            }
            let collapsed = layout.zone(side).collapsed;
            if item(ui, if collapsed { tr!("Expand the Dock") } else { tr!("Collapse the Dock to Icons") }) {
                acts.push(Act::Collapse(side, !collapsed));
            }
        }
        Where::Floating { window, tabs } => {
            if item(ui, tr!("Dock")) {
                acts.push(if tabs > 1 { Act::DockBackWindow(window) } else { Act::DockBack(p) });
            }
        }
        Where::Bar(_) => {}
    }
    ui.separator();
    if item(ui, &tr!("Close {panel}", panel = crate::i18n::t(p.label()))) {
        acts.push(Act::Close(p));
    }
    match at {
        Where::Docked { tabs, .. } if tabs > 1 && item(ui, tr!("Close Tab Group")) => acts.push(Act::CloseGroup(p)),
        Where::Floating { window, tabs } if tabs > 1 && item(ui, tr!("Close Window")) => acts.push(Act::CloseWindow(window)),
        _ => {}
    }
}

/// The shown panel's contents, wherever it is.
pub fn panel_body(app: &mut App, ui: &mut egui::Ui, p: Panel) {
    // A vertical scroll area grows to its widest row and remembers it; held to the panel's
    // width, a row that can't fit can't widen every other one (ADR 0087).
    let width = ui.available_width();
    let pad = |x: i8, y: i8| egui::Frame::NONE.inner_margin(egui::Margin::symmetric(x, y));
    let scroll = |salt: &'static str| egui::ScrollArea::vertical().id_salt(salt).auto_shrink([false, false]);
    let held = |ui: &mut egui::Ui| ui.set_max_width(width);
    match p {
        Panel::Properties => {
            scroll("props").show(ui, |ui| {
                held(ui);
                pad(12, 8).show(ui, |ui| crate::panels::properties(app, ui))
            });
        }
        Panel::Align => {
            scroll("align").show(ui, |ui| {
                held(ui);
                pad(12, 8).show(ui, |ui| crate::panels::align_panel(app, ui))
            });
        }
        Panel::Colour => {
            scroll("colour").show(ui, |ui| {
                held(ui);
                pad(12, 8).show(ui, |ui| crate::panels::colour_panel(app, ui))
            });
        }
        Panel::Swatches => {
            scroll("swatches").show(ui, |ui| {
                held(ui);
                pad(12, 8).show(ui, |ui| crate::swatches_panel::panel(app, ui))
            });
        }
        Panel::Library => {
            scroll("library_dock").show(ui, |ui| {
                held(ui);
                pad(12, 10).show(ui, |ui| crate::panels::library(app, ui))
            });
        }
        Panel::MovieExplorer => {
            pad(12, 4).show(ui, |ui| crate::movie_explorer::panel(app, ui));
        }
        Panel::History => {
            pad(12, 8).show(ui, |ui| crate::panels::history(app, ui));
        }
        Panel::LayerDepth => {
            scroll("layer_depth").show(ui, |ui| {
                held(ui);
                pad(12, 8).show(ui, |ui| crate::layer_depth::panel(app, ui))
            });
        }
        Panel::Timeline => crate::timeline::show(app, ui),
        Panel::MotionEditor => crate::motion_editor::show(app, ui),
        Panel::Actions => {
            scroll("actions").show(ui, |ui| {
                held(ui);
                pad(10, 8).show(ui, |ui| crate::actions_panel::panel(app, ui))
            });
        }
        Panel::Output => {
            pad(10, 8).show(ui, |ui| crate::test_movie::output_panel(app, ui));
        }
        Panel::Tools => {
            pad(6, 6).show(ui, |ui| crate::panels::tools(app, ui, crate::panels::ToolFlow::Grid));
        }
    }
}

// ----- docks collapsed to icons -------------------------------------------------------------------

/// A dock collapsed to icons: a strip with the expand button, then one icon per panel (named
/// for screen readers, its name as its tooltip); a click opens the panel over the stage.
fn strip(app: &mut App, ui: &mut egui::Ui, side: Side, zone: &Zone, layout: &Layout, acts: &mut Vec<Act>) {
    let id = Id::new(("dock_strip", side));
    let panel = match side {
        Side::Left => egui::Panel::left(id),
        Side::Right => egui::Panel::right(id),
        Side::Top => egui::Panel::top(id),
        Side::Bottom => egui::Panel::bottom(id),
    };
    let flyout = app.dock_view.flyout;
    let shown = panel.resizable(false).exact_size(STRIP).frame(egui::Frame::NONE.fill(T.panel_alt)).show(ui, |ui| {
        let area = ui.max_rect();
        ui.allocate_rect(area, Sense::hover());
        let down = side.stacks();
        let b = theme::TOOL_BUTTON;
        let mut at = area.min + vec2((area.width() - b).max(0.0) * 0.5, (area.height() - b).max(0.0) * 0.5);
        if down {
            at.y = area.top() + 4.0;
        } else {
            at.x = area.left() + 4.0;
        }
        let step = |at: &mut Pos2, by: f32| {
            if down {
                at.y += by;
            } else {
                at.x += by;
            }
        };
        let expand = match side {
            Side::Left => Icon::Last,
            Side::Right => Icon::First,
            Side::Top => Icon::ChevronDown,
            Side::Bottom => Icon::ChevronUp,
        };
        let r = Rect::from_min_size(at, vec2(b, b * 0.75));
        if icon_button(ui, r, Id::new(("dock_expand", side)), expand, tr!("Expand the dock"), None).clicked() {
            acts.push(Act::Collapse(side, false));
        }
        step(&mut at, b * 0.75 + 4.0);
        let first = zone.panels().next();
        for (gi, g) in zone.groups.iter().enumerate() {
            if gi > 0 {
                // Groups are parted by a short rule.
                let c = if down { pos2(area.center().x, at.y + 2.0) } else { pos2(at.x + 2.0, area.center().y) };
                if down {
                    ui.painter().hline((c.x - b * 0.35)..=(c.x + b * 0.35), c.y, Stroke::new(1.0, T.border));
                } else {
                    ui.painter().vline(c.x, (c.y - b * 0.35)..=(c.y + b * 0.35), Stroke::new(1.0, T.border));
                }
                step(&mut at, 5.0);
            }
            for p in &g.tabs {
                let r = Rect::from_min_size(at, vec2(b, b));
                step(&mut at, b + 2.0);
                let open = flyout == Some(*p);
                let name = crate::i18n::t(p.label());
                // Dragged, the icon moves its panel, as a tab does.
                let resp = icon_button(ui, r, Id::new(("dock_icon", p.id())), panel_icon(*p), name, Some(open)).interact(Sense::drag());
                if resp.drag_started() && app.dock_view.drag.is_none() {
                    app.dock_view.drag = Some(Drag::Panel(*p));
                }
                if resp.clicked() {
                    acts.push(Act::Flyout(if open { None } else { Some(*p) }));
                }
                let at_menu = Where::Docked { side, folded: g.folded, tabs: g.tabs.len() };
                resp.context_menu(|ui| panel_menu(ui, at_menu, *p, layout, acts));
                // Ctrl+F6 lands on the strip's first icon (an open flyout is its panel's stop).
                if Some(*p) == first && flyout != Some(*p) {
                    a11y::anchor(ui, region(*p), area, &resp);
                }
            }
        }
        area
    });
    let rect = shown.inner;
    if let Some(first) = zone.panels().next() {
        app.dock_view.drawn.push(Drawn { over: Over::Strip(side), first, next: None, rect, header: rect });
    }
}

/// The panel open over the stage from a collapsed dock, beside its strip, as wide (or high)
/// as the dock was. Esc or a click outside it closes it.
fn flyout(app: &mut App, ctx: &egui::Context, auto: (f32, f32), layout: &Layout, acts: &mut Vec<Act>) {
    let Some(p) = app.dock_view.flyout else { return };
    let (Some(side), Some(strip)) = (layout.collapsed_side(p), app.dock_view.drawn.iter().find(|d| matches!(d.over, Over::Strip(s) if Some(s) == layout.collapsed_side(p))).map(|d| d.rect)) else {
        app.dock_view.flyout = None;
        return;
    };
    let Some(stage) = app.dock_view.stage else { return };
    let zone = layout.zone(side);
    let rect = if side.stacks() {
        let w = zone.size.unwrap_or(auto.0).clamp(MIN_SIDE, (stage.width() - 20.0).max(MIN_SIDE));
        let x = if side == Side::Left { strip.right() } else { strip.left() - w };
        Rect::from_min_size(pos2(x, strip.top()), vec2(w, strip.height()))
    } else {
        let h = zone.size.unwrap_or(auto.1).clamp(MIN_ROW, (stage.height() - 20.0).max(MIN_ROW));
        let y = if side == Side::Top { strip.bottom() } else { strip.top() - h };
        Rect::from_min_size(pos2(strip.left(), y), vec2(strip.width(), h))
    };
    let Some(Place::Docked { group, .. }) = layout.place(p) else { return };
    let tabs = layout.zone(side).groups.get(group).map_or(1, |g| g.tabs.len());
    let at = Where::Docked { side, folded: false, tabs };
    egui::Area::new(Id::new("dock_flyout")).order(egui::Order::Foreground).fixed_pos(rect.min).constrain(false).show(ctx, |ui| {
        ui.set_clip_rect(rect);
        ui.painter().rect_filled(rect, theme::SQUARE, T.panel);
        ui.painter().rect_stroke(rect, theme::SQUARE, Stroke::new(1.0, T.border), StrokeKind::Inside);
        let header = Rect::from_min_size(rect.min, vec2(rect.width(), BAND_H));
        ui.painter().rect_filled(header, theme::SQUARE, T.panel_alt);
        ui.painter().rect_filled(Rect::from_min_size(header.left_top(), vec2(3.0, header.height())), theme::SQUARE, T.accent);
        let title = crate::i18n::t(p.label());
        let galley = ui.painter().layout_no_wrap(title.to_string(), font(Type::Heading), theme::color(Type::Heading));
        let tr = Rect::from_min_max(header.min + vec2(12.0, 0.0), pos2(header.right() - 2.0 * MENU_W - 6.0, header.bottom()));
        ui.painter().with_clip_rect(tr).galley(pos2(tr.left(), header.center().y - galley.rect.height() * 0.5), galley, theme::color(Type::Heading));
        let close = Rect::from_center_size(pos2(header.right() - MENU_W * 0.5 - 2.0, header.center().y), vec2(MENU_W - 2.0, 24.0));
        let resp = icon_button(ui, close, Id::new("dock_flyout_close"), Icon::ChevronRight, &tr!("Close {panel}", panel = title), None);
        // Its first control: Ctrl+F6's stop while it is open.
        a11y::anchor(ui, region(p), rect, &resp);
        if resp.clicked() {
            acts.push(Act::Flyout(None));
        }
        let menu = close.translate(vec2(-MENU_W, 0.0));
        menu_button(ui, at, p, menu, layout, acts);
        let body = Rect::from_min_max(pos2(rect.left() + 1.0, header.bottom()), rect.max - vec2(1.0, 1.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)).id_salt(("dock_body", p.id())));
        child.set_clip_rect(body);
        panel_body(app, &mut child, p);
        ui.allocate_rect(rect, Sense::hover());
    });
    // Esc closes it (after a drag's own Esc), focus going back to its icon.
    let popup = egui::Popup::is_any_open(ctx);
    if app.dock_view.drag.is_none() && !popup && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        acts.push(Act::Flyout(None));
        let icon = Id::new(("dock_icon", p.id()));
        ctx.memory_mut(|m| m.request_focus(icon));
        return;
    }
    // A press outside it and its strip (not in a menu or list it opened) closes it.
    let press = ctx.input(|i| if i.pointer.any_pressed() { i.pointer.press_origin() } else { None });
    if let Some(at) = press
        && !rect.contains(at)
        && !strip.contains(at)
        && !popup
        && ctx.layer_id_at(at).is_none_or(|l| !matches!(l.order, egui::Order::Foreground | egui::Order::Tooltip))
    {
        acts.push(Act::Flyout(None));
    }
}

// ----- dragging a tab -----------------------------------------------------------------------------

/// Where a panel (or a floating window) dropped at `at` (editor points) goes, and the area that
/// shows it (none for a window of its own: it can't be drawn over).
pub(crate) fn drop_target(view: &DockView, layout: &Layout, what: Drag, at: Pos2) -> (Option<Target>, Option<Rect>) {
    let tools = what == Drag::Panel(Panel::Tools);
    // Floating windows inside the editor first: they are drawn over the docks.
    for d in view.drawn.iter().filter(|d| matches!(d.over, Over::Window(_))).chain(view.drawn.iter().filter(|d| !matches!(d.over, Over::Window(_)))) {
        if !d.rect.contains(at) {
            continue;
        }
        match d.over {
            Over::Window(id) if what == Drag::Window(id) => continue,
            Over::Window(_) | Over::Strip(_) => return (Some(Target::Stack(d.first)), Some(d.rect)),
            Over::Group(side) => {
                if d.header.contains(at) {
                    return (Some(Target::Stack(d.first)), Some(d.rect));
                }
                // The body's first half: a group of its own before this one; the second, after.
                let body = Rect::from_min_max(pos2(d.rect.left(), d.header.bottom()), d.rect.max);
                let first_half = if side.stacks() { at.y < body.center().y } else { at.x < body.center().x };
                let half = match (side.stacks(), first_half) {
                    (true, true) => Rect::from_min_max(body.min, pos2(body.right(), body.center().y)),
                    (true, false) => Rect::from_min_max(pos2(body.left(), body.center().y), body.max),
                    (false, true) => Rect::from_min_max(d.rect.min, pos2(d.rect.center().x, d.rect.bottom())),
                    (false, false) => Rect::from_min_max(pos2(d.rect.center().x, d.rect.top()), d.rect.max),
                };
                let before = if first_half { Some(d.first) } else { d.next };
                return (Some(Target::Split { side, before }), Some(half));
            }
        }
    }
    // Over a window of its own: a tab there.
    let desk = view.origin() + at.to_vec2();
    for (id, r) in &view.desk {
        if what != Drag::Window(*id)
            && r.contains(desk)
            && let Some(first) = layout.window(*id).and_then(|i| layout.floating.get(i)).and_then(|f| f.tabs.first())
        {
            return (Some(Target::Stack(*first)), None);
        }
    }
    // Near an edge of the stage: a dock on that side (the tools: their bar there).
    if let Some(s) = view.stage
        && s.contains(at)
        && s.width() > 1.0
        && s.height() > 1.0
    {
        let (fx, fy) = ((at.x - s.left()) / s.width(), (at.y - s.top()) / s.height());
        let side = [(fx, Side::Left), (1.0 - fx, Side::Right), (fy, Side::Top), (1.0 - fy, Side::Bottom)].into_iter().filter(|(f, _)| *f < EDGE).min_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, s)| s);
        if let Some(side) = side {
            let band = match side {
                Side::Left => Rect::from_min_max(s.min, pos2(s.left() + s.width() * EDGE, s.bottom())),
                Side::Right => Rect::from_min_max(pos2(s.right() - s.width() * EDGE, s.top()), s.max),
                Side::Top => Rect::from_min_max(s.min, pos2(s.right(), s.top() + s.height() * EDGE)),
                Side::Bottom => Rect::from_min_max(pos2(s.left(), s.bottom() - s.height() * EDGE), s.max),
            };
            let target = if tools { Target::Bar(side) } else { Target::Split { side, before: None } };
            return (Some(target), Some(band));
        }
    }
    match what {
        // Anywhere else: a window of its own there.
        Drag::Panel(p) => {
            let size = p.float_size();
            (Some(Target::Float(Some([at.x, at.y]))), Some(Rect::from_min_size(at, vec2(size[0], size[1]) * 0.5)))
        }
        // A window dragged elsewhere stays where it was moved to.
        Drag::Window(_) => (None, None),
    }
}

/// While a tab or a floating window is dragged: where it would go, shown over the editor;
/// dropped, it goes there. Esc cancels.
fn drag_and_drop(app: &mut App, ctx: &egui::Context, acts: &mut Vec<Act>) {
    if let Some(d) = ctx.data_mut(|d| d.remove_temp::<Option<Drag>>(Id::new("dock_drag_start"))).flatten() {
        app.dock_view.drag = Some(d);
        app.dock_view.from_window = None;
    }
    let Some(what) = app.dock_view.drag else { return };
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        end_drag(&mut app.dock_view);
        return;
    }
    let (pos, down) = if app.dock_view.from_window.is_some() {
        // The window it began in sees the pointer (it holds the mouse while the button is down).
        match app.dock_view.remote {
            Some((pos, down)) => (Some(pos), down),
            None => return,
        }
    } else {
        ctx.input(|i| (i.pointer.latest_pos(), i.pointer.primary_down()))
    };
    let Some(pos) = pos else { return };
    let (target, area) = drop_target(&app.dock_view, &app.dock, what, pos);
    if !down {
        end_drag(&mut app.dock_view);
        let Some(target) = target else { return };
        let target = match target {
            // A window of its own goes where the pointer is on the desktop.
            Target::Float(Some([x, y])) => {
                let own = own_windows(app, ctx);
                let o = app.dock_view.origin();
                Target::Float(Some(if own { [x + o.x, y + o.y] } else { [x, y] }))
            }
            t => t,
        };
        acts.push(match what {
            Drag::Panel(p) => Act::Move(p, target),
            Drag::Window(id) => Act::MoveWindow(id, target),
        });
        return;
    }
    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, Id::new("dock_drop")));
    if let Some(r) = area {
        painter.rect_filled(r, theme::SQUARE, theme::dock_drop());
        painter.rect_stroke(r, theme::SQUARE, Stroke::new(2.0, T.accent), StrokeKind::Inside);
    }
    let label = match what {
        Drag::Panel(p) => crate::i18n::t(p.label()).to_string(),
        Drag::Window(id) => {
            let tabs = app.dock.window(id).and_then(|i| app.dock.floating.get(i)).map(|f| f.tabs.clone()).unwrap_or_default();
            tabs.iter().map(|p| crate::i18n::t(p.label())).collect::<Vec<_>>().join(" · ")
        }
    };
    let g = painter.layout_no_wrap(label, font(Type::Label), T.on_fill);
    let tag = Rect::from_min_size(pos + vec2(14.0, 10.0), vec2(g.rect.width() + 16.0, g.rect.height() + 8.0));
    painter.rect_filled(tag, T.rc(), T.accent);
    painter.galley(tag.min + vec2(8.0, 4.0), g, T.on_fill);
    ctx.request_repaint();
}

pub(crate) fn end_drag(view: &mut DockView) {
    view.drag = None;
    view.remote = None;
    view.from_window = None;
}

pub(crate) fn apply(app: &mut App, acts: Vec<Act>) {
    if acts.is_empty() {
        return;
    }
    let mut changed = false;
    for act in acts {
        match act {
            Act::Show(p) => {
                if !app.dock.is_shown(p) {
                    app.dock.show(p);
                    changed = true;
                }
            }
            Act::Fold(p) => {
                if let Some(Place::Docked { side, group, .. }) = app.dock.place(p)
                    && let Some(g) = app.dock.zone_mut(side).groups.get_mut(group)
                {
                    g.folded = !g.folded;
                    changed = true;
                }
            }
            Act::Shares(side, shares) => {
                let zone = app.dock.zone_mut(side);
                if zone.groups.len() == shares.len() {
                    for (g, s) in zone.groups.iter_mut().zip(shares) {
                        g.share = s;
                    }
                    app.dock.tidy();
                    changed = true;
                }
            }
            Act::Move(p, target) => {
                app.dock.move_to(p, target);
                if let (Target::Float(_), Some(Place::Floating { window, .. })) = (target, app.dock.place(p))
                    && let Some(f) = app.dock.floating.get(window)
                {
                    app.dock_view.placed.remove(&f.id);
                }
                changed = true;
            }
            Act::MoveWindow(id, target) => {
                app.dock.move_window(id, target);
                app.dock_view.placed.remove(&id);
                changed = true;
            }
            Act::Close(p) => {
                app.dock.close(p);
                changed = true;
            }
            Act::CloseGroup(p) => {
                if let Some(Place::Docked { side, group, .. }) = app.dock.place(p) {
                    let tabs = app.dock.zone(side).groups.get(group).map(|g| g.tabs.clone()).unwrap_or_default();
                    for t in tabs {
                        app.dock.close(t);
                    }
                    changed = true;
                }
            }
            Act::CloseWindow(id) => {
                app.dock.close_window(id);
                changed = true;
            }
            Act::DockBack(p) => {
                app.dock.dock_back(p);
                changed = true;
            }
            Act::DockBackWindow(id) => {
                app.dock.dock_back_window(id);
                changed = true;
            }
            Act::Placed(id, at, size) => {
                if let Some(f) = app.dock.window(id).and_then(|i| app.dock.floating.get_mut(i)) {
                    f.at = Some(at);
                    f.size = size;
                    changed = true;
                }
            }
            Act::Collapse(side, on) => {
                app.dock.zone_mut(side).collapsed = on;
                app.dock_view.flyout = None;
                changed = true;
            }
            Act::Flyout(p) => {
                app.dock_view.flyout = p;
                if let Some(p) = p
                    && !app.dock.is_shown(p)
                {
                    app.dock.show(p);
                    changed = true;
                }
            }
        }
    }
    // A flyout whose panel left its collapsed dock closes.
    if let Some(p) = app.dock_view.flyout
        && app.dock.collapsed_side(p).is_none()
    {
        app.dock_view.flyout = None;
    }
    if changed {
        app.dock_changed();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dock::{Place, built_in};

    pub(crate) fn app() -> (App, egui::Context) {
        let ctx = egui::Context::default();
        let session = anicraft_engine::Session::new(anicraft_engine::doc::sample::demo());
        // Screenshot mode: default preferences, nothing saved.
        let app = App::with_context(&ctx, session, Some(std::path::PathBuf::from("unused.png")));
        (app, ctx)
    }

    fn input(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1600.0, 940.0))), events, ..Default::default() }
    }

    pub(crate) fn frame(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) {
        ctx.run_ui(input(events), |ui| app.frame(ui)).textures_delta.clear();
    }

    pub(crate) fn settle(ctx: &egui::Context, app: &mut App) {
        for _ in 0..3 {
            frame(ctx, app, Vec::new());
        }
    }

    fn pointer(pos: egui::Pos2, pressed: Option<bool>) -> Vec<egui::Event> {
        let mut v = vec![egui::Event::PointerMoved(pos)];
        if let Some(pressed) = pressed {
            v.push(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        }
        v
    }

    fn key(key: egui::Key) -> Vec<egui::Event> {
        vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }]
    }

    fn click(ctx: &egui::Context, app: &mut App, at: egui::Pos2) {
        frame(ctx, app, pointer(at, None));
        frame(ctx, app, pointer(at, Some(true)));
        frame(ctx, app, pointer(at, Some(false)));
        settle(ctx, app);
    }

    /// Drags from `from` to `to` in a few steps, as a hand would.
    fn drag(ctx: &egui::Context, app: &mut App, from: egui::Pos2, to: egui::Pos2) {
        frame(ctx, app, pointer(from, None));
        frame(ctx, app, pointer(from, Some(true)));
        for i in 1..=8 {
            frame(ctx, app, pointer(from + (to - from) * (i as f32 / 8.0), None));
        }
        frame(ctx, app, pointer(to, Some(false)));
        settle(ctx, app);
    }

    fn drawn(app: &App, over: Over) -> Option<Drawn> {
        app.dock_view.drawn.iter().find(|d| d.over == over).copied()
    }

    #[test]
    fn tabs_shrink_to_fit_and_short_names_stay_whole() {
        // They fit: as they are.
        assert_eq!(fit_tabs(&[60.0, 80.0], 200.0), vec![60.0, 80.0]);
        // They don't: the long ones share what's left, the short one keeps its width.
        let w = fit_tabs(&[60.0, 120.0, 140.0], 250.0);
        assert_eq!(w[0], 60.0);
        assert!((w[1] - w[2]).abs() < 1e-3 && (w.iter().sum::<f32>() + 2.0 * TAB_GAP - 250.0).abs() < 1e-3, "{w:?}");
        // Narrower still: never below the narrowest a tab goes (the strip scrolls then).
        assert_eq!(fit_tabs(&[60.0, 120.0, 140.0], 20.0), vec![MIN_TAB; 3]);
        assert_eq!(fit_tabs(&[30.0, 120.0], 20.0), vec![30.0, MIN_TAB]);
        assert!(fit_tabs(&[], 10.0).is_empty());
    }

    #[test]
    fn at_the_narrowest_dock_no_tab_runs_under_the_menu_button() {
        let tabs = [Panel::Library, Panel::MovieExplorer, Panel::LayerDepth];
        let rect = |ctx: &egui::Context, t: Panel| ctx.data(|m| m.get_temp::<(Rect, bool)>(Id::new(("dock_tab_rect", t.id()))));
        for size in [MIN_SIDE, 260.0, 292.0] {
            // Three long names in one group, the dock at `size` (egui keeps a panel's size, so
            // a fresh window for each).
            let (mut app, ctx) = app();
            let Some(Place::Docked { side: Side::Right, group, .. }) = app.dock.place(Panel::Library) else { panic!("library docked right") };
            app.dock.right.groups[group].tabs = tabs.to_vec();
            app.dock.right.groups[group].active = 2;
            app.dock.right.size = Some(size);
            settle(&ctx, &mut app);
            let Some(d) = app.dock_view.drawn.iter().find(|d| d.first == Panel::Library) else { panic!("drawn") };
            assert!((d.header.width() - size).abs() <= 2.0, "the dock is {} wide, not {size}", d.header.width());
            let menu_left = d.header.right() - MENU_W - 4.0;
            for t in tabs {
                if let Some((r, _)) = rect(&ctx, t) {
                    assert!(r.right() <= menu_left + 0.5 && r.left() >= d.header.left() + 29.5, "{size}: {t:?} {r:?} leaves the strip (menu at {menu_left})");
                }
            }
            // The shown tab is always in view, and its name is whole or ends in an ellipsis.
            assert!(rect(&ctx, Panel::LayerDepth).is_some(), "{size}");
            if size == MIN_SIDE {
                let cut = tabs.iter().any(|t| rect(&ctx, *t).is_some_and(|(_, cut)| cut));
                assert!(cut, "{size}: the long names shrink with an ellipsis");
            }
        }
    }

    #[test]
    fn the_default_layout_is_essentials() {
        let (mut app, ctx) = app();
        settle(&ctx, &mut app);
        assert_eq!(app.dock, built_in("essentials").expect("built in"));
        assert_eq!(app.prefs.workspaces.current, "essentials");
        assert_eq!(app.dock.tools_bar, Some(Side::Left), "the tools on the left, as before");
        // Its groups: four on the right, one below the stage.
        let sides: Vec<Over> = app.dock_view.drawn.iter().map(|d| d.over).collect();
        assert_eq!(sides, vec![Over::Group(Side::Right), Over::Group(Side::Right), Over::Group(Side::Right), Over::Group(Side::Right), Over::Group(Side::Bottom)]);
    }

    #[test]
    fn dragging_the_timeline_tab_docks_it_above_the_stage() {
        let (mut app, ctx) = app();
        settle(&ctx, &mut app);
        let tab = drawn(&app, Over::Group(Side::Bottom)).map(|d| d.header.left_center() + vec2(40.0, 0.0));
        let stage = app.dock_view.stage;
        let (Some(tab), Some(stage)) = (tab, stage) else { panic!("drawn") };
        drag(&ctx, &mut app, tab, pos2(stage.center().x, stage.top() + 20.0));
        assert!(matches!(app.dock.place(Panel::Timeline), Some(Place::Docked { side: Side::Top, .. })), "{:?}", app.dock.place(Panel::Timeline));
        assert!(app.dock_view.drag.is_none());
        assert_eq!(app.prefs.workspaces.live.get("essentials"), Some(&app.dock), "kept as the arrangement of Essentials");
        // Dropped on the right dock's Library header: stacked there as a tab.
        let library = app.dock_view.drawn.iter().find(|d| d.first == Panel::Library).map(|d| d.header.center());
        let tab = drawn(&app, Over::Group(Side::Top)).map(|d| d.header.left_center() + vec2(40.0, 0.0));
        let (Some(library), Some(tab)) = (library, tab) else { panic!("drawn") };
        drag(&ctx, &mut app, tab, library);
        let Some(Place::Docked { side: Side::Right, group, .. }) = app.dock.place(Panel::Timeline) else { panic!("{:?}", app.dock.place(Panel::Timeline)) };
        assert_eq!(app.dock.right.groups[group].tabs, vec![Panel::Library, Panel::MovieExplorer, Panel::Timeline]);
        assert!(app.dock.top.groups.is_empty(), "the top dock is gone with its last group");
    }

    #[test]
    fn a_tab_dropped_away_from_the_docks_floats_and_esc_cancels_a_drag() {
        let (mut app, ctx) = app();
        settle(&ctx, &mut app);
        let history = app.dock_view.drawn.iter().find(|d| d.first == Panel::History).map(|d| d.header.left_center() + vec2(50.0, 0.0));
        let stage = app.dock_view.stage;
        let (Some(history), Some(stage)) = (history, stage) else { panic!("drawn") };
        // Esc while dragging: nothing moves.
        frame(&ctx, &mut app, pointer(history, None));
        frame(&ctx, &mut app, pointer(history, Some(true)));
        for i in 1..=6 {
            frame(&ctx, &mut app, pointer(history + (stage.center() - history) * (i as f32 / 6.0), None));
        }
        assert_eq!(app.dock_view.drag, Some(Drag::Panel(Panel::History)));
        frame(&ctx, &mut app, key(egui::Key::Escape));
        frame(&ctx, &mut app, pointer(stage.center(), Some(false)));
        assert_eq!(app.dock, built_in("essentials").expect("built in"));
        // The middle of the stage: a floating window there.
        drag(&ctx, &mut app, history, stage.center());
        assert!(app.dock.is_floating(Panel::History));
        assert_eq!(app.dock.floating.first().and_then(|f| f.at).map(|a| [a[0].round(), a[1].round()]), Some([stage.center().x.round(), stage.center().y.round()]));
    }

    #[test]
    fn floating_panels_fall_back_inside_the_editor_without_windows() {
        // Headless egui has no window system: it embeds viewports, as a system that can't open
        // another window does. The panel floats inside, as an egui window with Dock.
        let (mut app, ctx) = app();
        assert!(ctx.embed_viewports());
        app.dock.move_to(Panel::Output, Target::Float(Some([300.0, 200.0])));
        settle(&ctx, &mut app);
        let id = app.dock.floating[0].id;
        assert!(app.dock_view.placed.contains(&id), "drawn inside the editor");
        let area = ctx.memory(|m| m.area_rect(Id::new(("dock_float", id))));
        assert!(area.is_some_and(|r| (r.left() - 300.0).abs() < 1.0 && (r.top() - 200.0).abs() < 1.0), "where it was put: {area:?}");
        // A window of its own where there can be one: drawn in its viewport, not inside.
        let (mut app, ctx) = self::app();
        ctx.set_embed_viewports(false);
        app.dock.move_to(Panel::Output, Target::Float(None));
        settle(&ctx, &mut app);
        assert!(!app.dock_view.placed.contains(&app.dock.floating[0].id));
        assert!(app.dock.is_floating(Panel::Output));
        // Ctrl+F6 leaves it out (its keyboard is its window's).
        assert!(!a11y::order(&ctx).contains(&Region::Panel(Panel::Output)));
        // Dock puts it back where it came from.
        apply(&mut app, vec![Act::DockBack(Panel::Output)]);
        assert!(matches!(app.dock.place(Panel::Output), Some(Place::Docked { side: Side::Bottom, .. })));
    }

    #[test]
    fn every_panel_opens_from_the_window_menu_and_draws() {
        let (mut app, ctx) = app();
        for p in Panel::ALL {
            app.toggle_panel(p);
            if !app.panel_visible(p) {
                app.toggle_panel(p);
            }
            settle(&ctx, &mut app);
            assert!(app.panel_visible(p), "{p:?}");
            let drawn = app.dock.is_floating(p) || app.dock.tools_bar.is_some() && p == Panel::Tools || app.dock_view.drawn.iter().any(|d| app.dock.zone(match d.over {
                Over::Group(s) | Over::Strip(s) => s,
                Over::Window(_) => return false,
            }).groups.iter().any(|g| g.tabs.first() == Some(&d.first) && g.shown() == Some(p)));
            assert!(drawn, "{p:?} is on screen");
        }
    }

    #[test]
    fn workspaces_switch_and_reset_with_their_sizes() {
        let (mut app, ctx) = app();
        settle(&ctx, &mut app);
        let width = |app: &App| drawn(app, Over::Group(Side::Right)).map_or(0.0, |d| d.rect.width());
        let essentials = width(&app);
        app.switch_workspace("designer");
        settle(&ctx, &mut app);
        assert_eq!(app.dock, built_in("designer").expect("built in"));
        assert!(width(&app) > essentials + 50.0, "the Properties of Designer is wide: {} vs {essentials}", width(&app));
        app.dock.close(Panel::History);
        app.dock_changed();
        app.switch_workspace("essentials");
        app.switch_workspace("designer");
        assert!(!app.dock.is_open(Panel::History), "changes kept until reset");
        app.reset_workspace();
        assert_eq!(app.dock, built_in("designer").expect("built in"));
        app.save_workspace("Mine");
        assert_eq!(app.prefs.workspaces.current, "Mine");
        assert!(app.prefs.workspaces.saved.contains_key("Mine"));
        // Renamed and deleted (the one in use: back to Essentials); built-in ones stay.
        assert!(app.rename_workspace("Mine", "Inking"));
        assert_eq!(app.prefs.workspaces.current, "Inking");
        assert!(!app.rename_workspace("designer", "Mine"));
        app.delete_workspace("designer");
        assert!(app.prefs.workspaces.exists("designer"));
        app.delete_workspace("Inking");
        assert!(!app.prefs.workspaces.exists("Inking"));
        assert_eq!(app.prefs.workspaces.current, "essentials");
        assert_eq!(app.dock, built_in("essentials").expect("built in"));
    }

    #[test]
    fn the_tools_move_like_any_panel() {
        let (mut app, ctx) = app();
        settle(&ctx, &mut app);
        // The grip at the top of the tools' bar, dragged to the stage's right edge: the bar
        // goes to the right.
        let grip = ctx.read_response(Id::new("dock_grip_tools")).map(|r| r.rect.center());
        let stage = app.dock_view.stage;
        let (Some(grip), Some(stage)) = (grip, stage) else { panic!("drawn") };
        drag(&ctx, &mut app, grip, pos2(stage.right() - 10.0, stage.center().y));
        assert_eq!(app.dock.tools_bar, Some(Side::Right));
        // Onto the Library's header: a tab there, drawn as a grid of tools.
        let library = app.dock_view.drawn.iter().find(|d| d.first == Panel::Library).map(|d| d.header.center());
        let grip = ctx.read_response(Id::new("dock_grip_tools")).map(|r| r.rect.center());
        let (Some(library), Some(grip)) = (library, grip) else { panic!("drawn") };
        drag(&ctx, &mut app, grip, library);
        assert_eq!(app.dock.tools_bar, None);
        assert!(matches!(app.dock.place(Panel::Tools), Some(Place::Docked { side: Side::Right, .. })));
        assert!(a11y::order(&ctx).contains(&Region::Tools), "Ctrl+F6 still reaches the tools");
        // Its menu (the keyboard's way): back to a bar above the stage, then floating, then
        // closed and reopened from the Window menu where it was.
        apply(&mut app, vec![Act::Move(Panel::Tools, Target::Bar(Side::Top))]);
        settle(&ctx, &mut app);
        assert_eq!(app.dock.tools_bar, Some(Side::Top));
        apply(&mut app, vec![Act::Move(Panel::Tools, Target::Float(None))]);
        settle(&ctx, &mut app);
        assert!(app.dock.is_floating(Panel::Tools));
        app.toggle_panel(Panel::Tools);
        assert!(!app.dock.is_open(Panel::Tools));
        app.toggle_panel(Panel::Tools);
        assert!(app.dock.is_floating(Panel::Tools), "reopens floating, where it was");
        apply(&mut app, vec![Act::DockBack(Panel::Tools)]);
        assert_eq!(app.dock.tools_bar, Some(Side::Top), "Dock: back to its bar");
    }

    #[test]
    fn a_floating_window_holds_several_tabs_and_drags_back_into_a_dock() {
        let (mut app, ctx) = app();
        app.dock.move_to(Panel::Colour, Target::Float(Some([500.0, 260.0])));
        settle(&ctx, &mut app);
        let id = app.dock.floating[0].id;
        // History's tab dropped on the window's tab strip: a second tab there.
        let strip = drawn(&app, Over::Window(id)).map(|d| d.header.center());
        let history = app.dock_view.drawn.iter().find(|d| d.first == Panel::History).map(|d| d.header.left_center() + vec2(50.0, 0.0));
        let (Some(strip), Some(history)) = (strip, history) else { panic!("drawn: {:?}", app.dock_view.drawn) };
        drag(&ctx, &mut app, history, strip);
        assert_eq!(app.dock.floating.len(), 1);
        assert_eq!(app.dock.floating[0].tabs, vec![Panel::Colour, Panel::History]);
        assert!(app.dock.is_shown(Panel::History) && !app.dock.is_shown(Panel::Colour));
        // Clicking the Colour tab shows it.
        // The first tab, after the window's grip.
        let colour = drawn(&app, Over::Window(id)).map(|d| d.header.left_center() + vec2(40.0, 0.0));
        let Some(colour) = colour else { panic!("tab drawn") };
        click(&ctx, &mut app, colour);
        assert!(app.dock.is_shown(Panel::Colour));
        // The window dragged by its title bar onto the Library's header: both tabs join it.
        let title = ctx.memory(|m| m.area_rect(Id::new(("dock_float", id)))).map(|r| r.left_top() + vec2(30.0, 12.0));
        let library = app.dock_view.drawn.iter().find(|d| d.first == Panel::Library).map(|d| d.header.center());
        let (Some(title), Some(library)) = (title, library) else { panic!("drawn") };
        frame(&ctx, &mut app, pointer(title, None));
        frame(&ctx, &mut app, pointer(title, Some(true)));
        for i in 1..=8 {
            frame(&ctx, &mut app, pointer(title + (library - title) * (i as f32 / 8.0), None));
        }
        assert_eq!(app.dock_view.drag, Some(Drag::Window(id)), "drop targets show while the window is dragged");
        frame(&ctx, &mut app, pointer(library, Some(false)));
        settle(&ctx, &mut app);
        assert!(app.dock.floating.is_empty(), "{:?}", app.dock.floating);
        let Some(Place::Docked { side: Side::Right, group, .. }) = app.dock.place(Panel::Colour) else { panic!("{:?}", app.dock.place(Panel::Colour)) };
        assert_eq!(app.dock.right.groups[group].tabs, vec![Panel::Library, Panel::MovieExplorer, Panel::Colour, Panel::History], "in the window's order");
        assert!(app.dock.is_shown(Panel::Colour), "the one shown stays in front");
    }

    #[test]
    fn a_dock_collapses_to_icons_with_flyouts() {
        let (mut app, ctx) = app();
        settle(&ctx, &mut app);
        let collapse = ctx.read_response(Id::new(("dock_collapse", Side::Right))).map(|r| r.rect.center());
        let Some(collapse) = collapse else { panic!("drawn") };
        click(&ctx, &mut app, collapse);
        assert!(app.dock.right.collapsed);
        let strip = drawn(&app, Over::Strip(Side::Right)).map(|d| d.rect);
        let Some(strip) = strip else { panic!("a strip") };
        assert!((strip.width() - STRIP).abs() < 2.0, "a slim strip: {}", strip.width());
        let stage = app.dock_view.stage.map_or(0.0, |s| s.width());
        // An icon opens its panel over the stage, which keeps its size.
        let icon = ctx.read_response(Id::new(("dock_icon", Panel::Library.id()))).map(|r| r.rect.center());
        let Some(icon) = icon else { panic!("an icon") };
        click(&ctx, &mut app, icon);
        assert_eq!(app.dock_view.flyout, Some(Panel::Library));
        assert!(app.panel_visible(Panel::Library) && !app.panel_visible(Panel::Properties));
        let fly = ctx.memory(|m| m.area_rect(Id::new("dock_flyout")));
        assert!(fly.is_some_and(|r| r.right() <= strip.left() + 1.0 && r.width() >= MIN_SIDE), "beside the strip: {fly:?}");
        assert_eq!(app.dock_view.stage.map_or(0.0, |s| s.width()), stage);
        // Esc closes it; so does a click outside it.
        frame(&ctx, &mut app, key(egui::Key::Escape));
        settle(&ctx, &mut app);
        assert_eq!(app.dock_view.flyout, None);
        click(&ctx, &mut app, icon);
        assert_eq!(app.dock_view.flyout, Some(Panel::Library));
        let Some(s) = app.dock_view.stage else { panic!("stage") };
        click(&ctx, &mut app, s.left_center() + vec2(40.0, 0.0));
        assert_eq!(app.dock_view.flyout, None);
        // The Window menu opens a panel of a collapsed dock as a flyout; Ctrl+F6 reaches it.
        app.toggle_panel(Panel::Properties);
        settle(&ctx, &mut app);
        assert_eq!(app.dock_view.flyout, Some(Panel::Properties));
        assert!(a11y::order(&ctx).contains(&Region::Panel(Panel::Properties)));
        // Expanded again: the dock as it was.
        let expand = ctx.read_response(Id::new(("dock_expand", Side::Right))).map(|r| r.rect.center());
        let Some(expand) = expand else { panic!("drawn") };
        click(&ctx, &mut app, expand);
        assert!(!app.dock.right.collapsed && app.dock_view.flyout.is_none());
        assert_eq!(app.dock.right.groups, built_in("essentials").expect("built in").right.groups);
    }

    #[test]
    fn a_floating_window_keeps_its_size_from_frame_to_frame() {
        // Its size is kept as the arrangement changes; fed back as its contents' least size
        // it grew a little every frame.
        let (mut app, ctx) = app();
        app.dock.move_to(Panel::Colour, Target::Float(Some([760.0, 240.0])));
        app.dock.move_to(Panel::Swatches, Target::Stack(Panel::Colour));
        settle(&ctx, &mut app);
        let before = app.dock.floating[0].size;
        for _ in 0..10 {
            frame(&ctx, &mut app, Vec::new());
        }
        assert_eq!(app.dock.floating[0].size, before);
        assert_eq!(before, Panel::Colour.float_size());
    }
}
