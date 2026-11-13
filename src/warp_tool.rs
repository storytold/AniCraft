//! The Asset Warp tool (ADR 0079): pins on shapes and bitmaps that bend them.
//!
//! With the tool in hand every warped shape or bitmap on the stage shows its mesh (faint, in
//! its layer's colour) and its pins. A click on artwork pins it there (the first pin covers it
//! with a warp mesh) and picks the pin; a click on a pin picks it; a click on empty stage drops
//! the pick. Dragging a pin bends the artwork around the other pins, live; dragging artwork
//! where there is no pin adds one there and drags it. Either drag is one undo step. Delete (or
//! Backspace) removes the picked pin, Esc drops the pick. Everything goes through the `warp.*`
//! commands.

use crate::app::{App, Drag, Tool};
use crate::pathtools::Xf;
use crate::theme::{T, to_color32};
use anicraft_engine::doc::warp::PinId;
use anicraft_engine::doc::{ElementId, ElementKind, LayerId, LayerKind, evaluate_layer_elements};
use anicraft_engine::geom::{Affine, Vec2};
use egui::{Color32, Key, Modifiers, Pos2, Stroke};
use serde_json::json;

/// How close (screen points) a press must be to a pin to grab it, and the pin's radius.
const GRAB: f32 = 8.0;
const RADIUS: f32 = 4.5;

/// One warped artwork as the stage shows it at the playhead.
struct Shown {
    element: ElementId,
    color: Color32,
    locked: bool,
    /// The mesh's edges, in stage space.
    edges: Vec<(Vec2, Vec2)>,
    /// Pins: id and where they show (stage).
    pins: Vec<(PinId, Vec2)>,
}

fn shown(app: &App) -> Vec<Shown> {
    let tl = app.s.timeline();
    let mut out = Vec::new();
    for (i, l) in tl.layers.iter().enumerate() {
        if !l.visible || tl.hidden_by_folder(i) || !matches!(l.kind, LayerKind::Normal) {
            continue;
        }
        let Some((_, els)) = evaluate_layer_elements(l, app.s.frame) else { continue };
        for e in els.iter() {
            let ElementKind::Shape(sh) = &e.kind else { continue };
            let Some(w) = sh.live_warp() else { continue };
            let Some(mesh) = w.mesh() else { continue };
            let m: Affine = e.transform;
            let at = |i: u32| mesh.shown.get(i as usize).map(|v| m.apply(*v));
            let mut edges = Vec::with_capacity(mesh.tris.len() * 2);
            for t in &mesh.tris {
                // Each triangle's first two edges draw the grid and its diagonals once.
                if let (Some(a), Some(b), Some(c)) = (at(t[0]), at(t[1]), at(t[2])) {
                    edges.push((a, b));
                    edges.push((b, c));
                }
            }
            let pins = w.pins.iter().map(|p| (p.id, m.apply(p.at))).collect();
            out.push(Shown { element: e.id, color: to_color32(l.color), locked: l.locked || tl.locked_by_folder(l.id), edges, pins });
        }
    }
    out
}

/// The unlocked pin under screen point `p`, topmost first.
fn pin_at(app: &App, xf: Xf, p: Pos2) -> Option<(ElementId, PinId)> {
    shown(app).into_iter().rev().filter(|s| !s.locked).find_map(|s| s.pins.iter().find(|(_, at)| xf.to_screen(*at).distance(p) <= GRAB).map(|(id, _)| (s.element, *id)))
}

/// The topmost shape or bitmap under stage point `sp` on an unlocked normal layer.
fn artwork_at(app: &App, sp: Vec2) -> Option<(LayerId, ElementId)> {
    let dl = app.s.display_list();
    let tl = app.s.timeline();
    for item in dl.items.iter().rev() {
        if item.guide || item.masks.is_some() || item.fill.is_none() {
            continue;
        }
        let Some(layer) = tl.layer(item.layer).filter(|l| l.kind == LayerKind::Normal && !l.locked && !tl.locked_by_folder(l.id)) else { continue };
        if item.clip.is_some_and(|c| !dl.clip_contains(c, sp)) || !item.path.contains(sp, &item.transform, item.fill_rule) {
            continue;
        }
        let shape = evaluate_layer_elements(layer, app.s.frame).is_some_and(|(_, els)| els.iter().any(|e| e.id == item.element && matches!(e.kind, ElementKind::Shape(_))));
        if shape {
            return Some((layer.id, item.element));
        }
        // Something else (a symbol, text) is on top: nothing to warp here.
        return None;
    }
    None
}

