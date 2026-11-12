//! The work session service: applies the transitions of
//! [`deckpilot_core::work_session`] through the store (schema 9), runs the
//! start and end-verification workflows as tagged runs, recovers the
//! persisted session whenever the database opens, keeps the heartbeat and
//! notices sleep and jumps of the wall clock. See
//! `docs/gestures-and-work-session.md` §5–§7.
//!
//! - **Who operates it.** Only a key does (decision D11): the gesture layer
//!   calls [`Agent::work_session_operation`] with the operation its rule
//!   picked from the state **at the press** ([`PressContext`]). There is no
//!   IPC request that starts, verifies or finishes a session. Tests reach
//!   the same entry point through `AgentHandle::work_session_operation`
//!   (feature `test-hooks`).
//! - **Guards.** Each operation checks the state of the press against the
//!   current state, so a press that began in another state does nothing.
//!   Start and verify need the session's profile to be active; finishing
//!   does not (D7). Finishing needs a single click on the key, the
//!   approval seen at the press, and the arming delay (D9, D11). Without a
//!   verification workflow, only a long press verifies, and it passes at
//!   once (D12).
//! - **Persistence first.** Every transition is committed by the store
//!   before the in-memory state changes; the store's compare-and-set
//!   answers a repeated request (a duplicated click, a late result) with
//!   `Repeated` and a stale one with `Refused`, so nothing is applied twice
//!   and an old verification never moves a newer one.
//! - **Time.** Elapsed time is never counted here: it is the sum of the
//!   stored intervals. The wall clock is stored; the monotonic clock only
//!   detects sleep and clock jumps (§6.1) and times the arming delay.
//! - **Events.** Every change updates the [`WorkSessionSnapshot`] watch
//!   (`AgentHandle::work_session`) and redraws the keys; key faces (W4) and
//!   the `workSessionChanged` event (W5) build on
//!   [`Agent::work_session_changed`].

use std::collections::BTreeMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use deckpilot_core::gesture::Gesture;
use deckpilot_core::graph::{GraphReport, RunStatus, WorkflowOutput};
use deckpilot_core::work_session::{
    VerificationOutcome, VerificationRecord, WorkSessionNotice, WorkSessionProblem,
    WorkSessionSnapshot, WorkSessionState, verification_contract,
};
use deckpilot_core::{
    ExecutionId, ExecutionTrigger, ProfileId, VerificationId, WorkSessionId, WorkflowId,
};
use deckpilot_ipc::SecurityState;
use deckpilot_storage::SqliteStore;
use deckpilot_storage::work_sessions::{
    GapKind, NewWorkSession, Refusal, SessionStatus, Transition, VerificationResult,
    WorkSessionRecord, WorkSessionSettings,
};
use tokio::sync::watch;
use tracing::{debug, info, warn};

use super::Agent;
use super::triggers::Launch;

/// A finish click whose press comes earlier than this after entering
/// `readyToFinish` is ignored (D9).
pub const ARMING_DELAY: Duration = Duration::from_secs(1);
/// How often `last_alive_at` moves while a session is active (§6.2).
pub const HEARTBEAT: Duration = Duration::from_secs(60);
/// Wall-clock time that passed this much faster than monotonic time between
/// two checks means the computer slept (or the clock was set forward; the
/// two cannot be told apart, and both are handled alike).
pub const SLEEP_GAP: Duration = Duration::from_secs(90);
/// Wall-clock time going back by more than this between two checks means
/// the clock was set back.
pub const CLOCK_BACK: Duration = Duration::from_secs(2);
/// Details of a verification that passed because none is configured (D12).
pub const NO_VERIFICATION: &str = "no verification configured";

/// The operations of the `workSession` machine that a key binding can ask
/// for (§4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkSessionOperation {
    Start,
    CancelStart,
    Verify,
    CancelVerification,
    Finish,
    ContinueWorking,
}

/// The work session as the key saw it when it was pressed. The binding's
/// rule was picked from `state`; the operation is refused if the session
/// has moved on since.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PressContext {
    pub state: WorkSessionState,
    pub session: Option<WorkSessionId>,
    /// The approval a finish click finishes with.
    pub approval: Option<VerificationId>,
    /// When the key was pressed (stamped where the input was read).
    pub at: Instant,
}

impl PressContext {
    pub fn of(snapshot: &WorkSessionSnapshot, at: Instant) -> Self {
        Self {
            state: snapshot.state,
            session: snapshot.session,
            approval: snapshot.approval,
            at,
        }
    }
}

/// One operation asked for by a key gesture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkSessionRequest {
    pub operation: WorkSessionOperation,
    pub gesture: Gesture,
    pub press: PressContext,
}

/// Why an operation did nothing. [`Ignored::hint`] is short enough for a
/// key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ignored {
    /// The database is locked: nothing can be written.
    Locked,
    /// The session is no longer in the state of the press.
    StateChanged,
    /// The operation does not apply in this state.
    NotNow,
    /// Start and verify need the session's profile to be active (D7).
    ProfileNotActive,
    /// A finish that is not a single click (D11).
    NotAClick,
    /// A finish click too soon after entering `readyToFinish` (D9).
    NotArmed,
    /// A click in working without a verification workflow (D12).
    HoldToFinish,
    /// A long press in working while a verification workflow is set.
    ClickToCheck,
}

impl Ignored {
    pub fn hint(self) -> &'static str {
        match self {
            Self::Locked => "Unlock DeckPilot",
            Self::StateChanged | Self::NotNow | Self::NotArmed | Self::NotAClick => "",
            Self::ProfileNotActive => "Switch to the session's profile",
            Self::HoldToFinish => "Hold to finish",
            Self::ClickToCheck => "Click to check",
        }
    }
}

