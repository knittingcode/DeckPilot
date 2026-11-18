//! `deck.multi-switch`, *Multi Action Switch*: a key with two to eight
//! states that it goes through in turn. Each state runs a saved action or
//! workflow (a Multi Action is a workflow) and may name the title and the
//! glyph the key shows while it is the current state.
//!
//! The handler checks the states and asks the agent with
//! [`DeckRequest::RunSwitch`]; the agent keeps which state is current (per
//! Multi Action Switch action, saved in its settings so it survives a
//! restart), runs that state's target as a key bound to it would, and moves
//! on to the next state when the run succeeded (or always, with
//! *advance on failure*). What the key shows is a [`ButtonState`], the
//! description every key with live state uses: the state's title as the
//! label, its glyph, and `n/m` as the detail.
//!
//! States are lines of `<target>[ | <title>[ | <glyph>]]`, where the
//! target is `action:<id>` or `workflow:<id>` and the glyph one of the
//! renderer's closed set (`check`, `play`, `pause`, …).

use deckpilot_core::button_state::MAX_STATE_TEXT;
use deckpilot_core::graph::{ActionEffect, NodeSpec};
use deckpilot_core::{
    ActionHandler, ActionParameters, ActionResult, ActionType, ButtonState, Capability,
    DeckRequest, ExecutionContext, Glyph, ParameterKind, ParameterSpec, ParameterValue, Profile,
    lines_of,
};

use crate::random::Candidate;
use crate::{MULTI_SWITCH, RANDOM, type_id};

pub const STATES: &str = "states";
pub const ADVANCE_ON_FAILURE: &str = "advanceOnFailure";
pub const MIN_STATES: usize = 2;
pub const MAX_STATES: usize = 8;

pub mod codes {
    pub const TOO_FEW_STATES: &str = "too-few-states";
    pub const INVALID_STATE: &str = "invalid-state";
    pub const STATE_NOT_FOUND: &str = "state-not-found";
    /// A state runs a Random Action or another switch: they do not nest.
    pub const NESTED_SWITCH: &str = "nested-switch";
}

/// One state of a switch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchState {
    pub target: Candidate,
    pub title: Option<String>,
    pub glyph: Option<Glyph>,
}

impl SwitchState {
    /// What the key shows while this is state `index` of `count`.
    pub fn face(&self, index: usize, count: usize) -> ButtonState {
        let mut state = ButtonState::new().with_detail(format!("{}/{count}", index + 1));
        if let Some(title) = &self.title {
            state = state.with_label(title.clone());
        }
        if let Some(glyph) = self.glyph {
            state = state.with_glyph(glyph);
        }
        state.sanitized()
    }
}

/// The glyph `name` (`check`, `play`, …): its name in profiles and IPC.
pub fn glyph(name: &str) -> Option<Glyph> {
    Some(match name {
        "check" => Glyph::Check,
        "cross" => Glyph::Cross,
        "play" => Glyph::Play,
        "pause" => Glyph::Pause,
        "stop" => Glyph::Stop,
        "timer" => Glyph::Timer,
        "bell" => Glyph::Bell,
        "up" => Glyph::Up,
        "down" => Glyph::Down,
        "dot" => Glyph::Dot,
        "circle" => Glyph::Circle,
        "square" => Glyph::Square,
        "diamond" => Glyph::Diamond,
        "triangle" => Glyph::Triangle,
        "star" => Glyph::Star,
        "heart" => Glyph::Heart,
        _ => return None,
    })
}

