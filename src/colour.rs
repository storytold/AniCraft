//! Colour swatches and the colour picker (#201, #246, #248), as Animate's swatch popup: a click
//! on a swatch opens the web-safe palette with the colours used lately below it, a hex field,
//! and the full mixer (hue, saturation and value, alpha) one click away.
//!
//! The 216 web-safe colours are the open standard cube (each channel 00, 33, 66, 99, CC or FF),
//! laid out as the usual six-by-six blocks: red steps across the blocks, green down a block and
//! blue across it. A column of greys and primaries sits on its left.
//!
//! View state only: what a colour is used for is the caller's (a tool's swatch, or a command).

use crate::a11y::Named;
use crate::theme::{T, Type, text, to_color32, with_type};
use anicraft_engine::doc::Rgba;
use egui::{Color32, Id, Pos2, Rect, Response, Sense, Stroke, StrokeKind, pos2, vec2};

/// One palette cell, in points.
const CELL: f32 = 14.0;
/// Columns of the web-safe palette, and its rows.
const COLS: usize = 18;
const ROWS: usize = 12;
/// Most colours kept in the recent row.
const RECENT: usize = COLS + 1;
/// Most rows of document swatches the picker shows (the Swatches panel has them all).
const DOC_ROWS: usize = 3;

/// The channel steps of the web-safe cube.
const STEPS: [u8; 6] = [0x00, 0x33, 0x66, 0x99, 0xcc, 0xff];

/// The web-safe colour at `row` (0..12), `col` (0..18): blocks of six by six, red by block,
/// green down the block, blue across it.
pub fn web_safe(row: usize, col: usize) -> Option<[u8; 3]> {
    if row >= ROWS || col >= COLS {
        return None;
    }
    let red = (row / 6) * 3 + col / 6;
    Some([*STEPS.get(red)?, *STEPS.get(row % 6)?, *STEPS.get(col % 6)?])
}

/// The column left of the palette: greys from black to white, then the primaries and
/// secondaries.
const SIDE: [[u8; 3]; ROWS] = [
    [0x00, 0x00, 0x00],
    [0x33, 0x33, 0x33],
    [0x66, 0x66, 0x66],
    [0x99, 0x99, 0x99],
    [0xcc, 0xcc, 0xcc],
    [0xff, 0xff, 0xff],
    [0xff, 0x00, 0x00],
    [0x00, 0xff, 0x00],
    [0x00, 0x00, 0xff],
    [0xff, 0xff, 0x00],
    [0x00, 0xff, 0xff],
    [0xff, 0x00, 0xff],
];

/// `#rrggbb`, or `#rrggbbaa` when `alpha` and it isn't opaque.
pub fn to_hex(c: [u8; 4], alpha: bool) -> String {
    if alpha && c[3] != 255 {
        format!("#{:02x}{:02x}{:02x}{:02x}", c[0], c[1], c[2], c[3])
    } else {
        format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
    }
}

/// Reads `rgb`, `rrggbb` or (with `alpha`) `rgba`, `rrggbbaa`, with or without one `#`, and
/// spaces around it. The alpha is opaque unless given.
pub fn parse_hex(s: &str, alpha: bool) -> Option<[u8; 4]> {
    let s = s.trim();
    let s = s.strip_prefix('#').unwrap_or(s);
    if !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let digit = |i: usize| s.get(i..i + 1).and_then(|d| u8::from_str_radix(d, 16).ok());
    let pair = |i: usize| s.get(i..i + 2).and_then(|d| u8::from_str_radix(d, 16).ok());
    match (s.len(), alpha) {
        (3, _) => Some([digit(0)? * 17, digit(1)? * 17, digit(2)? * 17, 255]),
        (4, true) => Some([digit(0)? * 17, digit(1)? * 17, digit(2)? * 17, digit(3)? * 17]),
        (6, _) => Some([pair(0)?, pair(2)?, pair(4)?, 255]),
        (8, true) => Some([pair(0)?, pair(2)?, pair(4)?, pair(6)?]),
        _ => None,
    }
}

fn c32(c: [u8; 4]) -> Color32 {
    to_color32(Rgba::new(f32::from(c[0]) / 255.0, f32::from(c[1]) / 255.0, f32::from(c[2]) / 255.0, f32::from(c[3]) / 255.0))
}

