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
    Arc::new(Symbol { id: SymbolId(id), name: format!("S{id}"), kind, linkage: None, timeline: Timeline { layers } })
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

// ----- ActionScript (ADR 0013 tier 2) -----------------------------------------------------------

/// `k` with a kept ActionScript 3.0 script in place of its actions.
fn as3(k: Arc<Keyframe>, src: &str) -> Arc<Keyframe> {
    let mut k = (*k).clone();
    k.script = Some(FrameScript::Source(Arc::new(crate::actions::SourceScript::new(crate::actions::Language::ActionScript3, src))));
    Arc::new(k)
}

/// The state without the listeners scripts add (hand-made actions use handlers instead).
fn lean(st: &PlayerState) -> PlayerState {
    PlayerState { listeners: BTreeMap::new(), ..st.clone() }
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
    let d = doc(vec![layer(1, vec![as3(kf(0, 10, vec![], vec![], ""), "stop();\ntrace(\"half\");\nif (true) { play(); }")])], vec![]);
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
