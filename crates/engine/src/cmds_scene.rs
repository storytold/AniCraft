//! Scene commands: a movie is a list of scenes that play in order, each with its own main
//! timeline. The session edits one scene at a time (`Session::scene`).

use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::{Element, ElementId, Keyframe, Layer, LayerId, MAX_SCENES, Scene, Timeline};
use serde_json::{Value, json};
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c("scene.new", "Scene", "Insert", None, "{name?}", true, new_scene as crate::RunFn),
        c("scene.duplicate", "Duplicate Scene", "Modify/Scene", None, "{scene?}", true, duplicate_scene),
        c("scene.rename", "Rename Scene", "Modify/Scene", None, "{name, scene?}", true, rename_scene),
        c("scene.delete", "Delete Scene", "Modify/Scene", None, "{scene?}", true, delete_scene),
        c("scene.move_up", "Move Scene Earlier", "Modify/Scene", None, "{scene?}", true, |s, p| move_scene(s, p, -1)),
        c("scene.move_down", "Move Scene Later", "Modify/Scene", None, "{scene?}", true, |s, p| move_scene(s, p, 1)),
        c("scene.select", "Go to Scene", "", None, "{scene: index}", false, select_scene),
        c("scene.prev", "Previous Scene", "Control", Some("Shift+PageUp"), "{}", false, |s, _| step_scene(s, -1)),
        c("scene.next", "Next Scene", "Control", Some("Shift+PageDown"), "{}", false, |s, _| step_scene(s, 1)),
    ]
}

/// The scene a command targets: `scene` (an index), else the one being edited.
pub(crate) fn target_scene(s: &Session, p: &Value) -> Result<usize> {
    match opt_u64(p, "scene")? {
        None => Ok(s.scene),
        Some(i) => usize::try_from(i).ok().filter(|i| *i < s.doc.scenes.len()).ok_or_else(|| EngineError::Param(format!("there is no scene {} (the movie has {})", i + 1, s.doc.scenes.len()))),
    }
}

/// Opens scene `i` at its first frame, on its top layer.
pub(crate) fn open_scene(s: &mut Session, i: usize) {
    // A scene is edited from its own timeline: open symbols close first (ADR 0045).
    s.exit_all();
    s.scene = i.min(s.doc.scenes.len().saturating_sub(1));
    s.frame = 0;
    s.layer = s.timeline().layers.first().map_or(LayerId(0), |l| l.id);
    s.selection.clear();
}

fn scene_json(s: &Session) -> Value {
    let name = s.doc.scenes.get(s.scene).map_or("", |sc| sc.name.as_str());
    json!({"scene": s.scene, "name": name, "scenes": s.doc.scenes.len(), "frames": s.frame_count()})
}

fn unused_name(s: &Session, base: &str) -> String {
    let taken = |n: &str| s.doc.scenes.iter().any(|sc| sc.name == n);
    if !taken(base) {
        return base.to_string();
    }
    (2..).map(|k| format!("{base} {k}")).find(|n| !taken(n)).unwrap_or_else(|| base.to_string())
}

fn check_room(s: &Session) -> Result<()> {
    if s.doc.scenes.len() >= MAX_SCENES {
        return Err(EngineError::State(format!("a movie can have at most {MAX_SCENES} scenes")));
    }
    Ok(())
}

/// Insert › Scene: an empty scene after the current one, opened.
fn new_scene(s: &mut Session, p: &Value) -> Result<Value> {
    check_room(s)?;
    let name = match opt_str(p, "name")? {
        Some(n) if !n.trim().is_empty() => n.chars().take(256).collect(),
        _ => (1..).map(|k| format!("Scene {k}")).find(|n| !s.doc.scenes.iter().any(|sc| &sc.name == n)).unwrap_or_else(|| "Scene".into()),
    };
    let mut scene = Scene::new(name);
    let id = LayerId(s.doc.alloc_id());
    scene.timeline.layers.push(Arc::new(Layer::new(id, "Layer 1")));
    let at = (s.scene + 1).min(s.doc.scenes.len());
    s.doc.scenes.insert(at, Arc::new(scene));
    open_scene(s, at);
    Ok(scene_json(s))
}

