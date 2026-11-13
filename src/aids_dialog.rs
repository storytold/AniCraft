//! View › Grid › Edit Grid and View › Guides › Edit Guides (ADR 0076), as in Animate: colour,
//! show, snap and snap accuracy, the grid's spacing and whether it shows over the artwork. Edit
//! Guides also lists this scene's guides by position and adds new ones, so guides can be made,
//! moved and deleted from the keyboard. Each dialog works on a copy: OK applies it as one undo
//! step, Cancel (or Esc) leaves everything as it was.

use crate::a11y::Named;
use crate::app::App;
use crate::theme::{FORM_LABEL_DIALOG, Type, form_row, text, with_type};
use anicraft_engine::Accuracy;
use anicraft_engine::doc::{Guide, GuideAxis, MAX_GUIDES, Rgba};
use egui::Key;
use serde_json::json;

/// Edit Grid while it is open.
#[derive(Clone, Debug, PartialEq)]
pub struct GridDialog {
    pub color: [u8; 4],
    pub show: bool,
    pub over_objects: bool,
    pub snap: bool,
    pub spacing_x: f32,
    pub spacing_y: f32,
    pub accuracy: Accuracy,
}

/// Edit Guides while it is open.
#[derive(Clone, Debug, PartialEq)]
pub struct GuidesDialog {
    pub color: [u8; 4],
    pub show: bool,
    pub lock: bool,
    pub snap: bool,
    pub accuracy: Accuracy,
    /// This scene's guides as they will be after OK.
    pub guides: Vec<Guide>,
    /// The guide to add next.
    pub new_axis: GuideAxis,
    pub new_at: f32,
    /// The position field takes the keyboard on the first frame (View › Guides › Add Guide).
    pub focus_new: bool,
}

/// The dialogs that are open.
#[derive(Default)]
pub struct AidsDialogs {
    pub grid: Option<GridDialog>,
    pub guides: Option<GuidesDialog>,
}

fn to_u8(c: Rgba) -> [u8; 4] {
    let c = c.sanitized();
    [c.r, c.g, c.b, c.a].map(|v| (v * 255.0).round() as u8)
}

fn hex(c: [u8; 4]) -> String {
    crate::colour::to_hex(c, true)
}

/// Opens Edit Grid with the grid as it is.
pub fn open_grid(app: &mut App) {
    let (a, g) = (app.s.aids, app.s.doc.aids.grid);
    app.aids_dialogs.grid = Some(GridDialog { color: to_u8(g.color), show: a.show_grid, over_objects: g.over_objects, snap: a.snap_grid, spacing_x: g.spacing_x, spacing_y: g.spacing_y, accuracy: a.grid_accuracy });
}

/// Opens Edit Guides with this scene's guides; `add` puts the keyboard on the new guide's
/// position (View › Guides › Add Guide), at the middle of the stage.
pub fn open_guides(app: &mut App, add: bool) {
    let a = app.s.aids;
    let st = app.s.doc.stage;
    app.aids_dialogs.guides = Some(GuidesDialog {
        color: to_u8(app.s.doc.aids.guide_color),
        show: a.show_guides,
        lock: a.lock_guides,
        snap: a.snap_guides,
        accuracy: a.guide_accuracy.min_guides(),
        guides: app.s.guides().to_vec(),
        new_axis: GuideAxis::Vertical,
        new_at: (st.width / 2) as f32,
        focus_new: add,
    });
}

trait GuideAccuracy {
    fn min_guides(self) -> Self;
}

impl GuideAccuracy for Accuracy {
    /// Guides never "always" snap.
    fn min_guides(self) -> Accuracy {
        if self == Accuracy::Always { Accuracy::Distant } else { self }
    }
}

fn accuracy_combo(ui: &mut egui::Ui, id: &str, a: &mut Accuracy, choices: &[Accuracy]) {
    egui::ComboBox::from_id_salt(id)
        .width(160.0)
        .selected_text(crate::i18n::t(a.label()))
        .show_ui(ui, |ui| {
            for c in choices {
                if ui.selectable_label(a == c, crate::i18n::t(c.label())).clicked() {
                    *a = *c;
                }
            }
        })
        .response
        .set_name(tr!("Snap accuracy"));
}

/// The names the dialogs show for the accuracies (so the catalogs list them).
const _ACCURACY_NAMES: [&str; 4] = [tr_noop!("Must be close"), tr_noop!("Normal"), tr_noop!("Can be distant"), tr_noop!("Always snap")];

fn buttons(ui: &mut egui::Ui) -> (bool, bool) {
    let (mut ok, mut cancel) = (false, false);
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui.button(tr!("OK")).clicked() {
            ok = true;
        }
        if ui.button(tr!("Cancel")).clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
            cancel = true;
        }
    });
    (ok, cancel)
}

fn spacing_field(ui: &mut egui::Ui, v: &mut f32, name: &str) {
    use anicraft_engine::doc::aids::{GRID_MAX, GRID_MIN};
    with_type(ui, Type::Data, |ui| ui.add(egui::DragValue::new(v).range(GRID_MIN..=GRID_MAX).suffix(" px").speed(0.5).max_decimals(2)).named(name));
}

