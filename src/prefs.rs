//! User preferences: appearance (fonts, UI scale), stage rendering, file dialogs. Stored as
//! JSON in the platform's settings folder (`platform::load_settings`; browser storage on the web). A missing or broken file
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
        (RenderMode::Auto, tr_noop!("Auto"), tr_noop!("Edits show immediately; heavy scenes render in the background with an outline preview.")),
        (RenderMode::Immediate, tr_noop!("Immediate"), tr_noop!("Every edit renders before the next frame. Lowest latency; can stutter on very heavy scenes.")),
        (RenderMode::Background, tr_noop!("Background"), tr_noop!("Always render off the UI thread. The window never waits; the picture can trail by a frame.")),
    ];
}

/// Which renderer draws the stage (ADR 0019, ADR 0032). The GPU takes part in builds with the
/// `gpu-stage` feature (on by default) on a computer with a usable graphics card; elsewhere
/// the setting is kept but the CPU draws.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum StageRenderer {
    /// The graphics card, the CPU drawing whatever it can't (the default, ADR 0032).
    #[default]
    Gpu,
    /// The reference renderer: exact, and what every export uses.
    Cpu,
}

impl StageRenderer {
    pub const ALL: [(StageRenderer, &'static str, &'static str); 2] = [
        (StageRenderer::Gpu, tr_noop!("Graphics card (GPU)"), tr_noop!("Draws the stage on the graphics card, and on the CPU when there is no card that can. The picture is the same either way.")),
        (StageRenderer::Cpu, tr_noop!("CPU (reference)"), tr_noop!("The reference renderer: slower on large stages. Exports always use it.")),
    ];
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// The interface language (ADR 0040): a craft-i18n locale code (`de`, `pt-br`), or
    /// `auto` to follow the system's. A code this build has no catalog for follows the system
    /// too, and is kept, so a newer build that has it shows it again.
    pub language: String,
    pub ui_font: FontChoice,
    pub data_font: FontChoice,
    /// Graphite or high contrast (ADR 0022).
    pub theme: crate::theme::Palette,
    /// Interface scale (1.0 = 100%).
    pub ui_scale: f32,
    pub render_mode: RenderMode,
    /// Stored as `stage` since the GPU became the default (ADR 0032): every build before wrote
    /// the old `stage_renderer` key as `Cpu` without anyone choosing it (no released build had
    /// the choice), so that key is not read.
    #[serde(rename = "stage")]
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
    /// Keep a crash-recovery copy of unsaved work (ADR 0015).
    pub autosave: bool,
    /// Longest time unsaved work goes without a recovery copy, in seconds.
    pub autosave_secs: u32,
    /// Also make a copy after this many changes (0: by time only).
    pub autosave_edits: u32,
    /// Onion skin options (ADR 0047), kept from session to session.
    pub onion: OnionPrefs,
}

/// Onion skin options that carry over between sessions (ADR 0047). Whether onion skin and Edit
/// Multiple Frames are on, and anchored markers, last for the session only, as in Animate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OnionPrefs {
    /// Frames before and after the playhead the markers reach (unless Onion All).
    pub before: u32,
    pub after: u32,
    /// Onion All: the markers span the whole timeline.
    pub all: bool,
    pub tint: bool,
    pub fade: bool,
    pub outlines: bool,
    pub keyframes_only: bool,
    /// The markers show on the frame ruler even while onion skin is off.
    pub always_markers: bool,
    /// The past and future colours the user picked (sRGB); `None`: the theme's.
    pub past: Option<[u8; 3]>,
    pub future: Option<[u8; 3]>,
}

/// Most frames a marker reaches from the playhead in the preferences (the timeline's own
/// limit is far beyond what anyone onion-skins).
pub const ONION_REACH_MAX: u32 = 999;

impl Default for OnionPrefs {
    fn default() -> Self {
        OnionPrefs { before: 2, after: 2, all: false, tint: true, fade: true, outlines: false, keyframes_only: false, always_markers: false, past: None, future: None }
    }
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            language: crate::i18n::AUTO.to_string(),
            ui_font: FontChoice::Bundled,
            data_font: FontChoice::Bundled,
            theme: crate::theme::Palette::Graphite,
            ui_scale: 1.0,
            render_mode: RenderMode::Auto,
            stage_renderer: StageRenderer::Gpu,
            native_dialogs: true,
            timeline_start_gap: 0.0,
            shortcuts: Default::default(),
            autosave: true,
            autosave_secs: 60,
            autosave_edits: 25,
            onion: OnionPrefs::default(),
        }
    }
}

