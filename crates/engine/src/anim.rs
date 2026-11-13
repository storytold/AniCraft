//! Animated PNG and animated WebP export (ADR 0063): full colour and real alpha, for the web
//! and chat apps, where an animated GIF has 256 colours and 1-bit transparency.
//!
//! Both formats share one pipeline. Frames render in parallel, a batch at a time (memory stays
//! bounded), as GIF export does. A frame identical to the one before it is merged into that
//! frame's delay. Every other frame is cut down to the rectangle that changed since the frame
//! before and compressed in parallel as an image of its own (a PNG, or a lossless WebP); the
//! compressed data is then put into one file in order, by hand, chunk by chunk:
//!
//! * **APNG** (the APNG specification): `IHDR`, `acTL`, then per frame an `fcTL` (its region,
//!   delay, dispose op none, blend op) and its `IDAT` / `fdAT` data. Delays are exact fractions
//!   of the document's frame rate (2 frames at 29.97 fps: 200/2997 s). A region whose changed
//!   pixels are all opaque blends *over* the frame before, with its unchanged pixels left
//!   transparent (they compress to almost nothing); otherwise it replaces (*source*).
//! * **WebP** (the WebP container specification): `VP8X`, `ANIM`, then an `ANMF` per frame
//!   wrapping its `VP8L` (lossless) bitstream. Regions start on even pixels, as the format
//!   needs, and replace what was there (no blending: the decoders' blend rounds opaque pixels).
//!   Durations are whole milliseconds; each is rounded from the exact time the frame starts, so
//!   the error never adds up and the running time is exact to the millisecond.
//!
//! A delay longer than one frame header can hold goes on, after the frame, as frames that
//! change nothing (one transparent pixel blended over), so timing is never cut short.

use crate::params::*;
use crate::{EngineError, Result, Session};
use anicraft_render::{View, render};
use rayon::prelude::*;
use serde_json::{Value, json};
use std::path::PathBuf;

/// The two animated formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Apng,
    Webp,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Apng => "animated PNG",
            Kind::Webp => "animated WebP",
        }
    }
}

/// Largest animation made (frames × pixels), as for GIF: time stays bounded.
pub const MAX_ANIM_PIXELS: u64 = 600 * 1280 * 720;
/// WebP canvases and frames are at most 16384 px per side.
pub const MAX_WEBP_EDGE: u32 = 16_384;
/// Pixels rendered at once (a batch of frames), so memory stays bounded at any size.
const BATCH_PIXELS: u64 = 32 * 1024 * 1024;
/// Most plays a loop count says (both formats store it in 16 bits for WebP's sake).
pub const MAX_PLAYS: u32 = anicraft_doc::publish::MAX_PLAYS;
/// Longest WebP frame duration (24 bits of milliseconds).
const MAX_WEBP_MS: u64 = 0xFF_FFFF;

/// A rectangle of the canvas, in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// One frame, compressed: the region it covers, whether it blends over what is there (else it
/// replaces it), and its image data (PNG `IDAT` data, or a whole `VP8L` chunk).
struct Encoded {
    rect: Rect,
    over: bool,
    data: Vec<u8>,
}

/// The frame rate as a fraction `p / q` frames per second: 24 → 24/1, 29.97 → 2997/100,
/// 23.976 → 2997/125. Rates no small fraction matches are taken to a thousandth.
pub fn fps_ratio(fps: f32) -> (u64, u64) {
    let fps = if fps.is_finite() && fps > 0.0 { f64::from(fps) } else { 24.0 };
    for q in 1..=1001u64 {
        let p = (fps * q as f64).round();
        if p >= 1.0 && (p / q as f64 - fps).abs() <= fps * 1e-6 {
            return (p as u64, q);
        }
    }
    let p = ((fps * 1000.0).round() as u64).max(1);
    let g = gcd(p, 1000);
    (p / g, 1000 / g)
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.max(1)
}

/// When frame `t` (from the first exported) starts, in whole milliseconds, rounded from the
/// exact time at `p / q` fps.
pub fn start_ms(t: u64, (p, q): (u64, u64)) -> u64 {
    let p = u128::from(p.max(1));
    ((2 * u128::from(t) * 1000 * u128::from(q) + p) / (2 * p)) as u64
}

