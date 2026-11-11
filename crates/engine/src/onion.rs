//! Onion skin and Edit Multiple Frames (ADR 0047).
//!
//! **Onion skin** draws the frames around the playhead under it, faded: frames before it in the
//! *past* colour, frames after it in the *future* colour, each fainter the farther it is (when
//! fading is on). Which frames are shown is set by two markers: they follow the playhead
//! (`before` and `after` frames of it) or stay where they were put (anchored). Options: tint or
//! the artwork's own colours, outlines only, keyframes only.
//!
//! **Edit Multiple Frames** shows the artwork on every keyframe within the markers at full
//! strength, as if on one frame, and `frames.edit_multiple` moves it: the elements chosen (or all
//! of them) on each of those keyframes, as one undo step.
//!
//! Both are *view* settings: the shell keeps an [`OnionSkin`] and passes it with each render job
//! (keyed by [`OnionSkin::key`], so the frame cache keeps working), and the composition here is a
//! pure function of the document, the frame and the settings. Only the moves change the
//! document, through the command.
//!
//! Behaviour learned from Adobe's public help pages on onion skinning and from watching the
//! program; nothing was copied.

use crate::params::*;
use crate::{CommandSpec, EngineError, MAX_FRAME, Result, Session};
use anicraft_doc::{ColorEffect, DisplayList, Document, ElementId, LayerId, LayerKind, Rgba, Timeline};
use anicraft_geom::{Affine, Rect, Vec2};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};
use std::hash::{Hash, Hasher};

/// Most frames onion skin or Edit Multiple Frames draws besides the playhead's (the nearest
/// ones are kept): each is one more evaluation of the scene.
pub const MAX_EXTRA_FRAMES: usize = 120;
/// Opacity of the nearest onion frame, and of the farthest when fading by distance.
pub const NEAR_ALPHA: f32 = 0.5;
pub const FAR_ALPHA: f32 = 0.12;
/// How much of the past or future colour tints an onion frame (the rest is its own colour, so
/// fills and lines inside it stay apart).
pub const TINT_AMOUNT: f32 = 0.8;
/// The marker presets (Onion 2, Onion 5): frames on each side of the playhead.
pub const PRESETS: [u32; 2] = [2, 5];

/// Where the onion markers are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Markers {
    /// `before` frames before the playhead to `after` frames after it, following it.
    Relative { before: u32, after: u32 },
    /// Fixed frames (0-based, inclusive), whatever the playhead does. `last` beyond the end
    /// reaches the end however long the timeline gets (Onion All).
    Anchored { first: u32, last: u32 },
}

/// Onion skin and Edit Multiple Frames settings (view state the shell owns).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OnionSkin {
    /// Onion skin is on: frames around the playhead show under it.
    pub ghosts: bool,
    /// Edit Multiple Frames is on: keyframes within the markers show at full strength (and
    /// onion frames are not drawn).
    pub edit_multiple: bool,
    pub markers: Markers,
    /// Tint onion frames with `past` and `future`; off, they keep their own colours.
    pub tint: bool,
    pub past: Rgba,
    pub future: Rgba,
    /// Fainter with distance from the playhead; off, every onion frame has [`NEAR_ALPHA`].
    pub fade: bool,
    /// Onion frames as hairline outlines in their colour.
    pub outlines: bool,
    /// Only frames where a keyframe starts (on each layer, its own keyframes).
    pub keyframes_only: bool,
}

impl Default for OnionSkin {
    fn default() -> Self {
        OnionSkin {
            ghosts: true,
            edit_multiple: false,
            markers: Markers::Relative { before: 2, after: 2 },
            tint: true,
            past: Rgba::hex(0x4f8dff),
            future: Rgba::hex(0x4cd07d),
            fade: true,
            outlines: false,
            keyframes_only: false,
        }
    }
}

/// Most frames a marker reaches from the playhead.
pub const MAX_REACH: u32 = MAX_FRAME;

impl OnionSkin {
    /// Whether it changes what the stage shows.
    pub fn shows(&self) -> bool {
        self.ghosts || self.edit_multiple
    }

