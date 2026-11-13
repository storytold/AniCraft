//! Eases (ADR 0081): the preset library, custom eases edited point by point, eases saved in
//! the document, and copy and paste of an ease between tweens.
//!
//! Every command addresses one ease, the *target*:
//!
//! * a classic or shape tween's (the span at `layer?`/`frame?`), or with `group` one property
//!   group's (`position`, `rotation`, `scale`, `color`, `filters`; ADR 0081 §3);
//! * a motion tween's property segment (the span at `layer?`/`frame?`, the segment the frame is
//!   in, of `property`, or every property when it is left out);
//! * a saved ease (`saved: name`).
//!
//! Point edits turn a preset into the custom curve that draws it ([`Easing::to_custom`]) and
//! edit that. The curve stays valid whatever is asked (time never runs backwards; values stay
//! within bounds; see [`CustomEase`]); an edit that can't apply (moving an end, deleting the
//! last point between them) is an error that changes nothing.

use crate::cmds_timeline::target;
use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::{CustomEase, EaseGroup, Easing, Handle, LayerId, MAX_SAVED_EASES, Property, SavedEase, Tween, TweenKind, ease_name};
use anicraft_geom::Vec2;
use serde_json::{Value, json};
use std::sync::Arc;


pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c("ease.info", "Ease", "", None, "{target: layer?, frame?, group? | property? | saved?} → the ease (as easing params take it), its label, and its curve as points (exact unless the preset is fitted)", false, info as crate::RunFn),
        c("ease.set", "Set Ease", "", None, "{easing: a preset name | {classic: n} | {bezier: [..]} | {custom: ..} | {saved: name} | {plugin: id, ease: id} (the tween keeps the sampled curve) | {sampled: ..}, layer?, frame?, group? | property? | saved?}", true, set),
        c("ease.add_point", "Add Ease Point", "", None, "{t: time 0..1 on the curve (its shape stays), layer?, frame?, group? | property? | saved?} → {index}", true, add_point),
        c("ease.move_point", "Move Ease Point", "", None, "{index, x (time, kept between its neighbours), y (value, may overshoot), layer?, frame?, group? | property? | saved?} (the ends stay at (0, 0) and (1, 1))", true, move_point),
        c("ease.move_handle", "Move Ease Handle", "", None, "{index, handle: in|out, x, y (absolute; x stays between the point and its neighbour), layer?, frame?, group? | property? | saved?} (a smooth point's other handle turns with it)", true, move_handle),
        c("ease.delete_point", "Delete Ease Point", "", None, "{index (not an end), layer?, frame?, group? | property? | saved?}", true, delete_point),
        c("ease.convert_point", "Convert Ease Point", "", None, "{index, smooth: bool (handles in line) or corner (handles on the point), layer?, frame?, group? | property? | saved?}", true, convert_point),
        c("ease.separate", "Ease Properties Separately", "", None, "{separate: bool, layer?, frame?}: a classic tween with one ease per property group (each starting from the tween's), or one ease for all (the groups' own go)", true, separate),
        c("ease.save", "Save Ease", "", None, "{name, easing? (default: the target's ease), replace?: bool, layer?, frame?, group? | property?} → saved in the document, offered with the presets", true, save),
        c("ease.delete", "Delete Saved Ease", "", None, "{name} (tweens using it keep their curves)", true, delete),
        c("ease.rename", "Rename Saved Ease", "", None, "{name, to}", true, rename),
        c("ease.list", "Eases", "", None, "{} → the presets (name, label, family, direction), the eases saved in the document, and the installed ease plug-ins' eases ({plugin, ease} as easing params take them)", false, list),
        c("ease.copy", "Copy Ease", "", None, "{layer?, frame?, group? | property? | saved?}: the ease, for ease.paste", false, copy),
        c("ease.paste", "Paste Ease", "", None, "{layer?, frame?, group? | property? | saved?}: the copied ease onto this target", true, paste),
    ]
}

/// The one ease a command addresses (see the module docs).
#[derive(Clone, Debug, PartialEq)]
pub enum EaseAt {
    Tween { layer: LayerId, frame: u32, group: Option<EaseGroup> },
    Motion { layer: LayerId, frame: u32, property: Option<Property> },
    Saved(String),
}

