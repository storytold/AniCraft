//! The script IR (ADR 0052): a small, typed form of statements and expressions that a
//! compiled script runs as, evaluated by [`super::run`]. It is data, not code: the player (and
//! the HTML canvas player, which reads the same IR as JSON) walks it with a fuel budget.
//!
//! Names are resolved when the script compiles: a function's own variables are numbered slots,
//! the timeline's variables (`var` at the top of a frame script, shared by every frame script
//! of that timeline, as ActionScript 3.0 timelines keep them per instance) are looked up by
//! name, and instances are name paths from a base (the timeline, the stage, or the event's
//! target). Calls to functions go by name to the timeline's functions, so a frame can call a
//! function another frame of the same timeline declares.

use crate::{FrameRef, Property, Target};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::sync::Arc;

/// A variable's type: what values written to it are coerced to (ECMA-262 §9, ActionScript 3.0's
/// `int` and `uint` as ToInt32 and ToUint32).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ty {
    /// Untyped (`*`, `Object`, or no annotation): keeps any value.
    #[default]
    Any,
    Number,
    Int,
    Uint,
    Boolean,
    String,
}

impl Ty {
    /// The type an annotation names, when it is one scripts can hold.
    pub fn from_name(s: &str) -> Option<Ty> {
        Some(match s {
            "*" | "Object" => Ty::Any,
            "Number" => Ty::Number,
            "int" => Ty::Int,
            "uint" => Ty::Uint,
            "Boolean" => Ty::Boolean,
            "String" => Ty::String,
            _ => return None,
        })
    }

    /// What a variable of this type holds before anything is written to it.
    pub fn default_value(self) -> Value {
        match self {
            Ty::Any => Value::Undefined,
            Ty::Number => Value::Num(f64::NAN),
            Ty::Int | Ty::Uint => Value::Num(0.0),
            Ty::Boolean => Value::Bool(false),
            Ty::String => Value::Null,
        }
    }

    /// `v` as this type.
    pub fn coerce(self, v: Value) -> Value {
        match self {
            Ty::Any => v,
            Ty::Number => Value::Num(v.number()),
            Ty::Int => Value::Num(f64::from(to_int32(v.number()))),
            Ty::Uint => Value::Num(f64::from(to_uint32(v.number()))),
            Ty::Boolean => Value::Bool(v.truthy()),
            Ty::String => match v {
                Value::Null | Value::Undefined => Value::Null,
                Value::Str(_) => v,
                other => Value::Str(other.text().into()),
            },
        }
    }
}

/// ToInt32 (ECMA-262 §9.5).
pub fn to_int32(n: f64) -> i32 {
    let u = to_uint32(n);
    u as i32
}

/// ToUint32 (ECMA-262 §9.6).
pub fn to_uint32(n: f64) -> u32 {
    if !n.is_finite() {
        return 0;
    }
    let m = n.trunc().rem_euclid(4_294_967_296.0);
    // In 0 .. 2^32 by construction.
    if (0.0..4_294_967_296.0).contains(&m) { m as u32 } else { 0 }
}

/// A value a script computes with: ActionScript's primitive values.
#[derive(Clone, Debug, Default)]
pub enum Value {
    #[default]
    Undefined,
    Null,
    Bool(bool),
    Num(f64),
    Str(Arc<str>),
}

impl PartialEq for Value {
    /// The same value (NaN is NaN): what state comparisons need, not ActionScript's `==`.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Undefined, Value::Undefined) | (Value::Null, Value::Null) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Num(a), Value::Num(b)) => a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan()),
            (Value::Str(a), Value::Str(b)) => a == b,
            _ => false,
        }
    }
}

impl Value {
    pub fn str(s: &str) -> Value {
        Value::Str(s.into())
    }

    /// ToNumber (ECMA-262 §9.3).
    pub fn number(&self) -> f64 {
        match self {
            Value::Num(n) => *n,
            Value::Bool(b) => f64::from(u8::from(*b)),
            Value::Null => 0.0,
            Value::Undefined => f64::NAN,
            Value::Str(s) => string_number(s),
        }
    }

