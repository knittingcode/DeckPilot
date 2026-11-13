//! The work session requests of protocol 8 (§9): the session, its history
//! and settings, clearing notices and deciding flagged gaps. They read
//! state or change what the user decides about it; none starts, verifies
//! or finishes a session (D11). Every text sent is bounded
//! ([`deckpilot_ipc::work_session`]).

use deckpilot_core::work_session::{WorkSessionSnapshot, verification_contract};
use deckpilot_core::{ProfileId, WorkSessionId, Workflow, WorkflowId};
use deckpilot_ipc::work_session::{
    self as wire, IntervalEndReason, WorkSessionRecordState, WorkSessionReviewKind,
    WorkSessionReviewResolution,
};
use deckpilot_ipc::{ErrorCode, IpcError, Request, Response};
use deckpilot_storage::work_sessions::{
    ElapsedFormat, GapKind, HistoryQuery, IntervalEnd, ReviewResolution, SessionStatus,
    WorkSessionRecord, WorkSessionSettings,
};
use deckpilot_storage::{ConfigStore, SqliteStore, StorageError};

use super::super::Agent;
use super::VerificationRecord;

/// Inputs of a session workflow the agent fills (§7); others need a
/// default or must be optional.
pub const SESSION_INPUTS: [&str; 3] = ["sessionId", "projectId", "elapsedSeconds"];

fn storage(error: &StorageError) -> IpcError {
    match error {
        StorageError::Invalid(message) => IpcError::new(ErrorCode::InvalidRequest, message),
        StorageError::UnknownProfile => IpcError::new(ErrorCode::NotFound, error.to_string()),
        other => super::super::storage_error(other),
    }
}

fn invalid(message: impl Into<String>) -> IpcError {
    IpcError::new(ErrorCode::InvalidRequest, message)
}

impl Agent {
    /// Answers a work session request; `None` for every other request.
    pub(crate) fn handle_work_session_request(
        &mut self,
        request: &Request,
    ) -> Option<Result<Response, IpcError>> {
        let result = match request {
            Request::GetWorkSession => self.get_work_session(),
            Request::ListWorkSessions { limit, before } => self.list_work_sessions(*limit, *before),
            Request::GetWorkSessionSettings { profile } => self.get_work_session_settings(*profile),
            Request::SaveWorkSessionSettings { settings } => {
                self.save_work_session_settings(settings)
            }
            Request::ResolveWorkSessionReview {
                session,
                review,
                count,
            } => self.resolve_work_session_review(*session, *review, *count),
            Request::ClearWorkSessionNotices { session } => {
                self.clear_work_session_notices(*session)
            }
            _ => return None,
        };
        Some(result)
    }

    fn work_session_store(&self) -> Result<&SqliteStore, IpcError> {
        self.store.as_ref().ok_or_else(super::super::locked)
    }

    /// The snapshot as it is now, as sent.
    fn current_work_session(&mut self) -> Response {
        // Moves elapsed time and `asOfMs` to now; announces real changes.
        self.work_session_changed();
        Response::WorkSession {
            snapshot: Box::new(sent(self.work.snapshot())),
        }
    }

    fn get_work_session(&mut self) -> Result<Response, IpcError> {
        self.work_session_store()?;
        Ok(self.current_work_session())
    }

