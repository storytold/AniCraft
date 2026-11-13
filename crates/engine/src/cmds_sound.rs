//! Sound commands (ADR 0020, ADR 0039): import sounds into the library, put them on keyframes,
//! set their sync, repeats and effect envelopes, set layer volumes and mutes, delete sounds,
//! and mix the movie's soundtrack to a WAV file.

use crate::cmds_timeline::{keyframe_mut, target};
use crate::params::*;
use crate::{CommandSpec, EngineError, MAX_FRAME, Result, Session};
use anicraft_doc::audio::{Voice, mix, wav};
use anicraft_doc::sound::{self, MAX_ENVELOPE_POINTS, MAX_ENVELOPE_SECONDS, MAX_REPEAT, clean_points, frame_start};
use anicraft_doc::{EnvelopePoint, FrameSound, LayerKind, Sound, SoundEffect, SoundId, SoundSync, Timeline};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c(
            "file.import_to_library",
            "Import to Library…",
            "File/Import",
            None,
            "{path (.wav, .ogg or .mp3 sound; .png or .jpg image; .svg drawing: a graphic symbol named after the file holding its shapes), name?}",
            true,
            import_to_library as crate::RunFn,
        ),
        c(
            "file.export_audio",
            "Export Audio…",
            "File/Export",
            None,
            "{path (.wav), from?, to? (movie frames), rate? (Hz, default 44100), bits? (16, 24 or 32 = float; default 16), confirm?}",
            false,
            export_audio,
        ),
        c(
            "sound.place",
            "Place Sound",
            "",
            None,
            "{sound, layer?, frame? (a blank keyframe is made there if the frame is empty), sync?: event|start|stop|stream, repeat?, loop?, effect?, envelope?, extend? (lengthen the layer's last span to the sound's length)}",
            true,
            place,
        ),
        c(
            "sound.set",
            "Sound Settings",
            "",
            None,
            "{layer?, frame?, sound?: id | null (null takes the sound off), sync?, repeat?, loop?, effect?: none|left_channel|right_channel|fade_left_to_right|fade_right_to_left|fade_in|fade_out|custom (presets shape over the time the sound is heard; custom starts from the effect before), envelope?: [{at (seconds from the sound's start), left (0–1), right (0–1)}…] (1–16 points; makes the effect custom)}",
            true,
            set,
        ),
        c("layer.toggle_mute", "Mute/Unmute Layer", "", None, "{layer?} (its sounds, and those in the symbols on it, go silent in playback and every export)", true, toggle_mute),
        c("layer.set_volume", "Layer Volume", "", None, "{volume? (0–1: the layer's sounds and those in the symbols on it), muted?, layer?}", true, set_volume),
        c("sound.remove", "Remove Sound", "", None, "{layer?, frame?}", true, |s, p| set(s, &with_null_sound(p))),
        c("sound.delete", "Delete Sound", "", None, "{sound} (from the library and every keyframe that uses it)", true, delete),
        c("sound.rename", "Rename Sound", "", None, "{sound, name}", true, rename),
    ]
}

fn with_null_sound(p: &Value) -> Value {
    let mut q = match p {
        Value::Object(m) => m.clone(),
        _ => serde_json::Map::new(),
    };
    q.insert("sound".into(), Value::Null);
    Value::Object(q)
}

/// Largest sound file read.
const MAX_SOUND_BYTES: u64 = 256 * 1024 * 1024;

/// Whether `path` names a sound file AniCraft reads.
pub fn is_sound(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    matches!(ext.as_deref(), Some("wav" | "wave" | "ogg" | "oga" | "mp3"))
}

fn file_name(path: &Path, fallback: &str) -> String {
    path.file_name().map_or_else(|| fallback.into(), |n| n.to_string_lossy().chars().take(128).collect())
}

