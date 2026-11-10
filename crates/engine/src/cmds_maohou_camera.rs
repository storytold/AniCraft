//! Moho scene-camera commands (ADR 0102): where a scene's imported camera sits, how it zooms and how it rolls,
//! each on its own property channel. The camera is a scene property, not an object: it moves
//! the whole view, and layers marked `camera_immune` ignore it. Zoom uses Moho's units, 2
//! being the neutral view (100 %), because that is what imported Moho documents carry and
//! what exports compare against; the stage's own zoom stays view state and is untouched.

use crate::params::*;
use crate::{CommandSpec, Result, Session};
use anicraft_doc::{Scene, SceneCamera};
use anicraft_geom::Vec2;
use serde_json::{json, Value};
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c("camera.set_track", "Camera Position", "", None, "{x, y, at_frame? (stage pixels)}", true, set_track),
        c("camera.set_zoom", "Camera Zoom", "", None, "{zoom, at_frame? (2 = neutral, larger magnifies)}", true, set_zoom),
        c("camera.set_roll", "Camera Roll", "", None, "{degrees, at_frame? (clockwise)}", true, set_roll),
        c("camera.clear", "Reset Camera", "Modify/Scene", None, "{}", true, clear),
    ]
}

/// The camera of the scene being edited (commands take no scene index: one scene is edited at
/// a time, like every other scene-scoped edit).
fn camera_mut(s: &mut Session) -> &mut SceneCamera {
    &mut Arc::make_mut(&mut s.doc.scenes[s.scene]).camera
}

fn at_frame(s: &Session, p: &Value) -> Result<u32> {
    Ok(opt_u32(p, "at_frame")?.unwrap_or(s.frame))
}

fn set_track(s: &mut Session, p: &Value) -> Result<Value> {
    let x = f32_req(p, "x")?;
    let y = f32_req(p, "y")?;
    let frame = at_frame(s, p)?;
    // Somewhere on or near the stage: the visible view is what the artist aims at.
    let (x, y) = (x.clamp(-1e5, 1e5), y.clamp(-1e5, 1e5));
    camera_mut(s).track.set_key(frame, Vec2::new(x, y));
    Ok(json!({"x": x, "y": y, "frame": frame}))
}

fn set_zoom(s: &mut Session, p: &Value) -> Result<Value> {
    let zoom = f32_req(p, "zoom")?;
    let frame = at_frame(s, p)?;
    let zoom = zoom.clamp(0.01, 100.0);
    camera_mut(s).zoom.set_key(frame, zoom);
    Ok(json!({"zoom": zoom, "frame": frame}))
}

fn set_roll(s: &mut Session, p: &Value) -> Result<Value> {
    let degrees = f32_req(p, "degrees")?;
    let frame = at_frame(s, p)?;
    let roll = degrees.to_radians();
    camera_mut(s).roll.set_key(frame, roll);
    Ok(json!({"degrees": roll.to_degrees(), "frame": frame}))
}

fn clear(s: &mut Session, _p: &Value) -> Result<Value> {
    Arc::make_mut(&mut s.doc.scenes[s.scene]).camera = SceneCamera::default();
    Ok(json!({"cleared": true}))
}

/// The camera's current state, for scene info in command results.
pub(crate) fn camera_json(scene: &Scene) -> Value {
    if scene.camera.is_neutral() {
        return json!(null);
    }
    let f = 0.0f32;
    let track = scene.camera.track.sample_or(f, Vec2::new(0.0, 0.0));
    json!({
        "track": [track.x, track.y],
        "zoom": scene.camera.zoom.sample_or(f, 2.0),
        "roll_degrees": scene.camera.roll.sample_or(f, 0.0).to_degrees(),
        "keys": scene.camera.track.keys.len() + scene.camera.zoom.keys.len() + scene.camera.roll.keys.len(),
    })
}
