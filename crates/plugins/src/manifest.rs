//! The plug-in manifest (`ac_manifest`) and its parameter schema.

use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{Error, Result};

/// Longest manifest the host reads.
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
/// Most parameters a plug-in may declare.
pub const MAX_PARAMS: usize = 32;

/// What a plug-in is (ADR 0017): a command, a frame script (ADR 0024), an export format or a
/// set of eases (ADR 0088), all on the same host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    /// Runs once on demand (a menu item, the palette, `plugin.run`) and edits the document
    /// through commands: generators and selection transforms.
    Command,
    /// Runs while a movie plays, when the playhead enters a keyframe that names one of its
    /// entry points; it reads the player state and emits timeline actions, never edits
    /// (ADR 0024).
    #[serde(rename = "frame-script")]
    FrameScript,
    /// An export format (File › Export, `file.export_plugin`): it is handed the movie's
    /// evaluated frames, never the document, and returns the file's bytes from `ac_export`.
    Exporter,
    /// Named eases, each a pure function of time (`ac_ease`) that AniCraft samples into a table
    /// once, when the plug-in loads; a tween that uses one keeps the table.
    Ease,
}

impl Kind {
    pub fn key(self) -> &'static str {
        match self {
            Kind::Command => "command",
            Kind::FrameScript => "frame-script",
            Kind::Exporter => "exporter",
            Kind::Ease => "ease",
        }
    }
}

/// Most entry points a frame-script plug-in may declare.
pub const MAX_ENTRIES: usize = 32;
/// Most eases one ease plug-in may declare.
pub const MAX_EASES: usize = 32;

/// The file format an exporter plug-in writes (`"format"` in its manifest).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportFormat {
    /// The name File › Export lists ("Plain Text Frames"), 1-64 printable characters.
    pub name: String,
    /// The file extension, without the dot: 1-16 characters from a-z 0-9 (kept lower case).
    pub extension: String,
}

/// One ease an ease plug-in declares (`"eases": [{"id", "name"}, …]`); `ac_ease` gets its
/// index in the list.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EaseDecl {
    /// Stable id within the plug-in, `[A-Za-z0-9_-]{1,32}`.
    pub id: String,
    /// The name the ease menu shows, 1-48 printable characters.
    pub name: String,
}

/// Whether a command plug-in works on the selection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SelectionNeed {
    /// Runs with or without a selection (a generator).
    #[default]
    Optional,
    /// Needs selected elements; menus disable it without them (a selection transform).
    Required,
}

/// One declared parameter: its label in the dialog, and its type.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamSpec {
    pub label: String,
    pub kind: ParamKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ParamKind {
    /// A real number in `min..=max` (a slider in the dialog).
    Number { min: f64, max: f64, default: f64 },
    /// A whole number in `min..=max`.
    Int { min: i64, max: i64, default: i64 },
    /// A checkbox.
    Bool { default: bool },
    /// One of `options` (a dropdown).
    Choice { options: Vec<String>, default: String },
    /// A colour, `#rrggbb` or `#rrggbbaa` (a swatch).
    Color { default: String },
}

/// The manifest a plug-in returns from `ac_manifest` (UTF-8 JSON).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    /// Stable id, `[A-Za-z0-9._-]{1,64}` (reverse-DNS style is recommended: `org.example.stars`).
    pub id: String,
    /// Menu label.
    pub name: String,
    #[serde(default)]
    pub version: String,
    pub kind: Kind,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub selection: SelectionNeed,
    /// Parameters in declaration (dialog) order; a JSON object `{"name": spec, …}` in the manifest.
    #[serde(default, deserialize_with = "ordered_params")]
    pub params: Vec<(String, ParamSpec)>,
    /// A frame-script plug-in's entry points (`["main"]` when it names none), each
    /// `[A-Za-z0-9_]{1,32}`; a keyframe picks one.
    #[serde(default)]
    pub entries: Vec<String>,
    /// An exporter's file format (required for `kind: "exporter"`).
    #[serde(default)]
    pub format: Option<ExportFormat>,
    /// An ease plug-in's eases, 1-32 (required for `kind: "ease"`).
    #[serde(default)]
    pub eases: Vec<EaseDecl>,
}

