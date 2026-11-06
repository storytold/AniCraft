//! Immutable straight-alpha RGBA8 assets, with bounded PNG/JPEG decoding.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::io::Cursor;
use zune_core::{bytestream::ZCursor, colorspace::ColorSpace, options::DecoderOptions};

pub const MAX_PIXELS: u64 = 4096 * 4096;
pub const MAX_EDGE: u32 = 16_384;
pub const MAX_INPUT_BYTES: usize = 80 * 1024 * 1024;

/// Validated pixels; fields stay private so renderers can rely on dimensions and byte length.
#[derive(Clone, Debug, PartialEq)]
pub struct Bitmap {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

fn dimensions(width: u32, height: u32) -> Result<usize, String> {
    let pixels = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 || width > MAX_EDGE || height > MAX_EDGE || pixels > MAX_PIXELS {
        return Err(format!("bitmap {width} × {height} exceeds the limit of {MAX_EDGE} px per side or {MAX_PIXELS} pixels"));
    }
    Ok(pixels as usize * 4)
}

impl Bitmap {
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, String> {
        if rgba.len() != dimensions(width, height)? {
            return Err("bitmap RGBA byte count does not match its dimensions".into());
        }
        Ok(Self { width, height, rgba })
    }
    pub fn width(&self) -> u32 { self.width }
    pub fn height(&self) -> u32 { self.height }
    pub fn rgba(&self) -> &[u8] { &self.rgba }
    pub fn bytes(&self) -> usize { self.rgba.len() }
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        if x >= self.width || y >= self.height { return [0; 4] }
        let start = (y as usize * self.width as usize + x as usize) * 4;
        self.rgba.get(start..start + 4).and_then(|p| p.try_into().ok()).unwrap_or([0; 4])
    }

    pub fn encode_png(&self) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, self.width, self.height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
            writer.write_image_data(&self.rgba).map_err(|e| e.to_string())?;
        }
        Ok(bytes)
    }
}

/// Detect by signature, not the filename. Reject unsupported and animated formats explicitly.
pub fn decode(bytes: &[u8]) -> Result<Bitmap, String> {
    if bytes.len() > MAX_INPUT_BYTES { return Err("image file exceeds the 80 MiB input limit".into()) }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        decode_png(bytes)
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        if !bytes.ends_with(&[0xff, 0xd9]) { return Err("truncated JPEG: missing end marker".into()) }
        let options = DecoderOptions::default().set_max_width(MAX_EDGE as usize).set_max_height(MAX_EDGE as usize).set_use_unsafe(false).jpeg_set_out_colorspace(ColorSpace::RGB);
        let mut decoder = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), options);
        decoder.decode_headers().map_err(|e| e.to_string())?;
        let (w, h) = decoder.dimensions().ok_or("JPEG has no dimensions")?;
        let capacity = dimensions(w as u32, h as u32)?;
        let rgb = decoder.decode().map_err(|e| e.to_string())?;
        if rgb.len() != capacity / 4 * 3 { return Err("JPEG decoder returned an unexpected colour layout".into()) }
        let mut rgba = Vec::with_capacity(capacity);
        for p in rgb.chunks_exact(3) { rgba.extend_from_slice(&[p[0], p[1], p[2], 255]); }
        let image = Bitmap::new(w as u32, h as u32, rgba)?;
        Ok(orient(image, decoder.exif().and_then(|b| exif_orientation(b)).unwrap_or(1)))
    } else {
        Err("unsupported image: choose a PNG or JPEG file".into())
    }
}

