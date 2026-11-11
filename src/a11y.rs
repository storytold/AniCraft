//! Accessibility in the shell (ADR 0022, docs/accessibility.md).
//!
//! egui publishes its widgets to screen readers through AccessKit (Narrator and NVDA on
//! Windows, VoiceOver on macOS, Orca on Linux). Built-in widgets with visible text name
//! themselves; this module covers the rest:
//!
//! * [`Named::named`] gives a control without its own text (a number field, a colour well, a
//!   menu showing its current value) the name a screen reader says, e.g. "Position X".
//! * [`control`] gives a custom-painted control (tool buttons, layer toggles, icon buttons) its
//!   role, name and on/off state, and draws the keyboard focus ring (ADR 0008).
//! * [`Region`]s are the panels Ctrl+F6 / Ctrl+Shift+F6 move between: Stage, Timeline,
//!   Properties, Library and Tools. Each registers one anchor widget per frame.
//!
//! The test at the bottom runs the whole interface headless with AccessKit on and fails on any
//! interactive node without a name or role.

use egui::{Id, Rect, Response, Stroke, StrokeKind, WidgetInfo, WidgetType};

/// Names a control for assistive technology, for controls whose text is a value or nothing.
pub trait Named {
    /// Names it and passes it on: `ui.add(DragValue…).named("Position X").changed()`.
    fn named(self, name: &str) -> Self;
    /// Names it where nothing else is done with it (a menu's response).
    fn set_name(&self, name: &str);
}

impl Named for Response {
    fn named(self, name: &str) -> Response {
        self.set_name(name);
        self
    }

    fn set_name(&self, name: &str) {
        self.ctx.accesskit_node_builder(self.id, |node| node.set_label(name));
    }
}

/// The keyboard focus ring of a custom-painted control: the same 2 px accent ring egui draws
/// on its own focused widgets (`theme::apply`, `widgets.active`). Drawn just outside the control,
/// so it shows on an active (accent-filled) tool too.
pub fn focus_ring(ui: &egui::Ui, resp: &Response) {
    if resp.has_focus() {
        ui.painter().rect_stroke(resp.rect, crate::theme::T.rc(), Stroke::new(2.0, crate::theme::T.accent), StrokeKind::Outside);
    }
}

/// A custom-painted control: its role and name (and, for toggles, whether it is on) for
/// assistive technology, and its focus ring.
/// A control allocated with `Sense::hover()` because it can't act now (Align with nothing
/// selected) reads as unavailable.
pub fn control(ui: &egui::Ui, resp: &Response, typ: WidgetType, name: &str, on: Option<bool>) {
    let enabled = resp.enabled() && (resp.sense.senses_click() || resp.sense.senses_drag());
    resp.widget_info(|| match on {
        Some(v) => WidgetInfo::selected(typ, enabled, v, name),
        None => WidgetInfo::labeled(typ, enabled, name),
    });
    focus_ring(ui, resp);
}

/// A drawing surface (the stage, the Motion Editor's graph): a canvas with a name, and its focus
/// ring.
pub fn surface(ui: &egui::Ui, resp: &Response, name: &str) {
    ui.ctx().accesskit_node_builder(resp.id, |node| {
        node.set_role(egui::accesskit::Role::Canvas);
        node.set_label(name);
    });
    // A surface fills its panel: the ring goes inside, where the panel doesn't clip it.
    if resp.has_focus() {
        ui.painter().rect_stroke(resp.rect.shrink(1.0), crate::theme::SQUARE, Stroke::new(2.0, crate::theme::T.accent), StrokeKind::Inside);
    }
}

/// Clicks and drags but no Tab stop: pointer surfaces whose keyboard route is elsewhere (the
/// frame ruler and grid have `,` `.` F5 F6; the timeline scrollbar has Shift+wheel). Screen
/// readers still find and can click them by name.
pub fn pointer_only() -> egui::Sense {
    egui::Sense::click_and_drag() - egui::Sense::FOCUSABLE
}

/// Names the resize handle egui puts on a panel (`Panel::…(id)`), which it leaves anonymous.
pub fn name_splitter(ctx: &egui::Context, panel: &str, name: &str) {
    ctx.accesskit_node_builder(Id::new(panel).with("__resize"), |node| {
        node.set_role(egui::accesskit::Role::Splitter);
        node.set_label(name);
    });
}