impl Manifest {
    /// Parses and validates manifest JSON.
    pub fn parse(bytes: &[u8]) -> Result<Manifest> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(Error::Manifest(format!("it is {} bytes (limit {MAX_MANIFEST_BYTES})", bytes.len())));
        }
        let text = std::str::from_utf8(bytes).map_err(|_| Error::Manifest("it is not UTF-8".into()))?;
        let mut man: Manifest = serde_json::from_str(text).map_err(|e| Error::Manifest(e.to_string()))?;
        man.validate()?;
        if man.kind == Kind::FrameScript && man.entries.is_empty() {
            man.entries.push("main".into());
        }
        if let Some(f) = &mut man.format {
            f.name = f.name.trim().to_string();
            f.extension = f.extension.trim().trim_start_matches('.').to_ascii_lowercase();
        }
        for e in &mut man.eases {
            e.name = e.name.trim().to_string();
        }
        man.validate_kind()?;
        Ok(man)
    }

    fn validate(&self) -> Result<()> {
        if !valid_id(&self.id) {
            return Err(Error::Manifest(format!("invalid id {:?}: use 1-64 characters from A-Z a-z 0-9 . _ -", self.id)));
        }
        if !printable(&self.name, 64) {
            return Err(Error::Manifest("`name` must be 1-64 printable characters".into()));
        }
        if self.version.chars().count() > 32 || self.description.chars().count() > 1024 || self.author.chars().count() > 128 {
            return Err(Error::Manifest("`version`, `description` or `author` is too long".into()));
        }
        if [&self.version, &self.description, &self.author].iter().any(|s| s.chars().any(|c| (c.is_control() && c != '\n') || hidden_format(c))) {
            return Err(Error::Manifest("`version`, `description` and `author` must be printable text".into()));
        }
        if self.entries.len() > MAX_ENTRIES {
            return Err(Error::Manifest(format!("at most {MAX_ENTRIES} entry points")));
        }
        for (i, e) in self.entries.iter().enumerate() {
            if !(1..=32).contains(&e.len()) || !e.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                return Err(Error::Manifest(format!("invalid entry point {e:?}: use 1-32 characters from a-z A-Z 0-9 _")));
            }
            if self.entries.iter().take(i).any(|o| o == e) {
                return Err(Error::Manifest(format!("entry point `{e}` is declared twice")));
            }
        }
        for (i, (k, _)) in self.params.iter().enumerate() {
            if self.params.iter().take(i).any(|(o, _)| o == k) {
                return Err(Error::Manifest(format!("parameter `{k}` is declared twice")));
            }
        }
        Ok(())
    }

    /// What only some kinds declare: an exporter's format, an ease plug-in's eases.
    fn validate_kind(&self) -> Result<()> {
        match (&self.format, self.kind) {
            (None, Kind::Exporter) => return Err(Error::Manifest("an exporter needs a `format`: {\"name\": \"…\", \"extension\": \"txt\"}".into())),
            (Some(_), k) if k != Kind::Exporter => return Err(Error::Manifest("only an exporter declares a `format`".into())),
            (Some(f), _) => {
                if !printable(&f.name, 64) {
                    return Err(Error::Manifest("the format's `name` must be 1-64 printable characters".into()));
                }
                if !(1..=16).contains(&f.extension.len()) || !f.extension.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()) {
                    return Err(Error::Manifest(format!("the format's `extension` {:?} must be 1-16 characters from a-z 0-9, without the dot", f.extension)));
                }
            }
            (None, _) => {}
        }
        if self.kind != Kind::Ease {
            if !self.eases.is_empty() {
                return Err(Error::Manifest("only an ease plug-in (`kind: \"ease\"`) declares `eases`".into()));
            }
            return Ok(());
        }
        if self.eases.is_empty() || self.eases.len() > MAX_EASES {
            return Err(Error::Manifest(format!("an ease plug-in declares 1-{MAX_EASES} `eases`: [{{\"id\": \"steps\", \"name\": \"Stepped\"}}, …]")));
        }
        if !self.params.is_empty() {
            return Err(Error::Manifest("an ease plug-in has no `params`: declare one ease per variant instead".into()));
        }
        for (i, e) in self.eases.iter().enumerate() {
            if !(1..=32).contains(&e.id.len()) || !e.id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
                return Err(Error::Manifest(format!("invalid ease id {:?}: use 1-32 characters from a-z A-Z 0-9 _ -", e.id)));
            }
            if !printable(&e.name, 48) {
                return Err(Error::Manifest(format!("ease `{}`: `name` must be 1-48 printable characters", e.id)));
            }
            if self.eases.iter().take(i).any(|o| o.id == e.id) {
                return Err(Error::Manifest(format!("ease `{}` is declared twice", e.id)));
            }
        }
        Ok(())
    }

    /// The parameter notation the command palette and `plugin.list` show
    /// (`{count: int 1..500 = 120, color: color = "#ffffff", mode: "a"|"b"}`).
    pub fn params_notation(&self) -> String {
        let parts: Vec<String> = self
            .params
            .iter()
            .map(|(k, p)| match &p.kind {
                ParamKind::Number { min, max, default } => format!("{k}: {min}..{max} = {default}"),
                ParamKind::Int { min, max, default } => format!("{k}: int {min}..{max} = {default}"),
                ParamKind::Bool { default } => format!("{k}: bool = {default}"),
                ParamKind::Choice { options, default } => format!("{k}: {} = \"{default}\"", options.iter().map(|o| format!("\"{o}\"")).collect::<Vec<_>>().join("|")),
                ParamKind::Color { default } => format!("{k}: color = \"{default}\""),
            })
            .collect();
        format!("{{{}}}", parts.join(", "))
    }

    /// The defaults of every parameter, as the plug-in receives them when none are given.
    pub fn defaults(&self) -> Map<String, Value> {
        self.resolve_params(&Value::Null).unwrap_or_default()
    }

    /// Validates user `params` against the schema: missing keys take their defaults, numbers
    /// are clamped into range (ints rounded), unknown keys are dropped; a value of the wrong
    /// type, an unknown choice or a malformed colour is an error.
    pub fn resolve_params(&self, params: &Value) -> Result<Map<String, Value>> {
        let empty = Map::new();
        let given = match params {
            Value::Null => &empty,
            Value::Object(m) => m,
            _ => return Err(Error::Params("plug-in parameters must be a JSON object".into())),
        };
        let mut out = Map::new();
        for (k, spec) in &self.params {
            let v = given.get(k).filter(|v| !v.is_null());
            let wrong = |what: &str| Error::Params(format!("`{k}` must be {what}"));
            let resolved = match &spec.kind {
                ParamKind::Number { min, max, default } => match v {
                    None => json!(default),
                    Some(v) => json!(v.as_f64().filter(|f| f.is_finite()).ok_or_else(|| wrong("a number"))?.clamp(*min, *max)),
                },
                ParamKind::Int { min, max, default } => match v {
                    None => json!(default),
                    Some(v) => {
                        let f = v.as_f64().filter(|f| f.is_finite()).ok_or_else(|| wrong("a number"))?;
                        json!(f.round().clamp(*min as f64, *max as f64) as i64)
                    }
                },
                ParamKind::Bool { default } => match v {
                    None => json!(default),
                    Some(v) => json!(v.as_bool().ok_or_else(|| wrong("true or false"))?),
                },
                ParamKind::Choice { options, default } => match v {
                    None => json!(default),
                    Some(v) => {
                        let s = v.as_str().ok_or_else(|| wrong("a string"))?;
                        if !options.iter().any(|o| o == s) {
                            return Err(Error::Params(format!("`{k}` must be one of {}", options.join(", "))));
                        }
                        json!(s)
                    }
                },
                ParamKind::Color { default } => match v {
                    None => json!(default),
                    Some(v) => {
                        let s = v.as_str().filter(|s| valid_color(s)).ok_or_else(|| wrong("a colour like \"#ff8800\""))?;
                        json!(s.to_ascii_lowercase())
                    }
                },
            };
            out.insert(k.clone(), resolved);
        }
        Ok(out)
    }
}

