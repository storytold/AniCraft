//! Templates in the window (ADR 0065): the Templates row of the start screen (the built-in
//! templates and the user's own, each with its picture), which File › New from Template opens
//! on, and the File › Save as Template dialog. The commands do the work
//! (`file.new_from_template`, `file.save_as_template`, `file.delete_template`); the user's
//! templates live in the settings folder (`platform::templates_dir`).

use crate::a11y::{self, Named};
use crate::app::App;
use crate::icons::{self, Icon};
use crate::theme::{FORM_LABEL_DOCK, T, Type, form_row, text};
use anicraft_engine::cmds_template::{self, UserTemplate};
use anicraft_engine::doc::templates::{TEMPLATES, Template};
use egui::{Align2, Rect, Sense, Stroke, StrokeKind, Vec2, WidgetType, pos2, vec2};
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The built-in templates' names, categories and descriptions, for the catalogs: the engine's
/// English is the key, translated where it is drawn.
pub const TEXTS: [&str; 17] = [
    tr_noop!("Leaderboard banner"),
    tr_noop!("Rectangle banner"),
    tr_noop!("Social square"),
    tr_noop!("Storyboard 16:9"),
    tr_noop!("Character turnaround"),
    tr_noop!("Lip-sync mouth chart"),
    tr_noop!("Advertising"),
    tr_noop!("Social"),
    tr_noop!("Storyboard"),
    tr_noop!("Character"),
    tr_noop!("A 728 × 90 web banner at 24 fps, 15 seconds long, with its safe area on a guide layer"),
    tr_noop!("A 300 × 250 web banner at 24 fps, 15 seconds long, with its safe area on a guide layer"),
    tr_noop!("A 1080 × 1080 post at 30 fps, 15 seconds long, with a safe margin and thirds on a guide layer"),
    tr_noop!("Six 2-second panels at 1920 × 1080, labelled Panel 1 to 6, with action- and title-safe areas"),
    tr_noop!("Front, three-quarter, side and back views side by side, on head-height guides eight heads tall"),
    tr_noop!("A Mouth graphic symbol with ten labelled mouth shapes to redraw, placed on a face guide"),
    tr_noop!("My Templates"),
];

/// The folder the user's templates are in. Screenshot and test runs never touch the real one:
/// they use a temporary folder (unless `ANICRAFT_TEMPLATES_DIR` names one).
pub fn folder(app: &App) -> Option<PathBuf> {
    if app.shot.is_some() && std::env::var_os("ANICRAFT_TEMPLATES_DIR").is_none() && !crate::platform::WEB {
        return Some(std::env::temp_dir().join(format!("anicraft-templates-shot-{}", std::process::id())));
    }
    crate::platform::templates_dir()
}

