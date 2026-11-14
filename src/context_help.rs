//! Context help (ADR 0096): F1, or Help › Help for This, opens the user guide at the section
//! for whatever the person is working with: the dialog in front, the tool or panel under the
//! pointer, the panel that has the keyboard, else the tool in hand. Each panel's ☰ menu has
//! Help for its own section, and F1 in the command palette opens the chosen command's.
//!
//! The map from tools, panels, dialogs and command namespaces to the guide is here, written as
//! chapter links (`timeline.md#onion-skin`) so the guide's own link check proves every one of
//! them names a heading that exists (`anicraft_guide::check_targets`); tests also require one
//! for every tool, panel, dialog and command namespace, so a new one can't ship without help.

use crate::app::{App, Tool};
use crate::dock::Panel;

/// The guide's place for a tool.
pub fn tool_topic(t: Tool) -> &'static str {
    match t {
        Tool::Select => "drawing.md#the-tools",
        Tool::Subselect => "drawing.md#editing-points-with-subselection",
        Tool::Transform => "drawing.md#transforming-artwork",
        Tool::Warp => "masks-guides-camera-bones.md#asset-warp",
        Tool::GradientTransform => "drawing.md#gradients-and-bitmap-fills",
        Tool::Width => "drawing.md#variable-width-lines",
        Tool::Bone => "masks-guides-camera-bones.md#building-an-armature",
        Tool::Bind => "masks-guides-camera-bones.md#bones-inside-a-shape",
        Tool::Pen => "drawing.md#the-pen",
        Tool::Text => "text.md#typing-text",
        Tool::Pencil | Tool::Rect | Tool::Oval | Tool::Star | Tool::Line => "drawing.md#drawing-shapes-and-lines",
        Tool::Brush | Tool::PaintBrush => "drawing.md#the-brush",
        Tool::Eraser => "drawing.md#eraser",
        Tool::PaintBucket | Tool::InkBottle | Tool::Eyedropper => "drawing.md#paint-bucket-ink-bottle-and-eyedropper",
        Tool::Camera => "masks-guides-camera-bones.md#moving-the-camera",
        Tool::Hand => "workspace.md#panning",
        Tool::Zoom => "workspace.md#zooming",
    }
}

/// The guide's place for a panel.
pub fn panel_topic(p: Panel) -> &'static str {
    match p {
        Panel::Properties => "workspace.md#properties",
        Panel::Align => "drawing.md#align-and-distribute",
        Panel::Colour => "drawing.md#choosing-colours",
        Panel::Library => "symbols-and-library.md#the-library-panel",
        Panel::MovieExplorer => "workspace.md#the-movie-explorer",
        Panel::History => "workspace.md#undo-and-history",
        Panel::LayerDepth => "masks-guides-camera-bones.md#layer-depth",
        Panel::Timeline => "timeline.md#a-tour-of-the-timeline",
        Panel::MotionEditor => "tweens.md#the-motion-editor",
        Panel::Actions => "interactivity.md#the-actions-panel",
        Panel::Output => "workspace.md#colour-and-output",
        Panel::Swatches => "drawing.md#swatches",
        Panel::Tools => "drawing.md#the-tools",
    }
}

/// A window or dialog with a place in the guide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialog {
    Preferences,
    Shortcuts,
    Licenses,
    About,
    ProjectFiles,
    PublishSettings,
    PublishProgress,
    FindReplace,
    BrushLibrary,
    ConvertSymbol,
    LayerProperties,
    EditGrid,
    EditGuides,
    Plugins,
    PluginSettings,
    PluginExport,
    SaveTemplate,
    TestMovie,
    EaseEditor,
    SoundEnvelope,
    Recover,
    Updates,
    ManageWorkspaces,
    Palette,
    CloseUnsaved,
}

