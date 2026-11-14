//! Host protocol 1: messages between the supervisor and one plugin host.
//!
//! Order on the channel:
//!
//! 1. host → `ready` (after the host has sandboxed itself)
//! 2. supervisor → `load` (module bytes and limits; the only large frame)
//! 3. host → `loaded`, or `fatal` and exit
//! 4. any number of: supervisor → `invoke`; host → `call` / supervisor →
//!    `reply` (zero or more, strictly alternating); host → `result`
//! 5. supervisor → `shutdown`, or the supervisor closes the channel
//!
//! Every message is checked against this order; anything else is a protocol
//! violation and ends the channel. No message identifies a plugin: the
//! supervisor knows which plugin is at the other end of each channel.

use serde::{Deserialize, Serialize};

use crate::capability::{CallResult, CapabilityRequest};
use crate::describe::PluginDescriptor;
use crate::value::{ActionOutcome, Params};

/// Version of this protocol, sent in `ready`.
pub const PROTOCOL_VERSION: u32 = 1;

/// Supervisor to host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum ToHost {
    #[serde(rename_all = "camelCase")]
    Load {
        abi_version: u32,
        /// The wasm module, base64 (standard alphabet, padded).
        #[serde(with = "base64_bytes")]
        module: Vec<u8>,
        limits: GuestLimits,
    },
    #[serde(rename_all = "camelCase")]
    Invoke {
        invocation_id: u64,
        action: String,
        #[serde(default)]
        params: Params,
    },
    #[serde(rename_all = "camelCase")]
    Reply {
        invocation_id: u64,
        call_id: u64,
        result: CallResult,
    },
    /// A struct variant (not a unit variant) so that unknown fields are
    /// rejected like everywhere else.
    Shutdown {},
}

/// Host to supervisor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum ToSupervisor {
    #[serde(rename_all = "camelCase")]
    Ready {
        protocol: u32,
        host_version: String,
        engine: String,
        sandbox: SandboxReport,
    },
    #[serde(rename_all = "camelCase")]
    Loaded {
        abi_version: u32,
        /// What the module's optional `dp_describe` returned (validated by
        /// the host). Absent for modules without that export; omitted from
        /// the JSON then, so `loaded` keeps its original ABI 1 form.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        descriptor: Option<PluginDescriptor>,
    },
    #[serde(rename_all = "camelCase")]
    Call {
        invocation_id: u64,
        call_id: u64,
        request: CapabilityRequest,
    },
    #[serde(rename_all = "camelCase")]
    Result {
        invocation_id: u64,
        outcome: ActionOutcome,
    },
    /// Sent (best effort) right before the host exits with a non-zero code.
    Fatal { reason: FatalReason, detail: String },
}

/// What the host did to confine itself before reading any plugin byte.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SandboxReport {
    /// e.g. `macos-seatbelt`, `linux-landlock-seccomp`, `windows-job`, `none`.
    pub mechanism: String,
    pub enforcement: Enforcement,
    /// Human-readable notes (e.g. the Landlock ABI that was available).
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Enforcement {
    /// Every layer the platform implementation has was applied.
    Full,
    /// Some layer was unavailable (e.g. an old kernel without Landlock).
    Partial,
    /// No OS sandbox (only with the host's explicit test/development flag).
    None,
}

/// Per-guest limits chosen by the supervisor. The host clamps each value to
/// its own hard maximum, so a wrong value can only make limits stricter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuestLimits {
    /// Continuous guest execution between two host yields, in milliseconds.
    pub deadline_ms: u64,
    /// Fuel per invocation (wasm instructions, roughly).
    pub fuel_per_invocation: u64,
    /// Linear memory cap in bytes.
    pub memory_bytes: u64,
    /// Table elements across the guest's tables.
    pub table_elements: u64,
}

impl Default for GuestLimits {
    /// Values from the design (§3.6).
    fn default() -> Self {
        Self {
            deadline_ms: 250,
            fuel_per_invocation: 5_000_000_000,
            memory_bytes: 64 << 20,
            table_elements: 20_000,
        }
    }
}

