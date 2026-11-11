//! DeckPilot background agent.
//!
//! The agent owns device communication and action execution. It keeps
//! running without the configuration app, which talks to it over local IPC.
//! See `docs/background-agent.md` for the architecture.

pub mod autostart;
mod devices;
pub mod instance;
pub mod launcher;
pub mod notifications;
pub mod render;
mod server;
pub mod service;
pub mod tray;

pub use instance::{InstanceLock, LockError};
pub use service::{
    AUTO_LOCK_CHECK, Agent, AgentError, AgentHandle, AgentOptions, Clock, TrayCommand,
    UNLOCK_WINDOW, WORKFLOW_SUMMARY_WINDOW, WindowOpener, default_options, system_clock,
};
