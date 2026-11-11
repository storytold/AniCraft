//! Lowering: the syntax tree to the action IR. It accepts only what it can say exactly in the
//! IR and reports everything else where it is, so a script either runs whole or not at all.

use super::parse::{E, Expr, Func, S, Stmt};
use super::{Compiled, Listener, MAX_INLINE_DEPTH, MAX_LISTENERS, MAX_PROBLEMS, Pos, Problem, ScriptOp};
use crate::{Action, ButtonEvent, FrameRef, MAX_ACTIONS, MAX_FRAME, MAX_LABEL_CHARS, MAX_TRACE_CHARS, Property, Target, valid_instance_name};
use std::collections::BTreeMap;
use std::sync::Arc;

/// The timeline methods a script may call (on this timeline or on a named instance).
const METHODS: [&str; 8] = ["stop", "play", "gotoAndPlay", "gotoAndStop", "nextFrame", "prevFrame", "nextScene", "prevScene"];
/// Names a script's own function can't take: they would hide the timeline's methods.
const BUILTINS: [&str; 11] = ["stop", "play", "gotoAndPlay", "gotoAndStop", "nextFrame", "prevFrame", "nextScene", "prevScene", "trace", "addEventListener", "removeEventListener"];
/// Calls that would reach outside the movie: ignored, and noted.
const OUTSIDE: [&str; 4] = ["navigateToURL", "sendToURL", "fscommand", "fsCommand"];
/// Type casts that leave an instance path as it is (`MovieClip(parent)`).
const CASTS: [&str; 7] = ["MovieClip", "Object", "Sprite", "DisplayObject", "DisplayObjectContainer", "InteractiveObject", "SimpleButton"];

/// A constant value, as ActionScript computes it.
#[derive(Clone, Debug, PartialEq)]
enum Val {
    Num(f64),
    Str(String),
    Bool(bool),
    Null,
    Undefined,
}

impl Val {
    fn number(&self) -> f64 {
        match self {
            Val::Num(n) => *n,
            Val::Bool(b) => f64::from(u8::from(*b)),
            Val::Null => 0.0,
            Val::Undefined => f64::NAN,
            Val::Str(s) => {
                // ECMA-262 §9.3.1: a decimal number with optional sign and exponent (Rust would
                // also read "inf" and "nan").
                let t = s.trim();
                match t.trim_start_matches(['+', '-']) {
                    "" if t.is_empty() => 0.0,
                    "Infinity" => if t.starts_with('-') { f64::NEG_INFINITY } else { f64::INFINITY },
                    u if u.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-')) => t.parse().unwrap_or(f64::NAN),
                    _ => f64::NAN,
                }
            }
        }
    }

    fn text(&self) -> String {
        match self {
            Val::Num(n) => number_text(*n),
            Val::Str(s) => s.clone(),
            Val::Bool(b) => b.to_string(),
            Val::Null => "null".into(),
            Val::Undefined => "undefined".into(),
        }
    }
}

/// A number as ActionScript writes it (`Number.toString()`, ECMA-262 §9.8.1): integers without
/// a point, exponents outside 1e-6 … 1e21.
pub(super) fn number_text(n: f64) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    if n.is_infinite() {
        return if n > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if n == 0.0 {
        return "0".into();
    }
    let a = n.abs();
    if (1e-6..1e21).contains(&a) {
        return format!("{n}");
    }
    let s = format!("{n:e}");
    match s.split_once('e') {
        Some((m, e)) if !e.starts_with('-') => format!("{m}e+{e}"),
        _ => s,
    }
}

/// Where lowering is.
#[derive(Clone, Default)]
struct Cx {
    /// Inside a listener's function: its event parameter and the instance it listens on (what
    /// `e.currentTarget` is).
    event: Option<(String, Vec<String>)>,
    /// Inside a listener (or a function it calls): listeners can't be added there yet.
    in_listener: bool,
    /// Inside a function body (functions can't be declared there).
    in_function: bool,
    /// The parameters of the function being lowered (they can't be read).
    params: Vec<String>,
    /// The script's functions being lowered, innermost last (to refuse recursion).
    stack: Vec<String>,
}

