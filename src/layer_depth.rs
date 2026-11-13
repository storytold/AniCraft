//! Window › Layer Depth (ADR 0082): the scene's layers listed by depth, nearest first, each with
//! a track to drag it nearer (left) or farther (right), its depth in pixels, a key toggle that
//! animates the depth over time, and Attach to Camera. The track is laid out by how far a layer
//! moves when the camera pans (`FOCAL / (FOCAL + depth)`), so equal distances on it look like
//! equal steps in parallax, and the busy range near the stage gets the room. Each drag is one
//! undo step; edits show on the stage in the same frame.

use crate::a11y::{self, Named};
use crate::app::App;
use crate::theme::{T, TARGET, Type, text, to_color32};
use anicraft_engine::doc::camera::{FOCAL, MAX_DEPTH, MIN_DEPTH, parallax};
use anicraft_engine::doc::{Layer, LayerKind};
use egui::{Sense, Stroke, StrokeKind, pos2, vec2};
use serde_json::json;

/// Whether a layer of this kind has a depth (the camera, folders and guides don't).
pub fn has_depth(kind: LayerKind) -> bool {
    !matches!(kind, LayerKind::Camera | LayerKind::Folder | LayerKind::Guide)
}

/// Where depth `d` sits on the track, `0` (nearest) to `1` (farthest).
pub fn track_pos(d: f32) -> f32 {
    let (near, far) = (parallax(MIN_DEPTH), parallax(MAX_DEPTH));
    ((near - parallax(d)) / (near - far)).clamp(0.0, 1.0)
}

/// The depth at track position `u` (the inverse of [`track_pos`]), rounded to whole pixels.
pub fn track_depth(u: f32) -> f32 {
    let (near, far) = (parallax(MIN_DEPTH), parallax(MAX_DEPTH));
    let u = if u.is_finite() { u.clamp(0.0, 1.0) } else { 0.0 };
    let p = near - u * (near - far);
    (FOCAL / p - FOCAL).round().clamp(MIN_DEPTH, MAX_DEPTH)
}

/// The Depth row in Properties for the active layer: the value at the playhead, a key toggle,
/// and Attach to Camera.
pub fn depth_row(app: &mut App, ui: &mut egui::Ui, layer: &Layer, frame: u32) {
    let mut depth = layer.depth_at(frame);
    ui.horizontal(|ui| {
        ui.add_sized(vec2(84.0, TARGET * 0.8), egui::Label::new(text(Type::Label, tr!("Depth"))));
        let r = ui.add(egui::DragValue::new(&mut depth).suffix(" px").speed(2.0).range(MIN_DEPTH..=MAX_DEPTH).max_decimals(0)).named(tr!("Layer depth"));
        if r.on_hover_text(tr!("How far behind the stage this layer is: through the camera a deeper layer moves and zooms less, and draws behind nearer ones. Negative is in front.")).changed() {
            crate::panels::live(app, tr_noop!("Layer Depth"), "layer.set_depth", json!({"layer": layer.id.0, "depth": depth}));
        }
        key_button(app, ui, layer, frame);
    });
    let mut attached = layer.attached;
    if crate::theme::checkbox(ui, &mut attached, tr!("Attach layer to camera")).on_hover_text(tr!("The layer stays where it is on screen while the camera moves: for titles and HUDs")).changed() {
        app.run("layer.set_attached", json!({"layer": layer.id.0, "attached": attached}));
    }
}

/// The diamond that keys the layer's depth at the playhead (filled: a key here), or removes it.
fn key_button(app: &mut App, ui: &mut egui::Ui, layer: &Layer, frame: u32) {
    let here = layer.depth_keys.iter().any(|k| k.frame == frame);
    let keyed = !layer.depth_keys.is_empty();
    let (rect, resp) = ui.allocate_exact_size(vec2(TARGET * 0.8, TARGET * 0.8), Sense::click());
    let c = rect.center();
    let r = rect.height() * 0.28;
    let pts = vec![c - vec2(0.0, r), c + vec2(r, 0.0), c + vec2(0.0, r), c - vec2(r, 0.0)];
    let color = if resp.hovered() { T.icon_hot } else if keyed { T.accent } else { T.icon };
    let fill = if here { color } else { egui::Color32::TRANSPARENT };
    ui.painter().add(egui::Shape::convex_polygon(pts, fill, Stroke::new(1.5, color)));
    let tip = if here { tr!("Remove the depth key on this frame") } else { tr!("Key the depth on this frame: the depth then tweens between keys") };
    a11y::control(ui, &resp, egui::WidgetType::Button, tr!("Depth key"), Some(here));
    if resp.on_hover_text(tip).clicked() {
        if here {
            app.run("layer.remove_depth_key", json!({"layer": layer.id.0, "frame": frame}));
        } else {
            app.run("layer.set_depth", json!({"layer": layer.id.0, "depth": layer.depth_at(frame), "key": true, "frame": frame}));
        }
    }
}

