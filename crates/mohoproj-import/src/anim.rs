//! Animated transform channels → AniCraft's property channels (ADR 0102): the stage-2
//! animation foundation. A Moho channel `{when[], val[], interp[]}` maps key by key onto a
//! [`anicraft_doc::Channel`]; frames are already 0-based (corpus: CharWalk keys 0..45,
//! follow-path 0..50, Lua `SetValue(v, 0)`), so `when` moves over untouched.
//!
//! Interpolation follows Moho's public scripting constants (plan/moho › interp-modes):
//! im 0 Linear, 1 Smooth, 3 Step, 9 Bézier (per-component `b[]` handles), 2/7/8 the ease
//! family (approximated as Smooth), 4/5/10/11 noisy/cycle/bounce/elastic (held, reported).
//! The exact Smooth formula, per-dimension `interp_x/y/z` and action poses are Moho-only
//! approximations or sidecar data, and the report says which.
//!
//! Unit conversion: Moho layers and camera live in inches, y up, origin at the canvas
//! centre; AniCraft channels live in stage pixels, y down. Each key converts on its own —
//! translation ·dpi and y-flipped, rotation sign-flipped, shear sign-flipped, scale and the
//! flips verbatim — which is exact because every one of those maps conjugates through the
//! importer's inch→pixel transform (`to_px · M · to_px⁻¹`).

use crate::channel::{self, Channel};
use crate::import::{Report, MOHO_DPI};
use crate::json::{self, get};
use anicraft_doc::{ChannelValue, Handle, Interp, Key, MAX_CHANNEL_FRAME};
use anicraft_geom::Vec2;
use serde_json::Value;

/// Moho's default camera distance (every corpus document stores it).
pub(crate) const CAMERA_Z_DEFAULT: f64 = 3.732051;

/// One parsed `interp` entry: the mode and its Bézier handles per component.
#[derive(Clone, Debug, Default)]
pub(crate) struct InterpEntry {
    im: i64,
    /// `(ao, ai, po, pi)` per component — out/in angles (radians) and out/in percentages.
    b: Vec<(f64, f64, f64, f64)>,
    hold: u32,
    interval: u32,
    stagger: bool,
}

