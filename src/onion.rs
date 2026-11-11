//! Onion skin and Edit Multiple Frames in the editor (ADR 0047): the timeline's buttons and
//! onion menu, the markers on the frame ruler, and Edit Multiple Frames on the stage.
//!
//! The settings are view state ([`OnionSkin`], in `App::onion`); the options that carry over
//! between sessions are also in the preferences. Only Edit Multiple Frames' edits change the
//! document, through the `frames.*` commands (ADR 0051): moves, Free Transform, align, delete,
//! copy, cut and paste, each on every keyframe the selection is on. Inside a symbol open in place
//! both work on the symbol's own timeline.

use crate::a11y::Named;
use crate::app::{App, Drag};
use crate::prefs::{ONION_REACH_MAX, OnionPrefs};
use crate::theme::{T, Type, color, font, text, to_color32, to_rgba};
use anicraft_engine::doc::Rgba;
use anicraft_engine::geom::{Rect as GRect, Vec2};
use anicraft_engine::onion::{FrameItem, Markers, OnionSkin, PRESETS};
use egui::{Color32, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use serde_json::json;
use std::sync::Arc;

/// The onion menu's popup (opened by its button, a right-click on the ruler, or `--open onion`).
pub const MENU_ID: &str = "onion_menu";

/// What the shown items depend on: document state, scene, the symbol open (and how deep),
/// playhead and the markers' frames.
type ItemsKey = (u64, usize, (usize, Option<u64>), u32, u32, u32);
/// Every element Edit Multiple Frames shows, with its stage bounds.
pub type Shown = Arc<Vec<(FrameItem, GRect)>>;

/// The editor's onion state beyond [`OnionSkin`].
#[derive(Default)]
pub struct OnionUi {
    /// The marker being dragged on the ruler: the start (`true`) or the end.
    pub drag: Option<bool>,
    /// Edit Multiple Frames' selection: elements on keyframes within the markers.
    pub sel: Vec<FrameItem>,
    /// Open the onion menu on the next frame.
    pub open_menu: bool,
    /// What Edit Multiple Frames shows, for (state, scene, playhead, first, last).
    cache: Option<(ItemsKey, Shown)>,
}

fn rgb(c: [u8; 3]) -> Rgba {
    Rgba::new(f32::from(c[0]) / 255.0, f32::from(c[1]) / 255.0, f32::from(c[2]) / 255.0, 1.0)
}

/// The settings a session starts with: off, with the saved options and colours.
pub fn from_prefs(p: &OnionPrefs) -> OnionSkin {
    let mut o = OnionSkin { ghosts: false, edit_multiple: false, markers: Markers::Relative { before: p.before, after: p.after }, tint: p.tint, fade: p.fade, outlines: p.outlines, keyframes_only: p.keyframes_only, ..OnionSkin::default() };
    if p.all {
        o.all();
    }
    o.past = p.past.map_or_else(|| to_rgba(T.onion_past), rgb);
    o.future = p.future.map_or_else(|| to_rgba(T.onion_future), rgb);
    o
}

/// The theme's colours where the user didn't pick their own (after a palette change).
pub fn refresh_colors(app: &mut App) {
    app.onion.past = app.prefs.onion.past.map_or_else(|| to_rgba(T.onion_past), rgb);
    app.onion.future = app.prefs.onion.future.map_or_else(|| to_rgba(T.onion_future), rgb);
}

/// Changes the settings and saves the options that carry over.
fn change(app: &mut App, ctx: &egui::Context, f: impl FnOnce(&mut OnionSkin, &mut OnionPrefs)) {
    let mut p = app.prefs.clone();
    f(&mut app.onion, &mut p.onion);
    let o = app.onion;
    match o.markers {
        Markers::Relative { before, after } => {
            p.onion.before = before.min(ONION_REACH_MAX);
            p.onion.after = after.min(ONION_REACH_MAX);
            p.onion.all = false;
        }
        Markers::Anchored { .. } => p.onion.all = o.is_all(),
    }
    (p.onion.tint, p.onion.fade, p.onion.outlines, p.onion.keyframes_only) = (o.tint, o.fade, o.outlines, o.keyframes_only);
    app.set_prefs(ctx, p);
}

/// The frames within the markers (0-based, inclusive).
pub fn range(app: &App) -> (u32, u32) {
    app.onion.range(app.s.frame, app.s.frame_count())
}

/// What the stage composes now: `None` while playing or when nothing is on.
pub fn stage_skin(app: &App) -> Option<&OnionSkin> {
    // Inside a symbol open in place too: the stage composes the symbol's own frames (ADR 0051).
    (!app.playing && app.onion.shows()).then_some(&app.onion)
}

/// Onion skin on or off (the timeline button, View › Onion Skin, Alt+Shift+O).
pub fn toggle_ghosts(app: &mut App) {
    app.onion.ghosts = !app.onion.ghosts;
}

/// Edit Multiple Frames on or off. The selection starts empty either way: what was selected on
/// one frame is not what is selected on several.
pub fn toggle_edit_multiple(app: &mut App) {
    app.onion.edit_multiple = !app.onion.edit_multiple;
    app.onion_ui.sel.clear();
    if app.onion.edit_multiple {
        app.s.selection.clear();
        app.status_ok(tr!("Edit Multiple Frames: the keyframes between the onion markers show at full strength. Select and drag artwork to move it on all of them; drag the markers on the frame ruler to choose the frames."));
    }
}

/// Edit Multiple Frames is taking the Selection tool's input.
pub fn editing(app: &App) -> bool {
    app.onion.edit_multiple && !app.playing && !crate::camera_tool::previewing(app)
}

// ----- the menu ------------------------------------------------------------------------------

/// The onion menu: on/off, options, markers, colours. Stays open while options change.
pub fn menu(app: &mut App, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let o = app.onion;
    let key = app.keymap.primary("view.onion").unwrap_or("").to_string();
    let mut ghosts = o.ghosts;
    if ui.checkbox(&mut ghosts, a11y_key(tr!("Onion Skin"), &key)).changed() {
        toggle_ghosts(app);
    }
    let mut multi = o.edit_multiple;
    if ui.checkbox(&mut multi, tr!("Edit Multiple Frames")).on_hover_text(tr!("Show the keyframes between the markers at full strength, and select and move their artwork together")).changed() {
        toggle_edit_multiple(app);
    }
    let mut outlines = o.outlines;
    if ui.checkbox(&mut outlines, tr!("Outlines")).on_hover_text(tr!("Draw onion frames as outlines only")).changed() {
        change(app, &ctx, |o, _| o.outlines = outlines);
    }
    let mut keys = o.keyframes_only;
    if ui.checkbox(&mut keys, tr!("Keyframes Only")).on_hover_text(tr!("Onion-skin only the frames where a keyframe starts")).changed() {
        change(app, &ctx, |o, _| o.keyframes_only = keys);
    }
    ui.separator();
    ui.label(text(Type::Caption, tr!("Markers")));
    let mut always = app.prefs.onion.always_markers;
    if ui.checkbox(&mut always, tr!("Always Show Markers")).on_hover_text(tr!("Show the markers on the frame ruler even when onion skin is off")).changed() {
        change(app, &ctx, |_, p| p.always_markers = always);
    }
    let mut anchored = o.anchored();
    if ui.checkbox(&mut anchored, tr!("Anchor Markers")).on_hover_text(tr!("Keep the markers on their frames instead of moving them with the playhead")).changed() {
        let (f, n) = (app.s.frame, app.s.frame_count());
        change(app, &ctx, |o, _| o.set_anchored(anchored, f, n));
    }
    ui.horizontal(|ui| {
        for n in PRESETS {
            let on = o.markers == Markers::Relative { before: n, after: n };
            if ui.selectable_label(on, tr!("Onion {n}", n = n)).on_hover_text(trn!("{n} frame before and after the playhead", "{n} frames before and after the playhead", n as usize)).clicked() {
                change(app, &ctx, |o, _| o.preset(n));
            }
        }
        if ui.selectable_label(o.is_all(), tr!("Onion All")).on_hover_text(tr!("Every frame of the timeline")).clicked() {
            change(app, &ctx, |o, _| o.all());
        }
    });
    ui.separator();
    ui.label(text(Type::Caption, tr!("Colour")));
    let mut tint = o.tint;
    if ui.checkbox(&mut tint, tr!("Tint")).on_hover_text(tr!("Tint frames before the playhead and after it in their colours; off, they keep their own colours")).changed() {
        change(app, &ctx, |o, _| o.tint = tint);
    }
    let mut fade = o.fade;
    if ui.checkbox(&mut fade, tr!("Fade by Distance")).on_hover_text(tr!("Frames farther from the playhead are fainter")).changed() {
        change(app, &ctx, |o, _| o.fade = fade);
    }
    ui.horizontal(|ui| {
        for past in [true, false] {
            let name = if past { tr!("Before colour") } else { tr!("After colour") };
            ui.label(text(Type::Body, if past { tr!("Before") } else { tr!("After") }));
            let mut c = to_color32(if past { o.past } else { o.future });
            let r = ui.color_edit_button_srgba(&mut c).named(name).on_hover_text(if past { tr!("Colour of the frames before the playhead") } else { tr!("Colour of the frames after the playhead") });
            if r.changed() {
                let [cr, cg, cb, _] = c.to_srgba_unmultiplied();
                change(app, &ctx, |o, p| {
                    let pick = [cr, cg, cb];
                    if past {
                        o.past = rgb(pick);
                        p.past = Some(pick);
                    } else {
                        o.future = rgb(pick);
                        p.future = Some(pick);
                    }
                });
            }
        }
        let custom = app.prefs.onion.past.is_some() || app.prefs.onion.future.is_some();
        if ui.add_enabled(custom, egui::Button::new(tr!("Reset"))).on_hover_text(tr!("Back to the theme's colours")).clicked() {
            change(app, &ctx, |o, p| {
                p.past = None;
                p.future = None;
                o.past = to_rgba(T.onion_past);
                o.future = to_rgba(T.onion_future);
            });
        }
    });
}

/// A checkbox label with its shortcut, as menus show it.
fn a11y_key(name: &str, key: &str) -> String {
    crate::a11y::with_key(name, Some(key))
}

/// The timeline's onion buttons: Onion Skin, Edit Multiple Frames and the onion menu.
pub fn buttons(app: &mut App, ui: &mut egui::Ui) {
    let key = app.keymap.primary("view.onion").map(str::to_string);
    if crate::timeline::icon_button(ui, &crate::a11y::with_key(tr!("Onion skin"), key.as_deref()), Some(app.onion.ghosts), crate::icons::Icon::Onion).clicked() {
        toggle_ghosts(app);
    }
    if crate::timeline::icon_button(ui, tr!("Edit Multiple Frames"), Some(app.onion.edit_multiple), crate::icons::Icon::EditMultiple).clicked() {
        toggle_edit_multiple(app);
    }
    let (rect, resp) = ui.allocate_exact_size(vec2(16.0, crate::theme::TARGET), Sense::click());
    let hot = resp.hovered() || egui::Popup::is_id_open(ui.ctx(), egui::Id::new(MENU_ID));
    ui.painter().rect_filled(rect, T.rc(), if hot { T.header } else { Color32::TRANSPARENT });
    crate::icons::paint(ui.painter(), Rect::from_center_size(rect.center(), vec2(12.0, 12.0)), crate::icons::Icon::ChevronDown, if hot { T.icon_hot } else { T.icon });
    crate::a11y::control(ui, &resp, egui::WidgetType::Button, tr!("Onion skin options"), None);
    let resp = resp.on_hover_text(tr!("Onion skin options: markers, outlines, colours"));
    if std::mem::take(&mut app.onion_ui.open_menu) {
        egui::Popup::open_id(ui.ctx(), egui::Id::new(MENU_ID));
    }
    let shown = egui::Popup::menu(&resp).id(egui::Id::new(MENU_ID)).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        ui.set_min_width(230.0);
        menu(app, ui);
    });
    // egui leaves the popup's own surface without a role or name.
    if let Some(r) = shown {
        ui.ctx().accesskit_node_builder(r.response.id, |node| {
            node.set_role(egui::accesskit::Role::Menu);
            node.set_label(tr!("Onion skin options"));
        });
    }
}

