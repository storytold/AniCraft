//! `anicraft-cli mcp`: a Model Context Protocol server on stdin/stdout.
//!
//! The transport is MCP's stdio transport: JSON-RPC 2.0 messages, one per line. AI assistants
//! (any MCP client) get a handful of tools over the engine's command registry rather than one
//! tool per command: list the commands, run one or a batch, read the document, and look at a
//! frame as a picture. Everything still goes through `Session::execute`, so undo, validation
//! and the confirmation tiers (ADR 0004: `"confirm": true` before irreversible loss) apply to
//! an assistant exactly as to the app.

use anicraft_engine::{Session, commands};
use serde_json::{Value, json};
use std::io::{BufRead, Write};

/// Protocol revisions this server speaks, newest first.
const PROTOCOLS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// Longest edge of a frame picture, in pixels (keeps replies small).
const MAX_PICTURE_EDGE: f32 = 2048.0;

/// Most steps in one `anicraft_run_script` call.
const MAX_STEPS: usize = 10_000;

const INSTRUCTIONS: &str = "AniCraft is a 2D animation studio. Every edit is a command: call \
anicraft_commands to find one (filter by words such as 'tween' or 'export'), then anicraft_run \
with its id and params, or anicraft_run_script for several in order. Frames in params are \
0-based. Commands act on the current scene, layer, frame and selection, which anicraft_document \
reports. Use anicraft_render to see a frame. Edits can be undone with edit.undo. Commands that \
would lose work or replace a file refuse until params include \"confirm\": true; ask the person \
before confirming.";

fn tools() -> Value {
    json!([
        {
            "name": "anicraft_commands",
            "title": "List AniCraft commands",
            "description": "Lists the commands anicraft_run accepts: id, label, menu, shortcut, a summary of their params, and whether they change the document. Optionally only those whose id or label contains `filter`.",
            "inputSchema": {"type": "object", "properties": {"filter": {"type": "string", "description": "Words to look for in the id or label, e.g. \"tween\" or \"export\"."}}},
            "annotations": {"readOnlyHint": true, "openWorldHint": false}
        },
        {
            "name": "anicraft_run",
            "title": "Run an AniCraft command",
            "description": "Runs one command (see anicraft_commands) with its params and returns its result, plus the document revision and current frame. Commands that would lose work or replace a file need \"confirm\": true in params.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "command": {"type": "string", "description": "Command id, e.g. \"draw.rect\"."},
                    "params": {"type": "object", "description": "The command's params, e.g. {\"x\": 10, \"y\": 10, \"w\": 50, \"h\": 50}."}
                },
                "required": ["command"]
            },
            "annotations": {"readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": false}
        },
        {
            "name": "anicraft_run_script",
            "title": "Run several AniCraft commands",
            "description": "Runs commands in order and returns each result. Stops at the first error and says which step failed; the steps before it stay done (undo them with edit.undo).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "steps": {
                        "type": "array",
                        "items": {"type": "object", "properties": {"command": {"type": "string"}, "params": {"type": "object"}}, "required": ["command"]}
                    }
                },
                "required": ["steps"]
            },
            "annotations": {"readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": false}
        },
        {
            "name": "anicraft_document",
            "title": "Describe the document",
            "description": "The open document: stage size and frame rate, scenes, the current scene's layers and keyframes, library symbols, and the current scene, frame, layer and selection.",
            "inputSchema": {"type": "object", "properties": {}},
            "annotations": {"readOnlyHint": true, "openWorldHint": false}
        },
        {
            "name": "anicraft_render",
            "title": "Look at a frame",
            "description": "A frame of the current scene as a PNG picture, exactly as exported. `frame` is 0-based (default: the current frame); `scale` defaults to 0.5 of the stage size.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "frame": {"type": "integer", "minimum": 0},
                    "scale": {"type": "number", "exclusiveMinimum": 0, "maximum": 4}
                }
            },
            "annotations": {"readOnlyHint": true, "openWorldHint": false}
        }
    ])
}

