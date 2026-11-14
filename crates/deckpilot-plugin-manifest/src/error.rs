//! Errors, worded so the installer or `deckpilot-plugin validate` can show
//! them to a plugin author or user as they are.

use std::fmt;

/// What kind of rule a [`Issue`] broke. Stable for tests and tooling; the
/// message carries the detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum IssueCode {
    /// A text is empty, too short or too long.
    Length,
    /// A list or object has too few or too many entries.
    Count,
    /// A number is outside its range.
    Range,
    /// The same value appears twice where values must be unique.
    Duplicate,
    /// An id, key or name has the wrong form.
    InvalidId,
    /// A publisher id DeckPilot keeps for itself.
    Reserved,
    /// A version does not follow DeckPilot's release grammar.
    InvalidVersion,
    /// A version range no version can satisfy.
    UnsatisfiableRange,
    /// A path is absolute, leaves the package, or has bad characters.
    InvalidPath,
    /// A URL is not https or does not parse.
    InvalidUrl,
    /// A host pattern is too broad or malformed.
    InvalidHost,
    /// A variable read entry is malformed or not allowed.
    InvalidVariable,
    /// Two fields disagree (e.g. `id` and `publisher.id`).
    Mismatch,
    /// A feature needs a permission the manifest does not declare.
    MissingPermission,
    /// A name refers to something that is not declared.
    UnknownReference,
    /// A default value does not fit its parameter kind.
    InvalidDefault,
    /// A value has the wrong form for its field.
    InvalidValue,
    /// A field that only makes sense together with another one.
    Conflict,
    /// A file is missing from, or not allowed in, the package.
    PackageFile,
    /// Review advice; never an error.
    Review,
}

/// One problem, at a path such as `contributes.actions[0].id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub path: String,
    pub code: IssueCode,
    pub message: String,
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.path.is_empty() {
            f.write_str(&self.message)
        } else {
            write!(f, "{}: {}", self.path, self.message)
        }
    }
}

/// Every problem a validation found, in manifest order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationErrors(pub Vec<Issue>);

impl ValidationErrors {
    pub fn issues(&self) -> &[Issue] {
        &self.0
    }

    /// Whether some issue has `code` at exactly `path`.
    pub fn has(&self, path: &str, code: IssueCode) -> bool {
        self.0.iter().any(|i| i.path == path && i.code == code)
    }
}

impl fmt::Display for ValidationErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0.as_slice() {
            [one] => write!(f, "the manifest is invalid: {one}"),
            many => {
                write!(f, "the manifest has {} problems:", many.len())?;
                for issue in many {
                    write!(f, "\n- {issue}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for ValidationErrors {}

/// Why `manifest.json` could not be read as a manifest.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ManifestError {
    #[error("manifest.json is {size} bytes; the limit is {limit} bytes")]
    TooLarge { size: usize, limit: usize },
    #[error("manifest.json is not valid JSON: {message} (line {line}, column {column})")]
    Syntax {
        message: String,
        line: usize,
        column: usize,
    },
    #[error("manifestVersion {found} is not supported: this build reads manifestVersion 1")]
    UnsupportedVersion { found: String },
    #[error(
        "permissions[{index}]: plugins cannot be granted `{kind}`. Plugin API 1 has no file \
         system, process, clipboard, raw device or UI access; the kinds are network, credentials, \
         notifications, profile, variables, storage, openUrl, background and device"
    )]
    ForbiddenPermission { index: usize, kind: String },
    #[error(
        "manifest.json does not match manifestVersion 1: {message} (line {line}, column {column})"
    )]
    Structure {
        message: String,
        line: usize,
        column: usize,
    },
    #[error(transparent)]
    Invalid(#[from] ValidationErrors),
}
