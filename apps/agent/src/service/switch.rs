//! Multi Action Switches (`deck.multi-switch`, see
//! `modules/deck/src/switch.rs`): which state each one is in, running that
//! state's target, moving on when the run succeeded, and what the key shows.
//!
//! - **One state per key binding** ([`SwitchKey`]: profile, page, control
//!   and switch action), so two keys bound to the same switch action each
//!   go their own way, and every device showing that page shows the same
//!   key state. Run from the app (no key), the switch has a state of its
//!   own. Every switch starts at its first state.
//! - **Persisted.** The states are saved in the `settings` table
//!   (`switch.states`, a JSON list of profile, page, control, action and
//!   state index) when they change
//!   and read when the agent starts, so a restart keeps them. An index past
//!   the end of the switch's states (states were removed) is the first
//!   state; entries of switch actions that no longer exist are dropped when
//!   the states are saved.
//! - **Advancing.** An action target advances the switch when its result is
//!   a success; a workflow when its execution finishes as succeeded. A
//!   failed or cancelled run leaves the state, unless the switch advances on
//!   failure. A run that cannot be known (a plugin's action, which reports
//!   later) advances at once.
//! - **Shown as a key state.** The key shows the current state as a
//!   [`ButtonState`] (title, glyph and `n/m`), the description every key
//!   with live state uses, applied in `decorate_faces`.

use std::collections::BTreeMap;

use deckpilot_core::{
    ActionInstanceId, ActionResult, BindingTarget, ButtonState, DeckRequest, ExecutionId,
    ExecutionState, ExecutionSummary, ExecutionTrigger, PageId, ProfileId, RunOrigin,
    RuntimeOutcome,
};
use deckpilot_device::{ControlId, DeviceId};
use deckpilot_modules::deck::{self, random::Candidate, switch::SwitchState};
use deckpilot_storage::ConfigStore;
use tracing::warn;

use super::Agent;

/// The setting the states are saved in.
const SWITCH_SETTING: &str = "switch.states";

/// Whose state: a key binding of a switch action, or the switch run from
/// the app (`page` and `control` none).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SwitchKey {
    profile: ProfileId,
    page: Option<PageId>,
    control: Option<ControlId>,
    action: ActionInstanceId,
}

/// One entry of [`SWITCH_SETTING`], a JSON list: a key and its state.
type SavedSwitch = (
    ProfileId,
    Option<PageId>,
    Option<ControlId>,
    ActionInstanceId,
    usize,
);

#[derive(Debug, Default)]
pub(crate) struct Switches {
    /// The current state per key; missing: the first.
    current: BTreeMap<SwitchKey, usize>,
    /// Workflows a switch started: the key and the state it ran, to advance
    /// from when the run succeeds.
    pending: BTreeMap<ExecutionId, (SwitchKey, usize)>,
}

/// Takes a switch's request out of `result`: whether it advances on
/// failure.
pub(super) fn take_switch(result: &mut ActionResult) -> Option<bool> {
    let DeckRequest::RunSwitch { advance_on_failure } = result.deck_request()? else {
        return None;
    };
    result.request = None;
    Some(advance_on_failure)
}

fn target(state: &SwitchState) -> BindingTarget {
    match state.target {
        Candidate::Action(action) => BindingTarget::action(action),
        Candidate::Workflow(workflow) => BindingTarget::Workflow { workflow },
    }
}

impl Agent {
    /// The key of the switch `action` bound to `control` on `page` (both
    /// none: run from the app), in the active profile.
    fn switch_key(
        &self,
        page: Option<PageId>,
        control: Option<ControlId>,
        action: ActionInstanceId,
    ) -> Option<SwitchKey> {
        Some(SwitchKey {
            profile: self.runtime.profile()?.id,
            page,
            control,
            action,
        })
    }

    /// The states of the switch of `key` and the current one, if its action
    /// is a switch with states that can be read.
    fn switch_states(&self, key: SwitchKey) -> Option<(Vec<SwitchState>, usize)> {
        let instance = self.runtime.profile()?.action(key.action)?;
        if instance.action_type.as_str() != deck::MULTI_SWITCH {
            return None;
        }
        let text = instance.parameters.text(deck::switch::STATES)?;
        let states = deck::switch::parse_states(text).ok()?;
        if states.is_empty() {
            return None;
        }
        let index = self
            .switches
            .current
            .get(&key)
            .copied()
            .filter(|index| *index < states.len())
            .unwrap_or(0);
        Some((states, index))
    }

    /// What `control` on `page`, bound to the switch action `action`,
    /// shows.
    pub(super) fn switch_face(
        &self,
        page: Option<PageId>,
        control: ControlId,
        action: ActionInstanceId,
    ) -> Option<ButtonState> {
        let key = self.switch_key(page, Some(control), action)?;
        let (states, index) = self.switch_states(key)?;
        Some(states[index].face(index, states.len()))
    }

