//! Bitmaps: images in the document, kept as PNG data (what is saved) and decoded once, on first
//! use, into straight-alpha RGBA8 pixels for rendering. Shapes show them through
//! [`crate::Paint::Bitmap`]; an imported image is a rectangle filled with its bitmap.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::sync::{Arc, OnceLock};

/// Unique id of a bitmap in the document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BitmapId(pub u64);

/// Largest bitmap edge accepted (pixels).
pub const MAX_BITMAP_EDGE: u32 = 8192;

/// Decoded pixels: `width × height` straight-alpha RGBA8, row by row.
#[derive(Debug, PartialEq)]
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Pixels {
    /// The pixel at `(x, y)`, clamped to the edges, as straight RGBA in `0..=1`.
    pub fn at(&self, x: i64, y: i64) -> [f32; 4] {
        let (w, h) = (i64::from(self.width.max(1)), i64::from(self.height.max(1)));
        let (x, y) = (x.clamp(0, w - 1), y.clamp(0, h - 1));
        let i = ((y * w + x) * 4) as usize;
        match self.rgba.get(i..i + 4) {
            Some(p) => [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0, p[3] as f32 / 255.0],
            None => [0.0; 4],
        }
    }
}

/// A bitmap in the document.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Bitmap {
    pub id: BitmapId,
    pub name: String,
    pub width: u32,
    pub height: u32,
    /// The image as PNG.
    #[serde(with = "base64_bytes")]
    pub png: Arc<Vec<u8>>,
    /// Allow Smoothing (ADR 0050): scaled or turned, the image is filtered between its pixels;
    /// off, each pixel stays a hard-edged square (pixel art). On unless a file says otherwise.
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub smoothing: bool,
    #[serde(skip)]
    decoded: Arc<OnceLock<Option<Arc<Pixels>>>>,
}

fn yes() -> bool {
    true
}

fn is_yes(b: &bool) -> bool {
    *b
}

impl PartialEq for Bitmap {
    fn eq(&self, o: &Bitmap) -> bool {
        self.id == o.id && self.name == o.name && self.width == o.width && self.height == o.height && self.smoothing == o.smoothing && (Arc::ptr_eq(&self.png, &o.png) || self.png == o.png)
    }
}

impl Bitmap {
    /// A bitmap from PNG data. `None` when it isn't a readable PNG or is too large.
    pub fn from_png(id: BitmapId, name: impl Into<String>, png: Vec<u8>) -> Option<Bitmap> {
        let px = decode_png(&png)?;
        let (width, height) = (px.width, px.height);
        let decoded = OnceLock::new();
        let _ = decoded.set(Some(Arc::new(px)));
        Some(Bitmap { id, name: name.into(), width, height, png: Arc::new(png), smoothing: true, decoded: Arc::new(decoded) })
    }

    /// A bitmap from PNG or JPEG data (told apart by their signatures; JPEGs become PNG).
    pub fn from_image(id: BitmapId, name: impl Into<String>, data: Vec<u8>) -> Result<Bitmap, ImageError> {
        let png = image_to_png(data)?;
        Bitmap::from_png(id, name, png).ok_or(ImageError::Unreadable)
    }

    /// A bitmap from straight RGBA8 pixels, row by row (a decoded video frame, say). `None`
    /// when the size is empty, over [`MAX_BITMAP_EDGE`] or doesn't match the pixels.
    pub fn from_rgba(id: BitmapId, name: impl Into<String>, width: u32, height: u32, rgba: Vec<u8>) -> Option<Bitmap> {
        if width == 0 || height == 0 || width > MAX_BITMAP_EDGE || height > MAX_BITMAP_EDGE || rgba.len() != width as usize * height as usize * 4 {
            return None;
        }
        let png = encode_png(width, height, &rgba)?;
        let decoded = OnceLock::new();
        let _ = decoded.set(Some(Arc::new(Pixels { width, height, rgba })));
        Some(Bitmap { id, name: name.into(), width, height, png: Arc::new(png), smoothing: true, decoded: Arc::new(decoded) })
    }

    /// The decoded pixels (decoded once and shared by every clone). `None` for unreadable data.
    pub fn pixels(&self) -> Option<Arc<Pixels>> {
        self.decoded.get_or_init(|| decode_png(&self.png).map(Arc::new)).clone()
    }
}

/// Decodes PNG data to straight RGBA8, whatever its colour type and bit depth.
pub fn decode_png(data: &[u8]) -> Option<Pixels> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(data));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = dec.read_info().ok()?;
    let (w, h) = reader.info().size();
    if w == 0 || h == 0 || w > MAX_BITMAP_EDGE || h > MAX_BITMAP_EDGE {
        return None;
    }
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let n = (w * h) as usize;
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => buf.get(..n * 4)?.to_vec(),
        png::ColorType::Rgb => buf.get(..n * 3)?.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf.get(..n * 2)?.as_chunks::<2>().0.iter().flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => buf.get(..n)?.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return None, // expanded by normalize_to_color8
    };
    Some(Pixels { width: w, height: h, rgba })
}

/// Why image data can't become a bitmap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageError {
    /// Neither PNG nor JPEG.
    UnknownFormat,
    /// A PNG or JPEG that doesn't decode.
    Unreadable,
    /// Empty, or wider or taller than [`MAX_BITMAP_EDGE`].
    BadSize,
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ImageError::UnknownFormat => "not a PNG or JPEG image",
            ImageError::Unreadable => "a damaged or unsupported image",
            ImageError::BadSize => "the image is empty or larger than 8192 px a side",
        })
    }
}