impl Dialog {
    pub const ALL: [Dialog; 25] = [
        Dialog::Preferences,
        Dialog::Shortcuts,
        Dialog::Licenses,
        Dialog::About,
        Dialog::ProjectFiles,
        Dialog::PublishSettings,
        Dialog::PublishProgress,
        Dialog::FindReplace,
        Dialog::BrushLibrary,
        Dialog::ConvertSymbol,
        Dialog::LayerProperties,
        Dialog::EditGrid,
        Dialog::EditGuides,
        Dialog::Plugins,
        Dialog::PluginSettings,
        Dialog::PluginExport,
        Dialog::SaveTemplate,
        Dialog::TestMovie,
        Dialog::EaseEditor,
        Dialog::SoundEnvelope,
        Dialog::Recover,
        Dialog::Updates,
        Dialog::ManageWorkspaces,
        Dialog::Palette,
        Dialog::CloseUnsaved,
    ];

    /// The id its window is drawn under (`egui::Window::id`), for the ones that are windows.
    pub fn window_id(self) -> Option<&'static str> {
        Some(match self {
            Dialog::Preferences => "prefs_window",
            Dialog::Shortcuts => "shortcuts_window",
            Dialog::Licenses => "licenses_window",
            Dialog::About => "about_window",
            Dialog::ProjectFiles => "files_window",
            Dialog::PublishSettings => "publish_settings",
            Dialog::PublishProgress => "publish_progress",
            Dialog::FindReplace => "find_replace",
            Dialog::BrushLibrary => "brush_library",
            Dialog::ConvertSymbol => "convert_symbol",
            Dialog::LayerProperties => "layer_props",
            Dialog::EditGrid => "edit_grid",
            Dialog::EditGuides => "edit_guides",
            Dialog::Plugins => "plugins_window",
            Dialog::PluginExport => "plugin_export",
            Dialog::SaveTemplate => "save_template",
            Dialog::TestMovie => "test_movie",
            Dialog::EaseEditor => "ease_editor",
            Dialog::SoundEnvelope => "envelope_editor",
            Dialog::Recover => "recover_window",
            Dialog::ManageWorkspaces => "manage_workspaces",
            Dialog::Updates | Dialog::PluginSettings | Dialog::Palette | Dialog::CloseUnsaved => return None,
        })
    }

    /// The guide's place for it.
    pub fn topic(self) -> &'static str {
        match self {
            Dialog::Preferences => "preferences-shortcuts-accessibility.md#preferences",
            Dialog::Shortcuts => "preferences-shortcuts-accessibility.md#keyboard-shortcuts",
            Dialog::Licenses => "troubleshooting-and-faq.md#what-is-the-licence",
            Dialog::About => "preferences-shortcuts-accessibility.md#about",
            Dialog::ProjectFiles => "index.md#about-this-guide",
            Dialog::PublishSettings => "exporting-and-publish.md#publish-settings",
            Dialog::PublishProgress => "exporting-and-publish.md#publish",
            Dialog::FindReplace => "workspace.md#find-and-replace",
            Dialog::BrushLibrary => "drawing.md#the-brush",
            Dialog::ConvertSymbol => "symbols-and-library.md#making-a-symbol",
            Dialog::LayerProperties => "timeline.md#layer-properties",
            Dialog::EditGrid | Dialog::EditGuides => "drawing.md#rulers-grid-and-guides",
            Dialog::Plugins => "plugins.md#managing-plug-ins",
            Dialog::PluginSettings => "plugins.md#the-settings-dialog",
            Dialog::PluginExport => "plugins.md#exporting-with-a-plug-in",
            Dialog::SaveTemplate => "getting-started.md#the-start-screen",
            Dialog::TestMovie => "interactivity.md#test-movie",
            Dialog::EaseEditor => "tweens.md#custom-eases",
            Dialog::SoundEnvelope => "images-and-sound.md#effects-and-the-envelope",
            Dialog::Recover => "getting-started.md#autosave-and-crash-recovery",
            Dialog::Updates => "getting-started.md#install-anicraft",
            Dialog::ManageWorkspaces => "workspace.md#workspaces",
            Dialog::Palette => "workspace.md#the-command-palette",
            Dialog::CloseUnsaved => "getting-started.md#save-your-work",
        }
    }
}

