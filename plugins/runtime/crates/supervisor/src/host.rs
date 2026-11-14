//! One running plugin host process.

use std::ffi::OsString;
use std::fmt;
use std::io::{self, BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use deckpilot_plugin_abi::frame::{FrameError, read_frame, write_frame};
use deckpilot_plugin_abi::limits::{
    MAX_FRAME_TO_SUPERVISOR, MAX_HOST_CALL_REPLY, MAX_HOST_CALLS_PER_INVOCATION, MAX_LOAD_FRAME,
    MAX_MODULE_BYTES,
};
use deckpilot_plugin_abi::protocol::{
    Enforcement, FatalReason, GuestLimits, PROTOCOL_VERSION, SandboxReport, ToHost, ToSupervisor,
};
use deckpilot_plugin_abi::value::{validate_name, validate_params};
use deckpilot_plugin_abi::{ABI_VERSION, ActionOutcome, CallErrorCode, CallResult, Params};
use deckpilot_plugin_broker::{Allowed, Decision, PluginId, Session};

/// Carries out what the broker allowed (the agent's HTTP client, storage,
/// notifications, log). Called on the supervisor's thread.
pub trait CapabilityExecutor {
    fn execute(&mut self, plugin: &PluginId, allowed: &Allowed) -> CallResult;
}

/// The program to run as the host and its arguments.
#[derive(Debug, Clone)]
pub struct HostCommand {
    pub program: PathBuf,
    /// Production: empty. Tests may pass `--no-os-sandbox`.
    pub args: Vec<OsString>,
}

/// The working directory every host gets: the file system root on Unix,
/// the host binary's directory on Windows.
pub fn fixed_working_dir(program: &Path) -> PathBuf {
    if cfg!(windows) {
        program
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("\\"))
    } else {
        PathBuf::from("/")
    }
}

impl HostCommand {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
        }
    }

    pub fn arg(mut self, a: impl Into<OsString>) -> Self {
        self.args.push(a.into());
        self
    }

    /// The command as the supervisor spawns it: empty environment, fixed
    /// working directory, piped stdin/stdout, no stderr.
    pub fn command(&self) -> Command {
        let mut c = Command::new(&self.program);
        c.args(&self.args)
            .env_clear()
            .current_dir(fixed_working_dir(&self.program))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        crate::fds::inherit_only_stdio(&mut c);
        c
    }
}

#[derive(Debug, Clone)]
pub struct HostConfig {
    pub command: HostCommand,
    /// The verified wasm module, sent over the channel.
    pub module: Arc<[u8]>,
    pub limits: GuestLimits,
    /// Refuse a host whose `ready` reports no OS sandbox.
    pub require_sandbox: bool,
    /// Spawn until `ready`.
    pub handshake_timeout: Duration,
    /// `load` until `loaded` (compilation included).
    pub load_timeout: Duration,
    /// After `shutdown`, how long to wait before killing.
    pub shutdown_grace: Duration,
    /// How long to wait for the exit status after the channel closed.
    pub reap_timeout: Duration,
}

impl HostConfig {
    pub fn new(command: HostCommand, module: Arc<[u8]>) -> Self {
        Self {
            command,
            module,
            limits: GuestLimits::default(),
            require_sandbox: true,
            handshake_timeout: Duration::from_secs(10),
            load_timeout: Duration::from_secs(10),
            shutdown_grace: Duration::from_secs(5),
            reap_timeout: Duration::from_secs(2),
        }
    }
}

#[derive(Debug)]
pub enum HostError {
    Spawn(io::Error),
    /// The module is too large to send.
    ModuleTooLarge,
    /// `ready` was missing, late, of another protocol version or malformed.
    Handshake(String),
    /// The host reports no OS sandbox and the config requires one.
    SandboxMissing(SandboxReport),
    /// The host rejected the module (or died while loading it).
    LoadFailed {
        detail: String,
        status: Option<ExitStatus>,
    },
    /// The deadline passed; the host was killed.
    Timeout {
        status: Option<ExitStatus>,
    },
    /// The host exited (trap, deadline, fuel, crash); `fatal` if it said why.
    Crashed {
        fatal: Option<(FatalReason, String)>,
        status: Option<ExitStatus>,
    },
    /// The host broke the protocol; it was killed.
    Protocol {
        detail: String,
        status: Option<ExitStatus>,
    },
    /// The action or parameters break ABI 1 bounds (nothing was sent).
    InvalidParams,
    /// The host is not running any more.
    NotRunning,
}

