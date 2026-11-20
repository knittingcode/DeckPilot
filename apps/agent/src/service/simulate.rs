//! Simulated presses for the editor's interactive preview (`simulatePress`,
//! protocol 14, `docs/editor-navigation-and-simulation.md`).
//!
//! - **The real pipeline.** A simulated press is a sequence of press and
//!   release edges with their times, fed into a [`GestureEngine`] and
//!   dispatched by the same code as a device's input
//!   ([`Agent::handle_gesture_outputs`]): raw keys get their edges, gesture
//!   keys the gesture the engine recognizes with the key's own timings
//!   (the page's per-key settings over this computer's defaults). There is
//!   no second execution path, and the agent stays the one that decides.
//! - **A virtual clock.** The edges carry instants on a clock of their
//!   own, starting now: a long press is released after the key's
//!   long-press time, a double press's second press comes well inside the
//!   double-click window, and the engine is advanced past the window at
//!   the end. Nothing sleeps and the service task is never blocked; the
//!   engine times every gesture from those instants exactly as it would a
//!   held key.
//! - **A lane of its own.** The engine is a new one per press, on a
//!   connection id of its own ([`simulated_connection`]): a physical
//!   sequence of the same key is neither joined, ended nor cancelled, and
//!   a stateful key's captured press is kept apart. Nothing is written to
//!   the device except the key images the binding itself changes.
//! - **Checked first.** Refused while locked, for a device that is not
//!   connected, unless the device shows the profile, layout and page the
//!   app asked for, and for a control the model does not have or that
//!   cannot be pressed.
//! - **Not user initiated, never more permitted.** Bindings run as
//!   `RunOrigin::Simulated` (plugin dialogs and `url.open` stay refused);
//!   workflows they start are recorded as simulated. A press whose
//!   bindings may change something outside DeckPilot needs `confirmed`
//!   ([`Agent::needs_confirmation`]); the settings that gate those actions
//!   on this computer apply as for a real press either way.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use deckpilot_core::gesture::{Gesture, GestureEngine, GestureTimings};
use deckpilot_core::graph::{ActionEffect, NodeKind};
use deckpilot_core::{
    ActionInstanceId, ActionParameters, BindingTarget, LayoutId, PageId, ProfileId, Trigger,
    WorkflowId,
};
use deckpilot_device::{ControlId, ControlInput, DeviceId};
use deckpilot_ipc::SecurityState;
use deckpilot_ipc::simulation::{
    MAX_CONFIRMATION_NAMES, PressKind, SimulatedTrigger, SimulationOutcome, SimulationRefusal,
};
use deckpilot_modules::deck;
use tracing::info;

use super::Agent;
use super::gestures::{Lane, key_context, simulated_connection};

/// How long a simulated key is held for a single or double press.
const TAP: Duration = Duration::from_millis(40);
/// Between the release and the second press of a double press: well inside
/// the shortest double-click window (150 ms).
const GAP: Duration = Duration::from_millis(40);
/// A long press is held this much past the key's long-press time.
const HOLD_MARGIN: Duration = Duration::from_millis(40);
/// Modules whose actions change only what DeckPilot itself shows or
/// selects (pages, layouts, profiles, brightness, the active project): no
/// confirmation for them.
const LOCAL_MODULES: [&str; 2] = [deck::MODULE_ID, "projects"];
/// How deep confirmation follows workflows and picks into each other.
const MAX_DEPTH: usize = 8;

/// What the app asked to press.
pub(super) struct PressRequest {
    pub device: DeviceId,
    pub profile: ProfileId,
    pub layout: LayoutId,
    pub page: PageId,
    pub control: ControlId,
    pub kind: PressKind,
    pub confirmed: bool,
}