/// Pins the artwork under stage point `sp`; picks the new pin.
fn add_pin(app: &mut App, layer: LayerId, el: ElementId, sp: Vec2) -> Option<PinId> {
    if app.s.layer != layer {
        app.run("layer.select", json!({"layer": layer.0}));
    }
    let v = app.run("warp.pin_add", json!({"element": el.0, "x": sp.x, "y": sp.y}))?;
    let pin = PinId(v["pin"].as_u64()?);
    app.warp_pin = Some((el, pin));
    if v["created"].as_bool() == Some(true) {
        app.status_ok(tr!("The artwork has a warp mesh. Add more pins, then drag one to bend it around the others."));
    }
    Some(pin)
}

/// A Warp tool click at screen point `p`.
pub fn click(app: &mut App, xf: Xf, p: Pos2) {
    if let Some(hit) = pin_at(app, xf, p) {
        app.warp_pin = Some(hit);
        return;
    }
    let sp = xf.to_stage(p);
    match artwork_at(app, sp) {
        Some((layer, el)) => {
            add_pin(app, layer, el, sp);
        }
        None => app.warp_pin = None,
    }
}

/// A Warp tool drag from screen point `press`: a pin (or a new one, on artwork) follows the
/// pointer, as one undo step.
pub fn drag_start(app: &mut App, xf: Xf, press: Pos2) -> Drag {
    if let Some((element, pin)) = pin_at(app, xf, press) {
        app.warp_pin = Some((element, pin));
        app.s.begin_gesture(tr_noop!("Move Warp Pin"));
        return Drag::WarpPin { element, pin };
    }
    let sp = xf.to_stage(press);
    let Some((layer, el)) = artwork_at(app, sp) else { return Drag::None };
    app.s.begin_gesture(tr_noop!("Add Warp Pin"));
    match add_pin(app, layer, el, sp) {
        Some(pin) => Drag::WarpPin { element: el, pin },
        None => {
            app.s.end_gesture();
            Drag::None
        }
    }
}

/// Continues a pin drag to stage point `c`.
pub fn drag_to(app: &mut App, element: ElementId, pin: PinId, c: Pos2) {
    app.run("warp.pin_move", json!({"element": element.0, "pin": pin.0, "x": c.x, "y": c.y}));
}

/// Delete removes the picked pin; Esc drops the pick.
pub fn consume_keys(app: &mut App, ctx: &egui::Context) {
    let Some((el, pin)) = app.warp_pin else { return };
    if app.tool != Tool::Warp {
        return;
    }
    let take = |k: Key| ctx.input_mut(|i| i.consume_key(Modifiers::NONE, k));
    if take(Key::Delete) || take(Key::Backspace) {
        if app.run("warp.pin_remove", json!({"element": el.0, "pin": pin.0})).is_some() {
            app.status_ok(tr!("Pin removed. {undo} to undo.", undo = app.undo_key()));
        }
        app.warp_pin = None;
    } else if take(Key::Escape) {
        app.warp_pin = None;
    }
}

/// Drops a picked pin that no longer shows.
pub fn upkeep(app: &mut App) {
    if let Some((el, pin)) = app.warp_pin
        && !shown(app).iter().any(|s| s.element == el && s.pins.iter().any(|(id, _)| *id == pin))
    {
        app.warp_pin = None;
    }
}

/// With the tool in hand: every warped artwork's mesh and pins.
pub fn draw(app: &App, painter: &egui::Painter, xf: Xf) {
    if app.tool != Tool::Warp {
        return;
    }
    for s in shown(app) {
        let fade = |c: Color32| if s.locked { c.gamma_multiply(0.4) } else { c };
        let line = Stroke::new(1.0, fade(s.color.gamma_multiply(0.45)));
        for (a, b) in &s.edges {
            painter.line_segment([xf.to_screen(*a), xf.to_screen(*b)], line);
        }
        for (id, at) in &s.pins {
            let c = xf.to_screen(*at);
            let picked = app.warp_pin == Some((s.element, *id));
            let fill = fade(if picked { T.selection } else { Color32::WHITE });
            painter.circle(c, RADIUS, fill, Stroke::new(1.5, fade(if picked { Color32::WHITE } else { s.color })));
        }
    }
}
