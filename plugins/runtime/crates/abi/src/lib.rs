//! DeckPilot plugin ABI, version 1.
//!
//! Two interfaces are defined here, both versioned together as ABI 1:
//!
//! * **Guest ABI** ([`guest`]): what a wasm module must export and may
//!   import. The only import is [`guest::IMPORT_MODULE`]`.`[`guest::IMPORT_HOST_CALL`];
//!   everything the guest exchanges with the host is UTF-8 JSON in linear
//!   memory.
//! * **Host protocol** ([`protocol`], [`frame`]): the private channel between
//!   the supervisor (in the agent) and one plugin host process. Frames are a
//!   4-byte big-endian length followed by UTF-8 JSON.
//!
//! The payload types shared by both ([`value`], [`capability`]) are strict:
//! unknown fields are rejected, every string and collection is bounded
//! ([`limits`]), and no message carries a plugin id (identity comes from the
//! channel, never from a message).
#![forbid(unsafe_code)]

pub mod capability;
pub mod frame;
pub mod guest;
pub mod limits;
pub mod protocol;
pub mod value;

pub use capability::{CallError, CallErrorCode, CallResult, CapabilityRequest, HttpMethod};
pub use value::{ActionOutcome, ParamValue, Params, ValidationError};

/// The ABI version implemented by this crate.
pub const ABI_VERSION: u32 = 1;
