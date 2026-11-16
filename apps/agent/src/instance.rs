//! Single-instance guarantee per user and data directory.
//!
//! The lock itself lives in `deckpilot_ipc::instance`, so that the
//! configuration app can prove that a stopped agent has released it before
//! it starts the next one (docs/distribution/README.md, section 7.1,
//! step 6). The agent and the app use the same code, and so the same OS
//! lock.

pub use deckpilot_ipc::instance::{InstanceLock, LockError, STARTUP_LOCK_WAIT};
