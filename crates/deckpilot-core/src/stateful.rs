//! Stateful key bindings (`docs/gestures-and-work-session.md`, section 4).
//!
//! A stateful key asks a built-in state machine for an operation, and
//! which operation depends on the machine's state **at the press** and on
//! the gesture. The binding never sets state itself: it names an
//! operation, and the machine's owner checks its guards against its
//! current state before it applies it.
//!
//! - **Typed.** Each machine declares its states and operations as closed
//!   Rust enums, and its rules are `(state, gesture) → operation` data. A
//!   binding cannot name a state or operation its machine does not have
//!   (it does not decode), and there are no expressions, conditions or
//!   scripts.
//! - **Explicit transitions.** [`StateMachine::ALLOWED`] lists the
//!   operations a key may ask for in each state; every one of them is a
//!   transition of the machine's own table (for the work session,
//!   [`crate::work_session::TRANSITIONS`]; tested below).
//! - **Validated.** [`StatefulBinding::issues`] refuses an operation that
//!   is not allowed in its state, a `(state, gesture)` listed twice, a rule
//!   for a gesture the key does not bind to the machine, and what the
//!   machine itself forbids (a work session finishes only by a click,
//!   section 5.3).
//! - **Defaults.** A binding without rules follows the machine's default
//!   table, so a key works as soon as it is assigned. The table is not
//!   stored.

use std::fmt::Debug;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::gesture::{BoundGestures, Gesture};
use crate::work_session::{WorkSessionEvent, WorkSessionState};

/// One rule of a stateful key: in `state`, `gesture` asks for `operation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StatefulRule<S, O> {
    pub state: S,
    pub gesture: Gesture,
    pub operation: O,
}

impl<S, O> StatefulRule<S, O> {
    pub const fn new(state: S, gesture: Gesture, operation: O) -> Self {
        Self {
            state,
            gesture,
            operation,
        }
    }
}

/// A built-in state machine a key can follow. Its states, operations and
/// rules are data; nothing in a binding is evaluated.
pub trait StateMachine {
    type State: Copy + Eq + Debug + Serialize + 'static;
    type Operation: Copy + Eq + Debug + Serialize + 'static;

    /// The machine's name in a binding (`"machine": …`).
    const ID: &'static str;
    /// Shown on a key that has no label of its own.
    const NAME: &'static str;
    /// Every state, a closed list.
    const STATES: &'static [Self::State];
    /// Every operation a key can ask for, a closed list.
    const OPERATIONS: &'static [Self::Operation];
    /// The operations a key may ask for in each state.
    const ALLOWED: &'static [(Self::State, Self::Operation)];
    /// The rules of a binding that lists none.
    const DEFAULT_RULES: &'static [StatefulRule<Self::State, Self::Operation>];

    /// Whether `gesture` may ask for `operation` at all.
    fn gesture_allowed(_operation: Self::Operation, _gesture: Gesture) -> bool {
        true
    }
}

/// What is wrong with the rules of a stateful binding.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StatefulIssue {
    #[error("`{operation}` cannot be asked for in `{state}`")]
    NotAllowed { state: String, operation: String },
    #[error("`{state}` has more than one rule for {gesture:?}")]
    DuplicateRule { state: String, gesture: Gesture },
    #[error(
        "a rule reacts to {gesture:?}, but {gesture:?} on this key is not bound to the {machine}"
    )]
    GestureNotBound {
        machine: &'static str,
        gesture: Gesture,
    },
    #[error("`{operation}` cannot be asked for by {gesture:?}")]
    GestureNotAllowed { operation: String, gesture: Gesture },
}

/// The name a value has in a document, e.g. `readyToFinish`.
fn wire_name(value: &impl Serialize) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(name)) => name,
        other => format!("{other:?}"),
    }
}

/// The rules in effect: `rules`, or the machine's defaults without any.
fn effective<M: StateMachine>(
    rules: &[StatefulRule<M::State, M::Operation>],
) -> &[StatefulRule<M::State, M::Operation>] {
    if rules.is_empty() {
        M::DEFAULT_RULES
    } else {
        rules
    }
}

/// The operation `gesture` asks for in `state`, if a rule has one.
fn operation_for<M: StateMachine>(
    rules: &[StatefulRule<M::State, M::Operation>],
    state: M::State,
    gesture: Gesture,
) -> Option<M::Operation> {
    effective::<M>(rules)
        .iter()
        .find(|rule| rule.state == state && rule.gesture == gesture)
        .map(|rule| rule.operation)
}

