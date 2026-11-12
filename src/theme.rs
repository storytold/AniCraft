//! Design tokens. Every colour, radius and target size in the shell comes from here
//! (docs/ui-guidelines.md).
//!
//! The look (ADR 0008): graphite neutrals, geometric controls, borders only where they separate
//! layout regions, flat buttons, and vibrant colour spent where it carries meaning (timeline
//! blocks, playhead, selection). Corners follow the role of a shape, not one global value:
//! structure is square, controls are softened slightly, floating surfaces are rounder.

use egui::{Color32, CornerRadius, Shadow, Stroke, Visuals};
use std::sync::atomic::{AtomicU8, Ordering};

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
    /// The active tool or a toggle that is on: a lighter block, its icon in `on_active`.
    pub active: Color32,
    pub on_active: Color32,
    /// Icons at rest: a step down from `icon_hot` so the active one stands out.
    pub icon: Color32,
    /// Icons under the pointer.
    pub icon_hot: Color32,
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
    /// Accent-coloured text (the current frame, links, prompts): the accent lightened until it
    /// reads at 4.5:1 on the panels.
    pub accent_text: Color32,
    /// Text and icons on filled `accent` and `danger` buttons (Save, Delete, Restore).
    pub on_fill: Color32,
    /// Dark marks drawn on vibrant blocks (keyframe dots, tween arrows, labels, the number on
    /// the playhead's flag).
    pub ink: Color32,
    pub pasteboard: Color32,
    pub stage_shadow: Color32,
    /// Timeline: span holding artwork.
    pub span: Color32,
    /// Timeline: empty span.
    pub span_empty: Color32,
    /// Timeline: frame cells of the active layer.
    pub row_active: Color32,
    /// Timeline: classic tween span (violet), shape tween span (amber) and motion tween span
    /// (sky blue, also its curves in the Motion Editor).
    pub tween: Color32,
    pub tween_shape: Color32,
    pub tween_motion: Color32,
    /// Timeline: a sound's waveform on an empty span (on artwork spans it is drawn in ink).
    pub waveform: Color32,
    pub tween_line: Color32,
    pub playhead: Color32,
    pub selection: Color32,
    /// Destructive buttons.
    pub danger: Color32,
    pub error: Color32,
    pub ok: Color32,
    /// Shape hint markers on the stage (ADR 0023): not on an edge yet (rose), placed on the
    /// tween's first keyframe (amber, like the shape tween span) and on its last (mint).
    pub hint_unplaced: Color32,
    pub hint_start: Color32,
    pub hint_end: Color32,
    /// Onion skin (ADR 0047): frames before the playhead (blue, as Animate users know it) and
    /// after it (green); the defaults the onion menu can change. Also the ruler's markers.
    pub onion_past: Color32,
    pub onion_future: Color32,
    /// A symbol's 9-slice guides while it is open for editing (ADR 0055), dashed over a light
    /// underlay so they read on any artwork.
    pub slice_guide: Color32,
    /// A button's hit area (ADR 0057): laid over its other states while it is open for editing,
    /// and over the movie in Test Movie's Show Hit Areas, at [`HIT_AREA_ALPHA`].
    pub hit_area: Color32,
    /// Library symbol kinds.
    pub kind_graphic: Color32,
    pub kind_movie_clip: Color32,
    pub kind_button: Color32,
    /// Buttons and fields carry a visible `control_edge` (WCAG 1.4.11: a control's boundary at
    /// 3:1). Off in the graphite look, where fills and hover carry it; on in high contrast.
    pub control_outline: bool,
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