/// A built-in template's text in the interface language.
fn t(s: &'static str) -> &'static str {
    if TEXTS.contains(&s) { crate::i18n::t(s) } else { s }
}

/// A tile's picture size.
const THUMB: Vec2 = Vec2::new(168.0, 95.0);

/// What the window keeps for templates between frames.
#[derive(Default)]
pub struct Templates {
    /// Pictures, by built-in id or user template path (`None`: couldn't be made).
    thumbs: HashMap<String, Option<egui::TextureHandle>>,
    /// The user's templates, as last read from the folder.
    pub user: Vec<UserTemplate>,
    /// The Save as Template dialog, while it is open.
    pub save: Option<SaveDialog>,
    /// Pictures made in memory (screenshot runs show sample user templates without a folder).
    pub(crate) seeded: HashMap<String, Vec<u8>>,
}

/// The Save as Template dialog.
pub struct SaveDialog {
    pub name: String,
    pub category: String,
    focus: bool,
    /// The document's picture, made on the dialog's first frame (`None` inside: none to show).
    picture: Option<Option<egui::TextureHandle>>,
}

fn texture(ctx: &egui::Context, key: &str, png: &[u8]) -> Option<egui::TextureHandle> {
    let px = anicraft_engine::doc::decode_png(png)?;
    let (w, h) = (usize::try_from(px.width).ok()?, usize::try_from(px.height).ok()?);
    (w > 0 && h > 0 && px.rgba.len() == w * h * 4).then(|| ctx.load_texture(format!("template-{key}"), egui::ColorImage::from_rgba_unmultiplied([w, h], &px.rgba), egui::TextureOptions::LINEAR))
}

/// A document's picture as a template shows it: its first frame, guides drawn.
fn picture_png(doc: &anicraft_engine::doc::Document) -> Option<Vec<u8>> {
    cmds_template::picture_png(doc).ok()
}

impl Templates {
    /// A built-in template's picture, rendered by the CPU oracle once.
    fn builtin(&mut self, ctx: &egui::Context, t: &Template) -> Option<egui::TextureHandle> {
        self.thumbs.entry(t.id.to_string()).or_insert_with(|| picture_png(&(t.build)()).and_then(|png| texture(ctx, t.id, &png))).clone()
    }

    /// A user template's picture, read from beside it once.
    fn user_picture(&mut self, ctx: &egui::Context, t: &UserTemplate) -> Option<egui::TextureHandle> {
        let key = t.path.display().to_string();
        if let Some(tex) = self.thumbs.get(&key) {
            return tex.clone();
        }
        let png = self.seeded.get(&key).cloned().or_else(|| t.thumbnail.as_deref().and_then(|p| std::fs::read(p).ok()));
        let tex = png.and_then(|png| texture(ctx, &key, &png));
        self.thumbs.insert(key, tex.clone());
        tex
    }

    /// Reads the user's templates again (when the start screen opens, after a save or delete).
    pub fn rescan(&mut self, folder: Option<&Path>) {
        if !self.seeded.is_empty() {
            return; // sample entries (screenshots)
        }
        self.user = folder.map(cmds_template::user_templates).unwrap_or_default();
        // Pictures may have changed under the same names.
        self.thumbs.retain(|k, _| TEMPLATES.iter().any(|t| t.id == k));
    }
}

/// A template's tile: its picture, name, and category with size. `None` size: unknown.
fn tile(ui: &mut egui::Ui, tex: Option<egui::TextureHandle>, name: &str, note: &str, size: Option<String>, tip: &str) -> egui::Response {
    ui.vertical(|ui| {
        ui.set_width(THUMB.x);
        ui.spacing_mut().item_spacing.y = 2.0;
        let (r, resp) = ui.allocate_exact_size(THUMB, Sense::click());
        a11y::control(ui, &resp, WidgetType::Button, &tr!("New from template {name}", name = name), None);
        let hot = resp.hovered() || resp.has_focus();
        let p = ui.painter();
        p.rect_filled(r, T.rc(), T.panel_alt);
        match tex {
            Some(tex) => {
                let s = tex.size_vec2();
                let fit = ((r.width() - 8.0) / s.x.max(1.0)).min((r.height() - 8.0) / s.y.max(1.0));
                p.image(tex.id(), Rect::from_center_size(r.center(), s * fit), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
            }
            None => icons::paint(p, Rect::from_center_size(r.center(), vec2(28.0, 28.0)), Icon::Image, T.text_faint),
        }
        p.rect_stroke(r, T.rc(), Stroke::new(if hot { 1.5 } else { 1.0 }, if hot { T.accent } else { T.border }), StrokeKind::Inside);
        a11y::focus_ring(ui, &resp);
        ui.add(egui::Label::new(text(Type::Body, name)).truncate());
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(text(Type::Caption, note).color(T.text_dim)).truncate());
            if let Some(size) = size {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(text(Type::DataSmall, size).color(T.text_dim)));
            }
        });
        resp.on_hover_text(tip)
    })
    .inner
}

