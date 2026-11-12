//! Work sessions (schema 9): the persisted state of the work session the
//! agent runs, its counted intervals, reviews of uncertain time, the audit
//! log of its transitions, and the per-profile settings (see
//! `docs/gestures-and-work-session.md` §5–§6).
//!
//! The store applies the transitions of [`deckpilot_core::work_session`]
//! and nothing else. Each transition is one `IMMEDIATE` transaction that
//! reads the session, checks it is the expected session **in the expected
//! state** (and, where it matters, for the expected verification or
//! approval), updates it with a compare-and-set on `(id, state)`, and
//! appends to `work_session_transitions`. A request that does not match
//! changes nothing and is answered with a [`Refusal`]; a request that was
//! already applied (a duplicated signal) is answered with
//! [`Transition::Repeated`]. So an approval or a verification result of an
//! older verification, or of an older session, can never move the current
//! one.
//!
//! Elapsed time is never stored: it is the sum of the persisted intervals
//! ([`deckpilot_core::work_session::elapsed_ms`]). The heartbeat only
//! moves `last_alive_at`.
//!
//! Timestamps are wall-clock milliseconds since the Unix epoch, supplied by
//! the caller (the agent's injectable clock).

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use deckpilot_core::work_session::{
    MAX_DETAILS, VerificationOutcome, WorkInterval, WorkSessionEvent, WorkSessionNotice,
    WorkSessionProblem, WorkSessionState, elapsed_ms,
};
use deckpilot_core::{
    ExecutionId, ProfileId, ProjectId, VerificationId, WorkSessionId, WorkflowId,
};

use crate::sqlite::{constraint_violation, parse_id};
use crate::{Result, StorageError};

/// Schema migration 9. `active` is 1 exactly while a session is not
/// finished (`stopped`) or abandoned (`aborted`), and unique: there is at
/// most one active session per database (decision D7).
pub(crate) const SCHEMA: &str = r"
    CREATE TABLE work_session_settings (
        profile_id            TEXT PRIMARY KEY NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
        start_workflow        TEXT,
        verification_workflow TEXT,
        approval_ttl_seconds  INTEGER NOT NULL DEFAULT 600
                              CHECK (approval_ttl_seconds BETWEEN 60 AND 86400),
        review_gap_seconds    INTEGER NOT NULL DEFAULT 900
                              CHECK (review_gap_seconds BETWEEN 60 AND 86400),
        elapsed_format        TEXT NOT NULL DEFAULT 'minutes'
                              CHECK (elapsed_format IN ('minutes', 'seconds')),
        updated_at            INTEGER NOT NULL
    );
    CREATE TABLE work_sessions (
        id                     TEXT PRIMARY KEY NOT NULL,
        profile_id             TEXT NOT NULL,
        project_id             TEXT,
        state                  TEXT NOT NULL
                               CHECK (state IN ('starting', 'working', 'verifyingEnd',
                                   'readyToFinish', 'stopping', 'stopped', 'aborted')),
        active                 INTEGER UNIQUE CHECK (active IS NULL OR active = 1),
        requested_at           INTEGER NOT NULL,
        started_at             INTEGER,
        stopped_at             INTEGER,
        start_workflow         TEXT,
        verification_workflow  TEXT,
        start_execution        TEXT,
        verification_id        TEXT,
        verification_execution TEXT,
        can_finish             INTEGER CHECK (can_finish IN (0, 1)),
        verification_details   TEXT CHECK (length(verification_details) <= 200),
        verified_at            INTEGER,
        approval_id            TEXT,
        approved_at            INTEGER,
        approval_boot          TEXT,
        last_alive_at          INTEGER NOT NULL,
        problem                TEXT,
        problem_message        TEXT,
        problem_at             INTEGER,
        updated_at             INTEGER NOT NULL,
        CHECK ((active IS NULL) = (state IN ('stopped', 'aborted')))
    );
    CREATE INDEX work_sessions_started ON work_sessions (profile_id, requested_at);
    CREATE INDEX work_sessions_finished ON work_sessions (requested_at) WHERE active IS NULL;
    CREATE TABLE work_session_intervals (
        session_id TEXT NOT NULL REFERENCES work_sessions (id) ON DELETE CASCADE,
        seq        INTEGER NOT NULL,
        started_at INTEGER NOT NULL,
        ended_at   INTEGER,
        end_reason TEXT CHECK (end_reason IN ('stopped', 'gapExcluded')),
        PRIMARY KEY (session_id, seq)
    );
    CREATE UNIQUE INDEX work_session_open_interval
        ON work_session_intervals (session_id) WHERE ended_at IS NULL;
    CREATE TABLE work_session_reviews (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        session_id  TEXT NOT NULL REFERENCES work_sessions (id) ON DELETE CASCADE,
        kind        TEXT NOT NULL CHECK (kind IN ('agentDown', 'sleep', 'clockBack', 'clockForward')),
        from_at     INTEGER NOT NULL,
        to_at       INTEGER NOT NULL,
        resolution  TEXT CHECK (resolution IN ('counted', 'excluded')),
        resolved_at INTEGER
    );
    CREATE INDEX work_session_reviews_session ON work_session_reviews (session_id);
    CREATE TABLE work_session_transitions (
        session_id TEXT NOT NULL REFERENCES work_sessions (id) ON DELETE CASCADE,
        seq        INTEGER NOT NULL,
        at         INTEGER NOT NULL,
        from_state TEXT NOT NULL,
        event      TEXT NOT NULL,
        to_state   TEXT NOT NULL,
        PRIMARY KEY (session_id, seq)
    );
";

/// How long an approval to finish lasts by default (decision D9).
pub const DEFAULT_APPROVAL_TTL_SECONDS: u32 = 600;
/// Gaps longer than this are flagged for review by default (decision D8).
pub const DEFAULT_REVIEW_GAP_SECONDS: u32 = 900;
/// Bounds of both settings, in seconds.
pub const SETTING_SECONDS: std::ops::RangeInclusive<u32> = 60..=86_400;
/// Finished or aborted sessions kept; older ones are removed.
pub const KEPT_SESSIONS: usize = 1000;

/// How elapsed time is shown on keys (decision D4).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ElapsedFormat {
    /// `h:mm`, redrawn once per minute.
    #[default]
    Minutes,
    /// With seconds, redrawn every second.
    Seconds,
}

impl ElapsedFormat {
    fn as_str(self) -> &'static str {
        match self {
            Self::Minutes => "minutes",
            Self::Seconds => "seconds",
        }
    }
}