    /// ToBoolean (§9.2).
    pub fn truthy(&self) -> bool {
        match self {
            Value::Num(n) => *n != 0.0 && !n.is_nan(),
            Value::Str(s) => !s.is_empty(),
            Value::Bool(b) => *b,
            Value::Null | Value::Undefined => false,
        }
    }

    /// ToString (§9.8).
    pub fn text(&self) -> String {
        match self {
            Value::Num(n) => number_text(*n),
            Value::Str(s) => s.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Null => "null".into(),
            Value::Undefined => "undefined".into(),
        }
    }

    pub fn is_str(&self) -> bool {
        matches!(self, Value::Str(_))
    }
}

/// The JSON form: `"u"`, `"null"`, `{"b": true}`, `{"n": 1.5}`, `{"s": "text"}`, `"nan"`,
/// `{"inf": false}` (JSON has no NaN or infinities).
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ValueRepr {
    U,
    Null,
    B(bool),
    N(f64),
    S(String),
    Nan,
    /// `true`: negative.
    Inf(bool),
}

impl Serialize for Value {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Value::Undefined => ValueRepr::U,
            Value::Null => ValueRepr::Null,
            Value::Bool(b) => ValueRepr::B(*b),
            Value::Num(n) if n.is_nan() => ValueRepr::Nan,
            Value::Num(n) if n.is_infinite() => ValueRepr::Inf(*n < 0.0),
            Value::Num(n) => ValueRepr::N(*n),
            Value::Str(t) => ValueRepr::S(t.to_string()),
        }
        .serialize(s)
    }
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Value, D::Error> {
        Ok(match ValueRepr::deserialize(d)? {
            ValueRepr::U => Value::Undefined,
            ValueRepr::Null => Value::Null,
            ValueRepr::B(b) => Value::Bool(b),
            ValueRepr::N(n) => Value::Num(n),
            ValueRepr::S(t) => Value::Str(t.into()),
            ValueRepr::Nan => Value::Num(f64::NAN),
            ValueRepr::Inf(neg) => Value::Num(if neg { f64::NEG_INFINITY } else { f64::INFINITY }),
        })
    }
}

/// White space and line terminators as ECMAScript trims them from numbers in text.
fn js_space(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}

/// A number written as text (StringToNumber, ECMA-262 §9.3.1 as browsers read it: decimal,
/// `Infinity`, and `0x`/`0o`/`0b` integers), or NaN.
pub fn string_number(s: &str) -> f64 {
    let t = s.trim_matches(js_space);
    if t.is_empty() {
        return 0.0;
    }
    let radix = |p: &str, r: u32| -> f64 {
        if p.is_empty() || !p.chars().all(|c| c.is_digit(r)) {
            return f64::NAN;
        }
        p.chars().filter_map(|c| c.to_digit(r)).fold(0.0, |v, d| v * f64::from(r) + f64::from(d))
    };
    if let Some(p) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        return radix(p, 16);
    }
    if let Some(p) = t.strip_prefix("0o").or_else(|| t.strip_prefix("0O")) {
        return radix(p, 8);
    }
    if let Some(p) = t.strip_prefix("0b").or_else(|| t.strip_prefix("0B")) {
        return radix(p, 2);
    }
    let (neg, body) = match t.as_bytes().first() {
        Some(b'-') => (true, t.get(1..).unwrap_or("")),
        Some(b'+') => (false, t.get(1..).unwrap_or("")),
        _ => (false, t),
    };
    if body == "Infinity" {
        return if neg { f64::NEG_INFINITY } else { f64::INFINITY };
    }
    // digits [. digits] [e[+-]digits], or . digits [e…]
    let b = body.as_bytes();
    let mut i = 0;
    let digits = |i: &mut usize| {
        let from = *i;
        while b.get(*i).is_some_and(u8::is_ascii_digit) {
            *i += 1;
        }
        *i - from
    };
    let int = digits(&mut i);
    let mut frac = 0;
    if b.get(i) == Some(&b'.') {
        i += 1;
        frac = digits(&mut i);
    }
    if int + frac == 0 {
        return f64::NAN;
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(b.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        if digits(&mut i) == 0 {
            return f64::NAN;
        }
    }
    if i != b.len() {
        return f64::NAN;
    }
    let v: f64 = body.parse().unwrap_or(f64::NAN);
    if neg { -v } else { v }
}

