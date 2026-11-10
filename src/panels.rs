//! Tools bar, edit bar, status bar, Properties and Library panels.

use crate::app::{App, LibraryItem, Tool};
use crate::icons::{self, Icon};
use crate::theme::{Section, T, TARGET, TOOL_BUTTON, TOOL_ICON, Type, color, font, section, text, to_color32, with_type};
use anicraft_engine::doc::{ElementKind, Paint, Rgba, SymbolKind, evaluate_layer_elements};
use anicraft_engine::element_label;
use crate::a11y::{self, Named};
use egui::{Align2, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, WidgetType, pos2, vec2};
use serde_json::{Value, json};

// ----- tools --------------------------------------------------------------------------------

/// The Lucide icon for a tool.
fn tool_icon(tool: Tool) -> Icon {
    match tool {
        Tool::Select => Icon::Select,
        Tool::Subselect => Icon::Subselect,
        Tool::Transform => Icon::Transform,
        Tool::Width => Icon::Width,
        Tool::Pen => Icon::Pen,
        Tool::Text => Icon::Type,
        Tool::Pencil => Icon::Pencil,
        Tool::Brush => Icon::Brush,
        Tool::Eraser => Icon::Eraser,
        Tool::Rect => Icon::Rect,
        Tool::Oval => Icon::Oval,
        Tool::Star => Icon::PolyStar,
        Tool::Line => Icon::Line,
        Tool::PaintBucket => Icon::Bucket,
        Tool::InkBottle => Icon::Ink,
        Tool::Eyedropper => Icon::Eyedropper,
        Tool::Camera => Icon::Camera,
        Tool::Hand => Icon::Hand,
        Tool::Zoom => Icon::Zoom,
    }
}

pub fn tools(app: &mut App, ui: &mut egui::Ui) {
    let area = ui.max_rect();
    // Scrolls (wheel, or Tab to a tool) when a large interface scale leaves too little height.
    // The margin is inside the scroll area, so focus rings drawn around a tool are not clipped.
    egui::ScrollArea::vertical().id_salt("tools").scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
        egui::Frame::NONE.inner_margin(egui::Margin::symmetric(6, 0)).show(ui, |ui| tool_column(app, ui, area));
    });
}

fn tool_column(app: &mut App, ui: &mut egui::Ui, area: Rect) {
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
            let tint = if active { T.on_active } else if resp.hovered() { T.icon_hot } else { T.icon };
            icons::paint(ui.painter(), Rect::from_center_size(rect.center(), vec2(TOOL_ICON, TOOL_ICON)), tool_icon(tool), tint);
            // "Pen tool (P)": what the tooltip says first, and what a screen reader says.
            let name = a11y::with_key(&format!("{} tool", tool.label()), app.keymap.primary(crate::keymap::tool_id(tool)));
            a11y::control(ui, &resp, WidgetType::Button, &name, Some(active));
            if active {
                a11y::anchor(ui, a11y::Region::Tools, area, &resp);
            }
            let resp = resp.on_hover_text(format!("{name}\n{}", tool.hint()));
            if resp.clicked() {
                app.tool = tool;
            }
            // Animate: double-clicking the Eraser erases everything on the stage (it asks first).
            if tool == Tool::Eraser && resp.double_clicked() {
                crate::erasertool::erase_everything(app);
            }
            if matches!(tool, Tool::Width | Tool::Eraser | Tool::Line | Tool::Eyedropper) {
                // Tool groups: select, draw by hand, shapes, view.
                ui.add_space(3.0);
                let y = ui.cursor().top();
                ui.painter().line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], Stroke::new(1.0, T.border));
                ui.add_space(4.0);
            }
        }
        ui.add_space(10.0);
        // fill / stroke swatches
        let st = &app.s.style;
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
        })
        .response
        .set_name("Scene");
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
            })
            .response
            .set_name("Zoom");
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
        // Readouts from the right first; the message takes the rest and is cut short (whole on
        // hover) rather than running under them. Narrow windows and large interface scales drop
        // the render and cache diagnostics before the message.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let wide = ui.available_width() >= 1000.0;
            let small = |t: String| text(Type::Data, t).color(T.text_faint);
            ui.label(small(format!("undo {} · {:.1} MB", app.s.history().undo_len(), app.s.history().bytes() as f32 / (1024.0 * 1024.0))));
            ui.add_space(12.0);
            if let Some(a) = crate::autosave::status(app, now) {
                ui.label(small(a)).on_hover_text("A recovery copy of your unsaved work. If AniCraft closes unexpectedly, the next start offers it back.");
                ui.add_space(12.0);
            }
            if wide {
                if let Some((n, mb, hit)) = app.stage_cache.stats() {
                    ui.label(small(format!("cache {n} fr · {mb:.0} MB · {hit:.0}% hit")));
                    ui.add_space(12.0);
                }
                let (ready, needed) = app.stage_cache.ready;
                ui.label(small(format!("render {:.1} ms{} · tiles {ready}/{needed}", app.stage_cache.ms, renderer_tag(app))));
            }
            if let Some(p) = app.cursor_stage {
                ui.label(small(format!("x {:.0}  y {:.0}", p.x, p.y)));
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let msg = app.status.as_ref().filter(|s| s.error || now - s.at < 8.0 || s.at == 0.0);
                let (line, colour) = match msg {
                    Some(s) => (s.text.as_str(), if s.error { T.error } else { T.ok }),
                    None => (app.tool.hint(), color(Type::Label)),
                };
                let r = ui.add(egui::Label::new(text(Type::Label, line).color(colour)).truncate());
                // Screen readers announce what an action did ("Deleted layer — Ctrl+Z to undo")
                // without moving focus (a polite live region).
                ui.ctx().accesskit_node_builder(r.id, |n| n.set_live(egui::accesskit::Live::Polite));
            });
        });
    });
}

/// Who draws the stage, when this build can draw it on the GPU (ADR 0019): " GPU", or " CPU"
/// with what kept the GPU out.
fn renderer_tag(app: &App) -> String {
    use crate::gpu_stage::State;
    if !crate::gpu_stage::AVAILABLE {
        return String::new();
    }
    match (app.stage_cache.gpu.state(), &app.stage_cache.drawn_by) {
        (State::Starting, _) => " · GPU starting".into(),
        (State::Failed(_), _) => " · CPU (GPU unavailable)".into(),
        (State::Running { .. }, Some((true, _))) => " · GPU".into(),
        (State::Running { .. }, Some((false, Some(why)))) => format!(" · CPU for {why}"),
        _ => " · CPU".into(),
    }
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
    let dock = ui.max_rect();
    // Dock sections are layout bands; each folds away to give the others room.
    let props = egui::ScrollArea::vertical().id_salt("props").max_height(h * 0.56).auto_shrink([false, true]).show(ui, |ui| {
        crate::theme::section_with_header(ui, "dock_properties", Section::Band, "Properties", None, |ui| {
            egui::Frame::NONE.inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| properties(app, ui));
        })
        .0
    });
    section(ui, "dock_align", Section::Band, "Align", None, |ui| {
        egui::Frame::NONE.inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| align_panel(app, ui));
    });
    // Ctrl+F6 lands on the Properties and Library headers; each panel owns the dock down to the
    // next one (Align goes with Properties, History with the Library).
    let split = ui.cursor().top();
    let (library_header, _) = crate::theme::section_with_header(ui, "dock_library", Section::Band, "Library", None, |ui| {
        egui::Frame::NONE.inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| library(app, ui));
    });
    section(ui, "dock_history", Section::Band, "History", None, |ui| {
        egui::Frame::NONE.inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| history(app, ui));
    });
    a11y::anchor(ui, a11y::Region::Properties, Rect::from_min_max(dock.min, pos2(dock.right(), split)), &props.inner);
    a11y::anchor(ui, a11y::Region::Library, Rect::from_min_max(pos2(dock.left(), split), dock.max), &library_header);
}