struct Lower<'a> {
    funcs: BTreeMap<&'a str, &'a Func>,
    problems: Vec<Problem>,
    notes: Vec<Problem>,
    listeners: usize,
    /// Statements lowered so far, counting each inlined copy (calls can fan out).
    work: usize,
}

/// Most statements lowered for one script, inlined copies included.
const MAX_WORK: usize = 100_000;

/// Whether the statement list ended in a `return`.
type Returned = bool;

impl<'a> Lower<'a> {
    fn problem(&mut self, pos: Pos, msg: impl Into<String>) {
        if self.problems.len() < MAX_PROBLEMS * 4 {
            self.problems.push(Problem::at(pos, msg));
        }
    }

    fn note(&mut self, pos: Pos, msg: impl Into<String>) {
        if self.notes.len() < MAX_PROBLEMS * 4 {
            self.notes.push(Problem::at(pos, msg));
        }
    }

    fn stmts(&mut self, list: &'a [Stmt], cx: &Cx, out: &mut Vec<ScriptOp>) -> Returned {
        for s in list {
            if self.stmt(s, cx, out) {
                return true;
            }
        }
        false
    }

    fn stmt(&mut self, s: &'a Stmt, cx: &Cx, out: &mut Vec<ScriptOp>) -> Returned {
        self.work += 1;
        if self.work > MAX_WORK {
            if self.work == MAX_WORK + 1 {
                self.problem(s.pos, format!("the script's functions call each other into more than {MAX_WORK} statements"));
            }
            return true;
        }
        match &s.kind {
            S::Empty | S::Import => {}
            // Declared functions run where they are called or listened for.
            S::Function(_) => {
                if cx.in_function {
                    self.problem(s.pos, "functions declared inside functions aren't supported yet");
                }
            }
            S::Block(b) => return self.stmts(b, cx, out),
            S::Var(w) => self.problem(s.pos, format!("variables (`{w}`) aren't supported yet")),
            S::If(..) => self.problem(s.pos, "`if` statements aren't supported yet"),
            S::Other(w, _) => {
                let what = match *w {
                    "for" | "while" | "do" => format!("`{w}` loops aren't supported yet"),
                    _ => format!("`{w}` statements aren't supported yet"),
                };
                self.problem(s.pos, what);
            }
            S::Return(None) => return true,
            S::Return(Some(e)) => {
                if self.constant(e, cx).is_err() {
                    self.problem(e.pos, "returning a value isn't supported");
                }
                return true;
            }
            S::Expr(e) => self.expr_stmt(e, cx, out),
        }
        false
    }

    fn expr_stmt(&mut self, e: &'a Expr, cx: &Cx, out: &mut Vec<ScriptOp>) {
        match &e.kind {
            E::Call(callee, args) => self.call(e.pos, callee, args, cx, out),
            E::Assign("=", lhs, rhs) => self.assign(e.pos, lhs, rhs, cx, out),
            E::Assign(op, _, _) => self.problem(e.pos, format!("`{op}` reads a value first; only `=` with a value is supported")),
            E::Comma(list) => {
                for x in list {
                    self.expr_stmt(x, cx, out);
                }
            }
            // A literal alone does nothing (`"use strict"`).
            E::Num(_) | E::Str(_) | E::Bool(_) | E::Null => {}
            E::Unary("++" | "--", _) | E::Postfix(..) => self.problem(e.pos, "`++` and `--` read a value first and aren't supported"),
            _ => self.problem(e.pos, "only calls (such as `stop()`) and settings (such as `hero_mc.x = 10`) run as statements"),
        }
    }