/// The edges of a press of `kind` for a key with `timings`, as offsets
/// from the first press, and when the engine has seen everything.
fn edges(kind: PressKind, timings: GestureTimings) -> (Vec<(Duration, ControlInput)>, Duration) {
    let window = Duration::from_millis(timings.double_click_ms.into());
    let hold = Duration::from_millis(timings.long_press_ms.into());
    let edges = match kind {
        PressKind::Single => vec![
            (Duration::ZERO, ControlInput::Press),
            (TAP, ControlInput::Release),
        ],
        PressKind::Double => vec![
            (Duration::ZERO, ControlInput::Press),
            (TAP, ControlInput::Release),
            (TAP + GAP, ControlInput::Press),
            (TAP + GAP + TAP, ControlInput::Release),
        ],
        PressKind::Long => vec![
            (Duration::ZERO, ControlInput::Press),
            (hold + HOLD_MARGIN, ControlInput::Release),
        ],
    };
    let last = edges.last().map_or(Duration::ZERO, |(at, _)| *at);
    // Past the double-click window after the last release, so a pending
    // click is decided.
    (edges, last + window + Duration::from_millis(1))
}

fn trigger(trigger: Trigger) -> SimulatedTrigger {
    match trigger {
        Trigger::Press => SimulatedTrigger::Press,
        Trigger::Release => SimulatedTrigger::Release,
        Trigger::Click => SimulatedTrigger::Click,
        Trigger::DoubleClick => SimulatedTrigger::DoubleClick,
        Trigger::LongPress => SimulatedTrigger::LongPress,
        _ => SimulatedTrigger::Other,
    }
}

impl Agent {
    /// Runs a simulated press, or says why not.
    pub(super) fn simulate_press(&mut self, request: &PressRequest) -> SimulationOutcome {
        if let Err(refused) = self.check_press(request) {
            return refused;
        }
        let names = self.needs_confirmation(request);
        if !names.is_empty() && !request.confirmed {
            return SimulationOutcome::ConfirmationRequired {
                action_names: names,
                reason: "this press runs actions that may change something outside DeckPilot \
                         (commands, services, requests, plugins that modify)"
                    .into(),
            };
        }
        let PressRequest {
            device,
            control,
            kind,
            ..
        } = request;
        let Some(session) = self.sessions.get(device).map(|s| s.id) else {
            return SimulationOutcome::refused(
                SimulationRefusal::DeviceNotConnected,
                "the device is not connected",
            );
        };
        info!(%device, %control, ?kind, confirmed = request.confirmed, "simulated press");
        self.last_activity = self.now_seconds();
        let connection = simulated_connection(session);
        let timings = self.key_timings(device, *control);
        let (edges, end) = edges(*kind, timings);
        let mut engine = GestureEngine::new();
        let start = Instant::now();
        let mut dispatched = Vec::new();
        for (offset, input) in edges {
            let at = start + offset;
            let idle = engine.is_idle(device, connection, *control);
            self.capture_stateful_press(connection, device, *control, input, at, idle);
            let outputs = {
                let (runtime, gestures) = (&self.runtime, &self.gestures);
                let context = || {
                    key_context(
                        runtime,
                        gestures.hooks(),
                        gestures.generation(),
                        device,
                        *control,
                    )
                };
                engine.on_input(at, device, connection, *control, input, context)
            };
            dispatched.extend(self.handle_gesture_outputs(outputs, Lane::Simulated));
        }
        let outputs = engine.advance(start + end);
        dispatched.extend(self.handle_gesture_outputs(outputs, Lane::Simulated));
        SimulationOutcome::Ran {
            dispatched: dispatched.into_iter().map(trigger).collect(),
        }
    }

    /// The timings the gesture engine uses for `control` now.
    fn key_timings(&self, device: &DeviceId, control: ControlId) -> GestureTimings {
        key_context(
            &self.runtime,
            self.gestures.hooks(),
            self.gestures.generation(),
            device,
            control,
        )
        .timings
    }

