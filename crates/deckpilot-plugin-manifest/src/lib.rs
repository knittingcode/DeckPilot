//! **Experimental.** The DeckPilot plugin manifest contract (`manifestVersion`
//! 1): a typed model, validation with messages fit to show a user, and
//! compatibility checks.
//!
//! Nothing in DeckPilot uses this crate yet: no app, agent or module crate
//! depends on it, and plugins stay disabled. It implements the manifest part
//! of the plugin marketplace design (`docs/marketplace/architecture.md`,
//! sections 5.2, 6.5, 7 and 8, proposed in pull request #191); the JSON
//! Schema it follows is bundled as [`SCHEMA_JSON`].
//!
//! ```
//! # let json = include_str!("../tests/fixtures/example-manifest.json");
//! use deckpilot_plugin_manifest::{Manifest, Platform};
//!
//! let manifest = Manifest::parse_and_validate(json).unwrap();
//! let app = semver::Version::new(0, 9, 0);
//! assert!(manifest.is_compatible(&app, Platform::Linux));
//! ```
//!
//! Two stages, both required before a manifest is trusted:
//!
//! 1. [`Manifest::from_json`] checks structure: JSON syntax, the size limit,
//!    `manifestVersion`, field names (unknown fields are refused everywhere),
//!    types, enum values, semantic versions and version ranges.
//! 2. [`Manifest::validate`] checks every other constraint of the schema
//!    (lengths, counts, patterns, uniqueness) and the cross-field rules of
//!    section 7, and reports all problems at once with their JSON paths.
//!
//! Permissions are deny by default: [`Manifest::permissions`] is the
//! complete list of what the plugin may ever request. Kinds that are never
//! grantable in plugin API 1 (file system, processes, clipboard…) have no
//! representation at all.

mod compat;
mod error;
mod identity;
mod integrity;
mod model;
mod path_rules;
mod unique_map;
mod validate;

pub use compat::{
    ApiLevel, ApiRequirement, HostEnvironment, Incompatibility, MAX_RANGE_LEN, Platform,
    RangeError, VersionRange,
};
pub use error::{Issue, IssueCode, ManifestError, ValidationErrors};
pub use identity::{
    IdError, MAX_PLUGIN_NAME_LEN, MAX_PUBLISHER_LEN, PluginId, RESERVED_PUBLISHERS,
    is_publisher_id, lower_camel,
};
pub use integrity::{
    DigestError, IntegrityError, MAX_PACKAGE_BYTES, MAX_PACKAGE_ENTRIES, PackageDigest,
    PackageIntegrity, REQUIRED_FILES, Sha256Digest,
};
pub use model::*;
pub use path_rules::{MAX_PATH_LEN, check_relative as check_relative_path};
pub use validate::{GENERATED_SETTING_KEY, SHARED_ERROR_CODES};

/// The manifest JSON Schema this crate implements (draft 2020-12).
pub const SCHEMA_JSON: &str = include_str!("../schema/plugin-manifest.schema.json");

/// The only `manifestVersion` this crate reads.
pub const MANIFEST_VERSION: u64 = 1;

/// Largest `manifest.json` accepted, in bytes.
///
/// #191 sets no limit for the manifest itself (the package is at most 5 MiB
/// and the registry detail, which repeats the permissions, 256 KiB). This
/// crate uses 256 KiB: ample for the schema's maximums in practice (the
/// example is about 5 KiB), and it bounds the work done on untrusted input
/// before validation.
pub const MAX_MANIFEST_BYTES: usize = 256 * 1024;

/// Permission kinds a manifest might plausibly ask for that plugin API 1
/// never grants (section 5.2), reported with a clearer message than
/// "unknown variant".
const FORBIDDEN_PERMISSION_KINDS: &[&str] = &[
    "filesystem",
    "fileSystem",
    "fs",
    "files",
    "file",
    "process",
    "processes",
    "exec",
    "shell",
    "clipboard",
    "machineResources",
    "projectContext",
    "hid",
    "usb",
    "brightness",
    "firmware",
    "workflows",
    "ui",
];

impl Manifest {
    /// Reads `manifest.json` (structure only; see [`Manifest::validate`]).
    pub fn from_json(json: &str) -> Result<Self, ManifestError> {
        Self::from_slice(json.as_bytes())
    }

