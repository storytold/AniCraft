//! Draws the dock (ADR 0075): the tools, the four docks around the stage with their tab groups,
//! panels floating in windows of their own (or inside the editor where there can't be one), and
//! dragging a tab to dock, stack or float its panel.
//!
//! The arrangement itself is `crate::dock` (pure, tested headless); this module only draws it
//! and turns clicks and drags into changes of it. Each change is kept in the preferences.

use crate::a11y::{self, Region};
use crate::app::App;
use crate::dock::{Group, Layout, Panel, Side, Target};
use crate::theme::{self, T, Type, font, text};
use egui::{Id, Rect, Sense, Stroke, StrokeKind, WidgetType, pos2, vec2};
use std::collections::{HashMap, HashSet};

/// A side dock's group header (the band look of Properties and the Library before docking).
const BAND_H: f32 = 32.0;
/// The least a group's body is given by dragging the line between two groups.
const MIN_BODY: f32 = 48.0;
/// The panel menu button.
const MENU_W: f32 = 26.0;
/// The narrowest a side dock gets.
const MIN_SIDE: f32 = 160.0;
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

/// The dock's view state (not saved): a tab being dragged, where groups were drawn this frame
/// (the drop targets), and floating windows of their own as they are now.
#[derive(Default)]
pub struct DockView {
    /// The panel whose tab is being dragged.
    pub drag: Option<Panel>,
    /// Bumped when a workspace is switched or reset, so the docks take its sizes.
    pub epoch: u64,
    /// The arrangement changed: written to the preferences once nothing is being dragged.
    pub save: bool,
    drawn: Vec<Drawn>,
    stage: Option<Rect>,
    /// Floating windows placed this session (later frames leave them where the user put them).
    placed: HashSet<Panel>,
    /// Where windows of their own are now (desktop points), kept when AniCraft quits.
    live: HashMap<Panel, ([f32; 2], [f32; 2])>,
}

/// A group as drawn this frame.
#[derive(Clone, Copy)]
struct Drawn {
    side: Side,
    first: Panel,
    /// The first panel of the group after it, in the same dock.
    next: Option<Panel>,
    rect: Rect,
    header: Rect,
}

/// What the dock's controls asked for this frame, applied once it has drawn.
enum Act {
    Show(Panel),
    Fold(Panel),
    Shares(Side, Vec<f32>),
    Move(Panel, Target),
    Close(Panel),
    CloseGroup(Panel),
    DockBack(Panel),
    Placed(Panel, [f32; 2], [f32; 2]),
}

// ----- the app's side ---------------------------------------------------------------------------

impl App {
    /// Brings a panel forward, reopening it where it was if it was closed.
    pub fn show_panel(&mut self, p: Panel) {
        if !self.dock.is_shown(p) {
            self.dock.show(p);
            self.dock_changed();
        }
    }

