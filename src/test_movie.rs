//! Test Movie (Control › Test Movie, Ctrl+Enter, as in Animate; ADR 0024): the movie played by
//! the interactive player in a window of its own on the desktop (ADR 0062; an egui immediate
//! viewport, sized to the stage, back where it was last), or floating inside the editor where
//! there can't be one (the web build) or by choice (Preferences › Test Movie), at the
//! document's frame rate, with its
//! frame actions, frame scripts and buttons live under the pointer, and an Output panel for
//! `trace` lines and messages.
//!
//! It plays a snapshot of the document taken when it opened (Restart takes a new one), so the
//! stage editor, its playhead and its undo history are untouched while it runs.
//!
//! Its sound (ADR 0039): each step's entered keyframes start their sounds
//! (`doc::sound::trigger`), movie clips' and graphics' included, and while sound plays the
//! movie keeps time with it, as the stage does.
//!
//! Its keyboard (ADR 0052): while the movie has the focus (it takes it when the window
//! opens and when clicked), keys go to the movie's keyboard listeners as ActionScript key
//! codes, Tab and the arrows included; shortcuts with Ctrl (⌘ on macOS) still reach the app,
//! and Esc or Ctrl+W (⌘W) closes the window.
//!
//! Its text fields (ADR 0066): a click in an input field (or a selectable dynamic one) gives it
//! the keyboard; typing, pasting and the system's input methods go into it, Ctrl+A (⌘A) selects
//! its text and Ctrl+C (⌘C) copies the selection, while the editing keys also reach the movie's
//! key listeners. Screen readers find each field as a text input (or a label, for dynamic text)
//! with its instance name and text.

use crate::app::App;
use crate::icons::Icon;
use crate::theme::{T, Type, text};
use anicraft_engine::doc::audio::Voice;
use anicraft_engine::doc::player::Event;
use anicraft_engine::doc::player::PlayerState;
use anicraft_engine::doc::sound::{self, frame_start};
use anicraft_engine::doc::{Document, SoundId, evaluate_player};
use anicraft_engine::geom::{Affine, Vec2};
use anicraft_engine::player::{Movie, ScriptRunner};
use anicraft_engine::render::{View, render};
use std::sync::Arc;

/// Most frames played in one UI frame when the window falls behind (a slow frame skips ahead
/// instead of racing to catch up).
const MAX_CATCH_UP: u32 = 4;
/// Largest picture edge rendered, in pixels.
const MAX_EDGE: f32 = 2048.0;

/// The movie surface's id: while it has the focus, keys go to the movie.
pub fn surface_id() -> egui::Id {
    egui::Id::new("test_movie_surface")
}

/// Whether the movie has the keyboard.
pub fn has_keys(ctx: &egui::Context) -> bool {
    ctx.memory(|m| m.focused()) == Some(surface_id())
}

/// The ActionScript key code (`flash.ui.Keyboard`) of a key, for the movie's listeners.
pub fn key_code(k: egui::Key) -> Option<u32> {
    use egui::Key as K;
    let name = k.name();
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next())
        && (c.is_ascii_uppercase() || c.is_ascii_digit())
    {
        return Some(u32::from(c));
    }
    Some(match k {
        K::ArrowLeft => 37,
        K::ArrowUp => 38,
        K::ArrowRight => 39,
        K::ArrowDown => 40,
        K::Space => 32,
        K::Enter => 13,
        K::Tab => 9,
        K::Backspace => 8,
        K::Delete => 46,
        K::Insert => 45,
        K::Home => 36,
        K::End => 35,
        K::PageUp => 33,
        K::PageDown => 34,
        K::Minus => 189,
        K::Equals | K::Plus => 187,
        K::Comma => 188,
        K::Period => 190,
        K::Slash => 191,
        K::Semicolon => 186,
        K::Quote => 222,
        K::OpenBracket => 219,
        K::CloseBracket => 221,
        K::Backslash => 220,
        K::Backtick => 192,
        K::F1 => 112,
        K::F2 => 113,
        K::F3 => 114,
        K::F4 => 115,
        K::F5 => 116,
        K::F6 => 117,
        K::F7 => 118,
        K::F8 => 119,
        K::F9 => 120,
        K::F10 => 121,
        K::F11 => 122,
        K::F12 => 123,
        _ => return None,
    })
}

pub struct TestMovie {
    doc: Document,
    movie: Movie,
    paused: bool,
    acc: f64,
    texture: Option<egui::TextureHandle>,
    /// The state and pixel size the texture shows, and whether with the hit areas.
    shown: Option<(PlayerState, [u32; 2], bool)>,
    /// Where the pointer was over the picture last frame, and whether it was down.
    pointer: Option<Vec2>,
    down: bool,
    /// The document's file name, `None` while untitled (the title is made when drawn, in the
    /// interface language).
    name: Option<String>,
    pub ms: f32,
    /// What it plays (with each voice's sound id), and whether the device plays it.
    voices: Vec<(SoundId, Voice)>,
    audio: bool,
    /// Kept scripts that don't run (ADR 0013), counted when it opened.
    skipped: usize,
    /// The movie takes the keyboard the next time it is drawn.
    focus: bool,
    /// Shift was held last frame (the movie hears key code 16 when it changes).
    shift: bool,
    /// Show Hit Areas: every button's hit area drawn over the movie (ADR 0057).
    pub hit_areas: bool,
    /// The window of its own's size and place, once worked out (ADR 0062).
    pub placement: Option<Placement>,
    /// Where the window of its own was last drawn (its outer top-left corner), when the system
    /// says.
    pub last_pos: Option<egui::Pos2>,
}

