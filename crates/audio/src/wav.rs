//! WAV (RIFF WAVE) reading and writing, from the public RIFF and WAVE format descriptions
//! (Microsoft/IBM *Multimedia Programming Interface and Data Specifications 1.0*, 1991, and
//! the `WAVE_FORMAT_EXTENSIBLE` note).
//!
//! Reads integer PCM (8, 16, 24 and 32 bit) and IEEE float (32 and 64 bit), plain or
//! extensible, any channel count (the first two are kept). Writes 16-bit, 24-bit or 32-bit
//! float. Every read is bounds-checked: a damaged file is an error, never a crash.

use crate::{AudioError, MAX_SAMPLES, Pcm};

/// Sample formats written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bits {
    Int16,
    Int24,
    Float32,
}

impl Bits {
    fn bytes(self) -> u16 {
        match self {
            Bits::Int16 => 2,
            Bits::Int24 => 3,
            Bits::Float32 => 4,
        }
    }
}

const PCM: u16 = 1;
const FLOAT: u16 = 3;
const EXTENSIBLE: u16 = 0xFFFE;

fn u16_at(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

struct Fmt {
    tag: u16,
    channels: u16,
    rate: u32,
    block_align: u16,
    bits: u16,
}

fn read_fmt(c: &[u8]) -> Option<Fmt> {
    let mut f = Fmt { tag: u16_at(c, 0)?, channels: u16_at(c, 2)?, rate: u32_at(c, 4)?, block_align: u16_at(c, 12)?, bits: u16_at(c, 14)? };
    if f.tag == EXTENSIBLE {
        // cbSize, valid bits, channel mask, then the sub-format GUID whose first two bytes are
        // the format code.
        f.tag = u16_at(c, 24)?;
    }
    Some(f)
}

/// Decodes a WAV file.
pub fn decode(data: &[u8]) -> Result<Pcm, AudioError> {
    if data.get(0..4) != Some(b"RIFF") || data.get(8..12) != Some(b"WAVE") {
        return Err(AudioError::UnknownFormat);
    }
    let (mut fmt, mut body) = (None, None);
    let mut at = 12usize;
    // Chunks: id, size, data, a pad byte after odd sizes. Bounded by the data length.
    while let (Some(id), Some(size)) = (data.get(at..at + 4), u32_at(data, at + 4)) {
        let start = at + 8;
        let end = start.saturating_add(size as usize).min(data.len());
        let chunk = data.get(start..end).unwrap_or(&[]);
        match id {
            b"fmt " => fmt = read_fmt(chunk),
            // Streamed files may give 0 or 0xFFFFFFFF as the size: the data runs to the end.
            b"data" => body = Some(if size == 0 || size == u32::MAX { data.get(start..).unwrap_or(&[]) } else { chunk }),
            _ => {}
        }
        if body.is_some() && fmt.is_some() {
            break;
        }
        at = end + (size as usize & 1);
        if end >= data.len() {
            break;
        }
    }
    let fmt = fmt.ok_or(AudioError::Unreadable)?;
    let body = body.ok_or(AudioError::Empty)?;
    if fmt.channels == 0 || fmt.channels > 32 {
        return Err(AudioError::Unsupported(format!("{} channels", fmt.channels)));
    }
    let width = match (fmt.tag, fmt.bits) {
        (PCM, 8) => 1usize,
        (PCM, 16) => 2,
        (PCM, 24) => 3,
        (PCM, 32) | (FLOAT, 32) => 4,
        (FLOAT, 64) => 8,
        (PCM | FLOAT, b) => return Err(AudioError::Unsupported(format!("{b}-bit samples"))),
        (t, _) => return Err(AudioError::Unsupported(format!("WAV codec 0x{t:04x} (only uncompressed PCM and float)"))),
    };
    let channels = usize::from(fmt.channels);
    // Samples may sit in wider containers (block align); read each from its slot's start.
    let block = usize::from(fmt.block_align).max(width * channels);
    let slot = block / channels;
    let frames = body.len() / block;
    if frames.saturating_mul(channels.min(2)) > MAX_SAMPLES {
        return Err(AudioError::TooLong);
    }
    let read = |s: &[u8]| -> f32 {
        match (fmt.tag, width, s) {
            (PCM, 1, [a, ..]) => (f32::from(*a) - 128.0) / 128.0,
            (PCM, 2, [a, b, ..]) => f32::from(i16::from_le_bytes([*a, *b])) / 32_768.0,
            (PCM, 3, [a, b, c, ..]) => (i32::from_le_bytes([0, *a, *b, *c]) >> 8) as f32 / 8_388_608.0,
            (PCM, 4, [a, b, c, d, ..]) => (f64::from(i32::from_le_bytes([*a, *b, *c, *d])) / 2_147_483_648.0) as f32,
            (FLOAT, 4, [a, b, c, d, ..]) => f32::from_le_bytes([*a, *b, *c, *d]),
            (FLOAT, 8, [a, b, c, d, e, f, g, h, ..]) => f64::from_le_bytes([*a, *b, *c, *d, *e, *f, *g, *h]) as f32,
            _ => 0.0,
        }
    };
    // The first two channels are kept (front left and right in WAV channel order).
    let keep = channels.min(2);
    let mut out = Vec::with_capacity(frames * keep);
    for f in body.chunks_exact(block) {
        out.extend(f.chunks_exact(slot).take(keep).map(read));
    }
    Pcm::new(fmt.rate, keep as u16, out)
}

/// A WAV header for `frames` frames of `channels` channels.
pub fn header(rate: u32, channels: u16, bits: Bits, frames: u64) -> Option<Vec<u8>> {
    let b = bits.bytes();
    let data = u32::try_from(frames.checked_mul(u64::from(b) * u64::from(channels))?).ok()?;
    let riff = data.checked_add(36)?;
    let block = channels.checked_mul(b)?;
    let tag = if bits == Bits::Float32 { FLOAT } else { PCM };
    let mut h = Vec::with_capacity(44);
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&riff.to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16u32.to_le_bytes());
    h.extend_from_slice(&tag.to_le_bytes());
    h.extend_from_slice(&channels.to_le_bytes());
    h.extend_from_slice(&rate.to_le_bytes());
    h.extend_from_slice(&(rate.saturating_mul(u32::from(block))).to_le_bytes());
    h.extend_from_slice(&block.to_le_bytes());
    h.extend_from_slice(&(b * 8).to_le_bytes());
    h.extend_from_slice(b"data");
    h.extend_from_slice(&data.to_le_bytes());
    Some(h)
}