/// Reads the `params` object keeping its key order (serde_json's `Map` would sort it).
fn ordered_params<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Vec<(String, ParamSpec)>, D::Error> {
    struct V;
    impl<'de> serde::de::Visitor<'de> for V {
        type Value = Vec<(String, ParamSpec)>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("an object of parameter specs")
        }
        fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
            Ok(Vec::new())
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> std::result::Result<Self::Value, A::Error> {
            let mut out = Vec::new();
            while let Some((k, v)) = map.next_entry::<String, Value>()? {
                if out.len() >= MAX_PARAMS {
                    return Err(serde::de::Error::custom(format!("too many parameters (limit {MAX_PARAMS})")));
                }
                let spec = param_spec(&k, &v).map_err(serde::de::Error::custom)?;
                out.push((k, spec));
            }
            Ok(out)
        }
    }
    d.deserialize_any(V)
}

/// One parameter spec from its JSON object.
fn param_spec(name: &str, v: &Value) -> std::result::Result<ParamSpec, String> {
    if !valid_key(name) {
        return Err(format!("invalid parameter name {name:?}: use 1-32 characters from a-z A-Z 0-9 _, not starting with _ (and not `id`)"));
    }
    let bad = |what: &str| format!("parameter `{name}`: {what}");
    let obj = v.as_object().ok_or_else(|| bad("must be an object like {\"type\": \"number\", …}"))?;
    let label = match obj.get("label") {
        None | Some(Value::Null) => humanize(name),
        Some(Value::String(s)) if printable(s, 48) => s.trim().to_string(),
        Some(_) => return Err(bad("`label` must be 1-48 printable characters")),
    };
    let num = |key: &str| -> std::result::Result<Option<f64>, String> {
        match obj.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v.as_f64().filter(|f| f.is_finite() && f.abs() <= 1e12).map(Some).ok_or_else(|| bad(&format!("`{key}` must be a finite number"))),
        }
    };
    let kind = match obj.get("type").and_then(Value::as_str) {
        Some("number") => {
            let (min, max) = (num("min")?.unwrap_or(0.0), num("max")?.unwrap_or(1.0));
            if min > max {
                return Err(bad("`min` is above `max`"));
            }
            ParamKind::Number { min, max, default: num("default")?.unwrap_or(min).clamp(min, max) }
        }
        Some("int") => {
            let (min, max) = (num("min")?.unwrap_or(0.0).round() as i64, num("max")?.unwrap_or(1_000_000.0).round() as i64);
            if min > max {
                return Err(bad("`min` is above `max`"));
            }
            ParamKind::Int { min, max, default: (num("default")?.unwrap_or(min as f64).round() as i64).clamp(min, max) }
        }
        Some("bool") => ParamKind::Bool { default: obj.get("default").and_then(Value::as_bool).unwrap_or(false) },
        Some("choice") => {
            let options: Vec<String> = obj.get("options").and_then(Value::as_array).map(|a| a.iter().filter_map(|o| o.as_str().map(str::to_string)).collect()).unwrap_or_default();
            if options.is_empty() || options.len() > 32 || options.iter().any(|o| !printable(o, 48)) {
                return Err(bad("`options` must be 1-32 printable strings of up to 48 characters"));
            }
            let default = match obj.get("default").and_then(Value::as_str) {
                Some(d) if options.iter().any(|o| o == d) => d.to_string(),
                Some(_) => return Err(bad("`default` is not one of the options")),
                None => options.first().cloned().unwrap_or_default(),
            };
            ParamKind::Choice { options, default }
        }
        Some("color") => {
            let default = obj.get("default").and_then(Value::as_str).unwrap_or("#000000");
            if !valid_color(default) {
                return Err(bad("`default` must be a colour like \"#ff8800\""));
            }
            ParamKind::Color { default: default.to_ascii_lowercase() }
        }
        _ => return Err(bad("`type` must be number, int, bool, choice or color")),
    };
    Ok(ParamSpec { label, kind })
}

