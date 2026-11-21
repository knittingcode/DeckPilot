//! Running external processes from an action, cancellably.
//!
//! [`run`] is what an action uses to run a program as part of a workflow:
//!
//! - The program runs in **its own process group** (Unix), so stopping it
//!   reaches everything it started and nothing else.
//! - When the action's token is cancelled — the run was cancelled or the
//!   attempt's timeout passed — the group gets `SIGTERM`, and after a grace
//!   period `SIGKILL`. On Windows the process tree is ended with
//!   `taskkill /T /F`.
//! - When the program exits on its own, processes it left behind in its
//!   group are ended too: an action never leaves orphans.
//! - Output is captured up to a limit; the rest is read and discarded so the
//!   program never blocks on a full pipe.
//!
//! Actions normally go through [`crate::ExecutionContext::run_process`],
//! which checks the module's capability and bounds the run's time.

use std::io::{self, Read};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use crate::{CancelReason, CancellationToken};

/// How often the process is checked for exit and cancellation.
const POLL: Duration = Duration::from_millis(10);

#[derive(Debug, Clone, Copy)]
pub struct ProcessLimits {
    /// Time between the polite and the forced stop.
    pub grace: Duration,
    /// Bytes of output kept per stream.
    pub max_output: usize,
}

impl Default for ProcessLimits {
    fn default() -> Self {
        Self {
            grace: Duration::from_secs(2),
            max_output: 64 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessOutput {
    /// Exit code; `None` when ended by a signal.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    /// More output was produced than kept.
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

impl ProcessOutput {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("the program could not be started: {0}")]
    Spawn(#[source] io::Error),
    #[error("cancelled; the program was stopped")]
    Cancelled,
    #[error("did not finish in time; the program was stopped")]
    TimedOut,
    #[error("waiting for the program failed: {0}")]
    Wait(#[source] io::Error),
    #[error(transparent)]
    NotPermitted(crate::CapabilityDenied),
}

impl ProcessError {
    /// Stable error code for [`crate::ActionResult::code`].
    pub fn code(&self) -> &'static str {
        match self {
            Self::Spawn(error) if error.kind() == io::ErrorKind::NotFound => "program-not-found",
            Self::Spawn(_) => "spawn-failed",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed-out",
            Self::Wait(_) => "wait-failed",
            Self::NotPermitted(_) => "not-permitted",
        }
    }
}

/// Variables a program started by an action keeps from the agent's
/// environment; everything else (tokens, keys, passwords that happen to be
/// set for the agent) is removed. Modules add what their tool needs, such
/// as `DOCKER_HOST` or `SSH_AUTH_SOCK`.
pub const BASE_ENVIRONMENT: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "LC_MESSAGES",
    "TMPDIR",
    "TEMP",
    "TMP",
    "TZ",
    "XDG_RUNTIME_DIR",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
    // Windows
    "SystemRoot",
    "SystemDrive",
    "windir",
    "ComSpec",
    "PATHEXT",
    "USERPROFILE",
    "USERNAME",
    "USERDOMAIN",
    "HOMEDRIVE",
    "HOMEPATH",
    "APPDATA",
    "LOCALAPPDATA",
    "ProgramData",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramW6432",
    "CommonProgramFiles",
    "NUMBER_OF_PROCESSORS",
    "PROCESSOR_ARCHITECTURE",
    "OS",
];

/// Clears `command`'s environment except [`BASE_ENVIRONMENT`] and `extra`.
pub fn minimal_environment(command: &mut Command, extra: &[&str]) {
    command.env_clear();
    for name in BASE_ENVIRONMENT.iter().chain(extra) {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
}

/// Runs `command` to completion, or stops it when `token` is cancelled.
/// Stdin is closed; stdout and stderr are captured.
pub fn run(
    mut command: Command,
    token: Option<&CancellationToken>,
    limits: ProcessLimits,
) -> Result<ProcessOutput, ProcessError> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = spawn(&mut command).map_err(ProcessError::Spawn)?;
    let stdout = child.stdout.take().map(|s| reader(s, limits.max_output));
    let stderr = child.stderr.take().map(|s| reader(s, limits.max_output));
    let outcome = supervise(&mut child, token, limits.grace);
    // The program is gone; end whatever it left in its group, so the pipes
    // close and nothing is orphaned.
    end_group(&child, true);
    // A process that left the group (`setsid`) may still hold the pipes
    // open: the output is waited for at most `grace` more, then taken as it
    // is and marked truncated. The readers end when the pipes close.
    let until = Instant::now() + limits.grace;
    let collect = |stream: Option<Captured>| stream.map(|s| s.take(until)).unwrap_or_default();
    let ((stdout, stdout_truncated), (stderr, stderr_truncated)) =
        (collect(stdout), collect(stderr));
    let status = outcome?;
    Ok(ProcessOutput {
        code: status.code(),
        stdout,
        stderr,
        stdout_truncated,
        stderr_truncated,
    })
}

/// Waits for exit, stopping the program when the token is cancelled.
/// Starts `command`, retrying briefly while its program is still open for
/// writing elsewhere. On Linux a program that was just written (an
/// installer's or a test's script) cannot start while another thread's
/// child still holds the file open between fork and exec.
pub fn spawn(command: &mut Command) -> io::Result<Child> {
    let mut attempts = 0;
    loop {
        match command.spawn() {
            Err(error) if error.kind() == io::ErrorKind::ExecutableFileBusy && attempts < 20 => {
                attempts += 1;
                std::thread::sleep(Duration::from_millis(25));
            }
            result => return result,
        }
    }
}

fn supervise(
    child: &mut Child,
    token: Option<&CancellationToken>,
    grace: Duration,
) -> Result<ExitStatus, ProcessError> {
    loop {
        if let Some(status) = child.try_wait().map_err(ProcessError::Wait)? {
            return Ok(status);
        }
        if let Some(reason) = token.and_then(CancellationToken::reason) {
            stop(child, grace);
            return Err(match reason {
                CancelReason::Cancelled => ProcessError::Cancelled,
                CancelReason::DeadlineExceeded => ProcessError::TimedOut,
            });
        }
        std::thread::sleep(POLL);
    }
}

/// Asks the program to stop, then forces it after `grace`.
fn stop(child: &mut Child, grace: Duration) {
    end_group(child, false);
    let until = Instant::now() + grace;
    while Instant::now() < until {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        std::thread::sleep(POLL);
    }
    end_group(child, true);
    let _ = child.kill();
    let _ = child.wait();
}

/// Signals the program's process group: `SIGTERM`, or `SIGKILL` when
/// `force`. Only the group the program was started in is signalled.
#[cfg(unix)]
fn end_group(child: &Child, force: bool) {
    use rustix::process::{Pid, Signal, kill_process_group};
    let group = Pid::from_child(child);
    let signal = if force { Signal::KILL } else { Signal::TERM };
    // Fails harmlessly when the group no longer exists.
    let _ = kill_process_group(group, signal);
}

/// Ends the program's process tree. Windows has no polite stop for console
/// programs; the tree is ended at once.
#[cfg(windows)]
fn end_group(child: &Child, _force: bool) {
    let _ = Command::new("taskkill")
        .args(["/T", "/F", "/PID", &child.id().to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// What a reader kept so far: the bytes, whether more was discarded, and
/// whether the stream ended.
#[derive(Default)]
struct Kept {
    bytes: Vec<u8>,
    truncated: bool,
    ended: bool,
}

/// The output of one stream, read on its own thread.
struct Captured(std::sync::Arc<(std::sync::Mutex<Kept>, std::sync::Condvar)>);

impl Captured {
    /// The output once the stream ended, or what there is at `until`
    /// (then marked truncated, since more may follow).
    fn take(&self, until: Instant) -> (String, bool) {
        let (lock, ended) = &*self.0;
        let mut kept = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while !kept.ended {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                kept.truncated = true;
                break;
            }
            kept = ended
                .wait_timeout(kept, left)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        (
            String::from_utf8_lossy(&kept.bytes).into_owned(),
            kept.truncated,
        )
    }
}

/// Reads a stream to its end, keeping at most `limit` bytes; truncated
/// when more was discarded.
fn reader<R: Read + Send + 'static>(mut stream: R, limit: usize) -> Captured {
    let shared = std::sync::Arc::new((
        std::sync::Mutex::new(Kept::default()),
        std::sync::Condvar::new(),
    ));
    let writer = shared.clone();
    std::thread::spawn(move || {
        let (lock, ended) = &*writer;
        let mut buffer = [0u8; 8192];
        loop {
            let read = stream.read(&mut buffer);
            let mut kept = lock
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match read {
                Ok(0) | Err(_) => {
                    kept.ended = true;
                    ended.notify_all();
                    break;
                }
                Ok(n) => {
                    let room = limit.saturating_sub(kept.bytes.len());
                    kept.truncated |= n > room;
                    kept.bytes.extend_from_slice(&buffer[..n.min(room)]);
                }
            }
        }
    });
    Captured(shared)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn sh(script: &str) -> Command {
        let mut command = Command::new("sh");
        command.args(["-c", script]);
        command
    }

    fn alive(pid: i32) -> bool {
        rustix::process::Pid::from_raw(pid)
            .is_some_and(|pid| rustix::process::test_kill_process(pid).is_ok())
    }

    #[test]
    fn programs_run_to_completion_with_their_output() {
        let output = run(
            sh("echo out; echo err >&2; exit 3"),
            None,
            ProcessLimits::default(),
        )
        .unwrap();
        assert_eq!(output.code, Some(3));
        assert_eq!(output.stdout, "out\n");
        assert_eq!(output.stderr, "err\n");
        assert!(!output.success());
    }

    #[test]
    fn output_is_limited() {
        let limits = ProcessLimits {
            max_output: 10,
            ..ProcessLimits::default()
        };
        let output = run(sh("yes | head -c 100000"), None, limits).unwrap();
        assert_eq!(output.stdout.len(), 10);
        assert!(output.stdout_truncated && !output.stderr_truncated);
        assert!(output.success());
    }

    #[test]
    fn cancelling_stops_the_program_and_everything_it_started() {
        let dir = std::env::temp_dir().join(format!("deckpilot-process-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pid_file = dir.join("background.pid");
        let script = format!("sleep 30 & echo $! > '{}'; wait", pid_file.display());
        let token = CancellationToken::new();
        let canceller = {
            let token = token.clone();
            let pid_file = pid_file.clone();
            std::thread::spawn(move || {
                while !pid_file.exists() {
                    std::thread::sleep(Duration::from_millis(10));
                }
                std::thread::sleep(Duration::from_millis(50));
                token.cancel();
            })
        };
        let begun = Instant::now();
        let result = run(sh(&script), Some(&token), ProcessLimits::default());
        canceller.join().unwrap();
        assert!(matches!(result, Err(ProcessError::Cancelled)), "{result:?}");
        assert!(begun.elapsed() < Duration::from_secs(5));
        let background: i32 = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        // The program's own child is gone too.
        let deadline = Instant::now() + Duration::from_secs(2);
        while alive(background) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!alive(background), "process {background} was orphaned");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn deadlines_stop_programs_as_timeouts_even_when_they_ignore_sigterm() {
        let token = CancellationToken::new()
            .child_with_deadline(Instant::now() + Duration::from_millis(100));
        let limits = ProcessLimits {
            grace: Duration::from_millis(200),
            ..ProcessLimits::default()
        };
        let begun = Instant::now();
        let result = run(sh("trap '' TERM; sleep 30"), Some(&token), limits);
        assert!(matches!(result, Err(ProcessError::TimedOut)), "{result:?}");
        assert!(begun.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn programs_get_a_minimal_environment() {
        // SAFETY of the test: the variable is unique to this test.
        let name = "DECKPILOT_TEST_SECRET_FOR_ENV";
        assert!(std::env::var_os(name).is_none());
        let mut command = sh("echo \"[${DECKPILOT_TEST_SECRET_FOR_ENV:-}]$PATH\"");
        command.env(name, "s3cret");
        minimal_environment(&mut command, &[]);
        let output = run(command, None, ProcessLimits::default()).unwrap();
        assert!(output.stdout.starts_with("[]"), "{}", output.stdout);
        assert!(output.stdout.len() > 3, "PATH is kept");
    }

    /// A process that left the program's group (`setsid`) and still holds
    /// its output open does not keep the run waiting past the grace period.
    #[test]
    fn a_detached_grandchild_holding_the_pipes_does_not_outlast_the_run() {
        let dir = std::env::temp_dir().join(format!("deckpilot-setsid-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pid_file = dir.join("detached.pid");
        // The detached process writes its pid once it left the group; the
        // program exits only then, so the group's end cannot reach it.
        let script = format!(
            "echo started; perl -MPOSIX -e 'POSIX::setsid(); open(F, \">\", $ARGV[0]); \
             print F $$; close F; sleep 30' '{pid}' & \
             while [ ! -s '{pid}' ]; do sleep 0.02; done; exit 0",
            pid = pid_file.display()
        );
        let limits = ProcessLimits {
            grace: Duration::from_millis(300),
            ..ProcessLimits::default()
        };
        let begun = Instant::now();
        let output = run(sh(&script), None, limits).unwrap();
        assert!(
            begun.elapsed() < Duration::from_secs(5),
            "{:?}",
            begun.elapsed()
        );
        assert!(output.success());
        assert!(output.stdout.starts_with("started"), "{output:?}");
        assert!(output.stdout_truncated);
        // The detached process is the test's to end.
        let deadline = Instant::now() + Duration::from_secs(2);
        while !pid_file.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        if let Some(pid) = std::fs::read_to_string(&pid_file)
            .ok()
            .and_then(|p| p.trim().parse::<i32>().ok())
            .and_then(rustix::process::Pid::from_raw)
        {
            let _ = rustix::process::kill_process(pid, rustix::process::Signal::KILL);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_programs_fail_to_start() {
        let result = run(
            Command::new("/nonexistent/deckpilot-test"),
            None,
            ProcessLimits::default(),
        );
        assert!(matches!(result, Err(ProcessError::Spawn(_))));
    }
}
