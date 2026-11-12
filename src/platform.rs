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
    /// The platform's name in the interface language (Preferences shows it).
    pub fn name(self) -> &'static str {
        match self {
            Os::Mac => tr!("macOS"),
            Os::Windows => tr!("Windows"),
            Os::Linux => tr!("Linux / BSD"),
            Os::Web => tr!("web browser"),
            Os::Other => tr!("other"),
        }
    }
}

/// The web build runs in a browser (ADR 0036).
pub const WEB: bool = cfg!(target_arch = "wasm32");

/// The language the system's interface is in, as a BCP 47 tag (`de-DE`, `pt-BR`): Windows'
/// user locale, macOS' preferred languages, `LC_ALL` / `LC_MESSAGES` / `LANG` on Linux and the
/// BSDs, the browser's languages on the web (ADR 0040). `None` when it can't be told.
pub fn system_locale() -> Option<String> {
    sys_locale::get_locale().filter(|l| !l.is_empty() && l.len() <= 64)
}

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
        Os::Mac => env("HOME").map(|p| p.join("Library").join("Application Support").join("AniCraft")), // i18n-exempt: folder names
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
        None => config_dir().map(|d| d.join("Recovery")), // i18n-exempt: a folder name
    }
}

/// Whether the recovery folder was chosen explicitly (screenshot runs only touch it then).
pub fn recovery_dir_overridden() -> bool {
    std::env::var_os("ANICRAFT_RECOVERY_DIR").is_some_and(|v| !v.is_empty())
}

/// Where settings are kept, for Preferences (in the interface language).
pub fn settings_place() -> String {
    if WEB {
        return tr!("this browser's storage for this site").into();
    }
    config_dir().map_or_else(|| tr!("memory only").to_string(), |d| d.display().to_string())
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
        let dir = config_dir().ok_or(tr!("no settings folder on this system"))?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let (path, tmp) = (dir.join("prefs.json"), dir.join("prefs.json.tmp"));
        std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
    }
}

/// A recent file's start-screen picture (ADR 0059): `Thumbnails/<key>.png` in the settings
/// folder, or browser storage on the web. `None` when there is none yet.
pub fn load_thumbnail(key: &str) -> Option<Vec<u8>> {
    #[cfg(target_arch = "wasm32")]
    return crate::web::storage_get(&format!("anicraft.thumb.{key}")).and_then(|t| anicraft_engine::doc::base64_decode(&t));
    #[cfg(not(target_arch = "wasm32"))]
    config_dir().and_then(|d| std::fs::read(d.join("Thumbnails").join(format!("{key}.png"))).ok()) // i18n-exempt: a folder name
}

/// Keeps a recent file's picture (a small PNG) for the start screen.
pub fn save_thumbnail(key: &str, png: &[u8]) -> Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    return crate::web::storage_set(&format!("anicraft.thumb.{key}"), &anicraft_engine::doc::base64_encode(png));
    #[cfg(not(target_arch = "wasm32"))]
    {
        let dir = config_dir().ok_or(tr!("no settings folder on this system"))?.join("Thumbnails"); // i18n-exempt: a folder name
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let (path, tmp) = (dir.join(format!("{key}.png")), dir.join(format!("{key}.png.tmp")));
        std::fs::write(&tmp, png).map_err(|e| e.to_string())?;
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
        "file.export_video" => Some(tr_noop!("Video export runs ffmpeg, which a web browser can't start: use the desktop app, or export an animated GIF or a PNG sequence")),
        "plugin.install" | "plugin.remove" | "plugin.reload" => Some(tr_noop!("Plug-ins are installed into a folder on your computer: use the desktop app")),
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

/// Another window of the app (Test Movie, ADR 0062): the same app id, so docks and taskbars group
/// it with the editor, and the same icon. Native frames everywhere (the editor's unified macOS
/// title bar is for its menu row, which this window doesn't have).
pub fn secondary_window(b: egui::ViewportBuilder) -> egui::ViewportBuilder {
    #[cfg(not(target_arch = "wasm32"))]
    return b.with_app_id(APP_ID).with_icon(app_icon());
    #[cfg(target_arch = "wasm32")]
    b
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

// ----- more than one window (ADR 0062) ---------------------------------------------------------

/// Where Test Movie plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MovieHost {
    /// A window of its own on the desktop (an egui immediate viewport).
    Window,
    /// A floating window inside the editor, and why.
    Inside(Inside),
}

/// Why Test Movie plays inside the editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inside {
    /// Preferences › Test Movie says so.
    Chosen,
    /// A web page has one window (ADR 0036).
    WebPage,
    /// The windowing layer can't open another window here.
    NoWindows,
}

