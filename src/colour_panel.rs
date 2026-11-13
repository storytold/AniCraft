//! The fill side of the colour picker (ADR 0074): one picker, as Animate has one Color panel.
//!
//! The Tools panel's Fill square and every Fill swatch in Properties open the same popup. Its
//! top row is the fill's type (None, Solid, Linear, Radial, Bitmap). A solid fill shows the
//! colour picker (`crate::colour`: web-safe swatches, the mixer, hex). A gradient shows its stop
//! bar, overflow, Linear RGB and focal point above that same picker, which then edits the picked
//! stop's colour and opacity. A bitmap fill shows the Library image it uses. The Stroke square
//! opens the colour picker alone (strokes are one colour).
//!
//! The popup says what was asked ([`FillEdit`]); its caller applies it: a solid colour or None
//! the way that place always has, anything else as `paint.set_fill`, which sets the fill swatch
//! and the selected fills in one step, so agents and the CLI work the same way.

use crate::a11y::{self, Named};
use crate::app::App;
use crate::theme::{T, Type, text, to_color32, with_type};
use anicraft_engine::doc::{ElementKind, GradientStop, MAX_FOCAL, MAX_GRADIENT_STOPS, Paint, Rgba, Spread, ramp_at};
use egui::{Id, Key, Rect, Response, Sense, Stroke, StrokeKind, WidgetType, pos2, vec2};
use serde_json::{Value, json};

/// How far (screen points) a stop dragged off the bar must go to be removed.
const TEAR_OFF: f32 = 26.0;
/// The stop bar's width: the colour picker's swatch grid under it.
const BAR_WIDTH: f32 = 270.0;

/// What the fill picker asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum FillEdit {
    /// A solid colour (opaque).
    Solid([u8; 3]),
    /// No fill (true), or a fill again (false).
    None(bool),
    /// `paint.set_fill` params, with its undo label; `live` while a drag or typing goes on (one
    /// undo step for all of it).
    Set { label: &'static str, live: bool, params: Value },
}

/// The fill the pickers show: the first selected shape's on the active layer, else the swatch.
/// True when it is the selection's.
pub fn current(app: &App) -> (Paint, bool) {
    let selected = app.s.selected_elements().into_iter().filter(|(l, _)| *l == app.s.layer).find_map(|(_, e)| match &e.kind {
        ElementKind::Shape(sh) => sh.fill.clone(),
        _ => None,
    });
    match selected {
        Some(p) => (p, true),
        None => (app.s.style.fill_paint(), false),
    }
}

/// The Library's bitmaps, for a bitmap fill: `(id, name)`.
pub fn bitmaps(app: &App) -> Vec<(u64, String)> {
    app.s.doc.bitmaps.iter().map(|b| (b.id.0, b.name.clone())).collect()
}

/// Applies `edit` as `paint.set_fill` (`selection`: also to the selected fills). Solid and None
/// edits are the caller's (each place keeps its own rule for those).
pub fn apply_set(app: &mut App, edit: &FillEdit, selection: bool) {
    if let FillEdit::Set { label, live, params } = edit {
        let mut p = params.clone();
        if !selection && let Some(o) = p.as_object_mut() {
            o.insert("selection".into(), json!(false));
        }
        if *live {
            crate::panels::live(app, label, "paint.set_fill", p);
        } else {
            app.run("paint.set_fill", p);
        }
    }
}

/// `#rrggbb`, or `#rrggbbaa` when not opaque.
fn hex(c: Rgba) -> String {
    let c = c.sanitized();
    if c.a >= 1.0 { c.to_hex() } else { format!("{}{:02x}", c.to_hex(), (c.a * 255.0).round() as u8) }
}

fn stops_json(stops: &[GradientStop]) -> Value {
    json!(stops.iter().map(|s| json!({"offset": s.offset.clamp(0.0, 1.0), "color": hex(s.color)})).collect::<Vec<_>>())
}

