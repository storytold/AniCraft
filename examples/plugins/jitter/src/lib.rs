//! Jitter Selection: moves, turns and scales each selected element by its own random amount,
//! about its own centre, for a hand-placed look. A selection transform: it needs a selection,
//! and leaves the same elements selected.

use anicraft_plugin_sdk::{self as ac, Rng};
use serde_json::{Value, json};

const MANIFEST: &str = r##"{
  "id": "org.anicraft.jitter",
  "name": "Jitter Selection",
  "version": "1.0.0",
  "kind": "command",
  "author": "AniCraft contributors",
  "description": "Moves, turns and scales each selected element by a random amount.",
  "selection": "required",
  "params": {
    "move":   {"type": "number", "min": 0, "max": 200, "default": 12, "label": "Move (px)"},
    "rotate": {"type": "number", "min": 0, "max": 180, "default": 15, "label": "Turn (°)"},
    "scale":  {"type": "number", "min": 0, "max": 90, "default": 10, "label": "Scale (%)"},
    "seed":   {"type": "int", "min": 0, "max": 99999, "default": 1, "label": "Seed"}
  }
}"##;

ac::plugin!(MANIFEST, run);

fn run(p: &Value) -> Result<(), String> {
    let (mv, rot, sc) = (p["move"].as_f64().unwrap_or(12.0), p["rotate"].as_f64().unwrap_or(15.0), p["scale"].as_f64().unwrap_or(10.0) / 100.0);
    let mut rng = Rng::new(p["seed"].as_u64().unwrap_or(1));
    let elements = ac::selected_elements(false)?;
    let ids: Vec<u64> = elements.iter().filter_map(|e| e["id"].as_u64()).collect();
    for e in &elements {
        let (Some(id), Some(b)) = (e["id"].as_u64(), e["bounds"].as_array()) else { continue };
        let f = |i: usize| b.get(i).and_then(Value::as_f64).unwrap_or(0.0);
        let (cx, cy) = (f(0) + f(2) * 0.5, f(1) + f(3) * 0.5);
        let (dx, dy) = (rng.signed() * mv, rng.signed() * mv);
        let angle = (rng.signed() * rot).to_radians();
        let k = 1.0 + rng.signed() * sc;
        // T(centre + offset) · R(angle) · S(k) · T(-centre), in stage space.
        let (s, c) = angle.sin_cos();
        let (a, b2, c2, d) = (c * k, s * k, -s * k, c * k);
        let (e2, f2) = (cx + dx - (a * cx + c2 * cy), cy + dy - (b2 * cx + d * cy));
        ac::command("element.select", json!({"ids": [id]}))?;
        ac::command("modify.transform", json!({"a": a, "b": b2, "c": c2, "d": d, "e": e2, "f": f2}))?;
    }
    ac::command("element.select", json!({"ids": ids}))?;
    Ok(())
}
