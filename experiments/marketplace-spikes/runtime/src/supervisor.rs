//! Supervisor side: spawns one plugin host per plugin, speaks framed JSON
//! over its stdin/stdout, routes `call` frames to the broker, and kills the
//! child on timeout. The supervisor stands in for `deckpilot-agent`.
//!
//! The spike uses stdin/stdout pipes; the design uses one inherited
//! socketpair / pipe handle. Neither is a sandbox by itself.

use std::io::{self, BufReader, BufWriter};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::broker::Broker;
use crate::protocol::{MAX_TO_SUPERVISOR, ToHost, ToSupervisor, read_frame, write_frame};
use crate::sandbox::SandboxProfile;

#[derive(Debug)]
pub enum SupError {
    Timeout,
    /// The channel closed (child exited or crashed).
    Closed(Option<ExitStatus>),
    Protocol(String),
    Io(io::Error),
}

impl std::fmt::Display for SupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for SupError {}

impl From<io::Error> for SupError {
    fn from(e: io::Error) -> Self {
        SupError::Io(e)
    }
}

pub struct SpawnConfig {
    pub host_bin: PathBuf,
    pub plugin_id: String,
    pub sandbox: Option<SandboxProfile>,
    pub ready_timeout: Duration,
}

impl SpawnConfig {
    pub fn new(host_bin: impl Into<PathBuf>, plugin_id: &str) -> Self {
        Self {
            host_bin: host_bin.into(),
            plugin_id: plugin_id.to_owned(),
            sandbox: None,
            ready_timeout: Duration::from_secs(10),
        }
    }
}

pub struct PluginProcess {
    pub plugin_id: String,
    child: Child,
    stdin: BufWriter<ChildStdin>,
    rx: Receiver<io::Result<ToSupervisor>>,
    pub ready: ToSupervisor,
    next_invocation: u64,
}

#[derive(Debug)]
pub struct InvokeOutcome {
    pub result: Result<Value, String>,
    pub calls: u64,
}

impl PluginProcess {
    pub fn spawn(cfg: SpawnConfig) -> Result<Self, SupError> {
        let mut cmd = match &cfg.sandbox {
            Some(profile) => profile.command(&cfg.host_bin),
            None => Command::new(&cfg.host_bin),
        };
        cmd.env_clear()
            .current_dir("/")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = cmd.spawn()?;
        let stdin = BufWriter::new(child.stdin.take().unwrap());
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            loop {
                match read_frame::<_, ToSupervisor>(&mut stdout, MAX_TO_SUPERVISOR) {
                    Ok(Some(msg)) => {
                        if tx.send(Ok(msg)).is_err() {
                            return;
                        }
                    }
                    Ok(None) => return,
                    Err(e) => {
                        let _ = tx.send(Err(e));
                        return;
                    }
                }
            }
        });
        let mut p = Self {
            plugin_id: cfg.plugin_id,
            child,
            stdin,
            rx,
            ready: ToSupervisor::Pong { seq: 0 },
            next_invocation: 1,
        };
        match p.recv(cfg.ready_timeout)? {
            r @ ToSupervisor::Ready { .. } => p.ready = r,
            other => return Err(SupError::Protocol(format!("expected ready, got {other:?}"))),
        }
        Ok(p)
    }

    pub fn send(&mut self, msg: &ToHost) -> Result<(), SupError> {
        write_frame(&mut self.stdin, msg).map_err(|e| match e.kind() {
            io::ErrorKind::BrokenPipe => SupError::Closed(self.child.try_wait().ok().flatten()),
            _ => SupError::Io(e),
        })
    }

    /// Waits for one frame; kills the child on timeout or malformed frame.
    pub fn recv(&mut self, timeout: Duration) -> Result<ToSupervisor, SupError> {
        match self.rx.recv_timeout(timeout) {
            Ok(Ok(msg)) => Ok(msg),
            Ok(Err(e)) => {
                self.kill();
                Err(SupError::Protocol(e.to_string()))
            }
            Err(RecvTimeoutError::Timeout) => {
                self.kill();
                Err(SupError::Timeout)
            }
            Err(RecvTimeoutError::Disconnected) => {
                let status = self.child.wait().ok();
                Err(SupError::Closed(status))
            }
        }
    }

    pub fn ping(&mut self, seq: u64, timeout: Duration) -> Result<(), SupError> {
        self.send(&ToHost::Ping { seq })?;
        match self.recv(timeout)? {
            ToSupervisor::Pong { seq: s } if s == seq => Ok(()),
            other => Err(SupError::Protocol(format!("unexpected {other:?}"))),
        }
    }

    /// Runs one guest entry point, serving its capability calls from
    /// `broker` under this process's plugin id. `timeout` is a wall-clock
    /// deadline for the whole invocation.
    pub fn invoke(
        &mut self,
        broker: &Broker,
        entry: &str,
        input: Option<&str>,
        timeout: Duration,
    ) -> Result<InvokeOutcome, SupError> {
        let id = self.next_invocation;
        self.next_invocation += 1;
        self.send(&ToHost::Invoke {
            invocation_id: id,
            entry: entry.to_owned(),
            input: input.map(str::to_owned),
        })?;
        let deadline = Instant::now() + timeout;
        let mut calls = 0;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.recv(left)? {
                ToSupervisor::Call {
                    invocation_id,
                    call_id,
                    op,
                    args,
                } if invocation_id == id => {
                    calls += 1;
                    // Identity comes from the channel, not the message.
                    let reply = match broker.handle(&self.plugin_id, &op, &args) {
                        Ok(v) => ToHost::Reply {
                            call_id,
                            ok: Some(v),
                            error: None,
                        },
                        Err(e) => ToHost::Reply {
                            call_id,
                            ok: None,
                            error: Some(e),
                        },
                    };
                    self.send(&reply)?;
                }
                ToSupervisor::Result {
                    invocation_id,
                    ok,
                    error,
                } if invocation_id == id => {
                    let result = match (ok, error) {
                        (_, Some(e)) => Err(e),
                        (Some(v), None) => Ok(v),
                        (None, None) => Ok(Value::Null),
                    };
                    return Ok(InvokeOutcome { result, calls });
                }
                other => {
                    self.kill();
                    return Err(SupError::Protocol(format!("unexpected {other:?}")));
                }
            }
        }
    }

    pub fn probe(&mut self, kind: &str, target: &str) -> Result<(bool, String), SupError> {
        self.send(&ToHost::Probe {
            kind: kind.into(),
            target: target.into(),
        })?;
        match self.recv(Duration::from_secs(10))? {
            ToSupervisor::ProbeResult { ok, detail } => Ok((ok, detail)),
            other => Err(SupError::Protocol(format!("unexpected {other:?}"))),
        }
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        self.child.wait()
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }
}

impl Drop for PluginProcess {
    fn drop(&mut self) {
        let _ = write_frame(&mut self.stdin, &ToHost::Shutdown);
        self.kill();
    }
}