fn bytes(c: Rgba) -> [u8; 4] {
    let c = c.sanitized();
    let b = |v: f32| (v * 255.0).round() as u8;
    [b(c.r), b(c.g), b(c.b), b(c.a)]
}

fn from_bytes(c: [u8; 4]) -> Rgba {
    Rgba::new(f32::from(c[0]) / 255.0, f32::from(c[1]) / 255.0, f32::from(c[2]) / 255.0, f32::from(c[3]) / 255.0)
}

/// Paints `paint` into `r`: a colour, a gradient's ramp, a bitmap's mark, or the empty field
/// with a slash for no fill.
fn paint_preview(p: &egui::Painter, r: Rect, paint: Option<&Paint>) {
    match paint {
        None => {
            p.rect_filled(r, 0.0, T.field);
            p.line_segment([r.left_bottom(), r.right_top()], Stroke::new(1.5, T.danger));
        }
        Some(Paint::Solid(c)) => egui::color_picker::show_color_at(p, to_color32(*c), r),
        Some(Paint::Bitmap { .. }) => {
            p.rect_filled(r, 0.0, T.text_faint);
            crate::icons::paint(p, r.shrink(2.0), crate::icons::Icon::Image, T.panel);
        }
        Some(g) => {
            let stops = g.stops().unwrap_or(&[]);
            let mut mesh = egui::Mesh::default();
            let steps = 24u32;
            for i in 0..=steps {
                let t = i as f32 / steps as f32;
                let c = to_color32(ramp_at(stops, t));
                let x = r.left() + t * r.width();
                mesh.colored_vertex(pos2(x, r.top()), c);
                mesh.colored_vertex(pos2(x, r.bottom()), c);
                if i > 0 {
                    let k = 2 * i;
                    mesh.add_triangle(k - 2, k - 1, k);
                    mesh.add_triangle(k - 1, k, k + 1);
                }
            }
            p.add(mesh);
        }
    }
}

/// A fill swatch for Properties: shows the fill (a gradient as its ramp) and opens the fill
/// picker. Name it with `Named::named`.
pub fn swatch(ui: &mut egui::Ui, paint: Option<&Paint>, bitmaps: &[(u64, String)]) -> (Response, Option<FillEdit>) {
    let size = ui.spacing().interact_size;
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::new(WidgetType::ColorButton));
    let popup = resp.id.with("colour_popup");
    let open = egui::Popup::is_id_open(ui.ctx(), popup);
    if ui.is_rect_visible(rect) {
        paint_preview(ui.painter(), rect.shrink(1.0), paint);
        ui.painter().rect_stroke(rect, T.rc(), Stroke::new(1.0, if open || resp.hovered() { T.selection } else { T.border }), StrokeKind::Inside);
    }
    let edit = popup_for(ui, &resp, popup, paint, bitmaps);
    if edit.is_some() {
        resp.mark_changed();
    }
    (resp, edit)
}

/// Opens the fill picker under `anchor` when it is clicked, and shows it while open. `paint`
/// is the fill (`None`: no fill).
pub fn popup_for(ui: &egui::Ui, anchor: &Response, popup: Id, paint: Option<&Paint>, bitmaps: &[(u64, String)]) -> Option<FillEdit> {
    if anchor.clicked() {
        crate::colour::new_session(ui.ctx());
    }
    let mut edit = None;
    let shown = egui::Popup::menu(anchor).id(popup).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        edit = fill_picker(ui, popup, paint, bitmaps);
    });
    if let Some(r) = shown {
        crate::colour::name_popup(&r.response, tr!("Fill colour picker"));
    }
    edit
}