    fn list_work_sessions(&self, limit: u32, before: Option<i64>) -> Result<Response, IpcError> {
        let store = self.work_session_store()?;
        let now = self.wall_ms();
        let query = HistoryQuery {
            profile: None,
            before_ms: before,
            limit: usize::try_from(limit).unwrap_or(usize::MAX),
        };
        let entries = store
            .work_session_history(query, now)
            .map_err(|e| storage(&e))?
            .into_iter()
            .map(|summary| summary_of(store, &summary.session, summary.elapsed_ms))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| storage(&e))?;
        Ok(Response::WorkSessions { entries })
    }

    fn get_work_session_settings(&self, profile: ProfileId) -> Result<Response, IpcError> {
        let store = self.work_session_store()?;
        if store
            .load_profile(profile)
            .map_err(|e| storage(&e))?
            .is_none()
        {
            return Err(storage(&StorageError::UnknownProfile));
        }
        let settings = store
            .work_session_settings(profile)
            .map_err(|e| storage(&e))?;
        Ok(Response::WorkSessionSettings {
            settings: Box::new(settings_to_wire(&settings)),
        })
    }

    fn save_work_session_settings(
        &mut self,
        settings: &wire::WorkSessionSettings,
    ) -> Result<Response, IpcError> {
        let store = self.work_session_store()?;
        let Some(profile) = store
            .load_profile(settings.profile)
            .map_err(|e| storage(&e))?
        else {
            return Err(storage(&StorageError::UnknownProfile));
        };
        let find = |id: WorkflowId| {
            profile
                .workflow(id)
                .ok_or_else(|| invalid("the workflow does not exist in this profile"))
        };
        if let Some(id) = settings.start_workflow {
            let workflow = find(id)?;
            unfilled_inputs(workflow)
                .map_err(|issue| invalid(format!("the start workflow {issue}")))?;
        }
        if let Some(id) = settings.verification_workflow {
            let workflow = find(id)?;
            verification_contract(&workflow.outputs)
                .map_err(|issue| invalid(format!("the verification workflow: {issue}")))?;
            unfilled_inputs(workflow)
                .map_err(|issue| invalid(format!("the verification workflow {issue}")))?;
        }
        let saved = store
            .save_work_session_settings(&settings_from_wire(settings), self.wall_ms())
            .map_err(|e| storage(&e))?;
        // A changed setting drops an approval of the session (D9).
        self.work_session_settings_reloaded();
        Ok(Response::WorkSessionSettings {
            settings: Box::new(settings_to_wire(&saved)),
        })
    }

    fn resolve_work_session_review(
        &mut self,
        session: WorkSessionId,
        review: i64,
        count: bool,
    ) -> Result<Response, IpcError> {
        let store = self.work_session_store()?;
        let of_session = store
            .work_session_reviews(session)
            .map_err(|e| storage(&e))?
            .iter()
            .any(|r| r.id == review);
        if !of_session {
            return Err(IpcError::new(
                ErrorCode::NotFound,
                "the work session has no such review",
            ));
        }
        let resolution = if count {
            ReviewResolution::Counted
        } else {
            ReviewResolution::Excluded
        };
        let now = self.wall_ms();
        store
            .resolve_work_session_review(review, resolution, now)
            .map_err(|e| storage(&e))?;
        let elapsed = store
            .work_session_elapsed_ms(session, now)
            .map_err(|e| storage(&e))?;
        let record = store
            .work_session(session)
            .map_err(|e| storage(&e))?
            .ok_or_else(|| IpcError::new(ErrorCode::NotFound, "the work session is gone"))?;
        let entry = summary_of(store, &record, elapsed).map_err(|e| storage(&e))?;
        // The counted time and maybe the review notice changed.
        self.publish_work_session(true);
        Ok(Response::WorkSessionEntry {
            entry: Box::new(entry),
        })
    }

    fn clear_work_session_notices(
        &mut self,
        session: Option<WorkSessionId>,
    ) -> Result<Response, IpcError> {
        let store = self.work_session_store()?;
        match session {
            Some(id) => {
                if store.work_session(id).map_err(|e| storage(&e))?.is_none() {
                    return Err(IpcError::new(
                        ErrorCode::NotFound,
                        "the work session no longer exists",
                    ));
                }
                store
                    .clear_work_session_problem(id)
                    .map_err(|e| storage(&e))?;
                // The active session's notice lives in memory as well.
                if self.work.record.as_ref().is_some_and(|r| r.id == id) {
                    let fresh = store.work_session(id).map_err(|e| storage(&e))?;
                    if let Some(record) = fresh {
                        self.work.record = Some(record);
                    }
                }
            }
            None => {
                store
                    .clear_idle_work_session_notices()
                    .map_err(|e| storage(&e))?;
            }
        }
        Ok(self.current_work_session())
    }
}

fn sent(snapshot: WorkSessionSnapshot) -> WorkSessionSnapshot {
    wire::bounded_snapshot(snapshot)
}

/// Why `workflow` cannot run for a work session: a required input without
/// a default that the session does not fill.
fn unfilled_inputs(workflow: &Workflow) -> Result<(), String> {
    let missing: Vec<&str> = workflow
        .inputs
        .iter()
        .filter(|input| input.required && input.default.is_none())
        .filter(|input| !SESSION_INPUTS.contains(&input.key.as_str()))
        .map(|input| input.key.as_str())
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "needs inputs a work session cannot give: {}",
            missing.join(", ")
        ))
    }
}