/// The graphite look (ADR 0008), the default.
pub const GRAPHITE: Tokens = Tokens {
    chrome: Color32::from_rgb(0x13, 0x13, 0x16),
    panel: Color32::from_rgb(0x1b, 0x1b, 0x1f),
    panel_alt: Color32::from_rgb(0x22, 0x22, 0x27),
    header: Color32::from_rgb(0x2c, 0x2c, 0x32),
    hover: Color32::from_rgb(0x3a, 0x3a, 0x42),
    active: Color32::from_rgb(0x4a, 0x4a, 0x53),
    on_active: Color32::WHITE,
    icon: Color32::from_rgb(0xb4, 0xb4, 0xbc),
    icon_hot: Color32::WHITE,
    field: Color32::from_rgb(0x0e, 0x0e, 0x11),
    border: Color32::from_rgb(0x36, 0x36, 0x3e),
    control_edge: Color32::from_rgb(0x78, 0x78, 0x82),
    separator: Color32::from_rgb(0x26, 0x26, 0x2c),
    text: Color32::from_rgb(0xee, 0xee, 0xf0),
    text_dim: Color32::from_rgb(0xb0, 0xb0, 0xb8),
    // ≥ 4.5:1 on every surface up to `header`, so even hints stay readable.
    text_faint: Color32::from_rgb(0x98, 0x98, 0xa2),
    accent: Color32::from_rgb(0x2e, 0x66, 0xf0),
    accent_dim: Color32::from_rgb(0x1f, 0x35, 0x70),
    accent_text: Color32::from_rgb(0x6f, 0x97, 0xff),
    on_fill: Color32::WHITE,
    ink: Color32::from_rgb(0x0e, 0x0e, 0x10),
    pasteboard: Color32::from_rgb(0x2a, 0x2a, 0x30),
    stage_shadow: Color32::from_black_alpha(150),
    span: Color32::from_rgb(0x22, 0xc7, 0xa9),
    span_empty: Color32::from_rgb(0x2e, 0x2e, 0x35),
    row_active: Color32::from_rgb(0x24, 0x27, 0x33),
    tween: Color32::from_rgb(0x8a, 0x5c, 0xff),
    tween_shape: Color32::from_rgb(0xf2, 0xa1, 0x3b),
    tween_motion: Color32::from_rgb(0x4c, 0xb8, 0xf5),
    waveform: Color32::from_rgb(0x7c, 0xc4, 0xff),
    tween_line: Color32::from_rgb(0x0e, 0x0e, 0x10),
    playhead: Color32::from_rgb(0xff, 0x4d, 0x3d),
    selection: Color32::from_rgb(0x2e, 0x66, 0xf0),
    danger: Color32::from_rgb(0xd8, 0x34, 0x2a),
    error: Color32::from_rgb(0xff, 0x6e, 0x60),
    ok: Color32::from_rgb(0x5f, 0xe0, 0x9a),
    hint_unplaced: Color32::from_rgb(0xff, 0x5c, 0x8a),
    hint_start: Color32::from_rgb(0xff, 0xc2, 0x3d),
    hint_end: Color32::from_rgb(0x4f, 0xe3, 0xa8),
    onion_past: Color32::from_rgb(0x4f, 0x8d, 0xff),
    onion_future: Color32::from_rgb(0x4c, 0xd0, 0x7d),
    slice_guide: Color32::from_rgb(0xd9, 0x3c, 0xc4),
    hit_area: Color32::from_rgb(0x1f, 0xc8, 0x8c),
    kind_graphic: Color32::from_rgb(0x22, 0xc7, 0xa9),
    kind_movie_clip: Color32::from_rgb(0x5a, 0x8f, 0xff),
    kind_button: Color32::from_rgb(0xff, 0xb5, 0x3d),
    control_outline: false,
    radius_control: 3,
    radius_menu: 6,
    radius_window: 10,
};

