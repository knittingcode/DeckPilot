//! The variables the work session module owns: the state and the timer of
//! the active session (docs/gestures-and-work-session.md §8.1).
//!
//! `workSession.…` used to be the core system variables
//! `system.work.session.…`; those ids stay readable as aliases
//! (`deckpilot_core::variables::ALIASES`). Only the agent knows the
//! session: values are computed from its live snapshot
//! ([`deckpilot_core::variables::LiveState`]) at the moment of the
//! resolution, so elapsed time is the counted time of the snapshot plus the
//! time since it was taken while the timer runs. Without a session every
//! variable is `missing(noValue)`.

use deckpilot_core::variables::{
    DeclaredScope, MissingReason, Requirement, Resolution, ValueSource, VariableDeclaration,
    VariableEnvironment, VariableSource, VariableValue,
};
use deckpilot_core::work_session::{WorkSessionSnapshot, WorkSessionVariable};

/// The canonical id of `variable`, in the module's namespace.
pub fn canonical(variable: WorkSessionVariable) -> &'static str {
    match variable {
        WorkSessionVariable::Id => "workSession.id",
        WorkSessionVariable::State => "workSession.state",
        WorkSessionVariable::StartedAt => "workSession.startedAt",
        WorkSessionVariable::ElapsedSeconds => "workSession.elapsedSeconds",
        WorkSessionVariable::ProjectId => "workSession.projectId",
        WorkSessionVariable::CanFinish => "workSession.canFinish",
    }
}

fn name(variable: WorkSessionVariable) -> &'static str {
    match variable {
        WorkSessionVariable::Id => "Work session id",
        WorkSessionVariable::State => "Work session state",
        WorkSessionVariable::StartedAt => "Work session start",
        WorkSessionVariable::ElapsedSeconds => "Work session time",
        WorkSessionVariable::ProjectId => "Work session project",
        WorkSessionVariable::CanFinish => "Work session can finish",
    }
}

/// The declarations of the module's variables.
pub fn declarations() -> Vec<VariableDeclaration> {
    WorkSessionVariable::ALL
        .into_iter()
        .map(|variable| {
            VariableDeclaration::new(
                canonical(variable),
                name(variable),
                variable.value_type(),
                DeclaredScope::WorkSession,
            )
            .describe(variable.description())
            .requires(Requirement::WorkSession)
        })
        .collect()
}

/// Resolves the module's variables from the agent's live session.
#[derive(Debug, Clone, Copy, Default)]
pub struct WorkSessionVariables;

impl VariableSource for WorkSessionVariables {
    fn resolve(
        &self,
        declaration: &VariableDeclaration,
        environment: &VariableEnvironment<'_>,
    ) -> Resolution {
        let Some(variable) = WorkSessionVariable::ALL
            .into_iter()
            .find(|variable| canonical(*variable) == declaration.id.as_str())
        else {
            return Resolution::Missing(MissingReason::UnknownVariable);
        };
        match environment.live.work_session() {
            Some(snapshot) => value(variable, &snapshot, environment.live.now_ms()),
            None => Resolution::Missing(MissingReason::NoValue),
        }
    }
}

/// The value of a work session variable from `snapshot` at `now_ms`.
/// Without a session every one is `missing(noValue)` (§8.1).
pub fn value(
    variable: WorkSessionVariable,
    snapshot: &WorkSessionSnapshot,
    now_ms: i64,
) -> Resolution {
    let Some(session) = snapshot.session else {
        return Resolution::Missing(MissingReason::NoValue);
    };
    let value = match variable {
        WorkSessionVariable::Id => Some(VariableValue::String(session.to_string())),
        WorkSessionVariable::State => Some(VariableValue::String(snapshot.state.name().to_owned())),
        WorkSessionVariable::StartedAt => snapshot
            .started_at_ms
            .and_then(rfc3339)
            .map(VariableValue::String),
        WorkSessionVariable::ElapsedSeconds => {
            let mut elapsed = snapshot.elapsed_ms;
            if snapshot.state.timer_running() {
                // A wall clock that went back adds nothing.
                let since = u64::try_from(now_ms - snapshot.as_of_ms).unwrap_or(0);
                elapsed = elapsed.saturating_add(since);
            }
            Some(VariableValue::Integer(
                i64::try_from(elapsed / 1000).unwrap_or(i64::MAX),
            ))
        }
        WorkSessionVariable::ProjectId => match snapshot.project {
            Some(project) => Some(VariableValue::ProjectId(project)),
            None => return Resolution::Missing(MissingReason::NoProject),
        },
        WorkSessionVariable::CanFinish => snapshot
            .verification
            .as_ref()
            .and_then(|v| v.outcome.as_ref())
            .map(|outcome| VariableValue::Boolean(outcome.can_finish)),
    };
    // Nothing of a session is sensitive; verification details are not a
    // variable.
    value.map_or(Resolution::Missing(MissingReason::NoValue), |value| {
        Resolution::Value {
            value,
            source: ValueSource::System,
            sensitive: false,
        }
    })
}

/// RFC 3339 in UTC, to the second.
fn rfc3339(ms: i64) -> Option<String> {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|time| time.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}
