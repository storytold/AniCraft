//! The `.anicraft` native format: a versioned JSON envelope around the document.

use crate::EngineError;
use anicraft_doc::{Document, path_table};
use anicraft_geom::Path;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const FORMAT_NAME: &str = "anicraft";
/// v1: inline tagged paths. v2: SVG path strings, shared paths stored once in `paths`.
/// v3: `scenes` (a list of named timelines) instead of one `timeline`; v1/v2 files load as one
/// scene.
/// v4: a document may carry the `maop_maohou` sidecar (the Moho import's untouched source
/// JSON, dual-write rule: standard document keys stay standard, Moho-only data rides beside
/// them); older files simply have no sidecar.
/// v5: property channels (ADR 0102) — each layer's own transform (translation, scale,
/// rotation, shear, flips, origin pivot, camera_immune) and each scene's camera, timeline
/// markers and playback frame range. All optional keys: older files read as before, with no
/// layer transform and the plain unmoving camera.
pub const FORMAT_VERSION: u32 = 5;
/// Largest file we will parse.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024 * 1024;
/// Files smaller than this (compact) are pretty-printed for humans and diffs.
const PRETTY_LIMIT: usize = 1024 * 1024;

/// Written in this field order: the path table precedes the document, so a reader can stream
/// the file in one pass with references already resolvable.
#[derive(Serialize)]
struct EnvelopeOut<'a> {
    format: &'a str,
    version: u32,
    generator: String,
    paths: &'a [Arc<Path>],
    document: &'a Document,
}

/// Streaming reader: when `paths` arrives it opens a path-table scope for `document`.
struct EnvelopeIn {
    format: String,
    version: u32,
    document: Option<Document>,
}

impl<'de> Deserialize<'de> for EnvelopeIn {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = EnvelopeIn;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an .anicraft envelope object")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<EnvelopeIn, A::Error> {
                let (mut format, mut version, mut document) = (None, None, None);
                let mut _scope = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "format" => format = Some(map.next_value::<String>()?),
                        "version" => version = Some(map.next_value::<u32>()?),
                        "paths" => {
                            let paths: Vec<Path> = map.next_value()?;
                            _scope = Some(path_table::begin_read(paths.into_iter().map(Arc::new).collect()));
                        }
                        "document" => document = Some(map.next_value::<Document>()?),
                        _ => {
                            map.next_value::<serde::de::IgnoredAny>()?;
                        }
                    }
                }
                Ok(EnvelopeIn {
                    format: format.ok_or_else(|| serde::de::Error::missing_field("format"))?,
                    version: version.ok_or_else(|| serde::de::Error::missing_field("version"))?,
                    document,
                })
            }
        }
        d.deserialize_map(V)
    }
}

pub fn to_json(doc: &Document) -> Result<String, EngineError> {
    let paths = path_table::build_table(doc);
    let env = EnvelopeOut { format: FORMAT_NAME, version: FORMAT_VERSION, generator: format!("AniCraft {}", env!("CARGO_PKG_VERSION")), paths: &paths, document: doc };
    let compact = path_table::write_with_paths(&paths, || serde_json::to_string(&env)).map_err(|e| EngineError::Format(e.to_string()))?;
    if compact.len() < PRETTY_LIMIT {
        return path_table::write_with_paths(&paths, || serde_json::to_string_pretty(&env)).map_err(|e| EngineError::Format(e.to_string()));
    }
    Ok(compact)
}

pub fn from_json(text: &str) -> Result<Document, EngineError> {
    let env: EnvelopeIn = serde_json::from_str(text).map_err(|e| EngineError::Format(format!("not a valid .anicraft file: {e}")))?;
    if env.format != FORMAT_NAME {
        return Err(EngineError::Format(format!("unexpected format '{}'", env.format)));
    }
    if env.version > FORMAT_VERSION {
        return Err(EngineError::Format(format!("file version {} is newer than this AniCraft (supports {FORMAT_VERSION})", env.version)));
    }
    let mut doc = env.document.ok_or_else(|| EngineError::Format("the file has no document".into()))?;
    doc.sanitize();
    Ok(doc)
}

pub fn load(path: &std::path::Path) -> Result<Document, EngineError> {
    let len = crate::vfs::len(path)?;
    if len > MAX_FILE_BYTES {
        return Err(EngineError::Format(format!("file is {len} bytes; the limit is {MAX_FILE_BYTES}")));
    }
    from_json(&crate::vfs::read_to_string(path)?)
}