impl std::error::Error for ImageError {}

/// PNG or JPEG data as PNG: PNG passes through; JPEG is decoded and re-encoded losslessly.
pub fn image_to_png(data: Vec<u8>) -> Result<Vec<u8>, ImageError> {
    if data.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        return Ok(data);
    }
    if !data.starts_with(&[0xff, 0xd8, 0xff]) {
        return Err(ImageError::UnknownFormat);
    }
    use zune_jpeg::zune_core::{bytestream::ZCursor, colorspace::ColorSpace, options::DecoderOptions};
    let opts = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGBA);
    let mut dec = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(&data), opts);
    let rgba = dec.decode().map_err(|_| ImageError::Unreadable)?;
    let (w, h) = dec.dimensions().ok_or(ImageError::Unreadable)?;
    let (w, h) = (u32::try_from(w).map_err(|_| ImageError::BadSize)?, u32::try_from(h).map_err(|_| ImageError::BadSize)?);
    if w == 0 || h == 0 || w > MAX_BITMAP_EDGE || h > MAX_BITMAP_EDGE || rgba.len() != w as usize * h as usize * 4 {
        return Err(ImageError::BadSize);
    }
    encode_png(w, h, &rgba).ok_or(ImageError::Unreadable)
}

/// Straight RGBA8 pixels as PNG.
fn encode_png(w: u32, h: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header().ok()?;
        wr.write_image_data(rgba).ok()?;
    }
    Some(out)
}

/// Bytes as base64 text in files.
pub(crate) mod base64_bytes {
    use super::*;

    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn encode(data: &[u8]) -> String {
        let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
        for chunk in data.chunks(3) {
            let b = [chunk.first().copied().unwrap_or(0), chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
            let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
            for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
                if i <= chunk.len() {
                    out.push(ALPHABET[((n >> shift) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    pub fn decode(text: &str) -> Option<Vec<u8>> {
        let val = |c: u8| ALPHABET.iter().position(|&a| a == c).map(|v| v as u32);
        let clean: Vec<u8> = text.bytes().filter(|c| !c.is_ascii_whitespace()).collect();
        if !clean.len().is_multiple_of(4) {
            return None;
        }
        let mut out = Vec::with_capacity(clean.len() / 4 * 3);
        for q in clean.chunks(4) {
            let pad = q.iter().rev().take_while(|&&c| c == b'=').count();
            if pad > 2 {
                return None;
            }
            let mut n = 0u32;
            for (i, &c) in q.iter().enumerate() {
                n <<= 6;
                if i < 4 - pad {
                    n |= val(c)?;
                }
            }
            let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
            out.extend_from_slice(bytes.get(..3 - pad)?);
        }
        Some(out)
    }

    pub fn serialize<S: Serializer>(data: &Arc<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&encode(data))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Arc<Vec<u8>>, D::Error> {
        let text = String::deserialize(d)?;
        decode(&text).map(Arc::new).ok_or_else(|| serde::de::Error::custom("bitmap data is not valid base64"))
    }
}

pub use base64_bytes::decode as base64_decode;
pub use base64_bytes::encode as base64_encode;

#[cfg(test)]
mod tests {
    use super::*;

    fn png_of(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut wr = enc.write_header().unwrap();
            wr.write_image_data(rgba).unwrap();
        }
        out
    }

    #[test]
    fn base64_round_trips() {
        for data in [&b""[..], b"f", b"fo", b"foo", b"foob", b"fooba", b"foobar", &[0, 255, 128, 7, 9]] {
            let e = base64_bytes::encode(data);
            assert_eq!(base64_bytes::decode(&e).as_deref(), Some(data), "{e}");
        }
        assert_eq!(base64_bytes::encode(b"foobar"), "Zm9vYmFy");
        assert!(base64_bytes::decode("abc").is_none());
        assert!(base64_bytes::decode("ab$=").is_none());
    }

    #[test]
    fn bitmaps_decode_once_and_survive_a_save() {
        let px = [255, 0, 0, 255, 0, 255, 0, 128];
        let b = Bitmap::from_png(BitmapId(3), "two", png_of(2, 1, &px)).unwrap();
        assert_eq!((b.width, b.height), (2, 1));
        let p = b.pixels().unwrap();
        assert_eq!(p.rgba, px);
        assert_eq!(p.at(1, 0), [0.0, 1.0, 0.0, 128.0 / 255.0]);
        assert_eq!(p.at(-5, 9), [1.0, 0.0, 0.0, 1.0], "clamped to the edge");
        let json = serde_json::to_string(&b).unwrap();
        let back: Bitmap = serde_json::from_str(&json).unwrap();
        assert_eq!(back, b);
        assert_eq!(back.pixels().unwrap().rgba, px, "decoded again after loading");
        assert!(Bitmap::from_png(BitmapId(1), "x", b"not a png".to_vec()).is_none());
        // Images in general: PNG passes through, other formats are told apart from damage.
        assert_eq!(Bitmap::from_image(BitmapId(4), "y", png_of(2, 1, &px)).unwrap().pixels().unwrap().rgba, px);
        assert_eq!(image_to_png(b"GIF89a".to_vec()).err(), Some(ImageError::UnknownFormat));
        assert_eq!(image_to_png(vec![0xff, 0xd8, 0xff, 0xe0, 0, 1]).err(), Some(ImageError::Unreadable));
    }
}
