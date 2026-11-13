//! Video items in Animate documents (ADR 0086). AniCraft has no video playback, so an embedded
//! video comes across as a still of its first frame when its stream is one AniCraft can read,
//! and is otherwise named in the report with its codec and length.
//!
//! Written from the public *Video File Format Specification* (FLV, version 10) and the *SWF File
//! Format Specification* (version 19), both published by Adobe as open specifications: the FLV
//! header and tags, and the **Screen Video** codec (codec 3), whose frames are blocks of
//! zlib-compressed BGR pixels. That is the one embedded-video codec here that is both public and
//! free of patents; Sorenson H.263, On2 VP6, Screen Video 2 and H.264 are named, not decoded.
//!
//! Every length is checked against the bytes it comes from; tags, blocks and pixels are bounded.
//! Never panics.

use anicraft_doc::MAX_BITMAP_EDGE;

/// Most FLV tags looked through for the first video frame (audio and script tags come first).
const MAX_TAGS: usize = 4096;

/// What a video's data turned out to be.
#[derive(Debug, Clone, PartialEq)]
pub enum Video {
    /// Its first frame, as straight RGBA8 pixels.
    Frame { width: u32, height: u32, rgba: Vec<u8> },
    /// A video AniCraft can't show, by the name of its format ("FLV, On2 VP6").
    Unreadable(String),
}

fn u24(d: &[u8], at: usize) -> Option<usize> {
    let b = d.get(at..at.checked_add(3)?)?;
    Some((usize::from(b[0]) << 16) | (usize::from(b[1]) << 8) | usize::from(b[2]))
}

fn u32_be(d: &[u8], at: usize) -> Option<usize> {
    let b = d.get(at..at.checked_add(4)?)?;
    usize::try_from(u32::from_be_bytes([b[0], b[1], b[2], b[3]])).ok()
}

/// The FLV codec ids, by name (FLV specification, `VIDEODATA`).
fn codec_name(id: u8) -> &'static str {
    match id {
        1 => "JPEG",
        2 => "Sorenson H.263",
        3 => "Screen Video",
        4 => "On2 VP6",
        5 => "On2 VP6 with alpha",
        6 => "Screen Video 2",
        7 => "H.264",
        _ => "an unknown codec",
    }
}

/// Reads a video's data: an FLV file's first Screen Video key frame, else the format's name.
pub fn read(data: &[u8]) -> Video {
    if data.starts_with(b"FLV") {
        return flv(data).unwrap_or_else(|| Video::Unreadable("a damaged FLV file".into()));
    }
    let name = if data.get(4..8) == Some(b"ftyp") {
        "MPEG-4 or QuickTime video"
    } else if data.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        "WebM or Matroska video"
    } else if data.starts_with(b"RIFF") && data.get(8..12) == Some(b"AVI ") {
        "AVI video"
    } else {
        "Animate's own video data"
    };
    Video::Unreadable(name.into())
}

/// The first video tag of an FLV file. `None` when the file is cut short or lies.
fn flv(data: &[u8]) -> Option<Video> {
    // Header: "FLV", version, flags, header size; then the size of the tag before (0).
    let mut at = u32_be(data, 5)?.checked_add(4)?;
    for _ in 0..MAX_TAGS {
        // Tag: type, data size (24 bits), time stamp (24 + 8 bits), stream id (24 bits).
        let kind = *data.get(at)?;
        let size = u24(data, at + 1)?;
        let body = data.get(at.checked_add(11)?..at.checked_add(11)?.checked_add(size)?)?;
        if kind & 0x1f == 9 {
            let first = *body.first()?;
            let (frame_type, codec) = (first >> 4, first & 0x0f);
            if codec != 3 {
                return Some(Video::Unreadable(format!("FLV, {}", codec_name(codec))));
            }
            if frame_type != 1 {
                return Some(Video::Unreadable("FLV, Screen Video starting without a key frame".into()));
            }
            return Some(screen_video(body.get(1..)?).unwrap_or_else(|| Video::Unreadable("FLV, damaged Screen Video".into())));
        }
        // The next tag, after this one's trailing size.
        at = at.checked_add(11)?.checked_add(size)?.checked_add(4)?;
    }
    Some(Video::Unreadable("FLV with no video frame near its start".into()))
}

