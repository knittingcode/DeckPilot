//! Rules on the files of a package, applied by the builder before writing
//! and by the reader after reading (architecture sections 7 and 8).

use std::collections::BTreeMap;

use deckpilot_plugin_manifest::{
    Manifest, PackageDigest, PackageIntegrity, REQUIRED_FILES, Sha256Digest,
};

use crate::error::PackageError;
use crate::limits::Limits;
use crate::paths::check_entry_set;
use crate::sha256;

/// The files of a package by path, sorted.
pub type Files = BTreeMap<String, Vec<u8>>;

/// The bundle every package runs.
pub const BUNDLE: &str = "plugin.js";
/// The manifest every package carries.
pub const MANIFEST: &str = "manifest.json";

fn len(bytes: &[u8]) -> u64 {
    u64::try_from(bytes.len()).unwrap_or(u64::MAX)
}

/// Shape checks that need no manifest: count, paths, sizes, required files.
pub(crate) fn check_shape(files: &Files, limits: &Limits) -> Result<(), PackageError> {
    if files.len() > limits.max_entries {
        return Err(PackageError::TooManyEntries {
            limit: limits.max_entries,
        });
    }
    check_entry_set(files.keys().map(String::as_str))?;
    let mut total: u64 = 0;
    for (path, bytes) in files {
        let size = len(bytes);
        if size > limits.max_file_bytes {
            return Err(PackageError::FileTooLarge {
                path: path.clone(),
                size,
                limit: limits.max_file_bytes,
            });
        }
        total = total.saturating_add(size);
        if total > limits.max_unpacked_bytes {
            return Err(PackageError::UnpackedTooLarge {
                limit: limits.max_unpacked_bytes,
            });
        }
    }
    for required in REQUIRED_FILES {
        if !files.contains_key(required) {
            return Err(PackageError::Missing(required));
        }
    }
    let bundle = len(&files[BUNDLE]);
    if bundle > limits.max_bundle_bytes {
        return Err(PackageError::BundleTooLarge {
            size: bundle,
            limit: limits.max_bundle_bytes,
        });
    }
    Ok(())
}

/// Parses and validates `manifest.json` (schema rules and the cross-field
/// rules of section 7).
pub(crate) fn parse_manifest(files: &Files) -> Result<Manifest, PackageError> {
    let manifest = Manifest::from_slice(&files[MANIFEST])?;
    manifest
        .validate()
        .map_err(deckpilot_plugin_manifest::ManifestError::from)?;
    Ok(manifest)
}

/// The integrity record of `files` as `package`, checked against the
/// manifest: only files the manifest names (plus the required ones and
/// README/CHANGELOG), every named file present, size range.
pub(crate) fn integrity(
    files: &Files,
    manifest: &Manifest,
    package: PackageDigest,
) -> Result<PackageIntegrity, PackageError> {
    let digests: BTreeMap<String, Sha256Digest> = files
        .iter()
        .map(|(path, bytes)| (path.clone(), sha256(bytes)))
        .collect();
    let integrity = PackageIntegrity {
        manifest_sha256: digests[MANIFEST],
        package,
        files: digests,
    };
    integrity
        .validate(manifest)
        .map_err(PackageError::Contents)?;
    Ok(integrity)
}
