//! The ActionScript 3.0 timeline subset (ADR 0013 tier 2): a hand-written lexer and
//! recursive-descent parser, and a lowering to the action IR.
//!
//! What runs is what most Animate frame scripts do: `stop()`, `play()`, `gotoAndPlay` and
//! `gotoAndStop` by frame number or label (and scene), `nextFrame()`, `prevFrame()`,
//! `nextScene()` and `prevScene()`, on this timeline or on instance-name paths
//! (`hero_mc.gotoAndStop("run")`, `MovieClip(this.parent).play()`); mouse listeners
//! (`btn.addEventListener(MouseEvent.CLICK, onClick)` with `function onClick(e:MouseEvent):void
//! { … }` made of those calls, or an inline function); `trace` of literals and their `+`; and
//! setting `x`, `y`, `alpha`, `visible`, `rotation`, `scaleX` and `scaleY` on named instances to
//! values. A function declared in the script and called from it runs in place.
//!
//! Anything else is a [`Problem`] with its line and column, and the script doesn't run at all:
//! nothing is ever half-run. Calls that would reach outside the movie (`navigateToURL`,
//! `fscommand`, `ExternalInterface`) are ignored and listed as notes.
//!
//! Learned from the ECMAScript 3rd edition standard (ECMA-262) and Adobe's public ActionScript
//! 3.0 language reference; no Adobe code. Everything is bounded: source length, token count,
//! nesting depth, inlining depth, actions, listeners and problems.

mod lex;
mod lower;
mod parse;

use crate::{Action, ButtonEvent, Target};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Longest script that runs, in bytes (far more than frame scripts written by hand).
pub const MAX_RUN_BYTES: usize = 256 * 1024;
/// Most tokens in a script that runs.
pub const MAX_TOKENS: usize = 65_536;
/// Deepest nesting of statements and expressions.
pub const MAX_DEPTH: usize = 64;
/// Deepest chain of calls to the script's own functions.
pub const MAX_INLINE_DEPTH: usize = 16;
/// Most listeners one script adds.
pub const MAX_LISTENERS: usize = 64;
/// Most problems reported for one script.
pub const MAX_PROBLEMS: usize = 32;

/// A place in the script: 1-based line and column (in characters; a tab is one).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Pos {
    pub line: u32,
    pub column: u32,
}

/// Why a script doesn't run (or, as a note, what in it is ignored). `line` and `column` are 0
/// when it is about the whole script.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Problem {
    pub line: u32,
    pub column: u32,
    pub message: String,
}

impl Problem {
    pub fn at(pos: Pos, message: impl Into<String>) -> Problem {
        Problem { line: pos.line, column: pos.column, message: message.into() }
    }
    pub fn whole(message: impl Into<String>) -> Problem {
        Problem { line: 0, column: 0, message: message.into() }
    }
    pub fn pos(&self) -> Option<Pos> {
        (self.line > 0).then_some(Pos { line: self.line, column: self.column })
    }
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.line > 0 { write!(f, "line {}, column {}: {}", self.line, self.column, self.message) } else { f.write_str(&self.message) }
    }
}

/// A mouse listener a script adds: when `event` happens on `target` (a button or movie clip,
/// named from the script's timeline), `actions` run on the script's timeline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Listener {
    #[serde(default, skip_serializing_if = "Target::is_this")]
    pub target: Target,
    pub event: ButtonEvent,
    /// The function's name (`function at line 7` for an inline one): adding the same function
    /// for the same event again changes nothing, as in ActionScript.
    pub function: String,
    pub actions: Arc<Vec<Action>>,
}

impl Listener {
    pub fn describe(&self) -> String {
        let body = if self.actions.is_empty() { "nothing".to_string() } else { self.actions.iter().map(Action::describe).collect::<Vec<_>>().join("; ") };
        format!("On {} ({}): {body}", self.event.key(), self.target)
    }
}

/// One step of a script, in source order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptOp {
    Action(Action),
    Listen(Listener),
}

impl ScriptOp {
    pub fn describe(&self) -> String {
        match self {
            ScriptOp::Action(a) => a.describe(),
            ScriptOp::Listen(l) => l.describe(),
        }
    }
}

/// A script that runs: its steps, and what in it is ignored.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Compiled {
    pub ops: Vec<ScriptOp>,
    /// Calls that are ignored (they would reach outside the movie, or do nothing here).
    pub notes: Vec<Problem>,
}

/// Compiles ActionScript 3.0 source to the action IR, or says why it doesn't run (the
/// problems sorted by position, at most [`MAX_PROBLEMS`]).
pub fn compile(src: &str) -> Result<Compiled, Vec<Problem>> {
    if src.len() > MAX_RUN_BYTES {
        return Err(vec![Problem::whole(format!("the script is longer than {} KiB, the most that runs", MAX_RUN_BYTES / 1024))]);
    }
    let toks = lex::tokens(src).map_err(|p| vec![p])?;
    let prog = parse::parse(&toks).map_err(|p| vec![p])?;
    lower::lower(&prog)
}

#[cfg(test)]
mod tests;
