//! Timeline panel: transport, layer list and a virtualised frame grid.

use crate::app::{App, BottomTab};
use crate::icons::{self, Icon};
use crate::menus;
use crate::theme::{SQUARE, T, TARGET, Type, color, font, text, to_color32, with_type};
use anicraft_engine::doc::{Easing, Layer, LayerId, LayerKind, TweenKind};
use crate::a11y::{self, Named};
use egui::{Align2, Color32, Rect, Sense, Stroke, StrokeKind, WidgetType, pos2, vec2};
use serde_json::json;
use std::sync::Arc;

const LAYER_W: f32 = 252.0;
/// Rows are click targets (≥ `theme::TARGET`).
const ROW_H: f32 = TARGET;
const RULER_H: f32 = 24.0;
/// Frame cell width at 100 % zoom, and the zoom range (Ctrl+wheel over the frames, or the
/// slider in the timeline header).
pub const CELL_DEFAULT: f32 = 12.0;
pub const CELL_RANGE: std::ops::RangeInclusive<f32> = 3.0..=48.0;
/// Width of the playhead's number flag (wider only for long frame numbers).
const HEAD_W: f32 = 22.0;

/// Room before frame 1: what the playhead's flag needs when centred on frame 1, plus `extra`
/// points. Fixed on screen: it shrinks to a hairline once frames are wider than the flag, and
/// never grows with the zoom.
fn start_pad(cell_w: f32, extra: f32) -> f32 {
    (HEAD_W * 0.5 - cell_w * 0.5).max(0.0) + 1.0 + extra
}

/// The pixel column through the middle of a frame whose cell starts at `cell_left`: ruler
/// ticks, their numbers and the playhead line all use it, so they always line up.
fn frame_center(cell_left: f32, cell_w: f32) -> f32 {
    (cell_left + cell_w * 0.5).round() + 0.5
}
const FOOTER_H: f32 = 38.0;
/// The bottom bar's frame zoom and Frame View at its right end: the 56 pt slider, its field, the
/// menu button and spacing.
const ZOOM_W: f32 = 168.0;
/// Frame View › Short (ADR 0061): row height (points). Layers 200 % and 300 % tall are that
/// many short rows.
const SHORT_ROW_H: f32 = 20.0;
/// Frame View's frame widths, Tiny to Large, as frame cell widths (the zoom slider sets any
/// width in between).
pub const FRAME_WIDTHS: [(&str, f32); 5] = [(tr_noop!("Tiny"), 4.0), (tr_noop!("Small"), 8.0), (tr_noop!("Normal"), CELL_DEFAULT), (tr_noop!("Medium"), 18.0), (tr_noop!("Large"), 28.0)];

/// A normal row's height: full, or Frame View › Short.
pub fn base_row_h(short: bool) -> f32 {
    if short { SHORT_ROW_H } else { ROW_H }
}

/// How many whole rows of heights `h` fit in `avail` points from row `first`: at least one
/// while any are left (a tall row in a small dock is cut off rather than hidden).
fn rows_fitting(h: &[f32], first: usize, avail: f32) -> usize {
    let (mut used, mut n) = (0.0, 0);
    for x in h.iter().skip(first) {
        if n > 0 && used + x > avail + 0.5 {
            break;
        }
        used += x;
        n += 1;
    }
    n
}

/// The scroll limit: the first row from which every row to the end fits.
fn max_first_row(h: &[f32], avail: f32) -> usize {
    first_showing(h, h.len().saturating_sub(1), avail)
}

/// The first row to show so that row `r` is the last one that fits (the least scroll that
/// brings it into view from above).
fn first_showing(h: &[f32], r: usize, avail: f32) -> usize {
    let mut used = 0.0;
    for i in (0..=r.min(h.len().saturating_sub(1))).rev() {
        used += h.get(i).copied().unwrap_or(0.0);
        if used > avail + 0.5 {
            return (i + 1).min(r);
        }
    }
    0
}

/// The row `dy` points below the top of row `first` (negative: above it), within the rows.
fn row_at(h: &[f32], first: usize, dy: f32) -> usize {
    if dy < 0.0 {
        let mut y = 0.0;
        for i in (0..first.min(h.len())).rev() {
            y -= h.get(i).copied().unwrap_or(0.0);
            if dy >= y {
                return i;
            }
        }
        return 0;
    }
    let mut y = 0.0;
    for (i, x) in h.iter().enumerate().skip(first) {
        y += x;
        if dy < y {
            return i;
        }
    }
    h.len().saturating_sub(1)
}
/// Eye / lock / outline toggles: square targets in each layer row.
const TOGGLE: f32 = 24.0;
/// Indent per folder or mask level in the layer list.
const INDENT: f32 = 14.0;
/// The selected-artwork square after a layer's name (points).
const SEL_MARK: f32 = 7.0;

/// A rectangle of frames: the layers between the anchor's and the head's rows (in the
/// current layer order) by the frames between them. View state: commands get explicit ranges.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameSel {
    pub anchor: (LayerId, u32),
    pub head: (LayerId, u32),
}

impl FrameSel {
    pub fn cell(layer: LayerId, frame: u32) -> FrameSel {
        FrameSel { anchor: (layer, frame), head: (layer, frame) }
    }
    /// First and last selected frame.
    pub fn frames(&self) -> (u32, u32) {
        (self.anchor.1.min(self.head.1), self.anchor.1.max(self.head.1))
    }
    /// Selected rows (indices into the layer list), or `None` when a layer went away.
    fn rows(&self, layers: &[Arc<Layer>]) -> Option<(usize, usize)> {
        let a = layers.iter().position(|l| l.id == self.anchor.0)?;
        let h = layers.iter().position(|l| l.id == self.head.0)?;
        Some((a.min(h), a.max(h)))
    }
    pub fn layers(&self, layers: &[Arc<Layer>]) -> Vec<LayerId> {
        self.rows(layers).map(|(a, b)| layers.iter().skip(a).take(b - a + 1).map(|l| l.id).collect()).unwrap_or_default()
    }
    fn contains(&self, layers: &[Arc<Layer>], row: usize, frame: u32) -> bool {
        let (f0, f1) = self.frames();
        self.rows(layers).is_some_and(|(a, b)| row >= a && row <= b) && frame >= f0 && frame <= f1
    }
    /// Whether any selected layer has a keyframe starting in the selected frames.
    pub fn has_keys(&self, layers: &[Arc<Layer>]) -> bool {
        let (f0, f1) = self.frames();
        let ids = self.layers(layers);
        layers.iter().filter(|l| ids.contains(&l.id)).any(|l| l.keyframes.iter().any(|k| k.start >= f0 && k.start <= f1))
    }
}

/// In-progress pointer gesture on the frame grid.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum TlDrag {
    #[default]
    None,
    /// Dragging out a frame selection.
    Select,
    /// Dragging the selected keyframes: grabbed at frame `grab`, currently `by` frames away.
    Move { grab: u32, by: i64, grab_row: usize, rows_by: i64 },
    /// Dragging the end of the span `start..end` on `layer` to `to`.
    Span { layer: LayerId, start: u32, end: u32, to: u32 },
}

/// Range params for the frame commands when frames are selected: `{layers, frame, count}`.
pub fn range_params(app: &App) -> Option<serde_json::Value> {
    let sel = app.frame_sel?;
    let layers = sel.layers(&app.s.timeline().layers);
    if layers.is_empty() {
        return None;
    }
    let (f0, f1) = sel.frames();
    let ids: Vec<u64> = layers.iter().map(|l| l.0).collect();
    Some(json!({"layers": ids, "frame": f0, "count": f1 - f0 + 1}))
}

/// Frame commands that act on the frame selection.
pub const RANGE_COMMANDS: [&str; 7] = ["timeline.insert_frame", "timeline.remove_frame", "timeline.insert_keyframe", "timeline.insert_blank_keyframe", "timeline.clear_keyframe", "timeline.copy_frames", "timeline.cut_frames"];

/// Params for Paste Frames: the selection's top layer and first frame.
pub fn paste_params(app: &App) -> Option<serde_json::Value> {
    let sel = app.frame_sel?;
    let top = *sel.layers(&app.s.timeline().layers).first()?;
    Some(json!({"layer": top.0, "frame": sel.frames().0}))
}

/// An icon button: a full-size target around a Lucide glyph, named by its tooltip.
/// `on` is a toggle's state (`None` for a plain button); a toggle that is on is filled.
pub fn icon_button(ui: &mut egui::Ui, tip: &str, on: Option<bool>, icon: Icon) -> egui::Response {
    let active = on == Some(true);
    let (rect, resp) = ui.allocate_exact_size(vec2(TARGET + 4.0, TARGET), Sense::click());
    // Same language as the tools: a toggle that is on is a lighter gray block, icon white.
    let bg = if active {
        T.active
    } else if resp.hovered() {
        T.header
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, T.rc(), bg);
    icons::paint(ui.painter(), Rect::from_center_size(rect.center(), vec2(16.0, 16.0)), icon, if active { T.on_active } else if resp.hovered() { T.icon_hot } else { T.icon });
    a11y::control(ui, &resp, WidgetType::Button, tip, on);
    resp.on_hover_text(tip)
}

