//! The brush library (ADR 0068): brush presets, the built-in ones and the user's own.
//!
//! A preset is a set of tip settings (size, shape, angle, roundness, smoothing), a pressure
//! curve, what pressure drives (size, opacity), and shaping (tapers, dab spacing, a ragged
//! edge). Everything is a parameter: no bitmaps, so a preset is a few numbers, renders at any
//! size and gives the same outline from the same points every time.
//!
//! The built-ins are AniCraft's own, made from these parameters. The user's presets belong to
//! the session (they survive New and Open, like recent files); the desktop app keeps them in
//! its preferences and puts them back at start, so the engine never reads the settings folder.
//! A brush set moves between computers as a small JSON file (`brush.preset_export`/`_import`).

use crate::params::*;
use crate::{EngineError, Result};
use anicraft_geom::brush::{Shaping, Tip, TipShape};
use anicraft_geom::pressure::PressureCurve;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Most presets of the user's own.
pub const MAX_USER_PRESETS: usize = 200;
/// Longest preset name, in characters.
pub const MAX_NAME: usize = 64;
/// Largest brush set file read.
pub const MAX_FILE_BYTES: u64 = 1 << 20;
/// What a brush set file says it is.
pub const FILE_FORMAT: &str = "anicraft-brushes";
pub const FILE_VERSION: u32 = 1;

/// A tip shape as a preset stores it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Shape {
    #[default]
    Round,
    Square,
}

impl Shape {
    pub fn tip(self) -> TipShape {
        match self {
            Shape::Round => TipShape::Round,
            Shape::Square => TipShape::Square,
        }
    }
    pub fn from_tip(t: TipShape) -> Shape {
        match t {
            TipShape::Round => Shape::Round,
            TipShape::Square => Shape::Square,
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Shape::Round => "round",
            Shape::Square => "square",
        }
    }
}

/// A brush preset. Percentages are as Properties shows them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BrushPreset {
    pub name: String,
    /// Diameter (or side), stage units.
    pub size: f32,
    pub shape: Shape,
    /// Degrees, clockwise.
    pub angle: f32,
    /// The narrow side as a percentage of the wide one (5..=100).
    pub roundness: f32,
    /// 0..=100.
    pub smoothing: f32,
    /// How pen pressure maps before it drives size or opacity.
    pub curve: PressureCurve,
    /// Pressure varies the size (from 10 % to 100 %).
    pub pressure_size: bool,
    /// Pressure varies the paint's opacity (the stroke's average, ADR 0068).
    pub pressure_opacity: bool,
    /// Percentages of the stroke's length (0..=50).
    pub taper_start: f32,
    pub taper_end: f32,
    /// 0: a continuous stroke; otherwise dabs this far apart, as a percentage of the size
    /// (5..=1000).
    pub spacing: f32,
    /// A ragged edge, worn in by up to this percentage of the size (0..=50).
    pub jitter: f32,
}

impl Default for BrushPreset {
    /// The Brush tool's own defaults (ADR 0033).
    fn default() -> Self {
        BrushPreset {
            name: String::new(),
            size: 10.0,
            shape: Shape::Round,
            angle: 0.0,
            roundness: 100.0,
            smoothing: 50.0,
            curve: PressureCurve::LINEAR,
            pressure_size: true,
            pressure_opacity: false,
            taper_start: 0.0,
            taper_end: 0.0,
            spacing: 0.0,
            jitter: 0.0,
        }
    }
}

/// The built-in presets: `(key, name)`, in the order the library shows them.
pub const BUILTIN: [(&str, &str); 7] = [
    ("pencil", "Pencil"),
    ("ink_pen", "Ink Pen"),
    ("calligraphy", "Flat Calligraphy"),
    ("round_taper", "Round Taper"),
    ("marker", "Marker"),
    ("chalk", "Chalk"),
    ("dotted", "Dotted"),
];

/// A one-line description of each built-in preset, for tooltips.
pub fn builtin_hint(key: &str) -> &'static str {
    match key {
        "pencil" => "A thin line with short tapered ends; it takes a firm press to paint wide.",
        "ink_pen" => "A smooth inking line that follows a light touch and tapers at both ends.",
        "calligraphy" => "A flat nib at 45°: thick and thin strokes by direction.",
        "round_taper" => "A round brush that swells from a point and thins back to one.",
        "marker" => "A chisel-tipped marker of one width, whatever the pressure.",
        "chalk" => "A broad stroke with a ragged, worn edge.",
        "dotted" => "Round dots spaced along the stroke.",
        _ => "",
    }
}

