//! Host side of host protocol 1 (see `deckpilot_plugin_abi::protocol`).

use std::cell::RefCell;
use std::fmt;
use std::io::{Read, Write};
use std::rc::Rc;

use deckpilot_plugin_abi::frame::{FrameError, read_frame, write_frame};
use deckpilot_plugin_abi::limits::{MAX_FRAME_TO_HOST, MAX_FRAME_TO_SUPERVISOR, MAX_LOAD_FRAME};
use deckpilot_plugin_abi::protocol::{
    FatalReason, PROTOCOL_VERSION, SandboxReport, ToHost, ToSupervisor, exit_code,
};
use deckpilot_plugin_abi::{ABI_VERSION, CallResult, CapabilityRequest};

use crate::engine::{ChannelError, ENGINE_NAME, Fault, HostCallHandler, PluginEngine};

struct Channel<R, W> {
    reader: R,
    writer: W,
}

impl<R: Read, W: Write> Channel<R, W> {
    fn send(&mut self, msg: &ToSupervisor) -> Result<(), FrameError> {
        write_frame(&mut self.writer, msg, MAX_FRAME_TO_SUPERVISOR)
    }

    fn recv(&mut self, max: usize) -> Result<Option<ToHost>, FrameError> {
        read_frame(&mut self.reader, max)
    }
}

struct Handler<R, W> {
    channel: Rc<RefCell<Channel<R, W>>>,
}

impl<R: Read, W: Write> HostCallHandler for Handler<R, W> {
    fn call(
        &mut self,
        invocation_id: u64,
        call_id: u64,
        request: CapabilityRequest,
    ) -> Result<CallResult, ChannelError> {
        let mut ch = self.channel.borrow_mut();
        ch.send(&ToSupervisor::Call {
            invocation_id,
            call_id,
            request,
        })
        .map_err(|e| ChannelError(e.to_string()))?;
        match ch.recv(MAX_FRAME_TO_HOST) {
            Ok(Some(ToHost::Reply {
                invocation_id: i,
                call_id: c,
                result,
            })) if i == invocation_id && c == call_id => Ok(result),
            Ok(Some(ToHost::Shutdown {})) => Err(ChannelError("shutdown during a call".into())),
            Ok(Some(_)) => Err(ChannelError("unexpected message during a call".into())),
            Ok(None) => Err(ChannelError("channel closed during a call".into())),
            Err(e) => Err(ChannelError(e.to_string())),
        }
    }
}

/// How a host run ended. [`Exit::code`] is the process exit code.
#[derive(Debug, PartialEq, Eq)]
pub enum Exit {
    /// `shutdown`, or the supervisor closed the channel between messages.
    Clean,
    Fatal(FatalReason, String),
    Io(String),
}

impl Exit {
    pub fn code(&self) -> i32 {
        match self {
            Self::Clean => exit_code::CLEAN,
            Self::Fatal(r, _) => r.exit_code(),
            Self::Io(_) => exit_code::IO,
        }
    }
}

impl fmt::Display for Exit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Clean => f.write_str("clean"),
            Self::Fatal(r, d) => write!(f, "{r:?}: {d}"),
            Self::Io(e) => write!(f, "i/o: {e}"),
        }
    }
}

/// Runs the protocol until shutdown or a fault. The sandbox must already be
/// applied; `report` says how. Sends `fatal` (best effort) before returning
/// a fatal exit.
pub fn run<R: Read + 'static, W: Write + 'static>(
    engine: &PluginEngine,
    report: SandboxReport,
    reader: R,
    writer: W,
) -> Exit {
    let channel = Rc::new(RefCell::new(Channel { reader, writer }));
    let exit = serve(engine, report, &channel);
    if let Exit::Fatal(reason, detail) = &exit {
        let _ = channel.borrow_mut().send(&ToSupervisor::Fatal {
            reason: *reason,
            detail: detail.chars().take(512).collect(),
        });
    }
    exit
}

fn violation(what: impl Into<String>) -> Exit {
    Exit::Fatal(FatalReason::ProtocolViolation, what.into())
}

fn frame_exit(e: FrameError) -> Exit {
    match e {
        FrameError::Io(e) => Exit::Io(e.to_string()),
        other => violation(other.to_string()),
    }
}

fn fault_exit(f: Fault) -> Exit {
    Exit::Fatal(f.reason(), f.to_string())
}

fn serve<R: Read + 'static, W: Write + 'static>(
    engine: &PluginEngine,
    report: SandboxReport,
    channel: &Rc<RefCell<Channel<R, W>>>,
) -> Exit {
    let send = |msg: &ToSupervisor| channel.borrow_mut().send(msg).map_err(frame_exit);
    if let Err(e) = send(&ToSupervisor::Ready {
        protocol: PROTOCOL_VERSION,
        host_version: env!("CARGO_PKG_VERSION").into(),
        engine: ENGINE_NAME.into(),
        sandbox: report,
    }) {
        return e;
    }

    // The only frame allowed to be large.
    let first = channel.borrow_mut().recv(MAX_LOAD_FRAME);
    let (module, limits) = match first {
        Ok(Some(ToHost::Load {
            abi_version,
            module,
            limits,
        })) => {
            if abi_version != ABI_VERSION {
                return Exit::Fatal(
                    FatalReason::LoadFailed,
                    format!("ABI {abi_version} not supported"),
                );
            }
            (module, limits)
        }
        Ok(Some(ToHost::Shutdown {})) | Ok(None) => return Exit::Clean,
        Ok(Some(_)) => return violation("expected load"),
        Err(e) => return frame_exit(e),
    };
    let handler = Handler {
        channel: channel.clone(),
    };
    let mut plugin = match engine.load(&module, &limits, Box::new(handler)) {
        Ok(p) => p,
        Err(f) => return fault_exit(f),
    };
    drop(module);
    if let Err(e) = send(&ToSupervisor::Loaded {
        abi_version: ABI_VERSION,
    }) {
        return e;
    }

    let mut last_invocation = 0u64;
    loop {
        let msg = channel.borrow_mut().recv(MAX_FRAME_TO_HOST);
        match msg {
            Ok(Some(ToHost::Invoke {
                invocation_id,
                action,
                params,
            })) => {
                if invocation_id <= last_invocation {
                    return violation("invocation ids must increase");
                }
                last_invocation = invocation_id;
                let outcome = match plugin.invoke(invocation_id, &action, &params) {
                    Ok(o) => o,
                    Err(f) => return fault_exit(f),
                };
                if let Err(e) = send(&ToSupervisor::Result {
                    invocation_id,
                    outcome,
                }) {
                    return e;
                }
            }
            Ok(Some(ToHost::Shutdown {})) | Ok(None) => return Exit::Clean,
            Ok(Some(_)) => return violation("unexpected message"),
            Err(e) => return frame_exit(e),
        }
    }
}
