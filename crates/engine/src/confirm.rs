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

fn plural(n: usize, word: &str) -> String {
    if n == 1 { format!("1 {word}") } else { format!("{n} {word}s") }
}

fn target_layer<'a>(s: &'a Session, p: &Value) -> Option<&'a Layer> {
    let id = p.get("layer").and_then(Value::as_u64).map_or(s.layer, LayerId);
    s.timeline().layer(id)
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
        .filter_map(|id| s.timeline().layer(*id))
        .flat_map(|l| l.keyframes.iter().filter(|k| !k.elements.is_empty() && hit(k)).map(|k| (l.name.clone(), k.elements.len())))
        .collect()
}

fn overwrite(path: &Path, what: &str) -> Confirmation {
    Confirmation {
        title: "Replace existing file?".into(),
        message: format!("{} already exists. {what} will replace it; this cannot be undone.", path.display()),
        action: "Replace".into(),
        irreversible: true,
    }
}

/// Removing the camera asks only when it moves (its keyframes are the animation lost).
fn camera_removal(_: &Session, l: &Layer) -> Option<Confirmation> {
    let n = crate::cmds_camera::camera_moves(l);
    (n > 0).then(|| Confirmation {
        title: "Remove camera?".into(),
        message: format!("The camera moves over {}. Removing it removes those moves, and the movie shows the stage as it is. You can undo this.", plural(n, "keyframe")),
        action: "Remove Camera".into(),
        irreversible: false,
    })
}

