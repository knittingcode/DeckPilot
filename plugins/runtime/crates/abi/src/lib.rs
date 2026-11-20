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
pub mod describe;
pub mod dialog;
pub mod frame;
pub mod guest;
pub mod limits;
pub mod locale;
pub mod protocol;
pub mod state;
pub mod value;

pub use capability::{
    CallError, CallErrorCode, CallResult, CapabilityRequest, HttpMethod, NetworkErrorKind,
};
pub use state::{KeyState, StateOutcome};
pub use value::{ActionOutcome, ParamValue, Params, ValidationError};

/// The ABI version implemented by this crate.
///
/// Compatible additions made within ABI 1 (each optional, so modules and
/// messages without them are unchanged): the optional guest exports
/// `dp_describe`, `dp_init` and `dp_shutdown` ([`guest`]); `auth` (a
/// credential handle) in `http.fetch` and the [`capability::HttpResponse`]
/// shape of its reply; `descriptor` in the `loaded` message; the
/// capability operations `storage.keys`, `storage.usage`,
/// `credentials.has` and `settings.get`; the optional export
/// `dp_features` and, for a guest that reports
/// [`guest::features::NETWORK_ERROR_KIND`] there, the `kind` of a failed
/// `http.fetch` ([`capability::NetworkErrorKind`]); the capability
/// operation `run.context` ([`capability::RunContext`], G8), which only a
/// guest that reads it sends; and, for a guest that reports
/// [`guest::features::NOTIFY_DELIVERY`], where a `notify` was shown
/// ([`capability::NotifyDelivery`], G9) instead of `null`; and the
/// capability operations `time.now` ([`capability::Timestamp`]) and
/// `random.bytes` ([`capability::RandomReply`]) of plugin API 1.1, which
/// only a guest that uses them sends; and the optional guest export
/// `dp_state` of plugin API 1.2 ([`state`]), with the host protocol
/// messages `state` and `stateResult` that carry it; and the capability
/// operation `dialog.open` of plugin API 1.5 ([`dialog`]), which only a
/// guest that uses it sends; and the capability operations `locale.get`
/// and `locale.translate` of plugin API 1.6 ([`locale`]), which only a
/// guest that uses them sends.
pub const ABI_VERSION: u32 = 1;
