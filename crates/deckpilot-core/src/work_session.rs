//! Work sessions: a timer started, verified and finished from a key.
//!
//! Contract only (see `docs/gestures-and-work-session.md`): the state
//! machine as data, the verification workflow contract, the read-only
//! variables and the snapshot the agent publishes. Nothing uses them yet.
//!
//! Invariants the transition table guarantees (tested below):
//!
//! - The timer runs in [`WorkSessionState::Working`],
//!   [`WorkSessionState::VerifyingEnd`] and
//!   [`WorkSessionState::ReadyToFinish`].
//! - The only transition that stops a running timer is
//!   [`WorkSessionEvent::Finish`] from `ReadyToFinish`. A failed or negative
//!   verification, an expired approval, a disconnect or a restart never
//!   stops it.
//!
//! Which session and which verification an event belongs to is checked by
//! the agent before it applies the event (see the document); the table only
//! says which state changes exist.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::graph::{ValueKind, WorkflowOutput};
use crate::variables::VariableType;
use crate::{ProfileId, ProjectId, VerificationId, WorkSessionId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkSessionState {
    /// No session is active.
    Idle,
    /// The start workflow runs; the session is not committed yet.
    Starting,
    Working,
    /// The end-verification workflow runs.
    VerifyingEnd,
    /// The last verification allowed finishing; one click finishes.
    ReadyToFinish,
    /// Finishing was committed; the stop time is fixed.
    Stopping,
}

impl WorkSessionState {
    pub const ALL: [Self; 6] = [
        Self::Idle,
        Self::Starting,
        Self::Working,
        Self::VerifyingEnd,
        Self::ReadyToFinish,
        Self::Stopping,
    ];

    /// Whether time is being counted.
    pub fn timer_running(self) -> bool {
        matches!(
            self,
            Self::Working | Self::VerifyingEnd | Self::ReadyToFinish
        )
    }

    /// Whether a session exists (persisted as the active one).
    pub fn has_session(self) -> bool {
        !matches!(self, Self::Idle | Self::Starting)
    }

    /// The name used in storage, events and `system.work.session.state`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Starting => "starting",
            Self::Working => "working",
            Self::VerifyingEnd => "verifyingEnd",
            Self::ReadyToFinish => "readyToFinish",
            Self::Stopping => "stopping",
        }
    }
}

/// What can happen to a session. User operations and workflow results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkSessionEvent {
    /// User: start a session.
    Start,
    /// The start workflow succeeded (or none is configured).
    StartSucceeded,
    /// The start workflow failed, was cancelled or was interrupted.
    StartFailed,
    /// User: check whether the session may end.
    Verify,
    /// The verification returned `canFinish = true`.
    VerificationPassed,
    /// The verification returned `canFinish = false`.
    VerificationRejected,
    /// The verification failed, returned no valid `canFinish`, was cancelled
    /// or was interrupted.
    VerificationFailed,
    /// The approval no longer holds: it expired, the computer slept, the
    /// agent restarted, the settings changed, or the user chose to go on
    /// working.
    ApprovalInvalidated,
    /// User: finish, with the current approval.
    Finish,
    /// The stop was recorded completely.
    StopCompleted,
}

/// Every state change there is. Anything else is refused.
pub const TRANSITIONS: &[(WorkSessionState, WorkSessionEvent, WorkSessionState)] = {
    use WorkSessionEvent as E;
    use WorkSessionState as S;
    &[
        (S::Idle, E::Start, S::Starting),
        (S::Starting, E::StartSucceeded, S::Working),
        (S::Starting, E::StartFailed, S::Idle),
        (S::Working, E::Verify, S::VerifyingEnd),
        (S::VerifyingEnd, E::VerificationPassed, S::ReadyToFinish),
        (S::VerifyingEnd, E::VerificationRejected, S::Working),
        (S::VerifyingEnd, E::VerificationFailed, S::Working),
        (S::ReadyToFinish, E::Verify, S::VerifyingEnd),
        (S::ReadyToFinish, E::ApprovalInvalidated, S::Working),
        (S::ReadyToFinish, E::Finish, S::Stopping),
        (S::Stopping, E::StopCompleted, S::Idle),
    ]
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("a work session cannot go from {} on {event:?}", from.name())]
pub struct InvalidTransition {
    pub from: WorkSessionState,
    pub event: WorkSessionEvent,
}