/// Reads a sound file into the library; its id.
pub(crate) fn add_sound(s: &mut Session, path: &Path, name: Option<&str>) -> Result<SoundId> {
    if crate::vfs::len(path)? > MAX_SOUND_BYTES {
        return Err(EngineError::Format(format!("{} is too large to import", path.display())));
    }
    let bytes = crate::vfs::read(path)?;
    let id = SoundId(s.doc.alloc_id());
    let name = name.map_or_else(|| file_name(path, "sound"), |n| n.chars().take(128).collect());
    let sound = Sound::from_data(id, name, bytes).map_err(|e| EngineError::Format(format!("{}: {e}", path.display())))?;
    s.doc.sounds.push(Arc::new(sound));
    Ok(id)
}

fn sound_json(s: &Sound) -> Value {
    json!({"sound": s.id.0, "name": s.name, "format": s.format.label(), "rate": s.rate, "channels": s.channels, "seconds": s.seconds()})
}

/// Import to Library: a sound (or an image) goes into the library without being placed.
pub(crate) fn import_to_library(s: &mut Session, p: &Value) -> Result<Value> {
    let path = PathBuf::from(str_req(p, "path")?);
    if is_sound(&path) {
        let id = add_sound(s, &path, opt_str(p, "name")?)?;
        let snd = s.doc.sound(id).ok_or_else(|| EngineError::State("the sound vanished".into()))?;
        return Ok(sound_json(snd));
    }
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    if matches!(ext.as_deref(), Some("png" | "jpg" | "jpeg")) {
        let bytes = crate::vfs::read(&path)?;
        let id = anicraft_doc::BitmapId(s.doc.alloc_id());
        let bitmap = anicraft_doc::Bitmap::from_image(id, file_name(&path, "image"), bytes).map_err(|e| EngineError::Format(format!("{}: {e}", path.display())))?;
        let (w, h) = (bitmap.width, bitmap.height);
        s.doc.bitmaps.push(Arc::new(bitmap));
        return Ok(json!({"bitmap": id.0, "width": w, "height": h}));
    }
    if ext.as_deref() == Some("svg") {
        // A drawing becomes a graphic symbol holding its shapes (ADR 0065).
        return crate::cmds_file::import_svg_symbol(s, &path, opt_str(p, "name")?);
    }
    Err(EngineError::Param("Import to Library reads WAV, Ogg Vorbis and MP3 sounds, PNG and JPEG images, and SVG drawings (as a graphic symbol)".into()))
}

/// Import to Stage with a sound: into the library, and onto a new layer named after the file,
/// at the playhead, its span as long as the sound (so its waveform shows in full).
pub(crate) fn import_to_stage(s: &mut Session, path: &Path) -> Result<Value> {
    let id = add_sound(s, path, None)?;
    let name: String = path.file_stem().map_or_else(|| "Sound".into(), |n| n.to_string_lossy().chars().take(128).collect());
    crate::cmds_timeline::new_layer(s, &json!({"name": name}))?;
    let v = place(s, &json!({"sound": id.0, "frame": s.frame, "extend": true}))?;
    let snd = s.doc.sound(id).ok_or_else(|| EngineError::State("the sound vanished".into()))?;
    let mut out = sound_json(snd);
    out["layer"] = v["layer"].clone();
    out["frames"] = v["frames"].clone();
    Ok(out)
}

/// Custom envelope points from `[{at, left, right}…]` (or `[[at, left, right]…]`): seconds
/// from the sound's start, volumes `0..=1`.
fn envelope_param(v: &Value) -> Result<Vec<EnvelopePoint>> {
    let bad = || EngineError::Param(format!("param 'envelope' must be 1 to {MAX_ENVELOPE_POINTS} points [{{at (seconds, 0–{MAX_ENVELOPE_SECONDS}), left (0–1), right (0–1)}}…]"));
    let list = v.as_array().filter(|a| (1..=MAX_ENVELOPE_POINTS).contains(&a.len())).ok_or_else(bad)?;
    let mut out = Vec::with_capacity(list.len());
    for item in list {
        let field = |k: &str, i: usize| item.get(k).or_else(|| item.get(i)).and_then(Value::as_f64).filter(|x| x.is_finite());
        let (Some(at), Some(left), Some(right)) = (field("at", 0), field("left", 1), field("right", 2)) else { return Err(bad()) };
        if !(0.0..=MAX_ENVELOPE_SECONDS).contains(&at) || !(0.0..=1.0).contains(&left) || !(0.0..=1.0).contains(&right) {
            return Err(bad());
        }
        out.push(EnvelopePoint::new(at, left as f32, right as f32));
    }
    Ok(clean_points(&out))
}

