//! Running the script IR (ADR 0052): a small tree-walking evaluator over a
//! [`World`] (the player, which knows the movie). It is deterministic (the only random numbers
//! come from the world's seeded generator) and bounded: every statement and expression costs
//! fuel, text has a length limit, calls a depth limit. When fuel runs out or something can't
//! be done (a name that isn't there, a value of the wrong kind), the run stops where it is,
//! as an ActionScript error stops a script, and says why; what it did before stays done.
//!
//! The HTML canvas player (`player.js`) mirrors this file step for step.

use super::ir::*;
use crate::{FrameRef, MAX_FRAME, MAX_LABEL_CHARS, MAX_TRACE_CHARS, Property};
use std::sync::Arc;

/// Fuel one frame script (or one listener call) may use.
pub const FUEL_PER_RUN: u64 = 100_000;
/// Deepest chain of calls.
pub const MAX_CALL_DEPTH: usize = 16;
/// Longest text a script makes, in UTF-16 units.
pub const MAX_TEXT: usize = 16_384;

/// Why a run stopped before its end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stop {
    /// It used its fuel (an endless loop, most likely).
    Fuel,
    /// Something couldn't be done, at `line`.
    Error { line: u32, message: String },
    /// The world stopped everything (a per-frame budget ran out); it already said why.
    Halt,
}

impl Stop {
    pub fn describe(&self) -> String {
        match self {
            Stop::Fuel => format!("it ran more than {FUEL_PER_RUN} steps (an endless loop?)"),
            Stop::Error { line, message } if *line > 0 => format!("line {line}: {message}"),
            Stop::Error { message, .. } => message.clone(),
            Stop::Halt => "the frame's budget ran out".into(),
        }
    }
}

/// What a running script talks to. Errors stop the run (`Err(message)`); actions that can't
/// find their instance write a line to Output and the script goes on, as before (tier 2).
pub trait World {
    /// A timeline action, on the instance `base` starts from (`action`'s target is the name path).
    fn act(&mut self, base: Base, action: &crate::Action) -> Result<(), Option<String>>;
    /// Reads a property.
    fn get(&mut self, at: &Place, p: Prop) -> Result<Value, String>;
    /// Sets an instance property.
    fn set(&mut self, at: &Place, p: Property, v: f64) -> Result<(), Option<String>>;
    /// Sets a text field's text (ADR 0066).
    fn set_text(&mut self, at: &Place, text: String) -> Result<(), Option<String>>;
    fn trace(&mut self, line: String);
    /// A timeline variable's value, and writing one (the world knows its type).
    fn read(&mut self, name: &str) -> Result<Value, String>;
    fn write(&mut self, name: &str, v: Value) -> Result<(), String>;
    /// The timeline's function by that name.
    fn function(&mut self, name: &str) -> Option<Arc<Function>>;
    /// Adds (or removes) a listener; `name` is how it is known (adding the same again does
    /// nothing).
    fn listen(&mut self, at: &Place, ev: ListenEvent, f: &Arc<Function>, add: bool) -> Result<(), Option<String>>;
    /// The next number from the movie's seeded generator, in [0, 1).
    fn random(&mut self) -> f64;
    /// Whether two names are one instance.
    fn same(&mut self, a: &Place, b: &Place) -> Result<bool, String>;
    /// The keyboard event's key code.
    fn key_code(&mut self) -> Result<f64, String>;
}

/// The error of a world call: `None` means the world halted (and said so).
fn lift(line: u32, e: Option<String>) -> Stop {
    match e {
        Some(message) => Stop::Error { line, message },
        None => Stop::Halt,
    }
}

enum Flow {
    Normal,
    Break,
    Continue,
    Return(Value),
}

struct Frame<'f> {
    locals: Vec<Value>,
    /// Each local's type (a write coerces to it).
    types: &'f [Ty],
    inner: &'f [Arc<Function>],
}

struct Vm<'w> {
    w: &'w mut dyn World,
    fuel: u64,
    depth: usize,
    line: u32,
}

type R<T> = Result<T, Stop>;

/// Runs a frame script's top-level statements; returns the fuel used and how it ended.
pub fn run_script(w: &mut dyn World, body: &[Stmt], inner: &[Arc<Function>], fuel: u64) -> (u64, Result<(), Stop>) {
    let mut vm = Vm { w, fuel, depth: 0, line: 0 };
    let mut frame = Frame { locals: Vec::new(), types: &[], inner };
    let r = vm.block(body, &mut frame).map(|_| ());
    (fuel - vm.fuel, r)
}

