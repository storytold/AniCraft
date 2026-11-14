//! Saved commands (command macros, ADR 0097): steps picked in the History panel, kept as a
//! small JSON file and run again as one undo step.
//!
//! Every undo step remembers the commands it ran (`history::Recorded`). Save As Command turns
//! the picked steps into a saved command, rewriting what only makes sense in this document
//! (an element's or layer's id) to act on the current selection or active layer where the
//! command allows it, and refusing, with the step named, what can't be kept. A saved command
//! may call what a plug-in may (`plugin_may_call`) and never the `macro.*` commands, so it
//! can't recurse, touch files or the clipboard.
//!
//! A saved command is a file `<folder>/<name>.json`: `{"format": "anicraft-command",
//! "version": 1, "steps": [{"cmd", "params", "label"}]}`. The file's name is the command's
//! name, so a shared file keeps whatever name it is given. `anicraft-cli run` scripts (a bare
//! JSON array of `{cmd, params}`) run the same way. Their order in the Commands menu is kept
//! in `<folder>/.order.json`. The shell passes the folder (its settings folder's `Commands`);
//! the engine never looks for it, as with templates.

use crate::history::Recorded;
use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session, vfs};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

/// The `format` a saved command's file declares.
pub const FORMAT: &str = "anicraft-command";
/// The newest file version this build reads and the one it writes.
pub const VERSION: u64 = 1;
/// Most steps in one saved command.
pub const MAX_STEPS: usize = 1000;
/// Largest saved command file read.
pub const MAX_FILE_BYTES: u64 = 1 << 20;
/// Most saved commands listed from one folder.
pub const MAX_LISTED: usize = 500;
/// The file that keeps the order of the saved commands (a dot-name: no command is called that).
pub const ORDER_FILE: &str = ".order.json";
/// Longest command id and step label kept from a file.
const MAX_CMD_CHARS: usize = 128;
const MAX_LABEL_CHARS: usize = 200;

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, params, mutates, run| CommandSpec { id, label, menu, shortcut: None, params, mutates, run };
    vec![
        c("macro.run", "Run Command…", "Commands", "{name + folder (a saved command) | path (a saved command's file, or a JSON array of {cmd, params}) | steps: [{cmd, params?, label?}], name? (for steps)} → {name, steps, results}: runs the steps in order as one undo step named after it. At the first step that fails nothing is changed and the error names the step. Steps may call what a plug-in may, never macro.*", true, run as crate::RunFn),
        c("macro.list", "Manage Saved Commands…", "Commands", "{folder} → {commands: [{name, path, steps}], problems: [{file, error}]}: the saved commands in the folder, in their order (the app shows them in a window)", false, list),
        c("macro.history", "List History Steps", "", "{} → {steps: [{index, label, undone, keep, commands?, why?, rewritten?}]}: every undo-history step as Save As Command keeps it: `keep` false says `why` it can't be kept; `rewritten` names params dropped so the step acts on the selection or active layer", false, history),
        c("macro.save", "Save As Command…", "", "{folder, name, history: [index, ..] (as macro.history lists them) | steps: [{cmd, params?, label?}], confirm? (to replace a saved command of that name)} → {name, path, steps, rewritten}: keeps the steps as <folder>/<name>.json, last in the list", false, save),
        c("macro.rename", "Rename Saved Command", "", "{folder, name, to} → {name, from, path}: renames a saved command (its file); its place in the list stays", false, rename),
        c("macro.delete", "Delete Saved Command", "", "{folder, name, confirm}: deletes a saved command's file (cannot be undone)", false, delete),
        c("macro.move", "Move Saved Command", "", "{folder, name, index (its new place in the list, 0-based)}", false, move_to),
        c("macro.import", "Add Saved Command from File", "", "{folder, path (a saved command's file), name? (default: the file's), confirm? (to replace one of that name)} → {name, path, steps}: checks it and copies it in, last in the list", false, import),
    ]
}

/// Whether a saved command may run command `id`, and if not, why: what a plug-in may, and
/// never a `macro.*` command (a saved command doesn't run, save or change saved commands).
pub fn macro_may_call(id: &str) -> std::result::Result<(), String> {
    if id.split('.').next() == Some("macro") {
        return Err("a saved command can't run, save or change saved commands".into());
    }
    crate::cmds_plugin::plugin_may_call(id).map_err(|why| format!("{why} (a saved command may do only what a plug-in may)"))
}