/// Work session settings of one profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkSessionSettings {
    pub profile: ProfileId,
    pub start_workflow: Option<WorkflowId>,
    pub verification_workflow: Option<WorkflowId>,
    pub approval_ttl_seconds: u32,
    pub review_gap_seconds: u32,
    pub elapsed_format: ElapsedFormat,
    /// When the settings were last saved (ms); `None` for defaults never
    /// saved. Ignored when saving.
    #[serde(default)]
    pub updated_at_ms: Option<i64>,
}

impl WorkSessionSettings {
    pub fn defaults(profile: ProfileId) -> Self {
        Self {
            profile,
            start_workflow: None,
            verification_workflow: None,
            approval_ttl_seconds: DEFAULT_APPROVAL_TTL_SECONDS,
            review_gap_seconds: DEFAULT_REVIEW_GAP_SECONDS,
            elapsed_format: ElapsedFormat::Minutes,
            updated_at_ms: None,
        }
    }

    pub fn approval_ttl_ms(&self) -> i64 {
        i64::from(self.approval_ttl_seconds) * 1000
    }

    pub fn review_gap_ms(&self) -> i64 {
        i64::from(self.review_gap_seconds) * 1000
    }

    /// Whether an approval given at `approved_at_ms` still holds at
    /// `now_ms`: not older than the lifetime, and not from the future (the
    /// clock was set back).
    pub fn approval_holds(&self, approved_at_ms: i64, now_ms: i64) -> bool {
        approved_at_ms <= now_ms && now_ms - approved_at_ms <= self.approval_ttl_ms()
    }
}

/// The persisted state of a session. `Stopped` and `Aborted` sessions are
/// history; the others are the active session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionStatus {
    Starting,
    Working,
    VerifyingEnd,
    ReadyToFinish,
    Stopping,
    /// Finished (T11).
    Stopped,
    /// The start failed or was interrupted (T3); no time was counted.
    Aborted,
}

impl SessionStatus {
    pub const ALL: [Self; 7] = [
        Self::Starting,
        Self::Working,
        Self::VerifyingEnd,
        Self::ReadyToFinish,
        Self::Stopping,
        Self::Stopped,
        Self::Aborted,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Working => "working",
            Self::VerifyingEnd => "verifyingEnd",
            Self::ReadyToFinish => "readyToFinish",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
            Self::Aborted => "aborted",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|status| status.as_str() == text)
    }

    /// The state of the work session machine this status stands for.
    pub fn state(self) -> WorkSessionState {
        match self {
            Self::Starting => WorkSessionState::Starting,
            Self::Working => WorkSessionState::Working,
            Self::VerifyingEnd => WorkSessionState::VerifyingEnd,
            Self::ReadyToFinish => WorkSessionState::ReadyToFinish,
            Self::Stopping => WorkSessionState::Stopping,
            Self::Stopped | Self::Aborted => WorkSessionState::Idle,
        }
    }

    pub fn is_active(self) -> bool {
        !matches!(self, Self::Stopped | Self::Aborted)
    }

    /// The status after `event`, following
    /// [`deckpilot_core::work_session::TRANSITIONS`]. Ending in idle is
    /// `Aborted` after a failed start and `Stopped` after a stop.
    fn next(self, event: WorkSessionEvent) -> Option<Self> {
        let to = self.state().next(event).ok()?;
        Some(match to {
            WorkSessionState::Idle if event == WorkSessionEvent::StartFailed => Self::Aborted,
            WorkSessionState::Idle => Self::Stopped,
            WorkSessionState::Starting => Self::Starting,
            WorkSessionState::Working => Self::Working,
            WorkSessionState::VerifyingEnd => Self::VerifyingEnd,
            WorkSessionState::ReadyToFinish => Self::ReadyToFinish,
            WorkSessionState::Stopping => Self::Stopping,
        })
    }
}

/// One stored session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkSessionRecord {
    pub id: WorkSessionId,
    pub profile: ProfileId,
    pub project: Option<ProjectId>,
    pub status: SessionStatus,
    pub requested_at_ms: i64,
    /// Entering working (T2); time counts from here (D10).
    pub started_at_ms: Option<i64>,
    /// The fixed stop time (T10).
    pub stopped_at_ms: Option<i64>,
    pub start_workflow: Option<WorkflowId>,
    pub verification_workflow: Option<WorkflowId>,
    pub start_execution: Option<ExecutionId>,
    /// The current (or last) verification.
    pub verification: Option<VerificationId>,
    pub verification_execution: Option<ExecutionId>,
    /// The outcome of that verification; `None` while it runs or when it
    /// failed.
    pub can_finish: Option<bool>,
    pub verification_details: Option<String>,
    /// When the result of that verification arrived.
    pub verified_at_ms: Option<i64>,
    /// The verification a finish click finishes with; kept after the stop,
    /// so a repeated click is recognised.
    pub approval: Option<VerificationId>,
    pub approved_at_ms: Option<i64>,
    /// The agent instance that received the approval.
    pub approval_boot: Option<String>,
    pub last_alive_at_ms: i64,
    pub problem: Option<WorkSessionNotice>,
    pub updated_at_ms: i64,
}

/// Why a stored interval ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IntervalEnd {
    Stopped,
    /// Closed where an excluded gap begins.
    GapExcluded,
}

impl IntervalEnd {
    fn as_str(self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::GapExcluded => "gapExcluded",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "stopped" => Some(Self::Stopped),
            "gapExcluded" => Some(Self::GapExcluded),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredInterval {
    pub seq: u32,
    pub interval: WorkInterval,
    pub end_reason: Option<IntervalEnd>,
}

/// What made a stretch of time uncertain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GapKind {
    /// No agent was running (`now - last_alive_at` after a restart).
    AgentDown,
    Sleep,
    /// The wall clock went backwards: `to` lies before `from`.
    ClockBack,
    ClockForward,
}

impl GapKind {
    pub const ALL: [Self; 4] = [
        Self::AgentDown,
        Self::Sleep,
        Self::ClockBack,
        Self::ClockForward,
    ];

    fn as_str(self) -> &'static str {
        match self {
            Self::AgentDown => "agentDown",
            Self::Sleep => "sleep",
            Self::ClockBack => "clockBack",
            Self::ClockForward => "clockForward",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == text)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReviewResolution {
    /// The gap stays counted (what happens anyway while unresolved).
    Counted,
    /// The gap is cut out of the intervals.
    Excluded,
}

impl ReviewResolution {
    fn as_str(self) -> &'static str {
        match self {
            Self::Counted => "counted",
            Self::Excluded => "excluded",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "counted" => Some(Self::Counted),
            "excluded" => Some(Self::Excluded),
            _ => None,
        }
    }
}

/// A flagged gap (decision D8): counted unless the user excludes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkSessionReview {
    pub id: i64,
    pub session: WorkSessionId,
    pub kind: GapKind,
    pub from_ms: i64,
    pub to_ms: i64,
    /// `None` while open.
    pub resolution: Option<ReviewResolution>,
    pub resolved_at_ms: Option<i64>,
}