/// Calls a listener's function with the event (its first parameter).
pub fn run_listener(w: &mut dyn World, f: &Arc<Function>, fuel: u64) -> (u64, Result<(), Stop>) {
    let mut vm = Vm { w, fuel, depth: 0, line: f.line };
    let r = vm.call(f, &[Expr::Event], &mut Frame { locals: Vec::new(), types: &[], inner: &[] }).map(|_| ());
    (fuel - vm.fuel, r)
}

/// JavaScript's `Math.round`: the nearest integer, halves toward +∞.
pub fn round(x: f64) -> f64 {
    if !x.is_finite() {
        return x;
    }
    let f = x.floor();
    if x - f >= 0.5 { f + 1.0 } else { f }
}

/// Math.min / Math.max: NaN wins, no arguments give ±Infinity.
fn min_max(vals: &[f64], max: bool) -> f64 {
    let mut out = if max { f64::NEG_INFINITY } else { f64::INFINITY };
    for &v in vals {
        if v.is_nan() {
            return f64::NAN;
        }
        if (max && v > out) || (!max && v < out) {
            out = v;
        }
    }
    out
}

/// `==` (ECMA-262 §11.9.3) over primitives.
pub fn loose_eq(a: &Value, b: &Value) -> bool {
    use Value as V;
    match (a, b) {
        (V::Undefined | V::Null, V::Undefined | V::Null) => true,
        (V::Undefined | V::Null, _) | (_, V::Undefined | V::Null) => false,
        (V::Num(x), V::Num(y)) => x == y,
        (V::Str(x), V::Str(y)) => x == y,
        (V::Bool(x), V::Bool(y)) => x == y,
        (V::Num(x), V::Str(_)) => *x == b.number(),
        (V::Str(_), V::Num(y)) => a.number() == *y,
        (V::Bool(_), _) => loose_eq(&V::Num(a.number()), b),
        (_, V::Bool(_)) => loose_eq(a, &V::Num(b.number())),
    }
}

/// `===` (§11.9.6).
pub fn strict_eq(a: &Value, b: &Value) -> bool {
    use Value as V;
    match (a, b) {
        (V::Num(x), V::Num(y)) => x == y,
        (V::Undefined, V::Undefined) | (V::Null, V::Null) => true,
        (V::Str(x), V::Str(y)) => x == y,
        (V::Bool(x), V::Bool(y)) => x == y,
        _ => false,
    }
}

/// `<` (§11.8.5): text by UTF-16 units, the rest as numbers; `None` when a NaN is involved.
fn compare(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    if let (Value::Str(x), Value::Str(y)) = (a, b) {
        return Some(x.encode_utf16().cmp(y.encode_utf16()));
    }
    a.number().partial_cmp(&b.number())
}

/// Text cut to [`MAX_TRACE_CHARS`] UTF-16 units (never through a surrogate pair), with `…`.
pub fn trace_line(s: String) -> String {
    let units: Vec<u16> = s.encode_utf16().collect();
    if units.len() <= MAX_TRACE_CHARS {
        return s;
    }
    let mut n = MAX_TRACE_CHARS;
    if units.get(n - 1).is_some_and(|u| (0xd800..0xdc00).contains(u)) {
        n -= 1;
    }
    let mut out = String::from_utf16_lossy(units.get(..n).unwrap_or(&[]));
    out.push('…');
    out
}

