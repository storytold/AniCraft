//! Eases (ADR 0081): the ease menu every tween shows, and the Custom Ease window.
//!
//! The menu offers what Animate's ease presets do: No Ease, Classic Ease (one strength slider),
//! and the families Quad, Cubic, Quart, Quint, Sine, Back, Circ, Bounce and Elastic, each In,
//! Out and In Out as a curve to click; then the eases saved in the document, the eases of the
//! installed ease plug-ins (ADR 0088), and Edit Ease….
//!
//! The Custom Ease window edits one ease: a classic tween's (or, eased separately, one of its
//! property groups': Position, Rotation, Scale, Colour, Filters), a motion-tween property's
//! segment, or a saved ease. The graph is time across and progress up, overshoot included:
//! click the curve to add a point, drag points and their handles, double-click a point to make
//! it smooth or a corner, right-click it to delete it. With the graph focused the keyboard does
//! the same: `[` `]` pick the previous or next point, the arrows move it (Shift: ten times
//! further), `S` makes it smooth or a corner, `+` adds a point after it, Delete removes it. Its
//! time and value can also be typed. Each drag is one undo step, and every change is an
//! `ease.*` command, so the stage shows it at once.

use crate::a11y::{self, Named};
use crate::app::App;
use crate::theme::{SQUARE, T, Type, font, text, with_type};
use anicraft_engine::cmds_ease::{ease_target, read, saved_name};
use anicraft_engine::doc::{CustomEase, Document, EaseDir, EaseFamily, EaseGroup, Easing, Handle, Property, TweenKind};
use anicraft_engine::params::easing_json;
use egui::{Align2, Key, Modifiers, Pos2, Rect, Sense, Stroke, WidgetType, pos2, vec2};
use serde_json::{Value, json};

/// The graph's height, and the room around the curve inside it.
const GRAPH_H: f32 = 250.0;
const PAD_X: f32 = 34.0;
const PAD_Y: f32 = 16.0;
/// Pointer distance that takes a point, a handle or the curve.
const GRAB: f32 = 8.0;
/// A preset's curve in the ease menu.
const THUMB: egui::Vec2 = egui::Vec2 { x: 44.0, y: 28.0 };

