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
    /// Graphite or high contrast (ADR 0022), when chosen by hand.
    pub theme: crate::theme::Palette,
    /// Follow the system's contrast setting (ADR 0064): high contrast while the system asks for
    /// more contrast, graphite otherwise. `None` (files from before): it follows, unless the
    /// file chose high contrast.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme_follows_system: Option<bool>,
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
    /// Frame view › Short (ADR 0061): timeline rows at their short height.
    pub timeline_short_rows: bool,
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
    /// Publish Settings presets, by name (ADR 0060): yours on this computer, for any document.
    pub publish_presets: std::collections::BTreeMap<String, anicraft_engine::doc::publish::PublishProfile>,
    /// File › Open Recent and the start screen's list (ADR 0059): pinned favourites, then the
    /// last 10 documents opened or saved.
    #[serde(deserialize_with = "lenient_recent")]
    pub recent: anicraft_engine::cmds_recent::RecentFiles,
    /// Show the start screen when AniCraft starts without a document to open (ADR 0059).
    pub start_screen: bool,
    /// Brushes of your own (ADR 0068), for the Brush and the Paint Brush.
    #[serde(deserialize_with = "lenient_brushes")]
    pub brushes: anicraft_engine::brush_library::BrushLibrary,
    /// Test Movie plays in a window of its own (ADR 0062); off, or where a system can't open
    /// one, it floats inside the editor.
    pub test_movie_window: bool,
    /// Where the Test Movie window was last, in points (its top-left corner); `None` until it
    /// has been opened where the system says where windows are.
    pub test_movie_pos: Option<[f32; 2]>,
}

/// Most publish presets kept, and the longest name one has.
pub const MAX_PUBLISH_PRESETS: usize = 100;
pub const MAX_PRESET_NAME: usize = 64;

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
            theme_follows_system: None,
            ui_scale: 1.0,
            render_mode: RenderMode::Auto,
            stage_renderer: StageRenderer::Gpu,
            native_dialogs: true,
            timeline_start_gap: 0.0,
            timeline_short_rows: false,
            shortcuts: Default::default(),
            autosave: true,
            autosave_secs: 60,
            autosave_edits: 25,
            onion: OnionPrefs::default(),
            publish_presets: Default::default(),
            recent: Default::default(),
            start_screen: true,
            brushes: Default::default(),
            test_movie_window: true,
            test_movie_pos: None,
        }
    }
}

/// The recent files as written, skipping entries that aren't one (a hand-edited file): one bad
/// entry must not reset every preference.
fn lenient_recent<'de, D: serde::Deserializer<'de>>(d: D) -> Result<anicraft_engine::cmds_recent::RecentFiles, D::Error> {
    use anicraft_engine::cmds_recent::{RecentFile, RecentFiles};
    let v = serde_json::Value::deserialize(d)?;
    let list: Vec<serde_json::Value> = match v {
        serde_json::Value::Array(a) => a.into_iter().take(1000).collect(),
        _ => Vec::new(),
    };
    let good: Vec<RecentFile> = list.into_iter().filter_map(|x| serde_json::from_value::<RecentFile>(x).ok()).collect();
    Ok(serde_json::from_value::<RecentFiles>(serde_json::to_value(good).unwrap_or_default()).unwrap_or_default())
}

/// Your brushes as written, skipping entries that aren't one (a hand-edited file).
fn lenient_brushes<'de, D: serde::Deserializer<'de>>(d: D) -> Result<anicraft_engine::brush_library::BrushLibrary, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    let list: Vec<serde_json::Value> = match v {
        serde_json::Value::Array(a) => a,
        _ => Vec::new(),
    };
    Ok(anicraft_engine::brush_library::BrushLibrary::from_values(list))
}

/// Autosave intervals offered in Preferences: `(seconds, label)`.
pub const AUTOSAVE_INTERVALS: [(u32, &str); 5] = [(30, tr_noop!("30 seconds")), (60, tr_noop!("1 minute")), (120, tr_noop!("2 minutes")), (300, tr_noop!("5 minutes")), (600, tr_noop!("10 minutes"))];
pub const AUTOSAVE_SECS_RANGE: std::ops::RangeInclusive<u32> = 15..=600;
pub const AUTOSAVE_EDITS_RANGE: std::ops::RangeInclusive<u32> = 0..=1000;

