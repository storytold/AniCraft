//! Motion Editor (ADR 0016): the property curves of the motion tween at the playhead.
//!
//! It shares the timeline's dock (a tab beside it, as in Animate) and follows the active layer
//! and the playhead. Left: the properties, with their value at the playhead (drag to key it)
//! and a diamond that adds or removes a key there. Right: the selected property's curve over
//! the span. Drag a key up or down to change its value, or sideways to move it in time (ADR
//! 0083: it snaps to frames and stops at its neighbours; Shift-click picks keys of several
//! properties, and the lane under the ruler picks every property's keys on a frame), drag the
//! two handles of the selected segment to shape its ease (a cubic Bézier), click to move the
//! playhead, double-click to add a key. With the curve focused, Left and Right move the picked
//! keys a frame (Shift: ten). Every drag is one undo step (a session gesture); every change is
//! a command.
//!
//! On X and Y, the keys of frames pinned to a motion path point (ADR 0049) are drawn in the
//! motion colour: they are the point's place and its timing, so dragging one moves the point on
//! the stage, and removing one unpins it.
//!
//! Below the colour effect, each filter of the object (ADR 0092) lists its numbers under the
//! filter's name (Blur X, Blur Y, Strength…), curves and keys like the other properties.

use crate::a11y::{self, Named};
use crate::app::App;
use crate::theme::{SQUARE, T, TARGET, Type, color, font, text, with_type};
use anicraft_engine::doc::{ColorStyle, Easing, Element, ElementKind, MotionTween, Pin, Property, Props, Track, filter_properties, own_value};
use egui::{Align2, Color32, Pos2, Rect, Sense, Stroke, WidgetType, pos2, vec2};
use serde_json::json;

/// Property list width (the timeline's layer column).
const LIST_W: f32 = 252.0;
const ROW_H: f32 = TARGET;
const RULER_H: f32 = 24.0;
/// The lane under the ruler with every property's keys.
const LANE_H: f32 = 16.0;
const FOOTER_H: f32 = 38.0;
/// Room inside the graph around the curve, in points.
const PAD: f32 = 18.0;
/// Pointer distance that grabs a key or a handle.
const GRAB: f32 = 8.0;
/// How far a key drag goes before it picks its direction (time or value).
const AXIS_SLOP: f32 = 4.0;

/// Which way a key drag goes, decided by its first few points of travel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Undecided,
    /// Sideways: the picked keys move in time.
    Time,
    /// Up or down: the grabbed key's value.
    Value,
}

/// An in-progress drag in the graph. The value range is frozen so the curve doesn't rescale
/// under the pointer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MeDrag {
    /// A key grabbed at `frame` (its offset in the span when the drag began): sideways it
    /// moves the picked keys in time (`by` frames so far), up or down it changes this key's
    /// value. `lane`: grabbed on the all-properties lane, which only moves keys in time.
    Key { frame: u32, lo: f32, hi: f32, axis: Axis, by: i64, lane: bool },
    /// Handle 1 or 2 of the segment starting at the key at `frame`.
    Handle { frame: u32, which: u8, lo: f32, hi: f32 },
    /// Scrubbing the playhead.
    Scrub,
}

/// The motion tween at the playhead on the active layer: its first frame, length and data, the
/// colour effect styles it shows (ADR 0056), and its object.
fn current(app: &App) -> Option<(u32, u32, MotionTween, Props, Vec<ColorStyle>, Element)> {
    let k = app.s.active_layer()?.keyframe_at(app.s.frame)?;
    let m = k.motion.clone()?;
    let target = m.target_index(&k.elements).and_then(|i| k.elements.get(i))?;
    let styles = m.colour_styles(&target.color);
    Some((k.start, k.duration, m, Props::of(target), styles, (**target).clone()))
}

/// A row of the property list: a property, or the name of a filter above its numbers.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Row {
    Prop(Property),
    /// A filter's place, name and whether it is on.
    Filter(u8, &'static str, bool),
}

/// The rows: [`listed`]'s properties, then each filter of the object with its numbers.
fn rows_for(used: &[ColorStyle], added: &[ColorStyle], target: &Element) -> Vec<Row> {
    let mut out: Vec<Row> = listed(used, added).into_iter().map(Row::Prop).collect();
    let ElementKind::Symbol { filters, .. } = &target.kind else { return out };
    let mut props = filter_properties(target).into_iter().peekable();
    for (slot, e) in filters.iter().enumerate() {
        out.push(Row::Filter(slot as u8, e.filter.label(), e.enabled));
        while let Some(p) = props.next_if(|p| p.filter().is_some_and(|f| usize::from(f.slot) == slot)) {
            out.push(Row::Prop(p));
        }
    }
    out
}

/// The properties the list shows: position and transform, and the colour effect styles the
/// tween uses or the artist added (Alpha when there are none).
fn listed(used: &[ColorStyle], added: &[ColorStyle]) -> Vec<Property> {
    let mut shown: Vec<ColorStyle> = used.iter().chain(added).copied().collect();
    if shown.is_empty() {
        shown.push(ColorStyle::Alpha);
    }
    Property::ALL.into_iter().filter(|p| p.style().is_none_or(|s| shown.contains(&s))).collect()
}

/// A readable step for grid lines across `span` (1, 2 or 5 × 10ⁿ).
fn nice_step(span: f32) -> f32 {
    let raw = (span / 4.0).max(1e-3);
    let p = 10f32.powf(raw.log10().floor());
    [1.0, 2.0, 5.0, 10.0].into_iter().map(|m| m * p).find(|s| *s >= raw).unwrap_or(10.0 * p)
}

fn fmt(p: Property, v: f32) -> String {
    match p {
        Property::X | Property::Y => format!("{v:.1} {}", p.unit()),
        _ => format!("{v:.1}{}", p.unit()),
    }
}

