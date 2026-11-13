//! Work session messages of protocol 8 (see
//! `docs/gestures-and-work-session.md` §9): the shapes the agent sends for
//! the history and settings, and the bounds applied to every text it sends.
//!
//! The snapshot itself is [`WorkSessionSnapshot`] from `deckpilot-core`.
//! Nothing here starts, verifies or finishes a session (decision D11): only
//! the session's key does.

use serde::{Deserialize, Serialize};

use deckpilot_core::work_session::{
    MAX_DETAILS, VerificationRecord, WorkSessionNotice, WorkSessionSnapshot,
};
use deckpilot_core::{ProfileId, ProjectId, WorkSessionId, WorkflowId};

/// Most sessions one `listWorkSessions` returns.
pub const MAX_WORK_SESSION_LIST: u32 = 100;

/// Longest notice message sent, in characters.
pub const MAX_NOTICE_MESSAGE: usize = deckpilot_core::history::MAX_MESSAGE_CHARS;

/// How elapsed time is shown on keys and in the app (decision D4).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ElapsedFormat {
    /// `h:mm`, redrawn once per minute.
    #[default]
    Minutes,
    /// `h:mm:ss`, redrawn every second.
    Seconds,
}

/// The work session settings of one profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkSessionSettings {
    pub profile: ProfileId,
    /// Runs before the timer starts; `None`: the session starts at once.
    #[serde(default)]
    pub start_workflow: Option<WorkflowId>,
    /// Must fulfil the verification contract; `None`: hold the key to
    /// finish (D12).
    #[serde(default)]
    pub verification_workflow: Option<WorkflowId>,
    pub approval_ttl_seconds: u32,
    pub review_gap_seconds: u32,
    #[serde(default)]
    pub elapsed_format: ElapsedFormat,
    /// When the settings were last saved; absent for defaults never saved.
    /// Ignored when saving.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at_ms: Option<i64>,
}

/// The stored state of a session: an active one, or history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkSessionRecordState {
    Starting,
    Working,
    VerifyingEnd,
    ReadyToFinish,
    Stopping,
    /// Finished.
    Stopped,
    /// The start failed or was interrupted; no time was counted.
    Aborted,
}

/// Why an interval of counted time ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IntervalEndReason {
    Stopped,
    /// Closed where an excluded gap begins.
    GapExcluded,
}

/// A stretch of counted time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkSessionInterval {
    pub seq: u32,
    pub started_at_ms: i64,
    /// Absent while the interval is open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_reason: Option<IntervalEndReason>,
}

/// What made a stretch of time uncertain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkSessionReviewKind {
    /// No agent was running.
    AgentDown,
    Sleep,
    ClockBack,
    ClockForward,
}

/// How a flagged gap was decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkSessionReviewResolution {
    Counted,
    Excluded,
}

/// A flagged gap (decision D8): counted unless the user excludes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkSessionReview {
    pub id: i64,
    pub kind: WorkSessionReviewKind,
    pub from_ms: i64,
    pub to_ms: i64,
    /// Absent while nobody decided (the gap is counted).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<WorkSessionReviewResolution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at_ms: Option<i64>,
}

/// One entry of a session's audit log. States are
/// [`WorkSessionRecordState`] names, plus `idle` before the session
/// existed; events are `WorkSessionEvent` names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkSessionTransition {
    pub seq: u32,
    pub at_ms: i64,
    pub from: String,
    pub event: String,
    pub to: String,
}

/// A session of the history (`listWorkSessions`), with everything the app
/// shows about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkSessionSummary {
    pub session: WorkSessionId,
    pub profile: ProfileId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<ProjectId>,
    pub state: WorkSessionRecordState,
    pub requested_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stopped_at_ms: Option<i64>,
    /// Counted time when the answer was made: the sum of the intervals.
    pub elapsed_ms: u64,
    pub intervals: Vec<WorkSessionInterval>,
    pub reviews: Vec<WorkSessionReview>,
    pub transitions: Vec<WorkSessionTransition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<VerificationRecord>,
    /// The session's last notice, until it is cleared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<WorkSessionNotice>,
}