/// Appends samples in `bits` format. Integers are rounded and clipped to range; `x · 32768`
/// for 16-bit, so 16-bit sources read by [`decode`] come back bit-identical.
pub fn append(out: &mut Vec<u8>, samples: &[f32], bits: Bits) {
    out.reserve(samples.len() * usize::from(bits.bytes()));
    for &s in samples {
        let s = if s.is_finite() { s } else { 0.0 };
        match bits {
            Bits::Int16 => out.extend_from_slice(&((s * 32_768.0).round().clamp(-32_768.0, 32_767.0) as i16).to_le_bytes()),
            Bits::Int24 => {
                let v = (f64::from(s) * 8_388_608.0).round().clamp(-8_388_608.0, 8_388_607.0) as i32;
                out.extend_from_slice(v.to_le_bytes().get(..3).unwrap_or(&[0, 0, 0]));
            }
            Bits::Float32 => out.extend_from_slice(&s.to_le_bytes()),
        }
    }
}

/// A whole WAV file in memory (empty when it would not fit a WAV's 4 GB limit).
pub fn encode(rate: u32, channels: u16, samples: &[f32], bits: Bits) -> Vec<u8> {
    let frames = samples.len() / usize::from(channels.max(1));
    let Some(mut out) = header(rate, channels, bits, frames as u64) else { return Vec::new() };
    append(&mut out, samples.get(..frames * usize::from(channels.max(1))).unwrap_or(&[]), bits);
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn sixteen_bit_round_trips_bit_exactly() {
        let src: Vec<i16> = vec![0, 1, -1, 32_767, -32_768, 12_345, -54, 7];
        let f: Vec<f32> = src.iter().map(|&v| f32::from(v) / 32_768.0).collect();
        let wav = encode(22_050, 2, &f, Bits::Int16);
        assert_eq!(wav.len(), 44 + 16);
        let p = decode(&wav).unwrap();
        assert_eq!((p.rate, p.channels, p.frames()), (22_050, 2, 4));
        assert_eq!(p.samples, f);
        let mut again = Vec::new();
        append(&mut again, &p.samples, Bits::Int16);
        assert_eq!(&again[..], &wav[44..], "bit-identical after a round trip");
    }

    #[test]
    fn every_sample_format_reads_to_the_same_values() {
        let vals = [0.0f32, 0.5, -0.5, 0.25];
        for bits in [Bits::Int16, Bits::Int24, Bits::Float32] {
            let p = decode(&encode(48_000, 1, &vals, bits)).unwrap();
            for (a, b) in p.samples.iter().zip(vals) {
                assert!((a - b).abs() < 1e-4, "{bits:?}: {a} vs {b}");
            }
        }
        // 8-bit unsigned, written by hand: 128 is silence.
        let mut w = header(8000, 1, Bits::Int16, 0).unwrap();
        w[34] = 8; // bits per sample
        w[32] = 1; // block align
        w.truncate(40);
        w.extend_from_slice(&4u32.to_le_bytes());
        w.extend_from_slice(&[128, 255, 0, 192]);
        let p = decode(&w).unwrap();
        assert_eq!(p.samples, vec![0.0, 127.0 / 128.0, -1.0, 0.5]);
    }

    #[test]
    fn extensible_surround_keeps_front_left_and_right_and_skips_unknown_chunks() {
        // 6 channels of 16-bit in a WAVE_FORMAT_EXTENSIBLE fmt, after a LIST chunk of odd size.
        let mut w = Vec::new();
        w.extend_from_slice(b"RIFF\0\0\0\0WAVE");
        w.extend_from_slice(b"LIST");
        w.extend_from_slice(&3u32.to_le_bytes());
        w.extend_from_slice(b"abc\0"); // odd size: one pad byte
        w.extend_from_slice(b"fmt ");
        w.extend_from_slice(&40u32.to_le_bytes());
        w.extend_from_slice(&EXTENSIBLE.to_le_bytes());
        w.extend_from_slice(&6u16.to_le_bytes());
        w.extend_from_slice(&44_100u32.to_le_bytes());
        w.extend_from_slice(&(44_100u32 * 12).to_le_bytes());
        w.extend_from_slice(&12u16.to_le_bytes());
        w.extend_from_slice(&16u16.to_le_bytes());
        w.extend_from_slice(&22u16.to_le_bytes());
        w.extend_from_slice(&16u16.to_le_bytes());
        w.extend_from_slice(&0x3fu32.to_le_bytes());
        w.extend_from_slice(&PCM.to_le_bytes());
        w.extend_from_slice(&[0; 14]);
        w.extend_from_slice(b"data");
        w.extend_from_slice(&12u32.to_le_bytes());
        for v in [16_384i16, -16_384, 1, 2, 3, 4] {
            w.extend_from_slice(&v.to_le_bytes());
        }
        let p = decode(&w).unwrap();
        assert_eq!((p.channels, p.samples.clone()), (2, vec![0.5, -0.5]));
    }

    #[test]
    fn unsupported_and_damaged_files_say_so() {
        let mut w = encode(8000, 1, &[0.0; 4], Bits::Int16);
        w[20] = 2; // ADPCM
        assert!(matches!(decode(&w), Err(AudioError::Unsupported(_))));
        let ok = encode(8000, 1, &[0.0; 4], Bits::Int16);
        assert_eq!(decode(&ok[..36]), Err(AudioError::Empty), "no data chunk");
        assert_eq!(decode(&ok[..20]), Err(AudioError::Unreadable), "fmt cut short");
        // A data chunk claiming more than there is reads what is there.
        let mut long = ok.clone();
        long[40..44].copy_from_slice(&1_000_000u32.to_le_bytes());
        assert_eq!(decode(&long).unwrap().frames(), 4);
        assert!(header(8000, 2, Bits::Float32, u64::MAX / 2).is_none(), "past the 4 GB limit");
    }
}
