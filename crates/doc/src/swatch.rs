//! The document's swatches (ADR 0078): saved colours and gradients, as Animate keeps them per
//! document in its Swatches panel.
//!
//! A swatch is a colour (with alpha) or a gradient's ramp and options, never its geometry: a
//! gradient swatch poured on a shape is fitted to it, as a new gradient is. A new document
//! starts with [`default_set`]; a document file that has no swatches (every file before ADR
//! 0078) opens with that set too, and the set is not written while it is unchanged.
//!
//! GIMP palettes (`.gpl`, an open plain-text format) import through [`parse_gpl`].

use crate::{GradientStop, MAX_FOCAL, MAX_GRADIENT_STOPS, Paint, Rgba, Spread, is_false, is_zero};
use serde::{Deserialize, Serialize};

/// Most swatches a document keeps (Animate's panel holds a few hundred at most).
pub const MAX_SWATCHES: usize = 1024;
/// Longest swatch name, in characters.
pub const MAX_SWATCH_NAME: usize = 128;

/// One saved colour or gradient.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Swatch {
    /// Shown on hover (empty: the colour's hex).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    pub paint: SwatchPaint,
}

/// What a swatch holds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SwatchPaint {
    Solid {
        color: Rgba,
    },
    Linear {
        stops: Vec<GradientStop>,
        #[serde(default, skip_serializing_if = "Spread::is_pad")]
        spread: Spread,
        #[serde(default, skip_serializing_if = "is_false")]
        linear_rgb: bool,
    },
    Radial {
        stops: Vec<GradientStop>,
        #[serde(default, skip_serializing_if = "Spread::is_pad")]
        spread: Spread,
        #[serde(default, skip_serializing_if = "is_false")]
        linear_rgb: bool,
        #[serde(default, skip_serializing_if = "is_zero")]
        focal: f32,
    },
}

impl SwatchPaint {
    /// The swatch of a fill: its colour, or its gradient's ramp and options (where it lies is
    /// left behind). `None` for a bitmap fill, which a swatch can't hold.
    pub fn from_paint(p: &Paint) -> Option<SwatchPaint> {
        match p {
            Paint::Solid(c) => Some(SwatchPaint::Solid { color: *c }),
            Paint::Linear { stops, spread, linear_rgb, .. } => Some(SwatchPaint::Linear { stops: stops.clone(), spread: *spread, linear_rgb: *linear_rgb }),
            Paint::Radial { stops, spread, linear_rgb, focal, .. } => Some(SwatchPaint::Radial { stops: stops.clone(), spread: *spread, linear_rgb: *linear_rgb, focal: *focal }),
            Paint::Bitmap { .. } => None,
        }
    }

    /// Its colour, or its gradient's first stop.
    pub fn primary_color(&self) -> Rgba {
        match self {
            SwatchPaint::Solid { color } => *color,
            SwatchPaint::Linear { stops, .. } | SwatchPaint::Radial { stops, .. } => stops.first().map_or(Rgba::BLACK, |s| s.color),
        }
    }

    pub fn stops(&self) -> Option<&[GradientStop]> {
        match self {
            SwatchPaint::Solid { .. } => None,
            SwatchPaint::Linear { stops, .. } | SwatchPaint::Radial { stops, .. } => Some(stops),
        }
    }

    pub fn is_gradient(&self) -> bool {
        !matches!(self, SwatchPaint::Solid { .. })
    }

    /// The key `paint.set_fill` and swatch files use for its kind.
    pub fn kind(&self) -> &'static str {
        match self {
            SwatchPaint::Solid { .. } => "solid",
            SwatchPaint::Linear { .. } => "linear",
            SwatchPaint::Radial { .. } => "radial",
        }
    }

    /// Repaired: colours in range, 2 to [`MAX_GRADIENT_STOPS`] stops in order, the focal point
    /// in range. `None` when it can't be (a gradient with fewer than two stops).
    pub fn sanitized(&self) -> Option<SwatchPaint> {
        let stops = |s: &[GradientStop]| -> Option<Vec<GradientStop>> {
            if s.len() < 2 {
                return None;
            }
            let mut v: Vec<GradientStop> = s.iter().take(MAX_GRADIENT_STOPS).map(|g| GradientStop { offset: if g.offset.is_finite() { g.offset.clamp(0.0, 1.0) } else { 0.0 }, color: g.color.sanitized() }).collect();
            v.sort_by(|a, b| a.offset.total_cmp(&b.offset));
            Some(v)
        };
        Some(match self {
            SwatchPaint::Solid { color } => SwatchPaint::Solid { color: color.sanitized() },
            SwatchPaint::Linear { stops: s, spread, linear_rgb } => SwatchPaint::Linear { stops: stops(s)?, spread: *spread, linear_rgb: *linear_rgb },
            SwatchPaint::Radial { stops: s, spread, linear_rgb, focal } => SwatchPaint::Radial { stops: stops(s)?, spread: *spread, linear_rgb: *linear_rgb, focal: if focal.is_finite() { focal.clamp(-MAX_FOCAL, MAX_FOCAL) } else { 0.0 } },
        })
    }
}

