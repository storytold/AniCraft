//! The Layer Properties dialog (ADR 0061), as in Animate: name, show and lock, type, outline
//! colour, View layer as outlines, onion skin and layer height; a mask layer's mask type (ADR
//! 0070: by shape, as in Animate, or soft, by alpha or luminance). Opened by double-clicking a
//! layer's icon, from the layer's right-click menu or Modify › Timeline. OK sends one
//! `layer.set_properties` (one undo step) with what changed; Cancel changes nothing.

use crate::a11y::Named;
use crate::app::App;
use crate::theme::{FORM_LABEL_DIALOG, Type, form_row, text, to_color32};
use anicraft_engine::doc::{LayerId, LayerKind, MAX_LAYER_HEIGHT, Rgba};
use egui::Key;
use serde_json::{Value, json};

/// The dialog while it is open: the layer and the values being edited.
pub struct LayerPropsDialog {
    pub layer: LayerId,
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    /// `normal`, `mask`, `masked`, `guide`, `guided`, or a kind that can't change (`folder`,
    /// `camera`, `armature`).
    pub ty: String,
    pub color: [u8; 3],
    /// A mask layer's mask type: `coverage`, `alpha` or `luminance`.
    pub mask: String,
    pub outline: bool,
    pub onion: bool,
    /// Rows: 1, 2 or 3 (100, 200, 300 %).
    pub height: u8,
    /// What the layer had when the dialog opened: OK sends only what changed.
    was: Value,
    focus: bool,
}

/// The layer's properties as the engine reports them.
fn current(app: &mut App, layer: LayerId) -> Option<Value> {
    app.s.execute("layer.properties", &json!({"layer": layer.0})).ok()
}

/// Opens the dialog for `layer` (default: the active layer).
pub fn open(app: &mut App, layer: Option<LayerId>) {
    let id = layer.unwrap_or(app.s.layer);
    let Some(p) = current(app, id) else { return };
    let Some(l) = app.s.timeline().layer(id) else { return };
    let c = to_color32(l.color);
    app.playing = false;
    app.layer_props = Some(LayerPropsDialog {
        layer: id,
        name: l.name.clone(),
        visible: l.visible,
        locked: l.locked,
        ty: p.get("type").and_then(Value::as_str).unwrap_or("normal").to_string(),
        color: [c.r(), c.g(), c.b()],
        mask: l.mask_mode.key().to_string(),
        outline: l.outline,
        onion: l.onion,
        height: l.height(),
        was: p,
        focus: true,
    });
}

/// The types a layer can be given here, with their names.
fn types() -> [(&'static str, &'static str); 5] {
    [("normal", tr!("Normal")), ("guide", tr!("Guide")), ("guided", tr!("Guided")), ("mask", tr!("Mask")), ("masked", tr!("Masked"))]
}

/// The mask types, with their names and what each does.
fn mask_types() -> [(&'static str, &'static str, &'static str); 3] {
    [
        ("coverage", tr!("Shape"), tr!("The mask's fills reveal fully, whatever their colour or opacity (as in Animate)")),
        ("alpha", tr!("Alpha"), tr!("The mask's opacity reveals: gradients to transparent and faded shapes give soft edges")),
        ("luminance", tr!("Luminance"), tr!("The mask's brightness reveals: white shows, black hides, greys show partly")),
    ]
}

fn type_label(ty: &str) -> &'static str {
    match ty {
        "folder" => tr!("Folder"),
        "camera" => tr!("Camera"),
        "armature" => tr!("Armature"),
        other => types().into_iter().find(|(k, _)| *k == other).map_or(tr!("Normal"), |(_, n)| n),
    }
}