/// Commands by id prefix, the longest that fits winning: a namespace (`layer.`) or a family
/// within one (`file.export`). Every namespace in the registry has a row (checked by test).
pub const COMMAND_TOPICS: &[(&str, &str)] = &[
    ("actions.", "interactivity.md#the-actions-panel"),
    ("align.", "drawing.md#align-and-distribute"),
    ("arrange.", "drawing.md#stacking-order"),
    ("bone.", "masks-guides-camera-bones.md#bones-and-inverse-kinematics"),
    ("brush.", "drawing.md#the-brush"),
    ("camera.", "masks-guides-camera-bones.md#the-camera"),
    ("control.", "interactivity.md#test-movie"),
    ("desktop.", "getting-started.md#running-the-appimage"),
    ("doc.", "workspace.md#properties"),
    ("draw.", "drawing.md#drawing-shapes-and-lines"),
    ("ease.", "tweens.md#easing"),
    ("edit.", "drawing.md#clipboard-and-duplicates"),
    ("edit.undo", "getting-started.md#undo-anything"),
    ("edit.redo", "getting-started.md#undo-anything"),
    ("element.", "drawing.md#transforming-artwork"),
    ("explorer.", "workspace.md#the-movie-explorer"),
    ("file.", "getting-started.md#save-your-work"),
    ("file.new", "getting-started.md#the-start-screen"),
    ("file.export", "exporting-and-publish.md#exporting-one-format"),
    ("file.import", "images-and-sound.md#importing-images"),
    ("file.publish", "exporting-and-publish.md#publish"),
    ("file.recovery", "getting-started.md#autosave-and-crash-recovery"),
    ("filter.", "drawing.md#filters-and-blend-modes"),
    ("find.", "workspace.md#find-and-replace"),
    ("frames.", "timeline.md#frames-and-keyframes"),
    ("instance.", "symbols-and-library.md#instance-properties"),
    ("layer.", "timeline.md#layers"),
    ("library.", "symbols-and-library.md#the-library-panel"),
    ("modify.", "drawing.md#transforming-artwork"),
    ("modify.convert_to_symbol", "symbols-and-library.md#making-a-symbol"),
    ("motion.", "tweens.md#motion-tweens"),
    ("paint.", "drawing.md#paint-bucket-ink-bottle-and-eyedropper"),
    ("path.", "drawing.md#editing-points-with-subselection"),
    ("plugin.", "plugins.md#running-a-plug-in"),
    ("prefs.", "preferences-shortcuts-accessibility.md#preferences"),
    ("publish.", "exporting-and-publish.md#publish-settings"),
    ("raw.", "drawing.md#merge-drawing-and-drawing-objects"),
    ("scene.", "timeline.md#scenes"),
    ("script.", "interactivity.md#frame-scripts-in-actionscript-30"),
    ("sound.", "images-and-sound.md#sounds-on-keyframes"),
    ("stroke.", "drawing.md#line-styles"),
    ("swatch.", "drawing.md#swatches"),
    ("symbol.", "symbols-and-library.md#making-a-symbol"),
    ("text.", "text.md#typing-text"),
    ("timeline.", "timeline.md#frames-and-keyframes"),
    ("tween.", "tweens.md#which-tween-should-i-use"),
    ("view.", "workspace.md#moving-around-the-stage"),
    ("view.grid", "drawing.md#rulers-grid-and-guides"),
    ("view.guide", "drawing.md#rulers-grid-and-guides"),
    ("view.onion", "timeline.md#onion-skin"),
    ("warp.", "masks-guides-camera-bones.md#asset-warp"),
];

/// Where the command reference lists every command, for one without a row.
pub const COMMAND_REFERENCE: &str = "reference/commands.md";

/// The guide's place for a command.
pub fn command_topic(id: &str) -> &'static str {
    COMMAND_TOPICS.iter().filter(|(prefix, _)| id.starts_with(prefix)).max_by_key(|(prefix, _)| prefix.len()).map_or(COMMAND_REFERENCE, |(_, t)| t)
}

