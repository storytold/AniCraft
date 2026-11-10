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
        c("scene.set_frames", "Playback Frame Range", "", None, "{start, end (0-based, end exclusive)}", true, set_frames),
        c("scene.clear_frames", "Clear Playback Frame Range", "", None, "{}", true, clear_frames),
        c("scene.add_marker", "Add Timeline Marker", "", None, "{frame, label}", true, add_marker),
        c("scene.remove_marker", "Remove Timeline Marker", "", None, "{frame}", true, remove_marker),
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
fn open_scene(s: &mut Session, i: usize) {
    s.scene = i.min(s.doc.scenes.len().saturating_sub(1));
    s.frame = 0;
    s.layer = s.doc.timeline(s.scene).layers.first().map_or(LayerId(0), |l| l.id);
    s.selection.clear();
}

fn scene_json(s: &Session) -> Value {
    let name = s.doc.scenes.get(s.scene).map_or("", |sc| sc.name.as_str());
    let mut v = json!({"scene": s.scene, "name": name, "scenes": s.doc.scenes.len(), "frames": s.frame_count()});
    if let Some(scene) = s.doc.scenes.get(s.scene) {
        if !scene.camera.is_neutral() {
            v["camera"] = crate::cmds_maohou_camera::camera_json(scene);
        }
        if !scene.markers.is_empty() {
            v["markers"] = json!(scene.markers.iter().map(|(f, label)| json!({"frame": f, "label": label})).collect::<Vec<_>>());
        }
        if let Some((start, end)) = scene.frames {
            v["frame_range"] = json!([start, end]);
        }
    }
    v
}

/// The frames the scene plays: a stored playback range (Moho's project range) or `clear`
/// to run to the timeline's content again.
fn set_frames(s: &mut Session, p: &Value) -> Result<Value> {
    let start = opt_u32(p, "start")?.ok_or_else(|| EngineError::Param("param 'start' must be a frame number".into()))?;
    let end = opt_u32(p, "end")?.ok_or_else(|| EngineError::Param("param 'end' must be a frame number".into()))?;
    if start >= end {
        return Err(EngineError::Param("the range's end must be after its start".into()));
    }
    if end > 100_000 {
        return Err(EngineError::Param("the range cannot go past frame 100,000".into()));
    }
    Arc::make_mut(&mut s.doc.scenes[s.scene]).frames = Some((start, end));
    Ok(scene_json(s))
}

fn clear_frames(s: &mut Session, _p: &Value) -> Result<Value> {
    Arc::make_mut(&mut s.doc.scenes[s.scene]).frames = None;
    Ok(scene_json(s))
}

/// A named point on the timeline (Moho's timeline markers): one marker per frame, the label
/// non-empty.
fn add_marker(s: &mut Session, p: &Value) -> Result<Value> {
    let frame = opt_u32(p, "frame")?.ok_or_else(|| EngineError::Param("param 'frame' must be a frame number".into()))?;
    let label = opt_str(p, "label")?.unwrap_or_default().trim().chars().take(256).collect::<String>();
    if label.is_empty() {
        return Err(EngineError::Param("param 'label' must be the marker's name".into()));
    }
    let scene = Arc::make_mut(&mut s.doc.scenes[s.scene]);
    scene.markers.retain(|(f, _)| *f != frame);
    scene.markers.push((frame.min(100_000), label));
    scene.markers.sort_by_key(|(f, _)| *f);
    Ok(scene_json(s))
}

fn remove_marker(s: &mut Session, p: &Value) -> Result<Value> {
    let frame = opt_u32(p, "frame")?.ok_or_else(|| EngineError::Param("param 'frame' must be a frame number".into()))?;
    let scene = Arc::make_mut(&mut s.doc.scenes[s.scene]);
    let before = scene.markers.len();
    scene.markers.retain(|(f, _)| *f != frame);
    if scene.markers.len() == before {
        return Err(EngineError::State(format!("there is no marker at frame {frame}")));
    }
    Ok(scene_json(s))
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
    s.doc.scenes.insert(i + 1, Arc::new(Scene { name, timeline, ..Scene::new("") }));
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