impl Vm<'_> {
    fn tick(&mut self, n: u64) -> R<()> {
        if self.fuel < n {
            self.fuel = 0;
            return Err(Stop::Fuel);
        }
        self.fuel -= n;
        Ok(())
    }

    fn err<T>(&self, message: impl Into<String>) -> R<T> {
        Err(Stop::Error { line: self.line, message: message.into() })
    }

    fn block(&mut self, list: &[Stmt], fr: &mut Frame<'_>) -> R<Flow> {
        for s in list {
            match self.stmt(s, fr)? {
                Flow::Normal => {}
                other => return Ok(other),
            }
        }
        Ok(Flow::Normal)
    }

    fn stmt(&mut self, st: &Stmt, fr: &mut Frame<'_>) -> R<Flow> {
        self.tick(1)?;
        self.line = st.l;
        match &st.s {
            S::Do { x } => {
                self.expr(x, fr)?;
            }
            S::If { c, a, b } => {
                let c = self.expr(c, fr)?.truthy();
                return self.block(if c { a } else { b }, fr);
            }
            S::Loop { c, body, next, first } => {
                let mut test = *first;
                loop {
                    if test && let Some(c) = c {
                        self.line = st.l;
                        if !self.expr(c, fr)?.truthy() {
                            break;
                        }
                    }
                    test = true;
                    // A loop with nothing in it still costs fuel.
                    self.tick(1)?;
                    match self.block(body, fr)? {
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        Flow::Normal | Flow::Continue => {}
                    }
                    if let Some(n) = next {
                        self.line = st.l;
                        self.expr(n, fr)?;
                    }
                }
            }
            S::Break => return Ok(Flow::Break),
            S::Continue => return Ok(Flow::Continue),
            S::Return { v } => {
                let v = match v {
                    Some(v) => self.expr(v, fr)?,
                    None => Value::Undefined,
                };
                return Ok(Flow::Return(v));
            }
            S::Act { at, m, args } => {
                let frame = match args.first() {
                    Some(a) => Some(self.expr(a, fr)?),
                    None => None,
                };
                let scene = match args.get(1) {
                    Some(a) => Some(self.expr(a, fr)?),
                    None => None,
                };
                self.line = st.l;
                let frame = match frame {
                    None => FrameRef::default(),
                    Some(Value::Str(s)) if label_ok(&s) => FrameRef::Label(s.to_string()),
                    Some(Value::Num(n)) if n.fract() == 0.0 && (1.0..=f64::from(MAX_FRAME) + 1.0).contains(&n) => FrameRef::Number(n as u32 - 1),
                    Some(v) => return self.err(format!("`{}` needs a frame number from 1 or a label, not {}", m.name(), shown(&v))),
                };
                let scene = match scene {
                    None | Some(Value::Null | Value::Undefined) => None,
                    Some(Value::Str(s)) if label_ok(&s) => Some(s.to_string()),
                    Some(v) => return self.err(format!("the scene is its name, not {}", shown(&v))),
                };
                let action = m.action(at.path.clone(), frame, scene);
                self.w.act(at.base, &action).map_err(|e| lift(st.l, e))?;
            }
            S::Trace { args } => {
                let mut parts = Vec::with_capacity(args.len());
                for a in args {
                    parts.push(self.expr(a, fr)?.text());
                }
                self.w.trace(trace_line(parts.join(" ")));
            }
            S::Listen { at, ev, f, add } => {
                let func = match f {
                    FuncRef::Named(n) => match self.w.function(n) {
                        Some(f) => f,
                        None => return self.err(format!("there's no function `{n}` on this timeline")),
                    },
                    FuncRef::Inline(i) => match fr.inner.get(usize::from(*i)) {
                        Some(f) => f.clone(),
                        None => return self.err("the listener's function is missing"),
                    },
                };
                self.w.listen(at, *ev, &func, *add).map_err(|e| lift(st.l, e))?;
            }
        }
        Ok(Flow::Normal)
    }

    fn text(&mut self, s: String) -> R<Value> {
        let n = text_len(&s);
        if n > MAX_TEXT {
            return self.err(format!("this text would be longer than {MAX_TEXT} characters"));
        }
        // Making text costs fuel for its length, so a loop building text can't run long.
        self.tick(n as u64 / 64)?;
        Ok(Value::Str(s.into()))
    }

    fn binary(&mut self, op: BinOp, a: Value, b: Value) -> R<Value> {
        use std::cmp::Ordering::*;
        Ok(match op {
            BinOp::Add if a.is_str() || b.is_str() => return self.text(a.text() + &b.text()),
            BinOp::Add => Value::Num(a.number() + b.number()),
            BinOp::Sub => Value::Num(a.number() - b.number()),
            BinOp::Mul => Value::Num(a.number() * b.number()),
            BinOp::Div => Value::Num(a.number() / b.number()),
            BinOp::Mod => Value::Num(a.number() % b.number()),
            BinOp::Lt => Value::Bool(compare(&a, &b) == Some(Less)),
            BinOp::Gt => Value::Bool(compare(&a, &b) == Some(Greater)),
            BinOp::Le => Value::Bool(matches!(compare(&a, &b), Some(Less | Equal))),
            BinOp::Ge => Value::Bool(matches!(compare(&a, &b), Some(Greater | Equal))),
            BinOp::Eq => Value::Bool(loose_eq(&a, &b)),
            BinOp::Ne => Value::Bool(!loose_eq(&a, &b)),
            BinOp::StrictEq => Value::Bool(strict_eq(&a, &b)),
            BinOp::StrictNe => Value::Bool(!strict_eq(&a, &b)),
        })
    }

    fn load(&mut self, to: &LValue, fr: &Frame<'_>) -> R<Value> {
        match to {
            LValue::Local { i } => Ok(fr.locals.get(usize::from(*i)).cloned().unwrap_or_default()),
            LValue::Var { n } => self.w.read(n).map_err(|message| Stop::Error { line: self.line, message }),
            LValue::Prop { at, p } => {
                let prop = match p {
                    Property::X => Prop::X,
                    Property::Y => Prop::Y,
                    Property::Alpha => Prop::Alpha,
                    Property::Visible => Prop::Visible,
                    Property::Rotation => Prop::Rotation,
                    Property::ScaleX => Prop::ScaleX,
                    Property::ScaleY => Prop::ScaleY,
                };
                self.w.get(at, prop).map_err(|message| Stop::Error { line: self.line, message })
            }
            LValue::Text { at } => self.w.get(at, Prop::Text).map_err(|message| Stop::Error { line: self.line, message }),
        }
    }

    /// Writes `v`; the value is what the place holds after (a typed variable's coercion).
    fn store(&mut self, to: &LValue, v: Value, fr: &mut Frame<'_>) -> R<Value> {
        match to {
            LValue::Local { i } => {
                let ty = fr.types.get(usize::from(*i)).copied().unwrap_or_default();
                let v = ty.coerce(v);
                if let Some(slot) = fr.locals.get_mut(usize::from(*i)) {
                    *slot = v.clone();
                }
                Ok(v)
            }
            LValue::Var { n } => {
                self.w.write(n, v).map_err(|message| Stop::Error { line: self.line, message })?;
                self.w.read(n).map_err(|message| Stop::Error { line: self.line, message })
            }
            LValue::Prop { at, p } => {
                let n = v.number();
                if !n.is_finite() {
                    return self.err(format!("`{}` is set to a number, not {}", prop_name(*p), shown(&v)));
                }
                self.w.set(at, *p, n).map_err(|e| lift(self.line, e))?;
                Ok(v)
            }
            LValue::Text { at } => {
                if matches!(v, Value::Null | Value::Undefined) {
                    return self.err(format!("`text` is set to text, not {}", v.text()));
                }
                self.w.set_text(at, v.text()).map_err(|e| lift(self.line, e))?;
                Ok(v)
            }
        }
    }

    fn expr(&mut self, e: &Expr, fr: &mut Frame<'_>) -> R<Value> {
        self.tick(1)?;
        Ok(match e {
            Expr::Lit { v } => v.clone(),
            Expr::Local { i } => fr.locals.get(usize::from(*i)).cloned().unwrap_or_default(),
            Expr::Var { n } => self.w.read(n).map_err(|message| Stop::Error { line: self.line, message })?,
            Expr::Get { at, p } => self.w.get(at, *p).map_err(|message| Stop::Error { line: self.line, message })?,
            Expr::Key => Value::Num(self.w.key_code().map_err(|message| Stop::Error { line: self.line, message })?),
            Expr::Event => return self.err("the event can only be passed to a function's event parameter"),
            Expr::Un { op, x } => {
                let v = self.expr(x, fr)?;
                match op {
                    UnOp::Neg => Value::Num(-v.number()),
                    UnOp::Plus => Value::Num(v.number()),
                    UnOp::Not => Value::Bool(!v.truthy()),
                }
            }
            Expr::Bin { op, a, b } => {
                let a = self.expr(a, fr)?;
                let b = self.expr(b, fr)?;
                return self.binary(*op, a, b);
            }
            Expr::And { a, b } => {
                let a = self.expr(a, fr)?;
                if a.truthy() { self.expr(b, fr)? } else { a }
            }
            Expr::Or { a, b } => {
                let a = self.expr(a, fr)?;
                if a.truthy() { a } else { self.expr(b, fr)? }
            }
            Expr::Cond { c, a, b } => {
                if self.expr(c, fr)?.truthy() {
                    self.expr(a, fr)?
                } else {
                    self.expr(b, fr)?
                }
            }
            Expr::Math { f, args } => {
                let mut v = Vec::with_capacity(args.len());
                for a in args {
                    v.push(self.expr(a, fr)?.number());
                }
                let x = v.first().copied().unwrap_or(f64::NAN);
                Value::Num(match f {
                    MathFn::Min => min_max(&v, false),
                    MathFn::Max => min_max(&v, true),
                    MathFn::Abs => x.abs(),
                    MathFn::Floor => x.floor(),
                    MathFn::Ceil => x.ceil(),
                    MathFn::Round => round(x),
                    MathFn::Random => self.w.random(),
                })
            }
            Expr::Call { f, args } => {
                let Some(func) = self.w.function(f) else { return self.err(format!("there's no function `{f}` on this timeline")) };
                let line = self.line;
                let v = self.call(&func, args, fr)?;
                self.line = line;
                v
            }
            Expr::Same { a, b, ne } => {
                let same = self.w.same(a, b).map_err(|message| Stop::Error { line: self.line, message })?;
                Value::Bool(same != *ne)
            }
            Expr::Set { to, op, v } => {
                let new = match op {
                    None => self.expr(v, fr)?,
                    Some(op) => {
                        let old = self.load(to, fr)?;
                        let rhs = self.expr(v, fr)?;
                        self.binary(*op, old, rhs)?
                    }
                };
                return self.store(to, new, fr);
            }
            Expr::Step { to, d, post } => {
                let old = Value::Num(self.load(to, fr)?.number());
                let new = Value::Num(old.number() + f64::from(*d));
                let stored = self.store(to, new, fr)?;
                if *post { old } else { stored }
            }
            Expr::Cast { t, x } => {
                let v = self.expr(x, fr)?;
                t.coerce(v)
            }
        })
    }

    /// Calls `f` with `args` evaluated in the caller's frame.
    fn call(&mut self, f: &Arc<Function>, args: &[Expr], caller: &mut Frame<'_>) -> R<Value> {
        if self.depth >= MAX_CALL_DEPTH {
            return self.err(format!("functions call each other more than {MAX_CALL_DEPTH} deep"));
        }
        let required = f.params.iter().filter(|p| p.d.is_none()).count();
        if args.len() < required || args.len() > f.params.len() {
            let want = if required == f.params.len() { format!("{required}") } else { format!("{required} to {}", f.params.len()) };
            return self.err(format!("`{}` takes {want} argument{}, not {}", f.name, if f.params.len() == 1 { "" } else { "s" }, args.len()));
        }
        let mut locals: Vec<Value> = f.locals.iter().map(|t| t.default_value()).collect();
        for (i, p) in f.params.iter().enumerate() {
            let v = match args.get(i) {
                Some(Expr::Event) if p.ev || p.t == Ty::Any => Value::Null,
                Some(Expr::Event) => return self.err("the event can only be passed to a function's event parameter"),
                Some(a) => self.expr(a, caller)?,
                None => p.d.clone().unwrap_or_default(),
            };
            if let Some(slot) = locals.get_mut(i) {
                *slot = p.t.coerce(v);
            }
        }
        self.depth += 1;
        self.line = f.line;
        let mut fr = Frame { locals, types: &f.locals, inner: &f.inner };
        let flow = self.block(&f.body, &mut fr);
        self.depth -= 1;
        let v = match flow? {
            Flow::Return(v) => v,
            _ => Value::Undefined,
        };
        Ok(match f.ret {
            Some(t) => t.coerce(v),
            None => Value::Undefined,
        })
    }
}

fn label_ok(s: &str) -> bool {
    let n = s.chars().count();
    (1..=MAX_LABEL_CHARS).contains(&n) && !s.chars().any(char::is_control) && s.chars().any(|c| !c.is_whitespace())
}

/// A value as a message names it.
fn shown(v: &Value) -> String {
    match v {
        Value::Str(s) => format!("\"{}\"", s.chars().take(40).collect::<String>()),
        other => other.text(),
    }
}

fn prop_name(p: Property) -> &'static str {
    match p {
        Property::X => "x",
        Property::Y => "y",
        Property::Alpha => "alpha",
        Property::Visible => "visible",
        Property::Rotation => "rotation",
        Property::ScaleX => "scaleX",
        Property::ScaleY => "scaleY",
    }
}
