//! The keyboard shortcut map: defaults plus the user's changes.
//!
//! Every bindable action has default keys: registry commands (their shortcut and any engine
//! alias), the tools and the view keys. The user's changes are stored in preferences as
//! overrides (`action id → its full key list`), so actions the user never touched keep
//! following the defaults as they evolve. A few keys are reserved and cannot be rebound: the
//! ones that open this editor, the palette and preferences, quitting, playing and cancelling.
//! A chord is written in registry notation, modifiers in a fixed order: `Ctrl+Alt+Shift+K`.

use crate::app::{Tool, VIEW_SHORTCUTS};
use anicraft_engine::{SHORTCUT_ALIASES, commands};
use egui::{Key, Modifiers};
use std::collections::BTreeMap;

/// Keys handled before the shortcut map, which the editor will not assign, and what for
/// (English: translated where it is shown).
pub const RESERVED: &[(&str, &str)] = &[
    ("Ctrl+K", tr_noop!("the command palette")),
    ("Ctrl+Shift+P", tr_noop!("the command palette")),
    ("F1", tr_noop!("the shortcut editor")),
    ("Ctrl+Alt+Shift+K", tr_noop!("the shortcut editor")),
    ("Ctrl+U", tr_noop!("Preferences")),
    ("Ctrl+,", tr_noop!("Preferences")),
    ("Ctrl+Q", tr_noop!("quitting")),
    ("Enter", tr_noop!("play / stop")),
    ("Escape", tr_noop!("cancelling")),
    ("Space", tr_noop!("panning with any tool")),
    ("Tab", tr_noop!("moving between fields")),
];

/// Most keys one action may have.
pub const MAX_KEYS: usize = 4;

/// A bindable action.
#[derive(Clone, Debug)]
pub struct Action {
    pub id: &'static str,
    /// English, as the registry has it: translated where it is shown (`i18n::t_cmd`, `i18n::t`).
    pub label: &'static str,
    /// Where it lives (a top menu, "Tools" or "View"); English, translated where it is shown.
    pub group: &'static str,
    pub defaults: Vec<&'static str>,
}

/// The id a tool is bound under.
pub fn tool_id(t: Tool) -> &'static str {
    match t {
        Tool::Select => "tool.selection",
        Tool::Subselect => "tool.subselection",
        Tool::Transform => "tool.free_transform",
        Tool::GradientTransform => "tool.gradient_transform",
        Tool::Width => "tool.width",
        Tool::Pen => "tool.pen",
        Tool::Text => "tool.text",
        Tool::Pencil => "tool.pencil",
        Tool::Brush => "tool.brush",
        Tool::PaintBrush => "tool.paint_brush",
        Tool::Eraser => "tool.eraser",
        Tool::Rect => "tool.rectangle",
        Tool::Oval => "tool.oval",
        Tool::Star => "tool.polystar",
        Tool::Line => "tool.line",
        Tool::PaintBucket => "tool.paint_bucket",
        Tool::InkBottle => "tool.ink_bottle",
        Tool::Eyedropper => "tool.eyedropper",
        Tool::Camera => "tool.camera",
        Tool::Bone => "tool.bone",
        Tool::Bind => "tool.bind",
        Tool::Warp => "tool.warp",
        Tool::Hand => "tool.hand",
        Tool::Zoom => "tool.zoom",
    }
}

pub fn tool_for(id: &str) -> Option<Tool> {
    Tool::ALL.into_iter().find(|t| tool_id(*t) == id)
}

fn view_label(id: &str) -> &'static str {
    match id {
        "view.fit" => tr_noop!("Fit Stage in Window"),
        "view.onion" => tr_noop!("Onion Skin On / Off"),
        "view.select_all_frames" => tr_noop!("Select All Frames"),
        "view.align_to_stage" => tr_noop!("Align to Stage On / Off"),
        "view.next_panel" => tr_noop!("Next Panel"),
        "view.prev_panel" => tr_noop!("Previous Panel"),
        "view.actions" => tr_noop!("Actions Panel"),
        "view.find_next" => tr_noop!("Find Next"),
        "view.find_prev" => tr_noop!("Find Previous"),
        "view.movie_explorer" => tr_noop!("Movie Explorer"),
        "view.brush_smaller" => tr_noop!("Brush Size Down"),
        "view.brush_larger" => tr_noop!("Brush Size Up"),
        _ => tr_noop!("View action"),
    }
}