// ----- the markers on the frame ruler --------------------------------------------------------

/// Draws the onion markers on the frame ruler and lets them be dragged. `x_of` is a frame
/// cell's left edge, `frame_at` the frame under a pointer x.
pub fn ruler(app: &mut App, ui: &mut egui::Ui, ruler: Rect, cell_w: f32, x_of: impl Fn(u32) -> f32, frame_at: impl Fn(f32) -> u32) {
    let shown = app.onion.shows() || app.prefs.onion.always_markers;
    if !shown || app.playing {
        app.onion_ui.drag = None;
        return;
    }
    let (first, last) = range(app);
    let p = app.s.frame;
    let painter = ui.painter_at(ruler);
    let (past, future) = (to_color32(app.onion.past), to_color32(app.onion.future));
    let dim = if app.onion.shows() { 1.0 } else { 0.5 };
    // The frames within the markers, before and after the playhead in their colours.
    if first < p {
        painter.rect_filled(Rect::from_min_max(pos2(x_of(first), ruler.top()), pos2(x_of(p), ruler.bottom())), 0.0, past.gamma_multiply(0.22 * dim));
    }
    if last > p {
        painter.rect_filled(Rect::from_min_max(pos2(x_of(p + 1), ruler.top()), pos2(x_of(last + 1), ruler.bottom())), 0.0, future.gamma_multiply(0.22 * dim));
    }
    let frames = app.s.frame_count();
    let anchored = app.onion.anchored();
    for start in [true, false] {
        let x = if start { x_of(first) } else { x_of(last + 1) };
        let c = if start { past } else { future }.gamma_multiply(dim);
        // A bracket: [ at the start, ] at the end.
        let serif = (cell_w * 0.5).clamp(3.0, 6.0) * if start { 1.0 } else { -1.0 };
        let (top, bottom) = (ruler.top() + 2.0, ruler.bottom() - 1.0);
        let stroke = Stroke::new(2.0, c);
        painter.line_segment([pos2(x, top), pos2(x, bottom)], stroke);
        painter.line_segment([pos2(x, top), pos2(x + serif, top)], stroke);
        painter.line_segment([pos2(x, bottom), pos2(x + serif, bottom)], stroke);
        if anchored {
            // Anchored markers carry a pin.
            painter.circle_filled(pos2(x + serif * 0.6, ruler.center().y), 2.0, c);
        }
        let hit = Rect::from_center_size(pos2(x, ruler.center().y), vec2(10.0, ruler.height()));
        let id = ui.id().with(("onion_marker", start));
        let resp = ui.interact(hit, id, crate::a11y::pointer_only());
        let at = if start { first } else { last };
        let name = if start { tr!("Onion start marker") } else { tr!("Onion end marker") };
        resp.widget_info(|| egui::WidgetInfo::slider(true, f64::from(at + 1), name));
        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if resp.drag_started() {
            app.onion_ui.drag = Some(start);
        }
        if resp.dragged()
            && app.onion_ui.drag == Some(start)
            && let Some(ptr) = resp.interact_pointer_pos()
        {
            // The end marker sits after its frame's cell: aim at the cell the pointer is in.
            let f = frame_at(if start { ptr.x + cell_w * 0.5 } else { ptr.x - cell_w * 0.5 });
            app.onion.drag_marker(start, f, p, frames);
        }
        if resp.drag_stopped() {
            app.onion_ui.drag = None;
            // Saved when let go, not on every step of the drag.
            let ctx = ui.ctx().clone();
            change(app, &ctx, |_, _| {});
        }
        let tip = if anchored { tr!("Drag to choose the frames onion skin shows (anchored: they stay when the playhead moves)") } else { tr!("Drag to choose how many frames onion skin shows before and after the playhead") };
        resp.on_hover_text(tip);
    }
}

