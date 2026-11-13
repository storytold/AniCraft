//! The ActionScript 3.0 subset (ADR 0013 tiers 2 and 2.5): a hand-written lexer and
//! recursive-descent parser, a lowering to a small typed script IR ([`ir`]), and the evaluator
//! that runs it ([`run`]) over the player.
//!
//! What runs is what Animate frame scripts do: timeline control (`stop()`, `play()`,
//! `gotoAndPlay`/`gotoAndStop` by frame or label and scene, `nextFrame()` … on this timeline or
//! on instance-name paths); **typed variables** (`var n:int = 0;` as Number, int, uint, Boolean,
//! String or untyped), kept per timeline instance across its frame scripts as ActionScript 3.0
//! timelines keep them; **expressions** (arithmetic, comparison, logic, joining text,
//! `Math.min/max/abs/floor/ceil/round/random`, `int(…)` and the other casts); `if`/`else`,
//! `for`, `while` and `do` loops; **reading properties** of named instances and of the
//! timeline (`x`, `y`, `alpha`, `visible`, `rotation`, `scaleX/Y`, `currentFrame`,
//! `totalFrames`, `currentLabel`, `mouseX/Y`, `name`); setting the instance ones; functions
//! with parameters and results, called from any frame script of the same timeline; listeners
//! (`addEventListener` and `removeEventListener`) for mouse events on buttons and movie clips,
//! `Event.ENTER_FRAME`, and `KeyboardEvent.KEY_DOWN`/`KEY_UP` on the stage with `e.keyCode`;
//! `e.target` and `e.currentTarget`; and `trace`.
//!
//! Anything else is a [`Problem`] with its line and column, and the script doesn't run at all.
//! Calls that would reach outside the movie (`navigateToURL`, `fscommand`, `ExternalInterface`)
//! are ignored and listed as notes. A compiled script names the variables and functions it
//! uses from other frames ([`Compiled::free`]); the timeline must declare them
//! ([`Decls::problems`]) or the script doesn't run either.
//!
//! Learned from the ECMAScript 3rd edition standard (ECMA-262) and Adobe's public ActionScript
//! 3.0 language reference; no Adobe code. Everything is bounded: source length, token count,
//! nesting depth, listeners and problems here; fuel, call depth and text length when it runs.

pub mod ir;
mod lex;
mod lower;
mod parse;
pub mod run;

use ir::{Function, Ty};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Longest script that runs, in bytes (far more than frame scripts written by hand).
pub const MAX_RUN_BYTES: usize = 256 * 1024;
/// Most tokens in a script that runs.
pub const MAX_TOKENS: usize = 65_536;
/// Deepest nesting of statements and expressions.
pub const MAX_DEPTH: usize = 64;
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

/// A name a script uses that it doesn't declare: a variable or function of another frame
/// script of the same timeline.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Free {
    pub name: String,
    pub line: u32,
    pub column: u32,
    /// Called (a function), or read and written (a variable).
    pub call: bool,
}

/// A script that runs.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Compiled {
    /// The top-level statements, in order.
    pub body: Vec<ir::Stmt>,
    /// Functions written in place at the top level (listeners).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inner: Vec<Arc<Function>>,
    /// The functions it declares (the timeline's, callable from its other frames).
    pub functions: Vec<Arc<Function>>,
    /// The variables it declares (the timeline's), with their types.
    pub vars: Vec<(String, Ty)>,
    /// Names it uses from other frames of the timeline.
    pub free: Vec<Free>,
    /// Calls that are ignored (they would reach outside the movie, or do nothing here).
    pub notes: Vec<Problem>,
    /// Statements it holds, functions included (for "runs, N steps").
    pub steps: usize,
}

/// A timeline's declarations: every function and variable its frame scripts declare (the
/// first one of a name, in frame order, top layer first), as ActionScript 3.0 makes them
/// members of the timeline.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Decls {
    pub functions: BTreeMap<String, Arc<Function>>,
    pub vars: BTreeMap<String, Ty>,
}

impl Decls {
    /// From the timeline's compiled scripts, in frame order (top layer first within a frame).
    pub fn build<'a>(scripts: impl IntoIterator<Item = &'a Compiled>) -> Decls {
        let mut d = Decls::default();
        for c in scripts {
            for f in &c.functions {
                d.functions.entry(f.name.clone()).or_insert_with(|| f.clone());
            }
            for (n, t) in &c.vars {
                d.vars.entry(n.clone()).or_insert(*t);
            }
        }
        d
    }

    /// Why `c` can't run on a timeline with these declarations: a name from another frame that
    /// the timeline doesn't declare, or a declaration another frame already makes differently.
    pub fn problems(&self, c: &Compiled) -> Vec<Problem> {
        let mut out = Vec::new();
        for f in &c.free {
            let pos = Pos { line: f.line, column: f.column };
            let known = if f.call { self.functions.contains_key(&f.name) } else { self.vars.contains_key(&f.name) };
            if !known {
                let msg = match (f.call, self.functions.contains_key(&f.name) || self.vars.contains_key(&f.name)) {
                    (true, true) => format!("`{}` is a variable, not a function", f.name),
                    (false, true) => format!("`{}` is a function; call it, as in `{}()`", f.name, f.name),
                    (true, false) => format!("`{}` isn't a function of this timeline (declare it in one of its frame scripts)", f.name),
                    (false, false) => format!("`{}` isn't a variable of this timeline (declare it with `var` in one of its frame scripts)", f.name),
                };
                out.push(Problem::at(pos, msg));
            }
        }
        for f in &c.functions {
            if self.functions.get(&f.name).is_some_and(|g| !Arc::ptr_eq(f, g) && **f != **g) {
                out.push(Problem::at(Pos { line: f.line, column: 1 }, format!("`{}` is also declared on another frame of this timeline", f.name)));
            }
        }
        for (n, t) in &c.vars {
            if self.vars.get(n).is_some_and(|u| u != t) {
                out.push(Problem::whole(format!("`{n}` is declared with another type on another frame of this timeline")));
            }
        }
        out.truncate(MAX_PROBLEMS);
        out
    }
}

/// Compiles ActionScript 3.0 source to the script IR, or says why it doesn't run (the problems
/// sorted by position, at most [`MAX_PROBLEMS`]).
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
