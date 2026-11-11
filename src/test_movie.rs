//! Test Movie (Control › Test Movie, Ctrl+Enter, as in Animate; ADR 0024): the movie played by
//! the interactive player in a window of its own, at the document's frame rate, with its
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
//! and Esc closes the window.

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
    /// Show Hit Areas: every button's hit area drawn over the movie (ADR 0057).
    pub hit_areas: bool,
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
    // Show Hit Areas stays as it was across a restart.
    let hit_areas = app.test_movie.as_ref().is_some_and(|t| t.hit_areas);
    let mut tm = TestMovie { doc, movie, paused: false, acc: 0.0, texture: None, shown: None, pointer: None, down: false, name, ms: 0.0, voices: Vec::new(), audio: false, skipped, focus: true, hit_areas };
    // Sound only when there is some, and never in screenshot runs or tests.
    if sound::has_sounds(&tm.doc) && app.shot.is_none() {
        tm.resume(&mut app.sound);
    }
    tm.feed(&mut app.sound);
    app.test_movie = Some(tm);
}

/// The Test Movie window, when open.
pub fn show(app: &mut App, ctx: &egui::Context) {
    let plugins = &app.s.plugins;
    let out = &mut app.sound;
    let Some(tm) = app.test_movie.as_mut() else { return };
    let mut shown = true;
    let mut restart = false;
    let (sw, sh) = (tm.doc.stage.width as f32, tm.doc.stage.height as f32);
    let default_w = 760.0f32;
    let default_h = default_w * sh / sw.max(1.0) + 190.0;
    let title = tr!("Test Movie — {name}", name = tm.name.as_deref().unwrap_or(tr!("Untitled")));
    let win = egui::Window::new(title.clone())
        .id(egui::Id::new("test_movie"))
        .open(&mut shown)
        .collapsible(false)
        .resizable(true)
        .default_size([default_w, default_h])
        .default_pos(ctx.content_rect().center() - egui::vec2(default_w * 0.5 + 120.0, default_h * 0.5))
        .min_size([320.0, 260.0])
        .show(ctx, |ui| {
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
                    let keys = if has_keys(ctx) { tr!("Keys go to the movie") } else { tr!("Click the movie to give it the keyboard") };
                    ui.label(text(Type::Caption, keys).color(T.text_dim));
                }
                ui.add_space(8.0);
                ui.checkbox(&mut tm.hit_areas, tr!("Show hit areas")).on_hover_text(tr!("Draw where each button answers the pointer (its Hit frame) over the movie"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // i18n-exempt: numbers and units (data)
                    ui.label(text(Type::Data, format!("{:.0} fps · {:.1} ms", tm.doc.stage.fps, tm.ms)).color(T.text_faint)).on_hover_text(tr!("Frame rate, and the time the last picture took to draw"));
                });
            });
            ui.add_space(4.0);
            // ---- the movie, fitted to the window, with the pointer ----
            let output_h = 120.0f32.min(ui.available_height() * 0.4);
            let avail = egui::vec2(ui.available_width(), (ui.available_height() - output_h - 40.0).max(60.0));
            let k = (avail.x / sw.max(1.0)).min(avail.y / sh.max(1.0)).max(0.01);
            let size = egui::vec2(sw * k, sh * k);
            let (outer, _) = ui.allocate_exact_size(egui::vec2(avail.x, size.y), egui::Sense::hover());
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
            let rect = egui::Rect::from_center_size(outer.center(), size);
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
            // The keyboard, while the movie has it (Ctrl/⌘ shortcuts stay the app's).
            if resp.has_focus() {
                ui.input(|i| {
                    for e in &i.events {
                        if let egui::Event::Key { key, pressed, modifiers, .. } = e
                            && !modifiers.command
                            && *key != egui::Key::Escape
                            && let Some(code) = key_code(*key)
                        {
                            events.push(if *pressed { Event::KeyDown(code) } else { Event::KeyUp(code) });
                        }
                    }
                });
            }
            let mut runner = ScriptRunner::new(plugins);
            tm.movie.input(&tm.doc, &events, &mut runner);
            tm.feed(out);
            if inside && tm.movie.state.hover.is_some() {
                ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
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
        });
    crate::a11y::name_title_bar(&win, &title);
    let esc = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    if !shown || esc {
        if app.test_movie.take().is_some_and(|tm| tm.audio) {
            app.sound.stop();
        }
    } else if restart {
        open(app);
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
