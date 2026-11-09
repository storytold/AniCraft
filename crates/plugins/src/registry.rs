//! The installed plug-ins and the plug-ins folder.
//!
//! A [`Registry`] belongs to a session (not to the process), so tests, the CLI and the app
//! each see exactly the plug-ins they installed. With a folder set (the app sets its per-OS
//! settings folder), installing copies the module into it as `<id>.wasm`, so it is there on
//! the next start, and removing deletes that copy.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::{Error, Limits, Manifest, Plugin, Result};

/// Most `.wasm` files read from one folder.
const MAX_FOLDER_FILES: usize = 1000;

#[derive(Debug, Default)]
pub struct Registry {
    folder: Option<PathBuf>,
    plugins: BTreeMap<String, Arc<Plugin>>,
    revision: u64,
    limits: Limits,
}

/// What [`Registry::reload`] and [`Registry::load_folder`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FolderReport {
    /// Ids of the plug-ins installed, in file-name order.
    pub loaded: Vec<String>,
    /// `(file, error)` for each `.wasm` that failed to load.
    pub failed: Vec<(String, String)>,
}

/// What [`Registry::install_file`] did.
#[derive(Clone, Debug)]
pub struct Installed {
    pub plugin: Arc<Plugin>,
    /// The plug-in with the same id it replaced.
    pub replaced: Option<Arc<Plugin>>,
    /// Where it was copied (the plug-ins folder), if it was.
    pub copied_to: Option<PathBuf>,
}

/// What [`Registry::remove`] did.
#[derive(Clone, Debug)]
pub struct Removed {
    pub plugin: Arc<Plugin>,
    /// Its file in the plug-ins folder, now deleted.
    pub deleted: Option<PathBuf>,
}

impl Registry {
    pub fn new(folder: Option<PathBuf>) -> Registry {
        Registry { folder, ..Registry::default() }
    }

    /// The plug-ins folder, if one is set.
    pub fn folder(&self) -> Option<&Path> {
        self.folder.as_deref()
    }