    /// Reads `manifest.json` bytes (structure only; see [`Manifest::validate`]).
    pub fn from_slice(bytes: &[u8]) -> Result<Self, ManifestError> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(ManifestError::TooLarge {
                size: bytes.len(),
                limit: MAX_MANIFEST_BYTES,
            });
        }
        let value: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|e| ManifestError::Syntax {
                message: strip_position(&e),
                line: e.line(),
                column: e.column(),
            })?;
        // Refuse other manifest versions before their fields produce
        // confusing "unknown field" errors.
        match value.get("manifestVersion") {
            Some(serde_json::Value::Number(n)) if n.as_u64() == Some(MANIFEST_VERSION) => {}
            Some(other) => {
                return Err(ManifestError::UnsupportedVersion {
                    found: other.to_string(),
                });
            }
            None => {}
        }
        if let Some(permissions) = value.get("permissions").and_then(|p| p.as_array()) {
            for (index, permission) in permissions.iter().enumerate() {
                if let Some(kind) = permission.get("kind").and_then(|k| k.as_str())
                    && FORBIDDEN_PERMISSION_KINDS.contains(&kind)
                {
                    return Err(ManifestError::ForbiddenPermission {
                        index,
                        kind: kind.to_owned(),
                    });
                }
            }
        }
        // Deserialize from the bytes again (not from `value`) so errors keep
        // their line and column.
        serde_json::from_slice(bytes).map_err(|e| ManifestError::Structure {
            message: strip_position(&e),
            line: e.line(),
            column: e.column(),
        })
    }

    /// [`Manifest::from_json`] then [`Manifest::validate`].
    pub fn parse_and_validate(json: &str) -> Result<Self, ManifestError> {
        let manifest = Self::from_json(json)?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Checks every rule beyond structure; returns all problems found.
    pub fn validate(&self) -> Result<(), ValidationErrors> {
        let errors = validate::check(self).errors;
        if errors.is_empty() {
            Ok(())
        } else {
            Err(ValidationErrors(errors))
        }
    }

    /// Advice for reviewers that does not make the manifest invalid (e.g.
    /// `retrySafe` on an action that modifies something).
    pub fn review_warnings(&self) -> Vec<Issue> {
        validate::check(self).warnings
    }

    /// Pretty JSON, two-space indented.
    ///
    /// # Panics
    ///
    /// Never: every field serializes to JSON (map keys are strings).
    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).expect("a manifest always serializes")
    }

    /// Whether the plugin runs on `platform` (`platforms` omitted: all).
    pub fn supports_platform(&self, platform: Platform) -> bool {
        self.platforms
            .as_ref()
            .is_none_or(|platforms| platforms.contains(&platform))
    }

    /// Whether the plugin can run on DeckPilot `app_version` on `platform`.
    ///
    /// The plugin API level is a separate axis (it is not tied to the app
    /// version number); [`Manifest::check_compatibility`] checks it too.
    pub fn is_compatible(&self, app_version: &semver::Version, platform: Platform) -> bool {
        self.engines.deckpilot.matches(app_version) && self.supports_platform(platform)
    }

    /// Every reason the plugin cannot run on `host`, worded for the user.
    pub fn check_compatibility(&self, host: &HostEnvironment) -> Result<(), Vec<Incompatibility>> {
        let mut reasons = Vec::new();
        if !self.engines.deckpilot.matches(&host.app_version) {
            reasons.push(Incompatibility::AppVersion {
                required: self.engines.deckpilot.clone(),
                actual: host.app_version.clone(),
            });
        }
        if !self.engines.plugin_api.is_satisfied_by(host.plugin_api) {
            reasons.push(Incompatibility::PluginApi {
                required: self.engines.plugin_api,
                actual: host.plugin_api,
            });
        }
        if !self.supports_platform(host.platform) {
            reasons.push(Incompatibility::Platform {
                supported: self.platforms.clone().unwrap_or_default(),
                actual: host.platform,
            });
        }
        if reasons.is_empty() {
            Ok(())
        } else {
            Err(reasons)
        }
    }

    /// The declared permission of `kind`, if any (at most one per kind).
    pub fn permission(&self, kind: PermissionKind) -> Option<&Permission> {
        self.permissions.iter().find(|p| p.kind() == kind)
    }
}

/// `serde_json` appends " at line L column C"; the errors here carry the
/// position in their own fields.
fn strip_position(error: &serde_json::Error) -> String {
    let text = error.to_string();
    match text.rfind(" at line ") {
        Some(at) => text[..at].to_owned(),
        None => text,
    }
}