// ----- Edit Multiple Frames on the stage ------------------------------------------------------

/// Every element Edit Multiple Frames shows, with its stage bounds (cached per document state,
/// playhead and markers).
pub fn items(app: &mut App) -> Shown {
    let (first, last) = range(app);
    let key = (app.s.state, app.s.scene, (app.s.editing.len(), app.s.editing_symbol().map(|s| s.0)), app.s.frame, first, last);
    if let Some((k, v)) = &app.onion_ui.cache
        && *k == key
    {
        return v.clone();
    }
    let v = Arc::new(app.s.multi_frame_items(first, last));
    app.onion_ui.cache = Some((key, v.clone()));
    v
}

/// The selection, less what isn't shown any more (after an undo, a marker drag, a new frame).
fn live_selection(app: &mut App) -> Vec<FrameItem> {
    let shown = items(app);
    let sel: Vec<FrameItem> = app.onion_ui.sel.iter().copied().filter(|it| shown.iter().any(|(s, _)| s == it)).collect();
    app.onion_ui.sel.clone_from(&sel);
    sel
}

/// The `items` a command answered with, as the selection.
fn parse_items(v: &serde_json::Value) -> Option<Vec<FrameItem>> {
    let list = v.get("items")?.as_array()?;
    Some(
        list.iter()
            .filter_map(|t| {
                let n = |i: usize| t.get(i).and_then(serde_json::Value::as_u64);
                Some(FrameItem { layer: anicraft_engine::doc::LayerId(n(0)?), frame: u32::try_from(n(1)?).ok()?, element: anicraft_engine::doc::ElementId(n(2)?) })
            })
            .collect(),
    )
}