/// One entry of the audit log. States are [`SessionStatus`] names, plus
/// `idle` before the session existed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransitionRecord {
    pub seq: u32,
    pub at_ms: i64,
    pub from: String,
    pub event: String,
    pub to: String,
}

/// The answer to a transition request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "value")]
pub enum Transition {
    /// Applied and committed; the session as it is now.
    Applied(WorkSessionRecord),
    /// The same request was applied before; nothing changed.
    Repeated(WorkSessionRecord),
    /// Not applied; nothing changed.
    Refused(Refusal),
}

impl Transition {
    pub fn applied(&self) -> Option<&WorkSessionRecord> {
        match self {
            Self::Applied(record) => Some(record),
            _ => None,
        }
    }
}

/// Why a transition request was refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "reason")]
pub enum Refusal {
    UnknownSession,
    /// Another session is active (decision D7).
    AnotherSessionActive {
        active: WorkSessionId,
    },
    /// The session is not in the state the request expects.
    WrongState {
        status: SessionStatus,
    },
    /// The request names another verification than the session's current
    /// one (an older one, or one of another session).
    StaleVerification,
    /// The approval is older than its lifetime, or from the future.
    ApprovalExpired,
    /// The approval was given to an earlier agent instance.
    ApprovalFromAnotherBoot,
}

/// T1: what a new session starts with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewWorkSession {
    pub id: WorkSessionId,
    pub profile: ProfileId,
    pub project: Option<ProjectId>,
    /// As configured when the session is requested.
    pub start_workflow: Option<WorkflowId>,
    pub at_ms: i64,
}

/// The result of a verification run, for T5–T7.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationResult {
    /// The run succeeded with a valid `canFinish` (T5 or T6).
    Outcome(VerificationOutcome),
    /// The run failed, was cancelled, gave no valid `canFinish`, or was
    /// interrupted (T7). The notice is `verificationFailed` or
    /// `verificationInterrupted`.
    Failed(WorkSessionNotice),
}

/// Which history to list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryQuery {
    /// Only sessions of this profile; all when `None`.
    pub profile: Option<ProfileId>,
    /// Only sessions requested before this time (for paging).
    pub before_ms: Option<i64>,
    pub limit: usize,
}

/// A session of the history with its counted time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkSessionSummary {
    pub session: WorkSessionRecord,
    pub elapsed_ms: u64,
    pub open_reviews: u32,
}

const COLUMNS: &str = "id, profile_id, project_id, state, requested_at, started_at, stopped_at,
    start_workflow, verification_workflow, start_execution, verification_id,
    verification_execution, can_finish, verification_details, verified_at, approval_id,
    approved_at, approval_boot, last_alive_at, problem, problem_message, problem_at, updated_at";

fn invalid_text(column: usize, value: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        rusqlite::types::Type::Text,
        format!("invalid value `{value}`").into(),
    )
}

fn optional_id<T: std::str::FromStr>(row: &Row<'_>, column: usize) -> rusqlite::Result<Option<T>> {
    row.get::<_, Option<String>>(column)?
        .map(|value| parse_id(&value))
        .transpose()
}

