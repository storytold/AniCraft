//! User preferences: appearance (fonts, UI scale), stage rendering, file dialogs. Stored as
//! JSON in the platform's settings folder (`platform::config_dir`). A missing or broken file
//! falls back to defaults; it never stops the app from starting.

use serde::{Deserialize, Serialize};

/// Which typeface a text role uses.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FontChoice {
    /// The fonts AniCraft bundles (Inter / JetBrains Mono, or egui's defaults).
    #[default]
    Bundled,
    /// A font family installed on this computer.
    System(String),
}

/// When the stage renders edits (ADR 0006).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RenderMode {
    /// Edits render immediately while frames are cheap, in the background when the scene is
    /// heavy (with an outline preview of what you drag).
    #[default]
    Auto,
    /// Always render edits immediately (lowest latency, may stutter on heavy scenes).
    Immediate,
    /// Always render in the background (UI never waits; picture can trail by a frame).
    Background,
}

impl RenderMode {
    pub const ALL: [(RenderMode, &'static str, &'static str); 3] = [
        (RenderMode::Auto, "Auto", "Edits show immediately; heavy scenes render in the background with an outline preview."),
        (RenderMode::Immediate, "Immediate", "Every edit renders before the next frame. Lowest latency; can stutter on very heavy scenes."),
        (RenderMode::Background, "Background", "Always render off the UI thread. The window never waits; the picture can trail by a frame."),
    ];
}

/// Which renderer draws the stage (ADR 0019). The GPU only takes part in builds with the
/// `gpu-stage` feature; elsewhere the setting is kept but the CPU draws.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum StageRenderer {
    /// The reference renderer: exact, and what every export uses.
    #[default]
    Cpu,
    /// The graphics card, for what it draws today (the CPU draws the rest).
    Gpu,
}

impl StageRenderer {
    pub const ALL: [(StageRenderer, &'static str, &'static str); 2] = [
        (StageRenderer::Cpu, "CPU (reference)", "The reference renderer. Exports always use it."),
        (StageRenderer::Gpu, "GPU (preview)", "Draws the stage on the graphics card. Masks, symbol opacity, blend modes, filters and bitmaps still draw on the CPU; the picture is the same either way."),
    ];
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub ui_font: FontChoice,
    pub data_font: FontChoice,
    /// Graphite or high contrast (ADR 0022).
    pub theme: crate::theme::Palette,
    /// Interface scale (1.0 = 100%).
    pub ui_scale: f32,
    pub render_mode: RenderMode,
    pub stage_renderer: StageRenderer,
    /// Native open/save dialogs; off = a built-in path field (for Linux setups without an XDG
    /// desktop portal).
    pub native_dialogs: bool,
    /// Extra space before frame 1 on the timeline, in points, on top of the room the playhead
    /// needs (the playhead can't go there). Fixed on screen, whatever the zoom.
    pub timeline_start_gap: f32,
    /// Keyboard shortcuts the user changed: action id → its full list of keys (empty =
    /// unbound). Actions not listed keep their defaults (`keymap`).
    pub shortcuts: std::collections::BTreeMap<String, Vec<String>>,
    /// Interface language: a craft-i18n code (`en`, `ar`), or `auto` for the system's (`i18n`).
    pub language: String,
    /// Keep a crash-recovery copy of unsaved work (ADR 0015).
    pub autosave: bool,
    /// Longest time unsaved work goes without a recovery copy, in seconds.
    pub autosave_secs: u32,
    /// Also make a copy after this many changes (0: by time only).
    pub autosave_edits: u32,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            ui_font: FontChoice::Bundled,
            data_font: FontChoice::Bundled,
            theme: crate::theme::Palette::Graphite,
            ui_scale: 1.0,
            render_mode: RenderMode::Auto,
            stage_renderer: StageRenderer::Cpu,
            native_dialogs: true,
            timeline_start_gap: 0.0,
            shortcuts: Default::default(),
            language: crate::i18n::AUTO.to_string(),
            autosave: true,
            autosave_secs: 60,
            autosave_edits: 25,
        }
    }
}

/// Autosave intervals offered in Preferences: `(seconds, label)`.
pub const AUTOSAVE_INTERVALS: [(u32, &str); 5] = [(30, "30 seconds"), (60, "1 minute"), (120, "2 minutes"), (300, "5 minutes"), (600, "10 minutes")];
pub const AUTOSAVE_SECS_RANGE: std::ops::RangeInclusive<u32> = 15..=600;
pub const AUTOSAVE_EDITS_RANGE: std::ops::RangeInclusive<u32> = 0..=1000;

pub const SCALE_RANGE: std::ops::RangeInclusive<f32> = 0.75..=2.0;
pub const GAP_RANGE: std::ops::RangeInclusive<f32> = 0.0..=48.0;

impl Prefs {
    fn path() -> Option<std::path::PathBuf> {
        crate::platform::config_dir().map(|d| d.join("prefs.json"))
    }

