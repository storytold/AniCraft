//! Menu bar generated from the engine's command registry, plus a few view-only items.

use crate::app::App;
use crate::platform::{self, Os};
use crate::theme::{Type, text};
use anicraft_engine::{CommandSpec, commands};

// Commands sits where Animate has it: after Modify, before Control.
// English keys (the registry's menu paths start with them), translated where they are drawn.
pub(crate) const TOP_MENUS: [&str; 9] = [tr_noop!("File"), tr_noop!("Edit"), tr_noop!("View"), tr_noop!("Insert"), tr_noop!("Modify"), tr_noop!("Commands"), tr_noop!("Control"), tr_noop!("Window"), tr_noop!("Help")];

pub fn bar(app: &mut App, ui: &mut egui::Ui) {
    let cmds = commands();
    let ctx = ui.ctx().clone();
    let labels = top_labels();
    let keys = access_keys(&labels.iter().map(String::as_str).collect::<Vec<_>>());
    app.menu_keys.before(&ctx);
    let underline = app.menu_keys.show_keys;
    let mut tops = Vec::with_capacity(TOP_MENUS.len());
    egui::MenuBar::new().ui(ui, |ui| {
        // macOS: leave room for the traffic lights over the unified title bar.
        ui.add_space(platform::menu_inset());
        if platform::OS == Os::Mac {
            // The application menu, as every Mac app has (About, Settings, Quit).
            ui.menu_button(text(Type::Body, "AniCraft").strong(), |ui| {
                ui.set_min_width(220.0);
                if ui_item(ui, tr!("About AniCraft"), "", None) {
                    app.show_about = true;
                }
                ui.separator();
                if ui_item(ui, tr!("Settings…"), "Ctrl+,", None) {
                    app.open_prefs();
                }
                ui.separator();
                if ui_item(ui, tr!("Quit AniCraft"), "Ctrl+Q", None) {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
        }
        // Windows and Linux: no name before the menus; the window title already shows it.
        for ((top, label), key) in TOP_MENUS.into_iter().zip(&labels).zip(&keys) {
            let title = if underline { egui::WidgetText::from(crate::theme::access_key_label(label, key.map(|(i, _)| i))) } else { egui::WidgetText::from(label.as_str()) };
            let r = ui.menu_button(title, |ui| {
                ui.set_min_width(230.0);
                // The menu's own area, which egui leaves anonymous: a menu named by its title.
                ui.ctx().accesskit_node_builder(ui.layer_id().id.with("move"), |n| {
                    n.set_role(egui::accesskit::Role::Menu);
                    n.set_label(label.as_str());
                });
                menu_contents(app, ui, top, &cmds);
            });
            if let Some((_, c)) = key
                && MenuKeys::available()
            {
                let k = platform::shortcut_label(&format!("Alt+{c}"));
                ctx.accesskit_node_builder(r.response.id, |n| n.set_access_key(k));
            }
            tops.push(r.response.id);
        }
    });
    app.menu_keys.after(&ctx, tops);
}

/// The top menus' titles in the interface language.
fn top_labels() -> Vec<String> {
    TOP_MENUS.iter().map(|t| crate::i18n::t(t).to_string()).collect()
}

/// Each top menu's access key: `(character index, letter)`, the first letter of its title that
/// no menu before it took (word starts first); ASCII letters only, so a title without one has
/// none.
pub fn access_keys(labels: &[&str]) -> Vec<Option<(usize, char)>> {
    let mut used: Vec<char> = Vec::new();
    labels
        .iter()
        .map(|l| {
            let chars: Vec<char> = l.chars().collect();
            let starts = (0..chars.len()).filter(|&i| i == 0 || chars.get(i - 1).is_some_and(|c| c.is_whitespace()));
            let pick = starts.chain(0..chars.len()).find_map(|i| {
                let c = chars.get(i)?.to_ascii_uppercase();
                (c.is_ascii_alphabetic() && !used.contains(&c)).then_some((i, c))
            });
            if let Some((_, c)) = pick {
                used.push(c);
            }
            pick
        })
        .collect()
}

/// The menu bar from the keyboard (ADR 0064), as on Windows and Linux: Alt (pressed and let go
/// alone) or F10 focuses the first menu; Left and Right move between menus; Down, Enter or Space
/// opens one and goes to its first item; Up and Down move through items (wrapping), Right opens
/// a submenu (or goes on to the next menu), Left comes back; Esc closes the menu, and again
/// gives the keyboard back to where it was. Alt+letter opens a menu directly; while Alt is held,
/// or the bar has the keyboard, each title shows its letter underlined. macOS keeps its own
/// conventions (the command palette, ⌘K, reaches every menu command): nothing here runs there.
#[derive(Default)]
pub struct MenuKeys {
    /// The top menu buttons, as drawn last.
    tops: Vec<egui::Id>,
    alt_down: bool,
    /// Alt went down with nothing else since: letting go of it is a tap.
    alt_clean: bool,
    /// Where the keyboard was before the bar took it.
    back: Option<egui::Id>,
    /// The bar or a menu had the keyboard last frame.
    engaged: bool,
    focus_top: Option<usize>,
    /// A menu or submenu just opened from the keyboard: its first item takes focus once drawn
    /// (within the frames left).
    into: Option<(egui::LayerId, u8)>,
    /// Submenus entered from the keyboard: their buttons and layers, outermost first.
    stack: Vec<(egui::Id, egui::LayerId)>,
    /// Underline the access keys.
    pub show_keys: bool,
}

enum At {
    Top(usize),
    Item(egui::Id, egui::LayerId),
    Elsewhere,
}

impl MenuKeys {
    /// Alt and F10 work on Windows, Linux and the web, not on a Mac (in a browser or not).
    pub fn available() -> bool {
        platform::menu_bar_keys()
    }

    fn popup(top: egui::Id) -> egui::Id {
        top.with("popup")
    }

    fn layer(top: egui::Id) -> egui::LayerId {
        egui::LayerId::new(egui::Order::Foreground, Self::popup(top))
    }

    fn open_top(&self, ctx: &egui::Context) -> Option<usize> {
        self.tops.iter().position(|t| egui::Popup::is_id_open(ctx, Self::popup(*t)))
    }

    /// Where the keyboard is: on a top menu, on an item of an open menu, or elsewhere.
    fn at(&self, ctx: &egui::Context) -> At {
        let Some(f) = ctx.memory(|m| m.focused()) else { return At::Elsewhere };
        if let Some(i) = self.tops.iter().position(|t| *t == f) {
            return At::Top(i);
        }
        match ctx.read_response(f) {
            Some(r) if self.open_top(ctx).is_some() && r.layer_id.order == egui::Order::Foreground => At::Item(f, r.layer_id),
            _ => At::Elsewhere,
        }
    }

    /// The top menu buttons (tests).
    #[cfg(test)]
    pub fn top_ids(&self) -> &[egui::Id] {
        &self.tops
    }

    /// The open menu's items, top to bottom (tests).
    #[cfg(test)]
    pub fn open_items(&self, ctx: &egui::Context) -> Vec<egui::Id> {
        self.open_top(ctx).and_then(|i| self.tops.get(i)).map(|t| Self::items(ctx, Self::layer(*t), false)).unwrap_or_default()
    }

    /// Whether the menu bar or one of its menus has the keyboard.
    pub fn engaged(&self, ctx: &egui::Context) -> bool {
        !matches!(self.at(ctx), At::Elsewhere)
    }

    /// The items of a menu drawn in `layer`, top to bottom: from this pass (`now`) or the last.
    fn items(ctx: &egui::Context, layer: egui::LayerId, now: bool) -> Vec<egui::Id> {
        let mut v: Vec<(f32, f32, egui::Id)> = ctx.viewport(|vp| {
            let pass = if now { &vp.this_pass } else { &vp.prev_pass };
            // (Not the menu's own area, which egui makes clickable.)
            let area = layer.id.with("move");
            pass.widgets.get_layer(layer).filter(|w| w.enabled && w.sense.is_focusable() && w.id != area).map(|w| (w.rect.top(), w.rect.left(), w.id)).collect()
        });
        v.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        v.into_iter().map(|(_, _, id)| id).collect()
    }

    fn step(&self, i: usize, forward: bool) -> usize {
        let n = self.tops.len().max(1);
        if forward { (i + 1) % n } else { (i + n - 1) % n }
    }

    fn open(&mut self, ctx: &egui::Context, i: usize) {
        if let Some(t) = self.tops.get(i).copied() {
            egui::Popup::open_id(ctx, Self::popup(t));
            self.into = Some((Self::layer(t), 3));
            self.focus_top = Some(i);
            self.stack.clear();
        }
    }

    fn leave(&mut self, ctx: &egui::Context) {
        egui::Popup::close_all(ctx);
        self.stack.clear();
        self.into = None;
        self.focus_top = None;
        let back = self.back.take();
        ctx.memory_mut(|m| match back {
            Some(id) => m.request_focus(id),
            None => {
                if let Some(f) = m.focused() {
                    m.surrender_focus(f);
                }
            }
        });
    }

    /// Alt pressed and let go with nothing else in between.
    fn alt_tap(&mut self, ctx: &egui::Context) -> bool {
        let (alt, other, focused) = ctx.input(|i| {
            let events = i.events.iter().any(|e| matches!(e, egui::Event::Key { .. } | egui::Event::PointerButton { .. } | egui::Event::Text(_) | egui::Event::MouseWheel { .. }));
            (i.modifiers.alt, events || i.modifiers.ctrl || i.modifiers.shift || i.modifiers.mac_cmd, i.focused)
        });
        let mut tap = false;
        if alt && focused {
            if !self.alt_down {
                self.alt_clean = true;
            }
            if other {
                self.alt_clean = false;
            }
        } else {
            tap = self.alt_down && self.alt_clean && !other && focused;
            self.alt_clean = false;
        }
        self.alt_down = alt && focused;
        tap
    }

    /// Alt+letter opens that menu, unless the shortcut table has the key (the user bound it).
    /// Runs before the shortcuts: egui matches a plain "H" with Alt held too.
    pub fn access_key(&mut self, ctx: &egui::Context, bound: impl Fn(egui::Key) -> bool) {
        if !Self::available() || !ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, modifiers, .. } if modifiers.alt))) {
            return;
        }
        let labels = top_labels();
        for (i, k) in access_keys(&labels.iter().map(String::as_str).collect::<Vec<_>>()).into_iter().enumerate() {
            let Some(key) = k.and_then(|(_, c)| egui::Key::from_name(&c.to_string())) else { continue };
            if !bound(key) && ctx.input_mut(|inp| inp.consume_key(egui::Modifiers::ALT, key)) {
                if !self.engaged(ctx) {
                    self.back = ctx.memory(|m| m.focused());
                }
                self.open(ctx, i);
                return;
            }
        }
    }

    /// The keyboard left the bar: a chosen item closed its menu (the keyboard goes back where it
    /// was), or a click went elsewhere (it stays there).
    fn settle(&mut self, ctx: &egui::Context, engaged: bool) {
        if self.engaged && !engaged && self.into.is_none() && self.focus_top.is_none() {
            // (A closed menu's item keeps the focus until the end of the frame.)
            let lost = ctx.memory(|m| m.focused()).is_none_or(|f| ctx.read_response(f).is_none_or(|r| r.layer_id.order == egui::Order::Foreground));
            if let Some(id) = self.back.take().filter(|_| lost) {
                ctx.memory_mut(|m| m.request_focus(id));
            }
            self.stack.clear();
            self.engaged = false;
        }
    }

    /// Before the bar draws: the keys that reach it.
    fn before(&mut self, ctx: &egui::Context) {
        use egui::{Key, Modifiers};
        if !Self::available() {
            self.show_keys = false;
            return;
        }
        let at = self.at(ctx);
        let engaged = !matches!(at, At::Elsewhere);
        self.settle(ctx, engaged);
        let tap = self.alt_tap(ctx);
        let f10 = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F10));
        if tap || f10 {
            if engaged || self.open_top(ctx).is_some() {
                self.leave(ctx);
            } else {
                self.back = ctx.memory(|m| m.focused());
                self.focus_top = Some(0);
            }
            return;
        }
        let pressed = |k: Key| ctx.input_mut(|i| i.consume_key(Modifiers::NONE, k));
        let enter = || ctx.input(|i| i.key_pressed(Key::Enter) || i.key_pressed(Key::Space));
        match at {
            At::Top(i) => {
                let open = self.open_top(ctx).is_some();
                let side = if pressed(Key::ArrowLeft) {
                    Some(false)
                } else if pressed(Key::ArrowRight) {
                    Some(true)
                } else {
                    None
                };
                if let Some(forward) = side {
                    let j = self.step(i, forward);
                    if open {
                        self.open(ctx, j);
                    } else {
                        self.focus_top = Some(j);
                    }
                } else if pressed(Key::ArrowDown) {
                    self.open(ctx, i);
                } else if pressed(Key::ArrowUp) {
                    // (Nothing above the bar.)
                } else if pressed(Key::Escape) {
                    if open {
                        egui::Popup::close_all(ctx);
                    } else {
                        self.leave(ctx);
                    }
                } else if enter()
                    && let Some(t) = self.tops.get(i)
                {
                    // egui presses the button, which opens its menu: its first item comes next.
                    self.into = Some((Self::layer(*t), 3));
                }
            }
            At::Item(f, layer) => {
                let items = Self::items(ctx, layer, false);
                let pos = items.iter().position(|x| *x == f).unwrap_or(0);
                let n = items.len().max(1);
                let open = self.open_top(ctx).unwrap_or(0);
                let focus = |id: Option<&egui::Id>| {
                    if let Some(id) = id.copied() {
                        ctx.memory_mut(|m| m.request_focus(id));
                    }
                };
                let sub = egui::LayerId::new(egui::Order::Foreground, f.with("submenu"));
                if pressed(Key::ArrowDown) {
                    focus(items.get((pos + 1) % n));
                } else if pressed(Key::ArrowUp) {
                    focus(items.get((pos + n - 1) % n));
                } else if pressed(Key::ArrowRight) {
                    // On an open submenu's button: into it. Elsewhere: the next menu.
                    let inside = Self::items(ctx, sub, false);
                    if inside.is_empty() {
                        let j = self.step(open, true);
                        self.open(ctx, j);
                    } else {
                        self.stack.push((f, sub));
                        focus(inside.first());
                    }
                } else if pressed(Key::ArrowLeft) {
                    match self.stack.last().copied() {
                        Some((button, l)) if l == layer => {
                            self.stack.pop();
                            focus(Some(&button));
                        }
                        _ => {
                            let j = self.step(open, false);
                            self.open(ctx, j);
                        }
                    }
                } else if pressed(Key::Escape) {
                    egui::Popup::close_all(ctx);
                    self.stack.clear();
                    self.focus_top = Some(open);
                } else if enter() {
                    // Pressing a submenu's button opens it: go in once it shows.
                    self.into = Some((sub, 3));
                }
            }
            At::Elsewhere => {}
        }
    }

    /// After the bar drew: focus moves, and the arrows and Esc stay with the bar.
    fn after(&mut self, ctx: &egui::Context, tops: Vec<egui::Id>) {
        self.tops = tops;
        if !Self::available() {
            return;
        }
        if let Some(i) = self.focus_top.take()
            && let Some(t) = self.tops.get(i).copied()
            && ctx.memory(|m| m.focused()) != Some(t)
        {
            ctx.memory_mut(|m| m.request_focus(t));
        }
        if let Some((layer, left)) = self.into.take() {
            match Self::items(ctx, layer, true).first().copied() {
                Some(first) => {
                    let top_layer = self.tops.iter().any(|t| Self::layer(*t) == layer);
                    if !top_layer && let Some(button) = ctx.memory(|m| m.focused()) {
                        // A submenu opened with Enter: Left comes back to its button.
                        self.stack.push((button, layer));
                    }
                    ctx.memory_mut(|m| m.request_focus(first));
                }
                None if left > 1 => self.into = Some((layer, left - 1)),
                None => {}
            }
            ctx.request_repaint();
        }
        let engaged = self.engaged(ctx);
        self.settle(ctx, engaged);
        if engaged && let Some(f) = ctx.memory(|m| m.focused()) {
            let filter = egui::EventFilter { tab: false, horizontal_arrows: true, vertical_arrows: true, escape: true };
            ctx.memory_mut(|m| m.set_focus_lock_filter(f, filter));
        }
        self.engaged = engaged || self.focus_top.is_some() || self.into.is_some();
        let show = engaged || ctx.input(|i| i.modifiers.alt && !i.modifiers.ctrl);
        if show != self.show_keys {
            ctx.request_repaint();
        }
        self.show_keys = show;
    }
}

