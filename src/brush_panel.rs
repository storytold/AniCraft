//! Window › Brush Library and the brush options in Properties (ADR 0068).
//!
//! The library lists the built-in brushes and yours, each with a stroke preview drawn by the
//! CPU renderer (`cmds_brush::preview`, cached as textures). Clicking one sets the brush in hand
//! (the Brush, or the Paint Brush when it is in hand). Your own brushes are saved, renamed,
//! deleted, imported and exported through the `brush.preset_*` commands, so everything here also
//! works from the CLI and MCP; the list lives in the session and is kept in the preferences.
//!
//! Properties shows the brush's preset, a preview of its stroke, what pressure drives, the
//! pressure curve (an editor with Linear, Soft and Firm), Velocity, tapers, dab spacing and the
//! ragged edge.

use crate::a11y::{self, Named};
use crate::app::{App, Tool};
use crate::theme::{T, Type, text};
use anicraft_engine::brush_library::{BUILTIN, BrushPreset, builtin, builtin_hint};
use anicraft_engine::geom::pressure::{CURVE_PRESETS, PressureCurve};
use egui::{Rect, Sense, Stroke, WidgetType, pos2, vec2};
use serde_json::json;
use std::collections::HashMap;

/// Most previews kept as textures (the cache starts over past it).
const MAX_PREVIEWS: usize = 96;
/// A preview in the library, in points.
const ROW_PREVIEW: egui::Vec2 = egui::Vec2::new(150.0, 40.0);
/// The pressure curve editor's side, in points.
const CURVE_SIDE: f32 = 96.0;

/// The library window's state and the preview cache (view state).
#[derive(Default)]
pub struct BrushLibraryView {
    pub open: bool,
    /// The brush last chosen for the Brush and for the Paint Brush, by name.
    pub chosen: [Option<String>; 2],
    /// The name typed to save the brush in hand under.
    pub save_name: String,
    /// A brush of yours being renamed: its name, and the new one as typed.
    pub renaming: Option<(String, String)>,
    previews: HashMap<u64, egui::TextureHandle>,
}

fn slot(tool: Tool) -> usize {
    usize::from(tool == Tool::PaintBrush)
}

/// The brush tool the library sets: the one in hand, or the Brush.
fn target(app: &App) -> Tool {
    if app.tool == Tool::PaintBrush { Tool::PaintBrush } else { Tool::Brush }
}

/// Copies the session's brushes into the preferences when a command changed them, and saves
/// them (not in screenshot or test runs, which never write the user's preferences).
pub fn sync(app: &mut App) {
    if app.s.brushes == app.prefs.brushes {
        return;
    }
    app.prefs.brushes = app.s.brushes.clone();
    if app.shot.is_none()
        && let Err(e) = app.prefs.save()
    {
        app.status_err(tr!("Could not save preferences: {e}", e = e));
    }
}

