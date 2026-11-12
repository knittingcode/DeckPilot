//! Single-instance guarantee per user and data directory.

use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

/// How long a starting agent waits for the lock of an agent that is still
/// stopping. A stopping agent closes its endpoint before it has finished
/// (and released the lock), so a client that restarts it (an update, the
/// app's "restart agent") can start the next one a moment too early.
pub const STARTUP_LOCK_WAIT: Duration = Duration::from_secs(5);
const RETRY_DELAY: Duration = Duration::from_millis(50);

/// An exclusive OS file lock held for the lifetime of the agent. The OS
/// releases it automatically if the process dies, so stale locks cannot
/// block a restart.
#[derive(Debug)]
pub struct InstanceLock {
    _file: File,
}

#[derive(Debug, thiserror::Error)]
pub enum LockError {
    #[error("another DeckPilot agent is already running for this data directory")]
    AlreadyRunning,
    #[error("cannot create the instance lock: {0}")]
    Io(#[from] io::Error),
}

impl InstanceLock {
    pub fn acquire(path: &Path) -> Result<Self, LockError> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)?;
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(TryLockError::WouldBlock) => Err(LockError::AlreadyRunning),
            Err(TryLockError::Error(error)) => Err(LockError::Io(error)),
        }
    }

    /// Like [`acquire`](Self::acquire), but retries for up to `wait` while
    /// another instance holds the lock, so that a previous agent that is
    /// still shutting down does not make this one give up. Returns
    /// [`LockError::AlreadyRunning`] if the lock stays taken.
    pub fn acquire_waiting(path: &Path, wait: Duration) -> Result<Self, LockError> {
        let deadline = Instant::now() + wait;
        loop {
            match Self::acquire(path) {
                Err(LockError::AlreadyRunning) if Instant::now() < deadline => {
                    std::thread::sleep(
                        RETRY_DELAY.min(deadline.saturating_duration_since(Instant::now())),
                    );
                }
                other => return other,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_lock_fails_until_first_is_released() {
        let dir = std::env::temp_dir().join(format!("deckpilot-lock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("agent.lock");
        let first = InstanceLock::acquire(&path).unwrap();
        assert!(matches!(
            InstanceLock::acquire(&path),
            Err(LockError::AlreadyRunning)
        ));
        drop(first);
        // Released, but not necessarily at once: a program that another
        // test of this process is starting right now holds a copy of every
        // descriptor until it executes (close-on-exec), the lock's
        // included. That is why a starting agent waits briefly for the lock
        // (`acquire_waiting`), and so does this test.
        let again = InstanceLock::acquire_waiting(&path, Duration::from_secs(2));
        assert!(again.is_ok(), "{again:?}");
        drop(again);
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn lock_path(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("deckpilot-lock-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("agent.lock")
    }

    #[test]
    fn waiting_acquire_succeeds_once_a_stopping_instance_releases_the_lock() {
        let path = lock_path("released");
        let first = InstanceLock::acquire(&path).unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            drop(first);
        });
        let started = Instant::now();
        let second = InstanceLock::acquire_waiting(&path, Duration::from_secs(10));
        assert!(second.is_ok(), "{second:?}");
        assert!(started.elapsed() >= Duration::from_millis(250));
        release.join().unwrap();
        drop(second);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn waiting_acquire_gives_up_while_an_instance_keeps_running() {
        let path = lock_path("held");
        let _first = InstanceLock::acquire(&path).unwrap();
        let started = Instant::now();
        assert!(matches!(
            InstanceLock::acquire_waiting(&path, Duration::from_millis(300)),
            Err(LockError::AlreadyRunning)
        ));
        let waited = started.elapsed();
        assert!(waited >= Duration::from_millis(300), "{waited:?}");
        assert!(waited < Duration::from_secs(5), "{waited:?}");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_lock_file_left_by_a_crashed_instance_does_not_block() {
        // The file stays after a crash; only the OS lock matters, and the
        // OS releases it with the process.
        let path = lock_path("leftover");
        std::fs::write(&path, b"").unwrap();
        assert!(InstanceLock::acquire_waiting(&path, Duration::ZERO).is_ok());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