impl Inside {
    /// What the floating window says about it (English: translated where it is shown); nothing
    /// when the user chose it.
    pub fn note(self) -> Option<&'static str> {
        match self {
            Inside::Chosen => None,
            Inside::WebPage => Some(tr_noop!("Inside the editor: a web page can't open a window of its own")),
            Inside::NoWindows => Some(tr_noop!("Inside the editor: this system can't open another window for it")),
        }
    }
}

/// Where Test Movie plays, given whether egui embeds viewports (eframe sets that on the web and
/// wherever it has no multi-window support) and the user's choice.
pub fn movie_host(embed_viewports: bool, own_window: bool) -> MovieHost {
    movie_host_for(OS, embed_viewports, own_window)
}

pub fn movie_host_for(os: Os, embed_viewports: bool, own_window: bool) -> MovieHost {
    if os == Os::Web {
        MovieHost::Inside(Inside::WebPage)
    } else if embed_viewports {
        MovieHost::Inside(Inside::NoWindows)
    } else if !own_window {
        MovieHost::Inside(Inside::Chosen)
    } else {
        MovieHost::Window
    }
}

/// Whether AniCraft can put a window back where it was. Wayland never tells an app where its
/// windows are, nor lets it place them: the desktop does.
pub fn window_positions() -> bool {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    window_positions_for(OS, env("WAYLAND_DISPLAY").is_some(), env("WINIT_UNIX_BACKEND").as_deref())
}

/// winit picks Wayland when `WAYLAND_DISPLAY` is set, unless `WINIT_UNIX_BACKEND=x11`.
pub fn window_positions_for(os: Os, wayland_display: bool, winit_backend: Option<&str>) -> bool {
    match os {
        Os::Linux | Os::Other => !wayland_display || winit_backend.is_some_and(|b| b.eq_ignore_ascii_case("x11")),
        Os::Web => false,
        Os::Mac | Os::Windows => true,
    }
}

/// A remembered window corner, if it can still be used: finite, and on or beside the screen the
/// editor is on (a monitor to its left or right is fine; a window lost far off-screen isn't).
pub fn usable_position(at: [f32; 2], monitor: Option<egui::Vec2>) -> Option<egui::Pos2> {
    let [x, y] = at;
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    let m = monitor.filter(|m| m.x > 0.0 && m.y > 0.0).unwrap_or(egui::vec2(16384.0, 16384.0));
    let ok = x >= -m.x && x <= 2.0 * m.x - 64.0 && y >= -m.y && y <= 2.0 * m.y - 64.0;
    ok.then(|| egui::pos2(x, y))
}

// ----- documents the system asks AniCraft to open (ADR 0062) ------------------------------------

/// Something the system asked of the running app, outside its windows.
// Only macOS hands documents to a running app; elsewhere the tests make requests.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SystemRequest {
    /// Open these files (macOS: a double-click in Finder, a drop on the Dock icon, Open With).
    Open(Vec<PathBuf>),
    /// Quit (macOS: the Dock's Quit, logging out): through the unsaved-changes guard.
    Quit,
}

