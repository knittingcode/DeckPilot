//! `deck.random`: runs one of several saved actions or workflows, picked at
//! random. The handler only picks; it asks the agent to run the pick with a
//! [`DeckRequest::RunAction`] or [`DeckRequest::RunWorkflow`], which runs
//! it as a key bound to it would (from a key, for that key's device).
//!
//! Candidates are lines of `action:<id>` or `workflow:<id>`, each
//! optionally followed by a weight (`workflow:<id> 3`, 1 to 100; 1 when
//! left out). The pick is uniform over the weights, from the operating
//! system's random source (`getrandom`); tests inject their own
//! [`Chooser`]. With *never the same twice*, the candidate picked last time
//! is left out of the next pick, if another one exists.

use std::collections::BTreeMap;
use std::sync::Mutex;

use deckpilot_core::graph::{ActionEffect, NodeSpec};
use deckpilot_core::{
    ActionHandler, ActionInstanceId, ActionParameters, ActionResult, ActionType, Capability,
    DeckRequest, ExecutionContext, ParameterKind, ParameterSpec, ParameterValue, Profile,
    WorkflowId, lines_of,
};

use crate::{RANDOM, output, type_id};

pub const CANDIDATES: &str = "candidates";
pub const NO_REPEAT: &str = "noRepeat";
/// Most candidates one action picks from.
pub const MAX_CANDIDATES: usize = 32;
pub const MAX_WEIGHT: u64 = 100;

pub mod codes {
    pub const NO_CANDIDATES: &str = "no-candidates";
    pub const INVALID_CANDIDATE: &str = "invalid-candidate";
    pub const CANDIDATE_NOT_FOUND: &str = "candidate-not-found";
    /// A candidate is itself a Random Action: picks do not nest.
    pub const NESTED_RANDOM: &str = "nested-random";
    pub const NO_RANDOMNESS: &str = "no-randomness";
}

/// A source of random numbers.
pub trait Chooser: Send + Sync + std::fmt::Debug {
    /// A number in `0..bound` (`bound` > 0), uniformly distributed; `None`
    /// when no randomness is available.
    fn below(&self, bound: u64) -> Option<u64>;
}

/// The operating system's random source, without modulo bias.
#[derive(Debug, Default)]
pub struct OsChooser;

impl Chooser for OsChooser {
    fn below(&self, bound: u64) -> Option<u64> {
        if bound == 0 {
            return None;
        }
        // The largest multiple of `bound` that fits; values above it are
        // drawn again (at most half of all draws for any bound).
        let zone = u64::MAX - (u64::MAX % bound);
        loop {
            let value = getrandom::u64().ok()?;
            if value < zone {
                return Some(value % bound);
            }
        }
    }
}

/// What a candidate line names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Candidate {
    Action(ActionInstanceId),
    Workflow(WorkflowId),
}

/// The candidates of `text` with their weights, or why a line is wrong.
pub fn parse_candidates(text: &str) -> Result<Vec<(Candidate, u64)>, String> {
    let mut candidates = Vec::new();
    for (number, line) in lines_of(text).enumerate() {
        let mut parts = line.split_whitespace();
        let name = parts.next().unwrap_or_default();
        let weight = match parts.next() {
            None => 1,
            Some(weight) => weight
                .parse::<u64>()
                .ok()
                .filter(|w| (1..=MAX_WEIGHT).contains(w))
                .ok_or_else(|| {
                    format!(
                        "line {}: the weight must be a number from 1 to {MAX_WEIGHT}",
                        number + 1
                    )
                })?,
        };
        if parts.next().is_some() {
            return Err(format!("line {}: one candidate per line", number + 1));
        }
        let candidate = match name.split_once(':') {
            Some(("action", id)) => id.parse().ok().map(Candidate::Action),
            Some(("workflow", id)) => id.parse().ok().map(Candidate::Workflow),
            _ => None,
        }
        .ok_or_else(|| format!("line {}: expected action:<id> or workflow:<id>", number + 1))?;
        candidates.push((candidate, weight));
    }
    Ok(candidates)
}

/// Why a candidate cannot run in `profile`, if it cannot.
fn problem(profile: &Profile, candidate: Candidate) -> Option<(&'static str, String)> {
    match candidate {
        Candidate::Action(id) => match profile.action(id) {
            None => Some((
                codes::CANDIDATE_NOT_FOUND,
                format!("action {id} does not exist"),
            )),
            Some(action) if action.action_type.as_str() == RANDOM => Some((
                codes::NESTED_RANDOM,
                format!("“{}” is a Random Action itself", action.name),
            )),
            Some(_) => None,
        },
        Candidate::Workflow(id) => profile.workflow(id).is_none().then(|| {
            (
                codes::CANDIDATE_NOT_FOUND,
                format!("workflow {id} does not exist"),
            )
        }),
    }
}