fn settings_to_wire(settings: &WorkSessionSettings) -> wire::WorkSessionSettings {
    wire::WorkSessionSettings {
        profile: settings.profile,
        start_workflow: settings.start_workflow,
        verification_workflow: settings.verification_workflow,
        approval_ttl_seconds: settings.approval_ttl_seconds,
        review_gap_seconds: settings.review_gap_seconds,
        elapsed_format: match settings.elapsed_format {
            ElapsedFormat::Minutes => wire::ElapsedFormat::Minutes,
            ElapsedFormat::Seconds => wire::ElapsedFormat::Seconds,
        },
        updated_at_ms: settings.updated_at_ms,
    }
}

fn settings_from_wire(settings: &wire::WorkSessionSettings) -> WorkSessionSettings {
    WorkSessionSettings {
        profile: settings.profile,
        start_workflow: settings.start_workflow,
        verification_workflow: settings.verification_workflow,
        approval_ttl_seconds: settings.approval_ttl_seconds,
        review_gap_seconds: settings.review_gap_seconds,
        elapsed_format: match settings.elapsed_format {
            wire::ElapsedFormat::Minutes => ElapsedFormat::Minutes,
            wire::ElapsedFormat::Seconds => ElapsedFormat::Seconds,
        },
        updated_at_ms: None,
    }
}

fn state_of(status: SessionStatus) -> WorkSessionRecordState {
    match status {
        SessionStatus::Starting => WorkSessionRecordState::Starting,
        SessionStatus::Working => WorkSessionRecordState::Working,
        SessionStatus::VerifyingEnd => WorkSessionRecordState::VerifyingEnd,
        SessionStatus::ReadyToFinish => WorkSessionRecordState::ReadyToFinish,
        SessionStatus::Stopping => WorkSessionRecordState::Stopping,
        SessionStatus::Stopped => WorkSessionRecordState::Stopped,
        SessionStatus::Aborted => WorkSessionRecordState::Aborted,
    }
}

fn verification_of(record: &WorkSessionRecord) -> Option<VerificationRecord> {
    record.verification.map(|id| VerificationRecord {
        id,
        outcome: record.can_finish.map(|can_finish| {
            deckpilot_core::work_session::VerificationOutcome {
                can_finish,
                details: record.verification_details.clone(),
            }
        }),
        at_ms: record.verified_at_ms.unwrap_or(record.updated_at_ms),
    })
}

/// The history entry of `session`, read from the store.
fn summary_of(
    store: &SqliteStore,
    record: &WorkSessionRecord,
    elapsed_ms: u64,
) -> Result<wire::WorkSessionSummary, StorageError> {
    let intervals = store
        .work_session_intervals(record.id)?
        .into_iter()
        .map(|stored| wire::WorkSessionInterval {
            seq: stored.seq,
            started_at_ms: stored.interval.started_at_ms,
            ended_at_ms: stored.interval.ended_at_ms,
            end_reason: stored.end_reason.map(|end| match end {
                IntervalEnd::Stopped => IntervalEndReason::Stopped,
                IntervalEnd::GapExcluded => IntervalEndReason::GapExcluded,
            }),
        })
        .collect();
    let reviews = store
        .work_session_reviews(record.id)?
        .into_iter()
        .map(|review| wire::WorkSessionReview {
            id: review.id,
            kind: match review.kind {
                GapKind::AgentDown => WorkSessionReviewKind::AgentDown,
                GapKind::Sleep => WorkSessionReviewKind::Sleep,
                GapKind::ClockBack => WorkSessionReviewKind::ClockBack,
                GapKind::ClockForward => WorkSessionReviewKind::ClockForward,
            },
            from_ms: review.from_ms,
            to_ms: review.to_ms,
            resolution: review.resolution.map(|resolution| match resolution {
                ReviewResolution::Counted => WorkSessionReviewResolution::Counted,
                ReviewResolution::Excluded => WorkSessionReviewResolution::Excluded,
            }),
            resolved_at_ms: review.resolved_at_ms,
        })
        .collect();
    let transitions = store
        .work_session_transitions(record.id)?
        .into_iter()
        .map(|t| wire::WorkSessionTransition {
            seq: t.seq,
            at_ms: t.at_ms,
            from: t.from,
            event: t.event,
            to: t.to,
        })
        .collect();
    Ok(wire::WorkSessionSummary {
        session: record.id,
        profile: record.profile,
        project: record.project,
        state: state_of(record.status),
        requested_at_ms: record.requested_at_ms,
        started_at_ms: record.started_at_ms,
        stopped_at_ms: record.stopped_at_ms,
        elapsed_ms,
        intervals,
        reviews,
        transitions,
        verification: verification_of(record),
        notice: record.problem.clone(),
    }
    .bounded())
}