/// The kinds of fill on the type row, as Animate's Color Type menu: key, label, what it is.
fn kinds() -> [(&'static str, &'static str, &'static str); 5] {
    [
        ("none", tr!("None"), tr!("No fill")),
        ("solid", tr!("Solid"), tr!("Solid colour")),
        ("linear", tr!("Linear"), tr!("Linear gradient: colours along a line")),
        ("radial", tr!("Radial"), tr!("Radial gradient: colours out from a centre")),
        ("bitmap", tr!("Bitmap"), tr!("Bitmap fill: a Library image, tiled")),
    ]
}

/// The picker's body: the type row, then what that type has.
pub fn fill_picker(ui: &mut egui::Ui, popup: Id, paint: Option<&Paint>, bitmaps: &[(u64, String)]) -> Option<FillEdit> {
    let now = match paint {
        None => "none",
        Some(Paint::Solid(_)) => "solid",
        Some(Paint::Linear { .. }) => "linear",
        Some(Paint::Radial { .. }) => "radial",
        Some(Paint::Bitmap { .. }) => "bitmap",
    };
    let mut pick = now;
    ui.horizontal(|ui| {
        for (key, label, tip) in kinds() {
            let usable = key != "bitmap" || !bitmaps.is_empty();
            let r = ui.add_enabled(usable, egui::Button::selectable(pick == key, label)).named(&tr!("Fill type: {kind}", kind = tip));
            let r = if usable { r.on_hover_text(tip) } else { r.on_disabled_hover_text(tr!("Import an image into the Library first (File › Import › Import to Library), then fill with it.")) };
            if r.clicked() {
                pick = key;
            }
        }
    });
    ui.add_space(4.0);
    // The document's gradient swatches (ADR 0078): a click fills with one.
    if let Some(edit) = gradient_swatches(ui, paint) {
        return Some(edit);
    }
    if pick != now {
        return Some(match pick {
            "none" => FillEdit::None(true),
            "solid" => match paint {
                // A fill again: the colour it shows.
                None => FillEdit::None(false),
                Some(p) => FillEdit::Set { label: tr_noop!("Fill"), live: false, params: json!({"type": "solid", "color": hex(p.primary_color().with_alpha(1.0))}) },
            },
            "bitmap" => FillEdit::Set { label: tr_noop!("Fill"), live: false, params: json!({"type": "bitmap", "bitmap": bitmaps.first().map_or(0, |b| b.0)}) },
            other => FillEdit::Set { label: tr_noop!("Fill"), live: false, params: json!({"type": other}) },
        });
    }
    match paint {
        None => {
            ui.label(text(Type::Caption, tr!("No fill. Choose a type above to fill again.")));
            None
        }
        Some(Paint::Solid(c)) => {
            let mut px = bytes(*c);
            crate::colour::picker(ui, popup, &mut px, false, None).then_some(FillEdit::Solid([px[0], px[1], px[2]]))
        }
        Some(Paint::Linear { stops, spread, linear_rgb, .. }) => gradient(ui, popup, stops, *spread, *linear_rgb, None),
        Some(Paint::Radial { stops, spread, linear_rgb, focal, .. }) => gradient(ui, popup, stops, *spread, *linear_rgb, Some(*focal)),
        Some(Paint::Bitmap { bitmap, .. }) => {
            let current = bitmap.0;
            let mut want = current;
            ui.horizontal(|ui| {
                ui.label(tr!("Bitmap"));
                egui::ComboBox::from_id_salt(popup.with("fill_bitmap"))
                    .width(180.0)
                    .selected_text(bitmaps.iter().find(|c| c.0 == current).map_or_else(|| tr!("missing bitmap").to_string(), |c| c.1.clone()))
                    .show_ui(ui, |ui| {
                        for (id, name) in bitmaps {
                            ui.selectable_value(&mut want, *id, name.as_str());
                        }
                    })
                    .response
                    .set_name(tr!("Fill bitmap"));
            });
            ui.label(text(Type::Caption, tr!("Scale, turn and skew it with the Gradient Transform tool (F).")));
            (want != current).then(|| FillEdit::Set { label: tr_noop!("Fill"), live: false, params: json!({"type": "bitmap", "bitmap": want}) })
        }
    }
}

/// The document's gradient swatches as a row (two at most; the Swatches panel has them all).
/// The fill a click asks for.
fn gradient_swatches(ui: &mut egui::Ui, paint: Option<&Paint>) -> Option<FillEdit> {
    const CELL: f32 = 16.0;
    const PER_ROW: usize = 16;
    let all = crate::swatches_panel::for_pickers(ui.ctx());
    let grads: Vec<&anicraft_engine::doc::Swatch> = all.iter().filter(|s| s.paint.is_gradient()).take(PER_ROW * 2).collect();
    if grads.is_empty() {
        return None;
    }
    let mut out = None;
    ui.label(text(Type::Caption, tr!("Gradient swatches")));
    for row in grads.chunks(PER_ROW) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for sw in row {
                let (r, resp) = ui.allocate_exact_size(vec2(CELL, CELL), Sense::click());
                crate::swatches_panel::preview(ui.painter(), r, &sw.paint);
                let on = paint.is_some_and(|p| crate::swatches_panel::matches(&sw.paint, p));
                let edge = if on { Stroke::new(2.0, T.accent) } else if resp.hovered() { Stroke::new(1.5, T.selection) } else { Stroke::new(1.0, T.border) };
                ui.painter().rect_stroke(r, 0.0, edge, StrokeKind::Outside);
                let name = crate::swatches_panel::describe(&sw.paint, &sw.name);
                a11y::control(ui, &resp, WidgetType::Button, &tr!("Gradient swatch: {name}", name = name.as_str()), Some(on));
                if resp.on_hover_text(name).clicked() {
                    out = Some(FillEdit::Set { label: tr_noop!("Fill"), live: false, params: crate::swatches_panel::fill_params(&sw.paint) });
                }
            }
        });
    }
    ui.add_space(4.0);
    out
}

