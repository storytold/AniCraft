//! Enforces docs/ui-guidelines.md: typography and colour come from `theme`, never raw values.

const SOURCES: [(&str, &str); 12] = [
    ("autosave.rs", include_str!("autosave.rs")),
    ("icons.rs", include_str!("icons.rs")),
    ("pathtools.rs", include_str!("pathtools.rs")),
    ("platform.rs", include_str!("platform.rs")),
    ("prefs.rs", include_str!("prefs.rs")),
    ("app.rs", include_str!("app.rs")),
    ("dialogs.rs", include_str!("dialogs.rs")),
    ("menus.rs", include_str!("menus.rs")),
    ("panels.rs", include_str!("panels.rs")),
    ("stage.rs", include_str!("stage.rs")),
    ("timeline.rs", include_str!("timeline.rs")),
    ("motion_editor.rs", include_str!("motion_editor.rs")),
    ("main.rs", ""),
];

#[test]
fn typography_goes_through_theme_roles() {
    // Raw font families/sizes outside theme.rs bypass the prose/data rules.
    let banned = ["FontId::", ".monospace()", ".size(", "RichText::new("];
    let mut found = Vec::new();
    for (file, src) in SOURCES {
        for (n, line) in src.lines().enumerate() {
            for b in banned {
                if line.contains(b) {
                    found.push(format!("{file}:{}: `{b}` (use theme::text / theme::font)", n + 1));
                }
            }
        }
    }
    assert!(found.is_empty(), "UI guideline violations:\n{}", found.join("\n"));
}

#[test]
fn colours_and_shapes_come_from_tokens() {
    // Raw colours and radii outside theme.rs drift from the palette and the square look.
    let banned = ["Color32::from_rgb(", "Color32::from_rgba_unmultiplied(", "CornerRadius::same("];
    let mut found = Vec::new();
    for (file, src) in SOURCES {
        for (n, line) in src.lines().enumerate() {
            for b in banned {
                if line.contains(b) {
                    found.push(format!("{file}:{}: `{b}` (add a token to theme::T)", n + 1));
                }
            }
        }
    }
    assert!(found.is_empty(), "UI guideline violations:\n{}", found.join("\n"));
}

/// WCAG 2.x relative luminance and contrast ratio.
fn contrast(a: egui::Color32, b: egui::Color32) -> f32 {
    let lum = |c: egui::Color32| {
        let ch = |v: u8| {
            let s = f32::from(v) / 255.0;
            if s <= 0.040_45 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
    };
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

#[test]
fn palette_meets_contrast_and_target_minimums() {
    use crate::theme::{T, TARGET, TOOL_BUTTON};
    use egui::Color32;
    for (bg, name) in [(T.panel, "panel"), (T.panel_alt, "panel_alt"), (T.chrome, "chrome")] {
        assert!(contrast(T.text, bg) >= 7.0, "text on {name}");
        assert!(contrast(T.text_dim, bg) >= 4.5, "text_dim on {name}");
        assert!(contrast(T.text_faint, bg) >= 4.5, "text_faint on {name}: {:.2}", contrast(T.text_faint, bg));
    }
    // Button text on raised and hovered surfaces.
    assert!(contrast(T.text, T.header) >= 7.0 && contrast(T.text, T.hover) >= 4.5);
    // Icons at rest, and the white icon on the active tool's gray block.
    assert!(contrast(T.icon, T.panel) >= 4.5 && contrast(Color32::WHITE, T.active) >= 7.0, "tool icons");
    // Text on filled buttons (Save, destructive actions).
    assert!(contrast(Color32::WHITE, T.accent) >= 4.5, "white on accent: {:.2}", contrast(Color32::WHITE, T.accent));
    assert!(contrast(Color32::WHITE, T.danger) >= 4.5);
    // Marks on timeline blocks, and the blocks against the grid (non-text: 3:1).
    assert!(contrast(T.ink, T.span) >= 4.5 && contrast(T.ink, T.tween) >= 4.5 && contrast(T.ink, T.tween_shape) >= 4.5 && contrast(T.ink, T.tween_motion) >= 4.5, "ink on blocks");
    assert!(contrast(T.text, T.span_empty) >= 4.5);
    assert!(contrast(T.span, T.panel) >= 3.0 && contrast(T.tween, T.panel) >= 3.0 && contrast(T.tween_shape, T.panel) >= 3.0 && contrast(T.tween_motion, T.panel) >= 3.0);
    // Focus and selection outlines, and checkbox edges, stand out from the panels (WCAG 1.4.11).
    assert!(contrast(T.control_edge, T.panel_alt) >= 3.0 && contrast(T.control_edge, T.panel) >= 3.0, "control edges");
    assert!(contrast(T.accent, T.panel) >= 3.0, "accent on panel: {:.2}", contrast(T.accent, T.panel));
    // Click targets (WCAG 2.5.8 asks for at least 24 × 24).
    const { assert!(TARGET >= 24.0 && TOOL_BUTTON >= TARGET) };
}

#[test]
fn data_roles_are_monospace_and_prose_roles_are_not() {
    use crate::theme::{Type, font};
    use egui::FontFamily;
    for t in [Type::Data, Type::DataSmall, Type::DataLarge, Type::Key, Type::Path] {
        assert_eq!(font(t).family, FontFamily::Monospace, "{t:?}");
    }
    for t in [Type::Title, Type::Heading, Type::Body, Type::Label, Type::Caption] {
        assert_eq!(font(t).family, FontFamily::Proportional, "{t:?}");
    }
}
