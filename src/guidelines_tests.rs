//! Enforces docs/ui-guidelines.md: typography and colour come from `theme`, never raw values.

const SOURCES: [(&str, &str); 31] = [
    ("ease_editor.rs", include_str!("ease_editor.rs")),
    ("colour.rs", include_str!("colour.rs")),
    ("drop.rs", include_str!("drop.rs")),
    ("recent.rs", include_str!("recent.rs")),
    ("start.rs", include_str!("start.rs")),
    ("actions_panel.rs", include_str!("actions_panel.rs")),
    ("test_movie.rs", include_str!("test_movie.rs")),
    ("autosave.rs", include_str!("autosave.rs")),
    ("icons.rs", include_str!("icons.rs")),
    ("pathtools.rs", include_str!("pathtools.rs")),
    ("platform.rs", include_str!("platform.rs")),
    ("plugins_ui.rs", include_str!("plugins_ui.rs")),
    ("prefs.rs", include_str!("prefs.rs")),
    ("a11y.rs", include_str!("a11y.rs")),
    ("app.rs", include_str!("app.rs")),
    ("dialogs.rs", include_str!("dialogs.rs")),
    ("menus.rs", include_str!("menus.rs")),
    ("panels.rs", include_str!("panels.rs")),
    ("stage.rs", include_str!("stage.rs")),
    ("stage_a11y.rs", include_str!("stage_a11y.rs")),
    ("timeline.rs", include_str!("timeline.rs")),
    ("widthtool.rs", include_str!("widthtool.rs")),
    ("motion_editor.rs", include_str!("motion_editor.rs")),
    ("shape_hints.rs", include_str!("shape_hints.rs")),
    ("bone_tool.rs", include_str!("bone_tool.rs")),
    ("find_panel.rs", include_str!("find_panel.rs")),
    ("movie_explorer.rs", include_str!("movie_explorer.rs")),
    ("dock.rs", include_str!("dock.rs")),
    ("dock_ui.rs", include_str!("dock_ui.rs")),
    ("publish_panel.rs", include_str!("publish_panel.rs")),
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

/// Every pair of tokens a palette draws together, with the ratio it must reach: text 4.5:1 (WCAG
/// 1.4.3 AA; the timeline's small numbers are small text too), UI components and focus 3:1
/// (1.4.11). Each palette also has stricter house rules (below).
fn palette_failures(name: &str, t: &crate::theme::Tokens) -> Vec<String> {
    let mut out = Vec::new();
    let mut need = |what: &str, fg: egui::Color32, bg: egui::Color32, min: f32| {
        let c = contrast(fg, bg);
        if c < min {
            out.push(format!("{name}: {what} is {c:.2}:1, needs {min}:1"));
        }
    };
    let surfaces = [("panel", t.panel), ("panel_alt", t.panel_alt), ("chrome", t.chrome), ("header", t.header), ("field", t.field)];
    for (s, bg) in surfaces {
        need(&format!("text on {s}"), t.text, bg, 7.0);
        need(&format!("text_dim on {s}"), t.text_dim, bg, 4.5);
        need(&format!("text_faint on {s}"), t.text_faint, bg, 4.5);
        need(&format!("accent_text on {s}"), t.accent_text, bg, 4.5);
        need(&format!("error on {s}"), t.error, bg, 4.5);
        need(&format!("ok on {s}"), t.ok, bg, 4.5);
    }
    need("text on hover", t.text, t.hover, 4.5);
    need("text on a selected row (accent_dim)", t.text, t.accent_dim, 4.5);
    need("text on row_active", t.text, t.row_active, 4.5);
    // Icons at rest and under the pointer, and on the active tool's block.
    need("icon on panel", t.icon, t.panel, 4.5);
    need("icon_hot on header", t.icon_hot, t.header, 4.5);
    need("icon_hot on hover", t.icon_hot, t.hover, 4.5);
    need("on_active on active", t.on_active, t.active, 7.0);
    // Text on filled buttons (Save, Restore, destructive actions).
    need("on_fill on accent", t.on_fill, t.accent, 4.5);
    need("on_fill on danger", t.on_fill, t.danger, 4.5);
    // Marks on timeline blocks and the playhead's number; the blocks against the grid.
    for (b, bg) in [("span", t.span), ("tween", t.tween), ("tween_shape", t.tween_shape), ("tween_motion", t.tween_motion), ("playhead", t.playhead)] {
        need(&format!("ink on {b}"), t.ink, bg, 4.5);
        need(&format!("{b} on panel"), bg, t.panel, 3.0);
    }
    // Shape hint letters (ink) on every marker colour.
    for (h, bg) in [("hint_unplaced", t.hint_unplaced), ("hint_start", t.hint_start), ("hint_end", t.hint_end)] {
        need(&format!("ink on {h}"), t.ink, bg, 4.5);
    }
    need("text on span_empty", t.text, t.span_empty, 4.5);
    // Sound layers: waveforms on empty spans and on the active row.
    need("waveform on span_empty", t.waveform, t.span_empty, 4.5);
    need("waveform on row_active", t.waveform, t.row_active, 4.5);
    // Focus rings, selection, the active tool and checkbox edges (WCAG 1.4.11).
    for (s, bg) in [("panel", t.panel), ("panel_alt", t.panel_alt), ("chrome", t.chrome)] {
        need(&format!("accent (focus) on {s}"), t.accent, bg, 3.0);
        need(&format!("control_edge on {s}"), t.control_edge, bg, 3.0);
    }
    need("active tool block on panel", t.active, t.panel, if t.control_outline { 3.0 } else { 1.0 });
    // Stage selection is drawn over artwork, usually on a white stage.
    need("selection on a white stage", t.selection, egui::Color32::WHITE, 3.0);
    // Marks drawn across the timeline grid while dragging frames (the drop outline, a span's new
    // end) are in `text`: they must show on the cells and on the active row.
    need("text (drag marks) on row_active", t.text, t.row_active, 4.5);
    if !t.dark {
        // Light: the white stage stands off the pasteboard by its edge line, and the pasteboard
        // is a step darker than the panels so the work area reads as one region.
        need("text on pasteboard", t.text, t.pasteboard, 7.0);
        need("pasteboard against a white stage", t.pasteboard, egui::Color32::WHITE, 1.4);
        need("selection on panel", t.selection, t.panel, 3.0);
    }
    if t.control_outline {
        // High contrast: every region border and control outline is a visible line.
        need("border on panel", t.border, t.panel, 3.0);
        need("control_edge on field", t.control_edge, t.field, 3.0);
        need("control_edge on header", t.control_edge, t.header, 3.0);
    }
    out
}

#[test]
fn palette_meets_contrast_and_target_minimums() {
    use crate::theme::{GRAPHITE, HIGH_CONTRAST, LIGHT, Palette, TARGET, TOOL_BUTTON};
    let mut found = palette_failures("Graphite", &GRAPHITE);
    found.extend(palette_failures("Light", &LIGHT));
    found.extend(palette_failures("High contrast", &HIGH_CONTRAST));
    assert!(found.is_empty(), "contrast below WCAG AA:\n{}", found.join("\n"));
    // High contrast is stricter than AA: every text step reads at 7:1 (AAA) on its surfaces.
    let hc = HIGH_CONTRAST;
    for bg in [hc.panel, hc.panel_alt, hc.chrome, hc.header, hc.hover, hc.field] {
        for fg in [hc.text, hc.text_dim, hc.text_faint, hc.accent_text, hc.icon] {
            assert!(contrast(fg, bg) >= 7.0, "high contrast text below 7:1: {:.2}", contrast(fg, bg));
        }
    }
    // Light is designed for light, not the dark look inverted (ADR 0077): dark text on light
    // surfaces, fields the lightest of them, the frame (chrome) darker than the panels, and egui
    // told it is a light look. Graphite and high contrast are the other way round.
    for (name, t) in [("Graphite", GRAPHITE), ("High contrast", HIGH_CONTRAST), ("Light", LIGHT)] {
        let lum = |c: egui::Color32| contrast(c, egui::Color32::BLACK);
        assert_eq!(t.dark, lum(t.text) > lum(t.panel), "{name}: `dark` says which way text goes");
    }
    let lum = |c: egui::Color32| contrast(c, egui::Color32::BLACK);
    assert!(lum(LIGHT.field) >= lum(LIGHT.panel_alt) && lum(LIGHT.panel_alt) > lum(LIGHT.panel) && lum(LIGHT.panel) > lum(LIGHT.chrome), "light surfaces step down from fields to the frame");
    const { assert!(LIGHT.control_hairline && !GRAPHITE.control_hairline, "light controls carry a hairline") };
    // Every palette the preferences offer is checked, once each.
    assert!(Palette::ALL.iter().all(|(p, _, _)| matches!(p, Palette::Graphite | Palette::HighContrast | Palette::Light)));
    for p in [Palette::Graphite, Palette::Light, Palette::HighContrast] {
        assert_eq!(Palette::ALL.iter().filter(|(q, _, _)| *q == p).count(), 1, "{p:?} offered once");
    }
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