fn enabled(app: &App, id: &str) -> bool {
    // With Edit Multiple Frames on, the selection is on several keyframes (ADR 0051).
    if let Some(on) = crate::onion::enabled(app, id) {
        return on;
    }
    match id {
        "edit.undo" => app.s.can_undo(),
        "edit.redo" => app.s.can_redo(),
        "edit.delete" | "edit.duplicate" | "modify.convert_to_symbol" | "modify.break_apart" | "arrange.bring_to_front" | "arrange.send_to_back" => !app.s.selection.is_empty(),
        "layer.delete" => app.s.timeline().layers.len() > 1,
        "scene.delete" => app.s.doc.scenes.len() > 1,
        // Ctrl+E opens the selected instance, or goes back to the document (ADR 0045).
        "symbol.exit" => !app.s.editing.is_empty(),
        "symbol.edit" => !app.s.editing.is_empty() || app.s.selected_elements().iter().any(|(_, e)| matches!(e.kind, anicraft_engine::doc::ElementKind::Symbol { .. })),
        "scene.prev" | "scene.move_up" => app.s.scene > 0,
        "scene.next" | "scene.move_down" => app.s.scene + 1 < app.s.doc.scenes.len(),
        "timeline.clear_keyframe" => match app.frame_sel {
            Some(sel) => sel.has_keys(&app.s.timeline().layers),
            None => app.s.active_layer().is_some_and(|l| l.keyframe_starting_at(app.s.frame).is_some()),
        },
        "timeline.paste_frames" => !app.s.frame_clipboard.is_empty(),
        "tween.add_hint" => app.s.active_layer().and_then(|l| l.hint_tween_at(app.s.frame)).is_some(),
        "tween.clear_hints" => app.s.active_layer().and_then(|l| l.hint_tween_at(app.s.frame).and_then(|(i, _)| l.keyframes.get(i))).is_some_and(|k| !k.hints.is_empty()),
        "timeline.remove_frame" | "timeline.create_tween" | "timeline.create_shape_tween" => app.s.active_layer().is_some_and(|l| l.keyframe_at(app.s.frame).is_some()),
        _ => true,
    }
}

