//! Kept scripts (ADR 0013): frame scripts written in another language (ActionScript, as
//! Animate documents bring them), symbol linkage classes and the document class, kept verbatim
//! with the document. Scripts in the ActionScript 3.0 subset run in Test Movie and the HTML
//! canvas export (tier 2); the rest are kept and not run, with the problems that say why.
//! Rendering and the other exports never run scripts.
//!
//! `script.set / clear / list` edit and list a keyframe's kept script; `script.check` says
//! whether a text would run; `script.set_class` sets a symbol's linkage class or the document
//! class. A keyframe holds actions, a plug-in
//! frame script *or* a kept script (one `Keyframe.script`), so setting one replaces the other,
//! as one undo step.

use crate::cmds_actions::{Scope, keyframe_in};
use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::actions::as3::Problem;
use anicraft_doc::actions::{self, CompileResult, FrameScript, Language, SourceScript};
use anicraft_doc::player::{TimelineRef, link_problems};
use anicraft_doc::{Document, Linkage, SymbolId, SymbolKind};
use serde_json::{Value, json};
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c("script.set", "Set Frame Script", "", None, "{text?: the script, verbatim (\"\" clears; at most 1 MiB), language?: actionscript3|actionscript1or2|other (default: the script's, else actionscript3), symbol?: id, layer?, frame?} keeps a script on the keyframe at the frame (it replaces actions there); ActionScript 3.0 in the subset runs in Test Movie and the HTML export, the rest is kept and not run (`runs`, `problems` say which)", true, set as crate::RunFn),
        c("script.clear", "Clear Frame Script", "", None, "{symbol?, layer?, frame?} removes the keyframe's kept script (actions are left alone: actions.clear)", true, clear),
        c("script.list", "List Frame Scripts", "", None, "{text?: bool (default true)} every kept script in every scene and symbol, with its text, whether it runs and its problems, plus symbol linkage classes and the document class", false, list),
        c("script.check", "Check Frame Script", "", None, "{text: the script, language?: actionscript3|actionscript1or2|other (default actionscript3)} whether it would run (the ActionScript 3.0 subset, ADR 0013), its problems with line and column, what is ignored, the names it takes from other frames of its timeline (`uses`), and what it compiles to (`steps`, `program`); changes nothing", false, check),
        c("script.set_class", "Set Script Class", "", None, "{symbol?: id (default: the document), class: dotted class name (\"\" clears), base?: the class it extends} sets a symbol's linkage class, or the document class; kept as text, not run", true, set_class),
    ]
}

fn language(p: &Value) -> Result<Option<Language>> {
    match opt_str(p, "language")? {
        None => Ok(None),
        Some(k) => Language::from_key(k).map(Some).ok_or_else(|| EngineError::Param("param 'language' must be actionscript3, actionscript1or2 or other".into())),
    }
}

/// What a keyframe held before a kept script replaced it.
fn what(script: &Option<FrameScript>) -> Value {
    match script {
        Some(FrameScript::Actions(_)) => json!("actions"),
        Some(FrameScript::Plugin(_)) => json!("plugin"),
        Some(FrameScript::Source(_)) => json!("script"),
        None => Value::Null,
    }
}

/// Whether a script runs, its problems and notes (each `{line, column, message}`; line 0 is
/// the whole script). `links`: why it doesn't run on its timeline although it compiles (a
/// name from another frame that isn't there, ADR 0052).
fn status(c: &CompileResult, links: &[Problem]) -> Value {
    match c {
        Ok(c) if links.is_empty() => json!({"runs": true, "problems": [], "notes": c.notes}),
        Ok(c) => json!({"runs": false, "problems": links, "notes": c.notes}),
        Err(p) => json!({"runs": false, "problems": p, "notes": []}),
    }
}

/// The timeline a keyframe's script belongs to, for linking (graphic and button timelines
/// don't run scripts: `None`).
fn timeline_of(doc: &Document, symbol: Option<SymbolId>) -> Option<TimelineRef> {
    match symbol {
        None => Some(TimelineRef::Root),
        Some(id) => doc.symbol(id).filter(|s| s.kind == SymbolKind::MovieClip).map(|_| TimelineRef::Symbol(id)),
    }
}

/// Why a compiled script doesn't link on its timeline.
fn links(doc: &Document, which: Option<TimelineRef>, c: &CompileResult) -> Vec<Problem> {
    match (which, c) {
        (Some(w), Ok(c)) => link_problems(doc, w, c),
        _ => Vec::new(),
    }
}