/// The states of `text`, or why a line is wrong.
pub fn parse_states(text: &str) -> Result<Vec<SwitchState>, String> {
    let mut states = Vec::new();
    for (number, line) in lines_of(text).enumerate() {
        let wrong = |what: &str| format!("state {}: {what}", number + 1);
        let mut parts = line.split('|').map(str::trim);
        let target = parts.next().unwrap_or_default();
        let target = match target.split_once(':') {
            Some(("action", id)) => id.parse().ok().map(Candidate::Action),
            Some(("workflow", id)) => id.parse().ok().map(Candidate::Workflow),
            _ => None,
        }
        .ok_or_else(|| wrong("expected action:<id> or workflow:<id>"))?;
        let title = parts
            .next()
            .filter(|t| !t.is_empty())
            .map(|t| t.chars().take(MAX_STATE_TEXT).collect());
        let glyph = match parts.next().filter(|g| !g.is_empty()) {
            None => None,
            Some(name) => Some(glyph(name).ok_or_else(|| wrong(&format!("unknown glyph {name}")))?),
        };
        if parts.next().is_some() {
            return Err(wrong("expected <target> | <title> | <glyph>"));
        }
        states.push(SwitchState {
            target,
            title,
            glyph,
        });
    }
    Ok(states)
}

/// The states of a switch's parameters, checked against `profile`: the
/// error code and message when they cannot run.
pub fn checked_states(
    parameters: &ActionParameters,
    profile: &Profile,
) -> Result<Vec<SwitchState>, (&'static str, String)> {
    let states = parse_states(parameters.text(STATES).unwrap_or_default())
        .map_err(|reason| (codes::INVALID_STATE, reason))?;
    if states.len() < MIN_STATES {
        return Err((
            codes::TOO_FEW_STATES,
            format!("a switch needs at least {MIN_STATES} states"),
        ));
    }
    for state in &states {
        match state.target {
            Candidate::Action(id) => match profile.action(id) {
                None => {
                    return Err((
                        codes::STATE_NOT_FOUND,
                        format!("action {id} does not exist"),
                    ));
                }
                Some(action) if matches!(action.action_type.as_str(), RANDOM | MULTI_SWITCH) => {
                    return Err((
                        codes::NESTED_SWITCH,
                        format!("“{}” picks or switches itself", action.name),
                    ));
                }
                Some(_) => {}
            },
            Candidate::Workflow(id) => {
                if profile.workflow(id).is_none() {
                    return Err((
                        codes::STATE_NOT_FOUND,
                        format!("workflow {id} does not exist"),
                    ));
                }
            }
        }
    }
    Ok(states)
}

#[derive(Debug)]
pub struct MultiSwitch(ActionType);

impl MultiSwitch {
    pub fn new() -> Self {
        Self(ActionType {
            id: type_id(MULTI_SWITCH),
            name: "Multi Action Switch".into(),
            description: "Goes through two or more states in turn, each running an action or \
                          workflow; the key shows the current state."
                .into(),
            category: "Deck".into(),
            parameters: vec![
                ParameterSpec {
                    key: STATES.into(),
                    label: "States".into(),
                    description: Some(
                        "One state per line: an action or workflow, then an optional title and \
                         glyph."
                            .into(),
                    ),
                    kind: ParameterKind::Lines {
                        item_label: Some("State".into()),
                        max_items: Some(MAX_STATES),
                        max_length: Some(128),
                    },
                    required: true,
                    default: None,
                },
                ParameterSpec {
                    key: ADVANCE_ON_FAILURE.into(),
                    label: "Go to the next state when the run fails".into(),
                    description: None,
                    kind: ParameterKind::Boolean,
                    required: false,
                    default: Some(ParameterValue::Boolean(false)),
                },
            ],
            requires_project: false,
            deprecated: false,
            controls: Some(deckpilot_core::ActionControls::keys()),
            node: NodeSpec {
                icon: Some("layers".into()),
                timeout_ms: 5_000,
                effect: ActionEffect::Modifies,
                ..NodeSpec::default()
            },
        })
    }
}

impl Default for MultiSwitch {
    fn default() -> Self {
        Self::new()
    }
}

impl ActionHandler for MultiSwitch {
    fn definition(&self) -> &ActionType {
        &self.0
    }

