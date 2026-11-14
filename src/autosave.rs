//! Crash recovery in the shell (ADR 0015): when to autosave, and the window that offers work a
//! previous session left behind (Restore / Discard / Later).
//!
//! The engine does the work: [`Autosaver`] serializes snapshots of the persistent document on
//! its own thread, and the `file.recover` / `file.discard_recovery` commands act on what was
//! left behind. This module only decides *when* (Preferences: every N seconds, or after N
//! changes, whichever comes first) and shows the choices.
//!
//! In a browser (ADR 0036) the copies go to the site's storage instead of the recovery folder
//! (`web::BrowserSaver`, the same calls), and which tabs are still open is known a moment after
//! start, so the Recover window comes up then.

use crate::a11y::Named;
use crate::app::App;
use crate::prefs::Prefs;
use crate::theme::{T, Type, text};
#[cfg(target_arch = "wasm32")]
use crate::web::BrowserSaver as Autosaver;
use anicraft_engine::recovery::{self, Entry, Outcome};
#[cfg(not(target_arch = "wasm32"))]
use anicraft_engine::recovery::{Autosaver, SessionLock};
use anicraft_engine::StateId;
use serde_json::json;
use std::path::PathBuf;

/// Autosave state of the running app.
#[derive(Default)]
pub struct Recovery {
    saver: Option<Autosaver>,
    pub dir: Option<PathBuf>,
    /// Document state the newest copy holds (handed to the writer, or restored from disk).
    written: Option<StateId>,
    /// This session has a copy on disk (or one on its way).
    has_copy: bool,
    /// Revision at the last copy, to count changes since.
    rev_at_copy: u64,
    /// When the document first differed from the newest copy (app time, seconds).
    dirty_since: Option<f64>,
    /// When the newest copy was finished (app time, seconds).
    pub last_copy_at: Option<f64>,
    /// A restored copy of another session, removed once this session holds the work.
    adopt: Option<String>,
    failed: bool,
    /// Copies left by sessions that didn't close cleanly, and whether the window shows them.
    pub found: Vec<Entry>,
    pub show: bool,
    /// The session whose Discard is waiting for its second click.
    confirm_discard: Option<String>,
    /// In a browser: the list of copies is on its way; `true` when the user asked for it (so
    /// an empty one is said).
    #[cfg(target_arch = "wasm32")]
    scanning: Option<bool>,
}

/// Whether a new copy is due: unsaved for the whole interval, or enough changes made (not in
/// the middle of a drag, which would write a copy per frame of it).
pub fn due(unsaved_for: f64, changes: u64, in_gesture: bool, p: &Prefs) -> bool {
    unsaved_for >= f64::from(p.autosave_secs) || (p.autosave_edits > 0 && changes >= u64::from(p.autosave_edits) && !in_gesture)
}