/// A number as ActionScript writes it (`Number.toString()`, ECMA-262 §9.8.1): integers without
/// a point, exponents outside 1e-6 … 1e21.
pub fn number_text(n: f64) -> String {
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

/// Length in UTF-16 units, as ActionScript's `String.length` counts.
pub fn text_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// Where an instance path starts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Base {
    /// The script's timeline.
    #[default]
    This,
    /// The stage (keyboard listeners live there).
    Stage,
    /// The event's `target`.
    Target,
    /// The event's `currentTarget`: the instance (or stage) listened to.
    Current,
}

impl Base {
    pub fn is_this(&self) -> bool {
        *self == Base::This
    }
}

/// An instance a script names: a base and a name path from it (`hero_mc.arm_mc`, `parent`,
/// `root.menu`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Place {
    #[serde(default, skip_serializing_if = "Base::is_this")]
    pub base: Base,
    #[serde(default, skip_serializing_if = "Target::is_this")]
    pub path: Target,
}

impl Place {
    pub fn this() -> Place {
        Place::default()
    }
    pub fn named(path: impl Into<String>) -> Place {
        Place { base: Base::This, path: Target::new(path) }
    }
}

impl std::fmt::Display for Place {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let base = match self.base {
            Base::This => "",
            Base::Stage => "stage",
            Base::Target => "e.target",
            Base::Current => "e.currentTarget",
        };
        match (base, self.path.is_this()) {
            ("", _) => write!(f, "{}", self.path),
            (b, true) => f.write_str(b),
            (b, false) => write!(f, "{b}.{}", self.path),
        }
    }
}

/// A property a script reads (of an instance or of the timeline).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Prop {
    X,
    Y,
    Alpha,
    Visible,
    Rotation,
    ScaleX,
    ScaleY,
    /// 1-based, as ActionScript counts frames.
    CurrentFrame,
    TotalFrames,
    /// The label at or before the playhead, or `null`.
    CurrentLabel,
    /// The pointer in the instance's own space.
    MouseX,
    MouseY,
    /// The instance name.
    Name,
    /// A text field's text, and its length in UTF-16 units (ADR 0066). Only as `field.text` and
    /// `field.length`: a bare `text` is a variable.
    Text,
    Length,
}