impl TestMovie {
    /// Starts the sounds the movie's last steps reached, and hands the result to the player.
    fn feed(&mut self, out: &mut crate::sound::SoundOut) {
        let started = self.movie.take_sounds();
        if started.is_empty() {
            return;
        }
        let (rate, fps) = (out.rate(), self.doc.stage.fps);
        for (tick, s) in started {
            sound::trigger(&self.doc, &mut self.voices, std::slice::from_ref(&s), frame_start(tick, fps, rate), rate);
        }
        if self.audio {
            out.set_movie(self.heard());
        }
    }

    fn heard(&self) -> Arc<Vec<Voice>> {
        Arc::new(self.voices.iter().map(|(_, v)| v.clone()).collect())
    }

    /// Sound from where the movie is (after a pause or at the start).
    fn resume(&mut self, out: &mut crate::sound::SoundOut) {
        let at = frame_start(self.movie.state.tick, self.doc.stage.fps, out.rate());
        self.audio = out.play_movie(self.heard(), at);
    }
}

/// Opens Test Movie on a snapshot of the document (or restarts it from frame 1).
pub fn open(app: &mut App) {
    app.playing = false;
    app.sound.stop();
    let doc = app.s.doc.clone();
    let mut runner = ScriptRunner::new(&app.s.plugins);
    // Each run draws its own Math.random() numbers; the player itself stays deterministic.
    let seed = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    let movie = Movie::start_seeded(&doc, if app.shot.is_some() { 0 } else { seed }, &mut runner);
    let name = app.s.path.as_ref().and_then(|p| p.file_stem()).map(|n| n.to_string_lossy().into_owned());
    let skipped = anicraft_engine::player::kept_scripts(&doc).iter().filter(|k| !k.runs()).count();
    // Show Hit Areas, and the window's size and place, stay as they were across a restart.
    let old = app.test_movie.as_ref();
    let hit_areas = old.is_some_and(|t| t.hit_areas);
    let (placement, last_pos) = (old.and_then(|t| t.placement), old.and_then(|t| t.last_pos));
    let mut tm = TestMovie { doc, movie, paused: false, acc: 0.0, texture: None, shown: None, pointer: None, down: false, name, ms: 0.0, voices: Vec::new(), audio: false, skipped, focus: true, shift: false, hit_areas, placement, last_pos };
    // Sound only when there is some, and never in screenshot runs or tests.
    if sound::has_sounds(&tm.doc) && app.shot.is_none() {
        tm.resume(&mut app.sound);
    }
    tm.feed(&mut app.sound);
    app.test_movie = Some(tm);
}

/// What a frame of the Test Movie window asks of the app, done once the window is drawn.
#[derive(Default)]
struct Asked {
    restart: bool,
    close: bool,
    /// Shortcuts with Ctrl (⌘) or Alt pressed in the window of its own: the app's commands.
    commands: Vec<&'static str>,
    quit: bool,
}

/// The size and place a window of its own opens at, worked out once (a restart keeps them, so
/// the window stays where it is).
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    pub size: egui::Vec2,
    pub at: Option<egui::Pos2>,
}

/// Room the controls and the Output take around the movie, in points.
const CHROME: egui::Vec2 = egui::vec2(24.0, 236.0);
/// The smallest window of its own.
const MIN_SIZE: [f32; 2] = [480.0, 320.0];

/// The window's size: the stage at 100 %, as in Animate, made smaller to fit 85 % of the
/// screen when it doesn't (aspect kept), never smaller than the controls need.
pub fn window_size(stage: egui::Vec2, monitor: Option<egui::Vec2>) -> egui::Vec2 {
    let stage = egui::vec2(stage.x.max(1.0), stage.y.max(1.0));
    let room = monitor.filter(|m| m.x > 0.0 && m.y > 0.0).map_or(egui::vec2(1600.0, 1000.0), |m| m * 0.85);
    let k = ((room.x - CHROME.x) / stage.x).min((room.y - CHROME.y) / stage.y).clamp(0.05, 1.0);
    let size = stage * k + CHROME;
    egui::vec2(size.x.max(MIN_SIZE[0]), size.y.max(MIN_SIZE[1]))
}

/// The Test Movie window, when open: a window of its own, or floating inside the editor where
/// there can't be one (ADR 0062).
pub fn show(app: &mut App, ctx: &egui::Context) {
    if app.test_movie.is_none() {
        return;
    }
    let asked = match crate::platform::movie_host(ctx.embed_viewports(), app.prefs.test_movie_window) {
        crate::platform::MovieHost::Window => own_window(app, ctx),
        crate::platform::MovieHost::Inside(why) => inside(app, ctx, why),
    };
    if asked.close {
        close(app);
    } else if asked.restart {
        open(app);
    }
    // The app's shortcuts pressed over the movie, run in the editor's context (a command that
    // talks to "the window" means the editor's).
    for id in asked.commands {
        app.invoke(id);
    }
    if asked.quit {
        // Through the editor's close guard, so unsaved work is never lost.
        ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
    }
}

/// Closes Test Movie, keeping where its window was.
pub fn close(app: &mut App) {
    remember_place(app);
    if app.test_movie.take().is_some_and(|tm| tm.audio) {
        app.sound.stop();
    }
}

