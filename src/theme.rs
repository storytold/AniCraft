//! Design tokens. Every colour, radius and target size in the shell comes from here
//! (docs/ui-guidelines.md).
//!
//! The look (ADR 0008): graphite neutrals, geometric controls, borders only where they separate
//! layout regions, flat buttons, and vibrant colour spent where it carries meaning (timeline
//! blocks, playhead, selection). Corners follow the role of a shape, not one global value:
//! structure is square, controls are softened slightly, floating surfaces are rounder.

use egui::{Color32, CornerRadius, Shadow, Stroke, Visuals};

#[derive(Clone, Copy)]
pub struct Tokens {
    /// Menu bar, status bar: the frame around the work.
    pub chrome: Color32,
    pub panel: Color32,
    /// Panel headers, alternate rows, windows.
    pub panel_alt: Color32,
    /// Raised surfaces: buttons at rest.
    pub header: Color32,
    /// Buttons under the pointer.
    pub hover: Color32,
    /// The active tool or a toggle that is on: a lighter gray block, the icon white.
    pub active: Color32,
    /// Icons at rest: a step down from white so the active one stands out.
    pub icon: Color32,
    /// Inset surfaces: text and number fields.
    pub field: Color32,
    /// Lines between layout regions and around popups.
    pub border: Color32,
    /// Edges of checkboxes and radio buttons (≥ 3:1 so the unticked state is visible).
    pub control_edge: Color32,
    /// Fine grid lines inside a region (timeline cells, row rules).
    pub separator: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_faint: Color32,
    /// Selection, focus, active tool.
    pub accent: Color32,
    pub accent_dim: Color32,
    /// Dark marks drawn on vibrant blocks (keyframe dots, tween arrows, labels).
    pub ink: Color32,
    pub pasteboard: Color32,
    pub stage_shadow: Color32,
    /// Timeline: span holding artwork.
    pub span: Color32,
    /// Timeline: empty span.
    pub span_empty: Color32,
    /// Timeline: frame cells of the active layer.
    pub row_active: Color32,
    /// Timeline: classic tween span (violet) and shape tween span (amber).
    pub tween: Color32,
    pub tween_shape: Color32,
    pub tween_line: Color32,
    pub playhead: Color32,
    pub selection: Color32,
    /// Destructive buttons.
    pub danger: Color32,
    pub error: Color32,
    pub ok: Color32,
    /// Library symbol kinds.
    pub kind_graphic: Color32,
    pub kind_movie_clip: Color32,
    pub kind_button: Color32,
    /// Corner radii by role (see `Shape` below).
    pub radius_control: u8,
    pub radius_menu: u8,
    pub radius_window: u8,
}

/// Corner shapes are chosen by what a surface *is*:
///
/// * **Structure is square** (`0.0`): panels, section bands, the timeline grid and its blocks,
///   selections, the stage. They tile against each other; rounding would leave gaps.
/// * **Controls are softened** (`radius_control`): buttons, fields, tools, list highlights,
///   swatches, the playhead flag. Rectangular and geometric, but not sharp.
/// * **Floating surfaces are rounder** (`radius_menu`, `radius_window`): menus, tooltips,
///   dialogs and windows sit above the work, and their shape says so.
/// * **Round only where the thing is round**: keyframe dots, dials, onion-skin marks.
pub const SQUARE: f32 = 0.0;

