//! Rulers, grid and guides (ADR 0076): View › Rulers, View › Grid, View › Guides and the pixel
//! snapping in View › Snapping. Whether each shows and snaps is the session's (`ViewAids`, no
//! undo step); the guides, the grid's spacing and colour and the guides' colour are the
//! document's, so changing them is an undoable edit saved with the file.

use crate::params::*;
use crate::snap::{Accuracy, ViewAids};
use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::aids::{GRID_MAX, GRID_MIN};
use anicraft_doc::{Guide, GuideAxis, MAX_GUIDES};
use serde_json::{Value, json};
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c("view.rulers", "Rulers", "View", Some("Ctrl+Alt+Shift+R"), "{on?: bool} (default: toggle) rulers along the stage's top and left edges, in stage pixels from its top-left corner; drag from one to make a guide", false, (|s, p| toggle(s, p, |a| &mut a.rulers)) as crate::RunFn),
        c("view.grid_show", "Show Grid", "View/Grid", Some("Ctrl+'"), "{on?: bool} (default: toggle) the grid on the stage (never in pictures or exports)", false, |s, p| toggle(s, p, |a| &mut a.show_grid)),
        c("view.grid_snap", "Snap to Grid", "View/Grid", Some("Ctrl+Shift+'"), "{on?: bool} (default: toggle) points being drawn, dragged or moved snap to the grid's lines (shown or not)", false, |s, p| toggle(s, p, |a| &mut a.snap_grid)),
        c(
            "view.grid_set",
            "Edit Grid…",
            "View/Grid",
            Some("Ctrl+Alt+G"),
            "{spacing?, spacing_x?, spacing_y? (stage pixels, 1..=4096), color?: \"#rrggbb[aa]\", over_objects?: bool, show?: bool, snap?: bool, accuracy?: \"close\"|\"normal\"|\"distant\"|\"always\"} sets the grid; spacing and colour are saved with the document (one undo step), the rest is the editor's; returns everything as view.aids does",
            true,
            grid_set,
        ),
        c("view.guides_show", "Show Guides", "View/Guides", Some("Ctrl+;"), "{on?: bool} (default: toggle) this scene's guides on the stage (never in pictures or exports)", false, |s, p| toggle(s, p, |a| &mut a.show_guides)),
        c("view.guides_lock", "Lock Guides", "View/Guides", Some("Ctrl+Alt+;"), "{on?: bool} (default: toggle) locked guides can't be dragged, moved or deleted one by one (new ones can still be added, and Clear Guides clears them)", false, |s, p| toggle(s, p, |a| &mut a.lock_guides)),
        c("view.guides_snap", "Snap to Guides", "View/Guides", Some("Ctrl+Shift+;"), "{on?: bool} (default: toggle) points being drawn, dragged or moved snap to the guides", false, |s, p| toggle(s, p, |a| &mut a.snap_guides)),
        c(
            "view.guides_set",
            "Edit Guides…",
            "View/Guides",
            Some("Ctrl+Alt+Shift+G"),
            "{color?: \"#rrggbb[aa]\", show?: bool, lock?: bool, snap?: bool, accuracy?: \"close\"|\"normal\"|\"distant\"} sets the guides; the colour is saved with the document (one undo step), the rest is the editor's; returns everything as view.aids does",
            true,
            guides_set,
        ),
        c("view.guide_add", "Add Guide…", "View/Guides", None, "{axis: \"horizontal\"|\"vertical\", at} adds a guide to this scene at `at` stage pixels (y for a horizontal guide, x for a vertical one); returns {index, guides}", true, guide_add),
        c("view.guide_move", "Move Guide", "", None, "{index, at} moves this scene's guide `index` (as view.aids lists them) to `at`; refused while guides are locked", true, guide_move),
        c("view.guide_delete", "Delete Guide", "", None, "{index} removes this scene's guide `index`; refused while guides are locked", true, guide_delete),
        c("view.guides_clear", "Clear Guides", "View/Guides", None, "{} removes every guide from this scene (undoable)", true, guides_clear),
        c("view.snap_pixels", "Snap to Pixels", "View/Snapping", None, "{on?: bool} (default: toggle) points being drawn, dragged or moved go to whole stage pixels where nothing else snaps them", false, |s, p| toggle(s, p, |a| &mut a.snap_pixels)),
        c("view.aids", "Rulers, Grid and Guides", "", None, "{} whether the rulers, grid and guides show and snap, the grid's and guides' settings, and this scene's guides [{axis, at}, ..]", false, |s, _| Ok(aids_json(s))),
    ]
}

