//! Shape hint markers on the stage (ADR 0023). The hints of the active layer's shape tween
//! show on the first frame of its first keyframe (where they sit on the start shape) and of
//! its last keyframe (where they sit on the end shape). A marker is a lettered dot: rose
//! while it isn't on an edge, amber once placed on the first keyframe, mint once placed on
//! the last. Drag one with the Selection tools to place it (it snaps to the nearest edge,
//! live); right-click it to remove it.

use crate::app::App;
use crate::theme::{T, Type, font};
use anicraft_engine::doc::{HintSide, hint_letter, placed_on};
use anicraft_engine::geom::Vec2;
use egui::{Align2, Painter, Pos2, Stroke};
use serde_json::json;

/// Marker radius in screen points (a 16-point dot; the grab area is a little larger).
pub const RADIUS: f32 = 8.0;
const GRAB: f32 = RADIUS + 3.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Marker {
    pub index: usize,
    pub side: HintSide,
    /// Stage position.
    pub at: Vec2,
    /// On an edge of the keyframe it is shown on.
    pub placed: bool,
}

/// The markers the stage shows now (none mid-tween, on hidden layers or without hints).
pub fn markers(app: &App) -> Vec<Marker> {
    let frame = app.s.frame;
    let Some(layer) = app.s.active_layer().filter(|l| l.visible) else { return Vec::new() };
    let Some((i, side)) = layer.hint_tween_at(frame) else { return Vec::new() };
    let (Some(first), Some(shown)) = (layer.keyframes.get(i), layer.hint_keyframe(i, side)) else { return Vec::new() };
    if shown.start != frame {
        return Vec::new();
    }
    first
        .hints
        .iter()
        .enumerate()
        .map(|(index, h)| {
            let at = h.at(side);
            Marker { index, side, at, placed: placed_on(&shown.elements, at).is_some() }
        })
        .collect()
}

/// The marker under screen point `p` (the one drawn on top when they overlap).
pub fn hit(markers: &[Marker], p: Pos2, to_screen: impl Fn(Vec2) -> Pos2) -> Option<Marker> {
    markers.iter().rev().find(|m| to_screen(m.at).distance(p) <= GRAB).copied()
}

pub fn draw(painter: &Painter, markers: &[Marker], to_screen: impl Fn(Vec2) -> Pos2, hot: Option<usize>) {
    for m in markers {
        let c = to_screen(m.at);
        let fill = match (m.placed, m.side) {
            (false, _) => T.hint_unplaced,
            (true, HintSide::Start) => T.hint_start,
            (true, HintSide::End) => T.hint_end,
        };
        let r = if hot == Some(m.index) { RADIUS + 1.5 } else { RADIUS };
        painter.circle(c, r, fill, Stroke::new(1.0, T.ink));
        painter.text(c, Align2::CENTER_CENTER, hint_letter(m.index), font(Type::Data), T.ink);
    }
}

/// The right-click menu over a marker; false when the click wasn't on one.
pub fn menu(app: &mut App, ui: &mut egui::Ui) -> bool {
    let Some(i) = app.hint_menu else { return false };
    if ui.button(tr!("Remove Hint {letter}", letter = hint_letter(i))).clicked() {
        if app.run("tween.remove_hint", json!({"hint": i})).is_some() {
            app.status_ok(tr!("Removed shape hint {letter}. {undo} to undo.", letter = hint_letter(i), undo = app.undo_key()));
        }
        ui.close();
    }
    crate::menus::cmd_item(app, ui, "tween.clear_hints");
    ui.separator();
    crate::menus::cmd_item(app, ui, "tween.add_hint");
    true
}

/// Add Shape Hint, with a pointer to what to do next.
pub fn add(app: &mut App) {
    if let Some(v) = app.run("tween.add_hint", json!({})) {
        let letter = v["hint"].as_str().unwrap_or("a").to_string();
        app.status_ok(tr!("Added shape hint {letter}: drag it onto an edge here, then onto the matching edge on the tween's last keyframe.", letter = letter));
    }
}