/// Every action that can carry a shortcut: the menu commands, any command with a default key,
/// the tools and the view keys.
pub fn actions() -> Vec<Action> {
    let mut v: Vec<Action> = commands()
        .into_iter()
        .filter(|c| !c.menu.is_empty() || c.shortcut.is_some())
        .map(|c| Action {
            id: c.id,
            label: c.label,
            group: if c.menu.is_empty() { tr_noop!("Other") } else { c.menu.split('/').next().unwrap_or(tr_noop!("Other")) },
            defaults: c.shortcut.into_iter().chain(SHORTCUT_ALIASES.iter().filter(|(_, id)| *id == c.id).map(|(k, _)| *k)).collect(),
        })
        .collect();
    v.extend(Tool::ALL.into_iter().map(|t| Action { id: tool_id(t), label: t.label(), group: tr_noop!("Tools"), defaults: t.shortcut().into_iter().collect() }));
    let mut views: Vec<&str> = Vec::new();
    for (_, id) in VIEW_SHORTCUTS {
        if !views.contains(id) {
            views.push(id);
        }
    }
    v.extend(views.into_iter().map(|id| Action {
        id,
        label: view_label(id),
        group: tr_noop!("View"),
        defaults: VIEW_SHORTCUTS.iter().filter(|(_, i)| *i == id).map(|(k, _)| *k).collect(),
    }));
    v
}

/// The registry spelling of a key.
fn key_name(k: Key) -> &'static str {
    match k {
        Key::Comma => ",",
        Key::Period => ".",
        Key::Minus => "-",
        Key::Equals => "=",
        Key::OpenBracket => "[",
        Key::CloseBracket => "]",
        Key::Semicolon => ";",
        Key::Quote => "'",
        other => other.name(),
    }
}

/// Parses a chord in registry notation (`Ctrl+Shift+Z`, `Shift+,`, `F5`).
pub fn parse(s: &str) -> Option<(Modifiers, Key)> {
    let mut m = Modifiers::NONE;
    let mut key = None;
    for part in s.split('+') {
        match part {
            "Ctrl" => m |= Modifiers::COMMAND,
            "Shift" => m |= Modifiers::SHIFT,
            "Alt" => m |= Modifiers::ALT,
            "" => return None,
            other => {
                if key.is_some() {
                    return None;
                }
                key = Some(Key::from_name(other)?);
            }
        }
    }
    key.map(|k| (m, k))
}

/// The canonical spelling of a key press.
pub fn chord(m: Modifiers, k: Key) -> String {
    let mut s = String::new();
    for (on, name) in [(m.command || m.ctrl, "Ctrl+"), (m.alt, "Alt+"), (m.shift, "Shift+")] {
        if on {
            s.push_str(name);
        }
    }
    s.push_str(key_name(k));
    s
}

/// The canonical spelling of a chord, or `None` when it is not a chord.
pub fn normalize(s: &str) -> Option<String> {
    parse(s).map(|(m, k)| chord(m, k))
}

/// What a reserved chord is kept for.
pub fn reserved(chord: &str) -> Option<&'static str> {
    let c = normalize(chord)?;
    RESERVED.iter().find(|(k, _)| normalize(k).as_deref() == Some(c.as_str())).map(|(_, why)| *why)
}

/// Defaults with the user's overrides applied.
#[derive(Clone, Debug)]
pub struct Keymap {
    pub actions: Vec<Action>,
    keys: Vec<Vec<String>>,
}

impl Keymap {
    pub fn new(overrides: &BTreeMap<String, Vec<String>>) -> Keymap {
        let actions = actions();
        let keys = actions
            .iter()
            .map(|a| match overrides.get(a.id) {
                Some(user) => user.iter().filter_map(|k| normalize(k)).collect(),
                None => a.defaults.iter().filter_map(|k| normalize(k)).collect(),
            })
            .collect();
        Keymap { actions, keys }
    }

