//! Live key states from module providers.
//!
//! Keys bound to an action whose module declares a
//! [`RefreshPolicy`](deckpilot_core::RefreshPolicy) show a
//! [`ButtonState`](deckpilot_core::ButtonState). The agent asks for it off
//! the device loop and keeps the last answer per key:
//!
//! - **Off the loop.** Providers run on their own threads (at most
//!   [`MAX_RUNNING`] at a time) and report back as
//!   [`Command::ButtonState`]; the agent task never waits for one.
//! - **Bounded.** A provider is called for a key only while the key is
//!   shown on a device, at most once per the policy's minimum interval
//!   (measured between starts), and never while a call for that key is
//!   still running. Nothing polls per frame: the run loop sleeps until the
//!   next state is due.
//! - **Cached.** The last state per key (action and project) is kept and
//!   shown at once when the key appears again, until a fresh one arrives;
//!   states of keys not shown for [`EVICT_AFTER`] are dropped.
//! - **Cancelled.** Each call gets a cancellation token with a time limit
//!   ([`CALL_TIMEOUT`]). Reloading the configuration, switching profiles,
//!   locking and shutting down cancel running calls and discard their
//!   results.
//! - **Harmless failures.** A failing or panicking provider shows an error
//!   mark on its key. Only the error code is logged, and only when a key
//!   turns to an error.

use std::collections::{BTreeMap, BTreeSet};
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::{Duration, Instant};

use deckpilot_core::{
    ActionInstanceId, BindingTarget, ButtonState, CancellationToken, Engine, Indicator, KeyFace,
    MachineConfig, Profile, ProjectId, RefreshPolicy, StateError, StateSnapshot,
};
use deckpilot_device::{ControlId, DeviceId};
use tracing::debug;

use super::{Agent, Command};

/// Providers that may run at the same time.
pub(crate) const MAX_RUNNING: usize = 4;
/// Longest time one call may take; it is cancelled after that.
pub(crate) const CALL_TIMEOUT: Duration = Duration::from_secs(20);
/// States of keys that were not shown for this long are dropped.
pub(crate) const EVICT_AFTER: Duration = Duration::from_secs(10 * 60);

/// Whose state: a key's action with the project it was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct StateKey {
    pub action: ActionInstanceId,
    pub project: Option<ProjectId>,
}

/// A provider's answer, sent from its thread to the agent task.
#[derive(Debug)]
pub(crate) struct StateUpdate {
    generation: u64,
    key: StateKey,
    result: Result<ButtonState, StateError>,
}

#[derive(Debug)]
struct Entry {
    policy: RefreshPolicy,
    snapshot: StateSnapshot,
    /// When the last call started.
    started: Option<Instant>,
    /// The token of the running call.
    running: Option<CancellationToken>,
    /// Something happened since the last call started.
    stale: bool,
    last_shown: Instant,
}

impl Entry {
    /// When the next call may start; `None`: not until an event.
    fn due_at(&self, now: Instant) -> Option<Instant> {
        if self.running.is_some() {
            return None;
        }
        let Some(started) = self.started else {
            return Some(now);
        };
        let polls = self.policy.expires_after().is_some();
        (polls || self.stale).then(|| started + self.policy.min_interval())
    }
}

/// The cache and scheduler of live key states. Pure bookkeeping with
/// explicit times; [`Agent`] runs the calls.
#[derive(Debug, Default)]
pub(crate) struct ButtonStates {
    /// Bumped by [`ButtonStates::reset`]; older answers are discarded.
    generation: u64,
    entries: BTreeMap<StateKey, Entry>,
    /// Keys with live state on each device's visible page.
    shown: BTreeMap<DeviceId, BTreeSet<StateKey>>,
    /// What calls run against; taken from the runtime when needed.
    context: Option<Arc<CallContext>>,
}

#[derive(Debug)]
struct CallContext {
    engine: Arc<Engine>,
    profile: Profile,
    machine: MachineConfig,
}

impl ButtonStates {
    /// What `key` shows now. Registers the key with `policy` the first time
    /// it is seen (as loading).
    pub(crate) fn observe(
        &mut self,
        key: StateKey,
        policy: RefreshPolicy,
        now: Instant,
    ) -> StateSnapshot {
        let entry = self.entries.entry(key).or_insert_with(|| Entry {
            policy,
            snapshot: StateSnapshot::Loading,
            started: None,
            running: None,
            stale: false,
            last_shown: now,
        });
        entry.policy = policy;
        entry.last_shown = now;
        entry.snapshot.clone()
    }

    /// Replaces the keys shown on `device`.
    pub(crate) fn set_shown(&mut self, device: &DeviceId, keys: BTreeSet<StateKey>) {
        if keys.is_empty() {
            self.shown.remove(device);
        } else {
            self.shown.insert(device.clone(), keys);
        }
    }