    /// The frames within the markers (0-based, inclusive) for a timeline of `frames` frames
    /// with the playhead on `playhead`: always inside the timeline, `first <= last`.
    pub fn range(&self, playhead: u32, frames: u32) -> (u32, u32) {
        let end = frames.max(1) - 1;
        match self.markers {
            Markers::Relative { before, after } => {
                let p = playhead.min(end);
                (p.saturating_sub(before.min(MAX_REACH)), p.saturating_add(after.min(MAX_REACH)).min(end))
            }
            Markers::Anchored { first, last } => {
                let (a, b) = (first.min(last), first.max(last));
                (a.min(end), b.min(end))
            }
        }
    }

    /// Anchors the markers where they are now (`on`), or makes them follow the playhead again,
    /// the same distance from it (a marker the playhead has passed is moved onto it).
    pub fn set_anchored(&mut self, on: bool, playhead: u32, frames: u32) {
        let (first, last) = self.range(playhead, frames);
        self.markers = match (on, self.markers) {
            (true, Markers::Relative { .. }) => Markers::Anchored { first, last },
            (false, Markers::Anchored { .. }) => {
                let p = playhead.min(frames.max(1) - 1);
                Markers::Relative { before: p.saturating_sub(first), after: last.saturating_sub(p) }
            }
            (_, m) => m,
        };
    }

    pub fn anchored(&self) -> bool {
        matches!(self.markers, Markers::Anchored { .. })
    }

    /// Onion 2 / Onion 5: `n` frames on each side, following the playhead.
    pub fn preset(&mut self, n: u32) {
        self.markers = Markers::Relative { before: n.min(MAX_REACH), after: n.min(MAX_REACH) };
    }

    /// Onion All: the whole timeline, however long it gets.
    pub fn all(&mut self) {
        self.markers = Markers::Anchored { first: 0, last: MAX_FRAME };
    }

    /// Whether the markers are Onion All's.
    pub fn is_all(&self) -> bool {
        self.markers == Markers::Anchored { first: 0, last: MAX_FRAME }
    }

    /// Moves the start marker (`start`) or the end marker to frame `to`, keeping the range on
    /// the right side of the playhead when relative, and never past the other marker.
    pub fn drag_marker(&mut self, start: bool, to: u32, playhead: u32, frames: u32) {
        let end = frames.max(1) - 1;
        let to = to.min(end);
        let (first, last) = self.range(playhead, frames);
        let p = playhead.min(end);
        self.markers = match self.markers {
            Markers::Relative { after, .. } if start => Markers::Relative { before: p.saturating_sub(to.min(p)), after },
            Markers::Relative { before, .. } => Markers::Relative { before, after: to.max(p) - p },
            Markers::Anchored { .. } if start => Markers::Anchored { first: to.min(last), last },
            Markers::Anchored { .. } => Markers::Anchored { first, last: to.max(first) },
        };
    }

    /// Opacity of an onion frame `distance` frames from the playhead, on a side whose marker
    /// is `reach` frames away.
    pub fn alpha(&self, distance: u32, reach: u32) -> f32 {
        if !self.fade || reach <= 1 {
            return NEAR_ALPHA;
        }
        let t = (distance.saturating_sub(1) as f32 / (reach - 1) as f32).clamp(0.0, 1.0);
        NEAR_ALPHA * (1.0 - t) + FAR_ALPHA * t
    }

    /// The colour effect an onion frame gets, and the outline colour (when outlines are on).
    pub fn look(&self, past: bool, distance: u32, reach: u32) -> (ColorEffect, Option<Rgba>) {
        let alpha = self.alpha(distance, reach);
        let color = if past { self.past } else { self.future };
        let (tint, tint_amount) = if self.tint { (color, TINT_AMOUNT) } else { (Rgba::WHITE, 0.0) };
        let outline = self.outlines.then(|| color.with_alpha(alpha * color.a.clamp(0.0, 1.0)));
        (ColorEffect { alpha, tint, tint_amount }, outline)
    }

    /// A hash of everything that changes the picture, for render-cache keys.
    pub fn key(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (self.ghosts, self.edit_multiple, self.markers, self.tint, self.fade, self.outlines, self.keyframes_only).hash(&mut h);
        for c in [self.past, self.future] {
            [c.r, c.g, c.b, c.a].map(f32::to_bits).hash(&mut h);
        }
        h.finish()
    }
}

