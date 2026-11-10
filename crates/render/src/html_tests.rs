use super::*;
use anicraft_doc::actions::{ButtonEvent, EventHandler, PluginScript, Property, Target};
use anicraft_doc::player::{Event, NoScripts, PlayerState, start, step};
use anicraft_doc::{ElementId, GradientStop, Keyframe, LayerId, LoopMode, Scene, Shape, Symbol, Tween, evaluate_player, sample};
use anicraft_geom::Vec2;
use boa_engine::{Context, Source};

// ----- fixtures ---------------------------------------------------------------------------------

fn rect(id: u64, x: f32, y: f32, w: f32, h: f32, c: u32) -> Arc<Element> {
    Arc::new(Element { id: ElementId(id), name: String::new(), transform: Affine::IDENTITY, color: ColorEffect::default(), kind: ElementKind::Shape(Shape::filled(Path::rect(x, y, w, h), Paint::Solid(Rgba::hex(c)))) })
}

fn inst(id: u64, symbol: u64, name: &str, m: Affine, events: Vec<EventHandler>) -> Arc<Element> {
    Arc::new(Element { id: ElementId(id), name: name.into(), transform: m, color: ColorEffect::default(), kind: ElementKind::Symbol { symbol: SymbolId(symbol), loop_mode: LoopMode::Loop, first_frame: 0, blend: BlendMode::Normal, filters: Vec::new(), events } })
}

fn kf(start: u32, duration: u32, elements: Vec<Arc<Element>>, actions: Vec<Action>, label: &str) -> Arc<Keyframe> {
    let mut k = Keyframe::new(start, duration);
    k.elements = elements;
    k.label = label.into();
    k.script = (!actions.is_empty()).then_some(FrameScript::Actions(actions));
    Arc::new(k)
}