    /// Refuses a press the device cannot take as asked.
    fn check_press(&self, request: &PressRequest) -> Result<(), SimulationOutcome> {
        let refuse = |code, message: &str| Err(SimulationOutcome::refused(code, message));
        if self.security != SecurityState::Unlocked {
            return refuse(SimulationRefusal::Locked, "DeckPilot is locked");
        }
        let device = &request.device;
        if !self.sessions.contains_key(device) {
            return refuse(
                SimulationRefusal::DeviceNotConnected,
                "the device is not connected",
            );
        }
        let Some(profile) = self.runtime.profile() else {
            return refuse(SimulationRefusal::NoActiveProfile, "no profile is active");
        };
        if profile.id != request.profile {
            return refuse(
                SimulationRefusal::WrongProfile,
                "another profile is active; the preview is out of date",
            );
        }
        if self.runtime.layout_of(device).map(|l| l.id) != Some(request.layout) {
            return refuse(
                SimulationRefusal::WrongLayout,
                "the device shows another layout",
            );
        }
        if self.runtime.current_page(device) != Some(request.page) {
            return refuse(
                SimulationRefusal::WrongPage,
                "the device shows another page",
            );
        }
        let Some(model) = self.runtime.catalog().get(&device.model) else {
            return refuse(
                SimulationRefusal::UnknownControl,
                "the device model is not known",
            );
        };
        let capabilities = &model.capabilities;
        if !capabilities.controls().contains(&request.control) {
            return refuse(
                SimulationRefusal::UnknownControl,
                "the device has no such control",
            );
        }
        let pressable = [ControlInput::Press, ControlInput::Release]
            .into_iter()
            .all(|input| capabilities.validate_input(request.control, input).is_ok());
        if !pressable {
            return refuse(
                SimulationRefusal::IncompatibleControl,
                "the control cannot be pressed",
            );
        }
        Ok(())
    }

    /// The names of what the press may run that changes something outside
    /// DeckPilot: the bindings the engine may dispatch for this press of
    /// this key (its press and release in raw mode, else the gestures this
    /// kind of press can end in), followed into workflows, Random Action
    /// candidates and Multi Action Switch states. Empty: nothing needs
    /// confirmation.
    fn needs_confirmation(&self, request: &PressRequest) -> Vec<String> {
        let (Some(layout), Some(profile)) = (
            self.runtime.layout_of(&request.device),
            self.runtime.profile(),
        ) else {
            return Vec::new();
        };
        let Some(page) = layout.page(request.page) else {
            return Vec::new();
        };
        let control = request.control;
        // A folder key only opens its folder (`deckpilot_core::folder_control`):
        // nothing that changes anything outside DeckPilot.
        if layout.folder_control(request.page, control).is_some() {
            return Vec::new();
        }
        let bound = key_context(
            &self.runtime,
            self.gestures.hooks(),
            self.gestures.generation(),
            &request.device,
            control,
        )
        .bound;
        let mut targets: Vec<&BindingTarget> = Vec::new();
        if bound.any() {
            let gesture = match request.kind {
                PressKind::Double if bound.double_click => Gesture::DoubleClick,
                PressKind::Long if bound.long_press => Gesture::LongPress,
                _ => Gesture::Click,
            };
            targets.extend(page.gesture_binding(control, gesture).map(|b| &b.target));
        } else {
            for input in [ControlInput::Press, ControlInput::Release] {
                targets.extend(page.binding_for(control, input).map(|b| &b.target));
            }
        }
        let mut walk = Confirmation {
            agent: self,
            profile,
            names: Vec::new(),
            seen_actions: BTreeSet::new(),
            seen_workflows: BTreeSet::new(),
        };
        for target in targets {
            walk.target(target, 0);
        }
        walk.names.truncate(MAX_CONFIRMATION_NAMES);
        walk.names
    }
}

/// Walks what a press may run, collecting the names that need confirmation.
struct Confirmation<'a> {
    agent: &'a Agent,
    profile: &'a deckpilot_core::Profile,
    names: Vec<String>,
    seen_actions: BTreeSet<ActionInstanceId>,
    seen_workflows: BTreeSet<WorkflowId>,
}

