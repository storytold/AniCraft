//! AppImage desktop integration (ADR 0094): `desktop.integrate / remove / status`.
//!
//! An AppImage is one file the user downloaded: nothing installs its launcher, icons or file
//! types. With the user's yes, `desktop.integrate` writes them into the user's XDG data folder
//! (`$XDG_DATA_HOME`, default `~/.local/share`), as a package would into `/usr/share`:
//!
//! * `applications/ai.storyteller.anicraft.desktop`: the packaged entry (`packaging/linux`) with
//!   `Exec` and `TryExec` pointing at the `.AppImage`, quoted as the Desktop Entry spec says, and
//!   an `X-AniCraft-AppImage` key that marks it as ours;
//! * `mime/packages/ai.storyteller.anicraft.xml`: the packaged MIME types (`.anicraft`, `.fla`,
//!   `.xfl`);
//! * `icons/hicolor/<size>/apps/ai.storyteller.anicraft.{png,svg}`: the packaged icons.
//!
//! Then it refreshes the user's MIME and desktop databases (`update-mime-database`,
//! `update-desktop-database`) where those tools are installed. It never writes outside the data
//! folder it is given, refuses system folders, and is idempotent: writing again changes only
//! what differs. `desktop.remove` deletes exactly these files, and only when the entry is ours.
//! The paths are parameters, so tests, the CLI and MCP name them; the desktop app passes its
//! own (`platform::appimage`, `platform::data_home`).

use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, params, run| CommandSpec { id, label, menu: "", shortcut: None, params, mutates: false, run };
    vec![
        c(
            "desktop.integrate",
            "Add to Applications Menu",
            "{appimage: absolute path to the .AppImage, data_home: absolute path (the user's $XDG_DATA_HOME, e.g. ~/.local/share), refresh? (default true: run update-mime-database and update-desktop-database)} writes the launcher, icons and file types there; again only rewrites what changed",
            integrate as crate::RunFn,
        ),
        c("desktop.remove", "Remove from Applications Menu", "{data_home, refresh? (default true)} deletes the launcher, icons and file types desktop.integrate wrote; nothing when the entry there isn't AniCraft's", remove),
        c("desktop.status", "Applications Menu Status", "{data_home, appimage? (to say whether the entry starts this copy)} whether AniCraft is in the applications menu, and which AppImage it starts", status),
    ]
}

/// The app id (ADR 0026), which names every file written.
pub const APP_ID: &str = "ai.storyteller.anicraft";
/// The key that marks a desktop entry as written by `desktop.integrate`; its value is the
/// AppImage it starts.
pub const MARKER_KEY: &str = "X-AniCraft-AppImage";

const DESKTOP_TEMPLATE: &str = include_str!("../../../packaging/linux/ai.storyteller.anicraft.desktop");
const MIME_XML: &str = include_str!("../../../packaging/linux/ai.storyteller.anicraft.mime.xml");

/// The packaged icons: `(hicolor size folder, file extension, contents)`. The 512 px one is left
/// out (desktops scale the SVG), which keeps the programs small.
const ICONS: [(&str, &str, &[u8]); 8] = [
    ("16x16", "png", include_bytes!("../../../assets/app-icon/hicolor/16x16/apps/ai.storyteller.anicraft.png")),
    ("24x24", "png", include_bytes!("../../../assets/app-icon/hicolor/24x24/apps/ai.storyteller.anicraft.png")),
    ("32x32", "png", include_bytes!("../../../assets/app-icon/hicolor/32x32/apps/ai.storyteller.anicraft.png")),
    ("48x48", "png", include_bytes!("../../../assets/app-icon/hicolor/48x48/apps/ai.storyteller.anicraft.png")),
    ("64x64", "png", include_bytes!("../../../assets/app-icon/hicolor/64x64/apps/ai.storyteller.anicraft.png")),
    ("128x128", "png", include_bytes!("../../../assets/app-icon/hicolor/128x128/apps/ai.storyteller.anicraft.png")),
    ("256x256", "png", include_bytes!("../../../assets/app-icon/hicolor/256x256/apps/ai.storyteller.anicraft.png")),
    ("scalable", "svg", include_bytes!("../../../assets/app-icon/hicolor/scalable/apps/ai.storyteller.anicraft.svg")),
];

/// Longest path accepted (anything longer is not a path a user has).
const MAX_PATH_BYTES: usize = 4096;

/// System folders desktop integration never writes into, whatever it is told: packages own them.
const SYSTEM: [&str; 10] = ["/usr", "/etc", "/opt", "/var/lib", "/bin", "/sbin", "/lib", "/lib64", "/boot", "/snap"];

/// `path` as text a desktop entry can hold: UTF-8, absolute, no control characters, bounded.
fn entry_text<'a>(path: &'a Path, what: &str) -> Result<&'a str> {
    let text = path.to_str().ok_or_else(|| EngineError::Param(format!("the {what} path is not valid UTF-8, which a desktop entry can't hold")))?;
    if text.is_empty() || text.len() > MAX_PATH_BYTES {
        return Err(EngineError::Param(format!("the {what} path is empty or too long")));
    }
    if text.chars().any(char::is_control) {
        return Err(EngineError::Param(format!("the {what} path has a line break or control character, which a desktop entry can't hold")));
    }
    // `/…` counts on every system, so the Linux layouts are tested on any OS.
    if !path.is_absolute() && !text.starts_with('/') {
        return Err(EngineError::Param(format!("the {what} path must be absolute: {text}")));
    }
    Ok(text)
}

