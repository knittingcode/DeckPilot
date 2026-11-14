//! Safe reading and verification of packages (architecture sections 8,
//! 9.3 step 5 and 12.3 steps 4 to 6).

use std::io::Read as _;

use deckpilot_plugin_manifest::{
    Manifest, PackageDigest, PackageIntegrity, PluginId, Sha256Digest,
};

use crate::contents::{self, Files, MANIFEST};
use crate::error::{PackageError, PathError};
use crate::limits::{BLOCK, Limits};
use crate::paths::{check_entry_path, check_entry_set};
use crate::sha256;

/// What the signed registry chain says a package must be: the detail's
/// `package.sha256` and `package.size`, its `manifest_sha256`, and the
/// index entry's id and version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedPackage {
    pub id: PluginId,
    pub version: semver::Version,
    pub package: PackageDigest,
    pub manifest_sha256: Sha256Digest,
    /// The permissions shown in the consent dialog (the detail's
    /// `permissions`). The manifest's `permissions` must be the same JSON
    /// value. `None` skips the comparison (development sideloading, where
    /// the dialog is built from this manifest).
    pub permissions: Option<Vec<serde_json::Value>>,
}

/// A package whose bytes matched the pinned digest and whose contents
/// passed every check. Only this type can be unpacked.
#[derive(Debug, Clone)]
pub struct VerifiedPackage {
    pub(crate) manifest: Manifest,
    pub(crate) files: Files,
    pub(crate) integrity: PackageIntegrity,
}

impl VerifiedPackage {
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Every file by package path.
    pub fn files(&self) -> &Files {
        &self.files
    }

    /// The record to store at install (`files_json` of section 12.4) and
    /// check again at every load.
    pub fn integrity(&self) -> &PackageIntegrity {
        &self.integrity
    }
}

fn len(bytes: &[u8]) -> u64 {
    u64::try_from(bytes.len()).unwrap_or(u64::MAX)
}

fn to_usize(value: u64) -> Result<usize, PackageError> {
    usize::try_from(value).map_err(|_| PackageError::Tar("offset out of range".into()))
}

/// Verifies `bytes` against `expected`, then reads and checks them.
///
/// Order: size limit, size equal to the pinned size, sha256 equal to the
/// pinned digest; only then is anything decompressed. After reading:
/// `manifest.json` against the pinned `manifest_sha256` (before it is
/// parsed), the manifest's rules, its id and version against the pins, its
/// permissions against the consent, and the package file rules.
pub fn verify_package(
    bytes: &[u8],
    expected: &ExpectedPackage,
    limits: &Limits,
) -> Result<VerifiedPackage, PackageError> {
    let size = len(bytes);
    if size > limits.max_archive_bytes {
        return Err(PackageError::ArchiveTooLarge {
            size,
            limit: limits.max_archive_bytes,
        });
    }
    if size != expected.package.size {
        return Err(PackageError::SizeMismatch {
            expected: expected.package.size,
            actual: size,
        });
    }
    let actual = sha256(bytes);
    if actual != expected.package.sha256 {
        return Err(PackageError::HashMismatch {
            expected: expected.package.sha256,
            actual,
        });
    }

    let files = read_archive(bytes, limits)?;
    contents::check_shape(&files, limits)?;
    let manifest_digest = sha256(&files[MANIFEST]);
    if manifest_digest != expected.manifest_sha256 {
        return Err(PackageError::ManifestHashMismatch {
            expected: expected.manifest_sha256,
            actual: manifest_digest,
        });
    }
    let manifest = contents::parse_manifest(&files)?;
    if manifest.id != expected.id || manifest.version != expected.version {
        return Err(PackageError::WrongPlugin {
            expected: format!("{}@{}", expected.id, expected.version),
            found: format!("{}@{}", manifest.id, manifest.version),
        });
    }
    if let Some(consented) = &expected.permissions {
        let raw: serde_json::Value = serde_json::from_slice(&files[MANIFEST])
            .map_err(|_| PackageError::PermissionsMismatch)?;
        let declared = raw
            .get("permissions")
            .cloned()
            .unwrap_or_else(|| serde_json::Value::Array(Vec::new()));
        if declared != serde_json::Value::Array(consented.clone()) {
            return Err(PackageError::PermissionsMismatch);
        }
    }
    let integrity = contents::integrity(&files, &manifest, expected.package.clone())?;
    Ok(VerifiedPackage {
        manifest,
        files,
        integrity,
    })
}