/// Keeps where the window of its own is in the preferences, for next time (not in screenshot
/// runs, and not where the system doesn't say where windows are).
pub fn remember_place(app: &mut App) {
    let Some(at) = app.test_movie.as_ref().and_then(|t| t.last_pos) else { return };
    let at = [at.x.round(), at.y.round()];
    if app.prefs.test_movie_pos == Some(at) || !crate::platform::window_positions() {
        return;
    }
    app.prefs.test_movie_pos = Some(at);
    if app.shot.is_none()
        && let Err(e) = app.prefs.save()
    {
        app.status_err(tr!("Could not save preferences: {e}", e = e));
    }
}

/// The id of the window of its own.
pub fn viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("test_movie_window")
}

fn title(tm: &TestMovie) -> String {
    tr!("Test Movie — {name}", name = tm.name.as_deref().unwrap_or(tr!("Untitled")))
}

/// Test Movie in a window of its own: an immediate viewport, drawn in the editor's frame with
/// the editor's state (the sound output, the plug-ins, the shortcuts), so nothing is shared
/// across threads; its repaints run the editor's frame too (ADR 0062).
fn own_window(app: &mut App, ctx: &egui::Context) -> Asked {
    let monitor = ctx.input(|i| i.viewport().monitor_size);
    let remembered = app.prefs.test_movie_pos.and_then(|p| crate::platform::usable_position(p, monitor));
    let plugins = &app.s.plugins;
    let out = &mut app.sound;
    let shortcuts = &app.shortcuts;
    let Some(tm) = app.test_movie.as_mut() else { return Asked::default() };
    let stage = egui::vec2(tm.doc.stage.width as f32, tm.doc.stage.height as f32);
    // Worked out once the screen's size is known (not always on the very first frame).
    let place = match (tm.placement, monitor) {
        (Some(p), _) => p,
        (None, Some(_)) => *tm.placement.insert(Placement { size: window_size(stage, monitor), at: remembered }),
        (None, None) => Placement { size: window_size(stage, None), at: remembered },
    };
    let mut builder = egui::ViewportBuilder::default().with_title(title(tm)).with_inner_size(place.size).with_min_inner_size(MIN_SIZE);
    if let Some(at) = place.at.filter(|_| crate::platform::window_positions()) {
        builder = builder.with_position(at);
    }
    let builder = crate::platform::secondary_window(builder);
    ctx.show_viewport_immediate(viewport_id(), builder, |ui, class| {
        let mut asked = Asked::default();
        let (close_requested, outer) = ui.input(|i| (i.viewport().close_requested(), i.viewport().outer_rect));
        if let Some(r) = outer {
            tm.last_pos = Some(r.min);
        }
        // Opened or restarted: the window comes to the front, with the keyboard.
        if tm.focus {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        asked.restart = if class == egui::ViewportClass::EmbeddedWindow {
            // egui's own stand-in, with no window system under it (tests): already in a window.
            crate::a11y::name_own_title_bar(ui, &title(tm));
            body(tm, plugins, out, ui, None)
        } else {
            let frame = egui::Frame::NONE.fill(T.panel).inner_margin(egui::Margin::same(10));
            egui::CentralPanel::default().frame(frame).show(ui, |ui| body(tm, plugins, out, ui, None)).inner
        };
        // Its keys: Esc and Ctrl+W (⌘W) close it, Ctrl+Q (⌘Q) quits, and the app's shortcuts
        // with Ctrl (⌘) or Alt still work (Restart is Ctrl+Enter); the rest are the movie's.
        let (close, quit, commands) = ui.input_mut(|i| {
            let close = i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) || i.consume_key(egui::Modifiers::COMMAND, egui::Key::W);
            let quit = i.consume_key(egui::Modifiers::COMMAND, egui::Key::Q);
            let fire: Vec<&'static str> = shortcuts.iter().filter(|(m, _, id)| (m.command || m.ctrl || m.alt) && !id.starts_with("view.") && !id.starts_with("tool.")).filter_map(|(m, k, id)| i.consume_key(*m, *k).then_some(*id)).collect();
            (close, quit, fire)
        });
        asked.close = close_requested || close;
        asked.quit = quit;
        asked.commands = commands;
        asked
    })
}

/// Test Movie floating inside the editor: where there can't be a window of its own, or by choice.
fn inside(app: &mut App, ctx: &egui::Context, why: crate::platform::Inside) -> Asked {
    let plugins = &app.s.plugins;
    let out = &mut app.sound;
    let Some(tm) = app.test_movie.as_mut() else { return Asked::default() };
    let mut shown = true;
    let mut restart = false;
    let (sw, sh) = (tm.doc.stage.width as f32, tm.doc.stage.height as f32);
    let default_w = 760.0f32;
    let default_h = default_w * sh / sw.max(1.0) + 190.0;
    let title = title(tm);
    let note = why.note().map(crate::i18n::t);
    let win = egui::Window::new(title.clone())
        .id(egui::Id::new("test_movie"))
        .open(&mut shown)
        .collapsible(false)
        .resizable(true)
        .default_size([default_w, default_h])
        .default_pos(ctx.content_rect().center() - egui::vec2(default_w * 0.5 + 120.0, default_h * 0.5))
        .min_size([320.0, 260.0])
        .show(ctx, |ui| restart = body(tm, plugins, out, ui, note));
    crate::a11y::name_title_bar(&win, &title);
    // Esc closes it from anywhere in the editor, as it always has; Ctrl+W (⌘W) while the movie
    // has the keyboard.
    let esc = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) || (has_keys(ctx) && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::W)));
    Asked { restart, close: !shown || esc, ..Default::default() }
}

