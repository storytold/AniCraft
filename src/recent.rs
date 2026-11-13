//! Recent files in the window (ADR 0059): File › Open Recent, the list kept in the preferences,
//! and the thumbnails the start screen shows. The list itself is the session's
//! (`anicraft_engine::cmds_recent`): `file.open` and `file.save` add to it, the `prefs.recent_*`
//! commands change it, and the window copies it into the preferences whenever it changes.

use crate::app::App;
use crate::platform;
use crate::theme::{Type, text};
use anicraft_engine::cmds_recent::{self, RecentFile};
use serde_json::json;
use std::collections::HashMap;
use std::path::Path;

/// Thumbnails loaded for the start screen, by thumbnail key (`None`: there is none).
#[derive(Default)]
pub struct Thumbs {
    loaded: HashMap<String, Option<egui::TextureHandle>>,
    /// Pictures made in memory (screenshot runs show sample entries without touching disk).
    pub(crate) seeded: HashMap<String, Vec<u8>>,
}

impl Thumbs {
    /// The picture of `path`, decoded and uploaded once.
    pub fn get(&mut self, ctx: &egui::Context, path: &Path) -> Option<egui::TextureHandle> {
        let key = cmds_recent::thumb_key(path);
        if let Some(t) = self.loaded.get(&key) {
            return t.clone();
        }
        let png = self.seeded.get(&key).cloned().or_else(|| platform::load_thumbnail(&key));
        let tex = png.and_then(|png| anicraft_engine::doc::decode_png(&png)).and_then(|px| {
            let (w, h) = (usize::try_from(px.width).ok()?, usize::try_from(px.height).ok()?);
            (w > 0 && h > 0 && px.rgba.len() == w * h * 4).then(|| ctx.load_texture(format!("recent-{key}"), egui::ColorImage::from_rgba_unmultiplied([w, h], &px.rgba), egui::TextureOptions::LINEAR))
        });
        self.loaded.insert(key, tex.clone());
        tex
    }

    /// Forgets `path`'s picture, so the next look reads the new one.
    pub fn invalidate(&mut self, path: &Path) {
        self.loaded.remove(&cmds_recent::thumb_key(path));
    }
}

/// The picture of the open document's first frame, kept for `path` (after a save or an open).
/// Screenshot and test runs keep nothing on disk.
pub fn capture(app: &mut App, path: &Path) {
    if app.shot.is_some() {
        return;
    }
    let t = web_time::Instant::now();
    match cmds_recent::thumbnail_png(&app.s.doc) {
        Ok(png) => {
            if let Err(e) = platform::save_thumbnail(&cmds_recent::thumb_key(path), &png) {
                eprintln!("could not keep the thumbnail of {}: {e}", path.display());
            }
            app.thumbs.invalidate(path);
        }
        Err(e) => eprintln!("could not draw the thumbnail of {}: {e}", path.display()),
    }
    if cfg!(debug_assertions) {
        eprintln!("thumbnail of {} in {:.1} ms", path.display(), t.elapsed().as_secs_f64() * 1000.0);
    }
}

/// After a document was opened: a picture if it has none yet (one made on save is kept).
pub fn captured_on_open(app: &mut App, path: &Path) {
    if app.shot.is_none() && platform::load_thumbnail(&cmds_recent::thumb_key(path)).is_none() {
        capture(app, path);
    }
}

/// Copies the session's list into the preferences when a command changed it, and saves them
/// (not in screenshot or test runs, which never write the user's preferences).
pub fn sync(app: &mut App) {
    if app.s.recent == app.prefs.recent {
        return;
    }
    app.prefs.recent = app.s.recent.clone();
    if app.shot.is_none()
        && let Err(e) = app.prefs.save()
    {
        app.status_err(tr!("Could not save preferences: {e}", e = e));
    }
}

