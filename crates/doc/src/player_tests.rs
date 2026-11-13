use super::*;
use crate::actions::{Action, ButtonEvent, EventHandler, FrameRef, FrameScript, PluginScript, Property, Target};
use crate::{ColorEffect, Keyframe, Layer, LayerId, Paint, Rgba, Shape, Symbol, SymbolId, SymbolKind, evaluate, evaluate_player, sample};
use anicraft_geom::Path;
use std::sync::Arc;

fn rect(id: u64, x: f32, y: f32, w: f32, h: f32) -> Arc<Element> {
    Arc::new(Element { id: ElementId(id), name: String::new(), transform: Affine::IDENTITY, color: ColorEffect::default(), kind: ElementKind::Shape(Shape::filled(Path::rect(x, y, w, h), Paint::Solid(Rgba::BLACK))) })
}

fn inst(id: u64, symbol: u64, name: &str, x: f32, y: f32, events: Vec<EventHandler>) -> Arc<Element> {
    Arc::new(Element {
        id: ElementId(id),
        name: name.into(),
        transform: Affine::translate(x, y),
        color: ColorEffect::default(),
        kind: ElementKind::Symbol { symbol: SymbolId(symbol), loop_mode: Default::default(), first_frame: 0, blend: Default::default(), filters: Vec::new(), events },
    })
}

fn kf(start: u32, duration: u32, elements: Vec<Arc<Element>>, actions: Vec<Action>, label: &str) -> Arc<Keyframe> {
    let mut k = Keyframe::new(start, duration);
    k.elements = elements;
    k.label = label.into();
    k.script = (!actions.is_empty()).then_some(FrameScript::Actions(actions));
    Arc::new(k)
}

fn layer(id: u64, keys: Vec<Arc<Keyframe>>) -> Arc<Layer> {
    let mut l = Layer::new(LayerId(id), format!("L{id}"));
    l.keyframes = keys;
    Arc::new(l)
}

fn symbol(id: u64, kind: SymbolKind, layers: Vec<Arc<Layer>>) -> Arc<Symbol> {
    Arc::new(Symbol { id: SymbolId(id), name: format!("S{id}"), kind, linkage: None, scale_grid_kept: None, scale_grid: None, timeline: Timeline { layers } })
}

fn doc(root: Vec<Arc<Layer>>, library: Vec<Arc<Symbol>>) -> Document {
    let mut d = Document::default();
    d.timeline_mut(0).layers = root;
    d.library = library;
    d.next_id = 10_000;
    d
}

fn stop() -> Action {
    Action::Stop { target: Target::this() }
}

fn goto_stop(target: &str, frame: FrameRef) -> Action {
    Action::GotoAndStop { target: Target::new(target), frame, scene: None }
}

fn run(d: &Document, steps: usize) -> Step {
    let mut s = start(d, &mut NoScripts);
    for _ in 0..steps {
        s = step(d, &s.state, &[], &mut NoScripts);
    }
    s
}

#[test]
fn a_stop_action_holds_the_playhead_on_its_frame() {
    let d = doc(vec![layer(1, vec![kf(0, 3, vec![], vec![], ""), kf(3, 7, vec![], vec![stop()], "")]), layer(2, vec![kf(0, 10, vec![rect(5, 0.0, 0.0, 10.0, 10.0)], vec![], "")])], vec![]);
    let s = start(&d, &mut NoScripts);
    assert_eq!((s.state.root.frame, s.state.root.playing), (0, true));
    let s = run(&d, 6);
    assert_eq!((s.state.root.frame, s.state.root.playing), (3, false), "stops on the frame with the action");
}

#[test]
fn goto_a_label_jumps_and_unknown_labels_are_reported() {
    let jump = Action::GotoAndPlay { target: Target::this(), frame: FrameRef::Label("loop".into()), scene: None };
    let d = doc(vec![layer(1, vec![kf(0, 1, vec![], vec![jump], ""), kf(1, 4, vec![], vec![], ""), kf(5, 5, vec![], vec![], "loop")])], vec![]);
    let s = start(&d, &mut NoScripts);
    assert_eq!((s.state.root.frame, s.state.root.playing), (5, true));
    let missing = doc(vec![layer(1, vec![kf(0, 4, vec![], vec![goto_stop("", FrameRef::Label("nowhere".into()))], "")])], vec![]);
    let s = start(&missing, &mut NoScripts);
    assert_eq!(s.state.root.frame, 0);
    assert!(s.output.iter().any(|l| l.contains("no frame labelled “nowhere”")), "{:?}", s.output);
}

/// Root (10 frames) → `hero` (movie clip, 6 frames, "run" at 3) → `arm` (movie clip, 4 frames;
/// its frame 2 stops the main timeline through `parent.parent`).
fn nested() -> Document {
    let arm = symbol(200, SymbolKind::MovieClip, vec![layer(21, vec![kf(0, 2, vec![rect(22, 0.0, 0.0, 5.0, 5.0)], vec![], ""), kf(2, 2, vec![rect(22, 0.0, 0.0, 5.0, 5.0)], vec![Action::Stop { target: Target::new("parent.parent") }], "")])]);
    let hero = symbol(100, SymbolKind::MovieClip, vec![layer(11, vec![kf(0, 3, vec![inst(12, 200, "arm", 5.0, 0.0, vec![])], vec![], ""), kf(3, 3, vec![inst(12, 200, "arm", 5.0, 0.0, vec![])], vec![], "run")])]);
    let root_actions = vec![goto_stop("hero", FrameRef::Label("run".into())), goto_stop("hero.arm", FrameRef::Number(2))];
    doc(vec![layer(1, vec![kf(0, 1, vec![], root_actions, ""), kf(1, 9, vec![], vec![], "")]), layer(2, vec![kf(0, 10, vec![inst(3, 100, "hero", 50.0, 50.0, vec![])], vec![], "")])], vec![hero, arm])
}

#[test]
fn actions_control_nested_movie_clips_by_instance_name_path() {
    let d = nested();
    let s = start(&d, &mut NoScripts);
    let hero = s.state.clips[&vec![ElementId(3)]];
    let arm = s.state.clips[&vec![ElementId(3), ElementId(12)]];
    assert_eq!((hero.frame, hero.playing), (3, false), "hero went to “run” and stopped");
    assert_eq!((arm.frame, arm.playing), (2, false), "arm went to frame 3 and stopped");
    // Entering arm's frame 2 ran its action, which stopped the main timeline.
    assert!(!s.state.root.playing);
    let later = run(&d, 5);
    assert_eq!(later.state, s.state.clone().with_tick(5), "nothing plays any more");
}

impl PlayerState {
    fn with_tick(mut self, t: u64) -> Self {
        self.tick = t;
        self
    }
}