impl Recovery {
    /// Starts this session's autosave and finds what earlier sessions left behind. Screenshot
    /// runs leave the real folder alone unless `ANICRAFT_RECOVERY_DIR` names one.
    pub fn start(shot_mode: bool) -> (Recovery, Option<String>) {
        let mut r = Recovery::default();
        #[cfg(target_arch = "wasm32")]
        {
            let _ = shot_mode;
            match Autosaver::start() {
                Ok(s) => {
                    r.saver = Some(s);
                    r.scanning = Some(false);
                    (r, None)
                }
                Err(e) => (r, Some(tr!("Autosave is off: {error}", error = e))),
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            if shot_mode && !crate::platform::recovery_dir_overridden() {
                return (r, None);
            }
            let Some(dir) = crate::platform::recovery_dir() else { return (r, Some(tr!("Autosave is off: this system has no settings folder").into())) };
            recovery::prune(&dir);
            r.found = recovery::scan(&dir, None);
            r.show = !r.found.is_empty();
            let err = match SessionLock::acquire(&dir).and_then(Autosaver::start) {
                Ok(s) => {
                    r.saver = Some(s);
                    None
                }
                Err(e) => Some(tr!("Autosave is off: {error}", error = e)),
            };
            r.dir = Some(dir);
            (r, err)
        }
    }

    pub fn active(&self) -> bool {
        self.saver.is_some()
    }

    /// Rescans for left-behind copies (File › Recover Autosaved Documents…). `false` when the
    /// list comes later (a browser asks which tabs are open), and [`tick`] shows it then.
    pub fn rescan(&mut self) -> bool {
        #[cfg(target_arch = "wasm32")]
        {
            crate::web::refresh_live();
            self.scanning = Some(true);
            false
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(dir) = &self.dir {
                let own = self.saver.as_ref().map(|s| s.id().to_string());
                self.found = recovery::scan(dir, own.as_deref());
            }
            true
        }
    }

    /// A left-behind copy was restored: this session takes the work over (a copy right away),
    /// and the old copy goes once that is on disk.
    pub fn restored(&mut self, session: &str) {
        self.found.retain(|e| e.session != session);
        self.show = self.show && !self.found.is_empty();
        self.adopt = Some(session.to_string());
        self.written = None;
        self.dirty_since = Some(f64::NEG_INFINITY);
    }

    pub fn discarded(&mut self, session: &str) {
        self.found.retain(|e| e.session != session);
        self.confirm_discard = None;
        self.show = self.show && !self.found.is_empty();
    }

    /// Writes a copy of unsaved work now, without waiting for the interval (the graphics driver
    /// reported trouble, ADR 0092: the next thing may be the window going). Whether one was
    /// started; nothing to do when the newest copy already holds this state.
    pub fn write_now(&mut self, s: &anicraft_engine::Session) -> bool {
        let Some(saver) = &mut self.saver else { return false };
        if !s.is_dirty() || self.written == Some(s.state) {
            return false;
        }
        saver.write(s.doc.clone(), s.path.as_deref());
        self.has_copy = true;
        self.written = Some(s.state);
        self.rev_at_copy = s.revision;
        self.dirty_since = None;
        true
    }

    /// Ends the session: a clean exit removes this session's copy; anything else (unsaved work
    /// the user didn't decide about) is written one last time for the next start.
    pub fn finish(&mut self, s: &anicraft_engine::Session, clean: bool) {
        if let Some(saver) = self.saver.take() {
            let keep = !clean && s.is_dirty();
            saver.finish(keep.then(|| (s.doc.clone(), s.path.clone())));
        }
    }
}

/// Removes another session's copy once this one holds its work (the user chose Restore).
fn drop_adopted(app: &mut App, old: &str) {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = &app;
        crate::web::discard(old);
    }
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(dir) = app.recovery.dir.clone() {
        // Through the command, as any front end would.
        let _ = app.s.execute("file.discard_recovery", &json!({"dir": dir.display().to_string(), "session": old, "confirm": true}));
    }
}

/// In a browser: shows the copies once the browser has said which tabs are still open.
#[cfg(target_arch = "wasm32")]
fn poll_scan(app: &mut App) {
    let Some(asked) = app.recovery.scanning else { return };
    let own = app.recovery.saver.as_ref().map(|s| s.id().to_string());
    let Some(found) = crate::web::scan(own.as_deref()) else { return };
    let r = &mut app.recovery;
    r.scanning = None;
    r.found = found;
    r.show = !r.found.is_empty();
    if asked && !r.show {
        app.status_ok(tr!("No autosaved work to recover"));
    }
}

/// Where a restored browser copy is opened from: `recovered/<session>/<name>`, so its download
/// keeps the document's name.
pub const RESTORED_DIR: &str = "recovered";

/// The session a `file.open` restores, when it opens a browser copy (see [`RESTORED_DIR`]).
pub fn restored_session(cmd: &str, params: &serde_json::Value) -> Option<String> {
    if cmd != "file.open" || params.get("unsaved") != Some(&json!(true)) {
        return None;
    }
    let path = std::path::Path::new(params.get("path")?.as_str()?);
    let mut parts = path.components().map(|c| c.as_os_str().to_string_lossy().into_owned());
    (parts.next()? == RESTORED_DIR).then(|| parts.next()).flatten().filter(|id| recovery::valid_id(id))
}