pub(crate) fn group(p: &Value) -> Result<Option<EaseGroup>> {
    match opt_str(p, "group")? {
        None => Ok(None),
        Some(g) if g.trim().eq_ignore_ascii_case("all") => Ok(None),
        Some(g) => EaseGroup::from_key(g).map(Some).ok_or_else(|| EngineError::Param(format!("unknown group '{g}' (all, position, rotation, scale, color, filters)"))),
    }
}

fn property(p: &Value) -> Result<Option<Property>> {
    match opt_str(p, "property")? {
        None => Ok(None),
        Some(k) => Property::from_key(k).map(Some).ok_or_else(|| EngineError::Param(format!("unknown property '{k}'"))),
    }
}

pub fn ease_target(s: &Session, p: &Value) -> Result<EaseAt> {
    if let Some(n) = opt_str(p, "saved")? {
        saved_ease(&s.doc, n)?;
        return Ok(EaseAt::Saved(n.to_string()));
    }
    let (lid, frame) = target(s, p)?;
    let layer = lid.unwrap_or(s.layer);
    let l = s.timeline().layer(layer).ok_or_else(|| EngineError::State(format!("no layer with id {}", layer.0)))?;
    let kf = l.keyframe_at(frame).ok_or_else(|| EngineError::State(format!("frame {} is empty on this layer", frame + 1)))?;
    if kf.motion.is_some() {
        if p.get("group").is_some_and(|g| !g.is_null()) {
            return Err(EngineError::Param("a motion tween eases each property's segments: give 'property', not 'group'".into()));
        }
        return Ok(EaseAt::Motion { layer, frame, property: property(p)? });
    }
    let Some(t) = &kf.tween else { return Err(EngineError::State(format!("frame {} has no tween; create one first (Insert › Create Classic Tween)", frame + 1))) };
    let group = group(p)?;
    if group.is_some() && t.kind == TweenKind::Shape {
        return Err(EngineError::State("a shape tween has one ease for everything; property groups ease separately in classic tweens".into()));
    }
    Ok(EaseAt::Tween { layer, frame, group })
}

pub fn read(s: &Session, at: &EaseAt) -> Result<Easing> {
    match at {
        EaseAt::Saved(n) => saved_ease(&s.doc, n),
        EaseAt::Tween { layer, frame, group } => {
            let t = s.timeline().layer(*layer).and_then(|l| l.keyframe_at(*frame)).and_then(|k| k.tween.as_ref()).ok_or_else(|| EngineError::State("the tween is gone".into()))?;
            Ok(group.map_or(&t.easing, |g| t.ease(g)).clone())
        }
        EaseAt::Motion { layer, frame, property } => {
            let kf = s.timeline().layer(*layer).and_then(|l| l.keyframe_at(*frame)).ok_or_else(|| EngineError::State("the tween is gone".into()))?;
            let m = kf.motion.as_ref().ok_or_else(|| EngineError::State("the motion tween is gone".into()))?;
            let off = frame.saturating_sub(kf.start);
            m.tracks.iter().filter(|t| property.is_none_or(|p| p == t.property)).find_map(|t| t.segment_at(off).and_then(|i| t.keys.get(i)).map(|k| k.ease.clone())).ok_or_else(|| not_between(*frame, *property))
        }
    }
}

fn not_between(frame: u32, p: Option<Property>) -> EngineError {
    EngineError::State(format!("frame {} is not between two property keyframes{}", frame + 1, p.map_or_else(String::new, |o| format!(" of {}", o.label()))))
}

