//! Filter presets (ADR 0091): named filter lists of the user's own, as Animate's Filters panel
//! keeps them (Save as Preset, apply, rename, delete).
//!
//! Like the user's brushes (ADR 0068) the presets belong to the session, not the document: they
//! survive New and Open, the desktop app keeps them in its preferences and puts them back at
//! start (so the engine never reads the settings folder), and they move between computers as a
//! small JSON file (`filter.preset_export`/`_import`). Applying one copies its list onto the
//! instances: a preset edited or deleted later never changes artwork behind the artist's back.

use crate::{EngineError, Result};
use anicraft_doc::FilterEntry;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Most presets.
pub const MAX_PRESETS: usize = 200;
/// Longest preset name, in characters.
pub const MAX_NAME: usize = 64;
/// Most filters on one instance (and so in one preset).
pub const MAX_FILTERS: usize = 16;
/// Largest preset file read.
pub const MAX_FILE_BYTES: u64 = 1 << 20;
/// What a preset file says it is.
pub const FILE_FORMAT: &str = "anicraft-filter-presets";
pub const FILE_VERSION: u32 = 1;

/// A named filter list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FilterPreset {
    pub name: String,
    #[serde(default)]
    pub filters: Vec<FilterEntry>,
}

impl FilterPreset {
    /// The preset with a clean name and its filters in range (at most [`MAX_FILTERS`]); `None`
    /// when the name is empty.
    pub fn sanitized(self) -> Option<FilterPreset> {
        let name = clean_name(&self.name)?;
        Some(FilterPreset { name, filters: self.filters.iter().take(MAX_FILTERS).map(FilterEntry::sanitized).collect() })
    }
}

/// A name as kept: one line, trimmed, at most [`MAX_NAME`] characters; `None` when empty.
pub fn clean_name(s: &str) -> Option<String> {
    let one: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let t: String = one.trim().chars().take(MAX_NAME).collect();
    let t = t.trim().to_string();
    (!t.is_empty()).then_some(t)
}

fn same_name(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

/// The user's filter presets, in the order they were saved; names are unique, case aside.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FilterLibrary {
    presets: Vec<FilterPreset>,
}

#[derive(Serialize, Deserialize)]
struct PresetFile {
    format: String,
    version: u32,
    presets: Vec<Value>,
}

impl FilterLibrary {
    pub fn presets(&self) -> &[FilterPreset] {
        &self.presets
    }

    pub fn is_empty(&self) -> bool {
        self.presets.is_empty()
    }

    /// A library from untrusted entries (preferences, a file): bad entries skipped, values
    /// clamped, names made unique, at most [`MAX_PRESETS`].
    pub fn from_values(values: impl IntoIterator<Item = Value>) -> FilterLibrary {
        let mut lib = FilterLibrary::default();
        for v in values.into_iter().take(MAX_PRESETS * 4) {
            if let Ok(p) = serde_json::from_value::<FilterPreset>(v) {
                let _ = lib.add_unique(p);
            }
        }
        lib
    }

    /// The same library with every entry sanitized (after deserializing preferences).
    pub fn sanitized(self) -> FilterLibrary {
        FilterLibrary::from_values(self.presets.into_iter().filter_map(|p| serde_json::to_value(p).ok()))
    }

    pub fn position(&self, name: &str) -> Option<usize> {
        self.presets.iter().position(|p| same_name(&p.name, name))
    }

    pub fn find(&self, name: &str) -> Option<&FilterPreset> {
        self.position(name).and_then(|i| self.presets.get(i))
    }

    fn missing(name: &str) -> EngineError {
        EngineError::Param(format!("you have no filter preset named '{}'", name.chars().take(MAX_NAME).collect::<String>()))
    }

    /// Saves `filters` under `name`, replacing the preset of that name. Returns the name kept
    /// and whether it replaced one.
    pub fn save(&mut self, name: &str, filters: &[FilterEntry]) -> Result<(String, bool)> {
        let p = FilterPreset { name: name.to_string(), filters: filters.to_vec() }.sanitized().ok_or_else(|| EngineError::Param("a filter preset needs a name".into()))?;
        if p.filters.is_empty() {
            return Err(EngineError::State("there are no filters to save; add one first".into()));
        }
        let name = p.name.clone();
        if let Some(i) = self.position(&name)
            && let Some(slot) = self.presets.get_mut(i)
        {
            // The name as written now (the case may differ).
            *slot = p;
            return Ok((name, true));
        }
        if self.presets.len() >= MAX_PRESETS {
            return Err(EngineError::State(format!("you have {MAX_PRESETS} filter presets already; delete some first")));
        }
        self.presets.push(p);
        Ok((name, false))
    }

    /// Adds `p`, renaming it ("Glow 2", "Glow 3", ..) if the name is taken. Returns its name.
    pub fn add_unique(&mut self, p: FilterPreset) -> Result<String> {
        let p = p.sanitized().ok_or_else(|| EngineError::Param("a filter preset needs a name".into()))?;
        if self.presets.len() >= MAX_PRESETS {
            return Err(EngineError::State(format!("you have {MAX_PRESETS} filter presets already; delete some first")));
        }
        let base = p.name.clone();
        let mut name = base.clone();
        let mut k = 2;
        while self.position(&name).is_some() {
            if k > MAX_PRESETS + 10 {
                return Err(EngineError::State("too many filter presets of that name".into()));
            }
            let suffix = format!(" {k}");
            name = format!("{}{suffix}", base.chars().take(MAX_NAME.saturating_sub(suffix.chars().count())).collect::<String>().trim_end());
            k += 1;
        }
        self.presets.push(FilterPreset { name: name.clone(), ..p });
        Ok(name)
    }

