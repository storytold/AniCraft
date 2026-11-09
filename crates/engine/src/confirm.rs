//! Which commands need the user's confirmation, and why (ADR 0004).
//!
//! The rules live in the engine so every front end (desktop, CLI, agents) is equally safe:
//! irreversible actions are refused unless the params carry `"confirm": true`.

use crate::Session;
use anicraft_doc::{Keyframe, Layer, LayerId};
use serde_json::Value;
use std::path::Path;

/// A confirmation the user must give before a command runs.
#[derive(Clone, Debug, PartialEq)]
pub struct Confirmation {
    pub title: String,
    pub message: String,
    /// Label for the destructive button, e.g. "Delete Layer".
    pub action: String,
    /// True when undo cannot bring the loss back (unsaved work, files on disk).
    pub irreversible: bool,
    /// `message` before its values go in, with `{name}` placeholders, and those values: what a
    /// front end translates to show the message in another language (ADR 0022).
    pub template: &'static str,
    pub args: Vec<(&'static str, Arg)>,
}

/// A value in a confirmation's message.
#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    /// Shown as it is: a name, a path, a frame number.
    Text(String),
    /// A count of things, with English `one` and `other` forms around `{n}`: (3, "{n} layer", "{n} layers").
    Count(usize, &'static str, &'static str),
}

impl Confirmation {
    fn new(title: &str, template: &'static str, args: Vec<(&'static str, Arg)>, action: &str, irreversible: bool) -> Confirmation {
        let message = fill(template, &args, |n, one, other| (if n == 1 { one } else { other }).replace("{n}", &n.to_string()));
        Confirmation { title: title.into(), message, action: action.into(), irreversible, template, args }
    }
}

