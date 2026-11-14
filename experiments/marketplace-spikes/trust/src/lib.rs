//! Throwaway feasibility code for the plugin marketplace trust model of
//! `docs/marketplace/architecture.md` (PR #191). Not production code; see
//! `RESULTS.md`.
//!
//! * [`package`]: spike 4, deterministic packages and their verification.
//! * [`client`]: spike 5, the client-side verifier of the static registry.
//! * [`sig`]: role and trusted-comment checks over the reused
//!   `deckpilot_update::signature::verify`.
//! * [`signer`]: registry-side signing with throwaway in-memory keys.
//! * [`registry`]: the registry documents.

pub mod client;
pub mod package;
pub mod registry;
pub mod sig;
pub mod signer;