/// A Screen Video key frame (SWF specification, `SCREENVIDEOPACKET`): block and image sizes,
/// then every block bottom row first, left to right, each a 16-bit size and zlib-compressed BGR
/// pixels, bottom row first. A key frame has every block.
fn screen_video(p: &[u8]) -> Option<Video> {
    let h0 = u16::from_be_bytes([*p.first()?, *p.get(1)?]);
    let h1 = u16::from_be_bytes([*p.get(2)?, *p.get(3)?]);
    let (bw, w) = ((usize::from(h0 >> 12) + 1) * 16, usize::from(h0 & 0x0fff));
    let (bh, h) = ((usize::from(h1 >> 12) + 1) * 16, usize::from(h1 & 0x0fff));
    if w == 0 || h == 0 || w > MAX_BITMAP_EDGE as usize || h > MAX_BITMAP_EDGE as usize {
        return None;
    }
    let mut rgba = vec![0u8; w * h * 4];
    let mut at = 4usize;
    // Block rows from the bottom of the picture.
    for by in 0..h.div_ceil(bh) {
        for bx in 0..w.div_ceil(bw) {
            let (x0, y0) = (bx * bw, by * bh);
            let (cw, ch) = (bw.min(w - x0), bh.min(h - y0));
            let size = usize::from(u16::from_be_bytes([*p.get(at)?, *p.get(at + 1)?]));
            let packed = p.get(at + 2..at + 2 + size)?;
            at += 2 + size;
            if size == 0 {
                // An unchanged block can't be in a key frame.
                return None;
            }
            let want = cw * ch * 3;
            let bgr = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(packed, want).ok()?;
            if bgr.len() != want {
                return None;
            }
            for (row, line) in bgr.chunks_exact(cw * 3).enumerate() {
                // Rows count up from the bottom; the bitmap's rows count down from the top.
                let y = h - 1 - (y0 + row);
                for (col, px) in line.as_chunks::<3>().0.iter().enumerate() {
                    let i = (y * w + x0 + col) * 4;
                    rgba.get_mut(i..i + 4)?.copy_from_slice(&[px[2], px[1], px[0], 255]);
                }
            }
        }
    }
    Some(Video::Frame { width: w as u32, height: h as u32, rgba })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// An FLV file holding one Screen Video key frame of `w × h` pixels coloured by `px(x, y)`
    /// (from the top left), in `block`-pixel blocks, after an audio tag.
    pub(crate) fn screen_flv(w: usize, h: usize, block: usize, px: impl Fn(usize, usize) -> [u8; 3]) -> Vec<u8> {
        let mut packet = vec![0x13u8];
        let n = (block / 16 - 1) as u16;
        packet.extend_from_slice(&((n << 12) | w as u16).to_be_bytes());
        packet.extend_from_slice(&((n << 12) | h as u16).to_be_bytes());
        for by in 0..h.div_ceil(block) {
            for bx in 0..w.div_ceil(block) {
                let (x0, y0) = (bx * block, by * block);
                let mut bgr = Vec::new();
                for row in 0..block.min(h - y0) {
                    for col in 0..block.min(w - x0) {
                        let [r, g, b] = px(x0 + col, h - 1 - (y0 + row));
                        bgr.extend_from_slice(&[b, g, r]);
                    }
                }
                let z = miniz_oxide::deflate::compress_to_vec_zlib(&bgr, 6);
                packet.extend_from_slice(&(z.len() as u16).to_be_bytes());
                packet.extend_from_slice(&z);
            }
        }
        let mut out = b"FLV\x01\x05\x00\x00\x00\x09\x00\x00\x00\x00".to_vec();
        let mut tag = |kind: u8, body: &[u8]| {
            out.push(kind);
            out.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
            out.extend_from_slice(&[0; 7]);
            out.extend_from_slice(body);
            out.extend_from_slice(&(body.len() as u32 + 11).to_be_bytes());
        };
        tag(8, &[0x2a, 0, 0, 0]);
        tag(9, &packet);
        out
    }

    #[test]
    fn a_screen_video_key_frame_decodes_with_its_blocks_in_place() {
        // 40 × 20 in 16-pixel blocks: partial blocks on the right and at the top.
        let px = |x: usize, y: usize| [(x * 6) as u8, (y * 12) as u8, if x < 20 { 255 } else { 0 }];
        let Video::Frame { width, height, rgba } = read(&screen_flv(40, 20, 16, px)) else { panic!("no frame") };
        assert_eq!((width, height), (40, 20));
        for (x, y) in [(0, 0), (39, 0), (0, 19), (39, 19), (17, 3), (21, 18)] {
            let i = (y * 40 + x) * 4;
            let [r, g, b] = px(x, y);
            assert_eq!(&rgba[i..i + 4], &[r, g, b, 255], "({x}, {y})");
        }
    }

    #[test]
    fn other_codecs_and_formats_are_named() {
        let mut flv = screen_flv(16, 16, 16, |_, _| [0; 3]);
        // The video tag's first byte, after the header, the audio tag and its size.
        let video = 13 + 11 + 4 + 4 + 11;
        for (codec, name) in [(2u8, "FLV, Sorenson H.263"), (4, "FLV, On2 VP6"), (7, "FLV, H.264")] {
            flv[video] = 0x10 | codec;
            assert_eq!(read(&flv), Video::Unreadable(name.into()));
        }
        assert_eq!(read(b"\0\0\0\x18ftypmp42"), Video::Unreadable("MPEG-4 or QuickTime video".into()));
        assert_eq!(read(b"garbage"), Video::Unreadable("Animate's own video data".into()));
    }

    #[test]
    fn damaged_screen_video_never_panics() {
        let good = screen_flv(40, 20, 32, |x, y| [x as u8, y as u8, 7]);
        for cut in 0..good.len() {
            let _ = read(&good[..cut]);
        }
        let mut rng = 0x2545_F491_4F6C_DD1Du64;
        for _ in 0..2000 {
            let mut bad = good.clone();
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            let i = (rng % bad.len() as u64) as usize;
            bad[i] = (rng >> 32) as u8;
            let _ = read(&bad);
        }
        // A tag size far past the end, and an image claiming the largest size the bits allow.
        let mut lying = good.clone();
        lying[14..17].copy_from_slice(&[0xff, 0xff, 0xff]);
        assert!(matches!(read(&lying), Video::Unreadable(_)));
    }
}