/// Picks with weights from `candidates`, leaving out `last` when another
/// candidate exists.
fn pick(
    candidates: &[(Candidate, u64)],
    last: Option<Candidate>,
    chooser: &dyn Chooser,
) -> Option<Candidate> {
    let others = candidates.iter().any(|(c, _)| Some(*c) != last);
    let open: Vec<_> = candidates
        .iter()
        .filter(|(c, _)| !others || Some(*c) != last)
        .collect();
    let total: u64 = open.iter().map(|(_, w)| w).sum();
    let mut drawn = chooser.below(total)?;
    for (candidate, weight) in open {
        if drawn < *weight {
            return Some(*candidate);
        }
        drawn -= weight;
    }
    None
}

#[derive(Debug)]
pub struct RandomAction {
    definition: ActionType,
    chooser: Box<dyn Chooser>,
    /// The last pick per list of candidates, for *never the same twice*.
    /// Kept while the agent runs.
    last: Mutex<BTreeMap<String, Candidate>>,
}

impl RandomAction {
    pub fn new() -> Self {
        Self::with_chooser(Box::new(OsChooser))
    }

    pub fn with_chooser(chooser: Box<dyn Chooser>) -> Self {
        Self {
            definition: ActionType {
                id: type_id(RANDOM),
                name: "Random action".into(),
                description: "Runs one of several actions or workflows, picked at random.".into(),
                category: "Deck".into(),
                parameters: vec![
                    ParameterSpec {
                        key: CANDIDATES.into(),
                        label: "Candidates".into(),
                        description: Some(
                            "One action or workflow per line, with an optional weight (1 to 100)."
                                .into(),
                        ),
                        kind: ParameterKind::Lines {
                            item_label: Some("Candidate".into()),
                            max_items: Some(MAX_CANDIDATES),
                            max_length: Some(64),
                        },
                        required: true,
                        default: None,
                    },
                    ParameterSpec {
                        key: NO_REPEAT.into(),
                        label: "Never the same twice in a row".into(),
                        description: None,
                        kind: ParameterKind::Boolean,
                        required: false,
                        default: Some(ParameterValue::Boolean(false)),
                    },
                ],
                requires_project: false,
                deprecated: false,
                node: NodeSpec {
                    icon: Some("shuffle".into()),
                    outputs: vec![output("picked", "Picked")],
                    timeout_ms: 5_000,
                    effect: ActionEffect::Modifies,
                    ..NodeSpec::default()
                },
            },
            chooser,
            last: Mutex::new(BTreeMap::new()),
        }
    }
}

impl Default for RandomAction {
    fn default() -> Self {
        Self::new()
    }
}

