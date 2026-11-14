//! Floating panels (ADR 0075, ADR 0087): each floating window holds one or more panels as
//! tabs, in a window of its own on the desktop (an egui immediate viewport, as Test Movie's,
//! ADR 0062) or inside the editor where there can't be one (the web page, a system that can't
//! open another window).
//!
//! The strip at the top of a window has a grip (drag the whole window into a dock), its tabs
//! (drag one panel out, or another panel in), Dock and the shown panel's menu. Inside the
//! editor, dragging the window's title bar shows the drop targets too.

use crate::a11y;
use crate::app::App;
use crate::dock::{Float, Layout, Panel};
use crate::dock_ui::{Act, Drag, Drawn, MENU_W, Over, Where, apply, end_drag, menu_button, panel_body, region, tab_row};
use crate::theme::T;
use egui::{Id, Rect, Sense, WidgetType, pos2, vec2};

/// Draws every floating window.
pub fn floating(app: &mut App, ctx: &egui::Context) {
    app.dock_view.drawn.retain(|d| !matches!(d.over, Over::Window(_)));
    if app.start.is_some() {
        return;
    }
    let floats = app.dock.floating.clone();
    let layout = app.dock.clone();
    app.dock_view.desk.retain(|id, _| floats.iter().any(|f| f.id == *id));
    let own = crate::dock_ui::own_windows(app, ctx);
    let mut acts = Vec::new();
    for f in &floats {
        if own {
            own_window(app, ctx, f, &layout, &mut acts);
        } else {
            inside(app, ctx, f, &layout, &mut acts);
        }
    }
    apply(app, acts);
}

/// The window's strip: the grip, the tabs, Dock, the menu. Returns its rect.
fn strip(app: &mut App, ui: &mut egui::Ui, f: &Float, layout: &Layout, anchor: bool, acts: &mut Vec<Act>) -> Rect {
    let at = Where::Floating { window: f.id, tabs: f.tabs.len() };
    let shown = f.shown();
    ui.horizontal(|ui| {
        // The grip: dragged, the whole window; clicked, the shown panel's menu.
        let (g, resp) = ui.allocate_exact_size(vec2(14.0, 24.0), Sense::click_and_drag());
        a11y::control(ui, &resp, WidgetType::Button, tr!("Window grip: drag to dock this window"), None);
        a11y::focus_ring(ui, &resp);
        let colour = if resp.hovered() || resp.dragged() { T.icon_hot } else { T.text_faint };
        for i in -1..=1 {
            for j in [-2.0_f32, 2.0] {
                ui.painter().circle_filled(g.center() + vec2(j, i as f32 * 4.5), 1.0, colour);
            }
        }
        if app.dock_view.drag.is_none() && resp.drag_started() {
            app.dock_view.drag = Some(Drag::Window(f.id));
            app.dock_view.from_window = own(ui);
        }
        let resp = resp.on_hover_text(tr!("Drag to dock the whole window; drag a tab to move one panel"));
        if let Some(p) = shown {
            egui::Popup::menu(&resp).show(|ui| crate::dock_ui::panel_menu(ui, at, p, layout, acts));
        }
        let r = tab_row(app, ui, at, &f.tabs, f.active, layout, acts);
        if anchor
            && let (Some(p), Some(r)) = (shown, &r)
        {
            a11y::anchor(ui, region(p), ui.max_rect(), r);
        }
        let tip = if f.tabs.len() > 1 { tr!("Put the panels back in the docks they came from") } else { tr!("Put the panel back in the dock it came from") };
        if ui.button(tr!("Dock")).on_hover_text(tip).clicked() {
            acts.push(match (f.tabs.len(), shown) {
                (1, Some(p)) => Act::DockBack(p),
                _ => Act::DockBackWindow(f.id),
            });
        }
        if shown == Some(Panel::Timeline) {
            let w = ui.available_width() - MENU_W - 8.0;
            crate::timeline::header_tools(app, ui, w);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (r, _) = ui.allocate_exact_size(vec2(MENU_W - 2.0, 24.0), Sense::hover());
            if let Some(p) = shown {
                menu_button(ui, at, p, r, layout, acts);
            }
        });
    })
    .response
    .rect
}

/// The viewport a drag began in, when it is a window of its own.
fn own(ui: &egui::Ui) -> Option<egui::ViewportId> {
    let id = ui.ctx().viewport_id();
    (id != egui::ViewportId::ROOT).then_some(id)
}

/// A floating window's contents: its strip, then the shown panel. Returns the strip's rect.
fn float_body(app: &mut App, ui: &mut egui::Ui, f: &Float, layout: &Layout, anchor: bool, acts: &mut Vec<Act>) -> Rect {
    let head = strip(app, ui, f, layout, anchor, acts);
    let rest = ui.available_rect_before_wrap();
    if let Some(p) = f.shown() {
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rest).layout(egui::Layout::top_down(egui::Align::Min)).id_salt(("dock_body", p.id())));
        child.set_clip_rect(rest.intersect(ui.clip_rect()));
        panel_body(app, &mut child, p);
    }
    ui.allocate_rect(rest, Sense::hover());
    head
}