/// The window's contents, wherever it is: the controls, the movie under the pointer and the
/// keyboard, and the Output. Returns whether Restart was clicked.
fn body(tm: &mut TestMovie, plugins: &anicraft_engine::plugins::Registry, out: &mut crate::sound::SoundOut, ui: &mut egui::Ui, note: Option<&str>) -> bool {
    let ctx = ui.ctx().clone();
    let mut restart = false;
    let (sw, sh) = (tm.doc.stage.width as f32, tm.doc.stage.height as f32);
    // ---- controls ----
    ui.horizontal(|ui| {
        if crate::timeline::icon_button(ui, &tr!("Restart from frame 1 with the document as it is now ({key})", key = crate::platform::shortcut_label("Ctrl+Enter")), None, Icon::First).clicked() {
            restart = true;
        }
        let (icon, tip) = if tm.paused { (Icon::Play, tr!("Play")) } else { (Icon::Pause, tr!("Pause")) };
        if crate::timeline::icon_button(ui, tip, None, icon).clicked() {
            tm.paused = !tm.paused;
            tm.acc = 0.0;
            if tm.paused {
                out.stop();
            } else if tm.audio {
                tm.resume(out);
            }
        }
        if tm.paused && crate::timeline::icon_button(ui, tr!("Step one frame"), None, Icon::Next).clicked() {
            let mut runner = ScriptRunner::new(plugins);
            tm.movie.step(&tm.doc, &[], &mut runner);
            tm.feed(out);
        }
        ui.add_space(8.0);
        let st = &tm.movie.state;
        let scene = tm.doc.scenes.get(st.scene).map_or("", |s| s.name.as_str());
        let frames = tm.doc.timeline(st.scene).frame_count();
        ui.label(text(Type::Body, scene).color(T.text_dim));
        ui.label(text(Type::Data, format!("{:>4} / {frames}", st.root.frame + 1))).on_hover_text(tr!("Main timeline frame"));
        let what = if tm.paused { tr!("paused") } else if st.root.playing { tr!("playing") } else { tr!("stopped") };
        ui.label(text(Type::Caption, what));
        // A movie that listens for keys says where they go.
        if tm.movie.state.stage.iter().any(|l| l.event.is_key()) {
            let keys = if has_keys(&ctx) { tr!("Keys go to the movie") } else { tr!("Click the movie to give it the keyboard") };
            ui.label(text(Type::Caption, keys).color(T.text_dim));
        }
        ui.add_space(8.0);
        ui.checkbox(&mut tm.hit_areas, tr!("Show hit areas")).on_hover_text(tr!("Draw where each button answers the pointer (its Hit frame) over the movie"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // i18n-exempt: numbers and units (data)
            ui.label(text(Type::Data, format!("{:.0} fps · {:.1} ms", tm.doc.stage.fps, tm.ms)).color(T.text_faint)).on_hover_text(tr!("Frame rate, and the time the last picture took to draw"));
        });
    });
    // Why it isn't in a window of its own, when that isn't the user's choice.
    if let Some(note) = note {
        ui.label(text(Type::Caption, note).color(T.text_dim));
    }
    ui.add_space(4.0);
    // ---- the movie, fitted to the window, with the pointer ----
    let output_h = 120.0f32.min(ui.available_height() * 0.4);
    let avail = egui::vec2(ui.available_width(), (ui.available_height() - output_h - 40.0).max(60.0));
    let k = (avail.x / sw.max(1.0)).min(avail.y / sh.max(1.0)).max(0.01);
    let size = egui::vec2(sw * k, sh * k);
    let (outer, _) = ui.allocate_exact_size(egui::vec2(avail.x, size.y), egui::Sense::hover());
    let rect = egui::Rect::from_center_size(outer.center(), size);
    // The text fields, for screen readers: made before the movie surface so the surface keeps
    // the pointer (ADR 0066).
    field_nodes(ui, tm, rect, k);
    let resp = ui.interact(outer, surface_id(), egui::Sense::click_and_drag());
    crate::a11y::surface(ui, &resp, tr!("The movie: point, click and type to play it"));
    if tm.focus || resp.is_pointer_button_down_on() {
        resp.request_focus();
        tm.focus = false;
    }
    // Tab and the arrows are the movie's keys while it has them, not focus moves.
    if resp.has_focus() {
        ui.memory_mut(|m| m.set_focus_lock_filter(resp.id, egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: false }));
    }
    // The pointer, in stage coordinates, to the player.
    let mut events = Vec::new();
    let to_stage = |p: egui::Pos2| Vec2::new((p.x - rect.left()) / k, (p.y - rect.top()) / k);
    let latest = ui.input(|i| i.pointer.latest_pos());
    let inside = resp.hover_pos().is_some_and(|p| rect.contains(p));
    let down = resp.is_pointer_button_down_on();
    if let Some(p) = latest.map(to_stage) {
        if inside && tm.pointer != Some(p) {
            events.push(Event::Move(p));
        }
        if !inside && tm.pointer.is_some() {
            events.push(Event::Leave);
        }
        if down && !tm.down && inside {
            events.push(Event::Down(p));
        }
        // Released off the movie: an Up out there, which clicks nothing.
        if !down && tm.down {
            events.push(Event::Up(p));
        }
        tm.pointer = inside.then_some(p);
    }
    tm.down = down && (tm.down || inside);
    // The keyboard, while the movie has it (Ctrl/⌘ shortcuts stay the app's, except Select All,
    // Copy and Paste while a text field has the keyboard).
    if resp.has_focus() {
        let field = tm.movie.state.focus.is_some();
        let shift = ui.input(|i| i.modifiers.shift);
        if shift != tm.shift {
            tm.shift = shift;
            events.push(if shift { Event::KeyDown(16) } else { Event::KeyUp(16) });
        }
        if field {
            let select_all = ui.input_mut(|i| {
                // The app's own Copy and Paste would act on the stage instead.
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::C);
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::V);
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::A)
            });
            if select_all {
                events.push(Event::SelectAll);
            }
        }
        let mut copy = false;
        ui.input(|i| {
            for e in &i.events {
                match e {
                    egui::Event::Key { key, pressed, modifiers, .. } if !modifiers.command && *key != egui::Key::Escape => {
                        if let Some(code) = key_code(*key) {
                            events.push(if *pressed { Event::KeyDown(code) } else { Event::KeyUp(code) });
                        }
                    }
                    // Typing, pasting and input methods go into the field with the keyboard.
                    egui::Event::Text(t) | egui::Event::Paste(t) | egui::Event::Ime(egui::ImeEvent::Commit(t)) => events.extend(t.chars().take(anicraft_engine::player::MAX_TYPED).map(Event::Char)),
                    egui::Event::Copy => copy = field,
                    _ => {}
                }
            }
        });
        if copy && let Some(text) = selected_text(tm) {
            ctx.copy_text(text);
        }
    }
    let mut runner = ScriptRunner::new(plugins);
    tm.movie.input(&tm.doc, &events, &mut runner);
    tm.feed(out);
    if inside && tm.movie.state.hover.is_some() {
        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
    } else if inside && let Some(p) = tm.pointer
        && anicraft_engine::doc::player::field_at(&tm.doc, &tm.movie.state, p).is_some()
    {
        ctx.set_cursor_icon(egui::CursorIcon::Text);
    }
    // The system's input methods compose where the focused field is.
    if resp.has_focus()
        && let Some(r) = focused_rect(tm, rect, k)
    {
        let password = focused_block(tm).and_then(|t| t.field.as_ref().map(|f| f.lines == anicraft_engine::doc::LineType::Password)).unwrap_or(false);
        let purpose = if password { egui::IMEPurpose::Password } else { egui::IMEPurpose::Normal };
        ctx.output_mut(|o| o.ime = Some(egui::output::IMEOutput { purpose, rect: r, cursor_rect: egui::Rect::from_min_size(r.left_bottom(), egui::vec2(1.0, 1.0)), should_interrupt_composition: false }));
    }
    // ---- time: the document's frame rate, kept by the sound while it plays ----
    if !tm.paused {
        let fps = f64::from(tm.doc.stage.fps.max(1.0));
        let rate = f64::from(out.rate().max(1));
        let heard = if tm.audio { out.movie_position() } else { None };
        let mut n = 0;
        if let Some(pos) = heard {
            let due = (pos as f64 * fps / rate).floor() as u64;
            while tm.movie.state.tick < due && n < MAX_CATCH_UP {
                tm.movie.step(&tm.doc, &[], &mut runner);
                n += 1;
            }
        } else {
            tm.acc += f64::from(ui.input(|i| i.stable_dt)).min(0.25);
            while tm.acc >= 1.0 / fps && n < MAX_CATCH_UP {
                tm.acc -= 1.0 / fps;
                tm.movie.step(&tm.doc, &[], &mut runner);
                n += 1;
            }
            if n == MAX_CATCH_UP {
                tm.acc = 0.0;
            }
        }
        tm.feed(out);
        ctx.request_repaint();
    }
    // ---- picture ----
    let ppp = ctx.pixels_per_point();
    let scale = (k * ppp).min(MAX_EDGE / sw.max(sh).max(1.0));
    let px = [((sw * scale).round() as u32).max(1), ((sh * scale).round() as u32).max(1)];
    let stale = tm.shown.as_ref().is_none_or(|(st, at, hits)| *st != tm.movie.state || *at != px || *hits != tm.hit_areas);
    if stale {
        let t0 = web_time::Instant::now();
        let mut dl = evaluate_player(&tm.doc, &tm.movie.state);
        if tm.hit_areas {
            let (tint, alpha) = crate::theme::hit_area();
            dl.append(anicraft_engine::doc::player::hit_overlay(&tm.doc, &tm.movie.state, tint, alpha));
        }
        let view = View { transform: Affine::scale(scale, scale), width: px[0], height: px[1], background: Some(tm.doc.stage.background) };
        if let Ok((pm, _)) = render(&dl, &view) {
            let image = egui::ColorImage::from_rgba_unmultiplied([px[0] as usize, px[1] as usize], &pm.to_rgba8());
            match &mut tm.texture {
                Some(t) => t.set(image, egui::TextureOptions::LINEAR),
                None => tm.texture = Some(ctx.load_texture("test-movie", image, egui::TextureOptions::LINEAR)),
            }
        }
        tm.ms = t0.elapsed().as_secs_f32() * 1000.0;
        tm.shown = Some((tm.movie.state.clone(), px, tm.hit_areas));
    }
    ui.painter().rect_filled(outer, crate::theme::SQUARE, T.pasteboard);
    if let Some(t) = &tm.texture {
        ui.painter().image(t.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
    }
    // ---- output ----
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(text(Type::Heading, tr!("Output")));
        let n = tm.movie.output.len();
        ui.label(text(Type::Data, format!("{n}")).color(T.text_faint)).on_hover_text(tr!("Lines written"));
        // Kept scripts that don't run (ADR 0013) are counted once here, not every
        // time they pass; the Output's first lines say which and why.
        if tm.skipped > 0 {
            let what = trn!("{n} frame script doesn't run", "{n} frame scripts don't run", tm.skipped);
            ui.label(text(Type::Caption, what).color(T.error)).on_hover_text(tr!("Kept with the document, but outside the ActionScript 3.0 subset AniCraft runs (or on a timeline that doesn't run scripts). The first Output lines say why; the Actions panel shows each problem in place."));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add_enabled(n > 0, egui::Button::new(tr!("Clear"))).clicked() {
                tm.movie.output.clear();
            }
        });
    });
    egui::Frame::NONE.fill(T.field).inner_margin(egui::Margin::symmetric(8, 6)).show(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("test_movie_output").max_height(output_h).auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            if tm.movie.output.is_empty() {
                ui.label(text(Type::Caption, tr!("Trace actions, frame-script logs and anything that couldn't run appear here.")));
            }
            for (tick, line) in &tm.movie.output {
                ui.horizontal(|ui| {
                    ui.label(text(Type::DataSmall, format!("{tick:>5}")).color(T.text_faint));
                    ui.label(text(Type::Data, line.as_str()));
                });
            }
        });
    });
    restart
}

