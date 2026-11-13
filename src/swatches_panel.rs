//! The Swatches panel (ADR 0078): the document's saved colours and gradients, as Animate's
//! Swatches panel.
//!
//! A click on a swatch applies it to the selection and the tool colour: to the stroke or the
//! fill, whichever is active (the Colour panel and this panel share that choice, as Animate's
//! Color panel and Tools do). A gradient always goes to the fill (strokes are one colour).
//! Drag a swatch to move it; right-click for Duplicate, Delete and its name; the buttons under
//! the swatches add the current colour, duplicate and delete; the menu imports and exports
//! swatch files and GIMP palettes, loads the default set or clears it. Every change is a
//! `swatch.*` command, so it is one undo step and saved with the document.

use crate::a11y::{self, Named};
use crate::app::App;
use crate::icons::{self, Icon};
use crate::theme::{T, TARGET, Type, text, to_color32};
use anicraft_engine::doc::{ElementKind, Paint, Rgba, SwatchPaint, ramp_at};
use egui::{Id, Key, Painter, Rect, Response, Sense, Stroke, StrokeKind, WidgetType, pos2, vec2};
use serde_json::json;

/// One swatch, in points, and the gap between swatches.
const CELL: f32 = 18.0;
const GAP: f32 = 2.0;

/// Which colour a swatch click sets: the stroke or the fill (view state, shared with the
/// Colour panel).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ColourTarget {
    Stroke,
    #[default]
    Fill,
}

impl ColourTarget {
    /// The `target` of `swatch.apply` and the `from` of `swatch.add`.
    pub fn key(self) -> &'static str {
        match self {
            ColourTarget::Stroke => "stroke",
            ColourTarget::Fill => "fill",
        }
    }
}

/// The Stroke / Fill choice, as two toggles.
pub fn target_toggle(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal_wrapped(|ui| {
        ui.label(tr!("Swatches set the"));
        for (t, label, tip) in [(ColourTarget::Stroke, tr!("Stroke"), tr!("Swatch clicks set the stroke colour (gradients still go to the fill)")), (ColourTarget::Fill, tr!("Fill"), tr!("Swatch clicks set the fill: a colour or a gradient"))] {
            if ui.selectable_label(app.colour_target == t, label).on_hover_text(tip).clicked() {
                app.colour_target = t;
            }
        }
    });
}

/// The swatch's name for people: its own, else its hex or kind.
pub fn describe(paint: &SwatchPaint, name: &str) -> String {
    let what = match paint {
        SwatchPaint::Solid { color } => anicraft_engine::cmds_swatch::hex(*color),
        SwatchPaint::Linear { stops, .. } => tr!("Linear gradient, {n} colours", n = stops.len()),
        SwatchPaint::Radial { stops, .. } => tr!("Radial gradient, {n} colours", n = stops.len()),
    };
    if name.is_empty() { what } else { format!("{name} ({what})") }
}