/// `sync`, `repeat`, `loop`, `effect` and `envelope` from params over `fs`. `heard` is how
/// long the sound would be heard, in seconds (Custom starts from the effect's shape over it).
fn apply_settings(p: &Value, fs: FrameSound, heard: &dyn Fn(&FrameSound) -> f64) -> Result<FrameSound> {
    let mut fs = apply_timing(p, fs)?;
    if let Some(e) = opt_str(p, "effect")? {
        let want = SoundEffect::from_key(e).ok_or_else(|| EngineError::Param("param 'effect' must be none, left_channel, right_channel, fade_left_to_right, fade_right_to_left, fade_in, fade_out or custom".into()))?;
        fs.effect = match want {
            // Custom keeps the points it has, or starts from the effect chosen before (full
            // volume: a point at each end, to drag).
            SoundEffect::Custom(_) if matches!(fs.effect, SoundEffect::Custom(_)) => fs.effect,
            SoundEffect::Custom(_) => {
                let t = heard(&fs);
                let pts = if fs.effect.is_none() { vec![EnvelopePoint::new(0.0, 1.0, 1.0), EnvelopePoint::new(t, 1.0, 1.0)] } else { fs.effect.points(t) };
                SoundEffect::Custom(clean_points(&pts))
            }
            preset => preset,
        };
    }
    if let Some(v) = p.get("envelope") {
        fs.effect = SoundEffect::Custom(envelope_param(v)?);
    }
    Ok(fs)
}

/// `sync`, `repeat` and `loop` from params over `fs`.
fn apply_timing(p: &Value, mut fs: FrameSound) -> Result<FrameSound> {
    if let Some(x) = opt_str(p, "sync")? {
        fs.sync = SoundSync::from_key(x).ok_or_else(|| EngineError::Param("param 'sync' must be event, start, stop or stream".into()))?;
    }
    if let Some(n) = opt_u32(p, "repeat")? {
        if !(1..=MAX_REPEAT).contains(&n) {
            return Err(EngineError::Param(format!("param 'repeat' must be between 1 and {MAX_REPEAT}")));
        }
        // Choosing a count is choosing Repeat over Loop (one menu in Properties).
        fs.repeat = n;
        fs.looping = false;
    }
    if let Some(b) = opt_bool(p, "loop")? {
        fs.looping = b;
    }
    Ok(fs)
}

fn sound_param(s: &Session, p: &Value) -> Result<SoundId> {
    let id = SoundId(opt_u64(p, "sound")?.ok_or_else(|| EngineError::Param("param 'sound' is required (a sound id from the library)".into()))?);
    s.doc.sound(id).map(|_| id).ok_or_else(|| EngineError::State(format!("no sound with id {} in the library", id.0)))
}

/// Place Sound: the sound goes on the keyframe covering the frame (a blank keyframe is made
/// on an empty frame), Animate-style. With `extend`, the layer's last span grows to the sound's
/// length so its waveform shows in full.
pub(crate) fn place(s: &mut Session, p: &Value) -> Result<Value> {
    let id = sound_param(s, p)?;
    let fps = s.doc.stage.fps;
    let want = s.doc.sound(id).map_or(1, |x| x.movie_frames(fps));
    let snd = s.doc.sound(id).cloned();
    let fs = apply_settings(p, FrameSound::new(id), &|fs| snd.as_ref().map_or(0.0, |x| fs.heard_seconds(x, want, want, fps)))?;
    let (lid, frame) = target(s, p)?;
    let extend = opt_bool(p, "extend")?.unwrap_or(false);
    let layer = s.layer_mut(lid)?;
    if layer.kind == LayerKind::Folder {
        return Err(EngineError::State(format!("'{}' is a folder; sounds go on a layer's keyframes", layer.name)));
    }
    if layer.keyframe_index_at(frame).is_none() {
        crate::cmds_timeline::insert_keyframe_at(layer, frame, false)?;
    }
    let i = layer.keyframe_index_at(frame).ok_or_else(|| EngineError::State("no keyframe there".into()))?;
    let last = i + 1 == layer.keyframes.len();
    let layer_id = layer.id;
    let k = layer.keyframe_mut(i).ok_or_else(|| EngineError::State("no keyframe there".into()))?;
    if extend && last && k.duration < want {
        k.duration = want.min(MAX_FRAME.saturating_sub(k.start).saturating_add(1)).max(1);
    }
    let out = json!({"layer": layer_id.0, "keyframe": k.start, "duration": k.duration, "sound": id.0, "sync": fs.sync.key(), "effect": fs.effect.key(), "frames": want});
    k.sound = Some(fs);
    Ok(out)
}