/// Verification details as they may be sent: without control characters,
/// at most [`MAX_DETAILS`] characters, and absent when empty.
pub fn bounded_details(details: Option<String>) -> Option<String> {
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

/// A notice as it may be sent: its message at most
/// [`MAX_NOTICE_MESSAGE`] characters.
pub fn bounded_notice(mut notice: WorkSessionNotice) -> WorkSessionNotice {
    notice.message = deckpilot_core::history::shorten(&notice.message);
    notice
}

fn bounded_verification(mut record: VerificationRecord) -> VerificationRecord {
    if let Some(outcome) = record.outcome.as_mut() {
        outcome.details = bounded_details(outcome.details.take());
    }
    record
}

/// The snapshot as the agent sends it: every text bounded.
#[must_use]
pub fn bounded_snapshot(mut snapshot: WorkSessionSnapshot) -> WorkSessionSnapshot {
    snapshot.verification = snapshot.verification.map(bounded_verification);
    snapshot.notices = snapshot.notices.into_iter().map(bounded_notice).collect();
    snapshot
}

impl WorkSessionSummary {
    /// The entry as the agent sends it: every text bounded.
    #[must_use]
    pub fn bounded(mut self) -> Self {
        self.verification = self.verification.map(bounded_verification);
        self.notice = self.notice.map(bounded_notice);
        self
    }
}

#[cfg(test)]
mod tests {
    use deckpilot_core::VerificationId;
    use deckpilot_core::work_session::{VerificationOutcome, WorkSessionProblem};
    use serde_json::json;

    use super::*;

    #[test]
    fn texts_are_bounded() {
        let long = format!("{}\u{7}{}", "a".repeat(150), "b".repeat(150));
        let mut snapshot = WorkSessionSnapshot::idle(1);
        snapshot.verification = Some(VerificationRecord {
            id: VerificationId::new(),
            outcome: Some(VerificationOutcome {
                can_finish: false,
                details: Some(long.clone()),
            }),
            at_ms: 1,
        });
        snapshot.notices.push(WorkSessionNotice {
            problem: WorkSessionProblem::NotReady,
            message: "x".repeat(2000),
            at_ms: 1,
        });
        let sent = bounded_snapshot(snapshot);
        let details = sent.verification.unwrap().outcome.unwrap().details.unwrap();
        assert_eq!(details.chars().count(), MAX_DETAILS);
        assert!(!details.contains('\u{7}'));
        assert!(sent.notices[0].message.chars().count() <= MAX_NOTICE_MESSAGE + 1);
        assert_eq!(bounded_details(Some(" \n ".into())), None);
    }

    #[test]
    fn settings_and_history_use_camel_case() {
        let profile = ProfileId::new();
        let settings: WorkSessionSettings = serde_json::from_value(json!({
            "profile": profile,
            "startWorkflow": null,
            "approvalTtlSeconds": 600,
            "reviewGapSeconds": 900,
        }))
        .unwrap();
        assert_eq!(settings.elapsed_format, ElapsedFormat::Minutes);
        assert_eq!(settings.verification_workflow, None);
        assert_eq!(
            serde_json::to_value(&settings).unwrap(),
            json!({
                "profile": profile,
                "startWorkflow": null,
                "verificationWorkflow": null,
                "approvalTtlSeconds": 600,
                "reviewGapSeconds": 900,
                "elapsedFormat": "minutes",
            })
        );
        let review = WorkSessionReview {
            id: 3,
            kind: WorkSessionReviewKind::AgentDown,
            from_ms: 1,
            to_ms: 2,
            resolution: Some(WorkSessionReviewResolution::Excluded),
            resolved_at_ms: Some(3),
        };
        assert_eq!(
            serde_json::to_value(&review).unwrap(),
            json!({"id": 3, "kind": "agentDown", "fromMs": 1, "toMs": 2,
                   "resolution": "excluded", "resolvedAtMs": 3})
        );
        assert_eq!(
            serde_json::to_value(WorkSessionRecordState::VerifyingEnd).unwrap(),
            json!("verifyingEnd")
        );
        assert_eq!(
            serde_json::to_value(IntervalEndReason::GapExcluded).unwrap(),
            json!("gapExcluded")
        );
    }
}