/// Every brush the library shows: `(name to show, preset, built-in key)`.
fn all(app: &App) -> Vec<(String, BrushPreset, Option<&'static str>)> {
    let mut v: Vec<(String, BrushPreset, Option<&'static str>)> = BUILTIN.iter().filter_map(|(k, n)| builtin(k).map(|p| (crate::i18n::t(n).to_string(), p, Some(*k)))).collect();
    v.extend(app.s.brushes.user().iter().map(|p| (p.name.clone(), p.clone(), None)));
    v
}

/// Sets the brush in hand (the Brush when neither brush is) to `p`.
pub fn choose(app: &mut App, p: &BrushPreset) {
    let tool = target(app);
    if !matches!(app.tool, Tool::Brush | Tool::PaintBrush) {
        app.tool = tool;
    }
    match tool {
        Tool::PaintBrush => app.paint_brush.apply(p),
        _ => app.brush.apply(p),
    }
    if let Some(c) = app.brush_lib.chosen.get_mut(slot(tool)) {
        *c = Some(p.name.clone());
    }
}

/// The name of the brush the tool's options are, if they still are one.
fn current_name(app: &App, tool: Tool) -> Option<String> {
    let o = if tool == Tool::PaintBrush { &app.paint_brush } else { &app.brush };
    let chosen = app.brush_lib.chosen.get(slot(tool)).cloned().flatten()?;
    let p = app.s.brushes.find(&chosen)?;
    o.matches(&p).then_some(chosen)
}

/// A stroke preview of `p`, `size` points, in the theme's text colour (cached).
fn preview(app: &mut App, ctx: &egui::Context, p: &BrushPreset, as_stroke: bool, size: egui::Vec2) -> Option<egui::TextureHandle> {
    use std::hash::{Hash, Hasher};
    let ppp = ctx.pixels_per_point().clamp(0.5, 4.0);
    let (w, h) = ((size.x * ppp).round().max(1.0) as u32, (size.y * ppp).round().max(1.0) as u32);
    let c = T.text;
    let mut hasher = std::hash::DefaultHasher::new();
    (p.params().to_string(), as_stroke, c.to_array(), w, h).hash(&mut hasher);
    let key = hasher.finish();
    if let Some(t) = app.brush_lib.previews.get(&key) {
        return Some(t.clone());
    }
    let color = anicraft_engine::doc::Rgba::new(f32::from(c.r()) / 255.0, f32::from(c.g()) / 255.0, f32::from(c.b()) / 255.0, 1.0);
    let px = anicraft_engine::cmds_brush::preview(p, as_stroke, w, h, ppp, color).ok()?;
    let image = egui::ColorImage::from_rgba_premultiplied([w as usize, h as usize], &px.to_premul_rgba8());
    let tex = ctx.load_texture(format!("brush-preview-{key:x}"), image, egui::TextureOptions::LINEAR);
    if app.brush_lib.previews.len() >= MAX_PREVIEWS {
        app.brush_lib.previews.clear();
    }
    app.brush_lib.previews.insert(key, tex.clone());
    Some(tex)
}

/// Opens or closes Window › Brush Library.
pub fn toggle(app: &mut App) {
    app.brush_lib.open = !app.brush_lib.open;
}

/// The Brush Library window.
pub fn show(app: &mut App, ctx: &egui::Context) {
    if !app.brush_lib.open {
        return;
    }
    let mut open = true;
    let at = ctx.content_rect().right_top() + vec2(-620.0, 90.0);
    let shown = egui::Window::new(tr!("Brush Library")).id(egui::Id::new("brush_library")).open(&mut open).default_pos(at).default_width(300.0).resizable(true).collapsible(false).show(ctx, |ui| panel(app, ui));
    a11y::name_title_bar(&shown, tr!("Brush Library"));
    app.brush_lib.open &= open;
}

fn panel(app: &mut App, ui: &mut egui::Ui) {
    let tool = target(app);
    let as_stroke = tool == Tool::PaintBrush;
    let for_tool = if as_stroke { tr!("Click a brush to draw with it in the Paint Brush.") } else { tr!("Click a brush to paint with it in the Brush.") };
    ui.label(text(Type::Caption, for_tool));
    ui.add_space(4.0);
    let current = current_name(app, tool);
    let items = all(app);
    let mut picked: Option<BrushPreset> = None;
    let mut action: Option<(&'static str, serde_json::Value)> = None;
    egui::ScrollArea::vertical().id_salt("brush_library_list").max_height(560.0).auto_shrink([false, true]).show(ui, |ui| {
        let header = |ui: &mut egui::Ui, s: &str| {
            ui.add_space(2.0);
            ui.label(text(Type::Heading, s));
        };
        header(ui, tr!("Built-in"));
        let mut yours_shown = false;
        for (shown, p, key) in &items {
            if key.is_none() && !yours_shown {
                yours_shown = true;
                header(ui, tr!("Your brushes"));
            }
            if key.is_none()
                && let Some((old, new)) = app.brush_lib.renaming.as_mut()
                && old == &p.name
            {
                // Renaming inline: Enter keeps it, Esc or a click elsewhere leaves it.
                let r = ui.add(egui::TextEdit::singleline(new).desired_width(ui.available_width())).named(tr!("Brush name"));
                r.request_focus();
                if r.lost_focus() {
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        action = Some(("brush.preset_rename", json!({"name": old, "to": new})));
                    }
                    app.brush_lib.renaming = None;
                }
                continue;
            }
            let tex = preview(app, ui.ctx(), p, as_stroke, ROW_PREVIEW);
            let selected = current.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(&p.name));
            let w = ui.available_width().max(ROW_PREVIEW.x + 60.0);
            let (rect, resp) = ui.allocate_exact_size(vec2(w, ROW_PREVIEW.y + 6.0), Sense::click());
            let bg = if selected {
                T.active
            } else if resp.hovered() {
                T.header
            } else {
                T.field
            };
            ui.painter().rect_filled(rect, T.rc(), bg);
            let img = Rect::from_min_size(rect.min + vec2(3.0, 3.0), ROW_PREVIEW);
            if let Some(t) = &tex {
                ui.painter().image(t.id(), img, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
            }
            let name_at = pos2(img.right() + 8.0, rect.center().y);
            let color = if selected { T.on_active } else { T.text };
            ui.painter().text(name_at, egui::Align2::LEFT_CENTER, shown, crate::theme::font(Type::Body), color);
            a11y::control(ui, &resp, WidgetType::SelectableLabel, &tr!("{name} brush", name = shown), Some(selected));
            let hint = key.map(|k| crate::i18n::t(builtin_hint(k)).to_string()).unwrap_or_else(|| tr!("Your brush. Right-click to rename, export or delete it.").to_string());
            let resp = resp.on_hover_text(hint);
            if resp.clicked() {
                picked = Some(p.clone());
            }
            let name = p.name.clone();
            let mine = key.is_none();
            resp.context_menu(|ui| {
                if ui.button(tr!("Use")).clicked() {
                    picked = Some(p.clone());
                    ui.close();
                }
                if mine {
                    if ui.button(tr!("Rename…")).clicked() {
                        app.brush_lib.renaming = Some((name.clone(), name.clone()));
                        ui.close();
                    }
                    if ui.button(tr!("Delete…")).clicked() {
                        action = Some(("brush.preset_delete", json!({"name": name})));
                        ui.close();
                    }
                }
            });
            ui.add_space(2.0);
        }
        if !yours_shown {
            header(ui, tr!("Your brushes"));
            ui.label(text(Type::Caption, tr!("None yet. Set up a brush in Properties, then save it below.")));
        }
    });
    if let Some(p) = picked {
        choose(app, &p);
    }
    ui.separator();
    // Save the brush in hand under a name.
    ui.horizontal(|ui| {
        let r = ui.add(egui::TextEdit::singleline(&mut app.brush_lib.save_name).desired_width((ui.available_width() - 110.0).max(80.0)).hint_text(tr!("Name"))).named(tr!("Name for the brush"));
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let name = app.brush_lib.save_name.trim().to_string();
        if (ui.add_enabled(!name.is_empty(), egui::Button::new(tr!("Save Brush"))).on_hover_text(tr!("Saves the brush in hand as one of yours (one with that name is replaced)")).clicked() || enter) && !name.is_empty() {
            let o = if tool == Tool::PaintBrush { app.paint_brush } else { app.brush };
            let mut params = o.preset(&name).params();
            params["name"] = json!(name);
            action = Some(("brush.preset_save", params));
        }
    });
    ui.horizontal(|ui| {
        if ui.button(tr!("Import…")).on_hover_text(tr!("Adds the brushes in a brush set file to yours")).clicked() {
            app.invoke("brush.preset_import");
        }
        let any = !app.s.brushes.is_empty();
        if ui.add_enabled(any, egui::Button::new(tr!("Export…"))).on_hover_text(tr!("Writes your brushes to a brush set file, to use on another computer")).clicked() {
            app.invoke("brush.preset_export");
        }
    });
    if let Some((id, params)) = action {
        let saving = id == "brush.preset_save";
        let name = params["name"].as_str().map(str::to_string);
        if id == "brush.preset_delete" {
            app.request(id, params);
        } else if let Some(v) = app.run(id, params)
            && saving
        {
            app.brush_lib.save_name.clear();
            if let (Some(c), Some(n)) = (app.brush_lib.chosen.get_mut(slot(tool)), v["name"].as_str()) {
                *c = Some(n.to_string());
            }
            app.status_ok(tr!("Saved the brush “{name}” to your brushes.", name = name.unwrap_or_default()));
        }
    }
}

/// The brush rows in Properties for the Brush and the Paint Brush (ADR 0068): which brush,
/// its preview, and what pressure, speed and shaping do. `tool` says whose options.
pub fn props(app: &mut App, ui: &mut egui::Ui, row: impl Fn(&mut egui::Ui, &str, &mut dyn FnMut(&mut egui::Ui)), row_prose: impl Fn(&mut egui::Ui, &str, &mut dyn FnMut(&mut egui::Ui))) {
    let tool = target(app);
    let as_stroke = tool == Tool::PaintBrush;
    let current = current_name(app, tool);
    let items = all(app);
    let mut picked: Option<BrushPreset> = None;
    let mut library = false;
    row_prose(ui, tr!("Brush"), &mut |ui| {
        let shown = current.as_deref().map_or_else(|| tr!("Custom").to_string(), |c| items.iter().find(|(_, p, _)| p.name.eq_ignore_ascii_case(c)).map_or(c.to_string(), |(s, _, _)| s.clone()));
        egui::ComboBox::from_id_salt(("brush_preset", slot(tool)))
            .width(130.0)
            .selected_text(shown)
            .show_ui(ui, |ui| {
                for (s, p, key) in &items {
                    let on = current.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(&p.name));
                    let r = ui.selectable_label(on, s.as_str());
                    let r = match key {
                        Some(k) => r.on_hover_text(crate::i18n::t(builtin_hint(k))),
                        None => r,
                    };
                    if r.clicked() {
                        picked = Some(p.clone());
                    }
                }
            })
            .response
            .set_name(tr!("Brush preset"));
        if ui.button(tr!("Library…")).on_hover_text(tr!("Window › Brush Library: every brush with a preview, and your own")).clicked() {
            library = true;
        }
    });
    if let Some(p) = picked {
        choose(app, &p);
    }
    if library {
        toggle(app);
    }
    // What a stroke with these settings looks like (the renderer's own picture).
    let o = *crate::brushtool::options(app);
    let size = vec2((ui.available_width() - crate::theme::FORM_LABEL_DOCK).clamp(80.0, 200.0), 36.0);
    if let Some(t) = preview(app, ui.ctx(), &o.preset(""), as_stroke, size) {
        // Under the fields, past the labels' column (a picture, not a control).
        ui.horizontal(|ui| {
            ui.add_space(crate::theme::FORM_LABEL_DOCK + ui.spacing().item_spacing.x);
            let (r, _) = ui.allocate_exact_size(size, Sense::hover());
            ui.painter().rect_filled(r, T.rc(), T.field);
            ui.painter().image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
        });
    }
    let tilt_seen = app.pen_tilt_seen;
    let o = crate::brushtool::options_mut(app);
    row_prose(ui, tr!("Pressure"), &mut |ui| {
        crate::theme::checkbox(ui, &mut o.pressure_size, tr!("Size")).on_hover_text(tr!("Pen pressure varies the size, from 10 % to the full size"));
        crate::theme::checkbox(ui, &mut o.pressure_opacity, tr!("Opacity")).on_hover_text(tr!("Pen pressure sets the stroke's opacity (its average over the stroke)"));
    });
    row_prose(ui, tr!("Curve"), &mut |ui| {
        ui.add_enabled_ui(o.pressure_size || o.pressure_opacity, |ui| {
            curve_editor(ui, &mut o.curve);
            ui.vertical(|ui| {
                for (key, label) in CURVE_PRESETS {
                    let on = o.curve.preset_key() == Some(key);
                    let hint = match key {
                        "soft" => tr!("A light touch already paints wide"),
                        "firm" => tr!("It takes a firm press to paint wide"),
                        _ => tr!("Pressure as the pen measures it"),
                    };
                    if ui.selectable_label(on, crate::i18n::t(label)).on_hover_text(hint).clicked()
                        && let Some(c) = PressureCurve::preset(key)
                    {
                        o.curve = c;
                    }
                }
            });
        });
    });
    row_prose(ui, tr!("Mouse"), &mut |ui| {
        crate::theme::checkbox(ui, &mut o.velocity, tr!("Speed as pressure")).on_hover_text(tr!("Without a pen: drawing faster makes the stroke thinner, as a pen pressed lightly would"));
    });
    // Tilt (ADR 0093): shown once a pen has reported its tilt this session. Until then the
    // options would do nothing, so they stay out of the way (brushes keep their tilt settings).
    if tilt_seen {
        row_prose(ui, tr!("Tilt"), &mut |ui| {
            if !as_stroke {
                crate::theme::checkbox(ui, &mut o.tilt_angle, tr!("Angle")).on_hover_text(tr!("The pen's tilt turns the tip: the way it leans adds to the brush's angle"));
            }
            crate::theme::checkbox(ui, &mut o.tilt_width, tr!("Width")).on_hover_text(tr!("A leaning pen paints wider, up to twice as wide lying flat"));
        });
    }
    row(ui, tr!("Taper"), &mut |ui| {
        ui.add(egui::DragValue::new(&mut o.taper_start).suffix(" %").speed(0.5).range(0.0..=50.0).max_decimals(0)).named(tr!("Taper at the start")).on_hover_text(tr!("How much of the stroke swells from a point at its start"));
        ui.add(egui::DragValue::new(&mut o.taper_end).suffix(" %").speed(0.5).range(0.0..=50.0).max_decimals(0)).named(tr!("Taper at the end")).on_hover_text(tr!("How much of the stroke thins to a point at its end"));
    });
    if !as_stroke {
        row(ui, tr!("Spacing"), &mut |ui| {
            let mut dabs = o.spacing > 0.0;
            if crate::theme::checkbox(ui, &mut dabs, "").named(tr!("Dabs")).on_hover_text(tr!("Stamp the tip as separate dabs along the stroke")).changed() {
                o.spacing = if dabs { 150.0 } else { 0.0 };
            }
            // A field of its own while off, so its range never turns a continuous stroke into dabs.
            let mut gap = if dabs { o.spacing } else { 150.0 };
            if ui.add_enabled(dabs, egui::DragValue::new(&mut gap).suffix(" %").speed(1.0).range(5.0..=1000.0).max_decimals(0)).named(tr!("Dab spacing")).on_hover_text(tr!("The distance between dabs, as a part of the size")).changed() && dabs {
                o.spacing = gap;
            }
        });
        row(ui, tr!("Rough edge"), &mut |ui| {
            ui.add(egui::DragValue::new(&mut o.jitter).suffix(" %").speed(0.5).range(0.0..=50.0).max_decimals(0)).named(tr!("Rough edge")).on_hover_text(tr!("Wears the edge in by up to this part of the size, like chalk. The same every time"));
        });
        // Textured brushes (ADR 0093): still vector paint.
        row(ui, tr!("Texture"), &mut |ui| {
            ui.add(egui::DragValue::new(&mut o.texture).suffix(" %").speed(0.5).range(0.0..=100.0).max_decimals(0)).named(tr!("Texture")).on_hover_text(tr!("A paper grain shows through the paint, like charcoal on rough paper. The grain stays put on the stage, so strokes over each other share it"));
        });
        row(ui, tr!("Scatter"), &mut |ui| {
            ui.add(egui::DragValue::new(&mut o.scatter).suffix(" %").speed(1.0).range(0.0..=200.0).max_decimals(0)).named(tr!("Scatter")).on_hover_text(tr!("Throws dabs either side of the stroke, up to this part of the size away, and varies their size: spatter and stipple"));
        });
    }
}