/// `--open testtext` (screenshots): a name typed into the name field and Enter pressed (the
/// script greets it), then the notes field clicked and a word selected with Shift; paused there.
pub fn demo_typing(app: &mut App) {
    let plugins = &app.s.plugins;
    let Some(tm) = app.test_movie.as_mut() else { return };
    let mut r = ScriptRunner::new(plugins);
    let name = Vec2::new(60.0, 147.0);
    let mut ev = vec![Event::Move(name), Event::Down(name), Event::Up(name)];
    // i18n-exempt: what the viewer types, document content
    ev.extend("Ada Lovelace".chars().map(Event::Char));
    ev.extend([Event::KeyDown(13), Event::KeyUp(13)]);
    let notes = Vec2::new(380.0, 228.0);
    ev.extend([Event::Down(notes), Event::Up(notes), Event::KeyDown(16), Event::KeyDown(37), Event::KeyDown(37), Event::KeyDown(37), Event::KeyDown(37), Event::KeyUp(16)]);
    tm.movie.input(&tm.doc, &ev, &mut r);
    tm.paused = true;
}

/// The block of the field that has the keyboard.
fn focused_block(tm: &TestMovie) -> Option<Arc<anicraft_engine::doc::TextBlock>> {
    let path = tm.movie.state.focus.as_ref()?;
    anicraft_engine::doc::player::tree(&tm.doc, &tm.movie.state).into_iter().find(|n| &n.path == path)?.text
}

