//! The stage's elements for screen readers and the keyboard (ADR 0064).
//!
//! While the stage has keyboard focus, its canvas node gets one child: a multi-select list box,
//! "Elements on frame 12", with one option per element on the frame (layers top to bottom, each
//! front to back). An option says what the element is ("Rectangle", "Text: Hello", "hero ·
//! Movie clip: Ball"), its layer, place and size, and whether it is selected; its bounds are
//! where it is on screen, so a reader's highlight lands on the artwork.
//!
//! Keys: Tab on the stage goes to the first selected element (or the first one); then Tab,
//! Shift+Tab and the arrow keys move between elements (wrapping), Enter or Space selects the
//! element alone, Shift+Enter or Shift+Space adds it to the selection or takes it out, and Esc
//! goes back to the stage itself (where the arrows nudge again). Selection goes through
//! `element.select`, so it is the same command a click, the CLI and MCP use.
//!
//! Nothing here exists while the stage doesn't have focus, so the rest of the interface keeps
//! its tab order and the tree stays small. A frame with more than [`MAX_NODES`] elements lists
//! the first ones and a summary of the rest.

use crate::app::App;
use crate::theme::T;
use anicraft_engine::doc::{Document, Element, ElementId, ElementKind, LayerId, LayerKind, Paint, SymbolKind};
use anicraft_engine::geom::{Path, PathCmd, Rect as GRect};
use egui::accesskit::{Action, Role};
use egui::{Id, Key, Modifiers, Pos2, Rect, Response, Sense, Stroke, StrokeKind, UiBuilder, Vec2};
use serde_json::json;
use std::collections::HashMap;

/// Most elements exposed one by one; the rest are summed up in one node.
pub const MAX_NODES: usize = 2_000;

/// One element as a screen reader hears it.
#[derive(Clone, Debug)]
pub struct Item {
    pub layer: LayerId,
    pub element: ElementId,
    /// What it is: "Rectangle", "Text: Hello", "hero · Movie clip: Ball".
    pub name: String,
    /// Where: "Layer Hills · x 120, y 80 · 200 × 40 px".
    pub detail: String,
    /// Bounds in the space the stage draws in (the open symbol's, while one is open).
    pub bounds: GRect,
    pub selected: bool,
    /// On a locked layer: listed, but it can't be selected.
    pub locked: bool,
}

/// The stage's keyboard state: the element nodes made last frame, to tell which one has focus.
#[derive(Default)]
pub struct StageNav {
    ids: Vec<(Id, LayerId, ElementId)>,
}

impl StageNav {
    /// The element with keyboard focus, if one has it.
    pub fn focused(&self, ctx: &egui::Context) -> Option<(LayerId, ElementId)> {
        let f = ctx.memory(|m| m.focused())?;
        self.ids.iter().find(|(id, _, _)| *id == f).map(|(_, l, e)| (*l, *e))
    }
}

pub fn node_id(layer: LayerId, element: ElementId) -> Id {
    Id::new(("stage_element", layer.0, element.0))
}

/// What a path looks like it was drawn with (English, a key): the Rectangle, Oval and PolyStar
/// tools' outlines and the Line tool's segment; anything else is a shape.
pub fn shape_kind(p: &Path) -> &'static str {
    let (mut moves, mut lines, mut curves, mut closes) = (0usize, 0usize, 0usize, 0usize);
    let mut pts: Vec<anicraft_engine::geom::Vec2> = Vec::new();
    for c in p.cmds.iter().take(64) {
        match c {
            PathCmd::MoveTo(v) => {
                moves += 1;
                pts.push(*v);
            }
            PathCmd::LineTo(v) => {
                lines += 1;
                pts.push(*v);
            }
            PathCmd::QuadTo(..) | PathCmd::CubicTo(..) => curves += 1,
            PathCmd::Close => closes += 1,
        }
    }
    if p.cmds.len() > 64 || moves != 1 {
        return tr_noop!("Shape");
    }
    let axis_aligned = || {
        let mut ring = pts.clone();
        if let Some(first) = pts.first() {
            ring.push(*first);
        }
        ring.windows(2).all(|w| matches!(w, [a, b] if (a.x - b.x).abs() < 1e-3 || (a.y - b.y).abs() < 1e-3))
    };
    match (lines, curves, closes) {
        (1, 0, 0) => tr_noop!("Line"),
        (3 | 4, 0, 1) if axis_aligned() => tr_noop!("Rectangle"),
        // A rounded rectangle: four sides and four corners.
        (4, 4, 1) => tr_noop!("Rectangle"),
        (0, 4, 1) => tr_noop!("Oval"),
        (n, 0, 1) if n >= 2 => tr_noop!("Polygon"),
        _ => tr_noop!("Shape"),
    }
}