    /// The Window menu's items: close a panel in front, bring any other forward.
    pub fn toggle_panel(&mut self, p: Panel) {
        self.dock.toggle(p);
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

    fn set_layout(&mut self, l: Layout) {
        self.dock = l;
        self.dock_view.epoch += 1;
        self.dock_view.placed.clear();
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
        if let Some((at, size)) = live.get(&f.panel) {
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
    egui::Panel::left("tools").resizable(false).exact_size(theme::TOOL_BUTTON + 12.0).frame(egui::Frame::NONE.fill(T.panel).inner_margin(egui::Margin::symmetric(0, 8))).show(ui, |ui| crate::panels::tools(app, ui));
    app.dock_view.drawn.clear();
    let mut acts = Vec::new();
    for side in [Side::Left, Side::Right, Side::Bottom, Side::Top] {
        let zone = app.dock.zone(side).clone();
        if zone.groups.is_empty() {
            continue;
        }
        let id = Id::new(("dock_zone", side, app.dock_view.epoch));
        let (auto, max, min) = if side.stacks() { (dock_w, (room.x * 0.5).max(220.0), MIN_SIDE) } else { (dock_h, (room.y * 0.6).max(200.0), 120.0) };
        let size = zone.size.unwrap_or(auto).clamp(min, max);
        let panel = match side {
            Side::Left => egui::Panel::left(id),
            Side::Right => egui::Panel::right(id),
            Side::Top => egui::Panel::top(id),
            Side::Bottom => egui::Panel::bottom(id),
        };
        let shown = panel.resizable(true).default_size(size).size_range(min..=max).frame(egui::Frame::NONE.fill(T.panel)).show(ui, |ui| zone_ui(app, ui, side, &zone.groups, &mut acts));
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
    drag_and_drop(app, &ctx, &mut acts);
    apply(app, acts);
    // Ctrl+F6: the stage, each panel as docked (and floating inside the editor), the tools.
    let inside = !matches!(crate::platform::movie_host(ctx.embed_viewports(), true), crate::platform::MovieHost::Window);
    let mut order = vec![Region::Stage];
    order.extend(app.dock.order().into_iter().filter(|p| inside || !app.dock.is_floating(*p)).map(Region::Panel));
    order.push(Region::Tools);
    a11y::set_order(&ctx, order);
}

/// One dock: its groups top to bottom (left and right) or side by side (top and bottom), with
/// a line between each two to drag.
fn zone_ui(app: &mut App, ui: &mut egui::Ui, side: Side, groups: &[Group], acts: &mut Vec<Act>) {
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
        group_ui(app, ui, side, g, rect, next, acts);
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
fn group_ui(app: &mut App, ui: &mut egui::Ui, side: Side, g: &Group, rect: Rect, next: Option<Panel>, acts: &mut Vec<Act>) {
    let Some(shown) = g.shown() else { return };
    let header = if side.stacks() {
        let header = Rect::from_min_size(rect.min, vec2(rect.width(), BAND_H));
        band_header(ui, side, g, header, acts);
        header
    } else {
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).id_salt(("dock_header", shown.id())));
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        row_header(app, &mut child, side, g, rect, acts)
    };
    if let Some(first) = g.tabs.first() {
        app.dock_view.drawn.push(Drawn { side, first: *first, next, rect, header });
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

/// A side dock's group header: a band (fold chevron, the tabs, the panel menu).
fn band_header(ui: &mut egui::Ui, side: Side, g: &Group, rect: Rect, acts: &mut Vec<Act>) {
    let Some(shown) = g.shown() else { return };
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
    crate::icons::paint(&p, Rect::from_center_size(chevron.center(), vec2(14.0, 14.0)), if open { crate::icons::Icon::ChevronDown } else { crate::icons::Icon::ChevronRight }, if hot { T.text } else { T.text_dim });
    if resp.on_hover_text(if open { tr!("Fold this panel group") } else { tr!("Unfold this panel group") }).clicked() {
        acts.push(Act::Fold(shown));
    }
    // The tab strip ends before the menu button: a name never runs under it (ADR 0083). Tabs
    // too wide for it shrink, their names cut with an ellipsis (the whole name is in the
    // tooltip); at their narrowest the strip scrolls with the wheel, keeping the shown tab in view.
    let strip = Rect::from_min_max(pos2(rect.left() + 30.0, rect.top()), pos2(rect.right() - MENU_W - 4.0, rect.bottom()));
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
        tab_actions(ui, side, g, *tab, &resp, cut, acts);
        if i == active_i {
            a11y::anchor(ui, Region::Panel(*tab), Rect::from_min_max(rect.min, pos2(rect.right(), ui.max_rect().bottom().max(rect.bottom()))), &resp);
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
    menu_button(ui, side, g, shown, menu, acts);
}

/// A top or bottom dock's group header: its tabs in a row (and the timeline's own tools beside
/// them, as in Animate), the panel menu at the end. Returns the header's rect.
fn row_header(app: &mut App, ui: &mut egui::Ui, side: Side, g: &Group, group: Rect, acts: &mut Vec<Act>) -> Rect {
    let shown = g.shown();
    let frame = egui::Frame::NONE.fill(T.panel_alt).inner_margin(egui::Margin::symmetric(10, 5));
    let out = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let mut anchor = None;
            for (i, tab) in g.tabs.iter().enumerate() {
                let active = i == g.active.min(g.tabs.len() - 1);
                let r = ui.selectable_label(active, text(Type::Heading, crate::i18n::t(tab.label())));
                // A drag on the tab moves the panel; its clicks stay the label's.
                let drag = ui.interact(r.rect, Id::new(("dock_tab_drag", tab.id())), Sense::drag() - Sense::FOCUSABLE);
                if drag.drag_started() {
                    app.dock_view.drag = Some(*tab);
                }
                let r = r.on_hover_text(tab_tip(*tab));
                if r.clicked() {
                    acts.push(Act::Show(*tab));
                }
                r.context_menu(|ui| panel_menu(ui, side, g, *tab, acts));
                if active {
                    anchor = Some(r);
                }
            }
            if let (Some(p), Some(r)) = (shown, &anchor) {
                a11y::anchor(ui, Region::Panel(p), group, r);
            }
            if shown == Some(Panel::Timeline) {
                ui.add_space(4.0);
                let w = ui.available_width() - MENU_W - 8.0;
                crate::timeline::header_tools(app, ui, w);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (r, _) = ui.allocate_exact_size(vec2(MENU_W - 2.0, 24.0), Sense::hover());
                if let Some(p) = shown {
                    menu_button(ui, side, g, p, r, acts);
                }
            });
        });
    });
    out.response.rect
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
    // Drawn inside the strip only: never over the menu button.
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
fn tab_actions(ui: &egui::Ui, side: Side, g: &Group, tab: Panel, resp: &egui::Response, cut: bool, acts: &mut Vec<Act>) {
    if resp.drag_started() {
        ui.ctx().data_mut(|d| d.insert_temp(Id::new("dock_drag_start"), Some(tab)));
    }
    if resp.double_clicked() && side.stacks() {
        acts.push(Act::Fold(tab));
    } else if resp.clicked() {
        acts.push(Act::Show(tab));
    }
    let tip = if cut { format!("{}\n{}", crate::i18n::t(tab.label()), tab_tip(tab)) } else { tab_tip(tab).to_string() };
    let resp = resp.clone().on_hover_text(tip);
    resp.context_menu(|ui| panel_menu(ui, side, g, tab, acts));
}

/// The ≡ button at the end of a group's header: the shown panel's menu.
fn menu_button(ui: &mut egui::Ui, side: Side, g: &Group, p: Panel, r: Rect, acts: &mut Vec<Act>) {
    let resp = ui.interact(r, Id::new(("dock_menu", p.id())), Sense::click());
    let name = tr!("{panel} panel menu", panel = crate::i18n::t(p.label()));
    a11y::control(ui, &resp, WidgetType::Button, &name, None);
    let hot = resp.hovered() || resp.has_focus();
    if resp.hovered() {
        ui.painter().rect_filled(r, T.rc(), T.hover);
    }
    // Three short lines: the panel menu, as in other studios.
    let c = r.center();
    for dy in [-4.0, 0.0, 4.0] {
        ui.painter().hline((c.x - 6.0)..=(c.x + 6.0), c.y + dy, Stroke::new(1.5, if hot { T.icon_hot } else { T.icon }));
    }
    let resp = resp.on_hover_text(tr!("Dock, float or close this panel"));
    egui::Popup::menu(&resp).show(|ui| panel_menu(ui, side, g, p, acts));
}

/// A panel's menu: dock it on any side, float it, fold or close it (or its group).
fn panel_menu(ui: &mut egui::Ui, side: Side, g: &Group, p: Panel, acts: &mut Vec<Act>) {
    let item = |ui: &mut egui::Ui, label: &str| {
        let c = ui.button(label).clicked();
        if c {
            ui.close();
        }
        c
    };
    for s in Side::ALL {
        let here = s == side && g.tabs.len() == 1;
        if !here && item(ui, crate::i18n::t(s.label())) {
            acts.push(Act::Move(p, Target::Split { side: s, before: None }));
        }
    }
    if item(ui, tr!("Float in a Window")) {
        acts.push(Act::Move(p, Target::Float(None)));
    }
    if side.stacks() && item(ui, if g.folded { tr!("Unfold Group") } else { tr!("Fold Group") }) {
        acts.push(Act::Fold(p));
    }
    ui.separator();
    if item(ui, &tr!("Close {panel}", panel = crate::i18n::t(p.label()))) {
        acts.push(Act::Close(p));
    }
    if g.tabs.len() > 1 && item(ui, tr!("Close Tab Group")) {
        acts.push(Act::CloseGroup(p));
    }
}

/// The shown panel's contents, wherever it is.
pub fn panel_body(app: &mut App, ui: &mut egui::Ui, p: Panel) {
    let pad = |x: i8, y: i8| egui::Frame::NONE.inner_margin(egui::Margin::symmetric(x, y));
    let scroll = |salt: &'static str| egui::ScrollArea::vertical().id_salt(salt).auto_shrink([false, false]);
    match p {
        Panel::Properties => {
            scroll("props").show(ui, |ui| pad(12, 8).show(ui, |ui| crate::panels::properties(app, ui)));
        }
        Panel::Align => {
            scroll("align").show(ui, |ui| pad(12, 8).show(ui, |ui| crate::panels::align_panel(app, ui)));
        }
        Panel::Colour => {
            scroll("colour").show(ui, |ui| pad(12, 8).show(ui, |ui| crate::panels::colour_panel(app, ui)));
        }
        Panel::Swatches => {
            scroll("swatches").show(ui, |ui| pad(12, 8).show(ui, |ui| crate::swatches_panel::panel(app, ui)));
        }
        Panel::Library => {
            scroll("library_dock").show(ui, |ui| pad(12, 10).show(ui, |ui| crate::panels::library(app, ui)));
        }
        Panel::MovieExplorer => {
            pad(12, 4).show(ui, |ui| crate::movie_explorer::panel(app, ui));
        }
        Panel::History => {
            pad(12, 8).show(ui, |ui| crate::panels::history(app, ui));
        }
        Panel::LayerDepth => {
            scroll("layer_depth").show(ui, |ui| pad(12, 8).show(ui, |ui| crate::layer_depth::panel(app, ui)));
        }
        Panel::Timeline => crate::timeline::show(app, ui),
        Panel::MotionEditor => crate::motion_editor::show(app, ui),
        Panel::Actions => {
            scroll("actions").show(ui, |ui| pad(10, 8).show(ui, |ui| crate::actions_panel::panel(app, ui)));
        }
        Panel::Output => {
            pad(10, 8).show(ui, |ui| crate::test_movie::output_panel(app, ui));
        }
    }
}

// ----- dragging a tab -----------------------------------------------------------------------------

/// Where a tab dropped at `at` puts its panel, and the area that shows it.
fn drop_target(view: &DockView, p: Panel, at: egui::Pos2) -> (Target, Option<Rect>) {
    for d in &view.drawn {
        if !d.rect.contains(at) {
            continue;
        }
        if d.header.contains(at) {
            return (Target::Stack(d.first), Some(d.rect));
        }
        // The body's first half: a group of its own before this one; the second, after.
        let body = Rect::from_min_max(pos2(d.rect.left(), d.header.bottom()), d.rect.max);
        let first_half = if d.side.stacks() { at.y < body.center().y } else { at.x < body.center().x };
        let half = match (d.side.stacks(), first_half) {
            (true, true) => Rect::from_min_max(body.min, pos2(body.right(), body.center().y)),
            (true, false) => Rect::from_min_max(pos2(body.left(), body.center().y), body.max),
            (false, true) => Rect::from_min_max(d.rect.min, pos2(d.rect.center().x, d.rect.bottom())),
            (false, false) => Rect::from_min_max(pos2(d.rect.center().x, d.rect.top()), d.rect.max),
        };
        let before = if first_half { Some(d.first) } else { d.next };
        return (Target::Split { side: d.side, before }, Some(half));
    }
    // Near an edge of the stage: a dock on that side.
    if let Some(s) = view.stage
        && s.contains(at) && s.width() > 1.0 && s.height() > 1.0 {
        let (fx, fy) = ((at.x - s.left()) / s.width(), (at.y - s.top()) / s.height());
        let side = [(fx, Side::Left), (1.0 - fx, Side::Right), (fy, Side::Top), (1.0 - fy, Side::Bottom)].into_iter().filter(|(f, _)| *f < EDGE).min_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, s)| s);
        if let Some(side) = side {
            let band = match side {
                Side::Left => Rect::from_min_max(s.min, pos2(s.left() + s.width() * EDGE, s.bottom())),
                Side::Right => Rect::from_min_max(pos2(s.right() - s.width() * EDGE, s.top()), s.max),
                Side::Top => Rect::from_min_max(s.min, pos2(s.right(), s.top() + s.height() * EDGE)),
                Side::Bottom => Rect::from_min_max(pos2(s.left(), s.bottom() - s.height() * EDGE), s.max),
            };
            return (Target::Split { side, before: None }, Some(band));
        }
    }
    // Anywhere else: a window of its own there.
    let size = p.float_size();
    (Target::Float(Some([at.x, at.y])), Some(Rect::from_min_size(at, vec2(size[0], size[1]) * 0.5)))
}