/// One step of a saved command.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub cmd: String,
    /// Always an object.
    pub params: Value,
    /// What the History panel called it (may be empty: then the command's label).
    pub label: String,
}

impl Step {
    /// The step's name in messages.
    pub fn name(&self) -> String {
        if !self.label.is_empty() {
            return self.label.clone();
        }
        crate::find_command(&self.cmd).map_or_else(|| self.cmd.clone(), |c| c.label.trim_end_matches('…').to_string())
    }
}

/// A saved command: its name and steps.
#[derive(Clone, Debug, PartialEq)]
pub struct Macro {
    pub name: String,
    pub steps: Vec<Step>,
}

impl Macro {
    /// The file's content.
    pub fn to_json(&self) -> Value {
        let steps: Vec<Value> = self.steps.iter().map(|s| json!({"cmd": s.cmd, "params": s.params, "label": s.label})).collect();
        json!({"format": FORMAT, "version": VERSION, "steps": steps})
    }

    /// Reads a saved command named `name`: a file's object, or a bare array of steps (an
    /// `anicraft-cli run` script). Every step is checked: a known command a saved command may
    /// call, with object params.
    pub fn from_json(v: &Value, name: &str) -> Result<Macro> {
        let steps = match v {
            Value::Array(a) => a,
            Value::Object(o) => {
                if let Some(f) = o.get("format")
                    && f.as_str() != Some(FORMAT)
                {
                    return Err(EngineError::Format(format!("not a saved command (its format is {f}, not \"{FORMAT}\")")));
                }
                if let Some(v) = o.get("version") {
                    let v = v.as_u64().ok_or_else(|| EngineError::Format("a saved command's version is a whole number".into()))?;
                    if v > VERSION {
                        return Err(EngineError::Format(format!("this saved command was made by a newer AniCraft (version {v}); update AniCraft to run it")));
                    }
                }
                o.get("steps").and_then(Value::as_array).ok_or_else(|| EngineError::Format("a saved command has a \"steps\" list".into()))?
            }
            _ => return Err(EngineError::Format("a saved command is a JSON object with \"steps\", or a list of {cmd, params}".into())),
        };
        if steps.len() > MAX_STEPS {
            return Err(EngineError::Format(format!("a saved command has at most {MAX_STEPS} steps (this one has {})", steps.len())));
        }
        let mut out = Vec::with_capacity(steps.len());
        for (i, st) in steps.iter().enumerate() {
            let n = i + 1;
            let o = st.as_object().ok_or_else(|| EngineError::Format(format!("step {n} is not an object {{cmd, params}}")))?;
            let cmd = o.get("cmd").or_else(|| o.get("command")).and_then(Value::as_str).ok_or_else(|| EngineError::Format(format!("step {n} has no \"cmd\"")))?;
            if cmd.chars().count() > MAX_CMD_CHARS || crate::find_command(cmd).is_none() {
                let shown: String = cmd.chars().take(40).collect();
                return Err(EngineError::Format(format!("step {n}: unknown command '{shown}'")));
            }
            macro_may_call(cmd).map_err(|why| EngineError::Format(format!("step {n} ({cmd}) can't be in a saved command: {why}")))?;
            let params = match o.get("params") {
                None | Some(Value::Null) => Value::Object(Map::new()),
                Some(p @ Value::Object(_)) => p.clone(),
                Some(_) => return Err(EngineError::Format(format!("step {n} ({cmd}): params are an object"))),
            };
            let label: String = o.get("label").and_then(Value::as_str).unwrap_or("").chars().filter(|c| !c.is_control()).take(MAX_LABEL_CHARS).collect();
            out.push(Step { cmd: cmd.to_string(), params, label });
        }
        Ok(Macro { name: name.to_string(), steps: out })
    }
}

/// The file of the saved command called `name` in `folder`.
pub fn command_path(folder: &Path, name: &str) -> Result<PathBuf> {
    let name = crate::cmds_template::clean_name(name, "command")?;
    Ok(folder.join(format!("{name}.json")))
}