/// One extra frame to draw under the playhead's: frame `frame`, only `layers`' artwork.
#[derive(Clone, Debug, PartialEq)]
pub struct Extra {
    pub frame: u32,
    pub layers: Vec<LayerId>,
    /// An onion frame before the playhead (`Some(true)`), after it (`Some(false)`), or an Edit
    /// Multiple Frames keyframe at full strength (`None`).
    pub past: Option<bool>,
    /// Frames from the playhead.
    pub distance: u32,
}

/// Whether onion skin draws `layer` (Animate doesn't onion-skin locked layers).
fn ghosted(tl: &Timeline, i: usize) -> bool {
    tl.layers.get(i).is_some_and(|l| l.visible && !l.locked && l.kind != LayerKind::Folder && !tl.locked_by_folder(l.id) && !tl.hidden_by_folder(i))
}

/// Whether Edit Multiple Frames shows and edits `layer`'s other keyframes: it is shown and
/// unlocked, and holds artwork (not a folder, the camera or an armature posed by its bones).
pub fn multi_editable(tl: &Timeline, i: usize) -> bool {
    ghosted(tl, i) && tl.layers.get(i).is_some_and(|l| !matches!(l.kind, LayerKind::Camera | LayerKind::Armature))
}

/// The onion frames to draw for the playhead on `playhead`, farthest first (nearest on top),
/// past and future taking turns at each distance.
pub fn onion_frames(tl: &Timeline, playhead: u32, skin: &OnionSkin) -> Vec<Extra> {
    let (first, last) = skin.range(playhead, tl.frame_count());
    let layers: Vec<usize> = (0..tl.layers.len()).filter(|&i| ghosted(tl, i)).collect();
    if layers.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<Extra> = Vec::new();
    // Nearest first, so the cap keeps the nearest; reversed at the end.
    let reach = playhead.saturating_sub(first).max(last.saturating_sub(playhead));
    for d in 1..=reach {
        if out.len() >= MAX_EXTRA_FRAMES {
            break;
        }
        // Future first: the list is reversed below, so at each distance the past frame is drawn
        // first.
        for (past, f) in [(false, playhead.checked_add(d)), (true, playhead.checked_sub(d))] {
            let Some(f) = f.filter(|f| (first..=last).contains(f)) else { continue };
            let ids: Vec<LayerId> = if skin.keyframes_only {
                // A layer's keyframe starting there; the one the playhead is on only when it
                // tweens (otherwise it looks the same as the playhead's frame).
                layers.iter().filter_map(|&i| tl.layers.get(i)).filter(|l| l.keyframe_starting_at(f).and_then(|k| l.keyframes.get(k)).is_some_and(|k| !k.covers(playhead) || k.tween.is_some() || k.motion.is_some())).map(|l| l.id).collect()
            } else {
                layers.iter().filter_map(|&i| tl.layers.get(i)).map(|l| l.id).collect()
            };
            if !ids.is_empty() && out.len() < MAX_EXTRA_FRAMES {
                out.push(Extra { frame: f, layers: ids, past: Some(past), distance: d });
            }
        }
    }
    out.reverse();
    out
}

/// The keyframes Edit Multiple Frames draws besides the playhead's: on each editable layer,
/// every keyframe whose span reaches into `first..=last` except the one under the playhead,
/// drawn as it looks on its first frame. Grouped by that frame, in frame order.
pub fn multi_frames(tl: &Timeline, playhead: u32, first: u32, last: u32) -> Vec<Extra> {
    let mut at: BTreeMap<u32, Vec<LayerId>> = BTreeMap::new();
    for (i, l) in tl.layers.iter().enumerate() {
        if !multi_editable(tl, i) {
            continue;
        }
        for k in l.keyframes.iter().filter(|k| k.start <= last && k.end() > first && !k.covers(playhead)) {
            at.entry(k.start).or_default().push(l.id);
        }
    }
    let mut out: Vec<Extra> = at.into_iter().map(|(frame, layers)| Extra { frame, layers, past: None, distance: frame.abs_diff(playhead) }).collect();
    if out.len() > MAX_EXTRA_FRAMES {
        // Keep the nearest, still in frame order.
        let mut by: Vec<u32> = out.iter().map(|e| e.distance).collect();
        by.sort_unstable();
        let cut = by.get(MAX_EXTRA_FRAMES - 1).copied().unwrap_or(0);
        let mut kept = 0usize;
        out.retain(|e| {
            let keep = e.distance <= cut && kept < MAX_EXTRA_FRAMES;
            kept += usize::from(keep);
            keep
        });
    }
    out
}