/// The APNG delays (numerator, denominator in seconds) of a frame shown from frame `start` for
/// `count` frames: one exact fraction, or more when it is longer than one `fcTL` holds.
pub fn apng_delays(start: u64, count: u64, (p, q): (u64, u64)) -> Vec<(u16, u16)> {
    let mut out = Vec::new();
    if let Ok(den) = u16::try_from(p) {
        // `count` frames last count·q/p s; pieces of at most 65535/q frames always fit.
        let most = (u64::from(u16::MAX) / q.max(1)).max(1);
        let mut left = count;
        while left > 0 {
            let m = left.min(most);
            let (num, g) = (m * q, gcd(m * q, p));
            out.push((u16::try_from(num / g).unwrap_or(u16::MAX), u16::try_from(u64::from(den) / g).unwrap_or(den)));
            left -= m;
        }
    } else {
        // A rate with a huge denominator: milliseconds, rounded from each frame's start.
        let mut ms = start_ms(start + count, (p, q)) - start_ms(start, (p, q));
        while ms > 0 {
            let m = ms.min(u64::from(u16::MAX));
            out.push((m as u16, 1000));
            ms -= m;
        }
    }
    if out.is_empty() {
        out.push((0, 1));
    }
    out
}

/// The WebP durations (milliseconds) of a frame shown from frame `start` for `count` frames:
/// one, or more when it is longer than one `ANMF` holds.
pub fn webp_durations(start: u64, count: u64, ratio: (u64, u64)) -> Vec<u32> {
    let mut ms = start_ms(start + count, ratio).saturating_sub(start_ms(start, ratio));
    let mut out = Vec::new();
    loop {
        let m = ms.min(MAX_WEBP_MS);
        out.push(m as u32);
        ms -= m;
        if ms == 0 {
            return out;
        }
    }
}

/// The smallest rectangle holding every pixel where RGBA8 images `a` and `b` (`w` × `h`) differ,
/// or `None` when they are the same.
pub fn changed(a: &[u8], b: &[u8], w: u32, h: u32) -> Option<Rect> {
    let stride = w as usize * 4;
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    for y in 0..h {
        let at = y as usize * stride;
        let (Some(ra), Some(rb)) = (a.get(at..at + stride), b.get(at..at + stride)) else { break };
        if ra == rb {
            continue;
        }
        let (pa, pb) = (ra.as_chunks::<4>().0, rb.as_chunks::<4>().0);
        let first = pa.iter().zip(pb).position(|(u, v)| u != v).unwrap_or(0) as u32;
        let last = pa.iter().zip(pb).rposition(|(u, v)| u != v).unwrap_or(0) as u32;
        x0 = x0.min(first);
        x1 = x1.max(last);
        y0 = y0.min(y);
        y1 = y;
    }
    (y0 != u32::MAX).then(|| Rect { x: x0, y: y0, w: x1 - x0 + 1, h: y1 - y0 + 1 })
}

/// Rectangle `r` of RGBA8 image `px` (`w` wide).
fn crop(px: &[u8], w: u32, r: Rect) -> Vec<u8> {
    let stride = w as usize * 4;
    let mut out = Vec::with_capacity(r.w as usize * r.h as usize * 4);
    for y in r.y..r.y + r.h {
        let at = y as usize * stride + r.x as usize * 4;
        if let Some(row) = px.get(at..at + r.w as usize * 4) {
            out.extend_from_slice(row);
        }
    }
    out
}

/// What frame `cur` writes after frame `before` (none: it is the first, and covers the canvas):
/// the region, whether it blends over, and its pixels.
fn region(kind: Kind, before: Option<&[u8]>, cur: &[u8], w: u32, h: u32) -> (Rect, bool, Vec<u8>) {
    let full = Rect { x: 0, y: 0, w, h };
    let Some(before) = before else { return (full, false, cur.to_vec()) };
    let mut r = changed(before, cur, w, h).unwrap_or(full);
    match kind {
        Kind::Webp => {
            // Frame offsets are stored halved: the region starts on an even pixel.
            let (ex, ey) = (r.x & !1, r.y & !1);
            r = Rect { x: ex, y: ey, w: r.w + (r.x - ex), h: r.h + (r.y - ey) };
            (r, false, crop(cur, w, r))
        }
        Kind::Apng => {
            let (old, new) = (crop(before, w, r), crop(cur, w, r));
            let (po, pn) = (old.as_chunks::<4>().0, new.as_chunks::<4>().0);
            if po.iter().zip(pn).all(|(o, n)| o == n || n[3] == 255) {
                // Every changed pixel is opaque: blend over, unchanged pixels transparent.
                let mut px = Vec::with_capacity(new.len());
                for (o, n) in po.iter().zip(pn) {
                    px.extend_from_slice(if o == n { &[0, 0, 0, 0] } else { n });
                }
                (r, true, px)
            } else {
                (r, false, new)
            }
        }
    }
}