/// The book's page for a chapter link: `timeline.md#onion-skin` → `timeline.html#onion-skin`
/// (the book's own rule, `anicraft_guide::target_html`; the web build doesn't link the book).
pub fn page_of(target: &str) -> String {
    let html = |f: &str| f.strip_suffix(".md").map_or_else(|| f.to_string(), |stem| format!("{stem}.html"));
    match target.split_once('#') {
        Some((file, anchor)) => format!("{}#{anchor}", html(file)),
        None => html(target),
    }
}

/// The book's search page.
pub const SEARCH_PAGE: &str = "search.html";

/// The heading a chapter link names, for the status bar. The desktop carries the book; the web
/// build serves it beside the editor instead, so it doesn't know the headings.
pub fn heading_of(target: &str) -> Option<String> {
    #[cfg(not(target_arch = "wasm32"))]
    return anicraft_guide::heading_of(anicraft_guide::PAGES, target);
    #[cfg(target_arch = "wasm32")]
    {
        let _ = target;
        None
    }
}

/// What context help is about.
#[derive(Clone, Debug, PartialEq)]
pub enum Subject {
    Tool(Tool),
    Panel(Panel),
    Dialog(Dialog),
    Command(String),
    /// A place in the guide given directly (a plug-in, a palette entry without a command).
    Topic(&'static str),
}

impl Subject {
    pub fn topic(&self) -> &'static str {
        match self {
            Subject::Tool(t) => tool_topic(*t),
            Subject::Panel(p) => panel_topic(*p),
            Subject::Dialog(d) => d.topic(),
            Subject::Command(id) => command_topic(id),
            Subject::Topic(t) => t,
        }
    }
}

/// A subject a widget offers while the pointer or keyboard is on it this pass (a tool's button),
/// read by [`subject`] in the next pass's shortcut handling.
#[derive(Clone, Debug)]
pub struct Offered {
    pub pass: u64,
    pub subject: Subject,
}

/// Called by a widget the person is on (hovered or focused): F1 is about it.
pub fn offer(app: &mut App, ctx: &egui::Context, subject: Subject) {
    app.help_offered = Some(Offered { pass: ctx.cumulative_pass_nr(), subject });
}

/// The dialog drawn on `layer`, if it is one of ours.
fn dialog_on(layer: egui::LayerId) -> Option<Dialog> {
    Dialog::ALL.into_iter().find(|d| d.window_id().is_some_and(|w| layer.id == egui::Id::new(w)))
}

fn visible(ctx: &egui::Context, d: Dialog) -> bool {
    d.window_id().is_some_and(|w| ctx.memory(|m| m.areas().is_visible(&egui::LayerId::new(egui::Order::Middle, egui::Id::new(w)))))
}

