//! Edit Envelope (ADR 0039): the volume of a keyframe's sound over its time, one lane per
//! channel, as Animate's envelope editor. Each lane shows that channel's waveform over the time
//! the sound is heard (its repeats, its span for a stream) with the envelope drawn on it:
//! click a lane to add a point (up to 16), drag a point to move it (a point's time is shared
//! by both channels, its level is each channel's own), right-click it to delete it. The
//! selected point's time and levels can be typed below. Every change is the `sound.set`
//! command (one undo step per drag), so a preset turns into Custom the moment it is edited.

use crate::a11y::Named;
use crate::app::App;
use crate::icons::Icon;
use crate::theme::{T, Type, font, text, with_type};
use anicraft_engine::doc::audio::{Peaks, Voice, VoiceKind};
use anicraft_engine::doc::sound::{MAX_ENVELOPE_POINTS, MAX_ENVELOPE_SECONDS, clean_points, frame_start};
use anicraft_engine::doc::{EnvelopePoint, FrameSound, LayerId, Sound, SoundEffect, SoundId};
use egui::{Align2, Rect, Sense, Stroke, WidgetType, pos2, vec2};
use serde_json::json;
use std::sync::Arc;

/// Height of one channel's lane, and of the time ruler between them.
const LANE_H: f32 = 112.0;
const RULER_H: f32 = 20.0;
/// Half the size of a point's handle, and how near the pointer must be to take it.
const HANDLE: f32 = 4.5;
const REACH: f32 = 8.0;

/// The editor's target and view.
pub struct EnvelopeEditor {
    scene: usize,
    layer: LayerId,
    /// The keyframe's first frame.
    frame: u32,
    /// Seconds across the lanes (zoom); `None` fits the time the sound is heard.
    window: Option<f64>,
    /// Seconds at the lanes' left edge.
    from: f64,
    selected: Option<usize>,
    /// Times read in frames instead of seconds.
    frames: bool,
    /// Each channel's waveform summary, for the sound shown.
    peaks: Option<(SoundId, Arc<[Peaks; 2]>)>,
}

/// Opens the editor on the active layer's keyframe at the playhead, if it has a sound.
pub fn open(app: &mut App) {
    let Some(layer) = app.s.active_layer() else { return };
    let Some(k) = layer.keyframe_at(app.s.frame).filter(|k| k.sound.is_some()) else {
        app.status_err(tr!("Select a keyframe with a sound to edit its envelope"));
        return;
    };
    let (layer, frame) = (layer.id, k.start);
    app.envelope = Some(EnvelopeEditor { scene: app.s.scene, layer, frame, window: None, from: 0.0, selected: None, frames: false, peaks: None });
}

/// Selects point `i` (screenshots).
pub fn select(app: &mut App, i: usize) {
    if let Some(ed) = app.envelope.as_mut() {
        ed.selected = Some(i);
    }
}

/// What the editor works on, as the document has it now.
struct Target {
    fs: FrameSound,
    sound: Sound,
    /// The keyframe's span, and the frames from it to the movie's end.
    span: u32,
    rest: u32,
    /// Seconds the sound is heard (what presets are shaped over).
    heard: f64,
}

fn target(app: &App, ed: &EnvelopeEditor) -> Option<Target> {
    let doc = &app.s.doc;
    let l = doc.timeline(ed.scene).layer(ed.layer)?;
    let k = l.keyframe_starting_at(ed.frame).and_then(|i| l.keyframes.get(i))?;
    let fs = k.sound.clone()?;
    let sound = doc.sound(fs.sound)?.clone();
    let rest = doc.frame_count().saturating_sub(doc.scene_start(ed.scene).saturating_add(k.start));
    let heard = fs.heard_seconds(&sound, k.duration, rest, doc.stage.fps);
    Some(Target { fs, sound, span: k.duration, rest, heard })
}