/// A standalone PNG's image data: its `IDAT` chunks' contents, joined.
fn idat(png: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut at = 8usize;
    while let Some(head) = png.get(at..at + 8) {
        let len = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
        let data = png.get(at + 8..at + 8 + len).ok_or_else(|| EngineError::Format("the PNG encoder wrote a short chunk".into()))?;
        if &head[4..8] == b"IDAT" {
            out.extend_from_slice(data);
        }
        at += 12 + len;
    }
    Ok(out)
}

/// A RIFF file's first chunk named `id`, whole (header, data and padding).
fn riff_chunk<'a>(riff: &'a [u8], id: &[u8; 4]) -> Option<&'a [u8]> {
    let mut at = 12usize;
    while let Some(head) = riff.get(at..at + 8) {
        let len = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
        let whole = 8 + len + (len & 1);
        if &head[0..4] == id {
            return riff.get(at..at + whole).or_else(|| riff.get(at..at + 8 + len));
        }
        at += whole;
    }
    None
}

/// Compresses `px` (`w` × `h` RGBA8) as one frame's data.
fn compress(kind: Kind, px: &[u8], w: u32, h: u32) -> Result<Vec<u8>> {
    let fail = |e: String| EngineError::Format(format!("{} frame: {e}", kind.name()));
    match kind {
        Kind::Apng => {
            let mut buf = Vec::new();
            {
                let mut enc = png::Encoder::new(&mut buf, w, h);
                enc.set_color(png::ColorType::Rgba);
                enc.set_depth(png::BitDepth::Eight);
                enc.set_adaptive_filter(png::AdaptiveFilterType::Adaptive);
                enc.set_compression(png::Compression::Best);
                let mut wr = enc.write_header().map_err(|e| fail(e.to_string()))?;
                wr.write_image_data(px).map_err(|e| fail(e.to_string()))?;
            }
            idat(&buf)
        }
        Kind::Webp => {
            if px.len() != w as usize * h as usize * 4 {
                return Err(fail("the frame has the wrong size".into()));
            }
            let mut buf = Vec::new();
            image_webp::WebPEncoder::new(&mut buf).encode(px, w, h, image_webp::ColorType::Rgba8).map_err(|e| fail(e.to_string()))?;
            riff_chunk(&buf, b"VP8L").map(<[u8]>::to_vec).ok_or_else(|| fail("the WebP encoder wrote no lossless image".into()))
        }
    }
}

fn encode(kind: Kind, before: Option<&[u8]>, cur: &[u8], w: u32, h: u32) -> Result<Encoded> {
    let (rect, over, px) = region(kind, before, cur, w, h);
    Ok(Encoded { rect, over, data: compress(kind, &px, rect.w, rect.h)? })
}

/// A frame that changes nothing (one transparent pixel blended over the top-left corner): how
/// a delay too long for one frame header goes on.
fn still(kind: Kind) -> Result<Encoded> {
    Ok(Encoded { rect: Rect { x: 0, y: 0, w: 1, h: 1 }, over: true, data: compress(kind, &[0, 0, 0, 0], 1, 1)? })
}

// ----- the files ---------------------------------------------------------------------------

fn png_chunk(out: &mut Vec<u8>, id: &[u8; 4], data: &[u8]) -> Result<()> {
    let len = u32::try_from(data.len()).ok().filter(|l| *l < 1 << 31).ok_or_else(|| EngineError::Format("an animated PNG frame is too large".into()))?;
    out.extend_from_slice(&len.to_be_bytes());
    let from = out.len();
    out.extend_from_slice(id);
    out.extend_from_slice(data);
    let crc = anicraft_xfl::zip::crc32(out.get(from..).unwrap_or(&[]));
    out.extend_from_slice(&crc.to_be_bytes());
    Ok(())
}