/// Puts `e` on the target; what it set, for the reply.
pub(crate) fn write(s: &mut Session, at: &EaseAt, e: Easing) -> Result<Value> {
    let label = e.label();
    match at {
        EaseAt::Saved(n) => {
            let want = n.to_lowercase();
            let mut list = (*s.doc.eases).clone();
            let slot = list.iter_mut().find(|x| x.name.to_lowercase() == want).ok_or_else(|| EngineError::State(format!("no ease is saved as '{n}'")))?;
            slot.ease = e;
            let name = slot.name.clone();
            s.doc.eases = Arc::new(list);
            Ok(json!({"saved": name, "easing": label}))
        }
        EaseAt::Tween { layer, frame, group } => {
            let l = s.layer_mut(Some(*layer))?;
            let i = l.keyframe_index_at(*frame).ok_or_else(|| EngineError::State("the tween is gone".into()))?;
            let t = l.keyframe_mut(i).and_then(|k| k.tween.as_mut()).ok_or_else(|| EngineError::State("the tween is gone".into()))?;
            match group {
                Some(g) => t.set_ease(*g, Some(e)),
                None => t.easing = e,
            }
            Ok(json!({"easing": label, "group": group.map_or("all", |g| g.key())}))
        }
        EaseAt::Motion { layer, frame, property } => {
            let l = s.layer_mut(Some(*layer))?;
            let i = l.keyframe_index_at(*frame).ok_or_else(|| EngineError::State("the tween is gone".into()))?;
            let kf = l.keyframe_mut(i).ok_or_else(|| EngineError::State("the tween is gone".into()))?;
            let off = frame.saturating_sub(kf.start);
            let m = kf.motion.as_mut().ok_or_else(|| EngineError::State("the motion tween is gone".into()))?;
            let mut set = Vec::new();
            for t in m.tracks.iter_mut().filter(|t| property.is_none_or(|p| p == t.property)) {
                if let Some(k) = t.segment_at(off).and_then(|i| t.keys.get_mut(i)) {
                    k.ease = e.clone();
                    set.push(t.property.key());
                }
            }
            if set.is_empty() {
                return Err(not_between(*frame, *property));
            }
            Ok(json!({"easing": label, "properties": set}))
        }
    }
}

/// An ease's curve for replies: each point and its handles (absolute), and smooth.
pub fn points_json(c: &CustomEase) -> Value {
    Value::Array(c.points().iter().map(|p| json!({"x": p.at.x, "y": p.at.y, "in": [p.at.x + p.h_in.x, p.at.y + p.h_in.y], "out": [p.at.x + p.out.x, p.at.y + p.out.y], "smooth": p.smooth})).collect())
}

fn kind(e: &Easing) -> &'static str {
    match e {
        Easing::Classic(_) => "classic",
        Easing::CubicBezier(..) => "bezier",
        Easing::Custom(_) => "custom",
        Easing::Sampled(_) => "plugin",
        _ => "preset",
    }
}

/// The name a saved ease with exactly this curve goes by, if any.
pub fn saved_name<'a>(doc: &'a anicraft_doc::Document, e: &Easing) -> Option<&'a str> {
    doc.eases.iter().find(|x| x.ease == *e).map(|x| x.name.as_str())
}

fn info(s: &mut Session, p: &Value) -> Result<Value> {
    let at = ease_target(s, p)?;
    let e = read(s, &at)?;
    let curve = e.to_custom();
    let exact = !matches!(e.family(), Some((f, _)) if !matches!(f, anicraft_doc::EaseFamily::Quad | anicraft_doc::EaseFamily::Cubic | anicraft_doc::EaseFamily::Back | anicraft_doc::EaseFamily::Bounce));
    let mut out = json!({"easing": easing_json(&e), "label": e.label(), "kind": kind(&e), "saved_as": saved_name(&s.doc, &e), "points": points_json(&curve), "exact": exact});
    if let EaseAt::Tween { layer, frame, .. } = at
        && let Some(t) = s.timeline().layer(layer).and_then(|l| l.keyframe_at(frame)).and_then(|k| k.tween.as_ref())
    {
        out["groups"] = Value::Array(t.groups.iter().map(|(g, e)| json!({"group": g.key(), "easing": easing_json(e), "label": e.label()})).collect());
    }
    Ok(out)
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    let e = opt_easing_in(s, p, "easing")?.ok_or_else(|| EngineError::Param("param 'easing' is required".into()))?;
    let at = ease_target(s, p)?;
    write(s, &at, e)
}

/// The target's curve, edited by `f`; `f` saying `None` is the error `why`.
fn edit(s: &mut Session, p: &Value, why: &str, f: impl FnOnce(&CustomEase) -> Option<(CustomEase, Value)>) -> Result<Value> {
    let at = ease_target(s, p)?;
    let curve = read(s, &at)?.to_custom();
    let (next, mut reply) = f(&curve).ok_or_else(|| EngineError::State(why.to_string()))?;
    let n = next.points().len();
    write(s, &at, Easing::Custom(Arc::new(next)))?;
    reply["points"] = json!(n);
    Ok(reply)
}

