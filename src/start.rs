//! The start screen (ADR 0059): what AniCraft shows when it starts without a document to open,
//! as Animate's Home screen does. New with stage-size presets (and a custom size), Open, the
//! recent files with their pictures, the sample animation, and the templates (ADR 0065: built-in
//! and the user's own; File › New from Template opens the screen on them). It takes the place of the
//! workspace, never sits over it as a dialog: the menus keep working, Esc goes back to the
//! document, and New, Open or a recent file dismisses it once the document is there.

use crate::a11y::{self, Named};
use crate::app::App;
use crate::icons::{self, Icon};
use crate::theme::{T, TARGET, Type, color, font, text};
use egui::{Align2, Rect, Sense, Stroke, StrokeKind, Vec2, WidgetType, pos2, vec2};
use serde_json::json;

/// A New preset: its name (English, translated where drawn), stage width and height.
pub struct Preset {
    pub name: &'static str,
    pub width: u32,
    pub height: u32,
}

/// The sizes artists start from most: video in HD and Full HD, square and vertical social posts.
pub const PRESETS: [Preset; 4] = [
    Preset { name: tr_noop!("HD video"), width: 1280, height: 720 },
    Preset { name: tr_noop!("Full HD video"), width: 1920, height: 1080 },
    Preset { name: tr_noop!("Square post"), width: 1080, height: 1080 },
    Preset { name: tr_noop!("Vertical story"), width: 1080, height: 1920 },
];

/// The frame rates offered for a new document.
pub const RATES: [f32; 5] = [12.0, 24.0, 25.0, 30.0, 60.0];

/// What the start screen holds between frames: the custom size being typed.
pub struct StartScreen {
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    /// Opened from File › New from Template: scroll to the templates once.
    pub show_templates: bool,
}

impl Default for StartScreen {
    fn default() -> Self {
        let d = anicraft_engine::doc::Stage::default();
        StartScreen { width: d.width, height: d.height, fps: d.fps, show_templates: false }
    }
}

/// `file.new`'s params for a size and rate.
pub fn new_params(width: u32, height: u32, fps: f32) -> serde_json::Value {
    json!({"width": width, "height": height, "fps": fps})
}

/// Shows the start screen (File › Start Screen, and at launch).
pub fn open(app: &mut App) {
    app.playing = false;
    app.start = Some(StartScreen { fps: app.s.doc.stage.fps, ..StartScreen::default() });
    crate::templates::rescan(app);
    app.status_ok(tr!("Pick a size to start a new animation, or open one you made. Esc goes back to the document."));
}

/// The start screen in the space the workspace normally takes.
pub fn show(app: &mut App, ui: &mut egui::Ui) {
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) && !app.modal_open() {
        app.start = None;
        return;
    }
    let ctx = ui.ctx().clone();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let width = ui.available_width().min(1120.0);
        let pad = ((ui.available_width() - width) / 2.0).max(24.0);
        ui.add_space(28.0);
        ui.horizontal(|ui| {
            ui.add_space(pad);
            ui.vertical(|ui| {
                ui.set_width(width - 48.0);
                header(app, ui, &ctx);
                ui.add_space(20.0);
                ui.columns(2, |cols| {
                    if let [left, right] = cols {
                        new_column(app, left);
                        open_column(app, right, &ctx);
                    }
                });
                ui.add_space(20.0);
                let head = crate::templates::section(app, ui, &ctx);
                if let Some(st) = app.start.as_mut().filter(|s| s.show_templates) {
                    st.show_templates = false;
                    ui.scroll_to_rect(head, Some(egui::Align::TOP));
                }
                ui.add_space(16.0);
                footer(app, ui);
            });
        });
        ui.add_space(28.0);
    });
}