    fn call(&mut self, pos: Pos, callee: &'a Expr, args: &'a [Expr], cx: &Cx, out: &mut Vec<ScriptOp>) {
        match &callee.kind {
            E::Name(n) => {
                let n = n.as_str();
                if let Some(f) = self.funcs.get(n).copied() {
                    return self.inline(pos, f, args, cx, out);
                }
                match n {
                    "trace" => self.trace(pos, args, cx, out),
                    "addEventListener" => self.listen(pos, Vec::new(), args, cx, out),
                    n if METHODS.contains(&n) => self.method(pos, Vec::new(), n, args, cx, out),
                    n if OUTSIDE.contains(&n) => self.note(pos, format!("`{n}` is ignored: a movie here can't open web pages or reach outside itself")),
                    "removeEventListener" => self.problem(pos, "removing listeners isn't supported yet"),
                    n if CASTS.contains(&n) => self.problem(pos, format!("`{n}(…)` alone does nothing; call a method on it, as in `{n}(parent).play()`")),
                    n => self.problem(pos, format!("`{n}` isn't declared in this frame script (functions from other frames, and the rest of the language, aren't supported yet)")),
                }
            }
            E::Member(obj, m) => {
                let m = m.as_str();
                if OUTSIDE.contains(&m) || (matches!(&obj.kind, E::Name(o) if o == "ExternalInterface") && matches!(m, "call" | "addCallback")) {
                    return self.note(pos, format!("`{m}` is ignored: a movie here can't open web pages or reach outside itself"));
                }
                let known = METHODS.contains(&m) || m == "addEventListener";
                if !known {
                    let msg = if m == "removeEventListener" { "removing listeners isn't supported yet".to_string() } else { format!("calling `{m}` isn't supported (timeline methods such as `play()` and `gotoAndStop()`, and `addEventListener`, are)") };
                    return self.problem(pos, msg);
                }
                let Some(t) = self.target(obj, cx) else { return };
                if m == "addEventListener" { self.listen(pos, t, args, cx, out) } else { self.method(pos, t, m, args, cx, out) }
            }
            _ => self.problem(pos, "only named functions and methods can be called"),
        }
    }

    /// A call to one of the script's own functions: its body runs in place.
    fn inline(&mut self, pos: Pos, f: &'a Func, args: &'a [Expr], cx: &Cx, out: &mut Vec<ScriptOp>) {
        let name = f.name.clone().unwrap_or_default();
        if cx.stack.contains(&name) {
            return self.problem(pos, format!("`{name}` calls itself; recursion isn't supported"));
        }
        if cx.stack.len() >= MAX_INLINE_DEPTH {
            return self.problem(pos, format!("functions call each other more than {MAX_INLINE_DEPTH} deep"));
        }
        // Arguments are passed but never read (a parameter read is reported in the body).
        for a in args {
            let passes_event = matches!((&a.kind, &cx.event), (E::Name(n), Some((e, _))) if n == e);
            if !passes_event && self.constant(a, cx).is_err() {
                self.problem(a.pos, "arguments are values or the event; computing them isn't supported");
            }
        }
        if args.len() < f.params.iter().filter(|p| !p.default && !p.rest).count() {
            return self.problem(pos, format!("`{name}` needs more arguments than this call passes"));
        }
        let mut inner = Cx { event: None, in_listener: cx.in_listener, in_function: true, params: f.params.iter().map(|p| p.name.clone()).collect(), stack: cx.stack.clone() };
        inner.stack.push(name);
        self.stmts(&f.body, &inner, out);
    }

    fn trace(&mut self, pos: Pos, args: &'a [Expr], cx: &Cx, out: &mut Vec<ScriptOp>) {
        let mut parts = Vec::new();
        for a in args {
            match self.constant(a, cx) {
                Ok(v) => parts.push(v.text()),
                Err(p) => return self.problems.push(p),
            }
        }
        let message = parts.join(" ");
        if message.chars().count() > MAX_TRACE_CHARS {
            return self.problem(pos, format!("a trace message is at most {MAX_TRACE_CHARS} characters"));
        }
        out.push(ScriptOp::Action(Action::Trace { message }));
    }

