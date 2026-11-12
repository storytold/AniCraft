//! Interactivity (ADR 0024): frame actions and frame scripts on keyframes, event handlers on
//! button and movie clip instances, instance names, and Test Movie.
//!
//! `actions.set / clear / list` edit and list what keyframes and instances do; the player
//! (`anicraft_doc::player`) runs it. `instance.set_name` gives instances the names actions
//! address. `control.test_movie` plays the movie headless with the player (the app opens the
//! Test Movie window instead) and reports what it wrote and where it ended.

use crate::params::*;
use crate::player::{Movie, ScriptRunner, parse_events, state_view};
use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::actions::{self, Action, ButtonEvent, EventHandler, FrameScript, PluginScript};
use anicraft_doc::{Element, ElementId, ElementKind, Keyframe, LayerId, LayerKind, SymbolId, SymbolKind, Timeline};
use serde_json::{Value, json};
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c("control.test_movie", "Test Movie", "Control", Some("Ctrl+Enter"), "{frames?: steps to play (default 10 s of frames, at most 10,000), events?: [{tick, type: move|down|up|click|leave, x, y} or {tick, type: key_down|key_up|key, key: code|letter|LEFT…}], seed?: Math.random()'s seed (default 0)} plays the movie with its actions and frame scripts from the first frame; returns the output, the sounds it started ({tick, sound, name, sync, gain}; buttons' as the pointer changes their state) and where it ended (the app opens the Test Movie window)", false, test_movie as crate::RunFn),
        c("actions.set", "Set Actions", "", None, "{actions: [{op: stop|play|goto_and_play|goto_and_stop|next_frame|prev_frame|next_scene|prev_scene|trace|set_property, target?, frame?, scene?, message?, property?, value?}] | plugin: id, entry?, symbol?: id (a movie clip's timeline), layer?, frame?} sets what the keyframe at the frame runs; or {element, event: click|press|release|over|out, actions} sets an instance's event handler. An empty list clears it", true, set),
        c("actions.clear", "Clear Actions", "", None, "{symbol?, layer?, frame?} clears the keyframe's actions; {element, event?} clears an instance's handlers (one event, or all)", true, clear),
        c("actions.list", "List Actions", "", None, "{} every keyframe with actions or a frame script and every instance with event handlers, in every scene and symbol", false, list),
        c("instance.set_name", "Instance Name", "", None, "{element?: id (default: the selected instance), name: letters, digits, _ or $, not starting with a digit; \"\" clears} names a movie clip or button instance, or a dynamic or input text field, for actions and scripts", true, set_name),
    ]
}

/// What a keyframe command may address: frames that run (actions and plug-in frame scripts:
/// the main timeline and movie clips, not guides), or frames that keep a script (any symbol
/// and guide layers too: kept scripts are content, ADR 0013 tier 1).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope {
    Runs,
    Keeps,
}

/// A timeline commands address: a scene's (the one being edited) or a library symbol's.
fn timeline_mut(s: &mut Session, symbol: Option<SymbolId>, scope: Scope) -> Result<&mut Timeline> {
    match symbol {
        None => Ok(crate::editing::edited_mut(&mut s.doc, s.scene, &s.editing)),
        Some(id) => {
            let sym = s.doc.library.iter_mut().find(|x| x.id == id).ok_or_else(|| EngineError::State(format!("no symbol with id {}", id.0)))?;
            if scope == Scope::Runs && sym.kind != SymbolKind::MovieClip {
                return Err(EngineError::State(format!("“{}” is not a movie clip: only the main timeline and movie clips run frame actions", sym.name)));
            }
            Ok(&mut Arc::make_mut(sym).timeline)
        }
    }
}

/// The keyframe covering `frame` on `layer` of the addressed timeline.
fn keyframe_at<'a>(s: &'a mut Session, p: &Value) -> Result<(LayerId, &'a mut Keyframe)> {
    keyframe_in(s, p, Scope::Runs)
}