/// The editor window, when open.
pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut ed) = app.envelope.take() else { return };
    // Closed when its keyframe or sound goes away (undo, delete).
    let Some(t) = target(app, &ed) else { return };
    let mut open = true;
    let title = tr!("Edit Envelope — {name}", name = t.sound.name);
    let shown = egui::Window::new(title.clone())
        .id(egui::Id::new("envelope_editor"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size([780.0, 2.0 * LANE_H + RULER_H + 150.0])
        .min_size([420.0, 2.0 * LANE_H + RULER_H + 120.0])
        .show(ctx, |ui| body(app, &mut ed, ui, &t));
    crate::a11y::name_title_bar(&shown, &title);
    let esc = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    if open && !esc {
        app.envelope = Some(ed);
    }
}

/// The points being edited: the custom ones, or the preset's over the heard time.
fn points_of(t: &Target) -> Vec<EnvelopePoint> {
    t.fs.effect.points(t.heard)
}

/// The levels (left, right) of `pts` at `at` seconds.
pub fn level_at(pts: &[EnvelopePoint], at: f64) -> (f32, f32) {
    let i = pts.partition_point(|p| p.at <= at);
    match (i.checked_sub(1).and_then(|j| pts.get(j)), pts.get(i)) {
        (Some(a), Some(b)) if b.at > a.at => {
            let x = (at - a.at) / (b.at - a.at);
            let mix = |p: f32, q: f32| (f64::from(p) + (f64::from(q) - f64::from(p)) * x) as f32;
            (mix(a.left, b.left), mix(a.right, b.right))
        }
        (Some(a), _) => (a.left, a.right),
        (None, Some(b)) => (b.left, b.right),
        (None, None) => (1.0, 1.0),
    }
}

fn set_points(app: &mut App, ed: &EnvelopeEditor, label: &str, pts: &[EnvelopePoint], live: bool) {
    let list: Vec<serde_json::Value> = clean_points(pts).iter().map(|p| json!({"at": p.at, "left": p.left, "right": p.right})).collect();
    let p = json!({"layer": ed.layer.0, "frame": ed.frame, "envelope": list});
    if live {
        crate::panels::live(app, label, "sound.set", p);
    } else {
        app.run("sound.set", p);
    }
}

fn body(app: &mut App, ed: &mut EnvelopeEditor, ui: &mut egui::Ui, t: &Target) {
    let fps = app.s.doc.stage.fps.max(1.0);
    let mut pts = points_of(t);
    // ---- effect, preview, zoom, units ----
    ui.horizontal(|ui| {
        ui.label(text(Type::Label, tr!("Effect")));
        egui::ComboBox::from_id_salt("envelope_effect").width(160.0).selected_text(crate::i18n::t(t.fs.effect.label())).show_ui(ui, |ui| {
            for e in SoundEffect::PRESETS {
                if ui.selectable_label(t.fs.effect == e, crate::i18n::t(e.label())).on_hover_text(crate::i18n::t(e.hint())).clicked() && t.fs.effect != e {
                    app.run("sound.set", json!({"layer": ed.layer.0, "frame": ed.frame, "effect": e.key()}));
                    ed.selected = None;
                }
            }
            let custom = matches!(t.fs.effect, SoundEffect::Custom(_));
            if ui.selectable_label(custom, tr!("Custom")).on_hover_text(crate::i18n::t(SoundEffect::Custom(Vec::new()).hint())).clicked() && !custom {
                app.run("sound.set", json!({"layer": ed.layer.0, "frame": ed.frame, "effect": "custom"}));
            }
        })
        .response
        .set_name(tr!("Sound effect"));
        ui.add_space(8.0);
        if crate::timeline::icon_button(ui, tr!("Play the sound with this envelope"), None, Icon::Play).clicked() {
            preview(app, t);
        }
        if ui.small_button(tr!("Stop")).on_hover_text(tr!("Stop the preview")).clicked() {
            app.sound.stop();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.selectable_label(ed.frames, tr!("Frames")).on_hover_text(tr!("Read times in frames")).clicked() {
                ed.frames = true;
            }
            if ui.selectable_label(!ed.frames, tr!("Seconds")).on_hover_text(tr!("Read times in seconds")).clicked() {
                ed.frames = false;
            }
            ui.add_space(8.0);
            let total = total_seconds(t, &pts);
            let shown = ed.window.unwrap_or(total);
            if ui.small_button(tr!("Fit")).on_hover_text(tr!("Show the whole sound")).named(tr!("Zoom to fit")).clicked() {
                ed.window = None;
                ed.from = 0.0;
            }
            if ui.small_button("+").on_hover_text(tr!("Zoom in")).named(tr!("Zoom in")).clicked() {
                ed.window = Some((shown * 0.5).max(1.0 / f64::from(fps) / 4.0));
            }
            if ui.small_button("−").on_hover_text(tr!("Zoom out")).named(tr!("Zoom out")).clicked() {
                let w = shown * 2.0;
                ed.window = if w >= total { None } else { Some(w) };
            }
        });
    });
    ui.add_space(4.0);
    // ---- the lanes ----
    let total = total_seconds(t, &pts);
    let window = ed.window.unwrap_or(total).clamp(1e-3, total.max(1e-3));
    ed.from = ed.from.clamp(0.0, (total - window).max(0.0));
    let peaks = channel_peaks(ed, &t.sound);
    let from = ed.from;
    let width = ui.available_width().max(200.0);
    let (area, _) = ui.allocate_exact_size(vec2(width, 2.0 * LANE_H + RULER_H), Sense::hover());
    let lanes = [Rect::from_min_size(area.min, vec2(width, LANE_H)), Rect::from_min_size(area.min + vec2(0.0, LANE_H + RULER_H), vec2(width, LANE_H))];
    let ruler = Rect::from_min_size(area.min + vec2(0.0, LANE_H), vec2(width, RULER_H));
    let x_of = |at: f64| area.left() + ((at - from) / window) as f32 * width;
    let t_of = |x: f32| (from + f64::from((x - area.left()) / width) * window).clamp(0.0, MAX_ENVELOPE_SECONDS);
    let y_of = |lane: Rect, level: f32| lane.bottom() - 4.0 - level.clamp(0.0, 1.0) * (lane.height() - 8.0);
    let level_of = |lane: Rect, y: f32| ((lane.bottom() - 4.0 - y) / (lane.height() - 8.0)).clamp(0.0, 1.0);
    let p = ui.painter_at(area);
    let mut changed: Option<(&str, bool)> = None;
    for (c, lane) in lanes.iter().enumerate() {
        let name = if c == 0 { tr!("Left channel") } else { tr!("Right channel") };
        let bg = ui.interact(*lane, ui.id().with(("env_lane", c)), Sense::click());
        crate::a11y::surface(ui, &bg, &tr!("{channel}: click to add a volume point", channel = name));
        p.rect_filled(*lane, crate::theme::SQUARE, T.field);
        p.line_segment([pos2(lane.left(), lane.center().y), pos2(lane.right(), lane.center().y)], Stroke::new(1.0, T.separator));
        if let Some(pk) = &peaks
            && let Some(pk) = pk.get(c)
        {
            draw_wave(&p, *lane, pk, &t.sound, t, from, window);
        }
        p.text(lane.left_top() + vec2(6.0, 4.0), Align2::LEFT_TOP, if c == 0 { trc!("left channel", "L") } else { trc!("right channel", "R") }, font(Type::Label), T.text_dim);
        // The envelope: its level before the first point and after the last, lines between.
        let level = |q: &EnvelopePoint| if c == 0 { q.left } else { q.right };
        let mut line: Vec<egui::Pos2> = Vec::new();
        match (pts.first(), pts.last()) {
            (Some(a), Some(b)) => {
                line.push(pos2(lane.left(), y_of(*lane, level(a))));
                line.extend(pts.iter().map(|q| pos2(x_of(q.at), y_of(*lane, level(q)))));
                line.push(pos2(lane.right(), y_of(*lane, level(b))));
            }
            _ => line.extend([pos2(lane.left(), y_of(*lane, 1.0)), pos2(lane.right(), y_of(*lane, 1.0))]),
        }
        p.add(egui::Shape::line(line, Stroke::new(1.5, T.accent)));
        // Clicking the lane adds a point there, at the level clicked.
        if bg.clicked()
            && let Some(at) = bg.interact_pointer_pos()
            && pts.len() < MAX_ENVELOPE_POINTS
        {
            let when = t_of(at.x);
            let (mut l, mut r) = level_at(&pts, when);
            if c == 0 { l = level_of(*lane, at.y) } else { r = level_of(*lane, at.y) }
            pts.push(EnvelopePoint::new(when, l, r));
            pts = clean_points(&pts);
            ed.selected = pts.iter().position(|q| q.at == when);
            changed = Some((tr_noop!("Add Envelope Point"), false));
        }
        // The points' handles: drag to move, right-click to delete.
        let mut remove = None;
        for i in 0..pts.len() {
            let Some(q) = pts.get(i).copied() else { continue };
            let centre = pos2(x_of(q.at), y_of(*lane, level(&q)));
            if !lane.expand(HANDLE).contains(centre) {
                continue;
            }
            let hit = Rect::from_center_size(centre, vec2(2.0 * REACH, 2.0 * REACH));
            let r = ui.interact(hit, ui.id().with(("env_point", c, i)), Sense::click_and_drag());
            let secs = time_label(q.at, ed.frames, fps);
            crate::a11y::control(ui, &r, WidgetType::Slider, &tr!("Point {index} of {count}, {channel}: {level} % at {time}", index = i + 1, count = pts.len(), channel = name, level = format!("{:.0}", level(&q) * 100.0), time = secs), None);
            let hot = r.hovered() || r.dragged() || ed.selected == Some(i);
            let h = Rect::from_center_size(centre, vec2(2.0 * HANDLE, 2.0 * HANDLE));
            p.rect_filled(h, crate::theme::SQUARE, if hot { T.accent } else { T.panel });
            p.rect_stroke(h, crate::theme::SQUARE, Stroke::new(1.5, T.accent), egui::StrokeKind::Inside);
            let r = r.on_hover_text(tr!("{level} % at {time} · drag to move, right-click to delete", level = format!("{:.0}", level(&q) * 100.0), time = secs));
            if r.clicked() {
                ed.selected = Some(i);
            }
            if r.dragged()
                && let Some(at) = ui.ctx().pointer_interact_pos()
            {
                // Between its neighbours, so the points keep their order.
                let lo = i.checked_sub(1).and_then(|j| pts.get(j)).map_or(0.0, |n| n.at);
                let hi = pts.get(i + 1).map_or(MAX_ENVELOPE_SECONDS, |n| n.at);
                if let Some(q) = pts.get_mut(i) {
                    q.at = t_of(at.x).clamp(lo, hi);
                    let v = level_of(*lane, at.y);
                    if c == 0 { q.left = v } else { q.right = v }
                }
                ed.selected = Some(i);
                changed = Some((tr_noop!("Move Envelope Point"), true));
            }
            if r.secondary_clicked() && pts.len() > 1 {
                remove = Some(i);
            }
        }
        if let Some(i) = remove {
            pts.remove(i);
            ed.selected = None;
            changed = Some((tr_noop!("Delete Envelope Point"), false));
        }
    }
    // The ruler: times under the lanes' x positions.
    draw_ruler(&p, ruler, from, window, ed.frames, fps);
    // Scrolling along a zoomed view: the wheel over the lanes.
    if ed.window.is_some() && ui.rect_contains_pointer(area) {
        let d = ui.input(|i| i.smooth_scroll_delta);
        let step = f64::from(-(d.x + d.y) / width) * window;
        ed.from = (ed.from + step).clamp(0.0, (total - window).max(0.0));
    }
    // ---- the selected point, typed ----
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let sel = ed.selected.filter(|&i| i < pts.len());
        match sel.and_then(|i| pts.get(i).copied().map(|q| (i, q))) {
            Some((i, mut q)) => {
                ui.label(text(Type::Label, tr!("Point {index} of {count}", index = i + 1, count = pts.len())));
                let lo = i.checked_sub(1).and_then(|j| pts.get(j)).map_or(0.0, |n| n.at);
                let hi = pts.get(i + 1).map_or(MAX_ENVELOPE_SECONDS, |n| n.at);
                let mut edit = false;
                with_type(ui, Type::Data, |ui| {
                    if ed.frames {
                        let mut f = q.at * f64::from(fps);
                        edit |= ui.add(egui::DragValue::new(&mut f).range(lo * f64::from(fps)..=hi * f64::from(fps)).speed(0.1).max_decimals(1).suffix(" f")).named(tr!("Point time")).changed();
                        q.at = f / f64::from(fps);
                    } else {
                        edit |= ui.add(egui::DragValue::new(&mut q.at).range(lo..=hi).speed(0.005).max_decimals(3).suffix(" s")).named(tr!("Point time")).changed();
                    }
                    for (left, v) in [(true, &mut q.left), (false, &mut q.right)] {
                        ui.label(text(Type::Label, if left { tr!("Left") } else { tr!("Right") }));
                        let mut pct = f64::from(*v) * 100.0;
                        if ui.add(egui::DragValue::new(&mut pct).range(0.0..=100.0).speed(0.5).max_decimals(0).suffix(" %")).named(if left { tr!("Left channel volume") } else { tr!("Right channel volume") }).changed() {
                            *v = (pct / 100.0) as f32;
                            edit = true;
                        }
                    }
                });
                if edit && let Some(slot) = pts.get_mut(i) {
                    *slot = q;
                    changed = Some((tr_noop!("Envelope Point"), true));
                }
                if ui.add_enabled(pts.len() > 1, egui::Button::new(tr!("Delete Point"))).on_hover_text(tr!("A sound keeps at least one point")).clicked() {
                    pts.remove(i);
                    ed.selected = None;
                    changed = Some((tr_noop!("Delete Envelope Point"), false));
                }
            }
            None => {
                ui.label(text(Type::Caption, trn!("Click a channel to add a point (up to {max}), drag a point to shape the volume, right-click it to delete it. {n} point.", "Click a channel to add a point (up to {max}), drag a point to shape the volume, right-click it to delete it. {n} points.", pts.len(), max = MAX_ENVELOPE_POINTS)));
            }
        }
    });
    let span = if ed.frames { let n = (t.heard * f64::from(fps)).round().max(0.0) as u64; trn!("{n} frame", "{n} frames", n) } else { format!("{:.2} s", t.heard) };
    ui.label(text(Type::Caption, tr!("Heard for {time} ({why}). Presets are shaped over this time; editing one makes it Custom.", time = span, why = heard_reason(&t.fs, t.span, t.rest))));
    if let Some((label, live)) = changed {
        set_points(app, ed, label, &pts, live);
    }
}