/// Keeps only `layers`' items (and the mask artwork their clips need, which draws nothing).
fn keep_layers(dl: &mut DisplayList, layers: &[LayerId]) {
    let set: HashSet<LayerId> = layers.iter().copied().collect();
    dl.items.retain(|i| set.contains(&i.layer) || (i.masks.is_some() && i.group.is_none()));
}

/// The parts as one list, layer by layer: each layer's items from every part, in part order,
/// then the next layer up. So a frame before or after the playhead shows under the playhead's
/// artwork *on its own layer* and over the layers below, as in Animate, and an opaque
/// background never hides it. Also says which part each item came from.
///
/// Reordering items is safe: a group's run lies within one layer (it stays unbroken), and the
/// renderers take mask artwork apart from what they draw, whatever its place in the list.
fn interleave(tl: &Timeline, parts: Vec<DisplayList>) -> (DisplayList, Vec<usize>) {
    let rank: std::collections::HashMap<LayerId, usize> = tl.layers.iter().enumerate().map(|(i, l)| (l.id, i)).collect();
    let mut dl = DisplayList::default();
    let mut from = Vec::new();
    for (k, part) in parts.into_iter().enumerate() {
        from.extend(std::iter::repeat_n(k, part.items.len()));
        dl.append(part);
    }
    // Back to front: the bottom layer (the highest index) first; stable, so parts keep order.
    let mut order: Vec<usize> = (0..dl.items.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(dl.items.get(i).and_then(|it| rank.get(&it.layer)).copied().unwrap_or(usize::MAX)));
    let mut items: Vec<Option<anicraft_doc::DisplayItem>> = std::mem::take(&mut dl.items).into_iter().map(Some).collect();
    dl.items = order.iter().filter_map(|&i| items.get_mut(i).and_then(Option::take)).collect();
    let from = order.iter().map(|&i| from.get(i).copied().unwrap_or(0)).collect();
    (dl, from)
}

/// Scene `scene` at its frame `local` with what `skin` adds (nothing for `None`): onion frames,
/// or Edit Multiple Frames' keyframes, each under the playhead's artwork on its own layer.
/// `eval` evaluates one frame of the scene (with or without its camera).
pub fn compose(doc: &Document, scene: usize, local: u32, skin: Option<&OnionSkin>, eval: impl Fn(usize, u32) -> DisplayList) -> DisplayList {
    let now = eval(scene, local);
    let Some(skin) = skin.filter(|s| s.shows()) else { return now };
    let tl = doc.timeline(scene);
    let (first, last) = skin.range(local, tl.frame_count());
    let extras = if skin.edit_multiple { multi_frames(tl, local, first, last) } else { onion_frames(tl, local, skin) };
    if extras.is_empty() {
        return now;
    }
    let mut parts = Vec::with_capacity(extras.len() + 1);
    for e in extras {
        let mut part = eval(scene, e.frame);
        keep_layers(&mut part, &e.layers);
        if let Some(past) = e.past {
            let reach = if past { local.saturating_sub(first) } else { last.saturating_sub(local) };
            let (fx, outline) = skin.look(past, e.distance, reach);
            for item in &mut part.items {
                item.color = item.color.concat(&fx);
                if outline.is_some() && item.masks.is_none() {
                    item.outline = outline;
                }
            }
        }
        parts.push(part);
    }
    parts.push(now);
    interleave(tl, parts).0
}

// ----- Edit Multiple Frames: artwork, picking and moves ------------------------------------

/// An element on one keyframe: its layer, the keyframe's first frame and its id (an element
/// keeps its id from keyframe to keyframe, so the frame says which copy).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FrameItem {
    pub layer: LayerId,
    pub frame: u32,
    pub element: ElementId,
}

impl FrameItem {
    pub fn to_json(&self) -> Value {
        json!([self.layer.0, self.frame, self.element.0])
    }
}

