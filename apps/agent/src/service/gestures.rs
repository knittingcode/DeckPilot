//! Key gestures on the service task (`docs/gestures-and-work-session.md`,
//! section 2).
//!
//! Every press and release goes through the core's
//! [`GestureEngine`](deckpilot_core::gesture::GestureEngine), with the time
//! the device thread read it:
//!
//! - **Raw mode.** Keys without gesture bindings (every key until layouts
//!   can bind gestures) get their edges back unchanged and at once, and are
//!   dispatched as before.
//! - **Timing.** Inputs carry the instant they were read, so a slow action
//!   on this task never turns a click into a long press.
//! - **Timers.** The run loop sleeps until the engine's next deadline;
//!   nothing is spawned per press and nothing runs on a device thread. On
//!   waking, the inputs waiting on the channel are handled first, so a late
//!   wake-up never puts a deadline before an input that was read earlier.
//! - **Invalidation.** A disconnect, a page change, a profile change or
//!   reload, and locking drop pending gestures; a gesture whose page or
//!   layout generation no longer matches is not dispatched.

use std::sync::Arc;
use std::time::Instant;

use deckpilot_core::gesture::{
    BoundGestures, GestureCancel, GestureEngine, GestureFired, GestureOutput, GestureTimings,
    KeyContext,
};
use deckpilot_core::{ExecutionTrigger, PageId, RuntimeOutcome};
use deckpilot_device::{ControlId, ControlInput, DeviceEvent, DeviceId};
use deckpilot_ipc::SecurityState;
use tracing::{debug, warn};

use super::Agent;

/// Signals taken off the channel before a deadline is handled; the rest
/// waits for the next loop turn, which keeps the loop responsive.
const DRAIN_LIMIT: usize = 256;

/// Supplies gesture bindings and observes recognized gestures.
///
/// Layouts cannot bind gestures yet, so without hooks every key is in raw
/// mode. Tests (and later the layout model) use this to put keys in
/// gesture mode.
pub trait GestureHooks: Send + Sync {
    /// The gestures bound to `control` on `page` of `device`, with the
    /// key's timings. `None`: raw mode.
    fn bindings(
        &self,
        device: &DeviceId,
        page: Option<PageId>,
        control: ControlId,
    ) -> Option<(BoundGestures, GestureTimings)>;

    /// Called on the service task for every gesture that is dispatched.
    fn fired(&self, fired: &GestureFired);
}

#[derive(Default)]
pub(super) struct Gestures {
    engine: GestureEngine,
    /// Changes whenever the profile or its layout may have changed.
    generation: u64,
    hooks: Option<Arc<dyn GestureHooks>>,
}

impl Agent {
    /// Puts keys in gesture mode through `hooks`; see [`GestureHooks`].
    #[must_use]
    pub fn with_gesture_hooks(mut self, hooks: Arc<dyn GestureHooks>) -> Self {
        self.gestures.hooks = Some(hooks);
        self
    }

    /// One press, release or other input of `device`, read at `at`.
    pub(super) fn handle_input(
        &mut self,
        session: u64,
        device: &DeviceId,
        control: ControlId,
        input: ControlInput,
        at: Instant,
    ) {
        if self.sessions.get(device).map(|s| s.id) != Some(session) {
            return;
        }
        self.last_input_at = Some(super::unix_now());
        self.last_activity = self.now_seconds();
        if self.security != SecurityState::Unlocked {
            if matches!(input, ControlInput::Press) {
                self.prompt_unlock();
            }
            return;
        }
        let Self {
            gestures, runtime, ..
        } = self;
        // Looked up only when the input may start a sequence.
        let context = || {
            key_context(
                runtime,
                gestures.hooks.as_deref(),
                gestures.generation,
                device,
                control,
            )
        };
        let outputs =
            gestures
                .engine
                .on_input(at, device, session, control, input, context);
        self.handle_gesture_outputs(outputs);
    }

    /// When the next pending gesture is due, if any is pending.
    pub(super) fn next_gesture_deadline(&self) -> Option<Instant> {
        self.gestures.engine.next_deadline()
    }

