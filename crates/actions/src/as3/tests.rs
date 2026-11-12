use super::ir::*;
use super::run::{self, Stop, World};
use super::*;
use crate::{Action, FrameRef, Property};
use std::collections::BTreeMap;

fn ok(src: &str) -> Compiled {
    compile(src).unwrap_or_else(|p| panic!("{src:?} didn't compile: {p:?}"))
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

/// A movie in miniature: one timeline with its variables and functions, instances with
/// properties, and a log of everything the script did.
#[derive(Default)]
struct Mock {
    decls: Decls,
    vars: BTreeMap<String, Value>,
    props: BTreeMap<(String, &'static str), f64>,
    log: Vec<String>,
    rng: u32,
    key: Option<f64>,
    frame: u32,
    /// Text fields' text, by name.
    texts: BTreeMap<String, String>,
}

impl World for Mock {
    fn act(&mut self, base: Base, a: &Action) -> Result<(), Option<String>> {
        if let Action::GotoAndStop { frame: FrameRef::Number(n), .. } = a {
            self.frame = *n;
        }
        self.log.push(format!("{base:?} {}", a.describe()));
        Ok(())
    }
    fn get(&mut self, at: &Place, p: Prop) -> Result<Value, String> {
        if at.path.0 == "ghost" {
            return Err("no instance named “ghost” here".into());
        }
        Ok(match p {
            Prop::CurrentFrame => Value::Num(f64::from(self.frame + 1)),
            Prop::Name => Value::str(&at.to_string()),
            Prop::Visible => Value::Bool(true),
            Prop::Text | Prop::Length => {
                let t = self.texts.get(&at.path.0).ok_or_else(|| format!("“{at}” isn't a text field"))?;
                if p == Prop::Text { Value::str(t) } else { Value::Num(text_len(t) as f64) }
            }
            p => Value::Num(self.props.get(&(at.path.0.clone(), p.name())).copied().unwrap_or(0.0)),
        })
    }
    fn set(&mut self, at: &Place, p: Property, v: f64) -> Result<(), Option<String>> {
        let name = Prop::from_name(match p {
            Property::X => "x",
            Property::Y => "y",
            Property::Alpha => "alpha",
            Property::Visible => "visible",
            Property::Rotation => "rotation",
            Property::ScaleX => "scaleX",
            Property::ScaleY => "scaleY",
        })
        .map_or("?", Prop::name);
        self.props.insert((at.path.0.clone(), name), v);
        Ok(())
    }
    fn set_text(&mut self, at: &Place, text: String) -> Result<(), Option<String>> {
        if !self.texts.contains_key(&at.path.0) {
            return Err(Some(format!("“{at}” isn't a text field")));
        }
        self.log.push(format!("text {at} = {text}"));
        self.texts.insert(at.path.0.clone(), text);
        Ok(())
    }
    fn trace(&mut self, line: String) {
        self.log.push(line);
    }
    fn read(&mut self, name: &str) -> Result<Value, String> {
        match self.vars.get(name) {
            Some(v) => Ok(v.clone()),
            None => self.decls.vars.get(name).map(|t| t.default_value()).ok_or_else(|| format!("`{name}` isn't defined")),
        }
    }
    fn write(&mut self, name: &str, v: Value) -> Result<(), String> {
        let t = *self.decls.vars.get(name).ok_or_else(|| format!("`{name}` isn't defined"))?;
        self.vars.insert(name.to_string(), t.coerce(v));
        Ok(())
    }
    fn function(&mut self, name: &str) -> Option<Arc<Function>> {
        self.decls.functions.get(name).cloned()
    }
    fn listen(&mut self, at: &Place, ev: ListenEvent, f: &Arc<Function>, add: bool) -> Result<(), Option<String>> {
        self.log.push(format!("{} {} {at} {}", if add { "listen" } else { "unlisten" }, ev.key(), f.name));
        Ok(())
    }
    fn random(&mut self) -> f64 {
        self.rng = self.rng.wrapping_mul(1_103_515_245).wrapping_add(12345);
        f64::from(self.rng >> 8) / f64::from(1u32 << 24)
    }
    fn same(&mut self, a: &Place, b: &Place) -> Result<bool, String> {
        Ok(a == b)
    }
    fn key_code(&mut self) -> Result<f64, String> {
        self.key.ok_or_else(|| "there's no keyboard event here".into())
    }
}

/// Compiles and runs `src` on a fresh timeline; the log and how the run ended.
fn exec(src: &str) -> (Vec<String>, Result<(), Stop>, Mock) {
    let c = ok(src);
    let mut m = Mock { decls: Decls::build([&c]), ..Mock::default() };
    assert!(m.decls.problems(&c).is_empty(), "{:?}", m.decls.problems(&c));
    let (_, r) = run::run_script(&mut m, &c.body, &c.inner, run::FUEL_PER_RUN);
    (std::mem::take(&mut m.log), r, m)
}

fn lines(src: &str) -> Vec<String> {
    let (log, r, _) = exec(src);
    assert_eq!(r, Ok(()), "{src}: {log:?}");
    log
}

#[test]
fn timeline_calls_run_as_actions() {
    let a = lines("stop();\nplay()\nthis.nextFrame();\nprevFrame();\nnextScene(); prevScene();\ngotoAndStop(1); gotoAndPlay(\"run\"); gotoAndPlay(12, \"Scene 2\"); gotoAndStop('end', null); gotoAndStop(2 + 3);");
    assert_eq!(a, ["This Stop", "This Play", "This Next frame", "This Previous frame", "This Next scene", "This Previous scene", "This Go to frame 1 and stop", "This Go to “run” and play", "This Go to frame 12 in “Scene 2” and play", "This Go to “end” and stop", "This Go to frame 5 and stop"]);
    let a = lines("hero_mc.gotoAndStop(\"run\");\nthis.hero_mc.arm_mc.play();\nMovieClip(parent).stop();\nMovieClip(this.parent.parent).nextFrame();\nMovieClip(root).menu.play();\n(root as MovieClip).stop();\nhero_mc.root.stop();\nObject(parent).door.prevFrame();");
    assert_eq!(a, ["This Go to “run” and stop (hero_mc)", "This Play (hero_mc.arm_mc)", "This Stop (parent)", "This Next frame (parent.parent)", "This Play (root.menu)", "This Stop (root)", "This Stop (root)", "This Previous frame (parent.door)"]);
}

#[test]
fn typed_variables_coerce_as_actionscript_does() {
    let a = lines(
        "var n:int = 7.9; var u:uint = -1; var num:Number = \"2.5\"; var b:Boolean = \"no\"; var s:String = 12; var z:String; var any = \"x\";\n\
         var big:int = 2147483647; big = big + 1;\n\
         var nan:Number; var none:int;\n\
         trace(n, u, num, b, s, z, any, big, nan, none);\n\
         n += 0.7; n++; ++n; trace(n, n++ + n, --n);",
    );
    assert_eq!(a, ["7 4294967295 2.5 true 12 null x -2147483648 NaN 0", "9 19 9"]);
}

#[test]
fn expressions_follow_ecmascript() {
    let a = lines(
        r#"trace(1 + 2 * 3 - 4 / 8, 7 % 3, -7 % 3, 1 / 0, -1 / 0, 0 / 0);
        trace("a" + 1 + 2, 1 + 2 + "a", "5" * "2", "x" - 1, true + 1, null + 1, undefined + 1);
        trace(1 < 2, "b" < "a", "10" < "9", 10 < "9", NaN < 1, NaN >= 1, 2 >= 2);
        trace(1 == "1", 1 === "1", null == undefined, null === undefined, 0 == false, "" == 0, NaN == NaN, "a" != "b");
        trace(0 || "x", 1 && "y", !0, !"", true ? "t" : "f", null || undefined);
        trace(int("12px"), int(" 0x1F "), Number(""), Number("1e3"), String(null), Boolean("false"), uint(-2), Number("Infinity"), Number("infinity"));
        trace(Math.min(3, 1, 2), Math.max(), Math.min(1, NaN), Math.abs(-4.5), Math.floor(-1.5), Math.ceil(1.2), Math.round(2.5), Math.round(-2.5), Math.round(-0.4), Math.PI > 3.14);"#,
    );
    assert_eq!(
        a,
        [
            "6.5 1 -1 Infinity -Infinity NaN",
            "a12 3a 10 NaN 2 1 NaN",
            "true false true false false false true",
            "true false true false true true false true",
            "x y true true t undefined",
            "0 31 0 1000 null true 4294967294 Infinity NaN",
            "1 -Infinity NaN 4.5 -2 2 3 -2 0 true",
        ]
    );
}

#[test]
fn if_else_and_loops_run_with_break_and_continue() {
    let a = lines(
        "var total:int = 0;\n\
         for (var i:int = 0; i < 10; i++) { if (i == 2) continue; if (i > 5) break; total += i; }\n\
         trace(total, i);\n\
         var k:int = 3; while (k > 0) { k--; } trace(k);\n\
         do { k++; } while (k < 2); trace(k);\n\
         do { k = 100; } while (false); trace(k);\n\
         if (k > 50) trace(\"big\"); else if (k > 5) trace(\"mid\"); else trace(\"small\");\n\
         for (;;) { break; } trace(\"out\");",
    );
    assert_eq!(a, ["13 6", "0", "2", "100", "big", "out"]);
}

#[test]
fn functions_take_parameters_return_values_and_recurse() {
    let a = lines(
        "function add(a:int, b:int = 10):int { return a + b; }\n\
         function fact(n:Number):Number { if (n <= 1) return 1; return n * fact(n - 1); }\n\
         function hello(who:String):void { trace(\"hi \" + who); return; trace(\"never\"); }\n\
         function untyped(x) { return x; }\n\
         trace(add(1, 2), add(5), add(1.9, 0.9), fact(5), untyped(\"u\"), hello(\"you\"));\n\
         function loopy():int { var s:int; for (var j:int = 0; j < 4; j++) s += j; return s; } trace(loopy(), loopy());",
    );
    assert_eq!(a, ["hi you", "3 15 1 120 u undefined", "6 6"]);
}

#[test]
fn properties_are_read_and_set() {
    let (log, r, m) = exec("hero.x = 10; hero.x += 5; hero.alpha = .5; trace(hero.x, hero.alpha, hero.name, currentFrame, this.currentFrame, hero.visible);\ngotoAndStop(4); trace(currentFrame);\nhero.rotation = hero.x * 2;");
    assert_eq!(r, Ok(()));
    assert_eq!(log, ["15 0.5 hero 1 1 true", "This Go to frame 4 and stop", "4"]);
    assert_eq!(m.props.get(&("hero".into(), "rotation")), Some(&30.0));
    // Reading an instance that isn't there stops the script there.
    let (log, r, _) = exec("trace(\"a\");\ntrace(ghost.x);\ntrace(\"b\");");
    assert_eq!(log, ["a"]);
    assert_eq!(r, Err(Stop::Error { line: 2, message: "no instance named “ghost” here".into() }));
}

#[test]
fn listeners_name_their_events_and_functions() {
    let c = ok("import flash.events.*;\nbtn.addEventListener(MouseEvent.CLICK, go);\naddEventListener(Event.ENTER_FRAME, tick);\nstage.addEventListener(KeyboardEvent.KEY_DOWN, function (e:KeyboardEvent):void { if (e.keyCode == Keyboard.LEFT) trace(\"left\"); });\nbtn.removeEventListener(MouseEvent.CLICK, go);\nfunction go(e:MouseEvent):void { MovieClip(e.currentTarget).gotoAndStop(2); trace(e.target.name); }\nfunction tick(e:Event):void {}\nhero.addEventListener(\"mouseOver\", go, false, 0, true);");
    assert_eq!(c.inner.len(), 1);
    assert_eq!(c.functions.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["go", "tick"]);
    let mut m = Mock { decls: Decls::build([&c]), ..Mock::default() };
    let (_, r) = run::run_script(&mut m, &c.body, &c.inner, run::FUEL_PER_RUN);
    assert_eq!(r, Ok(()));
    assert_eq!(m.log, ["listen click btn go", "listen enter_frame this tick", "listen key_down stage function at line 4, column 48", "unlisten click btn go", "listen over hero go"]);
    // The listener functions: the event's key code, target and current target.
    m.log.clear();
    m.key = Some(37.0);
    let key = c.inner[0].clone();
    assert_eq!(run::run_listener(&mut m, &key, 1000).1, Ok(()));
    let go = c.functions[0].clone();
    assert_eq!(run::run_listener(&mut m, &go, 1000).1, Ok(()));
    assert_eq!(m.log, ["left", "Current Go to frame 2 and stop", "e.target"]);
    // Keyboard listeners belong on the stage; elsewhere they are ignored and noted.
    let c = ok("addEventListener(KeyboardEvent.KEY_UP, f); function f(e) {}");
    assert!(c.body.is_empty() && c.notes[0].message.starts_with("keys go to the stage"), "{c:?}");
}

#[test]
fn names_from_other_frames_link_through_the_timeline() {
    let frame1 = ok("var score:int = 0;\nfunction addPoints(n:int):void { score += n; }\nfunction shout():String { return \"score \" + score; }");
    let frame5 = ok("addPoints(3);\nscore = score * 2;\ntrace(shout(), lives);");
    assert_eq!(frame5.free.iter().map(|f| (f.name.as_str(), f.call)).collect::<Vec<_>>(), [("addPoints", true), ("score", false), ("shout", true), ("lives", false)]);
    let d = Decls::build([&frame1, &frame5]);
    let p = d.problems(&frame5);
    assert_eq!(p.len(), 1, "{p:?}");
    assert_eq!((p[0].line, p[0].column), (3, 16));
    assert!(p[0].message.starts_with("`lives` isn't a variable of this timeline"), "{}", p[0].message);
    let ok5 = ok("addPoints(3);\nscore = score * 2;\ntrace(shout());");
    let d = Decls::build([&frame1, &ok5]);
    assert!(d.problems(&ok5).is_empty() && d.problems(&frame1).is_empty());
    let mut m = Mock { decls: d, ..Mock::default() };
    for c in [&frame1, &ok5] {
        assert_eq!(run::run_script(&mut m, &c.body, &c.inner, 10_000).1, Ok(()));
    }
    assert_eq!(m.log, ["score 6"]);
    // A function declared on two frames, or a variable with two types, is a problem.
    let again = ok("function shout():String { return \"again\"; }\nvar score:String;");
    let d = Decls::build([&frame1, &again]);
    assert_eq!(d.problems(&again).len(), 2, "{:?}", d.problems(&again));
}

#[test]
fn fuel_stops_an_endless_loop_cleanly() {
    let c = ok("trace(\"before\");\nwhile (true) {}\ntrace(\"after\");");
    let mut m = Mock::default();
    let (used, r) = run::run_script(&mut m, &c.body, &c.inner, 5_000);
    assert_eq!((used, r), (5_000, Err(Stop::Fuel)));
    assert_eq!(m.log, ["before"]);
    // Recursion is bounded by depth, text by length, and both stop the run.
    let (_, r, _) = exec("function f(n:int):int { return f(n + 1); } f(0);");
    assert!(matches!(r, Err(Stop::Error { ref message, .. }) if message.contains("more than 16 deep")), "{r:?}");
    let (_, r, _) = exec("var s:String = \"x\"; while (true) s = s + s;");
    assert!(matches!(r, Err(Stop::Error { ref message, .. }) if message.contains("longer than 16384")), "{r:?}");
    // Building text costs fuel for its length.
    let c = ok("var s:String = \"\"; for (var i:int = 0; i < 100000; i++) s = s + \"ab\";");
    let mut m = Mock { decls: Decls::build([&c]), ..Mock::default() };
    assert_eq!(run::run_script(&mut m, &c.body, &c.inner, run::FUEL_PER_RUN).1, Err(Stop::Fuel));
}

#[test]
fn random_comes_from_the_world() {
    let a = lines("var r:Number = Math.random(); trace(r >= 0 && r < 1, Math.floor(Math.random() * 6) + 1 <= 6);");
    assert_eq!(a, ["true true"]);
}

#[test]
fn calls_outside_the_movie_are_ignored_and_noted() {
    let c = ok("navigateToURL(new URLRequest(\"https://example.com\"), \"_blank\");\nflash.net.navigateToURL(x);\nExternalInterface.call(\"f\");\nfscommand(\"quit\");\nbtn.buttonMode = true;\nstop();");
    assert_eq!(c.body.len(), 1);
    let lines: Vec<u32> = c.notes.iter().map(|n| n.line).collect();
    assert_eq!(lines, [1, 2, 3, 4, 5]);
    assert!(c.notes[0].message.contains("navigateToURL") && c.notes[0].message.contains("ignored"), "{:?}", c.notes);
}

#[test]
fn comments_white_space_and_semicolons_are_read_as_actionscript_does() {
    let a = lines("\u{feff}/* a\r\n block */ stop() // done\r\n;;\r\n{ play() }\r\n\"use strict\"");
    assert_eq!(a, ["This Stop", "This Play"]);
    assert_eq!(err("stop() play()"), (1, 8, "expected `;` or a new line, found `play`".into()));
}

#[test]
fn every_problem_says_where_it_is() {
    let cases: &[(&str, u32, u32, &str)] = &[
        ("  for each (var v in list) {}", 1, 3, "`for each` loops aren't supported yet"),
        ("for (var k in o) {}", 1, 1, "`for in` loops aren't supported yet"),
        ("switch (x) { case 1: stop(); }", 1, 1, "`switch` statements aren't supported yet"),
        ("var a:Array = [];", 1, 7, "variables of type `Array` aren't supported yet"),
        ("var x:int = 1;", 1, 5, "`x` is already a name of the timeline"),
        ("hero.width = 5;", 1, 1, "setting `width` isn't supported yet"),
        ("currentFrame = 5;", 1, 1, "`currentFrame` can only be read"),
        ("hero.addChild(x);", 1, 1, "calling `addChild` isn't supported"),
        ("gotoAndStop(0);", 1, 13, "a frame number is a whole number from 1"),
        ("gotoAndStop(1.5);", 1, 13, "a frame number is a whole number"),
        ("gotoAndStop();", 1, 1, "`gotoAndStop` takes a frame"),
        ("stop(1);", 1, 1, "`stop()` takes no arguments"),
        ("hero.nextScene();", 1, 1, "call `nextScene()` in the main timeline's own script"),
        ("trace(hero.width);", 1, 7, "reading `width` isn't supported yet"),
        ("trace([1]);", 1, 7, "arrays aren't supported yet"),
        ("trace(stage.stageWidth);", 1, 7, "`stage.stageWidth` isn't supported yet"),
        ("stage.addEventListener(MouseEvent.CLICK, f); function f(e) {}", 1, 1, "mouse listeners on the stage"),
        ("b.addEventListener(Event.RESIZE, f); function f(e) {}", 1, 20, "listening for Event.RESIZE isn't supported yet"),
        ("b.addEventListener(MouseEvent.CLICK, f); function f() { stop(); }", 1, 42, "a listener function takes the event"),
        ("b.addEventListener(MouseEvent.CLICK, f, true); function f(e) {}", 1, 41, "capture-phase listeners"),
        ("b.addEventListener(MouseEvent.CLICK, f); function f(e:MouseEvent) { trace(e.stageX); }", 1, 75, "reading the event's `stageX`"),
        ("b.removeEventListener(MouseEvent.CLICK, function (e) {});", 1, 41, "a function written in place can't be removed"),
        ("function stop() {}", 1, 1, "a function named `stop` would hide"),
        ("function f() {} function f() {}", 1, 17, "`f` is declared twice"),
        ("function f() { function g() {} } f();", 1, 16, "functions declared inside functions"),
        ("function f(a:int, n) { n.play(); } f(1, 2);", 1, 24, "`n` is a variable, not an instance"),
        ("function f(a:int) {} f();", 1, 22, "`f` takes 1 argument, not 0"),
        ("function f(...rest) {}", 1, 15, "rest parameters"),
        ("var n:int; n.play();", 1, 12, "`n` is a variable, not an instance"),
        ("var s:String = \"ab\"; trace(s.length);", 1, 28, "reading `length` of a value"),
        ("hero_mc[\"arm\"].play();", 1, 1, "this isn't an instance"),
        ("trace(new Date());", 1, 7, "`new` isn't supported yet"),
        ("trace(1 & 2);", 1, 7, "the `&` operator isn't supported yet"),
        ("var n:int; n <<= 2;", 1, 12, "`<<=` isn't supported yet"),
        ("break;", 1, 1, "`break` is only for loops here"),
        ("trace(stop());", 1, 7, "`stop()` gives no value"),
        ("trace(Math.sqrt(4));", 1, 7, "`Math.sqrt` isn't supported yet"),
        ("trace(Keyboard.WIGGLE);", 1, 7, "`Keyboard.WIGGLE` isn't a key"),
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
        ("é.play();", 1, 1, "`é` isn't an instance name AniCraft can address"),
        ("\tstop();\n\t\twith (a) {}", 2, 3, "`with` statements"),
    ];
    for (src, line, col, msg) in cases {
        let (l, c, m) = err(src);
        assert!(m.starts_with(msg) && (l, c) == (*line, *col), "{src:?}: got {l}:{c} {m:?}, want {line}:{col} {msg:?}");
    }
}

#[test]
fn every_problem_is_listed_and_nothing_half_runs() {
    let p = compile("stop();\nvar a:Array;\nswitch (a) {}\nplay();\nhero.width = 3;").unwrap_err();
    let at: Vec<(u32, u32)> = p.iter().map(|p| (p.line, p.column)).collect();
    assert_eq!(at, [(2, 7), (3, 1), (5, 1)]);
    let many: String = (0..200).map(|i| format!("hero.w{i} = {i};\n")).collect();
    assert_eq!(compile(&many).unwrap_err().len(), MAX_PROBLEMS);
}

#[test]
fn number_text_matches_actionscript() {
    for (n, s) in [(0.0, "0"), (-0.0, "0"), (5.0, "5"), (-3.25, "-3.25"), (0.1 + 0.2, "0.30000000000000004"), (1e21, "1e+21"), (1.5e-7, "1.5e-7"), (123456789012.0, "123456789012"), (f64::NAN, "NaN"), (f64::NEG_INFINITY, "-Infinity"), (0.000001, "0.000001")] {
        assert_eq!(number_text(n), s, "{n}");
    }
    for (s, n) in [("", 0.0), ("  12 ", 12.0), ("1e3", 1000.0), (".5", 0.5), ("5.", 5.0), ("-0x10", f64::NAN), ("0x10", 16.0), ("0b101", 5.0), ("+Infinity", f64::INFINITY), ("inf", f64::NAN), ("1_0", f64::NAN), ("\u{a0}7\u{feff}", 7.0), ("1e", f64::NAN), ("-", f64::NAN)] {
        let v = string_number(s);
        assert!(v == n || (v.is_nan() && n.is_nan()), "{s:?}: {v}");
    }
    assert_eq!((to_int32(4_294_967_296.0 + 5.0), to_int32(-1.5), to_uint32(-1.0), to_int32(f64::NAN)), (5, -1, 4_294_967_295, 0));
    assert_eq!((run::round(0.49999999999999994), run::round(-0.5), run::round(1.5)), (0.0, 0.0, 2.0));
}

#[test]
fn the_ir_reads_and_writes_as_json() {
    let c = ok("var n:Number = NaN; var s:String = \"a\"; if (n != n) trace(s + 1, -Infinity); function f(e:MouseEvent = null):void {} btn.addEventListener(MouseEvent.CLICK, f);");
    let v = serde_json::to_value(&c).unwrap();
    let back: Compiled = serde_json::from_value(v.clone()).unwrap();
    assert_eq!(back, c);
    let text = v.to_string();
    assert!(text.contains("\"nan\"") && text.contains("{\"inf\":false}") && text.contains("\"s\":\"if\""), "{text}");
}

#[test]
fn limits_are_problems_not_crashes() {
    let deep = format!("trace({}1{});", "(".repeat(10_000), ")".repeat(10_000));
    assert!(err(&deep).2.contains("nested more than"));
    let deep = format!("{}stop();{}", "{".repeat(10_000), "}".repeat(10_000));
    assert!(err(&deep).2.contains("nested more than"));
    let deep = format!("trace({}1);", "-".repeat(100_000));
    assert!(err(&deep).2.contains("nested more than"));
    let deep = format!("var a; {}1;", "a = ".repeat(10_000));
    assert!(err(&deep).2.contains("nested more than"));
    // A long row of operators would make a deep tree; it counts as nesting.
    let long = format!("trace({}1);", "1 + ".repeat(10_000));
    let m = err(&long).2;
    assert!(m.contains("too many operators in a row") || m.contains("nested more than"), "{m}");
    let deep = format!("b.addEventListener(\"click\", {}", "function (e) { ".repeat(5_000));
    assert!(err(&deep).2.contains("nested more than"));
    let long = "stop();".repeat(MAX_RUN_BYTES / 7 + 1);
    assert!(err(&long).2.contains("longer than 256 KiB"));
    let tokens = ";".repeat(MAX_TOKENS + 1);
    assert!(err(&tokens).2.contains("tokens"));
}

/// The deepest code the limits allow runs in a small stack: calls 16 deep, each as nested as
/// the parser lets code be.
#[test]
fn the_deepest_allowed_code_fits_a_small_stack() {
    let nest = |depth: usize| format!("{}n{}", "(1 + ".repeat(depth), ")".repeat(depth));
    let mut src = String::from("function f0(n:int):int { return n; }\n");
    for i in 1..20 {
        src.push_str(&format!("function f{i}(n:int):int {{ return f{}({}); }}\n", i - 1, nest(18)));
    }
    src.push_str("trace(f19(0));");
    let c = ok(&src);
    let r = std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(move || {
            let mut m = Mock { decls: Decls::build([&c]), ..Mock::default() };
            run::run_script(&mut m, &c.body, &c.inner, run::FUEL_PER_RUN).1
        })
        .unwrap()
        .join()
        .unwrap();
    assert!(matches!(r, Err(Stop::Error { ref message, .. }) if message.contains("deep")), "{r:?}");
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
    const PIECES: [&str; 80] = [
        "stop", "play", "gotoAndStop", "(", ")", "{", "}", "[", "]", ";", ",", ".", "\n", " ", "\"a", "\"", "'", "\\", "/*", "*/", "//", "1", "0x", "1e", ".5", "e", "function", "f", "e:MouseEvent", ":void", "addEventListener", "MouseEvent.CLICK", "=", "+", "-", "!", "?", ":", "if", "else", "var", "for", "new", "this", "parent", "root", "MovieClip", "as", "trace", "x", "alpha", "é", "\u{0}", "\r", "\t", "<", ">", "...", "::", "..", ".<", "return", "import", "@", "while", "do", "break", "continue", "int", "n", "++", "+=", "==", "&&", "||", "Math.random()", "e.keyCode", "stage", "Event.ENTER_FRAME", "removeEventListener",
    ];
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let t0 = std::time::Instant::now();
    let mut compiled = 0;
    for _ in 0..5_000 {
        let n = rng.next() as usize % 80;
        let src: String = (0..n).map(|_| rng.pick(&PIECES)).collect();
        if let Ok(c) = compile(&src) {
            compiled += 1;
            // What compiles runs, within its fuel, whatever it is.
            let mut m = Mock { decls: Decls::build([&c]), ..Mock::default() };
            let _ = run::run_script(&mut m, &c.body, &c.inner, 2_000);
            for f in &c.functions {
                let _ = run::run_listener(&mut m, f, 2_000);
            }
        }
    }
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
    let c = s.clone();
    assert_eq!(c, s);
    assert!(!crate::SourceScript::new(crate::Language::ActionScript1or2, "stop();").runs());
}

#[test]
fn trace_lines_are_cut_at_whole_characters() {
    let long = "é".repeat(2000);
    let cut = run::trace_line(long);
    assert_eq!(cut.chars().count(), crate::MAX_TRACE_CHARS + 1);
    let emoji = format!("{}{}", "a".repeat(crate::MAX_TRACE_CHARS - 1), "😀".repeat(5));
    assert_eq!(run::trace_line(emoji).encode_utf16().count(), crate::MAX_TRACE_CHARS);
}

#[test]
fn a_full_budget_runs_quickly() {
    let c = ok("var n:int = 0; var s:Number = 0; while (true) { n++; s += Math.abs(n % 7 - 3) * 0.5; }");
    let mut m = Mock { decls: Decls::build([&c]), ..Mock::default() };
    let t0 = std::time::Instant::now();
    let (used, r) = run::run_script(&mut m, &c.body, &c.inner, run::FUEL_PER_RUN);
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    eprintln!("{used} fuel in {ms:.2} ms");
    assert_eq!(r, Err(Stop::Fuel));
    assert!(ms < 500.0, "{ms} ms");
}

#[test]
fn scripts_set_append_and_read_text_fields() {
    let src = r#"
        var n:int = 3;
        score_txt.text = "Score: " + n;
        score_txt.appendText("!");
        score_txt.text += "?";
        trace(score_txt.text, score_txt.length);
        panel.name_txt.text = String(score_txt.length * 2);
        var text:String = "a variable named text";
        trace(text);
    "#;
    let c = ok(src);
    let mut m = Mock { decls: Decls::build([&c]), ..Mock::default() };
    m.texts.insert("score_txt".into(), String::new());
    m.texts.insert("panel.name_txt".into(), "x".into());
    let (_, r) = run::run_script(&mut m, &c.body, &c.inner, run::FUEL_PER_RUN);
    assert_eq!(r, Ok(()));
    assert_eq!(m.texts["score_txt"], "Score: 3!?");
    assert_eq!(m.texts["panel.name_txt"], "20");
    assert!(m.log.contains(&"Score: 3!? 10".to_string()), "{:?}", m.log);
    assert!(m.log.contains(&"a variable named text".to_string()), "a bare `text` stays a variable");
    // Setting null stops the script with a message; `this.text` and the stage aren't fields.
    let (_, r, _) = {
        let c = ok("score_txt.text = null;");
        let mut m = Mock { decls: Decls::build([&c]), ..Mock::default() };
        m.texts.insert("score_txt".into(), String::new());
        let (_, r) = run::run_script(&mut m, &c.body, &c.inner, run::FUEL_PER_RUN);
        ((), r, ())
    };
    assert!(matches!(r, Err(Stop::Error { ref message, .. }) if message.contains("not null")), "{r:?}");
    assert!(err("this.text = 'x';").2.contains("isn't a text field"));
    assert!(err("stage.text = 'x';").2.contains("text field") || err("stage.text = 'x';").2.contains("stage"));
    assert!(err("a_txt.appendText();").2.contains("one argument"));
    assert!(err("a_txt.length = 3;").2.contains("can only be read"));
}