/// Paints a swatch into `r`: its colour (a checkerboard under any transparency), a linear
/// gradient's ramp left to right, or a radial one from the centre out.
pub fn preview(p: &Painter, r: Rect, paint: &SwatchPaint) {
    match paint {
        SwatchPaint::Solid { color } => egui::color_picker::show_color_at(p, to_color32(*color), r),
        SwatchPaint::Linear { stops, .. } => {
            let mut mesh = egui::Mesh::default();
            let steps = 16u32;
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
        SwatchPaint::Radial { stops, .. } => {
            let clip = p.with_clip_rect(r.intersect(p.clip_rect()));
            let outer = r.width().max(r.height()) * 0.5;
            let rings = 10;
            for k in (1..=rings).rev() {
                let t = k as f32 / rings as f32;
                clip.circle_filled(r.center(), outer * t, to_color32(ramp_at(stops, t)));
            }
        }
    }
}

/// What a click on `paint` applies to: a gradient is always a fill.
fn target_for(app: &App, paint: &SwatchPaint) -> ColourTarget {
    if paint.is_gradient() { ColourTarget::Fill } else { app.colour_target }
}

/// Applies swatch `i` to the selection and the tool colour.
pub fn apply(app: &mut App, i: usize) {
    let Some(sw) = app.s.doc.swatches.get(i).cloned() else { return };
    let target = target_for(app, &sw.paint);
    app.run("swatch.apply", json!({"index": i, "target": target.key()}));
}

/// `swatch.add` params for the active colour: the selection's (as the Colour panel and the
/// pickers show it), else the tool's.
fn add_params(app: &App) -> Option<serde_json::Value> {
    match app.colour_target {
        ColourTarget::Fill => {
            let (paint, _) = crate::colour_panel::current(app);
            let sw = SwatchPaint::from_paint(&paint)?;
            let mut v = anicraft_engine::cmds_swatch::swatch_json(&anicraft_engine::doc::Swatch { name: String::new(), paint: sw });
            if let Some(o) = v.as_object_mut() {
                o.remove("name");
            }
            Some(v)
        }
        ColourTarget::Stroke => {
            let selected = app.s.selected_elements().into_iter().filter(|(l, _)| *l == app.s.layer).find_map(|(_, e)| match &e.kind {
                ElementKind::Shape(sh) => sh.stroke.as_ref().map(|s| s.color),
                _ => None,
            });
            let c: Rgba = selected.unwrap_or(app.s.style.stroke);
            Some(json!({"type": "solid", "color": anicraft_engine::cmds_swatch::hex(c)}))
        }
    }
}

/// Adds the active colour (or gradient) as a swatch, after the selected one or at the end.
pub fn add_current(app: &mut App) {
    let Some(mut p) = add_params(app) else {
        app.status_err(tr!("A bitmap fill can't be a swatch: swatches hold colours and gradients."));
        return;
    };
    if let (Some(i), Some(o)) = (app.swatch_sel, p.as_object_mut()) {
        o.insert("at".into(), json!((i + 1).min(app.s.doc.swatches.len())));
    }
    if let Some(v) = app.run("swatch.add", p) {
        app.swatch_sel = v["index"].as_u64().map(|i| i as usize);
    }
}

/// A small icon button for the panel's bar, dimmed and inert when `enabled` is false.
fn bar_button(ui: &mut egui::Ui, tip: &str, icon: Icon, enabled: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(TARGET, TARGET), if enabled { Sense::click() } else { Sense::hover() });
    if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, T.rc(), T.hover);
    }
    let colour = if !enabled {
        T.text_faint
    } else if resp.hovered() {
        T.icon_hot
    } else {
        T.icon
    };
    icons::paint(ui.painter(), Rect::from_center_size(rect.center(), vec2(16.0, 16.0)), icon, colour);
    a11y::control(ui, &resp, WidgetType::Button, tip, None);
    resp.on_hover_text(tip)
}

/// The panel.
pub fn panel(app: &mut App, ui: &mut egui::Ui) {
    target_toggle(app, ui);
    ui.add_space(4.0);
    let n = app.s.doc.swatches.len();
    app.swatch_sel = app.swatch_sel.filter(|i| *i < n);
    grid(app, ui);
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let sel = app.swatch_sel;
        let add_tip = match app.colour_target {
            ColourTarget::Fill => tr!("Add the fill as a swatch"),
            ColourTarget::Stroke => tr!("Add the stroke colour as a swatch"),
        };
        if bar_button(ui, add_tip, Icon::Plus, true).clicked() {
            add_current(app);
        }
        if bar_button(ui, tr!("Duplicate the selected swatch"), Icon::Copy, sel.is_some()).clicked()
            && let Some(i) = sel
            && let Some(v) = app.run("swatch.duplicate", json!({"index": i}))
        {
            app.swatch_sel = v["index"].as_u64().map(|i| i as usize);
        }
        if bar_button(ui, tr!("Delete the selected swatch"), Icon::Trash, sel.is_some()).clicked()
            && let Some(i) = sel
        {
            delete(app, i);
        }
        ui.add_space(6.0);
        let more = bar_button(ui, tr!("Swatch options: import, export, defaults"), Icon::Ellipsis, true);
        egui::Popup::menu(&more).id(Id::new("anicraft.swatches.menu")).show(|ui| menu(app, ui));
    });
    let caption = if n == 0 { tr!("No swatches. Add the current colour with +, or load the defaults from the menu.") } else { tr!("Click to apply; drag to move; right-click for more.") };
    ui.label(text(Type::Caption, caption));
}

fn delete(app: &mut App, i: usize) {
    if app.run("swatch.remove", json!({"index": i})).is_some() {
        let n = app.s.doc.swatches.len();
        app.swatch_sel = (n > 0).then(|| i.min(n - 1));
    }
}

