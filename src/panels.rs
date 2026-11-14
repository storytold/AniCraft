//! Tools bar, edit bar, status bar, Properties and Library panels.

use crate::app::{App, Tool};
use crate::icons::{self, Icon};
use crate::theme::{Section, T, TARGET, TOOL_BUTTON, TOOL_ICON, Type, color, section, text, to_color32, with_type};
use anicraft_engine::doc::{ColorEffect, ColorStyle, ElementKind, LibraryItem, LineType, MAX_PATH_OFFSET, Paint, Rgba, SymbolKind, TextKind, evaluate_layer_elements};
use crate::a11y::{self, Named};
use egui::{Color32, Rect, Sense, Stroke, StrokeKind, WidgetType, pos2, vec2};
use serde_json::{Value, json};

// ----- tools --------------------------------------------------------------------------------

/// The Lucide icon for a tool.
fn tool_icon(tool: Tool) -> Icon {
    match tool {
        Tool::Select => Icon::Select,
        Tool::Subselect => Icon::Subselect,
        Tool::Transform => Icon::Transform,
        Tool::GradientTransform => Icon::GradientTransform,
        Tool::Width => Icon::Width,
        Tool::Pen => Icon::Pen,
        Tool::Text => Icon::Type,
        Tool::Pencil => Icon::Pencil,
        Tool::Brush => Icon::Brush,
        Tool::PaintBrush => Icon::PaintBrush,
        Tool::Eraser => Icon::Eraser,
        Tool::Rect => Icon::Rect,
        Tool::Oval => Icon::Oval,
        Tool::Star => Icon::PolyStar,
        Tool::Line => Icon::Line,
        Tool::PaintBucket => Icon::Bucket,
        Tool::InkBottle => Icon::Ink,
        Tool::Eyedropper => Icon::Eyedropper,
        Tool::Camera => Icon::Camera,
        Tool::Bone => Icon::Bone,
        Tool::Bind => Icon::Bind,
        Tool::Warp => Icon::Warp,
        Tool::Hand => Icon::Hand,
        Tool::Zoom => Icon::Zoom,
    }
}

/// How the tools are laid out where they are (ADR 0087).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolFlow {
    /// One column: the bar on the left or right of the editor.
    Column,
    /// One row: the bar above or below the stage.
    Row,
    /// As many columns as fit: a panel in a dock or a floating window.
    Grid,
}

pub fn tools(app: &mut App, ui: &mut egui::Ui, flow: ToolFlow) {
    let area = ui.max_rect();
    // Scrolls (wheel, or Tab to a tool) when a large interface scale leaves too little room.
    // The margin is inside the scroll area, so focus rings drawn around a tool are not clipped.
    let hidden = egui::scroll_area::ScrollBarVisibility::AlwaysHidden;
    match flow {
        ToolFlow::Column => {
            egui::ScrollArea::vertical().id_salt("tools").scroll_bar_visibility(hidden).show(ui, |ui| {
                egui::Frame::NONE.inner_margin(egui::Margin::symmetric(6, 0)).show(ui, |ui| ui.vertical_centered(|ui| tool_items(app, ui, area, flow)));
            });
        }
        ToolFlow::Row => {
            egui::ScrollArea::horizontal().id_salt("tools_row").scroll_bar_visibility(hidden).show(ui, |ui| {
                egui::Frame::NONE.inner_margin(egui::Margin::symmetric(0, 6)).show(ui, |ui| ui.horizontal(|ui| tool_items(app, ui, area, flow)));
            });
        }
        ToolFlow::Grid => {
            egui::ScrollArea::vertical().id_salt("tools_grid").auto_shrink([false, false]).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
                    tool_items(app, ui, area, flow);
                });
            });
        }
    }
}

fn tool_items(app: &mut App, ui: &mut egui::Ui, area: Rect, flow: ToolFlow) {
    for tool in Tool::ALL {
        let (rect, resp) = ui.allocate_exact_size(vec2(TOOL_BUTTON, TOOL_BUTTON), Sense::click());
        let active = app.tool == tool;
        // The active tool sits on a lighter gray block with a white icon; the rest are a
        // step dimmer, so the one in hand stands out without colour.
        let bg = if active {
            T.active
        } else if resp.hovered() {
            T.header
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, T.rc(), bg);
        let tint = if active { T.on_active } else if resp.hovered() { T.icon_hot } else { T.icon };
        icons::paint(ui.painter(), Rect::from_center_size(rect.center(), vec2(TOOL_ICON, TOOL_ICON)), tool_icon(tool), tint);
        // "Pen tool (P)": what the tooltip says first, and what a screen reader says.
        let name = a11y::with_key(&tr!("{tool} tool", tool = crate::i18n::t(tool.label())), app.keymap.primary(crate::keymap::tool_id(tool)));
        a11y::control(ui, &resp, WidgetType::Button, &name, Some(active));
        if active {
            a11y::anchor(ui, a11y::Region::Tools, area, &resp);
        }
        let resp = resp.on_hover_text(format!("{name}
{}", crate::i18n::t(tool.hint())));
        if resp.clicked() {
            app.tool = tool;
        }
        // Animate: double-clicking the Eraser erases everything on the stage (it asks first).
        if tool == Tool::Eraser && resp.double_clicked() {
            crate::erasertool::erase_everything(app);
        }
        if matches!(tool, Tool::Width | Tool::Eraser | Tool::Line | Tool::Eyedropper) {
            // Tool groups: select, draw by hand, shapes, view.
            match flow {
                ToolFlow::Column => {
                    ui.add_space(3.0);
                    let y = ui.cursor().top();
                    ui.painter().line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], Stroke::new(1.0, T.border));
                    ui.add_space(4.0);
                }
                ToolFlow::Row => {
                    ui.add_space(3.0);
                    let x = ui.cursor().left();
                    ui.painter().line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.0, T.border));
                    ui.add_space(4.0);
                }
                ToolFlow::Grid => ui.add_space(6.0),
            }
        }
    }
    // Snap to Objects (Animate's magnet, ADR 0042): on, dragged points and selections snap
    // to points, lines and fill edges.
    let (rect, resp) = ui.allocate_exact_size(vec2(TOOL_BUTTON, TOOL_BUTTON), Sense::click());
    let on = app.s.style.snap_objects;
    let bg = if on {
        T.active
    } else if resp.hovered() {
        T.header
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, T.rc(), bg);
    let tint = if on { T.on_active } else if resp.hovered() { T.icon_hot } else { T.icon };
    icons::paint(ui.painter(), Rect::from_center_size(rect.center(), vec2(TOOL_ICON, TOOL_ICON)), Icon::Magnet, tint);
    let name = a11y::with_key(tr!("Snap to Objects"), app.keymap.primary("draw.snap_to_objects"));
    a11y::control(ui, &resp, WidgetType::Checkbox, &name, Some(on));
    let resp = resp.on_hover_text(if on { tr!("{name}: on
Dragged points and selections snap to points, lines and fill edges within 10 pixels.", name = name) } else { tr!("{name}: off
Dragged points and selections snap to points, lines and fill edges within 10 pixels.", name = name) });
    if resp.clicked() {
        app.run("draw.snap_to_objects", json!({}));
    }
    ui.add_space(if flow == ToolFlow::Grid { 6.0 } else { 10.0 });
    tool_swatches(app, ui);
}

/// The stroke and fill swatches under the tools, as in Animate: the stroke's frame at the top
/// left, the fill behind it. A click on either opens the colour picker (#246); the colour is
/// the tools' (what they draw next) and, with shapes selected, theirs too.
fn tool_swatches(app: &mut App, ui: &mut egui::Ui) {
    let (r, _) = ui.allocate_exact_size(vec2(TOOL_BUTTON, TOOL_BUTTON), Sense::hover());
    let back = Rect::from_min_size(r.left_top() + vec2(14.0, 14.0), vec2(24.0, 24.0));
    let front = Rect::from_min_size(r.left_top() + vec2(2.0, 2.0), vec2(24.0, 24.0));
    // The fill first, so the stroke's frame (on top) takes the clicks where they overlap.
    let fill_resp = ui.interact(back, ui.id().with("tool_fill"), Sense::click());
    let stroke_resp = ui.interact(front, ui.id().with("tool_stroke"), Sense::click());
    let st = &app.s.style;
    let hot = |r: &egui::Response| if r.hovered() { T.selection } else { T.border };
    ui.painter().rect_filled(back, T.rc(), if st.use_fill { to_color32(st.fill) } else { T.field });
    ui.painter().rect_stroke(back, T.rc(), Stroke::new(1.0, hot(&fill_resp)), StrokeKind::Inside);
    ui.painter().rect_filled(front, T.rc(), T.panel);
    ui.painter().rect_stroke(front.shrink(1.0), T.rc(), Stroke::new(5.0, if st.use_stroke { to_color32(st.stroke) } else { T.field }), StrokeKind::Inside);
    ui.painter().rect_stroke(front, T.rc(), Stroke::new(1.0, hot(&stroke_resp)), StrokeKind::Inside);
    for (fill, resp) in [(true, fill_resp), (false, stroke_resp)] {
        let name = if fill { tr!("Fill colour") } else { tr!("Stroke colour") };
        a11y::control(ui, &resp, WidgetType::ColorButton, name, None);
        let resp = resp.on_hover_text(if fill { tr!("Fill colour: click to pick the colour shapes are filled with") } else { tr!("Stroke colour: click to pick the colour of lines and outlines") });
        // As in Animate, the square clicked last is the one swatches set (ADR 0078).
        if resp.clicked() {
            app.colour_target = if fill { crate::swatches_panel::ColourTarget::Fill } else { crate::swatches_panel::ColourTarget::Stroke };
        }
        let st = &app.s.style;
        let (colour, mut none) = if fill { (st.fill, !st.use_fill) } else { (st.stroke, !st.use_stroke) };
        let mut c = rgba(colour);
        let was = none;
        if fill && std::mem::take(&mut app.open_fill_swatch) {
            egui::Popup::open_id(ui.ctx(), resp.id.with("colour_popup"));
        }
        // The fill opens the one colour picker with the fill's type on top (ADR 0074): what the
        // selection is filled with, else the swatch. Solid colours and None go on as below.
        let picked = if fill {
            let (paint, selected) = crate::colour_panel::current(app);
            let shown = (selected || st.use_fill).then_some(paint);
            match crate::colour_panel::popup_for(ui, &resp, resp.id.with("colour_popup"), shown.as_ref(), &crate::colour_panel::bitmaps(app)) {
                Some(crate::colour_panel::FillEdit::Solid(p)) => {
                    c = [p[0], p[1], p[2], 255];
                    none = false;
                    true
                }
                Some(crate::colour_panel::FillEdit::None(n)) => {
                    none = n;
                    true
                }
                Some(edit) => {
                    app.s.style.use_fill = true;
                    crate::colour_panel::apply_set(app, &edit, true);
                    false
                }
                None => false,
            }
        } else {
            crate::colour::popup_for(ui, &resp, resp.id.with("colour_popup"), &mut c, false, Some(&mut none))
        };
        if picked {
            // No Colour toggled says which; a colour picked turns the colour back on.
            let none = none != was && none;
            let pick = Rgba::new(f32::from(c[0]) / 255.0, f32::from(c[1]) / 255.0, f32::from(c[2]) / 255.0, 1.0);
            let style = &mut app.s.style;
            if fill {
                style.use_fill = !none;
                if !none {
                    style.set_fill_color(pick);
                }
            } else {
                style.use_stroke = !none;
                if !none {
                    style.stroke = pick;
                }
            }
            // Selected shapes take it too (one undo step for a drag in the mixer).
            let shapes = app.s.selected_elements().iter().any(|(_, e)| matches!(e.kind, ElementKind::Shape(_)));
            if shapes {
                let colour = if none { Value::Null } else { json!(hex([c[0], c[1], c[2]])) };
                let (label, id, p) = if fill { (tr_noop!("Fill colour"), "element.set_fill", json!({"color": colour})) } else { (tr_noop!("Stroke colour"), "element.set_stroke", json!({"color": colour})) };
                live(app, label, id, p);
            }
        }
    }
}

/// What a gradient or bitmap fill is, for the caption beside its swatch (nothing for a colour).
fn fill_kind_label(p: Option<&Paint>) -> Option<&'static str> {
    match p? {
        Paint::Linear { .. } => Some(tr!("Linear gradient")),
        Paint::Radial { .. } => Some(tr!("Radial gradient")),
        Paint::Bitmap { .. } => Some(tr!("Bitmap fill")),
        Paint::Solid(_) => None,
    }
}

fn rgba(c: Rgba) -> [u8; 4] {
    let [r, g, b] = rgb(c);
    [r, g, b, 255]
}

// ----- edit bar & status ------------------------------------------------------------------

pub fn edit_bar(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal_centered(|ui| {
        // The scene being edited; the menu switches scenes and manages them.
        let name = app.s.doc.scenes.get(app.s.scene).map_or_else(|| tr!("Scene").to_string(), |s| s.name.clone());
        egui::ComboBox::from_id_salt("scene").selected_text(text(Type::Body, name).strong()).height(480.0).show_ui(ui, |ui| {
            ui.set_min_width(240.0);
            let scenes: Vec<(String, u32)> = app.s.doc.scenes.iter().map(|s| (s.name.clone(), s.timeline.frame_count())).collect();
            for (i, (n, frames)) in scenes.iter().enumerate() {
                let r = ui.horizontal(|ui| {
                    let r = ui.selectable_label(i == app.s.scene, n.as_str());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(text(Type::DataSmall, format!("{frames} f")).color(T.text_faint));
                    });
                    r
                });
                if r.inner.clicked() && i != app.s.scene {
                    app.playing = false;
                    app.frame_sel = None;
                    app.run("scene.select", serde_json::json!({"scene": i}));
                } else if r.inner.clicked() && !app.s.editing.is_empty() {
                    // The scene itself, from inside a symbol: back to it (ADR 0045).
                    app.playing = false;
                    app.frame_sel = None;
                    app.run("symbol.exit", serde_json::json!({"all": true}));
                }
            }
            ui.separator();
            for id in ["scene.new", "scene.duplicate", "scene.rename", "scene.delete", "scene.move_up", "scene.move_down"] {
                crate::menus::cmd_item(app, ui, id);
            }
        })
        .response
        .set_name(tr!("Scene"));
        // The symbols open for editing (ADR 0045), as in Animate's edit bar: a click on one
        // goes back to it; the last is the one being edited.
        // A group (ADR 0073) shows as "Group" and a drawing object (ADR 0080) as "Drawing
        // Object", as Animate's edit bar does.
        let open: Vec<String> = app
            .s
            .editing
            .iter()
            .map(|l| match l.symbol() {
                Some(id) => app.s.doc.symbol(id).map_or_else(String::new, |s| s.name.clone()),
                None if l.open == anicraft_engine::editing::Opened::Drawing => tr!("Drawing Object").to_string(),
                None => tr!("Group").to_string(),
            })
            .collect();
        let mut back: Option<usize> = None;
        for (i, name) in open.iter().enumerate() {
            ui.label(text(Type::Caption, "›"));
            if i + 1 == open.len() {
                ui.label(text(Type::Body, name.clone()).strong());
            } else if ui.link(text(Type::Body, name.clone())).on_hover_text(tr!("Back to this symbol")).clicked() {
                back = Some(i + 1);
            }
        }
        if let Some(keep) = back {
            app.playing = false;
            while app.s.editing.len() > keep && app.run("symbol.exit", serde_json::json!({})).is_some() {}
        }
        ui.label(text(Type::Caption, "›"));
        let layer = app.s.active_layer().map_or("—".to_string(), |l| l.name.clone());
        ui.label(text(Type::Body, layer).color(T.text_dim));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let z = app.view.zoom;
            let canvas = app.stage_cache.canvas;
            egui::ComboBox::from_id_salt("zoom").width(84.0).selected_text(text(Type::Data, format!("{:>4.0}%", z * 100.0))).show_ui(ui, |ui| {
                if ui.selectable_label(false, tr!("Fit in window")).clicked() {
                    app.view.fit = true;
                }
                for pct in [25.0f32, 50.0, 100.0, 200.0, 400.0, 800.0] {
                    if ui.selectable_label((z * 100.0 - pct).abs() < 0.5, text(Type::Data, format!("{pct:>4.0}%"))).clicked() {
                        app.zoom_about(canvas, canvas.center(), pct / 100.0);
                    }
                }
            })
            .response
            .set_name(tr!("Zoom"));
            // Beside the zoom, as in Animate (#251): a toggle, on = the pasteboard is hidden.
            let clip = app.view.clip_to_stage;
            if crate::timeline::icon_button(ui, tr!("Clip content outside the stage"), Some(clip), Icon::ClipStage).clicked() {
                app.view.clip_to_stage = !clip;
            }
            if ui.selectable_label(app.onion.ghosts, tr!("Onion skin")).clicked() {
                crate::onion::toggle_ghosts(app);
            }
            let st = app.s.doc.stage;
            ui.label(text(Type::Data, format!("{} × {} px", st.width, st.height)).color(T.text_faint)); // i18n-exempt: stage size, numbers and a unit
        });
    });
}

pub fn status_bar(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal_centered(|ui| {
        let now = ui.input(|i| i.time);
        // Readouts from the right first; the message takes the rest and is cut short (whole on
        // hover) rather than running under them. Narrow windows and large interface scales drop
        // the render and cache diagnostics before the message.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let wide = ui.available_width() >= 1000.0;
            let small = |t: String| text(Type::Data, t).color(T.text_faint);
            ui.label(small(tr!("undo {steps} · {size} MB", steps = app.s.history().undo_len(), size = format!("{:.1}", app.s.history().bytes() as f32 / (1024.0 * 1024.0)))));
            ui.add_space(12.0);
            if let Some(a) = crate::autosave::status(app, now) {
                ui.label(small(a)).on_hover_text(tr!("A recovery copy of your unsaved work. If AniCraft closes unexpectedly, the next start offers it back."));
                ui.add_space(12.0);
            }
            if wide {
                if let Some((n, mb, hit)) = app.stage_cache.stats() {
                    ui.label(small(tr!("cache {frames} fr · {size} MB · {hit}% hit", frames = n, size = format!("{mb:.0}"), hit = format!("{hit:.0}"))));
                    ui.add_space(12.0);
                }
                let (ready, needed) = app.stage_cache.ready;
                let ms = format!("{:.1}", app.stage_cache.ms);
                let line = match renderer_tag(app) {
                    Some(renderer) => tr!("render {ms} ms · {renderer} · tiles {ready}/{needed}", ms = ms, renderer = renderer, ready = ready, needed = needed),
                    None => tr!("render {ms} ms · tiles {ready}/{needed}", ms = ms, ready = ready, needed = needed),
                };
                ui.label(small(line));
            }
            if let Some(p) = app.cursor_stage {
                ui.label(small(format!("x {:.0}  y {:.0}", p.x, p.y)));
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let msg = app.status.as_ref().filter(|s| s.error || now - s.at < 8.0 || s.at == 0.0);
                let (line, colour) = match msg {
                    Some(s) => (s.text.as_str(), if s.error { T.error } else { T.ok }),
                    None => (crate::i18n::t(app.tool.hint()), color(Type::Label)),
                };
                let r = ui.add(egui::Label::new(text(Type::Label, line).color(colour)).truncate());
                // Screen readers announce what an action did ("Deleted layer — Ctrl+Z to undo")
                // without moving focus (a polite live region).
                ui.ctx().accesskit_node_builder(r.id, |n| n.set_live(egui::accesskit::Live::Polite));
            });
        });
    });
}

/// Who draws the stage, when this build can draw it on the GPU (ADR 0019): "GPU", or "CPU"
/// with what kept the GPU out.
fn renderer_tag(app: &App) -> Option<String> {
    use crate::gpu_stage::State;
    if !crate::gpu_stage::AVAILABLE {
        return None;
    }
    Some(match (app.stage_cache.gpu.state(), &app.stage_cache.drawn_by) {
        (State::Starting, _) => tr!("GPU starting").into(),
        (State::Failed(_), _) => tr!("CPU (GPU unavailable)").into(),
        (State::Stopped { retry_at: Some(_), .. }, _) => tr!("CPU (GPU stopped; trying again soon)").into(),
        (State::Stopped { retry_at: None, .. }, _) => tr!("CPU (GPU stopped)").into(),
        (State::Running { .. }, Some((true, _))) => "GPU".into(), // i18n-exempt: renderer name
        (State::Running { .. }, Some((false, Some(why)))) => tr!("CPU for {reason}", reason = crate::i18n::t_dyn(why)),
        _ => "CPU".into(), // i18n-exempt: renderer name
    })
}

// ----- right dock ---------------------------------------------------------------------------

