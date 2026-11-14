//! Crash recovery (ADR 0015): autosaved copies of open documents in a recovery folder, so a
//! crash, power cut or killed process loses at most the last autosave interval of work.
//!
//! One running editor is one *recovery session* with an id (`SessionLock`). Its files in the
//! recovery folder are:
//!
//! * `<id>.anicraft`: the document, an ordinary `.anicraft` file (it opens like any other);
//! * `<id>.json`: what it is ([`RecoveryMeta`]: original path, name, when, which version);
//! * `<id>.lock`: held with an exclusive OS file lock while the session runs. The OS drops
//!   the lock when the process dies, however it dies, so "locked" means "alive" and an
//!   unlockable-free file means "left behind by a session that didn't close cleanly".
//!
//! Writes are atomic: each file goes to `<name>.tmp`, is flushed to disk and renamed over the
//! old one, so a crash mid-write leaves the previous autosave intact. Recovery files only ever
//! live in the recovery folder under a validated session id, and [`write`] refuses a target
//! that is the user's own document, so autosave never writes over the user's file.
//!
//! [`Autosaver`] does the writing on a background thread: the caller hands it a snapshot of
//! the persistent document (an `Arc` clone, O(1)) and never waits for serialization or disk.

use crate::{EngineError, Result, format};
use anicraft_doc::Document;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use web_time::{Instant, SystemTime, UNIX_EPOCH};

/// Version of the `<id>.json` description.
pub const META_VERSION: u32 = 1;
/// Longest session id accepted (ids are generated; anything else is someone else's file).
const MAX_ID: usize = 64;
/// Most entries looked at in a recovery folder.
const MAX_SCAN: usize = 4096;
/// Largest description file read.
const MAX_META_BYTES: u64 = 64 * 1024;

/// What a recovery file holds, written beside it as `<id>.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecoveryMeta {
    pub version: u32,
    /// The session that wrote it.
    pub session: String,
    /// Where the document was saved before (`None`: never saved, or imported).
    pub original: Option<PathBuf>,
    /// Display name: the original's file name, else "Untitled".
    pub name: String,
    /// Seconds since 1970 (UTC) when it was written.
    pub saved_at: u64,
    /// The AniCraft version that wrote it.
    pub app: String,
    /// Movie frames and scenes, for the restore list.
    pub frames: u32,
    pub scenes: usize,
}

impl RecoveryMeta {
    /// Describes `doc`, last saved at `original`, for session `session`.
    pub fn describe(session: &str, doc: &Document, original: Option<&Path>) -> RecoveryMeta {
        RecoveryMeta {
            version: META_VERSION,
            session: session.to_string(),
            original: original.map(Path::to_path_buf),
            name: display_name(original),
            saved_at: now_secs(),
            app: env!("CARGO_PKG_VERSION").to_string(),
            frames: doc.frame_count(),
            scenes: doc.scenes.len(),
        }
    }
}

/// "walk.anicraft" for a saved document, "Untitled" otherwise.
pub fn display_name(original: Option<&Path>) -> String {
    original.and_then(Path::file_name).map_or_else(|| "Untitled".to_string(), |n| n.to_string_lossy().into_owned())
}

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// A recovery file left behind by a session that is no longer running.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub session: String,
    /// The `.anicraft` copy.
    pub document: PathBuf,
    pub meta: RecoveryMeta,
    pub bytes: u64,
}

impl Entry {
    /// The original file changed after this copy was written (saved later, e.g. just before
    /// the crash): restoring must not put the older copy back over it.
    pub fn original_is_newer(&self) -> bool {
        let Some(o) = &self.meta.original else { return false };
        let modified = std::fs::metadata(o).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok());
        modified.is_some_and(|m| m.as_secs() > self.meta.saved_at)
    }

    pub fn to_json(&self) -> Value {
        json!({
            "session": self.session,
            "document": self.document.display().to_string(),
            "name": self.meta.name,
            "original": self.meta.original.as_ref().map(|p| p.display().to_string()),
            "saved_at": self.meta.saved_at,
            "frames": self.meta.frames,
            "scenes": self.meta.scenes,
            "app": self.meta.app,
            "bytes": self.bytes,
            "original_newer": self.original_is_newer(),
        })
    }
}

