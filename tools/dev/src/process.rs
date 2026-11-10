//! Process ownership checks and termination without pattern matching.
//!
//! A process belongs to the session only if both its PID *and* its start
//! time match what was recorded when the session started it. This prevents
//! signalling an unrelated process that later reused the PID.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, Signal, System};

/// A process started by the development session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedProcess {
    pub role: String,
    pub pid: u32,
    /// Start time in seconds since the Unix epoch, as reported by the OS.
    pub start_time: u64,
}

pub fn system() -> System {
    System::new_with_specifics(RefreshKind::nothing().with_processes(ProcessRefreshKind::nothing()))
}

fn refreshed() -> System {
    let mut system = system();
    system.refresh_processes(ProcessesToUpdate::All, true);
    system
}

/// Records a freshly spawned child. Returns `None` if it already exited.
pub fn record(role: &str, pid: u32) -> Option<OwnedProcess> {
    let system = refreshed();
    let process = system.process(Pid::from_u32(pid))?;
    Some(OwnedProcess {
        role: role.to_owned(),
        pid,
        start_time: process.start_time(),
    })
}

/// Whether the recorded process is still the same running process.
pub fn is_alive(owned: &OwnedProcess) -> bool {
    alive_in(&refreshed(), owned)
}

fn alive_in(system: &System, owned: &OwnedProcess) -> bool {
    system
        .process(Pid::from_u32(owned.pid))
        .is_some_and(|p| p.start_time() == owned.start_time && is_running(p))
}

fn is_running(process: &sysinfo::Process) -> bool {
    !matches!(
        process.status(),
        sysinfo::ProcessStatus::Zombie | sysinfo::ProcessStatus::Dead
    )
}

/// The process and all its descendants, children before parents.
fn tree(system: &System, root: Pid) -> Vec<Pid> {
    let mut ordered = Vec::new();
    let mut stack = vec![root];
    while let Some(pid) = stack.pop() {
        ordered.push(pid);
        for (child, process) in system.processes() {
            if process.parent() == Some(pid) && !ordered.contains(child) {
                stack.push(*child);
            }
        }
    }
    ordered.reverse();
    ordered
}

/// Stops an owned process and its descendants: a polite termination
/// request first, a forced kill after `grace`. Does nothing if the PID now
/// belongs to a different process.
pub fn terminate_tree(owned: &OwnedProcess, grace: Duration) {
    let system = refreshed();
    if !alive_in(&system, owned) {
        return;
    }
    let pids = tree(&system, Pid::from_u32(owned.pid));
    for pid in &pids {
        if let Some(process) = system.process(*pid) {
            // `None`: the platform has no SIGTERM (Windows); fall back below.
            if process.kill_with(Signal::Term).is_none() {
                process.kill();
            }
        }
    }
    let deadline = Instant::now() + grace;
    while Instant::now() < deadline {
        let system = refreshed();
        if pids
            .iter()
            .all(|pid| system.process(*pid).is_none_or(|p| !is_running(p)))
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let system = refreshed();
    for pid in &pids {
        if let Some(process) = system.process(*pid) {
            process.kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ownership_requires_matching_start_time() {
        let me = record("test", std::process::id()).unwrap();
        assert!(is_alive(&me));
        let impostor = OwnedProcess {
            start_time: me.start_time + 1,
            ..me.clone()
        };
        assert!(!is_alive(&impostor), "a reused PID must not count as owned");
        // Terminating a non-matching record must not touch this process.
        terminate_tree(&impostor, Duration::from_millis(10));
        assert!(is_alive(&me));
    }

    #[test]
    fn terminates_an_owned_child_tree() {
        let child = if cfg!(windows) {
            std::process::Command::new("cmd")
                .args(["/C", "ping -n 30 127.0.0.1 > NUL"])
                .spawn()
        } else {
            std::process::Command::new("sleep").arg("30").spawn()
        }
        .unwrap();
        let mut child = child;
        let owned = record("child", child.id()).unwrap();
        terminate_tree(&owned, Duration::from_secs(5));
        let status = child.wait().unwrap();
        assert!(!status.success());
        assert!(!is_alive(&owned));
    }
}