/// Sound Settings: changes the keyframe's sound (keeping its settings), its sync, repeats or
/// loop; `sound: null` takes it off.
fn set(s: &mut Session, p: &Value) -> Result<Value> {
    let new_id = match p.get("sound") {
        Some(Value::Null) => None,
        None => Some(None),
        Some(_) => Some(Some(sound_param(s, p)?)),
    };
    // The keyframe's span and the frames from it to the movie's end: the time a preset is
    // shaped over, which Custom starts from.
    let (lid, frame) = target(s, p)?;
    let fps = s.doc.stage.fps;
    let tl = s.timeline();
    let place = tl.layer(lid.unwrap_or(s.layer)).and_then(|l| l.keyframe_at(frame)).map(|k| (k.duration, s.doc.frame_count().saturating_sub(s.doc.scene_start(s.scene).saturating_add(k.start))));
    let sounds = s.doc.sounds.clone();
    let heard = move |fs: &FrameSound| match (sounds.iter().find(|x| x.id == fs.sound), place) {
        (Some(x), Some((span, rest))) => fs.heard_seconds(x, span, rest, fps),
        _ => 0.0,
    };
    let k = keyframe_mut(s, p)?;
    let Some(change) = new_id else {
        let had = k.sound.take().is_some();
        return Ok(json!({"keyframe": k.start, "removed": had}));
    };
    let base = match (change, k.sound.clone()) {
        (Some(id), Some(cur)) => FrameSound { sound: id, ..cur },
        (Some(id), None) => FrameSound::new(id),
        (None, Some(cur)) => cur,
        (None, None) => return Err(EngineError::State(format!("the keyframe at frame {} has no sound; pass \"sound\"", k.start + 1))),
    };
    let fs = apply_settings(p, base, &heard)?;
    let out = json!({"keyframe": k.start, "sound": fs.sound.0, "sync": fs.sync.key(), "repeat": fs.repeat, "loop": fs.looping, "effect": fs.effect.key(), "envelope": envelope_json(&fs.effect)});
    k.sound = Some(fs);
    Ok(out)
}

/// A custom effect's points as `[{at, left, right}…]` (`null` for the presets).
fn envelope_json(e: &SoundEffect) -> Value {
    match e {
        SoundEffect::Custom(p) => Value::Array(p.iter().map(|p| json!({"at": p.at, "left": p.left, "right": p.right})).collect()),
        _ => Value::Null,
    }
}

/// Mute or unmute a layer's sounds (and those in the symbols on it).
fn toggle_mute(s: &mut Session, p: &Value) -> Result<Value> {
    let layer = s.layer_mut(opt_u64(p, "layer")?.map(anicraft_doc::LayerId))?;
    layer.muted = !layer.muted;
    Ok(json!({"layer": layer.id.0, "muted": layer.muted, "volume": layer.volume}))
}

