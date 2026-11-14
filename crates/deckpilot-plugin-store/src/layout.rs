//! The plugin directory (architecture section 12.1) and its file system
//! steps.
//!
//! ```text
//! <data dir>/plugins/plugins.db            this crate's encrypted database
//! <data dir>/plugins/.lock                 held by the one PluginManager
//! <data dir>/plugins/<id>/<version>/       unpacked, read-only files
//! <data dir>/plugins/.staging/             unpack area (same volume)
//! <data dir>/plugins/.trash/               directories pending deletion
//! ```
//!
//! `plugins.db` cannot clash with a plugin directory: `plugins` is a
//! reserved publisher id, and the dot directories are not plugin ids.
//! Every removal is a rename into `.trash` first (atomic), then a delete
//! that may be repeated.

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use deckpilot_plugin_manifest::PluginId;

use crate::error::{Result, StoreError};

pub const PLUGINS_DIR: &str = "plugins";
pub const DATABASE_FILE: &str = "plugins.db";
const LOCK_FILE: &str = ".lock";
const STAGING_DIR: &str = ".staging";
const TRASH_DIR: &str = ".trash";

/// Paths under `<data dir>/plugins`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    root: PathBuf,
}

impl Layout {
    /// The layout under an app data directory.
    pub fn new(data_dir: &Path) -> Self {
        Self {
            root: data_dir.join(PLUGINS_DIR),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn database(&self) -> PathBuf {
        self.root.join(DATABASE_FILE)
    }

    pub fn lock_file(&self) -> PathBuf {
        self.root.join(LOCK_FILE)
    }

    pub fn staging(&self) -> PathBuf {
        self.root.join(STAGING_DIR)
    }

    pub fn trash(&self) -> PathBuf {
        self.root.join(TRASH_DIR)
    }

    pub fn plugin_dir(&self, id: &PluginId) -> PathBuf {
        self.root.join(id.as_str())
    }

    /// `<id>/<version>`, relative to the root (the stored `install_dir`).
    pub fn relative_version_dir(id: &PluginId, version: &semver::Version) -> String {
        format!("{}/{version}", id.as_str())
    }

    pub fn resolve(&self, relative: &str) -> PathBuf {
        relative
            .split('/')
            .fold(self.root.clone(), |path, part| path.join(part))
    }

    pub fn version_dir(&self, id: &PluginId, version: &semver::Version) -> PathBuf {
        self.resolve(&Self::relative_version_dir(id, version))
    }

    pub(crate) fn ensure(&self) -> Result<()> {
        for dir in [self.root.clone(), self.staging(), self.trash()] {
            fs::create_dir_all(&dir).map_err(|e| StoreError::io(&dir, &e))?;
        }
        Ok(())
    }

    /// Takes the exclusive lock of the plugin directory (one writer per data
    /// directory). Released when the returned file is dropped, also when
    /// the process dies.
    pub(crate) fn lock(&self) -> Result<File> {
        let path = self.lock_file();
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|e| StoreError::io(&path, &e))?;
        match file.try_lock() {
            Ok(()) => Ok(file),
            Err(fs::TryLockError::WouldBlock) => {
                Err(StoreError::Locked(self.root.display().to_string()))
            }
            Err(fs::TryLockError::Error(e)) => Err(StoreError::io(&path, &e)),
        }
    }

    /// Moves `path` into `.trash` (one rename). A missing `path` is fine:
    /// the step may run again after a crash.
    pub(crate) fn move_to_trash(&self, path: &Path) -> Result<bool> {
        match fs::symlink_metadata(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(StoreError::io(path, &e)),
            Ok(_) => {}
        }
        let trash = self.trash();
        fs::create_dir_all(&trash).map_err(|e| StoreError::io(&trash, &e))?;
        let name = path
            .file_name()
            .map_or_else(|| "entry".into(), |n| n.to_string_lossy().into_owned());
        let target = trash.join(unique_name(&name));
        fs::rename(path, &target).map_err(|e| StoreError::io(path, &e))?;
        if let Some(parent) = path.parent() {
            sync_directory(parent);
        }
        Ok(true)
    }

    /// Deletes everything in `.trash`.
    pub(crate) fn empty_trash(&self) -> Result<usize> {
        empty_dir(&self.trash())
    }

    /// Deletes everything in `.staging` (only leftovers of a crash: the
    /// manager holds the lock, so no unpack is running).
    pub(crate) fn empty_staging(&self) -> Result<usize> {
        empty_dir(&self.staging())
    }

    /// Plugin directories (`<id>`) present on disk, as names.
    pub(crate) fn plugin_dirs(&self) -> Result<Vec<String>> {
        list_dirs(&self.root).map(|names| {
            names
                .into_iter()
                .filter(|name| !name.starts_with('.'))
                .collect()
        })
    }

    /// Version directory names under `<id>`.
    pub(crate) fn version_dirs(&self, plugin_dir_name: &str) -> Result<Vec<String>> {
        list_dirs(&self.root.join(plugin_dir_name))
    }

    /// Entries (files or directories) in `.staging` and `.trash`.
    pub(crate) fn leftovers(&self) -> Result<usize> {
        Ok(count_entries(&self.staging())? + count_entries(&self.trash())?)
    }
}

fn unique_name(base: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{base}-{}-{nanos}-{n}", std::process::id())
}

fn list_dirs(dir: &Path) -> Result<Vec<String>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(StoreError::io(dir, &e)),
    };
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| StoreError::io(dir, &e))?;
        let kind = entry.file_type().map_err(|e| StoreError::io(dir, &e))?;
        if kind.is_dir() {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    names.sort();
    Ok(names)
}

fn count_entries(dir: &Path) -> Result<usize> {
    match fs::read_dir(dir) {
        Ok(entries) => Ok(entries.count()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(StoreError::io(dir, &e)),
    }
}

fn empty_dir(dir: &Path) -> Result<usize> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(StoreError::io(dir, &e)),
    };
    let mut removed = 0;
    for entry in entries {
        let entry = entry.map_err(|e| StoreError::io(dir, &e))?;
        remove_all(&entry.path())?;
        removed += 1;
    }
    sync_directory(dir);
    Ok(removed)
}

/// Removes a file or a directory tree; read-only files (unpacked packages)
/// are made writable first, which Windows requires.
pub(crate) fn remove_all(path: &Path) -> Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(StoreError::io(path, &e)),
    };
    if metadata.is_dir() {
        let entries = fs::read_dir(path).map_err(|e| StoreError::io(path, &e))?;
        for entry in entries {
            let entry = entry.map_err(|e| StoreError::io(path, &e))?;
            remove_all(&entry.path())?;
        }
        fs::remove_dir(path).map_err(|e| StoreError::io(path, &e))
    } else {
        if metadata.permissions().readonly() && metadata.is_file() {
            let mut permissions = metadata.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
            fs::set_permissions(path, permissions).map_err(|e| StoreError::io(path, &e))?;
        }
        fs::remove_file(path).map_err(|e| StoreError::io(path, &e))
    }
}

/// Flushes a directory entry change (Unix); best effort.
fn sync_directory(dir: &Path) {
    #[cfg(unix)]
    if let Ok(handle) = File::open(dir) {
        let _ = handle.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}
