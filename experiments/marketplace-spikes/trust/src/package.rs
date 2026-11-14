//! Spike 4: deterministic `.dpplugin` packages (§8) and their verification
//! (§9.3 step 5 and §12.3).
//!
//! Trust model of #191: the registry does NOT sign packages directly. The
//! index key signs `index/<seq>.json`, which pins the detail file by sha256
//! and length, which pins the package by sha256 and size. A package is
//! therefore accepted only if its bytes hash to the value reached through
//! that signed chain ([`ExpectedPackage`], produced by
//! `client::Client::resolve_install`). Optionally (deferred decision U-2) a
//! publisher signature over the package is checked as well.
//!
//! Verification order: size, sha256, publisher signature (if pinned), and
//! only then decompress and parse, with hard limits, and finally bind the
//! contained manifest to the expected id, version and manifest hash.

use std::io::Read as _;

use flate2::Compression;
use flate2::read::GzDecoder;
use thiserror::Error;

use crate::registry::sha256_hex;
use crate::sig::{self, SigError};

pub const MAX_ARCHIVE: u64 = 5 * 1024 * 1024;
pub const MAX_UNPACKED: u64 = 10 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 256;
pub const MAX_PATH: usize = 128;
pub const MAX_BUNDLE: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PackageError {
    #[error("invalid entry path {0:?}")]
    BadPath(String),
    #[error("duplicate entry path {0:?} (case-insensitive)")]
    DuplicatePath(String),
    #[error("too many entries")]
    TooManyEntries,
    #[error("archive is {0} bytes, more than allowed")]
    ArchiveTooLarge(u64),
    #[error("unpacked content exceeds the limit")]
    UnpackedTooLarge,
    #[error("plugin.js exceeds 2 MiB")]
    BundleTooLarge,
    #[error("size {actual} differs from the pinned size {expected}")]
    SizeMismatch { expected: u64, actual: u64 },
    #[error("sha256 {actual} differs from the pinned {expected}")]
    HashMismatch { expected: String, actual: String },
    #[error("publisher signature: {0}")]
    PublisherSignature(SigError),
    #[error("a publisher key is pinned but the version has no publisher signature")]
    PublisherSignatureMissing,
    #[error("archive is malformed: {0}")]
    Malformed(String),
    #[error("entry {0:?} is not a regular ustar file")]
    NotRegularFile(String),
    #[error("required entry {0} is missing")]
    Missing(&'static str),
    #[error("manifest sha256 {actual} differs from the pinned {expected}")]
    ManifestHashMismatch { expected: String, actual: String },
    #[error("package contains {found}, expected {expected}")]
    WrongPlugin { expected: String, found: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageFile {
    pub path: String,
    pub bytes: Vec<u8>,
}

impl PackageFile {
    pub fn new(path: &str, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            path: path.to_owned(),
            bytes: bytes.into(),
        }
    }
}

/// What the signed registry chain says the package must be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedPackage {
    pub id: String,
    pub version: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
    pub manifest_sha256: String,
    /// EXTENSION (U-2): `(publisher key, signature)` if the index pins a
    /// publisher key. `Some((key, None))` means the key is pinned but the
    /// version is unsigned, which is refused.
    pub publisher: Option<(String, Option<String>)>,
}

#[derive(Debug)]
pub struct VerifiedPackage {
    pub files: Vec<PackageFile>,
    pub manifest: serde_json::Value,
}

pub fn check_path(path: &str) -> Result<(), PackageError> {
    let bad = || PackageError::BadPath(path.to_owned());
    if path.is_empty() || path.len() > MAX_PATH || path.starts_with('/') || path.ends_with('/') {
        return Err(bad());
    }
    if !path
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'/' | b'-'))
    {
        return Err(bad());
    }
    if path
        .split('/')
        .any(|seg| seg.is_empty() || seg == "." || seg == "..")
    {
        return Err(bad());
    }
    Ok(())
}

fn check_paths<'a>(paths: impl Iterator<Item = &'a str>) -> Result<(), PackageError> {
    let mut seen = std::collections::HashSet::new();
    let mut n = 0;
    for p in paths {
        n += 1;
        if n > MAX_ENTRIES {
            return Err(PackageError::TooManyEntries);
        }
        check_path(p)?;
        if !seen.insert(p.to_ascii_lowercase()) {
            return Err(PackageError::DuplicatePath(p.to_owned()));
        }
    }
    Ok(())
}

/// Deterministic package: entries sorted by path, ustar headers with mtime
/// 0, mode 0644, uid/gid 0 and empty owner names, gzip level 9 with no file
/// name and mtime 0. The same inputs always give the same bytes,
/// independent of input order, time or the machine.
pub fn build(files: &[PackageFile]) -> Result<Vec<u8>, PackageError> {
    check_paths(files.iter().map(|f| f.path.as_str()))?;
    let mut sorted: Vec<&PackageFile> = files.iter().collect();
    sorted.sort_by(|a, b| a.path.as_bytes().cmp(b.path.as_bytes()));

    let gz = flate2::GzBuilder::new()
        .mtime(0)
        .operating_system(255)
        .write(Vec::new(), Compression::new(9));
    let mut tar = tar::Builder::new(gz);
    for f in sorted {
        let mut h = tar::Header::new_ustar();
        h.set_path(&f.path)
            .map_err(|e| PackageError::Malformed(e.to_string()))?;
        h.set_size(f.bytes.len() as u64);
        h.set_mode(0o644);
        h.set_mtime(0);
        h.set_uid(0);
        h.set_gid(0);
        h.set_username("").expect("empty user name");
        h.set_groupname("").expect("empty group name");
        h.set_entry_type(tar::EntryType::Regular);
        h.set_cksum();
        tar.append(&h, f.bytes.as_slice())
            .map_err(|e| PackageError::Malformed(e.to_string()))?;
    }
    let gz = tar
        .into_inner()
        .map_err(|e| PackageError::Malformed(e.to_string()))?;
    let bytes = gz
        .finish()
        .map_err(|e| PackageError::Malformed(e.to_string()))?;
    if bytes.len() as u64 > MAX_ARCHIVE {
        return Err(PackageError::ArchiveTooLarge(bytes.len() as u64));
    }
    Ok(bytes)
}

