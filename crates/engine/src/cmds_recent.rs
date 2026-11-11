//! Recent files (ADR 0059): the documents opened or saved last, most recent first, with pinned
//! favourites kept above them. The list belongs to the session (it survives New and Open) and
//! is plain data: the desktop app keeps it in its preferences and puts it back at start, so the
//! engine never reads or writes the settings folder. `file.open` and `file.save` add to it;
//! these commands open, pin, remove and clear entries for menus, the CLI and MCP alike.

use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Most recent (unpinned) files kept, as Animate's Open Recent shows.
pub const MAX_RECENT: usize = 10;
/// Most pinned favourites.
pub const MAX_PINNED: usize = 10;
/// Longest path kept (anything longer is not a path a user picked).
const MAX_PATH_BYTES: usize = 4096;

/// One file in the list.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecentFile {
    pub path: PathBuf,
    /// A favourite: it stays at the top, and adding or clearing recent files leaves it alone.
    pub pinned: bool,
}

/// Pinned favourites first (in the order they were pinned), then the recent files, most recent
/// first. At most [`MAX_PINNED`] + [`MAX_RECENT`] entries, never the same file twice.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RecentFiles {
    entries: Vec<RecentFile>,
}

/// Whether two paths name the same file as the user sees it: Windows and macOS file names
/// ignore case (by default), and Windows takes either slash.
pub fn same_file(a: &Path, b: &Path) -> bool {
    if cfg!(any(windows, target_os = "macos")) {
        let norm = |p: &Path| p.to_string_lossy().replace('\\', "/").to_lowercase();
        norm(a) == norm(b)
    } else {
        a == b
    }
}

/// Whether a recent file is still there to open (a file, or an XFL folder).
pub fn exists(path: &Path) -> bool {
    crate::vfs::is_file(path) || crate::vfs::is_dir(path)
}

/// A short, stable key for a path (64-bit FNV-1a of its normalised text, as hex): the name its
/// thumbnail is kept under.
pub fn thumb_key(path: &Path) -> String {
    let text = if cfg!(any(windows, target_os = "macos")) { path.to_string_lossy().replace('\\', "/").to_lowercase() } else { path.to_string_lossy().into_owned() };
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Largest edge of a recent file's thumbnail, in pixels.
pub const THUMB_W: u32 = 240;
pub const THUMB_H: u32 = 135;

/// The document's first frame as a small PNG that fits [`THUMB_W`] × [`THUMB_H`] (the start
/// screen's picture of a recent file), rendered by the CPU oracle.
pub fn thumbnail_png(doc: &anicraft_doc::Document) -> Result<Vec<u8>> {
    let (w, h) = (doc.stage.width.max(1) as f32, doc.stage.height.max(1) as f32);
    let scale = (THUMB_W as f32 / w).min(THUMB_H as f32 / h).min(1.0);
    crate::cmds_file::render_png(doc, 0, scale)
}

impl RecentFiles {
    pub fn entries(&self) -> &[RecentFile] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn position(&self, path: &Path) -> Option<usize> {
        self.entries.iter().position(|e| same_file(&e.path, path))
    }

    fn pinned_count(&self) -> usize {
        self.entries.iter().take_while(|e| e.pinned).count()
    }

    /// A file was opened or saved: it becomes the most recent. A pinned file keeps its place;
    /// the oldest recent file beyond [`MAX_RECENT`] drops off. Returns whether the list changed.
    pub fn add(&mut self, path: &Path) -> bool {
        if path.as_os_str().is_empty() || path.as_os_str().len() > MAX_PATH_BYTES {
            return false;
        }
        let before = self.entries.clone();
        if let Some(i) = self.position(path) {
            if self.entries.get(i).is_some_and(|e| e.pinned) {
                return false;
            }
            self.entries.remove(i);
        }
        let at = self.pinned_count();
        self.entries.insert(at, RecentFile { path: path.to_path_buf(), pinned: false });
        self.entries.truncate(at + MAX_RECENT);
        before != self.entries
    }

    /// Takes a file off the list (pinned or not). Returns whether it was there.
    pub fn remove(&mut self, path: &Path) -> bool {
        match self.position(path) {
            Some(i) => {
                self.entries.remove(i);
                true
            }
            None => false,
        }
    }

    /// Pins a file (it goes below the other pinned files) or unpins it (it becomes the most
    /// recent file). A file not on the list is added pinned.
    pub fn pin(&mut self, path: &Path, pinned: bool) -> std::result::Result<(), String> {
        let current = self.position(path).and_then(|i| self.entries.get(i)).map(|e| e.pinned);
        if current == Some(pinned) {
            return Ok(());
        }
        if pinned && self.pinned_count() >= MAX_PINNED {
            return Err(format!("at most {MAX_PINNED} files can be pinned; unpin one first"));
        }
        if path.as_os_str().is_empty() || path.as_os_str().len() > MAX_PATH_BYTES {
            return Err("that is not a file path".into());
        }
        self.remove(path);
        let at = self.pinned_count();
        self.entries.insert(at, RecentFile { path: path.to_path_buf(), pinned });
        let pins = self.pinned_count();
        self.entries.truncate(pins + MAX_RECENT);
        Ok(())
    }

    /// Clears the recent files; pinned favourites stay unless `pinned` too. Returns how many
    /// were taken off.
    pub fn clear(&mut self, pinned: bool) -> usize {
        let before = self.entries.len();
        self.entries.retain(|e| e.pinned && !pinned);
        before - self.entries.len()
    }

    /// The list as read from a preferences file that anyone may have edited: empty and overlong
    /// paths and repeats dropped, pinned files first, each part within its limit.
    pub fn sanitized(self) -> RecentFiles {
        let mut out = RecentFiles::default();
        let (pins, recents): (Vec<RecentFile>, Vec<RecentFile>) = self.entries.into_iter().partition(|e| e.pinned);
        for e in pins.into_iter().chain(recents) {
            let bad = e.path.as_os_str().is_empty() || e.path.as_os_str().len() > MAX_PATH_BYTES;
            if bad || out.position(&e.path).is_some() {
                continue;
            }
            let pins = out.pinned_count();
            if (e.pinned && pins >= MAX_PINNED) || (!e.pinned && out.entries.len() - pins >= MAX_RECENT) {
                continue;
            }
            out.entries.push(e);
        }
        out
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "file.open_recent",
            label: "Open Recent",
            // The File menu shows the files themselves (a submenu the shell draws).
            menu: "",
            shortcut: None,
            params: "{index? (in file.recent_list order, from 0) | path?, confirm? (needed over unsaved changes)}: opens it as file.open does; a file that is no longer there stays on the list, marked missing",
            mutates: false,
            run: open_recent,
        },
        CommandSpec {
            id: "file.recent_list",
            label: "Recent Files",
            menu: "",
            shortcut: None,
            params: "{} → {files: [{index, path, name, pinned, missing, thumb}]}: pinned favourites first, then the last 10 files opened or saved, most recent first (`thumb` names the start screen's picture)",
            mutates: false,
            run: list,
        },
        CommandSpec {
            id: "prefs.recent_pin",
            label: "Pin to Recent Files",
            menu: "",
            shortcut: None,
            params: "{index? | path?, pinned? (default: toggle)}: a pinned file stays at the top of Open Recent",
            mutates: false,
            run: pin,
        },
        CommandSpec { id: "prefs.recent_remove", label: "Remove from Recent Files", menu: "", shortcut: None, params: "{index? | path?}", mutates: false, run: remove },
        CommandSpec {
            id: "prefs.recent_clear",
            label: "Clear Recent Files",
            menu: "",
            shortcut: None,
            params: "{pinned? (default false: pinned favourites stay)} → {removed}",
            mutates: false,
            run: clear,
        },
    ]
}