/// What the window edits, as the `ease.*` commands address it.
#[derive(Clone, Debug, PartialEq)]
pub enum EaseTarget {
    /// The classic or shape tween of the span starting at `frame` on `layer`.
    Tween { layer: u64, frame: u32 },
    /// The segment of a motion tween's `property` that `frame` is in.
    Motion { layer: u64, frame: u32, property: Property },
    /// A saved ease, by name.
    Saved(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Drag {
    /// A point or one of its handles; the value range is frozen so the graph doesn't rescale
    /// under the pointer.
    Point { i: usize, lo: f32, hi: f32 },
    Handle { i: usize, h: Handle, lo: f32, hi: f32 },
}

/// The Custom Ease window's state.
pub struct EaseEditor {
    pub target: EaseTarget,
    /// The property group shown, for a tween eased group by group.
    pub group: Option<EaseGroup>,
    pub selected: Option<usize>,
    drag: Option<Drag>,
    /// The name typed to save the ease as.
    save_name: String,
    /// The graph's id last frame (its keys are taken before the app's shortcuts).
    graph: Option<egui::Id>,
}

impl EaseEditor {
    fn new(target: EaseTarget) -> EaseEditor {
        EaseEditor { target, group: None, selected: None, drag: None, save_name: String::new(), graph: None }
    }

    /// The target as command params.
    fn params(&self) -> Value {
        match &self.target {
            EaseTarget::Tween { layer, frame } => match self.group {
                Some(g) => json!({"layer": layer, "frame": frame, "group": g.key()}),
                None => json!({"layer": layer, "frame": frame}),
            },
            EaseTarget::Motion { layer, frame, property } => json!({"layer": layer, "frame": frame, "property": property.key()}),
            EaseTarget::Saved(n) => json!({"saved": n}),
        }
    }

    /// The target's params with `extra`'s.
    fn with(&self, extra: Value) -> Value {
        let mut p = self.params();
        if let (Some(o), Value::Object(e)) = (p.as_object_mut(), extra) {
            o.extend(e);
        }
        p
    }
}

/// Opens the window on the tween at the playhead on the active layer: a classic or shape
/// tween's ease, or the selected property's segment of a motion tween.
pub fn open_here(app: &mut App) {
    let Some(layer) = app.s.active_layer() else { return };
    let Some(k) = layer.keyframe_at(app.s.frame) else {
        app.status_err(tr!("Select a tweened span to edit its ease"));
        return;
    };
    let (lid, start, motion, tween) = (layer.id.0, k.start, k.motion.is_some(), k.tween.is_some());
    if motion {
        let p = app.me_prop;
        open_motion(app, app.s.frame, p);
    } else if tween {
        app.ease = Some(EaseEditor::new(EaseTarget::Tween { layer: lid, frame: start }));
    } else {
        app.status_err(tr!("This span has no tween: create a classic or motion tween to ease it"));
    }
}

/// Opens the window on a motion tween property's segment at `frame` (on the active layer).
pub fn open_motion(app: &mut App, frame: u32, property: Property) {
    let Some(layer) = app.s.active_layer().map(|l| l.id.0) else { return };
    app.ease = Some(EaseEditor::new(EaseTarget::Motion { layer, frame, property }));
}

/// Selects point `i` (screenshots).
pub fn select(app: &mut App, i: usize) {
    if let Some(ed) = app.ease.as_mut() {
        ed.selected = Some(i);
    }
}

/// The ease the window edits, as the document has it now.
fn ease_now(app: &App, ed: &EaseEditor) -> Option<Easing> {
    let at = ease_target(&app.s, &ed.params()).ok()?;
    read(&app.s, &at).ok()
}

/// An ease's name as the menus show it: a saved custom curve by its name.
pub fn label(doc: &Document, e: &Easing) -> String {
    if matches!(e, Easing::Custom(_) | Easing::CubicBezier(..))
        && let Some(n) = saved_name(doc, e)
    {
        return n.to_string();
    }
    match e {
        Easing::Linear => tr!("No Ease").to_string(),
        Easing::Classic(s) => tr!("Classic Ease {strength}", strength = format!("{s:+.0}")),
        Easing::CubicBezier(..) | Easing::Custom(_) => tr!("Custom").to_string(),
        // A plug-in's ease goes by the name its plug-in gave it.
        Easing::Sampled(s) => s.name().to_string(),
        named => named.family().map_or_else(|| tr!("Custom").to_string(), |(f, d)| preset_name(f, d)),
    }
}

fn preset_name(f: EaseFamily, d: EaseDir) -> String {
    tr!("{family} {direction}", family = crate::i18n::t(f.label()), direction = crate::i18n::t(d.label()))
}

/// What was picked in the ease menu.
pub enum Picked {
    /// An ease, as the easing param of `timeline.set_easing`, `motion.set_ease` or `ease.set`.
    Ease(Value),
    /// Edit Ease…: open the Custom Ease window.
    Edit,
}

/// The ease menu (see the module docs), showing `current`; `name` is its accessible name.
pub fn picker(app: &App, ui: &mut egui::Ui, menu: &str, current: &Easing, width: f32, name: &str) -> Option<Picked> {
    let mut out = None;
    let r = egui::ComboBox::from_id_salt(menu).width(width).height(560.0).selected_text(label(&app.s.doc, current)).show_ui(ui, |ui| {
        ui.set_min_width(4.0 * THUMB.x + 80.0);
        if ui.selectable_label(*current == Easing::Linear, tr!("No Ease")).clicked() {
            out = Some(Picked::Ease(json!("Linear")));
        }
        let classic = matches!(current, Easing::Classic(_));
        if ui.selectable_label(classic, tr!("Classic Ease")).on_hover_text(tr!("One strength: −100 eases in, +100 eases out")).clicked() && !classic {
            out = Some(Picked::Ease(json!({"classic": -50})));
        }
        ui.separator();
        for f in EaseFamily::ALL {
            ui.horizontal(|ui| {
                ui.add_sized([60.0, THUMB.y], egui::Label::new(text(Type::Label, crate::i18n::t(f.label()))));
                for d in EaseDir::ALL {
                    let e = Easing::preset(f, d);
                    if thumb(ui, &e, *current == e, &preset_name(f, d)).clicked() {
                        out = Some(Picked::Ease(easing_json(&e)));
                    }
                }
            });
        }
        if !app.s.doc.eases.is_empty() {
            ui.separator();
            ui.label(text(Type::Heading, tr!("Saved eases")));
            for s in app.s.doc.eases.iter() {
                if ui.selectable_label(*current == s.ease, &s.name).clicked() {
                    out = Some(Picked::Ease(json!({"saved": s.name})));
                }
            }
        }
        if let Some(p) = plugin_eases(app, ui, current) {
            out = Some(p);
        }
        ui.separator();
        if ui.button(tr!("Edit Ease…")).on_hover_text(tr!("Shape the curve point by point, save it, copy it to other tweens")).clicked() {
            out = Some(Picked::Edit);
        }
    });
    if app.shot_ease_menu.get() == Some(menu) {
        // A screenshot of the menu open (`--open plugineases`): a click on the button, as a
        // person would (see `App::press_scripted_key`).
        app.shot_ease_menu.set(None);
        app.shot_click.set(Some(r.response.rect.center()));
    }
    r.response.set_name(name);
    out
}

/// The installed ease plug-ins' eases, as curves to click under each plug-in's name.
fn plugin_eases(app: &App, ui: &mut egui::Ui, current: &Easing) -> Option<Picked> {
    let eases = anicraft_engine::cmds_ease::plugin_eases(&app.s);
    if eases.is_empty() {
        return None;
    }
    let mut out = None;
    ui.separator();
    ui.label(text(Type::Heading, tr!("Plug-in eases")));
    let mut i = 0;
    while let Some((plugin, _)) = eases.get(i) {
        let group: Vec<_> = eases[i..].iter().take_while(|(p, _)| p.id() == plugin.id()).collect();
        i += group.len();
        // The plug-in's own name and its eases' names are its text, shown as written.
        ui.label(text(Type::Caption, plugin.manifest().name.clone()));
        ui.horizontal_wrapped(|ui| {
            for (p, c) in group {
                let e = Easing::Sampled(std::sync::Arc::new(anicraft_engine::doc::SampledEase::new(&c.name, p.id(), &c.id, c.values.clone())));
                let on = current.sampled().is_some_and(|s| s.plugin() == p.id() && s.ease() == c.id);
                ui.vertical(|ui| {
                    if thumb(ui, &e, on, &c.name).clicked() {
                        out = Some(Picked::Ease(json!({"plugin": p.id(), "ease": c.id})));
                    }
                    ui.add_sized([THUMB.x + 24.0, 14.0], egui::Label::new(text(Type::Caption, c.name.clone())).truncate());
                });
            }
        });
    }
    out
}

/// A preset as a small curve to click.
fn thumb(ui: &mut egui::Ui, e: &Easing, on: bool, name: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(THUMB, Sense::click());
    let p = ui.painter();
    p.rect_filled(rect, T.rc(), if on { T.accent_dim } else if resp.hovered() { T.hover } else { T.field });
    plot(p, rect.shrink(4.0), e, -0.35, 1.35, Stroke::new(1.5, if on { T.accent_text } else { T.tween }));
    a11y::control(ui, &resp, WidgetType::SelectableLabel, name, Some(on));
    resp.on_hover_text(name)
}

/// `e` drawn into `r`, values `lo..hi` bottom to top.
fn plot(p: &egui::Painter, r: Rect, e: &Easing, lo: f32, hi: f32, stroke: Stroke) {
    let n = (r.width() / 1.5).clamp(16.0, 400.0) as usize;
    let pts: Vec<Pos2> = (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            pos2(r.left() + t * r.width(), r.bottom() - (e.apply(t) - lo) / (hi - lo).max(1e-6) * r.height())
        })
        .collect();
    p.add(egui::Shape::line(pts, stroke));
}

/// A small plot of `e` (the Properties panel's), opening the window when clicked.
pub fn preview(app: &mut App, ui: &mut egui::Ui, e: &Easing) {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width().min(240.0), 70.0), Sense::click());
    let (lo, hi) = value_range(&e.to_custom(), e);
    let p = ui.painter();
    p.rect_filled(r, T.rc(), if resp.hovered() { T.hover } else { T.field });
    let inner = r.shrink(8.0);
    let y = |v: f32| inner.bottom() - (v - lo) / (hi - lo) * inner.height();
    p.line_segment([pos2(inner.left(), y(0.0)), pos2(inner.right(), y(1.0))], Stroke::new(1.0, T.border));
    plot(p, inner, e, lo, hi, Stroke::new(2.0, T.tween));
    a11y::control(ui, &resp, WidgetType::Button, tr!("Edit Ease…"), None);
    if resp.on_hover_text(tr!("Edit the ease: drag its points and handles")).clicked() {
        open_here(app);
    }
}