fn rgb(c: Rgba) -> [u8; 3] {
    let c = c.sanitized();
    [(c.r * 255.0).round() as u8, (c.g * 255.0).round() as u8, (c.b * 255.0).round() as u8]
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// Runs a command from a continuously-edited widget, coalescing into one undo step.
pub(crate) fn live(app: &mut App, label: &str, id: &str, p: Value) {
    app.s.begin_gesture(label);
    app.run(id, p);
}

/// A symbol's 9-slice guides (ADR 0055): Animate's "Enable guides for 9-slice scaling" and,
/// when on, where the four guides are (in the symbol's space; also dragged on the stage while
/// the symbol is open). `place` keeps two copies of the controls apart (Library, Properties).
pub(crate) fn scale_grid_props(app: &mut App, ui: &mut egui::Ui, id: anicraft_engine::doc::SymbolId, place: &str) {
    let Some(grid) = app.s.doc.symbol(id).map(|s| s.scale_grid) else { return };
    let mut on = grid.is_some();
    ui.push_id(place, |ui| {
        let tip = tr!("Scaled instances keep the corners outside the guides unscaled, stretch the edges one way and the centre both ways (for buttons and panels). Edit the symbol to drag the guides");
        if crate::theme::checkbox(ui, &mut on, tr!("Enable guides for 9-slice scaling")).on_hover_text(tip).changed() {
            app.run("symbol.set_scale_grid", json!({"symbol": id.0, "on": on}));
        }
        let Some(g) = grid else { return };
        let limit = anicraft_engine::doc::MAX_GRID_COORD;
        let (mut l, mut r, mut t, mut b) = (g.left, g.right, g.top, g.bottom);
        let mut changed = None;
        row(ui, tr!("Guides x"), |ui| {
            if ui.add(egui::DragValue::new(&mut l).range(-limit..=g.right).speed(0.5).max_decimals(1)).named(tr!("Left 9-slice guide")).changed() {
                changed = Some(("left", l));
            }
            if ui.add(egui::DragValue::new(&mut r).range(g.left..=limit).speed(0.5).max_decimals(1)).named(tr!("Right 9-slice guide")).changed() {
                changed = Some(("right", r));
            }
        });
        row(ui, tr!("Guides y"), |ui| {
            if ui.add(egui::DragValue::new(&mut t).range(-limit..=g.bottom).speed(0.5).max_decimals(1)).named(tr!("Top 9-slice guide")).changed() {
                changed = Some(("top", t));
            }
            if ui.add(egui::DragValue::new(&mut b).range(g.top..=limit).speed(0.5).max_decimals(1)).named(tr!("Bottom 9-slice guide")).changed() {
                changed = Some(("bottom", b));
            }
        });
        if let Some((key, v)) = changed {
            live(app, tr_noop!("Move 9-Slice Guide"), "symbol.set_scale_grid", json!({"symbol": id.0, key: v}));
        }
    });
}

/// A property row for values: proportional label (left-aligned column), monospace fields.
fn row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    crate::theme::form_row(ui, crate::theme::FORM_LABEL_DOCK, label, |ui| with_type(ui, Type::Data, add));
}

/// A property row whose field holds prose (names, labels, preset choices).
fn row_prose(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    crate::theme::form_row(ui, crate::theme::FORM_LABEL_DOCK, label, add);
}

/// The Colour panel (ADR 0075): the stroke and fill colours new drawing uses, and, with
/// something selected, the selection's (one undo step per change), with Swap and Black and
/// White as in Animate.
pub(crate) fn colour_panel(app: &mut App, ui: &mut egui::Ui) {
    let mut st = app.s.style.clone();
    let to_rgba = |c: [u8; 3]| Rgba::new(f32::from(c[0]) / 255.0, f32::from(c[1]) / 255.0, f32::from(c[2]) / 255.0, 1.0);
    let (mut stroke, mut fill) = (rgb(st.stroke), rgb(st.fill));
    let (mut stroke_changed, mut fill_changed) = (false, false);
    let mut target = app.colour_target;
    row(ui, tr!("Stroke"), |ui| {
        let r = crate::colour::button(ui, &mut stroke).named(tr!("Stroke colour"));
        if r.clicked() {
            target = crate::swatches_panel::ColourTarget::Stroke;
        }
        stroke_changed = r.changed();
        ui.label(text(Type::Data, hex(stroke)).color(T.text_dim));
    });
    row(ui, tr!("Fill"), |ui| {
        let r = crate::colour::button(ui, &mut fill).named(tr!("Fill colour"));
        if r.clicked() {
            target = crate::swatches_panel::ColourTarget::Fill;
        }
        fill_changed = r.changed();
        ui.label(text(Type::Data, hex(fill)).color(T.text_dim));
    });
    let mut undo = tr_noop!("Colour");
    ui.horizontal_wrapped(|ui| {
        if ui.button(tr!("Swap")).on_hover_text(tr!("Swap the stroke and fill colours")).clicked() {
            (stroke, fill) = (fill, stroke);
            (stroke_changed, fill_changed, undo) = (true, true, tr_noop!("Swap Colours"));
        }
        if ui.button(tr!("Black and White")).on_hover_text(tr!("A black stroke and a white fill")).clicked() {
            (stroke, fill) = ([0, 0, 0], [255, 255, 255]);
            (stroke_changed, fill_changed, undo) = (true, true, tr_noop!("Black and White"));
        }
    });
    app.colour_target = target;
    crate::swatches_panel::target_toggle(app, ui);
    if !(stroke_changed || fill_changed) {
        ui.label(text(Type::Caption, if app.s.selection.is_empty() { tr!("New shapes and text use these colours.") } else { tr!("Changes apply to the selection and to new drawing.") }));
        return;
    }
    if stroke_changed {
        st.stroke = to_rgba(stroke);
    }
    if fill_changed {
        st.set_fill_color(to_rgba(fill));
    }
    app.s.style = st;
    if !app.s.selection.is_empty() {
        if stroke_changed {
            live(app, undo, "element.set_stroke", json!({"color": hex(stroke)}));
        }
        if fill_changed {
            live(app, undo, "element.set_fill", json!({"color": hex(fill)}));
        }
    }
}

/// The Align panel: align edges or centres, distribute, relative to the selection or the stage.
pub(crate) fn align_panel(app: &mut App, ui: &mut egui::Ui) {
    let has_sel = !app.s.selection.is_empty();
    let button = |app: &mut App, ui: &mut egui::Ui, icon: Icon, id: &str| {
        let (rect, resp) = ui.allocate_exact_size(vec2(TARGET + 4.0, TARGET), if has_sel { Sense::click() } else { Sense::hover() });
        if has_sel && resp.hovered() {
            ui.painter().rect_filled(rect, T.rc(), T.hover);
        }
        icons::paint(ui.painter(), Rect::from_center_size(rect.center(), vec2(18.0, 18.0)), icon, if !has_sel { T.text_faint } else if resp.hovered() { T.icon_hot } else { T.icon });
        let spec = anicraft_engine::find_command(id);
        let tip = spec.map_or_else(String::new, |c| match app.keymap.primary(c.id) {
            Some(k) => format!("{} ({})", crate::i18n::t_cmd(c.id, c.label), crate::platform::shortcut_label(k)),
            None => crate::i18n::t_cmd(c.id, c.label).to_string(),
        });
        a11y::control(ui, &resp, WidgetType::Button, &tip, None);
        let tip = if has_sel { tip } else { tr!("{command}\nSelect something to align", command = tip) };
        if resp.on_hover_text(tip).clicked() {
            // Leave it unset when off, so a single shape still aligns to the stage.
            app.run(id, if app.align_to_stage { json!({"to_stage": true}) } else { json!({}) });
        }
    };
    // Rows wrap in a narrow dock (ADR 0087): the vertical alignments go under the horizontal.
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (icon, id) in [(Icon::AlignLeft, "align.left"), (Icon::AlignHCenter, "align.hcenter"), (Icon::AlignRight, "align.right")] {
            button(app, ui, icon, id);
        }
        ui.add_space(10.0);
        for (icon, id) in [(Icon::AlignTop, "align.top"), (Icon::AlignVCenter, "align.vcenter"), (Icon::AlignBottom, "align.bottom")] {
            button(app, ui, icon, id);
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        button(app, ui, Icon::DistributeH, "align.distribute_h");
        button(app, ui, Icon::DistributeV, "align.distribute_v");
        ui.add_space(12.0);
        crate::theme::checkbox(ui, &mut app.align_to_stage, text(Type::Body, tr!("To stage"))).on_hover_text(tr!("Align to the stage instead of the selection's bounds (a single shape always aligns to the stage)"));
    });
}

/// Undo history: past steps (click to go back), the current state, and redoable steps (dimmed).
pub(crate) fn history(app: &mut App, ui: &mut egui::Ui) {
    let hist = app.s.history();
    let past: Vec<String> = hist.undo_labels().map(str::to_string).collect();
    let future: Vec<String> = hist.redo_labels().map(str::to_string).collect();
    let evicted = hist.evicted;
    let mut target = None;
    egui::ScrollArea::vertical().id_salt("history").max_height(ui.available_height().max(60.0)).auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
        let base = if evicted > 0 { tr!("… older steps freed (memory budget)") } else { tr!("Opened document") };
        if ui.selectable_label(past.is_empty(), text(Type::Body, base).color(if evicted > 0 { T.text_faint } else { T.text_dim })).clicked() {
            target = Some(0);
        }
        for (i, label) in past.iter().enumerate() {
            let current = i + 1 == past.len();
            if ui.selectable_label(current, crate::i18n::t_dyn(label).into_owned()).clicked() {
                target = Some(i + 1);
            }
        }
        for (i, label) in future.iter().enumerate() {
            if ui.selectable_label(false, text(Type::Body, crate::i18n::t_dyn(label)).color(T.text_faint).italics()).clicked() {
                target = Some(past.len() + i + 1);
            }
        }
    });
    if let Some(t) = target
        && let Err(e) = app.s.travel_to(t)
    {
        app.status_err(e.to_string());
    }
}

pub(crate) fn properties(app: &mut App, ui: &mut egui::Ui) {
    // The selection on every layer (ADR 0044).
    let sel: Vec<_> = app.s.selected_elements().into_iter().map(|(_, e)| e).collect();

    // ---- tool options first while a drawing tool is in hand: they're what you reach for ----
    if matches!(app.tool, Tool::Rect | Tool::Oval | Tool::Star | Tool::Line | Tool::Pen | Tool::Text | Tool::Pencil | Tool::Brush | Tool::PaintBrush | Tool::Eraser | Tool::PaintBucket | Tool::InkBottle | Tool::Eyedropper) {
        let title = tr!("{tool} tool", tool = crate::i18n::t(app.tool.label()));
        section(ui, "prop_tool", Section::Rule, &title, None, |ui| tool_props(app, ui));
    }
    // ---- Snap to Objects for the tools that drag points and selections (ADR 0042) ----
    if matches!(app.tool, Tool::Select | Tool::Subselect | Tool::Transform) {
        let title = tr!("{tool} tool", tool = crate::i18n::t(app.tool.label()));
        section(ui, "prop_tool", Section::Rule, &title, None, |ui| {
            let mut on = app.s.style.snap_objects;
            row_prose(ui, tr!("Snapping"), |ui| {
                crate::theme::checkbox(ui, &mut on, tr!("Snap to objects")).on_hover_text(tr!("Dragged points and selections snap to points, lines and fill edges within 10 pixels (the magnet under the tools)"));
            });
            if on != app.s.style.snap_objects {
                app.run("draw.snap_to_objects", json!({"on": on}));
            }
        });
    }
    // ---- the camera, with the Camera tool or its layer active (ADR 0027) ----
    let camera_layer = app.s.timeline().camera_layer().map(|l| l.id);
    if camera_layer.is_some() && (app.tool == Tool::Camera || camera_layer == Some(app.s.layer)) {
        section(ui, "prop_camera", Section::Rule, tr!("Camera"), None, |ui| camera_props(app, ui));
    }
    // ---- the bone picked on the stage (ADR 0043) ----
    if let Some(id) = app.bone_sel {
        let name = app.s.timeline().layers.iter().find_map(|l| l.armature.as_ref().and_then(|a| a.bone(id)).map(|b| b.name.clone()));
        if let Some(name) = name {
            let title = if name.is_empty() { tr!("Bone").to_string() } else { tr!("Bone: {name}", name = name) };
            section(ui, "prop_bone", Section::Rule, &title, None, |ui| bone_props(app, ui, id));
        }
    }
    // ---- Asset Warp (ADR 0079): the warped artwork picked with the tool, or selected ----
    let warp_target = app.warp_pin.map(|(e, _)| e).or_else(|| match sel.as_slice() {
        [e] if matches!(&e.kind, ElementKind::Shape(sh) if sh.live_warp().is_some()) => Some(e.id),
        _ => None,
    });
    if app.tool == Tool::Warp || warp_target.is_some() {
        section(ui, "prop_warp", Section::Rule, tr!("Asset Warp"), None, |ui| warp_props(app, ui, warp_target));
    }
    if let Some(e) = sel.first() {
        // One count per template: the layers' phrase is its own plural, with its preposition, so
        // its grammar travels with it, and the outer sentence places it.
        let title = match (sel.len(), app.s.selected_layers().len()) {
            (1, _) => element_title(&app.s.doc, e),
            (n, 0 | 1) => trn!("{n} element", "{n} elements", n),
            (n, l) => trn!("{n} element {on_layers}", "{n} elements {on_layers}", n, on_layers = trn!("on {n} layer", "on {n} layers", l)),
        };
        section(ui, "prop_selection", Section::Rule, &title, None, |ui| selection_props(app, ui, e, &sel));
    } else if let Some((id, name)) = app.s.editing_symbol().and_then(|id| app.s.doc.symbol(id).map(|x| (id, x.name.clone()))) {
        // Inside a symbol, nothing selected: the symbol's own settings (ADR 0055).
        section(ui, "prop_symbol", Section::Rule, &tr!("Symbol: {name}", name = name), None, |ui| scale_grid_props(app, ui, id, "open"));
    } else {
        section(ui, "prop_document", Section::Rule, tr!("Document"), None, |ui| document_props(app, ui));
    }
    // ---- frame ----
    let frame = app.s.frame;
    if let Some(layer) = app.s.active_layer().cloned() {
        // Mixed title: prose label, data value.
        let detail = format!("{} · {}", frame + 1, layer.name);
        section(ui, "prop_frame", Section::Rule, tr!("Frame"), Some(&detail), |ui| frame_props(app, ui, &layer, frame));
    }
}

/// The Blending row of a movie clip or button instance.
fn blend_row(app: &mut App, ui: &mut egui::Ui, current: anicraft_engine::doc::BlendMode, differs: bool) {
    row_prose(ui, tr!("Blending"), |ui| {
        egui::ComboBox::from_id_salt("prop_blend").selected_text(crate::i18n::t(current.label())).show_ui(ui, |ui| {
            for mode in anicraft_engine::doc::BlendMode::ALL {
                if ui.selectable_label(mode == current, crate::i18n::t(mode.label())).clicked() && (mode != current || differs) {
                    app.run("element.set_blend", json!({"mode": mode.key()}));
                }
            }
        })
        .response
        .set_name(tr!("Blending"));
        mixed(ui, differs);
    });
}

/// A symbol type's name, as the Properties panel shows it.
pub(crate) fn symbol_kind_label(kind: SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Graphic => tr!("Graphic"),
        SymbolKind::MovieClip => tr!("Movie clip"),
        SymbolKind::Button => tr!("Button"),
    }
}

/// [`anicraft_engine::element_label`] in the interface language: the selection's title (the
/// same English; `i18n::tests` compares them).
pub(crate) fn element_title(doc: &anicraft_engine::doc::Document, e: &anicraft_engine::doc::Element) -> String {
    let kind = match &e.kind {
        ElementKind::Shape(_) => tr!("Shape").to_string(),
        ElementKind::Symbol { symbol, .. } => match doc.symbol(*symbol) {
            Some(s) => match s.kind {
                SymbolKind::Graphic => tr!("Graphic: {name}", name = s.name),
                SymbolKind::MovieClip => tr!("Movie clip: {name}", name = s.name),
                SymbolKind::Button => tr!("Button: {name}", name = s.name),
            },
            None => tr!("Graphic: missing").to_string(),
        },
        ElementKind::Camera => tr!("Camera").to_string(),
        ElementKind::Group { .. } => tr!("Group").to_string(),
        ElementKind::Text(t) if t.text.trim().is_empty() => text_kind_label(t.kind()).to_string(),
        ElementKind::Text(t) => {
            let first: String = t.text.lines().next().unwrap_or("").chars().take(24).collect();
            let more = if t.text.chars().count() > first.chars().count() { "…" } else { "" };
            tr!("Text: {text}", text = format!("{first}{more}"))
        }
    };
    if e.name.is_empty() { kind } else { tr!("{name} ({kind})", name = e.name, kind = kind) }
}

/// "mixed" after a value that differs across the selection (the field shows the first one's).
fn mixed(ui: &mut egui::Ui, differs: bool) {
    if differs {
        ui.label(text(Type::Caption, tr!("mixed")).color(T.text_faint)).on_hover_text(tr!("The selected elements differ here; it shows the first one's. A change applies to all of them"));
    }
}

/// Whose colour effect the rows edit: the selected elements' (every style), or the camera's
/// (ADR 0082: no opacity, so no Alpha style and no alpha channel in Advanced).
#[derive(Clone, Copy, PartialEq)]
enum EffectOf {
    Selection,
    Camera,
}

impl EffectOf {
    fn command(self) -> &'static str {
        match self {
            EffectOf::Selection => "element.set_color_effect",
            EffectOf::Camera => "camera.set",
        }
    }
}

/// The colour effect (ADR 0056): Animate's Style menu, then the chosen style's controls. Each
/// change sets that value on every selected element (ADR 0053), or on the camera, as one undo
/// step per drag.
fn colour_effect_rows(app: &mut App, ui: &mut egui::Ui, fx: &ColorEffect, m: crate::mixed::ColorMixed, of: EffectOf) {
    let current = fx.style();
    let cmd = of.command();
    let camera = of == EffectOf::Camera;
    row_prose(ui, tr!("Colour effect"), |ui| {
        egui::ComboBox::from_id_salt(if camera { "prop_camera_color_style" } else { "prop_color_style" }).selected_text(crate::i18n::t(current.label())).show_ui(ui, |ui| {
            for st in ColorStyle::ALL.into_iter().filter(|st| !camera || *st != ColorStyle::Alpha) {
                if ui.selectable_label(st == current, crate::i18n::t(st.label())).clicked() && (st != current || m.style) {
                    app.run(cmd, json!({"style": st.key()}));
                }
            }
        })
        .response
        .set_name(if camera { tr!("Camera colour effect style") } else { tr!("Colour effect style") });
        mixed(ui, m.style);
    });
    let pct = a11y::SliderLook { suffix: " %", max_decimals: Some(0), ..Default::default() };
    let set = |app: &mut App, p: Value| live(app, if camera { tr_noop!("Camera Colour Effect") } else { tr_noop!("Colour Effect") }, cmd, p);
    match *fx {
        ColorEffect::None => {}
        ColorEffect::Alpha(a) => {
            let mut v = a * 100.0;
            row(ui, tr!("Alpha"), |ui| {
                if a11y::slider(ui, &mut v, 0.0..=100.0, tr!("Alpha"), pct).changed() {
                    set(app, json!({"alpha": v}));
                }
                mixed(ui, m.alpha);
            });
        }
        ColorEffect::Brightness(b) => {
            let mut v = b * 100.0;
            row(ui, tr!("Brightness"), |ui| {
                if a11y::slider(ui, &mut v, -100.0..=100.0, tr!("Brightness"), pct).changed() {
                    set(app, json!({"brightness": v}));
                }
                mixed(ui, m.brightness);
            });
        }
        ColorEffect::Tint { color, amount } => {
            let mut c = rgb(color);
            row(ui, tr!("Tint"), |ui| {
                if crate::colour::button(ui, &mut c).named(tr!("Tint colour")).changed() {
                    set(app, json!({"tint": hex(c)}));
                }
                ui.label(text(Type::Data, hex(c)));
                mixed(ui, m.tint);
            });
            let mut v = amount * 100.0;
            row(ui, tr!("Tint amount"), |ui| {
                if a11y::slider(ui, &mut v, 0.0..=100.0, tr!("Tint amount"), pct).changed() {
                    set(app, json!({"tint_amount": v}));
                }
                mixed(ui, m.tint_amount);
            });
        }
        ColorEffect::Advanced(t) => {
            // Each channel: its multiplier in percent, then the offset added (−255..255).
            let channels = [(tr!("Red"), tr!("Red multiplier"), tr!("Red offset"), "red"), (tr!("Green"), tr!("Green multiplier"), tr!("Green offset"), "green"), (tr!("Blue"), tr!("Blue multiplier"), tr!("Blue offset"), "blue"), (tr!("Alpha"), tr!("Alpha multiplier"), tr!("Alpha offset"), "alpha")];
            // The camera has no opacity: its Advanced effect has no alpha channel.
            for (i, (label, mul_name, off_name, key)) in channels.into_iter().enumerate().take(if camera { 3 } else { 4 }) {
                let mut mv = t.mul.get(i).copied().unwrap_or(1.0) * 100.0;
                let mut ov = t.add.get(i).copied().unwrap_or(0.0) * 255.0;
                row(ui, label, |ui| {
                    if ui.add(egui::DragValue::new(&mut mv).suffix(" %").speed(0.5).range(-100.0..=100.0).max_decimals(0)).named(mul_name).changed() {
                        set(app, json!({format!("{key}_multiplier"): mv}));
                    }
                    if ui.add(egui::DragValue::new(&mut ov).custom_formatter(|v, _| format!("{v:+.0}")).speed(1.0).range(-255.0..=255.0).max_decimals(0)).named(off_name).changed() {
                        set(app, json!({format!("{key}_offset"): ov}));
                    }
                    mixed(ui, m.advanced.get(i).copied().unwrap_or(false) || m.advanced.get(i + 4).copied().unwrap_or(false));
                });
            }
        }
    }
}