fn symbol_kind(k: SymbolKind) -> &'static str {
    match k {
        SymbolKind::Graphic => tr_noop!("Graphic"),
        SymbolKind::MovieClip => tr_noop!("Movie clip"),
        SymbolKind::Button => tr_noop!("Button"),
    }
}

/// The first `max` characters of `s` on one line, with "…" when cut.
fn excerpt(s: &str, max: usize) -> String {
    let one: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= max {
        return one;
    }
    let mut cut: String = one.chars().take(max).collect();
    cut.push('…');
    cut
}

/// What an element is, in the interface language; its instance name (document text) first.
pub fn element_name(doc: &Document, e: &Element) -> String {
    let what = match &e.kind {
        ElementKind::Shape(sh) if matches!(sh.fill, Some(Paint::Bitmap { .. })) => tr!("Bitmap").to_string(),
        ElementKind::Shape(sh) => crate::i18n::t(shape_kind(&sh.path)).to_string(),
        ElementKind::Symbol { symbol, .. } => match doc.symbol(*symbol) {
            Some(sym) => tr!("{kind}: {symbol}", kind = crate::i18n::t(symbol_kind(sym.kind)), symbol = sym.name),
            None => tr!("Symbol instance").to_string(),
        },
        ElementKind::Text(tb) if tb.text.trim().is_empty() => tr!("Empty text").to_string(),
        ElementKind::Text(tb) => tr!("Text: {text}", text = excerpt(&tb.text, 80)),
        ElementKind::Camera => tr!("Camera").to_string(),
    };
    let name = excerpt(&e.name, 80);
    if name.is_empty() { what } else { tr!("{name} · {what}", name = name, what = what) }
}

/// The current frame's elements, as the list shows them, and how many there are in all.
pub fn items(app: &App) -> (Vec<Item>, usize) {
    let s = &app.s;
    let tl = s.timeline();
    // Bounds of every element in one pass over what the stage draws.
    let mut bounds: HashMap<(LayerId, ElementId), GRect> = HashMap::new();
    for item in s.display_list().items.iter() {
        let pad = item.stroke.as_ref().map_or(0.0, anicraft_engine::doc::Stroke::reach);
        let r = item.path.bounds().inflate(pad).transformed(&item.transform);
        let e = bounds.entry((item.layer, item.element)).or_insert(GRect::EMPTY);
        *e = e.union(&r);
    }
    let mut out = Vec::new();
    let mut total = 0usize;
    for (i, layer) in tl.layers.iter().enumerate() {
        // Only what is on the stage: shown layers with artwork, not the camera.
        if !layer.visible || tl.hidden_by_folder(i) || matches!(layer.kind, LayerKind::Folder | LayerKind::Camera) {
            continue;
        }
        let locked = !s.selectable(layer.id);
        let elements = s.elements_on(layer.id);
        for e in elements.iter().rev().filter(|e| !matches!(e.kind, ElementKind::Camera)) {
            total += 1;
            if out.len() >= MAX_NODES {
                continue;
            }
            let b = bounds.get(&(layer.id, e.id)).copied().unwrap_or_else(|| e.bounds(&s.doc));
            let detail = if b.is_empty() {
                tr!("Layer {layer}", layer = layer.name)
            } else {
                tr!("Layer {layer} · x {x}, y {y} · {w} × {h} px", layer = layer.name, x = format!("{:.0}", b.min.x), y = format!("{:.0}", b.min.y), w = format!("{:.0}", b.width()), h = format!("{:.0}", b.height()))
            };
            let detail = if locked { tr!("{detail} · locked", detail = detail) } else { detail };
            out.push(Item { layer: layer.id, element: e.id, name: element_name(&s.doc, e), detail, bounds: b, selected: s.selection.contains(layer.id, e.id), locked });
        }
    }
    (out, total)
}

fn ak_rect(r: Rect) -> egui::accesskit::Rect {
    egui::accesskit::Rect { x0: r.min.x.into(), y0: r.min.y.into(), x1: r.max.x.into(), y1: r.max.y.into() }
}

/// What a key press on a focused element does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Next,
    Prev,
    Select,
    Toggle,
    Leave,
}