/// Floating inside the editor: an egui window.
fn inside(app: &mut App, ctx: &egui::Context, f: &Float, layout: &Layout, acts: &mut Vec<Act>) {
    let Some(p) = f.shown() else { return };
    let id = Id::new(("dock_float", f.id));
    let title = crate::i18n::t(p.label());
    let mut open = true;
    let size = vec2(f.size[0], f.size[1]);
    let mut w = egui::Window::new(title).id(id).open(&mut open).collapsible(false).resizable(true).default_size(size).min_size([crate::dock::MIN_FLOAT, 120.0]);
    let at = f.at.map_or_else(|| ctx.content_rect().center() - size * 0.5, |a| pos2(a[0], a[1]));
    w = if app.dock_view.placed.insert(f.id) { w.current_pos(at) } else { w.default_pos(at) };
    let shown = w.show(ctx, |ui| {
        float_body(app, ui, f, layout, true, acts)
    });
    a11y::name_title_bar(&shown, title);
    if let Some(s) = &shown {
        let r = s.response.rect;
        let head = s.inner.unwrap_or(r);
        if let Some(first) = f.tabs.first() {
            app.dock_view.drawn.push(Drawn { over: Over::Window(f.id), first: *first, next: None, rect: r, header: head });
        }
        let (now_at, now_size) = ([r.left().round(), r.top().round()], [r.width().round(), r.height().round()]);
        let moved = f.at.is_none_or(|a| (a[0] - now_at[0]).abs() > 0.5 || (a[1] - now_at[1]).abs() > 0.5);
        if (moved || (f.size[0] - now_size[0]).abs() > 1.0 || (f.size[1] - now_size[1]).abs() > 1.0) && !ctx.input(|i| i.pointer.any_down()) {
            acts.push(Act::Placed(f.id, now_at, now_size));
        }
    }
    // Dragged by its title bar: the drop targets show, and a dock takes it where it is let go.
    if app.dock_view.drag.is_none() && ctx.is_being_dragged(id.with("__title_click")) {
        app.dock_view.drag = Some(Drag::Window(f.id));
        app.dock_view.from_window = None;
    }
    if !open {
        acts.push(Act::CloseWindow(f.id));
    }
}

/// Floating in a window of its own: drawn in the editor's frame with the editor's state, so
/// nothing is shared across threads (as Test Movie's window, ADR 0062).
fn own_window(app: &mut App, ctx: &egui::Context, f: &Float, layout: &Layout, acts: &mut Vec<Act>) {
    let Some(p) = f.shown() else { return };
    let title = crate::i18n::t(p.label()).to_string();
    let mut builder = egui::ViewportBuilder::default().with_title(title.clone()).with_inner_size(f.size).with_min_inner_size([crate::dock::MIN_FLOAT, 120.0]);
    if let Some(at) = f.at.filter(|_| crate::platform::window_positions()) {
        builder = builder.with_position(at);
    }
    let builder = crate::platform::secondary_window(builder);
    let shortcuts = app.shortcuts.clone();
    let (close, commands) = ctx.show_viewport_immediate(egui::ViewportId::from_hash_of(("dock_float", f.id)), builder, |ui, class| {
        let (close, outer, inner) = ui.input(|i| (i.viewport().close_requested(), i.viewport().outer_rect, i.viewport().inner_rect));
        if let (Some(o), Some(i)) = (outer, inner) {
            app.dock_view.live.insert(f.id, ([o.min.x.round(), o.min.y.round()], [i.width().round(), i.height().round()]));
            app.dock_view.desk.insert(f.id, i);
        }
        // A drag that began here: this window holds the pointer until the button is let go,
        // so it tells the editor where the pointer is (the editor shows the drop targets).
        if app.dock_view.drag.is_some() && app.dock_view.from_window.is_some() && app.dock_view.from_window == own(ui) {
            let (pos, down, esc) = ui.input_mut(|i| (i.pointer.latest_pos(), i.pointer.primary_down(), i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)));
            if esc {
                end_drag(&mut app.dock_view);
            } else if let (Some(pos), Some(i)) = (pos, inner) {
                let o = app.dock_view.origin();
                app.dock_view.remote = Some((pos2(i.min.x + pos.x - o.x, i.min.y + pos.y - o.y), down));
                ui.ctx().request_repaint_of(egui::ViewportId::ROOT);
            }
        }
        if class == egui::ViewportClass::EmbeddedWindow {
            // egui's own stand-in, with no window system under it (tests): already in a window.
            a11y::name_own_title_bar(ui, &title);
            float_body(app, ui, f, layout, false, acts);
        } else {
            let frame = egui::Frame::NONE.fill(T.panel).inner_margin(egui::Margin::same(6));
            egui::CentralPanel::default().frame(frame).show(ui, |ui| float_body(app, ui, f, layout, false, acts));
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
        acts.push(Act::CloseWindow(f.id));
    }
}
