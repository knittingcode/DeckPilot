//! **Experimental.** The DeckPilot plugin package (`.dpplugin`): a
//! deterministic builder and a safe reader.
//!
//! Nothing in DeckPilot uses this crate yet: no app, agent or module crate
//! depends on it, and plugins stay disabled. It implements section 8 of the
//! plugin marketplace design (`docs/marketplace/architecture.md`, proposed
//! in pull request #191) and the package steps of sections 9.3 (step 5),
//! 12.3 (steps 4 to 8) and 12.5.
//!
//! A package is a gzip-compressed POSIX ustar archive of regular files:
//! `manifest.json`, `plugin.js` (the bundle), `LICENSE`, optionally
//! `README.md` and `CHANGELOG.md`, and the `assets/` the manifest names.
//!
//! ```text
//! registry pins (sha256, size, manifest_sha256, id, version, permissions)
//!        │
//!        ▼
//! verify_package(bytes, &ExpectedPackage, &Limits)   no decompression before
//!        │                                           size and sha256 match
//!        ▼
//! VerifiedPackage ── unpack(staging_root, target)    staging dir + one rename
//!        │
//!        ▼
//! integrity() ── stored; verify_installed(dir, …) at every load
//! ```
//!
//! Only a [`VerifiedPackage`] can be unpacked, and only [`verify_package`]
//! makes one, so unpacking cannot happen before verification.

mod build;
mod contents;
mod error;
mod limits;
mod paths;
mod read;
mod unpack;

pub use build::{BuiltPackage, GZIP_OS_UNKNOWN, PackageBuilder};
pub use contents::{BUNDLE, Files, MANIFEST};
pub use error::{PackageError, PathError};
pub use limits::Limits;
pub use paths::{check_entry_path, check_entry_set};
pub use read::{ExpectedPackage, VerifiedPackage, read_archive, verify_package};
pub use unpack::verify_installed;

use deckpilot_plugin_manifest::Sha256Digest;
use sha2::{Digest as _, Sha256};

/// The sha256 digest of `bytes`.
pub fn sha256(bytes: &[u8]) -> Sha256Digest {
    Sha256Digest::from_bytes(Sha256::digest(bytes).into())
}