/// What F1 is about now. `pointer`: whether the pointer counts (not when the request comes from
/// a menu, which the pointer is on).
pub fn subject(app: &App, ctx: &egui::Context, pointer: bool) -> Subject {
    // A dialog that holds the editor until it is answered.
    if let Some(c) = &app.confirm {
        return Subject::Command(c.id.clone());
    }
    if let Some(p) = &app.prompt {
        return Subject::Command(p.cmd.to_string());
    }
    if app.convert.is_some() {
        return Subject::Dialog(Dialog::ConvertSymbol);
    }
    if app.plugin_dialog.is_some() {
        return Subject::Dialog(Dialog::PluginSettings);
    }
    if app.templates.save.is_some() {
        return Subject::Dialog(Dialog::SaveTemplate);
    }
    if app.close_dialog {
        return Subject::Dialog(Dialog::CloseUnsaved);
    }
    if let Some(p) = &app.palette {
        return p.help.clone().unwrap_or(Subject::Dialog(Dialog::Palette));
    }
    let now = ctx.cumulative_pass_nr();
    let offered = app.help_offered.as_ref().filter(|o| o.pass + 1 >= now).map(|o| o.subject.clone());
    if pointer && let Some(pos) = ctx.pointer_hover_pos() {
        if let Some(d) = ctx.layer_id_at(pos).and_then(dialog_on) {
            return Subject::Dialog(d);
        }
        if let Some(s) = offered.clone() {
            return s;
        }
        match crate::a11y::region_at(ctx, pos) {
            Some(crate::a11y::Region::Panel(p)) => return Subject::Panel(p),
            Some(crate::a11y::Region::Tools) => return Subject::Panel(Panel::Tools),
            Some(crate::a11y::Region::Stage) => return Subject::Tool(app.tool),
            None => {}
        }
    }
    // The keyboard: a focused tool button offers its tool; a focused panel, the panel.
    if let Some(s) = offered {
        return s;
    }
    if let Some(id) = ctx.memory(|m| m.focused())
        && let Some(d) = ctx.read_response(id).and_then(|r| dialog_on(r.layer_id))
    {
        return Subject::Dialog(d);
    }
    match crate::a11y::focused_region(ctx) {
        Some(crate::a11y::Region::Panel(p)) => return Subject::Panel(p),
        Some(crate::a11y::Region::Tools | crate::a11y::Region::Stage) => return Subject::Tool(app.tool),
        None => {}
    }
    // The window in front, if one is open; else the tool in hand.
    if let Some(d) = ctx.memory(|m| m.areas().top_layer_id(egui::Order::Middle)).and_then(dialog_on).filter(|d| visible(ctx, *d)) {
        return Subject::Dialog(d);
    }
    if let Some(d) = Dialog::ALL.into_iter().find(|d| visible(ctx, *d)) {
        return Subject::Dialog(d);
    }
    Subject::Tool(app.tool)
}

/// F1 / Help › Help for This: the guide at the place for what the person is working with.
pub fn help_for_this(app: &mut App, ctx: &egui::Context, pointer: bool) {
    let s = subject(app, ctx, pointer);
    app.open_help(s.topic());
}

#[cfg(test)]
mod tests {
    use super::*;
    use anicraft_engine::Session;
    use anicraft_engine::doc::sample;