/// Restore: opens the copy as unsaved work. Through `request`, so restoring over unsaved
/// changes asks first (ADR 0004).
fn restore(app: &mut App, id: String) {
    #[cfg(target_arch = "wasm32")]
    {
        let Some((json, meta)) = crate::web::load(&id) else {
            app.status_err(tr!("That autosaved copy is gone"));
            app.recovery.discarded(&id);
            return;
        };
        let name = meta.original.as_deref().and_then(std::path::Path::file_name).map_or_else(|| "Untitled.anicraft".to_string(), |n| n.to_string_lossy().into_owned()); // i18n-exempt: a file name
        let path = std::path::Path::new(RESTORED_DIR).join(&id).join(&name);
        match anicraft_engine::vfs::put(&path, json.into_bytes()) {
            Ok(()) => app.request("file.open", json!({"path": path.display().to_string(), "unsaved": true, "untitled": meta.original.is_none()})),
            Err(e) => app.status_err(tr!("Could not restore {name}: {error}", name = name, error = e)),
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let dir = app.recovery.dir.as_ref().map(|d| d.display().to_string()).unwrap_or_default();
        app.request("file.recover", json!({"dir": dir, "session": id}));
    }
}

/// Discard (after its second click): the copy is deleted for good.
fn discard(app: &mut App, id: String) {
    #[cfg(target_arch = "wasm32")]
    {
        crate::web::discard(&id);
        app.recovery.discarded(&id);
        app.status_ok(tr!("Discarded the autosaved copy"));
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let dir = app.recovery.dir.as_ref().map(|d| d.display().to_string()).unwrap_or_default();
        app.perform("file.discard_recovery", json!({"dir": dir, "session": id, "confirm": true}));
    }
}

/// Per frame: collects finished copies and starts a new one when it is due.
pub fn tick(app: &mut App, ctx: &egui::Context) {
    #[cfg(target_arch = "wasm32")]
    poll_scan(app);
    let now = ctx.input(|i| i.time);
    let outcomes = app.recovery.saver.as_ref().map(Autosaver::poll).unwrap_or_default();
    for o in outcomes {
        match o {
            Outcome::Written { .. } => {
                app.recovery.last_copy_at = Some(now);
                app.recovery.failed = false;
                if let Some(old) = app.recovery.adopt.take() {
                    drop_adopted(app, &old);
                }
            }
            Outcome::Failed { error, .. } => {
                if !app.recovery.failed {
                    app.status_err(tr!("Autosave failed: {error}. Save your work to be safe; AniCraft will keep trying.", error = error));
                }
                app.recovery.failed = true;
                app.recovery.written = None;
                app.recovery.dirty_since = Some(now);
            }
            Outcome::Cleared { .. } => {}
        }
    }
    let r = &mut app.recovery;
    let Some(saver) = &mut r.saver else { return };
    if !app.prefs.autosave || !app.s.is_dirty() {
        // Nothing to protect (saved, back at the saved state, a new document) or autosave off.
        if r.has_copy {
            saver.clear();
            r.has_copy = false;
            r.last_copy_at = None;
        }
        r.written = app.prefs.autosave.then_some(app.s.state);
        r.rev_at_copy = app.s.revision;
        r.dirty_since = None;
        return;
    }
    if r.written == Some(app.s.state) {
        return;
    }
    let since = *r.dirty_since.get_or_insert(now);
    let changes = app.s.revision.saturating_sub(r.rev_at_copy);
    if due(now - since, changes, app.s.in_gesture(), &app.prefs) {
        // An `Arc` snapshot: O(1) here; serializing and writing happen on the worker.
        saver.write(app.s.doc.clone(), app.s.path.as_deref());
        r.has_copy = true;
        r.written = Some(app.s.state);
        r.rev_at_copy = app.s.revision;
        r.dirty_since = None;
    } else {
        // egui repaints on input only: wake up when the interval runs out.
        let left = (f64::from(app.prefs.autosave_secs) - (now - since)).max(0.05);
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(left));
    }
}

