//! Unpacking into the plugin directory (architecture sections 12.1, 12.3
//! steps 5 and 8, and 12.5).

use std::fs::{self, File, OpenOptions};
use std::io::Write as _;
use std::path::Path;

use deckpilot_plugin_manifest::PackageIntegrity;

use crate::error::PackageError;
use crate::read::VerifiedPackage;
use crate::sha256;

impl VerifiedPackage {
    /// Writes the package to `target` through a fresh staging directory
    /// under `staging_root`, then moves it into place with one rename.
    ///
    /// * `target` must not exist (a crash after an earlier rename leaves a
    ///   directory the caller compares with [`verify_installed`] and moves
    ///   away first, section 12.3 step 8).
    /// * `staging_root` must be on the same volume as `target`
    ///   (`<data dir>/plugins/.staging`), so the rename is atomic; across
    ///   volumes it fails and nothing is moved.
    /// * Files are created with `create_new` (never through an existing
    ///   path), synced, made read-only, and the directories are synced on
    ///   Unix before and after the rename.
    ///
    /// On any error the staging directory is removed and `target` is left
    /// untouched.
    pub fn unpack(&self, staging_root: &Path, target: &Path) -> Result<(), PackageError> {
        ensure_absent(target)?;
        fs::create_dir_all(staging_root).map_err(|e| PackageError::io(staging_root, &e))?;
        let staging = tempfile::Builder::new()
            .prefix("unpack-")
            .tempdir_in(staging_root)
            .map_err(|e| PackageError::io(staging_root, &e))?;
        let root = staging.path().join("files");
        fs::create_dir(&root).map_err(|e| PackageError::io(&root, &e))?;

        let mut written = Vec::with_capacity(self.files.len());
        let mut directories = vec![root.clone()];
        for (path, bytes) in &self.files {
            // Paths were checked: relative, `/`-separated, no `.` or `..`,
            // no case collisions, no file that is also a directory.
            let (parents, name) = path.rsplit_once('/').unwrap_or(("", path));
            let mut destination = root.clone();
            for segment in parents.split('/').filter(|s| !s.is_empty()) {
                destination.push(segment);
                match fs::create_dir(&destination) {
                    Ok(()) => directories.push(destination.clone()),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(PackageError::io(&destination, &e)),
                }
            }
            destination.push(name);
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&destination)
                .map_err(|e| PackageError::io(&destination, &e))?;
            file.write_all(bytes)
                .and_then(|()| file.sync_all())
                .map_err(|e| PackageError::io(&destination, &e))?;
            written.push(destination);
        }
        for path in &written {
            let mut permissions = fs::metadata(path)
                .map_err(|e| PackageError::io(path, &e))?
                .permissions();
            permissions.set_readonly(true);
            fs::set_permissions(path, permissions).map_err(|e| PackageError::io(path, &e))?;
        }
        for directory in directories.iter().rev() {
            sync_directory(directory)?;
        }

        ensure_absent(target)?;
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| PackageError::io(parent, &e))?;
        }
        fs::rename(&root, target).map_err(|e| PackageError::io(target, &e))?;
        if let Some(parent) = target.parent() {
            sync_directory(parent)?;
        }
        sync_directory(staging.path())?;
        // Dropping `staging` removes the now empty staging directory.
        Ok(())
    }
}

fn ensure_absent(target: &Path) -> Result<(), PackageError> {
    match fs::symlink_metadata(target) {
        Ok(_) => Err(PackageError::TargetExists(target.display().to_string())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(PackageError::io(target, &e)),
    }
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), PackageError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|e| PackageError::io(path, &e))
}

#[cfg(not(unix))]
#[allow(clippy::unnecessary_wraps)]
fn sync_directory(_path: &Path) -> Result<(), PackageError> {
    // Windows cannot open a directory as a file; NTFS journals the rename.
    Ok(())
}

/// Checks an installed plugin directory against its integrity record
/// before its bundle is loaded (section 12.5): exactly the recorded files,
/// regular files only (no symbolic links anywhere), each with its recorded
/// sha256.
pub fn verify_installed(dir: &Path, integrity: &PackageIntegrity) -> Result<(), PackageError> {
    let mut found = Vec::new();
    walk(dir, "", &mut found)?;
    for (path, bytes) in &found {
        integrity.verify_file(path, &sha256(bytes)).map_err(|e| {
            PackageError::InstalledMismatch {
                path: path.clone(),
                problem: e.to_string(),
            }
        })?;
    }
    for path in integrity.files.keys() {
        if !found.iter().any(|(p, _)| p == path) {
            return Err(PackageError::InstalledMismatch {
                path: path.clone(),
                problem: "is missing".into(),
            });
        }
    }
    Ok(())
}

fn walk(dir: &Path, prefix: &str, found: &mut Vec<(String, Vec<u8>)>) -> Result<(), PackageError> {
    let entries = fs::read_dir(dir).map_err(|e| PackageError::io(dir, &e))?;
    for entry in entries {
        let entry = entry.map_err(|e| PackageError::io(dir, &e))?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| PackageError::InstalledMismatch {
                path: path.display().to_string(),
                problem: "has a name that is not UTF-8".into(),
            })?;
        let relative = if prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{prefix}/{name}")
        };
        let kind = fs::symlink_metadata(&path)
            .map_err(|e| PackageError::io(&path, &e))?
            .file_type();
        if kind.is_dir() {
            walk(&path, &relative, found)?;
        } else if kind.is_file() {
            let bytes = fs::read(&path).map_err(|e| PackageError::io(&path, &e))?;
            found.push((relative, bytes));
        } else {
            return Err(PackageError::InstalledMismatch {
                path: relative,
                problem: "is not a regular file or directory".into(),
            });
        }
    }
    Ok(())
}
