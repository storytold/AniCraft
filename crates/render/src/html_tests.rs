use super::*;
use anicraft_doc::actions::{ButtonEvent, EventHandler, PluginScript, Property, Target};
use anicraft_doc::player::{Event, NoScripts, PlayerState, start, step};
use anicraft_doc::{ColorEffect, ColorTransform, ElementId, GradientStop, Keyframe, LayerId, LoopMode, Scene, Shape, Symbol, Tween, evaluate_player, sample};
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
    Arc::new(Symbol { id: SymbolId(id), name: format!("S{id}"), kind, linkage: None, scale_grid_kept: None, scale_grid: None, timeline: Timeline { layers } })
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
    red.color = ColorEffect::Brightness(0.2).with_alpha(0.8);
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
    // Advanced, with an alpha offset (it stays on each item: no group fade).
    faded.color = ColorEffect::Advanced(ColorTransform { mul: [0.4, 0.4, 0.4, 0.5], add: [0.0, 0.6, 0.6, 0.1] });
    let gradient = Arc::new(Element {
        id: ElementId(5004),
        name: String::new(),
        transform: Affine::translate(20.0, 300.0),
        color: ColorEffect::Tint { color: Rgba::hex(0xff00ff), amount: 0.25 },
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
            Event::KeyDown(k) => json!({"type": "keydown", "code": k}),
            Event::KeyUp(k) => json!({"type": "keyup", "code": k}),
            Event::Char(c) => json!({"type": "char", "text": c.to_string()}),
            Event::SelectAll => json!({"type": "selectall"}),
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
        "listeners": st.listeners.iter().filter(|(_, l)| !l.is_empty()).map(|(p, l)| json!([ids(p), l.iter().map(|x| json!([x.event.key(), ids(&x.owner), x.function])).collect::<Vec<_>>()])).collect::<Vec<_>>(),
        "stage": st.stage.iter().map(|x| json!([x.event.key(), ids(&x.owner), x.function])).collect::<Vec<_>>(),
        "vars": st.vars.iter().map(|(p, v)| json!([ids(p), v.iter().map(|(k, v)| json!([k, format!("{}:{}", kind(v), v.text())])).collect::<Vec<_>>()])).collect::<Vec<_>>(),
        "hover": st.hover.as_deref().map(ids), "pressed": st.pressed.as_deref().map(ids), "down": st.down,
        "fields": st.fields.iter().map(|(p, f)| json!([ids(p), f.text, f.caret, f.anchor])).collect::<Vec<_>>(),
        "focus": st.focus.as_deref().map(ids), "selecting": st.selecting, "shift": st.shift,
    })
}

