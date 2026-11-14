//! Frames: 4-byte big-endian length, then UTF-8 JSON (design §3.8).

use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

/// Supervisor to host frame limit (design: 2 MiB).
pub const MAX_TO_HOST: usize = 2 << 20;
/// Host to supervisor frame limit (design: 256 KiB).
pub const MAX_TO_SUPERVISOR: usize = 256 << 10;

/// Supervisor (agent) to plugin host.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ToHost {
    Ping {
        seq: u64,
    },
    /// Runs a guest export in a fresh Store. `entry` is one of the guest's
    /// exports; `input` is passed to `relay` as request bytes.
    #[serde(rename_all = "camelCase")]
    Invoke {
        invocation_id: u64,
        entry: String,
        input: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Reply {
        call_id: u64,
        ok: Option<Value>,
        error: Option<String>,
    },
    /// Test hook: the host process itself (native code, outside wasm)
    /// attempts an operation, to check the OS sandbox.
    Probe {
        kind: String,
        target: String,
    },
    /// Test hook: abort the host process.
    Crash,
    /// Test hook: hang the host process in native code.
    Hang,
    Shutdown,
}

/// Plugin host to supervisor.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ToSupervisor {
    #[serde(rename_all = "camelCase")]
    Ready {
        host_version: String,
        engine: String,
        sandbox: String,
    },
    Pong {
        seq: u64,
    },
    /// One capability request from the guest. Carries no plugin id: the
    /// supervisor knows which plugin is on the other end of the channel.
    #[serde(rename_all = "camelCase")]
    Call {
        invocation_id: u64,
        call_id: u64,
        op: String,
        args: Value,
    },
    #[serde(rename_all = "camelCase")]
    Result {
        invocation_id: u64,
        ok: Option<Value>,
        error: Option<String>,
    },
    ProbeResult {
        ok: bool,
        detail: String,
    },
}

pub fn write_frame<W: Write, T: Serialize>(w: &mut W, msg: &T) -> io::Result<()> {
    let body = serde_json::to_vec(msg).map_err(io::Error::other)?;
    let len = u32::try_from(body.len()).map_err(io::Error::other)?;
    w.write_all(&len.to_be_bytes())?;
    w.write_all(&body)?;
    w.flush()
}

/// Reads one frame. `Ok(None)` on clean EOF before a frame starts.
pub fn read_frame<R: Read, T: DeserializeOwned>(r: &mut R, max: usize) -> io::Result<Option<T>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_be_bytes(len) as usize;
    if len > max {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame of {len} bytes exceeds {max}"),
        ));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}