impl InterpEntry {
    /// Reads one object of a channel's `interp[]` (missing entries are linear).
    pub(crate) fn parse(v: Option<&Value>) -> InterpEntry {
        let Some(v) = v else { return InterpEntry::default() };
        let im = json::integer(get(v, "im")).unwrap_or(1);
        let b = json::array(get(v, "b"))
            .map(|arr| {
                arr.iter()
                    .map(|h| {
                        (
                            json::num(get(h, "ao")).unwrap_or(0.0),
                            json::num(get(h, "ai")).unwrap_or(0.0),
                            json::num(get(h, "po")).unwrap_or(1.0 / 3.0),
                            json::num(get(h, "pi")).unwrap_or(1.0 / 3.0),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        InterpEntry {
            im,
            b,
            hold: json::integer(get(v, "h")).unwrap_or(0).clamp(0, 100_000) as u32,
            interval: json::integer(get(v, "in")).unwrap_or(1).clamp(1, 100) as u32,
            stagger: json::boolean(get(v, "s")).unwrap_or(false),
        }
    }

    /// The outgoing interpolation a key gets from this entry, plus hold and interval,
    /// counting everything AniCraft approximates or holds.
    fn convert(&self, report: &mut Report, components: usize) -> (Interp, u32, u32) {
        match self.im {
            0 => (Interp::Linear, self.hold, self.interval),
            1 => (Interp::Smooth, self.hold, self.interval),
            3 => (Interp::Hold, self.hold, self.interval),
            9 if !self.b.is_empty() => {
                let out = (0..components)
                    .map(|c| {
                        self.b.get(c).or(self.b.first()).map_or(Handle { percent: 1.0 / 3.0, angle: 0.0 }, |&(ao, _ai, po, _pi)| Handle {
                            percent: po.clamp(0.0, 1.0) as f32,
                            angle: ao.clamp(-1.396, 1.396) as f32,
                        })
                    })
                    .collect();
                (Interp::Bezier(out), self.hold, self.interval)
            }
            // A Bézier entry without handles: linear until a key carries them.
            9 => (Interp::Linear, self.hold, self.interval),
            // The ease family as smooth (Moho's exact ease curves are not public).
            2 | 7 | 8 => {
                report.skip("ease keys (drawn smooth)");
                (Interp::Smooth, self.hold, self.interval)
            }
            // Noisy, cycle, pose, bounce, elastic: held, and said so.
            other => {
                report.skip(&format!("interp mode {other} keys (held)"));
                (Interp::Hold, self.hold, self.interval)
            }
        }
    }

    /// The incoming handles this key contributes to the segment before it.
    fn in_handles(&self, components: usize) -> Vec<Handle> {
        (0..components)
            .map(|c| {
                self.b.get(c).or(self.b.first()).map_or(Handle { percent: 1.0 / 3.0, angle: 0.0 }, |&(_ao, ai, _po, pi)| Handle {
                    percent: pi.clamp(0.0, 1.0) as f32,
                    angle: ai.clamp(-1.396, 1.396) as f32,
                })
            })
            .collect()
    }
}

/// Reads the channel object `name` out of `tr` and converts it key by key into `out`
/// (`value(ch, i)` converting key `i`'s raw value; `components` = interpolatable
/// dimensions). A `mute: true` channel animates in the file but Moho plays none of it: it
/// imports frozen at its first key, and the report says so. Negative frames (the "always"
/// sentinel) clamp to 0; keys past [`MAX_CHANNEL_FRAME`] drop, counted. Returns the parsed
/// channel view when the result animates (more than one key), else `None`.
pub(crate) fn read_channel<'a, T: ChannelValue>(
    out: &mut anicraft_doc::Channel<T>,
    tr: &'a Value,
    name: &str,
    report: &mut Report,
    components: usize,
    mut value: impl FnMut(&Channel, usize) -> T,
) -> Option<Channel<'a>> {
    let v = get(tr, name)?;
    let ch = Channel::parse(v)?;
    if json::boolean(get(v, "mute")).unwrap_or(false) && ch.is_animated() {
        report.skip("muted channels (held at their first key)");
        out.keys = vec![Key::new(0, value(&ch, 0))];
        return None;
    }
    let interp = ch.interp().unwrap_or(&[]);
    let mut keys: Vec<Key<T>> = Vec::new();
    for i in 0..ch.len() {
        let Some(frame) = ch.frame(i) else { continue };
        if frame > MAX_CHANNEL_FRAME {
            report.skip("channel keys past frame 100,000");
            continue;
        }
        let entry = InterpEntry::parse(interp.get(i));
        if entry.stagger {
            report.skip("stagger keys (drawn without stagger)");
        }
        let (out_interp, hold, interval) = entry.convert(report, components);
        let mut key = Key::new(frame, value(&ch, i));
        key.out = out_interp;
        key.hold = hold;
        key.interval = interval;
        keys.push(key);
    }
    // The `in_` handles live on key i+1 and shape the segment i→i+1 when it is a Bézier.
    for i in 1..keys.len() {
        let entry = InterpEntry::parse(interp.get(i));
        if !entry.b.is_empty() && matches!(keys.get(i - 1), Some(k) if matches!(k.out, Interp::Bezier(_))) {
            keys[i].in_ = entry.in_handles(components);
        }
    }
    let animated = keys.len() > 1;
    out.keys = keys;
    animated.then_some(ch)
}

/// The layer's transform channels, stage-space, ready for [`anicraft_doc::Layer`].
pub(crate) struct LayerTransform {
    pub translation: anicraft_doc::Channel<Vec2>,
    pub scale: anicraft_doc::Channel<Vec2>,
    pub rotation: anicraft_doc::Channel<f32>,
    pub shear: anicraft_doc::Channel<Vec2>,
    pub flip_h: anicraft_doc::Channel<bool>,
    pub flip_v: anicraft_doc::Channel<bool>,
}

impl LayerTransform {
    /// Whether any of the six channels animates.
    pub fn is_animated(&self) -> bool {
        self.translation.is_animated()
            || self.scale.is_animated()
            || self.rotation.is_animated()
            || self.shear.is_animated()
            || self.flip_h.is_animated()
            || self.flip_v.is_animated()
    }
}

/// Reads a layer's `transforms` object into stage-space channels. Reports the 3D parts
/// (z translation/scale, x/y rotation) when they hold values.
pub(crate) fn layer_transform_channels(tr: &Value, report: &mut Report) -> LayerTransform {
    let dpi = MOHO_DPI as f64;

    let mut translation = anicraft_doc::Channel::default();
    {
        let mut zed = 0.0f64;
        read_channel(&mut translation, tr, "translation", report, 2, |ch, i| {
            let (x, y, z) = channel::sample_vec3_at(ch, i).unwrap_or((0.0, 0.0, 0.0));
            zed = zed.max((z as f64).abs());
            Vec2::new((dpi * x as f64) as f32, (-dpi * y as f64) as f32)
        });
        if zed > 1e-6 {
            report.skip("3D layer translation (z)");
        }
    }

    let mut scale = anicraft_doc::Channel::default();
    {
        let mut szed = 0.0f64;
        read_channel(&mut scale, tr, "scale", report, 2, |ch, i| {
            let (x, y, z) = channel::sample_vec3_at(ch, i).unwrap_or((1.0, 1.0, 1.0));
            szed = szed.max(((z - 1.0) as f64).abs());
            Vec2::new(x, y)
        });
        if szed > 1e-6 {
            report.skip("3D layer scale (z)");
        }
    }

    let mut rotation = anicraft_doc::Channel::default();
    read_channel(&mut rotation, tr, "rotation_z", report, 1, |ch, i| {
        -(channel::sample_f64_at(ch, i).unwrap_or(0.0)) as f32
    });

    let mut shear = anicraft_doc::Channel::default();
    // The y-flip flips both shear directions with it.
    read_channel(&mut shear, tr, "shear", report, 2, |ch, i| {
        let (x, y, _) = channel::sample_vec3_at(ch, i).unwrap_or((0.0, 0.0, 0.0));
        Vec2::new(-x, -y)
    });

    let mut flip_h = anicraft_doc::Channel::default();
    read_channel(&mut flip_h, tr, "flip_h", report, 0, |ch, i| channel::sample_bool_at(ch, i).unwrap_or(false));
    let mut flip_v = anicraft_doc::Channel::default();
    read_channel(&mut flip_v, tr, "flip_v", report, 0, |ch, i| channel::sample_bool_at(ch, i).unwrap_or(false));

    for name in ["rotation_x", "rotation_y"] {
        if let Some(c) = get(tr, name).and_then(Channel::parse)
            && (c.is_animated() || c.val().iter().any(|v| json::num(Some(v)).is_some_and(|v| v.abs() > 1e-6)))
        {
            report.skip("3D layer rotation (x/y)");
            break;
        }
    }
    LayerTransform { translation, scale, rotation, shear, flip_h, flip_v }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn interp_constants_map_like_moho() {
        let mut r = Report::default();
        let e = |im: i64| InterpEntry { im, ..InterpEntry::default() };
        assert_eq!(e(0).convert(&mut r, 1).0, Interp::Linear);
        assert_eq!(e(1).convert(&mut r, 1).0, Interp::Smooth);
        assert_eq!(e(3).convert(&mut r, 1).0, Interp::Hold);
        let (interp, _, _) = InterpEntry { im: 9, b: vec![(0.5, -0.25, 0.4, 0.6)], ..InterpEntry::default() }.convert(&mut r, 1);
        assert_eq!(interp, Interp::Bezier(vec![Handle { percent: 0.4, angle: 0.5 }]));
        assert_eq!(e(2).convert(&mut r, 1).0, Interp::Smooth);
        assert_eq!(e(5).convert(&mut r, 1).0, Interp::Hold);
        assert_eq!(r.skipped["ease keys (drawn smooth)"], 1);
        assert_eq!(r.skipped["interp mode 5 keys (held)"], 1);
    }

    #[test]
    fn vec3_channel_splits_into_per_component_handles() {
        let mut r = Report::default();
        let conv = |ch: &Channel, i: usize| {
            let (x, y, _) = channel::sample_vec3_at(ch, i).unwrap_or((0.0, 0.0, 0.0));
            Vec2::new(x, y)
        };
        let tr = json!({
            "translation": {"type": "Vec3", "when": [0, 12], "val": [{"x": 0.0, "y": 0.0}, {"x": 1.0, "y": 2.0}],
            "interp": [{"im": 9, "b": [{"ao": 0.0, "ai": 0.0, "po": 0.1, "pi": 0.2}, {"ao": 0.3, "ai": 0.4, "po": 0.5, "pi": 0.6}, {"ao": 9.0, "ai": 9.0, "po": 9.0, "pi": 9.0}]}, {"im": 0}]}
        });
        let mut out = anicraft_doc::Channel::<Vec2>::default();
        assert!(read_channel(&mut out, &tr, "translation", &mut r, 2, conv).is_some());
        assert_eq!(out.keys.len(), 2);
        // x takes b[0], y takes b[1] (the z entry is beyond the two components).
        assert_eq!(out.keys[0].out, Interp::Bezier(vec![Handle { percent: 0.1, angle: 0.0 }, Handle { percent: 0.5, angle: 0.3 }]));
        // Key 1 is linear, so segment 0 is linear: no in-handles stored.
        assert!(out.keys[1].in_.is_empty());
        // A Bézier on key 1 hands its ai/pi to key 0 as segment 0's in-handles.
        let tr = json!({
            "translation": {"type": "Vec3", "when": [0, 12], "val": [{"x": 0.0, "y": 0.0}, {"x": 1.0, "y": 2.0}],
            "interp": [{"im": 9, "b": [{"ao": 0.0, "ai": 0.0, "po": 0.1, "pi": 0.2}, {"ao": 0.3, "ai": 0.4, "po": 0.5, "pi": 0.6}, {"ao": 0.0, "ai": 0.0, "po": 0.0, "pi": 0.0}]}, {"im": 9, "b": [{"ao": 1.0, "ai": 2.0, "po": 0.3, "pi": 0.7}, {"ao": 1.0, "ai": 2.0, "po": 0.3, "pi": 0.7}, {"ao": 0.0, "ai": 0.0, "po": 0.0, "pi": 0.0}]}]}
        });
        let mut out = anicraft_doc::Channel::<Vec2>::default();
        read_channel(&mut out, &tr, "translation", &mut r, 2, conv);
        // ai 2.0 rad exceeds the ±80° bound and clamps.
        assert_eq!(out.keys[1].in_, vec![Handle { percent: 0.7, angle: 1.396 }, Handle { percent: 0.7, angle: 1.396 }]);
    }

    #[test]
    fn negative_frames_clamp_and_muted_freezes() {
        let mut r = Report::default();
        let conv = |ch: &Channel, i: usize| channel::sample_f64_at(ch, i).unwrap_or(0.0) as f32;
        let mut out = anicraft_doc::Channel::<f32>::default();
        read_channel(&mut out, &json!({"rotation_z": {"type": "Val", "when": [-1000000, 5], "val": [1.0, 2.0]}}), "rotation_z", &mut r, 1, conv);
        assert_eq!(out.keys.iter().map(|k| k.frame).collect::<Vec<_>>(), vec![0, 5]);
        assert_eq!(out.keys[0].val, 1.0);

        let mut out = anicraft_doc::Channel::<f32>::default();
        let tr = json!({"rotation_z": {"type": "Val", "mute": true, "when": [0, 5], "val": [1.0, 2.0]}});
        let animated = read_channel(&mut out, &tr, "rotation_z", &mut r, 1, conv);
        assert!(animated.is_none());
        assert_eq!(out.keys.len(), 1);
        assert_eq!(out.keys[0].val, 1.0);
        assert_eq!(r.skipped["muted channels (held at their first key)"], 1);
    }

    #[test]
    fn layer_transform_reads_and_converts() {
        // Moho writes radians as decimals; the corpus stores half and full turns like these.
        let (half, full) = (std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
        let mut r = Report::default();
        let tr = json!({
            "translation": {"type": "Vec3", "when": [0, 10], "val": [{"x": 0.5, "y": -0.25, "z": 0.0}, {"x": 1.5, "y": 0.75, "z": 0.0}]},
            "scale": {"type": "Vec3", "when": [0], "val": [{"x": 2.0, "y": 1.0, "z": 1.0}]},
            "rotation_z": {"type": "Val", "when": [0, 10], "val": [half, full]},
            "flip_h": {"type": "Bool", "when": [0], "val": [true]}
        });
        let lt = layer_transform_channels(&tr, &mut r);
        assert!(lt.is_animated());
        // Inches → pixels: 0.5 in × 320 = 160 px, y flips.
        assert_eq!(lt.translation.keys[0].val, Vec2::new(160.0, 80.0));
        assert_eq!(lt.translation.keys[1].val, Vec2::new(480.0, -240.0));
        assert_eq!(lt.scale.keys[0].val, Vec2::new(2.0, 1.0));
        // Rotation flips sign (y-up CCW → screen clockwise).
        assert!((lt.rotation.keys[0].val + std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert!((lt.rotation.keys[1].val + std::f32::consts::PI).abs() < 1e-5);
        assert!(lt.flip_h.keys[0].val);
        assert!(lt.flip_v.is_empty());
        assert!(r.skipped.is_empty(), "{:?}", r.skipped);
    }
}
