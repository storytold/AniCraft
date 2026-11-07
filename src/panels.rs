//! Tools bar, edit bar, status bar, Properties and Library panels.

use crate::app::{App, Tool};
use crate::icons::{self, Icon};
use crate::theme::{Section, T, TARGET, TOOL_BUTTON, TOOL_ICON, Type, color, font, section, text, to_color32, with_type};
use anicraft_engine::doc::{ElementKind, Paint, Rgba, SymbolKind, evaluate_layer_elements};
use anicraft_engine::element_label;
use egui::{Align2, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use serde_json::{Value, json};

// ----- tools --------------------------------------------------------------------------------

/// The Lucide icon for a tool.
fn tool_icon(tool: Tool) -> Icon {
    match tool {
        Tool::Select => Icon::Select,
        Tool::Subselect => Icon::Subselect,
        Tool::Transform => Icon::Transform,
        Tool::Pen => Icon::Pen,
        Tool::Pencil => Icon::Pencil,
        Tool::Rect => Icon::Rect,
        Tool::Oval => Icon::Oval,
        Tool::Star => Icon::PolyStar,
        Tool::Line => Icon::Line,
        Tool::PaintBucket => Icon::Bucket,
        Tool::InkBottle => Icon::Ink,
        Tool::Hand => Icon::Hand,
        Tool::Zoom => Icon::Zoom,
    }
}

pub fn tools(app: &mut App, ui: &mut egui::Ui) {
    ui.vertical_centered(|ui| {
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
            let tint = if active || resp.hovered() { Color32::WHITE } else { T.icon };
            icons::paint(ui.painter(), Rect::from_center_size(rect.center(), vec2(TOOL_ICON, TOOL_ICON)), tool_icon(tool), tint);
            let key = app.keymap.primary(crate::keymap::tool_id(tool)).map(|k| format!(" ({})", crate::platform::shortcut_label(k))).unwrap_or_default();
            if resp.on_hover_text(format!("{}{key}\n{}", tool.label(), tool.hint())).clicked() {
                app.tool = tool;
            }
            if matches!(tool, Tool::Transform | Tool::Pencil | Tool::Line | Tool::InkBottle) {
                // Tool groups: select, draw by hand, shapes, view.
                ui.add_space(3.0);
                let y = ui.cursor().top();
                ui.painter().line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], Stroke::new(1.0, T.border));
                ui.add_space(4.0);
            }
        }
        ui.add_space(10.0);
        // fill / stroke swatches
        let st = app.s.style;
        let (r, _) = ui.allocate_exact_size(vec2(TOOL_BUTTON, TOOL_BUTTON), Sense::hover());
        let back = Rect::from_min_size(r.left_top() + vec2(14.0, 14.0), vec2(24.0, 24.0));
        let front = Rect::from_min_size(r.left_top() + vec2(2.0, 2.0), vec2(24.0, 24.0));
        ui.painter().rect_filled(back, T.rc(), if st.use_fill { to_color32(st.fill) } else { T.field });
        ui.painter().rect_stroke(back, T.rc(), Stroke::new(1.0, T.border), StrokeKind::Inside);
        ui.painter().rect_filled(front, T.rc(), T.panel);
        ui.painter().rect_stroke(front.shrink(1.0), T.rc(), Stroke::new(5.0, if st.use_stroke { to_color32(st.stroke) } else { T.field }), StrokeKind::Inside);
        ui.painter().rect_stroke(front, T.rc(), Stroke::new(1.0, T.border), StrokeKind::Inside);
    });
}

// ----- edit bar & status ------------------------------------------------------------------