    fn all_targets() -> Vec<&'static str> {
        let mut v: Vec<&'static str> = Tool::ALL.iter().map(|t| tool_topic(*t)).collect();
        v.extend(Panel::ALL.iter().map(|p| panel_topic(*p)));
        v.extend(Dialog::ALL.iter().map(|d| d.topic()));
        v.extend(COMMAND_TOPICS.iter().map(|(_, t)| *t));
        v.push(COMMAND_REFERENCE);
        v
    }

    #[test]
    fn every_place_in_the_map_is_a_heading_of_the_guide() {
        let problems = anicraft_guide::check_targets(anicraft_guide::PAGES, &all_targets());
        assert!(problems.is_empty(), "context help points at places the guide doesn't have:\n{}", problems.join("\n"));
        // And each opens a page the book has, with a heading to name in the status bar.
        let site: Vec<String> = anicraft_guide::site().into_iter().map(|f| f.path).collect();
        assert_eq!(SEARCH_PAGE, anicraft_guide::SEARCH_PAGE);
        for t in all_targets() {
            let page = anicraft_guide::target_html(t);
            assert_eq!(page_of(t), page, "the shell opens the page the book has");
            let file = page.split('#').next().unwrap_or("");
            assert!(site.iter().any(|p| p == file), "{t}: no page {file}");
            assert!(anicraft_guide::heading_of(anicraft_guide::PAGES, t).is_some(), "{t}");
        }
    }

    #[test]
    fn every_command_namespace_has_a_place() {
        let mut missing: Vec<String> = Vec::new();
        for c in anicraft_engine::commands() {
            let ns = c.id.split('.').next().unwrap_or("");
            if !COMMAND_TOPICS.iter().any(|(p, _)| *p == format!("{ns}.")) && !missing.contains(&ns.to_string()) {
                missing.push(ns.to_string());
            }
        }
        assert!(missing.is_empty(), "command namespaces without context help (add a row to COMMAND_TOPICS): {missing:?}");
        assert_eq!(command_topic("file.export_gif"), "exporting-and-publish.md#exporting-one-format");
        assert_eq!(command_topic("file.save"), "getting-started.md#save-your-work");
        assert_eq!(command_topic("view.onion_skin"), "timeline.md#onion-skin");
        assert_eq!(command_topic("nobody.knows"), COMMAND_REFERENCE);
        // Every dialog has a window id or is found some other way, and the ids are distinct.
        let ids: Vec<&str> = Dialog::ALL.iter().filter_map(|d| d.window_id()).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "two dialogs share a window id");
    }

    fn input(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 940.0))), events, ..Default::default() }
    }

    fn frame(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) {
        ctx.run_ui(input(events), |ui| app.frame(ui)).textures_delta.clear();
    }

    fn app(ctx: &egui::Context) -> App {
        App::with_context(ctx, Session::new(sample::demo()), Some(std::path::PathBuf::from("unused.png")))
    }

    fn settle(ctx: &egui::Context, app: &mut App) {
        for _ in 0..3 {
            frame(ctx, app, Vec::new());
        }
    }

    fn f1() -> Vec<egui::Event> {
        vec![egui::Event::Key { key: egui::Key::F1, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }]
    }

    #[test]
    fn f1_on_the_stage_opens_the_active_tool_and_over_a_panel_that_panel() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        app.tool = Tool::Bone;
        settle(&ctx, &mut app);
        // Nothing hovered or focused: the tool in hand.
        assert_eq!(subject(&app, &ctx, true), Subject::Tool(Tool::Bone));
        frame(&ctx, &mut app, f1());
        assert_eq!(app.help_opened.as_deref(), Some("masks-guides-camera-bones.md#building-an-armature"));
        // Over the stage, the same; over the timeline, the timeline's section.
        let stage = crate::a11y::region_rect(&ctx, crate::a11y::Region::Stage).expect("the stage drew");
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(stage.center())]);
        assert_eq!(subject(&app, &ctx, true), Subject::Tool(Tool::Bone));
        let tl = crate::a11y::region_rect(&ctx, crate::a11y::Region::Timeline).expect("the timeline drew");
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(tl.center())]);
        assert_eq!(subject(&app, &ctx, true), Subject::Panel(Panel::Timeline));
        frame(&ctx, &mut app, f1());
        assert_eq!(app.help_opened.as_deref(), Some("timeline.md#a-tour-of-the-timeline"));
        // From the menu the pointer doesn't count.
        assert_eq!(subject(&app, &ctx, false), Subject::Tool(Tool::Bone));
    }

    #[test]
    fn f1_in_a_dialog_opens_its_section() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        app.show_prefs = true;
        settle(&ctx, &mut app);
        assert_eq!(subject(&app, &ctx, false), Subject::Dialog(Dialog::Preferences));
        frame(&ctx, &mut app, f1());
        assert_eq!(app.help_opened.as_deref(), Some("preferences-shortcuts-accessibility.md#preferences"));
        app.show_prefs = false;
        settle(&ctx, &mut app);
        // A dialog that asks first: help for the command it asks about.
        app.close_dialog = true;
        assert_eq!(subject(&app, &ctx, true), Subject::Dialog(Dialog::CloseUnsaved));
        frame(&ctx, &mut app, f1());
        assert_eq!(app.help_opened.as_deref(), Some("getting-started.md#save-your-work"), "F1 works while a dialog holds the editor");
    }

    #[test]
    fn f1_in_the_palette_opens_the_chosen_command() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        app.palette = Some(crate::dialogs::Palette { query: "export animated gif".into(), ..Default::default() });
        settle(&ctx, &mut app);
        frame(&ctx, &mut app, f1());
        assert_eq!(app.help_opened.as_deref(), Some("exporting-and-publish.md#exporting-one-format"));
        assert!(app.palette.is_some(), "the palette stays open");
    }

    #[test]
    fn the_panel_menu_offers_help_for_its_panel() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        settle(&ctx, &mut app);
        crate::dock_ui::apply(&mut app, vec![crate::dock_ui::Act::Help(Panel::Library)]);
        assert_eq!(app.help_opened.as_deref(), Some("symbols-and-library.md#the-library-panel"));
    }
}