pub const T: Tokens = Tokens {
    chrome: Color32::from_rgb(0x13, 0x13, 0x16),
    panel: Color32::from_rgb(0x1b, 0x1b, 0x1f),
    panel_alt: Color32::from_rgb(0x22, 0x22, 0x27),
    header: Color32::from_rgb(0x2c, 0x2c, 0x32),
    hover: Color32::from_rgb(0x3a, 0x3a, 0x42),
    active: Color32::from_rgb(0x4a, 0x4a, 0x53),
    icon: Color32::from_rgb(0xb4, 0xb4, 0xbc),
    field: Color32::from_rgb(0x0e, 0x0e, 0x11),
    border: Color32::from_rgb(0x36, 0x36, 0x3e),
    control_edge: Color32::from_rgb(0x78, 0x78, 0x82),
    separator: Color32::from_rgb(0x26, 0x26, 0x2c),
    text: Color32::from_rgb(0xee, 0xee, 0xf0),
    text_dim: Color32::from_rgb(0xb0, 0xb0, 0xb8),
    // ≥ 4.5:1 on `panel`, so even hints stay readable.
    text_faint: Color32::from_rgb(0x8e, 0x8e, 0x98),
    accent: Color32::from_rgb(0x2e, 0x66, 0xf0),
    accent_dim: Color32::from_rgb(0x1f, 0x35, 0x70),
    ink: Color32::from_rgb(0x0e, 0x0e, 0x10),
    pasteboard: Color32::from_rgb(0x2a, 0x2a, 0x30),
    stage_shadow: Color32::from_black_alpha(150),
    span: Color32::from_rgb(0x22, 0xc7, 0xa9),
    span_empty: Color32::from_rgb(0x2e, 0x2e, 0x35),
    row_active: Color32::from_rgb(0x24, 0x27, 0x33),
    tween: Color32::from_rgb(0x8a, 0x5c, 0xff),
    tween_shape: Color32::from_rgb(0xf2, 0xa1, 0x3b),
    tween_line: Color32::from_rgb(0x0e, 0x0e, 0x10),
    playhead: Color32::from_rgb(0xff, 0x4d, 0x3d),
    selection: Color32::from_rgb(0x2e, 0x66, 0xf0),
    danger: Color32::from_rgb(0xd8, 0x34, 0x2a),
    error: Color32::from_rgb(0xff, 0x6e, 0x60),
    ok: Color32::from_rgb(0x5f, 0xe0, 0x9a),
    kind_graphic: Color32::from_rgb(0x22, 0xc7, 0xa9),
    kind_movie_clip: Color32::from_rgb(0x5a, 0x8f, 0xff),
    kind_button: Color32::from_rgb(0xff, 0xb5, 0x3d),
    radius_control: 3,
    radius_menu: 6,
    radius_window: 10,
};

impl Tokens {
    /// Control corner radius for painter calls.
    pub fn rc(&self) -> f32 {
        f32::from(self.radius_control)
    }
}

/// Smallest click target (WCAG 2.5.8 asks for 24 × 24; we use 28).
pub const TARGET: f32 = 28.0;
/// Tool buttons in the tools bar, and the glyph inside them.
pub const TOOL_BUTTON: f32 = 36.0;
pub const TOOL_ICON: f32 = 18.0;

/// Depth for floating surfaces: tight and downward, so a window lifts off the work without a
/// glow around it.
fn depth(y: i8, blur: u8, alpha: u8) -> Shadow {
    Shadow { offset: [0, y], blur, spread: 0, color: Color32::from_black_alpha(alpha) }
}

pub fn apply(ctx: &egui::Context) {
    install_fonts(ctx, None, None);
    let t = T;
    let mut v = Visuals::dark();
    v.panel_fill = t.panel;
    v.window_fill = t.panel_alt;
    v.extreme_bg_color = t.field;
    v.text_edit_bg_color = Some(t.field);
    v.faint_bg_color = t.panel_alt;
    v.override_text_color = Some(t.text);
    v.selection.bg_fill = t.accent_dim;
    v.selection.stroke = Stroke::new(2.0, t.accent);
    v.hyperlink_color = t.accent;
    v.window_stroke = Stroke::new(1.0, t.border);
    v.window_shadow = depth(10, 28, 150);
    v.popup_shadow = depth(4, 12, 130);
    v.window_corner_radius = CornerRadius::same(t.radius_window);
    v.menu_corner_radius = CornerRadius::same(t.radius_menu);
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = CornerRadius::same(t.radius_control);
        w.expansion = 0.0;
    }
    // Separators, group frames and other non-interactive shapes are structure: square.
    v.widgets.noninteractive.corner_radius = CornerRadius::ZERO;
    // Separators and group frames: the layout lines.
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, t.border);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, t.text_dim);
    // Buttons are flat fills at rest; no border on every button.
    v.widgets.inactive.weak_bg_fill = t.header;
    v.widgets.inactive.bg_fill = t.header;
    v.widgets.inactive.bg_stroke = Stroke::NONE;
    v.widgets.inactive.fg_stroke = Stroke::new(1.5, t.text);
    v.widgets.hovered.weak_bg_fill = t.hover;
    v.widgets.hovered.bg_fill = t.hover;
    v.widgets.hovered.bg_stroke = Stroke::NONE;
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, t.text);
    // Pressed and keyboard-focused widgets (egui styles focus as "active"): a visible ring.
    v.widgets.active.weak_bg_fill = t.accent_dim;
    v.widgets.active.bg_fill = t.accent_dim;
    v.widgets.active.bg_stroke = Stroke::new(2.0, t.accent);
    v.widgets.active.fg_stroke = Stroke::new(1.5, Color32::WHITE);
    v.widgets.open.weak_bg_fill = t.hover;
    v.widgets.open.bg_fill = t.hover;
    v.widgets.open.bg_stroke = Stroke::NONE;
    // Same dark look whatever the OS theme is.
    ctx.all_styles_mut(|s| {
        s.visuals = v.clone();
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(10.0, 5.0);
        s.spacing.interact_size = egui::vec2(TARGET, TARGET);
        s.spacing.icon_width = 16.0;
        s.spacing.icon_width_inner = 9.0;
        s.spacing.menu_margin = egui::Margin::same(6);
        // Built-in text styles follow the roles, so plain widgets match the guidelines too.
        use egui::TextStyle;
        s.text_styles.insert(TextStyle::Body, font(Type::Body));
        s.text_styles.insert(TextStyle::Button, font(Type::Body));
        s.text_styles.insert(TextStyle::Small, font(Type::Caption));
        s.text_styles.insert(TextStyle::Heading, font(Type::Title));
        s.text_styles.insert(TextStyle::Monospace, font(Type::Data));
    });
}