/// Reads a saved command's file (its name is the file's).
pub fn load(path: &Path) -> Result<Macro> {
    let len = vfs::len(path).map_err(|e| EngineError::Io(std::io::Error::new(e.kind(), format!("{}: {e}", path.display()))))?;
    if len > MAX_FILE_BYTES {
        return Err(EngineError::Format(format!("{} is larger than a saved command can be ({} KiB at most)", path.display(), MAX_FILE_BYTES / 1024)));
    }
    let text = vfs::read_to_string(path)?;
    // Windows editors often save UTF-8 with a byte-order mark; JSON parsers reject it.
    let v: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| EngineError::Format(format!("{}: not JSON ({e})", path.display())))?;
    let name = path.file_stem().map_or_else(|| "Command".to_string(), |n| n.to_string_lossy().into_owned());
    Macro::from_json(&v, &name)
}

/// A saved command found in the folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Saved {
    pub name: String,
    pub path: PathBuf,
    pub steps: usize,
}

fn read_order(folder: &Path) -> Vec<String> {
    let path = folder.join(ORDER_FILE);
    if vfs::len(&path).map_or(true, |n| n > MAX_FILE_BYTES) {
        return Vec::new();
    }
    let v: Value = vfs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(t.trim_start_matches('\u{feff}')).ok()).unwrap_or(Value::Null);
    v.as_array().into_iter().flatten().filter_map(Value::as_str).take(MAX_LISTED).map(str::to_string).collect()
}

fn write_order(folder: &Path, names: &[String]) -> Result<()> {
    let text = serde_json::to_string_pretty(names).map_err(|e| EngineError::Format(e.to_string()))?;
    vfs::write(&folder.join(ORDER_FILE), text)?;
    Ok(())
}

/// The saved commands in `folder`, in their order (then by name), and the files that aren't
/// saved commands with why: `(file name, error)`.
pub fn saved_commands(folder: &Path) -> (Vec<Saved>, Vec<(String, String)>) {
    let order: Vec<String> = read_order(folder).into_iter().map(|n| n.to_lowercase()).collect();
    let mut found = Vec::new();
    let mut problems = Vec::new();
    for f in vfs::list(folder, MAX_LISTED * 2) {
        let Some(stem) = f.file_stem().map(|n| n.to_string_lossy().into_owned()) else { continue };
        if stem.starts_with('.') || !f.extension().is_some_and(|e| e.eq_ignore_ascii_case("json")) || !vfs::is_file(&f) {
            continue;
        }
        let file = f.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        match load(&f) {
            Ok(m) if found.len() < MAX_LISTED => found.push(Saved { name: m.name, path: f, steps: m.steps.len() }),
            Ok(_) => {}
            Err(e) => problems.push((file, e.to_string())),
        }
    }
    found.sort_by_key(|s| (order.iter().position(|o| *o == s.name.to_lowercase()).unwrap_or(usize::MAX), s.name.to_lowercase()));
    problems.sort();
    (found, problems)
}

fn folder(p: &Value) -> Result<PathBuf> {
    let f = str_req(p, "folder")?;
    if f.trim().is_empty() {
        return Err(EngineError::Param("name the saved commands' folder".into()));
    }
    Ok(PathBuf::from(f))
}

/// The saved command file the params name (`folder` and `name`).
pub(crate) fn target(p: &Value) -> Result<PathBuf> {
    command_path(&folder(p)?, str_req(p, "name")?)
}

/// Where `macro.import` copies its file to.
pub(crate) fn import_target(p: &Value) -> Result<PathBuf> {
    let from = PathBuf::from(str_req(p, "path")?);
    let name = match opt_str(p, "name")? {
        Some(n) => n.to_string(),
        None => from.file_stem().map(|n| n.to_string_lossy().into_owned()).ok_or_else(|| EngineError::Param("the file has no name".into()))?,
    };
    command_path(&folder(p)?, &name)
}