fn header(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context) {
    ui.horizontal(|ui| {
        ui.add(egui::Image::new((crate::app::app_mark(ctx).id(), vec2(44.0, 44.0))));
        ui.add_space(8.0);
        ui.vertical(|ui| {
            ui.label(text(Type::Title, "AniCraft")); // i18n-exempt: the product name
            ui.label(text(Type::Caption, tr!("Version {version} · {stage}", version = env!("CARGO_PKG_VERSION"), stage = crate::i18n::t(crate::app::RELEASE_STAGE))));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let back = ui.add(egui::Button::new(tr!("Back to the document")).shortcut_text(text(Type::Key, "Esc"))); // i18n-exempt: a key name
            if back.on_hover_text(tr!("The document that is open stays as it is")).clicked() {
                app.start = None;
            }
        });
    });
}

/// A preset tile: the stage's shape drawn to scale, its name and size.
fn tile(ui: &mut egui::Ui, name: &str, w: u32, h: u32) -> egui::Response {
    let size = vec2(118.0, 112.0);
    let (r, resp) = ui.allocate_exact_size(size, Sense::click());
    let size_text = format!("{w} × {h}");
    a11y::control(ui, &resp, WidgetType::Button, &tr!("New {name}, {size}", name = name, size = size_text), None);
    let p = ui.painter();
    let hot = resp.hovered() || resp.has_focus();
    p.rect_filled(r, T.rc(), if hot { T.hover } else { T.panel_alt });
    p.rect_stroke(r, T.rc(), Stroke::new(1.0, if hot { T.accent } else { T.border }), StrokeKind::Inside);
    // The stage's aspect, in a 64-point box.
    let box_ = Rect::from_center_size(pos2(r.center().x, r.top() + 38.0), vec2(64.0, 52.0));
    let aspect = w.max(1) as f32 / h.max(1) as f32;
    let shape = if aspect >= box_.width() / box_.height() { vec2(box_.width(), box_.width() / aspect) } else { vec2(box_.height() * aspect, box_.height()) };
    p.rect_filled(Rect::from_center_size(box_.center(), shape), crate::theme::SQUARE, T.text_faint);
    p.text(pos2(r.center().x, r.bottom() - 34.0), Align2::CENTER_CENTER, name, font(Type::Label), color(Type::Body));
    p.text(pos2(r.center().x, r.bottom() - 16.0), Align2::CENTER_CENTER, size_text, font(Type::DataSmall), T.text_dim);
    a11y::focus_ring(ui, &resp);
    resp
}

fn new_column(app: &mut App, ui: &mut egui::Ui) {
    ui.label(text(Type::Heading, tr!("Create new")));
    ui.add_space(6.0);
    let fps = app.start.as_ref().map_or(24.0, |s| s.fps);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(10.0, 10.0);
        for p in &PRESETS {
            if tile(ui, crate::i18n::t(p.name), p.width, p.height).clicked() {
                app.request("file.new", new_params(p.width, p.height, fps));
            }
        }
    });
    ui.add_space(12.0);
    ui.label(text(Type::Label, tr!("Custom size")));
    let Some(st) = app.start.as_mut() else { return };
    let max = anicraft_engine::doc::MAX_STAGE_EDGE;
    let mut create = false;
    ui.horizontal(|ui| {
        ui.add(egui::DragValue::new(&mut st.width).range(1..=max).suffix(" px")).named(tr!("Custom width")); // i18n-exempt: a unit
        ui.label(text(Type::Data, "×")); // i18n-exempt: a symbol
        ui.add(egui::DragValue::new(&mut st.height).range(1..=max).suffix(" px")).named(tr!("Custom height")); // i18n-exempt: a unit
        ui.add_space(8.0);
        egui::ComboBox::from_id_salt("start_fps")
            .selected_text(text(Type::Data, tr!("{fps} fps", fps = st.fps)))
            .show_ui(ui, |ui| {
                for r in RATES {
                    ui.selectable_value(&mut st.fps, r, text(Type::Data, tr!("{fps} fps", fps = r)));
                }
            })
            .response
            .set_name(tr!("Frame rate"));
        ui.add_space(8.0);
        create = ui.add(egui::Button::new(tr!("Create")).min_size(vec2(0.0, TARGET))).clicked();
    });
    ui.label(text(Type::Caption, tr!("The frame rate is for every new document made here. Change the size later in Modify › Document Settings.")));
    let (w, h, f) = (st.width, st.height, st.fps);
    if create {
        app.request("file.new", new_params(w, h, f));
    }
}

