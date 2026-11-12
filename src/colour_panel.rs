//! The Colour panel (ADR 0074): the fill's type (solid colour, linear or radial gradient,
//! bitmap), a gradient's colour stops on a bar, its overflow (Extend, Reflect, Repeat), Linear
//! RGB and focal point, and a bitmap fill's image.
//!
//! It edits the selected shapes' fill when one is selected, else the fill swatch (what new
//! shapes, the Paint Bucket and the Brush pour). Every change is `paint.set_fill`, which sets
//! both, so agents and the CLI work the same way; drags are one undo step.

use crate::a11y::{self, Named};
use crate::app::App;
use crate::theme::{T, Type, text, to_color32, with_type};
use anicraft_engine::doc::{ElementKind, GradientStop, MAX_FOCAL, MAX_GRADIENT_STOPS, Paint, Rgba, Spread, ramp_at};
use egui::{Key, Rect, Sense, Stroke, StrokeKind, WidgetType, pos2, vec2};
use serde_json::{Value, json};

/// How far (screen points) a stop dragged off the bar must go to be removed.
const TEAR_OFF: f32 = 26.0;

fn row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    crate::theme::form_row(ui, crate::theme::FORM_LABEL_DOCK, label, |ui| with_type(ui, Type::Data, add));
}

fn row_prose(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    crate::theme::form_row(ui, crate::theme::FORM_LABEL_DOCK, label, add);
}

/// The fill the panel shows: the first selected shape's on the active layer, else the swatch.
fn current(app: &App) -> (Paint, bool) {
    let selected = app.s.selected_elements().into_iter().filter(|(l, _)| *l == app.s.layer).find_map(|(_, e)| match &e.kind {
        ElementKind::Shape(sh) => sh.fill.clone(),
        _ => None,
    });
    match selected {
        Some(p) => (p, true),
        None => (app.s.style.fill_paint(), false),
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

/// Runs `paint.set_fill`: a drag or typing (`live`) coalesces into one undo step.
fn set(app: &mut App, label: &str, live: bool, p: Value) {
    if live {
        crate::panels::live(app, label, "paint.set_fill", p);
    } else {
        app.run("paint.set_fill", p);
    }
}

/// A colour swatch with opacity and its hex field. True when either changed `c`.
fn colour_field(ui: &mut egui::Ui, c: &mut Rgba, name: &str) -> bool {
    let mut changed = crate::panels::alpha_swatch(ui, c, name);
    // The field keeps what is being typed while it has focus; it applies as soon as it parses.
    let id = ui.id().with(("hex", name));
    let mut s: String = ui.data(|d| d.get_temp(id)).unwrap_or_else(|| hex(*c));
    let r = ui.add(egui::TextEdit::singleline(&mut s).desired_width(78.0).char_limit(9).font(egui::TextStyle::Monospace)).named(&tr!("{name} (hex)", name = name));
    if r.changed()
        && let Some(v) = Rgba::parse_hex(&s)
    {
        *c = v;
        changed = true;
    }
    if r.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, s));
    } else {
        ui.data_mut(|d| d.remove::<String>(id));
    }
    changed
}