/// High contrast (ADR 0022): the same roles on black, with white text, a yellow accent for
/// focus, selection and the active tool, outlined controls and brighter timeline blocks. Every
/// pair a role puts together meets WCAG AA (text 4.5:1, most of it 7:1 and more; UI parts 3:1),
/// checked by `guidelines_tests`.
pub const HIGH_CONTRAST: Tokens = Tokens {
    chrome: Color32::from_rgb(0x00, 0x00, 0x00),
    panel: Color32::from_rgb(0x00, 0x00, 0x00),
    panel_alt: Color32::from_rgb(0x10, 0x10, 0x10),
    header: Color32::from_rgb(0x1c, 0x1c, 0x1c),
    hover: Color32::from_rgb(0x38, 0x38, 0x38),
    active: Color32::from_rgb(0xff, 0xd8, 0x1a),
    on_active: Color32::from_rgb(0x00, 0x00, 0x00),
    icon: Color32::from_rgb(0xff, 0xff, 0xff),
    icon_hot: Color32::from_rgb(0xff, 0xd8, 0x1a),
    field: Color32::from_rgb(0x00, 0x00, 0x00),
    border: Color32::from_rgb(0x9a, 0x9a, 0x9a),
    control_edge: Color32::from_rgb(0xd6, 0xd6, 0xd6),
    separator: Color32::from_rgb(0x44, 0x44, 0x44),
    text: Color32::from_rgb(0xff, 0xff, 0xff),
    text_dim: Color32::from_rgb(0xe8, 0xe8, 0xe8),
    text_faint: Color32::from_rgb(0xc8, 0xc8, 0xc8),
    accent: Color32::from_rgb(0xff, 0xd8, 0x1a),
    accent_dim: Color32::from_rgb(0x4d, 0x3d, 0x00),
    accent_text: Color32::from_rgb(0xff, 0xd8, 0x1a),
    on_fill: Color32::from_rgb(0x00, 0x00, 0x00),
    ink: Color32::from_rgb(0x00, 0x00, 0x00),
    pasteboard: Color32::from_rgb(0x1c, 0x1c, 0x1c),
    stage_shadow: Color32::from_black_alpha(200),
    span: Color32::from_rgb(0x3d, 0xf0, 0xcf),
    span_empty: Color32::from_rgb(0x26, 0x26, 0x26),
    row_active: Color32::from_rgb(0x1f, 0x1a, 0x00),
    tween: Color32::from_rgb(0xb3, 0x99, 0xff),
    tween_shape: Color32::from_rgb(0xff, 0xb8, 0x5c),
    tween_motion: Color32::from_rgb(0x6b, 0xd0, 0xff),
    tween_line: Color32::from_rgb(0x00, 0x00, 0x00),
    waveform: Color32::from_rgb(0x8f, 0xd3, 0xff),
    playhead: Color32::from_rgb(0xff, 0x6a, 0x5c),
    // Drawn over the artwork, which is usually light: a deep blue stays visible on white.
    selection: Color32::from_rgb(0x1f, 0x5b, 0xff),
    danger: Color32::from_rgb(0xff, 0x70, 0x63),
    error: Color32::from_rgb(0xff, 0x9a, 0x8f),
    ok: Color32::from_rgb(0x74, 0xff, 0xb0),
    hint_unplaced: Color32::from_rgb(0xff, 0x7a, 0xa3),
    hint_start: Color32::from_rgb(0xff, 0xd8, 0x1a),
    hint_end: Color32::from_rgb(0x74, 0xff, 0xb0),
    onion_past: Color32::from_rgb(0x6f, 0xa8, 0xff),
    onion_future: Color32::from_rgb(0x5c, 0xf0, 0x9a),
    slice_guide: Color32::from_rgb(0xff, 0x6e, 0xe6),
    hit_area: Color32::from_rgb(0x3d, 0xff, 0xb0),
    kind_graphic: Color32::from_rgb(0x3d, 0xf0, 0xcf),
    kind_movie_clip: Color32::from_rgb(0x85, 0xad, 0xff),
    kind_button: Color32::from_rgb(0xff, 0xc8, 0x5c),
    control_outline: true,
    radius_control: 3,
    radius_menu: 6,
    radius_window: 10,
};

/// Which palette the interface uses (Preferences › Appearance › Theme).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Palette {
    #[default]
    Graphite,
    HighContrast,
}

impl Palette {
    pub const ALL: [(Palette, &'static str, &'static str); 2] = [
        (Palette::Graphite, tr_noop!("Graphite"), tr_noop!("The standard dark look: graphite panels, colour where it means something.")),
        (Palette::HighContrast, tr_noop!("High contrast"), tr_noop!("White text on black, a yellow focus and selection colour, outlined controls.")),
    ];

    pub fn tokens(self) -> &'static Tokens {
        match self {
            Palette::Graphite => &GRAPHITE,
            Palette::HighContrast => &HIGH_CONTRAST,
        }
    }
}

/// The palette in use. Process-wide view state, like the interface scale: one window, one look.
static ACTIVE: AtomicU8 = AtomicU8::new(0);

pub fn palette() -> Palette {
    match ACTIVE.load(Ordering::Relaxed) {
        1 => Palette::HighContrast,
        _ => Palette::Graphite,
    }
}

/// The tokens of the palette in use, read as `T.panel` or `T.rc()`. A palette change shows from
/// the next read on, so switching themes is live in the same frame.
pub struct ActiveTokens;

impl std::ops::Deref for ActiveTokens {
    type Target = Tokens;
    fn deref(&self) -> &Tokens {
        palette().tokens()
    }
}

pub static T: ActiveTokens = ActiveTokens;

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

/// Fonts and the look, at start-up.
pub fn apply(ctx: &egui::Context) {
    install_fonts(ctx, None, None);
    apply_palette(ctx, Palette::Graphite);
}