/// The value range the graph shows for `track` (or the flat base value): every sampled value,
/// every key and every handle, with a margin.
fn value_range(track: Option<&Track>, base: f32, frames: u32, p: Property) -> (f32, f32) {
    let (mut lo, mut hi) = (base, base);
    if let Some(t) = track {
        for f in 0..frames.max(1) {
            if let Some(v) = t.value_at(f as f32) {
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
        for (i, k) in t.keys.iter().enumerate() {
            lo = lo.min(k.value);
            hi = hi.max(k.value);
            if let Some(n) = t.keys.get(i + 1)
                && let Some((_, y1, _, y2)) = k.ease.as_bezier()
            {
                for y in [y1, y2] {
                    let v = k.value + (n.value - k.value) * y;
                    lo = lo.min(v);
                    hi = hi.max(v);
                }
            }
        }
    }
    let min_span = match p {
        Property::X | Property::Y => 20.0,
        _ => 10.0,
    };
    if hi - lo < min_span {
        let c = (hi + lo) * 0.5;
        lo = c - min_span * 0.5;
        hi = c + min_span * 0.5;
    }
    let m = (hi - lo) * 0.12;
    (lo - m, hi + m)
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let body = ui.available_rect_before_wrap();
    ui.allocate_rect(body, Sense::hover());
    let painter = ui.painter_at(body);
    painter.rect_filled(body, 0.0, T.panel);
    painter.line_segment([body.left_top(), body.right_top()], Stroke::new(1.0, T.border));

    let Some((start, frames, m, base, used, target)) = current(app) else {
        empty_state(app, ui, body);
        return;
    };
    let offset = app.s.frame.saturating_sub(start);
    // The picked keys belong to this span: another span or layer starts afresh, and keys that
    // went (removed, or undone) leave the pick.
    let here = (app.s.layer.0, start);
    if app.me_sel_at != here {
        app.me_sel.clear();
        app.me_sel_at = here;
    }
    app.me_sel.retain(|(p, f)| m.track(*p).is_some_and(|t| t.key_at(*f).is_some()));
    let list = Rect::from_min_max(body.left_top(), pos2(body.left() + LIST_W, body.bottom() - FOOTER_H));
    let graph_full = Rect::from_min_max(pos2(list.right(), body.top()), pos2(body.right(), body.bottom() - FOOTER_H));
    let ruler = Rect::from_min_max(graph_full.left_top(), pos2(graph_full.right(), graph_full.top() + RULER_H));
    let lane = Rect::from_min_max(pos2(graph_full.left(), ruler.bottom()), pos2(graph_full.right(), ruler.bottom() + LANE_H));
    let graph = Rect::from_min_max(pos2(graph_full.left(), lane.bottom()), graph_full.right_bottom());

    // ---- property list ---------------------------------------------------------------------
    painter.rect_filled(Rect::from_min_max(list.left_top(), pos2(list.right(), list.top() + RULER_H)), 0.0, T.panel_alt);
    painter.text(pos2(list.left() + 10.0, list.top() + RULER_H * 0.5), Align2::LEFT_CENTER, tr!("Property"), font(Type::Heading), color(Type::Heading));
    painter.text(pos2(list.right() - 44.0, list.top() + RULER_H * 0.5), Align2::RIGHT_CENTER, tr!("Frame {frame}", frame = app.s.frame + 1), font(Type::DataSmall), color(Type::DataSmall));
    let now = m.props_at(&base, offset as f32);
    // The object at the playhead, for its filters' numbers (ADR 0092).
    let at = m.apply(&target, offset as f32);
    // A property's own value (its base) and its value at the playhead.
    let own = |p: Property| if p.is_filter() { own_value(&target, p).unwrap_or(0.0) } else { base.get(p) };
    let now_of = |p: Property| if p.is_filter() { own_value(&at, p).unwrap_or(0.0) } else { now.get(p) };
    // Frames pinned to path points: their X and Y keys are the points' places (ADR 0049).
    let pins = m.pinned_anchors(&base);
    let pin_at = |p: Property, f: u32| pins.iter().find(|q| q.frame == f).copied().filter(|_| matches!(p, Property::X | Property::Y));
    let rows_top = list.top() + RULER_H;
    // Rows that don't fit scroll with the wheel over the list.
    let fit = (((list.bottom() - rows_top) / ROW_H).floor() as usize).max(1);
    let list_rows = rows_for(&used, &app.me_styles, &target);
    let rows: Vec<Property> = list_rows.iter().filter_map(|r| if let Row::Prop(p) = r { Some(*p) } else { None }).collect();
    if !rows.contains(&app.me_prop) {
        app.me_prop = Property::X;
    }
    let n = list_rows.len();
    if ui.rect_contains_pointer(list) {
        let d = ui.input(|i| i.smooth_scroll_delta.y);
        app.me_vscroll = (app.me_vscroll - d / ROW_H).clamp(0.0, n.saturating_sub(fit) as f32);
    }
    app.me_vscroll = app.me_vscroll.min(n.saturating_sub(fit) as f32);
    let first = app.me_vscroll.floor() as usize;
    let mut last_group = "";
    for (r, item) in list_rows.iter().copied().enumerate().skip(first).take(fit) {
        let y = rows_top + (r - first) as f32 * ROW_H;
        let row = Rect::from_min_size(pos2(list.left(), y), vec2(LIST_W, ROW_H));
        let p = match item {
            Row::Prop(p) => p,
            Row::Filter(_, name, on) => {
                // A filter's name above its numbers, set apart by a rule (faint when it is off).
                painter.rect_filled(row, 0.0, T.panel_alt);
                painter.line_segment([row.left_top(), row.right_top()], Stroke::new(1.0, T.border));
                let name = crate::i18n::t(name);
                let label = if on { name.to_string() } else { tr!("{filter} (off)", filter = name) };
                painter.text(pos2(row.left() + 10.0, row.center().y), Align2::LEFT_CENTER, label, font(Type::Label), if on { color(Type::Label) } else { T.text_faint });
                painter.line_segment([row.left_bottom(), row.right_bottom()], Stroke::new(1.0, T.separator));
                last_group = "Filters"; // i18n-exempt: a group key (Property::group), not shown
                continue;
            }
        };
        let selected = app.me_prop == p;
        painter.rect_filled(row, 0.0, if selected { T.accent_dim } else if r % 2 == 0 { T.panel } else { T.panel_alt });
        let track = m.track(p);
        let label = crate::i18n::t(p.label());
        // Groups (Position, Transform, Colour effect) are set apart by a rule.
        if p.group() != last_group {
            if !last_group.is_empty() {
                painter.line_segment([row.left_top(), row.right_top()], Stroke::new(1.0, T.border));
            }
            last_group = p.group();
        }
        painter.text(pos2(row.left() + 12.0, row.center().y), Align2::LEFT_CENTER, label, font(Type::Body), if track.is_some() { T.text } else { T.text_dim });
        if track.is_some() {
            painter.text(pos2(row.left() + 84.0, row.center().y), Align2::LEFT_CENTER, tr!("keyed"), font(Type::Caption), T.tween_motion);
        }
        let rr = ui.interact(row, ui.id().with(("me_row", p.key())), Sense::click());
        let label = match p.filter() {
            // Named with its filter for screen readers and tips ("Blur: Blur X").
            Some(f) => tr!("{filter}: {property}", filter = crate::i18n::t(filter_name(&target, f.slot)), property = label),
            None => label.to_string(),
        };
        let label = label.as_str();
        a11y::control(ui, &rr, WidgetType::SelectableLabel, &if track.is_some() { tr!("{property} · keyed", property = label) } else { label.to_string() }, Some(selected));
        if rr.clicked() {
            app.me_prop = p;
            app.me_key = None;
        }
        // The value at the playhead: drag or type it to key the property there.
        let field = Rect::from_min_size(pos2(row.right() - 128.0, row.top() + 3.0), vec2(88.0, ROW_H - 6.0));
        let mut v = now_of(p);
        let range = p.range();
        let speed = if matches!(p, Property::X | Property::Y) { 1.0 } else { 0.5 };
        let suffix = if p.unit() == "px" { " px" } else { p.unit() };
        // In a child placed on the row, so the field never grows the dock.
        let mut cell = ui.new_child(egui::UiBuilder::new().max_rect(field).layout(egui::Layout::left_to_right(egui::Align::Center)));
        let r = with_type(&mut cell, Type::Data, |ui| ui.add_sized(vec2(field.width(), field.height()), egui::DragValue::new(&mut v).speed(speed).range(range).max_decimals(1).suffix(suffix)).named(label));
        if r.changed() {
            app.me_prop = p;
            app.s.begin_gesture(tr_noop!("Property keyframe"));
            app.run("motion.set_key", json!({"property": p.key(), "frame": app.s.frame, "value": v}));
        }
        if r.drag_stopped() {
            app.s.end_gesture();
        }
        // Key toggle: a filled diamond where this property has a key at the playhead.
        let keyed = track.is_some_and(|t| t.key_at(offset).is_some());
        let toggle = Rect::from_center_size(pos2(row.right() - 20.0, row.center().y), vec2(24.0, 24.0));
        let tr = ui.interact(toggle, ui.id().with(("me_key", p.key())), Sense::click());
        if tr.hovered() {
            painter.rect_filled(toggle, T.rc(), T.hover);
        }
        diamond(&painter, toggle.center(), 5.0, if keyed { T.text } else { Color32::TRANSPARENT }, Stroke::new(1.5, if keyed { T.text } else { T.text_faint }));
        let tip = match (keyed, pin_at(p, offset)) {
            (true, Some(q)) if m.path.is_some() && interior(&pins, q) => tr!("Unpin point {anchor} of the motion path from frame {frame}: its X and Y keyframes go", anchor = q.anchor, frame = app.s.frame + 1),
            (true, _) => tr!("Remove the {property} keyframe at frame {frame}", property = label, frame = app.s.frame + 1),
            (false, _) => tr!("Add a {property} keyframe at frame {frame}", property = label, frame = app.s.frame + 1),
        };
        a11y::control(ui, &tr, WidgetType::Checkbox, &tr!("{property} keyframe at frame {frame}", property = label, frame = app.s.frame + 1), Some(keyed));
        if tr.on_hover_text(tip).clicked() {
            app.me_prop = p;
            if keyed {
                app.run("motion.remove_key", json!({"property": p.key(), "frame": app.s.frame}));
            } else {
                app.run("motion.set_key", json!({"property": p.key(), "frame": app.s.frame, "value": now_of(p)}));
            }
        }
        painter.line_segment([row.left_bottom(), row.right_bottom()], Stroke::new(1.0, T.separator));
    }
    painter.line_segment([pos2(list.right(), body.top()), pos2(list.right(), body.bottom())], Stroke::new(1.0, T.border));

    // ---- graph -----------------------------------------------------------------------------
    let p = app.me_prop;
    let track = m.track(p);
    let label = match p.filter() {
        Some(f) => tr!("{filter}: {property}", filter = crate::i18n::t(filter_name(&target, f.slot)), property = crate::i18n::t(p.label())),
        None => crate::i18n::t(p.label()).to_string(),
    };
    let (lo, hi) = match app.me_drag {
        Some(MeDrag::Key { lo, hi, lane: false, .. } | MeDrag::Handle { lo, hi, .. }) => (lo, hi),
        _ => value_range(track, own(p), frames, p),
    };
    let last = frames.saturating_sub(1).max(1) as f32;
    // Wider on the left: the value labels sit there, clear of the first key.
    let inner = Rect::from_min_max(graph.min + vec2(PAD * 2.0, PAD * 0.75), graph.max - vec2(PAD, PAD * 0.75));
    let x_of = |f: f32| inner.left() + f / last * inner.width();
    let f_of = |x: f32| ((x - inner.left()) / inner.width().max(1.0) * last).clamp(0.0, last);
    let y_of = |v: f32| inner.bottom() - (v - lo) / (hi - lo).max(1e-6) * inner.height();
    let v_of = |y: f32| lo + (inner.bottom() - y) / inner.height().max(1.0) * (hi - lo);
    let gp = painter.with_clip_rect(graph_full);
    gp.rect_filled(ruler, 0.0, T.panel_alt);
    // Value grid with labels; frame ticks on the ruler.
    let step = nice_step(hi - lo);
    let mut v = (lo / step).ceil() * step;
    for _ in 0..64 {
        if v > hi {
            break;
        }
        let y = y_of(v);
        gp.line_segment([pos2(graph.left(), y), pos2(graph.right(), y)], Stroke::new(1.0, T.separator));
        gp.text(pos2(graph.left() + 4.0, y - 1.0), Align2::LEFT_BOTTOM, format!("{:.0}", v + 0.0), font(Type::DataSmall), T.text_faint);
        v += step;
    }
    let px_per_frame = inner.width() / last;
    let fstep = [1u32, 2, 5, 10, 20, 50, 100, 200, 500, 1000].into_iter().find(|s| *s as f32 * px_per_frame >= 28.0).unwrap_or(5000);
    for f in 0..frames {
        let n = start + f + 1;
        let x = x_of(f as f32).round() + 0.5;
        if n % fstep == 0 || f == 0 {
            gp.line_segment([pos2(x, ruler.bottom() - 6.0), pos2(x, ruler.bottom())], Stroke::new(1.0, T.text_faint));
            gp.text(pos2(x, ruler.top() + 3.0), Align2::CENTER_TOP, n.to_string(), font(Type::DataSmall), color(Type::DataSmall));
            gp.line_segment([pos2(x, graph.top()), pos2(x, graph.bottom())], Stroke::new(1.0, T.separator));
        }
    }
    // The playhead.
    if app.s.frame >= start && app.s.frame < start + frames {
        let x = x_of(offset as f32).round() + 0.5;
        gp.line_segment([pos2(x, ruler.bottom()), pos2(x, graph.bottom())], Stroke::new(1.5, T.playhead));
        let label = (app.s.frame + 1).to_string();
        let w = painter.layout_no_wrap(label.clone(), font(Type::DataSmall), T.ink).rect.width();
        let head = Rect::from_center_size(pos2(x, ruler.center().y), vec2((w + 8.0).max(22.0), RULER_H - 4.0));
        gp.rect_filled(head, T.rc(), T.playhead);
        gp.text(head.center(), Align2::CENTER_CENTER, label, font(Type::DataSmall), T.ink);
    }
    // The curve: sampled every two points; flat at the object's own value when unkeyed.
    let mut pts = Vec::new();
    let mut x = inner.left();
    while x <= inner.right() + 0.5 {
        let f = f_of(x);
        let v = track.and_then(|t| t.value_at(f)).unwrap_or(own(p));
        pts.push(pos2(x, y_of(v)));
        x += 2.0;
    }
    gp.add(egui::Shape::line(pts, Stroke::new(2.0, if track.is_some() { T.tween_motion } else { T.text_faint })));
    gp.text(pos2(graph.right() - 8.0, graph.top() + 6.0), Align2::RIGHT_TOP, format!("{label} · {}", fmt(p, now_of(p))), font(Type::Data), color(Type::Data));
    if track.is_none() {
        gp.text(graph.center(), Align2::CENTER_CENTER, tr!("{property} has no keyframes: it stays {value}. Click ◇ or drag its value to key it here.", property = label, value = fmt(p, own(p))), font(Type::Caption), color(Type::Caption));
    }

    // Keys and the selected segment's handles.
    let keys: Vec<(u32, f32, Easing)> = track.map(|t| t.keys.iter().filter(|k| k.frame < frames).map(|k| (k.frame, k.value, k.ease.clone())).collect()).unwrap_or_default();
    // The selected key: the one picked, else the one starting the segment at the playhead.
    let sel_key = app.me_key.filter(|f| keys.iter().any(|k| k.0 == *f)).or_else(|| keys.iter().rev().find(|k| k.0 <= offset).map(|k| k.0));
    let handles: Option<[Pos2; 4]> = sel_key.and_then(|f| {
        let i = keys.iter().position(|k| k.0 == f)?;
        let (a, b) = (keys.get(i)?, track?.keys.get(i + 1)?);
        // Eases one curve can't draw (Bounce, Elastic, custom curves) are shaped in the
        // Custom Ease window instead.
        let (x1, y1, x2, y2) = a.2.as_bezier()?;
        let (f0, f1) = (a.0 as f32, b.frame as f32);
        let dv = handle_span(a.1, b.value, lo, hi);
        let at = |x: f32, y: f32| pos2(x_of(f0 + x * (f1 - f0)), y_of(a.1 + y * dv));
        Some([pos2(x_of(f0), y_of(a.1)), at(x1, y1), at(x2, y2), pos2(x_of(f1), y_of(b.value))])
    });
    if let Some([p0, p1, p2, p3]) = handles {
        gp.line_segment([p0, p1], Stroke::new(1.0, T.text_faint));
        gp.line_segment([p3, p2], Stroke::new(1.0, T.text_faint));
        for h in [p1, p2] {
            gp.circle_filled(h, 4.5, T.tween_shape);
            gp.circle_stroke(h, 4.5, Stroke::new(1.0, T.ink));
        }
    }
    // Every property's keys, a diamond per frame: filled in the accent when all are picked.
    gp.rect_filled(lane, 0.0, T.panel);
    gp.line_segment([lane.left_bottom(), lane.right_bottom()], Stroke::new(1.0, T.separator));
    gp.text(pos2(lane.left() + 4.0, lane.center().y), Align2::LEFT_CENTER, tr!("All"), font(Type::DataSmall), T.text_faint);
    let lane_frames: Vec<u32> = m.key_frames().into_iter().filter(|f| *f < frames).collect();
    let all_at = |f: u32| -> Vec<(Property, u32)> { m.tracks.iter().filter(|t| t.key_at(f).is_some()).map(|t| (t.property, f)).collect() };
    for &f in &lane_frames {
        let all_picked = all_at(f).iter().all(|k| app.me_sel.contains(k));
        let some = all_at(f).iter().any(|k| app.me_sel.contains(k));
        let fill = if all_picked { T.accent } else if some { T.accent_dim } else { T.text_dim };
        diamond(&gp, pos2(x_of(f as f32), lane.center().y), 4.5, fill, Stroke::new(1.0, T.ink));
    }
    for &(f, v, _) in &keys {
        let c = pos2(x_of(f as f32), y_of(v));
        let picked = sel_key == Some(f) || app.me_sel.contains(&(p, f));
        if pin_at(p, f).is_some() {
            // A path point's place: the motion colour, ringed like the stage's pinned points.
            diamond(&gp, c, if picked { 7.5 } else { 6.5 }, if picked { T.accent } else { T.tween_motion }, Stroke::new(1.5, T.ink));
        } else {
            diamond(&gp, c, if picked { 6.5 } else { 5.5 }, if picked { T.accent } else { T.text }, Stroke::new(1.0, T.ink));
        }
    }
    if matches!(p, Property::X | Property::Y) && pins.len() > 2 {
        gp.text(pos2(graph.right() - 8.0, graph.bottom() - 6.0), Align2::RIGHT_BOTTOM, tr!("Blue keys are pinned motion path points: drag one to move its point"), font(Type::Caption), T.tween_motion);
    }

    // ---- graph interaction -------------------------------------------------------------------
    // Pointer work; the keyboard route to the same values is the property list and its fields.
    // Focusable: with the curve focused, Left and Right move the picked keys (ADR 0083).
    let gr = ui.interact(graph_full, ui.id().with("me_graph"), Sense::click_and_drag());
    // Where frames and values are drawn, for tests that point at keys.
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("me_geometry"), (inner, lane, last, lo, hi)));
    a11y::surface(ui, &gr, &tr!("{property} curve: Left and Right move the picked keys a frame", property = label));
    keyboard(app, ui, &gr, start);
    let hover = gr.hover_pos();
    let near_key = |pt: Pos2| keys.iter().find(|k| pt.y > lane.bottom() && pos2(x_of(k.0 as f32), y_of(k.1)).distance(pt) <= GRAB).map(|k| k.0);
    let near_lane = |pt: Pos2| lane.expand2(vec2(0.0, 2.0)).contains(pt).then(|| lane_frames.iter().copied().find(|f| (x_of(*f as f32) - pt.x).abs() <= GRAB)).flatten();
    let add = ui.input(|i| i.modifiers.shift || i.modifiers.command);
    let near_handle = |pt: Pos2| handles.and_then(|h| [(1u8, h[1]), (2u8, h[2])].into_iter().find(|(_, q)| q.distance(pt) <= GRAB).map(|(w, _)| w));
    if app.me_drag.is_none()
        && let Some(h) = hover
    {
        if near_handle(h).is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        } else if near_key(h).is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
        } else if near_lane(h).is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
    }
    let to_frame = |x: f32| start + f_of(x).round() as u32;
    if gr.drag_started()
        && let Some(pt) = ui.input(|i| i.pointer.press_origin())
    {
        app.playing = false;
        gr.request_focus();
        app.me_drag = Some(match (near_handle(pt), sel_key, near_key(pt), near_lane(pt)) {
            (Some(which), Some(frame), _, _) => {
                app.s.begin_gesture(tr_noop!("Property ease"));
                MeDrag::Handle { frame, which, lo, hi }
            }
            (_, _, Some(frame), _) => {
                // A key outside the pick: it becomes the pick (or joins it, with Shift).
                if !app.me_sel.contains(&(p, frame)) {
                    if !add {
                        app.me_sel.clear();
                    }
                    app.me_sel.push((p, frame));
                }
                app.me_key = Some(frame);
                app.s.begin_gesture(tr_noop!("Property keyframe"));
                app.run("timeline.goto", json!({"frame": start + frame}));
                MeDrag::Key { frame, lo, hi, axis: Axis::Undecided, by: 0, lane: false }
            }
            (_, _, _, Some(frame)) => {
                pick_frame(app, all_at(frame), add);
                app.s.begin_gesture(tr_noop!("Move property keyframes"));
                app.run("timeline.goto", json!({"frame": start + frame}));
                MeDrag::Key { frame, lo, hi, axis: Axis::Time, by: 0, lane: true }
            }
            _ => MeDrag::Scrub,
        });
    }
    if gr.dragged()
        && let Some(pt) = gr.interact_pointer_pos()
    {
        match app.me_drag {
            Some(MeDrag::Key { frame, lo, hi, axis, by, lane }) => {
                let origin = ui.input(|i| i.pointer.press_origin()).unwrap_or(pt);
                let d = pt - origin;
                let axis = match axis {
                    Axis::Undecided if d.length() < AXIS_SLOP => Axis::Undecided,
                    Axis::Undecided if d.x.abs() > d.y.abs() => Axis::Time,
                    Axis::Undecided => Axis::Value,
                    a => a,
                };
                let mut by = by;
                match axis {
                    Axis::Time => {
                        // Snapped to frames; the engine stops the keys at their neighbours.
                        let want = f_of(pt.x).round() as i64 - i64::from(frame);
                        if want != by
                            && let Some(moved) = retime(app, start, want - by)
                        {
                            by += moved;
                            let now = u32::try_from(i64::from(frame) + by).unwrap_or(frame);
                            if !lane {
                                app.me_key = Some(now);
                            }
                            app.run("timeline.goto", json!({"frame": start + now}));
                        }
                    }
                    Axis::Value if !lane => {
                        let r = p.range();
                        let v = v_of(pt.y).clamp(*r.start(), *r.end());
                        app.run("motion.set_key", json!({"property": p.key(), "frame": start + frame, "value": v}));
                    }
                    _ => {}
                }
                app.me_drag = Some(MeDrag::Key { frame, lo, hi, axis, by, lane });
            }
            Some(MeDrag::Handle { frame, which, .. }) => {
                if let Some(t) = track
                    && let Some(i) = t.key_at(frame)
                    && let (Some(a), Some(b)) = (t.keys.get(i), t.keys.get(i + 1))
                {
                    let (mut x1, mut y1, mut x2, mut y2) = a.ease.as_bezier().unwrap_or((1.0 / 3.0, 1.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0));
                    let span = (b.frame - a.frame).max(1) as f32;
                    let nx = ((f_of(pt.x) - a.frame as f32) / span).clamp(0.0, 1.0);
                    let ny = ((v_of(pt.y) - a.value) / handle_span(a.value, b.value, lo, hi)).clamp(-5.0, 5.0);
                    if which == 1 {
                        (x1, y1) = (nx, ny);
                    } else {
                        (x2, y2) = (nx, ny);
                    }
                    app.run("motion.set_ease", json!({"property": p.key(), "frame": start + frame, "easing": {"bezier": [x1, y1, x2, y2]}}));
                }
            }
            Some(MeDrag::Scrub) => {
                let f = to_frame(pt.x);
                if f != app.s.frame {
                    app.run("timeline.goto", json!({"frame": f}));
                }
            }
            None => {}
        }
    }
    // Mid-drag in time: how far the keys have gone, at the pointer.
    let shot_at = |f: u32, by: i64| app.me_shot_drag.then(|| pos2(x_of((i64::from(f) + by) as f32), lane.bottom() + 26.0));
    if let Some(MeDrag::Key { frame, by, axis: Axis::Time, .. }) = app.me_drag
        && let Some(pt) = gr.interact_pointer_pos().or_else(|| shot_at(frame, by))
    {
        let n = app.me_sel.len();
        let label = trn!("{n} key · {by} frames → frame {frame}", "{n} keys · {by} frames → frame {frame}", n, by = format!("{by:+}"), frame = i64::from(start + frame) + by + 1);
        readout(&painter, pt + vec2(14.0, -22.0), label);
    }
    if gr.drag_stopped() {
        if let Some(MeDrag::Key { by, axis: Axis::Time, .. }) = app.me_drag
            && by != 0
        {
            let n = app.me_sel.len();
            app.status_ok(trn!("Moved {n} property keyframe by {by} frames — {undo} to undo", "Moved {n} property keyframes by {by} frames — {undo} to undo", n, by = format!("{by:+}"), undo = app.undo_key()));
        }
        if matches!(app.me_drag, Some(MeDrag::Key { .. } | MeDrag::Handle { .. })) {
            app.s.end_gesture();
        }
        app.me_drag = None;
    }
    if gr.clicked()
        && let Some(pt) = gr.interact_pointer_pos()
    {
        app.playing = false;
        gr.request_focus();
        match (near_key(pt), near_lane(pt)) {
            (Some(f), _) => {
                // Shift- (or Ctrl-) click adds a key to the pick, or takes it out.
                match app.me_sel.iter().position(|k| *k == (p, f)) {
                    Some(i) if add => {
                        app.me_sel.remove(i);
                    }
                    None if add => app.me_sel.push((p, f)),
                    _ => app.me_sel = vec![(p, f)],
                }
                app.me_key = Some(f);
                app.run("timeline.goto", json!({"frame": start + f}));
            }
            (None, Some(f)) => {
                pick_frame(app, all_at(f), add);
                app.run("timeline.goto", json!({"frame": start + f}));
            }
            (None, None) => {
                app.me_key = None;
                if !add {
                    app.me_sel.clear();
                }
                app.run("timeline.goto", json!({"frame": to_frame(pt.x)}));
            }
        }
    }
    if gr.double_clicked()
        && let Some(pt) = gr.interact_pointer_pos()
        && near_key(pt).is_none()
        && pt.y > ruler.bottom()
    {
        // A key on the curve where it is now (the motion doesn't change until it is dragged).
        let f = to_frame(pt.x);
        let v = track.and_then(|t| t.value_at((f - start) as f32)).unwrap_or(own(p));
        if app.run("motion.set_key", json!({"property": p.key(), "frame": f, "value": v})).is_some() {
            app.me_key = Some(f - start);
        }
    }
    if app.me_drag.is_none() {
        match (hover.and_then(near_key).and_then(|f| pin_at(p, f)), hover.and_then(near_lane)) {
            (Some(q), _) => gr.on_hover_text_at_pointer(tr!("Frame {frame} · point {anchor} of the motion path: drag to move the point", frame = start + q.frame + 1, anchor = q.anchor)),
            (None, Some(f)) => gr.on_hover_text_at_pointer(tr!("Every property's keys at frame {frame}: click to pick them, drag to move them in time", frame = start + f + 1)),
            (None, None) => gr.on_hover_text_at_pointer(tr!("Drag a key sideways to move it in time, or up or down to change it; drag a handle to shape the ease. Shift-click picks more keys, Left and Right move them a frame. Double-click to add a key.")),
        };
    }

    footer(app, ui, body, track, sel_key, start, &rows, &label);
}