pub fn panel(app: &mut App, ui: &mut egui::Ui) {
    let (paint, selected) = current(app);
    ui.label(text(Type::Caption, if selected { tr!("The selected fill") } else { tr!("The fill swatch: new shapes, the Paint Bucket and the Brush") }));
    ui.add_space(2.0);
    // The type, as Animate's Color Type menu.
    let kinds = [("solid", tr!("Solid colour")), ("linear", tr!("Linear gradient")), ("radial", tr!("Radial gradient")), ("bitmap", tr!("Bitmap fill"))];
    let now = match paint {
        Paint::Solid(_) => "solid",
        Paint::Linear { .. } => "linear",
        Paint::Radial { .. } => "radial",
        Paint::Bitmap { .. } => "bitmap",
    };
    let mut pick = now;
    row_prose(ui, tr!("Type"), |ui| {
        egui::ComboBox::from_id_salt("fill_type")
            .width(150.0)
            .selected_text(kinds.iter().find(|k| k.0 == now).map_or("", |k| k.1))
            .show_ui(ui, |ui| {
                for (key, label) in &kinds {
                    ui.selectable_value(&mut pick, *key, *label);
                }
            })
            .response
            .set_name(tr!("Fill type"));
    });
    if pick != now {
        if pick == "bitmap" {
            match app.s.doc.bitmaps.first().map(|b| b.id) {
                Some(b) => set(app, tr_noop!("Fill"), false, json!({"type": "bitmap", "bitmap": b.0})),
                None => app.status_err(tr!("Import an image into the Library first (File › Import › Import to Library), then fill with it.")),
            }
        } else {
            set(app, tr_noop!("Fill"), false, json!({"type": pick}));
        }
        return;
    }
    match &paint {
        Paint::Solid(c) => {
            let mut c = *c;
            row(ui, tr!("Colour"), |ui| {
                if colour_field(ui, &mut c, tr!("Fill colour")) {
                    set(app, tr_noop!("Fill colour"), true, json!({"type": "solid", "color": hex(c)}));
                }
            });
        }
        Paint::Linear { stops, spread, linear_rgb, .. } | Paint::Radial { stops, spread, linear_rgb, .. } => {
            gradient_rows(app, ui, stops, *spread, *linear_rgb);
            if let Paint::Radial { focal, .. } = &paint {
                let mut pct = focal * 100.0;
                row(ui, tr!("Focal point"), |ui| {
                    let lim = MAX_FOCAL * 100.0;
                    let r = ui.add(egui::DragValue::new(&mut pct).suffix(" %").speed(0.5).range(-lim..=lim).max_decimals(0)).named(tr!("Focal point")).on_hover_text(tr!("Where the ramp starts, from the centre along the gradient's axis (drag the triangle with the Gradient Transform tool)"));
                    if r.changed() {
                        set(app, tr_noop!("Focal point"), true, json!({"focal": (pct / 100.0).clamp(-MAX_FOCAL, MAX_FOCAL)}));
                    }
                });
            }
        }
        Paint::Bitmap { bitmap, .. } => {
            let current = *bitmap;
            let choices: Vec<(u64, String)> = app.s.doc.bitmaps.iter().map(|b| (b.id.0, b.name.clone())).collect();
            let mut want = current.0;
            row_prose(ui, tr!("Bitmap"), |ui| {
                egui::ComboBox::from_id_salt("fill_bitmap")
                    .width(150.0)
                    .selected_text(choices.iter().find(|c| c.0 == current.0).map_or_else(|| tr!("missing bitmap").to_string(), |c| c.1.clone()))
                    .show_ui(ui, |ui| {
                        for (id, name) in &choices {
                            ui.selectable_value(&mut want, *id, name.as_str());
                        }
                    })
                    .response
                    .set_name(tr!("Fill bitmap"));
            });
            if want != current.0 {
                set(app, tr_noop!("Fill"), false, json!({"type": "bitmap", "bitmap": want}));
            }
        }
    }
}

/// Overflow, Linear RGB, the stop bar and the picked stop's colour and position.
fn gradient_rows(app: &mut App, ui: &mut egui::Ui, stops: &[GradientStop], spread: Spread, linear_rgb: bool) {
    let mut sp = spread;
    row_prose(ui, tr!("Overflow"), |ui| {
        for (s, label, tip) in [
            (Spread::Pad, tr!("Extend"), tr!("Past its ends the gradient keeps its end colours")),
            (Spread::Reflect, tr!("Reflect"), tr!("Past its ends the gradient mirrors back and forth")),
            (Spread::Repeat, tr!("Repeat"), tr!("Past its ends the gradient starts again")),
        ] {
            ui.selectable_value(&mut sp, s, label).on_hover_text(tip);
        }
    });
    if sp != spread {
        set(app, tr_noop!("Overflow"), false, json!({"spread": sp.key()}));
    }
    let mut lin = linear_rgb;
    row_prose(ui, tr!("Blending"), |ui| {
        if crate::theme::checkbox(ui, &mut lin, tr!("Linear RGB")).on_hover_text(tr!("Mix the colours in linear light: brighter, more even blends")).changed() {
            set(app, tr_noop!("Linear RGB"), false, json!({"linear_rgb": lin}));
        }
    });
    let mut edited = stops.to_vec();
    if let Some(label) = stop_bar(ui, &mut edited) {
        set(app, label, true, json!({"stops": stops_json(&edited)}));
    }
}