/// Switches the palette and restyles every widget with it (Preferences › Theme), live.
pub fn apply_palette(ctx: &egui::Context, palette: Palette) {
    ACTIVE.store(
        match palette {
            Palette::Graphite => 0,
            Palette::HighContrast => 1,
        },
        Ordering::Relaxed,
    );
    let t = *palette.tokens();
    let mut v = Visuals::dark();
    v.panel_fill = t.panel;
    v.window_fill = t.panel_alt;
    v.extreme_bg_color = t.field;
    v.text_edit_bg_color = Some(t.field);
    v.faint_bg_color = t.panel_alt;
    v.override_text_color = Some(t.text);
    v.selection.bg_fill = t.accent_dim;
    v.selection.stroke = Stroke::new(2.0, t.accent);
    v.hyperlink_color = t.accent_text;
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
    v.widgets.inactive.bg_stroke = if t.control_outline { Stroke::new(1.0, t.control_edge) } else { Stroke::NONE };
    v.widgets.inactive.fg_stroke = Stroke::new(1.5, t.text);
    v.widgets.hovered.weak_bg_fill = t.hover;
    v.widgets.hovered.bg_fill = t.hover;
    v.widgets.hovered.bg_stroke = if t.control_outline { Stroke::new(1.0, t.text) } else { Stroke::NONE };
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, t.text);
    // Pressed and keyboard-focused widgets (egui styles focus as "active"): a visible ring.
    v.widgets.active.weak_bg_fill = t.accent_dim;
    v.widgets.active.bg_fill = t.accent_dim;
    v.widgets.active.bg_stroke = Stroke::new(2.0, t.accent);
    v.widgets.active.fg_stroke = Stroke::new(1.5, t.text);
    v.widgets.open.weak_bg_fill = t.hover;
    v.widgets.open.bg_fill = t.hover;
    v.widgets.open.bg_stroke = if t.control_outline { Stroke::new(1.0, t.control_edge) } else { Stroke::NONE };
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

/// Which typefaces are active (for About and diagnostics), in the interface language.
pub fn font_summary() -> String {
    match (craft_fonts::UI_FONT, craft_fonts::MONO_FONT) {
        (Some(_), Some(_)) => tr!("Inter + JetBrains Mono (craft-fonts, {size} KB)", size = craft_fonts::CRAFT_FONT_BYTES / 1024),
        (Some(_), None) => tr!("Inter + Hack (fallback)").into(),
        (None, Some(_)) => tr!("Ubuntu (fallback) + JetBrains Mono").into(),
        (None, None) => tr!("Ubuntu + Hack (built-in defaults; build with CRAFT_FONTS_DIR for Inter + JetBrains Mono)").into(),
    }
}

/// A user-chosen system font: family name, file bytes, face index.
pub type UserFont = (String, Vec<u8>, u32);