fn index(p: &Value) -> Result<usize> {
    opt_u32(p, "index")?.map(|i| i as usize).ok_or_else(|| EngineError::Param("param 'index' is required (0 is the start, as ease.info lists the points)".into()))
}

fn add_point(s: &mut Session, p: &Value) -> Result<Value> {
    let t = f32_req(p, "t")?;
    edit(s, p, "a point can't go there: inside the curve (0 < t < 1), not on a point, and at most 64 points", |c| c.insert_at(t).map(|(c, i)| (c, json!({"index": i}))))
}

fn move_point(s: &mut Session, p: &Value) -> Result<Value> {
    let (i, x, y) = (index(p)?, f32_req(p, "x")?, f32_req(p, "y")?);
    edit(s, p, "that point can't move: the ends stay at (0, 0) and (1, 1)", |c| c.move_point(i, Vec2::new(x, y)).map(|d| {
        let at = d.points().get(i).map_or(Vec2::ZERO, |q| q.at);
        (d, json!({"index": i, "x": at.x, "y": at.y}))
    }))
}

fn move_handle(s: &mut Session, p: &Value) -> Result<Value> {
    let (i, x, y) = (index(p)?, f32_req(p, "x")?, f32_req(p, "y")?);
    let h = match str_req(p, "handle")?.trim() {
        "in" => Handle::In,
        "out" => Handle::Out,
        other => return Err(EngineError::Param(format!("param 'handle' must be in or out, not '{other}'"))),
    };
    edit(s, p, "that point has no such handle (the start has no in handle, the end no out handle)", |c| c.move_handle(i, h, Vec2::new(x, y)).map(|d| (d, json!({"index": i}))))
}

fn delete_point(s: &mut Session, p: &Value) -> Result<Value> {
    let i = index(p)?;
    edit(s, p, "that point can't be deleted: the ends stay", |c| c.remove(i).map(|d| (d, json!({"deleted": i}))))
}

fn convert_point(s: &mut Session, p: &Value) -> Result<Value> {
    let i = index(p)?;
    let smooth = opt_bool(p, "smooth")?.ok_or_else(|| EngineError::Param("param 'smooth' is required (true: smooth, false: corner)".into()))?;
    edit(s, p, "there is no such point", |c| c.set_smooth(i, smooth).map(|d| (d, json!({"index": i, "smooth": smooth}))))
}

fn separate(s: &mut Session, p: &Value) -> Result<Value> {
    let on = opt_bool(p, "separate")?.ok_or_else(|| EngineError::Param("param 'separate' is required".into()))?;
    let at = ease_target(s, &json!({"layer": p.get("layer"), "frame": p.get("frame")}))?;
    let EaseAt::Tween { layer, frame, .. } = at else { return Err(EngineError::State("only classic tweens ease property groups separately; a motion tween eases each property already".into())) };
    if on && s.timeline().layer(layer).and_then(|l| l.keyframe_at(frame)).and_then(|k| k.tween.as_ref()).is_some_and(|t| t.kind == TweenKind::Shape) {
        return Err(EngineError::State("a shape tween has one ease for everything; property groups ease separately in classic tweens".into()));
    }
    let l = s.layer_mut(Some(layer))?;
    let i = l.keyframe_index_at(frame).ok_or_else(|| EngineError::State("the tween is gone".into()))?;
    let t: &mut Tween = l.keyframe_mut(i).and_then(|k| k.tween.as_mut()).ok_or_else(|| EngineError::State("the tween is gone".into()))?;
    if on {
        for g in EaseGroup::ALL {
            if !t.has_own(g) {
                let e = t.easing.clone();
                t.set_ease(g, Some(e));
            }
        }
    } else {
        t.groups.clear();
    }
    Ok(json!({"separate": on, "groups": t.groups.iter().map(|(g, _)| g.key()).collect::<Vec<_>>()}))
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    let name = ease_name(str_req(p, "name")?).ok_or_else(|| EngineError::Param("param 'name' must have something in it".into()))?;
    let e = match opt_easing_in(s, p, "easing")? {
        Some(e) => e,
        None => {
            let at = ease_target(s, p)?;
            read(s, &at)?
        }
    };
    let replace = opt_bool(p, "replace")?.unwrap_or(false);
    let mut list = (*s.doc.eases).clone();
    let full = list.len() >= MAX_SAVED_EASES;
    match list.iter_mut().find(|x| x.name.to_lowercase() == name.to_lowercase()) {
        Some(x) if replace => x.ease = e.clone(),
        Some(x) => return Err(EngineError::State(format!("an ease is already saved as '{}'; pass replace: true to replace it", x.name))),
        None if full => return Err(EngineError::State(format!("a document keeps at most {MAX_SAVED_EASES} saved eases; delete one first"))),
        None => list.push(SavedEase { name: name.clone(), ease: e.clone() }),
    }
    s.doc.eases = Arc::new(list);
    Ok(json!({"saved": name, "easing": e.label(), "count": s.doc.eases.len()}))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    let name = str_req(p, "name")?;
    saved_ease(&s.doc, name)?;
    let want = name.trim().to_lowercase();
    let mut list = (*s.doc.eases).clone();
    list.retain(|x| x.name.to_lowercase() != want);
    s.doc.eases = Arc::new(list);
    Ok(json!({"deleted": name.trim(), "count": s.doc.eases.len()}))
}

