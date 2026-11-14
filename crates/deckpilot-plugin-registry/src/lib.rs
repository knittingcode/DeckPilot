//! **Experimental.** Client-side verification of the signed, static
//! DeckPilot plugin registry.
//!
//! Nothing in DeckPilot uses this crate yet: no app, agent or module crate
//! depends on it, and plugins stay disabled. It implements sections 9, 10,
//! 12.3 (steps 1 and 2), 13.5 and 14 of the plugin marketplace design
//! (`docs/marketplace/architecture.md`, proposed in pull request #191),
//! promoting the trust spike of `experiments/marketplace-spikes/trust`.
//!
//! There is no network code. The client takes document bytes (directly,
//! or through a [`RegistrySource`] the agent's fetcher implements) and a
//! [`TrustStore`] for its state.
//!
//! ```text
//! embedded roots ─signs→ keys/<n>.json ─names→ index + timestamp keys
//! timestamp.json ─pins (sha256, length)→ index/<seq>.json
//! index/<seq>.json ─pins→ plugins/<id>/<sha256>.json ─pins→ .dpplugin
//! ```
//!
//! Decisions on the spike's open findings are documented in the README of
//! this crate: freshness comes from the timestamp alone (the index expiry
//! is not enforced by default), revocations can be lifted only by a
//! root-signed key set with `seq_floor_reset`, and `seq_floor` is a floor
//! (`max`) unless reset.

mod client;
mod datetime;
mod error;
mod model;
pub mod sig;
mod state;

pub use client::{
    Freshness, InstallTarget, InstalledVerdict, MAX_KEYSET_BYTES, MAX_KEYSET_SIGNATURES,
    MAX_KEYSET_SIGNATURES_BYTES, MAX_SIGNATURE_BYTES, MAX_TIMESTAMP_BYTES, Policy, Refresh,
    RegistryClient, RegistrySource, StaleReason, VerifiedTimestamp,
};
pub use error::{FetchError, InstallError, RegistryError};
pub use model::*;
pub use state::{
    FileTrustStore, MemoryTrustStore, STATE_FORMAT, StoreError, TrustState, TrustStore,
    write_atomic,
};

/// The #191 registry JSON Schemas (draft 2020-12), unchanged. They refer to
/// the manifest schema (`deckpilot_plugin_manifest::SCHEMA_JSON`) by its
/// `$id`.
pub mod schemas {
    pub const KEYSET: &str = include_str!("../schema/registry-keyset.schema.json");
    pub const TIMESTAMP: &str = include_str!("../schema/registry-timestamp.schema.json");
    pub const INDEX: &str = include_str!("../schema/registry-index.schema.json");
    pub const PLUGIN: &str = include_str!("../schema/registry-plugin.schema.json");
}
