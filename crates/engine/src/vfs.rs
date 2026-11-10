//! The files commands read and write (ADR 0036).
//!
//! On the desktop this is the operating system's file system: every function here is the
//! `std::fs` call it names, with nothing in between. In the browser (`wasm32`) there is no file
//! system, so commands read and write an in-memory store instead. The web shell puts the file
//! the user picked into it before a command runs (`file.open {"path": "walk.anicraft"}`) and
//! hands what the command wrote to the browser as downloads afterwards ([`take_written`]).
//! Commands keep taking paths, so one implementation serves the editor, the CLI, MCP and the
//! web build.
//!
//! The store is per thread (the browser has one), bounded ([`MAX_MEMORY_BYTES`]), and tests can
//! switch a thread to it on any platform with [`use_memory`]. So commands read and write files
//! on their own thread, never from the rayon pool (render there, write after).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io;
use std::path::{Component, Path, PathBuf};

/// Most bytes the in-memory store holds at once (wasm32 has 4 GiB of address space in all).
pub const MAX_MEMORY_BYTES: u64 = 1 << 30;

/// Most path components a stored file may have.
const MAX_DEPTH: usize = 64;

/// One file in the store, and who put it there.
struct MemFile {
    data: Vec<u8>,
    /// Written by a command (an export or a save), as opposed to put there by the front end.
    written: bool,
}

/// An in-memory file system: files by normalized path; folders exist where files are under them.
#[derive(Default)]
pub struct MemFs {
    files: BTreeMap<PathBuf, MemFile>,
    bytes: u64,
}

fn unsupported(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, what.to_string())
}

/// `a/./b/../c` → `a/c`, `/x` → `x`: one spelling per file. `..` above the root is refused.
fn key(path: &Path) -> io::Result<PathBuf> {
    let mut out = PathBuf::new();
    let mut depth = 0usize;
    for c in path.components() {
        match c {
            Component::Normal(n) => {
                depth += 1;
                if depth > MAX_DEPTH {
                    return Err(unsupported("the path is too deep"));
                }
                out.push(n);
            }
            Component::ParentDir => {
                if !out.pop() {
                    return Err(unsupported("the path leaves the root folder"));
                }
                depth = depth.saturating_sub(1);
            }
            Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
        }
    }
    if out.as_os_str().is_empty() {
        return Err(unsupported("the path names no file"));
    }
    Ok(out)
}

fn not_found(path: &Path) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, format!("{} was not found", path.display()))
}

impl MemFs {
    fn store(&mut self, path: &Path, data: Vec<u8>, written: bool) -> io::Result<()> {
        let k = key(path)?;
        let old = self.files.get(&k).map_or(0, |f| f.data.len() as u64);
        let total = self.bytes - old + data.len() as u64;
        if total > MAX_MEMORY_BYTES {
            return Err(io::Error::new(io::ErrorKind::OutOfMemory, format!("{} does not fit in memory ({} MB at most)", path.display(), MAX_MEMORY_BYTES >> 20)));
        }
        if self.is_dir(path) {
            return Err(unsupported("a folder has that name"));
        }
        self.bytes = total;
        self.files.insert(k, MemFile { data, written });
        Ok(())
    }

    /// A file from outside (the user picked it): read by commands, never handed back.
    pub fn put(&mut self, path: &Path, data: Vec<u8>) -> io::Result<()> {
        self.store(path, data, false)
    }

    /// A file a command writes (handed back by [`MemFs::take_written`]).
    pub fn write(&mut self, path: &Path, data: Vec<u8>) -> io::Result<()> {
        self.store(path, data, true)
    }

    pub fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.files.get(&key(path)?).map(|f| f.data.clone()).ok_or_else(|| not_found(path))
    }

    pub fn len(&self, path: &Path) -> io::Result<u64> {
        self.files.get(&key(path)?).map(|f| f.data.len() as u64).ok_or_else(|| not_found(path))
    }

    pub fn is_file(&self, path: &Path) -> bool {
        key(path).is_ok_and(|k| self.files.contains_key(&k))
    }

    /// A folder exists while a file is inside it.
    pub fn is_dir(&self, path: &Path) -> bool {
        let Ok(k) = key(path) else { return false };
        self.files.range(k.clone()..).next().is_some_and(|(p, _)| p != &k && p.starts_with(&k))
    }

    pub fn rename(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        let (a, b) = (key(from)?, key(to)?);
        let f = self.files.remove(&a).ok_or_else(|| not_found(from))?;
        if let Some(old) = self.files.insert(b, f) {
            self.bytes -= old.data.len() as u64;
        }
        Ok(())
    }

    pub fn remove_file(&mut self, path: &Path) -> io::Result<()> {
        let f = self.files.remove(&key(path)?).ok_or_else(|| not_found(path))?;
        self.bytes -= f.data.len() as u64;
        Ok(())
    }

    /// The files directly in folder `dir`, at most `limit`.
    pub fn list(&self, dir: &Path, limit: usize) -> Vec<PathBuf> {
        let Ok(k) = key(dir) else { return Vec::new() };
        self.files.keys().filter(|p| p.parent() == Some(k.as_path())).take(limit).cloned().collect()
    }

    /// What commands wrote since the last call, in path order; the store is emptied (the
    /// front end's own files included: they have been read by then).
    pub fn take_written(&mut self) -> Vec<(PathBuf, Vec<u8>)> {
        self.bytes = 0;
        std::mem::take(&mut self.files).into_iter().filter(|(_, f)| f.written).map(|(p, f)| (p, f.data)).collect()
    }

    /// Bytes held.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
}