/// The focused field's selected text (Copy).
fn selected_text(tm: &TestMovie) -> Option<String> {
    let t = focused_block(tm)?;
    let path = tm.movie.state.focus.as_ref()?;
    // A password's text is never copied.
    if t.field.as_ref().is_some_and(|f| f.lines == anicraft_engine::doc::LineType::Password) {
        return None;
    }
    let s = tm.movie.state.fields.get(path).map(|f| f.selected(&t)).unwrap_or_default();
    (!s.is_empty()).then_some(s)
}

/// A field's box on screen, for the picture at `rect` drawn `k` points per stage pixel.
fn screen_rect(n: &anicraft_engine::doc::player::Node, t: &anicraft_engine::doc::TextBlock, rect: egui::Rect, k: f32) -> egui::Rect {
    let b = anicraft_engine::geom::Rect::new(0.0, 0.0, t.box_width, t.box_height).transformed(&n.xf);
    egui::Rect::from_min_max(rect.min + egui::vec2(b.min.x, b.min.y) * k, rect.min + egui::vec2(b.max.x, b.max.y) * k)
}

/// The focused field's box on screen.
fn focused_rect(tm: &TestMovie, rect: egui::Rect, k: f32) -> Option<egui::Rect> {
    let path = tm.movie.state.focus.as_ref()?;
    let nodes = anicraft_engine::doc::player::tree(&tm.doc, &tm.movie.state);
    let n = nodes.iter().find(|n| &n.path == path)?;
    let (t, _) = n.field()?;
    Some(screen_rect(n, t, rect, k))
}