pub fn save(doc: &Document, path: &std::path::Path) -> Result<(), EngineError> {
    let json = to_json(doc)?;
    // Write next to the target, then rename: a crash mid-write never corrupts the old file.
    let tmp = path.with_extension("anicraft.tmp");
    crate::vfs::write(&tmp, json)?;
    crate::vfs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anicraft_doc::sample;

    #[test]
    fn round_trip_is_lossless() {
        let d = sample::demo();
        let back = from_json(&to_json(&d).unwrap()).unwrap();
        assert_eq!(d, back);
    }

    #[test]
    fn maop_maohou_sidecar_round_trips_and_v3_files_load_without_one() {
        use anicraft_doc::MaohouSidecar;
        let mut d = sample::demo();
        d.maop_maohou = Some(Arc::new(MaohouSidecar { schema: 1, saved_by: "AniCraft 0.1.0".into(), source: "mohoproj 1045".into(), raw: serde_json::json!({"layers": [1, 2, 3]}) }));
        let back = from_json(&to_json(&d).unwrap()).unwrap();
        assert_eq!(d, back, "the sidecar survives save/load whole");
        // A v3 file (no sidecar) still loads as before.
        let v3 = r#"{"format":"anicraft","version":3,"document":{"stage":{"width":100,"height":80,"fps":12.0,"background":{"r":1.0,"g":1.0,"b":1.0,"a":1.0}}},"next_id":1}"#;
        let old = from_json(v3).unwrap();
        assert!(old.maop_maohou.is_none());
        assert_eq!(old.stage.width, 100);
    }

    #[test]
    fn shared_paths_are_written_once_and_shared_again_on_load() {
        let d = anicraft_doc::sample::stress(2, 40, 48); // 4 keyframes per element share a path
        let json = to_json(&d).unwrap();
        let env: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(env["version"], FORMAT_VERSION);
        assert_eq!(env["paths"].as_array().unwrap().len(), 80, "one entry per distinct path");
        let back = from_json(&json).unwrap();
        assert_eq!(d, back);
        let l = &back.timeline(0).layers[0];
        let (anicraft_doc::ElementKind::Shape(a), anicraft_doc::ElementKind::Shape(b)) = (&l.keyframes[0].elements[0].kind, &l.keyframes[3].elements[0].kind) else { panic!() };
        assert!(Arc::ptr_eq(&a.path, &b.path), "sharing restored on load");
    }

    #[test]
    fn version_1_files_still_load() {
        let v1 = r#"{"format":"anicraft","version":1,"document":{"timeline":{"layers":[{"id":1,"name":"L","keyframes":[{"start":0,"duration":2,"elements":[
            {"id":2,"kind":{"Shape":{"path":{"cmds":[{"MoveTo":{"x":0.0,"y":0.0}},{"LineTo":{"x":10.0,"y":0.0}},{"LineTo":{"x":10.0,"y":10.0}},"Close"]},"fill":{"Solid":{"r":1.0,"g":0.0,"b":0.0,"a":1.0}}}}}]}]}]},"next_id":3}}"#;
        let d = from_json(v1).unwrap();
        assert_eq!(d.frame_count(), 2);
        assert_eq!(anicraft_doc::evaluate(&d, 1).items.len(), 1);
    }

    #[test]
    fn bad_path_references_are_errors() {
        let bad = r#"{"format":"anicraft","version":2,"paths":["M0 0L1 1Z"],"document":{"timeline":{"layers":[{"id":1,"name":"L","keyframes":[{"start":0,"duration":1,"elements":[
            {"id":2,"kind":{"Shape":{"path":7}}}]}]}]}}}"#;
        assert!(from_json(bad).is_err());
        assert!(from_json(r#"{"format":"anicraft","version":2,"paths":["M0 0 Q"],"document":{}}"#).is_err());
    }

    #[test]
    fn rejects_garbage_and_future_versions() {
        assert!(from_json("{").is_err());
        assert!(from_json(r#"{"format":"other","version":1,"document":{}}"#).is_err());
        assert!(from_json(r#"{"format":"anicraft","version":99,"document":{}}"#).is_err());
    }

    #[test]
    fn missing_fields_default() {
        let d = from_json(r#"{"format":"anicraft","version":1,"document":{"stage":{"width":0,"height":50,"fps":-3,"background":{"r":2,"g":0,"b":0,"a":1}}}}"#).unwrap();
        assert_eq!(d.stage.width, 1);
        assert_eq!(d.stage.fps, 24.0);
        assert_eq!(d.stage.background.r, 1.0);
    }
}