pub fn window(app: &mut App, ctx: &egui::Context) {
    if app.layer_props.as_ref().is_some_and(|d| app.s.timeline().layer(d.layer).is_none()) {
        // The layer went away (undo, another edit): nothing to set.
        app.layer_props = None;
    }
    let Some(d) = &mut app.layer_props else { return };
    // What each type needs: a mask or guide right above for masked and guided.
    let (can_mask, can_guide, kind) = {
        let tl = app.s.timeline();
        let i = tl.layers.iter().position(|l| l.id == d.layer);
        let kind = i.and_then(|i| tl.layers.get(i)).map_or(LayerKind::Normal, |l| l.kind);
        (i.and_then(|i| tl.mask_above(i)).is_some(), i.and_then(|i| tl.guide_above(i)).is_some(), kind)
    };
    let fixed = matches!(kind, LayerKind::Folder | LayerKind::Camera | LayerKind::Armature);
    let (mut submit, mut cancel) = (false, false);
    egui::Window::new(tr!("Layer Properties")).id(egui::Id::new("layer_props")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, -80.0]).show(ctx, |ui| {
        ui.set_min_width(420.0);
        form_row(ui, FORM_LABEL_DIALOG, tr!("Name"), |ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut d.name).desired_width(f32::INFINITY)).named(tr!("Layer name"));
            if d.focus {
                r.request_focus();
                d.focus = false;
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                submit = true;
            }
        });
        form_row(ui, FORM_LABEL_DIALOG, tr!("Layer"), |ui| {
            crate::theme::checkbox(ui, &mut d.visible, tr!("Show"));
            crate::theme::checkbox(ui, &mut d.locked, tr!("Lock"));
        });
        form_row(ui, FORM_LABEL_DIALOG, tr!("Type"), |ui| {
            if fixed {
                ui.label(text(Type::Body, type_label(&d.ty)));
                return;
            }
            ui.vertical(|ui| {
                for (k, name) in types() {
                    let allowed = match k {
                        "masked" => can_mask || d.ty == k,
                        "guided" => can_guide || d.ty == k,
                        _ => true,
                    };
                    let r = ui.add_enabled(allowed, egui::RadioButton::new(d.ty == k, name));
                    let r = if allowed {
                        r
                    } else if k == "masked" {
                        r.on_disabled_hover_text(tr!("Only a layer right under a mask layer can be masked"))
                    } else {
                        r.on_disabled_hover_text(tr!("Only a layer right under a motion guide can be guided"))
                    };
                    if r.clicked() {
                        d.ty = k.to_string();
                    }
                }
            });
        });
        if d.ty == "mask" {
            form_row(ui, FORM_LABEL_DIALOG, tr!("Mask type"), |ui| {
                ui.horizontal(|ui| {
                    for (k, name, what) in mask_types() {
                        if ui.radio(d.mask == k, name).on_hover_text(what).clicked() {
                            d.mask = k.to_string();
                        }
                    }
                });
            });
        }
        form_row(ui, FORM_LABEL_DIALOG, tr!("Outline colour"), |ui| {
            crate::colour::button(ui, &mut d.color).named(tr!("Outline colour")).on_hover_text(tr!("The colour of the layer's outlines and of its bar on the timeline"));
        });
        if kind != LayerKind::Camera && kind != LayerKind::Folder {
            form_row(ui, FORM_LABEL_DIALOG, tr!("Outlines"), |ui| {
                crate::theme::checkbox(ui, &mut d.outline, tr!("View layer as outlines")).on_hover_text(tr!("Draw the layer's artwork as outlines in its colour, on the stage only (never in the movie or an export)"));
            });
            form_row(ui, FORM_LABEL_DIALOG, tr!("Onion skin"), |ui| {
                crate::theme::checkbox(ui, &mut d.onion, tr!("Onion skin this layer")).on_hover_text(tr!("Show this layer's frames around the playhead while onion skin is on"));
            });
        }
        form_row(ui, FORM_LABEL_DIALOG, tr!("Layer height"), |ui| {
            egui::ComboBox::from_id_salt("layer_props_height")
                .width(96.0)
                .selected_text(text(Type::Data, format!("{} %", u32::from(d.height) * 100)))
                .show_ui(ui, |ui| {
                    for h in 1..=MAX_LAYER_HEIGHT {
                        if ui.selectable_label(d.height == h, text(Type::Data, format!("{} %", u32::from(h) * 100))).clicked() {
                            d.height = h;
                        }
                    }
                })
                .response
                .set_name(tr!("Layer height"));
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.button(tr!("OK")).clicked() {
                submit = true;
            }
            if ui.button(tr!("Cancel")).clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                cancel = true;
            }
        });
    });
    if submit {
        accept(app);
    } else if cancel {
        app.layer_props = None;
    }
}