/// A built-in preset by key.
pub fn builtin(key: &str) -> Option<BrushPreset> {
    let (_, name) = BUILTIN.iter().find(|(k, _)| *k == key)?;
    let d = BrushPreset { name: (*name).to_string(), ..BrushPreset::default() };
    Some(match key {
        "pencil" => BrushPreset { size: 3.0, smoothing: 30.0, curve: PressureCurve::FIRM, taper_start: 5.0, taper_end: 5.0, ..d },
        "ink_pen" => BrushPreset { size: 6.0, smoothing: 60.0, curve: PressureCurve::SOFT, taper_start: 12.0, taper_end: 18.0, ..d },
        "calligraphy" => BrushPreset { size: 18.0, angle: 45.0, roundness: 15.0, smoothing: 50.0, ..d },
        "round_taper" => BrushPreset { size: 14.0, smoothing: 55.0, taper_start: 35.0, taper_end: 35.0, ..d },
        "marker" => BrushPreset { size: 16.0, shape: Shape::Square, angle: 30.0, roundness: 45.0, smoothing: 40.0, pressure_size: false, ..d },
        "chalk" => BrushPreset { size: 20.0, roundness: 80.0, angle: 20.0, smoothing: 40.0, jitter: 18.0, ..d },
        "dotted" => BrushPreset { size: 8.0, smoothing: 60.0, pressure_size: false, spacing: 220.0, ..d },
        _ => d,
    })
}

impl BrushPreset {
    /// The values clamped to their ranges (a file or a hand-edited preference).
    pub fn sanitized(mut self) -> BrushPreset {
        let d = BrushPreset::default();
        let c = |v: f32, lo: f32, hi: f32, def: f32| if v.is_finite() { v.clamp(lo, hi) } else { def };
        self.name = clean_name(&self.name).unwrap_or_default();
        self.size = c(self.size, *crate::cmds_brush::SIZE_RANGE.start(), *crate::cmds_brush::SIZE_RANGE.end(), d.size);
        self.angle = c(self.angle, -1e6, 1e6, 0.0).rem_euclid(180.0);
        self.roundness = c(self.roundness, 5.0, 100.0, 100.0);
        self.smoothing = c(self.smoothing, 0.0, 100.0, d.smoothing);
        self.curve = PressureCurve::new(self.curve.points());
        self.taper_start = c(self.taper_start, 0.0, 50.0, 0.0);
        self.taper_end = c(self.taper_end, 0.0, 50.0, 0.0);
        self.spacing = c(self.spacing, 0.0, 1000.0, 0.0);
        if self.spacing > 0.0 {
            self.spacing = self.spacing.max(5.0);
        }
        self.jitter = c(self.jitter, 0.0, 50.0, 0.0);
        self
    }

    pub fn tip(&self) -> Tip {
        Tip { shape: self.shape.tip(), size: self.size, angle: self.angle, roundness: self.roundness / 100.0 }
    }

    pub fn shaping(&self) -> Shaping {
        Shaping { taper_start: self.taper_start / 100.0, taper_end: self.taper_end / 100.0, spacing: self.spacing / 100.0, jitter: self.jitter / 100.0 }.sanitized()
    }

    /// The tip's size factor input at a measured pressure: the curve's output when pressure
    /// drives the size, else full.
    pub fn size_pressure(&self, pressure: f32) -> f32 {
        if self.pressure_size { self.curve.map(pressure) } else { 1.0 }
    }

    /// The settings without the name, as command params (what `draw.brush` takes).
    pub fn params(&self) -> Value {
        let mut v = serde_json::to_value(self).unwrap_or_default();
        if let Some(o) = v.as_object_mut() {
            o.remove("name");
        }
        v
    }

    /// This preset with any settings `p` gives instead (the params of `draw.brush` and
    /// `brush.preset_save`). Refuses values out of range.
    pub fn with_params(&self, p: &Value) -> Result<BrushPreset> {
        let mut b = self.clone();
        let r = crate::cmds_brush::ranged;
        b.size = r(p, "size", crate::cmds_brush::SIZE_RANGE, b.size)?;
        b.shape = match opt_str(p, "shape")? {
            None => b.shape,
            Some("round") => Shape::Round,
            Some("square") => Shape::Square,
            Some(other) => return Err(EngineError::Param(format!("unknown tip shape '{other}' (round or square)"))),
        };
        if let Some(a) = opt_f32(p, "angle")? {
            if !a.is_finite() || a.abs() > 1e6 {
                return Err(EngineError::Param("param 'angle' must be a finite number of degrees".into()));
            }
            b.angle = a;
        }
        b.roundness = r(p, "roundness", 5.0..=100.0, b.roundness)?;
        b.smoothing = r(p, "smoothing", 0.0..=100.0, b.smoothing)?;
        if let Some(c) = curve_param(p, "curve")? {
            b.curve = c;
        }
        b.pressure_size = opt_bool(p, "pressure_size")?.unwrap_or(b.pressure_size);
        b.pressure_opacity = opt_bool(p, "pressure_opacity")?.unwrap_or(b.pressure_opacity);
        b.taper_start = r(p, "taper_start", 0.0..=50.0, b.taper_start)?;
        b.taper_end = r(p, "taper_end", 0.0..=50.0, b.taper_end)?;
        b.spacing = r(p, "spacing", 0.0..=1000.0, b.spacing)?;
        if b.spacing > 0.0 && b.spacing < 5.0 {
            return Err(EngineError::Param("param 'spacing' must be 0 (a continuous stroke) or from 5 to 1000 %".into()));
        }
        b.jitter = r(p, "jitter", 0.0..=50.0, b.jitter)?;
        Ok(b)
    }
}