/// The values the graph shows: the curve, its points and handles, and 0..1, with a margin.
fn value_range(c: &CustomEase, e: &Easing) -> (f32, f32) {
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for i in 0..=200 {
        let v = e.apply(i as f32 / 200.0);
        lo = lo.min(v);
        hi = hi.max(v);
    }
    for p in c.points() {
        for y in [p.at.y, p.at.y + p.h_in.y, p.at.y + p.out.y] {
            lo = lo.min(y);
            hi = hi.max(y);
        }
    }
    let m = (hi - lo) * 0.08;
    (lo - m, hi + m)
}

/// The Custom Ease window, when open.
pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut ed) = app.ease.take() else { return };
    // It closes when what it edits goes away (undo, another scene, a removed tween).
    let Some(ease) = ease_now(app, &ed) else { return };
    let mut open = true;
    let title = tr!("Custom Ease");
    let shown = egui::Window::new(title)
        .id(egui::Id::new("ease_editor"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(560.0)
        .min_width(420.0)
        .show(ctx, |ui| body(app, &mut ed, ui, &ease));
    a11y::name_title_bar(&shown, title);
    let esc = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
    if open && !esc {
        app.ease = Some(ed);
    }
}

/// What is being eased, in words.
fn heading(app: &App, ed: &EaseEditor) -> String {
    match &ed.target {
        EaseTarget::Tween { layer, frame } => {
            let k = app.s.timeline().layer(anicraft_engine::doc::LayerId(*layer)).and_then(|l| l.keyframe_at(*frame).map(|k| (l.name.clone(), k.start, k.end())));
            let (name, a, b) = k.unwrap_or_default();
            tr!("Tween on {layer}, frames {a}–{b}", layer = name, a = a + 1, b = b)
        }
        EaseTarget::Motion { property, frame, .. } => tr!("Motion tween: {property} at frame {frame}", property = crate::i18n::t(property.label()), frame = frame + 1),
        EaseTarget::Saved(n) => tr!("Saved ease “{name}”", name = n),
    }
}

fn body(app: &mut App, ed: &mut EaseEditor, ui: &mut egui::Ui, ease: &Easing) {
    ui.label(text(Type::Heading, heading(app, ed)));
    ui.add_space(4.0);
    // ---- property groups (classic tweens) ----
    if let EaseTarget::Tween { layer, frame } = ed.target {
        let tw = app.s.timeline().layer(anicraft_engine::doc::LayerId(layer)).and_then(|l| l.keyframe_at(frame)).and_then(|k| k.tween.clone());
        if let Some(tw) = tw.filter(|t| t.kind == TweenKind::Classic) {
            let mut separate = !tw.groups.is_empty();
            ui.horizontal_wrapped(|ui| {
                if crate::theme::checkbox(ui, &mut separate, tr!("Ease each property separately")).on_hover_text(tr!("Position, rotation, scale, colour and filters each with their own ease")).changed() {
                    app.run("ease.separate", json!({"separate": separate, "layer": layer, "frame": frame}));
                    ed.group = separate.then_some(EaseGroup::Position);
                    ed.selected = None;
                }
                if separate {
                    ui.add_space(8.0);
                    for g in EaseGroup::ALL {
                        let on = ed.group == Some(g);
                        if ui.selectable_label(on, crate::i18n::t(g.label())).on_hover_text(label(&app.s.doc, tw.ease(g))).clicked() && !on {
                            ed.group = Some(g);
                            ed.selected = None;
                        }
                    }
                }
            });
            if !separate {
                ed.group = None;
            } else if ed.group.is_none() {
                ed.group = Some(EaseGroup::Position);
            }
        }
    }
    // ---- the preset menu, the classic strength ----
    ui.horizontal(|ui| {
        ui.label(text(Type::Label, tr!("Ease")));
        match picker(app, ui, "ease_editor_menu", ease, 170.0, tr!("Ease")) {
            Some(Picked::Ease(e)) => {
                app.run("ease.set", ed.with(json!({"easing": e})));
                ed.selected = None;
            }
            Some(Picked::Edit) | None => {}
        }
        if let Easing::Classic(s) = ease {
            let mut v = *s;
            ui.label(text(Type::Label, tr!("Strength")));
            let r = with_type(ui, Type::Data, |ui| ui.add(egui::DragValue::new(&mut v).range(-100.0..=100.0).speed(1.0).max_decimals(0))).named(tr!("Classic ease strength"));
            if r.changed() {
                crate::panels::live(app, tr_noop!("Ease"), "ease.set", ed.with(json!({"easing": {"classic": v}})));
            }
        }
    });
    ui.add_space(4.0);
    // ---- the graph ----
    graph(app, ed, ui, ease);
    // ---- the selected point, typed ----
    let curve = ease.to_custom();
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        let n = curve.points().len();
        match ed.selected.filter(|i| *i < n).and_then(|i| curve.points().get(i).map(|p| (i, *p))) {
            Some((i, p)) => {
                ui.label(text(Type::Label, tr!("Point {index} of {count}", index = i + 1, count = n)));
                let end = i == 0 || i + 1 == n;
                let (mut x, mut y) = (p.at.x * 100.0, p.at.y * 100.0);
                ui.add_enabled_ui(!end, |ui| {
                    ui.label(text(Type::Label, tr!("Time")));
                    let rx = with_type(ui, Type::Data, |ui| ui.add(egui::DragValue::new(&mut x).range(0.0..=100.0).speed(0.5).max_decimals(1).suffix(" %"))).named(tr!("Point time"));
                    ui.label(text(Type::Label, tr!("Value")));
                    let ry = with_type(ui, Type::Data, |ui| ui.add(egui::DragValue::new(&mut y).range(-1000.0..=1000.0).speed(0.5).max_decimals(1).suffix(" %"))).named(tr!("Point value"));
                    if rx.changed() || ry.changed() {
                        crate::panels::live(app, tr_noop!("Ease point"), "ease.move_point", ed.with(json!({"index": i, "x": x / 100.0, "y": y / 100.0})));
                    }
                    let mut smooth = p.smooth;
                    if crate::theme::checkbox(ui, &mut smooth, tr!("Smooth")).on_hover_text(tr!("Handles in line (S); off: a corner")).changed() {
                        app.run("ease.convert_point", ed.with(json!({"index": i, "smooth": smooth})));
                    }
                    if ui.button(tr!("Delete Point")).on_hover_text(tr!("Delete or right-click; the ends stay")).clicked() {
                        app.run("ease.delete_point", ed.with(json!({"index": i})));
                        ed.selected = None;
                    }
                });
            }
            None => {
                ui.label(text(Type::Caption, tr!("Click the curve to add a point; drag points and handles. Keys: [ ] pick a point, arrows move it, S smooth or corner, + add, Delete remove.")));
            }
        }
    });
    ui.separator();
    // ---- save, copy, paste, reset ----
    ui.horizontal_wrapped(|ui| {
        let saving_saved = matches!(ed.target, EaseTarget::Saved(_));
        if !saving_saved {
            ui.label(text(Type::Label, tr!("Save as")));
            ui.add(egui::TextEdit::singleline(&mut ed.save_name).desired_width(140.0).hint_text(tr!("Name"))).named(tr!("Saved ease name"));
            let name = ed.save_name.trim().to_string();
            let exists = app.s.doc.eases.iter().any(|e| e.name.eq_ignore_ascii_case(&name));
            let button = if exists { tr!("Replace") } else { tr!("Save") };
            if ui.add_enabled(!name.is_empty(), egui::Button::new(button)).on_hover_text(tr!("Keep this ease in the document, with the presets, for any tween")).clicked() {
                app.run("ease.save", ed.with(json!({"name": name, "replace": exists})));
            }
            ui.separator();
        }
        if ui.button(tr!("Copy Ease")).clicked() {
            app.run("ease.copy", ed.params());
        }
        if ui.add_enabled(app.s.ease_clipboard.is_some(), egui::Button::new(tr!("Paste Ease"))).clicked() {
            app.run("ease.paste", ed.params());
            ed.selected = None;
        }
        if ui.button(tr!("Reset")).on_hover_text(tr!("No ease: a steady speed")).clicked() {
            app.run("ease.set", ed.with(json!({"easing": "Linear"})));
            ed.selected = None;
        }
        if let EaseTarget::Saved(n) = ed.target.clone()
            && ui.button(tr!("Delete Saved Ease")).on_hover_text(tr!("Tweens using it keep their curves ({undo} to undo)", undo = app.undo_key())).clicked()
        {
            app.run("ease.delete", json!({"name": n}));
        }
    });
    if !app.s.doc.eases.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.label(text(Type::Caption, tr!("Saved:")));
            for s in app.s.doc.eases.iter().map(|s| s.name.clone()).collect::<Vec<_>>() {
                let on = ed.target == EaseTarget::Saved(s.clone());
                if ui.selectable_label(on, &s).on_hover_text(tr!("Edit this saved ease")).clicked() && !on {
                    ed.target = EaseTarget::Saved(s);
                    ed.group = None;
                    ed.selected = None;
                }
            }
        });
    }
}

