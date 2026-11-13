//! Lowering: the syntax tree to the script IR ([`super::ir`]). It accepts what the IR can say
//! exactly and reports everything else where it is, so a script either compiles whole or
//! doesn't run at all. Names resolve here: a function's own variables become numbered slots,
//! the timeline's variables and functions are named (those another frame declares are listed
//! as [`Free`] names, which the timeline must have for the script to run), and instances become
//! name paths from a base.

use super::ir::{self, Base, BinOp, Expr as X, FuncRef, Function, LValue, ListenEvent, MathFn, Method, Param as IrParam, Place, Prop, S as IS, Stmt as IStmt, Ty, UnOp, Value};
use super::parse::{E, Expr, Func, S, Stmt, VarDecl};
use super::{Compiled, Free, MAX_LISTENERS, MAX_PROBLEMS, Pos, Problem};
use crate::{MAX_FRAME, MAX_LABEL_CHARS, Target, valid_instance_name};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Names a script's own function can't take: they would hide the timeline's methods.
const BUILTINS: [&str; 12] = ["stop", "play", "gotoAndPlay", "gotoAndStop", "nextFrame", "prevFrame", "nextScene", "prevScene", "trace", "addEventListener", "removeEventListener", "getChildByName"];
/// Calls that would reach outside the movie: ignored, and noted.
const OUTSIDE: [&str; 4] = ["navigateToURL", "sendToURL", "fscommand", "fsCommand"];
/// Type casts that leave an instance path as it is (`MovieClip(parent)`).
const CASTS: [&str; 7] = ["MovieClip", "Object", "Sprite", "DisplayObject", "DisplayObjectContainer", "InteractiveObject", "SimpleButton"];
/// Names that are classes, not values.
const CLASSES: [&str; 6] = ["Math", "Keyboard", "MouseEvent", "Event", "KeyboardEvent", "ExternalInterface"];
/// Names that are always the path words.
const PATH_WORDS: [&str; 3] = ["root", "parent", "stage"];

/// A type annotation as a script variable's type, or why it can't be one. `…Event` types are
/// events (`Ok(None)`).
fn var_type(ty: &Option<(String, Pos)>) -> Result<Option<Ty>, (Pos, String)> {
    match ty {
        None => Ok(Some(Ty::Any)),
        Some((t, _)) if t.ends_with("Event") => Ok(None),
        Some((t, p)) => Ty::from_name(t).map(Some).ok_or_else(|| (*p, format!("variables of type `{t}` aren't supported yet (Number, int, uint, Boolean, String and untyped ones are)"))),
    }
}

/// What lowering a function's (or the script's top level's) code knows.
struct Unit {
    /// The function's own variables: slot and type.
    locals: BTreeMap<String, (u16, Ty)>,
    types: Vec<Ty>,
    /// Its parameters that are the event.
    events: BTreeSet<String>,
    in_function: bool,
    /// Loops it is inside (for `break` and `continue`).
    loops: usize,
    /// Functions written in place inside it.
    inner: Vec<Arc<Function>>,
}

impl Unit {
    fn top() -> Unit {
        Unit { locals: BTreeMap::new(), types: Vec::new(), events: BTreeSet::new(), in_function: false, loops: 0, inner: Vec::new() }
    }
}

struct Lower<'a> {
    funcs: BTreeMap<&'a str, &'a Func>,
    /// The script's top-level variables (the timeline's).
    vars: BTreeMap<String, Ty>,
    problems: Vec<Problem>,
    notes: Vec<Problem>,
    free: Vec<Free>,
    listeners: usize,
    /// Functions being lowered (inline listeners nest).
    depth: usize,
}