#[test]
fn movie_clips_play_on_their_own_and_restart_when_they_reappear() {
    // hero is on the stage for frames 0-4, gone at 5, back at 6.
    let hero = symbol(100, SymbolKind::MovieClip, vec![layer(11, vec![kf(0, 4, vec![rect(12, 0.0, 0.0, 5.0, 5.0)], vec![], "")])]);
    let d = doc(vec![layer(1, vec![kf(0, 5, vec![inst(3, 100, "hero", 0.0, 0.0, vec![])], vec![], ""), kf(5, 1, vec![], vec![], ""), kf(6, 4, vec![inst(3, 100, "hero", 0.0, 0.0, vec![])], vec![], "")])], vec![hero]);
    let frame = |s: &Step| s.state.clips.get(&vec![ElementId(3)]).map(|c| c.frame);
    assert_eq!(frame(&run(&d, 3)), Some(3));
    assert_eq!(frame(&run(&d, 4)), Some(0), "loops round its 4 frames");
    assert_eq!(frame(&run(&d, 5)), None, "gone");
    assert_eq!(frame(&run(&d, 6)), Some(0), "back from its first frame");
}

/// A button `go` at (100, 100): its art is 10 × 10, its Hit frame 100 × 100. Click goes to
/// frame 5 and stops; Over and Out trace.
fn button_doc() -> Document {
    let art = |id| vec![rect(id, 0.0, 0.0, 10.0, 10.0)];
    let b = symbol(300, SymbolKind::Button, vec![layer(31, vec![kf(0, 1, art(32), vec![], ""), kf(1, 1, art(32), vec![], ""), kf(2, 1, art(32), vec![], ""), kf(3, 1, vec![rect(33, 0.0, 0.0, 100.0, 100.0)], vec![], "")])]);
    let events = vec![
        EventHandler { event: ButtonEvent::Click, actions: vec![goto_stop("", FrameRef::Number(4))] },
        EventHandler { event: ButtonEvent::Over, actions: vec![Action::Trace { message: "over".into() }] },
        EventHandler { event: ButtonEvent::Out, actions: vec![Action::Trace { message: "out".into() }] },
    ];
    doc(vec![layer(1, vec![kf(0, 10, vec![], vec![stop()], "")]), layer(2, vec![kf(0, 10, vec![inst(3, 300, "go", 100.0, 100.0, events)], vec![], "")])], vec![b])
}

#[test]
fn buttons_follow_the_pointer_through_their_hit_area_and_route_clicks() {
    let d = button_doc();
    let path = vec![ElementId(3)];
    let mut s = start(&d, &mut NoScripts);
    let at = |x, y| Vec2::new(x, y);
    // Inside the Hit frame's area but outside the visible art: still the button.
    s = step(&d, &s.state, &[Event::Move(at(180.0, 180.0))], &mut NoScripts);
    assert_eq!(s.state.hover.as_ref(), Some(&path));
    assert_eq!(s.state.button_frame(&path, 4), 1, "Over");
    assert_eq!(s.output, ["over"]);
    s = step(&d, &s.state, &[Event::Down(at(180.0, 180.0))], &mut NoScripts);
    assert_eq!(s.state.button_frame(&path, 4), 2, "Down");
    assert_eq!(s.state.root.frame, 0);
    s = step(&d, &s.state, &[Event::Up(at(181.0, 181.0))], &mut NoScripts);
    assert_eq!((s.state.root.frame, s.state.root.playing), (4, false), "the click went to frame 5");
    // Off the button: Out, Up state.
    s = step(&d, &s.state, &[Event::Move(at(300.0, 300.0))], &mut NoScripts);
    assert_eq!((s.state.hover.as_ref(), s.output.as_slice()), (None, ["out".to_string()].as_slice()));
    assert_eq!(s.state.button_frame(&path, 4), 0);
    // Pressed on it, released off it: no click.
    let mut s = start(&d, &mut NoScripts);
    s = step(&d, &s.state, &[Event::Down(at(150.0, 150.0)), Event::Move(at(400.0, 400.0)), Event::Up(at(400.0, 400.0))], &mut NoScripts);
    assert_eq!(s.state.root.frame, 0);
    assert!(hit(&d, &s.state, at(150.0, 150.0)).is_some() && hit(&d, &s.state, at(99.0, 99.0)).is_none());
}

#[test]
fn script_set_properties_change_what_the_player_draws() {
    let hero = symbol(100, SymbolKind::MovieClip, vec![layer(11, vec![kf(0, 1, vec![rect(12, 0.0, 0.0, 5.0, 5.0)], vec![], "")])]);
    let set = |p, v| Action::SetProperty { target: Target::new("hero"), property: p, value: v };
    let d = doc(vec![layer(1, vec![kf(0, 1, vec![], vec![set(Property::X, 300.0), set(Property::Alpha, 0.5)], ""), kf(1, 1, vec![], vec![set(Property::Visible, 0.0)], "")]), layer(2, vec![kf(0, 2, vec![inst(3, 100, "hero", 0.0, 0.0, vec![])], vec![], "")])], vec![hero]);
    let s = start(&d, &mut NoScripts);
    let dl = evaluate_player(&d, &s.state);
    assert_eq!(dl.items.len(), 1);
    assert_eq!(dl.items[0].transform.translation().x, 300.0);
    assert_eq!(dl.effective_alpha(&dl.items[0]), 0.5);
    assert_eq!(evaluate(&d, 0).items[0].transform.translation().x, 0.0, "the document is untouched");
    let s = step(&d, &s.state, &[], &mut NoScripts);
    assert!(evaluate_player(&d, &s.state).items.is_empty(), "hidden");
}

#[test]
fn without_actions_the_player_draws_what_the_timeline_does() {
    // The demo with its star as a movie clip: the player's clip playheads must match the
    // authoring formula frame for frame.
    let mut d = sample::demo();
    for sym in &mut d.library {
        Arc::make_mut(sym).kind = SymbolKind::MovieClip;
    }
    let mut s = start(&d, &mut NoScripts);
    for f in 0..d.frame_count() {
        let (a, b) = (evaluate(&d, f), evaluate_player(&d, &s.state));
        assert_eq!(a.items.len(), b.items.len(), "frame {f}");
        for (x, y) in a.items.iter().zip(&b.items) {
            assert_eq!(x.transform, y.transform, "frame {f}");
        }
        s = step(&d, &s.state, &[], &mut NoScripts);
    }
}

#[test]
fn the_same_inputs_give_the_same_movie() {
    let d = sample::interactive();
    let script = |tick: usize| -> Vec<Event> {
        match tick {
            2 => vec![Event::Move(Vec2::new(640.0, 400.0))],
            3 => vec![Event::Down(Vec2::new(640.0, 400.0))],
            4 => vec![Event::Up(Vec2::new(640.0, 400.0))],
            9 => vec![Event::Leave],
            _ => vec![],
        }
    };
    let play = || {
        let mut s = start(&d, &mut NoScripts);
        let mut trace = vec![(s.state.clone(), s.output.clone())];
        for t in 0..80 {
            s = step(&d, &s.state, &script(t), &mut NoScripts);
            trace.push((s.state.clone(), s.output.clone()));
        }
        trace
    };
    let (a, b) = (play(), play());
    assert_eq!(a, b);
    // The click started the show; its last frame went back to the menu and stopped.
    let out: Vec<String> = a.iter().flat_map(|(_, o)| o.clone()).collect();
    assert_eq!(out, ["The show starts", "Back to the menu"]);
    let last = &a.last().unwrap().0;
    assert_eq!((last.root.frame, last.root.playing), (0, false));
}

