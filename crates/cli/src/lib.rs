//! AniCraft headless front end: argument parsing, the JSON-lines `serve` loop and the MCP
//! server (`mcp`). Every operation goes through the engine's command registry.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod mcp;

use anicraft_engine::{Session, commands, format};
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::time::Instant;

pub const USAGE: &str = "\
AniCraft CLI — headless animation engine

USAGE:
  anicraft-cli info <file|--sample>
  anicraft-cli render <file|--sample> --out <frame.png> [--frame N] [--scale S]
  anicraft-cli render-seq <file|--sample> --out-dir <dir> [--from A] [--to B] [--scale S]
  anicraft-cli gif <file|--sample> --out <anim.gif> [--from A] [--to B] [--scale S] [--overwrite yes]
  anicraft-cli apng|webp <file|--sample> --out <anim.png|anim.webp> [--from A] [--to B] [--scale S]
                         [--loop forever|N] [--background yes] [--overwrite yes]
                                         full colour with real transparency (WebP: lossless)
  anicraft-cli export <file|--sample> --format <id> --out <path> [--from A] [--to B] [--frame N]
                      [--options '<json>'] [--overwrite yes]
                                         any export format: the built-in ones (gif, apng, webp,
                                         lottie, html, video, svg, png, png-sequence, sprite-sheet)
                                         and exporter plug-ins (by id, with --plugins); --options
                                         are the plug-in's options, or a built-in's extra params
  anicraft-cli formats                   lists every export format, plug-ins included (JSON)
  anicraft-cli commands
  anicraft-cli run <file|--sample> <script.json> [--save <out.anicraft>]
  anicraft-cli exec <file|--sample> <command.id> [<json-params>] [--save <out.anicraft>]
  anicraft-cli new --out <file.anicraft> [--width W] [--height H] [--fps F]
  anicraft-cli guide --out <folder>      writes the user guide as HTML pages (open index.html)
  anicraft-cli bench [--layers N] [--shapes N] [--frames N]   performance probe (use --release)
  anicraft-cli bench --check yes         the default scenario against the budgets in
                                         docs/performance.md; fails when one is missed
  anicraft-cli serve [file|--sample]     JSON-lines on stdin/stdout:
        {\"id\":1,\"cmd\":\"draw.rect\",\"params\":{\"x\":10,\"y\":10,\"w\":50,\"h\":50}}
  anicraft-cli mcp [file|--sample]       Model Context Protocol server on stdin/stdout,
                                         for AI assistants (see docs/mcp.md)
  anicraft-cli --version

  --plugins <folder>   loads the WebAssembly plug-ins in a folder first (also the
                       ANICRAFT_PLUGINS environment variable); see docs/plugins.md.
                       Without it, headless runs use no plug-ins, so they are repeatable.

Frames on the command line are 1-based, as in the timeline.";

/// Parsed flags: positional args plus `--key value` pairs.
pub struct Args {
    pub positional: Vec<String>,
    pub flags: Vec<(String, String)>,
}

impl Args {
    pub fn parse(args: &[String]) -> Result<Args, String> {
        let mut positional = Vec::new();
        let mut flags = Vec::new();
        let mut it = args.iter();
        while let Some(a) = it.next() {
            if a == "--sample" {
                positional.push(a.clone());
            } else if let Some(k) = a.strip_prefix("--") {
                let v = it.next().ok_or_else(|| format!("flag --{k} needs a value"))?;
                flags.push((k.to_string(), v.clone()));
            } else {
                positional.push(a.clone());
            }
        }
        Ok(Args { positional, flags })
    }
    pub fn flag(&self, k: &str) -> Option<&str> {
        self.flags.iter().rev().find(|(key, _)| key == k).map(|(_, v)| v.as_str())
    }
    pub fn num<T: std::str::FromStr>(&self, k: &str) -> Result<Option<T>, String> {
        self.flag(k).map(|v| v.parse::<T>().map_err(|_| format!("--{k} expects a number, got '{v}'"))).transpose()
    }
}

/// Opens a session from a path or `--sample`.
pub fn open_session(src: Option<&str>) -> Result<Session, String> {
    match src {
        None => Ok(Session::default()),
        Some("--sample") => Ok(Session::new(anicraft_doc::sample::demo())),
        // An Animate document imports as File › Open does (#226), untitled.
        Some(p) if anicraft_engine::is_animate(std::path::Path::new(p)) => {
            let mut s = Session::default();
            s.execute("file.open", &serde_json::json!({"path": p})).map_err(|e| e.to_string())?;
            Ok(s)
        }
        Some(p) => {
            let doc = format::load(std::path::Path::new(p)).map_err(|e| format!("{p}: {e}"))?;
            let mut s = Session::new(doc);
            s.path = Some(PathBuf::from(p));
            Ok(s)
        }
    }
}

/// [`open_session`], with the plug-ins of `--plugins <folder>` (or `ANICRAFT_PLUGINS`) loaded.
pub fn session(a: &Args, src: Option<&str>) -> Result<Session, String> {
    let mut s = open_session(src)?;
    let folder = a.flag("plugins").map(str::to_string).or_else(|| std::env::var("ANICRAFT_PLUGINS").ok().filter(|v| !v.trim().is_empty()));
    if let Some(folder) = folder {
        let r = exec(&mut s, "plugin.reload", &json!({"folder": folder}))?;
        for f in r["failed"].as_array().into_iter().flatten() {
            eprintln!("plug-in {} not loaded: {}", f["file"].as_str().unwrap_or(""), f["error"].as_str().unwrap_or(""));
        }
    }
    Ok(s)
}

fn exec(s: &mut Session, id: &str, params: &Value) -> Result<Value, String> {
    s.execute(id, params).map_err(|e| format!("{id}: {e}"))
}

fn one_based(a: &Args, k: &str) -> Result<Option<u32>, String> {
    match a.num::<u32>(k)? {
        Some(0) => Err(format!("--{k} is 1-based; the first frame is 1")),
        Some(n) => Ok(Some(n - 1)),
        None => Ok(None),
    }
}

/// Runs the CLI; returns the text to print on success.
pub fn run(argv: &[String]) -> Result<String, String> {
    let Some((cmd, rest)) = argv.split_first() else { return Ok(USAGE.to_string()) };
    let a = Args::parse(rest)?;
    let src = a.positional.first().map(String::as_str);
    match cmd.as_str() {
        "help" | "--help" | "-h" => Ok(USAGE.to_string()),
        // The release packaging smoke-tests every build with this (packaging/, ADR 0026).
        "version" | "--version" | "-V" => Ok(format!("anicraft-cli {}", env!("CARGO_PKG_VERSION"))),
        "commands" => {
            let list: Vec<Value> = commands().iter().map(|c| c.to_json()).collect();
            serde_json::to_string_pretty(&list).map_err(|e| e.to_string())
        }
        // The user guide as the static HTML book Help › User Guide opens (ADR 0069).
        "guide" => {
            let out = a.flag("out").ok_or("guide needs --out <folder>")?;
            let n = anicraft_guide::write_site(std::path::Path::new(out)).map_err(|e| format!("{out}: {e}"))?;
            Ok(format!("wrote {n} files of the user guide to {out}; open {out}/index.html"))
        }
        "info" => {
            let s = session(&a, Some(src.ok_or("info needs a file or --sample")?))?;
            serde_json::to_string_pretty(&s.info()).map_err(|e| e.to_string())
        }
        "render" => {
            let mut s = session(&a, Some(src.ok_or("render needs a file or --sample")?))?;
            let out = a.flag("out").ok_or("render needs --out <file.png>")?;
            let mut p = json!({"path": out, "frame": one_based(&a, "frame")?.unwrap_or(0)});
            if let Some(sc) = a.num::<f32>("scale")? {
                p["scale"] = json!(sc);
            }
            exec(&mut s, "file.export_png", &p).map(|v| v.to_string())
        }
        "render-seq" => {
            let mut s = session(&a, Some(src.ok_or("render-seq needs a file or --sample")?))?;
            let dir = a.flag("out-dir").ok_or("render-seq needs --out-dir <dir>")?;
            let mut p = json!({"dir": dir});
            if let Some(f) = one_based(&a, "from")? {
                p["from"] = json!(f);
            }
            if let Some(t) = one_based(&a, "to")? {
                p["to"] = json!(t);
            }
            if let Some(sc) = a.num::<f32>("scale")? {
                p["scale"] = json!(sc);
            }
            exec(&mut s, "file.export_png_sequence", &p).map(|v| v.to_string())
        }
        "gif" | "apng" | "webp" => {
            let mut s = session(&a, Some(src.ok_or(format!("{cmd} needs a file or --sample"))?))?;
            let cmd = cmd.as_str();
            let ext = if cmd == "apng" { "png" } else { cmd };
            let out = a.flag("out").ok_or(format!("{cmd} needs --out <anim.{ext}>"))?;
            let mut p = json!({"path": out});
            if let Some(f) = one_based(&a, "from")? {
                p["from"] = json!(f);
            }
            if let Some(t) = one_based(&a, "to")? {
                p["to"] = json!(t);
            }
            if let Some(sc) = a.num::<f32>("scale")? {
                p["scale"] = json!(sc);
            }
            if a.flag("overwrite").is_some_and(|v| v == "yes" || v == "true") {
                p["confirm"] = json!(true);
            }
            if cmd != "gif" {
                match a.flag("loop") {
                    None | Some("forever") => {}
                    Some(n) => p["loop"] = json!(n.parse::<u32>().map_err(|_| format!("--loop is forever or how many times it plays, not '{n}'"))?),
                }
                if a.flag("background").is_some_and(|v| v == "yes" || v == "true") {
                    p["background"] = json!(true);
                }
            }
            exec(&mut s, &format!("file.export_{cmd}"), &p).map(|v| v.to_string())
        }
        "formats" => {
            let mut s = session(&a, None)?;
            let v = exec(&mut s, "file.export_formats", &json!({}))?;
            serde_json::to_string_pretty(&v["formats"]).map_err(|e| e.to_string())
        }
        "export" => export(&a, src),
        "new" => {
            let mut s = Session::default();
            let out = a.flag("out").ok_or("new needs --out <file.anicraft>")?;
            let mut p = json!({});
            for k in ["width", "height"] {
                if let Some(v) = a.num::<u32>(k)? {
                    p[k] = json!(v);
                }
            }
            if let Some(v) = a.num::<f32>("fps")? {
                p["fps"] = json!(v);
            }
            exec(&mut s, "file.new", &p)?;
            exec(&mut s, "file.save", &json!({"path": out})).map(|v| v.to_string())
        }
        "exec" => {
            let mut s = session(&a, Some(src.ok_or("exec needs a file or --sample")?))?;
            let id = a.positional.get(1).ok_or("exec needs a command id")?;
            let params: Value = match a.positional.get(2) {
                Some(t) => serde_json::from_str(t).map_err(|e| format!("params are not JSON: {e}"))?,
                None => json!({}),
            };
            let v = exec(&mut s, id, &params)?;
            if let Some(out) = a.flag("save") {
                exec(&mut s, "file.save", &json!({"path": out}))?;
            }
            Ok(v.to_string())
        }
        "run" => {
            let mut s = session(&a, Some(src.ok_or("run needs a file or --sample")?))?;
            let script = a.positional.get(1).ok_or("run needs a script.json")?;
            let text = std::fs::read_to_string(script).map_err(|e| format!("{script}: {e}"))?;
            // Windows editors often save UTF-8 with a byte-order mark; JSON parsers reject it.
            let steps: Vec<Value> = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| format!("{script}: expected a JSON array of {{cmd, params}}: {e}"))?;
            let mut results = Vec::new();
            for (i, step) in steps.iter().enumerate() {
                let id = step.get("cmd").and_then(Value::as_str).ok_or_else(|| format!("step {}: missing \"cmd\"", i + 1))?;
                let params = step.get("params").cloned().unwrap_or_else(|| json!({}));
                results.push(exec(&mut s, id, &params).map_err(|e| format!("step {}: {e}", i + 1))?);
            }
            if let Some(out) = a.flag("save") {
                exec(&mut s, "file.save", &json!({"path": out}))?;
            }
            serde_json::to_string_pretty(&results).map_err(|e| e.to_string())
        }
        "bench" => bench(&a),
        "serve" => {
            let mut s = session(&a, src)?;
            let stdin = std::io::stdin();
            let mut stdout = std::io::stdout();
            serve(&mut s, stdin.lock(), &mut stdout).map_err(|e| e.to_string())?;
            Ok(String::new())
        }
        "mcp" => {
            let mut s = session(&a, src)?;
            let stdin = std::io::stdin();
            let mut stdout = std::io::stdout();
            mcp::serve(&mut s, stdin.lock(), &mut stdout).map_err(|e| e.to_string())?;
            Ok(String::new())
        }
        other => Err(format!("unknown subcommand '{other}'\n\n{USAGE}")),
    }
}