/// The pressure curve editor: drag a point to move it (the ends move only up and down), drag
/// elsewhere to add one, double-click or right-click a point to remove it.
pub fn curve_editor(ui: &mut egui::Ui, curve: &mut PressureCurve) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(CURVE_SIDE, CURVE_SIDE), Sense::click_and_drag());
    a11y::control(ui, &resp, WidgetType::Button, tr!("Pressure curve"), None);
    let resp = resp.on_hover_text(tr!("Pen pressure (across) to the brush's size or opacity (up). Drag a point to move it, drag elsewhere to add one, double-click a point to remove it"));
    let inner = rect.shrink(5.0);
    let to_screen = |p: [f32; 2]| pos2(inner.left() + p[0] * inner.width(), inner.bottom() - p[1] * inner.height());
    let to_curve = |s: egui::Pos2| [((s.x - inner.left()) / inner.width()).clamp(0.0, 1.0), ((inner.bottom() - s.y) / inner.height()).clamp(0.0, 1.0)];
    let id = resp.id.with("dragged");
    let near = |c: &PressureCurve, at: egui::Pos2| c.points().iter().enumerate().map(|(i, p)| (i, to_screen(*p).distance(at))).filter(|(_, d)| *d <= 8.0).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(i, _)| i);
    if ui.is_enabled() {
        let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
        if resp.drag_started()
            && let Some(at) = pointer
        {
            let i = near(curve, at).unwrap_or_else(|| {
                let c = curve.with_point(usize::MAX, to_curve(at));
                let x = to_curve(at)[0];
                *curve = c;
                curve.points().iter().position(|p| (p[0] - x).abs() < 0.03).unwrap_or(0)
            });
            ui.data_mut(|d| d.insert_temp(id, i));
        }
        if resp.dragged()
            && let (Some(i), Some(at)) = (ui.data(|d| d.get_temp::<usize>(id)), pointer)
        {
            let pts = curve.points();
            let mut to = to_curve(at);
            // An interior point stays between its neighbours.
            if let (Some(a), Some(b)) = (i.checked_sub(1).and_then(|j| pts.get(j)), pts.get(i + 1)) {
                to[0] = to[0].clamp(a[0] + 0.03, (b[0] - 0.03).max(a[0] + 0.03));
            }
            *curve = curve.with_point(i, to);
        }
        if resp.drag_stopped() {
            ui.data_mut(|d| d.remove::<usize>(id));
        }
        if (resp.double_clicked() || resp.secondary_clicked())
            && let Some(at) = pointer
            && let Some(i) = near(curve, at)
        {
            *curve = curve.without_point(i);
        }
    }
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, T.rc(), T.field);
    for k in 1..4 {
        let f = k as f32 / 4.0;
        let x = inner.left() + f * inner.width();
        let y = inner.bottom() - f * inner.height();
        painter.line_segment([pos2(x, inner.top()), pos2(x, inner.bottom())], Stroke::new(1.0, T.separator));
        painter.line_segment([pos2(inner.left(), y), pos2(inner.right(), y)], Stroke::new(1.0, T.separator));
    }
    painter.line_segment([to_screen([0.0, 0.0]), to_screen([1.0, 1.0])], Stroke::new(1.0, T.border));
    let line: Vec<egui::Pos2> = (0..=48).map(|i| i as f32 / 48.0).map(|x| to_screen([x, curve.map(x)])).collect();
    let ink = if ui.is_enabled() { T.accent } else { T.text_faint };
    painter.add(egui::Shape::line(line, Stroke::new(2.0, ink)));
    let hot = resp.hover_pos().and_then(|at| near(curve, at));
    for (i, p) in curve.points().iter().enumerate() {
        let r = if hot == Some(i) { 4.5 } else { 3.5 };
        painter.circle_filled(to_screen(*p), r, if hot == Some(i) { T.accent_text } else { T.text });
    }
    resp
}

