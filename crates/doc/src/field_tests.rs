use super::*;
use crate::{Rgba, TextAlign};

fn block(text: &str, f: TextField) -> TextBlock {
    TextBlock { text: text.into(), font: "Mono".into(), size: 10.0, bold: false, italic: false, color: Rgba::BLACK, align: TextAlign::Left, letter_spacing: 0.0, line_spacing: 0.0, width: None, outline: Arc::new(Path::new()), box_width: 54.0, box_height: 24.0, field: Some(f) }
}

/// Runs edits on a field as the player does: each change of text lays it out again (10 px per
/// character, 10 px lines) and keeps the caret in view.
struct Rig {
    block: TextBlock,
    field: TextField,
    st: FieldState,
}

impl Rig {
    fn new(text: &str, kind: TextKind, lines: LineType) -> Rig {
        let field = TextField { lines, ..TextField::new(kind) };
        let mut r = Rig { block: block(text, field.clone()), field, st: FieldState::default() };
        r.relayout();
        r
    }
    fn relayout(&mut self) {
        let shown = self.field.shown(self.st.value(&self.block));
        self.st.look = Some(Arc::new(mono_look(&shown, 10.0, 10.0, self.field.wrap_width(self.block.box_width))));
        scroll_to_caret(&self.block, &self.field, &mut self.st);
    }
    fn apply(&mut self, ch: Changed) {
        if ch.text {
            self.relayout();
        } else {
            scroll_to_caret(&self.block, &self.field, &mut self.st);
        }
    }
    fn ed(&mut self) -> (Editor<'_>, Option<Arc<FieldLook>>) {
        let look = self.st.look.clone();
        (Editor { block: &self.block, field: &self.field, st: &mut self.st }, look)
    }
    fn key(&mut self, e: Edit, shift: bool) -> &mut Self {
        let (mut ed, look) = self.ed();
        let ch = ed.key(e, shift, look.as_deref());
        self.apply(ch);
        self
    }
    fn typed(&mut self, s: &str) -> &mut Self {
        for c in s.chars() {
            let (mut ed, _) = self.ed();
            let ch = ed.insert(&c.to_string());
            self.apply(ch);
        }
        self
    }
    fn click(&mut self, x: f32, y: f32, extend: bool) -> &mut Self {
        let (mut ed, look) = self.ed();
        let ch = ed.point(Vec2::new(x, y), extend, look.as_deref());
        self.apply(ch);
        self
    }
    fn text(&self) -> &str {
        self.st.value(&self.block)
    }
}

#[test]
fn typing_moves_the_caret_and_keys_edit_around_it() {
    let mut r = Rig::new("", TextKind::Input, LineType::Single);
    r.typed("hello");
    assert_eq!((r.text(), r.st.caret, r.st.anchor), ("hello", 5, 5));
    r.key(Edit::Left, false).key(Edit::Left, false).typed("X");
    assert_eq!((r.text(), r.st.caret), ("helXlo", 4));
    r.key(Edit::Backspace, false);
    assert_eq!((r.text(), r.st.caret), ("hello", 3));
    r.key(Edit::Delete, false);
    assert_eq!((r.text(), r.st.caret), ("helo", 3));
    r.key(Edit::Home, false);
    assert_eq!(r.st.caret, 0);
    r.key(Edit::Backspace, false);
    assert_eq!(r.text(), "helo", "nothing before the caret");
    r.key(Edit::End, false).key(Edit::Delete, false);
    assert_eq!((r.text(), r.st.caret), ("helo", 4), "nothing after it");
    // Enter types nothing in a one-line field; control characters never go in.
    r.key(Edit::Enter, false).typed("\u{7}\t");
    assert_eq!(r.text(), "helo");
}

