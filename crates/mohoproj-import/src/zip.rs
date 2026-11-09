//! Reading files out of a `.moho` package (a zip archive holding `Project.mohoproj`).
//!
//! Implements the parts of the published ZIP format (PKWARE APPNOTE) that `.moho` files use —
//! the same subset the `.fla` reader takes, as the crates share the format family: the
//! end-of-central-directory record, central directory entries and local headers, with "stored"
//! and "deflate" entries (inflated by `miniz_oxide`). ZIP64 and encryption are refused with a
//! clear error. Every offset and size is checked; sizes are capped so a zip bomb can't exhaust
//! memory. Never panics.

use std::collections::HashMap;

/// Largest single entry inflated (bytes).
pub const MAX_ENTRY: usize = 256 * 1024 * 1024;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ZipError {
    #[error("not a zip archive (no end-of-directory record)")]
    NotZip,
    #[error("the archive is damaged: {0}")]
    Damaged(&'static str),
    #[error("'{0}' uses an unsupported zip feature ({1})")]
    Unsupported(String, &'static str),
    #[error("'{0}' is too large to read")]
    TooLarge(String),
}

#[derive(Debug, Clone)]
struct Entry {
    method: u16,
    flags: u16,
    comp: usize,
    size: usize,
    offset: usize,
}

pub struct Archive<'a> {
    data: &'a [u8],
    entries: HashMap<String, Entry>,
}

fn u16_at(d: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(i..i + 2)?.try_into().ok()?))
}
fn u32_at(d: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(i..i + 4)?.try_into().ok()?))
}

impl<'a> Archive<'a> {
    pub fn open(data: &'a [u8]) -> Result<Archive<'a>, ZipError> {
        // The end record is in the last 22 + 65535 bytes (it may carry a comment).
        let lo = data.len().saturating_sub(22 + 65_535);
        let eocd = (lo..data.len().saturating_sub(21)).rev().find(|&i| u32_at(data, i) == Some(0x0605_4b50)).ok_or(ZipError::NotZip)?;
        let count = u16_at(data, eocd + 10).ok_or(ZipError::Damaged("end record"))? as usize;
        let dir = u32_at(data, eocd + 16).ok_or(ZipError::Damaged("end record"))? as usize;
        if count == 0xffff || dir == 0xffff_ffff {
            return Err(ZipError::Unsupported("archive".into(), "ZIP64"));
        }
        let mut entries = HashMap::with_capacity(count.min(100_000));
        let mut i = dir;
        for _ in 0..count {
            if u32_at(data, i) != Some(0x0201_4b50) {
                return Err(ZipError::Damaged("central directory"));
            }
            let field = |o: usize| u32_at(data, i + o).ok_or(ZipError::Damaged("central directory"));
            let short = |o: usize| u16_at(data, i + o).ok_or(ZipError::Damaged("central directory"));
            let (flags, method) = (short(8)?, short(10)?);
            let (comp, size) = (field(20)? as usize, field(24)? as usize);
            let (nlen, xlen, clen) = (short(28)? as usize, short(30)? as usize, short(32)? as usize);
            let offset = field(42)? as usize;
            let name = data.get(i + 46..i + 46 + nlen).ok_or(ZipError::Damaged("entry name"))?;
            let name = String::from_utf8_lossy(name).replace('\\', "/");
            entries.insert(name, Entry { method, flags, comp, size, offset });
            i += 46 + nlen + xlen + clen;
        }
        Ok(Archive { data, entries })
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    /// The bytes of entry `name` (path inside the archive, `/`-separated).
    pub fn read(&self, name: &str) -> Result<Option<Vec<u8>>, ZipError> {
        let Some(e) = self.entries.get(name) else { return Ok(None) };
        if e.flags & 1 != 0 {
            return Err(ZipError::Unsupported(name.into(), "encryption"));
        }
        if e.size > MAX_ENTRY || e.comp > MAX_ENTRY {
            return Err(ZipError::TooLarge(name.into()));
        }
        let d = self.data;
        if u32_at(d, e.offset) != Some(0x0403_4b50) {
            return Err(ZipError::Damaged("local header"));
        }
        let nlen = u16_at(d, e.offset + 26).ok_or(ZipError::Damaged("local header"))? as usize;
        let xlen = u16_at(d, e.offset + 28).ok_or(ZipError::Damaged("local header"))? as usize;
        let start = e.offset + 30 + nlen + xlen;
        let raw = d.get(start..start + e.comp).ok_or(ZipError::Damaged("entry data"))?;
        match e.method {
            0 => Ok(Some(raw.to_vec())),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(raw, e.size.clamp(1, MAX_ENTRY)).map(Some).map_err(|_| ZipError::Damaged("deflate data")),
            _ => Err(ZipError::Unsupported(name.into(), "compression method")),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Builds a zip with stored and deflated entries (for tests).
    pub fn make_zip(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut dir = Vec::new();
        for (name, body, deflate) in files {
            let data = if *deflate { miniz_oxide::deflate::compress_to_vec(body, 6) } else { body.to_vec() };
            let offset = out.len() as u32;
            let method: u16 = if *deflate { 8 } else { 0 };
            out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            out.extend_from_slice(&[20, 0, 0, 0]);
            out.extend_from_slice(&method.to_le_bytes());
            out.extend_from_slice(&[0; 8]); // time, date, crc (not checked)
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(body.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&data);
            dir.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            dir.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
            dir.extend_from_slice(&method.to_le_bytes());
            dir.extend_from_slice(&[0; 8]);
            dir.extend_from_slice(&(data.len() as u32).to_le_bytes());
            dir.extend_from_slice(&(body.len() as u32).to_le_bytes());
            dir.extend_from_slice(&(name.len() as u16).to_le_bytes());
            dir.extend_from_slice(&[0; 12]);
            dir.extend_from_slice(&offset.to_le_bytes());
            dir.extend_from_slice(name.as_bytes());
        }
        let dir_at = out.len() as u32;
        out.extend_from_slice(&dir);
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(dir.len() as u32).to_le_bytes());
        out.extend_from_slice(&dir_at.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out
    }

    #[test]
    fn reads_stored_and_deflated_entries() {
        let z = make_zip(&[("DOMDocument.xml", b"<DOMDocument/>", true), ("LIBRARY\\Ball.xml", b"<DOMSymbolItem/>", false)]);
        let a = Archive::open(&z).unwrap();
        assert_eq!(a.read("DOMDocument.xml").unwrap().unwrap(), b"<DOMDocument/>");
        assert_eq!(a.read("LIBRARY/Ball.xml").unwrap().unwrap(), b"<DOMSymbolItem/>", "backslashes normalised");
        assert_eq!(a.read("missing").unwrap(), None);
    }

    #[test]
    fn damaged_archives_are_errors() {
        assert_eq!(Archive::open(b"not a zip").err(), Some(ZipError::NotZip));
        let mut z = make_zip(&[("a", b"hello", true)]);
        let n = z.len();
        // Point the directory past the end.
        z[n - 6] = 0xff;
        assert!(Archive::open(&z).is_err());
    }
}
