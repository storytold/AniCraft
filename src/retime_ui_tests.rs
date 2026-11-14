//! Retiming in the editor (ADR 0083), driven by pointer and key events: keys dragged in time in
//! the Motion Editor (one key, the All lane, the arrow keys), a span's end stretched, a motion
//! span picked whole and moved or Alt-copied, Split and Join Motion.

use crate::app::App;
use crate::timeline::{self, FrameSel, TlDrag};
use anicraft_engine::doc::Property;
use egui::{Event, Modifiers, PointerButton, Pos2, Rect, pos2, vec2};

fn opened(open: &str) -> (App, egui::Context) {
    let ctx = egui::Context::default();
    let session = anicraft_engine::Session::new(anicraft_engine::doc::sample::demo());
    let mut app = App::with_context(&ctx, session, Some(std::path::PathBuf::from("unused.png")));
    app.open_panel(Some(open));
    settle(&ctx, &mut app);
    (app, ctx)
}

fn frame(ctx: &egui::Context, app: &mut App, events: Vec<Event>, modifiers: Modifiers) {
    let mut all = vec![Event::ModifiersChanged(modifiers)];
    all.extend(events);
    let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 940.0))), events: all, ..Default::default() };
    ctx.run_ui(input, |ui| app.frame(ui)).textures_delta.clear();
}

fn settle(ctx: &egui::Context, app: &mut App) {
    for _ in 0..3 {
        frame(ctx, app, Vec::new(), Modifiers::NONE);
    }
}

fn button(pos: Pos2, pressed: bool, modifiers: Modifiers) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers }
}

/// A drag from `from` to `to` in steps, with `modifiers` held throughout.
fn drag(ctx: &egui::Context, app: &mut App, from: Pos2, to: Pos2, modifiers: Modifiers) {
    frame(ctx, app, vec![Event::PointerMoved(from)], modifiers);
    frame(ctx, app, vec![button(from, true, modifiers)], modifiers);
    for i in 1..=10 {
        frame(ctx, app, vec![Event::PointerMoved(from + (to - from) * (i as f32 / 10.0))], modifiers);
    }
    frame(ctx, app, vec![button(to, false, modifiers)], modifiers);
    settle(ctx, app);
}

fn click(ctx: &egui::Context, app: &mut App, at: Pos2, modifiers: Modifiers) {
    frame(ctx, app, vec![Event::PointerMoved(at)], modifiers);
    frame(ctx, app, vec![button(at, true, modifiers)], modifiers);
    frame(ctx, app, vec![button(at, false, modifiers)], modifiers);
    settle(ctx, app);
}

fn key(ctx: &egui::Context, app: &mut App, key: egui::Key, modifiers: Modifiers) {
    frame(ctx, app, vec![Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }], modifiers);
    frame(ctx, app, vec![Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers }], modifiers);
    settle(ctx, app);
}

/// The Motion Editor's graph: where offset `f` and value `v` are drawn, and the lane's middle.
struct Geometry {
    inner: Rect,
    lane: Rect,
    last: f32,
    lo: f32,
    hi: f32,
}

impl Geometry {
    fn of(ctx: &egui::Context) -> Geometry {
        let (inner, lane, last, lo, hi) = ctx.data(|d| d.get_temp::<(Rect, Rect, f32, f32, f32)>(egui::Id::new("me_geometry"))).expect("the Motion Editor drew");
        Geometry { inner, lane, last, lo, hi }
    }
    fn x(&self, f: u32) -> f32 {
        self.inner.left() + f as f32 / self.last * self.inner.width()
    }
    fn at(&self, f: u32, v: f32) -> Pos2 {
        pos2(self.x(f), self.inner.bottom() - (v - self.lo) / (self.hi - self.lo) * self.inner.height())
    }
    fn lane(&self, f: u32) -> Pos2 {
        pos2(self.x(f), self.lane.center().y)
    }
}

/// The frames (offsets) of property `p`'s keys on the active layer's first span.
fn keys(app: &App, p: Property) -> Vec<(u32, f32)> {
    app.s.active_layer().and_then(|l| l.keyframes.first()).and_then(|k| k.motion.as_ref()).and_then(|m| m.track(p)).map(|t| t.keys.iter().map(|k| (k.frame, k.value)).collect()).unwrap_or_default()
}