/// The APNG file: frames with their delays (the first frame is the default image).
fn apng_file(w: u32, h: u32, plays: u32, frames: &[(&Encoded, (u16, u16))]) -> Result<Vec<u8>> {
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA, deflate, adaptive filters, no interlace
    png_chunk(&mut out, b"IHDR", &ihdr)?;
    let count = u32::try_from(frames.len()).map_err(|_| EngineError::Format("too many frames".into()))?;
    let mut actl = count.to_be_bytes().to_vec();
    actl.extend_from_slice(&plays.to_be_bytes());
    png_chunk(&mut out, b"acTL", &actl)?;
    let mut seq = 0u32;
    for (k, (f, (num, den))) in frames.iter().enumerate() {
        let mut fctl = Vec::with_capacity(26);
        for v in [seq, f.rect.w, f.rect.h, f.rect.x, f.rect.y] {
            fctl.extend_from_slice(&v.to_be_bytes());
        }
        fctl.extend_from_slice(&num.to_be_bytes());
        fctl.extend_from_slice(&den.to_be_bytes());
        fctl.extend_from_slice(&[0, u8::from(f.over)]); // dispose: none; blend: source or over
        png_chunk(&mut out, b"fcTL", &fctl)?;
        seq += 1;
        if k == 0 {
            png_chunk(&mut out, b"IDAT", &f.data)?;
        } else {
            let mut fdat = Vec::with_capacity(f.data.len() + 4);
            fdat.extend_from_slice(&seq.to_be_bytes());
            fdat.extend_from_slice(&f.data);
            png_chunk(&mut out, b"fdAT", &fdat)?;
            seq += 1;
        }
    }
    png_chunk(&mut out, b"IEND", &[])?;
    Ok(out)
}

fn le24(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes()[..3]);
}

fn riff_push(out: &mut Vec<u8>, id: &[u8; 4], data: &[u8]) -> Result<()> {
    let len = u32::try_from(data.len()).map_err(|_| EngineError::Format("an animated WebP frame is too large".into()))?;
    out.extend_from_slice(id);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(data);
    if len & 1 == 1 {
        out.push(0);
    }
    Ok(())
}

/// The animated WebP file: frames with their durations. `background` is the stage colour as
/// the `ANIM` background (RGBA), or transparent.
fn webp_file(w: u32, h: u32, plays: u32, alpha: bool, background: [u8; 4], frames: &[(&Encoded, u32)]) -> Result<Vec<u8>> {
    let mut body = b"WEBP".to_vec();
    let mut vp8x = vec![if alpha { 0x10 | 0x02 } else { 0x02 }, 0, 0, 0];
    le24(&mut vp8x, w - 1);
    le24(&mut vp8x, h - 1);
    riff_push(&mut body, b"VP8X", &vp8x)?;
    let [r, g, b, a] = background;
    let mut anim = vec![b, g, r, a];
    anim.extend_from_slice(&u16::try_from(plays).unwrap_or(u16::MAX).to_le_bytes());
    riff_push(&mut body, b"ANIM", &anim)?;
    for (f, ms) in frames {
        let mut anmf = Vec::with_capacity(16 + f.data.len());
        le24(&mut anmf, f.rect.x / 2);
        le24(&mut anmf, f.rect.y / 2);
        le24(&mut anmf, f.rect.w - 1);
        le24(&mut anmf, f.rect.h - 1);
        le24(&mut anmf, *ms);
        anmf.push(if f.over { 0 } else { 0x02 }); // blending: alpha-blend or replace; dispose: none
        anmf.extend_from_slice(&f.data);
        riff_push(&mut body, b"ANMF", &anmf)?;
    }
    let mut out = Vec::with_capacity(body.len() + 8);
    riff_push(&mut out, b"RIFF", &body)?;
    Ok(out)
}

// ----- the command -------------------------------------------------------------------------

/// How many times it plays: `loop` true (or absent) forever (0), false once, or a count.
pub(crate) fn plays(p: &Value) -> Result<u32> {
    match p.get("loop") {
        None | Some(Value::Null) | Some(Value::Bool(true)) => Ok(0),
        Some(Value::Bool(false)) => Ok(1),
        Some(v) => v
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n <= MAX_PLAYS)
            .ok_or_else(|| EngineError::Param(format!("param 'loop' must be true (forever), false (once) or how many times it plays (0 is forever, at most {MAX_PLAYS})"))),
    }
}

