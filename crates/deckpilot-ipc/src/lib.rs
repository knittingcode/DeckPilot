//! Local IPC between the DeckPilot background agent and its clients.
//!
//! * [`paths`]: where the agent keeps its database, logs, lock and token.
//! * [`endpoint`]: the per-user local socket (Unix domain socket or Windows
//!   named pipe) and how it is secured.
//! * [`protocol`]: versioned request, response and event types.
//! * [`framing`]: newline-delimited JSON with a size limit.
//! * [`client`]: an async client used by the configuration app.
//! * [`work_session`]: the work session shapes of protocol 8.
//!
//! Every connection starts with a `hello` carrying the protocol version and a
//! random token that the agent writes to a file readable only by the current
//! user. The socket itself is also restricted to the current user, so the
//! token is a second, platform-independent layer of access control.

pub mod client;
pub mod endpoint;
pub mod framing;
pub mod paths;
pub mod protocol;
pub mod stream;
pub mod token;
pub mod work_session;

pub use client::{AgentClient, ClientError};
pub use endpoint::Endpoint;
pub use paths::AgentPaths;
pub use protocol::*;
pub use stream::SharedStream;