impl Session {
    /// What Edit Multiple Frames shows in `first..=last`, layer by layer as the stage draws it
    /// (each keyframe as it looks on its first frame, the playhead's as it looks there), with
    /// the keyframe start of each item's layer (`None`: a layer it doesn't edit).
    fn multi_list(&self, first: u32, last: u32) -> (DisplayList, Vec<Option<u32>>) {
        let tl = self.doc.timeline(self.scene);
        let mut parts = Vec::new();
        let mut starts: Vec<Vec<(LayerId, u32)>> = Vec::new();
        for e in multi_frames(tl, self.frame, first, last) {
            let mut dl = anicraft_doc::evaluate_scene(&self.doc, self.scene, e.frame);
            keep_layers(&mut dl, &e.layers);
            parts.push(dl);
            starts.push(e.layers.iter().map(|l| (*l, e.frame)).collect());
        }
        // The playhead's frame: every editable layer, at the start of its keyframe there.
        let now: Vec<(LayerId, u32)> = tl.layers.iter().enumerate().filter(|(i, _)| multi_editable(tl, *i)).filter_map(|(_, l)| l.keyframe_at(self.frame).map(|k| (l.id, k.start))).collect();
        let mut dl = (*self.display_list()).clone();
        keep_layers(&mut dl, &now.iter().map(|(l, _)| *l).collect::<Vec<_>>());
        parts.push(dl);
        starts.push(now);
        let (dl, from) = interleave(tl, parts);
        let frames = dl.items.iter().zip(&from).map(|(it, &k)| starts.get(k).and_then(|s| s.iter().find(|(l, _)| *l == it.layer)).map(|(_, f)| *f)).collect();
        (dl, frames)
    }

    /// Every element Edit Multiple Frames shows in `first..=last`, with its stage bounds (the
    /// playhead's elements as they look there), back to front.
    pub fn multi_frame_items(&self, first: u32, last: u32) -> Vec<(FrameItem, Rect)> {
        let (first, last) = (first.min(last), first.max(last));
        let mut out: Vec<(FrameItem, Rect)> = Vec::new();
        let mut index: std::collections::HashMap<FrameItem, usize> = std::collections::HashMap::new();
        let (dl, frames) = self.multi_list(first, last);
        {
            for (item, frame) in dl.items.iter().zip(frames) {
                let Some(frame) = frame else { continue };
                let key = FrameItem { layer: item.layer, frame, element: item.element };
                let pad = item.stroke.as_ref().map_or(0.0, anicraft_doc::Stroke::reach);
                let b = item.path.bounds().inflate(pad).transformed(&item.transform);
                match index.get(&key).and_then(|&i| out.get_mut(i)) {
                    Some((_, r)) => *r = r.union(&b),
                    None => {
                        index.insert(key, out.len());
                        out.push((key, b));
                    }
                }
            }
        }
        out
    }

    /// The topmost element Edit Multiple Frames shows under `p` in `first..=last`.
    pub fn multi_frame_pick(&self, first: u32, last: u32, p: Vec2, slop: f32) -> Option<FrameItem> {
        let (first, last) = (first.min(last), first.max(last));
        let (dl, frames) = self.multi_list(first, last);
        let i = crate::pick_index(&dl, p, slop, |i, _| frames.get(i).copied().flatten().is_some())?;
        let item = dl.items.get(i)?;
        Some(FrameItem { layer: item.layer, frame: frames.get(i).copied().flatten()?, element: item.element })
    }
}

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c("frames.artwork", "Artwork on Multiple Frames", "", None, "{first, last} lists every element Edit Multiple Frames shows in frames first..=last (0-based): [layer, frame (its keyframe's first frame), id] with stage bounds; the playhead's elements as they look there", false, artwork as crate::RunFn),
        c("frames.edit_multiple", "Move on Multiple Frames", "", None, "{first, last, dx, dy, items?: [[layer, frame, id], ..]} moves artwork on every keyframe within frames first..=last (0-based) on visible, unlocked layers, as Edit Multiple Frames does: the items given (a keyframe's first frame and an element id), or all of them; a motion tween's path and X/Y keys move with its object; one undo step", true, edit_multiple),
    ]
}