/// The panel.
pub fn panel(app: &mut App, ui: &mut egui::Ui) {
    if !app.s.editing.is_empty() {
        ui.label(text(Type::Caption, tr!("Layer depth belongs to a scene's layers. Edit the document (Edit › Edit Document) to set it.")));
        return;
    }
    let frame = app.s.frame;
    let tl = app.s.doc.timeline(app.s.scene).clone();
    if tl.camera_layer().is_none() {
        ui.label(text(Type::Caption, tr!("Depth works through the camera: deeper layers move and zoom less as it moves.")));
        if ui.button(tr!("Add Camera")).named(tr!("Add camera")).clicked() {
            app.run("camera.add", json!({}));
        }
        return;
    }
    // Nearest first; ties in timeline order (top first), as `layer.depths` lists them.
    let mut rows: Vec<(usize, &Layer)> = tl.layers.iter().enumerate().filter(|(_, l)| has_depth(l.kind)).map(|(i, l)| (i, &**l)).collect();
    rows.sort_by(|a, b| a.1.depth_at(frame).total_cmp(&b.1.depth_at(frame)).then(a.0.cmp(&b.0)));
    ui.horizontal(|ui| {
        ui.label(text(Type::Caption, tr!("Nearer")));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(text(Type::Caption, tr!("Farther"))));
    });
    for (_, layer) in rows {
        ui.push_id(layer.id.0, |ui| layer_row(app, ui, layer, frame));
    }
    ui.add_space(4.0);
    ui.label(text(Type::Caption, tr!("Drag a layer along its track, or type its depth. The diamond keys the depth on this frame.")));
}

/// One layer: its colour and name, the track with its handle, the value, the key and Attach.
fn layer_row(app: &mut App, ui: &mut egui::Ui, layer: &Layer, frame: u32) {
    let depth = layer.depth_at(frame);
    let active = app.s.layer == layer.id;
    let label = tr!("Depth of {layer}", layer = layer.name);
    // First line: the layer's colour and name; its value, key and Attach on the right.
    ui.horizontal(|ui| {
        let (chip, _) = ui.allocate_exact_size(vec2(10.0, TARGET * 0.8), Sense::hover());
        ui.painter().rect_filled(egui::Rect::from_center_size(chip.center(), vec2(8.0, 8.0)), 2.0, to_color32(layer.color));
        let name = ui.add(egui::Label::new(text(Type::Body, layer.name.clone()).color(if active { T.text } else { T.text_dim })).truncate().sense(Sense::click()));
        if name.clicked() {
            app.run("layer.select", json!({"layer": layer.id.0}));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut attached = layer.attached;
            if ui.toggle_value(&mut attached, text(Type::Label, tr!("Attach"))).named(&tr!("Attach {layer} to camera", layer = layer.name)).on_hover_text(tr!("Attach Layer to Camera: the layer stays where it is on screen while the camera moves")).changed() {
                app.run("layer.set_attached", json!({"layer": layer.id.0, "attached": attached}));
            }
            key_button(app, ui, layer, frame);
            let mut v = depth;
            if ui.add(egui::DragValue::new(&mut v).suffix(" px").speed(2.0).range(MIN_DEPTH..=MAX_DEPTH).max_decimals(0)).named(&label).changed() {
                crate::panels::live(app, tr_noop!("Layer Depth"), "layer.set_depth", json!({"layer": layer.id.0, "depth": v}));
            }
        });
    });
    // Second line: the track, the panel's width. Drag the handle (or click on the track).
    ui.horizontal(|ui| {
        ui.add_space(14.0);
        let width = (ui.available_width() - 4.0).max(48.0);
        let (track, resp) = ui.allocate_exact_size(vec2(width, TARGET * 0.6), Sense::click_and_drag());
        let p = ui.painter();
        let y = track.center().y;
        p.line_segment([pos2(track.left(), y), pos2(track.right(), y)], Stroke::new(2.0, T.border));
        // The stage plane, marked.
        let x0 = track.left() + track_pos(0.0) * track.width();
        p.line_segment([pos2(x0, y - 5.0), pos2(x0, y + 5.0)], Stroke::new(1.0, T.text_faint));
        let hx = track.left() + track_pos(depth) * track.width();
        let hot = resp.hovered() || resp.dragged();
        let handle = egui::Rect::from_center_size(pos2(hx, y), vec2(8.0, track.height() * 0.8));
        let fill = if layer.attached { T.text_faint } else { to_color32(layer.color) };
        p.rect(handle, 2.0, fill, Stroke::new(if hot { 1.5 } else { 1.0 }, if hot { T.icon_hot } else { T.border }), StrokeKind::Inside);
        a11y::control(ui, &resp, egui::WidgetType::Slider, &label, None);
        if (resp.dragged() || resp.clicked())
            && let Some(at) = resp.interact_pointer_pos()
        {
            let d = track_depth((at.x - track.left()) / track.width().max(1.0));
            if d != depth {
                crate::panels::live(app, tr_noop!("Layer Depth"), "layer.set_depth", json!({"layer": layer.id.0, "depth": d}));
            }
        }
        if resp.drag_stopped() {
            app.s.end_gesture();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_track_runs_nearest_to_farthest_and_back() {
        assert_eq!(track_pos(MIN_DEPTH), 0.0);
        assert_eq!(track_pos(MAX_DEPTH), 1.0);
        assert!(track_pos(0.0) > 0.4 && track_pos(0.0) < 0.6, "the stage plane is mid-track: {}", track_pos(0.0));
        for d in [-500.0, -120.0, 0.0, 37.0, 1000.0, 4321.0, 10_000.0] {
            assert!((track_depth(track_pos(d)) - d).abs() <= 1.0, "{d}");
        }
        assert_eq!(track_depth(f32::NAN), MIN_DEPTH);
    }
}