    fn execute(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        if let Err(denied) = context.permit(Capability::DeckControl) {
            return denied.into();
        }
        let profile = match context.profile() {
            Ok(profile) => profile,
            Err(denied) => return denied.into(),
        };
        match checked_states(parameters, profile) {
            Ok(_) => ActionResult::success("switch").with_request(DeckRequest::RunSwitch {
                advance_on_failure: parameters.boolean(ADVANCE_ON_FAILURE).unwrap_or(false),
            }),
            Err((code, reason)) => ActionResult::error(code, reason),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use deckpilot_core::{ActionInstance, ActionInstanceId, MachineConfig, Os, Workflow};

    use super::*;

    fn instance(name: &str, action_type: &str) -> ActionInstance {
        ActionInstance {
            id: ActionInstanceId::new(),
            name: name.into(),
            action_type: deckpilot_core::ActionTypeId::new(action_type).unwrap(),
            parameters: ActionParameters::new(),
            description: String::new(),
            icon: None,
            label: None,
        }
    }

    fn run(profile: &Profile, text: &str) -> ActionResult {
        let machine = MachineConfig::new(Os::Linux);
        let context = ExecutionContext::new(profile, None, &machine);
        let granted = BTreeSet::from([Capability::DeckControl, Capability::ProfileRead]);
        let handler = MultiSwitch::new();
        let parameters = handler
            .definition()
            .normalize(&ActionParameters::new().with(STATES, ParameterValue::Text(text.into())))
            .unwrap();
        handler.execute(&parameters, &context.restricted(&granted))
    }

    #[test]
    fn reads_targets_titles_and_glyphs() {
        let (a, w) = (ActionInstanceId::new(), deckpilot_core::WorkflowId::new());
        let states = parse_states(&format!("action:{a} | Mute | pause\nworkflow:{w}\n")).unwrap();
        assert_eq!(states.len(), 2);
        assert_eq!(states[0].target, Candidate::Action(a));
        assert_eq!(states[0].title.as_deref(), Some("Mute"));
        assert_eq!(states[0].glyph, Some(Glyph::Pause));
        assert_eq!(states[1].title, None);
        let face = states[0].face(0, 2);
        assert_eq!(face.label.as_deref(), Some("Mute"));
        assert_eq!(face.detail.as_deref(), Some("1/2"));
        assert_eq!(face.glyph, Some(Glyph::Pause));
        assert!(parse_states(&format!("action:{a} | x | sparkles")).is_err());
        // Every glyph of the renderer's set has its name.
        let names = [
            "check", "cross", "play", "pause", "stop", "timer", "bell", "up", "down", "dot",
            "circle", "square", "diamond", "triangle", "star", "heart",
        ];
        let parsed: std::collections::HashSet<Glyph> =
            names.iter().map(|name| glyph(name).unwrap()).collect();
        assert_eq!(parsed.len(), Glyph::ALL.len());
        assert!(parse_states("nothing").is_err());
    }

    #[test]
    fn asks_the_agent_to_run_the_current_state_and_refuses_what_cannot_run() {
        let mut profile = Profile::new("Work");
        let log = instance("Hello", "general.log");
        let dice = instance("Dice", RANDOM);
        let workflow = Workflow::new("Build");
        let (log_id, dice_id, workflow_id) = (log.id, dice.id, workflow.id);
        profile.actions.extend([log, dice]);
        profile.workflows.push(workflow);

        let ok = run(
            &profile,
            &format!("action:{log_id} | On\nworkflow:{workflow_id} | Off"),
        );
        assert!(ok.is_success(), "{}", ok.message);
        assert_eq!(
            ok.deck_request(),
            Some(DeckRequest::RunSwitch {
                advance_on_failure: false
            })
        );
        let code = |text: String| run(&profile, &text).code;
        assert_eq!(
            code(format!("action:{log_id}")).as_deref(),
            Some(codes::TOO_FEW_STATES)
        );
        assert_eq!(
            code(format!(
                "action:{log_id}\naction:{}",
                ActionInstanceId::new()
            ))
            .as_deref(),
            Some(codes::STATE_NOT_FOUND)
        );
        assert_eq!(
            code(format!("action:{log_id}\naction:{dice_id}")).as_deref(),
            Some(codes::NESTED_SWITCH)
        );
    }
}