/// A JSON-RPC error object.
fn error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}})
}

fn text_result(v: &Value, is_error: bool) -> Value {
    let text = if let Value::String(s) = v { s.clone() } else { serde_json::to_string_pretty(v).unwrap_or_default() };
    json!({"content": [{"type": "text", "text": text}], "isError": is_error})
}

/// Runs one tool. Tool failures are results with `isError` (so the assistant can read and
/// correct them); only malformed calls are protocol errors.
fn call_tool(s: &mut Session, name: &str, args: &Value) -> Result<Value, String> {
    let run = |s: &mut Session, step: &Value| -> Result<Value, String> {
        let id = step.get("command").and_then(Value::as_str).ok_or("missing \"command\"")?;
        let params = step.get("params").cloned().unwrap_or_else(|| json!({}));
        if !params.is_object() {
            return Err(format!("{id}: \"params\" must be an object"));
        }
        crate::refuse_remote_ffmpeg(id, &params).map_err(|e| format!("{id}: {e}"))?;
        s.execute(id, &params).map_err(|e| format!("{id}: {e}"))
    };
    Ok(match name {
        "anicraft_commands" => {
            let filter = args.get("filter").and_then(Value::as_str).unwrap_or("").to_lowercase();
            let words: Vec<&str> = filter.split_whitespace().collect();
            let list: Vec<Value> = commands()
                .iter()
                .filter(|c| {
                    let hay = format!("{} {}", c.id, c.label).to_lowercase();
                    words.iter().all(|w| hay.contains(w))
                })
                .map(|c| c.to_json())
                .collect();
            text_result(&Value::Array(list), false)
        }
        "anicraft_run" => match run(s, args) {
            Ok(v) => text_result(&json!({"result": v, "revision": s.revision, "frame": s.frame, "scene": s.scene}), false),
            Err(e) => text_result(&Value::String(e), true),
        },
        "anicraft_run_script" => {
            let steps = args.get("steps").and_then(Value::as_array).ok_or("\"steps\" must be an array")?;
            if steps.len() > MAX_STEPS {
                return Err(format!("at most {MAX_STEPS} steps per call"));
            }
            let mut results = Vec::with_capacity(steps.len());
            for (i, step) in steps.iter().enumerate() {
                match run(s, step) {
                    Ok(v) => results.push(v),
                    Err(e) => return Ok(text_result(&json!({"failed_step": i + 1, "error": e, "done": results}), true)),
                }
            }
            text_result(&json!({"results": results, "revision": s.revision, "frame": s.frame}), false)
        }
        "anicraft_document" => text_result(&s.info(), false),
        "anicraft_render" => {
            let frame = match args.get("frame") {
                None | Some(Value::Null) => s.frame,
                Some(v) => v.as_u64().and_then(|f| u32::try_from(f).ok()).ok_or("\"frame\" must be a whole number from 0")?,
            };
            if frame >= s.frame_count() {
                return Ok(text_result(&Value::String(format!("frame {frame} is past the end: the scene has {} frames (0 to {})", s.frame_count(), s.frame_count().saturating_sub(1))), true));
            }
            let stage = s.doc.stage;
            let largest = stage.width.max(stage.height).max(1) as f32;
            let scale = match args.get("scale") {
                None | Some(Value::Null) => 0.5,
                Some(v) => v.as_f64().filter(|x| x.is_finite() && *x > 0.0 && *x <= 4.0).ok_or("\"scale\" must be a number above 0, up to 4")? as f32,
            };
            let scale = scale.min(MAX_PICTURE_EDGE / largest);
            let movie_frame = s.doc.scene_start(s.scene).saturating_add(frame);
            let png = anicraft_render::render_frame(&s.doc, movie_frame, scale).and_then(|p| p.encode_png()).map_err(|e| e.to_string())?;
            json!({
                "content": [
                    {"type": "image", "data": anicraft_doc::base64_encode(&png), "mimeType": "image/png"},
                    {"type": "text", "text": format!("Frame {frame} of scene {} at {:.0}% ({}Ã—{} stage).", s.scene, scale * 100.0, stage.width, stage.height)}
                ],
                "isError": false
            })
        }
        other => return Err(format!("unknown tool '{other}'")),
    })
}