// Read only TIFF's orientation entry; metadata offsets are bounded before every access.
fn exif_orientation(bytes: &[u8]) -> Option<u16> {
    let little = match bytes.get(..2)? { b"II" => true, b"MM" => false, _ => return None };
    let u16_at = |at: usize| -> Option<u16> {
        let b: [u8; 2] = bytes.get(at..at.checked_add(2)?)?.try_into().ok()?;
        Some(if little { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    };
    let u32_at = |at: usize| -> Option<u32> {
        let b: [u8; 4] = bytes.get(at..at.checked_add(4)?)?.try_into().ok()?;
        Some(if little { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    };
    if u16_at(2)? != 42 { return None }
    let offset = usize::try_from(u32_at(4)?).ok()?;
    let count = u16_at(offset)?;
    for i in 0..usize::from(count).min(4096) {
        let at = offset.checked_add(2)?.checked_add(i.checked_mul(12)?)?;
        if u16_at(at)? == 0x0112 && u16_at(at + 2)? == 3 && u32_at(at + 4)? == 1 {
            return u16_at(at + 8).filter(|o| (1..=8).contains(o));
        }
    }
    None
}

fn orient(image: Bitmap, orientation: u16) -> Bitmap {
    if !(2..=8).contains(&orientation) { return image }
    let (w, h) = (image.width, image.height);
    let (width, height) = if orientation >= 5 { (h, w) } else { (w, h) };
    let mut rgba = Vec::with_capacity(image.bytes());
    for y in 0..height {
        for x in 0..width {
            let (sx, sy) = match orientation {
                2 => (w - 1 - x, y), 3 => (w - 1 - x, h - 1 - y), 4 => (x, h - 1 - y),
                5 => (y, x), 6 => (y, h - 1 - x), 7 => (w - 1 - y, h - 1 - x), 8 => (w - 1 - y, x),
                _ => (x, y),
            };
            rgba.extend_from_slice(&image.pixel(sx, sy));
        }
    }
    Bitmap { width, height, rgba }
}

fn decode_png(bytes: &[u8]) -> Result<Bitmap, String> {
    let mut decoder = png::Decoder::new_with_limits(Cursor::new(bytes), png::Limits { bytes: 128 * 1024 * 1024 });
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let (w, h) = (reader.info().width, reader.info().height);
    let capacity = dimensions(w, h)?;
    if reader.info().animation_control.is_some() { return Err("animated PNG is not supported; import a still image".into()) }
    let mut raw = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut raw).map_err(|e| e.to_string())?;
    let channels = info.color_type.samples();
    let mut rgba = Vec::with_capacity(capacity);
    for p in raw.get(..info.buffer_size()).ok_or("invalid PNG output length")?.chunks_exact(channels) {
        match p {
            [r, g, b, a] => rgba.extend_from_slice(&[*r, *g, *b, *a]),
            [r, g, b] => rgba.extend_from_slice(&[*r, *g, *b, 255]),
            [g, a] => rgba.extend_from_slice(&[*g, *g, *g, *a]),
            [g] => rgba.extend_from_slice(&[*g, *g, *g, 255]),
            _ => return Err("unsupported PNG colour layout".into()),
        }
    }
    Bitmap::new(w, h, rgba)
}

/// Canonical embedded representation: lossless PNG in base64, independent of source files.
impl Serialize for Bitmap {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let png = self.encode_png().map_err(serde::ser::Error::custom)?;
        serializer.serialize_str(&STANDARD.encode(png))
    }
}

impl<'de> Deserialize<'de> for Bitmap {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let encoded = String::deserialize(deserializer)?;
        if encoded.len() > MAX_INPUT_BYTES.div_ceil(3) * 4 { return Err(serde::de::Error::custom("embedded image is too large")) }
        let bytes = STANDARD.decode(encoded).map_err(serde::de::Error::custom)?;
        decode_png(&bytes).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba_png_round_trip_and_native_embedding() {
        let image = Bitmap::new(2, 1, vec![255, 32, 0, 128, 0, 0, 0, 0]).unwrap();
        assert_eq!(decode(&image.encode_png().unwrap()).unwrap(), image);
        let encoded = serde_json::to_string(&image).unwrap();
        assert_eq!(serde_json::from_str::<Bitmap>(&encoded).unwrap(), image);
        assert!(serde_json::from_str::<Bitmap>("\"not base64\"").is_err());
    }

    #[test]
    fn indexed_and_sixteen_bit_pngs_expand_to_rgba() {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
            encoder.set_color(png::ColorType::Indexed);
            encoder.set_depth(png::BitDepth::One);
            encoder.set_palette(vec![255, 0, 0, 0, 255, 0]);
            encoder.set_trns(vec![255, 64]);
            encoder.write_header().unwrap().write_image_data(&[0b0100_0000]).unwrap();
        }
        assert_eq!(decode(&bytes).unwrap().rgba(), &[255, 0, 0, 255, 0, 255, 0, 64]);
        bytes.clear();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
            encoder.set_color(png::ColorType::GrayscaleAlpha);
            encoder.set_depth(png::BitDepth::Sixteen);
            encoder.write_header().unwrap().write_image_data(&[0x80, 0, 0x40, 0]).unwrap();
        }
        assert_eq!(decode(&bytes).unwrap().rgba(), &[128, 128, 128, 64]);
    }

    // Original minimal 8×8 grayscale JPEG: quantization = 1, zero DC/AC coefficients,
    // one-bit codes for DC category 0 and EOB. No third-party image fixture.
    fn jpeg() -> Vec<u8> {
        let mut bytes = vec![0xff, 0xd8];
        let mut segment = |marker: u8, payload: &[u8]| {
            bytes.extend_from_slice(&[0xff, marker]);
            bytes.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
            bytes.extend_from_slice(payload);
        };
        let mut quantization = vec![0];
        quantization.extend([1; 64]);
        segment(0xdb, &quantization);
        segment(0xc0, &[8, 0, 8, 0, 8, 1, 1, 0x11, 0]);
        for table in [0, 0x10] {
            let mut huffman = vec![table, 1];
            huffman.extend([0; 15]);
            huffman.push(0);
            segment(0xc4, &huffman);
        }
        segment(0xda, &[1, 1, 0, 0, 63, 0]);
        bytes.extend_from_slice(&[0x3f, 0xff, 0xd9]);
        bytes
    }

    #[test]
    fn jpeg_decodes_and_hostile_files_are_errors() {
        let bytes = jpeg();
        let image = decode(&bytes).unwrap();
        assert_eq!((image.width(), image.height()), (8, 8));
        assert!(image.rgba().chunks_exact(4).all(|p| p == [128, 128, 128, 255]));
        for end in 0..bytes.len() - 2 { assert!(decode(&bytes[..end]).is_err(), "truncated at {end}"); }
        assert!(Bitmap::new(0, 1, vec![]).is_err());
        assert!(Bitmap::new(MAX_EDGE, MAX_EDGE, vec![]).is_err());
        assert!(Bitmap::new(2, 2, vec![0; 3]).is_err());
        assert!(decode(b"GIF89a").is_err());
        // Header dimensions are checked before allocating decoded pixels.
        let mut bytes = Vec::new();
        png::Encoder::new(&mut bytes, 16_384, 16_384).write_header().unwrap().finish().unwrap();
        assert!(decode(&bytes).is_err());
    }

    #[test]
    fn exif_orientation_rotates_pixels_and_checks_offsets() {
        let image = Bitmap::new(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 255]).unwrap();
        let rotated = orient(image.clone(), 6);
        assert_eq!((rotated.width(), rotated.height()), (1, 2));
        assert_eq!(rotated.pixel(0, 0), image.pixel(0, 0));
        assert_eq!(rotated.pixel(0, 1), image.pixel(1, 0));
        for o in 1..=8 { assert_eq!(orient(image.clone(), o).bytes(), image.bytes()); }
        let exif = [b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 1, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0];
        assert_eq!(exif_orientation(&exif), Some(6));
        for end in 0..18 { assert_eq!(exif_orientation(&exif[..end]), None); }
        let mut bad = exif;
        bad[4..8].fill(255);
        assert_eq!(exif_orientation(&bad), None);
    }
}