/// Reads an archive with every structural limit, but **without any
/// authenticity check**: use [`verify_package`] for registry packages. For
/// tools and development sideloading.
///
/// Refuses: anything over the limits (archive, decompressed stream, ratio,
/// entries, per-file and total size), data after the gzip stream, a second
/// gzip member, non-ustar headers (GNU, v7, pax and GNU long-name entries),
/// every entry type except regular files (directories, symlinks, hard
/// links, devices, FIFOs, sparse files), link targets, non-UTF-8 names, bad
/// or colliding paths, and bytes hidden in block padding or after the last
/// entry.
pub fn read_archive(bytes: &[u8], limits: &Limits) -> Result<Files, PackageError> {
    let size = len(bytes);
    if size > limits.max_archive_bytes {
        return Err(PackageError::ArchiveTooLarge {
            size,
            limit: limits.max_archive_bytes,
        });
    }
    if bytes.len() < 18 || bytes[..3] != [0x1f, 0x8b, 0x08] {
        return Err(PackageError::NotGzip);
    }
    let cap = limits.max_tar_stream_bytes();
    let mut decoder = flate2::bufread::GzDecoder::new(bytes);
    let mut tar_bytes = Vec::new();
    (&mut decoder)
        .take(cap.saturating_add(1))
        .read_to_end(&mut tar_bytes)
        .map_err(|e| PackageError::Gzip(e.to_string()))?;
    let stream = len(&tar_bytes);
    if stream > cap {
        return Err(PackageError::UnpackedTooLarge {
            limit: limits.max_unpacked_bytes,
        });
    }
    // The whole input must be one gzip member (reached its end, so its CRC
    // and length were checked).
    let rest = decoder.into_inner();
    if !rest.is_empty() {
        return Err(PackageError::TrailingData(rest.len()));
    }
    if stream > limits.ratio_exempt_bytes
        && stream > size.saturating_mul(limits.max_compression_ratio)
    {
        return Err(PackageError::CompressionRatio {
            ratio: stream / size.max(1),
            limit: limits.max_compression_ratio,
        });
    }
    read_tar(&tar_bytes, limits)
}

fn describe(typeflag: u8) -> String {
    match typeflag {
        b'1' => "hard link".into(),
        b'2' => "symbolic link".into(),
        b'3' => "character device".into(),
        b'4' => "block device".into(),
        b'5' => "directory".into(),
        b'6' => "FIFO".into(),
        b'7' => "contiguous file".into(),
        b'x' | b'g' => "pax extended header".into(),
        b'L' | b'K' => "GNU long name".into(),
        b'S' => "sparse file".into(),
        0 => "old-style (v7) regular file".into(),
        other => format!("entry of type {:?}", char::from(other)),
    }
}

fn read_tar(tar_bytes: &[u8], limits: &Limits) -> Result<Files, PackageError> {
    let tar_error = |e: std::io::Error| PackageError::Tar(e.to_string());
    let mut archive = tar::Archive::new(tar_bytes);
    // Raw: the tar crate must not interpret pax or GNU long-name entries;
    // they are refused below as entry types.
    let entries = archive.entries().map_err(tar_error)?.raw(true);
    let mut files = Files::new();
    let mut total: u64 = 0;
    let mut end: u64 = 0;
    for entry in entries {
        let mut entry = entry.map_err(tar_error)?;
        if files.len() >= limits.max_entries {
            return Err(PackageError::TooManyEntries {
                limit: limits.max_entries,
            });
        }
        let header = entry.header();
        let raw_path = header.path_bytes().into_owned();
        let path = String::from_utf8(raw_path)
            .map_err(|e| PathError::NotUtf8(String::from_utf8_lossy(e.as_bytes()).into_owned()))?;
        let typeflag = header.as_bytes()[156];
        if typeflag != b'0' {
            return Err(PackageError::NotRegularFile {
                path,
                kind: describe(typeflag),
            });
        }
        if header.as_ustar().is_none() {
            return Err(PackageError::NotUstar { path });
        }
        if header.link_name_bytes().is_some() {
            return Err(PackageError::NotRegularFile {
                path,
                kind: "file with a link target".into(),
            });
        }
        check_entry_path(&path)?;
        let size = header.entry_size().map_err(tar_error)?;
        if size > limits.max_file_bytes {
            return Err(PackageError::FileTooLarge {
                path,
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
        let start = entry.raw_file_position();
        let mut data = Vec::with_capacity(to_usize(size)?);
        entry.read_to_end(&mut data).map_err(tar_error)?;
        if len(&data) != size {
            return Err(PackageError::Tar(format!("`{path}` is truncated")));
        }
        let data_end = start.saturating_add(size);
        let padded = data_end.div_ceil(BLOCK).saturating_mul(BLOCK);
        let padding = tar_bytes
            .get(to_usize(data_end)?..to_usize(padded)?)
            .ok_or_else(|| PackageError::Tar(format!("`{path}` is truncated")))?;
        if padding.iter().any(|&b| b != 0) {
            return Err(PackageError::HiddenData);
        }
        end = padded;
        if files.contains_key(&path) {
            return Err(PathError::Duplicate(path).into());
        }
        files.insert(path, data);
    }
    // After the last entry: the end-of-archive marker (two zero blocks) and
    // optional zero record padding, nothing else.
    let tail = tar_bytes.get(to_usize(end)?..).unwrap_or_default();
    if len(tail) < 2 * BLOCK {
        return Err(PackageError::Tar(
            "the end-of-archive marker is missing".into(),
        ));
    }
    if tail.iter().any(|&b| b != 0) {
        return Err(PackageError::HiddenData);
    }
    check_entry_set(files.keys().map(String::as_str))?;
    Ok(files)
}