#[test]
fn shift_selects_and_typing_replaces_the_selection() {
    let mut r = Rig::new("abcdef", TextKind::Input, LineType::Single);
    r.click(20.0, 5.0, false);
    assert_eq!(r.st.caret, 2, "a click puts the caret at the nearest boundary");
    r.key(Edit::Right, true).key(Edit::Right, true);
    assert_eq!(r.st.selection(), (2, 4));
    assert_eq!(r.st.selected(&r.block), "cd");
    // An arrow without Shift collapses the selection to its side.
    r.key(Edit::Left, false);
    assert_eq!((r.st.caret, r.st.anchor), (2, 2));
    r.key(Edit::End, true);
    assert_eq!(r.st.selection(), (2, 6));
    r.typed("Z");
    assert_eq!((r.text(), r.st.caret), ("abZ", 3));
    // Dragging extends from where the press was; Backspace deletes the selection.
    r.click(0.0, 5.0, false).click(19.0, 5.0, true);
    assert_eq!(r.st.selection(), (0, 2));
    r.key(Edit::Backspace, false);
    assert_eq!((r.text(), r.st.caret), ("Z", 0));
    r.key(Edit::SelectAll, false);
    assert_eq!(r.st.selection(), (0, 1));
}

#[test]
fn multiline_fields_take_enter_and_move_between_lines() {
    let mut r = Rig::new("", TextKind::Input, LineType::MultilineNoWrap);
    r.typed("abc").key(Edit::Enter, false).typed("de");
    assert_eq!(r.text(), "abc\nde");
    assert_eq!(r.st.caret, 6);
    r.key(Edit::Up, false);
    assert_eq!(r.st.caret, 2, "up keeps the column");
    r.key(Edit::End, false);
    assert_eq!(r.st.caret, 3);
    r.key(Edit::Down, false);
    assert_eq!(r.st.caret, 6, "down to the shorter line's end");
    r.key(Edit::Down, false);
    assert_eq!(r.st.caret, 6, "down from the last line goes to the end");
    r.key(Edit::Up, false).key(Edit::Up, false);
    assert_eq!(r.st.caret, 0, "up from the first line goes to the start");
    // Clicking on the second line.
    r.click(17.0, 14.0, false);
    assert_eq!(r.st.caret, 6, "x 17 on line 2 is nearest the boundary after d");
}

#[test]
fn max_chars_and_passwords() {
    let mut r = Rig::new("", TextKind::Input, LineType::Password);
    r.field.max_chars = 4;
    r.typed("secret");
    assert_eq!(r.text(), "secr", "at most max_chars");
    let shown = r.field.shown(r.text());
    assert_eq!(shown, "****");
    // A paste is cut to the room left, and single-line fields drop its line breaks.
    let mut r = Rig::new("ab", TextKind::Input, LineType::Single);
    r.field.max_chars = 6;
    r.key(Edit::End, false);
    let (mut ed, _) = r.ed();
    let ch = ed.insert("c\nd\nefgh");
    r.apply(ch);
    assert_eq!(r.text(), "abcdef");
}

#[test]
fn dynamic_fields_select_but_never_change() {
    let mut r = Rig::new("read only", TextKind::Dynamic, LineType::Single);
    r.typed("x").key(Edit::Backspace, false).key(Edit::Delete, false);
    assert_eq!(r.text(), "read only");
    assert!(r.st.text.is_none(), "untouched");
    r.key(Edit::SelectAll, false);
    assert_eq!(r.st.selected(&r.block), "read only");
}

#[test]
fn one_line_fields_scroll_to_keep_the_caret_in_view() {
    // The box shows 50 px of text (54 less the gutters): five characters.
    let mut r = Rig::new("", TextKind::Input, LineType::Single);
    r.typed("abcdefgh");
    assert_eq!(r.st.scroll.x, 30.0, "the caret at 80 px shows at the right edge");
    r.key(Edit::Home, false);
    assert_eq!(r.st.scroll.x, 0.0);
    // Multiline: lines wrap and it scrolls down instead (the box shows two 10 px lines).
    let mut r = Rig::new("", TextKind::Input, LineType::Multiline);
    r.typed("aaaaabbbbbccccc");
    assert_eq!(r.st.look.as_ref().map(|l| l.lines.len()), Some(3));
    assert_eq!((r.st.scroll.x, r.st.scroll.y), (0.0, 10.0));
}

#[test]
fn script_text_is_capped_and_takes_actionscript_line_ends() {
    let f = TextField::new(TextKind::Dynamic);
    assert_eq!(fit(&f, "a\rb\r\nc"), "a\nb\nc");
    assert_eq!(chars(&fit(&f, &"x".repeat(MAX_FIELD_CHARS + 10))), MAX_FIELD_CHARS as u32);
    assert_eq!(slice("héllo", 1, 3), "él");
    assert_eq!(slice("héllo", 4, 99), "o");
    assert_eq!(byte_at("héllo", 2), 3);
}