/// A gradient: overflow, Linear RGB and the focal point, the stop bar, the picked stop's
/// position, and the colour picker for its colour.
fn gradient(ui: &mut egui::Ui, popup: Id, stops: &[GradientStop], spread: Spread, linear_rgb: bool, focal: Option<f32>) -> Option<FillEdit> {
    let set = |label: &'static str, live: bool, params: Value| Some(FillEdit::Set { label, live, params });
    let mut out = None;
    let mut sp = spread;
    ui.horizontal(|ui| {
        ui.label(tr!("Overflow"));
        for (s, label, tip) in [
            (Spread::Pad, tr!("Extend"), tr!("Past its ends the gradient keeps its end colours")),
            (Spread::Reflect, tr!("Reflect"), tr!("Past its ends the gradient mirrors back and forth")),
            (Spread::Repeat, tr!("Repeat"), tr!("Past its ends the gradient starts again")),
        ] {
            ui.selectable_value(&mut sp, s, label).on_hover_text(tip);
        }
    });
    if sp != spread {
        out = set(tr_noop!("Overflow"), false, json!({"spread": sp.key()}));
    }
    ui.horizontal(|ui| {
        let mut lin = linear_rgb;
        if crate::theme::checkbox(ui, &mut lin, tr!("Linear RGB")).on_hover_text(tr!("Mix the colours in linear light: brighter, more even blends")).changed() {
            out = set(tr_noop!("Linear RGB"), false, json!({"linear_rgb": lin}));
        }
        if let Some(f) = focal {
            ui.add_space(8.0);
            ui.label(tr!("Focal point"));
            let mut pct = f * 100.0;
            let lim = MAX_FOCAL * 100.0;
            let r = with_type(ui, Type::Data, |ui| ui.add(egui::DragValue::new(&mut pct).suffix(" %").speed(0.5).range(-lim..=lim).max_decimals(0)));
            if r.named(tr!("Focal point")).on_hover_text(tr!("Where the ramp starts, from the centre along the gradient's axis (drag the triangle with the Gradient Transform tool)")).changed() {
                out = set(tr_noop!("Focal point"), true, json!({"focal": (pct / 100.0).clamp(-MAX_FOCAL, MAX_FOCAL)}));
            }
        }
    });
    ui.add_space(4.0);
    let mut edited = stops.to_vec();
    if let Some((label, live)) = stop_bar(ui, popup, &mut edited) {
        out = set(label, live, json!({"stops": stops_json(&edited)}));
    }
    out
}