fn problem_name(problem: WorkSessionProblem) -> String {
    serde_json::to_value(problem)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn event_name(event: WorkSessionEvent) -> String {
    serde_json::to_value(event)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn record(row: &Row<'_>) -> rusqlite::Result<WorkSessionRecord> {
    let status: String = row.get(3)?;
    let problem = match row.get::<_, Option<String>>(19)? {
        Some(name) => Some(WorkSessionNotice {
            problem: serde_json::from_value(serde_json::Value::String(name.clone()))
                .map_err(|_| invalid_text(19, &name))?,
            message: row.get::<_, Option<String>>(20)?.unwrap_or_default(),
            at_ms: row.get::<_, Option<i64>>(21)?.unwrap_or_default(),
        }),
        None => None,
    };
    Ok(WorkSessionRecord {
        id: parse_id(&row.get::<_, String>(0)?)?,
        profile: parse_id(&row.get::<_, String>(1)?)?,
        project: optional_id(row, 2)?,
        status: SessionStatus::parse(&status).ok_or_else(|| invalid_text(3, &status))?,
        requested_at_ms: row.get(4)?,
        started_at_ms: row.get(5)?,
        stopped_at_ms: row.get(6)?,
        start_workflow: optional_id(row, 7)?,
        verification_workflow: optional_id(row, 8)?,
        start_execution: optional_id(row, 9)?,
        verification: optional_id(row, 10)?,
        verification_execution: optional_id(row, 11)?,
        can_finish: row.get(12)?,
        verification_details: row.get(13)?,
        verified_at_ms: row.get(14)?,
        approval: optional_id(row, 15)?,
        approved_at_ms: row.get(16)?,
        approval_boot: row.get(17)?,
        last_alive_at_ms: row.get(18)?,
        problem,
        updated_at_ms: row.get(22)?,
    })
}

pub(crate) fn get(connection: &Connection, id: WorkSessionId) -> Result<Option<WorkSessionRecord>> {
    Ok(connection
        .query_row(
            &format!("SELECT {COLUMNS} FROM work_sessions WHERE id = ?1"),
            [id.to_string()],
            record,
        )
        .optional()?)
}

pub(crate) fn active(connection: &Connection) -> Result<Option<WorkSessionRecord>> {
    Ok(connection
        .query_row(
            &format!("SELECT {COLUMNS} FROM work_sessions WHERE active = 1"),
            [],
            record,
        )
        .optional()?)
}

/// The session, which must exist after a write in the same transaction.
fn reload(tx: &Connection, id: WorkSessionId) -> Result<WorkSessionRecord> {
    get(tx, id)?.ok_or(StorageError::Invalid(
        "the work session disappeared".to_owned(),
    ))
}

/// Changes the state with a compare-and-set on `(id, from)` and appends the
/// transition to the log. The pair must be in the core table.
fn change_state(
    tx: &Connection,
    id: WorkSessionId,
    from: SessionStatus,
    event: WorkSessionEvent,
    at_ms: i64,
) -> Result<SessionStatus> {
    let to = from.next(event).ok_or_else(|| {
        StorageError::Invalid(format!(
            "a work session cannot go from {} on {}",
            from.as_str(),
            event_name(event)
        ))
    })?;
    let changed = tx.execute(
        "UPDATE work_sessions
         SET state = ?3, active = CASE WHEN ?4 THEN 1 END, updated_at = ?5
         WHERE id = ?1 AND state = ?2",
        params![
            id.to_string(),
            from.as_str(),
            to.as_str(),
            to.is_active(),
            at_ms
        ],
    )?;
    if changed != 1 {
        return Err(StorageError::Invalid(
            "the work session changed during its transition".to_owned(),
        ));
    }
    log(tx, id, from.as_str(), event, to.as_str(), at_ms)?;
    Ok(to)
}

fn log(
    tx: &Connection,
    id: WorkSessionId,
    from: &str,
    event: WorkSessionEvent,
    to: &str,
    at_ms: i64,
) -> Result<()> {
    tx.execute(
        "INSERT INTO work_session_transitions (session_id, seq, at, from_state, event, to_state)
         VALUES (?1, (SELECT IFNULL(MAX(seq), 0) + 1 FROM work_session_transitions
                      WHERE session_id = ?1), ?2, ?3, ?4, ?5)",
        params![id.to_string(), at_ms, from, event_name(event), to],
    )?;
    Ok(())
}

fn set_problem(
    tx: &Connection,
    id: WorkSessionId,
    notice: Option<&WorkSessionNotice>,
) -> Result<()> {
    tx.execute(
        "UPDATE work_sessions SET problem = ?2, problem_message = ?3, problem_at = ?4
         WHERE id = ?1",
        params![
            id.to_string(),
            notice.map(|n| problem_name(n.problem)),
            notice.map(|n| n.message.as_str()),
            notice.map(|n| n.at_ms),
        ],
    )?;
    Ok(())
}

fn require_problem(notice: &WorkSessionNotice, allowed: &[WorkSessionProblem]) -> Result<()> {
    if allowed.contains(&notice.problem) {
        Ok(())
    } else {
        Err(StorageError::Invalid(format!(
            "`{}` is not a notice of this transition",
            problem_name(notice.problem)
        )))
    }
}

fn wrong_state(record: &WorkSessionRecord) -> Transition {
    Transition::Refused(Refusal::WrongState {
        status: record.status,
    })
}

/// Removes the oldest finished and aborted sessions beyond
/// [`KEPT_SESSIONS`], with their intervals, reviews and log.
fn prune(tx: &Connection) -> Result<()> {
    tx.execute(
        "DELETE FROM work_sessions WHERE active IS NULL AND id NOT IN (
             SELECT id FROM work_sessions WHERE active IS NULL
             ORDER BY requested_at DESC, id DESC LIMIT ?1)",
        [i64::try_from(KEPT_SESSIONS).unwrap_or(i64::MAX)],
    )?;
    Ok(())
}

/// T1.
pub(crate) fn begin(tx: &Connection, new: &NewWorkSession) -> Result<Transition> {
    if let Some(existing) = get(tx, new.id)? {
        return Ok(if existing.status == SessionStatus::Starting {
            Transition::Repeated(existing)
        } else {
            wrong_state(&existing)
        });
    }
    if let Some(active) = active(tx)? {
        return Ok(Transition::Refused(Refusal::AnotherSessionActive {
            active: active.id,
        }));
    }
    let inserted = tx.execute(
        "INSERT INTO work_sessions (id, profile_id, project_id, state, active, requested_at,
             start_workflow, last_alive_at, updated_at)
         VALUES (?1, ?2, ?3, 'starting', 1, ?4, ?5, ?4, ?4)",
        params![
            new.id.to_string(),
            new.profile.to_string(),
            new.project.map(|p| p.to_string()),
            new.at_ms,
            new.start_workflow.map(|w| w.to_string()),
        ],
    );
    match inserted {
        Ok(_) => {}
        // The unique `active` column: another session became active.
        Err(error) if constraint_violation(&error) => {
            return Ok(match active(tx)? {
                Some(active) => {
                    Transition::Refused(Refusal::AnotherSessionActive { active: active.id })
                }
                None => return Err(error.into()),
            });
        }
        Err(error) => return Err(error.into()),
    }
    log(
        tx,
        new.id,
        "idle",
        WorkSessionEvent::Start,
        SessionStatus::Starting.as_str(),
        new.at_ms,
    )?;
    Ok(Transition::Applied(reload(tx, new.id)?))
}

/// Records the execution of the start run (while starting).
pub(crate) fn set_start_execution(
    tx: &Connection,
    id: WorkSessionId,
    execution: ExecutionId,
) -> Result<bool> {
    Ok(tx.execute(
        "UPDATE work_sessions SET start_execution = ?2 WHERE id = ?1 AND state = 'starting'",
        params![id.to_string(), execution.to_string()],
    )? == 1)
}

/// Records the execution of the current verification run.
pub(crate) fn set_verification_execution(
    tx: &Connection,
    id: WorkSessionId,
    verification: VerificationId,
    execution: ExecutionId,
) -> Result<bool> {
    Ok(tx.execute(
        "UPDATE work_sessions SET verification_execution = ?3
         WHERE id = ?1 AND state = 'verifyingEnd' AND verification_id = ?2",
        params![
            id.to_string(),
            verification.to_string(),
            execution.to_string()
        ],
    )? == 1)
}

/// T2: the start workflow succeeded (or none is set). Opens interval 1.
pub(crate) fn start_succeeded(
    tx: &Connection,
    id: WorkSessionId,
    at_ms: i64,
) -> Result<Transition> {
    let Some(session) = get(tx, id)? else {
        return Ok(Transition::Refused(Refusal::UnknownSession));
    };
    if session.status != SessionStatus::Starting {
        return Ok(if session.started_at_ms.is_some() {
            Transition::Repeated(session)
        } else {
            wrong_state(&session)
        });
    }
    change_state(
        tx,
        id,
        session.status,
        WorkSessionEvent::StartSucceeded,
        at_ms,
    )?;
    tx.execute(
        "UPDATE work_sessions SET started_at = ?2, last_alive_at = ?2, problem = NULL,
             problem_message = NULL, problem_at = NULL
         WHERE id = ?1",
        params![id.to_string(), at_ms],
    )?;
    tx.execute(
        "INSERT INTO work_session_intervals (session_id, seq, started_at) VALUES (?1, 1, ?2)",
        params![id.to_string(), at_ms],
    )?;
    Ok(Transition::Applied(reload(tx, id)?))
}