impl Prop {
    pub fn from_name(s: &str) -> Option<Prop> {
        Some(match s {
            "x" => Prop::X,
            "y" => Prop::Y,
            "alpha" => Prop::Alpha,
            "visible" => Prop::Visible,
            "rotation" => Prop::Rotation,
            "scaleX" => Prop::ScaleX,
            "scaleY" => Prop::ScaleY,
            "currentFrame" => Prop::CurrentFrame,
            "totalFrames" => Prop::TotalFrames,
            "currentLabel" => Prop::CurrentLabel,
            "mouseX" => Prop::MouseX,
            "mouseY" => Prop::MouseY,
            "name" => Prop::Name,
            _ => return None,
        })
    }
    /// A property read or set as `instance.m`: those [`Prop::from_name`] knows, and a text
    /// field's `text` and `length`.
    pub fn from_member(s: &str) -> Option<Prop> {
        match s {
            "text" => Some(Prop::Text),
            "length" => Some(Prop::Length),
            _ => Prop::from_name(s),
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Prop::X => "x",
            Prop::Y => "y",
            Prop::Alpha => "alpha",
            Prop::Visible => "visible",
            Prop::Rotation => "rotation",
            Prop::ScaleX => "scaleX",
            Prop::ScaleY => "scaleY",
            Prop::CurrentFrame => "currentFrame",
            Prop::TotalFrames => "totalFrames",
            Prop::CurrentLabel => "currentLabel",
            Prop::MouseX => "mouseX",
            Prop::MouseY => "mouseY",
            Prop::Name => "name",
            Prop::Text => "text",
            Prop::Length => "length",
        }
    }
    /// The instance property a script may also set.
    pub fn settable(self) -> Option<Property> {
        Some(match self {
            Prop::X => Property::X,
            Prop::Y => Property::Y,
            Prop::Alpha => Property::Alpha,
            Prop::Visible => Property::Visible,
            Prop::Rotation => Property::Rotation,
            Prop::ScaleX => Property::ScaleX,
            Prop::ScaleY => Property::ScaleY,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnOp {
    Neg,
    Plus,
    Not,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
    Ne,
    StrictEq,
    StrictNe,
}

impl BinOp {
    pub fn from_token(s: &str) -> Option<BinOp> {
        Some(match s {
            "+" => BinOp::Add,
            "-" => BinOp::Sub,
            "*" => BinOp::Mul,
            "/" => BinOp::Div,
            "%" => BinOp::Mod,
            "<" => BinOp::Lt,
            ">" => BinOp::Gt,
            "<=" => BinOp::Le,
            ">=" => BinOp::Ge,
            "==" => BinOp::Eq,
            "!=" => BinOp::Ne,
            "===" => BinOp::StrictEq,
            "!==" => BinOp::StrictNe,
            _ => return None,
        })
    }
}

/// `Math` functions scripts may call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MathFn {
    Min,
    Max,
    Abs,
    Floor,
    Ceil,
    Round,
    /// From the player's seeded generator: the same movie and events give the same numbers.
    Random,
}

impl MathFn {
    pub fn from_name(s: &str) -> Option<MathFn> {
        Some(match s {
            "min" => MathFn::Min,
            "max" => MathFn::Max,
            "abs" => MathFn::Abs,
            "floor" => MathFn::Floor,
            "ceil" => MathFn::Ceil,
            "round" => MathFn::Round,
            "random" => MathFn::Random,
            _ => return None,
        })
    }
    /// How many arguments it takes (`None`: any number).
    pub fn arity(self) -> Option<usize> {
        match self {
            MathFn::Min | MathFn::Max => None,
            MathFn::Random => Some(0),
            MathFn::Abs | MathFn::Floor | MathFn::Ceil | MathFn::Round => Some(1),
        }
    }
}

/// What an assignment writes to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "k", rename_all = "snake_case")]
pub enum LValue {
    /// A function's own variable (or parameter).
    Local { i: u16 },
    /// A variable of the timeline.
    Var { n: String },
    /// An instance property.
    Prop { at: Place, p: Property },
    /// A text field's text (`field.text = …`, `field.appendText(…)` as `+=`).
    Text { at: Place },
}

/// An expression.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "e", rename_all = "snake_case")]
pub enum Expr {
    Lit { v: Value },
    Local { i: u16 },
    Var { n: String },
    Get { at: Place, p: Prop },
    /// The keyboard event's `keyCode`.
    Key,
    /// The event a listener handles, passed on to a function's event parameter.
    Event,
    Un { op: UnOp, x: Box<Expr> },
    Bin { op: BinOp, a: Box<Expr>, b: Box<Expr> },
    /// `a && b`, `a || b` (the operand's value, as ECMAScript gives it).
    And { a: Box<Expr>, b: Box<Expr> },
    Or { a: Box<Expr>, b: Box<Expr> },
    Cond { c: Box<Expr>, a: Box<Expr>, b: Box<Expr> },
    Math { f: MathFn, args: Vec<Expr> },
    /// A call to a function of the timeline, by name.
    Call { f: String, args: Vec<Expr> },
    /// Whether two names are the same instance (`e.currentTarget == btn_a`); `ne` negates.
    Same { a: Place, b: Place, ne: bool },
    /// `to = v` or `to op= v`; its value is what was written.
    Set { to: LValue, op: Option<BinOp>, v: Box<Expr> },
    /// `++` / `--` (`d` is +1 or -1); `post`: the value before.
    Step { to: LValue, d: i8, post: bool },
    /// `int(x)`, `Number(x)`, `String(x)`, `Boolean(x)`, `uint(x)`.
    Cast { t: Ty, x: Box<Expr> },
}

impl Expr {
    pub fn lit(v: Value) -> Expr {
        Expr::Lit { v }
    }
}

/// A timeline method.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    Stop,
    Play,
    GotoAndPlay,
    GotoAndStop,
    NextFrame,
    PrevFrame,
    NextScene,
    PrevScene,
}

impl Method {
    pub fn from_name(s: &str) -> Option<Method> {
        Some(match s {
            "stop" => Method::Stop,
            "play" => Method::Play,
            "gotoAndPlay" => Method::GotoAndPlay,
            "gotoAndStop" => Method::GotoAndStop,
            "nextFrame" => Method::NextFrame,
            "prevFrame" => Method::PrevFrame,
            "nextScene" => Method::NextScene,
            "prevScene" => Method::PrevScene,
            _ => return None,
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            Method::Stop => "stop",
            Method::Play => "play",
            Method::GotoAndPlay => "gotoAndPlay",
            Method::GotoAndStop => "gotoAndStop",
            Method::NextFrame => "nextFrame",
            Method::PrevFrame => "prevFrame",
            Method::NextScene => "nextScene",
            Method::PrevScene => "prevScene",
        }
    }
    /// The action it is, at a frame (go-tos) and scene.
    pub fn action(self, target: Target, frame: FrameRef, scene: Option<String>) -> crate::Action {
        use crate::Action as A;
        match self {
            Method::Stop => A::Stop { target },
            Method::Play => A::Play { target },
            Method::GotoAndPlay => A::GotoAndPlay { target, frame, scene },
            Method::GotoAndStop => A::GotoAndStop { target, frame, scene },
            Method::NextFrame => A::NextFrame { target },
            Method::PrevFrame => A::PrevFrame { target },
            Method::NextScene => A::NextScene,
            Method::PrevScene => A::PrevScene,
        }
    }
}

/// An event a listener waits for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListenEvent {
    Click,
    Press,
    Release,
    Over,
    Out,
    /// `Event.ENTER_FRAME`: every frame, playing or not.
    EnterFrame,
    /// `KeyboardEvent.KEY_DOWN` / `KEY_UP` on the stage.
    KeyDown,
    KeyUp,
}

impl ListenEvent {
    pub fn key(self) -> &'static str {
        match self {
            ListenEvent::Click => "click",
            ListenEvent::Press => "press",
            ListenEvent::Release => "release",
            ListenEvent::Over => "over",
            ListenEvent::Out => "out",
            ListenEvent::EnterFrame => "enter_frame",
            ListenEvent::KeyDown => "key_down",
            ListenEvent::KeyUp => "key_up",
        }
    }
    /// The pointer event it is, for mouse listeners.
    pub fn mouse(self) -> Option<crate::ButtonEvent> {
        use crate::ButtonEvent as B;
        Some(match self {
            ListenEvent::Click => B::Click,
            ListenEvent::Press => B::Press,
            ListenEvent::Release => B::Release,
            ListenEvent::Over => B::Over,
            ListenEvent::Out => B::Out,
            _ => return None,
        })
    }
    pub fn from_mouse(b: crate::ButtonEvent) -> ListenEvent {
        use crate::ButtonEvent as B;
        match b {
            B::Click => ListenEvent::Click,
            B::Press => ListenEvent::Press,
            B::Release => ListenEvent::Release,
            B::Over => ListenEvent::Over,
            B::Out => ListenEvent::Out,
        }
    }
    pub fn is_key(self) -> bool {
        matches!(self, ListenEvent::KeyDown | ListenEvent::KeyUp)
    }
}

/// The function a listener runs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FuncRef {
    /// A function of the timeline, by name.
    Named(String),
    /// A function written in place: the index in the enclosing code's `inner` list.
    Inline(u16),
}