fn toggle(s: &mut Session, p: &Value, field: fn(&mut ViewAids) -> &mut bool) -> Result<Value> {
    let f = field(&mut s.aids);
    *f = opt_bool(p, "on")?.unwrap_or(!*f);
    Ok(aids_json(s))
}

/// Everything about the rulers, grid and guides, as `view.aids` returns it.
pub(crate) fn aids_json(s: &Session) -> Value {
    let a = s.aids;
    let g = s.doc.aids.grid;
    json!({
        "rulers": a.rulers,
        "grid": {"show": a.show_grid, "snap": a.snap_grid, "accuracy": a.grid_accuracy.key(), "spacing_x": g.spacing_x, "spacing_y": g.spacing_y, "color": color_hex(g.color), "over_objects": g.over_objects},
        "guides": {"show": a.show_guides, "lock": a.lock_guides, "snap": a.snap_guides, "accuracy": a.guide_accuracy.key(), "color": color_hex(s.doc.aids.guide_color), "list": guides_json(s)},
        "snap_pixels": a.snap_pixels,
        "snap_objects": s.style.snap_objects,
    })
}

fn color_hex(c: anicraft_doc::Rgba) -> String {
    let c = c.sanitized();
    if c.a >= 1.0 { c.to_hex() } else { format!("{}{:02x}", c.to_hex(), (c.a * 255.0).round() as u8) }
}

fn guides_json(s: &Session) -> Value {
    json!(s.guides().iter().map(|g| json!({"axis": g.axis.key(), "at": g.at})).collect::<Vec<_>>())
}

fn accuracy(p: &Value, grid: bool) -> Result<Option<Accuracy>> {
    let Some(k) = opt_str(p, "accuracy")? else { return Ok(None) };
    match Accuracy::parse(k) {
        Some(Accuracy::Always) if !grid => Err(EngineError::Param("guides snap 'close', 'normal' or 'distant' (only the grid can 'always' snap)".into())),
        Some(a) => Ok(Some(a)),
        None => Err(EngineError::Param(format!("unknown accuracy '{k}' (close, normal, distant{})", if grid { " or always" } else { "" }))),
    }
}

fn spacing(p: &Value, key: &str) -> Result<Option<f32>> {
    match opt_f32(p, key)? {
        Some(v) if !(GRID_MIN..=GRID_MAX).contains(&v) => Err(EngineError::Param(format!("'{key}' must be between {GRID_MIN} and {GRID_MAX} pixels"))),
        v => Ok(v),
    }
}

fn grid_set(s: &mut Session, p: &Value) -> Result<Value> {
    // Everything is read (and checked) before anything changes.
    let both = spacing(p, "spacing")?;
    let (sx, sy) = (spacing(p, "spacing_x")?.or(both), spacing(p, "spacing_y")?.or(both));
    let color = opt_color(p, "color")?;
    let over = opt_bool(p, "over_objects")?;
    let (show, snap, acc) = (opt_bool(p, "show")?, opt_bool(p, "snap")?, accuracy(p, true)?);
    let mut g = s.doc.aids.grid;
    g.spacing_x = sx.unwrap_or(g.spacing_x);
    g.spacing_y = sy.unwrap_or(g.spacing_y);
    g.color = color.map_or(g.color, anicraft_doc::Rgba::sanitized);
    g.over_objects = over.unwrap_or(g.over_objects);
    if g != s.doc.aids.grid {
        Arc::make_mut(&mut s.doc.aids).grid = g;
    }
    let a = &mut s.aids;
    a.show_grid = show.unwrap_or(a.show_grid);
    a.snap_grid = snap.unwrap_or(a.snap_grid);
    a.grid_accuracy = acc.unwrap_or(a.grid_accuracy);
    Ok(aids_json(s))
}