/// What the picker shows first: the swatches (Animate's default) or the mixer.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum View {
    #[default]
    Swatches,
    Mixer,
}

/// The colours used lately, newest first, and the popup that added the newest (a drag in the
/// mixer or typing in the hex field changes that one entry instead of adding one per step).
#[derive(Clone, Default)]
struct Recent {
    colours: Vec<[u8; 3]>,
    from: Option<Id>,
}

fn recent_id() -> Id {
    Id::new("anicraft.colour.recent")
}

fn remember(ctx: &egui::Context, popup: Id, c: [u8; 3]) {
    ctx.data_mut(|d| {
        let r: &mut Recent = d.get_temp_mut_or_default(recent_id());
        if r.from == Some(popup) && !r.colours.is_empty() {
            r.colours.remove(0);
        }
        r.colours.retain(|q| *q != c);
        r.colours.insert(0, c);
        r.colours.truncate(RECENT);
        r.from = Some(popup);
    });
}

/// A new popup opening adds its first change as a new recent colour.
pub(crate) fn new_session(ctx: &egui::Context) {
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Recent>(recent_id()).from = None);
}

/// A swatch button for an opaque colour: a click opens the picker. `changed()` when the colour
/// changed this frame. Name it with `Named::named`.
pub fn button(ui: &mut egui::Ui, c: &mut [u8; 3]) -> Response {
    let mut full = [c[0], c[1], c[2], 255];
    let r = button_inner(ui, &mut full, false);
    *c = [full[0], full[1], full[2]];
    r
}

/// A swatch button for a colour with alpha (unmultiplied).
pub fn button_alpha(ui: &mut egui::Ui, c: &mut [u8; 4]) -> Response {
    button_inner(ui, c, true)
}

fn button_inner(ui: &mut egui::Ui, c: &mut [u8; 4], alpha: bool) -> Response {
    let size = ui.spacing().interact_size;
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::new(egui::WidgetType::ColorButton));
    let popup = resp.id.with("colour_popup");
    let open = egui::Popup::is_id_open(ui.ctx(), popup);
    if ui.is_rect_visible(rect) {
        let v = if open { &ui.visuals().widgets.open } else { ui.style().interact(&resp) };
        let rect = rect.expand(v.expansion);
        egui::color_picker::show_color_at(ui.painter(), c32(*c), rect.shrink(1.0));
        ui.painter().rect_stroke(rect, T.rc(), Stroke::new(1.0, if open || resp.hovered() { T.selection } else { T.border }), StrokeKind::Inside);
    }
    if popup_for(ui, &resp, popup, c, alpha, None) {
        resp.mark_changed();
    }
    resp
}

/// Opens the picker under `anchor` when it is clicked, and shows it while open. True when the
/// colour changed. `none`, when given, offers No Colour (true: none); picking a colour turns it
/// off again.
pub fn popup_for(ui: &egui::Ui, anchor: &Response, popup: Id, c: &mut [u8; 4], alpha: bool, none: Option<&mut bool>) -> bool {
    if anchor.clicked() {
        new_session(ui.ctx());
    }
    let mut changed = false;
    let shown = egui::Popup::menu(anchor).id(popup).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        changed = picker(ui, popup, c, alpha, none);
    });
    if let Some(r) = shown {
        name_popup(&r.response, tr!("Colour picker"));
    }
    changed
}

/// Names a picker popup for screen readers: a dialog holding the picker's controls.
pub(crate) fn name_popup(r: &Response, name: &str) {
    r.ctx.accesskit_node_builder(r.id, |node| {
        node.set_role(egui::accesskit::Role::Dialog);
        node.set_label(name);
    });
}