/// The entry `index` or `path` names.
fn which(s: &Session, p: &Value) -> Result<PathBuf> {
    match (opt_u64(p, "index")?, opt_str(p, "path")?) {
        (Some(i), None) => usize::try_from(i).ok().and_then(|i| s.recent.entries().get(i)).map(|e| e.path.clone()).ok_or_else(|| EngineError::Param(format!("no recent file {i}; file.recent_list has {}", s.recent.len()))),
        (None, Some(path)) if !path.trim().is_empty() => Ok(PathBuf::from(path)),
        (None, Some(_)) => Err(EngineError::Param("param 'path' must name a file".into())),
        _ => Err(EngineError::Param("name the file with 'index' or 'path' (one of them)".into())),
    }
}

fn open_recent(s: &mut Session, p: &Value) -> Result<Value> {
    let path = which(s, p)?;
    if !exists(&path) {
        return Err(EngineError::State(format!("{} is no longer there (moved, renamed or deleted); remove it from Open Recent", path.display())));
    }
    crate::cmds_file::open(s, &json!({"path": path.display().to_string()}))
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let files: Vec<Value> = s
        .recent
        .entries()
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let name = e.path.file_name().map_or_else(|| e.path.display().to_string(), |n| n.to_string_lossy().into_owned());
            json!({"index": i, "path": e.path.display().to_string(), "name": name, "pinned": e.pinned, "missing": !exists(&e.path), "thumb": thumb_key(&e.path)})
        })
        .collect();
    Ok(json!({"files": files}))
}

fn pin(s: &mut Session, p: &Value) -> Result<Value> {
    let path = which(s, p)?;
    let now = s.recent.entries().iter().find(|e| same_file(&e.path, &path)).is_some_and(|e| e.pinned);
    let pinned = opt_bool(p, "pinned")?.unwrap_or(!now);
    s.recent.pin(&path, pinned).map_err(EngineError::State)?;
    Ok(json!({"path": path.display().to_string(), "pinned": pinned}))
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    let path = which(s, p)?;
    if !s.recent.remove(&path) {
        return Err(EngineError::State(format!("{} is not in the recent files", path.display())));
    }
    Ok(json!({"removed": path.display().to_string()}))
}