/// Whether a registry command that switches a setting has it on (`None`: not a setting).
fn checked(app: &App, id: &str) -> Option<bool> {
    match id {
        "draw.snap_to_objects" => Some(app.s.style.snap_objects),
        "draw.object_drawing" => Some(app.s.style.object_drawing),
        _ => None,
    }
}

fn item(app: &mut App, ui: &mut egui::Ui, spec: &CommandSpec) {
    let label = match spec.id {
        "edit.undo" => app.s.undo_label().map_or_else(|| crate::i18n::t_cmd(spec.id, spec.label).to_string(), |l| tr!("Undo {action}", action = crate::i18n::t_dyn(l))),
        _ => crate::i18n::t_cmd(spec.id, spec.label).to_string(),
    };
    // Settings show whether they are on, as check items do.
    let label = match checked(app, spec.id) {
        Some(true) => format!("✔ {label}"),
        Some(false) => format!("   {label}"),
        None => label,
    };
    let button = egui::Button::new(label).shortcut_text(text(Type::Key, platform::shortcut_label(app.keymap.primary(spec.id).unwrap_or(""))));
    // What this build can't do stays in its usual place, dimmed, saying why (the web build).
    let missing = platform::unavailable(spec.id);
    let r = ui.add_enabled(enabled(app, spec.id) && missing.is_none(), button);
    let r = match missing {
        Some(why) => r.on_disabled_hover_text(crate::i18n::t(why)),
        None => r,
    };
    if r.clicked() {
        pressed(ui);
        app.invoke(spec.id);
        ui.close();
    }
}