/// T3: the start failed or was interrupted. No interval was opened.
pub(crate) fn start_failed(
    tx: &Connection,
    id: WorkSessionId,
    notice: &WorkSessionNotice,
    at_ms: i64,
) -> Result<Transition> {
    require_problem(
        notice,
        &[
            WorkSessionProblem::StartFailed,
            WorkSessionProblem::StartInterrupted,
        ],
    )?;
    let Some(session) = get(tx, id)? else {
        return Ok(Transition::Refused(Refusal::UnknownSession));
    };
    match session.status {
        SessionStatus::Starting => {}
        SessionStatus::Aborted => return Ok(Transition::Repeated(session)),
        _ => return Ok(wrong_state(&session)),
    }
    change_state(tx, id, session.status, WorkSessionEvent::StartFailed, at_ms)?;
    set_problem(tx, id, Some(notice))?;
    let record = reload(tx, id)?;
    prune(tx)?;
    Ok(Transition::Applied(record))
}

/// T4 (from working) and T8 (from readyToFinish): a new verification
/// `verification` starts; any approval is dropped.
pub(crate) fn begin_verification(
    tx: &Connection,
    id: WorkSessionId,
    from: SessionStatus,
    verification: VerificationId,
    workflow: Option<WorkflowId>,
    at_ms: i64,
) -> Result<Transition> {
    let Some(session) = get(tx, id)? else {
        return Ok(Transition::Refused(Refusal::UnknownSession));
    };
    if session.status == SessionStatus::VerifyingEnd && session.verification == Some(verification) {
        return Ok(Transition::Repeated(session));
    }
    if session.status != from
        || !matches!(from, SessionStatus::Working | SessionStatus::ReadyToFinish)
    {
        return Ok(wrong_state(&session));
    }
    if session.verification == Some(verification) {
        // A verification id is used once.
        return Ok(Transition::Refused(Refusal::StaleVerification));
    }
    change_state(tx, id, from, WorkSessionEvent::Verify, at_ms)?;
    tx.execute(
        "UPDATE work_sessions SET verification_id = ?2, verification_workflow = ?3,
             verification_execution = NULL, can_finish = NULL, verification_details = NULL,
             verified_at = NULL, approval_id = NULL, approved_at = NULL, approval_boot = NULL
         WHERE id = ?1",
        params![
            id.to_string(),
            verification.to_string(),
            workflow.map(|w| w.to_string())
        ],
    )?;
    Ok(Transition::Applied(reload(tx, id)?))
}

fn clean_details(details: Option<&str>) -> Option<String> {
    details
        .map(|text| {
            text.chars()
                .filter(|c| !c.is_control())
                .take(MAX_DETAILS)
                .collect::<String>()
                .trim()
                .to_owned()
        })
        .filter(|text| !text.is_empty())
}

/// T5, T6 and T7: the result of verification `verification`. Applied only
/// if it is the session's current verification and still running.
pub(crate) fn verification_result(
    tx: &Connection,
    id: WorkSessionId,
    verification: VerificationId,
    result: &VerificationResult,
    boot: &str,
    at_ms: i64,
) -> Result<Transition> {
    if let VerificationResult::Failed(notice) = result {
        require_problem(
            notice,
            &[
                WorkSessionProblem::VerificationFailed,
                WorkSessionProblem::VerificationInterrupted,
            ],
        )?;
    }
    let Some(session) = get(tx, id)? else {
        return Ok(Transition::Refused(Refusal::UnknownSession));
    };
    if session.verification != Some(verification) {
        return Ok(Transition::Refused(Refusal::StaleVerification));
    }
    if session.status != SessionStatus::VerifyingEnd {
        return Ok(if session.verified_at_ms.is_some() {
            Transition::Repeated(session)
        } else {
            wrong_state(&session)
        });
    }
    let id_text = id.to_string();
    match result {
        VerificationResult::Outcome(outcome) if outcome.can_finish => {
            change_state(
                tx,
                id,
                session.status,
                WorkSessionEvent::VerificationPassed,
                at_ms,
            )?;
            tx.execute(
                "UPDATE work_sessions SET can_finish = 1, verification_details = ?2,
                     verified_at = ?3, approval_id = ?4, approved_at = ?3, approval_boot = ?5,
                     problem = NULL, problem_message = NULL, problem_at = NULL
                 WHERE id = ?1",
                params![
                    id_text,
                    clean_details(outcome.details.as_deref()),
                    at_ms,
                    verification.to_string(),
                    boot
                ],
            )?;
        }
        VerificationResult::Outcome(outcome) => {
            change_state(
                tx,
                id,
                session.status,
                WorkSessionEvent::VerificationRejected,
                at_ms,
            )?;
            let details = clean_details(outcome.details.as_deref());
            tx.execute(
                "UPDATE work_sessions SET can_finish = 0, verification_details = ?2,
                     verified_at = ?3
                 WHERE id = ?1",
                params![id_text, details, at_ms],
            )?;
            let message = details.unwrap_or_else(|| "The work is not ready to finish".to_owned());
            set_problem(
                tx,
                id,
                Some(&WorkSessionNotice {
                    problem: WorkSessionProblem::NotReady,
                    message,
                    at_ms,
                }),
            )?;
        }
        VerificationResult::Failed(notice) => {
            change_state(
                tx,
                id,
                session.status,
                WorkSessionEvent::VerificationFailed,
                at_ms,
            )?;
            tx.execute(
                "UPDATE work_sessions SET can_finish = NULL, verification_details = NULL,
                     verified_at = ?2
                 WHERE id = ?1",
                params![id_text, at_ms],
            )?;
            set_problem(tx, id, Some(notice))?;
        }
    }
    Ok(Transition::Applied(reload(tx, id)?))
}

/// T9: the approval `approval` no longer holds. `notice` is
/// `approvalExpired` or `approvalInvalidated`, or `None` when the user
/// chose to go on working.
pub(crate) fn invalidate_approval(
    tx: &Connection,
    id: WorkSessionId,
    approval: VerificationId,
    notice: Option<&WorkSessionNotice>,
    at_ms: i64,
) -> Result<Transition> {
    if let Some(notice) = notice {
        require_problem(
            notice,
            &[
                WorkSessionProblem::ApprovalExpired,
                WorkSessionProblem::ApprovalInvalidated,
            ],
        )?;
    }
    let Some(session) = get(tx, id)? else {
        return Ok(Transition::Refused(Refusal::UnknownSession));
    };
    if session.status != SessionStatus::ReadyToFinish {
        return Ok(
            if session.status == SessionStatus::Working
                && session.verification == Some(approval)
                && session.approval.is_none()
            {
                Transition::Repeated(session)
            } else {
                wrong_state(&session)
            },
        );
    }
    if session.approval != Some(approval) {
        return Ok(Transition::Refused(Refusal::StaleVerification));
    }
    change_state(
        tx,
        id,
        session.status,
        WorkSessionEvent::ApprovalInvalidated,
        at_ms,
    )?;
    tx.execute(
        "UPDATE work_sessions SET approval_id = NULL, approved_at = NULL, approval_boot = NULL
         WHERE id = ?1",
        [id.to_string()],
    )?;
    set_problem(tx, id, notice)?;
    Ok(Transition::Applied(reload(tx, id)?))
}