thread_local! {
    /// This thread's store, when it has one (always in the browser).
    static MEMORY: RefCell<Option<MemFs>> = RefCell::new(if cfg!(target_arch = "wasm32") { Some(MemFs::default()) } else { None });
}

/// From now on, commands on this thread read and write memory, not the disk (tests; the
/// browser always does).
pub fn use_memory() {
    MEMORY.with(|m| {
        let mut m = m.borrow_mut();
        if m.is_none() {
            *m = Some(MemFs::default());
        }
    });
}

/// Whether this thread's commands use the in-memory store.
pub fn in_memory() -> bool {
    MEMORY.with(|m| m.borrow().is_some())
}

/// Runs `f` on this thread's store; `None` when files live on disk.
fn mem<R>(f: impl FnOnce(&mut MemFs) -> R) -> Option<R> {
    MEMORY.with(|m| m.borrow_mut().as_mut().map(f))
}

/// Puts a file the user picked where a command will read it. On disk this writes the file.
pub fn put(path: &Path, data: Vec<u8>) -> io::Result<()> {
    match mem(|m| m.put(path, data.clone())) {
        Some(r) => r,
        None => std::fs::write(path, data),
    }
}

/// What commands wrote since the last call (the store is emptied). Nothing on disk.
pub fn take_written() -> Vec<(PathBuf, Vec<u8>)> {
    mem(MemFs::take_written).unwrap_or_default()
}

pub fn read(path: &Path) -> io::Result<Vec<u8>> {
    mem(|m| m.read(path)).unwrap_or_else(|| std::fs::read(path))
}

pub fn read_to_string(path: &Path) -> io::Result<String> {
    match mem(|m| m.read(path)) {
        Some(r) => String::from_utf8(r?).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "the file is not UTF-8 text")),
        None => std::fs::read_to_string(path),
    }
}

pub fn write(path: &Path, data: impl AsRef<[u8]>) -> io::Result<()> {
    match mem(|m| m.write(path, data.as_ref().to_vec())) {
        Some(r) => r,
        None => std::fs::write(path, data),
    }
}

/// The file's size in bytes (`std::fs::metadata(path)?.len()`).
pub fn len(path: &Path) -> io::Result<u64> {
    mem(|m| m.len(path)).unwrap_or_else(|| std::fs::metadata(path).map(|m| m.len()))
}

pub fn exists(path: &Path) -> bool {
    mem(|m| m.is_file(path) || m.is_dir(path)).unwrap_or_else(|| path.exists())
}

pub fn is_file(path: &Path) -> bool {
    mem(|m| m.is_file(path)).unwrap_or_else(|| path.is_file())
}

pub fn is_dir(path: &Path) -> bool {
    mem(|m| m.is_dir(path)).unwrap_or_else(|| path.is_dir())
}

/// Folders in memory exist through their files, so this only checks the name there.
pub fn create_dir_all(path: &Path) -> io::Result<()> {
    match mem(|m| if m.is_file(path) { Err(unsupported("a file has that name")) } else { Ok(()) }) {
        Some(r) => r,
        None => std::fs::create_dir_all(path),
    }
}

pub fn rename(from: &Path, to: &Path) -> io::Result<()> {
    mem(|m| m.rename(from, to)).unwrap_or_else(|| std::fs::rename(from, to))
}

pub fn remove_file(path: &Path) -> io::Result<()> {
    mem(|m| m.remove_file(path)).unwrap_or_else(|| std::fs::remove_file(path))
}

/// The entries of folder `dir`, at most `limit` (none if it can't be read).
pub fn list(dir: &Path, limit: usize) -> Vec<PathBuf> {
    match mem(|m| m.list(dir, limit)) {
        Some(l) => l,
        None => std::fs::read_dir(dir).map(|rd| rd.take(limit).flatten().map(|e| e.path()).collect()).unwrap_or_default(),
    }
}