/// Why a host is about to exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FatalReason {
    /// The supervisor sent something out of order or malformed.
    ProtocolViolation,
    /// The module was rejected (forbidden import, missing export, limits).
    LoadFailed,
    /// The guest trapped (panic, unreachable, out-of-bounds, stack overflow).
    Trap,
    /// The epoch deadline passed.
    Deadline,
    /// The fuel budget ran out.
    OutOfFuel,
    /// The guest broke the guest ABI (bad buffer, too many host calls).
    GuestAbiViolation,
}

impl FatalReason {
    /// The host's exit code for this reason.
    pub fn exit_code(self) -> i32 {
        match self {
            Self::ProtocolViolation => exit_code::PROTOCOL_VIOLATION,
            Self::LoadFailed => exit_code::LOAD_FAILED,
            Self::Trap => exit_code::TRAP,
            Self::Deadline => exit_code::DEADLINE,
            Self::OutOfFuel => exit_code::OUT_OF_FUEL,
            Self::GuestAbiViolation => exit_code::GUEST_ABI_VIOLATION,
        }
    }
}

/// Exit codes of the host process.
pub mod exit_code {
    pub const CLEAN: i32 = 0;
    pub const USAGE: i32 = 2;
    pub const PROTOCOL_VIOLATION: i32 = 3;
    pub const LOAD_FAILED: i32 = 4;
    pub const SANDBOX_FAILED: i32 = 5;
    pub const IO: i32 = 6;
    pub const TRAP: i32 = 10;
    pub const DEADLINE: i32 = 11;
    pub const OUT_OF_FUEL: i32 = 12;
    pub const GUEST_ABI_VIOLATION: i32 = 13;
}

mod base64_bytes {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        STANDARD
            .decode(s.as_bytes())
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_carries_module_bytes() {
        let msg = ToHost::Load {
            abi_version: 1,
            module: vec![0, 97, 115, 109, 255],
            limits: GuestLimits::default(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""module":"AGFzbf8=""#), "{json}");
        assert_eq!(serde_json::from_str::<ToHost>(&json).unwrap(), msg);
        let bad = json.replace("AGFzbf8=", "!!!");
        assert!(serde_json::from_str::<ToHost>(&bad).is_err());
    }

    #[test]
    fn loaded_without_descriptor_keeps_its_original_form() {
        let old = r#"{"type":"loaded","abiVersion":1}"#;
        let m: ToSupervisor = serde_json::from_str(old).unwrap();
        assert_eq!(
            m,
            ToSupervisor::Loaded {
                abi_version: 1,
                descriptor: None
            }
        );
        assert_eq!(serde_json::to_string(&m).unwrap(), old);
        let new = r#"{"type":"loaded","abiVersion":1,"descriptor":{"sdk":"s/1","apiLevel":"1.0","actions":[{"id":"a"}]}}"#;
        let m: ToSupervisor = serde_json::from_str(new).unwrap();
        let ToSupervisor::Loaded {
            descriptor: Some(d),
            ..
        } = &m
        else {
            panic!("{m:?}")
        };
        assert_eq!(d.actions[0].id, "a");
        assert!(
            serde_json::from_str::<ToSupervisor>(
                r#"{"type":"loaded","abiVersion":1,"descriptor":{"sdk":"s","apiLevel":"1.0","actions":[],"pluginId":"x"}}"#
            )
            .is_err()
        );
    }

    #[test]
    fn messages_are_strict() {
        for bad in [
            r#"{"type":"shutdown","pluginId":"x"}"#,
            r#"{"type":"invoke","invocationId":1,"action":"a","pluginId":"x"}"#,
            r#"{"type":"getDatabaseKey"}"#,
            r#"{"invocationId":1}"#,
        ] {
            assert!(serde_json::from_str::<ToHost>(bad).is_err(), "{bad}");
        }
        assert!(
            serde_json::from_str::<ToSupervisor>(
                r#"{"type":"call","invocationId":1,"callId":1,"plugin":"other","request":{"op":"log","args":{"level":"info","message":"m"}}}"#
            )
            .is_err()
        );
    }
}