/// `file.export_apng` and `file.export_webp`.
pub fn export(s: &mut Session, p: &Value, kind: Kind) -> Result<Value> {
    let path = PathBuf::from(str_req(p, "path")?);
    // The whole movie: every scene in order.
    let last = s.doc.frame_count().saturating_sub(1);
    let from = opt_u32(p, "from")?.unwrap_or(0).min(last);
    let to = opt_u32(p, "to")?.unwrap_or(last).min(last);
    if to < from {
        return Err(EngineError::Param("'to' must not be before 'from'".into()));
    }
    let scale = crate::cmds_file::scale_param(p)?;
    let plays = plays(p)?;
    let background = opt_bool(p, "background")?.unwrap_or(false);
    let mut view = View::stage(&s.doc, scale);
    if !background {
        view.background = None;
    }
    let (w, h) = (view.width.max(1), view.height.max(1));
    if kind == Kind::Webp && (w > MAX_WEBP_EDGE || h > MAX_WEBP_EDGE) {
        return Err(EngineError::Param(format!("WebP frames are at most {MAX_WEBP_EDGE} px per side; lower 'scale'")));
    }
    let n = u64::from(to - from) + 1;
    if n * u64::from(w) * u64::from(h) > MAX_ANIM_PIXELS {
        return Err(EngineError::Param(format!("that {} would be too large ({n} frames at {w}×{h}); lower 'scale' or the frame range", kind.name())));
    }
    let doc = &s.doc;
    let job = s.job.as_deref();
    let started = web_time::Instant::now();
    let batch = (BATCH_PIXELS / (u64::from(w) * u64::from(h))).clamp(2, 16) as usize;
    // Each distinct frame: its first frame (from 0) and how many frames it shows for.
    let mut shown: Vec<(u64, u64)> = Vec::new();
    let mut encoded: Vec<Encoded> = Vec::new();
    let mut prev: Option<Vec<u8>> = None;
    let frames: Vec<u32> = (from..=to).collect();
    for (b, chunk) in frames.chunks(batch).enumerate() {
        let images: Vec<Vec<u8>> = chunk
            .par_iter()
            .map(|&f| {
                let (pm, _) = render(&anicraft_doc::evaluate(doc, f), &view)?;
                crate::publish::tick(job, 1)?;
                Ok(pm.to_rgba8())
            })
            .collect::<Result<_>>()?;
        // A frame like the one before it only lengthens that frame's delay.
        let mut fresh: Vec<usize> = Vec::new();
        let mut last_seen: Option<&[u8]> = prev.as_deref();
        for (i, img) in images.iter().enumerate() {
            match shown.last_mut() {
                Some((_, count)) if last_seen == Some(img.as_slice()) => *count += 1,
                _ => {
                    shown.push(((b * batch + i) as u64, 1));
                    fresh.push(i);
                }
            }
            last_seen = Some(img.as_slice());
        }
        let made = (0..fresh.len())
            .into_par_iter()
            .map(|k| {
                let cur = fresh.get(k).and_then(|&i| images.get(i)).ok_or_else(|| EngineError::Format("lost a frame".into()))?;
                let before = match k.checked_sub(1) {
                    None => prev.as_deref(),
                    Some(j) => fresh.get(j).and_then(|&i| images.get(i)).map(Vec::as_slice),
                };
                encode(kind, before, cur, w, h)
            })
            .collect::<Result<Vec<_>>>()?;
        encoded.extend(made);
        if let Some(&i) = fresh.last() {
            prev = images.into_iter().nth(i);
        }
    }
    let ratio = fps_ratio(doc.stage.fps);
    let pad = still(kind)?;
    let bytes = match kind {
        Kind::Apng => {
            let mut list: Vec<(&Encoded, (u16, u16))> = Vec::new();
            for (e, &(start, count)) in encoded.iter().zip(&shown) {
                for (k, d) in apng_delays(start, count, ratio).into_iter().enumerate() {
                    list.push((if k == 0 { e } else { &pad }, d));
                }
            }
            apng_file(w, h, plays, &list)?
        }
        Kind::Webp => {
            let mut list: Vec<(&Encoded, u32)> = Vec::new();
            for (e, &(start, count)) in encoded.iter().zip(&shown) {
                for (k, d) in webp_durations(start, count, ratio).into_iter().enumerate() {
                    list.push((if k == 0 { e } else { &pad }, d));
                }
            }
            let padded = list.len() > encoded.len();
            let c = s.doc.stage.background.sanitized();
            let q = |v: f32| (v * 255.0).round() as u8;
            let bg = if background { [q(c.r), q(c.g), q(c.b), 255] } else { [0; 4] };
            webp_file(w, h, plays, !background || padded, bg, &list)?
        }
    };
    crate::vfs::write(&path, &bytes)?;
    Ok(json!({
        "written": path.display().to_string(),
        "frames": n,
        "stored_frames": encoded.len(),
        "bytes": bytes.len(),
        "width": w,
        "height": h,
        "loop": plays,
        "ms": started.elapsed().as_millis() as u64,
    }))
}

#[cfg(test)]
#[path = "anim_tests.rs"]
mod tests;
