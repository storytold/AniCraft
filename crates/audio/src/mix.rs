//! Mixing: sounds placed in time ([`Voice`]s) summed into stereo output. A pure function of its
//! inputs, so the exported WAV, the video's soundtrack and what the stage plays are the same
//! samples, and tests can check them exactly.

use crate::Pcm;
use std::sync::Arc;

/// How a voice relates to the timeline (Animate's sync modes reduce to these two once the
/// timeline is laid out; see `anicraft_doc::sound`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceKind {
    /// Starts when its keyframe is reached and plays on its own: playback that starts after
    /// its keyframe does not hear it.
    Event,
    /// Tied to the frames it spans: heard from wherever the playhead is.
    Stream,
}

/// One sound placed in time, in output frames. The sound starts at `at` and repeats (wrapping
/// to its first sample) until `end`.
#[derive(Clone, Debug)]
pub struct Voice {
    pub pcm: Arc<Pcm>,
    pub kind: VoiceKind,
    /// Output frame of the sound's first sample.
    pub at: u64,
    /// Output frame where it stops (exclusive).
    pub end: u64,
    /// Linear gain.
    pub gain: f32,
}

impl Voice {
    /// Output frames one play of `pcm` lasts at `rate` (rounded up).
    pub fn play_frames(pcm: &Pcm, rate: u32) -> u64 {
        let n = pcm.frames() as u128 * u128::from(rate);
        let d = u128::from(pcm.rate.max(1));
        u64::try_from(n.div_ceil(d)).unwrap_or(u64::MAX)
    }
}

/// What a mix includes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MixOptions {
    /// Where playback started. Event sounds that began before it stay silent (an event sound is
    /// heard when its keyframe is reached); stream sounds play from wherever the playhead is.
    pub origin: u64,
    /// Only stream sounds (scrubbing plays those, as in Animate).
    pub streams_only: bool,
}

/// Mixes output frames `start .. start + out.len() / 2` at `rate` into `out` (stereo,
/// interleaved; overwritten). Equal rates copy samples exactly (times the gain); others use
/// cubic Hermite interpolation (Catmull-Rom), positions computed from the voice start so they
/// never drift.
pub fn mix(voices: &[Voice], rate: u32, opts: MixOptions, start: u64, out: &mut [f32]) {
    out.fill(0.0);
    let stop = start.saturating_add((out.len() / 2) as u64);
    for v in voices {
        if (v.kind == VoiceKind::Event && (v.at < opts.origin || opts.streams_only)) || v.gain == 0.0 {
            continue;
        }
        let (a, b) = (v.at.max(start), v.end.min(stop));
        let len = v.pcm.frames();
        if a >= b || len == 0 {
            continue;
        }
        let Some(dst) = out.get_mut(((a - start) * 2) as usize..((b - start) * 2) as usize) else { continue };
        let g = v.gain;
        if v.pcm.rate == rate {
            let mut i = ((a - v.at) % len as u64) as usize;
            for [x, y] in dst.as_chunks_mut::<2>().0.iter_mut() {
                let (l, r) = v.pcm.frame(i);
                *x += l * g;
                *y += r * g;
                i += 1;
                if i == len {
                    i = 0;
                }
            }
        } else {
            let looping = v.end - v.at > Voice::play_frames(&v.pcm, rate);
            let step = f64::from(v.pcm.rate) / f64::from(rate.max(1));
            for (k, [x, y]) in dst.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                let t = (a - v.at + k as u64) as f64 * step;
                let (l, r) = interpolate(&v.pcm, t, looping);
                *x += l * g;
                *y += r * g;
            }
        }
    }
}

/// The sound at fractional frame `t`, from its four neighbours; outside the sound is silence
/// unless it loops.
fn interpolate(p: &Pcm, t: f64, looping: bool) -> (f32, f32) {
    let len = p.frames() as i64;
    let tf = t.floor();
    let x = (t - tf) as f32;
    let i = tf as i64;
    let get = |j: i64| {
        let j = if looping { j.rem_euclid(len.max(1)) } else { j };
        if (0..len).contains(&j) { p.frame(j as usize) } else { (0.0, 0.0) }
    };
    let (y0, y1, y2, y3) = (get(i - 1), get(i), get(i + 1), get(i + 2));
    (hermite(y0.0, y1.0, y2.0, y3.0, x), hermite(y0.1, y1.1, y2.1, y3.1, x))
}