/// The panel menu: files and whole-set actions.
fn menu(app: &mut App, ui: &mut egui::Ui) {
    if ui.button(tr!("Add Swatch")).clicked() {
        add_current(app);
    }
    ui.separator();
    if ui.button(tr!("Import Swatches…")).on_hover_text(tr!("Add the swatches in a swatch file (.json) or a GIMP palette (.gpl)")).clicked() {
        app.invoke("swatch.import");
    }
    if ui.add_enabled(!app.s.doc.swatches.is_empty(), egui::Button::new(tr!("Export Swatches…"))).on_hover_text(tr!("Save these swatches as a swatch file (.json) for other documents")).clicked() {
        app.invoke("swatch.export");
    }
    ui.separator();
    if ui.button(tr!("Load Default Swatches")).on_hover_text(tr!("Replace these swatches with a new document's set")).clicked() {
        app.request("swatch.reset", json!({}));
        app.swatch_sel = None;
    }
    if ui.button(tr!("Clear Swatches")).on_hover_text(tr!("Remove every swatch but black and white")).clicked() {
        app.request("swatch.clear", json!({}));
        app.swatch_sel = None;
    }
}

/// What the grid was asked to do this frame (done after drawing it).
enum Act {
    Apply(usize),
    Remove(usize),
    Duplicate(usize),
    Move(usize, usize),
}