/// Names the title bar of a window that `shown` (the result of `Window::show`) put on screen.
/// egui makes the bar a focusable drag area with no name.
pub fn name_title_bar<R>(shown: &Option<egui::InnerResponse<R>>, title: &str) {
    if let Some(w) = shown {
        // The window's area id is its layer's (the response is the area's "move" widget).
        w.response.ctx.accesskit_node_builder(w.response.layer_id.id.with("__title_click"), |node| {
            node.set_role(egui::accesskit::Role::TitleBar);
            node.set_label(title);
        });
    }
}

/// How a [`slider`]'s number reads.
#[derive(Clone, Copy, Default)]
pub struct SliderLook {
    pub suffix: &'static str,
    pub max_decimals: Option<usize>,
    pub step: Option<f64>,
    pub logarithmic: bool,
}

/// A slider and its number field, both named. (egui's own number field beside a slider is
/// anonymous unless the slider shows a text label, which our form rows already have on the
/// left.) Returns both as one response: `changed`, `drag_stopped` and the rest cover either.
pub fn slider<N: egui::emath::Numeric>(ui: &mut egui::Ui, value: &mut N, range: std::ops::RangeInclusive<N>, name: &str, look: SliderLook) -> Response {
    let mut s = egui::Slider::new(&mut *value, range.clone()).show_value(false).logarithmic(look.logarithmic);
    if let Some(step) = look.step {
        s = s.step_by(step);
    }
    let bar = ui.add(s).named(name);
    let (lo, hi) = (range.start().to_f64(), range.end().to_f64());
    let speed = look.step.unwrap_or(((hi - lo) / 200.0).abs().max(f64::EPSILON));
    let mut field = egui::DragValue::new(value).range(range).suffix(look.suffix).speed(speed);
    if let Some(d) = look.max_decimals {
        field = field.max_decimals(d);
    }
    let num = ui.add(field).named(name);
    bar.union(num)
}

/// A name with its shortcut, as tooltips and screen readers give it: "Pen tool (P)".
pub fn with_key(name: &str, key: Option<&str>) -> String {
    match key {
        Some(k) if !k.is_empty() => format!("{name} ({})", crate::platform::shortcut_label(k)),
        _ => name.to_string(),
    }
}

// ----- panels and the keyboard -------------------------------------------------------------------

/// The panels Ctrl+F6 moves keyboard focus between, in that order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    Stage,
    Timeline,
    Properties,
    Library,
    Tools,
}

impl Region {
    pub const CYCLE: [Region; 5] = [Region::Stage, Region::Timeline, Region::Properties, Region::Library, Region::Tools];

    /// English (a key): translate where it is shown with `crate::i18n::t`.
    pub fn label(self) -> &'static str {
        match self {
            Region::Stage => tr_noop!("Stage"),
            Region::Timeline => tr_noop!("Timeline"),
            Region::Properties => tr_noop!("Properties"),
            Region::Library => tr_noop!("Library"),
            Region::Tools => tr_noop!("Tools"),
        }
    }

    /// The stage and the timeline are where the work is: while their anchor has focus every
    /// shortcut, Enter (play) and the arrows (nudge) work as with nothing focused. The other
    /// anchors are ordinary controls (a section header, the active tool).
    pub fn is_surface(self) -> bool {
        matches!(self, Region::Stage | Region::Timeline)
    }

    fn index(self) -> usize {
        Region::CYCLE.iter().position(|r| *r == self).unwrap_or(0)
    }

    /// The next panel (`step` 1) or the previous one (`step` -1), wrapping around.
    pub fn step(from: Option<Region>, forward: bool) -> Region {
        let n = Region::CYCLE.len();
        let i = match (from, forward) {
            (None, true) => 0,
            (None, false) => n - 1,
            (Some(r), true) => (r.index() + 1) % n,
            (Some(r), false) => (r.index() + n - 1) % n,
        };
        Region::CYCLE.get(i).copied().unwrap_or(Region::Stage)
    }

    fn key(self) -> Id {
        Id::new(("a11y_region", self.index()))
    }
}