/// The keyframe covering `{symbol?, layer?, frame?}`: on the active layer at the playhead by
/// default, or a symbol's first layer at its first frame.
pub(crate) fn keyframe_in<'a>(s: &'a mut Session, p: &Value, scope: Scope) -> Result<(LayerId, &'a mut Keyframe)> {
    let symbol = opt_u64(p, "symbol")?.map(SymbolId);
    let frame = opt_u32(p, "frame")?.unwrap_or(if symbol.is_some() { 0 } else { s.frame });
    if frame > crate::MAX_FRAME {
        return Err(EngineError::Param(format!("param 'frame' must be at most {}", crate::MAX_FRAME)));
    }
    let active = s.layer;
    let layer_param = opt_u64(p, "layer")?.map(LayerId);
    let tl = timeline_mut(s, symbol, scope)?;
    let lid = match (layer_param, symbol) {
        (Some(l), _) => l,
        (None, None) => active,
        (None, Some(_)) => tl.layers.iter().find(|l| l.kind != LayerKind::Folder).map(|l| l.id).ok_or_else(|| EngineError::State("the symbol has no layers".into()))?,
    };
    let layer = tl.layer_mut(lid).ok_or_else(|| EngineError::State(format!("no layer with id {}", lid.0)))?;
    if layer.kind == LayerKind::Folder || (scope == Scope::Runs && layer.kind == LayerKind::Guide) {
        return Err(EngineError::State(format!("'{}' is a {} layer: its frames don't run actions", layer.name, if layer.kind == LayerKind::Folder { "folder" } else { "guide" })));
    }
    let i = layer.keyframe_index_at(frame).ok_or_else(|| EngineError::State(format!("frame {} is empty on '{}': add a keyframe first (F6 or F7)", frame + 1, layer.name)))?;
    let k = layer.keyframe_mut(i).ok_or_else(|| EngineError::State("keyframe vanished".into()))?;
    Ok((lid, k))
}

fn parse_actions(p: &Value) -> Result<Vec<Action>> {
    let v = p.get("actions").cloned().unwrap_or(Value::Null);
    let list: Vec<Action> = serde_json::from_value(v).map_err(|e| EngineError::Param(format!("param 'actions' must be a list of actions ({e})")))?;
    actions::check_actions(&list).map_err(EngineError::Param)?;
    Ok(list)
}

/// The symbol instance `element` on the active layer's keyframe at the playhead (or the selected
/// one, on its layer), for editing.
fn instance_mut<'a>(s: &'a mut Session, p: &Value) -> Result<&'a mut Element> {
    let (layer, id) = match opt_u64(p, "element")? {
        // A selected element is looked for on its own layer.
        Some(id) => (s.selection.iter().find(|(_, e)| e.0 == id).map_or(s.layer, |(l, _)| l), ElementId(id)),
        None => match (s.selection.first(), s.selection.len()) {
            (Some(one), 1) => one,
            (None, _) => return Err(EngineError::State("select a movie clip or button instance, or pass 'element'".into())),
            _ => return Err(EngineError::State("select one instance".into())),
        },
    };
    let frame = s.frame;
    let layer = s.layer_mut(Some(layer))?;
    let i = layer.keyframe_index_at(frame).ok_or_else(|| EngineError::State(format!("frame {} is empty on this layer", frame + 1)))?;
    let k = layer.keyframe_mut(i).ok_or_else(|| EngineError::State("keyframe vanished".into()))?;
    let e = k.elements.iter_mut().find(|e| e.id == id).ok_or_else(|| EngineError::State(format!("no element {} on the active layer at frame {}", id.0, frame + 1)))?;
    Ok(Arc::make_mut(e))
}

/// The instance's kind, checked: movie clips and buttons take names and event handlers.
fn interactive_kind(s: &Session, e: &Element) -> Result<SymbolKind> {
    match &e.kind {
        ElementKind::Symbol { symbol, .. } => match s.doc.symbol(*symbol).map(|x| x.kind) {
            Some(k @ (SymbolKind::MovieClip | SymbolKind::Button)) => Ok(k),
            Some(SymbolKind::Graphic) => Err(EngineError::State("a graphic instance keeps step with its timeline and can't be addressed: make the symbol a movie clip or a button (Properties › Type)".into())),
            None => Err(EngineError::State("the instance's symbol is missing".into())),
        },
        _ => Err(EngineError::State("only movie clip and button instances have instance names and events: convert the artwork to a symbol first (F8)".into())),
    }
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    if p.get("element").is_some() || p.get("event").is_some() {
        return set_handler(s, p);
    }
    let script = match opt_str(p, "plugin")? {
        Some(id) => {
            let ps = PluginScript { id: id.to_string(), entry: opt_str(p, "entry")?.unwrap_or("main").to_string() };
            ps.check().map_err(EngineError::Param)?;
            Some(FrameScript::Plugin(ps))
        }
        None => {
            if p.get("actions").is_none() {
                return Err(EngineError::Param("pass 'actions' (a list), or 'plugin' for a frame script".into()));
            }
            let list = parse_actions(p)?;
            (!list.is_empty()).then_some(FrameScript::Actions(list))
        }
    };
    let (lid, k) = keyframe_at(s, p)?;
    let summary = script.as_ref().map_or_else(String::new, FrameScript::describe);
    let n = script.as_ref().map_or(0, |x| x.actions().len());
    k.script = script;
    Ok(json!({"layer": lid.0, "keyframe": k.start, "actions": n, "summary": summary}))
}