/// The params OK sends: the layer, and every property that differs from when the dialog opened.
pub fn changes(d: &LayerPropsDialog) -> Value {
    let [r, g, b] = d.color;
    let color = Rgba::new(f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0, 1.0).to_hex();
    let now = json!({
        "name": d.name.trim(),
        "visible": d.visible,
        "locked": d.locked,
        "type": d.ty,
        "color": color,
        "outline": d.outline,
        "onion": d.onion,
        "height": u32::from(d.height) * 100,
    });
    let mut now = now;
    // The mask type is a mask layer's (and only sent when it changes).
    if d.ty == "mask" {
        now["mask"] = json!(d.mask);
    }
    let mut out = json!({"layer": d.layer.0});
    if let (Some(now), Some(o)) = (now.as_object(), out.as_object_mut()) {
        for (k, v) in now {
            if d.was.get(k) != Some(v) {
                o.insert(k.clone(), v.clone());
            }
        }
    }
    out
}

/// OK: one `layer.set_properties` with what changed (nothing changed: nothing sent).
fn accept(app: &mut App) {
    let Some(d) = app.layer_props.take() else { return };
    let p = changes(&d);
    if p.as_object().is_some_and(|o| o.len() > 1) {
        app.request("layer.set_properties", p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anicraft_engine::Session;
    use anicraft_engine::doc::sample;

    #[test]
    fn ok_sets_what_changed_as_one_step_and_cancel_nothing() {
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, Session::new(sample::demo()), Some(std::path::PathBuf::from("unused.png")));
        let id = app.s.layer;
        open(&mut app, None);
        let d = app.layer_props.as_mut().expect("the dialog opens on the active layer");
        assert_eq!(d.layer, id);
        assert_eq!(changes(d), json!({"layer": id.0}), "nothing changed yet");
        d.name = "  Hero ".into();
        d.height = 2;
        d.outline = true;
        assert_eq!(changes(d), json!({"layer": id.0, "name": "Hero", "height": 200, "outline": true}));
        let steps = app.s.history().undo_len();
        accept(&mut app);
        assert!(app.layer_props.is_none());
        let l = app.s.timeline().layer(id).unwrap();
        assert_eq!((l.name.as_str(), l.height(), l.outline), ("Hero", 2, true));
        assert_eq!(app.s.history().undo_len(), steps + 1, "one undo step");
        // OK with nothing changed sends nothing.
        open(&mut app, Some(id));
        accept(&mut app);
        assert_eq!(app.s.history().undo_len(), steps + 1);
        // A mask layer's mask type, by itself.
        app.s.execute("layer.set_properties", &json!({"layer": id.0, "type": "mask"})).unwrap();
        open(&mut app, Some(id));
        let d = app.layer_props.as_mut().unwrap();
        assert_eq!(changes(d), json!({"layer": id.0}));
        d.mask = "alpha".into();
        assert_eq!(changes(d), json!({"layer": id.0, "mask": "alpha"}));
        accept(&mut app);
        assert_eq!(app.s.timeline().layer(id).unwrap().mask_mode, anicraft_engine::doc::MaskMode::Alpha);
        // The menu command opens it too.
        app.invoke("layer.set_properties");
        assert!(app.layer_props.is_some());
    }
}