/// A desktop entry string value: backslashes doubled (the spec's only escape we need, since
/// control characters were refused).
fn escape_value(s: &str) -> String {
    s.replace('\\', "\\\\")
}

/// Reads a string value back ([`escape_value`] and the spec's `\s \n \t \r`).
fn unescape_value(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// The program argument of an `Exec` line for `path`: always in double quotes, with `"`, `` ` ``,
/// `$` and `\` escaped inside them and `%` doubled (it starts field codes), then escaped as a
/// string value. So spaces, quotes and dollar signs in folder names start the right file.
pub fn exec_argument(path: &str) -> String {
    let mut quoted = String::with_capacity(path.len() + 2);
    quoted.push('"');
    for c in path.chars() {
        match c {
            '"' | '`' | '$' | '\\' => {
                quoted.push('\\');
                quoted.push(c);
            }
            '%' => quoted.push_str("%%"),
            _ => quoted.push(c),
        }
    }
    quoted.push('"');
    escape_value(&quoted)
}

/// The desktop entry that starts the AppImage at `appimage`: the packaged one with `Exec`,
/// `TryExec` (so the entry hides itself if the file is moved or deleted) and the marker key.
pub fn desktop_entry(appimage: &Path) -> Result<String> {
    let text = entry_text(appimage, "AppImage")?;
    let mut out = String::from("# Added by AniCraft for the AppImage named in X-AniCraft-AppImage.\n# AniCraft's Preferences › About › Remove takes it away again.\n");
    for line in DESKTOP_TEMPLATE.lines() {
        let line = line.trim_end_matches('\r');
        if line.starts_with("Exec=") {
            out.push_str(&format!("Exec={} %f\n", exec_argument(text)));
        } else if line.starts_with("TryExec=") {
            out.push_str(&format!("TryExec={}\n", escape_value(text)));
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push_str(&format!("{MARKER_KEY}={}\n", escape_value(text)));
    Ok(out)
}

/// The AppImage a desktop entry written by [`desktop_entry`] starts, or `None` if the entry is
/// not ours.
pub fn entry_appimage(entry: &str) -> Option<PathBuf> {
    let prefix = format!("{MARKER_KEY}=");
    entry.lines().find_map(|l| l.trim_end_matches('\r').strip_prefix(&prefix)).map(|v| PathBuf::from(unescape_value(v)))
}

/// Checks the data folder: absolute, valid, and not a system folder.
fn check_data_home(data_home: &Path) -> Result<()> {
    let text = entry_text(data_home, "data folder")?;
    let norm = text.trim_end_matches('/');
    if norm.is_empty() || SYSTEM.iter().any(|s| norm == *s || norm.starts_with(&format!("{s}/"))) {
        return Err(EngineError::Param(format!("{text} is a system folder; AniCraft adds itself only to your own data folder (~/.local/share)")));
    }
    Ok(())
}

/// Every file desktop integration owns under `data_home`, relative to it.
pub fn owned_files() -> Vec<PathBuf> {
    let mut v = vec![PathBuf::from("applications").join(format!("{APP_ID}.desktop")), PathBuf::from("mime").join("packages").join(format!("{APP_ID}.xml"))];
    v.extend(ICONS.iter().map(|(size, ext, _)| PathBuf::from("icons").join("hicolor").join(size).join("apps").join(format!("{APP_ID}.{ext}"))));
    v
}

/// The files to write for `appimage`: `(path under data_home, contents)`, in [`owned_files`] order.
fn planned(appimage: &Path) -> Result<Vec<(PathBuf, Vec<u8>)>> {
    let mut contents: Vec<Vec<u8>> = vec![desktop_entry(appimage)?.into_bytes(), MIME_XML.as_bytes().to_vec()];
    contents.extend(ICONS.iter().map(|(_, _, b)| b.to_vec()));
    Ok(owned_files().into_iter().zip(contents).collect())
}

/// Writes `data` to `path` through a temporary file and a rename, so a crash never leaves half a
/// launcher; leaves the file alone when it already holds `data`. Whether it wrote.
fn write_if_changed(path: &Path, data: &[u8]) -> Result<bool> {
    if std::fs::read(path).is_ok_and(|old| old == data) {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".anicraft-tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, data)?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(true)
}

/// What [`install`] did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Files written (new or changed), and files already as they should be.
    pub written: Vec<PathBuf>,
    pub unchanged: Vec<PathBuf>,
}

/// The desktop entry under `data_home`, and whether it is ours.
fn entry_path(data_home: &Path) -> PathBuf {
    data_home.join("applications").join(format!("{APP_ID}.desktop"))
}

/// Adds AniCraft (the AppImage at `appimage`) to the menu under `data_home`. Refuses to replace an
/// entry of the same name that it didn't write (a hand-made one, say).
pub fn install(appimage: &Path, data_home: &Path) -> Result<Report> {
    check_data_home(data_home)?;
    entry_text(appimage, "AppImage")?;
    if !appimage.is_file() {
        return Err(EngineError::Param(format!("there is no AppImage at {}", appimage.display())));
    }
    let entry = entry_path(data_home);
    if let Ok(old) = std::fs::read_to_string(&entry)
        && entry_appimage(&old).is_none()
    {
        return Err(EngineError::State(format!("{} was not written by AniCraft, so it is left as it is", entry.display())));
    }
    let mut r = Report::default();
    for (rel, data) in planned(appimage)? {
        let path = data_home.join(&rel);
        if write_if_changed(&path, &data)? { r.written.push(path) } else { r.unchanged.push(path) }
    }
    Ok(r)
}

/// Removes what [`install`] wrote under `data_home`: the files that are there. Nothing when the
/// entry there isn't ours (an error then), so it never deletes someone else's launcher.
pub fn uninstall(data_home: &Path) -> Result<Vec<PathBuf>> {
    check_data_home(data_home)?;
    let entry = entry_path(data_home);
    if let Ok(old) = std::fs::read_to_string(&entry)
        && entry_appimage(&old).is_none()
    {
        return Err(EngineError::State(format!("{} was not written by AniCraft, so it is left as it is", entry.display())));
    }
    let mut removed = Vec::new();
    for rel in owned_files() {
        let path = data_home.join(rel);
        if path.is_file() {
            std::fs::remove_file(&path)?;
            removed.push(path);
        }
    }
    Ok(removed)
}

/// Whether AniCraft is in the menu under `data_home`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// The AppImage our entry starts, when there is one.
    pub appimage: Option<PathBuf>,
    /// An entry of our name is there that AniCraft didn't write.
    pub foreign: bool,
}

pub fn read_status(data_home: &Path) -> Status {
    match std::fs::read_to_string(entry_path(data_home)) {
        Ok(text) => match entry_appimage(&text) {
            Some(a) => Status { appimage: Some(a), foreign: false },
            None => Status { appimage: None, foreign: true },
        },
        Err(_) => Status::default(),
    }
}

/// Runs `program args…` if it is installed, for at most a few seconds, ignoring what it says:
/// the caches it refreshes speed things up, and desktops rescan without them.
fn run_quietly(program: &str, args: &[&std::ffi::OsStr]) {
    use std::process::{Command, Stdio};
    let Ok(mut child) = Command::new(program).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() else { return };
    for _ in 0..250 {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => return,
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(20)),
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Refreshes the user's MIME and desktop databases under `data_home` (best effort).
fn refresh(data_home: &Path) {
    let mime = data_home.join("mime");
    if mime.join("packages").is_dir() {
        run_quietly("update-mime-database", &[mime.as_os_str()]);
    }
    let apps = data_home.join("applications");
    if apps.is_dir() {
        run_quietly("update-desktop-database", &[std::ffi::OsStr::new("-q"), apps.as_os_str()]);
    }
}

fn path_req(p: &Value, key: &str) -> Result<PathBuf> {
    let s = str_req(p, key)?;
    if s.trim().is_empty() {
        return Err(EngineError::Param(format!("'{key}' is empty")));
    }
    Ok(PathBuf::from(s))
}

fn shown(paths: &[PathBuf]) -> Vec<String> {
    paths.iter().map(|p| p.display().to_string()).collect()
}

fn integrate(_: &mut Session, p: &Value) -> Result<Value> {
    let (appimage, data_home) = (path_req(p, "appimage")?, path_req(p, "data_home")?);
    let refresh_after = opt_bool(p, "refresh")?.unwrap_or(true);
    let r = install(&appimage, &data_home)?;
    if refresh_after && !r.written.is_empty() {
        refresh(&data_home);
    }
    Ok(json!({"added": true, "appimage": appimage.display().to_string(), "written": shown(&r.written), "unchanged": r.unchanged.len()}))
}

fn remove(_: &mut Session, p: &Value) -> Result<Value> {
    let data_home = path_req(p, "data_home")?;
    let refresh_after = opt_bool(p, "refresh")?.unwrap_or(true);
    let removed = uninstall(&data_home)?;
    if refresh_after && !removed.is_empty() {
        refresh(&data_home);
    }
    Ok(json!({"removed": shown(&removed)}))
}

fn status(_: &mut Session, p: &Value) -> Result<Value> {
    let data_home = path_req(p, "data_home")?;
    check_data_home(&data_home)?;
    let st = read_status(&data_home);
    let current = match opt_str(p, "appimage")? {
        Some(a) => st.appimage.as_deref().is_some_and(|x| x == Path::new(a)),
        None => false,
    };
    Ok(json!({
        "added": st.appimage.is_some(),
        "appimage": st.appimage.as_ref().map(|a| a.display().to_string()),
        "this_copy": current,
        "target_exists": st.appimage.as_deref().is_some_and(Path::is_file),
        "foreign": st.foreign,
    }))
}