/// Screen ↔ ease space of the graph.
#[derive(Clone, Copy)]
struct View {
    inner: Rect,
    lo: f32,
    hi: f32,
}

impl View {
    fn at(&self, x: f32, y: f32) -> Pos2 {
        pos2(self.inner.left() + x * self.inner.width(), self.inner.bottom() - (y - self.lo) / (self.hi - self.lo).max(1e-6) * self.inner.height())
    }
    fn time(&self, px: f32) -> f32 {
        (px - self.inner.left()) / self.inner.width().max(1.0)
    }
    fn value(&self, py: f32) -> f32 {
        self.lo + (self.inner.bottom() - py) / self.inner.height().max(1.0) * (self.hi - self.lo)
    }
}

fn graph(app: &mut App, ed: &mut EaseEditor, ui: &mut egui::Ui, ease: &Easing) {
    let curve = ease.to_custom();
    // A named preset or a plug-in's ease shows as its curve alone (its fitted points would
    // only clutter it); clicking the curve adds a point and makes it a custom ease to shape.
    let preset = ease.family().is_some() || ease.sampled().is_some();
    if preset && ed.drag.is_none() {
        ed.selected = None;
    }
    let pts: &[anicraft_engine::doc::EasePoint] = if preset { &[] } else { curve.points() };
    let w = ui.available_width().max(320.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, GRAPH_H), Sense::click_and_drag());
    ed.graph = Some(resp.id);
    // While the graph has focus the arrows move the picked point, not focus.
    if resp.has_focus() {
        let filter = egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() };
        ui.memory_mut(|m| m.set_focus_lock_filter(resp.id, filter));
    }
    a11y::surface(ui, &resp, &tr!("Ease curve, {count} points. [ and ] pick a point, the arrows move it, S makes it smooth or a corner, + adds one, Delete removes it", count = pts.len()));
    let (lo, hi) = match ed.drag {
        Some(Drag::Point { lo, hi, .. } | Drag::Handle { lo, hi, .. }) => (lo, hi),
        None => value_range(&curve, ease),
    };
    let v = View { inner: Rect::from_min_max(rect.min + vec2(PAD_X, PAD_Y), rect.max - vec2(PAD_Y, PAD_Y)), lo, hi };
    let p = ui.painter_at(rect);
    p.rect_filled(rect, SQUARE, T.field);
    // Time quarters, the start and end values, and the straight line for reference.
    for q in 0..=4 {
        let x = v.at(q as f32 / 4.0, 0.0).x;
        p.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.0, T.separator));
    }
    for (val, tag) in [(0.0, "0%"), (1.0, "100%")] {
        let y = v.at(0.0, val).y;
        p.line_segment([pos2(v.inner.left(), y), pos2(v.inner.right(), y)], Stroke::new(1.0, T.border));
        p.text(pos2(rect.left() + 4.0, y), Align2::LEFT_CENTER, tag, font(Type::DataSmall), T.text_faint);
    }
    p.line_segment([v.at(0.0, 0.0), v.at(1.0, 1.0)], Stroke::new(1.0, T.text_faint));
    plot(&p, v.inner, ease, lo, hi, Stroke::new(2.0, T.tween));
    // The selected point's handles, then every point.
    let sel = ed.selected.filter(|i| *i < pts.len());
    let handles: Vec<(Handle, Pos2)> = sel
        .and_then(|i| pts.get(i).map(|q| (i, q)))
        .map(|(i, q)| {
            let mut hs = Vec::new();
            if i > 0 {
                hs.push((Handle::In, v.at(q.at.x + q.h_in.x, q.at.y + q.h_in.y)));
            }
            if i + 1 < pts.len() {
                hs.push((Handle::Out, v.at(q.at.x + q.out.x, q.at.y + q.out.y)));
            }
            hs
        })
        .unwrap_or_default();
    if let Some(q) = sel.and_then(|i| pts.get(i)) {
        let c = v.at(q.at.x, q.at.y);
        for (_, h) in &handles {
            p.line_segment([c, *h], Stroke::new(1.0, T.text_faint));
            p.circle_filled(*h, 4.5, T.tween_shape);
            p.circle_stroke(*h, 4.5, Stroke::new(1.0, T.ink));
        }
    }
    for (i, q) in pts.iter().enumerate() {
        let c = v.at(q.at.x, q.at.y);
        let r = Rect::from_center_size(c, vec2(9.0, 9.0));
        let picked = sel == Some(i);
        if q.smooth {
            p.circle_filled(c, 5.0, if picked { T.accent } else { T.text });
            p.circle_stroke(c, 5.0, Stroke::new(1.0, T.ink));
        } else {
            p.rect_filled(r, SQUARE, if picked { T.accent } else { T.text });
            p.rect_stroke(r, SQUARE, Stroke::new(1.0, T.ink), egui::StrokeKind::Inside);
        }
    }
    // ---- pointer ----
    let near_point = |pt: Pos2| pts.iter().enumerate().filter(|(_, q)| v.at(q.at.x, q.at.y).distance(pt) <= GRAB).min_by(|a, b| v.at(a.1.at.x, a.1.at.y).distance(pt).total_cmp(&v.at(b.1.at.x, b.1.at.y).distance(pt))).map(|(i, _)| i);
    let near_handle = |pt: Pos2| handles.iter().find(|(_, h)| h.distance(pt) <= GRAB).map(|(w, _)| *w);
    let near_curve = |pt: Pos2| {
        let t = v.time(pt.x);
        (0.0..=1.0).contains(&t) && (v.at(t, ease.apply(t)).y - pt.y).abs() <= GRAB
    };
    if ed.drag.is_none()
        && let Some(h) = resp.hover_pos()
    {
        if near_handle(h).is_some() || near_point(h).is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        } else if near_curve(h) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
    }
    if resp.drag_started()
        && let Some(pt) = ui.input(|i| i.pointer.press_origin())
    {
        ed.drag = match (sel, near_handle(pt), near_point(pt)) {
            (Some(i), Some(h), _) => Some(Drag::Handle { i, h, lo, hi }),
            (_, _, Some(i)) if i > 0 && i + 1 < pts.len() => Some(Drag::Point { i, lo, hi }),
            (_, _, Some(i)) => {
                ed.selected = Some(i);
                None
            }
            _ => None,
        };
        if let Some(Drag::Point { i, .. }) = ed.drag {
            ed.selected = Some(i);
        }
        if ed.drag.is_some() {
            app.playing = false;
            app.s.begin_gesture(tr_noop!("Ease point"));
        }
    }
    if resp.dragged()
        && let (Some(d), Some(pt)) = (ed.drag, resp.interact_pointer_pos())
    {
        let (x, y) = (v.time(pt.x), v.value(pt.y));
        match d {
            Drag::Point { i, .. } => {
                app.run("ease.move_point", ed.with(json!({"index": i, "x": x, "y": y})));
            }
            Drag::Handle { i, h, .. } => {
                app.run("ease.move_handle", ed.with(json!({"index": i, "handle": if h == Handle::In { "in" } else { "out" }, "x": x, "y": y})));
            }
        }
    }
    if resp.drag_stopped() && ed.drag.take().is_some() {
        app.s.end_gesture();
    }
    if resp.clicked()
        && let Some(pt) = resp.interact_pointer_pos()
    {
        match near_point(pt) {
            Some(i) => ed.selected = Some(i),
            None if near_curve(pt) => {
                if let Some(r) = app.run("ease.add_point", ed.with(json!({"t": v.time(pt.x)}))) {
                    ed.selected = r["index"].as_u64().map(|i| i as usize);
                }
            }
            None => ed.selected = None,
        }
    }
    if resp.double_clicked()
        && let Some(i) = resp.interact_pointer_pos().and_then(near_point).filter(|i| *i > 0 && *i + 1 < pts.len())
        && let Some(q) = pts.get(i)
    {
        app.run("ease.convert_point", ed.with(json!({"index": i, "smooth": !q.smooth})));
    }
    if resp.secondary_clicked()
        && let Some(i) = resp.interact_pointer_pos().and_then(near_point).filter(|i| *i > 0 && *i + 1 < pts.len())
    {
        app.run("ease.delete_point", ed.with(json!({"index": i})));
        ed.selected = None;
    }
    resp.on_hover_text_at_pointer(tr!("Click the curve to add a point. Drag a point or handle; double-click a point for smooth or corner, right-click to delete it."));
}