/// The name of the object's filter at `slot` (Animate's, for the list and screen readers).
fn filter_name(target: &Element, slot: u8) -> &'static str {
    match &target.kind {
        ElementKind::Symbol { filters, .. } => filters.get(usize::from(slot)).map_or("Filter", |e| e.filter.label()), // i18n-exempt: translated where drawn (i18n::t)
        _ => "Filter", // i18n-exempt: translated where drawn (i18n::t)
    }
}

/// Picks `keys` (every property's keys on one frame): instead of the pick, or added to it.
fn pick_frame(app: &mut App, keys: Vec<(Property, u32)>, add: bool) {
    if !add {
        app.me_sel.clear();
    }
    for k in keys {
        if !app.me_sel.contains(&k) {
            app.me_sel.push(k);
        }
    }
}

/// Moves the picked keys `by` frames (`motion.move_keys`, which stops them at their
/// neighbours); the pick follows them. How far they went, `None` when nothing moved.
pub fn retime(app: &mut App, start: u32, by: i64) -> Option<i64> {
    if app.me_sel.is_empty() || by == 0 {
        return None;
    }
    let keys: Vec<serde_json::Value> = app.me_sel.iter().map(|(q, f)| json!({"property": q.key(), "frame": start + f})).collect();
    let v = app.run("motion.move_keys", json!({"keys": keys, "by": by, "frame": start}))?;
    let moved = v["by"].as_i64().unwrap_or(0);
    if let Some(list) = v["keys"].as_array() {
        app.me_sel = list.iter().filter_map(|k| Some((Property::from_key(k["property"].as_str()?)?, u32::try_from(k["frame"].as_u64()?).ok()?.checked_sub(start)?))).collect();
    }
    (moved != 0).then_some(moved)
}