/// Where stop `i` may move: between its neighbours, so the order (and the pick) stays.
fn room(stops: &[GradientStop], i: usize) -> (f32, f32) {
    let lo = i.checked_sub(1).and_then(|j| stops.get(j)).map_or(0.0, |s| s.offset);
    let hi = stops.get(i + 1).map_or(1.0, |s| s.offset);
    (lo.min(hi), hi.max(lo))
}

/// Moves stop `i` to `offset`, held between its neighbours.
fn move_stop(stops: &mut [GradientStop], i: usize, offset: f32) {
    let (lo, hi) = room(stops, i);
    if let Some(s) = stops.get_mut(i) {
        s.offset = if offset.is_finite() { offset.clamp(lo, hi) } else { s.offset };
    }
}

/// The middle of the widest gap between stops: where the keyboard adds one.
fn widest_gap(stops: &[GradientStop]) -> f32 {
    stops.windows(2).filter_map(|w| if let [a, b] = w { Some((b.offset - a.offset, (a.offset + b.offset) / 2.0)) } else { None }).max_by(|a, b| a.0.total_cmp(&b.0)).map_or(0.5, |g| g.1)
}

/// Adds a stop at `t` in the colour the ramp has there; its index, or `None` when the gradient
/// is full.
fn add_stop(stops: &mut Vec<GradientStop>, t: f32) -> Option<usize> {
    if stops.len() >= MAX_GRADIENT_STOPS {
        return None;
    }
    let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.5 };
    let i = stops.iter().position(|s| s.offset > t).unwrap_or(stops.len());
    let color = ramp_at(stops, t);
    stops.insert(i, GradientStop { offset: t, color });
    Some(i)
}

/// Removes stop `i` unless only two are left. True when it went.
fn remove_stop(stops: &mut Vec<GradientStop>, i: usize) -> bool {
    if stops.len() <= 2 || i >= stops.len() {
        return false;
    }
    stops.remove(i);
    true
}