fn element_keys(i: &mut egui::InputState) -> Option<Step> {
    for (m, k, step) in [
        (Modifiers::SHIFT, Key::Tab, Step::Prev),
        (Modifiers::NONE, Key::Tab, Step::Next),
        (Modifiers::NONE, Key::ArrowRight, Step::Next),
        (Modifiers::NONE, Key::ArrowDown, Step::Next),
        (Modifiers::NONE, Key::ArrowLeft, Step::Prev),
        (Modifiers::NONE, Key::ArrowUp, Step::Prev),
        (Modifiers::SHIFT, Key::Enter, Step::Toggle),
        (Modifiers::SHIFT, Key::Space, Step::Toggle),
        (Modifiers::NONE, Key::Enter, Step::Select),
        (Modifiers::NONE, Key::Space, Step::Select),
        (Modifiers::NONE, Key::Escape, Step::Leave),
    ] {
        if i.consume_key(m, k) {
            return Some(step);
        }
    }
    None
}

/// Selects `it` alone (`toggle`: adds it to the selection or takes it out) and says so.
fn select(app: &mut App, it: &Item, toggle: bool) {
    let params = if toggle { json!({"ids": [it.element.0], "layer": it.layer.0, "toggle": true}) } else { json!({"ids": [it.element.0], "layer": it.layer.0}) };
    if app.run("element.select", params).is_some() {
        let n = app.s.selection.len();
        let now = app.s.selection.contains(it.layer, it.element);
        let what = if now { tr!("Selected {name}", name = it.name) } else { tr!("Deselected {name}", name = it.name) };
        app.status_ok(trn!("{what} · {n} element selected", "{what} · {n} elements selected", n, what = what));
    }
}

