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

/// Names the resize handle egui puts on a panel (`Panel::…(id)`, its `id.with("__resize")`),
/// which it leaves anonymous.
pub fn name_resize(ctx: &egui::Context, id: Id, name: &str) {
    ctx.accesskit_node_builder(id, |node| {
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

/// Names the title bar of the window `ui` is drawn in (a window egui made itself, such as its
/// stand-in for a viewport where there is no window system).
pub fn name_own_title_bar(ui: &egui::Ui, title: &str) {
    ui.ctx().accesskit_node_builder(ui.layer_id().id.with("__title_click"), |node| {
        node.set_role(egui::accesskit::Role::TitleBar);
        node.set_label(title);
    });
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

/// The panels Ctrl+F6 moves keyboard focus between: the stage, each docked panel (ADR 0075),
/// the tools.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Region {
    Stage,
    Tools,
    Panel(crate::dock::Panel),
}

#[allow(non_upper_case_globals)]
impl Region {
    pub const Timeline: Region = Region::Panel(crate::dock::Panel::Timeline);
    #[cfg(test)]
    pub const Properties: Region = Region::Panel(crate::dock::Panel::Properties);
    #[cfg(test)]
    pub const Library: Region = Region::Panel(crate::dock::Panel::Library);

    /// English (a key): translate where it is shown with `crate::i18n::t`.
    pub fn label(self) -> &'static str {
        match self {
            Region::Stage => tr_noop!("Stage"),
            Region::Tools => tr_noop!("Tools"),
            Region::Panel(p) => p.label(),
        }
    }

    /// The stage and the timeline are where the work is: while their anchor has focus every
    /// shortcut, Enter (play) and the arrows (nudge) work as with nothing focused. The other
    /// anchors are ordinary controls (a panel's tab, the active tool).
    pub fn is_surface(self) -> bool {
        matches!(self, Region::Stage) || self == Region::Timeline
    }

    /// The next panel in `order` (`forward`) or the previous one, wrapping around.
    pub fn step(order: &[Region], from: Option<Region>, forward: bool) -> Region {
        let n = order.len().max(1);
        let at = from.and_then(|r| order.iter().position(|o| *o == r));
        let i = match (at, forward) {
            (None, true) => 0,
            (None, false) => n - 1,
            (Some(i), true) => (i + 1) % n,
            (Some(i), false) => (i + n - 1) % n,
        };
        order.get(i).copied().unwrap_or(Region::Stage)
    }

    fn key(self) -> Id {
        Id::new(("a11y_region", self))
    }
}

/// Sets this frame's Ctrl+F6 order (the dock's, ADR 0075).
pub fn set_order(ctx: &egui::Context, order: Vec<Region>) {
    ctx.data_mut(|d| d.insert_temp(Id::new("a11y_region_order"), order));
}

/// The Ctrl+F6 order: the stage, the panels as docked, the tools.
pub fn order(ctx: &egui::Context) -> Vec<Region> {
    ctx.data(|d| d.get_temp::<Vec<Region>>(Id::new("a11y_region_order"))).unwrap_or_else(|| vec![Region::Stage, Region::Tools])
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
    let order = order(ctx);
    if let Some(r) = order.iter().copied().find(|r| anchor_of(ctx, *r).is_some_and(|a| a.id == id)) {
        return Some(r);
    }
    let at = ctx.read_response(id)?.rect.center();
    order.into_iter().find(|r| anchor_of(ctx, *r).is_some_and(|a| a.rect.contains(at)))
}

/// Moves keyboard focus to the next or previous panel's anchor. Returns the panel, or `None`
/// when no panel has drawn yet.
pub fn cycle_focus(ctx: &egui::Context, from: Option<Region>, forward: bool) -> Option<Region> {
    let order = order(ctx);
    let mut region = Region::step(&order, focused_region(ctx).or(from), forward);
    // A panel that isn't on screen (a tab behind another, a folded group) is skipped.
    for _ in 0..order.len() {
        if let Some(a) = anchor_of(ctx, region) {
            ctx.memory_mut(|m| m.request_focus(a.id));
            return Some(region);
        }
        region = Region::step(&order, Some(region), forward);
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
    if [Region::Stage, Region::Timeline].into_iter().any(|r| anchor_of(ctx, r).is_some_and(|a| a.id == id)) {
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
        let o = [Region::Stage, Region::Timeline, Region::Properties, Region::Library, Region::Tools];
        assert_eq!(Region::step(&o, None, true), Region::Stage);
        assert_eq!(Region::step(&o, None, false), Region::Tools);
        assert_eq!(Region::step(&o, Some(Region::Tools), true), Region::Stage);
        assert_eq!(Region::step(&o, Some(Region::Stage), false), Region::Tools);
        assert_eq!(Region::step(&o, Some(Region::Timeline), true), Region::Properties);
        assert_eq!(Region::step(&[], None, true), Region::Stage);
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

    /// Test Movie exposes its text fields (ADR 0066): input fields as text inputs named by their
    /// instance names, with what was typed as their value; dynamic text as a label.
    #[test]
    fn test_movie_text_fields_are_text_inputs() {
        let (t, _, _) = tree(|a, _| a.open_panel(Some("testtext")));
        let find = |label: &str| t.nodes.iter().map(|(_, n)| n).find(|n| n.label() == Some(label)).unwrap_or_else(|| panic!("no node named {label}"));
        let name = find("name_txt");
        assert_eq!((name.role(), name.value()), (Role::TextInput, Some("Ada Lovelace")));
        assert_eq!(find("notes_txt").role(), Role::MultilineTextInput);
        let score = find("score_txt");
        assert_eq!((score.role(), score.value()), (Role::Label, Some("Hello, Ada Lovelace!")));
    }

    /// Painted weights (ADR 0090) are reachable without a pointer: the brush and a point's weight
    /// are named fields, the weight a number to type.
    #[test]
    fn shape_weights_are_named_numeric_fields() {
        let (t, _, _) = tree(|a, _| a.open_panel(Some("shapeweights")));
        for n in ["Paint Weights", "Weight brush size", "Weight brush strength", "Focused point", "Bone weight at the point"] {
            assert!(t.nodes.iter().any(|(_, x)| x.label() == Some(n)), "no node named {n}");
        }
        let w = t.nodes.iter().map(|(_, x)| x).find(|x| x.label() == Some("Bone weight at the point")).expect("the weight field");
        assert!(matches!(w.role(), Role::SpinButton | Role::Slider | Role::TextInput), "{:?}", w.role());
    }

    /// The layer footer (after #265 cut "Delete" short): icon buttons only, each named, side
    /// by side inside the layer column at the default window size, none clipped or overlapping.
    #[test]
    fn the_layer_footer_buttons_fit_and_are_named() {
        let (t, _, _) = tree(|_, _| {});
        let names = ["New layer", "Delete layer", "New folder above the active layer (move layers in under it with Up and Down)", "Move layer up", "Move layer down"];
        let rects: Vec<egui::accesskit::Rect> = names
            .iter()
            .map(|n| {
                let node = t.nodes.iter().map(|(_, x)| x).find(|x| x.label() == Some(n)).unwrap_or_else(|| panic!("no footer button named {n}"));
                assert_eq!(node.role(), Role::Button, "{n}");
                node.bounds().unwrap_or_else(|| panic!("{n} has no bounds"))
            })
            .collect();
        for (i, a) in rects.iter().enumerate() {
            assert!(a.width() >= 20.0 && a.height() >= 20.0, "{} is too small: {a:?}", names[i]);
            for (j, b) in rects.iter().enumerate().skip(i + 1) {
                let apart = a.x1 <= b.x0 + 0.5 || b.x1 <= a.x0 + 0.5 || a.y1 <= b.y0 + 0.5 || b.y1 <= a.y0 + 0.5;
                assert!(apart, "{} overlaps {}", names[i], names[j]);
            }
        }
        // All on one row, New layer and Delete first.
        assert!(rects.iter().all(|r| (r.y0 - rects[0].y0).abs() < 1.0));
        assert!(rects[0].x0 < rects[1].x0 && rects[1].x1 < rects[2].x0);
    }

    #[test]
    fn every_interactive_node_has_a_name_and_a_role() {
        let mut found = Vec::new();
        found.extend(check("default", |_, _| {}));
        for tool in [Tool::Bone, Tool::Bind, Tool::Warp, Tool::Rect, Tool::Star, Tool::Text, Tool::Pencil, Tool::Brush, Tool::PaintBrush, Tool::Eraser, Tool::PaintBucket, Tool::InkBottle, Tool::Eyedropper, Tool::Pen] {
            found.extend(check(&format!("{tool:?} tool"), |a, _| a.tool = tool));
        }
        for panel in ["select", "motion", "motionspan", "motionpath", "motionpoints", "motionedit", "motionpins", "motionpinsspan", "rawsub", "camera", "cameraview", "bones", "springbones", "shapebones", "shapeweights", "warp", "sound", "envelope", "prefs", "shortcuts", "palette", "about", "files", "licenses", "confirm", "recovery", "frames", "actions", "script", "scriptproblems", "testscript", "onion", "onionmenu", "editmultiple", "library", "librarymenu", "edittransform", "onionsymbol", "layerprops", "frameview", "layercolour", "timelinepolish", "convert", "mixed", "bitmap", "nineslice", "nineslicedit", "find", "explorer", "publish", "publishreport", "publishanim", "start", "drop", "droplibrary", "testmovie", "testmovieinside", "templates", "savetemplate", "droptimeline", "textfields", "testtext", "softmask", "pathtext", "gradient", "gradientbucket", "swatches", "swatchpanel", "brushlibrary", "brushcurve", "paintbrush"] {
            found.extend(check(panel, |a, _| a.open_panel(Some(panel))));
        }
        // Every panel in front where it docks, and floating (ADR 0075); the other workspaces.
        for p in crate::dock::Panel::ALL {
            found.extend(check(&format!("{p:?} panel"), |a, _| a.show_panel(p)));
            found.extend(check(&format!("{p:?} floating"), |a, _| a.dock.move_to(p, crate::dock::Target::Float(None))));
        }
        found.extend(check("floating panel window", |a, ctx| {
            ctx.set_embed_viewports(false);
            a.dock.move_to(crate::dock::Panel::Colour, crate::dock::Target::Float(None));
        }));
        for w in ["animator", "designer"] {
            found.extend(check(w, |a, _| a.switch_workspace(w)));
        }
        // Dockable panels, second slice (ADR 0087).
        for state in ["dockicons", "floattabs", "docknarrow", "workspaces"] {
            found.extend(check(state, |a, _| a.open_panel(Some(state))));
        }
        found.extend(check("tools above the stage", |a, _| a.dock.move_to(crate::dock::Panel::Tools, crate::dock::Target::Bar(crate::dock::Side::Top))));
        found.extend(check("tools as a tab", |a, _| a.dock.move_to(crate::dock::Panel::Tools, crate::dock::Target::Stack(crate::dock::Panel::Library))));
        found.extend(check("timeline above the stage", |a, _| a.dock.move_to(crate::dock::Panel::Timeline, crate::dock::Target::Split { side: crate::dock::Side::Top, before: None })));
        // Test Movie in a window of its own (ADR 0062): its controls, named in that window.
        found.extend(check("test movie window", |a, ctx| {
            ctx.set_embed_viewports(false);
            a.open_panel(Some("testmovie"));
        }));
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
        // Each colour effect style's controls (ADR 0056), and the Motion Editor's colour rows.
        for style in ["brightness", "tint", "advanced", "alpha"] {
            found.extend(check(&format!("colour effect {style}"), |a, _| {
                a.run("layer.select", serde_json::json!({"index": 0}));
                a.run("edit.select_all", serde_json::json!({}));
                assert!(a.run("element.set_color_effect", serde_json::json!({"style": style})).is_some());
            }));
        }
        found.extend(check("motion editor colour rows", |a, _| {
            a.open_panel(Some("motionedit"));
            a.me_styles = vec![anicraft_engine::doc::ColorStyle::Tint, anicraft_engine::doc::ColorStyle::Advanced, anicraft_engine::doc::ColorStyle::Brightness];
        }));
        found.extend(check("light", |a, ctx| {
            let mut p = a.prefs.clone();
            p.theme = crate::theme::Palette::Light;
            p.theme_follows_system = Some(false);
            a.set_prefs(ctx, p);
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
        focus_stage(&ctx, &mut app);
        assert_eq!(focused_region(&ctx), Some(Region::Stage));
        frame(&ctx, &mut app, press(egui::Key::Enter, egui::Modifiers::NONE));
        assert!(app.playing, "Enter on the focused stage plays");
    }

    /// Presses `key` and lets a frame pass for focus to settle. Returns the last frame's tree.
    fn key(ctx: &egui::Context, app: &mut App, key: egui::Key, modifiers: egui::Modifiers) -> Option<TreeUpdate> {
        frame(ctx, app, press(key, modifiers));
        frame(ctx, app, input(1.0)).platform_output.accesskit_update
    }

    fn focus_stage(ctx: &egui::Context, app: &mut App) {
        while focused_region(ctx) != Some(Region::Stage) {
            key(ctx, app, egui::Key::F6, egui::Modifiers::COMMAND);
        }
    }

    fn node(t: &TreeUpdate, id: egui::Id) -> Option<&Node> {
        t.nodes.iter().find(|(n, _)| *n == id.accesskit_id()).map(|(_, n)| n)
    }

    #[test]
    fn the_stage_lists_its_elements_and_selects_them_from_the_keyboard() {
        let (_, mut app, ctx) = tree(|_, _| {});
        // Unfocused, the stage has no element nodes: the tree stays small.
        assert!(app.stage_nav.focused(&ctx).is_none());
        focus_stage(&ctx, &mut app);
        app.run("edit.deselect", serde_json::json!({}));
        let (items, total) = crate::stage_a11y::items(&app);
        assert!(items.len() >= 2 && total == items.len(), "the sample frame has a few elements: {items:?}");
        // Tab goes into the list, to the first element.
        let t = key(&ctx, &mut app, egui::Key::Tab, egui::Modifiers::NONE).unwrap_or_else(|| panic!("no tree"));
        let first = app.stage_nav.focused(&ctx).unwrap_or_else(|| panic!("Tab on the stage focuses an element"));
        assert_eq!(first, (items[0].layer, items[0].element));
        let n = node(&t, crate::stage_a11y::node_id(first.0, first.1)).unwrap_or_else(|| panic!("the element has a node"));
        assert_eq!(n.role(), Role::ListBoxOption);
        assert_eq!(n.label(), Some(items[0].name.as_str()));
        assert_eq!(n.is_selected(), Some(false));
        assert!(n.bounds().is_some_and(|b| b.x1 > b.x0 || b.y1 > b.y0) || items[0].bounds.is_empty(), "bounds on screen");
        assert_eq!(t.focus, crate::stage_a11y::node_id(first.0, first.1).accesskit_id(), "the reader's focus is on it");
        // The list box is the canvas's child, and its options are the list's.
        let list = t.nodes.iter().find(|(_, n)| n.role() == Role::ListBox && n.children().contains(&crate::stage_a11y::node_id(first.0, first.1).accesskit_id()));
        assert!(list.is_some(), "the options sit in a list box");
        let canvas = t.nodes.iter().find(|(_, n)| n.role() == Role::Canvas && n.label().is_some_and(|l| l.starts_with("Stage"))).map(|(_, n)| n);
        assert!(canvas.is_some_and(|c| list.is_some_and(|(id, _)| c.children().contains(id))), "the list box is the stage's child");
        // Space selects it alone; the arrows move on without nudging; Shift+Space adds the next.
        key(&ctx, &mut app, egui::Key::Space, egui::Modifiers::NONE);
        assert!(app.s.selection.contains(first.0, first.1) && app.s.selection.len() == 1);
        let before = crate::stage_a11y::items(&app).0.iter().map(|i| i.bounds).collect::<Vec<_>>();
        key(&ctx, &mut app, egui::Key::ArrowRight, egui::Modifiers::NONE);
        let second = app.stage_nav.focused(&ctx).unwrap_or_else(|| panic!("an element has focus"));
        assert_eq!(second, (items[1].layer, items[1].element));
        assert_eq!(crate::stage_a11y::items(&app).0.iter().map(|i| i.bounds).collect::<Vec<_>>(), before, "the arrows moved focus, not the artwork");
        key(&ctx, &mut app, egui::Key::Space, egui::Modifiers::SHIFT);
        assert!(app.s.selection.contains(second.0, second.1) && app.s.selection.contains(first.0, first.1));
        let t = key(&ctx, &mut app, egui::Key::Space, egui::Modifiers::SHIFT).unwrap_or_else(|| panic!("no tree"));
        assert!(!app.s.selection.contains(second.0, second.1) && app.s.selection.len() == 1, "Shift+Space again takes it out");
        assert_eq!(node(&t, crate::stage_a11y::node_id(first.0, first.1)).and_then(Node::is_selected), Some(true));
        // Left from the first wraps to the last; Enter selects it alone.
        key(&ctx, &mut app, egui::Key::ArrowLeft, egui::Modifiers::NONE);
        key(&ctx, &mut app, egui::Key::ArrowLeft, egui::Modifiers::NONE);
        let last = items.last().map(|i| (i.layer, i.element));
        assert_eq!(app.stage_nav.focused(&ctx), last);
        key(&ctx, &mut app, egui::Key::Enter, egui::Modifiers::NONE);
        assert!(!app.playing, "Enter on an element selects, it doesn't play");
        // The sample's sky is on a locked layer: listed (unavailable), never selected.
        assert!(items.last().is_some_and(|i| i.locked));
        assert!(app.s.selection.contains(first.0, first.1) && app.s.selection.len() == 1, "a locked element isn't selected");
        key(&ctx, &mut app, egui::Key::ArrowLeft, egui::Modifiers::NONE);
        key(&ctx, &mut app, egui::Key::Enter, egui::Modifiers::NONE);
        let before_last = items.get(items.len() - 2).map(|i| (i.layer, i.element));
        assert!(before_last.is_some_and(|(l, e)| app.s.selection.contains(l, e)) && app.s.selection.len() == 1, "Enter selects it alone");
        // Esc goes back to the stage itself.
        key(&ctx, &mut app, egui::Key::Escape, egui::Modifiers::NONE);
        assert!(app.stage_nav.focused(&ctx).is_none());
        assert_eq!(focused_region(&ctx), Some(Region::Stage));
        assert_eq!(keyboard(&ctx), Keyboard::App, "the stage has the keyboard again: arrows nudge");
    }

    #[test]
    fn stage_elements_and_timeline_frames_pass_the_audit() {
        let (_, mut app, ctx) = tree(|_, _| {});
        focus_stage(&ctx, &mut app);
        let t = key(&ctx, &mut app, egui::Key::Tab, egui::Modifiers::NONE).unwrap_or_else(|| panic!("no tree"));
        assert!(t.nodes.iter().filter(|(_, n)| n.role() == Role::ListBoxOption).count() >= 2);
        let found = audit(&t);
        assert!(found.is_empty(), "{}", found.join("\n"));
    }

    #[test]
    fn the_focused_timeline_moves_and_announces_the_playhead_and_layer_and_lists_its_spans() {
        let (_, mut app, ctx) = tree(|_, _| {});
        while focused_region(&ctx) != Some(Region::Timeline) {
            key(&ctx, &mut app, egui::Key::F6, egui::Modifiers::COMMAND);
        }
        app.run("layer.select", serde_json::json!({"index": 0}));
        app.tl_refocus = true;
        frame(&ctx, &mut app, input(1.0));
        frame(&ctx, &mut app, input(1.0));
        let (frame0, layer0) = (app.s.frame, app.s.layer);
        let t = key(&ctx, &mut app, egui::Key::ArrowRight, egui::Modifiers::NONE).unwrap_or_else(|| panic!("no tree"));
        assert_eq!(app.s.frame, frame0 + 1, "Right is the next frame");
        let said = app.status.as_ref().map(|s| s.text.clone()).unwrap_or_default();
        assert!(said.starts_with(&format!("Frame {}", frame0 + 2)), "announced: {said}");
        // The active layer's spans are list items under its frame grid, the playhead's selected.
        let items: Vec<&Node> = t.nodes.iter().filter(|(_, n)| n.role() == Role::ListItem).map(|(_, n)| n).collect();
        assert!(!items.is_empty(), "the active layer's spans are listed");
        assert!(items.iter().all(|n| n.label().is_some_and(|l| l.starts_with("Frame"))), "{:?}", items.iter().map(|n| n.label()).collect::<Vec<_>>());
        assert_eq!(items.iter().filter(|n| n.is_selected() == Some(true)).count(), 1, "one span holds the playhead");
        let grid = t.nodes.iter().find(|(_, n)| n.label().is_some_and(|l| l.starts_with("Frames of")) && !n.children().is_empty());
        assert!(grid.is_some(), "the list sits under the active layer's frame grid");
        assert!(audit(&t).is_empty(), "{}", audit(&t).join("\n"));
        // Left goes back; Down moves to the next layer and keeps the keyboard on the timeline.
        key(&ctx, &mut app, egui::Key::ArrowLeft, egui::Modifiers::NONE);
        assert_eq!(app.s.frame, frame0);
        key(&ctx, &mut app, egui::Key::ArrowDown, egui::Modifiers::NONE);
        assert_ne!(app.s.layer, layer0, "Down is the layer below");
        assert_eq!(app.s.timeline().layers.iter().position(|l| l.id == app.s.layer), Some(1));
        assert_eq!((focused_region(&ctx), keyboard(&ctx)), (Some(Region::Timeline), Keyboard::App), "the new row has the keyboard");
        key(&ctx, &mut app, egui::Key::ArrowUp, egui::Modifiers::NONE);
        assert_eq!(app.s.layer, layer0);
        let said = app.status.as_ref().map(|s| s.text.clone()).unwrap_or_default();
        let name = app.s.timeline().layer(layer0).map(|l| l.name.clone()).unwrap_or_default();
        assert!(said.contains(&name), "the layer is announced: {said}");
    }

    #[test]
    fn access_keys_take_the_first_free_letter() {
        let keys = crate::menus::access_keys(&["File", "Edit", "View", "Insert", "Modify", "Commands", "Control", "Window", "Help"]);
        let letters: String = keys.iter().map(|k| k.map_or('-', |(_, c)| c)).collect();
        assert_eq!(letters, "FEVIMCOWH", "Control takes its O, as in Animate");
        assert_eq!(crate::menus::access_keys(&["Fichier", "Édition", "Ändern", "日本"]), vec![Some((0, 'F')), Some((1, 'D')), Some((1, 'N')), None]);
    }

    #[test]
    fn the_menu_bar_works_from_the_keyboard() {
        if !crate::menus::MenuKeys::available() {
            return; // macOS keeps its own menu conventions (ADR 0064).
        }
        use egui::{Key, Modifiers as M};
        let (_, mut app, ctx) = tree(|_, _| {});
        let tops = app.menu_keys.top_ids().to_vec();
        assert_eq!(tops.len(), crate::menus::TOP_MENUS.len());
        let focused = |ctx: &egui::Context| ctx.memory(|m| m.focused());
        let open = |ctx: &egui::Context, i: usize| egui::Popup::is_id_open(ctx, tops[i].with("popup"));
        // F10 focuses File; the titles show their access keys.
        let t = key(&ctx, &mut app, Key::F10, M::NONE).unwrap_or_else(|| panic!("no tree"));
        assert_eq!(focused(&ctx), Some(tops[0]));
        assert!(app.menu_keys.show_keys);
        assert_eq!(node(&t, tops[0]).and_then(Node::access_key), Some("Alt+F"));
        // Right: Edit; Down opens it on its first item; Down, Up and Up wrap round.
        key(&ctx, &mut app, Key::ArrowRight, M::NONE);
        assert_eq!(focused(&ctx), Some(tops[1]));
        key(&ctx, &mut app, Key::ArrowDown, M::NONE);
        frame(&ctx, &mut app, input(1.0));
        assert!(open(&ctx, 1), "Down opens Edit");
        let items = app.menu_keys.open_items(&ctx);
        assert!(items.len() > 3, "Edit has items: {}", items.len());
        assert_eq!(focused(&ctx), items.first().copied(), "on its first item");
        key(&ctx, &mut app, Key::ArrowDown, M::NONE);
        assert_eq!(focused(&ctx), items.get(1).copied());
        key(&ctx, &mut app, Key::ArrowUp, M::NONE);
        key(&ctx, &mut app, Key::ArrowUp, M::NONE);
        assert_eq!(focused(&ctx), items.last().copied(), "Up from the first wraps to the last");
        assert_eq!(keyboard(&ctx), Keyboard::Control, "the menu keeps Enter and the arrows");
        // Right on an item: the next menu, open, on its first item.
        key(&ctx, &mut app, Key::ArrowRight, M::NONE);
        frame(&ctx, &mut app, input(1.0));
        assert!(open(&ctx, 2) && !open(&ctx, 1), "Right goes on to View");
        assert_eq!(focused(&ctx), app.menu_keys.open_items(&ctx).first().copied());
        // Esc closes the menu, back on its title; Esc again leaves the bar.
        key(&ctx, &mut app, Key::Escape, M::NONE);
        assert!(!open(&ctx, 2));
        assert_eq!(focused(&ctx), Some(tops[2]));
        key(&ctx, &mut app, Key::Escape, M::NONE);
        assert_eq!(focused(&ctx), None, "nothing had focus before");
        assert!(!app.menu_keys.show_keys);
        // Alt pressed and let go alone also focuses the bar (and again leaves it).
        let mods = |m: M| {
            let mut i = input(1.0);
            i.events.push(egui::Event::ModifiersChanged(m));
            i
        };
        frame(&ctx, &mut app, mods(M::ALT));
        frame(&ctx, &mut app, input(1.0));
        assert!(app.menu_keys.show_keys, "titles are underlined while Alt is held");
        frame(&ctx, &mut app, mods(M::NONE));
        frame(&ctx, &mut app, input(1.0));
        assert_eq!(focused(&ctx), Some(tops[0]), "an Alt tap focuses File");
        key(&ctx, &mut app, Key::Escape, M::NONE);
        // Alt+H opens Help from the stage; an item chosen with Enter runs (and its Enter goes no
        // further than the item).
        focus_stage(&ctx, &mut app);
        key(&ctx, &mut app, Key::H, M::ALT);
        frame(&ctx, &mut app, input(1.0));
        let help = crate::menus::TOP_MENUS.iter().position(|t| *t == "Help").unwrap_or(0);
        assert!(open(&ctx, help), "Alt+H opens Help");
        let t = frame(&ctx, &mut app, input(1.0)).platform_output.accesskit_update.unwrap_or_else(|| panic!("no tree"));
        let found = audit(&t);
        assert!(found.is_empty(), "the open menu passes the audit:\n{}", found.join("\n"));
        let palette = app.menu_keys.open_items(&ctx).into_iter().position(|id| node(&t, id).and_then(Node::label).is_some_and(|l| l.starts_with("Command Palette")));
        let palette = palette.unwrap_or_else(|| panic!("Help has Command Palette…"));
        while app.menu_keys.open_items(&ctx).iter().position(|id| Some(*id) == focused(&ctx)) != Some(palette) {
            key(&ctx, &mut app, Key::ArrowDown, M::NONE);
        }
        key(&ctx, &mut app, Key::Enter, M::NONE);
        frame(&ctx, &mut app, input(1.0));
        assert!(app.palette.is_some(), "Enter ran the item");
        assert!(!open(&ctx, help));
        app.palette = None;
        // View's first item is Zoom In: chosen with Enter, it zooms and the keyboard goes back
        // to the stage.
        focus_stage(&ctx, &mut app);
        let stage = focused(&ctx);
        let zoom = app.view.zoom;
        key(&ctx, &mut app, Key::V, M::ALT);
        frame(&ctx, &mut app, input(1.0));
        assert!(open(&ctx, 2), "Alt+V opens View");
        key(&ctx, &mut app, Key::Enter, M::NONE);
        frame(&ctx, &mut app, input(1.0));
        assert!(app.view.zoom > zoom, "Zoom In ran");
        assert_eq!(focused(&ctx), stage, "the keyboard went back to the stage");
        assert_eq!(app.tool, crate::app::Tool::Select, "Alt+V opened View; it didn't pick the Selection tool's V");
    }

    #[test]
    fn the_palette_follows_the_system_contrast_setting_in_the_same_frame() {
        use crate::platform::SystemLook;
        use crate::theme::Palette;
        let look = |high_contrast, dark| SystemLook { high_contrast, dark };
        let (_, mut app, ctx) = tree(|_, _| {});
        app.set_system_look(&ctx, look(Some(true), Some(true)));
        let followed = crate::theme::palette();
        app.set_system_look(&ctx, look(Some(false), Some(true)));
        let back = crate::theme::palette();
        // The system switches its apps to light: the default look stays dark (#266)...
        app.set_system_look(&ctx, look(Some(false), Some(false)));
        let default_on_light = crate::theme::palette();
        // ...and with Follow system chosen, so does the interface (ADR 0077), egui's own style
        // with it.
        let mut p = app.prefs.clone();
        p.theme_follows_system = Some(true);
        app.set_prefs(&ctx, p);
        app.set_system_look(&ctx, look(Some(false), Some(false)));
        let light = (crate::theme::palette(), ctx.global_style().visuals.panel_fill, ctx.global_style().visuals.dark_mode);
        assert_eq!(default_on_light, Palette::Graphite);
        // Chosen by hand: the system's setting no longer changes it.
        let mut p = app.prefs.clone();
        p.theme_follows_system = Some(false);
        app.set_prefs(&ctx, p);
        app.set_system_look(&ctx, look(Some(true), Some(false)));
        let manual = crate::theme::palette();
        crate::theme::apply_palette(&egui::Context::default(), Palette::Graphite);
        assert_eq!((followed, back, manual), (Palette::HighContrast, Palette::Graphite, Palette::Graphite));
        assert_eq!(light, (Palette::Light, crate::theme::LIGHT.panel, false));
    }

    #[test]
    fn choosing_a_theme_in_preferences_restyles_everything_in_the_same_frame() {
        use crate::theme::{LIGHT, Palette, T};
        let (_, mut app, ctx) = tree(|_, _| {});
        for palette in [Palette::Light, Palette::HighContrast, Palette::Graphite] {
            let mut p = app.prefs.clone();
            p.theme = palette;
            p.theme_follows_system = Some(false);
            app.set_prefs(&ctx, p);
            let tokens = palette.tokens();
            // Tokens read from here on, egui's widgets, and both of egui's styles.
            for theme in [egui::Theme::Dark, egui::Theme::Light] {
                let v = ctx.style_of(theme).visuals.clone();
                assert_eq!((v.panel_fill, v.window_fill, v.widgets.inactive.bg_fill, v.override_text_color), (tokens.panel, tokens.panel_alt, tokens.header, Some(tokens.text)), "{palette:?} in egui's {theme:?} style");
            }
            assert_eq!((crate::theme::palette(), T.panel, ctx.theme() == egui::Theme::Dark), (palette, tokens.panel, tokens.dark));
        }
        // The light look's controls carry their hairline; Graphite's stay flat.
        app.set_prefs(&ctx, crate::prefs::Prefs { theme: Palette::Light, theme_follows_system: Some(false), ..app.prefs.clone() });
        assert_eq!(ctx.global_style().visuals.widgets.inactive.bg_stroke.color, LIGHT.border);
        crate::theme::apply_palette(&egui::Context::default(), Palette::Graphite);
    }

    #[test]
    fn ctrl_f6_moves_focus_through_the_panels_and_back() {
        let (_, mut app, ctx) = tree(|_, _| {});
        let mut seen = Vec::new();
        // Each docked panel in Essentials is a stop (ADR 0075); the Motion Editor and the Movie
        // Explorer are behind other tabs, so they are skipped.
        use crate::dock::Panel as P;
        let expect = [Region::Stage, Region::Timeline, Region::Properties, Region::Panel(P::Align), Region::Library, Region::Panel(P::History), Region::Tools];
        for _ in 0..expect.len() {
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
        assert_eq!(seen, expect.map(Some).to_vec());
        let mut i = input(1.0);
        i.events.push(egui::Event::Key { key: egui::Key::F6, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND | egui::Modifiers::SHIFT });
        frame(&ctx, &mut app, i);
        frame(&ctx, &mut app, input(1.0));
        assert_eq!(focused_region(&ctx), Some(Region::Panel(P::History)), "Ctrl+Shift+F6 goes back");
    }
}