fn transport(app: &mut App, ui: &mut egui::Ui) {
    // Named with the keys in effect (the user may have changed them).
    let step = |app: &mut App, ui: &mut egui::Ui, name: &str, id: &str, icon: Icon| {
        if icon_button(ui, &a11y::with_key(name, app.keymap.primary(id)), None, icon).clicked() {
            app.run(id, json!({}));
        }
    };
    step(app, ui, tr!("First frame"), "timeline.first_frame", Icon::First);
    step(app, ui, tr!("Previous frame"), "timeline.prev_frame", Icon::Prev);
    let playing = app.playing;
    if icon_button(ui, &a11y::with_key(tr!("Play / Stop"), Some("Enter")), Some(playing), if playing { Icon::Pause } else { Icon::Play }).clicked() {
        app.toggle_play();
    }
    step(app, ui, tr!("Next frame"), "timeline.next_frame", Icon::Next);
    step(app, ui, tr!("Last frame"), "timeline.last_frame", Icon::Last);
    let looping = app.looping;
    if icon_button(ui, tr!("Loop playback"), Some(looping), Icon::Loop).clicked() {
        app.looping = !app.looping;
    }
    crate::onion::buttons(app, ui);
    // Add Camera, as Animate has it on the timeline: on while the scene has a camera; a click
    // then takes it away (asking first when the camera moves; undoable).
    let has = app.s.timeline().camera_layer().is_some();
    if icon_button(ui, if has { tr!("Remove camera") } else { tr!("Add camera") }, Some(has), Icon::Camera).clicked() {
        if has {
            app.request("camera.remove", json!({}));
        } else if app.run("camera.add", json!({})).is_some() {
            app.tool = crate::app::Tool::Camera;
            app.last_tool = crate::app::Tool::Camera;
            app.status_ok(tr!("Added a camera. Drag on the stage to pan it, turn it with the round handle, zoom with the wheel. {undo} to undo.", undo = app.undo_key()));
        }
    }
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let dock = ui.max_rect();
    // ---- header ---------------------------------------------------------------------------
    egui::Frame::NONE.fill(T.panel_alt).inner_margin(egui::Margin::symmetric(10, 5)).show(ui, |ui| {
        ui.horizontal(|ui| {
            // The whole width is the header's (frame zoom and Frame View sit in the bottom bar,
            // as in Animate). What doesn't fit scrolls sideways (Shift+wheel) in a small window
            // or at a large interface scale, instead of being cut off.
            let w = ui.available_width();
            let out = egui::ScrollArea::horizontal().id_salt("tl_header").max_width(w).show(ui, |ui| ui.horizontal(|ui| header(app, ui, dock)));
            app.tl_header_spill = out.content_size.x - out.inner_rect.width();
        });
    });

    // A frame selection lives while the playhead is inside it and its layers exist.
    if let Some(sel) = app.frame_sel {
        let (f0, f1) = sel.frames();
        let gone = sel.layers(&app.s.timeline().layers).is_empty();
        if gone || app.playing || (app.tl_drag == TlDrag::None && !(f0..=f1).contains(&app.s.frame)) {
            app.frame_sel = None;
        }
    }

    if app.bottom == BottomTab::MotionEditor {
        crate::motion_editor::show(app, ui);
        return;
    }

    // ---- body -----------------------------------------------------------------------------
    let body = ui.available_rect_before_wrap();
    ui.allocate_rect(body, Sense::hover());
    let painter = ui.painter_at(body);
    painter.rect_filled(body, 0.0, T.panel);

    let grid = Rect::from_min_max(pos2(body.left() + LAYER_W, body.top()), body.right_bottom());
    let frames_total = app.s.frame_count();
    app.tl_cell_w = app.tl_cell_w.clamp(*CELL_RANGE.start(), *CELL_RANGE.end());
    // Ctrl+wheel (or pinch) over the frames zooms about the pointer: the frame under it stays put.
    // Room before frame 1: exactly what the playhead's flag needs to be drawn whole when it
    // sits on frame 1 (the flag is centred on the frame), plus an optional extra gap in points
    // (a preference). Fixed on screen: it does not grow with the zoom.
    let pad_for = |cw: f32| start_pad(cw, app.prefs.timeline_start_gap);
    if ui.rect_contains_pointer(body) {
        let (zoom, hover) = ui.input(|i| (i.zoom_delta(), i.pointer.hover_pos()));
        if (zoom - 1.0).abs() > 1e-4
            && let Some(h) = hover.filter(|h| h.x >= grid.left())
        {
            let old = app.tl_cell_w;
            let new = (old * zoom).clamp(*CELL_RANGE.start(), *CELL_RANGE.end());
            let under = (h.x - grid.left() - pad_for(old)) / old + app.tl_scroll;
            app.tl_cell_w = new;
            app.tl_scroll = (under - (h.x - grid.left() - pad_for(new)) / new).max(0.0);
        }
    }
    // A button open (ADR 0057): its four frames are named Up, Over, Down and Hit, as in Animate,
    // and drawn wide enough for their names; the zoom it had comes back when it closes.
    let states = button_states(app);
    match (states, app.tl_button_zoom) {
        (Some(names), None) => {
            let widest = names.iter().map(|n| painter.layout_no_wrap(n.to_string(), font(Type::Caption), T.text).rect.width()).fold(0.0f32, f32::max);
            let set = app.tl_cell_w.max((widest + 8.0).min(*CELL_RANGE.end()));
            app.tl_button_zoom = Some((app.tl_cell_w, set));
            app.tl_cell_w = set;
        }
        (None, Some((before, set))) => {
            if (app.tl_cell_w - set).abs() < 1e-3 {
                app.tl_cell_w = before;
            }
            app.tl_button_zoom = None;
        }
        _ => {}
    }
    let cell_w = app.tl_cell_w;
    // Dead space before frame 1 (a preference): the playhead's flag never touches the edge.
    let pad = pad_for(cell_w);
    let visible_cols = ((grid.width() / cell_w).ceil() as u32).max(1);
    // Scrolling, as in a web browser: the wheel scrolls the layers up and down; Shift+wheel and
    // sideways wheels / trackpad swipes scroll the frames (egui turns Shift+wheel into a
    // horizontal delta). Anywhere over the timeline, rows and cells included. Only visible
    // rows and columns are painted, so cost is independent of document size.
    // Rows are the layers not inside a closed folder (`rows[r]` is row r's layer index).
    let rows: Vec<usize> = {
        let tl = app.s.timeline();
        (0..tl.layers.len()).filter(|&i| !app.tl_collapsed.iter().any(|f| tl.is_inside(i, *f))).collect()
    };
    let n_layers = rows.len();
    // Each row is its layer's height (100, 200 or 300 % of a normal or short row, ADR 0061).
    let base_h = base_row_h(app.prefs.timeline_short_rows);
    let heights: Vec<f32> = {
        let tl = app.s.timeline();
        rows.iter().map(|&i| f32::from(tl.layers.get(i).map_or(1, |l| l.height())) * base_h).collect()
    };
    let avail = (body.height() - RULER_H - FOOTER_H).max(0.0);
    if ui.rect_contains_pointer(body) {
        let d = ui.input(|i| i.smooth_scroll_delta);
        if d.x != 0.0 {
            app.tl_scroll = (app.tl_scroll - d.x / cell_w).max(0.0);
        }
        if d.y != 0.0 {
            app.tl_vscroll = (app.tl_vscroll - d.y / base_h).max(0.0);
        }
    }
    app.tl_vscroll = app.tl_vscroll.min(max_first_row(&heights, avail) as f32);
    if app.tl_last_layer != Some(app.s.layer)
        && let Some(ai) = app.s.active_layer_index().and_then(|li| rows.iter().position(|&r| r == li))
    {
        // the active layer changed (e.g. picked on the stage): bring its row into view
        let first = app.tl_vscroll.floor() as usize;
        if ai < first {
            app.tl_vscroll = ai as f32;
        } else if ai >= first + rows_fitting(&heights, first, avail) {
            app.tl_vscroll = first_showing(&heights, ai, avail) as f32;
        }
        app.tl_last_layer = Some(app.s.layer);
    }
    let first_row = app.tl_vscroll.floor() as usize;
    let rows_visible = rows_fitting(&heights, first_row, avail);
    let max_scroll = (frames_total + 30).saturating_sub(visible_cols) as f32;
    app.tl_scroll = app.tl_scroll.min(max_scroll.max(0.0));
    // keep the playhead visible while playing
    if app.playing {
        let f = app.s.frame as f32;
        if f < app.tl_scroll || f >= app.tl_scroll + visible_cols as f32 - 2.0 {
            app.tl_scroll = (f - 2.0).max(0.0);
        }
    }
    let scroll = app.tl_scroll;
    let first = scroll.floor() as u32;
    let x_of = |f: u32| grid.left() + pad + (f as f32 - scroll) * cell_w;
    // The padding is dead space: a click there means frame 1, never a frame before it.
    let frame_at = |x: f32| ((x - grid.left() - pad) / cell_w + scroll).floor().max(0.0) as u32;
    let gp = painter.with_clip_rect(grid);

    // ruler: numbers every `step` frames, chosen so labels never collide at any zoom
    let ruler = Rect::from_min_max(grid.left_top(), pos2(grid.right(), grid.top() + RULER_H));
    painter.rect_filled(ruler, 0.0, T.panel_alt);
    painter.rect_filled(Rect::from_min_max(body.left_top(), pos2(grid.left(), ruler.bottom())), 0.0, T.panel_alt);
    painter.text(pos2(body.left() + 10.0, ruler.center().y), Align2::LEFT_CENTER, tr!("Layers"), font(Type::Heading), color(Type::Heading));
    let step = [1u32, 2, 5, 10, 20, 50, 100, 200, 500].into_iter().find(|s| *s as f32 * cell_w >= 28.0).unwrap_or(1000);
    let minor = if cell_w >= 6.0 { 1 } else { step / 5 }.max(1);
    for f in first..first + visible_cols + 1 {
        let x = x_of(f);
        let n = f + 1;
        // Ticks and numbers sit on the frame's centre, where the playhead line and the
        // keyframe dots are, so "1" is exactly under the playhead on frame 1 at any zoom.
        let cx = frame_center(x, cell_w);
        if let Some(name) = states.and_then(|s| s.get(f as usize).copied()) {
            // A button's state frame: its name in place of the number, the Hit frame underlined
            // in the colour its area is drawn in on the stage.
            let cell = Rect::from_min_max(pos2(x, ruler.top()), pos2(x + cell_w, ruler.bottom()));
            gp.with_clip_rect(cell.intersect(grid)).text(pos2(cx, ruler.center().y - 1.0), Align2::CENTER_CENTER, name, font(Type::Caption), color(Type::Caption));
            if f == 3 {
                gp.line_segment([pos2(x + 2.0, ruler.bottom() - 1.5), pos2(x + cell_w - 2.0, ruler.bottom() - 1.5)], Stroke::new(2.0, T.hit_area));
            }
        } else if n % step == 0 || n == 1 {
            gp.line_segment([pos2(cx, ruler.bottom() - 6.0), pos2(cx, ruler.bottom())], Stroke::new(1.0, T.text_faint));
            gp.text(pos2(cx, ruler.top() + 3.0), Align2::CENTER_TOP, n.to_string(), font(Type::DataSmall), color(Type::DataSmall));
        } else if n % minor == 0 {
            gp.line_segment([pos2(cx, ruler.bottom() - 3.0), pos2(cx, ruler.bottom())], Stroke::new(1.0, T.border));
        }
    }
    let ruler_resp = ui.interact(ruler, ui.id().with("ruler"), a11y::pointer_only());
    let playhead = f64::from(app.s.frame + 1);
    ruler_resp.widget_info(|| egui::WidgetInfo::slider(true, playhead, tr!("Playhead (frame ruler)")));
    if (ruler_resp.clicked() || ruler_resp.dragged())
        && app.onion_ui.drag.is_none()
        && let Some(p) = ruler_resp.interact_pointer_pos() {
            app.playing = false;
            app.run("timeline.goto", json!({"frame": frame_at(p.x)}));
        }
    // The onion menu on a right-click, as the markers live here (ADR 0047).
    ruler_resp.context_menu(|ui| crate::onion::menu(app, ui));
    // What a button's state frame is for, under the pointer.
    if let Some(f) = states.and(ruler_resp.hover_pos()).map(|p| frame_at(p.x)).filter(|f| *f < 4) {
        let tip = match f {
            0 => tr!("Up: the button as it first shows"),
            1 => tr!("Over: the button with the pointer over it"),
            2 => tr!("Down: the button while it is pressed"),
            _ => tr!("Hit: the area that answers the pointer, never shown (drawn over the other states while you edit them)"),
        };
        ruler_resp.clone().on_hover_text(tip);
    }
    // Onion markers over the ruler (they take the pointer before it).
    crate::onion::ruler(app, ui, ruler, cell_w, x_of, frame_at);

    // rows
    let layers = app.s.timeline().layers.clone();
    let active = app.s.layer;
    // Layers with selected artwork show a square in their colour after the name, as in
    // Animate: a selection can span layers (ADR 0044).
    let holding = app.s.selection.layers();
    let rows_top = ruler.bottom();
    let rows_bottom = body.bottom() - FOOTER_H;
    let tl_now = app.s.timeline().clone();
    let mut next_y = rows_top;
    // Where each row is, for files dropped on a layer's frame (ADR 0065).
    let mut drop_rows = Vec::new();
    for (r, &i) in rows.iter().enumerate().skip(first_row) {
        let Some(layer) = layers.get(i) else { continue };
        let y = next_y;
        let rh = heights.get(r).copied().unwrap_or(base_h);
        // Whole rows only: a cut-off row would sit under the footer (scroll for the rest). The
        // first row always shows, however tall.
        if r > first_row && y + rh > rows_bottom + 0.5 {
            break;
        }
        next_y += rh;
        let row = Rect::from_min_size(pos2(body.left(), y), vec2(body.width(), rh));
        drop_rows.push((row, layer.id));
        let left = Rect::from_min_max(row.left_top(), pos2(grid.left(), row.bottom()));
        let is_active = layer.id == active;
        painter.rect_filled(left, 0.0, if is_active { T.accent_dim } else if r % 2 == 0 { T.panel } else { T.panel_alt });
        // The layer's colour as a full-height edge bar.
        painter.rect_filled(Rect::from_min_size(left.left_top(), vec2(4.0, rh)), 0.0, to_color32(layer.color));
        // Layers are indented under the folders and masks that hold them. Folders carry a
        // disclosure chevron and a folder glyph; masks a stencil; masked layers its inside.
        let is_folder = layer.kind == LayerKind::Folder;
        let masked = layer.parent.is_some_and(|p| tl_now.layer(p).is_some_and(|m| m.kind == LayerKind::Mask));
        let guided = layer.parent.is_some_and(|p| tl_now.layer(p).is_some_and(|m| m.kind == LayerKind::Guide));
        let hidden = !layer.visible || tl_now.hidden_by_folder(i);
        let name_color = if hidden { T.text_faint } else { T.text };
        let type_icon = |x: f32, icon: Icon| icons::paint(&painter, Rect::from_center_size(pos2(x, left.center().y), vec2(14.0, 14.0)), icon, name_color);
        let x0 = left.left() + 12.0 + tl_now.depth(i) as f32 * INDENT;
        let open = !app.tl_collapsed.contains(&layer.id);
        // Toggles and targets fit a short row.
        let tg = TOGGLE.min(base_h - 2.0);
        let chevron = Rect::from_center_size(pos2(x0 + 6.0, left.center().y), vec2(tg, tg));
        // Every layer has its type's icon; double-click it for Layer Properties (ADR 0061).
        let (icon_x, name_x, icon) = match layer.kind {
            LayerKind::Folder => (x0 + 24.0, x0 + 37.0, if open { Icon::FolderOpen } else { Icon::Folder }),
            LayerKind::Mask => (x0 + 7.0, x0 + 20.0, Icon::Mask),
            LayerKind::Normal if masked => (x0 + 7.0, x0 + 20.0, Icon::Masked),
            LayerKind::Guide => (x0 + 7.0, x0 + 20.0, Icon::Guide),
            LayerKind::Camera => (x0 + 7.0, x0 + 20.0, Icon::Camera),
            LayerKind::Armature => (x0 + 7.0, x0 + 20.0, Icon::Bone),
            LayerKind::Normal => (x0 + 7.0, x0 + 20.0, Icon::Layer),
        };
        if is_folder {
            icons::paint(&painter, Rect::from_center_size(chevron.center(), vec2(12.0, 12.0)), if open { Icon::ChevronDown } else { Icon::ChevronRight }, T.text_dim);
        }
        type_icon(icon_x, icon);
        let icon_rect = Rect::from_center_size(pos2(icon_x, left.center().y), vec2(16.0, tg));
        let name_at = painter.text(pos2(name_x, left.center().y), Align2::LEFT_CENTER, &layer.name, font(Type::Body), name_color);
        let has_selection = holding.contains(&layer.id);
        if has_selection {
            let sq = Rect::from_center_size(pos2(name_at.right() + 9.0, left.center().y), vec2(SEL_MARK, SEL_MARK));
            painter.rect_filled(sq, 0.0, to_color32(layer.color));
            painter.rect_stroke(sq, 0.0, Stroke::new(1.0, T.text), StrokeKind::Outside);
        }

        // Onion / eye / lock / outline toggles, right to left from the outline square: square
        // targets with a hover fill (smaller on short rows).
        let toggle = |k: f32| Rect::from_center_size(pos2(left.right() - 16.0 - k * (TOGGLE + 2.0), left.center().y), vec2(tg, tg));
        let (eye, lock, outl) = (toggle(2.0), toggle(1.0), toggle(0.0));
        // The camera has no artwork to outline or onion-skin: its row has the eye (camera on or
        // off) and lock. A folder has no frames to onion-skin.
        let is_camera = layer.kind == LayerKind::Camera;
        let onion_t = (!is_camera && !is_folder).then(|| toggle(3.0));
        // A layer with sounds has a speaker before the others: Mute (ADR 0039).
        let speaker = layer.has_sounds().then(|| toggle(if onion_t.is_some() { 4.0 } else { 3.0 }));
        let hover = ui.input(|inp| inp.pointer.hover_pos());
        for r in [eye, lock, outl].into_iter().take(if is_camera { 2 } else { 3 }).chain(onion_t).chain(speaker) {
            if hover.is_some_and(|h| r.contains(h)) {
                painter.rect_filled(r, T.rc(), T.hover);
            }
        }
        let c_on = T.text;
        let c_off = T.text_faint;
        // eye and lock (state shows in the glyph and its brightness)
        let glyph = |r: Rect| {
            let s = r.height().min(15.0);
            Rect::from_center_size(r.center(), vec2(s, s))
        };
        icons::paint(&painter, glyph(eye), if layer.visible { Icon::Eye } else { Icon::EyeOff }, if layer.visible { c_on } else { c_off });
        icons::paint(&painter, glyph(lock), if layer.locked { Icon::Lock } else { Icon::Unlock }, if layer.locked { c_on } else { c_off });
        if let Some(sp) = speaker {
            icons::paint(&painter, glyph(sp), if layer.muted { Icon::SoundOff } else { Icon::Sound }, if layer.muted { T.accent_text } else { c_off });
        }
        // Onion skin for this layer (ADR 0061): bright while onion skin is on and draws it,
        // faint while onion skin is off, struck through when the layer is left out.
        if let Some(ot) = onion_t {
            let g = glyph(ot);
            icons::paint(&painter, g, Icon::Onion, if layer.onion && app.onion.ghosts { c_on } else { c_off });
            if !layer.onion {
                painter.line_segment([g.left_bottom(), g.right_top()], Stroke::new(1.5, c_off));
            }
        }
        // outline swatch
        let sw = Rect::from_center_size(outl.center(), vec2(10.0, 10.0));
        if is_camera {
            // The swatch shows the frame's colour on the stage.
            painter.rect_filled(sw.shrink(2.0), 0.0, to_color32(layer.color));
        } else if layer.outline {
            painter.rect_stroke(sw, 0.0, Stroke::new(1.5, to_color32(layer.color)), StrokeKind::Inside);
        } else {
            painter.rect_filled(sw, 0.0, to_color32(layer.color));
        }
        let name_rect = Rect::from_min_max(left.left_top(), pos2(speaker.or(onion_t).unwrap_or(eye).left() - 4.0, left.bottom()));
        let nr = ui.interact(name_rect, ui.id().with(("lname", layer.id.0)), Sense::click());
        // Screen readers hear the row as a selectable item, "Ball · layer", then its toggles.
        let name = &layer.name;
        let row_name = match layer.kind {
            LayerKind::Folder => tr!("{name} · folder", name = name),
            LayerKind::Mask => tr!("{name} · mask layer", name = name),
            LayerKind::Guide => tr!("{name} · guide layer", name = name),
            LayerKind::Camera => tr!("{name} · camera layer", name = name),
            LayerKind::Armature => tr!("{name} · armature layer", name = name),
            LayerKind::Normal if masked => tr!("{name} · masked layer", name = name),
            LayerKind::Normal => tr!("{name} · layer", name = name),
        };
        let row_name = if has_selection { tr!("{row} · has selected artwork", row = row_name) } else { row_name };
        a11y::control(ui, &nr, WidgetType::SelectableLabel, &row_name, Some(is_active));
        // Ctrl+F6 lands on the active layer, or on the top row shown when it is scrolled away
        // (rows are drawn top down, so the active row, when drawn, has the last word).
        if is_active || r == first_row {
            a11y::anchor(ui, a11y::Region::Timeline, dock, &nr);
        }
        if is_active && app.tl_refocus {
            app.tl_refocus = false;
            // (Asking again would reset the focus filter that keeps the arrows here.)
            if !nr.has_focus() {
                nr.request_focus();
            }
        }
        for (r, id, salt, label, on) in [(eye, "layer.toggle_visible", "eye", tr!("Show {name}", name = name), layer.visible), (lock, "layer.toggle_lock", "lock", tr!("Lock {name}", name = name), layer.locked), (outl, "layer.toggle_outline", "outline", tr!("Outline {name}", name = name), layer.outline)].into_iter().take(if is_camera { 2 } else { 3 }) {
            let rr = ui.interact(r, ui.id().with((salt, layer.id.0)), Sense::click());
            a11y::control(ui, &rr, WidgetType::Checkbox, &label, Some(on));
            let rr = if salt == "outline" { rr.on_hover_text(tr!("View as outlines in the layer's colour (on the stage only)")) } else { rr };
            if rr.clicked() {
                app.run(id, json!({"layer": layer.id.0}));
            }
        }
        if let Some(ot) = onion_t {
            let rr = ui.interact(ot, ui.id().with(("onion", layer.id.0)), Sense::click());
            a11y::control(ui, &rr, WidgetType::Checkbox, &tr!("Onion skin {name}", name = name), Some(layer.onion));
            if rr.on_hover_text(tr!("Onion skin this layer (Alt-click: this layer only, again: every layer)")).clicked() {
                let solo = ui.input(|inp| inp.modifiers.alt);
                if let Some(v) = app.run("layer.set_onion", json!({"layer": layer.id.0, "solo": solo})) {
                    if !app.onion.ghosts {
                        app.status_ok(tr!("Onion skin is off: turn it on in the timeline header to see the frames around the playhead"));
                    } else if solo {
                        let all = v["onion_layers"].as_array().map_or(0, Vec::len) > 1;
                        app.status_ok(if all { tr!("Onion skin on every layer again").to_string() } else { tr!("Onion skin on {name} only (Alt-click it again for every layer)", name = name) });
                    }
                }
            }
        }
        // The colour chip on the row's edge: a click opens the layer's colour (ADR 0061).
        let chip = Rect::from_min_size(left.left_top(), vec2(8.0, rh));
        let chip_r = ui.interact(chip, ui.id().with(("lcolor", layer.id.0)), Sense::click());
        a11y::control(ui, &chip_r, WidgetType::Button, &tr!("Colour of {name}", name = name), None);
        if chip_r.hovered() {
            painter.rect_filled(Rect::from_min_size(left.left_top(), vec2(7.0, rh)), 0.0, to_color32(layer.color));
        }
        let chip_r = chip_r.on_hover_text(tr!("Layer colour: its outlines and its bar on the timeline"));
        let menu_id = egui::Id::new(("layer_colour", layer.id.0));
        if app.tl_color_open == Some(layer.id) {
            app.tl_color_open = None;
            egui::Popup::open_id(ui.ctx(), menu_id);
        }
        let shown = egui::Popup::menu(&chip_r).id(menu_id).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| color_menu(app, ui, layer.id, layer.color));
        if let Some(r) = shown {
            ui.ctx().accesskit_node_builder(r.response.id, |node| {
                node.set_role(egui::accesskit::Role::Menu);
                node.set_label(tr!("Layer colour"));
            });
        }
        if let Some(sp) = speaker {
            let rr = ui.interact(sp, ui.id().with(("mute", layer.id.0)), Sense::click());
            a11y::control(ui, &rr, WidgetType::Checkbox, &tr!("Mute {name}", name = layer.name), Some(layer.muted));
            let tip = if layer.muted { tr!("Unmute this layer's sounds") } else { tr!("Mute this layer's sounds (in playback and every export)") };
            if rr.on_hover_text(tip).clicked() {
                app.run("layer.toggle_mute", json!({"layer": layer.id.0}));
            }
        }
        // As in Animate: a click on the name makes the layer active and selects its artwork.
        if nr.clicked() {
            app.run("layer.select", json!({"layer": layer.id.0, "select": true}));
        }
        if nr.secondary_clicked() && app.s.layer != layer.id {
            app.run("layer.select", json!({"layer": layer.id.0}));
        }
        if nr.double_clicked() {
            // On the layer's icon: Layer Properties, as in Animate; on its name: rename.
            if nr.interact_pointer_pos().is_some_and(|p| icon_rect.contains(p)) {
                crate::layer_props::open(app, Some(layer.id));
            } else {
                app.invoke("layer.rename");
            }
        }
        // The folder chevron opens and closes the folder (on top of the name's click target).
        if is_folder {
            let cr = ui.interact(chevron, ui.id().with(("fold", layer.id.0)), Sense::click());
            a11y::control(ui, &cr, WidgetType::CollapsingHeader, &tr!("Open {name}", name = layer.name), Some(open));
            if cr.on_hover_text(if open { tr!("Close folder") } else { tr!("Open folder") }).clicked() && !app.tl_collapsed.remove(&layer.id) {
                app.tl_collapsed.insert(layer.id);
            }
        }
        nr.context_menu(|ui| {
            ui.set_min_width(200.0);
            if is_camera {
                // The camera layer: on or off, locked or not, or gone.
                for id in ["layer.rename", "layer.toggle_visible", "layer.toggle_lock"] {
                    menus::cmd_item(app, ui, id);
                }
                if ui.button(tr!("Properties…")).clicked() {
                    crate::layer_props::open(app, Some(layer.id));
                    ui.close();
                }
                ui.separator();
                if ui.button(tr!("Preview Through Camera")).clicked() {
                    app.toggle_camera_preview();
                    ui.close();
                }
                if ui.button(tr!("Remove Camera")).clicked() {
                    app.request("camera.remove", json!({}));
                    ui.close();
                }
                return;
            }
            for id in ["layer.rename", "layer.new", "layer.new_folder", "layer.delete"] {
                menus::cmd_item(app, ui, id);
            }
            ui.separator();
            for id in ["layer.toggle_visible", "layer.toggle_lock", "layer.toggle_outline"] {
                menus::cmd_item(app, ui, id);
            }
            if layer.has_sounds() && menus::check_item(ui, tr!("Mute"), layer.muted, true) {
                app.run("layer.toggle_mute", json!({"layer": layer.id.0}));
            }
            if !is_folder && menus::check_item(ui, tr!("Onion Skin"), layer.onion, true) {
                app.run("layer.set_onion", json!({"layer": layer.id.0}));
            }
            ui.menu_button(tr!("Layer Height"), |ui| {
                for h in 1..=anicraft_engine::doc::MAX_LAYER_HEIGHT {
                    if menus::check_item(ui, &format!("{} %", u32::from(h) * 100), layer.height() == h, true) {
                        app.run("layer.set_height", json!({"layer": layer.id.0, "height": u32::from(h) * 100}));
                    }
                }
            });
            if ui.button(tr!("Properties…")).clicked() {
                crate::layer_props::open(app, Some(layer.id));
                ui.close();
            }
            ui.separator();
            // Layer type, as check items: Mask (masks the layer below) and Masked.
            if menus::check_item(ui, tr!("Mask"), layer.kind == LayerKind::Mask, !is_folder) {
                app.run("layer.toggle_mask", json!({"layer": layer.id.0}));
            }
            let can_mask = layer.kind == LayerKind::Normal && app.s.timeline().mask_above(i).is_some();
            if menus::check_item(ui, tr!("Masked"), masked, can_mask || masked) {
                let ty = if masked { "normal" } else { "masked" };
                app.run("layer.set_type", json!({"layer": layer.id.0, "type": ty}));
            }
            // Guides: a plain guide never exports; a motion guide steers the layers it guides.
            if menus::check_item(ui, tr!("Guide"), layer.kind == LayerKind::Guide, matches!(layer.kind, LayerKind::Normal | LayerKind::Guide) && !masked && !guided) {
                let ty = if layer.kind == LayerKind::Guide { "normal" } else { "guide" };
                app.run("layer.set_type", json!({"layer": layer.id.0, "type": ty}));
            }
            let can_guide = layer.kind == LayerKind::Normal && app.s.timeline().guide_above(i).is_some();
            if menus::check_item(ui, tr!("Guided"), guided, can_guide || guided) {
                let ty = if guided { "normal" } else { "guided" };
                app.run("layer.set_type", json!({"layer": layer.id.0, "type": ty}));
            }
            if layer.kind == LayerKind::Normal && !masked && !guided {
                menus::cmd_item(app, ui, "layer.add_motion_guide");
            }
            ui.separator();
            for id in ["layer.move_up", "layer.move_down"] {
                menus::cmd_item(app, ui, id);
            }
        });

        // frame cells
        let cells = Rect::from_min_max(pos2(grid.left(), y), pos2(grid.right(), y + rh));
        let cp = painter.with_clip_rect(cells);
        if is_folder {
            // A folder has no frames: its row is a plain band across the timeline.
            cp.rect_filled(cells, 0.0, if is_active { T.row_active } else { T.panel_alt });
            let fr = ui.interact(cells, ui.id().with(("fcells", layer.id.0)), Sense::click() - Sense::FOCUSABLE);
            a11y::control(ui, &fr, WidgetType::Button, &tr!("{name} folder row", name = layer.name), None);
            if fr.clicked() && app.s.layer != layer.id {
                app.run("layer.select", json!({"layer": layer.id.0}));
            }
            painter.line_segment([row.left_bottom(), row.right_bottom()], Stroke::new(1.0, T.separator));
            continue;
        }
        cp.rect_filled(cells, 0.0, if is_active { T.row_active } else { T.panel });
        for f in first..first + visible_cols + 1 {
            let x = x_of(f);
            if (f + 1) % 5 == 0 && cell_w >= 6.0 {
                cp.rect_filled(Rect::from_min_size(pos2(x, y), vec2(cell_w, rh)), 0.0, Color32::from_white_alpha(6));
            }
            // Cell lines thin out with the ruler when zoomed out, so rows never turn to hatching.
            if (f + 1) % minor == 0 {
                cp.line_segment([pos2(x + cell_w - 0.5, y), pos2(x + cell_w - 0.5, y + rh)], Stroke::new(1.0, T.separator));
            }
        }
        // Spans are solid square blocks, separated by a one-pixel gap rather than outlines:
        // mint holds artwork, violet tweens, graphite is empty. Marks on them are ink.
        for (ki, k) in layer.keyframes.iter().enumerate() {
            if k.end() < first || k.start > first + visible_cols {
                continue;
            }
            let x0 = x_of(k.start);
            let x1 = x_of(k.end());
            let span = Rect::from_min_max(pos2(x0, y + 3.0), pos2(x1 - 1.0, y + rh - 3.0));
            let next_adjacent = layer.keyframes.get(ki + 1).is_some_and(|n| n.start == k.end());
            let motion = k.motion.as_ref();
            let tweened = k.tween.is_some() && next_adjacent && motion.is_none();
            let empty = k.elements.is_empty();
            let fill = if motion.is_some() {
                T.tween_motion
            } else if tweened {
                if k.tween.is_some_and(|t| t.kind == TweenKind::Shape) { T.tween_shape } else { T.tween }
            } else if empty {
                T.span_empty
            } else {
                T.span
            };
            let mark = if empty && !tweened { T.text } else { T.ink };
            cp.rect_filled(span, SQUARE, fill);
            // A muted layer's waveform is faint (ADR 0039).
            let wave = if layer.muted { T.text_faint } else if empty && !tweened { T.waveform } else { T.ink };
            if let Some(fs) = &k.sound
                && !waveform(&cp, &app.s.doc, fs, k, span, x0, cell_w, wave)
            {
                // Still decoding (just opened): draw it when it's ready.
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
            }
            let kc = pos2(x0 + cell_w * 0.5, y + rh * 0.5);
            if let Some(m) = motion {
                // Property keyframes: small diamonds where any property has a key. A frame a
                // motion path point is pinned to (ADR 0049) has a larger one with a hole, as the
                // stage marks that point.
                let pins = m.live_pins();
                for f in m.key_frames().into_iter().filter(|f| *f > 0 && *f < k.duration) {
                    let c = pos2(x_of(k.start + f) + cell_w * 0.5, kc.y);
                    let pinned = pins.iter().any(|p| p.frame == f);
                    let r = (cell_w * 0.35).clamp(2.5, 4.5) + if pinned { 1.0 } else { 0.0 };
                    cp.add(egui::Shape::convex_polygon(vec![c + vec2(0.0, -r), c + vec2(r, 0.0), c + vec2(0.0, r), c + vec2(-r, 0.0)], T.ink, Stroke::NONE));
                    if pinned {
                        cp.circle_filled(c, r * 0.4, T.tween_motion);
                    }
                }
            } else if tweened && k.duration > 1 {
                let end = pos2(x1 - cell_w * 0.5 - 4.0, kc.y);
                cp.line_segment([kc + vec2(5.0, 0.0), end], Stroke::new(1.5, T.tween_line));
                cp.add(egui::Shape::convex_polygon(vec![end + vec2(0.0, -3.5), end + vec2(5.0, 0.0), end + vec2(0.0, 3.5)], T.tween_line, Stroke::NONE));
            } else if k.tween.is_some() && motion.is_none() && !next_adjacent && k.duration > 1 {
                // broken tween: dashed line
                let mut x = kc.x + 6.0;
                while x < x1 - 6.0 {
                    cp.line_segment([pos2(x, kc.y), pos2((x + 3.0).min(x1 - 6.0), kc.y)], Stroke::new(1.5, mark));
                    x += 6.0;
                }
            } else if k.duration > 1 {
                let er = Rect::from_center_size(pos2(x1 - cell_w * 0.5 - 1.0, kc.y + 2.0), vec2(5.0, 7.0));
                cp.rect_stroke(er, 0.0, Stroke::new(1.2, mark), StrokeKind::Inside);
            }
            if empty {
                cp.circle_stroke(kc, 3.5, Stroke::new(1.5, mark));
            } else {
                cp.circle_filled(kc, 4.0, mark);
            }
            // Keyframes that run actions carry an "a" after the dot (Animate puts one there too).
            let after = if k.script.is_some() {
                cp.text(kc + vec2(6.5, -1.0), Align2::LEFT_CENTER, "a", font(Type::DataSmall), mark);
                9.0
            } else {
                0.0
            };
            if !k.label.is_empty() {
                cp.text(kc + vec2(8.0 + after, -1.0), Align2::LEFT_CENTER, &k.label, font(Type::Caption), mark);
            }
        }
        // Selected frames, and where a keyframe drag would drop them.
        if let Some(sel) = app.frame_sel
            && sel.rows(&layers).is_some_and(|(a, b)| i >= a && i <= b)
        {
            let (f0, f1) = sel.frames();
            let r = Rect::from_min_max(pos2(x_of(f0), y + 1.0), pos2(x_of(f1 + 1) - 1.0, y + rh - 1.0));
            cp.rect_filled(r, SQUARE, T.selection.gamma_multiply(0.28));
            cp.rect_stroke(r, SQUARE, Stroke::new(2.0, T.selection), StrokeKind::Inside);
        }
        // Where a keyframe drag would drop them: shifted in time and, across rows, onto this row.
        if let (TlDrag::Move { by, rows_by, .. }, Some(sel)) = (app.tl_drag, app.frame_sel)
            && (by != 0 || rows_by != 0)
            && let Some((a, b)) = sel.rows(&layers)
            && let Some(src) = i64::try_from(i).ok().map(|r| r - rows_by)
            && src >= a as i64
            && src <= b as i64
        {
            let (f0, f1) = sel.frames();
            let g = Rect::from_min_max(pos2(x_of(f0), y + 1.0), pos2(x_of(f1 + 1) - 1.0, y + rh - 1.0)).translate(vec2(by as f32 * cell_w, 0.0));
            cp.rect_stroke(g, SQUARE, Stroke::new(2.0, Color32::WHITE), StrokeKind::Inside);
        }
        // Span length being dragged on this row: the new end line and its length readout.
        if let TlDrag::Span { layer: lid, start, end, to } = app.tl_drag
            && lid == layer.id
        {
            // The frames gained (or given up), so the change reads as part of the span.
            let (a, b) = (x_of(end.min(to)), x_of(end.max(to)));
            let changed = Rect::from_min_max(pos2(a, y + 2.0), pos2(b - 1.0, y + rh - 2.0));
            cp.rect_filled(changed, SQUARE, if to > end { T.selection.gamma_multiply(0.35) } else { Color32::from_black_alpha(120) });
            let x = x_of(to);
            cp.line_segment([pos2(x, y + 1.0), pos2(x, y + rh - 1.0)], Stroke::new(2.0, Color32::WHITE));
            let n = to - start;
            let secs = n as f32 / app.s.doc.stage.fps.max(1.0);
            let label = trn!("{n} frame · {secs} s", "{n} frames · {secs} s", n, secs = format!("{secs:.2}"));
            let at = pos2(x + 6.0, y + rh * 0.5);
            let galley = painter.layout_no_wrap(label, font(Type::DataSmall), T.text);
            let (w, h) = (galley.rect.width(), galley.rect.height());
            let bg = Rect::from_min_size(at - vec2(3.0, h * 0.5 + 2.0), vec2(w + 6.0, h + 4.0));
            painter.rect_filled(bg, T.rc(), T.chrome);
            painter.rect_stroke(bg, T.rc(), Stroke::new(1.0, T.border), StrokeKind::Inside);
            painter.galley(bg.min + vec2(3.0, 2.0), galley, T.text);
        }
        let cr = ui.interact(cells, ui.id().with(("cells", layer.id.0)), a11y::pointer_only());
        a11y::control(ui, &cr, WidgetType::Button, &tr!("Frames of {name}", name = layer.name), None);
        if is_active {
            // Its keyframes and spans, one by one, for screen readers (ADR 0064).
            frame_nodes(app, ui, &cr, layer, x_of, y);
        }
        let shift = ui.input(|inp| inp.modifiers.shift);
        if cr.clicked() || cr.drag_started() || cr.secondary_clicked() {
            app.timeline_focus = true;
        }
        // Double-click a motion tween span: its curves open in the Motion Editor.
        if cr.double_clicked()
            && let Some(p) = cr.interact_pointer_pos()
            && layer.keyframe_at(frame_at(p.x)).is_some_and(|k| k.motion.is_some())
        {
            app.bottom = BottomTab::MotionEditor;
        }
        // The right edge of a span is a handle: drag it to make the span longer or shorter
        // (later frames move with it, as with F5 / Shift+F5).
        let span_end_at = |px: f32| layer.keyframes.iter().find(|k| k.end() > first && k.start <= first + visible_cols && (x_of(k.end()) - px).abs() <= 4.0).map(|k| (k.start, k.end()));
        if cr.hovered() && app.tl_drag == TlDrag::None && cr.hover_pos().and_then(|p| span_end_at(p.x)).is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if matches!(app.tl_drag, TlDrag::Span { .. }) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if cr.drag_started()
            && let Some(p) = ui.input(|inp| inp.pointer.press_origin())
            && let Some((start, end)) = span_end_at(p.x)
        {
            app.playing = false;
            app.tl_drag = TlDrag::Span { layer: layer.id, start, end, to: end };
        }
        let select_here = |app: &mut App, frame: u32| {
            app.playing = false;
            if app.s.layer != layer.id {
                app.run("layer.select", json!({"layer": layer.id.0}));
            }
            app.run("timeline.goto", json!({"frame": frame}));
        };
        // Press on selected frames that hold keyframes: drag them. Elsewhere: select a range.
        if cr.drag_started()
            && app.tl_drag == TlDrag::None
            && let Some(p) = ui.input(|inp| inp.pointer.press_origin())
        {
            let f = frame_at(p.x);
            let grab = app.frame_sel.filter(|s| s.contains(&layers, i, f) && s.has_keys(&layers));
            if grab.is_some() {
                app.tl_drag = TlDrag::Move { grab: f, by: 0, grab_row: i, rows_by: 0 };
            } else {
                app.frame_sel = Some(FrameSel::cell(layer.id, f));
                app.tl_drag = TlDrag::Select;
                select_here(app, f);
            }
        }
        if cr.dragged()
            && let Some(p) = cr.interact_pointer_pos()
        {
            let f = frame_at(p.x);
            match (app.tl_drag, app.frame_sel.as_mut()) {
                (TlDrag::Select, Some(sel)) => {
                    let r = row_at(&heights, first_row, p.y - rows_top).max(first_row);
                    let row = rows.get(r.min(rows.len().saturating_sub(1))).copied().unwrap_or(0);
                    if let Some(l) = layers.get(row) {
                        sel.head = (l.id, f);
                    }
                }
                (TlDrag::Move { grab, grab_row, .. }, Some(sel)) => {
                    // Never before frame 1, and only onto layers that exist.
                    let by = (i64::from(f) - i64::from(grab)).max(-i64::from(sel.frames().0));
                    // The row under the pointer, as a layer index (rows skip closed folders).
                    let vis = row_at(&heights, first_row, p.y - rows_top);
                    let row = rows.get(vis).map_or(grab_row as i64, |&li| li as i64);
                    let rows_by = match sel.rows(&layers) {
                        Some((a, b)) => (row - grab_row as i64).clamp(-(a as i64), layers.len() as i64 - 1 - b as i64),
                        None => 0,
                    };
                    app.tl_drag = TlDrag::Move { grab, by, grab_row, rows_by };
                }
                (TlDrag::Span { layer: lid, start, end, .. }, _) => {
                    // Snap to the nearest frame boundary; a span keeps at least one frame.
                    let b = ((p.x - grid.left() - pad) / cell_w + scroll).round().max(0.0) as u32;
                    app.tl_drag = TlDrag::Span { layer: lid, start, end, to: b.max(start + 1) };
                }
                _ => {}
            }
        }
        if cr.drag_stopped()
            && let TlDrag::Span { layer: lid, start: _, end, to } = app.tl_drag
        {
            let lid = lid.0;
            if to > end {
                app.request("timeline.insert_frame", json!({"layer": lid, "frame": end - 1, "count": to - end}));
            } else if to < end {
                app.request("timeline.remove_frame", json!({"layer": lid, "frame": to, "count": end - to}));
            }
            app.tl_drag = TlDrag::None;
        }
        if cr.drag_stopped() {
            if let (TlDrag::Move { by, rows_by, .. }, Some(sel)) = (app.tl_drag, app.frame_sel)
                && (by != 0 || rows_by != 0)
            {
                let (f0, f1) = sel.frames();
                let ids: Vec<u64> = sel.layers(&layers).iter().map(|l| l.0).collect();
                if app.run("timeline.move_keyframes", json!({"layers": ids, "from": f0, "to": f1, "by": by, "layer_by": rows_by})).is_some() {
                    // The selection follows the frames: shifted in time and onto the target rows.
                    let shift_by = |f: u32| (i64::from(f) + by).max(0) as u32;
                    let row_of = |id: LayerId| layers.iter().position(|l| l.id == id).and_then(|r| layers.get((r as i64 + rows_by).max(0) as usize)).map_or(id, |l| l.id);
                    app.frame_sel = Some(FrameSel { anchor: (row_of(sel.anchor.0), shift_by(sel.anchor.1)), head: (row_of(sel.head.0), shift_by(sel.head.1)) });
                    let n = f1 - f0 + 1;
                    // One sentence per direction; the frame count is a phrase of its own.
                    let frames = trn!("{n} frame", "{n} frames", n);
                    app.status_ok(match rows_by {
                        0 => trn!("Moved {n} frame by {by} — {undo} to undo", "Moved {n} frames by {by} — {undo} to undo", n, by = format!("{by:+}"), undo = app.undo_key()),
                        r if r < 0 => trn!("Moved {frames} {n} layer up — {undo} to undo", "Moved {frames} {n} layers up — {undo} to undo", r.unsigned_abs(), frames = frames, undo = app.undo_key()),
                        r => trn!("Moved {frames} {n} layer down — {undo} to undo", "Moved {frames} {n} layers down — {undo} to undo", r.unsigned_abs(), frames = frames, undo = app.undo_key()),
                    });
                    app.run("timeline.goto", json!({"frame": shift_by(app.s.frame)}));
                    if rows_by != 0
                        && let Some(&top) = sel.layers(&layers).first()
                    {
                        let target = row_of(top);
                        app.run("layer.select", json!({"layer": target.0}));
                    }
                }
            }
            app.tl_drag = TlDrag::None;
        }
        if cr.clicked()
            && let Some(p) = cr.interact_pointer_pos()
        {
            let f = frame_at(p.x);
            match app.frame_sel.as_mut() {
                Some(sel) if shift => sel.head = (layer.id, f),
                _ => app.frame_sel = Some(FrameSel::cell(layer.id, f)),
            }
            select_here(app, f);
        }
        // Right-click: keep a selection under the pointer, else select that frame; then offer
        // the frame actions (which act on the selection).
        if cr.secondary_clicked()
            && let Some(p) = cr.interact_pointer_pos()
        {
            let f = frame_at(p.x);
            if !app.frame_sel.is_some_and(|s| s.contains(&layers, i, f)) {
                app.frame_sel = Some(FrameSel::cell(layer.id, f));
                select_here(app, f);
            } else {
                app.playing = false;
            }
        }
        cr.context_menu(|ui| {
            ui.set_min_width(220.0);
            for id in ["timeline.insert_frame", "timeline.remove_frame", "timeline.insert_keyframe", "timeline.insert_blank_keyframe", "timeline.clear_keyframe"] {
                menus::cmd_item(app, ui, id);
            }
            ui.separator();
            for id in ["timeline.cut_frames", "timeline.copy_frames", "timeline.paste_frames"] {
                menus::cmd_item(app, ui, id);
            }
            ui.separator();
            let here = app.s.active_layer().and_then(|l| l.keyframe_at(app.s.frame));
            if here.is_some_and(|k| k.motion.is_some()) {
                if ui.button(tr!("Motion Editor")).clicked() {
                    app.bottom = BottomTab::MotionEditor;
                    ui.close();
                }
                menus::cmd_item(app, ui, "timeline.create_tween");
                menus::cmd_item(app, ui, "timeline.remove_tween");
                return;
            }
            let tween = here.and_then(|k| k.tween);
            match tween {
                Some(t) => {
                    menus::cmd_item(app, ui, "timeline.create_motion_tween");
                    menus::cmd_item(app, ui, if t.kind == TweenKind::Shape { "timeline.create_tween" } else { "timeline.create_shape_tween" });
                    menus::cmd_item(app, ui, "timeline.remove_tween");
                    ui.menu_button(tr!("Ease"), |ui| {
                        for (name, e) in Easing::PRESETS {
                            if ui.selectable_label(t.easing == e, crate::i18n::t(name)).clicked() {
                                app.run("timeline.set_easing", json!({"easing": easing_param(e)}));
                                ui.close();
                            }
                        }
                    });
                }
                None => {
                    menus::cmd_item(app, ui, "timeline.create_motion_tween");
                    menus::cmd_item(app, ui, "timeline.create_tween");
                    menus::cmd_item(app, ui, "timeline.create_shape_tween");
                }
            }
        });
        painter.line_segment([row.left_bottom(), row.right_bottom()], Stroke::new(1.0, T.separator));
    }
    // Layout lines: layer list | frame grid, and the header above.
    painter.line_segment([pos2(grid.left(), body.top()), pos2(grid.left(), body.bottom())], Stroke::new(1.0, T.border));
    painter.line_segment([body.left_top(), body.right_top()], Stroke::new(1.0, T.border));

    // end-of-timeline marker
    let end_x = x_of(frames_total);
    if end_x > grid.left() && end_x < grid.right() {
        painter.line_segment([pos2(end_x, rows_top), pos2(end_x, body.bottom())], Stroke::new(1.0, T.text_faint.gamma_multiply(0.5)));
    }

    // playhead
    let px = x_of(app.s.frame);
    if px + cell_w > grid.left() && px < grid.right() {
        // The same pixel as the ruler tick for this frame.
        let cx = frame_center(px, cell_w);
        // The flag fits its number (a button's state name on its state frames); it never grows
        // with the zoom.
        let state = states.and_then(|s| s.get(app.s.frame as usize).copied());
        let (label, role) = state.map_or_else(|| ((app.s.frame + 1).to_string(), Type::DataSmall), |n| (n.to_string(), Type::Caption));
        let tw = painter.layout_no_wrap(label.clone(), font(role), T.ink).rect.width();
        let head = Rect::from_center_size(pos2(cx, ruler.center().y), vec2((tw + 8.0).max(HEAD_W), RULER_H - 4.0));
        // The frame's band, centred on the same pixel column as the line and the flag: an odd
        // whole-pixel width around a half-pixel centre puts both edges on pixel boundaries,
        // equally far from the line.
        let band = (cell_w.round() as i32 | 1) as f32;
        painter.with_clip_rect(grid).rect_filled(Rect::from_min_max(pos2(cx - band * 0.5, rows_top), pos2(cx + band * 0.5, body.bottom())), 0.0, T.playhead.gamma_multiply(0.12));
        painter.with_clip_rect(grid).line_segment([pos2(cx, ruler.bottom()), pos2(cx, body.bottom())], Stroke::new(1.5, T.playhead));
        painter.with_clip_rect(grid).rect_filled(head, T.rc(), T.playhead);
        painter.with_clip_rect(grid).text(head.center(), Align2::CENTER_CENTER, label, font(role), T.ink);
    }

    app.drop.timeline = Some(crate::drop::TimelineGeo { rows: drop_rows, cells_left: grid.left(), x0: grid.left() + pad - scroll * cell_w, cell_w });

    // footer buttons for layers
    let footer = Rect::from_min_max(pos2(body.left(), body.bottom() - FOOTER_H), pos2(grid.left(), body.bottom()));
    if n_layers > rows_visible {
        let more = trn!("{first}–{last} of {n} layer", "{first}–{last} of {n} layers", n_layers, first = first_row + 1, last = (first_row + rows_visible).min(n_layers));
        // In the "Layers" header row, never over a layer row.
        painter.text(pos2(grid.left() - 8.0, ruler.center().y), Align2::RIGHT_CENTER, more, font(Type::Caption), color(Type::Caption));
    }
    {
        painter.rect_filled(footer, 0.0, T.panel_alt);
        painter.line_segment([footer.left_top(), footer.right_top()], Stroke::new(1.0, T.border));
        // New folder, Up and Down as icon buttons, laid out first from the right edge so they
        // always show; the text buttons share what is left, shortened if a language makes them
        // long (pseudo-locale screenshots lost the Down chevron off the column's edge).
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(footer.shrink2(vec2(8.0, 4.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
        for (icon, id, tip) in [
            (Icon::ChevronDown, "layer.move_down", tr!("Move layer down")),
            (Icon::ChevronUp, "layer.move_up", tr!("Move layer up")),
            (Icon::FolderPlus, "layer.new_folder", tr!("New folder above the active layer (move layers in under it with Up and Down)")),
        ] {
            let (r, resp) = child.allocate_exact_size(vec2(TARGET, TARGET), Sense::click());
            if resp.hovered() {
                child.painter().rect_filled(r, T.rc(), T.hover);
            }
            icons::paint(child.painter(), Rect::from_center_size(r.center(), vec2(16.0, 16.0)), icon, if resp.hovered() { T.icon_hot } else { T.icon });
            a11y::control(&child, &resp, WidgetType::Button, tip, None);
            if resp.on_hover_text(tip).clicked() {
                app.run(id, json!({}));
            }
        }
        child.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            if ui.add(egui::Button::new(tr!("+ Layer")).truncate()).named(tr!("New layer")).on_hover_text(tr!("New layer")).clicked() {
                app.run("layer.new", json!({}));
            }
            if ui.add_enabled(layers.len() > 1, egui::Button::new(tr!("Delete")).truncate()).named(tr!("Delete layer")).on_hover_text(tr!("Delete the active layer")).clicked() {
                app.request("layer.delete", json!({}));
            }
        });
    }

    // Horizontal scrollbar under the frames: the thumb is the visible part of the timeline.
    let bar = Rect::from_min_max(pos2(grid.left(), body.bottom() - FOOTER_H), body.right_bottom());
    painter.rect_filled(bar, 0.0, T.panel_alt);
    painter.line_segment([bar.left_top(), bar.right_top()], Stroke::new(1.0, T.border));
    // Frame zoom and Frame View at the bar's right end, as Animate has them; the scroll track
    // takes the rest.
    let zoom_area = Rect::from_min_max(pos2((bar.right() - ZOOM_W).max(bar.left()), bar.top()), bar.right_bottom());
    let bar = Rect::from_min_max(bar.left_top(), pos2(zoom_area.left(), bar.bottom()));
    let track = bar.shrink2(vec2(10.0, (FOOTER_H - 12.0) * 0.5));
    let content = (frames_total + 30).max(visible_cols) as f32;
    let frac = (visible_cols as f32 / content).clamp(0.05, 1.0);
    let thumb_w = (track.width() * frac).max(TARGET);
    let travel = (track.width() - thumb_w).max(1.0);
    let max_s = (content - visible_cols as f32).max(0.0);
    let t = if max_s > 0.0 { (app.tl_scroll / max_s).clamp(0.0, 1.0) } else { 0.0 };
    let thumb = Rect::from_min_size(pos2(track.left() + t * travel, track.top()), vec2(thumb_w, track.height()));
    let sr = ui.interact(bar, ui.id().with("tl_hscroll"), a11y::pointer_only());
    a11y::control(ui, &sr, WidgetType::ScrollBar, tr!("Scroll the timeline"), None);
    if sr.dragged() && max_s > 0.0 {
        app.tl_scroll = (app.tl_scroll + sr.drag_delta().x / travel * max_s).clamp(0.0, max_s);
    } else if sr.clicked()
        && let Some(p) = sr.interact_pointer_pos()
        && max_s > 0.0
    {
        // A click on the track pages to that spot.
        app.tl_scroll = (((p.x - track.left() - thumb_w * 0.5) / travel) * max_s).clamp(0.0, max_s);
    }
    painter.rect_filled(track, T.rc(), T.field);
    painter.rect_filled(thumb, T.rc(), if sr.hovered() || sr.dragged() { T.control_edge } else { T.hover });
    sr.on_hover_text(tr!("Scroll the timeline (or Shift+wheel anywhere over it)"));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(zoom_area.shrink2(vec2(6.0, 4.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
    frame_view_button(app, &mut child);
    let mut pct = app.tl_cell_w / CELL_DEFAULT * 100.0;
    let (lo, hi) = (CELL_RANGE.start() / CELL_DEFAULT * 100.0, CELL_RANGE.end() / CELL_DEFAULT * 100.0);
    child.spacing_mut().slider_width = 56.0;
    let r = with_type(&mut child, Type::Data, |ui| a11y::slider(ui, &mut pct, lo..=hi, tr!("Frame zoom"), a11y::SliderLook { suffix: " %", max_decimals: Some(0), logarithmic: true, ..Default::default() }));
    if r.on_hover_text(tr!("Frame zoom: frame width (Ctrl+wheel over the frames)")).changed() {
        app.tl_cell_w = (pct / 100.0 * CELL_DEFAULT).clamp(*CELL_RANGE.start(), *CELL_RANGE.end());
    }
}

/// The layer colour popup (a click on a layer's colour chip, ADR 0061): the layer colours as
/// swatches, and a picker for any other (a drag in it is one undo step).
fn color_menu(app: &mut App, ui: &mut egui::Ui, layer: LayerId, color: anicraft_engine::doc::Rgba) {
    use anicraft_engine::doc::Rgba;
    ui.set_min_width(200.0);
    ui.label(text(Type::Caption, tr!("Layer colour")));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        ui.set_max_width(6.0 * 24.0);
        for hex in anicraft_engine::LAYER_COLORS {
            let c = Rgba::hex(hex);
            let (r, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
            ui.painter().rect_filled(r, T.rc(), to_color32(c));
            if c.to_hex() == color.to_hex() {
                ui.painter().rect_stroke(r, T.rc(), Stroke::new(2.0, T.text), StrokeKind::Outside);
            } else if resp.hovered() {
                ui.painter().rect_stroke(r, T.rc(), Stroke::new(1.0, T.text_dim), StrokeKind::Outside);
            }
            a11y::control(ui, &resp, WidgetType::Button, &tr!("Layer colour {hex}", hex = c.to_hex()), Some(c.to_hex() == color.to_hex()));
            if resp.clicked() {
                app.run("layer.set_color", json!({"layer": layer.0, "color": c.to_hex()}));
            }
        }
    });
    ui.separator();
    // Any other colour: typed as hex, or picked in Layer Properties.
    ui.horizontal(|ui| {
        let id = ui.id().with(("layer_hex", layer.0));
        let mut hex = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| color.to_hex());
        let r = with_type(ui, Type::Data, |ui| ui.add(egui::TextEdit::singleline(&mut hex).desired_width(72.0).char_limit(9))).named(tr!("Layer colour as hex"));
        if r.changed() {
            ui.data_mut(|d| d.insert_temp(id, hex.clone()));
        }
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            ui.data_mut(|d| d.remove::<String>(id));
            let typed = if hex.starts_with('#') { hex.trim().to_string() } else { format!("#{}", hex.trim()) };
            app.run("layer.set_color", json!({"layer": layer.0, "color": typed}));
        }
        if ui.button(tr!("More…")).on_hover_text(tr!("Pick any colour in Layer Properties")).clicked() {
            crate::layer_props::open(app, Some(layer));
            ui.close();
        }
    });
}