/// While a tab is dragged: where it would go, shown over the editor; dropped, it goes there.
/// Esc cancels.
fn drag_and_drop(app: &mut App, ctx: &egui::Context, acts: &mut Vec<Act>) {
    if let Some(p) = ctx.data_mut(|d| d.remove_temp::<Option<Panel>>(Id::new("dock_drag_start"))).flatten() {
        app.dock_view.drag = Some(p);
    }
    let Some(p) = app.dock_view.drag else { return };
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        app.dock_view.drag = None;
        return;
    }
    let (pos, down) = ctx.input(|i| (i.pointer.latest_pos(), i.pointer.primary_down()));
    let Some(pos) = pos else { return };
    let (target, area) = drop_target(&app.dock_view, p, pos);
    if !down {
        app.dock_view.drag = None;
        let target = match target {
            // A window of its own goes where the pointer is on the desktop.
            Target::Float(Some([x, y])) => {
                let origin = ctx.input(|i| i.viewport().inner_rect).map_or(egui::Pos2::ZERO, |r| r.min);
                let own = matches!(crate::platform::movie_host(ctx.embed_viewports(), true), crate::platform::MovieHost::Window);
                Target::Float(Some(if own { [x + origin.x, y + origin.y] } else { [x, y] }))
            }
            t => t,
        };
        acts.push(Act::Move(p, target));
        return;
    }
    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::new("dock_drop")));
    if let Some(r) = area {
        painter.rect_filled(r, theme::SQUARE, theme::dock_drop());
        painter.rect_stroke(r, theme::SQUARE, Stroke::new(2.0, T.accent), StrokeKind::Inside);
    }
    let label = crate::i18n::t(p.label());
    let g = painter.layout_no_wrap(label.to_string(), font(Type::Label), T.on_fill);
    let tag = Rect::from_min_size(pos + vec2(14.0, 10.0), vec2(g.rect.width() + 16.0, g.rect.height() + 8.0));
    painter.rect_filled(tag, T.rc(), T.accent);
    painter.galley(tag.min + vec2(8.0, 4.0), g, T.on_fill);
    ctx.request_repaint();
}