/// The focused curve's keys: Left and Right move the picked keys a frame (Shift: ten), one
/// undo step a press; the arrows stay with the curve instead of moving focus.
fn keyboard(app: &mut App, ui: &egui::Ui, gr: &egui::Response, start: u32) {
    if !gr.has_focus() {
        return;
    }
    // A press elsewhere takes the keyboard back.
    if ui.input(|i| i.pointer.any_pressed()) && !gr.hovered() {
        gr.surrender_focus();
        return;
    }
    a11y::focus_ring(ui, gr);
    ui.memory_mut(|m| m.set_focus_lock_filter(gr.id, egui::EventFilter { horizontal_arrows: true, ..Default::default() }));
    if app.me_sel.is_empty() {
        return;
    }
    let by = ui.input_mut(|i| {
        let mut by = 0i64;
        // Shift first: a plain arrow's match would take Shift+arrow too.
        for (m, k, d) in [(egui::Modifiers::SHIFT, egui::Key::ArrowLeft, -10), (egui::Modifiers::SHIFT, egui::Key::ArrowRight, 10), (egui::Modifiers::NONE, egui::Key::ArrowLeft, -1), (egui::Modifiers::NONE, egui::Key::ArrowRight, 1)] {
            while i.consume_key(m, k) {
                by += d;
            }
        }
        by
    });
    if by != 0 && retime(app, start, by).is_some() {
        if let Some(f) = app.me_sel.iter().find(|k| k.0 == app.me_prop).map(|k| k.1) {
            app.me_key = Some(f);
        }
        let n = app.me_sel.len();
        app.status_ok(trn!("Moved {n} property keyframe — {undo} to undo", "Moved {n} property keyframes — {undo} to undo", n, undo = app.undo_key()));
    }
}