thread_local! {
    /// Requests not yet handled. Filled on the main thread (AppKit calls back there), read by
    /// the frame on the same thread.
    static REQUESTS: std::cell::RefCell<Vec<SystemRequest>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Most requests kept between two frames (a flood of Apple Events can't grow memory).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const MAX_REQUESTS: usize = 64;

/// Queues a request for the next frame.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn push_request(r: SystemRequest) {
    REQUESTS.with(|q| {
        let mut q = q.borrow_mut();
        if q.len() < MAX_REQUESTS {
            q.push(r);
        }
    });
}

/// The requests that arrived since the last frame, in order.
pub fn take_requests() -> Vec<SystemRequest> {
    #[cfg(target_os = "macos")]
    MAC_INBOX.with(|i| {
        if let Some(inbox) = i.borrow().as_ref() {
            for e in inbox.drain() {
                push_request(match e {
                    fmv_macos_events::Event::Open(paths) => SystemRequest::Open(paths),
                    fmv_macos_events::Event::Quit => SystemRequest::Quit,
                });
            }
        }
    });
    REQUESTS.with(|q| std::mem::take(&mut *q.borrow_mut()))
}

#[cfg(target_os = "macos")]
thread_local! {
    static MAC_INBOX: std::cell::RefCell<Option<fmv_macos_events::Inbox>> = const { std::cell::RefCell::new(None) };
}

/// Listens for documents the system hands over while AniCraft runs. Keep the result until the
/// event loop returns. On macOS it must start before the event loop does, so the document that
/// launched the app (sent as an Apple Event, not on the command line) isn't missed. Windows and
/// Linux pass that document on the command line (`main`), and a second document starts a second
/// AniCraft, so there is nothing to listen for there.
pub struct DocumentListener {
    #[cfg(target_os = "macos")]
    _registration: fmv_macos_events::Registration,
}

#[cfg(not(target_arch = "wasm32"))]
pub fn listen_for_documents() -> DocumentListener {
    #[cfg(target_os = "macos")]
    {
        let (registration, inbox) = fmv_macos_events::Registration::install();
        MAC_INBOX.with(|i| *i.borrow_mut() = Some(inbox));
        DocumentListener { _registration: registration }
    }
    #[cfg(not(target_os = "macos"))]
    DocumentListener {}
}

/// A request that arrives while the app is idle wakes it.
#[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
pub fn wake_on_requests(ctx: &egui::Context) {
    #[cfg(target_os = "macos")]
    MAC_INBOX.with(|i| {
        if let Some(inbox) = i.borrow().as_ref() {
            let ctx = ctx.clone();
            inbox.set_wake(move || ctx.request_repaint());
        }
    });
}

/// Arguments the system adds that aren't files: macOS before 10.9 passed `-psn_0_12345` (the
/// process serial number) to apps started from Finder.
pub fn is_system_argument(arg: &str) -> bool {
    arg.starts_with("-psn_")
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

/// File type filters per command: the filter's name (English, translated where it is shown)
/// and its extensions.
pub fn filters(cmd: &str) -> (&'static str, &'static [&'static str]) {
    match cmd {
        "file.export_png" => (tr_noop!("PNG image"), &["png"]),
        "file.export_gif" => (tr_noop!("Animated GIF"), &["gif"]),
        "file.export_apng" => (tr_noop!("Animated PNG"), &["png", "apng"]),
        "file.export_webp" => (tr_noop!("Animated WebP"), &["webp"]),
        "file.export_svg" => (tr_noop!("SVG image"), &["svg"]),
        "file.export_lottie" => (tr_noop!("Lottie animation"), &["json"]),
        "file.export_html" => (tr_noop!("Web page"), &["html", "htm"]),
        "file.import" => (tr_noop!("Drawing, image or sound"), &["svg", "png", "jpg", "jpeg", "wav", "ogg", "mp3"]),
        "file.import_to_library" => (tr_noop!("Sound or image"), &["wav", "ogg", "mp3", "png", "jpg", "jpeg"]),
        "file.export_audio" => (tr_noop!("WAV sound"), &["wav"]),
        "file.export_sprite_sheet" => (tr_noop!("PNG sprite sheet"), &["png"]),
        "file.export_video" => (tr_noop!("Video (WebM or MP4)"), &["webm", "mp4"]),
        // Opening also imports .fla archives and .xfl packages.
        "file.open" => (tr_noop!("Documents"), &["anicraft", "fla", "xfl"]),
        "plugin.install" => (tr_noop!("WebAssembly plug-in"), &["wasm"]),
        _ => (tr_noop!("AniCraft document"), &["anicraft"]),
    }
}

/// Commands that read a file the user picks (the others write one, or a folder of them).
pub fn reads_file(cmd: &str) -> bool {
    matches!(cmd, "file.open" | "plugin.install" | "file.import" | "file.import_to_library")
}

/// Shows the platform's own dialog (Windows common dialog, macOS panel, Linux XDG portal), with
/// `title` already in the interface language. `None` when cancelled.
#[cfg(not(target_arch = "wasm32"))]
pub fn file_dialog(cmd: &str, title: &str, default_name: &str) -> Option<PathBuf> {
    let (label, exts) = filters(cmd);
    let (label, d) = (crate::i18n::t(label), rfd::FileDialog::new().set_title(title));
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

    /// Test Movie gets a window of its own wherever the windowing layer can make one and the user
    /// wants it; elsewhere it floats inside the editor and says why (ADR 0062).
    #[test]
    fn test_movie_falls_back_inside_the_editor_and_says_why() {
        for os in [Os::Windows, Os::Mac, Os::Linux] {
            assert_eq!(movie_host_for(os, false, true), MovieHost::Window, "{os:?}");
            assert_eq!(movie_host_for(os, false, false), MovieHost::Inside(Inside::Chosen));
            assert_eq!(movie_host_for(os, true, true), MovieHost::Inside(Inside::NoWindows));
        }
        // A web page never has a second window, whatever egui or the preference say.
        assert_eq!(movie_host_for(Os::Web, false, true), MovieHost::Inside(Inside::WebPage));
        assert!(Inside::Chosen.note().is_none(), "the user's own choice needs no explaining");
        assert!(Inside::WebPage.note().is_some_and(|n| n.contains("web page")) && Inside::NoWindows.note().is_some());
        // eframe embeds viewports where it can't open windows (the web, a headless context).
        assert!(egui::Context::default().embed_viewports());
    }

    #[test]
    fn wayland_hides_window_positions() {
        assert!(window_positions_for(Os::Windows, false, None) && window_positions_for(Os::Mac, true, None));
        assert!(window_positions_for(Os::Linux, false, None), "X11");
        assert!(!window_positions_for(Os::Linux, true, None), "Wayland");
        assert!(window_positions_for(Os::Linux, true, Some("X11")), "XWayland, chosen");
        assert!(!window_positions_for(Os::Web, false, None));
    }

    #[test]
    fn a_remembered_place_is_used_only_where_the_window_can_be_seen() {
        let monitor = Some(egui::vec2(1920.0, 1080.0));
        assert_eq!(usable_position([100.0, 80.0], monitor), Some(egui::pos2(100.0, 80.0)));
        assert!(usable_position([-1500.0, 100.0], monitor).is_some(), "a monitor to the left");
        assert!(usable_position([2500.0, 100.0], monitor).is_some(), "a monitor to the right");
        assert_eq!(usable_position([90_000.0, 0.0], monitor), None, "lost far off-screen");
        assert_eq!(usable_position([f32::NAN, 0.0], monitor), None);
        assert!(usable_position([3000.0, 10.0], None).is_some(), "without the screen's size, a broad check");
    }

    #[test]
    fn system_requests_arrive_in_order_and_are_bounded() {
        assert!(take_requests().is_empty());
        push_request(SystemRequest::Open(vec![PathBuf::from("a.anicraft")]));
        push_request(SystemRequest::Quit);
        assert_eq!(take_requests(), vec![SystemRequest::Open(vec![PathBuf::from("a.anicraft")]), SystemRequest::Quit]);
        assert!(take_requests().is_empty(), "each is handled once");
        for _ in 0..1000 {
            push_request(SystemRequest::Quit);
        }
        assert_eq!(take_requests().len(), MAX_REQUESTS);
        assert!(is_system_argument("-psn_0_8194") && !is_system_argument("walk.anicraft"));
    }

    #[test]
    fn renderer_override() {
        assert_eq!(forced_renderer(Some("gl")), Some(eframe::Renderer::Glow));
        assert_eq!(forced_renderer(Some("WGPU")), Some(eframe::Renderer::Wgpu));
    }
}
