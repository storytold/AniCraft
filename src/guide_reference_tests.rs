//! The user guide's generated pages (ADR 0069): the command reference, from the engine's
//! command registry, and the shortcut reference, from the same defaults the shortcut editor
//! shows (registry shortcuts and aliases, tool keys, view keys and the fixed keys), with the
//! macOS spelling beside the Windows and Linux one.
//!
//! They live in the shell because the keys do: tool and view keys and the macOS labels are
//! the shell's. A test compares them with `docs/guide/reference/` and fails when they are
//! stale; `ANICRAFT_BLESS_GUIDE=1 cargo test -p anicraft guide_reference` writes them.

use crate::platform::{Os, shortcut_label_for};

/// The command reference page.
pub const COMMANDS_FILE: &str = "reference/commands.md";
/// The shortcut reference page.
pub const SHORTCUTS_FILE: &str = "reference/shortcuts.md";

/// Markdown-safe text: the characters that would start emphasis, links, code or a table cell.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '|' | '#') {
            out.push('\\');
        }
        out.push(if c == '\n' { ' ' } else { c });
    }
    out
}

/// Text in a code span, with a delimiter longer than any run of backticks inside.
fn code(s: &str) -> String {
    let s = s.replace('\n', " ");
    let mut longest = 0;
    let mut run = 0;
    for c in s.chars() {
        run = if c == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    let fence = "`".repeat(longest + 1);
    let pad = if s.starts_with('`') || s.ends_with('`') { " " } else { "" };
    format!("{fence}{pad}{s}{pad}{fence}")
}

/// A registry chord on macOS, with the symbols.
fn mac(k: &str) -> String {
    shortcut_label_for(Os::Mac, k, true)
}

/// `Ctrl+Shift+S` and, where macOS writes it differently, `⇧⌘S` beside it.
fn both(k: &str) -> String {
    let m = mac(k);
    if m == k { format!("**{}**", esc(k)) } else { format!("**{}** (macOS **{}**)", esc(k), esc(&m)) }
}

/// The groups in the order the shortcut editor shows them, then any others in name order.
fn ordered(mut groups: Vec<&str>) -> Vec<&str> {
    let rank = |g: &str| crate::dialogs::GROUP_ORDER.iter().position(|x| *x == g).unwrap_or(usize::MAX);
    groups.sort_by(|a, b| rank(a).cmp(&rank(b)).then(a.cmp(b)));
    groups.dedup();
    groups
}

/// The command reference: every command, by top menu, with its id, menu path, default key,
/// whether it changes the document, and its parameters.
pub fn commands_md() -> String {
    let cmds = anicraft_engine::commands();
    let mut out = String::from("# Command reference\n\n");
    out.push_str(&format!(
        "Everything you can do in AniCraft is a command: the menus, the shortcuts, the command palette (**Ctrl+K**, macOS **⌘K**), \
         `anicraft-cli` and AI assistants over MCP all run the same {} commands, listed here by menu. The id is what scripts and \
         [working headless](../working-headless.md) use; the parameters are what a command takes as JSON. Commands that change the \
         document are one undo step each. Your own shortcuts (**F1**) replace the defaults shown here; see the \
         [shortcut reference](shortcuts.md).\n\n",
        cmds.len()
    ));
    out.push_str("This page is generated from the command registry, so it always matches the version of AniCraft it came with.\n");
    const HEADLESS: &str = "Not in a menu";
    let top = |m: &'static str| if m.is_empty() { HEADLESS } else { m.split('/').next().unwrap_or(HEADLESS) };
    let mut groups = ordered(cmds.iter().map(|c| top(c.menu)).filter(|g| *g != HEADLESS).collect());
    groups.push(HEADLESS);
    for g in groups {
        let in_group: Vec<_> = cmds.iter().filter(|c| top(c.menu) == g).collect();
        if in_group.is_empty() {
            continue;
        }
        out.push_str(&format!("\n## {}\n\n", esc(g)));
        if g == HEADLESS {
            out.push_str("These commands have no menu item: the panels, the stage and the timeline use them, and so can scripts and assistants.\n\n");
        }
        for c in in_group {
            let mut line = format!("- **{}** {}", esc(c.label), code(c.id));
            // The submenu, when there is one (the section is the top menu).
            if c.menu.contains('/') {
                line.push_str(&format!(" · {}", esc(&c.menu.replace('/', " › "))));
            }
            if let Some(k) = c.shortcut {
                line.push_str(&format!(" · {}", both(k)));
            }
            if c.mutates {
                line.push_str(" · changes the document");
            }
            out.push_str(&line);
            out.push('\n');
            let params = c.params.trim();
            if !params.is_empty() {
                out.push_str(&format!("  Parameters: {}\n", code(params)));
            }
        }
    }
    out
}

/// The shortcut reference: every default key, by group, for Windows and Linux and for macOS.
pub fn shortcuts_md() -> String {
    let mut out = String::from("# Shortcut reference\n\n");
    out.push_str(
        "AniCraft's default keyboard shortcuts. On macOS, **Ctrl** is **⌘** (Command) and **Alt** is **⌥** (Option). \
         To change a shortcut, open **Help › Keyboard Shortcuts** (**F1**): see \
         [Preferences, shortcuts, accessibility and languages](../preferences-shortcuts-accessibility.md). \
         The keys in the fixed rows can't be reassigned. Every command, with or without a key, is in the \
         [command reference](commands.md).\n\n\
         This page is generated from the editor's shortcut map, so it always matches the version of AniCraft it came with.\n",
    );
    // (group, action, keys): the bindable actions with default keys, then the fixed keys.
    // A command in a submenu is named with it ("Timeline › Keyframe"), as the menus show it.
    let cmds = anicraft_engine::commands();
    let name = |a: &crate::keymap::Action| match cmds.iter().find(|c| c.id == a.id).and_then(|c| c.menu.split_once('/')) {
        Some((_, sub)) => format!("{} › {}", sub.replace('/', " › "), a.label),
        None => a.label.to_string(),
    };
    let mut rows: Vec<(&str, String, Vec<&str>)> = crate::keymap::actions().into_iter().filter(|a| !a.defaults.is_empty()).map(|a| (a.group, name(&a), a.defaults)).collect();
    for (group, label, keys) in crate::dialogs::fixed_rows() {
        let label = if matches!(label, "Quit" | "Exit") { "Exit (Quit on macOS)".to_string() } else { label.to_string() };
        rows.push((group, label, keys));
    }
    for g in ordered(rows.iter().map(|r| r.0).collect()) {
        out.push_str(&format!("\n## {}\n\n| Action | Windows and Linux | macOS |\n|---|---|---|\n", esc(g)));
        for (_, label, keys) in rows.iter().filter(|r| r.0 == g) {
            let win: Vec<String> = keys.iter().map(|k| esc(k)).collect();
            let mac: Vec<String> = keys.iter().map(|k| esc(&mac(k))).collect();
            out.push_str(&format!("| {} | {} | {} |\n", esc(label), win.join(", "), mac.join(", ")));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn reference_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs").join("guide")
    }

    /// Compares a generated page with the committed one (line endings aside), or writes it
    /// when `ANICRAFT_BLESS_GUIDE` is set.
    fn fresh(file: &str, generated: &str) {
        let path = file.split('/').fold(reference_dir(), |p, part| p.join(part));
        if std::env::var_os("ANICRAFT_BLESS_GUIDE").is_some() {
            std::fs::create_dir_all(path.parent().expect("has a folder")).expect("folder");
            std::fs::write(&path, generated).expect("writes the reference");
            return;
        }
        let committed = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
        assert!(
            committed == generated,
            "docs/guide/{file} is out of date with the commands and shortcuts. Regenerate it with\n  ANICRAFT_BLESS_GUIDE=1 cargo test -p anicraft guide_reference\nand commit the result."
        );
    }

    #[test]
    fn guide_reference_commands_are_current() {
        fresh(COMMANDS_FILE, &commands_md());
    }

    #[test]
    fn guide_reference_shortcuts_are_current() {
        fresh(SHORTCUTS_FILE, &shortcuts_md());
    }

    #[test]
    fn guide_reference_lists_every_command_and_key() {
        let md = commands_md();
        for c in anicraft_engine::commands() {
            assert!(md.contains(&code(c.id)), "{} missing from the command reference", c.id);
        }
        let keys = shortcuts_md();
        assert!(keys.contains("| Timeline › Keyframe | F6 | F6 |"), "{keys}");
        assert!(keys.contains("Ctrl+Shift+S") && keys.contains("⇧⌘S"), "macOS spellings beside the others");
        assert!(keys.contains("| Selection | V | V |"), "tool keys are listed");
    }

    #[test]
    fn reference_text_is_markdown_safe() {
        assert_eq!(esc("a|b *c* [d]"), "a\\|b \\*c\\* \\[d\\]");
        assert_eq!(code("x"), "`x`");
        assert_eq!(code("a`b"), "``a`b``");
        assert_eq!(code("`a"), "`` `a ``");
    }
}