/// Why the sound is heard for as long as it is.
fn heard_reason(fs: &FrameSound, span: u32, rest: u32) -> String {
    use anicraft_engine::doc::SoundSync;
    match (fs.sync, fs.looping) {
        (SoundSync::Stream, true) => trn!("a stream looping over its {n}-frame span", "a stream looping over its {n}-frame span", span),
        (SoundSync::Stream, false) => trn!("a stream, within its {n}-frame span", "a stream, within its {n}-frame span", span),
        (_, true) => trn!("looping to the end of the movie, {n} frame on", "looping to the end of the movie, {n} frames on", rest),
        (_, false) if fs.repeat > 1 => trn!("played {n} time", "played {n} times", fs.repeat),
        _ => tr!("played once").to_string(),
    }
}

/// Seconds the lanes cover when fitted: the heard time, or as far as the last point.
fn total_seconds(t: &Target, pts: &[EnvelopePoint]) -> f64 {
    let last = pts.last().map_or(0.0, |p| p.at);
    t.heard.max(last).max(t.sound.seconds().min(t.heard.max(1e-3))).max(1e-3)
}

fn time_label(at: f64, frames: bool, fps: f32) -> String {
    if frames { tr!("frame {number}", number = format!("{:.1}", at * f64::from(fps) + 1.0)) } else { format!("{at:.3} s") }
}