pub fn edit_bar(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal_centered(|ui| {
        // The scene being edited; the menu switches scenes and manages them.
        let name = app.s.doc.scenes.get(app.s.scene).map_or_else(|| "Scene".to_string(), |s| s.name.clone());
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
                }
            }
            ui.separator();
            for id in ["scene.new", "scene.duplicate", "scene.rename", "scene.delete", "scene.move_up", "scene.move_down"] {
                crate::menus::cmd_item(app, ui, id);
            }
        });
        ui.label(text(Type::Caption, "›"));
        let layer = app.s.active_layer().map_or("—".to_string(), |l| l.name.clone());
        ui.label(text(Type::Body, layer).color(T.text_dim));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let z = app.view.zoom;
            let canvas = app.stage_cache.canvas;
            egui::ComboBox::from_id_salt("zoom").width(84.0).selected_text(text(Type::Data, format!("{:>4.0}%", z * 100.0))).show_ui(ui, |ui| {
                if ui.selectable_label(false, "Fit in window").clicked() {
                    app.view.fit = true;
                }
                for pct in [25.0f32, 50.0, 100.0, 200.0, 400.0, 800.0] {
                    if ui.selectable_label((z * 100.0 - pct).abs() < 0.5, text(Type::Data, format!("{pct:>4.0}%"))).clicked() {
                        app.zoom_about(canvas, canvas.center(), pct / 100.0);
                    }
                }
            });
            if ui.selectable_label(app.onion, "Onion skin").clicked() {
                app.onion = !app.onion;
            }
            let st = app.s.doc.stage;
            ui.label(text(Type::Data, format!("{} × {} px", st.width, st.height)).color(T.text_faint));
        });
    });
}

pub fn status_bar(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal_centered(|ui| {
        let now = ui.input(|i| i.time);
        let msg = app.status.as_ref().filter(|s| s.error || now - s.at < 8.0 || s.at == 0.0);
        match msg {
            Some(s) => {
                ui.label(text(Type::Label, s.text.clone()).color(if s.error { T.error } else { T.ok }));
            }
            None => {
                ui.label(text(Type::Label, app.tool.hint()));
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let small = |t: String| text(Type::Data, t).color(T.text_faint);
            ui.label(small(format!("undo {} · {:.1} MB", app.s.history().undo_len(), app.s.history().bytes() as f32 / (1024.0 * 1024.0))));
            ui.add_space(12.0);
            if let Some((n, mb, hit)) = app.stage_cache.stats() {
                ui.label(small(format!("cache {n} fr · {mb:.0} MB · {hit:.0}% hit")));
                ui.add_space(12.0);
            }
            let (ready, needed) = app.stage_cache.ready;
            ui.label(small(format!("render {:.1} ms · tiles {ready}/{needed}", app.stage_cache.ms)));
            if let Some(p) = app.cursor_stage {
                ui.label(small(format!("x {:.0}  y {:.0}", p.x, p.y)));
            }
        });
    });
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
fn live(app: &mut App, label: &str, id: &str, p: Value) {
    app.s.begin_gesture(label);
    app.run(id, p);
}

/// A property row for values: proportional label (left-aligned column), monospace fields.
fn row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    crate::theme::form_row(ui, crate::theme::FORM_LABEL_DOCK, label, |ui| with_type(ui, Type::Data, add));
}

/// A property row whose field holds prose (names, labels, preset choices).
fn row_prose(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    crate::theme::form_row(ui, crate::theme::FORM_LABEL_DOCK, label, add);
}

pub fn right(app: &mut App, ui: &mut egui::Ui) {
    let h = ui.available_height();
    // Dock sections are layout bands; each folds away to give the others room.
    egui::ScrollArea::vertical().id_salt("props").max_height(h * 0.56).auto_shrink([false, true]).show(ui, |ui| {
        section(ui, "dock_properties", Section::Band, "Properties", None, |ui| {
            egui::Frame::NONE.inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| properties(app, ui));
        });
    });
    section(ui, "dock_align", Section::Band, "Align", None, |ui| {
        egui::Frame::NONE.inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| align_panel(app, ui));
    });
    section(ui, "dock_library", Section::Band, "Library", None, |ui| {
        egui::Frame::NONE.inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| library(app, ui));
    });
    section(ui, "dock_history", Section::Band, "History", None, |ui| {
        egui::Frame::NONE.inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| history(app, ui));
    });
}

