//! Portable mode and AppImage desktop integration in the shell (ADR 0094): the one-time,
//! non-modal offer to add AniCraft to the applications menu, and Preferences › About (where this
//! copy keeps its things, and the menu entry's Add / Remove button).
//!
//! Where things are kept is `platform::placement`; the menu entry is written by the engine's
//! `desktop.*` commands, so the CLI and MCP can do the same.

use crate::app::App;
use crate::theme::{T, Type, text};
use serde_json::json;
use std::path::PathBuf;

/// This copy as an AppImage: the file it runs from and where its menu entry goes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Desktop {
    pub appimage: PathBuf,
    pub data_home: PathBuf,
    /// The menu entry there now (read at start and after each change).
    pub entry: Entry,
    /// The card offering to add AniCraft to the menu is showing.
    pub offer: bool,
}

/// The menu entry under the data folder.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Entry {
    #[default]
    None,
    /// AniCraft's entry, starting this copy.
    ThisCopy,
    /// AniCraft's entry, starting another AppImage (`true`: that file is still there).
    Other(PathBuf, bool),
    /// An entry of AniCraft's name that AniCraft didn't write: left alone.
    Foreign,
}

impl Desktop {
    fn params(&self) -> serde_json::Value {
        json!({"appimage": self.appimage.display().to_string(), "data_home": self.data_home.display().to_string()})
    }
}

/// Reads the menu entry under `d.data_home`.
fn read_entry(app: &mut App, d: &Desktop) -> Entry {
    let Ok(v) = app.s.execute("desktop.status", &d.params()) else { return Entry::None };
    if v["foreign"].as_bool() == Some(true) {
        return Entry::Foreign;
    }
    match (v["added"].as_bool(), v["this_copy"].as_bool(), v["appimage"].as_str()) {
        (Some(true), Some(true), _) => Entry::ThisCopy,
        (Some(true), _, Some(other)) => Entry::Other(PathBuf::from(other), v["target_exists"].as_bool() == Some(true)),
        _ => Entry::None,
    }
}

/// At start: on an AppImage, see whether AniCraft is in the menu. An entry left pointing at an
/// AppImage that is gone (moved, or replaced by an update) follows this copy without asking: the
/// user said yes once. Otherwise, the first time only, offer to add it, unless this copy is
/// portable (portable copies leave no trace on the computer).
pub fn at_start(app: &mut App) {
    if app.shot.is_some() {
        return;
    }
    let (Some(appimage), Some(data_home)) = (crate::platform::appimage(), crate::platform::data_home()) else { return };
    start_with(app, Desktop { appimage, data_home, ..Default::default() }, crate::platform::placement().portable.is_some());
}

/// [`at_start`] for a given AppImage and data folder (tests and screenshots).
pub fn start_with(app: &mut App, mut d: Desktop, portable: bool) {
    d.entry = read_entry(app, &d);
    if matches!(d.entry, Entry::Other(_, false)) && app.run("desktop.integrate", d.params()).is_some() {
        d.entry = Entry::ThisCopy;
        app.status_ok(tr!("The applications menu now starts this copy of AniCraft"));
    }
    d.offer = d.entry == Entry::None && !app.prefs.desktop_offered && !portable;
    app.desktop = Some(d);
}

/// Adds AniCraft to the menu (or points the entry at this copy).
fn add(app: &mut App) {
    let Some(d) = app.desktop.clone() else { return };
    if app.run("desktop.integrate", d.params()).is_some() {
        app.status_ok(tr!("AniCraft is in your applications menu, and opens .anicraft, .fla and .xfl files. Preferences › About removes it."));
    }
    let entry = read_entry(app, &d);
    if let Some(d) = app.desktop.as_mut() {
        d.entry = entry;
        d.offer = false;
    }
}

/// Removes AniCraft's menu entry, icons and file types.
fn remove(app: &mut App) {
    let Some(d) = app.desktop.clone() else { return };
    if app.run("desktop.remove", json!({"data_home": d.data_home.display().to_string()})).is_some() {
        app.status_ok(tr!("Removed AniCraft from the applications menu. Add it again here any time."));
    }
    let entry = read_entry(app, &d);
    if let Some(d) = app.desktop.as_mut() {
        d.entry = entry;
    }
}

/// The answer to the offer is kept, so it is asked once.
fn answered(app: &mut App, ctx: &egui::Context) {
    let mut p = app.prefs.clone();
    p.desktop_offered = true;
    app.set_prefs(ctx, p);
    if let Some(d) = app.desktop.as_mut() {
        d.offer = false;
    }
}