/// `template` with each `{name}` replaced by its value, counts written by `count(n, one, other)`.
/// One pass, so a value that itself contains `{…}` (a layer named that way) is left alone.
pub fn fill(template: &str, args: &[(&str, Arg)], count: impl Fn(usize, &str, &str) -> String) -> String {
    let mut out = String::with_capacity(template.len() + 32);
    let mut rest = template;
    while let Some((before, after)) = rest.split_once('{') {
        out.push_str(before);
        let found = after.split_once('}').and_then(|(key, tail)| args.iter().find(|(k, _)| *k == key).map(|(_, v)| (v, tail)));
        match found {
            Some((v, tail)) => {
                match v {
                    Arg::Text(t) => out.push_str(t),
                    Arg::Count(n, one, other) => out.push_str(&count(*n, one, other)),
                }
                rest = tail;
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn text(v: impl ToString) -> Arg {
    Arg::Text(v.to_string())
}

fn keyframes(n: usize) -> Arg {
    Arg::Count(n, "{n} keyframe", "{n} keyframes")
}

fn elements(n: usize) -> Arg {
    Arg::Count(n, "{n} element", "{n} elements")
}

/// Whether the params carry an explicit confirmation.
pub fn confirmed(p: &Value) -> bool {
    p.get("confirm").and_then(Value::as_bool) == Some(true)
}

fn doc_name(s: &Session) -> String {
    s.path.as_ref().and_then(|p| p.file_name()).map_or_else(|| "Untitled".to_string(), |n| n.to_string_lossy().into_owned())
}

fn layer_content(l: &Layer) -> (usize, usize) {
    let keys = l.keyframes.iter().filter(|k| !k.elements.is_empty()).count();
    let elements = l.keyframes.iter().map(|k| k.elements.len()).sum();
    (keys, elements)
}

fn target_layer<'a>(s: &'a Session, p: &Value) -> Option<&'a Layer> {
    let id = p.get("layer").and_then(Value::as_u64).map_or(s.layer, LayerId);
    s.doc.timeline(s.scene).layer(id)
}

fn target_frame(s: &Session, p: &Value) -> u32 {
    p.get("frame").and_then(Value::as_u64).and_then(|f| u32::try_from(f).ok()).unwrap_or(s.frame)
}

fn target_count(p: &Value) -> u32 {
    p.get("count").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()).unwrap_or(1).max(1)
}

/// Keyframes with artwork that `hit` selects on the target layers (`layers` or `layer`), as
/// `(layer name, element count)`.
fn lost_keys(s: &Session, p: &Value, hit: impl Fn(&Keyframe) -> bool) -> Vec<(String, usize)> {
    let ids: Vec<LayerId> = match p.get("layers").and_then(Value::as_array) {
        Some(v) => v.iter().filter_map(Value::as_u64).map(LayerId).take(1000).collect(),
        None => vec![p.get("layer").and_then(Value::as_u64).map_or(s.layer, LayerId)],
    };
    ids.iter()
        .filter_map(|id| s.doc.timeline(s.scene).layer(*id))
        .flat_map(|l| l.keyframes.iter().filter(|k| !k.elements.is_empty() && hit(k)).map(|k| (l.name.clone(), k.elements.len())))
        .collect()
}

/// `template` names the file `{path}`: "… Saving will replace it …" or "… Exporting …".
fn overwrite(path: &Path, template: &'static str) -> Confirmation {
    Confirmation::new("Replace existing file?", template, vec![("path", text(path.display()))], "Replace", true)
}

const SAVING_REPLACES: &str = "{path} already exists. Saving will replace it; this cannot be undone.";
const EXPORTING_REPLACES: &str = "{path} already exists. Exporting will replace it; this cannot be undone.";

/// The confirmation `id` with `p` needs right now, if any.
pub fn confirmation(s: &Session, id: &str, p: &Value) -> Option<Confirmation> {
    match id {
        "file.discard_recovery" => {
            // Always: the autosaved work is the last copy there is.
            let dir = Path::new(p.get("dir").and_then(Value::as_str)?);
            let e = crate::recovery::find(dir, p.get("session").and_then(Value::as_str)?).ok()?;
            Some(Confirmation::new(
                "Discard autosaved document?",
                "The autosaved copy of “{name}” will be deleted, with the work it holds.",
                vec![("name", text(e.meta.name))],
                "Discard",
                true,
            ))
        }
        "file.new" | "file.new_sample" | "file.open" | "file.recover" if s.is_dirty() => Some(Confirmation::new(
            "Discard unsaved changes?",
            "“{name}” has changes that are not saved. They will be lost, and its undo history is cleared.",
            vec![("name", text(doc_name(s)))],
            "Discard Changes",
            true,
        )),
        "file.save" | "file.save_as" => {
            let path = Path::new(p.get("path").and_then(Value::as_str)?);
            let same = s.path.as_deref() == Some(path);
            (!same && path.exists()).then(|| overwrite(path, SAVING_REPLACES))
        }
        "file.export_png" | "file.export_gif" | "file.export_svg" | "file.export_video" | "file.export_lottie" | "file.export_audio" => {
            let path = Path::new(p.get("path").and_then(Value::as_str)?);
            path.exists().then(|| overwrite(path, EXPORTING_REPLACES))
        }
        "file.export_sprite_sheet" => {
            // The sheet and its frame map: either one already there is replaced.
            let path = Path::new(p.get("path").and_then(Value::as_str)?);
            let json = path.with_extension("json");
            [path, json.as_path()].into_iter().find(|f| f.exists()).map(|f| overwrite(f, EXPORTING_REPLACES))
        }
        "file.export_png_sequence" => {
            let dir = Path::new(p.get("dir").and_then(Value::as_str)?);
            let has_frames = std::fs::read_dir(dir).ok()?.take(10_000).flatten().any(|e| {
                let n = e.file_name();
                let n = n.to_string_lossy();
                n.starts_with("frame_") && n.ends_with(".png")
            });
            has_frames.then(|| {
                let template = "{dir} already contains exported frames. Matching files will be replaced; this cannot be undone.";
                Confirmation::new("Replace exported frames?", template, vec![("dir", text(dir.display()))], "Replace", true)
            })
        }
        "sound.delete" => {
            // A sound in use comes off every keyframe that plays it.
            let id = anicraft_doc::SoundId(p.get("sound").and_then(Value::as_u64)?);
            let snd = s.doc.sound(id)?;
            let used: usize = s.doc.timelines().map(|tl| crate::cmds_sound::uses_in(tl, id)).sum();
            (used > 0).then(|| {
                Confirmation::new(
                    "Delete sound?",
                    "“{name}” plays on {keyframes}. It comes off them and out of the library. You can undo this.",
                    vec![("name", text(&snd.name)), ("keyframes", keyframes(used))],
                    "Delete Sound",
                    false,
                )
            })
        }
        "layer.delete" => {
            let l = target_layer(s, p)?;
            if l.kind == anicraft_doc::LayerKind::Folder {
                // A folder goes with everything in it.
                let tl = s.doc.timeline(s.scene);
                let i = tl.layers.iter().position(|x| x.id == l.id)?;
                let inner = tl.layers.get(tl.group_range(i))?;
                let n = inner.len().saturating_sub(1);
                let (keys, elements) = inner.iter().map(|x| layer_content(x)).fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
                return (n > 0).then(|| {
                    let template = if elements > 0 {
                        "Folder “{name}” holds {layers} with {elements} in {keyframes}. They are deleted with it. You can undo this."
                    } else {
                        "Folder “{name}” holds {layers}. They are deleted with it. You can undo this."
                    };
                    let args = vec![("name", text(&l.name)), ("layers", Arg::Count(n, "{n} layer", "{n} layers")), ("elements", self::elements(elements)), ("keyframes", self::keyframes(keys))];
                    Confirmation::new("Delete folder?", template, args, "Delete Folder", false)
                });
            }
            let (keys, n) = layer_content(l);
            (n > 0).then(|| {
                let args = vec![("name", text(&l.name)), ("keyframes", keyframes(keys)), ("elements", elements(n))];
                Confirmation::new("Delete layer?", "Layer “{name}” has {keyframes} with {elements}. You can undo this.", args, "Delete Layer", false)
            })
        }
        "scene.delete" => {
            let i = crate::cmds_scene::target_scene(s, p).ok()?;
            let sc = s.doc.scenes.get(i)?;
            let (keys, n) = sc.timeline.layers.iter().map(|l| layer_content(l)).fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
            (n > 0 && s.doc.scenes.len() > 1).then(|| {
                let args = vec![("name", text(&sc.name)), ("keyframes", keyframes(keys)), ("elements", elements(n))];
                Confirmation::new("Delete scene?", "Scene “{name}” has {keyframes} with {elements}. You can undo this.", args, "Delete Scene", false)
            })
        }
        "timeline.clear_keyframe" => {
            let (f, n) = (target_frame(s, p), target_count(p));
            let lost = lost_keys(s, p, |k| k.start >= f && k.start - f < n);
            let (keys, total) = (lost.len(), lost.iter().map(|(_, e)| e).sum::<usize>());
            (total > 0).then(|| {
                let (template, args) = match lost.as_slice() {
                    [(name, e)] if n == 1 => ("The keyframe at frame {frame} on “{name}” holds {elements}. You can undo this.", vec![("frame", text(f + 1)), ("name", text(name)), ("elements", elements(*e))]),
                    _ => ("The selected frames hold {keyframes} with {elements}. You can undo this.", vec![("keyframes", keyframes(keys)), ("elements", elements(total))]),
                };
                let (title, action) = if keys == 1 { ("Clear keyframe?", "Clear Keyframe") } else { ("Clear keyframes?", "Clear Keyframes") };
                Confirmation::new(title, template, args, action, false)
            })
        }
        "timeline.remove_frame" => {
            // A keyframe whose whole span is removed takes its artwork with it.
            let (f, n) = (target_frame(s, p), target_count(p));
            let lost = lost_keys(s, p, |k| k.start >= f && k.end() <= f.saturating_add(n));
            let (keys, total) = (lost.len(), lost.iter().map(|(_, e)| e).sum::<usize>());
            (total > 0).then(|| {
                let (template, args) = match lost.as_slice() {
                    [(name, e)] if n == 1 => (
                        "Frame {frame} is a one-frame keyframe on “{name}” with {elements}; removing it deletes that artwork. You can undo this.",
                        vec![("frame", text(f + 1)), ("name", text(name)), ("elements", elements(*e))],
                    ),
                    _ => ("Removing these frames deletes {keyframes} with {elements}. You can undo this.", vec![("keyframes", keyframes(keys)), ("elements", elements(total))]),
                };
                let (title, action) = if n == 1 { ("Remove frame?", "Remove Frame") } else { ("Remove frames?", "Remove Frames") };
                Confirmation::new(title, template, args, action, false)
            })
        }
        "plugin.install" => {
            let path = Path::new(p.get("path").and_then(Value::as_str)?);
            let (m, dest) = s.plugins.install_overwrites(path)?;
            let old = s.plugins.get(&m.id).map(|o| o.manifest().version.clone()).filter(|v| !v.is_empty());
            let mut args = vec![("name", text(&m.name)), ("path", text(path.display())), ("dest", text(dest.display()))];
            let template = match old {
                Some(v) => {
                    args.push(("version", text(v)));
                    "“{name}” {version} is already installed. Installing {path} replaces its file in the plug-ins folder ({dest}); this cannot be undone."
                }
                None => "“{name}” is already installed. Installing {path} replaces its file in the plug-ins folder ({dest}); this cannot be undone.",
            };
            Some(Confirmation::new("Replace plug-in?", template, args, "Replace Plug-in", true))
        }
        "plugin.remove" => {
            let plugin = s.plugins.get(p.get("id").and_then(Value::as_str)?)?;
            let file = plugin.source().filter(|_| s.plugins.owns_file(&plugin))?;
            Some(Confirmation::new(
                "Remove plug-in?",
                "“{name}” is uninstalled and its file is deleted from the plug-ins folder ({path}). This cannot be undone.",
                vec![("name", text(&plugin.manifest().name)), ("path", text(file.display()))],
                "Remove Plug-in",
                true,
            ))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_puts_each_value_in_once() {
        let english = |n: usize, one: &str, other: &str| (if n == 1 { one } else { other }).replace("{n}", &n.to_string());
        let args = vec![("name", text("{layers}")), ("layers", Arg::Count(1, "{n} layer", "{n} layers")), ("keys", keyframes(3))];
        // A value is never filled in again (a layer named "{layers}"), and unknown or unclosed
        // braces stay as written.
        assert_eq!(fill("“{name}” holds {layers} and {keys}; {other} {", &args, english), "“{layers}” holds 1 layer and 3 keyframes; {other} {");
    }
}
