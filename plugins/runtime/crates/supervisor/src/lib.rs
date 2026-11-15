//! Agent side of the plugin runtime (design §3.7): one supervised host
//! process per plugin.
//!
//! * [`HostCommand`] spawns the host with an **empty environment**, a fixed
//!   working directory, stdin/stdout as the only channel and stderr
//!   discarded. Rust's standard library opens every descriptor
//!   close-on-exec and `fds.rs` marks inherited ones (Unix); on Windows the
//!   host gets exactly its two pipe handles (handle list) and runs in an
//!   AppContainer without capabilities inside a supervisor-held Job Object
//!   (`windows.rs`). Nothing of the agent's IPC (token, socket, pipe)
//!   reaches the host.
//! * [`PluginHost`] performs the handshake (`ready`, sandbox check, `load`,
//!   `loaded`), runs invocations with a wall-clock timeout, relays
//!   capability requests to the broker [`Session`] bound to this process,
//!   kills the host on a timeout or any protocol violation and collects its
//!   exit status.
//! * [`Supervisor`] adds the restart policy: exponential backoff and a
//!   crash-loop breaker that disables the plugin (design §3.6).
#![deny(unsafe_code)]

#[cfg(not(windows))]
mod fds;
mod host;
mod process;
mod supervisor;
#[cfg(windows)]
mod windows;

pub use host::{
    AbortSignal, CapabilityExecutor, HostCommand, HostConfig, HostError, PluginHost,
    fixed_working_dir,
};
pub use process::{HostProcess, HostStdin, HostStdout};
pub use supervisor::{RestartPolicy, Supervisor, SupervisorError, SupervisorState};
#[cfg(windows)]
pub use windows::PROFILE_PREFIX as APP_CONTAINER_PROFILE_PREFIX;

pub use deckpilot_plugin_broker::Session;