/// A file being written in pieces: buffered to the disk, or collected and stored on
/// [`Writer::finish`] in memory. Dropped unfinished, nothing reaches memory.
pub enum Writer {
    Disk(io::BufWriter<std::fs::File>),
    Memory(PathBuf, Vec<u8>),
}

impl Writer {
    pub fn create(path: &Path) -> io::Result<Writer> {
        if in_memory() {
            key(path)?;
            return Ok(Writer::Memory(path.to_path_buf(), Vec::new()));
        }
        Ok(Writer::Disk(io::BufWriter::new(std::fs::File::create(path)?)))
    }

    pub fn finish(self) -> io::Result<()> {
        match self {
            Writer::Disk(mut f) => io::Write::flush(&mut f),
            Writer::Memory(path, data) => write(&path, data),
        }
    }
}

impl io::Write for Writer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Writer::Disk(f) => f.write(buf),
            Writer::Memory(_, data) => {
                if data.len() as u64 + buf.len() as u64 > MAX_MEMORY_BYTES {
                    return Err(io::Error::new(io::ErrorKind::OutOfMemory, "the file does not fit in memory"));
                }
                data.extend_from_slice(buf);
                Ok(buf.len())
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Writer::Disk(f) => f.flush(),
            Writer::Memory(..) => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_have_one_spelling_and_stay_inside() {
        assert_eq!(key(Path::new("/a/./b/../c.txt")).unwrap(), PathBuf::from("a").join("c.txt"));
        assert_eq!(key(Path::new("c.txt")).unwrap(), key(Path::new("./c.txt")).unwrap());
        assert!(key(Path::new("../etc/passwd")).is_err());
        assert!(key(Path::new("/")).is_err());
        assert!(key(Path::new(&"a/".repeat(100))).is_err());
    }

    #[test]
    fn files_round_trip_and_folders_follow_their_files() {
        let mut m = MemFs::default();
        m.put(Path::new("in/walk.anicraft"), b"{}".to_vec()).unwrap();
        assert!(m.is_file(Path::new("in/walk.anicraft")) && m.is_dir(Path::new("in")) && !m.is_dir(Path::new("in/walk.anicraft")));
        assert!(!m.is_dir(Path::new("i")), "a prefix of a name is not a folder");
        m.write(Path::new("out/frame_0001.png"), vec![1, 2, 3]).unwrap();
        m.write(Path::new("out/frame_0002.png.tmp"), vec![4]).unwrap();
        m.rename(Path::new("out/frame_0002.png.tmp"), Path::new("out/frame_0002.png")).unwrap();
        assert_eq!(m.list(Path::new("out"), 10).len(), 2);
        assert_eq!(m.len(Path::new("out/frame_0001.png")).unwrap(), 3);
        assert_eq!(m.bytes(), 6);
        assert!(m.write(Path::new("in"), vec![]).is_err(), "a folder's name is taken");
        assert_eq!(m.read(Path::new("nope")).unwrap_err().kind(), io::ErrorKind::NotFound);
        let out = m.take_written();
        assert_eq!(out.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>(), vec![PathBuf::from("out").join("frame_0001.png"), PathBuf::from("out").join("frame_0002.png")]);
        assert_eq!((m.bytes(), m.is_file(Path::new("in/walk.anicraft"))), (0, false), "emptied, the picked file too");
    }

    #[test]
    fn memory_is_bounded() {
        let mut m = MemFs { bytes: MAX_MEMORY_BYTES - 2, ..MemFs::default() };
        assert_eq!(m.write(Path::new("big"), vec![0; 3]).unwrap_err().kind(), io::ErrorKind::OutOfMemory);
        m.write(Path::new("fits"), vec![0; 2]).unwrap();
        m.write(Path::new("fits"), vec![0; 1]).unwrap();
        assert_eq!(m.bytes(), MAX_MEMORY_BYTES - 1, "replacing a file counts its new size only");
    }

    #[test]
    fn a_thread_can_switch_to_memory() {
        std::thread::spawn(|| {
            assert!(!in_memory());
            use_memory();
            put(Path::new("a.txt"), b"hi".to_vec()).unwrap();
            assert_eq!(read_to_string(Path::new("a.txt")).unwrap(), "hi");
            let mut w = Writer::create(Path::new("out/b.wav")).unwrap();
            io::Write::write_all(&mut w, b"RIFF").unwrap();
            w.finish().unwrap();
            assert!(exists(Path::new("out")) && is_file(Path::new("out/b.wav")) && !Path::new("out/b.wav").exists());
            assert_eq!(take_written(), vec![(PathBuf::from("out").join("b.wav"), b"RIFF".to_vec())]);
        })
        .join()
        .unwrap();
        assert!(!in_memory(), "other threads keep the disk");
    }
}
