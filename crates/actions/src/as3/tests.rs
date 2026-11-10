use super::*;
use crate::{Action, ButtonEvent, FrameRef, Property, Target};

fn ok(src: &str) -> Compiled {
    compile(src).unwrap_or_else(|p| panic!("{src:?} didn't compile: {p:?}"))
}

fn actions(src: &str) -> Vec<Action> {
    ok(src)
        .ops
        .into_iter()
        .map(|o| match o {
            ScriptOp::Action(a) => a,
            ScriptOp::Listen(l) => panic!("unexpected listener {l:?}"),
        })
        .collect()
}

/// The first problem: (line, column, message).
fn err(src: &str) -> (u32, u32, String) {
    match compile(src) {
        Ok(c) => panic!("{src:?} compiled to {c:?}"),
        Err(p) => {
            let f = p.first().cloned().unwrap_or_else(|| Problem::whole("none"));
            (f.line, f.column, f.message)
        }
    }
}

fn t(s: &str) -> Target {
    Target::new(s)
}

#[test]
fn timeline_calls_lower_to_actions() {
    let a = actions("stop();\nplay()\nthis.nextFrame();\nprevFrame();\nnextScene(); prevScene();");
    assert_eq!(a, vec![Action::Stop { target: t("") }, Action::Play { target: t("") }, Action::NextFrame { target: t("") }, Action::PrevFrame { target: t("") }, Action::NextScene, Action::PrevScene]);
    // Frames are 1-based in ActionScript, 0-based in the IR; strings are labels.
    let a = actions(r#"gotoAndStop(1); gotoAndPlay("run"); gotoAndPlay(12, "Scene 2"); gotoAndStop('end', null); gotoAndStop(2 + 3);"#);
    assert_eq!(
        a,
        vec![
            Action::GotoAndStop { target: t(""), frame: FrameRef::Number(0), scene: None },
            Action::GotoAndPlay { target: t(""), frame: FrameRef::Label("run".into()), scene: None },
            Action::GotoAndPlay { target: t(""), frame: FrameRef::Number(11), scene: Some("Scene 2".into()) },
            Action::GotoAndStop { target: t(""), frame: FrameRef::Label("end".into()), scene: None },
            Action::GotoAndStop { target: t(""), frame: FrameRef::Number(4), scene: None },
        ]
    );
}

#[test]
fn instance_paths_become_targets() {
    let a = actions("hero_mc.gotoAndStop(\"run\");\nthis.hero_mc.arm_mc.play();\nMovieClip(parent).stop();\nMovieClip(this.parent.parent).nextFrame();\nMovieClip(root).menu.play();\n(root as MovieClip).stop();\nhero_mc.root.stop();\nObject(parent).door.prevFrame();");
    let targets: Vec<String> = a.iter().map(|a| a.target().map(|t| t.0.clone()).unwrap_or_default()).collect();
    assert_eq!(targets, ["hero_mc", "hero_mc.arm_mc", "parent", "parent.parent", "root.menu", "root", "root", "parent.door"]);
    assert!(a.iter().all(|x| x.check().is_ok()));
}

#[test]
fn properties_are_set_to_values() {
    let a = actions("hero_mc.x = 100;\nhero_mc.y = -20.5;\nhero_mc.alpha = .5;\nhero_mc.visible = false;\ndoor.rotation = 45;\ndoor.scaleX = 2; door.scaleY = 1.5\nalpha = 0.25;");
    assert_eq!(
        a,
        vec![
            Action::SetProperty { target: t("hero_mc"), property: Property::X, value: 100.0 },
            Action::SetProperty { target: t("hero_mc"), property: Property::Y, value: -20.5 },
            Action::SetProperty { target: t("hero_mc"), property: Property::Alpha, value: 0.5 },
            Action::SetProperty { target: t("hero_mc"), property: Property::Visible, value: 0.0 },
            Action::SetProperty { target: t("door"), property: Property::Rotation, value: 45.0 },
            Action::SetProperty { target: t("door"), property: Property::ScaleX, value: 2.0 },
            Action::SetProperty { target: t("door"), property: Property::ScaleY, value: 1.5 },
            Action::SetProperty { target: t(""), property: Property::Alpha, value: 0.25 },
        ]
    );
}

#[test]
fn trace_writes_literals_and_their_sums_as_actionscript_does() {
    let msgs: Vec<String> = actions(r#"trace("hello"); trace('a' + 1 + 2); trace(1 + 2 + "a"); trace(0.1, true, null, undefined); trace(1e21, 1e-7, -0, 5 / 2, 7 % 4); trace(); trace("tab\there é \x41");"#)
        .into_iter()
        .map(|a| match a {
            Action::Trace { message } => message,
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(msgs, ["hello", "a12", "3a", "0.1 true null undefined", "1e+21 1e-7 0 2.5 3", "", "tab\there é A"]);
}

#[test]
fn listeners_lower_to_handlers_with_their_bodies() {
    let src = "import flash.events.MouseEvent;\n\nstop();\nplay_btn.addEventListener(MouseEvent.CLICK, onPlay);\n\nfunction onPlay(e:MouseEvent):void {\n\tgotoAndPlay(\"walk\"); // on\n\ttrace(\"go\");\n}\nmenu.btn.addEventListener(MouseEvent.ROLL_OVER, function (event:MouseEvent):void { MovieClip(event.currentTarget).gotoAndStop(2); });\naddEventListener(\"mouseDown\", onPlay);";
    let c = ok(src);
    assert_eq!(c.ops.len(), 4);
    assert_eq!(c.ops[0], ScriptOp::Action(Action::Stop { target: t("") }));
    let ScriptOp::Listen(l) = &c.ops[1] else { panic!("{:?}", c.ops[1]) };
    assert_eq!((l.target.0.as_str(), l.event, l.function.as_str()), ("play_btn", ButtonEvent::Click, "onPlay"));
    assert_eq!(*l.actions, vec![Action::GotoAndPlay { target: t(""), frame: FrameRef::Label("walk".into()), scene: None }, Action::Trace { message: "go".into() }]);
    let ScriptOp::Listen(l) = &c.ops[2] else { panic!() };
    // `currentTarget` is the instance listened to, named from the script's timeline.
    assert_eq!((l.target.0.as_str(), l.event, l.function.as_str()), ("menu.btn", ButtonEvent::Over, "function at line 10"));
    assert_eq!(*l.actions, vec![Action::GotoAndStop { target: t("menu.btn"), frame: FrameRef::Number(1), scene: None }]);
    let ScriptOp::Listen(l) = &c.ops[3] else { panic!() };
    assert_eq!((l.target.is_this(), l.event), (true, ButtonEvent::Press));
    assert!(l.describe().starts_with("On press (this): Go to “walk” and play"), "{}", l.describe());
    for (k, e) in [("MOUSE_UP", ButtonEvent::Release), ("ROLL_OUT", ButtonEvent::Out), ("MOUSE_OVER", ButtonEvent::Over), ("MOUSE_OUT", ButtonEvent::Out)] {
        let c = ok(&format!("b.addEventListener(MouseEvent.{k}, f, false, 0, true); function f(e) {{}}"));
        assert!(matches!(&c.ops[0], ScriptOp::Listen(l) if l.event == e), "{k}");
    }
}

#[test]
fn own_functions_run_where_they_are_called() {
    let a = actions("function go(n:int = 1):void { gotoAndStop(3); return; trace(\"never\"); }\ngo();\nfunction twice() { go(); go(); }\ntwice();");
    assert_eq!(a.len(), 3, "{a:?}");
    assert!(a.iter().all(|x| matches!(x, Action::GotoAndStop { frame: FrameRef::Number(2), .. })));
}

#[test]
fn calls_outside_the_movie_are_ignored_and_noted() {
    let c = ok("navigateToURL(new URLRequest(\"https://example.com\"), \"_blank\");\nflash.net.navigateToURL(x);\nExternalInterface.call(\"f\");\nfscommand(\"quit\");\nbtn.buttonMode = true;\nstop();");
    assert_eq!(c.ops, vec![ScriptOp::Action(Action::Stop { target: t("") })]);
    let lines: Vec<u32> = c.notes.iter().map(|n| n.line).collect();
    assert_eq!(lines, [1, 2, 3, 4, 5]);
    assert!(c.notes[0].message.contains("navigateToURL") && c.notes[0].message.contains("ignored"), "{:?}", c.notes);
}

#[test]
fn comments_white_space_and_semicolons_are_read_as_actionscript_does() {
    // Line and block comments, CR LF line ends, no semicolons, a BOM, empty statements.
    let a = actions("\u{feff}/* a\r\n block */ stop() // done\r\n;;\r\n{ play() }\r\n\"use strict\"");
    assert_eq!(a, vec![Action::Stop { target: t("") }, Action::Play { target: t("") }]);
    assert_eq!(err("stop() play()"), (1, 8, "expected `;` or a new line, found `play`".into()));
}

#[test]
fn every_problem_says_where_it_is() {
    let cases: &[(&str, u32, u32, &str)] = &[
        ("stop();\nif (x) { play(); }", 2, 1, "`if` statements aren't supported yet"),
        ("var n:int = 0;", 1, 1, "variables (`var`) aren't supported yet"),
        ("  for (var i = 0; i < 3; i++) {}", 1, 3, "`for` loops aren't supported yet"),
        ("while (true) stop();", 1, 1, "`while` loops aren't supported yet"),
        ("switch (x) { case 1: stop(); }", 1, 1, "`switch` statements aren't supported yet"),
        ("hero.x += 5;", 1, 1, "`+=` reads a value first"),
        ("hero.width = 5;", 1, 1, "setting `width` isn't supported yet"),
        ("score = 5;", 1, 1, "setting `score` isn't supported: variables"),
        ("foo();", 1, 1, "`foo` isn't declared in this frame script"),
        ("hero.addChild(x);", 1, 1, "calling `addChild` isn't supported"),
        ("gotoAndStop(0);", 1, 13, "a frame number is a whole number from 1"),
        ("gotoAndStop(1.5);", 1, 13, "a frame number is a whole number"),
        ("gotoAndStop(frame);", 1, 13, "reading `frame` isn't supported"),
        ("gotoAndStop();", 1, 1, "`gotoAndStop` takes a frame"),
        ("stop(1);", 1, 1, "`stop()` takes no arguments"),
        ("hero.nextScene();", 1, 1, "call `nextScene()` in the main timeline's own script"),
        ("trace(hero.x);", 1, 7, "reading properties isn't supported"),
        ("trace([1]);", 1, 7, "only values written out"),
        ("stage.addEventListener(MouseEvent.CLICK, f); function f(e) {}", 1, 1, "the stage isn't supported"),
        ("b.addEventListener(Event.ENTER_FRAME, f); function f(e) {}", 1, 20, "listening for Event.ENTER_FRAME isn't supported yet"),
        ("b.addEventListener(MouseEvent.CLICK, nope);", 1, 38, "`nope` isn't a function declared"),
        ("b.addEventListener(MouseEvent.CLICK, f); function f() { stop(); }", 1, 42, "a listener function takes the event"),
        ("b.addEventListener(MouseEvent.CLICK, f, true); function f(e) {}", 1, 41, "capture-phase listeners"),
        ("b.addEventListener(MouseEvent.CLICK, f); function f(e) { trace(e.target.name); }", 1, 64, "reading the event `e`"),
        ("b.addEventListener(MouseEvent.CLICK, f); function f(e) { e.target.play(); }", 1, 58, "`target` can be a part inside"),
        ("b.addEventListener(MouseEvent.CLICK, f); function f(e) { c.addEventListener(MouseEvent.CLICK, f); }", 1, 58, "adding a listener from inside a listener"),
        ("b.removeEventListener(MouseEvent.CLICK, f);", 1, 1, "removing listeners isn't supported yet"),
        ("function f() { f(); } f();", 1, 16, "`f` calls itself"),
        ("function stop() {}", 1, 1, "a function named `stop` would hide"),
        ("function f() {} function f() {}", 1, 17, "`f` is declared twice"),
        ("function f() { function g() {} } f();", 1, 16, "functions declared inside functions"),
        ("function f(n) { n.play(); } f(1);", 1, 17, "reading the parameter `n`"),
        ("hero_mc[\"arm\"].play();", 1, 1, "this isn't an instance"),
        ("hero_mc.x = \"10\";", 1, 13, "`x` is set to a number"),
        ("hero_mc.x = 1 / 0;", 1, 13, "the value must be a finite number"),
        ("package { }", 1, 1, "`package` belongs to class files"),
        ("class Hero {}", 1, 1, "`class` belongs to class files"),
        ("public function f() {}", 1, 1, "`public` declarations belong to classes"),
        ("stop(;", 1, 6, "expected an expression, found `;`"),
        ("stop(", 1, 6, "expected an expression, found the end of the script"),
        ("{ stop();", 1, 10, "expected `}` to close the block"),
        ("trace(\"open);", 1, 7, "this string is never closed"),
        ("/* never", 1, 1, "this comment is never closed"),
        ("trace(3x);", 1, 8, "a number can't run straight into a name"),
        ("stop(); #", 1, 9, "unexpected character '#'"),
        ("var re = /ab+c/;", 1, 10, "regular expressions aren't supported"),
        ("1 = 2;", 1, 3, "the left side of this assignment can't be assigned to"),
        ("x++;", 1, 1, "`++` and `--` read a value first"),
        ("é.play();", 1, 1, "`é` isn't an instance name AniCraft can address"),
        ("\tstop();\n\t\tif (a) {}", 2, 3, "`if` statements"),
    ];
    for (src, line, col, msg) in cases {
        let (l, c, m) = err(src);
        assert!(m.starts_with(msg) && (l, c) == (*line, *col), "{src:?}: got {l}:{c} {m:?}, want {line}:{col} {msg:?}");
    }
}

#[test]
fn every_problem_is_listed_and_nothing_half_runs() {
    let p = compile("stop();\nvar a = 1;\nif (a) {}\nplay();\nhero.width = 3;").unwrap_err();
    let at: Vec<(u32, u32)> = p.iter().map(|p| (p.line, p.column)).collect();
    assert_eq!(at, [(2, 1), (3, 1), (5, 1)]);
    assert_eq!(p[0].to_string(), "line 2, column 1: variables (`var`) aren't supported yet");
    let many: String = (0..200).map(|i| format!("var v{i} = {i};\n")).collect();
    assert_eq!(compile(&many).unwrap_err().len(), MAX_PROBLEMS);
}

#[test]
fn number_text_matches_actionscript() {
    for (n, s) in [(0.0, "0"), (-0.0, "0"), (5.0, "5"), (-3.25, "-3.25"), (0.1 + 0.2, "0.30000000000000004"), (1e21, "1e+21"), (1.5e-7, "1.5e-7"), (123456789012.0, "123456789012"), (f64::NAN, "NaN"), (f64::NEG_INFINITY, "-Infinity"), (0.000001, "0.000001")] {
        assert_eq!(lower::number_text(n), s, "{n}");
    }
}

#[test]
fn limits_are_problems_not_crashes() {
    let deep = format!("trace({}1{});", "(".repeat(10_000), ")".repeat(10_000));
    assert!(err(&deep).2.contains("nested more than"));
    let deep = format!("{}stop();{}", "{".repeat(10_000), "}".repeat(10_000));
    assert!(err(&deep).2.contains("nested more than"));
    let deep = format!("trace({}1);", "-".repeat(100_000));
    assert!(err(&deep).2.contains("nested more than"));
    let deep = format!("{}1;", "a = ".repeat(10_000));
    assert!(err(&deep).2.contains("nested more than"));
    let deep = format!("b.addEventListener(\"click\", {}", "function (e) { ".repeat(5_000));
    assert!(err(&deep).2.contains("nested more than"));
    let long = "stop();".repeat(MAX_RUN_BYTES / 7 + 1);
    assert!(err(&long).2.contains("longer than 256 KiB"));
    let tokens = ";".repeat(MAX_TOKENS + 1);
    assert!(err(&tokens).2.contains("tokens"));
    let steps = "stop();".repeat(crate::MAX_ACTIONS + 1);
    assert!(err(&steps).2.contains("at most 256 steps"));
    // Calls that fan out are bounded, however deep they could go.
    let mut src = String::from("function f0() { stop(); }\n");
    for i in 1..16 {
        src.push_str(&format!("function f{i}() {{ {} }}\n", format!("f{}(); ", i - 1).repeat(8)));
    }
    src.push_str("f15();");
    let t0 = std::time::Instant::now();
    assert!(compile(&src).is_err());
    assert!(t0.elapsed().as_secs_f32() < 5.0);
}

/// A small deterministic generator (no clock, no randomness crate).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn pick<'a>(&mut self, v: &[&'a str]) -> &'a str {
        v.get(self.next() as usize % v.len()).copied().unwrap_or("")
    }
}

#[test]
fn hostile_input_never_panics_and_ends_quickly() {
    const PIECES: [&str; 64] = [
        "stop", "play", "gotoAndStop", "(", ")", "{", "}", "[", "]", ";", ",", ".", "\n", " ", "\"a", "\"", "'", "\\", "/*", "*/", "//", "1", "0x", "1e", ".5", "e", "function", "f", "e:MouseEvent", ":void", "addEventListener", "MouseEvent.CLICK", "=", "+", "-", "!", "?", ":", "if", "else", "var", "for", "new", "this", "parent", "root", "MovieClip", "as", "trace", "x", "alpha", "é", "\u{0}", "\r", "\t", "<", ">", "...", "::", "..", ".<", "return", "import", "@",
    ];
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let t0 = std::time::Instant::now();
    let mut compiled = 0;
    for _ in 0..4_000 {
        let n = rng.next() as usize % 80;
        let src: String = (0..n).map(|_| rng.pick(&PIECES)).collect();
        if compile(&src).is_ok() {
            compiled += 1;
        }
    }
    // Raw bytes as text, too.
    for _ in 0..1_000 {
        let n = rng.next() as usize % 200;
        let bytes: Vec<u8> = (0..n).map(|_| rng.next() as u8).collect();
        let _ = compile(&String::from_utf8_lossy(&bytes));
    }
    assert!(compiled > 0, "some soup should compile (empty scripts at least)");
    assert!(t0.elapsed().as_secs_f32() < 20.0, "fuzzing took {:?}", t0.elapsed());
}

#[test]
fn compiles_once_per_script() {
    let s = crate::SourceScript::new(crate::Language::ActionScript3, "stop();");
    let a: *const crate::CompileResult = s.compiled();
    let b: *const crate::CompileResult = s.compiled();
    assert!(std::ptr::eq(a, b));
    assert!(s.runs());
    // A copy compiles on its own (its text may change); equality ignores the cache.
    let c = s.clone();
    assert_eq!(c, s);
    assert!(!crate::SourceScript::new(crate::Language::ActionScript1or2, "stop();").runs());
}