impl HostError {
    /// Whether the host process is gone because of this error (counts as a
    /// crash for the restart policy).
    pub fn host_died(&self) -> bool {
        !matches!(self, Self::InvalidParams)
    }

    /// Exit status, when one was collected.
    pub fn status(&self) -> Option<ExitStatus> {
        match self {
            Self::LoadFailed { status, .. }
            | Self::Timeout { status }
            | Self::Crashed { status, .. }
            | Self::Protocol { status, .. } => *status,
            _ => None,
        }
    }
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for HostError {}

enum Recv {
    Msg(ToSupervisor),
    Closed,
    Malformed(String),
    Timeout,
}

/// A running host. Dropping it kills the process.
pub struct PluginHost {
    child: Child,
    stdin: Option<BufWriter<ChildStdin>>,
    rx: Receiver<Result<ToSupervisor, String>>,
    session: Session,
    sandbox: SandboxReport,
    next_invocation: u64,
    reap_timeout: Duration,
    dead: bool,
}

impl fmt::Debug for PluginHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PluginHost")
            .field("pid", &self.child.id())
            .field("plugin", self.session.plugin())
            .field("sandbox", &self.sandbox)
            .finish_non_exhaustive()
    }
}

impl PluginHost {
    /// Spawns the host, checks `ready`, sends the module and waits for
    /// `loaded`. `session` is the plugin's identity for every capability
    /// request that arrives on this process's channel.
    pub fn spawn(config: &HostConfig, session: Session) -> Result<Self, HostError> {
        if config.module.len() > MAX_MODULE_BYTES {
            return Err(HostError::ModuleTooLarge);
        }
        let mut child = config.command.command().spawn().map_err(HostError::Spawn)?;
        let stdin = child.stdin.take().map(BufWriter::new);
        let stdout = child.stdout.take();
        let (tx, rx) = mpsc::channel();
        if let Some(stdout) = stdout {
            std::thread::Builder::new()
                .name("plugin-host-reader".into())
                .spawn(move || {
                    let mut r = BufReader::new(stdout);
                    loop {
                        match read_frame::<_, ToSupervisor>(&mut r, MAX_FRAME_TO_SUPERVISOR) {
                            Ok(Some(m)) => {
                                if tx.send(Ok(m)).is_err() {
                                    return;
                                }
                            }
                            Ok(None) | Err(FrameError::Io(_)) => return,
                            Err(e) => {
                                let _ = tx.send(Err(e.to_string()));
                                return;
                            }
                        }
                    }
                })
                .map_err(HostError::Spawn)?;
        }
        let mut host = Self {
            child,
            stdin,
            rx,
            session,
            sandbox: SandboxReport {
                mechanism: String::new(),
                enforcement: Enforcement::None,
                notes: vec![],
            },
            next_invocation: 1,
            reap_timeout: config.reap_timeout,
            dead: false,
        };
        host.handshake(config)?;
        Ok(host)
    }

