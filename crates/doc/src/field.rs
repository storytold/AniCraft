//! Text fields while a movie plays (ADR 0066): what a dynamic or input field holds, where its
//! caret and selection are, and the editing that keys and the pointer do to it.
//!
//! Everything here is pure data and pure functions, like the rest of the player: the same field,
//! look and keys always give the same text, caret and selection. Laying text out needs the
//! fonts, which live a layer above (`anicraft-text`), so the player asks its host for a
//! [`FieldLook`] whenever a field's text changes and keeps it in the state; drawing a state then
//! needs nothing else.

use crate::{FIELD_GUTTER, LineType, MAX_FIELD_CHARS, TextBlock, TextField, TextKind};
use anicraft_geom::{Path, Vec2};
use std::sync::Arc;

/// One laid-out line's characters and caret places (indices count `char`s of the shown text).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CaretLine {
    /// The first character on the line and the boundary after its last.
    pub start: u32,
    pub end: u32,
    /// `(index, x)` of the boundaries glyphs start at and of the line's end, ascending; `x` from
    /// the text's left edge (the box's, less the gutter).
    pub stops: Vec<(u32, f32)>,
}

impl CaretLine {
    /// The caret's x at boundary `i`.
    pub fn x_at(&self, i: u32) -> f32 {
        let mut x = self.stops.first().map_or(0.0, |s| s.1);
        for &(c, sx) in &self.stops {
            if c > i {
                break;
            }
            x = sx;
        }
        x
    }

    /// The boundary nearest `x`.
    pub fn index_at(&self, x: f32) -> u32 {
        let mut best = (self.start, f32::INFINITY);
        for &(c, sx) in &self.stops {
            let d = (sx - x).abs();
            if d < best.1 {
                best = (c, d);
            }
        }
        best.0.clamp(self.start, self.end)
    }
}

/// A field's text laid out: its outline (in the box's space, the gutter applied, as a text
/// block's outline is) and where the caret can stand.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldLook {
    pub outline: Arc<Path>,
    /// Top line first; never empty.
    pub lines: Arc<[CaretLine]>,
    /// From one line's top to the next's.
    pub line_height: f32,
    /// The laid-out text's size (it may be bigger than the box, which clips it).
    pub width: f32,
    pub height: f32,
}

impl FieldLook {
    /// The line holding boundary `i`: the last that starts at or before it.
    pub fn line_of(&self, i: u32) -> usize {
        let mut at = 0;
        for (n, l) in self.lines.iter().enumerate() {
            if l.start <= i {
                at = n;
            } else {
                break;
            }
        }
        at
    }

    /// The caret's place at boundary `i`: x from the text's left, and the line's top.
    pub fn caret_at(&self, i: u32) -> Vec2 {
        let n = self.line_of(i);
        let x = self.lines.get(n).map_or(0.0, |l| l.x_at(i));
        Vec2::new(x, n as f32 * self.line_height)
    }

    /// The boundary nearest a point in the text's space.
    pub fn index_at(&self, p: Vec2) -> u32 {
        let h = self.line_height.max(1e-3);
        let row = if p.y.is_finite() { (p.y / h).floor().max(0.0) as usize } else { 0 };
        let row = row.min(self.lines.len().saturating_sub(1));
        self.lines.get(row).map_or(0, |l| l.index_at(p.x))
    }
}

/// What a field holds while the movie plays.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FieldState {
    /// Its text once a script or the viewer changed it; `None`: the timeline's own (so a
    /// keyframe with other words still shows them).
    pub text: Option<String>,
    /// The text laid out (the host's work); `None` while it shows the authored outline.
    pub look: Option<Arc<FieldLook>>,
    /// The caret and the other end of the selection (character boundaries; equal: nothing
    /// selected).
    pub caret: u32,
    pub anchor: u32,
    /// How far the text has scrolled to keep the caret in view (pixels, never negative).
    pub scroll: Vec2,
}

impl FieldState {
    /// The field's text: what was set, or the block's own.
    pub fn value<'a>(&'a self, block: &'a TextBlock) -> &'a str {
        self.text.as_deref().unwrap_or(&block.text)
    }

    /// The selected range, low end first.
    pub fn selection(&self) -> (u32, u32) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }

    /// The selected text.
    pub fn selected(&self, block: &TextBlock) -> String {
        let (a, b) = self.selection();
        slice(self.value(block), a, b).to_string()
    }
}

/// The byte offset of character boundary `i` in `s` (its end when past it).
pub fn byte_at(s: &str, i: u32) -> usize {
    s.char_indices().nth(i as usize).map_or(s.len(), |(b, _)| b)
}