/// The swatches, wrapped to the panel's width: click, drag, keys and the right-click menu.
fn grid(app: &mut App, ui: &mut egui::Ui) {
    let swatches = app.s.doc.swatches.clone();
    let n = swatches.len();
    let cols = (((ui.available_width() + GAP) / (CELL + GAP)).floor() as usize).max(1);
    let rows = n.div_ceil(cols).max(1);
    let size = vec2(cols as f32 * (CELL + GAP) - GAP, rows as f32 * (CELL + GAP) - GAP);
    let (area, _) = ui.allocate_exact_size(size, Sense::hover());
    let cell_rect = |i: usize| Rect::from_min_size(pos2(area.left() + (i % cols) as f32 * (CELL + GAP), area.top() + (i / cols) as f32 * (CELL + GAP)), vec2(CELL, CELL));
    let drag_id = Id::new("anicraft.swatches.drag");
    let mut dragging: Option<usize> = ui.data(|d| d.get_temp(drag_id));
    let pointer = ui.ctx().pointer_interact_pos();
    // Where a dragged swatch would land: the cell under the pointer (the last one past the end).
    let drop_at = |p: egui::Pos2| -> usize {
        let col = (((p.x - area.left()) / (CELL + GAP)).floor().max(0.0) as usize).min(cols - 1);
        let row = ((p.y - area.top()) / (CELL + GAP)).floor().max(0.0) as usize;
        (row * cols + col).min(n.saturating_sub(1))
    };
    let mut act: Option<Act> = None;
    for (i, sw) in swatches.iter().enumerate() {
        let r = cell_rect(i);
        let resp = ui.interact(r, Id::new(("anicraft.swatch", i)), Sense::click_and_drag());
        let name = describe(&sw.paint, &sw.name);
        let selected = app.swatch_sel == Some(i);
        a11y::control(ui, &resp, WidgetType::Button, &tr!("Swatch {n}: {name}", n = i + 1, name = name.as_str()), Some(selected));
        let p = ui.painter();
        preview(p, r, &sw.paint);
        let edge = if selected {
            Stroke::new(2.0, T.accent)
        } else if resp.hovered() {
            Stroke::new(1.5, T.selection)
        } else {
            Stroke::new(1.0, T.border)
        };
        p.rect_stroke(r, 0.0, edge, StrokeKind::Outside);
        if resp.has_focus() {
            p.rect_stroke(r.expand(3.0), 2.0, Stroke::new(1.0, T.text), StrokeKind::Outside);
            // The keyboard: Enter applies (a click), Delete removes, Alt+arrows move it.
            let (del, left, right, alt) = ui.input(|inp| (inp.key_pressed(Key::Delete) || inp.key_pressed(Key::Backspace), inp.key_pressed(Key::ArrowLeft), inp.key_pressed(Key::ArrowRight), inp.modifiers.alt));
            if del {
                act = Some(Act::Remove(i));
            } else if alt && left && i > 0 {
                act = Some(Act::Move(i, i - 1));
            } else if alt && right && i + 1 < n {
                act = Some(Act::Move(i, i + 1));
            }
            if alt {
                ui.memory_mut(|m| m.set_focus_lock_filter(resp.id, egui::EventFilter { horizontal_arrows: true, ..Default::default() }));
            }
        }
        if resp.drag_started() {
            dragging = Some(i);
            app.swatch_sel = Some(i);
        }
        if resp.drag_stopped() && dragging == Some(i) {
            if let Some(to) = pointer.map(drop_at)
                && to != i
            {
                act = Some(Act::Move(i, to));
            }
            dragging = None;
        }
        if resp.clicked() {
            app.swatch_sel = Some(i);
            act = Some(Act::Apply(i));
        }
        let tip_target = match target_for(app, &sw.paint) {
            ColourTarget::Fill => tr!("Click to set the fill"),
            ColourTarget::Stroke => tr!("Click to set the stroke"),
        };
        let resp = resp.on_hover_ui(|ui| {
            ui.label(text(if sw.name.is_empty() { Type::Data } else { Type::Body }, name.clone()));
            ui.label(text(Type::Caption, tip_target));
        });
        resp.context_menu(|ui| {
            app.swatch_sel = Some(i);
            if ui.button(tr!("Apply to Fill")).clicked() {
                app.run("swatch.apply", json!({"index": i, "target": "fill"}));
                ui.close();
            }
            let solid = !sw.paint.is_gradient();
            let r = ui.add_enabled(solid, egui::Button::new(tr!("Apply to Stroke")));
            if r.on_disabled_hover_text(tr!("Strokes are one colour")).clicked() {
                app.run("swatch.apply", json!({"index": i, "target": "stroke"}));
                ui.close();
            }
            ui.separator();
            if ui.button(tr!("Duplicate Swatch")).clicked() {
                act = Some(Act::Duplicate(i));
                ui.close();
            }
            if ui.button(tr!("Delete Swatch")).clicked() {
                act = Some(Act::Remove(i));
                ui.close();
            }
            ui.separator();
            name_field(app, ui, i, &sw.name);
        });
    }
    // The drop mark while a swatch is dragged: a bar on the side it would land.
    if let (Some(from), Some(at)) = (dragging, pointer)
        && ui.ctx().dragged_id().is_some()
    {
        let to = drop_at(at);
        if to != from {
            let r = cell_rect(to);
            let x = if to > from { r.right() + GAP * 0.5 } else { r.left() - GAP * 0.5 };
            ui.painter().line_segment([pos2(x, r.top() - 2.0), pos2(x, r.bottom() + 2.0)], Stroke::new(2.0, T.accent));
        }
    } else if ui.ctx().dragged_id().is_none() {
        dragging = None;
    }
    ui.data_mut(|d| match dragging {
        Some(v) => {
            d.insert_temp(drag_id, v);
        }
        None => d.remove::<usize>(drag_id),
    });
    match act {
        Some(Act::Apply(i)) => apply(app, i),
        Some(Act::Remove(i)) => delete(app, i),
        Some(Act::Duplicate(i)) => {
            if let Some(v) = app.run("swatch.duplicate", json!({"index": i})) {
                app.swatch_sel = v["index"].as_u64().map(|i| i as usize);
            }
        }
        Some(Act::Move(from, to)) => {
            let moved = app.run("swatch.move", json!({"from": from, "to": to})).is_some();
            if moved {
                app.swatch_sel = Some(to);
            }
        }
        None => {}
    }
}

/// The swatch's name, typed in its right-click menu (Enter or leaving the field renames it).
fn name_field(app: &mut App, ui: &mut egui::Ui, i: usize, current: &str) {
    let buf_id = Id::new(("anicraft.swatch.name", i));
    let mut s: String = ui.data(|d| d.get_temp(buf_id)).unwrap_or_else(|| current.to_string());
    ui.horizontal(|ui| {
        ui.label(tr!("Name"));
        let r = ui.add(egui::TextEdit::singleline(&mut s).desired_width(crate::theme::fit(ui, 140.0)).char_limit(anicraft_engine::doc::swatch::MAX_SWATCH_NAME).hint_text(tr!("no name"))).named(tr!("Swatch name"));
        if r.lost_focus() {
            if s.trim() != current {
                app.run("swatch.rename", json!({"index": i, "name": s}));
            }
            ui.data_mut(|d| d.remove::<String>(buf_id));
        } else if r.changed() {
            ui.data_mut(|d| d.insert_temp(buf_id, s.clone()));
        }
    });
}

