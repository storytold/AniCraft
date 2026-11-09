//! Star Field: scatters stars of varied size, angle and brightness over the stage, on a new
//! layer by default. A generator: it needs no selection. The seed makes it repeatable.

use anicraft_plugin_sdk::{self as ac, Rng};
use serde_json::{Value, json};

const MANIFEST: &str = r##"{
  "id": "org.anicraft.star-field",
  "name": "Star Field",
  "version": "1.0.0",
  "kind": "command",
  "author": "AniCraft contributors",
  "description": "Scatters stars of varied size, angle and brightness over the stage.",
  "params": {
    "count":     {"type": "int", "min": 1, "max": 1000, "default": 120, "label": "Stars"},
    "size_min":  {"type": "number", "min": 1, "max": 100, "default": 2, "label": "Smallest (px)"},
    "size_max":  {"type": "number", "min": 1, "max": 200, "default": 9, "label": "Largest (px)"},
    "points":    {"type": "int", "min": 3, "max": 12, "default": 5, "label": "Points"},
    "color":     {"type": "color", "default": "#fff4c2", "label": "Colour"},
    "seed":      {"type": "int", "min": 0, "max": 99999, "default": 7, "label": "Seed"},
    "new_layer": {"type": "bool", "default": true, "label": "On a new layer"}
  }
}"##;

ac::plugin!(MANIFEST, run);

fn run(p: &Value) -> Result<(), String> {
    let doc = ac::document()?;
    let (w, h) = (doc["stage"]["width"].as_f64().unwrap_or(550.0), doc["stage"]["height"].as_f64().unwrap_or(400.0));
    let count = p["count"].as_u64().unwrap_or(120);
    let (a, b) = (p["size_min"].as_f64().unwrap_or(2.0), p["size_max"].as_f64().unwrap_or(9.0));
    let (small, large) = (a.min(b), a.max(b));
    let points = p["points"].as_u64().unwrap_or(5);
    let color = p["color"].as_str().unwrap_or("#fff4c2");
    let mut rng = Rng::new(p["seed"].as_u64().unwrap_or(7));
    if p["new_layer"].as_bool().unwrap_or(true) {
        ac::command("layer.new", json!({"name": "Star Field"}))?;
    }
    for _ in 0..count {
        // Many small stars, few large ones.
        let t = rng.unit();
        let outer = small + (large - small) * t * t * t;
        let star = json!({
            "cx": rng.unit() * w, "cy": rng.unit() * h,
            "outer": outer, "inner": outer * (0.38 + 0.12 * rng.unit()),
            "points": points, "fill": color, "stroke": null,
        });
        ac::command("draw.star", star)?;
        // The new star is selected: turn it and dim it a little.
        ac::command("element.set_transform", json!({"rotation": rng.unit() * 360.0 / points as f64}))?;
        ac::command("element.set_alpha", json!({"alpha": 0.45 + 0.55 * rng.unit()}))?;
    }
    ac::command("edit.deselect", json!({}))?;
    ac::log(&format!("{count} stars"));
    Ok(())
}