/// The `curve` param: a preset name (`linear`, `soft`, `firm`) or `[[pressure, output], ..]`
/// (both 0..1).
pub fn curve_param(p: &Value, key: &str) -> Result<Option<PressureCurve>> {
    let bad = || EngineError::Param(format!("param '{key}' must be linear, soft, firm or a list of [pressure, output] pairs from 0 to 1"));
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(k)) => PressureCurve::preset(k).map(Some).ok_or_else(bad),
        Some(Value::Array(list)) => {
            if list.is_empty() || list.len() > 64 {
                return Err(bad());
            }
            let pts = list
                .iter()
                .map(|v| {
                    let xy = v.as_array().filter(|a| a.len() == 2).ok_or_else(bad)?;
                    let n = |i: usize| xy.get(i).and_then(Value::as_f64).map(|f| f as f32).filter(|f| f.is_finite() && (-1e-3..=1.001).contains(f)).ok_or_else(bad);
                    Ok([n(0)?, n(1)?])
                })
                .collect::<Result<Vec<[f32; 2]>>>()?;
            Ok(Some(PressureCurve::new(&pts)))
        }
        Some(_) => Err(bad()),
    }
}

/// A name as kept: trimmed, one line, at most [`MAX_NAME`] characters; `None` when empty.
pub fn clean_name(s: &str) -> Option<String> {
    let one: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let t: String = one.trim().chars().take(MAX_NAME).collect();
    let t = t.trim().to_string();
    (!t.is_empty()).then_some(t)
}

/// Whether two names are the same preset (case doesn't matter).
fn same_name(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

/// Whether `name` is a built-in preset's (by key or by name).
pub fn is_builtin_name(name: &str) -> bool {
    BUILTIN.iter().any(|(k, n)| same_name(k, name) || same_name(n, name))
}

/// The user's own presets, in the order they were saved.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BrushLibrary {
    user: Vec<BrushPreset>,
}

/// A brush set file.
#[derive(Serialize, Deserialize)]
struct BrushFile {
    format: String,
    version: u32,
    brushes: Vec<Value>,
}

impl BrushLibrary {
    pub fn user(&self) -> &[BrushPreset] {
        &self.user
    }

    pub fn is_empty(&self) -> bool {
        self.user.is_empty()
    }

    /// A library from untrusted entries (preferences, a file): bad entries and names skipped,
    /// values clamped, names made unique, at most [`MAX_USER_PRESETS`].
    pub fn from_values(values: impl IntoIterator<Item = Value>) -> BrushLibrary {
        let mut lib = BrushLibrary::default();
        for v in values.into_iter().take(MAX_USER_PRESETS * 4) {
            if let Ok(p) = serde_json::from_value::<BrushPreset>(v) {
                let _ = lib.add_unique(p);
            }
        }
        lib
    }

    /// The same library with every entry sanitized (after deserializing preferences).
    pub fn sanitized(self) -> BrushLibrary {
        BrushLibrary::from_values(self.user.into_iter().filter_map(|p| serde_json::to_value(p).ok()))
    }

    /// A preset by built-in key, built-in name or the user's name (case doesn't matter; the
    /// user's own win only where no built-in has the name, which saving refuses anyway).
    pub fn find(&self, name: &str) -> Option<BrushPreset> {
        if let Some((k, _)) = BUILTIN.iter().find(|(k, n)| *k == name || same_name(n, name)) {
            return builtin(k);
        }
        self.user.iter().find(|p| same_name(&p.name, name)).cloned()
    }

    pub fn position(&self, name: &str) -> Option<usize> {
        self.user.iter().position(|p| same_name(&p.name, name))
    }