fn set_handler(s: &mut Session, p: &Value) -> Result<Value> {
    let event = ButtonEvent::from_key(str_req(p, "event")?).ok_or_else(|| EngineError::Param("param 'event' must be click, press, release, over or out".into()))?;
    let list = parse_actions(p)?;
    let e = instance_mut(s, p)?.clone();
    interactive_kind(s, &e)?;
    let el = instance_mut(s, p)?;
    let id = el.id;
    if let ElementKind::Symbol { events, .. } = &mut el.kind {
        events.retain(|h| h.event != event);
        if !list.is_empty() {
            if events.len() >= actions::MAX_HANDLERS {
                return Err(EngineError::State(format!("an instance has at most {} event handlers", actions::MAX_HANDLERS)));
            }
            // Kept in event order, so files and panels list them the same way.
            let at = events.iter().position(|h| h.event > event).unwrap_or(events.len());
            events.insert(at, EventHandler { event, actions: list.clone() });
        }
    }
    Ok(json!({"element": id.0, "event": event.key(), "actions": list.len()}))
}

fn clear(s: &mut Session, p: &Value) -> Result<Value> {
    if p.get("element").is_some() {
        let event = match opt_str(p, "event")? {
            Some(k) => Some(ButtonEvent::from_key(k).ok_or_else(|| EngineError::Param("param 'event' must be click, press, release, over or out".into()))?),
            None => None,
        };
        let el = instance_mut(s, p)?;
        let id = el.id;
        let mut n = 0;
        if let ElementKind::Symbol { events, .. } = &mut el.kind {
            let before = events.len();
            events.retain(|h| event.is_some_and(|e| e != h.event));
            n = before - events.len();
        }
        return Ok(json!({"element": id.0, "cleared": n}));
    }
    let (lid, k) = keyframe_at(s, p)?;
    let had = k.script.take().is_some();
    Ok(json!({"layer": lid.0, "keyframe": k.start, "cleared": u32::from(had)}))
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let mut frames = Vec::new();
    let mut instances = Vec::new();
    let timelines = s.doc.scenes.iter().enumerate().map(|(i, sc)| (json!({"scene": i}), sc.name.as_str(), &sc.timeline)).chain(s.doc.library.iter().map(|sym| (json!({"symbol": sym.id.0}), sym.name.as_str(), &sym.timeline)));
    for (at, name, tl) in timelines {
        for l in &tl.layers {
            for k in &l.keyframes {
                if let Some(script) = &k.script {
                    let mut v = json!({"timeline": name, "layer": l.id.0, "layer_name": l.name, "frame": k.start, "label": k.label, "summary": script.describe()});
                    merge(&mut v, &at);
                    match script {
                        FrameScript::Actions(a) => v["actions"] = serde_json::to_value(a).unwrap_or(Value::Null),
                        FrameScript::Plugin(ps) => v["plugin"] = json!({"id": ps.id, "entry": ps.entry}),
                        // The text is in `script.list`.
                        FrameScript::Source(src) => v["script"] = json!({"language": src.language.key(), "lines": src.lines()}),
                    }
                    frames.push(v);
                }
                for e in &k.elements {
                    if let ElementKind::Symbol { symbol, events, .. } = &e.kind
                        && !events.is_empty()
                    {
                        let evs: Vec<Value> = events.iter().map(|h| json!({"event": h.event.key(), "actions": serde_json::to_value(&h.actions).unwrap_or(Value::Null), "summary": h.actions.iter().map(Action::describe).collect::<Vec<_>>().join("; ")})).collect();
                        let mut v = json!({"timeline": name, "layer": l.id.0, "frame": k.start, "element": e.id.0, "name": e.name, "symbol": s.doc.symbol(*symbol).map(|x| x.name.clone()), "events": evs});
                        merge(&mut v, &at);
                        instances.push(v);
                    }
                }
            }
        }
    }
    Ok(json!({"frames": frames, "instances": instances}))
}