    /// A timeline method on the instance at `path`.
    fn method(&mut self, pos: Pos, path: Vec<String>, m: &str, args: &'a [Expr], cx: &Cx, out: &mut Vec<ScriptOp>) {
        let Some(target) = self.path(pos, &path) else { return };
        let none = |l: &mut Self| {
            if args.is_empty() {
                true
            } else {
                l.problem(pos, format!("`{m}()` takes no arguments"));
                false
            }
        };
        let action = match m {
            "stop" if none(self) => Action::Stop { target },
            "play" if none(self) => Action::Play { target },
            "nextFrame" if none(self) => Action::NextFrame { target },
            "prevFrame" if none(self) => Action::PrevFrame { target },
            "nextScene" | "prevScene" => {
                if !none(self) {
                    return;
                }
                // Scenes belong to the main timeline; on this timeline the player checks that
                // it is the main one (a movie clip has no other scenes).
                if !target.is_this() {
                    return self.problem(pos, format!("call `{m}()` in the main timeline's own script: scenes belong to the main timeline"));
                }
                if m == "nextScene" { Action::NextScene } else { Action::PrevScene }
            }
            "gotoAndPlay" | "gotoAndStop" => {
                let (Some(f), true) = (args.first(), args.len() <= 2) else {
                    return self.problem(pos, format!("`{m}` takes a frame, and optionally a scene"));
                };
                let frame = match self.constant(f, cx) {
                    Ok(Val::Num(n)) if n.fract() == 0.0 && (1.0..=f64::from(MAX_FRAME) + 1.0).contains(&n) => FrameRef::Number(n as u32 - 1),
                    Ok(Val::Num(_)) => return self.problem(f.pos, format!("a frame number is a whole number from 1 to {}", u64::from(MAX_FRAME) + 1)),
                    Ok(Val::Str(s)) if label_ok(&s) => FrameRef::Label(s),
                    Ok(Val::Str(_)) => return self.problem(f.pos, format!("a frame label is 1 to {MAX_LABEL_CHARS} printable characters")),
                    Ok(_) => return self.problem(f.pos, "the frame is a number or a label"),
                    Err(p) => return self.problems.push(p),
                };
                let scene = match args.get(1).map(|s| (s.pos, self.constant(s, cx))) {
                    None | Some((_, Ok(Val::Null | Val::Undefined))) => None,
                    Some((_, Ok(Val::Str(s)))) if label_ok(&s) => Some(s),
                    Some((p, Ok(_))) => return self.problem(p, "the scene is its name, as a string"),
                    Some((_, Err(p))) => return self.problems.push(p),
                };
                if m == "gotoAndPlay" { Action::GotoAndPlay { target, frame, scene } } else { Action::GotoAndStop { target, frame, scene } }
            }
            _ => return,
        };
        out.push(ScriptOp::Action(action));
    }

    /// `target.addEventListener(event, function)`.
    fn listen(&mut self, pos: Pos, path: Vec<String>, args: &'a [Expr], cx: &Cx, out: &mut Vec<ScriptOp>) {
        if cx.in_listener {
            return self.problem(pos, "adding a listener from inside a listener isn't supported yet");
        }
        let Some(target) = self.path(pos, &path) else { return };
        let (Some(ev), Some(handler), true) = (args.first(), args.get(1), args.len() <= 5) else {
            return self.problem(pos, "`addEventListener` takes an event and a function");
        };
        let Some(event) = self.event(ev) else { return };
        if let Some(c) = args.get(2)
            && !matches!(self.constant(c, cx), Ok(Val::Bool(false) | Val::Null | Val::Undefined))
        {
            return self.problem(c.pos, "capture-phase listeners (`useCapture`) aren't supported");
        }
        for extra in args.iter().skip(3) {
            if self.constant(extra, cx).is_err() {
                return self.problem(extra.pos, "listener priority and weak references are values");
            }
        }
        let (f, function) = match &handler.kind {
            E::Name(n) => match self.funcs.get(n.as_str()).copied() {
                Some(f) => (f, n.clone()),
                None => return self.problem(handler.pos, format!("`{n}` isn't a function declared in this frame script")),
            },
            E::Function(f) => (&**f, format!("function at line {}", f.pos.line)),
            _ => return self.problem(handler.pos, "the listener is a function declared in this frame script, or written in place"),
        };
        // The event is passed to the function: it takes exactly one argument.
        let required = f.params.iter().filter(|p| !p.default && !p.rest).count();
        let Some(first) = f.params.first().filter(|_| required <= 1) else {
            return self.problem(f.pos, "a listener function takes the event as its one parameter: `function f(e:MouseEvent):void`");
        };
        if cx.stack.contains(&function) {
            return self.problem(pos, format!("`{function}` adds itself as a listener; recursion isn't supported"));
        }
        if self.listeners >= MAX_LISTENERS {
            return self.problem(pos, format!("a frame script adds at most {MAX_LISTENERS} listeners"));
        }
        self.listeners += 1;
        let mut inner = Cx { event: Some((first.name.clone(), path)), in_listener: true, in_function: true, params: f.params.iter().map(|p| p.name.clone()).collect(), stack: cx.stack.clone() };
        inner.stack.push(function.clone());
        let mut body = Vec::new();
        self.stmts(&f.body, &inner, &mut body);
        let actions: Vec<Action> = body
            .into_iter()
            .filter_map(|op| match op {
                ScriptOp::Action(a) => Some(a),
                ScriptOp::Listen(_) => None,
            })
            .collect();
        if actions.len() > MAX_ACTIONS {
            return self.problem(f.pos, format!("a listener runs at most {MAX_ACTIONS} actions"));
        }
        out.push(ScriptOp::Listen(Listener { target, event, function, actions: Arc::new(actions) }));
    }