fn open_column(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context) {
    ui.label(text(Type::Heading, tr!("Open")));
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui.add(egui::Button::new(tr!("Open…")).min_size(vec2(0.0, TARGET))).on_hover_text(tr!("An AniCraft document, or an Animate .fla or .xfl")).clicked() {
            app.invoke("file.open");
        }
        if ui.add(egui::Button::new(tr!("Sample animation")).min_size(vec2(0.0, TARGET))).on_hover_text(tr!("A short animation to look at and take apart")).clicked() {
            app.request("file.new_sample", json!({}));
        }
    });
    ui.label(text(Type::Caption, tr!("Or drop a document anywhere on this window.")));
    ui.add_space(14.0);
    ui.label(text(Type::Heading, tr!("Recent")));
    ui.add_space(6.0);
    let list = crate::recent::entries(app);
    if list.is_empty() {
        ui.label(text(Type::Caption, tr!("Documents you open or save show here, with a picture of their first frame.")));
        return;
    }
    let mut act: Option<(&'static str, serde_json::Value)> = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(10.0, 10.0);
        for (i, e, missing) in &list {
            card(app, ui, ctx, *i, e, *missing, &mut act);
        }
    });
    if let Some((id, params)) = act {
        crate::recent::open_or_run(app, id, params);
    }
}

/// A recent file: its picture, name and folder. A click opens it; a missing file is dimmed with
/// Remove; Pin keeps it first.
fn card(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context, i: usize, e: &anicraft_engine::cmds_recent::RecentFile, missing: bool, act: &mut Option<(&'static str, serde_json::Value)>) {
    let thumb = Vec2::new(anicraft_engine::cmds_recent::THUMB_W as f32 * 0.66, anicraft_engine::cmds_recent::THUMB_H as f32 * 0.66);
    let name = crate::recent::display_name(&e.path);
    ui.vertical(|ui| {
        ui.set_width(thumb.x);
        let (r, resp) = ui.allocate_exact_size(thumb, if missing { Sense::hover() } else { Sense::click() });
        let label = if missing { tr!("{name}, missing", name = name) } else if e.pinned { tr!("Open {name}, pinned", name = name) } else { tr!("Open {name}", name = name) };
        a11y::control(ui, &resp, WidgetType::Button, &label, None);
        let hot = !missing && (resp.hovered() || resp.has_focus());
        let p = ui.painter();
        p.rect_filled(r, T.rc(), T.panel_alt);
        match (!missing).then(|| app.thumbs.get(ctx, &e.path)).flatten() {
            Some(tex) => {
                let s = tex.size_vec2();
                let fit = (r.width() / s.x.max(1.0)).min(r.height() / s.y.max(1.0));
                let img = Rect::from_center_size(r.center(), s * fit);
                p.image(tex.id(), img, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
            }
            None => icons::paint(p, Rect::from_center_size(r.center(), vec2(28.0, 28.0)), Icon::Image, T.text_faint),
        }
        p.rect_stroke(r, T.rc(), Stroke::new(if hot { 1.5 } else { 1.0 }, if hot { T.accent } else { T.border }), StrokeKind::Inside);
        a11y::focus_ring(ui, &resp);
        let tip = e.path.display().to_string();
        let resp = if missing { resp.on_hover_text(tr!("{path} is no longer there", path = tip)) } else { resp.on_hover_text(tip) };
        if resp.clicked() {
            *act = Some(("file.open_recent", json!({"index": i})));
        }
        let c = if missing { T.text_dim } else { color(Type::Body) };
        ui.add(egui::Label::new(text(Type::Body, name.clone()).color(c)).truncate());
        ui.horizontal(|ui| {
            let note = if missing { tr!("Missing").to_string() } else if e.pinned { tr!("Pinned").to_string() } else { crate::recent::folder_name(&e.path) };
            ui.add(egui::Label::new(text(if missing || e.pinned { Type::Caption } else { Type::Path }, note).color(T.text_dim)).truncate());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if missing {
                    if ui.small_button(tr!("Remove")).on_hover_text(tr!("Take it off the list")).clicked() {
                        *act = Some(("prefs.recent_remove", json!({"index": i})));
                    }
                } else if ui.small_button(if e.pinned { tr!("Unpin") } else { tr!("Pin") }).on_hover_text(if e.pinned { tr!("Let it move down the list again") } else { tr!("Keep it at the top of the list") }).clicked() {
                    *act = Some(("prefs.recent_pin", json!({"index": i})));
                }
            });
        });
    });
}