fn apply(app: &mut App, acts: Vec<Act>) {
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
                if let Some(crate::dock::Place::Docked { side, group, .. }) = app.dock.place(p)
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
                if matches!(target, Target::Float(_)) {
                    app.dock_view.placed.remove(&p);
                }
                app.dock.move_to(p, target);
                changed = true;
            }
            Act::Close(p) => {
                app.dock.close(p);
                changed = true;
            }
            Act::CloseGroup(p) => {
                if let Some(crate::dock::Place::Docked { side, group, .. }) = app.dock.place(p) {
                    let tabs = app.dock.zone(side).groups.get(group).map(|g| g.tabs.clone()).unwrap_or_default();
                    for t in tabs {
                        app.dock.close(t);
                    }
                    changed = true;
                }
            }
            Act::DockBack(p) => {
                app.dock.dock_back(p);
                changed = true;
            }
            Act::Placed(p, at, size) => {
                if let Some(f) = app.dock.floating.iter_mut().find(|f| f.panel == p) {
                    f.at = Some(at);
                    f.size = size;
                    changed = true;
                }
            }
        }
    }
    if changed {
        app.dock_changed();
    }
}

// ----- floating panels ------------------------------------------------------------------------------

/// Floating panels: each in a window of its own on the desktop (an egui immediate viewport, as
/// Test Movie's, ADR 0062), or inside the editor where there can't be one (the web page, a
/// system that can't open another window).
pub fn floating(app: &mut App, ctx: &egui::Context) {
    if app.start.is_some() {
        return;
    }
    let floats = app.dock.floating.clone();
    let host = crate::platform::movie_host(ctx.embed_viewports(), true);
    let mut acts = Vec::new();
    for f in floats {
        match host {
            crate::platform::MovieHost::Window => own_window(app, ctx, &f, &mut acts),
            crate::platform::MovieHost::Inside(_) => inside(app, ctx, &f, &mut acts),
        }
    }
    apply(app, acts);
}