/// `export --format <id>`: a built-in format through its own command, or an exporter plug-in
/// through `file.export_plugin` (ADR 0088).
fn export(a: &Args, src: Option<&str>) -> Result<String, String> {
    let mut s = session(a, Some(src.ok_or("export needs a file or --sample")?))?;
    let id = a.flag("format").ok_or("export needs --format <id> (anicraft-cli formats lists them)")?;
    let out = a.flag("out").ok_or("export needs --out <path>")?;
    let list = exec(&mut s, "file.export_formats", &json!({}))?;
    let formats = list["formats"].as_array().cloned().unwrap_or_default();
    let Some(f) = formats.iter().find(|f| f["id"] == id) else {
        let ids: Vec<&str> = formats.iter().filter_map(|f| f["id"].as_str()).collect();
        return Err(format!("no export format '{id}' (formats: {}; exporter plug-ins load with --plugins <folder>)", ids.join(", ")));
    };
    let (cmd, target, plugin) = (f["command"].as_str().unwrap_or(""), f["target"].as_str().unwrap_or("path"), f["plugin"] == true);
    let mut p = json!({ target: out });
    let options: Option<Value> = a.flag("options").map(|t| serde_json::from_str(t).map_err(|e| format!("--options is not JSON: {e}"))).transpose()?;
    if plugin {
        p["format"] = json!(id);
        if let Some(o) = options {
            p["options"] = o;
        }
    } else if let Some(Value::Object(o)) = options {
        for (k, v) in o {
            p[k] = v;
        }
    } else if options.is_some() {
        return Err("--options must be a JSON object".into());
    }
    if matches!(id, "png" | "svg") {
        if let Some(f) = one_based(a, "frame")? {
            p["frame"] = json!(f);
        }
    } else {
        for k in ["from", "to"] {
            if let Some(f) = one_based(a, k)? {
                p[k] = json!(f);
            }
        }
    }
    if a.flag("overwrite").is_some_and(|v| v == "yes" || v == "true") {
        p["confirm"] = json!(true);
    }
    exec(&mut s, cmd, &p).map(|v| v.to_string())
}

