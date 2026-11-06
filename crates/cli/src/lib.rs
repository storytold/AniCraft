//! AniCraft headless front end: argument parsing and the JSON-lines `serve` loop.
//! Every operation goes through the engine's command registry.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

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
  anicraft-cli sprite-sheet <file|--sample> --out <sheet.png> [--from A] [--to B] [--scale S] [--columns N] [--transparent true|false] [--overwrite yes]
  anicraft-cli commands
  anicraft-cli run <file|--sample> <script.json> [--save <out.anicraft>]
  anicraft-cli exec <file|--sample> <command.id> [<json-params>] [--save <out.anicraft>]
  anicraft-cli new --out <file.anicraft> [--width W] [--height H] [--fps F]
  anicraft-cli bench [--layers N] [--shapes N] [--frames N]   performance probe (use --release)
  anicraft-cli serve [file|--sample]     JSON-lines on stdin/stdout:
        {\"id\":1,\"cmd\":\"draw.rect\",\"params\":{\"x\":10,\"y\":10,\"w\":50,\"h\":50}}

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
        Some(p) => {
            let doc = format::load(std::path::Path::new(p)).map_err(|e| format!("{p}: {e}"))?;
            let mut s = Session::new(doc);
            s.path = Some(PathBuf::from(p));
            Ok(s)
        }
    }
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
        "commands" => {
            let list: Vec<Value> = commands().iter().map(|c| c.to_json()).collect();
            serde_json::to_string_pretty(&list).map_err(|e| e.to_string())
        }
        "info" => {
            let s = open_session(Some(src.ok_or("info needs a file or --sample")?))?;
            serde_json::to_string_pretty(&s.info()).map_err(|e| e.to_string())
        }
        "render" => {
            let mut s = open_session(Some(src.ok_or("render needs a file or --sample")?))?;
            let out = a.flag("out").ok_or("render needs --out <file.png>")?;
            let mut p = json!({"path": out, "frame": one_based(&a, "frame")?.unwrap_or(0)});
            if let Some(sc) = a.num::<f32>("scale")? {
                p["scale"] = json!(sc);
            }
            exec(&mut s, "file.export_png", &p).map(|v| v.to_string())
        }
        "render-seq" => {
            let mut s = open_session(Some(src.ok_or("render-seq needs a file or --sample")?))?;
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
        "gif" | "sprite-sheet" => {
            let mut s = open_session(Some(src.ok_or_else(|| format!("{cmd} needs a file or --sample"))?))?;
            let out = a.flag("out").ok_or_else(|| format!("{cmd} needs --out <file>"))?;
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
            let id = if cmd == "sprite-sheet" {
                if let Some(columns) = a.num::<u32>("columns")? {
                    p["columns"] = json!(columns);
                }
                if let Some(transparent) = a.flag("transparent") {
                    p["transparent"] = json!(transparent.parse::<bool>().map_err(|_| "--transparent expects true or false")?);
                }
                "file.export_sprite_sheet"
            } else {
                "file.export_gif"
            };
            exec(&mut s, id, &p).map(|v| v.to_string())
        }
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
            let mut s = open_session(Some(src.ok_or("exec needs a file or --sample")?))?;
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
            let mut s = open_session(Some(src.ok_or("run needs a file or --sample")?))?;
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
            let mut s = open_session(src)?;
            let stdin = std::io::stdin();
            let mut stdout = std::io::stdout();
            serve(&mut s, stdin.lock(), &mut stdout).map_err(|e| e.to_string())?;
            Ok(String::new())
        }
        other => Err(format!("unknown subcommand '{other}'\n\n{USAGE}")),
    }
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

/// Performance probe on a generated stress document. Run in release for meaningful numbers:
/// `cargo run --release -p anicraft-cli -- bench`.
pub fn bench(a: &Args) -> Result<String, String> {
    use anicraft_doc::{evaluate, sample};
    use anicraft_engine::history::document_bytes;
    use anicraft_render::{View, render};

    let layers = a.num::<u32>("layers")?.unwrap_or(50);
    let shapes = a.num::<u32>("shapes")?.unwrap_or(200);
    let frames = a.num::<u32>("frames")?.unwrap_or(240);
    let t = Instant::now();
    let doc = sample::stress(layers, shapes, frames);
    let build_ms = t.elapsed().as_secs_f64() * 1000.0;
    let instances: usize = doc.timeline.layers.iter().flat_map(|l| l.keyframes.iter()).map(|k| k.elements.len()).sum();

    let mut json_text = String::new();
    let save_ms = avg_ms(1, |_| json_text = format::to_json(&doc).unwrap_or_default());
    let load_ms = avg_ms(1, |_| {
        let _ = format::from_json(&json_text);
    });

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
        "evaluate_ms_per_frame": round(eval_ms),
        "render_1080p_ms_per_frame": round(render_ms), "render_shapes_drawn": drawn,
        "render_zoom400_ms_per_frame": round(zoom_ms), "render_zoom400_culled": culled,
        "snapshot_clone_us": round(clone_us),
        "edit_command_ms": round(edit_ms), "undo_bytes_per_step": per_step, "undo_ms": round(undo_ms),
        "hit_test_us": round(hit_us),
        "threads": std::thread::available_parallelism().map_or(1, |n| n.get()),
        "build": if cfg!(debug_assertions) { "debug (use --release for real numbers)" } else { "release" },
    });
    serde_json::to_string_pretty(&out).map_err(|e| e.to_string())
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

    #[test]
    fn sprite_sheet_cli_range_columns_and_overwrite() {
        let dir = std::env::temp_dir().join(format!("anicraft-cli-sprites-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("walk.png");
        let mut argv: Vec<String> = ["sprite-sheet", "--sample", "--from", "2", "--to", "4", "--columns", "2", "--scale", "0.01", "--transparent", "false", "--out", out.to_str().unwrap()].iter().map(|s| s.to_string()).collect();
        let result: Value = serde_json::from_str(&run(&argv).unwrap()).unwrap();
        assert_eq!(result["frames"], 3);
        let metadata: Value = serde_json::from_slice(&std::fs::read(out.with_extension("json")).unwrap()).unwrap();
        assert_eq!(metadata["frames"][0]["frame"], 1, "CLI frames are 1-based, metadata is 0-based");
        assert_eq!(metadata["frames"][2]["frame"], 3);
        assert_eq!(metadata["columns"], 2);
        assert!(run(&argv).is_err());
        argv.extend(["--overwrite".into(), "yes".into()]);
        run(&argv).unwrap();
        argv.extend(["--from".into(), "0".into()]);
        assert!(run(&argv).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn usage_and_errors() {
        assert!(run(&[]).unwrap().contains("USAGE"));
        assert!(run(&["bogus".into()]).is_err());
        assert!(run(&["info".into()]).is_err());
        assert!(run(&["commands".into()]).unwrap().contains("timeline.insert_keyframe"));
    }
}