    /// Saves `p` under its name, replacing the user's preset of that name. Returns whether it
    /// replaced one.
    pub fn save(&mut self, p: BrushPreset) -> Result<bool> {
        let p = p.sanitized();
        let name = clean_name(&p.name).ok_or_else(|| EngineError::Param("a brush needs a name".into()))?;
        if is_builtin_name(&name) {
            return Err(EngineError::Param(format!("'{name}' is a built-in brush; save yours under another name")));
        }
        let p = BrushPreset { name, ..p };
        if let Some(i) = self.position(&p.name)
            && let Some(slot) = self.user.get_mut(i)
        {
            *slot = p;
            return Ok(true);
        }
        if self.user.len() >= MAX_USER_PRESETS {
            return Err(EngineError::State(format!("you have {MAX_USER_PRESETS} brushes already; delete some first")));
        }
        self.user.push(p);
        Ok(false)
    }

    /// Adds `p`, renaming it ("Ink 2", "Ink 3", ..) if the name is taken. Returns its name.
    pub fn add_unique(&mut self, p: BrushPreset) -> Result<String> {
        let p = p.sanitized();
        let base = clean_name(&p.name).ok_or_else(|| EngineError::Param("a brush needs a name".into()))?;
        let taken = |lib: &BrushLibrary, n: &str| is_builtin_name(n) || lib.position(n).is_some();
        let mut name = base.clone();
        let mut k = 2;
        while taken(self, &name) {
            if k > MAX_USER_PRESETS + 10 {
                return Err(EngineError::State("too many brushes of that name".into()));
            }
            let suffix = format!(" {k}");
            name = format!("{}{suffix}", base.chars().take(MAX_NAME.saturating_sub(suffix.chars().count())).collect::<String>().trim_end());
            k += 1;
        }
        if self.user.len() >= MAX_USER_PRESETS {
            return Err(EngineError::State(format!("you have {MAX_USER_PRESETS} brushes already; delete some first")));
        }
        self.user.push(BrushPreset { name: name.clone(), ..p });
        Ok(name)
    }

    pub fn rename(&mut self, name: &str, to: &str) -> Result<String> {
        let i = self.position(name).ok_or_else(|| EngineError::Param(format!("you have no brush named '{name}'")))?;
        let to = clean_name(to).ok_or_else(|| EngineError::Param("a brush needs a name".into()))?;
        if is_builtin_name(&to) {
            return Err(EngineError::Param(format!("'{to}' is a built-in brush's name")));
        }
        if self.position(&to).is_some_and(|j| j != i) {
            return Err(EngineError::Param(format!("you already have a brush named '{to}'")));
        }
        if let Some(p) = self.user.get_mut(i) {
            p.name = to.clone();
        }
        Ok(to)
    }

    pub fn delete(&mut self, name: &str) -> Result<BrushPreset> {
        let i = self.position(name).ok_or_else(|| EngineError::Param(format!("you have no brush named '{name}'")))?;
        Ok(self.user.remove(i))
    }

    /// A brush set file holding `presets`.
    pub fn file_json(presets: &[BrushPreset]) -> String {
        let f = BrushFile { format: FILE_FORMAT.into(), version: FILE_VERSION, brushes: presets.iter().filter_map(|p| serde_json::to_value(p).ok()).collect() };
        serde_json::to_string_pretty(&f).unwrap_or_default()
    }

