//! The DeckPilot plugin host (design §3.5): one process per plugin.
//!
//! * [`sandbox`] confines the process (macOS Seatbelt, Linux Landlock +
//!   seccomp, Windows Job Object + low integrity) before any plugin byte is
//!   read.
//! * [`engine`] runs the plugin's wasm module in wasmtime with no WASI and
//!   exactly one import, under an epoch deadline, fuel, memory, table and
//!   instance limits and a bounded wasm stack.
//! * [`channel`] speaks host protocol 1 on stdin/stdout. The module bytes
//!   arrive over the channel, so the host never opens a file.
//!
//! After any trap, deadline, fuel exhaustion or guest ABI violation the
//! host reports `fatal` and **exits**; a Store is never reused after a trap
//! and the supervisor decides about restarting.

pub mod channel;
pub mod engine;
pub mod sandbox;
