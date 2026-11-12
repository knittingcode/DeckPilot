//! Application updates distributed through GitHub Releases.
//!
//! Downloading and installing is done by the Tauri updater in the
//! configuration app, which verifies every artifact's minisign signature
//! against the public key compiled into the app. This crate contains the
//! parts that are independent of Tauri and testable on their own:
//!
//! * [`channel`]: release tags, their kind (stable, pre-release, test
//!   build) and the update channels;
//! * [`manifest`]: the `latest.json` format, validation and selection of
//!   the artifact for this platform;
//! * [`signature`]: signature verification, also used by the release
//!   pipeline to check artifacts before publishing;
//! * [`policy`]: update modes, check scheduling and when installing is
//!   safe;
//! * [`compatibility`]: whether a running agent fits the application.

pub mod channel;
pub mod compatibility;
pub mod manifest;
pub mod policy;
pub mod signature;

/// Repository whose releases provide updates.
pub const REPOSITORY: &str = "knittingcode/DeckPilot";

/// Update manifest of the latest stable release. GitHub's `latest` alias
/// never points at drafts or pre-releases, which makes it the stable channel.
pub const STABLE_ENDPOINT: &str =
    "https://github.com/knittingcode/DeckPilot/releases/latest/download/latest.json";

/// Prefix that every artifact URL in a manifest must have.
pub const DOWNLOAD_PREFIX: &str = "https://github.com/knittingcode/DeckPilot/releases/download/";
