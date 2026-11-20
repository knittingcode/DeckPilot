//! Key gestures on the service task (`docs/gestures-and-work-session.md`,
//! section 2).
//!
//! Every press and release goes through the core's
//! [`GestureEngine`](deckpilot_core::gesture::GestureEngine), with the time
//! the device thread read it:
//!
//! - **Raw mode.** Keys without gesture bindings (`click`, `doubleClick`,
//!   `longPress` triggers) on the page shown, which includes every key of
//!   layouts from before gestures, get their edges back unchanged and at
//!   once, and are dispatched as before.
//! - **Gesture mode.** The bound gestures and the key's timings are read
//!   from the page when a sequence starts; a recognized gesture runs its
//!   binding through [`deckpilot_core::Runtime::dispatch_gesture`].
//! - **Timing.** Inputs carry the instant they were read, so a slow action
//!   on this task never turns a click into a long press.
//! - **Timers.** The run loop sleeps until the engine's next deadline;
//!   nothing is spawned per press and nothing runs on a device thread. On
//!   waking, the inputs waiting on the channel are handled first, so a late
//!   wake-up never puts a deadline before an input that was read earlier.
//! - **Invalidation.** A disconnect, a page change, a profile change or
//!   reload, and locking drop pending gestures; a gesture whose page or
//!   layout generation no longer matches is not dispatched.
//! - **Simulated presses.** A press the app simulates
//!   (`service/simulate.rs`) goes through an engine of its own, on a
//!   connection id of its own ([`simulated_connection`]), so it never
//!   joins, ends or cancels a sequence of the device's physical keys; its
//!   gestures are dispatched here like any other, as
//!   [`RunOrigin::Simulated`].

use std::sync::Arc;
use std::time::Instant;

use deckpilot_core::gesture::{
    BoundGestures, GestureCancel, GestureEngine, GestureFired, GestureOutput, GestureTimings,
    KeyContext,
};
use deckpilot_core::{ExecutionTrigger, PageId, RunOrigin, RuntimeOutcome, Trigger};
use deckpilot_device::{ControlId, ControlInput, DeviceEvent, DeviceId};
use deckpilot_ipc::SecurityState;
use tracing::{debug, warn};

use super::Agent;

/// Signals taken off the channel before a deadline is handled; the rest
/// waits for the next loop turn, which keeps the loop responsive.
const DRAIN_LIMIT: usize = 256;

/// Set in the connection id of a simulated press: device sessions count
/// up from 1 and never reach it, so a simulated sequence (and a stateful
/// key's captured press) never shares a key with the physical one.
const SIMULATED_LANE: u64 = 1 << 63;

/// The connection id simulated presses on the device session `session`
/// use.
pub(super) fn simulated_connection(session: u64) -> u64 {
    session | SIMULATED_LANE
}

/// The device session a connection id belongs to, physical or simulated.
fn session_of(connection: u64) -> u64 {
    connection & !SIMULATED_LANE
}

/// Whose input the engine's outputs are: the device's physical keys, or a
/// press the app simulated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Lane {
    Physical,
    Simulated,
}

impl Lane {
    fn edge_origin(self) -> RunOrigin {
        match self {
            Self::Physical => RunOrigin::Key,
            Self::Simulated => RunOrigin::Simulated,
        }
    }

    fn gesture_origin(self) -> RunOrigin {
        match self {
            Self::Physical => RunOrigin::Gesture,
            Self::Simulated => RunOrigin::Simulated,
        }
    }
}

/// How a workflow a key started is recorded: from the device, or, for a
/// simulated press, as simulated (whose actions are not user initiated).
pub(super) fn workflow_trigger(origin: RunOrigin) -> ExecutionTrigger {
    if origin == RunOrigin::Simulated {
        ExecutionTrigger::Simulated
    } else {
        ExecutionTrigger::Device
    }
}

/// Test hooks: supply gesture bindings and observe recognized gestures.
///
/// Without hooks, the bindings come from the page the device shows.
pub trait GestureHooks: Send + Sync {
    /// The gestures bound to `control` on `page` of `device`, with the
    /// key's timings, in place of the page's. `None`: the page's.
    fn bindings(
        &self,
        device: &DeviceId,
        page: Option<PageId>,
        control: ControlId,
    ) -> Option<(BoundGestures, GestureTimings)>;