/// The Templates row of the start screen. Returns the heading's rect (File › New from Template
/// scrolls to it).
pub fn section(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context) -> Rect {
    let head = ui.label(text(Type::Heading, tr!("Templates"))).rect;
    ui.add_space(6.0);
    let mut act: Option<(&'static str, serde_json::Value)> = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(10.0, 10.0);
        for t in &TEMPLATES {
            let d = (t.build)();
            let size = format!("{} × {}", d.stage.width, d.stage.height);
            let tex = app.templates.builtin(ctx, t);
            let tip = tr!("{description} · {fps} fps", description = self::t(t.description), fps = d.stage.fps);
            if tile(ui, tex, self::t(t.name), self::t(t.category), Some(size), &tip).clicked() {
                act = Some(("file.new_from_template", json!({"template": t.id})));
            }
        }
    });
    ui.add_space(12.0);
    ui.label(text(Type::Label, tr!("My templates")));
    ui.add_space(4.0);
    let user = app.templates.user.clone();
    if user.is_empty() {
        let hint = if folder(app).is_some() { tr!("File › Save as Template keeps the open document here, with a picture, to start from again.") } else { tr!("Your own templates are kept in a folder on your computer: save them in the desktop app.") };
        ui.label(text(Type::Caption, hint));
    } else {
        let folder = folder(app);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(10.0, 10.0);
            for t in &user {
                ui.vertical(|ui| {
                    let tex = app.templates.user_picture(ctx, t);
                    if tile(ui, tex, &t.name, &t.category, None, &t.path.display().to_string()).clicked() {
                        act = Some(("file.new_from_template", json!({"path": t.path.display().to_string()})));
                    }
                    if let Some(f) = &folder
                        && ui.small_button(tr!("Delete")).named(&tr!("Delete template {name}", name = t.name)).on_hover_text(tr!("Delete this template and its picture (asks first)")).clicked()
                    {
                        act = Some(("file.delete_template", json!({"folder": f.display().to_string(), "category": t.category, "name": t.name})));
                    }
                });
            }
        });
    }
    if let Some((id, params)) = act {
        app.request(id, params);
    }
    head
}

/// File › Save as Template: the dialog, with the open document's picture, a name and a
/// category.
pub fn open_save(app: &mut App) {
    if folder(app).is_none() {
        app.status_err(crate::i18n::t(crate::platform::unavailable("file.save_as_template").unwrap_or(tr_noop!("There is no settings folder to keep templates in on this system"))));
        return;
    }
    app.playing = false;
    rescan(app);
    let name = app.s.path.as_deref().and_then(Path::file_stem).map_or_else(|| tr!("Untitled").to_string(), |n| n.to_string_lossy().into_owned());
    app.templates.save = Some(SaveDialog { name, category: self::t(cmds_template::DEFAULT_CATEGORY).to_string(), focus: true, picture: None });
}

/// Reads the user's templates again.
pub fn rescan(app: &mut App) {
    let f = folder(app);
    app.templates.rescan(f.as_deref());
}

/// The categories the user has saved templates in, and the default, each once.
fn categories(user: &[UserTemplate]) -> Vec<String> {
    let mut out = vec![self::t(cmds_template::DEFAULT_CATEGORY).to_string()];
    for t in user {
        if !out.iter().any(|c| c.eq_ignore_ascii_case(&t.category)) {
            out.push(t.category.clone());
        }
    }
    out
}

pub fn save_window(app: &mut App, ctx: &egui::Context) {
    let Some(folder) = folder(app) else {
        app.templates.save = None;
        return;
    };
    let cats = categories(&app.templates.user);
    let doc = &app.s.doc;
    let Some(d) = &mut app.templates.save else { return };
    if d.picture.is_none() {
        d.picture = Some(picture_png(doc).and_then(|png| texture(ctx, "save-preview", &png)));
    }
    let (mut submit, mut cancel) = (false, false);
    egui::Window::new(tr!("Save as Template")).id(egui::Id::new("save_template")).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, -60.0]).show(ctx, |ui| {
        ui.set_min_width(420.0);
        if let Some(Some(tex)) = &d.picture {
            let s = tex.size_vec2();
            let fit = (240.0 / s.x.max(1.0)).min(135.0 / s.y.max(1.0));
            ui.vertical_centered(|ui| {
                let r = ui.add(egui::Image::new((tex.id(), s * fit))).rect;
                ui.painter().rect_stroke(r, crate::theme::SQUARE, Stroke::new(1.0, T.border), StrokeKind::Outside);
            });
            ui.add_space(6.0);
        }
        form_row(ui, FORM_LABEL_DOCK, tr!("Name"), |ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut d.name).desired_width(f32::INFINITY)).named(tr!("Template name"));
            if d.focus {
                r.request_focus();
                d.focus = false;
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                submit = true;
            }
        });
        form_row(ui, FORM_LABEL_DOCK, tr!("Category"), |ui| {
            ui.add(egui::TextEdit::singleline(&mut d.category).desired_width(f32::INFINITY)).named(tr!("Template category"));
        });
        ui.horizontal_wrapped(|ui| {
            ui.add_space(FORM_LABEL_DOCK);
            for c in &cats {
                if ui.selectable_label(d.category.trim().eq_ignore_ascii_case(c), c.as_str()).on_hover_text(tr!("Save it in this category")).clicked() {
                    d.category = c.clone();
                }
            }
        });
        // Where it goes, and what it replaces.
        let target = cmds_template::template_path(&folder, d.category.trim(), d.name.trim());
        match &target {
            Ok(p) if p.is_file() => ui.label(text(Type::Caption, tr!("A template called “{name}” is in this category already: saving asks to replace it.", name = d.name.trim())).color(T.accent_text)),
            Ok(p) => ui.label(text(Type::Path, p.display().to_string()).color(T.text_dim)),
            Err(e) => ui.label(text(Type::Caption, crate::i18n::t_dyn(&e.to_string()).into_owned()).color(T.error)),
        };
        ui.label(text(Type::Caption, tr!("The template keeps the document as it is now. The open document doesn't change.")));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(target.is_ok(), egui::Button::new(tr!("Save Template"))).clicked() {
                submit = true;
            }
            if ui.button(tr!("Cancel")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                cancel = true;
            }
        });
    });
    if submit {
        if let Some(d) = app.templates.save.take() {
            app.request("file.save_as_template", json!({"folder": folder.display().to_string(), "name": d.name.trim(), "category": d.category.trim()}));
        }
    } else if cancel {
        app.templates.save = None;
    }
}