/// Layer Volume: the volume of a layer's sounds, and whether they are muted.
fn set_volume(s: &mut Session, p: &Value) -> Result<Value> {
    let volume = opt_f32(p, "volume")?;
    if volume.is_some_and(|v| !(0.0..=1.0).contains(&v)) {
        return Err(EngineError::Param("param 'volume' must be between 0 and 1".into()));
    }
    let muted = opt_bool(p, "muted")?;
    if volume.is_none() && muted.is_none() {
        return Err(EngineError::Param("pass 'volume' (0–1), 'muted' or both".into()));
    }
    let layer = s.layer_mut(opt_u64(p, "layer")?.map(anicraft_doc::LayerId))?;
    if layer.kind == LayerKind::Folder {
        return Err(EngineError::State(format!("'{}' is a folder; set the volume of the layers in it", layer.name)));
    }
    if let Some(v) = volume {
        layer.volume = v;
    }
    if let Some(m) = muted {
        layer.muted = m;
    }
    Ok(json!({"layer": layer.id.0, "volume": layer.volume, "muted": layer.muted}))
}

/// Keyframes in `tl` that use sound `id`.
pub fn uses_in(tl: &Timeline, id: SoundId) -> usize {
    tl.layers.iter().map(|l| l.keyframes.iter().filter(|k| k.sound.as_ref().is_some_and(|f| f.sound == id)).count()).sum()
}

/// Takes sound `id` off every keyframe of `tl`, touching (un-sharing) only those keyframes.
fn strip(tl: &mut Timeline, id: SoundId) {
    for l in tl.layers.iter_mut().filter(|l| l.keyframes.iter().any(|k| k.sound.as_ref().is_some_and(|f| f.sound == id))) {
        let l = Arc::make_mut(l);
        for k in l.keyframes.iter_mut().filter(|k| k.sound.as_ref().is_some_and(|f| f.sound == id)) {
            Arc::make_mut(k).sound = None;
        }
    }
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    let id = sound_param(s, p)?;
    let mut used = 0;
    for scene in &mut s.doc.scenes {
        let n = uses_in(&scene.timeline, id);
        if n > 0 {
            used += n;
            strip(&mut Arc::make_mut(scene).timeline, id);
        }
    }
    for sym in &mut s.doc.library {
        if uses_in(&sym.timeline, id) > 0 {
            strip(&mut Arc::make_mut(sym).timeline, id);
        }
    }
    s.doc.sounds.retain(|x| x.id != id);
    Ok(json!({"deleted": id.0, "keyframes": used}))
}

fn rename(s: &mut Session, p: &Value) -> Result<Value> {
    let id = sound_param(s, p)?;
    let name: String = str_req(p, "name")?.trim().chars().take(128).collect();
    if name.is_empty() {
        return Err(EngineError::Param("a sound needs a name".into()));
    }
    if let Some(snd) = s.doc.sounds.iter_mut().find(|x| x.id == id) {
        Arc::make_mut(snd).name = name.clone();
    }
    Ok(json!({"sound": id.0, "name": name}))
}