/// A script value's kind, as the JS snapshot tags it.
fn kind(v: &anicraft_doc::actions::as3::ir::Value) -> &'static str {
    use anicraft_doc::actions::as3::ir::Value as V;
    match v {
        V::Undefined => "u",
        V::Null => "null",
        V::Bool(_) => "b",
        V::Num(_) => "n",
        V::Str(_) => "s",
    }
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
        for (i, y) in b.color.mul.into_iter().chain(b.color.add).enumerate() {
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
    d.library.push(Arc::new(Symbol { id: SymbolId(999_999), name: "unused".into(), kind: SymbolKind::Graphic, linkage: None, scale_grid_kept: None, scale_grid: None, timeline: Timeline::default() }));
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
fn stroke_caps_joins_and_dashes_reach_the_page() {
    use anicraft_doc::{Cap, Dash, Join, Stroke, StrokeStyle};
    let mut doc = Document::default();
    let style = StrokeStyle { cap: Cap::Butt, join: Join::Miter, miter_limit: 6.0, dash: Some(Dash::Dashed { lengths: vec![4.0, 1.5].into(), offset: 1.0 }) };
    let line = |y: f32, st: Stroke| Shape { path: Arc::new(Path::line(Vec2::new(10.0, y), Vec2::new(200.0, y))), fill: None, stroke: Some(st), fill_rule: FillRule::NonZero, merge: false };
    let shapes = [line(10.0, Stroke::new(Rgba::BLACK, 3.0)), line(30.0, Stroke::new(Rgba::BLACK, 3.0).with_style(style)), line(50.0, Stroke::new(Rgba::BLACK, 3.0).with_style(StrokeStyle { dash: Dash::dotted(2.0), ..StrokeStyle::default() }))];
    for (i, sh) in shapes.into_iter().enumerate() {
        let el = Element { id: ElementId(70 + i as u64), name: String::new(), transform: Affine::scale(2.0, 2.0), color: Default::default(), kind: ElementKind::Shape(sh) };
        Arc::make_mut(&mut doc.timeline_mut(0).layers[0]).keyframe_mut(0).unwrap().elements.push(Arc::new(el));
    }
    let m = movie(&doc, &HtmlOptions::default());
    let items = &m.json["contents"][0]["l"][0]["e"];
    // Round and solid: no style entry, as before.
    assert!(items[0]["s"].is_array() && items[0].get("k").is_none(), "{items}");
    // Lengths stay in local units; the page scales them like the width.
    assert_eq!(items[1]["k"], json!({"c": "butt", "j": "miter", "ml": 6, "d": [4, 1.5], "o": 1}));
    assert_eq!(items[2]["k"], json!({"c": "round", "j": "round", "ml": 3, "d": [0, 5], "o": 0}), "dots are round zero-length dashes");
    // The player hands the style to the page with each item.
    let mut js = Js::new(&m);
    js.eval("P.start()");
    let ours = js.eval("P.evaluate()");
    let theirs = ours["items"].as_array().unwrap();
    assert!(theirs[0]["k"].is_null());
    assert_eq!(theirs[1]["k"], items[1]["k"]);
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
    // A 9-slice symbol, scaled: the page scales it as a whole (ADR 0055).
    let ElementKind::Symbol { symbol: sym, .. } = doc.timeline(0).layers[2].keyframes[0].elements[0].kind else { panic!("the fixture's third layer holds an instance") };
    if let Some(x) = doc.library.iter_mut().find(|x| x.id == sym) {
        Arc::make_mut(x).scale_grid = Some(anicraft_doc::ScaleGrid { left: 1.0, right: 2.0, top: 1.0, bottom: 2.0 });
    }
    let m = movie(&doc, &HtmlOptions::default());
    assert_eq!(m.report.get(NINE_SLICE), None, "unscaled instances draw the same either way");
    let tl = doc.timeline_mut(0);
    Arc::make_mut(&mut Arc::make_mut(&mut tl.layers[2]).keyframe_mut(0).unwrap().elements[0]).transform = Affine::scale(2.0, 1.0);
    let m = movie(&doc, &HtmlOptions::default());
    assert_eq!(m.report.get(FILTERS), Some(&1));
    assert_eq!(m.report.get(NINE_SLICE), Some(&1));
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
    // A bitmap without Allow Smoothing says so; the page draws it with hard pixel edges, and
    // an SVG frame marks its image pixelated (ADR 0050).
    if let Some(b) = doc.bitmaps.first_mut() {
        Arc::make_mut(b).smoothing = false;
    }
    let hard = movie(&doc, &HtmlOptions::default());
    assert_eq!(hard.json["paints"][0], json!({"b": 0, "rep": true, "m": [2, 0, 0, 2, 0, 0], "px": true}));
    let svg = crate::svg::to_svg(&anicraft_doc::evaluate(&doc, 0), &doc.stage);
    assert!(svg.contains("image-rendering=\"pixelated\""), "{svg}");
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
    doc.library.push(Arc::new(Symbol { linkage: None, scale_grid_kept: None, scale_grid: None, id: SymbolId(20), name: "g".into(), kind: SymbolKind::Graphic, timeline: Timeline { layers: vec![layer(21, vec![key(0, 1, None, vec![]), key(1, 1, snd(SoundSync::Event, SoundEffect::FadeOut), vec![])], 1.0)] } }));
    doc.library.push(Arc::new(Symbol { linkage: None, scale_grid_kept: None, scale_grid: None, id: SymbolId(30), name: "c".into(), kind: SymbolKind::MovieClip, timeline: Timeline { layers: vec![layer(31, vec![key(0, 3, snd(SoundSync::Stream, SoundEffect::LeftChannel), vec![])], 0.5)] } }));
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
fn button_sounds_start_in_the_page_as_in_the_player() {
    use anicraft_doc::{FrameSound, Sound, SoundId, SoundSync};
    // A button at (100, 100), 10 × 10 art, a 100 × 100 Hit frame; a sound on each of its frames
    // (Over's a stream) on a layer at half volume; the button's own layer at half volume too.
    let mut d = Document::default();
    for (i, name) in ["up", "over", "down", "hit"].iter().enumerate() {
        let wav = anicraft_doc::audio::wav::encode(8000, 1, &[0.25; 800], anicraft_doc::audio::wav::Bits::Int16);
        d.sounds.push(Arc::new(Sound::from_data(SoundId(60 + i as u64), *name, wav).unwrap()));
    }
    let with = |k: Arc<Keyframe>, i: u64, sync| {
        let mut k = (*k).clone();
        k.sound = Some(FrameSound { sync, ..FrameSound::new(SoundId(60 + i)) });
        Arc::new(k)
    };
    let mut sounds = (*layer(34, vec![with(kf(0, 1, vec![], vec![], ""), 0, SoundSync::Event), with(kf(1, 1, vec![], vec![], ""), 1, SoundSync::Stream), with(kf(2, 1, vec![], vec![], ""), 2, SoundSync::Event), with(kf(3, 1, vec![], vec![], ""), 3, SoundSync::Event)])).clone();
    sounds.volume = 0.5;
    let art = || vec![rect(32, 0.0, 0.0, 10.0, 10.0, 0x336699)];
    let states = layer(31, vec![kf(0, 1, art(), vec![], ""), kf(1, 1, art(), vec![], ""), kf(2, 1, art(), vec![], ""), kf(3, 1, vec![rect(33, 0.0, 0.0, 100.0, 100.0, 0)], vec![], "")]);
    d.library.push(symbol(300, SymbolKind::Button, vec![Arc::new(sounds), states]));
    let mut holder = (*layer(2, vec![kf(0, 4, vec![inst(3, 300, "go", Affine::translate(100.0, 100.0), vec![])], vec![], "")])).clone();
    holder.volume = 0.5;
    d.timeline_mut(0).layers = vec![Arc::new(holder)];
    let m = movie(&d, &HtmlOptions::default());
    let names: Vec<String> = m.json["sounds"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap().to_string()).collect();
    assert_eq!(names.len(), 4, "every button sound is embedded");
    assert!(m.report.keys().all(|n| !n.contains("button")), "nothing reported: {:?}", m.report);
    let mut js = Js::new(&m);
    let mut r = start(&d, &mut NoScripts);
    js.eval("P.start()");
    let at = |x, y| Vec2::new(x, y);
    // Over, about, Down, Hit (released), Up (left), Over and Down at once, dragged off and back.
    let batches: Vec<Vec<Event>> = vec![
        vec![Event::Move(at(150.0, 150.0))],
        vec![Event::Move(at(160.0, 150.0))],
        vec![Event::Down(at(160.0, 150.0))],
        vec![Event::Up(at(160.0, 150.0))],
        vec![Event::Leave],
        vec![Event::Down(at(150.0, 150.0))],
        vec![Event::Move(at(400.0, 300.0)), Event::Move(at(150.0, 150.0)), Event::Up(at(150.0, 150.0))],
    ];
    let mut heard = Vec::new();
    for (i, events) in batches.iter().enumerate() {
        // Between frames and with a frame, alike.
        let (rs, page) = if i % 2 == 0 { (anicraft_doc::player::input(&d, &r.state, events, &mut NoScripts), js.eval(&format!("P.input({}).sounds", js_events(events)))) } else { (step(&d, &r.state, events, &mut NoScripts), js.eval(&format!("P.step({}).sounds", js_events(events)))) };
        let theirs: Vec<Value> = rs.sounds.iter().map(|s| json!([d.sound(s.sound.sound).unwrap().name, s.sound.sync.key(), f64::from(s.gain)])).collect();
        let ours: Vec<Value> = page.as_array().unwrap().iter().map(|s| json!([names[s[0].as_u64().unwrap() as usize], s[1], s[5].as_f64()])).collect();
        assert_eq!(ours, theirs, "batch {i}");
        heard.extend(theirs);
        r = rs;
    }
    let order: Vec<&str> = heard.iter().map(|h| h[0].as_str().unwrap()).collect();
    assert_eq!(order, ["over", "down", "hit", "up", "over", "down", "hit"]);
    assert!(heard.iter().all(|h| h[1] == "event" && h[2] == 0.25), "{heard:?}");
}

#[test]
fn hostile_documents_export_without_panicking() {
    let mut doc = Document::default();
    let bad = Element { id: ElementId(5), name: "x".into(), transform: Affine { m: [f32::NAN, 0.0, 0.0, 1.0, f32::INFINITY, 0.0] }, color: ColorEffect::Advanced(ColorTransform { mul: [f32::NAN, 2.0, -1.0, f32::INFINITY], add: [f32::NAN, 0.0, 0.0, 0.0] }), kind: ElementKind::Shape(Shape::filled(Path::rect(0.0, 0.0, f32::NAN, 1.0), Paint::Radial { center: Vec2::ZERO, radius: -5.0, stops: vec![GradientStop { offset: f32::NAN, color: Rgba::BLACK }] })) };
    let fine = Element { id: ElementId(6), transform: Affine::IDENTITY, ..bad.clone() };
    let missing = inst(7, 12345, "ghost", Affine::IDENTITY, vec![]);
    Arc::make_mut(&mut doc.timeline_mut(0).layers[0]).keyframe_mut(0).unwrap().elements = vec![Arc::new(bad), Arc::new(fine), missing];
    to_html(&doc, &HtmlOptions::default()).unwrap();
    let m = movie(&doc, &HtmlOptions::default());
    let data = m.json.to_string();
    assert!(!data.contains("NaN") && !data.contains("inf") && !data.contains("null,"), "numbers stay finite: {data}");
    assert_eq!(m.json["paths"][0], "", "a path with a NaN point is left empty");
    let mut js = Js::new(&m);
    js.eval("P.start()");
    js.eval("P.step([{type: 'move', x: 1, y: 1}])");
    // An empty document is a one-frame movie.
    let m = movie(&Document::default(), &HtmlOptions::default());
    assert_eq!(m.json["timelines"][0]["n"], 1);
}

/// `k` with a kept ActionScript 3.0 script in place of its actions.
fn as3(k: &mut Arc<Keyframe>, src: &str) {
    Arc::make_mut(k).script = Some(FrameScript::Source(Arc::new(anicraft_doc::actions::SourceScript::new(anicraft_doc::actions::Language::ActionScript3, src))));
}

/// The fixture with its frame actions and the knob's handlers written as ActionScript: the
/// main timeline's first frame adds the knob's listeners (named and inline functions,
/// `currentTarget`), a listener makes the panel clip clickable, one names an instance that
/// isn't there, the door's last frame calls `nextScene()` in its own script, and a script
/// outside the subset stays kept and unrun.
fn scripted() -> Document {
    let mut d = fixture();
    let main = "import flash.events.MouseEvent;\n\
        trace(\"start\");\n\
        knob.addEventListener(MouseEvent.CLICK, openDoor);\n\
        knob.addEventListener(MouseEvent.ROLL_OVER, function (e:MouseEvent):void { door.alpha = 0.5; });\n\
        knob.addEventListener(MouseEvent.ROLL_OUT, function (e:MouseEvent):void { door.alpha = 1; });\n\
        knob.addEventListener(MouseEvent.MOUSE_DOWN, pressed);\n\
        knob.addEventListener(MouseEvent.MOUSE_UP, released);\n\
        panel.addEventListener(MouseEvent.CLICK, function (e:MouseEvent):void { trace(\"panel \" + 1); MovieClip(e.currentTarget).gotoAndStop(1); e.currentTarget.alpha = 0.75; });\n\
        ghost.addEventListener(MouseEvent.CLICK, pressed);\n\
        navigateToURL(new URLRequest(\"https://example.com\"));\n\
        function openDoor(e:MouseEvent):void { door.gotoAndPlay(\"opening\"); panel.door.play(); }\n\
        function pressed(e:MouseEvent):void { trace(\"pressed\"); }\n\
        function released(e:MouseEvent):void { MovieClip(root).panel.gotoAndStop(100); }";
    let tl = d.timeline_mut(0);
    let l1 = Arc::make_mut(&mut tl.layers[0]);
    as3(&mut l1.keyframes[0], main);
    as3(&mut l1.keyframes[1], "ghost.stop()\npanel.stop()");
    as3(&mut l1.keyframes[2], "var n:int = 3; stop(); switch (n) { }");
    let l2 = Arc::make_mut(&mut tl.layers[1]);
    if let ElementKind::Symbol { events, .. } = &mut Arc::make_mut(&mut Arc::make_mut(&mut l2.keyframes[0]).elements[0]).kind {
        events.clear();
    }
    let door = Arc::make_mut(&mut d.library[0]);
    let dl = Arc::make_mut(&mut door.timeline.layers[0]);
    as3(&mut dl.keyframes[2], "stop();\ntrace('door open');\nnextScene();");
    d
}

#[test]
fn the_js_player_runs_actionscript_like_the_rust_player() {
    let doc = scripted();
    let m = movie(&doc, &HtmlOptions::default());
    assert_eq!(m.report.get(KEPT_SCRIPTS), Some(&1), "{:?}", m.report);
    assert_eq!(m.report.get(OUTSIDE_CALLS), Some(&1), "{:?}", m.report);
    let none = || (vec![], vec![]);
    let mut script: Vec<(Vec<Event>, Vec<Event>)> = (0..2).map(|_| none()).collect();
    script.push((vec![Event::Move(at(100.0, 100.0))], vec![]));
    script.push((vec![Event::Down(at(101.0, 99.0))], vec![]));
    script.push((vec![], vec![Event::Up(at(102.0, 101.0))]));
    script.extend((0..6).map(|_| none()));
    // Click the panel (a movie clip that a listener made clickable).
    script.push((vec![Event::Move(at(380.0, 300.0))], vec![Event::Down(at(380.0, 300.0)), Event::Up(at(380.0, 300.0))]));
    script.push((vec![Event::Move(at(10.0, 10.0)), Event::Leave], vec![]));
    script.extend((0..30).map(|_| none()));
    let lines = compare(&doc, &script);
    for want in ["start", "pressed", "door open", "panel 1", "Listen for click (ghost): no instance named “ghost” here", "Stop (ghost): no instance named “ghost” here", "Next scene: a movie clip's timeline has one scene; call it in the main timeline's script"] {
        assert!(lines.iter().any(|l| l == want), "the scripted fixture never wrote {want:?}: {lines:?}");
    }
}

#[test]
fn actionscript_plays_the_movie_its_hand_made_actions_play() {
    // The scripted fixture and the fixture write the same lines and reach the same states, but
    // for the lines only scripts write and the listeners they hold.
    let (a, b) = (fixture(), scripted());
    let events = [vec![], vec![], vec![Event::Move(at(100.0, 100.0))], vec![Event::Down(at(101.0, 99.0))], vec![Event::Up(at(102.0, 101.0))], vec![], vec![], vec![], vec![Event::Move(at(10.0, 10.0))]];
    let (mut x, mut y) = (start(&a, &mut NoScripts), start(&b, &mut NoScripts));
    let only_scripts = |l: &String| l.starts_with("Listen for") || l.starts_with("Next scene:");
    for (i, ev) in events.iter().enumerate() {
        x = step(&a, &x.state, ev, &mut NoScripts);
        y = step(&b, &y.state, ev, &mut NoScripts);
        let ys: Vec<&String> = y.output.iter().filter(|l| !only_scripts(l)).collect();
        assert_eq!(x.output.iter().collect::<Vec<_>>(), ys, "step {}: output", i + 1);
        let mut st = y.state.clone();
        st.listeners.clear();
        st.seq = 0;
        assert_eq!(x.state, st, "step {}: state", i + 1);
    }
}

/// A movie that uses the small-logic tier (ADR 0052): typed variables kept per
/// timeline and per clip instance, expressions, `if`, loops, `Math.random()`, functions from
/// another frame, property reads, enter-frame and keyboard listeners that add and remove
/// themselves, `e.target`, a runtime error and a loop that runs out of fuel.
fn logic() -> Document {
    let mut d = fixture();
    let main = "import flash.events.*;\n\
        var score:int = 0;\n\
        var label:String = \"go\";\n\
        var ticks:uint;\n\
        var total:Number = 0;\n\
        trace(\"start \" + score + \" \" + label + \" \" + ticks + \" \" + currentFrame + \"/\" + totalFrames + \" \" + currentLabel);\n\
        for (var i:int = 0; i < 5; i++) { if (i % 2 == 0) continue; total += i * 1.5; }\n\
        var k:int = 3; while (k > 0) { k--; } do { k += 2; } while (k < 5);\n\
        trace(total, k, Math.max(1, 7, 3), Math.min(), Math.round(-2.5), Math.abs(-0.25), int(\"42\") + uint(-1), 7 / 2, \"10\" < \"9\", 1 == \"1\", null == undefined);\n\
        trace(Math.floor(Math.random() * 100), Math.random() < 1, knob.x, knob.y, knob.alpha, knob.visible, knob.name, panel.currentFrame, panel.totalFrames);\n\
        addEventListener(Event.ENTER_FRAME, onTick);\n\
        stage.addEventListener(KeyboardEvent.KEY_DOWN, onKey);\n\
        stage.addEventListener(KeyboardEvent.KEY_UP, function (e:KeyboardEvent):void { trace(\"up \" + e.keyCode + \" \" + (e.currentTarget == stage)); });\n\
        knob.addEventListener(MouseEvent.CLICK, function (e:MouseEvent):void { score += 10; trace(\"click \" + e.target.name + \" \" + (e.currentTarget == knob) + \" \" + score); door.x = door.x + 0.1; trace(door.x); });\n\
        function onTick(e:Event):void {\n\
          ticks++;\n\
          if (ticks % 4 == 0) trace(\"tick \" + ticks + \" frame \" + currentFrame + \" mouse \" + mouseX + \",\" + mouseY);\n\
          if (ticks >= 12) { removeEventListener(Event.ENTER_FRAME, onTick); trace(\"stopped ticking\"); }\n\
        }\n\
        function onKey(e:KeyboardEvent):void {\n\
          if (e.keyCode == Keyboard.RIGHT) { knob.x += 5; } else if (e.keyCode == Keyboard.SPACE) { trace(\"space \" + bump(2)); } else { trace(\"key \" + e.keyCode); }\n\
          trace(\"knob at \" + knob.x + \" \" + label);\n\
        }";
    let tl = d.timeline_mut(0);
    let l1 = Arc::make_mut(&mut tl.layers[0]);
    as3(&mut l1.keyframes[0], main);
    as3(&mut l1.keyframes[1], "function bump(n:int):int { score += n; return score % 3 + 1; }\nlabel = label + \"!\";\ntrace(\"frame 2: \" + bump(1) + \" \" + label);\ntrace(ghost.x);\ntrace(\"never\");");
    as3(&mut l1.keyframes[2], "var spin:int = 0; while (true) { spin++; }");
    let l2 = Arc::make_mut(&mut tl.layers[1]);
    if let ElementKind::Symbol { events, .. } = &mut Arc::make_mut(&mut Arc::make_mut(&mut l2.keyframes[0]).elements[0]).kind {
        events.clear();
    }
    // The door clip keeps its own count across its loops.
    let door = Arc::make_mut(&mut d.library[0]);
    let dl = Arc::make_mut(&mut door.timeline.layers[0]);
    as3(&mut dl.keyframes[0], "var laps:int; laps++; if (laps > 1) trace(\"door lap \" + laps);");
    d
}

#[test]
fn the_js_player_runs_small_logic_like_the_rust_player() {
    let doc = logic();
    let m = movie(&doc, &HtmlOptions::default());
    assert_eq!(m.report.get(KEPT_SCRIPTS), None, "every script links: {:?}", m.report);
    assert!(m.json["timelines"][0]["fn"]["bump"].is_object() && m.json["timelines"][0]["vt"]["score"] == "int", "{}", m.json["timelines"][0]["vt"]);
    let none = || (vec![], vec![]);
    let mut script: Vec<(Vec<Event>, Vec<Event>)> = vec![none()];
    script.push((vec![Event::Move(at(100.0, 100.0))], vec![]));
    script.push((vec![Event::Down(at(101.0, 99.0))], vec![Event::Up(at(101.0, 99.0))]));
    script.push((vec![Event::KeyDown(39), Event::KeyUp(39)], vec![Event::KeyDown(65)]));
    script.push((vec![Event::KeyDown(32)], vec![]));
    script.extend((0..14).map(|_| none()));
    script.push((vec![Event::Move(at(10.0, 10.0)), Event::Leave], vec![Event::KeyDown(39)]));
    script.extend((0..20).map(|_| none()));
    let lines = compare(&doc, &script);
    for want in ["start 0 go 0 1/33 null", "6 6 7 Infinity -2 0.25 4294967337 3.5 true true true", "10 true 100 100 1 true knob 1 4", "frame 2: 2 go!", "Script on frame 4 of the main timeline stopped: line 4: no instance named “ghost” here", "stopped ticking", "up 39 true", "key 65"] {
        assert!(lines.iter().any(|l| l == want), "the logic fixture never wrote {want:?}: {lines:?}");
    }
    assert!(lines.iter().any(|l| l == "click knob true 11"), "{lines:?}");
    assert!(lines.iter().any(|l| l.ends_with("ran more than 100000 steps (an endless loop?)")), "{lines:?}");
    assert!(!lines.iter().any(|l| l == "never"));
}

#[test]
fn the_js_player_draws_random_numbers_like_the_rust_player() {
    // The seeded generator, number for number, at several seeds.
    for seed in [0u64, 1, 7, 0xdead_beef, u64::from(u32::MAX) + 5] {
        let mut d = Document::default();
        let k = Arc::make_mut(&mut Arc::make_mut(&mut d.timeline_mut(0).layers[0]).keyframes[0]);
        k.script = Some(FrameScript::Source(Arc::new(anicraft_doc::actions::SourceScript::new(anicraft_doc::actions::Language::ActionScript3, "for (var i:int = 0; i < 20; i++) trace(Math.random());"))));
        let m = movie(&d, &HtmlOptions::default());
        let rust = anicraft_doc::player::start_seeded(&d, seed, &mut NoScripts).output;
        let mut js = Js::new(&m);
        js.eval(&format!("(P = new AniCraft.Player({}, host, {seed}), 0)", m.json));
        assert_eq!(js.eval("P.start().output"), json!(rust), "seed {seed}");
    }
}

/// A text field element (ADR 0066): `text` in a `w` × `h` box at `at`, its outline a stand-in
/// (layout isn't compared: neither player lays text out here).
fn field_el(id: u64, name: &str, at: Vec2, (w, h): (f32, f32), text: &str, f: Option<anicraft_doc::TextField>) -> Arc<Element> {
    let n = text.chars().count() as f32;
    let t = anicraft_doc::TextBlock { text: text.into(), font: "Inter".into(), size: 14.0, bold: true, italic: false, color: Rgba::hex(0x112233), align: anicraft_doc::TextAlign::Center, letter_spacing: 0.5, line_spacing: 2.0, width: None, outline: Arc::new(Path::rect(2.0, 2.0, 8.0 * n, 12.0)), box_width: w, box_height: h, field: f };
    Arc::new(Element { id: ElementId(id), name: name.into(), transform: Affine::translate(at.x, at.y), color: ColorEffect::default(), kind: ElementKind::Text(Arc::new(t)) })
}

/// A movie with a dynamic field (`score_txt`, with a background and a border), an input field
/// (`name_txt`, at most 5 characters) and a movie clip (`door`): frame scripts set, append and
/// read text and length, name a field that isn't there, a clip and the main timeline as fields,
/// set text to null, give a field a playhead, and a keyboard listener copies what was typed into
/// the dynamic field until a key takes the movie to a frame without the fields.
fn fields_doc() -> Document {
    use anicraft_doc::{TextField, TextKind};
    let mut d = Document { stage: anicraft_doc::Stage { width: 200, height: 100, fps: 24.0, background: Rgba::hex(0xffffff) }, ..Document::default() };
    d.library = vec![symbol(100, SymbolKind::MovieClip, vec![layer(101, vec![kf(0, 1, vec![rect(1001, 0.0, 0.0, 20.0, 20.0, 0x884422)], vec![], "")])])];
    let mut score = TextField::new(TextKind::Dynamic);
    (score.background, score.background_color, score.border, score.border_color) = (true, Rgba::hex(0xffffee), true, Rgba::hex(0x333333));
    let mut name = TextField::new(TextKind::Input);
    (name.max_chars, name.border) = (5, true);
    let fields = vec![field_el(50, "score_txt", Vec2::new(10.0, 10.0), (100.0, 20.0), "Score", Some(score)), field_el(51, "name_txt", Vec2::new(10.0, 40.0), (120.0, 20.0), "", Some(name)), inst(52, 100, "door", Affine::translate(150.0, 60.0), vec![])];
    let mut scripts = layer(1, (0..8).map(|f| kf(f, 1, vec![], vec![], "")).collect());
    let main = "import flash.events.*;\n\
        score_txt.text = \"Score: \" + 1;\n\
        score_txt.appendText(\"0\");\n\
        trace(score_txt.text + \" \" + score_txt.length);\n\
        trace(\"[\" + name_txt.text + \"] \" + name_txt.length);\n\
        name_txt.x = 12;\n\
        trace(name_txt.x, name_txt.name);\n\
        stage.addEventListener(KeyboardEvent.KEY_DOWN, onKey);\n\
        function onKey(e:KeyboardEvent):void {\n\
          if (e.keyCode == 39) gotoAndStop(8);\n\
          else { score_txt.text = name_txt.text + \":\" + e.keyCode; trace(score_txt.text, name_txt.length); }\n\
        }";
    let frames = [main, "ghost_txt.text = \"x\";", "door.text = \"x\";", "trace(door.length);", "trace(root.text);", "score_txt.text = null;", "stop();\nscore_txt.gotoAndStop(1);\nscore_txt.text = \"a\\r\\nb\\rc\";\ntrace(score_txt.length);\ntrace(score_txt.currentFrame);", "trace(\"gone\");"];
    {
        let l = Arc::make_mut(&mut scripts);
        for (k, src) in l.keyframes.iter_mut().zip(frames) {
            as3(k, src);
        }
    }
    d.timeline_mut(0).layers = vec![scripts, layer(2, vec![kf(0, 7, fields, vec![], ""), kf(7, 1, vec![], vec![], "")])];
    d.sanitize();
    d
}

#[test]
fn the_js_player_runs_text_fields_like_the_rust_player() {
    let doc = fields_doc();
    let m = movie(&doc, &HtmlOptions::default());
    assert_eq!(m.report.get(KEPT_SCRIPTS), None, "every script runs: {:?}", m.report);
    let none = || (vec![], vec![]);
    let mut script: Vec<(Vec<Event>, Vec<Event>)> = (0..6).map(|_| none()).collect();
    let name = at(30.0, 50.0);
    // Click the input field, type at the end (past its limit), delete, select all and replace,
    // Tab (the only input field: it selects all), type again; then the dynamic field (it takes
    // the keyboard but not typing) and the stage (nothing has it).
    script.push((vec![Event::Down(name), Event::Up(name)], vec![]));
    script.push((vec![Event::Char('A'), Event::Char('n'), Event::Char('n')], vec![Event::KeyDown(65), Event::KeyUp(65)]));
    script.push((vec![Event::Char('a'), Event::Char('b'), Event::Char('c')], vec![Event::KeyDown(8)]));
    script.push((vec![Event::SelectAll, Event::Char('Z'), Event::KeyDown(13), Event::KeyUp(13)], vec![]));
    script.push((vec![Event::KeyDown(9)], vec![Event::Char('Q'), Event::KeyDown(65)]));
    script.push((vec![Event::Down(at(20.0, 15.0)), Event::Up(at(20.0, 15.0))], vec![Event::Char('x')]));
    script.push((vec![Event::Down(at(190.0, 95.0)), Event::Up(at(190.0, 95.0))], vec![Event::Char('y'), Event::KeyDown(66)]));
    // Right arrow: to a frame without the fields (their text and the focus go with them).
    script.push((vec![Event::Down(name), Event::Up(name)], vec![Event::KeyDown(39)]));
    script.push((vec![Event::KeyDown(65)], vec![]));
    script.extend((0..2).map(|_| none()));
    let lines = compare(&doc, &script);
    for want in [
        "Score: 10 9",
        "[] 0",
        "12 name_txt",
        "Script on frame 2 of the main timeline stopped: line 1: no instance named “ghost_txt” here",
        "Script on frame 3 of the main timeline stopped: line 1: “door” is a movie clip, not a text field",
        "Script on frame 4 of the main timeline stopped: line 1: “door” is a movie clip, not a text field",
        "Script on frame 5 of the main timeline stopped: line 1: the main timeline isn't a text field",
        "Script on frame 6 of the main timeline stopped: line 1: `text` is set to text, not null",
        "Go to frame 1 and stop (score_txt): “score_txt” is a text field; only movie clips and the main timeline have a playhead",
        "5",
        "Script on frame 7 of the main timeline stopped: line 5: “score_txt” is a text field; only movie clips and the main timeline have frames",
        "Ann:65 3",
        "Anna:8 4",
        // Enter types nothing in a one-line field; Tab (to the same field) selects all of it.
        "Z:13 1",
        "Z:9 1",
        "Q:65 1",
        "Q:66 1",
        "gone",
    ] {
        assert!(lines.iter().any(|l| l == want), "the fields movie never wrote {want:?}: {lines:?}");
    }
    assert!(lines.iter().any(|l| l.starts_with("Listener onKey stopped: line ") && l.ends_with("no instance named “name_txt” here")), "{lines:?}");
}

#[test]
fn text_fields_export_as_named_instances_and_static_text_as_before() {
    use anicraft_doc::{LineType, TextField, TextKind};
    let mut doc = Document::default();
    let mut pass = TextField::new(TextKind::Input);
    (pass.lines, pass.max_chars) = (LineType::Password, 5);
    let mut note = TextField::new(TextKind::Dynamic);
    (note.lines, note.selectable, note.background, note.border) = (LineType::Multiline, false, true, true);
    let els = vec![field_el(60, "", Vec2::new(5.0, 5.0), (40.0, 16.0), "Hi", None), field_el(61, "note_txt", Vec2::new(5.0, 30.0), (150.0, 40.0), "Hello", Some(note)), field_el(62, "pass_txt", Vec2::new(5.0, 80.0), (90.0, 18.0), "", Some(pass))];
    let ElementKind::Text(static_text) = &els[0].kind else { panic!("the first element is text") };
    let outline = static_text.outline.clone();
    Arc::make_mut(&mut doc.timeline_mut(0).layers[0]).keyframe_mut(0).unwrap().elements = els;
    let m = movie(&doc, &HtmlOptions::default());
    let items = &m.json["contents"][0]["l"][0]["e"];
    // Static text: a shape, its outline filled in its colour, as before.
    let s = &items[0];
    assert_eq!(s.as_object().unwrap().keys().collect::<Vec<_>>(), ["f", "m", "p"], "{s}");
    assert_eq!(m.json["paths"][s["p"].as_u64().unwrap() as usize], path_data(&outline));
    assert_eq!(m.json["paints"][s["f"].as_u64().unwrap() as usize], json!({"s": color(Rgba::hex(0x112233))}));
    // A field: an instance with its name, box, kind, text and font.
    let f = &items[1];
    assert_eq!((f["id"].as_u64(), f["n"].as_str(), &f["m"]), (Some(61), Some("note_txt"), &json!([1, 0, 0, 1, 5, 30])));
    let tf = &f["tf"];
    assert_eq!((tf["k"].as_str(), tf["ln"].as_str(), tf["w"].as_f64(), tf["h"].as_f64(), tf["sel"].as_bool(), tf["t"].as_str()), (Some("dynamic"), Some("multiline"), Some(150.0), Some(40.0), Some(false), Some("Hello")));
    assert_eq!(tf["font"], json!({"family": "Inter", "size": 14, "bold": true, "italic": false, "align": "center", "ls": 0.5, "lsp": 2}));
    assert_eq!((&tf["bd"], &tf["bk"]["j"]), (&json!([0, 0, 0, 1, 1]), &json!("miter")), "a 1 px mitred border");
    assert_eq!(m.json["paints"][tf["bg"].as_u64().unwrap() as usize], json!({"s": [1, 1, 1, 1]}));
    assert!(tf.get("mx").is_none());
    let p = &items[2]["tf"];
    assert_eq!((p["k"].as_str(), p["ln"].as_str(), p["mx"].as_u64(), p.get("bg"), p.get("bd")), (Some("input"), Some("password"), Some(5), None, None));
    assert_eq!(m.report.get(FIELD_FONTS), Some(&2), "{:?}", m.report);
    // The page's calls: the viewer types into the input field, the keyboard moves.
    let mut js = Js::new(&m);
    js.eval("P.start()");
    assert_eq!(js.eval("P.typed('62', 'Hello\\nWorld')"), json!("Hello"), "one line, at most 5 characters");
    assert_eq!(js.eval("[P.fieldText('62'), P.fieldText('61'), P.fieldText('60')]"), json!(["Hello", "Hello", null]));
    assert_eq!(js.eval("P.typed('61', 'x')"), Value::Null, "a dynamic field isn't typed into");
    assert_eq!(js.eval("[P.focus('62'), P.snapshot().focus, P.focus('61'), P.focus(null), P.snapshot().focus]"), json!([true, [62], false, false, null]));
    // The page hears each field as one, with its text, in place of its items.
    let seen = js.eval("(function () { var out = []; P.evaluate({ item: function () {}, field: function (f) { out.push([f.key, f.text, f.tf.k, f.items.length]); }, clipBegin: function () {}, clipEnd: function () {}, maskBegin: function () {}, maskEnd: function () {}, groupBegin: function () {}, groupEnd: function () {} }); return out; })()");
    assert_eq!(seen, json!([["61", null, "dynamic", 4], ["62", "Hello", "input", 1]]));
}
