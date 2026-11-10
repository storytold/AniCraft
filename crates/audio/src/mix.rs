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
    /// Output frame where it is first heard.
    pub at: u64,
    /// Output frame where it stops (exclusive).
    pub end: u64,
    /// Linear gain (both channels).
    pub gain: f32,
    /// Output frames of the sound already gone by at `at`: it is heard from that far in (a
    /// stream joined mid-way, inside a symbol that starts in the middle of it).
    pub skip: u64,
    /// Volume over the sound's time, per channel; `None` is full volume.
    pub envelope: Option<Arc<Envelope>>,
}

/// One point of an [`Envelope`]: a position in output frames from the sound's start (repeats
/// included) and the gain of each channel there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvPoint {
    pub at: u64,
    pub left: f32,
    pub right: f32,
}

/// A volume envelope: straight lines between its points, level before the first and after the
/// last. Never empty.
#[derive(Clone, Debug, PartialEq)]
pub struct Envelope {
    points: Vec<EnvPoint>,
}

impl Envelope {
    /// Sorts the points by position and keeps gains in `0..=1` (non-finite gains are silence).
    /// `None` without points.
    pub fn new(mut points: Vec<EnvPoint>) -> Option<Envelope> {
        if points.is_empty() {
            return None;
        }
        let clean = |g: f32| if g.is_finite() { g.clamp(0.0, 1.0) } else { 0.0 };
        for p in &mut points {
            p.left = clean(p.left);
            p.right = clean(p.right);
        }
        points.sort_by_key(|p| p.at);
        Some(Envelope { points })
    }

    pub fn points(&self) -> &[EnvPoint] {
        &self.points
    }

    /// The gains (left, right) at output frame `t` of the sound's time.
    pub fn gain(&self, t: u64) -> (f32, f32) {
        self.between(self.points.partition_point(|p| p.at <= t), t)
    }

    /// Gains at `t`, where `i` is the index of the first point after `t`.
    #[inline]
    fn between(&self, i: usize, t: u64) -> (f32, f32) {
        match (i.checked_sub(1).and_then(|j| self.points.get(j)), self.points.get(i)) {
            (Some(a), Some(b)) if b.at > a.at => {
                let x = (t - a.at) as f64 / (b.at - a.at) as f64;
                (lerp(a.left, b.left, x), lerp(a.right, b.right, x))
            }
            (Some(a), _) => (a.left, a.right),
            (None, Some(b)) => (b.left, b.right),
            (None, None) => (1.0, 1.0),
        }
    }

    /// Gains for output frames `from, from + 1, …` of the sound's time, into `out`.
    fn fill(&self, from: u64, out: &mut [(f32, f32)]) {
        let mut i = self.points.partition_point(|p| p.at <= from);
        for (k, g) in out.iter_mut().enumerate() {
            let t = from.saturating_add(k as u64);
            while self.points.get(i).is_some_and(|p| p.at <= t) {
                i += 1;
            }
            *g = self.between(i, t);
        }
    }
}

/// `a + (b - a) · x` in double precision: the gain between two envelope points.
#[inline]
pub fn lerp(a: f32, b: f32, x: f64) -> f32 {
    (f64::from(a) + (f64::from(b) - f64::from(a)) * x) as f32
}

