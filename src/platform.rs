//! Platform layer (ADR 0006). Every OS-specific decision in the shell lives here, so the rest
//! of the UI is written once: shortcut labels (⌘ on macOS), window chrome, the settings folder,
//! native file dialogs and the renderer choice.
//!
//! The web build (ADR 0036) is one more platform: the browser's file picker and downloads take
//! the dialogs' place, its storage the settings folder's, and the glue lives in `web.rs`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Mac,
    Windows,
    Linux,
    /// A web browser (the wasm32 build).
    Web,
    Other,
}

pub const OS: Os = if cfg!(target_os = "macos") {
    Os::Mac
} else if cfg!(target_os = "windows") {
    Os::Windows
} else if cfg!(target_arch = "wasm32") {
    Os::Web
} else if cfg!(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd", target_os = "dragonfly")) {
    Os::Linux
} else {
    Os::Other
};

impl Os {
    pub fn name(self) -> &'static str {
        match self {
            Os::Mac => "macOS",
            Os::Windows => "Windows",
            Os::Linux => "Linux / BSD",
            Os::Web => "web browser",
            Os::Other => "other",
        }
    }
}

/// The web build runs in a browser (ADR 0036).
pub const WEB: bool = cfg!(target_arch = "wasm32");

/// Whether the active fonts can draw ⌘ ⇧ ⌥ ⌃ (set once fonts are installed).
static MAC_GLYPHS: AtomicBool = AtomicBool::new(false);

/// In a browser: whether it runs on a Mac (its command key is ⌘, as egui reads it there).
static WEB_ON_MAC: AtomicBool = AtomicBool::new(false);

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn set_web_on_mac(mac: bool) {
    WEB_ON_MAC.store(mac, Ordering::Relaxed);
}

/// Whose conventions shortcut labels follow: the OS's, or in a browser the computer's under it.
fn keys_os() -> Os {
    if OS == Os::Web && WEB_ON_MAC.load(Ordering::Relaxed) { Os::Mac } else { OS }
}

pub fn set_mac_glyphs(available: bool) {
    MAC_GLYPHS.store(available, Ordering::Relaxed);
}

/// A registry shortcut ("Ctrl+Shift+S") as this platform writes it: unchanged on Windows and
/// Linux; "⇧⌘S" on macOS (or "Shift+Cmd+S" when the font lacks the symbols).
pub fn shortcut_label(s: &str) -> String {
    shortcut_label_for(keys_os(), s, MAC_GLYPHS.load(Ordering::Relaxed))
}

pub fn shortcut_label_for(os: Os, s: &str, glyphs: bool) -> String {
    if os != Os::Mac || s.is_empty() {
        return s.to_string();
    }
    // A trailing "+" key ("Ctrl++") must survive the split.
    let (mods, key) = match s.rsplit_once('+') {
        Some((m, "")) => (m.trim_end_matches('+'), "+"),
        Some((m, k)) => (m, k),
        None => ("", s),
    };
    let has = |m: &str| mods.split('+').any(|x| x == m);
    let key = match (key, glyphs) {
        ("Delete", true) => "⌫",
        ("Enter", true) => "↩",
        ("Up", true) => "↑",
        ("Down", true) => "↓",
        ("Left", true) => "←",
        ("Right", true) => "→",
        (k, _) => k,
    };
    // Apple order: Control, Option, Shift, Command. Our "Ctrl" is the command modifier.
    let mut out = String::new();
    for (name, glyph, text) in [("Alt", "⌥", "Option+"), ("Shift", "⇧", "Shift+"), ("Ctrl", "⌘", "Cmd+")] {
        if has(name) {
            out.push_str(if glyphs { glyph } else { text });
        }
    }
    out.push_str(key);
    out
}

/// Where preferences live: %APPDATA%\AniCraft, ~/Library/Application Support/AniCraft, or
/// $XDG_CONFIG_HOME/anicraft (default ~/.config/anicraft).
pub fn config_dir() -> Option<PathBuf> {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    match OS {
        Os::Windows => env("APPDATA").map(|p| p.join("AniCraft")),
        Os::Mac => env("HOME").map(|p| p.join("Library").join("Application Support").join("AniCraft")),
        // A web page has no folders: settings go to browser storage (`load_settings`).
        Os::Web => None,
        Os::Linux | Os::Other => env("XDG_CONFIG_HOME").or_else(|| env("HOME").map(|h| h.join(".config"))).map(|p| p.join("anicraft")),
    }
}

/// Where crash-recovery copies live (ADR 0015): `Recovery` in the settings folder.
/// `ANICRAFT_RECOVERY_DIR` points it elsewhere (tests, portable installs).
pub fn recovery_dir() -> Option<PathBuf> {
    match std::env::var_os("ANICRAFT_RECOVERY_DIR").filter(|v| !v.is_empty()) {
        Some(d) => Some(PathBuf::from(d)),
        None => config_dir().map(|d| d.join("Recovery")),
    }
}