/// Step 1 (before any parsing): bytes are exactly what the signed chain
/// pins. A streaming download would abort at `expected.size + 1` bytes.
pub fn verify_bytes(bytes: &[u8], expected: &ExpectedPackage) -> Result<(), PackageError> {
    let actual = bytes.len() as u64;
    if actual > MAX_ARCHIVE {
        return Err(PackageError::ArchiveTooLarge(actual));
    }
    if actual != expected.size {
        return Err(PackageError::SizeMismatch {
            expected: expected.size,
            actual,
        });
    }
    let hash = sha256_hex(bytes);
    if hash != expected.sha256 {
        return Err(PackageError::HashMismatch {
            expected: expected.sha256.clone(),
            actual: hash,
        });
    }
    if let Some((key, signature)) = &expected.publisher {
        let signature = signature
            .as_deref()
            .ok_or(PackageError::PublisherSignatureMissing)?;
        let comment = sig::package_comment(&expected.id, &expected.version, &hash);
        sig::verify_exact(key, signature, bytes, &comment)
            .map_err(PackageError::PublisherSignature)?;
    }
    Ok(())
}

/// Step 2: unpack with limits. Only called on bytes that passed
/// [`verify_bytes`]; the limits still apply in case the registry itself
/// pinned something hostile.
pub fn unpack(bytes: &[u8]) -> Result<Vec<PackageFile>, PackageError> {
    let mut raw = Vec::new();
    GzDecoder::new(bytes)
        .take(MAX_UNPACKED + 1)
        .read_to_end(&mut raw)
        .map_err(|e| PackageError::Malformed(format!("gzip: {e}")))?;
    if raw.len() as u64 > MAX_UNPACKED {
        return Err(PackageError::UnpackedTooLarge);
    }
    let mut archive = tar::Archive::new(raw.as_slice());
    let mut files = Vec::new();
    for entry in archive
        .entries()
        .map_err(|e| PackageError::Malformed(e.to_string()))?
    {
        let mut entry = entry.map_err(|e| PackageError::Malformed(e.to_string()))?;
        if files.len() >= MAX_ENTRIES {
            return Err(PackageError::TooManyEntries);
        }
        let header_path = String::from_utf8_lossy(&entry.header().path_bytes()).into_owned();
        let effective_path = String::from_utf8_lossy(&entry.path_bytes()).into_owned();
        // No pax/GNU long-name overrides, no links, devices or directories.
        if entry.header().as_ustar().is_none()
            || entry.header().entry_type() != tar::EntryType::Regular
            || header_path != effective_path
        {
            return Err(PackageError::NotRegularFile(effective_path));
        }
        let mut data = Vec::new();
        entry
            .read_to_end(&mut data)
            .map_err(|e| PackageError::Malformed(e.to_string()))?;
        files.push(PackageFile {
            path: effective_path,
            bytes: data,
        });
    }
    check_paths(files.iter().map(|f| f.path.as_str()))?;
    Ok(files)
}

/// Full verification: bytes, then content bound to id, version and the
/// pinned manifest hash.
pub fn verify(bytes: &[u8], expected: &ExpectedPackage) -> Result<VerifiedPackage, PackageError> {
    verify_bytes(bytes, expected)?;
    let files = unpack(bytes)?;
    let get = |name: &'static str| {
        files
            .iter()
            .find(|f| f.path == name)
            .ok_or(PackageError::Missing(name))
    };
    let manifest_file = get("manifest.json")?;
    let bundle = get("plugin.js")?;
    get("LICENSE")?;
    if bundle.bytes.len() as u64 > MAX_BUNDLE {
        return Err(PackageError::BundleTooLarge);
    }
    let manifest_hash = sha256_hex(&manifest_file.bytes);
    if manifest_hash != expected.manifest_sha256 {
        return Err(PackageError::ManifestHashMismatch {
            expected: expected.manifest_sha256.clone(),
            actual: manifest_hash,
        });
    }
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_file.bytes)
        .map_err(|e| PackageError::Malformed(format!("manifest: {e}")))?;
    let found = format!(
        "{}@{}",
        manifest["id"].as_str().unwrap_or("?"),
        manifest["version"].as_str().unwrap_or("?")
    );
    let wanted = format!("{}@{}", expected.id, expected.version);
    if found != wanted {
        return Err(PackageError::WrongPlugin {
            expected: wanted,
            found,
        });
    }
    Ok(VerifiedPackage { files, manifest })
}
