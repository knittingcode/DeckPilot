//! Package integrity metadata (architecture sections 8, 10 and 12.3).
//!
//! #191 keeps hashes **out of** the manifest: the manifest is itself inside
//! the package, so it cannot carry its own digest. The signed registry
//! detail pins `manifest_sha256` and the package `sha256` and `size`, and
//! the installer records `path → sha256` for every unpacked file, which the
//! agent re-checks on load. This module models that record and checks it
//! against a manifest; it does not hash anything itself.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::{Issue, IssueCode, ValidationErrors};
use crate::model::Manifest;
use crate::path_rules;

/// Largest `.dpplugin` archive.
pub const MAX_PACKAGE_BYTES: u64 = 5 * 1024 * 1024;
/// Most entries in a package.
pub const MAX_PACKAGE_ENTRIES: usize = 256;
/// Files every package contains.
pub const REQUIRED_FILES: [&str; 3] = ["manifest.json", "plugin.js", "LICENSE"];

/// A sha256 digest, written as 64 lowercase hex digits.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Sha256Digest([u8; 32]);

/// A digest that is not 64 lowercase hex digits.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` is not a sha256 digest (64 lowercase hex digits)")]
pub struct DigestError(pub String);

impl Sha256Digest {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn parse(text: &str) -> Result<Self, DigestError> {
        let nibble = |b: u8| match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            _ => None,
        };
        let bytes = text.as_bytes();
        if bytes.len() != 64 {
            return Err(DigestError(text.to_owned()));
        }
        let mut out = [0u8; 32];
        let (pairs, _) = bytes.as_chunks::<2>();
        for (i, [high, low]) in pairs.iter().enumerate() {
            let (Some(high), Some(low)) = (nibble(*high), nibble(*low)) else {
                return Err(DigestError(text.to_owned()));
            };
            out[i] = (high << 4) | low;
        }
        Ok(Self(out))
    }
}

impl fmt::Display for Sha256Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Sha256Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sha256Digest({self})")
    }
}

impl TryFrom<String> for Sha256Digest {
    type Error = DigestError;

    fn try_from(text: String) -> Result<Self, DigestError> {
        Self::parse(&text)
    }
}

impl From<Sha256Digest> for String {
    fn from(digest: Sha256Digest) -> Self {
        digest.to_string()
    }
}

/// The package archive as the registry pins it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageDigest {
    pub sha256: Sha256Digest,
    pub size: u64,
}

/// What a package must hash to: the registry's `manifest_sha256` and
/// `package`, plus the per-file hashes recorded at install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageIntegrity {
    pub manifest_sha256: Sha256Digest,
    pub package: PackageDigest,
    /// Every file in the package, by package-relative path.
    pub files: BTreeMap<String, Sha256Digest>,
}

/// A file whose digest is not the recorded one.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IntegrityError {
    #[error("{path} is not part of the installed package")]
    Unknown { path: String },
    #[error("{path} was modified: expected sha256 {expected}, found {actual}")]
    Mismatch {
        path: String,
        expected: Sha256Digest,
        actual: Sha256Digest,
    },
}

impl Manifest {
    /// Every package file the manifest names (icons, key icons,
    /// screenshots, readme, changelog). `entry` is a source path and is not
    /// in the package.
    pub fn referenced_files(&self) -> BTreeSet<&str> {
        let mut files = BTreeSet::new();
        files.insert(self.icons.size128.as_str());
        files.extend(self.icons.size32.as_deref());
        files.extend(self.icons.size256.as_deref());
        if let Some(assets) = &self.assets {
            if let Some(icons) = &assets.key_icons {
                files.extend(icons.values().map(String::as_str));
            }
            if let Some(screenshots) = &assets.screenshots {
                files.extend(screenshots.iter().map(String::as_str));
            }
            files.extend(assets.readme.as_deref());
            files.extend(assets.changelog.as_deref());
        }
        files
    }
}

impl PackageIntegrity {
    /// Checks the record on its own and against `manifest` (section 8:
    /// required files, only referenced assets, path rules, no names that
    /// collide case-insensitively, size and count limits; section 7 rule 12:
    /// every named file exists).
    pub fn validate(&self, manifest: &Manifest) -> Result<(), ValidationErrors> {
        let mut issues = Vec::new();
        let mut error = |path: String, code, message: String| {
            issues.push(Issue {
                path,
                code,
                message,
            });
        };
        if self.package.size == 0 || self.package.size > MAX_PACKAGE_BYTES {
            error(
                "package.size".into(),
                IssueCode::Range,
                format!(
                    "must be between 1 and {MAX_PACKAGE_BYTES} bytes (it is {})",
                    self.package.size
                ),
            );
        }
        if self.files.len() > MAX_PACKAGE_ENTRIES {
            error(
                "files".into(),
                IssueCode::Count,
                format!(
                    "a package holds at most {MAX_PACKAGE_ENTRIES} files (this one has {})",
                    self.files.len()
                ),
            );
        }
        let referenced = manifest.referenced_files();
        let mut folded = HashSet::new();
        for path in self.files.keys() {
            let at = format!("files.{path}");
            if let Err(message) = path_rules::check_relative(path) {
                error(
                    at.clone(),
                    IssueCode::InvalidPath,
                    format!("`{path}` {message}"),
                );
            }
            if !folded.insert(path.to_lowercase()) {
                error(
                    at.clone(),
                    IssueCode::Duplicate,
                    format!("`{path}` differs from another file only in case"),
                );
            }
            let allowed = REQUIRED_FILES.contains(&path.as_str())
                || referenced.contains(path.as_str())
                || (!path.starts_with("assets/")
                    && matches!(path.as_str(), "README.md" | "CHANGELOG.md"));
            if !allowed {
                error(
                    at,
                    IssueCode::PackageFile,
                    format!("`{path}` is not allowed: packages hold only files the manifest names"),
                );
            }
        }
        for required in REQUIRED_FILES.iter().copied().chain(referenced) {
            if !self.files.contains_key(required) {
                error(
                    "files".into(),
                    IssueCode::PackageFile,
                    format!("`{required}` is missing from the package"),
                );
            }
        }
        if self
            .files
            .get("manifest.json")
            .is_some_and(|d| *d != self.manifest_sha256)
        {
            error(
                "files.manifest.json".into(),
                IssueCode::Mismatch,
                "does not match manifest_sha256".into(),
            );
        }
        if issues.is_empty() {
            Ok(())
        } else {
            Err(ValidationErrors(issues))
        }
    }

    /// Checks one file's digest against the record.
    pub fn verify_file(&self, path: &str, actual: &Sha256Digest) -> Result<(), IntegrityError> {
        match self.files.get(path) {
            None => Err(IntegrityError::Unknown {
                path: path.to_owned(),
            }),
            Some(expected) if expected != actual => Err(IntegrityError::Mismatch {
                path: path.to_owned(),
                expected: *expected,
                actual: *actual,
            }),
            Some(_) => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digests() {
        let text = "0123456789abcdef".repeat(4);
        let digest = Sha256Digest::parse(&text).unwrap();
        assert_eq!(digest.to_string(), text);
        assert!(Sha256Digest::parse(&text.to_uppercase()).is_err());
        assert!(Sha256Digest::parse(&text[..63]).is_err());
        assert!(Sha256Digest::parse(&format!("{}g", &text[..63])).is_err());
    }
}