/// Answers one JSON-RPC message; `None` for notifications (and responses), which get no reply.
pub fn handle(s: &mut Session, line: &str) -> Option<Value> {
    let msg: Value = match serde_json::from_str(line.trim_start_matches('\u{feff}')) {
        Ok(v) => v,
        Err(e) => return Some(error(Value::Null, -32700, format!("parse error: {e}"))),
    };
    if !msg.is_object() {
        return Some(error(Value::Null, -32600, "expected a JSON-RPC request object"));
    }
    let method = msg.get("method").and_then(Value::as_str);
    let Some(id) = msg.get("id").cloned().filter(|v| !v.is_null()) else {
        // Notifications (`notifications/initialized`, `notifications/cancelled`) and replies to
        // requests we never send: nothing to answer.
        return None;
    };
    let Some(method) = method else { return Some(error(id, -32600, "missing \"method\"")) };
    let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));
    let result = match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("");
            let version = PROTOCOLS.iter().find(|v| **v == asked).copied().unwrap_or(PROTOCOLS[0]);
            json!({
                "protocolVersion": version,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "anicraft", "title": "AniCraft", "version": env!("CARGO_PKG_VERSION")},
                "instructions": INSTRUCTIONS
            })
        }
        "ping" => json!({}),
        "tools/list" => json!({"tools": tools()}),
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else { return Some(error(id, -32602, "tools/call needs a tool \"name\"")) };
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            match call_tool(s, name, &args) {
                Ok(v) => v,
                Err(e) => return Some(error(id, -32602, e)),
            }
        }
        other => return Some(error(id, -32601, format!("method not found: {other}"))),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

