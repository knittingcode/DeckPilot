//! Single-instance guarantee per user and data directory.

use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::Path;

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
        assert!(InstanceLock::acquire(&path).is_ok());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