    pub(crate) fn forget_device(&mut self, device: &DeviceId) {
        self.shown.remove(device);
    }

    fn is_shown(&self, key: &StateKey) -> bool {
        self.shown.values().any(|keys| keys.contains(key))
    }

    /// Something happened (a key press, a project switch, a finished
    /// workflow): every state may be refreshed once its interval allows.
    pub(crate) fn invalidate(&mut self) {
        for entry in self.entries.values_mut() {
            entry.stale = true;
        }
    }

    /// Cancels running calls, discards their answers and every cached
    /// state. For a reload, a profile switch, locking and shutdown.
    pub(crate) fn reset(&mut self) {
        for entry in self.entries.values() {
            if let Some(token) = &entry.running {
                token.cancel();
            }
        }
        self.entries.clear();
        self.context = None;
        self.generation += 1;
    }

    fn running(&self) -> usize {
        self.entries
            .values()
            .filter(|entry| entry.running.is_some())
            .count()
    }

    /// The keys whose call should start now, at most as many as may run
    /// alongside the running ones, each with its token. Marks them as
    /// running. Drops states not shown for [`EVICT_AFTER`].
    pub(crate) fn start_due(&mut self, now: Instant) -> Vec<(StateKey, CancellationToken)> {
        self.entries.retain(|_, entry| {
            entry.running.is_some() || now.saturating_duration_since(entry.last_shown) < EVICT_AFTER
        });
        let free = MAX_RUNNING.saturating_sub(self.running());
        let mut due: Vec<(Instant, StateKey)> = self
            .entries
            .iter()
            .filter(|(key, _)| self.is_shown(key))
            .filter_map(|(key, entry)| entry.due_at(now).map(|at| (at, *key)))
            .filter(|(at, _)| *at <= now)
            .collect();
        // The longest waiting first.
        due.sort();
        due.truncate(free);
        due.into_iter()
            .filter_map(|(_, key)| {
                let entry = self.entries.get_mut(&key)?;
                let token = CancellationToken::new().child_with_deadline(now + CALL_TIMEOUT);
                entry.started = Some(now);
                entry.stale = false;
                entry.running = Some(token.clone());
                Some((key, token))
            })
            .collect()
    }

    /// When the next shown state is due, if any; a running call reports
    /// back by itself.
    pub(crate) fn next_due(&self, now: Instant) -> Option<Instant> {
        if self.running() >= MAX_RUNNING {
            return None;
        }
        self.entries
            .iter()
            .filter(|(key, _)| self.is_shown(key))
            .filter_map(|(_, entry)| entry.due_at(now))
            .min()
    }

    /// Records an answer. Returns whether what the key shows changed;
    /// answers from before a reset are ignored.
    pub(crate) fn finish(&mut self, update: StateUpdate) -> bool {
        if update.generation != self.generation {
            return false;
        }
        let Some(entry) = self.entries.get_mut(&update.key) else {
            return false;
        };
        entry.running = None;
        let snapshot = match update.result {
            Ok(state) => StateSnapshot::Ready { state },
            Err(error) => {
                if !matches!(entry.snapshot, StateSnapshot::Failed { .. }) {
                    // The code only: messages may contain machine details.
                    debug!(action = %update.key.action, code = %error.code, "key state unavailable");
                }
                StateSnapshot::Failed { error }
            }
        };
        if entry.snapshot == snapshot {
            return false;
        }
        entry.snapshot = snapshot;
        true
    }
}

/// Runs one provider call on its own thread and reports back.
fn spawn_call(
    context: Arc<CallContext>,
    generation: u64,
    key: StateKey,
    cancel: CancellationToken,
    commands: tokio::sync::mpsc::UnboundedSender<Command>,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("button-state".into())
        .spawn(move || {
            // A panicking provider must not leave the key waiting forever.
            let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
                let project = key
                    .project
                    .and_then(|project| context.profile.project(project));
                context.engine.button_state(
                    &context.profile,
                    &context.machine,
                    project,
                    key.action,
                    &cancel,
                )
            }));
            let result = match result {
                Ok(Some(result)) => result,
                Ok(None) => Ok(ButtonState::default()),
                Err(_) => Err(StateError::new(
                    "provider-failed",
                    "the state could not be determined",
                )),
            };
            let _ = commands.send(Command::ButtonState(StateUpdate {
                generation,
                key,
                result,
            }));
        })
        .map(|_| ())
}