/// The stdio loop: one message per line in, one reply per line out, until stdin closes.
/// Nothing but protocol messages is written to `out` (logs would corrupt the stream).
pub fn serve<R: BufRead, W: Write>(s: &mut Session, input: R, out: &mut W) -> std::io::Result<()> {
    let mut input = input;
    loop {
        let line = match crate::read_line(&mut input, crate::MAX_LINE)? {
            crate::Line::Eof => break,
            crate::Line::TooLong => {
                writeln!(out, "{}", json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32600, "message": "request line too long"}}))?;
                out.flush()?;
                continue;
            }
            crate::Line::Text(l) => l,
        };
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = handle(s, &line) {
            writeln!(out, "{reply}")?;
            out.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(s: &mut Session, id: u64, method: &str, params: Value) -> Value {
        handle(s, &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string()).unwrap()
    }

    fn tool(s: &mut Session, name: &str, args: Value) -> Value {
        call(s, 9, "tools/call", json!({"name": name, "arguments": args}))["result"].clone()
    }

    fn text(result: &Value) -> Value {
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap_or_else(|_| result["content"][0]["text"].clone())
    }

    #[test]
    fn handshake_lists_tools_and_ignores_notifications() {
        let mut s = Session::default();
        let init = call(&mut s, 1, "initialize", json!({"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}));
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26", "a version we speak is echoed");
        assert_eq!(init["result"]["serverInfo"]["name"], "anicraft");
        let newer = call(&mut s, 2, "initialize", json!({"protocolVersion": "2099-01-01"}));
        assert_eq!(newer["result"]["protocolVersion"], PROTOCOLS[0], "otherwise our newest");
        assert!(handle(&mut s, r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
        assert_eq!(call(&mut s, 3, "ping", json!({}))["result"], json!({}));
        let names: Vec<String> = call(&mut s, 4, "tools/list", json!({}))["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
        assert_eq!(names, ["anicraft_commands", "anicraft_run", "anicraft_run_script", "anicraft_document", "anicraft_render"]);
        // Protocol errors carry JSON-RPC codes.
        assert_eq!(call(&mut s, 5, "nope", json!({}))["error"]["code"], -32601);
        assert_eq!(handle(&mut s, "{oops").unwrap()["error"]["code"], -32700);
        assert_eq!(call(&mut s, 6, "tools/call", json!({"name": "nope"}))["error"]["code"], -32602);
    }

    #[test]
    fn mcp_refuses_a_caller_supplied_ffmpeg() {
        let mut s = Session::default();
        for r in [
            tool(&mut s, "anicraft_run", json!({"command": "file.export_video", "params": {"path": "a.mp4", "ffmpeg": "/bin/sh"}})),
            tool(&mut s, "anicraft_run_script", json!({"steps": [{"command": "file.export_video", "params": {"path": "a.mp4", "ffmpeg": "/bin/sh"}}]})),
        ] {
            assert_eq!(r["isError"], true);
            assert!(r["content"][0]["text"].as_str().unwrap().contains("ANICRAFT_FFMPEG"), "{r}");
        }
    }

    #[test]
    fn tools_drive_the_engine() {
        let mut s = Session::default();
        let found = text(&tool(&mut s, "anicraft_commands", json!({"filter": "shape tween"})));
        assert!(found.as_array().unwrap().iter().any(|c| c["id"] == "timeline.create_shape_tween"), "{found}");
        let r = tool(&mut s, "anicraft_run", json!({"command": "draw.rect", "params": {"x": 10, "y": 10, "w": 50, "h": 40, "fill": "#ff0000"}}));
        assert_eq!(r["isError"], false);
        assert_eq!(s.visible_elements().len(), 1);
        // A failing command is a tool result the assistant can read, not a protocol error.
        let bad = tool(&mut s, "anicraft_run", json!({"command": "draw.rect", "params": {"x": "left"}}));
        assert_eq!(bad["isError"], true);
        assert!(bad["content"][0]["text"].as_str().unwrap().starts_with("draw.rect:"));
        // Scripts run in order and stop at the first failure.
        let script = tool(&mut s, "anicraft_run_script", json!({"steps": [
            {"command": "draw.ellipse", "params": {"x": 100, "y": 10, "w": 30, "h": 30}},
            {"command": "no.such"},
            {"command": "draw.ellipse", "params": {"x": 200, "y": 10, "w": 30, "h": 30}}
        ]}));
        assert_eq!(script["isError"], true);
        assert_eq!(text(&script)["failed_step"], 2);
        assert_eq!(s.visible_elements().len(), 2, "the step before the failure stays done");
        // Confirmation tiers apply: discarding unsaved work needs "confirm".
        assert_eq!(tool(&mut s, "anicraft_run", json!({"command": "file.new"}))["isError"], true);
        let doc = text(&tool(&mut s, "anicraft_document", json!({})));
        assert_eq!(doc["stage"]["width"], s.doc.stage.width);
    }

    #[test]
    fn render_returns_a_png_picture() {
        let mut s = Session::new(anicraft_doc::sample::demo());
        let r = tool(&mut s, "anicraft_render", json!({"frame": 3, "scale": 0.25}));
        assert_eq!(r["content"][0]["type"], "image");
        assert_eq!(r["content"][0]["mimeType"], "image/png");
        assert!(r["content"][0]["data"].as_str().unwrap().starts_with("iVBORw0KGgo"), "base64 of the PNG signature");
        assert_eq!(tool(&mut s, "anicraft_render", json!({"frame": 1_000_000}))["isError"], true);
        assert_eq!(call(&mut s, 7, "tools/call", json!({"name": "anicraft_render", "arguments": {"scale": -1}}))["error"]["code"], -32602);
    }

    #[test]
    fn stdio_loop_answers_requests_only() {
        let mut s = Session::default();
        let input = b"\xef\xbb\xbf{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n\n{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n{\"jsonrpc\":\"2.0\",\"id\":\"b\",\"method\":\"tools/list\"}\n";
        let mut out = Vec::new();
        serve(&mut s, &input[..], &mut out).unwrap();
        let lines: Vec<Value> = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["id"], 1);
        assert_eq!(lines[1]["id"], "b");
    }
}