/// What an operation did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkSessionOutcome {
    /// Applied, or a workflow was started whose result applies later.
    Applied,
    /// The same operation was applied before (a duplicated click).
    AlreadyDone,
    Ignored(Ignored),
    /// Could not be done; the session carries a notice where it has one.
    Failed(String),
}

/// What a run started by the work session is for. Kept on the running
/// execution; its result is applied only while it is the session's
/// current run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunPurpose {
    Start {
        session: WorkSessionId,
    },
    Verify {
        session: WorkSessionId,
        verification: VerificationId,
    },
}

/// How a tagged run ended, read before its report is made safe for
/// history.
#[derive(Debug)]
pub(crate) struct RunEnd {
    pub status: Option<RunStatus>,
    pub message: Option<String>,
    pub declared: Vec<WorkflowOutput>,
    pub outputs: BTreeMap<String, String>,
}

impl RunEnd {
    pub(crate) fn from_report(
        definition: Option<&deckpilot_core::Workflow>,
        report: &GraphReport,
    ) -> Self {
        let failure = report
            .nodes
            .iter()
            .find(|n| n.status == deckpilot_core::graph::NodeStatus::Failed)
            .map(|n| {
                let message = n.error.as_ref().map_or(n.message.as_str(), |e| &e.message);
                deckpilot_core::history::shorten(message)
            });
        Self {
            status: Some(report.status),
            message: failure,
            declared: definition.map(|d| d.outputs.clone()).unwrap_or_default(),
            outputs: definition
                .map(|d| deckpilot_core::declared_outputs(d, report))
                .unwrap_or_default(),
        }
    }

    pub(crate) fn failed(message: impl Into<String>) -> Self {
        Self {
            status: None,
            message: Some(message.into()),
            declared: Vec::new(),
            outputs: BTreeMap::new(),
        }
    }

    /// A verification run that succeeded with `canFinish`, or failed.
    #[cfg(feature = "test-hooks")]
    pub(crate) fn with_can_finish(can_finish: Option<bool>) -> Self {
        let Some(can_finish) = can_finish else {
            return Self::failed("failed on purpose");
        };
        Self {
            status: Some(RunStatus::Succeeded),
            message: None,
            declared: vec![WorkflowOutput::new(
                deckpilot_core::work_session::CAN_FINISH_OUTPUT,
                deckpilot_core::graph::ValueKind::Boolean,
            )],
            outputs: BTreeMap::from([(
                deckpilot_core::work_session::CAN_FINISH_OUTPUT.to_owned(),
                can_finish.to_string(),
            )]),
        }
    }

    pub(crate) fn cancelled() -> Self {
        Self {
            status: Some(RunStatus::Cancelled),
            ..Self::failed("cancelled")
        }
    }
}

/// The agent's side of the work session.
pub(crate) struct WorkSessions {
    /// This agent instance; an approval given to another one never
    /// finishes (§5.2).
    boot: String,
    /// The active session as last committed.
    record: Option<WorkSessionRecord>,
    /// The settings of the active session's profile, as last read.
    settings: Option<WorkSessionSettings>,
    /// When the session entered `readyToFinish` in this agent instance.
    ready_since: Option<Instant>,
    /// The session already recovered by this agent instance: a later
    /// unlock is not an agent restart.
    seen: Option<WorkSessionId>,
    /// The last clock check (wall and monotonic).
    last_wall: SystemTime,
    last_mono: Instant,
    /// The last heartbeat (wall and monotonic).
    beat_wall: SystemTime,
    beat_mono: Instant,
    /// Gaps noticed while the database was locked, flagged at the unlock.
    pending_gaps: Vec<(GapKind, i64, i64)>,
    /// The computer slept or the clock jumped while locked: an approval
    /// does not survive the unlock.
    pending_invalidation: bool,
    snapshot: watch::Sender<WorkSessionSnapshot>,
}

impl std::fmt::Debug for WorkSessions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkSessions")
            .field("boot", &self.boot)
            .field("state", &self.state())
            .finish_non_exhaustive()
    }
}

impl WorkSessions {
    pub(crate) fn new(now: SystemTime) -> (Self, watch::Receiver<WorkSessionSnapshot>) {
        let (snapshot, receiver) = watch::channel(WorkSessionSnapshot::idle(millis(now)));
        let mono = Instant::now();
        (
            Self {
                boot: WorkSessionId::new().to_string(),
                record: None,
                settings: None,
                ready_since: None,
                seen: None,
                last_wall: now,
                last_mono: mono,
                beat_wall: now,
                beat_mono: mono,
                pending_gaps: Vec::new(),
                pending_invalidation: false,
                snapshot,
            },
            receiver,
        )
    }

    pub(crate) fn state(&self) -> WorkSessionState {
        self.record
            .as_ref()
            .map_or(WorkSessionState::Idle, |r| r.status.state())
    }

    /// The snapshot last published.
    pub(crate) fn snapshot(&self) -> WorkSessionSnapshot {
        self.snapshot.borrow().clone()
    }
}

fn millis(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_millis()).unwrap_or(i64::MAX),
        Err(e) => -i64::try_from(e.duration().as_millis()).unwrap_or(i64::MAX),
    }
}

fn notice(
    problem: WorkSessionProblem,
    message: impl Into<String>,
    at_ms: i64,
) -> WorkSessionNotice {
    WorkSessionNotice {
        problem,
        message: message.into(),
        at_ms,
    }
}