/// The characters of `s` from boundary `a` to `b`.
pub fn slice(s: &str, a: u32, b: u32) -> &str {
    let (x, y) = (byte_at(s, a.min(b)), byte_at(s, a.max(b)));
    s.get(x..y).unwrap_or("")
}

/// Characters in `s`, as boundaries count them.
pub fn chars(s: &str) -> u32 {
    s.chars().count().min(u32::MAX as usize) as u32
}

/// Text a script sets, as a field keeps it: at most [`MAX_FIELD_CHARS`] characters, with `\r`
/// and `\r\n` (ActionScript's line ends) as line breaks.
pub fn fit(field: &TextField, text: &str) -> String {
    let _ = field;
    text.replace("\r\n", "\n").replace('\r', "\n").chars().take(MAX_FIELD_CHARS).collect()
}

/// What an editing key does (the ActionScript key codes the player receives).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Backspace,
    Delete,
    Enter,
    SelectAll,
}

impl Edit {
    /// The edit a key code makes, if any.
    pub fn from_key(code: u32) -> Option<Edit> {
        Some(match code {
            37 => Edit::Left,
            39 => Edit::Right,
            38 => Edit::Up,
            40 => Edit::Down,
            36 => Edit::Home,
            35 => Edit::End,
            8 => Edit::Backspace,
            46 => Edit::Delete,
            13 => Edit::Enter,
            _ => return None,
        })
    }
}

/// What an edit changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Changed {
    pub text: bool,
    pub caret: bool,
}

/// A focused field being edited: its block (style and authored text), its state, and whether
/// the viewer may change the text (input fields) or only select it.
pub struct Editor<'a> {
    pub block: &'a TextBlock,
    pub field: &'a TextField,
    pub st: &'a mut FieldState,
}

impl Editor<'_> {
    fn len(&self) -> u32 {
        chars(self.st.value(self.block))
    }

    fn editable(&self) -> bool {
        self.field.kind == TextKind::Input
    }

    /// Replaces the selection with `s` (typed characters, a paste or a line break).
    pub fn insert(&mut self, s: &str) -> Changed {
        if !self.editable() {
            return Changed::default();
        }
        let one = !self.field.lines.multiline();
        let mut add: String = s.chars().filter(|c| !c.is_control() || (*c == '\n' && !one)).collect();
        let (a, b) = self.st.selection();
        let value = self.st.value(self.block).to_string();
        let kept = chars(&value) - (b - a);
        let room = if self.field.max_chars > 0 { self.field.max_chars.saturating_sub(kept) } else { u32::MAX };
        let room = room.min((MAX_FIELD_CHARS as u32).saturating_sub(kept));
        if chars(&add) > room {
            add = add.chars().take(room as usize).collect();
        }
        if add.is_empty() && a == b {
            return Changed::default();
        }
        let mut next = String::with_capacity(value.len() + add.len());
        next.push_str(slice(&value, 0, a));
        next.push_str(&add);
        next.push_str(slice(&value, b, u32::MAX));
        let caret = a + chars(&add);
        self.st.text = Some(next);
        self.st.caret = caret;
        self.st.anchor = caret;
        Changed { text: true, caret: true }
    }

    /// One editing key; `shift` extends the selection with caret moves.
    pub fn key(&mut self, e: Edit, shift: bool, look: Option<&FieldLook>) -> Changed {
        let len = self.len();
        let (a, b) = self.st.selection();
        let moved = |st: &mut FieldState, to: u32, shift: bool| {
            st.caret = to.min(len);
            if !shift {
                st.anchor = st.caret;
            }
            Changed { text: false, caret: true }
        };
        match e {
            Edit::SelectAll => {
                self.st.anchor = 0;
                self.st.caret = len;
                Changed { text: false, caret: true }
            }
            Edit::Left if !shift && a != b => moved(self.st, a, false),
            Edit::Right if !shift && a != b => moved(self.st, b, false),
            Edit::Left => {
                let to = self.st.caret.saturating_sub(1);
                moved(self.st, to, shift)
            }
            Edit::Right => {
                let to = self.st.caret.saturating_add(1);
                moved(self.st, to, shift)
            }
            Edit::Home | Edit::End | Edit::Up | Edit::Down => {
                let Some(look) = look else {
                    let to = if matches!(e, Edit::Home | Edit::Up) { 0 } else { len };
                    return moved(self.st, to, shift);
                };
                let n = look.line_of(self.st.caret);
                let line = look.lines.get(n);
                let to = match e {
                    Edit::Home => line.map_or(0, |l| l.start),
                    Edit::End => line.map_or(len, |l| l.end),
                    _ => {
                        let x = line.map_or(0.0, |l| l.x_at(self.st.caret));
                        let target = if e == Edit::Up { n.checked_sub(1) } else { Some(n + 1).filter(|m| *m < look.lines.len()) };
                        match target.and_then(|m| look.lines.get(m)) {
                            Some(l) => l.index_at(x),
                            None if e == Edit::Up => 0,
                            None => len,
                        }
                    }
                };
                moved(self.st, to, shift)
            }
            Edit::Backspace | Edit::Delete if !self.editable() => Changed::default(),
            Edit::Backspace | Edit::Delete if a != b => self.insert(""),
            Edit::Backspace => {
                if a == 0 {
                    return Changed::default();
                }
                self.st.anchor = a - 1;
                self.insert("")
            }
            Edit::Delete => {
                if b >= len {
                    return Changed::default();
                }
                self.st.anchor = a;
                self.st.caret = b + 1;
                self.insert("")
            }
            Edit::Enter if self.field.lines.multiline() => self.insert("\n"),
            Edit::Enter => Changed::default(),
        }
    }

    /// The pointer at `p` (the text's space): pressed (`extend`: dragging from an earlier press).
    pub fn point(&mut self, p: Vec2, extend: bool, look: Option<&FieldLook>) -> Changed {
        let i = look.map_or(self.len(), |l| l.index_at(p + self.st.scroll)).min(self.len());
        self.st.caret = i;
        if !extend {
            self.st.anchor = i;
        }
        Changed { text: false, caret: true }
    }
}