/// The selection's properties: `e` is the first selected element, `all` the whole selection.
fn selection_props(app: &mut App, ui: &mut egui::Ui, e: &anicraft_engine::doc::Element, all: &[std::sync::Arc<anicraft_engine::doc::Element>]) {
    let many = all.len() > 1;
    // Which rows differ across the selection (ADR 0053): they say "mixed", and a change sets all.
    let m = crate::mixed::of(&app.s.doc, all);
    // Members of an armature (ADR 0084): their bones place them, so only the look is edited
    // here, in place on every pose.
    let tl = app.s.timeline();
    let layers = app.s.selected_layers();
    if !layers.is_empty() && layers.iter().all(|l| tl.layer(*l).is_some_and(|l| l.kind == anicraft_engine::doc::LayerKind::Armature)) {
        colour_effect_rows(app, ui, &e.color, m.color, EffectOf::Selection);
        if let ElementKind::Symbol { symbol, filters, blend, .. } = &e.kind
            && app.s.doc.symbol(*symbol).is_some_and(|x| x.kind != SymbolKind::Graphic)
        {
            filter_rows(app, ui, filters, m.filters);
            blend_row(app, ui, *blend, m.blend);
        }
        ui.label(text(Type::Caption, tr!("Posed by its bone: drag it with the Selection tool. Colour effect, blending and filters apply on every pose of the armature.")));
        return;
    }
    if let ElementKind::Symbol { symbol, events, .. } = &e.kind
        && app.s.doc.symbol(*symbol).is_some_and(|x| x.kind != SymbolKind::Graphic)
    {
        instance_rows(app, ui, e, events);
    }
    {
        let d = e.transform.decompose();
        let (mut x, mut y, mut sx, mut sy, mut rot) = (d.tx, d.ty, d.sx * 100.0, d.sy * 100.0, d.rotation.to_degrees());
        if many {
            // Several elements: where their combined box is, as in Animate; a change moves them
            // all together.
            let b = app.s.selection_bounds();
            let (ox, oy) = (b.min.x, b.min.y);
            let (mut bx, mut by) = (ox, oy);
            row(ui, tr!("Position"), |ui| {
                let a = ui.add(egui::DragValue::new(&mut bx).prefix("X ").speed(1.0).max_decimals(1)).named(tr!("Position X"));
                let c = ui.add(egui::DragValue::new(&mut by).prefix("Y ").speed(1.0).max_decimals(1)).named(tr!("Position Y"));
                if (a.changed() || c.changed()) && !b.is_empty() {
                    live(app, tr_noop!("Move"), "element.translate", json!({"dx": bx - ox, "dy": by - oy}));
                }
            });
        } else {
            row(ui, tr!("Position"), |ui| {
                let a = ui.add(egui::DragValue::new(&mut x).prefix("X ").speed(1.0).max_decimals(1)).named(tr!("Position X"));
                let b = ui.add(egui::DragValue::new(&mut y).prefix("Y ").speed(1.0).max_decimals(1)).named(tr!("Position Y"));
                if a.changed() || b.changed() {
                    live(app, tr_noop!("Move"), "element.set_transform", json!({"x": x, "y": y}));
                }
            });
        }
        row(ui, tr!("Scale"), |ui| {
            let a = ui.add(egui::DragValue::new(&mut sx).suffix(" %").speed(0.5).range(1.0..=10000.0).max_decimals(1)).named(tr!("Scale width"));
            let b = ui.add(egui::DragValue::new(&mut sy).suffix(" %").speed(0.5).range(1.0..=10000.0).max_decimals(1)).named(tr!("Scale height"));
            if a.changed() || b.changed() {
                live(app, tr_noop!("Scale"), "element.set_transform", json!({"sx": sx / 100.0, "sy": sy / 100.0}));
            }
            mixed(ui, m.scale);
        });
        row(ui, tr!("Rotation"), |ui| {
            if ui.add(egui::DragValue::new(&mut rot).suffix("°").speed(0.5).range(-360.0..=360.0).max_decimals(1)).named(tr!("Rotation")).changed() {
                live(app, tr_noop!("Rotate"), "element.set_transform", json!({"rotation": rot}));
            }
            mixed(ui, m.rotation);
        });
        if !matches!(e.kind, ElementKind::Camera) {
            colour_effect_rows(app, ui, &e.color, m.color, EffectOf::Selection);
        }
        // Blending and filters, on movie clips and buttons as in Animate.
        if let ElementKind::Symbol { symbol, filters, .. } = &e.kind
            && app.s.doc.symbol(*symbol).is_some_and(|x| x.kind != SymbolKind::Graphic)
        {
            filter_rows(app, ui, filters, m.filters);
        }
        if let ElementKind::Symbol { symbol, blend, .. } = &e.kind
            && app.s.doc.symbol(*symbol).is_some_and(|x| x.kind != SymbolKind::Graphic)
        {
            blend_row(app, ui, *blend, m.blend);
        }
        if let ElementKind::Symbol { symbol, loop_mode, first_frame, .. } = &e.kind
            && let Some(sym) = app.s.doc.symbol(*symbol).map(|s| (s.kind, s.timeline.frame_count()))
        {
            use anicraft_engine::doc::LoopMode;
            let (kind, frames) = sym;
            ui.add_space(4.0);
            // How the instance plays: the symbol's type, and a graphic's looping.
            row_prose(ui, tr!("Type"), |ui| {
                egui::ComboBox::from_id_salt("prop_symkind").width(crate::theme::fit(ui, 150.0)).selected_text(symbol_kind_label(kind)).show_ui(ui, |ui| {
                    for (k, id, tip) in [
                        (SymbolKind::Graphic, "graphic", tr!("Keeps step with the timeline it is on (looping options below)")),
                        (SymbolKind::MovieClip, "movie_clip", tr!("Plays its own timeline on a loop, from when it appears")),
                        (SymbolKind::Button, "button", tr!("Shows its Up frame")),
                    ] {
                        if ui.selectable_label(kind == k, symbol_kind_label(k)).on_hover_text(tip).clicked() && (kind != k || m.kind) {
                            // Every selected instance's symbol (ADR 0053), or just this one's.
                            app.run("symbol.set_kind", if many { json!({"kind": id}) } else { json!({"symbol": symbol.0, "kind": id}) });
                        }
                    }
                })
                .response
                .set_name(tr!("Symbol type"));
                mixed(ui, m.kind);
            });
            // Swap Symbol (ADR 0050): another symbol in this instance's place, keeping its
            // transform, colour effect, filters and name.
            let current = *symbol;
            let mut choices: Vec<(anicraft_engine::doc::SymbolId, String)> = app.s.doc.library.iter().map(|x| (x.id, app.s.doc.entry_path(anicraft_engine::doc::LibraryEntry::Item(LibraryItem::Symbol(x.id))))).collect();
            choices.sort_by_key(|(_, p)| p.to_lowercase());
            let name = app.s.doc.symbol(current).map(|x| x.name.clone()).unwrap_or_default();
            row_prose(ui, tr!("Symbol"), |ui| {
                ui.label(text(Type::Body, name));
                ui.menu_button(tr!("Swap…"), |ui| {
                    egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                        for (id, path) in &choices {
                            if ui.selectable_label(*id == current, path.as_str()).clicked() {
                                ui.close();
                                if *id != current || m.symbol {
                                    app.run("element.swap_symbol", json!({"symbol": id.0}));
                                }
                            }
                        }
                    });
                })
                .response
                .on_hover_text(tr!("Show another symbol here, keeping position, colour effect, filters and name"))
                .set_name(tr!("Swap Symbol"));
                mixed(ui, m.symbol);
            });
            // 9-slice scaling is the symbol's (ADR 0055): on here, it is on for every instance.
            let sym_id = *symbol;
            row_prose(ui, tr!("Scaling"), |ui| {
                ui.vertical(|ui| scale_grid_props(app, ui, sym_id, "instance"));
            });
            // The class scripts know the symbol by (kept, not run; ADR 0013), in documents
            // that use scripts.
            let linkage = app.s.doc.symbol(*symbol).and_then(|s| s.linkage.clone());
            if linkage.is_some() || crate::actions_panel::scripted(&app.s.doc) {
                let (class, base) = linkage.map_or((String::new(), String::new()), |l| (l.class, l.base));
                let hint = if base.is_empty() { tr!("The class scripts create this symbol by (its linkage). Kept with the document; scripts don't run yet").to_string() } else { tr!("The class scripts create this symbol by (its linkage), extending {base}. Kept with the document; scripts don't run yet", base = base) };
                row_prose(ui, tr!("Class"), |ui| {
                    if let Some(v) = crate::actions_panel::class_field(ui, "prop_symclass", &class, tr!("Linkage class"), &hint) {
                        let base = if v.is_empty() { String::new() } else { base.clone() };
                        app.run("script.set_class", json!({"symbol": symbol.0, "class": v, "base": base}));
                    }
                });
            }
            if kind == SymbolKind::Graphic {
                let names = [(LoopMode::Loop, "loop", tr!("Loop")), (LoopMode::PlayOnce, "play_once", tr!("Play once")), (LoopMode::SingleFrame, "single_frame", tr!("Single frame"))];
                let current = names.iter().find(|(mode, _, _)| mode == loop_mode).map_or(tr!("Loop"), |(_, _, l)| l);
                row_prose(ui, tr!("Looping"), |ui| {
                    egui::ComboBox::from_id_salt("prop_loop").width(crate::theme::fit(ui, 150.0)).selected_text(current).show_ui(ui, |ui| {
                        for (mode, id, l) in names {
                            if ui.selectable_label(mode == *loop_mode, l).clicked() && (mode != *loop_mode || m.looping) {
                                app.run("element.set_loop", json!({"loop": id}));
                            }
                        }
                    })
                    .response
                    .set_name(tr!("Looping"));
                    mixed(ui, m.looping);
                });
                let mut first = first_frame + 1;
                row(ui, tr!("First frame"), |ui| {
                    let r = ui.add(egui::DragValue::new(&mut first).range(1..=frames.max(1)).speed(0.2)).named(tr!("First frame"));
                    if r.changed() {
                        live(app, tr_noop!("First frame"), "element.set_loop", json!({"first_frame": first.saturating_sub(1)}));
                    }
                    ui.label(text(Type::Caption, tr!("of {frames}", frames = frames)));
                    mixed(ui, m.first_frame);
                });
            } else if kind == SymbolKind::MovieClip {
                ui.label(text(Type::Caption, trn!("Plays its {n}-frame timeline on a loop from when it appears.", "Plays its {n}-frame timeline on a loop from when it appears.", frames)));
            }
        }
        if let ElementKind::Text(t) = &e.kind {
            ui.add_space(4.0);
            text_field_rows(app, ui, e, t);
            let mut st = anicraft_engine::cmds_text::TextStyle { font: t.font.clone(), size: t.size, bold: t.bold, italic: t.italic, color: t.color, align: t.align, letter_spacing: t.letter_spacing, line_spacing: t.line_spacing };
            // A field's lines wrap by its line type, at its box (rows above); static text by width.
            let mut width = t.width;
            let wrap = if t.field.is_some() { None } else { Some(&mut width) };
            if let Some((label, p)) = text_style_rows(app, ui, &mut st, wrap) {
                live(app, label, "text.edit", p);
            }
            row_prose(ui, tr!("Text"), |ui| {
                if ui.button(tr!("Edit Text")).on_hover_text(tr!("Type on the stage (or double-click the text)")).clicked() {
                    let at = e.transform.apply(anicraft_engine::geom::Vec2::ZERO);
                    app.text_edit = Some(crate::app::TextEditing { at: pos2(at.x, at.y), element: Some(e.id), text: t.text.clone(), original: t.text.clone(), focus: true });
                }
            });
            text_path_rows(app, ui, e, t);
        }
        if let ElementKind::Shape(sh) = &e.kind {
            ui.add_space(4.0);
            let fill = sh.fill.as_ref().map(Paint::primary_color);
            let bitmaps = crate::colour_panel::bitmaps(app);
            row(ui, tr!("Fill"), |ui| {
                let c = rgb(fill.unwrap_or(Rgba::WHITE));
                // The one colour picker, with the fill's type (ADR 0074): a gradient shows as its
                // ramp here and is edited in the picker.
                let (r, edit) = crate::colour_panel::swatch(ui, sh.fill.as_ref(), &bitmaps);
                r.named(tr!("Fill colour")).on_hover_text(tr!("Fill: click for its colour, or a gradient or bitmap fill"));
                match edit {
                    Some(crate::colour_panel::FillEdit::Solid(p)) => live(app, tr_noop!("Fill colour"), "element.set_fill", json!({"color": hex(p)})),
                    Some(crate::colour_panel::FillEdit::None(n)) => {
                        app.run("element.set_fill", if n { json!({"color": null}) } else { json!({"color": hex(c)}) });
                    }
                    Some(edit) => crate::colour_panel::apply_set(app, &edit, true),
                    None => {}
                }
                let mut on = fill.is_some();
                if crate::theme::checkbox(ui, &mut on, "").named(tr!("Fill")).on_hover_text(tr!("Fill on/off")).changed() {
                    app.run("element.set_fill", if on { json!({"color": hex(c)}) } else { json!({"color": null}) });
                }
                if let Some(what) = fill_kind_label(sh.fill.as_ref()) {
                    ui.label(text(Type::Caption, what));
                }
                mixed(ui, m.fill);
            });
            if let Some(Paint::Bitmap { bitmap, .. }) = &sh.fill {
                bitmap_row(app, ui, *bitmap, m.bitmap);
            }
            let mut sc = rgb(sh.stroke.as_ref().map_or(app.s.style.stroke, |s| s.color));
            let mut sw = sh.stroke.as_ref().map_or(0.0, |s| s.width);
            row(ui, tr!("Stroke"), |ui| {
                if crate::colour::button(ui, &mut sc).named(tr!("Stroke colour")).changed() {
                    live(app, tr_noop!("Stroke colour"), "element.set_stroke", json!({"color": hex(sc)}));
                }
                if ui.add(egui::DragValue::new(&mut sw).suffix(" px").speed(0.1).range(0.0..=200.0).max_decimals(1)).named(tr!("Stroke width")).changed() {
                    live(app, tr_noop!("Stroke width"), "element.set_stroke", json!({"width": sw}));
                }
                mixed(ui, m.stroke);
            });
            if let Some(st) = &sh.stroke {
                // The width profile, as Animate's Width menu: built-ins with a picture of each.
                let current = st.profile.clone();
                if let Some(pick) = profile_picker(ui, ProfileRow::Selection, current.as_ref(), m.profile) {
                    app.run("stroke.set_profile", json!({"profile": pick.key()}));
                }
                // Style, caps and joins (ADR 0046).
                if let Some(edit) = line_style_rows(ui, ProfileRow::Selection, &st.style, m.style) {
                    if edit.drag {
                        live(app, edit.label, "stroke.set_style", edit.params);
                    } else {
                        app.run("stroke.set_style", edit.params);
                    }
                }
            }
        }
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            for (label, id) in [(tr!("Convert to Symbol"), "modify.convert_to_symbol"), (tr!("Break Apart"), "modify.break_apart"), (tr!("Duplicate"), "edit.duplicate"), (tr!("Delete"), "edit.delete")] {
                if ui.button(label).clicked() {
                    app.invoke(id);
                }
            }
        });
    }
}

/// The bitmap a placed bitmap or bitmap fill shows, with Swap Bitmap (ADR 0053): another
/// Library bitmap in its place, for every selected bitmap.
fn bitmap_row(app: &mut App, ui: &mut egui::Ui, current: anicraft_engine::doc::BitmapId, differs: bool) {
    let name = app.s.doc.bitmap(current).map_or_else(|| tr!("missing bitmap").to_string(), |b| format!("{} · {}×{}", b.name, b.width, b.height));
    let mut choices: Vec<(anicraft_engine::doc::BitmapId, String)> = app.s.doc.bitmaps.iter().map(|b| (b.id, app.s.doc.entry_path(anicraft_engine::doc::LibraryEntry::Item(LibraryItem::Bitmap(b.id))))).collect();
    choices.sort_by_key(|(_, p)| p.to_lowercase());
    row_prose(ui, tr!("Bitmap"), |ui| {
        ui.label(text(Type::Body, name));
        ui.menu_button(tr!("Swap…"), |ui| {
            egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                for (id, path) in &choices {
                    if ui.selectable_label(*id == current, path.as_str()).clicked() {
                        ui.close();
                        if *id != current || differs {
                            app.run("element.swap_bitmap", json!({"bitmap": id.0}));
                        }
                    }
                }
            });
        })
        .response
        .on_hover_text(tr!("Show another bitmap from the Library here, keeping its position and transform"))
        .set_name(tr!("Swap Bitmap"));
        mixed(ui, differs);
    });
}

/// Text on a path (ADR 0070): Attach to Path (then a click on a shape) for static text; for text
/// on a path, where it sits along it, its side, and Release.
fn text_path_rows(app: &mut App, ui: &mut egui::Ui, e: &anicraft_engine::doc::Element, t: &anicraft_engine::doc::TextBlock) {
    if t.field.is_some() {
        return;
    }
    let Some(on) = &t.on_path else {
        row_prose(ui, tr!("Path"), |ui| {
            let waiting = app.path_pick == Some(e.id);
            let label = if waiting { tr!("Click a shape…") } else { tr!("Attach to Path") };
            if ui.selectable_label(waiting, label).on_hover_text(tr!("Then click a shape on the stage: the text follows its outline (Esc cancels)")).clicked() {
                if waiting {
                    app.path_pick = None;
                } else {
                    app.path_pick = Some(e.id);
                    app.status_ok(tr!("Click a shape to set the text along its outline. Esc cancels."));
                }
            }
        });
        return;
    };
    let mut offset = on.offset;
    row(ui, tr!("Path"), |ui| {
        if ui.add(egui::DragValue::new(&mut offset).prefix(format!("{} ", tr!("Offset"))).suffix(" px").speed(0.5).range(-MAX_PATH_OFFSET..=MAX_PATH_OFFSET).max_decimals(1)).named(tr!("Offset along the path")).on_hover_text(tr!("Where the text sits along the path: its alignment decides whether it starts, is centred or ends there")).changed() {
            live(app, tr_noop!("Text on Path"), "text.path", json!({"offset": offset}));
        }
    });
    row_prose(ui, tr!("Side"), |ui| {
        let mut flip = on.flip;
        if crate::theme::checkbox(ui, &mut flip, tr!("Flip")).on_hover_text(tr!("Run the text along the path the other way, on its other side")).changed() {
            app.run("text.path", json!({"flip": flip}));
        }
        if ui.button(tr!("Release")).on_hover_text(tr!("Back to lines of text, where the text starts")).clicked() {
            app.run("text.release_path", json!({}));
        }
    });
}

/// A text type's name, as the Properties panel shows it.
pub(crate) fn text_kind_label(kind: TextKind) -> &'static str {
    match kind {
        TextKind::Static => tr!("Static text"),
        TextKind::Dynamic => tr!("Dynamic text"),
        TextKind::Input => tr!("Input text"),
    }
}

/// A line type's name, as the Properties panel shows it.
pub(crate) fn line_type_label(lines: LineType) -> &'static str {
    match lines {
        LineType::Single => tr!("Single line"),
        LineType::Multiline => tr!("Multiline"),
        LineType::MultilineNoWrap => tr!("Multiline no wrap"),
        LineType::Password => tr!("Password"),
    }
}