/// The settings without their save time, to tell a real change.
fn comparable(settings: &WorkSessionSettings) -> WorkSessionSettings {
    WorkSessionSettings {
        updated_at_ms: None,
        ..settings.clone()
    }
}

impl Agent {
    fn wall_ms(&self) -> i64 {
        millis((self.options.clock)())
    }

    /// The work session as published.
    #[cfg_attr(
        not(feature = "test-hooks"),
        allow(
            dead_code,
            reason = "read by work session key faces once they land (W4)"
        )
    )]
    pub(crate) fn work_session_snapshot(&self) -> WorkSessionSnapshot {
        self.work.snapshot()
    }

    /// The entry point of key gestures (G2/G4) and tests: applies one
    /// operation, guarded by the state at the press.
    #[cfg_attr(
        not(feature = "test-hooks"),
        allow(
            dead_code,
            reason = "called by stateful key bindings once they land (G4)"
        )
    )]
    pub(crate) fn work_session_operation(
        &mut self,
        request: &WorkSessionRequest,
    ) -> WorkSessionOutcome {
        if self.security != SecurityState::Unlocked || self.store.is_none() {
            return WorkSessionOutcome::Ignored(Ignored::Locked);
        }
        let outcome = self.apply_operation(request);
        debug!(operation = ?request.operation, gesture = ?request.gesture, ?outcome, "work session operation");
        self.work_session_changed();
        outcome
    }

    fn apply_operation(&mut self, request: &WorkSessionRequest) -> WorkSessionOutcome {
        use WorkSessionOperation as Op;
        use WorkSessionState as S;
        let press = &request.press;
        let current = self.work.state();
        let current_id = self.work.record.as_ref().map(|r| r.id);
        // Finishing is answered by the store, so that a repeated click
        // is recognised after the session became history.
        if request.operation != Op::Finish
            && (press.state != current || press.session != current_id)
        {
            return WorkSessionOutcome::Ignored(Ignored::StateChanged);
        }
        match (request.operation, current) {
            (Op::Start, S::Idle) => self.start_session(),
            (Op::CancelStart, S::Starting) => self.cancel_run(false),
            (Op::Verify, S::Working | S::ReadyToFinish) => self.verify(request.gesture),
            (Op::CancelVerification, S::VerifyingEnd) => self.cancel_run(true),
            (Op::ContinueWorking, S::ReadyToFinish) => self.continue_working(),
            (Op::Finish, _) => self.finish(request),
            _ => WorkSessionOutcome::Ignored(Ignored::NotNow),
        }
    }

    fn work_store(&self) -> Result<&SqliteStore, WorkSessionOutcome> {
        self.store
            .as_ref()
            .ok_or(WorkSessionOutcome::Ignored(Ignored::Locked))
    }

    /// Applies a committed transition to the in-memory state.
    fn take_transition(
        &mut self,
        transition: Result<Transition, deckpilot_storage::StorageError>,
    ) -> Result<Transition, WorkSessionOutcome> {
        let transition = transition.map_err(|error| {
            warn!(%error, "cannot record a work session transition");
            WorkSessionOutcome::Failed(error.to_string())
        })?;
        if let Transition::Applied(record) | Transition::Repeated(record) = &transition {
            self.adopt(record.clone());
        }
        Ok(transition)
    }

    /// Makes `record` the in-memory session (or none, once it is history).
    fn adopt(&mut self, record: WorkSessionRecord) {
        let was_ready = self.work.state() == WorkSessionState::ReadyToFinish;
        if record.status == SessionStatus::ReadyToFinish {
            if !was_ready || self.work.ready_since.is_none() {
                self.work.ready_since = Some(Instant::now());
            }
        } else {
            self.work.ready_since = None;
        }
        if record.status.is_active() {
            // Created or continued by this agent instance.
            self.work.seen = Some(record.id);
            self.work.record = Some(record);
        } else if self.work.record.as_ref().is_some_and(|r| r.id == record.id)
            || self.work.record.is_none()
        {
            self.work.record = None;
        }
    }

    /// The profile a session operation may use: the active one.
    fn active_profile_id(&self) -> Option<ProfileId> {
        self.runtime.profile().map(|p| p.id)
    }

    /// Values for the inputs a session workflow declares by these names.
    fn session_inputs(&self, workflow: WorkflowId) -> BTreeMap<String, String> {
        let mut inputs = BTreeMap::new();
        let (Some(record), Some(definition)) = (
            self.work.record.as_ref(),
            self.runtime.profile().and_then(|p| p.workflow(workflow)),
        ) else {
            return inputs;
        };
        let elapsed = self
            .store
            .as_ref()
            .and_then(|s| s.work_session_elapsed_ms(record.id, self.wall_ms()).ok())
            .unwrap_or(0)
            / 1000;
        for input in &definition.inputs {
            let value = match input.key.as_str() {
                "sessionId" => Some(record.id.to_string()),
                "projectId" => record.project.map(|p| p.to_string()),
                "elapsedSeconds" => Some(elapsed.to_string()),
                _ => None,
            };
            if let Some(value) = value {
                inputs.insert(input.key.clone(), value);
            }
        }
        inputs
    }

    fn start_tagged_run(
        &mut self,
        workflow: WorkflowId,
        purpose: RunPurpose,
        detail: &str,
    ) -> Result<ExecutionId, String> {
        let launch = Launch {
            inputs: self.session_inputs(workflow),
            detail: Some(detail.to_owned()),
            purpose: Some(purpose),
        };
        self.start_execution_with(workflow, ExecutionTrigger::WorkSession, launch)
            .map(|summary| summary.id)
            .map_err(|error| error.message)
    }

    /// T1, then T2 at once or once the start workflow succeeded.
    fn start_session(&mut self) -> WorkSessionOutcome {
        let Some(profile) = self.active_profile_id() else {
            return WorkSessionOutcome::Failed("no profile is active".into());
        };
        let now = self.wall_ms();
        let settings = match self.work_store() {
            Ok(store) => store.work_session_settings(profile),
            Err(outcome) => return outcome,
        };
        let settings = match settings {
            Ok(settings) => settings,
            Err(error) => return WorkSessionOutcome::Failed(error.to_string()),
        };
        let new = NewWorkSession {
            id: WorkSessionId::new(),
            profile,
            project: self.runtime.active_project_id(),
            start_workflow: settings.start_workflow,
            at_ms: now,
        };
        let begun = self.work_store().map(|s| s.begin_work_session(&new));
        let begun = match begun {
            Ok(result) => self.take_transition(result),
            Err(outcome) => return outcome,
        };
        match begun {
            Ok(Transition::Applied(_)) => {}
            Ok(Transition::Repeated(_)) => return WorkSessionOutcome::AlreadyDone,
            Ok(Transition::Refused(refusal)) => {
                return WorkSessionOutcome::Failed(refusal_message(&refusal));
            }
            Err(outcome) => return outcome,
        }
        self.work.settings = Some(settings.clone());
        info!(session = %new.id, "work session requested");
        let Some(workflow) = settings.start_workflow else {
            return self.start_ended(
                new.id,
                &RunEnd {
                    status: Some(RunStatus::Succeeded),
                    ..RunEnd::failed("")
                },
            );
        };
        match self.start_tagged_run(
            workflow,
            RunPurpose::Start { session: new.id },
            "work session start",
        ) {
            Ok(execution) => {
                if let Ok(store) = self.work_store()
                    && let Err(error) = store.set_work_session_start_execution(new.id, execution)
                {
                    warn!(%error, "cannot record the start run of the work session");
                }
                WorkSessionOutcome::Applied
            }
            Err(message) => {
                self.start_ended(new.id, &RunEnd::failed(message.clone()));
                WorkSessionOutcome::Failed(message)
            }
        }
    }

    /// T2 or T3 for the start run of `session`.
    fn start_ended(&mut self, session: WorkSessionId, end: &RunEnd) -> WorkSessionOutcome {
        let now = self.wall_ms();
        let result = match end.status {
            Some(RunStatus::Succeeded) => self
                .work_store()
                .map(|s| s.work_session_start_succeeded(session, now)),
            status => {
                let reason = end.message.as_deref().unwrap_or("it failed");
                let message = if status == Some(RunStatus::Cancelled) {
                    "The start was cancelled".to_owned()
                } else {
                    format!("The start workflow failed: {reason}")
                };
                let notice = notice(WorkSessionProblem::StartFailed, message, now);
                self.work_store()
                    .map(|s| s.work_session_start_failed(session, &notice, now))
            }
        };
        let result = match result {
            Ok(result) => self.take_transition(result),
            Err(outcome) => return outcome,
        };
        match result {
            Ok(Transition::Applied(record)) => {
                info!(%session, state = record.status.as_str(), "work session start ended");
                WorkSessionOutcome::Applied
            }
            Ok(Transition::Repeated(_)) => WorkSessionOutcome::AlreadyDone,
            Ok(Transition::Refused(refusal)) => {
                debug!(%session, ?refusal, "a start result was dropped");
                WorkSessionOutcome::Ignored(Ignored::StateChanged)
            }
            Err(outcome) => outcome,
        }
    }

    /// T4 (or T8 from readyToFinish): runs the verification, or passes at
    /// once by long press when none is configured (D12).
    fn verify(&mut self, gesture: Gesture) -> WorkSessionOutcome {
        let Some(record) = self.work.record.clone() else {
            return WorkSessionOutcome::Ignored(Ignored::NotNow);
        };
        if self.active_profile_id() != Some(record.profile) {
            return WorkSessionOutcome::Ignored(Ignored::ProfileNotActive);
        }
        let settings = match self
            .work_store()
            .map(|s| s.work_session_settings(record.profile))
        {
            Ok(Ok(settings)) => settings,
            Ok(Err(error)) => return WorkSessionOutcome::Failed(error.to_string()),
            Err(outcome) => return outcome,
        };
        let workflow = settings.verification_workflow;
        match (workflow, gesture) {
            (None, Gesture::LongPress) | (Some(_), Gesture::Click | Gesture::DoubleClick) => {}
            (None, _) => return WorkSessionOutcome::Ignored(Ignored::HoldToFinish),
            (Some(_), Gesture::LongPress) => {
                return WorkSessionOutcome::Ignored(Ignored::ClickToCheck);
            }
        }
        self.work.settings = Some(settings);
        let verification = VerificationId::new();
        let now = self.wall_ms();
        let begun = self.work_store().map(|s| {
            s.begin_work_session_verification(record.id, record.status, verification, workflow, now)
        });
        let begun = match begun {
            Ok(result) => self.take_transition(result),
            Err(outcome) => return outcome,
        };
        match begun {
            Ok(Transition::Applied(_)) => {}
            Ok(Transition::Repeated(_)) => return WorkSessionOutcome::AlreadyDone,
            Ok(Transition::Refused(refusal)) => {
                return WorkSessionOutcome::Failed(refusal_message(&refusal));
            }
            Err(outcome) => return outcome,
        }
        let Some(workflow) = workflow else {
            // D12: T4 and T5 together.
            let result = VerificationResult::Outcome(VerificationOutcome {
                can_finish: true,
                details: Some(NO_VERIFICATION.to_owned()),
            });
            return self.record_verification(record.id, verification, &result);
        };
        let contract = self
            .runtime
            .profile()
            .and_then(|p| p.workflow(workflow))
            .ok_or_else(|| "the verification workflow no longer exists".to_owned())
            .and_then(|d| verification_contract(&d.outputs).map_err(|e| e.to_string()));
        let started = contract.and_then(|()| {
            self.start_tagged_run(
                workflow,
                RunPurpose::Verify {
                    session: record.id,
                    verification,
                },
                "work session verification",
            )
        });
        match started {
            Ok(execution) => {
                if let Ok(store) = self.work_store()
                    && let Err(error) = store.set_work_session_verification_execution(
                        record.id,
                        verification,
                        execution,
                    )
                {
                    warn!(%error, "cannot record the verification run of the work session");
                }
                WorkSessionOutcome::Applied
            }
            Err(message) => {
                let failed = VerificationResult::Failed(notice(
                    WorkSessionProblem::VerificationFailed,
                    format!("The verification could not run: {message}"),
                    now,
                ));
                self.record_verification(record.id, verification, &failed);
                WorkSessionOutcome::Failed(message)
            }
        }
    }

    /// T5, T6 or T7 with the result of `verification`.
    fn record_verification(
        &mut self,
        session: WorkSessionId,
        verification: VerificationId,
        result: &VerificationResult,
    ) -> WorkSessionOutcome {
        let now = self.wall_ms();
        let boot = self.work.boot.clone();
        let recorded = self
            .work_store()
            .map(|s| s.record_work_session_verification(session, verification, result, &boot, now));
        let recorded = match recorded {
            Ok(result) => self.take_transition(result),
            Err(outcome) => return outcome,
        };
        match recorded {
            Ok(Transition::Applied(record)) => {
                info!(%session, state = record.status.as_str(), "work session verified");
                WorkSessionOutcome::Applied
            }
            Ok(Transition::Repeated(_)) => WorkSessionOutcome::AlreadyDone,
            Ok(Transition::Refused(refusal)) => {
                // An older verification or another session: dropped.
                debug!(%session, %verification, ?refusal, "a verification result was dropped");
                WorkSessionOutcome::Ignored(Ignored::StateChanged)
            }
            Err(outcome) => outcome,
        }
    }

    /// `cancelStart` / `cancelVerification`: cancels the tagged run; its
    /// end applies T3 or T7. Without a run, the transition is applied here.
    fn cancel_run(&mut self, verification: bool) -> WorkSessionOutcome {
        let Some(record) = self.work.record.clone() else {
            return WorkSessionOutcome::Ignored(Ignored::NotNow);
        };
        let purpose = |p: &RunPurpose| match p {
            RunPurpose::Start { session } => !verification && *session == record.id,
            RunPurpose::Verify {
                session,
                verification: v,
            } => verification && *session == record.id && Some(*v) == record.verification,
        };
        if self.cancel_tagged_runs(purpose) {
            return WorkSessionOutcome::Applied;
        }
        if verification {
            let Some(v) = record.verification else {
                return WorkSessionOutcome::Ignored(Ignored::NotNow);
            };
            self.verification_ended(record.id, v, &RunEnd::cancelled())
        } else {
            self.start_ended(record.id, &RunEnd::cancelled())
        }
    }

    /// `continueWorking`: T9 without a notice.
    fn continue_working(&mut self) -> WorkSessionOutcome {
        self.invalidate_approval(None)
    }

    /// T9 for the current approval.
    fn invalidate_approval(
        &mut self,
        problem: Option<(WorkSessionProblem, &str)>,
    ) -> WorkSessionOutcome {
        let Some(record) = self.work.record.clone() else {
            return WorkSessionOutcome::Ignored(Ignored::NotNow);
        };
        let Some(approval) = record.approval else {
            return WorkSessionOutcome::Ignored(Ignored::NotNow);
        };
        let now = self.wall_ms();
        let notice = problem.map(|(problem, message)| notice(problem, message, now));
        let result = self
            .work_store()
            .map(|s| s.invalidate_work_session_approval(record.id, approval, notice.as_ref(), now));
        let result = match result {
            Ok(result) => self.take_transition(result),
            Err(outcome) => return outcome,
        };
        match result {
            Ok(Transition::Applied(_)) => {
                info!(session = %record.id, ?problem, "work session approval dropped");
                WorkSessionOutcome::Applied
            }
            Ok(Transition::Repeated(_)) => WorkSessionOutcome::AlreadyDone,
            Ok(Transition::Refused(_)) => WorkSessionOutcome::Ignored(Ignored::StateChanged),
            Err(outcome) => outcome,
        }
    }

    /// T10 and T11: only a single click, with the approval of the press,
    /// after the arming delay. Atomic and idempotent in the store.
    fn finish(&mut self, request: &WorkSessionRequest) -> WorkSessionOutcome {
        let press = &request.press;
        if press.state != WorkSessionState::ReadyToFinish {
            return WorkSessionOutcome::Ignored(Ignored::StateChanged);
        }
        if request.gesture != Gesture::Click {
            return WorkSessionOutcome::Ignored(Ignored::NotAClick);
        }
        let (Some(session), Some(approval)) = (press.session, press.approval) else {
            return WorkSessionOutcome::Ignored(Ignored::StateChanged);
        };
        let current = self.work.record.as_ref().map(|r| (r.id, r.status));
        if current == Some((session, SessionStatus::ReadyToFinish))
            && self
                .work
                .ready_since
                .is_none_or(|since| press.at < since + ARMING_DELAY)
        {
            return WorkSessionOutcome::Ignored(Ignored::NotArmed);
        }
        let now = self.wall_ms();
        let boot = self.work.boot.clone();
        let finished = self
            .work_store()
            .map(|s| s.finish_work_session(session, approval, &boot, now));
        let finished = match finished {
            Ok(result) => self.take_transition(result),
            Err(outcome) => return outcome,
        };
        match finished {
            Ok(Transition::Applied(_)) => {
                info!(%session, "work session finished");
                self.complete_stop(session);
                WorkSessionOutcome::Applied
            }
            Ok(Transition::Repeated(record)) => {
                if record.status == SessionStatus::Stopping {
                    self.complete_stop(session);
                }
                WorkSessionOutcome::AlreadyDone
            }
            Ok(Transition::Refused(Refusal::ApprovalExpired)) => {
                self.invalidate_approval(Some((
                    WorkSessionProblem::ApprovalExpired,
                    "The approval expired; check again",
                )));
                WorkSessionOutcome::Failed("the approval expired".into())
            }
            Ok(Transition::Refused(Refusal::ApprovalFromAnotherBoot)) => {
                self.invalidate_approval(Some((
                    WorkSessionProblem::ApprovalInvalidated,
                    "DeckPilot restarted after the check; check again",
                )));
                WorkSessionOutcome::Failed("the approval is no longer valid".into())
            }
            Ok(Transition::Refused(refusal)) => {
                debug!(%session, ?refusal, "a finish click was refused");
                WorkSessionOutcome::Ignored(Ignored::StateChanged)
            }
            Err(outcome) => outcome,
        }
    }

    /// T11.
    fn complete_stop(&mut self, session: WorkSessionId) {
        let now = self.wall_ms();
        let result = self
            .work_store()
            .map(|s| s.complete_work_session_stop(session, now));
        if let Ok(result) = result
            && let Err(outcome) = self.take_transition(result)
        {
            // Recovery completes it at the next start or unlock.
            warn!(%session, ?outcome, "cannot complete the stop of the work session");
        }
    }

    /// A tagged run ended (called by the execution service, with the
    /// outputs read before the report was made safe).
    pub(crate) fn work_session_run_ended(&mut self, purpose: RunPurpose, end: &RunEnd) {
        if self.store.is_none() {
            // Locked: recovery at the unlock decides (T3/T7 interrupted).
            debug!(?purpose, "a work session run ended while locked");
            return;
        }
        let outcome = match purpose {
            RunPurpose::Start { session } => self.start_ended(session, end),
            RunPurpose::Verify {
                session,
                verification,
            } => self.verification_ended(session, verification, end),
        };
        debug!(?purpose, ?outcome, "work session run ended");
        self.work_session_changed();
    }

    fn verification_ended(
        &mut self,
        session: WorkSessionId,
        verification: VerificationId,
        end: &RunEnd,
    ) -> WorkSessionOutcome {
        let now = self.wall_ms();
        let failed = |message: String| {
            VerificationResult::Failed(notice(WorkSessionProblem::VerificationFailed, message, now))
        };
        let result = match end.status {
            Some(RunStatus::Succeeded) => {
                match VerificationOutcome::from_outputs(&end.declared, &end.outputs) {
                    Ok(outcome) => VerificationResult::Outcome(outcome),
                    Err(error) => failed(format!("The verification gave no valid result: {error}")),
                }
            }
            Some(RunStatus::Cancelled) => failed("The verification was cancelled".into()),
            _ => failed(format!(
                "The verification failed: {}",
                end.message.as_deref().unwrap_or("it did not finish")
            )),
        };
        self.record_verification(session, verification, &result)
    }

    /// Whenever the database opened (agent start and every unlock): brings
    /// the persisted session to a state this agent instance can continue
    /// (§6.5). Ambiguous cases keep the timer and leave the next step to
    /// the user.
    pub(crate) fn recover_work_session(&mut self) {
        let Some(store) = self.store.as_ref() else {
            return;
        };
        let now = self.wall_ms();
        let active = match store.active_work_session() {
            Ok(active) => active,
            Err(error) => {
                warn!(%error, "cannot read the work session");
                return;
            }
        };
        let Some(record) = active else {
            self.work.record = None;
            self.work.ready_since = None;
            self.work.pending_gaps.clear();
            self.work.pending_invalidation = false;
            self.work_session_changed();
            return;
        };
        let restarted = self.work.seen != Some(record.id);
        let settings = store.work_session_settings(record.profile).ok();
        if self.work.record.as_ref().map(|r| r.id) != Some(record.id) {
            self.work.ready_since = None;
        }
        self.work.record = Some(record.clone());
        self.work.seen = Some(record.id);
        info!(session = %record.id, state = record.status.as_str(), restarted, "work session recovered");

        // Gaps first: the transition's notice is the newer one.
        if record.status.state().timer_running() {
            let mut gaps = std::mem::take(&mut self.work.pending_gaps);
            if restarted {
                let from = record.last_alive_at_ms;
                if now < from - millis_of(CLOCK_BACK) {
                    gaps.push((GapKind::ClockBack, from, now));
                } else if now > from {
                    gaps.push((GapKind::AgentDown, from, now));
                }
            }
            for (kind, from, to) in gaps {
                self.flag_gap(record.id, kind, from, to);
            }
        } else {
            self.work.pending_gaps.clear();
        }
        let invalidate = std::mem::take(&mut self.work.pending_invalidation);
        let transition = self.recovery_transition(&record, settings.as_ref(), invalidate, now);
        if let Some(result) = transition
            && let Err(outcome) = self.take_transition(result)
        {
            warn!(?outcome, "cannot recover the work session");
        }
        self.work.settings = settings;
        if let Some(record) = self.work.record.as_ref()
            && let Some(store) = self.store.as_ref()
            && let Ok(Some(fresh)) = store.work_session(record.id)
        {
            self.work.record = fresh.status.is_active().then_some(fresh);
        }
        self.heartbeat(true);
        self.work_session_changed();
    }

    /// The transition that makes a recovered session one this agent
    /// instance can continue (§6.5), applied by the store.
    fn recovery_transition(
        &self,
        record: &WorkSessionRecord,
        settings: Option<&WorkSessionSettings>,
        invalidate: bool,
        now: i64,
    ) -> Option<Result<Transition, deckpilot_storage::StorageError>> {
        let at = |problem, message: &str| notice(problem, message, now);
        let store = self.store.as_ref()?;
        match record.status {
            SessionStatus::Starting => Some(store.work_session_start_failed(
                record.id,
                &at(
                    WorkSessionProblem::StartInterrupted,
                    "The start was interrupted; start again",
                ),
                now,
            )),
            SessionStatus::VerifyingEnd => record.verification.map(|v| {
                store.record_work_session_verification(
                    record.id,
                    v,
                    &VerificationResult::Failed(at(
                        WorkSessionProblem::VerificationInterrupted,
                        "The check was interrupted; check again",
                    )),
                    &self.work.boot,
                    now,
                )
            }),
            SessionStatus::ReadyToFinish => {
                let expired = settings.is_some_and(|s| {
                    !record
                        .approved_at_ms
                        .is_some_and(|approved| s.approval_holds(approved, now))
                });
                let other_boot = record.approval_boot.as_deref() != Some(self.work.boot.as_str());
                match record.approval {
                    Some(approval) if other_boot || invalidate || expired => {
                        let notice = if expired && !other_boot && !invalidate {
                            at(
                                WorkSessionProblem::ApprovalExpired,
                                "The approval expired; check again",
                            )
                        } else {
                            at(
                                WorkSessionProblem::ApprovalInvalidated,
                                "The approval is no longer valid; check again",
                            )
                        };
                        Some(store.invalidate_work_session_approval(
                            record.id,
                            approval,
                            Some(&notice),
                            now,
                        ))
                    }
                    _ => None,
                }
            }
            SessionStatus::Stopping => Some(store.complete_work_session_stop(record.id, now)),
            SessionStatus::Working | SessionStatus::Stopped | SessionStatus::Aborted => None,
        }
    }

    fn flag_gap(&mut self, session: WorkSessionId, kind: GapKind, from: i64, to: i64) {
        let Some(store) = self.store.as_ref() else {
            self.work.pending_gaps.push((kind, from, to));
            return;
        };
        match store.flag_work_session_gap(session, kind, from, to, self.wall_ms()) {
            Ok(Some(review)) => {
                info!(%session, ?kind, minutes = (to - from).abs() / 60_000, review = review.id, "a gap of the work session needs a review");
            }
            Ok(None) => {}
            Err(error) => warn!(%error, "cannot flag a gap of the work session"),
        }
    }

    /// Moves `last_alive_at` once per [`HEARTBEAT`] (or now, if `force`).
    fn heartbeat(&mut self, force: bool) {
        let now = (self.options.clock)();
        let due = force
            || self.work.beat_mono.elapsed() >= HEARTBEAT
            || now
                .duration_since(self.work.beat_wall)
                .or_else(|e| Ok::<_, ()>(e.duration()))
                .is_ok_and(|d| d >= HEARTBEAT);
        if !due {
            return;
        }
        let (Some(store), Some(record)) = (self.store.as_ref(), self.work.record.as_ref()) else {
            return;
        };
        match store.touch_work_session(record.id, millis(now)) {
            Ok(_) => {
                self.work.beat_wall = now;
                self.work.beat_mono = Instant::now();
            }
            Err(error) => warn!(%error, "cannot record that the work session is alive"),
        }
    }

    /// On every periodic check (before auto-lock decides): heartbeat,
    /// approval lifetime, and sleep or clock jumps from the wall clock
    /// against the monotonic clock (§6.1).
    pub(crate) fn check_work_session(&mut self) {
        let wall = (self.options.clock)();
        let mono = Instant::now();
        let mono_delta = mono.duration_since(self.work.last_mono);
        let expected = self.work.last_wall + mono_delta;
        let (from, to) = (millis(expected), millis(wall));
        self.work.last_wall = wall;
        self.work.last_mono = mono;
        let jump = if to - from > millis_of(SLEEP_GAP) {
            Some(GapKind::Sleep)
        } else if from - to > millis_of(CLOCK_BACK) {
            Some(GapKind::ClockBack)
        } else {
            None
        };
        let session = self
            .work
            .record
            .as_ref()
            .filter(|r| r.status.state().timer_running())
            .map(|r| r.id);
        if let (Some(kind), Some(session)) = (jump, session) {
            info!(
                ?kind,
                minutes = (to - from).abs() / 60_000,
                "the wall clock jumped during a work session"
            );
            self.flag_gap(session, kind, from, to);
            if self.store.is_none() {
                self.work.pending_invalidation = true;
            } else if self.work.state() == WorkSessionState::ReadyToFinish {
                self.invalidate_approval(Some((
                    WorkSessionProblem::ApprovalInvalidated,
                    "The computer slept or the clock changed; check again",
                )));
            }
            self.work_session_changed();
        }
        if self.store.is_none() {
            return;
        }
        // The approval lifetime (T9 approvalExpired).
        if let (Some(record), Some(settings)) =
            (self.work.record.as_ref(), self.work.settings.as_ref())
            && record.status == SessionStatus::ReadyToFinish
            && !record
                .approved_at_ms
                .is_some_and(|approved| settings.approval_holds(approved, to))
        {
            self.invalidate_approval(Some((
                WorkSessionProblem::ApprovalExpired,
                "The approval expired; check again",
            )));
            self.work_session_changed();
        }
        self.heartbeat(false);
    }

    /// The configuration was reloaded: changed settings of the session's
    /// profile drop an approval (D9).
    pub(crate) fn work_session_settings_reloaded(&mut self) {
        let (Some(store), Some(record)) = (self.store.as_ref(), self.work.record.as_ref()) else {
            return;
        };
        let Ok(fresh) = store.work_session_settings(record.profile) else {
            return;
        };
        let changed = self
            .work
            .settings
            .as_ref()
            .is_some_and(|old| comparable(old) != comparable(&fresh));
        let ready = record.status == SessionStatus::ReadyToFinish;
        self.work.settings = Some(fresh);
        if changed && ready {
            self.invalidate_approval(Some((
                WorkSessionProblem::ApprovalInvalidated,
                "The work session settings changed; check again",
            )));
            self.work_session_changed();
        }
    }

    /// The agent stops cleanly: the last heartbeat.
    pub(crate) fn work_session_shutdown(&mut self) {
        self.heartbeat(true);
    }

    /// Publishes the session as it is now and redraws keys. W4 (faces) and
    /// W5 (the `workSessionChanged` event) hook in here.
    pub(crate) fn work_session_changed(&mut self) {
        let now = self.wall_ms();
        let snapshot = if let Some(store) = self.store.as_ref() {
            build_snapshot(store, self.work.record.as_ref(), now)
        } else {
            // Locked: the last known state, with time moved on.
            let mut last = self.work.snapshot();
            if last.state.timer_running() {
                last.elapsed_ms = last
                    .elapsed_ms
                    .saturating_add(u64::try_from(now - last.as_of_ms).unwrap_or(0));
            }
            last.as_of_ms = now;
            last
        };
        let changed = self.work.snapshot.send_if_modified(|current| {
            let differs = current.state != snapshot.state
                || current.session != snapshot.session
                || current.approval != snapshot.approval
                || current.verification != snapshot.verification
                || current.notices != snapshot.notices
                || current.started_at_ms != snapshot.started_at_ms;
            *current = snapshot;
            differs
        });
        if changed {
            self.render_all();
        }
    }
}

