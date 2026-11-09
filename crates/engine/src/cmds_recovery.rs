//! Crash-recovery commands (ADR 0015): list, restore and discard the autosaved copies that a
//! session which didn't close cleanly left in a recovery folder, and autosave on request.
//! The desktop app autosaves in the background with [`crate::recovery::Autosaver`]; these
//! commands are how it (and the CLI, scripts and agents) acts on what was left behind.

use crate::params::*;
use crate::recovery::{self, RecoveryMeta};
use crate::{CommandSpec, EngineError, Result, Session};
use serde_json::{Value, json};
use std::path::PathBuf;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "file.recovery_list",
            label: "Recover Autosaved Documents…",
            menu: "File",
            shortcut: None,
            params: "{dir (the recovery folder)}",
            mutates: false,
            run: list,
        },
        CommandSpec {
            id: "file.recover",
            label: "Restore Autosaved Document",
            menu: "",
            shortcut: None,
            params: "{dir, session (an id from file.recovery_list), confirm? (needed over unsaved changes)}",
            mutates: false,
            run: recover,
        },
        CommandSpec {
            id: "file.discard_recovery",
            label: "Discard Autosaved Document",
            menu: "",
            shortcut: None,
            params: "{dir, session, confirm (always: it cannot be undone)}",
            mutates: false,
            run: discard,
        },
        CommandSpec {
            id: "file.autosave",
            label: "Autosave Now",
            menu: "",
            shortcut: None,
            params: "{dir, session? (default headless-<process id>)}",
            mutates: false,
            run: autosave,
        },
    ]
}

fn dir(p: &Value) -> Result<PathBuf> {
    let d = str_req(p, "dir")?;
    if d.trim().is_empty() {
        return Err(EngineError::Param("param 'dir' must name the recovery folder".into()));
    }
    Ok(PathBuf::from(d))
}

fn list(_: &mut Session, p: &Value) -> Result<Value> {
    let dir = dir(p)?;
    let found = recovery::scan(&dir, None);
    Ok(json!({"dir": dir.display().to_string(), "documents": found.iter().map(recovery::Entry::to_json).collect::<Vec<_>>()}))
}

/// Opens an autosaved copy as the document: unsaved, with its original path remembered so Save
/// goes back there, unless that file was saved after the copy was made (then Save asks for a
/// name, so newer work on disk is never replaced by older recovered work).
fn recover(s: &mut Session, p: &Value) -> Result<Value> {
    let dir = dir(p)?;
    let id = str_req(p, "session")?;
    let entry = recovery::find(&dir, id)?;
    let newer = entry.original_is_newer();
    let (doc, meta) = recovery::load(&dir, id)?;
    let path = if newer { None } else { meta.original.clone() };
    s.replace_document(doc, path);
    s.mark_unsaved();
    let mut info = s.info();
    info["recovered"] = json!(id);
    info["name"] = json!(meta.name);
    info["original"] = json!(meta.original.as_ref().map(|o| o.display().to_string()));
    info["original_newer"] = json!(newer);
    Ok(info)
}

fn discard(_: &mut Session, p: &Value) -> Result<Value> {
    let dir = dir(p)?;
    let id = str_req(p, "session")?;
    let removed = recovery::discard(&dir, id)?;
    Ok(json!({"discarded": id, "files": removed}))
}

fn autosave(s: &mut Session, p: &Value) -> Result<Value> {
    let dir = dir(p)?;
    let id = match opt_str(p, "session")? {
        Some(id) => id.to_string(),
        None => format!("headless-{}", std::process::id()),
    };
    let meta = RecoveryMeta::describe(&id, &s.doc, s.path.as_deref());
    let st = recovery::write(&dir, &id, &s.doc, &meta)?;
    Ok(json!({
        "session": id,
        "written": recovery::document_path(&dir, &id).display().to_string(),
        "bytes": st.bytes,
        "serialize_ms": (st.serialize_ms * 100.0).round() / 100.0,
        "write_ms": (st.write_ms * 100.0).round() / 100.0,
    }))
}