/// The file's name, as the menu and the start screen show it.
pub fn display_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// The name of the folder it is in.
pub fn folder_name(path: &Path) -> String {
    path.parent().and_then(Path::file_name).map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

/// The recent files with whether each is still there.
pub fn entries(app: &App) -> Vec<(usize, RecentFile, bool)> {
    app.s.recent.entries().iter().enumerate().map(|(i, e)| (i, e.clone(), !cmds_recent::exists(&e.path))).collect()
}

/// File › Open Recent: pinned favourites, then the last files; a missing file is greyed out
/// with Remove beside it. Every action is a command.
pub fn menu(app: &mut App, ui: &mut egui::Ui) {
    ui.set_min_width(320.0);
    let list = entries(app);
    if list.is_empty() {
        ui.label(text(Type::Caption, tr!("No recent files yet. Documents you open or save show here.")));
        return;
    }
    let mut act: Option<(&'static str, serde_json::Value)> = None;
    let mut last_pinned = list.first().is_some_and(|(_, e, _)| e.pinned);
    if last_pinned {
        ui.label(text(Type::Caption, tr!("Pinned")));
    }
    for (i, e, missing) in &list {
        if last_pinned && !e.pinned {
            ui.separator();
            last_pinned = false;
        }
        let name = display_name(&e.path);
        ui.horizontal(|ui| {
            let label = if *missing { tr!("{name} (missing)", name = name) } else { name.clone() };
            let r = ui.add_enabled(!*missing, egui::Button::new(label).min_size(egui::vec2(200.0, 0.0)));
            let tip = e.path.display().to_string();
            let r = if *missing { r.on_disabled_hover_text(tr!("{path} is no longer there", path = tip)) } else { r.on_hover_text(tip) };
            if r.clicked() {
                act = Some(("file.open_recent", json!({"index": i})));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if *missing {
                    if ui.small_button(tr!("Remove")).on_hover_text(tr!("Take it off the list")).clicked() {
                        act = Some(("prefs.recent_remove", json!({"index": i})));
                    }
                } else if ui.small_button(if e.pinned { tr!("Unpin") } else { tr!("Pin") }).on_hover_text(if e.pinned { tr!("Let it move down the list again") } else { tr!("Keep it at the top of the list") }).clicked() {
                    act = Some(("prefs.recent_pin", json!({"index": i})));
                }
            });
        });
    }
    ui.separator();
    if ui.button(tr!("Clear Recent Files")).on_hover_text(tr!("Pinned files stay")).clicked() {
        act = Some(("prefs.recent_clear", json!({})));
    }
    if let Some((id, params)) = act {
        // Opening keeps the menu's promise; pinning and removing keep the menu open.
        if id == "file.open_recent" {
            ui.close();
        }
        open_or_run(app, id, params);
    }
}

/// Runs a recent-files command from a menu or the start screen; opening goes through
/// `request`, so unsaved work asks first.
pub fn open_or_run(app: &mut App, id: &str, params: serde_json::Value) {
    if id == "file.open_recent" {
        app.request(id, params);
    } else if let Some(v) = app.run(id, params)
        && id == "prefs.recent_clear"
    {
        let n = v["removed"].as_u64().unwrap_or(0) as usize;
        app.status_ok(trn!("Cleared {n} recent file", "Cleared {n} recent files", n));
    }
}

/// Sample entries for screenshot and test runs (`--open start`): two documents made from the
/// samples in a scratch folder (one pinned), and one that is missing. Their pictures stay in
/// memory; nothing goes into the settings folder.
pub fn sample_entries(app: &mut App) {
    let folder = std::env::temp_dir().join("AniCraft samples"); // i18n-exempt: a folder name
    let _ = anicraft_engine::vfs::create_dir_all(&folder);
    let docs = [("Bouncing ball.anicraft", Some(anicraft_engine::doc::sample::demo()), true), ("Title card.anicraft", Some(anicraft_engine::doc::sample::interactive()), false), ("Old walk cycle.anicraft", None, false)]; // i18n-exempt: sample file names
    let mut list = anicraft_engine::cmds_recent::RecentFiles::default();
    for (name, doc, pinned) in docs.into_iter().rev() {
        let path = folder.join(name);
        match doc {
            Some(doc) => {
                if let Ok(png) = cmds_recent::thumbnail_png(&doc) {
                    app.thumbs.seeded.insert(cmds_recent::thumb_key(&path), png);
                }
                let _ = anicraft_engine::format::save(&doc, &path);
            }
            None => {
                let _ = anicraft_engine::vfs::remove_file(&path);
            }
        }
        list.add(&path);
        if pinned {
            let _ = list.pin(&path, true);
        }
    }
    app.s.recent = list;
}