fn millis_of(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

fn refusal_message(refusal: &Refusal) -> String {
    match refusal {
        Refusal::AnotherSessionActive { .. } => "another work session is active".into(),
        Refusal::UnknownSession => "the work session no longer exists".into(),
        Refusal::WrongState { status } => format!("the work session is {}", status.as_str()),
        Refusal::StaleVerification => "the check is out of date".into(),
        Refusal::ApprovalExpired => "the approval expired".into(),
        Refusal::ApprovalFromAnotherBoot => "the approval is no longer valid".into(),
    }
}

/// The snapshot of `record` at `now` (idle with the last notice when there
/// is none).
fn build_snapshot(
    store: &SqliteStore,
    record: Option<&WorkSessionRecord>,
    now: i64,
) -> WorkSessionSnapshot {
    let Some(record) = record else {
        let mut idle = WorkSessionSnapshot::idle(now);
        if let Ok(Some((_, notice))) = store.idle_work_session_notice() {
            idle.notices.push(notice);
        }
        return idle;
    };
    let elapsed_ms = store
        .work_session_elapsed_ms(record.id, now)
        .unwrap_or_default();
    let mut notices: Vec<WorkSessionNotice> = record.problem.iter().cloned().collect();
    let open_reviews = store.work_session_reviews(record.id).map_or(0, |reviews| {
        reviews.iter().filter(|r| r.resolution.is_none()).count()
    });
    if open_reviews > 0
        && !notices
            .iter()
            .any(|n| n.problem == WorkSessionProblem::NeedsReview)
    {
        notices.push(notice(
            WorkSessionProblem::NeedsReview,
            "Time of this session needs a review",
            record.updated_at_ms,
        ));
    }
    let state = record.status.state();
    WorkSessionSnapshot {
        state,
        session: Some(record.id),
        profile: Some(record.profile),
        project: record.project,
        started_at_ms: record.started_at_ms,
        elapsed_ms,
        as_of_ms: now,
        verification: record.verification.map(|id| VerificationRecord {
            id,
            outcome: record.can_finish.map(|can_finish| VerificationOutcome {
                can_finish,
                details: record.verification_details.clone(),
            }),
            at_ms: record.verified_at_ms.unwrap_or(record.updated_at_ms),
        }),
        approval: (state == WorkSessionState::ReadyToFinish)
            .then_some(record.approval)
            .flatten(),
        notices,
    }
}