const FRAME_VIEW_MENU: &str = "tl_frame_view";

/// The timeline's Frame View menu (ADR 0061), as in Animate: frame widths Tiny to Large (the
/// zoom slider beside it sets any width between) and Short rows. View state.
fn frame_view_button(app: &mut App, ui: &mut egui::Ui) {
    let id = egui::Id::new(FRAME_VIEW_MENU);
    let (rect, resp) = ui.allocate_exact_size(vec2(TARGET, TARGET), Sense::click());
    let hot = resp.hovered() || egui::Popup::is_id_open(ui.ctx(), id);
    ui.painter().rect_filled(rect, T.rc(), if hot { T.header } else { Color32::TRANSPARENT });
    icons::paint(ui.painter(), Rect::from_center_size(rect.center(), vec2(16.0, 16.0)), Icon::Layer, if hot { T.icon_hot } else { T.icon });
    a11y::control(ui, &resp, WidgetType::Button, tr!("Frame view"), None);
    let resp = resp.on_hover_text(tr!("Frame view: frame width and row height"));
    if std::mem::take(&mut app.tl_frame_view_open) {
        egui::Popup::open_id(ui.ctx(), id);
    }
    let shown = egui::Popup::menu(&resp).id(id).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        ui.set_min_width(160.0);
        for (name, w) in FRAME_WIDTHS {
            if menus::check_item(ui, crate::i18n::t(name), (app.tl_cell_w - w).abs() < 0.01, true) {
                app.tl_cell_w = w;
            }
        }
        ui.separator();
        if menus::check_item(ui, tr!("Short"), app.prefs.timeline_short_rows, true) {
            let mut p = app.prefs.clone();
            p.timeline_short_rows = !p.timeline_short_rows;
            let ctx = ui.ctx().clone();
            app.set_prefs(&ctx, p);
        }
    });
    // egui leaves the popup's own surface without a role or name.
    if let Some(r) = shown {
        ui.ctx().accesskit_node_builder(r.response.id, |node| {
            node.set_role(egui::accesskit::Role::Menu);
            node.set_label(tr!("Frame view"));
        });
    }
}