/// Autosave intervals offered in Preferences: `(seconds, label)`.
pub const AUTOSAVE_INTERVALS: [(u32, &str); 5] = [(30, tr_noop!("30 seconds")), (60, tr_noop!("1 minute")), (120, tr_noop!("2 minutes")), (300, tr_noop!("5 minutes")), (600, tr_noop!("10 minutes"))];
pub const AUTOSAVE_SECS_RANGE: std::ops::RangeInclusive<u32> = 15..=600;
pub const AUTOSAVE_EDITS_RANGE: std::ops::RangeInclusive<u32> = 0..=1000;

pub const SCALE_RANGE: std::ops::RangeInclusive<f32> = 0.75..=2.0;
pub const GAP_RANGE: std::ops::RangeInclusive<f32> = 0.0..=48.0;

impl Prefs {
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
        // A locale code is short ASCII (`sr-latn`, `zh-hant`); anything else follows the system.
        let lang = self.language.trim().to_ascii_lowercase();
        self.language = if lang.is_empty() || lang.len() > 35 || !lang.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') { crate::i18n::AUTO.to_string() } else { lang };
        self.shortcuts = crate::keymap::sanitize(std::mem::take(&mut self.shortcuts));
        self.autosave_secs = self.autosave_secs.clamp(*AUTOSAVE_SECS_RANGE.start(), *AUTOSAVE_SECS_RANGE.end());
        self.autosave_edits = self.autosave_edits.min(*AUTOSAVE_EDITS_RANGE.end());
        self.onion.before = self.onion.before.min(ONION_REACH_MAX);
        self.onion.after = self.onion.after.min(ONION_REACH_MAX);
        self
    }

    pub fn from_json(text: &str) -> Prefs {
        serde_json::from_str::<Prefs>(text).unwrap_or_default().sanitized()
    }

    pub fn load() -> Prefs {
        crate::platform::load_settings().map_or_else(Prefs::default, |t| Prefs::from_json(&t))
    }

    /// Writes where the platform keeps settings (a file, atomically; or browser storage).
    pub fn save(&self) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        crate::platform::save_settings(&json)
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
        assert_eq!(p.language, "auto", "older files follow the system's language");
        assert_eq!(Prefs::from_json(r#"{"language": " DE "}"#).language, "de");
        assert_eq!(Prefs::from_json(r#"{"language": "xx-new"}"#).language, "xx-new", "a code this build lacks is kept");
        assert_eq!(Prefs::from_json(r#"{"language": "<script>"}"#).language, "auto");
        assert_eq!(Prefs::from_json(&format!(r#"{{"language": "{}"}}"#, "a".repeat(100))).language, "auto");
        assert_eq!(p.stage_renderer, StageRenderer::Gpu, "the GPU draws the stage unless the CPU is chosen");
        assert_eq!(Prefs::from_json(r#"{"stage_renderer": "Cpu"}"#).stage_renderer, StageRenderer::Gpu, "the old default, written by every earlier build, is not a choice");
        assert_eq!(Prefs::from_json(r#"{"stage": "Cpu"}"#).stage_renderer, StageRenderer::Cpu);
        let chosen = Prefs { stage_renderer: StageRenderer::Cpu, ..Prefs::default() };
        assert_eq!(Prefs::from_json(&serde_json::to_string(&chosen).unwrap()).stage_renderer, StageRenderer::Cpu, "a choice survives saving");
        let p = Prefs::from_json(r#"{"shortcuts": {"tool.pen": ["F10", "Ctrl+K"], "nope": ["F2"]}}"#);
        assert_eq!(p.shortcuts.len(), 1);
        assert_eq!(p.shortcuts.get("tool.pen"), Some(&vec!["F10".to_string()]));
        // Older preference files have no autosave settings: autosave is on, every minute.
        let p = Prefs::from_json(r#"{"ui_scale": 1.25}"#);
        assert!(p.autosave);
        assert_eq!((p.autosave_secs, p.autosave_edits), (60, 25));
        let p = Prefs::from_json(r#"{"autosave": false, "autosave_secs": 1, "autosave_edits": 99999999}"#);
        assert!(!p.autosave);
        assert_eq!((p.autosave_secs, p.autosave_edits), (15, 1000));
    }
}