/// Session ids are ours: ASCII letters, digits, `-` and `_`, so they can never name a path
/// outside the recovery folder.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn check_id(id: &str) -> Result<()> {
    if valid_id(id) { Ok(()) } else { Err(EngineError::Param(format!("'{}' is not a recovery session id", id.chars().take(80).collect::<String>()))) }
}

fn file(dir: &Path, id: &str, ext: &str) -> PathBuf {
    dir.join(format!("{id}.{ext}"))
}

/// The recovery copy of session `id` in `dir`.
pub fn document_path(dir: &Path, id: &str) -> PathBuf {
    file(dir, id, "anicraft")
}

/// Writes `bytes` to `path` atomically: a temp file beside it, flushed to disk, renamed over it.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let result = (|| -> std::io::Result<()> {
        {
            // Closed before the rename (Windows can't rename an open file).
            let mut f = File::create(&tmp)?;
            f.write_all(bytes)?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    Ok(result?)
}

/// Same file, after resolving links and `..` where the file exists.
fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// Timings of one autosave, for the performance budget.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WriteStats {
    pub bytes: u64,
    pub serialize_ms: f64,
    pub write_ms: f64,
}

/// Writes the recovery copy of session `id`: the document, then its description, each atomic.
pub fn write(dir: &Path, id: &str, doc: &Document, meta: &RecoveryMeta) -> Result<WriteStats> {
    check_id(id)?;
    let target = document_path(dir, id);
    if meta.original.as_deref().is_some_and(|o| same_file(o, &target)) {
        return Err(EngineError::State(format!("{} is the document itself; autosave never writes over it", target.display())));
    }
    // A file there that isn't a recovery copy (no lock or description beside it) is someone's.
    let ours = file(dir, id, "lock").is_file() || file(dir, id, "json").is_file();
    if target.exists() && !ours {
        return Err(EngineError::State(format!("{} is not an autosave; autosave never writes over it", target.display())));
    }
    let t = Instant::now();
    let json = format::to_json(doc)?;
    let serialize_ms = t.elapsed().as_secs_f64() * 1000.0;
    let t = Instant::now();
    std::fs::create_dir_all(dir)?;
    write_atomic(&target, json.as_bytes())?;
    let mut meta = meta.clone();
    meta.session = id.to_string();
    let text = serde_json::to_string_pretty(&meta).map_err(|e| EngineError::Format(e.to_string()))?;
    write_atomic(&file(dir, id, "json"), text.as_bytes())?;
    Ok(WriteStats { bytes: json.len() as u64, serialize_ms, write_ms: t.elapsed().as_secs_f64() * 1000.0 })
}

/// Removes session `id`'s recovery copy and description (and leftover temp files), keeping its
/// lock. Returns how many files went.
pub fn clear(dir: &Path, id: &str) -> Result<usize> {
    check_id(id)?;
    let mut n = 0;
    for ext in ["anicraft", "json", "anicraft.tmp", "json.tmp"] {
        if std::fs::remove_file(file(dir, id, ext)).is_ok() {
            n += 1;
        }
    }
    Ok(n)
}

/// Whether session `id` is running: its lock file is locked by a live process.
pub fn is_live(dir: &Path, id: &str) -> bool {
    let Ok(f) = OpenOptions::new().read(true).write(true).open(file(dir, id, "lock")) else { return false };
    match f.try_lock() {
        Ok(()) => {
            let _ = f.unlock();
            false
        }
        Err(std::fs::TryLockError::WouldBlock) => true,
        // No locking on this file system: we can't tell, so offer it (restoring only reads).
        Err(std::fs::TryLockError::Error(_)) => false,
    }
}

fn read_meta(dir: &Path, id: &str) -> RecoveryMeta {
    let path = file(dir, id, "json");
    let small = std::fs::metadata(&path).is_ok_and(|m| m.len() <= MAX_META_BYTES);
    let meta = if small { std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<RecoveryMeta>(&t).ok()) } else { None };
    let mut meta = meta.unwrap_or_else(|| RecoveryMeta { name: "Untitled".into(), ..Default::default() });
    meta.session = id.to_string();
    if meta.name.trim().is_empty() {
        meta.name = display_name(meta.original.as_deref());
    }
    meta.name = meta.name.chars().take(256).collect();
    meta
}