/// The picker itself: swatches or mixer, then the colour, its hex and its alpha.
pub(crate) fn picker(ui: &mut egui::Ui, popup: Id, c: &mut [u8; 4], alpha: bool, none: Option<&mut bool>) -> bool {
    let view_id = Id::new("anicraft.colour.view");
    let mut view: View = ui.data(|d| d.get_temp(view_id)).unwrap_or_default();
    let mut changed = false;
    ui.horizontal(|ui| {
        let before = view;
        ui.selectable_value(&mut view, View::Swatches, tr!("Swatches")).on_hover_text(tr!("The web-safe palette and the colours used lately"));
        ui.selectable_value(&mut view, View::Mixer, tr!("Mixer")).on_hover_text(tr!("Any colour: hue, saturation and brightness"));
        if view != before {
            ui.data_mut(|d| d.insert_temp(view_id, view));
        }
        if let Some(none) = none {
            ui.separator();
            if ui.selectable_label(*none, tr!("No colour")).on_hover_text(tr!("Draw without this colour")).clicked() {
                *none = !*none;
                changed = true;
            }
        }
    });
    ui.add_space(4.0);
    match view {
        View::Swatches => {
            if let Some(p) = swatches(ui, [c[0], c[1], c[2]]) {
                *c = [p[0], p[1], p[2], c[3]];
                changed = true;
            }
        }
        View::Mixer => {
            let mut m = c32(*c);
            ui.spacing_mut().slider_width = (CELL * (COLS + 1) as f32).max(200.0);
            let a = if alpha { egui::color_picker::Alpha::OnlyBlend } else { egui::color_picker::Alpha::Opaque };
            if egui::color_picker::color_picker_color32(ui, &mut m, a) {
                let [r, g, b, al] = m.to_srgba_unmultiplied();
                *c = [r, g, b, if alpha { al } else { 255 }];
                remember(ui.ctx(), popup, [r, g, b]);
                changed = true;
            }
        }
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(CELL * 2.0, CELL * 1.5), Sense::hover());
        egui::color_picker::show_color_at(ui.painter(), c32(*c), r);
        ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0, T.border), StrokeKind::Inside);
        if let Some(h) = hex_field(ui, popup, c, alpha) {
            *c = h;
            remember(ui.ctx(), popup, [h[0], h[1], h[2]]);
            changed = true;
        }
        if alpha {
            let mut pct = f32::from(c[3]) / 255.0 * 100.0;
            let r = with_type(ui, Type::Data, |ui| ui.add(egui::DragValue::new(&mut pct).range(0.0..=100.0).suffix(" %").max_decimals(0))).named(tr!("Alpha"));
            if r.on_hover_text(tr!("Alpha: 0 % is clear, 100 % opaque")).changed() {
                c[3] = (pct.clamp(0.0, 100.0) / 100.0 * 255.0).round() as u8;
                changed = true;
            }
        }
    });
    changed
}

/// The hex field: shows the colour, and takes a typed one as soon as it reads as a colour.
fn hex_field(ui: &mut egui::Ui, popup: Id, c: &[u8; 4], alpha: bool) -> Option<[u8; 4]> {
    let buf_id = popup.with("hex");
    let current = to_hex(*c, alpha);
    let mut s: String = ui.data(|d| d.get_temp(buf_id)).unwrap_or_else(|| current.clone());
    let r = with_type(ui, Type::Data, |ui| ui.add(egui::TextEdit::singleline(&mut s).desired_width(if alpha { 86.0 } else { 66.0 }).char_limit(10)));
    let r = r.named(tr!("Hex colour")).on_hover_text(if alpha { tr!("Hex colour: #rrggbb, or #rrggbbaa with alpha") } else { tr!("Hex colour: #rrggbb") });
    let mut out = None;
    if r.has_focus() || r.lost_focus() {
        if r.changed()
            && let Some(p) = parse_hex(&s, alpha)
            && p != *c
        {
            out = Some(p);
        }
        if r.lost_focus() {
            // Back to the colour it holds (an unreadable entry is dropped).
            ui.data_mut(|d| d.remove::<String>(buf_id));
        } else {
            ui.data_mut(|d| d.insert_temp(buf_id, s));
        }
    } else {
        ui.data_mut(|d| d.remove::<String>(buf_id));
    }
    out
}

/// The document's solid swatches (ADR 0078), as the picker's last rows show them: colour and
/// name.
fn document_colours(ctx: &egui::Context) -> Vec<([u8; 3], String)> {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    crate::swatches_panel::for_pickers(ctx)
        .iter()
        .filter_map(|s| match &s.paint {
            anicraft_engine::doc::SwatchPaint::Solid { color } => Some(([b(color.r), b(color.g), b(color.b)], s.name.clone())),
            _ => None,
        })
        .take((COLS + 1) * DOC_ROWS)
        .collect()
}