/// Checks `rules` against the machine; `bound` are the gestures of the key
/// that are bound to this machine. Default rules for gestures the key does
/// not bind simply never fire.
fn check<M: StateMachine>(
    rules: &[StatefulRule<M::State, M::Operation>],
    bound: BoundGestures,
) -> Vec<StatefulIssue> {
    let mut issues = Vec::new();
    for (index, rule) in rules.iter().enumerate() {
        if !M::ALLOWED.contains(&(rule.state, rule.operation)) {
            issues.push(StatefulIssue::NotAllowed {
                state: wire_name(&rule.state),
                operation: wire_name(&rule.operation),
            });
        }
        if !M::gesture_allowed(rule.operation, rule.gesture) {
            issues.push(StatefulIssue::GestureNotAllowed {
                operation: wire_name(&rule.operation),
                gesture: rule.gesture,
            });
        }
        if rules[..index]
            .iter()
            .any(|other| other.state == rule.state && other.gesture == rule.gesture)
        {
            issues.push(StatefulIssue::DuplicateRule {
                state: wire_name(&rule.state),
                gesture: rule.gesture,
            });
        }
        if !bound.has(rule.gesture) {
            issues.push(StatefulIssue::GestureNotBound {
                machine: M::NAME,
                gesture: rule.gesture,
            });
        }
    }
    issues
}

/// The operations of the work session a key can ask for (section 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkSessionOperation {
    /// T1: start a session.
    Start,
    /// T3: cancel the start workflow.
    CancelStart,
    /// T4/T8: check whether the session may end.
    Verify,
    /// T7: cancel the running verification.
    CancelVerification,
    /// T10: finish with the current approval. Only by a click.
    Finish,
    /// T9: drop the approval and go on working.
    ContinueWorking,
}

impl WorkSessionOperation {
    pub const ALL: [Self; 6] = [
        Self::Start,
        Self::CancelStart,
        Self::Verify,
        Self::CancelVerification,
        Self::Finish,
        Self::ContinueWorking,
    ];

    /// The transition the operation asks for.
    pub fn event(self) -> WorkSessionEvent {
        match self {
            Self::Start => WorkSessionEvent::Start,
            Self::CancelStart => WorkSessionEvent::StartFailed,
            Self::Verify => WorkSessionEvent::Verify,
            Self::CancelVerification => WorkSessionEvent::VerificationFailed,
            Self::Finish => WorkSessionEvent::Finish,
            Self::ContinueWorking => WorkSessionEvent::ApprovalInvalidated,
        }
    }
}

/// The work session as a key's state machine.
#[derive(Debug, Clone, Copy)]
pub struct WorkSessionMachine;

impl StateMachine for WorkSessionMachine {
    type State = WorkSessionState;
    type Operation = WorkSessionOperation;

    const ID: &'static str = "workSession";
    const NAME: &'static str = "Work Session";
    const STATES: &'static [WorkSessionState] = &WorkSessionState::ALL;
    const OPERATIONS: &'static [WorkSessionOperation] = &WorkSessionOperation::ALL;
    const ALLOWED: &'static [(WorkSessionState, WorkSessionOperation)] = {
        use WorkSessionOperation as O;
        use WorkSessionState as S;
        &[
            (S::Idle, O::Start),
            (S::Starting, O::CancelStart),
            (S::Working, O::Verify),
            (S::VerifyingEnd, O::CancelVerification),
            (S::ReadyToFinish, O::Finish),
            (S::ReadyToFinish, O::Verify),
            (S::ReadyToFinish, O::ContinueWorking),
        ]
    };
    /// Section 4: a click starts and finishes; in working both a click and
    /// a long press verify, and the agent lets only the one that fits the
    /// settings through (a click with a verification workflow, a long press
    /// without one, D12); a long press cancels what runs and leaves
    /// `readyToFinish`. Double click is not bound, so a click never waits.
    const DEFAULT_RULES: &'static [StatefulRule<WorkSessionState, WorkSessionOperation>] = {
        use Gesture as G;
        use WorkSessionOperation as O;
        use WorkSessionState as S;
        &[
            StatefulRule::new(S::Idle, G::Click, O::Start),
            StatefulRule::new(S::Starting, G::LongPress, O::CancelStart),
            StatefulRule::new(S::Working, G::Click, O::Verify),
            StatefulRule::new(S::Working, G::LongPress, O::Verify),
            StatefulRule::new(S::VerifyingEnd, G::LongPress, O::CancelVerification),
            StatefulRule::new(S::ReadyToFinish, G::Click, O::Finish),
            StatefulRule::new(S::ReadyToFinish, G::LongPress, O::ContinueWorking),
        ]
    };

    /// Finishing is a single click on the key only (D11, section 5.3).
    fn gesture_allowed(operation: WorkSessionOperation, gesture: Gesture) -> bool {
        operation != WorkSessionOperation::Finish || gesture == Gesture::Click
    }
}

