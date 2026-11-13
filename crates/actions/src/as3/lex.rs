//! The lexer: source text to tokens with their line and column (ECMA-262 3rd edition §7, with
//! ActionScript 3.0's extra punctuators). Bounded: it stops with a problem after
//! [`super::MAX_TOKENS`] tokens.

use super::{MAX_TOKENS, Pos, Problem};

/// A token's kind.
#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    /// An identifier or a keyword (the parser tells them apart).
    Name(String),
    Num(f64),
    Str(String),
    /// A punctuator, as written (`(`, `===`, `>>>=`, …).
    Punct(&'static str),
    Eof,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub pos: Pos,
    /// A line break comes before it (for semicolon insertion).
    pub nl: bool,
}

/// Longest first, so the scan takes the longest punctuator that matches.
const PUNCTS: [&str; 54] = [
    ">>>=", "...", "===", "!==", ">>>", "<<=", ">>=", "&&=", "||=", "::", "..", "==", "!=", "<=", ">=", "&&", "||", "++", "--", "<<", ">>", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", ".<", "{", "}", "(", ")", "[", "]", ";", ",", "<", ">", "+", "-", "*", "/", "%", "&", "|", "^", "!", "~", "?", ":", "=", ".",
];

fn ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == '$'
}

fn ident_part(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

struct Lexer<'a> {
    src: &'a str,
    /// Byte offset of the next character.
    at: usize,
    line: u32,
    col: u32,
}

impl Lexer<'_> {
    fn peek(&self) -> Option<char> {
        self.src.get(self.at..).and_then(|s| s.chars().next())
    }

    fn peek2(&self) -> Option<char> {
        self.src.get(self.at..).and_then(|s| s.chars().nth(1))
    }

    fn pos(&self) -> Pos {
        Pos { line: self.line, column: self.col }
    }

    /// Takes one character; a CR LF pair counts as one line break.
    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.at += c.len_utf8();
        match c {
            '\r' if self.peek() == Some('\n') => {}
            '\n' | '\r' | '\u{2028}' | '\u{2029}' => {
                self.line = self.line.saturating_add(1);
                self.col = 1;
            }
            _ => self.col = self.col.saturating_add(1),
        }
        Some(c)
    }

    /// Skips white space and comments; `true` when a line break was among them.
    fn skip(&mut self) -> Result<bool, Problem> {
        let mut nl = false;
        loop {
            match self.peek() {
                Some('\n' | '\r' | '\u{2028}' | '\u{2029}') => {
                    nl = true;
                    self.bump();
                }
                Some(c) if c.is_whitespace() || c == '\u{feff}' => {
                    self.bump();
                }
                Some('/') if self.peek2() == Some('/') => {
                    while self.peek().is_some_and(|c| !matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')) {
                        self.bump();
                    }
                }
                Some('/') if self.peek2() == Some('*') => {
                    let start = self.pos();
                    self.bump();
                    self.bump();
                    loop {
                        match self.bump() {
                            None => return Err(Problem::at(start, "this comment is never closed (a `*/` is missing)")),
                            Some('*') if self.peek() == Some('/') => {
                                self.bump();
                                break;
                            }
                            Some('\n' | '\r' | '\u{2028}' | '\u{2029}') => nl = true,
                            Some(_) => {}
                        }
                    }
                }
                _ => return Ok(nl),
            }
        }
    }

    fn number(&mut self, start: Pos) -> Result<f64, Problem> {
        let from = self.at;
        if self.peek() == Some('0') && matches!(self.peek2(), Some('x' | 'X')) {
            self.bump();
            self.bump();
            let mut v = 0f64;
            let mut digits = 0;
            while let Some(d) = self.peek().and_then(|c| c.to_digit(16)) {
                v = v * 16.0 + f64::from(d);
                digits += 1;
                self.bump();
            }
            if digits == 0 {
                return Err(Problem::at(start, "a hexadecimal number needs digits after `0x`"));
            }
            if self.peek().is_some_and(ident_part) {
                return Err(Problem::at(self.pos(), "a number can't run straight into a name"));
            }
            return Ok(v);
        }
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.bump();
        }
        if self.peek() == Some('.') && self.peek2().is_none_or(|c| c != '.') {
            self.bump();
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.bump();
            }
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            let save = (self.at, self.line, self.col);
            self.bump();
            if matches!(self.peek(), Some('+' | '-')) {
                self.bump();
            }
            if !self.peek().is_some_and(|c| c.is_ascii_digit()) {
                (self.at, self.line, self.col) = save;
                return Err(Problem::at(self.pos(), "a number's exponent needs digits"));
            }
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.bump();
            }
        }
        if self.peek().is_some_and(ident_part) {
            return Err(Problem::at(self.pos(), "a number can't run straight into a name"));
        }
        let text = self.src.get(from..self.at).unwrap_or("0");
        text.parse::<f64>().map_err(|_| Problem::at(start, "this isn't a number"))
    }

    fn string(&mut self, quote: char, start: Pos) -> Result<String, Problem> {
        self.bump();
        let mut s = String::new();
        loop {
            let here = self.pos();
            match self.bump() {
                None | Some('\n' | '\r' | '\u{2028}' | '\u{2029}') => return Err(Problem::at(start, "this string is never closed (its quote is missing on this line)")),
                Some(c) if c == quote => return Ok(s),
                Some('\\') => match self.bump() {
                    None => return Err(Problem::at(start, "this string is never closed")),
                    Some('n') => s.push('\n'),
                    Some('t') => s.push('\t'),
                    Some('r') => s.push('\r'),
                    Some('b') => s.push('\u{8}'),
                    Some('f') => s.push('\u{c}'),
                    Some('v') => s.push('\u{b}'),
                    Some('0') if !self.peek().is_some_and(|c| c.is_ascii_digit()) => s.push('\0'),
                    Some('x') => s.push(self.hex(2, here)?),
                    Some('u') => s.push(self.hex(4, here)?),
                    // A backslash before a line break continues the string on the next line.
                    Some('\n' | '\u{2028}' | '\u{2029}') => {}
                    Some('\r') => {
                        if self.peek() == Some('\n') {
                            self.bump();
                        }
                    }
                    Some(c) => s.push(c),
                },
                Some(c) => s.push(c),
            }
        }
    }

    fn hex(&mut self, n: usize, at: Pos) -> Result<char, Problem> {
        let mut v = 0u32;
        for _ in 0..n {
            let d = self.peek().and_then(|c| c.to_digit(16)).ok_or_else(|| Problem::at(at, "this escape needs hexadecimal digits"))?;
            v = v * 16 + d;
            self.bump();
        }
        char::from_u32(v).ok_or_else(|| Problem::at(at, "this escape isn't a character"))
    }
}