    fn index(&self, id: &str) -> Option<usize> {
        self.actions.iter().position(|a| a.id == id)
    }

    pub fn keys(&self, id: &str) -> &[String] {
        self.index(id).and_then(|i| self.keys.get(i)).map_or(&[], Vec::as_slice)
    }

    /// The first key, for menus and tooltips.
    pub fn primary(&self, id: &str) -> Option<&str> {
        self.keys(id).first().map(String::as_str)
    }

    pub fn is_default(&self, id: &str) -> bool {
        self.index(id).is_none_or(|i| {
            let defaults: Vec<String> = self.actions.get(i).map(|a| a.defaults.iter().filter_map(|k| normalize(k)).collect()).unwrap_or_default();
            self.keys.get(i) == Some(&defaults)
        })
    }

    /// The action a chord is bound to.
    pub fn owner(&self, chord: &str) -> Option<&Action> {
        let c = normalize(chord)?;
        self.keys.iter().position(|ks| ks.contains(&c)).and_then(|i| self.actions.get(i))
    }

    /// `(modifiers, key, action)` for every binding, most specific chord first: egui matches a
    /// chord with extra Shift/Alt held, so Ctrl+Shift+Z must be tried before Ctrl+Z.
    pub fn table(&self) -> Vec<(Modifiers, Key, &'static str)> {
        let mut t: Vec<(Modifiers, Key, &'static str)> =
            self.actions.iter().zip(&self.keys).flat_map(|(a, ks)| ks.iter().filter_map(|k| parse(k)).map(|(m, k)| (m, k, a.id))).collect();
        t.sort_by_key(|(m, _, _)| std::cmp::Reverse(u8::from(m.command) + u8::from(m.shift) + u8::from(m.alt)));
        t
    }
}

/// Why a key cannot be assigned to an action as asked.
#[derive(Clone, Debug, PartialEq)]
pub enum Refusal {
    Reserved(&'static str),
    /// Bound to another action; assign again with `steal` to move it.
    Taken { id: &'static str, label: &'static str },
    TooMany,
}

/// Overrides after giving `id` the key `chord` (replacing its key at `replace`, or adding one).
/// With `steal`, a chord bound elsewhere moves here; otherwise that is refused.
pub fn assign(overrides: &BTreeMap<String, Vec<String>>, id: &str, replace: Option<usize>, chord: &str, steal: bool) -> Result<BTreeMap<String, Vec<String>>, Refusal> {
    let map = Keymap::new(overrides);
    let c = normalize(chord).ok_or(Refusal::Reserved(tr_noop!("nothing (not a key)")))?;
    if let Some(why) = reserved(&c) {
        return Err(Refusal::Reserved(why));
    }
    let mut out = overrides.clone();
    if let Some(other) = map.owner(&c)
        && other.id != id
    {
        if !steal {
            return Err(Refusal::Taken { id: other.id, label: other.label });
        }
        let rest: Vec<String> = map.keys(other.id).iter().filter(|k| **k != c).cloned().collect();
        set(&mut out, &map, other.id, rest);
    }
    let mut mine: Vec<String> = map.keys(id).to_vec();
    match replace.filter(|&i| i < mine.len()) {
        Some(i) => {
            if let Some(slot) = mine.get_mut(i) {
                *slot = c.clone();
            }
        }
        None if mine.len() >= MAX_KEYS => return Err(Refusal::TooMany),
        None => mine.push(c.clone()),
    }
    // A chord the action already had elsewhere in its list collapses into one.
    let mut seen = Vec::new();
    mine.retain(|k| !seen.contains(k) && {
        seen.push(k.clone());
        true
    });
    set(&mut out, &map, id, mine);
    Ok(out)
}

/// Overrides after removing `id`'s key at `index`.
pub fn remove(overrides: &BTreeMap<String, Vec<String>>, id: &str, index: usize) -> BTreeMap<String, Vec<String>> {
    let map = Keymap::new(overrides);
    let mut mine = map.keys(id).to_vec();
    if index < mine.len() {
        mine.remove(index);
    }
    let mut out = overrides.clone();
    set(&mut out, &map, id, mine);
    out
}

/// Overrides with `id` back on its defaults. Defaults that another action has taken over stay
/// with that action, so a reset never creates a clash.
pub fn reset(overrides: &BTreeMap<String, Vec<String>>, id: &str) -> BTreeMap<String, Vec<String>> {
    let mut out = overrides.clone();
    out.remove(id);
    let map = Keymap::new(&out);
    if let Some(a) = map.actions.iter().find(|a| a.id == id) {
        let free: Vec<String> = a.defaults.iter().filter_map(|k| normalize(k)).filter(|k| map.actions.iter().all(|o| o.id == id || !map.keys(o.id).contains(k))).collect();
        set(&mut out, &map, id, free);
    }
    out
}

/// Records `keys` for `id`, as an override only when they differ from the defaults.
fn set(out: &mut BTreeMap<String, Vec<String>>, map: &Keymap, id: &str, keys: Vec<String>) {
    let defaults: Vec<String> = map.actions.iter().find(|a| a.id == id).map(|a| a.defaults.iter().filter_map(|k| normalize(k)).collect()).unwrap_or_default();
    if keys == defaults {
        out.remove(id);
    } else {
        out.insert(id.to_string(), keys);
    }
}

/// Overrides as loaded from a file: unknown actions and malformed or reserved keys dropped, at
/// most [`MAX_KEYS`] per action, and no chord bound twice (the first binding wins).
pub fn sanitize(overrides: BTreeMap<String, Vec<String>>) -> BTreeMap<String, Vec<String>> {
    let known = actions();
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (id, keys) in overrides.into_iter().take(1_000) {
        if !known.iter().any(|a| a.id == id) {
            continue;
        }
        let mut clean: Vec<String> = Vec::new();
        for k in keys.iter().take(16).filter(|k| k.len() <= 64) {
            if let Some(c) = normalize(k)
                && reserved(&c).is_none()
                && !clean.contains(&c)
                && clean.len() < MAX_KEYS
            {
                clean.push(c);
            }
        }
        out.insert(id, clean);
    }
    // Resolve clashes between overrides and the defaults of untouched actions.
    let map = Keymap::new(&out);
    let mut seen: Vec<String> = Vec::new();
    let mut fixes: Vec<(&'static str, Vec<String>)> = Vec::new();
    for a in &map.actions {
        let ks = map.keys(a.id);
        let keep: Vec<String> = ks.iter().filter(|k| !seen.contains(k)).cloned().collect();
        seen.extend(keep.iter().cloned());
        if keep.len() != ks.len() {
            fixes.push((a.id, keep));
        }
    }
    for (id, keep) in fixes {
        out.insert(id.to_string(), keep);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none() -> BTreeMap<String, Vec<String>> {
        BTreeMap::new()
    }

    #[test]
    fn defaults_cover_commands_tools_and_view_keys_without_clashes() {
        let m = Keymap::new(&none());
        assert_eq!(m.keys("edit.undo"), ["Ctrl+Z"]);
        assert_eq!(m.keys("edit.delete"), ["Delete", "Backspace"], "aliases are second keys");
        assert_eq!(m.primary("tool.pen"), Some("P"));
        assert_eq!(m.primary("tool.brush"), Some("B"));
        assert_eq!(m.primary("tool.eraser"), Some("E"));
        assert_eq!(m.primary("tool.eyedropper"), Some("I"));
        assert_eq!(m.keys("view.brush_smaller"), ["["]);
        assert_eq!(m.keys("view.brush_larger"), ["]"]);
        assert_eq!(m.primary("view.onion"), Some("Alt+Shift+O"));
        assert_eq!(m.owner("ctrl+z").map(|a| a.id), None, "chords are case-sensitive registry names");
        assert_eq!(m.owner("Ctrl+Shift+Z").map(|a| a.id), Some("modify.remove_transform"));
        let mut all: Vec<String> = m.actions.iter().flat_map(|a| m.keys(a.id).to_vec()).collect();
        let n = all.len();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), n, "a default chord is bound twice");
        assert!(all.iter().all(|k| reserved(k).is_none()), "a default uses a reserved key");
        assert!(m.actions.iter().all(|a| m.is_default(a.id)));
    }

    #[test]
    fn chords_round_trip_in_one_spelling() {
        for s in ["Ctrl+Shift+Z", "Shift+,", "Ctrl+Up", "F5", "Ctrl+Alt+Shift+K", "Ctrl+-", "Alt+Shift+O", "Backspace", "[", "]"] {
            assert_eq!(normalize(s).as_deref(), Some(s), "{s}");
        }
        assert_eq!(normalize("Shift+Ctrl+Z").as_deref(), Some("Ctrl+Shift+Z"));
        assert_eq!(chord(Modifiers::COMMAND | Modifiers::ALT, Key::Num8), "Ctrl+Alt+8");
        for bad in ["", "Ctrl+", "Ctrl+A+B", "Hyper+Q", "Ctrl++"] {
            assert_eq!(normalize(bad), None, "{bad}");
        }
    }

    #[test]
    fn assigning_moves_keys_only_when_asked_and_stores_only_changes() {
        // Rebind Undo to Ctrl+D (Duplicate's key): refused, then moved on request.
        let r = assign(&none(), "edit.undo", Some(0), "Ctrl+D", false);
        assert_eq!(r, Err(Refusal::Taken { id: "edit.duplicate", label: "Duplicate" }));
        let o = assign(&none(), "edit.undo", Some(0), "Ctrl+D", true).unwrap();
        let m = Keymap::new(&o);
        assert_eq!(m.keys("edit.undo"), ["Ctrl+D"]);
        assert!(m.keys("edit.duplicate").is_empty());
        assert_eq!(o.len(), 2, "both changes are stored, nothing else");
        // Reserved keys stay reserved.
        assert_eq!(assign(&none(), "edit.undo", None, "Ctrl+K", true), Err(Refusal::Reserved("the command palette")));
        // Adding a second key, then putting it back the way it was, stores nothing.
        let o = assign(&none(), "tool.pen", None, "Shift+P", false).unwrap();
        assert_eq!(Keymap::new(&o).keys("tool.pen"), ["P", "Shift+P"]);
        assert!(remove(&o, "tool.pen", 1).is_empty());
        // Removing every key is a change too (the action is unbound).
        let o = remove(&none(), "tool.pen", 0);
        assert!(Keymap::new(&o).keys("tool.pen").is_empty());
        assert!(!Keymap::new(&o).is_default("tool.pen"));
    }

    #[test]
    fn reset_never_creates_a_clash() {
        let o = assign(&none(), "edit.undo", Some(0), "Ctrl+D", true).unwrap();
        // Duplicate's default (Ctrl+D) now belongs to Undo, so resetting Duplicate skips it...
        let o = reset(&o, "edit.duplicate");
        assert!(Keymap::new(&o).keys("edit.duplicate").is_empty());
        // ...until Undo is reset too and Duplicate is reset again.
        let o = reset(&reset(&o, "edit.undo"), "edit.duplicate");
        assert!(o.is_empty());
    }

    #[test]
    fn hostile_preference_files_are_cleaned() {
        let mut o = BTreeMap::new();
        o.insert("no.such.command".to_string(), vec!["F9".to_string()]);
        o.insert("tool.pen".to_string(), vec!["Ctrl+K".into(), "bogus".into(), "F9".into(), "F9".into(), "x".repeat(500)]);
        // Undo takes Ctrl+D without Duplicate giving it up: the clash is resolved.
        o.insert("edit.undo".to_string(), vec!["Ctrl+D".into()]);
        let o = sanitize(o);
        let m = Keymap::new(&o);
        assert!(!o.contains_key("no.such.command"));
        assert_eq!(m.keys("tool.pen"), ["F9"]);
        let owners: Vec<&str> = m.actions.iter().filter(|a| m.keys(a.id).iter().any(|k| k == "Ctrl+D")).map(|a| a.id).collect();
        assert_eq!(owners.len(), 1, "{owners:?}");
    }
}