/// The Align panel: align edges or centres, distribute, relative to the selection or the stage.
fn align_panel(app: &mut App, ui: &mut egui::Ui) {
    let has_sel = !app.s.selection.is_empty();
    let button = |app: &mut App, ui: &mut egui::Ui, icon: Icon, id: &str| {
        let (rect, resp) = ui.allocate_exact_size(vec2(TARGET + 4.0, TARGET), if has_sel { Sense::click() } else { Sense::hover() });
        if has_sel && resp.hovered() {
            ui.painter().rect_filled(rect, T.rc(), T.hover);
        }
        icons::paint(ui.painter(), Rect::from_center_size(rect.center(), vec2(18.0, 18.0)), icon, if !has_sel { T.text_faint } else if resp.hovered() { T.icon_hot } else { T.icon });
        let spec = anicraft_engine::find_command(id);
        let tip = spec.map_or_else(String::new, |c| match app.keymap.primary(c.id) {
            Some(k) => format!("{} ({})", c.label, crate::platform::shortcut_label(k)),
            None => c.label.to_string(),
        });
        a11y::control(ui, &resp, WidgetType::Button, &tip, None);
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
    if matches!(app.tool, Tool::Rect | Tool::Oval | Tool::Star | Tool::Line | Tool::Pen | Tool::Text | Tool::Pencil | Tool::Brush | Tool::Eraser | Tool::PaintBucket | Tool::InkBottle | Tool::Eyedropper) {
        let title = format!("{} tool", app.tool.label());
        section(ui, "prop_tool", Section::Rule, &title, None, |ui| tool_props(app, ui));
    }
    // ---- the camera, with the Camera tool or its layer active (ADR 0027) ----
    let camera_layer = app.s.doc.timeline(app.s.scene).camera_layer().map(|l| l.id);
    if camera_layer.is_some() && (app.tool == Tool::Camera || camera_layer == Some(app.s.layer)) {
        section(ui, "prop_camera", Section::Rule, "Camera", None, |ui| camera_props(app, ui));
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
    if let ElementKind::Symbol { symbol, events, .. } = &e.kind
        && app.s.doc.symbol(*symbol).is_some_and(|x| x.kind != SymbolKind::Graphic)
    {
        instance_rows(app, ui, e, events);
    }
    {
        let d = e.transform.decompose();
        let (mut x, mut y, mut sx, mut sy, mut rot) = (d.tx, d.ty, d.sx * 100.0, d.sy * 100.0, d.rotation.to_degrees());
        row(ui, "Position", |ui| {
            let a = ui.add(egui::DragValue::new(&mut x).prefix("X ").speed(1.0).max_decimals(1)).named("Position X");
            let b = ui.add(egui::DragValue::new(&mut y).prefix("Y ").speed(1.0).max_decimals(1)).named("Position Y");
            if a.changed() || b.changed() {
                live(app, "Move", "element.set_transform", json!({"x": x, "y": y}));
            }
        });
        row(ui, "Scale", |ui| {
            let a = ui.add(egui::DragValue::new(&mut sx).suffix(" %").speed(0.5).range(1.0..=10000.0).max_decimals(1)).named("Scale width");
            let b = ui.add(egui::DragValue::new(&mut sy).suffix(" %").speed(0.5).range(1.0..=10000.0).max_decimals(1)).named("Scale height");
            if a.changed() || b.changed() {
                live(app, "Scale", "element.set_transform", json!({"sx": sx / 100.0, "sy": sy / 100.0}));
            }
        });
        row(ui, "Rotation", |ui| {
            if ui.add(egui::DragValue::new(&mut rot).suffix("°").speed(0.5).range(-360.0..=360.0).max_decimals(1)).named("Rotation").changed() {
                live(app, "Rotate", "element.set_transform", json!({"rotation": rot}));
            }
        });
        let mut alpha = e.color.alpha * 100.0;
        row(ui, "Alpha", |ui| {
            if a11y::slider(ui, &mut alpha, 0.0..=100.0, "Alpha", a11y::SliderLook { suffix: " %", max_decimals: Some(0), ..Default::default() }).changed() {
                live(app, "Alpha", "element.set_alpha", json!({"alpha": alpha / 100.0}));
            }
        });
        // Blending and filters, on movie clips and buttons as in Animate.
        if let ElementKind::Symbol { symbol, filters, .. } = &e.kind
            && app.s.doc.symbol(*symbol).is_some_and(|x| x.kind != SymbolKind::Graphic)
        {
            filter_rows(app, ui, filters);
        }
        if let ElementKind::Symbol { symbol, blend, .. } = &e.kind
            && app.s.doc.symbol(*symbol).is_some_and(|x| x.kind != SymbolKind::Graphic)
        {
            let current = *blend;
            row_prose(ui, "Blending", |ui| {
                egui::ComboBox::from_id_salt("prop_blend").selected_text(current.label()).show_ui(ui, |ui| {
                    for m in anicraft_engine::doc::BlendMode::ALL {
                        if ui.selectable_label(m == current, m.label()).clicked() && m != current {
                            app.run("element.set_blend", json!({"mode": m.key()}));
                        }
                    }
                })
                .response
                .set_name("Blending");
            });
        }
        if let ElementKind::Symbol { symbol, loop_mode, first_frame, .. } = &e.kind
            && let Some(sym) = app.s.doc.symbol(*symbol).map(|s| (s.kind, s.timeline.frame_count()))
        {
            use anicraft_engine::doc::LoopMode;
            let (kind, frames) = sym;
            ui.add_space(4.0);
            // How the instance plays: the symbol's type, and a graphic's looping.
            row_prose(ui, "Type", |ui| {
                let label = |k: SymbolKind| match k {
                    SymbolKind::Graphic => "Graphic",
                    SymbolKind::MovieClip => "Movie clip",
                    SymbolKind::Button => "Button",
                };
                egui::ComboBox::from_id_salt("prop_symkind").width(150.0).selected_text(label(kind)).show_ui(ui, |ui| {
                    for (k, id, tip) in [
                        (SymbolKind::Graphic, "graphic", "Keeps step with the timeline it is on (looping options below)"),
                        (SymbolKind::MovieClip, "movie_clip", "Plays its own timeline on a loop, from when it appears"),
                        (SymbolKind::Button, "button", "Shows its Up frame"),
                    ] {
                        if ui.selectable_label(kind == k, label(k)).on_hover_text(tip).clicked() && kind != k {
                            app.run("symbol.set_kind", json!({"symbol": symbol.0, "kind": id}));
                        }
                    }
                })
                .response
                .set_name("Symbol type");
            });
            // The class scripts know the symbol by (kept, not run; ADR 0013), in documents
            // that use scripts.
            let linkage = app.s.doc.symbol(*symbol).and_then(|s| s.linkage.clone());
            if linkage.is_some() || crate::actions_panel::scripted(&app.s.doc) {
                let (class, base) = linkage.map_or((String::new(), String::new()), |l| (l.class, l.base));
                let hint = if base.is_empty() { "The class scripts create this symbol by (its linkage). Kept with the document; scripts don't run yet".to_string() } else { format!("The class scripts create this symbol by (its linkage), extending {base}. Kept with the document; scripts don't run yet") };
                row_prose(ui, "Class", |ui| {
                    if let Some(v) = crate::actions_panel::class_field(ui, "prop_symclass", &class, "Linkage class", &hint) {
                        let base = if v.is_empty() { String::new() } else { base.clone() };
                        app.run("script.set_class", json!({"symbol": symbol.0, "class": v, "base": base}));
                    }
                });
            }
            if kind == SymbolKind::Graphic {
                let names = [(LoopMode::Loop, "loop", "Loop"), (LoopMode::PlayOnce, "play_once", "Play once"), (LoopMode::SingleFrame, "single_frame", "Single frame")];
                let current = names.iter().find(|(m, _, _)| m == loop_mode).map_or("Loop", |(_, _, l)| l);
                row_prose(ui, "Looping", |ui| {
                    egui::ComboBox::from_id_salt("prop_loop").width(150.0).selected_text(current).show_ui(ui, |ui| {
                        for (m, id, l) in names {
                            if ui.selectable_label(m == *loop_mode, l).clicked() && m != *loop_mode {
                                app.run("element.set_loop", json!({"loop": id}));
                            }
                        }
                    })
                    .response
                    .set_name("Looping");
                });
                let mut first = first_frame + 1;
                row(ui, "First frame", |ui| {
                    let r = ui.add(egui::DragValue::new(&mut first).range(1..=frames.max(1)).speed(0.2)).named("First frame");
                    if r.changed() {
                        live(app, "First frame", "element.set_loop", json!({"first_frame": first.saturating_sub(1)}));
                    }
                    ui.label(text(Type::Caption, format!("of {frames}")));
                });
            } else if kind == SymbolKind::MovieClip {
                ui.label(text(Type::Caption, format!("Plays its {frames}-frame timeline on a loop from when it appears.")));
            }
        }
        if let ElementKind::Text(t) = &e.kind {
            ui.add_space(4.0);
            let mut st = anicraft_engine::cmds_text::TextStyle { font: t.font.clone(), size: t.size, bold: t.bold, italic: t.italic, color: t.color, align: t.align, letter_spacing: t.letter_spacing, line_spacing: t.line_spacing };
            let mut width = t.width;
            if let Some((label, p)) = text_style_rows(app, ui, &mut st, Some(&mut width)) {
                live(app, label, "text.edit", p);
            }
            row_prose(ui, "Text", |ui| {
                if ui.button("Edit Text").on_hover_text("Type on the stage (or double-click the text)").clicked() {
                    let at = e.transform.apply(anicraft_engine::geom::Vec2::ZERO);
                    app.text_edit = Some(crate::app::TextEditing { at: pos2(at.x, at.y), element: Some(e.id), text: t.text.clone(), original: t.text.clone(), focus: true });
                }
            });
        }
        if let ElementKind::Shape(sh) = &e.kind {
            ui.add_space(4.0);
            let fill = sh.fill.as_ref().map(Paint::primary_color);
            row(ui, "Fill", |ui| {
                let is_grad = matches!(sh.fill, Some(Paint::Linear { .. } | Paint::Radial { .. }));
                let mut c = rgb(fill.unwrap_or(Rgba::WHITE));
                if ui.color_edit_button_srgb(&mut c).named("Fill colour").changed() {
                    live(app, "Fill colour", "element.set_fill", json!({"color": hex(c)}));
                }
                let mut on = fill.is_some();
                if crate::theme::checkbox(ui, &mut on, "").named("Fill").on_hover_text("Fill on/off").changed() {
                    app.run("element.set_fill", if on { json!({"color": hex(c)}) } else { json!({"color": null}) });
                }
                if let Some(Paint::Bitmap { bitmap, .. }) = &sh.fill {
                    let name = app.s.doc.bitmap(*bitmap).map_or_else(|| "missing bitmap".to_string(), |b| format!("{} · {}×{}", b.name, b.width, b.height));
                    ui.label(text(Type::Caption, name));
                } else if is_grad {
                    ui.label(text(Type::Caption, "gradient"));
                }
            });
            let mut sc = rgb(sh.stroke.as_ref().map_or(app.s.style.stroke, |s| s.color));
            let mut sw = sh.stroke.as_ref().map_or(0.0, |s| s.width);
            row(ui, "Stroke", |ui| {
                if ui.color_edit_button_srgb(&mut sc).named("Stroke colour").changed() {
                    live(app, "Stroke colour", "element.set_stroke", json!({"color": hex(sc)}));
                }
                if ui.add(egui::DragValue::new(&mut sw).suffix(" px").speed(0.1).range(0.0..=200.0).max_decimals(1)).named("Stroke width").changed() {
                    live(app, "Stroke width", "element.set_stroke", json!({"width": sw}));
                }
            });
            if let Some(st) = &sh.stroke {
                // The width profile, as Animate's Width menu: built-ins with a picture of each.
                let current = st.profile.clone();
                if let Some(pick) = profile_picker(ui, ProfileRow::Selection, current.as_ref()) {
                    app.run("stroke.set_profile", json!({"profile": pick.key()}));
                }
            }
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

/// A movie clip or button instance's name (what actions address it by) and its events, as at
/// the top of Animate's Properties.
fn instance_rows(app: &mut App, ui: &mut egui::Ui, e: &anicraft_engine::doc::Element, events: &[anicraft_engine::doc::EventHandler]) {
    let mut name = e.name.clone();
    row_prose(ui, "Instance", |ui| {
        let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(150.0).hint_text("instance name")).named("Instance name");
        if r.on_hover_text("The name actions use to address it (letters, digits and _)").changed() {
            live(app, "Instance name", "instance.set_name", json!({"element": e.id.0, "name": name}));
        }
    });
    let key = app.keymap.primary("view.actions").map(crate::platform::shortcut_label).unwrap_or_default();
    row_prose(ui, "Events", |ui| {
        let what = if events.is_empty() { "None".to_string() } else { events.iter().map(|h| h.event.label()).collect::<Vec<_>>().join(", ") };
        ui.label(text(Type::Body, what).color(if events.is_empty() { T.text_faint } else { T.text }));
        if ui.small_button("Actions…").on_hover_text(format!("Edit what it does on clicks and the pointer ({key})")).clicked() {
            crate::actions_panel::open(app, crate::actions_panel::Tab::Instance);
        }
    });
    ui.add_space(4.0);
}

/// The Filters rows of a movie clip or button instance: an Add menu, then each filter's values
/// with a remove button. Every change replaces the list (`element.set_filters`), one undo step
/// per gesture.
fn filter_rows(app: &mut App, ui: &mut egui::Ui, current: &[anicraft_engine::doc::Filter]) {
    use anicraft_engine::doc::Filter;
    // As the renderer sees them (a file may hold values or ramps a command would refuse).
    let mut list: Vec<Filter> = current.iter().map(Filter::sanitized).collect();
    let mut changed: Option<&'static str> = None;
    row_prose(ui, "Filters", |ui| {
        egui::ComboBox::from_id_salt("add_filter").selected_text(if list.is_empty() { "None" } else { "Add filter" }).show_ui(ui, |ui| {
            for (kind, label) in Filter::KINDS {
                if ui.selectable_label(false, label).clicked()
                    && let Some(f) = Filter::new(kind)
                {
                    list.push(f);
                    changed = Some("Add filter");
                }
            }
        })
        .response
        .set_name("Add filter");
    });
    let quality_names = ["Low", "Medium", "High"];
    let mut remove = None;
    for (i, f) in list.iter_mut().enumerate() {
        // Screen readers hear which filter a value belongs to: "Drop Shadow blur X".
        let fname = f.label();
        ui.push_id(("filter", i), |ui| {
            row_prose(ui, fname, |ui| {
                if ui.small_button("Remove").named(&format!("Remove {fname}")).on_hover_text("Remove this filter").clicked() {
                    remove = Some(i);
                }
            });
            let blur_rows = |ui: &mut egui::Ui, bx: &mut f32, by: &mut f32, quality: &mut u8, changed: &mut Option<&'static str>| {
                row(ui, "Blur", |ui| {
                    let a = ui.add(egui::DragValue::new(bx).prefix("X ").suffix(" px").speed(0.25).range(0.0..=255.0).max_decimals(1)).named(&format!("{fname} blur X"));
                    let b = ui.add(egui::DragValue::new(by).prefix("Y ").suffix(" px").speed(0.25).range(0.0..=255.0).max_decimals(1)).named(&format!("{fname} blur Y"));
                    if a.changed() || b.changed() {
                        *changed = Some("Filter blur");
                    }
                });
                row_prose(ui, "Quality", |ui| {
                    for (q, name) in quality_names.iter().enumerate() {
                        if ui.selectable_label(usize::from(*quality) == q + 1, *name).named(&format!("{fname} quality {name}")).clicked() {
                            *quality = q as u8 + 1;
                            *changed = Some("Filter quality");
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
                    offset_row(ui, fname, angle, distance, "Shadow offset", &mut changed);
                    row_prose(ui, "Options", |ui| {
                        for (v, label) in [(inner, "Inner"), (knockout, "Knockout"), (hide_object, "Hide object")] {
                            if crate::theme::checkbox(ui, v, label).changed() {
                                changed = Some("Shadow options");
                            }
                        }
                    });
                }
                Filter::Glow { blur_x, blur_y, strength, quality, color, inner, knockout } => {
                    blur_rows(ui, blur_x, blur_y, quality, &mut changed);
                    shadow_rows(ui, fname, strength, color, &mut changed);
                    row_prose(ui, "Options", |ui| {
                        for (v, label) in [(inner, "Inner"), (knockout, "Knockout")] {
                            if crate::theme::checkbox(ui, v, label).changed() {
                                changed = Some("Glow options");
                            }
                        }
                    });
                }
                Filter::Bevel { blur_x, blur_y, strength, quality, angle, distance, shadow_color, highlight_color, side, knockout } => {
                    blur_rows(ui, blur_x, blur_y, quality, &mut changed);
                    strength_row(ui, fname, strength, &mut changed);
                    row_prose(ui, "Colours", |ui| {
                        for (c, label, undo) in [(shadow_color, "Shadow", "Bevel shadow colour"), (highlight_color, "Highlight", "Bevel highlight colour")] {
                            ui.label(label);
                            if alpha_swatch(ui, c, &format!("{fname} {} colour", label.to_lowercase())) {
                                changed = Some(undo);
                            }
                        }
                    });
                    offset_row(ui, fname, angle, distance, "Bevel offset", &mut changed);
                    side_rows(ui, fname, side, knockout, &mut changed);
                }
                Filter::GradientGlow { blur_x, blur_y, strength, quality, angle, distance, stops, side, knockout } | Filter::GradientBevel { blur_x, blur_y, strength, quality, angle, distance, stops, side, knockout } => {
                    // The ramp first: it is what makes the filter.
                    ramp_editor(ui, fname, stops, pin, &mut changed);
                    blur_rows(ui, blur_x, blur_y, quality, &mut changed);
                    strength_row(ui, fname, strength, &mut changed);
                    offset_row(ui, fname, angle, distance, "Filter offset", &mut changed);
                    side_rows(ui, fname, side, knockout, &mut changed);
                }
                Filter::AdjustColor { brightness, contrast, saturation, hue } => {
                    for (v, label, range) in [(brightness, "Brightness", 100.0), (contrast, "Contrast", 100.0), (saturation, "Saturation", 100.0), (hue, "Hue", 180.0)] {
                        row(ui, label, |ui| {
                            if a11y::slider(ui, v, -range..=range, label, a11y::SliderLook { max_decimals: Some(0), ..Default::default() }).changed() {
                                changed = Some("Adjust colour");
                            }
                        });
                    }
                }
            }
        });
    }
    if let Some(i) = remove {
        list.remove(i);
        changed = Some("Remove filter");
    }
    if let Some(label) = changed
        && let Ok(v) = serde_json::to_value(&list)
    {
        live(app, label, "element.set_filters", json!({"filters": v}));
    }
}

/// Strength and colour rows of a drop shadow or glow.
fn shadow_rows(ui: &mut egui::Ui, fname: &str, strength: &mut f32, color: &mut Rgba, changed: &mut Option<&'static str>) {
    row(ui, "Strength", |ui| {
        let mut pct = *strength * 100.0;
        if ui.add(egui::DragValue::new(&mut pct).suffix(" %").speed(1.0).range(0.0..=1000.0).max_decimals(0)).named(&format!("{fname} strength")).changed() {
            *strength = pct / 100.0;
            *changed = Some("Filter strength");
        }
        let mut c = rgb(*color);
        if ui.color_edit_button_srgb(&mut c).named(&format!("{fname} colour")).on_hover_text("Colour").changed() {
            *color = Rgba::new(f32::from(c[0]) / 255.0, f32::from(c[1]) / 255.0, f32::from(c[2]) / 255.0, color.a);
            *changed = Some("Filter colour");
        }
    });
}

/// The Strength row of a bevel or gradient filter.
fn strength_row(ui: &mut egui::Ui, fname: &str, strength: &mut f32, changed: &mut Option<&'static str>) {
    row(ui, "Strength", |ui| {
        let mut pct = *strength * 100.0;
        if ui.add(egui::DragValue::new(&mut pct).suffix(" %").speed(1.0).range(0.0..=1000.0).max_decimals(0)).named(&format!("{fname} strength")).changed() {
            *strength = pct / 100.0;
            *changed = Some("Filter strength");
        }
    });
}

/// Angle and distance of a shadow, bevel or gradient filter.
fn offset_row(ui: &mut egui::Ui, fname: &str, angle: &mut f32, distance: &mut f32, undo: &'static str, changed: &mut Option<&'static str>) {
    row(ui, "Offset", |ui| {
        let a = ui
            .add(egui::DragValue::new(angle).prefix("Angle ").suffix("°").speed(1.0).range(0.0..=360.0).max_decimals(0))
            .named(&format!("{fname} angle"))
            .on_hover_text("The direction the shadow falls (the light comes from the opposite side)");
        let b = ui.add(egui::DragValue::new(distance).prefix("Distance ").suffix(" px").speed(0.25).range(-1000.0..=1000.0).max_decimals(1)).named(&format!("{fname} distance"));
        if a.changed() || b.changed() {
            *changed = Some(undo);
        }
    });
}

/// The Type (inner, outer, full) and Knockout rows of a bevel or gradient filter.
fn side_rows(ui: &mut egui::Ui, fname: &str, side: &mut anicraft_engine::doc::FilterSide, knockout: &mut bool, changed: &mut Option<&'static str>) {
    use anicraft_engine::doc::FilterSide;
    row_prose(ui, "Type", |ui| {
        for (v, tip) in [(FilterSide::Inner, "Inside the artwork's edge"), (FilterSide::Outer, "Outside the artwork's edge"), (FilterSide::Full, "Inside and outside the edge")] {
            if ui.selectable_label(*side == v, v.label()).named(&format!("{fname} type {}", v.label())).on_hover_text(tip).clicked() && *side != v {
                *side = v;
                *changed = Some("Filter type");
            }
        }
    });
    row_prose(ui, "Options", |ui| {
        if crate::theme::checkbox(ui, knockout, "Knockout").named(&format!("{fname} knockout")).on_hover_text("Show only the effect, not the artwork").changed() {
            *changed = Some("Filter options");
        }
    });
}

/// A colour swatch with opacity (Animate's bevel colours carry alpha). True when changed.
fn alpha_swatch(ui: &mut egui::Ui, c: &mut Rgba, name: &str) -> bool {
    let s = c.sanitized();
    let b = |v: f32| (v * 255.0).round() as u8;
    let mut px = [b(s.r), b(s.g), b(s.b), b(s.a)];
    let r = ui.color_edit_button_srgba_unmultiplied(&mut px).named(name).on_hover_text(name);
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
    row_prose(ui, "Gradient", |ui| {
        ui.vertical(|ui| {
            let w = ui.available_width().clamp(80.0, 200.0);
            let (strip, resp) = ui.allocate_exact_size(vec2(w, 16.0), Sense::click());
            a11y::control(ui, &resp, WidgetType::Button, &format!("{fname} gradient: add a colour stop"), None);
            let resp = resp.on_hover_text("Click to add a colour stop");
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
                let label = format!("{fname} colour stop {} at {:.0}%{}", i + 1, st.offset * 100.0, if pinned { " (fixed)" } else { "" });
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
                    *changed = Some("Move colour stop");
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
                *changed = Some("Add colour stop");
            }
        });
    });
    sel = sel.min(stops.len().saturating_sub(1));
    let mut remove = false;
    let count = stops.len();
    if let Some(st) = stops.get_mut(sel) {
        let pinned = is_pinned(st);
        row(ui, "Stop", |ui| {
            if pinned {
                // Colour only: this stop stays transparent.
                let mut c = rgb(st.color);
                if ui.color_edit_button_srgb(&mut c).named(&format!("{fname} stop colour")).on_hover_text("Stop colour (this stop stays transparent)").changed() {
                    st.color = Rgba::new(f32::from(c[0]) / 255.0, f32::from(c[1]) / 255.0, f32::from(c[2]) / 255.0, 0.0);
                    *changed = Some("Colour stop");
                }
            } else if alpha_swatch(ui, &mut st.color, &format!("{fname} stop colour")) {
                *changed = Some("Colour stop");
            }
            let mut pct = st.offset * 100.0;
            let r = ui.add_enabled(!pinned, egui::DragValue::new(&mut pct).suffix(" %").speed(0.5).range(0.0..=100.0).max_decimals(0)).named(&format!("{fname} stop position"));
            if r.changed() {
                let (lo, hi) = room.get(sel).copied().unwrap_or((st.offset, st.offset));
                st.offset = (pct / 100.0).clamp(lo, hi);
                *changed = Some("Move colour stop");
            }
            if ui.add_enabled(!pinned && count > 2, egui::Button::new("Remove").small()).named(&format!("Remove {fname} colour stop")).on_hover_text("Remove this colour stop").clicked() {
                remove = true;
            }
        });
    }
    if remove && sel < stops.len() {
        stops.remove(sel);
        sel = sel.saturating_sub(1);
        *changed = Some("Remove colour stop");
    }
    ui.data_mut(|d| d.insert_temp(sel_id, sel));
}

/// Font, size, style, colour, alignment and spacing rows for text. Returns the edit made, as
/// `text.edit` params (one key per change), with an undo label.
pub fn text_style_rows(app: &mut App, ui: &mut egui::Ui, st: &mut anicraft_engine::cmds_text::TextStyle, width: Option<&mut Option<f32>>) -> Option<(&'static str, Value)> {
    use anicraft_engine::doc::TextAlign;
    let families = app.font_families.get_or_insert_with(|| anicraft_engine::text::Fonts::get().families().to_vec()).clone();
    let mut out: Option<(&'static str, Value)> = None;
    row_prose(ui, "Font", |ui| {
        let shown = if st.font.is_empty() { anicraft_engine::text::Fonts::get().resolve("").unwrap_or_default() } else { st.font.clone() };
        egui::ComboBox::from_id_salt("text_font").selected_text(shown.clone()).width(ui.available_width().min(220.0)).height(320.0).show_ui(ui, |ui| {
            if families.is_empty() {
                ui.label(text(Type::Caption, "No fonts found on this computer"));
            }
            for f in &families {
                if ui.selectable_label(*f == shown, f).clicked() {
                    st.font = f.clone();
                    out = Some(("Font", json!({"font": f})));
                }
            }
        })
        .response
        .set_name("Font");
    });
    row(ui, "Size", |ui| {
        if ui.add(egui::DragValue::new(&mut st.size).suffix(" px").speed(0.25).range(1.0..=2000.0).max_decimals(1)).named("Font size").changed() {
            out = Some(("Font size", json!({"size": st.size})));
        }
        let mut c = rgb(st.color);
        if ui.color_edit_button_srgb(&mut c).named("Text colour").on_hover_text("Text colour").changed() {
            st.color = Rgba::hex(u32::from(c[0]) << 16 | u32::from(c[1]) << 8 | u32::from(c[2]));
            out = Some(("Text colour", json!({"color": hex(c)})));
        }
    });
    row_prose(ui, "Style", |ui| {
        if ui.selectable_label(st.bold, text(Type::Body, "B").strong()).named("Bold").on_hover_text("Bold").clicked() {
            st.bold = !st.bold;
            out = Some(("Bold", json!({"bold": st.bold})));
        }
        if ui.selectable_label(st.italic, text(Type::Body, "I").italics()).named("Italic").on_hover_text("Italic").clicked() {
            st.italic = !st.italic;
            out = Some(("Italic", json!({"italic": st.italic})));
        }
        ui.separator();
        for (a, label, key) in [(TextAlign::Left, "Left", "left"), (TextAlign::Center, "Centre", "center"), (TextAlign::Right, "Right", "right")] {
            let name = format!("Align {}", label.to_lowercase());
            if ui.selectable_label(st.align == a, label).named(&name).on_hover_text(name.as_str()).clicked() {
                st.align = a;
                out = Some(("Text alignment", json!({"align": key})));
            }
        }
    });
    row(ui, "Spacing", |ui| {
        if ui.add(egui::DragValue::new(&mut st.letter_spacing).prefix("Letters ").suffix(" px").speed(0.1).range(-1000.0..=1000.0).max_decimals(1)).named("Letter spacing").changed() {
            out = Some(("Letter spacing", json!({"letter_spacing": st.letter_spacing})));
        }
        if ui.add(egui::DragValue::new(&mut st.line_spacing).prefix("Lines ").suffix(" px").speed(0.1).range(-1000.0..=1000.0).max_decimals(1)).named("Line spacing").changed() {
            out = Some(("Line spacing", json!({"line_spacing": st.line_spacing})));
        }
    });
    if let Some(width) = width {
        row(ui, "Width", |ui| {
            let mut wrap = width.is_some();
            if crate::theme::checkbox(ui, &mut wrap, "Wrap").on_hover_text("Wrap lines at a fixed width; off, each paragraph stays on one line").changed() {
                *width = wrap.then_some(300.0);
                out = Some(("Text width", json!({"width": *width})));
            }
            if let Some(w) = width.as_mut()
                && ui.add(egui::DragValue::new(w).suffix(" px").speed(1.0).range(1.0..=100_000.0).max_decimals(0)).named("Text width").changed()
            {
                out = Some(("Text width", json!({"width": *w})));
            }
        });
    }
    out
}

/// The camera at the playhead: where it looks, its zoom, turn and tint, and the preview switch.
/// Edits key the playhead like any edit of a tweened object.
fn camera_props(app: &mut App, ui: &mut egui::Ui) {
    let Some(cam) = crate::camera_tool::camera(app) else { return };
    let (mut x, mut y) = (cam.center.x, cam.center.y);
    row(ui, "Position", |ui| {
        let a = ui.add(egui::DragValue::new(&mut x).prefix("x ").suffix(" px").speed(1.0).max_decimals(1)).named("Camera X");
        let b = ui.add(egui::DragValue::new(&mut y).prefix("y ").suffix(" px").speed(1.0).max_decimals(1)).named("Camera Y");
        if a.changed() || b.changed() {
            live(app, "Move Camera", "camera.set", json!({"x": x, "y": y}));
        }
    });
    let mut zoom = cam.zoom * 100.0;
    let (lo, hi) = (anicraft_engine::doc::camera::MIN_ZOOM * 100.0, anicraft_engine::doc::camera::MAX_ZOOM * 100.0);
    row(ui, "Zoom", |ui| {
        ui.spacing_mut().slider_width = 96.0;
        if a11y::slider(ui, &mut zoom, lo..=hi, "Camera zoom", a11y::SliderLook { suffix: " %", max_decimals: Some(0), logarithmic: true, ..Default::default() }).changed() {
            live(app, "Zoom Camera", "camera.set", json!({"zoom": zoom}));
        }
    });
    let mut rot = cam.rotation.to_degrees();
    row(ui, "Rotation", |ui| {
        if ui.add(egui::DragValue::new(&mut rot).suffix("°").speed(0.5).max_decimals(1)).named("Camera rotation").changed() {
            live(app, "Rotate Camera", "camera.set", json!({"rotation": rot}));
        }
    });
    let mut tint = rgb(cam.tint);
    let mut amount = cam.tint_amount * 100.0;
    row(ui, "Tint", |ui| {
        let a = ui.color_edit_button_srgb(&mut tint).named("Camera tint colour");
        let b = ui.add(egui::DragValue::new(&mut amount).suffix(" %").speed(0.5).range(0.0..=100.0).max_decimals(0)).named("Camera tint amount");
        if a.changed() || b.changed() {
            live(app, "Camera Tint", "camera.set", json!({"tint": hex(tint), "tint_amount": amount}));
        }
    });
    ui.horizontal_wrapped(|ui| {
        let mut on = app.camera_preview;
        if crate::theme::checkbox(ui, &mut on, "Preview through camera").on_hover_text("Show the stage as the movie shows it (View › Preview Through Camera)").changed() {
            app.toggle_camera_preview();
        }
        if ui.button("Reset").named("Reset camera").on_hover_text("Back to the stage as it is: centred, 100 %, no turn, no tint").clicked() {
            app.run("camera.set", json!({"reset": true}));
        }
    });
    let hidden = app.s.doc.timeline(app.s.scene).camera_layer().is_some_and(|l| !l.visible);
    ui.label(text(Type::Caption, if hidden { "The camera layer is hidden: the movie shows the stage as it is." } else { "The movie shows what the frame on the stage shows. Layer depth (in Frame, per layer) moves far layers less." }));
}

fn document_props(app: &mut App, ui: &mut egui::Ui) {
    {
        let st = app.s.doc.stage;
        let (mut w, mut h, mut fps) = (st.width, st.height, st.fps);
        row(ui, "Size", |ui| {
            let a = ui.add(egui::DragValue::new(&mut w).suffix(" px").range(1..=8192)).named("Stage width");
            let b = ui.add(egui::DragValue::new(&mut h).suffix(" px").range(1..=8192)).named("Stage height");
            if a.changed() || b.changed() {
                live(app, "Stage size", "doc.set_stage", json!({"width": w, "height": h}));
            }
        });
        row(ui, "Frame rate", |ui| {
            if ui.add(egui::DragValue::new(&mut fps).suffix(" fps").speed(0.1).range(1.0..=120.0).max_decimals(2)).named("Frame rate").changed() {
                live(app, "Frame rate", "doc.set_stage", json!({"fps": fps}));
            }
        });
        let mut bg = rgb(st.background);
        row(ui, "Stage", |ui| {
            if ui.color_edit_button_srgb(&mut bg).named("Stage colour").changed() {
                live(app, "Stage colour", "doc.set_stage", json!({"background": hex(bg)}));
            }
            ui.label(text(Type::Data, format!("{} frames", app.s.frame_count())).color(T.text_faint));
        });
        // The document class, in documents that use scripts (kept, not run; ADR 0013).
        if crate::actions_panel::scripted(&app.s.doc) {
            let current = app.s.doc.document_class.clone();
            row_prose(ui, "Class", |ui| {
                if let Some(v) = crate::actions_panel::class_field(ui, "prop_docclass", &current, "Document class", "The class scripts run the main timeline as (the document class). Kept with the document; scripts don't run yet") {
                    app.run("script.set_class", json!({"class": v}));
                }
            });
        }
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
    // Layer depth matters once the scene has a camera: deeper layers move less as it pans.
    if layer.kind != anicraft_engine::doc::LayerKind::Camera && app.s.doc.timeline(app.s.scene).camera_layer().is_some() {
        let mut depth = layer.depth;
        let (lo, hi) = (anicraft_engine::doc::camera::MIN_DEPTH, anicraft_engine::doc::camera::MAX_DEPTH);
        row(ui, "Depth", |ui| {
            let r = ui.add(egui::DragValue::new(&mut depth).suffix(" px").speed(2.0).range(lo..=hi).max_decimals(0)).named("Layer depth");
            if r.on_hover_text("How far behind the stage this layer is: when the camera pans, a deeper layer moves less (parallax). Negative is in front.").changed() {
                live(app, "Layer Depth", "layer.set_depth", json!({"depth": depth}));
            }
        });
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
                    if ui.add(egui::TextEdit::singleline(&mut label).desired_width(150.0)).named("Frame label").changed() {
                        live(app, "Frame label", "timeline.set_label", json!({"label": label}));
                    }
                });
                sound_rows(app, ui, k.sound.clone());
                if layer.has_sounds() {
                    volume_row(app, ui, layer);
                }
                let key = app.keymap.primary("view.actions").map(crate::platform::shortcut_label).unwrap_or_default();
                row_prose(ui, "Actions", |ui| {
                    let what = k.script.as_ref().map_or_else(|| "None".to_string(), anicraft_engine::doc::FrameScript::describe);
                    let short: String = what.chars().take(36).collect();
                    let short = if short.len() < what.len() { format!("{short}…") } else { short };
                    ui.label(text(Type::Body, short).color(if k.script.is_none() { T.text_faint } else { T.text })).on_hover_text(what);
                    if ui.small_button("Edit…").on_hover_text(format!("What runs when the playhead enters this keyframe ({key})")).clicked() {
                        app.actions.symbol = None;
                        crate::actions_panel::open(app, crate::actions_panel::Tab::Frame);
                    }
                });
                if let Some(m) = &k.motion {
                    // A motion tween: what is keyed, and the way to its curves.
                    row_prose(ui, "Tween", |ui| {
                        ui.label(text(Type::Body, "Motion"));
                    });
                    let keyed: Vec<&str> = m.tracks.iter().map(|t| t.property.label()).collect();
                    let n = m.key_frames().len();
                    ui.label(text(Type::Caption, if keyed.is_empty() { "No property keyframes yet: move or change the object at a later frame to add some.".to_string() } else { format!("{} · {n} {}", keyed.join(", "), if n == 1 { "key frame" } else { "key frames" }) }));
                    // The motion path (ADR 0030): bent on the stage, straightened here.
                    let segments = m.path.as_ref().map(|p| p.cmds.len().saturating_sub(1));
                    row_prose(ui, "Path", |ui| {
                        let what = match segments {
                            Some(1) => "Curved · 1 segment".to_string(),
                            Some(n) => format!("Curved · {n} segments"),
                            None => "Straight".to_string(),
                        };
                        let how = format!(
                            "Select the object to see its path on the stage. Drag the path to bend it and its ends to move them; {} the path or the object to move the whole path. With the Subselection tool (A), drag points and handles, double-click the path to add a point (or Alt-click it with the Pen), Delete removes points, and right-click makes them smooth or corners.",
                            crate::platform::shortcut_label("Ctrl+Drag")
                        );
                        ui.label(text(Type::Body, what)).on_hover_text(how);
                        if segments.is_some() && ui.small_button("Straighten").on_hover_text("Back to the straight path the X and Y curves trace").named("Straighten motion path").clicked() {
                            app.run("motion.clear_path", json!({}));
                        }
                    });
                    // Where the path starts; changing it moves the whole path (ADR 0038).
                    let base = m.target_index(&k.elements).and_then(|i| k.elements.get(i)).map(|e| anicraft_engine::doc::Props::of(e));
                    let start = m.path.clone().or_else(|| base.and_then(|b| m.path_from_tracks(&b))).and_then(|p| anicraft_engine::doc::motion::path_ends(&p)).map(|(a, _)| a);
                    if let Some(a) = start {
                        let (mut x, mut y) = (a.x, a.y);
                        row(ui, "Path start", |ui| {
                            let hint = "Where the motion starts. Changing it moves the whole path, and the object with it; the timing stays.";
                            let rx = ui.add(egui::DragValue::new(&mut x).prefix("X ").speed(1.0).max_decimals(1)).named("Motion path start X").on_hover_text(hint);
                            let ry = ui.add(egui::DragValue::new(&mut y).prefix("Y ").speed(1.0).max_decimals(1)).named("Motion path start Y").on_hover_text(hint);
                            if rx.changed() || ry.changed() {
                                live(app, "Move Motion Path", "motion.move_path", json!({"dx": x - a.x, "dy": y - a.y}));
                            }
                        });
                    }
                    let mut orient = m.orient;
                    if crate::theme::checkbox(ui, &mut orient, "Orient to path").on_hover_text("Turn the object as its motion path turns").named("Orient to path").changed() {
                        app.run("motion.orient", json!({"orient": orient}));
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Motion Editor").on_hover_text("Edit the property curves and eases").clicked() {
                            app.bottom = crate::app::BottomTab::MotionEditor;
                        }
                        if ui.button("Remove Tween").on_hover_text("The object stays as it is on the span's first frame").clicked() {
                            app.run("timeline.remove_tween", json!({}));
                        }
                    });
                    return;
                }
                match k.tween {
                    Some(t) => {
                        row_prose(ui, "Tween", |ui| {
                            egui::ComboBox::from_id_salt("prop_ease").width(150.0).selected_text(t.easing.label()).show_ui(ui, |ui| {
                                for (name, e) in anicraft_engine::doc::Easing::PRESETS {
                                    if ui.selectable_label(t.easing == e, name).clicked() {
                                        app.run("timeline.set_easing", json!({"easing": crate::timeline::easing_param(e)}));
                                    }
                                }
                            })
                            .response
                            .set_name("Easing");
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
                            })
                            .response
                            .set_name("Tween kind");
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
                        ui.horizontal_wrapped(|ui| {
                            if ui.button("Create Motion Tween").on_hover_text("Animate this span's object property by property").clicked() {
                                app.run("timeline.create_motion_tween", json!({}));
                            }
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

/// A layer with sounds: their volume, and Mute (ADR 0039).
fn volume_row(app: &mut App, ui: &mut egui::Ui, layer: &anicraft_engine::doc::Layer) {
    let (mut pct, mut muted) = (f64::from(layer.volume) * 100.0, layer.muted);
    let id = layer.id.0;
    row_prose(ui, "Volume", |ui| {
        let r = with_type(ui, Type::Data, |ui| ui.add_enabled(!muted, egui::DragValue::new(&mut pct).range(0.0..=100.0).speed(0.5).max_decimals(0).suffix(" %")));
        if r.named("Layer volume").on_hover_text("The volume of this layer's sounds, and of the sounds in the symbols on it").changed() {
            live(app, "Layer Volume", "layer.set_volume", json!({"layer": id, "volume": (pct / 100.0).clamp(0.0, 1.0)}));
        }
        if crate::theme::checkbox(ui, &mut muted, "Mute").on_hover_text("Silence this layer's sounds while you work and in every export").named("Mute layer").changed() {
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
    let name = current.as_ref().and_then(|f| app.s.doc.sound(f.sound)).map_or_else(|| "None".to_string(), |x| x.name.clone());
    row_prose(ui, "Sound", |ui| {
        egui::ComboBox::from_id_salt("prop_sound").width(150.0).selected_text(name).show_ui(ui, |ui| {
            if ui.selectable_label(current.is_none(), "None").clicked() && current.is_some() {
                app.run("sound.remove", json!({}));
            }
            for (id, n) in &sounds {
                if ui.selectable_label(current.as_ref().is_some_and(|f| f.sound.0 == *id), n.as_str()).clicked() {
                    app.run("sound.set", json!({"sound": id}));
                }
            }
        })
        .response
        .set_name("Sound");
    });
    let Some(f) = current else { return };
    row_prose(ui, "Sync", |ui| {
        egui::ComboBox::from_id_salt("prop_sync").width(150.0).selected_text(f.sync.label()).show_ui(ui, |ui| {
            for m in SoundSync::ALL {
                if ui.selectable_label(f.sync == m, m.label()).on_hover_text(m.hint()).clicked() {
                    app.run("sound.set", json!({"sync": m.key()}));
                }
            }
        })
        .response
        .set_name("Sound sync");
    });
    if f.sync == SoundSync::Stop {
        return;
    }
    row_prose(ui, "Repeat", |ui| {
        egui::ComboBox::from_id_salt("prop_repeat").width(90.0).selected_text(if f.looping { "Loop" } else { "Repeat" }).show_ui(ui, |ui| {
            if ui.selectable_label(!f.looping, "Repeat").clicked() && f.looping {
                app.run("sound.set", json!({"loop": false}));
            }
            if ui.selectable_label(f.looping, "Loop").on_hover_text("Plays again and again: to the end of the movie (event) or of this span (stream)").clicked() && !f.looping {
                app.run("sound.set", json!({"loop": true}));
            }
        })
        .response
        .set_name("Repeat or loop");
        if !f.looping {
            let mut n = f.repeat;
            let r = with_type(ui, Type::Data, |ui| ui.add(egui::DragValue::new(&mut n).range(1..=anicraft_engine::doc::sound::MAX_REPEAT).suffix(" ×")));
            if r.named("Times it plays").on_hover_text("Times it plays in a row").changed() {
                live(app, "Sound repeat", "sound.set", json!({"repeat": n}));
            }
        }
    });
    // The effect, as Animate's Effect menu, with the envelope editor beside it (ADR 0039).
    row_prose(ui, "Effect", |ui| {
        egui::ComboBox::from_id_salt("prop_effect").width(150.0).selected_text(f.effect.label()).show_ui(ui, |ui| {
            for e in SoundEffect::PRESETS {
                if ui.selectable_label(f.effect == e, e.label()).on_hover_text(e.hint()).clicked() && f.effect != e {
                    app.run("sound.set", json!({"effect": e.key()}));
                }
            }
            let custom = matches!(f.effect, SoundEffect::Custom(_));
            if ui.selectable_label(custom, "Custom").on_hover_text(SoundEffect::Custom(Vec::new()).hint()).clicked() {
                if !custom {
                    app.run("sound.set", json!({"effect": "custom"}));
                }
                crate::envelope::open(app);
            }
        })
        .response
        .set_name("Sound effect");
        if ui.small_button("Edit…").on_hover_text("Edit Envelope: the volume of each channel over the sound's time").named("Edit sound envelope").clicked() {
            crate::envelope::open(app);
        }
    });
    if let Some(x) = app.s.doc.sound(f.sound) {
        let ch = if x.channels == 2 { "stereo" } else { "mono" };
        ui.label(text(Type::Data, format!("{:.2} s · {:.1} kHz {ch} · {}", x.seconds(), f64::from(x.rate) / 1000.0, x.format.label())).color(T.text_faint));
    }
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
    // Each tool shows only what it uses: the bucket and the brush always fill, the ink bottle
    // always strokes.
    let (fills, ink) = (matches!(app.tool, Tool::PaintBucket | Tool::Brush), app.tool == Tool::InkBottle);
    if !ink {
        row(ui, "Fill", |ui| {
            let mut c = rgb(st.fill);
            if ui.color_edit_button_srgb(&mut c).named("Fill colour").changed() {
                st.set_fill_color(Rgba::new(c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0));
            }
            // A gradient or bitmap picked with the Eyedropper (ADR 0034): the swatch shows its
            // first colour, and says what it holds.
            if let Some(p) = &st.fill_paint {
                let what = match p {
                    Paint::Linear { .. } => "Linear gradient",
                    Paint::Radial { .. } => "Radial gradient",
                    Paint::Bitmap { .. } => "Bitmap fill",
                    Paint::Solid(_) => "Solid",
                };
                ui.label(text(Type::Caption, what)).on_hover_text("Picked with the Eyedropper. It stays where it was on the stage (lock fill). Pick a colour to fill with a solid colour again.");
            }
            if !fills {
                crate::theme::checkbox(ui, &mut st.use_fill, "").named("Fill").on_hover_text("Fill on/off");
            }
        });
    }
    if !fills {
        row(ui, "Stroke", |ui| {
            let mut c = rgb(st.stroke);
            if ui.color_edit_button_srgb(&mut c).named("Stroke colour").changed() {
                st.stroke = Rgba::new(c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0);
            }
            if !ink {
                crate::theme::checkbox(ui, &mut st.use_stroke, "").named("Stroke").on_hover_text("Stroke on/off");
            }
            ui.add(egui::DragValue::new(&mut st.stroke_width).suffix(" px").speed(0.1).range(0.0..=100.0).max_decimals(1)).set_name("Stroke width");
        });
        // The width profile new strokes get.
        let current = st.profile();
        if let Some(pick) = profile_picker(ui, ProfileRow::Tool, current.as_ref()) {
            st.set_profile(pick);
        }
    }
    // Merge drawing (ADR 0031) or drawing objects, for the tools that draw shapes.
    if matches!(app.tool, Tool::Rect | Tool::Oval | Tool::Star | Tool::Line | Tool::Pen | Tool::Pencil | Tool::Brush) {
        let key = anicraft_engine::find_command("draw.object_drawing").and_then(|c| c.shortcut).map(crate::platform::shortcut_label).unwrap_or_default();
        row_prose(ui, "Drawing", |ui| {
            crate::theme::checkbox(ui, &mut st.object_drawing, "Object drawing")
                .on_hover_text(format!("On: each shape is an object of its own. Off: shapes cut and merge with the shapes they touch ({key})"));
        });
    }
    app.s.style = st;
    match app.tool {
        Tool::Rect => row(ui, "Corners", |ui| {
            ui.add(egui::DragValue::new(&mut app.rect_radius).suffix(" px").range(0.0..=500.0)).set_name("Corner radius");
        }),
        Tool::Star => row(ui, "Points", |ui| {
            ui.add(egui::DragValue::new(&mut app.star_points).range(3..=32)).set_name("Star points");
        }),
        Tool::PaintBucket => row_prose(ui, "Gap size", |ui| {
            egui::ComboBox::from_id_salt("bucket_gap").width(170.0).selected_text(app.bucket_gap.label()).show_ui(ui, |ui| {
                for g in crate::app::GapSize::ALL {
                    ui.selectable_value(&mut app.bucket_gap, g, g.label());
                }
            })
            .response
            .set_name("Gap size");
        }),
        Tool::Pencil => row(ui, "Mode", |ui| {
            use crate::pathtools::PencilMode;
            for m in [PencilMode::Smooth, PencilMode::Straighten, PencilMode::Ink] {
                ui.selectable_value(&mut app.pencil_mode, m, m.label()).on_hover_text(m.hint());
            }
        }),
        Tool::Brush => brush_props(app, ui),
        _ => {}
    }
}

/// The Eraser's options (ADR 0034): the Faucet, size, shape and erase mode, as in Animate.
fn eraser_props(app: &mut App, ui: &mut egui::Ui) {
    use crate::brushtool::{SIZE_MAX, SIZE_MIN};
    use anicraft_engine::cmds_erase::EraseMode;
    use anicraft_engine::geom::brush::TipShape;
    let keys = [app.keymap.primary("view.brush_smaller"), app.keymap.primary("view.brush_larger")].into_iter().flatten().map(crate::platform::shortcut_label).collect::<Vec<_>>().join(" / ");
    let o = &mut app.eraser;
    row_prose(ui, "Faucet", |ui| {
        crate::theme::checkbox(ui, &mut o.faucet, "Delete whole fills and lines").on_hover_text("On: a click deletes the whole fill or line under it. Off: drag to erase");
    });
    ui.add_enabled_ui(!o.faucet, |ui| {
        row(ui, "Size", |ui| {
            let r = ui.add(egui::DragValue::new(&mut o.size).suffix(" px").speed(0.25).range(SIZE_MIN..=SIZE_MAX).max_decimals(1)).named("Eraser size");
            if !keys.is_empty() {
                r.on_hover_text(format!("Eraser size ({keys})"));
            }
        });
        row(ui, "Shape", |ui| {
            for (shape, label, hint) in [(TipShape::Round, "Round", "A round eraser"), (TipShape::Square, "Square", "A square eraser")] {
                ui.selectable_value(&mut o.shape, shape, label).on_hover_text(hint);
            }
        });
        row_prose(ui, "Mode", |ui| {
            egui::ComboBox::from_id_salt("erase_mode").width(170.0).selected_text(o.mode.label()).show_ui(ui, |ui| {
                for m in EraseMode::ALL {
                    ui.selectable_value(&mut o.mode, m, m.label()).on_hover_text(m.hint());
                }
            })
            .response
            .set_name("Erase mode");
        });
    });
    row_prose(ui, "Stage", |ui| {
        if ui.button("Erase Everything…").on_hover_text("Clears every visible, unlocked layer at this frame, as double-clicking the Eraser tool does. You can undo it").clicked() {
            crate::erasertool::erase_everything(app);
        }
    });
}

/// The Brush tool's options (ADR 0033): what Animate's brush offers in its options.
fn brush_props(app: &mut App, ui: &mut egui::Ui) {
    use crate::brushtool::{SIZE_MAX, SIZE_MIN};
    use anicraft_engine::cmds_brush::PaintMode;
    use anicraft_engine::geom::brush::TipShape;
    let keys = [app.keymap.primary("view.brush_smaller"), app.keymap.primary("view.brush_larger")].into_iter().flatten().map(crate::platform::shortcut_label).collect::<Vec<_>>().join(" / ");
    let b = &mut app.brush;
    row(ui, "Size", |ui| {
        let r = ui.add(egui::DragValue::new(&mut b.size).suffix(" px").speed(0.25).range(SIZE_MIN..=SIZE_MAX).max_decimals(1)).named("Brush size");
        if !keys.is_empty() {
            r.on_hover_text(format!("Brush size ({keys})"));
        }
    });
    row(ui, "Shape", |ui| {
        for (shape, label, hint) in [(TipShape::Round, "Round", "A round or oval tip"), (TipShape::Square, "Square", "A square or flat tip")] {
            ui.selectable_value(&mut b.shape, shape, label).on_hover_text(hint);
        }
    });
    row(ui, "Angle", |ui| {
        ui.add(egui::DragValue::new(&mut b.angle).suffix("°").speed(1.0).range(0.0..=180.0).max_decimals(0)).named("Brush angle").on_hover_text("Turns a flattened tip (clockwise)");
    });
    row(ui, "Roundness", |ui| {
        ui.add(egui::DragValue::new(&mut b.roundness).suffix(" %").speed(0.5).range(5.0..=100.0).max_decimals(0)).named("Brush roundness").on_hover_text("100 %: as wide as it is tall. Less flattens the tip, for calligraphic strokes");
    });
    row(ui, "Smoothing", |ui| {
        ui.add(egui::DragValue::new(&mut b.smoothing).speed(0.5).range(0.0..=100.0).max_decimals(0)).named("Brush smoothing").on_hover_text("How much the stroke is smoothed: 0 follows the hand closely");
    });
    row_prose(ui, "Mode", |ui| {
        egui::ComboBox::from_id_salt("brush_mode").width(170.0).selected_text(b.mode.label()).show_ui(ui, |ui| {
            for m in PaintMode::ALL {
                ui.selectable_value(&mut b.mode, m, m.label()).on_hover_text(m.hint());
            }
        })
        .response
        .set_name("Paint mode");
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

/// Which Width row a profile menu is (each needs its own popup).
#[derive(Clone, Copy, Debug, Hash)]
enum ProfileRow {
    Selection,
    Tool,
}

/// The Width row: a profile menu with a picture of each built-in. Returns the one picked.
fn profile_picker(ui: &mut egui::Ui, which: ProfileRow, current: Option<&anicraft_engine::doc::WidthProfile>) -> Option<anicraft_engine::doc::ProfilePreset> {
    use anicraft_engine::doc::ProfilePreset;
    let preset = match current {
        None => Some(ProfilePreset::Uniform),
        Some(p) => p.preset(),
    };
    let label = preset.map_or_else(|| format!("Custom ({} points)", current.map_or(0, |p| p.points().len())), |p| p.label().to_string());
    let mut picked = None;
    row_prose(ui, "Width", |ui| {
        profile_preview(ui, current, vec2(44.0, 12.0), T.text_dim, None);
        let menu = ui.id().with(which);
        egui::ComboBox::from_id_salt(menu).width(150.0).selected_text(label).show_ui(ui, |ui| {
            for pr in ProfilePreset::ALL {
                let on = preset == Some(pr);
                let r = ui.horizontal(|ui| {
                    let pic = profile_preview(ui, pr.profile().as_ref(), vec2(44.0, 12.0), if on { T.text } else { T.text_dim }, Some(pr.label()));
                    let lab = ui.selectable_label(on, pr.label());
                    pic.clicked() || lab.clicked()
                });
                if r.inner && !on {
                    picked = Some(pr);
                }
            }
        })
        .response
        .on_hover_text("How the stroke's width varies along it. Shape it by hand with the Width tool (U).")
        .set_name("Stroke width profile");
    });
    picked
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
    let doc = &app.s.doc;
    let rows: Vec<LibraryRow> = doc
        .library
        .iter()
        .map(|sym| LibraryRow {
            item: LibraryItem::Symbol(sym.id),
            name: sym.name.clone(),
            detail: format!("{:?} · {}f", sym.kind, sym.timeline.frame_count()),
            kind: Some(sym.kind),
        })
        .chain(doc.bitmaps.iter().map(|b| LibraryRow { item: LibraryItem::Bitmap(b.id), name: b.name.clone(), detail: format!("Bitmap · {}×{}", b.width, b.height), kind: None }))
        .chain(doc.sounds.iter().map(|x| LibraryRow { item: LibraryItem::Sound(x.id), name: x.name.clone(), detail: format!("Sound · {:.1} s", x.seconds()), kind: None }))
        .collect();
    if rows.is_empty() {
        let key = |id: &str| app.keymap.primary(id).map(crate::platform::shortcut_label).unwrap_or_default();
        ui.label(text(Type::Caption, format!("No symbols, images or sounds yet. Select artwork and press {} to make a symbol, or import an image or a sound with {}.", key("modify.convert_to_symbol"), key("file.import"))));
        return;
    }
    ui.label(text(Type::Caption, format!("{} {}", rows.len(), if rows.len() == 1 { "item" } else { "items" })));
    // Sounds go on the active layer's keyframe at the playhead, their span grown to fit.
    let place = |app: &mut App, item: LibraryItem| match item {
        LibraryItem::Symbol(id) => app.run("library.place", json!({"symbol": id.0})),
        LibraryItem::Bitmap(id) => app.run("library.place", json!({"bitmap": id.0})),
        LibraryItem::Sound(id) => app.run("sound.place", json!({"sound": id.0, "extend": true})),
    };
    egui::ScrollArea::vertical().id_salt("lib").max_height(140.0).auto_shrink([false, true]).show(ui, |ui| {
        for row in &rows {
            let selected = app.library_sel == Some(row.item);
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), TARGET), Sense::click());
            if selected || resp.hovered() {
                ui.painter().rect_filled(r, T.rc(), if selected { T.accent_dim } else { T.hover });
            }
            if selected {
                ui.painter().rect_filled(Rect::from_min_size(r.left_top(), vec2(3.0, r.height())), 0.0, T.accent);
            }
            let icon = Rect::from_center_size(r.left_center() + vec2(14.0, 0.0), vec2(14.0, 14.0));
            match (row.kind, row.item) {
                (Some(kind), _) => {
                    let kind_color = match kind {
                        SymbolKind::Graphic => T.kind_graphic,
                        SymbolKind::MovieClip => T.kind_movie_clip,
                        SymbolKind::Button => T.kind_button,
                    };
                    ui.painter().rect_stroke(icon, 2.0, Stroke::new(1.5, kind_color), StrokeKind::Inside);
                    ui.painter().circle_filled(icon.center(), 2.5, kind_color);
                }
                (None, LibraryItem::Sound(_)) => icons::paint(ui.painter(), icon.expand(1.0), Icon::Sound, color(Type::Body)),
                (None, _) => icons::paint(ui.painter(), icon.expand(1.0), Icon::Image, color(Type::Body)),
            }
            ui.painter().text(r.left_center() + vec2(30.0, 0.0), Align2::LEFT_CENTER, &row.name, font(Type::Body), color(Type::Body));
            ui.painter().text(r.right_center() - vec2(8.0, 0.0), Align2::RIGHT_CENTER, &row.detail, font(Type::Caption), color(Type::Caption));
            a11y::control(ui, &resp, WidgetType::SelectableLabel, &format!("{} · {}", row.name, row.detail), Some(selected));
            if resp.clicked() {
                app.library_sel = Some(row.item);
            }
            if resp.double_clicked() {
                place(app, row.item);
            }
            if let LibraryItem::Sound(id) = row.item {
                resp.context_menu(|ui| {
                    if ui.button("Delete Sound").clicked() {
                        ui.close();
                        app.request("sound.delete", json!({"sound": id.0}));
                    }
                });
            }
        }
    });
    ui.add_space(4.0);
    let can_place = app.library_sel.filter(|sel| rows.iter().any(|r| r.item == *sel));
    match can_place {
        Some(LibraryItem::Sound(id)) => {
            ui.horizontal(|ui| {
                if ui.button("Preview").on_hover_text("Plays the sound on its own").clicked()
                    && let Some(dec) = app.s.doc.sound(id).and_then(|x| x.decoded())
                {
                    app.playing = false;
                    app.sound.preview(dec.pcm.clone());
                    if let Some(e) = app.sound.error.take() {
                        app.status_err(format!("No sound: {e}"));
                    }
                }
                if ui.button("Place on timeline").on_hover_text("Puts it on the active layer's keyframe at the playhead (or double-click it)").clicked() {
                    place(app, LibraryItem::Sound(id));
                }
            });
        }
        _ => {
            if ui.add_enabled(can_place.is_some(), egui::Button::new("Place on stage")).on_hover_text("Adds it at the stage centre (or double-click an item)").clicked()
                && let Some(item) = can_place {
                    place(app, item);
                }
        }
    }
}

/// One Library row: a symbol (with its kind), a bitmap or a sound.
struct LibraryRow {
    item: LibraryItem,
    name: String,
    detail: String,
    kind: Option<SymbolKind>,
}