/// A floating panel's contents: Dock and the panel menu, then the panel.
fn float_body(app: &mut App, ui: &mut egui::Ui, p: Panel, anchor: bool, acts: &mut Vec<Act>) {
    ui.horizontal(|ui| {
        let r = ui.button(tr!("Dock")).on_hover_text(tr!("Put the panel back in the dock it came from"));
        if r.clicked() {
            acts.push(Act::DockBack(p));
        }
        if anchor {
            a11y::anchor(ui, Region::Panel(p), ui.max_rect(), &r);
        }
        if p == Panel::Timeline {
            let w = ui.available_width() - 4.0;
            crate::timeline::header_tools(app, ui, w);
        }
    });
    let rest = ui.available_rect_before_wrap();
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rest).layout(egui::Layout::top_down(egui::Align::Min)).id_salt(("dock_body", p.id())));
    child.set_clip_rect(rest.intersect(ui.clip_rect()));
    panel_body(app, &mut child, p);
    ui.allocate_rect(rest, Sense::hover());
}

/// Floating inside the editor: an egui window.
fn inside(app: &mut App, ctx: &egui::Context, f: &crate::dock::Float, acts: &mut Vec<Act>) {
    let p = f.panel;
    let title = crate::i18n::t(p.label());
    let mut open = true;
    let size = vec2(f.size[0], f.size[1]);
    let mut w = egui::Window::new(title).id(Id::new(("dock_float", p.id()))).open(&mut open).collapsible(false).resizable(true).default_size(size).min_size([crate::dock::MIN_FLOAT, 120.0]);
    let at = f.at.map_or_else(|| ctx.content_rect().center() - size * 0.5, |a| pos2(a[0], a[1]));
    w = if app.dock_view.placed.insert(p) { w.current_pos(at) } else { w.default_pos(at) };
    let shown = w.show(ctx, |ui| {
        ui.set_min_size(size - vec2(16.0, 40.0));
        float_body(app, ui, p, true, acts);
    });
    a11y::name_title_bar(&shown, title);
    if let Some(s) = &shown {
        let r = s.response.rect;
        let (now_at, now_size) = ([r.left().round(), r.top().round()], [r.width().round(), r.height().round()]);
        let moved = f.at.is_none_or(|a| (a[0] - now_at[0]).abs() > 0.5 || (a[1] - now_at[1]).abs() > 0.5);
        if (moved || (f.size[0] - now_size[0]).abs() > 1.0 || (f.size[1] - now_size[1]).abs() > 1.0) && !ctx.input(|i| i.pointer.any_down()) {
            acts.push(Act::Placed(p, now_at, now_size));
        }
    }
    if !open {
        acts.push(Act::Close(p));
    }
}

