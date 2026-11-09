//! Platform layer (ADR 0006). Every OS-specific decision in the shell lives here, so the rest
//! of the UI is written once: shortcut labels (⌘ on macOS), window chrome, the settings folder,
//! native file dialogs and the renderer choice.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Mac,
    Windows,
    Linux,
    Other,
}

pub const OS: Os = if cfg!(target_os = "macos") {
    Os::Mac
} else if cfg!(target_os = "windows") {
    Os::Windows
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
            Os::Other => "other",
        }
    }
}

/// Whether the active fonts can draw ⌘ ⇧ ⌥ ⌃ (set once fonts are installed).
static MAC_GLYPHS: AtomicBool = AtomicBool::new(false);

pub fn set_mac_glyphs(available: bool) {
    MAC_GLYPHS.store(available, Ordering::Relaxed);
}

/// A registry shortcut ("Ctrl+Shift+S") as this platform writes it: unchanged on Windows and
/// Linux; "⇧⌘S" on macOS (or "Shift+Cmd+S" when the font lacks the symbols).
pub fn shortcut_label(s: &str) -> String {
    shortcut_label_for(OS, s, MAC_GLYPHS.load(Ordering::Relaxed))
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
        Os::Linux | Os::Other => env("XDG_CONFIG_HOME").or_else(|| env("HOME").map(|h| h.join(".config"))).map(|p| p.join("anicraft")),
    }
}

/// Main-window chrome. macOS gets the unified look (content under a transparent title bar,
/// traffic lights over our menu row); Windows and Linux keep their native frames.
pub fn viewport(b: egui::ViewportBuilder) -> egui::ViewportBuilder {
    let b = b.with_app_id("anicraft").with_icon(app_icon()); // app ID: Wayland/X11 window class, for docks and window rules
    match OS {
        Os::Mac => b.with_fullsize_content_view(true).with_title_shown(false),
        _ => b,
    }
}

/// The window, Dock, taskbar and app-switcher icon (`assets/app-icon/`, see its README). macOS gets
/// the tile on Apple's grid (with its transparent margin); elsewhere the full-bleed tile.
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
        "file.import" => ("SVG drawing or image", &["svg", "png", "jpg", "jpeg"]),
        "file.export_sprite_sheet" => ("PNG sprite sheet", &["png"]),
        "file.export_video" => ("Video (WebM or MP4)", &["webm", "mp4"]),
        // Opening also imports .fla archives and .xfl packages.
        "file.open" => ("Documents", &["anicraft", "fla", "xfl"]),
        _ => ("AniCraft document", &["anicraft"]),
    }
}

/// Shows the platform's own dialog (Windows common dialog, macOS panel, Linux XDG portal).
/// `None` when cancelled.
pub fn file_dialog(cmd: &str, title: &str, default_name: &str) -> Option<PathBuf> {
    let (label, exts) = filters(cmd);
    let label = crate::i18n::t(label);
    let d = rfd::FileDialog::new().set_title(title);
    match cmd {
        "file.open" => d.add_filter(label, exts).pick_file(),
        "file.export_png_sequence" => d.pick_folder(),
        _ => d.add_filter(label, exts).set_file_name(default_name).save_file(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