/// The Align panel: align edges or centres, distribute, relative to the selection or the stage.
fn align_panel(app: &mut App, ui: &mut egui::Ui) {
    let has_sel = !app.s.selection.is_empty();
    let button = |app: &mut App, ui: &mut egui::Ui, icon: Icon, id: &str| {
        let (rect, resp) = ui.allocate_exact_size(vec2(TARGET + 4.0, TARGET), if has_sel { Sense::click() } else { Sense::hover() });
        if has_sel && resp.hovered() {
            ui.painter().rect_filled(rect, T.rc(), T.hover);
        }
        icons::paint(ui.painter(), Rect::from_center_size(rect.center(), vec2(18.0, 18.0)), icon, if !has_sel { T.text_faint } else if resp.hovered() { Color32::WHITE } else { T.icon });
        let spec = anicraft_engine::find_command(id);
        let tip = spec.map_or_else(String::new, |c| match app.keymap.primary(c.id) {
            Some(k) => format!("{} ({})", c.label, crate::platform::shortcut_label(k)),
            None => c.label.to_string(),
        });
        let tip = if has_sel { tip } else { format!("{tip}\nSelect something to align") };
        if resp.on_hover_text(tip).clicked() {
            // Leave it unset when off, so a single shape still aligns to the stage.
            app.run(id, if app.align_to_stage { json!({"to_stage": true}) } else { json!({}) });
        }
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (icon, id) in [(Icon::AlignLeft, "align.left"), (Icon::AlignHCenter, "align.hcenter"), (Icon::AlignRight, "align.right")] {
            button(app, ui, icon, id);
        }
        ui.add_space(10.0);
        for (icon, id) in [(Icon::AlignTop, "align.top"), (Icon::AlignVCenter, "align.vcenter"), (Icon::AlignBottom, "align.bottom")] {
            button(app, ui, icon, id);
        }
    });
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        button(app, ui, Icon::DistributeH, "align.distribute_h");
        button(app, ui, Icon::DistributeV, "align.distribute_v");
        ui.add_space(12.0);
        crate::theme::checkbox(ui, &mut app.align_to_stage, text(Type::Body, "To stage")).on_hover_text("Align to the stage instead of the selection's bounds (a single shape always aligns to the stage)");
    });
}