impl Confirmation<'_> {
    fn push(&mut self, name: String) {
        if !self.names.contains(&name) {
            self.names.push(name);
        }
    }

    fn target(&mut self, target: &BindingTarget, depth: usize) {
        match target {
            BindingTarget::Action { action, overrides } => self.action(*action, overrides, depth),
            BindingTarget::Workflow { workflow } => self.workflow(*workflow, depth),
            // A stateful key changes the work session, which may run its
            // start or verification workflow.
            BindingTarget::Stateful(_) => self.push("Work session".into()),
            BindingTarget::OpenPage { .. } | BindingTarget::Back => {}
        }
    }

    fn action(&mut self, action: ActionInstanceId, overrides: &ActionParameters, depth: usize) {
        let Some(instance) = self.profile.action(action) else {
            return;
        };
        if depth > MAX_DEPTH || !self.seen_actions.insert(action) {
            return;
        }
        let registry = self.agent.runtime.engine().registry();
        let kind = &instance.action_type;
        let parameters = instance.parameters.overlay(overrides);
        // A pick runs one of its candidates, a switch its current state:
        // what they may run decides.
        let picks = match kind.as_str() {
            deck::RANDOM => parameters
                .text(deck::random::CANDIDATES)
                .and_then(|text| deck::random::parse_candidates(text).ok())
                .map(|candidates| candidates.into_iter().map(|(c, _)| c).collect::<Vec<_>>()),
            deck::MULTI_SWITCH => parameters
                .text(deck::switch::STATES)
                .and_then(|text| deck::switch::parse_states(text).ok())
                .map(|states| states.into_iter().map(|s| s.target).collect()),
            _ => None,
        };
        if let Some(picks) = picks {
            for pick in picks {
                match pick {
                    deck::random::Candidate::Action(next) => {
                        self.action(next, &ActionParameters::new(), depth + 1);
                    }
                    deck::random::Candidate::Workflow(next) => self.workflow(next, depth + 1),
                }
            }
            return;
        }
        let Some(definition) = registry.definition(kind) else {
            // Not known here (a plugin that is not loaded): it cannot run,
            // but nothing says it is harmless.
            self.push(instance.name.clone());
            return;
        };
        if definition.node.effect != ActionEffect::Modifies {
            return;
        }
        let local = registry
            .module_of(kind)
            .is_some_and(|module| LOCAL_MODULES.contains(&module.as_str()));
        // An HTTP request that only reads.
        let reads = kind.as_str() == "http.request"
            && parameters
                .text("method")
                .is_some_and(|m| matches!(m.trim(), "GET" | "HEAD"));
        if !local && !reads {
            self.push(instance.name.clone());
        }
    }

    fn workflow(&mut self, workflow: WorkflowId, depth: usize) {
        let Some(definition) = self.profile.workflow(workflow) else {
            return;
        };
        if depth > MAX_DEPTH || !self.seen_workflows.insert(workflow) {
            return;
        }
        let before = self.names.len();
        match &definition.graph {
            Some(graph) => {
                for node in &graph.nodes {
                    match &node.kind {
                        NodeKind::Action {
                            action, overrides, ..
                        } => self.action(*action, overrides, depth + 1),
                        NodeKind::Call { workflow, .. } | NodeKind::Loop { workflow, .. } => {
                            self.workflow(*workflow, depth + 1);
                        }
                        _ => {}
                    }
                }
            }
            None => {
                for step in &definition.steps {
                    self.action(step.action, &step.overrides, depth + 1);
                }
            }
        }
        // Named as the workflow the user bound, then its steps.
        if self.names.len() > before {
            self.names.insert(before, definition.name.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_follow_the_keys_timings() {
        let timings = GestureTimings {
            double_click_ms: 250,
            long_press_ms: 450,
        };
        let (single, end) = edges(PressKind::Single, timings);
        assert_eq!(
            single,
            [
                (Duration::ZERO, ControlInput::Press),
                (TAP, ControlInput::Release)
            ]
        );
        assert!(end > TAP + Duration::from_millis(250));
        let (double, _) = edges(PressKind::Double, timings);
        assert_eq!(double.len(), 4);
        // The second press comes inside the shortest window there is.
        assert!(double[2].0.checked_sub(double[1].0).unwrap() < Duration::from_millis(150));
        let (long, _) = edges(PressKind::Long, timings);
        assert!(long[1].0 >= Duration::from_millis(450));
    }
}
