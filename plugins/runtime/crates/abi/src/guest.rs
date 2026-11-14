//! Guest ABI 1: the contract between the host and a wasm module.
//!
//! A module **must** export:
//!
//! | Export | Type | Meaning |
//! | --- | --- | --- |
//! | `memory` | memory | the guest's single linear memory |
//! | `dp_abi_version` | `() -> i32` | returns [`ABI_VERSION`](crate::ABI_VERSION) |
//! | `dp_alloc` | `(len: i32) -> i32` | returns a pointer to `len` writable bytes the guest owns (0 on failure) |
//! | `dp_invoke` | `(ptr: i32, len: i32) -> i64` | runs one invocation; input is an [`InvokeInput`] JSON at `ptr..ptr+len` (allocated with `dp_alloc`, owned by the guest afterwards); returns `ptr << 32 \| len` of an [`ActionOutcome`](crate::ActionOutcome) JSON that stays valid until the next `dp_invoke` |
//!
//! A module **may** additionally export (compatible additions within ABI 1:
//! a module without them loads and runs exactly as before, and a host that
//! predates them simply never calls them):
//!
//! | Export | Type | Meaning |
//! | --- | --- | --- |
//! | `dp_describe` | `() -> i64` | `ptr << 32 \| len` of a [`PluginDescriptor`](crate::describe::PluginDescriptor) JSON (at most [`MAX_DESCRIPTOR`](crate::limits::MAX_DESCRIPTOR) bytes); called once after `dp_abi_version`, reported in `loaded` |
//! | `dp_init` | `() -> i32` | lifecycle hook, called once after `dp_describe`; 0 = ready, anything else fails the load |
//! | `dp_shutdown` | `() -> ()` | lifecycle hook, called once on `shutdown` before the host exits |
//!
//! All three run under the same deadline and fuel limits as an invocation.
//! `host_call` from inside them gets an `unavailable` error reply without
//! reaching the supervisor (there is no invocation to relay it for). An
//! export with one of these names but another type fails the load.
//!
//! A module **may** import exactly one function and nothing else:
//!
//! | Import | Type | Meaning |
//! | --- | --- | --- |
//! | `deckpilot_v1.host_call` | `(ptr: i32, len: i32) -> i64` | sends a [`CapabilityRequest`](crate::CapabilityRequest) JSON; returns `ptr << 32 \| len` of a [`CallResult`](crate::CallResult) JSON written into memory obtained from `dp_alloc` (owned by the guest) |
//!
//! Any other import (WASI included) makes the module fail to load. A
//! `host_call` with an out-of-bounds or oversized buffer, or more than
//! [`MAX_HOST_CALLS_PER_INVOCATION`](crate::limits::MAX_HOST_CALLS_PER_INVOCATION)
//! calls in one invocation, is a guest fault: the invocation traps and the
//! host process exits. A request that is merely malformed or not permitted
//! gets an error reply.

use serde::{Deserialize, Serialize};

use crate::value::Params;

pub const IMPORT_MODULE: &str = "deckpilot_v1";
pub const IMPORT_HOST_CALL: &str = "host_call";

pub const EXPORT_MEMORY: &str = "memory";
pub const EXPORT_ABI_VERSION: &str = "dp_abi_version";
pub const EXPORT_ALLOC: &str = "dp_alloc";
pub const EXPORT_INVOKE: &str = "dp_invoke";
/// Optional: `() -> i64`, the plugin descriptor.
pub const EXPORT_DESCRIBE: &str = "dp_describe";
/// Optional: `() -> i32`, the init hook (0 = ready).
pub const EXPORT_INIT: &str = "dp_init";
/// Optional: `() -> ()`, the shutdown hook.
pub const EXPORT_SHUTDOWN: &str = "dp_shutdown";

/// Input of one `dp_invoke`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvokeInput {
    pub action: String,
    #[serde(default)]
    pub params: Params,
}

/// Packs a guest pointer and length into the `i64` returned across the ABI.
pub fn pack(ptr: u32, len: u32) -> u64 {
    (u64::from(ptr) << 32) | u64::from(len)
}

/// Inverse of [`pack`].
pub fn unpack(packed: u64) -> (u32, u32) {
    ((packed >> 32) as u32, packed as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_round_trips() {
        for (p, l) in [(0, 0), (1, 2), (u32::MAX, u32::MAX), (0x1000, 0)] {
            assert_eq!(unpack(pack(p, l)), (p, l));
        }
    }
}