/// The element list under the stage's node, and its keys. `to_screen` maps the stage's drawing
/// space to the screen. Call after the stage has drawn, while `stage` is its canvas response.
pub fn show(app: &mut App, ui: &mut egui::Ui, stage: &Response, to_screen: impl Fn(anicraft_engine::geom::Vec2) -> Pos2) {
    let ctx = ui.ctx().clone();
    let focused = ctx.memory(|m| m.focused());
    let on_canvas = focused == Some(stage.id);
    let on_element = app.stage_nav.focused(&ctx);
    if !on_canvas && on_element.is_none() {
        app.stage_nav.ids.clear();
        return;
    }
    let (mut list, mut total) = items(app);
    let screen = |b: GRect| -> Rect {
        if b.is_empty() {
            return Rect::from_center_size(stage.rect.center(), Vec2::ZERO);
        }
        Rect::from_two_pos(to_screen(b.min), to_screen(b.max))
    };

    // ---- keys -----------------------------------------------------------------------------
    let mut focus_to: Option<Id> = None;
    if on_canvas && !list.is_empty() {
        // Tab goes into the list: to the selection, or the first element.
        let (fwd, back) = ctx.input_mut(|i| (i.consume_key(Modifiers::NONE, Key::Tab), i.consume_key(Modifiers::SHIFT, Key::Tab)));
        let pick = if fwd {
            list.iter().position(|it| it.selected).or(Some(0))
        } else if back {
            list.iter().rposition(|it| it.selected).or(Some(list.len() - 1))
        } else {
            None
        };
        if let Some(it) = pick.and_then(|i| list.get(i)) {
            focus_to = Some(node_id(it.layer, it.element));
        }
    }
    if let Some((layer, el)) = on_element {
        let at = list.iter().position(|it| it.layer == layer && it.element == el);
        match (at, ctx.input_mut(element_keys)) {
            // Gone from the frame (deleted, or the playhead moved): back to the stage.
            (None, _) => focus_to = Some(stage.id),
            (Some(i), Some(Step::Next)) => focus_to = list.get((i + 1) % list.len()).map(|it| node_id(it.layer, it.element)),
            (Some(i), Some(Step::Prev)) => focus_to = list.get((i + list.len() - 1) % list.len()).map(|it| node_id(it.layer, it.element)),
            (Some(i), Some(step @ (Step::Select | Step::Toggle))) => {
                if let Some(it) = list.get(i).cloned() {
                    select(app, &it, step == Step::Toggle);
                    (list, total) = items(app);
                }
            }
            (Some(_), Some(Step::Leave)) => focus_to = Some(stage.id),
            (Some(_), None) => {}
        }
    }
    // A screen reader's "click" on an option selects it, as Enter does.
    let clicked = list.iter().find(|it| ctx.input(|i| i.has_accesskit_action_request(node_id(it.layer, it.element), Action::Click))).cloned();
    if let Some(it) = clicked {
        select(app, &it, false);
        (list, total) = items(app);
    }

    // ---- nodes ----------------------------------------------------------------------------
    let child = ui.new_child(UiBuilder::new().id_salt("stage_elements").accessibility_parent(stage.id).max_rect(stage.rect).sense(Sense::hover()));
    let frame = app.s.frame + 1;
    ctx.accesskit_node_builder(child.unique_id(), |n| {
        n.set_role(Role::ListBox);
        n.set_label(trn!("{n} element on frame {frame}", "{n} elements on frame {frame}", total, frame = frame));
        n.set_multiselectable();
        n.set_bounds(ak_rect(stage.rect));
    });
    app.stage_nav.ids.clear();
    let count = list.len();
    let filter = egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true };
    for (i, it) in list.iter().enumerate() {
        let id = node_id(it.layer, it.element);
        let r = screen(it.bounds);
        // No pointer area (the canvas takes the clicks): a focus stop with its bounds.
        let resp = child.interact(Rect::from_center_size(r.center(), Vec2::ZERO), id, Sense::FOCUSABLE);
        ctx.accesskit_node_builder(id, |n| {
            n.set_role(Role::ListBoxOption);
            n.set_label(it.name.as_str());
            n.set_description(it.detail.as_str());
            n.set_selected(it.selected);
            n.set_bounds(ak_rect(r));
            n.set_position_in_set(i + 1);
            n.set_size_of_set(total);
            n.add_action(Action::Click);
            if it.locked {
                n.set_disabled();
            }
        });
        if resp.has_focus() {
            ctx.memory_mut(|m| m.set_focus_lock_filter(id, filter));
            // The keyboard focus, drawn on the artwork.
            ui.painter().rect_stroke(r.expand(3.0), T.rc(), Stroke::new(2.0, T.accent), StrokeKind::Outside);
        }
        app.stage_nav.ids.push((id, it.layer, it.element));
    }
    if total > count {
        let id = child.unique_id().with("more");
        child.interact(Rect::from_center_size(stage.rect.center(), Vec2::ZERO), id, Sense::hover());
        ctx.accesskit_node_builder(id, |n| {
            n.set_role(Role::Label);
            n.set_label(trn!(
                "{n} more element is not listed: pick it on the stage, or use Select All or Find",
                "{n} more elements are not listed: pick them on the stage, or use Select All or Find",
                total - count
            ));
        });
    }
    if on_canvas {
        // Tab goes into the list (when there is one); the arrows nudge, as before.
        let tab = !list.is_empty();
        ctx.memory_mut(|m| m.set_focus_lock_filter(stage.id, egui::EventFilter { tab, horizontal_arrows: true, vertical_arrows: true, ..Default::default() }));
    }
    if let Some(id) = focus_to {
        ctx.memory_mut(|m| m.request_focus(id));
        if id != stage.id
            && let Some(it) = list.iter().find(|it| node_id(it.layer, it.element) == id)
        {
            // A reader says the newly focused option itself; the status bar says it for everyone.
            let state = if it.selected { tr!("selected") } else { tr!("not selected") };
            app.status_ok(tr!("{name} · {state} · Enter selects, Esc goes back to the stage", name = it.name, state = state));
        }
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anicraft_engine::geom::Vec2 as V;

    #[test]
    fn shapes_are_named_by_the_tool_that_draws_them() {
        assert_eq!(shape_kind(&Path::rect(0.0, 0.0, 10.0, 5.0)), "Rectangle");
        assert_eq!(shape_kind(&Path::rounded_rect(0.0, 0.0, 10.0, 5.0, 2.0)), "Rectangle");
        assert_eq!(shape_kind(&Path::ellipse(0.0, 0.0, 4.0, 3.0)), "Oval");
        assert_eq!(shape_kind(&Path::star(0.0, 0.0, 10.0, 4.0, 5)), "Polygon");
        assert_eq!(shape_kind(&Path::line(V::new(0.0, 0.0), V::new(3.0, 4.0))), "Line");
        // A slanted quadrilateral is a polygon, not a rectangle.
        let mut p = Path::new();
        p.move_to(V::new(0.0, 0.0)).line_to(V::new(10.0, 1.0)).line_to(V::new(10.0, 6.0)).line_to(V::new(0.0, 5.0)).close();
        assert_eq!(shape_kind(&p), "Polygon");
        assert_eq!(shape_kind(&Path::new()), "Shape");
    }

    #[test]
    fn long_text_is_cut_to_one_line() {
        assert_eq!(excerpt("Hello\n  world", 80), "Hello world");
        assert_eq!(excerpt(&"a".repeat(100), 5), "aaaaa…");
    }
}