/// While the timeline has the keyboard, says where the playhead went (through the status bar,
/// a live region): "Frame 12 · Ball · Frames 1–24: keyframe · motion tween". Quiet while playing,
/// and on the frame the focus arrived at (Ctrl+F6 has just said the panel).
pub fn announce(app: &mut App, ctx: &egui::Context) {
    let focused = ctx.memory(|m| m.focused().is_some()) && a11y::focused_region(ctx) == Some(a11y::Region::Timeline);
    let now = (app.s.layer, app.s.frame);
    if !focused || app.playing {
        app.tl_heard = if focused { Some(now) } else { None };
        return;
    }
    if app.tl_heard.is_some_and(|h| h != now) {
        let Some(layer) = app.s.timeline().layer(now.0) else { return };
        let what = frame_text(layer, now.1).unwrap_or_else(|| tr!("no frame").to_string());
        let msg = tr!("Frame {frame} · {layer} · {what}", frame = now.1 + 1, layer = layer.name, what = what);
        app.status_ok(msg);
    }
    app.tl_heard = Some(now);
}

/// Most spans of the active layer listed one by one; the rest are summed up.
pub const MAX_SPAN_NODES: usize = 1_000;

/// What a span is, as a screen reader hears it: "Frames 1–24: keyframe · motion tween · label
/// “walk”". `next_adjacent`: the next keyframe starts where this one ends (a tween reaches it).
pub fn span_text(k: &anicraft_engine::doc::Keyframe, next_adjacent: bool) -> String {
    let (a, b) = (k.start + 1, k.end());
    let frames = if k.duration <= 1 { tr!("Frame {a}", a = a) } else { tr!("Frames {a}–{b}", a = a, b = b) };
    let mut parts: Vec<String> = vec![if k.elements.is_empty() { tr!("blank keyframe").to_string() } else { tr!("keyframe").to_string() }];
    if k.motion.is_some() {
        parts.push(tr!("motion tween").to_string());
    } else if let Some(t) = k.tween {
        parts.push(match (t.kind, next_adjacent) {
            (_, false) => tr!("broken tween").to_string(),
            (TweenKind::Shape, true) => tr!("shape tween").to_string(),
            (TweenKind::Classic, true) => tr!("classic tween").to_string(),
        });
    }
    if !k.label.is_empty() {
        parts.push(tr!("label “{label}”", label = k.label));
    }
    if k.sound.is_some() {
        parts.push(tr!("sound").to_string());
    }
    if k.script.is_some() {
        parts.push(tr!("actions").to_string());
    }
    tr!("{frames}: {what}", frames = frames, what = parts.join(" · "))
}