fn spans(app: &App) -> Vec<(u32, u32, bool)> {
    app.s.active_layer().map(|l| l.keyframes.iter().map(|k| (k.start, k.duration, k.motion.is_some())).collect()).unwrap_or_default()
}

#[test]
fn the_all_lane_and_the_arrow_keys_move_every_propertys_keys() {
    let (mut app, ctx) = opened("motion");
    let mid = keys(&app, Property::X).get(1).map(|k| k.0).expect("an X key in the middle");
    let g = Geometry::of(&ctx);
    let steps = app.s.history_len();
    // Drag the lane's diamond three frames later: X, Y (and every key there) go with it.
    drag(&ctx, &mut app, g.lane(mid), g.lane(mid + 3), Modifiers::NONE);
    assert_eq!(keys(&app, Property::X).get(1).map(|k| k.0), Some(mid + 3));
    assert_eq!(keys(&app, Property::Y).get(1).map(|k| k.0), Some(mid + 3));
    assert_eq!(app.s.history_len(), steps + 1, "one undo step for the drag");
    assert!(app.me_drag.is_none());
    // The curve has the keyboard: Left moves the picked keys back a frame, Shift+Right ten on.
    key(&ctx, &mut app, egui::Key::ArrowLeft, Modifiers::NONE);
    assert_eq!(keys(&app, Property::X).get(1).map(|k| k.0), Some(mid + 2));
    key(&ctx, &mut app, egui::Key::ArrowRight, Modifiers::SHIFT);
    assert_eq!(keys(&app, Property::Y).get(1).map(|k| k.0), Some(mid + 12));
    // A click on empty graph drops the pick; the arrows then leave the keys alone.
    let g = Geometry::of(&ctx);
    click(&ctx, &mut app, pos2(g.x(2), g.inner.top() + 4.0), Modifiers::NONE);
    assert!(app.me_sel.is_empty());
}

#[test]
fn a_key_drags_sideways_in_time_or_up_and_down_in_value() {
    let (mut app, ctx) = opened("motion");
    app.me_prop = Property::X;
    settle(&ctx, &mut app);
    let g = Geometry::of(&ctx);
    let (f, v) = keys(&app, Property::X).get(1).copied().expect("an X key in the middle");
    // Sideways: only X moves, Y stays.
    let y_before = keys(&app, Property::Y);
    drag(&ctx, &mut app, g.at(f, v), g.at(f, v) + vec2(g.x(f + 2) - g.x(f), 1.0), Modifiers::NONE);
    assert_eq!(keys(&app, Property::X).get(1), Some(&(f + 2, v)), "moved in time, its value kept");
    assert_eq!(keys(&app, Property::Y), y_before);
    // Up: the value changes, the frame doesn't.
    let g = Geometry::of(&ctx);
    drag(&ctx, &mut app, g.at(f + 2, v), g.at(f + 2, v) + vec2(1.0, -40.0), Modifiers::NONE);
    let (f2, v2) = keys(&app, Property::X).get(1).copied().expect("key");
    assert_eq!(f2, f + 2);
    assert!(v2 > v, "{v2} > {v}");
    // Shift-click another property's key: both are picked.
    app.me_prop = Property::Y;
    settle(&ctx, &mut app);
    let g = Geometry::of(&ctx);
    let (yf, yv) = keys(&app, Property::Y).get(1).copied().expect("a Y key");
    click(&ctx, &mut app, g.at(yf, yv), Modifiers::SHIFT);
    assert!(app.me_sel.contains(&(Property::X, f + 2)) && app.me_sel.contains(&(Property::Y, yf)), "{:?}", app.me_sel);
    // Shift-click it again: out of the pick.
    click(&ctx, &mut app, g.at(yf, yv), Modifiers::SHIFT);
    assert!(!app.me_sel.contains(&(Property::Y, yf)));
}

