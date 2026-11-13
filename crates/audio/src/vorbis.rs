//! Ogg Vorbis through `lewton` (pure Rust, MIT OR Apache-2.0).

use crate::{AudioError, Collector, Pcm};
use lewton::inside_ogg::OggStreamReader;
use lewton::samples::InterleavedSamples;

pub fn decode(data: &[u8]) -> Result<Pcm, AudioError> {
    let mut r = OggStreamReader::new(std::io::Cursor::new(data)).map_err(|_| AudioError::Unreadable)?;
    let rate = r.ident_hdr.audio_sample_rate;
    let mut col = Collector::new(usize::from(r.ident_hdr.audio_channels));
    loop {
        match r.read_dec_packet_generic::<InterleavedSamples<f32>>() {
            Ok(Some(p)) => {
                if p.channel_count != col.channels {
                    break; // a chained stream with another layout: keep the first part
                }
                col.push(&p.samples)?;
            }
            Ok(None) => break,
            // A damaged tail: keep what decoded before it.
            Err(_) if !col.samples.is_empty() => break,
            Err(_) => return Err(AudioError::Unreadable),
        }
    }
    Pcm::new(rate, col.out_channels(), col.samples)
}