/// Mix a stretch of output frames to a WAV file in blocks (memory stays bounded however long
/// the movie is). Written beside the target, then renamed. Returns the bytes written.
pub(crate) fn write_mix(path: &Path, voices: &[Voice], rate: u32, from: u64, to: u64, bits: wav::Bits) -> Result<u64> {
    use std::io::Write;
    let frames = to.saturating_sub(from);
    let header = wav::header(rate, 2, bits, frames).ok_or_else(|| EngineError::Param("that much audio does not fit in a WAV file (4 GB); lower 'rate' or the frame range".into()))?;
    let tmp = path.with_extension("wav.tmp");
    let result = (|| -> Result<u64> {
        let mut f = crate::vfs::Writer::create(&tmp)?;
        f.write_all(&header)?;
        const BLOCK: u64 = 65_536;
        let mut buf = vec![0f32; BLOCK as usize * 2];
        let mut bytes = Vec::new();
        let mut at = from;
        while at < to {
            let n = (to - at).min(BLOCK) as usize;
            let out = buf.get_mut(..n * 2).unwrap_or_default();
            mix(voices, rate, anicraft_doc::audio::MixOptions::default(), at, out);
            bytes.clear();
            wav::append(&mut bytes, out, bits);
            f.write_all(&bytes)?;
            at += n as u64;
        }
        f.finish()?;
        Ok(header.len() as u64 + frames * 2 * u64::from(match bits {
            wav::Bits::Int16 => 2u8,
            wav::Bits::Int24 => 3,
            wav::Bits::Float32 => 4,
        }))
    })();
    match result {
        Ok(n) => {
            crate::vfs::rename(&tmp, path)?;
            Ok(n)
        }
        Err(e) => {
            let _ = crate::vfs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Output frames `from..to` of movie frames `first..=last` at `rate`.
pub(crate) fn movie_span(s: &Session, first: u32, last: u32, rate: u32) -> (u64, u64) {
    let fps = s.doc.stage.fps;
    (frame_start(u64::from(first), fps, rate), frame_start(u64::from(last) + 1, fps, rate))
}

/// The voices of movie frames `first..=last` as heard playing from `first` (event sounds that
/// began earlier are left out; stream sounds join mid-way), and their output frame span.
pub(crate) fn soundtrack(s: &Session, first: u32, last: u32, rate: u32) -> (Vec<Voice>, u64, u64) {
    let mut voices = sound::voices(&s.doc, rate);
    let (a, b) = movie_span(s, first, last, rate);
    voices.retain(|v| (v.kind == anicraft_doc::audio::VoiceKind::Stream || v.at >= a) && v.at < b && v.end > a);
    (voices, a, b)
}

/// A soundtrack WAV in the temp folder (for ffmpeg), removed when dropped.
pub(crate) struct TempTrack(pub PathBuf);

impl Drop for TempTrack {
    fn drop(&mut self) {
        let _ = crate::vfs::remove_file(&self.0);
    }
}

/// The soundtrack of movie frames `first..=last` as a 48 kHz 16-bit WAV in the temp folder.
pub(crate) fn temp_track(s: &Session, first: u32, last: u32) -> Result<TempTrack> {
    const RATE: u32 = 48_000;
    let nanos = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let track = TempTrack(std::env::temp_dir().join(format!("anicraft-track-{}-{nanos}.wav", std::process::id())));
    let (voices, a, b) = soundtrack(s, first, last, RATE);
    write_mix(&track.0, &voices, RATE, a, b, wav::Bits::Int16)?;
    Ok(track)
}

/// Export Audio: the movie's soundtrack, as heard playing it from the first frame of the range
/// (event sounds that started earlier are not included), as a stereo WAV.
fn export_audio(s: &mut Session, p: &Value) -> Result<Value> {
    let path = PathBuf::from(str_req(p, "path")?);
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    if !matches!(ext.as_deref(), Some("wav" | "wave")) {
        return Err(EngineError::Param("audio goes to a .wav file".into()));
    }
    let rate = opt_u32(p, "rate")?.unwrap_or(44_100);
    if !(8_000..=192_000).contains(&rate) {
        return Err(EngineError::Param("param 'rate' must be between 8000 and 192000".into()));
    }
    let bits = match opt_u32(p, "bits")?.unwrap_or(16) {
        16 => wav::Bits::Int16,
        24 => wav::Bits::Int24,
        32 => wav::Bits::Float32,
        _ => return Err(EngineError::Param("param 'bits' must be 16, 24 or 32 (float)".into())),
    };
    let last = s.doc.frame_count().saturating_sub(1);
    let first = opt_u32(p, "from")?.unwrap_or(0).min(last);
    let to = opt_u32(p, "to")?.unwrap_or(last).min(last);
    if to < first {
        return Err(EngineError::Param("'to' must not be before 'from'".into()));
    }
    let (voices, a, b) = soundtrack(s, first, to, rate);
    let bytes = write_mix(&path, &voices, rate, a, b, bits)?;
    Ok(json!({"written": path.display().to_string(), "bytes": bytes, "rate": rate, "seconds": (b - a) as f64 / f64::from(rate), "frames": to - first + 1, "sounds": voices.len()}))
}

#[cfg(test)]
pub(crate) fn test_wav(rate: u32, samples: &[f32]) -> Vec<u8> {
    wav::encode(rate, 1, samples, wav::Bits::Int16)
}