/// The `first` and `last` params, in order.
fn range_param(p: &Value) -> Result<(u32, u32)> {
    let req = |k: &str| opt_u32(p, k)?.ok_or_else(|| EngineError::Param(format!("param '{k}' is required (a frame, 0-based)")));
    let (a, b) = (req("first")?, req("last")?);
    if a.max(b) > MAX_FRAME {
        return Err(EngineError::Param(format!("frames go up to {MAX_FRAME}")));
    }
    Ok((a.min(b), a.max(b)))
}

fn artwork(s: &mut Session, p: &Value) -> Result<Value> {
    let (first, last) = range_param(p)?;
    let items: Vec<Value> = s.multi_frame_items(first, last).into_iter().map(|(it, b)| json!({"layer": it.layer.0, "frame": it.frame, "id": it.element.0, "bounds": [b.min.x, b.min.y, b.max.x, b.max.y]})).collect();
    Ok(json!({"first": first, "last": last, "items": items}))
}

/// Most items `frames.edit_multiple` takes.
const MAX_ITEMS: usize = 100_000;

fn items_param(p: &Value) -> Result<Option<Vec<FrameItem>>> {
    let Some(v) = p.get("items").filter(|v| !v.is_null()) else { return Ok(None) };
    let bad = || EngineError::Param("param 'items' must be a list of [layer, frame, id]".into());
    let list = v.as_array().ok_or_else(bad)?;
    if list.len() > MAX_ITEMS {
        return Err(EngineError::Param(format!("param 'items' has more than {MAX_ITEMS} items")));
    }
    list.iter()
        .map(|t| {
            let t = t.as_array().filter(|t| t.len() == 3).ok_or_else(bad)?;
            let n = |i: usize| t.get(i).and_then(Value::as_u64).ok_or_else(bad);
            let frame = u32::try_from(n(1)?).map_err(|_| bad())?;
            Ok(FrameItem { layer: LayerId(n(0)?), frame, element: ElementId(n(2)?) })
        })
        .collect::<Result<Vec<_>>>()
        .map(Some)
}

fn edit_multiple(s: &mut Session, p: &Value) -> Result<Value> {
    let (first, last) = range_param(p)?;
    let d = Vec2::new(f32_req(p, "dx")?, f32_req(p, "dy")?);
    if d.x.abs() > crate::cmds_draw::MAX_EXTENT || d.y.abs() > crate::cmds_draw::MAX_EXTENT {
        return Err(EngineError::Param("the move is too large".into()));
    }
    let shown: Vec<FrameItem> = s.multi_frame_items(first, last).into_iter().map(|(it, _)| it).collect();
    let items = match items_param(p)? {
        Some(list) => {
            let have: HashSet<FrameItem> = shown.iter().copied().collect();
            if let Some(bad) = list.iter().find(|it| !have.contains(it)) {
                return Err(EngineError::State(format!(
                    "element {} on layer {} at frame {} is not on a keyframe Edit Multiple Frames shows in frames {first}..={last} (on a visible, unlocked layer)",
                    bad.element.0, bad.layer.0, bad.frame
                )));
            }
            list
        }
        None => shown,
    };
    // One pass per keyframe: (layer, keyframe start) → its ids.
    let mut by_key: BTreeMap<(LayerId, u32), HashSet<ElementId>> = BTreeMap::new();
    for it in &items {
        by_key.entry((it.layer, it.frame)).or_default().insert(it.element);
    }
    let mv = Affine::translate(d.x, d.y);
    let mut moved = 0usize;
    for ((layer, start), ids) in &by_key {
        let l = s.layer_mut(Some(*layer))?;
        let Some(ki) = l.keyframe_starting_at(*start) else { continue };
        let Some(kf) = l.keyframe_mut(ki) else { continue };
        // A motion tween's object carries its path and X/Y keys along (refused, with nothing
        // changed, if they would leave the stage's range).
        if let Some(m) = kf.motion.as_mut()
            && ids.contains(&m.target)
            && d != Vec2::ZERO
            && !m.translate(d)
        {
            return Err(EngineError::State("the motion path would move out of range".into()));
        }
        for e in kf.elements_mut_where(|id| ids.contains(&id)) {
            e.transform = mv.mul(&e.transform);
            moved += 1;
        }
    }
    Ok(json!({"moved": moved, "keyframes": by_key.len(), "dx": d.x, "dy": d.y}))
}

#[cfg(test)]
#[path = "onion_tests.rs"]
mod tests;