    /// Runs the current state of the switch `action`: for the key `at`
    /// (device, control, origin and the page the binding is on) as that
    /// key's own binding would, or, with none, as from the app.
    pub(super) fn run_switch(
        &mut self,
        at: Option<(&DeviceId, ControlId, RunOrigin, Option<PageId>)>,
        action: ActionInstanceId,
        advance_on_failure: bool,
    ) {
        let key = match at {
            Some((_, control, _, page)) => self.switch_key(page, Some(control), action),
            None => self.switch_key(None, None, action),
        };
        let Some((key, (states, index))) = key.and_then(|k| Some((k, self.switch_states(k)?)))
        else {
            return;
        };
        let state = &states[index];
        let Some((device, control, origin, _)) = at else {
            self.run_switch_from_app(key, index, state, advance_on_failure);
            return;
        };
        let shown = self.runtime.current_page(device);
        let outcome = self.runtime.run_picked(device, target(state), origin);
        match &outcome {
            Ok(RuntimeOutcome::Action { result, .. }) => {
                if result.is_success() || advance_on_failure {
                    self.advance_switch(key, index);
                }
            }
            // Started here, so that its end can advance the switch.
            Ok(RuntimeOutcome::WorkflowRequested { workflow }) => {
                self.start_switch_workflow(
                    key,
                    index,
                    *workflow,
                    super::gestures::workflow_trigger(origin),
                    advance_on_failure,
                );
                self.after_change();
                return;
            }
            Ok(_) => self.advance_switch(key, index),
            Err(_) if advance_on_failure => self.advance_switch(key, index),
            Err(_) => {}
        }
        self.handle_outcome(device, control, origin, shown, outcome);
    }

    fn run_switch_from_app(
        &mut self,
        key: SwitchKey,
        index: usize,
        state: &SwitchState,
        advance_on_failure: bool,
    ) {
        match state.target {
            Candidate::Workflow(workflow) => self.start_switch_workflow(
                key,
                index,
                workflow,
                ExecutionTrigger::App,
                advance_on_failure,
            ),
            Candidate::Action(run) => match self.runtime.run_action(run) {
                Ok(mut result) => {
                    let name = self.action_name(run);
                    self.apply_deck_request(&mut result);
                    self.report_action_as(run, name, &result);
                    if result.is_success() || advance_on_failure {
                        self.advance_switch(key, index);
                    }
                }
                Err(error) => {
                    warn!(%error, "the switch's state could not run");
                    if advance_on_failure {
                        self.advance_switch(key, index);
                    }
                }
            },
        }
    }

    fn start_switch_workflow(
        &mut self,
        key: SwitchKey,
        index: usize,
        workflow: deckpilot_core::WorkflowId,
        trigger: ExecutionTrigger,
        advance_on_failure: bool,
    ) {
        match self.start_execution(workflow, trigger) {
            Ok(_) if advance_on_failure => self.advance_switch(key, index),
            Ok(execution) => {
                self.switches.pending.insert(execution.id, (key, index));
            }
            Err(error) => {
                warn!(%error, "the switch's workflow could not start");
                if advance_on_failure {
                    self.advance_switch(key, index);
                }
            }
        }
    }

    /// A workflow ended: the switch that started it moves on if it
    /// succeeded.
    pub(super) fn switch_run_ended(&mut self, execution: &ExecutionSummary) {
        if let Some((key, index)) = self.switches.pending.remove(&execution.id)
            && execution.state == ExecutionState::Succeeded
        {
            self.advance_switch(key, index);
        }
    }

    /// Moves the switch of `key` from state `from` to the next one, if it
    /// is still in `from`; saves the states and redraws.
    fn advance_switch(&mut self, key: SwitchKey, from: usize) {
        let Some((states, index)) = self.switch_states(key) else {
            return;
        };
        if index != from {
            return;
        }
        self.switches
            .current
            .insert(key, (index + 1) % states.len());
        self.persist_switches();
        self.render_all();
    }

    fn persist_switches(&mut self) {
        // Switches of the active profile that no longer exist are dropped.
        if let Some(profile) = self.runtime.profile() {
            let (id, actions): (ProfileId, Vec<ActionInstanceId>) =
                (profile.id, profile.actions.iter().map(|a| a.id).collect());
            self.switches
                .current
                .retain(|key, _| key.profile != id || actions.contains(&key.action));
        }
        let Some(store) = self.store.as_ref() else {
            return;
        };
        let entries: Vec<SavedSwitch> = self
            .switches
            .current
            .iter()
            .map(|(key, index)| (key.profile, key.page, key.control, key.action, *index))
            .collect();
        let Ok(value) = serde_json::to_string(&entries) else {
            return;
        };
        if let Err(error) = store.set_setting(SWITCH_SETTING, &value) {
            warn!(%error, "cannot save the states of the switches");
        }
    }

    /// Reads the saved states; an unreadable value is ignored (every switch
    /// starts at its first state).
    pub(super) fn restore_switches(&mut self) {
        let Some(store) = self.store.as_ref() else {
            return;
        };
        match store.setting(SWITCH_SETTING) {
            Ok(Some(saved)) => {
                if let Ok(entries) = serde_json::from_str::<Vec<SavedSwitch>>(&saved) {
                    self.switches.current = entries
                        .into_iter()
                        .map(|(profile, page, control, action, index)| {
                            let key = SwitchKey {
                                profile,
                                page,
                                control,
                                action,
                            };
                            (key, index)
                        })
                        .collect();
                } else {
                    warn!("the saved switch states are unreadable and were ignored");
                }
            }
            Ok(None) => {}
            Err(error) => warn!(%error, "cannot read the saved switch states"),
        }
    }
}