fn clear(s: &mut Session, p: &Value) -> Result<Value> {
    let pinned = opt_bool(p, "pinned")?.unwrap_or(false);
    Ok(json!({"removed": s.recent.clear(pinned)}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(r: &RecentFiles) -> Vec<String> {
        r.entries().iter().map(|e| format!("{}{}", if e.pinned { "*" } else { "" }, e.path.display())).collect()
    }

    #[test]
    fn most_recent_first_ten_at_most_never_twice() {
        let mut r = RecentFiles::default();
        for i in 0..14 {
            assert!(r.add(Path::new(&format!("f{i}.anicraft"))));
        }
        assert_eq!(r.len(), MAX_RECENT);
        assert_eq!(paths(&r).first().map(String::as_str), Some("f13.anicraft"));
        assert_eq!(paths(&r).last().map(String::as_str), Some("f4.anicraft"), "the oldest drop off");
        // Opening one again moves it to the top, without a second entry.
        assert!(r.add(Path::new("f8.anicraft")));
        assert_eq!(paths(&r)[..2], ["f8.anicraft", "f13.anicraft"]);
        assert_eq!(r.len(), MAX_RECENT);
        assert!(!r.add(Path::new("f8.anicraft")), "already the most recent");
        assert!(!r.add(Path::new("")), "no empty paths");
    }

    #[test]
    fn pinned_files_stay_on_top_and_survive_clearing() {
        let mut r = RecentFiles::default();
        for f in ["a", "b", "c"] {
            r.add(Path::new(f));
        }
        r.pin(Path::new("a"), true).unwrap();
        assert_eq!(paths(&r), ["*a", "c", "b"]);
        // New files go below the pins; opening a pinned file leaves it where it is.
        r.add(Path::new("d"));
        assert!(!r.add(Path::new("a")));
        assert_eq!(paths(&r), ["*a", "d", "c", "b"]);
        // Pins don't count towards the ten recent files.
        for i in 0..12 {
            r.add(Path::new(&format!("n{i}")));
        }
        assert_eq!(r.len(), 1 + MAX_RECENT);
        assert_eq!(r.clear(false), MAX_RECENT);
        assert_eq!(paths(&r), ["*a"]);
        // Unpinning makes it the most recent file.
        r.add(Path::new("z"));
        r.pin(Path::new("a"), false).unwrap();
        assert_eq!(paths(&r), ["a", "z"]);
        assert!(r.remove(Path::new("z")));
        assert!(!r.remove(Path::new("z")));
        assert_eq!(r.clear(true), 1);
        assert!(r.is_empty());
        for i in 0..MAX_PINNED {
            r.pin(Path::new(&format!("p{i}")), true).unwrap();
        }
        assert!(r.pin(Path::new("one too many"), true).is_err());
    }

    #[test]
    fn a_hand_edited_list_is_cleaned_up() {
        let raw = r#"[{"path": "b"}, {"path": "", "pinned": true}, {"path": "a", "pinned": true}, {"path": "b"}, {"nonsense": 1}]"#;
        let r: RecentFiles = serde_json::from_str::<RecentFiles>(raw).unwrap().sanitized();
        assert_eq!(paths(&r), ["*a", "b"]);
        let many: Vec<RecentFile> = (0..40).map(|i| RecentFile { path: PathBuf::from(format!("{i}")), pinned: i % 2 == 0 }).collect();
        let r = RecentFiles { entries: many }.sanitized();
        assert_eq!(r.entries().iter().filter(|e| e.pinned).count(), MAX_PINNED);
        assert_eq!(r.entries().iter().filter(|e| !e.pinned).count(), MAX_RECENT);
        assert!(r.entries().iter().take(MAX_PINNED).all(|e| e.pinned), "pins first");
        assert!(RecentFiles { entries: vec![RecentFile { path: PathBuf::from("x".repeat(5000)), pinned: false }] }.sanitized().is_empty());
    }

    #[test]
    fn thumbnail_keys_are_stable_and_distinct() {
        assert_eq!(thumb_key(Path::new("a/b.anicraft")), thumb_key(Path::new("a/b.anicraft")));
        assert_ne!(thumb_key(Path::new("a/b.anicraft")), thumb_key(Path::new("a/c.anicraft")));
        assert_eq!(thumb_key(Path::new("x")).len(), 16);
    }

    #[test]
    fn thumbnails_fit_the_start_screen() {
        let t = std::time::Instant::now();
        let png = thumbnail_png(&anicraft_doc::sample::demo()).unwrap();
        eprintln!("thumbnail of the sample: {:.2} ms, {} bytes", t.elapsed().as_secs_f64() * 1000.0, png.len());
        let img = image_size(&png);
        assert!(img.0 <= THUMB_W && img.1 <= THUMB_H && img.0 > 0 && img.1 > 0, "{img:?}");
    }

    /// Width and height from a PNG's header.
    fn image_size(png: &[u8]) -> (u32, u32) {
        let be = |i: usize| u32::from_be_bytes([png[i], png[i + 1], png[i + 2], png[i + 3]]);
        (be(16), be(20))
    }
}