/// A small readout box at `at` (a drag's distance), as the timeline's span drag shows.
fn readout(painter: &egui::Painter, at: Pos2, label: String) {
    let galley = painter.layout_no_wrap(label, font(Type::DataSmall), T.text);
    let (w, h) = (galley.rect.width(), galley.rect.height());
    let bg = Rect::from_min_size(at, vec2(w + 8.0, h + 4.0));
    painter.rect_filled(bg, T.rc(), T.chrome);
    painter.rect_stroke(bg, T.rc(), Stroke::new(1.0, T.border), egui::StrokeKind::Inside);
    painter.galley(bg.min + vec2(4.0, 2.0), galley, T.text);
}

/// Whether `q` is a pin between the ends (which can be let go).
fn interior(pins: &[Pin], q: Pin) -> bool {
    pins.first() != Some(&q) && pins.last() != Some(&q)
}

/// The value distance a segment's handles are measured against: the segment's own rise, or a
/// quarter of the view for a flat segment (whose ease then shows no change until its ends
/// differ).
fn handle_span(v0: f32, v1: f32, lo: f32, hi: f32) -> f32 {
    let d = v1 - v0;
    if d.abs() > (hi - lo) * 1e-3 { d } else { (hi - lo) * 0.25 }
}

fn diamond(p: &egui::Painter, c: Pos2, r: f32, fill: Color32, stroke: Stroke) {
    p.add(egui::Shape::convex_polygon(vec![c + vec2(0.0, -r), c + vec2(r, 0.0), c + vec2(0.0, r), c + vec2(-r, 0.0)], fill, stroke));
}