/// The span of `layer` at `frame`, described (`None`: no frame there).
pub fn frame_text(layer: &Layer, frame: u32) -> Option<String> {
    let ki = layer.keyframes.iter().position(|k| k.covers(frame))?;
    let k = layer.keyframes.get(ki)?;
    Some(span_text(k, layer.keyframes.get(ki + 1).is_some_and(|n| n.start == k.end())))
}

/// The active layer's spans as a list under its frame grid's node: each says its frames, what
/// is there and its tween; the one under the playhead is selected; a reader's click moves the
/// playhead there.
fn frame_nodes(app: &mut App, ui: &mut egui::Ui, grid: &egui::Response, layer: &Layer, x_of: impl Fn(u32) -> f32, y: f32) {
    use egui::accesskit::{Action, Role};
    let ctx = ui.ctx().clone();
    let child = ui.new_child(egui::UiBuilder::new().id_salt(("frame_list", layer.id.0)).accessibility_parent(grid.id).max_rect(grid.rect).sense(Sense::hover()));
    let ak = |r: Rect| egui::accesskit::Rect { x0: r.min.x.into(), y0: r.min.y.into(), x1: r.max.x.into(), y1: r.max.y.into() };
    let total = layer.keyframes.len();
    ctx.accesskit_node_builder(child.unique_id(), |n| {
        n.set_role(Role::List);
        n.set_label(trn!("{n} keyframe span on {name}", "{n} keyframe spans on {name}", total, name = layer.name));
        n.set_bounds(ak(grid.rect));
    });
    let mut goto = None;
    for (ki, k) in layer.keyframes.iter().enumerate().take(MAX_SPAN_NODES) {
        let id = child.unique_id().with(("span", k.start));
        let r = Rect::from_min_max(pos2(x_of(k.start), y), pos2(x_of(k.end()), y + ROW_H)).intersect(grid.rect);
        child.interact(Rect::from_center_size(r.center(), egui::Vec2::ZERO), id, Sense::hover());
        let next_adjacent = layer.keyframes.get(ki + 1).is_some_and(|n| n.start == k.end());
        let name = span_text(k, next_adjacent);
        ctx.accesskit_node_builder(id, |n| {
            n.set_role(Role::ListItem);
            n.set_label(name);
            n.set_selected(k.covers(app.s.frame));
            if r.is_positive() {
                n.set_bounds(ak(r));
            }
            n.set_position_in_set(ki + 1);
            n.set_size_of_set(total);
            n.add_action(Action::Click);
        });
        if ctx.input(|i| i.has_accesskit_action_request(id, Action::Click)) {
            goto = Some(k.start);
        }
    }
    if total > MAX_SPAN_NODES {
        let id = child.unique_id().with("more");
        child.interact(Rect::from_center_size(grid.rect.center(), egui::Vec2::ZERO), id, Sense::hover());
        ctx.accesskit_node_builder(id, |n| {
            n.set_role(Role::Label);
            n.set_label(trn!("{n} more span is not listed: go there with , and . or Home and End", "{n} more spans are not listed: go there with , and . or Home and End", total - MAX_SPAN_NODES));
        });
    }
    if let Some(f) = goto {
        app.playing = false;
        app.run("timeline.goto", json!({"frame": f}));
    }
}