pub fn window(app: &mut App, ctx: &egui::Context) {
    grid_window(app, ctx);
    guides_window(app, ctx);
}

fn grid_window(app: &mut App, ctx: &egui::Context) {
    let Some(d) = &mut app.aids_dialogs.grid else { return };
    let (mut ok, mut cancel) = (false, false);
    egui::Window::new(tr!("Edit Grid")).id(egui::Id::new("edit_grid")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, -80.0]).show(ctx, |ui| {
        ui.set_min_width(380.0);
        form_row(ui, FORM_LABEL_DIALOG, tr!("Colour"), |ui| {
            crate::colour::button_alpha(ui, &mut d.color).named(tr!("Grid colour"));
        });
        form_row(ui, FORM_LABEL_DIALOG, tr!("Grid"), |ui| {
            ui.vertical(|ui| {
                crate::theme::checkbox(ui, &mut d.show, tr!("Show grid"));
                crate::theme::checkbox(ui, &mut d.over_objects, tr!("Show over objects")).on_hover_text(tr!("Draw the grid over the artwork; off, it shows under it, on the stage"));
                crate::theme::checkbox(ui, &mut d.snap, tr!("Snap to grid"));
            });
        });
        form_row(ui, FORM_LABEL_DIALOG, tr!("Spacing ↔"), |ui| spacing_field(ui, &mut d.spacing_x, tr!("Horizontal spacing")));
        form_row(ui, FORM_LABEL_DIALOG, tr!("Spacing ↕"), |ui| spacing_field(ui, &mut d.spacing_y, tr!("Vertical spacing")));
        form_row(ui, FORM_LABEL_DIALOG, tr!("Snap accuracy"), |ui| accuracy_combo(ui, "grid_accuracy", &mut d.accuracy, &Accuracy::ALL));
        (ok, cancel) = buttons(ui);
    });
    if ok {
        accept_grid(app);
    } else if cancel {
        app.aids_dialogs.grid = None;
    }
}

/// OK in Edit Grid.
pub fn accept_grid(app: &mut App) {
    let Some(d) = app.aids_dialogs.grid.take() else { return };
    let p = json!({"color": hex(d.color), "show": d.show, "over_objects": d.over_objects, "snap": d.snap, "spacing_x": d.spacing_x, "spacing_y": d.spacing_y, "accuracy": d.accuracy.key()});
    app.run("view.grid_set", p);
}

fn guides_window(app: &mut App, ctx: &egui::Context) {
    let Some(d) = &mut app.aids_dialogs.guides else { return };
    let (mut ok, mut cancel) = (false, false);
    egui::Window::new(tr!("Edit Guides")).id(egui::Id::new("edit_guides")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, -80.0]).show(ctx, |ui| {
        ui.set_min_width(420.0);
        form_row(ui, FORM_LABEL_DIALOG, tr!("Colour"), |ui| {
            crate::colour::button_alpha(ui, &mut d.color).named(tr!("Guide colour"));
        });
        form_row(ui, FORM_LABEL_DIALOG, tr!("Guides"), |ui| {
            ui.vertical(|ui| {
                crate::theme::checkbox(ui, &mut d.show, tr!("Show guides"));
                crate::theme::checkbox(ui, &mut d.snap, tr!("Snap to guides"));
                crate::theme::checkbox(ui, &mut d.lock, tr!("Lock guides")).on_hover_text(tr!("Locked guides can't be dragged; new ones can still be added"));
            });
        });
        form_row(ui, FORM_LABEL_DIALOG, tr!("Snap accuracy"), |ui| accuracy_combo(ui, "guide_accuracy", &mut d.accuracy, &[Accuracy::Close, Accuracy::Normal, Accuracy::Distant]));
        ui.separator();
        ui.label(text(Type::Heading, tr!("Guides in this scene")));
        if d.guides.is_empty() {
            ui.label(text(Type::Caption, tr!("None yet. Add one below, or drag one from a ruler (View › Rulers).")));
        }
        let mut remove = None;
        egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
            for (i, g) in d.guides.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    let axis = axis_label(g.axis);
                    ui.add_sized([FORM_LABEL_DIALOG - 8.0, crate::theme::TARGET], egui::Label::new(text(Type::Label, axis)));
                    let name = tr!("{axis} guide {n} position", axis = axis, n = i + 1);
                    with_type(ui, Type::Data, |ui| ui.add(egui::DragValue::new(&mut g.at).range(-anicraft_engine::doc::GUIDE_REACH..=anicraft_engine::doc::GUIDE_REACH).suffix(" px").speed(1.0).max_decimals(2)).named(&name));
                    if ui.button(tr!("Delete")).on_hover_text(tr!("Delete this guide")).named(&tr!("Delete {axis} guide {n}", axis = axis, n = i + 1)).clicked() {
                        remove = Some(i);
                    }
                });
            }
        });
        if let Some(i) = remove.filter(|i| *i < d.guides.len()) {
            d.guides.remove(i);
        }
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("new_guide_axis")
                .width(FORM_LABEL_DIALOG - 16.0)
                .selected_text(axis_label(d.new_axis))
                .show_ui(ui, |ui| {
                    for a in [GuideAxis::Horizontal, GuideAxis::Vertical] {
                        if ui.selectable_label(d.new_axis == a, axis_label(a)).clicked() {
                            d.new_axis = a;
                        }
                    }
                })
                .response
                .set_name(tr!("New guide direction"));
            let r = with_type(ui, Type::Data, |ui| ui.add(egui::DragValue::new(&mut d.new_at).range(-anicraft_engine::doc::GUIDE_REACH..=anicraft_engine::doc::GUIDE_REACH).suffix(" px").speed(1.0).max_decimals(2)).named(tr!("New guide position")));
            if d.focus_new {
                r.request_focus();
                d.focus_new = false;
            }
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            let full = d.guides.len() >= MAX_GUIDES;
            if (ui.add_enabled(!full, egui::Button::new(tr!("Add guide"))).on_hover_text(tr!("Horizontal guides are at y, vertical ones at x, in stage pixels from its top-left corner")).clicked() || (enter && !full))
                && let Some(g) = Guide::new(d.new_axis, d.new_at)
            {
                d.guides.push(g);
            }
        });
        (ok, cancel) = buttons(ui);
    });
    if ok {
        accept_guides(app);
    } else if cancel {
        app.aids_dialogs.guides = None;
    }
}