impl ActionHandler for RandomAction {
    fn definition(&self) -> &ActionType {
        &self.definition
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
        let text = parameters.text(CANDIDATES).unwrap_or_default();
        let candidates = match parse_candidates(text) {
            Ok(candidates) if candidates.is_empty() => {
                return ActionResult::error(codes::NO_CANDIDATES, "there is nothing to pick from");
            }
            Ok(candidates) => candidates,
            Err(reason) => return ActionResult::error(codes::INVALID_CANDIDATE, reason),
        };
        if let Some((code, reason)) = candidates.iter().find_map(|(c, _)| problem(profile, *c)) {
            return ActionResult::error(code, reason);
        }
        let no_repeat = parameters.boolean(NO_REPEAT).unwrap_or(false);
        let mut last = self
            .last
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let previous = no_repeat.then(|| last.get(text).copied()).flatten();
        let Some(picked) = pick(&candidates, previous, self.chooser.as_ref()) else {
            return ActionResult::error(codes::NO_RANDOMNESS, "no random number is available");
        };
        last.insert(text.to_owned(), picked);
        let (name, request) = match picked {
            Candidate::Action(action) => (
                profile.action(action).map(|a| a.name.clone()),
                DeckRequest::RunAction { action },
            ),
            Candidate::Workflow(workflow) => (
                profile.workflow(workflow).map(|w| w.name.clone()),
                DeckRequest::RunWorkflow { workflow },
            ),
        };
        let name = name.unwrap_or_default();
        ActionResult::success(format!("picked {name}"))
            .with_data("picked", name)
            .with_request(request)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    use deckpilot_core::{MachineConfig, Os, Workflow};

    use super::*;

    /// Draws `0, 1, 2, …` in turn, modulo the bound.
    #[derive(Debug, Default)]
    struct Counting(Arc<AtomicU64>);

    impl Chooser for Counting {
        fn below(&self, bound: u64) -> Option<u64> {
            Some(self.0.fetch_add(1, Ordering::Relaxed) % bound)
        }
    }

    fn instance(
        name: &str,
        action_type: deckpilot_core::ActionTypeId,
    ) -> deckpilot_core::ActionInstance {
        deckpilot_core::ActionInstance {
            id: ActionInstanceId::new(),
            name: name.into(),
            action_type,
            parameters: ActionParameters::new(),
            description: String::new(),
            icon: None,
            label: None,
        }
    }

    fn profile() -> (Profile, ActionInstanceId, WorkflowId, ActionInstanceId) {
        let mut profile = Profile::new("Work");
        let log = instance(
            "Hello",
            deckpilot_core::ActionTypeId::new("general.log").unwrap(),
        );
        let nested = instance("Dice", deckpilot_core::ActionTypeId::new(RANDOM).unwrap());
        let workflow = Workflow::new("Build");
        let ids = (log.id, workflow.id, nested.id);
        profile.actions.extend([log, nested]);
        profile.workflows.push(workflow);
        (profile, ids.0, ids.1, ids.2)
    }

    fn run(handler: &RandomAction, profile: &Profile, text: &str, no_repeat: bool) -> ActionResult {
        let machine = MachineConfig::new(Os::Linux);
        let context = ExecutionContext::new(profile, None, &machine);
        let granted = BTreeSet::from([Capability::DeckControl, Capability::ProfileRead]);
        let parameters = handler
            .definition()
            .normalize(
                &ActionParameters::new()
                    .with(CANDIDATES, ParameterValue::Text(text.into()))
                    .with(NO_REPEAT, ParameterValue::Boolean(no_repeat)),
            )
            .unwrap();
        handler.execute(&parameters, &context.restricted(&granted))
    }

    #[test]
    fn picks_one_candidate_and_asks_the_agent_to_run_it() {
        let (profile, log, workflow, _) = profile();
        let handler = RandomAction::with_chooser(Box::new(Counting::default()));
        let text = format!("action:{log}\nworkflow:{workflow}");
        let first = run(&handler, &profile, &text, false);
        assert!(first.is_success(), "{}", first.message);
        assert_eq!(
            first.deck_request(),
            Some(DeckRequest::RunAction { action: log })
        );
        assert_eq!(first.data["picked"], "Hello");
        let second = run(&handler, &profile, &text, false);
        assert_eq!(
            second.deck_request(),
            Some(DeckRequest::RunWorkflow { workflow })
        );
    }

    #[test]
    fn weights_shape_the_pick() {
        let (_, log, workflow, _) = profile();
        let candidates = [
            (Candidate::Action(log), 3),
            (Candidate::Workflow(workflow), 1),
        ];
        let chooser = Counting::default();
        let picks: Vec<_> = (0..8).map(|_| pick(&candidates, None, &chooser)).collect();
        let actions = picks
            .iter()
            .filter(|p| **p == Some(Candidate::Action(log)))
            .count();
        assert_eq!(actions, 6);
    }

    /// Always draws 0: without the setting, always the first candidate.
    #[derive(Debug)]
    struct Zero;

    impl Chooser for Zero {
        fn below(&self, _: u64) -> Option<u64> {
            Some(0)
        }
    }

    #[test]
    fn never_the_same_twice_when_asked() {
        let (profile, log, workflow, _) = profile();
        let handler = RandomAction::with_chooser(Box::new(Zero));
        let text = format!("action:{log}\nworkflow:{workflow}");
        let picks: Vec<_> = (0..4)
            .map(|_| run(&handler, &profile, &text, true).deck_request())
            .collect();
        assert_eq!(
            picks,
            [
                Some(DeckRequest::RunAction { action: log }),
                Some(DeckRequest::RunWorkflow { workflow }),
                Some(DeckRequest::RunAction { action: log }),
                Some(DeckRequest::RunWorkflow { workflow }),
            ]
        );
        // One candidate is picked every time.
        let only = format!("action:{log}");
        for _ in 0..2 {
            assert!(run(&handler, &profile, &only, true).is_success());
        }
    }

    #[test]
    fn refuses_what_it_cannot_pick_from() {
        let (profile, _, _, nested) = profile();
        let handler = RandomAction::new();
        let code = |text: &str| run(&handler, &profile, text, false).code;
        assert_eq!(
            code("not a candidate").as_deref(),
            Some(codes::INVALID_CANDIDATE)
        );
        assert_eq!(
            code(&format!("workflow:{} 0", WorkflowId::new())).as_deref(),
            Some(codes::INVALID_CANDIDATE)
        );
        assert_eq!(
            code(&format!("workflow:{}", WorkflowId::new())).as_deref(),
            Some(codes::CANDIDATE_NOT_FOUND)
        );
        assert_eq!(
            code(&format!("action:{nested}")).as_deref(),
            Some(codes::NESTED_RANDOM)
        );
    }

    #[test]
    fn the_system_source_stays_below_its_bound() {
        for bound in [1, 2, 3, 7, 100] {
            for _ in 0..50 {
                assert!(OsChooser.below(bound).unwrap() < bound);
            }
        }
        assert_eq!(OsChooser.below(0), None);
    }
}