    /// The mouse event an `addEventListener` names.
    fn event(&mut self, e: &Expr) -> Option<ButtonEvent> {
        let (pos, name, constant) = match &e.kind {
            E::Member(o, k) if matches!(&o.kind, E::Name(c) if c == "MouseEvent" || c == "Event" || c.ends_with("Event")) => {
                let class = if let E::Name(c) = &o.kind { c.as_str() } else { "" };
                (e.pos, format!("{class}.{k}"), if class == "MouseEvent" { k.as_str() } else { "" })
            }
            E::Str(s) => (e.pos, format!("\"{s}\""), match s.as_str() {
                "click" => "CLICK",
                "mouseDown" => "MOUSE_DOWN",
                "mouseUp" => "MOUSE_UP",
                "rollOver" => "ROLL_OVER",
                "rollOut" => "ROLL_OUT",
                "mouseOver" => "MOUSE_OVER",
                "mouseOut" => "MOUSE_OUT",
                _ => "",
            }),
            _ => {
                self.problem(e.pos, "the event is a constant such as `MouseEvent.CLICK`");
                return None;
            }
        };
        let ev = match constant {
            "CLICK" => ButtonEvent::Click,
            "MOUSE_DOWN" => ButtonEvent::Press,
            "MOUSE_UP" => ButtonEvent::Release,
            "ROLL_OVER" | "MOUSE_OVER" => ButtonEvent::Over,
            "ROLL_OUT" | "MOUSE_OUT" => ButtonEvent::Out,
            _ => {
                self.problem(pos, format!("listening for {name} isn't supported yet (mouse CLICK, MOUSE_DOWN, MOUSE_UP, ROLL_OVER and ROLL_OUT are)"));
                return None;
            }
        };
        Some(ev)
    }

    /// `x = value` on this timeline or `path.x = value`.
    fn assign(&mut self, pos: Pos, lhs: &'a Expr, rhs: &'a Expr, cx: &Cx, out: &mut Vec<ScriptOp>) {
        let (obj, prop) = match &lhs.kind {
            E::Member(o, p) => (Some(&**o), p.as_str()),
            E::Name(n) => (None, n.as_str()),
            _ => return self.problem(pos, "only instance properties can be set"),
        };
        let property = match prop {
            "x" => Property::X,
            "y" => Property::Y,
            "alpha" => Property::Alpha,
            "visible" => Property::Visible,
            "rotation" => Property::Rotation,
            "scaleX" => Property::ScaleX,
            "scaleY" => Property::ScaleY,
            "buttonMode" | "useHandCursor" => return self.note(pos, format!("`{prop}` is ignored: the pointer is a hand over everything clickable here")),
            p if obj.is_none() => return self.problem(pos, format!("setting `{p}` isn't supported: variables don't run yet")),
            p => return self.problem(pos, format!("setting `{p}` isn't supported yet (x, y, alpha, visible, rotation, scaleX and scaleY are)")),
        };
        let path = match obj {
            Some(o) => match self.target(o, cx) {
                Some(t) => t,
                None => return,
            },
            None => Vec::new(),
        };
        let Some(target) = self.path(pos, &path) else { return };
        let value = match self.constant(rhs, cx) {
            Ok(v @ (Val::Num(_) | Val::Bool(_))) => v.number(),
            Ok(_) => return self.problem(rhs.pos, format!("`{prop}` is set to a number{}", if property == Property::Visible { " or true or false" } else { "" })),
            Err(p) => return self.problems.push(p),
        };
        if !value.is_finite() {
            return self.problem(rhs.pos, "the value must be a finite number");
        }
        out.push(ScriptOp::Action(Action::SetProperty { target, property, value }));
    }