fn check(_: &mut Session, p: &Value) -> Result<Value> {
    let text = str_req(p, "text")?;
    if text.len() > actions::MAX_SOURCE_BYTES {
        return Err(EngineError::Param(format!("a script is at most {} KiB", actions::MAX_SOURCE_BYTES / 1024)));
    }
    let script = SourceScript::new(language(p)?.unwrap_or_default(), text);
    let c = script.compiled();
    let mut v = status(c, &[]);
    if let Ok(c) = c {
        v["steps"] = json!(c.steps);
        v["uses"] = json!(c.free);
        v["program"] = json!(c);
    }
    Ok(v)
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    let lang = language(p)?;
    let text = match p.get("text") {
        None | Some(Value::Null) => None,
        Some(Value::String(t)) => Some(t.clone()),
        Some(_) => return Err(EngineError::Param("param 'text' must be a string".into())),
    };
    if text.is_none() && lang.is_none() {
        return Err(EngineError::Param("pass 'text' (the script), or 'language' to change a kept script's".into()));
    }
    if let Some(t) = &text
        && t.len() > actions::MAX_SOURCE_BYTES
    {
        return Err(EngineError::Param(format!("a script is at most {} KiB", actions::MAX_SOURCE_BYTES / 1024)));
    }
    let (lid, k) = keyframe_in(s, p, Scope::Keeps)?;
    let before = what(&k.script);
    let old = k.script.as_ref().and_then(FrameScript::source).cloned();
    let text = match text {
        Some(t) => t,
        None => old.as_ref().map(|o| o.text.clone()).ok_or_else(|| EngineError::State(format!("the keyframe at frame {} keeps no script to change the language of", k.start + 1)))?,
    };
    if text.trim().is_empty() {
        // An empty script clears it (not actions that are there).
        let cleared = old.is_some();
        if cleared {
            k.script = None;
        }
        return Ok(json!({"layer": lid.0, "keyframe": k.start, "cleared": u32::from(cleared)}));
    }
    let script = SourceScript::new(lang.or(old.map(|o| o.language)).unwrap_or_default(), text);
    script.check().map_err(EngineError::Param)?;
    let mut out = json!({"layer": lid.0, "keyframe": k.start, "language": script.language.key(), "lines": script.lines(), "bytes": script.text.len(), "replaced": if before == json!("script") { Value::Null } else { before }});
    let script = Arc::new(script);
    k.script = Some(FrameScript::Source(script.clone()));
    let which = timeline_of(&s.doc, opt_u64(p, "symbol")?.map(SymbolId));
    if let (Some(o), Value::Object(st)) = (out.as_object_mut(), status(script.compiled(), &links(&s.doc, which, script.compiled()))) {
        o.extend(st);
    }
    Ok(out)
}

fn clear(s: &mut Session, p: &Value) -> Result<Value> {
    let (lid, k) = keyframe_in(s, p, Scope::Keeps)?;
    let had = matches!(k.script, Some(FrameScript::Source(_)));
    if had {
        k.script = None;
    }
    Ok(json!({"layer": lid.0, "keyframe": k.start, "cleared": u32::from(had)}))
}

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    let with_text = opt_bool(p, "text")?.unwrap_or(true);
    let mut frames = Vec::new();
    let doc = &s.doc;
    let timelines = doc.scenes.iter().enumerate().map(|(i, sc)| (json!({"scene": i}), sc.name.as_str(), &sc.timeline, Some(TimelineRef::Root))).chain(doc.library.iter().map(|sym| (json!({"symbol": sym.id.0}), sym.name.as_str(), &sym.timeline, timeline_of(doc, Some(sym.id)))));
    for (at, name, tl, which) in timelines {
        for l in &tl.layers {
            for k in &l.keyframes {
                let Some(src) = k.script.as_ref().and_then(FrameScript::source) else { continue };
                let mut v = json!({"timeline": name, "layer": l.id.0, "layer_name": l.name, "frame": k.start, "label": k.label, "language": src.language.key(), "lines": src.lines(), "bytes": src.text.len()});
                if let (Some(o), Value::Object(st)) = (v.as_object_mut(), status(src.compiled(), &links(doc, which, src.compiled()))) {
                    o.extend(st);
                }
                if with_text {
                    v["text"] = json!(src.text);
                }
                if let (Some(o), Some(e)) = (v.as_object_mut(), at.as_object()) {
                    o.extend(e.clone());
                }
                frames.push(v);
            }
        }
    }
    let classes: Vec<Value> = s.doc.library.iter().filter_map(|sym| sym.linkage.as_ref().map(|l| json!({"symbol": sym.id.0, "name": sym.name, "class": l.class, "base": l.base}))).collect();
    Ok(json!({"frames": frames, "classes": classes, "document_class": s.doc.document_class}))
}

fn set_class(s: &mut Session, p: &Value) -> Result<Value> {
    let class = str_req(p, "class")?.trim().to_string();
    let base = opt_str(p, "base")?.unwrap_or("").trim().to_string();
    let linkage = (!class.is_empty()).then(|| Linkage { class: class.clone(), base: base.clone() });
    if let Some(l) = &linkage {
        l.check().map_err(EngineError::Param)?;
    } else if !base.is_empty() {
        return Err(EngineError::Param("a base class needs a class: pass 'class' too".into()));
    }
    match opt_u64(p, "symbol")?.map(SymbolId) {
        None => {
            if !base.is_empty() {
                return Err(EngineError::Param("the document class has no base class here: pass 'symbol' for a symbol's linkage".into()));
            }
            s.doc.document_class = class.clone();
            Ok(json!({"document_class": class}))
        }
        Some(id) => {
            let sym = s.doc.library.iter_mut().find(|x| x.id == id).ok_or_else(|| EngineError::State(format!("no symbol with id {}", id.0)))?;
            if sym.linkage != linkage {
                Arc::make_mut(sym).linkage = linkage;
            }
            Ok(json!({"symbol": id.0, "class": class, "base": base}))
        }
    }
}