/// "just now", "4 min ago", "2 h ago", "3 days ago".
pub fn ago(secs: u64) -> String {
    match secs {
        0..60 => tr!("just now").into(),
        60..3600 => trn!("{n} min ago", "{n} min ago", secs / 60),
        3600..86400 => trn!("{n} h ago", "{n} h ago", secs / 3600),
        _ => {
            let d = secs / 86400;
            if d == 1 { tr!("yesterday").into() } else { trn!("{n} day ago", "{n} days ago", d) }
        }
    }
}

/// The status-bar readout: when the open work was last copied, while it is unsaved.
pub fn status(app: &App, now: f64) -> Option<String> {
    if !app.s.is_dirty() || !app.prefs.autosave {
        return None;
    }
    let at = app.recovery.last_copy_at?;
    Some(tr!("autosaved {when}", when = ago((now - at).max(0.0) as u64)))
}

/// Two made-up left-behind documents, for `--open recovery` screenshots.
pub fn sample_entries() -> Vec<Entry> {
    let now = recovery::now_secs();
    let entry = |session: &str, name: &str, original: Option<&str>, age: u64, frames: u32, scenes: usize| Entry {
        session: session.into(),
        document: PathBuf::new(),
        meta: recovery::RecoveryMeta { name: name.into(), original: original.map(PathBuf::from), saved_at: now.saturating_sub(age), frames, scenes, ..Default::default() },
        bytes: 0,
    };
    let walk = if crate::platform::OS == crate::platform::Os::Windows { "C:\\Users\\you\\Documents\\walk-cycle.anicraft" } else { "/home/you/Documents/walk-cycle.anicraft" };
    // i18n-exempt: document names, as the engine records them
    vec![entry("sample-1", "walk-cycle.anicraft", Some(walk), 240, 48, 2), entry("sample-2", "Untitled", None, 3 * 3600, 120, 1)]
}

enum Act {
    Restore(String),
    AskDiscard(String),
    Discard(String),
    KeepIt,
    Later,
}