#[test]
fn go_to_loops_end_in_a_message_not_a_hang() {
    let d = doc(vec![layer(1, vec![kf(0, 1, vec![], vec![goto_stop("", FrameRef::Number(1))], ""), kf(1, 1, vec![], vec![goto_stop("", FrameRef::Number(0))], "")])], vec![]);
    let s = start(&d, &mut NoScripts);
    assert!(s.output.iter().any(|l| l.starts_with("Stopped running actions this frame")), "{:?}", s.output);
    let s = step(&d, &s.state, &[], &mut NoScripts);
    assert!(s.output.len() <= 2);
    // More traces than one step keeps: the rest are counted.
    let many: Vec<Action> = (0..crate::actions::MAX_ACTIONS).map(|i| Action::Trace { message: format!("{i}") }).collect();
    let d = doc((1..=5).map(|l| layer(l, vec![kf(0, 1, vec![], many.clone(), "")])).collect(), vec![]);
    let s = start(&d, &mut NoScripts);
    assert_eq!(s.output.len(), MAX_OUTPUT_PER_STEP + 1);
    assert_eq!(s.output.last().map(String::as_str), Some("… 280 more lines"));
}

#[test]
fn bad_targets_and_graphics_are_reported() {
    let g = symbol(100, SymbolKind::Graphic, vec![layer(11, vec![kf(0, 1, vec![rect(12, 0.0, 0.0, 5.0, 5.0)], vec![], "")])]);
    let acts = vec![Action::Stop { target: Target::new("nobody") }, Action::Play { target: Target::new("g") }, Action::Stop { target: Target::new("parent") }, Action::GotoAndStop { target: Target::this(), frame: FrameRef::Number(0), scene: Some("Nope".into()) }];
    let d = doc(vec![layer(1, vec![kf(0, 2, vec![inst(3, 100, "g", 0.0, 0.0, vec![])], acts, "")])], vec![g]);
    let s = start(&d, &mut NoScripts);
    assert_eq!(s.output.len(), 4, "{:?}", s.output);
    assert!(s.output[1].contains("is a graphic"));
}

#[test]
fn plug_in_frame_scripts_run_through_the_host() {
    struct Host(u32);
    impl ScriptHost for Host {
        fn run(&mut self, script: &PluginScript, cx: &ScriptContext<'_>) -> ScriptRun {
            self.0 += 1;
            ScriptRun { actions: vec![stop()], output: vec![format!("{} at {} of “{}”", script.entry, cx.frame, cx.nodes[cx.timeline].name)] }
        }
    }
    let mut k = Keyframe::new(2, 8);
    k.script = Some(FrameScript::Plugin(PluginScript { id: "org.test.s".into(), entry: "main".into() }));
    let d = doc(vec![layer(1, vec![kf(0, 2, vec![], vec![], ""), Arc::new(k)])], vec![]);
    let mut h = Host(0);
    let mut s = start(&d, &mut h);
    for _ in 0..5 {
        s = step(&d, &s.state, &[], &mut h);
    }
    assert_eq!(h.0, 1, "entered once");
    assert_eq!((s.state.root.frame, s.state.root.playing), (2, false));
    let s = run(&d, 3);
    assert!(s.state.root.playing);
}

#[test]
fn input_between_frames_moves_no_playhead() {
    let d = button_doc();
    let s = start(&d, &mut NoScripts);
    let s = input(&d, &s.state, &[Event::Move(Vec2::new(150.0, 150.0)), Event::Down(Vec2::new(150.0, 150.0)), Event::Up(Vec2::new(150.0, 150.0))], &mut NoScripts);
    assert_eq!((s.state.root.frame, s.state.tick), (4, 0), "the click ran, no frame advanced");
}


#[test]
fn the_player_sees_through_the_camera_and_buttons_are_hit_where_they_show() {
    let mut d = button_doc();
    // A camera zoomed 2x on the button's corner (100, 100): it shows at the output's middle.
    let cam = crate::Camera { center: Vec2::new(100.0, 100.0), zoom: 2.0, ..crate::Camera::rest(&d.stage) };
    let mut cl = Layer::new(LayerId(90), "Camera");
    cl.kind = crate::LayerKind::Camera;
    cl.keyframes = vec![kf(0, 10, vec![Arc::new(cam.to_element(ElementId(91)))], vec![], "")];
    d.timeline_mut(0).layers.insert(0, Arc::new(cl));
    let s = start(&d, &mut NoScripts);
    let mid = Vec2::new(d.stage.width as f32 / 2.0, d.stage.height as f32 / 2.0);
    let item = &evaluate_player(&d, &s.state).items[0];
    assert!(item.transform.apply(Vec2::ZERO).distance(mid) < 1e-3, "drawn through the camera");
    assert!((item.transform.mean_scale() - 2.0).abs() < 1e-4);
    // The Hit area (100 x 100 at the corner) is now 200 x 200 from the middle.
    assert!(hit(&d, &s.state, mid + Vec2::new(150.0, 150.0)).is_some());
    assert!(hit(&d, &s.state, mid - Vec2::new(5.0, 5.0)).is_none());
}

fn with_sound(k: Arc<Keyframe>, id: u64, sync: crate::SoundSync) -> Arc<Keyframe> {
    let mut k = (*k).clone();
    k.sound = Some(crate::FrameSound { sync, ..crate::FrameSound::new(crate::SoundId(id)) });
    Arc::new(k)
}

/// The sound ids a step started, with their volumes.
fn started(s: &Step) -> Vec<(u64, f32)> {
    s.sounds.iter().map(|x| (x.sound.sound.0, x.gain)).collect()
}

#[test]
fn entered_keyframes_start_their_sounds_in_clips_and_graphics_with_layer_volumes() {
    use crate::SoundSync::{Event, Stop};
    // A 3-frame movie clip with a sound on its frame 1 (index 1); a 2-frame graphic with one on
    // its frame 0; the main timeline has its own on frame 0 and a stop at 2.
    let clip = symbol(20, SymbolKind::MovieClip, vec![layer(21, vec![kf(0, 1, vec![], vec![], ""), with_sound(kf(1, 2, vec![], vec![], ""), 2, Event)])]);
    let graphic = symbol(30, SymbolKind::Graphic, vec![layer(31, vec![with_sound(kf(0, 1, vec![], vec![], ""), 3, Event), kf(1, 1, vec![], vec![], "")])]);
    let mut holder = (*layer(2, vec![kf(0, 6, vec![inst(40, 20, "c", 0.0, 0.0, vec![]), inst(41, 30, "g", 0.0, 0.0, vec![])], vec![], "")])).clone();
    holder.volume = 0.5;
    let root = layer(1, vec![with_sound(kf(0, 1, vec![], vec![], ""), 1, Event), with_sound(kf(1, 5, vec![], vec![], ""), 1, Stop)]);
    let d = doc(vec![root, Arc::new(holder)], vec![clip, graphic]);
    // Frame 1: the main timeline's sound and the graphic's (it appeared on its frame 0).
    let s = start(&d, &mut NoScripts);
    assert_eq!(started(&s), vec![(1, 1.0), (3, 0.5)]);
    // Frame 2: the clip reaches its frame 1; the graphic moves to its frame 1 (no sound); the
    // main timeline's Stop.
    let s = step(&d, &s.state, &[], &mut NoScripts);
    assert_eq!(started(&s), vec![(1, 1.0), (2, 0.5)]);
    assert_eq!(s.sounds[0].sound.sync, Stop);
    // Frame 3: the graphic is back on its frame 0.
    let s = step(&d, &s.state, &[], &mut NoScripts);
    assert_eq!(started(&s), vec![(3, 0.5)]);
    // Muted, the sounds still start (Stop and Start still work) but silently.
    let mut m = d.clone();
    Arc::make_mut(&mut m.timeline_mut(0).layers[1]).muted = true;
    assert_eq!(started(&start(&m, &mut NoScripts)), vec![(1, 1.0), (3, 0.0)]);
}