/// Where a panel is this frame and the widget that takes focus when Ctrl+F6 lands on it.
#[derive(Clone, Copy, Debug)]
struct Anchor {
    id: Id,
    rect: Rect,
    /// The pass that drew it: a panel that stopped drawing (the timeline while the Motion
    /// Editor shows) has no anchor.
    pass: u64,
}

/// Registers `resp` as the anchor of `region`, which covers `area` on screen. While a surface's
/// anchor has focus the arrow keys stay with the app (nudge, as on an unfocused stage) instead
/// of moving focus to the neighbouring widget.
pub fn anchor(ui: &egui::Ui, region: Region, area: Rect, resp: &Response) {
    let pass = ui.ctx().cumulative_pass_nr();
    ui.ctx().data_mut(|d| d.insert_temp(region.key(), Anchor { id: resp.id, rect: area, pass }));
    if region.is_surface() && resp.has_focus() {
        let filter = egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() };
        ui.memory_mut(|m| m.set_focus_lock_filter(resp.id, filter));
    }
}

/// The anchor drawn this pass or the one before (shortcuts run before the panels draw).
fn anchor_of(ctx: &egui::Context, region: Region) -> Option<Anchor> {
    let now = ctx.cumulative_pass_nr();
    ctx.data(|d| d.get_temp::<Anchor>(region.key())).filter(|a| a.pass + 1 >= now)
}

/// The panel holding the focused widget, if any.
pub fn focused_region(ctx: &egui::Context) -> Option<Region> {
    let id = ctx.memory(|m| m.focused())?;
    if let Some(r) = Region::CYCLE.into_iter().find(|r| anchor_of(ctx, *r).is_some_and(|a| a.id == id)) {
        return Some(r);
    }
    let at = ctx.read_response(id)?.rect.center();
    Region::CYCLE.into_iter().find(|r| anchor_of(ctx, *r).is_some_and(|a| a.rect.contains(at)))
}

/// Moves keyboard focus to the next or previous panel's anchor. Returns the panel, or `None`
/// when no panel has drawn yet.
pub fn cycle_focus(ctx: &egui::Context, from: Option<Region>, forward: bool) -> Option<Region> {
    let mut region = Region::step(focused_region(ctx).or(from), forward);
    // A panel that isn't on screen (the Motion Editor in place of the timeline) is skipped.
    for _ in 0..Region::CYCLE.len() {
        if let Some(a) = anchor_of(ctx, region) {
            ctx.memory_mut(|m| m.request_focus(a.id));
            return Some(region);
        }
        region = Region::step(Some(region), forward);
    }
    None
}

/// Who the keyboard belongs to this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keyboard {
    /// Nothing has focus, or the stage or timeline does: every shortcut works.
    App,
    /// A button, checkbox or menu has focus: shortcuts work, but Enter and Space press the
    /// control and the arrows move between controls.
    Control,
    /// A text field: it gets every key.
    Text,
}