/// Runs `id` (a `frames.*` command) on the markers' frames with `extra` params, on the selection
/// when `on_sel` (nothing happens without one), and takes the items it answers as the new
/// selection.
fn run_on(app: &mut App, id: &str, extra: serde_json::Value, on_sel: bool) -> Option<serde_json::Value> {
    let (first, last) = range(app);
    let mut p = json!({"first": first, "last": last});
    if on_sel {
        let sel = live_selection(app);
        if sel.is_empty() {
            return None;
        }
        p["items"] = json!(sel.iter().map(FrameItem::to_json).collect::<Vec<_>>());
    }
    if let (Some(o), Some(e)) = (p.as_object_mut(), extra.as_object()) {
        o.extend(e.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    let v = app.run(id, p)?;
    if let Some(items) = parse_items(&v) {
        app.onion_ui.sel = items;
    }
    Some(v)
}

/// Moves the selection by `d` on every keyframe it is on (no-op when nothing is selected). Raw
/// shapes merge where they land when `merge` (a drag merges once, when it ends).
fn move_by(app: &mut App, d: Vec2, merge: bool) -> bool {
    if d == Vec2::ZERO {
        return false;
    }
    run_on(app, "frames.edit_multiple", json!({"dx": d.x, "dy": d.y, "merge": merge}), true).is_some()
}

/// The end of a drag that moved or transformed the selection: raw shapes merge where they landed
/// (in the drag's undo step), then the step closes.
pub fn drag_end(app: &mut App) {
    let raw = live_selection(app).iter().any(|it| is_raw(app, it));
    if raw {
        run_on(app, "frames.merge", json!({}), true);
    }
    app.s.end_gesture();
}

/// Whether `it` is a raw (merge-drawing) shape.
fn is_raw(app: &App, it: &FrameItem) -> bool {
    let k = app.s.timeline().layer(it.layer).and_then(|l| l.keyframe_starting_at(it.frame).and_then(|i| l.keyframes.get(i)));
    k.and_then(|k| k.elements.iter().find(|e| e.id == it.element)).is_some_and(|e| matches!(&e.kind, anicraft_engine::doc::ElementKind::Shape(sh) if sh.merge))
}

/// The selection's bounds on every keyframe together (the Free Transform box), in stage space.
pub fn selection_bounds(app: &mut App) -> Option<[f32; 4]> {
    let sel = live_selection(app);
    let r = items(app).iter().filter(|(it, _)| sel.contains(it)).fold(GRect::EMPTY, |r, (_, b)| r.union(b));
    (!r.is_empty()).then_some([r.min.x, r.min.y, r.max.x, r.max.y])
}

/// One step of a Free Transform drag: `delta` (stage space, about the shared pivot the drag
/// chose) on every keyframe; merged when the drag ends.
pub fn transform_by(app: &mut App, delta: &anicraft_engine::geom::Affine) -> bool {
    let [a, b, c, d, e, f] = delta.m;
    run_on(app, "frames.transform", json!({"a": a, "b": b, "c": c, "d": d, "e": e, "f": f, "merge": false}), true).is_some()
}

/// The Edit menu, the Align panel and their shortcuts with Edit Multiple Frames on: they act on
/// the selection on every keyframe. True when handled here.
pub fn invoke(app: &mut App, id: &str) -> bool {
    let has_sel = !live_selection(app).is_empty();
    let undo = app.undo_key();
    match id {
        "edit.select_all" => select_all(app),
        "edit.deselect" => app.onion_ui.sel.clear(),
        "edit.delete" if has_sel => {
            if let Some(v) = run_on(app, "frames.delete", json!({}), true) {
                let n = v["deleted"].as_u64().unwrap_or(0) as usize;
                app.status_ok(trn!("Deleted {n} element on the frames between the markers. {undo} to undo.", "Deleted {n} elements on the frames between the markers. {undo} to undo.", n, undo = undo));
            }
        }
        "edit.copy" if has_sel => {
            if let Some(v) = run_on(app, "frames.copy", json!({}), true) {
                let n = v["copied"].as_u64().unwrap_or(0) as usize;
                let key = crate::platform::shortcut_label(app.keymap.primary("edit.paste_in_place").unwrap_or("Ctrl+Shift+V"));
                app.status_ok(trn!("Copied {n} element. Paste in Place ({key}) puts it back on its keyframe.", "Copied {n} elements. Paste in Place ({key}) puts each back on its keyframe.", n, key = key));
            }
        }
        "edit.cut" if has_sel => {
            if let Some(v) = run_on(app, "frames.cut", json!({}), true) {
                let n = v["cut"].as_u64().unwrap_or(0) as usize;
                app.status_ok(trn!("Cut {n} element from the frames between the markers. {undo} to undo.", "Cut {n} elements from the frames between the markers. {undo} to undo.", n, undo = undo));
            }
        }
        // Paste in Place: onto the keyframes between the markers.
        "edit.paste_in_place" if !app.s.clipboard.is_empty() => {
            if let Some(v) = run_on(app, "frames.paste", json!({}), false) {
                let n = v["keyframes"].as_u64().unwrap_or(0) as usize;
                app.status_ok(trn!("Pasted in place onto {n} keyframe. {undo} to undo.", "Pasted in place onto {n} keyframes. {undo} to undo.", n, undo = undo));
            }
        }
        // Paste: onto the current frame, as without Edit Multiple Frames; what lands is selected.
        "edit.paste" if !app.s.clipboard.is_empty() => {
            if app.run("edit.paste", json!({})).is_some() {
                adopt_selection(app);
            }
        }
        "edit.duplicate" | "modify.convert_to_symbol" | "modify.break_apart" if has_sel => {
            app.status_err(tr!("Edit Multiple Frames moves, transforms, aligns, deletes, copies and pastes artwork on several keyframes; for this, turn it off and work frame by frame."));
        }
        "align.left" | "align.hcenter" | "align.right" | "align.top" | "align.vcenter" | "align.bottom" if has_sel => {
            let edge = id.trim_start_matches("align.");
            // The Align panel's To Stage option.
            let p = if app.align_to_stage { json!({"edge": edge, "to_stage": true}) } else { json!({"edge": edge}) };
            run_on(app, "frames.align", p, true);
        }
        "modify.rotate_cw" | "modify.rotate_ccw" | "modify.flip_h" | "modify.flip_v" if has_sel => {
            let p = match id {
                "modify.rotate_cw" => json!({"rotate": 90}),
                "modify.rotate_ccw" => json!({"rotate": -90}),
                "modify.flip_h" => json!({"scale_x": -1}),
                _ => json!({"scale_y": -1}),
            };
            run_on(app, "frames.transform", p, true);
        }
        _ => return false,
    }
    true
}

/// Takes what a command selected at the playhead as the multi-frame selection (each element on
/// its layer's keyframe there), and lets go of the playhead's selection.
fn adopt_selection(app: &mut App) {
    let frame = app.s.frame;
    let picked: Vec<FrameItem> = app.s.selected_elements().iter().filter_map(|(l, e)| app.s.timeline().layer(*l).and_then(|x| x.keyframe_at(frame)).map(|k| FrameItem { layer: *l, frame: k.start, element: e.id })).collect();
    app.s.selection.clear();
    app.onion_ui.sel = picked;
}

/// Whether a menu command can act with Edit Multiple Frames on (`None`: decided as usual).
pub fn enabled(app: &App, id: &str) -> Option<bool> {
    if !editing(app) {
        return None;
    }
    let sel = !app.onion_ui.sel.is_empty();
    match id {
        "edit.delete" | "edit.cut" | "edit.copy" => Some(sel),
        "edit.duplicate" | "modify.convert_to_symbol" | "modify.break_apart" | "arrange.bring_to_front" | "arrange.send_to_back" | "align.distribute_h" | "align.distribute_v" => Some(false),
        _ => None,
    }
}

/// The stage's right-click menu with Edit Multiple Frames on.
pub fn context_menu(app: &mut App, ui: &mut egui::Ui) {
    let selected = !app.onion_ui.sel.is_empty();
    let ids: &[&str] = if selected { &["edit.cut", "edit.copy", "edit.paste", "edit.paste_in_place", "edit.delete"] } else { &["edit.paste", "edit.paste_in_place", "edit.select_all"] };
    for id in ids {
        crate::menus::cmd_item(app, ui, id);
    }
    if selected {
        ui.separator();
        ui.menu_button(tr!("Transform"), |ui| {
            for id in ["modify.rotate_cw", "modify.rotate_ccw", "modify.flip_h", "modify.flip_v"] {
                crate::menus::cmd_item(app, ui, id);
            }
        });
        ui.menu_button(tr!("Align"), |ui| {
            for id in ["align.left", "align.hcenter", "align.right", "align.top", "align.vcenter", "align.bottom"] {
                crate::menus::cmd_item(app, ui, id);
            }
        });
    }
    ui.separator();
    for id in ["edit.undo", "edit.redo"] {
        crate::menus::cmd_item(app, ui, id);
    }
}

/// A press on the stage with Edit Multiple Frames on: on artwork, select it (unless it is) and
/// move; elsewhere, a marquee.
pub fn drag_start(app: &mut App, sp: Pos2, shift: bool, zoom: f32) -> Drag {
    let (first, last) = range(app);
    match app.s.multi_frame_pick(first, last, Vec2::new(sp.x, sp.y), 3.0 / zoom.max(1e-3)) {
        Some(it) => {
            if !app.onion_ui.sel.contains(&it) {
                if !shift {
                    app.onion_ui.sel.clear();
                }
                app.onion_ui.sel.push(it);
            }
            // Undo labels stay English in the history and are translated where they show.
            app.s.begin_gesture(tr_noop!("Move"));
            Drag::MultiMove { last: sp }
        }
        None => Drag::MultiMarquee { start: sp, cur: sp },
    }
}

/// The pointer moved to `c` during a move.
pub fn drag_to(app: &mut App, last: &mut Pos2, c: Pos2) {
    let d = c - *last;
    if d.length_sq() > 0.0 && move_by(app, Vec2::new(d.x, d.y), false) {
        *last = c;
    }
}

/// A click: select what is under the pointer (Shift adds or takes away), or nothing.
pub fn click(app: &mut App, sp: Pos2, shift: bool, zoom: f32) {
    let (first, last) = range(app);
    match app.s.multi_frame_pick(first, last, Vec2::new(sp.x, sp.y), 3.0 / zoom.max(1e-3)) {
        Some(it) if shift => match app.onion_ui.sel.iter().position(|s| *s == it) {
            Some(i) => {
                app.onion_ui.sel.remove(i);
            }
            None => app.onion_ui.sel.push(it),
        },
        Some(it) => app.onion_ui.sel = vec![it],
        None if !shift => app.onion_ui.sel.clear(),
        None => {}
    }
}

/// A marquee let go: what lies wholly inside it, on every keyframe shown.
pub fn marquee(app: &mut App, a: Pos2, b: Pos2, shift: bool) {
    let r = GRect::from_points(Vec2::new(a.x, a.y), Vec2::new(b.x, b.y));
    let inside: Vec<FrameItem> = items(app).iter().filter(|(_, bb)| !bb.is_empty() && r.contains(bb.min) && r.contains(bb.max)).map(|(it, _)| *it).collect();
    if !shift {
        app.onion_ui.sel.clear();
    }
    for it in inside {
        if !app.onion_ui.sel.contains(&it) {
            app.onion_ui.sel.push(it);
        }
    }
}

/// Select All with Edit Multiple Frames on: every element on every keyframe shown.
pub fn select_all(app: &mut App) {
    app.onion_ui.sel = items(app).iter().map(|(it, _)| *it).collect();
    let n = app.onion_ui.sel.len();
    app.status_ok(trn!("Selected {n} element on the frames between the markers", "Selected {n} elements on the frames between the markers", n));
}

/// The arrow keys with Edit Multiple Frames on: one undo step per press.
pub fn nudge(app: &mut App, d: Vec2) {
    let own = !app.s.in_gesture();
    if own {
        app.s.begin_gesture(tr_noop!("Move")); // i18n-exempt: an undo label (the engine's English, translated where shown)
    }
    move_by(app, d, true);
    if own {
        app.s.end_gesture();
    }
}

/// Boxes around the selection on every keyframe (and the artwork under the pointer), and the
/// marquee being dragged.
pub fn draw(app: &mut App, painter: &egui::Painter, to_screen: impl Fn(Vec2) -> Pos2, hover: Option<Pos2>, zoom: f32) {
    let sel = live_selection(app);
    let shown = items(app);
    let box_of = |b: &GRect| Rect::from_min_max(to_screen(b.min), to_screen(b.max)).expand(1.0);
    if let Some(h) = hover
        && app.drag == Drag::None
    {
        let (first, last) = range(app);
        if let Some(it) = app.s.multi_frame_pick(first, last, Vec2::new(h.x, h.y), 3.0 / zoom.max(1e-3))
            && !sel.contains(&it)
            && let Some((_, b)) = shown.iter().find(|(s, _)| *s == it)
        {
            painter.rect_stroke(box_of(b), 0.0, Stroke::new(1.0, T.selection.gamma_multiply(0.55)), StrokeKind::Middle);
        }
    }
    for (it, b) in shown.iter().filter(|(it, _)| sel.contains(it)) {
        if b.is_empty() {
            continue;
        }
        let r = box_of(b);
        // The playhead's keyframe in the selection colour; the others a step lighter, so it is
        // clear which artwork is on this frame.
        let here = app.s.timeline().layer(it.layer).and_then(|l| l.keyframe_at(app.s.frame)).is_some_and(|k| k.start == it.frame);
        let c = if here { T.selection } else { T.selection.gamma_multiply(0.7) };
        painter.rect_stroke(r, 0.0, Stroke::new(if here { 1.5 } else { 1.0 }, c), StrokeKind::Middle);
    }
    if let Drag::MultiMarquee { start, cur } = app.drag {
        let r = Rect::from_two_pos(to_screen(Vec2::new(start.x, start.y)), to_screen(Vec2::new(cur.x, cur.y)));
        painter.rect_filled(r, 0.0, T.selection.gamma_multiply(0.12));
        painter.rect_stroke(r, 0.0, Stroke::new(1.0, T.selection), StrokeKind::Middle);
    }
}

/// A caption on the stage while Edit Multiple Frames is on: which frames it shows.
pub fn stage_badge(app: &App, painter: &egui::Painter, canvas: Rect) {
    if !editing(app) {
        return;
    }
    let (first, last) = range(app);
    let s = tr!("Edit Multiple Frames: {first}–{last}", first = first + 1, last = last + 1);
    let at = canvas.left_top() + vec2(10.0, 8.0);
    let galley = painter.layout_no_wrap(s, font(Type::Caption), color(Type::Caption));
    let r = Rect::from_min_size(at, galley.size()).expand2(vec2(6.0, 3.0));
    painter.rect_filled(r, T.rc(), T.panel_alt);
    painter.galley(at, galley, color(Type::Caption));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let ctx = egui::Context::default();
        let session = anicraft_engine::Session::new(anicraft_engine::doc::sample::demo());
        // Screenshot mode: default preferences, nothing saved.
        App::with_context(&ctx, session, Some(std::path::PathBuf::from("unused.png")))
    }

    #[test]
    fn edit_multiple_frames_selects_and_moves_across_keyframes_in_one_step() {
        let mut a = app();
        a.open_panel(Some("editmultiple"));
        assert!(editing(&a) && a.s.selection.is_empty(), "the playhead's selection is let go");
        let sel = a.onion_ui.sel.clone();
        assert!(sel.iter().any(|it| it.frame != sel[0].frame), "artwork on more than one keyframe: {sel:?}");
        // Where each selected element is on its own keyframe.
        let at = |a: &App| -> Vec<f32> {
            sel.iter()
                .map(|it| {
                    let l = a.s.doc.timeline(a.s.scene).layer(it.layer).expect("layer");
                    let k = l.keyframes.iter().find(|k| k.start == it.frame).expect("keyframe");
                    k.elements.iter().find(|e| e.id == it.element).expect("element").transform.translation().x
                })
                .collect()
        };
        let before = at(&a);
        let steps = a.s.history_len();
        nudge(&mut a, Vec2::new(10.0, 0.0));
        assert_eq!(a.s.history_len(), steps + 1, "one undo step");
        for (b, c) in before.iter().zip(at(&a)) {
            assert!((c - b - 10.0).abs() < 1e-3, "{b} → {c}");
        }
        a.s.undo().expect("undo");
        assert_eq!(at(&a), before, "one undo takes it all back");
        assert_eq!(a.onion_ui.sel, sel, "the selection stays");
        // Off: the selection goes with it, and arrow keys act on the playhead's selection again.
        toggle_edit_multiple(&mut a);
        assert!(!editing(&a) && a.onion_ui.sel.is_empty());
    }

    #[test]
    fn a_marquee_takes_what_lies_wholly_inside_on_every_keyframe() {
        let mut a = app();
        a.open_panel(Some("editmultiple"));
        let shown = items(&mut a);
        let (first, _) = shown.iter().find(|(_, b)| b.max.x < 640.0 && b.max.y < 400.0 && !b.is_empty()).copied().expect("something in the top left");
        marquee(&mut a, pos2(-5.0, -5.0), pos2(640.0, 400.0), false);
        assert!(a.onion_ui.sel.contains(&first));
        assert!(a.onion_ui.sel.iter().all(|it| shown.iter().any(|(s, b)| s == it && b.max.x <= 640.0 && b.max.y <= 400.0)));
        marquee(&mut a, pos2(5000.0, 5000.0), pos2(5001.0, 5001.0), false);
        assert!(a.onion_ui.sel.is_empty());
    }

    #[test]
    fn options_are_saved_and_come_back() {
        let mut a = app();
        let ctx = egui::Context::default();
        change(&mut a, &ctx, |o, p| {
            o.preset(5);
            o.outlines = true;
            o.keyframes_only = true;
            o.fade = false;
            p.past = Some([1, 2, 3]);
        });
        let p = a.prefs.onion.clone();
        assert_eq!((p.before, p.after, p.all, p.outlines, p.keyframes_only, p.fade, p.past), (5, 5, false, true, true, false, Some([1, 2, 3])));
        let back = from_prefs(&p);
        assert_eq!((back.markers, back.outlines, back.keyframes_only, back.fade, back.ghosts), (Markers::Relative { before: 5, after: 5 }, true, true, false, false));
        assert_eq!(srgb_of(back.past), [1, 2, 3]);
        change(&mut a, &ctx, |o, _| o.all());
        assert!(a.prefs.onion.all && from_prefs(&a.prefs.onion).is_all());
        // A broken file can't ask for more than the limit.
        let mut big = a.prefs.clone();
        big.onion.before = u32::MAX;
        assert_eq!(big.sanitized().onion.before, ONION_REACH_MAX);
    }

    /// How many elements each keyframe of the sample's top layer (the ball) holds.
    fn ball_counts(a: &App) -> Vec<usize> {
        a.s.timeline().layers.first().map(|l| l.keyframes.iter().map(|k| k.elements.len()).collect()).unwrap_or_default()
    }

    #[test]
    fn the_edit_menu_deletes_copies_and_pastes_on_every_keyframe() {
        let mut a = app();
        a.open_panel(Some("edittransform"));
        let sel = a.onion_ui.sel.clone();
        assert!(sel.len() >= 2, "the ball on two keyframes: {sel:?}");
        let before = ball_counts(&a);
        let steps = a.s.history_len();
        assert!(invoke(&mut a, "edit.cut"));
        assert_eq!(a.s.history_len(), steps + 1, "one undo step");
        let after = ball_counts(&a);
        assert_eq!(before.iter().sum::<usize>() - after.iter().sum::<usize>(), sel.len());
        assert!(a.onion_ui.sel.is_empty(), "nothing left to select");
        // Paste in Place puts each back on its keyframe, selected.
        assert!(invoke(&mut a, "edit.paste_in_place"));
        assert_eq!(ball_counts(&a), before);
        assert_eq!(a.onion_ui.sel.len(), sel.len());
        assert_eq!(a.onion_ui.sel.iter().map(|it| it.frame).collect::<Vec<_>>(), sel.iter().map(|it| it.frame).collect::<Vec<_>>());
        // Menus follow the multi-frame selection; duplicate waits for a later slice.
        assert_eq!(enabled(&a, "edit.delete"), Some(true));
        assert_eq!(enabled(&a, "edit.duplicate"), Some(false));
        assert!(invoke(&mut a, "edit.delete"));
        assert_eq!(enabled(&a, "edit.delete"), Some(false));
        // Without Edit Multiple Frames the menu is decided as usual.
        toggle_edit_multiple(&mut a);
        assert_eq!(enabled(&a, "edit.delete"), None);
    }

    #[test]
    fn free_transform_scales_every_keyframe_about_one_pivot_in_one_step() {
        let mut a = app();
        a.open_panel(Some("edittransform"));
        let b = selection_bounds(&mut a).expect("a box");
        let (cx, cy) = ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
        let steps = a.s.history_len();
        a.s.begin_gesture("Free Transform");
        // A drag in two steps: twice as big about the box's centre, sent as changes.
        let half = anicraft_engine::geom::Affine::translate(cx, cy).mul(&anicraft_engine::geom::Affine::scale(1.5, 1.5)).mul(&anicraft_engine::geom::Affine::translate(-cx, -cy));
        let rest = anicraft_engine::geom::Affine::translate(cx, cy).mul(&anicraft_engine::geom::Affine::scale(2.0 / 1.5, 2.0 / 1.5)).mul(&anicraft_engine::geom::Affine::translate(-cx, -cy));
        assert!(transform_by(&mut a, &half) && transform_by(&mut a, &rest));
        drag_end(&mut a);
        assert_eq!(a.s.history_len(), steps + 1, "one undo step");
        let n = selection_bounds(&mut a).expect("a box");
        for (got, want) in n.iter().zip([cx + 2.0 * (b[0] - cx), cy + 2.0 * (b[1] - cy), cx + 2.0 * (b[2] - cx), cy + 2.0 * (b[3] - cy)]) {
            assert!((got - want).abs() < 0.05, "{n:?}");
        }
        // Rotate 90° from the menu, about the same centre.
        assert!(invoke(&mut a, "modify.rotate_cw"));
        assert_eq!(a.s.history_len(), steps + 2);
    }

    #[test]
    fn onion_skin_and_edit_multiple_frames_work_inside_a_symbol() {
        let mut a = app();
        a.open_panel(Some("onionsymbol"));
        assert_eq!(a.s.editing.len(), 1, "the star is open in place");
        assert!(stage_skin(&a).is_some(), "onion skin draws inside it");
        let pic = a.s.stage_picture();
        assert!(pic.front.is_some());
        toggle_edit_multiple(&mut a);
        assert!(editing(&a));
        select_all(&mut a);
        // The star's own keyframes (ADR 0051).
        let frames: Vec<u32> = a.onion_ui.sel.iter().map(|it| it.frame).collect();
        assert!(frames.len() >= 2 && frames.iter().all(|f| a.s.timeline().layers.iter().any(|l| l.keyframe_starting_at(*f).is_some())), "{frames:?}");
        nudge(&mut a, Vec2::new(5.0, 0.0));
        assert_eq!(a.onion_ui.sel.len(), frames.len(), "the selection stays");
    }

    fn srgb_of(c: Rgba) -> [u8; 3] {
        let [r, g, b, _] = to_color32(c).to_srgba_unmultiplied();
        [r, g, b]
    }
}