// ----- Sounds on button frames (ADR 0057) --------------------------------------------------------

/// [`button_doc`] with a sound on each state frame (ids 10 Up, 11 Over, 12 Down, 13 Hit; Over's
/// a stream), on a layer at half volume, the button on a layer at half volume too.
fn sounding_button() -> Document {
    use crate::SoundSync::{Event, Stream};
    let mut d = button_doc();
    let art = |id| vec![rect(id, 0.0, 0.0, 10.0, 10.0)];
    let mut sounds = (*layer(34, vec![with_sound(kf(0, 1, vec![], vec![], ""), 10, Event), with_sound(kf(1, 1, vec![], vec![], ""), 11, Stream), with_sound(kf(2, 1, vec![], vec![], ""), 12, Event), with_sound(kf(3, 1, vec![], vec![], ""), 13, Event)])).clone();
    sounds.volume = 0.5;
    let states = layer(31, vec![kf(0, 1, art(32), vec![], ""), kf(1, 1, art(32), vec![], ""), kf(2, 1, art(32), vec![], ""), kf(3, 1, vec![rect(33, 0.0, 0.0, 100.0, 100.0)], vec![], "")]);
    d.library = vec![symbol(300, SymbolKind::Button, vec![Arc::new(sounds), states])];
    Arc::make_mut(&mut d.timeline_mut(0).layers[1]).volume = 0.5;
    let tone: Vec<f32> = (0..800).map(|i| (i as f32 * 0.1).sin() * 0.5).collect();
    for id in 10..14 {
        let data = anicraft_audio::wav::encode(8000, 1, &tone, anicraft_audio::wav::Bits::Float32);
        d.sounds.push(Arc::new(crate::Sound::from_data(crate::SoundId(id), format!("s{id}.wav"), data).expect("a WAV")));
    }
    d
}

#[test]
fn a_button_plays_the_sound_of_the_state_it_enters() {
    let d = sounding_button();
    let at = |x, y| Vec2::new(x, y);
    let s = start(&d, &mut NoScripts);
    assert!(s.sounds.is_empty(), "a button on the stage is silent until the pointer comes");
    // Over: the Over frame's sound, as an event (a button frame has no span to stream over), at
    // the volumes of both layers.
    let s = step(&d, &s.state, &[Event::Move(at(150.0, 150.0))], &mut NoScripts);
    assert_eq!(started(&s), vec![(11, 0.25)]);
    assert_eq!(s.sounds[0].sound.sync, crate::SoundSync::Event);
    // Moving about inside it plays nothing more.
    let s = step(&d, &s.state, &[Event::Move(at(160.0, 150.0))], &mut NoScripts);
    assert!(s.sounds.is_empty());
    // Pressed: Down's. Released over it: Hit's (the SWF format's Down → Over sound).
    let s = input(&d, &s.state, &[Event::Down(at(160.0, 150.0))], &mut NoScripts);
    assert_eq!(started(&s), vec![(12, 0.25)]);
    let s = input(&d, &s.state, &[Event::Up(at(160.0, 150.0))], &mut NoScripts);
    assert_eq!(started(&s), vec![(13, 0.25)]);
    // Left: Up's.
    let s = step(&d, &s.state, &[Event::Leave], &mut NoScripts);
    assert_eq!(started(&s), vec![(10, 0.25)]);
    // Arriving and pressing in one event: Over's, then Down's.
    let s = input(&d, &s.state, &[Event::Down(at(150.0, 150.0))], &mut NoScripts);
    assert_eq!(started(&s), vec![(11, 0.25), (12, 0.25)]);
    // Dragged off while pressed, back on and released off: nothing (no state with a sound).
    let s = input(&d, &s.state, &[Event::Move(at(400.0, 400.0)), Event::Move(at(150.0, 150.0)), Event::Move(at(400.0, 400.0)), Event::Up(at(400.0, 400.0))], &mut NoScripts);
    assert!(s.sounds.is_empty(), "{:?}", started(&s));
}

#[test]
fn button_sounds_follow_the_frames_a_button_has_and_stay_out_of_the_soundtrack() {
    // A two-frame button (Up and Over): pressing it shows Over still, and plays nothing.
    let mut d = sounding_button();
    let short = Timeline { layers: d.library[0].timeline.layers.iter().map(|l| {
        let mut l = (**l).clone();
        l.keyframes.truncate(2);
        Arc::new(l)
    }).collect() };
    Arc::make_mut(&mut d.library[0]).timeline = short;
    let s = start(&d, &mut NoScripts);
    // Its Up frame is its hit area (10 × 10 at 100, 100).
    let s = input(&d, &s.state, &[Event::Move(Vec2::new(105.0, 105.0)), Event::Down(Vec2::new(105.0, 105.0))], &mut NoScripts);
    assert_eq!(started(&s), vec![(11, 0.25)]);
    // The mix and every export leave buttons out: they only sound under the pointer.
    assert!(crate::sound::voices(&sounding_button(), 48_000).is_empty());
}

#[test]
fn the_hit_overlay_is_the_area_the_pointer_finds() {
    let d = button_doc();
    let s = start(&d, &mut NoScripts);
    let tint = Rgba::new(1.0, 0.0, 0.0, 1.0);
    let dl = hit_overlay(&d, &s.state, tint, 0.4);
    // One area: the Hit frame's 100 × 100 square at (100, 100), tinted, in one group at 40 %.
    assert_eq!(dl.items.len(), 1);
    assert_eq!(dl.groups.first().map(|g| g.alpha), Some(0.4));
    let i = &dl.items[0];
    assert_eq!((i.color, i.group), (ColorTransform::tint(tint, 1.0), Some(0)));
    assert_eq!(i.color.apply(Rgba::new(0.1, 0.6, 0.9, 0.8)), Rgba::new(1.0, 0.0, 0.0, 0.8), "every colour becomes the tint, its alpha kept");
    for p in [Vec2::new(101.0, 101.0), Vec2::new(199.0, 150.0), Vec2::new(50.0, 50.0), Vec2::new(201.0, 150.0)] {
        assert_eq!(i.path.contains(p, &i.transform, i.fill_rule), hit(&d, &s.state, p).is_some(), "{p:?}");
    }
    // A hidden button has no area; a movie without buttons none at all.
    assert!(hit_overlay(&doc(vec![], vec![]), &PlayerState::default(), tint, 0.4).items.is_empty());
    assert_eq!(hit_frame(4), 3);
    assert_eq!(hit_frame(3), 0);
    assert_eq!(hit_frame(0), 0);
}

// ----- ActionScript (ADR 0013 tier 2) -----------------------------------------------------------

