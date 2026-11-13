//! MP3 (MPEG-1/2 audio layers I–III) through `nanomp3` (pure Rust, MIT OR Apache-2.0). The
//! MP3 patents have expired; decoding is royalty-free. Tags are skipped, and the encoder delay
//! and padding are trimmed when the file's LAME tag records them (gapless).

use crate::{AudioError, Collector, Pcm};

pub fn decode(data: &[u8]) -> Result<Pcm, AudioError> {
    let mut r = nanomp3::SliceReader::<f32>::new(data);
    let mut col: Option<Collector> = None;
    let mut rate = 0;
    let mut buf: Vec<f32> = Vec::new();
    loop {
        match r.read_frame() {
            Ok(Some(frame)) => {
                buf.clear();
                buf.extend_from_slice(frame);
                let ch = r.channels().map_or(1, |c| usize::from(c.num()));
                let c = col.get_or_insert_with(|| Collector::new(ch));
                if c.channels != ch {
                    break;
                }
                rate = r.sample_rate();
                c.push(&buf)?;
            }
            Ok(None) => break,
            // The format changed mid-stream or the data is damaged: keep what came before.
            Err(_) if col.as_ref().is_some_and(|c| !c.samples.is_empty()) => break,
            Err(_) => return Err(AudioError::Unreadable),
        }
    }
    let col = col.ok_or(AudioError::Unreadable)?;
    Pcm::new(rate, col.out_channels(), col.samples)
}
