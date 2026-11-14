//! Why a package was refused.

use deckpilot_plugin_manifest::{ManifestError, Sha256Digest, ValidationErrors};

/// Why a package path was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    /// `$defs/relativePath` (absolute, `..`, bad characters, too long…).
    #[error("`{path}` {reason}")]
    Invalid { path: String, reason: &'static str },
    #[error("`{0}` is not valid UTF-8")]
    NotUtf8(String),
    #[error("`{0}` uses a name Windows reserves (CON, PRN, AUX, NUL, COM1-9, LPT1-9)")]
    WindowsReserved(String),
    #[error("`{0}` has a segment ending in `.`, which Windows drops")]
    TrailingDot(String),
    #[error("`{0}` appears twice")]
    Duplicate(String),
    #[error("`{0}` and `{1}` differ only in case")]
    CaseCollision(String, String),
    #[error("`{0}` is a file and also the directory of `{1}`")]
    FileIsDirectory(String, String),
}

/// Why a package could not be built, read, verified or unpacked.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PackageError {
    #[error("the package is {size} bytes; the limit is {limit} bytes")]
    ArchiveTooLarge { size: u64, limit: u64 },
    #[error("the package is {actual} bytes, but the registry pinned {expected} bytes")]
    SizeMismatch { expected: u64, actual: u64 },
    #[error("the package has sha256 {actual}, but the registry pinned {expected}")]
    HashMismatch {
        expected: Sha256Digest,
        actual: Sha256Digest,
    },
    #[error("the package is not a gzip stream")]
    NotGzip,
    #[error("the gzip stream is corrupt: {0}")]
    Gzip(String),
    #[error("{0} bytes follow the gzip stream")]
    TrailingData(usize),
    #[error("the package unpacks to more than {limit} bytes")]
    UnpackedTooLarge { limit: u64 },
    #[error("the package compresses {ratio}:1, more than the {limit}:1 allowed")]
    CompressionRatio { ratio: u64, limit: u64 },
    #[error("the tar archive is corrupt: {0}")]
    Tar(String),
    #[error("the package has more than {limit} entries")]
    TooManyEntries { limit: usize },
    #[error("`{path}` is not a ustar header (GNU, v7 and pax archives are refused)")]
    NotUstar { path: String },
    #[error("`{path}` is a {kind}; packages hold regular files only")]
    NotRegularFile { path: String, kind: String },
    #[error("`{path}` is {size} bytes; the limit per file is {limit} bytes")]
    FileTooLarge { path: String, size: u64, limit: u64 },
    #[error("plugin.js is {size} bytes; the limit is {limit} bytes")]
    BundleTooLarge { size: u64, limit: u64 },
    #[error("the archive hides data in padding or after its last entry")]
    HiddenData,
    #[error(transparent)]
    Path(#[from] PathError),
    #[error("`{0}` is missing from the package")]
    Missing(&'static str),
    #[error("manifest.json: {0}")]
    Manifest(#[from] ManifestError),
    #[error("manifest.json has sha256 {actual}, but the registry pinned {expected}")]
    ManifestHashMismatch {
        expected: Sha256Digest,
        actual: Sha256Digest,
    },
    #[error("the package contains {found}, but the registry pinned {expected}")]
    WrongPlugin { expected: String, found: String },
    #[error("the manifest's permissions differ from the ones shown for consent")]
    PermissionsMismatch,
    #[error("the package contents break the package rules: {0}")]
    Contents(ValidationErrors),
    #[error("{0} already exists; refusing to overwrite it")]
    TargetExists(String),
    #[error("{path}: {message}")]
    Io { path: String, message: String },
    #[error("the installed file {path} {problem}")]
    InstalledMismatch { path: String, problem: String },
}

impl PackageError {
    pub(crate) fn io(path: &std::path::Path, error: &std::io::Error) -> Self {
        Self::Io {
            path: path.display().to_string(),
            message: error.to_string(),
        }
    }
}
