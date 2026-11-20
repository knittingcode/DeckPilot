//! Local IPC between the DeckPilot background agent and its clients.
//!
//! * [`paths`]: where the agent keeps its database, logs, lock and token.
//! * [`instance`]: the agent's single-instance lock, and the probe with
//!   which a client proves that a stopped agent released it.
//! * [`endpoint`]: the per-user local socket (Unix domain socket or Windows
//!   named pipe) and how it is secured.
//! * [`protocol`]: versioned request, response and event types.
//! * [`framing`]: newline-delimited JSON with a size limit.
//! * [`client`]: an async client used by the configuration app.
//! * [`work_session`]: the work session shapes of protocol 8.
//! * [`plugins`]: the plugin shapes of protocol 10.
//! * [`plugin_dialogs`]: plugin dialogs of protocol 12.
//! * [`plugin_answers`]: plugin answers that fit one message (lazy
//!   listings, pages, icon chunks), on both sides.
//! * [`simulation`]: simulated presses of protocol 14.
//! * [`live_preview`]: the editor's live preview of protocol 16.
//!
//! Every connection starts with a `hello` carrying the protocol version and a
//! random token that the agent writes to a file readable only by the current
//! user. The socket itself is also restricted to the current user, so the
//! token is a second, platform-independent layer of access control.

pub mod client;
pub mod endpoint;
pub mod framing;
pub mod instance;
pub mod live_preview;
pub mod paths;
pub mod plugin_answers;
pub mod plugin_dialogs;
pub mod plugins;
pub mod protocol;
pub mod simulation;
pub mod stream;
pub mod token;
pub mod work_session;

pub use client::{AgentClient, ClientError, MessageReceiver, RequestWriter};
pub use endpoint::Endpoint;
pub use paths::AgentPaths;
pub use protocol::*;
pub use stream::SharedStream;