pub const SCALE_RANGE: std::ops::RangeInclusive<f32> = 0.75..=2.0;
pub const GAP_RANGE: std::ops::RangeInclusive<f32> = 0.0..=48.0;

impl Prefs {
    /// Whether the theme follows the system's contrast setting ("Follow system", the default).
    pub fn follows_system(&self) -> bool {
        self.theme_follows_system.unwrap_or(self.theme != crate::theme::Palette::HighContrast)
    }

    /// The palette to draw with, given what the system says about contrast (`None`: it
    /// doesn't say, or hasn't yet).
    pub fn palette(&self, system_high_contrast: Option<bool>) -> crate::theme::Palette {
        match (self.follows_system(), system_high_contrast) {
            (true, Some(true)) => crate::theme::Palette::HighContrast,
            (true, _) => crate::theme::Palette::Graphite,
            (false, _) => self.theme,
        }
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
        // A locale code is short ASCII (`sr-latn`, `zh-hant`); anything else follows the system.
        let lang = self.language.trim().to_ascii_lowercase();
        self.language = if lang.is_empty() || lang.len() > 35 || !lang.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') { crate::i18n::AUTO.to_string() } else { lang };
        self.shortcuts = crate::keymap::sanitize(std::mem::take(&mut self.shortcuts));
        self.autosave_secs = self.autosave_secs.clamp(*AUTOSAVE_SECS_RANGE.start(), *AUTOSAVE_SECS_RANGE.end());
        self.autosave_edits = self.autosave_edits.min(*AUTOSAVE_EDITS_RANGE.end());
        self.onion.before = self.onion.before.min(ONION_REACH_MAX);
        self.onion.after = self.onion.after.min(ONION_REACH_MAX);
        self.publish_presets = std::mem::take(&mut self.publish_presets)
            .into_iter()
            .filter(|(name, _)| !name.trim().is_empty() && name.chars().count() <= MAX_PRESET_NAME)
            .take(MAX_PUBLISH_PRESETS)
            .map(|(name, mut p)| {
                p.sanitize();
                (name, p)
            })
            .collect();
        self.recent = std::mem::take(&mut self.recent).sanitized();
        self.brushes = std::mem::take(&mut self.brushes).sanitized();
        self.test_movie_pos = self.test_movie_pos.filter(|[x, y]| x.is_finite() && y.is_finite() && x.abs() < 1e5 && y.abs() < 1e5);
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
    fn the_theme_follows_the_system_unless_one_was_chosen() {
        use crate::theme::Palette;
        // New installs, and files from before ADR 0064 that kept the default, follow the system.
        for p in [Prefs::default(), Prefs::from_json(r#"{"theme": "Graphite"}"#)] {
            assert!(p.follows_system());
            assert_eq!(p.palette(Some(true)), Palette::HighContrast);
            assert_eq!(p.palette(Some(false)), Palette::Graphite);
            assert_eq!(p.palette(None), Palette::Graphite, "a system that doesn't say: graphite");
        }
        // A file that chose high contrast keeps it whatever the system says.
        let hc = Prefs::from_json(r#"{"theme": "HighContrast"}"#);
        assert!(!hc.follows_system());
        assert_eq!(hc.palette(Some(false)), Palette::HighContrast);
        // Graphite chosen by hand stays graphite; the choice survives a round trip.
        let manual = Prefs { theme_follows_system: Some(false), ..Prefs::default() };
        assert_eq!(manual.palette(Some(true)), Palette::Graphite);
        let back = Prefs::from_json(&serde_json::to_string(&manual).unwrap_or_default());
        assert_eq!(back.theme_follows_system, Some(false));
        assert!(!serde_json::to_string(&Prefs::default()).unwrap_or_default().contains("theme_follows_system"), "nothing new written until chosen");
    }

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
    fn publish_presets_round_trip_and_hostile_ones_are_dropped() {
        let mut gif = anicraft_engine::doc::publish::PublishProfile::default();
        gif.gif.enabled = true;
        gif.gif.scale = 0.5;
        let mut p = Prefs::default();
        p.publish_presets.insert("Web small".into(), gif.clone());
        let back = Prefs::from_json(&serde_json::to_string(&p).unwrap());
        assert_eq!(back.publish_presets.get("Web small"), Some(&gif));
        let hostile = format!(r#"{{"publish_presets": {{"": {{}}, "{}": {{}}, "ok": {{"gif": {{"scale": 1e30}}, "audio": {{"bits": 3}}}}, "junk": 5}}}}"#, "n".repeat(100));
        let p = Prefs::from_json(&hostile);
        assert!(p.publish_presets.is_empty() || p.publish_presets.len() == 1, "{:?}", p.publish_presets.keys());
        if let Some(ok) = p.publish_presets.get("ok") {
            assert_eq!((ok.gif.scale, ok.audio.bits), (anicraft_engine::doc::publish::MAX_PUBLISH_SCALE, 16));
        }
        assert_eq!(Prefs::from_json(r#"{"publish_presets": {"": {}, "a": {"gif": {"enabled": true}}}}"#).publish_presets.len(), 1);
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
        // Older files have no recent files and show the start screen (ADR 0059); a list
        // someone edited by hand is cleaned up.
        let p = Prefs::from_json(r#"{"ui_scale": 1.25}"#);
        assert!(p.recent.is_empty() && p.start_screen);
        let p = Prefs::from_json(r#"{"recent": [{"path": "a.anicraft"}, {"path": "a.anicraft"}, {"path": ""}, 7], "start_screen": false}"#);
        let paths: Vec<_> = p.recent.entries().iter().map(|e| e.path.display().to_string()).collect();
        assert_eq!(paths, ["a.anicraft"], "bad entries are skipped, the rest kept");
        assert!(!p.start_screen, "and the other preferences with them");
        // Your brushes (ADR 0068): none in older files; junk entries skipped, values clamped,
        // and they survive a save and load.
        assert!(Prefs::from_json(r#"{"ui_scale": 1.25}"#).brushes.is_empty());
        let p = Prefs::from_json(r#"{"brushes": [{"name": "Soft", "size": 99999, "curve": "x"}, 3, {"name": "Ink", "jitter": 12}, {"name": ""}], "start_screen": false}"#);
        let names: Vec<_> = p.brushes.user().iter().map(|b| (b.name.clone(), b.size)).collect();
        assert_eq!(names, [("Ink".to_string(), 10.0)], "a bad curve drops that brush, the rest stay");
        assert!(!p.start_screen);
        let back = Prefs::from_json(&serde_json::to_string(&p).unwrap());
        assert_eq!(back.brushes, p.brushes);
        // Older files open Test Movie in its own window, with no remembered place (ADR 0062);
        // a place someone broke by hand is forgotten, not used.
        let p = Prefs::from_json(r#"{"ui_scale": 1.25}"#);
        assert!(p.test_movie_window && p.test_movie_pos.is_none());
        let p = Prefs::from_json(r#"{"test_movie_window": false, "test_movie_pos": [120.5, 80]}"#);
        assert_eq!((p.test_movie_window, p.test_movie_pos), (false, Some([120.5, 80.0])));
        assert_eq!(Prefs::from_json(r#"{"test_movie_pos": [1e9, 0]}"#).test_movie_pos, None);
        assert_eq!(Prefs::from_json(&serde_json::to_string(&p).unwrap()), p, "both survive saving");
        let p = Prefs::from_json(r#"{"recent": [{"path": "b.anicraft"}, {"path": "a.anicraft", "pinned": true}, {"path": "b.anicraft"}]}"#);
        let paths: Vec<_> = p.recent.entries().iter().map(|e| (e.path.display().to_string(), e.pinned)).collect();
        assert_eq!(paths, [("a.anicraft".to_string(), true), ("b.anicraft".to_string(), false)]);
    }
}