/// Floating in a window of its own: drawn in the editor's frame with the editor's state, so
/// nothing is shared across threads (as Test Movie's window, ADR 0062).
fn own_window(app: &mut App, ctx: &egui::Context, f: &crate::dock::Float, acts: &mut Vec<Act>) {
    let p = f.panel;
    let title = crate::i18n::t(p.label()).to_string();
    let mut builder = egui::ViewportBuilder::default().with_title(title.clone()).with_inner_size(f.size).with_min_inner_size([crate::dock::MIN_FLOAT, 120.0]);
    if let Some(at) = f.at.filter(|_| crate::platform::window_positions()) {
        builder = builder.with_position(at);
    }
    let builder = crate::platform::secondary_window(builder);
    let shortcuts = app.shortcuts.clone();
    let (close, commands) = ctx.show_viewport_immediate(egui::ViewportId::from_hash_of(("dock_float", p.id())), builder, |ui, class| {
        let (close, outer, inner) = ui.input(|i| (i.viewport().close_requested(), i.viewport().outer_rect, i.viewport().inner_rect));
        if let (Some(o), Some(i)) = (outer, inner) {
            app.dock_view.live.insert(p, ([o.min.x.round(), o.min.y.round()], [i.width().round(), i.height().round()]));
        }
        if class == egui::ViewportClass::EmbeddedWindow {
            // egui's own stand-in, with no window system under it (tests): already in a window.
            a11y::name_own_title_bar(ui, &title);
            float_body(app, ui, p, false, acts);
        } else {
            let frame = egui::Frame::NONE.fill(T.panel).inner_margin(egui::Margin::same(6));
            egui::CentralPanel::default().frame(frame).show(ui, |ui| float_body(app, ui, p, false, acts));
        }
        // The app's shortcuts with Ctrl (⌘) or Alt still work over it (Undo, Save, Test Movie),
        // unless a text field has the keyboard.
        let commands: Vec<&'static str> = if ui.ctx().text_edit_focused() {
            Vec::new()
        } else {
            ui.input_mut(|i| shortcuts.iter().filter(|(m, _, id)| (m.command || m.ctrl || m.alt) && !id.starts_with("view.") && !id.starts_with("tool.")).filter_map(|(m, k, id)| i.consume_key(*m, *k).then_some(*id)).collect())
        };
        (close, commands)
    });
    for id in commands {
        app.invoke(id);
    }
    if close {
        acts.push(Act::Close(p));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dock::{Place, built_in};

    fn app() -> (App, egui::Context) {
        let ctx = egui::Context::default();
        let session = anicraft_engine::Session::new(anicraft_engine::doc::sample::demo());
        // Screenshot mode: default preferences, nothing saved.
        let app = App::with_context(&ctx, session, Some(std::path::PathBuf::from("unused.png")));
        (app, ctx)
    }

    fn input(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1600.0, 940.0))), events, ..Default::default() }
    }

    fn frame(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) {
        ctx.run_ui(input(events), |ui| app.frame(ui)).textures_delta.clear();
    }

    fn settle(ctx: &egui::Context, app: &mut App) {
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
        for size in [MIN_SIDE, 200.0, 260.0, 292.0] {
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
            // Wide enough to show them all, shrunk; narrower, the strip scrolls.
            let all = tabs.iter().all(|t| rect(&ctx, *t).is_some_and(|(r, _)| r.width() >= MIN_TAB - 0.5));
            let cut = tabs.iter().any(|t| rect(&ctx, *t).is_some_and(|(_, cut)| cut));
            if size == MIN_SIDE {
                assert!(!all && cut, "{size}: the strip scrolls");
            } else if size < 250.0 {
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
        // Its groups: four on the right, one below the stage.
        let sides: Vec<Side> = app.dock_view.drawn.iter().map(|d| d.side).collect();
        assert_eq!(sides, vec![Side::Right, Side::Right, Side::Right, Side::Right, Side::Bottom]);
    }

    #[test]
    fn dragging_the_timeline_tab_docks_it_above_the_stage() {
        let (mut app, ctx) = app();
        settle(&ctx, &mut app);
        let tab = app.dock_view.drawn.iter().find(|d| d.side == Side::Bottom).map(|d| d.header.left_center() + vec2(40.0, 0.0));
        let stage = app.dock_view.stage;
        let (Some(tab), Some(stage)) = (tab, stage) else { panic!("drawn") };
        drag(&ctx, &mut app, tab, pos2(stage.center().x, stage.top() + 20.0));
        assert!(matches!(app.dock.place(Panel::Timeline), Some(Place::Docked { side: Side::Top, .. })), "{:?}", app.dock.place(Panel::Timeline));
        assert!(app.dock_view.drag.is_none());
        assert_eq!(app.prefs.workspaces.live.get("essentials"), Some(&app.dock), "kept as the arrangement of Essentials");
        // Dropped on the right dock's Library header: stacked there as a tab.
        let library = app.dock_view.drawn.iter().find(|d| d.first == Panel::Library).map(|d| d.header.center());
        let tab = app.dock_view.drawn.iter().find(|d| d.side == Side::Top).map(|d| d.header.left_center() + vec2(40.0, 0.0));
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
        assert_eq!(app.dock_view.drag, Some(Panel::History));
        frame(&ctx, &mut app, vec![egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }]);
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
        assert!(app.dock_view.placed.contains(&Panel::Output), "drawn inside the editor");
        let area = ctx.memory(|m| m.area_rect(Id::new(("dock_float", Panel::Output.id()))));
        assert!(area.is_some_and(|r| (r.left() - 300.0).abs() < 1.0 && (r.top() - 200.0).abs() < 1.0), "where it was put: {area:?}");
        // A window of its own where there can be one: drawn in its viewport, not inside.
        let (mut app, ctx) = self::app();
        ctx.set_embed_viewports(false);
        app.dock.move_to(Panel::Output, Target::Float(None));
        settle(&ctx, &mut app);
        assert!(!app.dock_view.placed.contains(&Panel::Output));
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
            if !app.dock.is_shown(p) {
                app.toggle_panel(p);
            }
            settle(&ctx, &mut app);
            assert!(app.dock.is_shown(p), "{p:?}");
            let drawn = app.dock.is_floating(p) || app.dock_view.drawn.iter().any(|d| app.dock.zone(d.side).groups.iter().any(|g| g.tabs.first() == Some(&d.first) && g.shown() == Some(p)));
            assert!(drawn, "{p:?} is on screen");
        }
    }

    #[test]
    fn workspaces_switch_and_reset_with_their_sizes() {
        let (mut app, ctx) = app();
        settle(&ctx, &mut app);
        let width = |app: &App| app.dock_view.drawn.iter().find(|d| d.side == Side::Right).map_or(0.0, |d| d.rect.width());
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
    }
}
