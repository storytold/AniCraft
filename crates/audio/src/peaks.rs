//! Waveform summaries: the lowest and highest sample of each block of frames, so the timeline
//! can draw a sound at any zoom without touching its samples.

use crate::Pcm;

#[derive(Clone, Debug, PartialEq)]
pub struct Peaks {
    min: Vec<f32>,
    max: Vec<f32>,
    frames: usize,
}

impl Peaks {
    /// Frames per block.
    pub const BLOCK: usize = 256;

    /// The peaks of `pcm` (both channels together).
    pub fn of(pcm: &Pcm) -> Peaks {
        let ch = usize::from(pcm.channels.max(1));
        let (mut min, mut max) = (Vec::new(), Vec::new());
        for block in pcm.samples.chunks(Self::BLOCK * ch) {
            let (lo, hi) = block.iter().fold((0.0f32, 0.0f32), |(lo, hi), &s| (lo.min(s), hi.max(s)));
            min.push(lo);
            max.push(hi);
        }
        Peaks { min, max, frames: pcm.frames() }
    }

    /// The peaks of one channel of `pcm` (0 left, 1 right; a mono sound's one channel for
    /// either), for drawing the channels apart (the envelope editor).
    pub fn of_channel(pcm: &Pcm, channel: usize) -> Peaks {
        let ch = usize::from(pcm.channels.max(1));
        let c = channel.min(ch - 1);
        let (mut min, mut max) = (Vec::new(), Vec::new());
        for block in pcm.samples.chunks(Self::BLOCK * ch) {
            let (lo, hi) = block.iter().skip(c).step_by(ch).fold((0.0f32, 0.0f32), |(lo, hi), &s| (lo.min(s), hi.max(s)));
            min.push(lo);
            max.push(hi);
        }
        Peaks { min, max, frames: pcm.frames() }
    }

    /// Frames of the sound summarised.
    pub fn frames(&self) -> usize {
        self.frames
    }

    /// Lowest and highest sample among frames `a..b` (to block precision); `None` outside the
    /// sound.
    pub fn range(&self, a: usize, b: usize) -> Option<(f32, f32)> {
        let b = b.min(self.frames);
        if a >= b {
            return None;
        }
        let (i, j) = (a / Self::BLOCK, (b - 1) / Self::BLOCK + 1);
        let lo = self.min.get(i..j)?.iter().fold(0.0f32, |m, &v| m.min(v));
        let hi = self.max.get(i..j)?.iter().fold(0.0f32, |m, &v| m.max(v));
        Some((lo, hi))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn peaks_cover_both_channels_by_block() {
        let mut s = vec![0.0f32; 2 * 600];
        s[2 * 10] = 0.75; // left, block 0
        s[2 * 300 + 1] = -0.5; // right, block 1
        let p = Peaks::of(&Pcm::new(8000, 2, s).unwrap());
        assert_eq!(p.frames(), 600);
        assert_eq!(p.range(0, 256), Some((0.0, 0.75)));
        assert_eq!(p.range(256, 512), Some((-0.5, 0.0)));
        assert_eq!(p.range(0, 600), Some((-0.5, 0.75)));
        assert_eq!(p.range(512, 600), Some((0.0, 0.0)));
        assert_eq!(p.range(600, 900), None);
        assert_eq!(p.range(5, 5), None);
    }
}