    /// The presets in a brush set file (bad entries skipped; values clamped).
    pub fn parse_file(text: &str) -> Result<Vec<BrushPreset>> {
        let f: BrushFile = serde_json::from_str(text).map_err(|e| EngineError::Param(format!("not a brush set file: {e}")))?;
        if f.format != FILE_FORMAT {
            return Err(EngineError::Param(format!("not a brush set file (its format is '{}')", f.format.chars().take(40).collect::<String>())));
        }
        if f.version > FILE_VERSION {
            return Err(EngineError::Param(format!("this brush set is version {}, newer than this AniCraft reads ({FILE_VERSION})", f.version)));
        }
        Ok(f.brushes.into_iter().take(MAX_USER_PRESETS * 4).filter_map(|v| serde_json::from_value::<BrushPreset>(v).ok()).map(BrushPreset::sanitized).filter(|p| !p.name.is_empty()).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_are_named_and_in_range() {
        for (key, name) in BUILTIN {
            let p = builtin(key).unwrap();
            assert_eq!(p.name, name);
            assert_eq!(p.clone().sanitized(), p, "{key} is already in range");
            assert!(!builtin_hint(key).is_empty());
        }
        assert_eq!(BrushPreset::default().shaping(), Shaping::default(), "the plain brush shapes nothing");
    }

    #[test]
    fn saving_renaming_and_deleting() {
        let mut lib = BrushLibrary::default();
        assert!(!lib.save(BrushPreset { name: "  My ink \n".into(), size: 1e9, ..BrushPreset::default() }).unwrap());
        assert_eq!(lib.user()[0].name, "My ink");
        assert_eq!(lib.user()[0].size, 2000.0, "clamped");
        assert!(lib.save(BrushPreset { name: "my INK".into(), size: 4.0, ..BrushPreset::default() }).unwrap(), "same name replaces");
        assert_eq!(lib.user().len(), 1);
        assert!(lib.save(BrushPreset { name: "Chalk".into(), ..BrushPreset::default() }).is_err(), "a built-in's name");
        assert!(lib.save(BrushPreset { name: " ".into(), ..BrushPreset::default() }).is_err());
        assert_eq!(lib.add_unique(BrushPreset { name: "My ink".into(), ..BrushPreset::default() }).unwrap(), "My ink 2");
        assert_eq!(lib.add_unique(BrushPreset { name: "chalk".into(), ..BrushPreset::default() }).unwrap(), "chalk 2");
        assert_eq!(lib.rename("my ink 2", "Wet").unwrap(), "Wet");
        assert!(lib.rename("Wet", "My Ink").is_err() && lib.rename("Wet", "Marker").is_err() && lib.rename("nope", "x").is_err());
        assert_eq!(lib.find("wet").map(|p| p.name), Some("Wet".into()));
        assert_eq!(lib.find("ink_pen").map(|p| p.name), Some("Ink Pen".into()));
        assert_eq!(lib.find("ink pen").map(|p| p.size), Some(6.0));
        assert_eq!(lib.delete("WET").unwrap().name, "Wet");
        assert!(lib.delete("Wet").is_err());
    }

    #[test]
    fn brush_set_files_round_trip_and_reject_junk() {
        let mine = vec![BrushPreset { name: "A".into(), curve: PressureCurve::SOFT, jitter: 12.0, ..BrushPreset::default() }, builtin("dotted").unwrap()];
        let text = BrushLibrary::file_json(&mine);
        assert_eq!(BrushLibrary::parse_file(&text).unwrap(), mine);
        assert!(BrushLibrary::parse_file("{}").is_err());
        assert!(BrushLibrary::parse_file(r#"{"format": "other", "version": 1, "brushes": []}"#).is_err());
        assert!(BrushLibrary::parse_file(r#"{"format": "anicraft-brushes", "version": 99, "brushes": []}"#).is_err());
        // Bad entries are skipped, out-of-range values clamped, older files without the newer
        // fields read with defaults.
        let got = BrushLibrary::parse_file(r#"{"format": "anicraft-brushes", "version": 1, "brushes": [7, {"name": ""}, {"name": "Old", "size": -4, "spacing": 2, "curve": [[0.5, 9]]}]}"#).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].size, got[0].spacing, got[0].smoothing), (0.1, 5.0, 50.0));
        assert_eq!(got[0].curve.points(), &[[0.0, 1.0], [0.5, 1.0], [1.0, 1.0]]);
        // Preferences: the library as a list.
        let lib = BrushLibrary::from_values(serde_json::from_str::<Vec<Value>>(&serde_json::to_string(&mine).unwrap()).unwrap());
        assert_eq!(lib.user().len(), 2);
        assert_eq!(lib.user()[1].name, "Dotted 2", "a built-in's name is renamed");
    }

    #[test]
    fn params_override_a_preset() {
        let p = builtin("calligraphy").unwrap();
        let q = p.with_params(&serde_json::json!({"size": 30, "curve": "firm", "taper_end": 20})).unwrap();
        assert_eq!((q.size, q.curve, q.taper_end, q.roundness), (30.0, PressureCurve::FIRM, 20.0, 15.0));
        let q = p.with_params(&serde_json::json!({"curve": [[0, 0.2], [1, 0.8]]})).unwrap();
        assert_eq!(q.curve.map(0.0), 0.2);
        for bad in [serde_json::json!({"curve": "wobbly"}), serde_json::json!({"curve": [[0, 2]]}), serde_json::json!({"curve": []}), serde_json::json!({"spacing": 2}), serde_json::json!({"jitter": 60}), serde_json::json!({"shape": "star"})] {
            assert!(p.with_params(&bad).is_err(), "{bad}");
        }
        // The params name every setting and read back as the same preset.
        let back = BrushPreset::default().with_params(&p.params()).unwrap();
        assert_eq!(BrushPreset { name: p.name.clone(), ..back }, p);
    }
}