    pub fn set_folder(&mut self, folder: Option<PathBuf>) {
        self.folder = folder;
        self.revision += 1;
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// Limits for plug-ins loaded from now on.
    pub fn set_limits(&mut self, limits: Limits) {
        self.limits = limits;
    }

    pub fn get(&self, id: &str) -> Option<Arc<Plugin>> {
        self.plugins.get(id).cloned()
    }

    /// Installed plug-ins, sorted by id.
    pub fn list(&self) -> impl Iterator<Item = &Arc<Plugin>> {
        self.plugins.values()
    }

    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// Changes whenever a plug-in is installed or removed, or the folder changes (menus).
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Installs a loaded plug-in in memory, replacing one with the same id.
    pub fn install(&mut self, plugin: Plugin) -> Arc<Plugin> {
        let p = Arc::new(plugin);
        self.plugins.insert(p.id().to_string(), p.clone());
        self.revision += 1;
        p
    }

    /// Loads a module from bytes with this registry's limits and installs it in memory.
    pub fn install_bytes(&mut self, bytes: &[u8]) -> Result<Arc<Plugin>> {
        Ok(self.install(Plugin::load(bytes, self.limits.clone())?))
    }

    /// Reads and validates the module at `path` without installing it.
    pub fn load_file(&self, path: &Path) -> Result<Plugin> {
        let bytes = read_module(path, self.limits.max_module_bytes)?;
        Ok(Plugin::load(&bytes, self.limits.clone())?.with_source(path))
    }

    /// The manifest of the module at `path` and the installed plug-in it would replace.
    pub fn would_replace(&self, path: &Path) -> Option<(Manifest, Arc<Plugin>)> {
        let p = self.load_file(path).ok()?;
        let old = self.get(p.id())?;
        Some((p.manifest().clone(), old))
    }

    /// Whether installing the module at `path` would overwrite a file in the plug-ins folder.
    pub fn install_overwrites(&self, path: &Path) -> Option<(Manifest, PathBuf)> {
        let folder = self.folder.as_deref()?;
        let p = self.load_file(path).ok()?;
        let dest = folder.join(format!("{}.wasm", p.id()));
        (dest.exists() && !same_file(&dest, path)).then(|| (p.manifest().clone(), dest))
    }

    /// Installs the module at `path`. With a plug-ins folder, the module is copied into it as
    /// `<id>.wasm` (atomically) unless it is already there, so it loads on the next start.
    pub fn install_file(&mut self, path: &Path) -> Result<Installed> {
        let bytes = read_module(path, self.limits.max_module_bytes)?;
        let plugin = Plugin::load(&bytes, self.limits.clone())?;
        let id = plugin.id().to_string();
        let mut copied_to = None;
        let plugin = match &self.folder {
            Some(folder) if !in_folder(path, folder) => {
                let dest = folder.join(format!("{id}.wasm"));
                write_atomic(folder, &dest, &bytes)?;
                copied_to = Some(dest.clone());
                plugin.with_source(dest)
            }
            _ => plugin.with_source(path),
        };
        // An older copy of the same plug-in under another file name would load again later.
        let old = self.get(&id);
        if let (Some(old), Some(folder)) = (&old, &self.folder)
            && let Some(src) = old.source()
            && in_folder(src, folder)
            && plugin.source().is_some_and(|s| !same_file(s, src))
        {
            let _ = std::fs::remove_file(src);
        }
        let plugin = self.install(plugin);
        Ok(Installed { plugin, replaced: old, copied_to })
    }

    /// Whether `plugin`'s file lives in the plug-ins folder (removing it deletes that file).
    pub fn owns_file(&self, plugin: &Plugin) -> bool {
        matches!((plugin.source(), &self.folder), (Some(src), Some(folder)) if in_folder(src, folder))
    }

    /// Uninstalls `id`, deleting its file when it lives in the plug-ins folder.
    pub fn remove(&mut self, id: &str) -> Result<Removed> {
        let plugin = self.get(id).ok_or_else(|| Error::NotFound(id.to_string()))?;
        let mut deleted = None;
        if self.owns_file(&plugin)
            && let Some(src) = plugin.source()
        {
            match std::fs::remove_file(src) {
                Ok(()) => deleted = Some(src.to_path_buf()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(Error::Io(format!("can't delete {}: {e}", src.display()))),
            }
        }
        self.plugins.remove(id);
        self.revision += 1;
        Ok(Removed { plugin, deleted })
    }

    /// Re-reads the plug-ins folder: plug-ins loaded from it are dropped and every `*.wasm` in
    /// it is loaded again (so edited, added and deleted files take effect). Plug-ins installed
    /// from elsewhere stay.
    pub fn reload(&mut self) -> Result<FolderReport> {
        let folder = self.folder.clone().ok_or_else(|| Error::Io("no plug-ins folder is set".into()))?;
        let from_folder: Vec<String> = self.plugins.values().filter(|p| self.owns_file(p)).map(|p| p.id().to_string()).collect();
        for id in from_folder {
            self.plugins.remove(&id);
        }
        self.revision += 1;
        if !folder.exists() {
            return Ok(FolderReport::default());
        }
        self.load_folder(&folder)
    }

    /// Installs every `*.wasm` directly in `dir` (not recursive), in file-name order. A bad
    /// module is reported and skipped; the others still load.
    pub fn load_folder(&mut self, dir: &Path) -> Result<FolderReport> {
        let rd = std::fs::read_dir(dir).map_err(|e| Error::Io(format!("{}: {e}", dir.display())))?;
        let mut files: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("wasm"))).take(MAX_FOLDER_FILES * 4).collect();
        files.sort();
        files.truncate(MAX_FOLDER_FILES);
        let mut report = FolderReport::default();
        for f in files {
            match self.load_file(&f) {
                Ok(p) => report.loaded.push(self.install(p).id().to_string()),
                Err(e) => report.failed.push((f.display().to_string(), e.to_string())),
            }
        }
        Ok(report)
    }
}

/// Reads a module file, refusing anything that isn't a regular file or is over `max` bytes.
fn read_module(path: &Path, max: usize) -> Result<Vec<u8>> {
    use std::io::Read;
    let io = |e: std::io::Error| Error::Io(format!("{}: {e}", path.display()));
    let meta = std::fs::metadata(path).map_err(io)?;
    if !meta.is_file() {
        return Err(Error::Io(format!("{} is not a file", path.display())));
    }
    if meta.len() > max as u64 {
        return Err(Error::Module(format!("{} is {} bytes (limit {max})", path.display(), meta.len())));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path).map_err(io)?.take(max as u64 + 1).read_to_end(&mut bytes).map_err(io)?;
    Ok(bytes)
}

/// Writes `dest` through a temporary file in the same folder, so a crash never leaves half a
/// module behind.
fn write_atomic(folder: &Path, dest: &Path, bytes: &[u8]) -> Result<()> {
    let io = |e: std::io::Error| Error::Io(format!("can't write to the plug-ins folder {}: {e}", folder.display()));
    std::fs::create_dir_all(folder).map_err(io)?;
    let tmp = dest.with_extension("wasm.part");
    std::fs::write(&tmp, bytes).map_err(io)?;
    std::fs::rename(&tmp, dest).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        io(e)
    })
}

fn canon(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

fn same_file(a: &Path, b: &Path) -> bool {
    canon(a) == canon(b)
}

/// Whether `file` sits directly in `folder`.
fn in_folder(file: &Path, folder: &Path) -> bool {
    file.parent().is_some_and(|p| canon(p) == canon(folder))
}