/// The document's swatches as the colour pickers show them (ADR 0078): the solid colours, and
/// the gradients for the fill picker. Put in the context each frame by the app.
pub fn publish_to_pickers(ctx: &egui::Context, app: &App) {
    ctx.data_mut(|d| d.insert_temp(Id::new("anicraft.colour.document"), app.s.doc.swatches.clone()));
}

/// The document's swatches, for a picker.
pub fn for_pickers(ctx: &egui::Context) -> std::sync::Arc<Vec<anicraft_engine::doc::Swatch>> {
    ctx.data(|d| d.get_temp(Id::new("anicraft.colour.document"))).unwrap_or_default()
}

/// The fill a gradient swatch makes, as `paint.set_fill` params.
pub fn fill_params(sw: &SwatchPaint) -> serde_json::Value {
    let mut v = anicraft_engine::cmds_swatch::swatch_json(&anicraft_engine::doc::Swatch { name: String::new(), paint: sw.clone() });
    if let Some(o) = v.as_object_mut() {
        o.remove("name");
    }
    v
}

/// Whether `paint` is what a swatch holds (to mark it in the pickers).
pub fn matches(sw: &SwatchPaint, paint: &Paint) -> bool {
    SwatchPaint::from_paint(paint).as_ref() == Some(sw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn frames(ctx: &egui::Context, app: &mut App) {
        let screen = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1600.0, 1000.0))), ..Default::default() };
        for _ in 0..2 {
            let mut out = ctx.run_ui(screen.clone(), |ui| app.frame(ui));
            out.textures_delta.clear();
        }
    }

    #[test]
    fn a_click_applies_to_the_active_colour_and_gradients_to_the_fill() {
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, anicraft_engine::Session::default(), Some(PathBuf::from("unused.png")));
        app.run("draw.rect", json!({"x": 10, "y": 10, "w": 100, "h": 80, "fill": "#ff0000", "stroke": "#000000"}));
        let green = app.s.doc.swatches.iter().position(|w| w.paint == SwatchPaint::Solid { color: Rgba::hex(0x00ff00) }).unwrap();
        app.colour_target = ColourTarget::Stroke;
        apply(&mut app, green);
        assert_eq!(app.s.style.stroke, Rgba::hex(0x00ff00));
        let shape = |app: &App| match &app.s.selected_elements().first().map(|(_, e)| e.kind.clone()) {
            Some(ElementKind::Shape(sh)) => sh.clone(),
            _ => panic!("a shape"),
        };
        assert_eq!(shape(&app).stroke.map(|s| s.color), Some(Rgba::hex(0x00ff00)));
        // A gradient goes to the fill whatever is active.
        let ball = app.s.doc.swatches.iter().position(|w| w.name == "Blue ball").unwrap();
        apply(&mut app, ball);
        assert!(matches!(shape(&app).fill, Some(Paint::Radial { .. })));
        // Add takes the active colour: the stroke, after the selected swatch.
        app.swatch_sel = Some(0);
        add_current(&mut app);
        assert_eq!(app.swatch_sel, Some(1));
        assert_eq!(app.s.doc.swatches[1].paint, SwatchPaint::Solid { color: Rgba::hex(0x00ff00) });
        // The fill: the selection's gradient.
        app.colour_target = ColourTarget::Fill;
        add_current(&mut app);
        assert!(app.s.doc.swatches[2].paint.is_gradient());
        assert!(describe(&app.s.doc.swatches[2].paint, "").starts_with("Radial gradient"));
        assert_eq!(describe(&SwatchPaint::Solid { color: Rgba::WHITE }, "Snow"), "Snow (#ffffff)");
        // The whole window draws with the panel shown, docked and floating.
        app.show_panel(crate::dock::Panel::Swatches);
        frames(&ctx, &mut app);
        app.dock.move_to(crate::dock::Panel::Swatches, crate::dock::Target::Float(None));
        frames(&ctx, &mut app);
        assert_eq!(for_pickers(&ctx).len(), app.s.doc.swatches.len(), "the pickers see the document's swatches");
    }
}