#[test]
fn the_screenshot_states_show_a_drag_and_a_stretch() {
    let (app, _ctx) = opened("motionretime");
    assert!(matches!(app.me_drag, Some(crate::motion_editor::MeDrag::Key { by: 3, lane: true, .. })));
    let (app, _ctx) = opened("motionstretch");
    assert!(matches!(app.tl_drag, TlDrag::Span { .. }));
    assert_eq!(spans(&app).first().map(|s| s.1), Some(72), "48 frames made half as long again");
    let (app, _ctx) = opened("docktabs");
    assert_eq!(app.dock.right.size, Some(160.0));
}

#[test]
fn spans_stretch_move_copy_split_and_join_from_the_timeline() {
    let (mut app, ctx) = opened("motionspan");
    let lid = app.s.layer;
    let frames = spans(&app).first().map(|s| s.1).expect("a span");
    let x_before = keys(&app, Property::X);
    // Stretch by the end (what the drag sends on release).
    app.tl_drag = TlDrag::Span { layer: lid, start: 0, end: frames, to: frames * 2 - 1 };
    settle(&ctx, &mut app);
    app.run("timeline.resize_span", serde_json::json!({"layer": lid.0, "frame": 0, "frames": frames * 2 - 1}));
    app.tl_drag = TlDrag::None;
    let x_after = keys(&app, Property::X);
    assert_eq!(x_after.iter().map(|k| k.0).collect::<Vec<_>>(), x_before.iter().map(|k| k.0 * 2).collect::<Vec<_>>());
    // A motion span is picked whole, and as whole tween spans it moves as spans.
    let layers = app.s.timeline().layers.clone();
    let layer = layers.iter().find(|l| l.id == lid).expect("layer");
    let sel = FrameSel { anchor: (lid, 0), head: (lid, frames * 2 - 2) };
    assert!(sel.is_whole_tweens(&layers));
    assert!(!FrameSel { anchor: (lid, 1), head: (lid, 4) }.is_whole_tweens(&layers));
    assert!(layer.keyframe_at(3).is_some_and(|k| k.motion.is_some()));
    // Split at the playhead, then join back.
    app.run("timeline.goto", serde_json::json!({"frame": 10}));
    app.frame_sel = None;
    let (split, join) = timeline::motion_cuts(&app);
    assert!(split && join.is_none());
    let whole = app.s.active_layer().map(|l| l.keyframes.clone());
    timeline::split_motion(&mut app);
    assert_eq!(spans(&app).iter().filter(|s| s.2).count(), 2);
    app.run("timeline.goto", serde_json::json!({"frame": 0}));
    let (split, join) = timeline::motion_cuts(&app);
    assert!(!split, "nothing to split on the first frame");
    timeline::join_motion(&mut app, join);
    assert_eq!(app.s.active_layer().map(|l| l.keyframes.clone()), whole, "split then join is exact");
    settle(&ctx, &mut app);
}

#[test]
fn a_motion_span_drags_whole_and_alt_drag_copies_it() {
    let (mut app, ctx) = opened("motionspan");
    app.show_panel(crate::dock::Panel::Timeline);
    settle(&ctx, &mut app);
    let len = spans(&app).first().map(|s| s.1).expect("span");
    // The active row's cells, as the timeline drew them.
    let row = ctx.data(|d| d.get_temp::<(Rect, f32, f32, f32)>(egui::Id::new("tl_geometry")));
    let Some((cells, x0, cell_w, scroll)) = row else { panic!("the timeline drew") };
    let x = |f: u32| x0 + (f as f32 - scroll) * cell_w + cell_w * 0.5;
    let y = cells.center().y;
    // A click on the span picks it whole.
    click(&ctx, &mut app, pos2(x(3), y), Modifiers::NONE);
    assert_eq!(app.frame_sel.map(|s| s.frames()), Some((0, len - 1)));
    // Alt-drag five frames on: a copy, the original stays.
    drag(&ctx, &mut app, pos2(x(3), y), pos2(x(3 + len + 5), y), Modifiers::ALT);
    let s = spans(&app);
    assert!(s.contains(&(0, len, true)) && s.contains(&(len + 5, len, true)), "{s:?}");
}