impl Agent {
    /// `faces` of `device` with live states and running workflows applied;
    /// records which keys with live state the device shows.
    pub(super) fn decorate_faces(
        &mut self,
        device: &DeviceId,
        faces: &mut [(ControlId, KeyFace)],
        now: Instant,
    ) {
        let project = self.runtime.active_project_id();
        let mut shown = BTreeSet::new();
        for (control, target) in self.runtime.shown_targets(device) {
            let Some(face) = faces
                .iter_mut()
                .find(|(c, _)| *c == control)
                .map(|(_, face)| face)
            else {
                continue;
            };
            match target {
                BindingTarget::Action { action, .. } => {
                    let Some(policy) = self
                        .runtime
                        .profile()
                        .and_then(|profile| self.runtime.engine().state_policy(profile, action))
                    else {
                        continue;
                    };
                    let key = StateKey { action, project };
                    shown.insert(key);
                    let snapshot = self.states.observe(key, policy, now);
                    *face = std::mem::replace(face, KeyFace::empty()).with_snapshot(&snapshot);
                }
                BindingTarget::Workflow { workflow } => {
                    if let Some(percent) = self.executions.progress_of(workflow) {
                        *face = std::mem::replace(face, KeyFace::empty())
                            .with_indicator(Indicator::Running)
                            .with_progress(Some(percent));
                    }
                }
                BindingTarget::Stateful(_) => {
                    // The machine's face is W4's; a hint the last gesture
                    // left is shown under it.
                    if let Some(hint) = self.stateful_hint(device, control) {
                        *face = std::mem::replace(face, KeyFace::empty()).with_detail(hint);
                    }
                }
                BindingTarget::OpenPage { .. } | BindingTarget::Back => {}
            }
        }
        self.states.set_shown(device, shown);
    }

    /// Starts the provider calls that are due.
    pub(super) fn refresh_states(&mut self) {
        let now = Instant::now();
        let due = self.states.start_due(now);
        if due.is_empty() {
            return;
        }
        if self.states.context.is_none() {
            // Calls run against a snapshot, taken once per configuration.
            let Some(profile) = self.runtime.profile() else {
                self.states.reset();
                return;
            };
            self.states.context = Some(Arc::new(CallContext {
                engine: self.runtime.shared_engine(),
                profile: profile.clone(),
                machine: self.runtime.machine().clone(),
            }));
        }
        let Some(context) = self.states.context.clone() else {
            return;
        };
        for (key, cancel) in due {
            if let Err(error) = spawn_call(
                context.clone(),
                self.states.generation,
                key,
                cancel,
                self.handle.commands.clone(),
            ) {
                debug!(%error, "cannot start a key state thread");
                self.states.finish(StateUpdate {
                    generation: self.states.generation,
                    key,
                    result: Err(StateError::new(
                        "provider-failed",
                        "the state could not be determined",
                    )),
                });
            }
        }
    }

    /// When the run loop should wake up for the next state.
    pub(super) fn next_state_due(&self) -> Option<Instant> {
        self.states.next_due(Instant::now())
    }