/// The graph's keys, while it has the keyboard (taken before the app's shortcuts, so Delete
/// removes a point here rather than artwork on the stage).
pub fn consume_keys(app: &mut App, ctx: &egui::Context) {
    let Some(ed) = app.ease.as_ref() else { return };
    let Some(id) = ed.graph.filter(|id| ctx.memory(|m| m.has_focus(*id))) else { return };
    let _ = id;
    let Some(curve) = ease_now(app, ed).map(|e| e.to_custom()) else { return };
    let n = curve.points().len();
    let (prev, next, del, smooth, add, mv) = ctx.input_mut(|i| {
        let step = if i.modifiers.shift { 0.1 } else { 0.01 };
        let mut mv = (0.0f32, 0.0f32);
        for (k, d) in [(Key::ArrowLeft, (-step, 0.0)), (Key::ArrowRight, (step, 0.0)), (Key::ArrowUp, (0.0, step)), (Key::ArrowDown, (0.0, -step))] {
            if i.consume_key(Modifiers::NONE, k) || i.consume_key(Modifiers::SHIFT, k) {
                mv = (mv.0 + d.0, mv.1 + d.1);
            }
        }
        let prev = i.consume_key(Modifiers::NONE, Key::OpenBracket);
        let next = i.consume_key(Modifiers::NONE, Key::CloseBracket);
        let del = i.consume_key(Modifiers::NONE, Key::Delete) || i.consume_key(Modifiers::NONE, Key::Backspace);
        let smooth = i.consume_key(Modifiers::NONE, Key::S);
        let add = i.consume_key(Modifiers::NONE, Key::Plus) || i.consume_key(Modifiers::NONE, Key::Equals) || i.consume_key(Modifiers::SHIFT, Key::Equals) || i.consume_key(Modifiers::NONE, Key::Insert);
        (prev, next, del, smooth, add, mv)
    });
    let Some(ed) = app.ease.as_mut() else { return };
    let sel = ed.selected.filter(|i| *i < n);
    if prev || next {
        ed.selected = Some(match (sel, next) {
            (None, true) => 0,
            (None, false) => n.saturating_sub(1),
            (Some(i), true) => (i + 1).min(n.saturating_sub(1)),
            (Some(i), false) => i.saturating_sub(1),
        });
        return;
    }
    let Some(i) = sel else { return };
    let interior = i > 0 && i + 1 < n;
    let q = curve.points().get(i).copied();
    let p = ed.params();
    let with = |extra: Value| {
        let mut p = p.clone();
        if let (Some(o), Value::Object(e)) = (p.as_object_mut(), extra) {
            o.extend(e);
        }
        p
    };
    if del && interior {
        ed.selected = None;
        app.run("ease.delete_point", with(json!({"index": i})));
    } else if smooth && interior && let Some(q) = q {
        app.run("ease.convert_point", with(json!({"index": i, "smooth": !q.smooth})));
    } else if add && let (Some(a), Some(b)) = (q, curve.points().get(i + 1)) {
        if let Some(r) = app.run("ease.add_point", with(json!({"t": 0.5 * (a.at.x + b.at.x)})))
            && let Some(ed) = app.ease.as_mut()
        {
            ed.selected = r["index"].as_u64().map(|i| i as usize);
        }
    } else if (mv.0 != 0.0 || mv.1 != 0.0) && interior && let Some(q) = q {
        app.run("ease.move_point", with(json!({"index": i, "x": q.at.x + mv.0, "y": q.at.y + mv.1})));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anicraft_engine::Session;
    use anicraft_engine::doc::sample;

    fn app(ctx: &egui::Context) -> App {
        App::with_context(ctx, Session::new(sample::demo()), Some(std::path::PathBuf::from("unused.png")))
    }

    fn frame(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) {
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 940.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            consume_keys(app, ui.ctx());
            show(app, ui.ctx());
        });
        out.textures_delta.clear();
    }

    fn key(k: Key) -> egui::Event {
        egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
    }

    fn points(app: &App) -> usize {
        app.ease.as_ref().and_then(|ed| ease_now(app, ed)).map_or(0, |e| e.to_custom().points().len())
    }

    #[test]
    fn the_window_draws_and_the_graph_edits_from_the_keyboard() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        app.open_panel(Some("ease"));
        assert!(app.ease.is_some(), "the demo opens the window on the ball's tween");
        for _ in 0..3 {
            frame(&ctx, &mut app, vec![]);
        }
        assert!(app.ease.is_some(), "it stays open while drawn");
        let id = app.ease.as_ref().and_then(|ed| ed.graph).expect("the graph has an id once drawn");
        ctx.memory_mut(|m| m.request_focus(id));
        frame(&ctx, &mut app, vec![]);
        let n = points(&app);
        assert!(n >= 3);
        // ] and [ walk the points; + adds one after the picked point; S toggles smooth; arrows
        // move it; Delete removes it again.
        frame(&ctx, &mut app, vec![key(Key::CloseBracket)]);
        frame(&ctx, &mut app, vec![key(Key::OpenBracket)]);
        let picked = app.ease.as_ref().and_then(|ed| ed.selected);
        assert!(picked.is_some());
        frame(&ctx, &mut app, vec![key(Key::Insert)]);
        assert_eq!(points(&app), n + 1, "a point added");
        frame(&ctx, &mut app, vec![key(Key::S)]);
        let at = |app: &App| {
            let ed = app.ease.as_ref()?;
            let q = *ease_now(app, ed)?.to_custom().points().get(ed.selected?)?;
            Some((q.at.x, q.at.y))
        };
        let p0 = at(&app).expect("a picked point");
        frame(&ctx, &mut app, vec![key(Key::ArrowUp)]);
        frame(&ctx, &mut app, vec![key(Key::ArrowLeft)]);
        let p1 = at(&app).expect("still picked");
        assert!(p1.1 > p0.1 && p1.0 < p0.0, "both arrows moved the point (focus stays on the graph): {p0:?} -> {p1:?}");
        frame(&ctx, &mut app, vec![key(Key::Delete)]);
        assert_eq!(points(&app), n, "the added point deleted");
        // Escape closes the window.
        frame(&ctx, &mut app, vec![key(Key::Escape)]);
        assert!(app.ease.is_none());
    }

    #[test]
    fn the_window_draws_a_group_a_saved_ease_and_closes_when_its_tween_goes() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        app.open_panel(Some("easepreset"));
        assert!(app.ease.as_ref().is_some_and(|ed| ed.group.is_some()), "the Position group shown");
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        // The saved ease by name.
        app.ease = Some(EaseEditor::new(EaseTarget::Saved("Overshoot".into())));
        assert!(heading(&app, app.ease.as_ref().unwrap()).contains("Overshoot"));
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        assert!(app.ease.is_some());
        // A saved ease that doesn't exist: the window closes.
        app.ease = Some(EaseEditor::new(EaseTarget::Saved("No such ease".into())));
        frame(&ctx, &mut app, vec![]);
        assert!(app.ease.is_none());
        // No tween at the playhead: nothing opens and the status says why.
        app.run("layer.new", json!({}));
        open_here(&mut app);
        assert!(app.ease.is_none());
    }
}