impl Voice {
    /// A voice at full volume, heard from the sound's first sample.
    pub fn new(pcm: Arc<Pcm>, kind: VoiceKind, at: u64, end: u64) -> Voice {
        Voice { pcm, kind, at, end, gain: 1.0, skip: 0, envelope: None }
    }

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
    let mut gains: Vec<(f32, f32)> = Vec::new();
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
        // Where in the sound's time (output frames from its first sample) the window starts.
        let pos = (a - v.at).saturating_add(v.skip);
        // The envelope's gains, one pair per output frame.
        gains.clear();
        if let Some(env) = &v.envelope {
            gains.resize((b - a) as usize, (0.0, 0.0));
            env.fill(pos, &mut gains);
            if gains.iter().all(|&(l, r)| l == 0.0 && r == 0.0) {
                continue;
            }
        }
        let gain_at = |k: usize| gains.get(k).map_or((g, g), |&(l, r)| (l * g, r * g));
        if v.pcm.rate == rate {
            let mut i = (pos % len as u64) as usize;
            for (k, [x, y]) in dst.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                let (l, r) = v.pcm.frame(i);
                let (gl, gr) = gain_at(k);
                *x += l * gl;
                *y += r * gr;
                i += 1;
                if i == len {
                    i = 0;
                }
            }
        } else {
            let looping = (v.end - v.at).saturating_add(v.skip) > Voice::play_frames(&v.pcm, rate);
            let step = f64::from(v.pcm.rate) / f64::from(rate.max(1));
            for (k, [x, y]) in dst.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                let t = pos.saturating_add(k as u64) as f64 * step;
                let (l, r) = interpolate(&v.pcm, t, looping);
                let (gl, gr) = gain_at(k);
                *x += l * gl;
                *y += r * gr;
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
        Voice::new(p.clone(), kind, at, end)
    }

    fn env(points: &[(u64, f32, f32)]) -> Option<Arc<Envelope>> {
        Envelope::new(points.iter().map(|&(at, left, right)| EnvPoint { at, left, right }).collect()).map(Arc::new)
    }

    #[test]
    fn an_envelope_scales_each_channel_by_straight_lines_between_its_points() {
        let a = pcm(8000, 1, &[0.5; 40]);
        let mut v = voice(&a, VoiceKind::Event, 4, 44);
        // Left fades out over 20 frames from frame 10 of the sound; right rises and holds.
        v.envelope = env(&[(30, 0.0, 1.0), (10, 1.0, 0.0)]);
        v.gain = 0.5;
        let out = render(&[v.clone()], 8000, MixOptions::default(), 0, 48);
        for t in 0..40u64 {
            let (gl, gr) = match t {
                0..=10 => (1.0, 0.0),
                11..=29 => (lerp(1.0, 0.0, (t - 10) as f64 / 20.0), lerp(0.0, 1.0, (t - 10) as f64 / 20.0)),
                _ => (0.0, 1.0),
            };
            let k = (t + 4) as usize;
            assert_eq!((out[2 * k], out[2 * k + 1]), (0.5 * (0.5 * gl), 0.5 * (0.5 * gr)), "sound frame {t}");
        }
        // Blocks of any size give the same samples.
        let mut joined = render(&[v.clone()], 8000, MixOptions::default(), 0, 17);
        joined.extend(render(&[v.clone()], 8000, MixOptions::default(), 17, 31));
        assert_eq!(joined, out);
        // A voice whose envelope is silent over the window adds nothing; one point is a level.
        v.envelope = env(&[(0, 0.0, 0.0)]);
        assert!(render(&[v.clone()], 8000, MixOptions::default(), 0, 48).iter().all(|&s| s == 0.0));
        v.envelope = env(&[(7, 0.25, 1.0)]);
        assert_eq!(&render(&[v], 8000, MixOptions::default(), 4, 1)[..], &[0.5 * 0.5 * 0.25, 0.5 * 0.5]);
        assert!(Envelope::new(Vec::new()).is_none());
        let e = Envelope::new(vec![EnvPoint { at: 0, left: f32::NAN, right: 7.0 }]).unwrap();
        assert_eq!(e.gain(99), (0.0, 1.0), "gains stay in 0..=1");
    }

    #[test]
    fn the_envelope_runs_across_repeats_and_from_a_skip() {
        let a = pcm(8000, 1, &[0.1, 0.2, 0.3, 0.4]);
        // Played twice from frame 0, the envelope fades over all eight frames.
        let mut v = voice(&a, VoiceKind::Event, 0, 8);
        v.envelope = env(&[(0, 1.0, 1.0), (8, 0.0, 0.0)]);
        let left: Vec<f32> = render(&[v.clone()], 8000, MixOptions::default(), 0, 8).into_iter().step_by(2).collect();
        let want: Vec<f32> = (0..8).map(|t| [0.1f32, 0.2, 0.3, 0.4][t % 4] * lerp(1.0, 0.0, t as f64 / 8.0)).collect();
        assert_eq!(left, want);
        // Skipping three frames: heard from the fourth sample, the envelope three frames in.
        v.skip = 3;
        v.at = 10;
        v.end = 13;
        let left: Vec<f32> = render(&[v], 8000, MixOptions::default(), 10, 3).into_iter().step_by(2).collect();
        assert_eq!(left, vec![0.4 * lerp(1.0, 0.0, 3.0 / 8.0), 0.1 * lerp(1.0, 0.0, 4.0 / 8.0), 0.2 * lerp(1.0, 0.0, 5.0 / 8.0)]);
        // Resampled voices skip and fade the same way (a constant stays the envelope).
        let dc = pcm(4000, 1, &[0.5; 64]);
        let mut v = voice(&dc, VoiceKind::Stream, 0, 64);
        v.skip = 20;
        v.envelope = env(&[(20, 1.0, 0.0), (60, 0.0, 1.0)]);
        let out = render(&[v], 8000, MixOptions::default(), 0, 40);
        for k in 4..36 {
            let x = k as f64 / 40.0;
            assert!((out[2 * k] - 0.5 * lerp(1.0, 0.0, x)).abs() < 1e-6 && (out[2 * k + 1] - 0.5 * lerp(0.0, 1.0, x)).abs() < 1e-6, "{k}");
        }
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