    pub fn rename(&mut self, name: &str, to: &str) -> Result<String> {
        let i = self.position(name).ok_or_else(|| Self::missing(name))?;
        let to = clean_name(to).ok_or_else(|| EngineError::Param("a filter preset needs a name".into()))?;
        if self.position(&to).is_some_and(|j| j != i) {
            return Err(EngineError::Param(format!("you already have a filter preset named '{to}'")));
        }
        if let Some(p) = self.presets.get_mut(i) {
            p.name = to.clone();
        }
        Ok(to)
    }

    pub fn delete(&mut self, name: &str) -> Result<FilterPreset> {
        let i = self.position(name).ok_or_else(|| Self::missing(name))?;
        Ok(self.presets.remove(i))
    }

    /// A preset file holding `presets`.
    pub fn file_json(presets: &[FilterPreset]) -> String {
        let f = PresetFile { format: FILE_FORMAT.into(), version: FILE_VERSION, presets: presets.iter().filter_map(|p| serde_json::to_value(p).ok()).collect() };
        serde_json::to_string_pretty(&f).unwrap_or_default()
    }

    /// The presets in a preset file (bad entries skipped; values clamped).
    pub fn parse_file(text: &str) -> Result<Vec<FilterPreset>> {
        let f: PresetFile = serde_json::from_str(text).map_err(|e| EngineError::Param(format!("not a filter preset file: {e}")))?;
        if f.format != FILE_FORMAT {
            return Err(EngineError::Param(format!("not a filter preset file (its format is '{}')", f.format.chars().take(40).collect::<String>())));
        }
        if f.version > FILE_VERSION {
            return Err(EngineError::Param(format!("this filter preset file is version {}, newer than this AniCraft reads ({FILE_VERSION})", f.version)));
        }
        Ok(f.presets.into_iter().take(MAX_PRESETS * 4).filter_map(|v| serde_json::from_value::<FilterPreset>(v).ok()).filter_map(FilterPreset::sanitized).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anicraft_doc::Filter;

    fn blur(b: f32) -> FilterEntry {
        Filter::Blur { blur_x: b, blur_y: b, quality: 1 }.into()
    }

    #[test]
    fn saving_renaming_and_deleting() {
        let mut lib = FilterLibrary::default();
        assert_eq!(lib.save("  Soft \n glow ", &[blur(4.0)]).unwrap(), ("Soft   glow".to_string(), false));
        assert!(lib.save("soft   GLOW", &[blur(8.0), blur(1e9)]).unwrap().1, "same name, case aside, replaces");
        assert_eq!(lib.presets().len(), 1);
        assert_eq!(lib.presets()[0].filters[1], blur(anicraft_doc::MAX_BLUR), "values clamped");
        assert!(lib.save(" ", &[blur(1.0)]).is_err());
        assert!(lib.save("Empty", &[]).is_err(), "nothing to save");
        assert_eq!(lib.add_unique(FilterPreset { name: "soft   glow".into(), filters: vec![blur(1.0)] }).unwrap(), "soft   glow 2");
        assert_eq!(lib.rename("SOFT   GLOW 2", "Haze").unwrap(), "Haze");
        assert!(lib.rename("Haze", "Soft   Glow").is_err() && lib.rename("nope", "x").is_err() && lib.rename("Haze", "  ").is_err());
        assert_eq!(lib.rename("haze", "HAZE").unwrap(), "HAZE", "a new case of its own name");
        assert_eq!(lib.find("haze").map(|p| p.filters.len()), Some(1));
        assert_eq!(lib.delete("haze").unwrap().name, "HAZE");
        assert!(lib.delete("haze").is_err());
        let long = "x".repeat(200);
        assert_eq!(lib.save(&long, &[blur(1.0)]).unwrap().0.chars().count(), MAX_NAME);
    }

    #[test]
    fn preset_files_round_trip_and_reject_junk() {
        let off = FilterEntry { filter: Filter::new("glow").unwrap(), enabled: false };
        let mine = vec![FilterPreset { name: "A".into(), filters: vec![blur(3.0), off] }, FilterPreset { name: "B".into(), filters: vec![Filter::new("bevel").unwrap().into()] }];
        let text = FilterLibrary::file_json(&mine);
        assert!(text.contains("\"enabled\": false"), "a switched-off filter keeps its switch: {text}");
        assert_eq!(FilterLibrary::parse_file(&text).unwrap(), mine);
        assert!(FilterLibrary::parse_file("{}").is_err());
        assert!(FilterLibrary::parse_file(r#"{"format": "anicraft-brushes", "version": 1, "presets": []}"#).is_err());
        assert!(FilterLibrary::parse_file(r#"{"format": "anicraft-filter-presets", "version": 99, "presets": []}"#).is_err());
        // Bad entries are skipped, values clamped, bare filters take their defaults.
        let got = FilterLibrary::parse_file(r#"{"format": "anicraft-filter-presets", "version": 1, "presets": [7, {"name": ""}, {"name": "x", "filters": [{"type": "sparkle"}]}, {"name": "Old", "filters": [{"type": "blur", "blur_x": -4}, {"type": "glow"}]}]}"#).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].filters[0], Filter::Blur { blur_x: 0.0, blur_y: 5.0, quality: 1 }.into());
        assert!(matches!(got[0].filters[1], FilterEntry { filter: Filter::Glow { blur_x: 5.0, .. }, enabled: true }));
        // Preferences: the library as a list, names made unique.
        let lib = FilterLibrary::from_values(serde_json::from_str::<Vec<Value>>(&serde_json::to_string(&[mine[0].clone(), mine[0].clone()]).unwrap()).unwrap());
        assert_eq!(lib.presets().iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["A", "A 2"]);
        assert_eq!(lib.clone().sanitized(), lib);
    }
}