fn name_of(path: &Path) -> String {
    path.file_stem().map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

/// The saved command the params name: by `name` in `folder`, a file's `path`, or `steps`.
fn resolve(p: &Value) -> Result<Macro> {
    if let Some(steps) = p.get("steps") {
        let name = opt_str(p, "name")?.map_or_else(|| "Saved command".to_string(), |n| n.chars().filter(|c| !c.is_control()).take(crate::cmds_template::MAX_NAME_CHARS).collect());
        return Macro::from_json(steps, &name);
    }
    if let Some(path) = opt_str(p, "path")? {
        return load(Path::new(path));
    }
    let path = target(p).map_err(|e| EngineError::Param(format!("name a saved command (name and folder), a file (path) or steps: {e}")))?;
    if !vfs::is_file(&path) {
        return Err(EngineError::State(format!("there is no saved command “{}”", name_of(&path))));
    }
    load(&path)
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let m = resolve(p)?;
    if m.steps.is_empty() {
        return Err(EngineError::State(format!("“{}” has no steps to run", m.name)));
    }
    let n = m.steps.len();
    let mut results = Vec::with_capacity(n);
    for (i, st) in m.steps.iter().enumerate() {
        // (Checked when it was read; the rule is the run's, so it is checked here too.)
        let r = macro_may_call(&st.cmd).map_err(EngineError::State).and_then(|()| s.execute_nested(&st.cmd, &st.params));
        match r {
            Ok(v) => results.push(v),
            // The session puts the document back as it was before the run.
            Err(e) => return Err(EngineError::State(format!("“{}” stopped at step {} of {n} ({}): {e}. Nothing was changed.", m.name, i + 1, st.name()))),
        }
    }
    s.step_label = Some(m.name.clone());
    // History keeps the steps, so a run can itself be saved again with other steps.
    s.record_as = Some(m.steps.iter().map(|st| Recorded::new(&st.cmd, &st.params)).collect());
    Ok(json!({"name": m.name, "steps": n, "results": results}))
}

fn list(_: &mut Session, p: &Value) -> Result<Value> {
    let (found, problems) = saved_commands(&folder(p)?);
    let commands: Vec<Value> = found.iter().map(|c| json!({"name": c.name, "path": c.path.display().to_string(), "steps": c.steps})).collect();
    let problems: Vec<Value> = problems.iter().map(|(f, e)| json!({"file": f, "error": e})).collect();
    Ok(json!({"commands": commands, "problems": problems}))
}

/// Params naming who a step acts on: dropped, where the command then acts on the selection
/// (or, for layers, the active layer), so the step replays elsewhere.
const TARGETS: &[&str] = &["layer", "layers", "element", "elements", "id", "ids", "shape", "path", "symbol"];
/// Params naming things that exist only in this document: Library items, folders and bones.
const FIXED: &[&str] = &["bitmap", "sound", "folder", "bone", "parent", "items"];

/// What a param names, in a sentence.
fn noun(key: &str) -> &'static str {
    match key {
        "layer" | "layers" => "particular layers",
        "symbol" | "bitmap" | "sound" | "folder" | "items" => "particular Library items",
        "bone" | "parent" | "from" | "to" => "particular bones",
        _ => "particular objects",
    }
}

/// Whether the command's params doc makes `key` optional (`key?`), and, but for layers, says
/// it defaults to what is selected.
fn defaults_to_selection(doc: &str, key: &str) -> bool {
    let pat = format!("{key}?");
    doc.match_indices(&pat).any(|(i, _)| {
        let whole = doc.get(..i).and_then(|b| b.chars().next_back()).is_none_or(|c| !c.is_alphanumeric() && c != '_');
        let after: String = doc.get(i + pat.len()..).unwrap_or("").chars().take(90).collect();
        whole && (key.starts_with("layer") || after.contains("select"))
    })
}

/// Whether a recorded command can be kept: the params it would drop (they name this
/// document's layers or objects and the command can act on the selection or active layer
/// instead), or why it can't be kept. Reads only: listing a long history costs no copies.
fn check(cmd: &str, params: &Value) -> std::result::Result<Vec<&'static str>, String> {
    macro_may_call(cmd)?;
    let spec = crate::find_command(cmd).ok_or_else(|| format!("'{cmd}' is not a command any more"))?;
    let mut dropped = Vec::new();
    for (k, v) in params.as_object().into_iter().flatten() {
        let fixed = FIXED.contains(&k.as_str()) || (cmd.starts_with("bone.") && matches!(k.as_str(), "from" | "to")) || (cmd == "library.move" && k == "to");
        let target = TARGETS.iter().find(|t| **t == k.as_str());
        match target {
            _ if v.is_null() => {}
            Some(t) if !fixed && defaults_to_selection(spec.params, t) => dropped.push(*t),
            None if !fixed => {}
            _ => return Err(format!("it works on {} of this document", noun(k))),
        }
    }
    Ok(dropped)
}