fn merge(v: &mut Value, extra: &Value) {
    if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
        for (k, x) in e {
            o.insert(k.clone(), x.clone());
        }
    }
}

fn set_name(s: &mut Session, p: &Value) -> Result<Value> {
    let name = str_req(p, "name")?.trim().to_string();
    if !name.is_empty() && !actions::valid_instance_name(&name) {
        return Err(EngineError::Param(format!("“{name}” can't be an instance name: use letters, digits, _ or $ (no spaces), not starting with a digit, at most {} characters", actions::MAX_NAME_CHARS)));
    }
    let e = instance_mut(s, p)?.clone();
    // Dynamic and input text fields take names too (ADR 0066); static text has none.
    match &e.kind {
        ElementKind::Text(t) if t.field.is_some() => {}
        ElementKind::Text(_) => return Err(EngineError::State("static text has no instance name: make it dynamic or input text first (Properties › Text type)".into())),
        _ => {
            interactive_kind(s, &e)?;
        }
    }
    let el = instance_mut(s, p)?;
    el.name = name.clone();
    Ok(json!({"element": el.id.0, "name": name}))
}

/// Most steps and events of a headless test.
const MAX_TEST_STEPS: u32 = 10_000;
const MAX_TEST_EVENTS: usize = 10_000;
/// Most started sounds `control.test_movie` lists (the rest are counted).
const MAX_TEST_SOUNDS: usize = 10_000;

fn test_movie(s: &mut Session, p: &Value) -> Result<Value> {
    let fps = s.doc.stage.fps.max(1.0);
    let frames = opt_u32(p, "frames")?.unwrap_or((fps * 10.0) as u32).min(MAX_TEST_STEPS);
    let mut timed: Vec<(u64, Vec<anicraft_doc::player::Event>)> = Vec::new();
    match p.get("events") {
        None | Some(Value::Null) => {}
        Some(Value::Array(list)) => {
            if list.len() > MAX_TEST_EVENTS {
                return Err(EngineError::Param(format!("at most {MAX_TEST_EVENTS} events")));
            }
            for (i, e) in list.iter().enumerate() {
                let tick = e.get("tick").and_then(Value::as_u64).ok_or_else(|| EngineError::Param(format!("event {}: needs a \"tick\" (the step it happens in, from 1)", i + 1)))?;
                timed.push((tick, parse_events(e).map_err(|m| EngineError::Param(format!("event {}: {m}", i + 1)))?));
            }
        }
        Some(_) => return Err(EngineError::Param("param 'events' must be a list".into())),
    }
    let seed = opt_u64(p, "seed")?.unwrap_or(anicraft_doc::player::DEFAULT_SEED);
    let mut runner = ScriptRunner::new(&s.plugins);
    let mut movie = Movie::start_seeded(&s.doc, seed, &mut runner);
    // The sounds the movie started (buttons' under the pointer too, ADR 0057), in order.
    let mut sounds: Vec<Value> = Vec::new();
    let mut more = 0usize;
    let mut heard = |movie: &mut Movie, doc: &anicraft_doc::Document| {
        for (tick, st) in movie.take_sounds() {
            if sounds.len() >= MAX_TEST_SOUNDS {
                more += 1;
                continue;
            }
            let name = doc.sound(st.sound.sound).map(|x| x.name.clone());
            sounds.push(json!({"tick": tick, "sound": st.sound.sound.0, "name": name, "sync": st.sound.sync.key(), "gain": st.gain}));
        }
    };
    heard(&mut movie, &s.doc);
    for tick in 1..=u64::from(frames) {
        let events: Vec<_> = timed.iter().filter(|(t, _)| *t == tick).flat_map(|(_, e)| e.iter().copied()).collect();
        movie.step(&s.doc, &events, &mut runner);
        heard(&mut movie, &s.doc);
    }
    let nodes = anicraft_doc::player::tree(&s.doc, &movie.state);
    Ok(json!({
        "steps": frames,
        "output": movie.output.iter().map(|(t, l)| json!({"tick": t, "line": l})).collect::<Vec<_>>(),
        "sounds": sounds,
        "more_sounds": more,
        "state": state_view(&s.doc, &movie.state, &nodes, None),
        "script_runs": runner.runs,
        "script_fuel": runner.fuel_used,
    }))
}