fn avg_ms(n: u32, mut f: impl FnMut(u32)) -> f64 {
    let t = Instant::now();
    for i in 0..n {
        f(i);
    }
    t.elapsed().as_secs_f64() * 1000.0 / f64::from(n.max(1))
}

fn round(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// Timing budgets for the default `bench` scenario (50 layers × 200 shapes × 240 frames,
/// 200k element instances), in release builds: `(key in the bench output, what, limit)`.
/// Keep in step with the table in `docs/performance.md`. Budgets that don't depend on the
/// machine (memory, sharing, file size, culling) are tests instead, run on every PR.
pub const TIME_BUDGETS: &[(&str, &str, f64)] = &[
    ("edit_command_ms", "edit command (ms)", 1.0),
    ("undo_ms", "undo (ms)", 1.0),
    ("snapshot_clone_us", "document snapshot (µs)", 50.0),
    ("evaluate_ms_per_frame", "evaluate a frame (ms)", 4.0),
    ("render_1080p_ms_per_frame", "render 1080p (ms)", 16.7),
    ("render_zoom400_ms_per_frame", "render at 400% (ms)", 16.7),
    ("hit_test_us", "hit test (µs)", 200.0),
    ("file.save_ms", "save the 65 MB file (ms)", 500.0),
    ("file.load_ms", "load the 65 MB file (ms)", 500.0),
    ("autosave.ui_us", "autosave: time on the editing thread (µs)", 1000.0),
    ("autosave.background_ms", "autosave the 65 MB file in the background (ms)", 2000.0),
];

/// One crash-recovery autosave of `doc` as the app does it: an `Autosaver` in a scratch
/// recovery folder, timing what the editing thread pays (the hand-off) apart from the
/// background serialize + atomic write (temp file, flush to disk, rename).
fn bench_autosave(doc: &anicraft_doc::Document) -> Value {
    use anicraft_engine::recovery::{Autosaver, Outcome, SessionLock};
    let dir = std::env::temp_dir().join(format!("anicraft-bench-recovery-{}", std::process::id()));
    let Ok(mut saver) = SessionLock::acquire(&dir).and_then(Autosaver::start) else { return json!({"error": "no scratch folder"}) };
    let t = Instant::now();
    saver.write(doc.clone(), None);
    let ui_us = t.elapsed().as_secs_f64() * 1e6;
    let mut out = json!({"error": "timed out"});
    while t.elapsed().as_secs() < 120 {
        if let Some(o) = saver.poll().into_iter().next() {
            let background_ms = t.elapsed().as_secs_f64() * 1000.0;
            out = match o {
                Outcome::Written { stats, .. } => json!({
                    "ui_us": round(ui_us), "background_ms": round(background_ms),
                    "serialize_ms": round(stats.serialize_ms), "write_ms": round(stats.write_ms),
                    "mb": round(stats.bytes as f64 / 1048576.0),
                }),
                Outcome::Failed { error, .. } => json!({"error": error}),
                Outcome::Cleared { .. } => out,
            };
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    saver.finish(None);
    let _ = std::fs::remove_dir_all(&dir);
    out
}

/// Compares bench output with [`TIME_BUDGETS`]: one row per budget, and the ones missed.
pub fn check_budgets(out: &Value) -> (Vec<Value>, Vec<String>) {
    let mut rows = Vec::new();
    let mut missed = Vec::new();
    for (key, what, limit) in TIME_BUDGETS {
        let value = key.split('.').try_fold(out, |v, k| v.get(k)).and_then(Value::as_f64);
        let ok = value.is_some_and(|v| v <= *limit);
        if !ok {
            missed.push(match value {
                Some(v) => format!("{what}: {v} (budget {limit})"),
                None => format!("{what}: not measured"),
            });
        }
        rows.push(json!({"budget": what, "limit": limit, "measured": value, "ok": ok}));
    }
    (rows, missed)
}

/// Performance probe on a generated stress document. Run in release for meaningful numbers:
/// `cargo run --release -p anicraft-cli -- bench`. With `--check yes`, the default scenario
/// is compared with [`TIME_BUDGETS`] and a missed budget is an error.
pub fn bench(a: &Args) -> Result<String, String> {
    let check = a.flag("check").is_some_and(|v| v == "yes" || v == "true");
    if check {
        if cfg!(debug_assertions) {
            return Err("bench --check needs a release build: cargo run --release -p anicraft-cli -- bench --check yes".into());
        }
        if ["layers", "shapes", "frames"].iter().any(|k| a.flag(k).is_some()) {
            return Err("the budgets are for the default scenario; leave out --layers, --shapes and --frames with --check".into());
        }
    }
    use anicraft_doc::{evaluate, sample};
    use anicraft_engine::history::document_bytes;
    use anicraft_render::{View, render};

    let layers = a.num::<u32>("layers")?.unwrap_or(50);
    let shapes = a.num::<u32>("shapes")?.unwrap_or(200);
    let frames = a.num::<u32>("frames")?.unwrap_or(240);
    let t = Instant::now();
    let doc = sample::stress(layers, shapes, frames);
    let build_ms = t.elapsed().as_secs_f64() * 1000.0;
    let instances: usize = doc.timeline(0).layers.iter().flat_map(|l| l.keyframes.iter()).map(|k| k.elements.len()).sum();

    let mut json_text = String::new();
    let save_ms = avg_ms(1, |_| json_text = format::to_json(&doc).unwrap_or_default());
    let load_ms = avg_ms(1, |_| {
        let _ = format::from_json(&json_text);
    });

    let autosave = bench_autosave(&doc);

    let eval_ms = avg_ms(48, |i| {
        let _ = evaluate(&doc, (i * 5) % frames.max(1));
    });
    let view = View::stage(&doc, 1.0);
    let mut drawn = 0;
    let render_ms = avg_ms(12, |i| {
        if let Ok((_, st)) = render(&evaluate(&doc, (i * 20) % frames.max(1)), &view) {
            drawn = st.drawn;
        }
    });
    // 400% zoom on the stage centre: most shapes are off-screen and culled.
    let zoomed = View { transform: anicraft_geom::Affine::translate(-2880.0, -1620.0).mul(&anicraft_geom::Affine::scale(4.0, 4.0)), ..view };
    let mut culled = 0;
    let zoom_ms = avg_ms(12, |i| {
        if let Ok((_, st)) = render(&evaluate(&doc, (i * 20) % frames.max(1)), &zoomed) {
            culled = st.culled;
        }
    });
    let clone_us = avg_ms(1000, |_| {
        let _ = doc.clone();
    }) * 1000.0;

    let mut s = Session::new(doc);
    let _ = s.execute("layer.select", &json!({"index": 0}));
    s.set_frame(7);
    let first = s.visible_elements().first().map(|e| e.id.0);
    if let Some(id) = first {
        let _ = s.execute("element.select", &json!({"ids": [id]}));
    }
    let edit_ms = avg_ms(100, |_| {
        let _ = s.execute("element.translate", &json!({"dx": 1, "dy": 0}));
    });
    let steps = s.history().undo_len().max(1);
    let per_step = s.history().bytes() / steps;
    let undo_ms = avg_ms(50, |_| {
        let _ = s.undo();
    });
    let hit_us = avg_ms(500, |i| {
        let _ = s.hit_test(anicraft_geom::Vec2::new((i * 37 % 1920) as f32, (i * 53 % 1080) as f32), 2.0);
    }) * 1000.0;

    let out = json!({
        "document": {"layers": layers, "shapes_per_layer": shapes, "frames": frames, "element_instances": instances,
                     "estimated_mb": round(document_bytes(&s.doc) as f64 / 1048576.0), "build_ms": round(build_ms)},
        "file": {"json_mb": round(json_text.len() as f64 / 1048576.0), "save_ms": round(save_ms), "load_ms": round(load_ms)},
        "autosave": autosave,
        "evaluate_ms_per_frame": round(eval_ms),
        "render_1080p_ms_per_frame": round(render_ms), "render_shapes_drawn": drawn,
        "render_zoom400_ms_per_frame": round(zoom_ms), "render_zoom400_culled": culled,
        "snapshot_clone_us": round(clone_us),
        "edit_command_ms": round(edit_ms), "undo_bytes_per_step": per_step, "undo_ms": round(undo_ms),
        "hit_test_us": round(hit_us),
        "threads": std::thread::available_parallelism().map_or(1, |n| n.get()),
        "build": if cfg!(debug_assertions) { "debug (use --release for real numbers)" } else { "release" },
    });
    if !check {
        return serde_json::to_string_pretty(&out).map_err(|e| e.to_string());
    }
    let (rows, missed) = check_budgets(&out);
    let report = serde_json::to_string_pretty(&json!({"measured": out, "budgets": rows})).map_err(|e| e.to_string())?;
    if missed.is_empty() {
        Ok(report)
    } else {
        Err(format!("{report}\n\nover budget: {}", missed.join("; ")))
    }
}

/// Answers one JSON request.
pub fn handle_request(s: &mut Session, line: &str) -> Value {
    // Windows shells often prefix piped UTF-8 with a byte-order mark.
    let line = line.trim_start_matches('\u{feff}');
    let req: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => return json!({"id": null, "ok": false, "error": format!("invalid JSON: {e}")}),
    };
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let Some(cmd) = req.get("cmd").and_then(Value::as_str) else {
        return json!({"id": id, "ok": false, "error": "missing \"cmd\""});
    };
    let params = req.get("params").cloned().unwrap_or_else(|| json!({}));
    let result = match cmd {
        "commands" => Ok(Value::Array(commands().iter().map(|c| c.to_json()).collect())),
        _ => s.execute(cmd, &params).map_err(|e| e.to_string()),
    };
    match result {
        Ok(v) => json!({"id": id, "ok": true, "result": v, "revision": s.revision, "frame": s.frame}),
        Err(e) => json!({"id": id, "ok": false, "error": e}),
    }
}

/// JSON-lines loop: one request per line, one response per line. Ends on EOF or `{"cmd":"quit"}`.
pub fn serve<R: BufRead, W: Write>(s: &mut Session, input: R, out: &mut W) -> std::io::Result<()> {
    writeln!(out, "{}", json!({"ready": true, "app": "AniCraft", "version": env!("CARGO_PKG_VERSION")}))?;
    out.flush()?;
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if line.contains("\"quit\"") && serde_json::from_str::<Value>(&line).ok().and_then(|v| v.get("cmd").cloned()) == Some(json!("quit")) {
            writeln!(out, "{}", json!({"ok": true, "bye": true}))?;
            break;
        }
        let resp = handle_request(s, &line);
        writeln!(out, "{resp}")?;
        out.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #226: `info`, `render` and the other subcommands take an Animate document as File › Open
    /// does (an import, untitled), not only `.anicraft` files.
    #[test]
    fn subcommands_open_animate_documents() {
        let xfl = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../xfl/tests/corpus/shapes/shapes.xfl");
        let s = open_session(Some(&xfl.display().to_string())).unwrap();
        assert!(s.path.is_none() && !s.doc.timeline(0).layers.is_empty());
        let info = run(&["info".to_string(), xfl.display().to_string()]).unwrap();
        assert!(info.contains("\"layers\""), "{info}");
    }

    #[test]
    fn serve_round_trip() {
        let mut s = Session::default();
        let input = b"\xef\xbb\xbf{\"id\":1,\"cmd\":\"draw.rect\",\"params\":{\"x\":1,\"y\":1,\"w\":5,\"h\":5}}\nnot json\n{\"id\":3,\"cmd\":\"nope\"}\n{\"cmd\":\"quit\"}\n{\"id\":4,\"cmd\":\"edit.undo\"}\n";
        let mut out = Vec::new();
        serve(&mut s, &input[..], &mut out).unwrap();
        let lines: Vec<Value> = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines[0]["ready"], true);
        assert_eq!(lines[1]["ok"], true);
        assert_eq!(lines[2]["ok"], false);
        assert_eq!(lines[3]["ok"], false);
        assert_eq!(lines[4]["bye"], true);
        assert_eq!(lines.len(), 5, "nothing runs after quit");
        assert_eq!(s.visible_elements().len(), 1);
    }

    #[test]
    fn version_names_the_workspace_version() {
        for flag in ["--version", "-V", "version"] {
            assert_eq!(run(&[flag.to_string()]).unwrap(), format!("anicraft-cli {}", env!("CARGO_PKG_VERSION")));
        }
    }

    #[test]
    fn render_sample_to_png() {
        let dir = std::env::temp_dir().join(format!("anicraft-cli-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("f.png");
        let argv: Vec<String> = ["render", "--sample", "--frame", "12", "--scale", "0.25", "--out", out.to_str().unwrap()].iter().map(|s| s.to_string()).collect();
        run(&argv).unwrap();
        let bytes = std::fs::read(&out).unwrap();
        assert_eq!(&bytes[1..4], b"PNG");
        assert!(run(&["render".into(), "--sample".into(), "--frame".into(), "0".into(), "--out".into(), "x.png".into()]).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The budgets in docs/performance.md that don't depend on the machine, checked on every
    /// build. (Timing budgets are `bench --check yes`, on a known machine.)
    #[test]
    fn machine_independent_budgets() {
        use anicraft_doc::{evaluate, sample};
        use anicraft_engine::history::document_bytes;
        // 20 layers × 500 shapes × 120 frames: 100k element instances.
        let doc = sample::stress(20, 500, 120);
        let instances: usize = doc.timeline(0).layers.iter().flat_map(|l| l.keyframes.iter()).map(|k| k.elements.len()).sum();
        assert_eq!(instances, 100_000);

        // A snapshot (for undo, the render worker, autosave) shares everything: O(1).
        let snap = doc.clone();
        assert!(snap.scenes.iter().zip(&doc.scenes).all(|(a, b)| std::sync::Arc::ptr_eq(a, b)));

        // Undo memory per small edit < 16 KB, however large the document.
        let mut s = Session::new(doc);
        s.execute("layer.select", &json!({"index": 3})).unwrap();
        s.set_frame(30);
        let id = s.visible_elements()[0].id.0;
        s.execute("element.select", &json!({"ids": [id]})).unwrap();
        s.execute("element.translate", &json!({"dx": 5, "dy": 0})).unwrap();
        let step = s.history().bytes();
        assert!(step < 16 * 1024, "one edit kept {step} bytes of undo history (budget 16 KB)");
        assert!(step * 1000 < document_bytes(&s.doc), "an edit keeps what changed, not the document");

        // File size: < 400 bytes per element instance (shared paths are written once).
        let small = sample::stress(10, 100, 48);
        let n: usize = small.timeline(0).layers.iter().flat_map(|l| l.keyframes.iter()).map(|k| k.elements.len()).sum();
        let bytes = format::to_json(&small).unwrap().len();
        assert!(bytes < n * 400, "{} bytes per instance (budget 400)", bytes / n.max(1));

        // At 400% zoom on the stage centre, off-screen shapes are culled: at most 1/8 drawn.
        let dl = evaluate(&small, 5);
        let view = anicraft_render::View::stage(&small, 1.0);
        let zoomed = anicraft_render::View { transform: anicraft_geom::Affine::translate(-2880.0, -1620.0).mul(&anicraft_geom::Affine::scale(4.0, 4.0)), ..view };
        let (_, st) = anicraft_render::render(&dl, &zoomed).unwrap();
        assert!(st.items >= 1000 && st.drawn * 8 <= st.items, "{} of {} shapes drawn at 400%", st.drawn, st.items);
    }

    #[test]
    fn budget_check_reads_bench_output() {
        let mut out = json!({"edit_command_ms": 0.01, "undo_ms": 0.01, "snapshot_clone_us": 0.4, "evaluate_ms_per_frame": 1.5,
            "render_1080p_ms_per_frame": 20.0, "render_zoom400_ms_per_frame": 10.0, "hit_test_us": 50.0, "file": {"save_ms": 100.0}, "autosave": {"ui_us": 5.0, "background_ms": 300.0}});
        let (rows, missed) = check_budgets(&out);
        assert_eq!(rows.len(), TIME_BUDGETS.len());
        assert_eq!(missed, ["render 1080p (ms): 20 (budget 16.7)", "load the 65 MB file (ms): not measured"]);
        out["render_1080p_ms_per_frame"] = json!(16.0);
        out["file"]["load_ms"] = json!(120.0);
        assert!(check_budgets(&out).1.is_empty());
        // Debug builds and custom sizes can't be checked against the budgets.
        if cfg!(debug_assertions) {
            assert!(run(&["bench".into(), "--check".into(), "yes".into()]).unwrap_err().contains("release"));
        }
    }

    #[test]
    fn plugins_folder_flag() {
        let dir = std::env::temp_dir().join(format!("anicraft-cli-plugins-{}", std::process::id()));
        let argv: Vec<String> = ["--sample", "--plugins", dir.to_str().unwrap()].iter().map(|s| s.to_string()).collect();
        let a = Args::parse(&argv).unwrap();
        let s = session(&a, Some("--sample")).unwrap();
        // A folder that doesn't exist yet is the plug-ins folder, with nothing in it.
        assert_eq!(s.plugins.folder(), Some(dir.as_path()));
        assert!(s.plugins.is_empty());
    }

    #[test]
    fn export_any_format_built_in_or_plugin() {
        let dir = std::env::temp_dir().join(format!("anicraft-cli-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("plugins")).unwrap();
        let argv = |v: &[&str]| -> Vec<String> { v.iter().map(|s| s.to_string()).collect() };
        // A built-in format by id.
        let svg = dir.join("f.svg");
        run(&argv(&["export", "--sample", "--format", "svg", "--frame", "3", "--out", svg.to_str().unwrap()])).unwrap();
        assert!(std::fs::read_to_string(&svg).unwrap().contains("<svg"));
        // An exporter plug-in that writes "frames:" and the count it was handed.
        let m = r#"{"id":"org.test.count","name":"Count","kind":"exporter","format":{"name":"Count","extension":"txt"}}"#;
        let req = r#"{"op":"movie"}"#;
        let wat = format!(
            r#"(module (import "anicraft" "call" (func $call (param i32 i32) (result i32))) (import "anicraft" "response" (func $response (param i32 i32) (result i32)))
              (memory (export "memory") 4) (data (i32.const 1024) "{}") (data (i32.const 4096) "{}")
              (func (export "ac_abi_version") (result i32) i32.const 1)
              (func (export "ac_manifest") (result i64) (i64.or (i64.shl (i64.const {}) (i64.const 32)) (i64.const 1024)))
              (func (export "ac_alloc") (param i32) (result i32) i32.const 60000)
              (func (export "ac_export") (param i32 i32) (result i64) (local $n i32)
                (local.set $n (call $response (i32.const 100000) (call $call (i32.const 4096) (i32.const {}))))
                (i64.or (i64.shl (i64.extend_i32_u (local.get $n)) (i64.const 32)) (i64.const 100000))))"#,
            m.replace('"', "\\22"),
            req.replace('"', "\\22"),
            m.len(),
            req.len()
        );
        std::fs::write(dir.join("plugins").join("count.wasm"), wat::parse_str(&wat).unwrap()).unwrap();
        let plugins = dir.join("plugins");
        let listed = run(&argv(&["formats", "--plugins", plugins.to_str().unwrap()])).unwrap();
        assert!(listed.contains("org.test.count") && listed.contains("\"gif\""));
        let out = dir.join("movie.txt");
        let v = run(&argv(&["export", "--sample", "--format", "org.test.count", "--from", "2", "--to", "5", "--out", out.to_str().unwrap(), "--plugins", plugins.to_str().unwrap()])).unwrap();
        assert!(v.contains("\"frames\":4"), "{v}");
        let movie: Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
        assert_eq!((movie["result"]["from"].as_u64(), movie["result"]["frames"].as_u64()), (Some(1), Some(4)));
        // Replacing it needs --overwrite yes; unknown formats name the known ones.
        assert!(run(&argv(&["export", "--sample", "--format", "org.test.count", "--out", out.to_str().unwrap(), "--plugins", plugins.to_str().unwrap()])).is_err());
        run(&argv(&["export", "--sample", "--format", "org.test.count", "--out", out.to_str().unwrap(), "--plugins", plugins.to_str().unwrap(), "--overwrite", "yes"])).unwrap();
        let err = run(&argv(&["export", "--sample", "--format", "org.test.count", "--out", out.to_str().unwrap()])).unwrap_err();
        assert!(err.contains("lottie") && err.contains("--plugins"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn usage_and_errors() {
        assert!(run(&[]).unwrap().contains("USAGE"));
        assert!(run(&["bogus".into()]).is_err());
        assert!(run(&["info".into()]).is_err());
        assert!(run(&["commands".into()]).unwrap().contains("timeline.insert_keyframe"));
        assert!(run(&["guide".into()]).is_err());
    }
}