/// T10: the final stop with approval `approval`, given to agent instance
/// `boot` and still within the profile's approval lifetime. Fixes the stop
/// time and closes the open interval at `at_ms`.
pub(crate) fn finish(
    tx: &Connection,
    id: WorkSessionId,
    approval: VerificationId,
    boot: &str,
    at_ms: i64,
) -> Result<Transition> {
    let Some(session) = get(tx, id)? else {
        return Ok(Transition::Refused(Refusal::UnknownSession));
    };
    if matches!(
        session.status,
        SessionStatus::Stopping | SessionStatus::Stopped
    ) && session.approval == Some(approval)
    {
        return Ok(Transition::Repeated(session));
    }
    if session.status != SessionStatus::ReadyToFinish {
        return Ok(wrong_state(&session));
    }
    if session.approval != Some(approval) {
        return Ok(Transition::Refused(Refusal::StaleVerification));
    }
    if session.approval_boot.as_deref() != Some(boot) {
        return Ok(Transition::Refused(Refusal::ApprovalFromAnotherBoot));
    }
    let settings = settings(tx, session.profile)?;
    if !session
        .approved_at_ms
        .is_some_and(|approved| settings.approval_holds(approved, at_ms))
    {
        return Ok(Transition::Refused(Refusal::ApprovalExpired));
    }
    change_state(tx, id, session.status, WorkSessionEvent::Finish, at_ms)?;
    tx.execute(
        "UPDATE work_sessions SET stopped_at = ?2, last_alive_at = ?2 WHERE id = ?1",
        params![id.to_string(), at_ms],
    )?;
    tx.execute(
        "UPDATE work_session_intervals SET ended_at = ?2, end_reason = 'stopped'
         WHERE session_id = ?1 AND ended_at IS NULL",
        params![id.to_string(), at_ms],
    )?;
    Ok(Transition::Applied(reload(tx, id)?))
}

/// T11: the stop is complete; the session becomes history.
pub(crate) fn complete_stop(tx: &Connection, id: WorkSessionId, at_ms: i64) -> Result<Transition> {
    let Some(session) = get(tx, id)? else {
        return Ok(Transition::Refused(Refusal::UnknownSession));
    };
    match session.status {
        SessionStatus::Stopping => {}
        SessionStatus::Stopped => return Ok(Transition::Repeated(session)),
        _ => return Ok(wrong_state(&session)),
    }
    change_state(
        tx,
        id,
        session.status,
        WorkSessionEvent::StopCompleted,
        at_ms,
    )?;
    let record = reload(tx, id)?;
    prune(tx)?;
    Ok(Transition::Applied(record))
}

/// The heartbeat: only `last_alive_at` of the active session moves.
pub(crate) fn touch(tx: &Connection, id: WorkSessionId, at_ms: i64) -> Result<bool> {
    Ok(tx.execute(
        "UPDATE work_sessions SET last_alive_at = ?2 WHERE id = ?1 AND active = 1",
        params![id.to_string(), at_ms],
    )? == 1)
}

pub(crate) fn clear_problem(tx: &Connection, id: WorkSessionId) -> Result<bool> {
    Ok(tx.execute(
        "UPDATE work_sessions SET problem = NULL, problem_message = NULL, problem_at = NULL
         WHERE id = ?1 AND problem IS NOT NULL",
        [id.to_string()],
    )? == 1)
}

/// The notice to show while no session is active (an idle snapshot has
/// no session id): the notice of the newest session, if it is finished or
/// aborted and still has one, e.g. `startFailed` after T3.
pub(crate) fn idle_notice(
    connection: &Connection,
) -> Result<Option<(WorkSessionId, WorkSessionNotice)>> {
    if active(connection)?.is_some() {
        return Ok(None);
    }
    let newest = connection
        .query_row(
            &format!(
                "SELECT {COLUMNS} FROM work_sessions ORDER BY requested_at DESC, id DESC LIMIT 1"
            ),
            [],
            record,
        )
        .optional()?;
    Ok(newest.and_then(|session| session.problem.map(|notice| (session.id, notice))))
}

/// Clears the notices of every finished or aborted session (what clearing
/// the notice of an idle snapshot means). Returns how many were cleared.
pub(crate) fn clear_idle_notices(tx: &Connection) -> Result<usize> {
    Ok(tx.execute(
        "UPDATE work_sessions SET problem = NULL, problem_message = NULL, problem_at = NULL
         WHERE active IS NULL AND problem IS NOT NULL",
        [],
    )?)
}

fn minutes(ms: i64) -> i64 {
    ms.unsigned_abs()
        .div_ceil(60_000)
        .try_into()
        .unwrap_or(i64::MAX)
}

/// Flags a gap of uncertain time while the timer runs (decision D8): if it
/// is longer than the profile's review threshold, a review is recorded and
/// the session gets the notice `needsReview`. The gap stays counted.
pub(crate) fn flag_gap(
    tx: &Connection,
    id: WorkSessionId,
    kind: GapKind,
    from_ms: i64,
    to_ms: i64,
    at_ms: i64,
) -> Result<Option<WorkSessionReview>> {
    let Some(session) = get(tx, id)? else {
        return Ok(None);
    };
    if !session.status.state().timer_running() {
        return Ok(None);
    }
    let length = to_ms.saturating_sub(from_ms);
    if (kind == GapKind::ClockBack) != (length < 0) {
        return Err(StorageError::Invalid(
            "only a clock set back has a gap that ends before it begins".to_owned(),
        ));
    }
    if length.unsigned_abs()
        <= settings(tx, session.profile)?
            .review_gap_ms()
            .unsigned_abs()
    {
        return Ok(None);
    }
    tx.execute(
        "INSERT INTO work_session_reviews (session_id, kind, from_at, to_at) VALUES (?1, ?2, ?3, ?4)",
        params![id.to_string(), kind.as_str(), from_ms, to_ms],
    )?;
    let review = tx.last_insert_rowid();
    let what = match kind {
        GapKind::AgentDown => "DeckPilot was not running",
        GapKind::Sleep => "the computer slept",
        GapKind::ClockBack => "the clock was set back",
        GapKind::ClockForward => "the clock was set forward",
    };
    set_problem(
        tx,
        id,
        Some(&WorkSessionNotice {
            problem: WorkSessionProblem::NeedsReview,
            message: format!(
                "{} min while {what} need a review; they are counted until you exclude them",
                minutes(length)
            ),
            at_ms,
        }),
    )?;
    review_by_id(tx, review)
}