/// Undo history: past steps (click to go back), the current state, and redoable steps (dimmed).
fn history(app: &mut App, ui: &mut egui::Ui) {
    let hist = app.s.history();
    let past: Vec<String> = hist.undo_labels().map(str::to_string).collect();
    let future: Vec<String> = hist.redo_labels().map(str::to_string).collect();
    let evicted = hist.evicted;
    let mut target = None;
    egui::ScrollArea::vertical().id_salt("history").max_height(ui.available_height().max(60.0)).auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
        let base = if evicted > 0 { "… older steps freed (memory budget)" } else { "Opened document" };
        if ui.selectable_label(past.is_empty(), text(Type::Body, base).color(if evicted > 0 { T.text_faint } else { T.text_dim })).clicked() {
            target = Some(0);
        }
        for (i, label) in past.iter().enumerate() {
            let current = i + 1 == past.len();
            if ui.selectable_label(current, label.as_str()).clicked() {
                target = Some(i + 1);
            }
        }
        for (i, label) in future.iter().enumerate() {
            if ui.selectable_label(false, text(Type::Body, label.as_str()).color(T.text_faint).italics()).clicked() {
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

fn properties(app: &mut App, ui: &mut egui::Ui) {
    let elements = app.s.visible_elements();
    let sel: Vec<_> = elements.iter().filter(|e| app.s.selection.contains(&e.id)).cloned().collect();

    // ---- tool options first while a drawing tool is in hand: they're what you reach for ----
    if matches!(app.tool, Tool::Rect | Tool::Oval | Tool::Star | Tool::Line | Tool::Pen | Tool::Pencil | Tool::PaintBucket | Tool::InkBottle) {
        let title = format!("{} tool", app.tool.label());
        section(ui, "prop_tool", Section::Rule, &title, None, |ui| tool_props(app, ui));
    }
    if let Some(e) = sel.first() {
        let title = if sel.len() > 1 { format!("{} elements", sel.len()) } else { element_label(&app.s.doc, e) };
        section(ui, "prop_selection", Section::Rule, &title, None, |ui| selection_props(app, ui, e, sel.len()));
    } else {
        section(ui, "prop_document", Section::Rule, "Document", None, |ui| document_props(app, ui));
    }
    // ---- frame ----
    let frame = app.s.frame;
    if let Some(layer) = app.s.active_layer().cloned() {
        // Mixed title: prose label, data value.
        let detail = format!("{} · {}", frame + 1, layer.name);
        section(ui, "prop_frame", Section::Rule, "Frame", Some(&detail), |ui| frame_props(app, ui, &layer, frame));
    }
}

fn selection_props(app: &mut App, ui: &mut egui::Ui, e: &anicraft_engine::doc::Element, _count: usize) {
    {
        let d = e.transform.decompose();
        let (mut x, mut y, mut sx, mut sy, mut rot) = (d.tx, d.ty, d.sx * 100.0, d.sy * 100.0, d.rotation.to_degrees());
        row(ui, "Position", |ui| {
            let a = ui.add(egui::DragValue::new(&mut x).prefix("X ").speed(1.0).max_decimals(1));
            let b = ui.add(egui::DragValue::new(&mut y).prefix("Y ").speed(1.0).max_decimals(1));
            if a.changed() || b.changed() {
                live(app, "Move", "element.set_transform", json!({"x": x, "y": y}));
            }
        });
        row(ui, "Scale", |ui| {
            let a = ui.add(egui::DragValue::new(&mut sx).suffix(" %").speed(0.5).range(1.0..=10000.0).max_decimals(1));
            let b = ui.add(egui::DragValue::new(&mut sy).suffix(" %").speed(0.5).range(1.0..=10000.0).max_decimals(1));
            if a.changed() || b.changed() {
                live(app, "Scale", "element.set_transform", json!({"sx": sx / 100.0, "sy": sy / 100.0}));
            }
        });
        row(ui, "Rotation", |ui| {
            if ui.add(egui::DragValue::new(&mut rot).suffix("°").speed(0.5).range(-360.0..=360.0).max_decimals(1)).changed() {
                live(app, "Rotate", "element.set_transform", json!({"rotation": rot}));
            }
        });
        let mut alpha = e.color.alpha * 100.0;
        row(ui, "Alpha", |ui| {
            if ui.add(egui::Slider::new(&mut alpha, 0.0..=100.0).suffix(" %").max_decimals(0)).changed() {
                live(app, "Alpha", "element.set_alpha", json!({"alpha": alpha / 100.0}));
            }
        });
        if let ElementKind::Shape(sh) = &e.kind {
            ui.add_space(4.0);
            let fill = sh.fill.as_ref().map(Paint::primary_color);
            row(ui, "Fill", |ui| {
                let is_grad = matches!(sh.fill, Some(Paint::Linear { .. } | Paint::Radial { .. }));
                let mut c = rgb(fill.unwrap_or(Rgba::WHITE));
                if ui.color_edit_button_srgb(&mut c).changed() {
                    live(app, "Fill colour", "element.set_fill", json!({"color": hex(c)}));
                }
                let mut on = fill.is_some();
                if crate::theme::checkbox(ui, &mut on, "").on_hover_text("Fill on/off").changed() {
                    app.run("element.set_fill", if on { json!({"color": hex(c)}) } else { json!({"color": null}) });
                }
                if is_grad {
                    ui.label(text(Type::Caption, "gradient"));
                }
            });
            let mut sc = rgb(sh.stroke.map_or(app.s.style.stroke, |s| s.color));
            let mut sw = sh.stroke.map_or(0.0, |s| s.width);
            row(ui, "Stroke", |ui| {
                if ui.color_edit_button_srgb(&mut sc).changed() {
                    live(app, "Stroke colour", "element.set_stroke", json!({"color": hex(sc)}));
                }
                if ui.add(egui::DragValue::new(&mut sw).suffix(" px").speed(0.1).range(0.0..=200.0).max_decimals(1)).changed() {
                    live(app, "Stroke width", "element.set_stroke", json!({"width": sw}));
                }
            });
        }
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            for (label, id) in [("Convert to Symbol", "modify.convert_to_symbol"), ("Break Apart", "modify.break_apart"), ("Duplicate", "edit.duplicate"), ("Delete", "edit.delete")] {
                if ui.button(label).clicked() {
                    app.invoke(id);
                }
            }
        });
    }
}

fn document_props(app: &mut App, ui: &mut egui::Ui) {
    {
        let st = app.s.doc.stage;
        let (mut w, mut h, mut fps) = (st.width, st.height, st.fps);
        row(ui, "Size", |ui| {
            let a = ui.add(egui::DragValue::new(&mut w).suffix(" px").range(1..=8192));
            let b = ui.add(egui::DragValue::new(&mut h).suffix(" px").range(1..=8192));
            if a.changed() || b.changed() {
                live(app, "Stage size", "doc.set_stage", json!({"width": w, "height": h}));
            }
        });
        row(ui, "Frame rate", |ui| {
            if ui.add(egui::DragValue::new(&mut fps).suffix(" fps").speed(0.1).range(1.0..=120.0).max_decimals(2)).changed() {
                live(app, "Frame rate", "doc.set_stage", json!({"fps": fps}));
            }
        });
        let mut bg = rgb(st.background);
        row(ui, "Stage", |ui| {
            if ui.color_edit_button_srgb(&mut bg).changed() {
                live(app, "Stage colour", "doc.set_stage", json!({"background": hex(bg)}));
            }
            ui.label(text(Type::Data, format!("{} frames", app.s.frame_count())).color(T.text_faint));
        });
    }
}

fn frame_props(app: &mut App, ui: &mut egui::Ui, layer: &anicraft_engine::doc::Layer, frame: u32) {
    if layer.kind == anicraft_engine::doc::LayerKind::Folder {
        let tl = app.s.doc.timeline(app.s.scene);
        let n = tl.layers.iter().position(|l| l.id == layer.id).map_or(0, |i| tl.group_range(i).len().saturating_sub(1));
        let what = if n == 1 { "1 layer".to_string() } else { format!("{n} layers") };
        ui.label(text(Type::Label, format!("Folder · {what}")));
        ui.label(text(Type::Caption, "Folders hold layers, not frames. Hiding or locking the folder hides or locks everything in it; move layers in under it with Up and Down."));
        return;
    }
    {
        match layer.keyframe_at(frame) {
            Some(k) => {
                let n = evaluate_layer_elements(layer, frame).map_or(0, |(_, e)| e.len());
                let (frames, els) = (if k.duration == 1 { "frame" } else { "frames" }, if n == 1 { "element" } else { "elements" });
                ui.label(text(Type::Label, format!("Keyframe at {} · {} {frames} · {n} {els}", k.start + 1, k.duration)));
                if layer.kind == anicraft_engine::doc::LayerKind::Mask {
                    ui.label(text(Type::Caption, "Mask layer: its fills reveal the masked layers below. Lock it to hide its outline."));
                }
                let mut label = k.label.clone();
                row_prose(ui, "Label", |ui| {
                    if ui.add(egui::TextEdit::singleline(&mut label).desired_width(150.0)).changed() {
                        live(app, "Frame label", "timeline.set_label", json!({"label": label}));
                    }
                });
                match k.tween {
                    Some(t) => {
                        row_prose(ui, "Tween", |ui| {
                            egui::ComboBox::from_id_salt("prop_ease").width(150.0).selected_text(t.easing.label()).show_ui(ui, |ui| {
                                for (name, e) in anicraft_engine::doc::Easing::PRESETS {
                                    if ui.selectable_label(t.easing == e, name).clicked() {
                                        app.run("timeline.set_easing", json!({"easing": crate::timeline::easing_param(e)}));
                                    }
                                }
                            });
                        });
                        let shape = t.kind == anicraft_engine::doc::TweenKind::Shape;
                        row_prose(ui, "Kind", |ui| {
                            egui::ComboBox::from_id_salt("prop_kind").width(150.0).selected_text(if shape { "Shape" } else { "Classic" }).show_ui(ui, |ui| {
                                if ui.selectable_label(!shape, "Classic").clicked() && shape {
                                    app.run("timeline.create_tween", json!({}));
                                }
                                if ui.selectable_label(shape, "Shape").on_hover_text("Morphs one shape into another").clicked() && !shape {
                                    app.run("timeline.create_shape_tween", json!({}));
                                }
                            });
                        });
                        // On a layer a motion guide steers: turn with the path.
                        let guided = layer.parent.is_some_and(|p| app.s.doc.timeline(app.s.scene).layer(p).is_some_and(|g| g.kind == anicraft_engine::doc::LayerKind::Guide));
                        if guided {
                            let mut orient = t.orient;
                            if crate::theme::checkbox(ui, &mut orient, "Orient to path").on_hover_text("Turn with the motion guide as it bends").changed() {
                                app.run("timeline.set_orient", json!({"orient": orient}));
                            }
                        }
                        ease_curve(ui, t.easing);
                    }
                    None => {
                        ui.horizontal(|ui| {
                            if ui.button("Create Classic Tween").clicked() {
                                app.run("timeline.create_tween", json!({}));
                            }
                            if ui.button("Create Shape Tween").on_hover_text("Morph this keyframe's shapes into the next keyframe's").clicked() {
                                app.run("timeline.create_shape_tween", json!({}));
                            }
                        });
                    }
                }
            }
            None => {
                ui.label(text(Type::Caption, "Empty frame — press F7 for a blank keyframe"));
            }
        }
    }
}

fn tool_props(app: &mut App, ui: &mut egui::Ui) {
    let mut st = app.s.style;
    // Each tool shows only what it uses: the bucket always fills, the ink bottle always strokes.
    let (bucket, ink) = (app.tool == Tool::PaintBucket, app.tool == Tool::InkBottle);
    if !ink {
        row(ui, "Fill", |ui| {
            let mut c = rgb(st.fill);
            if ui.color_edit_button_srgb(&mut c).changed() {
                st.fill = Rgba::new(c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0);
            }
            if !bucket {
                crate::theme::checkbox(ui, &mut st.use_fill, "");
            }
        });
    }
    if !bucket {
        row(ui, "Stroke", |ui| {
            let mut c = rgb(st.stroke);
            if ui.color_edit_button_srgb(&mut c).changed() {
                st.stroke = Rgba::new(c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0);
            }
            if !ink {
                crate::theme::checkbox(ui, &mut st.use_stroke, "");
            }
            ui.add(egui::DragValue::new(&mut st.stroke_width).suffix(" px").speed(0.1).range(0.0..=100.0).max_decimals(1));
        });
    }
    app.s.style = st;
    match app.tool {
        Tool::Rect => row(ui, "Corners", |ui| {
            ui.add(egui::DragValue::new(&mut app.rect_radius).suffix(" px").range(0.0..=500.0));
        }),
        Tool::Star => row(ui, "Points", |ui| {
            ui.add(egui::DragValue::new(&mut app.star_points).range(3..=32));
        }),
        Tool::PaintBucket => row_prose(ui, "Gap size", |ui| {
            egui::ComboBox::from_id_salt("bucket_gap").width(170.0).selected_text(app.bucket_gap.label()).show_ui(ui, |ui| {
                for g in crate::app::GapSize::ALL {
                    ui.selectable_value(&mut app.bucket_gap, g, g.label());
                }
            });
        }),
        Tool::Pencil => row(ui, "Mode", |ui| {
            use crate::pathtools::PencilMode;
            for m in [PencilMode::Smooth, PencilMode::Straighten, PencilMode::Ink] {
                ui.selectable_value(&mut app.pencil_mode, m, m.label()).on_hover_text(m.hint());
            }
        }),
        _ => {}
    }
}

/// Plots the easing curve (output against time).
fn ease_curve(ui: &mut egui::Ui, e: anicraft_engine::doc::Easing) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width().min(240.0), 70.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(r, T.rc(), T.field);
    let inner = r.shrink(8.0);
    let pts: Vec<Pos2> = (0..=48)
        .map(|i| {
            let t = i as f32 / 48.0;
            pos2(inner.left() + t * inner.width(), inner.bottom() - e.apply(t) * inner.height())
        })
        .collect();
    p.line_segment([inner.left_bottom(), inner.right_top()], Stroke::new(1.0, T.border));
    p.add(egui::Shape::line(pts, Stroke::new(2.0, T.tween)));
}

