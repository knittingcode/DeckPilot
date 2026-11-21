//! **Experimental, feature `publish`.** Offline tooling that produces the
//! static registry site the client verifies (architecture sections 9 to
//! 11): keys for local and test use, root-signed key sets, per-plugin
//! version history, revocations, the signed index and the timestamp, and
//! an end-to-end check of the result with this crate's own client.
//!
//! Everything works on plain files in one working directory
//! ([`Workspace`]); nothing here touches the network. The documents it
//! writes are deterministic (same inputs, same bytes); only the minisign
//! signatures are randomized. Secrets are read from files, environment
//! variables or standard input and never written to the output.
//!
//! The command line is [`run`]; the `deckpilot-registry` binary is a thin
//! wrapper around it.

mod catalog;
mod cli;
mod index;
mod keys;
mod keyset;
mod layout;
mod pack;
mod release;
mod verify;

use std::fmt;

pub use catalog::PluginInfo;
pub use cli::{Io, run};
pub use keys::{KeyRole, Password, SigningKey, generate_key};
pub use layout::{SiteSource, Workspace};
pub use pack::package;
pub use release::{VerifyRequest, floor_from_released_pins, sign as sign_release};
pub use verify::{SiteReport, verify_site};

/// Why a publishing command failed. The message never contains a secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishError(pub String);

impl fmt::Display for PublishError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PublishError {}

impl From<String> for PublishError {
    fn from(message: String) -> Self {
        Self(message)
    }
}

impl From<&str> for PublishError {
    fn from(message: &str) -> Self {
        Self(message.to_owned())
    }
}

pub type Result<T, E = PublishError> = std::result::Result<T, E>;

fn fail<T>(message: impl Into<String>) -> Result<T> {
    Err(PublishError(message.into()))
}

/// The canonical encoding of every document this tool writes: pretty JSON
/// (two-space indent, struct field order) and a final newline.
pub(crate) fn to_json<T: serde::Serialize>(value: &T) -> Result<Vec<u8>> {
    let mut bytes =
        serde_json::to_vec_pretty(value).map_err(|e| PublishError(format!("encoding: {e}")))?;
    bytes.push(b'\n');
    Ok(bytes)
}