fn tweened(k: Arc<Keyframe>) -> Arc<Keyframe> {
    let mut k = (*k).clone();
    k.tween = Some(Tween::default());
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

fn this() -> Target {
    Target::this()
}

fn on(event: ButtonEvent, actions: Vec<Action>) -> EventHandler {
    EventHandler { event, actions }
}

/// A self-authored interactive movie that exercises the player: a door movie clip with labels
/// and stops, a tweened slide inside it, a button with all four states that opens the door and
/// fades it on hover, a nested clip addressed by a dotted path, a looping graphic under a mask,
/// a faded group, a gradient, a target that doesn't exist, and a second scene that goes back.
fn fixture() -> Document {
    let mut d = Document { stage: anicraft_doc::Stage { width: 640, height: 360, fps: 24.0, background: Rgba::hex(0x223344) }, ..Document::default() };
    // The door: closed (stop), opening (a slide), open (stop, trace).
    let slide_a = rect(1001, 0.0, 0.0, 80.0, 120.0, 0x8b5a2b);
    let mut slide_b = (*slide_a).clone();
    slide_b.transform = Affine::translate(60.0, 0.0);
    let door = symbol(100, SymbolKind::MovieClip, vec![
        layer(101, vec![kf(0, 1, vec![], vec![Action::Stop { target: this() }], "closed"), kf(1, 4, vec![], vec![], "opening"), kf(5, 1, vec![], vec![Action::Stop { target: this() }, Action::Trace { message: "door open".into() }], "open")]),
        layer(102, vec![kf(0, 1, vec![slide_a.clone()], vec![], ""), tweened(kf(1, 4, vec![slide_a], vec![], "")), kf(5, 1, vec![Arc::new(slide_b)], vec![], "")]),
    ]);
    // The knob: Up, Over, Down, Hit.
    let knob = symbol(200, SymbolKind::Button, vec![layer(201, vec![kf(0, 1, vec![rect(2001, -20.0, -20.0, 40.0, 40.0, 0x3d8bfd)], vec![], ""), kf(1, 1, vec![rect(2002, -20.0, -20.0, 40.0, 40.0, 0x6aa8ff)], vec![], ""), kf(2, 1, vec![rect(2003, -20.0, -20.0, 40.0, 40.0, 0x2a64c0)], vec![], ""), kf(3, 1, vec![rect(2004, -25.0, -25.0, 50.0, 50.0, 0)], vec![], "")])]);
    // A looping three-frame graphic.
    let mut red = (*rect(3001, 0.0, 0.0, 30.0, 30.0, 0xff0000)).clone();
    red.color = ColorEffect { alpha: 0.8, tint: Rgba::hex(0xffffff), tint_amount: 0.2 };
    let blink = symbol(300, SymbolKind::Graphic, vec![layer(301, vec![kf(0, 1, vec![Arc::new(red)], vec![], ""), kf(1, 1, vec![rect(3002, 0.0, 0.0, 30.0, 30.0, 0x00ff00)], vec![], ""), kf(2, 1, vec![rect(3003, 0.0, 0.0, 30.0, 30.0, 0x0000ff)], vec![], "")])]);
    // A panel clip holding its own door (panel.door), that plays on and talks to its parent.
    let panel = symbol(400, SymbolKind::MovieClip, vec![
        layer(401, vec![kf(0, 2, vec![], vec![], ""), kf(2, 2, vec![], vec![Action::Trace { message: "panel frame 3".into() }, Action::SetProperty { target: Target::new("parent.door"), property: Property::X, value: 250.0 }], "")]),
        layer(402, vec![kf(0, 4, vec![inst(4001, 100, "door", Affine::translate(10.0, 10.0), vec![])], vec![], "")]),
    ]);
    d.library = vec![door, knob, blink, panel];

    let open_door = vec![Action::GotoAndPlay { target: Target::new("door"), frame: FrameRef::Label("opening".into()), scene: None }, Action::Play { target: Target::new("panel.door") }];
    let knob_events = vec![
        on(ButtonEvent::Click, open_door),
        on(ButtonEvent::Over, vec![Action::SetProperty { target: Target::new("door"), property: Property::Alpha, value: 0.5 }]),
        on(ButtonEvent::Out, vec![Action::SetProperty { target: Target::new("door"), property: Property::Alpha, value: 1.0 }]),
        on(ButtonEvent::Press, vec![Action::Trace { message: "pressed".into() }]),
        on(ButtonEvent::Release, vec![Action::GotoAndStop { target: Target::new("panel"), frame: FrameRef::Number(99), scene: None }]),
    ];
    let mut faded = (*inst(5003, 300, "", Affine::translate(500.0, 40.0), vec![])).clone();
    faded.color = ColorEffect { alpha: 0.5, tint: Rgba::hex(0x00ffff), tint_amount: 0.6 };
    let gradient = Arc::new(Element {
        id: ElementId(5004),
        name: String::new(),
        transform: Affine::translate(20.0, 300.0),
        color: ColorEffect { alpha: 1.0, tint: Rgba::hex(0xff00ff), tint_amount: 0.25 },
        kind: ElementKind::Shape(Shape::filled(Path::rect(0.0, 0.0, 100.0, 40.0), Paint::Linear { start: Vec2::new(0.0, 0.0), end: Vec2::new(100.0, 0.0), stops: vec![GradientStop { offset: 0.0, color: Rgba::hex(0xffffff) }, GradientStop { offset: 1.0, color: Rgba::hex(0x000000) }] }).with_stroke(Rgba::hex(0xffff00), 2.0)),
    });
    let mut mask = Layer::new(LayerId(6), "mask");
    mask.kind = LayerKind::Mask;
    mask.keyframes = vec![kf(0, 30, vec![rect(5005, 400.0, 0.0, 20.0, 100.0, 0)], vec![], "")];
    let mut masked = Layer::new(LayerId(7), "masked");
    masked.parent = Some(LayerId(6));
    masked.keyframes = vec![kf(0, 30, vec![inst(5006, 300, "", Affine::translate(395.0, 30.0), vec![])], vec![], "")];
    d.timeline_mut(0).layers = vec![
        layer(1, vec![
            kf(0, 3, vec![], vec![Action::Trace { message: "start".into() }], ""),
            kf(3, 7, vec![], vec![Action::Stop { target: Target::new("ghost") }, Action::Stop { target: Target::new("panel") }], "mid"),
            kf(10, 20, vec![], vec![], ""),
            kf(29, 1, vec![], vec![Action::GotoAndPlay { target: this(), frame: FrameRef::Label("back".into()), scene: None }], ""),
        ]),
        layer(2, vec![kf(0, 30, vec![inst(5001, 200, "knob", Affine::translate(100.0, 100.0), knob_events)], vec![], "")]),
        layer(3, vec![kf(0, 30, vec![inst(5002, 100, "door", Affine::translate(300.0, 100.0), vec![]), inst(5007, 400, "panel", Affine::translate(300.0, 240.0), vec![]), Arc::new(faded), gradient], vec![], "")]),
        Arc::new(mask),
        Arc::new(masked),
    ];
    // Scene 2 has the "back" label and returns to the first scene's "mid".
    let mut two = Scene::new("Two");
    two.timeline.layers = vec![layer(8, vec![kf(0, 2, vec![rect(6001, 0.0, 0.0, 640.0, 360.0, 0x112233)], vec![], "back"), kf(2, 1, vec![], vec![Action::GotoAndStop { target: this(), frame: FrameRef::Label("mid".into()), scene: Some("scene 1".into()) }], "")])];
    d.scenes.push(Arc::new(two));
    d.next_id = 10_000;
    d.sanitize();
    d
}

// ----- the JavaScript player in Boa --------------------------------------------------------------

/// Hit tests for the JS player, over the exported paths flattened in Rust (what `Path::contains`
/// and `distance_to_edge` test), in place of the browser's canvas.
const HOST_JS: &str = r#"
function xf(m, q) { return [m[0] * q[0] + m[2] * q[1] + m[4], m[1] * q[0] + m[3] * q[1] + m[5]]; }
var host = {
  inFill: function (p, m, x, y, eo) {
    var wind = 0, cross = 0;
    for (var line of (POLYS[p] || [])) {
      var pts = line.p.map(function (q) { return xf(m, q); });
      for (var i = 0; i < pts.length; i++) {
        var a = pts[i], b = pts[(i + 1) % pts.length];
        if ((a[1] <= y) !== (b[1] <= y)) {
          var t = (y - a[1]) / (b[1] - a[1]);
          if (a[0] + t * (b[0] - a[0]) > x) { cross++; wind += b[1] > a[1] ? 1 : -1; }
        }
      }
    }
    return eo ? cross % 2 === 1 : wind !== 0;
  },
  nearStroke: function (p, m, x, y, reach) {
    for (var line of (POLYS[p] || [])) {
      var pts = line.p.map(function (q) { return xf(m, q); });
      var n = line.c ? pts.length : pts.length - 1;
      for (var i = 0; i < n; i++) {
        var a = pts[i], b = pts[(i + 1) % pts.length];
        var dx = b[0] - a[0], dy = b[1] - a[1], l = dx * dx + dy * dy;
        var t = l > 0 ? Math.max(0, Math.min(1, ((x - a[0]) * dx + (y - a[1]) * dy) / l)) : 0;
        var ex = a[0] + t * dx - x, ey = a[1] + t * dy - y;
        if (Math.sqrt(ex * ex + ey * ey) <= reach) return true;
      }
    }
    return false;
  }
};
"#;

struct Js {
    ctx: Context,
}

impl Js {
    fn new(m: &Movie) -> Js {
        let mut ctx = Context::default();
        ctx.eval(Source::from_bytes(PLAYER_JS)).unwrap_or_else(|e| panic!("player.js: {e}"));
        ctx.eval(Source::from_bytes(HOST_JS)).unwrap();
        let polys: Vec<Value> = m.paths.iter().map(|p| Value::Array(p.flatten(&Affine::IDENTITY, 0.05).into_iter().map(|l| json!({"c": l.closed, "p": l.points.iter().map(|q| json!([q.x, q.y])).collect::<Vec<_>>()})).collect())).collect();
        let setup = format!("var POLYS = {}; var P = new AniCraft.Player({}, host);", Value::Array(polys), m.json);
        ctx.eval(Source::from_bytes(&setup)).unwrap_or_else(|e| panic!("setup: {e}"));
        Js { ctx }
    }

    fn eval(&mut self, code: &str) -> Value {
        let v = self.ctx.eval(Source::from_bytes(&format!("JSON.stringify({code})"))).unwrap_or_else(|e| panic!("{code}: {e}"));
        let s = v.to_string(&mut self.ctx).unwrap().to_std_string_escaped();
        serde_json::from_str(&s).unwrap_or_else(|e| panic!("{code}: {e}: {s}"))
    }
}

fn js_events(events: &[Event]) -> String {
    let list: Vec<Value> = events
        .iter()
        .map(|e| match e {
            Event::Move(p) => json!({"type": "move", "x": p.x, "y": p.y}),
            Event::Down(p) => json!({"type": "down", "x": p.x, "y": p.y}),
            Event::Up(p) => json!({"type": "up", "x": p.x, "y": p.y}),
            Event::Leave => json!({"type": "leave"}),
        })
        .collect();
    Value::Array(list).to_string()
}

fn ids(p: &[ElementId]) -> Value {
    Value::Array(p.iter().map(|e| json!(e.0)).collect())
}

/// The Rust state in the shape of the JS player's `snapshot()` (props compared apart).
fn snapshot(st: &PlayerState) -> Value {
    json!({
        "scene": st.scene, "frame": st.root.frame, "playing": st.root.playing,
        "clips": st.clips.iter().map(|(p, c)| json!([ids(p), c.frame, c.playing])).collect::<Vec<_>>(),
        "hover": st.hover.as_deref().map(ids), "pressed": st.pressed.as_deref().map(ids), "down": st.down,
    })
}

fn props_of_rust(st: &PlayerState) -> Vec<String> {
    let mut out = Vec::new();
    for (p, o) in &st.props {
        let f = |k: &str, v: Option<f32>| v.map(|v| format!("{}:{k}={:.3}", ids(p), v));
        out.extend([f("x", o.x), f("y", o.y), f("alpha", o.alpha), f("rotation", o.rotation), f("scale_x", o.scale_x), f("scale_y", o.scale_y)].into_iter().flatten());
        if let Some(v) = o.visible {
            out.push(format!("{}:visible={v}", ids(p)));
        }
    }
    out.sort();
    out
}

fn props_of_js(snap: &Value) -> Vec<String> {
    let mut out = Vec::new();
    for e in snap["props"].as_array().unwrap() {
        let path = &e[0];
        for (k, v) in e[1].as_object().unwrap() {
            out.push(match v {
                Value::Bool(b) => format!("{path}:{k}={b}"),
                v => format!("{path}:{k}={:.3}", v.as_f64().unwrap()),
            });
        }
    }
    out.sort();
    out
}

fn close(a: f64, b: f32, tol: f64) -> bool {
    (a - f64::from(b)).abs() <= tol
}

/// The JS player's display list against `evaluate_player` at the same state: the same items in
/// the same order, with the same paths, placement, paints, colour effects, clips and groups.
fn same_display_list(js: &mut Js, m: &Movie, doc: &Document, st: &PlayerState, when: &str) {
    let dl = evaluate_player(doc, st);
    let ours = js.eval("P.evaluate()");
    let items: Vec<&anicraft_doc::DisplayItem> = dl.items.iter().filter(|i| !i.guide).collect();
    let theirs = ours["items"].as_array().unwrap();
    assert_eq!(theirs.len(), items.len(), "{when}: item count");
    for (k, (a, b)) in theirs.iter().zip(&items).enumerate() {
        let p = a["p"].as_u64().unwrap() as usize;
        assert_eq!(*m.paths[p], *b.path, "{when}: item {k} path");
        let mj: Vec<f64> = a["m"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
        for (i, (x, y)) in mj.iter().zip(b.transform.m).enumerate() {
            assert!(close(*x, y, if i < 4 { 1e-3 } else { 1e-2 }), "{when}: item {k} transform {mj:?} vs {:?}", b.transform.m);
        }
        let fill = a["f"].as_i64().unwrap();
        assert_eq!(fill >= 0, b.fill.is_some(), "{when}: item {k} fill");
        if let (Some(Paint::Solid(c)), true) = (&b.fill, fill >= 0) {
            let s = &m.json["paints"][fill as usize]["s"];
            for (x, y) in s.as_array().unwrap().iter().zip([c.r, c.g, c.b, c.a]) {
                assert!(close(x.as_f64().unwrap(), y, 1e-3), "{when}: item {k} colour");
            }
        }
        assert_eq!(a["s"].is_array(), b.stroke.is_some(), "{when}: item {k} stroke");
        let c = &a["c"];
        for (i, y) in [b.color.alpha, b.color.tint.r, b.color.tint.g, b.color.tint.b, b.color.tint_amount].into_iter().enumerate() {
            assert!(close(c[i].as_f64().unwrap(), y, 1e-3), "{when}: item {k} colour effect {c} vs {:?}", b.color);
        }
        let idx = |v: &Value| v.as_u64().map(|x| x as u32);
        assert_eq!((idx(&a["clip"]), idx(&a["masks"]), idx(&a["group"])), (b.clip, b.masks, b.group), "{when}: item {k} clip, mask and group");
    }
    let clips: Vec<Option<u32>> = ours["clips"].as_array().unwrap().iter().map(|v| v.as_u64().map(|x| x as u32)).collect();
    assert_eq!(clips, dl.clip_parents, "{when}: clips");
    let groups = ours["groups"].as_array().unwrap();
    assert_eq!(groups.len(), dl.groups.len(), "{when}: groups");
    for (a, b) in groups.iter().zip(&dl.groups) {
        assert!(close(a["alpha"].as_f64().unwrap(), b.alpha, 1e-3), "{when}: group alpha");
        assert_eq!(a["blend"], b.blend.key(), "{when}: group blend");
        assert_eq!((a["parent"].as_u64().map(|x| x as u32), a["clip"].as_u64().map(|x| x as u32)), (b.parent, b.clip), "{when}: group nesting");
    }
}

/// Runs `doc` in both players through `script` (per step: events after the frame, and pointer
/// events routed between frames), comparing state, output and display list at every step.
fn compare(doc: &Document, script: &[(Vec<Event>, Vec<Event>)]) -> Vec<String> {
    let m = movie(doc, &HtmlOptions::default());
    let mut js = Js::new(&m);
    let r = start(doc, &mut NoScripts);
    let out = js.eval("P.start().output");
    assert_eq!(out, json!(r.output), "start: output");
    let mut lines = r.output.clone();
    let mut st = r.state;
    for (i, (between, after)) in script.iter().enumerate() {
        let when = format!("step {}", i + 1);
        if !between.is_empty() {
            let r = anicraft_doc::player::input(doc, &st, between, &mut NoScripts);
            assert_eq!(js.eval(&format!("P.input({}).output", js_events(between))), json!(r.output), "{when}: input output");
            lines.extend(r.output.iter().cloned());
            st = r.state;
        }
        let r = step(doc, &st, after, &mut NoScripts);
        assert_eq!(js.eval(&format!("P.step({}).output", js_events(after))), json!(r.output), "{when}: output");
        lines.extend(r.output.iter().cloned());
        st = r.state;
        let snap = js.eval("P.snapshot()");
        let mut lean = snap.clone();
        lean.as_object_mut().unwrap().remove("props");
        assert_eq!(lean, snapshot(&st), "{when}: state");
        assert_eq!(props_of_js(&snap), props_of_rust(&st), "{when}: properties");
        same_display_list(&mut js, &m, doc, &st, &when);
    }
    lines
}

fn at(x: f32, y: f32) -> Vec2 {
    Vec2::new(x, y)
}

// ----- the tests --------------------------------------------------------------------------------

#[test]
fn the_js_player_plays_the_fixture_frame_for_frame_like_the_rust_player() {
    let doc = fixture();
    let none = || (vec![], vec![]);
    let mut script: Vec<(Vec<Event>, Vec<Event>)> = (0..2).map(|_| none()).collect();
    // Hover the knob between frames (Over: the door fades), press and release on it (a click
    // opens both doors), wander off (Out), then click again while it is open.
    script.push((vec![Event::Move(at(100.0, 100.0))], vec![]));
    script.push((vec![Event::Down(at(101.0, 99.0))], vec![]));
    script.push((vec![], vec![Event::Up(at(102.0, 101.0))]));
    script.extend((0..6).map(|_| none()));
    script.push((vec![Event::Move(at(10.0, 10.0))], vec![]));
    script.push((vec![], vec![Event::Down(at(122.0, 122.0)), Event::Up(at(122.0, 122.0))]));
    script.push((vec![Event::Move(at(124.0, 76.0)), Event::Leave], vec![]));
    // On through the end of scene 1, scene 2 and back.
    script.extend((0..30).map(|_| none()));
    let lines = compare(&doc, &script);
    for want in ["start", "pressed", "door open", "panel frame 3", "Stop (ghost): no instance named “ghost” here"] {
        assert!(lines.iter().any(|l| l == want), "the fixture never wrote {want:?}: {lines:?}");
    }
}

#[test]
fn the_js_player_plays_the_interactive_sample_like_the_rust_player() {
    let doc = sample::interactive();
    let mut script: Vec<(Vec<Event>, Vec<Event>)> = vec![(vec![], vec![]); 3];
    script.push((vec![Event::Move(at(640.0, 400.0))], vec![]));
    script.push((vec![Event::Down(at(640.0, 400.0))], vec![Event::Up(at(640.0, 400.0))]));
    script.extend(vec![(vec![], vec![]); 40]);
    assert!(compare(&doc, &script).iter().any(|l| l == "The show starts"), "the Play button starts the show");
}

#[test]
fn the_js_player_matches_on_the_demo_and_on_go_to_loops() {
    // No actions: the movie loops; every movie clip and graphic follows.
    let demo = sample::demo();
    compare(&demo, &vec![(vec![], vec![]); 60]);
    // A frame that goes to itself and back runs into the budget, in both players alike.
    let mut d = Document::default();
    d.timeline_mut(0).layers = vec![layer(1, vec![
        kf(0, 1, vec![], vec![Action::GotoAndPlay { target: this(), frame: FrameRef::Number(1), scene: None }], ""),
        kf(1, 1, vec![], vec![Action::GotoAndPlay { target: this(), frame: FrameRef::Number(0), scene: None }, Action::SetProperty { target: this(), property: Property::X, value: 1.0 }, Action::NextFrame { target: Target::new("_root.nobody") }], ""),
    ])];
    d.sanitize();
    compare(&d, &vec![(vec![], vec![]); 3]);
}

#[test]
fn the_camera_and_layer_depth_come_across_and_buttons_answer_through_them() {
    // The fixture through a camera that pans, zooms, turns and tints over frames 1-20, with a
    // layer behind the stage (parallax): the knob is clicked where the camera shows it.
    let mut doc = fixture();
    let stage = doc.stage;
    let mut far = anicraft_doc::Camera::rest(&stage);
    far.center = Vec2::new(260.0, 150.0);
    far.zoom = 1.5;
    far.rotation = 0.2;
    far.tint = Rgba::hex(0x3366ff);
    far.tint_amount = 0.3;
    let rest = anicraft_doc::Camera::rest(&stage).to_element(ElementId(7001));
    let near = far.to_element(ElementId(7001));
    let mut cam = Layer::new(LayerId(70), "Camera");
    cam.kind = LayerKind::Camera;
    cam.keyframes = vec![tweened(kf(0, 20, vec![Arc::new(rest)], vec![], "")), kf(20, 10, vec![Arc::new(near)], vec![], "")];
    let tl = doc.timeline_mut(0);
    tl.layers.insert(0, Arc::new(cam));
    Arc::make_mut(&mut tl.layers[3]).depth = 200.0;
    doc.sanitize();
    let m = movie(&doc, &HtmlOptions::default());
    let first = &m.json["timelines"][0];
    assert!(m.json["contents"].as_array().unwrap().iter().any(|c| c["cc"].is_array()), "the camera's tint is baked");
    assert!(first["f"].as_array().unwrap().len() > 15, "the camera moves every frame");
    // Where the knob is on screen once the camera has arrived.
    let knob = far.view(&stage, 0.0).apply(Vec2::new(100.0, 100.0));
    let mut script: Vec<(Vec<Event>, Vec<Event>)> = vec![(vec![], vec![]); 22];
    script.push((vec![Event::Move(knob), Event::Down(knob)], vec![Event::Up(knob)]));
    script.extend(vec![(vec![], vec![]); 6]);
    let lines = compare(&doc, &script);
    assert!(lines.iter().any(|l| l == "door open"), "the click reached the knob through the camera: {lines:?}");
}

#[test]
fn the_page_holds_the_movie_and_the_player_and_nothing_can_close_its_scripts() {
    let doc = fixture();
    let e = to_html(&doc, &HtmlOptions { title: "Doors </script> & \"knobs\"".into(), ..Default::default() }).unwrap();
    assert!(e.html.starts_with("<!DOCTYPE html>"));
    assert!(e.html.contains("<title>Doors &lt;/script&gt; &amp; &quot;knobs&quot;</title>"));
    for js in [PLAYER_JS, CANVAS_JS] {
        assert!(!js.to_ascii_lowercase().contains("</script") && !js.contains("<!--"), "a script source would end its element");
    }
    // Exactly the three scripts' closing tags, and the data reads back as the movie.
    assert_eq!(e.html.matches("</script>").count(), 2);
    let start = e.html.find("id=\"anicraft-movie\">").unwrap() + "id=\"anicraft-movie\">".len();
    let end = start + e.html[start..].find("</script>").unwrap();
    let back: Value = serde_json::from_str(&e.html[start..end]).unwrap();
    assert_eq!(back, movie(&doc, &HtmlOptions::default()).json);
    assert_eq!(back["v"], FORMAT_VERSION);
    assert_eq!(back["scenes"].as_array().unwrap().len(), 2);
    assert_eq!((back["stage"]["w"].as_u64(), back["stage"]["fps"].as_f64()), (Some(640), Some(24.0)));
    assert_eq!(e.stats.data_bytes, end - start);
    assert!(e.report.is_empty(), "nothing lost: {:?}", e.report);
    // The page code needs a browser to run, but it must compile.
    let mut ctx = Context::default();
    ctx.eval(Source::from_bytes(&format!("new Function({})", json!(CANVAS_JS)))).unwrap_or_else(|e| panic!("canvas.js: {e}"));
}

#[test]
fn frames_are_baked_and_stored_once() {
    let doc = sample::demo();
    let m = movie(&doc, &HtmlOptions::default());
    let frames = doc.frame_count() as usize;
    assert_eq!(m.json["timelines"][0]["n"].as_u64(), Some(frames as u64));
    assert!(m.stats.frames >= frames && m.stats.contents <= m.stats.frames, "{:?}", m.stats);
    // A keyframe held over many frames is one stored frame.
    let mut still = Document::default();
    {
        let l = Arc::make_mut(&mut still.timeline_mut(0).layers[0]);
        l.keyframes = vec![kf(0, 50, vec![rect(9, 0.0, 0.0, 10.0, 10.0, 0xff0000)], vec![], "")];
    }
    let s = movie(&still, &HtmlOptions::default());
    assert_eq!((s.stats.frames, s.stats.contents, s.json["timelines"][0]["f"].clone()), (50, 1, json!([[0, 50]])));
    // Runs add up to each timeline's length.
    for tl in m.json["timelines"].as_array().unwrap() {
        let total: u64 = tl["f"].as_array().unwrap().iter().map(|r| r[1].as_u64().unwrap()).sum();
        assert_eq!(Some(total), tl["n"].as_u64());
    }
    // A still shape over many frames is one path; a tween moves its transform, not its path.
    assert!(m.stats.paths < 40, "{:?}", m.stats);
    // Unused library symbols stay out.
    let mut d = doc.clone();
    d.library.push(Arc::new(Symbol { id: SymbolId(999_999), name: "unused".into(), kind: SymbolKind::Graphic, linkage: None, timeline: Timeline::default() }));
    assert_eq!(movie(&d, &HtmlOptions::default()).json["symbols"], m.json["symbols"]);
}

#[test]
fn variable_width_strokes_become_filled_outlines() {
    use anicraft_doc::{ProfilePreset, Stroke};
    let mut doc = Document::default();
    let shape = Shape { path: Arc::new(Path::line(Vec2::new(10.0, 10.0), Vec2::new(200.0, 10.0))), fill: None, stroke: Some(Stroke::new(Rgba::hex(0x00ff00), 10.0).with_profile(ProfilePreset::TaperEnd.profile())), fill_rule: FillRule::NonZero, merge: false };
    let el = Element { id: ElementId(77), name: String::new(), transform: Affine::translate(5.0, 0.0), color: Default::default(), kind: ElementKind::Shape(shape) };
    Arc::make_mut(&mut doc.timeline_mut(0).layers[0]).keyframe_mut(0).unwrap().elements.push(Arc::new(el));
    let m = movie(&doc, &HtmlOptions::default());
    let items = &m.json["contents"][0]["l"][0]["e"];
    assert_eq!(items.as_array().unwrap().len(), 1);
    assert!(items[0]["f"].is_u64() && items[0]["s"].is_null(), "{items}");
    assert!(items[0]["m"].is_null(), "the outline is already in place");
}

#[test]
fn what_does_not_carry_across_is_reported() {
    let mut doc = fixture();
    let tl = doc.timeline_mut(0);
    let l = Arc::make_mut(&mut tl.layers[2]);
    let k = l.keyframe_mut(0).unwrap();
    if let ElementKind::Symbol { filters, blend, .. } = &mut Arc::make_mut(&mut k.elements[0]).kind {
        filters.push(anicraft_doc::Filter::Blur { blur_x: 4.0, blur_y: 4.0, quality: 1 });
        *blend = BlendMode::Subtract;
    }
    Arc::make_mut(&mut tl.layers[0]).keyframe_mut(1).unwrap().script = Some(FrameScript::Plugin(PluginScript { id: "org.example.menu".into(), entry: "main".into() }));
    let m = movie(&doc, &HtmlOptions::default());
    assert_eq!(m.report.get(FILTERS), Some(&1));
    assert_eq!(m.report.get(BLENDS), Some(&1));
    assert_eq!(m.report.get(PLUGIN_SCRIPTS), Some(&1));
    // The player says so when the frame script's keyframe is entered.
    let mut js = Js::new(&m);
    js.eval("P.start()");
    let lines: Vec<Value> = (0..3).map(|_| js.eval("P.step([]).output")).collect();
    assert!(lines[2].to_string().contains("Frame script org.example.menu · main not run"), "{lines:?}");
}

#[test]
fn bitmaps_and_sounds_are_embedded_once() {
    let mut doc = Document::default();
    let png = {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, 1, 1);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&[255, 0, 0, 255]).unwrap();
        out
    };
    let bitmap = anicraft_doc::Bitmap::from_png(anicraft_doc::BitmapId(50), "dot", png).unwrap();
    doc.bitmaps.push(Arc::new(bitmap));
    let wav = anicraft_doc::audio::wav::encode(8000, 1, &[0.0; 80], anicraft_doc::audio::wav::Bits::Int16);
    doc.sounds.push(Arc::new(anicraft_doc::Sound::from_data(anicraft_doc::SoundId(60), "beep", wav).unwrap()));
    let fill = Paint::Bitmap { bitmap: anicraft_doc::BitmapId(50), repeat: true, matrix: Affine::scale(2.0, 2.0) };
    let a = Element { id: ElementId(70), name: String::new(), transform: Affine::IDENTITY, color: Default::default(), kind: ElementKind::Shape(Shape::filled(Path::rect(0.0, 0.0, 10.0, 10.0), fill.clone())) };
    let b = Element { id: ElementId(71), kind: ElementKind::Shape(Shape::filled(Path::rect(20.0, 0.0, 10.0, 10.0), fill)), ..a.clone() };
    {
        let k = Arc::make_mut(&mut doc.timeline_mut(0).layers[0]).keyframe_mut(0).unwrap();
        k.elements = vec![Arc::new(a), Arc::new(b)];
        k.sound = Some(anicraft_doc::FrameSound::new(anicraft_doc::SoundId(60)));
    }
    let m = movie(&doc, &HtmlOptions::default());
    assert_eq!(m.stats.images, 1);
    assert!(m.json["images"][0].as_str().unwrap().starts_with("data:image/png;base64,iVBOR"));
    assert_eq!(m.json["paints"][0], json!({"b": 0, "rep": true, "m": [2, 0, 0, 2, 0, 0]}));
    assert_eq!(m.json["sounds"][0]["mime"], "audio/wav");
    assert_eq!(m.json["timelines"][0]["snd"]["0"], json!([[0, "event", 1, false, 1]]));
    // The player hands the frame's sounds to the page.
    let mut js = Js::new(&m);
    assert_eq!(js.eval("P.start().sounds"), json!([[0, "event", 1, false, 1, 1, null]]));
    let silent = movie(&doc, &HtmlOptions { sound: false, ..Default::default() });
    assert_eq!((silent.stats.sounds, silent.json["timelines"][0].get("snd")), (0, None));
}