    fn handshake(&mut self, config: &HostConfig) -> Result<(), HostError> {
        let deadline = Instant::now() + config.handshake_timeout;
        match self.recv(deadline) {
            Recv::Msg(ToSupervisor::Ready {
                protocol, sandbox, ..
            }) => {
                if protocol != PROTOCOL_VERSION {
                    return Err(self.fail_handshake(format!("protocol {protocol}")));
                }
                if config.require_sandbox && sandbox.enforcement == Enforcement::None {
                    self.kill();
                    return Err(HostError::SandboxMissing(sandbox));
                }
                self.sandbox = sandbox;
            }
            Recv::Msg(other) => {
                return Err(self.fail_handshake(format!("expected ready, got {}", kind(&other))));
            }
            Recv::Timeout => return Err(self.fail_handshake("no ready before the deadline".into())),
            Recv::Closed => {
                let status = self.reap();
                return Err(HostError::Handshake(format!(
                    "host exited before ready ({status:?})"
                )));
            }
            Recv::Malformed(e) => return Err(self.fail_handshake(e)),
        }

        let load = ToHost::Load {
            abi_version: ABI_VERSION,
            module: config.module.to_vec(),
            limits: config.limits.clone(),
        };
        if let Err(e) = self.send(&load, MAX_LOAD_FRAME) {
            let status = self.reap();
            return Err(HostError::LoadFailed { detail: e, status });
        }
        let deadline = Instant::now() + config.load_timeout;
        match self.recv(deadline) {
            Recv::Msg(ToSupervisor::Loaded { abi_version }) if abi_version == ABI_VERSION => Ok(()),
            Recv::Msg(ToSupervisor::Fatal { reason, detail }) => {
                let status = self.reap();
                Err(HostError::LoadFailed {
                    detail: format!("{reason:?}: {detail}"),
                    status,
                })
            }
            Recv::Closed => {
                let status = self.reap();
                Err(HostError::LoadFailed {
                    detail: "host exited while loading".into(),
                    status,
                })
            }
            Recv::Timeout => {
                self.kill();
                Err(HostError::Timeout {
                    status: self.child.try_wait().ok().flatten(),
                })
            }
            Recv::Msg(other) => {
                Err(self.protocol(format!("expected loaded, got {}", kind(&other))))
            }
            Recv::Malformed(e) => Err(self.protocol(e)),
        }
    }

    fn fail_handshake(&mut self, detail: String) -> HostError {
        self.kill();
        HostError::Handshake(detail)
    }

    pub fn sandbox(&self) -> &SandboxReport {
        &self.sandbox
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn plugin(&self) -> &PluginId {
        self.session.plugin()
    }

    pub fn is_running(&self) -> bool {
        !self.dead
    }

    /// Runs one action. `timeout` is wall clock for the whole invocation,
    /// capability calls included; on expiry the host is killed.
    pub fn invoke(
        &mut self,
        action: &str,
        params: &Params,
        timeout: Duration,
        executor: &mut dyn CapabilityExecutor,
    ) -> Result<ActionOutcome, HostError> {
        if self.dead {
            return Err(HostError::NotRunning);
        }
        if validate_name(action).is_err() || validate_params(params).is_err() {
            return Err(HostError::InvalidParams);
        }
        let deadline = Instant::now() + timeout;
        let id = self.next_invocation;
        self.next_invocation += 1;
        let msg = ToHost::Invoke {
            invocation_id: id,
            action: action.to_owned(),
            params: params.clone(),
        };
        if let Err(e) = self.send(&msg, deckpilot_plugin_abi::limits::MAX_FRAME_TO_HOST) {
            return Err(self.after_send_failure(e));
        }
        let mut last_call = 0u64;
        loop {
            match self.recv(deadline) {
                Recv::Msg(ToSupervisor::Call {
                    invocation_id,
                    call_id,
                    request,
                }) => {
                    if invocation_id != id || call_id != last_call + 1 {
                        return Err(
                            self.protocol("call for another invocation or out of order".into())
                        );
                    }
                    last_call = call_id;
                    if call_id > u64::from(MAX_HOST_CALLS_PER_INVOCATION) {
                        return Err(self.protocol("too many capability calls".into()));
                    }
                    // Identity is the session bound to this process.
                    let result = match self.session.authorize(&request) {
                        Decision::Allow(allowed) => {
                            let r = executor.execute(self.session.plugin(), &allowed);
                            bound_reply(r)
                        }
                        Decision::Deny(reason) => {
                            CallResult::error(reason.call_error_code(), reason.to_string())
                        }
                    };
                    let reply = ToHost::Reply {
                        invocation_id: id,
                        call_id,
                        result,
                    };
                    if let Err(e) =
                        self.send(&reply, deckpilot_plugin_abi::limits::MAX_FRAME_TO_HOST)
                    {
                        return Err(self.after_send_failure(e));
                    }
                }
                Recv::Msg(ToSupervisor::Result {
                    invocation_id,
                    outcome,
                }) if invocation_id == id => {
                    // The host is assumed compromised: re-check the bounds.
                    return Ok(match outcome.validate() {
                        Ok(()) => outcome,
                        Err(_) => ActionOutcome::failed("invalid-result", "result breaks a bound"),
                    });
                }
                Recv::Msg(ToSupervisor::Fatal { reason, detail }) => {
                    let status = self.reap();
                    return Err(HostError::Crashed {
                        fatal: Some((reason, detail)),
                        status,
                    });
                }
                Recv::Msg(other) => {
                    return Err(self.protocol(format!("unexpected {}", kind(&other))));
                }
                Recv::Closed => {
                    let status = self.reap();
                    return Err(HostError::Crashed {
                        fatal: None,
                        status,
                    });
                }
                Recv::Malformed(e) => return Err(self.protocol(e)),
                Recv::Timeout => {
                    self.kill();
                    return Err(HostError::Timeout {
                        status: self.child.try_wait().ok().flatten(),
                    });
                }
            }
        }
    }

    /// Asks the host to exit; kills it after `grace`. Returns its status.
    pub fn shutdown(mut self, grace: Duration) -> Option<ExitStatus> {
        if !self.dead {
            let _ = self.send(&ToHost::Shutdown {}, 1024);
        }
        self.stdin = None; // EOF also ends the host
        let status = wait_for(&mut self.child, grace);
        if status.is_none() {
            self.kill();
        }
        self.dead = true;
        status.or_else(|| self.child.try_wait().ok().flatten())
    }

    /// Kills the host now.
    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.stdin = None;
        self.dead = true;
    }

