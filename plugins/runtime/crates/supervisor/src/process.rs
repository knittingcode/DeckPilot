//! The host process as the supervisor sees it: two pipes, an id, kill and
//! wait. On Windows it is launched by [`crate::windows`] (AppContainer and
//! a supervisor-held Job Object); elsewhere by `std::process::Command`.

use std::io;
use std::process::ExitStatus;

/// The write end of the host's standard input.
#[cfg(not(windows))]
pub type HostStdin = std::process::ChildStdin;
/// The read end of the host's standard output.
#[cfg(not(windows))]
pub type HostStdout = std::process::ChildStdout;
/// The write end of the host's standard input.
#[cfg(windows)]
pub type HostStdin = std::io::PipeWriter;
/// The read end of the host's standard output.
#[cfg(windows)]
pub type HostStdout = std::io::PipeReader;

/// A spawned host (see [`crate::HostCommand::spawn`]). Like
/// `std::process::Child`, dropping it does not wait; on Windows it kills
/// the process (Job Object) and deletes its AppContainer profile.
#[derive(Debug)]
pub struct HostProcess {
    pub stdin: Option<HostStdin>,
    pub stdout: Option<HostStdout>,
    #[cfg(not(windows))]
    child: std::process::Child,
    #[cfg(windows)]
    child: crate::windows::Process,
}

impl HostProcess {
    #[cfg(not(windows))]
    pub(crate) fn from_child(mut child: std::process::Child) -> Self {
        Self {
            stdin: child.stdin.take(),
            stdout: child.stdout.take(),
            child,
        }
    }

    #[cfg(windows)]
    pub(crate) fn from_spawned(s: crate::windows::Spawned) -> Self {
        Self {
            stdin: Some(s.stdin),
            stdout: Some(s.stdout),
            child: s.process,
        }
    }

    pub fn id(&self) -> u32 {
        self.child.id()
    }

    /// The Job Object the supervisor created for this host and holds (kill
    /// on close). Borrowed for inspection (`IsProcessInJob`,
    /// `QueryInformationJobObject`); closing it is the supervisor's job.
    #[cfg(windows)]
    pub fn job(&self) -> std::os::windows::io::BorrowedHandle<'_> {
        self.child.job()
    }

    pub fn kill(&mut self) -> io::Result<()> {
        self.child.kill()
    }

    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        self.stdin = None;
        self.child.wait()
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }
}