impl WorkSessionState {
    /// The state after `event`, if [`TRANSITIONS`] has that change.
    pub fn next(self, event: WorkSessionEvent) -> Result<Self, InvalidTransition> {
        TRANSITIONS
            .iter()
            .find(|(from, on, _)| *from == self && *on == event)
            .map(|(_, _, to)| *to)
            .ok_or(InvalidTransition { from: self, event })
    }
}

/// One stretch of counted time, in wall-clock milliseconds since the Unix
/// epoch. The open interval of a running session has no end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkInterval {
    pub started_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at_ms: Option<i64>,
}

/// Counted time of `intervals` at `now_ms`. Computed, never stored: an
/// open interval counts up to `now_ms`, and an interval whose end lies
/// before its start (the clock was set back) counts as zero.
pub fn elapsed_ms(intervals: &[WorkInterval], now_ms: i64) -> u64 {
    intervals
        .iter()
        .map(|interval| {
            let end = interval.ended_at_ms.unwrap_or(now_ms);
            u64::try_from(end.saturating_sub(interval.started_at_ms)).unwrap_or(0)
        })
        .fold(0, u64::saturating_add)
}

/// The output a verification workflow must declare, as a boolean.
pub const CAN_FINISH_OUTPUT: &str = "canFinish";
/// An optional text output shown to the user with the result.
pub const DETAILS_OUTPUT: &str = "details";
/// Longest `details` kept, in characters.
pub const MAX_DETAILS: usize = 200;

/// Why a workflow cannot be the verification workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Error)]
#[serde(rename_all = "camelCase")]
pub enum VerificationContractIssue {
    #[error("the workflow does not declare the output `canFinish`")]
    MissingCanFinish,
    #[error("the output `canFinish` must be a boolean")]
    CanFinishNotBoolean,
    #[error("the output `details` must be text")]
    DetailsNotText,
}

/// Checks a workflow's declared outputs against the verification contract.
pub fn verification_contract(declared: &[WorkflowOutput]) -> Result<(), VerificationContractIssue> {
    let can_finish = declared
        .iter()
        .find(|output| output.key == CAN_FINISH_OUTPUT)
        .ok_or(VerificationContractIssue::MissingCanFinish)?;
    if can_finish.kind != ValueKind::Boolean {
        return Err(VerificationContractIssue::CanFinishNotBoolean);
    }
    if declared
        .iter()
        .any(|output| output.key == DETAILS_OUTPUT && output.kind != ValueKind::Text)
    {
        return Err(VerificationContractIssue::DetailsNotText);
    }
    Ok(())
}

/// The typed result of a verification run that succeeded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationOutcome {
    pub can_finish: bool,
    /// Shown to the user; absent when not set or declared sensitive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum VerificationError {
    #[error(transparent)]
    Contract(#[from] VerificationContractIssue),
    #[error("the verification workflow did not set `canFinish`")]
    NoCanFinish,
    #[error("`canFinish` must be true or false")]
    InvalidCanFinish,
}

impl VerificationOutcome {
    /// Reads the outcome from the declared outputs of a run that
    /// succeeded. `canFinish` must be exactly `true` or `false`; anything
    /// else is an error, never a "no".
    pub fn from_outputs(
        declared: &[WorkflowOutput],
        values: &BTreeMap<String, String>,
    ) -> Result<Self, VerificationError> {
        verification_contract(declared)?;
        let can_finish = match values.get(CAN_FINISH_OUTPUT).map(String::as_str) {
            None => return Err(VerificationError::NoCanFinish),
            Some("true") => true,
            Some("false") => false,
            Some(_) => return Err(VerificationError::InvalidCanFinish),
        };
        let shown = declared
            .iter()
            .any(|output| output.key == DETAILS_OUTPUT && !output.sensitive);
        let details = values
            .get(DETAILS_OUTPUT)
            .filter(|_| shown)
            .map(|text| {
                text.chars()
                    .filter(|c| !c.is_control())
                    .take(MAX_DETAILS)
                    .collect::<String>()
                    .trim()
                    .to_owned()
            })
            .filter(|text| !text.is_empty());
        Ok(Self {
            can_finish,
            details,
        })
    }
}