/// `k` with a kept ActionScript 3.0 script in place of its actions.
fn as3(k: Arc<Keyframe>, src: &str) -> Arc<Keyframe> {
    let mut k = (*k).clone();
    k.script = Some(FrameScript::Source(Arc::new(crate::actions::SourceScript::new(crate::actions::Language::ActionScript3, src))));
    Arc::new(k)
}

/// The state without the listeners scripts add (hand-made actions use handlers instead).
fn lean(st: &PlayerState) -> PlayerState {
    PlayerState { listeners: BTreeMap::new(), seq: 0, ..st.clone() }
}

/// Plays both documents through the same pointer events and asserts the same movie: state and
/// output at every step.
fn same_movie(a: &Document, b: &Document, events: &[Vec<Event>]) {
    let (mut x, mut y) = (start(a, &mut NoScripts), start(b, &mut NoScripts));
    assert_eq!((lean(&x.state), &x.output), (lean(&y.state), &y.output), "start");
    for (i, ev) in events.iter().enumerate() {
        x = step(a, &x.state, ev, &mut NoScripts);
        y = step(b, &y.state, ev, &mut NoScripts);
        assert_eq!((lean(&x.state), &x.output), (lean(&y.state), &y.output), "step {}", i + 1);
    }
}

#[test]
fn actionscript_and_hand_made_actions_play_the_same_movie() {
    // Nested clips told what to do by instance-name path.
    let hand = nested();
    let mut script = nested();
    {
        let l = Arc::make_mut(&mut script.timeline_mut(0).layers[0]);
        l.keyframes[0] = as3(l.keyframes[0].clone(), "hero.gotoAndStop(\"run\");\nthis.hero.arm.gotoAndStop(3);");
        let arm = Arc::make_mut(&mut script.library[1]);
        let al = Arc::make_mut(&mut arm.timeline.layers[0]);
        al.keyframes[1] = as3(al.keyframes[1].clone(), "MovieClip(this.parent.parent).stop();");
    }
    same_movie(&hand, &script, &vec![vec![]; 6]);
    // A button's handlers, and the same as listeners a frame script adds.
    let hand = button_doc();
    let mut script = button_doc();
    {
        let tl = script.timeline_mut(0);
        let l0 = Arc::make_mut(&mut tl.layers[0]);
        l0.keyframes[0] = as3(l0.keyframes[0].clone(), "import flash.events.MouseEvent;\nstop();\ngo.addEventListener(MouseEvent.CLICK, onGo);\ngo.addEventListener(MouseEvent.ROLL_OVER, function (e:MouseEvent):void { trace(\"over\"); });\ngo.addEventListener(MouseEvent.ROLL_OUT, function (e:MouseEvent):void { trace(\"o\" + \"ut\"); });\nfunction onGo(e:MouseEvent):void {\n  gotoAndStop(5);\n}\n");
        let l1 = Arc::make_mut(&mut tl.layers[1]);
        let k = Arc::make_mut(&mut l1.keyframes[0]);
        if let ElementKind::Symbol { events, .. } = &mut Arc::make_mut(&mut k.elements[0]).kind {
            events.clear();
        }
    }
    let at = |x, y| Vec2::new(x, y);
    let events = vec![vec![], vec![Event::Move(at(180.0, 180.0))], vec![Event::Down(at(180.0, 180.0))], vec![Event::Up(at(181.0, 181.0))], vec![Event::Move(at(300.0, 300.0))], vec![], vec![Event::Move(at(150.0, 150.0)), Event::Leave]];
    same_movie(&hand, &script, &events);
    let s = start(&script, &mut NoScripts);
    assert_eq!(s.state.listeners.get(&vec![ElementId(3)]).map(Vec::len), Some(3), "three listeners on the button");
}

#[test]
fn a_script_outside_the_subset_does_not_run_at_all() {
    let d = doc(vec![layer(1, vec![as3(kf(0, 10, vec![], vec![], ""), "stop();\ntrace(\"half\");\nswitch (1) { }")])], vec![]);
    let s = run(&d, 3);
    assert!(s.state.root.playing && s.output.is_empty(), "nothing of it ran: {:?}", s.output);
    assert_eq!(s.state.root.frame, 3);
    // In ActionScript 1.0 & 2.0 nothing runs either.
    let mut d2 = doc(vec![layer(1, vec![kf(0, 10, vec![], vec![], "")])], vec![]);
    Arc::make_mut(&mut Arc::make_mut(&mut d2.timeline_mut(0).layers[0]).keyframes[0]).script = Some(FrameScript::Source(Arc::new(crate::actions::SourceScript::new(crate::actions::Language::ActionScript1or2, "stop();"))));
    assert!(run(&d2, 2).state.root.playing);
}

#[test]
fn listeners_live_with_their_instance_and_are_added_once() {
    // `pad` is a movie clip with no handlers: a listener makes it clickable. It is on the stage
    // for frames 0-4; the main timeline loops round 8 frames, adding the listener again each
    // time it enters frame 1.
    let pad = symbol(100, SymbolKind::MovieClip, vec![layer(11, vec![kf(0, 1, vec![rect(12, 0.0, 0.0, 20.0, 20.0)], vec![], "")])]);
    let script = "pad.addEventListener(MouseEvent.CLICK, hit);\nfunction hit(e:MouseEvent):void { trace(\"hit\"); }\nghost.addEventListener(MouseEvent.CLICK, hit);";
    let d = doc(vec![layer(1, vec![as3(kf(0, 8, vec![], vec![], ""), script)]), layer(2, vec![kf(0, 5, vec![inst(3, 100, "pad", 10.0, 10.0, vec![])], vec![], ""), kf(5, 3, vec![], vec![], "")])], vec![pad]);
    let path = vec![ElementId(3)];
    let s = start(&d, &mut NoScripts);
    assert_eq!(s.output, ["Listen for click (ghost): no instance named “ghost” here"]);
    assert!(hit(&d, &s.state, Vec2::new(15.0, 15.0)).is_some(), "the listener makes the clip clickable");
    let at = Vec2::new(15.0, 15.0);
    let s = step(&d, &s.state, &[Event::Move(at), Event::Down(at), Event::Up(at)], &mut NoScripts);
    assert!(s.output.iter().any(|l| l == "hit"), "{:?}", s.output);
    let s = run(&d, 5);
    assert!(s.state.listeners.is_empty(), "pad left the stage, and its listener with it");
    // Round the loop: the frame-1 script adds the same function again, which changes nothing.
    let s = run(&d, 8);
    assert_eq!(s.state.listeners.get(&path).map(Vec::len), Some(1));
    let s = run(&d, 16);
    assert_eq!(s.state.listeners.get(&path).map(Vec::len), Some(1));
}

#[test]
fn next_scene_in_a_movie_clip_script_stays_in_its_clip() {
    // A clip's own script calling nextScene(): in ActionScript the clip has one scene, so the
    // main timeline stays (hand-made Next scene actions do move it, as before).
    let clip = symbol(100, SymbolKind::MovieClip, vec![layer(11, vec![as3(kf(0, 2, vec![rect(12, 0.0, 0.0, 5.0, 5.0)], vec![], ""), "nextScene();")])]);
    let mut d = doc(vec![layer(1, vec![kf(0, 4, vec![inst(3, 100, "c", 0.0, 0.0, vec![])], vec![], "")])], vec![clip]);
    d.scenes.push(Arc::new(crate::Scene::new("Two")));
    let s = start(&d, &mut NoScripts);
    assert_eq!(s.state.scene, 0);
    assert_eq!(s.output, ["Next scene: a movie clip's timeline has one scene; call it in the main timeline's script"]);
}