    /// Called on the service task for every gesture that is dispatched.
    fn fired(&self, fired: &GestureFired);

    /// Called on the service task with what a dispatched gesture's binding
    /// did.
    fn dispatched(&self, _fired: &GestureFired, _outcome: &RuntimeOutcome) {}
}

#[derive(Default)]
pub(super) struct Gestures {
    engine: GestureEngine,
    /// Changes whenever the profile or its layout may have changed.
    generation: u64,
    hooks: Option<Arc<dyn GestureHooks>>,
}

impl Gestures {
    /// Whether no sequence of the key is under way.
    pub(super) fn is_idle(&self, device: &DeviceId, connection: u64, control: ControlId) -> bool {
        self.engine.is_idle(device, connection, control)
    }

    /// Drops the pending gestures of `device`: the page it shows changed
    /// without a key of it (e.g. a folder's Auto Exit).
    pub(super) fn page_changed(&mut self, device: &DeviceId) {
        self.engine.cancel(device, GestureCancel::PageChanged);
    }

    /// The layout generation gestures are recognized for now.
    pub(super) fn generation(&self) -> u64 {
        self.generation
    }

    pub(super) fn hooks(&self) -> Option<&dyn GestureHooks> {
        self.hooks.as_deref()
    }
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
        let idle = self.gestures.is_idle(device, session, control);
        self.capture_stateful_press(session, device, control, input, at, idle);
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
        let outputs = gestures
            .engine
            .on_input(at, device, session, control, input, context);
        self.handle_gesture_outputs(outputs, Lane::Physical);
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
        self.handle_gesture_outputs(outputs, Lane::Physical);
    }

    /// Drops the pending gestures of every device after the profile, its
    /// layout or the lock state changed.
    pub(super) fn invalidate_gestures(&mut self, reason: GestureCancel) {
        self.gestures.generation += 1;
        self.gestures.engine.cancel_all(reason);
        self.stateful.clear();
    }

    /// Drops the pending gestures of a device that was disconnected.
    pub(super) fn forget_gestures(&mut self, device: &DeviceId) {
        self.gestures
            .engine
            .cancel(device, GestureCancel::Disconnected);
        self.stateful.forget_device(device);
    }

    /// Dispatches what the engine reported for `lane`; returns the triggers
    /// that were dispatched (edges, and gestures that were still current).
    pub(super) fn handle_gesture_outputs(
        &mut self,
        outputs: Vec<GestureOutput>,
        lane: Lane,
    ) -> Vec<Trigger> {
        let mut dispatched = Vec::new();
        for output in outputs {
            match output {
                GestureOutput::Edge {
                    device,
                    control,
                    input,
                    ..
                } => {
                    self.dispatch_edge(&device, control, input, lane.edge_origin());
                    match input {
                        ControlInput::Press => dispatched.push(Trigger::Press),
                        ControlInput::Release => dispatched.push(Trigger::Release),
                        _ => {}
                    }
                }
                GestureOutput::Fired(fired) => {
                    if self.dispatch_gesture(&fired, lane.gesture_origin()) {
                        dispatched.push(Trigger::from(fired.gesture));
                    }
                }
            }
        }
        dispatched
    }

    /// A raw-mode edge: dispatched exactly as before gestures existed.
    fn dispatch_edge(
        &mut self,
        device: &DeviceId,
        control: ControlId,
        input: ControlInput,
        origin: RunOrigin,
    ) {
        let event = DeviceEvent::Input {
            device: device.clone(),
            control,
            input,
        };
        let shown = self.runtime.current_page(device);
        let outcome = self.runtime.handle_event_as(&event, origin);
        self.handle_outcome(device, control, origin, shown, outcome);
    }

    /// What a binding did, from a raw edge or a gesture: report an action
    /// (after applying what it asked of the agent), start a workflow in the
    /// background, and drop the device's pending gestures when the page it
    /// shows is no longer `shown`, the page the binding was on.
    pub(super) fn handle_outcome(
        &mut self,
        device: &DeviceId,
        control: ControlId,
        origin: RunOrigin,
        shown: Option<PageId>,
        outcome: Result<RuntimeOutcome, deckpilot_core::EngineError>,
    ) {
        // An action that turned the page, or a folder that closed after
        // its key ran: gestures recognized on the old page are dropped.
        if self.runtime.current_page(device) != shown {
            self.gestures
                .engine
                .cancel(device, GestureCancel::PageChanged);
        }
        match outcome {
            Ok(RuntimeOutcome::Action { action, mut result }) => {
                let name = self.action_name(action);
                // A Random Action's pick runs as the key's own binding
                // would, for this device; picks do not nest.
                let picked = super::deck::take_pick(&mut result);
                // A Multi Action Switch runs its current state the same way.
                let switch = super::switch::take_switch(&mut result);
                self.apply_deck_request(&mut result);
                self.report_action_as(action, name, &result);
                if let Some(advance_on_failure) = switch {
                    self.run_switch(
                        Some((device, control, origin, shown)),
                        action,
                        advance_on_failure,
                    );
                    return;
                }
                if let Some(target) = picked {
                    let shown = self.runtime.current_page(device);
                    let outcome = self.runtime.run_picked(device, target, origin);
                    self.handle_outcome(device, control, origin, shown, outcome);
                    return;
                }
            }
            // A plugin's action: it runs on a thread of its own and is
            // reported when it finished.
            Ok(RuntimeOutcome::ActionRequested {
                action,
                overrides,
                device,
            }) => {
                self.run_detached(action, Some(overrides), Some(device), origin, None);
                return;
            }
            Ok(RuntimeOutcome::WorkflowRequested { workflow }) => {
                if let Err(error) = self.start_execution(workflow, workflow_trigger(origin)) {
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
            // Refused, not run: the binding's action cannot run from this
            // control (`docs/device-controls.md`). Told once per binding.
            Ok(RuntimeOutcome::Unavailable {
                action,
                trigger,
                reason,
                first,
                ..
            }) => {
                let name = self.action_name(action);
                if first {
                    warn!(%device, %control, ?trigger, action = %name, %reason, "binding not run: its action cannot run from this control");
                    self.notify_unavailable(&name, control, &reason);
                } else {
                    debug!(%device, %control, ?trigger, action = %name, "unavailable binding not run");
                }
                return;
            }
            Ok(_) => {}
            Err(error) => warn!(%error, "input could not be handled"),
        }
        self.after_change();
    }

    /// A recognized gesture, if what it was recognized on is still shown;
    /// returns whether it was dispatched.
    fn dispatch_gesture(&mut self, fired: &GestureFired, origin: RunOrigin) -> bool {
        let current = self.sessions.get(&fired.device).map(|s| s.id)
            == Some(session_of(fired.connection))
            && self.security == SecurityState::Unlocked
            && fired.generation == self.gestures.generation
            && self.runtime.current_page(&fired.device) == fired.page;
        if !current {
            debug!(device = %fired.device, control = %fired.control, gesture = ?fired.gesture, "stale gesture dropped");
            return false;
        }
        debug!(device = %fired.device, control = %fired.control, gesture = ?fired.gesture, "gesture");
        if let Some(hooks) = &self.gestures.hooks {
            hooks.fired(fired);
        }
        let outcome =
            self.runtime
                .dispatch_gesture_as(&fired.device, fired.control, fired.gesture, origin);
        if let (Some(hooks), Ok(outcome)) = (&self.gestures.hooks, &outcome) {
            hooks.dispatched(fired, outcome);
        }
        if let Ok(RuntimeOutcome::StatefulRequested { binding, gesture }) = &outcome {
            self.run_stateful(fired, binding, *gesture);
            return true;
        }
        self.handle_outcome(&fired.device, fired.control, origin, fired.page, outcome);
        true
    }
}

/// The key's gesture bindings on the page `device` shows now.
pub(super) fn key_context(
    runtime: &deckpilot_core::Runtime,
    hooks: Option<&dyn GestureHooks>,
    generation: u64,
    device: &DeviceId,
    control: ControlId,
) -> KeyContext {
    let page = runtime.current_page(device);
    let bound = hooks
        .and_then(|hooks| hooks.bindings(device, page, control))
        .or_else(|| runtime.key_gestures(device, control));
    match bound {
        Some((bound, timings)) => KeyContext {
            bound,
            timings,
            page,
            generation,
        },
        None => KeyContext::raw(page, generation),
    }
}