/// Text type (Static, Dynamic, Input) and, for dynamic and input fields, the instance name
/// scripts use, the line type, the box and the options (ADR 0066), where Animate's Properties
/// has them: at the top of a text field's properties.
fn text_field_rows(app: &mut App, ui: &mut egui::Ui, e: &anicraft_engine::doc::Element, t: &anicraft_engine::doc::TextBlock) {
    let kind = t.kind();
    row_prose(ui, tr!("Text type"), |ui| {
        egui::ComboBox::from_id_salt("prop_text_type").selected_text(text_kind_label(kind)).show_ui(ui, |ui| {
            for k in TextKind::ALL {
                if ui.selectable_label(k == kind, text_kind_label(k)).clicked() && k != kind {
                    app.run("text.field", json!({"type": k.key()}));
                }
            }
        })
        .response
        .on_hover_text(tr!("Static text is drawn as laid out. Scripts set dynamic text while the movie plays; viewers type into input text."))
        .set_name(tr!("Text type"));
    });
    let Some(f) = &t.field else { return };
    let mut name = e.name.clone();
    row_prose(ui, tr!("Instance"), |ui| {
        let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(crate::theme::fit(ui, 150.0)).hint_text(tr!("instance name"))).named(tr!("Instance name"));
        if r.on_hover_text(tr!("The name scripts use for the field, as in score_txt.text = \"10\" (letters, digits and _)")).changed() {
            live(app, tr_noop!("Instance name"), "instance.set_name", json!({"element": e.id.0, "name": name}));
        }
    });
    row_prose(ui, tr!("Lines"), |ui| {
        egui::ComboBox::from_id_salt("prop_text_lines").selected_text(line_type_label(f.lines)).show_ui(ui, |ui| {
            for l in LineType::ALL {
                // A password hides what is typed, so only input text has one.
                if l == LineType::Password && kind != TextKind::Input {
                    continue;
                }
                if ui.selectable_label(l == f.lines, line_type_label(l)).clicked() && l != f.lines {
                    app.run("text.field", json!({"lines": l.key()}));
                }
            }
        })
        .response
        .set_name(tr!("Line type"));
    });
    let (mut w, mut h) = (t.box_width, t.box_height);
    row(ui, tr!("Box"), |ui| {
        if ui.add(egui::DragValue::new(&mut w).prefix("W ").suffix(" px").speed(0.5).range(1.0..=100_000.0).max_decimals(1)).named(tr!("Field width")).changed() {
            live(app, tr_noop!("Field size"), "text.field", json!({"width": w}));
        }
        if ui.add(egui::DragValue::new(&mut h).prefix("H ").suffix(" px").speed(0.5).range(1.0..=100_000.0).max_decimals(1)).named(tr!("Field height")).changed() {
            live(app, tr_noop!("Field size"), "text.field", json!({"height": h}));
        }
    });
    ui.horizontal_wrapped(|ui| {
        // Animate's "Show border around text": a line round a filled box.
        let mut border = f.border;
        if crate::theme::checkbox(ui, &mut border, tr!("Show border")).on_hover_text(tr!("Draw a line round the field and fill its box")).changed() {
            app.run("text.field", json!({"border": border, "background": border}));
        }
        if kind == TextKind::Dynamic {
            let mut sel = f.selectable;
            if crate::theme::checkbox(ui, &mut sel, tr!("Selectable")).on_hover_text(tr!("Viewers can select the text while the movie plays")).changed() {
                app.run("text.field", json!({"selectable": sel}));
            }
        }
    });
    if kind == TextKind::Input {
        let mut max = f.max_chars;
        row(ui, tr!("Max characters"), |ui| {
            if ui.add(egui::DragValue::new(&mut max).speed(0.2).range(0..=anicraft_engine::doc::MAX_FIELD_CHARS as u32)).named(tr!("Max characters")).on_hover_text(tr!("Most characters a viewer may type (0: no limit)")).changed() {
                live(app, tr_noop!("Max characters"), "text.field", json!({"max_chars": max}));
            }
            if max == 0 {
                ui.label(text(Type::Caption, tr!("no limit")).color(T.text_faint));
            }
        });
    }
    if !f.variable.is_empty() {
        row(ui, tr!("Variable"), |ui| {
            ui.label(text(Type::Data, f.variable.as_str())).on_hover_text(tr!("Kept from the Animate document: an ActionScript 1.0 or 2.0 variable, which doesn't run here"));
        });
    }
    ui.add_space(4.0);
}

/// A movie clip or button instance's name (what actions address it by) and its events, as at
/// the top of Animate's Properties.
fn instance_rows(app: &mut App, ui: &mut egui::Ui, e: &anicraft_engine::doc::Element, events: &[anicraft_engine::doc::EventHandler]) {
    let mut name = e.name.clone();
    row_prose(ui, tr!("Instance"), |ui| {
        let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(crate::theme::fit(ui, 150.0)).hint_text(tr!("instance name"))).named(tr!("Instance name"));
        if r.on_hover_text(tr!("The name actions use to address it (letters, digits and _)")).changed() {
            live(app, tr_noop!("Instance name"), "instance.set_name", json!({"element": e.id.0, "name": name}));
        }
    });
    let key = app.keymap.primary("view.actions").map(crate::platform::shortcut_label).unwrap_or_default();
    row_prose(ui, tr!("Events"), |ui| {
        let what = if events.is_empty() { tr!("None").to_string() } else { events.iter().map(|h| crate::i18n::t(h.event.label())).collect::<Vec<_>>().join(", ") };
        ui.label(text(Type::Body, what).color(if events.is_empty() { T.text_faint } else { T.text }));
        if ui.small_button(tr!("Actions…")).on_hover_text(tr!("Edit what it does on clicks and the pointer ({key})", key = key)).clicked() {
            crate::actions_panel::open(app, crate::actions_panel::Tab::Instance);
        }
    });
    ui.add_space(4.0);
}

/// Naming a filter preset in Properties (ADR 0091): saving the list under a new name, or
/// renaming one.
#[derive(Clone, Debug, Default)]
pub(crate) struct FilterNaming {
    /// The name typed so far, and the preset being renamed (`None`: saving a new one).
    pub name: Option<(String, Option<String>)>,
}

/// Copies the session's filter presets into the preferences when a command changed them, and
/// saves them (not in screenshot or test runs, which never write the user's preferences).
pub fn sync_filter_presets(app: &mut App) {
    if app.s.filter_presets == app.prefs.filter_presets {
        return;
    }
    app.prefs.filter_presets = app.s.filter_presets.clone();
    if app.shot.is_none()
        && let Err(e) = app.prefs.save()
    {
        app.status_err(tr!("Could not save preferences: {e}", e = e));
    }
}

/// What the filter list was asked to do this frame (done after drawing it), each one command.
enum FilterAct {
    Move(usize, usize),
    Enable(Option<usize>, bool),
    Remove(Option<usize>),
    Copy(Option<usize>),
    Paste,
    Apply(String),
    Delete(String),
}

/// A filter's grip: drag it up or down the list to reorder, or, focused, Alt+Up and Alt+Down
/// move it.
fn filter_grip(ui: &mut egui::Ui, fname: &str, i: usize, n: usize, act: &mut Option<FilterAct>) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(14.0, crate::theme::TARGET), Sense::click_and_drag());
    let hot = resp.hovered() || resp.dragged() || resp.has_focus();
    let c = if hot { T.icon_hot } else { T.text_faint };
    for (dx, dy) in [(-2.5, -5.0), (2.5, -5.0), (-2.5, 0.0), (2.5, 0.0), (-2.5, 5.0), (2.5, 5.0)] {
        ui.painter().circle_filled(rect.center() + vec2(dx, dy), 1.4, c);
    }
    if resp.has_focus() {
        ui.painter().rect_stroke(rect.expand(1.0), 2.0, Stroke::new(1.0, T.text), StrokeKind::Outside);
        let (up, down, alt) = ui.input(|inp| (inp.key_pressed(egui::Key::ArrowUp), inp.key_pressed(egui::Key::ArrowDown), inp.modifiers.alt));
        if alt {
            ui.memory_mut(|m| m.set_focus_lock_filter(resp.id, egui::EventFilter { vertical_arrows: true, ..Default::default() }));
            if up && i > 0 {
                *act = Some(FilterAct::Move(i, i - 1));
            } else if down && i + 1 < n {
                *act = Some(FilterAct::Move(i, i + 1));
            }
        }
    }
    let (up, down) = (crate::platform::shortcut_label("Alt+Up"), crate::platform::shortcut_label("Alt+Down"));
    a11y::control(ui, &resp, WidgetType::Button, &tr!("Move {filter}: drag, or {up} and {down}", filter = fname, up = up.as_str(), down = down.as_str()), None);
    resp.on_hover_text(tr!("Drag to change the order (filters apply from the top down)"))
}