fn guides_set(s: &mut Session, p: &Value) -> Result<Value> {
    let color = opt_color(p, "color")?;
    let (show, lock, snap, acc) = (opt_bool(p, "show")?, opt_bool(p, "lock")?, opt_bool(p, "snap")?, accuracy(p, false)?);
    if let Some(c) = color.map(anicraft_doc::Rgba::sanitized)
        && c != s.doc.aids.guide_color
    {
        Arc::make_mut(&mut s.doc.aids).guide_color = c;
    }
    let a = &mut s.aids;
    a.show_guides = show.unwrap_or(a.show_guides);
    a.lock_guides = lock.unwrap_or(a.lock_guides);
    a.snap_guides = snap.unwrap_or(a.snap_guides);
    a.guide_accuracy = acc.unwrap_or(a.guide_accuracy);
    Ok(aids_json(s))
}

/// This scene's guides, copy-on-write.
fn guides_mut(s: &mut Session) -> Result<&mut Vec<Guide>> {
    let i = s.scene;
    let scene = s.doc.scenes.get_mut(i).ok_or_else(|| EngineError::State("no scene is open".into()))?;
    Ok(&mut Arc::make_mut(scene).guides)
}

fn position(p: &Value) -> Result<f32> {
    let at = f32_req(p, "at")?;
    if at.abs() > anicraft_doc::GUIDE_REACH {
        return Err(EngineError::Param(format!("'at' must be within {} pixels of the stage's corner", anicraft_doc::GUIDE_REACH)));
    }
    Ok(at)
}

fn index(s: &Session, p: &Value) -> Result<usize> {
    let n = s.guides().len();
    let i = opt_u64(p, "index")?.ok_or_else(|| EngineError::Param("missing param 'index'".into()))?;
    usize::try_from(i).ok().filter(|i| *i < n).ok_or_else(|| EngineError::Param(if n == 0 { "this scene has no guides".into() } else { format!("no guide {i} (this scene has {n}: 0 to {})", n - 1) }))
}

fn unlocked(s: &Session) -> Result<()> {
    if s.aids.lock_guides {
        return Err(EngineError::State("guides are locked: turn off View › Guides › Lock Guides to move or delete them".into()));
    }
    Ok(())
}

fn guide_add(s: &mut Session, p: &Value) -> Result<Value> {
    let axis = str_req(p, "axis")?;
    let axis = GuideAxis::parse(axis).ok_or_else(|| EngineError::Param(format!("unknown axis '{axis}' (horizontal or vertical)")))?;
    let at = position(p)?;
    let guide = Guide::new(axis, at).ok_or_else(|| EngineError::Param("'at' must be a number".into()))?;
    if s.guides().len() >= MAX_GUIDES {
        return Err(EngineError::State(format!("a scene has at most {MAX_GUIDES} guides")));
    }
    let guides = guides_mut(s)?;
    guides.push(guide);
    let index = guides.len() - 1;
    Ok(json!({"index": index, "guides": guides_json(s)}))
}

fn guide_move(s: &mut Session, p: &Value) -> Result<Value> {
    unlocked(s)?;
    let i = index(s, p)?;
    let at = position(p)?;
    if let Some(g) = guides_mut(s)?.get_mut(i)
        && g.at != at
    {
        g.at = at;
    }
    Ok(json!({"index": i, "guides": guides_json(s)}))
}

fn guide_delete(s: &mut Session, p: &Value) -> Result<Value> {
    unlocked(s)?;
    let i = index(s, p)?;
    let guides = guides_mut(s)?;
    if i < guides.len() {
        guides.remove(i);
    }
    Ok(json!({"deleted": i, "guides": guides_json(s)}))
}

fn guides_clear(s: &mut Session, _: &Value) -> Result<Value> {
    let n = s.guides().len();
    if n > 0 {
        guides_mut(s)?.clear();
    }
    Ok(json!({"cleared": n}))
}
