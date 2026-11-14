//! Agent side of the plugin runtime (design §3.7): one supervised host
//! process per plugin.
//!
//! * [`HostCommand`] spawns the host with an **empty environment**, a fixed
//!   working directory, stdin/stdout as the only channel and stderr
//!   discarded. Rust's standard library opens every descriptor
//!   close-on-exec (Unix) and the agent's own handles are not inheritable
//!   (Windows), so nothing of the agent's IPC (token, socket, pipe) reaches
//!   the host.
//! * [`PluginHost`] performs the handshake (`ready`, sandbox check, `load`,
//!   `loaded`), runs invocations with a wall-clock timeout, relays
//!   capability requests to the broker [`Session`] bound to this process,
//!   kills the host on a timeout or any protocol violation and collects its
//!   exit status.
//! * [`Supervisor`] adds the restart policy: exponential backoff and a
//!   crash-loop breaker that disables the plugin (design §3.6).
#![deny(unsafe_code)]

mod fds;
mod host;
mod supervisor;

pub use host::{
    CapabilityExecutor, HostCommand, HostConfig, HostError, PluginHost, fixed_working_dir,
};
pub use supervisor::{RestartPolicy, Supervisor, SupervisorError, SupervisorState};

pub use deckpilot_plugin_broker::Session;