impl Swatch {
    pub fn new(name: &str, paint: SwatchPaint) -> Swatch {
        Swatch { name: clean_name(name), paint }
    }
    pub fn solid(c: Rgba) -> Swatch {
        Swatch { name: String::new(), paint: SwatchPaint::Solid { color: c } }
    }
    /// Repaired (see [`SwatchPaint::sanitized`]), its name trimmed and bounded.
    pub fn sanitized(&self) -> Option<Swatch> {
        Some(Swatch { name: clean_name(&self.name), paint: self.paint.sanitized()? })
    }
}

/// A swatch name: one line, trimmed, at most [`MAX_SWATCH_NAME`] characters.
pub fn clean_name(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(MAX_SWATCH_NAME).collect::<String>().trim().to_string()
}

fn stop(offset: f32, rgb: u32) -> GradientStop {
    GradientStop { offset, color: Rgba::hex(rgb) }
}

/// A new document's swatches: greys from black to white, the primaries and secondaries (the
/// picker's web-safe palette has the rest), then seven gradients.
pub fn default_set() -> Vec<Swatch> {
    let colours = [0x000000, 0x333333, 0x666666, 0x999999, 0xcccccc, 0xffffff, 0xff0000, 0x00ff00, 0x0000ff, 0xffff00, 0x00ffff, 0xff00ff];
    let mut v: Vec<Swatch> = colours.iter().map(|c| Swatch::solid(Rgba::hex(*c))).collect();
    let linear = |stops| SwatchPaint::Linear { stops, spread: Spread::Pad, linear_rgb: false };
    let radial = |stops| SwatchPaint::Radial { stops, spread: Spread::Pad, linear_rgb: false, focal: 0.0 };
    v.extend([
        Swatch::new("Black to white", linear(vec![stop(0.0, 0x000000), stop(1.0, 0xffffff)])),
        Swatch::new("White to black", radial(vec![stop(0.0, 0xffffff), stop(1.0, 0x000000)])),
        Swatch::new("Red ball", radial(vec![stop(0.0, 0xff8080), stop(1.0, 0x990000)])),
        Swatch::new("Green ball", radial(vec![stop(0.0, 0x80ff80), stop(1.0, 0x006600)])),
        Swatch::new("Blue ball", radial(vec![stop(0.0, 0x80b0ff), stop(1.0, 0x000099)])),
        Swatch::new("Spectrum", linear(vec![stop(0.0, 0xff0000), stop(0.2, 0xffff00), stop(0.4, 0x00ff00), stop(0.6, 0x00ffff), stop(0.8, 0x0000ff), stop(1.0, 0xff00ff)])),
        Swatch::new("Gold", linear(vec![stop(0.0, 0xfff2a8), stop(0.5, 0xd4a017), stop(1.0, 0x7a5200)])),
    ]);
    v
}

/// Whether `v` is exactly the default set (then a document file leaves it out).
pub fn is_default_set(v: &[Swatch]) -> bool {
    let d = default_set();
    v.len() == d.len() && v.iter().zip(&d).all(|(a, b)| a == b)
}

/// Largest palette file read, in bytes.
pub const MAX_PALETTE_BYTES: usize = 1 << 20;