fn review(row: &Row<'_>) -> rusqlite::Result<WorkSessionReview> {
    let kind: String = row.get(2)?;
    let resolution: Option<String> = row.get(5)?;
    Ok(WorkSessionReview {
        id: row.get(0)?,
        session: parse_id(&row.get::<_, String>(1)?)?,
        kind: GapKind::parse(&kind).ok_or_else(|| invalid_text(2, &kind))?,
        from_ms: row.get(3)?,
        to_ms: row.get(4)?,
        resolution: resolution
            .map(|text| ReviewResolution::parse(&text).ok_or_else(|| invalid_text(5, &text)))
            .transpose()?,
        resolved_at_ms: row.get(6)?,
    })
}

const REVIEW_COLUMNS: &str = "id, session_id, kind, from_at, to_at, resolution, resolved_at";

fn review_by_id(connection: &Connection, id: i64) -> Result<Option<WorkSessionReview>> {
    Ok(connection
        .query_row(
            &format!("SELECT {REVIEW_COLUMNS} FROM work_session_reviews WHERE id = ?1"),
            [id],
            review,
        )
        .optional()?)
}

pub(crate) fn reviews(
    connection: &Connection,
    id: WorkSessionId,
) -> Result<Vec<WorkSessionReview>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {REVIEW_COLUMNS} FROM work_session_reviews WHERE session_id = ?1 ORDER BY id"
    ))?;
    let rows = statement.query_map([id.to_string()], review)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Resolves a review, or changes its resolution:
///
/// - `Excluded` cuts `from..to` out of the session's intervals.
/// - `Counted` on an excluded review counts the gap again: `from..to` is
///   added back (except where another excluded review still applies, and
///   never before the start or after the stop), joining the intervals the
///   exclusion split.
///
/// Both happen in this transaction; elapsed time stays a plain sum of the
/// intervals. Resolving again the same way changes nothing. The notice
/// `needsReview` goes once no review of the session is open.
pub(crate) fn resolve_review(
    tx: &Connection,
    review_id: i64,
    resolution: ReviewResolution,
    at_ms: i64,
) -> Result<WorkSessionReview> {
    let Some(review) = review_by_id(tx, review_id)? else {
        return Err(StorageError::Invalid(format!(
            "there is no review {review_id}"
        )));
    };
    if review.resolution == Some(resolution) {
        return Ok(review);
    }
    let Some(session) = get(tx, review.session)? else {
        return Err(StorageError::Invalid(
            "the work session of this review is gone".to_owned(),
        ));
    };
    match resolution {
        ReviewResolution::Excluded => {
            if review.to_ms <= review.from_ms {
                return Err(StorageError::Invalid(
                    "a clock set back added no time that could be excluded".to_owned(),
                ));
            }
            rewrite_intervals(tx, &session, |ranges| {
                subtract(ranges, (review.from_ms, review.to_ms))
            })?;
        }
        ReviewResolution::Counted if review.resolution == Some(ReviewResolution::Excluded) => {
            let start = session.started_at_ms.unwrap_or(i64::MAX);
            let end = session.stopped_at_ms.unwrap_or(i64::MAX);
            let mut back = vec![(review.from_ms.max(start), review.to_ms.min(end))];
            for other in reviews(tx, review.session)? {
                if other.id != review.id && other.resolution == Some(ReviewResolution::Excluded) {
                    back = subtract(back, (other.from_ms, other.to_ms));
                }
            }
            rewrite_intervals(tx, &session, |mut ranges| {
                ranges.extend(back);
                ranges
            })?;
        }
        ReviewResolution::Counted => {}
    }
    tx.execute(
        "UPDATE work_session_reviews SET resolution = ?2, resolved_at = ?3 WHERE id = ?1",
        params![review_id, resolution.as_str(), at_ms],
    )?;
    if !reviews(tx, review.session)?
        .iter()
        .any(|r| r.resolution.is_none())
    {
        tx.execute(
            "UPDATE work_sessions SET problem = NULL, problem_message = NULL, problem_at = NULL
             WHERE id = ?1 AND problem = 'needsReview'",
            [review.session.to_string()],
        )?;
    }
    review_by_id(tx, review_id)?.ok_or(StorageError::Invalid("the review disappeared".to_owned()))
}

/// `ranges` without `cut`. Ranges are half-open; `i64::MAX` is "open".
fn subtract(ranges: Vec<(i64, i64)>, (from, to): (i64, i64)) -> Vec<(i64, i64)> {
    let mut kept = Vec::with_capacity(ranges.len() + 1);
    for (start, end) in ranges {
        if end <= from || start >= to {
            kept.push((start, end));
            continue;
        }
        if start < from {
            kept.push((start, from));
        }
        if end > to {
            kept.push((to, end));
        }
    }
    kept
}