/// An item was pressed: the Enter or Space that pressed it from the keyboard is used up, so it
/// doesn't also reach what the item opens (the command palette's field, a dialog's button).
fn pressed(ui: &egui::Ui) {
    ui.input_mut(|i| {
        i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
        i.consume_key(egui::Modifiers::NONE, egui::Key::Space);
    });
}

/// A registry command as a menu item (label and shortcut from the registry); used by context
/// menus so every surface shows the same names and keys.
pub fn cmd_item(app: &mut App, ui: &mut egui::Ui, id: &str) {
    if let Some(spec) = anicraft_engine::find_command(id) {
        item(app, ui, &spec);
    }
}

/// A check item for state that is not a registry command's own (context menus). `label` is
/// already in the interface language (`tr!`).
pub fn check_item(ui: &mut egui::Ui, label: &str, checked: bool, enabled: bool) -> bool {
    let label = if checked { format!("✔ {label}") } else { format!("   {label}") };
    let clicked = ui.add_enabled(enabled, egui::Button::new(label)).clicked();
    if clicked {
        pressed(ui);
        ui.close();
    }
    clicked
}

/// A view item (not a registry command); `label` is already translated.
fn ui_item(ui: &mut egui::Ui, label: &str, shortcut: &str, checked: Option<bool>) -> bool {
    let label_text = match checked {
        Some(true) => format!("✔ {label}"),
        Some(false) => format!("   {label}"),
        None => label.to_string(),
    };
    let clicked = ui.add(egui::Button::new(label_text).shortcut_text(text(Type::Key, platform::shortcut_label(shortcut)))).clicked();
    if clicked {
        pressed(ui);
        ui.close();
    }
    clicked
}