/// Recovery copies in `dir` left by sessions that are no longer running (not `own`), newest
/// first. A missing or unreadable folder has none.
pub fn scan(dir: &Path, own: Option<&str>) -> Vec<Entry> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<Entry> = rd
        .take(MAX_SCAN)
        .flatten()
        .filter_map(|e| {
            let name = e.file_name();
            let id = name.to_str()?.strip_suffix(".anicraft")?.to_string();
            // Ours only: a session's lock or description sits beside its copy (a document the
            // user saved into this folder has neither).
            let ours = file(dir, &id, "lock").is_file() || file(dir, &id, "json").is_file();
            if !valid_id(&id) || !ours || own == Some(id.as_str()) || is_live(dir, &id) {
                return None;
            }
            let bytes = e.metadata().ok().filter(|m| m.is_file())?.len();
            let meta = read_meta(dir, &id);
            Some(Entry { document: e.path(), session: id, meta, bytes })
        })
        .collect();
    out.sort_by(|a, b| b.meta.saved_at.cmp(&a.meta.saved_at).then_with(|| a.session.cmp(&b.session)));
    out
}

/// The entry for session `id`, if it was left behind.
pub fn find(dir: &Path, id: &str) -> Result<Entry> {
    check_id(id)?;
    if is_live(dir, id) {
        return Err(EngineError::State("that document is open in another AniCraft window; it is still being autosaved there".into()));
    }
    let document = document_path(dir, id);
    let ours = file(dir, id, "lock").is_file() || file(dir, id, "json").is_file();
    let bytes = std::fs::metadata(&document).ok().filter(|m| ours && m.is_file()).ok_or_else(|| EngineError::State(format!("no autosaved document '{id}' in {}", dir.display())))?.len();
    Ok(Entry { session: id.to_string(), document, meta: read_meta(dir, id), bytes })
}

/// Removes lock files of ended sessions that left no recovery copy (sessions that crashed
/// with nothing unsaved). Returns how many went.
pub fn prune(dir: &Path) -> usize {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    let stale: Vec<String> = rd
        .take(MAX_SCAN)
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.strip_suffix(".lock").map(str::to_string))
        .filter(|id| valid_id(id) && !document_path(dir, id).exists() && !is_live(dir, id))
        .collect();
    stale.iter().filter(|id| std::fs::remove_file(file(dir, id, "lock")).is_ok()).count()
}

/// Loads a left-behind recovery copy.
pub fn load(dir: &Path, id: &str) -> Result<(Document, RecoveryMeta)> {
    let e = find(dir, id)?;
    let doc = format::load(&e.document)?;
    Ok((doc, e.meta))
}

/// Deletes a left-behind recovery copy and everything else of its session. Irreversible.
pub fn discard(dir: &Path, id: &str) -> Result<usize> {
    find(dir, id)?;
    let n = clear(dir, id)?;
    let _ = std::fs::remove_file(file(dir, id, "lock"));
    Ok(n)
}

// ----- the running session ----------------------------------------------------------------------

/// A running session's identity, held as an exclusive lock on `<id>.lock` until it is
/// [released](SessionLock::release) or the process ends.
#[derive(Debug)]
pub struct SessionLock {
    file: File,
    dir: PathBuf,
    id: String,
}