// ----- ActionScript small logic (ADR 0052) ---------------------------------------------

/// A one-layer main timeline of `frames` frames with scripts on some of them, and `extra` layers.
fn scripted(frames: u32, scripts: &[(u32, &str)], extra: Vec<Arc<Layer>>, library: Vec<Arc<Symbol>>) -> Document {
    let mut keys = Vec::new();
    let mut starts: Vec<u32> = scripts.iter().map(|(f, _)| *f).collect();
    starts.push(frames);
    for (i, (f, src)) in scripts.iter().enumerate() {
        let end = starts.get(i + 1).copied().unwrap_or(frames);
        keys.push(if src.is_empty() { kf(*f, end - f, vec![], vec![], "") } else { as3(kf(*f, end - f, vec![], vec![], ""), src) });
    }
    let mut layers = vec![layer(1, keys)];
    layers.extend(extra);
    doc(layers, library)
}

fn outputs(d: &Document, steps: usize) -> Vec<String> {
    let mut s = start(d, &mut NoScripts);
    let mut out = s.output.clone();
    for _ in 0..steps {
        s = step(d, &s.state, &[], &mut NoScripts);
        out.extend(s.output.iter().cloned());
    }
    out
}

#[test]
fn timeline_variables_and_functions_live_across_frames() {
    let d = scripted(4, &[(0, "var score:int = 10;\nfunction bump(k:int):int { score += k; return score; }\ntrace(\"start \" + score + \" \" + level);"), (2, "trace(bump(5), bump(1.9), level);\nlevel = \"two\";"), (3, "trace(level);\nvar level:String = \"one\"; trace(level); stop();")], vec![], vec![]);
    // `level` is declared on frame 4 but is the timeline's from the start (null until set).
    assert_eq!(outputs(&d, 5), ["start 10 null", "15 16 null", "two", "one"]);
    let s = run(&d, 5);
    assert_eq!(s.state.vars.get(&Vec::new()).and_then(|v| v.get("score")), Some(&Value::Num(16.0)));
    // A script that uses a name no frame declares doesn't run at all.
    let d = scripted(3, &[(0, "trace(\"one\");"), (1, "trace(\"two\"); trace(nobody);")], vec![], vec![]);
    assert_eq!(outputs(&d, 2), ["one"]);
    assert_eq!(link_problems(&d, TimelineRef::Root, d.scenes[0].timeline.layers[0].keyframes[1].script.as_ref().and_then(FrameScript::source).unwrap().compiled().as_ref().unwrap())[0].message, "`nobody` isn't a variable of this timeline (declare it with `var` in one of its frame scripts)");
}

#[test]
fn each_movie_clip_instance_keeps_its_own_variables() {
    // A two-frame clip counts its loops; two instances count apart, and one that leaves the
    // stage and comes back starts again.
    let clip = symbol(100, SymbolKind::MovieClip, vec![layer(11, vec![as3(kf(0, 2, vec![rect(12, 0.0, 0.0, 5.0, 5.0)], vec![], ""), "var n:int; n++; trace(name + \" \" + n);")])]);
    let a = layer(2, vec![kf(0, 8, vec![inst(3, 100, "a", 0.0, 0.0, vec![])], vec![], "")]);
    let b = layer(3, vec![kf(0, 3, vec![inst(4, 100, "b", 50.0, 0.0, vec![])], vec![], ""), kf(3, 2, vec![], vec![], ""), kf(5, 3, vec![inst(5, 100, "b", 50.0, 0.0, vec![])], vec![], "")]);
    let d = scripted(8, &[(0, "")], vec![a, b], vec![clip]);
    let out = outputs(&d, 6);
    assert_eq!(out, ["a 1", "b 1", "a 2", "b 2", "a 3", "b 1", "a 4"], "{out:?}");
}

#[test]
fn enter_frame_listeners_run_every_frame_until_removed() {
    let src = "var t:int = 0;\nstop();\naddEventListener(Event.ENTER_FRAME, tick);\nfunction tick(e:Event):void {\n  t++;\n  trace(\"tick \" + t + \" on \" + currentFrame + \" of \" + totalFrames);\n  if (t == 3) { removeEventListener(Event.ENTER_FRAME, tick); gotoAndStop(2); }\n}";
    let d = scripted(3, &[(0, src), (1, "trace(\"frame 2 runs after the listener\");")], vec![], vec![]);
    assert_eq!(outputs(&d, 5), ["tick 1 on 1 of 3", "tick 2 on 1 of 3", "tick 3 on 1 of 3", "frame 2 runs after the listener"]);
    assert!(run(&d, 5).state.listeners.is_empty());
}

#[test]
fn keys_reach_the_stage_listeners_with_their_codes() {
    let clip = symbol(100, SymbolKind::MovieClip, vec![layer(11, vec![kf(0, 1, vec![rect(12, 0.0, 0.0, 5.0, 5.0)], vec![], "")])]);
    let src = "stop();\nstage.addEventListener(KeyboardEvent.KEY_DOWN, down);\nstage.addEventListener(KeyboardEvent.KEY_UP, function (e:KeyboardEvent):void { trace(\"up \" + e.keyCode); });\nfunction down(e:KeyboardEvent):void {\n  if (e.keyCode == Keyboard.RIGHT) hero.x += 10;\n  else if (e.keyCode == Keyboard.LEFT) hero.x -= 10;\n  trace(e.keyCode, hero.x, e.target == stage);\n}";
    let d = scripted(2, &[(0, src)], vec![layer(2, vec![kf(0, 2, vec![inst(3, 100, "hero", 30.0, 40.0, vec![])], vec![], "")])], vec![clip]);
    let s = start(&d, &mut NoScripts);
    assert_eq!(s.state.stage.len(), 2);
    let s = step(&d, &s.state, &[Event::KeyDown(39), Event::KeyDown(39), Event::KeyUp(39)], &mut NoScripts);
    assert_eq!(s.output, ["39 40 true", "39 50 true", "up 39"]);
    // Between frames too.
    let s = input(&d, &s.state, &[Event::KeyDown(37)], &mut NoScripts);
    assert_eq!(s.output, ["37 40 true"]);
    assert_eq!(s.state.props.get(&vec![ElementId(3)]).and_then(|o| o.x), Some(40.0));
}