#[test]
fn sounds_carry_their_envelopes_and_volumes_and_start_in_graphics_and_clips_as_in_the_player() {
    use anicraft_doc::{FrameSound, Keyframe, Layer, LayerId, SoundEffect, SoundId, SoundSync, Symbol, SymbolId, SymbolKind, Timeline};
    let mut doc = Document::default();
    doc.stage.fps = 8.0;
    let wav = anicraft_doc::audio::wav::encode(8000, 1, &[0.25; 4000], anicraft_doc::audio::wav::Bits::Int16);
    doc.sounds.push(Arc::new(anicraft_doc::Sound::from_data(SoundId(60), "beep", wav).unwrap()));
    let snd = |sync, effect| Some(FrameSound { sync, effect, ..FrameSound::new(SoundId(60)) });
    let key = |start, n, sound: Option<FrameSound>, elements: Vec<Arc<Element>>| {
        let mut k = Keyframe::new(start, n);
        k.sound = sound;
        k.elements = elements;
        Arc::new(k)
    };
    let layer = |id, keys, volume| {
        let mut l = Layer::new(LayerId(id), "L");
        l.keyframes = keys;
        l.volume = volume;
        Arc::new(l)
    };
    let inst = |id, symbol| Arc::new(Element { id: ElementId(id), name: String::new(), transform: Affine::IDENTITY, color: Default::default(), kind: ElementKind::Symbol { symbol: SymbolId(symbol), loop_mode: Default::default(), first_frame: 0, blend: Default::default(), filters: Vec::new(), events: Vec::new() } });
    // A graphic with a fading sound on its frame 1 (of 2), a clip with a left-channel one on its
    // frame 0 (of 3), both on a layer at half volume; a sound of the main timeline's own.
    doc.library.push(Arc::new(Symbol { linkage: None, id: SymbolId(20), name: "g".into(), kind: SymbolKind::Graphic, timeline: Timeline { layers: vec![layer(21, vec![key(0, 1, None, vec![]), key(1, 1, snd(SoundSync::Event, SoundEffect::FadeOut), vec![])], 1.0)] } }));
    doc.library.push(Arc::new(Symbol { linkage: None, id: SymbolId(30), name: "c".into(), kind: SymbolKind::MovieClip, timeline: Timeline { layers: vec![layer(31, vec![key(0, 3, snd(SoundSync::Stream, SoundEffect::LeftChannel), vec![])], 0.5)] } }));
    doc.timeline_mut(0).layers = vec![layer(1, vec![key(0, 6, snd(SoundSync::Event, SoundEffect::None), vec![])], 1.0), layer(2, vec![key(0, 6, None, vec![inst(40, 20), inst(41, 30)])], 0.5)];
    let m = movie(&doc, &HtmlOptions::default());
    // The fade's points span one play (half a second); the clip's span its 3 frames.
    let g_tl = m.json["symbols"].as_array().unwrap().iter().find(|s| s["name"] == "g").unwrap()["tl"].as_u64().unwrap() as usize;
    assert_eq!(m.json["timelines"][g_tl]["snd"]["1"], json!([[0, "event", 1, false, 1, 1, [[0, 1, 1], [0.5, 0, 0]]]]));
    let c_tl = m.json["symbols"].as_array().unwrap().iter().find(|s| s["name"] == "c").unwrap()["tl"].as_u64().unwrap() as usize;
    assert_eq!(m.json["timelines"][c_tl]["snd"]["0"], json!([[0, "stream", 1, false, 3, 0.5, [[0, 1, 0]]]]));
    // Frame by frame, the page starts what the Rust player starts, at the same volumes.
    let mut js = Js::new(&m);
    let mut r = anicraft_doc::player::start(&doc, &mut NoScripts);
    let mut ours = js.eval("P.start().sounds");
    for step in 0..6 {
        let theirs: Vec<Value> = r.sounds.iter().map(|s| json!([s.sound.sound.0 - 60, s.sound.sync.key(), f64::from(s.gain)])).collect();
        let page: Vec<Value> = ours.as_array().unwrap().iter().map(|s| json!([s[0], s[1], s[5].as_f64()])).collect();
        assert_eq!(page, theirs, "step {step}");
        r = anicraft_doc::player::step(&doc, &r.state, &[], &mut NoScripts);
        ours = js.eval("P.step([]).sounds");
    }
}

