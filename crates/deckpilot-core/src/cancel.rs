//! Cooperative cancellation.
//!
//! A [`CancellationToken`] is shared by everything that belongs to one
//! workflow run. Every attempt of an action gets a child token that is also
//! cancelled when the attempt's deadline (its timeout) passes, so a handler
//! that checks its token stops both when the run is cancelled and when it
//! takes too long. [`CancellationToken::reason`] tells the two apart.
//!
//! Cancellation is cooperative: nothing is interrupted forcibly. A handler
//! that does not check its token finishes, and its node spec declares that
//! it cannot be interrupted (`NodeSpec::cancellable`).

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// How often a wait re-checks tokens it is not woken by directly (parents
/// and deadlines), at most.
const POLL: Duration = Duration::from_millis(10);

/// Why a token is cancelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelReason {
    /// Someone asked the run to stop.
    Cancelled,
    /// The token's deadline passed (a timeout).
    DeadlineExceeded,
}

#[derive(Default)]
struct Inner {
    cancelled: AtomicBool,
    parent: Option<CancellationToken>,
    deadline: Option<Instant>,
    lock: Mutex<()>,
    wake: Condvar,
}

/// Stops work cooperatively. Cheap to clone; clones share the state.
#[derive(Clone, Default)]
pub struct CancellationToken(Arc<Inner>);

impl fmt::Debug for CancellationToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CancellationToken")
            .field("reason", &self.reason())
            .finish_non_exhaustive()
    }
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    /// A token that is cancelled with this one, and can be cancelled on its
    /// own without affecting this one.
    #[must_use]
    pub fn child(&self) -> Self {
        Self(Arc::new(Inner {
            parent: Some(self.clone()),
            ..Inner::default()
        }))
    }

    /// A child that is also cancelled once `deadline` passes.
    #[must_use]
    pub fn child_with_deadline(&self, deadline: Instant) -> Self {
        Self(Arc::new(Inner {
            parent: Some(self.clone()),
            deadline: Some(deadline),
            ..Inner::default()
        }))
    }

    /// Cancels this token and its children. Waits on it end immediately.
    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::SeqCst);
        let _guard = self
            .0
            .lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.0.wake.notify_all();
    }

    pub fn is_cancelled(&self) -> bool {
        self.reason().is_some()
    }

    /// Why the token is cancelled; an explicit cancellation anywhere up the
    /// chain wins over a deadline.
    pub fn reason(&self) -> Option<CancelReason> {
        let mut deadline_passed = false;
        let mut token = Some(self);
        while let Some(current) = token {
            if current.0.cancelled.load(Ordering::SeqCst) {
                return Some(CancelReason::Cancelled);
            }
            deadline_passed |= current.0.deadline.is_some_and(|d| Instant::now() >= d);
            token = current.0.parent.as_ref();
        }
        deadline_passed.then_some(CancelReason::DeadlineExceeded)
    }

    /// The earliest deadline of this token and its parents.
    pub fn deadline(&self) -> Option<Instant> {
        let mut earliest: Option<Instant> = None;
        let mut token = Some(self);
        while let Some(current) = token {
            if let Some(deadline) = current.0.deadline {
                earliest = Some(earliest.map_or(deadline, |e| e.min(deadline)));
            }
            token = current.0.parent.as_ref();
        }
        earliest
    }

    /// Waits `duration` unless cancelled first; `false` if cancelled. A
    /// cancellation of this token ends the wait at once; one of a parent or
    /// a deadline within [`POLL`].
    pub fn wait(&self, duration: Duration) -> bool {
        let end = Instant::now() + duration;
        let mut guard = self
            .0
            .lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            if self.is_cancelled() {
                return false;
            }
            let now = Instant::now();
            if now >= end {
                return true;
            }
            let step = (end - now).min(POLL);
            guard = self
                .0
                .wake
                .wait_timeout(guard, step)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn children_follow_their_parent_but_not_the_other_way() {
        let run = CancellationToken::new();
        let attempt = run.child();
        attempt.cancel();
        assert!(!run.is_cancelled());
        let other = run.child();
        run.cancel();
        assert_eq!(other.reason(), Some(CancelReason::Cancelled));
    }

    #[test]
    fn deadlines_are_reported_as_such() {
        let run = CancellationToken::new();
        let attempt = run.child_with_deadline(Instant::now() + Duration::from_millis(30));
        assert!(!attempt.is_cancelled());
        assert!(!attempt.wait(Duration::from_secs(5)));
        assert_eq!(attempt.reason(), Some(CancelReason::DeadlineExceeded));
        // A cancellation wins over a passed deadline.
        run.cancel();
        assert_eq!(attempt.reason(), Some(CancelReason::Cancelled));
        assert!(attempt.deadline().is_some());
    }

    #[test]
    fn waits_end_promptly_when_cancelled() {
        let token = CancellationToken::new();
        let waiting = token.clone();
        let begun = Instant::now();
        let handle = std::thread::spawn(move || waiting.wait(Duration::from_secs(30)));
        std::thread::sleep(Duration::from_millis(50));
        token.cancel();
        assert!(!handle.join().unwrap());
        assert!(begun.elapsed() < Duration::from_secs(2));
        // Waiting on a child of a cancelled parent ends within the poll.
        let child = CancellationToken::new();
        let parent = child.clone();
        let grandchild = child.child();
        parent.cancel();
        assert!(!grandchild.wait(Duration::from_secs(30)));
    }

    #[test]
    fn uncancelled_waits_take_their_time() {
        let token = CancellationToken::new();
        let begun = Instant::now();
        assert!(token.wait(Duration::from_millis(40)));
        assert!(begun.elapsed() >= Duration::from_millis(40));
    }
}