/// Whether the recovery folder was chosen explicitly (screenshot runs only touch it then).
pub fn recovery_dir_overridden() -> bool {
    std::env::var_os("ANICRAFT_RECOVERY_DIR").is_some_and(|v| !v.is_empty())
}

/// Where settings are kept, for Preferences.
pub fn settings_place() -> String {
    if WEB {
        return "this browser's storage for this site".into();
    }
    config_dir().map_or_else(|| "memory only".to_string(), |d| d.display().to_string())
}

/// The saved preferences (JSON): `prefs.json` in the settings folder, or browser storage.
pub fn load_settings() -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    return crate::web::storage_get(crate::web::PREFS_KEY);
    #[cfg(not(target_arch = "wasm32"))]
    config_dir().and_then(|d| std::fs::read_to_string(d.join("prefs.json")).ok())
}

/// Saves the preferences; on disk atomically (temp file + rename).
pub fn save_settings(json: &str) -> Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    return crate::web::storage_set(crate::web::PREFS_KEY, json);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let dir = config_dir().ok_or("no settings folder on this system")?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let (path, tmp) = (dir.join("prefs.json"), dir.join("prefs.json.tmp"));
        std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
    }
}

/// Why this build can't run `cmd`, if it can't: a web page can't start ffmpeg or keep files in
/// a folder of the computer (ADR 0036). The desktop app runs everything.
pub fn unavailable(cmd: &str) -> Option<&'static str> {
    if !WEB {
        return None;
    }
    match cmd {
        "file.export_video" => Some("Video export runs ffmpeg, which a web browser can't start: use the desktop app, or export an animated GIF or a PNG sequence"),
        "plugin.install" | "plugin.remove" | "plugin.reload" => Some("Plug-ins are installed into a folder on your computer: use the desktop app"),
        _ => None,
    }
}

/// Where installed plug-ins live: a `plugins` folder in the settings folder (ADR 0017).
pub fn plugins_dir() -> Option<PathBuf> {
    config_dir().map(|d| d.join("plugins"))
}

/// The app id, one value everywhere (ADR 0026): the window's Wayland app_id and X11 class, the
/// Linux desktop entry, AppStream and MIME package names, the Flatpak id and the macOS bundle id.
pub const APP_ID: &str = "ai.storyteller.anicraft";

/// Main-window chrome. macOS gets the unified look (content under a transparent title bar,
/// traffic lights over our menu row); Windows and Linux keep their native frames.
#[cfg(not(target_arch = "wasm32"))]
pub fn viewport(b: egui::ViewportBuilder) -> egui::ViewportBuilder {
    // The Wayland app_id / X11 class: docks match it to the installed desktop entry
    // (packaging/linux/ai.storyteller.anicraft.desktop) for the icon and the launcher.
    let b = b.with_app_id(APP_ID).with_icon(app_icon());
    match OS {
        Os::Mac => b.with_fullsize_content_view(true).with_title_shown(false),
        _ => b,
    }
}

/// The window, Dock, taskbar and app-switcher icon (`assets/app-icon/`, see its README). macOS gets
/// the tile on Apple's grid (with its transparent margin); elsewhere the full-bleed tile.
#[cfg(not(target_arch = "wasm32"))]
fn app_icon() -> egui::IconData {
    #[cfg(target_os = "macos")]
    let png: &[u8] = include_bytes!("../assets/app-icon/anicraft-macos-512.png");
    #[cfg(not(target_os = "macos"))]
    let png: &[u8] = include_bytes!("../assets/app-icon/hicolor/256x256/apps/ai.storyteller.anicraft.png");
    eframe::icon_data::from_png_bytes(png).unwrap_or_default()
}

/// Space left of the menu row for the macOS traffic-light buttons.
pub fn menu_inset() -> f32 {
    if OS == Os::Mac { 72.0 } else { 0.0 }
}

/// Renderer requested by `--renderer` / `ANICRAFT_RENDERER` (`wgpu` or `gl`), if any.
#[cfg(not(target_arch = "wasm32"))]
pub fn forced_renderer(arg: Option<&str>) -> Option<eframe::Renderer> {
    let v = arg.map(str::to_owned).or_else(|| std::env::var("ANICRAFT_RENDERER").ok())?;
    match v.to_ascii_lowercase().as_str() {
        "gl" | "glow" | "opengl" => Some(eframe::Renderer::Glow),
        "wgpu" | "vulkan" | "metal" | "dx12" => Some(eframe::Renderer::Wgpu),
        _ => None,
    }
}

// ----- native file dialogs ----------------------------------------------------------------------