fn library(app: &mut App, ui: &mut egui::Ui) {
    let lib = app.s.doc.library.clone();
    if lib.is_empty() {
        ui.label(text(Type::Caption, "No symbols yet. Select artwork and press F8."));
        return;
    }
    ui.label(text(Type::Caption, format!("{} items", lib.len())));
    egui::ScrollArea::vertical().id_salt("lib").max_height(140.0).auto_shrink([false, true]).show(ui, |ui| {
        for (i, sym) in lib.iter().enumerate() {
            let selected = app.library_sel == Some(i);
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), TARGET), Sense::click());
            if selected || resp.hovered() {
                ui.painter().rect_filled(r, T.rc(), if selected { T.accent_dim } else { T.hover });
            }
            if selected {
                ui.painter().rect_filled(Rect::from_min_size(r.left_top(), vec2(3.0, r.height())), 0.0, T.accent);
            }
            let icon = Rect::from_center_size(r.left_center() + vec2(14.0, 0.0), vec2(14.0, 14.0));
            let kind_color = match sym.kind {
                SymbolKind::Graphic => T.kind_graphic,
                SymbolKind::MovieClip => T.kind_movie_clip,
                SymbolKind::Button => T.kind_button,
            };
            ui.painter().rect_stroke(icon, 2.0, Stroke::new(1.5, kind_color), StrokeKind::Inside);
            ui.painter().circle_filled(icon.center(), 2.5, kind_color);
            ui.painter().text(r.left_center() + vec2(30.0, 0.0), Align2::LEFT_CENTER, &sym.name, font(Type::Body), color(Type::Body));
            ui.painter().text(
                r.right_center() - vec2(8.0, 0.0),
                Align2::RIGHT_CENTER,
                format!("{:?} · {}f", sym.kind, sym.timeline.frame_count()),
                font(Type::Caption),
                color(Type::Caption),
            );
            if resp.clicked() {
                app.library_sel = Some(i);
            }
            if resp.double_clicked() {
                app.run("library.place", json!({"symbol": sym.id.0}));
            }
        }
    });
    ui.add_space(4.0);
    let can_place = app.library_sel.and_then(|i| lib.get(i)).map(|s| s.id.0);
    if ui.add_enabled(can_place.is_some(), egui::Button::new("Place on stage")).on_hover_text("Adds an instance at the stage centre (or double-click an item)").clicked()
        && let Some(id) = can_place {
            app.run("library.place", json!({"symbol": id}));
        }
}