type O<T> = Option<T>;

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

    fn free(&mut self, name: &str, pos: Pos, call: bool) {
        if !self.free.iter().any(|f| f.name == name && f.call == call) && self.free.len() < 1024 {
            self.free.push(Free { name: name.to_string(), line: pos.line, column: pos.column, call });
        }
    }

    // ----- statements ----------------------------------------------------------------------

    fn stmts(&mut self, list: &'a [Stmt], u: &mut Unit, out: &mut Vec<IStmt>) {
        for s in list {
            self.stmt(s, u, out);
        }
    }

    fn body(&mut self, list: &'a [Stmt], u: &mut Unit) -> Vec<IStmt> {
        let mut out = Vec::new();
        self.stmts(list, u, &mut out);
        out
    }

    fn stmt(&mut self, s: &'a Stmt, u: &mut Unit, out: &mut Vec<IStmt>) {
        let l = s.pos.line;
        let push = |out: &mut Vec<IStmt>, s: IS| out.push(IStmt { l, s });
        match &s.kind {
            S::Empty | S::Import => {}
            // Declared functions run where they are called or listened for.
            S::Function(_) => {
                if u.in_function {
                    self.problem(s.pos, "functions declared inside functions aren't supported yet");
                }
            }
            S::Block(b) => self.stmts(b, u, out),
            S::Var(_, list) => self.var_stmt(list, u, out, l),
            S::If(c, a, b) => {
                let Some(c) = self.expr(c, u) else { return };
                let a = self.body(a, u);
                let b = self.body(b, u);
                push(out, IS::If { c, a, b });
            }
            S::For(init, c, next, body) => {
                if let Some(init) = init {
                    self.stmt(init, u, out);
                }
                let c = c.as_ref().and_then(|c| self.expr(c, u));
                let next = next.as_ref().and_then(|n| self.expr(n, u));
                u.loops += 1;
                let body = self.body(body, u);
                u.loops -= 1;
                push(out, IS::Loop { c, body, next, first: true });
            }
            S::While(c, body) | S::DoWhile(body, c) => {
                let first = matches!(s.kind, S::While(..));
                let c = self.expr(c, u);
                u.loops += 1;
                let body = self.body(body, u);
                u.loops -= 1;
                push(out, IS::Loop { c, body, next: None, first });
            }
            S::Break | S::Continue => {
                if u.loops == 0 {
                    let w = if matches!(s.kind, S::Break) { "break" } else { "continue" };
                    return self.problem(s.pos, format!("`{w}` is only for loops here"));
                }
                push(out, if matches!(s.kind, S::Break) { IS::Break } else { IS::Continue });
            }
            S::Other(w, _) => {
                let what = match *w {
                    "for each" | "for in" => format!("`{w}` loops aren't supported yet (`for`, `while` and `do` are)"),
                    "labelled break" => "labels on `break` and `continue` aren't supported".to_string(),
                    _ => format!("`{w}` statements aren't supported yet"),
                };
                self.problem(s.pos, what);
            }
            S::Return(e) => {
                let v = match e {
                    Some(e) => match self.expr(e, u) {
                        Some(v) => Some(v),
                        None => return,
                    },
                    None => None,
                };
                push(out, IS::Return { v });
            }
            S::Expr(e) => self.expr_stmt(e, u, out, l),
        }
    }

    fn var_stmt(&mut self, list: &'a [VarDecl], u: &mut Unit, out: &mut Vec<IStmt>, l: u32) {
        for d in list {
            let Some(init) = &d.init else { continue };
            let to = if u.in_function {
                match u.locals.get(&d.name) {
                    Some((i, _)) => LValue::Local { i: *i },
                    None => continue,
                }
            } else {
                LValue::Var { n: d.name.clone() }
            };
            let Some(v) = self.expr(init, u) else { continue };
            out.push(IStmt { l, s: IS::Do { x: X::Set { to, op: None, v: Box::new(v) } } });
        }
    }

    fn expr_stmt(&mut self, e: &'a Expr, u: &mut Unit, out: &mut Vec<IStmt>, l: u32) {
        match &e.kind {
            E::Call(callee, args) => {
                if let Some(s) = self.call_stmt(e.pos, callee, args, u) {
                    out.push(IStmt { l, s });
                }
            }
            E::Comma(list) => {
                for x in list {
                    self.expr_stmt(x, u, out, l);
                }
            }
            // A literal alone does nothing (`"use strict"`).
            E::Num(_) | E::Str(_) | E::Bool(_) | E::Null => {}
            E::Assign(_, lhs, _) if matches!(&lhs.kind, E::Member(_, p) if p == "buttonMode" || p == "useHandCursor") => {
                let E::Member(_, p) = &lhs.kind else { return };
                self.note(e.pos, format!("`{p}` is ignored: the pointer is a hand over everything clickable here"));
            }
            _ => {
                if let Some(x) = self.expr(e, u) {
                    out.push(IStmt { l, s: IS::Do { x } });
                }
            }
        }
    }

    /// A call as a statement: timeline methods, `trace`, listeners, and any call with a value.
    fn call_stmt(&mut self, pos: Pos, callee: &'a Expr, args: &'a [Expr], u: &mut Unit) -> O<IS> {
        match &callee.kind {
            E::Name(n) if !u.locals.contains_key(n) => {
                let n = n.as_str();
                if self.funcs.contains_key(n) {
                    return self.call_expr(pos, callee, args, u).map(|x| IS::Do { x });
                }
                match n {
                    "trace" => {
                        let args = self.args(args, u)?;
                        Some(IS::Trace { args })
                    }
                    "addEventListener" | "removeEventListener" => self.listen(pos, Place::this(), args, n == "addEventListener", u),
                    n if Method::from_name(n).is_some() => self.method(pos, Place::this(), n, args, u),
                    n if OUTSIDE.contains(&n) => {
                        self.note(pos, format!("`{n}` is ignored: a movie here can't open web pages or reach outside itself"));
                        None
                    }
                    n if CASTS.contains(&n) => {
                        self.problem(pos, format!("`{n}(…)` alone does nothing; call a method on it, as in `{n}(parent).play()`"));
                        None
                    }
                    _ => self.call_expr(pos, callee, args, u).map(|x| IS::Do { x }),
                }
            }
            E::Member(obj, m) => {
                let m = m.as_str();
                if OUTSIDE.contains(&m) || (matches!(&obj.kind, E::Name(o) if o == "ExternalInterface") && matches!(m, "call" | "addCallback")) {
                    self.note(pos, format!("`{m}` is ignored: a movie here can't open web pages or reach outside itself"));
                    return None;
                }
                if matches!(&obj.kind, E::Name(o) if o == "Math") {
                    return self.call_expr(pos, callee, args, u).map(|x| IS::Do { x });
                }
                // `field.appendText(v)` is `field.text += v` (ADR 0066).
                if m == "appendText" {
                    let [arg] = args else {
                        self.problem(pos, "`appendText` takes one argument, the text to add");
                        return None;
                    };
                    let (at, v) = (self.field_place(obj, u), self.expr(arg, u));
                    return Some(IS::Do { x: X::Set { to: LValue::Text { at: at? }, op: Some(BinOp::Add), v: Box::new(v?) } });
                }
                if Method::from_name(m).is_none() && m != "addEventListener" && m != "removeEventListener" {
                    self.problem(pos, format!("calling `{m}` isn't supported (timeline methods such as `play()` and `gotoAndStop()`, `addEventListener` and `removeEventListener` are)"));
                    return None;
                }
                let at = self.place(obj, u)?;
                match m {
                    "addEventListener" | "removeEventListener" => self.listen(pos, at, args, m == "addEventListener", u),
                    _ => self.method(pos, at, m, args, u),
                }
            }
            _ => self.call_expr(pos, callee, args, u).map(|x| IS::Do { x }),
        }
    }

    fn args(&mut self, args: &'a [Expr], u: &mut Unit) -> O<Vec<X>> {
        let mut out = Vec::with_capacity(args.len());
        let mut ok = true;
        for a in args {
            match self.expr(a, u) {
                Some(x) => out.push(x),
                None => ok = false,
            }
        }
        ok.then_some(out)
    }

    /// A timeline method on the instance at `at`.
    fn method(&mut self, pos: Pos, at: Place, m: &str, args: &'a [Expr], u: &mut Unit) -> O<IS> {
        let method = Method::from_name(m)?;
        let takes = matches!(method, Method::GotoAndPlay | Method::GotoAndStop);
        if !takes && !args.is_empty() {
            self.problem(pos, format!("`{m}()` takes no arguments"));
            return None;
        }
        if matches!(method, Method::NextScene | Method::PrevScene) && (!at.path.is_this() || !at.base.is_this()) {
            // Scenes belong to the main timeline; on this timeline the player checks that it is
            // the main one (a movie clip has no other scenes).
            self.problem(pos, format!("call `{m}()` in the main timeline's own script: scenes belong to the main timeline"));
            return None;
        }
        if takes && (args.is_empty() || args.len() > 2) {
            self.problem(pos, format!("`{m}` takes a frame, and optionally a scene"));
            return None;
        }
        // Frames and scenes written out are checked now; computed ones when they run.
        if let Some(f) = args.first() {
            match &f.kind {
                E::Num(n) if !(n.fract() == 0.0 && (1.0..=f64::from(MAX_FRAME) + 1.0).contains(n)) => {
                    self.problem(f.pos, format!("a frame number is a whole number from 1 to {}", u64::from(MAX_FRAME) + 1));
                    return None;
                }
                E::Str(s) if !label_ok(s) => {
                    self.problem(f.pos, format!("a frame label is 1 to {MAX_LABEL_CHARS} printable characters"));
                    return None;
                }
                E::Bool(_) | E::Null => {
                    self.problem(f.pos, "the frame is a number or a label");
                    return None;
                }
                _ => {}
            }
        }
        if let Some(s) = args.get(1)
            && (matches!(&s.kind, E::Num(_) | E::Bool(_)) || matches!(&s.kind, E::Str(t) if !label_ok(t)))
        {
            self.problem(s.pos, "the scene is its name, as a string");
            return None;
        }
        let args = self.args(args, u)?;
        Some(IS::Act { at, m: method, args })
    }

    /// `at.addEventListener(event, function)` (or `removeEventListener`).
    fn listen(&mut self, pos: Pos, at: Place, args: &'a [Expr], add: bool, u: &mut Unit) -> O<IS> {
        let verb = if add { "addEventListener" } else { "removeEventListener" };
        let (Some(ev), Some(handler), true) = (args.first(), args.get(1), args.len() <= if add { 5 } else { 3 }) else {
            self.problem(pos, format!("`{verb}` takes an event and a function"));
            return None;
        };
        let event = self.event(ev)?;
        if let Some(c) = args.get(2)
            && !matches!(c.kind, E::Bool(false) | E::Null)
        {
            self.problem(c.pos, "capture-phase listeners (`useCapture`) aren't supported");
            return None;
        }
        for extra in args.iter().skip(3) {
            if !matches!(extra.kind, E::Num(_) | E::Bool(_) | E::Null) {
                self.problem(extra.pos, "listener priority and weak references are values");
                return None;
            }
        }
        if event.is_key() && at.base != Base::Stage {
            self.note(pos, "keys go to the stage here: listen with `stage.addEventListener(KeyboardEvent.KEY_DOWN, …)`; this listener is ignored");
            return None;
        }
        if at.base == Base::Stage && event.mouse().is_some() {
            self.problem(pos, "mouse listeners on the stage aren't supported yet (on buttons and movie clips they are)");
            return None;
        }
        let f = match &handler.kind {
            E::Name(n) if !u.locals.contains_key(n) => {
                match self.funcs.get(n.as_str()).copied() {
                    Some(f) => {
                        if !takes_event(f) {
                            self.problem(f.pos, "a listener function takes the event as its one parameter: `function f(e:MouseEvent):void`");
                            return None;
                        }
                    }
                    None if CLASSES.contains(&n.as_str()) || BUILTINS.contains(&n.as_str()) => {
                        self.problem(handler.pos, format!("`{n}` isn't a function declared in a frame script"));
                        return None;
                    }
                    None => self.free(n, handler.pos, true),
                }
                FuncRef::Named(n.clone())
            }
            E::Function(f) if add => {
                if !takes_event(f) {
                    self.problem(f.pos, "a listener function takes the event as its one parameter: `function f(e:MouseEvent):void`");
                    return None;
                }
                let name = format!("function at line {}, column {}", f.pos.line, f.pos.column);
                let func = self.function(f, name)?;
                if u.inner.len() >= usize::from(u16::MAX) {
                    return None;
                }
                u.inner.push(func);
                FuncRef::Inline((u.inner.len() - 1) as u16)
            }
            E::Function(_) => {
                self.problem(handler.pos, "a function written in place can't be removed; give it a name and listen with that");
                return None;
            }
            _ => {
                self.problem(handler.pos, "the listener is a function declared in a frame script, or written in place");
                return None;
            }
        };
        if add {
            if self.listeners >= MAX_LISTENERS {
                self.problem(pos, format!("a frame script adds at most {MAX_LISTENERS} listeners"));
                return None;
            }
            self.listeners += 1;
        }
        Some(IS::Listen { at, ev: event, f, add })
    }

    /// The event an `addEventListener` names.
    fn event(&mut self, e: &Expr) -> O<ListenEvent> {
        let (name, key) = match &e.kind {
            E::Member(o, k) if matches!(&o.kind, E::Name(c) if c.ends_with("Event")) => {
                let class = if let E::Name(c) = &o.kind { c.as_str() } else { "" };
                let ev = match (class, k.as_str()) {
                    ("MouseEvent", "CLICK") => Some(ListenEvent::Click),
                    ("MouseEvent", "MOUSE_DOWN") => Some(ListenEvent::Press),
                    ("MouseEvent", "MOUSE_UP") => Some(ListenEvent::Release),
                    ("MouseEvent", "ROLL_OVER" | "MOUSE_OVER") => Some(ListenEvent::Over),
                    ("MouseEvent", "ROLL_OUT" | "MOUSE_OUT") => Some(ListenEvent::Out),
                    ("Event", "ENTER_FRAME") => Some(ListenEvent::EnterFrame),
                    ("KeyboardEvent", "KEY_DOWN") => Some(ListenEvent::KeyDown),
                    ("KeyboardEvent", "KEY_UP") => Some(ListenEvent::KeyUp),
                    _ => None,
                };
                (format!("{class}.{k}"), ev)
            }
            E::Str(s) => (
                format!("\"{s}\""),
                match s.as_str() {
                    "click" => Some(ListenEvent::Click),
                    "mouseDown" => Some(ListenEvent::Press),
                    "mouseUp" => Some(ListenEvent::Release),
                    "rollOver" | "mouseOver" => Some(ListenEvent::Over),
                    "rollOut" | "mouseOut" => Some(ListenEvent::Out),
                    "enterFrame" => Some(ListenEvent::EnterFrame),
                    "keyDown" => Some(ListenEvent::KeyDown),
                    "keyUp" => Some(ListenEvent::KeyUp),
                    _ => None,
                },
            ),
            _ => {
                self.problem(e.pos, "the event is a constant such as `MouseEvent.CLICK`");
                return None;
            }
        };
        if key.is_none() {
            self.problem(e.pos, format!("listening for {name} isn't supported yet (mouse CLICK, MOUSE_DOWN, MOUSE_UP, ROLL_OVER and ROLL_OUT, Event.ENTER_FRAME and KeyboardEvent.KEY_DOWN and KEY_UP are)"));
        }
        key
    }

    // ----- functions -----------------------------------------------------------------------

    /// A function's IR: parameters, its own variables (hoisted, as ActionScript does) and body.
    fn function(&mut self, f: &'a Func, name: String) -> O<Arc<Function>> {
        if self.depth >= 8 {
            self.problem(f.pos, "listeners written in place nest more than 8 deep");
            return None;
        }
        let before = self.problems.len();
        let mut u = Unit { in_function: true, ..Unit::top() };
        let mut params = Vec::new();
        for (i, p) in f.params.iter().enumerate() {
            if p.rest {
                self.problem(p.pos, "rest parameters (`...args`) aren't supported");
                continue;
            }
            let (t, ev) = match var_type(&p.ty) {
                Ok(Some(t)) => (t, false),
                Ok(None) => (Ty::Any, true),
                Err((pos, m)) => {
                    self.problem(pos, m.replace("variables of type", "parameters of type"));
                    continue;
                }
            };
            // An untyped first parameter can be the event, too (`function f(e)`).
            if ev || (i == 0 && p.ty.is_none()) {
                u.events.insert(p.name.clone());
            }
            let d = match &p.default {
                None => None,
                Some(e) => match literal(e) {
                    Some(v) => Some(t.coerce(v)),
                    None => {
                        self.problem(e.pos, "a parameter's default is a value written out (a number, a string, true, false or null)");
                        continue;
                    }
                },
            };
            if u.locals.insert(p.name.clone(), (u.types.len() as u16, t)).is_some() {
                self.problem(p.pos, format!("the parameter `{}` is named twice", p.name));
            }
            u.types.push(t);
            params.push(IrParam { t, d, ev });
        }
        // Every `var` in the body is the function's (hoisted).
        let mut decls = Vec::new();
        collect_vars(&f.body, &mut decls);
        for d in decls {
            let t = match var_type(&d.ty) {
                Ok(Some(t)) => t,
                Ok(None) => {
                    self.problem(d.pos, "variables can't hold events; read `e.keyCode`, `e.target` or `e.currentTarget` from the parameter");
                    continue;
                }
                Err((pos, m)) => {
                    self.problem(pos, m);
                    continue;
                }
            };
            match u.locals.get(&d.name) {
                Some((_, old)) if *old != t && d.ty.is_some() => self.problem(d.pos, format!("`{}` is declared again with another type", d.name)),
                Some(_) => {}
                None => {
                    if u.types.len() >= 1024 {
                        self.problem(d.pos, "a function has at most 1024 variables");
                        continue;
                    }
                    u.locals.insert(d.name.clone(), (u.types.len() as u16, t));
                    u.types.push(t);
                }
            }
        }
        let ret = match &f.ret {
            None => Some(Ty::Any),
            Some((t, _)) if t == "void" => None,
            Some((t, p)) => match Ty::from_name(t) {
                Some(t) => Some(t),
                None => {
                    self.problem(*p, format!("functions returning `{t}` aren't supported yet (Number, int, uint, Boolean, String, void and untyped ones are)"));
                    None
                }
            },
        };
        self.depth += 1;
        let body = self.body(&f.body, &mut u);
        self.depth -= 1;
        if self.problems.len() > before {
            return None;
        }
        Some(Arc::new(Function { name, line: f.pos.line, params, locals: u.types, ret, body, inner: u.inner }))
    }

    // ----- names and places ----------------------------------------------------------------

    /// An instance a script names, from the timeline or the event; `None` after reporting why
    /// it isn't one.
    fn place(&mut self, e: &Expr, u: &Unit) -> O<Place> {
        let mut at = match &e.kind {
            E::This => Place::this(),
            E::Name(n) if u.locals.contains_key(n) && !u.events.contains(n) => {
                self.problem(e.pos, format!("`{n}` is a variable, not an instance"));
                return None;
            }
            E::Name(n) if u.events.contains(n) => {
                self.problem(e.pos, format!("the event `{n}` isn't an instance; `{n}.currentTarget` is the one listened to"));
                return None;
            }
            E::Name(n) if n == "stage" => Place { base: Base::Stage, path: Target::this() },
            E::Name(n) if n == "root" || n == "parent" => Place::named(n.clone()),
            E::Name(n) if self.vars.contains_key(n) => {
                self.problem(e.pos, format!("`{n}` is a variable, not an instance"));
                return None;
            }
            E::Name(n) if CLASSES.contains(&n.as_str()) => {
                self.problem(e.pos, format!("`{n}` is a class, not an instance"));
                return None;
            }
            E::Name(n) => Place::named(n.clone()),
            E::Member(o, m) if (m == "currentTarget" || m == "target") && matches!(&o.kind, E::Name(n) if u.events.contains(n)) => Place { base: if m == "target" { Base::Target } else { Base::Current }, path: Target::this() },
            E::Member(o, m) => {
                let mut p = self.place(o, u)?;
                if p.base == Base::Stage {
                    self.problem(e.pos, format!("`stage.{m}` isn't supported"));
                    return None;
                }
                let path = if p.path.is_this() { m.clone() } else { format!("{}.{m}", p.path.0.trim()) };
                p.path = Target::new(path);
                p
            }
            E::Call(f, args) if matches!(&f.kind, E::Name(c) if CASTS.contains(&c.as_str())) && args.len() == 1 => return args.first().and_then(|a| self.place(a, u)),
            E::Binary("as", x, _) => return self.place(x, u),
            _ => {
                self.problem(e.pos, "this isn't an instance (a name path such as `hero_mc.arm_mc`, `parent` or `root`)");
                return None;
            }
        };
        // Every instance's `root` is the main timeline.
        let segs: Vec<String> = at.path.0.trim().split('.').filter(|s| !s.is_empty()).map(str::to_string).collect();
        if let Some(i) = segs.iter().rposition(|s| s == "root") {
            at = Place { base: Base::This, path: Target::new(segs.get(i..).unwrap_or_default().join(".")) };
        }
        for s in at.path.0.split('.').filter(|s| !s.is_empty()) {
            if !matches!(s, "root" | "parent" | "this") && !valid_instance_name(s) {
                self.problem(e.pos, format!("`{s}` isn't an instance name AniCraft can address (1-64 letters, digits, `_` or `$`)"));
                return None;
            }
        }
        if let Err(m) = at.path.segments() {
            self.problem(e.pos, m);
            return None;
        }
        Some(at)
    }

    /// The text field a script names (`score_txt`, `panel.name_txt`): an instance, not the
    /// stage or a timeline (`this` is one only if it is a field, which a frame script's isn't).
    fn field_place(&mut self, e: &Expr, u: &Unit) -> O<Place> {
        let at = self.place(e, u)?;
        if at.base == Base::Stage || (at.base == Base::This && at.path.is_this()) {
            self.problem(e.pos, "this isn't a text field; name the field by its instance name, as in `score_txt.text`");
            return None;
        }
        Some(at)
    }

    /// Whether an expression names an instance rather than a value (for `==` between instances).
    fn is_instance(&self, e: &Expr, u: &Unit) -> bool {
        match &e.kind {
            E::This => true,
            E::Name(n) => n == "stage" || n == "root" || n == "parent",
            E::Member(o, m) => (m == "currentTarget" || m == "target") && matches!(&o.kind, E::Name(n) if u.events.contains(n)),
            E::Call(f, args) => matches!(&f.kind, E::Name(c) if CASTS.contains(&c.as_str())) && args.len() == 1,
            E::Binary("as", ..) => true,
            _ => false,
        }
    }

    /// Where an assignment writes.
    fn lvalue(&mut self, e: &'a Expr, u: &mut Unit) -> O<LValue> {
        match &e.kind {
            E::Name(n) => {
                if let Some((i, _)) = u.locals.get(n) {
                    return Some(LValue::Local { i: *i });
                }
                if self.vars.contains_key(n) {
                    return Some(LValue::Var { n: n.clone() });
                }
                if let Some(p) = Prop::from_name(n) {
                    return match p.settable() {
                        Some(p) => Some(LValue::Prop { at: Place::this(), p }),
                        None => {
                            self.problem(e.pos, format!("`{n}` can only be read"));
                            None
                        }
                    };
                }
                if self.funcs.contains_key(n.as_str()) || PATH_WORDS.contains(&n.as_str()) || CLASSES.contains(&n.as_str()) || BUILTINS.contains(&n.as_str()) || matches!(n.as_str(), "undefined" | "NaN" | "Infinity") {
                    self.problem(e.pos, format!("`{n}` can't be assigned to"));
                    return None;
                }
                self.free(n, e.pos, false);
                Some(LValue::Var { n: n.clone() })
            }
            E::Member(o, m) if m == "text" => Some(LValue::Text { at: self.field_place(o, u)? }),
            E::Member(o, m) => {
                let Some(p) = Prop::from_member(m) else {
                    self.problem(e.pos, format!("setting `{m}` isn't supported yet (x, y, alpha, visible, rotation, scaleX, scaleY and a text field's text are)"));
                    return None;
                };
                let Some(p) = p.settable() else {
                    self.problem(e.pos, format!("`{m}` can only be read"));
                    return None;
                };
                let at = self.place(o, u)?;
                if at.base == Base::Stage {
                    self.problem(e.pos, "the stage's properties can't be set here");
                    return None;
                }
                Some(LValue::Prop { at, p })
            }
            _ => {
                self.problem(e.pos, "only variables and instance properties can be assigned to");
                None
            }
        }
    }

    // ----- expressions ---------------------------------------------------------------------

    fn expr(&mut self, e: &'a Expr, u: &mut Unit) -> O<X> {
        Some(match &e.kind {
            E::Num(n) => X::lit(Value::Num(*n)),
            E::Str(s) => {
                if ir::text_len(s) > super::run::MAX_TEXT {
                    self.problem(e.pos, "this text is too long");
                    return None;
                }
                X::lit(Value::str(s))
            }
            E::Bool(b) => X::lit(Value::Bool(*b)),
            E::Null => X::lit(Value::Null),
            E::Name(n) => return self.name(e.pos, n, u),
            E::Member(o, m) => return self.member(e, o, m, u),
            E::Unary(op, x) => match *op {
                "-" | "+" | "!" => {
                    let x = self.expr(x, u)?;
                    let op = match *op {
                        "-" => UnOp::Neg,
                        "+" => UnOp::Plus,
                        _ => UnOp::Not,
                    };
                    X::Un { op, x: Box::new(x) }
                }
                "++" | "--" => X::Step { to: self.lvalue(x, u)?, d: if *op == "++" { 1 } else { -1 }, post: false },
                other => {
                    self.problem(e.pos, format!("`{other}` isn't supported yet"));
                    return None;
                }
            },
            E::Postfix(op, x) => X::Step { to: self.lvalue(x, u)?, d: if *op == "++" { 1 } else { -1 }, post: true },
            E::Binary(op, a, b) => match *op {
                "&&" | "||" => {
                    let (a, b) = (self.expr(a, u), self.expr(b, u));
                    let (a, b) = (Box::new(a?), Box::new(b?));
                    if *op == "&&" { X::And { a, b } } else { X::Or { a, b } }
                }
                "==" | "!=" | "===" | "!==" if self.is_instance(a, u) || self.is_instance(b, u) => {
                    let (pa, pb) = (self.place(a, u), self.place(b, u));
                    X::Same { a: pa?, b: pb?, ne: op.starts_with('!') }
                }
                op => {
                    let Some(bop) = BinOp::from_token(op) else {
                        let what = match op {
                            "as" | "is" | "instanceof" | "in" => format!("`{op}` isn't supported here"),
                            _ => format!("the `{op}` operator isn't supported yet"),
                        };
                        self.problem(e.pos, what);
                        return None;
                    };
                    let (a, b) = (self.expr(a, u), self.expr(b, u));
                    X::Bin { op: bop, a: Box::new(a?), b: Box::new(b?) }
                }
            },
            E::Assign(op, lhs, rhs) => {
                let op = match *op {
                    "=" => None,
                    o => match BinOp::from_token(o.trim_end_matches('=')) {
                        Some(b @ (BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod)) => Some(b),
                        _ => {
                            self.problem(e.pos, format!("`{o}` isn't supported yet (`=`, `+=`, `-=`, `*=`, `/=` and `%=` are)"));
                            return None;
                        }
                    },
                };
                let (to, v) = (self.lvalue(lhs, u), self.expr(rhs, u));
                X::Set { to: to?, op, v: Box::new(v?) }
            }
            E::Cond(c, a, b) => {
                let (c, a, b) = (self.expr(c, u), self.expr(a, u), self.expr(b, u));
                X::Cond { c: Box::new(c?), a: Box::new(a?), b: Box::new(b?) }
            }
            E::Call(callee, args) => return self.call_expr(e.pos, callee, args, u),
            E::Comma(_) => {
                self.problem(e.pos, "`,` between expressions isn't supported here");
                return None;
            }
            E::Function(_) => {
                self.problem(e.pos, "a function written in place is only supported as a listener");
                return None;
            }
            E::This => {
                self.problem(e.pos, "`this` is the timeline; read a property of it, as in `this.currentFrame`");
                return None;
            }
            E::Array(_) => {
                self.problem(e.pos, "arrays aren't supported yet");
                return None;
            }
            E::Object(_) => {
                self.problem(e.pos, "objects (`{…}`) aren't supported yet");
                return None;
            }
            E::New(..) => {
                self.problem(e.pos, "`new` isn't supported yet");
                return None;
            }
            E::Index(..) => {
                self.problem(e.pos, "`[…]` access isn't supported yet");
                return None;
            }
        })
    }

    /// A bare name as a value.
    fn name(&mut self, pos: Pos, n: &str, u: &mut Unit) -> O<X> {
        if let Some((i, _)) = u.locals.get(n) {
            return Some(X::Local { i: *i });
        }
        if self.vars.contains_key(n) {
            return Some(X::Var { n: n.to_string() });
        }
        match n {
            "undefined" => return Some(X::lit(Value::Undefined)),
            "NaN" => return Some(X::lit(Value::Num(f64::NAN))),
            "Infinity" => return Some(X::lit(Value::Num(f64::INFINITY))),
            _ => {}
        }
        if let Some(p) = Prop::from_name(n) {
            return Some(X::Get { at: Place::this(), p });
        }
        if self.funcs.contains_key(n) || BUILTINS.contains(&n) {
            self.problem(pos, format!("`{n}` is a function; call it, as in `{n}()`"));
            return None;
        }
        if PATH_WORDS.contains(&n) || CLASSES.contains(&n) || CASTS.contains(&n) {
            self.problem(pos, format!("`{n}` isn't a value; read one of its properties"));
            return None;
        }
        self.free(n, pos, false);
        Some(X::Var { n: n.to_string() })
    }

    /// `o.m` as a value: constants, the event's key code, or a property of an instance.
    fn member(&mut self, e: &'a Expr, o: &'a Expr, m: &str, u: &mut Unit) -> O<X> {
        if let E::Name(c) = &o.kind
            && (!u.locals.contains_key(c) || u.events.contains(c))
            && !self.vars.contains_key(c)
        {
            match c.as_str() {
                "Math" => {
                    return match m {
                        "PI" => Some(X::lit(Value::Num(std::f64::consts::PI))),
                        "E" => Some(X::lit(Value::Num(std::f64::consts::E))),
                        _ => {
                            self.problem(e.pos, format!("`Math.{m}` isn't supported yet (min, max, abs, floor, ceil, round, random and PI are)"));
                            None
                        }
                    };
                }
                "Keyboard" => {
                    return match ir::key_code(m) {
                        Some(k) => Some(X::lit(Value::Num(f64::from(k)))),
                        None => {
                            self.problem(e.pos, format!("`Keyboard.{m}` isn't a key AniCraft knows"));
                            None
                        }
                    };
                }
                ev if u.events.contains(ev) => {
                    return match m {
                        "keyCode" => Some(X::Key),
                        "currentTarget" | "target" => {
                            self.problem(e.pos, format!("`{ev}.{m}` is an instance; read one of its properties, as in `{ev}.{m}.name`"));
                            None
                        }
                        _ => {
                            self.problem(e.pos, format!("reading the event's `{m}` isn't supported yet (keyCode, target and currentTarget are)"));
                            None
                        }
                    };
                }
                _ => {}
            }
        }
        if let E::Name(n) = &o.kind
            && n == "stage"
            && !matches!(m, "mouseX" | "mouseY")
        {
            self.problem(e.pos, format!("`stage.{m}` isn't supported yet"));
            return None;
        }
        if let E::Name(n) = &o.kind
            && ((u.locals.contains_key(n) && !u.events.contains(n)) || self.vars.contains_key(n))
        {
            self.problem(e.pos, format!("reading `{m}` of a value isn't supported yet"));
            return None;
        }
        let Some(p) = Prop::from_member(m) else {
            self.problem(e.pos, format!("reading `{m}` isn't supported yet (x, y, alpha, visible, rotation, scaleX, scaleY, currentFrame, totalFrames, currentLabel, mouseX, mouseY, name, and a text field's text and length are)"));
            return None;
        };
        let at = if matches!(p, Prop::Text | Prop::Length) { self.field_place(o, u)? } else { self.place(o, u)? };
        if at.base == Base::Stage && !matches!(p, Prop::MouseX | Prop::MouseY) {
            self.problem(e.pos, format!("`stage.{m}` isn't supported yet"));
            return None;
        }
        Some(X::Get { at, p })
    }

    /// A call with a value: the timeline's functions, `Math`, and casts.
    fn call_expr(&mut self, pos: Pos, callee: &'a Expr, args: &'a [Expr], u: &mut Unit) -> O<X> {
        match &callee.kind {
            E::Member(o, m) if matches!(&o.kind, E::Name(c) if c == "Math") => {
                let Some(f) = MathFn::from_name(m) else {
                    self.problem(pos, format!("`Math.{m}` isn't supported yet (min, max, abs, floor, ceil, round and random are)"));
                    return None;
                };
                if let Some(n) = f.arity()
                    && n != args.len()
                {
                    self.problem(pos, format!("`Math.{m}` takes {n} argument{}", if n == 1 { "" } else { "s" }));
                    return None;
                }
                let args = self.args(args, u)?;
                Some(X::Math { f, args })
            }
            E::Name(n) if !u.locals.contains_key(n) => {
                let n = n.as_str();
                let cast = match n {
                    "int" => Some(Ty::Int),
                    "uint" => Some(Ty::Uint),
                    "Number" => Some(Ty::Number),
                    "String" => Some(Ty::String),
                    "Boolean" => Some(Ty::Boolean),
                    _ => None,
                };
                if let Some(t) = cast {
                    let [a] = args else {
                        self.problem(pos, format!("`{n}(…)` takes one value"));
                        return None;
                    };
                    let x = self.expr(a, u)?;
                    // `String(x)` of null is "null"; as a variable type it stays null.
                    return Some(match t {
                        Ty::String => X::Bin { op: BinOp::Add, a: Box::new(X::lit(Value::str(""))), b: Box::new(x) },
                        t => X::Cast { t, x: Box::new(x) },
                    });
                }
                if Method::from_name(n).is_some() || matches!(n, "trace" | "addEventListener" | "removeEventListener") {
                    self.problem(pos, format!("`{n}()` gives no value; call it as a statement"));
                    return None;
                }
                if OUTSIDE.contains(&n) || CASTS.contains(&n) || CLASSES.contains(&n) || n == "getChildByName" {
                    self.problem(pos, format!("`{n}(…)` isn't supported here"));
                    return None;
                }
                let local = self.funcs.get(n).copied();
                if local.is_none() {
                    self.free(n, callee.pos, true);
                }
                if let Some(f) = local {
                    let required = f.params.iter().filter(|p| p.default.is_none() && !p.rest).count();
                    if args.len() < required || args.len() > f.params.len() {
                        self.problem(pos, format!("`{n}` takes {}{} argument{}, not {}", required, if required == f.params.len() { String::new() } else { format!(" to {}", f.params.len()) }, if f.params.len() == 1 { "" } else { "s" }, args.len()));
                        return None;
                    }
                }
                let mut out = Vec::with_capacity(args.len());
                let mut ok = true;
                for a in args {
                    // The event a listener handles passes on to the function's event parameter.
                    if let E::Name(x) = &a.kind
                        && u.events.contains(x)
                    {
                        out.push(X::Event);
                        continue;
                    }
                    match self.expr(a, u) {
                        Some(x) => out.push(x),
                        None => ok = false,
                    }
                }
                ok.then_some(X::Call { f: n.to_string(), args: out })
            }
            E::Member(_, m) => {
                self.problem(pos, format!("calling `{m}` here isn't supported"));
                None
            }
            _ => {
                self.problem(pos, "only named functions and methods can be called");
                None
            }
        }
    }
}