/// A statement, with the line it starts on (for messages).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stmt {
    pub l: u32,
    #[serde(flatten)]
    pub s: S,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "s", rename_all = "snake_case")]
pub enum S {
    /// An expression for its effect.
    Do { x: Expr },
    If { c: Expr, a: Vec<Stmt>, b: Vec<Stmt> },
    /// `while (c)`, `for (; c; next)` (`first`: the condition is tested before the body) and
    /// `do … while (c)`.
    Loop {
        c: Option<Expr>,
        body: Vec<Stmt>,
        next: Option<Expr>,
        first: bool,
    },
    Break,
    Continue,
    Return { v: Option<Expr> },
    /// A timeline method on an instance; `args` are the frame and the scene, when it takes them.
    Act { at: Place, m: Method, args: Vec<Expr> },
    Trace { args: Vec<Expr> },
    /// `addEventListener` (`add`) or `removeEventListener`.
    Listen { at: Place, ev: ListenEvent, f: FuncRef, add: bool },
}

/// A parameter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Param {
    pub t: Ty,
    /// The value when a call leaves it out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub d: Option<Value>,
    /// It is the event (typed `…Event`): its fields read the event the listener handles.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ev: bool,
}

/// A function: its parameters, its own variables (parameters first, each slot's type), what it
/// returns and its body.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Function {
    pub name: String,
    pub line: u32,
    pub params: Vec<Param>,
    pub locals: Vec<Ty>,
    /// `None`: `void`, or no result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ret: Option<Ty>,
    pub body: Vec<Stmt>,
    /// Functions written in place inside it (listeners).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inner: Vec<Arc<Function>>,
}

