//! Rulers, grid and guides through whole headless frames (ADR 0076): a guide dragged out of a
//! ruler and back, the grid under and over the artwork, Free Transform and drawing snapping,
//! the dialogs and the View menu.

use crate::app::{App, Drag, Tool};
use crate::theme::RULER;
use anicraft_engine::Session;
use anicraft_engine::doc::{GuideAxis, sample};
use egui::{Event, Modifiers, PointerButton, Pos2, Rect, pos2, vec2};
use serde_json::json;

fn screen() -> egui::RawInput {
    egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 940.0))), ..Default::default() }
}

fn frame(ctx: &egui::Context, app: &mut App, events: Vec<Event>) {
    let mut out = ctx.run_ui(egui::RawInput { events, ..screen() }, |ui| app.frame(ui));
    out.textures_delta.clear();
}

fn app() -> (egui::Context, App) {
    let ctx = egui::Context::default();
    let app = App::with_context(&ctx, Session::new(sample::demo()), Some(std::path::PathBuf::from("unused.png")));
    (ctx, app)
}

fn press(p: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

/// A drag from `a` to `b`, a few frames per step so egui sees a drag.
fn drag(ctx: &egui::Context, app: &mut App, a: Pos2, b: Pos2) {
    frame(ctx, app, vec![Event::PointerMoved(a)]);
    frame(ctx, app, vec![press(a, true)]);
    for k in 1..=6 {
        let t = k as f32 / 6.0;
        frame(ctx, app, vec![Event::PointerMoved(a + (b - a) * t)]);
    }
    frame(ctx, app, vec![press(b, false)]);
    frame(ctx, app, vec![]);
}

fn canvas(app: &App) -> crate::drop::StageGeo {
    app.drop.stage.expect("the stage drew")
}

#[test]
fn a_guide_comes_out_of_the_ruler_moves_and_goes_back() {
    let (ctx, mut app) = app();
    app.run("view.rulers", json!({"on": true}));
    app.run("view.grid_set", json!({"show": true, "spacing": 25}));
    for _ in 0..3 {
        frame(&ctx, &mut app, vec![]);
    }
    let g = canvas(&app);
    // The stage view starts inside the rulers.
    let top_ruler = pos2(g.canvas.center().x, g.canvas.min.y - RULER * 0.5);
    let to = pos2(g.canvas.center().x, g.canvas.center().y);
    drag(&ctx, &mut app, top_ruler, to);
    let guides = app.s.guides().to_vec();
    assert_eq!(guides.len(), 1, "a guide out of the top ruler");
    assert_eq!(guides[0].axis, GuideAxis::Horizontal);
    let expect = ((to.y - g.origin.y) / g.zoom).round();
    assert!((guides[0].at - expect).abs() <= 1.0, "{} vs {expect}", guides[0].at);
    // With the Selection tool: moved down, then dragged back onto the ruler.
    app.tool = Tool::Select;
    let on = pos2(g.canvas.min.x + 30.0, g.origin.y + guides[0].at * g.zoom);
    drag(&ctx, &mut app, on, on + vec2(0.0, 40.0));
    let moved = app.s.guides()[0].at;
    assert!((moved - (guides[0].at + 40.0 / g.zoom)).abs() <= 1.5, "{moved}");
    let on = pos2(g.canvas.min.x + 30.0, g.origin.y + moved * g.zoom);
    drag(&ctx, &mut app, on, top_ruler);
    assert!(app.s.guides().is_empty(), "dragged back onto the ruler");
    assert_eq!(app.drag, Drag::None);
    // A vertical one from the left ruler, then the grid over the artwork.
    drag(&ctx, &mut app, pos2(g.canvas.min.x - RULER * 0.5, g.canvas.center().y), to);
    assert_eq!(app.s.guides().first().map(|g| g.axis), Some(GuideAxis::Vertical));
    app.run("view.grid_set", json!({"over_objects": true}));
    frame(&ctx, &mut app, vec![Event::PointerMoved(to)]);
}

#[test]
fn drawing_and_free_transform_snap_to_guides() {
    let (ctx, mut app) = app();
    app.run("layer.new", json!({}));
    app.run("view.guide_add", json!({"axis": "vertical", "at": 300}));
    app.run("view.guide_add", json!({"axis": "horizontal", "at": 200}));
    app.run("draw.snap_to_objects", json!({"on": false}));
    app.run("draw.object_drawing", json!({"on": true}));
    // A rectangle drawn from 3 screen pixels off the guides' crossing starts on it. (The
    // tool's options change the panels, so the stage is measured once they have settled.)
    app.tool = Tool::Rect;
    for _ in 0..4 {
        frame(&ctx, &mut app, vec![]);
    }
    let g = canvas(&app);
    let screen_of = |x: f32, y: f32| pos2(g.origin.x + x * g.zoom, g.origin.y + y * g.zoom);
    drag(&ctx, &mut app, screen_of(300.0, 200.0) + vec2(3.0, -3.0), screen_of(400.0, 260.0));
    let b = app.s.display_list().items.last().map(|i| i.path.bounds().transformed(&i.transform)).expect("drawn");
    assert!((b.min.x - 300.0).abs() < 2.0 && (b.min.y - 200.0).abs() < 2.0, "{b:?}");
    // Free Transform: the selected box, moved, snaps onto the other guide.
    app.run("view.guide_add", json!({"axis": "vertical", "at": 500}));
    app.run("element.select", json!({"ids": [app.s.display_list().items.last().map_or(0, |i| i.element.0)]}));
    app.tool = Tool::Transform;
    for _ in 0..4 {
        frame(&ctx, &mut app, vec![]);
    }
    let g = canvas(&app);
    let screen_of = |x: f32, y: f32| pos2(g.origin.x + x * g.zoom, g.origin.y + y * g.zoom);
    // Grabbed by the middle, so the box's centre is what snaps; let go 3 screen pixels short.
    let sel = app.s.selection_bounds();
    let cx = (sel.min.x + sel.max.x) * 0.5;
    let inside = screen_of(cx, (sel.min.y + sel.max.y) * 0.5);
    let dx = (500.0 - cx) * g.zoom - 3.0;
    drag(&ctx, &mut app, inside, inside + vec2(dx, 0.0));
    let after = app.s.selection_bounds();
    assert!(((after.min.x + after.max.x) * 0.5 - 500.0).abs() < 0.5, "{after:?}");
    assert!((after.width() - sel.width()).abs() < 1e-3, "moved, not scaled");
}

#[test]
fn the_dialogs_and_the_view_menu_draw() {
    let (ctx, mut app) = app();
    app.run("view.guide_add", json!({"axis": "vertical", "at": 10}));
    app.invoke("view.grid_set");
    assert!(app.aids_dialogs.grid.is_some());
    app.invoke("view.guide_add");
    assert!(app.aids_dialogs.guides.as_ref().is_some_and(|d| d.focus_new));
    for _ in 0..3 {
        frame(&ctx, &mut app, vec![]);
    }
    // Esc closes both without a change.
    let esc = Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    frame(&ctx, &mut app, vec![esc]);
    assert!(app.aids_dialogs.grid.is_none() && app.aids_dialogs.guides.is_none());
    assert_eq!(app.s.guides().len(), 1);
    // The View menu, with its check items.
    let cmds = anicraft_engine::commands();
    let mut out = ctx.run_ui(screen(), |ui| crate::menus::menu_contents(&mut app, ui, "View", &cmds));
    out.textures_delta.clear();
    // Clear Guides says how many went.
    app.invoke("view.guides_clear");
    assert!(app.s.guides().is_empty());
    // The settings carry over into the preferences.
    app.run("view.snap_pixels", json!({"on": true}));
    crate::rulers::sync(&mut app);
    assert!(app.prefs.aids.snap_pixels);
}

#[test]
fn the_screenshot_states_draw() {
    for which in ["rulers", "editguides", "editgrid"] {
        let (ctx, mut app) = app();
        app.open_panel(Some(which));
        for _ in 0..3 {
            frame(&ctx, &mut app, vec![]);
        }
        assert!(app.s.aids.rulers && app.s.aids.show_grid, "{which}");
        assert_eq!(app.s.guides().len(), 4, "{which}");
        assert_eq!(app.aids_dialogs.guides.is_some(), which == "editguides");
        assert_eq!(app.aids_dialogs.grid.is_some(), which == "editgrid");
    }
}