    /// A gesture deadline passed.
    pub(super) fn on_gesture_deadline(&mut self) {
        let now = Instant::now();
        // Inputs read before `now` may still wait on the channel; they come
        // first. Each device sends in the order it reads, so this is
        // timestamp order per device, and devices are independent.
        for _ in 0..DRAIN_LIMIT {
            match self.signals.try_recv() {
                Ok(signal) => self.handle_signal(signal),
                Err(_) => break,
            }
        }
        let outputs = self.gestures.engine.advance(now);
        self.handle_gesture_outputs(outputs);
    }

    /// Drops the pending gestures of every device after the profile, its
    /// layout or the lock state changed.
    pub(super) fn invalidate_gestures(&mut self, reason: GestureCancel) {
        self.gestures.generation += 1;
        self.gestures.engine.cancel_all(reason);
    }

    /// Drops the pending gestures of a device that was disconnected.
    pub(super) fn forget_gestures(&mut self, device: &DeviceId) {
        self.gestures
            .engine
            .cancel(device, GestureCancel::Disconnected);
    }

    fn handle_gesture_outputs(&mut self, outputs: Vec<GestureOutput>) {
        for output in outputs {
            match output {
                GestureOutput::Edge {
                    device,
                    control,
                    input,
                    ..
                } => self.dispatch_edge(&device, control, input),
                GestureOutput::Fired(fired) => self.dispatch_gesture(&fired),
            }
        }
    }

    /// A raw-mode edge: dispatched exactly as before gestures existed.
    fn dispatch_edge(&mut self, device: &DeviceId, control: ControlId, input: ControlInput) {
        let event = DeviceEvent::Input {
            device: device.clone(),
            control,
            input,
        };
        match self.runtime.handle_event(&event) {
            Ok(RuntimeOutcome::Action { action, result }) => {
                self.report_action(action, &result);
            }
            Ok(RuntimeOutcome::WorkflowRequested { workflow }) => {
                if let Err(error) = self.start_execution(workflow, ExecutionTrigger::Device) {
                    warn!(%error, "cannot start the workflow");
                }
            }
            Ok(RuntimeOutcome::Navigated { .. }) => {
                self.gestures
                    .engine
                    .cancel(device, GestureCancel::PageChanged);
            }
            Ok(RuntimeOutcome::Ignored { reason }) => {
                debug!(%device, %control, %reason, "input ignored");
                return;
            }
            Ok(_) => {}
            Err(error) => warn!(%error, "input could not be handled"),
        }
        self.after_change();
    }

    /// A recognized gesture, if what it was recognized on is still shown.
    fn dispatch_gesture(&mut self, fired: &GestureFired) {
        let current = self.sessions.get(&fired.device).map(|s| s.id) == Some(fired.connection)
            && self.security == SecurityState::Unlocked
            && fired.generation == self.gestures.generation
            && self.runtime.current_page(&fired.device) == fired.page;
        if !current {
            debug!(device = %fired.device, control = %fired.control, gesture = ?fired.gesture, "stale gesture dropped");
            return;
        }
        debug!(device = %fired.device, control = %fired.control, gesture = ?fired.gesture, "gesture");
        if let Some(hooks) = &self.gestures.hooks {
            hooks.fired(fired);
        }
        // TODO(G2): `Runtime::dispatch_gesture` runs the bound target here,
        // cancelling the device's gestures when it navigates.
    }
}

/// The key's gesture bindings on the page `device` shows now.
fn key_context(
    runtime: &deckpilot_core::Runtime,
    hooks: Option<&dyn GestureHooks>,
    generation: u64,
    device: &DeviceId,
    control: ControlId,
) -> KeyContext {
    let page = runtime.current_page(device);
    // TODO(G2): read the bound gestures and timings from the page.
    match hooks.and_then(|hooks| hooks.bindings(device, page, control)) {
        Some((bound, timings)) => KeyContext {
            bound,
            timings,
            page,
            generation,
        },
        None => KeyContext::raw(page, generation),
    }
}