/// The read-only variables of the active work session.
///
/// Each is a [`crate::variables::SystemVariable`] (`system.work.session.…`,
/// D5). Only the agent knows the session: its resolver supplies the values
/// from the live snapshot, and every other resolver reports
/// `missing(noValue)`, as for no session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkSessionVariable {
    Id,
    State,
    StartedAt,
    ElapsedSeconds,
    ProjectId,
    CanFinish,
}

impl WorkSessionVariable {
    pub const ALL: [Self; 6] = [
        Self::Id,
        Self::State,
        Self::StartedAt,
        Self::ElapsedSeconds,
        Self::ProjectId,
        Self::CanFinish,
    ];

    pub fn path(self) -> &'static str {
        match self {
            Self::Id => "system.work.session.id",
            Self::State => "system.work.session.state",
            Self::StartedAt => "system.work.session.startedAt",
            Self::ElapsedSeconds => "system.work.session.elapsedSeconds",
            Self::ProjectId => "system.work.session.projectId",
            Self::CanFinish => "system.work.session.canFinish",
        }
    }

    pub fn value_type(self) -> VariableType {
        match self {
            Self::Id | Self::State | Self::StartedAt => VariableType::String,
            Self::ElapsedSeconds => VariableType::Integer,
            Self::ProjectId => VariableType::ProjectId,
            Self::CanFinish => VariableType::Boolean,
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Id => "Id of the active work session",
            Self::State => "State of the work session (idle, working, …)",
            Self::StartedAt => "When the work session started (RFC 3339, UTC)",
            Self::ElapsedSeconds => "Counted time of the work session, in seconds",
            Self::ProjectId => "Project the work session was started for",
            Self::CanFinish => "Result of the last end verification",
        }
    }
}

/// Why the last operation of a session did not do what was asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkSessionProblem {
    StartFailed,
    /// The agent stopped while the start workflow ran.
    StartInterrupted,
    VerificationFailed,
    /// The agent stopped while the verification ran.
    VerificationInterrupted,
    /// The verification answered `canFinish = false`.
    NotReady,
    ApprovalExpired,
    /// The computer slept or the agent restarted after the approval.
    ApprovalInvalidated,
    /// A gap without a running agent or a jump of the clock needs a
    /// decision whether it counts.
    NeedsReview,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkSessionNotice {
    pub problem: WorkSessionProblem,
    /// Safe to show: never a value of a sensitive output or variable.
    pub message: String,
    pub at_ms: i64,
}

/// The last verification of a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationRecord {
    pub id: VerificationId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<VerificationOutcome>,
    pub at_ms: i64,
}

/// What the agent publishes about the work session; the app and the keys
/// draw elapsed time from `elapsed_ms` at `as_of_ms` without asking again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkSessionSnapshot {
    pub state: WorkSessionState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<WorkSessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<ProfileId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<ProjectId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at_ms: Option<i64>,
    pub elapsed_ms: u64,
    pub as_of_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<VerificationRecord>,
    /// The verification a click in `readyToFinish` finishes with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<VerificationId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notices: Vec<WorkSessionNotice>,
}

