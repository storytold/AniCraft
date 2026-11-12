//! Publish Settings (ADR 0060): which formats File › Publish writes, each with its options and
//! output name, kept in the document as Animate keeps them.
//!
//! Every format is always present, in the fixed order Publish runs them; a format is on or off.
//! Outputs are templates relative to the document's folder: `{name}` is the document's file name
//! without its extension. A file written before publish settings existed has none and gets
//! [`PublishProfile::default`] (the HTML canvas page on, the rest off) the first time it opens.

use serde::{Deserialize, Serialize};

/// The formats File › Publish can write, in the order it writes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublishFormat {
    Html,
    Gif,
    Apng,
    Webp,
    Video,
    Lottie,
    SpriteSheet,
    PngSequence,
    Png,
    Svg,
    Audio,
}

impl PublishFormat {
    /// Every format, in the order Publish runs them.
    pub const ALL: [PublishFormat; 11] = [
        PublishFormat::Html,
        PublishFormat::Gif,
        PublishFormat::Apng,
        PublishFormat::Webp,
        PublishFormat::Video,
        PublishFormat::Lottie,
        PublishFormat::SpriteSheet,
        PublishFormat::PngSequence,
        PublishFormat::Png,
        PublishFormat::Svg,
        PublishFormat::Audio,
    ];

    /// The stable key used by commands and the file format.
    pub fn key(self) -> &'static str {
        match self {
            PublishFormat::Html => "html",
            PublishFormat::Gif => "gif",
            PublishFormat::Apng => "apng",
            PublishFormat::Webp => "webp",
            PublishFormat::Video => "video",
            PublishFormat::Lottie => "lottie",
            PublishFormat::SpriteSheet => "sprite_sheet",
            PublishFormat::PngSequence => "png_sequence",
            PublishFormat::Png => "png",
            PublishFormat::Svg => "svg",
            PublishFormat::Audio => "audio",
        }
    }

    pub fn from_key(key: &str) -> Option<PublishFormat> {
        PublishFormat::ALL.into_iter().find(|f| f.key() == key)
    }

    /// The English name of the format (translated where it is drawn).
    pub fn label(self) -> &'static str {
        match self {
            PublishFormat::Html => "HTML canvas",
            PublishFormat::Gif => "Animated GIF",
            PublishFormat::Apng => "Animated PNG",
            PublishFormat::Webp => "Animated WebP",
            PublishFormat::Video => "Video",
            PublishFormat::Lottie => "Lottie",
            PublishFormat::SpriteSheet => "Sprite sheet",
            PublishFormat::PngSequence => "PNG sequence",
            PublishFormat::Png => "PNG image",
            PublishFormat::Svg => "SVG image",
            PublishFormat::Audio => "Audio",
        }
    }

    /// The output a new profile gives the format.
    pub fn default_output(self) -> &'static str {
        match self {
            PublishFormat::Html => "{name}.html",
            PublishFormat::Gif => "{name}.gif",
            // Not {name}.png: the PNG image has that.
            PublishFormat::Apng => "{name}_animated.png",
            PublishFormat::Webp => "{name}.webp",
            PublishFormat::Video => "{name}.webm",
            PublishFormat::Lottie => "{name}.json",
            PublishFormat::SpriteSheet => "{name}_sheet.png",
            PublishFormat::PngSequence => "{name}_frames",
            PublishFormat::Png => "{name}.png",
            PublishFormat::Svg => "{name}.svg",
            PublishFormat::Audio => "{name}.wav",
        }
    }

    /// The file extensions the output may end in (none for the PNG sequence: a folder).
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            PublishFormat::Html => &["html", "htm"],
            PublishFormat::Gif => &["gif"],
            PublishFormat::Apng => &["png", "apng"],
            PublishFormat::Webp => &["webp"],
            PublishFormat::Video => &["webm", "mp4"],
            PublishFormat::Lottie => &["json"],
            PublishFormat::SpriteSheet | PublishFormat::Png => &["png"],
            PublishFormat::PngSequence => &[],
            PublishFormat::Svg => &["svg"],
            PublishFormat::Audio => &["wav"],
        }
    }
}