/// What the bar did.
fn stop_bar(ui: &mut egui::Ui, stops: &mut Vec<GradientStop>) -> Option<&'static str> {
    let sel_id = ui.id().with("fill_stop");
    let tear_id = ui.id().with("fill_stop_tear");
    let mut sel: usize = ui.data(|d| d.get_temp(sel_id)).unwrap_or(0).min(stops.len().saturating_sub(1));
    let mut changed = None;
    // Each stop moves between its neighbours, so the order (and the pick) stays.
    let room = |stops: &[GradientStop], i: usize| -> (f32, f32) {
        let lo = i.checked_sub(1).and_then(|j| stops.get(j)).map_or(0.0, |s| s.offset);
        let hi = stops.get(i + 1).map_or(1.0, |s| s.offset);
        (lo.min(hi), hi.max(lo))
    };
    let can_remove = stops.len() > 2;
    let w = ui.available_width().clamp(120.0, 240.0);
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
        let r = ui.interact(hit, ui.id().with(("fill_stop_marker", i)), Sense::click_and_drag());
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
                let (lo, hi) = room(stops, i);
                if let Some(s) = stops.get_mut(i) {
                    s.offset = ((pt.x - strip.left()) / strip.width()).clamp(lo, hi);
                }
                changed = Some(tr_noop!("Move colour stop"));
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
                let (lo, hi) = room(stops, i);
                if let Some(s) = stops.get_mut(i) {
                    s.offset = (s.offset + d).clamp(lo, hi);
                }
                changed = Some(tr_noop!("Move colour stop"));
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
    if resp.clicked() && stops.len() < MAX_GRADIENT_STOPS {
        // Where the pointer clicked; from the keyboard or a screen reader, the middle of the
        // widest gap.
        let gap = stops.windows(2).filter_map(|w| if let [a, b] = w { Some((b.offset - a.offset, (a.offset + b.offset) / 2.0)) } else { None }).max_by(|a, b| a.0.total_cmp(&b.0)).map_or(0.5, |g| g.1);
        let t = resp.interact_pointer_pos().map_or(gap, |at| ((at.x - strip.left()) / strip.width()).clamp(0.0, 1.0));
        let i = stops.iter().position(|s| s.offset > t).unwrap_or(stops.len());
        stops.insert(i, GradientStop { offset: t, color: ramp_at(stops, t) });
        sel = i;
        changed = Some(tr_noop!("Add colour stop"));
    }
    if let Some(i) = remove.take().filter(|i| *i < stops.len() && stops.len() > 2) {
        stops.remove(i);
        sel = sel.min(stops.len().saturating_sub(1));
        changed = Some(tr_noop!("Remove colour stop"));
    }
    // The picked stop: its colour (with opacity), hex and position.
    let n = stops.len();
    if let Some(st) = stops.get_mut(sel) {
        let mut c = st.color;
        row(ui, tr!("Stop colour"), |ui| {
            if colour_field(ui, &mut c, &tr!("Colour stop {stop}", stop = sel + 1)) {
                st.color = c;
                changed = Some(tr_noop!("Colour stop"));
            }
        });
        let mut pct = st.offset * 100.0;
        row(ui, tr!("Position"), |ui| {
            if ui.add(egui::DragValue::new(&mut pct).suffix(" %").speed(0.5).range(0.0..=100.0).max_decimals(0)).named(tr!("Colour stop position")).changed() {
                st.offset = (pct / 100.0).clamp(0.0, 1.0);
                changed = Some(tr_noop!("Move colour stop"));
            }
            if ui.add_enabled(n > 2, egui::Button::new(tr!("Remove")).small()).named(tr!("Remove colour stop")).on_hover_text(tr!("Remove this colour stop (or drag it off the bar)")).clicked() {
                remove = Some(sel);
            }
        });
    }
    if let Some(i) = remove.filter(|i| *i < stops.len() && stops.len() > 2) {
        stops.remove(i);
        sel = sel.min(stops.len().saturating_sub(1));
        changed = Some(tr_noop!("Remove colour stop"));
    }
    // Typed positions may pass a neighbour: keep the stops in order (the pick follows its stop).
    if let Some(picked) = stops.get(sel).copied() {
        stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
        sel = stops.iter().position(|s| *s == picked).unwrap_or(sel);
    }
    ui.data_mut(|d| d.insert_temp(sel_id, sel));
    changed
}