/// Each text field on the stage as a node screen readers find: an input field as a text input
/// (a password as a password input, a multiline one as a multiline input) and a dynamic field as
/// a label, named by its instance name, with its text as its value (ADR 0066).
fn field_nodes(ui: &mut egui::Ui, tm: &TestMovie, rect: egui::Rect, k: f32) {
    use anicraft_engine::doc::{LineType, TextKind};
    use egui::accesskit::Role;
    let st = &tm.movie.state;
    for n in anicraft_engine::doc::player::tree(&tm.doc, st).iter().filter(|n| n.hittable).take(256) {
        let Some((t, f)) = n.field() else { continue };
        let r = screen_rect(n, t, rect, k).intersect(rect);
        if !r.is_positive() {
            continue;
        }
        let id = surface_id().with(("field", &n.path));
        ui.interact(r, id, egui::Sense::hover());
        let value = st.fields.get(&n.path).map_or(t.text.as_str(), |x| x.value(t));
        let shown = f.shown(value);
        let role = match (f.kind, f.lines) {
            (TextKind::Input, LineType::Password) => Role::PasswordInput,
            (TextKind::Input, LineType::Multiline | LineType::MultilineNoWrap) => Role::MultilineTextInput,
            (TextKind::Input, _) => Role::TextInput,
            _ => Role::Label,
        };
        let name = if n.name.is_empty() { tr!("Text field").to_string() } else { n.name.clone() };
        ui.ctx().accesskit_node_builder(id, |node| {
            node.set_role(role);
            node.set_label(name.as_str());
            node.set_value(shown.as_str());
            if st.focus.as_ref() == Some(&n.path) {
                node.set_selected(true);
            }
        });
    }
}