mod craft_fonts {
    include!(concat!(env!("OUT_DIR"), "/fonts.rs"));
}

/// Text being typed on the stage: document content, so it follows the stage zoom rather than a
/// UI role (clamped to stay readable).
pub fn stage_text_font(size: f32) -> egui::FontId {
    egui::FontId::proportional(size.clamp(12.0, 64.0))
}

/// The typefaces embedded at build time (Inter, JetBrains Mono), for text on the stage.
pub fn embedded_fonts() -> Vec<std::sync::Arc<[u8]>> {
    [craft_fonts::UI_FONT, craft_fonts::MONO_FONT].into_iter().flatten().map(std::sync::Arc::from).collect()
}

/// Which typefaces are active (for About and diagnostics).
pub fn font_summary() -> String {
    match (craft_fonts::UI_FONT, craft_fonts::MONO_FONT) {
        (Some(_), Some(_)) => crate::i18n::fmt("Inter + JetBrains Mono (craft-fonts, {kb} KB)", &[("kb", &(craft_fonts::CRAFT_FONT_BYTES / 1024).to_string())]),
        (Some(_), None) => crate::i18n::t("Inter + Hack (fallback)").into(),
        (None, Some(_)) => crate::i18n::t("Ubuntu (fallback) + JetBrains Mono").into(),
        (None, None) => crate::i18n::t("Ubuntu + Hack (built-in defaults; build with CRAFT_FONTS_DIR for Inter + JetBrains Mono)").into(),
    }
}

/// A user-chosen system font: family name, file bytes, face index.
pub type UserFont = (String, Vec<u8>, u32);

/// Installs the UI typefaces (ADR 0005). Order in each family: the user's system font (from
/// Preferences), then the craft-fonts pair (when embedded at build time), then egui's bundled
/// fonts, which stay as fallbacks for glyph coverage, then Noto Sans Arabic for the Arabic
/// letters none of them has (ADR 0014).
pub fn install_fonts(ctx: &egui::Context, ui_font: Option<&UserFont>, data_font: Option<&UserFont>) {
    use egui::{FontData, FontDefinitions, FontFamily};
    use std::sync::Arc;
    let mut defs = FontDefinitions::default();
    if let Some(b) = craft_fonts::ARABIC_FONT {
        defs.font_data.insert("craft-noto-arabic".to_owned(), Arc::new(FontData::from_static(b)));
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            defs.families.entry(family).or_default().push("craft-noto-arabic".to_owned());
        }
    }
    for (bytes, name, family) in [(craft_fonts::UI_FONT, "craft-inter", FontFamily::Proportional), (craft_fonts::MONO_FONT, "craft-jetbrains-mono", FontFamily::Monospace)] {
        if let Some(b) = bytes {
            defs.font_data.insert(name.to_owned(), Arc::new(FontData::from_static(b)));
            defs.families.entry(family).or_default().insert(0, name.to_owned());
        }
    }
    for (user, family) in [(ui_font, FontFamily::Proportional), (data_font, FontFamily::Monospace)] {
        if let Some((name, bytes, index)) = user {
            let key = format!("user:{name}");
            let mut data = FontData::from_owned(bytes.clone());
            data.index = *index;
            defs.font_data.insert(key.clone(), Arc::new(data));
            defs.families.entry(family).or_default().insert(0, key);
        }
    }
    ctx.set_fonts(defs);
}