    /// An instance path as names (`root`, `parent` and instance names), relative to the
    /// script's timeline; `None` after reporting why it isn't one.
    fn target(&mut self, e: &Expr, cx: &Cx) -> Option<Vec<String>> {
        let mut path = match &e.kind {
            E::This => Vec::new(),
            E::Name(n) if n == "root" || n == "parent" => vec![n.clone()],
            E::Name(n) if n == "stage" => {
                self.problem(e.pos, "the stage isn't supported as a target");
                return None;
            }
            E::Name(n) if cx.event.as_ref().is_some_and(|(p, _)| p == n) => {
                self.problem(e.pos, format!("the event `{n}` isn't an instance; `{n}.currentTarget` is the one listened to"));
                return None;
            }
            E::Name(n) if cx.params.contains(n) => {
                self.problem(e.pos, format!("reading the parameter `{n}` isn't supported"));
                return None;
            }
            E::Name(n) => vec![n.clone()],
            E::Member(o, m) if m == "currentTarget" && matches!((&o.kind, &cx.event), (E::Name(n), Some((p, _))) if n == p) => cx.event.as_ref().map(|(_, t)| t.clone()).unwrap_or_default(),
            E::Member(o, m) if m == "target" && matches!((&o.kind, &cx.event), (E::Name(n), Some((p, _))) if n == p) => {
                self.problem(e.pos, "`target` can be a part inside the instance; use `currentTarget`, the instance listened to");
                return None;
            }
            E::Member(o, m) => {
                let mut p = self.target(o, cx)?;
                p.push(m.clone());
                p
            }
            E::Call(f, args) if matches!(&f.kind, E::Name(c) if CASTS.contains(&c.as_str())) && args.len() == 1 => return args.first().and_then(|a| self.target(a, cx)),
            E::Binary("as", x, _) => return self.target(x, cx),
            _ => {
                self.problem(e.pos, "this isn't an instance (a name path such as `hero_mc.arm_mc`, `parent` or `root`)");
                return None;
            }
        };
        // Every instance's `root` is the main timeline.
        if let Some(i) = path.iter().rposition(|s| s == "root") {
            path.drain(..i);
        }
        Some(path)
    }

    /// The IR target for a path, checked.
    fn path(&mut self, pos: Pos, path: &[String]) -> Option<Target> {
        for s in path {
            if !matches!(s.as_str(), "root" | "parent") && !valid_instance_name(s) {
                self.problem(pos, format!("`{s}` isn't an instance name AniCraft can address (1-64 letters, digits, `_` or `$`)"));
                return None;
            }
        }
        let t = Target::new(path.join("."));
        match t.segments() {
            Ok(_) => Some(t),
            Err(e) => {
                self.problem(pos, e);
                None
            }
        }
    }