    pub fn sanitized(mut self) -> Prefs {
        self.ui_scale = if self.ui_scale.is_finite() { self.ui_scale.clamp(*SCALE_RANGE.start(), *SCALE_RANGE.end()) } else { 1.0 };
        self.timeline_start_gap = if self.timeline_start_gap.is_finite() { self.timeline_start_gap.clamp(*GAP_RANGE.start(), *GAP_RANGE.end()) } else { 0.0 };
        for f in [&mut self.ui_font, &mut self.data_font] {
            if let FontChoice::System(name) = f
                && (name.trim().is_empty() || name.len() > 256)
            {
                *f = FontChoice::Bundled;
            }
        }
        self.shortcuts = crate::keymap::sanitize(std::mem::take(&mut self.shortcuts));
        // A code is short; anything else came from a broken file. An unknown code is kept (a newer
        // version's language) and shows the system's language meanwhile.
        if self.language.trim().is_empty() || self.language.len() > 32 {
            self.language = crate::i18n::AUTO.to_string();
        }
        self.autosave_secs = self.autosave_secs.clamp(*AUTOSAVE_SECS_RANGE.start(), *AUTOSAVE_SECS_RANGE.end());
        self.autosave_edits = self.autosave_edits.min(*AUTOSAVE_EDITS_RANGE.end());
        self
    }

    pub fn from_json(text: &str) -> Prefs {
        serde_json::from_str::<Prefs>(text).unwrap_or_default().sanitized()
    }

    pub fn load() -> Prefs {
        Self::path().and_then(|p| std::fs::read_to_string(p).ok()).map_or_else(Prefs::default, |t| Prefs::from_json(&t))
    }

    /// Writes atomically (temp file + rename).
    pub fn save(&self) -> Result<(), String> {
        let path = Self::path().ok_or("no settings folder on this system")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
    }
}

// ----- system fonts -----------------------------------------------------------------------------

/// Font families installed on this computer (scanned once, off the UI thread).
pub struct SystemFonts {
    db: fontdb::Database,
    pub families: Vec<String>,
    pub monospace: Vec<String>,
}

impl SystemFonts {
    /// Scans the OS font folders (Windows Fonts, macOS Library/Fonts, fontconfig dirs on Linux).
    pub fn scan() -> SystemFonts {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        let mut families = std::collections::BTreeSet::new();
        let mut monospace = std::collections::BTreeSet::new();
        for face in db.faces() {
            if let Some((name, _)) = face.families.first() {
                if name.starts_with('.') {
                    continue; // hidden system UI faces on macOS
                }
                families.insert(name.clone());
                if face.monospaced {
                    monospace.insert(name.clone());
                }
            }
        }
        SystemFonts { db, families: families.into_iter().collect(), monospace: monospace.into_iter().collect() }
    }

    /// Bytes and face index of the regular style of `family`.
    pub fn regular(&self, family: &str) -> Option<(Vec<u8>, u32)> {
        let q = fontdb::Query { families: &[fontdb::Family::Name(family)], weight: fontdb::Weight::NORMAL, stretch: fontdb::Stretch::Normal, style: fontdb::Style::Normal };
        let id = self.db.query(&q)?;
        self.db.with_face_data(id, |data, index| (data.to_vec(), index))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_font_scan_is_safe_and_loads_a_face() {
        // Machine-dependent (CI containers may have no fonts); must never panic.
        let fonts = SystemFonts::scan();
        eprintln!("{} families, {} monospace", fonts.families.len(), fonts.monospace.len());
        assert!(fonts.monospace.iter().all(|m| fonts.families.contains(m)));
        if let Some(first) = fonts.families.first() {
            assert!(fonts.regular(first).is_some_and(|(bytes, _)| !bytes.is_empty()), "{first}");
        }
        assert!(fonts.regular("No Such Font Family 12345").is_none());
    }

    #[test]
    fn broken_or_hostile_files_fall_back_to_defaults() {
        assert_eq!(Prefs::from_json("not json"), Prefs::default());
        let p = Prefs::from_json(r#"{"ui_scale": 99, "ui_font": {"System": ""}, "unknown": 1}"#);
        assert_eq!(p.ui_scale, 2.0);
        assert_eq!(p.ui_font, FontChoice::Bundled);
        let p = Prefs::from_json(r#"{"render_mode": "Background", "data_font": {"System": "Consolas"}}"#);
        assert_eq!(p.render_mode, RenderMode::Background);
        assert_eq!(p.data_font, FontChoice::System("Consolas".into()));
        assert!(p.native_dialogs, "missing fields keep their defaults");
        assert_eq!(p.stage_renderer, StageRenderer::Cpu, "files from before the GPU stage draw on the CPU");
        assert_eq!(Prefs::from_json(r#"{"stage_renderer": "Gpu"}"#).stage_renderer, StageRenderer::Gpu);        let p = Prefs::from_json(r#"{"shortcuts": {"tool.pen": ["F9", "Ctrl+K"], "nope": ["F2"]}}"#);
        assert_eq!(p.shortcuts.len(), 1);
        assert_eq!(p.shortcuts.get("tool.pen"), Some(&vec!["F9".to_string()]));
        // Older preference files have no autosave settings: autosave is on, every minute.
        let p = Prefs::from_json(r#"{"ui_scale": 1.25}"#);
        assert!(p.autosave);
        assert_eq!((p.autosave_secs, p.autosave_edits), (60, 25));
        let p = Prefs::from_json(r#"{"autosave": false, "autosave_secs": 1, "autosave_edits": 99999999}"#);
        assert!(!p.autosave);
        assert_eq!((p.autosave_secs, p.autosave_edits), (15, 1000));
    }
}