/// The palette grid (side column, a gap, the web-safe cube), the recent row under it, and the
/// document's swatches under that. Returns the colour clicked.
fn swatches(ui: &mut egui::Ui, current: [u8; 3]) -> Option<[u8; 3]> {
    let recent: Vec<[u8; 3]> = ui.data(|d| d.get_temp::<Recent>(recent_id())).map(|r| r.colours).unwrap_or_default();
    let doc = document_colours(ui.ctx());
    let doc_rows = doc.len().div_ceil(COLS + 1);
    let gap = CELL * 0.5;
    let width = CELL * (COLS + 1) as f32 + gap;
    let height = CELL * (ROWS + 1) as f32 + gap + if doc_rows > 0 { gap + CELL * (doc_rows + 1) as f32 } else { 0.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    let painter = ui.painter_at(rect.expand(2.0));
    let o = rect.min;
    // Each cell: where it is, and its colour.
    let mut cells: Vec<(Rect, [u8; 3])> = Vec::with_capacity(ROWS * (COLS + 1) + RECENT);
    for (row, side) in SIDE.iter().enumerate() {
        let y = o.y + CELL * row as f32;
        cells.push((Rect::from_min_size(pos2(o.x, y), vec2(CELL, CELL)), *side));
        for col in 0..COLS {
            if let Some(c) = web_safe(row, col) {
                cells.push((Rect::from_min_size(pos2(o.x + CELL + gap + CELL * col as f32, y), vec2(CELL, CELL)), c));
            }
        }
    }
    let ry = o.y + CELL * ROWS as f32 + gap;
    for (i, c) in recent.iter().enumerate().take(RECENT) {
        cells.push((Rect::from_min_size(pos2(o.x + CELL * i as f32 + if i > 0 { gap } else { 0.0 }, ry), vec2(CELL, CELL)), *c));
    }
    // The document's swatches under their caption, a row at a time (names show on hover).
    let dy = ry + CELL + gap + CELL;
    let first_doc = cells.len();
    for (i, (c, _)) in doc.iter().enumerate() {
        cells.push((Rect::from_min_size(pos2(o.x + CELL * (i % (COLS + 1)) as f32, dy + CELL * (i / (COLS + 1)) as f32), vec2(CELL, CELL)), *c));
    }
    // The palette's own grid lines are the gaps between cells, in the panel's border colour.
    let grid = Rect::from_min_size(o, vec2(width, CELL * ROWS as f32));
    painter.rect_filled(grid, 0.0, T.border);
    for (r, c) in &cells {
        painter.rect_filled(r.shrink(0.5), 0.0, c32([c[0], c[1], c[2], 255]));
    }
    if !doc.is_empty() {
        painter.text(pos2(o.x, dy - CELL * 0.5), egui::Align2::LEFT_CENTER, tr!("Document swatches (Window › Swatches)"), crate::theme::font(Type::Caption), T.text_faint);
    }
    if recent.is_empty() {
        painter.text(pos2(o.x, ry + CELL * 0.5), egui::Align2::LEFT_CENTER, tr!("Colours you mix or type show here"), crate::theme::font(Type::Caption), T.text_faint);
    }
    let hit = |p: Pos2| cells.iter().find(|(r, _)| r.contains(p)).copied();
    if let Some((r, _)) = cells.iter().find(|(_, c)| *c == current) {
        painter.rect_stroke(r.expand(1.0), 0.0, Stroke::new(1.5, T.on_active), StrokeKind::Middle);
    }
    let hovered = resp.hover_pos().and_then(|p| cells.iter().position(|(r, _)| r.contains(p)));
    if let Some((r, c)) = hovered.and_then(|i| cells.get(i)).copied() {
        painter.rect_stroke(r.expand(1.0), 0.0, Stroke::new(1.5, T.selection), StrokeKind::Middle);
        let hex = to_hex([c[0], c[1], c[2], 255], false);
        let name = hovered.and_then(|i| i.checked_sub(first_doc)).and_then(|i| doc.get(i)).map(|d| d.1.as_str()).filter(|n| !n.is_empty());
        let tip = match name {
            Some(n) => tr!("{name} ({hex}), a document swatch", name = n, hex = hex.as_str()),
            None if hovered.is_some_and(|i| i >= first_doc) => tr!("{hex}, a document swatch", hex = hex.as_str()),
            None => hex,
        };
        resp.clone().on_hover_text(text(Type::Data, tip));
    }
    // One control for the whole grid (a click picks the cell under the pointer).
    crate::a11y::control(ui, &resp, egui::WidgetType::Button, if doc.is_empty() { tr!("Colour swatches") } else { tr!("Colour swatches and the document's swatches") }, None);
    if resp.clicked() { resp.interact_pointer_pos().and_then(hit).map(|(_, c)| c) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_reads_short_long_and_alpha_forms() {
        assert_eq!(parse_hex("#ff8000", false), Some([255, 128, 0, 255]));
        assert_eq!(parse_hex(" FF8000 ", false), Some([255, 128, 0, 255]));
        assert_eq!(parse_hex("#f80", false), Some([255, 136, 0, 255]));
        assert_eq!(parse_hex("#ff800080", true), Some([255, 128, 0, 128]));
        assert_eq!(parse_hex("#f808", true), Some([255, 136, 0, 136]));
        for bad in ["", "#", "#ff80", "#ff800080", "##ff8000", "#gg0000", "#+f0000", "#ff 800", "#ff8000ff00"] {
            assert_eq!(parse_hex(bad, false), None, "{bad:?}");
        }
        assert_eq!(parse_hex("#ff80008", true), None);
        // What the field shows reads back as the same colour.
        for c in [[0, 0, 0, 255], [18, 52, 86, 255], [255, 255, 255, 7]] {
            assert_eq!(parse_hex(&to_hex(c, true), true), Some(c));
        }
        assert_eq!(to_hex([1, 2, 3, 4], false), "#010203", "no alpha where the colour has none");
    }

    /// #248: the 216 web-safe colours, each once, black at the top left and white where the
    /// last block ends (the open standard's cube).
    #[test]
    fn the_palette_is_the_web_safe_cube() {
        let mut all: Vec<[u8; 3]> = (0..ROWS).flat_map(|r| (0..COLS).filter_map(move |c| web_safe(r, c))).collect();
        assert_eq!(all.len(), 216);
        assert!(all.iter().all(|c| c.iter().all(|v| STEPS.contains(v))));
        assert_eq!(web_safe(0, 0), Some([0, 0, 0]));
        assert_eq!(web_safe(11, 17), Some([255, 255, 255]));
        assert_eq!(web_safe(0, 6), Some([0x33, 0, 0]), "red steps across the blocks");
        assert_eq!(web_safe(1, 0), Some([0, 0x33, 0]), "green down a block");
        assert_eq!(web_safe(0, 1), Some([0, 0, 0x33]), "blue across it");
        assert_eq!(web_safe(12, 0), None);
        assert_eq!(web_safe(0, 18), None);
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), 216, "no colour twice");
    }

    #[test]
    fn recent_colours_keep_one_entry_per_popup_session() {
        let ctx = egui::Context::default();
        let (a, b) = (Id::new("a"), Id::new("b"));
        let list = |ctx: &egui::Context| ctx.data(|d| d.get_temp::<Recent>(recent_id())).map(|r| r.colours).unwrap_or_default();
        // A drag in the mixer: one entry, the last colour.
        for v in [10, 20, 30] {
            remember(&ctx, a, [v, 0, 0]);
        }
        assert_eq!(list(&ctx), vec![[30, 0, 0]]);
        // Another popup adds one, newest first; the same colour again moves to the front.
        remember(&ctx, b, [0, 0, 9]);
        new_session(&ctx);
        remember(&ctx, b, [30, 0, 0]);
        assert_eq!(list(&ctx), vec![[30, 0, 0], [0, 0, 9]]);
        new_session(&ctx);
        for v in 0..40u8 {
            new_session(&ctx);
            remember(&ctx, a, [v, v, v]);
        }
        assert_eq!(list(&ctx).len(), RECENT, "bounded");
    }
}