/// A recorded command as a saved command keeps it: its params without what names this
/// document's layers and objects (and which of those went), or why it can't be kept.
pub fn portable(cmd: &str, params: &Value) -> std::result::Result<(Value, Vec<String>), String> {
    let dropped = check(cmd, params)?;
    let out: Map<String, Value> = params.as_object().into_iter().flatten().filter(|(k, _)| !dropped.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
    Ok((Value::Object(out), dropped.into_iter().map(str::to_string).collect()))
}

/// An undo step as a saved command keeps it: its steps and the params dropped, or why not.
fn keep_step(label: &str, cmds: Option<&[Recorded]>) -> std::result::Result<(Vec<Step>, Vec<String>), String> {
    let cmds = usable(cmds)?;
    let mut steps = Vec::with_capacity(cmds.len());
    let mut dropped = Vec::new();
    for r in cmds {
        let (params, gone) = portable(&r.cmd, &r.params)?;
        for g in gone {
            if !dropped.contains(&g) {
                dropped.push(g);
            }
        }
        steps.push(Step { cmd: r.cmd.clone(), params, label: label.to_string() });
    }
    Ok((steps, dropped))
}

fn usable(cmds: Option<&[Recorded]>) -> std::result::Result<&[Recorded], String> {
    let cmds = cmds.ok_or("it ran too many commands to keep (a very long drag?)")?;
    if cmds.is_empty() {
        return Err("it has nothing to run again".into());
    }
    Ok(cmds)
}

/// [`keep_step`] without building the steps: how many commands, and the params dropped.
fn check_step(cmds: Option<&[Recorded]>) -> std::result::Result<(usize, Vec<String>), String> {
    let cmds = usable(cmds)?;
    let mut dropped: Vec<String> = Vec::new();
    for r in cmds {
        for g in check(&r.cmd, &r.params)? {
            if !dropped.iter().any(|d| d == g) {
                dropped.push(g.to_string());
            }
        }
    }
    Ok((cmds.len(), dropped))
}

/// What dropped params mean for the step, in a sentence.
fn rewritten_note(dropped: &[String]) -> Option<String> {
    let layers = dropped.iter().any(|d| d.starts_with("layer"));
    let objects = dropped.iter().any(|d| !d.starts_with("layer"));
    match (objects, layers) {
        (true, true) => Some("acts on the selection and the active layer".into()),
        (true, false) => Some("acts on the selection".into()),
        (false, true) => Some("acts on the active layer".into()),
        (false, false) => None,
    }
}

fn history(s: &mut Session, _: &Value) -> Result<Value> {
    let done = s.history().undo_len();
    let steps: Vec<Value> = s
        .history()
        .steps()
        .enumerate()
        .map(|(i, (label, cmds))| {
            let mut v = json!({"index": i, "label": label, "undone": i >= done});
            match check_step(cmds) {
                Ok((n, dropped)) => {
                    v["keep"] = json!(true);
                    v["commands"] = json!(n);
                    if let Some(note) = rewritten_note(&dropped) {
                        v["rewritten"] = json!(note);
                    }
                }
                Err(why) => {
                    v["keep"] = json!(false);
                    v["why"] = json!(why);
                }
            }
            v
        })
        .collect();
    Ok(json!({"steps": steps}))
}

/// The steps the params ask to save: history steps by index, or steps given outright; and
/// what was rewritten, a sentence per step.
fn steps_to_save(s: &Session, p: &Value) -> Result<(Vec<Step>, Vec<Value>)> {
    if let Some(v) = p.get("steps") {
        return Ok((Macro::from_json(v, "")?.steps, Vec::new()));
    }
    let mut picked = opt_u64_list(p, "history")?.ok_or_else(|| EngineError::Param("pick the steps to keep: history (as macro.history lists them) or steps".into()))?;
    picked.sort_unstable();
    picked.dedup();
    if picked.is_empty() {
        return Err(EngineError::Param("pick at least one step to keep".into()));
    }
    let all: Vec<(&str, Option<&[Recorded]>)> = s.history().steps().collect();
    let mut steps = Vec::new();
    let mut notes = Vec::new();
    for i in picked {
        let (label, cmds) = usize::try_from(i).ok().and_then(|i| all.get(i).copied()).ok_or_else(|| EngineError::Param(format!("there is no history step {i} (macro.history lists them)")))?;
        let (kept, dropped) = keep_step(label, cmds).map_err(|why| EngineError::State(format!("step {} “{label}” can't be kept: {why}", i + 1)))?;
        if let Some(note) = rewritten_note(&dropped) {
            notes.push(json!({"index": i, "label": label, "note": note}));
        }
        steps.extend(kept);
        if steps.len() > MAX_STEPS {
            return Err(EngineError::State(format!("a saved command has at most {MAX_STEPS} steps")));
        }
    }
    Ok((steps, notes))
}

/// Adds `name` to the end of the folder's order (or leaves it where it is).
fn order_add(folder: &Path, name: &str) -> Result<()> {
    let mut order = read_order(folder);
    if !order.iter().any(|o| o.to_lowercase() == name.to_lowercase()) {
        // The order holds every listed command, so a new one goes after all of them.
        let (found, _) = saved_commands(folder);
        let mut names: Vec<String> = found.into_iter().map(|c| c.name).filter(|n| n.to_lowercase() != name.to_lowercase()).collect();
        names.push(name.to_string());
        order = names;
    }
    write_order(folder, &order)
}

fn write_macro(path: &Path, m: &Macro) -> Result<()> {
    if let Some(dir) = path.parent() {
        vfs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(&m.to_json()).map_err(|e| EngineError::Format(e.to_string()))?;
    vfs::write(path, text)?;
    Ok(())
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    let path = target(p)?;
    let (steps, rewritten) = steps_to_save(s, p)?;
    if steps.is_empty() {
        return Err(EngineError::Param("a saved command needs at least one step".into()));
    }
    let m = Macro { name: name_of(&path), steps };
    write_macro(&path, &m)?;
    if let Some(dir) = path.parent() {
        order_add(dir, &m.name)?;
    }
    Ok(json!({"name": m.name, "path": path.display().to_string(), "steps": m.steps.len(), "rewritten": rewritten}))
}

fn import(_: &mut Session, p: &Value) -> Result<Value> {
    let to = import_target(p)?;
    let from = PathBuf::from(str_req(p, "path")?);
    let m = Macro { name: name_of(&to), ..load(&from)? };
    write_macro(&to, &m)?;
    if let Some(dir) = to.parent() {
        order_add(dir, &m.name)?;
    }
    Ok(json!({"name": m.name, "path": to.display().to_string(), "steps": m.steps.len()}))
}

fn existing(p: &Value) -> Result<PathBuf> {
    let path = target(p)?;
    if !vfs::is_file(&path) {
        return Err(EngineError::State(format!("there is no saved command “{}”", name_of(&path))));
    }
    Ok(path)
}

fn rename(_: &mut Session, p: &Value) -> Result<Value> {
    let from = existing(p)?;
    let dir = folder(p)?;
    let to = command_path(&dir, str_req(p, "to")?)?;
    let (old, new) = (name_of(&from), name_of(&to));
    if old == new {
        return Ok(json!({"name": new, "from": old}));
    }
    // A change of case only is the same file on Windows and macOS.
    if old.to_lowercase() != new.to_lowercase() && vfs::exists(&to) {
        return Err(EngineError::State(format!("there is already a saved command “{new}”")));
    }
    vfs::rename(&from, &to)?;
    let mut order = read_order(&dir);
    for o in &mut order {
        if o.to_lowercase() == old.to_lowercase() {
            o.clone_from(&new);
        }
    }
    write_order(&dir, &order)?;
    Ok(json!({"name": new, "from": old, "path": to.display().to_string()}))
}

fn delete(_: &mut Session, p: &Value) -> Result<Value> {
    let path = existing(p)?;
    let dir = folder(p)?;
    let name = name_of(&path);
    vfs::remove_file(&path)?;
    let order: Vec<String> = read_order(&dir).into_iter().filter(|o| o.to_lowercase() != name.to_lowercase()).collect();
    write_order(&dir, &order)?;
    Ok(json!({"deleted": name}))
}

fn move_to(_: &mut Session, p: &Value) -> Result<Value> {
    let path = existing(p)?;
    let dir = folder(p)?;
    let name = name_of(&path);
    let index = opt_u64(p, "index")?.ok_or_else(|| EngineError::Param("give the new place: index (0-based)".into()))?;
    let (found, _) = saved_commands(&dir);
    let mut names: Vec<String> = found.into_iter().map(|c| c.name).filter(|n| n.to_lowercase() != name.to_lowercase()).collect();
    let at = usize::try_from(index).unwrap_or(usize::MAX).min(names.len());
    names.insert(at, name.clone());
    write_order(&dir, &names)?;
    Ok(json!({"name": name, "index": at}))
}