/// The Recover window: non-blocking (the editor stays usable behind it), one row per
/// left-behind document with Restore and Discard, and Later to decide another time.
pub fn window(app: &mut App, ctx: &egui::Context) {
    if !app.recovery.show || app.recovery.found.is_empty() {
        app.recovery.show = false;
        return;
    }
    let mut act: Option<Act> = None;
    let mut open = true;
    let now = recovery::now_secs();
    let n = app.recovery.found.len();
    let confirm = app.recovery.confirm_discard.clone();
    let btn = egui::vec2(96.0, crate::theme::TARGET + 4.0);
    let title = tr!("Recover unsaved work");
    let shown = egui::Window::new(title)
        .id(egui::Id::new("recover_window"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .default_width(520.0)
        .show(ctx, |ui| {
            ui.set_width(520.0);
            let lead = trn!(
                "AniCraft didn't close properly last time. This document had changes that weren't saved:",
                "AniCraft didn't close properly last time. These documents had changes that weren't saved:",
                n
            );
            ui.label(text(Type::Body, lead).color(T.text_dim));
            ui.add_space(8.0);
            for e in &app.recovery.found {
                egui::Frame::NONE.fill(T.panel_alt).corner_radius(T.radius_menu).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(text(Type::Body, e.meta.name.clone()).strong());
                            let when = ago(now.saturating_sub(e.meta.saved_at));
                            let detail = if e.meta.scenes > 1 {
                                trn!("Autosaved {when} · {n} frame · {scenes} scenes", "Autosaved {when} · {n} frames · {scenes} scenes", e.meta.frames, when = when, scenes = e.meta.scenes)
                            } else {
                                trn!("Autosaved {when} · {n} frame", "Autosaved {when} · {n} frames", e.meta.frames, when = when)
                            };
                            ui.label(text(Type::Caption, detail));
                            match &e.meta.original {
                                Some(o) => {
                                    ui.label(text(Type::Path, o.display().to_string()).color(T.text_faint));
                                }
                                None => {
                                    ui.label(text(Type::Caption, tr!("Never saved")));
                                }
                            }
                            if e.original_is_newer() {
                                ui.label(text(Type::Caption, tr!("The file was saved after this copy, so it opens as a new document and Save asks for a name.")).color(T.error));
                            }
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if confirm.as_deref() == Some(e.session.as_str()) {
                                let discard = egui::Button::new(text(Type::Body, tr!("Discard")).color(T.on_fill).strong()).fill(T.danger).min_size(btn);
                                if ui.add(discard).clicked() {
                                    act = Some(Act::Discard(e.session.clone()));
                                }
                                if ui.add(egui::Button::new(tr!("Keep")).min_size(btn)).clicked() {
                                    act = Some(Act::KeepIt);
                                }
                            } else {
                                let restore = egui::Button::new(text(Type::Body, tr!("Restore")).color(T.on_fill).strong()).fill(T.accent).min_size(btn);
                                if ui.add(restore).named(&tr!("Restore {name}", name = e.meta.name)).on_hover_text(tr!("Open it as unsaved work")).clicked() {
                                    act = Some(Act::Restore(e.session.clone()));
                                }
                                if ui.add(egui::Button::new(tr!("Discard…")).min_size(btn)).named(&tr!("Discard {name}…", name = e.meta.name)).on_hover_text(tr!("Delete this autosaved copy")).clicked() {
                                    act = Some(Act::AskDiscard(e.session.clone()));
                                }
                            }
                        });
                    });
                    if confirm.as_deref() == Some(e.session.as_str()) {
                        ui.label(text(Type::Caption, tr!("Discarding deletes this work for good. This cannot be undone.")).color(T.error));
                    }
                });
                ui.add_space(6.0);
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(tr!("Later")).min_size(btn)).clicked() {
                    act = Some(Act::Later);
                }
                ui.label(text(Type::Caption, tr!("Later keeps them: File › Recover Autosaved Documents… brings this back.")));
            });
        });
    crate::a11y::name_title_bar(&shown, title);
    if !open {
        act = Some(Act::Later);
    }
    match act {
        Some(Act::Restore(id)) => {
            app.recovery.confirm_discard = None;
            restore(app, id);
        }
        Some(Act::AskDiscard(id)) => app.recovery.confirm_discard = Some(id),
        // The second click on the red button is the confirmation.
        Some(Act::Discard(id)) => discard(app, id),
        Some(Act::KeepIt) => app.recovery.confirm_discard = None,
        Some(Act::Later) => {
            app.recovery.show = false;
            app.recovery.confirm_discard = None;
            app.status_ok(tr!("Kept the autosaved work. File › Recover Autosaved Documents… brings it back."));
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_are_due_by_time_or_by_changes() {
        let p = Prefs::default(); // 60 s or 25 changes
        assert!(!due(10.0, 3, false, &p));
        assert!(due(60.0, 1, false, &p), "never more than the interval");
        assert!(due(61.0, 1, true, &p), "even in a long gesture");
        assert!(due(5.0, 25, false, &p));
        assert!(!due(5.0, 400, true, &p), "not per frame of a drag");
        let time_only = Prefs { autosave_edits: 0, ..Prefs::default() };
        assert!(!due(5.0, 10_000, false, &time_only));
    }

    #[test]
    fn restored_browser_copies_are_recognized() {
        let open = |path: &str| json!({"path": path, "unsaved": true});
        assert_eq!(restored_session("file.open", &open("recovered/web-1a-2b/walk.anicraft")).as_deref(), Some("web-1a-2b"));
        assert_eq!(restored_session("file.open", &json!({"path": "recovered/web-1/walk.anicraft"})), None, "an ordinary open");
        assert_eq!(restored_session("file.open", &open("walk.anicraft")), None);
        assert_eq!(restored_session("file.open", &open("recovered/../x/y")), None);
        assert_eq!(restored_session("file.save", &open("recovered/web-1/walk.anicraft")), None);
    }

    #[test]
    fn ages_read_naturally() {
        assert_eq!(ago(5), "just now");
        assert_eq!(ago(240), "4 min ago");
        assert_eq!(ago(7300), "2 h ago");
        assert_eq!(ago(90_000), "yesterday");
        assert_eq!(ago(3 * 86_400), "3 days ago");
    }
}
