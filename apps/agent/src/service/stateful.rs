//! Stateful keys on the service task (`docs/gestures-and-work-session.md`,
//! section 4): a gesture of a key bound to a state machine asks the
//! machine for the operation its rules pick from the state **at the
//! press**.
//!
//! - **Press-time state.** When a press starts a sequence on a stateful
//!   key, the work session snapshot is captured with the press's time.
//!   The gesture that sequence ends in uses that state, so a press that
//!   began in `working` never becomes a finish because the verification
//!   passed while the key was down; the service also refuses an operation
//!   whose press state is no longer current.
//! - **Through the work session service only.** The operation goes to
//!   [`Agent::work_session_operation`] on this task, never on a device
//!   thread; the binding never sets state.
//! - **Repeats.** A press that comes less than [`REPEAT_GUARD`] after an
//!   operation of the machine was applied does nothing, so a double click
//!   on Finish does not start the next session, and a double click on
//!   Start does not verify at once. (The arming delay of a finish after an
//!   approval, D9, is the service's.)
//! - **Hints.** An operation the service ignores with a hint ("Hold to
//!   finish", "Unlock DeckPilot", …) puts the hint on that key until the
//!   session changes or the key's next gesture.
//! - **Bounded.** One captured press per key whose sequence has not ended
//!   and one hint per key; both are dropped with the device, the profile
//!   or the lock.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use deckpilot_core::WorkSessionId;
use deckpilot_core::gesture::{Gesture, GestureFired};
use deckpilot_core::stateful::StatefulBinding;
use deckpilot_core::work_session::{WorkSessionSnapshot, WorkSessionState};
use deckpilot_device::{ControlId, ControlInput, DeviceId};
use tracing::{debug, warn};

use super::Agent;
use super::work_session::{PressContext, WorkSessionOutcome, WorkSessionRequest};

/// A press this soon after an applied operation of the machine does
/// nothing.
pub const REPEAT_GUARD: Duration = Duration::from_secs(1);

/// The state a sequence started in.
#[derive(Debug)]
struct Press {
    at: Instant,
    snapshot: WorkSessionSnapshot,
}

/// A hint shown on a key while the session stays as it was.
#[derive(Debug, Clone, Copy)]
struct Hint {
    text: &'static str,
    state: WorkSessionState,
    session: Option<WorkSessionId>,
}

#[derive(Debug, Default)]
pub(super) struct StatefulKeys {
    /// Captured at the first press of a sequence, by device connection
    /// and key.
    presses: BTreeMap<(DeviceId, u64, ControlId), Press>,
    hints: BTreeMap<(DeviceId, ControlId), Hint>,
    /// When an operation was last applied from a key.
    last_applied: Option<Instant>,
}

impl StatefulKeys {
    /// Forgets the presses and hints of `device` (it was disconnected).
    pub(super) fn forget_device(&mut self, device: &DeviceId) {
        self.presses.retain(|(d, _, _), _| d != device);
        self.hints.retain(|(d, _), _| d != device);
    }

    /// Forgets every press and hint (the profile, its layout or the lock
    /// changed).
    pub(super) fn clear(&mut self) {
        self.presses.clear();
        self.hints.clear();
    }
}

impl Agent {
    /// Captures the work session's state when a press starts a sequence
    /// on a stateful key. Called before the gesture engine sees the press.
    pub(super) fn capture_stateful_press(
        &mut self,
        connection: u64,
        device: &DeviceId,
        control: ControlId,
        input: ControlInput,
        at: Instant,
    ) {
        if input != ControlInput::Press
            || !self.gestures.is_idle(device, connection, control)
            || self.runtime.stateful_binding(device, control).is_none()
        {
            return;
        }
        let snapshot = self.work_session_snapshot();
        self.stateful.presses.insert(
            (device.clone(), connection, control),
            Press { at, snapshot },
        );
    }

    /// The hint `control` of `device` shows, while the session is as it
    /// was when the hint was given.
    pub(super) fn stateful_hint(&self, device: &DeviceId, control: ControlId) -> Option<&str> {
        let hint = self.stateful.hints.get(&(device.clone(), control))?;
        let now = self.work.snapshot();
        (now.state == hint.state && now.session == hint.session).then_some(hint.text)
    }

    /// A gesture of a stateful key: the operation its rules pick from the
    /// state at the press, applied by the work session service.
    pub(super) fn run_stateful(
        &mut self,
        fired: &GestureFired,
        binding: &StatefulBinding,
        gesture: Gesture,
    ) {
        let device = &fired.device;
        let press = self
            .stateful
            .presses
            .remove(&(device.clone(), fired.connection, fired.control))
            .filter(|press| press.at == fired.pressed_at);
        self.stateful.hints.remove(&(device.clone(), fired.control));
        let outcome = self.stateful_outcome(press, binding, gesture, fired.at);
        if let Some(WorkSessionOutcome::Ignored(reason)) = &outcome
            && !reason.hint().is_empty()
        {
            let now = self.work.snapshot();
            self.stateful.hints.insert(
                (device.clone(), fired.control),
                Hint {
                    text: reason.hint(),
                    state: now.state,
                    session: now.session,
                },
            );
        }
        if let Some(WorkSessionOutcome::Failed(message)) = &outcome {
            warn!(%message, "work session operation failed");
        }
        self.render(device, false);
    }

    fn stateful_outcome(
        &mut self,
        press: Option<Press>,
        binding: &StatefulBinding,
        gesture: Gesture,
        fired_at: Instant,
    ) -> Option<WorkSessionOutcome> {
        let Some(press) = press else {
            debug!(?gesture, "stateful gesture without its press state ignored");
            return None;
        };
        if self
            .stateful
            .last_applied
            .is_some_and(|last| press.at < last + REPEAT_GUARD)
        {
            debug!(?gesture, "pressed again too soon after an operation");
            return None;
        }
        let Some(operation) = binding.work_session_operation(press.snapshot.state, gesture) else {
            debug!(state = press.snapshot.state.name(), ?gesture, "no rule");
            return None;
        };
        let outcome = self.work_session_operation(&WorkSessionRequest {
            operation,
            gesture,
            press: PressContext::of(&press.snapshot, press.at),
        });
        if outcome == WorkSessionOutcome::Applied {
            self.stateful.last_applied = Some(fired_at);
        }
        Some(outcome)
    }
}