/// The Filters rows of a movie clip or button instance, as Animate's Filters section: Add,
/// Presets and an options menu (copy, paste, all on or off, remove all), then each filter with
/// its grip (drag, or Alt+arrows, to reorder), its on/off switch, its name and Remove, and its
/// values. A filter switched off keeps its values. Value changes replace the list
/// (`element.set_filters`, one undo step per gesture); the rest are `filter.*` commands.
fn filter_rows(app: &mut App, ui: &mut egui::Ui, current: &[anicraft_engine::doc::FilterEntry], differs: bool) {
    use anicraft_engine::doc::{Filter, FilterEntry};
    // As the renderer sees them (a file may hold values or ramps a command would refuse).
    let mut list: Vec<FilterEntry> = current.iter().map(FilterEntry::sanitized).collect();
    let n = list.len();
    let mut changed: Option<&'static str> = None;
    let mut act: Option<FilterAct> = None;
    let presets: Vec<(String, String)> = app.s.filter_presets.presets().iter().map(|p| (p.name.clone(), p.filters.iter().map(|f| crate::i18n::t(f.filter.label())).collect::<Vec<_>>().join(", "))).collect();
    let copied = app.s.filter_clipboard.as_ref().map_or(0, Vec::len);
    let mut naming = app.filter_naming.name.clone();
    row_prose(ui, tr!("Filters"), |ui| {
        egui::ComboBox::from_id_salt("add_filter").selected_text(if list.is_empty() { tr!("None") } else { tr!("Add filter") }).show_ui(ui, |ui| {
            for (kind, label) in Filter::KINDS {
                if ui.selectable_label(false, crate::i18n::t(label)).clicked()
                    && let Some(f) = Filter::new(kind)
                {
                    list.push(f.into());
                    changed = Some(tr_noop!("Add filter"));
                }
            }
        })
        .response
        .set_name(tr!("Add filter"));
        let pr = crate::timeline::icon_button(ui, tr!("Filter presets: apply, save, rename, delete, import, export"), None, Icon::Folder);
        egui::Popup::menu(&pr).id(egui::Id::new("anicraft.filters.presets")).show(|ui| {
            if presets.is_empty() {
                ui.label(text(Type::Caption, tr!("No presets yet. Set up filters, then Save as Preset.")));
            }
            for (name, what) in &presets {
                if ui.button(name.as_str()).on_hover_text(tr!("Apply: {filters} (replaces the filters)", filters = what.as_str())).clicked() {
                    act = Some(FilterAct::Apply(name.clone()));
                }
            }
            ui.separator();
            if ui.add_enabled(n > 0, egui::Button::new(tr!("Save as Preset…"))).on_hover_text(tr!("Keep these filters under a name, for any document")).clicked() {
                naming = Some((String::new(), None));
            }
            ui.add_enabled_ui(!presets.is_empty(), |ui| {
                ui.menu_button(tr!("Rename Preset"), |ui| {
                    for (name, _) in &presets {
                        if ui.button(name.as_str()).clicked() {
                            naming = Some((name.clone(), Some(name.clone())));
                        }
                    }
                });
                ui.menu_button(tr!("Delete Preset"), |ui| {
                    for (name, _) in &presets {
                        if ui.button(name.as_str()).clicked() {
                            act = Some(FilterAct::Delete(name.clone()));
                        }
                    }
                });
            });
            ui.separator();
            if ui.button(tr!("Import Presets…")).on_hover_text(tr!("Add the presets in a filter preset file (.json)")).clicked() {
                app.invoke("filter.preset_import");
            }
            if ui.add_enabled(!presets.is_empty(), egui::Button::new(tr!("Export Presets…"))).on_hover_text(tr!("Write your filter presets to a file, to use on another computer")).clicked() {
                app.invoke("filter.preset_export");
            }
        });
        let more = crate::timeline::icon_button(ui, tr!("Filter options: copy, paste, enable, disable, remove"), None, Icon::Ellipsis);
        egui::Popup::menu(&more).id(egui::Id::new("anicraft.filters.menu")).show(|ui| {
            if ui.add_enabled(n > 0, egui::Button::new(tr!("Copy All Filters"))).clicked() {
                act = Some(FilterAct::Copy(None));
            }
            let paste = if copied == 0 { tr!("Paste Filters").to_string() } else { trn!("Paste {n} Filter", "Paste {n} Filters", copied as u64, n = copied) };
            if ui.add_enabled(copied > 0, egui::Button::new(paste)).on_hover_text(tr!("Add the copied filters after these")).clicked() {
                act = Some(FilterAct::Paste);
            }
            ui.separator();
            if ui.add_enabled(list.iter().any(|f| !f.enabled), egui::Button::new(tr!("Enable All"))).clicked() {
                act = Some(FilterAct::Enable(None, true));
            }
            if ui.add_enabled(list.iter().any(|f| f.enabled), egui::Button::new(tr!("Disable All"))).clicked() {
                act = Some(FilterAct::Enable(None, false));
            }
            if ui.add_enabled(n > 0, egui::Button::new(tr!("Remove All"))).clicked() {
                act = Some(FilterAct::Remove(None));
            }
        });
        mixed(ui, differs);
    });
    // Naming a preset, inline: Enter saves, Escape cancels.
    if let Some((buf, renaming)) = naming.as_mut() {
        let mut done: Option<bool> = None;
        let rename = renaming.is_some();
        row_prose(ui, if rename { tr!("Rename") } else { tr!("Preset") }, |ui| {
            let r = ui.add(egui::TextEdit::singleline(buf).desired_width(120.0).hint_text(tr!("Name"))).named(tr!("Filter preset name"));
            if !r.has_focus() && !r.lost_focus() && buf.is_empty() {
                r.request_focus();
            }
            let (enter, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
            let ok = !buf.trim().is_empty();
            if ui.add_enabled(ok, egui::Button::new(if rename { tr!("Rename") } else { tr!("Save") })).clicked() || (enter && r.lost_focus() && ok) {
                done = Some(true);
            } else if ui.button(tr!("Cancel")).clicked() || esc {
                done = Some(false);
            }
        });
        match done {
            Some(true) => {
                let to = buf.trim().to_string();
                let ok = match renaming.clone() {
                    Some(from) => app.run("filter.preset_rename", json!({"name": from, "to": to})).is_some(),
                    None => {
                        // Asks first when it would replace a preset of that name.
                        app.request("filter.preset_save", json!({"name": to}));
                        true
                    }
                };
                if ok {
                    naming = None;
                }
            }
            Some(false) => naming = None,
            None => {}
        }
    }
    app.filter_naming.name = naming;
    let quality_names = [tr!("Low"), tr!("Medium"), tr!("High")];
    // Dragging a filter by its grip: which one, and where each header row starts (last frame's,
    // for the drop: rows don't move while one is dragged).
    let drag_id = egui::Id::new("anicraft.filters.drag");
    let mut dragging: Option<usize> = ui.data(|d| d.get_temp(drag_id));
    let last_tops: Vec<f32> = ui.data(|d| d.get_temp(drag_id.with("tops"))).unwrap_or_default();
    let drop_at = |y: f32| last_tops.iter().take(n).rposition(|t| y >= *t).unwrap_or(0);
    let mut tops: Vec<f32> = Vec::with_capacity(n + 1);
    let (list_left, list_w) = (ui.cursor().left(), ui.available_width());
    for (i, entry) in list.iter_mut().enumerate() {
        // Screen readers hear which filter a value belongs to: "Drop Shadow blur X".
        let fname = crate::i18n::t(entry.filter.label());
        let on = entry.enabled;
        ui.push_id(("filter", i), |ui| {
            tops.push(ui.cursor().top());
            let header = ui.horizontal(|ui| {
                let grip = filter_grip(ui, fname, i, n, &mut act);
                let tip = if on { tr!("Turn {filter} off (it keeps its settings)", filter = fname) } else { tr!("Turn {filter} on", filter = fname) };
                if crate::timeline::icon_button(ui, &tip, Some(on), if on { Icon::Eye } else { Icon::EyeOff }).clicked() {
                    act = Some(FilterAct::Enable(Some(i), !on));
                }
                let name = if on { text(Type::Label, fname) } else { text(Type::Label, tr!("{filter} (off)", filter = fname)).color(T.text_faint) };
                ui.label(name);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if crate::timeline::icon_button(ui, &tr!("Remove {filter}", filter = fname), None, Icon::Trash).clicked() {
                        act = Some(FilterAct::Remove(Some(i)));
                    }
                });
                grip
            });
            let grip = header.inner;
            if grip.drag_started() {
                dragging = Some(i);
            }
            if grip.drag_stopped() && dragging == Some(i) {
                dragging = None;
                if let Some(p) = ui.ctx().pointer_interact_pos()
                    && drop_at(p.y) != i
                {
                    act = Some(FilterAct::Move(i, drop_at(p.y)));
                }
            }
            header.response.context_menu(|ui| {
                if ui.add_enabled(i > 0, egui::Button::new(tr!("Move Up"))).clicked() {
                    act = Some(FilterAct::Move(i, i - 1));
                }
                if ui.add_enabled(i + 1 < n, egui::Button::new(tr!("Move Down"))).clicked() {
                    act = Some(FilterAct::Move(i, i + 1));
                }
                ui.separator();
                if ui.button(if on { tr!("Turn Off") } else { tr!("Turn On") }).clicked() {
                    act = Some(FilterAct::Enable(Some(i), !on));
                }
                if ui.button(tr!("Copy Filter")).clicked() {
                    act = Some(FilterAct::Copy(Some(i)));
                }
                if ui.button(tr!("Remove Filter")).clicked() {
                    act = Some(FilterAct::Remove(Some(i)));
                }
            });
            let f = &mut entry.filter;
            let blur_rows = |ui: &mut egui::Ui, bx: &mut f32, by: &mut f32, quality: &mut u8, changed: &mut Option<&'static str>| {
                row(ui, tr!("Blur"), |ui| {
                    let a = ui.add(egui::DragValue::new(bx).prefix("X ").suffix(" px").speed(0.25).range(0.0..=255.0).max_decimals(1)).named(&tr!("{filter} blur X", filter = fname));
                    let b = ui.add(egui::DragValue::new(by).prefix("Y ").suffix(" px").speed(0.25).range(0.0..=255.0).max_decimals(1)).named(&tr!("{filter} blur Y", filter = fname));
                    if a.changed() || b.changed() {
                        *changed = Some(tr_noop!("Filter blur"));
                    }
                });
                row_prose(ui, tr!("Quality"), |ui| {
                    for (q, name) in quality_names.iter().enumerate() {
                        if ui.selectable_label(usize::from(*quality) == q + 1, *name).named(&tr!("{filter} quality {quality}", filter = fname, quality = name)).clicked() {
                            *quality = q as u8 + 1;
                            *changed = Some(tr_noop!("Filter quality"));
                        }
                    }
                });
            };
            let pin = anicraft_engine::doc::pinned_stop(f);
            match f {
                Filter::Blur { blur_x, blur_y, quality } => blur_rows(ui, blur_x, blur_y, quality, &mut changed),
                Filter::DropShadow { blur_x, blur_y, strength, quality, angle, distance, color, inner, knockout, hide_object } => {
                    blur_rows(ui, blur_x, blur_y, quality, &mut changed);
                    shadow_rows(ui, fname, strength, color, &mut changed);
                    offset_row(ui, fname, angle, distance, tr_noop!("Shadow offset"), &mut changed);
                    row_prose(ui, tr!("Options"), |ui| {
                        for (v, label) in [(inner, tr!("Inner")), (knockout, tr!("Knockout")), (hide_object, tr!("Hide object"))] {
                            if crate::theme::checkbox(ui, v, label).changed() {
                                changed = Some(tr_noop!("Shadow options"));
                            }
                        }
                    });
                }
                Filter::Glow { blur_x, blur_y, strength, quality, color, inner, knockout } => {
                    blur_rows(ui, blur_x, blur_y, quality, &mut changed);
                    shadow_rows(ui, fname, strength, color, &mut changed);
                    row_prose(ui, tr!("Options"), |ui| {
                        for (v, label) in [(inner, tr!("Inner")), (knockout, tr!("Knockout"))] {
                            if crate::theme::checkbox(ui, v, label).changed() {
                                changed = Some(tr_noop!("Glow options"));
                            }
                        }
                    });
                }
                Filter::Bevel { blur_x, blur_y, strength, quality, angle, distance, shadow_color, highlight_color, side, knockout } => {
                    blur_rows(ui, blur_x, blur_y, quality, &mut changed);
                    strength_row(ui, fname, strength, &mut changed);
                    row_prose(ui, tr!("Colours"), |ui| {
                        let colours = [
                            (shadow_color, tr!("Shadow"), tr!("{filter} shadow colour", filter = fname), tr_noop!("Bevel shadow colour")),
                            (highlight_color, tr!("Highlight"), tr!("{filter} highlight colour", filter = fname), tr_noop!("Bevel highlight colour")),
                        ];
                        for (c, label, name, undo) in colours {
                            ui.label(label);
                            if alpha_swatch(ui, c, &name) {
                                changed = Some(undo);
                            }
                        }
                    });
                    offset_row(ui, fname, angle, distance, tr_noop!("Bevel offset"), &mut changed);
                    side_rows(ui, fname, side, knockout, &mut changed);
                }
                Filter::GradientGlow { blur_x, blur_y, strength, quality, angle, distance, stops, side, knockout } | Filter::GradientBevel { blur_x, blur_y, strength, quality, angle, distance, stops, side, knockout } => {
                    // The ramp first: it is what makes the filter.
                    ramp_editor(ui, fname, stops, pin, &mut changed);
                    blur_rows(ui, blur_x, blur_y, quality, &mut changed);
                    strength_row(ui, fname, strength, &mut changed);
                    offset_row(ui, fname, angle, distance, tr_noop!("Filter offset"), &mut changed);
                    side_rows(ui, fname, side, knockout, &mut changed);
                }
                Filter::AdjustColor { brightness, contrast, saturation, hue } => {
                    for (v, label, range) in [(brightness, tr!("Brightness"), 100.0), (contrast, tr!("Contrast"), 100.0), (saturation, tr!("Saturation"), 100.0), (hue, tr!("Hue"), 180.0)] {
                        row(ui, label, |ui| {
                            if a11y::slider(ui, v, -range..=range, label, a11y::SliderLook { max_decimals: Some(0), ..Default::default() }).changed() {
                                changed = Some(tr_noop!("Adjust colour"));
                            }
                        });
                    }
                }
            }
        });
    }
    // Where the rows are, for the next frame's drop, and the drop mark while one is dragged:
    // a bar where it would land.
    tops.push(ui.cursor().top());
    ui.data_mut(|d| d.insert_temp(drag_id.with("tops"), tops.clone()));
    if let (Some(from), Some(p)) = (dragging, ui.ctx().pointer_interact_pos())
        && ui.ctx().dragged_id().is_some()
    {
        let to = drop_at(p.y);
        if to != from
            && let Some(y) = tops.get(if to > from { to + 1 } else { to }).copied()
        {
            ui.painter().line_segment([pos2(list_left, y), pos2(list_left + list_w, y)], Stroke::new(2.0, T.accent));
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
    if let Some(a) = act {
        match a {
            FilterAct::Move(from, to) => {
                app.run("filter.move", json!({"from": from, "to": to}));
            }
            FilterAct::Enable(index, on) => {
                app.run("filter.enable", json!({"index": index, "enabled": on}));
            }
            FilterAct::Remove(index) => {
                app.run("filter.remove", json!({"index": index}));
            }
            FilterAct::Copy(index) => {
                if let Some(v) = app.run("filter.copy", json!({"index": index})) {
                    let k = v["copied"].as_u64().unwrap_or(0);
                    app.status_ok(trn!("Copied {n} filter.", "Copied {n} filters.", k, n = k));
                }
            }
            FilterAct::Paste => {
                app.run("filter.paste", json!({}));
            }
            FilterAct::Apply(name) => {
                app.run("filter.preset_apply", json!({"name": name}));
            }
            FilterAct::Delete(name) => app.request("filter.preset_delete", json!({"name": name})),
        }
        return;
    }
    if let Some(label) = changed
        && let Ok(v) = serde_json::to_value(&list)
    {
        live(app, label, "element.set_filters", json!({"filters": v}));
    }
}

/// Strength and colour rows of a drop shadow or glow.
fn shadow_rows(ui: &mut egui::Ui, fname: &str, strength: &mut f32, color: &mut Rgba, changed: &mut Option<&'static str>) {
    row(ui, tr!("Strength"), |ui| {
        let mut pct = *strength * 100.0;
        if ui.add(egui::DragValue::new(&mut pct).suffix(" %").speed(1.0).range(0.0..=1000.0).max_decimals(0)).named(&tr!("{filter} strength", filter = fname)).changed() {
            *strength = pct / 100.0;
            *changed = Some(tr_noop!("Filter strength"));
        }
        let mut c = rgb(*color);
        if crate::colour::button(ui, &mut c).named(&tr!("{filter} colour", filter = fname)).on_hover_text(tr!("Colour")).changed() {
            *color = Rgba::new(f32::from(c[0]) / 255.0, f32::from(c[1]) / 255.0, f32::from(c[2]) / 255.0, color.a);
            *changed = Some(tr_noop!("Filter colour"));
        }
    });
}

/// The Strength row of a bevel or gradient filter.
fn strength_row(ui: &mut egui::Ui, fname: &str, strength: &mut f32, changed: &mut Option<&'static str>) {
    row(ui, tr!("Strength"), |ui| {
        let mut pct = *strength * 100.0;
        if ui.add(egui::DragValue::new(&mut pct).suffix(" %").speed(1.0).range(0.0..=1000.0).max_decimals(0)).named(&tr!("{filter} strength", filter = fname)).changed() {
            *strength = pct / 100.0;
            *changed = Some(tr_noop!("Filter strength"));
        }
    });
}

/// Angle and distance of a shadow, bevel or gradient filter.
fn offset_row(ui: &mut egui::Ui, fname: &str, angle: &mut f32, distance: &mut f32, undo: &'static str, changed: &mut Option<&'static str>) {
    row(ui, tr!("Offset"), |ui| {
        let a = ui
            .add(egui::DragValue::new(angle).prefix(format!("{} ", tr!("Angle"))).suffix("°").speed(1.0).range(0.0..=360.0).max_decimals(0))
            .named(&tr!("{filter} angle", filter = fname))
            .on_hover_text(tr!("The direction the shadow falls (the light comes from the opposite side)"));
        let b = ui.add(egui::DragValue::new(distance).prefix(format!("{} ", tr!("Distance"))).suffix(" px").speed(0.25).range(-1000.0..=1000.0).max_decimals(1)).named(&tr!("{filter} distance", filter = fname));
        if a.changed() || b.changed() {
            *changed = Some(undo);
        }
    });
}

/// The Type (inner, outer, full) and Knockout rows of a bevel or gradient filter.
fn side_rows(ui: &mut egui::Ui, fname: &str, side: &mut anicraft_engine::doc::FilterSide, knockout: &mut bool, changed: &mut Option<&'static str>) {
    use anicraft_engine::doc::FilterSide;
    row_prose(ui, tr!("Type"), |ui| {
        for (v, tip) in [(FilterSide::Inner, tr!("Inside the artwork's edge")), (FilterSide::Outer, tr!("Outside the artwork's edge")), (FilterSide::Full, tr!("Inside and outside the edge"))] {
            let label = crate::i18n::t(v.label());
            if ui.selectable_label(*side == v, label).named(&tr!("{filter} type {kind}", filter = fname, kind = label)).on_hover_text(tip).clicked() && *side != v {
                *side = v;
                *changed = Some(tr_noop!("Filter type"));
            }
        }
    });
    row_prose(ui, tr!("Options"), |ui| {
        if crate::theme::checkbox(ui, knockout, tr!("Knockout")).named(&tr!("{filter} knockout", filter = fname)).on_hover_text(tr!("Show only the effect, not the artwork")).changed() {
            *changed = Some(tr_noop!("Filter options"));
        }
    });
}

/// A colour swatch with opacity (Animate's bevel colours carry alpha). True when changed.
pub(crate) fn alpha_swatch(ui: &mut egui::Ui, c: &mut Rgba, name: &str) -> bool {
    let s = c.sanitized();
    let b = |v: f32| (v * 255.0).round() as u8;
    let mut px = [b(s.r), b(s.g), b(s.b), b(s.a)];
    let r = crate::colour::button_alpha(ui, &mut px).named(name).on_hover_text(name);
    if r.changed() {
        *c = Rgba::new(f32::from(px[0]) / 255.0, f32::from(px[1]) / 255.0, f32::from(px[2]) / 255.0, f32::from(px[3]) / 255.0);
    }
    r.changed()
}

/// A gradient filter's ramp, as Animate's: a strip of its colours over a checkerboard, a
/// marker under it per stop. Click a marker to pick that stop (its colour, position and Remove
/// below), drag it to move it (between its neighbours), click the strip to add a stop there.
/// The pinned stop (Gradient Glow's start, Gradient Bevel's middle) can't move or go, and
/// stays transparent.
fn ramp_editor(ui: &mut egui::Ui, fname: &str, stops: &mut Vec<anicraft_engine::doc::GradientStop>, pin: Option<f32>, changed: &mut Option<&'static str>) {
    use anicraft_engine::doc::{GradientStop, MAX_RAMP_STOPS, REST_SNAP, ramp_at};
    let sel_id = ui.id().with("ramp_stop");
    let mut sel: usize = ui.data(|d| d.get_temp(sel_id)).unwrap_or(0);
    let is_pinned = |s: &GradientStop| pin.is_some_and(|p| (s.offset - p).abs() < 1e-4);
    // Where each stop may move (the ramp is in order): between its neighbours, and clear of the
    // pinned stop, so stops keep their order (and the pick, and a drag, stay on the same stop).
    let clear = 2.0 * REST_SNAP;
    let room: Vec<(f32, f32)> = (0..stops.len())
        .map(|i| {
            let prev = i.checked_sub(1).and_then(|j| stops.get(j));
            let next = stops.get(i + 1);
            let lo = prev.map_or(0.0, |s| s.offset + if is_pinned(s) { clear } else { 0.0 });
            let hi = next.map_or(1.0, |s| s.offset - if is_pinned(s) { clear } else { 0.0 });
            let here = stops.get(i).map_or(0.0, |s| s.offset);
            if lo <= hi { (lo, hi) } else { (here, here) }
        })
        .collect();
    row_prose(ui, tr!("Gradient"), |ui| {
        ui.vertical(|ui| {
            let w = ui.available_width().clamp(80.0, 200.0);
            let (strip, resp) = ui.allocate_exact_size(vec2(w, 16.0), Sense::click());
            a11y::control(ui, &resp, WidgetType::Button, &tr!("{filter} gradient: add a colour stop", filter = fname), None);
            let resp = resp.on_hover_text(tr!("Click to add a colour stop"));
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
            let steps = 48u32;
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
            // Markers: a small pointer per stop, under the strip.
            let (track, _) = ui.allocate_exact_size(vec2(w, 12.0), Sense::hover());
            for (i, st) in stops.iter_mut().enumerate() {
                let x = strip.left() + st.offset * strip.width();
                let hit = Rect::from_center_size(pos2(x, track.center().y), vec2(10.0, 12.0));
                let pinned = is_pinned(st);
                let r = ui.interact(hit, ui.id().with(("ramp_marker", i)), if pinned { Sense::click() } else { Sense::click_and_drag() });
                let at = format!("{:.0}", st.offset * 100.0);
                let label = if pinned { tr!("{filter} colour stop {stop} at {at}% (fixed)", filter = fname, stop = i + 1, at = at) } else { tr!("{filter} colour stop {stop} at {at}%", filter = fname, stop = i + 1, at = at) };
                a11y::control(ui, &r, WidgetType::Button, &label, Some(i == sel));
                if r.clicked() || r.drag_started() {
                    sel = i;
                }
                if r.dragged()
                    && !pinned
                    && let Some(at) = r.interact_pointer_pos()
                {
                    let (lo, hi) = room.get(i).copied().unwrap_or((st.offset, st.offset));
                    st.offset = ((at.x - strip.left()) / strip.width()).clamp(lo, hi);
                    sel = i;
                    *changed = Some(tr_noop!("Move colour stop"));
                }
                let tip = pos2(x, track.top());
                let body = [tip, pos2(x + 5.0, track.top() + 5.0), pos2(x + 5.0, track.bottom()), pos2(x - 5.0, track.bottom()), pos2(x - 5.0, track.top() + 5.0)];
                let edge = if i == sel { T.accent } else if r.hovered() { T.text } else { T.control_edge };
                ui.painter().add(egui::Shape::convex_polygon(body.to_vec(), to_color32(st.color.with_alpha(1.0)), Stroke::new(if i == sel { 2.0 } else { 1.0 }, edge)));
            }
            if resp.clicked() && stops.len() < MAX_RAMP_STOPS {
                // Where the pointer clicked; from the keyboard or a screen reader, the middle of
                // the widest gap. Never on the pinned stop.
                let gap = stops.windows(2).filter_map(|w| if let [a, b] = w { Some((b.offset - a.offset, (a.offset + b.offset) / 2.0)) } else { None }).max_by(|a, b| a.0.total_cmp(&b.0)).map_or(0.5, |g| g.1);
                let mut t = resp.interact_pointer_pos().map_or(gap, |at| ((at.x - strip.left()) / strip.width()).clamp(0.0, 1.0));
                if let Some(p) = pin
                    && (t - p).abs() < clear
                {
                    t = if t < p || p >= 1.0 { p - clear } else { p + clear }.clamp(0.0, 1.0);
                }
                let new = GradientStop { offset: t, color: ramp_at(stops, t).with_alpha(1.0) };
                let i = stops.iter().position(|s| s.offset > t).unwrap_or(stops.len());
                stops.insert(i, new);
                sel = i;
                *changed = Some(tr_noop!("Add colour stop"));
            }
        });
    });
    sel = sel.min(stops.len().saturating_sub(1));
    let mut remove = false;
    let count = stops.len();
    if let Some(st) = stops.get_mut(sel) {
        let pinned = is_pinned(st);
        row(ui, tr!("Stop"), |ui| {
            if pinned {
                // Colour only: this stop stays transparent.
                let mut c = rgb(st.color);
                if crate::colour::button(ui, &mut c).named(&tr!("{filter} stop colour", filter = fname)).on_hover_text(tr!("Stop colour (this stop stays transparent)")).changed() {
                    st.color = Rgba::new(f32::from(c[0]) / 255.0, f32::from(c[1]) / 255.0, f32::from(c[2]) / 255.0, 0.0);
                    *changed = Some(tr_noop!("Colour stop"));
                }
            } else if alpha_swatch(ui, &mut st.color, &tr!("{filter} stop colour", filter = fname)) {
                *changed = Some(tr_noop!("Colour stop"));
            }
            let mut pct = st.offset * 100.0;
            let r = ui.add_enabled(!pinned, egui::DragValue::new(&mut pct).suffix(" %").speed(0.5).range(0.0..=100.0).max_decimals(0)).named(&tr!("{filter} stop position", filter = fname));
            if r.changed() {
                let (lo, hi) = room.get(sel).copied().unwrap_or((st.offset, st.offset));
                st.offset = (pct / 100.0).clamp(lo, hi);
                *changed = Some(tr_noop!("Move colour stop"));
            }
            if ui.add_enabled(!pinned && count > 2, egui::Button::new(tr!("Remove")).small()).named(&tr!("Remove {filter} colour stop", filter = fname)).on_hover_text(tr!("Remove this colour stop")).clicked() {
                remove = true;
            }
        });
    }
    if remove && sel < stops.len() {
        stops.remove(sel);
        sel = sel.saturating_sub(1);
        *changed = Some(tr_noop!("Remove colour stop"));
    }
    ui.data_mut(|d| d.insert_temp(sel_id, sel));
}

/// Font, size, style, colour, alignment and spacing rows for text. Returns the edit made, as
/// `text.edit` params (one key per change), with an undo label.
pub fn text_style_rows(app: &mut App, ui: &mut egui::Ui, st: &mut anicraft_engine::cmds_text::TextStyle, width: Option<&mut Option<f32>>) -> Option<(&'static str, Value)> {
    use anicraft_engine::doc::TextAlign;
    let families = app.font_families.get_or_insert_with(|| anicraft_engine::text::Fonts::get().families().to_vec()).clone();
    let mut out: Option<(&'static str, Value)> = None;
    row_prose(ui, tr!("Font"), |ui| {
        let shown = if st.font.is_empty() { anicraft_engine::text::Fonts::get().resolve("").unwrap_or_default() } else { st.font.clone() };
        egui::ComboBox::from_id_salt("text_font").selected_text(shown.clone()).width(ui.available_width().min(220.0)).height(320.0).show_ui(ui, |ui| {
            if families.is_empty() {
                ui.label(text(Type::Caption, tr!("No fonts found on this computer")));
            }
            for f in &families {
                if ui.selectable_label(*f == shown, f).clicked() {
                    st.font = f.clone();
                    out = Some((tr_noop!("Font"), json!({"font": f})));
                }
            }
        })
        .response
        .set_name(tr!("Font"));
    });
    row(ui, tr!("Size"), |ui| {
        if ui.add(egui::DragValue::new(&mut st.size).suffix(" px").speed(0.25).range(1.0..=2000.0).max_decimals(1)).named(tr!("Font size")).changed() {
            out = Some((tr_noop!("Font size"), json!({"size": st.size})));
        }
        let mut c = rgb(st.color);
        if crate::colour::button(ui, &mut c).named(tr!("Text colour")).on_hover_text(tr!("Text colour")).changed() {
            st.color = Rgba::hex(u32::from(c[0]) << 16 | u32::from(c[1]) << 8 | u32::from(c[2]));
            out = Some((tr_noop!("Text colour"), json!({"color": hex(c)})));
        }
    });
    row_prose(ui, tr!("Style"), |ui| {
        // The letters on the two buttons are the usual bold and italic marks.
        if ui.selectable_label(st.bold, text(Type::Body, "B").strong()).named(tr!("Bold")).on_hover_text(tr!("Bold")).clicked() {
            st.bold = !st.bold;
            out = Some((tr_noop!("Bold"), json!({"bold": st.bold})));
        }
        if ui.selectable_label(st.italic, text(Type::Body, "I").italics()).named(tr!("Italic")).on_hover_text(tr!("Italic")).clicked() {
            st.italic = !st.italic;
            out = Some((tr_noop!("Italic"), json!({"italic": st.italic})));
        }
        ui.separator();
        for (a, label, name, key) in [(TextAlign::Left, tr!("Left"), tr!("Align left"), "left"), (TextAlign::Center, tr!("Centre"), tr!("Align centre"), "center"), (TextAlign::Right, tr!("Right"), tr!("Align right"), "right")] {
            if ui.selectable_label(st.align == a, label).named(name).on_hover_text(name).clicked() {
                st.align = a;
                out = Some((tr_noop!("Text alignment"), json!({"align": key})));
            }
        }
    });
    row(ui, tr!("Spacing"), |ui| {
        if ui.add(egui::DragValue::new(&mut st.letter_spacing).prefix(format!("{} ", tr!("Letters"))).suffix(" px").speed(0.1).range(-1000.0..=1000.0).max_decimals(1)).named(tr!("Letter spacing")).changed() {
            out = Some((tr_noop!("Letter spacing"), json!({"letter_spacing": st.letter_spacing})));
        }
        if ui.add(egui::DragValue::new(&mut st.line_spacing).prefix(format!("{} ", tr!("Lines"))).suffix(" px").speed(0.1).range(-1000.0..=1000.0).max_decimals(1)).named(tr!("Line spacing")).changed() {
            out = Some((tr_noop!("Line spacing"), json!({"line_spacing": st.line_spacing})));
        }
    });
    if let Some(width) = width {
        row(ui, tr!("Width"), |ui| {
            let mut wrap = width.is_some();
            if crate::theme::checkbox(ui, &mut wrap, tr!("Wrap")).on_hover_text(tr!("Wrap lines at a fixed width; off, each paragraph stays on one line")).changed() {
                *width = wrap.then_some(300.0);
                out = Some((tr_noop!("Text width"), json!({"width": *width})));
            }
            if let Some(w) = width.as_mut()
                && ui.add(egui::DragValue::new(w).suffix(" px").speed(1.0).range(1.0..=100_000.0).max_decimals(0)).named(tr!("Text width")).changed()
            {
                out = Some((tr_noop!("Text width"), json!({"width": *w})));
            }
        });
    }
    out
}

/// The camera at the playhead: where it looks, its zoom, turn and colour effect, and the
/// preview switch.
/// Edits key the playhead like any edit of a tweened object.
fn camera_props(app: &mut App, ui: &mut egui::Ui) {
    let Some(cam) = crate::camera_tool::camera(app) else { return };
    let (mut x, mut y) = (cam.center.x, cam.center.y);
    row(ui, tr!("Position"), |ui| {
        let a = ui.add(egui::DragValue::new(&mut x).prefix("x ").suffix(" px").speed(1.0).max_decimals(1)).named(tr!("Camera X"));
        let b = ui.add(egui::DragValue::new(&mut y).prefix("y ").suffix(" px").speed(1.0).max_decimals(1)).named(tr!("Camera Y"));
        if a.changed() || b.changed() {
            live(app, tr_noop!("Move Camera"), "camera.set", json!({"x": x, "y": y}));
        }
    });
    let mut zoom = cam.zoom * 100.0;
    let (lo, hi) = (anicraft_engine::doc::camera::MIN_ZOOM * 100.0, anicraft_engine::doc::camera::MAX_ZOOM * 100.0);
    row(ui, tr!("Zoom"), |ui| {
        ui.spacing_mut().slider_width = 96.0;
        if a11y::slider(ui, &mut zoom, lo..=hi, tr!("Camera zoom"), a11y::SliderLook { suffix: " %", max_decimals: Some(0), logarithmic: true, ..Default::default() }).changed() {
            live(app, tr_noop!("Zoom Camera"), "camera.set", json!({"zoom": zoom}));
        }
    });
    let mut rot = cam.rotation.to_degrees();
    row(ui, tr!("Rotation"), |ui| {
        if ui.add(egui::DragValue::new(&mut rot).suffix("°").speed(0.5).max_decimals(1)).named(tr!("Camera rotation")).changed() {
            live(app, tr_noop!("Rotate Camera"), "camera.set", json!({"rotation": rot}));
        }
    });
    // Brightness, Tint or Advanced over the whole view, as on an instance (ADR 0082).
    colour_effect_rows(app, ui, &cam.color, crate::mixed::ColorMixed::default(), EffectOf::Camera);
    ui.horizontal_wrapped(|ui| {
        let mut on = app.camera_preview;
        if crate::theme::checkbox(ui, &mut on, tr!("Preview through camera")).on_hover_text(tr!("Show the stage as the movie shows it (View › Preview Through Camera)")).changed() {
            app.toggle_camera_preview();
        }
        if ui.button(tr!("Reset")).named(tr!("Reset camera")).on_hover_text(tr!("Back to the stage as it is: centred, 100 %, no turn, no colour effect")).clicked() {
            app.run("camera.set", json!({"reset": true}));
        }
    });
    let hidden = app.s.timeline().camera_layer().is_some_and(|l| !l.visible);
    ui.label(text(Type::Caption, if hidden { tr!("The camera layer is hidden: the movie shows the stage as it is.") } else { tr!("The movie shows what the frame on the stage shows. Deeper layers (Window › Layer Depth) move and zoom less.") }));
}

/// The selected bone: its joint angle at the playhead (editing it keys the pose), the joint's
/// limits, the pin, and removal.
/// A warped shape or bitmap (ADR 0079): its mesh's resolution, its pins, and taking the warp
/// off. Without one, how to start.
fn warp_props(app: &mut App, ui: &mut egui::Ui, target: Option<anicraft_engine::doc::ElementId>) {
    let frame = app.s.frame;
    let warp = target.and_then(|id| {
        app.s.timeline().layers.iter().find_map(|l| {
            let (_, els) = anicraft_engine::doc::evaluate_layer_elements(l, frame)?;
            els.iter().find(|e| e.id == id).and_then(|e| match &e.kind {
                ElementKind::Shape(sh) => sh.live_warp().cloned(),
                _ => None,
            })
        })
    });
    let (Some(id), Some(warp)) = (target, warp) else {
        ui.label(text(Type::Caption, tr!("Click a shape or a bitmap with the Asset Warp tool to pin it; drag a pin to bend it around the others.")));
        return;
    };
    let mut res = warp.resolution;
    let (cols, rows) = warp.grid().map_or((0, 0), |g| (g.nx, g.ny));
    row(ui, tr!("Mesh"), |ui| {
        let r = ui
            .add(egui::DragValue::new(&mut res).range(anicraft_engine::doc::warp::MIN_RESOLUTION..=anicraft_engine::doc::warp::MAX_RESOLUTION).speed(0.1))
            .named(tr!("Mesh resolution"))
            .on_hover_text(tr!("Cells along the artwork's longer side: more bends more smoothly, fewer is faster"));
        if r.changed() {
            live(app, tr_noop!("Warp Mesh Resolution"), "warp.add", json!({"element": id.0, "resolution": res}));
        }
        ui.label(text(Type::Data, format!("{cols} × {rows}")));
    });
    let pins = warp.pins.len();
    row(ui, tr!("Pins"), |ui| {
        ui.label(text(Type::Data, pins.to_string()));
    });
    ui.horizontal_wrapped(|ui| {
        if let Some((el, pin)) = app.warp_pin.filter(|(el, _)| *el == id)
            && ui.button(tr!("Remove Pin")).named(tr!("Remove warp pin")).on_hover_text(tr!("Removes the picked pin from every keyframe (Delete)")).clicked()
        {
            app.run("warp.pin_remove", json!({"element": el.0, "pin": pin.0}));
            app.warp_pin = None;
        }
        if ui.button(tr!("Remove Warp")).named(tr!("Remove warp")).on_hover_text(tr!("Takes the mesh and its pins off the artwork on every keyframe: it shows as drawn again")).clicked() {
            app.run("warp.remove", json!({"element": id.0}));
            app.warp_pin = None;
        }
    });
    ui.label(text(Type::Caption, tr!("Drag pins with the Asset Warp tool to bend the artwork; a pin moved at another frame keys it, and pins tween.")));
}

fn bone_props(app: &mut App, ui: &mut egui::Ui, id: anicraft_engine::doc::BoneId) {
    let tl = app.s.timeline();
    let Some((layer, bone)) = tl.layers.iter().find_map(|l| l.armature.as_ref().and_then(|a| a.bone(id)).map(|b| (l.clone(), b.clone()))) else { return };
    let pose = anicraft_engine::doc::armature::pose_at(&layer, app.s.frame).unwrap_or_default();
    let mut angle = bone.allow(pose.angle(id)).to_degrees();
    row(ui, tr!("Angle"), |ui| {
        if ui.add(egui::DragValue::new(&mut angle).suffix("°").speed(0.5).max_decimals(1)).named(tr!("Bone angle")).on_hover_text(tr!("The joint's turn from where the bone was drawn, measured from the bone it hangs from")).changed() {
            live(app, tr_noop!("Pose Bone"), "bone.pose", json!({"bone": id.0, "angle": angle}));
        }
    });
    let mut constrain = bone.limits.is_some();
    let mut pinned = bone.pinned;
    ui.horizontal_wrapped(|ui| {
        if crate::theme::checkbox(ui, &mut constrain, tr!("Constrain rotation")).on_hover_text(tr!("Limit how far the joint turns, in every pose")).changed() {
            app.run("bone.set_limits", json!({"bone": id.0, "constrain": constrain}));
        }
        if crate::theme::checkbox(ui, &mut pinned, tr!("Pin")).on_hover_text(tr!("A pinned bone, and the bones it hangs from, stay put while you drag the bones below it")).changed() {
            app.run("bone.pin", json!({"bone": id.0, "pinned": pinned}));
        }
    });
    if let Some(l) = bone.limits {
        let (mut min, mut max) = (l.min.to_degrees(), l.max.to_degrees());
        row(ui, tr!("Limits"), |ui| {
            let a = ui.add(egui::DragValue::new(&mut min).prefix("min ").suffix("°").speed(0.5).range(-360.0..=max).max_decimals(0)).named(tr!("Joint minimum angle"));
            let b = ui.add(egui::DragValue::new(&mut max).prefix("max ").suffix("°").speed(0.5).range(min..=360.0).max_decimals(0)).named(tr!("Joint maximum angle"));
            if a.changed() || b.changed() {
                live(app, tr_noop!("Joint Limits"), "bone.set_limits", json!({"bone": id.0, "min": min, "max": max}));
            }
        });
    }
    // Joint translation (ADR 0084): the joint may slide along X and Y, within limits.
    let slide = bone.allow_slide(pose.slide(id));
    for (axis, limits, now) in [("x", bone.slide_x, slide.x), ("y", bone.slide_y, slide.y)] {
        let (label, name, tip) = if axis == "x" {
            (tr!("X translation"), tr!("Joint X translation"), tr!("Lets the joint slide along the x axis of what the bone starts on, so the bone can stretch away from it. Posing slides it only where turning can't reach"))
        } else {
            (tr!("Y translation"), tr!("Joint Y translation"), tr!("Lets the joint slide along the y axis of what the bone starts on, so the bone can stretch away from it. Posing slides it only where turning can't reach"))
        };
        let mut on = limits.is_some();
        let free = anicraft_engine::doc::Limits { min: -anicraft_engine::doc::armature::MAX_SLIDE, max: anicraft_engine::doc::armature::MAX_SLIDE };
        let mut constrain = limits.is_some_and(|l| l != free);
        row_prose(ui, label, |ui| {
            if crate::theme::checkbox(ui, &mut on, tr!("Enable")).named(name).on_hover_text(tip).changed() {
                app.run("bone.set_translation", json!({"bone": id.0, "axis": axis, "enabled": on}));
            }
            if on && crate::theme::checkbox(ui, &mut constrain, tr!("Constrain")).on_hover_text(tr!("Limit how far the joint slides, in every pose")).changed() {
                app.run("bone.set_translation", json!({"bone": id.0, "axis": axis, "constrain": constrain}));
            }
        });
        if let Some(l) = limits {
            let mut v = now;
            row(ui, if axis == "x" { tr!("Slide X") } else { tr!("Slide Y") }, |ui| {
                let r = ui.add(egui::DragValue::new(&mut v).suffix(" px").speed(0.5).range(l.min..=l.max).max_decimals(1)).named(if axis == "x" { tr!("Joint slide X") } else { tr!("Joint slide Y") });
                if r.on_hover_text(tr!("How far the joint has slid at the playhead, from where it was drawn")).changed() {
                    live(app, tr_noop!("Pose Bone"), "bone.pose", json!({"bone": id.0, if axis == "x" { "slide_x" } else { "slide_y" }: v}));
                }
            });
            if l != free {
                row(ui, if axis == "x" { tr!("Limits X") } else { tr!("Limits Y") }, |ui| {
                    let (mut min, mut max) = (l.min, l.max);
                    let a = ui.add(egui::DragValue::new(&mut min).prefix("min ").suffix(" px").speed(0.5).range(-anicraft_engine::doc::armature::MAX_SLIDE..=max).max_decimals(0)).named(if axis == "x" { tr!("Joint X translation minimum") } else { tr!("Joint Y translation minimum") });
                    let b = ui.add(egui::DragValue::new(&mut max).prefix("max ").suffix(" px").speed(0.5).range(min..=anicraft_engine::doc::armature::MAX_SLIDE).max_decimals(0)).named(if axis == "x" { tr!("Joint X translation maximum") } else { tr!("Joint Y translation maximum") });
                    if a.changed() || b.changed() {
                        live(app, tr_noop!("Joint Translation"), "bone.set_translation", json!({"bone": id.0, "axis": axis, "min": min, "max": max}));
                    }
                });
            }
        }
    }
    // Spring (ADR 0084): secondary motion as the bones it hangs from move.
    let sp = bone.spring.unwrap_or(anicraft_engine::doc::armature::Spring { strength: 0.0, damping: 0.0 });
    let (mut strength, mut damping) = (sp.strength * 100.0, sp.damping * 100.0);
    row(ui, tr!("Spring"), |ui| {
        let r = ui.add(egui::DragValue::new(&mut strength).speed(0.5).range(0.0..=100.0).max_decimals(0)).named(tr!("Spring strength"));
        if r.on_hover_text(tr!("How stiff the bone's spring is, 0 to 100: 0 is no spring; a stiffer spring swings faster and lags less")).changed() {
            live(app, tr_noop!("Bone Spring"), "bone.set_spring", json!({"bone": id.0, "strength": strength}));
        }
    });
    row(ui, tr!("Damping"), |ui| {
        let r = ui.add(egui::DragValue::new(&mut damping).speed(0.5).range(0.0..=100.0).max_decimals(0)).named(tr!("Spring damping"));
        if r.on_hover_text(tr!("How quickly the spring's swing dies down, 0 to 100: 0 swings on, 100 absorbs it all")).changed() {
            live(app, tr_noop!("Bone Spring"), "bone.set_spring", json!({"bone": id.0, "damping": damping}));
        }
    });
    let mut springs = layer.armature.as_ref().is_some_and(|a| !a.springs_off);
    row_prose(ui, tr!("Springs"), |ui| {
        if crate::theme::checkbox(ui, &mut springs, tr!("Enable")).named(tr!("Enable springs")).on_hover_text(tr!("Turn off to see and pose the armature's keyed poses without the springs' swing; every export plays them on")).changed() {
            app.run("bone.enable_springs", json!({"layer": layer.id.0, "enabled": springs}));
        }
    });
    // A bone inside a shape (ADR 0071): how many of the shape's points it moves.
    let skin = layer.armature.as_ref().and_then(|a| a.skin.as_ref()).map(|k| (k.points.iter().filter(|p| p.weight(id) > 0.0).count(), k.points.len()));
    if let Some((mine, total)) = skin {
        row(ui, tr!("Points"), |ui| {
            ui.label(text(Type::Data, format!("{mine} / {total}"))).on_hover_text(tr!("How many of the shape's points this bone moves. Adjust them with the Bind tool"));
        });
        bind_props(app, ui, id, &layer, total);
    }
    ui.horizontal_wrapped(|ui| {
        if skin.is_some() {
            if ui.button(tr!("Bind Tool")).named(tr!("Bind tool")).on_hover_text(tr!("Choose which points of the shape follow this bone")).clicked() {
                app.tool = crate::app::Tool::Bind;
            }
            if ui.button(tr!("Rebind All")).named(tr!("Rebind all points")).on_hover_text(tr!("Binds every point of the shape to its nearest bones again, dropping changes made with the Bind tool")).clicked() {
                app.run("bone.bind", json!({"bone": id.0, "mode": "auto"}));
            }
        }
        let hint = if skin.is_some() { tr!("Removes the bone and the bones below it; the shape's points follow the bones that are left") } else { tr!("Removes the bone and the bones below it; their instances stay where they are, on a layer of their own") };
        if ui.button(tr!("Remove Bone")).named(tr!("Remove bone")).on_hover_text(hint).clicked() {
            app.request("bone.remove", json!({"bone": id.0}));
        }
    });
    ui.label(text(Type::Caption, if skin.is_some() { tr!("Drag bones with the Selection tool to bend the shape; a pose at another frame keys it, and poses tween.") } else { tr!("Drag bones or instances with the Selection tool to pose; a pose at another frame keys it, and poses tween.") }));
}

/// A shape bone's weights (ADR 0090): with the Bind tool, Paint Weights and its brush; always, the
/// focused point's share of the bone as a number (keyboard users edit weights here) and the bones
/// the point follows.
fn bind_props(app: &mut App, ui: &mut egui::Ui, id: anicraft_engine::doc::BoneId, layer: &anicraft_engine::doc::Layer, total: usize) {
    use crate::bind_tool::{RADIUS_MAX, RADIUS_MIN, brush_label};
    use anicraft_engine::doc::weights::Brush;
    if app.tool == Tool::Bind {
        let keys = [app.keymap.primary("view.brush_smaller"), app.keymap.primary("view.brush_larger")].into_iter().flatten().map(crate::platform::shortcut_label).collect::<Vec<_>>().join(" / ");
        let o = &mut app.bind;
        row_prose(ui, tr!("Weights"), |ui| {
            crate::theme::checkbox(ui, &mut o.paint, tr!("Paint Weights")).on_hover_text(tr!("Drag over the shape to brush this bone's influence onto its points, shown as a heat map: blue none, red all. Off: click or box points to bind them"));
        });
        if o.paint {
            row(ui, tr!("Brush"), |ui| {
                for b in [Brush::Add, Brush::Subtract, Brush::Smooth] {
                    let (label, hint) = brush_label(b);
                    ui.selectable_value(&mut o.brush, b, crate::i18n::t(label)).on_hover_text(crate::i18n::t(hint));
                }
            });
            row(ui, tr!("Size"), |ui| {
                let r = ui.add(egui::DragValue::new(&mut o.radius).suffix(" px").speed(0.25).range(RADIUS_MIN..=RADIUS_MAX).max_decimals(1)).named(tr!("Weight brush size"));
                if !keys.is_empty() {
                    r.on_hover_text(tr!("The brush's radius on the stage ({keys})", keys = keys));
                }
            });
            row(ui, tr!("Strength"), |ui| {
                ui.add(egui::DragValue::new(&mut o.strength).speed(0.5).range(1.0..=100.0).max_decimals(0)).named(tr!("Weight brush strength")).on_hover_text(tr!("How much one dab changes the weights at the brush's centre, 1 to 100; it fades to nothing at the rim"));
            });
        }
    }
    if total == 0 {
        return;
    }
    let point = app.bind.point.unwrap_or(0).min(total - 1);
    let mut shown = point + 1;
    row(ui, tr!("Point"), |ui| {
        let r = ui.add(egui::DragValue::new(&mut shown).speed(0.1).range(1..=total)).named(tr!("Focused point")).on_hover_text(tr!("The point whose weights show below (ringed on the stage). Click one with the Bind tool, or type its number"));
        if r.changed() {
            app.bind.point = Some(shown.saturating_sub(1).min(total - 1));
        }
    });
    let arm = layer.armature.as_ref();
    let binding = arm.and_then(|a| a.skin.as_ref()).and_then(|k| k.points.get(point)).cloned().unwrap_or_default();
    let mut w = binding.weight(id) * 100.0;
    row(ui, tr!("Weight"), |ui| {
        let r = ui.add(egui::DragValue::new(&mut w).suffix(" %").speed(0.5).range(0.0..=100.0).max_decimals(1)).named(tr!("Bone weight at the point"));
        if r.on_hover_text(tr!("How much this bone moves the point, 0 to 100 %; the point's other bones share the rest")).changed() {
            app.bind.point = Some(point);
            live(app, tr_noop!("Set Point Weight"), "bone.set_weight", json!({"bone": id.0, "points": [point], "weight": w}));
        }
    });
    let name = |b: anicraft_engine::doc::BoneId| arm.and_then(|a| a.bone(b)).map(|x| x.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| format!("#{}", b.0));
    // Names and numbers are data; the note for an unbound point is prose.
    let follows = if binding.bones.is_empty() { text(Type::Body, tr!("No bone: the point stays where it was drawn")) } else { text(Type::Data, binding.bones.iter().map(|i| format!("{} {:.0} %", name(i.0), i.1 * 100.0)).collect::<Vec<_>>().join(" · ")) };
    row(ui, tr!("Follows"), |ui| {
        ui.label(follows).on_hover_text(tr!("The bones that move the point, and their shares"));
    });
}

fn document_props(app: &mut App, ui: &mut egui::Ui) {
    {
        let st = app.s.doc.stage;
        let (mut w, mut h, mut fps) = (st.width, st.height, st.fps);
        row(ui, tr!("Size"), |ui| {
            let a = ui.add(egui::DragValue::new(&mut w).suffix(" px").range(1..=8192)).named(tr!("Stage width"));
            let b = ui.add(egui::DragValue::new(&mut h).suffix(" px").range(1..=8192)).named(tr!("Stage height"));
            if a.changed() || b.changed() {
                live(app, tr_noop!("Stage size"), "doc.set_stage", json!({"width": w, "height": h}));
            }
        });
        row(ui, tr!("Frame rate"), |ui| {
            if ui.add(egui::DragValue::new(&mut fps).suffix(" fps").speed(0.1).range(1.0..=120.0).max_decimals(2)).named(tr!("Frame rate")).changed() {
                live(app, tr_noop!("Frame rate"), "doc.set_stage", json!({"fps": fps}));
            }
        });
        let mut bg = rgb(st.background);
        row(ui, tr!("Stage"), |ui| {
            if crate::colour::button(ui, &mut bg).named(tr!("Stage colour")).changed() {
                live(app, tr_noop!("Stage colour"), "doc.set_stage", json!({"background": hex(bg)}));
            }
            ui.label(text(Type::Data, trn!("{n} frame", "{n} frames", app.s.frame_count())).color(T.text_faint));
        });
        // The document class, in documents that use scripts (kept, not run; ADR 0013).
        if crate::actions_panel::scripted(&app.s.doc) {
            let current = app.s.doc.document_class.clone();
            row_prose(ui, tr!("Class"), |ui| {
                if let Some(v) = crate::actions_panel::class_field(ui, "prop_docclass", &current, tr!("Document class"), tr!("The class scripts run the main timeline as (the document class). Kept with the document; scripts don't run yet")) {
                    app.run("script.set_class", json!({"class": v}));
                }
            });
        }
    }
}

fn frame_props(app: &mut App, ui: &mut egui::Ui, layer: &anicraft_engine::doc::Layer, frame: u32) {
    if layer.kind == anicraft_engine::doc::LayerKind::Folder {
        let tl = app.s.timeline();
        let n = tl.layers.iter().position(|l| l.id == layer.id).map_or(0, |i| tl.group_range(i).len().saturating_sub(1));
        ui.label(text(Type::Label, trn!("Folder · {n} layer", "Folder · {n} layers", n)));
        ui.label(text(Type::Caption, tr!("Folders hold layers, not frames. Hiding or locking the folder hides or locks everything in it; move layers in under it with Up and Down.")));
        return;
    }
    // Layer depth matters once the scene has a camera (ADR 0082): deeper layers move and zoom
    // less, and draw behind nearer ones.
    if crate::layer_depth::has_depth(layer.kind) && app.s.timeline().camera_layer().is_some() && app.s.editing.is_empty() {
        crate::layer_depth::depth_row(app, ui, layer, frame);
    }
    {
        match layer.keyframe_at(frame) {
            Some(k) => {
                let n = evaluate_layer_elements(layer, frame).map_or(0, |(_, e)| e.len());
                // One sentence per count: the frames, then the elements, each with its plural.
                let frames = trn!("{n} frame", "{n} frames", k.duration);
                let elements = trn!("{n} element", "{n} elements", n);
                ui.label(text(Type::Label, tr!("Keyframe at {frame} · {frames} · {elements}", frame = k.start + 1, frames = frames, elements = elements)));
                if layer.kind == anicraft_engine::doc::LayerKind::Mask {
                    ui.label(text(Type::Caption, tr!("Mask layer: its fills reveal the masked layers below. Lock it to hide its outline.")));
                }
                let mut label = k.label.clone();
                row_prose(ui, tr!("Label"), |ui| {
                    if ui.add(egui::TextEdit::singleline(&mut label).desired_width(crate::theme::fit(ui, 150.0))).named(tr!("Frame label")).changed() {
                        live(app, tr_noop!("Frame label"), "timeline.set_label", json!({"label": label}));
                    }
                });
                sound_rows(app, ui, k.sound.clone());
                if layer.has_sounds() {
                    volume_row(app, ui, layer);
                }
                let key = app.keymap.primary("view.actions").map(crate::platform::shortcut_label).unwrap_or_default();
                row_prose(ui, tr!("Actions"), |ui| {
                    let what = k.script.as_ref().map_or_else(|| tr!("None").to_string(), |s| crate::i18n::t_dyn(&s.describe()).into_owned());
                    let short: String = what.chars().take(36).collect();
                    let short = if short.len() < what.len() { format!("{short}…") } else { short };
                    ui.label(text(Type::Body, short).color(if k.script.is_none() { T.text_faint } else { T.text })).on_hover_text(what);
                    if ui.small_button(tr!("Edit…")).on_hover_text(tr!("What runs when the playhead enters this keyframe ({key})", key = key)).clicked() {
                        app.actions.symbol = None;
                        crate::actions_panel::open(app, crate::actions_panel::Tab::Frame);
                    }
                });
                if let Some(m) = &k.motion {
                    // A motion tween: what is keyed, and the way to its curves.
                    row_prose(ui, tr!("Tween"), |ui| {
                        ui.label(text(Type::Body, tr!("Motion")));
                    });
                    let keyed: Vec<&str> = m.tracks.iter().map(|t| crate::i18n::t(t.property.label())).collect();
                    let n = m.key_frames().len();
                    ui.label(text(Type::Caption, if keyed.is_empty() { tr!("No property keyframes yet: move or change the object at a later frame to add some.").to_string() } else { trn!("{properties} · {n} key frame", "{properties} · {n} key frames", n, properties = keyed.join(", ")) }));
                    // The motion path (ADR 0030): bent on the stage, straightened here.
                    let segments = m.path.as_ref().map(|p| p.cmds.len().saturating_sub(1));
                    row_prose(ui, tr!("Path"), |ui| {
                        let what = match segments {
                            Some(n) => trn!("Curved · {n} segment", "Curved · {n} segments", n),
                            None => tr!("Straight").to_string(),
                        };
                        let how = tr!(
                            "Select the object to see its path on the stage. Drag the path to bend it and its ends to move them; {drag} the path or the object to move the whole path. With the Subselection tool (A), drag points and handles, double-click the path to add a point (or Alt-click it with the Pen), Delete removes points, and right-click makes them smooth or corners.",
                            drag = crate::platform::shortcut_label("Ctrl+Drag")
                        );
                        ui.label(text(Type::Body, what)).on_hover_text(how);
                        if segments.is_some() && ui.small_button(tr!("Straighten")).on_hover_text(tr!("Back to the straight path the X and Y curves trace")).named(tr!("Straighten motion path")).clicked() {
                            app.run("motion.clear_path", json!({}));
                        }
                    });
                    // Where the path starts; changing it moves the whole path (ADR 0038).
                    let base = m.target_index(&k.elements).and_then(|i| k.elements.get(i)).map(|e| anicraft_engine::doc::Props::of(e));
                    let start = m.path.clone().or_else(|| base.and_then(|b| m.path_from_tracks(&b))).and_then(|p| anicraft_engine::doc::motion::path_ends(&p)).map(|(a, _)| a);
                    if let Some(a) = start {
                        let (mut x, mut y) = (a.x, a.y);
                        row(ui, tr!("Path start"), |ui| {
                            let hint = tr!("Where the motion starts. Changing it moves the whole path, and the object with it; the timing stays.");
                            let rx = ui.add(egui::DragValue::new(&mut x).prefix("X ").speed(1.0).max_decimals(1)).named(tr!("Motion path start X")).on_hover_text(hint);
                            let ry = ui.add(egui::DragValue::new(&mut y).prefix("Y ").speed(1.0).max_decimals(1)).named(tr!("Motion path start Y")).on_hover_text(hint);
                            if rx.changed() || ry.changed() {
                                live(app, tr_noop!("Move Motion Path"), "motion.move_path", json!({"dx": x - a.x, "dy": y - a.y}));
                            }
                        });
                    }
                    // Points pinned to frames (ADR 0049), and the keyboard route to pinning at the
                    // playhead.
                    let pins = base.map(|b| m.pinned_anchors(&b)).unwrap_or_default();
                    if let (Some(&first), Some(&last)) = (pins.first(), pins.last()) {
                        let offset = frame.saturating_sub(k.start);
                        let n = pins.len().saturating_sub(2);
                        row_prose(ui, tr!("Pinned"), |ui| {
                            let how = tr!("Points of the motion path pinned to frames: the object passes each one exactly then, and the X and Y keyframes there are its place. The ends are always pinned. Moving the object on the stage at a frame pins a point there; with the Subselection tool (A), right-click a point to pin or unpin it.");
                            ui.label(text(Type::Body, trn!("{n} point", "{n} points", n))).on_hover_text(how);
                            match pins.iter().find(|q| q.frame == offset) {
                                Some(&q) if q != first && q != last => {
                                    let tip = tr!("Point {anchor} shapes the path only again; the X and Y keyframes of frame {frame} go", anchor = q.anchor, frame = frame + 1);
                                    if ui.small_button(tr!("Unpin")).on_hover_text(tip).named(tr!("Unpin the motion path point at this frame")).clicked() {
                                        app.run("motion.unpin_point", json!({"frame": frame}));
                                    }
                                }
                                Some(_) => {}
                                None if first.frame < offset && offset < last.frame => {
                                    let tip = tr!("Add a point where the object is on frame {frame}, pinned to it: nothing moves now, and the object passes the point then however the path is edited", frame = frame + 1);
                                    if ui.small_button(tr!("Pin Here")).on_hover_text(tip).named(tr!("Pin the motion path at this frame")).clicked() {
                                        app.run("motion.pin_point", json!({"frame": frame}));
                                    }
                                }
                                None => {}
                            }
                        });
                    }
                    let mut orient = m.orient;
                    if crate::theme::checkbox(ui, &mut orient, tr!("Orient to path")).on_hover_text(tr!("Turn the object as its motion path turns")).named(tr!("Orient to path")).changed() {
                        app.run("motion.orient", json!({"orient": orient}));
                    }
                    ui.horizontal(|ui| {
                        if ui.button(tr!("Motion Editor")).on_hover_text(tr!("Edit the property curves and eases")).clicked() {
                            app.show_panel(crate::dock::Panel::MotionEditor);
                        }
                        if ui.button(tr!("Remove Tween")).on_hover_text(tr!("The object stays as it is on the span's first frame")).clicked() {
                            app.run("timeline.remove_tween", json!({}));
                        }
                    });
                    return;
                }
                match k.tween.clone() {
                    Some(t) => {
                        row_prose(ui, tr!("Ease"), |ui| {
                            match crate::ease_editor::picker(app, ui, "prop_ease", &t.easing, 150.0, tr!("Easing")) {
                                Some(crate::ease_editor::Picked::Ease(e)) => {
                                    app.run("timeline.set_easing", json!({"easing": e}));
                                }
                                Some(crate::ease_editor::Picked::Edit) => crate::ease_editor::open_here(app),
                                None => {}
                            }
                            if ui.button("✎").on_hover_text(tr!("Edit Ease…")).named(tr!("Edit Ease…")).clicked() {
                                crate::ease_editor::open_here(app);
                            }
                        });
                        // Classic Ease: its strength, as Animate's Ease field.
                        if let anicraft_engine::doc::Easing::Classic(s) = t.easing {
                            let mut v = s;
                            row(ui, tr!("Strength"), |ui| {
                                if ui.add(egui::Slider::new(&mut v, -100.0..=100.0).step_by(1.0)).named(tr!("Classic ease strength")).changed() {
                                    live(app, tr_noop!("Ease"), "timeline.set_easing", json!({"easing": {"classic": v}}));
                                }
                            });
                        }
                        // Property groups eased their own way (ADR 0081), named.
                        if !t.groups.is_empty() {
                            let groups: Vec<String> = t.groups.iter().map(|(g, e)| format!("{} {}", crate::i18n::t(g.label()), crate::ease_editor::label(&app.s.doc, e))).collect();
                            ui.label(text(Type::Caption, tr!("Eased separately: {groups}", groups = groups.join(", "))));
                        }
                        let shape = t.kind == anicraft_engine::doc::TweenKind::Shape;
                        row_prose(ui, tr!("Kind"), |ui| {
                            egui::ComboBox::from_id_salt("prop_kind").width(crate::theme::fit(ui, 150.0)).selected_text(if shape { tr!("Shape") } else { tr!("Classic") }).show_ui(ui, |ui| {
                                if ui.selectable_label(!shape, tr!("Classic")).clicked() && shape {
                                    app.run("timeline.create_tween", json!({}));
                                }
                                if ui.selectable_label(shape, tr!("Shape")).on_hover_text(tr!("Morphs one shape into another")).clicked() && !shape {
                                    app.run("timeline.create_shape_tween", json!({}));
                                }
                            })
                            .response
                            .set_name(tr!("Tween kind"));
                        });
                        // On a layer a motion guide steers: turn with the path.
                        let guided = layer.parent.is_some_and(|p| app.s.timeline().layer(p).is_some_and(|g| g.kind == anicraft_engine::doc::LayerKind::Guide));
                        if guided {
                            let mut orient = t.orient;
                            if crate::theme::checkbox(ui, &mut orient, tr!("Orient to path")).on_hover_text(tr!("Turn with the motion guide as it bends")).changed() {
                                app.run("timeline.set_orient", json!({"orient": orient}));
                            }
                        }
                        crate::ease_editor::preview(app, ui, &t.easing);
                    }
                    None => {
                        ui.horizontal_wrapped(|ui| {
                            if ui.button(tr!("Create Motion Tween")).on_hover_text(tr!("Animate this span's object property by property")).clicked() {
                                app.run("timeline.create_motion_tween", json!({}));
                            }
                            if ui.button(tr!("Create Classic Tween")).clicked() {
                                app.run("timeline.create_tween", json!({}));
                            }
                            if ui.button(tr!("Create Shape Tween")).on_hover_text(tr!("Morph this keyframe's shapes into the next keyframe's")).clicked() {
                                app.run("timeline.create_shape_tween", json!({}));
                            }
                        });
                    }
                }
            }
            None => {
                ui.label(text(Type::Caption, tr!("Empty frame — press F7 for a blank keyframe")));
            }
        }
    }
}

/// A layer with sounds: their volume, and Mute (ADR 0039).
fn volume_row(app: &mut App, ui: &mut egui::Ui, layer: &anicraft_engine::doc::Layer) {
    let (mut pct, mut muted) = (f64::from(layer.volume) * 100.0, layer.muted);
    let id = layer.id.0;
    row_prose(ui, tr!("Volume"), |ui| {
        let r = with_type(ui, Type::Data, |ui| ui.add_enabled(!muted, egui::DragValue::new(&mut pct).range(0.0..=100.0).speed(0.5).max_decimals(0).suffix(" %")));
        if r.named(tr!("Layer volume")).on_hover_text(tr!("The volume of this layer's sounds, and of the sounds in the symbols on it")).changed() {
            live(app, tr_noop!("Layer Volume"), "layer.set_volume", json!({"layer": id, "volume": (pct / 100.0).clamp(0.0, 1.0)}));
        }
        if crate::theme::checkbox(ui, &mut muted, tr!("Mute")).on_hover_text(tr!("Silence this layer's sounds while you work and in every export")).named(tr!("Mute layer")).changed() {
            app.run("layer.set_volume", json!({"layer": id, "muted": muted}));
        }
    });
}

/// The keyframe's sound, as Animate's Sound section: which sound (or none), its sync, whether
/// it repeats a number of times or loops, and its effect.
fn sound_rows(app: &mut App, ui: &mut egui::Ui, current: Option<anicraft_engine::doc::FrameSound>) {
    use anicraft_engine::doc::{SoundEffect, SoundSync};
    let sounds: Vec<(u64, String)> = app.s.doc.sounds.iter().map(|x| (x.id.0, x.name.clone())).collect();
    if sounds.is_empty() && current.is_none() {
        return;
    }
    let name = current.as_ref().and_then(|f| app.s.doc.sound(f.sound)).map_or_else(|| tr!("None").to_string(), |x| x.name.clone());
    row_prose(ui, tr!("Sound"), |ui| {
        egui::ComboBox::from_id_salt("prop_sound").width(crate::theme::fit(ui, 150.0)).selected_text(name).show_ui(ui, |ui| {
            if ui.selectable_label(current.is_none(), tr!("None")).clicked() && current.is_some() {
                app.run("sound.remove", json!({}));
            }
            for (id, n) in &sounds {
                if ui.selectable_label(current.as_ref().is_some_and(|f| f.sound.0 == *id), n.as_str()).clicked() {
                    app.run("sound.set", json!({"sound": id}));
                }
            }
        })
        .response
        .set_name(tr!("Sound"));
    });
    let Some(f) = current else { return };
    row_prose(ui, tr!("Sync"), |ui| {
        egui::ComboBox::from_id_salt("prop_sync").width(crate::theme::fit(ui, 150.0)).selected_text(crate::i18n::t(f.sync.label())).show_ui(ui, |ui| {
            for m in SoundSync::ALL {
                if ui.selectable_label(f.sync == m, crate::i18n::t(m.label())).on_hover_text(crate::i18n::t(m.hint())).clicked() {
                    app.run("sound.set", json!({"sync": m.key()}));
                }
            }
        })
        .response
        .set_name(tr!("Sound sync"));
    });
    if f.sync == SoundSync::Stop {
        return;
    }
    row_prose(ui, tr!("Repeat"), |ui| {
        egui::ComboBox::from_id_salt("prop_repeat").width(crate::theme::fit(ui, 90.0)).selected_text(if f.looping { tr!("Loop") } else { tr!("Repeat") }).show_ui(ui, |ui| {
            if ui.selectable_label(!f.looping, tr!("Repeat")).clicked() && f.looping {
                app.run("sound.set", json!({"loop": false}));
            }
            if ui.selectable_label(f.looping, tr!("Loop")).on_hover_text(tr!("Plays again and again: to the end of the movie (event) or of this span (stream)")).clicked() && !f.looping {
                app.run("sound.set", json!({"loop": true}));
            }
        })
        .response
        .set_name(tr!("Repeat or loop"));
        if !f.looping {
            let mut n = f.repeat;
            let r = with_type(ui, Type::Data, |ui| ui.add(egui::DragValue::new(&mut n).range(1..=anicraft_engine::doc::sound::MAX_REPEAT).suffix(" ×")));
            if r.named(tr!("Times it plays")).on_hover_text(tr!("Times it plays in a row")).changed() {
                live(app, tr_noop!("Sound repeat"), "sound.set", json!({"repeat": n}));
            }
        }
    });
    // The effect, as Animate's Effect menu, with the envelope editor beside it (ADR 0039).
    row_prose(ui, tr!("Effect"), |ui| {
        egui::ComboBox::from_id_salt("prop_effect").width(crate::theme::fit(ui, 150.0)).selected_text(crate::i18n::t(f.effect.label())).show_ui(ui, |ui| {
            for e in SoundEffect::PRESETS {
                if ui.selectable_label(f.effect == e, crate::i18n::t(e.label())).on_hover_text(crate::i18n::t(e.hint())).clicked() && f.effect != e {
                    app.run("sound.set", json!({"effect": e.key()}));
                }
            }
            let custom = matches!(f.effect, SoundEffect::Custom(_));
            if ui.selectable_label(custom, tr!("Custom")).on_hover_text(crate::i18n::t(SoundEffect::Custom(Vec::new()).hint())).clicked() {
                if !custom {
                    app.run("sound.set", json!({"effect": "custom"}));
                }
                crate::envelope::open(app);
            }
        })
        .response
        .set_name(tr!("Sound effect"));
        if ui.small_button(tr!("Edit…")).on_hover_text(tr!("Edit Envelope: the volume of each channel over the sound's time")).named(tr!("Edit sound envelope")).clicked() {
            crate::envelope::open(app);
        }
    });
    if let Some(x) = app.s.doc.sound(f.sound) {
        let (seconds, khz, format) = (format!("{:.2}", x.seconds()), format!("{:.1}", f64::from(x.rate) / 1000.0), crate::i18n::t(x.format.label()));
        let line = if x.channels == 2 { tr!("{seconds} s · {khz} kHz stereo · {format}", seconds = seconds, khz = khz, format = format) } else { tr!("{seconds} s · {khz} kHz mono · {format}", seconds = seconds, khz = khz, format = format) };
        ui.label(text(Type::Data, line).color(T.text_faint));
    }
}

/// Tools that only ever draw or paint a line, in the stroke colour: Properties shows no fill
/// for them, nor a stroke on/off (#252).
pub(crate) fn stroke_only(tool: Tool) -> bool {
    matches!(tool, Tool::InkBottle | Tool::PaintBrush | Tool::Pencil | Tool::Line)
}

fn tool_props(app: &mut App, ui: &mut egui::Ui) {
    if app.tool == Tool::Eraser {
        eraser_props(app, ui);
        return;
    }
    if app.tool == Tool::Text {
        // What new text gets (the last style used, as in Animate).
        let mut st = app.s.text_style.clone();
        if text_style_rows(app, ui, &mut st, None).is_some() {
            app.s.text_style = st;
        }
        return;
    }
    let mut st = app.s.style.clone();
    // Each tool shows only what it uses: the bucket and the brush always fill; the ink bottle,
    // the Pencil and the Line tool always stroke and never fill (#252).
    // The Paint Brush (ADR 0068) draws in the stroke colour at its own size and profile.
    let (fills, ink) = (matches!(app.tool, Tool::PaintBucket | Tool::Brush), stroke_only(app.tool));
    let brush_line = app.tool == Tool::PaintBrush;
    if !ink {
        let bitmaps = crate::colour_panel::bitmaps(app);
        let mut set = None;
        row(ui, tr!("Fill"), |ui| {
            // The fill swatch: what new shapes, the Paint Bucket and the Brush pour, in the one
            // colour picker with the fill's type (ADR 0074).
            let paint = st.fill_paint();
            let (r, edit) = crate::colour_panel::swatch(ui, (st.use_fill || fills).then_some(&paint), &bitmaps);
            r.named(tr!("Fill colour")).on_hover_text(tr!("Fill: click for its colour, or a gradient or bitmap fill"));
            match edit {
                Some(crate::colour_panel::FillEdit::Solid(p)) => {
                    st.set_fill_color(Rgba::new(f32::from(p[0]) / 255.0, f32::from(p[1]) / 255.0, f32::from(p[2]) / 255.0, 1.0));
                    st.use_fill = true;
                }
                Some(crate::colour_panel::FillEdit::None(n)) => st.use_fill = !n || fills,
                other => set = other,
            }
            if let Some(what) = fill_kind_label(st.fill_paint.as_ref()) {
                ui.label(text(Type::Caption, what)).on_hover_text(tr!("Set in the colour picker or picked with the Eyedropper. Choose Solid in the picker to fill with a colour again."));
            }
            if !fills {
                crate::theme::checkbox(ui, &mut st.use_fill, "").named(tr!("Fill")).on_hover_text(tr!("Fill on/off"));
            }
        });
        // A type, stops or bitmap: the swatch through the command (the rest of the options go
        // on from what it set).
        if let Some(edit) = set {
            app.s.style = st;
            crate::colour_panel::apply_set(app, &edit, false);
            st = app.s.style.clone();
        }
    }
    if !fills {
        row(ui, tr!("Stroke"), |ui| {
            let mut c = rgb(st.stroke);
            if crate::colour::button(ui, &mut c).named(tr!("Stroke colour")).changed() {
                st.stroke = Rgba::new(c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0);
            }
            if !ink {
                crate::theme::checkbox(ui, &mut st.use_stroke, "").named(tr!("Stroke")).on_hover_text(tr!("Stroke on/off"));
            }
            if !brush_line {
                ui.add(egui::DragValue::new(&mut st.stroke_width).suffix(" px").speed(0.1).range(0.0..=100.0).max_decimals(1)).set_name(tr!("Stroke width"));
            }
        });
    }
    if !fills && !brush_line {
        // The width profile new strokes get.
        let current = st.profile();
        if let Some(pick) = profile_picker(ui, ProfileRow::Tool, current.as_ref(), false) {
            st.set_profile(pick);
        }
        // And their style, caps and joins (with the params the commands take).
        if let Some(edit) = line_style_rows(ui, ProfileRow::Tool, &st.line, false)
            && let Ok(Some(line)) = anicraft_engine::params::opt_stroke_style(&edit.params, &st.line, st.stroke_width)
        {
            st.line = line;
        }
    }
    // Lock Fill (ADR 0074): a gradient or bitmap runs on across shapes instead of being fitted
    // to each one. The Eyedropper turns it on for what it picks, as in Animate.
    if matches!(app.tool, Tool::PaintBucket | Tool::Brush) {
        row_prose(ui, tr!("Lock fill"), |ui| {
            crate::theme::checkbox(ui, &mut st.lock_fill, "").named(tr!("Lock fill")).on_hover_text(tr!("On: a gradient or bitmap is poured where it lies on the stage, so it runs on across shapes. Off: each shape gets it fitted to its own bounds."));
        });
    }
    // Merge drawing (ADR 0031) or drawing objects, for the tools that draw shapes.
    if matches!(app.tool, Tool::Rect | Tool::Oval | Tool::Star | Tool::Line | Tool::Pen | Tool::Pencil | Tool::Brush | Tool::PaintBrush) {
        let key = anicraft_engine::find_command("draw.object_drawing").and_then(|c| c.shortcut).map(crate::platform::shortcut_label).unwrap_or_default();
        row_prose(ui, tr!("Drawing"), |ui| {
            crate::theme::checkbox(ui, &mut st.object_drawing, tr!("Object drawing"))
                .on_hover_text(tr!("On: each shape is an object of its own. Off: shapes cut and merge with the shapes they touch ({key})", key = key));
        });
    }
    app.s.style = st;
    match app.tool {
        Tool::Rect => row(ui, tr!("Corners"), |ui| {
            ui.add(egui::DragValue::new(&mut app.rect_radius).suffix(" px").range(0.0..=500.0)).set_name(tr!("Corner radius"));
        }),
        Tool::Star => polystar_props(app, ui),
        Tool::PaintBucket => row_prose(ui, tr!("Gap size"), |ui| {
            egui::ComboBox::from_id_salt("bucket_gap").width(crate::theme::fit(ui, 170.0)).selected_text(crate::i18n::t(app.bucket_gap.label())).show_ui(ui, |ui| {
                for g in crate::app::GapSize::ALL {
                    ui.selectable_value(&mut app.bucket_gap, g, crate::i18n::t(g.label()));
                }
            })
            .response
            .set_name(tr!("Gap size"));
        }),
        Tool::Pencil => row(ui, tr!("Mode"), |ui| {
            use crate::pathtools::PencilMode;
            for m in [PencilMode::Smooth, PencilMode::Straighten, PencilMode::Ink] {
                ui.selectable_value(&mut app.pencil_mode, m, crate::i18n::t(m.label())).on_hover_text(crate::i18n::t(m.hint()));
            }
        }),
        Tool::Brush | Tool::PaintBrush => brush_props(app, ui),
        _ => {}
    }
}

/// PolyStar's Tool Settings (ADR 0080), inline as the other tools' options: the style, the number
/// of sides (or points) and, for a star, its point size.
fn polystar_props(app: &mut App, ui: &mut egui::Ui) {
    let o = &mut app.polystar;
    // "Shape", not Animate's "Style": the line style's row above is Style.
    row_prose(ui, tr!("Shape"), |ui| {
        ui.selectable_value(&mut o.star, false, tr!("Polygon")).on_hover_text(tr!("Draws a regular polygon."));
        ui.selectable_value(&mut o.star, true, tr!("Star")).on_hover_text(tr!("Draws a star."));
    });
    row(ui, if o.star { tr!("Points") } else { tr!("Sides") }, |ui| {
        ui.add(egui::DragValue::new(&mut o.sides).range(3..=32)).set_name(if o.star { tr!("Number of points") } else { tr!("Number of sides") });
    });
    if o.star {
        row(ui, tr!("Point size"), |ui| {
            ui.add(egui::DragValue::new(&mut o.point_size).range(0.0..=1.0).speed(0.01).fixed_decimals(2))
                .on_hover_text(tr!("How deep the points are: near 0 they are thin spikes, at 1 the star is the polygon through its tips."))
                .set_name(tr!("Star point size"));
        });
    }
}

/// The Eraser's options (ADR 0034): the Faucet, size, shape and erase mode, as in Animate.
fn eraser_props(app: &mut App, ui: &mut egui::Ui) {
    use crate::brushtool::{SIZE_MAX, SIZE_MIN};
    use anicraft_engine::cmds_erase::EraseMode;
    use anicraft_engine::geom::brush::TipShape;
    let keys = [app.keymap.primary("view.brush_smaller"), app.keymap.primary("view.brush_larger")].into_iter().flatten().map(crate::platform::shortcut_label).collect::<Vec<_>>().join(" / ");
    let o = &mut app.eraser;
    row_prose(ui, tr!("Faucet"), |ui| {
        crate::theme::checkbox(ui, &mut o.faucet, tr!("Delete whole fills and lines")).on_hover_text(tr!("On: a click deletes the whole fill or line under it. Off: drag to erase"));
    });
    ui.add_enabled_ui(!o.faucet, |ui| {
        row(ui, tr!("Size"), |ui| {
            let r = ui.add(egui::DragValue::new(&mut o.size).suffix(" px").speed(0.25).range(SIZE_MIN..=SIZE_MAX).max_decimals(1)).named(tr!("Eraser size"));
            if !keys.is_empty() {
                r.on_hover_text(tr!("Eraser size ({keys})", keys = keys));
            }
        });
        row(ui, tr!("Shape"), |ui| {
            for (shape, label, hint) in [(TipShape::Round, tr!("Round"), tr!("A round eraser")), (TipShape::Square, tr!("Square"), tr!("A square eraser"))] {
                ui.selectable_value(&mut o.shape, shape, label).on_hover_text(hint);
            }
        });
        row_prose(ui, tr!("Mode"), |ui| {
            egui::ComboBox::from_id_salt("erase_mode").width(crate::theme::fit(ui, 170.0)).selected_text(crate::i18n::t(o.mode.label())).show_ui(ui, |ui| {
                for m in EraseMode::ALL {
                    ui.selectable_value(&mut o.mode, m, crate::i18n::t(m.label())).on_hover_text(crate::i18n::t(m.hint()));
                }
            })
            .response
            .set_name(tr!("Erase mode"));
        });
    });
    row_prose(ui, tr!("Stage"), |ui| {
        if ui.button(tr!("Erase Everything…")).on_hover_text(tr!("Clears every visible, unlocked layer at this frame, as double-clicking the Eraser tool does. You can undo it")).clicked() {
            crate::erasertool::erase_everything(app);
        }
    });
}

/// The Brush's and the Paint Brush's options (ADR 0033, 0068): the brush, its tip, pressure
/// and shaping. The tip's shape and the paint mode are the Brush's only (the Paint Brush draws
/// a round line).
fn brush_props(app: &mut App, ui: &mut egui::Ui) {
    crate::brush_panel::props(app, ui, |ui, l, f| row(ui, l, |ui| f(ui)), |ui, l, f| row_prose(ui, l, |ui| f(ui)));
    let paint_brush = app.tool == Tool::PaintBrush;
    use crate::brushtool::{SIZE_MAX, SIZE_MIN};
    use anicraft_engine::cmds_brush::PaintMode;
    use anicraft_engine::geom::brush::TipShape;
    let keys = [app.keymap.primary("view.brush_smaller"), app.keymap.primary("view.brush_larger")].into_iter().flatten().map(crate::platform::shortcut_label).collect::<Vec<_>>().join(" / ");
    let b = crate::brushtool::options_mut(app);
    row(ui, tr!("Size"), |ui| {
        let r = ui.add(egui::DragValue::new(&mut b.size).suffix(" px").speed(0.25).range(SIZE_MIN..=SIZE_MAX).max_decimals(1)).named(tr!("Brush size"));
        if !keys.is_empty() {
            r.on_hover_text(tr!("Brush size ({keys})", keys = keys));
        }
    });
    if paint_brush {
        row(ui, tr!("Smoothing"), |ui| {
            ui.add(egui::DragValue::new(&mut b.smoothing).speed(0.5).range(0.0..=100.0).max_decimals(0)).named(tr!("Brush smoothing")).on_hover_text(tr!("How much the stroke is smoothed: 0 follows the hand closely"));
        });
        return;
    }
    row(ui, tr!("Shape"), |ui| {
        for (shape, label, hint) in [(TipShape::Round, tr!("Round"), tr!("A round or oval tip")), (TipShape::Square, tr!("Square"), tr!("A square or flat tip"))] {
            ui.selectable_value(&mut b.shape, shape, label).on_hover_text(hint);
        }
    });
    row(ui, tr!("Angle"), |ui| {
        ui.add(egui::DragValue::new(&mut b.angle).suffix("°").speed(1.0).range(0.0..=180.0).max_decimals(0)).named(tr!("Brush angle")).on_hover_text(tr!("Turns a flattened tip (clockwise)"));
    });
    row(ui, tr!("Roundness"), |ui| {
        ui.add(egui::DragValue::new(&mut b.roundness).suffix(" %").speed(0.5).range(5.0..=100.0).max_decimals(0)).named(tr!("Brush roundness")).on_hover_text(tr!("100 %: as wide as it is tall. Less flattens the tip, for calligraphic strokes"));
    });
    row(ui, tr!("Smoothing"), |ui| {
        ui.add(egui::DragValue::new(&mut b.smoothing).speed(0.5).range(0.0..=100.0).max_decimals(0)).named(tr!("Brush smoothing")).on_hover_text(tr!("How much the stroke is smoothed: 0 follows the hand closely"));
    });
    row_prose(ui, tr!("Mode"), |ui| {
        egui::ComboBox::from_id_salt("brush_mode").width(crate::theme::fit(ui, 170.0)).selected_text(crate::i18n::t(b.mode.label())).show_ui(ui, |ui| {
            for m in PaintMode::ALL {
                ui.selectable_value(&mut b.mode, m, crate::i18n::t(m.label())).on_hover_text(crate::i18n::t(m.hint()));
            }
        })
        .response
        .set_name(tr!("Paint mode"));
    });
}

/// A small picture of a width profile: the stroke's outline, left side up.
/// `name`: a clickable preview (in the profile menu) named for assistive technology; `None`:
/// a decoration beside the menu, not a control.
fn profile_preview(ui: &mut egui::Ui, profile: Option<&anicraft_engine::doc::WidthProfile>, size: egui::Vec2, color: Color32, name: Option<&str>) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(size, if name.is_some() { Sense::click() } else { Sense::hover() });
    if let Some(name) = name {
        crate::a11y::control(ui, &resp, egui::WidgetType::Button, name, None);
    }
    let p = ui.painter();
    let n = 32;
    let half = r.height() * 0.5 / profile.map_or(1.0, |pr| pr.max_factor().max(1.0));
    let at = |i: usize| {
        let t = i as f32 / n as f32;
        let (l, rr) = profile.map_or((1.0, 1.0), |pr| pr.at(t));
        let x = r.left() + t * r.width();
        (pos2(x, r.center().y - l * half), pos2(x, r.center().y + rr * half))
    };
    // Each slice is a convex quad, so any profile (a pinch too) fills correctly.
    for i in 0..n {
        let ((a0, b0), (a1, b1)) = (at(i), at(i + 1));
        p.add(egui::Shape::convex_polygon(vec![a0, a1, b1, b0], color, Stroke::NONE));
    }
    resp
}

/// Which place a profile or line style menu is in (each needs its own popup).
#[derive(Clone, Copy, Debug, Hash)]
enum ProfileRow {
    Selection,
    Tool,
}

/// The Width row: a profile menu with a picture of each built-in. Returns the one picked.
fn profile_picker(ui: &mut egui::Ui, which: ProfileRow, current: Option<&anicraft_engine::doc::WidthProfile>, differs: bool) -> Option<anicraft_engine::doc::ProfilePreset> {
    use anicraft_engine::doc::ProfilePreset;
    let preset = match current {
        None => Some(ProfilePreset::Uniform),
        Some(p) => p.preset(),
    };
    let label = preset.map_or_else(|| trn!("Custom ({n} point)", "Custom ({n} points)", current.map_or(0, |p| p.points().len())), |p| crate::i18n::t(p.label()).to_string());
    let mut picked = None;
    row_prose(ui, tr!("Width"), |ui| {
        profile_preview(ui, current, vec2(44.0, 12.0), T.text_dim, None);
        let menu = ui.id().with(which);
        egui::ComboBox::from_id_salt(menu).width(crate::theme::fit(ui, 150.0)).selected_text(label).show_ui(ui, |ui| {
            for pr in ProfilePreset::ALL {
                let on = preset == Some(pr);
                let r = ui.horizontal(|ui| {
                    let name = crate::i18n::t(pr.label());
                    let pic = profile_preview(ui, pr.profile().as_ref(), vec2(44.0, 12.0), if on { T.text } else { T.text_dim }, Some(name));
                    let lab = ui.selectable_label(on, name);
                    pic.clicked() || lab.clicked()
                });
                if r.inner && (!on || differs) {
                    picked = Some(pr);
                }
            }
        })
        .response
        .on_hover_text(tr!("How the stroke's width varies along it. Shape it by hand with the Width tool (U)."))
        .set_name(tr!("Stroke width profile"));
        mixed(ui, differs);
    });
    picked
}

/// A picture of a line style: solid, dashed or dotted, as the Style menu shows them.
fn line_preview(ui: &mut egui::Ui, kind: anicraft_engine::doc::DashKind, size: egui::Vec2, color: Color32) {
    use anicraft_engine::doc::DashKind;
    let (r, _) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter();
    let y = r.center().y;
    let line = Stroke::new(2.0, color);
    match kind {
        DashKind::Solid => {
            p.line_segment([pos2(r.left(), y), pos2(r.right(), y)], line);
        }
        DashKind::Dashed => {
            p.extend(egui::Shape::dashed_line(&[pos2(r.left(), y), pos2(r.right(), y)], line, 6.0, 3.0));
        }
        DashKind::Dotted => {
            let mut x = r.left() + 1.5;
            while x < r.right() {
                p.circle_filled(pos2(x, y), 1.5, color);
                x += 5.0;
            }
        }
    }
}

/// A change made in the line style rows: its undo label, the `stroke.set_style` params, and
/// whether it is a drag (coalesced into one undo step) rather than a pick.
struct LineEdit {
    label: &'static str,
    params: Value,
    drag: bool,
}

/// The rows under Stroke that shape the line, as in Animate's Properties: Style (solid, dashed,
/// dotted) with its lengths, Cap, Join and, for miter joins, the miter limit. `which` keeps each
/// place's menus apart. Returns the change, if any.
fn line_style_rows(ui: &mut egui::Ui, which: ProfileRow, st: &anicraft_engine::doc::StrokeStyle, differs: bool) -> Option<LineEdit> {
    use anicraft_engine::doc::{Cap, Dash, DashKind, Join};
    let mut edit = None;
    let kind = st.dash_kind();
    row_prose(ui, tr!("Style"), |ui| {
        line_preview(ui, kind, vec2(44.0, 12.0), T.text_dim);
        egui::ComboBox::from_id_salt((which, "line_style")).width(crate::theme::fit(ui, 110.0)).selected_text(crate::i18n::t(kind.label())).show_ui(ui, |ui| {
            for k in DashKind::ALL {
                let on = k == kind;
                let r = ui.horizontal(|ui| {
                    line_preview(ui, k, vec2(44.0, 12.0), if on { T.text } else { T.text_dim });
                    ui.selectable_label(on, crate::i18n::t(k.label())).clicked()
                });
                if r.inner && !on {
                    edit = Some(LineEdit { label: tr_noop!("Stroke style"), params: json!({"style": k.key()}), drag: false });
                }
            }
        })
        .response
        .on_hover_text(tr!("Solid, dashed or dotted"))
        .set_name(tr!("Stroke style"));
        mixed(ui, differs);
    });
    // The small dash editor: the first dash and gap (an imported pattern keeps the rest).
    match &st.dash {
        Some(Dash::Dashed { lengths, .. }) => {
            let mut v = lengths.to_vec();
            row(ui, tr!("Dash"), |ui| {
                let mut changed = false;
                for (i, name, hint) in [(0usize, tr!("Dash length"), tr!("Length of each dash")), (1, tr!("Gap length"), tr!("Length of each gap"))] {
                    if let Some(x) = v.get_mut(i) {
                        let r = ui.add(egui::DragValue::new(x).suffix(" px").speed(0.1).range(0.0..=1000.0).max_decimals(1)).named(name);
                        changed |= r.on_hover_text(hint).changed();
                    }
                }
                if lengths.len() > 2 {
                    ui.label(text(Type::Caption, trn!("+{n} more", "+{n} more", lengths.len() - 2))).on_hover_text(tr!("This pattern has more dashes and gaps than these two; they stay as they are"));
                }
                // Without any gap the line would be solid: that is the Style menu's job.
                if changed && v.iter().skip(1).step_by(2).sum::<f32>() > 0.0 {
                    edit = Some(LineEdit { label: tr_noop!("Dash length"), params: json!({"dash": v}), drag: true });
                }
            });
        }
        Some(Dash::Dotted { spacing }) => {
            let mut sp = *spacing;
            row(ui, tr!("Spacing"), |ui| {
                let r = ui.add(egui::DragValue::new(&mut sp).suffix(" px").speed(0.1).range(0.0..=1000.0).max_decimals(1)).named(tr!("Dot spacing"));
                if r.on_hover_text(tr!("Space between the dots")).changed() {
                    edit = Some(LineEdit { label: tr_noop!("Dot spacing"), params: json!({"dot_spacing": sp}), drag: true });
                }
            });
        }
        None => {}
    }
    row_prose(ui, tr!("Cap"), |ui| {
        egui::ComboBox::from_id_salt((which, "line_cap")).width(crate::theme::fit(ui, 110.0)).selected_text(crate::i18n::t(st.cap.label())).show_ui(ui, |ui| {
            for c in Cap::ALL {
                if ui.selectable_label(c == st.cap, crate::i18n::t(c.label())).clicked() && c != st.cap {
                    edit = Some(LineEdit { label: tr_noop!("Stroke cap"), params: json!({"cap": c.key()}), drag: false });
                }
            }
        })
        .response
        .on_hover_text(tr!("How open ends finish: flat at the end (None), rounded, or squared off past it"))
        .set_name(tr!("Stroke cap"));
        if kind == DashKind::Dotted {
            ui.label(text(Type::Caption, tr!("dots are round")));
        }
    });
    row_prose(ui, tr!("Join"), |ui| {
        egui::ComboBox::from_id_salt((which, "line_join")).width(crate::theme::fit(ui, 110.0)).selected_text(crate::i18n::t(st.join.label())).show_ui(ui, |ui| {
            for j in Join::ALL {
                if ui.selectable_label(j == st.join, crate::i18n::t(j.label())).clicked() && j != st.join {
                    edit = Some(LineEdit { label: tr_noop!("Stroke join"), params: json!({"join": j.key()}), drag: false });
                }
            }
        })
        .response
        .on_hover_text(tr!("How corners turn: sharp (Miter), rounded, or cut across (Bevel)"))
        .set_name(tr!("Stroke join"));
    });
    if st.join == Join::Miter {
        let mut m = st.miter_limit;
        row(ui, tr!("Miter"), |ui| {
            let r = ui.add(egui::DragValue::new(&mut m).speed(0.05).range(1.0..=100.0).max_decimals(1)).named(tr!("Miter limit"));
            if r.on_hover_text(tr!("Corners sharper than this are bevelled: how far a point may reach, in stroke widths")).changed() {
                edit = Some(LineEdit { label: tr_noop!("Miter limit"), params: json!({"miter": m}), drag: true });
            }
        });
    }
    edit
}

pub(crate) fn library(app: &mut App, ui: &mut egui::Ui) {
    // Where the panel is, for files dropped on it (ADR 0059).
    let r = ui.scope(|ui| crate::library::panel(app, ui)).response.rect;
    app.library.drop_zone.panel = Some(r);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #252: the Pencil and the Line tool never fill, whatever the fill swatch says, so their
    /// Properties show no Fill row; the Pen and the shape tools still fill.
    #[test]
    fn stroke_only_tools_never_fill() {
        let mut s = anicraft_engine::Session::default();
        s.style.use_fill = true;
        let pts: Vec<[f32; 2]> = (0..20).map(|i| [i as f32 * 5.0, (i % 4) as f32 * 6.0]).collect();
        // A loop back to where it began, too.
        let loop_pts: Vec<[f32; 2]> = (0..=24).map(|i| { let a = i as f32 / 24.0 * std::f32::consts::TAU; [200.0 + 40.0 * a.cos(), 200.0 + 40.0 * a.sin()] }).collect();
        for (id, p) in [("draw.freehand", json!({"points": pts})), ("draw.freehand", json!({"points": loop_pts})), ("draw.line", json!({"x0": 0, "y0": 0, "x1": 90, "y1": 40}))] {
            s.execute(id, &p).expect("drawn");
        }
        assert!(s.visible_elements().iter().all(|e| matches!(&e.kind, ElementKind::Shape(sh) if sh.fill.is_none() && sh.stroke.is_some())));
        assert!(stroke_only(Tool::Pencil) && stroke_only(Tool::Line) && stroke_only(Tool::InkBottle) && stroke_only(Tool::PaintBrush));
        for t in [Tool::Pen, Tool::Rect, Tool::Oval, Tool::Star, Tool::Brush, Tool::PaintBucket] {
            assert!(!stroke_only(t), "{t:?} fills");
        }
    }
}