fn footer(app: &mut App, ui: &mut egui::Ui) {
    ui.separator();
    ui.horizontal(|ui| {
        let mut on = app.prefs.start_screen;
        if crate::theme::checkbox(ui, &mut on, tr!("Show this screen when AniCraft starts")).changed() {
            let mut p = app.prefs.clone();
            p.start_screen = on;
            let ctx = ui.ctx().clone();
            app.set_prefs(&ctx, p);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(text(Type::Caption, tr!("File › Start Screen brings it back")));
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use anicraft_engine::Session;

    fn app() -> (App, egui::Context) {
        let ctx = egui::Context::default();
        let app = App::with_context(&ctx, Session::new(anicraft_engine::doc::sample::demo()), Some(std::path::PathBuf::from("unused.png")));
        (app, ctx)
    }

    #[test]
    fn every_preset_makes_a_document_of_its_size() {
        let (mut app, _) = app();
        for p in &PRESETS {
            for fps in [24.0, 30.0] {
                crate::start::open(&mut app);
                app.request("file.new", new_params(p.width, p.height, fps));
                let st = app.s.doc.stage;
                assert_eq!((st.width, st.height, st.fps), (p.width, p.height, fps), "{}", p.name);
                assert!(app.start.is_none(), "New dismisses the start screen");
            }
        }
        let names: Vec<(u32, u32)> = PRESETS.iter().map(|p| (p.width, p.height)).collect();
        for want in [(1280, 720), (1920, 1080), (1080, 1080)] {
            assert!(names.contains(&want), "{want:?} is a preset");
        }
        // A custom size, and one past the limit that the engine refuses (the screen stays).
        crate::start::open(&mut app);
        app.request("file.new", new_params(640, 480, 12.0));
        assert_eq!((app.s.doc.stage.width, app.s.doc.stage.height, app.s.doc.stage.fps), (640, 480, 12.0));
        crate::start::open(&mut app);
        app.request("file.new", new_params(anicraft_engine::doc::MAX_STAGE_EDGE + 1, 10, 24.0));
        assert!(app.start.is_some(), "a refused New leaves the start screen up");
    }

    #[test]
    fn over_unsaved_work_new_asks_first_and_esc_goes_back() {
        let (mut app, ctx) = app();
        app.run("draw.rect", json!({"x": 0, "y": 0, "w": 10, "h": 10}));
        crate::start::open(&mut app);
        app.request("file.new", new_params(1920, 1080, 24.0));
        assert!(app.confirm.is_some(), "unsaved work asks first");
        assert!(app.start.is_some(), "and the start screen waits for the answer");
        app.confirm = None;
        // Esc returns to the work, untouched.
        let input = egui::RawInput { events: vec![egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }], ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| app.frame(ui));
        out.textures_delta.clear();
        assert!(app.start.is_none());
        assert!(app.s.is_dirty(), "the work is still there");
    }
}