/// Whether a declared function can be a listener: the event is its one required parameter.
fn takes_event(f: &Func) -> bool {
    let required = f.params.iter().filter(|p| p.default.is_none() && !p.rest).count();
    required <= 1 && !f.params.is_empty()
}

/// A value written out (for parameter defaults).
fn literal(e: &Expr) -> O<Value> {
    Some(match &e.kind {
        E::Num(n) => Value::Num(*n),
        E::Str(s) => Value::str(s),
        E::Bool(b) => Value::Bool(*b),
        E::Null => Value::Null,
        E::Name(n) if n == "undefined" => Value::Undefined,
        E::Name(n) if n == "NaN" => Value::Num(f64::NAN),
        E::Unary("-", x) => match &x.kind {
            E::Num(n) => Value::Num(-n),
            _ => return None,
        },
        _ => return None,
    })
}

fn label_ok(s: &str) -> bool {
    let n = s.chars().count();
    (1..=MAX_LABEL_CHARS).contains(&n) && !s.chars().any(char::is_control) && s.chars().any(|c| !c.is_whitespace())
}

/// Every `var` in a list of statements, into blocks and loops but not into functions.
fn collect_vars<'a>(list: &'a [Stmt], out: &mut Vec<&'a VarDecl>) {
    for s in list {
        match &s.kind {
            S::Var(_, l) => out.extend(l.iter()),
            S::Block(b) | S::While(_, b) | S::DoWhile(b, _) | S::Other(_, b) => collect_vars(b, out),
            S::If(_, a, b) => {
                collect_vars(a, out);
                collect_vars(b, out);
            }
            S::For(init, _, _, b) => {
                if let Some(i) = init {
                    collect_vars(std::slice::from_ref(&**i), out);
                }
                collect_vars(b, out);
            }
            _ => {}
        }
    }
}