#[test]
fn hostile_documents_export_without_panicking() {
    let mut doc = Document::default();
    let bad = Element { id: ElementId(5), name: "x".into(), transform: Affine { m: [f32::NAN, 0.0, 0.0, 1.0, f32::INFINITY, 0.0] }, color: ColorEffect { alpha: f32::NAN, tint: Rgba::new(f32::NAN, 2.0, -1.0, 1.0), tint_amount: f32::INFINITY }, kind: ElementKind::Shape(Shape::filled(Path::rect(0.0, 0.0, f32::NAN, 1.0), Paint::Radial { center: Vec2::ZERO, radius: -5.0, stops: vec![GradientStop { offset: f32::NAN, color: Rgba::BLACK }] })) };
    let fine = Element { id: ElementId(6), transform: Affine::IDENTITY, ..bad.clone() };
    let missing = inst(7, 12345, "ghost", Affine::IDENTITY, vec![]);
    Arc::make_mut(&mut doc.timeline_mut(0).layers[0]).keyframe_mut(0).unwrap().elements = vec![Arc::new(bad), Arc::new(fine), missing];
    let e = to_html(&doc, &HtmlOptions::default()).unwrap();
    assert!(!e.html.contains("NaN") && !e.html.contains("inf"), "numbers stay finite");
    let m = movie(&doc, &HtmlOptions::default());
    assert_eq!(m.json["paths"][0], "", "a path with a NaN point is left empty");
    let mut js = Js::new(&m);
    js.eval("P.start()");
    js.eval("P.step([{type: 'move', x: 1, y: 1}])");
    // An empty document is a one-frame movie.
    let m = movie(&Document::default(), &HtmlOptions::default());
    assert_eq!(m.json["timelines"][0]["n"], 1);
}