#[test]
fn scripts_read_instance_and_timeline_properties() {
    let clip = symbol(100, SymbolKind::MovieClip, vec![layer(11, vec![kf(0, 3, vec![rect(12, 0.0, 0.0, 5.0, 5.0)], vec![], "walk")])]);
    let mut hero = (*inst(3, 100, "hero", 30.0, 40.0, vec![])).clone();
    hero.transform = Affine::translate(30.25, 40.0).mul(&Affine::scale(2.0, 0.5));
    hero.color = crate::ColorEffect::Alpha(0.3);
    let src = "stop();\ntrace(hero.x, hero.y, hero.scaleX, hero.scaleY, hero.rotation, hero.alpha, hero.visible, hero.name);\ntrace(hero.currentFrame, hero.totalFrames, hero.currentLabel, currentFrame, totalFrames, currentLabel, this.name);\nhero.x = 0.1; hero.alpha = hero.alpha + 0.2; hero.visible = false;\ntrace(hero.x, hero.alpha, hero.visible);\nstage.addEventListener(KeyboardEvent.KEY_DOWN, function (e:KeyboardEvent):void { trace(mouseX, mouseY, hero.mouseX, stage.mouseY); });";
    let k = as3(kf(0, 2, vec![], vec![], "intro"), src);
    let mut hk = Keyframe::new(0, 2);
    hk.elements = vec![Arc::new(hero)];
    let d = doc(vec![layer(1, vec![k]), layer(2, vec![Arc::new(hk)])], vec![clip]);
    let s = start(&d, &mut NoScripts);
    assert_eq!(s.output, ["30.25 40 2 0.5 0 0.3 true hero", "1 3 walk 1 2 intro root1", "0.1 0.5 false"]);
    let s = input(&d, &s.state, &[Event::Move(Vec2::new(50.5, 60.0)), Event::Leave, Event::KeyDown(32)], &mut NoScripts);
    // The pointer's last place stays after it leaves; a clip sees it in its own space.
    assert_eq!(s.output, ["50.5 60 25.2 60"]);
}

#[test]
fn random_numbers_are_seeded_and_repeat() {
    let src = "trace(Math.random(), Math.floor(Math.random() * 6) + 1);";
    let d = scripted(2, &[(0, src)], vec![], vec![]);
    let a = start_seeded(&d, 7, &mut NoScripts).output;
    let b = start_seeded(&d, 7, &mut NoScripts).output;
    let c = start_seeded(&d, 8, &mut NoScripts).output;
    assert_eq!(a, b);
    assert_ne!(a, c);
    // The generator itself: in [0, 1), and spread out.
    let mut r = Rng::seeded(DEFAULT_SEED);
    let v: Vec<f64> = (0..10_000).map(|_| r.next_f64()).collect();
    assert!(v.iter().all(|x| (0.0..1.0).contains(x)));
    let mean = v.iter().sum::<f64>() / v.len() as f64;
    assert!((mean - 0.5).abs() < 0.02, "{mean}");
    assert_eq!(start(&d, &mut NoScripts).output, start_seeded(&d, DEFAULT_SEED, &mut NoScripts).output);
}

#[test]
fn an_endless_loop_stops_its_script_and_the_movie_plays_on() {
    let d = scripted(3, &[(0, "trace(\"a\");\nwhile (true) { }\ntrace(\"never\");"), (1, "trace(\"frame 2\");")], vec![], vec![]);
    let s = start(&d, &mut NoScripts);
    assert_eq!(s.output, ["a", "Script on frame 1 of the main timeline stopped: it ran more than 100000 steps (an endless loop?)"]);
    let s = step(&d, &s.state, &[], &mut NoScripts);
    assert_eq!((s.state.root.frame, s.output.as_slice()), (1, ["frame 2".to_string()].as_slice()));
    // A runtime error stops only its script.
    let d = scripted(2, &[(0, "trace(\"x\");\ntrace(ghost.x);\ntrace(\"y\");")], vec![], vec![]);
    assert_eq!(start(&d, &mut NoScripts).output, ["x", "Script on frame 1 of the main timeline stopped: line 2: no instance named “ghost” here"]);
    // Many listeners that each loop share one step's fuel; the step ends with one message.
    let mut src = String::from("stop();\n");
    for i in 0..15 {
        src.push_str(&format!("addEventListener(Event.ENTER_FRAME, function (e:Event):void {{ var n:int = {i}; while (true) n++; }});\n"));
    }
    let d = scripted(2, &[(0, &src)], vec![], vec![]);
    let s = step(&d, &start(&d, &mut NoScripts).state, &[], &mut NoScripts);
    assert_eq!(s.output.iter().filter(|l| l.ends_with("(an endless loop?)")).count(), 10, "{:?}", s.output);
    assert!(s.output.last().is_some_and(|l| l.starts_with("Stopped running actions this frame: scripts ran more than 1000000 steps")), "{:?}", s.output);
}

#[test]
fn listeners_know_their_target_and_current_target() {
    let mut d = button_doc();
    {
        let tl = d.timeline_mut(0);
        let l0 = Arc::make_mut(&mut tl.layers[0]);
        l0.keyframes[0] = as3(l0.keyframes[0].clone(), "stop();\ngo.addEventListener(MouseEvent.CLICK, clicked);\nfunction clicked(e:MouseEvent):void { trace(e.target.name, e.currentTarget == go, e.target != stage, e.currentTarget.x); report(e); }\nfunction report(ev:MouseEvent):void { trace(\"from \" + ev.currentTarget.name); }");
        let k = Arc::make_mut(&mut Arc::make_mut(&mut tl.layers[1]).keyframes[0]);
        if let ElementKind::Symbol { events, .. } = &mut Arc::make_mut(&mut k.elements[0]).kind {
            events.clear();
        }
    }
    let p = Vec2::new(150.0, 150.0);
    let s = step(&d, &start(&d, &mut NoScripts).state, &[Event::Move(p), Event::Down(p), Event::Up(p)], &mut NoScripts);
    assert_eq!(s.output, ["go true true 100", "from go"]);
}

#[test]
fn scripted_play_is_deterministic() {
    let src = "var n:Number = 0;\naddEventListener(Event.ENTER_FRAME, function (e:Event):void { n += Math.random(); if (n > 2) { trace(n); n = 0; } });";
    let d = scripted(4, &[(0, src)], vec![], vec![]);
    let play = || {
        let mut s = start_seeded(&d, 3, &mut NoScripts);
        let mut out = Vec::new();
        for _ in 0..40 {
            s = step(&d, &s.state, &[Event::KeyDown(65)], &mut NoScripts);
            out.extend(s.output.clone());
        }
        (s.state, out)
    };
    let (a, b) = (play(), play());
    assert!(!a.1.is_empty());
    assert_eq!(a, b);
}

// ----- text fields (ADR 0066) ---------------------------------------------------------------

/// A host that lays fields out in a 10 px monospace (the fonts live above this crate).
struct Mono;

impl ScriptHost for Mono {
    fn run(&mut self, _: &PluginScript, _: &ScriptContext<'_>) -> ScriptRun {
        ScriptRun::default()
    }
    fn lay_out_field(&mut self, block: &TextBlock, shown: &str) -> Option<FieldLook> {
        let f = block.field.as_ref()?;
        Some(crate::field::mono_look(shown, 10.0, 10.0, f.wrap_width(block.box_width)))
    }
}

fn field(id: u64, name: &str, text: &str, kind: TextKind, x: f32, y: f32) -> Arc<Element> {
    let block = TextBlock { text: text.into(), font: "Mono".into(), size: 10.0, bold: false, italic: false, color: Rgba::BLACK, align: crate::TextAlign::Left, letter_spacing: 0.0, line_spacing: 0.0, width: None, outline: Arc::new(Path::rect(2.0, 2.0, 8.0, 8.0)), box_width: 104.0, box_height: 14.0, field: Some(TextField { border: true, ..TextField::new(kind) }), on_path: None };
    Arc::new(Element { id: ElementId(id), name: name.into(), transform: Affine::translate(x, y), color: ColorEffect::default(), kind: ElementKind::Text(Arc::new(block)) })
}