/// The bar of colour stops, the picked stop's position and its colour (the colour picker,
/// with opacity). What changed (its undo label), and whether it is live (a drag or typing).
fn stop_bar(ui: &mut egui::Ui, popup: Id, stops: &mut Vec<GradientStop>) -> Option<(&'static str, bool)> {
    let sel_id = popup.with("fill_stop");
    let tear_id = popup.with("fill_stop_tear");
    let mut sel: usize = ui.data(|d| d.get_temp(sel_id)).unwrap_or(0).min(stops.len().saturating_sub(1));
    let mut changed = None;
    let can_remove = stops.len() > 2;
    let w = BAR_WIDTH;
    let (strip, resp) = ui.allocate_exact_size(vec2(w, 18.0), Sense::click());
    a11y::control(ui, &resp, WidgetType::Button, tr!("Gradient: add a colour stop"), None);
    let resp = resp.on_hover_text(tr!("Click to add a colour stop. Drag a stop to move it, or off the bar to remove it. With a stop focused, the arrow keys move it and Delete removes it."));
    let p = ui.painter();
    // A checkerboard under the ramp shows its opacity.
    let cell = 4.0;
    let (cols, rows) = ((strip.width() / cell).ceil() as usize, (strip.height() / cell).ceil() as usize);
    p.rect_filled(strip, 0.0, T.header);
    for (cx, cy) in (0..cols).flat_map(|x| (0..rows).map(move |y| (x, y))).filter(|(x, y)| (x + y) % 2 == 0) {
        let r = Rect::from_min_size(pos2(strip.left() + cx as f32 * cell, strip.top() + cy as f32 * cell), vec2(cell, cell)).intersect(strip);
        p.rect_filled(r, 0.0, T.text_faint);
    }
    let mut mesh = egui::Mesh::default();
    let steps = 64u32;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let c = to_color32(ramp_at(stops, t));
        let x = strip.left() + t * strip.width();
        mesh.colored_vertex(pos2(x, strip.top()), c);
        mesh.colored_vertex(pos2(x, strip.bottom()), c);
        if i > 0 {
            let k = 2 * i;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k, k + 1);
        }
    }
    p.add(mesh);
    p.rect_stroke(strip, 0.0, Stroke::new(1.0, T.border), StrokeKind::Outside);
    // A marker under the bar per stop: click to pick it, drag to move it, drag off to remove it.
    let (track, _) = ui.allocate_exact_size(vec2(w, 14.0), Sense::hover());
    let mut remove: Option<usize> = None;
    let mut tearing: Option<usize> = ui.data(|d| d.get_temp(tear_id));
    let count = stops.len();
    for i in 0..count {
        let Some(off) = stops.get(i).map(|s| s.offset) else { continue };
        let x = strip.left() + off.clamp(0.0, 1.0) * strip.width();
        let hit = Rect::from_center_size(pos2(x, track.center().y), vec2(12.0, 14.0));
        let r = ui.interact(hit, popup.with(("fill_stop_marker", i)), Sense::click_and_drag());
        let at = format!("{:.0}", off * 100.0);
        a11y::control(ui, &r, WidgetType::Button, &tr!("Colour stop {stop} of {count} at {at}%", stop = i + 1, count = count, at = at), Some(i == sel));
        if r.clicked() || r.drag_started() || r.gained_focus() {
            sel = i;
        }
        if r.dragged()
            && let Some(pt) = r.interact_pointer_pos()
        {
            sel = i;
            // Far enough above or below the bar: the stop comes off when let go.
            let off_bar = can_remove && (pt.y - track.center().y).abs() > TEAR_OFF && (pt.y - strip.center().y).abs() > TEAR_OFF;
            tearing = off_bar.then_some(i);
            if !off_bar {
                move_stop(stops, i, (pt.x - strip.left()) / strip.width());
                changed = Some((tr_noop!("Move colour stop"), true));
            }
        }
        if r.drag_stopped() && tearing == Some(i) {
            remove = Some(i);
            tearing = None;
        }
        // The keyboard: arrows move the focused stop (Shift: by 10 %), Delete removes it.
        if r.has_focus() {
            let (left, right, shift, del) = ui.input(|inp| (inp.key_pressed(Key::ArrowLeft), inp.key_pressed(Key::ArrowRight), inp.modifiers.shift, inp.key_pressed(Key::Delete) || inp.key_pressed(Key::Backspace)));
            let step = if shift { 0.1 } else { 0.01 };
            let d = if right { step } else if left { -step } else { 0.0 };
            if d != 0.0 {
                move_stop(stops, i, off + d);
                changed = Some((tr_noop!("Move colour stop"), true));
            }
            if del && can_remove {
                remove = Some(i);
            }
            // Arrow keys stay on the stop rather than moving focus.
            ui.memory_mut(|m| m.set_focus_lock_filter(r.id, egui::EventFilter { horizontal_arrows: true, ..Default::default() }));
        }
        let Some(st) = stops.get(i) else { continue };
        let torn = tearing == Some(i);
        let tip = pos2(x, track.top());
        let body = [tip, pos2(x + 6.0, track.top() + 6.0), pos2(x + 6.0, track.bottom()), pos2(x - 6.0, track.bottom()), pos2(x - 6.0, track.top() + 6.0)];
        let edge = if i == sel { T.accent } else if r.hovered() { T.text } else { T.control_edge };
        let fill = to_color32(st.color.with_alpha(if torn { 0.3 } else { 1.0 }));
        ui.painter().add(egui::Shape::convex_polygon(body.to_vec(), fill, Stroke::new(if i == sel { 2.0 } else { 1.0 }, edge)));
    }
    ui.data_mut(|d| match tearing {
        Some(t) => {
            d.insert_temp(tear_id, t);
        }
        None => d.remove::<usize>(tear_id),
    });
    if resp.clicked() {
        // Where the pointer clicked; from the keyboard or a screen reader, the middle of the
        // widest gap.
        let t = resp.interact_pointer_pos().map_or_else(|| widest_gap(stops), |at| (at.x - strip.left()) / strip.width());
        if let Some(i) = add_stop(stops, t) {
            sel = i;
            changed = Some((tr_noop!("Add colour stop"), false));
        }
    }
    if let Some(i) = remove.take()
        && remove_stop(stops, i)
    {
        sel = sel.min(stops.len().saturating_sub(1));
        changed = Some((tr_noop!("Remove colour stop"), false));
    }
    // The picked stop: its position, then its colour in the colour picker.
    let n = stops.len();
    if let Some(st) = stops.get_mut(sel) {
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.label(tr!("Stop {stop} at", stop = sel + 1));
            let mut pct = st.offset * 100.0;
            let r = with_type(ui, Type::Data, |ui| ui.add(egui::DragValue::new(&mut pct).suffix(" %").speed(0.5).range(0.0..=100.0).max_decimals(0)));
            if r.named(tr!("Colour stop position")).changed() {
                st.offset = (pct / 100.0).clamp(0.0, 1.0);
                changed = Some((tr_noop!("Move colour stop"), true));
            }
            if ui.add_enabled(n > 2, egui::Button::new(tr!("Remove")).small()).named(tr!("Remove colour stop")).on_hover_text(tr!("Remove this colour stop (or drag it off the bar)")).clicked() {
                remove = Some(sel);
            }
        });
        ui.add_space(4.0);
        let mut px = bytes(st.color);
        if crate::colour::picker(ui, popup.with(("stop", sel)), &mut px, true, None) {
            st.color = from_bytes(px);
            changed = Some((tr_noop!("Colour stop"), true));
        }
    }
    if let Some(i) = remove
        && remove_stop(stops, i)
    {
        sel = sel.min(stops.len().saturating_sub(1));
        changed = Some((tr_noop!("Remove colour stop"), false));
    }
    // Typed positions may pass a neighbour: keep the stops in order (the pick follows its stop).
    if let Some(picked) = stops.get(sel).copied() {
        stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
        sel = stops.iter().position(|s| *s == picked).unwrap_or(sel);
    }
    ui.data_mut(|d| d.insert_temp(sel_id, sel));
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Tool;
    use anicraft_engine::geom::Vec2;
    use std::path::PathBuf;

    fn stop(offset: f32, c: u32) -> GradientStop {
        GradientStop { offset, color: Rgba::hex(c) }
    }

    #[test]
    fn stops_are_added_moved_and_removed_in_order() {
        let mut st = vec![stop(0.0, 0x000000), stop(1.0, 0xffffff)];
        // Added where asked, in the ramp's colour there, in order.
        assert_eq!(add_stop(&mut st, 0.25), Some(1));
        assert!((st[1].color.r - 0.25).abs() < 1e-6);
        assert_eq!(widest_gap(&st), 0.625);
        assert_eq!(add_stop(&mut st, f32::NAN), Some(2), "a bad position lands in the middle");
        // A stop moves only between its neighbours.
        move_stop(&mut st, 1, 0.9);
        assert_eq!(st[1].offset, 0.5);
        move_stop(&mut st, 1, -1.0);
        assert_eq!(st[1].offset, 0.0);
        move_stop(&mut st, 9, 0.5);
        // Full at 15 stops; never fewer than 2.
        while add_stop(&mut st, 0.7).is_some() {}
        assert_eq!(st.len(), MAX_GRADIENT_STOPS);
        while remove_stop(&mut st, 0) {}
        assert_eq!(st.len(), 2);
        assert!(!remove_stop(&mut st, 5));
        assert_eq!(hex(Rgba::new(1.0, 0.0, 0.0, 0.5)), "#ff000080");
        assert_eq!(hex(Rgba::hex(0x00ff00)), "#00ff00");
    }

    /// Draws a few frames of the whole window.
    fn frames(ctx: &egui::Context, app: &mut App) {
        let screen = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1600.0, 1400.0))), ..Default::default() };
        for _ in 0..2 {
            let mut out = ctx.run_ui(screen.clone(), |ui| app.frame(ui));
            out.textures_delta.clear();
        }
    }

    /// The fill picker drawn alone for `paint`: what it asked for with no input.
    fn picker_once(paint: Option<&Paint>, bitmaps: &[(u64, String)]) -> Option<FillEdit> {
        let ctx = egui::Context::default();
        let mut edit = Some(FillEdit::None(false));
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| edit = fill_picker(ui, Id::new("test_fill"), paint, bitmaps));
        out.textures_delta.clear();
        edit
    }

    #[test]
    fn the_picker_shows_every_kind_of_fill_without_changing_it() {
        let two = vec![stop(0.0, 0x000000), stop(1.0, 0xffffff)];
        let bitmaps = vec![(7u64, "tile.png".to_string())];
        let mut radial = Paint::radial(Vec2::ZERO, 10.0, two.clone());
        if let Paint::Radial { focal, .. } = &mut radial {
            *focal = 0.4;
        }
        for p in [None, Some(Paint::Solid(Rgba::hex(0x3d8bfd))), Some(Paint::linear(Vec2::ZERO, Vec2::new(10.0, 0.0), two)), Some(radial), Some(Paint::Bitmap { bitmap: anicraft_engine::doc::BitmapId(7), repeat: true, matrix: anicraft_engine::geom::Affine::IDENTITY })] {
            assert_eq!(picker_once(p.as_ref(), &bitmaps), None, "{p:?}");
        }
    }

    #[test]
    fn edits_from_the_picker_reach_the_swatch_and_the_selection() {
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, anicraft_engine::Session::default(), Some(PathBuf::from("unused.png")));
        app.run("draw.rect", json!({"x": 10, "y": 10, "w": 100, "h": 80, "fill": "#ff0000", "stroke": null}));
        app.tool = Tool::Select;
        assert!(matches!(current(&app), (Paint::Solid(_), true)));
        // A type change is one step on the selection and the swatch.
        let steps = app.s.history_len();
        apply_set(&mut app, &FillEdit::Set { label: "Fill", live: false, params: json!({"type": "radial"}) }, true);
        assert_eq!(app.s.history_len(), steps + 1);
        assert!(matches!(current(&app).0, Paint::Radial { .. }) && matches!(app.s.style.fill_paint, Some(Paint::Radial { .. })));
        // A drag of a stop's colour is one step however many frames it takes.
        let steps = app.s.history_len();
        for c in ["#112233", "#445566", "#778899"] {
            let stops = json!([{"offset": 0, "color": c}, {"offset": 1, "color": "#000000"}]);
            apply_set(&mut app, &FillEdit::Set { label: "Colour stop", live: true, params: json!({"stops": stops}) }, true);
        }
        app.s.end_gesture();
        assert_eq!(app.s.history_len(), steps + 1);
        // From the tool options: the swatch only.
        apply_set(&mut app, &FillEdit::Set { label: "Fill", live: false, params: json!({"type": "linear"}) }, false);
        assert!(matches!(current(&app).0, Paint::Radial { .. }), "the selection keeps its fill");
        assert!(matches!(app.s.style.fill_paint, Some(Paint::Linear { .. })));
        // Solid and None are the caller's: apply_set leaves them alone.
        let before = app.s.doc.clone();
        apply_set(&mut app, &FillEdit::Solid([1, 2, 3]), true);
        apply_set(&mut app, &FillEdit::None(true), true);
        assert!(app.s.doc == before);
        // The whole window draws with the Tools square's picker open on the selection's gradient,
        // and on the swatch with a fill tool in hand.
        app.open_fill_swatch = true;
        frames(&ctx, &mut app);
        app.run("edit.deselect", json!({}));
        app.tool = Tool::PaintBucket;
        frames(&ctx, &mut app);
        assert!(!current(&app).1);
        assert_eq!(stops_json(&[stop(0.0, 0xff0000)])[0]["color"], "#ff0000");
        assert_eq!(from_bytes(bytes(Rgba::new(1.0, 0.0, 0.0, 0.5))).a, 128.0 / 255.0);
    }
}