/// Engine param for an easing preset.
/// A keyframe's sound over its span: the waveform from the keyframe on, repeated as the sound
/// repeats, as far as the span reaches (all a stream sound plays; an event sound plays on past
/// it), shaped by its effect's envelope (the louder channel's level). One line per point
/// column, from the lowest to the highest sample under it. `false` while the sound is still
/// being decoded.
#[allow(clippy::too_many_arguments)]
fn waveform(p: &egui::Painter, doc: &anicraft_engine::doc::Document, fs: &anicraft_engine::doc::FrameSound, k: &anicraft_engine::doc::Keyframe, span: Rect, x0: f32, cell_w: f32, color: Color32) -> bool {
    let Some(snd) = doc.sound(fs.sound) else { return true };
    let Some(dec) = snd.decoded_if_ready() else { return false };
    let len = dec.peaks.frames() as u64;
    if len == 0 || cell_w <= 0.0 {
        return true;
    }
    let env = (!fs.effect.is_none()).then(|| fs.effect.points(fs.heard_seconds(snd, k.duration, doc.frame_count().saturating_sub(k.start), doc.stage.fps)));
    let rate = f64::from(dec.pcm.rate.max(1));
    let total = if fs.looping { u64::MAX } else { len.saturating_mul(u64::from(fs.repeat.max(1))) };
    let per_point = f64::from(dec.pcm.rate) / f64::from(doc.stage.fps.max(1.0)) / f64::from(cell_w);
    let clip = p.clip_rect().intersect(span.shrink2(vec2(0.0, 1.0)));
    let (cy, h) = (clip.center().y, clip.height() * 0.5);
    let stroke = Stroke::new(1.0, color);
    let mut x = clip.left().floor().max(span.left());
    while x < clip.right() {
        let a = (f64::from(x - x0) * per_point).max(0.0) as u64;
        let b = (f64::from(x + 1.0 - x0) * per_point).max(0.0) as u64 + 1;
        if a >= total {
            break;
        }
        let b = b.min(total);
        // The source frames under this column, within one play of the sound.
        let (s, n) = ((a % len) as usize, (b - a).min(len) as usize);
        let range = if s + n <= len as usize {
            dec.peaks.range(s, s + n)
        } else {
            let (p1, p2) = (dec.peaks.range(s, len as usize), dec.peaks.range(0, s + n - len as usize));
            match (p1, p2) {
                (Some((l1, h1)), Some((l2, h2))) => Some((l1.min(l2), h1.max(h2))),
                (one, other) => one.or(other),
            }
        };
        if let Some((lo, hi)) = range {
            let g = env.as_deref().map_or(1.0, |pts| {
                let (l, r) = crate::envelope::level_at(pts, a as f64 / rate);
                l.max(r)
            });
            let (top, bottom) = (cy - (hi * g).clamp(-1.0, 1.0) * h, cy - (lo * g).clamp(-1.0, 1.0) * h);
            p.line_segment([pos2(x + 0.5, top.min(cy - 0.5)), pos2(x + 0.5, bottom.max(cy + 0.5))], stroke);
        }
        x += 1.0;
    }
    true
}