/// File type filters per command.
pub fn filters(cmd: &str) -> (&'static str, &'static [&'static str]) {
    match cmd {
        "file.export_png" => ("PNG image", &["png"]),
        "file.export_gif" => ("Animated GIF", &["gif"]),
        "file.export_svg" => ("SVG image", &["svg"]),
        "file.export_lottie" => ("Lottie animation", &["json"]),
        "file.export_html" => ("Web page", &["html", "htm"]),
        "file.import" => ("Drawing, image or sound", &["svg", "png", "jpg", "jpeg", "wav", "ogg", "mp3"]),
        "file.import_to_library" => ("Sound or image", &["wav", "ogg", "mp3", "png", "jpg", "jpeg"]),
        "file.export_audio" => ("WAV sound", &["wav"]),
        "file.export_sprite_sheet" => ("PNG sprite sheet", &["png"]),
        "file.export_video" => ("Video (WebM or MP4)", &["webm", "mp4"]),
        // Opening also imports .fla/.xfl and Moho .moho/.mohoproj documents.
        "file.open" => ("Documents", &["anicraft", "fla", "xfl", "moho", "mohoproj"]),
        "plugin.install" => ("WebAssembly plug-in", &["wasm"]),
        _ => ("AniCraft document", &["anicraft"]),
    }
}

/// Commands that read a file the user picks (the others write one, or a folder of them).
pub fn reads_file(cmd: &str) -> bool {
    matches!(cmd, "file.open" | "plugin.install" | "file.import" | "file.import_to_library")
}

/// Shows the platform's own dialog (Windows common dialog, macOS panel, Linux XDG portal).
/// `None` when cancelled.
#[cfg(not(target_arch = "wasm32"))]
pub fn file_dialog(cmd: &str, title: &str, default_name: &str) -> Option<PathBuf> {
    let (label, exts) = filters(cmd);
    let d = rfd::FileDialog::new().set_title(title);
    match cmd {
        c if reads_file(c) => d.add_filter(label, exts).pick_file(),
        "file.export_png_sequence" => d.pick_folder(),
        _ => d.add_filter(label, exts).set_file_name(default_name).save_file(),
    }
}

/// In a browser: asks for a file with the browser's file picker. It arrives later through
/// [`picked_file`]; nothing arrives if the user cancels. The desktop has [`file_dialog`].
#[cfg_attr(not(target_arch = "wasm32"), allow(unused_variables, dead_code))]
pub fn pick_file(cmd: &'static str) {
    #[cfg(target_arch = "wasm32")]
    crate::web::pick_file(cmd, filters(cmd).1);
}

/// A file the user picked in the browser: the command that asked for it, its name, and its
/// bytes (or why the browser couldn't read them).
pub struct Picked {
    pub cmd: &'static str,
    pub name: String,
    pub data: Result<Vec<u8>, String>,
}

/// The next file the browser handed over, if one arrived.
pub fn picked_file() -> Option<Picked> {
    #[cfg(target_arch = "wasm32")]
    return crate::web::take_picked();
    #[cfg(not(target_arch = "wasm32"))]
    None
}

/// In a browser: offers each file a command saved or exported as a download.
#[cfg_attr(not(target_arch = "wasm32"), allow(unused_variables))]
pub fn deliver(files: Vec<(PathBuf, Vec<u8>)>) {
    #[cfg(target_arch = "wasm32")]
    for (path, data) in files {
        crate::web::download(&path, &data);
    }
}

/// In a browser: whether leaving the page asks first (there are unsaved changes).
#[cfg_attr(not(target_arch = "wasm32"), allow(unused_variables))]
pub fn set_unsaved(unsaved: bool) {
    #[cfg(target_arch = "wasm32")]
    crate::web::set_unsaved(unsaved);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_desktop_runs_every_command() {
        assert!(anicraft_engine::commands().iter().all(|c| unavailable(c.id).is_none()));
        assert!(reads_file("file.open") && reads_file("file.import") && !reads_file("file.save"));
    }

    #[test]
    fn mac_labels_use_apple_order_and_glyphs() {
        assert_eq!(shortcut_label_for(Os::Mac, "Ctrl+Shift+S", true), "⇧⌘S");
        assert_eq!(shortcut_label_for(Os::Mac, "Ctrl+Alt+1", true), "⌥⌘1");
        assert_eq!(shortcut_label_for(Os::Mac, "Ctrl+Shift+S", false), "Shift+Cmd+S");
        assert_eq!(shortcut_label_for(Os::Mac, "Delete", true), "⌫");
        assert_eq!(shortcut_label_for(Os::Mac, "F6", true), "F6");
        assert_eq!(shortcut_label_for(Os::Mac, "Ctrl++", true), "⌘+");
    }

    #[test]
    fn other_platforms_keep_registry_labels() {
        for os in [Os::Windows, Os::Linux] {
            assert_eq!(shortcut_label_for(os, "Ctrl+Shift+Z", true), "Ctrl+Shift+Z");
        }
    }

    #[test]
    fn renderer_override() {
        assert_eq!(forced_renderer(Some("gl")), Some(eframe::Renderer::Glow));
        assert_eq!(forced_renderer(Some("WGPU")), Some(eframe::Renderer::Wgpu));
    }
}