/// After a template command: the status line and the list (called from `App::announce`).
pub fn announce(app: &mut App, id: &str, v: &serde_json::Value) {
    match id {
        "file.save_as_template" => {
            rescan(app);
            let name = v["path"].as_str().map(PathBuf::from).and_then(|p| p.file_stem().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default();
            app.status_ok(tr!("Saved the template “{name}”. File › New from Template starts a document from it.", name = name));
        }
        "file.delete_template" => {
            rescan(app);
            app.status_ok(tr!("Template deleted"));
        }
        _ => {}
    }
}

/// Sample user templates for screenshots (pictures made in memory, no folder touched).
pub fn sample_user(app: &mut App) {
    let samples = [(tr_noop!("Walk cycle"), tr_noop!("Characters"), anicraft_engine::doc::sample::demo()), (tr_noop!("Title card"), cmds_template::DEFAULT_CATEGORY, anicraft_engine::doc::templates::find("social-square").map_or_else(Default::default, |t| (t.build)()))];
    app.templates.user.clear();
    for (name, category, doc) in samples {
        let path = PathBuf::from(format!("{category}/{name}.anicraft"));
        if let Some(png) = picture_png(&doc) {
            app.templates.seeded.insert(path.display().to_string(), png);
        }
        app.templates.user.push(UserTemplate { name: crate::i18n::t(name).to_string(), category: crate::i18n::t(category).to_string(), path, thumbnail: None });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_built_in_templates_text_is_in_the_catalog_list() {
        for t in &TEMPLATES {
            for s in [t.name, t.category, t.description] {
                assert!(TEXTS.contains(&s), "{s:?} is listed for translation");
            }
        }
        assert!(TEXTS.contains(&cmds_template::DEFAULT_CATEGORY));
    }

    #[test]
    fn a_template_tile_opens_a_new_document_from_it() {
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, anicraft_engine::Session::new(anicraft_engine::doc::sample::demo()), Some(PathBuf::from("unused.png")));
        crate::start::open(&mut app);
        app.request("file.new_from_template", json!({"template": "banner-300x250"}));
        assert!(app.start.is_none(), "the start screen goes once the document is there");
        assert_eq!((app.s.doc.stage.width, app.s.doc.stage.height), (300, 250));
        assert!(app.status.as_ref().is_some_and(|s| !s.error && s.text.contains("Rectangle banner")), "{:?}", app.status.as_ref().map(|s| &s.text));
        // Over unsaved work it asks first.
        app.run("draw.rect", json!({"x": 0, "y": 0, "w": 10, "h": 10}));
        app.request("file.new_from_template", json!({"template": "storyboard"}));
        assert!(app.confirm.is_some());
        // The section draws every built-in template with its picture.
        app.confirm = None;
        crate::start::open(&mut app);
        sample_user(&mut app);
        let screen = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1600.0, 940.0))), ..Default::default() };
        let mut out = ctx.run_ui(screen, |ui| app.frame(ui));
        out.textures_delta.clear();
        assert_eq!(TEMPLATES.iter().filter(|t| app.templates.thumbs.get(t.id).is_some_and(Option::is_some)).count(), TEMPLATES.len());
    }
}