/// What a stateful key follows, and its rules. Stored in a binding as
/// `{"type": "stateful", "machine": "workSession", "rules": [...]}`.
///
/// A key reacts to a gesture through the machine only if that gesture's
/// binding on the key is stateful; every stateful binding of one key
/// carries the same machine and rules.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "machine", rename_all = "camelCase")]
pub enum StatefulBinding {
    WorkSession {
        /// Empty: the machine's default rules.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        rules: Vec<StatefulRule<WorkSessionState, WorkSessionOperation>>,
    },
}

impl StatefulBinding {
    /// The work session with its default rules.
    pub fn work_session() -> Self {
        Self::WorkSession { rules: Vec::new() }
    }

    /// The machine's name in a binding.
    pub fn machine(&self) -> &'static str {
        match self {
            Self::WorkSession { .. } => WorkSessionMachine::ID,
        }
    }

    /// Shown on a key without a label of its own.
    pub fn name(&self) -> &'static str {
        match self {
            Self::WorkSession { .. } => WorkSessionMachine::NAME,
        }
    }

    /// The operation of the work session that `gesture` asks for when the
    /// key was pressed in `state`; `None`: the gesture does nothing there.
    pub fn work_session_operation(
        &self,
        state: WorkSessionState,
        gesture: Gesture,
    ) -> Option<WorkSessionOperation> {
        match self {
            Self::WorkSession { rules } => {
                operation_for::<WorkSessionMachine>(rules, state, gesture)
            }
        }
    }

    /// What is wrong with the rules, given the gestures of the key that
    /// are bound to this machine.
    pub fn issues(&self, bound: BoundGestures) -> Vec<StatefulIssue> {
        match self {
            Self::WorkSession { rules } => check::<WorkSessionMachine>(rules, bound),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::work_session::TRANSITIONS;

    use Gesture as G;
    use WorkSessionOperation as O;
    use WorkSessionState as S;

    fn bound(gestures: &[Gesture]) -> BoundGestures {
        BoundGestures {
            click: gestures.contains(&G::Click),
            double_click: gestures.contains(&G::DoubleClick),
            long_press: gestures.contains(&G::LongPress),
        }
    }

    fn rules(list: &[(S, G, O)]) -> StatefulBinding {
        StatefulBinding::WorkSession {
            rules: list
                .iter()
                .map(|(s, g, o)| StatefulRule::new(*s, *g, *o))
                .collect(),
        }
    }

    #[test]
    fn every_allowed_operation_is_a_transition_of_the_machine() {
        for (state, operation) in WorkSessionMachine::ALLOWED {
            assert!(
                TRANSITIONS
                    .iter()
                    .any(|(from, event, _)| from == state && *event == operation.event()),
                "{state:?} {operation:?}"
            );
            assert!(WorkSessionMachine::STATES.contains(state));
            assert!(WorkSessionMachine::OPERATIONS.contains(operation));
        }
        // Stopping takes no operation from a key.
        assert!(
            !WorkSessionMachine::ALLOWED
                .iter()
                .any(|(state, _)| *state == S::Stopping)
        );
    }

    #[test]
    fn the_default_rules_are_valid_and_follow_the_document() {
        let defaults = StatefulBinding::WorkSession {
            rules: WorkSessionMachine::DEFAULT_RULES.to_vec(),
        };
        assert_eq!(defaults.issues(bound(&[G::Click, G::LongPress])), []);
        let key = StatefulBinding::work_session();
        let op = |state, gesture| key.work_session_operation(state, gesture);
        assert_eq!(op(S::Idle, G::Click), Some(O::Start));
        assert_eq!(op(S::Idle, G::LongPress), None);
        assert_eq!(op(S::Starting, G::Click), None);
        assert_eq!(op(S::Starting, G::LongPress), Some(O::CancelStart));
        assert_eq!(op(S::Working, G::Click), Some(O::Verify));
        assert_eq!(op(S::Working, G::LongPress), Some(O::Verify));
        // Duplicate clicks while checking do nothing.
        assert_eq!(op(S::VerifyingEnd, G::Click), None);
        assert_eq!(
            op(S::VerifyingEnd, G::LongPress),
            Some(O::CancelVerification)
        );
        assert_eq!(op(S::ReadyToFinish, G::Click), Some(O::Finish));
        assert_eq!(op(S::ReadyToFinish, G::LongPress), Some(O::ContinueWorking));
        assert_eq!(op(S::Stopping, G::Click), None);
        // No default rule for double click: binding it would only delay
        // the click.
        for state in S::ALL {
            assert_eq!(op(state, G::DoubleClick), None);
        }
    }

    #[test]
    fn own_rules_replace_the_defaults() {
        let key = rules(&[
            (S::Idle, G::DoubleClick, O::Start),
            (S::ReadyToFinish, G::LongPress, O::Verify),
        ]);
        assert_eq!(key.issues(bound(&[G::DoubleClick, G::LongPress])), []);
        assert_eq!(key.work_session_operation(S::Idle, G::Click), None);
        assert_eq!(
            key.work_session_operation(S::Idle, G::DoubleClick),
            Some(O::Start)
        );
        assert_eq!(
            key.work_session_operation(S::ReadyToFinish, G::LongPress),
            Some(O::Verify)
        );
        assert_eq!(key.work_session_operation(S::ReadyToFinish, G::Click), None);
    }

    #[test]
    fn rules_are_refused_unless_the_machine_allows_them() {
        let key = rules(&[
            // Not a transition from idle.
            (S::Idle, G::Click, O::Finish),
            // Finishing only by a click, even where it is allowed.
            (S::ReadyToFinish, G::LongPress, O::Finish),
            (S::ReadyToFinish, G::DoubleClick, O::Finish),
            // Twice the same state and gesture.
            (S::Working, G::Click, O::Verify),
            (S::Working, G::Click, O::Verify),
            // Nothing can be asked for while stopping.
            (S::Stopping, G::Click, O::Start),
        ]);
        let issues = key.issues(bound(&[G::Click, G::LongPress]));
        assert_eq!(
            issues,
            [
                StatefulIssue::NotAllowed {
                    state: "idle".into(),
                    operation: "finish".into()
                },
                StatefulIssue::GestureNotAllowed {
                    operation: "finish".into(),
                    gesture: G::LongPress
                },
                StatefulIssue::GestureNotAllowed {
                    operation: "finish".into(),
                    gesture: G::DoubleClick
                },
                StatefulIssue::GestureNotBound {
                    machine: "Work Session",
                    gesture: G::DoubleClick
                },
                StatefulIssue::DuplicateRule {
                    state: "working".into(),
                    gesture: G::Click
                },
                StatefulIssue::NotAllowed {
                    state: "stopping".into(),
                    operation: "start".into()
                },
            ]
        );
        assert_eq!(
            issues[0].to_string(),
            "`finish` cannot be asked for in `idle`"
        );
    }

    #[test]
    fn bindings_serialize_as_documented_and_refuse_unknown_names() {
        let key = rules(&[(S::Idle, G::Click, O::Start)]);
        let json = serde_json::to_value(&key).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "machine": "workSession",
                "rules": [{"state": "idle", "gesture": "click", "operation": "start"}]
            })
        );
        assert_eq!(
            serde_json::from_value::<StatefulBinding>(json).unwrap(),
            key
        );
        // Without rules: the defaults, not stored.
        assert_eq!(
            serde_json::to_value(StatefulBinding::work_session()).unwrap(),
            serde_json::json!({"machine": "workSession"})
        );
        // No machine, state or operation outside the closed lists; no
        // expressions.
        for bad in [
            serde_json::json!({"machine": "custom"}),
            serde_json::json!({"machine": "workSession", "rules": [
                {"state": "paused", "gesture": "click", "operation": "start"}]}),
            serde_json::json!({"machine": "workSession", "rules": [
                {"state": "idle", "gesture": "click", "operation": "deleteEverything"}]}),
            serde_json::json!({"machine": "workSession", "rules": [
                {"state": "idle", "gesture": "click", "operation": "start", "when": "true"}]}),
        ] {
            assert!(
                serde_json::from_value::<StatefulBinding>(bad.clone()).is_err(),
                "{bad}"
            );
        }
        for operation in O::ALL {
            assert_eq!(
                serde_json::to_value(operation).unwrap(),
                wire_name(&operation)
            );
        }
    }

    #[test]
    fn the_frontend_mirrors_the_operations_and_the_machine() {
        let types = include_str!("../../../apps/desktop/src/lib/types.ts");
        for name in O::ALL
            .iter()
            .map(wire_name)
            .chain([WorkSessionMachine::ID.to_owned(), "stateful".to_owned()])
        {
            assert!(
                types.contains(&format!("\"{name}\"")),
                "types.ts lacks \"{name}\""
            );
        }
    }
}