fn field_doc(script: &str) -> Document {
    let fields = vec![field(5, "score_txt", "0", TextKind::Dynamic, 10.0, 10.0), field(6, "name_txt", "", TextKind::Input, 10.0, 50.0), field(7, "", "", TextKind::Input, 10.0, 90.0)];
    doc(vec![layer(1, vec![as3(kf(0, 1, fields, vec![], ""), script)])], vec![])
}

fn text_of(st: &PlayerState, id: u64) -> Option<String> {
    st.fields.get(&vec![ElementId(id)]).and_then(|f| f.text.clone())
}

#[test]
fn scripts_set_and_read_dynamic_text() {
    let d = field_doc("stop();\nvar n:int = 0;\nscore_txt.text = \"Score: \" + n;\nscore_txt.appendText(\"!\");\ntrace(score_txt.text, score_txt.length, name_txt.text == \"\");\nscore_txt.x = 20;\nghost_txt.text = \"x\";");
    let s = start(&d, &mut Mono);
    assert_eq!(text_of(&s.state, 5).as_deref(), Some("Score: 0!"));
    assert!(s.output.contains(&"Score: 0! 9 true".to_string()), "{:?}", s.output);
    assert!(s.output.iter().any(|l| l.contains("no instance named “ghost_txt”")), "{:?}", s.output);
    // Laid out by the host, drawn in place of the authored outline, where the script moved it.
    let look = s.state.fields.get(&vec![ElementId(5)]).and_then(|f| f.look.clone()).expect("laid out");
    assert_eq!(look.lines[0].end, 9);
    let dl = evaluate_player(&d, &s.state);
    assert!(dl.items.iter().any(|i| Arc::ptr_eq(&i.path, &look.outline) && i.transform.m[4] == 20.0));
    // Without a host that can lay out, the text is still set and read; it draws only its box.
    let s2 = start(&d, &mut NoScripts);
    assert_eq!(text_of(&s2.state, 5).as_deref(), Some("Score: 0!"));
    // Static text and other instances aren't fields.
    let d = field_doc("hero.text = \"x\";");
    assert!(start(&d, &mut Mono).output.iter().any(|l| l.contains("no instance named “hero”")));
}

#[test]
fn the_viewer_types_into_input_fields_and_scripts_read_it() {
    let d = field_doc("stop();\nstage.addEventListener(KeyboardEvent.KEY_DOWN, onKey);\nfunction onKey(e:KeyboardEvent):void {\n  if (e.keyCode == Keyboard.ENTER) { score_txt.text = \"Hello, \" + name_txt.text; }\n}");
    let mut h = Mono;
    let s = start(&d, &mut h);
    // A click in the input field gives it the keyboard; typing goes in, editing keys edit.
    let mut ev = vec![Event::Down(Vec2::new(20.0, 55.0)), Event::Up(Vec2::new(20.0, 55.0))];
    ev.extend("Adx".chars().map(Event::Char));
    ev.extend([Event::KeyDown(8), Event::KeyDown(37), Event::Char('a'), Event::KeyDown(35), Event::Char('!')]);
    let s = input(&d, &s.state, &ev, &mut h);
    assert_eq!(s.state.focus, Some(vec![ElementId(6)]));
    assert_eq!(text_of(&s.state, 6).as_deref(), Some("Aad!"));
    // Shift+Home selects; typing replaces it; Enter reaches the stage's key listener too.
    let s = input(&d, &s.state, &[Event::KeyDown(16), Event::KeyDown(36), Event::KeyUp(16), Event::Char('B'), Event::Char('o'), Event::KeyDown(13)], &mut h);
    assert_eq!(text_of(&s.state, 6).as_deref(), Some("Bo"));
    assert_eq!(text_of(&s.state, 5).as_deref(), Some("Hello, Bo"));
    // The focused field draws its caret; a click elsewhere takes the keyboard away.
    let caret = |st: &PlayerState| evaluate_player(&d, st).items.iter().filter(|i| i.fill.is_some() && i.masks.is_none() && i.path.bounds().width() <= 1.01 && i.path.bounds().height() > 5.0).count();
    assert_eq!(caret(&s.state), 1);
    let s = input(&d, &s.state, &[Event::Down(Vec2::new(300.0, 300.0)), Event::Up(Vec2::new(300.0, 300.0)), Event::Char('z')], &mut h);
    assert_eq!(s.state.focus, None);
    assert_eq!(text_of(&s.state, 6).as_deref(), Some("Bo"), "no field, no typing");
    assert_eq!(caret(&s.state), 0);
    // A dynamic field takes no typing; Tab goes from input field to input field.
    let s = input(&d, &s.state, &[Event::Down(Vec2::new(20.0, 15.0)), Event::Up(Vec2::new(20.0, 15.0)), Event::Char('q')], &mut h);
    assert_eq!(s.state.focus, Some(vec![ElementId(5)]), "a selectable dynamic field can be focused");
    assert_eq!(text_of(&s.state, 5).as_deref(), Some("Hello, Bo"));
    let s = input(&d, &s.state, &[Event::Down(Vec2::new(20.0, 55.0)), Event::Up(Vec2::new(20.0, 55.0)), Event::KeyDown(9)], &mut h);
    assert_eq!(s.state.focus, Some(vec![ElementId(7)]));
    let s = input(&d, &s.state, &[Event::KeyDown(9)], &mut h);
    assert_eq!(s.state.focus, Some(vec![ElementId(6)]), "Tab wraps round");
    assert_eq!(s.state.fields.get(&vec![ElementId(6)]).map(|f| f.selection()), Some((0, 2)), "and selects the field's text");
}

#[test]
fn fields_draw_their_box_and_the_authored_text_everywhere() {
    let d = field_doc("");
    // The authoring frame and the player draw the authored outline, a border, inside a clip.
    for dl in [evaluate(&d, 0), evaluate_player(&d, &start(&d, &mut NoScripts).state)] {
        let texts = dl.items.iter().filter(|i| i.path.bounds() == Path::rect(2.0, 2.0, 8.0, 8.0).bounds() && i.masks.is_none()).count();
        assert_eq!(texts, 3);
        assert_eq!(dl.items.iter().filter(|i| i.stroke.is_some()).count(), 3, "three borders");
        assert_eq!(dl.items.iter().filter(|i| i.masks.is_some()).count(), 3, "each box clips its text");
        assert_eq!(dl.clip_parents.len(), 3);
    }
}

#[test]
fn typing_is_deterministic() {
    let d = field_doc("stop();");
    let ev: Vec<Event> = [Event::Down(Vec2::new(30.0, 55.0)), Event::Up(Vec2::new(30.0, 55.0))].into_iter().chain("deterministic".chars().map(Event::Char)).chain([Event::KeyDown(16), Event::KeyDown(37), Event::KeyDown(37), Event::Char('X')]).collect();
    let a = step(&d, &start(&d, &mut Mono).state, &ev, &mut Mono);
    let b = step(&d, &start(&d, &mut Mono).state, &ev, &mut Mono);
    assert_eq!(a, b);
    assert_eq!(text_of(&a.state, 6).as_deref(), Some("deterministX"));
}