    /// A value computed from literals (`"Score: " + 10`, `-5`, `0.5`).
    fn constant(&self, e: &Expr, cx: &Cx) -> Result<Val, Problem> {
        Ok(match &e.kind {
            E::Num(n) => Val::Num(*n),
            E::Str(s) => Val::Str(s.clone()),
            E::Bool(b) => Val::Bool(*b),
            E::Null => Val::Null,
            E::Name(n) if n == "undefined" => Val::Undefined,
            E::Name(n) if n == "NaN" => Val::Num(f64::NAN),
            E::Name(n) if n == "Infinity" => Val::Num(f64::INFINITY),
            E::Unary("-", x) => Val::Num(-self.constant(x, cx)?.number()),
            E::Unary("+", x) => Val::Num(self.constant(x, cx)?.number()),
            E::Unary("!", x) => Val::Bool(!truthy(&self.constant(x, cx)?)),
            E::Binary(op @ ("+" | "-" | "*" | "/" | "%"), a, b) => {
                let (a, b) = (self.constant(a, cx)?, self.constant(b, cx)?);
                match *op {
                    "+" if matches!(a, Val::Str(_)) || matches!(b, Val::Str(_)) => {
                        let s = a.text() + &b.text();
                        if s.len() > MAX_TRACE_CHARS * 4 {
                            return Err(Problem::at(e.pos, "this text is too long"));
                        }
                        Val::Str(s)
                    }
                    "+" => Val::Num(a.number() + b.number()),
                    "-" => Val::Num(a.number() - b.number()),
                    "*" => Val::Num(a.number() * b.number()),
                    "/" => Val::Num(a.number() / b.number()),
                    _ => Val::Num(a.number() % b.number()),
                }
            }
            E::Name(n) => {
                let msg = if cx.event.as_ref().is_some_and(|(p, _)| p == n) { format!("reading the event `{n}` isn't supported") } else { format!("reading `{n}` isn't supported: only values written out (numbers, strings, true, false) and `+` of them are") };
                return Err(Problem::at(e.pos, msg));
            }
            E::Member(..) => {
                let mut root = e;
                while let E::Member(o, _) = &root.kind {
                    root = o;
                }
                let msg = match &root.kind {
                    E::Name(n) if cx.event.as_ref().is_some_and(|(p, _)| p == n) => format!("reading the event `{n}` isn't supported"),
                    _ => "reading properties isn't supported: only values written out (numbers, strings, true, false) and `+` of them are".into(),
                };
                return Err(Problem::at(e.pos, msg));
            }
            _ => return Err(Problem::at(e.pos, "only values written out (numbers, strings, true, false) and `+` of them are supported here")),
        })
    }
}

fn truthy(v: &Val) -> bool {
    match v {
        Val::Num(n) => *n != 0.0 && !n.is_nan(),
        Val::Str(s) => !s.is_empty(),
        Val::Bool(b) => *b,
        Val::Null | Val::Undefined => false,
    }
}

fn label_ok(s: &str) -> bool {
    let n = s.chars().count();
    (1..=MAX_LABEL_CHARS).contains(&n) && !s.chars().any(char::is_control) && s.chars().any(|c| !c.is_whitespace())
}

/// The statements' top-level function declarations (also those in plain blocks, which
/// ActionScript hoists too).
fn declare<'a>(list: &'a [Stmt], funcs: &mut BTreeMap<&'a str, &'a Func>, problems: &mut Vec<Problem>) {
    for s in list {
        match &s.kind {
            S::Function(f) => {
                let Some(n) = f.name.as_deref() else { continue };
                if BUILTINS.contains(&n) || CASTS.contains(&n) {
                    problems.push(Problem::at(s.pos, format!("a function named `{n}` would hide ActionScript's own `{n}`")));
                } else if funcs.insert(n, f).is_some() {
                    problems.push(Problem::at(s.pos, format!("`{n}` is declared twice")));
                }
            }
            S::Block(b) => declare(b, funcs, problems),
            _ => {}
        }
    }
}

pub(super) fn lower(prog: &[Stmt]) -> Result<Compiled, Vec<Problem>> {
    let mut funcs = BTreeMap::new();
    let mut problems = Vec::new();
    declare(prog, &mut funcs, &mut problems);
    let mut l = Lower { funcs, problems, notes: Vec::new(), listeners: 0, work: 0 };
    let mut ops = Vec::new();
    l.stmts(prog, &Cx::default(), &mut ops);
    if ops.len() > MAX_ACTIONS {
        l.problems.push(Problem::whole(format!("a frame script runs at most {MAX_ACTIONS} steps")));
    }
    let tidy = |mut v: Vec<Problem>| {
        v.sort();
        v.dedup();
        v.truncate(MAX_PROBLEMS);
        v
    };
    if l.problems.is_empty() { Ok(Compiled { ops, notes: tidy(l.notes) }) } else { Err(tidy(l.problems)) }
}