/// Installs the UI typefaces (ADR 0005). Order in each family: the user's system font (from
/// Preferences), then the craft-fonts pair (when embedded at build time), then egui's bundled
/// fonts, which stay as fallbacks for glyph coverage.
pub fn install_fonts(ctx: &egui::Context, ui_font: Option<&UserFont>, data_font: Option<&UserFont>) {
    use egui::{FontData, FontDefinitions, FontFamily};
    use std::sync::Arc;
    let mut defs = FontDefinitions::default();
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
    // i18n-exempt: font family names
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

/// A menu title with its access key underlined (ADR 0064), as Windows and Linux show it while
/// Alt is held or the menu bar has the keyboard. `at`: the character (not byte) to underline.
pub fn access_key_label(s: &str, at: Option<usize>) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let plain = egui::TextFormat { font_id: font(Type::Body), color: Color32::PLACEHOLDER, ..Default::default() };
    let marked = egui::TextFormat { underline: Stroke::new(1.0, T.text), ..plain.clone() };
    let cut = |n: usize| s.char_indices().nth(n).map_or(s.len(), |(b, _)| b);
    match at {
        Some(i) if i < s.chars().count() => {
            let (a, b) = (cut(i), cut(i + 1));
            job.append(s.get(..a).unwrap_or(""), 0.0, plain.clone());
            job.append(s.get(a..b).unwrap_or(""), 0.0, marked);
            job.append(s.get(b..).unwrap_or(""), 0.0, plain);
        }
        _ => job.append(s, 0.0, plain),
    }
    job
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

/// Label column widths for forms: dialogs (Preferences) and the narrow dock.
pub const FORM_LABEL_DIALOG: f32 = 150.0;
pub const FORM_LABEL_DOCK: f32 = 78.0;

/// A form row on a two-column grid: the label left-aligned in a fixed column, the controls after
/// it. Every row's controls start at the same x, so a form reads as one column of values.
pub fn form_row<R>(ui: &mut egui::Ui, label_w: f32, label: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(egui::vec2(label_w, TARGET), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_size(egui::vec2(label_w, TARGET));
            ui.label(text(Type::Label, label));
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
    section_with_header(ui, id, style, title, detail, add).1
}

/// [`section`], also returning its header (a panel's keyboard anchor, `a11y::anchor`).
pub fn section_with_header<R>(ui: &mut egui::Ui, id: &str, style: Section, title: &str, detail: Option<&str>, add: impl FnOnce(&mut egui::Ui) -> R) -> (egui::Response, Option<R>) {
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
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::CollapsingHeader, enabled, open, title));
    let p = ui.painter();
    let hot = resp.hovered() || resp.has_focus();
    match style {
        Section::Band => {
            p.rect_filled(rect, SQUARE, if hot { T.header } else { T.panel_alt });
            p.line_segment([rect.left_top(), rect.right_top()], Stroke::new(1.0, T.border));
            p.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, T.border));
            p.rect_filled(egui::Rect::from_min_size(rect.left_top() + vec2(0.0, 1.0), vec2(3.0, rect.height() - 1.0)), SQUARE, T.accent);
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
    // Chevron: right when closed, down when open.
    let left = if style == Section::Band { 10.0 } else { 2.0 };
    let chevron = egui::Rect::from_center_size(pos2(rect.left() + left + 7.0, rect.center().y), vec2(14.0, 14.0));
    let icon = if open { crate::icons::Icon::ChevronDown } else { crate::icons::Icon::ChevronRight };
    crate::icons::paint(p, chevron, icon, if hot { T.text } else { T.text_dim });
    let mut x = rect.left() + left + 20.0;
    let title_g = p.layout_no_wrap(title.to_string(), font(Type::Heading), color(Type::Heading));
    let w = title_g.rect.width();
    p.galley(pos2(x, rect.center().y - title_g.rect.height() * 0.5), title_g, color(Type::Heading));
    x += w;
    if let Some(d) = detail {
        let g = p.layout_no_wrap(d.to_string(), font(Type::Data), T.text_dim);
        x += 8.0;
        let dw = g.rect.width();
        p.galley(pos2(x, rect.center().y - g.rect.height() * 0.5), g, T.text_dim);
        x += dw;
    }
    if style == Section::Rule && x + 12.0 < rect.right() {
        p.hline((x + 12.0)..=rect.right(), rect.center().y, Stroke::new(1.0, T.border));
    }
    if !open {
        if style == Section::Rule {
            ui.add_space(2.0);
        }
        return (resp, None);
    }
    if style == Section::Rule {
        ui.add_space(2.0);
    }
    let r = add(ui);
    ui.add_space(if style == Section::Rule { 10.0 } else { 0.0 });
    (resp, Some(r))
}

/// Runs `add` with every widget's text in `role` (e.g. numeric fields in the Data role).
pub fn with_type<R>(ui: &mut egui::Ui, role: Type, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.scope(|ui| {
        ui.style_mut().override_font_id = Some(font(role));
        add(ui)
    })
    .inner
}

/// The document colour of an interface colour (the onion skin's theme colours, ADR 0047).
pub fn to_rgba(c: Color32) -> anicraft_engine::doc::Rgba {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    anicraft_engine::doc::Rgba::new(f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0, f32::from(a) / 255.0)
}

pub fn to_color32(c: anicraft_engine::doc::Rgba) -> Color32 {
    let c = c.sanitized();
    Color32::from_rgba_unmultiplied((c.r * 255.0).round() as u8, (c.g * 255.0).round() as u8, (c.b * 255.0).round() as u8, (c.a * 255.0).round() as u8)
}

/// How strongly a button's hit area is laid over the artwork (ADR 0057): enough to see where the
/// pointer finds the button, light enough to see the state under it.
pub const HIT_AREA_ALPHA: f32 = 0.35;

/// The tint and opacity a button's hit area is drawn in (ADR 0057).
pub fn hit_area() -> (anicraft_engine::doc::Rgba, f32) {
    (to_rgba(T.hit_area), HIT_AREA_ALPHA)
}