/// Replaces the intervals of `session` by `change` applied to them, as
/// sorted, merged, non-empty ranges numbered from 1. An interval that ends
/// at the stop of a finished session ends as `stopped`, any other closed
/// one as `gapExcluded`; the open one stays open. Intervals whose end lies
/// before their start (the clock was set back before the stop) count
/// nothing and are kept as they are.
fn rewrite_intervals(
    tx: &Connection,
    session: &WorkSessionRecord,
    change: impl FnOnce(Vec<(i64, i64)>) -> Vec<(i64, i64)>,
) -> Result<()> {
    let (backwards, ranges): (Vec<StoredInterval>, Vec<StoredInterval>) =
        intervals(tx, session.id)?.into_iter().partition(|stored| {
            stored
                .interval
                .ended_at_ms
                .is_some_and(|end| end < stored.interval.started_at_ms)
        });
    let ranges = ranges
        .into_iter()
        .map(|stored| {
            (
                stored.interval.started_at_ms,
                stored.interval.ended_at_ms.unwrap_or(i64::MAX),
            )
        })
        .collect();
    let mut ranges: Vec<(i64, i64)> = change(ranges)
        .into_iter()
        .filter(|(start, end)| start < end)
        .collect();
    ranges.sort_unstable();
    let mut merged: Vec<(i64, i64)> = Vec::with_capacity(ranges.len());
    for (start, end) in ranges {
        match merged.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    let stopped = matches!(
        session.status,
        SessionStatus::Stopping | SessionStatus::Stopped
    );
    let mut rows: Vec<(i64, Option<i64>, Option<&str>)> = merged
        .into_iter()
        .map(|(start, end)| {
            if end == i64::MAX {
                (start, None, None)
            } else if stopped && Some(end) == session.stopped_at_ms {
                (start, Some(end), Some(IntervalEnd::Stopped.as_str()))
            } else {
                (start, Some(end), Some(IntervalEnd::GapExcluded.as_str()))
            }
        })
        .collect();
    rows.extend(backwards.iter().map(|stored| {
        (
            stored.interval.started_at_ms,
            stored.interval.ended_at_ms,
            stored.end_reason.map(IntervalEnd::as_str),
        )
    }));
    rows.sort_unstable();
    let id = session.id.to_string();
    tx.execute(
        "DELETE FROM work_session_intervals WHERE session_id = ?1",
        [&id],
    )?;
    for (seq, (start, end, reason)) in (1_i64..).zip(rows) {
        tx.execute(
            "INSERT INTO work_session_intervals (session_id, seq, started_at, ended_at, end_reason)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, seq, start, end, reason],
        )?;
    }
    Ok(())
}

pub(crate) fn intervals(connection: &Connection, id: WorkSessionId) -> Result<Vec<StoredInterval>> {
    let mut statement = connection.prepare(
        "SELECT seq, started_at, ended_at, end_reason FROM work_session_intervals
         WHERE session_id = ?1 ORDER BY seq",
    )?;
    let rows = statement.query_map([id.to_string()], |row| {
        let reason: Option<String> = row.get(3)?;
        Ok(StoredInterval {
            seq: row.get(0)?,
            interval: WorkInterval {
                started_at_ms: row.get(1)?,
                ended_at_ms: row.get(2)?,
            },
            end_reason: reason
                .map(|text| IntervalEnd::parse(&text).ok_or_else(|| invalid_text(3, &text)))
                .transpose()?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Counted time of a session at `now_ms`, from its persisted intervals.
pub(crate) fn elapsed(connection: &Connection, id: WorkSessionId, now_ms: i64) -> Result<u64> {
    let intervals: Vec<WorkInterval> = intervals(connection, id)?
        .into_iter()
        .map(|stored| stored.interval)
        .collect();
    Ok(elapsed_ms(&intervals, now_ms))
}

pub(crate) fn transitions(
    connection: &Connection,
    id: WorkSessionId,
) -> Result<Vec<TransitionRecord>> {
    let mut statement = connection.prepare(
        "SELECT seq, at, from_state, event, to_state FROM work_session_transitions
         WHERE session_id = ?1 ORDER BY seq",
    )?;
    let rows = statement.query_map([id.to_string()], |row| {
        Ok(TransitionRecord {
            seq: row.get(0)?,
            at_ms: row.get(1)?,
            from: row.get(2)?,
            event: row.get(3)?,
            to: row.get(4)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Sessions, newest request first, with their counted time at `now_ms`.
pub(crate) fn history(
    connection: &Connection,
    query: HistoryQuery,
    now_ms: i64,
) -> Result<Vec<WorkSessionSummary>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {COLUMNS} FROM work_sessions
         WHERE (?1 IS NULL OR profile_id = ?1) AND (?2 IS NULL OR requested_at < ?2)
         ORDER BY requested_at DESC, id DESC LIMIT ?3"
    ))?;
    let sessions: Vec<WorkSessionRecord> = statement
        .query_map(
            params![
                query.profile.map(|p| p.to_string()),
                query.before_ms,
                i64::try_from(query.limit).unwrap_or(i64::MAX)
            ],
            record,
        )?
        .collect::<rusqlite::Result<_>>()?;
    sessions
        .into_iter()
        .map(|session| {
            let open_reviews: u32 = connection.query_row(
                "SELECT COUNT(*) FROM work_session_reviews
                 WHERE session_id = ?1 AND resolution IS NULL",
                [session.id.to_string()],
                |row| row.get(0),
            )?;
            Ok(WorkSessionSummary {
                elapsed_ms: elapsed(connection, session.id, now_ms)?,
                open_reviews,
                session,
            })
        })
        .collect()
}

pub(crate) fn settings(connection: &Connection, profile: ProfileId) -> Result<WorkSessionSettings> {
    let stored = connection
        .query_row(
            "SELECT start_workflow, verification_workflow, approval_ttl_seconds,
                 review_gap_seconds, elapsed_format, updated_at
             FROM work_session_settings WHERE profile_id = ?1",
            [profile.to_string()],
            |row| {
                let format: String = row.get(4)?;
                Ok(WorkSessionSettings {
                    profile,
                    start_workflow: optional_id(row, 0)?,
                    verification_workflow: optional_id(row, 1)?,
                    approval_ttl_seconds: row.get(2)?,
                    review_gap_seconds: row.get(3)?,
                    elapsed_format: match format.as_str() {
                        "seconds" => ElapsedFormat::Seconds,
                        "minutes" => ElapsedFormat::Minutes,
                        _ => return Err(invalid_text(4, &format)),
                    },
                    updated_at_ms: row.get(5)?,
                })
            },
        )
        .optional()?;
    Ok(stored.unwrap_or_else(|| WorkSessionSettings::defaults(profile)))
}

pub(crate) fn save_settings(
    tx: &Connection,
    settings: &WorkSessionSettings,
    at_ms: i64,
) -> Result<WorkSessionSettings> {
    for (name, value) in [
        ("approval lifetime", settings.approval_ttl_seconds),
        ("review threshold", settings.review_gap_seconds),
    ] {
        if !SETTING_SECONDS.contains(&value) {
            return Err(StorageError::Invalid(format!(
                "the {name} must be between {} and {} seconds",
                SETTING_SECONDS.start(),
                SETTING_SECONDS.end()
            )));
        }
    }
    let saved = tx.execute(
        "INSERT INTO work_session_settings (profile_id, start_workflow, verification_workflow,
             approval_ttl_seconds, review_gap_seconds, elapsed_format, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (profile_id) DO UPDATE SET start_workflow = excluded.start_workflow,
             verification_workflow = excluded.verification_workflow,
             approval_ttl_seconds = excluded.approval_ttl_seconds,
             review_gap_seconds = excluded.review_gap_seconds,
             elapsed_format = excluded.elapsed_format, updated_at = excluded.updated_at",
        params![
            settings.profile.to_string(),
            settings.start_workflow.map(|w| w.to_string()),
            settings.verification_workflow.map(|w| w.to_string()),
            settings.approval_ttl_seconds,
            settings.review_gap_seconds,
            settings.elapsed_format.as_str(),
            at_ms
        ],
    );
    match saved {
        Ok(_) => self::settings(tx, settings.profile),
        Err(error) if constraint_violation(&error) => Err(StorageError::UnknownProfile),
        Err(error) => Err(error.into()),
    }
}

/// Whether `profile` has the active session (it cannot be deleted then).
pub(crate) fn profile_has_active(connection: &Connection, profile: ProfileId) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS (SELECT 1 FROM work_sessions WHERE active = 1 AND profile_id = ?1)",
        [profile.to_string()],
        |row| row.get(0),
    )?)
}

#[cfg(test)]
mod tests;
