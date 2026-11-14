//! The layering rule from AGENTS.md, enforced: a crate depends only on crates in lower layers,
//! nothing below the shell uses the UI toolkit, and the workspace map in AGENTS.md matches the
//! crates that exist.

use std::collections::BTreeMap;
use std::path::Path;

/// Each crate's layer, as AGENTS.md documents it. The shell (`anicraft`, `src/`) is L5.
const LAYERS: &[(&str, u8)] =
    &[("geom", 0), ("xml", 0), ("plugins", 0), ("audio", 0), ("actions", 0), ("guide", 0), ("doc", 1), ("text", 1), ("render", 2), ("xfl", 2), ("svg", 2), ("engine", 3), ("gpu", 3), ("cli", 4)];
const SHELL_LAYER: u8 = 5;
/// UI crates only the shell may use.
const UI_CRATES: &[&str] = &["egui", "eframe", "winit", "egui-wgpu", "egui_glow"];

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn layer_of(name: &str) -> Option<u8> {
    LAYERS.iter().find(|(n, _)| *n == name).map(|(_, l)| *l)
}

/// Dependency names in a manifest's `[dependencies]`, `[dev-dependencies]` and
/// `[build-dependencies]` tables.
fn dependencies(manifest: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_deps = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_deps = matches!(line, "[dependencies]" | "[dev-dependencies]" | "[build-dependencies]");
            continue;
        }
        if !in_deps || line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.split(['=', '.', ' ']).next().filter(|n| !n.is_empty()) {
            out.push(name.to_string());
        }
    }
    out
}

#[test]
fn crates_depend_only_on_lower_layers_and_never_on_the_ui() {
    let mut problems = Vec::new();
    let mut seen = Vec::new();
    for entry in std::fs::read_dir(root().join("crates")).expect("crates/ exists").flatten() {
        let manifest = entry.path().join("Cargo.toml");
        let Ok(text) = std::fs::read_to_string(&manifest) else { continue };
        let name = entry.file_name().to_string_lossy().into_owned();
        seen.push(name.clone());
        let Some(own) = layer_of(&name) else {
            problems.push(format!("crates/{name} has no layer: add it to LAYERS here and to the workspace map in AGENTS.md"));
            continue;
        };
        for dep in dependencies(&text) {
            if UI_CRATES.contains(&dep.as_str()) {
                problems.push(format!("crates/{name} (L{own}) depends on {dep}; only the shell may use the UI toolkit"));
            }
            if let Some(d) = dep.strip_prefix("anicraft-")
                && let Some(l) = layer_of(d)
                && l >= own
            {
                problems.push(format!("crates/{name} (L{own}) depends on anicraft-{d} (L{l}); a crate may use only lower layers"));
            }
        }
    }
    assert!(seen.len() >= LAYERS.len(), "expected every crate in LAYERS to exist, found {seen:?}");
    // The shell may use any crate.
    for dep in dependencies(&std::fs::read_to_string(root().join("Cargo.toml")).unwrap_or_default()) {
        if let Some(l) = dep.strip_prefix("anicraft-").and_then(layer_of) {
            assert!(l < SHELL_LAYER);
        }
    }
    assert!(problems.is_empty(), "layering broken:\n{}", problems.join("\n"));
}

#[test]
fn the_workspace_map_in_agents_md_matches_the_crates() {
    let agents = std::fs::read_to_string(root().join("AGENTS.md")).expect("AGENTS.md");
    let mut documented: BTreeMap<String, u8> = BTreeMap::new();
    for line in agents.lines() {
        let mut words = line.split_whitespace();
        if let (Some(path), Some(layer)) = (words.next(), words.next())
            && let Some(name) = path.strip_prefix("crates/")
            && let Some(l) = layer.strip_prefix('L').and_then(|n| n.parse::<u8>().ok())
        {
            documented.insert(name.to_string(), l);
        }
    }
    let expected: BTreeMap<String, u8> = LAYERS.iter().map(|(n, l)| (n.to_string(), *l)).collect();
    assert_eq!(documented, expected, "AGENTS.md's workspace map and the layers enforced here disagree");
}

#[test]
fn dependency_lines_are_read_from_every_table() {
    let m = "[package]\nname = \"x\"\n\n[dependencies]\nanicraft-geom.workspace = true\nserde = { version = \"1\" }\n# a comment\n\n[dev-dependencies]\npng.workspace = true\n\n[lints]\nworkspace = true\n";
    assert_eq!(dependencies(m), ["anicraft-geom", "serde", "png"]);
}
