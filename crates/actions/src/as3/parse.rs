//! The parser: tokens to a syntax tree, by recursive descent (the grammar of ECMA-262 3rd
//! edition §11–14, with ActionScript 3.0's type annotations, `import`, `as`, `is`, rest
//! parameters and `for each`). It reads more of the language than AniCraft runs, so lowering
//! can say exactly which construct isn't run and where; what it reads but never runs (`for in`
//! heads, `switch` and `try` bodies) is skipped by balanced brackets. Bounded: nesting
//! deeper than [`super::MAX_DEPTH`] is a problem, not a stack overflow.

use super::lex::{Tok, Token};
use super::{MAX_DEPTH, Pos, Problem};

#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub pos: Pos,
    pub kind: E,
}

#[derive(Clone, Debug, PartialEq)]
pub enum E {
    Num(f64),
    Str(String),
    Bool(bool),
    Null,
    This,
    Name(String),
    /// `a.b`
    Member(Box<Expr>, String),
    /// `a[b]`
    Index(Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>),
    New(Box<Expr>, Vec<Expr>),
    /// A prefix operator (`!`, `-`, `typeof`, `++`, …).
    Unary(&'static str, Box<Expr>),
    /// `x++`, `x--`
    Postfix(&'static str, Box<Expr>),
    Binary(&'static str, Box<Expr>, Box<Expr>),
    /// `=`, `+=`, …
    Assign(&'static str, Box<Expr>, Box<Expr>),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    Function(Box<Func>),
    Array(Vec<Expr>),
    Object(Vec<(String, Expr)>),
    /// `a, b`
    Comma(Vec<Expr>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub name: String,
    pub pos: Pos,
    /// The type annotation as written (`int`, `MouseEvent`, `*`), when there is one.
    pub ty: Option<(String, Pos)>,
    pub default: Option<Expr>,
    pub rest: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Func {
    pub name: Option<String>,
    pub pos: Pos,
    pub params: Vec<Param>,
    /// The result type as written (`void`, `int`), when there is one.
    pub ret: Option<(String, Pos)>,
    pub body: Vec<Stmt>,
}

/// One name of a `var` (or `const`) statement.
#[derive(Clone, Debug, PartialEq)]
pub struct VarDecl {
    pub name: String,
    pub pos: Pos,
    pub ty: Option<(String, Pos)>,
    pub init: Option<Expr>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stmt {
    pub pos: Pos,
    pub kind: S,
}

#[derive(Clone, Debug, PartialEq)]
pub enum S {
    Expr(Expr),
    Import,
    Function(Func),
    /// `var`, `const`.
    Var(&'static str, Vec<VarDecl>),
    If(Expr, Vec<Stmt>, Vec<Stmt>),
    /// `for (init; cond; update) body` (`init` a `var` statement or an expression).
    For(Option<Box<Stmt>>, Option<Expr>, Option<Expr>, Vec<Stmt>),
    While(Expr, Vec<Stmt>),
    DoWhile(Vec<Stmt>, Expr),
    /// `break` and `continue` (without a label).
    Break,
    Continue,
    Return(Option<Expr>),
    Block(Vec<Stmt>),
    Empty,
    /// A statement the parser reads but lowering never runs, by its keyword (`for`, `while`,
    /// `switch`, …), with the statements inside it.
    Other(&'static str, Vec<Stmt>),
}

/// Words that can't name a variable or function (ActionScript 3.0's reserved words).
const RESERVED: [&str; 46] = [
    "as", "break", "case", "catch", "class", "const", "continue", "default", "delete", "do", "else", "extends", "false", "finally", "for", "function", "if", "implements", "import", "in", "instanceof", "interface", "internal", "is", "native", "new", "null", "package", "private", "protected", "public", "return", "super", "switch", "this", "throw", "to", "true", "try", "typeof", "use", "var", "void", "while", "with", "include",
];

/// Attribute words that may stand before a declaration in a frame script.
const ATTRIBUTES: [&str; 9] = ["public", "private", "protected", "internal", "static", "override", "final", "dynamic", "native"];

pub fn reserved(s: &str) -> bool {
    RESERVED.contains(&s)
}

struct Parser<'a> {
    toks: &'a [Token],
    i: usize,
    depth: usize,
}

type R<T> = Result<T, Problem>;

/// What a token reads as in a message.
fn shown(t: &Tok) -> String {
    match t {
        Tok::Name(n) => format!("`{n}`"),
        Tok::Num(_) => "a number".into(),
        Tok::Str(_) => "a string".into(),
        Tok::Punct(p) => format!("`{p}`"),
        Tok::Eof => "the end of the script".into(),
    }
}

impl<'a> Parser<'a> {
    fn tok(&self) -> &'a Tok {
        self.toks.get(self.i).map_or(&Tok::Eof, |t| &t.tok)
    }

    fn tok_at(&self, k: usize) -> &'a Tok {
        self.toks.get(self.i + k).map_or(&Tok::Eof, |t| &t.tok)
    }

    fn pos(&self) -> Pos {
        self.toks.get(self.i).or(self.toks.last()).map_or(Pos { line: 1, column: 1 }, |t| t.pos)
    }

    fn nl(&self) -> bool {
        self.toks.get(self.i).is_none_or(|t| t.nl)
    }

    fn next(&mut self) -> &'a Tok {
        let t = self.tok();
        if self.i < self.toks.len() {
            self.i += 1;
        }
        t
    }

    fn is(&self, p: &str) -> bool {
        matches!(self.tok(), Tok::Punct(q) if *q == p)
    }

    fn is_word(&self, w: &str) -> bool {
        matches!(self.tok(), Tok::Name(n) if n == w)
    }

    fn eat(&mut self, p: &str) -> bool {
        if self.is(p) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn eat_word(&mut self, w: &str) -> bool {
        if self.is_word(w) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, p: &str, what: &str) -> R<()> {
        if self.eat(p) { Ok(()) } else { Err(self.unexpected(&format!("`{p}` {what}"))) }
    }

    fn unexpected(&self, wanted: &str) -> Problem {
        Problem::at(self.pos(), format!("expected {wanted}, found {}", shown(self.tok())))
    }

    /// Enters one level of nesting.
    fn deeper(&mut self) -> R<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH { Err(Problem::at(self.pos(), format!("the code is nested more than {MAX_DEPTH} levels deep"))) } else { Ok(()) }
    }

    fn name(&mut self, what: &str) -> R<(String, Pos)> {
        let pos = self.pos();
        match self.tok() {
            Tok::Name(n) if !reserved(n) => {
                self.i += 1;
                Ok((n.clone(), pos))
            }
            _ => Err(self.unexpected(what)),
        }
    }

    /// The end of a statement: `;`, or a line break, `}` or the end (semicolon insertion).
    fn end(&mut self) -> R<()> {
        if self.eat(";") || self.is("}") || matches!(self.tok(), Tok::Eof) || self.nl() {
            Ok(())
        } else {
            Err(self.unexpected("`;` or a new line"))
        }
    }

    /// Skips a balanced `( … )`, `[ … ]` or `{ … }` starting at the current token.
    fn skip_group(&mut self) -> R<()> {
        let start = self.pos();
        let mut stack: Vec<&str> = Vec::new();
        loop {
            match self.next() {
                Tok::Punct(p @ ("(" | "[" | "{")) => {
                    stack.push(match *p {
                        "(" => ")",
                        "[" => "]",
                        _ => "}",
                    });
                    if stack.len() > MAX_DEPTH {
                        return Err(Problem::at(start, format!("the code is nested more than {MAX_DEPTH} levels deep")));
                    }
                }
                Tok::Punct(p @ (")" | "]" | "}")) => {
                    if stack.pop() != Some(*p) {
                        return Err(Problem::at(self.toks.get(self.i.saturating_sub(1)).map_or(start, |t| t.pos), format!("`{p}` doesn't match an opening bracket")));
                    }
                    if stack.is_empty() {
                        return Ok(());
                    }
                }
                Tok::Eof => return Err(Problem::at(start, "this bracket is never closed")),
                _ if stack.is_empty() => return Err(Problem::at(start, "expected a bracket")),
                _ => {}
            }
        }
    }

    /// `: Type` (`*`, `void`, `a.b.C`, `Vector.<T>`), as written.
    fn annotation(&mut self) -> R<Option<(String, Pos)>> {
        if !self.eat(":") {
            return Ok(None);
        }
        let pos = self.pos();
        let start = self.i;
        self.type_name()?;
        let text: String = self
            .toks
            .get(start..self.i)
            .unwrap_or_default()
            .iter()
            .map(|t| match &t.tok {
                Tok::Name(n) => n.clone(),
                Tok::Punct(p) => (*p).to_string(),
                _ => String::new(),
            })
            .collect();
        Ok(Some((text, pos)))
    }

    /// The names of a `var` statement after the keyword.
    fn var_list(&mut self) -> R<Vec<VarDecl>> {
        let mut out = Vec::new();
        loop {
            let (name, pos) = self.name("a variable name")?;
            let ty = self.annotation()?;
            let init = if self.eat("=") { Some(self.assign()?) } else { None };
            out.push(VarDecl { name, pos, ty, init });
            if !self.eat(",") {
                return Ok(out);
            }
        }
    }

    /// `( init ; cond ; update )` after `for`, or `None` for `for … in` (read, not run).
    #[allow(clippy::type_complexity)]
    fn for_head(&mut self) -> R<Option<(Option<Box<Stmt>>, Option<Expr>, Option<Expr>)>> {
        let open = self.i;
        self.expect("(", "after `for`")?;
        let pos = self.pos();
        let init = if self.is(";") {
            None
        } else if self.is_word("var") || self.is_word("const") {
            let w = if self.is_word("var") { "var" } else { "const" };
            self.i += 1;
            let list = self.var_list()?;
            if self.is_word("in") || list.iter().any(|d| matches!(&d.init, Some(Expr { kind: E::Binary("in", ..), .. }))) {
                self.i = open;
                self.skip_group()?;
                return Ok(None);
            }
            Some(Box::new(Stmt { pos, kind: S::Var(w, list) }))
        } else {
            let e = self.expr()?;
            if matches!(e.kind, E::Binary("in", ..)) {
                self.i = open;
                self.skip_group()?;
                return Ok(None);
            }
            Some(Box::new(Stmt { pos, kind: S::Expr(e) }))
        };
        self.expect(";", "after the loop's start")?;
        let cond = if self.is(";") { None } else { Some(self.expr()?) };
        self.expect(";", "after the loop's condition")?;
        let update = if self.is(")") { None } else { Some(self.expr()?) };
        self.expect(")", "to close the loop's head")?;
        Ok(Some((init, cond, update)))
    }

    /// `( expr )`
    fn paren(&mut self, what: &str) -> R<Expr> {
        self.expect("(", what)?;
        let e = self.expr()?;
        self.expect(")", "after the condition")?;
        Ok(e)
    }

    fn type_name(&mut self) -> R<()> {
        if self.eat("*") || self.eat_word("void") {
            return Ok(());
        }
        self.name("a type")?;
        loop {
            if self.eat(".<") {
                self.deeper()?;
                self.type_name()?;
                self.depth -= 1;
                // `>>` closes two generics at once.
                if !self.eat(">") {
                    return Err(self.unexpected("`>` to close the type"));
                }
            } else if self.eat(".") {
                self.name("a type")?;
            } else {
                return Ok(());
            }
        }
    }

    fn program(&mut self) -> R<Vec<Stmt>> {
        let mut out = Vec::new();
        while !matches!(self.tok(), Tok::Eof) {
            out.push(self.statement()?);
        }
        Ok(out)
    }

    fn block(&mut self) -> R<Vec<Stmt>> {
        self.expect("{", "to open the block")?;
        let mut out = Vec::new();
        while !self.eat("}") {
            if matches!(self.tok(), Tok::Eof) {
                return Err(self.unexpected("`}` to close the block"));
            }
            out.push(self.statement()?);
        }
        Ok(out)
    }

    /// A statement as a body (`if (…) stmt`): a block's statements, or one statement.
    fn body(&mut self) -> R<Vec<Stmt>> {
        if self.is("{") {
            let pos = self.pos();
            let b = self.block()?;
            Ok(vec![Stmt { pos, kind: S::Block(b) }])
        } else {
            Ok(vec![self.statement()?])
        }
    }

    fn statement(&mut self) -> R<Stmt> {
        self.deeper()?;
        let s = self.statement_inner();
        self.depth -= 1;
        s
    }

    fn statement_inner(&mut self) -> R<Stmt> {
        let pos = self.pos();
        let kind = match self.tok() {
            Tok::Punct(";") => {
                self.i += 1;
                S::Empty
            }
            Tok::Punct("{") => S::Block(self.block()?),
            Tok::Name(w) => match w.as_str() {
                "import" => {
                    self.i += 1;
                    self.name("a package name after `import`")?;
                    while self.eat(".") {
                        if !self.eat("*") {
                            self.name("a name")?;
                        }
                    }
                    self.end()?;
                    S::Import
                }
                "function" if matches!(self.tok_at(1), Tok::Name(_)) => {
                    self.i += 1;
                    S::Function(self.function(pos, true)?)
                }
                "var" | "const" => {
                    let w = if w == "var" { "var" } else { "const" };
                    self.i += 1;
                    let list = self.var_list()?;
                    self.end()?;
                    S::Var(w, list)
                }
                "if" => {
                    self.i += 1;
                    self.expect("(", "after `if`")?;
                    let c = self.expr()?;
                    self.expect(")", "after the condition")?;
                    let then = self.body()?;
                    let other = if self.eat_word("else") { self.body()? } else { Vec::new() };
                    S::If(c, then, other)
                }
                "return" => {
                    self.i += 1;
                    let e = if self.is(";") || self.is("}") || matches!(self.tok(), Tok::Eof) || self.nl() { None } else { Some(self.expr()?) };
                    self.end()?;
                    S::Return(e)
                }
                "for" => {
                    self.i += 1;
                    if self.eat_word("each") {
                        self.skip_group()?;
                        S::Other("for each", self.body()?)
                    } else {
                        match self.for_head()? {
                            Some((init, cond, update)) => S::For(init, cond, update, self.body()?),
                            None => S::Other("for in", self.body()?),
                        }
                    }
                }
                "while" => {
                    self.i += 1;
                    let c = self.paren("after `while`")?;
                    S::While(c, self.body()?)
                }
                "do" => {
                    self.i += 1;
                    let b = self.body()?;
                    if !self.eat_word("while") {
                        return Err(self.unexpected("`while` after the `do` body"));
                    }
                    let c = self.paren("after `while`")?;
                    self.end()?;
                    S::DoWhile(b, c)
                }
                "with" => {
                    self.i += 1;
                    self.skip_group()?;
                    S::Other("with", self.body()?)
                }
                "switch" => {
                    self.i += 1;
                    self.skip_group()?;
                    self.skip_group()?;
                    S::Other("switch", Vec::new())
                }
                "try" => {
                    self.i += 1;
                    let mut inner = self.block()?;
                    let mut any = false;
                    while self.eat_word("catch") {
                        any = true;
                        self.skip_group()?;
                        inner.extend(self.block()?);
                    }
                    if self.eat_word("finally") {
                        any = true;
                        inner.extend(self.block()?);
                    }
                    if !any {
                        return Err(self.unexpected("`catch` or `finally` after the `try` block"));
                    }
                    S::Other("try", inner)
                }
                "break" | "continue" | "throw" => {
                    let w = match w.as_str() {
                        "break" => "break",
                        "continue" => "continue",
                        _ => "throw",
                    };
                    self.i += 1;
                    let bare = self.is(";") || self.is("}") || matches!(self.tok(), Tok::Eof) || self.nl();
                    if !bare {
                        self.expr()?;
                    }
                    self.end()?;
                    match (w, bare) {
                        ("break", true) => S::Break,
                        ("continue", true) => S::Continue,
                        ("throw", _) => S::Other("throw", Vec::new()),
                        _ => S::Other("labelled break", Vec::new()),
                    }
                }
                // Applications, not timeline scripts (ADR 0013 tier 3).
                "package" | "class" | "interface" | "namespace" | "use" | "include" | "default" => {
                    let w = w.clone();
                    return Err(Problem::at(pos, format!("`{w}` belongs to class files and isn't supported in frame scripts")));
                }
                a if ATTRIBUTES.contains(&a) && matches!(self.tok_at(1), Tok::Name(_)) => {
                    let a = a.to_string();
                    return Err(Problem::at(pos, format!("`{a}` declarations belong to classes and aren't supported in frame scripts")));
                }
                _ => {
                    let e = self.expr()?;
                    if self.is(":") {
                        return Err(Problem::at(pos, "statement labels aren't supported"));
                    }
                    self.end()?;
                    S::Expr(e)
                }
            },
            _ => {
                let e = self.expr()?;
                self.end()?;
                S::Expr(e)
            }
        };
        Ok(Stmt { pos, kind })
    }

    /// After `function`: an optional name, the parameters, the result type and the body.
    fn function(&mut self, pos: Pos, named: bool) -> R<Func> {
        let name = if named || matches!(self.tok(), Tok::Name(n) if !reserved(n)) { Some(self.name("a function name")?.0) } else { None };
        // Getters and setters are class members.
        if matches!(name.as_deref(), Some("get" | "set")) && matches!(self.tok(), Tok::Name(_)) {
            return Err(Problem::at(pos, "getters and setters belong to classes and aren't supported in frame scripts"));
        }
        self.expect("(", "to open the parameter list")?;
        let mut params = Vec::new();
        if !self.eat(")") {
            loop {
                let rest = self.eat("...");
                let (n, p) = self.name("a parameter name")?;
                let ty = self.annotation()?;
                let default = if self.eat("=") { Some(self.assign()?) } else { None };
                params.push(Param { name: n, pos: p, ty, default, rest });
                if self.eat(")") {
                    break;
                }
                self.expect(",", "or `)` after the parameter")?;
            }
        }
        let ret = self.annotation()?;
        let body = self.block()?;
        Ok(Func { name, pos, params, ret, body })
    }

    /// An expression (`a, b` included).
    fn expr(&mut self) -> R<Expr> {
        let first = self.assign()?;
        if !self.is(",") {
            return Ok(first);
        }
        let pos = first.pos;
        let mut list = vec![first];
        while self.eat(",") {
            list.push(self.assign()?);
        }
        Ok(Expr { pos, kind: E::Comma(list) })
    }

    fn assign(&mut self) -> R<Expr> {
        self.deeper()?;
        let r = self.assign_inner();
        self.depth -= 1;
        r
    }

    fn assign_inner(&mut self) -> R<Expr> {
        let lhs = self.conditional()?;
        let op = match self.tok() {
            Tok::Punct(p @ ("=" | "+=" | "-=" | "*=" | "/=" | "%=" | "<<=" | ">>=" | ">>>=" | "&=" | "|=" | "^=" | "&&=" | "||=")) => *p,
            _ => return Ok(lhs),
        };
        let pos = self.pos();
        if !matches!(lhs.kind, E::Name(_) | E::Member(..) | E::Index(..)) {
            return Err(Problem::at(pos, "the left side of this assignment can't be assigned to"));
        }
        self.i += 1;
        let rhs = self.assign()?;
        Ok(Expr { pos: lhs.pos, kind: E::Assign(op, Box::new(lhs), Box::new(rhs)) })
    }

    fn conditional(&mut self) -> R<Expr> {
        let c = self.binary(0)?;
        if !self.eat("?") {
            return Ok(c);
        }
        let a = self.assign()?;
        self.expect(":", "in the `? :` expression")?;
        let b = self.assign()?;
        Ok(Expr { pos: c.pos, kind: E::Cond(Box::new(c), Box::new(a), Box::new(b)) })
    }

    /// Binary operators by precedence level (0 = `||`, loosest).
    fn binary(&mut self, level: usize) -> R<Expr> {
        const LEVELS: [&[&str]; 10] = [
            &["||"],
            &["&&"],
            &["|"],
            &["^"],
            &["&"],
            &["==", "!=", "===", "!=="],
            &["<", ">", "<=", ">=", "instanceof", "is", "as", "in"],
            &["<<", ">>", ">>>"],
            &["+", "-"],
            &["*", "/", "%"],
        ];
        let Some(ops) = LEVELS.get(level) else { return self.unary() };
        let mut lhs = self.binary(level + 1)?;
        // Each operator in a row nests the tree one deeper: it counts toward the depth limit,
        // so what runs the tree can't recurse without bound.
        let mut chain = 0;
        let r = loop {
            let op = match self.tok() {
                Tok::Punct(p) if ops.contains(p) => ops.iter().find(|o| *o == p).copied(),
                Tok::Name(n) if ops.contains(&n.as_str()) => ops.iter().find(|o| **o == n.as_str()).copied(),
                _ => None,
            };
            let Some(op) = op else { break Ok(lhs) };
            chain += 1;
            self.depth += 1;
            if self.depth > MAX_DEPTH {
                break Err(Problem::at(self.pos(), format!("this expression has too many operators in a row (code nests at most {MAX_DEPTH} levels deep)")));
            }
            self.i += 1;
            // The right side of `as` and `is` is a type.
            let rhs = if matches!(op, "as" | "is") {
                let pos = self.pos();
                let start = self.i;
                if let Err(e) = self.type_name() {
                    break Err(e);
                }
                let name = match self.toks.get(start..self.i) {
                    Some([Token { tok: Tok::Name(n), .. }]) => n.clone(),
                    _ => "*".into(),
                };
                Expr { pos, kind: E::Name(name) }
            } else {
                match self.binary(level + 1) {
                    Ok(e) => e,
                    Err(e) => break Err(e),
                }
            };
            lhs = Expr { pos: lhs.pos, kind: E::Binary(op, Box::new(lhs), Box::new(rhs)) };
        };
        self.depth -= chain;
        r
    }

    fn unary(&mut self) -> R<Expr> {
        self.deeper()?;
        let r = self.unary_inner();
        self.depth -= 1;
        r
    }

    fn unary_inner(&mut self) -> R<Expr> {
        let pos = self.pos();
        let op = match self.tok() {
            Tok::Punct(p @ ("!" | "~" | "-" | "+" | "++" | "--")) => Some(*p),
            Tok::Name(n) if matches!(n.as_str(), "typeof" | "void" | "delete") => Some(match n.as_str() {
                "typeof" => "typeof",
                "void" => "void",
                _ => "delete",
            }),
            _ => None,
        };
        if let Some(op) = op {
            self.i += 1;
            let e = self.unary()?;
            return Ok(Expr { pos, kind: E::Unary(op, Box::new(e)) });
        }
        let e = self.postfix()?;
        if !self.nl()
            && let Tok::Punct(p @ ("++" | "--")) = self.tok()
        {
            self.i += 1;
            return Ok(Expr { pos, kind: E::Postfix(p, Box::new(e)) });
        }
        Ok(e)
    }

    /// A primary expression with its member accesses, indexes and calls.
    fn postfix(&mut self) -> R<Expr> {
        let mut e = if self.is_word("new") { self.new_expr()? } else { self.primary()? };
        loop {
            let pos = self.pos();
            if self.eat(".") {
                let n = match self.next() {
                    // Any word can follow a dot (`e.default` is a property).
                    Tok::Name(n) => n.clone(),
                    Tok::Punct("(") => {
                        return Err(Problem::at(pos, "XML filters aren't supported"));
                    }
                    t => return Err(Problem::at(pos, format!("expected a property name after `.`, found {}", shown(t)))),
                };
                e = Expr { pos: e.pos, kind: E::Member(Box::new(e), n) };
            } else if self.is("..") || self.is("::") || self.is(".<") {
                let what = match self.next() {
                    Tok::Punct("..") => "XML descendant access (`..`)",
                    Tok::Punct("::") => "namespace-qualified names (`::`)",
                    _ => "typed vectors (`Vector.<T>`)",
                };
                return Err(Problem::at(pos, format!("{what} isn't supported")));
            } else if self.is("[") {
                self.i += 1;
                let ix = self.expr()?;
                self.expect("]", "to close the index")?;
                e = Expr { pos: e.pos, kind: E::Index(Box::new(e), Box::new(ix)) };
            } else if self.is("(") {
                let args = self.arguments()?;
                e = Expr { pos: e.pos, kind: E::Call(Box::new(e), args) };
            } else {
                return Ok(e);
            }
        }
    }

    fn new_expr(&mut self) -> R<Expr> {
        let pos = self.pos();
        self.i += 1;
        self.deeper()?;
        // `new X.Y(args)`: the constructor is a name path (or another `new`).
        let mut callee = if self.is_word("new") { self.new_expr()? } else { self.primary()? };
        while self.eat(".") {
            let (n, _) = match self.tok() {
                Tok::Name(n) => (n.clone(), self.pos()),
                _ => return Err(self.unexpected("a name after `.`")),
            };
            self.i += 1;
            callee = Expr { pos: callee.pos, kind: E::Member(Box::new(callee), n) };
        }
        if self.is(".<") {
            return Err(Problem::at(self.pos(), "typed vectors (`Vector.<T>`) aren't supported"));
        }
        let args = if self.is("(") { self.arguments()? } else { Vec::new() };
        self.depth -= 1;
        Ok(Expr { pos, kind: E::New(Box::new(callee), args) })
    }

    fn arguments(&mut self) -> R<Vec<Expr>> {
        self.expect("(", "")?;
        let mut args = Vec::new();
        if self.eat(")") {
            return Ok(args);
        }
        loop {
            if self.eat("...") {
                return Err(Problem::at(self.pos(), "spreading arguments (`...`) isn't supported"));
            }
            args.push(self.assign()?);
            if self.eat(")") {
                return Ok(args);
            }
            self.expect(",", "or `)` after the argument")?;
        }
    }

    fn primary(&mut self) -> R<Expr> {
        let pos = self.pos();
        let kind = match self.next() {
            Tok::Num(n) => E::Num(*n),
            Tok::Str(s) => E::Str(s.clone()),
            Tok::Name(n) => match n.as_str() {
                "true" => E::Bool(true),
                "false" => E::Bool(false),
                "null" => E::Null,
                "this" => E::This,
                "function" => E::Function(Box::new(self.function(pos, false)?)),
                "super" => return Err(Problem::at(pos, "`super` belongs to classes and isn't supported in frame scripts")),
                w if reserved(w) => return Err(Problem::at(pos, format!("expected an expression, found `{w}`"))),
                _ => E::Name(n.clone()),
            },
            Tok::Punct("(") => {
                let e = self.expr()?;
                self.expect(")", "to close the parenthesis")?;
                // Parentheses only group; the inner expression keeps its own position.
                return Ok(e);
            }
            Tok::Punct("[") => {
                let mut items = Vec::new();
                while !self.eat("]") {
                    if self.eat(",") {
                        continue;
                    }
                    items.push(self.assign()?);
                    if !self.is("]") {
                        self.expect(",", "or `]` in the array")?;
                    }
                }
                E::Array(items)
            }
            Tok::Punct("{") => {
                let mut fields = Vec::new();
                while !self.eat("}") {
                    let key = match self.next() {
                        Tok::Name(n) => n.clone(),
                        Tok::Str(s) => s.clone(),
                        Tok::Num(n) => format!("{n}"),
                        _ => return Err(Problem::at(self.pos(), "expected a field name in the object")),
                    };
                    self.expect(":", "after the field name")?;
                    fields.push((key, self.assign()?));
                    if !self.is("}") {
                        self.expect(",", "or `}` in the object")?;
                    }
                }
                E::Object(fields)
            }
            Tok::Punct("/" | "/=") => return Err(Problem::at(pos, "regular expressions aren't supported")),
            Tok::Punct("<") => return Err(Problem::at(pos, "XML literals aren't supported")),
            t => {
                self.i = self.i.saturating_sub(1);
                return Err(Problem::at(pos, format!("expected an expression, found {}", shown(t))));
            }
        };
        Ok(Expr { pos, kind })
    }
}

/// The statements of a script.
pub fn parse(toks: &[Token]) -> Result<Vec<Stmt>, Problem> {
    Parser { toks, i: 0, depth: 0 }.program()
}