/// The tokens of `src`, ending with [`Tok::Eof`].
pub fn tokens(src: &str) -> Result<Vec<Token>, Problem> {
    let mut lx = Lexer { src, at: 0, line: 1, col: 1 };
    let mut out = Vec::new();
    loop {
        let nl = lx.skip()?;
        let pos = lx.pos();
        if out.len() >= MAX_TOKENS {
            return Err(Problem::at(pos, format!("the script is longer than {MAX_TOKENS} tokens")));
        }
        let Some(c) = lx.peek() else {
            out.push(Token { tok: Tok::Eof, pos, nl: true });
            return Ok(out);
        };
        let tok = if ident_start(c) {
            let from = lx.at;
            while lx.peek().is_some_and(ident_part) {
                lx.bump();
            }
            Tok::Name(src.get(from..lx.at).unwrap_or_default().to_string())
        } else if c.is_ascii_digit() || (c == '.' && lx.peek2().is_some_and(|d| d.is_ascii_digit())) {
            Tok::Num(lx.number(pos)?)
        } else if c == '"' || c == '\'' {
            Tok::Str(lx.string(c, pos)?)
        } else {
            let rest = src.get(lx.at..).unwrap_or_default();
            let Some(p) = PUNCTS.iter().find(|p| rest.starts_with(**p)) else {
                return Err(Problem::at(pos, format!("unexpected character {c:?}")));
            };
            for _ in 0..p.chars().count() {
                lx.bump();
            }
            Tok::Punct(p)
        };
        out.push(Token { tok, pos, nl });
    }
}