/// An ease's name in the interface language: the presets and "Custom" are catalog strings, a
/// classic ease's strength is filled in, a free Bézier's numbers are data.
/// The names of a button's four state frames, while a button is open for editing (ADR 0057).
pub fn button_states(app: &App) -> Option<[&'static str; 4]> {
    let open = app.s.editing_symbol().and_then(|id| app.s.doc.symbol(id))?;
    (open.kind == anicraft_engine::doc::SymbolKind::Button).then(|| [trc!("button state", "Up"), trc!("button state", "Over"), trc!("button state", "Down"), trc!("button state", "Hit")])
}

pub fn easing_label(e: &Easing) -> String {
    match e {
        Easing::Classic(s) => tr!("Classic {strength}", strength = format!("{s:+.0}")),
        other => crate::i18n::t_dyn(&other.label()).into_owned(),
    }
}

pub fn easing_param(e: Easing) -> serde_json::Value {
    match e {
        Easing::Linear => json!("Linear"),
        Easing::QuadIn => json!("QuadIn"),
        Easing::QuadOut => json!("QuadOut"),
        Easing::QuadInOut => json!("QuadInOut"),
        Easing::CubicIn => json!("CubicIn"),
        Easing::CubicOut => json!("CubicOut"),
        Easing::CubicInOut => json!("CubicInOut"),
        Easing::Classic(s) => json!({"classic": s}),
        Easing::CubicBezier(a, b, c, d) => json!({"bezier": [a, b, c, d]}),
    }
}