/// Longest output template, in characters.
pub const MAX_OUTPUT_CHARS: usize = 512;
/// Longest HTML page title, in characters.
pub const MAX_TITLE_CHARS: usize = 256;
/// Largest export scale.
pub const MAX_PUBLISH_SCALE: f32 = 8.0;
/// Most plays an animated PNG or WebP's loop count says (WebP stores it in 16 bits).
pub const MAX_PLAYS: u32 = 65_535;

/// Movie frames to publish (every scene in order, 0-based): `None` ends are the movie's ends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FrameRange {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<u32>,
}

macro_rules! format_settings {
    ($(#[$meta:meta])* $name:ident, $format:expr, $enabled:expr, { $($(#[$fmeta:meta])* $field:ident : $ty:ty = $default:expr),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
        #[serde(default)]
        pub struct $name {
            /// Publish writes this format.
            pub enabled: bool,
            /// A path relative to the document's folder; `{name}` is the document's name.
            pub output: String,
            $($(#[$fmeta])* pub $field: $ty,)*
        }
        impl Default for $name {
            fn default() -> Self {
                $name { enabled: $enabled, output: $format.default_output().to_string(), $($field: $default,)* }
            }
        }
    };
}

format_settings!(
    /// The movie as one web page (`file.export_html`, ADR 0028).
    HtmlSettings, PublishFormat::Html, true, {
        /// The page's title; empty: the output's file name.
        title: String = String::new(),
        sound: bool = true,
        fit: bool = true,
    }
);
format_settings!(
    /// An animated GIF (`file.export_gif`).
    GifSettings, PublishFormat::Gif, false, {
        range: FrameRange = FrameRange::default(),
        scale: f32 = 1.0,
        #[serde(rename = "loop")]
        looping: bool = true,
    }
);
format_settings!(
    /// An animated PNG: full colour, real alpha (`file.export_apng`, ADR 0063).
    ApngSettings, PublishFormat::Apng, false, {
        range: FrameRange = FrameRange::default(),
        scale: f32 = 1.0,
        /// How many times it plays; 0: forever.
        #[serde(rename = "loop")]
        plays: u32 = 0,
        /// The stage colour behind it; off: transparent.
        background: bool = false,
    }
);
format_settings!(
    /// An animated WebP, lossless (`file.export_webp`, ADR 0063).
    WebpSettings, PublishFormat::Webp, false, {
        range: FrameRange = FrameRange::default(),
        scale: f32 = 1.0,
        /// How many times it plays; 0: forever.
        #[serde(rename = "loop")]
        plays: u32 = 0,
        /// The stage colour behind it; off: transparent.
        background: bool = false,
    }
);
format_settings!(
    /// WebM or MP4 through the user's ffmpeg (`file.export_video`).
    VideoSettings, PublishFormat::Video, false, {
        range: FrameRange = FrameRange::default(),
        scale: f32 = 1.0,
        audio: bool = true,
    }
);
format_settings!(
    /// A Lottie animation (`file.export_lottie`).
    LottieSettings, PublishFormat::Lottie, false, {
        range: FrameRange = FrameRange::default(),
        background: bool = false,
    }
);
format_settings!(
    /// A sprite sheet and its JSON frame map (`file.export_sprite_sheet`).
    SpriteSheetSettings, PublishFormat::SpriteSheet, false, {
        range: FrameRange = FrameRange::default(),
        scale: f32 = 1.0,
        /// 0: as square as it can be.
        columns: u32 = 0,
        padding: u32 = 0,
        background: bool = false,
    }
);
format_settings!(
    /// One PNG per frame in a folder (`file.export_png_sequence`).
    PngSequenceSettings, PublishFormat::PngSequence, false, {
        range: FrameRange = FrameRange::default(),
        scale: f32 = 1.0,
    }
);
format_settings!(
    /// One frame as a PNG (`file.export_png`).
    PngSettings, PublishFormat::Png, false, {
        /// The movie frame (0-based, every scene in order).
        frame: u32 = 0,
        scale: f32 = 1.0,
    }
);
format_settings!(
    /// One frame as an SVG (`file.export_svg`).
    SvgSettings, PublishFormat::Svg, false, {
        /// The movie frame (0-based, every scene in order).
        frame: u32 = 0,
    }
);
format_settings!(
    /// The movie's soundtrack as a WAV (`file.export_audio`).
    AudioSettings, PublishFormat::Audio, false, {
        range: FrameRange = FrameRange::default(),
        rate: u32 = 44_100,
        /// 16, 24, or 32 (float).
        bits: u32 = 16,
    }
);

/// What File › Publish writes: every format with its options (ADR 0060).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PublishProfile {
    pub html: HtmlSettings,
    pub gif: GifSettings,
    pub apng: ApngSettings,
    pub webp: WebpSettings,
    pub video: VideoSettings,
    pub lottie: LottieSettings,
    pub sprite_sheet: SpriteSheetSettings,
    pub png_sequence: PngSequenceSettings,
    pub png: PngSettings,
    pub svg: SvgSettings,
    pub audio: AudioSettings,
}

fn clean_scale(s: f32) -> f32 {
    if s.is_finite() && s > 0.0 { s.min(MAX_PUBLISH_SCALE) } else { 1.0 }
}

fn clip(s: &mut String, max: usize) {
    if s.chars().count() > max {
        *s = s.chars().take(max).collect();
    }
}

impl PublishProfile {
    pub fn is_default(&self) -> bool {
        *self == PublishProfile::default()
    }

    /// Whether Publish writes `f`, and its output template.
    pub fn target(&self, f: PublishFormat) -> (bool, &str) {
        match f {
            PublishFormat::Html => (self.html.enabled, &self.html.output),
            PublishFormat::Gif => (self.gif.enabled, &self.gif.output),
            PublishFormat::Apng => (self.apng.enabled, &self.apng.output),
            PublishFormat::Webp => (self.webp.enabled, &self.webp.output),
            PublishFormat::Video => (self.video.enabled, &self.video.output),
            PublishFormat::Lottie => (self.lottie.enabled, &self.lottie.output),
            PublishFormat::SpriteSheet => (self.sprite_sheet.enabled, &self.sprite_sheet.output),
            PublishFormat::PngSequence => (self.png_sequence.enabled, &self.png_sequence.output),
            PublishFormat::Png => (self.png.enabled, &self.png.output),
            PublishFormat::Svg => (self.svg.enabled, &self.svg.output),
            PublishFormat::Audio => (self.audio.enabled, &self.audio.output),
        }
    }

    /// `f`'s on/off switch and output template, to change.
    pub fn target_mut(&mut self, f: PublishFormat) -> (&mut bool, &mut String) {
        match f {
            PublishFormat::Html => (&mut self.html.enabled, &mut self.html.output),
            PublishFormat::Gif => (&mut self.gif.enabled, &mut self.gif.output),
            PublishFormat::Apng => (&mut self.apng.enabled, &mut self.apng.output),
            PublishFormat::Webp => (&mut self.webp.enabled, &mut self.webp.output),
            PublishFormat::Video => (&mut self.video.enabled, &mut self.video.output),
            PublishFormat::Lottie => (&mut self.lottie.enabled, &mut self.lottie.output),
            PublishFormat::SpriteSheet => (&mut self.sprite_sheet.enabled, &mut self.sprite_sheet.output),
            PublishFormat::PngSequence => (&mut self.png_sequence.enabled, &mut self.png_sequence.output),
            PublishFormat::Png => (&mut self.png.enabled, &mut self.png.output),
            PublishFormat::Svg => (&mut self.svg.enabled, &mut self.svg.output),
            PublishFormat::Audio => (&mut self.audio.enabled, &mut self.audio.output),
        }
    }

    /// The formats that are on, in publishing order.
    pub fn enabled(&self) -> Vec<PublishFormat> {
        PublishFormat::ALL.into_iter().filter(|f| self.target(*f).0).collect()
    }

    /// Brings every value into range, so whatever a file holds, publishing can run: scales in
    /// (0, 8], a known sample rate and bit depth, bounded text.
    pub fn sanitize(&mut self) {
        for f in PublishFormat::ALL {
            let (_, output) = self.target_mut(f);
            clip(output, MAX_OUTPUT_CHARS);
            if output.trim().is_empty() {
                *output = f.default_output().to_string();
            }
        }
        clip(&mut self.html.title, MAX_TITLE_CHARS);
        for s in [&mut self.gif.scale, &mut self.apng.scale, &mut self.webp.scale, &mut self.video.scale, &mut self.sprite_sheet.scale, &mut self.png_sequence.scale, &mut self.png.scale] {
            *s = clean_scale(*s);
        }
        self.sprite_sheet.padding = self.sprite_sheet.padding.min(256);
        self.apng.plays = self.apng.plays.min(MAX_PLAYS);
        self.webp.plays = self.webp.plays.min(MAX_PLAYS);
        self.sprite_sheet.columns = self.sprite_sheet.columns.min(10_000);
        self.audio.rate = self.audio.rate.clamp(8_000, 192_000);
        if !matches!(self.audio.bits, 16 | 24 | 32) {
            self.audio.bits = 16;
        }
        for r in [&mut self.gif.range, &mut self.apng.range, &mut self.webp.range, &mut self.video.range, &mut self.lottie.range, &mut self.sprite_sheet.range, &mut self.png_sequence.range, &mut self.audio.range] {
            if let (Some(a), Some(b)) = (r.from, r.to)
                && b < a
            {
                *r = FrameRange { from: Some(b), to: Some(a) };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip_and_defaults_are_distinct() {
        for f in PublishFormat::ALL {
            assert_eq!(PublishFormat::from_key(f.key()), Some(f));
        }
        let p = PublishProfile::default();
        assert_eq!(p.enabled(), vec![PublishFormat::Html], "a new document publishes its web page");
        let mut outs: Vec<&str> = PublishFormat::ALL.iter().map(|f| p.target(*f).1).collect();
        outs.sort_unstable();
        outs.dedup();
        assert_eq!(outs.len(), PublishFormat::ALL.len(), "no two formats write the same file by default");
    }

    #[test]
    fn profile_round_trips_and_missing_fields_default() {
        let mut p = PublishProfile::default();
        p.gif.enabled = true;
        p.gif.scale = 0.5;
        p.gif.looping = false;
        p.gif.range = FrameRange { from: Some(2), to: Some(9) };
        p.html.title = "Walk".into();
        p.audio.bits = 24;
        let text = serde_json::to_string(&p).unwrap();
        assert!(text.contains("\"loop\":false") && text.contains("\"enabled\":true"), "{text}");
        let back: PublishProfile = serde_json::from_str(&text).unwrap();
        assert_eq!(back, p);
        // A profile from an older build, with fields and formats missing.
        let partial: PublishProfile = serde_json::from_str(r#"{"gif": {"enabled": true}}"#).unwrap();
        assert!(partial.gif.enabled && partial.html.enabled);
        assert_eq!(partial.gif.output, "{name}.gif");
        assert_eq!(partial.gif.scale, 1.0);
        assert_eq!((partial.apng.plays, partial.webp.background, partial.webp.enabled), (0, false, false), "formats newer than the file get their defaults");
        let mut q = PublishProfile::default();
        q.apng.plays = 3;
        q.webp.background = true;
        let text = serde_json::to_string(&q).unwrap();
        assert!(text.contains("\"apng\"") && text.contains("\"loop\":3"), "{text}");
        assert_eq!(serde_json::from_str::<PublishProfile>(&text).unwrap(), q);
    }

    #[test]
    fn sanitize_brings_everything_into_range() {
        let mut p = PublishProfile::default();
        p.gif.scale = f32::NAN;
        p.png.scale = 1e9;
        p.audio.rate = 5;
        p.audio.bits = 7;
        p.sprite_sheet.padding = u32::MAX;
        p.webp.plays = u32::MAX;
        p.video.output = "  ".into();
        p.html.title = "x".repeat(10_000);
        p.lottie.range = FrameRange { from: Some(9), to: Some(2) };
        p.sanitize();
        assert_eq!((p.gif.scale, p.png.scale, p.audio.rate, p.audio.bits, p.sprite_sheet.padding), (1.0, MAX_PUBLISH_SCALE, 8_000, 16, 256));
        assert_eq!(p.video.output, "{name}.webm");
        assert_eq!(p.webp.plays, MAX_PLAYS);
        assert_eq!(p.html.title.chars().count(), MAX_TITLE_CHARS);
        assert_eq!(p.lottie.range, FrameRange { from: Some(2), to: Some(9) });
    }
}