/// Reads a GIMP palette (`.gpl`): a `GIMP Palette` line, optional `Name:` and `Columns:`
/// lines and `#` comments, then one colour per line, `R G B` (0 to 255) and an optional name.
/// Lines that aren't colours are skipped; at most [`MAX_SWATCHES`] colours are read. The
/// palette's own name comes back with the colours.
pub fn parse_gpl(text: &str) -> Result<(String, Vec<Swatch>), String> {
    if text.len() > MAX_PALETTE_BYTES {
        return Err("the palette file is too large".into());
    }
    let mut lines = text.lines();
    let first = lines.next().unwrap_or("").trim_start_matches('\u{feff}').trim();
    if first != "GIMP Palette" {
        return Err("this isn't a GIMP palette (its first line must be \"GIMP Palette\")".into());
    }
    let mut name = String::new();
    let mut out = Vec::new();
    for line in lines {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        if let Some(n) = l.strip_prefix("Name:") {
            name = clean_name(n);
            continue;
        }
        if l.starts_with("Columns:") || l.starts_with("Channels:") {
            continue;
        }
        let mut parts = l.split_whitespace();
        let channel = |s: Option<&str>| s.and_then(|v| v.parse::<u16>().ok()).filter(|v| *v <= 255).map(|v| f32::from(v) / 255.0);
        let (Some(r), Some(g), Some(b)) = (channel(parts.next()), channel(parts.next()), channel(parts.next())) else { continue };
        let rest: Vec<&str> = parts.collect();
        let label = rest.join(" ");
        // GIMP names unnamed colours "Untitled"; the hex says more.
        let label = if label == "Untitled" { String::new() } else { label };
        out.push(Swatch::new(&label, SwatchPaint::Solid { color: Rgba::new(r, g, b, 1.0) }));
        if out.len() >= MAX_SWATCHES {
            break;
        }
    }
    if out.is_empty() {
        return Err("the palette has no colours in it".into());
    }
    Ok((name, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_set_is_valid_and_recognised() {
        let d = default_set();
        assert_eq!(d.len(), 19);
        assert!(is_default_set(&d));
        assert!(d.iter().all(|s| s.sanitized().as_ref() == Some(s)), "nothing to repair");
        assert_eq!(d.iter().filter(|s| s.paint.is_gradient()).count(), 7);
        let mut other = d.clone();
        other.swap(0, 1);
        assert!(!is_default_set(&other));
        assert!(!is_default_set(&[]));
    }

    #[test]
    fn swatches_are_repaired() {
        let bad = Swatch { name: "  a\nb  ".into(), paint: SwatchPaint::Radial { stops: vec![GradientStop { offset: 2.0, color: Rgba::new(f32::NAN, 9.0, -1.0, 1.0) }, GradientStop { offset: -1.0, color: Rgba::WHITE }], spread: Spread::Pad, linear_rgb: false, focal: 5.0 } };
        let s = bad.sanitized().unwrap();
        assert_eq!(s.name, "ab");
        let SwatchPaint::Radial { stops, focal, .. } = &s.paint else { panic!() };
        assert_eq!((stops[0].offset, stops[1].offset), (0.0, 1.0), "in order and in range");
        assert_eq!(stops[1].color, Rgba::new(0.0, 1.0, 0.0, 1.0));
        assert_eq!(*focal, MAX_FOCAL);
        let one = Swatch { name: String::new(), paint: SwatchPaint::Linear { stops: vec![GradientStop { offset: 0.0, color: Rgba::BLACK }], spread: Spread::Pad, linear_rgb: false } };
        assert!(one.sanitized().is_none(), "a gradient needs two stops");
        assert_eq!(clean_name(&"x".repeat(500)).chars().count(), MAX_SWATCH_NAME);
    }

    #[test]
    fn a_swatch_keeps_a_fills_ramp_not_its_geometry() {
        let p = Paint::linear(crate::Vec2::new(5.0, 5.0), crate::Vec2::new(50.0, 5.0), vec![stop(0.0, 0xff0000), stop(1.0, 0x0000ff)]);
        let s = SwatchPaint::from_paint(&p).unwrap();
        assert_eq!(s.kind(), "linear");
        assert_eq!(s.primary_color(), Rgba::hex(0xff0000));
        assert!(SwatchPaint::from_paint(&Paint::Bitmap { bitmap: crate::BitmapId(1), repeat: true, matrix: crate::Affine::IDENTITY }).is_none());
    }

    #[test]
    fn gimp_palettes_are_read() {
        let text = "GIMP Palette\nName: Brand\nColumns: 4\n# a comment\n255   0   0\tCrimson red\n  0 128 255 Untitled\n300 0 0 too bright\nnot a colour\n\n10 20 30\n";
        let (name, sw) = parse_gpl(text).unwrap();
        assert_eq!(name, "Brand");
        assert_eq!(sw.len(), 3);
        assert_eq!(sw[0].name, "Crimson red");
        assert_eq!(sw[0].paint, SwatchPaint::Solid { color: Rgba::hex(0xff0000) });
        assert_eq!(sw[1].name, "", "GIMP's Untitled shows as the hex");
        assert_eq!(sw[2].paint.primary_color().to_hex(), "#0a141e");
        // A byte-order mark and Windows line ends are fine.
        assert!(parse_gpl("\u{feff}GIMP Palette\r\n1 2 3\r\n").is_ok());
        for bad in ["", "JASC-PAL\n0 0 0", "GIMP Palette\nName: empty\n", "GIMP Palette\n1 2\n"] {
            assert!(parse_gpl(bad).is_err(), "{bad:?}");
        }
        let many = format!("GIMP Palette\n{}", "1 2 3\n".repeat(MAX_SWATCHES + 10));
        assert_eq!(parse_gpl(&many).unwrap().1.len(), MAX_SWATCHES, "bounded");
        assert!(parse_gpl(&"x".repeat(MAX_PALETTE_BYTES + 1)).is_err());
    }
}