impl SessionLock {
    /// Creates the folder if needed and takes a fresh session id.
    pub fn acquire(dir: &Path) -> Result<SessionLock> {
        std::fs::create_dir_all(dir)?;
        let millis = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
        let base = format!("{millis:x}-{}", std::process::id());
        for n in 0..64u32 {
            let id = if n == 0 { base.clone() } else { format!("{base}-{n}") };
            let path = file(dir, &id, "lock");
            // `create_new`: two sessions never share a lock file, even when started together.
            let Ok(f) = OpenOptions::new().read(true).write(true).create_new(true).open(&path) else { continue };
            match f.try_lock() {
                Ok(()) => return Ok(SessionLock { file: f, dir: dir.to_path_buf(), id }),
                Err(std::fs::TryLockError::Error(e)) if e.kind() == std::io::ErrorKind::Unsupported => {
                    // A file system without locks: run without liveness (see `is_live`).
                    return Ok(SessionLock { file: f, dir: dir.to_path_buf(), id });
                }
                Err(_) => continue,
            }
        }
        Err(EngineError::State(format!("could not start a recovery session in {}", dir.display())))
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Ends the session cleanly: unlocks and removes the lock file. Call [`clear`] first unless
    /// the recovery copy should be offered next time.
    // (On wasm32 `File` can't exist, so there it has no `Drop`; the desktop needs the close.)
    #[cfg_attr(target_arch = "wasm32", allow(clippy::drop_non_drop))]
    pub fn release(self) {
        let _ = self.file.unlock();
        drop(self.file);
        let _ = std::fs::remove_file(file(&self.dir, &self.id, "lock"));
    }
}

// ----- background writer ------------------------------------------------------------------------

enum Job {
    Write { doc: Box<Document>, meta: Box<RecoveryMeta>, tag: u64 },
    Clear { tag: u64 },
}

/// What the background writer finished.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Written { tag: u64, stats: WriteStats },
    Cleared { tag: u64 },
    Failed { tag: u64, error: String },
}

/// Writes recovery copies on its own thread. Jobs run in order; when several are waiting only
/// the newest runs (each write replaces the last, and a clear removes it), so a slow disk never
/// builds a queue. Dropping it stops the thread but leaves the files: only
/// [`Autosaver::finish`] decides whether a session ended cleanly.
pub struct Autosaver {
    lock: Option<SessionLock>,
    tx: Option<Sender<Job>>,
    done: Receiver<Outcome>,
    thread: Option<std::thread::JoinHandle<()>>,
    next_tag: u64,
}

impl Autosaver {
    pub fn start(lock: SessionLock) -> Result<Autosaver> {
        let (tx, rx) = std::sync::mpsc::channel::<Job>();
        let (done_tx, done) = std::sync::mpsc::channel();
        let dir = lock.dir.clone();
        let id = lock.id.clone();
        let thread = std::thread::Builder::new().name("anicraft-autosave".into()).spawn(move || worker(&dir, &id, &rx, &done_tx))?;
        Ok(Autosaver { lock: Some(lock), tx: Some(tx), done, thread: Some(thread), next_tag: 0 })
    }

    pub fn id(&self) -> &str {
        self.lock.as_ref().map_or("", |l| l.id.as_str())
    }

    pub fn dir(&self) -> Option<&Path> {
        self.lock.as_ref().map(|l| l.dir.as_path())
    }

    fn send(&mut self, job: impl FnOnce(u64) -> Job) -> u64 {
        self.next_tag += 1;
        let tag = self.next_tag;
        if let Some(tx) = &self.tx {
            let _ = tx.send(job(tag));
        }
        tag
    }

    /// Queues a write of `doc` (a cheap snapshot). Returns its tag.
    pub fn write(&mut self, doc: Document, original: Option<&Path>) -> u64 {
        let meta = RecoveryMeta::describe(self.id(), &doc, original);
        self.send(|tag| Job::Write { doc: Box::new(doc), meta: Box::new(meta), tag })
    }

    /// Queues removal of this session's recovery copy (after a save, or when the work is gone).
    pub fn clear(&mut self) -> u64 {
        self.send(|tag| Job::Clear { tag })
    }

    /// Finished jobs since the last call.
    pub fn poll(&self) -> Vec<Outcome> {
        let mut v = Vec::new();
        loop {
            match self.done.try_recv() {
                Ok(o) => v.push(o),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return v,
            }
        }
    }

    /// Ends the session. `Some(doc)`: the work isn't safe yet, so write it one last time and
    /// keep it for the next start. `None`: a clean exit, so the recovery copy and the lock go.
    /// Waits for the writer (at most one write).
    pub fn finish(mut self, last: Option<(Document, Option<PathBuf>)>) -> Vec<Outcome> {
        let clean = last.is_none();
        match last {
            Some((doc, original)) => {
                self.write(doc, original.as_deref());
            }
            None => {
                self.clear();
            }
        }
        self.stop();
        let out = self.poll();
        if let Some(lock) = self.lock.take() {
            if clean {
                lock.release();
            } else {
                drop(lock); // unlocked by closing; the copy stays for the next start
            }
        }
        out
    }