/// The confirmation `id` with `p` needs right now, if any.
pub fn confirmation(s: &Session, id: &str, p: &Value) -> Option<Confirmation> {
    match id {
        "file.discard_recovery" => {
            // Always: the autosaved work is the last copy there is.
            let dir = Path::new(p.get("dir").and_then(Value::as_str)?);
            let e = crate::recovery::find(dir, p.get("session").and_then(Value::as_str)?).ok()?;
            Some(Confirmation {
                title: "Discard autosaved document?".into(),
                message: format!("The autosaved copy of “{}” will be deleted, with the work it holds.", e.meta.name),
                action: "Discard".into(),
                irreversible: true,
            })
        }
        "file.save_as_template" => {
            let path = crate::cmds_template::target(p).ok()?;
            crate::vfs::exists(&path).then(|| overwrite(&path, "Saving the template"))
        }
        "file.delete_template" => {
            let path = crate::cmds_template::target(p).ok()?;
            let name = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            Some(Confirmation {
                title: "Delete template?".into(),
                message: format!("The template “{name}” and its picture will be deleted from your templates. This cannot be undone."),
                action: "Delete Template".into(),
                irreversible: true,
            })
        }
        // Brushes of your own live in your preferences, outside undo (ADR 0068).
        "brush.preset_delete" => {
            let name = p.get("name").and_then(Value::as_str)?;
            let i = s.brushes.position(name)?;
            let name = s.brushes.user().get(i)?.name.clone();
            Some(Confirmation {
                title: "Delete brush?".into(),
                message: format!("The brush “{name}” will be deleted from your brushes. This cannot be undone."),
                action: "Delete Brush".into(),
                irreversible: true,
            })
        }
        "brush.preset_export" => {
            let path = Path::new(p.get("path").and_then(Value::as_str)?);
            crate::vfs::exists(path).then(|| overwrite(path, "Exporting the brushes"))
        }
        "file.new" | "file.new_sample" | "file.new_from_template" | "file.open" | "file.open_recent" | "file.recover" if s.is_dirty() => Some(Confirmation {
            title: "Discard unsaved changes?".into(),
            message: format!("“{}” has changes that are not saved. They will be lost, and its undo history is cleared.", doc_name(s)),
            action: "Discard Changes".into(),
            irreversible: true,
        }),
        "file.save" | "file.save_as" => {
            let path = Path::new(p.get("path").and_then(Value::as_str)?);
            let same = s.path.as_deref() == Some(path);
            (!same && crate::vfs::exists(path)).then(|| overwrite(path, "Saving"))
        }
        "file.export_png" | "file.export_gif" | "file.export_apng" | "file.export_webp" | "file.export_svg" | "file.export_video" | "file.export_lottie" | "file.export_html" | "file.export_audio" => {
            let path = Path::new(p.get("path").and_then(Value::as_str)?);
            crate::vfs::exists(path).then(|| overwrite(path, "Exporting"))
        }
        "file.publish" => {
            // Publishing replaces what is already there; what this session published itself
            // it replaces without asking again (ADR 0060).
            let found = crate::publish::existing(s, &crate::publish::planned(s, p)?);
            let names: Vec<String> = found.iter().take(3).map(|f| f.display().to_string()).collect();
            let more = found.len().saturating_sub(names.len());
            (!found.is_empty()).then(|| Confirmation {
                title: if found.len() == 1 { "Replace published file?".into() } else { "Replace published files?".into() },
                message: format!(
                    "Publishing replaces {} already there: {}{}. This cannot be undone.",
                    if found.len() == 1 { "a file".to_string() } else { plural(found.len(), "file") },
                    names.join(", "),
                    if more > 0 { format!(" and {more} more") } else { String::new() }
                ),
                action: "Publish".into(),
                irreversible: true,
            })
        }
        "file.export_sprite_sheet" => {
            // The sheet and its frame map: either one already there is replaced.
            let path = Path::new(p.get("path").and_then(Value::as_str)?);
            let json = path.with_extension("json");
            [path, json.as_path()].into_iter().find(|f| crate::vfs::exists(f)).map(|f| overwrite(f, "Exporting"))
        }
        "file.export_png_sequence" => {
            let dir = Path::new(p.get("dir").and_then(Value::as_str)?);
            let has_frames = crate::vfs::list(dir, 10_000).iter().any(|e| {
                let n = e.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
                n.starts_with("frame_") && n.ends_with(".png")
            });
            has_frames.then(|| Confirmation {
                title: "Replace exported frames?".into(),
                message: format!("{} already contains exported frames. Matching files will be replaced; this cannot be undone.", dir.display()),
                action: "Replace".into(),
                irreversible: true,
            })
        }
        "sound.delete" => {
            // A sound in use comes off every keyframe that plays it.
            let id = anicraft_doc::SoundId(p.get("sound").and_then(Value::as_u64)?);
            let snd = s.doc.sound(id)?;
            let used: usize = s.doc.timelines().map(|tl| crate::cmds_sound::uses_in(tl, id)).sum();
            (used > 0).then(|| Confirmation {
                title: "Delete sound?".into(),
                message: format!("“{}” plays on {}. It comes off them and out of the library. You can undo this.", snd.name, plural(used, "keyframe")),
                action: "Delete Sound".into(),
                irreversible: false,
            })
        }
        "find.replace" => {
            // Replace All over many places asks first (ADR 0058); one undo step takes it back.
            let n = crate::find::replace_count(s, p)?;
            (n > crate::find::REPLACE_ASK).then(|| Confirmation {
                title: "Replace all?".into(),
                message: format!("This replaces what was found in {}. You can undo this in one step.", plural(n, "place")),
                action: "Replace All".into(),
                irreversible: false,
            })
        }
        "library.delete" => {
            // Routine when nothing deleted is in use and no folder holds anything (undo is there);
            // asks when instances go with it or a folder takes its contents (ADR 0050).
            let list = crate::cmds_library::entries_of(s, p, "items").ok()?;
            let (items, folders) = crate::cmds_library::deletion(s, &list);
            let uses = s.doc.library_uses();
            let used: Vec<&anicraft_doc::LibraryItem> = items.iter().filter(|i| uses.get(i).is_some_and(|n| *n > 0)).collect();
            let n_uses: usize = used.iter().filter_map(|i| uses.get(*i)).sum();
            let inside = items.len() + folders.len() - list.len();
            if used.is_empty() && inside == 0 {
                return None;
            }
            let what = match list.as_slice() {
                [one] => format!("“{}”", s.doc.entry_name(*one).unwrap_or("")),
                _ => plural(list.len(), "item"),
            };
            let mut parts = Vec::new();
            if inside > 0 {
                parts.push(format!("{what} {} {}, deleted with {}.", if list.len() == 1 { "holds" } else { "hold" }, plural(inside, "item"), if list.len() == 1 { "it" } else { "them" }));
            }
            if !used.is_empty() {
                parts.push(format!("{} used {} in the movie; {} go too.", if used.len() == 1 { "One of them is".to_string() } else { format!("{} of them are", used.len()) }, plural(n_uses, "time"), if n_uses == 1 { "that use" } else { "those uses" }));
            }
            parts.push("You can undo this.".into());
            Some(Confirmation {
                title: if inside > 0 && list.len() == 1 { "Delete folder?".into() } else { "Delete from Library?".into() },
                message: parts.join(" "),
                action: "Delete".into(),
                irreversible: false,
            })
        }
        "library.delete_unused" => {
            let unused = s.doc.unused_items();
            let names: Vec<String> = unused.iter().take(5).filter_map(|i| s.doc.item_name(*i)).map(|n| format!("“{n}”")).collect();
            let more = unused.len().saturating_sub(names.len());
            (!unused.is_empty()).then(|| Confirmation {
                title: "Delete unused items?".into(),
                message: format!(
                    "Nothing in the movie uses {}: {}{}. They leave the Library. You can undo this.",
                    if unused.len() == 1 { "this item".to_string() } else { format!("these {} items", unused.len()) },
                    names.join(", "),
                    if more > 0 { format!(" and {more} more") } else { String::new() }
                ),
                action: "Delete Unused".into(),
                irreversible: false,
            })
        }
        "draw.erase_all" => {
            // Double-clicking the Eraser: everything on the stage at this frame (ADR 0034).
            let layers = crate::cmds_erase::erasable_layers(s);
            let elements: usize = layers.iter().map(|(_, n)| n).sum();
            (elements > 0).then(|| Confirmation {
                title: "Erase everything?".into(),
                message: format!("Everything on the stage at frame {} goes: {} on {}. Locked and hidden layers are kept. You can undo this.", s.frame + 1, plural(elements, "element"), plural(layers.len(), "layer")),
                action: "Erase Everything".into(),
                irreversible: false,
            })
        }
        "camera.remove" => camera_removal(s, s.timeline().camera_layer()?),
        "bone.remove" => {
            // The bone's moves in every pose go with it (ADR 0043); one pose loses nothing.
            let id = anicraft_doc::BoneId(p.get("bone").and_then(Value::as_u64)?);
            let l = s.timeline().layers.iter().find(|l| l.armature.as_ref().is_some_and(|a| a.bone(id).is_some()))?;
            let arm = l.armature.as_deref()?;
            let n = crate::cmds_bone::poses(l);
            let below = arm.subtree(id).len().saturating_sub(1);
            let name = arm.bone(id).map(|b| if b.name.is_empty() { "The bone".to_string() } else { format!("“{}”", b.name) })?;
            (n > 1).then(|| Confirmation {
                title: "Remove bone?".into(),
                message: format!(
                    "{name}{} {} posed over {}; removing takes {} out of every pose. The instances stay where they are now, on a layer of their own. You can undo this.",
                    if below > 0 { format!(" and the {} below it", plural(below, "bone")) } else { String::new() },
                    if below > 0 { "are" } else { "is" },
                    plural(n, "keyframe"),
                    if below > 0 { "them" } else { "it" },
                ),
                action: "Remove Bone".into(),
                irreversible: false,
            })
        }
        "layer.delete" => {
            let l = target_layer(s, p)?;
            if l.kind == anicraft_doc::LayerKind::Camera {
                return camera_removal(s, l);
            }
            if l.kind == anicraft_doc::LayerKind::Folder {
                // A folder goes with everything in it.
                let tl = s.timeline();
                let i = tl.layers.iter().position(|x| x.id == l.id)?;
                let inner = tl.layers.get(tl.group_range(i))?;
                let n = inner.len().saturating_sub(1);
                let (keys, elements) = inner.iter().map(|x| layer_content(x)).fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
                return (n > 0).then(|| Confirmation {
                    title: "Delete folder?".into(),
                    message: format!(
                        "Folder “{}” holds {}{}. They are deleted with it. You can undo this.",
                        l.name,
                        plural(n, "layer"),
                        if elements > 0 { format!(" with {} in {}", plural(elements, "element"), plural(keys, "keyframe")) } else { String::new() }
                    ),
                    action: "Delete Folder".into(),
                    irreversible: false,
                });
            }
            let (keys, elements) = layer_content(l);
            (elements > 0).then(|| Confirmation {
                title: "Delete layer?".into(),
                message: format!("Layer “{}” has {} with {}. You can undo this.", l.name, plural(keys, "keyframe"), plural(elements, "element")),
                action: "Delete Layer".into(),
                irreversible: false,
            })
        }
        "scene.delete" => {
            let i = crate::cmds_scene::target_scene(s, p).ok()?;
            let sc = s.doc.scenes.get(i)?;
            let (keys, elements) = sc.timeline.layers.iter().map(|l| layer_content(l)).fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
            (elements > 0 && s.doc.scenes.len() > 1).then(|| Confirmation {
                title: "Delete scene?".into(),
                message: format!("Scene “{}” has {} with {}. You can undo this.", sc.name, plural(keys, "keyframe"), plural(elements, "element")),
                action: "Delete Scene".into(),
                irreversible: false,
            })
        }
        "timeline.clear_keyframe" => {
            let (f, n) = (target_frame(s, p), target_count(p));
            let lost = lost_keys(s, p, |k| k.start >= f && k.start - f < n);
            let (keys, elements) = (lost.len(), lost.iter().map(|(_, e)| e).sum::<usize>());
            (elements > 0).then(|| Confirmation {
                title: if keys == 1 { "Clear keyframe?".into() } else { "Clear keyframes?".into() },
                message: match lost.as_slice() {
                    [(name, e)] if n == 1 => format!("The keyframe at frame {} on “{name}” holds {}. You can undo this.", f + 1, plural(*e, "element")),
                    _ => format!("The selected frames hold {} with {}. You can undo this.", plural(keys, "keyframe"), plural(elements, "element")),
                },
                action: if keys == 1 { "Clear Keyframe".into() } else { "Clear Keyframes".into() },
                irreversible: false,
            })
        }
        "timeline.remove_frame" => {
            // A keyframe whose whole span is removed takes its artwork with it.
            let (f, n) = (target_frame(s, p), target_count(p));
            let lost = lost_keys(s, p, |k| k.start >= f && k.end() <= f.saturating_add(n));
            let (keys, elements) = (lost.len(), lost.iter().map(|(_, e)| e).sum::<usize>());
            (elements > 0).then(|| Confirmation {
                title: if n == 1 { "Remove frame?".into() } else { "Remove frames?".into() },
                message: match lost.as_slice() {
                    [(name, e)] if n == 1 => format!("Frame {} is a one-frame keyframe on “{name}” with {}; removing it deletes that artwork. You can undo this.", f + 1, plural(*e, "element")),
                    _ => format!("Removing these frames deletes {} with {}. You can undo this.", plural(keys, "keyframe"), plural(elements, "element")),
                },
                action: if n == 1 { "Remove Frame".into() } else { "Remove Frames".into() },
                irreversible: false,
            })
        }
        "plugin.install" => {
            let path = Path::new(p.get("path").and_then(Value::as_str)?);
            let (m, dest) = s.plugins.install_overwrites(path)?;
            let old = s.plugins.get(&m.id).map(|o| o.manifest().version.clone()).filter(|v| !v.is_empty());
            Some(Confirmation {
                title: "Replace plug-in?".into(),
                message: format!(
                    "“{}”{} is already installed. Installing {} replaces its file in the plug-ins folder ({}); this cannot be undone.",
                    m.name,
                    old.map_or_else(String::new, |v| format!(" {v}")),
                    path.display(),
                    dest.display()
                ),
                action: "Replace Plug-in".into(),
                irreversible: true,
            })
        }
        "plugin.remove" => {
            let plugin = s.plugins.get(p.get("id").and_then(Value::as_str)?)?;
            let file = plugin.source().filter(|_| s.plugins.owns_file(&plugin))?;
            Some(Confirmation {
                title: "Remove plug-in?".into(),
                message: format!("“{}” is uninstalled and its file is deleted from the plug-ins folder ({}). This cannot be undone.", plugin.manifest().name, file.display()),
                action: "Remove Plug-in".into(),
                irreversible: true,
            })
        }
        _ => None,
    }
}