/// The sound's channels summarised apart (once per sound; a mono sound is one channel twice).
fn channel_peaks(ed: &mut EnvelopeEditor, sound: &Sound) -> Option<Arc<[Peaks; 2]>> {
    if let Some((id, p)) = &ed.peaks
        && *id == sound.id
    {
        return Some(p.clone());
    }
    let dec = sound.decoded_if_ready()?;
    let p = Arc::new([Peaks::of_channel(&dec.pcm, 0), Peaks::of_channel(&dec.pcm, 1)]);
    ed.peaks = Some((sound.id, p.clone()));
    Some(p)
}

/// One channel's waveform over seconds `from .. from + window` of the sound's time (repeats
/// wrap to its start), as far as it is heard.
fn draw_wave(p: &egui::Painter, lane: Rect, peaks: &Peaks, sound: &Sound, t: &Target, from: f64, window: f64) {
    let len = peaks.frames() as u64;
    if len == 0 {
        return;
    }
    let rate = f64::from(sound.rate.max(1));
    let heard = (t.heard * rate) as u64;
    let per_px = window * rate / f64::from(lane.width().max(1.0));
    let (cy, h) = (lane.center().y, lane.height() * 0.5 - 6.0);
    let stroke = Stroke::new(1.0, T.waveform);
    let mut x = lane.left();
    while x < lane.right() {
        let a = ((from * rate) + f64::from(x - lane.left()) * per_px).max(0.0) as u64;
        let b = (a as f64 + per_px).ceil() as u64;
        if a >= heard {
            break;
        }
        let b = b.max(a + 1).min(heard);
        let (s, n) = ((a % len) as usize, (b - a).min(len) as usize);
        let range = if s + n <= len as usize {
            peaks.range(s, s + n)
        } else {
            match (peaks.range(s, len as usize), peaks.range(0, s + n - len as usize)) {
                (Some((l1, h1)), Some((l2, h2))) => Some((l1.min(l2), h1.max(h2))),
                (one, other) => one.or(other),
            }
        };
        if let Some((lo, hi)) = range {
            p.line_segment([pos2(x + 0.5, (cy - hi.clamp(-1.0, 1.0) * h).min(cy - 0.5)), pos2(x + 0.5, (cy - lo.clamp(-1.0, 1.0) * h).max(cy + 0.5))], stroke);
        }
        x += 1.0;
    }
}