/// A copy of `tl` with fresh layer and element ids (ids are unique across the whole document),
/// and mask links pointing at the copied masks.
fn reid(s: &mut Session, tl: &Timeline) -> Timeline {
    let map: Vec<(LayerId, LayerId)> = tl.layers.iter().map(|l| (l.id, LayerId(s.doc.alloc_id()))).collect();
    let new_id = |old: LayerId| map.iter().find(|(o, _)| *o == old).map(|(_, n)| *n);
    let mut layers = Vec::with_capacity(tl.layers.len());
    for l in &tl.layers {
        let mut copy: Layer = (**l).clone();
        copy.id = new_id(l.id).unwrap_or(copy.id);
        copy.parent = l.parent.and_then(new_id);
        // Elements keep their id across a layer's keyframes (tweens pair them), so map per layer.
        let mut emap: Vec<(ElementId, ElementId)> = Vec::new();
        copy.keyframes = l
            .keyframes
            .iter()
            .map(|k| {
                let mut nk: Keyframe = (**k).clone();
                nk.elements = k
                    .elements
                    .iter()
                    .map(|e| {
                        let id = match emap.iter().find(|(o, _)| *o == e.id) {
                            Some((_, n)) => *n,
                            None => {
                                let n = ElementId(s.doc.alloc_id());
                                emap.push((e.id, n));
                                n
                            }
                        };
                        Arc::new(Element { id, ..(**e).clone() })
                    })
                    .collect();
                if let Some(m) = nk.motion.as_mut()
                    && let Some((_, n)) = emap.iter().find(|(o, _)| *o == m.target)
                {
                    m.target = *n;
                }
                Arc::new(nk)
            })
            .collect();
        layers.push(Arc::new(copy));
    }
    Timeline { layers }
}

fn duplicate_scene(s: &mut Session, p: &Value) -> Result<Value> {
    check_room(s)?;
    let i = target_scene(s, p)?;
    let Some(src) = s.doc.scenes.get(i).cloned() else { return Err(EngineError::State("no such scene".into())) };
    let timeline = reid(s, &src.timeline);
    let name = unused_name(s, &format!("{} copy", src.name));
    s.doc.scenes.insert(i + 1, Arc::new(Scene { name, timeline, guides: src.guides.clone() }));
    open_scene(s, i + 1);
    Ok(scene_json(s))
}

fn rename_scene(s: &mut Session, p: &Value) -> Result<Value> {
    let name: String = str_req(p, "name")?.chars().take(256).collect();
    if name.trim().is_empty() {
        return Err(EngineError::Param("a scene name must not be empty".into()));
    }
    let i = target_scene(s, p)?;
    if let Some(sc) = s.doc.scenes.get_mut(i) {
        Arc::make_mut(sc).name = name.clone();
    }
    Ok(json!({"scene": i, "name": name}))
}

fn delete_scene(s: &mut Session, p: &Value) -> Result<Value> {
    if s.doc.scenes.len() <= 1 {
        return Err(EngineError::State("a movie needs at least one scene".into()));
    }
    let i = target_scene(s, p)?;
    s.doc.scenes.remove(i);
    if i <= s.scene {
        let keep = if i == s.scene { i.min(s.doc.scenes.len() - 1) } else { s.scene - 1 };
        open_scene(s, keep);
    }
    Ok(scene_json(s))
}

fn move_scene(s: &mut Session, p: &Value, d: i64) -> Result<Value> {
    let i = target_scene(s, p)?;
    let j = i as i64 + d;
    if j < 0 || j as usize >= s.doc.scenes.len() {
        return Ok(json!({"scene": i}));
    }
    let j = j as usize;
    s.doc.scenes.swap(i, j);
    // The scene being edited stays the one being edited.
    if s.scene == i {
        s.scene = j;
    } else if s.scene == j {
        s.scene = i;
    }
    Ok(json!({"scene": j}))
}

fn select_scene(s: &mut Session, p: &Value) -> Result<Value> {
    if p.get("scene").is_none() {
        return Err(EngineError::Param("pass {\"scene\": index}".into()));
    }
    let i = target_scene(s, p)?;
    if i != s.scene {
        open_scene(s, i);
    }
    Ok(scene_json(s))
}

fn step_scene(s: &mut Session, d: i64) -> Result<Value> {
    let n = s.doc.scenes.len() as i64;
    let i = (s.scene as i64 + d).clamp(0, n - 1) as usize;
    if i != s.scene {
        open_scene(s, i);
    }
    Ok(scene_json(s))
}
