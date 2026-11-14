//! Plugin runtime feasibility spike. See RESULTS.md.
//!
//! - [`engine`]: wasmtime host for the guest (spike 1).
//! - [`protocol`]: length-prefixed JSON frames between supervisor and plugin host (spike 2).
//! - [`supervisor`]: spawns and supervises one plugin host process (spike 2).
//! - [`broker`]: deny-by-default permission broker in the supervisor (spike 3).
//! - [`sandbox`]: OS sandbox wrapper; macOS only (spike 4).

pub mod broker;
pub mod engine;
pub mod protocol;
pub mod sandbox;
pub mod supervisor;