/// Time ticks under the lanes: a step that leaves room for each label.
fn draw_ruler(p: &egui::Painter, r: Rect, from: f64, window: f64, frames: bool, fps: f32) {
    p.rect_filled(r, crate::theme::SQUARE, T.panel_alt);
    let unit = if frames { 1.0 / f64::from(fps) } else { 1.0 };
    let span = window / unit;
    let target = span * 70.0 / f64::from(r.width().max(1.0));
    let step = [0.001, 0.002, 0.005, 0.01, 0.02, 0.05, 0.1, 0.2, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 1200.0, 3600.0]
        .into_iter()
        .map(|s: f64| if frames { s.max(1.0).round() } else { s })
        .find(|s| *s >= target)
        .unwrap_or(3600.0);
    let first = (from / unit / step).ceil() as i64;
    let last = ((from + window) / unit / step).floor() as i64;
    for k in first..=last.min(first + 400) {
        let v = k as f64 * step;
        let x = r.left() + ((v * unit - from) / window) as f32 * r.width();
        p.line_segment([pos2(x, r.top()), pos2(x, r.top() + 5.0)], Stroke::new(1.0, T.text_faint));
        let label = if frames { format!("{}", v.round() as i64 + 1) } else if step < 1.0 { format!("{v:.2}s") } else { format!("{v:.0}s") };
        p.text(pos2(x + 3.0, r.center().y + 2.0), Align2::LEFT_CENTER, label, font(Type::DataSmall), T.text_dim);
    }
}

/// Plays the keyframe's sound on its own with its effect, for as long as it is heard.
fn preview(app: &mut App, t: &Target) {
    let Some(dec) = t.sound.decoded() else { return };
    let rate = app.sound.rate();
    let fps = app.s.doc.stage.fps;
    let heard = t.fs.heard(Voice::play_frames(&dec.pcm, rate), frame_start(u64::from(t.span), fps, rate), frame_start(u64::from(t.rest), fps, rate));
    let mut v = Voice::new(dec.pcm.clone(), VoiceKind::Event, 0, heard.max(1));
    v.envelope = t.fs.effect.envelope(heard, rate);
    app.sound.preview_voice(v);
    if let Some(e) = app.sound.error.take() {
        app.status_err(tr!("No sound: {error}", error = crate::i18n::t_dyn(&e)));
    }
}

