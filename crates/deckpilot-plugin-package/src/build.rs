//! Deterministic package builder (architecture section 8).

use std::io::Write as _;

use deckpilot_plugin_manifest::{Manifest, PackageDigest, PackageIntegrity};
use flate2::{Compression, GzBuilder};

use crate::contents::{self, Files};
use crate::error::{PackageError, PathError};
use crate::limits::Limits;
use crate::paths::check_entry_path;
use crate::sha256;

/// The gzip header OS byte: 255 ("unknown"), so the output does not depend
/// on the platform that built it.
pub const GZIP_OS_UNKNOWN: u8 = 255;

/// Builds a `.dpplugin` package.
///
/// The output depends only on the file paths and contents: entries are
/// sorted by path (bytewise), every header is ustar with mode 0644, mtime 0,
/// uid and gid 0 and empty owner names, there are no directory entries, and
/// the gzip header has mtime 0, no file name and OS byte 255, at
/// compression level 9.
///
/// [`PackageBuilder::build`] refuses anything the reader would refuse: bad
/// or colliding paths, oversized files, a missing or invalid manifest, files
/// the manifest does not name, or names it lacks.
#[derive(Debug, Clone, Default)]
pub struct PackageBuilder {
    files: Files,
    limits: Limits,
}

/// A built package and the integrity record the registry would pin for it.
#[derive(Debug, Clone)]
pub struct BuiltPackage {
    pub bytes: Vec<u8>,
    pub manifest: Manifest,
    pub integrity: PackageIntegrity,
}

impl PackageBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// A builder that enforces `limits` instead of the defaults.
    pub fn with_limits(limits: Limits) -> Self {
        Self {
            files: Files::new(),
            limits,
        }
    }

    /// Adds a file. The path must follow the package path rules and must
    /// not collide (exactly or by case) with a file already added.
    pub fn add(
        &mut self,
        path: &str,
        bytes: impl Into<Vec<u8>>,
    ) -> Result<&mut Self, PackageError> {
        check_entry_path(path)?;
        if self.files.contains_key(path) {
            return Err(PathError::Duplicate(path.to_owned()).into());
        }
        if let Some(other) = self
            .files
            .keys()
            .find(|other| other.eq_ignore_ascii_case(path))
        {
            return Err(PathError::CaseCollision(other.clone(), path.to_owned()).into());
        }
        self.files.insert(path.to_owned(), bytes.into());
        Ok(self)
    }

    /// Validates the contents and writes the archive.
    pub fn build(&self) -> Result<BuiltPackage, PackageError> {
        contents::check_shape(&self.files, &self.limits)?;
        let manifest = contents::parse_manifest(&self.files)?;
        let bytes = encode(&self.files)?;
        let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if size > self.limits.max_archive_bytes {
            return Err(PackageError::ArchiveTooLarge {
                size,
                limit: self.limits.max_archive_bytes,
            });
        }
        let package = PackageDigest {
            sha256: sha256(&bytes),
            size,
        };
        let integrity = contents::integrity(&self.files, &manifest, package)?;
        Ok(BuiltPackage {
            bytes,
            manifest,
            integrity,
        })
    }
}

/// Writes `files` as a deterministic gzip-compressed ustar archive. No
/// content checks; [`PackageBuilder::build`] runs them first.
pub(crate) fn encode(files: &Files) -> Result<Vec<u8>, PackageError> {
    let tar_error = |e: std::io::Error| PackageError::Tar(e.to_string());
    let mut tar = tar::Builder::new(Vec::new());
    for (path, bytes) in files {
        let mut header = tar::Header::new_ustar();
        header.set_path(path).map_err(|_| PathError::Invalid {
            path: path.clone(),
            reason: "cannot be stored in a ustar header (a path over 100 characters needs a / \
                     within its last 100 characters)",
        })?;
        header.set_size(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
        header.set_mode(0o644);
        header.set_mtime(0);
        header.set_uid(0);
        header.set_gid(0);
        header.set_username("").map_err(tar_error)?;
        header.set_groupname("").map_err(tar_error)?;
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        tar.append(&header, bytes.as_slice()).map_err(tar_error)?;
    }
    let tar_bytes = tar.into_inner().map_err(tar_error)?;
    let mut gzip = GzBuilder::new()
        .mtime(0)
        .operating_system(GZIP_OS_UNKNOWN)
        .write(Vec::new(), Compression::best());
    gzip.write_all(&tar_bytes).map_err(tar_error)?;
    gzip.finish().map_err(tar_error)
}
