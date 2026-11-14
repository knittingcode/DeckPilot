//! Injectable crash points.
//!
//! Every multi-step operation calls [`Failpoints::should_crash`] after each
//! durable step (a committed transaction or a completed file system
//! change). When it answers `true`, the operation returns
//! [`StoreError::Crashed`](crate::StoreError::Crashed) at once and cleans
//! nothing up, which is what a process that dies at that point leaves
//! behind. Tests drop the manager, open a new one and run
//! [`PluginManager::recover`](crate::PluginManager::recover).

use std::fmt;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// A lifecycle operation (also the `op` of a journal entry).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Install,
    Update,
    ApproveUpdate,
    DeclineUpdate,
    Rollback,
    Uninstall,
    Recover,
}

impl Operation {
    pub const ALL: [Self; 7] = [
        Self::Install,
        Self::Update,
        Self::ApproveUpdate,
        Self::DeclineUpdate,
        Self::Rollback,
        Self::Uninstall,
        Self::Recover,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Update => "update",
            Self::ApproveUpdate => "approve_update",
            Self::DeclineUpdate => "decline_update",
            Self::Rollback => "rollback",
            Self::Uninstall => "uninstall",
            Self::Recover => "recover",
        }
    }

    pub(crate) fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|op| op.as_str() == text)
    }

    /// The crash points this operation passes, in order.
    pub fn failpoints(self) -> &'static [Failpoint] {
        use Failpoint::{
            Committed, FilesTrashed, FilesUnpacked, JournalWritten, RecoveryFilesResolved,
            SecretsDeleted, StepRecorded,
        };
        match self {
            Self::Install | Self::Update => {
                &[JournalWritten, FilesUnpacked, StepRecorded, Committed]
            }
            Self::Uninstall => &[
                JournalWritten,
                FilesTrashed,
                StepRecorded,
                SecretsDeleted,
                Committed,
            ],
            Self::ApproveUpdate | Self::DeclineUpdate | Self::Rollback => &[Committed],
            Self::Recover => &[RecoveryFilesResolved],
        }
    }
}

impl fmt::Display for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A point right after a durable step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Failpoint {
    /// The journal entry (and the transient state) is committed; nothing
    /// else happened.
    JournalWritten,
    /// The package was unpacked into its version directory.
    FilesUnpacked,
    /// The journal entry records the completed file system step.
    StepRecorded,
    /// The plugin directory was moved to `.trash`.
    FilesTrashed,
    /// The secret clean-up hook ran.
    SecretsDeleted,
    /// The final transaction is committed; only clean-up remains.
    Committed,
    /// Recovery changed the file system for a journal entry but has not
    /// committed its database change.
    RecoveryFilesResolved,
}

/// Decides where an operation crashes. Production uses [`NoFailpoints`].
pub trait Failpoints: Send + Sync + fmt::Debug {
    fn should_crash(&self, op: Operation, point: Failpoint) -> bool;
}

/// Never crashes.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoFailpoints;

impl Failpoints for NoFailpoints {
    fn should_crash(&self, _op: Operation, _point: Failpoint) -> bool {
        false
    }
}

/// Crashes once, at the first time `op` reaches `point`, and remembers
/// whether that happened (so a test can prove the point is reachable).
#[derive(Debug)]
pub struct CrashOnce {
    op: Operation,
    point: Failpoint,
    fired: Mutex<bool>,
}

impl CrashOnce {
    pub fn new(op: Operation, point: Failpoint) -> Self {
        Self {
            op,
            point,
            fired: Mutex::new(false),
        }
    }

    pub fn fired(&self) -> bool {
        *self
            .fired
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Failpoints for CrashOnce {
    fn should_crash(&self, op: Operation, point: Failpoint) -> bool {
        let mut fired = self
            .fired
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !*fired && op == self.op && point == self.point {
            *fired = true;
            return true;
        }
        false
    }
}