/// `--open testmovie` (screenshots): the sample's Play button clicked, the show played through
/// and back on the menu, the pointer resting on the button (its Over state); paused there.
pub fn demo_pointer(app: &mut App) {
    let plugins = &app.s.plugins;
    let Some(tm) = app.test_movie.as_mut() else { return };
    let mut r = ScriptRunner::new(plugins);
    let at = Vec2::new(640.0, 400.0);
    tm.movie.input(&tm.doc, &[Event::Move(at), Event::Down(at), Event::Up(at), Event::Leave], &mut r);
    for _ in 0..40 {
        tm.movie.step(&tm.doc, &[], &mut r);
    }
    tm.movie.input(&tm.doc, &[Event::Move(Vec2::new(700.0, 410.0))], &mut r);
    tm.paused = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn frame(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) {
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 940.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| app.frame(ui));
        out.textures_delta.clear();
    }

    fn key(key: egui::Key, pressed: bool) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE }
    }

    fn lines(app: &App) -> Vec<String> {
        app.test_movie.as_ref().map(|t| t.movie.output.iter().map(|(_, l)| l.clone()).collect()).unwrap_or_default()
    }

    #[test]
    fn keys_go_to_the_movie_while_it_has_them() {
        let ctx = egui::Context::default();
        let session = anicraft_engine::Session::new(anicraft_engine::doc::sample::interactive());
        let mut app = App::with_context(&ctx, session, Some(std::path::PathBuf::from("unused.png")));
        app.run("layer.select", json!({"index": 0}));
        // i18n-exempt: script source, document content
        let script = "stop();\nstage.addEventListener(KeyboardEvent.KEY_DOWN, function (e:KeyboardEvent):void { trace(\"down \" + e.keyCode); });\nstage.addEventListener(KeyboardEvent.KEY_UP, function (e:KeyboardEvent):void { trace(\"up \" + e.keyCode); });";
        assert!(app.run("script.set", json!({"frame": 0, "text": script})).is_some_and(|v| v["runs"] == true));
        open(&mut app);
        for _ in 0..3 {
            frame(&ctx, &mut app, vec![]);
        }
        assert!(has_keys(&ctx), "the movie takes the keyboard when Test Movie opens");
        let tool = app.tool;
        // A letter that is a tool shortcut on the stage goes to the movie instead.
        frame(&ctx, &mut app, vec![key(egui::Key::R, true)]);
        frame(&ctx, &mut app, vec![key(egui::Key::R, false), key(egui::Key::ArrowLeft, true), key(egui::Key::Space, true)]);
        assert_eq!(app.tool, tool, "R didn't change the tool");
        let out = lines(&app);
        for want in ["down 82", "up 82", "down 37", "down 32"] {
            assert!(out.iter().any(|l| l == want), "{want} not in {out:?}");
        }
        assert!(has_keys(&ctx), "the arrows didn't move the focus away");
        // Esc still closes the window.
        frame(&ctx, &mut app, vec![key(egui::Key::Escape, true)]);
        assert!(app.test_movie.is_none());
    }

    #[test]
    fn an_open_button_names_its_states_widens_its_frames_and_shows_its_hit_area() {
        let ctx = egui::Context::default();
        let session = anicraft_engine::Session::new(anicraft_engine::doc::sample::interactive());
        let mut app = App::with_context(&ctx, session, Some(std::path::PathBuf::from("unused.png")));
        frame(&ctx, &mut app, vec![]);
        let zoom = app.tl_cell_w;
        assert!(crate::timeline::button_states(&app).is_none(), "on the stage, frames are numbered");
        let button = app.s.doc.library.iter().find(|s| s.kind == anicraft_engine::doc::SymbolKind::Button).map(|s| s.id.0);
        app.run("symbol.edit", json!({"symbol": button}));
        app.run("timeline.goto", json!({"frame": 1}));
        frame(&ctx, &mut app, vec![]);
        assert_eq!(crate::timeline::button_states(&app).map(|s| s.len()), Some(4));
        assert!(app.tl_cell_w > zoom, "wide enough for “Over”: {} vs {zoom}", app.tl_cell_w);
        assert!(app.s.hit_overlay().is_some(), "the Hit area over the Over state, in the theme's colour");
        app.run("symbol.exit", json!({}));
        frame(&ctx, &mut app, vec![]);
        assert_eq!(app.tl_cell_w, zoom, "the zoom it had comes back");
        // Test Movie keeps Show hit areas across a restart.
        open(&mut app);
        if let Some(tm) = app.test_movie.as_mut() {
            tm.hit_areas = true;
        }
        open(&mut app);
        assert!(app.test_movie.as_ref().is_some_and(|t| t.hit_areas));
        frame(&ctx, &mut app, vec![]);
    }

    fn ctrl(key: egui::Key) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND }
    }

    /// The window of its own (ADR 0062), with egui standing in for the window system: it opens
    /// with the keyboard, its keys are the movie's, Ctrl+Enter restarts it, Ctrl+W and Esc
    /// close it, and the preference puts it back inside the editor.
    #[test]
    fn the_window_of_its_own_has_the_keyboard_and_closes_with_esc_or_ctrl_w() {
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let session = anicraft_engine::Session::new(anicraft_engine::doc::sample::interactive());
        let mut app = App::with_context(&ctx, session, Some(std::path::PathBuf::from("unused.png")));
        app.run("layer.select", json!({"index": 0}));
        // i18n-exempt: script source, document content
        let script = "stop();\nstage.addEventListener(KeyboardEvent.KEY_DOWN, function (e:KeyboardEvent):void { trace(\"down \" + e.keyCode); });";
        assert!(app.run("script.set", json!({"frame": 0, "text": script})).is_some_and(|v| v["runs"] == true));
        assert_eq!(crate::platform::movie_host(ctx.embed_viewports(), app.prefs.test_movie_window), crate::platform::MovieHost::Window);
        open(&mut app);
        for _ in 0..3 {
            frame(&ctx, &mut app, vec![]);
        }
        assert!(has_keys(&ctx), "the movie takes the keyboard when its window opens");
        let place = app.test_movie.as_ref().and_then(|t| t.placement);
        frame(&ctx, &mut app, vec![key(egui::Key::A, true)]);
        assert!(lines(&app).iter().any(|l| l == "down 65"), "{:?}", lines(&app));
        // Ctrl+Enter (⌘↩) in the movie's window restarts it, in the same place.
        frame(&ctx, &mut app, vec![ctrl(egui::Key::Enter)]);
        assert!(app.test_movie.as_ref().is_some_and(|t| t.movie.output.iter().all(|(_, l)| l != "down 65")), "a fresh run");
        assert_eq!(app.test_movie.as_ref().and_then(|t| t.placement).map(|p| p.size), place.map(|p| p.size), "a restart keeps the window as it is");
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![ctrl(egui::Key::W)]);
        assert!(app.test_movie.is_none(), "Ctrl+W closes it");
        open(&mut app);
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![key(egui::Key::Escape, true)]);
        assert!(app.test_movie.is_none(), "Esc closes it");
        // Inside the editor by choice: the floating window, Ctrl+W there too.
        app.prefs.test_movie_window = false;
        open(&mut app);
        for _ in 0..2 {
            frame(&ctx, &mut app, vec![]);
        }
        assert!(ctx.memory(|m| m.area_rect(egui::Id::new("test_movie")).is_some()), "the floating window");
        frame(&ctx, &mut app, vec![ctrl(egui::Key::W)]);
        assert!(app.test_movie.is_none());
    }

    #[test]
    fn the_window_opens_at_the_stage_size_and_fits_the_screen() {
        let chrome = CHROME;
        // A 1280 × 720 stage at 100 % on a large screen, as in Animate.
        let big = window_size(egui::vec2(1280.0, 720.0), Some(egui::vec2(2560.0, 1440.0)));
        assert_eq!(big, egui::vec2(1280.0, 720.0) + chrome);
        // On a small screen it shrinks to fit, keeping the movie's shape.
        let small = window_size(egui::vec2(1280.0, 720.0), Some(egui::vec2(1366.0, 768.0)));
        assert!(small.y <= 768.0 * 0.85 + 0.5 && small.x < big.x, "{small:?}");
        let (mw, mh) = (small.x - chrome.x, small.y - chrome.y);
        assert!((mw / mh - 1280.0 / 720.0).abs() < 0.01);
        // Tiny or broken stages still get usable controls.
        assert_eq!(window_size(egui::vec2(10.0, 0.0), None), egui::vec2(MIN_SIZE[0], MIN_SIZE[1]));
        assert!(window_size(egui::vec2(f32::MAX, 1.0), Some(egui::vec2(800.0, 600.0))).x.is_finite());
    }

    #[test]
    fn key_codes_are_actionscript_s() {
        use egui::Key as K;
        for (k, code) in [(K::A, 65), (K::Z, 90), (K::Num0, 48), (K::Num9, 57), (K::ArrowLeft, 37), (K::ArrowUp, 38), (K::ArrowRight, 39), (K::ArrowDown, 40), (K::Space, 32), (K::Enter, 13), (K::Tab, 9), (K::Backspace, 8), (K::F1, 112), (K::F12, 123), (K::Comma, 188)] {
            assert_eq!(key_code(k), Some(code), "{k:?}");
            assert!(anicraft_engine::doc::actions::as3::ir::key_code(match k {
                K::ArrowLeft => "LEFT",
                K::Space => "SPACE",
                K::A => "A",
                K::Num0 => "NUMBER_0",
                _ => "F1",
            })
            .is_some());
        }
        assert_eq!(anicraft_engine::doc::actions::as3::ir::key_code("LEFT"), key_code(K::ArrowLeft));
        assert_eq!(anicraft_engine::doc::actions::as3::ir::key_code("NUMBER_7"), key_code(K::Num7));
    }
}