fn axis_label(a: GuideAxis) -> &'static str {
    match a {
        GuideAxis::Horizontal => tr!("Horizontal"),
        GuideAxis::Vertical => tr!("Vertical"),
    }
}

/// OK in Edit Guides: the settings and the scene's guides as the dialog has them, one undo
/// step.
pub fn accept_guides(app: &mut App) {
    let Some(d) = app.aids_dialogs.guides.take() else { return };
    app.s.begin_gesture(tr_noop!("Edit Guides"));
    app.run("view.guides_set", json!({"color": hex(d.color), "show": d.show, "lock": d.lock, "snap": d.snap, "accuracy": d.accuracy.key()}));
    if app.s.guides() != d.guides.as_slice() {
        app.run("view.guides_clear", json!({}));
        for g in &d.guides {
            app.run("view.guide_add", json!({"axis": g.axis.key(), "at": g.at}));
        }
    }
    app.s.end_gesture();
}

#[cfg(test)]
mod tests {
    use super::*;
    use anicraft_engine::Session;
    use anicraft_engine::doc::Document;

    fn app() -> App {
        let ctx = egui::Context::default();
        App::with_context(&ctx, Session::new(Document::default()), Some(std::path::PathBuf::from("unused.png")))
    }

    #[test]
    fn edit_grid_applies_on_ok_only() {
        let mut app = app();
        open_grid(&mut app);
        let d = app.aids_dialogs.grid.as_mut().unwrap();
        (d.spacing_x, d.spacing_y, d.show, d.snap, d.accuracy) = (32.0, 16.0, true, true, Accuracy::Always);
        d.color = [255, 0, 0, 255];
        app.aids_dialogs.grid = None; // Cancel
        assert_eq!(app.s.doc.aids.grid.spacing_x, 20.0);
        assert!(!app.s.aids.show_grid);
        open_grid(&mut app);
        let d = app.aids_dialogs.grid.as_mut().unwrap();
        (d.spacing_x, d.spacing_y, d.show, d.snap, d.accuracy) = (32.0, 16.0, true, true, Accuracy::Always);
        accept_grid(&mut app);
        let g = app.s.doc.aids.grid;
        assert_eq!((g.spacing_x, g.spacing_y), (32.0, 16.0));
        assert!(app.s.aids.show_grid && app.s.aids.snap_grid && app.s.aids.grid_accuracy == Accuracy::Always);
    }

    #[test]
    fn guides_are_added_moved_and_deleted_from_the_keyboard_in_one_step() {
        let mut app = app();
        app.run("view.guide_add", json!({"axis": "horizontal", "at": 10}));
        open_guides(&mut app, true);
        let d = app.aids_dialogs.guides.as_mut().unwrap();
        assert!(d.focus_new);
        assert_eq!(d.new_at, 640.0, "the stage's middle");
        d.guides[0].at = 15.5;
        d.guides.push(Guide { axis: GuideAxis::Vertical, at: 99.0 });
        d.lock = true;
        accept_guides(&mut app);
        assert_eq!(app.s.guides(), [Guide { axis: GuideAxis::Horizontal, at: 15.5 }, Guide { axis: GuideAxis::Vertical, at: 99.0 }]);
        assert!(app.s.aids.lock_guides, "lock works whatever order it is applied in");
        assert_eq!(app.s.undo_label(), Some("Edit Guides"));
        app.run("edit.undo", json!({}));
        assert_eq!(app.s.guides(), [Guide { axis: GuideAxis::Horizontal, at: 10.0 }], "one undo step");
    }
}