/// The timeline header, left to right: tabs, transport, readouts, frame and tween buttons.
/// The frame rate as the header shows it: whole rates without decimals ("24 fps"), others
/// with as many as they need ("29.97 fps", "12.5 fps").
fn fps_label(fps: f32) -> String {
    let s = format!("{fps:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    format!("{s} fps")
}

fn header(app: &mut App, ui: &mut egui::Ui, dock: Rect) {
    // Two tabs share the dock: the timeline and the Motion Editor.
    for (tab, name, tip) in [(BottomTab::Timeline, tr!("Timeline"), tr!("Layers and frames")), (BottomTab::MotionEditor, tr!("Motion Editor"), tr!("Property curves of the motion tween at the playhead (double-click a motion span)"))] {
        let r = ui.selectable_label(app.bottom == tab, text(Type::Heading, name));
        // With the Motion Editor up, Ctrl+F6 lands on its tab (the layer rows are hidden).
        if tab == BottomTab::MotionEditor && app.bottom == tab {
            a11y::anchor(ui, a11y::Region::Timeline, dock, &r);
        }
        if r.on_hover_text(tip).clicked() {
            app.bottom = tab;
        }
    }
    ui.add_space(4.0);
    transport(app, ui);
    ui.add_space(4.0);
    let fps = app.s.doc.stage.fps;
    let f = app.s.frame;
    // Fixed-width readouts: digits stay put while the playhead moves.
    let digits = app.s.frame_count().max(f + 1).to_string().len().max(3);
    ui.label(text(Type::DataLarge, format!("{:>digits$}", f + 1)).color(T.accent_text)).named(tr!("Current frame")).on_hover_text(tr!("Current frame"));
    ui.label(text(Type::Data, fps_label(fps)).color(T.text_dim)).on_hover_text(tr!("Frame rate"));
    ui.label(text(Type::Data, format!("{:>6.2} s", f as f32 / fps.max(1.0))).color(T.text_dim)).on_hover_text(tr!("Time at the playhead"));
    ui.separator();
    // Through `invoke`, so the buttons act on the frame selection like the keys do.
    let cmd = |app: &mut App, ui: &mut egui::Ui, label: &str, tip: &str, id: &'static str| {
        if ui.button(label).named(tip).on_hover_text(tip).clicked() {
            app.invoke(id);
        }
    };
    cmd(app, ui, tr!("Frame"), tr!("Insert frame (F5)"), "timeline.insert_frame");
    cmd(app, ui, tr!("Keyframe"), tr!("Insert keyframe (F6)"), "timeline.insert_keyframe");
    cmd(app, ui, tr!("Blank"), tr!("Insert blank keyframe (F7)"), "timeline.insert_blank_keyframe");
    let here = app.s.active_layer().and_then(|l| l.keyframe_at(app.s.frame));
    let in_tween = here.and_then(|k| k.tween);
    let in_motion = here.is_some_and(|k| k.motion.is_some());
    if in_motion {
        egui::ComboBox::from_id_salt("tl_kind").width(120.0).selected_text(tr!("Motion tween")).show_ui(ui, |ui| {
            if ui.selectable_label(false, tr!("Motion Editor")).on_hover_text(tr!("Edit the property curves")).clicked() {
                app.bottom = BottomTab::MotionEditor;
            }
            ui.separator();
            if ui.selectable_label(false, tr!("Classic tween")).on_hover_text(tr!("Tween to the next keyframe instead")).clicked() {
                app.run("timeline.create_tween", json!({}));
            }
            if ui.selectable_label(false, tr!("Remove tween")).clicked() {
                app.invoke("timeline.remove_tween");
            }
        })
        .response
        .set_name(tr!("Tween"));
    }
    match in_tween.filter(|_| !in_motion) {
        Some(tw) => {
            egui::ComboBox::from_id_salt("tl_ease").width(112.0).selected_text(easing_label(&tw.easing)).show_ui(ui, |ui| {
                for (name, e) in Easing::PRESETS {
                    if ui.selectable_label(tw.easing == e, crate::i18n::t(name)).clicked() {
                        app.run("timeline.set_easing", json!({"easing": easing_param(e)}));
                    }
                }
            })
            .response
            .set_name(tr!("Easing"));
            // The tween's kind: classic ⇄ shape (keeping the easing), or none.
            let shape = tw.kind == TweenKind::Shape;
            egui::ComboBox::from_id_salt("tl_kind").width(104.0).selected_text(if shape { tr!("Shape tween") } else { tr!("Classic tween") }).show_ui(ui, |ui| {
                if ui.selectable_label(!shape, tr!("Classic tween")).on_hover_text(tr!("Moves, scales, rotates and recolours")).clicked() && shape {
                    app.run("timeline.create_tween", json!({}));
                }
                if ui.selectable_label(shape, tr!("Shape tween")).on_hover_text(tr!("Also morphs one shape into another")).clicked() && !shape {
                    app.run("timeline.create_shape_tween", json!({}));
                }
                if ui.selectable_label(false, tr!("Motion tween")).on_hover_text(tr!("Animate each property on its own curve")).clicked() {
                    app.run("timeline.create_motion_tween", json!({}));
                }
                ui.separator();
                if ui.selectable_label(false, tr!("Remove tween")).clicked() {
                    app.invoke("timeline.remove_tween");
                }
            })
            .response
            .set_name(tr!("Tween"));
        }
        None if in_motion => {}
        None => {
            // One menu for the three kinds keeps the header short.
            egui::ComboBox::from_id_salt("tl_create").width(120.0).selected_text(tr!("Create tween")).show_ui(ui, |ui| {
                for (label, tip, id) in [
                    (tr!("Motion tween"), tr!("Animate this span's object property by property (edit the curves in the Motion Editor)"), "timeline.create_motion_tween"),
                    (tr!("Classic tween"), tr!("Tween to the next keyframe"), "timeline.create_tween"),
                    (tr!("Shape tween"), tr!("Morph this keyframe's shapes into the next keyframe's"), "timeline.create_shape_tween"),
                ] {
                    if ui.selectable_label(false, label).on_hover_text(tip).clicked() {
                        app.run(id, json!({}));
                    }
                }
            })
            .response
            .set_name(tr!("Tween"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layers(n: u64) -> Vec<Arc<Layer>> {
        (1..=n).map(|i| Arc::new(Layer::new(LayerId(i), format!("L{i}")))).collect()
    }

    #[test]
    fn rows_take_their_layer_height_and_scroll_by_whole_rows() {
        // A normal row, a 300 % row, two normal rows; 100 points to show them in.
        let h = [28.0, 84.0, 28.0, 28.0];
        assert_eq!(rows_fitting(&h, 0, 100.0), 1, "the tall row doesn't fit after the first");
        assert_eq!(rows_fitting(&h, 1, 100.0), 1);
        assert_eq!(rows_fitting(&h, 2, 100.0), 2);
        assert_eq!(rows_fitting(&h, 1, 10.0), 1, "a row taller than the space still shows");
        assert_eq!(rows_fitting(&h, 4, 100.0), 0);
        // Scrolled to the end, the last rows fill the space; a row is brought into view from above.
        assert_eq!(max_first_row(&h, 100.0), 2);
        assert_eq!(max_first_row(&h, 1000.0), 0, "everything fits: no scrolling");
        assert_eq!(first_showing(&h, 1, 100.0), 1);
        assert_eq!(first_showing(&h, 3, 60.0), 2);
        assert_eq!(first_showing(&h, 0, 5.0), 0);
        // The pointer: rows by their own heights, from the first row shown, and above it.
        assert_eq!(row_at(&h, 0, 10.0), 0);
        assert_eq!(row_at(&h, 0, 28.0), 1);
        assert_eq!(row_at(&h, 0, 111.0), 1);
        assert_eq!(row_at(&h, 0, 112.0), 2);
        assert_eq!(row_at(&h, 2, 30.0), 3);
        assert_eq!(row_at(&h, 0, 9999.0), 3, "below the last row: the last row");
        assert_eq!(row_at(&h, 2, -1.0), 1, "just above the first row shown: the tall row");
        assert_eq!(row_at(&h, 2, -90.0), 0);
        assert_eq!(row_at(&h, 2, -9999.0), 0);
        assert_eq!(row_at(&[], 0, 5.0), 0);
        assert_eq!(max_first_row(&[], 50.0), 0);
        // Short rows, and a 200 % short row is two of them.
        assert!(base_row_h(true) < base_row_h(false));
        // The frame widths run Tiny to Large inside the zoom range, Normal at 100 %.
        assert!(FRAME_WIDTHS.windows(2).all(|w| w[0].1 < w[1].1));
        assert!(FRAME_WIDTHS.iter().all(|(_, w)| CELL_RANGE.contains(w)));
        assert_eq!(FRAME_WIDTHS[2].1, CELL_DEFAULT);
    }

    /// The header at the default window size (1600 × 940, `main.rs`), in the widest state the
    /// sample has: a classic tween at the playhead (Ease and tween-kind menus both shown).
    fn header_spill(width: f32) -> f32 {
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, anicraft_engine::Session::new(anicraft_engine::doc::sample::demo()), Some(std::path::PathBuf::from("unused.png")));
        assert!(app.s.active_layer().and_then(|l| l.keyframe_at(app.s.frame)).is_some_and(|k| k.tween.is_some()), "the widest header: a classic tween");
        let input = || egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(width, 940.0))), ..Default::default() };
        for _ in 0..4 {
            ctx.run_ui(input(), |ui| app.frame(ui)).textures_delta.clear();
        }
        app.tl_header_spill
    }

    #[test]
    fn the_header_fits_at_the_default_window_size() {
        let spill = header_spill(1600.0);
        assert!(spill <= 0.5, "the timeline header overflows by {spill:.0} pt at 1600 × 940");
        // The measure is real: at the smallest window the header scrolls sideways instead.
        assert!(header_spill(960.0) > 0.5);
    }

    #[test]
    fn frame_rates_read_without_needless_decimals() {
        assert_eq!(fps_label(24.0), "24 fps");
        assert_eq!(fps_label(30.0), "30 fps");
        assert_eq!(fps_label(29.97), "29.97 fps");
        assert_eq!(fps_label(12.5), "12.5 fps");
        assert_eq!(fps_label(120.0), "120 fps");
    }

    #[test]
    fn the_playhead_flag_fits_at_frame_one_at_every_zoom_and_padding_stays_small() {
        let left = 100.0;
        let mut cw = *CELL_RANGE.start();
        while cw <= *CELL_RANGE.end() {
            let pad = start_pad(cw, 0.0);
            // Frame 1's centre, as the ruler and the playhead draw it.
            let c = frame_center(left + pad, cw);
            assert!(c - HEAD_W * 0.5 >= left - 0.5, "flag clipped at cell width {cw}: centre {c}");
            // The padding is room for the flag, not a frame's worth that grows with the zoom.
            assert!(pad <= HEAD_W * 0.5 + 1.0, "pad {pad} at cell width {cw}");
            cw += 0.5;
        }
        assert_eq!(start_pad(48.0, 0.0), 1.0, "wide frames need only a hairline");
        assert_eq!(start_pad(48.0, 10.0), 11.0, "the preference adds points");
    }

    #[test]
    fn ticks_and_the_playhead_share_a_pixel_column() {
        // Same cell, same column, whatever the fractional position or width.
        for (x, cw) in [(100.3, 12.0), (99.9, 7.5), (250.49, 3.0), (10.0, 47.25)] {
            let c = frame_center(x, cw);
            assert_eq!(c.fract(), 0.5, "centred on a pixel");
            assert!((c - (x + cw * 0.5)).abs() <= 1.0);
        }
    }

    #[test]
    fn a_selection_spans_rows_and_frames_in_either_direction() {
        let ls = layers(4);
        // Dragged up and to the left.
        let sel = FrameSel { anchor: (LayerId(3), 9), head: (LayerId(2), 4) };
        assert_eq!(sel.frames(), (4, 9));
        assert_eq!(sel.layers(&ls), vec![LayerId(2), LayerId(3)]);
        assert!(sel.contains(&ls, 1, 4) && sel.contains(&ls, 2, 9));
        assert!(!sel.contains(&ls, 0, 5) && !sel.contains(&ls, 1, 10));
        // Each new layer has one keyframe at frame 0.
        assert!(!sel.has_keys(&ls));
        assert!(FrameSel::cell(LayerId(1), 0).has_keys(&ls));
        // A deleted layer invalidates the selection.
        assert!(FrameSel { anchor: (LayerId(9), 0), head: (LayerId(1), 0) }.layers(&ls).is_empty());
    }
}