/// Names of the bundled typefaces actually embedded in this build.
pub fn bundled_names() -> (&'static str, &'static str) {
    (if craft_fonts::UI_FONT.is_some() { "Inter" } else { "Ubuntu" }, if craft_fonts::MONO_FONT.is_some() { "JetBrains Mono" } else { "Hack" })
}

/// Typographic roles (docs/ui-guidelines.md › Typography). **Prose is proportional; data is
/// monospace.** Data means values that are measured, counted or typed as codes: frame numbers,
/// timecode, rulers, coordinates, sizes, percentages, readouts, shortcut keys, paths and
/// preformatted text. Fixed-width digits do not jitter while a value changes, and columns line
/// up. Everything people *read* (labels, menus, buttons, names, dialog text, hints) stays
/// proportional. Code outside this module uses these roles, never raw font sizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Type {
    /// Dialog titles.
    Title,
    /// Panel and section headings (sentence case, bold; see `section`).
    Heading,
    /// Names and primary text: layer and symbol names, list items.
    Body,
    /// Secondary prose: status messages, field labels.
    Label,
    /// Hints, explanations, footnotes.
    Caption,
    /// Values and readouts: numeric fields, fps, time, coordinates, zoom.
    Data,
    /// Dense data: ruler ticks, playhead and badge numbers.
    DataSmall,
    /// The one hero number: current frame.
    DataLarge,
    /// Keyboard shortcuts and keycaps.
    Key,
    /// File paths, ids and preformatted documents (licence texts, JSON).
    Path,
}

/// Font for a role.
pub fn font(t: Type) -> egui::FontId {
    use egui::FontId;
    match t {
        Type::Title => FontId::proportional(17.0),
        Type::Heading => FontId::proportional(12.5),
        Type::Body => FontId::proportional(13.0),
        Type::Label => FontId::proportional(12.0),
        Type::Caption => FontId::proportional(11.0),
        Type::Data => FontId::monospace(12.0),
        Type::DataSmall => FontId::monospace(10.0),
        Type::DataLarge => FontId::monospace(14.5),
        Type::Key => FontId::monospace(11.5),
        Type::Path => FontId::monospace(11.0),
    }
}

/// Default colour for a role (override with `.color()` for state: error, accent…).
pub fn color(t: Type) -> Color32 {
    match t {
        Type::Title | Type::Heading | Type::Body | Type::Data | Type::DataLarge | Type::Path => T.text,
        Type::Label | Type::DataSmall => T.text_dim,
        Type::Caption | Type::Key => T.text_faint,
    }
}

/// Text in a role: font, size, colour (and weight for titles and headings).
pub fn text(t: Type, s: impl Into<String>) -> egui::RichText {
    let rt = egui::RichText::new(s).font(font(t)).color(color(t));
    match t {
        Type::Title | Type::Heading => rt.strong(),
        _ => rt,
    }
}

/// Buttons have no resting border, but a checkbox or radio button without one would vanish
/// when off. These give selection controls their own visible edge.
fn with_control_edges<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.scope(|ui| {
        let w = &mut ui.visuals_mut().widgets;
        w.inactive.bg_stroke = Stroke::new(1.0, T.control_edge);
        w.hovered.bg_stroke = Stroke::new(1.0, T.text_dim);
        add(ui)
    })
    .inner
}

pub fn checkbox(ui: &mut egui::Ui, on: &mut bool, label: impl Into<egui::WidgetText>) -> egui::Response {
    with_control_edges(ui, |ui| ui.checkbox(on, label))
}

pub fn radio_value<V: PartialEq>(ui: &mut egui::Ui, current: &mut V, value: V, label: impl Into<egui::WidgetText>) -> egui::Response {
    with_control_edges(ui, |ui| ui.radio_value(current, value, label))
}