    pub(super) fn handle_button_state(&mut self, update: StateUpdate) {
        if self.states.finish(update) {
            self.render_all();
        } else {
            // A slot became free.
            self.refresh_states();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> StateKey {
        StateKey {
            action: ActionInstanceId::new(),
            project: None,
        }
    }

    fn device(serial: &str) -> DeviceId {
        DeviceId {
            model: deckpilot_device::DeviceModelId::new("test.model"),
            serial: serial.into(),
        }
    }

    fn show(states: &mut ButtonStates, keys: &[StateKey]) {
        states.set_shown(&device("A"), keys.iter().copied().collect());
    }

    fn answer(states: &ButtonStates, key: StateKey, label: &str) -> StateUpdate {
        StateUpdate {
            generation: states.generation,
            key,
            result: Ok(ButtonState::new().with_label(label)),
        }
    }

    const SECOND: Duration = Duration::from_secs(1);

    #[test]
    fn a_new_key_loads_once_and_then_shows_its_state() {
        let mut states = ButtonStates::default();
        let now = Instant::now();
        let key = key();
        let policy = RefreshPolicy::every(30 * SECOND);
        assert_eq!(states.observe(key, policy, now), StateSnapshot::Loading);
        show(&mut states, &[key]);
        assert_eq!(states.next_due(now), Some(now));
        assert_eq!(states.start_due(now).len(), 1);
        // Not again while the call runs.
        assert!(states.start_due(now + 60 * SECOND).is_empty());
        assert_eq!(states.next_due(now), None);

        assert!(states.finish(answer(&states, key, "main")));
        let ready = states.observe(key, policy, now + SECOND);
        assert!(
            matches!(ready, StateSnapshot::Ready { state } if state.label.as_deref() == Some("main"))
        );
        // The same answer again changes nothing to draw.
        states.start_due(now + 30 * SECOND);
        assert!(!states.finish(answer(&states, key, "main")));
    }

    #[test]
    fn polled_states_are_refreshed_at_their_interval_and_no_sooner() {
        let mut states = ButtonStates::default();
        let start = Instant::now();
        let key = key();
        states.observe(key, RefreshPolicy::every(30 * SECOND), start);
        show(&mut states, &[key]);
        let mut calls = 0;
        // Ten minutes of events every second.
        for second in 0..600 {
            let now = start + second * SECOND;
            states.invalidate();
            states.observe(key, RefreshPolicy::every(30 * SECOND), now);
            for (key, _) in states.start_due(now) {
                calls += 1;
                states.finish(answer(&states, key, "x"));
            }
        }
        assert_eq!(calls, 20);
        assert_eq!(
            states.next_due(start + 600 * SECOND),
            Some(start + 600 * SECOND)
        );
    }

    #[test]
    fn event_driven_states_wait_for_an_event_and_their_interval() {
        let mut states = ButtonStates::default();
        let now = Instant::now();
        let key = key();
        let policy = RefreshPolicy::on_events(5 * SECOND);
        states.observe(key, policy, now);
        show(&mut states, &[key]);
        states.start_due(now);
        states.finish(answer(&states, key, "a"));
        // No event: never again.
        assert_eq!(states.next_due(now + 60 * SECOND), None);
        assert!(states.start_due(now + 60 * SECOND).is_empty());

        let later = now + 60 * SECOND;
        states.observe(key, policy, later);
        assert!(states.start_due(later).is_empty());
        states.invalidate();
        assert_eq!(states.next_due(later), Some(now + 5 * SECOND));
        assert_eq!(states.start_due(later).len(), 1);
        states.finish(answer(&states, key, "b"));
        // An event right after waits for the interval.
        states.invalidate();
        assert!(states.start_due(later + SECOND).is_empty());
        assert_eq!(states.next_due(later + SECOND), Some(later + 5 * SECOND));
        assert_eq!(states.start_due(later + 5 * SECOND).len(), 1);
    }

    #[test]
    fn hidden_keys_are_not_refreshed_and_are_evicted() {
        let mut states = ButtonStates::default();
        let now = Instant::now();
        let (a, b) = (key(), key());
        let policy = RefreshPolicy::every(SECOND);
        states.observe(a, policy, now);
        states.observe(b, policy, now);
        show(&mut states, &[a]);
        let started: Vec<_> = states.start_due(now).into_iter().map(|(k, _)| k).collect();
        assert_eq!(started, [a]);
        states.finish(answer(&states, a, "a"));

        states.forget_device(&device("A"));
        assert!(states.start_due(now + 10 * SECOND).is_empty());
        assert_eq!(states.next_due(now + 10 * SECOND), None);

        // Kept for a while: shown again, it shows the cached state at once.
        let back = now + 60 * SECOND;
        assert!(matches!(
            states.observe(a, policy, back),
            StateSnapshot::Ready { .. }
        ));
        states.start_due(back + EVICT_AFTER);
        assert_eq!(
            states.observe(a, policy, back + EVICT_AFTER),
            StateSnapshot::Loading
        );
    }

    #[test]
    fn concurrency_is_bounded() {
        let mut states = ButtonStates::default();
        let now = Instant::now();
        let keys: Vec<StateKey> = (0..10).map(|_| key()).collect();
        for key in &keys {
            states.observe(*key, RefreshPolicy::every(SECOND), now);
        }
        show(&mut states, &keys);
        let first = states.start_due(now);
        assert_eq!(first.len(), MAX_RUNNING);
        assert!(states.start_due(now).is_empty());
        assert_eq!(states.next_due(now), None);
        states.finish(answer(&states, first[0].0, "x"));
        assert_eq!(states.start_due(now).len(), 1);
    }

    #[test]
    fn a_reset_cancels_calls_and_discards_their_answers() {
        let mut states = ButtonStates::default();
        let now = Instant::now();
        let key = key();
        states.observe(key, RefreshPolicy::every(SECOND), now);
        show(&mut states, &[key]);
        let (_, token) = states.start_due(now).pop().unwrap();
        let late = answer(&states, key, "old");
        assert!(token.deadline().is_some());
        states.reset();
        assert!(token.is_cancelled());
        assert!(!states.finish(late));
        assert_eq!(
            states.observe(key, RefreshPolicy::every(SECOND), now),
            StateSnapshot::Loading
        );
        // The device still shows the key, so it loads again.
        assert_eq!(states.start_due(now).len(), 1);
    }

    #[test]
    fn failures_become_error_states() {
        let mut states = ButtonStates::default();
        let now = Instant::now();
        let key = key();
        states.observe(key, RefreshPolicy::every(SECOND), now);
        show(&mut states, &[key]);
        states.start_due(now);
        assert!(states.finish(StateUpdate {
            generation: states.generation,
            key,
            result: Err(StateError::new("not-found", "missing")),
        }));
        assert!(matches!(
            states.observe(key, RefreshPolicy::every(SECOND), now),
            StateSnapshot::Failed { error } if error.code == "not-found"
        ));
    }
}
