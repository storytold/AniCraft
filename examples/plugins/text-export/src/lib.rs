//! Plain Text Frames: an exporter plug-in. It writes the movie as a small, readable text file,
//! one block per frame: the shapes on it, where they are and how they are painted, and with
//! Detail › Paths their outlines as SVG path data. Handy for diffing animations in version
//! control, and a template for any export format: AniCraft hands it evaluated frames and
//! writes the bytes it returns.

use anicraft_plugin_sdk as ac;
use serde_json::Value;
use std::fmt::Write as _;

const MANIFEST: &str = r#"{
  "id": "org.anicraft.text-export",
  "name": "Plain Text Frames",
  "version": "1.0.0",
  "kind": "exporter",
  "author": "AniCraft contributors",
  "description": "The movie as readable text, one block per frame: each shape, its position and paint (and outline, with Paths).",
  "format": {"name": "Plain Text Frames", "extension": "txt"},
  "params": {
    "detail": {"type": "choice", "options": ["Summary", "Shapes", "Paths"], "default": "Shapes", "label": "Detail"},
    "every":  {"type": "int", "min": 1, "max": 100, "default": 1, "label": "Every nth frame"}
  }
}"#;

ac::exporter!(MANIFEST, export);

fn export(options: &Value) -> Result<Vec<u8>, String> {
    let movie = ac::movie()?;
    let frames = movie["frames"].as_u64().unwrap_or(0);
    let every = options["every"].as_u64().unwrap_or(1).max(1);
    let mut list = Vec::new();
    let mut i = 0;
    while i < frames {
        list.push(ac::frame(i)?);
        i += every;
    }
    Ok(text(&movie, &list, Detail::from(options["detail"].as_str().unwrap_or("Shapes"))).into_bytes())
}

/// How much of each frame the text shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Detail {
    Summary,
    Shapes,
    Paths,
}

impl Detail {
    fn from(s: &str) -> Detail {
        match s {
            "Summary" => Detail::Summary,
            "Paths" => Detail::Paths,
            _ => Detail::Shapes,
        }
    }
}

fn num(v: &Value) -> String {
    let f = v.as_f64().unwrap_or(0.0);
    let r = (f * 100.0).round() / 100.0;
    if r == r.trunc() { format!("{}", r as i64) } else { format!("{r}") }
}

/// A paint in a few words: `#ff8800ff`, `linear gradient (3 stops)`, `bitmap 2`, `none`.
fn paint(p: &Value) -> String {
    match p["type"].as_str() {
        Some("solid") => p["color"].as_str().unwrap_or("?").to_string(),
        Some(kind @ ("linear" | "radial")) => format!("{kind} gradient ({} stops)", p["stops"].as_array().map_or(0, Vec::len)),
        Some("bitmap") => format!("bitmap {}", num(&p["bitmap"])),
        _ => "none".to_string(),
    }
}

/// The whole file: a header, then one block per frame.
fn text(movie: &Value, frames: &[Value], detail: Detail) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# {} — {} × {} px at {} fps, frames {}–{} of {}", movie["name"].as_str().unwrap_or("Untitled"), num(&movie["width"]), num(&movie["height"]), num(&movie["fps"]), movie["from"].as_u64().unwrap_or(0) + 1, movie["to"].as_u64().unwrap_or(0) + 1, movie["total_frames"].as_u64().unwrap_or(0));
    let _ = writeln!(out, "# background {}", movie["background"].as_str().unwrap_or("?"));
    for f in frames {
        let items = f["items"].as_array().map_or(&[][..], Vec::as_slice);
        let _ = writeln!(out, "\nframe {} (scene {}): {} shape{}", f["frame"].as_u64().unwrap_or(0) + 1, f["scene"].as_u64().unwrap_or(0) + 1, items.len(), if items.len() == 1 { "" } else { "s" });
        if detail == Detail::Summary {
            continue;
        }
        for it in items {
            let m = &it["transform"];
            let stroke = if it["stroke"].is_null() { "none".to_string() } else { format!("{} {}px", it["stroke"]["color"].as_str().unwrap_or("?"), num(&it["stroke"]["width"])) };
            let role = if it["masks"].is_null() { "" } else { " (mask)" };
            let _ = writeln!(out, "  element {} on layer {}{role}: at ({}, {}) fill {} stroke {} alpha {}", num(&it["element"]), num(&it["layer"]), num(&m[4]), num(&m[5]), paint(&it["fill"]), stroke, num(&it["alpha"]));
            if detail == Detail::Paths {
                let _ = writeln!(out, "    {}", it["path"].as_str().unwrap_or(""));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn movie() -> Value {
        json!({"name": "walk", "width": 550, "height": 400, "fps": 24, "background": "#ffffffff", "from": 0, "to": 1, "frames": 2, "total_frames": 2})
    }

    fn frame(i: u64) -> Value {
        json!({"index": i, "frame": i, "scene": 0, "items": [
            {"layer": 1, "element": 7, "path": "M0 0L10 0L10 10Z", "transform": [1, 0, 0, 1, 12.5, 40.004], "fill": {"type": "solid", "color": "#ff8800ff"}, "stroke": {"color": "#000000ff", "width": 2}, "alpha": 1},
            {"layer": 2, "element": 9, "path": "M0 0L5 5", "transform": [1, 0, 0, 1, 0, 0], "fill": {"type": "linear", "stops": [[0, "#000000ff"], [1, "#ffffffff"]]}, "stroke": null, "alpha": 0.5, "masks": 0}
        ]})
    }

    #[test]
    fn the_text_lists_each_frame_and_shape() {
        let t = text(&movie(), &[frame(0), frame(1)], Detail::Shapes);
        assert!(t.starts_with("# walk — 550 × 400 px at 24 fps, frames 1–2 of 2\n"), "{t}");
        assert!(t.contains("\nframe 2 (scene 1): 2 shapes\n"), "{t}");
        assert!(t.contains("  element 7 on layer 1: at (12.5, 40) fill #ff8800ff stroke #000000ff 2px alpha 1\n"), "{t}");
        assert!(t.contains("  element 9 on layer 2 (mask): at (0, 0) fill linear gradient (2 stops) stroke none alpha 0.5\n"), "{t}");
        assert!(!t.contains("M0 0"));
        assert!(text(&movie(), &[frame(0)], Detail::Paths).contains("    M0 0L10 0L10 10Z\n"));
        let summary = text(&movie(), &[frame(0)], Detail::Summary);
        assert!(summary.ends_with("frame 1 (scene 1): 2 shapes\n") && !summary.contains("element"), "{summary}");
    }

    #[test]
    fn odd_input_never_panics() {
        let t = text(&json!({}), &[json!({}), json!({"items": [{}]}), json!(null)], Detail::Paths);
        assert!(t.contains("frame 1 (scene 1): 0 shapes"));
        assert_eq!(paint(&json!(null)), "none");
    }

    #[test]
    fn the_manifest_is_valid_json() {
        let m: Value = serde_json::from_str(MANIFEST).unwrap();
        assert_eq!((m["kind"].as_str(), m["format"]["extension"].as_str()), (Some("exporter"), Some("txt")));
    }
}