impl WorkSessionSnapshot {
    pub fn idle(now_ms: i64) -> Self {
        Self {
            state: WorkSessionState::Idle,
            session: None,
            profile: None,
            project: None,
            started_at_ms: None,
            elapsed_ms: 0,
            as_of_ms: now_ms,
            verification: None,
            approval: None,
            notices: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_has_no_duplicates_and_reaches_every_state() {
        for (index, (from, event, _)) in TRANSITIONS.iter().enumerate() {
            assert!(
                !TRANSITIONS[..index]
                    .iter()
                    .any(|(f, e, _)| f == from && e == event),
                "{from:?} on {event:?} is listed twice"
            );
        }
        for state in WorkSessionState::ALL {
            assert!(
                state == WorkSessionState::Idle
                    || TRANSITIONS.iter().any(|(_, _, to)| *to == state),
                "{state:?} is unreachable"
            );
            assert!(
                TRANSITIONS.iter().any(|(from, _, _)| *from == state),
                "{state:?} is a dead end"
            );
        }
    }

    #[test]
    fn only_finish_from_ready_stops_a_running_timer() {
        for (from, event, to) in TRANSITIONS {
            if from.timer_running() && !to.timer_running() {
                assert_eq!(
                    (*from, *event),
                    (WorkSessionState::ReadyToFinish, WorkSessionEvent::Finish)
                );
            }
        }
    }

    #[test]
    fn verification_never_ends_the_session() {
        use WorkSessionEvent as E;
        use WorkSessionState as S;
        assert_eq!(
            S::VerifyingEnd.next(E::VerificationRejected),
            Ok(S::Working)
        );
        assert_eq!(S::VerifyingEnd.next(E::VerificationFailed), Ok(S::Working));
        assert_eq!(
            S::VerifyingEnd.next(E::VerificationPassed),
            Ok(S::ReadyToFinish)
        );
        for state in [
            S::Idle,
            S::Starting,
            S::Working,
            S::VerifyingEnd,
            S::Stopping,
        ] {
            assert!(state.next(E::Finish).is_err(), "finish from {state:?}");
        }
        assert_eq!(
            S::Working.next(E::VerificationPassed),
            Err(InvalidTransition {
                from: S::Working,
                event: E::VerificationPassed
            })
        );
    }

    #[test]
    fn the_full_cycle() {
        use WorkSessionEvent as E;
        let mut state = WorkSessionState::Idle;
        for event in [
            E::Start,
            E::StartSucceeded,
            E::Verify,
            E::VerificationRejected,
            E::Verify,
            E::VerificationPassed,
            E::ApprovalInvalidated,
            E::Verify,
            E::VerificationPassed,
            E::Finish,
            E::StopCompleted,
        ] {
            state = state.next(event).unwrap();
        }
        assert_eq!(state, WorkSessionState::Idle);
        assert_eq!(
            WorkSessionState::Starting.next(E::StartFailed),
            Ok(WorkSessionState::Idle)
        );
    }

    #[test]
    fn elapsed_time_comes_from_intervals() {
        let intervals = [
            WorkInterval {
                started_at_ms: 1_000,
                ended_at_ms: Some(61_000),
            },
            // The clock was set back: counts as nothing.
            WorkInterval {
                started_at_ms: 100_000,
                ended_at_ms: Some(90_000),
            },
            WorkInterval {
                started_at_ms: 200_000,
                ended_at_ms: None,
            },
        ];
        assert_eq!(elapsed_ms(&intervals, 230_000), 60_000 + 30_000);
        // An open interval that starts after "now" counts as nothing.
        assert_eq!(elapsed_ms(&intervals, 150_000), 60_000);
        assert_eq!(elapsed_ms(&[], 5), 0);
    }

    fn declared(kind: ValueKind) -> Vec<WorkflowOutput> {
        vec![
            WorkflowOutput::new(CAN_FINISH_OUTPUT, kind),
            WorkflowOutput::new(DETAILS_OUTPUT, ValueKind::Text),
        ]
    }

    fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn the_verification_contract_is_checked() {
        assert_eq!(verification_contract(&declared(ValueKind::Boolean)), Ok(()));
        assert_eq!(
            verification_contract(&declared(ValueKind::Text)),
            Err(VerificationContractIssue::CanFinishNotBoolean)
        );
        assert_eq!(
            verification_contract(&[]),
            Err(VerificationContractIssue::MissingCanFinish)
        );
        let details_number = vec![
            WorkflowOutput::new(CAN_FINISH_OUTPUT, ValueKind::Boolean),
            WorkflowOutput::new(DETAILS_OUTPUT, ValueKind::Integer),
        ];
        assert_eq!(
            verification_contract(&details_number),
            Err(VerificationContractIssue::DetailsNotText)
        );
    }

    #[test]
    fn can_finish_is_read_strictly() {
        let declared = declared(ValueKind::Boolean);
        assert_eq!(
            VerificationOutcome::from_outputs(
                &declared,
                &values(&[("canFinish", "false"), ("details", " 2 tests fail\n ")])
            ),
            Ok(VerificationOutcome {
                can_finish: false,
                details: Some("2 tests fail".into())
            })
        );
        assert_eq!(
            VerificationOutcome::from_outputs(&declared, &values(&[("canFinish", "true")])),
            Ok(VerificationOutcome {
                can_finish: true,
                details: None
            })
        );
        for bad in ["yes", "True", "1", ""] {
            assert_eq!(
                VerificationOutcome::from_outputs(&declared, &values(&[("canFinish", bad)])),
                Err(VerificationError::InvalidCanFinish),
                "{bad:?}"
            );
        }
        assert_eq!(
            VerificationOutcome::from_outputs(&declared, &values(&[])),
            Err(VerificationError::NoCanFinish)
        );
    }

    #[test]
    fn sensitive_details_are_not_shown() {
        let mut declared = declared(ValueKind::Boolean);
        declared[1].sensitive = true;
        let outcome = VerificationOutcome::from_outputs(
            &declared,
            &values(&[("canFinish", "true"), ("details", "token=abc")]),
        )
        .unwrap();
        assert_eq!(outcome.details, None);
    }

    #[test]
    fn variables_have_unique_paths_under_system() {
        let mut paths: Vec<_> = WorkSessionVariable::ALL
            .map(WorkSessionVariable::path)
            .to_vec();
        assert!(paths.iter().all(|p| p.starts_with("system.work.session.")));
        paths.sort_unstable();
        paths.dedup();
        assert_eq!(paths.len(), WorkSessionVariable::ALL.len());
        assert_eq!(
            WorkSessionVariable::CanFinish.value_type(),
            VariableType::Boolean
        );
    }

    #[test]
    fn snapshots_serialize_in_camel_case() {
        let json = serde_json::to_value(WorkSessionSnapshot::idle(42)).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"state": "idle", "elapsedMs": 0, "asOfMs": 42})
        );
        for state in WorkSessionState::ALL {
            assert_eq!(serde_json::to_value(state).unwrap(), state.name());
        }
    }

    /// The app's mirror of these types; every serialized name must be there.
    const FRONTEND_TYPES: &str = include_str!("../../../apps/desktop/src/lib/types.ts");

    #[test]
    fn the_frontend_mirrors_every_variant() {
        let names = WorkSessionState::ALL
            .map(|s| s.name().to_owned())
            .into_iter()
            .chain(
                [
                    WorkSessionProblem::StartFailed,
                    WorkSessionProblem::StartInterrupted,
                    WorkSessionProblem::VerificationFailed,
                    WorkSessionProblem::VerificationInterrupted,
                    WorkSessionProblem::NotReady,
                    WorkSessionProblem::ApprovalExpired,
                    WorkSessionProblem::ApprovalInvalidated,
                    WorkSessionProblem::NeedsReview,
                ]
                .map(|p| {
                    serde_json::to_value(p)
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .to_owned()
                }),
            )
            .chain(crate::gesture::Gesture::ALL.map(|g| {
                serde_json::to_value(g)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_owned()
            }))
            .chain(WorkSessionVariable::ALL.map(|v| v.path().to_owned()));
        for name in names {
            assert!(
                FRONTEND_TYPES.contains(&format!("\"{name}\"")),
                "types.ts lacks \"{name}\""
            );
        }
    }
}