/// Keeps the caret in view: scrolls the text so the caret's line fits in the box's height and
/// (when lines don't wrap) the caret in its width.
pub fn scroll_to_caret(block: &TextBlock, field: &TextField, st: &mut FieldState) {
    let Some(look) = st.look.clone() else {
        st.scroll = Vec2::ZERO;
        return;
    };
    let view = Vec2::new((block.box_width - 2.0 * FIELD_GUTTER).max(1.0), (block.box_height - 2.0 * FIELD_GUTTER).max(1.0));
    let c = look.caret_at(st.caret);
    let mut s = st.scroll;
    if field.lines != LineType::Multiline {
        if c.x - s.x > view.x {
            s.x = c.x - view.x;
        }
        if c.x < s.x {
            s.x = c.x;
        }
    } else {
        s.x = 0.0;
    }
    if c.y + look.line_height - s.y > view.y {
        s.y = c.y + look.line_height - view.y;
    }
    if c.y < s.y {
        s.y = c.y;
    }
    // Never past the text's end.
    s.x = s.x.min((look.width - view.x).max(0.0)).max(0.0);
    s.y = s.y.min((look.height - view.y).max(0.0)).max(0.0);
    st.scroll = if s.x.is_finite() && s.y.is_finite() { s } else { Vec2::ZERO };
}

/// A look for tests: every character `cw` wide and `lh` tall (a square per visible one), lines
/// broken at `\n` and, with `wrap`, after as many characters as fit.
#[cfg(test)]
pub(crate) fn mono_look(text: &str, cw: f32, lh: f32, wrap: Option<f32>) -> FieldLook {
    let per = wrap.map(|w| ((w / cw).floor() as u32).max(1));
    let mut lines = Vec::new();
    let mut outline = Path::new();
    let (mut start, mut col, mut stops, mut width) = (0u32, 0u32, vec![(0u32, 0.0f32)], 0.0f32);
    let mut i = 0u32;
    for c in text.chars() {
        if c == '\n' {
            lines.push(CaretLine { start, end: i, stops: std::mem::take(&mut stops) });
            i += 1;
            (start, col) = (i, 0);
            stops.push((i, 0.0));
            continue;
        }
        if per.is_some_and(|p| col == p) {
            lines.push(CaretLine { start, end: i, stops: std::mem::take(&mut stops) });
            (start, col) = (i, 0);
            stops.push((i, 0.0));
        }
        if !c.is_whitespace() {
            let r = Path::rect(FIELD_GUTTER + col as f32 * cw + 1.0, FIELD_GUTTER + lines.len() as f32 * lh + 1.0, cw - 2.0, lh - 2.0);
            outline.cmds.extend(r.cmds);
        }
        col += 1;
        i += 1;
        stops.push((i, col as f32 * cw));
        width = width.max(col as f32 * cw);
    }
    lines.push(CaretLine { start, end: i, stops });
    let n = lines.len();
    FieldLook { outline: Arc::new(outline), lines: lines.into(), line_height: lh, width, height: n as f32 * lh }
}

#[cfg(test)]
#[path = "field_tests.rs"]
mod tests;