fn hermite(y0: f32, y1: f32, y2: f32, y3: f32, x: f32) -> f32 {
    let c1 = 0.5 * (y2 - y0);
    let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
    let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
    ((c3 * x + c2) * x + c1) * x + y1
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn pcm(rate: u32, ch: u16, s: &[f32]) -> Arc<Pcm> {
        Arc::new(Pcm::new(rate, ch, s.to_vec()).unwrap())
    }

    fn voice(p: &Arc<Pcm>, kind: VoiceKind, at: u64, end: u64) -> Voice {
        Voice { pcm: p.clone(), kind, at, end, gain: 1.0 }
    }

    fn render(voices: &[Voice], rate: u32, opts: MixOptions, start: u64, n: usize) -> Vec<f32> {
        let mut out = vec![9.0; n * 2];
        mix(voices, rate, opts, start, &mut out);
        out
    }

    #[test]
    fn equal_rates_copy_samples_exactly_and_sum_voices() {
        let a = pcm(8000, 1, &[0.5, 0.25, -0.5]);
        let b = pcm(8000, 2, &[0.125, -0.125, 0.0625, -0.0625]);
        let v = [voice(&a, VoiceKind::Event, 2, 5), voice(&b, VoiceKind::Stream, 3, 5)];
        let out = render(&v, 8000, MixOptions::default(), 0, 6);
        #[rustfmt::skip]
        let want = [0.0, 0.0,  0.0, 0.0,  0.5, 0.5,  0.25 + 0.125, 0.25 - 0.125,  -0.5 + 0.0625, -0.5 - 0.0625,  0.0, 0.0];
        assert_eq!(out, want);
        // Any window of the same mix gives the same samples (block boundaries are invisible).
        let mut joined = render(&v, 8000, MixOptions::default(), 0, 4);
        joined.extend(render(&v, 8000, MixOptions::default(), 4, 2));
        assert_eq!(joined, want);
    }

    #[test]
    fn repeats_wrap_to_the_start_and_end_where_told() {
        let a = pcm(8000, 1, &[0.1, 0.2, 0.3]);
        let out = render(&[voice(&a, VoiceKind::Event, 1, 8)], 8000, MixOptions::default(), 0, 9);
        let left: Vec<f32> = out.iter().step_by(2).copied().collect();
        assert_eq!(left, vec![0.0, 0.1, 0.2, 0.3, 0.1, 0.2, 0.3, 0.1, 0.0]);
    }

    #[test]
    fn events_before_the_origin_are_silent_streams_resync() {
        let a = pcm(8000, 1, &[0.5; 10]);
        let v = [voice(&a, VoiceKind::Event, 0, 10), voice(&a, VoiceKind::Stream, 0, 10)];
        let from_two = MixOptions { origin: 2, streams_only: false };
        assert_eq!(render(&v, 8000, from_two, 2, 1), vec![0.5, 0.5], "only the stream is heard");
        assert_eq!(render(&v, 8000, MixOptions::default(), 2, 1), vec![1.0, 1.0], "both when played from 0");
        let scrub = MixOptions { origin: 0, streams_only: true };
        assert_eq!(render(&v, 8000, scrub, 2, 1), vec![0.5, 0.5]);
    }

    #[test]
    fn resampling_keeps_dc_and_hits_the_source_samples() {
        // A constant stays constant; at 2× the rate every other output is a source sample.
        let dc = pcm(22_050, 1, &[0.25; 64]);
        let n = Voice::play_frames(&dc, 44_100);
        assert_eq!(n, 128);
        let out = render(&[voice(&dc, VoiceKind::Event, 0, n)], 44_100, MixOptions::default(), 0, 120);
        assert!(out[4..236].iter().all(|s| (s - 0.25).abs() < 1e-6), "{:?}", &out[..16]);
        let ramp: Vec<f32> = (0..32).map(|i| i as f32 / 64.0).collect();
        let r = pcm(1000, 1, &ramp);
        let out = render(&[voice(&r, VoiceKind::Event, 0, 64)], 2000, MixOptions::default(), 0, 60);
        for k in 1..29 {
            assert_eq!(out[4 * k], ramp[k], "source sample {k}");
            // A straight line interpolates to the straight line.
            assert!((out[4 * k + 2] - (k as f32 + 0.5) / 64.0).abs() < 1e-6);
        }
    }

    #[test]
    fn gain_and_silent_voices() {
        let a = pcm(8000, 1, &[0.5]);
        let mut v = voice(&a, VoiceKind::Event, 0, 1);
        v.gain = 0.5;
        assert_eq!(render(&[v.clone()], 8000, MixOptions::default(), 0, 1), vec![0.25, 0.25]);
        v.gain = 0.0;
        assert_eq!(render(&[v], 8000, MixOptions::default(), 0, 1), vec![0.0, 0.0]);
        // Voices far in the future, or ending before the window, cost nothing and add nothing.
        let far = voice(&a, VoiceKind::Event, u64::MAX - 1, u64::MAX);
        assert_eq!(render(&[far], 8000, MixOptions::default(), 0, 2), vec![0.0; 4]);
    }
}