// ----- right to left (ADR 0014) -------------------------------------------------------------
// In a right-to-left language (Arabic) the window mirrors: rows start at the right, the tools
// sit on the right and the dock on the left. The stage, the timeline's frames and the playback
// buttons keep their direction: they follow time and the artwork, not the reading order.

pub fn rtl() -> bool {
    crate::i18n::rtl()
}

/// A row filled from where reading starts: the left, or the right in a right-to-left language.
pub fn start_row(align: egui::Align) -> egui::Layout {
    if rtl() { egui::Layout::right_to_left(align) } else { egui::Layout::left_to_right(align) }
}

/// A row filled from its end (what is pushed to the far side): from the right, or from the left
/// in a right-to-left language.
pub fn end_row(align: egui::Align) -> egui::Layout {
    if rtl() { egui::Layout::left_to_right(align) } else { egui::Layout::right_to_left(align) }
}

/// Draw `f` from where reading starts. Panels and windows lay out from the left whatever the
/// language; inside this, items line up at the right and `ui.horizontal` rows start there.
pub fn reading<R>(ui: &mut egui::Ui, f: impl FnOnce(&mut egui::Ui) -> R) -> R {
    if rtl() && ui.layout().main_dir() == egui::Direction::TopDown { ui.with_layout(egui::Layout::top_down(egui::Align::Max), f).inner } else { f(ui) }
}

/// `ui.vertical`, lined up where reading starts. (`vertical` in a right-to-left row would put the
/// column at the far left.)
pub fn column<R>(ui: &mut egui::Ui, f: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    let align = if rtl() { egui::Align::Max } else { egui::Align::Min };
    ui.scope_builder(egui::UiBuilder::new().layout(egui::Layout::top_down(align)), f)
}

/// Left to right whatever the language, for egui's `Grid` (it has no right-to-left layout).
pub fn ltr<R>(ui: &mut egui::Ui, f: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.with_layout(egui::Layout::top_down(egui::Align::Min), f).inner
}

/// For hand-painted widgets: `x` measured from the left of `within`, moved to the same distance
/// from its right in a right-to-left language.
pub fn mirror_x(within: egui::Rect, x: f32) -> f32 {
    if rtl() { within.left() + within.right() - x } else { x }
}

/// For hand-painted widgets: `r` laid out from the left of `within`, mirrored in a right-to-left
/// language.
pub fn mirror(within: egui::Rect, r: egui::Rect) -> egui::Rect {
    if rtl() { egui::Rect::from_x_y_ranges(mirror_x(within, r.right())..=mirror_x(within, r.left()), r.y_range()) } else { r }
}

/// Label column widths for forms: dialogs (Preferences) and the narrow dock.
pub const FORM_LABEL_DIALOG: f32 = 150.0;
pub const FORM_LABEL_DOCK: f32 = 78.0;

/// A form row on a two-column grid: the label at the start of a fixed column, the controls after
/// it. Every row's controls start at the same x, so a form reads as one column of values.
pub fn form_row<R>(ui: &mut egui::Ui, label_w: f32, label: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    // `horizontal` runs from the right inside `reading`.
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(egui::vec2(label_w, TARGET), start_row(egui::Align::Center), |ui| {
            ui.set_min_size(egui::vec2(label_w, TARGET));
            ui.label(text(Type::Label, crate::i18n::t(label)));
        });
        add(ui)
    })
    .inner
}

/// A note that belongs to the controls above it (an explanation, a preview): aligned with the
/// control column, wrapping within it, never back at the label column.
pub fn form_note(ui: &mut egui::Ui, label_w: f32, note: impl Into<egui::WidgetText>) {
    ui.horizontal(|ui| {
        // A row puts one item spacing between its label column and the controls.
        ui.add_space(label_w + ui.spacing().item_spacing.x);
        ui.add(egui::Label::new(note).wrap());
    });
}

/// How a collapsible section header looks.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// A full-width band with layout borders and an accent edge: top-level dock sections.
    Band,
    /// Title followed by a horizontal rule across the row: groups inside a panel or dialog.
    Rule,
}