/// A top menu's registry items: its own, then one submenu per "Top/Sub" path, in order.
fn registry_items(app: &mut App, ui: &mut egui::Ui, top: &str, cmds: &[CommandSpec]) {
    for spec in cmds.iter().filter(|c| c.menu == top) {
        item(app, ui, spec);
        if spec.id == "file.open" {
            // Below Open, as in Animate: the recent files, and the start screen (ADR 0059).
            ui.menu_button(tr!("Open Recent"), |ui| crate::recent::menu(app, ui));
            if ui_item(ui, tr!("Start Screen"), "", None) {
                crate::start::open(app);
            }
        }
    }
    let mut subs: Vec<&str> = Vec::new();
    for c in cmds {
        if let Some(sub) = c.menu.strip_prefix(top).and_then(|r| r.strip_prefix('/'))
            && !subs.contains(&sub)
        {
            subs.push(sub);
        }
    }
    for sub in subs {
        let path = format!("{top}/{sub}");
        ui.menu_button(crate::i18n::t(sub), |ui| {
            ui.set_min_width(220.0);
            for spec in cmds.iter().filter(|c| c.menu == path) {
                item(app, ui, spec);
            }
        });
    }
}

pub(crate) fn menu_contents(app: &mut App, ui: &mut egui::Ui, top: &str, cmds: &[CommandSpec]) {
    if top == "Commands" {
        // Installed plug-ins first (they change at run time), then the registry's items.
        crate::plugins_ui::menu(app, ui, cmds, item);
        return;
    }
    // Direct items first, then one submenu per "Top/Sub" path, in registry order (in View,
    // after its zoom and preview items, where Animate keeps Snapping).
    if top != "View" {
        registry_items(app, ui, top, cmds);
    }

    match top {
        // Windows / Linux conventions; on macOS these live in the AniCraft (application) menu.
        // A web page is closed with its tab.
        "File" if platform::OS != Os::Mac && !platform::WEB => {
            ui.separator();
            if ui_item(ui, tr!("Exit"), "Ctrl+Q", None) {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        "Edit" if platform::OS != Os::Mac => {
            ui.separator();
            if ui_item(ui, tr!("Preferences…"), "Ctrl+U", None) {
                app.open_prefs();
            }
        }
        "View" => {
            let canvas = app.stage_cache.canvas;
            let center = canvas.center();
            if ui_item(ui, tr!("Zoom In"), "Ctrl+=", None) {
                let z = app.view.zoom * 1.5;
                app.zoom_about(canvas, center, z);
            }
            if ui_item(ui, tr!("Zoom Out"), "Ctrl+-", None) {
                let z = app.view.zoom / 1.5;
                app.zoom_about(canvas, center, z);
            }
            if ui_item(ui, tr!("Fit in Window"), "Ctrl+0", None) {
                app.view.fit = true;
            }
            if ui_item(ui, "100%", "Ctrl+1", None) {
                app.zoom_about(canvas, center, 1.0);
            }
            ui.separator();
            if ui_item(ui, tr!("Onion Skin"), app.keymap.primary("view.onion").unwrap_or(""), Some(app.onion.ghosts)) {
                crate::onion::toggle_ghosts(app);
            }
            if ui_item(ui, tr!("Edit Multiple Frames"), "", Some(app.onion.edit_multiple)) {
                crate::onion::toggle_edit_multiple(app);
            }
            ui.menu_button(tr!("Onion Skin Options"), |ui| crate::onion::menu(app, ui));
            if ui_item(ui, tr!("Preview Through Camera"), "", Some(app.camera_preview)) {
                app.toggle_camera_preview();
            }
            ui.menu_button(tr!("Stage Rendering"), |ui| {
                for (mode, name, tip) in crate::prefs::RenderMode::ALL {
                    if ui_item(ui, crate::i18n::t(name), "", Some(app.prefs.render_mode == mode)) {
                        let mut p = app.prefs.clone();
                        p.render_mode = mode;
                        let ctx = ui.ctx().clone();
                        app.set_prefs(&ctx, p);
                    }
                    ui.label(text(Type::Caption, crate::i18n::t(tip)));
                }
                if crate::gpu_stage::AVAILABLE {
                    ui.separator();
                    for (choice, name, tip) in crate::prefs::StageRenderer::ALL {
                        if ui_item(ui, crate::i18n::t(name), "", Some(app.prefs.stage_renderer == choice)) {
                            let mut p = app.prefs.clone();
                            p.stage_renderer = choice;
                            let ctx = ui.ctx().clone();
                            app.set_prefs(&ctx, p);
                        }
                        ui.label(text(Type::Caption, crate::i18n::t(tip)));
                    }
                }
            });
            ui.separator();
            registry_items(app, ui, top, cmds);
        }
        "Control" => {
            ui.separator();
            if ui_item(ui, if app.playing { tr!("Stop") } else { tr!("Play") }, "Enter", None) {
                app.toggle_play();
            }
            if ui_item(ui, tr!("Loop Playback"), "", Some(app.looping)) {
                app.looping = !app.looping;
            }
        }
        "Window" => {
            use crate::app::BottomTab;
            if ui_item(ui, tr!("Timeline"), "", Some(app.bottom == BottomTab::Timeline)) {
                app.bottom = BottomTab::Timeline;
            }
            if ui_item(ui, tr!("Motion Editor"), "", Some(app.bottom == BottomTab::MotionEditor)) {
                app.bottom = BottomTab::MotionEditor;
            }
            if ui_item(ui, tr!("Actions"), app.keymap.primary("view.actions").unwrap_or(""), Some(app.show_actions)) {
                app.show_actions = !app.show_actions;
            }
            use crate::movie_explorer::DockTab;
            if ui_item(ui, tr!("Library"), "", Some(app.explorer.tab == DockTab::Library)) {
                app.explorer.tab = DockTab::Library;
            }
            if ui_item(ui, tr!("Movie Explorer"), app.keymap.primary("view.movie_explorer").unwrap_or(""), Some(app.explorer.tab == DockTab::Explorer)) {
                crate::movie_explorer::open(app);
            }
            // Brush presets with previews, and your own (ADR 0068).
            if ui_item(ui, tr!("Brush Library"), "", Some(app.brush_lib.open)) {
                crate::brush_panel::toggle(app);
            }
            ui.separator();
            if ui_item(ui, tr!("Onion Skin"), "", Some(app.onion.ghosts)) {
                crate::onion::toggle_ghosts(app);
            }
            ui.label(text(Type::Caption, tr!("Panels are docked: Tools, Stage, Timeline, Properties, Library and Movie Explorer")));
        }
        "Help" => {
            if ui_item(ui, tr!("User Guide"), "", None) {
                app.open_user_guide();
            }
            ui.separator();
            #[cfg(not(target_arch = "wasm32"))]
            if ui_item(ui, tr!("Check for Updates…"), "", None) {
                app.updates.check(ui.ctx());
            }
            if ui_item(ui, tr!("Command Palette…"), "Ctrl+K", None) {
                app.palette = Some(crate::dialogs::Palette::default());
            }
            if ui_item(ui, tr!("Keyboard Shortcuts"), "F1", None) {
                app.show_shortcuts = true;
            }
            ui.separator();
            // (The web build has no folders to show.)
            if !platform::WEB && ui_item(ui, tr!("Project Files & Docs…"), "", None) {
                app.show_files = true;
            }
            if ui_item(ui, tr!("Licenses & Notices"), "", None) {
                app.show_licenses = true;
            }
            if ui_item(ui, tr!("About AniCraft"), "", None) {
                app.show_about = true;
            }
            ui.label(text(Type::Caption, trn!("{n} command is scriptable with anicraft-cli", "{n} commands are scriptable with anicraft-cli", cmds.len())));
        }
        _ => {}
    }
}