pub fn keyboard(ctx: &egui::Context) -> Keyboard {
    let Some(id) = ctx.memory(|m| m.focused()) else { return Keyboard::App };
    if ctx.text_edit_focused() {
        return Keyboard::Text;
    }
    if Region::CYCLE.into_iter().any(|r| r.is_surface() && anchor_of(ctx, r).is_some_and(|a| a.id == id)) {
        return Keyboard::App;
    }
    Keyboard::Control
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, Tool};
    use egui::accesskit::{Action, Node, Role, TreeUpdate};

    #[test]
    fn region_cycle_wraps_both_ways() {
        assert_eq!(Region::step(None, true), Region::Stage);
        assert_eq!(Region::step(None, false), Region::Tools);
        assert_eq!(Region::step(Some(Region::Tools), true), Region::Stage);
        assert_eq!(Region::step(Some(Region::Stage), false), Region::Tools);
        assert_eq!(Region::step(Some(Region::Timeline), true), Region::Properties);
    }

    /// Scroll bars inside egui's scroll areas: screen readers scroll by moving focus, and egui
    /// gives them no name to set (a known gap, docs/accessibility.md).
    const PLUMBING: [Role; 1] = [Role::ScrollBar];

    fn named(n: &Node) -> bool {
        let some = |s: Option<&str>| s.is_some_and(|l| !l.trim().is_empty());
        // A text label (selectable, so it takes clicks) is named by its text.
        some(n.label()) || !n.labelled_by().is_empty() || (n.role() == Role::Label && some(n.value()))
    }

    /// Interactive nodes (focusable or clickable) without a role or a name.
    fn audit(update: &TreeUpdate) -> Vec<String> {
        let mut out = Vec::new();
        for (_, n) in &update.nodes {
            let interactive = n.supports_action(Action::Focus) || n.supports_action(Action::Click);
            if !interactive || PLUMBING.contains(&n.role()) {
                continue;
            }
            let b = n.bounds().unwrap_or_default();
            let at = format!("{:?} at ({:.0}, {:.0}) {:.0}×{:.0}{}", n.role(), b.x0, b.y0, b.x1 - b.x0, b.y1 - b.y0, n.value().map(|v| format!(" value “{v}”")).unwrap_or_default());
            if n.role() == Role::Unknown {
                out.push(format!("no role: {at}"));
            } else if !named(n) {
                out.push(format!("no name: {at}"));
            }
        }
        out
    }

    /// One headless frame. The textures it made have nowhere to go: drop them.
    fn frame(ctx: &egui::Context, app: &mut App, input: egui::RawInput) -> egui::FullOutput {
        let mut out = ctx.run_ui(input, |ui| app.frame(ui));
        out.textures_delta.clear();
        out
    }

    fn input(scale: f32) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 940.0) / scale)),
            ..Default::default()
        }
    }

    /// The app on a headless context with AccessKit on, after `setup` (with a few frames for
    /// layout to settle). Returns the last frame's tree.
    fn tree(setup: impl FnOnce(&mut App, &egui::Context)) -> (TreeUpdate, App, egui::Context) {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let session = anicraft_engine::Session::new(anicraft_engine::doc::sample::demo());
        // Screenshot mode: default preferences, no autosave, no plug-in folder.
        let mut app = App::with_context(&ctx, session, Some(std::path::PathBuf::from("unused.png")));
        setup(&mut app, &ctx);
        let scale = app.prefs.ui_scale;
        let mut last = None;
        for _ in 0..3 {
            let out = frame(&ctx, &mut app, input(scale));
            last = out.platform_output.accesskit_update;
        }
        (last.unwrap_or_else(|| TreeUpdate { nodes: Vec::new(), tree: None, tree_id: egui::accesskit::TreeId::ROOT, focus: egui::accesskit::NodeId(0) }), app, ctx)
    }

    fn check(name: &str, setup: impl FnOnce(&mut App, &egui::Context)) -> Vec<String> {
        let (t, _, _) = tree(setup);
        assert!(t.nodes.len() > 20, "{name}: the tree is nearly empty ({} nodes); is AccessKit on?", t.nodes.len());
        audit(&t).into_iter().map(|m| format!("[{name}] {m}")).collect()
    }

    #[test]
    fn every_interactive_node_has_a_name_and_a_role() {
        let mut found = Vec::new();
        found.extend(check("default", |_, _| {}));
        for tool in [Tool::Bone, Tool::Rect, Tool::Star, Tool::Text, Tool::Pencil, Tool::Brush, Tool::Eraser, Tool::PaintBucket, Tool::InkBottle, Tool::Eyedropper, Tool::Pen] {
            found.extend(check(&format!("{tool:?} tool"), |a, _| a.tool = tool));
        }
        for panel in ["select", "motion", "motionspan", "motionpath", "motionpoints", "motionedit", "motionpins", "motionpinsspan", "rawsub", "camera", "cameraview", "bones", "sound", "envelope", "prefs", "shortcuts", "palette", "about", "files", "licenses", "confirm", "recovery", "frames", "actions", "script", "scriptproblems", "testscript", "onion", "onionmenu", "editmultiple", "library", "librarymenu", "edittransform", "onionsymbol", "convert", "mixed", "bitmap"] {
            found.extend(check(panel, |a, _| a.open_panel(Some(panel))));
        }
        // The layer depth row shows on artwork layers once the scene has a camera.
        found.extend(check("layer depth", |a, _| {
            a.open_panel(Some("camera"));
            a.run("layer.select", serde_json::json!({"index": 1}));
        }));
        found.extend(check("movie clip with filters", |a, _| {
            a.run("layer.select", serde_json::json!({"index": 0}));
            a.run("edit.select_all", serde_json::json!({}));
            let symbol = a.run("modify.convert_to_symbol", serde_json::json!({"name": "Clip"})).and_then(|v| v["symbol"].as_u64());
            assert!(symbol.is_some(), "the sample's top layer has artwork to convert");
            a.run("symbol.set_kind", serde_json::json!({"symbol": symbol, "kind": "movie_clip"}));
            let filters: Vec<_> = anicraft_engine::doc::Filter::KINDS.iter().map(|(k, _)| serde_json::json!({"type": k})).collect();
            assert!(a.run("element.set_filters", serde_json::json!({"filters": filters})).is_some());
        }));
        found.extend(check("high contrast at 200%", |a, ctx| {
            let mut p = a.prefs.clone();
            p.theme = crate::theme::Palette::HighContrast;
            p.ui_scale = 2.0;
            a.set_prefs(ctx, p);
        }));
        crate::theme::apply_palette(&egui::Context::default(), crate::theme::Palette::Graphite);
        assert!(found.is_empty(), "{} interactive nodes a screen reader can't name:\n{}", found.len(), found.join("\n"));
    }

    fn press(key: egui::Key, modifiers: egui::Modifiers) -> egui::RawInput {
        let mut i = input(1.0);
        i.events.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
        i
    }

    #[test]
    fn a_focused_control_keeps_enter_but_shortcuts_still_work() {
        let (_, mut app, ctx) = tree(|_, _| {});
        // Stage, Timeline, then the Properties header: a control, not a surface.
        for _ in 0..3 {
            frame(&ctx, &mut app, press(egui::Key::F6, egui::Modifiers::COMMAND));
            frame(&ctx, &mut app, input(1.0));
        }
        assert_eq!(focused_region(&ctx), Some(Region::Properties));
        assert_eq!(keyboard(&ctx), Keyboard::Control);
        // A tool key still works with the header focused...
        frame(&ctx, &mut app, press(egui::Key::R, egui::Modifiers::NONE));
        assert_eq!(app.tool, Tool::Rect);
        // ...but Enter presses the header (folds the section) instead of playing.
        frame(&ctx, &mut app, press(egui::Key::Enter, egui::Modifiers::NONE));
        assert!(!app.playing, "Enter went to the focused header, not to play");
        // On the stage, Enter plays as usual.
        for _ in 0..3 {
            frame(&ctx, &mut app, press(egui::Key::F6, egui::Modifiers::COMMAND));
            frame(&ctx, &mut app, input(1.0));
        }
        assert_eq!(focused_region(&ctx), Some(Region::Stage));
        frame(&ctx, &mut app, press(egui::Key::Enter, egui::Modifiers::NONE));
        assert!(app.playing, "Enter on the focused stage plays");
    }

    #[test]
    fn ctrl_f6_moves_focus_through_the_panels_and_back() {
        let (_, mut app, ctx) = tree(|_, _| {});
        let mut seen = Vec::new();
        for _ in 0..Region::CYCLE.len() {
            let mut i = input(1.0);
            i.events.push(egui::Event::Key { key: egui::Key::F6, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND });
            frame(&ctx, &mut app, i);
            frame(&ctx, &mut app, input(1.0));
            let at = focused_region(&ctx);
            seen.push(at);
            // The stage and timeline leave the keyboard to the app; a header or tool is a control.
            let expect = if at.is_some_and(Region::is_surface) { Keyboard::App } else { Keyboard::Control };
            assert_eq!(keyboard(&ctx), expect, "{at:?}");
        }
        assert_eq!(seen, Region::CYCLE.map(Some).to_vec());
        let mut i = input(1.0);
        i.events.push(egui::Event::Key { key: egui::Key::F6, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND | egui::Modifiers::SHIFT });
        frame(&ctx, &mut app, i);
        frame(&ctx, &mut app, input(1.0));
        assert_eq!(focused_region(&ctx), Some(Region::Library), "Ctrl+Shift+F6 goes back");
    }
}
