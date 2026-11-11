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
    #[serde(skip)]
    decoded: Arc<OnceLock<Option<Arc<Pixels>>>>,
}

impl PartialEq for Bitmap {
    fn eq(&self, o: &Bitmap) -> bool {
        self.id == o.id && self.name == o.name && self.width == o.width && self.height == o.height && (Arc::ptr_eq(&self.png, &o.png) || self.png == o.png)
    }
}

impl Bitmap {
    /// A bitmap from PNG data. `None` when it isn't a readable PNG or is too large.
    pub fn from_png(id: BitmapId, name: impl Into<String>, png: Vec<u8>) -> Option<Bitmap> {
        let px = decode_png(&png)?;
        let (width, height) = (px.width, px.height);
        let decoded = OnceLock::new();
        let _ = decoded.set(Some(Arc::new(px)));
        Some(Bitmap { id, name: name.into(), width, height, png: Arc::new(png), decoded: Arc::new(decoded) })
    }

    /// A bitmap from PNG or JPEG data (told apart by their signatures; JPEGs become PNG).
    pub fn from_image(id: BitmapId, name: impl Into<String>, data: Vec<u8>) -> Result<Bitmap, ImageError> {
        let png = image_to_png(data)?;
        Bitmap::from_png(id, name, png).ok_or(ImageError::Unreadable)
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
        png::ColorType::Rgba => {
            buf.truncate(n * 4); // no second copy of a big image
            if buf.len() != n * 4 {
                return None;
            }
            std::mem::take(&mut buf)
        }
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
    // The size comes from the header, so a huge JPEG is refused before any pixel buffer exists.
    dec.decode_headers().map_err(|_| ImageError::Unreadable)?;
    let (w, h) = dec.dimensions().ok_or(ImageError::Unreadable)?;
    let (w, h) = (u32::try_from(w).map_err(|_| ImageError::BadSize)?, u32::try_from(h).map_err(|_| ImageError::BadSize)?);
    if w == 0 || h == 0 || w > MAX_BITMAP_EDGE || h > MAX_BITMAP_EDGE {
        return Err(ImageError::BadSize);
    }
    let rgba = dec.decode().map_err(|_| ImageError::Unreadable)?;
    if rgba.len() != w as usize * h as usize * 4 {
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

    /// An 8x8 grey baseline JPEG.
    #[rustfmt::skip]
    const TINY_JPEG: &[u8] = &[0xff,0xd8,0xff,0xe0,0x00,0x10,0x4a,0x46,0x49,0x46,0x00,0x01,0x01,0x00,0x00,0x01,0x00,0x01,0x00,0x00,0xff,0xdb,0x00,0x43,0x00,0x10,0x0b,0x0c,0x0e,0x0c,0x0a,0x10,0x0e,0x0d,0x0e,0x12,0x11,0x10,0x13,0x18,0x28,0x1a,0x18,0x16,0x16,0x18,0x31,0x23,0x25,0x1d,0x28,0x3a,0x33,0x3d,0x3c,0x39,0x33,0x38,0x37,0x40,0x48,0x5c,0x4e,0x40,0x44,0x57,0x45,0x37,0x38,0x50,0x6d,0x51,0x57,0x5f,0x62,0x67,0x68,0x67,0x3e,0x4d,0x71,0x79,0x70,0x64,0x78,0x5c,0x65,0x67,0x63,0xff,0xc0,0x00,0x0b,0x08,0x00,0x08,0x00,0x08,0x01,0x01,0x11,0x00,0xff,0xc4,0x00,0x1f,0x00,0x00,0x01,0x05,0x01,0x01,0x01,0x01,0x01,0x01,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x01,0x02,0x03,0x04,0x05,0x06,0x07,0x08,0x09,0x0a,0x0b,0xff,0xc4,0x00,0xb5,0x10,0x00,0x02,0x01,0x03,0x03,0x02,0x04,0x03,0x05,0x05,0x04,0x04,0x00,0x00,0x01,0x7d,0x01,0x02,0x03,0x00,0x04,0x11,0x05,0x12,0x21,0x31,0x41,0x06,0x13,0x51,0x61,0x07,0x22,0x71,0x14,0x32,0x81,0x91,0xa1,0x08,0x23,0x42,0xb1,0xc1,0x15,0x52,0xd1,0xf0,0x24,0x33,0x62,0x72,0x82,0x09,0x0a,0x16,0x17,0x18,0x19,0x1a,0x25,0x26,0x27,0x28,0x29,0x2a,0x34,0x35,0x36,0x37,0x38,0x39,0x3a,0x43,0x44,0x45,0x46,0x47,0x48,0x49,0x4a,0x53,0x54,0x55,0x56,0x57,0x58,0x59,0x5a,0x63,0x64,0x65,0x66,0x67,0x68,0x69,0x6a,0x73,0x74,0x75,0x76,0x77,0x78,0x79,0x7a,0x83,0x84,0x85,0x86,0x87,0x88,0x89,0x8a,0x92,0x93,0x94,0x95,0x96,0x97,0x98,0x99,0x9a,0xa2,0xa3,0xa4,0xa5,0xa6,0xa7,0xa8,0xa9,0xaa,0xb2,0xb3,0xb4,0xb5,0xb6,0xb7,0xb8,0xb9,0xba,0xc2,0xc3,0xc4,0xc5,0xc6,0xc7,0xc8,0xc9,0xca,0xd2,0xd3,0xd4,0xd5,0xd6,0xd7,0xd8,0xd9,0xda,0xe1,0xe2,0xe3,0xe4,0xe5,0xe6,0xe7,0xe8,0xe9,0xea,0xf1,0xf2,0xf3,0xf4,0xf5,0xf6,0xf7,0xf8,0xf9,0xfa,0xff,0xda,0x00,0x08,0x01,0x01,0x00,0x00,0x3f,0x00,0xa3,0x5f,0xff,0xd9];

    #[test]
    fn jpeg_size_is_checked_from_the_header() {
        let png = image_to_png(TINY_JPEG.to_vec()).unwrap();
        assert_eq!(decode_png(&png).map(|p| (p.width, p.height)), Some((8, 8)), "a valid JPEG still imports");
        // Same file, but the frame header claims 16000 x 16000 (about 1 GB of RGBA): refused as BadSize
        // from the header, before any pixel buffer is made.
        let mut big = TINY_JPEG.to_vec();
        let sof = big.windows(2).position(|w| w == [0xff, 0xc0]).unwrap();
        big[sof + 5..sof + 9].copy_from_slice(&[0x3e, 0x80, 0x3e, 0x80]);
        assert_eq!(image_to_png(big).err(), Some(ImageError::BadSize));
    }
}