/// The offer: a small card at the bottom right, over the editor, that nothing waits on.
pub fn offer(app: &mut App, ctx: &egui::Context) {
    if !app.desktop.as_ref().is_some_and(|d| d.offer) {
        return;
    }
    let mut act: Option<bool> = None;
    egui::Area::new(egui::Id::new("desktop_offer")).order(egui::Order::Foreground).anchor(egui::Align2::RIGHT_BOTTOM, [-16.0, -40.0]).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).fill(T.panel_alt).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
            ui.set_width(340.0);
            ui.label(text(Type::Label, tr!("Add AniCraft to your applications menu?")).strong());
            ui.add_space(4.0);
            ui.add(egui::Label::new(text(Type::Caption, tr!("Start it from the menu like any app, and open .anicraft, .fla and .xfl files by double-clicking them. Only your own data folder changes; Preferences › About removes it again."))).wrap());
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let add = egui::Button::new(text(Type::Body, tr!("Add to Menu")).color(T.on_fill).strong()).fill(T.accent).min_size(egui::vec2(110.0, crate::theme::TARGET));
                if ui.add(add).clicked() {
                    act = Some(true);
                }
                if ui.add(egui::Button::new(text(Type::Body, tr!("Not Now"))).min_size(egui::vec2(90.0, crate::theme::TARGET))).clicked() {
                    act = Some(false);
                }
            });
        });
    });
    match act {
        Some(true) => {
            add(app);
            answered(app, ctx);
        }
        Some(false) => {
            answered(app, ctx);
            app.status_ok(tr!("You can add AniCraft to the applications menu any time in Preferences › About."));
        }
        None => {}
    }
}

/// What the About section asks for after drawing (the preferences window runs it).
pub enum Act {
    Add,
    Remove,
    Open(PathBuf),
}

/// Preferences › About: this copy's version and mode, every folder it keeps things in, and on
/// an AppImage the applications menu entry.
pub fn about(app: &App, ui: &mut egui::Ui, label_w: f32) -> Option<Act> {
    use crate::theme::{form_note, form_row};
    let mut act = None;
    let place = crate::platform::placement();
    form_row(ui, label_w, tr!("Version"), |ui| {
        ui.label(text(Type::Data, env!("CARGO_PKG_VERSION")));
        ui.label(text(Type::Caption, crate::platform::OS.name()));
    });
    form_row(ui, label_w, tr!("This copy"), |ui| {
        let mode = if crate::platform::WEB {
            tr!("In the browser: settings are kept in this site's storage")
        } else if place.portable.is_some() {
            tr!("Portable: everything is kept in the data folder beside AniCraft")
        } else {
            tr!("Installed: settings are kept in your user folder")
        };
        ui.label(text(Type::Body, mode));
    });
    if let Some((dir, why)) = &place.unwritable {
        form_note(ui, label_w, text(Type::Caption, tr!("This copy is portable, but {path} can't be written ({error}), so your user folder is used instead.", path = dir.display(), error = why)).color(T.error));
    }
    for (name, dir) in crate::platform::places() {
        let Some(dir) = dir else { continue };
        form_row(ui, label_w, crate::i18n::t(name), |ui| {
            let there = dir.exists();
            if ui.add_enabled(there, egui::Button::new(text(Type::Body, tr!("Open")))).on_disabled_hover_text(tr!("Not made yet")).clicked() {
                act = Some(Act::Open(dir.clone()));
            }
            ui.add(egui::Label::new(text(Type::Path, dir.display().to_string()).color(if there { T.text } else { T.text_faint })).wrap());
        });
    }
    if let Some(d) = &app.desktop {
        ui.add_space(4.0);
        form_row(ui, label_w, tr!("Applications menu"), |ui| {
            ui.vertical(|ui| {
                let (state, button) = match &d.entry {
                    Entry::None => (tr!("Not added: this AppImage is not in your applications menu").to_string(), Some((tr!("Add to Menu"), Act::Add))),
                    Entry::ThisCopy => (tr!("Added: the menu starts this AppImage and it opens your .anicraft, .fla and .xfl files").to_string(), Some((tr!("Remove"), Act::Remove))),
                    Entry::Other(other, _) => (tr!("The menu starts another AppImage: {path}", path = other.display()), Some((tr!("Use This Copy"), Act::Add))),
                    Entry::Foreign => (tr!("A menu entry of AniCraft's name that AniCraft didn't add is there; it is left as it is").to_string(), None),
                };
                ui.add(egui::Label::new(text(Type::Body, state)).wrap());
                ui.horizontal(|ui| {
                    if let Some((label, a)) = button
                        && ui.button(text(Type::Body, label)).clicked()
                    {
                        act = Some(a);
                    }
                    if matches!(d.entry, Entry::Other(..)) && ui.button(text(Type::Body, tr!("Remove"))).clicked() {
                        act = Some(Act::Remove);
                    }
                });
                ui.add(egui::Label::new(text(Type::Path, d.data_home.display().to_string()).color(T.text_faint)).wrap());
            });
        });
    }
    act
}