/// `count_max` → "Count max".
fn humanize(key: &str) -> String {
    let s = key.replace('_', " ");
    let mut c = s.chars();
    c.next().map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

/// `[A-Za-z0-9._-]{1,64}`.
pub fn valid_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

fn valid_key(k: &str) -> bool {
    (1..=32).contains(&k.len()) && !k.starts_with('_') && k != "id" && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// `#rrggbb` or `#rrggbbaa`.
pub fn valid_color(s: &str) -> bool {
    s.strip_prefix('#').is_some_and(|h| (h.len() == 6 || h.len() == 8) && h.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// 1..=`max` characters, none of them control or invisible format characters, at least one
/// visible: a name that can't spoof or hide in a menu.
fn printable(s: &str, max: usize) -> bool {
    let s = s.trim();
    let n = s.chars().count();
    (1..=max).contains(&n) && s.chars().all(|c| !c.is_control() && !hidden_format(c)) && s.chars().any(|c| !c.is_whitespace())
}

/// Unicode format characters that change or hide text (bidi overrides, zero-width spaces,
/// soft hyphens), except the zero-width (non-)joiner that some scripts need.
fn hidden_format(c: char) -> bool {
    matches!(c, '\u{00ad}' | '\u{061c}' | '\u{180e}' | '\u{200b}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}' | '\u{fff9}'..='\u{fffb}')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn man(params: &str) -> Result<Manifest> {
        Manifest::parse(format!(r#"{{"id":"org.test.x","name":"X","kind":"command","params":{params}}}"#).as_bytes())
    }

    #[test]
    fn params_keep_their_order_and_resolve_with_defaults_and_clamping() {
        let m = man(r##"{"zeta":{"type":"int","min":1,"max":10,"default":3},"alpha":{"type":"number","min":0,"max":1,"default":0.5,"label":"Opacity"},"on":{"type":"bool"},"mode":{"type":"choice","options":["a","b"]},"tint":{"type":"color","default":"#FF8800"}}"##).unwrap();
        assert_eq!(m.params.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["zeta", "alpha", "on", "mode", "tint"]);
        assert_eq!(m.params[0].1.label, "Zeta");
        assert_eq!(m.params[1].1.label, "Opacity");
        let d = m.defaults();
        assert_eq!(d["zeta"], 3);
        assert_eq!(d["mode"], "a");
        assert_eq!(d["tint"], "#ff8800");
        let r = m.resolve_params(&json!({"zeta": 99.6, "alpha": -3, "on": true, "mode": "b", "tint": "#00ff00", "extra": 1})).unwrap();
        assert_eq!(r["zeta"], 10);
        assert_eq!(r["alpha"], 0.0);
        assert!(!r.contains_key("extra"));
        assert!(m.resolve_params(&json!({"mode": "c"})).is_err());
        assert!(m.resolve_params(&json!({"tint": "red"})).is_err());
        assert!(m.resolve_params(&json!({"on": 1})).is_err());
        assert!(m.resolve_params(&json!([1])).is_err());
        assert!(m.params_notation().starts_with("{zeta: int 1..10 = 3, alpha: 0..1 = 0.5"));
    }

    #[test]
    fn bad_manifests_are_rejected_with_a_reason() {
        let bad = [
            r#"{"id":"","name":"X","kind":"command"}"#,
            r#"{"id":"a b","name":"X","kind":"command"}"#,
            r#"{"id":"a","name":"","kind":"command"}"#,
            "{\"id\":\"a\",\"name\":\"\\u202eevil\",\"kind\":\"command\"}",
            r#"{"id":"a","name":"X","kind":"filter"}"#,
            r#"{"id":"a","name":"X"}"#,
            r#"{"id":"a","name":"X","kind":"command","params":{"_x":{"type":"int"}}}"#,
            r#"{"id":"a","name":"X","kind":"command","params":{"x":{"type":"int","min":5,"max":1}}}"#,
            r#"{"id":"a","name":"X","kind":"command","params":{"x":{"type":"wat"}}}"#,
            r#"{"id":"a","name":"X","kind":"command","params":{"x":{"type":"choice","options":[]}}}"#,
            r#"{"id":"a","name":"X","kind":"command","params":{"x":{"type":"color","default":"blue"}}}"#,
            r#"{"id":"a","name":"X","kind":"command","params":{"x":{"type":"number","min":1e400}}}"#,
            r#"{"id":"a","name":"X","kind":"command","params":{"x":{"type":"int"},"x":{"type":"int"}}}"#,
            "not json",
        ];
        for b in bad {
            assert!(Manifest::parse(b.as_bytes()).is_err(), "accepted {b}");
        }
        assert!(Manifest::parse(&[0xff, 0xfe]).is_err());
        for bad in [r#"{"id":"a","name":"X","kind":"frame-script","entries":["a b"]}"#, r#"{"id":"a","name":"X","kind":"frame-script","entries":["a","a"]}"#] {
            assert!(Manifest::parse(bad.as_bytes()).is_err(), "accepted {bad}");
        }
        let fs = Manifest::parse(br#"{"id":"a","name":"X","kind":"frame-script"}"#).unwrap();
        assert_eq!((fs.kind, fs.entries.as_slice()), (Kind::FrameScript, ["main".to_string()].as_slice()));
        assert!(Manifest::parse(&vec![b' '; MAX_MANIFEST_BYTES + 1]).is_err());
        let many: Vec<String> = (0..=MAX_PARAMS).map(|i| format!("\"p{i}\":{{\"type\":\"bool\"}}")).collect();
        assert!(man(&format!("{{{}}}", many.join(","))).is_err());
    }
}