    fn stop(&mut self) {
        self.tx = None;
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Autosaver {
    fn drop(&mut self) {
        self.stop();
    }
}

fn worker(dir: &Path, id: &str, rx: &Receiver<Job>, done: &Sender<Outcome>) {
    while let Ok(mut job) = rx.recv() {
        // Only the newest waiting job matters.
        while let Ok(newer) = rx.try_recv() {
            job = newer;
        }
        let outcome = match job {
            Job::Write { doc, meta, tag } => match write(dir, id, &doc, &meta) {
                Ok(stats) => Outcome::Written { tag, stats },
                Err(e) => Outcome::Failed { tag, error: e.to_string() },
            },
            Job::Clear { tag } => match clear(dir, id) {
                Ok(_) => Outcome::Cleared { tag },
                Err(e) => Outcome::Failed { tag, error: e.to_string() },
            },
        };
        if done.send(outcome).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anicraft_doc::sample;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("anicraft-recovery-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn ids_cannot_escape_the_folder() {
        for bad in ["", "..", "../x", "a/b", "a\\b", "C:", "x.y", "é", &"a".repeat(65)] {
            assert!(!valid_id(bad), "{bad}");
        }
        assert!(valid_id("19a0b3c-1234-2"));
        let dir = tmp("ids");
        assert!(write(&dir, "../evil", &Document::default(), &RecoveryMeta::default()).is_err());
        assert!(load(&dir, "..\\evil").is_err());
        assert!(discard(&dir, "a/b").is_err());
    }

    #[test]
    fn write_is_atomic_and_round_trips() {
        let dir = tmp("write");
        let doc = sample::demo();
        let meta = RecoveryMeta::describe("s1", &doc, Some(Path::new("/work/walk.anicraft")));
        let st = write(&dir, "s1", &doc, &meta).unwrap();
        assert!(st.bytes > 0);
        // No temp files survive a write.
        let names: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert!(names.iter().all(|n| !n.ends_with(".tmp")), "{names:?}");
        let (back, m) = load(&dir, "s1").unwrap();
        assert_eq!(back, doc);
        assert_eq!(m.name, "walk.anicraft");
        assert_eq!(m.original.as_deref(), Some(Path::new("/work/walk.anicraft")));
        // A crash mid-write (a temp file left half-written) keeps the previous copy intact.
        std::fs::write(dir.join("s1.anicraft.tmp"), b"{\"format\":\"anica").unwrap();
        assert_eq!(load(&dir, "s1").unwrap().0, doc);
        assert_eq!(scan(&dir, None).len(), 1, "temp files are not entries");
        // The next write replaces both.
        let mut d2 = doc.clone();
        d2.stage.width = 77;
        write(&dir, "s1", &d2, &meta).unwrap();
        assert_eq!(load(&dir, "s1").unwrap().0.stage.width, 77);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn never_writes_over_the_users_file() {
        let dir = tmp("own");
        std::fs::create_dir_all(&dir).unwrap();
        // A user who saved their document inside the recovery folder under a session-like name.
        let theirs = dir.join("mine.anicraft");
        std::fs::write(&theirs, b"precious").unwrap();
        let meta = RecoveryMeta::describe("mine", &Document::default(), Some(&theirs));
        assert!(write(&dir, "mine", &Document::default(), &meta).is_err());
        assert_eq!(std::fs::read(&theirs).unwrap(), b"precious");
        // Even when the copy doesn't claim to be it: a file that isn't an autosave stays.
        let meta = RecoveryMeta::describe("mine", &Document::default(), None);
        assert!(write(&dir, "mine", &Document::default(), &meta).is_err());
        assert_eq!(std::fs::read(&theirs).unwrap(), b"precious");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn live_sessions_are_not_offered_and_crashed_ones_are() {
        let dir = tmp("live");
        let lock = SessionLock::acquire(&dir).unwrap();
        let id = lock.id().to_string();
        write(&dir, &id, &sample::demo(), &RecoveryMeta::describe(&id, &sample::demo(), None)).unwrap();
        assert!(is_live(&dir, &id));
        assert!(scan(&dir, None).is_empty(), "a running session's copy is not offered");
        assert!(load(&dir, &id).is_err() && discard(&dir, &id).is_err());
        // The process "crashes": the OS drops the lock but the files stay.
        drop(lock);
        let found = scan(&dir, None);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].session, id);
        assert_eq!(found[0].meta.name, "Untitled");
        assert_eq!(scan(&dir, Some(&id)).len(), 0, "a session never offers its own copy");
        assert!(discard(&dir, &id).unwrap() >= 2);
        assert!(scan(&dir, None).is_empty());
        assert!(!file(&dir, &id, "lock").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_sessions_get_different_ids() {
        let dir = tmp("two");
        let a = SessionLock::acquire(&dir).unwrap();
        let b = SessionLock::acquire(&dir).unwrap();
        assert_ne!(a.id(), b.id());
        a.release();
        b.release();
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "a clean release leaves nothing");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn garbage_in_the_folder_is_ignored_or_reported() {
        let dir = tmp("garbage");
        std::fs::create_dir_all(dir.join("folder.anicraft")).unwrap();
        std::fs::write(dir.join("bad id.anicraft"), b"x").unwrap();
        std::fs::write(dir.join("broken.anicraft"), b"not json").unwrap();
        std::fs::write(dir.join("broken.json"), b"{{{").unwrap();
        // A user's own document saved into the folder is not a recovery copy, and is never
        // discarded as one.
        std::fs::write(dir.join("mine.anicraft"), b"precious").unwrap();
        assert!(discard(&dir, "mine").is_err());
        assert_eq!(std::fs::read(dir.join("mine.anicraft")).unwrap(), b"precious");
        let found = scan(&dir, None);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].meta.name, "Untitled");
        assert!(load(&dir, "broken").is_err(), "a corrupt copy is an error, not a panic");
        assert!(scan(Path::new("/no/such/folder/anicraft"), None).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn autosaver_writes_in_the_background_and_cleans_up_on_a_clean_exit() {
        let dir = tmp("saver");
        let mut a = Autosaver::start(SessionLock::acquire(&dir).unwrap()).unwrap();
        let id = a.id().to_string();
        let doc = sample::demo();
        for _ in 0..5 {
            a.write(doc.clone(), None);
        }
        let mut written = false;
        for _ in 0..500 {
            if a.poll().iter().any(|o| matches!(o, Outcome::Written { .. })) {
                written = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(written);
        assert!(document_path(&dir, &id).exists());
        a.finish(None);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "clean exit removes the copy and the lock");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unclean_finish_keeps_the_work_for_next_time() {
        let dir = tmp("unclean");
        let a = Autosaver::start(SessionLock::acquire(&dir).unwrap()).unwrap();
        let mut doc = sample::demo();
        doc.stage.width = 640;
        let out = a.finish(Some((doc.clone(), Some(PathBuf::from("/w/a.anicraft")))));
        assert!(out.iter().any(|o| matches!(o, Outcome::Written { .. })), "{out:?}");
        let found = scan(&dir, None);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].meta.name, "a.anicraft");
        assert_eq!(load(&dir, &found[0].session).unwrap().0.stage.width, 640);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_save_then_clear_leaves_no_copy() {
        let dir = tmp("clear");
        let mut a = Autosaver::start(SessionLock::acquire(&dir).unwrap()).unwrap();
        let id = a.id().to_string();
        a.write(sample::demo(), None);
        a.clear();
        // In order: the clear never races ahead of the write.
        for _ in 0..500 {
            if a.poll().iter().any(|o| matches!(o, Outcome::Cleared { .. })) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!document_path(&dir, &id).exists());
        drop(a); // dropped without finish (a crash): the lock goes with the process
        assert!(scan(&dir, None).is_empty());
        assert_eq!(prune(&dir), 1, "the stale lock is tidied away");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