impl Function {
    /// Whether the function's first parameter takes an event (it can be a listener).
    pub fn takes_event(&self) -> bool {
        let required = self.params.iter().filter(|p| p.d.is_none()).count();
        required <= 1 && self.params.first().is_some_and(|p| p.ev || p.t == Ty::Any)
    }
}

/// The keyboard key codes ActionScript names (`Keyboard.LEFT`, …): the public `flash.ui.Keyboard`
/// constants scripts compare `e.keyCode` with.
pub fn key_code(name: &str) -> Option<u32> {
    let one = |s: &str| {
        let mut c = s.chars();
        match (c.next(), c.next()) {
            (Some(ch), None) => Some(ch),
            _ => None,
        }
    };
    if let Some(c) = one(name).filter(char::is_ascii_uppercase) {
        return Some(u32::from(c));
    }
    if let Some(d) = name.strip_prefix("NUMBER_").and_then(one).and_then(|c| c.to_digit(10)) {
        return Some(48 + d);
    }
    if let Some(d) = name.strip_prefix("NUMPAD_").and_then(one).and_then(|c| c.to_digit(10)) {
        return Some(96 + d);
    }
    if let Some(n) = name.strip_prefix('F').and_then(|n| n.parse::<u32>().ok()).filter(|n| (1..=15).contains(n)) {
        return Some(111 + n);
    }
    Some(match name {
        "BACKSPACE" => 8,
        "TAB" => 9,
        "ENTER" => 13,
        "SHIFT" => 16,
        "CONTROL" => 17,
        "ALTERNATE" => 18,
        "CAPS_LOCK" => 20,
        "ESCAPE" => 27,
        "SPACE" => 32,
        "PAGE_UP" => 33,
        "PAGE_DOWN" => 34,
        "END" => 35,
        "HOME" => 36,
        "LEFT" => 37,
        "UP" => 38,
        "RIGHT" => 39,
        "DOWN" => 40,
        "INSERT" => 45,
        "DELETE" => 46,
        "NUMPAD_MULTIPLY" => 106,
        "NUMPAD_ADD" => 107,
        "NUMPAD_ENTER" => 108,
        "NUMPAD_SUBTRACT" => 109,
        "NUMPAD_DECIMAL" => 110,
        "NUMPAD_DIVIDE" => 111,
        "SEMICOLON" => 186,
        "EQUAL" => 187,
        "COMMA" => 188,
        "MINUS" => 189,
        "PERIOD" => 190,
        "SLASH" => 191,
        "BACKQUOTE" => 192,
        "LEFTBRACKET" => 219,
        "BACKSLASH" => 220,
        "RIGHTBRACKET" => 221,
        "QUOTE" => 222,
        _ => return None,
    })
}