/// A collapsible section: a header the whole width of the row (one big click target, keyboard
/// focusable, announced as expanded/collapsed), then `add` while open. Open state is kept per
/// `id` for the session. `detail` is an optional data value shown after the title in the data
/// face (a frame number, a count).
pub fn section<R>(ui: &mut egui::Ui, id: &str, style: Section, title: &str, detail: Option<&str>, add: impl FnOnce(&mut egui::Ui) -> R) -> Option<R> {
    use egui::{Sense, StrokeKind, pos2, vec2};
    let key = ui.make_persistent_id(("section", id));
    let mut open = ui.data_mut(|d| *d.get_temp_mut_or(key, true));
    let height = if style == Section::Band { 32.0 } else { TARGET };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    if resp.clicked() {
        open = !open;
        ui.data_mut(|d| d.insert_temp(key, open));
    }
    let enabled = ui.is_enabled();
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::DisclosureTriangle, enabled, open, title));
    let p = ui.painter();
    let hot = resp.hovered() || resp.has_focus();
    match style {
        Section::Band => {
            p.rect_filled(rect, SQUARE, if hot { T.header } else { T.panel_alt });
            p.line_segment([rect.left_top(), rect.right_top()], Stroke::new(1.0, T.border));
            p.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, T.border));
            p.rect_filled(mirror(rect, egui::Rect::from_min_size(rect.left_top() + vec2(0.0, 1.0), vec2(3.0, rect.height() - 1.0))), SQUARE, T.accent);
        }
        Section::Rule => {
            if hot {
                p.rect_filled(rect, T.rc(), T.panel_alt);
            }
        }
    }
    if resp.has_focus() {
        p.rect_stroke(rect.shrink(1.0), T.rc(), Stroke::new(2.0, T.accent), StrokeKind::Inside);
    }
    // Chevron: pointing where reading goes when closed, down when open. Laid out from the left,
    // then mirrored in a right-to-left language.
    let left = if style == Section::Band { 10.0 } else { 2.0 };
    let chevron = mirror(rect, egui::Rect::from_center_size(pos2(rect.left() + left + 7.0, rect.center().y), vec2(14.0, 14.0)));
    // `Prev` is the same chevron pointing left.
    let icon = if open { crate::icons::Icon::ChevronDown } else if rtl() { crate::icons::Icon::Prev } else { crate::icons::Icon::ChevronRight };
    crate::icons::paint(p, chevron, icon, if hot { T.text } else { T.text_dim });
    let mut x = rect.left() + left + 20.0;
    let title_g = p.layout_no_wrap(title.to_string(), font(Type::Heading), color(Type::Heading));
    let w = title_g.rect.width();
    p.galley(pos2(mirror_x(rect, x + if rtl() { w } else { 0.0 }), rect.center().y - title_g.rect.height() * 0.5), title_g, color(Type::Heading));
    x += w;
    if let Some(d) = detail {
        let g = p.layout_no_wrap(d.to_string(), font(Type::Data), T.text_dim);
        x += 8.0;
        let dw = g.rect.width();
        p.galley(pos2(mirror_x(rect, x + if rtl() { dw } else { 0.0 }), rect.center().y - g.rect.height() * 0.5), g, T.text_dim);
        x += dw;
    }
    if style == Section::Rule && x + 12.0 < rect.right() {
        let (a, b) = (mirror_x(rect, x + 12.0), mirror_x(rect, rect.right()));
        p.hline(a.min(b)..=a.max(b), rect.center().y, Stroke::new(1.0, T.border));
    }
    if !open {
        if style == Section::Rule {
            ui.add_space(2.0);
        }
        return None;
    }
    if style == Section::Rule {
        ui.add_space(2.0);
    }
    let r = add(ui);
    ui.add_space(if style == Section::Rule { 10.0 } else { 0.0 });
    Some(r)
}

/// Runs `add` with every widget's text in `role` (e.g. numeric fields in the Data role).
pub fn with_type<R>(ui: &mut egui::Ui, role: Type, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.scope(|ui| {
        ui.style_mut().override_font_id = Some(font(role));
        add(ui)
    })
    .inner
}

pub fn to_color32(c: anicraft_engine::doc::Rgba) -> Color32 {
    let c = c.sanitized();
    Color32::from_rgba_unmultiplied((c.r * 255.0).round() as u8, (c.g * 255.0).round() as u8, (c.b * 255.0).round() as u8, (c.a * 255.0).round() as u8)
}