fn rename(s: &mut Session, p: &Value) -> Result<Value> {
    let name = str_req(p, "name")?;
    let to = ease_name(str_req(p, "to")?).ok_or_else(|| EngineError::Param("param 'to' must have something in it".into()))?;
    saved_ease(&s.doc, name)?;
    let (from, want) = (name.trim().to_lowercase(), to.to_lowercase());
    if from != want && s.doc.eases.iter().any(|x| x.name.to_lowercase() == want) {
        return Err(EngineError::State(format!("an ease is already saved as '{to}'")));
    }
    let mut list = (*s.doc.eases).clone();
    if let Some(x) = list.iter_mut().find(|x| x.name.to_lowercase() == from) {
        x.name = to.clone();
    }
    s.doc.eases = Arc::new(list);
    Ok(json!({"renamed": name.trim(), "to": to}))
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let presets: Vec<Value> = Easing::named()
        .map(|e| {
            let (f, d) = e.family().map_or((None, None), |(f, d)| (Some(f.label()), Some(d.label())));
            json!({"name": e.name(), "label": e.label(), "family": f, "direction": d})
        })
        .chain(std::iter::once(json!({"name": "Classic", "label": "Classic Ease", "easing": {"classic": 0}, "strength": "-100 (in) … 100 (out)"})))
        .collect();
    let saved: Vec<Value> = s.doc.eases.iter().map(|x| json!({"name": x.name, "label": x.ease.label(), "kind": kind(&x.ease), "easing": easing_json(&x.ease)})).collect();
    let plugins: Vec<Value> = plugin_eases(s).into_iter().map(|(p, c)| json!({"name": c.name, "plugin": p.id(), "plugin_name": p.manifest().name, "ease": c.id, "easing": {"plugin": p.id(), "ease": c.id}})).collect();
    Ok(json!({"presets": presets, "saved": saved, "plugins": plugins}))
}

/// The eases of every installed ease plug-in, by plug-in id then in declaration order.
pub fn plugin_eases(s: &Session) -> Vec<(Arc<anicraft_plugins::Plugin>, anicraft_plugins::SampledCurve)> {
    s.plugins.list().filter(|p| p.manifest().kind == anicraft_plugins::Kind::Ease).flat_map(|p| p.eases().iter().map(|c| (p.clone(), c.clone())).collect::<Vec<_>>()).collect()
}

fn copy(s: &mut Session, p: &Value) -> Result<Value> {
    let at = ease_target(s, p)?;
    let e = read(s, &at)?;
    let label = e.label();
    s.ease_clipboard = Some(e);
    Ok(json!({"copied": label}))
}

fn paste(s: &mut Session, p: &Value) -> Result<Value> {
    let e = s.ease_clipboard.clone().ok_or_else(|| EngineError::State("no ease has been copied (Copy Ease first)".into()))?;
    let at = ease_target(s, p)?;
    write(s, &at, e)
}