/// The ease of the selected segment, and the tween's own actions.
#[allow(clippy::too_many_arguments)]
fn footer(app: &mut App, ui: &mut egui::Ui, body: Rect, track: Option<&Track>, sel_key: Option<u32>, start: u32, rows: &[Property], label: &str) {
    let bar = Rect::from_min_max(pos2(body.left(), body.bottom() - FOOTER_H), body.right_bottom());
    ui.painter().rect_filled(bar, SQUARE, T.panel_alt);
    ui.painter().line_segment([bar.left_top(), bar.right_top()], Stroke::new(1.0, T.border));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink2(vec2(10.0, 4.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let ui = &mut child;
    let p = app.me_prop;
    let seg = sel_key.zip(track).and_then(|(f, t)| t.key_at(f).filter(|i| *i + 1 < t.keys.len()).and_then(|i| t.keys.get(i)).map(|k| (f, k.ease.clone())));
    ui.label(text(Type::Label, tr!("{property} ease", property = label)));
    match seg {
        Some((f, ease)) => {
            // The presets and the saved eases (ADR 0081); Edit Ease… shapes the segment's curve.
            match crate::ease_editor::picker(app, ui, "me_ease", &ease, 160.0, tr!("Ease")) {
                Some(crate::ease_editor::Picked::Ease(e)) => {
                    app.run("motion.set_ease", json!({"property": p.key(), "frame": start + f, "easing": e}));
                }
                Some(crate::ease_editor::Picked::Edit) => crate::ease_editor::open_motion(app, start + f, p),
                None => {}
            }
            if ui.button("✎").on_hover_text(tr!("Edit Ease…")).named(tr!("Edit Ease…")).clicked() {
                crate::ease_editor::open_motion(app, start + f, p);
            }
            if ui.button(tr!("Same for all")).on_hover_text(tr!("Give every property's segment at this frame this ease")).clicked() {
                app.run("motion.set_ease", json!({"frame": start + f, "easing": anicraft_engine::params::easing_json(&ease)}));
            }
        }
        None => {
            ui.label(text(Type::Caption, tr!("Pick a key that has another key after it")));
        }
    }
    ui.separator();
    // Animate's "+" for colour effects: list another style's properties (keying one adds it).
    let missing: Vec<ColorStyle> = [ColorStyle::Alpha, ColorStyle::Brightness, ColorStyle::Tint, ColorStyle::Advanced].into_iter().filter(|s| !rows.iter().any(|p| p.style() == Some(*s))).collect();
    if !missing.is_empty() {
        ui.menu_button(tr!("Add Colour Effect"), |ui| {
            for st in missing {
                if ui.button(crate::i18n::t(st.label())).clicked() {
                    app.me_styles.push(st);
                    app.me_prop = Property::ALL.into_iter().find(|p| p.style() == Some(st)).unwrap_or(Property::Alpha);
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text(tr!("List a colour effect style's properties here; keying one adds it to the tween"));
    }
    if ui.button(tr!("Remove Tween")).on_hover_text(tr!("The object stays as it is on the span's first frame ({undo} to undo)", undo = app.undo_key())).clicked() {
        app.run("timeline.remove_tween", json!({}));
    }
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.label(text(Type::Caption, tr!("Moving the object on the stage inside the span adds keys here")));
    });
}

/// No motion tween at the playhead: say how to get one.
fn empty_state(app: &mut App, ui: &mut egui::Ui, body: Rect) {
    let k = app.s.active_layer().and_then(|l| l.keyframe_at(app.s.frame).map(|k| (!k.elements.is_empty(), k.motion.is_some())));
    let msg = match k {
        Some((true, _)) => tr!("This span has no motion tween. Create one to animate its object property by property."),
        Some((false, _)) => tr!("This span is empty. Draw or place an object, then create a motion tween."),
        None => tr!("The playhead is on an empty frame of this layer."),
    };
    let r = Rect::from_center_size(body.center(), vec2(body.width().min(520.0), 90.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r).layout(egui::Layout::top_down(egui::Align::Center)));
    child.label(text(Type::Heading, tr!("Motion Editor")));
    child.add_space(4.0);
    child.label(text(Type::Caption, msg));
    child.add_space(6.0);
    if k.is_some_and(|(has, _)| has) && child.button(tr!("Create Motion Tween")).clicked() {
        app.run("timeline.create_motion_tween", json!({}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ADR 0092: each filter of the object is listed by name, its numbers under it, after the
    /// colour effect.
    #[test]
    fn filters_list_their_numbers_under_their_names() {
        use anicraft_engine::doc::{ColorEffect, ElementId, Filter, FilterEntry, SymbolId};
        use anicraft_engine::geom::Affine;
        let filters = vec![FilterEntry::from(Filter::new("blur").unwrap()), FilterEntry { filter: Filter::new("adjust_color").unwrap(), enabled: false }];
        let clip = Element { id: ElementId(1), name: String::new(), transform: Affine::IDENTITY, color: ColorEffect::default(), kind: ElementKind::Symbol { symbol: SymbolId(1), loop_mode: Default::default(), first_frame: 0, blend: Default::default(), filters, events: Vec::new() } };
        let rows = rows_for(&[], &[], &clip);
        let names: Vec<String> = rows.iter().skip_while(|r| matches!(r, Row::Prop(p) if !p.is_filter())).map(|r| match r {
            Row::Filter(slot, name, on) => format!("[{slot} {name} {on}]"),
            Row::Prop(p) => p.key().into_owned(),
        }).collect();
        assert_eq!(names, ["[0 Blur true]", "filter.0.blur_x", "filter.0.blur_y", "[1 Adjust Color false]", "filter.1.brightness", "filter.1.contrast", "filter.1.saturation", "filter.1.hue"]);
        assert_eq!(filter_name(&clip, 1), "Adjust Color");
        assert_eq!(filter_name(&clip, 7), "Filter");
    }

    #[test]
    fn grid_steps_are_round_numbers() {
        for (span, want) in [(4.0, 1.0), (10.0, 5.0), (100.0, 50.0), (360.0, 100.0), (0.0, 0.001)] {
            assert!((nice_step(span) - want).abs() < 1e-6, "{span}: {}", nice_step(span));
        }
    }

    #[test]
    fn the_view_holds_the_curve_its_keys_and_handles() {
        use anicraft_engine::doc::PropertyKey;
        let t = Track { property: Property::Y, keys: vec![PropertyKey { frame: 0, value: 0.0, ease: Easing::CubicBezier(0.3, 1.8, 0.7, 1.0) }, PropertyKey { frame: 10, value: 100.0, ease: Easing::Linear }] };
        let (lo, hi) = value_range(Some(&t), 0.0, 11, Property::Y);
        // The overshooting handle (180 % of the rise) is inside the view, with a margin.
        assert!(lo < 0.0 && hi > 180.0, "{lo} {hi}");
        // A flat property still gets a usable view around its value.
        let (lo, hi) = value_range(None, 50.0, 10, Property::Alpha);
        assert!(lo < 50.0 && hi > 50.0 && hi - lo >= 10.0);
        // Handles of a flat segment measure against a share of the view instead of zero.
        assert_eq!(handle_span(5.0, 5.0, 0.0, 100.0), 25.0);
        assert_eq!(handle_span(0.0, 40.0, 0.0, 100.0), 40.0);
    }
}