/// The statements' top-level function declarations (also those in plain blocks, which
/// ActionScript hoists too).
fn declare<'a>(list: &'a [Stmt], funcs: &mut BTreeMap<&'a str, &'a Func>, problems: &mut Vec<Problem>) {
    for s in list {
        match &s.kind {
            S::Function(f) => {
                let Some(n) = f.name.as_deref() else { continue };
                if BUILTINS.contains(&n) || CASTS.contains(&n) || CLASSES.contains(&n) || Prop::from_name(n).is_some() || matches!(n, "int" | "uint" | "Number" | "String" | "Boolean") {
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

/// Counts statements (for "runs, N steps").
fn count(list: &[IStmt]) -> usize {
    list.iter()
        .map(|s| {
            1 + match &s.s {
                IS::If { a, b, .. } => count(a) + count(b),
                IS::Loop { body, .. } => count(body),
                _ => 0,
            }
        })
        .sum()
}

pub(super) fn lower(prog: &[Stmt]) -> Result<Compiled, Vec<Problem>> {
    let mut funcs = BTreeMap::new();
    let mut problems = Vec::new();
    declare(prog, &mut funcs, &mut problems);
    // The script's top-level `var`s are the timeline's variables.
    let mut decls = Vec::new();
    collect_vars(prog, &mut decls);
    let mut vars = BTreeMap::new();
    for d in decls {
        let t = match var_type(&d.ty) {
            Ok(Some(t)) => t,
            Ok(None) => {
                problems.push(Problem::at(d.pos, "variables can't hold events"));
                continue;
            }
            Err((p, m)) => {
                problems.push(Problem::at(p, m));
                continue;
            }
        };
        if Prop::from_name(&d.name).is_some() || PATH_WORDS.contains(&d.name.as_str()) || funcs.contains_key(d.name.as_str()) || BUILTINS.contains(&d.name.as_str()) {
            problems.push(Problem::at(d.pos, format!("`{}` is already a name of the timeline; choose another name for the variable", d.name)));
            continue;
        }
        match vars.get(&d.name) {
            Some(old) if *old != t && d.ty.is_some() => problems.push(Problem::at(d.pos, format!("`{}` is declared again with another type", d.name))),
            Some(_) => {}
            None => {
                vars.insert(d.name.clone(), t);
            }
        }
    }
    let mut l = Lower { funcs, vars, problems, notes: Vec::new(), free: Vec::new(), listeners: 0, depth: 0 };
    let mut functions = Vec::new();
    let names: Vec<(&str, &Func)> = l.funcs.iter().map(|(n, f)| (*n, *f)).collect();
    for (n, f) in names {
        if let Some(func) = l.function(f, n.to_string()) {
            functions.push(func);
        }
    }
    let mut top = Unit::top();
    let body = l.body(prog, &mut top);
    let tidy = |mut v: Vec<Problem>| {
        v.sort();
        v.dedup();
        v.truncate(MAX_PROBLEMS);
        v
    };
    if !l.problems.is_empty() {
        return Err(tidy(l.problems));
    }
    let steps = count(&body) + functions.iter().map(|f| count(&f.body)).sum::<usize>();
    Ok(Compiled { body, inner: top.inner, functions, vars: l.vars.into_iter().collect(), free: l.free, notes: tidy(l.notes), steps })
}