    fn send(&mut self, msg: &ToHost, max: usize) -> Result<(), String> {
        let Some(stdin) = self.stdin.as_mut() else {
            return Err("channel closed".into());
        };
        write_frame(stdin, msg, max).map_err(|e| e.to_string())
    }

    /// A write to the host failed: it has (almost certainly) exited.
    fn after_send_failure(&mut self, _detail: String) -> HostError {
        let status = self.reap();
        HostError::Crashed {
            fatal: None,
            status,
        }
    }

    fn recv(&mut self, deadline: Instant) -> Recv {
        let left = deadline.saturating_duration_since(Instant::now());
        match self.rx.recv_timeout(left) {
            Ok(Ok(m)) => Recv::Msg(m),
            Ok(Err(e)) => Recv::Malformed(e),
            Err(RecvTimeoutError::Timeout) => Recv::Timeout,
            Err(RecvTimeoutError::Disconnected) => Recv::Closed,
        }
    }

    fn protocol(&mut self, detail: String) -> HostError {
        self.kill();
        HostError::Protocol {
            detail,
            status: self.child.try_wait().ok().flatten(),
        }
    }

    /// The channel closed: collect the exit status, killing the process if
    /// it does not exit on its own.
    fn reap(&mut self) -> Option<ExitStatus> {
        self.stdin = None;
        self.dead = true;
        let status = wait_for(&mut self.child, self.reap_timeout);
        if status.is_some() {
            return status;
        }
        let _ = self.child.kill();
        self.child.wait().ok()
    }
}

impl Drop for PluginHost {
    fn drop(&mut self) {
        if !self.dead {
            self.kill();
        }
    }
}

fn wait_for(child: &mut Child, timeout: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(s)) => return Some(s),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            _ => return None,
        }
    }
}

fn bound_reply(r: CallResult) -> CallResult {
    let len = serde_json::to_vec(&r)
        .map(|v| v.len())
        .unwrap_or(usize::MAX);
    if len > MAX_HOST_CALL_REPLY {
        CallResult::error(CallErrorCode::Failed, "reply too large")
    } else {
        r
    }
}

fn kind(m: &ToSupervisor) -> &'static str {
    match m {
        ToSupervisor::Ready { .. } => "ready",
        ToSupervisor::Loaded { .. } => "loaded",
        ToSupervisor::Call { .. } => "call",
        ToSupervisor::Result { .. } => "result",
        ToSupervisor::Fatal { .. } => "fatal",
    }
}