/// Runs what [`about`] asked for.
pub fn apply(app: &mut App, act: Act) {
    match act {
        Act::Add => add(app),
        Act::Remove => remove(app),
        Act::Open(dir) => {
            if let Err(e) = crate::app::open_in_system(&dir) {
                app.status_err(e);
            }
        }
    }
}

/// A made-up AppImage in the temporary folder, for screenshots of the offer and of About.
pub fn sample(tag: &str) -> Option<Desktop> {
    let root = std::env::temp_dir().join(format!("anicraft-shot-desktop-{tag}"));
    let apps = root.join("My Apps"); // i18n-exempt: a sample folder name
    std::fs::create_dir_all(&apps).ok()?;
    let appimage = apps.join(format!("AniCraft-{}-x86_64.AppImage", env!("CARGO_PKG_VERSION")));
    std::fs::write(&appimage, b"").ok()?;
    Some(Desktop { appimage, data_home: root.join("share"), ..Default::default() })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A screenshot-mode app: it never saves preferences.
    fn headless() -> App {
        App::with_context(&egui::Context::default(), anicraft_engine::Session::default(), Some(PathBuf::from("unused.png")))
    }

    #[test]
    fn the_offer_is_made_once_and_adding_writes_the_entry() {
        let mut app = headless();
        let d = sample("offer-test").unwrap();
        let _ = std::fs::remove_dir_all(&d.data_home);
        start_with(&mut app, d.clone(), false);
        assert!(app.desktop.as_ref().is_some_and(|d| d.offer), "a new AppImage is offered");
        add(&mut app);
        assert_eq!(app.desktop.as_ref().map(|d| d.entry.clone()), Some(Entry::ThisCopy));
        assert!(d.data_home.join("applications/ai.storyteller.anicraft.desktop").is_file());
        // Already added: no offer, even if the question was never answered.
        start_with(&mut app, d.clone(), false);
        assert!(!app.desktop.as_ref().is_some_and(|d| d.offer));
        remove(&mut app);
        assert_eq!(app.desktop.as_ref().map(|d| d.entry.clone()), Some(Entry::None));
        // Answered once: never asked again. Portable copies are never asked.
        app.prefs.desktop_offered = true;
        start_with(&mut app, d.clone(), false);
        assert!(!app.desktop.as_ref().is_some_and(|d| d.offer));
        app.prefs.desktop_offered = false;
        start_with(&mut app, d.clone(), true);
        assert!(!app.desktop.as_ref().is_some_and(|d| d.offer));
        let _ = std::fs::remove_dir_all(&d.data_home);
    }

    #[test]
    fn an_entry_left_pointing_at_a_gone_appimage_follows_this_copy() {
        let mut app = headless();
        let d = sample("moved-test").unwrap();
        let _ = std::fs::remove_dir_all(&d.data_home);
        let old = d.appimage.with_file_name("AniCraft-old.AppImage");
        std::fs::write(&old, b"").unwrap();
        app.s.execute("desktop.integrate", &json!({"appimage": old.display().to_string(), "data_home": d.data_home.display().to_string(), "refresh": false})).unwrap();
        // The old one is still there: the user chooses.
        start_with(&mut app, d.clone(), false);
        assert!(matches!(app.desktop.as_ref().map(|d| &d.entry), Some(Entry::Other(_, true))));
        // Gone (an update replaced it): the entry follows this copy.
        std::fs::remove_file(&old).unwrap();
        start_with(&mut app, d.clone(), false);
        assert_eq!(app.desktop.as_ref().map(|d| d.entry.clone()), Some(Entry::ThisCopy));
        let _ = std::fs::remove_dir_all(&d.data_home);
    }
}
